use crate::ccproxy::decision::{self, Answer, DecisionRequest, Question};
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::intelligence::IntelligenceManager;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct ToolApprovalReview {
    pub approved: bool,
    pub reason: String,
    pub risk_level: String,
}

const LANGUAGE_DECISION_CONFIDENCE: f64 = 0.92;
const LANGUAGE_DECISION_PROBABILITY: f64 = 0.95;
const APPROVAL_DECISION_CONFIDENCE: f64 = 0.98;
const APPROVAL_DECISION_PROBABILITY: f64 = 0.99;

/// Fifteen most-spoken varieties by total speakers (Berlitz/Ethnologue, 2025).
/// `other` makes unsupported or uncertain inputs fall back to the lite model.
const TOP_LANGUAGES: [(&str, &str); 15] = [
    ("en", "English"),
    ("zh", "中文"),
    ("hi", "हिन्दी"),
    ("es", "Español"),
    ("fr", "Français"),
    ("ar", "العربية"),
    ("bn", "বাংলা"),
    ("pt", "Português"),
    ("ru", "Русский"),
    ("ur", "اردو"),
    ("id", "Bahasa Indonesia"),
    ("de", "Deutsch"),
    ("ja", "日本語"),
    ("pcm", "Nigerian Pidgin"),
    ("arz", "العربية المصرية"),
];

fn confident_choice(
    answer: &Answer,
    allowed: &[&str],
    min_confidence: f64,
    min_probability: f64,
) -> Option<String> {
    let Answer::Choice {
        choice,
        probabilities,
        confidence,
    } = answer
    else {
        return None;
    };
    let selected = *probabilities.get(choice)?;
    (allowed.contains(&choice.as_str())
        && *confidence >= min_confidence
        && selected >= min_probability)
    .then(|| choice.clone())
}

fn language_criteria() -> BTreeMap<String, String> {
    TOP_LANGUAGES
        .iter()
        .map(|(code, name)| ((*code).to_string(), format!("{name} instructions")))
        .chain(std::iter::once((
            "other".to_string(),
            "Another language, mixed or unclear instructions, or no natural language".to_string(),
        )))
        .collect()
}

fn selected_language(answer: &Answer) -> Option<String> {
    let codes: Vec<&str> = TOP_LANGUAGES.iter().map(|(code, _)| *code).collect();
    let choice = confident_choice(
        answer,
        &codes,
        LANGUAGE_DECISION_CONFIDENCE,
        LANGUAGE_DECISION_PROBABILITY,
    )?;
    TOP_LANGUAGES
        .iter()
        .find(|(code, _)| *code == choice)
        .map(|(_, name)| (*name).to_string())
}

pub(crate) fn parse_tool_approval_review(result: &str) -> ToolApprovalReview {
    let invalid_review = || ToolApprovalReview {
        approved: false,
        reason: "Approval reviewer returned invalid structured output; manual review required"
            .to_string(),
        risk_level: "medium".to_string(),
    };

    let Ok(review_json) = serde_json::from_str::<serde_json::Value>(
        crate::libs::util::format_json_str(result).as_str(),
    ) else {
        return invalid_review();
    };
    let Some(review) = review_json.as_object() else {
        return invalid_review();
    };
    let Some(approved) = review.get("approved").and_then(|value| value.as_bool()) else {
        return invalid_review();
    };
    let Some(reason) = review
        .get("reason")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return invalid_review();
    };
    let Some(risk_level) = review
        .get("risk_level")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| matches!(*value, "low" | "medium" | "high"))
    else {
        return invalid_review();
    };

    ToolApprovalReview {
        approved: approved && risk_level == "low",
        reason: reason.to_string(),
        risk_level: risk_level.to_string(),
    }
}

