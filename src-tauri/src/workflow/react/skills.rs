use crate::workflow::react::error::WorkflowEngineError;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tools: Vec<serde_json::Value>,
    #[serde(default)]
    pub instructions: String,

    // Store skill directory path for on-demand reference loading
    #[serde(skip)]
    pub skill_dir: Option<PathBuf>,

    // Available reference files metadata
    #[serde(default)]
    pub references: Vec<ReferenceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceInfo {
    pub filename: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}
pub struct SkillScanner {
    search_paths: Vec<PathBuf>,
}

impl SkillScanner {
    pub fn new(app_data_dir: PathBuf, resource_path: Option<PathBuf>) -> Self {
        let mut search_paths = vec![];

        // 1. ~/.chatspeed/skills (Priority 1)
        if let Some(home) = dirs::home_dir() {
            search_paths.push(home.join(".chatspeed").join("skills"));
        }

        // 2. ~/.agents/skills (Priority 2)
        if let Some(home) = dirs::home_dir() {
            search_paths.push(home.join(".agents").join("skills"));
        }

        // 3. software data dir (Priority 3)
        search_paths.push(app_data_dir.join("skills"));

        // 4. builtin skills from tauri assets (Priority 4)
        if let Some(res_path) = resource_path {
            search_paths.push(res_path.join("skills"));
        }

        Self { search_paths }
    }

    pub fn get_search_paths(&self) -> Vec<PathBuf> {
        self.search_paths.clone()
    }

