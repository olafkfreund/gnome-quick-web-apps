//! The web-app data model and its JSON persistence.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::paths;

pub type WindowWidth = u32;
pub type WindowHeight = u32;

/// How in-app navigation to another page is handled — the tri-state successor
/// to the old `external_links_in_browser` bool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkScope {
    /// Everything loads in the app window — never diverted. Best for apps whose
    /// sign-in spans multiple domains (Microsoft, Slack SSO).
    #[default]
    InWindow,
    /// Links to a *different registrable domain* open in the system browser;
    /// sibling subdomains stay in-window.
    SameSite,
    /// Links to a *different host* open in the system browser; only the exact
    /// site stays in-window. Best for webmail (Gmail's `google.com/url` leaves).
    ExactHost,
}

/// Per-app appearance override for `prefers-color-scheme` and the window chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorScheme {
    /// Follow the system (no override).
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WindowSize(pub WindowWidth, pub WindowHeight);

impl Default for WindowSize {
    fn default() -> Self {
        WindowSize(960, 720)
    }
}

/// Freedesktop main categories we expose in the editor.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Category {
    Audio,
    AudioVideo,
    Video,
    Development,
    Education,
    Game,
    Graphics,
    Network,
    Office,
    Science,
    Settings,
    System,
    #[default]
    Utility,
}

impl Category {
    /// The value written to the `.desktop` `Categories=` key.
    pub fn freedesktop(&self) -> &'static str {
        match self {
            Category::Audio => "Audio",
            Category::AudioVideo => "AudioVideo",
            Category::Video => "Video",
            Category::Development => "Development",
            Category::Education => "Education",
            Category::Game => "Game",
            Category::Graphics => "Graphics",
            Category::Network => "Network",
            Category::Office => "Office",
            Category::Science => "Science",
            Category::Settings => "Settings",
            Category::System => "System",
            Category::Utility => "Utility",
        }
    }

    /// Human-readable label for the editor dropdown.
    pub fn label(&self) -> &'static str {
        match self {
            Category::Audio => "Audio",
            Category::AudioVideo => "Audio & Video",
            Category::Video => "Video",
            Category::Development => "Development",
            Category::Education => "Education",
            Category::Game => "Game",
            Category::Graphics => "Graphics",
            Category::Network => "Network",
            Category::Office => "Office",
            Category::Science => "Science",
            Category::Settings => "Settings",
            Category::System => "System",
            Category::Utility => "Utility",
        }
    }

    pub fn from_index(index: u32) -> Category {
        Self::ALL.get(index as usize).copied().unwrap_or_default()
    }

    pub const ALL: [Category; 13] = [
        Category::Audio,
        Category::AudioVideo,
        Category::Video,
        Category::Development,
        Category::Education,
        Category::Game,
        Category::Graphics,
        Category::Network,
        Category::Office,
        Category::Science,
        Category::Settings,
        Category::System,
        Category::Utility,
    ];
}

/// A registered URL-scheme handler (e.g. the system default email app).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlHandler {
    /// Freedesktop mime, e.g. `x-scheme-handler/mailto`.
    pub mime: String,
    /// Target URL template; `{value}` (or mailto fields) is filled from the
    /// incoming scheme URL.
    pub template: String,
}

impl UrlHandler {
    /// The bare scheme, e.g. `mailto` from `x-scheme-handler/mailto`.
    pub fn scheme(&self) -> &str {
        self.mime.rsplit('/').next().unwrap_or(&self.mime)
    }
}

