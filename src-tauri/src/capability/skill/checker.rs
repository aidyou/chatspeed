//! The deterministic, non-LLM Agent Skill checker.
//!
//! The checker is the only safety gate that can authorize an install (INV-4).
//! It never executes Skill content, never runs a shell, never installs a
//! dependency and never calls a model: it reads the staged files and applies
//! static rules. Anything it cannot prove benign is `inconclusive`, which
//! installs nothing — the same as `blocked`. There is no `force`.
//!
//! Every report carries [`SKILL_CHECKER_VERSION`] so a later upgrade can be
//! told apart from an older verdict without rewriting history.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::capability::error::CapabilityError;
use crate::capability::skill::archive;
use crate::capability::skill::manifest::compute_file_manifest;
use crate::capability::skill::source::{validate_skill_name, SkillSource};
use crate::capability::skill::staging::StagingArea;
use crate::capability::targets::TargetEnvironment;

/// Rule vocabulary version. Bump whenever a rule is added, removed or changes
/// severity; recorded verdicts keep the version they were produced with.
pub const SKILL_CHECKER_VERSION: &str = "skill-checker.v1";

/// Maximum bytes of one file the checker will read as text.
pub const MAX_SCANNED_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Files whose content is text by contract.
const TEXT_EXTENSIONS: &[&str] = &[
    "md", "markdown", "txt", "json", "yaml", "yml", "toml", "csv", "tsv", "ini", "cfg", "conf",
    "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd", "py", "js", "mjs", "cjs", "ts", "tsx", "jsx",
    "rb", "go", "rs", "java", "kt", "swift", "c", "h", "cc", "cpp", "hpp", "cs", "php", "pl", "lua",
    "sql", "html", "htm", "css", "scss", "xml", "svg",
];

/// The outcome of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckVerdict {
    /// Every rule passed; the content may be installed.
    Pass,
    /// The content is known to be unsafe; nothing may be installed.
    Blocked,
    /// The content could not be proven safe; nothing may be installed.
    Inconclusive,
}

impl CheckVerdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckVerdict::Pass => "pass",
            CheckVerdict::Blocked => "blocked",
            CheckVerdict::Inconclusive => "inconclusive",
        }
    }
}

/// How serious one finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckSeverity {
    /// Refuses the content.
    Block,
    /// Refuses the content because it could not be analyzed.
    Inconclusive,
    /// Recorded for the user; does not refuse the content.
    Warn,
}

/// One rule result.
#[derive(Debug, Clone, Serialize)]
pub struct SkillFinding {
    pub rule: String,
    pub severity: CheckSeverity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub detail: String,
}

/// The declared capability profile implied by the findings.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SkillPermissions {
    pub reads_files: bool,
    pub executes_processes: bool,
    pub uses_network: bool,
    pub downloads_content: bool,
    pub uses_dynamic_eval: bool,
    pub reads_credentials: bool,
    pub writes_sensitive_paths: bool,
    pub requires_elevation: bool,
    pub contains_binary_content: bool,
}

/// The full, deterministic report for one source.
#[derive(Debug, Clone, Serialize)]
pub struct SkillCheckReport {
    pub checker_version: String,
    pub verdict: CheckVerdict,
    pub findings: Vec<SkillFinding>,
    pub permissions: SkillPermissions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
    pub file_count: usize,
    pub total_bytes: u64,
    pub source_kind: String,
    pub source_ref: String,
}

impl SkillCheckReport {
    /// Whether the content may be installed.
    pub fn is_pass(&self) -> bool {
        self.verdict == CheckVerdict::Pass
    }

    /// Checks the report is internally consistent.
    ///
    /// A tampered or hand-built report must never authorize an install, so the
    /// verdict has to be exactly what the findings imply and the rules must be
    /// the ones this checker version ships.
    pub fn validate(&self) -> Result<(), CapabilityError> {
        if self.checker_version != SKILL_CHECKER_VERSION {
            return Err(CapabilityError::refused(
                "the skill check report was produced by a different checker version",
            ));
        }
        let expected = if self
            .findings
            .iter()
            .any(|finding| finding.severity == CheckSeverity::Block)
        {
            CheckVerdict::Blocked
        } else if self
            .findings
            .iter()
            .any(|finding| finding.severity == CheckSeverity::Inconclusive)
        {
            CheckVerdict::Inconclusive
        } else {
            CheckVerdict::Pass
        };
        if self.verdict != expected {
            return Err(CapabilityError::internal(
                "the skill check report verdict does not match its findings",
            ));
        }
        Ok(())
    }

