use super::types::{
    SandboxAvailabilityState, SandboxDetectorOptions, SandboxRuntime, SandboxRuntimeStatus,
    SandboxRuntimeStatusSummary,
};
use serde_json::Value;
use std::io::ErrorKind;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct SandboxRuntimeDetector {
    options: SandboxDetectorOptions,
}

impl SandboxRuntimeDetector {
    pub fn new(options: SandboxDetectorOptions) -> Self {
        Self { options }
    }

    pub fn detect(&self) -> SandboxRuntimeStatusSummary {
        SandboxRuntimeStatusSummary {
            msb: self.detect_msb(),
            docker: self.detect_docker(),
        }
    }

    fn detect_msb(&self) -> SandboxRuntimeStatus {
        if !cfg!(target_os = "linux") && !cfg!(target_os = "macos") {
            return status(
                SandboxRuntime::Msb,
                SandboxAvailabilityState::UnsupportedPlatform,
                None,
                None,
                "unsupported_platform",
                "Microsandbox is only supported on Unix-like platforms in this adapter",
                Vec::new(),
                self.options.required_images.clone(),
            );
        }

        let version = match command_output(&self.options.msb_binary, &["--version"], &self.options)
        {
            Ok(output) => parse_version_text(&output),
            Err(error) => {
                let (state, reason_code, executable) = match &error {
                    CommandProbeError::NotFound(_) => (
                        SandboxAvailabilityState::NotInstalled,
                        "not_installed",
                        None,
                    ),
                    CommandProbeError::Unhealthy(_) => (
                        SandboxAvailabilityState::InstalledButUnhealthy,
                        "unhealthy",
                        Some(self.options.msb_binary.clone()),
                    ),
                };
                return status(
                    SandboxRuntime::Msb,
                    state,
                    executable,
                    None,
                    reason_code,
                    error.reason(),
                    Vec::new(),
                    self.options.required_images.clone(),
                );
            }
        };
        if !supported_msb_version(version.as_deref()) {
            return status(
                SandboxRuntime::Msb,
                SandboxAvailabilityState::UnsupportedVersion,
                Some(self.options.msb_binary.clone()),
                version,
                "unsupported_version",
                "Microsandbox version is outside the supported range >=0.6.6,<0.7.0",
                Vec::new(),
                self.options.required_images.clone(),
            );
        }

        if let Err(reason) = command_output(&self.options.msb_binary, &["doctor"], &self.options) {
            return status(
                SandboxRuntime::Msb,
                SandboxAvailabilityState::InstalledButUnhealthy,
                Some(self.options.msb_binary.clone()),
                version,
                "unhealthy",
                reason.reason(),
                Vec::new(),
                self.options.required_images.clone(),
            );
        }

        let images = match command_output(
            &self.options.msb_binary,
            &["image", "list", "--format", "json"],
            &self.options,
        ) {
            Ok(output) => parse_image_list(&output),
            Err(reason) => {
                return status(
                    SandboxRuntime::Msb,
                    SandboxAvailabilityState::InstalledButUnhealthy,
                    Some(self.options.msb_binary.clone()),
                    version,
                    "unhealthy",
                    reason.reason(),
                    Vec::new(),
                    self.options.required_images.clone(),
                )
            }
        };
        let missing = missing_images(&self.options.required_images, &images);
        let state = if missing.is_empty() {
            SandboxAvailabilityState::Ready
        } else {
            SandboxAvailabilityState::ReadyMissingImage
        };

        status(
            SandboxRuntime::Msb,
            state,
            Some(self.options.msb_binary.clone()),
            version,
            if missing.is_empty() {
                "ready"
            } else {
                "ready_missing_image"
            },
            if missing.is_empty() {
                "Microsandbox is ready"
            } else {
                "Microsandbox is ready but one or more configured images are missing"
            },
            images,
            missing,
        )
    }