impl IntelligenceManager {
    fn configured_decision_model(&self) -> Option<(i64, String)> {
        if self.decision_provider_id <= 0 || self.decision_model_name.trim().is_empty() {
            return None;
        }
        let provider = self
            .chat_state
            .main_store
            .config
            .get_ai_model_by_id(self.decision_provider_id)
            .ok()?;
        if provider.disabled || provider.api_protocol != "decision" {
            return None;
        }
        provider
            .models
            .iter()
            .any(|entry| entry.id == self.decision_model_name)
            .then(|| (self.decision_provider_id, self.decision_model_name.clone()))
    }

    fn global_decision_model(&self) -> Option<(i64, String)> {
        let config: serde_json::Value = self
            .chat_state
            .main_store
            .get_config("decision_config", serde_json::json!({}));
        if config.get("enabled").and_then(serde_json::Value::as_bool) != Some(true) {
            return None;
        }
        let provider_id = config
            .get("providerId")
            .and_then(serde_json::Value::as_i64)?;
        let model = config.get("model").and_then(serde_json::Value::as_str)?.trim();
        if provider_id <= 0 || model.is_empty() {
            return None;
        }
        let provider = self.chat_state.main_store.config.get_ai_model_by_id(provider_id).ok()?;
        if provider.disabled || provider.api_protocol != "decision" {
            return None;
        }
        provider
            .models
            .iter()
            .any(|entry| entry.id == model)
            .then(|| (provider_id, model.to_string()))
    }

    fn decision_model(&self) -> Option<(i64, String)> {
        self.configured_decision_model().or_else(|| self.global_decision_model())
    }

    pub(crate) async fn try_decision_approval(
        &self,
        context: &ContextManager,
        workspace_context: &str,
        tool_name: &str,
        tool_category: &str,
        tool_scope: &str,
        tool_description: &str,
        tool_args: &serde_json::Value,
        assistant_text: &str,
    ) -> Option<ToolApprovalReview> {
        let Some((provider_id, model)) = self.decision_model() else {
            return None;
        };
        let state = serde_json::json!({
            "workspace": Self::truncate_text(workspace_context, 800),
            "goal": Self::truncate_text(&context.current_user_request_since_last_completion(), 1500),
            "intent": Self::truncate_text(assistant_text, 800),
            "tool": tool_name,
            "category": tool_category,
            "scope": tool_scope,
            "description": Self::truncate_text(tool_description, 500),
            "arguments": tool_args,
        })
        .to_string();
        let request = DecisionRequest {
            state,
            model,
            questions: BTreeMap::from([(
                "approval".into(),
                Question::Choice {
                    instructions: "Choose approve_low_risk only when this exact tool call is clearly low risk, within the user's goal and authorized workspace; if uncertain, unsafe, policy-sensitive, or needing confirmation choose review_required.".into(),
                    criteria: BTreeMap::from([
                        ("approve_low_risk".into(), "Clearly safe, read-only or authorized low-risk action; no policy, path, secret, destructive, or shell execution concerns".into()),
                        ("review_required".into(), "Any risk, uncertainty, sensitive information, destructive behavior, policy exception, or ambiguous intent".into()),
                    ]),
                },
            )]),
        };
        match decision::evaluate(self.chat_state.main_store.clone(), provider_id, request).await {
            Ok(response) => {
                if response
                    .answers
                    .get("approval")
                    .and_then(|answer| {
                        confident_choice(
                            answer,
                            &["approve_low_risk"],
                            APPROVAL_DECISION_CONFIDENCE,
                            APPROVAL_DECISION_PROBABILITY,
                        )
                    })
                    .is_some()
                {
                    log::info!(
                        "[Workflow][session={}][approval] Decision approved low risk: tool={tool_name}, model={}",
                        self.session_id,
                        response.model
                    );
                    Some(ToolApprovalReview {
                        approved: true,
                        reason: "Decision model confirmed low risk".into(),
                        risk_level: "low".into(),
                    })
                } else {
                    None
                }
            }
            Err(error) => {
                log::info!(
                    "[Workflow][session={}][approval] Decision unavailable; using AI reviewer: {error}",
                    self.session_id
                );
                None
            }
        }
    }

