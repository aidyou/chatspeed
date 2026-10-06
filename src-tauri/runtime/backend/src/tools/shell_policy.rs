//! Shared shell approval-policy DTOs.
//!
//! These two types are the only part of the shell tool surface the desktop
//! crate compiles: agents persist `ShellPolicyRule` and commands serialize
//! `ShellDecision` for the approval UI, while the policy engine and the shell
//! executor themselves live in the runtime-only `shell` module.

/// Decision levels for shell auditing
#[derive(Debug, PartialEq, Clone)]
pub enum ShellDecision {
    Allow,
    Review(String),
    Deny(String),
}

impl serde::Serialize for ShellDecision {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Allow => serializer.serialize_str("allow"),
            Self::Review(reason) => serializer.serialize_str(&format!("review:{reason}")),
            Self::Deny(reason) => serializer.serialize_str(&format!("deny:{reason}")),
        }
    }
}

impl<'de> serde::Deserialize<'de> for ShellDecision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?.to_lowercase();
        match s.as_str() {
            "allow" => Ok(ShellDecision::Allow),
            s if s.starts_with("review") => {
                // Handle "review" or "review:reason" format
                let reason = if s.len() > 6 {
                    s[6..].trim_start_matches(':').to_string()
                } else {
                    "Requires review".to_string()
                };
                Ok(ShellDecision::Review(reason))
            }
            s if s.starts_with("deny") => {
                let reason = if s.len() > 4 {
                    s[4..].trim_start_matches(':').to_string()
                } else {
                    "Command denied".to_string()
                };
                Ok(ShellDecision::Deny(reason))
            }
            _ => Ok(ShellDecision::Review(
                "Unknown decision, requires review".to_string(),
            )),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ShellPolicyRule {
    pub pattern: String,
    pub decision: ShellDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
