//! Adapter for the native workflow titlebar. Vue and the tray keep their
//! original entry points; this adapter dispatches through the same commands.

use chatspeed_runtime_client::desktop_menu::{self, Action};
use tauri::AppHandle;

/// Starts the local bridge once the desktop's window event handlers are ready.
pub fn start(app: AppHandle, runtime_dir: std::path::PathBuf) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) =
            desktop_menu::serve(&runtime_dir, |action| dispatch(app.clone(), action)).await
        {
            log::warn!("[DesktopMenu] Local menu bridge unavailable: {error}");
        }
    });
}

async fn dispatch(app: AppHandle, action: Action) -> bool {
    use crate::commands::window::{
        open_note_window, open_proxy_switcher_window, open_setting_window, quit_window, show_window,
    };

    let result = match action {
        Action::Chat => show_window(app, "main"),
        Action::Assistant => show_window(app, "assistant"),
        Action::Workflow => show_window(app, "workflow"),
        Action::Note => open_note_window(app).await,
        Action::ProxySwitcher => open_proxy_switcher_window(app),
        Action::Quit => quit_window(app),
        action => open_setting_window(app, settings_page(action).map(str::to_owned)).await,
    };
    if let Err(error) = result {
        log::warn!("[DesktopMenu] {action:?}: {error}");
        return false;
    }
    true
}

fn settings_page(action: Action) -> Option<&'static str> {
    match action {
        Action::Settings => Some("general"),
        Action::Model => Some("model"),
        Action::Agent => Some("agent"),
        Action::Proxy => Some("proxy"),
        Action::ChatHub => Some("chatHub"),
        Action::Mcp => Some("mcp"),
        Action::Skill => Some("skill"),
        Action::ScraperTest => Some("scraperTest"),
        Action::About => Some("about"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_uses_vue_setting_tab_ids() {
        assert_eq!(settings_page(Action::Settings), Some("general"));
        assert_eq!(settings_page(Action::ChatHub), Some("chatHub"));
        assert_eq!(settings_page(Action::ScraperTest), Some("scraperTest"));
        assert_eq!(settings_page(Action::Quit), None);
    }
}
