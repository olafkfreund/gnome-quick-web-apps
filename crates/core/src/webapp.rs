//! The web-app data model and its JSON persistence.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::paths;

pub type WindowWidth = u32;
pub type WindowHeight = u32;

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

/// A single installed web application.
///
/// `scope` powers the differentiator over Quick Web Apps: navigation that
/// leaves the scope is handed to the system browser instead of staying in
/// the app window (true PWA behaviour). It is auto-filled from the site
/// manifest when available.
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
    #[serde(default)]
    pub mobile: bool,
    #[serde(default)]
    pub window: WindowSize,
    /// Apply the bundled content-filter (adblock) ruleset.
    #[serde(default)]
    pub adblock: bool,
}

impl WebApp {
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
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(self.config_path(), json)?;
        Ok(())
    }

    /// Load a single app by id.
    pub fn load(id: &str) -> anyhow::Result<Self> {
        let data = std::fs::read_to_string(paths::app_config(id))?;
        Ok(serde_json::from_str(&data)?)
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