    /// The error a mutation must raise when the verdict is not `pass`.
    pub fn refusal(&self) -> Option<CapabilityError> {
        match self.verdict {
            CheckVerdict::Pass => None,
            CheckVerdict::Blocked => Some(CapabilityError::new(
                crate::capability::error::code::CHECK_BLOCKED,
                format!(
                    "the skill was refused by {} rules",
                    self.findings
                        .iter()
                        .filter(|finding| finding.severity == CheckSeverity::Block)
                        .count()
                ),
            )),
            CheckVerdict::Inconclusive => Some(CapabilityError::new(
                crate::capability::error::code::CHECK_INCONCLUSIVE,
                "the skill content could not be proven safe",
            )),
        }
    }
}

/// Checks one already-materialized Skill directory.
///
/// This is the function install and the standalone check share, so a
/// successful install can never have been authorized by a different gate.
pub fn check_directory(root: &Path) -> Result<SkillCheckReport, CapabilityError> {
    if !root.is_dir() {
        return Err(CapabilityError::invalid_request(
            "the skill source is not a readable directory",
        ));
    }

    let mut findings = Vec::new();
    let manifest = match compute_file_manifest(root) {
        Ok(entries) => entries,
        Err(error) => {
            // A symlink, special file or size breach inside the content is a
            // refusal, not a crash.
            if error.code() == crate::capability::error::code::REFUSED {
                findings.push(SkillFinding {
                    rule: "skill.content.unsafe_entry".to_string(),
                    severity: CheckSeverity::Block,
                    path: None,
                    detail: error.redacted_message(),
                });
                return Ok(finish_report(
                    findings,
                    None,
                    None,
                    0,
                    0,
                    "directory",
                    "directory",
                ));
            }
            return Err(error);
        }
    };

    let total_bytes = manifest
        .iter()
        .map(|entry| entry.size_bytes.max(0) as u64)
        .sum();
    let content_digest = manifest_digest(&manifest);

    // The declared manifest is mandatory: an unnamed bundle cannot be reasoned
    // about and cannot be installed under a stable name.
    let skill_name = match read_declared_name(root) {
        Ok(Some(name)) => Some(name),
        Ok(None) => {
            findings.push(SkillFinding {
                rule: "skill.manifest.missing".to_string(),
                severity: CheckSeverity::Block,
                path: None,
                detail: "the skill has no SKILL.md front matter and no skill.json manifest"
                    .to_string(),
            });
            None
        }
        Err(detail) => {
            findings.push(SkillFinding {
                rule: "skill.manifest.invalid".to_string(),
                severity: CheckSeverity::Block,
                path: None,
                detail,
            });
            None
        }
    };

    for entry in &manifest {
        let path = root.join(&entry.path);
        let relative = entry.path.clone();
        let extension = Path::new(&entry.path)
            .extension()
            .map(|value| value.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();

        if entry.size_bytes as u64 > MAX_SCANNED_FILE_BYTES && !TEXT_EXTENSIONS.contains(&extension.as_str()) {
            findings.push(SkillFinding {
                rule: "skill.content.oversize".to_string(),
                severity: CheckSeverity::Inconclusive,
                path: Some(relative),
                detail: format!(
                    "the file is larger than the {MAX_SCANNED_FILE_BYTES}-byte scan limit and has no text extension"
                ),
            });
            continue;
        }

        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                findings.push(SkillFinding {
                    rule: "skill.content.unreadable".to_string(),
                    severity: CheckSeverity::Inconclusive,
                    path: Some(relative),
                    detail: "the file could not be read".to_string(),
                });
                continue;
            }
        };

        let is_declared_text = TEXT_EXTENSIONS.contains(&extension.as_str());
        let text = match std::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(_) => {
                findings.push(SkillFinding {
                    rule: "skill.content.binary".to_string(),
                    severity: CheckSeverity::Inconclusive,
                    path: Some(relative),
                    detail: "the file is not valid UTF-8 text".to_string(),
                });
                continue;
            }
        };

        if !is_declared_text && looks_binary(text) {
            findings.push(SkillFinding {
                rule: "skill.content.binary".to_string(),
                severity: CheckSeverity::Inconclusive,
                path: Some(relative),
                detail: "the file has an unknown extension and binary content".to_string(),
            });
            continue;
        }

        scan_text(&relative, text, &mut findings);
    }

    if let (Some(declared), Some(_)) = (&skill_name, &content_digest) {
        if let Err(error) = validate_skill_name(declared) {
            findings.push(SkillFinding {
                rule: "skill.manifest.invalid_name".to_string(),
                severity: CheckSeverity::Block,
                path: None,
                detail: error.redacted_message(),
            });
        }
    }

    Ok(finish_report(
        findings,
        skill_name,
        content_digest,
        manifest.len(),
        total_bytes,
        "directory",
        "directory",
    ))
}

