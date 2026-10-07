//! The runtime-owned `agent-skills` plugin bundle lifecycle service.
//!
//! The standalone runtime is the only plugin-management owner: it resolves the
//! plugin root, materializes the static bundle, tracks its lifecycle state and
//! answers the `/control/v1/plugins/agent-skills` routes. The desktop never
//! reaches the filesystem for plugin management; its Tauri commands forward to
//! this service over the control plane.
//!
//! This module carries the *minimum provable boundary* of the Agent Skills
//! plugin package: a fixed manifest, a private staging area, an atomic publish
//! that keeps the previous version on any failure, and a lifecycle that removes
//! only the plugin-owned bundle. It deliberately does not execute plugin code
//! and does not build a webview host.
//!
//! The stable error codes, manifest descriptor and wire DTOs live in
//! [`crate::plugin_types`], which both crates compile; this executable
//! lifecycle is gated behind the backend's `plugin-service` feature. The desktop
//! links the backend with `default-features = false` for its runtime dependency,
//! so the desktop binary never compiles the plugin filesystem implementation it
//! reaches only over the control plane. A workspace-wide build still compiles it,
//! because the backend's default feature set enables `plugin-service`.
//!
//! # Isolation contract
//!
//! - The bundle is a *static* set of files compiled into the binary
//!   (`include_str!`) and materialized on disk byte-for-byte. Nothing in it is
//!   ever executed: no entry point is loaded into a webview and no plugin
//!   process is spawned (`plugin_code_executed: false`).
//! - The bundle lives under `${CHATSPEED_HOME:-~/.chatspeed}/plugins/agent-skills`
//!   and the service refuses to touch any path outside that bundle. Uninstall
//!   never removes `${CHATSPEED_HOME:-~/.chatspeed}/skills` and never removes any
//!   installed target directory.
//! - Every managed path is re-proved before a mutation and the service fails
//!   closed: a symbolic link anywhere from the target up to the OS home anchor is
//!   refused (never followed), including a symlinked `${CHATSPEED_HOME}` or one
//!   of its parents, and a `${CHATSPEED_HOME}` that resolves into a registered
//!   Agent Skill target directory is refused. A plain directory is only ever
//!   removed or replaced after it proves it is *exactly* the embedded bundle
//!   (manifest, assets and host state as real regular files, with no extra
//!   entry); residue is removed under the same proof. Foreign or drifted content
//!   is left in place.
//! - All lifecycle calls are serialized so a concurrent load and uninstall
//!   cannot interleave their staging and cleanup.
//! - The only surface is the fixed set of typed service methods the control
//!   plane exposes. No request takes a path, a shell string or a generic
//!   command/RPC envelope, so a client can never reach an arbitrary filesystem,
//!   database or token API through this service.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::capability::targets::TargetEnvironment;
use crate::plugin_types::*;

/// The manifest embedded at compile time. It is the single source of the
/// bundle's identity; staging re-verifies it before publishing.
const EMBEDDED_MANIFEST: &str = include_str!("../../../assets/agent-skills-plugin/plugin.json");
/// The entry document embedded at compile time. It is never executed.
const EMBEDDED_ENTRY: &str = include_str!("../../../assets/agent-skills-plugin/index.html");

/// One static asset of the bundle, embedded into the binary.
struct EmbeddedAsset {
    path: &'static str,
    bytes: &'static [u8],
}

/// The complete embedded bundle content. Staging writes exactly these bytes so
/// the on-disk bundle can never drift from what this service verified.
const EMBEDDED_ASSETS: &[EmbeddedAsset] = &[EmbeddedAsset {
    path: "index.html",
    bytes: EMBEDDED_ENTRY.as_bytes(),
}];

/// The absolute paths the service resolves against one `${CHATSPEED_HOME}`.
///
/// Every field is explicit so a test (or a misconfiguration) can inject an
/// adversarial layout and the confinement checks still run against it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPaths {
    pub chatspeed_home: PathBuf,
    pub plugins_root: PathBuf,
    pub plugin_dir: PathBuf,
    pub staging_root: PathBuf,
    /// The managed skills directory. It is a denial sentinel: the service must
    /// never read or remove it.
    pub skills_dir: PathBuf,
    pub state_file: PathBuf,
}

impl PluginPaths {
    /// Resolves `${CHATSPEED_HOME:-~/.chatspeed}/plugins/agent-skills`.
    ///
    /// The home convention is the same `${CHATSPEED_HOME}`-or-`~/.chatspeed`
    /// rule the capability targets use, resolved by the runtime so every client
    /// observes one plugin root.
    pub fn detect() -> Result<Self, PluginError> {
        let chatspeed_home = match std::env::var_os("CHATSPEED_HOME") {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => dirs::home_dir()
                .map(|home| home.join(".chatspeed"))
                .ok_or_else(|| {
                    PluginError::unavailable(
                        "neither CHATSPEED_HOME nor the user home directory is available",
                    )
                })?,
        };
        Ok(Self::under(chatspeed_home))
    }

    /// Builds the layout under an explicit ChatSpeed home.
    pub fn under(chatspeed_home: PathBuf) -> Self {
        let plugins_root = chatspeed_home.join("plugins");
        let plugin_dir = plugins_root.join(PLUGIN_ID);
        Self {
            staging_root: plugins_root.join(".staging"),
            skills_dir: chatspeed_home.join("skills"),
            state_file: plugin_dir.join(STATE_FILE_NAME),
            plugin_dir,
            plugins_root,
            chatspeed_home,
        }
    }

    /// Refuses a layout where the plugin bundle could overlap the ChatSpeed home
    /// or the managed skills directory. It runs before any mutation.
    fn ensure_confined(&self) -> Result<(), PluginError> {
        if !self.plugin_dir.starts_with(&self.chatspeed_home)
            || self.plugin_dir == self.chatspeed_home
        {
            return Err(PluginError::refused(
                "the plugin directory is not inside the ChatSpeed home",
            ));
        }
        if !self.plugin_dir.starts_with(&self.plugins_root) || self.plugin_dir == self.plugins_root
        {
            return Err(PluginError::refused(
                "the plugin directory is not a child of the plugins root",
            ));
        }
        if self.plugin_dir == self.skills_dir || self.plugin_dir.starts_with(&self.skills_dir) {
            return Err(PluginError::refused(
                "the plugin directory overlaps the managed skills directory",
            ));
        }
        if self.skills_dir.starts_with(&self.plugin_dir) {
            return Err(PluginError::refused(
                "the plugin directory would contain the managed skills directory",
            ));
        }
        Ok(())
    }

    /// The single OS-owned directory the plugin never re-proves: the user home,
    /// and only when `${CHATSPEED_HOME}` lives strictly inside it. It mirrors the
    /// target preflight's `HOME` anchor. `${CHATSPEED_HOME}` is an
    /// environment-controlled path, so it is *not* trusted: the anchor itself
    /// (and every parent below the OS home) is re-proved as a real directory.
    fn trust_anchor(&self) -> Option<PathBuf> {
        dirs::home_dir().filter(|home| self.chatspeed_home.starts_with(home))
    }

