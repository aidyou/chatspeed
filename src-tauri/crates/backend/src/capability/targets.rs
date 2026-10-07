//! Fixed registry of Agent Skill install targets.
//!
//! The target set is closed on purpose: a caller may select a target by id,
//! never by path, so no request can write to an arbitrary directory
//! (AC-3/AC-4/INV-5). Every external target carries the official convention it
//! was verified against; a target whose skills directory could not be verified
//! against an authoritative source stays `unsupported` and is refused instead
//! of guessed (D-5).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::error::CapabilityError;

/// The only target ChatSpeed manages itself.
pub const CHATSPEED_TARGET: &str = "chatspeed";

/// The Agent Skills directory convention shared by the `agents` target. It is
/// the same location the workflow `SkillScanner` already reads, so an install
/// there becomes visible to a rescan without a second discovery path.
const AGENTS_HOME_RELATIVE: &[&str] = &[".agents", "skills"];

/// Claude Code personal skills directory.
///
/// Verified against the official Claude Code skills documentation
/// (<https://docs.claude.com/en/docs/claude-code/skills>): personal skills are
/// loaded from `~/.claude/skills/<skill-name>/SKILL.md`.
const CLAUDE_CODE_HOME_RELATIVE: &[&str] = &[".claude", "skills"];

/// OpenCode global skills directory.
///
/// Verified against the official OpenCode skills documentation
/// (<https://opencode.ai/docs/skills/>): global skills are discovered from
/// `~/.config/opencode/skills/<name>/SKILL.md`, and the same page documents the
/// `.agents/skills` compatibility location used above.
const OPENCODE_HOME_RELATIVE: &[&str] = &[".config", "opencode", "skills"];

/// Global skill locations documented by the respective tool vendors.
const CURSOR_HOME_RELATIVE: &[&str] = &[".cursor", "skills"];
const WINDSURF_HOME_RELATIVE: &[&str] = &[".codeium", "windsurf", "skills"];
const CLINE_HOME_RELATIVE: &[&str] = &[".cline", "skills"];
const CODEX_HOME_RELATIVE: &[&str] = &[".codex", "skills"];

/// Trae CN (the China edition) personal skills directory.
///
/// The CN edition keeps its own profile directory, so its skills live under
/// `~/.trae-cn/skills`. It is registered as its own target; the international
/// build has no separate skills directory (it reads the shared Agents
/// directory), so it is not aliased here.
const TRAE_CN_HOME_RELATIVE: &[&str] = &[".trae-cn", "skills"];

/// Why a target cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetUnsupportedReason {
    /// No authoritative source for this tool's skills directory was found, so
    /// ChatSpeed refuses to guess a path.
    PathNotVerified,
}

impl TargetUnsupportedReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            TargetUnsupportedReason::PathNotVerified => "path_not_verified",
        }
    }
}

/// The stable identity of an install target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkillTargetId {
    Chatspeed,
    Agents,
    ClaudeCode,
    Codex,
    Opencode,
    Cursor,
    Windsurf,
    Cline,
    Trae,
    TraeCn,
    Dsh,
    Workbuddy,
    Qoder,
}

impl SkillTargetId {
    /// Every registered target id, in registry order.
    pub const ALL: &'static [SkillTargetId] = &[
        SkillTargetId::Chatspeed,
        SkillTargetId::Agents,
        SkillTargetId::ClaudeCode,
        SkillTargetId::Codex,
        SkillTargetId::Opencode,
        SkillTargetId::Cursor,
        SkillTargetId::Windsurf,
        SkillTargetId::Cline,
        SkillTargetId::Trae,
        SkillTargetId::TraeCn,
        SkillTargetId::Dsh,
        SkillTargetId::Workbuddy,
        SkillTargetId::Qoder,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            SkillTargetId::Chatspeed => "chatspeed",
            SkillTargetId::Agents => "agents",
            SkillTargetId::ClaudeCode => "claude-code",
            SkillTargetId::Codex => "codex",
            SkillTargetId::Opencode => "opencode",
            SkillTargetId::Cursor => "cursor",
            SkillTargetId::Windsurf => "windsurf",
            SkillTargetId::Cline => "cline",
            SkillTargetId::Trae => "trae",
            SkillTargetId::TraeCn => "trae-cn",
            SkillTargetId::Dsh => "dsh",
            SkillTargetId::Workbuddy => "workbuddy",
            SkillTargetId::Qoder => "qoder",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == value)
    }

    /// Whether the target is the ChatSpeed-managed directory.
    pub fn is_chatspeed(&self) -> bool {
        matches!(self, SkillTargetId::Chatspeed)
    }

    /// Whether the target is outside ChatSpeed's own directory and therefore
    /// requires an explicit selection on every mutation.
    pub fn is_external(&self) -> bool {
        !self.is_chatspeed()
    }

    pub fn spec(&self) -> &'static SkillTargetSpec {
        REGISTRY
            .iter()
            .find(|spec| spec.id == *self)
            .unwrap_or(&REGISTRY[0])
    }
}

