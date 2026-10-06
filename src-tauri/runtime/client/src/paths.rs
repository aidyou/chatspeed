//! Shared, Tauri-free runtime path resolution.
//!
//! The runtime process, the desktop client and `cscli` must agree on *exactly*
//! where the runtime directory, the database and the application-data directory
//! live, or two processes can end up owning different databases while believing
//! they share one. This module is that single resolution rule.
//!
//! It deliberately depends on neither Tauri nor the desktop crate, so the
//! standalone `chatspeed-runtime` binary can compute its own paths without an
//! `AppHandle`. Production and development resolve to isolated roots:
//!
//! - production: `<platform data dir>/ai.aidyou.chatspeed`, matching Tauri v2's
//!   `app_data_dir()` (`dirs::data_dir().join(identifier)`);
//! - development: `<repository root>/dev_data`, located from this crate's
//!   compile-time `CARGO_MANIFEST_DIR` rather than the process working directory.
//!
//! An explicitly configured runtime directory (an argument, or
//! `CHATSPEED_RUNTIME_DIR`) is a sandbox: the database and the application-data
//! directory are placed inside it. `CHATSPEED_RUNTIME_DB` overrides the database
//! file, and the application-data directory then follows the database's parent so
//! a redirected database cannot pull resources from another profile.
//! `CHATSPEED_HOME` is the legacy opt-in that keeps the `${home}/runtime` layout.
//!
//! When no platform directory is resolvable and no override is present, the
//! resolver fails closed: there is no `.`-style implicit fallback.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use chatspeed_contracts::DISCOVERY_FILE_NAME;

/// Environment key naming the runtime-owned directory (the test sandbox root).
pub const RUNTIME_DIR_ENV: &str = "CHATSPEED_RUNTIME_DIR";
/// Environment key overriding the runtime database file path.
pub const RUNTIME_DB_ENV: &str = "CHATSPEED_RUNTIME_DB";
/// Environment key overriding the runtime application-data directory.
pub const RUNTIME_APP_DATA_DIR_ENV: &str = "CHATSPEED_RUNTIME_APP_DATA_DIR";
/// Legacy environment key selecting a ChatSpeed home; keeps `${home}/runtime`.
pub const LEGACY_HOME_ENV: &str = "CHATSPEED_HOME";

/// Production bundle identifier. Tauri v2 resolves `app_data_dir()` to
/// `dirs::data_dir().join(identifier)`, so the standalone resolver uses the same
/// component to reach the same directory the desktop historically used.
pub const PRODUCTION_APP_IDENTIFIER: &str = "ai.aidyou.chatspeed";

/// Database file name inside the application-data directory.
pub const DB_FILE_NAME: &str = "chatspeed.db";

/// Subdirectory of the application-data directory the runtime exclusively owns.
pub const RUNTIME_DIR_NAME: &str = "runtime";

/// Development data directory name under the repository root.
pub const DEV_DATA_DIR_NAME: &str = "dev_data";

/// The build profile a resolved path belongs to.
///
/// The distinction matters because a release runtime binary started by a debug
/// client must still be told the development paths explicitly; resolving "the"
/// default is never enough on its own for a cross-profile spawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    /// `debug_assertions` enabled: repository-local `dev_data`.
    Debug,
    /// Release: the per-user platform data directory.
    Release,
}

/// The profile this binary was compiled for.
pub fn build_profile() -> BuildProfile {
    if cfg!(debug_assertions) {
        BuildProfile::Debug
    } else {
        BuildProfile::Release
    }
}

/// Production application-data directory for a resolved platform data directory.
///
/// `dirs::data_dir()` follows the platform rules (`$XDG_DATA_HOME` or
/// `$HOME/.local/share` on Linux, `$HOME/Library/Application Support` on macOS
/// and the roaming AppData folder on Windows); the identifier component is what
/// makes it the same directory Tauri v2's `app_data_dir()` returns.
pub fn production_app_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(PRODUCTION_APP_IDENTIFIER)
}

