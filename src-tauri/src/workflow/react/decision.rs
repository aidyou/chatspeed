use crate::ccproxy::decision::{self, Answer, DecisionRequest, Question};
use crate::ccproxy::utils::token_estimator::estimate_tokens;
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::intelligence::IntelligenceManager;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct ToolApprovalReview {
    pub approved: bool,
    pub reason: String,
    pub risk_level: String,
}

/// Acceptance bars calibrated against real database cases; a refused answer simply falls back to the
/// lite model, so these bars only decide whether the typed answer is trustworthy.
const APPROVAL_DECISION_CONFIDENCE: f64 = 0.85;
const APPROVAL_DECISION_PROBABILITY: f64 = 0.90;

/// Acceptance bar for the completion-report choice.
///
/// Calibration against real completion reports from the local database: candidates that differ in
/// provenance land at 0.77+ probability with 0.65+ confidence, while two faithful representations of
/// the same work stay under 0.68 probability, and choosing `ambiguous` is never accepted.
const COMPLETION_DECISION_CONFIDENCE: f64 = 0.62;
const COMPLETION_DECISION_PROBABILITY: f64 = 0.72;

/// Input ceiling of the decision protocol's evaluation endpoint. Measured against the live
/// `/v1/systemone` route: 7997 input tokens succeeds and roughly 8300 is rejected, so requests above
/// 8192 tokens are refused by the upstream.
const DECISION_INPUT_TOKEN_CEILING: usize = 8_192;

/// A refused request silently degrades the review to the lite model, so keep the whole candidate
/// payload clearly below the measured ceiling.
const COMPLETION_DECISION_TOKEN_BUDGET: usize = DECISION_INPUT_TOKEN_CEILING * 7 / 8;
const COMPLETION_CANDIDATE_CHAR_CEILING: usize = 6_000;
const COMPLETION_CANDIDATE_CHAR_FLOOR: usize = 200;
const COMPLETION_REQUEST_CHAR_LIMIT: usize = 600;

const COMPLETION_DECISION_INSTRUCTIONS: &str = concat!(
    "Exactly one candidate will be published as the workflow's final completion report and shown to ",
    "the user, so accuracy matters more than style. Decide from the candidate text alone which single ",
    "candidate is a faithful, sufficiently detailed account of the work this workflow actually ",
    "finished. A candidate produced by this complete_workflow call is the current answer; a candidate ",
    "captured earlier is only a draft and may describe superseded intermediate reasoning, so prefer ",
    "the current answer unless it is clearly not a completion report. Prefer a candidate that states ",
    "what was done, what was verified and what remains over one that merely narrates analysis or gives ",
    "a single line without evidence. Several candidates may be equally faithful; pick the best single ",
    "one instead of refusing. Choose ambiguous only when no candidate is a plausible completion report ",
    "or two candidates materially contradict each other."
);

/// Where a completion-report candidate came from, as far as the program can prove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionReportOrigin {
    /// The `summary` argument of the current `complete_workflow` call.
    ThisCallSummary,
    /// Assistant text written in the same turn as the current `complete_workflow` call.
    ThisCallText,
    /// An assistant message captured earlier in this segment, before the current call.
    EarlierDraft,
}

impl CompletionReportOrigin {
    fn label(self) -> &'static str {
        match self {
            Self::ThisCallSummary => "this_call_summary",
            Self::ThisCallText => "this_call_text",
            Self::EarlierDraft => "earlier_draft",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Self::ThisCallSummary => "the summary argument of this complete_workflow call",
            Self::ThisCallText => {
                "assistant text written in the same turn as this complete_workflow call"
            }
            Self::EarlierDraft => {
                "an assistant message captured earlier in this segment, before this call"
            }
        }
    }
}

/// One candidate report the task-completion decision may publish.
#[derive(Debug, Clone)]
pub(crate) struct CompletionReportCandidate {
    pub content: String,
    pub origin: CompletionReportOrigin,
}

