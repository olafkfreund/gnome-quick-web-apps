//! `.desktop` launcher install/uninstall via the XDG **DynamicLauncher**
//! portal. This is the same sandbox-safe approach used by
//! `cosmic-utils/web-apps` (GPL-3.0) and is the reason this crate is GPL.
//!
//! The portal lets us register an application launcher + icon without
//! writing directly into `~/.local/share/applications`, so it works inside
//! a Flatpak sandbox.

use anyhow::{Context, Result};
use ashpd::{
    desktop::{
        dynamic_launcher::{DynamicLauncherProxy, PrepareInstallOptions},
        Icon,
    },
    WindowIdentifier,
};

use crate::{webapp::WebApp, APP_ID};

const RUNNER_BIN: &str = "gnome-quick-web-apps-runner";

/// Search `$PATH` for an installed runner binary, returning its absolute
/// path if `<entry>/gnome-quick-web-apps-runner` exists and is a file.
fn runner_on_path() -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(RUNNER_BIN);
        if candidate.is_file() {
            return Some(candidate.display().to_string());
        }
    }
    None
}

/// Locate the runner binary: prefer one sitting next to the current
/// executable (installed builds / Flatpak), else trust `$PATH`.
///
/// Special-case dev builds: when `current_exe()` lives under
/// `target/debug` or `target/release`, the sibling runner is a dev binary
/// that only works inside `nix develop`. Baking that path into an installed
/// `.desktop` launcher means launching from the GNOME app grid fails. So for
/// dev builds we first try to find an INSTALLED runner on `$PATH`, and only
/// fall back to the dev sibling (with a loud warning) if none is found.
fn runner_path() -> String {
    if let Ok(exe) = std::env::current_exe() {
        let exe_str = exe.display().to_string();
        let is_dev_build =
            exe_str.contains("/target/debug/") || exe_str.contains("/target/release/");

        if is_dev_build {
            // Prefer an installed runner from `$PATH` so the generated
            // launcher works outside `nix develop`.
            if let Some(installed) = runner_on_path() {
                return installed;
            }
            // No installed runner: fall back to the dev sibling, but warn
            // that this path only works inside the dev shell.
            if let Some(dir) = exe.parent() {
                let sibling = dir.join(RUNNER_BIN);
                if sibling.exists() {
                    tracing::warn!(
                        runner = %sibling.display(),
                        "using a DEV-build runner path in the .desktop launcher; \
                         launching from the GNOME app grid will fail because this \
                         path only works inside `nix develop`. Install the package \
                         (Nix flake or Flatpak) for a working app-grid launcher.",
                    );
                    return sibling.display().to_string();
                }
            }
        } else if let Some(dir) = exe.parent() {
            // Installed binary under a real prefix: keep the original
            // sibling-path behavior exactly.
            let sibling = dir.join(RUNNER_BIN);
            if sibling.exists() {
                return sibling.display().to_string();
            }
        }
    }
    RUNNER_BIN.to_string()
}

/// `Exec=` line for the generated `.desktop`. Points at the CEF runner
/// binary with the web app id; the runner loads `apps/<id>.json` itself.
/// When a deployed CEF runtime is found, prefix `LD_LIBRARY_PATH` so the
/// runner can load `libcef.so` (mirrors the upstream launcher).
fn exec_line(app: &WebApp) -> String {
    let runner = runner_path();
    // `%u` lets the system pass a URL (e.g. a clicked mailto:) to the runner.
    let arg = if app.handlers.is_empty() {
        app.id.clone()
    } else {
        format!("{} %u", app.id)
    };
    match crate::cef_dir() {
        Some(cef) => format!("env LD_LIBRARY_PATH={} {} {}", cef.display(), runner, arg),
        None => format!("{} {}", runner, arg),
    }
}

