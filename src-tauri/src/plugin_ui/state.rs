//! Rust-only ownership of the plugin UI host state.
//!
//! The runtime state lives here instead of `mod.rs` so that module stays a pure
//! module list, and so the tab bookkeeping can be exercised without a live
//! runtime. Two pieces of state are owned:
//!
//! - [`PluginUiTabs`] keeps one session per open tab, keyed by the tab id the
//!   frontend generated. A session remembers the bundle it shows and the grant
//!   that authorises its assets, so a repeated show can reuse the live tab
//!   instead of revoking and rebuilding it.
//! - [`PluginUiRuntime`] holds the started gateway, the tab sessions and the
//!   asynchronous gate that serialises every UI command with the plugin
//!   lifecycle mutations, so an open can never interleave a close or a
//!   disable/uninstall cleanup.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use super::gateway::PluginUiGateway;
use super::types::PluginUiGrant;

/// Longest accepted plugin UI tab id.
///
/// The frontend names a panel with a fresh UUID, so a tab id is bounded to the
/// canonical UUID text; anything longer is refused before it reaches the host.
pub const MAX_TAB_ID_LEN: usize = 36;

/// Largest number of plugin UI panels one window may keep open at once.
pub const MAX_PLUGIN_UI_TABS: usize = 8;

/// Whether `tab_id` is the canonical UUID text the frontend generates.
///
/// A tab id is used as a map key and as a native carrier key, so it is validated
/// once here: the UUID form (hyphenated or simple) and the length bound keep a
/// hostile or malformed id out of the host.
pub fn is_valid_tab_id(tab_id: &str) -> bool {
    !tab_id.is_empty()
        && tab_id.len() <= MAX_TAB_ID_LEN
        && uuid::Uuid::parse_str(tab_id).is_ok()
}

/// One open plugin UI tab.
///
/// It remembers which bundle the tab shows, so the next open of the same tab can
/// tell a resize/reveal from a switch to another entry. The capability token and
/// the resolved URLs stay inside the grant and are never returned to Vue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginUiTabSession {
    /// The bundle the tab is currently showing.
    pub plugin_id: String,
    /// The verified UI entry the tab is currently showing.
    pub entry: String,
    /// The per-tab capability grant that authorises this tab's asset requests.
    pub grant: PluginUiGrant,
}

/// What an open request must do to a tab it already tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginUiTabPlan {
    /// The tab shows the same bundle: re-prove the inventory and re-place the live
    /// tab, keeping its grant.
    Reuse,
    /// The tab shows a different bundle: revoke the old grant and issue a new one.
    Replace,
    /// The tab is new and may be opened, issuing a new grant.
    Open,
    /// The tab is new but the panel limit is already reached.
    Rejected,
}

/// Decides how an open request treats the tab it names.
///
/// This is the pure part of `plugin_ui_open`, kept separate so the reuse,
/// replace and limit contracts are testable without a runtime or a native host.
pub fn plan_tab_open(
    existing: Option<&PluginUiTabSession>,
    plugin_id: &str,
    entry: &str,
    open_tabs: usize,
) -> PluginUiTabPlan {
    match existing {
        Some(session) if session.plugin_id == plugin_id && session.entry == entry => {
            PluginUiTabPlan::Reuse
        }
        Some(_) => PluginUiTabPlan::Replace,
        None if open_tabs >= MAX_PLUGIN_UI_TABS => PluginUiTabPlan::Rejected,
        None => PluginUiTabPlan::Open,
    }
}

/// Rust-only ownership of the grants held by the open plugin tabs.
#[derive(Default)]
pub struct PluginUiTabs {
    sessions: Mutex<HashMap<String, PluginUiTabSession>>,
}

impl PluginUiTabs {
    /// The session of `tab_id`, when the tab is open.
    pub fn session(&self, tab_id: &str) -> Result<Option<PluginUiTabSession>, String> {
        Ok(self.lock()?.get(tab_id).cloned())
    }

    /// Records the session of a freshly opened or replaced tab.
    pub fn remember(&self, tab_id: &str, session: PluginUiTabSession) -> Result<(), String> {
        self.lock()?.insert(tab_id.to_string(), session);
        Ok(())
    }

    /// Forgets `tab_id`, returning the session it held so its grant can be revoked.
    pub fn forget(&self, tab_id: &str) -> Result<Option<PluginUiTabSession>, String> {
        Ok(self.lock()?.remove(tab_id))
    }

    /// How many tabs the host currently tracks.
    ///
    /// The count can include a tab whose native panel failed to build, because a
    /// failed show forgets its session before returning.
    pub fn count(&self) -> Result<usize, String> {
        Ok(self.lock()?.len())
    }

    /// Removes every session, returning them so their grants can be revoked.
    pub fn take_all(&self) -> Result<Vec<PluginUiTabSession>, String> {
        Ok(self.lock()?.drain().map(|(_, session)| session).collect())
    }

    fn lock(&self) -> Result<MutexGuard<'_, HashMap<String, PluginUiTabSession>>, String> {
        self.sessions
            .lock()
            .map_err(|_| "plugin UI tab state is poisoned".to_string())
    }
}

/// Managed desktop state for the runtime-backed plugin UI host.
pub struct PluginUiRuntime {
    gateway: Mutex<Option<Arc<PluginUiGateway>>>,
    tabs: PluginUiTabs,
    gate: tokio::sync::Mutex<()>,
}