/// A single installed web application.
///
/// `scope` powers the differentiator over Quick Web Apps: navigation that
/// leaves the scope is handed to the system browser instead of staying in
/// the app window (true PWA behaviour). It is auto-filled from the site
/// manifest when available.
///
/// ## Storage-schema migration policy
///
/// App configs are persisted as JSON on disk and outlive the binary that
/// wrote them. To guarantee that older files keep deserializing as this
/// struct evolves:
///
/// - **Every new field MUST be `#[serde(default)]`** and have a sensible
///   `Default`, so files written before the field existed still load.
/// - **Never remove or rename a serialized field** without a migration step;
///   doing so silently drops (or fails to read) existing user data.
///
/// The `loads_v0_1_0_era_json` test pins this contract: a JSON file carrying
/// only the original fields must still deserialize, with every newer field
/// falling back to its default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebApp {
    /// Stable slug, e.g. `discord-com-a1b2`. Used for id, profile, WM class.
    pub id: String,
    pub name: String,
    pub url: String,
    /// Origin/path prefix in-app navigation is confined to. `None` = same host.
    pub scope: Option<String>,
    pub category: Category,
    /// Absolute path to the chosen/downloaded icon (PNG or SVG).
    pub icon_path: Option<PathBuf>,
    /// Optional theme colour from the manifest (`#rrggbb`), for the splash.
    pub theme_color: Option<String>,
    /// Override the user agent (e.g. to request the mobile site).
    pub user_agent: Option<String>,
    /// Shared session profile name. Apps with the same profile share cookies
    /// and logins (e.g. set every Google app to `google` to sign in once).
    /// `None`/empty = a private per-app profile keyed by `id`.
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub mobile: bool,
    /// When true, deliberate navigation to a different site opens in the system
    /// browser. Default false: everything (including multi-domain logins like
    /// Microsoft's) stays in the app window.
    #[serde(default)]
    pub external_links_in_browser: bool,
    /// Tri-state link handling. `None` falls back to `external_links_in_browser`
    /// (true => ExactHost) so older app files keep working.
    #[serde(default)]
    pub link_scope: Option<LinkScope>,
    /// URL-scheme handlers this app is registered as the system default for
    /// (e.g. mailto, webcal). Each carries the target URL template.
    #[serde(default)]
    pub handlers: Vec<UrlHandler>,
    #[serde(default)]
    pub window: WindowSize,
    /// Apply the bundled content-filter (adblock) ruleset.
    #[serde(default)]
    pub adblock: bool,
    /// Force light/dark appearance for the site, overriding the system theme.
    #[serde(default)]
    pub color_scheme: ColorScheme,
}

impl WebApp {
    /// Build a new web app from editor inputs, deriving a stable id with a
    /// short random suffix so two apps on the same host don't collide.
    pub fn new(name: String, url: String, category: Category) -> Self {
        use rand::Rng;
        let suffix: String = {
            let mut rng = rand::thread_rng();
            (0..4).map(|_| rng.gen_range(b'a'..=b'z') as char).collect()
        };
        let id = Self::slug_from_url(&url, &suffix);
        WebApp {
            id,
            name,
            url,
            scope: None,
            category,
            icon_path: None,
            theme_color: None,
            user_agent: None,
            profile: None,
            mobile: false,
            external_links_in_browser: false,
            link_scope: None,
            handlers: Vec::new(),
            window: WindowSize::default(),
            adblock: false,
            color_scheme: ColorScheme::System,
        }
    }