fn desktop_entry(app: &WebApp) -> String {
    let mut e = String::new();
    e.push_str("[Desktop Entry]\n");
    e.push_str("Version=1.0\n");
    e.push_str("Type=Application\n");
    e.push_str(&format!("Name={}\n", app.name));
    e.push_str("Comment=Web App (GNOME Quick Web Apps)\n");
    e.push_str(&format!("Exec={}\n", exec_line(app)));
    e.push_str("Terminal=false\n");
    e.push_str(&format!("StartupWMClass={}\n", app.wm_class()));
    e.push_str(&format!("Categories={};\n", app.category.freedesktop()));
    if !app.handlers.is_empty() {
        let mimes: String = app
            .handlers
            .iter()
            .map(|h| format!("{};", h.mime))
            .collect();
        e.push_str(&format!("MimeType={mimes}\n"));
    }
    e
}

fn launcher_filename(app: &WebApp) -> String {
    format!("{}.{}.desktop", APP_ID, app.id)
}

/// Install (or replace) the launcher for `app`. `icon_png` is the raw bytes
/// of the app icon to register with the portal.
pub async fn install(app: &WebApp, icon_png: Vec<u8>) -> Result<()> {
    let proxy = DynamicLauncherProxy::new()
        .await
        .context("connecting to DynamicLauncher portal")?;

    let prepared = proxy
        .prepare_install(
            &WindowIdentifier::default(),
            &app.name,
            Icon::Bytes(icon_png),
            PrepareInstallOptions::default().editable_icon(true),
        )
        .await
        .context("prepare_install")?
        .response()
        .context("prepare_install response")?;

    proxy
        .install(
            prepared.token(),
            &launcher_filename(app),
            &desktop_entry(app),
        )
        .await
        .context("install launcher")?;

    Ok(())
}

/// Register this app as the system default for each of its registered scheme
/// handlers. Best-effort via `xdg-mime`; runs after the launcher is installed.
pub fn set_as_default_handlers(app: &WebApp) {
    if app.handlers.is_empty() {
        return;
    }
    // On systems that manage `mimeapps.list` declaratively (e.g. NixOS with
    // home-manager) it is a read-only symlink into /nix/store, so `xdg-mime`
    // can't write it — and worse, exits 0 while printing a raw error. Detect
    // that up front and skip with a single actionable message.
    if let Some(path) = mimeapps_list_path() {
        if is_readonly_managed(&path) {
            let mimes: Vec<&str> = app.handlers.iter().map(|h| h.mime.as_str()).collect();
            tracing::warn!(
                "not setting default handlers: {} is read-only / declaratively managed. \
                 Set these in your system config instead ({}).",
                path.display(),
                mimes.join(", ")
            );
            return;
        }
    }
    let file = launcher_filename(app);
    for handler in &app.handlers {
        match std::process::Command::new("xdg-mime")
            .args(["default", &file, &handler.mime])
            .output()
        {
            Ok(o) if o.status.success() && o.stderr.is_empty() => {
                tracing::info!("{} -> default for {}", app.id, handler.mime)
            }
            // xdg-mime can exit 0 yet fail to write; treat any stderr as failure.
            Ok(o) => tracing::warn!(
                "could not set default handler for {}: {}",
                handler.mime,
                String::from_utf8_lossy(&o.stderr).trim()
            ),
            Err(e) => tracing::warn!("xdg-mime failed: {e}"),
        }
    }
}

/// Where `xdg-mime` writes the user's default-application associations.
fn mimeapps_list_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|c| c.join("mimeapps.list"))
}

/// True when `path` is a read-only, declaratively-managed file — i.e. a symlink
/// resolving into the immutable Nix store (home-manager/NixOS).
fn is_readonly_managed(path: &std::path::Path) -> bool {
    std::fs::canonicalize(path)
        .map(|real| real.starts_with("/nix/store"))
        .unwrap_or(false)
}

/// Remove the launcher for `app`.
pub async fn uninstall(app: &WebApp) -> Result<()> {
    let proxy = DynamicLauncherProxy::new()
        .await
        .context("connecting to DynamicLauncher portal")?;
    proxy
        .uninstall(&launcher_filename(app))
        .await
        .context("uninstall launcher")?;
    Ok(())
}