impl Default for PluginUiRuntime {
    fn default() -> Self {
        Self {
            gateway: Mutex::new(None),
            tabs: PluginUiTabs::default(),
            gate: tokio::sync::Mutex::new(()),
        }
    }
}

impl PluginUiRuntime {
    /// Records the gateway the startup task started.
    pub fn set_gateway(&self, gateway: PluginUiGateway) -> Result<(), String> {
        *self
            .gateway
            .lock()
            .map_err(|_| "plugin UI runtime state is poisoned".to_string())? =
            Some(Arc::new(gateway));
        Ok(())
    }

    /// The started gateway, or an explicit error when the runtime never started it.
    ///
    /// Commands never lazily start a gateway: a second server would race the one
    /// the startup task owns, so an unavailable gateway is refused instead.
    pub fn gateway(&self) -> Result<Arc<PluginUiGateway>, String> {
        self.gateway
            .lock()
            .map_err(|_| "plugin UI runtime state is poisoned".to_string())?
            .clone()
            .ok_or_else(|| "plugin_ui_gateway_unavailable".to_string())
    }

    /// The tab sessions the host tracks.
    pub fn tabs(&self) -> &PluginUiTabs {
        &self.tabs
    }

    /// Serialises one plugin UI command with the plugin lifecycle mutations.
    ///
    /// Holding this guard for the whole of a command keeps an open from
    /// interleaving a close, or a disable/uninstall cleanup, so the tab map, the
    /// capability table and the native panels never disagree.
    pub async fn serialize(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.gate.lock().await
    }

    /// Revokes every capability and drops every tracked tab session.
    pub async fn revoke_all(&self) -> Result<(), String> {
        if let Ok(gateway) = self.gateway() {
            gateway.revoke_all().await;
        }
        self.tabs.take_all().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(plugin_id: &str, entry: &str) -> PluginUiTabSession {
        PluginUiTabSession {
            plugin_id: plugin_id.to_string(),
            entry: entry.to_string(),
            grant: PluginUiGrant {
                capability: "capability".to_string(),
                url: format!("http://127.0.0.1:1/ui/capability/{plugin_id}/{entry}"),
                prefix: format!("http://127.0.0.1:1/ui/capability/{plugin_id}/"),
            },
        }
    }

    #[test]
    fn a_tab_id_must_be_a_bounded_uuid() {
        assert!(is_valid_tab_id("6a1cbb49-0f5d-4b2a-9e9b-2b1d5f0b1a2c"));
        // The simple form is a UUID too.
        assert!(is_valid_tab_id("6a1cbb490f5d4b2a9e9b2b1d5f0b1a2c"));
        assert!(!is_valid_tab_id(""));
        assert!(!is_valid_tab_id("temporary-tab"));
        assert!(!is_valid_tab_id(&"a".repeat(MAX_TAB_ID_LEN + 1)));
    }

    #[test]
    fn the_same_bundle_reuses_a_tab_while_another_entry_replaces_it() {
        let open = session("agent-skills", "index.html");
        assert_eq!(
            plan_tab_open(Some(&open), "agent-skills", "index.html", 1),
            PluginUiTabPlan::Reuse
        );
        assert_eq!(
            plan_tab_open(Some(&open), "agent-skills", "panel.html", 1),
            PluginUiTabPlan::Replace
        );
        assert_eq!(
            plan_tab_open(Some(&open), "other", "index.html", 1),
            PluginUiTabPlan::Replace
        );
    }

    #[test]
    fn a_new_tab_opens_until_the_panel_limit_is_reached() {
        assert_eq!(plan_tab_open(None, "agent-skills", "index.html", 0), PluginUiTabPlan::Open);
        assert_eq!(
            plan_tab_open(None, "agent-skills", "index.html", MAX_PLUGIN_UI_TABS),
            PluginUiTabPlan::Rejected
        );
        // A known tab still reuses or replaces even at the limit, so it never traps.
        let open = session("agent-skills", "index.html");
        assert_eq!(
            plan_tab_open(Some(&open), "agent-skills", "index.html", MAX_PLUGIN_UI_TABS),
            PluginUiTabPlan::Reuse
        );
    }

    #[test]
    fn the_tab_table_remembers_forgets_and_drains_sessions() {
        let tabs = PluginUiTabs::default();
        assert_eq!(tabs.count().expect("count"), 0);
        assert_eq!(tabs.session("tab").expect("session"), None);

        tabs.remember("tab", session("agent-skills", "index.html"))
            .expect("remember");
        assert_eq!(tabs.count().expect("count"), 1);
        assert_eq!(
            tabs.session("tab").expect("session"),
            Some(session("agent-skills", "index.html"))
        );

        let forgotten = tabs.forget("tab").expect("forget");
        assert_eq!(forgotten, Some(session("agent-skills", "index.html")));
        assert_eq!(tabs.count().expect("count"), 0);

        tabs.remember("one", session("agent-skills", "index.html"))
            .expect("remember");
        tabs.remember("two", session("agent-skills", "index.html"))
            .expect("remember");
        assert_eq!(tabs.take_all().expect("take_all").len(), 2);
        assert_eq!(tabs.count().expect("count"), 0);
    }
}