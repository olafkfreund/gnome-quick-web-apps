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

/// Decide whether in-app navigation to `target` should stay in the web app
/// window. Off-scope URLs are handed to the system browser instead — true PWA
/// behaviour and the key differentiator over a plain browser window.
///
/// Rules: internal runner schemes (`data:`, `about:`, blank) always stay.
/// With an explicit `scope` (from the manifest), a target is in-scope when it
/// is `http(s)` and string-prefixed by the scope. Without a scope, we fall
/// back to same-host as the app's own URL.
pub fn is_in_scope(target: &str, scope: Option<&str>, app_url: &str) -> bool {
    let t = target.trim();
    if t.is_empty() || t.starts_with("data:") || t.starts_with("about:") {
        return true;
    }
    if !(t.starts_with("http://") || t.starts_with("https://")) {
        // mailto:, tel:, intent:, etc. — hand off externally.
        return false;
    }
    match scope {
        Some(s) if !s.is_empty() => t.starts_with(s),
        _ => same_host(t, app_url),
    }
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string()))
}

fn same_host(a: &str, b: &str) -> bool {
    match (host_of(a), host_of(b)) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(&b),
        _ => false,
    }
}

pub use webapp::{Category, WebApp, WindowSize};

#[cfg(test)]
mod scope_tests {
    use super::is_in_scope;

    #[test]
    fn explicit_scope_prefix_matches() {
        let scope = Some("https://app.example.com/");
        assert!(is_in_scope("https://app.example.com/inbox", scope, "https://app.example.com/"));
        assert!(!is_in_scope("https://other.com/x", scope, "https://app.example.com/"));
        // same host but outside the scope path is off-scope
        assert!(!is_in_scope("https://app.example.com2/x", scope, "https://app.example.com/"));
    }

    #[test]
    fn no_scope_falls_back_to_same_host() {
        let app = "https://discord.com/app";
        assert!(is_in_scope("https://discord.com/channels/1", None, app));
        assert!(is_in_scope("https://www.discord.com/x", None, app)); // www-insensitive
        assert!(!is_in_scope("https://google.com", None, app));
    }

    #[test]
    fn internal_and_nonhttp_schemes() {
        assert!(is_in_scope("about:blank", None, "https://x.com"));
        assert!(is_in_scope("data:text/html,hi", None, "https://x.com"));
        assert!(!is_in_scope("mailto:a@b.com", None, "https://x.com"));
        assert!(!is_in_scope("tel:123", Some("https://x.com/"), "https://x.com"));
    }
}