impl std::fmt::Display for SkillTargetId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A registered target and the convention it was verified against.
#[derive(Debug, Clone, Copy)]
pub struct SkillTargetSpec {
    pub id: SkillTargetId,
    /// Whether an install with no explicit target list includes this target.
    pub default_selected: bool,
    /// Path segments under `HOME`, or `None` when unverified.
    pub home_relative: Option<&'static [&'static str]>,
    /// The authoritative source the path convention was verified against.
    pub verified_against: Option<&'static str>,
    pub unsupported_reason: Option<TargetUnsupportedReason>,
}

const REGISTRY: &[SkillTargetSpec] = &[
    SkillTargetSpec {
        id: SkillTargetId::Chatspeed,
        // ChatSpeed always installs into its own directory unless the caller
        // explicitly selects more targets (AC-3/INV-5).
        default_selected: true,
        home_relative: None,
        verified_against: None,
        unsupported_reason: None,
    },
    SkillTargetSpec {
        id: SkillTargetId::Agents,
        default_selected: false,
        home_relative: Some(AGENTS_HOME_RELATIVE),
        verified_against: Some("https://opencode.ai/docs/skills/"),
        unsupported_reason: None,
    },
    SkillTargetSpec {
        id: SkillTargetId::ClaudeCode,
        default_selected: false,
        home_relative: Some(CLAUDE_CODE_HOME_RELATIVE),
        verified_against: Some("https://docs.claude.com/en/docs/claude-code/skills"),
        unsupported_reason: None,
    },
    SkillTargetSpec {
        id: SkillTargetId::Codex,
        default_selected: false,
        home_relative: Some(CODEX_HOME_RELATIVE),
        verified_against: Some("https://github.com/openai/codex/tree/main/.codex/skills"),
        unsupported_reason: None,
    },
    SkillTargetSpec {
        id: SkillTargetId::Opencode,
        default_selected: false,
        home_relative: Some(OPENCODE_HOME_RELATIVE),
        verified_against: Some("https://opencode.ai/docs/skills/"),
        unsupported_reason: None,
    },
    SkillTargetSpec {
        id: SkillTargetId::Cursor,
        default_selected: false,
        home_relative: Some(CURSOR_HOME_RELATIVE),
        verified_against: Some("https://cursor.com/docs/context/skills"),
        unsupported_reason: None,
    },
    SkillTargetSpec {
        id: SkillTargetId::Windsurf,
        default_selected: false,
        home_relative: Some(WINDSURF_HOME_RELATIVE),
        verified_against: Some("https://docs.windsurf.com/windsurf/cascade/skills"),
        unsupported_reason: None,
    },
    SkillTargetSpec {
        id: SkillTargetId::Cline,
        default_selected: false,
        home_relative: Some(CLINE_HOME_RELATIVE),
        verified_against: Some("https://docs.cline.bot/features/skills"),
        unsupported_reason: None,
    },
    SkillTargetSpec {
        id: SkillTargetId::Trae,
        default_selected: false,
        // The international Trae build has no skills directory of its own, so
        // a separate `trae` target would only duplicate the shared Agents
        // directory without a Trae-specific path. It stays registered but
        // refused instead of aliasing `.agents/skills`.
        home_relative: None,
        verified_against: None,
        unsupported_reason: Some(TargetUnsupportedReason::PathNotVerified),
    },
    SkillTargetSpec {
        id: SkillTargetId::TraeCn,
        default_selected: false,
        home_relative: Some(TRAE_CN_HOME_RELATIVE),
        verified_against: Some("https://docs.trae.cn/ide/skills"),
        unsupported_reason: None,
    },
    SkillTargetSpec {
        id: SkillTargetId::Dsh,
        default_selected: false,
        home_relative: None,
        verified_against: None,
        unsupported_reason: Some(TargetUnsupportedReason::PathNotVerified),
    },
    SkillTargetSpec {
        id: SkillTargetId::Workbuddy,
        default_selected: false,
        home_relative: None,
        verified_against: None,
        unsupported_reason: Some(TargetUnsupportedReason::PathNotVerified),
    },
    SkillTargetSpec {
        id: SkillTargetId::Qoder,
        default_selected: false,
        home_relative: None,
        verified_against: None,
        unsupported_reason: Some(TargetUnsupportedReason::PathNotVerified),
    },
];

