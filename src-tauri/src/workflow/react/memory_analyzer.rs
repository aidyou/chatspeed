//! Memory analyzer for ChatSpeed workflow engine.
//!
//! The analyzer prefers structured LLM extraction for multilingual support,
//! and falls back to deterministic heuristics when the model call fails or
//! returns invalid output.

use std::sync::Arc;

use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::AiChatEnum;
use crate::ai::interaction::chat_completion::ChatState;
use crate::ai::traits::chat::{ChatMetadata, ChatResponse};
use crate::ccproxy::ChatProtocol;
use tokio::sync::mpsc;

use super::error::WorkflowEngineError;
use super::memory::{normalize_memory_entry, MemoryAnalysisResult, MemoryCandidateDraft};

/// Memory analyzer for task completion memory updates.
pub struct MemoryAnalyzer;

impl MemoryAnalyzer {
    pub async fn analyze(
        user_inputs: Vec<String>,
        current_global_memory: Option<String>,
        current_project_memory: Option<String>,
        chat_state: Arc<ChatState>,
        session_id: &str,
        provider_id: i64,
        model_name: &str,
    ) -> Result<MemoryAnalysisResult, WorkflowEngineError> {
        if user_inputs.is_empty() {
            return Ok(MemoryAnalysisResult {
                global_memory: None,
                project_memory: None,
                global_candidates: Vec::new(),
                project_candidates: Vec::new(),
                reasoning: Some("No user inputs to analyze".to_string()),
            });
        }

        log::info!(
            "[Workflow][session={}][phase=memory] Memory analyzer request starting: provider_id={}, model={}, user_inputs={}, has_global_memory={}, has_project_memory={}",
            session_id,
            provider_id,
            model_name,
            user_inputs.len(),
            current_global_memory
                .as_ref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false),
            current_project_memory
                .as_ref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
        );

        let llm_result = Self::analyze_with_llm(
            &user_inputs,
            current_global_memory,
            current_project_memory,
            chat_state,
            session_id,
            provider_id,
            model_name,
        )
        .await;

        let mut result = match llm_result {
            Ok(result) => {
                log::info!(
                    "[Workflow][session={}][phase=memory] Memory analyzer used structured LLM extraction",
                    session_id
                );
                result
            }
            Err(error) => {
                log::warn!(
                    "[Workflow][session={}][phase=memory] Memory analyzer LLM extraction failed, falling back to heuristics: {}",
                    session_id,
                    error
                );
                Self::analyze_with_heuristics(&user_inputs)
            }
        };

        Self::dedupe_candidates(&mut result.global_candidates);
        Self::dedupe_candidates(&mut result.project_candidates);