/// A source that has been materialized somewhere readable.
///
/// Holds the staging area it owns, if any, so the content stays available from
/// `check` until `install` finishes and is then removed.
pub struct MaterializedSource {
    root: PathBuf,
    pub source_kind: String,
    pub source_ref: String,
    staging: Option<StagingArea>,
}

impl MaterializedSource {
    /// The directory that holds the materialized content.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether this source was materialized into staging (rather than read in
    /// place from a local directory).
    pub fn is_staged(&self) -> bool {
        self.staging.is_some()
    }
}

/// Resolves, materializes and checks a structured source.
///
/// Archives and remote sources are extracted into private staging; a local
/// directory is read in place, read-only.
pub struct SkillSourceResolver {
    app_data_dir: PathBuf,
    /// Test seam: overrides the GitHub download origin. Production always uses
    /// the constant inside [`SkillSource::github_archive_url`].
    download_origin: Option<String>,
}

impl SkillSourceResolver {
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self {
            app_data_dir,
            download_origin: None,
        }
    }

    #[cfg(test)]
    fn with_download_origin(app_data_dir: PathBuf, origin: String) -> Self {
        Self {
            app_data_dir,
            download_origin: Some(origin),
        }
    }

    /// Materializes one source into a private, readable directory.
    ///
    /// The returned handle owns the staging area it may have created, so the
    /// caller can check and then install the *same* bytes; dropping the handle
    /// removes the staging tree. A local directory is referenced in place and
    /// is never modified.
    pub async fn materialize(
        &self,
        source: &SkillSource,
        environment: &TargetEnvironment,
        operation_id: &str,
    ) -> Result<MaterializedSource, CapabilityError> {
        source.validate()?;
        let source_kind = source.kind().to_string();
        let source_ref = source.redacted_ref();
        match source {
            SkillSource::LocalDirectory { path } => Ok(MaterializedSource {
                root: PathBuf::from(path),
                source_kind,
                source_ref,
                staging: None,
            }),
            SkillSource::LocalZip { path } => {
                let staging = StagingArea::create(&self.app_data_dir, operation_id)?;
                archive::extract_zip(Path::new(path), staging.root())?;
                let root = archive::single_root_child(staging.root())
                    .unwrap_or_else(|| staging.root().to_path_buf());
                Ok(MaterializedSource {
                    root,
                    source_kind,
                    source_ref,
                    staging: Some(staging),
                })
            }
            SkillSource::GitHub { path, .. } => {
                let staging = StagingArea::create(&self.app_data_dir, operation_id)?;
                let archive_path = staging.root().join("source.zip");
                self.download_github_archive(source, &archive_path).await?;
                let extracted = staging.root().join("content");
                archive::extract_zip(&archive_path, &extracted)?;
                let root = archive::single_root_child(&extracted).unwrap_or(extracted);
                let root = match path {
                    Some(sub_path) => {
                        crate::capability::skill::source::validate_relative_path(sub_path)?;
                        root.join(sub_path)
                    }
                    None => root,
                };
                Ok(MaterializedSource {
                    root,
                    source_kind,
                    source_ref,
                    staging: Some(staging),
                })
            }
            SkillSource::Installed { name } => {
                validate_skill_name(name)?;
                let root = environment
                    .chatspeed_skills_dir()
                    .ok_or_else(|| {
                        CapabilityError::invalid_request(
                            "the ChatSpeed skills directory could not be resolved",
                        )
                    })?
                    .join(name);
                if !root.is_dir() {
                    return Err(CapabilityError::not_found(format!(
                        "no installed skill named '{name}' was found"
                    )));
                }
                Ok(MaterializedSource {
                    root,
                    source_kind,
                    source_ref,
                    staging: None,
                })
            }
        }
    }

    /// Checks one source and returns its report.
    ///
    /// Materialization and checking are the same two steps install performs, so
    /// the standalone command can never disagree with the install gate (AC-6).
    pub async fn check(
        &self,
        source: &SkillSource,
        environment: &TargetEnvironment,
        operation_id: &str,
    ) -> Result<SkillCheckReport, CapabilityError> {
        let materialized = self.materialize(source, environment, operation_id).await?;
        let mut report = check_directory(materialized.root())?;
        report.source_kind = materialized.source_kind.clone();
        report.source_ref = materialized.source_ref.clone();
        Ok(report)
    }

    /// Downloads a GitHub source archive into staging.
    ///
    /// Any transport, status or size problem is `inconclusive`: an unreachable
    /// source must never be mistaken for a clean one.
    async fn download_github_archive(
        &self,
        source: &SkillSource,
        destination: &Path,
    ) -> Result<(), CapabilityError> {
        let url = match &self.download_origin {
            Some(origin) => {
                let suffix = source
                    .github_archive_url()?
                    .split_once("://")
                    .and_then(|(_, rest)| rest.split_once('/'))
                    .map(|(_, path)| path.to_string())
                    .unwrap_or_default();
                format!("{origin}/{suffix}")
            }
            None => source.github_archive_url()?,
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::limited(4))
            .build()
            .map_err(|error| {
                CapabilityError::internal(format!("failed to build the download client: {error}"))
            })?;

        let response = client.get(&url).send().await.map_err(|_| {
            CapabilityError::new(
                crate::capability::error::code::CHECK_INCONCLUSIVE,
                "the GitHub source could not be reached",
            )
        })?;
        if !response.status().is_success() {
            return Err(CapabilityError::new(
                crate::capability::error::code::CHECK_INCONCLUSIVE,
                format!("the GitHub source returned status {}", response.status().as_u16()),
            ));
        }

        let mut file = std::fs::File::create(destination).map_err(|error| {
            CapabilityError::internal(format!("failed to create the staged archive: {error}"))
        })?;
        let mut stream = response;
        let mut written: u64 = 0;
        while let Some(chunk) = stream.chunk().await.map_err(|_| {
            CapabilityError::new(
                crate::capability::error::code::CHECK_INCONCLUSIVE,
                "the GitHub download was interrupted",
            )
        })? {
            written = written.saturating_add(chunk.len() as u64);
            if written > archive::DEFAULT_LIMITS.max_total_bytes {
                return Err(CapabilityError::new(
                    crate::capability::error::code::CHECK_INCONCLUSIVE,
                    "the GitHub archive exceeds the download size limit",
                ));
            }
            std::io::Write::write_all(&mut file, &chunk).map_err(|error| {
                CapabilityError::internal(format!("failed to write the staged archive: {error}"))
            })?;
        }
        Ok(())
    }
}

