//! Detect installed browser profiles so the editor can offer friendly names
//! instead of asking users to type a profile id.
//!
//! Chromium-family browsers (Chrome, Chromium, Brave, Edge, Vivaldi) store
//! profile display names in `<config>/Local State` under
//! `profile.info_cache`. Firefox lists profiles in `profiles.ini`.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct DetectedProfile {
    /// Browser label, e.g. "Chrome".
    pub browser: String,
    /// Friendly profile name shown to the user, e.g. "Olaf (Work)".
    pub display: String,
    /// The on-disk profile directory.
    pub path: PathBuf,
    /// A stable key for our shared-profile dir (slug of browser + display).
    pub key: String,
    /// True for Chromium-based browsers (cookie import is at least plausible).
    pub chromium: bool,
}

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// All detected browser profiles, Chromium first, then Firefox.
pub fn detect() -> Vec<DetectedProfile> {
    let mut out = Vec::new();
    let Some(config) = dirs::config_dir() else {
        return out;
    };

    // (config subdir, friendly browser label)
    let chromium_browsers = [
        ("google-chrome", "Chrome"),
        ("chromium", "Chromium"),
        ("BraveSoftware/Brave-Browser", "Brave"),
        ("microsoft-edge", "Edge"),
        ("vivaldi", "Vivaldi"),
    ];

    for (subdir, label) in chromium_browsers {
        let base = config.join(subdir);
        let local_state = base.join("Local State");
        let Ok(text) = std::fs::read_to_string(&local_state) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let Some(cache) = json
            .get("profile")
            .and_then(|p| p.get("info_cache"))
            .and_then(|c| c.as_object())
        {
            for (dir, info) in cache {
                let display = info
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or(dir)
                    .to_string();
                let path = base.join(dir);
                if path.exists() {
                    out.push(DetectedProfile {
                        browser: label.to_string(),
                        display,
                        key: format!("{}-{}", slug(label), slug(dir)),
                        path,
                        chromium: true,
                    });
                }
            }
        }
    }

    // Firefox: ~/.mozilla/firefox/profiles.ini
    if let Some(home) = dirs::home_dir() {
        let ff_base = home.join(".mozilla/firefox");
        if let Ok(ini) = std::fs::read_to_string(ff_base.join("profiles.ini")) {
            for (name, rel_path) in parse_firefox_ini(&ini) {
                let path = ff_base.join(&rel_path);
                if path.exists() {
                    out.push(DetectedProfile {
                        browser: "Firefox".to_string(),
                        key: format!("firefox-{}", slug(&name)),
                        display: name,
                        path,
                        chromium: false,
                    });
                }
            }
        }
    }

    out
}

/// Best-effort, one-time copy of a Chromium profile's session into our CEF
/// profile so a freshly-created web app reuses an existing login.
///
/// SAFE: reads only from `src` (never writes to the user's real browser
/// profile) and is idempotent (skips if the destination already has cookies).
/// May still fail to *decrypt* the cookies if the CEF build looks up a
/// different OS-keyring entry than the source browser — in that case the user
/// simply signs in once.
pub fn import_session(src_profile: &std::path::Path, dest_root: &std::path::Path) -> bool {
    let dest_cache = dest_root.join("cache");
    let dest_cookies_new = dest_cache.join("Network").join("Cookies");
    let dest_cookies_old = dest_cache.join("Cookies");
    if dest_cookies_new.exists() || dest_cookies_old.exists() {
        return false; // already populated; don't clobber a live session
    }

    let mut copied_any = false;

    // The os_crypt key lives in the browser's root Local State (one level up).
    if let Some(src_root) = src_profile.parent() {
        let src_ls = src_root.join("Local State");
        if src_ls.exists() {
            let _ = std::fs::create_dir_all(dest_root);
            if std::fs::copy(&src_ls, dest_root.join("Local State")).is_ok() {
                copied_any = true;
            }
        }
    }

    // Cookies (+ their journal). Newer Chromium uses Network/Cookies.
    for rel in [
        std::path::Path::new("Network/Cookies"),
        std::path::Path::new("Network/Cookies-journal"),
        std::path::Path::new("Cookies"),
        std::path::Path::new("Cookies-journal"),
    ] {
        let s = src_profile.join(rel);
        if s.exists() {
            let d = dest_cache.join(rel);
            if let Some(parent) = d.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::copy(&s, &d).is_ok() {
                copied_any = true;
            }
        }
    }

    if copied_any {
        tracing::info!("imported session from {}", src_profile.display());
    }
    copied_any
}

/// Parse `profiles.ini` into (Name, Path) pairs.
fn parse_firefox_ini(ini: &str) -> Vec<(String, String)> {
    let mut profiles = Vec::new();
    let (mut name, mut path) = (None, None);
    let mut in_profile = false;
    for line in ini.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            if let (Some(n), Some(p)) = (name.take(), path.take()) {
                profiles.push((n, p));
            }
            in_profile = line.starts_with("[Profile");
            continue;
        }
        if !in_profile {
            continue;
        }
        if let Some(v) = line.strip_prefix("Name=") {
            name = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("Path=") {
            path = Some(v.to_string());
        }
    }
    if let (Some(n), Some(p)) = (name, path) {
        profiles.push((n, p));
    }
    profiles
}
