use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::ccproxy::utils::token_estimator::estimate_tokens;

pub const USER_CONTEXT_REFERENCE_TOKEN_THRESHOLD: f64 = 2048.0;
const USER_CONTEXT_METADATA_KEY: &str = "user_context_reference";
const USER_CONTEXT_INLINE_PREVIEW_CHAR_LIMIT: usize = 6_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserContextReference {
    pub message_id: i64,
    pub sha256: String,
    pub byte_count: usize,
    pub token_estimate: usize,
    #[serde(default)]
    pub inline_preview: String,
}

impl UserContextReference {
    pub fn projection_marker(&self) -> String {
        format!(
            "<USER_CONTEXT_REFERENCE message_id=\"{}\" source=\"workflow_messages\" sha256=\"{}\" bytes=\"{}\" tokens=\"{}\">\nThe complete user input remains in the authoritative workflow database. Read it only when omitted details are required, using read_history_message with this message_id and role=user.\n</USER_CONTEXT_REFERENCE>",
            self.message_id, self.sha256, self.byte_count, self.token_estimate
        )
    }

    pub fn projection_content(&self, fallback_message: &str) -> String {
        let preview = if self.inline_preview.trim().is_empty() {
            fallback_message
        } else {
            &self.inline_preview
        };
        format!("{}\n\n{}", preview, self.projection_marker())
    }
}

pub fn reference_from_metadata(metadata: Option<&Value>) -> Option<UserContextReference> {
    serde_json::from_value(metadata?.get(USER_CONTEXT_METADATA_KEY)?.clone()).ok()
}

pub fn merge_reference_metadata(
    metadata: Option<Value>,
    reference: &UserContextReference,
) -> Value {
    let mut metadata = metadata.unwrap_or_else(|| json!({}));
    if !metadata.is_object() {
        metadata = json!({ "original_metadata": metadata });
    }
    metadata[USER_CONTEXT_METADATA_KEY] =
        serde_json::to_value(reference).unwrap_or_else(|_| json!({}));
    metadata
}

fn merge_user_input(message: &str, attached_context: Option<&str>) -> String {
    match attached_context.filter(|attached| !attached.trim().is_empty()) {
        Some(attached) if !message.trim().is_empty() => {
            format!("## User message\n\n{message}\n\n## Supplemental context\n\n{attached}")
        }
        Some(attached) => format!("## Supplemental context\n\n{attached}"),
        None => format!("## User message\n\n{message}"),
    }
}

fn bounded_preview(content: &str) -> String {
    let char_count = content.chars().count();
    if char_count <= USER_CONTEXT_INLINE_PREVIEW_CHAR_LIMIT {
        return content.to_string();
    }

    let head_limit = USER_CONTEXT_INLINE_PREVIEW_CHAR_LIMIT * 2 / 3;
    let tail_limit = USER_CONTEXT_INLINE_PREVIEW_CHAR_LIMIT - head_limit;
    let head = content.chars().take(head_limit).collect::<String>();
    let tail = content
        .chars()
        .rev()
        .take(tail_limit)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!(
        "{head}\n\n<USER_CONTEXT_PREVIEW_TRUNCATED omitted_chars=\"{}\">\nRead the referenced database message only when details from the omitted middle are needed.\n</USER_CONTEXT_PREVIEW_TRUNCATED>\n\n{tail}",
        char_count.saturating_sub(USER_CONTEXT_INLINE_PREVIEW_CHAR_LIMIT)
    )
}

/// Produces a projection-only reference. The complete input remains in `workflow_messages`.
pub fn reference_large_user_context(
    message_id: i64,
    message: &str,
    attached_context: Option<&str>,
) -> Option<UserContextReference> {
    let full_input = merge_user_input(message, attached_context);
    let token_estimate = estimate_tokens(&full_input).ceil() as usize;
    if token_estimate <= USER_CONTEXT_REFERENCE_TOKEN_THRESHOLD as usize {
        return None;
    }

    Some(UserContextReference {
        message_id,
        sha256: hex::encode(Sha256::digest(full_input.as_bytes())),
        byte_count: full_input.len(),
        token_estimate,
        inline_preview: bounded_preview(&full_input),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_context_stays_inline() {
        let result = reference_large_user_context(1, "small request", Some("small context"));
        assert!(result.is_none());
    }

    #[test]
    fn large_primary_message_uses_bounded_preview_and_database_reference() {
        let primary = format!("goal\n{}\nlatest correction", "x".repeat(12_000));
        let reference = reference_large_user_context(101, &primary, None)
            .expect("primary input should produce a reference");

        let projection = reference.projection_content("fallback");
        assert!(projection.contains("goal"));
        assert!(projection.contains("latest correction"));
        assert!(projection.contains("USER_CONTEXT_REFERENCE"));
        assert!(projection.contains("read_history_message"));
        assert!(projection.chars().count() < primary.chars().count());
        assert!(reference
            .inline_preview
            .contains("USER_CONTEXT_PREVIEW_TRUNCATED"));
        assert!(!projection.contains("path=\".cs/"));
    }

    #[test]
    fn combined_message_and_attachment_share_one_database_reference() {
        let attachment = "a".repeat(8_000);
        let reference = reference_large_user_context(101, "important request", Some(&attachment))
            .expect("combined input should produce a reference");

        assert_eq!(reference.message_id, 101);
        assert!(reference.byte_count > attachment.len());
        assert!(reference.inline_preview.contains("## User message"));
        assert!(reference
            .inline_preview
            .contains("USER_CONTEXT_PREVIEW_TRUNCATED"));
        let metadata = merge_reference_metadata(None, &reference);
        assert_eq!(
            reference_from_metadata(Some(&metadata)),
            Some(reference),
            "metadata is an AI-projection hint; the full input stays in workflow_messages"
        );
    }

    #[test]
    fn legacy_file_reference_is_read_without_using_its_path() {
        let metadata = json!({
            "user_context_reference": {
                "message_id": 7,
                "relative_path": ".cs/old/user_question.md",
                "sha256": "legacy",
                "byte_count": 10,
                "token_estimate": 3,
                "externalized": true
            }
        });
        let reference = reference_from_metadata(Some(&metadata)).expect("legacy reference");
        assert!(!reference.projection_marker().contains("relative_path"));
        assert!(reference
            .projection_marker()
            .contains("read_history_message"));
    }
}