fn finish_report(
    mut findings: Vec<SkillFinding>,
    skill_name: Option<String>,
    content_digest: Option<String>,
    file_count: usize,
    total_bytes: u64,
    source_kind: &str,
    source_ref: &str,
) -> SkillCheckReport {
    findings.sort_by(|left, right| {
        left.rule
            .cmp(&right.rule)
            .then_with(|| left.path.cmp(&right.path))
    });
    let verdict = if findings
        .iter()
        .any(|finding| finding.severity == CheckSeverity::Block)
    {
        CheckVerdict::Blocked
    } else if findings
        .iter()
        .any(|finding| finding.severity == CheckSeverity::Inconclusive)
    {
        CheckVerdict::Inconclusive
    } else {
        CheckVerdict::Pass
    };
    let mut permissions = SkillPermissions {
        reads_files: true,
        ..SkillPermissions::default()
    };
    for finding in &findings {
        match finding.rule.as_str() {
            "skill.permission.process_execution" => permissions.executes_processes = true,
            "skill.permission.network_access" => permissions.uses_network = true,
            "skill.permission.download" => permissions.downloads_content = true,
            "skill.permission.dynamic_eval" => permissions.uses_dynamic_eval = true,
            "skill.permission.credential_access" => permissions.reads_credentials = true,
            "skill.permission.sensitive_path" => permissions.writes_sensitive_paths = true,
            "skill.permission.elevation" => permissions.requires_elevation = true,
            "skill.content.binary" | "skill.content.oversize" => {
                permissions.contains_binary_content = true
            }
            _ => {}
        }
    }
    SkillCheckReport {
        checker_version: SKILL_CHECKER_VERSION.to_string(),
        verdict,
        findings,
        permissions,
        skill_name,
        content_digest,
        file_count,
        total_bytes,
        source_kind: source_kind.to_string(),
        source_ref: source_ref.to_string(),
    }
}

