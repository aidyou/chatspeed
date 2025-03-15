use serde::Deserialize;
use std::env::{self};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Deserialize)]
pub struct PythonConfig {
    pub executable: PathBuf,
    pub home: PathBuf,
    pub library_path: PathBuf,
    pub include_path: PathBuf,
    pub venv_path: PathBuf,
    pub pip_index_url: String,
    pub pip_timeout: u32,
    pub packages_file: PathBuf,
    pub is_available: bool,
}

#[derive(Debug)]
pub struct PythonPaths {
    pub executable: PathBuf,
    pub home: PathBuf,
    pub library_path: PathBuf,
    pub include_path: PathBuf,
}

impl PythonConfig {
    fn find_python_executable() -> Option<PathBuf> {
        // 首先检查环境变量
        if let Ok(path) = env::var("PYTHON_EXECUTABLE") {
            let path = PathBuf::from(path);
            if path.exists() {
                return Some(path);
            }
        }

        // 然后尝试从 PATH 中查找
        let output = if cfg!(target_os = "windows") {
            Command::new("where")
                .arg("python3.12")
                .output()
                .or_else(|_| Command::new("where").arg("python3").output())
                .or_else(|_| Command::new("where").arg("python").output())
        } else {
            Command::new("which")
                .arg("python3.12")
                .output()
                .or_else(|_| Command::new("which").arg("python3").output())
                .or_else(|_| Command::new("which").arg("python").output())
        };

        if let Ok(output) = output {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout);
                let path = path.trim();
                let path = PathBuf::from(path);
                if path.exists() {
                    return Some(path);
                }
            }
        }

        None
    }

    fn find_python_home(executable: &PathBuf) -> Option<PathBuf> {
        // 首先检查环境变量
        if let Ok(path) = env::var("PYTHON_HOME") {
            let path = PathBuf::from(path);
            if path.exists() {
                return Some(path);
            }
        }

        // 使用 Python 命令获取 sys.prefix
        let output = Command::new(executable)
            .args(["-c", "import sys; print(sys.prefix)"])
            .output()
            .ok()?;

        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout);
            let path = path.trim();
            let path = PathBuf::from(path);
            if path.exists() {
                return Some(path);
            }
        }

        None
    }

    fn find_python_library(home: &PathBuf) -> Option<PathBuf> {
        // 首先检查环境变量
        if let Ok(path) = env::var("PYTHON_LIBRARY") {
            let path = PathBuf::from(path);
            if path.exists() {
                return Some(path);
            }
        }

        // 根据不同平台查找库文件
        let library_paths = if cfg!(target_os = "windows") {
            vec![
                home.join("libs").join("python3.dll"),
                home.join("libs").join("python312.dll"),
            ]
        } else if cfg!(target_os = "macos") {
            vec![
                home.join("lib").join("libpython3.12.dylib"),
                home.join("Python"),
                home.join("Frameworks")
                    .join("Python.framework")
                    .join("Versions")
                    .join("3.12")
                    .join("Python"),
            ]
        } else {
            vec![
                home.join("lib").join("libpython3.12.so"),
                home.join("lib").join("libpython3.so"),
            ]
        };

        for path in library_paths {
            if path.exists() {
                return Some(path);
            }
        }

        None
    }

    fn find_python_include(home: &PathBuf) -> Option<PathBuf> {
        // 首先检查环境变量
        if let Ok(path) = env::var("PYTHON_INCLUDE") {
            let path = PathBuf::from(path);
            if path.exists() {
                return Some(path);
            }
        }

        // 使用常见的包含路径
        let include_paths = if cfg!(target_os = "windows") {
            vec![home.join("include")]
        } else if cfg!(target_os = "macos") {
            vec![
                home.join("include").join("python3.12"),
                home.join("Frameworks")
                    .join("Python.framework")
                    .join("Versions")
                    .join("3.12")
                    .join("include")
                    .join("python3.12"),
            ]
        } else {
            vec![home.join("include").join("python3.12")]
        };

        for path in include_paths {
            if path.exists() {
                return Some(path);
            }
        }

        None
    }

    pub fn detect_paths() -> Option<PythonPaths> {
        let executable = Self::find_python_executable()?;
        let home = Self::find_python_home(&executable)?;
        let library_path = Self::find_python_library(&home)?;
        let include_path = Self::find_python_include(&home)?;

        Some(PythonPaths {
            executable,
            home,
            library_path,
            include_path,
        })
    }

    pub fn from_env() -> Self {
        let paths = Self::detect_paths().unwrap_or_else(|| PythonPaths {
            executable: PathBuf::from("python3"),
            home: PathBuf::from("/usr"),
            library_path: PathBuf::from("/usr/lib"),
            include_path: PathBuf::from("/usr/include"),
        });

        let venv_path = PathBuf::from(".venv");
        let packages_file = PathBuf::from("requirements.txt");

        Self {
            executable: paths.executable,
            home: paths.home,
            library_path: paths.library_path,
            include_path: paths.include_path,
            venv_path,
            pip_index_url: String::from("https://pypi.org/simple"),
            pip_timeout: 30,
            packages_file,
            is_available: true,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.executable.exists() {
            return Err(format!(
                "Python executable not found at: {}",
                self.executable.display()
            ));
        }

        if !self.library_path.exists() {
            return Err(format!(
                "Python library not found at: {}",
                self.library_path.display()
            ));
        }

        if !self.include_path.exists() {
            return Err(format!(
                "Python headers not found at: {}",
                self.include_path.display()
            ));
        }

        Ok(())
    }

    pub fn is_python_available(&self) -> bool {
        self.is_available
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_paths() {
        let paths = PythonConfig::detect_paths();
        assert!(paths.is_some());
    }

    #[test]
    fn test_from_env() {
        let config = PythonConfig::from_env();
        assert!(config.validate().is_ok());
    }
}