/// Every registered target specification, in registry order.
pub fn registry() -> &'static [SkillTargetSpec] {
    REGISTRY
}

/// The environment the target paths resolve against.
///
/// It is explicit so tests can inject a temporary HOME instead of touching the
/// real user profile (V-4).
#[derive(Debug, Clone)]
pub struct TargetEnvironment {
    pub home_dir: Option<PathBuf>,
    pub chatspeed_home: Option<PathBuf>,
}

impl TargetEnvironment {
    /// Resolves the real environment: `HOME` plus `${CHATSPEED_HOME:-~/.chatspeed}`.
    pub fn detect() -> Self {
        let home_dir = dirs::home_dir();
        let chatspeed_home = chatspeed_home(&home_dir);
        Self {
            home_dir,
            chatspeed_home,
        }
    }

    /// An injected environment for tests and hosted runs.
    pub fn injected(home_dir: PathBuf, chatspeed_home: PathBuf) -> Self {
        Self {
            home_dir: Some(home_dir),
            chatspeed_home: Some(chatspeed_home),
        }
    }

    /// The ChatSpeed-managed skills directory.
    pub fn chatspeed_skills_dir(&self) -> Option<PathBuf> {
        self.chatspeed_home.as_ref().map(|root| root.join("skills"))
    }
}

/// `${CHATSPEED_HOME}` when set, otherwise `~/.chatspeed`.
pub fn chatspeed_home(home_dir: &Option<PathBuf>) -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("CHATSPEED_HOME") {
        if !value.is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    home_dir.as_ref().map(|home| home.join(".chatspeed"))
}

/// A target resolved against one environment.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedSkillTarget {
    pub id: String,
    pub default_selected: bool,
    pub external: bool,
    pub supported: bool,
    /// The absolute install directory, present only when `supported`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Why the target is unsupported, present only when it is not supported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsupported_reason: Option<String>,
    /// The authoritative source the path convention was verified against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_against: Option<String>,
}

/// Resolves every registered target against an environment.
///
/// The ChatSpeed directory is always supported. An external target is
/// supported only when its documented path could be verified *and* `HOME`
/// resolved; otherwise it is reported `unsupported` and any mutation against
/// it is refused rather than guessed.
pub fn resolve_targets(environment: &TargetEnvironment) -> Vec<ResolvedSkillTarget> {
    REGISTRY
        .iter()
        .map(|spec| resolve_target(spec, environment))
        .collect()
}

fn resolve_target(spec: &SkillTargetSpec, environment: &TargetEnvironment) -> ResolvedSkillTarget {
    let mut resolved = ResolvedSkillTarget {
        id: spec.id.as_str().to_string(),
        default_selected: spec.default_selected,
        external: spec.id.is_external(),
        supported: false,
        path: None,
        unsupported_reason: None,
        verified_against: spec.verified_against.map(|value| value.to_string()),
    };

    if let Some(reason) = spec.unsupported_reason {
        resolved.unsupported_reason = Some(reason.as_str().to_string());
        return resolved;
    }

    let path = if spec.id.is_chatspeed() {
        environment.chatspeed_skills_dir()
    } else {
        match (spec.home_relative, environment.home_dir.as_ref()) {
            (Some(segments), Some(home)) => {
                let mut path = home.clone();
                for segment in segments {
                    path.push(segment);
                }
                Some(path)
            }
            _ => None,
        }
    };

    match path {
        Some(path) => {
            resolved.supported = true;
            resolved.path = Some(path.to_string_lossy().to_string());
        }
        None => {
            resolved.unsupported_reason = Some(
                TargetUnsupportedReason::PathNotVerified
                    .as_str()
                    .to_string(),
            );
        }
    }

    resolved
}

/// Resolves one target, failing closed for an unknown or unsupported id.
pub fn resolve_target_or_error(
    target_id: &str,
    environment: &TargetEnvironment,
) -> Result<(SkillTargetId, PathBuf), CapabilityError> {
    let id = SkillTargetId::parse(target_id).ok_or_else(|| {
        CapabilityError::invalid_request(format!("unknown skill target '{target_id}'"))
    })?;
    let resolved = resolve_target(id.spec(), environment);
    match (resolved.supported, resolved.path) {
        (true, Some(path)) => Ok((id, PathBuf::from(path))),
        _ => Err(CapabilityError::unsupported_target(format!(
            "skill target '{target_id}' has no verified directory ({})",
            resolved
                .unsupported_reason
                .unwrap_or_else(|| "path_not_verified".to_string())
        ))),
    }
}

