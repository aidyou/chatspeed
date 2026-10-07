//! Fail-closed physical preflight for Agent Skill target directories.
//!
//! [`super::targets`] resolves a *lexical* directory from the closed registry.
//! This module turns that into a *physical* decision before any write or delete:
//! a target whose directory, or any existing directory on the way to it, is a
//! symbolic link or a non-directory is refused instead of followed, the root is
//! verified with an exclusive (`create_new`) probe, and the canonical physical
//! root is reported so two registry ids that alias one directory can never both
//! claim it (AC-4/AC-7/INV-3).
//!
//! A refusal never leaves Skill content behind: the preflight runs before the
//! commit copy, so the only thing it can leave is the target directory the
//! install was already allowed to create.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::error::CapabilityError;
use super::targets::{
    registry, resolve_target_or_error, resolve_targets, ResolvedSkillTarget, SkillTargetId,
    TargetEnvironment,
};

/// Stable per-target codes surfaced on a target outcome's `error_code`.
pub mod code {
    /// The target has no verified directory, or its physical directory is
    /// already owned by a different registered target.
    pub const TARGET_NOT_SUPPORTED: &str = "target_not_supported";
    /// The target directory or an existing ancestor is a symbolic link, a
    /// non-directory, or otherwise unusable as a real directory.
    pub const TARGET_PATH_UNAVAILABLE: &str = "target_path_unavailable";
    /// The target directory cannot be created or written by this process.
    pub const TARGET_PERMISSION_DENIED: &str = "target_permission_denied";
}

/// A target whose physical preflight failed. The code is one of [`code`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetPreflightFailure {
    code: &'static str,
    message: String,
}

impl TargetPreflightFailure {
    /// The stable code the failure maps to.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// The non-secret detail, already redacted.
    pub fn detail(&self) -> String {
        self.to_error().redacted_message()
    }

    /// The failure as a capability error carrying the same code.
    pub fn to_error(&self) -> CapabilityError {
        CapabilityError::new(self.code, self.message.clone())
    }

    fn not_supported(message: impl Into<String>) -> Self {
        Self {
            code: code::TARGET_NOT_SUPPORTED,
            message: message.into(),
        }
    }

    fn path_unavailable(message: impl Into<String>) -> Self {
        Self {
            code: code::TARGET_PATH_UNAVAILABLE,
            message: message.into(),
        }
    }

    fn permission_denied(message: impl Into<String>) -> Self {
        Self {
            code: code::TARGET_PERMISSION_DENIED,
            message: message.into(),
        }
    }
}

/// A target root that passed the physical preflight.
#[derive(Debug, Clone)]
pub struct VerifiedTargetRoot {
    pub target_id: SkillTargetId,
    /// The directory a mutation may use, unchanged from the registry.
    pub root: PathBuf,
    /// The canonical physical directory `root` resolves to.
    pub physical_root: PathBuf,
}

/// Verifies a target root for a write.
///
/// The target must be supported, must not alias a higher-priority target, and
/// its existing directory chain must be real directories (never symbolic
/// links). Missing parents are created exactly as the install commit always
/// allowed, but a structural or permission failure is reported rather than
/// guessed, and the root must accept an exclusive probe before anything is
/// committed.
pub fn verify_install_root(
    target_id: &str,
    environment: &TargetEnvironment,
) -> Result<VerifiedTargetRoot, TargetPreflightFailure> {
    let (id, root) = resolve_mutation_target(target_id, environment)?;
    reject_symlinked_chain(id, &root, environment)?;

    // Missing parents are created as the commit path always allowed. A failure
    // here is structural or a permission problem, never a silent skip.
    std::fs::create_dir_all(&root).map_err(|error| {
        let message = format!("failed to create the target directory: {error}");
        match error.kind() {
            ErrorKind::PermissionDenied => TargetPreflightFailure::permission_denied(message),
            _ => TargetPreflightFailure::path_unavailable(message),
        }
    })?;

    // Re-inspect after creation: the root must now be a real directory and not
    // a link that a racing writer could have introduced.
    reject_root_type(&root)?;
    probe_writable(&root)?;

    Ok(VerifiedTargetRoot {
        target_id: id,
        physical_root: canonical_physical(&root),
        root,
    })
}

