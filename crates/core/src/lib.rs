//! Shared core for GNOME Quick Web Apps.
//!
//! This crate is UI-agnostic: it holds the data model, on-disk storage,
//! PWA manifest detection, icon handling and `.desktop` launcher install.
//! Both the GTK4 `manager` and the CEF `runner` depend on it.

pub mod icon;
pub mod launcher;
pub mod manifest;
pub mod paths;
pub mod webapp;

/// Reverse-DNS application id used for XDG dirs and the portal launcher prefix.
pub const APP_ID: &str = "io.github.olafkfreund.QuickWebApps";

pub use webapp::{Category, WebApp, WindowSize};
