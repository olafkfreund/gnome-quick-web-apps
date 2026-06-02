# Native Integration Pack — Design

> Created: 2026-06-02
> Status: Approved (design); ready for implementation planning
> Target: gnome-quick-web-apps (runner + core + manager)

## Summary

Make installed web apps behave like first-class GNOME desktop apps by adding four
desktop-integration features:

1. MPRIS media controls (lock screen, media keys, Quick Settings) — automatic.
2. Unread-count dock badges via the Unity LauncherEntry API — per-app toggle.
3. A StatusNotifier tray icon (Show / Quit) for background-mode apps.
4. "Start on login" autostart for background-mode apps.

All four are delivered through `gtk::gio` D-Bus (already a dependency — no new
crates, no Flatpak `cargo-sources` regeneration) and integrate with the GTK main
loop the runner already runs. Each integration keys per-app, which works inside
the shared one-process-per-profile model because every window already carries its
own `WebApp` context.

Non-goals: a unified multi-service hub, Chromium Badging API support (CEF exposes
no embedder hook), do-not-disturb scheduling, workspaces.

## Architecture

- D-Bus and desktop wiring use `gtk::gio::DBusConnection` (session bus), driven by
  the existing GTK main loop. No `zbus`/tokio addition.
- Two new CEF client callbacks on the per-window client:
  - `DisplayHandler::on_title_change` — drives the badge count.
  - `DisplayHandler::on_console_message` — the JS-to-Rust channel for media state.
- JS injection reuses the existing post-load `execute_java_script` path (the one
  that injects custom CSS).
- Per-window keying: the runner hosts one process per profile with multiple
  windows; each window's `OsrClient` holds its `Rc<WebApp>`, so each window owns
  its own MPRIS player, badge target, and (if background) tray item.

## Components

### 1. MPRIS media controls (automatic, per window)

Flow:
- On each load, inject a small script that:
  - reads `navigator.mediaSession.metadata` and playback state,
  - observes changes (metadata setter wrap + `play`/`pause`/`ratechange`
    listeners on media elements as a fallback),
  - emits `console.log("QWA_MEDIA:" + JSON.stringify(state))` on change.
- `on_console_message` parses the `QWA_MEDIA:` prefix and updates that window's
  MPRIS player.
- Publish `org.mpris.MediaPlayer2.qwa_<app-id>` implementing `MediaPlayer2` and
  `MediaPlayer2.Player`: PlaybackStatus, Metadata (xesam:title/artist/album,
  mpris:artUrl), CanPlay/CanPause/CanGoNext/CanGoPrevious, and methods
  Play/Pause/PlayPause/Next/Previous/Stop/Seek.
- MPRIS method calls map to `execute_java_script` that invokes the page's
  registered media-session action handlers (play/pause/nexttrack/previoustrack);
  if unavailable, dispatch the corresponding media key.
- The player is created lazily the first time media state is reported and removed
  when the window closes.

Thread bridge (important): `gio::DBusConnection::register_object` method-call
closures require `Fn + Send + Sync`, but the per-window CEF `Browser`/`Shared` are
`Rc` and not `Send`. D-Bus calls already arrive on the GTK main thread, but the
type bounds still forbid capturing the `Browser` directly. Resolve this with a
thread-local registry keyed by app id (e.g. `thread_local! { static PLAYERS:
RefCell<HashMap<String, WindowControls>> }`): the registered D-Bus closures
capture only the `String` app id (which is `Send`) and look the window up in the
registry on the GTK thread to call `execute_java_script`/`present`/`close`. The
same registry + lookup pattern covers the StatusNotifier menu actions in
component 3.

### 2. Unread badge (per-app toggle)

- `on_title_change` receives the live page title. A pure function
  `qwa_core::badge::count_from_title(title, pattern) -> Option<u32>` extracts the
  count using either the per-app `badge_pattern` or a default that matches a
  leading/trailing count such as `(3)`, `• 3`, `3 ·`.
- Publish via the Unity LauncherEntry D-Bus signal
  `com.canonical.Unity.LauncherEntry.Update` with `app_uri =
  "application://<APP_ID>.<app-id>.desktop"` and properties
  `{ count: i64, count-visible: bool }`. A zero/None count sets
  `count-visible=false`.
- Gated on the per-app `show_badge` flag. Pre-enabled on mail/chat templates.

### 3. Tray icon + Quit (per background-mode window)

- When `run_in_background` is set, register a StatusNotifierItem (via
  `org.kde.StatusNotifierWatcher`) for that window: app icon, tooltip
  (name + count), and a DBusMenu with "Show" (present the window) and "Quit"
  (close the browser + window for real).