fn truncate_decision_text(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

/// Per-candidate character limits that keep the uploaded payload inside the endpoint's token budget.
///
/// Candidates are only shortened when the whole payload would overflow, so ordinary reports reach the
/// model in full.
fn completion_candidate_limits(texts: &[&str]) -> Vec<usize> {
    let base: Vec<usize> = texts
        .iter()
        .map(|text| text.chars().count().min(COMPLETION_CANDIDATE_CHAR_CEILING))
        .collect();
    let limit_at = |index: usize, scale_permille: usize| -> usize {
        (base[index] * scale_permille / 1000).max(COMPLETION_CANDIDATE_CHAR_FLOOR)
    };
    let estimate = |scale_permille: usize| -> f64 {
        texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                estimate_tokens(truncate_decision_text(text, limit_at(index, scale_permille)))
            })
            .sum()
    };
    if estimate(1000) <= COMPLETION_DECISION_TOKEN_BUDGET as f64 {
        return base;
    }
    let (mut low, mut high) = (0usize, 1000usize);
    while low < high {
        let middle = (low + high + 1) / 2;
        if estimate(middle) <= COMPLETION_DECISION_TOKEN_BUDGET as f64 {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    (0..texts.len()).map(|index| limit_at(index, low)).collect()
}

/// Languages the decision model can name directly, the fifteen most-spoken varieties by total
/// speakers (Berlitz/Ethnologue, 2025). Anything else, or any answer the model cannot separate from
/// its strongest rival, falls back to the lite detector.
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

/// Language selection is the widest choice the decision model answers, so its probabilities stay low
/// even when the answer is right. Calibrated on real and multilingual inputs: the selected option
/// must beat its strongest rival by `LANGUAGE_DECISION_MARGIN` and clear both bars, which accepted
/// 15 of 50 labeled samples with no wrong answer, where a fifteen-way choice averages 0.42
/// probability and misreads prose that merely embeds English identifiers.
const LANGUAGE_DECISION_CONFIDENCE: f64 = 0.45;
const LANGUAGE_DECISION_PROBABILITY: f64 = 0.55;
const LANGUAGE_DECISION_MARGIN: f64 = 2.0;

const LANGUAGE_DECISION_INSTRUCTIONS: &str = concat!(
    "Identify the language the user writes their own instructions in. Judge the prose the user wrote, ",
    "not code, paths, URLs, identifiers, or English technical terms embedded in another language; when ",
    "the user's sentences are in one language but carry English technical terms, choose that language. ",
    "Choose other when the prose is written in a language outside the list, mixes languages evenly, or ",
    "carries no natural-language instruction."
);

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
        .map(|(code, name)| {
            (
                (*code).to_string(),
                format!("the user's own sentences are written in {name}"),
            )
        })
        .chain(std::iter::once((
            "other".to_string(),
            "another language, an even mix of languages, or no natural-language instruction"
                .to_string(),
        )))
        .collect()
}

/// The strongest probability the model placed on any language other than the selected one.
fn rival_probability(probabilities: &BTreeMap<String, f64>, choice: &str) -> f64 {
    probabilities
        .iter()
        .filter(|(code, _)| code.as_str() != choice)
        .map(|(_, probability)| *probability)
        .fold(0.0, f64::max)
}

