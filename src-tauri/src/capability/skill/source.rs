//! Agent Skill source resolution.
//!
//! A source is a strict, tagged document: there is no way to express "just
//! read this arbitrary path from the network". Local directories are read in
//! place (read-only); archives and remote sources are always materialized into
//! private staging first (AC-5). Unknown fields are rejected so a caller cannot
//! smuggle an extra directive past the contract.

use serde::{Deserialize, Serialize};

use crate::capability::error::CapabilityError;

/// The only remote host the checker will talk to.
pub const GITHUB_HOST: &str = "github.com";
/// The download host GitHub redirects archives to.
pub const GITHUB_DOWNLOAD_HOST: &str = "codeload.github.com";

/// A structured Agent Skill source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillSource {
    /// A local directory, read in place.
    LocalDirectory { path: String },
    /// A local ZIP archive, extracted into private staging.
    LocalZip { path: String },
    /// A constrained GitHub repository (or a sub-path inside it).
    #[serde(rename = "github")]
    GitHub {
        owner: String,
        repo: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        git_ref: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// An Agent Skill already installed under a registered target.
    Installed { name: String },
}

impl SkillSource {
    /// A short, non-secret label for logs and DTOs.
    pub fn kind(&self) -> &'static str {
        match self {
            SkillSource::LocalDirectory { .. } => "local_directory",
            SkillSource::LocalZip { .. } => "local_zip",
            SkillSource::GitHub { .. } => "github",
            SkillSource::Installed { .. } => "installed",
        }
    }

    /// A redacted reference: for a local path only the final component is kept,
    /// so a journal row never records a user's directory layout.
    pub fn redacted_ref(&self) -> String {
        match self {
            SkillSource::LocalDirectory { path } | SkillSource::LocalZip { path } => {
                let name = std::path::Path::new(path)
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_else(|| "<path>".to_string());
                format!("{}:{name}", self.kind())
            }
            SkillSource::GitHub {
                owner,
                repo,
                git_ref,
                path,
            } => format!(
                "github:{owner}/{repo}@{}{}",
                git_ref.as_deref().unwrap_or("default"),
                path.as_deref().map(|p| format!("#{p}")).unwrap_or_default()
            ),
            SkillSource::Installed { name } => format!("installed:{name}"),
        }
    }

    /// Parses and validates a source document.
    pub fn parse(value: &serde_json::Value) -> Result<Self, CapabilityError> {
        let source: SkillSource = serde_json::from_value(value.clone()).map_err(|error| {
            CapabilityError::invalid_request(format!("invalid skill source: {error}"))
        })?;
        source.validate()?;
        Ok(source)
    }

    /// Validates the source against the closed contract.
    pub fn validate(&self) -> Result<(), CapabilityError> {
        match self {
            SkillSource::LocalDirectory { path } | SkillSource::LocalZip { path } => {
                if path.trim().is_empty() {
                    return Err(CapabilityError::invalid_request(
                        "a local skill source requires a non-empty path",
                    ));
                }
                if matches!(self, SkillSource::LocalZip { .. })
                    && !path.to_ascii_lowercase().ends_with(".zip")
                {
                    return Err(CapabilityError::invalid_request(
                        "a local archive source must be a .zip file",
                    ));
                }
                Ok(())
            }
            SkillSource::GitHub {
                owner,
                repo,
                git_ref,
                path,
            } => {
                validate_github_segment(owner, "owner")?;
                validate_github_segment(repo, "repository")?;
                if let Some(reference) = git_ref {
                    validate_github_segment(reference, "ref")?;
                }
                if let Some(sub_path) = path {
                    validate_relative_path(sub_path)?;
                }
                Ok(())
            }
            SkillSource::Installed { name } => {
                validate_skill_name(name)?;
                Ok(())
            }
        }
    }

    /// The archive download URL for a GitHub source.
    ///
    /// The URL is always built from the validated host constant; a caller can
    /// never inject a host or scheme.
    pub fn github_archive_url(&self) -> Result<String, CapabilityError> {
        match self {
            SkillSource::GitHub {
                owner,
                repo,
                git_ref,
                ..
            } => {
                let reference = git_ref.clone().unwrap_or_else(|| "HEAD".to_string());
                Ok(format!(
                    "https://{GITHUB_DOWNLOAD_HOST}/{owner}/{repo}/zip/{reference}"
                ))
            }
            _ => Err(CapabilityError::invalid_request(
                "this source is not a GitHub source",
            )),
        }
    }
}