/// Development application-data directory: `<repository root>/dev_data`.
///
/// The repository root is located from this crate's compile-time
/// `CARGO_MANIFEST_DIR`, walking up to the `src-tauri` workspace directory, so
/// every workspace crate (`runtime-client`, `runtime`, the desktop crate)
/// resolves the same root regardless of the process working directory. Returns
/// `None` when the directory is not a `src-tauri` child, which fails the
/// resolver closed instead of guessing.
pub fn dev_app_data_dir() -> Option<PathBuf> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_tauri = manifest_dir
        .ancestors()
        .find(|dir| dir.file_name() == Some(OsStr::new("src-tauri")))?;
    Some(src_tauri.parent()?.join(DEV_DATA_DIR_NAME))
}

/// Why a launch configuration could not be resolved.
#[derive(Debug, thiserror::Error)]
pub enum LaunchConfigError {
    /// No platform data directory was available and no override was configured.
    #[error("cannot resolve the platform data directory; set {RUNTIME_DIR_ENV} to an explicit runtime directory")]
    PlatformDataDirUnavailable,
    /// The repository root could not be located from the crate manifest.
    #[error("cannot locate the repository root from the crate manifest directory")]
    ProjectRootUnavailable,
    /// A redirected database file had no parent directory to anchor the
    /// application-data directory to.
    #[error("database path {} has no parent directory", .0.display())]
    DatabasePathHasNoParent(PathBuf),
}

/// A fully resolved runtime launch configuration.
///
/// Every field is explicit so a spawn can pass all three to the runtime child
/// instead of relying on inherited environment or a matching build profile. The
/// three paths are the complete authority: the runtime directory owns the lock
/// and discovery document, the database file is the persistence authority, and
/// the application-data directory holds server-owned capability storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLaunchConfig {
    runtime_dir: PathBuf,
    db_path: PathBuf,
    app_data_dir: PathBuf,
}

impl RuntimeLaunchConfig {
    /// Builds a configuration from three explicit paths.
    pub fn new(
        runtime_dir: impl Into<PathBuf>,
        db_path: impl Into<PathBuf>,
        app_data_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            runtime_dir: runtime_dir.into(),
            db_path: db_path.into(),
            app_data_dir: app_data_dir.into(),
        }
    }

    /// Directory the runtime exclusively owns (lock and discovery document).
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    /// Persistent database file the runtime owns.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Application-data directory for server-owned capability storage.
    pub fn app_data_dir(&self) -> &Path {
        &self.app_data_dir
    }

    /// Discovery document path inside [`RuntimeLaunchConfig::runtime_dir`].
    pub fn discovery_file(&self) -> PathBuf {
        self.runtime_dir.join(DISCOVERY_FILE_NAME)
    }
}

/// Builds the explicit-sandbox configuration for an owned runtime directory.
///
/// The database and application-data directory default to children of the
/// runtime directory. This is infallible and never consults the environment, so
/// a caller that already knows its runtime directory (a test, or an explicit
/// CLI override) keeps full control.
pub fn sandbox_launch_config(runtime_dir: impl Into<PathBuf>) -> RuntimeLaunchConfig {
    let runtime_dir = runtime_dir.into();
    let db_path = runtime_dir.join(DB_FILE_NAME);
    RuntimeLaunchConfig {
        app_data_dir: runtime_dir.clone(),
        db_path,
        runtime_dir,
    }
}

/// Resolves the launch configuration from the process environment and the build
/// profile.
///
/// Priority:
/// 1. `CHATSPEED_RUNTIME_DIR` (sandbox; database and app-data inside it);
/// 2. `CHATSPEED_HOME`/runtime (legacy sandbox);
/// 3. the build-profile default (production platform dir, or `dev_data`);
/// then `CHATSPEED_RUNTIME_DB` overrides the database file and moves the
/// application-data directory to the database's parent, and
/// `CHATSPEED_RUNTIME_APP_DATA_DIR` overrides the application-data directory
/// last. Empty values are ignored at every step.
pub fn resolve_launch_config() -> Result<RuntimeLaunchConfig, LaunchConfigError> {
    resolve_launch_config_with_profile(build_profile())
}

