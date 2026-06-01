//! Shared core for GNOME Quick Web Apps.
//!
//! This crate is UI-agnostic: it holds the data model, on-disk storage,
//! PWA manifest detection, icon handling and `.desktop` launcher install.
//! Both the GTK4 `manager` and the CEF `runner` depend on it.

pub mod icon;
pub mod launcher;
pub mod manifest;
pub mod paths;
pub mod webapp;

/// Reverse-DNS application id used for XDG dirs and the portal launcher prefix.
pub const APP_ID: &str = "io.github.olafkfreund.QuickWebApps";

/// User-agent strings for the CEF runner.
pub const DESKTOP_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36";
pub const MOBILE_UA: &str = "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.7632.76 Mobile Safari/537.36";

/// Resolve the CEF subprocess helper binary: prefer one sitting next to the
/// current executable, else trust `$PATH`.
pub fn helper_bin() -> String {
    const NAME: &str = "gnome-quick-web-apps-runner-helper";
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(NAME);
            if sibling.exists() {
                return sibling.display().to_string();
            }
        }
    }
    NAME.to_string()
}

/// Locate the deployed CEF runtime directory (holding `libcef.so` and its
/// resource files). Checked: a sibling `cef/` dir, then Flatpak `/app`, then
/// `/usr/local`. Mirrors the upstream resolver. Returns `None` if not found.
pub fn cef_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("cef");
            if sibling.exists() {
                return Some(sibling);
            }
        }
    }
    let prefix = if PathBuf::from("/.flatpak-info").exists() {
        "/app"
    } else {
        "/usr/local"
    };
    let installed = PathBuf::from(prefix).join("share").join("cef");
    installed.exists().then_some(installed)
}

/// The effective user agent for an app: explicit override, else mobile or
/// desktop default.
pub fn effective_ua(app: &WebApp) -> &str {
    if let Some(ua) = app.user_agent.as_deref() {
        ua
    } else if app.mobile {
        MOBILE_UA
    } else {
        DESKTOP_UA
    }
}

pub use webapp::{Category, WebApp, WindowSize};
