use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::ccproxy::utils::token_estimator::estimate_tokens;

pub const USER_CONTEXT_EXTERNALIZATION_TOKEN_THRESHOLD: f64 = 2048.0;
const USER_CONTEXT_FILE_NAME: &str = "user_question.md";
const USER_CONTEXT_METADATA_KEY: &str = "user_context_reference";

fn archive_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserContextReference {
    pub message_id: i64,
    pub relative_path: String,
    pub sha256: String,
    pub byte_count: usize,
    pub token_estimate: usize,
    pub externalized: bool,
}

impl UserContextReference {
    pub fn projection_marker(&self) -> String {
        format!(
            "<USER_CONTEXT_REFERENCE externalized=\"true\" message_id=\"{}\" path=\"{}\" sha256=\"{}\" bytes=\"{}\" tokens=\"{}\">\nThe complete supplemental user context is archived at this workspace-relative path. Read it only when the current task requires details from it. If the file is unavailable, use read_history_message with this message_id and role=user to recover the authoritative database record.\n</USER_CONTEXT_REFERENCE>",
            self.message_id,
            self.relative_path,
            self.sha256,
            self.byte_count,
            self.token_estimate
        )
    }
}

fn validate_session_id(session_id: &str) -> Result<(), String> {
    let mut components = Path::new(session_id).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(component)), None) if !component.is_empty() => Ok(()),
        _ => Err("session_id must be a single safe path component".to_string()),
    }
}

fn reference_path(session_id: &str) -> String {
    format!(".cs/{session_id}/{USER_CONTEXT_FILE_NAME}")
}

fn archive_path(planning_root: &Path, session_id: &str) -> PathBuf {
    planning_root.join(session_id).join(USER_CONTEXT_FILE_NAME)
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

pub async fn externalize_long_attached_context(
    planning_root: &Path,
    session_id: &str,
    message_id: i64,
    attached_context: &str,
) -> Result<Option<UserContextReference>, String> {
    let token_estimate = estimate_tokens(attached_context).ceil() as usize;
    if token_estimate <= USER_CONTEXT_EXTERNALIZATION_TOKEN_THRESHOLD as usize {
        return Ok(None);
    }

    validate_session_id(session_id)?;

    let digest = hex::encode(Sha256::digest(attached_context.as_bytes()));
    let reference = UserContextReference {
        message_id,
        relative_path: reference_path(session_id),
        sha256: digest.clone(),
        byte_count: attached_context.len(),
        token_estimate,
        externalized: true,
    };
    let marker = format!("<!-- user-context:message_id={message_id} sha256={digest} -->");
    let path = archive_path(planning_root, session_id);

    let _guard = archive_lock().lock().await;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("failed to create user context directory: {error}"))?;
    }

    if let Ok(existing) = tokio::fs::read_to_string(&path).await {
        if existing.contains(&marker) {
            return Ok(Some(reference));
        }
    }

    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let section = format!(
        "{marker}\n## Supplemental user context\n\n- Source message ID: {message_id}\n- Recorded at (Unix ms): {timestamp_ms}\n- SHA-256: `{digest}`\n- Bytes: {}\n- Estimated tokens: {token_estimate}\n\n{}\n\n",
        attached_context.len(),
        attached_context
    );

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .map_err(|error| format!("failed to open user context archive: {error}"))?;
    file.write_all(section.as_bytes())
        .await
        .map_err(|error| format!("failed to append user context archive: {error}"))?;
    file.flush()
        .await
        .map_err(|error| format!("failed to flush user context archive: {error}"))?;

    Ok(Some(reference))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "chatspeed-user-context-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn short_context_stays_inline() {
        let root = temp_root("short");
        let result = externalize_long_attached_context(&root, "session-1", 1, "small context")
            .await
            .expect("short context should be accepted");
        assert!(result.is_none());
        assert!(!root.exists());
    }

    #[tokio::test]
    async fn long_context_is_archived_once_with_compact_metadata() {
        let root = temp_root("long");
        let attached = "a".repeat(8000);
        let reference = externalize_long_attached_context(&root, "session-1", 101, &attached)
            .await
            .expect("long context should be archived")
            .expect("reference should be returned");
        externalize_long_attached_context(&root, "session-1", 101, &attached)
            .await
            .expect("retry should be idempotent");

        let archived = tokio::fs::read_to_string(root.join("session-1/user_question.md"))
            .await
            .expect("archive should be readable");
        assert_eq!(archived.matches(&attached).count(), 1);
        assert!(archived.contains("Source message ID: 101"));
        assert_eq!(reference.message_id, 101);
        assert_eq!(reference.relative_path, ".cs/session-1/user_question.md");
        assert!(!reference.projection_marker().contains(&attached));
        assert_eq!(reference.byte_count, attached.len());

        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn identical_context_from_distinct_messages_has_distinct_auditable_sections() {
        let root = temp_root("distinct-messages");
        let attached = "a".repeat(8000);
        let first = externalize_long_attached_context(&root, "session-1", 101, &attached)
            .await
            .expect("first message should archive")
            .expect("first reference");
        let second = externalize_long_attached_context(&root, "session-1", 102, &attached)
            .await
            .expect("second message should archive")
            .expect("second reference");

        let archived = tokio::fs::read_to_string(root.join("session-1/user_question.md"))
            .await
            .expect("archive should be readable");
        assert_eq!(archived.matches(&attached).count(), 2);
        assert!(archived.contains("Source message ID: 101"));
        assert!(archived.contains("Source message ID: 102"));
        assert_ne!(first.message_id, second.message_id);

        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn archive_write_failure_returns_error_without_reference() {
        let root = temp_root("write-failure");
        tokio::fs::write(&root, "not a directory")
            .await
            .expect("test root file should be created");
        let error = externalize_long_attached_context(&root, "session-1", 1, &"a".repeat(8000))
            .await
            .expect_err("file planning root must prevent archive creation");
        assert!(error.contains("failed to create user context directory"));
        assert!(reference_from_metadata(None).is_none());
        let _ = tokio::fs::remove_file(root).await;
    }

    #[tokio::test]
    async fn unsafe_session_id_is_rejected() {
        let root = temp_root("unsafe");
        let error = externalize_long_attached_context(&root, "../escape", 1, &"a".repeat(8000))
            .await
            .expect_err("unsafe session id must fail");
        assert!(error.contains("single safe path component"));
        assert!(!root.exists());
    }
}