/// Resolves the launch configuration for an explicit build profile.
///
/// Split from [`resolve_launch_config`] so a test can execute the release branch
/// without depending on how this binary was compiled.
fn resolve_launch_config_with_profile(
    profile: BuildProfile,
) -> Result<RuntimeLaunchConfig, LaunchConfigError> {
    let runtime_dir_env = non_empty_env(RUNTIME_DIR_ENV).map(PathBuf::from);
    let db_env = non_empty_env(RUNTIME_DB_ENV).map(PathBuf::from);
    let app_data_env = non_empty_env(RUNTIME_APP_DATA_DIR_ENV).map(PathBuf::from);
    let home_env = non_empty_env(LEGACY_HOME_ENV).map(PathBuf::from);

    let (runtime_dir, mut app_data_dir, mut db_path) = if let Some(dir) = runtime_dir_env {
        let db_path = dir.join(DB_FILE_NAME);
        (dir.clone(), dir, db_path)
    } else if let Some(home) = home_env {
        let runtime_dir = home.join(RUNTIME_DIR_NAME);
        let db_path = runtime_dir.join(DB_FILE_NAME);
        (runtime_dir.clone(), runtime_dir, db_path)
    } else {
        let app_data_dir = default_app_data_dir(profile)?;
        let runtime_dir = app_data_dir.join(RUNTIME_DIR_NAME);
        let db_path = app_data_dir.join(DB_FILE_NAME);
        (runtime_dir, app_data_dir, db_path)
    };

    if let Some(db_path_override) = db_env {
        app_data_dir = db_path_override
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .ok_or_else(|| LaunchConfigError::DatabasePathHasNoParent(db_path_override.clone()))?;
        db_path = db_path_override;
    }
    if let Some(app_data_override) = app_data_env {
        app_data_dir = app_data_override;
    }

    Ok(RuntimeLaunchConfig {
        runtime_dir,
        db_path,
        app_data_dir,
    })
}

/// The build-profile default application-data directory, reading the platform
/// data directory from the process environment.
fn default_app_data_dir(profile: BuildProfile) -> Result<PathBuf, LaunchConfigError> {
    profile_app_data_dir(profile, dirs::data_dir().as_deref())
}

/// Application-data directory for an explicit profile and platform directory.
///
/// Pure in its inputs: the platform data directory is injected rather than read
/// from the process environment, so a test can execute the release branch
/// without reading or writing the real per-user data directory. Each profile
/// fails closed when it is missing the directory it needs instead of guessing a
/// `.`-relative path.
fn profile_app_data_dir(
    profile: BuildProfile,
    platform_data_dir: Option<&Path>,
) -> Result<PathBuf, LaunchConfigError> {
    match profile {
        BuildProfile::Debug => dev_app_data_dir().ok_or(LaunchConfigError::ProjectRootUnavailable),
        BuildProfile::Release => platform_data_dir
            .map(production_app_data_dir)
            .ok_or(LaunchConfigError::PlatformDataDirUnavailable),
    }
}

