//! XDG path helpers. All app state lives under `$XDG_DATA_HOME/<APP_ID>/`.

use std::path::PathBuf;

use crate::APP_ID;

fn data_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(std::env!("HOME")).join(".local/share"))
        .join(APP_ID)
}

fn ensure(dir: PathBuf) -> PathBuf {
    if !dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("failed to create {}: {e}", dir.display());
        }
    }
    dir
}

/// `.../apps/` — one JSON file per web app (the source of truth).
pub fn apps_dir() -> PathBuf {
    ensure(data_root().join("apps"))
}

/// `.../icons/` — downloaded / generated icons.
pub fn icons_dir() -> PathBuf {
    ensure(data_root().join("icons"))
}

/// `.../profiles/<id>/` — per-app CEF profile (isolated cookies/storage).
pub fn profile_dir(id: &str) -> PathBuf {
    ensure(data_root().join("profiles").join(id))
}

/// `.../filters/` — compiled content-blocker rulesets (adblock).
pub fn filters_dir() -> PathBuf {
    ensure(data_root().join("filters"))
}

/// Full path to a web app's JSON config.
pub fn app_config(id: &str) -> PathBuf {
    apps_dir().join(format!("{id}.json"))
}

/// `.../window/<id>.state` — last-session window geometry + zoom for an app,
/// restored on the next launch.
pub fn window_state(id: &str) -> PathBuf {
    ensure(data_root().join("window")).join(format!("{id}.state"))
}