    /// Remove this app's JSON config, icon and profile directory from disk.
    /// (Launcher uninstall via the portal is handled separately, async.)
    pub fn remove_local(&self) {
        let _ = std::fs::remove_file(self.config_path());
        if let Some(icon) = &self.icon_path {
            let _ = std::fs::remove_file(icon);
        }
        // Only remove a private (per-app) profile; never a shared one that
        // other apps may still be using.
        if self
            .profile
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            let profile = paths::profile_dir(&self.id);
            if profile.exists() {
                let _ = std::fs::remove_dir_all(profile);
            }
        }
    }

    /// Effective link-handling mode: the explicit `link_scope`, else derived
    /// from the legacy `external_links_in_browser` bool (true => ExactHost).
    pub fn link_scope(&self) -> LinkScope {
        self.link_scope
            .unwrap_or(if self.external_links_in_browser {
                LinkScope::ExactHost
            } else {
                LinkScope::InWindow
            })
    }

    /// The session/cache key: the shared `profile` if set, else the app `id`
    /// (a private per-app profile). Apps sharing a key share their login.
    pub fn profile_key(&self) -> &str {
        self.profile
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.id)
    }

    /// The X11/Wayland app id used for `StartupWMClass` so the window groups
    /// under its own icon in the dock and Alt-Tab.
    pub fn wm_class(&self) -> String {
        format!("{}.{}", crate::APP_ID, self.id)
    }

    pub fn config_path(&self) -> PathBuf {
        paths::app_config(&self.id)
    }

    /// Write the app to `apps/<id>.json`.
    pub fn save(&self) -> anyhow::Result<()> {
        std::fs::write(self.config_path(), self.to_json()?)?;
        Ok(())
    }

    /// Load a single app by id.
    pub fn load(id: &str) -> anyhow::Result<Self> {
        let data = std::fs::read_to_string(paths::app_config(id))?;
        Self::from_json(&data)
    }

    /// Enumerate every installed web app.
    pub fn load_all() -> Vec<WebApp> {
        let mut apps = Vec::new();
        if let Ok(entries) = std::fs::read_dir(paths::apps_dir()) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Ok(data) = std::fs::read_to_string(&path) {
                        match serde_json::from_str::<WebApp>(&data) {
                            Ok(app) => apps.push(app),
                            Err(e) => tracing::warn!("skip {}: {e}", path.display()),
                        }
                    }
                }
            }
        }
        apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        apps
    }

    /// Serialize to JSON (used by `save`, exposed for testing).
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Parse from JSON (used by `load`, exposed for testing).
    pub fn from_json(data: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(data)?)
    }

    /// Derive a stable, filesystem-safe slug from a URL plus a short random
    /// suffix to avoid collisions between two apps on the same host.
    pub fn slug_from_url(url: &str, rand_suffix: &str) -> String {
        let host = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_else(|| "webapp".to_string());
        let base: String = host
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        format!("{}-{}", base.trim_matches('-'), rand_suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_filesystem_safe_and_stable() {
        let a = WebApp::slug_from_url("https://chat.openai.com/c/x", "abcd");
        assert_eq!(a, "chat-openai-com-abcd");
        // Stable for the same inputs.
        assert_eq!(
            a,
            WebApp::slug_from_url("https://chat.openai.com/c/x", "abcd")
        );
        // No path separators or other unsafe characters.
        assert!(!a.contains('/') && !a.contains('.'));
    }

    #[test]
    fn slug_falls_back_when_url_unparseable() {
        assert_eq!(WebApp::slug_from_url("not a url", "zzzz"), "webapp-zzzz");
    }

    #[test]
    fn new_derives_id_from_host_with_suffix() {
        let app = WebApp::new(
            "Discord".into(),
            "https://discord.com/app".into(),
            Category::Network,
        );
        assert!(app.id.starts_with("discord-com-"));
        // host slug + '-' + 4 random lowercase chars
        let suffix = app.id.rsplit('-').next().unwrap();
        assert_eq!(suffix.len(), 4);
        assert!(suffix.chars().all(|c| c.is_ascii_lowercase()));
        assert_eq!(app.wm_class(), format!("{}.{}", crate::APP_ID, app.id));
    }

    #[test]
    fn json_round_trip_preserves_fields() {
        let mut app = WebApp::new(
            "Notion".into(),
            "https://notion.so".into(),
            Category::Office,
        );
        app.scope = Some("https://notion.so/".into());
        app.theme_color = Some("#000000".into());
        app.mobile = true;
        app.adblock = true;

        let json = app.to_json().unwrap();
        let back = WebApp::from_json(&json).unwrap();

        assert_eq!(back.id, app.id);
        assert_eq!(back.name, app.name);
        assert_eq!(back.url, app.url);
        assert_eq!(back.scope, app.scope);
        assert_eq!(back.theme_color, app.theme_color);
        assert_eq!(back.category, app.category);
        assert_eq!(back.mobile, app.mobile);
        assert_eq!(back.adblock, app.adblock);
    }

    #[test]
    fn loads_v0_1_0_era_json() {
        // A config written before any of the newer fields existed: only the
        // original schema (id, name, url, scope, category, icon_path,
        // theme_color, user_agent). Every later field must default in.
        let json = r##"{
            "id": "discord-com-abcd",
            "name": "Discord",
            "url": "https://discord.com/app",
            "scope": "https://discord.com/",
            "category": "Network",
            "icon_path": null,
            "theme_color": "#5865f2",
            "user_agent": null
        }"##;

        let app = WebApp::from_json(json).expect("v0.1.0-era JSON must still load");

        // Original fields parsed as written.
        assert_eq!(app.id, "discord-com-abcd");
        assert_eq!(app.name, "Discord");
        assert_eq!(app.url, "https://discord.com/app");
        assert_eq!(app.scope.as_deref(), Some("https://discord.com/"));
        assert_eq!(app.category, Category::Network);
        assert_eq!(app.theme_color.as_deref(), Some("#5865f2"));

        // Every field added after v0.1.0 falls back to its default.
        assert_eq!(app.profile, None);
        assert!(!app.mobile);
        assert!(!app.external_links_in_browser);
        assert_eq!(app.link_scope, None);
        assert!(app.handlers.is_empty());
        assert!(!app.adblock);
        assert_eq!(app.color_scheme, ColorScheme::System);

        let default_window = WindowSize::default();
        assert_eq!(app.window.0, default_window.0);
        assert_eq!(app.window.1, default_window.1);
    }

    #[test]
    fn category_index_round_trips() {
        for (i, cat) in Category::ALL.iter().enumerate() {
            assert_eq!(Category::from_index(i as u32), *cat);
        }
        // Out-of-range falls back to the default.
        assert_eq!(Category::from_index(999), Category::default());
    }

    #[test]
    fn category_freedesktop_values_are_valid_main_categories() {
        assert_eq!(Category::AudioVideo.freedesktop(), "AudioVideo");
        assert_eq!(Category::Network.freedesktop(), "Network");
        assert_eq!(Category::Utility.label(), "Utility");
    }
}
