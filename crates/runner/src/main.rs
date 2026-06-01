//! GNOME Quick Web Apps — runner (browser process).
//!
//! Invoked by the generated `.desktop` as `gnome-quick-web-apps-runner <id>`.
//! Loads `apps/<id>.json` and opens the site in an isolated CEF window with a
//! per-app profile. Structure mirrors the upstream cosmic-utils/web-apps CEF
//! port (cefsimple), adapted to our JSON `WebApp` model.

mod app;
mod osr;

fn main() -> Result<(), &'static str> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let _library = app::load_cef();

    let args = cef::args::Args::new();
    let Some(cmd_line) = args.as_cmd_line() else {
        return Err("Failed to parse command line arguments");
    };

    app::run_main(args.as_main_args(), &cmd_line, std::ptr::null_mut());

    Ok(())
}
