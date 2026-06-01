//! GNOME Quick Web Apps — manager (editor) application.
//!
//! Phase 1 skeleton: an Adwaita application window that lists the installed
//! web apps and offers a "New Web App" action. The editor dialog, manifest
//! autofill wiring and launcher install land on top of this shell.

mod editor;
mod window;

use adw::prelude::*;
use qwa_core::APP_ID;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .build();

    app.connect_activate(|app| {
        let win = window::build(app);
        win.present();
    });

    let exit = app.run();
    std::process::exit(exit.value());
}
