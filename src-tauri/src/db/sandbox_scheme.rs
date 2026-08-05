//! Shared sandbox scheme persistence.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{
    db::{MainStore, StoreError},
    tools::SandboxSchemeConfig,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxScheme {
    pub id: String,
    pub name: String,
    pub description: String,
    pub config: SandboxSchemeConfig,
    pub disabled: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl SandboxScheme {
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.id.trim().is_empty() {
            return Err(StoreError::InvalidData(
                "sandbox scheme id cannot be empty".to_string(),
            ));
        }
        if self.name.trim().is_empty() {
            return Err(StoreError::InvalidData(
                "sandbox scheme name cannot be empty".to_string(),
            ));
        }
        self.config.validate().map_err(StoreError::InvalidData)
    }
}

impl MainStore {
    pub fn add_sandbox_scheme(&self, scheme: &SandboxScheme) -> Result<String, StoreError> {
        scheme.validate()?;
        let scheme = scheme.clone();
        let id = scheme.id.clone();
        self.db_runtime()?.write_blocking(move |conn| {
            let config = serde_json::to_string(&scheme.config)?;
            conn.execute(
                "INSERT INTO sandbox_schemes (id, name, description, config, disabled)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    scheme.id,
                    scheme.name,
                    scheme.description,
                    config,
                    scheme.disabled
                ],
            )?;
            Ok(id)
        })
    }

    pub fn update_sandbox_scheme(&self, scheme: &SandboxScheme) -> Result<(), StoreError> {
        scheme.validate()?;
        let scheme = scheme.clone();
        self.db_runtime()?.write_blocking(move |conn| {
            if scheme.disabled {
                let references = scheme_reference_names(conn, &scheme.id)?;
                if !references.is_empty() {
                    return Err(StoreError::InvalidData(format!(
                        "sandbox scheme cannot be disabled while referenced by agents: {}",
                        references.join(", ")
                    )));
                }
            }
            let config = serde_json::to_string(&scheme.config)?;
            let changed = conn.execute(
                "UPDATE sandbox_schemes
                 SET name = ?1, description = ?2, config = ?3, disabled = ?4,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?5",
                params![
                    scheme.name,
                    scheme.description,
                    config,
                    scheme.disabled,
                    scheme.id
                ],
            )?;
            if changed == 0 {
                return Err(StoreError::NotFound("sandbox scheme not found".to_string()));
            }
            Ok(())
        })
    }

    pub fn get_sandbox_scheme(&self, id: &str) -> Result<Option<SandboxScheme>, StoreError> {
        let id = id.to_string();
        self.db_runtime()?.read_blocking(move |conn| {
            conn.query_row(
                "SELECT id, name, description, config, disabled, created_at, updated_at
                 FROM sandbox_schemes WHERE id = ?1",
                params![id],
                |row| {
                    let config: String = row.get("config")?;
                    let config = serde_json::from_str(&config).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                    Ok(SandboxScheme {
                        id: row.get("id")?,
                        name: row.get("name")?,
                        description: row.get("description")?,
                        config,
                        disabled: row.get("disabled")?,
                        created_at: row.get("created_at")?,
                        updated_at: row.get("updated_at")?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    pub fn get_all_sandbox_schemes(&self) -> Result<Vec<SandboxScheme>, StoreError> {
        self.db_runtime()?.read_blocking(|conn| {
            let mut statement = conn.prepare(
                "SELECT id, name, description, config, disabled, created_at, updated_at
                 FROM sandbox_schemes ORDER BY name COLLATE NOCASE, id",
            )?;
            let rows = statement.query_map([], |row| {
                let config: String = row.get("config")?;
                let config = serde_json::from_str(&config).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(SandboxScheme {
                    id: row.get("id")?,
                    name: row.get("name")?,
                    description: row.get("description")?,
                    config,
                    disabled: row.get("disabled")?,
                    created_at: row.get("created_at")?,
                    updated_at: row.get("updated_at")?,
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    pub fn delete_sandbox_scheme(&self, id: &str) -> Result<(), StoreError> {
        let id = id.to_string();
        self.db_runtime()?.write_blocking(move |conn| {
            let references = scheme_reference_names(conn, &id)?;
            if !references.is_empty() {
                return Err(StoreError::InvalidData(format!(
                    "sandbox scheme is referenced by agents: {}",
                    references.join(", ")
                )));
            }
            let changed = conn.execute("DELETE FROM sandbox_schemes WHERE id = ?1", params![id])?;
            if changed == 0 {
                return Err(StoreError::NotFound("sandbox scheme not found".to_string()));
            }
            Ok(())
        })
    }
}

fn scheme_reference_names(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<Vec<String>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT name FROM agents WHERE sandbox_scheme_id = ?1
         ORDER BY name COLLATE NOCASE, id",
    )?;
    let rows = statement.query_map(params![id], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::Agent,
        tools::{SandboxNetworkPolicy, SandboxProfileConfig, WorkspaceAccess},
    };

    fn scheme(id: &str) -> SandboxScheme {
        SandboxScheme {
            id: id.to_string(),
            name: "Shared Docker".to_string(),
            description: "Shared scheme".to_string(),
            config: SandboxSchemeConfig {
                profiles: vec![SandboxProfileConfig {
                    id: "node".to_string(),
                    name: "Node".to_string(),
                    enabled: true,
                    priority: 10,
                    command_patterns: vec!["^node(?:\\s|$)".to_string()],
                    runtime_preference: Default::default(),
                    image: "node:22".to_string(),
                    image_size_bytes: None,
                    network: SandboxNetworkPolicy::default(),
                    resources: Default::default(),
                    workspace_access: WorkspaceAccess::ReadWrite,
                }],
                host_rules: vec![],
                runtime_preference: Default::default(),
            },
            disabled: false,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn shared_schemes_round_trip_and_prevent_referenced_delete() {
        let store = MainStore::new(":memory:").expect("create store");
        let scheme = scheme("scheme-1");
        store.add_sandbox_scheme(&scheme).expect("add scheme");
        let loaded = store
            .get_sandbox_scheme("scheme-1")
            .expect("load scheme")
            .expect("scheme exists");
        assert_eq!(loaded.config.profiles[0].id, "node");

        let mut first = Agent::new(
            "agent-1".to_string(),
            "First".to_string(),
            None,
            Some("primary".to_string()),
            None,
            String::new(),
            None,
            None,
            Some(serde_json::json!([crate::tools::TOOL_BASH]).to_string()),
            Some("[]".to_string()),
            None,
            Some("[]".to_string()),
            Some("[]".to_string()),
            Some(false),
            Some("default".to_string()),
            Some(true),
            Some("[]".to_string()),
            Some("standard".to_string()),
            Some(false),
            Some(false),
            None,
        );
        first.sandbox_execution_mode = crate::tools::ShellExecutionMode::Auto;
        first.sandbox_scheme_id = Some("scheme-1".to_string());
        store.add_agent(&first).expect("add first agent");

        let mut second = first.clone();
        second.id = "agent-2".to_string();
        second.name = "Second".to_string();
        store.add_agent(&second).expect("add second agent");

        let error = store
            .delete_sandbox_scheme("scheme-1")
            .expect_err("referenced scheme cannot be deleted");
        assert!(error.to_string().contains("First"));
        assert!(error.to_string().contains("Second"));

        let mut disabled = loaded;
        disabled.disabled = true;
        let error = store
            .update_sandbox_scheme(&disabled)
            .expect_err("referenced scheme cannot be disabled");
        assert!(error.to_string().contains("First"));
        assert!(error.to_string().contains("Second"));
    }
}