/// A Skill name must match the Agent Skills convention.
pub fn validate_skill_name(name: &str) -> Result<(), CapabilityError> {
    if name.is_empty() || name.len() > 64 {
        return Err(CapabilityError::invalid_request(
            "a skill name must be 1-64 characters",
        ));
    }
    let mut previous_dash = true;
    for character in name.chars() {
        let valid = character.is_ascii_lowercase() || character.is_ascii_digit();
        if valid {
            previous_dash = false;
            continue;
        }
        if character == '-' && !previous_dash {
            previous_dash = true;
            continue;
        }
        return Err(CapabilityError::invalid_request(
            "a skill name must match ^[a-z0-9]+(-[a-z0-9]+)*$",
        ));
    }
    if previous_dash {
        return Err(CapabilityError::invalid_request(
            "a skill name must not end with '-'",
        ));
    }
    Ok(())
}

fn validate_github_segment(value: &str, field: &str) -> Result<(), CapabilityError> {
    if value.is_empty() || value.len() > 100 {
        return Err(CapabilityError::invalid_request(format!(
            "github {field} must be 1-100 characters"
        )));
    }
    let allowed = value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character));
    if !allowed || value.starts_with('.') || value == "." || value == ".." {
        return Err(CapabilityError::invalid_request(format!(
            "github {field} contains characters that are not allowed"
        )));
    }
    Ok(())
}

/// Validates a repository-relative path (no traversal, no absolute form).
pub fn validate_relative_path(value: &str) -> Result<(), CapabilityError> {
    if value.is_empty() || value.len() > 512 {
        return Err(CapabilityError::invalid_request(
            "a repository path must be 1-512 characters",
        ));
    }
    if value.starts_with('/') || value.starts_with('\\') || value.contains(':') {
        return Err(CapabilityError::invalid_request(
            "a repository path must be relative",
        ));
    }
    for segment in value.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(CapabilityError::invalid_request(
                "a repository path must not contain empty, '.' or '..' segments",
            ));
        }
        if segment.contains('\\') || segment.chars().any(|c| c.is_control()) {
            return Err(CapabilityError::invalid_request(
                "a repository path must not contain separators or control characters",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_local_source_requires_a_path_and_a_zip_extension() {
        let directory = SkillSource::parse(&json!({ "kind": "local_directory", "path": "/tmp/demo" }))
            .expect("directory source");
        assert_eq!(directory.kind(), "local_directory");

        assert!(SkillSource::parse(&json!({ "kind": "local_directory", "path": " " })).is_err());
        assert!(SkillSource::parse(&json!({ "kind": "local_zip", "path": "/tmp/demo.tar" })).is_err());
        assert!(SkillSource::parse(&json!({ "kind": "local_zip", "path": "/tmp/demo.zip" })).is_ok());
    }

    #[test]
    fn unknown_source_kinds_and_fields_are_rejected() {
        assert!(SkillSource::parse(&json!({ "kind": "http", "url": "https://example.test" })).is_err());
        assert!(SkillSource::parse(&json!({
            "kind": "local_directory",
            "path": "/tmp/demo",
            "extra": true
        }))
        .is_err());
    }

    #[test]
    fn a_github_source_is_validated_and_its_url_is_built_from_the_host_constant() {
        let source = SkillSource::parse(&json!({
            "kind": "github",
            "owner": "acme",
            "repo": "skills",
            "git_ref": "v1.2.3",
            "path": "skills/demo"
        }))
        .expect("github source");
        assert_eq!(
            source.github_archive_url().expect("url"),
            format!("https://{GITHUB_DOWNLOAD_HOST}/acme/skills/zip/v1.2.3")
        );

        // A traversal or absolute path can never reach the URL builder.
        assert!(SkillSource::parse(&json!({
            "kind": "github", "owner": "acme", "repo": "skills", "path": "../etc"
        }))
        .is_err());
        assert!(SkillSource::parse(&json!({
            "kind": "github", "owner": "acme", "repo": "skills", "path": "/etc/passwd"
        }))
        .is_err());
        assert!(SkillSource::parse(&json!({
            "kind": "github", "owner": "acme/../x", "repo": "skills"
        }))
        .is_err());
    }

    #[test]
    fn skill_names_follow_the_agent_skills_convention() {
        assert!(validate_skill_name("demo").is_ok());
        assert!(validate_skill_name("demo-skill-2").is_ok());
        for invalid in ["", "Demo", "-demo", "demo-", "de mo", "demo_skill", "a".repeat(65).as_str()] {
            assert!(validate_skill_name(invalid).is_err(), "{invalid} must be rejected");
        }
    }

    #[test]
    fn a_redacted_reference_never_exposes_a_full_local_path() {
        let source = SkillSource::LocalDirectory {
            path: "/home/alice/secret-project/skills".to_string(),
        };
        let reference = source.redacted_ref();
        assert_eq!(reference, "local_directory:skills");
        assert!(!reference.contains("alice"));
    }
}