/// Verifies an eligible target root before a delete.
///
/// Missing directories are allowed here — there is simply nothing to delete —
/// but an existing directory chain that contains a symbolic link is refused, so
/// an eligible id can never be used to delete through a link that points at an
/// external software root (AC-7/INV-6). A target that only aliases a
/// higher-priority target's physical directory is refused exactly as on install,
/// so a legacy `chatspeed` ownership record cannot delete the canonical Agents
/// directory when `CHATSPEED_HOME` points at it (AC-7/INV-2).
pub fn verify_delete_root(
    target_id: &str,
    environment: &TargetEnvironment,
) -> Result<VerifiedTargetRoot, TargetPreflightFailure> {
    let (id, root) = resolve_mutation_target(target_id, environment)?;
    reject_symlinked_chain(id, &root, environment)?;
    Ok(VerifiedTargetRoot {
        target_id: id,
        physical_root: canonical_physical(&root),
        root,
    })
}

/// Whether `target_id` is the canonical owner of the physical directory it
/// resolves to.
///
/// A registry id that only aliases a higher-priority target's directory — for
/// example `chatspeed` when `CHATSPEED_HOME` points at the shared Agents home —
/// does not own what it would touch, so it must never be used to write to,
/// delete from, or reconcile that directory (AC-7/INV-2).
pub fn is_canonical_owner(target_id: &str, environment: &TargetEnvironment) -> bool {
    resolve_mutation_target(target_id, environment).is_ok()
}

/// Resolves one target and refuses it when its physical directory is already
/// claimed by a higher-priority registered target.
fn resolve_mutation_target(
    target_id: &str,
    environment: &TargetEnvironment,
) -> Result<(SkillTargetId, PathBuf), TargetPreflightFailure> {
    let (id, root) = resolve_supported_target(target_id, environment)?;
    if let Some(owner) = higher_priority_owner(id, environment) {
        return Err(TargetPreflightFailure::not_supported(format!(
            "target '{target_id}' resolves to the same physical directory as '{owner}', which owns it"
        )));
    }
    Ok((id, root))
}

/// Resolves one supported target to its lexical root, failing closed otherwise.
fn resolve_supported_target(
    target_id: &str,
    environment: &TargetEnvironment,
) -> Result<(SkillTargetId, PathBuf), TargetPreflightFailure> {
    resolve_target_or_error(target_id, environment)
        .map_err(|error| TargetPreflightFailure::not_supported(error.redacted_message()))
}

/// The registered target that canonically owns the physical directory `root`.
///
/// Several registry ids can resolve to one physical directory — for example
/// `chatspeed` when `CHATSPEED_HOME` points at the shared Agents home — so a
/// lexical match alone cannot decide who owns a directory. The canonical Agents
/// directory is the preferred owner of its directory; every other id yields to
/// it, then to the earlier registry entry. The install path, the delete path and
/// the inventory all resolve ownership with this one rule, so a scanned root is
/// never attributed to — and never deleted through — an id that only aliases it
/// (AC-4/AC-7/INV-2/INV-3).
pub fn canonical_owner_for_root(targets: &[ResolvedSkillTarget], root: &Path) -> Option<String> {
    let root_physical = canonical_physical(root);
    targets
        .iter()
        .filter_map(|entry| {
            let path = entry.path.as_deref()?;
            let id = SkillTargetId::parse(&entry.id)?;
            (canonical_physical(Path::new(path)) == root_physical)
                .then(|| (priority(id), entry.id.clone()))
        })
        .min_by(|left, right| left.0.cmp(&right.0))
        .map(|(_, id)| id)
}

/// The registered target that physically owns this id's directory, when a
/// higher-priority target resolves to the same physical path.
///
/// The shared canonical Agents directory is the preferred owner of its
/// directory, so a different id that aliases it is refused rather than creating
/// a second ownership of one directory.
fn higher_priority_owner(id: SkillTargetId, environment: &TargetEnvironment) -> Option<String> {
    let resolved = resolve_targets(environment);
    let target = resolved.iter().find(|entry| entry.id == id.as_str())?;
    let target_path = target.path.as_deref()?;
    let owner = canonical_owner_for_root(&resolved, Path::new(target_path))?;
    (owner != id.as_str()).then_some(owner)
}

/// Registry precedence for a physical collision: a smaller tuple wins. The
/// canonical Agents directory is the preferred owner of its directory; every
/// other id yields to it, then to the earlier registry entry.
fn priority(id: SkillTargetId) -> (u8, usize) {
    if id == SkillTargetId::Agents {
        return (0, 0);
    }
    let index = registry()
        .iter()
        .position(|spec| spec.id == id)
        .unwrap_or(usize::MAX);
    (1, index)
}