/// The default selection: exactly the ChatSpeed directory (AC-3/INV-5).
pub fn default_target_selection() -> Vec<SkillTargetId> {
    REGISTRY
        .iter()
        .filter(|spec| spec.default_selected)
        .map(|spec| spec.id)
        .collect()
}

/// Resolves an explicit selection, rejecting unknown or unsupported ids.
///
/// An empty selection means "the default", never "everything".
pub fn resolve_selection(
    selection: &[String],
    environment: &TargetEnvironment,
) -> Result<Vec<(SkillTargetId, PathBuf)>, CapabilityError> {
    if selection.is_empty() {
        let mut resolved = Vec::new();
        for id in default_target_selection() {
            resolved.push(resolve_target_or_error(id.as_str(), environment)?);
        }
        return Ok(resolved);
    }

    let mut resolved: Vec<(SkillTargetId, PathBuf)> = Vec::new();
    for value in selection {
        let entry = resolve_target_or_error(value, environment)?;
        if !resolved.iter().any(|(existing, _)| *existing == entry.0) {
            resolved.push(entry);
        }
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn environment() -> (TempDir, TargetEnvironment) {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().join("home");
        let chatspeed = temp.path().join("chatspeed");
        (temp, TargetEnvironment::injected(home, chatspeed))
    }

    #[test]
    fn the_default_selection_is_only_the_chatspeed_directory() {
        let (_temp, environment) = environment();
        let selection = resolve_selection(&[], &environment).expect("default selection");
        assert_eq!(selection.len(), 1);
        assert_eq!(selection[0].0, SkillTargetId::Chatspeed);
        assert!(selection[0].1.ends_with("skills"));
        assert!(selection[0]
            .1
            .starts_with(&environment.chatspeed_home.clone().unwrap()));
    }

    #[test]
    fn every_registered_target_is_listed_and_only_verified_targets_are_supported() {
        let (_temp, environment) = environment();
        let resolved = resolve_targets(&environment);
        assert_eq!(resolved.len(), SkillTargetId::ALL.len());

        let supported: Vec<&str> = resolved
            .iter()
            .filter(|entry| entry.supported)
            .map(|entry| entry.id.as_str())
            .collect();
        for id in [
            "chatspeed",
            "agents",
            "claude-code",
            "opencode",
            "codex",
            "cursor",
            "windsurf",
            "cline",
            "trae-cn",
        ] {
            assert!(supported.contains(&id), "expected {id} supported");
        }

        // A registered target without a reliable directory is reported
        // unsupported instead of being guessed.
        let unsupported: Vec<&str> = resolved
            .iter()
            .filter(|entry| !entry.supported)
            .map(|entry| entry.id.as_str())
            .collect();
        for id in ["trae", "dsh", "workbuddy", "qoder"] {
            assert!(unsupported.contains(&id), "expected {id} unsupported");
        }

        // A supported external target resolves under HOME; the ChatSpeed
        // target resolves under CHATSPEED_HOME.
        let claude = resolved
            .iter()
            .find(|entry| entry.id == "claude-code")
            .expect("claude-code entry");
        assert!(claude
            .path
            .as_deref()
            .unwrap_or_default()
            .contains(".claude"));
    }

    #[test]
    fn verified_external_targets_resolve_to_documented_directories() {
        let (_temp, environment) = environment();
        let resolved = resolve_targets(&environment);
        for (id, suffix) in [
            ("codex", ".codex/skills"),
            ("cursor", ".cursor/skills"),
            ("windsurf", ".codeium/windsurf/skills"),
            ("cline", ".cline/skills"),
            ("trae-cn", ".trae-cn/skills"),
        ] {
            let target = resolved
                .iter()
                .find(|entry| entry.id == id)
                .expect("target");
            assert!(target.supported);
            assert!(target.path.as_deref().unwrap_or_default().ends_with(suffix));
            assert!(target.verified_against.is_some());
        }
    }

    #[test]
    fn explicit_target_selection_is_resolved_for_every_verified_tool() {
        let (_temp, environment) = environment();
        let selection = resolve_selection(
            &[
                "codex".to_string(),
                "cursor".to_string(),
                "windsurf".to_string(),
                "cline".to_string(),
                "trae-cn".to_string(),
            ],
            &environment,
        )
        .expect("verified target selection");
        assert_eq!(selection.len(), 5);
    }

    #[test]
    fn unknown_target_selection_is_refused() {
        let (_temp, environment) = environment();
        let unknown = resolve_selection(&["not-a-target".to_string()], &environment)
            .expect_err("unknown target must be refused");
        assert_eq!(unknown.code(), super::super::error::code::INVALID_REQUEST);
    }

    #[test]
    fn an_explicit_selection_is_deduplicated_and_never_adds_chatspeed_implicitly() {
        let (_temp, environment) = environment();
        let selection = resolve_selection(
            &[
                "claude-code".to_string(),
                "claude-code".to_string(),
                "agents".to_string(),
            ],
            &environment,
        )
        .expect("explicit selection");
        assert_eq!(selection.len(), 2);
        assert_eq!(selection[0].0, SkillTargetId::ClaudeCode);
        assert_eq!(selection[1].0, SkillTargetId::Agents);
    }

    #[test]
    fn the_registry_carries_every_required_target_and_one_default() {
        let ids: Vec<&str> = SkillTargetId::ALL.iter().map(|id| id.as_str()).collect();
        for required in [
            "chatspeed",
            "agents",
            "codex",
            "claude-code",
            "cursor",
            "windsurf",
            "opencode",
            "dsh",
            "workbuddy",
            "qoder",
            "trae",
            "trae-cn",
        ] {
            assert!(ids.contains(&required), "registry is missing {required}");
        }

        let defaults: Vec<&str> = REGISTRY
            .iter()
            .filter(|spec| spec.default_selected)
            .map(|spec| spec.id.as_str())
            .collect();
        assert_eq!(defaults, vec![CHATSPEED_TARGET]);

        // Every id parses back to itself, so only the closed ids are accepted.
        for id in SkillTargetId::ALL {
            assert_eq!(SkillTargetId::parse(id.as_str()), Some(*id));
        }
    }

    #[test]
    fn unverified_targets_are_unsupported_with_path_not_verified() {
        let (_temp, environment) = environment();
        let resolved = resolve_targets(&environment);
        for id in ["dsh", "workbuddy", "qoder", "trae"] {
            let target = resolved
                .iter()
                .find(|entry| entry.id == id)
                .expect("target");
            assert!(!target.supported, "{id} must be unsupported");
            assert!(target.path.is_none(), "{id} must not expose a path");
            assert_eq!(
                target.unsupported_reason.as_deref(),
                Some("path_not_verified"),
                "{id} reason"
            );
            assert!(target.verified_against.is_none(), "{id} source");
        }
    }

    #[test]
    fn agents_resolves_to_the_canonical_agents_skills_directory() {
        let (_temp, environment) = environment();
        let resolved = resolve_targets(&environment);
        let agents = resolved
            .iter()
            .find(|entry| entry.id == "agents")
            .expect("agents target");
        assert!(agents.supported);
        assert!(agents
            .path
            .as_deref()
            .unwrap_or_default()
            .ends_with(".agents/skills"));
    }

    #[test]
    fn trae_cn_uses_its_own_directory_and_trae_no_longer_aliases_agents() {
        let (_temp, environment) = environment();
        let resolved = resolve_targets(&environment);

        let trae_cn = resolved
            .iter()
            .find(|entry| entry.id == "trae-cn")
            .expect("trae-cn target");
        assert!(trae_cn.supported);
        let trae_cn_path = trae_cn.path.as_deref().unwrap_or_default();
        assert!(trae_cn_path.ends_with(".trae-cn/skills"));
        assert!(!trae_cn_path.contains(".agents/skills"));

        let trae = resolved
            .iter()
            .find(|entry| entry.id == "trae")
            .expect("trae target");
        assert!(!trae.supported);
        assert!(trae.path.is_none());
    }

    #[test]
    fn a_selection_is_resolved_by_id_only_and_refuses_paths_and_unsupported_ids() {
        let (_temp, environment) = environment();

        // A caller never supplies a directory: even a path-shaped string is an
        // unknown id and is refused before any path is derived from it.
        for value in ["/tmp/somewhere", "~/.agents/skills", "../escape"] {
            let error = resolve_selection(&[value.to_string()], &environment)
                .expect_err("path-shaped selection must be refused");
            assert_eq!(
                error.code(),
                super::super::error::code::INVALID_REQUEST,
                "{value} must be an unknown id"
            );
        }

        // A registered but unverified id is refused as unsupported.
        let error = resolve_selection(&["qoder".to_string()], &environment)
            .expect_err("unsupported id must be refused");
        assert_eq!(error.code(), super::super::error::code::UNSUPPORTED_TARGET);
    }
}