    /// Fails closed on a symlinked parent chain.
    ///
    /// The lexical [`ensure_confined`] check cannot see a symlink: a `plugins/`,
    /// `plugins/.staging` or terminal bundle link would silently redirect a load,
    /// disable or uninstall into whatever it points at (for example the managed
    /// skills directory). This walks every existing component from `target` up to
    /// [`Self::trust_anchor`] and refuses any symlink, so a symlinked
    /// `${CHATSPEED_HOME}` anchor or any of its parents is refused instead of
    /// followed. When `include_target` is true the target itself must also be a
    /// real directory; uninstall passes false so it can still remove a terminal
    /// bundle symlink as a link, without ever following it.
    fn ensure_real_path(&self, target: &Path, include_target: bool) -> Result<(), PluginError> {
        if !target.starts_with(&self.chatspeed_home) {
            return Err(PluginError::refused(
                "a plugin management path escapes the ChatSpeed home",
            ));
        }
        let anchor = self.trust_anchor();
        let start = if include_target {
            Some(target)
        } else {
            target.parent()
        };
        let mut cursor = start;
        while let Some(dir) = cursor {
            if anchor.as_deref() == Some(dir) {
                // The OS home is the single trusted physical anchor.
                break;
            }
            if is_symlink(dir) {
                return Err(PluginError::refused(format!(
                    "refusing a plugin management path through a symlinked directory: {}",
                    dir.display()
                )));
            }
            cursor = dir.parent();
        }
        Ok(())
    }

    /// Refuses a layout whose plugin area overlaps a registered Agent Skill
    /// target directory.
    ///
    /// The registry is the closed, authoritative set of external skill roots, so
    /// this denies exact fixed paths instead of guessing a directory name: a
    /// `${CHATSPEED_HOME}` that resolves into a target (for example
    /// `~/.agents/skills`) is refused before any write, so the bundle can never
    /// be published into a Skill target.
    fn ensure_clear_of_skill_targets(
        &self,
        environment: &TargetEnvironment,
    ) -> Result<(), PluginError> {
        for target in crate::capability::targets::resolve_targets(environment) {
            if !target.external {
                continue;
            }
            let Some(path) = target.path.as_deref() else {
                continue;
            };
            if paths_overlap(&self.plugins_root, Path::new(path)) {
                return Err(PluginError::refused(format!(
                    "the plugin directory would overlap the registered '{}' skill target",
                    target.id
                )));
            }
        }
        Ok(())
    }

    /// Whether the plugin root and bundle can be read without following a
    /// symlink or landing in a registered Skill target. A redirected read must
    /// observe "not installed" instead of the link target.
    fn is_readable(&self) -> bool {
        let environment = TargetEnvironment::detect();
        self.ensure_real_path(&self.plugins_root, true).is_ok()
            && self.ensure_real_path(&self.plugin_dir, true).is_ok()
            && self
                .ensure_clear_of_skill_targets(&environment)
                .is_ok()
    }
}

/// Whether two paths are the same directory or one contains the other.
fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

/// Whether `path` itself is a symlink. Uses `symlink_metadata`, so the link is
/// inspected, never followed.
fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

/// The service-owned lifecycle state, stored next to the bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostState {
    #[serde(default = "default_enabled")]
    enabled: bool,
    version: String,
}

fn default_enabled() -> bool {
    true
}

/// The runtime-owned plugin service. It owns only the resolved paths; the set
/// of assets is the compile-time embedded bundle, so a caller cannot inject
/// content. Every call is serialized through one mutex so a concurrent load and
/// uninstall cannot interleave their staging, publication and cleanup.
pub struct PluginService {
    paths: Option<PluginPaths>,
    lock: Mutex<()>,
}

impl PluginService {
    /// Builds the service for the real environment. A missing ChatSpeed home is
    /// reported through every call result instead of failing startup.
    pub fn detect() -> Self {
        match PluginPaths::detect() {
            Ok(paths) => Self {
                paths: Some(paths),
                lock: Mutex::new(()),
            },
            Err(error) => {
                log::warn!("agent-skills plugin service is unavailable: {error}");
                Self {
                    paths: None,
                    lock: Mutex::new(()),
                }
            }
        }
    }

    /// Builds the service for an explicit layout (tests and hosted runs).
    pub fn with_paths(paths: PluginPaths) -> Self {
        Self {
            paths: Some(paths),
            lock: Mutex::new(()),
        }
    }

    fn resolved_paths(&self) -> Result<&PluginPaths, PluginError> {
        self.paths.as_ref().ok_or_else(|| {
            PluginError::unavailable(
                "the agent-skills plugin service has no resolved CHATSPEED_HOME",
            )
        })
    }

    /// Takes the lifecycle lock, mapping a poisoned mutex to a structured error
    /// instead of panicking the whole service.
    fn lock(&self) -> Result<MutexGuard<'_, ()>, PluginError> {
        self.lock
            .lock()
            .map_err(|_| PluginError::internal("the plugin service lock is poisoned"))
    }

    /// Reports the current bundle state without mutating anything.
    pub fn inventory(&self) -> Result<PluginInventory, PluginError> {
        let _guard = self.lock()?;
        self.inventory_unlocked()
    }

    fn inventory_unlocked(&self) -> Result<PluginInventory, PluginError> {
        let paths = self.resolved_paths()?;
        let manifest = embedded_manifest()?;

        let mut inventory = PluginInventory {
            plugin_id: manifest.id.clone(),
            schema: manifest.schema.clone(),
            installed: false,
            enabled: false,
            version: None,
            entry: None,
            assets: Vec::new(),
            permissions: Vec::new(),
            root: paths.plugin_dir.to_string_lossy().to_string(),
            bundle_digest: None,
            uninstall_scope: UNINSTALL_SCOPE,
            managed_skills_dir: paths.skills_dir.to_string_lossy().to_string(),
            host: host_isolation(),
        };

        // A symlinked plugin root, bundle or registered Skill-target overlap is
        // never followed: the read reports "not installed" instead of reading
        // the link target, and only a complete evidence bundle is installed.
        if paths.is_readable() {
            if let Some((disk, state)) = inspect_installed(paths) {
                inventory.installed = true;
                inventory.enabled = state.enabled;
                inventory.version = Some(disk.version.clone());
                inventory.entry = Some(disk.entry.clone());
                inventory.assets = disk.assets.clone();
                inventory.permissions = disk.permissions.clone();
                inventory.bundle_digest =
                    installed_bundle_digest(&paths.plugin_dir, &disk.assets);
            }
        }
        Ok(inventory)
    }

    /// Stages, verifies and atomically publishes the embedded bundle.
    pub fn load(&self) -> Result<PluginInventory, PluginError> {
        let _guard = self.lock()?;
        let paths = self.resolved_paths()?;
        paths.ensure_confined()?;
        paths.ensure_clear_of_skill_targets(&TargetEnvironment::detect())?;
        // Create nothing through a symlinked root or staging area.
        paths.ensure_real_path(&paths.plugins_root, true)?;
        paths.ensure_real_path(&paths.staging_root, true)?;
        // Never publish through a terminal bundle symlink, and never replace a
        // directory that does not prove it is the embedded bundle.
        paths.ensure_real_path(&paths.plugin_dir, true)?;
        assert_owned_bundle(paths)?;

        let manifest = embedded_manifest()?;

        std::fs::create_dir_all(&paths.plugins_root).map_err(|error| {
            PluginError::io(format!("failed to create the plugins root: {error}"))
        })?;
        restrict_permissions(&paths.plugins_root)?;
        std::fs::create_dir_all(&paths.staging_root).map_err(|error| {
            PluginError::io(format!("failed to create the staging root: {error}"))
        })?;
        restrict_permissions(&paths.staging_root)?;

        let staged = paths.staging_root.join(format!("{PLUGIN_ID}-{}", nonce()));
        if staged.exists() {
            return Err(PluginError::conflict(format!(
                "a staging directory for '{PLUGIN_ID}' already exists"
            )));
        }
        std::fs::create_dir_all(&staged).map_err(|error| {
            PluginError::io(format!("failed to create a staging directory: {error}"))
        })?;
        restrict_permissions(&staged)?;

        if let Err(error) = stage_into(&staged, &manifest) {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(error);
        }

        let backup = paths
            .plugins_root
            .join(format!(".{PLUGIN_ID}-backup-{}", nonce()));
        if let Err(error) = replace_dir(&paths.plugin_dir, &staged, &backup) {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(error);
        }

        self.inventory_unlocked()
    }

    /// Marks the installed bundle disabled. Only the service state file changes;
    /// the bundle assets are left untouched.
    pub fn disable(&self) -> Result<PluginInventory, PluginError> {
        let _guard = self.lock()?;
        let paths = self.resolved_paths()?;
        paths.ensure_confined()?;
        paths.ensure_clear_of_skill_targets(&TargetEnvironment::detect())?;
        // A symlinked bundle would redirect the state write into its target, so
        // it must be a real directory (or the call fails closed).
        paths.ensure_real_path(&paths.plugin_dir, true)?;
        // Prove ownership before touching the state file: a foreign or drifted
        // directory is refused instead of being marked disabled.
        assert_owned_bundle(paths)?;
        let (manifest, _state) = inspect_installed(paths).ok_or_else(|| {
            PluginError::not_installed("the agent-skills plugin bundle is not installed")
        })?;
        write_state_file(
            &paths.state_file,
            &HostState {
                enabled: false,
                version: manifest.version,
            },
        )?;
        self.inventory_unlocked()
    }

    /// Removes only the plugin-owned bundle and its own staging residue. It
    /// never removes the managed skills directory or any installed target, and
    /// it refuses a foreign or drifted directory instead of deleting it.
    pub fn uninstall(&self) -> Result<PluginInventory, PluginError> {
        let _guard = self.lock()?;
        let paths = self.resolved_paths()?;
        paths.ensure_confined()?;
        paths.ensure_clear_of_skill_targets(&TargetEnvironment::detect())?;
        // The residue cleanup scans these roots, so they must not be symlinks.
        paths.ensure_real_path(&paths.plugins_root, true)?;
        paths.ensure_real_path(&paths.staging_root, true)?;
        // A terminal bundle symlink may be removed as a link (parents real), so
        // the target is never followed or deleted.
        paths.ensure_real_path(&paths.plugin_dir, false)?;
        remove_bundle(paths)?;
        cleanup_residue(paths)?;
        self.inventory_unlocked()
    }
}

