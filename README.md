<div align="center">
  <h1>GNOME Quick Web Apps</h1>
  <p><b>Turn any website into a first-class GNOME desktop app.</b></p>
  <p>A GTK4 / libadwaita web-app manager with PWA manifest auto-detection,
     automatic icons, true URL-scope confinement, and bundled Chromium (CEF)
     rendering for perfect site compatibility — including DRM.</p>

  <p>
    <a href="https://olafkfreund.github.io/gnome-quick-web-apps/">Website</a> ·
    <a href="https://github.com/olafkfreund/gnome-quick-web-apps/issues">Issues</a> ·
    <a href="#roadmap">Roadmap</a>
  </p>
</div>

---

## What is this?

A native GNOME alternative to [`cosmic-utils/web-apps`](https://github.com/cosmic-utils/web-apps)
(Quick Web Apps for the COSMIC desktop). You paste a URL, the app detects the
site's Web App Manifest, fills in the name/icon/theme for you, and installs a
launcher into your GNOME app grid. Each web app runs in its own isolated
window with its own profile and its own dock identity.

### Why it's better than the original

| | Quick Web Apps (COSMIC) | **GNOME Quick Web Apps** |
| --- | --- | --- |
| UI toolkit | libcosmic (iced) | **GTK4 + libadwaita** — native GNOME |
| Setup | type everything manually | **paste a URL → form autofills** from the PWA manifest |
| Icons | pick from Papirus / lettered | **auto-downloaded** best manifest/apple-touch icon, lettered fallback |
| Navigation | open browser window | **scope confinement** — off-scope links open in your system browser |
| Per-app | basic | per-app **user agent, adblock, zoom, custom CSS** |
| Discovery | app grid only | app grid **+ GNOME Shell search provider** (planned) |

Rendering uses **CEF (Chromium Embedded Framework)** for maximum site
compatibility (Widevine/DRM, Chrome-only sites), the same engine choice as
upstream's v3.

## Architecture

```
crates/core      shared model, JSON storage, PWA manifest detection,
                 icon pipeline, DynamicLauncher (.desktop) install
crates/manager   GTK4/libadwaita editor — create/edit/delete web apps
crates/runner    CEF binary launched by each .desktop (per-app window)
docs/            GitHub Pages showcase site
```

Two upstream techniques are reused (and are why this project is GPL-3.0):
the **XDG DynamicLauncher portal** for sandbox-safe `.desktop` install, and
`StartupWMClass` per app so each window gets its own dock/Alt-Tab identity.

## Roadmap

- **Phase 1 — Core + Manager (parity):** data model, storage, launcher install, GTK4 manager listing/CRUD.
- **Phase 2 — Differentiators:** PWA manifest autofill, auto-icon download, scope confinement, per-app UA, adblock.
- **Phase 3 — Native shell:** CEF off-screen rendering inside a libadwaita window with a real header bar, per-app zoom/CSS.
- **Phase 4 — Polish:** GNOME Shell search provider, import from COSMIC / Linux Mint webapp-manager, Flathub release.

See the [Epic and child issues](https://github.com/olafkfreund/gnome-quick-web-apps/issues) for live status.

## Building

> Requires the Rust toolchain, GTK4 ≥ 4.12 and libadwaita ≥ 1.5 development
> packages. CEF is vendored in Phase 2 (kept out of the default build until
> then so the workspace compiles without the Chromium download).

```sh
cargo build --workspace
```

## License

[GPL-3.0-only](LICENSE). Portions of the launcher logic are derived from
`cosmic-utils/web-apps` (GPL-3.0).