- This gives background apps a way to be re-shown and genuinely quit (today the
  only quit is closing the last window).
- Documented limitation: GNOME shows StatusNotifier items only with the
  AppIndicator extension; KDE and others show them natively.

### 4. Autostart (per background-mode app)

- New `autostart` flag, surfaced as a "Start on login" sub-row under "Run in
  background" in the editor.
- Use the XDG **Background portal** (`org.freedesktop.portal.Background`,
  `RequestBackground` with `autostart: true` and the launcher command), via the
  existing `ashpd` dependency. This is sandbox-safe (works under Flatpak) and
  consistent with the portal-based DynamicLauncher install already in
  `launcher.rs`. Disabling calls `RequestBackground` with `autostart: false`.
- Rationale: a direct `~/.config/autostart/*.desktop` write is NOT sandbox-safe
  (it lands in the Flatpak per-app dir, ignored by the host session) and would
  have to reconstruct the dynamic `Exec` (AppImage `$APPIMAGE`, CEF
  `LD_LIBRARY_PATH`, dev-vs-installed runner path) that `exec_line()` computes —
  the portal avoids both problems.

## Data model

Add to `WebApp` (all `#[serde(default)]`, backward compatible; covered by the
existing migration round-trip test):

- `show_badge: bool` — default false; templates for mail/chat set true.
- `badge_pattern: Option<String>` — optional regex override for the title count.
- `autostart: bool` — default false.

No field is needed for MPRIS (automatic) or the tray (gated on the existing
`run_in_background`).

## Interfaces / editor

- Editor "Run in background" group gains a "Start on login" sub-toggle
  (`autostart`).
- A "Show unread badge" toggle plus an optional "Badge pattern" entry (revealed
  when the toggle is on).
- Templates: set `show_badge = true` for Gmail, Outlook, Teams, WhatsApp,
  Telegram, Discord, Slack, Messenger, Proton Mail.

## Error handling

- All D-Bus work is best-effort: if the session bus, the LauncherEntry consumer,
  or the StatusNotifierWatcher is absent, log at debug/info and continue — the app
  must still run normally.
- Malformed `QWA_MEDIA:` payloads are ignored.
- A title with no parseable count clears the badge (count-visible=false) rather
  than leaving a stale number.
- Autostart file write failures are warned and non-fatal.

## Testing strategy

- Unit tests (core): `count_from_title` across real-world titles (Gmail
  "Inbox (3) - ...", Teams "(2) | Microsoft Teams", WhatsApp "(5) WhatsApp",
  no-count titles, custom pattern). This also begins closing the "runner has no
  tests" gap by keeping parsing logic in testable core code.
- Manual verification (documented in the PR): `playerctl status/metadata/play-pause`
  for MPRIS; a LauncherEntry-honoring dock (Dash-to-Dock/Ubuntu/KDE) for the
  badge; KDE or GNOME+AppIndicator for the tray; a reboot/login for autostart.

## Migration / compatibility

- Purely additive: new serde-default fields; existing app JSON loads unchanged.
  Note the new fields must also be added to the explicit `WebApp::new()` struct
  literal and to the `Template` `t()`/`tc()` const constructors (set
  `show_badge=true` in `tc()`, the communication-app helper).
- Autostart goes through the Background portal — no change to the launcher
  install path and sandbox-safe.
- Engine-independent: no CEF version change, no new dependency, so Flatpak
  `cargo-sources.json` and the Nix build are unaffected.

## Decision log

- Badges from the page title, not the Chromium Badging API: CEF exposes no
  embedder hook for `navigator.setAppBadge`, and few target sites call it; the
  title is the signal that actually fires for webmail/chat today. A Badging-API
  path can be added later if CEF surfaces a hook.
- D-Bus via `gtk::gio`, not `zbus`: avoids a second async runtime and a heavy new
  dependency (which would also force a `cargo-sources.json` regen), and integrates
  with the existing GTK loop.
- JS-to-Rust media state via console sniffing, not render-process messaging:
  lowest-complexity channel that needs no helper/render-process plumbing; can be
  upgraded to `SendProcessMessage` if it proves noisy.
- StatusNotifier tray despite GNOME needing an extension: chosen by the user for
  full coverage (KDE/AppIndicator); GNOME limitation documented.
- Per-app keying inside the shared process: each window's `WebApp` context drives
  its own MPRIS/badge/tray, consistent with the one-process-per-profile model.

## Known limitations (documented for users)

- MPRIS metadata depends on the site implementing `navigator.mediaSession`.
- The dock badge appears only on docks/panels that honor the Unity LauncherEntry
  API.
- The tray icon appears on GNOME only with the AppIndicator extension.
