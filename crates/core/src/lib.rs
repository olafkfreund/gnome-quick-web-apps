//! Shared core for GNOME Quick Web Apps.
//!
//! This crate is UI-agnostic: it holds the data model, on-disk storage,
//! PWA manifest detection, icon handling and `.desktop` launcher install.
//! Both the GTK4 `manager` and the CEF `runner` depend on it.

pub mod handlers;
pub mod icon;
pub mod launcher;
pub mod mailto;
pub mod manifest;
pub mod paths;
pub mod profiles;
pub mod templates;
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
            // Dev layout: a `cef/` dir beside the binary.
            let sibling = dir.join("cef");
            if sibling.exists() {
                return Some(sibling);
            }
            // Installed layout (Nix): <prefix>/share/cef next to <prefix>/bin.
            if let Some(prefix) = dir.parent() {
                let share = prefix.join("share").join("cef");
                if share.exists() {
                    return Some(share);
                }
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
    // Same registrable domain as the app is always in-scope (covers a site's
    // sibling subdomains). An explicit manifest scope only widens this.
    if same_host(t, app_url) {
        return true;
    }
    match scope {
        Some(s) if !s.is_empty() => t.starts_with(s),
        _ => false,
    }
}

/// Registrable domain (eTLD+1 heuristic: last two labels). Keeps a web app's
/// sibling subdomains in scope — e.g. `mail.google.com` and
/// `accounts.google.com` both reduce to `google.com`, so Google's login
/// redirect stays inside the app window.
fn registrable_domain(url: &str) -> Option<String> {
    let host = url::Url::parse(url).ok()?.host_str()?.to_lowercase();
    let host = host.trim_start_matches("www.");
    let labels: Vec<&str> = host.split('.').collect();
    Some(if labels.len() >= 2 {
        labels[labels.len() - 2..].join(".")
    } else {
        host.to_string()
    })
}

fn same_host(a: &str, b: &str) -> bool {
    same_site(a, b)
}

/// True when two URLs have the same exact host (www-insensitive). Used to
/// route a link to a *specific* installed web app — unlike `same_site`, this
/// does NOT collapse subdomains, so docs/drive/calendar.google.com each match
/// only their own app.
pub fn host_eq(a: &str, b: &str) -> bool {
    fn host(u: &str) -> Option<String> {
        url::Url::parse(u)
            .ok()?
            .host_str()
            .map(|h| h.trim_start_matches("www.").to_lowercase())
    }
    match (host(a), host(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// True when two URLs share a registrable domain (e.g. `mail.google.com` and
/// `contacts.google.com` are same-site). Used to decide whether a navigation
/// or popup belongs to the running app or is genuinely external.
pub fn same_site(a: &str, b: &str) -> bool {
    match (registrable_domain(a), registrable_domain(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Known identity / SSO / CAPTCHA hosts that must always load *in-window*,
/// even when external-links mode is on. Many sign-in flows hop to one of these
/// on a different registrable domain (and often via POST); ejecting them to the
/// system browser breaks login (e.g. Microsoft's AADSTS900561). Mirrors the
/// pragmatic allowlist used by eyekay/webapps, extended for common IdPs.
pub fn is_infra_host(target: &str) -> bool {
    let Some(host) = url::Url::parse(target)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_lowercase()))
    else {
        return false;
    };
    const EXACT: &[&str] = &[
        "accounts.google.com",
        "accounts.youtube.com",
        "appleid.apple.com",
        "challenges.cloudflare.com",
    ];
    // Subdomain suffixes (note the leading dot, so the bare domain isn't matched
    // unless it's actually a sub-host like `login.microsoftonline.com`).
    const SUFFIX: &[&str] = &[
        ".microsoftonline.com",
        ".live.com",
        ".microsoft.com",
        ".windows.net",
        ".okta.com",
        ".auth0.com",
        ".b2clogin.com",
        ".onelogin.com",
        ".duosecurity.com",
        ".pingidentity.com",
    ];
    EXACT.contains(&host.as_str()) || SUFFIX.iter().any(|s| host.ends_with(s))
}

/// Whether navigating to `target` should stay in the app window (vs. open in
/// the system browser), under the app's link-handling `mode`. Identity/SSO/
/// CAPTCHA hosts always stay. This is the single source of truth used by both
/// the navigation and popup handlers.
pub fn stays_in_window(target: &str, scope: Option<&str>, home: &str, mode: LinkScope) -> bool {
    if is_infra_host(target) {
        return true;
    }
    match mode {
        LinkScope::InWindow => true,
        LinkScope::SameSite => is_in_scope(target, scope, home),
        LinkScope::ExactHost => {
            host_eq(target, home) || scope.is_some_and(|s| !s.is_empty() && target.starts_with(s))
        }
    }
}

pub use webapp::{Category, ColorScheme, LinkScope, WebApp, WindowSize};

#[cfg(test)]
mod scope_tests {
    use super::is_in_scope;

    #[test]
    fn explicit_scope_prefix_matches() {
        let scope = Some("https://app.example.com/");
        assert!(is_in_scope(
            "https://app.example.com/inbox",
            scope,
            "https://app.example.com/"
        ));
        assert!(!is_in_scope(
            "https://other.com/x",
            scope,
            "https://app.example.com/"
        ));
        // same host but outside the scope path is off-scope
        assert!(!is_in_scope(
            "https://app.example.com2/x",
            scope,
            "https://app.example.com/"
        ));
    }

    #[test]
    fn no_scope_falls_back_to_same_host() {
        let app = "https://discord.com/app";
        assert!(is_in_scope("https://discord.com/channels/1", None, app));
        assert!(is_in_scope("https://www.discord.com/x", None, app)); // www-insensitive
        assert!(!is_in_scope("https://google.com", None, app));
    }

    #[test]
    fn sibling_subdomains_share_scope() {
        // A Gmail web app: the login redirect to accounts.google.com must
        // count as in-scope so it doesn't get punted to the system browser.
        let app = "https://mail.google.com/";
        assert!(is_in_scope("https://accounts.google.com/signin", None, app));
        assert!(is_in_scope("https://docs.google.com/x", None, app));
        assert!(!is_in_scope("https://youtube.com/x", None, app));
    }

    #[test]
    fn internal_and_nonhttp_schemes() {
        assert!(is_in_scope("about:blank", None, "https://x.com"));
        assert!(is_in_scope("data:text/html,hi", None, "https://x.com"));
        assert!(!is_in_scope("mailto:a@b.com", None, "https://x.com"));
        assert!(!is_in_scope(
            "tel:123",
            Some("https://x.com/"),
            "https://x.com"
        ));
    }

    #[test]
    fn infra_hosts_match() {
        use super::is_infra_host;
        // Identity / SSO / CAPTCHA hosts that must stay in-window.
        assert!(is_infra_host(
            "https://login.microsoftonline.com/common/oauth2"
        ));
        assert!(is_infra_host("https://login.live.com/oauth20"));
        assert!(is_infra_host("https://accounts.google.com/signin"));
        assert!(is_infra_host("https://challenges.cloudflare.com/turnstile"));
        assert!(is_infra_host("https://acme.okta.com/login"));
        // Ordinary destinations are not infra and may leave for the browser.
        assert!(!is_infra_host("https://www.foxtons.co.uk/"));
        assert!(!is_infra_host("https://example.com/"));
        // teams.microsoft.com is a `.microsoft.com` sub-host, so it counts as
        // infra; keeping it in-window is harmless since it's the app itself.
        assert!(is_infra_host("https://teams.microsoft.com/"));
    }
}
