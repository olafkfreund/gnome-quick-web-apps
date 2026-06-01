//! Browser-process entry: initialize CEF with a per-app profile and run the
//! message loop. Port of the upstream cefsimple `run_main`, fed by our JSON
//! `WebApp` config instead of a RON `Browser`.

use cef::*;
use qwa_core::WebApp;

pub mod simple_app;
pub mod simple_handler;

pub struct Library;

#[allow(dead_code)]
pub fn load_cef() -> Library {
    let library = Library;
    // Initialize the CEF API version.
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    library
}

/// Load this runner's web app from `apps/<id>.json`. The id is the first
/// non-flag argv entry (CEF may inject its own `--switches`).
pub(crate) fn current_app() -> Option<WebApp> {
    let id = std::env::args().skip(1).find(|a| !a.starts_with('-'))?;
    WebApp::load(&id).ok()
}

#[allow(dead_code)]
pub fn run_main(main_args: &MainArgs, cmd_line: &CommandLine, sandbox_info: *mut u8) {
    let switch = CefString::from("type");
    let is_browser_process = cmd_line.has_switch(Some(&switch)) != 1;

    let ret = execute_process(Some(main_args), None, sandbox_info);

    if is_browser_process {
        assert_eq!(ret, -1, "cannot execute browser process");
    } else {
        // Non-browser (helper) process: CEF handled it; do not initialize.
        assert!(ret >= 0, "cannot execute non-browser process");
        return;
    }

    let Some(webapp) = current_app() else {
        tracing::error!("no web app config found; pass a valid app id");
        return;
    };

    // Browser process: render CEF off-screen inside a native GNOME window.
    crate::osr::run(main_args, sandbox_info, webapp);
}

/// Build CEF [`Settings`] for the browser process (off-screen rendering,
/// per-app profile cache, user agent, deployed resource dirs).
pub fn build_settings(webapp: &WebApp) -> Settings {
    let helper_path = qwa_core::helper_bin();
    let root = qwa_core::paths::profile_dir(&webapp.id);
    let cache = root.join("cache");

    let (resources_dir_path, locales_dir_path) = match qwa_core::cef_dir() {
        Some(dir) => (
            CefString::from(dir.display().to_string().as_str()),
            CefString::from(dir.join("locales").display().to_string().as_str()),
        ),
        None => (CefString::default(), CefString::default()),
    };

    Settings {
        no_sandbox: 1,
        windowless_rendering_enabled: 1,
        browser_subprocess_path: CefString::from(helper_path.as_str()),
        root_cache_path: CefString::from(root.display().to_string().as_str()),
        cache_path: CefString::from(cache.display().to_string().as_str()),
        user_agent: CefString::from(qwa_core::effective_ua(webapp)),
        resources_dir_path,
        locales_dir_path,
        ..Default::default()
    }
}
