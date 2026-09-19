//! Persistence port for MCP desired state.
//!
//! The capability service needs to read and change the *desired* registration
//! of an MCP server without owning the database. `MainStore` stays the only
//! owner of the connection and of `ConfigCache` (INV-1); this port is the single
//! seam the service uses, and a fake implementation makes the whole MCP state
//! machine deterministic to test (V-6).
//!
//! The port is synchronous on purpose: it mirrors `MainStore`'s own short
//! transactions, and no download, connect or process wait may ever happen while
//! a database transaction is open (the service performs those through
//! [`crate::capability::mcp::runtime`] instead).

use std::sync::Arc;

use crate::capability::error::{code, CapabilityError};
use crate::db::{MainStore, Mcp};
use crate::db::StoreError;
use crate::mcp::client::McpServerConfig;

/// Maps a storage failure onto the stable capability error contract.
///
/// A missing record stays `not_found` so callers can distinguish "nothing to
/// remove" from "the database rejected us" (`store_error`). Only the variant
/// name crosses the boundary: store messages are localized and a serialization
/// failure can describe the value it rejected, so the message is never
/// forwarded (AC-13).
fn storage_error(error: StoreError) -> CapabilityError {
    match error {
        StoreError::NotFound(_) => {
            CapabilityError::new(code::NOT_FOUND, "the MCP record does not exist")
        }
        other => {
            let kind = serde_json::to_value(&other)
                .ok()
                .and_then(|value| {
                    value
                        .get("kind")
                        .and_then(|kind| kind.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "unknown".to_string());
            CapabilityError::new(
                code::STORE_ERROR,
                format!("the MCP record could not be stored: {kind}"),
            )
        }
    }
}

/// Reads a store write result as an optional record.
///
/// The store reports "no such row" as an error, but for a capability mutation
/// absence is a normal answer that must stay distinguishable from a storage
/// failure, so it becomes `Ok(None)` here.
fn to_record_option(
    result: Result<Mcp, StoreError>,
) -> Result<Option<Mcp>, CapabilityError> {
    match result {
        Ok(mcp) => Ok(Some(mcp)),
        Err(StoreError::NotFound(_)) => Ok(None),
        Err(other) => Err(storage_error(other)),
    }
}

/// A request to register a new MCP server.
#[derive(Debug, Clone)]
pub struct NewMcpRecord {    pub name: String,
    pub description: String,
    pub config: McpServerConfig,
    /// A new registration is disabled until the user explicitly enables it, so
    /// installing never starts a process (AC-9).
    pub disabled: bool,
}

/// The persisted desired state of MCP servers.
pub trait McpRepositoryPort: Send + Sync {
    /// Every registered server, in stable order.
    fn list(&self) -> Result<Vec<Mcp>, CapabilityError>;

    /// One server by its record id.
    fn get(&self, id: i64) -> Result<Option<Mcp>, CapabilityError>;

    /// One server by its configured name.
    fn find_by_name(&self, name: &str) -> Result<Option<Mcp>, CapabilityError>;

    /// Registers a new server and returns the persisted record.
    fn add(&self, record: NewMcpRecord) -> Result<Mcp, CapabilityError>;

    /// Rewrites an existing record's mutable fields.
    fn update(
        &self,
        id: i64,
        name: &str,
        description: &str,
        config: McpServerConfig,
        disabled: bool,
    ) -> Result<Option<Mcp>, CapabilityError>;

    /// Changes only the desired enabled state.
    fn set_disabled(&self, id: i64, disabled: bool) -> Result<Option<Mcp>, CapabilityError>;

    /// Removes the persisted record.
    fn delete(&self, id: i64) -> Result<(), CapabilityError>;
}

/// The production repository: `MainStore`, which also refreshes `ConfigCache`.
pub struct MainStoreMcpRepository {
    store: Arc<MainStore>,
}

impl MainStoreMcpRepository {
    pub fn new(store: Arc<MainStore>) -> Self {
        Self { store }
    }
}

impl McpRepositoryPort for MainStoreMcpRepository {
    fn list(&self) -> Result<Vec<Mcp>, CapabilityError> {
        Ok(self.store.config.get_mcps())
    }

    fn get(&self, id: i64) -> Result<Option<Mcp>, CapabilityError> {
        // The cache reports a missing record as an error; the port expresses
        // absence as `None` so uninstall and status can prove removal.
        match self.store.config.get_mcp_by_id(id) {
            Ok(mcp) => Ok(Some(mcp)),
            Err(StoreError::NotFound(_)) => Ok(None),
            Err(other) => Err(storage_error(other)),
        }
    }

    fn find_by_name(&self, name: &str) -> Result<Option<Mcp>, CapabilityError> {
        Ok(self
            .list()?
            .into_iter()
            .find(|mcp| mcp.name == name || mcp.config.name == name))
    }

    fn add(&self, record: NewMcpRecord) -> Result<Mcp, CapabilityError> {
        self.store
            .add_mcp(
                record.name,
                record.description,
                record.config,
                record.disabled,
            )
            .map_err(storage_error)
    }

    fn update(
        &self,
        id: i64,
        name: &str,
        description: &str,
        config: McpServerConfig,
        disabled: bool,
    ) -> Result<Option<Mcp>, CapabilityError> {
        to_record_option(self.store.update_mcp(id, name, description, config, disabled))
    }

    fn set_disabled(&self, id: i64, disabled: bool) -> Result<Option<Mcp>, CapabilityError> {
        to_record_option(self.store.change_mcp_status(id, disabled))
    }

    fn delete(&self, id: i64) -> Result<(), CapabilityError> {
        self.store
            .delete_mcp(id)
            .map_err(storage_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::client::McpProtocolType;
    use std::collections::HashSet;

    fn store() -> Arc<MainStore> {
        Arc::new(MainStore::new(":memory:").expect("in-memory store"))
    }

    fn config(name: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            protocol_type: McpProtocolType::Stdio,
            url: None,
            bearer_token: None,
            proxy: None,
            command: Some("node".to_string()),
            args: Some(vec!["server.js".to_string()]),
            env: Some(vec![("API_TOKEN".to_string(), "canary-value".to_string())]),
            disabled_tools: Some(HashSet::new()),
            timeout: None,
        }
    }

    #[test]
    fn a_record_round_trips_through_the_port() {
        let repository = MainStoreMcpRepository::new(store());
        let added = repository
            .add(NewMcpRecord {
                name: "weather".to_string(),
                description: "Weather data".to_string(),
                config: config("weather"),
                disabled: true,
            })
            .expect("add");

        assert_eq!(added.name, "weather");
        // A fresh registration is never enabled (AC-9).
        assert!(added.disabled);
        assert_eq!(repository.list().expect("list").len(), 1);
        assert_eq!(
            repository
                .find_by_name("weather")
                .expect("find")
                .map(|mcp| mcp.id),
            Some(added.id)
        );

        let enabled = repository
            .set_disabled(added.id, false)
            .expect("enable")
            .expect("record");
        assert!(!enabled.disabled);

        repository.delete(added.id).expect("delete");
        assert!(repository.list().expect("list").is_empty());
        assert!(
            repository.get(added.id).expect("get").is_none(),
            "a deleted record must not resolve, so uninstall can prove removal"
        );
    }

    #[test]
    fn an_absent_record_is_reported_as_absence_on_every_read_and_write() {
        let repository = MainStoreMcpRepository::new(store());
        // Absence must be provable, because uninstall success depends on it.
        assert!(repository.get(4242).expect("get").is_none());
        assert!(repository.find_by_name("ghost").expect("find").is_none());
        assert!(
            repository
                .set_disabled(4242, true)
                .expect("set_disabled")
                .is_none(),
            "changing a missing record cannot invent one"
        );
        assert!(repository.list().expect("list").is_empty());
    }
}