/// Parses and validates the compile-time manifest.
fn embedded_manifest() -> Result<PluginManifest, PluginError> {
    PluginManifest::parse(EMBEDDED_MANIFEST)
}

/// Writes the embedded manifest, assets and lifecycle state into a fresh
/// staging directory, then verifies every byte that was written.
fn stage_into(staged: &Path, manifest: &PluginManifest) -> Result<(), PluginError> {
    let canonical = serde_json::to_string_pretty(manifest).map_err(|error| {
        PluginError::internal(format!("failed to serialize the plugin manifest: {error}"))
    })?;
    write_new_file(&staged.join(MANIFEST_FILE_NAME), canonical.as_bytes()).map_err(|error| {
        PluginError::io(format!("failed to write the staged manifest: {error}"))
    })?;

    for asset in EMBEDDED_ASSETS {
        write_asset(staged, asset.path, asset.bytes)?;
    }

    write_state_file(
        &staged.join(STATE_FILE_NAME),
        &HostState {
            enabled: true,
            version: manifest.version.clone(),
        },
    )?;

    verify_bundle(staged, manifest)
}

/// Writes `bytes` to a brand-new file with `create_new`.
///
/// The exclusive create is the write-side half of the symlink proof: a
/// pre-existing name — in particular a symbolic link a racing writer planted —
/// makes the open fail instead of following the link and writing through it.
fn write_new_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.flush()
}

/// The exact set of names the embedded bundle may contain: the manifest, the
/// declared embedded assets and the service host state, and nothing else.
fn embedded_bundle_names() -> BTreeSet<OsString> {
    let mut names = BTreeSet::new();
    names.insert(OsString::from(MANIFEST_FILE_NAME));
    names.insert(OsString::from(STATE_FILE_NAME));
    for asset in EMBEDDED_ASSETS {
        names.insert(OsString::from(asset.path));
    }
    names
}

/// Refuses a path that is not an existing real regular file.
///
/// `symlink_metadata` inspects the entry itself, so a symbolic link is refused
/// rather than followed, for both the ownership proof and the later read.
fn require_real_regular_file(path: &Path, what: &str) -> Result<(), PluginError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(PluginError::refused(format!(
            "the plugin bundle {what} is a symbolic link"
        ))),
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(PluginError::refused(format!(
            "the plugin bundle {what} is not a regular file"
        ))),
        Err(error) if error.kind() == ErrorKind::NotFound => Err(PluginError::refused(format!(
            "the plugin bundle {what} is missing"
        ))),
        Err(error) => Err(PluginError::io(format!(
            "failed to inspect '{}': {error}",
            path.display()
        ))),
    }
}

/// Verifies a bundle tree is *exactly* the embedded bundle.
///
/// The directory must be a real directory (not a symlink), contain the exact set
/// of names (no stray file), and hold the manifest, every declared asset and the
/// host state as real regular files whose bytes match the embedded bundle. A
/// missing or unexpected entry is a refusal, not an IO error, so a foreign or
/// drifted directory is never mistaken for the owned bundle.
fn verify_bundle(root: &Path, manifest: &PluginManifest) -> Result<(), PluginError> {
    match std::fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(PluginError::refused(
                "the plugin bundle directory is a symbolic link",
            ))
        }
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(PluginError::refused(
                "the plugin bundle path is not a directory",
            ))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(PluginError::refused(
                "the plugin bundle directory is missing",
            ))
        }
        Err(error) => {
            return Err(PluginError::io(format!(
                "failed to inspect '{}': {error}",
                root.display()
            )))
        }
    }

    // The directory must contain exactly the embedded names: an extra entry is
    // unknown content the service refuses to replace or delete.
    let expected = embedded_bundle_names();
    let mut seen: BTreeSet<OsString> = BTreeSet::new();
    let entries = std::fs::read_dir(root)
        .map_err(|error| PluginError::io(format!("failed to read the plugin bundle: {error}")))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            PluginError::io(format!("failed to read the plugin bundle: {error}"))
        })?;
        let name = entry.file_name();
        if !expected.contains(&name) {
            return Err(PluginError::refused(format!(
                "the plugin bundle contains an unexpected entry '{}'",
                name.to_string_lossy()
            )));
        }
        seen.insert(name);
    }
    if seen.len() != expected.len() {
        return Err(PluginError::refused(
            "the plugin bundle is missing a required file",
        ));
    }

    require_real_regular_file(&root.join(MANIFEST_FILE_NAME), "manifest")?;
    let on_disk = read_manifest(root)?;
    if on_disk != *manifest {
        return Err(PluginError::refused(
            "the on-disk manifest does not match the embedded manifest",
        ));
    }
    for asset in EMBEDDED_ASSETS {
        let path = root.join(asset.path);
        require_real_regular_file(&path, "asset")?;
        let bytes = std::fs::read(&path).map_err(|error| {
            PluginError::io(format!("failed to read asset '{}': {error}", asset.path))
        })?;
        if sha256_hex(&bytes) != sha256_hex(asset.bytes) {
            return Err(PluginError::refused(format!(
                "asset '{}' failed its integrity check",
                asset.path
            )));
        }
    }
    require_real_regular_file(&root.join(STATE_FILE_NAME), "host state")?;
    Ok(())
}

/// Reads and validates the manifest of a bundle directory.
fn read_manifest(dir: &Path) -> Result<PluginManifest, PluginError> {
    let path = dir.join(MANIFEST_FILE_NAME);
    let text = std::fs::read_to_string(&path)
        .map_err(|error| PluginError::io(format!("failed to read the plugin manifest: {error}")))?;
    PluginManifest::parse(&text)
}

