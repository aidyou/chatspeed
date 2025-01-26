use crate::{plugins::runtime::RuntimeError, SHARED_DATA_DIR};
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use url::Url;

/// Plugin permission configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPermissions {
    /// Network access permissions
    #[serde(default)]
    pub network: NetworkPermissions,
    /// File system permissions
    #[serde(default)]
    pub fs: FileSystemPermissions,
}

/// Network access permission configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPermissions {
    /// List of allowed domains
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// Whether network access is enabled
    #[serde(default)]
    pub enabled: bool,
}

/// File system permission configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FileSystemPermissions {
    /// List of allowed relative paths (relative to plugin directory)
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    /// Whether access to shared data directory is allowed
    #[serde(default)]
    pub allow_shared: bool,
}

impl PluginPermissions {
    /// Load permissions from manifest.json
    pub fn from_manifest(plugin_dir: &str) -> Result<Self, anyhow::Error> {
        let manifest_path = Path::new(plugin_dir).join("manifest.json");
        let manifest_content = std::fs::read_to_string(manifest_path)?;
        log::debug!("Manifest content: {}", manifest_content);

        // 先解析成 Value 来获取 permissions 字段
        let manifest: serde_json::Value = serde_json::from_str(&manifest_content)?;
        if let Some(permissions) = manifest.get("permissions") {
            serde_json::from_value(permissions.clone()).map_err(Into::into)
        } else {
            // 如果没有 permissions 字段，使用默认值
            Ok(Self::default())
        }
    }
}

impl NetworkPermissions {
    /// Check if URL is allowed to access
    pub fn check_url(&self, url: &str) -> bool {
        if !self.enabled {
            return false;
        }

        if let Ok(url) = Url::parse(url) {
            if let Some(host) = url.host_str() {
                return self.allowed_domains.contains(&host.to_string());
            }
        }
        false
    }
}

impl FileSystemPermissions {
    /// Resolves a path to its cleaned absolute form without accessing the filesystem.
    ///
    /// This function deliberately avoids using `canonicalize` because:
    /// 1. `canonicalize` requires filesystem access, which is not suitable for permission checking
    /// 2. `canonicalize` fails if the path doesn't exist, but we need to check permissions before file operations
    /// 3. For security checks, we want to validate the path structure itself, not its actual filesystem state
    ///
    /// Instead, this function:
    /// 1. Handles path components logically without filesystem access
    /// 2. Resolves . and .. components
    /// 3. Normalizes path separators
    /// 4. Preserves the ability to check permissions on non-existent paths
    fn resolve_clean_path(base: &Path, path: &Path) -> Option<PathBuf> {
        // Always process components to handle .. and . regardless of whether path is absolute
        let mut components = Vec::new();
        for component in path.components() {
            match component {
                std::path::Component::Prefix(prefix) => {
                    components.push(PathBuf::from(prefix.as_os_str()));
                }
                std::path::Component::RootDir => {
                    components.clear(); // Start fresh for absolute path
                    components.push(PathBuf::from("/"));
                }
                std::path::Component::CurDir => {
                    // Skip . components
                }
                std::path::Component::ParentDir => {
                    // Pop the last non-empty component for ..
                    if !components.is_empty() {
                        if let Some(last) = components.last() {
                            if !last.as_os_str().is_empty() {
                                components.pop();
                            }
                        }
                    }
                }
                std::path::Component::Normal(name) => {
                    components.push(PathBuf::from(name));
                }
            }
        }

        // If path was relative, prepend base components
        if !path.is_absolute() {
            let mut base_components = Vec::new();
            for component in base.components() {
                match component {
                    std::path::Component::Prefix(prefix) => {
                        base_components.push(PathBuf::from(prefix.as_os_str()));
                    }
                    std::path::Component::RootDir => {
                        base_components.clear();
                        base_components.push(PathBuf::from("/"));
                    }
                    std::path::Component::Normal(name) => {
                        base_components.push(PathBuf::from(name));
                    }
                    _ => {} // Skip . and .. in base path as it should be clean
                }
            }
            components = [base_components, components].concat();
        }

        // Combine all components
        let mut result = PathBuf::new();
        for component in components {
            result.push(component);
        }

        Some(result)
    }

    /// Check if path is allowed to access
    pub fn check_path(&self, base_dir: &str, path: &Path, is_shared: bool) -> bool {
        // First check if plugin directory exists
        if !Path::new(base_dir).exists() {
            return false;
        }

        // Clean plugin directory path without following symlinks
        let plugin_dir = match Self::resolve_clean_path(Path::new("/"), Path::new(&base_dir)) {
            Some(p) => p,
            None => return false,
        };

        // Resolve the target path without accessing the filesystem
        let target_path = match Self::resolve_clean_path(&plugin_dir, path) {
            Some(p) => p,
            None => return false,
        };

        log::debug!(
            "Checking path access: target={}, base={}, is_shared={}, allow_shared={}",
            target_path.display(),
            base_dir,
            is_shared,
            self.allow_shared
        );

        // 如果是访问共享目录
        if is_shared {
            // 必须有共享目录权限
            if !self.allow_shared {
                log::debug!("Shared directory access denied: no permission");
                return false;
            }

            // 检查是否在共享目录内
            let shared_dir = &*SHARED_DATA_DIR.read();
            if let Some(shared_dir) =
                Self::resolve_clean_path(Path::new("/"), Path::new(&*shared_dir))
            {
                if target_path.starts_with(&shared_dir) {
                    log::debug!("Path is within shared directory, access allowed");
                    return true;
                }
            }
            log::debug!("Path is not within shared directory, access denied");
            return false;
        }

        // 非共享目录访问：检查是否在插件目录内
        if target_path.starts_with(&plugin_dir) {
            log::debug!("Path is within plugin directory, access allowed");
            return true;
        }

        log::debug!("Path is not within plugin directory, access denied");
        false
    }