    pub(crate) async fn try_decision_completion(
        &self,
        candidates: &[String],
        detailed_report: bool,
    ) -> Option<usize> {
        let Some((provider_id, model)) = self.decision_model() else {
            return None;
        };
        if candidates.is_empty() || candidates.len() > 16 {
            return None;
        }
        let criteria = candidates
            .iter()
            .enumerate()
            .map(|(index, _)| (format!("report_{index}"), format!("Select report {index} if it accurately represents the final work and satisfies the required report detail")))
            .chain(std::iter::once(("ambiguous".into(), "No candidate can be reliably selected or the reports materially contradict each other".into())))
            .collect();
        let request = DecisionRequest {
            state: serde_json::json!({
                "required_detail": if detailed_report { "detailed" } else { "brief" },
                "candidates": candidates.iter().enumerate().map(|(index, content)| {
                    serde_json::json!({"id": format!("report_{index}"), "content": Self::truncate_text(content, 6000)})
                }).collect::<Vec<_>>(),
            }).to_string(),
            model,
            questions: BTreeMap::from([(
                "report_selection".into(),
                Question::Choice {
                    instructions: "Select the single report that best reflects the final work and meets the required level of detail. Brief reports are acceptable when required_detail is brief. Choose ambiguous if no candidate is reliable or they materially contradict each other. Return a typed choice, not generated text.".into(),
                    criteria,
                },
            )]),
        };
        match decision::evaluate(self.chat_state.main_store.clone(), provider_id, request).await {
            Ok(response) => {
                let allowed = (0..candidates.len()).map(|index| format!("report_{index}")).collect::<Vec<_>>();
                let allowed_refs = allowed.iter().map(String::as_str).collect::<Vec<_>>();
                response.answers.get("report_selection")
                    .and_then(|answer| confident_choice(answer, &allowed_refs, 0.80, 0.80))
                    .and_then(|choice| choice.strip_prefix("report_")?.parse::<usize>().ok())
                    .filter(|index| *index < candidates.len())
            }
            Err(error) => {
                log::warn!("[Workflow][session={}][completion] Decision response invalid; falling back to lite: {error}", self.session_id);
                None
            }
        }
    }

