//! GNOME Quick Web Apps — runner.
//!
//! Invoked by the generated `.desktop` as `gnome-quick-web-apps-runner <id>`.
//! It loads `apps/<id>.json` and opens the site in an isolated CEF window
//! with a per-app profile.
//!
//! Phase 1: argument parsing + config loading + the design contract. The CEF
//! event loop, the `CefRequestHandler` that enforces `scope` confinement
//! (off-scope links open in the system browser), per-app user agent and the
//! content-filter ruleset land here in Phase 2. The native libadwaita
//! header bar via off-screen rendering arrives in Phase 3 (see #osr).

use anyhow::{Context, Result};
use qwa_core::WebApp;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let id = std::env::args()
        .nth(1)
        .context("usage: gnome-quick-web-apps-runner <app-id>")?;

    let app = WebApp::load(&id).with_context(|| format!("loading web app '{id}'"))?;
    let profile = qwa_core::paths::profile_dir(&app.id);

    tracing::info!(
        "would launch '{}' -> {} (scope: {:?}, profile: {})",
        app.name,
        app.url,
        app.scope,
        profile.display()
    );

    // PHASE 2 — CEF:
    //   let app = cef::App::new(RunnerApp { webapp: app, profile });
    //   cef::execute_process(...);   // routes helper subprocesses
    //   let settings = cef::Settings { ... cache_path: profile ... };
    //   cef::initialize(&settings, &app);
    //   browser_host::create_browser(window_info, client, &app.url, &browser_settings);
    //   cef::run_message_loop();
    //
    // The client's OnBeforeBrowse confines navigation to `app.scope`,
    // handing off-scope URLs to `open::that(url)`.

    Ok(())
}