/// Proves `root` is exactly the embedded bundle and returns its host state.
fn prove_embedded_bundle(root: &Path) -> Result<HostState, PluginError> {
    let embedded = embedded_manifest()?;
    verify_bundle(root, &embedded)?;
    let state = read_state_file(&root.join(STATE_FILE_NAME))
        .ok_or_else(|| PluginError::refused("the plugin bundle has no valid host state file"))?;
    if state.version != embedded.version {
        return Err(PluginError::refused(
            "the plugin bundle host state does not match the embedded bundle",
        ));
    }
    Ok(state)
}

/// Proves that the plugin directory, when it exists, is the embedded bundle
/// before it is replaced or removed. A missing directory owns nothing and is
/// fine; a symlink, a plain file, a foreign manifest, an extra entry or a
/// drifted asset is refused so the service never deletes unknown content.
fn assert_owned_bundle(paths: &PluginPaths) -> Result<(), PluginError> {
    let dir = &paths.plugin_dir;
    match std::fs::symlink_metadata(dir) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(PluginError::refused(
                "refusing to replace a plugin path that is a symlink",
            ))
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(PluginError::refused(
                "refusing to replace a plugin path that is not a directory",
            ))
        }
        Ok(_) => {}
        Err(error) => {
            return Err(PluginError::io(format!(
                "failed to inspect '{}': {error}",
                dir.display()
            )))
        }
    }
    prove_embedded_bundle(dir).map(|_| ())
}

/// Inspects an installed bundle, returning its manifest and lifecycle state.
///
/// Only a *complete evidence* bundle is installed: the manifest, every asset and
/// the host state must all be present as real regular files with the exact
/// embedded content and no extra entry. A missing manifest, a symlinked file, an
/// unexpected entry or an absent state file is not installed — the host state is
/// never fabricated.
fn inspect_installed(paths: &PluginPaths) -> Option<(PluginManifest, HostState)> {
    let state = prove_embedded_bundle(&paths.plugin_dir).ok()?;
    let manifest = embedded_manifest().ok()?;
    Some((manifest, state))
}

fn read_state_file(path: &Path) -> Option<HostState> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_state_file(path: &Path, state: &HostState) -> Result<(), PluginError> {
    let parent = path
        .parent()
        .ok_or_else(|| PluginError::io("the plugin state path has no parent directory"))?;
    std::fs::create_dir_all(parent).map_err(|error| {
        PluginError::io(format!("failed to create the plugin directory: {error}"))
    })?;
    let json = serde_json::to_string_pretty(state).map_err(|error| {
        PluginError::internal(format!("failed to serialize host state: {error}"))
    })?;
    // A process-unique name written with `create_new`, so the temp file can
    // never follow a pre-existing symlink into another file.
    let temp = parent.join(format!("{STATE_FILE_NAME}.{}.tmp", nonce()));
    write_new_file(&temp, json.as_bytes())
        .map_err(|error| PluginError::io(format!("failed to write host state: {error}")))?;
    std::fs::rename(&temp, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        PluginError::io(format!("failed to publish host state: {error}"))
    })?;
    Ok(())
}

fn write_asset(root: &Path, relative: &str, bytes: &[u8]) -> Result<(), PluginError> {
    let target = root.join(relative);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            PluginError::io(format!("failed to create asset directory: {error}"))
        })?;
    }
    write_new_file(&target, bytes)
        .map_err(|error| PluginError::io(format!("failed to write asset '{relative}': {error}")))
}

/// Atomically replaces `live` with `staged`. The previous `live` is moved aside
/// first and restored if publishing the new bundle fails, so a failed load
/// always keeps the old version.
///
/// The caller has already proven `live` is either absent or the owned embedded
/// bundle, so this only performs the swap.
fn replace_dir(live: &Path, staged: &Path, backup: &Path) -> Result<(), PluginError> {
    let had_old = live.exists();
    if had_old {
        std::fs::rename(live, backup).map_err(|error| {
            PluginError::io(format!("failed to set the old bundle aside: {error}"))
        })?;
    }
    match std::fs::rename(staged, live) {
        Ok(()) => {
            if had_old {
                if let Err(error) = std::fs::remove_dir_all(backup) {
                    log::warn!("failed to remove the previous plugin bundle backup: {error}");
                }
            }
            Ok(())
        }
        Err(error) => {
            if had_old {
                if let Err(restore) = std::fs::rename(backup, live) {
                    return Err(PluginError::io(format!(
                        "failed to publish the plugin bundle ({error}) and failed to restore the previous version ({restore})"
                    )));
                }
            }
            Err(PluginError::io(format!(
                "failed to publish the plugin bundle: {error}"
            )))
        }
    }
}

/// Removes the bundle fail-closed.
///
/// A terminal symlink is removed as a link so its target survives. A real
/// directory is only removed after the embedded ownership proof; a plain file
/// or a foreign/drifted directory is refused and left in place.
fn remove_bundle(paths: &PluginPaths) -> Result<(), PluginError> {
    let target = &paths.plugin_dir;
    let metadata = match std::fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(PluginError::io(format!(
                "failed to inspect '{}': {error}",
                target.display()
            )))
        }
    };
    if metadata.file_type().is_symlink() {
        return std::fs::remove_file(target).map_err(|error| {
            PluginError::io(format!(
                "failed to remove the plugin bundle symlink: {error}"
            ))
        });
    }
    if !metadata.is_dir() {
        return Err(PluginError::refused(
            "refusing to remove a plugin path that is not the owned bundle",
        ));
    }
    assert_owned_bundle(paths)?;
    std::fs::remove_dir_all(target).map_err(|error| {
        PluginError::io(format!("failed to remove '{}': {error}", target.display()))
    })
}

/// Re-proves that a path may be removed: it must be below the plugins root and
/// must not be the plugins root, the ChatSpeed home or the managed skills
/// directory (nor anything that contains it).
fn ensure_removable(paths: &PluginPaths, target: &Path) -> Result<(), PluginError> {
    if target == paths.plugins_root || !target.starts_with(&paths.plugins_root) {
        return Err(PluginError::refused(
            "refusing to remove a path that is not a plugin-owned bundle",
        ));
    }
    if target == paths.chatspeed_home {
        return Err(PluginError::refused(
            "refusing to remove the ChatSpeed home directory",
        ));
    }
    if target == paths.skills_dir || paths.skills_dir.starts_with(target) {
        return Err(PluginError::refused(
            "refusing to remove the managed skills directory",
        ));
    }
    if target.starts_with(&paths.skills_dir) {
        return Err(PluginError::refused(
            "refusing to remove a path inside the managed skills directory",
        ));
    }
    Ok(())
}

/// Removes a plugin-owned path after re-proving it is confined, never
/// following a symlink (so a link cannot redirect the removal into a target).
fn remove_owned(paths: &PluginPaths, target: &Path) -> Result<(), PluginError> {
    ensure_removable(paths, target)?;
    let metadata = match std::fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(PluginError::io(format!(
                "failed to inspect '{}': {error}",
                target.display()
            )))
        }
    };
    let result = if metadata.is_dir() {
        std::fs::remove_dir_all(target)
    } else {
        std::fs::remove_file(target)
    };
    result.map_err(|error| {
        PluginError::io(format!("failed to remove '{}': {error}", target.display()))
    })
}

