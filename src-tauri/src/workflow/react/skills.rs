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
}

fn default_version() -> String {
    "1.0.0".to_string()
}
pub struct SkillScanner {
    search_paths: Vec<PathBuf>,
}

impl SkillScanner {
    pub fn new(app_data_dir: PathBuf) -> Self {
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

                                    return Some(SkillManifest {
                                        name: skill_name,
                                        version: "1.0.0".to_string(),
                                        description,
                                        tools: vec![],
                                        instructions: body_part.to_string(),
                                    });
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
                        Ok(manifest) => return Some(manifest),
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
}