    /// Resolve and validate path
    pub fn resolve_path(
        &self,
        base_dir: &str,
        path: &str,
        is_shared: bool,
    ) -> Result<PathBuf, RuntimeError> {
        let path = Path::new(path);

        let resolved_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            PathBuf::from(base_dir).join(path)
        };

        // 检查路径是否在允许的范围内
        if self.check_path(base_dir, &resolved_path, is_shared) {
            Self::resolve_clean_path(Path::new("/"), &resolved_path).ok_or_else(|| {
                RuntimeError::FileError(
                    t!(
                        "plugin.runtime.invalid_path",
                        path = resolved_path.display()
                    )
                    .to_string(),
                )
            })
        } else {
            Err(RuntimeError::PermissionError(
                t!(
                    "plugin.runtime.access_denied_to_path",
                    path = resolved_path.display()
                )
                .to_string(),
            ))
        }
    }
}

/// Default implementations
impl Default for PluginPermissions {
    fn default() -> Self {
        Self {
            network: NetworkPermissions::default(),
            fs: FileSystemPermissions::default(),
        }
    }
}

impl Default for NetworkPermissions {
    fn default() -> Self {
        Self {
            allowed_domains: Vec::new(),
            enabled: false,
        }
    }
}

impl Default for FileSystemPermissions {
    fn default() -> Self {
        Self {
            allowed_paths: Vec::new(), // No additional paths allowed by default
            allow_shared: false,       // Shared directory access disabled by default
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_file_permissions() {
        let temp_dir = TempDir::new().unwrap();
        let plugin_dir = temp_dir.path().join("plugin1");
        std::fs::create_dir(&plugin_dir).unwrap();

        let shared_dir = temp_dir.path().join("shared");
        std::fs::create_dir(&shared_dir).unwrap();

        // Set up SHARED_DATA_DIR for testing
        {
            let mut shared_dir_path = SHARED_DATA_DIR.write();
            *shared_dir_path = shared_dir.to_string_lossy().to_string();
        }

        // Create a test file in plugin directory
        let plugin_file = plugin_dir.join("config.json");
        std::fs::write(&plugin_file, "{}").unwrap();

        // Create a test file in shared directory
        let shared_file = shared_dir.join("data.json");
        std::fs::write(&shared_file, "{}").unwrap();

        // Create a test file outside both directories
        let other_file = temp_dir.path().join("other.json");
        std::fs::write(&other_file, "{}").unwrap();

        let permissions = FileSystemPermissions {
            allow_shared: true,
            allowed_paths: vec![],
        };

        // Test plugin directory access
        assert!(permissions.check_path(plugin_dir.to_str().unwrap(), &plugin_file, false));

        // Test shared directory access
        assert!(permissions.check_path(plugin_dir.to_str().unwrap(), &shared_file, true));

        // Test parent directory access (should fail)
        assert!(!permissions.check_path(plugin_dir.to_str().unwrap(), &other_file, false));

        // Test non-existent file in plugin directory (should fail)
        assert!(!permissions.check_path(
            plugin_dir.to_str().unwrap(),
            &plugin_dir.join("non_existent.json"),
            false
        ));

        // Test path traversal attempt (should fail)
        assert!(!permissions.check_path(
            plugin_dir.to_str().unwrap(),
            &PathBuf::from("../other.json"),
            false
        ));

        // Test path traversal attempt，the path not exists, so should fail
        let check_result = permissions.check_path(
            plugin_dir.to_str().unwrap(),
            &PathBuf::from("a/b/../other.json"),
            false,
        );
        assert!(!check_result);

        let plugin_path = plugin_dir.join("a/b/../c");
        fs::create_dir_all(&plugin_path).unwrap();
        assert!(permissions.check_path(plugin_dir.to_str().unwrap(), &plugin_path, false));
        fs::remove_dir_all(&plugin_path).unwrap();

        // Clean up SHARED_DATA_DIR
        {
            let mut shared_dir_path = SHARED_DATA_DIR.write();
            *shared_dir_path = String::new();
        }
    }

    #[test]
    fn test_network_permissions() {
        let permissions = NetworkPermissions {
            enabled: true,
            allowed_domains: vec!["api.example.com".to_string()],
        };

        // Test allowed domain
        assert!(permissions.check_url("https://api.example.com/data"));

        // Test subdomain (should fail)
        assert!(!permissions.check_url("https://sub.api.example.com/data"));

        // Test different domain (should fail)
        assert!(!permissions.check_url("https://other.com/data"));

        // Test disabled network access
        let disabled = NetworkPermissions {
            enabled: false,
            allowed_domains: vec!["api.example.com".to_string()],
        };
        assert!(!disabled.check_url("https://api.example.com/data"));
    }
}