fn selected_language(answer: &Answer) -> Option<String> {
    let codes: Vec<&str> = TOP_LANGUAGES.iter().map(|(code, _)| *code).collect();
    let choice = confident_choice(
        answer,
        &codes,
        LANGUAGE_DECISION_CONFIDENCE,
        LANGUAGE_DECISION_PROBABILITY,
    )?;
    let Answer::Choice { probabilities, .. } = answer else {
        return None;
    };
    let selected = *probabilities.get(&choice)?;
    if selected < LANGUAGE_DECISION_MARGIN * rival_probability(probabilities, &choice) {
        return None;
    }
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
                    instructions: "Decide from the workspace, goal, tool and arguments alone whether this exact tool call is safe to run without asking the user. Approve only clearly read-only or reversible actions that serve the user's goal inside the authorized workspace. Choose review_required as soon as the call writes or deletes anything, installs or rebuilds software, changes configuration or system state, reaches outside the workspace, exposes secrets, or when the user's intent is unclear.".into(),
                    criteria: BTreeMap::from([
                        ("approve_low_risk".into(), "Read-only inspection, or a reversible action that stays inside the authorized workspace and creates no new state: no writes, deletions, installs, configuration or system-state changes, no secret exposure, no nested or interpolated execution, and no ambiguity about the user's intent".into()),
                        ("review_required".into(), "Anything that writes, deletes, installs, rebuilds, or changes configuration or system state; anything reaching outside the authorized workspace; anything handling secrets; nested or interpolated execution; or an unclear intent".into()),
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
        candidates: &[CompletionReportCandidate],
        detailed_report: bool,
        user_request: &str,
    ) -> Option<usize> {
        let Some((provider_id, model)) = self.decision_model() else {
            return None;
        };
        if candidates.is_empty() || candidates.len() > 16 {
            return None;
        }
        let texts: Vec<&str> = candidates
            .iter()
            .map(|candidate| candidate.content.as_str())
            .collect();
        let limits = completion_candidate_limits(&texts);
        let criteria = candidates
            .iter()
            .enumerate()
            .map(|(index, _)| {
                (
                    format!("report_{index}"),
                    "a faithful, sufficiently detailed account of the work this workflow actually finished"
                        .to_string(),
                )
            })
            .chain(std::iter::once((
                "ambiguous".into(),
                "no candidate is a plausible completion report, or two candidates materially contradict each other"
                    .into(),
            )))
            .collect();
        let request = DecisionRequest {
            state: serde_json::json!({
                "required_detail": if detailed_report { "detailed" } else { "brief" },
                "user_request": truncate_decision_text(user_request.trim(), COMPLETION_REQUEST_CHAR_LIMIT),
                "candidates": candidates.iter().zip(limits.iter()).enumerate().map(|(index, (candidate, limit))| {
                    serde_json::json!({
                        "id": format!("report_{index}"),
                        "origin": candidate.origin.label(),
                        "origin_note": candidate.origin.note(),
                        "produced_by_this_call": candidate.origin != CompletionReportOrigin::EarlierDraft,
                        "content": truncate_decision_text(&candidate.content, *limit),
                    })
                }).collect::<Vec<_>>(),
            }).to_string(),
            model,
            questions: BTreeMap::from([(
                "report_selection".into(),
                Question::Choice {
                    instructions: COMPLETION_DECISION_INSTRUCTIONS.into(),
                    criteria,
                },
            )]),
        };
        match decision::evaluate(self.chat_state.main_store.clone(), provider_id, request).await {
            Ok(response) => {
                let allowed = (0..candidates.len()).map(|index| format!("report_{index}")).collect::<Vec<_>>();
                let allowed_refs = allowed.iter().map(String::as_str).collect::<Vec<_>>();
                response.answers.get("report_selection")
                    .and_then(|answer| confident_choice(answer, &allowed_refs, COMPLETION_DECISION_CONFIDENCE, COMPLETION_DECISION_PROBABILITY))
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
            state: Self::truncate_to_token_budget(
                user_input.trim(),
                max_input_tokens.min(DECISION_INPUT_TOKEN_CEILING),
            ),
            model,
            questions: BTreeMap::from([(
                "language".into(),
                Question::Choice {
                    instructions: LANGUAGE_DECISION_INSTRUCTIONS.into(),
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
        completion_candidate_limits, confident_choice, language_criteria,
        parse_tool_approval_review, selected_language, truncate_decision_text,
        CompletionReportOrigin, APPROVAL_DECISION_CONFIDENCE, APPROVAL_DECISION_PROBABILITY,
        COMPLETION_CANDIDATE_CHAR_FLOOR, COMPLETION_DECISION_TOKEN_BUDGET,
        TOP_LANGUAGES, LANGUAGE_DECISION_CONFIDENCE, LANGUAGE_DECISION_PROBABILITY,
    };
    use crate::ccproxy::decision::Answer;
    use crate::ccproxy::utils::token_estimator::estimate_tokens;
    use std::collections::BTreeMap;

    #[test]
    fn completion_candidate_limits_fit_the_endpoint_budget() {
        let short = "简短报告。".repeat(4);
        let long = "中继转发镜像重连链路按设计工作，上游释放超时是唯一异常。".repeat(600);
        let texts = [short.as_str(), long.as_str()];
        let limits = completion_candidate_limits(&texts);
        let estimated: f64 = texts
            .iter()
            .zip(limits.iter())
            .map(|(text, limit)| estimate_tokens(truncate_decision_text(text, *limit)))
            .sum();
        assert!(estimated <= COMPLETION_DECISION_TOKEN_BUDGET as f64);
        assert!(limits[1] < long.chars().count());
        assert!(limits
            .iter()
            .all(|limit| *limit >= COMPLETION_CANDIDATE_CHAR_FLOOR));
    }

    #[test]
    fn completion_candidate_limits_keep_fitting_reports_untruncated() {
        let reports = [
            "报告一：完成改动并已通过测试。".repeat(30),
            "报告二：只读分析，未修改任何代码。".repeat(30),
        ];
        let texts: Vec<&str> = reports.iter().map(String::as_str).collect();
        assert_eq!(
            completion_candidate_limits(&texts),
            reports
                .iter()
                .map(|report| report.chars().count())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn completion_origins_describe_their_provenance() {
        assert_eq!(
            CompletionReportOrigin::ThisCallSummary.label(),
            "this_call_summary"
        );
        assert_eq!(
            CompletionReportOrigin::ThisCallText.label(),
            "this_call_text"
        );
        assert_eq!(
            CompletionReportOrigin::EarlierDraft.label(),
            "earlier_draft"
        );
        assert_eq!(
            truncate_decision_text("abcdef", 3),
            "abc",
            "characters outside the limit must be dropped"
        );
        assert_eq!(truncate_decision_text("abc", 9), "abc");
        assert_eq!(truncate_decision_text("中文报告", 2), "中文");
    }

    #[test]
    fn language_decision_exposes_fifteen_languages_and_a_dominance_rule() {
        let criteria = language_criteria();
        assert_eq!(TOP_LANGUAGES.len(), 15);
        assert_eq!(criteria.len(), 16);
        assert!(criteria.contains_key("other"));
        for (code, name) in TOP_LANGUAGES {
            assert!(criteria.get(code).is_some_and(|description| description.contains(name)));
            let answer = Answer::Choice {
                choice: code.into(),
                probabilities: BTreeMap::from([(code.into(), 0.62), ("other".into(), 0.30)]),
                confidence: 0.50,
            };
            assert_eq!(selected_language(&answer).as_deref(), Some(name));
        }
        let answer = |choice: &str, selected: f64, rival: f64, confidence: f64| Answer::Choice {
            choice: choice.into(),
            probabilities: BTreeMap::from([(choice.into(), selected), ("other".into(), rival)]),
            confidence,
        };
        assert_eq!(selected_language(&answer("ja", 0.60, 0.29, 0.50)).as_deref(), Some("日本語"));
        for (choice, selected, rival, confidence) in [
            ("en", 0.62, 0.32, 0.50),
            ("other", 0.99, 0.01, 0.99),
            ("ko", 0.99, 0.01, 0.99),
            ("en", 0.54, 0.10, 0.99),
            ("en", 0.99, 0.10, 0.44),
        ] {
            assert!(
                selected_language(&answer(choice, selected, rival, confidence)).is_none(),
                "{choice} must fall back to lite"
            );
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
            confident_choice(&answer("zh", 0.85, 0.70), &["zh"], LANGUAGE_DECISION_CONFIDENCE, LANGUAGE_DECISION_PROBABILITY),
            Some("zh".into())
        );
        assert!(confident_choice(&answer("other", 0.99, 0.99), &["zh"], LANGUAGE_DECISION_CONFIDENCE, LANGUAGE_DECISION_PROBABILITY).is_none());
        assert!(confident_choice(&answer("zh", 0.54, 0.99), &["zh"], LANGUAGE_DECISION_CONFIDENCE, LANGUAGE_DECISION_PROBABILITY).is_none());
        assert!(confident_choice(&answer("approve_low_risk", 0.95, 0.88), &["approve_low_risk"], APPROVAL_DECISION_CONFIDENCE, APPROVAL_DECISION_PROBABILITY).is_some());
        assert!(confident_choice(&answer("approve_low_risk", 0.85, 0.99), &["approve_low_risk"], APPROVAL_DECISION_CONFIDENCE, APPROVAL_DECISION_PROBABILITY).is_none());
        assert!(confident_choice(&answer("approve_low_risk", 0.99, 0.80), &["approve_low_risk"], APPROVAL_DECISION_CONFIDENCE, APPROVAL_DECISION_PROBABILITY).is_none());
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