/// Reads an environment variable, treating an empty value as unset.
fn non_empty_env(key: &str) -> Option<OsString> {
    match std::env::var_os(key) {
        Some(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clears every path-related environment key and restores it on drop.
    struct EnvGuard {
        saved: Vec<(&'static str, Option<OsString>)>,
    }

    impl EnvGuard {
        fn clear() -> Self {
            let keys = [
                RUNTIME_DIR_ENV,
                RUNTIME_DB_ENV,
                RUNTIME_APP_DATA_DIR_ENV,
                LEGACY_HOME_ENV,
                "HOME",
                "USERPROFILE",
                "XDG_DATA_HOME",
                // The Windows platform test sets this; save it so the override
                // cannot leak into another test after the guard drops.
                "APPDATA",
            ];
            let saved = keys
                .iter()
                .map(|key| (*key, std::env::var_os(key)))
                .collect();
            for key in keys {
                std::env::remove_var(key);
            }
            Self { saved }
        }

        fn set(&self, key: &str, value: impl AsRef<OsStr>) {
            std::env::set_var(key, value);
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in &self.saved {
                restore_env(key, value.clone());
            }
        }
    }

    fn restore_env(key: &str, value: Option<OsString>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn production_component_matches_tauri_app_data_dir() {
        // Tauri v2's `app_data_dir()` is `dirs::data_dir().join(identifier)`.
        assert_eq!(
            production_app_data_dir(Path::new("/platform/data")),
            PathBuf::from("/platform/data/ai.aidyou.chatspeed")
        );
        assert_ne!(PRODUCTION_APP_IDENTIFIER, "");
    }

    #[test]
    fn dev_app_data_dir_is_repository_local_and_never_the_platform_root() {
        let dev = dev_app_data_dir().expect("a workspace crate resolves the repository root");
        assert_eq!(dev.file_name(), Some(OsStr::new(DEV_DATA_DIR_NAME)));
        assert_eq!(dev.parent().map(Path::to_path_buf), {
            let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
            manifest
                .ancestors()
                .find(|dir| dir.file_name() == Some(OsStr::new("src-tauri")))
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        });

        // The release rule maps a platform root to a child directory, so it
        // never equals the platform root itself; pointing the platform data
        // directory at the repository root still yields a distinct child.
        let release_from_dev =
            profile_app_data_dir(BuildProfile::Release, Some(&dev)).expect("release dir");
        assert_eq!(release_from_dev, dev.join(PRODUCTION_APP_IDENTIFIER));
        assert_ne!(release_from_dev, dev);
    }

    #[test]
    fn profile_app_data_dir_release_derives_from_the_injected_platform_root() {
        // The release branch runs against an injected platform directory, so the
        // real per-user data directory is never read or written.
        let dir = profile_app_data_dir(BuildProfile::Release, Some(Path::new("/platform/data")))
            .expect("release dir");
        assert_eq!(
            dir,
            Path::new("/platform/data").join(PRODUCTION_APP_IDENTIFIER)
        );
    }

    #[test]
    fn profile_app_data_dir_release_fails_closed_without_a_platform_root() {
        // A missing platform data directory is a hard failure, never a
        // `.`-relative guess.
        let error =
            profile_app_data_dir(BuildProfile::Release, None).expect_err("must fail closed");
        assert!(matches!(
            error,
            LaunchConfigError::PlatformDataDirUnavailable
        ));
    }

    #[test]
    fn profile_app_data_dir_debug_uses_the_repository_root_and_ignores_the_platform_root() {
        let dev = dev_app_data_dir().expect("dev root");
        assert_eq!(
            profile_app_data_dir(BuildProfile::Debug, None).expect("debug dir"),
            dev
        );
        assert_eq!(
            profile_app_data_dir(BuildProfile::Debug, Some(Path::new("/platform/data")))
                .expect("debug dir"),
            dev
        );
    }

    #[test]
    fn profiles_derive_different_defaults_from_the_same_platform_root() {
        let platform = Path::new("/platform/data");
        let debug = profile_app_data_dir(BuildProfile::Debug, Some(platform)).expect("debug dir");
        let release =
            profile_app_data_dir(BuildProfile::Release, Some(platform)).expect("release dir");
        // Debug ignores the platform root while release derives from it, and the
        // leaf names differ (`dev_data` versus the identifier), so the profile
        // defaults land in different directories unless a caller explicitly
        // overrides the application-data directory to the same path for both.
        assert_eq!(debug, dev_app_data_dir().expect("dev root"));
        assert_eq!(release, platform.join(PRODUCTION_APP_IDENTIFIER));
        assert_ne!(debug, release);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn release_profile_resolves_inside_the_configured_platform_root() {
        let _guard = crate::ENV_LOCK.lock().expect("env lock");
        let env = EnvGuard::clear();
        let sandbox = tempfile::tempdir().expect("tempdir");
        // Execute the release branch end to end, anchored at a temporary platform
        // root instead of the real per-user data directory.
        env.set("XDG_DATA_HOME", sandbox.path());

        let config = resolve_launch_config_with_profile(BuildProfile::Release).expect("resolve");
        let root = production_app_data_dir(sandbox.path());
        assert_eq!(config.app_data_dir(), root);
        assert_eq!(config.runtime_dir(), root.join(RUNTIME_DIR_NAME));
        assert_eq!(config.db_path(), root.join(DB_FILE_NAME));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_platform_dir_follows_xdg_then_home() {
        let _guard = crate::ENV_LOCK.lock().expect("env lock");
        let env = EnvGuard::clear();

        env.set("XDG_DATA_HOME", "/xdg-data");
        env.set("HOME", "/home/user");
        assert_eq!(
            dirs::data_dir().expect("xdg data dir"),
            PathBuf::from("/xdg-data")
        );
        assert_eq!(
            production_app_data_dir(&dirs::data_dir().expect("xdg data dir")),
            PathBuf::from("/xdg-data/ai.aidyou.chatspeed")
        );

        // An empty XDG_DATA_HOME is ignored, falling back to $HOME/.local/share.
        std::env::remove_var("XDG_DATA_HOME");
        assert_eq!(
            dirs::data_dir().expect("home data dir"),
            PathBuf::from("/home/user/.local/share")
        );
        assert_eq!(
            production_app_data_dir(&dirs::data_dir().expect("home data dir")),
            PathBuf::from("/home/user/.local/share/ai.aidyou.chatspeed")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_platform_dir_is_application_support() {
        let _guard = crate::ENV_LOCK.lock().expect("env lock");
        let env = EnvGuard::clear();
        env.set("HOME", "/Users/user");
        assert_eq!(
            dirs::data_dir().expect("mac data dir"),
            PathBuf::from("/Users/user/Library/Application Support")
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_platform_dir_is_roaming_app_data() {
        let _guard = crate::ENV_LOCK.lock().expect("env lock");
        let env = EnvGuard::clear();
        env.set("APPDATA", r"C:\Users\user\AppData\Roaming");
        assert_eq!(
            dirs::data_dir().expect("windows data dir"),
            PathBuf::from(r"C:\Users\user\AppData\Roaming")
        );
    }

    #[test]
    fn explicit_runtime_dir_is_an_isolated_sandbox() {
        let config = sandbox_launch_config("/sandbox/runtime");
        assert_eq!(config.runtime_dir(), Path::new("/sandbox/runtime"));
        assert_eq!(config.db_path(), Path::new("/sandbox/runtime/chatspeed.db"));
        assert_eq!(config.app_data_dir(), Path::new("/sandbox/runtime"));
        assert_eq!(
            config.discovery_file(),
            Path::new("/sandbox/runtime").join(DISCOVERY_FILE_NAME)
        );
    }

    #[test]
    fn runtime_dir_env_wins_and_holds_db_and_app_data_inside_it() {
        let _guard = crate::ENV_LOCK.lock().expect("env lock");
        let env = EnvGuard::clear();
        env.set(RUNTIME_DIR_ENV, "/sandbox/env-runtime");
        env.set(LEGACY_HOME_ENV, "/legacy/home");

        let config = resolve_launch_config().expect("resolve");
        assert_eq!(config.runtime_dir(), Path::new("/sandbox/env-runtime"));
        assert_eq!(
            config.db_path(),
            Path::new("/sandbox/env-runtime/chatspeed.db")
        );
        assert_eq!(config.app_data_dir(), Path::new("/sandbox/env-runtime"));
    }

    #[test]
    fn legacy_home_uses_the_home_runtime_layout() {
        let _guard = crate::ENV_LOCK.lock().expect("env lock");
        let env = EnvGuard::clear();
        env.set(LEGACY_HOME_ENV, "/legacy/home");

        let config = resolve_launch_config().expect("resolve");
        assert_eq!(config.runtime_dir(), Path::new("/legacy/home/runtime"));
        assert_eq!(
            config.db_path(),
            Path::new("/legacy/home/runtime/chatspeed.db")
        );
        assert_eq!(config.app_data_dir(), Path::new("/legacy/home/runtime"));
    }

    #[test]
    fn empty_overrides_are_ignored() {
        let _guard = crate::ENV_LOCK.lock().expect("env lock");
        let env = EnvGuard::clear();
        env.set(RUNTIME_DIR_ENV, "");
        env.set(RUNTIME_DB_ENV, "");
        env.set(RUNTIME_APP_DATA_DIR_ENV, "");
        env.set(LEGACY_HOME_ENV, "");

        // With every override empty the profile default is used, never an empty
        // or `.` path.
        let config = resolve_launch_config().expect("resolve");
        let expected_root = match build_profile() {
            BuildProfile::Debug => dev_app_data_dir().expect("dev root"),
            BuildProfile::Release => {
                production_app_data_dir(&dirs::data_dir().expect("platform data dir"))
            }
        };
        assert_eq!(config.app_data_dir(), expected_root);
        assert_eq!(config.runtime_dir(), expected_root.join(RUNTIME_DIR_NAME));
        assert_eq!(config.db_path(), expected_root.join(DB_FILE_NAME));
        assert!(!config.runtime_dir().as_os_str().is_empty());
    }

    #[test]
    fn runtime_db_override_reanchors_app_data_to_the_database_parent() {
        let _guard = crate::ENV_LOCK.lock().expect("env lock");
        let env = EnvGuard::clear();
        env.set(RUNTIME_DIR_ENV, "/sandbox/runtime");
        env.set(RUNTIME_DB_ENV, "/other/profile/chatspeed.db");

        let config = resolve_launch_config().expect("resolve");
        // The runtime directory keeps owning the lock/discovery ...
        assert_eq!(config.runtime_dir(), Path::new("/sandbox/runtime"));
        // ... while the database and app-data follow the override's parent so a
        // redirected database never pulls resources from another profile.
        assert_eq!(config.db_path(), Path::new("/other/profile/chatspeed.db"));
        assert_eq!(config.app_data_dir(), Path::new("/other/profile"));
    }

    #[test]
    fn app_data_env_override_wins_last() {
        let _guard = crate::ENV_LOCK.lock().expect("env lock");
        let env = EnvGuard::clear();
        env.set(RUNTIME_DIR_ENV, "/sandbox/runtime");
        env.set(RUNTIME_DB_ENV, "/other/profile/chatspeed.db");
        env.set(RUNTIME_APP_DATA_DIR_ENV, "/explicit/app-data");

        let config = resolve_launch_config().expect("resolve");
        assert_eq!(config.db_path(), Path::new("/other/profile/chatspeed.db"));
        assert_eq!(config.app_data_dir(), Path::new("/explicit/app-data"));
    }

    #[test]
    fn profile_default_uses_the_compiled_profile_branch() {
        let _guard = crate::ENV_LOCK.lock().expect("env lock");
        let _env = EnvGuard::clear();

        // This build only ever executes its own compiled branch. Both branches
        // are covered by the pure `profile_app_data_dir` tests, and the release
        // branch is executed end to end under a temporary platform root, so the
        // real release default is never resolved here.
        let config = resolve_launch_config().expect("resolve");
        let expected = default_app_data_dir(build_profile()).expect("profile default");
        assert_eq!(config.app_data_dir(), expected);
        assert_eq!(config.runtime_dir(), expected.join(RUNTIME_DIR_NAME));
        assert_eq!(config.db_path(), expected.join(DB_FILE_NAME));
    }
}