    fn detect_docker(&self) -> SandboxRuntimeStatus {
        let version =
            match command_output(&self.options.docker_binary, &["--version"], &self.options) {
                Ok(output) => parse_version_text(&output),
                Err(error) => {
                    let (state, reason_code, executable) = match &error {
                        CommandProbeError::NotFound(_) => (
                            SandboxAvailabilityState::NotInstalled,
                            "not_installed",
                            None,
                        ),
                        CommandProbeError::Unhealthy(_) => (
                            SandboxAvailabilityState::InstalledButUnhealthy,
                            "unhealthy",
                            Some(self.options.docker_binary.clone()),
                        ),
                    };
                    return status(
                        SandboxRuntime::Docker,
                        state,
                        executable,
                        None,
                        reason_code,
                        error.reason(),
                        Vec::new(),
                        self.options.required_images.clone(),
                    );
                }
            };
        if !supported_docker_version(version.as_deref()) {
            return status(
                SandboxRuntime::Docker,
                SandboxAvailabilityState::UnsupportedVersion,
                Some(self.options.docker_binary.clone()),
                version,
                "unsupported_version",
                "Docker version is outside the supported range >=20.10.0",
                Vec::new(),
                self.options.required_images.clone(),
            );
        }

        if let Err(reason) = command_output(
            &self.options.docker_binary,
            &["info", "--format", "{{json .}}"],
            &self.options,
        ) {
            return status(
                SandboxRuntime::Docker,
                SandboxAvailabilityState::InstalledButUnhealthy,
                Some(self.options.docker_binary.clone()),
                version,
                "unhealthy",
                reason.reason(),
                Vec::new(),
                self.options.required_images.clone(),
            );
        }

        let images = match command_output(
            &self.options.docker_binary,
            &["image", "ls", "--format", "{{json .}}"],
            &self.options,
        ) {
            Ok(output) => parse_docker_image_lines(&output),
            Err(reason) => {
                return status(
                    SandboxRuntime::Docker,
                    SandboxAvailabilityState::InstalledButUnhealthy,
                    Some(self.options.docker_binary.clone()),
                    version,
                    "unhealthy",
                    reason.reason(),
                    Vec::new(),
                    self.options.required_images.clone(),
                )
            }
        };
        let missing = missing_images(&self.options.required_images, &images);
        let state = if missing.is_empty() {
            SandboxAvailabilityState::Ready
        } else {
            SandboxAvailabilityState::ReadyMissingImage
        };

        status(
            SandboxRuntime::Docker,
            state,
            Some(self.options.docker_binary.clone()),
            version,
            if missing.is_empty() {
                "ready"
            } else {
                "ready_missing_image"
            },
            if missing.is_empty() {
                "Docker is ready"
            } else {
                "Docker is ready but one or more configured images are missing"
            },
            images,
            missing,
        )
    }
}

enum CommandProbeError {
    NotFound(String),
    Unhealthy(String),
}

impl CommandProbeError {
    fn reason(&self) -> &str {
        match self {
            CommandProbeError::NotFound(reason) | CommandProbeError::Unhealthy(reason) => reason,
        }
    }
}

fn command_output(
    binary: &str,
    args: &[&str],
    options: &SandboxDetectorOptions,
) -> Result<String, CommandProbeError> {
    let mut child = Command::new(binary)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                CommandProbeError::NotFound(error.to_string())
            } else {
                CommandProbeError::Unhealthy(error.to_string())
            }
        })?;

    let started = std::time::Instant::now();
    loop {
        if let Some(_status) = child
            .try_wait()
            .map_err(|error| CommandProbeError::Unhealthy(error.to_string()))?
        {
            let output = child
                .wait_with_output()
                .map_err(|error| CommandProbeError::Unhealthy(error.to_string()))?;
            if output.status.success() {
                return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
            }
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(CommandProbeError::Unhealthy(if stderr.is_empty() {
                format!("{binary} exited with status {}", output.status)
            } else {
                stderr
            }));
        }
        if started.elapsed() >= options.timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(CommandProbeError::Unhealthy(format!(
                "{binary} timed out after {} ms",
                options.timeout.as_millis()
            )));
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

fn status(
    runtime: SandboxRuntime,
    state: SandboxAvailabilityState,
    executable: Option<String>,
    version: Option<String>,
    reason_code: &str,
    reason: &str,
    images: Vec<String>,
    missing_images: Vec<String>,
) -> SandboxRuntimeStatus {
    SandboxRuntimeStatus {
        runtime,
        state,
        executable,
        version,
        reason_code: Some(reason_code.to_string()),
        reason: Some(reason.to_string()),
        images,
        missing_images,
        checked_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_millis() as i64),
    }
}

pub fn parse_version_text(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|part| part.chars().any(|ch| ch.is_ascii_digit()))
        .map(|part| part.trim_start_matches('v').trim_matches(',').to_string())
}

pub fn parse_image_list(output: &str) -> Vec<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return images_from_json_value(&value);
    }

    trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn supported_msb_version(version: Option<&str>) -> bool {
    let Some(version) = version.and_then(|value| semver::Version::parse(value).ok()) else {
        return false;
    };
    version >= semver::Version::new(0, 6, 6) && version < semver::Version::new(0, 7, 0)
}

