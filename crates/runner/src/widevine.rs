//! Best-effort Widevine CDM provisioning for DRM playback (Apple Music,
//! Netflix, Spotify Web, …).
//!
//! Our runner uses CEF's **off-screen (windowless) rendering**, which runs the
//! Alloy runtime. Alloy ships no Widevine CDM and has no working component
//! updater, so DRM-gated playback fails — even though system Chromium (which
//! bundles a CDM) plays the same site (issue #36).
//!
//! The Widevine CDM is proprietary and **not redistributable**, so we cannot
//! bundle it. Instead, if the user already has a Chromium-family browser
//! installed, that browser has fetched a CDM we can reuse: Chromium discovers
//! an external CDM under its user-data dir at
//! `WidevineCdm/<version>/_platform_specific/linux_x64/libwidevinecdm.so`.
//! We copy that tree from a detected host browser into CEF's user-data dir so
//! the embedded engine discovers it the same way.
//!
//! This is entirely best-effort: if no host CDM is found, nothing is copied and
//! playback simply stays unavailable. Errors are logged and swallowed.

use std::path::{Path, PathBuf};

/// The platform-specific CDM library, relative to a `WidevineCdm/<version>`
/// directory, that proves a directory holds a usable Linux x64 CDM.
const CDM_LIB_REL: &str = "_platform_specific/linux_x64/libwidevinecdm.so";

/// Ensure CEF's `user_data_dir` has a Widevine CDM, copying one from a detected
/// host browser if needed. No-op if already provisioned or none is found.
pub fn provision(user_data_dir: &Path) {
    let dest_root = user_data_dir.join("WidevineCdm");

    // Already have a usable CDM? Leave it.
    if newest_cdm_version(&dest_root).is_some() {
        tracing::debug!("Widevine CDM already present under {}", dest_root.display());
        return;
    }

    let Some((src_version_dir, version)) = find_host_cdm() else {
        tracing::info!(
            "no host Widevine CDM found; DRM playback (e.g. Apple Music) will be \
             unavailable. Install a Chromium-family browser to enable it (#36)."
        );
        return;
    };

    let dest_version_dir = dest_root.join(&version);
    match copy_tree(&src_version_dir, &dest_version_dir) {
        Ok(()) => tracing::info!(
            "provisioned Widevine CDM {} from {} -> {}",
            version,
            src_version_dir.display(),
            dest_version_dir.display(),
        ),
        Err(e) => {
            tracing::warn!(
                "failed to provision Widevine CDM from {}: {e}",
                src_version_dir.display()
            );
            // Don't leave a half-copied tree that looks valid.
            let _ = std::fs::remove_dir_all(&dest_version_dir);
        }
    }
}

/// Host locations that may hold a `WidevineCdm` directory, in rough preference
/// order (a real user profile's component-updated CDM is freshest).
fn host_cdm_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let cfg = home.join(".config");
        for browser in [
            "google-chrome",
            "google-chrome-beta",
            "google-chrome-unstable",
            "chromium",
            "microsoft-edge",
            "BraveSoftware/Brave-Browser",
            "vivaldi",
        ] {
            roots.push(cfg.join(browser).join("WidevineCdm"));
        }
        // Flatpak-installed Chromium-family browsers (reachable from our sandbox
        // only if the matching --filesystem permission is granted).
        let var = home.join(".var/app");
        roots.push(var.join("org.chromium.Chromium/config/chromium/WidevineCdm"));
        roots.push(var.join("com.google.Chrome/config/google-chrome/WidevineCdm"));
        roots.push(var.join("com.brave.Browser/config/BraveSoftware/Brave-Browser/WidevineCdm"));
    }
    // System-wide installs.
    for sys in [
        "/opt/google/chrome/WidevineCdm",
        "/opt/microsoft/msedge/WidevineCdm",
        "/usr/lib/chromium/WidevineCdm",
        "/usr/lib64/chromium/WidevineCdm",
        "/usr/lib/chromium-browser/WidevineCdm",
    ] {
        roots.push(PathBuf::from(sys));
    }
    roots
}

/// First host root that contains a usable CDM, as `(version_dir, version)`.
fn find_host_cdm() -> Option<(PathBuf, String)> {
    host_cdm_roots()
        .into_iter()
        .find_map(|root| newest_cdm_version(&root))
}

/// Within a `WidevineCdm` directory, return the newest version subdir that
/// actually contains the platform CDM library, as `(version_dir, version)`.
fn newest_cdm_version(widevine_dir: &Path) -> Option<(PathBuf, String)> {
    let entries = std::fs::read_dir(widevine_dir).ok()?;
    let mut candidates: Vec<(PathBuf, String)> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join(CDM_LIB_REL).is_file())
        .filter_map(|p| {
            let v = p.file_name()?.to_str()?.to_string();
            Some((p, v))
        })
        .collect();
    // Version dirs sort sensibly enough lexically for Chromium's dotted ints at
    // equal segment widths; newest wins.
    candidates.sort_by(|a, b| a.1.cmp(&b.1));
    candidates.pop()
}

/// Recursively copy a directory tree (files + symlink targets resolved to data).
fn copy_tree(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            // Follows symlinks (copies the pointed-to bytes), which is what we
            // want when reusing a CDM that a packager may have symlinked.
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