/// Refuses a target whose existing directory chain is not a chain of real
/// directories. The user's `HOME` is the trusted anchor and is never rejected
/// for being a link itself; every directory below it is checked.
fn reject_symlinked_chain(
    id: SkillTargetId,
    root: &Path,
    environment: &TargetEnvironment,
) -> Result<(), TargetPreflightFailure> {
    let home = environment.home_dir.clone();
    let base = if id.is_chatspeed() {
        environment.chatspeed_home.clone()
    } else {
        home.clone()
    };
    let Some(base) = base else {
        return Ok(());
    };

    let mut current = base.clone();
    let mut chain = Vec::new();
    if home.as_deref() != Some(base.as_path()) {
        chain.push(current.clone());
    }
    for component in root
        .strip_prefix(&base)
        .unwrap_or(Path::new(""))
        .components()
    {
        current.push(component);
        chain.push(current.clone());
    }

    for path in chain {
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(TargetPreflightFailure::path_unavailable(format!(
                        "the target directory chain contains a symbolic link at '{}'",
                        path.display()
                    )));
                }
                if !metadata.is_dir() {
                    return Err(TargetPreflightFailure::path_unavailable(format!(
                        "the target directory chain is not a directory at '{}'",
                        path.display()
                    )));
                }
            }
            // The rest of the chain does not exist yet and may be created.
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(TargetPreflightFailure::path_unavailable(format!(
                    "failed to inspect the target directory '{}': {error}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

/// Refuses a root that exists as something other than a real directory.
fn reject_root_type(root: &Path) -> Result<(), TargetPreflightFailure> {
    let metadata = std::fs::symlink_metadata(root).map_err(|error| {
        TargetPreflightFailure::path_unavailable(format!(
            "failed to inspect the target directory '{}': {error}",
            root.display()
        ))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(TargetPreflightFailure::path_unavailable(format!(
            "the target directory '{}' is a symbolic link",
            root.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(TargetPreflightFailure::path_unavailable(format!(
            "the target directory '{}' exists and is not a directory",
            root.display()
        )));
    }
    Ok(())
}

/// Writes and removes a uniquely named probe inside the root.
///
/// The probe uses `create_new`, so it can never follow a pre-existing name, and
/// it is removed immediately: an unwritable root fails here instead of halfway
/// through a copy. Nothing but the probe itself is ever written.
fn probe_writable(root: &Path) -> Result<(), TargetPreflightFailure> {
    let probe = root.join(format!(
        ".chatspeed-preflight-{}",
        uuid::Uuid::now_v7().simple()
    ));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::remove_file(&probe);
            let message = format!("the target directory is not writable: {error}");
            match error.kind() {
                ErrorKind::PermissionDenied => {
                    Err(TargetPreflightFailure::permission_denied(message))
                }
                _ => Err(TargetPreflightFailure::path_unavailable(message)),
            }
        }
    }
}

/// Canonicalizes the deepest existing ancestor of `path` and re-appends the
/// missing tail, so two lexically different paths that will land on one
/// directory compare equal even before that directory exists.
fn canonical_physical(path: &Path) -> PathBuf {
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        match std::fs::symlink_metadata(&current) {
            Ok(_) => {
                let mut resolved =
                    std::fs::canonicalize(&current).unwrap_or_else(|_| current.clone());
                for part in missing.iter().rev() {
                    resolved.push(part);
                }
                return resolved;
            }
            Err(_) => match current.file_name() {
                Some(name) => missing.push(name.to_os_string()),
                None => return path.to_path_buf(),
            },
        }
        if !current.pop() {
            return path.to_path_buf();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn environment(home: &Path, chatspeed: PathBuf) -> TargetEnvironment {
        TargetEnvironment::injected(home.to_path_buf(), chatspeed)
    }

    #[test]
    fn a_normal_missing_target_root_is_created_and_probed() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).expect("home");
        let environment = environment(&home, temp.path().join("chatspeed"));

        let verified = verify_install_root("chatspeed", &environment).expect("chatspeed preflight");
        assert_eq!(verified.target_id, SkillTargetId::Chatspeed);
        assert!(verified.root.is_dir());
        assert!(verified.physical_root.is_dir());
        // The probe never remains behind.
        let leftovers: Vec<_> = std::fs::read_dir(&verified.root)
            .expect("read root")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains("preflight"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn a_symlinked_target_root_is_refused() {
        #[cfg(unix)]
        {
            let temp = TempDir::new().expect("temp dir");
            let home = temp.path().join("home");
            let chatspeed = temp.path().join("chatspeed");
            let outside = temp.path().join("outside");
            std::fs::create_dir_all(&chatspeed).expect("chatspeed home");
            std::fs::create_dir_all(&outside).expect("outside");
            std::os::unix::fs::symlink(&outside, chatspeed.join("skills")).expect("symlink root");
            let environment = environment(&home, chatspeed);

            let failure =
                verify_install_root("chatspeed", &environment).expect_err("symlink root refused");
            assert_eq!(failure.code(), code::TARGET_PATH_UNAVAILABLE);
        }
    }

    #[test]
    fn a_symlinked_target_parent_is_refused() {
        #[cfg(unix)]
        {
            let temp = TempDir::new().expect("temp dir");
            let home = temp.path().join("home");
            let outside = temp.path().join("outside");
            std::fs::create_dir_all(&home).expect("home");
            std::fs::create_dir_all(&outside).expect("outside");
            std::os::unix::fs::symlink(&outside, home.join(".agents")).expect("symlink parent");
            let environment = environment(&home, temp.path().join("chatspeed"));

            let failure =
                verify_install_root("agents", &environment).expect_err("symlink parent refused");
            assert_eq!(failure.code(), code::TARGET_PATH_UNAVAILABLE);
        }
    }

    #[test]
    fn a_non_directory_target_root_is_refused() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().join("home");
        let chatspeed = temp.path().join("chatspeed");
        std::fs::create_dir_all(&chatspeed).expect("chatspeed home");
        std::fs::write(chatspeed.join("skills"), "not a directory").expect("file root");
        let environment = environment(&home, chatspeed);

        let failure =
            verify_install_root("chatspeed", &environment).expect_err("file root refused");
        assert_eq!(failure.code(), code::TARGET_PATH_UNAVAILABLE);
    }

    #[test]
    fn a_write_protected_target_root_is_permission_denied() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let temp = TempDir::new().expect("temp dir");
            let home = temp.path().join("home");
            let chatspeed = temp.path().join("chatspeed");
            let skills = chatspeed.join("skills");
            std::fs::create_dir_all(&skills).expect("skills dir");
            std::fs::set_permissions(&skills, std::fs::Permissions::from_mode(0o555))
                .expect("read-only");
            let environment = environment(&home, chatspeed);

            let failure =
                verify_install_root("chatspeed", &environment).expect_err("read-only refused");
            assert_eq!(failure.code(), code::TARGET_PERMISSION_DENIED);

            // Restore write permission so the temp directory can be cleaned up.
            let _ = std::fs::set_permissions(&skills, std::fs::Permissions::from_mode(0o755));
        }
    }

    #[test]
    fn a_physical_alias_is_refused_for_the_lower_priority_target() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).expect("home");
        // CHATSPEED_HOME overlaps the shared Agents home, so the two ids resolve
        // to one physical directory.
        let environment = environment(&home, home.join(".agents"));

        let failure = verify_install_root("chatspeed", &environment)
            .expect_err("aliasing chatspeed is refused");
        assert_eq!(failure.code(), code::TARGET_NOT_SUPPORTED);
        // The preferred canonical Agents target remains available.
        verify_install_root("agents", &environment).expect("agents stays available");
    }

    #[test]
    fn a_physical_alias_is_refused_for_delete_on_the_lower_priority_target() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).expect("home");
        // CHATSPEED_HOME overlaps the shared Agents home, so the two ids resolve
        // to one physical directory: a delete through the alias would delete the
        // canonical Agents directory.
        let environment = environment(&home, home.join(".agents"));

        let failure = verify_delete_root("chatspeed", &environment)
            .expect_err("an aliasing chatspeed delete is refused");
        assert_eq!(failure.code(), code::TARGET_NOT_SUPPORTED);
        // The canonical Agents owner may still delete from its own directory.
        verify_delete_root("agents", &environment).expect("agents stays available for delete");
    }

    #[test]
    fn an_unverified_target_is_not_supported() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).expect("home");
        let environment = environment(&home, temp.path().join("chatspeed"));

        let failure = verify_install_root("qoder", &environment).expect_err("unverified refused");
        assert_eq!(failure.code(), code::TARGET_NOT_SUPPORTED);
    }

    #[test]
    fn a_delete_preflight_accepts_a_missing_root_but_refuses_a_symlink() {
        #[cfg(unix)]
        {
            let temp = TempDir::new().expect("temp dir");
            let home = temp.path().join("home");
            let chatspeed = temp.path().join("chatspeed");
            std::fs::create_dir_all(&home).expect("home");
            let environment = environment(&home, chatspeed.clone());

            // A missing root has nothing to delete and is not refused.
            verify_delete_root("chatspeed", &environment).expect("missing root is fine");

            let outside = temp.path().join("outside");
            std::fs::create_dir_all(&chatspeed).expect("chatspeed home");
            std::fs::create_dir_all(&outside).expect("outside");
            std::os::unix::fs::symlink(&outside, chatspeed.join("skills")).expect("symlink root");
            let failure =
                verify_delete_root("chatspeed", &environment).expect_err("symlink refused");
            assert_eq!(failure.code(), code::TARGET_PATH_UNAVAILABLE);
        }
    }
}