        Ok(result)
    }

    async fn analyze_with_llm(
        user_inputs: &[String],
        current_global_memory: Option<String>,
        current_project_memory: Option<String>,
        chat_state: Arc<ChatState>,
        session_id: &str,
        provider_id: i64,
        model_name: &str,
    ) -> Result<MemoryAnalysisResult, WorkflowEngineError> {
        let chat_interface = {
            let mut chats_guard = chat_state.chats.lock().await;
            let protocol = ChatProtocol::OpenAI;
            let chat_map = chats_guard
                .entry(protocol)
                .or_insert_with(std::collections::HashMap::new);
            chat_map
                .entry(session_id.to_string())
                .or_insert_with(|| crate::create_chat!(chat_state.main_store.clone()))
                .clone()
        };

        let system_prompt = super::prompts::MEMORY_ANALYZER_SYSTEM_PROMPT;
        let user_prompt = super::prompts::MEMORY_ANALYZER_USER_PROMPT_TEMPLATE
            .replace(
                "{global_memory}",
                &current_global_memory.unwrap_or_default(),
            )
            .replace(
                "{project_memory}",
                &current_project_memory.unwrap_or_default(),
            )
            .replace("{user_inputs}", &user_inputs.join("\n\n---\n\n"));

        let messages = vec![
            serde_json::json!({
                "role": "system",
                "content": system_prompt
            }),
            serde_json::json!({
                "role": "user",
                "content": user_prompt
            }),
        ];

        let (tx, _rx) = mpsc::channel::<Arc<ChatResponse>>(32);
        let response = chat_interface
            .chat(
                provider_id,
                model_name,
                format!("{}-memory", session_id),
                messages,
                None,
                Some(ChatMetadata {
                    temperature: Some(0.1),
                    ..Default::default()
                }),
                move |chunk| {
                    let _ = tx.try_send(chunk);
                },
            )
            .await
            .map_err(|e| {
                WorkflowEngineError::General(format!("Memory analysis LLM call failed: {}", e))
            })?;

        Self::parse_response(&response)
    }

    fn parse_response(response: &str) -> Result<MemoryAnalysisResult, WorkflowEngineError> {
        let json_str = crate::libs::util::format_json_str(response);
        serde_json::from_str(&json_str).map_err(|e| {
            WorkflowEngineError::General(format!(
                "Failed to parse memory analysis result: {}. Response: {}",
                e, response
            ))
        })
    }

    fn analyze_with_heuristics(user_inputs: &[String]) -> MemoryAnalysisResult {
        let mut global_candidates = Vec::new();
        let mut project_candidates = Vec::new();

        for input in user_inputs {
            for fragment in Self::split_candidate_fragments(input) {
                if let Some(candidate) = Self::build_candidate(&fragment) {
                    if Self::is_project_scoped(&fragment) {
                        project_candidates.push(candidate);
                    } else {
                        global_candidates.push(candidate);
                    }
                }
            }
        }

        MemoryAnalysisResult {
            global_memory: None,
            project_memory: None,
            global_candidates,
            project_candidates,
            reasoning: Some("Fallback heuristic extraction".to_string()),
        }
    }

    fn split_candidate_fragments(input: &str) -> Vec<String> {
        input
            .replace(['。', '！', '？', ';', '；'], "\n")
            .lines()
            .map(str::trim)
            .filter(|line| line.len() >= 8)
            .map(str::to_string)
            .collect()
    }

    fn build_candidate(fragment: &str) -> Option<MemoryCandidateDraft> {
        if Self::looks_like_transient_request(fragment) {
            return None;
        }

        let explicitness = Self::explicitness_score(fragment);
        if explicitness == 0 {
            return None;
        }

        let category = if Self::is_constraint(fragment) {
            "constraint"
        } else if Self::is_project_scoped(fragment) {
            "convention"
        } else {
            "preference"
        };

        Some(MemoryCandidateDraft {
            category: category.to_string(),
            content: Self::normalize_candidate_content(fragment),
            confidence: if explicitness >= 3 { 0.95 } else { 0.75 },
            explicitness,
        })
    }

    fn normalize_candidate_content(fragment: &str) -> String {
        fragment
            .trim()
            .trim_start_matches("我觉得")
            .trim_start_matches("我希望")
            .trim_start_matches("希望")
            .trim_start_matches("请")
            .trim()
            .to_string()
    }

    fn dedupe_candidates(candidates: &mut Vec<MemoryCandidateDraft>) {
        let mut seen = std::collections::HashSet::new();
        candidates.retain(|candidate| {
            let key = format!(
                "{}:{}",
                candidate.category,
                normalize_memory_entry(&candidate.content)
            );
            seen.insert(key)
        });
    }

    fn explicitness_score(fragment: &str) -> i32 {
        let lower = fragment.to_lowercase();
        let strong_markers = [
            "must",
            "must not",
            "never",
            "always",
            "始终",
            "forbid",
            "forbidden",
            "禁止",
            "必须",
            "不要",
            "一律",
            "统一",
        ];
        if strong_markers.iter().any(|marker| lower.contains(marker)) {
            return 3;
        }

        let medium_markers = [
            "prefer",
            "preferred",
            "avoid",
            "please use",
            "希望",
            "尽量",
            "偏好",
            "习惯",
            "建议",
            "最好",
        ];
        if medium_markers.iter().any(|marker| lower.contains(marker)) {
            return 2;
        }

        0
    }

    fn is_project_scoped(fragment: &str) -> bool {
        let markers = [
            "本项目",
            "这个项目",
            "当前项目",
            "这个仓库",
            "代码库",
            "注释",
            "命名",
            "格式化",
            "pnpm",
            "cargo",
            "tauri",
            "vue",
            "rust",
        ];
        let lower = fragment.to_lowercase();
        markers
            .iter()
            .any(|marker| fragment.contains(marker) || lower.contains(marker))
    }

    fn is_constraint(fragment: &str) -> bool {
        let lower = fragment.to_lowercase();
        [
            "must", "must not", "never", "forbid", "禁止", "必须", "不要", "不能", "avoid",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
    }

    fn looks_like_transient_request(fragment: &str) -> bool {
        let lower = fragment.to_lowercase();
        let action_markers = [
            "修复",
            "实现",
            "增加",
            "新增",
            "看看",
            "分析",
            "排查",
            "怎么改",
            "为什么",
            "fix ",
            "implement",
            "add ",
            "check ",
            "analyze",
            "look at",
        ];
        let preference_markers = [
            "must", "never", "always", "prefer", "禁止", "必须", "不要", "偏好", "希望", "尽量",
        ];

        action_markers.iter().any(|marker| lower.contains(marker))
            && !preference_markers
                .iter()
                .any(|marker| lower.contains(marker))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_response_with_candidates() {
        let response = r#"{
          "globalMemory": null,
          "projectMemory": null,
          "globalCandidates": [
            { "category": "preference", "content": "Use Chinese", "confidence": 0.8, "explicitness": 2 }
          ],
          "projectCandidates": [],
          "reasoning": "ok"
        }"#;
        let parsed = MemoryAnalyzer::parse_response(response).unwrap();
        assert_eq!(parsed.global_candidates.len(), 1);
        assert_eq!(parsed.global_candidates[0].category, "preference");
    }

    #[test]
    fn test_build_candidate_extracts_global_preference() {
        let candidate = MemoryAnalyzer::build_candidate("请始终用中文对话").unwrap();
        assert_eq!(candidate.category, "preference");
        assert!(candidate.explicitness >= 2);
    }

    #[test]
    fn test_build_candidate_extracts_project_constraint() {
        let candidate = MemoryAnalyzer::build_candidate("本项目代码注释必须使用英文").unwrap();
        assert_eq!(candidate.category, "constraint");
    }

    #[test]
    fn test_build_candidate_skips_transient_request() {
        let candidate = MemoryAnalyzer::build_candidate("帮我修复这个路径守卫 bug");
        assert!(candidate.is_none());
    }
}
