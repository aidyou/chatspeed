use serde_json::{json, Value};
use sha2::Digest;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

use crate::db::WorkflowMessage;
use crate::tools::{
    READ_ONLY_BASH_CMDS_EXACT, READ_ONLY_BASH_PREFIXES, TOOL_ASK_USER, TOOL_BASH,
    TOOL_COMPLETE_WORKFLOW, TOOL_EDIT_FILE, TOOL_PLAN_EDIT_NOTE, TOOL_PLAN_READ_NOTE,
    TOOL_PLAN_WRITE_NOTE, TOOL_SUBMIT_PLAN, TOOL_TODO_GET, TOOL_TODO_LIST, TOOL_WRITE_FILE,
};
use crate::workflow::react::constants::TASK_FINISHED;
use crate::workflow::react::engine::WorkflowExecutor;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::events::WorkflowEvent;
use crate::workflow::react::file_preview::{
    attach_display_context, attach_write_file_overwrite_old_content, normalize_preview_details,
    render_preview_details_text,
};
use crate::workflow::react::intelligence::ToolApprovalReview;
use crate::workflow::react::observation::{ObservationReinforcer, ReinforcedResult};
use crate::workflow::react::orchestrator::{spawn_call_sub_agent, FINAL_REVIEWER_BUILTIN_AGENT_ID};
use crate::workflow::react::policy::{ApprovalLevel, ExecutionPhase};
use crate::workflow::react::types::{
    GatewayPayload, PendingCompletionReport, StepType, WorkflowState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SmartApprovalDecision {
    AutoApprove,
    ReviewWithAi,
    ReviewByUser,
}

#[derive(Debug, PartialEq, Eq)]
struct ResolvedCompletionReport {
    content: String,
    source_message_id: Option<i64>,
    persist_as_message: bool,
    recency: (u8, usize, i64),
}

struct CompletionReportClaimDictionary {
    verification: [Vec<String>; 3],
    limitations: [Vec<String>; 2],
}

impl WorkflowExecutor {
    pub(crate) fn capture_pending_completion_report(&mut self, text_part: &str) -> bool {
        if !Self::is_valid_finish_task_summary(text_part) {
            return false;
        }
        let content = Self::normalized_finish_task_summary(text_part);
        let source_message_id = self
            .context
            .messages
            .iter()
            .rev()
            .find(|message| {
                message.role == "assistant" && message.segment_id == self.context.current_segment_id
            })
            .and_then(|message| message.id);
        let draft = PendingCompletionReport::new(
            &content,
            source_message_id,
            self.context.current_segment_id,
            self.current_step,
        );
        if self.pending_completion_reports.iter().any(|existing| {
            existing.segment_id == draft.segment_id
                && existing.content_hash == draft.content_hash
                && existing.content == draft.content
        }) {
            return false;
        }
        self.pending_completion_reports.push(draft);
        true
    }

    fn final_review_mode_enabled(&self) -> bool {
        self.agent_config.final_audit.unwrap_or(false)
    }

    pub(crate) fn should_invalidate_pending_completion_reports(tool_names: &[&str]) -> bool {
        tool_names
            .iter()
            .any(|name| *name != TOOL_COMPLETE_WORKFLOW)
    }

    pub(crate) fn todos_allow_completion_report_capture(todos: &[Value]) -> bool {
        todos.iter().all(|todo| {
            matches!(
                todo.get("status").and_then(Value::as_str),
                Some("completed" | "done" | "data_missing" | "failed" | "blocked")
            )
        })
    }

    fn has_implementation_action_after_approved_plan(messages: &[WorkflowMessage]) -> bool {
        let Some(approved_plan_index) = messages
            .iter()
            .rposition(|message| message.message_subtype.as_deref() == Some("approved_plan"))
        else {
            return false;
        };

        messages[approved_plan_index + 1..].iter().any(|message| {
            if message.role != "tool" || message.is_error {
                return false;
            }

            let tool_name = message
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("tool_name"))
                .and_then(Value::as_str);
            !matches!(
                tool_name,
                None | Some(
                    TOOL_SUBMIT_PLAN
                        | TOOL_COMPLETE_WORKFLOW
                        | TOOL_ASK_USER
                        | TOOL_TODO_LIST
                        | TOOL_TODO_GET
                        | TOOL_PLAN_READ_NOTE
                        | TOOL_PLAN_EDIT_NOTE
                        | TOOL_PLAN_WRITE_NOTE
                )
            )
        })
    }

    fn implementation_completion_is_blocked_for(
        phase: &ExecutionPhase,
        messages: &[WorkflowMessage],
    ) -> bool {
        if phase != &ExecutionPhase::Implementation
            || !messages
                .iter()
                .any(|message| message.message_subtype.as_deref() == Some("approved_plan"))
        {
            return false;
        }

        !Self::has_implementation_action_after_approved_plan(messages)
    }

    pub(crate) fn implementation_completion_is_blocked(&self) -> bool {
        Self::implementation_completion_is_blocked_for(
            &self.policy.phase,
            &self.context.messages_since_last_completion(),
        )
    }

    pub(crate) fn completion_report_capture_allowed(&self, todos: &[Value]) -> bool {
        Self::todos_allow_completion_report_capture(todos)
            && !self.implementation_completion_is_blocked()
    }

    async fn persist_completion_report(
        &mut self,
        report: ResolvedCompletionReport,
    ) -> Result<String, WorkflowEngineError> {
        if report.persist_as_message {
            self.add_message_and_notify_internal(
                "assistant".to_string(),
                report.content.clone(),
                None,
                None,
                Some(StepType::Think),
                false,
                None,
                Some(json!({
                    "message_kind": "completion_report",
                    "source": "complete_workflow.legacy_argument"
                })),
            )
            .await?;
        }
        Ok(report.content)
    }

    fn review_payload_changed_files(&self) -> Vec<Value> {
        let mut files = BTreeMap::<String, Value>::new();
        for message in self.context.messages_since_last_completion() {
            if message.role != "tool" || message.is_error {
                continue;
            }
            let Some(metadata) = message.metadata.as_ref() else {
                continue;
            };
            let Some(tool_name) = metadata.get("tool_name").and_then(|value| value.as_str()) else {
                continue;
            };
            if !matches!(
                tool_name,
                crate::tools::TOOL_EDIT_FILE | crate::tools::TOOL_WRITE_FILE
            ) {
                continue;
            }
            let Some(details) = metadata.get("details") else {
                continue;
            };
            let Some(path) = details
                .get("file_path")
                .or_else(|| details.get("path"))
                .and_then(|value| value.as_str())
            else {
                continue;
            };
            let entry = files.entry(path.to_string()).or_insert_with(|| {
                json!({
                    "path": path,
                    "operations": Vec::<String>::new(),
                    "line_ranges": Vec::<Value>::new(),
                    "summaries": Vec::<String>::new(),
                })
            });
            if let Some(operations) = entry
                .get_mut("operations")
                .and_then(|value| value.as_array_mut())
            {
                let operation = if tool_name == crate::tools::TOOL_WRITE_FILE {
                    "write_file"
                } else {
                    "edit_file"
                };
                if !operations
                    .iter()
                    .any(|value| value.as_str() == Some(operation))
                {
                    operations.push(json!(operation));
                }
            }
            if let Some(start_line) = details.get("start_line").and_then(|value| value.as_u64()) {
                let end_line = details
                    .get("new_string")
                    .or_else(|| details.get("content"))
                    .and_then(|value| value.as_str())
                    .map(|text| text.lines().count())
                    .filter(|count| *count > 0)
                    .map(|count| start_line + count.saturating_sub(1) as u64)
                    .unwrap_or(start_line);
                if let Some(line_ranges) = entry
                    .get_mut("line_ranges")
                    .and_then(|value| value.as_array_mut())
                {
                    line_ranges.push(json!({
                        "start_line": start_line,
                        "end_line": end_line
                    }));
                }
            }
            if let Some(summary) = metadata.get("summary").and_then(|value| value.as_str()) {
                if let Some(summaries) = entry
                    .get_mut("summaries")
                    .and_then(|value| value.as_array_mut())
                {
                    if !summary.trim().is_empty()
                        && !summaries
                            .iter()
                            .any(|value| value.as_str() == Some(summary))
                    {
                        summaries.push(json!(summary));
                    }
                }
            }
        }
        files.into_values().collect()
    }

    fn review_payload_previous_results(&self) -> Vec<Value> {
        self.context
            .messages_since_last_completion()
            .into_iter()
            .filter_map(|message| {
                let metadata = message.metadata?;
                if metadata
                    .get("message_kind")
                    .and_then(|value| value.as_str())
                    != Some("final_review_feedback")
                {
                    return None;
                }

                let verdict = metadata.get("review_verdict")?.clone();
                let summary = metadata
                    .get("review_summary")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                Some(json!({
                    "sub_agent_id": metadata.get("sub_agent_id").cloned().unwrap_or(Value::Null),
                    "summary": summary,
                    "verdict": verdict
                }))
            })
            .collect()
    }

    fn structured_tool_arguments(metadata: &Value) -> Option<Value> {
        let tool_call = metadata.get("tool_call")?;
        let function = tool_call.get("function").unwrap_or(tool_call);
        let arguments = function
            .get("arguments")
            .or_else(|| function.get("input"))?;
        match arguments {
            Value::Object(_) => Some(arguments.clone()),
            Value::String(raw) => serde_json::from_str(raw).ok(),
            _ => None,
        }
    }

    fn is_verification_command(command: &str) -> bool {
        let command = command.to_ascii_lowercase();
        [
            "cargo check",
            "cargo test",
            "cargo clippy",
            "cargo fmt --check",
            "npm test",
            "npm run test",
            "npm run lint",
            "npm run build",
            "pnpm test",
            "pnpm lint",
            "pnpm build",
            "yarn test",
            "yarn lint",
            "yarn build",
            "go test",
            "pytest",
            "vitest",
            "jest",
            "ruff check",
            "tsc --noemit",
        ]
        .iter()
        .any(|needle| command.contains(needle))
    }

    fn bounded_evidence_excerpt(value: Option<&str>) -> Option<String> {
        const MAX_CHARS: usize = 2_000;
        let value = value.map(str::trim).filter(|value| !value.is_empty())?;
        let mut excerpt = value.chars().take(MAX_CHARS).collect::<String>();
        if value.chars().count() > MAX_CHARS {
            excerpt.push_str("\n...[truncated]");
        }
        Some(excerpt)
    }

    pub(crate) fn build_final_review_evidence(messages: &[WorkflowMessage]) -> Value {
        let mut mutation_ledger = Vec::new();
        let mut verification_ledger = Vec::new();
        let mut failed_actions = Vec::new();

        for message in messages.iter().filter(|message| message.role == "tool") {
            let Some(metadata) = message.metadata.as_ref() else {
                continue;
            };
            let Some(tool_name) = metadata.get("tool_name").and_then(Value::as_str) else {
                continue;
            };
            let execution_status = metadata
                .get("execution_status")
                .and_then(Value::as_str)
                .unwrap_or(if message.is_error {
                    "failed"
                } else {
                    "completed"
                });
            let summary = metadata
                .get("summary")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let result_excerpt =
                Self::bounded_evidence_excerpt(metadata.get("llm_content").and_then(Value::as_str));

            if matches!(tool_name, TOOL_EDIT_FILE | TOOL_WRITE_FILE) {
                let details = metadata.get("details");
                mutation_ledger.push(json!({
                    "message_id": message.id,
                    "tool_name": tool_name,
                    "status": execution_status,
                    "path": details
                        .and_then(|value| value.get("file_path").or_else(|| value.get("path")))
                        .cloned()
                        .unwrap_or(Value::Null),
                    "summary": summary,
                    "details": details.cloned().unwrap_or(Value::Null)
                }));
            }

            let arguments = Self::structured_tool_arguments(metadata);
            let command = arguments
                .as_ref()
                .and_then(|value| value.get("command").or_else(|| value.get("cmd")))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if command.is_some_and(Self::is_verification_command) {
                verification_ledger.push(json!({
                    "message_id": message.id,
                    "tool_name": tool_name,
                    "command": command,
                    "status": execution_status,
                    "summary": summary,
                    "result_excerpt": result_excerpt.clone()
                }));
            }

            if message.is_error || matches!(execution_status, "failed" | "rejected") {
                failed_actions.push(json!({
                    "message_id": message.id,
                    "tool_name": tool_name,
                    "status": execution_status,
                    "error_type": metadata
                        .get("error_type")
                        .cloned()
                        .or_else(|| message.error_type.as_ref().map(|value| json!(value)))
                        .unwrap_or(Value::Null),
                    "summary": summary,
                    "result_excerpt": result_excerpt
                }));
            }
        }

        json!({
            "mutation_ledger": mutation_ledger,
            "verification_ledger": verification_ledger,
            "failed_actions": failed_actions
        })
    }

    fn build_final_review_prompt(
        &self,
        completion_summary: &str,
        completion_report_provenance: &Value,
        todo_status_overrides: &HashMap<String, String>,
    ) -> String {
        let user_messages = self
            .context
            .current_user_messages_since_last_completion()
            .into_iter()
            .map(|message| {
                let mut payload = serde_json::Map::new();
                payload.insert("id".to_string(), json!(message.id));
                if !message.message.trim().is_empty() {
                    payload.insert("content".to_string(), json!(message.message));
                }
                if let Some(timestamp) = message.created_at {
                    payload.insert("timestamp".to_string(), json!(timestamp));
                }
                if let Some(attached_context) = message.attached_context {
                    if !attached_context.trim().is_empty() {
                        payload.insert("attached_context".to_string(), json!(attached_context));
                    }
                }
                if let Some(metadata) = message.metadata {
                    payload.insert("metadata".to_string(), metadata);
                }
                Value::Object(payload)
            })
            .collect::<Vec<_>>();
        let approved_plan = self.context.current_approved_plan_since_last_completion();
        let changed_files = self.review_payload_changed_files();
        let evidence_messages = self.context.messages_since_last_completion();
        let evidence = Self::build_final_review_evidence(&evidence_messages);
        let previous_review_results = self.review_payload_previous_results();
        let todo_status =
            if let Ok(store) = self.context.main_store.read() {
                store
                    .get_todo_list_for_workflow(&self.session_id)
                    .ok()
                    .map(|todos| {
                        todos
                            .into_iter()
                            .map(|todo| {
                                let todo_id = todo["id"].as_str().unwrap_or_default().to_string();
                                let status =
                                    todo_status_overrides.get(&todo_id).cloned().unwrap_or_else(
                                        || todo["status"].as_str().unwrap_or_default().to_string(),
                                    );
                                json!({
                                    "id": todo_id,
                                    "subject": todo["subject"].as_str().unwrap_or_default(),
                                    "status": status
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
        let mut payload = serde_json::Map::new();
        payload.insert("review_type".to_string(), json!("final_code_review"));
        payload.insert("workflow_session_id".to_string(), json!(self.session_id));
        payload.insert("user_messages".to_string(), json!(user_messages));
        payload.insert("completion_report".to_string(), json!(completion_summary));
        payload.insert(
            "completion_report_provenance".to_string(),
            completion_report_provenance.clone(),
        );
        payload.insert("changed_files".to_string(), json!(changed_files));
        for key in ["mutation_ledger", "verification_ledger", "failed_actions"] {
            payload.insert(
                key.to_string(),
                evidence.get(key).cloned().unwrap_or_else(|| json!([])),
            );
        }
        if let Some(approved_plan) = approved_plan {
            if !approved_plan.trim().is_empty() {
                payload.insert("approved_plan".to_string(), json!(approved_plan));
            }
        }
        if !todo_status.is_empty() {
            payload.insert("todo_status".to_string(), json!(todo_status));
        }
        if !previous_review_results.is_empty() {
            payload.insert(
                "previous_review_results".to_string(),
                json!(previous_review_results),
            );
        }
        let payload = Value::Object(payload);
        let payload_text =
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
        format!(
            "Review the parent workflow's completion package below. Perform an independent final review using your read-only code inspection tools.\n\n\
Return the final verdict ONLY by calling `submit_result`.\n\
- `submit_result.result` MUST be a JSON object with this schema:\n\
  {{\"approved\": boolean, \"summary\": string, \"findings\": [{{\"severity\": \"blocker|major|minor|info\", \"file\": string|null, \"detail\": string}}], \"required_fixes\": [string]}}\n\
- You will also receive `previous_review_results`; use them to avoid repeating the same rejected reasoning unless the parent has actually addressed those findings.\n\
- If the work should not be allowed to finish, set `approved` to false and provide concrete required fixes.\n\
- If the work is acceptable, set `approved` to true and keep findings minimal.\n\
- Do not edit files. Do not ask the parent to run `git diff`; inspect the code directly with read/search tools when needed.\n\n\
<final_review_package>\n{}\n</final_review_package>",
            payload_text
        )
    }

    async fn launch_final_review(
        &mut self,
        completion_summary: String,
        completion_report_provenance: Value,
        todo_status_overrides: &HashMap<String, String>,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        let reviewer_agent = {
            let store = self
                .context
                .main_store
                .read()
                .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
            store
                .get_agent(FINAL_REVIEWER_BUILTIN_AGENT_ID)
                .map_err(WorkflowEngineError::Db)?
                .ok_or_else(|| {
                    WorkflowEngineError::General(format!(
                        "Final reviewer agent '{}' not found",
                        FINAL_REVIEWER_BUILTIN_AGENT_ID
                    ))
                })?
        };
        let review_prompt = self.build_final_review_prompt(
            &completion_summary,
            &completion_report_provenance,
            todo_status_overrides,
        );
        let raw_result = spawn_call_sub_agent(
            self.sub_agent_factory.clone(),
            self.context.main_store.clone(),
            self.gateway.clone(),
            self.tsid_generator.clone(),
            self.session_id.clone(),
            reviewer_agent.clone(),
            "Final review",
            &review_prompt,
        )
        .await?;
        let task_id = raw_result
            .get("task_id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| WorkflowEngineError::General("Missing reviewer task_id".to_string()))?
            .to_string();
        self.sub_agent_id = Some(task_id.clone());
        if !self.sub_agent_sessions.iter().any(|id| id == &task_id) {
            self.sub_agent_sessions.push(task_id.clone());
        }
        self.pending_final_review = Some(crate::workflow::react::types::PendingFinalReview {
            sub_agent_id: task_id,
            completion_summary,
        });
        self.update_state(WorkflowState::AwaitingSubAgent).await?;
        Ok(Some(ReinforcedResult {
            content: serde_json::to_string(&raw_result).unwrap_or_else(|_| raw_result.to_string()),
            llm_content: None,
            title: "Final Review".to_string(),
            summary: "Waiting for final review".to_string(),
            is_error: false,
            error_type: None,
            display_type: "text".to_string(),
            approval_status: Some("pending".to_string()),
            observation_kind: None,
        }))
    }

    fn parse_json_value_lossy(raw: &str) -> Option<Value> {
        let cleaned = crate::libs::util::format_json_str(raw);
        if let Ok(parsed) = serde_json::from_str::<Value>(&cleaned) {
            return Some(parsed);
        }

        let start = cleaned
            .char_indices()
            .find(|(_, ch)| *ch == '{' || *ch == '[')
            .map(|(idx, _)| idx)?;
        let candidate = &cleaned[start..];
        for (idx, ch) in candidate.char_indices().rev() {
            if ch != '}' && ch != ']' {
                continue;
            }
            let slice = &candidate[..=idx];
            if let Ok(parsed) = serde_json::from_str::<Value>(slice) {
                return Some(parsed);
            }
        }

        None
    }

    fn normalize_final_review_verdict(raw_value: &Value) -> Value {
        let parsed = match raw_value {
            Value::Object(_) => Some(raw_value.clone()),
            Value::String(raw) => Self::parse_json_value_lossy(raw),
            _ => None,
        }
        .unwrap_or_else(|| {
            json!({
                "approved": false,
                "summary": raw_value.as_str().filter(|value| !value.trim().is_empty()).unwrap_or("Final review returned no structured verdict."),
                "findings": [],
                "required_fixes": []
            })
        });

        let findings = parsed
            .get("findings")
            .and_then(|value| value.as_array())
            .map(|items| {
                items.iter()
                    .filter_map(|item| {
                        let detail = item.get("detail").and_then(|value| value.as_str())?.trim();
                        if detail.is_empty() {
                            return None;
                        }
                        Some(json!({
                            "severity": item.get("severity").and_then(|value| value.as_str()).unwrap_or("major"),
                            "file": item.get("file").cloned().unwrap_or(Value::Null),
                            "detail": detail
                        }))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut required_fixes = parsed
            .get("required_fixes")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::trim))
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if required_fixes.is_empty() {
            required_fixes = findings
                .iter()
                .filter_map(|item| {
                    let severity = item
                        .get("severity")
                        .and_then(|value| value.as_str())
                        .unwrap_or("major");
                    (severity == "blocker" || severity == "major").then(|| {
                        item.get("detail")
                            .and_then(|value| value.as_str())
                            .unwrap_or_default()
                            .to_string()
                    })
                })
                .filter(|item| !item.is_empty())
                .collect();
        }

        let has_blocking_findings = findings.iter().any(|item| {
            matches!(
                item.get("severity").and_then(|value| value.as_str()),
                Some("blocker" | "major")
            )
        });
        let approved = parsed
            .get("approved")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
            && !has_blocking_findings
            && required_fixes.is_empty();
        let summary = parsed
            .get("summary")
            .and_then(|value| value.as_str())
            .unwrap_or(if approved {
                "Final review approved the completion report."
            } else {
                "Final review rejected the completion report."
            })
            .to_string();

        json!({
            "approved": approved,
            "summary": summary,
            "findings": findings,
            "required_fixes": required_fixes
        })
    }

    fn parse_final_review_verdict(result: &Value) -> (Value, bool, String, Vec<String>) {
        let candidate = result
            .get("result")
            .cloned()
            .or_else(|| {
                result
                    .get("structured_content")
                    .and_then(|value| value.get("result"))
                    .cloned()
            })
            .or_else(|| result.get("structured_content").cloned())
            .or_else(|| result.get("summary").cloned())
            .unwrap_or(Value::Null);
        let parsed = Self::normalize_final_review_verdict(&candidate);
        let approved = parsed
            .get("approved")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let summary = parsed
            .get("summary")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let required_fixes = parsed
            .get("required_fixes")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        (parsed, approved, summary, required_fixes)
    }

    pub(crate) async fn handle_final_review_completion(
        &mut self,
        sub_agent_id: &str,
        result: &Value,
    ) -> Result<bool, WorkflowEngineError> {
        let Some(pending) = self.pending_final_review.clone() else {
            return Ok(false);
        };
        if pending.sub_agent_id != sub_agent_id {
            return Ok(false);
        }

        let (review_verdict, approved, summary, required_fixes) =
            Self::parse_final_review_verdict(result);
        self.pending_final_review = None;
        self.pending_completion_reports.clear();
        self.sub_agent_sessions.retain(|id| id != sub_agent_id);
        if self.sub_agent_id.as_deref() == Some(sub_agent_id) {
            self.sub_agent_id = None;
        }

        if approved {
            let completion_tool_call_id = crate::ccproxy::get_tool_id();
            self.add_message_and_notify_internal(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                Some(StepType::Observe),
                false,
                None,
                Some(json!({
                    "tool_call_id": completion_tool_call_id.clone(),
                    "tool_name": TOOL_COMPLETE_WORKFLOW,
                    "title": "Complete Workflow",
                    "summary": pending.completion_summary,
                    "review_display_state": "final_review_approved",
                    "execution_status": "completed",
                    "is_error": false,
                    "display_type": "text"
                })),
            )
            .await?;
            self.record_task_completed(&completion_tool_call_id).await;
            self.update_state(WorkflowState::Completed).await?;
            return Ok(true);
        }

        let mut content = format!(
            "Final review rejected the completion.\nSummary: {}",
            summary
        );
        if !required_fixes.is_empty() {
            content.push_str("\nRequired fixes:\n");
            for item in required_fixes {
                content.push_str("- ");
                content.push_str(&item);
                content.push('\n');
            }
        }
        content.push_str("\n<SYSTEM_REMINDER>Address the review findings above before calling `complete_workflow` again. Update the implementation, verify the fixes, and then submit a new completion report.</SYSTEM_REMINDER>");

        self.add_message_and_notify_internal(
            "user".to_string(),
            content,
            None,
            None,
            Some(StepType::Observe),
            true,
            Some("FinalReviewRejected".to_string()),
            Some(json!({
                "message_kind": "final_review_feedback",
                "sub_agent_id": sub_agent_id,
                "review_display_state": "final_review_rejected",
                "review_summary": summary,
                "review_verdict": review_verdict
            })),
        )
        .await?;
        self.update_state(WorkflowState::Thinking).await?;
        Ok(true)
    }

    pub(crate) fn is_smart_mode_read_only_tool(name: &str) -> bool {
        matches!(
            name,
            crate::tools::TOOL_READ_FILE
                | crate::tools::TOOL_LIST_DIR
                | crate::tools::TOOL_GLOB
                | crate::tools::TOOL_GREP
                | crate::tools::TOOL_GIT_DIFF
                | crate::tools::TOOL_GIT_INSPECT
                | crate::tools::TOOL_WEB_SEARCH
                | crate::tools::TOOL_WEB_FETCH
        )
    }

    pub(crate) fn is_smart_mode_auto_approved_tool(name: &str) -> bool {
        Self::is_smart_mode_read_only_tool(name) || matches!(name, TOOL_EDIT_FILE | TOOL_WRITE_FILE)
    }

    pub(crate) fn smart_mode_approval_decision(
        name: &str,
        args: &serde_json::Value,
    ) -> SmartApprovalDecision {
        if Self::is_smart_mode_auto_approved_tool(name) {
            return SmartApprovalDecision::AutoApprove;
        }

        if name == TOOL_BASH {
            let command_str = args["command"].as_str().unwrap_or("").trim();
            if Self::is_smart_mode_read_only_shell_command(command_str)
                || Self::is_smart_mode_safe_build_shell_command(command_str)
            {
                return SmartApprovalDecision::AutoApprove;
            }

            return SmartApprovalDecision::ReviewByUser;
        }

        SmartApprovalDecision::ReviewWithAi
    }

    fn completion_report_text_without_reasoning(text_part: &str) -> String {
        match regex::Regex::new(
            r"(?is)<think>.*?</think>|<thought>.*?</thought>|<(?:think|thought)>.*\z",
        ) {
            Ok(pattern) => pattern.replace_all(text_part, "").into_owned(),
            Err(error) => {
                log::error!("Failed to compile completion-report reasoning pattern: {error}");
                String::new()
            }
        }
    }

    fn normalized_finish_task_summary(text_part: &str) -> String {
        Self::completion_report_text_without_reasoning(text_part)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter(|line| {
                let lower = line.to_ascii_lowercase();
                !matches!(
                    lower.as_str(),
                    "done" | "finished" | "complete" | "completed" | "task complete"
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn is_valid_finish_task_summary(text_part: &str) -> bool {
        let normalized = Self::normalized_finish_task_summary(text_part);
        if normalized.is_empty() {
            return false;
        }

        let non_whitespace_len = normalized.chars().filter(|c| !c.is_whitespace()).count();
        if non_whitespace_len < 32 {
            return false;
        }

        let meaningful_lines = normalized
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        let has_sentence_shape = normalized.contains('\n')
            || normalized.contains('。')
            || normalized.contains('.')
            || normalized.contains(':');

        meaningful_lines >= 2 || has_sentence_shape
    }

    fn completion_report_similarity_text(text: &str) -> Vec<char> {
        text.chars()
            .flat_map(char::to_lowercase)
            .filter(|character| character.is_alphanumeric())
            .collect()
    }

    fn completion_report_trigram_scores(left: &[char], right: &[char]) -> (f64, f64) {
        if left.len() < 3 || right.len() < 3 {
            return (0.0, 0.0);
        }

        let mut left_grams = HashMap::<[char; 3], usize>::new();
        let mut right_grams = HashMap::<[char; 3], usize>::new();
        for window in left.windows(3) {
            *left_grams
                .entry([window[0], window[1], window[2]])
                .or_default() += 1;
        }
        for window in right.windows(3) {
            *right_grams
                .entry([window[0], window[1], window[2]])
                .or_default() += 1;
        }

        let intersection = left_grams
            .iter()
            .map(|(gram, left_count)| {
                (*left_count).min(right_grams.get(gram).copied().unwrap_or_default())
            })
            .sum::<usize>();
        let left_total = left_grams.values().sum::<usize>();
        let right_total = right_grams.values().sum::<usize>();
        let dice = (2 * intersection) as f64 / (left_total + right_total) as f64;
        let containment = intersection as f64 / left_total.min(right_total) as f64;
        (dice, containment)
    }

    fn completion_report_code_literals(text: &str) -> HashSet<String> {
        text.split('`')
            .skip(1)
            .step_by(2)
            .map(|literal| {
                literal
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase()
            })
            .filter(|literal| !literal.is_empty() && !literal.contains('\n'))
            .collect()
    }

    fn completion_report_paths(literals: &HashSet<String>) -> HashSet<&str> {
        literals
            .iter()
            .map(String::as_str)
            .filter(|literal| {
                literal.starts_with('/')
                    || (literal.contains('/') && !literal.chars().any(char::is_whitespace))
                    || literal
                        .as_bytes()
                        .get(1..3)
                        .is_some_and(|suffix| suffix == b":/" || suffix == b":\\")
            })
            .collect()
    }

    fn completion_report_numeric_literals(literals: &HashSet<String>) -> HashSet<&str> {
        literals
            .iter()
            .map(String::as_str)
            .filter(|literal| {
                literal.bytes().any(|byte| byte.is_ascii_digit())
                    && literal
                        .bytes()
                        .any(|byte| matches!(byte, b'.' | b'%' | b':' | b'='))
            })
            .collect()
    }

    fn completion_report_verification_commands(literals: &HashSet<String>) -> HashSet<&str> {
        literals
            .iter()
            .map(String::as_str)
            .filter(|literal| Self::is_verification_command(literal))
            .collect()
    }

    fn completion_report_hashes(literals: &HashSet<String>) -> HashSet<&str> {
        literals
            .iter()
            .map(String::as_str)
            .filter(|literal| {
                (7..=64).contains(&literal.len())
                    && literal.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && literal.bytes().any(|byte| byte.is_ascii_digit())
                    && literal
                        .bytes()
                        .any(|byte| matches!(byte.to_ascii_lowercase(), b'a'..=b'f'))
            })
            .collect()
    }

    fn completion_report_claim_dictionary() -> &'static CompletionReportClaimDictionary {
        static CLAIMS: OnceLock<CompletionReportClaimDictionary> = OnceLock::new();
        CLAIMS.get_or_init(|| {
            let load = |key: &str| {
                let mut phrases = rust_i18n::available_locales!()
                    .into_iter()
                    .flat_map(|locale| {
                        rust_i18n::t!(key, locale = locale)
                            .split('|')
                            .map(|phrase| {
                                phrase
                                    .to_lowercase()
                                    .split_whitespace()
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            })
                            .filter(|phrase| !phrase.is_empty())
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                phrases.sort();
                phrases.dedup();
                phrases
            };

            CompletionReportClaimDictionary {
                verification: [
                    load("workflow.completion_report_claims.verification_passed"),
                    load("workflow.completion_report_claims.verification_failed"),
                    load("workflow.completion_report_claims.verification_not_run"),
                ],
                limitations: [
                    load("workflow.completion_report_claims.limitations_absent"),
                    load("workflow.completion_report_claims.limitations_present"),
                ],
            }
        })
    }

    fn completion_report_claim_flags(text: &str, claims: &[Vec<String>]) -> u8 {
        let mut searchable = text
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let mut flags = 0_u8;
        for (index, alternatives) in claims.iter().enumerate() {
            let matches = alternatives
                .iter()
                .filter(|alternative| searchable.contains(alternative.as_str()))
                .collect::<Vec<_>>();
            if !matches.is_empty() {
                flags |= 1 << index;
                for matched in matches {
                    searchable = searchable.replace(matched.as_str(), " ");
                }
            }
        }
        flags
    }

    fn completion_report_claims_conflict(left: &str, right: &str) -> bool {
        let claims = Self::completion_report_claim_dictionary();
        [&claims.verification[..], &claims.limitations[..]]
            .iter()
            .any(|claims| {
                let left_flags = Self::completion_report_claim_flags(left, claims);
                let right_flags = Self::completion_report_claim_flags(right, claims);
                left_flags != 0 && right_flags != 0 && left_flags & right_flags == 0
            })
    }

    fn completion_reports_are_similar(left: &str, right: &str) -> bool {
        let left_text = Self::completion_report_similarity_text(left);
        let right_text = Self::completion_report_similarity_text(right);
        let shorter_len = left_text.len().min(right_text.len());
        let longer_len = left_text.len().max(right_text.len());
        if shorter_len < 3 || shorter_len as f64 / (longer_len as f64) < 0.60 {
            return false;
        }

        let (dice, containment) = Self::completion_report_trigram_scores(&left_text, &right_text);
        let left_literals = Self::completion_report_code_literals(left);
        let right_literals = Self::completion_report_code_literals(right);

        if !left_literals.is_empty() && !right_literals.is_empty() {
            let shared_literals = left_literals.intersection(&right_literals).count();
            let literal_containment =
                shared_literals as f64 / left_literals.len().min(right_literals.len()) as f64;
            if literal_containment < 0.50 || dice < 0.52 || containment < 0.52 {
                return false;
            }
        } else if dice < 0.72 || containment < 0.75 {
            return false;
        }

        let left_paths = Self::completion_report_paths(&left_literals);
        let right_paths = Self::completion_report_paths(&right_literals);
        if !left_paths.is_empty() && !right_paths.is_empty() && left_paths != right_paths {
            return false;
        }

        let left_hashes = Self::completion_report_hashes(&left_literals);
        let right_hashes = Self::completion_report_hashes(&right_literals);
        if !left_hashes.is_empty() && !right_hashes.is_empty() && left_hashes != right_hashes {
            return false;
        }

        let left_numbers = Self::completion_report_numeric_literals(&left_literals);
        let right_numbers = Self::completion_report_numeric_literals(&right_literals);
        if !left_numbers.is_empty() && !right_numbers.is_empty() && left_numbers != right_numbers {
            return false;
        }

        let left_commands = Self::completion_report_verification_commands(&left_literals);
        let right_commands = Self::completion_report_verification_commands(&right_literals);
        if !left_commands.is_empty()
            && !right_commands.is_empty()
            && left_commands != right_commands
        {
            return false;
        }

        !Self::completion_report_claims_conflict(left, right)
    }

    fn resolve_completion_report(
        args: &serde_json::Value,
        text_part: &str,
        pending_reports: &[PendingCompletionReport],
        current_segment_id: i32,
    ) -> Result<ResolvedCompletionReport, &'static str> {
        let object = args
            .as_object()
            .ok_or("complete_workflow arguments must be an object")?;
        if object
            .keys()
            .any(|key| !matches!(key.as_str(), "report_source" | "summary"))
        {
            return Err("complete_workflow contains unsupported arguments");
        }
        let report_source = object.get("report_source").and_then(Value::as_str);
        let summary = object.get("summary").and_then(Value::as_str);
        if object.contains_key("summary") && summary.is_none() {
            return Err("complete_workflow summary must be a string");
        }
        match report_source {
            Some("assistant_message") if object.contains_key("summary") => {
                return Err("legacy assistant_message source cannot include summary");
            }
            Some("tool_argument") if summary.is_none() => {
                return Err("legacy tool_argument source requires summary");
            }
            Some("assistant_message" | "tool_argument") | None => {}
            Some(_) => return Err("legacy report_source is invalid"),
        }

        let mut candidates = Vec::<ResolvedCompletionReport>::new();

        if Self::is_valid_finish_task_summary(text_part) {
            candidates.push(ResolvedCompletionReport {
                content: Self::normalized_finish_task_summary(text_part),
                source_message_id: None,
                persist_as_message: false,
                recency: (1, 0, 0),
            });
        }

        for pending in pending_reports
            .iter()
            .filter(|pending| pending.segment_id == current_segment_id)
        {
            let normalized = Self::normalized_finish_task_summary(&pending.content);
            let expected = PendingCompletionReport::new(
                &normalized,
                pending.source_message_id,
                pending.segment_id,
                pending.created_at_step,
            );
            if Self::is_valid_finish_task_summary(&pending.content)
                && expected.content_hash == pending.content_hash
            {
                candidates.push(ResolvedCompletionReport {
                    content: normalized,
                    source_message_id: pending.source_message_id,
                    persist_as_message: false,
                    recency: (
                        0,
                        pending.created_at_step,
                        pending.source_message_id.unwrap_or_default(),
                    ),
                });
            }
        }

        if let Some(summary) = summary {
            if Self::is_valid_finish_task_summary(summary) {
                candidates.push(ResolvedCompletionReport {
                    content: Self::normalized_finish_task_summary(summary),
                    source_message_id: None,
                    persist_as_message: true,
                    recency: (2, 0, 0),
                });
            } else {
                return Err("complete_workflow summary is not a valid completion report");
            }
        }

        let mut unique = Vec::<ResolvedCompletionReport>::new();
        for candidate in candidates {
            if let Some(existing) = unique
                .iter_mut()
                .find(|existing| existing.content == candidate.content)
            {
                if existing.source_message_id.is_none() && candidate.source_message_id.is_some() {
                    existing.source_message_id = candidate.source_message_id;
                }
                existing.persist_as_message &= candidate.persist_as_message;
                existing.recency = existing.recency.max(candidate.recency);
            } else {
                unique.push(candidate);
            }
        }

        match unique.len() {
            0 => Err("no valid completion report is available"),
            1 => Ok(unique.remove(0)),
            _ => {
                let latest_index = unique
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, candidate)| candidate.recency)
                    .map(|(index, _)| index)
                    .ok_or("no valid completion report is available")?;
                let latest = &unique[latest_index];
                if unique.iter().enumerate().all(|(index, candidate)| {
                    index == latest_index
                        || Self::completion_reports_are_similar(&candidate.content, &latest.content)
                }) {
                    Ok(unique.swap_remove(latest_index))
                } else {
                    Err("multiple different completion reports are available")
                }
            }
        }
    }

    fn completion_report_rejection_reminder(
        reason: &str,
        has_current_segment_pending_report: bool,
    ) -> String {
        if reason == "no valid completion report is available"
            && !has_current_segment_pending_report
        {
            return "<SYSTEM_REMINDER>Completion rejected: no valid completion report is available. STATE: PENDING_COMPLETION_REPORT=false. An empty complete_workflow({}) call cannot succeed because neither the current response nor the runtime contains a valid report. NEXT ACTION: call complete_workflow exactly once with a non-empty `summary` containing:\nCompleted: <what you completed>.\nVerified: <what you checked, or what was not run>.\nRemaining: <remaining limitations, or state that none remain>.\nUse `complete_workflow({\"summary\":\"...\"})`. Do not retry with an empty object. Do not send the report as a separate text-only response.</SYSTEM_REMINDER>".to_string();
        }

        if reason == "no valid completion report is available" {
            return "<SYSTEM_REMINDER>Completion rejected: stored completion draft data exists for the current segment, but no draft passed validation. STATE: PENDING_COMPLETION_REPORT=invalid. Repeating an empty complete_workflow({}) call cannot succeed. NEXT ACTION: use the next concrete task-relevant work or verification tool to invalidate the unusable draft. After that work is complete, call complete_workflow with one fresh non-empty `summary` covering completed work, verification, and remaining limitations. Do not reuse the invalid draft.</SYSTEM_REMINDER>".to_string();
        }

        if reason == "multiple different completion reports are available" {
            return "<SYSTEM_REMINDER>Completion rejected: multiple conflicting completion reports are available for the current segment. STATE: COMPLETION_REPORT=ambiguous. Repeating complete_workflow or adding another report cannot resolve this conflict. NEXT ACTION: continue with the next concrete task-relevant work or verification tool so the stale drafts are invalidated. When the work is complete, call complete_workflow with one fresh non-empty `summary` covering completed work, verification, and remaining limitations.</SYSTEM_REMINDER>".to_string();
        }

        if has_current_segment_pending_report {
            format!(
                "<SYSTEM_REMINDER>Completion rejected because the complete_workflow call was invalid: {reason}. STATE: PENDING_COMPLETION_REPORT=true. NEXT ACTION: emit no visible report, omit `summary`, and call complete_workflow({{}}) exactly once to commit the already captured draft. Do not repeat or replace that draft.</SYSTEM_REMINDER>"
            )
        } else {
            format!(
                "<SYSTEM_REMINDER>Completion rejected because the complete_workflow call was invalid: {reason}. STATE: PENDING_COMPLETION_REPORT=false. NEXT ACTION: call complete_workflow exactly once with a non-empty `summary` covering completed work, verification, and remaining limitations. Do not retry with an empty object.</SYSTEM_REMINDER>"
            )
        }
    }

    pub(crate) fn completion_turn_has_incompatible_tools(tool_names: &[&str]) -> bool {
        tool_names.contains(&TOOL_COMPLETE_WORKFLOW)
            && tool_names.iter().any(|name| {
                !matches!(
                    *name,
                    TOOL_COMPLETE_WORKFLOW | crate::tools::TOOL_TODO_UPDATE
                )
            })
    }

    fn is_read_only_shell_stage(stage: &str) -> bool {
        let stage = stage.trim();
        if stage.is_empty() {
            return true;
        }

        let stage = stage
            .replace("2>&1", " ")
            .replace("1>&2", " ")
            .replace("2>/dev/null", " ")
            .replace("2> /dev/null", " ")
            .replace("1>/dev/null", " ")
            .replace("1> /dev/null", " ")
            .replace("&>/dev/null", " ");
        let stage = stage.trim().to_lowercase();
        if stage.is_empty() {
            return true;
        }

        READ_ONLY_BASH_CMDS_EXACT.contains(stage.as_str())
            || READ_ONLY_BASH_PREFIXES
                .iter()
                .any(|&prefix| stage.starts_with(prefix))
    }

    fn has_shell_redirection(stage: &str) -> bool {
        let stage = stage
            .replace("2>&1", " ")
            .replace("1>&2", " ")
            .replace("2>/dev/null", " ")
            .replace("2> /dev/null", " ")
            .replace("1>/dev/null", " ")
            .replace("1> /dev/null", " ")
            .replace("&>/dev/null", " ");

        stage.contains('>') || stage.contains('<')
    }

    fn is_safe_shell_filter(stage: &str) -> bool {
        let stage = stage.trim().to_lowercase();
        if stage.is_empty() {
            return true;
        }

        const SAFE_FILTER_PREFIXES: &[&str] = &[
            "tail ", "head ", "grep ", "egrep ", "fgrep ", "less", "more", "sed ", "awk ", "cut ",
            "sort ", "uniq ", "wc ", "tr ", "jq ",
        ];

        SAFE_FILTER_PREFIXES
            .iter()
            .any(|&prefix| stage.starts_with(prefix))
    }

    fn strip_workspace_navigation_prefix(command_str: &str) -> String {
        let mut candidate = command_str.trim().to_lowercase();
        if let Some(rest) = candidate
            .strip_prefix("cd ")
            .or_else(|| candidate.strip_prefix("pushd "))
        {
            for delimiter in ["&&", "||", ";"] {
                if let Some((_, rest)) = rest.split_once(delimiter) {
                    candidate = rest.trim().to_string();
                    break;
                }
            }
        }
        candidate
    }

    fn is_safe_package_build_stage(stage: &str) -> bool {
        let tokens = match shlex::split(stage) {
            Some(tokens) => tokens,
            None => return false,
        };
        if tokens.is_empty() {
            return false;
        }

        match tokens[0].as_str() {
            "npm" | "pnpm" | "yarn" => tokens.get(1).map(String::as_str) == Some("build"),
            _ => false,
        }
    }

    fn is_smart_mode_safe_build_shell_command(command_str: &str) -> bool {
        let candidate = Self::strip_workspace_navigation_prefix(command_str);
        if candidate.is_empty() {
            return false;
        }

        for segment in candidate.split("&&") {
            for segment in segment.split("||") {
                for segment in segment.split(';') {
                    let segment = segment.trim();
                    if segment.is_empty() {
                        continue;
                    }

                    let mut stage_iter = segment.split('|');
                    let first_stage = stage_iter.next().unwrap_or("").trim();
                    if Self::has_shell_redirection(first_stage) {
                        return false;
                    }
                    if !Self::is_safe_package_build_stage(first_stage) {
                        return false;
                    }

                    for stage in stage_iter {
                        if Self::has_shell_redirection(stage) {
                            return false;
                        }
                        if !Self::is_safe_shell_filter(stage) {
                            return false;
                        }
                    }
                }
            }
        }

        true
    }

    fn is_smart_mode_read_only_shell_command(command_str: &str) -> bool {
        let candidate = Self::strip_workspace_navigation_prefix(command_str);
        if candidate.is_empty() {
            return false;
        }

        for segment in candidate.split("&&") {
            for segment in segment.split("||") {
                for segment in segment.split(';') {
                    let segment = segment.trim();
                    if segment.is_empty() {
                        continue;
                    }

                    let mut stage_iter = segment.split('|');
                    let first_stage = stage_iter.next().unwrap_or("").trim();
                    if Self::has_shell_redirection(first_stage) {
                        return false;
                    }
                    if !Self::is_read_only_shell_stage(first_stage) {
                        return false;
                    }

                    for stage in stage_iter {
                        if Self::has_shell_redirection(stage) {
                            return false;
                        }
                        if !Self::is_safe_shell_filter(stage) {
                            return false;
                        }
                    }
                }
            }
        }

        true
    }

    pub(crate) async fn review_tool_call_for_smart_mode(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        assistant_text: &str,
    ) -> Result<Option<ToolApprovalReview>, WorkflowEngineError> {
        let tool = match self.tool_manager.get_tool(tool_name).await {
            Ok(tool) => tool,
            Err(error) => {
                log::warn!(
                    "WorkflowExecutor {}: Skipping AI approval review for unknown tool '{}': {}",
                    self.session_id,
                    tool_name,
                    error
                );
                return Ok(None);
            }
        };

        match self
            .intelligence_manager
            .review_tool_approval(
                &self.context,
                &self.llm_processor.build_workspace_context(),
                tool_name,
                &tool.category().to_string(),
                tool.scope().as_str(),
                tool.description(),
                args,
                assistant_text,
            )
            .await
        {
            Ok(review) => {
                log::info!(
                    "WorkflowExecutor {}: Smart approval AI review for '{}' -> approved={}, risk_level={}, reason={}",
                    self.session_id,
                    tool_name,
                    review.approved,
                    review.risk_level,
                    review.reason
                );
                Ok(Some(review))
            }
            Err(error) => {
                log::warn!(
                    "WorkflowExecutor {}: AI approval review failed for '{}': {}",
                    self.session_id,
                    tool_name,
                    error
                );
                Ok(None)
            }
        }
    }

    /// Determines if a tool call should be intercepted for user approval based on the current ApprovalLevel.
    pub(crate) fn should_intercept_for_approval(
        &mut self,
        name: &str,
        args: &serde_json::Value,
    ) -> bool {
        if crate::tools::is_auto_execute_workflow_tool(name) {
            return false;
        }

        // Full mode never intercepts
        if self.policy.approval_level == ApprovalLevel::Full {
            return false;
        }

        // MCP tool schema loading is a built-in discovery step and should not require approval.
        if name == crate::tools::TOOL_MCP_TOOL_LOAD {
            return false;
        }

        // If already in auto_approve list, don't intercept
        if self.auto_approve.contains(name) {
            return false;
        }

        if name == TOOL_BASH {
            let command_str = args["command"].as_str().unwrap_or("").trim();
            if !command_str.is_empty() && self.smart_approved_bash_commands.remove(command_str) {
                log::info!(
                    "WorkflowExecutor {}: Skipping approval for Smart-AI-approved bash command: {}",
                    self.session_id,
                    command_str
                );
                return false;
            }
        }

        // Special handling for bash: Check shell policy first
        // If a command is explicitly allowed in shell_policy (e.g. via Approve All), don't intercept
        if name == TOOL_BASH {
            let command_str = args["command"].as_str().unwrap_or("").trim();
            if !command_str.is_empty() {
                let custom_rules: Vec<crate::tools::ShellPolicyRule> = self
                    .agent_config
                    .shell_policy
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_default();
                let policy_engine =
                    crate::tools::ShellPolicyEngine::new(self.path_guard.clone(), custom_rules);

                if let crate::tools::ShellDecision::Allow =
                    policy_engine.check(command_str, self.policy.phase == ExecutionPhase::Planning)
                {
                    log::info!(
                        "WorkflowExecutor {}: Auto-approving bash command allowed by policy: {}",
                        self.session_id,
                        command_str
                    );
                    return false;
                }
            }
        }

        // Smart mode: allow read/write tools, intercept risky bash or unknown mutations
        if self.policy.approval_level == ApprovalLevel::Smart {
            match Self::smart_mode_approval_decision(name, args) {
                SmartApprovalDecision::AutoApprove => {
                    if name == TOOL_BASH {
                        let command_str = args["command"].as_str().unwrap_or("").trim();
                        log::info!(
                            "WorkflowExecutor {}: Skipping approval for Smart-mode read-only bash command: {}",
                            self.session_id,
                            command_str
                        );
                    }
                    return false;
                }
                SmartApprovalDecision::ReviewWithAi | SmartApprovalDecision::ReviewByUser => {
                    if name == TOOL_BASH {
                        let command_str =
                            args["command"].as_str().unwrap_or("").trim().to_lowercase();
                        let has_operators = command_str
                            .chars()
                            .any(|c| matches!(c, '>' | '<' | '|' | '&' | ';'));
                        if has_operators {
                            log::info!(
                                "WorkflowExecutor {}: Intercepting bash due to shell operators: {}",
                                self.session_id,
                                command_str
                            );
                        }
                    }
                    return true;
                }
            }
        }

        // Default mode: intercept everything else
        true
    }

    pub(crate) async fn handle_submit_plan_intercept(
        &mut self,
        id: &str,
        args: &serde_json::Value,
        _text_part: &str,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        let plan_from_args = args
            .get("plan")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim();

        if plan_from_args.is_empty() {
            return Ok(Some(ReinforcedResult {
                content: "<SYSTEM_REMINDER>Error: You called 'submit_plan' without a non-empty `plan` argument. The approved plan MUST come from the structured tool argument `submit_plan.plan`, not from free-form assistant text. Put the complete plan in `plan` and call `submit_plan` again.</SYSTEM_REMINDER>".into(),
                llm_content: None,
                title: "SubmitPlan Error".to_string(),
                summary: "Missing plan payload".to_string(),
                is_error: true,
                error_type: Some("MissingPlan".into()),
                display_type: "text".to_string(),
                approval_status: None,
                observation_kind: None,
            }));
        }

        self.handle_approval_interception(id, TOOL_SUBMIT_PLAN, args, None)
            .await
    }

    pub(crate) async fn handle_ask_user_intercept(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        let groups = if let Some(groups) = args.as_array() {
            groups.clone()
        } else if args.get("title").is_some() || args.get("options").is_some() {
            vec![args.clone()]
        } else if let Some(groups) = args.get("items").and_then(|value| value.as_array()) {
            groups.clone()
        } else if let Some(groups) = args.get("groups").and_then(|value| value.as_array()) {
            groups.clone()
        } else if let (Some(question), Some(options)) = (
            args.get("question").and_then(|value| value.as_str()),
            args.get("options").and_then(|value| value.as_array()),
        ) {
            vec![json!({
                "title": question,
                "options": options
            })]
        } else {
            Vec::new()
        };

        let normalized_groups: Vec<serde_json::Value> = groups
            .into_iter()
            .filter_map(|group| {
                let title = group
                    .get("title")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())?;

                let options: Vec<String> = group
                    .get("options")
                    .and_then(|value| value.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();

                if options.is_empty() {
                    return None;
                }

                Some(json!({
                    "title": title,
                    "options": options
                }))
            })
            .collect();

        if normalized_groups.is_empty() {
            return Ok(Some(ReinforcedResult {
                content: "<SYSTEM_REMINDER>Error: 'ask_user' requires grouped choices with at least one valid item. Use {\"items\":[{\"title\":\"...\",\"options\":[\"...\"]}]} and ensure every group has a direct decision title plus at least one concise, actionable option. Use ask_user only for blocking decisions required to continue; do not use it for status updates, generic feedback, final answers, or plan approval. Do not include custom-input placeholder options because the UI already provides custom text input.</SYSTEM_REMINDER>".to_string(),
                llm_content: None,
                title: "Ask User Error".to_string(),
                summary: "Invalid ask_user payload".to_string(),
                is_error: true,
                error_type: Some("InvalidAskUserPayload".to_string()),
                display_type: "text".to_string(),
                approval_status: None,
                observation_kind: None,
            }));
        }

        self.update_state(WorkflowState::AwaitingUser).await?;
        let content =
            serde_json::to_string(&normalized_groups).unwrap_or_else(|_| "[]".to_string());

        Ok(Some(ReinforcedResult {
            content,
            llm_content: None,
            title: "Ask User".to_string(),
            summary: "Waiting for user".to_string(),
            is_error: false,
            error_type: None,
            display_type: "choice".to_string(),
            approval_status: None,
            observation_kind: None,
        }))
    }

    pub(crate) async fn handle_finish_task_intercept(
        &mut self,
        text_part: &str,
        args: &serde_json::Value,
        todo_status_overrides: &HashMap<String, String>,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        if self.implementation_completion_is_blocked() {
            log::warn!(
                "WorkflowExecutor {}: Rejected complete_workflow in phase {} because no successful implementation action followed the approved plan boundary",
                self.session_id,
                self.policy.phase
            );
            return Ok(Some(ReinforcedResult {
                content: "<SYSTEM_REMINDER>The approved plan has entered implementation, but no successful implementation action has been observed after the approval boundary. Do not complete the workflow yet. Start executing the approved plan with one concrete work tool; create a replacement execution todo list first when the plan has multiple units.</SYSTEM_REMINDER>".to_string(),
                llm_content: None,
                title: "Implementation Not Started".to_string(),
                summary: "Approved plan implementation has not started".to_string(),
                is_error: true,
                error_type: Some("ImplementationNotStarted".into()),
                display_type: "text".to_string(),
                approval_status: None,
                observation_kind: None,
            }));
        }

        let mut completion_report = match Self::resolve_completion_report(
            args,
            text_part,
            &self.pending_completion_reports,
            self.context.current_segment_id,
        ) {
            Ok(report) => report,
            Err(reason) => {
                let has_current_segment_pending_report = self
                    .pending_completion_reports
                    .iter()
                    .any(|pending| pending.segment_id == self.context.current_segment_id);
                return Ok(Some(ReinforcedResult {
                    content: Self::completion_report_rejection_reminder(
                        reason,
                        has_current_segment_pending_report,
                    ),
                    llm_content: None,
                    title: "FinishTask Error".to_string(),
                    summary: "Invalid completion report source".to_string(),
                    is_error: true,
                    error_type: Some("InvalidFinishSummary".into()),
                    display_type: "text".to_string(),
                    approval_status: None,
                    observation_kind: None,
                }));
            }
        };

        if completion_report.source_message_id.is_none() && !completion_report.persist_as_message {
            completion_report.source_message_id = self
                .context
                .messages
                .iter()
                .rev()
                .find(|message| {
                    message.role == "assistant"
                        && message.segment_id == self.context.current_segment_id
                        && Self::normalized_finish_task_summary(&message.message)
                            == completion_report.content
                })
                .and_then(|message| message.id);
        }

        if let Ok(store) = self.context.main_store.read() {
            if let Ok(todos) = store.get_todo_list_for_workflow(&self.session_id) {
                let active_tasks: Vec<String> = todos
                    .iter()
                    .filter(|t| {
                        let todo_id = t["id"].as_str().unwrap_or("");
                        let s = todo_status_overrides
                            .get(todo_id)
                            .map(String::as_str)
                            .unwrap_or_else(|| t["status"].as_str().unwrap_or(""));
                        s == "pending" || s == "in_progress"
                    })
                    .map(|t| {
                        let todo_id = t["id"].as_str().unwrap_or("?");
                        let status = todo_status_overrides
                            .get(todo_id)
                            .map(String::as_str)
                            .unwrap_or_else(|| t["status"].as_str().unwrap_or("?"));
                        format!(
                            "[{}] {} (ID: {})",
                            status,
                            t["subject"].as_str().unwrap_or("Untitled"),
                            todo_id
                        )
                    })
                    .collect();

                if !active_tasks.is_empty() {
                    return Ok(Some(ReinforcedResult {
                        content: format!("<SYSTEM_REMINDER>Block: You still have active tasks: {}. Do NOT retry complete_workflow yet. Do NOT call sub_agent_output for call-mode sub-agents; their results are delivered directly as observations. You MUST either complete these todos, or mark them as 'failed' or 'data_missing' if they cannot be fulfilled, before calling complete_workflow.</SYSTEM_REMINDER>", active_tasks.join(", ")),
                        llm_content: None,
                        title: "Tasks Pending".to_string(),
                        summary: "Incomplete todos".to_string(),
                        is_error: true,
                        error_type: Some("PendingTodos".into()),
                        display_type: "text".to_string(),
                        approval_status: None,
                        observation_kind: None,
                    }));
                }
            }
        }

        let pending_source = self.pending_completion_reports.iter().find(|pending| {
            pending.segment_id == self.context.current_segment_id
                && pending.content_hash
                    == hex::encode(sha2::Sha256::digest(completion_report.content.as_bytes()))
                && pending.content == completion_report.content
        });
        let completion_report_provenance = json!({
            "source_message_id": completion_report.source_message_id,
            "source": if completion_report.persist_as_message {
                "tool_argument_summary"
            } else if pending_source.is_some() {
                "pending_assistant_message"
            } else if completion_report.source_message_id.is_some() {
                "current_assistant_message"
            } else {
                "current_assistant_response"
            },
            "content_hash": hex::encode(sha2::Sha256::digest(completion_report.content.as_bytes())),
            "segment_id": self.context.current_segment_id,
            "captured_at_step": pending_source.map(|pending| pending.created_at_step),
            "resolved_at_step": self.current_step
        });
        let completion_summary = self.persist_completion_report(completion_report).await?;
        self.pending_completion_reports.clear();
        self.next_llm_runtime_reminder = None;
        self.save_snapshot().await?;

        if self.final_review_mode_enabled() {
            log::info!(
                "WorkflowExecutor {}: Launching final reviewer sub-agent...",
                self.session_id
            );
            self.update_state(WorkflowState::Auditing).await?;
            return self
                .launch_final_review(
                    completion_summary,
                    completion_report_provenance,
                    todo_status_overrides,
                )
                .await;
        }

        Ok(Some(ReinforcedResult {
            content: TASK_FINISHED.to_string(),
            llm_content: None,
            title: "Complete Workflow".to_string(),
            summary: completion_summary,
            is_error: false,
            error_type: None,
            display_type: "text".to_string(),
            approval_status: None,
            observation_kind: None,
        }))
    }

    pub(crate) async fn handle_submit_result_intercept(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        let result = args
            .get("result")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or_default();
        let summary = args
            .get("summary")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or_default();

        if result.is_empty() || summary.is_empty() {
            return Ok(Some(ReinforcedResult {
                content: "<SYSTEM_REMINDER>Error: `submit_result` requires both a non-empty `result` and a non-empty `summary`. Put the full delegated output in `result` and a short notification-safe summary in `summary`, then call `submit_result` again.</SYSTEM_REMINDER>".into(),
                llm_content: None,
                title: "Submit Result Error".to_string(),
                summary: "Missing result payload".to_string(),
                is_error: true,
                error_type: Some("InvalidSubmitResult".into()),
                display_type: "text".to_string(),
                approval_status: None,
                observation_kind: None,
            }));
        }

        let sanitized_result = Self::completion_response_without_reasoning(result);
        let sanitized_summary = Self::completion_response_without_reasoning(summary);

        Ok(Some(ReinforcedResult {
            content: sanitized_result,
            llm_content: None,
            title: "Submit Result".to_string(),
            summary: sanitized_summary,
            is_error: false,
            error_type: None,
            display_type: "text".to_string(),
            approval_status: None,
            observation_kind: None,
        }))
    }

    pub(crate) async fn handle_bash_security_intercept(
        &mut self,
        id: &str,
        args: &serde_json::Value,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        let command_str = args["command"].as_str().unwrap_or("");
        if !self.auto_approve.contains(TOOL_BASH) {
            let custom_rules: Vec<crate::tools::ShellPolicyRule> = self
                .agent_config
                .shell_policy
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();
            let policy_engine =
                crate::tools::ShellPolicyEngine::new(self.path_guard.clone(), custom_rules);

            match policy_engine.check(command_str, self.policy.phase == ExecutionPhase::Planning) {
                crate::tools::ShellDecision::Allow => {}
                crate::tools::ShellDecision::Deny(reason) => {
                    return Ok(Some(ReinforcedResult {
                        content: format!(
                            "Command blocked by security policy: {}. You may try an alternative command, modify the approach, or ask the user to adjust the policy if this restriction is blocking a legitimate task.",
                            reason
                        ),
                        llm_content: None,
                        title: format!("Run({})", command_str),
                        summary: "Blocked".to_string(),
                        is_error: true,
                        error_type: Some("Security".to_string()),
                        display_type: "text".to_string(),
                        approval_status: None,
                        observation_kind: None,
                    }));
                }
                crate::tools::ShellDecision::Review(reason) => {
                    if self.policy.approval_level == ApprovalLevel::Full {
                        log::info!(
                            "WorkflowExecutor {}: Auto-approving risky bash command in Full mode (Policy: {})",
                            self.session_id, reason
                        );
                    } else if self.policy.approval_level == ApprovalLevel::Smart {
                        // In Smart mode, allow read-only diagnostic commands even if they use
                        // command chaining or output shaping to trim noisy output.
                        if Self::is_smart_mode_read_only_shell_command(command_str)
                            || Self::is_smart_mode_safe_build_shell_command(command_str)
                        {
                            log::info!(
                                "WorkflowExecutor {}: Auto-approving low-risk bash command in Smart mode: {}",
                                self.session_id, command_str
                            );
                            return Ok(None);
                        }

                        // In Smart mode, check if this is a read-only command before intercepting
                        let command_str_lower = command_str.to_lowercase();
                        let is_read_only = READ_ONLY_BASH_CMDS_EXACT
                            .contains(command_str_lower.as_str())
                            || READ_ONLY_BASH_PREFIXES
                                .iter()
                                .any(|&p| command_str_lower.starts_with(p));

                        if is_read_only {
                            log::info!(
                                "WorkflowExecutor {}: Auto-approving read-only bash command in Smart mode: {}",
                                self.session_id, command_str
                            );
                            // Don't intercept - allow the read-only command
                        } else {
                            if let Some(review) = self
                                .review_tool_call_for_smart_mode(TOOL_BASH, args, command_str)
                                .await?
                            {
                                if review.approved {
                                    self.smart_approved_bash_commands
                                        .insert(command_str.trim().to_string());
                                    log::info!(
                                        "WorkflowExecutor {}: AI approved bash command in Smart mode (risk: {}, reason: {})",
                                        self.session_id,
                                        review.risk_level,
                                        review.reason
                                    );
                                    return Ok(None);
                                }

                                log::info!(
                                    "WorkflowExecutor {}: AI did not auto-approve bash command in Smart mode (risk: {}, reason: {})",
                                    self.session_id,
                                    review.risk_level,
                                    review.reason
                                );
                            }

                            log::info!(
                                "WorkflowExecutor {}: Intercepting bash command for review in Smart mode: {}",
                                self.session_id, reason
                            );
                            let display_content = command_str.to_string();
                            return self
                                .handle_approval_interception(
                                    id,
                                    TOOL_BASH,
                                    args,
                                    Some(display_content),
                                )
                                .await;
                        }
                    } else {
                        log::info!(
                            "WorkflowExecutor {}: Intercepting bash command for review: {}",
                            self.session_id,
                            reason
                        );
                        // Delegate to unified approval handler with descriptive command preview
                        let display_content = command_str.to_string();
                        return self
                            .handle_approval_interception(
                                id,
                                TOOL_BASH,
                                args,
                                Some(display_content),
                            )
                            .await;
                    }
                }
            }
        }
        Ok(None)
    }

    pub(crate) async fn handle_approval_interception(
        &mut self,
        id: &str,
        name: &str,
        args: &serde_json::Value,
        display_content: Option<String>,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        // 1. Determine what to show the user in the UI (Generate Diffs for File Ops)
        let mut display_type = "text".to_string();
        let (content_str, details_value) = if let Some(custom) = display_content {
            (
                custom.clone(),
                normalize_preview_details(serde_json::json!(custom)),
            )
        } else {
            match name {
                TOOL_EDIT_FILE | TOOL_WRITE_FILE | TOOL_PLAN_EDIT_NOTE | TOOL_PLAN_WRITE_NOTE => {
                    display_type = "diff".to_string();
                    let mut preview_args = args.clone();
                    let primary_root =
                        self.path_guard.read().ok().and_then(|guard| {
                            guard.get_primary_root().map(|path| path.to_path_buf())
                        });
                    if name == TOOL_WRITE_FILE {
                        attach_write_file_overwrite_old_content(
                            &mut preview_args,
                            primary_root.as_deref(),
                        );
                    }
                    attach_display_context(&mut preview_args, false, primary_root.as_deref());
                    let normalized_details = normalize_preview_details(preview_args);
                    (
                        render_preview_details_text(&normalized_details, &display_type),
                        normalized_details,
                    )
                }
                TOOL_BASH => {
                    let command = args
                        .get("command")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let preview = if command.is_empty() {
                        serde_json::to_string_pretty(args).unwrap_or_default()
                    } else {
                        command.clone()
                    };
                    (
                        preview.clone(),
                        serde_json::json!({
                            "command": command,
                        }),
                    )
                }
                TOOL_SUBMIT_PLAN => {
                    display_type = "markdown".to_string();
                    let plan = args
                        .get("plan")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let preview = if plan.is_empty() {
                        serde_json::to_string_pretty(args).unwrap_or_default()
                    } else {
                        plan
                    };
                    (
                        preview.clone(),
                        normalize_preview_details(serde_json::json!(preview)),
                    )
                }
                _ => {
                    let msg = format!(
                        "Tool: {}\nArguments: {}",
                        name,
                        serde_json::to_string_pretty(args).unwrap_or_default()
                    );
                    (
                        msg.clone(),
                        normalize_preview_details(serde_json::json!(msg)),
                    )
                }
            }
        };

        // 2. Stash the full tool name, arguments, and details for later request_confirm_broadcast
        let stash_obj = json!({
            "name": name,
            "arguments": args,
            "details": details_value.clone(),
            "display_type": display_type.clone()
        });
        self.pending_approvals.insert(id.to_string(), stash_obj);
        self.enqueue_pending_approval(id);

        self.update_state(WorkflowState::AwaitingApproval).await?;

        let event = WorkflowEvent::approval_requested(
            self.session_id.clone(),
            id.to_string(),
            name.to_string(),
            args.clone(),
            Some(details_value.clone()),
            Some(display_type.clone()),
        );
        if let Err(e) = self.append_event(&event) {
            log::error!(
                "[Workflow][session={}] workflow.event.append_failed - error={}",
                self.session_id,
                e
            );
        }

        self.gateway
            .send(
                &self.session_id,
                GatewayPayload::Confirm {
                    id: id.to_string(),
                    action: name.to_string(),
                    tool_name: name.to_string(),
                    arguments: args.clone(),
                    details: details_value,
                    display_type: Some(display_type.clone()),
                },
            )
            .await?;

        // 4. Return a 'waiting' result to the engine loop.
        // Use standard title generation to match the UI screenshot provided.
        let pretty_title = {
            let primary_root = self
                .path_guard
                .read()
                .unwrap()
                .get_primary_root()
                .map(|p| p.to_path_buf());
            ObservationReinforcer::generate_title(name, args, None, primary_root.as_deref())
        };

        Ok(Some(ReinforcedResult {
            content: content_str,
            llm_content: None,
            title: pretty_title,
            summary: rust_i18n::t!("workflow.awaiting_approval").to_string(),
            is_error: false,
            error_type: None,
            display_type,
            approval_status: Some("pending".to_string()),
            observation_kind: None,
        }))
    }

    pub(crate) fn handle_fs_path_guard_intercept(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        if let Some(path_str) = args["file_path"].as_str().or_else(|| args["path"].as_str()) {
            let guard = self.path_guard.read().map_err(|e| {
                WorkflowEngineError::General(format!("PathGuard lock poisoned: {}", e))
            })?;

            let is_write = [TOOL_WRITE_FILE, TOOL_EDIT_FILE].contains(&name);
            let is_planning = self.policy.phase == ExecutionPhase::Planning;

            // 1. Validate security boundaries
            if let Err(e) =
                guard.validate(std::path::Path::new(path_str), is_planning, is_write, false)
            {
                return Ok(Some(ReinforcedResult {
                    content: format!("Security Error: {}\n<SYSTEM_REMINDER>Access to this path is denied. If this path is essential, please ask the user to add it to the 'Authorized Paths' in settings. Otherwise, try to use a different path or approach.</SYSTEM_REMINDER>", e),
                    llm_content: None,
                    title: format!("Security Check: {}", name),
                    summary: "Access Denied".to_string(),
                    is_error: true,
                    error_type: Some("Security".to_string()),
                    display_type: "text".to_string(),
                    approval_status: None,
                    observation_kind: None,
                }));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::{SmartApprovalDecision, WorkflowExecutor};
    use crate::db::WorkflowMessage;
    use crate::tools::{TOOL_BASH, TOOL_EDIT_FILE, TOOL_READ_FILE, TOOL_WRITE_FILE};
    use crate::workflow::react::policy::ExecutionPhase;
    use crate::workflow::react::types::PendingCompletionReport;
    use serde_json::json;

    fn report(text: &str, message_id: i64, segment_id: i32) -> PendingCompletionReport {
        report_at(text, message_id, segment_id, 7)
    }

    fn report_at(
        text: &str,
        message_id: i64,
        segment_id: i32,
        created_at_step: usize,
    ) -> PendingCompletionReport {
        PendingCompletionReport::new(text, Some(message_id), segment_id, created_at_step)
    }

    #[test]
    fn finish_task_summary_rejects_placeholder_text() {
        assert!(!WorkflowExecutor::is_valid_finish_task_summary("done"));
        assert!(!WorkflowExecutor::is_valid_finish_task_summary(
            "<think>looks good</think>"
        ));
        assert!(!WorkflowExecutor::is_valid_finish_task_summary("Completed"));
    }

    #[test]
    fn finish_task_summary_accepts_meaningful_user_visible_report() {
        assert!(WorkflowExecutor::is_valid_finish_task_summary(
            "Implemented the workflow fix and verified it with cargo check.\nRemaining note: UI behavior still needs manual confirmation."
        ));
    }

    #[test]
    fn finish_task_summary_ignores_reasoning_blocks() {
        let report = "Completed the workflow change and verified the targeted tests passed.\nNo known limitations remain.";
        let long_reasoning = "This is a sufficiently long internal reasoning block. It must never be treated as a user-visible completion report.";

        assert!(!WorkflowExecutor::is_valid_finish_task_summary(&format!(
            "Progress update\n<think>{long_reasoning}</think>"
        )));
        assert!(!WorkflowExecutor::is_valid_finish_task_summary(&format!(
            "<thought>\n{long_reasoning}\n</thought>"
        )));
        assert_eq!(
            WorkflowExecutor::normalized_finish_task_summary(&format!(
                "<think>{long_reasoning}</think>\n{report}\n<thought>{long_reasoning}</thought>"
            )),
            report
        );
    }

    #[test]
    fn completion_report_resolution_rejects_reasoning_without_a_pending_report() {
        let reasoning = "<think>This is a sufficiently long internal reasoning block. It must never be treated as a user-visible completion report.</think>";

        assert!(
            WorkflowExecutor::resolve_completion_report(&json!({}), reasoning, &[], 1,).is_err()
        );
    }

    #[test]
    fn missing_completion_report_reminder_requires_summary_argument() {
        let reminder = WorkflowExecutor::completion_report_rejection_reminder(
            "no valid completion report is available",
            false,
        );

        assert!(reminder.contains("PENDING_COMPLETION_REPORT=false"));
        assert!(reminder.contains("Completed: <what you completed>."));
        assert!(reminder.contains("Verified: <what you checked, or what was not run>."));
        assert!(reminder.contains("Remaining: <remaining limitations"));
        assert!(reminder.contains("complete_workflow({\"summary\":\"...\"})"));
        assert!(reminder.contains("Do not retry with an empty object"));
        assert!(reminder.contains("Do not send the report as a separate text-only response"));
        assert!(!reminder.contains("If a pending report was captured"));
    }

    #[test]
    fn invalid_pending_completion_report_reminder_prohibits_empty_retry() {
        let reminder = WorkflowExecutor::completion_report_rejection_reminder(
            "no valid completion report is available",
            true,
        );

        assert!(reminder.contains("PENDING_COMPLETION_REPORT=invalid"));
        assert!(reminder.contains("Repeating an empty complete_workflow({}) call cannot succeed"));
        assert!(reminder.contains("one fresh non-empty `summary`"));
        assert!(!reminder.contains("call complete_workflow with no visible text"));
    }

    #[test]
    fn invalid_completion_arguments_use_the_actual_pending_state() {
        let with_pending = WorkflowExecutor::completion_report_rejection_reminder(
            "complete_workflow contains unsupported arguments",
            true,
        );
        let without_pending = WorkflowExecutor::completion_report_rejection_reminder(
            "complete_workflow contains unsupported arguments",
            false,
        );

        assert!(with_pending.contains("PENDING_COMPLETION_REPORT=true"));
        assert!(with_pending.contains("emit no visible report"));
        assert!(with_pending.contains("omit `summary`"));
        assert!(without_pending.contains("PENDING_COMPLETION_REPORT=false"));
        assert!(without_pending.contains("non-empty `summary`"));
        assert!(without_pending.contains("Do not retry with an empty object"));
    }

    #[test]
    fn completion_report_resolution_accepts_current_report_after_reasoning() {
        let report = "Completed the workflow change and verified the targeted tests passed.\nNo known limitations remain.";
        let text_part = format!(
            "<thought>Internal analysis that must not be included in the report.</thought>\n{report}"
        );

        let resolved = WorkflowExecutor::resolve_completion_report(&json!({}), &text_part, &[], 1)
            .expect("current report should resolve");

        assert_eq!(resolved.content, report);
        assert_eq!(resolved.source_message_id, None);
    }

    #[test]
    fn completion_report_resolution_uses_the_only_pending_report() {
        let full_report = "Implemented the requested workflow fix.\nVerified the focused tests and found no remaining limitations.";
        let pending = vec![report(full_report, 41, 3)];

        let resolved = WorkflowExecutor::resolve_completion_report(&json!({}), "", &pending, 3)
            .expect("pending report should resolve");

        assert_eq!(resolved.content, full_report);
        assert_eq!(resolved.source_message_id, Some(41));
    }

    #[test]
    fn completion_report_resolution_deduplicates_identical_current_and_pending_reports() {
        let full_report = "Implemented the requested workflow fix.\nVerified the focused tests and found no remaining limitations.";
        let pending = vec![report(full_report, 42, 3)];

        let resolved =
            WorkflowExecutor::resolve_completion_report(&json!({}), full_report, &pending, 3)
                .expect("identical reports should deduplicate");

        assert_eq!(resolved.content, full_report);
        assert_eq!(resolved.source_message_id, Some(42));
    }

    #[test]
    fn completion_report_resolution_rejects_different_current_and_pending_reports() {
        let full_report = "Implemented the requested workflow fix across the runtime.\nVerified all focused tests and found no remaining limitations.";
        let brief_report = "Implemented the workflow fix.\nThe focused verification passed.";
        let pending = vec![report(full_report, 43, 3)];

        let error =
            WorkflowExecutor::resolve_completion_report(&json!({}), brief_report, &pending, 3)
                .expect_err("different reports must be ambiguous");

        assert_eq!(error, "multiple different completion reports are available");
    }

    #[test]
    fn completion_report_resolution_uses_latest_similar_rewrite() {
        let earlier_report = "Created a global Git `pre-commit` hook at `/Users/test/.git-hooks/pre-commit` to reject staged `.db` files.\nVerified that database files are blocked, ordinary files are allowed, and the existing `commit-msg` hook still runs. No known limitations remain.";
        let latest_report = "Implemented `/Users/test/.git-hooks/pre-commit`, a global `pre-commit` hook that blocks staged `.db` files.\nVerification confirmed that `.db` commits fail, normal files pass, and the existing `commit-msg` hook remains active. No further action is required.";
        let pending = vec![report(earlier_report, 43, 3)];

        let resolved =
            WorkflowExecutor::resolve_completion_report(&json!({}), latest_report, &pending, 3)
                .expect("a related rewrite should resolve to the latest report");

        assert_eq!(resolved.content, latest_report);
        assert_eq!(resolved.source_message_id, None);
        assert!(!resolved.persist_as_message);
    }

    #[test]
    fn completion_report_resolution_uses_latest_similar_optional_summary() {
        let earlier_report = "Implemented `src/workflow.rs` so terminal todos can finish cleanly.\nThe focused completion tests and `cargo check` passed. No known limitations remain.";
        let latest_report = "Updated `src/workflow.rs` to complete workflows after all todos become terminal.\nVerified the focused completion suite and `cargo check`; both passed. No remaining limitations are known.";
        let pending = vec![report(earlier_report, 44, 3)];

        let resolved = WorkflowExecutor::resolve_completion_report(
            &json!({ "summary": latest_report }),
            "",
            &pending,
            3,
        )
        .expect("a compatible optional summary should resolve to the latest report");

        assert_eq!(resolved.content, latest_report);
        assert_eq!(resolved.source_message_id, None);
        assert!(resolved.persist_as_message);
    }

    #[test]
    fn completion_report_resolution_uses_latest_similar_pending_report() {
        let earlier_report = "Tested the `submit_plan` workflow in `/workspace/chatspeed`.\nThe plan was approved, `.gitignore` was read, and `src` plus `src-tauri` were listed successfully. No files were modified.";
        let latest_report = "Completed the `submit_plan` workflow test in `/workspace/chatspeed`.\nVerification covered plan approval, reading `.gitignore`, and listing `src` and `src-tauri`; every step passed without modifying files.";
        let pending = vec![
            report_at(earlier_report, 45, 3, 7),
            report_at(latest_report, 46, 3, 8),
        ];

        let resolved = WorkflowExecutor::resolve_completion_report(&json!({}), "", &pending, 3)
            .expect("similar pending drafts should resolve by capture order");

        assert_eq!(resolved.content, latest_report);
        assert_eq!(resolved.source_message_id, Some(46));
    }

    #[test]
    fn completion_report_resolution_rejects_similar_verification_conflicts() {
        for (earlier_report, latest_report) in [
            (
                "Updated `src/workflow.rs` to fix completion handling.\nThe focused completion tests passed. No known limitations remain.",
                "Updated `src/workflow.rs` to fix completion handling.\nThe focused completion tests failed. No known limitations remain.",
            ),
            (
                "已更新 `src/workflow.rs` 的完成处理逻辑。\n相关测试已经验证通过，目前没有已知限制。",
                "已更新 `src/workflow.rs` 的完成处理逻辑。\n相关测试尚未运行，目前没有已知限制。",
            ),
            (
                "Se actualizó `src/workflow.rs`.\nLas pruebas pasaron y no quedan limitaciones conocidas.",
                "Se actualizó `src/workflow.rs`.\nLas pruebas no se ejecutaron y no quedan limitaciones conocidas.",
            ),
            (
                "`src/workflow.rs` を更新しました。\nテストに合格し、既知の制限はありません。",
                "`src/workflow.rs` を更新しました。\nテストは実行されていません。既知の制限はありません。",
            ),
        ] {
            let pending = vec![report(earlier_report, 47, 3)];
            assert!(
                WorkflowExecutor::resolve_completion_report(
                    &json!({}),
                    latest_report,
                    &pending,
                    3,
                )
                .is_err(),
                "conflicting verification claims must remain ambiguous"
            );
        }
    }

    #[test]
    fn completion_report_resolution_rejects_similar_critical_literal_conflicts() {
        for (earlier_report, latest_report) in [
            (
                "Updated `/workspace/api/src/auth.rs` to correct token validation.\nThe focused authentication tests passed, and no known limitations remain.",
                "Updated `/workspace/web/src/auth.rs` to correct token validation.\nThe focused authentication tests passed, and no known limitations remain.",
            ),
            (
                "Updated `src/auth.rs` for release `2.0.8`.\nThe focused authentication tests passed, and no known limitations remain.",
                "Updated `src/auth.rs` for release `2.0.9`.\nThe focused authentication tests passed, and no known limitations remain.",
            ),
            (
                "Updated `src/auth.rs` for release `2.0.8`.\nThe focused authentication tests passed, and no known limitations remain.",
                "Updated `src/auth.rs` for release `20.8`.\nThe focused authentication tests passed, and no known limitations remain.",
            ),
            (
                "Updated `src/auth.rs` and verified it with `cargo test auth`.\nThe focused authentication tests passed, and no known limitations remain.",
                "Updated `src/auth.rs` and verified it with `cargo check`.\nThe focused authentication tests passed, and no known limitations remain.",
            ),
            (
                "Updated `src/auth.rs` in commit `abc1234`.\nThe focused authentication tests passed, and no known limitations remain.",
                "Updated `src/auth.rs` in commit `def5678`.\nThe focused authentication tests passed, and no known limitations remain.",
            ),
        ] {
            let pending = vec![report(earlier_report, 48, 3)];
            assert!(
                WorkflowExecutor::resolve_completion_report(
                    &json!({}),
                    latest_report,
                    &pending,
                    3,
                )
                .is_err(),
                "different critical literals must not be merged by aggregate similarity"
            );
        }
    }

    #[test]
    fn completion_report_resolution_rejects_similar_limitation_conflicts() {
        let earlier_report = "Updated `src/workflow.rs` to fix completion handling.\nThe focused tests passed. No known limitations remain.";
        let latest_report = "Updated `src/workflow.rs` to fix completion handling.\nThe focused tests passed. A remaining limitation affects Windows recovery.";
        let pending = vec![report(earlier_report, 49, 3)];

        assert!(
            WorkflowExecutor::resolve_completion_report(&json!({}), latest_report, &pending, 3)
                .is_err(),
            "conflicting limitation claims must remain ambiguous"
        );
    }

    #[test]
    fn completion_report_resolution_rejects_omission_heavy_summary() {
        let earlier_report = "Implemented `src/workflow.rs` completion report capture and invalidation.\nVerified pending-report recovery, queued-user invalidation, legacy payload compatibility, final-review evidence, `cargo test`, and `cargo check`. The broader suite still has one unrelated pre-existing failure.";
        let latest_report =
            "Implemented `src/workflow.rs` completion report capture.\nThe focused tests passed.";
        let pending = vec![report(earlier_report, 50, 3)];

        assert!(
            WorkflowExecutor::resolve_completion_report(&json!({}), latest_report, &pending, 3)
                .is_err(),
            "a short report that drops verification and limitations must remain ambiguous"
        );
    }

    #[test]
    fn completion_report_resolution_rejects_multiple_pending_reports() {
        let pending = vec![
            report(
                "Implemented the first version of the fix.\nVerification was not run yet.",
                44,
                3,
            ),
            report(
                "Implemented the corrected version of the fix.\nVerification passed.",
                45,
                3,
            ),
        ];

        assert!(WorkflowExecutor::resolve_completion_report(&json!({}), "", &pending, 3).is_err());
    }

    #[test]
    fn completion_report_resolution_ignores_stale_segment_reports() {
        let pending = vec![report(
            "Implemented the previous task.\nVerified its focused tests.",
            46,
            2,
        )];

        assert!(WorkflowExecutor::resolve_completion_report(&json!({}), "", &pending, 3).is_err());
    }

    #[test]
    fn completion_report_resolution_rejects_a_tampered_pending_report() {
        let mut pending = report(
            "Implemented the requested workflow fix.\nVerified the focused tests passed.",
            47,
            3,
        );
        pending.content =
            "A different report was injected after the draft was captured.\nVerification is unknown."
                .to_string();

        assert!(
            WorkflowExecutor::resolve_completion_report(&json!({}), "", &[pending], 3).is_err()
        );
    }

    #[test]
    fn completion_report_resolution_accepts_optional_summary_and_legacy_payload() {
        let legacy_report =
            "Implemented the legacy workflow change.\nVerified the focused tests passed.";

        for args in [
            json!({ "summary": legacy_report }),
            json!({ "report_source": "tool_argument", "summary": legacy_report }),
        ] {
            let resolved = WorkflowExecutor::resolve_completion_report(&args, "", &[], 1).unwrap();
            assert_eq!(resolved.content, legacy_report);
        }
    }

    #[test]
    fn completion_report_resolution_accepts_legacy_assistant_source_with_pending_report() {
        let full_report =
            "Implemented the requested workflow fix.\nVerified the focused tests passed.";
        let pending = vec![report(full_report, 48, 3)];

        let resolved = WorkflowExecutor::resolve_completion_report(
            &json!({ "report_source": "assistant_message" }),
            "",
            &pending,
            3,
        )
        .expect("legacy source marker should use the structured pending draft");

        assert_eq!(resolved.content, full_report);
        assert_eq!(resolved.source_message_id, Some(48));
    }

    #[test]
    fn completion_report_resolution_rejects_invalid_legacy_argument_combinations() {
        let full_report =
            "Implemented the requested workflow fix.\nVerified the focused tests passed.";

        for args in [
            json!({ "summary": 42 }),
            json!({ "report_source": "assistant_message", "summary": full_report }),
            json!({ "report_source": "tool_argument" }),
            json!({ "report_source": "unknown", "summary": full_report }),
            json!({ "unexpected": full_report }),
        ] {
            assert!(
                WorkflowExecutor::resolve_completion_report(&args, full_report, &[], 3).is_err(),
                "legacy payload should be rejected: {args}"
            );
        }
    }

    #[test]
    fn pending_completion_report_invalidation_obeys_tool_boundaries() {
        assert!(
            !WorkflowExecutor::should_invalidate_pending_completion_reports(&["complete_workflow"])
        );
        assert!(WorkflowExecutor::should_invalidate_pending_completion_reports(&["todo_update"]));
        assert!(
            WorkflowExecutor::should_invalidate_pending_completion_reports(&[
                "todo_update",
                "complete_workflow"
            ])
        );
        assert!(WorkflowExecutor::should_invalidate_pending_completion_reports(&["bash"]));
    }

    #[test]
    fn pending_completion_report_capture_requires_no_active_todos() {
        assert!(WorkflowExecutor::todos_allow_completion_report_capture(&[]));
        assert!(WorkflowExecutor::todos_allow_completion_report_capture(&[
            json!({"status": "completed"}),
            json!({"status": "data_missing"}),
            json!({"status": "failed"}),
        ]));
        assert!(!WorkflowExecutor::todos_allow_completion_report_capture(&[
            json!({"status": "completed"}),
            json!({"status": "in_progress"}),
        ]));
        assert!(!WorkflowExecutor::todos_allow_completion_report_capture(&[
            json!({"status": "pending"}),
        ]));
        assert!(!WorkflowExecutor::todos_allow_completion_report_capture(&[
            json!({"status": "unknown"}),
        ]));
    }

    #[test]
    fn approved_plan_completion_requires_a_successful_implementation_action() {
        let approved_plan = WorkflowMessage {
            id: Some(1),
            session_id: "implementation-boundary".to_string(),
            role: "system".to_string(),
            message: "# APPROVED EXECUTION PLAN".to_string(),
            reasoning: None,
            message_kind: "summary".to_string(),
            message_subtype: Some("approved_plan".to_string()),
            segment_id: 2,
            source_event_type: None,
            metadata: Some(json!({"plan_content": "Implement the fix"})),
            attached_context: None,
            step_type: None,
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        };
        let approval_observation = tool_message(
            2,
            "Plan approved",
            false,
            json!({"tool_name": "submit_plan"}),
        );
        let todo_read = tool_message(3, "[]", false, json!({"tool_name": "todo_list"}));
        let failed_action = tool_message(4, "read failed", true, json!({"tool_name": "read_file"}));
        let messages = vec![
            approved_plan.clone(),
            approval_observation,
            todo_read,
            failed_action,
        ];

        assert!(WorkflowExecutor::implementation_completion_is_blocked_for(
            &ExecutionPhase::Implementation,
            &messages,
        ));
        assert!(!WorkflowExecutor::implementation_completion_is_blocked_for(
            &ExecutionPhase::Standard,
            &messages,
        ));

        let mut started_messages = messages;
        started_messages.push(tool_message(
            5,
            "file contents",
            false,
            json!({"tool_name": "read_file"}),
        ));
        assert!(!WorkflowExecutor::implementation_completion_is_blocked_for(
            &ExecutionPhase::Implementation,
            &started_messages,
        ));
    }

    fn tool_message(
        id: i64,
        message: &str,
        is_error: bool,
        metadata: serde_json::Value,
    ) -> WorkflowMessage {
        WorkflowMessage {
            id: Some(id),
            session_id: "review-evidence".to_string(),
            role: "tool".to_string(),
            message: message.to_string(),
            reasoning: None,
            message_kind: "tool".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: Some(metadata),
            attached_context: None,
            step_type: Some("observe".to_string()),
            step_index: id as i32,
            is_error,
            error_type: is_error.then(|| "ToolExecutionError".to_string()),
            created_at: None,
        }
    }

    #[test]
    fn final_review_evidence_uses_structured_tool_metadata() {
        let messages = vec![
            tool_message(
                51,
                "unstructured edit transcript",
                false,
                json!({
                    "tool_name": "edit_file",
                    "execution_status": "completed",
                    "summary": "Updated completion resolution",
                    "details": {"file_path": "src/runtime.rs", "start_line": 10}
                }),
            ),
            tool_message(
                52,
                "this transcript must not be parsed for authority",
                false,
                json!({
                    "tool_name": "bash",
                    "execution_status": "completed",
                    "summary": "Focused tests passed",
                    "llm_content": "running 4 tests\ntest result: ok. 4 passed",
                    "tool_call": {
                        "function": {
                            "name": "bash",
                            "arguments": "{\"command\":\"cargo test --lib completion_report\"}"
                        }
                    }
                }),
            ),
            tool_message(
                53,
                "cargo test appears only in ordinary transcript text",
                false,
                json!({
                    "tool_name": "read_file",
                    "execution_status": "completed",
                    "summary": "Read documentation"
                }),
            ),
            tool_message(
                54,
                "unstructured failure transcript",
                true,
                json!({
                    "tool_name": "bash",
                    "execution_status": "failed",
                    "error_type": "ToolExecutionError",
                    "summary": "Formatting check failed",
                    "llm_content": "error: formatting differs",
                    "tool_call": {
                        "function": {
                            "name": "bash",
                            "arguments": {"command": "cargo fmt --check"}
                        }
                    }
                }),
            ),
        ];

        let evidence = WorkflowExecutor::build_final_review_evidence(&messages);

        assert_eq!(evidence["mutation_ledger"].as_array().unwrap().len(), 1);
        assert_eq!(evidence["mutation_ledger"][0]["path"], "src/runtime.rs");
        assert_eq!(evidence["verification_ledger"].as_array().unwrap().len(), 2);
        assert_eq!(
            evidence["verification_ledger"][0]["command"],
            "cargo test --lib completion_report"
        );
        assert_eq!(evidence["failed_actions"].as_array().unwrap().len(), 1);
        assert_eq!(evidence["failed_actions"][0]["message_id"], 54);
        assert!(!evidence.to_string().contains("ordinary transcript text"));
    }

    #[test]
    fn completion_report_resolution_rejects_conflicting_legacy_and_pending_reports() {
        let pending = vec![report(
            "Implemented the complete workflow fix.\nVerified every focused test passed.",
            47,
            1,
        )];
        let args = json!({
            "summary": "Implemented a smaller workflow change.\nOnly compilation was checked."
        });

        assert!(WorkflowExecutor::resolve_completion_report(&args, "", &pending, 1).is_err());
    }

    #[test]
    fn completion_turn_allows_only_todo_updates_beside_completion() {
        assert!(!WorkflowExecutor::completion_turn_has_incompatible_tools(
            &["todo_update", "complete_workflow",]
        ));
        assert!(WorkflowExecutor::completion_turn_has_incompatible_tools(&[
            "bash",
            "complete_workflow",
        ]));
        assert!(WorkflowExecutor::completion_turn_has_incompatible_tools(&[
            "edit_file",
            "complete_workflow",
        ]));
    }

    #[test]
    fn smart_mode_auto_approves_read_only_tools_and_file_writes() {
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(TOOL_READ_FILE, &json!({})),
            SmartApprovalDecision::AutoApprove
        );
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(
                TOOL_EDIT_FILE,
                &json!({"file_path":"/tmp/test.rs"})
            ),
            SmartApprovalDecision::AutoApprove
        );
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(
                TOOL_WRITE_FILE,
                &json!({"file_path":"/tmp/test.rs","content":"x"})
            ),
            SmartApprovalDecision::AutoApprove
        );
    }

    #[test]
    fn smart_mode_auto_approves_read_only_git_inspection_tools() {
        for tool in [crate::tools::TOOL_GIT_DIFF, crate::tools::TOOL_GIT_INSPECT] {
            assert_eq!(
                WorkflowExecutor::smart_mode_approval_decision(tool, &json!({})),
                SmartApprovalDecision::AutoApprove,
                "{tool} should be auto-approved as a fixed read-only Git tool"
            );
        }
    }

    #[test]
    fn smart_mode_auto_approves_read_only_bash_diagnostics_with_chaining() {
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(
                TOOL_BASH,
                &json!({"command":"cd /repo && git status"})
            ),
            SmartApprovalDecision::AutoApprove
        );
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(
                TOOL_BASH,
                &json!({"command":"git log --oneline | head -20"})
            ),
            SmartApprovalDecision::AutoApprove
        );
    }

    #[test]
    fn smart_mode_auto_approves_common_package_build_commands() {
        for command in [
            "npm build",
            "pnpm build",
            "yarn build",
            "cd /repo && pnpm build | tail -20",
        ] {
            assert_eq!(
                WorkflowExecutor::smart_mode_approval_decision(
                    TOOL_BASH,
                    &json!({ "command": command })
                ),
                SmartApprovalDecision::AutoApprove,
                "command should be auto-approved: {}",
                command
            );
        }

        for command in [
            "npm run build",
            "npm run -s build",
            "npm run-script build",
            "pnpm run build",
            "yarn run build",
            "pnpm tauri dev",
            "pnpm dev",
        ] {
            assert_eq!(
                WorkflowExecutor::smart_mode_approval_decision(
                    TOOL_BASH,
                    &json!({ "command": command })
                ),
                SmartApprovalDecision::ReviewByUser,
                "command should require user approval: {}",
                command
            );
        }
    }

    #[test]
    fn smart_mode_sends_unknown_mutations_to_ai_review() {
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(
                "delete_file",
                &json!({"file_path":"/tmp/test.rs"})
            ),
            SmartApprovalDecision::ReviewWithAi
        );
    }

    #[test]
    fn smart_mode_routes_non_read_only_bash_to_user_review() {
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(
                TOOL_BASH,
                &json!({"command":"cargo test"})
            ),
            SmartApprovalDecision::ReviewByUser
        );
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(
                TOOL_BASH,
                &json!({"command":"cat secret.txt > out.txt"})
            ),
            SmartApprovalDecision::ReviewByUser
        );
    }
}