/// Clears only residue that independently proves it is the embedded bundle.
///
/// A staging or backup directory is removed only when it passes the same
/// ownership proof the live bundle must pass: a directory whose name merely
/// matches the plugin prefix, a partial staging tree, a symlink or any foreign
/// directory is preserved instead of deleted.
fn cleanup_residue(paths: &PluginPaths) -> Result<(), PluginError> {
    remove_proven_residue(
        paths,
        &paths.staging_root,
        &format!("{PLUGIN_ID}-"),
    )?;
    remove_proven_residue(
        paths,
        &paths.plugins_root,
        &format!(".{PLUGIN_ID}-backup-"),
    )?;
    Ok(())
}

/// Removes prefix-matched entries of `root` only after [`prove_embedded_bundle`]
/// confirms each is the embedded bundle.
fn remove_proven_residue(
    paths: &PluginPaths,
    root: &Path,
    prefix: &str,
) -> Result<(), PluginError> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(prefix) {
            continue;
        }
        let path = entry.path();
        if prove_embedded_bundle(&path).is_ok() {
            remove_owned(paths, &path)?;
        }
    }
    Ok(())
}

/// Restricts a directory to the current user on Unix.
fn restrict_permissions(path: &Path) -> Result<(), PluginError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| PluginError::io(format!("failed to restrict directory permissions: {error}")),
        )?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
/// A digest of the embedded bundle: every path and its content hash.
fn embedded_bundle_digest() -> String {
    let mut canonical = String::new();
    for asset in EMBEDDED_ASSETS {
        canonical.push_str(asset.path);
        canonical.push('\0');
        canonical.push_str(&sha256_hex(asset.bytes));
    }
    sha256_hex(canonical.as_bytes())
}

/// A digest of the installed bundle, or `None` when any asset is unreadable.
fn installed_bundle_digest(root: &Path, assets: &[String]) -> Option<String> {
    if assets.is_empty() {
        return None;
    }
    let mut sorted: Vec<&String> = assets.iter().collect();
    sorted.sort();
    let mut canonical = String::new();
    for asset in sorted {
        let bytes = std::fs::read(root.join(asset)).ok()?;
        canonical.push_str(asset);
        canonical.push('\0');
        canonical.push_str(&sha256_hex(&bytes));
    }
    Some(sha256_hex(canonical.as_bytes()))
}