fn manifest_digest(entries: &[crate::capability::types::SkillFileEntry]) -> Option<String> {
    crate::capability::skill::manifest::manifest_digest(entries)
}

fn looks_binary(text: &str) -> bool {
    text.contains('\0')
}

/// Applies the static rule set to one file of text.
fn scan_text(relative: &str, text: &str, findings: &mut Vec<SkillFinding>) {
    let lowered = text.to_ascii_lowercase();

    let rule_sets: &[(&str, CheckSeverity, &[&str])] = &[
        (
            "skill.permission.credential_access",
            CheckSeverity::Block,
            &[
                "id_rsa",
                ".aws/credentials",
                ".git-credentials",
                ".netrc",
                "authorized_keys",
                "/.ssh/",
                "keychain",
            ],
        ),
        (
            "skill.permission.sensitive_path",
            CheckSeverity::Block,
            &[
                "/etc/passwd",
                "/etc/shadow",
                "/etc/sudoers",
                ".bashrc",
                ".zshrc",
                ".bash_profile",
                "c:\\windows",
                "/system32/",
                "launchagents",
            ],
        ),
        (
            "skill.permission.elevation",
            CheckSeverity::Block,
            &["sudo ", "runas /", "doas "],
        ),
        (
            "skill.permission.dynamic_eval",
            CheckSeverity::Warn,
            &["eval(", "exec(", "atob(", "fromcharcode", "invoke-expression", "iex("],
        ),
        (
            "skill.permission.process_execution",
            CheckSeverity::Warn,
            &[
                "subprocess",
                "os.system",
                "child_process",
                "process::command",
                "spawn(",
                "sh -c",
                "bash -c",
                "powershell",
                "cmd.exe",
            ],
        ),
        (
            "skill.permission.network_access",
            CheckSeverity::Warn,
            &[
                "curl ",
                "wget ",
                "requests.get",
                "urllib",
                "http.client",
                "fetch(",
                "axios",
            ],
        ),
        (
            "skill.permission.download",
            CheckSeverity::Warn,
            &["curl -o", "wget -o", "curl -l", "invoke-webrequest", "download("],
        ),
    ];

    for (rule, severity, patterns) in rule_sets {
        if let Some(pattern) = patterns.iter().find(|pattern| lowered.contains(**pattern)) {
            findings.push(SkillFinding {
                rule: (*rule).to_string(),
                severity: *severity,
                path: Some(relative.to_string()),
                detail: format!("the content contains '{pattern}'"),
            });
        }
    }

    if contains_obfuscated_blob(text) {
        findings.push(SkillFinding {
            rule: "skill.content.obfuscated_payload".to_string(),
            severity: CheckSeverity::Block,
            path: Some(relative.to_string()),
            detail: "the content contains a long encoded blob next to an eval/exec call".to_string(),
        });
    }
}