fn supported_docker_version(version: Option<&str>) -> bool {
    let Some(version) = version.and_then(|value| semver::Version::parse(value).ok()) else {
        return false;
    };
    version >= semver::Version::new(20, 10, 0)
}

pub fn parse_docker_image_lines(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .flat_map(|value| {
            let repository = value
                .get("Repository")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let tag = value.get("Tag").and_then(Value::as_str).unwrap_or_default();
            if repository.is_empty() || repository == "<none>" {
                Vec::new()
            } else if tag.is_empty() || tag == "<none>" {
                vec![repository.to_string()]
            } else {
                vec![format!("{repository}:{tag}")]
            }
        })
        .collect()
}

fn images_from_json_value(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items.iter().flat_map(images_from_json_value).collect(),
        Value::Object(map) => {
            let candidates = ["name", "image", "reference", "repository", "tag"];
            for key in candidates {
                if let Some(text) = map.get(key).and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        return vec![text.to_string()];
                    }
                }
            }
            Vec::new()
        }
        Value::String(text) => vec![text.to_string()],
        _ => Vec::new(),
    }
}

fn missing_images(required: &[String], available: &[String]) -> Vec<String> {
    if available.is_empty() {
        return required.to_vec();
    }
    required
        .iter()
        .filter(|image| !available.iter().any(|available| available == *image))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions_and_image_lists() {
        assert_eq!(parse_version_text("msb 0.6.6"), Some("0.6.6".to_string()));
        assert_eq!(
            parse_version_text("Docker version 27.0.1, build abc"),
            Some("27.0.1".to_string())
        );
        assert_eq!(
            parse_image_list(r#"[{"name":"busybox:latest"},{"image":"python:3.12-alpine"}]"#),
            vec![
                "busybox:latest".to_string(),
                "python:3.12-alpine".to_string()
            ]
        );
        assert_eq!(
            parse_docker_image_lines("{\"Repository\":\"busybox\",\"Tag\":\"latest\"}\n"),
            vec!["busybox:latest".to_string()]
        );
    }

    #[test]
    fn validates_supported_runtime_versions() {
        assert!(supported_msb_version(Some("0.6.6")));
        assert!(supported_msb_version(Some("0.6.9")));
        assert!(!supported_msb_version(Some("0.6.5")));
        assert!(!supported_msb_version(Some("0.7.0")));
        assert!(supported_docker_version(Some("20.10.0")));
        assert!(supported_docker_version(Some("27.0.1")));
        assert!(!supported_docker_version(Some("19.03.0")));
    }

    fn write_fake_runtime(name: &str, body: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(name);
        std::fs::write(&path, body).expect("write fake runtime");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).expect("chmod fake runtime");
        }
        (dir, path.display().to_string())
    }

    #[test]
    fn fake_msb_reports_unhealthy_when_doctor_or_images_fail() {
        let (_doctor_dir, doctor_binary) = write_fake_runtime(
            "msb",
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'msb 0.6.6'; exit 0; fi\nif [ \"$1\" = \"doctor\" ]; then echo 'doctor failed' >&2; exit 2; fi\nexit 0\n",
        );
        let detector = SandboxRuntimeDetector::new(SandboxDetectorOptions {
            msb_binary: doctor_binary,
            docker_binary: "definitely-missing-docker-for-chatspeed-test".to_string(),
            ..SandboxDetectorOptions::default()
        });
        assert_eq!(
            detector.detect().msb.state,
            SandboxAvailabilityState::InstalledButUnhealthy
        );

        let (_images_dir, images_binary) = write_fake_runtime(
            "msb",
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'msb 0.6.6'; exit 0; fi\nif [ \"$1\" = \"doctor\" ]; then exit 0; fi\nif [ \"$1\" = \"image\" ]; then echo 'image list failed' >&2; exit 3; fi\nexit 0\n",
        );
        let detector = SandboxRuntimeDetector::new(SandboxDetectorOptions {
            msb_binary: images_binary,
            docker_binary: "definitely-missing-docker-for-chatspeed-test".to_string(),
            ..SandboxDetectorOptions::default()
        });
        assert_eq!(
            detector.detect().msb.state,
            SandboxAvailabilityState::InstalledButUnhealthy
        );
    }

    #[test]
    fn fake_docker_reports_ready_missing_image_and_unhealthy_image_list() {
        let (_ready_dir, ready_binary) = write_fake_runtime(
            "docker",
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'Docker version 27.0.1, build abc'; exit 0; fi\nif [ \"$1\" = \"info\" ]; then echo '{}'; exit 0; fi\nif [ \"$1\" = \"image\" ]; then echo '{\"Repository\":\"busybox\",\"Tag\":\"latest\"}'; exit 0; fi\nexit 0\n",
        );
        let detector = SandboxRuntimeDetector::new(SandboxDetectorOptions {
            msb_binary: "definitely-missing-msb-for-chatspeed-test".to_string(),
            docker_binary: ready_binary,
            required_images: vec!["python:3.12-slim".to_string()],
            ..SandboxDetectorOptions::default()
        });
        assert_eq!(
            detector.detect().docker.state,
            SandboxAvailabilityState::ReadyMissingImage
        );

        let (_bad_dir, bad_binary) = write_fake_runtime(
            "docker",
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'Docker version 27.0.1, build abc'; exit 0; fi\nif [ \"$1\" = \"info\" ]; then echo '{}'; exit 0; fi\nif [ \"$1\" = \"image\" ]; then echo 'image failed' >&2; exit 4; fi\nexit 0\n",
        );
        let detector = SandboxRuntimeDetector::new(SandboxDetectorOptions {
            msb_binary: "definitely-missing-msb-for-chatspeed-test".to_string(),
            docker_binary: bad_binary,
            ..SandboxDetectorOptions::default()
        });
        assert_eq!(
            detector.detect().docker.state,
            SandboxAvailabilityState::InstalledButUnhealthy
        );
    }

    #[test]
    fn empty_image_list_reports_ready_missing_required_images() {
        let (_ready_dir, ready_binary) = write_fake_runtime(
            "docker",
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'Docker version 27.0.1, build abc'; exit 0; fi\nif [ \"$1\" = \"info\" ]; then echo '{}'; exit 0; fi\nif [ \"$1\" = \"image\" ]; then exit 0; fi\nexit 0\n",
        );
        let detector = SandboxRuntimeDetector::new(SandboxDetectorOptions {
            msb_binary: "definitely-missing-msb-for-chatspeed-test".to_string(),
            docker_binary: ready_binary,
            required_images: vec!["busybox:latest".to_string()],
            ..SandboxDetectorOptions::default()
        });
        let summary = detector.detect();
        assert_eq!(
            summary.docker.state,
            SandboxAvailabilityState::ReadyMissingImage
        );
        assert_eq!(summary.docker.missing_images, vec!["busybox:latest"]);
    }

    #[test]
    fn reports_unhealthy_for_version_probe_timeout_or_exit_error() {
        let (_exit_dir, exit_binary) = write_fake_runtime(
            "msb",
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'permission denied' >&2; exit 126; fi\nexit 0\n",
        );
        let detector = SandboxRuntimeDetector::new(SandboxDetectorOptions {
            msb_binary: exit_binary,
            docker_binary: "definitely-missing-docker-for-chatspeed-test".to_string(),
            ..SandboxDetectorOptions::default()
        });
        let summary = detector.detect();
        assert_eq!(
            summary.msb.state,
            SandboxAvailabilityState::InstalledButUnhealthy
        );
        assert_eq!(summary.msb.reason_code.as_deref(), Some("unhealthy"));

        let (_timeout_dir, timeout_binary) = write_fake_runtime(
            "docker",
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then sleep 2; fi\nexit 0\n",
        );
        let detector = SandboxRuntimeDetector::new(SandboxDetectorOptions {
            msb_binary: "definitely-missing-msb-for-chatspeed-test".to_string(),
            docker_binary: timeout_binary,
            timeout: std::time::Duration::from_millis(50),
            ..SandboxDetectorOptions::default()
        });
        let summary = detector.detect();
        assert_eq!(
            summary.docker.state,
            SandboxAvailabilityState::InstalledButUnhealthy
        );
        assert_eq!(summary.docker.reason_code.as_deref(), Some("unhealthy"));
    }

    #[test]
    fn reports_not_installed_for_missing_fake_binary() {
        let detector = SandboxRuntimeDetector::new(SandboxDetectorOptions {
            msb_binary: "definitely-missing-msb-for-chatspeed-test".to_string(),
            docker_binary: "definitely-missing-docker-for-chatspeed-test".to_string(),
            ..SandboxDetectorOptions::default()
        });
        let summary = detector.detect();
        assert_eq!(summary.msb.state, SandboxAvailabilityState::NotInstalled);
        assert_eq!(summary.docker.state, SandboxAvailabilityState::NotInstalled);
    }
}