    pub(crate) async fn try_decision_language(
        &self,
        user_input: &str,
        max_input_tokens: usize,
    ) -> Option<String> {
        let Some((provider_id, model)) = self.decision_model() else {
            return None;
        };
        if user_input.trim().is_empty() {
            return None;
        }
        let request = DecisionRequest {
            state: Self::truncate_to_token_budget(user_input.trim(), max_input_tokens),
            model,
            questions: BTreeMap::from([(
                "language".into(),
                Question::Choice {
                    instructions: "Identify the language of the user's own instructions. Ignore quoted material, code, paths, URLs and identifiers; select other when the language is not one of the listed options, mixed, or unclear.".into(),
                    criteria: language_criteria(),
                },
            )]),
        };
        match decision::evaluate(self.chat_state.main_store.clone(), provider_id, request).await {
            Ok(response) => response
                .answers
                .get("language")
                .and_then(selected_language)
                .inspect(|language| {
                    log::info!(
                        "[Workflow][session={}][language] Decision selected {} using model {}",
                        self.session_id,
                        language,
                        response.model
                    );
                }),
            Err(error) => {
                log::warn!(
                    "[Workflow][session={}][language] Decision model unavailable or response invalid; falling back to lite: {error}",
                    self.session_id
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        confident_choice, language_criteria, parse_tool_approval_review, selected_language,
        APPROVAL_DECISION_CONFIDENCE, APPROVAL_DECISION_PROBABILITY,
        LANGUAGE_DECISION_CONFIDENCE, LANGUAGE_DECISION_PROBABILITY, TOP_LANGUAGES,
    };
    use crate::ccproxy::decision::Answer;
    use std::collections::{BTreeMap, HashSet};

    #[test]
    fn language_decision_exposes_fifteen_languages_and_other_fallback() {
        let criteria = language_criteria();
        assert_eq!(TOP_LANGUAGES.len(), 15);
        assert_eq!(criteria.len(), 16);
        assert_eq!(TOP_LANGUAGES.iter().map(|(code, _)| *code).collect::<HashSet<_>>().len(), 15);
        assert!(criteria.contains_key("other"));
        for (code, name) in TOP_LANGUAGES {
            assert!(criteria.get(code).is_some_and(|description| description.contains(name)));
            let answer = Answer::Choice {
                choice: code.into(),
                probabilities: BTreeMap::from([(code.into(), 0.97)]),
                confidence: 0.94,
            };
            assert_eq!(selected_language(&answer).as_deref(), Some(name));
        }
        for (choice, probability, confidence) in [
            ("other", 0.99, 0.99),
            ("ko", 0.99, 0.99),
            ("en", 0.94, 0.99),
            ("en", 0.99, 0.91),
        ] {
            let answer = Answer::Choice {
                choice: choice.into(),
                probabilities: BTreeMap::from([(choice.into(), probability)]),
                confidence,
            };
            assert!(selected_language(&answer).is_none(), "{choice} must fall back to lite");
        }
    }

    #[test]
    fn smart_approval_requires_valid_low_risk_json() {
        let approved = parse_tool_approval_review(
            r#"{"approved":true,"reason":"Read-only inspection","risk_level":"low"}"#,
        );
        assert!(approved.approved);

        for invalid in [
            "I approve this action",
            "I do not recommend approving this action",
            r#"{"approved":true,"reason":"Mutation","risk_level":"medium"}"#,
            r#"{"approved":"true","reason":"Invalid type","risk_level":"low"}"#,
            r#"{"approved":true,"reason":"Missing risk"}"#,
        ] {
            let review = parse_tool_approval_review(invalid);
            assert!(!review.approved, "invalid review was approved: {invalid}");
            assert_eq!(review.risk_level, "medium");
        }
    }

    #[test]
    fn decision_choice_requires_conservative_probability_and_known_option() {
        let answer = |choice: &str, selected: f64, confidence: f64| Answer::Choice {
            choice: choice.into(),
            probabilities: BTreeMap::from([(choice.into(), selected), ("other".into(), 1.0 - selected)]),
            confidence,
        };
        assert_eq!(
            confident_choice(&answer("zh", 0.97, 0.94), &["zh"], LANGUAGE_DECISION_CONFIDENCE, LANGUAGE_DECISION_PROBABILITY),
            Some("zh".into())
        );
        assert!(confident_choice(&answer("other", 0.99, 0.99), &["zh"], LANGUAGE_DECISION_CONFIDENCE, LANGUAGE_DECISION_PROBABILITY).is_none());
        assert!(confident_choice(&answer("zh", 0.8, 0.99), &["zh"], LANGUAGE_DECISION_CONFIDENCE, LANGUAGE_DECISION_PROBABILITY).is_none());
        assert!(confident_choice(&answer("approve_low_risk", 0.995, 0.99), &["approve_low_risk"], APPROVAL_DECISION_CONFIDENCE, APPROVAL_DECISION_PROBABILITY).is_some());
        assert!(confident_choice(&answer("approve_low_risk", 0.98, 0.99), &["approve_low_risk"], APPROVAL_DECISION_CONFIDENCE, APPROVAL_DECISION_PROBABILITY).is_none());
        assert!(confident_choice(&answer("review_required", 0.999, 0.999), &["approve_low_risk"], APPROVAL_DECISION_CONFIDENCE, APPROVAL_DECISION_PROBABILITY).is_none());
    }

    #[test]
    fn smart_approval_preserves_structured_rejection() {
        let review = parse_tool_approval_review(
            r#"{"approved":false,"reason":"Needs user review","risk_level":"high"}"#,
        );
        assert!(!review.approved);
        assert_eq!(review.reason, "Needs user review");
        assert_eq!(review.risk_level, "high");
    }
}