    /// Scans all paths and returns a map of skill_name -> manifest.
    /// Higher priority paths (earlier in search_paths) override lower ones.
    pub fn scan(&self) -> Result<HashMap<String, SkillManifest>, WorkflowEngineError> {
        let mut skills = HashMap::new();

        // Iterate in REVERSE to allow higher priority paths to overwrite at the end
        for path in self.search_paths.iter().rev() {
            if !path.exists() {
                continue;
            }

            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let skill_dir = entry.path();
                    if skill_dir.is_dir() {
                        if let Some(manifest) = self.try_load_skill(&skill_dir) {
                            skills.insert(manifest.name.clone(), manifest);
                        }
                    }
                }
            }
        }

        Ok(skills)
    }

    fn try_load_skill(&self, dir: &std::path::Path) -> Option<SkillManifest> {
        // 1. Check for SKILL.md (Claude Code standard)
        let skill_md_path = dir.join("SKILL.md");
        if skill_md_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&skill_md_path) {
                // Trim BOM if present
                let content = content.strip_prefix('\u{feff}').unwrap_or(&content);

                if content.starts_with("---") {
                    let parts: Vec<&str> = content.splitn(3, "---").collect();
                    if parts.len() >= 3 {
                        let yaml_part = parts[1];
                        let body_part = parts[2]; // Everything after the second ---

                        match serde_yaml::from_str::<SkillManifest>(yaml_part) {
                            Ok(mut manifest) => {
                                manifest.instructions = body_part.to_string(); // Keep full body
                                manifest.skill_dir = Some(dir.to_path_buf());
                                manifest.references = self.scan_references(dir);
                                return Some(manifest);
                            }
                            Err(e) => {
                                log::warn!("Failed to parse YAML frontmatter in {:?}: {}. Trying fallback...", skill_md_path, e);

                                // Fallback: regex for name and description
                                let name_re = regex::Regex::new(r"(?m)^name:\s*(.+)$").unwrap();
                                let desc_re =
                                    regex::Regex::new(r"(?m)^description:\s*(.+)$").unwrap();

                                if let Some(cap) = name_re.captures(yaml_part) {
                                    let skill_name =
                                        cap.get(1).unwrap().as_str().trim().to_string();
                                    let description = desc_re
                                        .captures(yaml_part)
                                        .and_then(|c| c.get(1))
                                        .map(|m| m.as_str().trim().to_string())
                                        .unwrap_or_default();

                                    let manifest = SkillManifest {
                                        name: skill_name,
                                        version: "1.0.0".to_string(),
                                        description,
                                        tools: vec![],
                                        instructions: body_part.to_string(),
                                        skill_dir: Some(dir.to_path_buf()),
                                        references: self.scan_references(dir),
                                    };
                                    return Some(manifest);
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Check for skill.json or manifest.json (Legacy/Generic)
        let manifest_names = vec!["skill.json", "manifest.json"];
        for name in manifest_names {
            let manifest_path = dir.join(name);
            if manifest_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&manifest_path) {
                    match serde_json::from_str::<SkillManifest>(&content) {
                        Ok(mut manifest) => {
                            manifest.skill_dir = Some(dir.to_path_buf());
                            manifest.references = self.scan_references(dir);
                            return Some(manifest);
                        }
                        Err(e) => {
                            log::warn!(
                                "Failed to parse JSON manifest in {:?}: {}",
                                manifest_path,
                                e
                            );
                        }
                    }
                }
            }
        }

        None
    }

    /// Scan references directory and return metadata for available reference files
    fn scan_references(&self, skill_dir: &std::path::Path) -> Vec<ReferenceInfo> {
        let references_dir = skill_dir.join("references");
        if !references_dir.exists() {
            return vec![];
        }

        let mut references = vec![];
        if let Ok(entries) = std::fs::read_dir(&references_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && Self::is_valid_reference(&path) {
                    if let Some(ref_info) = self.extract_reference_info(&path) {
                        references.push(ref_info);
                    }
                }
            }
        }
        references.sort_by(|a, b| a.filename.cmp(&b.filename));
        references
    }

    /// Check if file is a valid reference file type
    fn is_valid_reference(path: &std::path::Path) -> bool {
        let valid_extensions = ["md", "txt", "json", "yaml", "yml"];
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| valid_extensions.contains(&ext))
            .unwrap_or(false)
    }

    /// Extract metadata from a reference file
    fn extract_reference_info(&self, path: &std::path::Path) -> Option<ReferenceInfo> {
        let metadata = std::fs::metadata(path).ok()?;
        let filename = path.file_name()?.to_str()?.to_string();

        // Read first line as description
        let description = std::fs::read_to_string(path)
            .ok()
            .and_then(|content| {
                content.lines().next().map(|line| {
                    // Remove Markdown heading symbols
                    line.trim_start_matches('#')
                        .trim()
                        .to_string()
                })
            })
            .unwrap_or_else(|| filename.clone());

        Some(ReferenceInfo {
            filename,
            description,
            size: Some(metadata.len()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_scan_references_with_files() {
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path();

        // Create references directory and files
        let refs_dir = skill_dir.join("references");
        fs::create_dir(&refs_dir).unwrap();

        fs::write(
            refs_dir.join("output-patterns.md"),
            "# Output Patterns\n\nDetailed patterns..."
        ).unwrap();

        fs::write(
            refs_dir.join("workflows.md"),
            "# Workflows\n\nWorkflow examples..."
        ).unwrap();

        let scanner = SkillScanner::new(PathBuf::new(), None);
        let references = scanner.scan_references(skill_dir);

        assert_eq!(references.len(), 2);
        assert!(references.iter().any(|r| r.filename == "output-patterns.md"));
        assert!(references.iter().any(|r| r.filename == "workflows.md"));
    }

    #[test]
    fn test_scan_references_empty_directory() {
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path();

        let scanner = SkillScanner::new(PathBuf::new(), None);
        let references = scanner.scan_references(skill_dir);

        assert!(references.is_empty());
    }

    #[test]
    fn test_reference_info_extraction() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.md");

        fs::write(&file_path, "# Test Reference\n\nContent here...").unwrap();

        let scanner = SkillScanner::new(PathBuf::new(), None);
        let ref_info = scanner.extract_reference_info(&file_path).unwrap();

        assert_eq!(ref_info.filename, "test.md");
        assert_eq!(ref_info.description, "Test Reference");
        assert!(ref_info.size.is_some());
    }

    #[test]
    fn test_invalid_reference_extension() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.exe");

        assert!(!SkillScanner::is_valid_reference(&file_path));
    }

    #[test]
    fn test_valid_reference_extensions() {
        let temp_dir = TempDir::new().unwrap();

        for ext in &["md", "txt", "json", "yaml", "yml"] {
            let file_path = temp_dir.path().join(format!("test.{}", ext));
            assert!(SkillScanner::is_valid_reference(&file_path));
        }
    }

    #[test]
    fn test_backward_compatibility() {
        // Create a skill without references directory
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path();

        // Only create SKILL.md
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: test-skill\ndescription: Test\n---\n\nInstructions..."
        ).unwrap();

        let scanner = SkillScanner::new(PathBuf::new(), None);
        let manifest = scanner.try_load_skill(skill_dir).unwrap();

        assert!(manifest.references.is_empty());
        assert!(manifest.skill_dir.is_some());
    }

    #[test]
    fn test_skill_manifest_with_references() {
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path();

        // Create SKILL.md
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: test-skill\ndescription: Test\n---\n\nInstructions..."
        ).unwrap();

        // Create references
        let refs_dir = skill_dir.join("references");
        fs::create_dir(&refs_dir).unwrap();
        fs::write(
            refs_dir.join("guide.md"),
            "# Guide\n\nDetailed guide..."
        ).unwrap();

        let scanner = SkillScanner::new(PathBuf::new(), None);
        let manifest = scanner.try_load_skill(skill_dir).unwrap();

        assert_eq!(manifest.name, "test-skill");
        assert_eq!(manifest.references.len(), 1);
        assert_eq!(manifest.references[0].filename, "guide.md");
        assert_eq!(manifest.references[0].description, "Guide");
        assert!(manifest.skill_dir.is_some());
    }
}
