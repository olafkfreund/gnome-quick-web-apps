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

  <br>

  <img alt="Multiple web apps running as native GNOME windows" src="resources/screenshots/running-apps.png" width="860"><br>
  <em>Your web apps run as real, separate GNOME windows — tile them, Alt-Tab between them, each with its own icon and session.</em>

  <br><br>

  <img alt="A web app rendering natively" src="resources/screenshots/app-window.png" width="860"><br>
  <em>Crisp native rendering with a proper window — Google Docs here, indistinguishable from a desktop app.</em>

  <br><br>

  <img alt="Manage your web apps" src="resources/screenshots/manager.png" width="720"><br>
  <em>Manage all your web apps in one place — each with its own icon, profile and dock identity.</em>

  <br><br>

  <table>
    <tr>
      <td align="center" width="50%">
        <img alt="Add from a curated template catalog" src="resources/screenshots/templates.png"><br>
        <em>One-click templates for 50+ popular apps.</em>
      </td>
      <td align="center" width="50%">
        <img alt="Editor with profiles and default-handler toggles" src="resources/screenshots/editor.png"><br>
        <em>Per-app profiles, icons, and dynamic “default for…” toggles.</em>
      </td>
    </tr>
  </table>
</div>

---

> [!NOTE]
> **Built with AI assistance.** This project was developed using
> [Claude Code](https://claude.com/claude-code) (Anthropic) under continuous
> human review and supervision. Every change was reviewed, tested, and approved
> by a human maintainer.

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
- **Phase 4 — Polish:** GNOME Shell search provider, import from COSMIC / Linux Mint webapp-manager, prebuilt release bundles.

See the [Epic and child issues](https://github.com/olafkfreund/gnome-quick-web-apps/issues) for live status.

## Installation

### NixOS / Nix (flake)

```sh
# Try it without installing
nix run github:olafkfreund/gnome-quick-web-apps

# Install into your profile
nix profile install github:olafkfreund/gnome-quick-web-apps
```

In a NixOS or Home Manager config:

```nix
{
  inputs.quick-web-apps.url = "github:olafkfreund/gnome-quick-web-apps";

  # then, in your packages:
  environment.systemPackages = [ inputs.quick-web-apps.packages.${pkgs.system}.default ];
  # or home.packages = [ ... ];
}
```

The flake pins the matching CEF build and patches it for NixOS, so no manual
setup is needed.

### Everyone else — Flatpak

```sh
flatpak install -y flathub org.gnome.Platform//50 org.gnome.Sdk//50 \
  org.freedesktop.Sdk.Extension.rust-stable//25.08
flatpak-builder --user --install --force-clean build \
  build-aux/flatpak/io.github.olafkfreund.QuickWebApps.yml
```

The offline cargo sources (`cargo-sources.json`) are committed, and CI builds an
installable `.flatpak` bundle on every push — grab it from the latest run's
artifacts.

## Building from source (dev)

> Requires the Rust toolchain, GTK4 ≥ 4.12 and libadwaita ≥ 1.5. On NixOS use
> the dev shell (it provides the CEF runtime libraries):

```sh
nix develop -c just build      # manager + runner + helper
nix develop -c just run <id>   # launch a web app's CEF window
nix develop -c just manager    # the editor
```

## License

[GPL-3.0-only](LICENSE). Portions of the launcher logic are derived from
`cosmic-utils/web-apps` (GPL-3.0).