/// An encoded blob of suspicious length next to a decoder or evaluator.
fn contains_obfuscated_blob(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    let has_decoder = lowered.contains("base64")
        || lowered.contains("atob(")
        || lowered.contains("fromcharcode")
        || lowered.contains("\\x");
    if !has_decoder {
        return false;
    }
    let mut run = 0usize;
    for character in text.chars() {
        let encoded = character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=');
        if encoded {
            run += 1;
            if run >= 200 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// Reads the declared Skill name from `SKILL.md` front matter or `skill.json`.
fn read_declared_name(root: &Path) -> Result<Option<String>, String> {
    let skill_md = root.join("SKILL.md");
    if skill_md.is_file() {
        let text = std::fs::read_to_string(&skill_md)
            .map_err(|_| "SKILL.md could not be read as text".to_string())?;
        let front = leading_front_matter(&text)
            .ok_or_else(|| "SKILL.md has no YAML front matter".to_string())?;
        return match scalar_field(front, "name") {
            Some(name) => Ok(Some(name)),
            None => Err("SKILL.md front matter has no 'name'".to_string()),
        };
    }

    for candidate in ["skill.json", "manifest.json"] {
        let path = root.join(candidate);
        if path.is_file() {
            let text = std::fs::read_to_string(&path)
                .map_err(|_| format!("{candidate} could not be read as text"))?;
            let value: serde_json::Value = serde_json::from_str(&text)
                .map_err(|_| format!("{candidate} is not valid JSON"))?;
            let name = value
                .get("name")
                .and_then(|name| name.as_str())
                .ok_or_else(|| format!("{candidate} has no 'name'"))?;
            return Ok(Some(name.to_string()));
        }
    }

    Ok(None)
}

/// The block between the first two `---` lines, when the document starts with
/// one.
fn leading_front_matter(text: &str) -> Option<&str> {
    let trimmed = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = trimmed.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

/// A single-line `key: value` scalar from front matter.
fn scalar_field(front_matter: &str, key: &str) -> Option<String> {
    for line in front_matter.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix(&format!("{key}:")) {
            let value = value.trim().trim_matches('"').trim_matches('\'').trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_skill(root: &Path, name: &str, extra: &[(&str, &str)]) {
        std::fs::create_dir_all(root).expect("create skill dir");
        std::fs::write(
            root.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: demo\n---\n\n# {name}\n"),
        )
        .expect("write SKILL.md");
        for (relative, content) in extra {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent");
            }
            std::fs::write(path, content).expect("write extra file");
        }
    }

    fn finding_rules(report: &SkillCheckReport) -> Vec<String> {
        report
            .findings
            .iter()
            .map(|finding| finding.rule.clone())
            .collect()
    }

    #[test]
    fn prose_skill_passes_cleanly() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("demo");
        write_skill(&root, "demo", &[("references/guide.md", "just prose")]);

        let report = check_directory(&root).expect("check");
        assert_eq!(report.verdict, CheckVerdict::Pass);
        assert!(report.is_pass());
        assert_eq!(report.skill_name.as_deref(), Some("demo"));
        assert_eq!(report.file_count, 2);
        assert!(report.content_digest.is_some());
        assert_eq!(report.checker_version, SKILL_CHECKER_VERSION);
        assert!(finding_rules(&report).is_empty());
    }

    #[test]
    fn a_missing_manifest_blocks() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("demo");
        std::fs::create_dir_all(&root).expect("create dir");
        std::fs::write(root.join("notes.md"), "no manifest").expect("write");

        let report = check_directory(&root).expect("check");
        assert_eq!(report.verdict, CheckVerdict::Blocked);
        assert!(finding_rules(&report).contains(&"skill.manifest.missing".to_string()));
        assert!(report.refusal().is_some());
    }

    #[test]
    fn an_invalid_declared_name_blocks() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("demo");
        write_skill(&root, "Not A Valid Name", &[]);

        let report = check_directory(&root).expect("check");
        assert_eq!(report.verdict, CheckVerdict::Blocked);
        assert!(finding_rules(&report).contains(&"skill.manifest.invalid_name".to_string()));
    }

    #[test]
    fn credential_and_sensitive_path_access_blocks() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("demo");
        write_skill(
            &root,
            "demo",
            &[("scripts/steal.sh", "cat ~/.ssh/id_rsa >> /etc/passwd\nsudo cp x /etc/\n")],
        );

        let report = check_directory(&root).expect("check");
        assert_eq!(report.verdict, CheckVerdict::Blocked);
        let rules = finding_rules(&report);
        assert!(rules.contains(&"skill.permission.credential_access".to_string()));
        assert!(rules.contains(&"skill.permission.sensitive_path".to_string()));
        assert!(rules.contains(&"skill.permission.elevation".to_string()));
        assert!(report.permissions.reads_credentials);
        assert!(report.permissions.writes_sensitive_paths);
        assert!(report.permissions.requires_elevation);
    }

    #[test]
    fn an_obfuscated_payload_blocks() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("demo");
        let blob = "A".repeat(240);
        write_skill(
            &root,
            "demo",
            &[("scripts/run.js", &format!("const x = atob(\"{blob}\"); eval(x);\n"))],
        );

        let report = check_directory(&root).expect("check");
        assert_eq!(report.verdict, CheckVerdict::Blocked);
        assert!(finding_rules(&report).contains(&"skill.content.obfuscated_payload".to_string()));
    }

    #[test]
    fn ordinary_script_usage_is_recorded_but_does_not_block() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("demo");
        write_skill(
            &root,
            "demo",
            &[("scripts/run.sh", "curl https://example.test/data | jq .\n")],
        );

        let report = check_directory(&root).expect("check");
        assert_eq!(report.verdict, CheckVerdict::Pass);
        let rules = finding_rules(&report);
        assert!(rules.contains(&"skill.permission.network_access".to_string()));
        assert!(report.permissions.uses_network);
    }

    #[test]
    fn unknown_binary_content_is_inconclusive() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("demo");
        write_skill(&root, "demo", &[]);
        std::fs::write(root.join("payload.bin"), [0u8, 159, 146, 150]).expect("write binary");

        let report = check_directory(&root).expect("check");
        assert_eq!(report.verdict, CheckVerdict::Inconclusive);
        assert!(finding_rules(&report).contains(&"skill.content.binary".to_string()));
        assert!(report.permissions.contains_binary_content);
        assert!(report.refusal().is_some());
    }

    #[test]
    fn a_symlink_in_the_content_blocks() {
        #[cfg(unix)]
        {
            let temp = TempDir::new().expect("temp dir");
            let root = temp.path().join("demo");
            write_skill(&root, "demo", &[]);
            std::os::unix::fs::symlink("/etc/passwd", root.join("escape")).expect("symlink");

            let report = check_directory(&root).expect("check");
            assert_eq!(report.verdict, CheckVerdict::Blocked);
            assert!(finding_rules(&report).contains(&"skill.content.unsafe_entry".to_string()));
        }
    }

    #[tokio::test]
    async fn an_unreachable_github_source_is_inconclusive() {
        let temp = TempDir::new().expect("temp dir");
        let resolver =
            SkillSourceResolver::with_download_origin(temp.path().join("app-data"), "http://127.0.0.1:1".to_string());
        let source = SkillSource::GitHub {
            owner: "acme".to_string(),
            repo: "skills".to_string(),
            git_ref: None,
            path: None,
        };
        let environment =
            TargetEnvironment::injected(temp.path().join("home"), temp.path().join("chatspeed"));

        let error = resolver
            .check(&source, &environment, "op-skill-download")
            .await
            .err()
            .expect("an unreachable source must not pass");
        assert_eq!(error.code(), crate::capability::error::code::CHECK_INCONCLUSIVE);
        // Staging must not be left behind by a failed check.
        assert!(!crate::capability::staging_dir(&temp.path().join("app-data")).join("op-skill-download").exists());
    }

    #[tokio::test]
    async fn a_local_zip_source_is_staged_checked_and_cleaned() {
        use std::io::Write as _;
        use zip::write::SimpleFileOptions;

        let temp = TempDir::new().expect("temp dir");
        let archive_path = temp.path().join("demo.zip");
        {
            let file = std::fs::File::create(&archive_path).expect("create archive");
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file("demo/SKILL.md", SimpleFileOptions::default())
                .expect("start entry");
            writer
                .write_all(b"---\nname: demo\n---\n\n# demo\n")
                .expect("write entry");
            writer.finish().expect("finish");
        }

        let app_data = temp.path().join("app-data");
        let resolver = SkillSourceResolver::new(app_data.clone());
        let source = SkillSource::LocalZip {
            path: archive_path.to_string_lossy().to_string(),
        };
        let environment =
            TargetEnvironment::injected(temp.path().join("home"), temp.path().join("chatspeed"));

        let report = resolver
            .check(&source, &environment, "op-skill-zip")
            .await
            .expect("check");
        assert_eq!(report.verdict, CheckVerdict::Pass);
        assert_eq!(report.source_kind, "local_zip");
        assert_eq!(report.source_ref, "local_zip:demo.zip");
        assert!(!crate::capability::staging_dir(&app_data).join("op-skill-zip").exists());
    }
}
