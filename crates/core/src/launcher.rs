//! `.desktop` launcher install/uninstall via the XDG **DynamicLauncher**
//! portal. This is the same sandbox-safe approach used by
//! `cosmic-utils/web-apps` (GPL-3.0) and is the reason this crate is GPL.
//!
//! The portal lets us register an application launcher + icon without
//! writing directly into `~/.local/share/applications`, so it works inside
//! a Flatpak sandbox.

use anyhow::{Context, Result};
use ashpd::desktop::dynamic_launcher::{
    DynamicLauncherProxy, Icon, InstallOptions, PrepareInstallOptions, UninstallOptions,
};

use crate::{webapp::WebApp, APP_ID};

/// `Exec=` line for the generated `.desktop`. Points at the CEF runner
/// binary with the web app id; the runner loads `apps/<id>.json` itself.
fn exec_line(app: &WebApp) -> String {
    // Resolved at install time to an absolute path when not on $PATH.
    format!("gnome-quick-web-apps-runner {}", app.id)
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
            None,
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
            InstallOptions::default(),
        )
        .await
        .context("install launcher")?;

    Ok(())
}

/// Remove the launcher for `app`.
pub async fn uninstall(app: &WebApp) -> Result<()> {
    let proxy = DynamicLauncherProxy::new()
        .await
        .context("connecting to DynamicLauncher portal")?;
    proxy
        .uninstall(&launcher_filename(app), UninstallOptions::default())
        .await
        .context("uninstall launcher")?;
    Ok(())
}
