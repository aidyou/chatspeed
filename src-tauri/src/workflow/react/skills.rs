use std::path::{PathBuf};
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::workflow::react::error::WorkflowEngineError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub tools: Vec<serde_json::Value>, 
    #[serde(default)]
    pub instructions: String,
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

        // 3. ~/.claude/skills (Priority 3 - compatibility)
        if let Some(home) = dirs::home_dir() {
            search_paths.push(home.join(".claude").join("skills"));
        }
        
        // 4. software data dir (Priority 4)
        search_paths.push(app_data_dir.join("skills"));
        
        Self { search_paths }
    }

    /// Scans all paths and returns a map of skill_name -> manifest.
    /// Higher priority paths (earlier in search_paths) override lower ones.
    pub fn scan(&self) -> Result<HashMap<String, SkillManifest>, WorkflowEngineError> {
        let mut skills = HashMap::new();
        
        // Iterate in REVERSE to allow higher priority paths to overwrite at the end
        for path in self.search_paths.iter().rev() {
            if !path.exists() { continue; }
            
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
                if content.starts_with("---") {
                    if let Some(end_idx) = content[3..].find("---") {
                        let yaml_part = &content[3..end_idx + 3];
                        let body_part = &content[end_idx + 6..];
                        
                        if let Ok(mut manifest) = serde_yaml::from_str::<SkillManifest>(yaml_part) {
                            manifest.instructions = body_part.trim().to_string();
                            return Some(manifest);
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
                    if let Ok(manifest) = serde_json::from_str::<SkillManifest>(&content) {
                        return Some(manifest);
                    }
                }
            }
        }

        None
    }
}