/// A process-unique staging/backup suffix.
fn nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}-{counter}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn embedded_json() -> serde_json::Value {
        serde_json::from_str(EMBEDDED_MANIFEST).expect("embedded manifest json")
    }

    fn service_at(root: &Path) -> PluginService {
        PluginService::with_paths(PluginPaths::under(root.to_path_buf()))
    }

    fn staged_is_empty(paths: &PluginPaths) -> bool {
        paths
            .staging_root
            .read_dir()
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true)
    }

    #[test]
    fn embedded_manifest_declares_the_plugin_contract() {
        let manifest = embedded_manifest().expect("embedded manifest validates");
        assert_eq!(manifest.schema, PLUGIN_SCHEMA);
        assert_eq!(manifest.id, PLUGIN_ID);
        assert_eq!(manifest.entry, "index.html");
        assert_eq!(manifest.assets, vec!["index.html".to_string()]);
        assert_eq!(manifest.permissions, vec!["skills:read".to_string()]);
        assert!(semver::Version::parse(&manifest.version).is_ok());
    }

    #[test]
    fn the_embedded_bundle_and_manifest_agree() {
        let json = embedded_json();
        let declared: std::collections::BTreeSet<String> = json["assets"]
            .as_array()
            .expect("assets array")
            .iter()
            .map(|value| value.as_str().expect("asset string").to_string())
            .collect();
        let embedded: std::collections::BTreeSet<String> = EMBEDDED_ASSETS
            .iter()
            .map(|asset| asset.path.to_string())
            .collect();
        assert_eq!(declared, embedded);
        assert!(!embedded.is_empty());
    }

    #[test]
    fn the_embedded_bundle_is_flat_for_the_exact_file_set_proof() {
        // The exact-set proof compares top-level directory entries, so every
        // embedded asset must stay a single path component.
        for asset in EMBEDDED_ASSETS {
            assert!(
                !asset.path.contains('/'),
                "asset '{}' must be a single directory entry",
                asset.path
            );
        }
    }

    #[test]
    fn manifest_rejects_unknown_permissions() {
        let mut json = embedded_json();
        json["permissions"] = serde_json::json!(["skills:read", "fs:write"]);
        let error = PluginManifest::parse(&json.to_string()).expect_err("must be refused");
        assert_eq!(error.code, plugin_code::INVALID_MANIFEST);
    }

    #[test]
    fn manifest_rejects_path_traversal_and_absolute_assets() {
        let mut traversal = embedded_json();
        traversal["entry"] = serde_json::json!("../escape.html");
        assert!(PluginManifest::parse(&traversal.to_string()).is_err());

        let mut absolute = embedded_json();
        absolute["entry"] = serde_json::json!("/etc/passwd");
        absolute["assets"] = serde_json::json!(["/etc/passwd"]);
        assert!(PluginManifest::parse(&absolute.to_string()).is_err());
    }

    #[test]
    fn manifest_rejects_unknown_fields_and_bad_identity() {
        let mut extra = embedded_json();
        extra["extra"] = serde_json::json!(1);
        assert!(PluginManifest::parse(&extra.to_string()).is_err());

        let mut wrong_id = embedded_json();
        wrong_id["id"] = serde_json::json!("something-else");
        assert!(PluginManifest::parse(&wrong_id.to_string()).is_err());

        let mut bad_version = embedded_json();
        bad_version["version"] = serde_json::json!("not-semver");
        assert!(PluginManifest::parse(&bad_version.to_string()).is_err());

        let mut missing_entry = embedded_json();
        missing_entry["entry"] = serde_json::json!("other.html");
        assert!(PluginManifest::parse(&missing_entry.to_string()).is_err());
    }

    #[test]
    fn load_stages_verifies_and_publishes_the_bundle() {
        let temp = TempDir::new().expect("temp dir");
        let service = service_at(temp.path());

        let inventory = service.load().expect("load");
        assert!(inventory.installed);
        assert!(inventory.enabled);
        assert_eq!(inventory.version.as_deref(), Some("0.1.0"));
        let expected_digest = embedded_bundle_digest();
        assert_eq!(
            inventory.bundle_digest.as_deref(),
            Some(expected_digest.as_str())
        );

        let paths = PluginPaths::under(temp.path().to_path_buf());
        assert!(paths.plugin_dir.join(MANIFEST_FILE_NAME).is_file());
        assert!(paths.plugin_dir.join("index.html").is_file());
        assert!(paths.state_file.is_file());
        assert!(
            !paths.skills_dir.exists(),
            "load must never create the managed skills directory"
        );
        assert!(
            staged_is_empty(&paths),
            "a successful load leaves no staging residue"
        );
    }

    #[test]
    fn a_pristine_second_load_replaces_the_previous_bundle() {
        let temp = TempDir::new().expect("temp dir");
        let service = service_at(temp.path());
        service.load().expect("first load");

        let paths = PluginPaths::under(temp.path().to_path_buf());
        let inventory = service.load().expect("second load");
        assert!(inventory.installed);
        assert!(inventory.enabled);
        assert!(paths.plugin_dir.join(MANIFEST_FILE_NAME).is_file());
        assert!(paths.plugin_dir.join("index.html").is_file());
        assert!(staged_is_empty(&paths), "no staging residue may survive");
    }

    #[test]
    fn a_second_load_refuses_a_bundle_with_an_extra_file() {
        let temp = TempDir::new().expect("temp dir");
        let service = service_at(temp.path());
        service.load().expect("first load");

        let paths = PluginPaths::under(temp.path().to_path_buf());
        std::fs::write(paths.plugin_dir.join("stray.txt"), b"stray").expect("write stray");

        // An undeclared extra entry makes the directory unprovable, so a second
        // load must refuse and leave the unknown content untouched.
        let error = service.load().expect_err("an extra file must be refused");
        assert_eq!(error.code, plugin_code::REFUSED);
        assert!(
            paths.plugin_dir.join("stray.txt").is_file(),
            "unknown content must be preserved, never deleted"
        );
        assert!(paths.plugin_dir.join("index.html").is_file());
    }

    #[test]
    fn uninstall_refuses_a_bundle_with_an_extra_file() {
        let temp = TempDir::new().expect("temp dir");
        let service = service_at(temp.path());
        service.load().expect("first load");

        let paths = PluginPaths::under(temp.path().to_path_buf());
        std::fs::write(paths.plugin_dir.join("stray.txt"), b"stray").expect("write stray");

        let error = service.uninstall().expect_err("an extra file must be refused");
        assert_eq!(error.code, plugin_code::REFUSED);
        assert!(
            paths.plugin_dir.join("stray.txt").is_file(),
            "unknown content must be preserved, never deleted"
        );
        assert!(paths.plugin_dir.exists());
    }

    #[test]
    fn replace_dir_restores_the_previous_version_on_failure() {
        let temp = TempDir::new().expect("temp dir");
        let live = temp.path().join("live");
        std::fs::create_dir_all(&live).expect("live");
        std::fs::write(live.join("old.txt"), b"old").expect("old");
        let missing_staged = temp.path().join("missing-staged");
        let backup = temp.path().join("backup");

        let error = replace_dir(&live, &missing_staged, &backup).expect_err("must fail");
        assert_eq!(error.code, plugin_code::IO);
        assert!(
            live.join("old.txt").is_file(),
            "the old version must survive"
        );
        assert!(!backup.exists(), "the backup must be restored");
    }

    #[test]
    fn uninstall_removes_only_the_plugin_bundle() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path();
        // The managed skills directory and an external target must survive.
        let skills = home.join("skills");
        std::fs::create_dir_all(&skills).expect("skills");
        std::fs::write(skills.join("SKILL.md"), b"managed").expect("skill");
        let external_target = temp.path().join(".claude").join("skills");
        std::fs::create_dir_all(&external_target).expect("external target");
        std::fs::write(external_target.join("other.md"), b"target").expect("target file");

        let service = service_at(home);
        service.load().expect("load");
        let inventory = service.uninstall().expect("uninstall");
        assert!(!inventory.installed);

        let paths = PluginPaths::under(home.to_path_buf());
        assert!(!paths.plugin_dir.exists());
        assert!(
            paths.plugins_root.is_dir(),
            "the plugins root is not plugin-owned"
        );
        assert!(
            skills.join("SKILL.md").is_file(),
            "skills must be untouched"
        );
        assert!(
            external_target.join("other.md").is_file(),
            "targets must be untouched"
        );
    }

    #[test]
    fn uninstall_refuses_a_bundle_inside_the_skills_directory() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().to_path_buf();
        let skills = home.join("skills");
        std::fs::create_dir_all(&skills).expect("skills");
        std::fs::write(skills.join("SKILL.md"), b"managed").expect("skill");

        let mut paths = PluginPaths::under(home);
        paths.plugin_dir = paths.skills_dir.join(PLUGIN_ID);
        let service = PluginService::with_paths(paths);

        let error = service.uninstall().expect_err("must be refused");
        assert_eq!(error.code, plugin_code::REFUSED);
        assert!(skills.join("SKILL.md").is_file());
    }

    #[test]
    fn uninstall_refuses_when_the_bundle_is_the_skills_directory() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().to_path_buf();
        let mut paths = PluginPaths::under(home);
        paths.plugin_dir = paths.skills_dir.clone();
        let service = PluginService::with_paths(paths);

        let error = service.uninstall().expect_err("must be refused");
        assert_eq!(error.code, plugin_code::REFUSED);
    }

    #[cfg(unix)]
    #[test]
    fn uninstall_does_not_follow_a_bundle_symlink() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path();
        let skills = home.join("skills");
        std::fs::create_dir_all(&skills).expect("skills");
        std::fs::write(skills.join("SKILL.md"), b"managed").expect("skill");

        let paths = PluginPaths::under(home.to_path_buf());
        std::fs::create_dir_all(&paths.plugins_root).expect("plugins root");
        std::os::unix::fs::symlink(&skills, &paths.plugin_dir).expect("symlink");

        let service = PluginService::with_paths(paths.clone());
        service.uninstall().expect("uninstall");
        assert!(!paths.plugin_dir.exists(), "the link must be removed");
        assert!(
            skills.join("SKILL.md").is_file(),
            "the link target must survive"
        );
    }

    #[test]
    fn disable_marks_the_bundle_disabled_without_touching_assets() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path();
        let service = service_at(home);
        service.load().expect("load");

        let inventory = service.disable().expect("disable");
        assert!(inventory.installed);
        assert!(!inventory.enabled);

        let paths = PluginPaths::under(home.to_path_buf());
        assert!(paths.plugin_dir.join("index.html").is_file());

        let empty = TempDir::new().expect("temp dir");
        let error = service_at(empty.path())
            .disable()
            .expect_err("must be refused");
        assert_eq!(error.code, plugin_code::NOT_INSTALLED);
    }

    #[test]
    fn inventory_on_an_empty_home_reports_not_installed() {
        let temp = TempDir::new().expect("temp dir");
        let inventory = service_at(temp.path()).inventory().expect("inventory");
        assert!(!inventory.installed);
        assert!(!inventory.enabled);
        assert!(inventory.version.is_none());
        assert_eq!(inventory.uninstall_scope, UNINSTALL_SCOPE);
    }

    #[test]
    fn inventory_reports_the_provable_isolation_contract() {
        let temp = TempDir::new().expect("temp dir");
        let inventory = service_at(temp.path()).inventory().expect("inventory");
        assert_eq!(inventory.host.host, HOST_KIND);
        assert_eq!(inventory.host.ipc_surface, IPC_SURFACE);
        assert!(!inventory.host.plugin_code_executed);
        assert!(!inventory.host.tauri_ipc);
        assert!(!inventory.host.generic_command_surface);
        assert!(!inventory.host.filesystem_api);
        assert!(!inventory.host.database_access);
        assert!(!inventory.host.token_access);
    }

    #[test]
    fn inventory_serializes_the_expected_wire_shape() {
        let temp = TempDir::new().expect("temp dir");
        let value = serde_json::to_value(service_at(temp.path()).inventory().expect("inventory"))
            .expect("serialize");
        assert_eq!(value["plugin_id"], PLUGIN_ID);
        assert_eq!(value["schema"], PLUGIN_SCHEMA);
        assert_eq!(value["installed"], serde_json::json!(false));
        assert_eq!(value["uninstall_scope"], UNINSTALL_SCOPE);
        assert!(value["root"]
            .as_str()
            .is_some_and(|root| root.ends_with(PLUGIN_ID)));
        // A token or credential must never appear in the inventory wire shape.
        assert!(value.get("token").is_none());
        assert!(value.get("access_token").is_none());
    }

    #[test]
    fn uninstall_refuses_a_foreign_directory_named_agent_skills() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path();
        let paths = PluginPaths::under(home.to_path_buf());
        // An arbitrary user directory that only shares the bundle's name: no
        // manifest, no host state. It must never be deleted.
        std::fs::create_dir_all(&paths.plugin_dir).expect("foreign dir");
        std::fs::write(paths.plugin_dir.join("keep.txt"), b"keep").expect("keep");

        let service = PluginService::with_paths(paths.clone());
        let error = service.uninstall().expect_err("must be refused");
        assert_eq!(error.code, plugin_code::REFUSED);
        assert!(
            paths.plugin_dir.join("keep.txt").is_file(),
            "foreign content must survive"
        );
    }

    #[test]
    fn load_refuses_to_replace_a_foreign_directory() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path();
        let paths = PluginPaths::under(home.to_path_buf());
        std::fs::create_dir_all(&paths.plugin_dir).expect("foreign dir");
        std::fs::write(paths.plugin_dir.join("keep.txt"), b"keep").expect("keep");

        let service = PluginService::with_paths(paths.clone());
        let error = service.load().expect_err("must be refused");
        assert_eq!(error.code, plugin_code::REFUSED);
        assert!(
            paths.plugin_dir.join("keep.txt").is_file(),
            "foreign content must survive"
        );
        assert!(
            !paths.plugin_dir.join(MANIFEST_FILE_NAME).exists(),
            "the refused load must not publish the embedded bundle"
        );
    }

    #[test]
    fn uninstall_refuses_a_drifted_bundle() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path();
        let service = service_at(home);
        service.load().expect("load");

        let paths = PluginPaths::under(home.to_path_buf());
        std::fs::write(paths.plugin_dir.join("index.html"), b"tampered").expect("tamper");

        let error = service.uninstall().expect_err("must be refused");
        assert_eq!(error.code, plugin_code::REFUSED);
        assert!(paths.plugin_dir.exists(), "drifted content must survive");
    }

    #[test]
    fn load_refuses_a_drifted_bundle() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path();
        let service = service_at(home);
        service.load().expect("load");

        let paths = PluginPaths::under(home.to_path_buf());
        // A drifted host state is ownership proof failure too.
        std::fs::write(
            paths.state_file.clone(),
            br#"{"enabled":true,"version":"9.9.9"}"#,
        )
        .expect("tamper state");

        let error = service.load().expect_err("must be refused");
        assert_eq!(error.code, plugin_code::REFUSED);
        assert!(paths.plugin_dir.join("index.html").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn lifecycle_refuses_a_symlinked_plugins_root() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path();
        let skills = home.join("skills");
        std::fs::create_dir_all(&skills).expect("skills");
        std::fs::write(skills.join("SKILL.md"), b"managed").expect("skill");

        let paths = PluginPaths::under(home.to_path_buf());
        // `plugins/` is a link into the managed skills directory: every
        // lifecycle path is redirected and must fail closed.
        std::os::unix::fs::symlink(&skills, &paths.plugins_root).expect("symlink root");

        let service = PluginService::with_paths(paths.clone());
        assert_eq!(service.load().expect_err("load").code, plugin_code::REFUSED);
        assert_eq!(
            service.disable().expect_err("disable").code,
            plugin_code::REFUSED
        );
        assert_eq!(
            service.uninstall().expect_err("uninstall").code,
            plugin_code::REFUSED
        );
        assert!(
            !service.inventory().expect("inventory").installed,
            "a redirected read reports not installed"
        );
        assert!(
            skills.join("SKILL.md").is_file(),
            "the skills directory must be untouched"
        );
        assert!(
            !skills.join(MANIFEST_FILE_NAME).exists(),
            "no bundle may be published into the link target"
        );
    }

    #[cfg(unix)]
    #[test]
    fn lifecycle_refuses_a_symlinked_staging_root() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path();
        let skills = home.join("skills");
        std::fs::create_dir_all(&skills).expect("skills");
        std::fs::write(skills.join("SKILL.md"), b"managed").expect("skill");

        let paths = PluginPaths::under(home.to_path_buf());
        std::fs::create_dir_all(&paths.plugins_root).expect("plugins root");
        std::os::unix::fs::symlink(&skills, &paths.staging_root).expect("symlink staging");

        let service = PluginService::with_paths(paths.clone());
        assert_eq!(service.load().expect_err("load").code, plugin_code::REFUSED);
        assert_eq!(
            service.uninstall().expect_err("uninstall").code,
            plugin_code::REFUSED
        );
        assert!(
            skills.join("SKILL.md").is_file(),
            "the skills directory must be untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn disable_refuses_a_symlinked_bundle_without_writing_through_it() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path();
        let skills = home.join("skills");
        std::fs::create_dir_all(&skills).expect("skills");
        std::fs::write(skills.join("SKILL.md"), b"managed").expect("skill");

        let paths = PluginPaths::under(home.to_path_buf());
        std::fs::create_dir_all(&paths.plugins_root).expect("plugins root");
        std::os::unix::fs::symlink(&skills, &paths.plugin_dir).expect("symlink bundle");

        let service = PluginService::with_paths(paths.clone());
        let error = service.disable().expect_err("must be refused");
        assert_eq!(error.code, plugin_code::REFUSED);
        assert!(
            !skills.join(STATE_FILE_NAME).exists(),
            "the state write must never follow the link into skills"
        );
    }

    #[cfg(unix)]
    #[test]
    fn inventory_does_not_follow_a_symlinked_bundle() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path();
        let skills = home.join("skills");
        std::fs::create_dir_all(&skills).expect("skills");
        // A full-looking bundle behind the link must still be ignored.
        std::fs::write(skills.join(MANIFEST_FILE_NAME), EMBEDDED_MANIFEST).expect("manifest");
        std::fs::write(skills.join("index.html"), EMBEDDED_ENTRY).expect("entry");
        std::fs::write(skills.join("SKILL.md"), b"managed").expect("skill");

        let paths = PluginPaths::under(home.to_path_buf());
        std::fs::create_dir_all(&paths.plugins_root).expect("plugins root");
        std::os::unix::fs::symlink(&skills, &paths.plugin_dir).expect("symlink bundle");

        let service = PluginService::with_paths(paths.clone());
        let inventory = service.inventory().expect("inventory");
        assert!(
            !inventory.installed,
            "a symlinked bundle is never read as installed"
        );
    }

    #[test]
    fn concurrent_load_and_uninstall_are_serialized() {
        let temp = TempDir::new().expect("temp dir");
        let paths = PluginPaths::under(temp.path().to_path_buf());
        let service = Arc::new(PluginService::with_paths(paths.clone()));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let loader = Arc::clone(&service);
            handles.push(std::thread::spawn(move || {
                let _ = loader.load();
            }));
            let cleaner = Arc::clone(&service);
            handles.push(std::thread::spawn(move || {
                let _ = cleaner.uninstall();
            }));
        }
        for handle in handles {
            handle.join().expect("a lifecycle call must not panic");
        }

        // Whichever operation ran last, the service is in one consistent state
        // and leaves no staging residue behind.
        let inventory = service.inventory().expect("inventory");
        if inventory.installed {
            assert!(paths.plugin_dir.join("index.html").is_file());
        } else {
            assert!(!paths.plugin_dir.exists());
        }
        assert!(staged_is_empty(&paths), "no staging residue may survive");
    }

    #[test]
    fn a_missing_host_state_is_not_fabricated() {
        let temp = TempDir::new().expect("temp dir");
        let service = service_at(temp.path());
        service.load().expect("load");

        let paths = PluginPaths::under(temp.path().to_path_buf());
        std::fs::remove_file(&paths.state_file).expect("remove state");

        // Without the state file the evidence is incomplete, so the read reports
        // "not installed" instead of fabricating an enabled host state.
        let inventory = service.inventory().expect("inventory");
        assert!(!inventory.installed);
        assert!(!inventory.enabled);

        assert_eq!(
            service.uninstall().expect_err("uninstall").code,
            plugin_code::REFUSED
        );
        assert!(paths.plugin_dir.join("index.html").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_state_file_is_refused() {
        let temp = TempDir::new().expect("temp dir");
        let service = service_at(temp.path());
        service.load().expect("load");

        let paths = PluginPaths::under(temp.path().to_path_buf());
        let real = temp.path().join("real-state.json");
        std::fs::write(&real, br#"{"enabled":false,"version":"0.1.0"}"#).expect("real state");
        std::fs::remove_file(&paths.state_file).expect("remove state");
        std::os::unix::fs::symlink(&real, &paths.state_file).expect("symlink state");

        assert!(
            !service.inventory().expect("inventory").installed,
            "a symlinked state file is not provable"
        );
        assert_eq!(
            service.uninstall().expect_err("uninstall").code,
            plugin_code::REFUSED
        );
        assert!(is_symlink(&paths.state_file), "the link must be preserved");
        assert!(
            paths.plugin_dir.join("index.html").is_file(),
            "the assets must survive"
        );
        assert!(real.is_file(), "the link target must survive");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_asset_is_never_read_or_written_through() {
        let temp = TempDir::new().expect("temp dir");
        let service = service_at(temp.path());
        service.load().expect("load");

        let paths = PluginPaths::under(temp.path().to_path_buf());
        // A link whose target carries the *correct* bytes must still be refused:
        // the proof is about the entry type, not only the content.
        let real = temp.path().join("real-index.html");
        std::fs::write(&real, EMBEDDED_ENTRY).expect("real asset");
        let asset = paths.plugin_dir.join("index.html");
        std::fs::remove_file(&asset).expect("remove asset");
        std::os::unix::fs::symlink(&real, &asset).expect("symlink asset");

        assert!(!service.inventory().expect("inventory").installed);
        assert_eq!(service.load().expect_err("load").code, plugin_code::REFUSED);
        assert_eq!(
            service.uninstall().expect_err("uninstall").code,
            plugin_code::REFUSED
        );
        assert!(is_symlink(&asset), "the link must be preserved");
        assert!(real.is_file(), "the link target must survive");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_manifest_is_refused() {
        let temp = TempDir::new().expect("temp dir");
        let service = service_at(temp.path());
        service.load().expect("load");

        let paths = PluginPaths::under(temp.path().to_path_buf());
        let real = temp.path().join("real-plugin.json");
        std::fs::write(&real, EMBEDDED_MANIFEST).expect("real manifest");
        let manifest = paths.plugin_dir.join(MANIFEST_FILE_NAME);
        std::fs::remove_file(&manifest).expect("remove manifest");
        std::os::unix::fs::symlink(&real, &manifest).expect("symlink manifest");

        assert!(!service.inventory().expect("inventory").installed);
        assert_eq!(service.load().expect_err("load").code, plugin_code::REFUSED);
        assert_eq!(
            service.uninstall().expect_err("uninstall").code,
            plugin_code::REFUSED
        );
        assert!(is_symlink(&manifest), "the link must be preserved");
    }

    #[cfg(unix)]
    #[test]
    fn lifecycle_refuses_a_symlinked_chatspeed_home_anchor() {
        let temp = TempDir::new().expect("temp dir");
        let real = temp.path().join("real-home");
        std::fs::create_dir_all(&real).expect("real home");
        let link = temp.path().join("chatspeed-link");
        std::os::unix::fs::symlink(&real, &link).expect("symlink home");

        let paths = PluginPaths::under(link.clone());
        let service = PluginService::with_paths(paths);
        assert_eq!(service.load().expect_err("load").code, plugin_code::REFUSED);
        assert_eq!(
            service.disable().expect_err("disable").code,
            plugin_code::REFUSED
        );
        assert_eq!(
            service.uninstall().expect_err("uninstall").code,
            plugin_code::REFUSED
        );
        assert!(!service.inventory().expect("inventory").installed);
        assert!(
            !real.join("plugins").exists(),
            "nothing may be written through the home link"
        );
    }

    #[cfg(unix)]
    #[test]
    fn lifecycle_refuses_a_symlinked_chatspeed_home_parent() {
        let temp = TempDir::new().expect("temp dir");
        let real = temp.path().join("real-home");
        std::fs::create_dir_all(&real).expect("real home");
        let link = temp.path().join(".agents");
        std::os::unix::fs::symlink(&real, &link).expect("symlink parent");

        // CHATSPEED_HOME = `<link>/skills`: its own parent `.agents` is a link,
        // so the whole chain is refused before anything is written.
        let paths = PluginPaths::under(link.join("skills"));
        let service = PluginService::with_paths(paths);
        assert_eq!(service.load().expect_err("load").code, plugin_code::REFUSED);
        assert_eq!(
            service.uninstall().expect_err("uninstall").code,
            plugin_code::REFUSED
        );
        assert!(!service.inventory().expect("inventory").installed);
        assert!(
            !real.join("skills").exists(),
            "nothing may be written through the parent link"
        );
    }

    #[test]
    fn a_chatspeed_home_inside_a_registered_skill_target_is_refused() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).expect("home");
        // The registered `agents` target is `<home>/.agents/skills`; a
        // CHATSPEED_HOME there would publish the bundle into that Skill target.
        let chatspeed_home = home.join(".agents").join("skills");
        let paths = PluginPaths::under(chatspeed_home.clone());
        let environment = TargetEnvironment::injected(home.clone(), chatspeed_home);
        let error = paths
            .ensure_clear_of_skill_targets(&environment)
            .expect_err("a home inside a target must be refused");
        assert_eq!(error.code, plugin_code::REFUSED);

        // The default `<home>/.chatspeed` home is clear of every target.
        let default_home = temp.path().join(".chatspeed");
        let paths = PluginPaths::under(default_home.clone());
        let environment = TargetEnvironment::injected(home, default_home);
        paths
            .ensure_clear_of_skill_targets(&environment)
            .expect("the default home is clear of every target");
    }

    #[test]
    fn uninstall_preserves_foreign_staging_and_backup_residue() {
        let temp = TempDir::new().expect("temp dir");
        let service = service_at(temp.path());
        service.load().expect("load");
        let paths = PluginPaths::under(temp.path().to_path_buf());

        // Directories that only share the naming prefix are foreign content.
        let foreign_staging = paths.staging_root.join(format!("{PLUGIN_ID}-foreign"));
        std::fs::create_dir_all(&foreign_staging).expect("foreign staging");
        std::fs::write(foreign_staging.join("keep.txt"), b"keep").expect("keep");
        let foreign_backup = paths
            .plugins_root
            .join(format!(".{PLUGIN_ID}-backup-foreign"));
        std::fs::create_dir_all(&foreign_backup).expect("foreign backup");
        std::fs::write(foreign_backup.join("keep.txt"), b"keep").expect("keep");

        let inventory = service.uninstall().expect("uninstall");
        assert!(!inventory.installed);
        assert!(
            foreign_staging.join("keep.txt").is_file(),
            "foreign staging must survive"
        );
        assert!(
            foreign_backup.join("keep.txt").is_file(),
            "foreign backup must survive"
        );
        assert!(!paths.plugin_dir.exists());
    }

    #[test]
    fn uninstall_removes_residue_that_proves_it_is_the_embedded_bundle() {
        let temp = TempDir::new().expect("temp dir");
        let service = service_at(temp.path());
        service.load().expect("load");
        let paths = PluginPaths::under(temp.path().to_path_buf());

        // A leftover staging directory that holds exactly the embedded bundle
        // passes the same ownership proof and is cleared.
        let residue = paths.staging_root.join(format!("{PLUGIN_ID}-orphan"));
        std::fs::create_dir_all(&residue).expect("residue");
        stage_into(&residue, &embedded_manifest().expect("manifest")).expect("stage residue");

        service.uninstall().expect("uninstall");
        assert!(!residue.exists(), "proven residue is cleared");
    }
}