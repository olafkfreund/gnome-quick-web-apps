//! Native GNOME shell (#11, iteration 1: rendering pipeline).
//!
//! Instead of letting CEF create its own Views window, we run CEF in
//! **off-screen rendering** mode and paint its pixel buffer into a
//! `GtkDrawingArea` inside an `AdwApplicationWindow` with an `AdwHeaderBar` —
//! a real GNOME window. CEF is single-threaded here and pumped from the GTK
//! main loop, so all CEF callbacks land on the GTK thread (safe to touch
//! widgets directly).
//!
//! This iteration wires rendering + resize. Input forwarding (mouse/keyboard)
//! and the header-bar nav controls land next (#11 input pass, #12).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use cef::{Rect, *};
use gtk::gio;
use qwa_core::WebApp;

/// The most recent painted frame (BGRA, premultiplied — matches Cairo ARGB32).
struct Frame {
    buf: Vec<u8>,
    width: i32,
    height: i32,
}

#[derive(Default)]
struct Shared {
    frame: RefCell<Option<Frame>>,
    browser: RefCell<Option<Browser>>,
    /// Last known pointer position (for wheel events, which carry a location).
    mouse: std::cell::Cell<(i32, i32)>,
    /// The app's settled home URL, recorded once the initial load (including
    /// its redirect chain, e.g. gmail.com -> mail.google.com) completes. Until
    /// then it is None and navigation is unrestricted; afterwards anything that
    /// leaves this site is treated as external.
    home: RefCell<Option<String>>,
    /// Current CEF zoom level (0 = 100%; each step ≈ ±20%).
    zoom: std::cell::Cell<f64>,
    /// Weak handle to this window, so the MPRIS player's Raise/Quit methods can
    /// present or close it. Weak to avoid a reference cycle (the window owns the
    /// handlers that own `Shared`).
    window: RefCell<Option<gtk::glib::WeakRef<adw::ApplicationWindow>>>,
}

/// Run `f` with the live browser host, if a browser exists.
fn with_host<F: FnOnce(BrowserHost)>(shared: &Shared, f: F) {
    if let Some(browser) = shared.browser.borrow().as_ref() {
        if let Some(host) = browser.host() {
            f(host);
        }
    }
}

/// Apply a zoom level (clamped) to the browser and remember it.
fn set_zoom(shared: &Shared, level: f64) {
    let level = level.clamp(-3.0, 5.0);
    shared.zoom.set(level);
    with_host(shared, |h| h.set_zoom_level(level));
}

/// Restore last-session window geometry + zoom: `maximized\nWxH\nzoom`.
fn load_window_state(id: &str) -> Option<(bool, i32, i32, f64)> {
    let text = std::fs::read_to_string(qwa_core::paths::window_state(id)).ok()?;
    let mut lines = text.lines();
    let maximized = lines.next()? == "true";
    let (ws, hs) = lines.next()?.split_once('x')?;
    let (w, h) = (ws.parse().ok()?, hs.parse().ok()?);
    let zoom = lines.next().and_then(|z| z.parse().ok()).unwrap_or(0.0);
    Some((maximized, w, h, zoom))
}

/// Persist window geometry + zoom for next launch.
fn save_window_state(id: &str, maximized: bool, w: i32, h: i32, zoom: f64) {
    let body = format!("{maximized}\n{w}x{h}\n{zoom}\n");
    if let Err(e) = std::fs::write(qwa_core::paths::window_state(id), body) {
        tracing::warn!("failed to save window state for {id}: {e}");
    }
}

/// Map a GDK key to a Windows virtual-key code (what CEF expects). Printable
/// keys fall back to their uppercase ASCII; named keys map explicitly.
fn vk_from_keyval(k: gtk::gdk::Key) -> i32 {
    use gtk::gdk::Key;
    if k == Key::Return || k == Key::KP_Enter {
        0x0D
    } else if k == Key::BackSpace {
        0x08
    } else if k == Key::Tab {
        0x09
    } else if k == Key::Escape {
        0x1B
    } else if k == Key::Delete {
        0x2E
    } else if k == Key::Left {
        0x25
    } else if k == Key::Up {
        0x26
    } else if k == Key::Right {
        0x27
    } else if k == Key::Down {
        0x28
    } else if k == Key::Home {
        0x24
    } else if k == Key::End {
        0x23
    } else if k == Key::Page_Up {
        0x21
    } else if k == Key::Page_Down {
        0x22
    } else {
        // Only alphanumerics share their VK with uppercase ASCII. For
        // punctuation/symbols, ASCII != VK (e.g. '.' = 0x2E = VK_DELETE!),
        // so return 0 and let the CHAR event insert the character.
        match k.to_unicode() {
            Some(c) if c.is_ascii_alphanumeric() => c.to_ascii_uppercase() as i32,
            _ => 0,
        }
    }
}

/// Translate GTK modifier state into CEF event flags (Shift/Ctrl/Alt) so
/// shortcuts like Ctrl+V (paste), Ctrl+C, Ctrl+A work.
fn cef_modifiers(state: gtk::gdk::ModifierType) -> u32 {
    use gtk::gdk::ModifierType;
    let mut m = 0u32;
    if state.contains(ModifierType::SHIFT_MASK) {
        m |= 1 << 1; // EVENTFLAG_SHIFT_DOWN
    }
    if state.contains(ModifierType::CONTROL_MASK) {
        m |= 1 << 2; // EVENTFLAG_CONTROL_DOWN
    }
    if state.contains(ModifierType::ALT_MASK) {
        m |= 1 << 3; // EVENTFLAG_ALT_DOWN
    }
    m
}

fn mouse_event(x: i32, y: i32) -> MouseEvent {
    MouseEvent { x, y, modifiers: 0 }
}

fn map_button(gdk_button: u32) -> MouseButtonType {
    match gdk_button {
        2 => MouseButtonType::MIDDLE,
        3 => MouseButtonType::RIGHT,
        _ => MouseButtonType::LEFT,
    }
}

/// Encode `s` as a double-quoted JavaScript string literal, escaping the
/// characters that would otherwise break out of the literal (backslash, double
/// quote, newline, carriage return). Used to inject user CSS safely.
fn js_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

wrap_app! {
    pub struct OsrApp;
    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            if let Some(cmd) = command_line {
                // Force software rendering. Off-screen rendering needs a working
                // GPU/EGL to composite; many systems (and NixOS outside a GL
                // runtime) lack a loadable native EGL, which leaves the view
                // blank. Software compositing paints reliably.
                cmd.append_switch(Some(&CefString::from("disable-gpu")));
                cmd.append_switch(Some(&CefString::from("disable-gpu-compositing")));

                // Force the page's `prefers-color-scheme` when the app overrides
                // it. blink PreferredColorScheme: kDark=0, kLight=1.
                match crate::app::current_app().map(|a| a.color_scheme) {
                    Some(qwa_core::ColorScheme::Dark) => cmd.append_switch_with_value(
                        Some(&CefString::from("blink-settings")),
                        Some(&CefString::from("preferredColorScheme=0")),
                    ),
                    Some(qwa_core::ColorScheme::Light) => cmd.append_switch_with_value(
                        Some(&CefString::from("blink-settings")),
                        Some(&CefString::from("preferredColorScheme=1")),
                    ),
                    _ => {}
                }
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(OsrBrowserProcessHandler::new())
        }
    }
}

wrap_browser_process_handler! {
    struct OsrBrowserProcessHandler {}

    impl BrowserProcessHandler {
        // A second launch of an app sharing this profile cannot start its own
        // CEF process (CEF is a singleton per root_cache_path), so CEF forwards
        // that process's command line here, to the already-running primary, and
        // the second process exits. We open a new window for it — Chrome's
        // model: same-profile apps run as multiple windows in one process,
        // sharing cookies/logins. This runs on the CEF UI thread, which is the
        // GTK main thread (CEF is pumped from the GTK loop), so touching GTK
        // widgets here is safe.
        fn on_already_running_app_relaunch(
            &self,
            command_line: Option<&mut CommandLine>,
            _current_directory: Option<&CefString>,
        ) -> i32 {
            if let Some(cmd) = command_line {
                let mut list = CefStringList::default();
                cmd.argv(Some(&mut list));
                let args: Vec<String> = list.into_iter().collect();

                // The app id is the first non-flag, non-scheme arg (CEF injects
                // its own --switches; scheme handler URLs contain ':').
                let id = args
                    .iter()
                    .skip(1)
                    .find(|a| !a.starts_with('-') && !a.contains(':'));
                // An optional scheme URL arg (mailto:, webcal:, …) the relaunch
                // carried, expanded via the app's handlers like run() does.
                let url_arg = args
                    .iter()
                    .skip(1)
                    .find(|a| !a.starts_with('-') && a.contains(':'));

                if let Some(id) = id {
                    match WebApp::load(id) {
                        Ok(webapp) => {
                            let url_override = url_arg.and_then(|arg| {
                                let scheme = arg.split(':').next().unwrap_or("").to_string();
                                webapp
                                    .handlers
                                    .iter()
                                    .find(|h| h.scheme() == scheme)
                                    .map(|h| qwa_core::handlers::expand(&h.template, arg))
                            });
                            GTK_APP.with(|cell| {
                                if let Some(app) = cell.borrow().as_ref() {
                                    open_window(app, webapp, url_override);
                                } else {
                                    tracing::warn!("relaunch before GTK app ready; ignoring");
                                }
                            });
                        }
                        Err(e) => tracing::warn!("relaunch for unknown app '{id}': {e}"),
                    }
                }
            }
            1
        }
    }
}

wrap_render_handler! {
    struct OsrRenderHandler {
        shared: Rc<Shared>,
        area: gtk::DrawingArea,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(rect) = rect {
                rect.x = 0;
                rect.y = 0;
                rect.width = self.area.width().max(1);
                rect.height = self.area.height().max(1);
            }
        }

        // Report the display scale so CEF renders at physical resolution
        // (crisp on HiDPI / scaled displays); view_rect stays logical.
        fn screen_info(
            &self,
            _browser: Option<&mut Browser>,
            screen_info: Option<&mut ScreenInfo>,
        ) -> i32 {
            if let Some(info) = screen_info {
                // Prefer the GdkSurface's FRACTIONAL scale (GTK 4.12+) over the
                // widget's INTEGER scale_factor(). On compositors with fractional
                // scaling (e.g. Niri at 1.5x), scale_factor() reports 1 (it only
                // ever returns whole numbers), so CEF would render at the wrong
                // resolution and look blurry. gdk::Surface::scale() exposes the
                // true fractional scale GTK itself uses to stay crisp, so feeding
                // it to CEF makes off-screen rendering match. Fall back to the
                // integer widget scale when the surface scale is unavailable or
                // implausible — this keeps integer-scaled GNOME (which already
                // works) unchanged.
                let surface_scale = self
                    .area
                    .native()
                    .and_then(|n| n.surface())
                    .map(|s| s.scale());
                info.device_scale_factor = match surface_scale {
                    Some(s) if s.is_finite() && s >= 1.0 => s as f32,
                    _ => self.area.scale_factor().max(1) as f32,
                };
                info.depth = 24;
                info.depth_per_component = 8;
                let (w, h) = (self.area.width().max(1), self.area.height().max(1));
                info.rect = Rect { x: 0, y: 0, width: w, height: h };
                info.available_rect = Rect { x: 0, y: 0, width: w, height: h };
                return 1;
            }
            0
        }

        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: i32,
            height: i32,
        ) {
            // Only the main view layer for now (ignore popup/overlay layers).
            if type_ != PaintElementType::VIEW {
                return;
            }
            let len = (width as usize) * (height as usize) * 4;
            let buf = unsafe { std::slice::from_raw_parts(buffer, len) }.to_vec();
            static FIRST_PAINT: std::sync::Once = std::sync::Once::new();
            FIRST_PAINT.call_once(|| tracing::info!("first osr paint {width}x{height}"));
            *self.shared.frame.borrow_mut() = Some(Frame { buf, width, height });
            self.area.queue_draw();
        }
    }
}

wrap_life_span_handler! {
    struct OsrLifeSpanHandler {
        shared: Rc<Shared>,
        app: Rc<WebApp>,
    }

    impl LifeSpanHandler {
        #[allow(clippy::too_many_arguments)]
        fn on_before_popup(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            _popup_id: i32,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            user_gesture: i32,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            // Never spawn a separate window. Popups NOT from a user click are
            // background preloads / ads / trackers (e.g. Gmail opens
            // Meet/Drive/Tasks on load) — block them silently.
            if user_gesture == 0 {
                return 1;
            }
            let mut browser = browser;
            // User-initiated popup. Compare against the page we're on right now
            // (post-redirect, e.g. gmail.com -> mail.google.com):
            //   same-site (Drive/Docs/Calendar from the waffle) -> open it in
            //              this window so the click actually does something;
            //   external   -> open in the system browser.
            let url = target_url.map(|u| u.to_string()).unwrap_or_default();
            if !url.is_empty() {
                // If the target is itself one of the user's installed web apps,
                // open THAT app in its own window instead of here / the browser.
                if crate::app::route_to_installed_app(&url) {
                    return 1;
                }
                let mode = self.app.link_scope();
                let scope = self.app.scope.clone();
                let app_url = self.app.url.clone();
                let home = crate::app::current_page_url(browser.as_deref_mut()).unwrap_or(app_url);

                // A user-opened popup leaves for the system browser only when it
                // wouldn't stay in-window under this app's link-scope mode — the
                // same predicate as on_before_browse (identity/SSO/CAPTCHA and
                // in-scope hosts always stay; same-host pop-outs like compose
                // stay too).
                if !qwa_core::stays_in_window(&url, scope.as_deref(), &home, mode) {
                    if let Err(e) = open::that(&url) {
                        tracing::warn!("failed to open external url {url}: {e}");
                    }
                } else if let Some(frame) = browser.and_then(|b| b.main_frame()) {
                    frame.load_url(Some(&CefString::from(url.as_str())));
                }
            }
            1 // cancel the popup
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            if let Some(browser) = browser {
                tracing::info!("osr browser created");
                *self.shared.browser.borrow_mut() = Some(browser.clone());
            }
        }

        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            *self.shared.browser.borrow_mut() = None;
            // Drop this window's MPRIS player (unregister objects + bus name).
            mpris_remove(&self.app.id);
            // Drop this window's StatusNotifier tray item, if it had one.
            tray_remove(&self.app.id);
        }
    }
}

wrap_load_handler! {
    struct OsrLoadHandler {
        shared: Rc<Shared>,
        back: gtk::Button,
        forward: gtk::Button,
        app: Rc<WebApp>,
    }

    impl LoadHandler {
        fn on_loading_state_change(
            &self,
            browser: Option<&mut Browser>,
            is_loading: i32,
            can_go_back: i32,
            can_go_forward: i32,
        ) {
            self.back.set_sensitive(can_go_back != 0);
            self.forward.set_sensitive(can_go_forward != 0);

            let mut browser = browser;

            // Record the settled home once the first load completes, after any
            // initial redirect chain (gmail.com -> mail.google.com).
            if is_loading == 0 && self.shared.home.borrow().is_none() {
                if let Some(url) = crate::app::current_page_url(browser.as_deref_mut()) {
                    if url.starts_with("http") {
                        tracing::info!("settled home: {url}");
                        *self.shared.home.borrow_mut() = Some(url);
                    }
                }
                // Re-apply the restored zoom once the host is ready (CEF starts
                // each browser at 100%).
                let z = self.shared.zoom.get();
                if z != 0.0 {
                    with_host(&self.shared, |h| h.set_zoom_level(z));
                }
            }

            // Inject the app's per-app custom CSS and the MPRIS media bridge
            // after each page finishes loading.
            if is_loading == 0 {
                if let Some(frame) = browser.and_then(|b| b.main_frame()) {
                    // Per-app custom CSS: append a <style> element to the head.
                    if let Some(css) = self.app.custom_css.clone() {
                        let js = format!(
                            "(function(){{var s=document.createElement('style');\
                             s.textContent={};document.head.appendChild(s);}})();",
                            js_string_literal(&css),
                        );
                        frame.execute_java_script(Some(&CefString::from(js.as_str())), None, 0);
                    }

                    // MPRIS media bridge: report media-session state over the
                    // console channel (consumed by OsrDisplayHandler). Injected
                    // for every app — MPRIS is automatic — and self-guarded so a
                    // re-load doesn't install duplicate timers/listeners.
                    frame.execute_java_script(Some(&CefString::from(MEDIA_BRIDGE_JS)), None, 0);
                }
            }
        }
    }
}

/// Page-side media reporter. Reads `navigator.mediaSession` metadata + playback
/// state (falling back to any non-paused `<audio>/<video>`), and emits
/// `console.log('QWA_MEDIA:'+json)` only when the payload changes — on a ~1s
/// interval and on media `play`/`pause` events. Guarded by `window.__qwaMediaBridge`
/// so repeated injection (one per page load) installs it only once.
const MEDIA_BRIDGE_JS: &str = r#"(function(){
if(window.__qwaMediaBridge)return;window.__qwaMediaBridge=true;
var last=null;
function state(){
var ms=navigator.mediaSession,m=(ms&&ms.metadata)||null;
var playing=false;
var els=document.querySelectorAll('audio,video');
for(var i=0;i<els.length;i++){if(!els[i].paused&&!els[i].ended&&els[i].currentTime>0){playing=true;break;}}
if(!playing&&ms&&ms.playbackState==='playing')playing=true;
var art='';
if(m&&m.artwork&&m.artwork.length){art=m.artwork[m.artwork.length-1].src||'';}
return{playing:playing,title:(m&&m.title)||'',artist:(m&&m.artist)||'',album:(m&&m.album)||'',art:art};
}
function report(){
var s=state();var j=JSON.stringify(s);
if(j!==last){last=j;console.log('QWA_MEDIA:'+j);}
}
setInterval(report,1000);
document.addEventListener('play',report,true);
document.addEventListener('pause',report,true);
report();
})();"#;

wrap_request_handler! {
    struct OsrRequestHandler {
        shared: Rc<Shared>,
        scope: Option<String>,
        mode: qwa_core::LinkScope,
        adblock: bool,
    }

    impl RequestHandler {
        fn on_before_browse(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            request: Option<&mut Request>,
            user_gesture: i32,
            is_redirect: i32,
        ) -> i32 {
            let Some(request) = request else {
                return 0;
            };
            let url = CefString::from(&request.url()).to_string();
            let method = CefString::from(&request.method()).to_string();

            // Only top-level navigations participate in routing / browser
            // diversion. Embedded service panels (Gmail's Tasks/Keep/Calendar
            // side panels, in-page iframes, OAuth frames) are sub-frames and
            // MUST be allowed to load in place — otherwise clicking Tasks inside
            // Gmail would be cancelled here and the panel hangs/dies. (#19)
            let is_main_frame = frame.map(|f| f.is_main() == 1).unwrap_or(true);
            if !is_main_frame {
                return 0;
            }

            // A *deliberately clicked* top-level link to another installed web
            // app opens that app. Gate strictly on a real user gesture (and not
            // a redirect): otherwise an app like Gmail, which performs
            // background navigations to sibling Google services (Drive, Tasks,
            // Docs), would spuriously launch each of those installed apps. (#19)
            if user_gesture == 1 && is_redirect == 0 && crate::app::route_to_installed_app(&url) {
                return 1;
            }

            // InWindow mode never diverts — multi-domain logins (e.g. Microsoft:
            // outlook -> login.microsoftonline -> login.live) stay in-window.
            if self.mode == qwa_core::LinkScope::InWindow {
                return 0;
            }
            // Until the app settles on its home, allow everything (initial load
            // + redirects, possibly across domains).
            let home = match self.shared.home.borrow().clone() {
                Some(h) => h,
                None => return 0,
            };
            // Only eject a *deliberate* top-level GET that leaves the app per its
            // link-scope mode. Excluded so sign-in keeps working in-window:
            //   - non-GET (a form POST opened in the browser becomes a broken
            //     GET, e.g. Microsoft's AADSTS900561);
            //   - automatic cross-domain redirects (SSO token hops) — not
            //     gestures;
            //   - identity/SSO/CAPTCHA + in-scope hosts (stays_in_window).
            if user_gesture == 1
                && is_redirect == 0
                && method.eq_ignore_ascii_case("GET")
                && !qwa_core::stays_in_window(&url, self.scope.as_deref(), &home, self.mode)
            {
                if let Err(e) = open::that(&url) {
                    tracing::warn!("failed to open external url {url}: {e}");
                }
                return 1;
            }
            0
        }

        fn resource_request_handler(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            _request: Option<&mut Request>,
            _is_navigation: i32,
            _is_download: i32,
            _request_initiator: Option<&CefString>,
            _disable_default_handling: Option<&mut i32>,
        ) -> Option<ResourceRequestHandler> {
            // Only attach the blocker when this app has adblock enabled.
            self.adblock.then(OsrResourceRequestHandler::new)
        }
    }
}

wrap_resource_request_handler! {
    struct OsrResourceRequestHandler {}

    impl ResourceRequestHandler {
        fn on_before_resource_load(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            // NB: ReturnValue::default() is CANCEL, so allowed requests must
            // explicitly CONTINUE.
            if let Some(request) = request {
                let url = CefString::from(&request.url()).to_string();
                if qwa_core::adblock::is_blocked(&url) {
                    return ReturnValue::CANCEL;
                }
            }
            ReturnValue::CONTINUE
        }
    }
}

wrap_permission_handler! {
    struct OsrPermissionHandler {
        app: Rc<WebApp>,
    }

    impl PermissionHandler {
        fn on_show_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            _prompt_id: u64,
            _requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut PermissionPromptCallback>,
        ) -> i32 {
            // OSR has no native permission popup, so we decide from the app's
            // per-app policy instead of prompting. Low-risk capabilities
            // (notifications — our whole point — clipboard, etc.) are granted;
            // the sensitive ones (camera/mic, geolocation) are granted only when
            // the user opted in via the editor, else denied. Persisted in the
            // app config. (#22)
            let cam_mic = PermissionRequestTypes::CAMERA_STREAM.get_raw()
                | PermissionRequestTypes::MIC_STREAM.get_raw()
                | PermissionRequestTypes::CAMERA_PAN_TILT_ZOOM.get_raw();
            let geo = PermissionRequestTypes::GEOLOCATION.get_raw();

            let allow_cam_mic = self.app.allow_camera_mic;
            let allow_location = self.app.allow_location;

            let wants_cam_mic = requested_permissions & cam_mic != 0;
            let wants_geo = requested_permissions & geo != 0;
            let deny = (wants_cam_mic && !allow_cam_mic) || (wants_geo && !allow_location);

            if let Some(callback) = callback {
                let result = if deny {
                    tracing::info!(
                        "denied permission request per policy (bits={requested_permissions:#x})"
                    );
                    PermissionRequestResult::DENY
                } else {
                    PermissionRequestResult::ACCEPT
                };
                callback.cont(result);
            }
            1 // handled
        }

        // getUserMedia (camera/microphone) goes through a SEPARATE CEF callback
        // from the prompt above; if we don't handle it, CEF denies media access
        // by default — which is why video calls (Teams/Zoom/Meet) had no camera.
        // Grant the requested capture when the app's policy allows it. (#22)
        fn on_request_media_access_permission(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            _requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut MediaAccessCallback>,
        ) -> i32 {
            let allow = self.app.allow_camera_mic;
            if let Some(callback) = callback {
                // Grant exactly what was requested when allowed, else nothing.
                callback.cont(if allow { requested_permissions } else { 0 });
            }
            1 // handled
        }
    }
}

/// The directory downloads are saved to: the user's XDG Downloads dir when
/// resolvable, otherwise `$HOME/Downloads`. Created if it does not exist.
fn downloads_dir() -> std::path::PathBuf {
    let dir = std::env::var_os("XDG_DOWNLOAD_DIR")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            home.join("Downloads")
        });
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("failed to create downloads dir {}: {e}", dir.display());
    }
    dir
}

wrap_download_handler! {
    struct OsrDownloadHandler {}

    impl DownloadHandler {
        fn on_before_download(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> i32 {
            // Pick a file name: the suggested name from CEF, falling back to the
            // download item's own suggestion, then a generic default.
            let name = suggested_name
                .map(|n| n.to_string())
                .filter(|n| !n.is_empty())
                .or_else(|| {
                    download_item
                        .map(|item| CefString::from(&item.suggested_file_name()).to_string())
                        .filter(|n| !n.is_empty())
                })
                .unwrap_or_else(|| "download".to_string());

            let dest = downloads_dir().join(&name);
            let dest_str = dest.to_string_lossy().to_string();
            tracing::info!("download saving to {dest_str}");

            if let Some(callback) = callback {
                // cont(path, show_dialog): 0 = save directly to `path` without a
                // modal OS save dialog (OSR has no native dialog UI anyway).
                callback.cont(Some(&CefString::from(dest_str.as_str())), 0);
            }
            1 // handled
        }

        fn on_download_updated(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            _callback: Option<&mut DownloadItemCallback>,
        ) {
            if let Some(item) = download_item {
                if item.is_complete() == 1 {
                    let path = CefString::from(&item.full_path()).to_string();
                    tracing::info!("download complete: {path}");
                }
            }
        }
    }
}

wrap_client! {
    struct OsrClient {
        shared: Rc<Shared>,
        area: gtk::DrawingArea,
        back: gtk::Button,
        forward: gtk::Button,
        app: Rc<WebApp>,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(OsrRenderHandler::new(self.shared.clone(), self.area.clone()))
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(OsrLifeSpanHandler::new(self.shared.clone(), self.app.clone()))
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            // Per-window app context: derive routing policy from THIS window's
            // app, not current_app() (one process now hosts several apps).
            Some(OsrRequestHandler::new(
                self.shared.clone(),
                self.app.scope.clone(),
                self.app.link_scope(),
                self.app.adblock,
            ))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(OsrLoadHandler::new(
                self.shared.clone(),
                self.back.clone(),
                self.forward.clone(),
                self.app.clone(),
            ))
        }

        fn permission_handler(&self) -> Option<PermissionHandler> {
            Some(OsrPermissionHandler::new(self.app.clone()))
        }

        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(OsrDownloadHandler::new())
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            // Per-window app context drives this window's dock badge; the shared
            // browser state lets the MPRIS controls reach this window's host.
            Some(OsrDisplayHandler::new(self.app.clone(), self.shared.clone()))
        }
    }
}

thread_local! {
    /// The running GApplication, stashed so the BrowserProcessHandler relaunch
    /// callback (which has no other handle to it) can open windows on it.
    static GTK_APP: RefCell<Option<adw::Application>> = const { RefCell::new(None) };

    /// The session-bus connection, opened once at process start, used to emit
    /// per-app dock-badge updates (Unity LauncherEntry). `None` when the bus is
    /// unavailable — badge updates are then silently skipped.
    static DBUS: RefCell<Option<gtk::gio::DBusConnection>> = const { RefCell::new(None) };

    /// Per-window MPRIS players, keyed by the window's app id. Created lazily on
    /// the first `QWA_MEDIA:` console message a window reports, removed when the
    /// window closes. The registered D-Bus method-call/get-property closures
    /// capture only this `String` key (which is `Send`) and look the player up
    /// here on the GTK main thread — the thread all gio D-Bus callbacks land on,
    /// the same thread CEF is pumped from — so they never need to capture the
    /// non-`Send` `Rc<Shared>`/`Browser` directly.
    static PLAYERS: RefCell<HashMap<String, MprisPlayer>> = RefCell::new(HashMap::new());

    /// Per-window StatusNotifier tray items, keyed by the window's app id.
    /// Registered when a background-mode window opens, removed when it closes.
    /// Mirrors PLAYERS: the registered D-Bus closures capture only the `String`
    /// app id and resolve the item via this registry on the GTK main thread.
    static TRAYS: RefCell<HashMap<String, TrayItem>> = RefCell::new(HashMap::new());

    /// Monotonic counter making each tray item's bus name unique within this
    /// process (`org.kde.StatusNotifierItem-<pid>-<n>`).
    static TRAY_SEQ: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

wrap_display_handler! {
    struct OsrDisplayHandler {
        app: Rc<WebApp>,
        shared: Rc<Shared>,
    }

    impl DisplayHandler {
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            // Only drive the dock badge when this app opted in.
            if !self.app.show_badge {
                return;
            }
            // Parse the unread count from the live page title; an absent/zero
            // count clears the badge rather than leaving a stale number.
            let count = title
                .map(|t| t.to_string())
                .and_then(|t| qwa_core::badge::count_from_title(&t))
                .unwrap_or(0);
            emit_badge(&self.app.id, count);
        }

        // The JS-to-Rust channel for media state. The injected media bridge
        // (see OsrLoadHandler) logs `QWA_MEDIA:{json}` whenever the page's media
        // session metadata or playing flag changes; we parse that here and push
        // it into this window's MPRIS player (created lazily on first message),
        // emitting a Properties.PropertiesChanged so GNOME's lock screen / media
        // keys / Quick Settings reflect it. Returns 0 to NOT suppress the message
        // (so it still reaches the devtools console).
        fn on_console_message(
            &self,
            _browser: Option<&mut Browser>,
            _level: LogSeverity,
            message: Option<&CefString>,
            _source: Option<&CefString>,
            _line: i32,
        ) -> i32 {
            if let Some(msg) = message.map(|m| m.to_string()) {
                if let Some(payload) = msg.strip_prefix("QWA_MEDIA:") {
                    if let Some(state) = parse_media_payload(payload) {
                        mpris_update(&self.app, &self.shared, state);
                    }
                }
            }
            0
        }
    }
}

/// The parsed media state a `QWA_MEDIA:` console message carries.
#[derive(Default, Clone, PartialEq)]
struct MediaState {
    playing: bool,
    title: String,
    artist: String,
    album: String,
    art: String,
}

/// Hand-parse the tiny JSON object the media bridge emits. The bridge controls
/// the exact shape — a flat object of one bool (`playing`) and four strings
/// (`title`, `artist`, `album`, `art`) — so a minimal field extractor suffices
/// and avoids a serde_json dependency. Malformed payloads yield `None`.
fn parse_media_payload(json: &str) -> Option<MediaState> {
    // Must look like an object; reject obvious garbage.
    let trimmed = json.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    Some(MediaState {
        playing: json_bool(trimmed, "playing"),
        title: json_string(trimmed, "title"),
        artist: json_string(trimmed, "artist"),
        album: json_string(trimmed, "album"),
        art: json_string(trimmed, "art"),
    })
}

/// Extract `"key":true|false` from a flat JSON object.
fn json_bool(json: &str, key: &str) -> bool {
    let needle = format!("\"{key}\"");
    if let Some(pos) = json.find(&needle) {
        let rest = &json[pos + needle.len()..];
        // skip whitespace + ':'
        let rest = rest.trim_start();
        let rest = rest.strip_prefix(':').unwrap_or(rest).trim_start();
        return rest.starts_with("true");
    }
    false
}

/// Extract `"key":"value"` from a flat JSON object, decoding the handful of
/// escapes the JS `JSON.stringify` may emit (\\ \" \n \r \t and \uXXXX).
fn json_string(json: &str, key: &str) -> String {
    let needle = format!("\"{key}\"");
    let Some(pos) = json.find(&needle) else {
        return String::new();
    };
    let rest = &json[pos + needle.len()..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(':').unwrap_or(rest).trim_start();
    let Some(rest) = rest.strip_prefix('"') else {
        return String::new();
    };
    // Walk to the closing unescaped quote, decoding escapes as we go.
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some('u') => {
                    // Read 4 hex digits into a code unit; emit the char if valid.
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            out.push(ch);
                        }
                    }
                }
                Some(other) => out.push(other),
                None => break,
            },
            _ => out.push(c),
        }
    }
    out
}

/// Emit a Unity LauncherEntry `Update` signal so docks/panels that honour the
/// API show `count` on this app's launcher. Best-effort: does nothing when the
/// session bus is unavailable.
fn emit_badge(app_id: &str, count: u32) {
    use gtk::glib::prelude::ToVariant;

    DBUS.with(|c| {
        if let Some(conn) = c.borrow().as_ref() {
            let app_uri = format!("application://{}.{}.desktop", qwa_core::APP_ID, app_id);
            let mut props: HashMap<String, gtk::glib::Variant> = HashMap::new();
            props.insert("count".to_string(), (count as i64).to_variant());
            props.insert("count-visible".to_string(), (count > 0).to_variant());
            let params = (app_uri, props).to_variant();
            if let Err(e) = conn.emit_signal(
                None,
                "/com/canonical/Unity/LauncherEntry",
                "com.canonical.Unity.LauncherEntry",
                "Update",
                Some(&params),
            ) {
                tracing::debug!("failed to emit launcher badge update: {e}");
            }
        }
    });
}

// ---------------------------------------------------------------------------
// MPRIS media controls (per window)
//
// Each window that reports media state owns a standard MPRIS player on the
// session bus (`org.mpris.MediaPlayer2.qwa_<app-id>`) implementing the two
// org.mpris.MediaPlayer2{,.Player} interfaces. GNOME's lock screen, media keys
// and Quick Settings see/control it.
//
// Metadata IN: the page-side bridge (MEDIA_BRIDGE_JS) logs `QWA_MEDIA:{json}`,
//   OsrDisplayHandler::on_console_message parses it and calls `mpris_update`,
//   which stores the new state in PLAYERS and emits Properties.PropertiesChanged.
// Control OUT: MPRIS Play/Pause/Next/... method calls land on the GTK main
//   thread, look this window's player up in PLAYERS by app id, and synthesize
//   the matching media key on the browser host (`with_host(... send_key_event)`),
//   which Chromium routes to the page's media session — site-agnostic.
//
// `Send`: the gio register_object/own_name closures only require `Fn + 'static`
//   (gio wraps them with `Closure::new_local`), but to also satisfy the C-side
//   `Send + Sync` contract and keep the non-`Send` `Rc<Shared>` out of them,
//   they capture only the `String` app id and resolve everything via PLAYERS on
//   the GTK thread.
// ---------------------------------------------------------------------------

/// One window's MPRIS presence: its current state plus the registration ids
/// needed to tear it down on close, plus the window's `Rc<Shared>` so the
/// control methods can reach the browser host (and the window for Raise/Quit).
struct MprisPlayer {
    state: MediaState,
    /// App display name, surfaced as the MPRIS `Identity` / used in `DesktopEntry`.
    name: String,
    /// `<APP_ID>.<id>` — the installed desktop file id (no `.desktop` suffix).
    desktop_entry: String,
    shared: Rc<Shared>,
    owner_id: gio::OwnerId,
    root_reg: gio::RegistrationId,
    player_reg: gio::RegistrationId,
}

/// The fixed MPRIS object path every player registers its interfaces at.
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";

/// Introspection XML describing the two MPRIS interfaces we implement. Only the
/// members we actually back are declared; the rest (TrackList, Playlists) are
/// intentionally absent.
const MPRIS_XML: &str = r#"<node>
  <interface name="org.mpris.MediaPlayer2">
    <method name="Raise"/>
    <method name="Quit"/>
    <property name="CanQuit" type="b" access="read"/>
    <property name="CanRaise" type="b" access="read"/>
    <property name="HasTrackList" type="b" access="read"/>
    <property name="Identity" type="s" access="read"/>
    <property name="DesktopEntry" type="s" access="read"/>
    <property name="SupportedUriSchemes" type="as" access="read"/>
    <property name="SupportedMimeTypes" type="as" access="read"/>
  </interface>
  <interface name="org.mpris.MediaPlayer2.Player">
    <method name="Next"/>
    <method name="Previous"/>
    <method name="Pause"/>
    <method name="PlayPause"/>
    <method name="Stop"/>
    <method name="Play"/>
    <method name="Seek"><arg direction="in" name="Offset" type="x"/></method>
    <method name="SetPosition">
      <arg direction="in" name="TrackId" type="o"/>
      <arg direction="in" name="Position" type="x"/>
    </method>
    <method name="OpenUri"><arg direction="in" name="Uri" type="s"/></method>
    <property name="PlaybackStatus" type="s" access="read"/>
    <property name="Rate" type="d" access="readwrite"/>
    <property name="Metadata" type="a{sv}" access="read"/>
    <property name="Volume" type="d" access="readwrite"/>
    <property name="Position" type="x" access="read"/>
    <property name="MinimumRate" type="d" access="read"/>
    <property name="MaximumRate" type="d" access="read"/>
    <property name="CanGoNext" type="b" access="read"/>
    <property name="CanGoPrevious" type="b" access="read"/>
    <property name="CanPlay" type="b" access="read"/>
    <property name="CanPause" type="b" access="read"/>
    <property name="CanSeek" type="b" access="read"/>
    <property name="CanControl" type="b" access="read"/>
  </interface>
</node>"#;

/// `PlaybackStatus` string for the current playing flag.
fn playback_status(playing: bool) -> &'static str {
    if playing {
        "Playing"
    } else {
        "Paused"
    }
}

/// Build the MPRIS `Metadata` `a{sv}` variant from the current state. Empty
/// fields are omitted (a track id is always present so players have a key).
fn build_metadata(state: &MediaState) -> gtk::glib::Variant {
    use gtk::glib::prelude::ToVariant;
    let mut m: HashMap<String, gtk::glib::Variant> = HashMap::new();
    // mpris:trackid must be an object path; use a fixed synthetic one.
    if let Ok(path) = gtk::glib::variant::ObjectPath::try_from("/org/qwa/track") {
        m.insert("mpris:trackid".to_string(), path.to_variant());
    }
    if !state.title.is_empty() {
        m.insert("xesam:title".to_string(), state.title.to_variant());
    }
    if !state.artist.is_empty() {
        // xesam:artist is a string array (`as`).
        m.insert(
            "xesam:artist".to_string(),
            vec![state.artist.clone()].to_variant(),
        );
    }
    if !state.album.is_empty() {
        m.insert("xesam:album".to_string(), state.album.to_variant());
    }
    if !state.art.is_empty() {
        m.insert("mpris:artUrl".to_string(), state.art.to_variant());
    }
    m.to_variant()
}

/// Look up an MPRIS property value for the player keyed by `app_id`. Returns the
/// variant the introspected type expects. Unknown properties fall back to an
/// empty string (gio surfaces a not-found error to the caller anyway).
fn mpris_get_property(app_id: &str, interface: &str, prop: &str) -> gtk::glib::Variant {
    use gtk::glib::prelude::ToVariant;
    let empty_strs: Vec<String> = Vec::new();
    PLAYERS.with(|players| {
        let players = players.borrow();
        let player = players.get(app_id);
        match (interface, prop) {
            ("org.mpris.MediaPlayer2", "Identity") => player
                .map(|p| p.name.clone())
                .unwrap_or_default()
                .to_variant(),
            ("org.mpris.MediaPlayer2", "DesktopEntry") => player
                .map(|p| p.desktop_entry.clone())
                .unwrap_or_default()
                .to_variant(),
            ("org.mpris.MediaPlayer2", "CanQuit") => true.to_variant(),
            ("org.mpris.MediaPlayer2", "CanRaise") => true.to_variant(),
            ("org.mpris.MediaPlayer2", "HasTrackList") => false.to_variant(),
            ("org.mpris.MediaPlayer2", "SupportedUriSchemes") => empty_strs.to_variant(),
            ("org.mpris.MediaPlayer2", "SupportedMimeTypes") => empty_strs.to_variant(),
            ("org.mpris.MediaPlayer2.Player", "PlaybackStatus") => {
                playback_status(player.map(|p| p.state.playing).unwrap_or(false)).to_variant()
            }
            ("org.mpris.MediaPlayer2.Player", "Metadata") => match player {
                Some(p) => build_metadata(&p.state),
                None => build_metadata(&MediaState::default()),
            },
            ("org.mpris.MediaPlayer2.Player", "CanGoNext")
            | ("org.mpris.MediaPlayer2.Player", "CanGoPrevious")
            | ("org.mpris.MediaPlayer2.Player", "CanPlay")
            | ("org.mpris.MediaPlayer2.Player", "CanPause")
            | ("org.mpris.MediaPlayer2.Player", "CanControl") => true.to_variant(),
            ("org.mpris.MediaPlayer2.Player", "CanSeek") => false.to_variant(),
            ("org.mpris.MediaPlayer2.Player", "Rate")
            | ("org.mpris.MediaPlayer2.Player", "MinimumRate")
            | ("org.mpris.MediaPlayer2.Player", "MaximumRate")
            | ("org.mpris.MediaPlayer2.Player", "Volume") => 1.0f64.to_variant(),
            ("org.mpris.MediaPlayer2.Player", "Position") => 0i64.to_variant(),
            _ => String::new().to_variant(),
        }
    })
}

/// Synthesize a media key on the player's browser host. Chromium routes media
/// keys to the page's registered media-session action handlers, so this works
/// without any per-site JS. `vk` is a Windows VK media key code.
fn mpris_send_media_key(app_id: &str, vk: i32) {
    PLAYERS.with(|players| {
        if let Some(player) = players.borrow().get(app_id) {
            with_host(&player.shared, |h| {
                h.send_key_event(Some(&KeyEvent {
                    type_: KeyEventType::RAWKEYDOWN,
                    windows_key_code: vk,
                    ..Default::default()
                }));
                h.send_key_event(Some(&KeyEvent {
                    type_: KeyEventType::KEYUP,
                    windows_key_code: vk,
                    ..Default::default()
                }));
            });
        }
    });
}

/// Handle an MPRIS method call for the player keyed by `app_id`.
fn mpris_method_call(app_id: &str, interface: &str, method: &str) {
    // Windows VK media keys (what CEF's send_key_event expects).
    const VK_MEDIA_NEXT_TRACK: i32 = 0xB0;
    const VK_MEDIA_PREV_TRACK: i32 = 0xB1;
    const VK_MEDIA_STOP: i32 = 0xB2;
    const VK_MEDIA_PLAY_PAUSE: i32 = 0xB3;

    match (interface, method) {
        ("org.mpris.MediaPlayer2", "Raise") => PLAYERS.with(|players| {
            if let Some(player) = players.borrow().get(app_id) {
                if let Some(win) = player
                    .shared
                    .window
                    .borrow()
                    .as_ref()
                    .and_then(|w| w.upgrade())
                {
                    win.set_visible(true);
                    win.present();
                }
            }
        }),
        ("org.mpris.MediaPlayer2", "Quit") => PLAYERS.with(|players| {
            if let Some(player) = players.borrow().get(app_id) {
                if let Some(win) = player
                    .shared
                    .window
                    .borrow()
                    .as_ref()
                    .and_then(|w| w.upgrade())
                {
                    win.close();
                }
            }
        }),
        ("org.mpris.MediaPlayer2.Player", "Play")
        | ("org.mpris.MediaPlayer2.Player", "Pause")
        | ("org.mpris.MediaPlayer2.Player", "PlayPause") => {
            mpris_send_media_key(app_id, VK_MEDIA_PLAY_PAUSE)
        }
        ("org.mpris.MediaPlayer2.Player", "Next") => {
            mpris_send_media_key(app_id, VK_MEDIA_NEXT_TRACK)
        }
        ("org.mpris.MediaPlayer2.Player", "Previous") => {
            mpris_send_media_key(app_id, VK_MEDIA_PREV_TRACK)
        }
        ("org.mpris.MediaPlayer2.Player", "Stop") => mpris_send_media_key(app_id, VK_MEDIA_STOP),
        // Seek/SetPosition/OpenUri are declared but no-ops (CanSeek=false).
        _ => {}
    }
}

/// Emit Properties.PropertiesChanged for the Player interface so subscribers
/// (lock screen, Quick Settings) pick up the new PlaybackStatus + Metadata.
fn mpris_emit_properties_changed(conn: &gio::DBusConnection, state: &MediaState) {
    use gtk::glib::prelude::ToVariant;
    let mut changed: HashMap<String, gtk::glib::Variant> = HashMap::new();
    changed.insert(
        "PlaybackStatus".to_string(),
        playback_status(state.playing).to_variant(),
    );
    changed.insert("Metadata".to_string(), build_metadata(state));
    let invalidated: Vec<String> = Vec::new();
    let params = (
        "org.mpris.MediaPlayer2.Player".to_string(),
        changed,
        invalidated,
    )
        .to_variant();
    if let Err(e) = conn.emit_signal(
        None,
        MPRIS_PATH,
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
        Some(&params),
    ) {
        tracing::debug!("failed to emit MPRIS PropertiesChanged: {e}");
    }
}

/// Update (or lazily create) this window's MPRIS player with new media state,
/// then signal the change to the bus. Best-effort: a missing session bus or a
/// registration failure logs and leaves the app running normally.
fn mpris_update(app: &WebApp, shared: &Rc<Shared>, state: MediaState) {
    DBUS.with(|c| {
        let Some(conn) = c.borrow().clone() else {
            return; // no session bus -> no MPRIS, app still runs
        };

        let app_id = app.id.clone();
        let exists = PLAYERS.with(|p| p.borrow().contains_key(&app_id));

        if !exists {
            // First media message for this window: register the player.
            let node = match gio::DBusNodeInfo::for_xml(MPRIS_XML) {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!("invalid MPRIS introspection xml: {e}");
                    return;
                }
            };
            let Some(root_iface) = node.lookup_interface("org.mpris.MediaPlayer2") else {
                return;
            };
            let Some(player_iface) = node.lookup_interface("org.mpris.MediaPlayer2.Player") else {
                return;
            };

            // org.mpris.MediaPlayer2 (root): Raise/Quit + identity properties.
            let root_reg = {
                let id_for_call = app_id.clone();
                let id_for_get = app_id.clone();
                match conn
                    .register_object(MPRIS_PATH, &root_iface)
                    .method_call(move |_conn, _sender, _path, iface, method, _params, inv| {
                        mpris_method_call(&id_for_call, iface.unwrap_or_default(), method);
                        inv.return_value(None);
                    })
                    .property(move |_conn, _sender, _path, iface, prop| {
                        mpris_get_property(&id_for_get, iface, prop)
                    })
                    .build()
                {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::warn!("failed to register MPRIS root object: {e}");
                        return;
                    }
                }
            };

            // org.mpris.MediaPlayer2.Player: transport methods + state properties.
            let player_reg = {
                let id_for_call = app_id.clone();
                let id_for_get = app_id.clone();
                match conn
                    .register_object(MPRIS_PATH, &player_iface)
                    .method_call(move |_conn, _sender, _path, iface, method, _params, inv| {
                        mpris_method_call(&id_for_call, iface.unwrap_or_default(), method);
                        inv.return_value(None);
                    })
                    .property(move |_conn, _sender, _path, iface, prop| {
                        mpris_get_property(&id_for_get, iface, prop)
                    })
                    .build()
                {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::warn!("failed to register MPRIS player object: {e}");
                        let _ = conn.unregister_object(root_reg);
                        return;
                    }
                }
            };

            // Own the well-known bus name for this player.
            let bus_name = format!("org.mpris.MediaPlayer2.qwa_{}", sanitize_app_id(&app_id));
            let owner_id = gio::bus_own_name_on_connection(
                &conn,
                &bus_name,
                gio::BusNameOwnerFlags::NONE,
                |_c, _n| {},
                |_c, _n| {},
            );

            PLAYERS.with(|players| {
                players.borrow_mut().insert(
                    app_id.clone(),
                    MprisPlayer {
                        state: MediaState::default(),
                        name: app.name.clone(),
                        desktop_entry: format!("{}.{}", qwa_core::APP_ID, app_id),
                        shared: shared.clone(),
                        owner_id,
                        root_reg,
                        player_reg,
                    },
                );
            });
            tracing::info!("registered MPRIS player {bus_name}");
        }

        // Store the new state; skip the signal when nothing actually changed.
        let changed = PLAYERS.with(|players| {
            let mut players = players.borrow_mut();
            if let Some(player) = players.get_mut(&app_id) {
                if player.state == state {
                    return false;
                }
                player.state = state.clone();
                true
            } else {
                false
            }
        });

        if changed {
            mpris_emit_properties_changed(&conn, &state);
        }
    });
}

/// Tear down a window's MPRIS player when the window closes: drop its registry
/// entry, unregister both objects, and release the bus name.
fn mpris_remove(app_id: &str) {
    let player = PLAYERS.with(|players| players.borrow_mut().remove(app_id));
    if let Some(player) = player {
        DBUS.with(|c| {
            if let Some(conn) = c.borrow().as_ref() {
                let _ = conn.unregister_object(player.root_reg);
                let _ = conn.unregister_object(player.player_reg);
            }
        });
        gio::bus_unown_name(player.owner_id);
        tracing::info!("removed MPRIS player for {app_id}");
    }
}

// ---------------------------------------------------------------------------
// StatusNotifier tray icon (per background-mode window)
//
// Background-mode apps get a tray icon (org.kde.StatusNotifierItem) with a tiny
// DBusMenu ("Show" / "Quit"), so they can be re-shown after the window is
// hidden and genuinely quit. The item is registered with the
// org.kde.StatusNotifierWatcher; without a watcher (bare GNOME with no
// AppIndicator extension) registration just no-ops and the app runs normally.
//
// Same `Send` strategy as MPRIS: the gio register_object closures capture only
// the window's `String` app id and resolve the live window via TRAYS on the GTK
// main thread (where all gio D-Bus callbacks land). Activate and the menu's
// Show/Quit reach the window through TrayItem's `Rc<Shared>` weak window handle.
// ---------------------------------------------------------------------------

/// One window's tray presence: the registration ids + bus name needed to tear
/// it down, plus the window's `Rc<Shared>` so Activate / menu actions can
/// present or close it, and the display fields the SNI properties expose.
struct TrayItem {
    name: String,
    icon_name: String,
    desktop_entry: String,
    shared: Rc<Shared>,
    owner_id: gio::OwnerId,
    item_reg: gio::RegistrationId,
    menu_reg: gio::RegistrationId,
}

/// Object paths the SNI + DBusMenu interfaces are registered at.
const SNI_ITEM_PATH: &str = "/StatusNotifierItem";
const SNI_MENU_PATH: &str = "/MenuBar";

/// Menu item ids the DBusMenu exposes (0 is the implicit root).
const MENU_ID_SHOW: i32 = 1;
const MENU_ID_QUIT: i32 = 2;

/// Introspection XML for the org.kde.StatusNotifierItem interface. Only the
/// members GNOME/AppIndicator/KDE actually read are declared.
const SNI_XML: &str = r#"<node>
  <interface name="org.kde.StatusNotifierItem">
    <property name="Category" type="s" access="read"/>
    <property name="Id" type="s" access="read"/>
    <property name="Title" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="IconName" type="s" access="read"/>
    <property name="IconThemePath" type="s" access="read"/>
    <property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
    <property name="ItemIsMenu" type="b" access="read"/>
    <property name="Menu" type="o" access="read"/>
    <method name="Activate">
      <arg name="x" type="i" direction="in"/>
      <arg name="y" type="i" direction="in"/>
    </method>
    <method name="SecondaryActivate">
      <arg name="x" type="i" direction="in"/>
      <arg name="y" type="i" direction="in"/>
    </method>
    <method name="ContextMenu">
      <arg name="x" type="i" direction="in"/>
      <arg name="y" type="i" direction="in"/>
    </method>
    <method name="Scroll">
      <arg name="delta" type="i" direction="in"/>
      <arg name="orientation" type="s" direction="in"/>
    </method>
    <signal name="NewIcon"/>
    <signal name="NewToolTip"/>
    <signal name="NewStatus"><arg name="status" type="s"/></signal>
  </interface>
</node>"#;

/// Introspection XML for the minimal com.canonical.dbusmenu we back: a flat
/// two-entry menu ("Show", "Quit") via GetLayout + Event + AboutToShow.
const DBUSMENU_XML: &str = r#"<node>
  <interface name="com.canonical.dbusmenu">
    <property name="Version" type="u" access="read"/>
    <property name="Status" type="s" access="read"/>
    <method name="GetLayout">
      <arg name="parentId" type="i" direction="in"/>
      <arg name="recursionDepth" type="i" direction="in"/>
      <arg name="propertyNames" type="as" direction="in"/>
      <arg name="revision" type="u" direction="out"/>
      <arg name="layout" type="(ia{sv}av)" direction="out"/>
    </method>
    <method name="GetGroupProperties">
      <arg name="ids" type="ai" direction="in"/>
      <arg name="propertyNames" type="as" direction="in"/>
      <arg name="properties" type="a(ia{sv})" direction="out"/>
    </method>
    <method name="Event">
      <arg name="id" type="i" direction="in"/>
      <arg name="eventId" type="s" direction="in"/>
      <arg name="data" type="v" direction="in"/>
      <arg name="timestamp" type="u" direction="in"/>
    </method>
    <method name="AboutToShow">
      <arg name="id" type="i" direction="in"/>
      <arg name="needUpdate" type="b" direction="out"/>
    </method>
    <signal name="LayoutUpdated">
      <arg name="revision" type="u"/>
      <arg name="parent" type="i"/>
    </signal>
  </interface>
</node>"#;

/// Look up an SNI property for the tray item keyed by `app_id`.
fn sni_get_property(app_id: &str, prop: &str) -> gtk::glib::Variant {
    use gtk::glib::prelude::ToVariant;
    TRAYS.with(|trays| {
        let trays = trays.borrow();
        let item = trays.get(app_id);
        match prop {
            "Category" => "ApplicationStatus".to_variant(),
            "Id" => app_id.to_variant(),
            "Title" => item
                .map(|i| i.name.clone())
                .unwrap_or_default()
                .to_variant(),
            "Status" => "Active".to_variant(),
            "IconName" => item
                .map(|i| i.icon_name.clone())
                .unwrap_or_else(|| "applications-internet".to_string())
                .to_variant(),
            "IconThemePath" => String::new().to_variant(),
            "ToolTip" => {
                // (sa(iiay)ss): icon-name, icon-data (empty), title, body.
                let icon_name = item
                    .map(|i| i.icon_name.clone())
                    .unwrap_or_else(|| "applications-internet".to_string());
                let title = item.map(|i| i.name.clone()).unwrap_or_default();
                let icon_data: Vec<(i32, i32, Vec<u8>)> = Vec::new();
                (icon_name, icon_data, title, String::new()).to_variant()
            }
            "ItemIsMenu" => false.to_variant(),
            "Menu" => gtk::glib::variant::ObjectPath::try_from(SNI_MENU_PATH)
                .map(|p| p.to_variant())
                .unwrap_or_else(|_| SNI_MENU_PATH.to_variant()),
            _ => String::new().to_variant(),
        }
    })
}

/// Present (show + raise) the window for the tray item keyed by `app_id`.
fn tray_show(app_id: &str) {
    TRAYS.with(|trays| {
        if let Some(item) = trays.borrow().get(app_id) {
            if let Some(win) = item
                .shared
                .window
                .borrow()
                .as_ref()
                .and_then(|w| w.upgrade())
            {
                win.set_visible(true);
                win.present();
            }
        }
    });
}

/// Close the window for the tray item keyed by `app_id` for real. The window's
/// close handler hides background apps, so force it shut by clearing the
/// background flag is not possible here; instead destroy the window directly.
fn tray_quit(app_id: &str) {
    TRAYS.with(|trays| {
        if let Some(item) = trays.borrow().get(app_id) {
            if let Some(win) = item
                .shared
                .window
                .borrow()
                .as_ref()
                .and_then(|w| w.upgrade())
            {
                // `destroy` bypasses the close-request handler (which would just
                // hide a background app), so the window genuinely goes away and
                // its on_before_close tears the browser + tray down.
                win.destroy();
            }
        }
    });
}

/// Build the DBusMenu `GetLayout` return value: `(u(ia{sv}av))` — a revision and
/// a root node whose children are the "Show" and "Quit" entries.
fn dbusmenu_layout() -> gtk::glib::Variant {
    use gtk::glib::prelude::ToVariant;

    // One leaf menu item: (id, properties a{sv}, children av [empty]).
    fn entry(id: i32, label: &str) -> gtk::glib::Variant {
        let mut props: HashMap<String, gtk::glib::Variant> = HashMap::new();
        props.insert("label".to_string(), label.to_variant());
        props.insert("enabled".to_string(), true.to_variant());
        props.insert("visible".to_string(), true.to_variant());
        let children: Vec<gtk::glib::Variant> = Vec::new();
        (id, props, children).to_variant()
    }

    // Root node (id 0) with the two entries as children. Children are `av`, so
    // each child node is boxed in a variant.
    let mut root_props: HashMap<String, gtk::glib::Variant> = HashMap::new();
    root_props.insert(
        "children-display".to_string(),
        "submenu".to_string().to_variant(),
    );
    let children = vec![
        gtk::glib::Variant::from_variant(&entry(MENU_ID_SHOW, "Show")),
        gtk::glib::Variant::from_variant(&entry(MENU_ID_QUIT, "Quit")),
    ];
    let root = (0i32, root_props, children).to_variant();
    (0u32, root).to_variant()
}

/// Handle a DBusMenu `Event`: a "clicked" on Show/Quit drives the window.
fn dbusmenu_event(app_id: &str, id: i32, event_id: &str) {
    if event_id != "clicked" {
        return;
    }
    match id {
        MENU_ID_SHOW => tray_show(app_id),
        MENU_ID_QUIT => tray_quit(app_id),
        _ => {}
    }
}

/// Register a StatusNotifier tray item for a background-mode window. Best-effort:
/// any failure (no session bus, registration error, absent watcher) logs and
/// leaves the app running normally.
fn tray_register(app: &WebApp, shared: &Rc<Shared>) {
    DBUS.with(|c| {
        let Some(conn) = c.borrow().clone() else {
            return; // no session bus -> no tray, app still runs
        };

        let app_id = app.id.clone();
        if TRAYS.with(|t| t.borrow().contains_key(&app_id)) {
            return;
        }

        let item_node = match gio::DBusNodeInfo::for_xml(SNI_XML) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("invalid SNI introspection xml: {e}");
                return;
            }
        };
        let menu_node = match gio::DBusNodeInfo::for_xml(DBUSMENU_XML) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("invalid dbusmenu introspection xml: {e}");
                return;
            }
        };
        let Some(item_iface) = item_node.lookup_interface("org.kde.StatusNotifierItem") else {
            return;
        };
        let Some(menu_iface) = menu_node.lookup_interface("com.canonical.dbusmenu") else {
            return;
        };

        // org.kde.StatusNotifierItem: Activate/menu pointers + properties.
        let item_reg = {
            let id_for_call = app_id.clone();
            let id_for_get = app_id.clone();
            match conn
                .register_object(SNI_ITEM_PATH, &item_iface)
                .method_call(move |_conn, _sender, _path, _iface, method, _params, inv| {
                    // Activate / SecondaryActivate present the window; the rest
                    // (ContextMenu/Scroll) are no-ops.
                    if method == "Activate" || method == "SecondaryActivate" {
                        tray_show(&id_for_call);
                    }
                    inv.return_value(None);
                })
                .property(move |_conn, _sender, _path, _iface, prop| {
                    sni_get_property(&id_for_get, prop)
                })
                .build()
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::warn!("failed to register SNI item object: {e}");
                    return;
                }
            }
        };

        // com.canonical.dbusmenu: GetLayout / Event / AboutToShow.
        let menu_reg = {
            let id_for_call = app_id.clone();
            match conn
                .register_object(SNI_MENU_PATH, &menu_iface)
                .method_call(move |_conn, _sender, _path, _iface, method, params, inv| {
                    match method {
                        "GetLayout" => inv.return_value(Some(&dbusmenu_layout())),
                        "GetGroupProperties" => {
                            // Return an empty a(ia{sv}); clients re-query via
                            // GetLayout, which carries the labels we need.
                            use gtk::glib::prelude::ToVariant;
                            let empty: Vec<(i32, HashMap<String, gtk::glib::Variant>)> = Vec::new();
                            inv.return_value(Some(&(empty,).to_variant()));
                        }
                        "Event" => {
                            // params: (i id, s eventId, v data, u timestamp).
                            let id: i32 = params.try_child_get(0).ok().flatten().unwrap_or(0);
                            let event_id: String =
                                params.try_child_get(1).ok().flatten().unwrap_or_default();
                            dbusmenu_event(&id_for_call, id, &event_id);
                            inv.return_value(None);
                        }
                        "AboutToShow" => {
                            use gtk::glib::prelude::ToVariant;
                            inv.return_value(Some(&(false,).to_variant()));
                        }
                        _ => inv.return_value(None),
                    }
                })
                .property(|_conn, _sender, _path, _iface, prop| {
                    use gtk::glib::prelude::ToVariant;
                    match prop {
                        "Version" => 3u32.to_variant(),
                        "Status" => "normal".to_string().to_variant(),
                        _ => String::new().to_variant(),
                    }
                })
                .build()
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::warn!("failed to register dbusmenu object: {e}");
                    let _ = conn.unregister_object(item_reg);
                    return;
                }
            }
        };

        // Own a unique bus name and register it with the watcher.
        let seq = TRAY_SEQ.with(|s| {
            let n = s.get();
            s.set(n + 1);
            n
        });
        let pid = std::process::id();
        let bus_name = format!("org.kde.StatusNotifierItem-{pid}-{seq}");
        let owner_id = gio::bus_own_name_on_connection(
            &conn,
            &bus_name,
            gio::BusNameOwnerFlags::NONE,
            {
                // On name acquisition, register with the watcher (best-effort).
                let conn = conn.clone();
                let bus_name = bus_name.clone();
                move |_c, _n| tray_register_with_watcher(&conn, &bus_name)
            },
            |_c, _n| {},
        );

        TRAYS.with(|trays| {
            trays.borrow_mut().insert(
                app_id.clone(),
                TrayItem {
                    name: app.name.clone(),
                    icon_name: tray_icon_name(app),
                    desktop_entry: format!("{}.{}", qwa_core::APP_ID, app.id),
                    shared: shared.clone(),
                    owner_id,
                    item_reg,
                    menu_reg,
                },
            );
        });
        tracing::info!("registered StatusNotifier tray item {bus_name}");
    });
}

/// The icon-name an SNI item advertises. CEF apps install a themed icon under
/// the desktop id, so try that first; fall back to a safe stock web-app icon.
fn tray_icon_name(app: &WebApp) -> String {
    // The DynamicLauncher install registers the icon under the desktop id, so a
    // themed lookup of `<APP_ID>.<id>` usually resolves. The fallback keeps the
    // tray usable even when no themed icon is found.
    format!("{}.{}", qwa_core::APP_ID, app.id)
}

/// Best-effort call to org.kde.StatusNotifierWatcher.RegisterStatusNotifierItem
/// with our item's bus name. A missing watcher (bare GNOME) just logs at debug.
fn tray_register_with_watcher(conn: &gio::DBusConnection, bus_name: &str) {
    use gtk::glib::prelude::ToVariant;
    let params = (bus_name.to_string(),).to_variant();
    conn.call(
        Some("org.kde.StatusNotifierWatcher"),
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
        "RegisterStatusNotifierItem",
        Some(&params),
        None,
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
        move |res| {
            if let Err(e) = res {
                tracing::debug!("StatusNotifierWatcher unavailable (tray hidden): {e}");
            }
        },
    );
}

/// Tear down a window's tray item on close: drop its registry entry, unregister
/// both objects, and release the bus name.
fn tray_remove(app_id: &str) {
    let item = TRAYS.with(|trays| trays.borrow_mut().remove(app_id));
    if let Some(item) = item {
        let _ = &item.desktop_entry; // kept for parity / future SNI fields
        DBUS.with(|c| {
            if let Some(conn) = c.borrow().as_ref() {
                let _ = conn.unregister_object(item.item_reg);
                let _ = conn.unregister_object(item.menu_reg);
            }
        });
        gio::bus_unown_name(item.owner_id);
        tracing::info!("removed StatusNotifier tray item for {app_id}");
    }
}

/// Build and present one app window (header + nav + profile cue + drawing area)
/// and create its off-screen CEF browser. Each window owns its own `Shared`
/// state and per-window `WebApp` context (threaded into the handlers), so one
/// process can host several same-profile apps as separate windows.
///
/// `url_override` is the URL to load instead of the app's home page — used for
/// the initial window when launched as a scheme handler (mailto:, …); relaunch
/// windows pass it through too, else load `webapp.url`.
fn open_window(app: &adw::Application, webapp: WebApp, url_override: Option<String>) {
    // Per-window app context shared (immutably) into every handler.
    let app_ctx = Rc::new(webapp.clone());

    let url = url_override.unwrap_or_else(|| webapp.url.clone());
    let title = webapp.name.clone();
    let app_id = webapp.id.clone();
    let color_scheme = webapp.color_scheme;
    // The login profile this app uses; surfaced as a colored dot + label in the
    // header so the cue (e.g. Work vs Private) persists while using the app.
    let profile = webapp.profile.clone();
    // When enabled, closing the window hides it (keeping the process + CEF
    // browser alive) so desktop notifications keep arriving.
    let background = webapp.run_in_background;

    // This window's own render/browser state.
    let shared = Rc::new(Shared::default());

    // Restore last-session geometry + zoom; fall back to the configured size.
    let (init_w, init_h, init_max) = match load_window_state(&webapp.id) {
        Some((m, w, h, z)) => {
            shared.zoom.set(z);
            (w, h, m)
        }
        None => (webapp.window.0 as i32, webapp.window.1 as i32, false),
    };

    // Match the window chrome to a forced color scheme (web content is
    // handled separately via the blink preferredColorScheme switch). The
    // StyleManager is process-level (best-effort with multiple windows).
    match color_scheme {
        qwa_core::ColorScheme::Light => {
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight)
        }
        qwa_core::ColorScheme::Dark => {
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark)
        }
        qwa_core::ColorScheme::System => {}
    }

    let header = adw::HeaderBar::new();

    // Navigation controls. Back/forward start insensitive and are toggled
    // by the LoadHandler; reload/stop is a simple reload for now.
    let back = gtk::Button::from_icon_name("go-previous-symbolic");
    back.set_tooltip_text(Some("Back"));
    back.set_sensitive(false);
    let forward = gtk::Button::from_icon_name("go-next-symbolic");
    forward.set_tooltip_text(Some("Forward"));
    forward.set_sensitive(false);
    let reload = gtk::Button::from_icon_name("view-refresh-symbolic");
    reload.set_tooltip_text(Some("Reload"));
    header.pack_start(&back);
    header.pack_start(&forward);
    header.pack_start(&reload);

    // Profile cue: a small colored dot + label so the user always knows
    // which login profile (e.g. Work vs Private) this app is running under.
    {
        let profile_label = profile
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Private")
            .to_string();
        let (pr, pg, pb) = qwa_core::profile_color(profile.as_deref());
        let dot = gtk::DrawingArea::builder()
            .content_width(12)
            .content_height(12)
            .valign(gtk::Align::Center)
            .build();
        dot.set_draw_func(move |_area, cr, w, h| {
            let d = (w.min(h) as f64) - 2.0;
            let radius = (d / 2.0).max(0.0);
            cr.arc(
                w as f64 / 2.0,
                h as f64 / 2.0,
                radius,
                0.0,
                std::f64::consts::TAU,
            );
            cr.set_source_rgb(pr as f64 / 255.0, pg as f64 / 255.0, pb as f64 / 255.0);
            let _ = cr.fill();
        });
        let label = gtk::Label::new(Some(&profile_label));
        label.add_css_class("dim-label");
        let profile_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        profile_box.set_valign(gtk::Align::Center);
        profile_box.append(&dot);
        profile_box.append(&label);
        header.pack_end(&profile_box);
    }

    {
        let shared = shared.clone();
        back.connect_clicked(move |_| {
            if let Some(b) = shared.browser.borrow().as_ref() {
                b.go_back();
            }
        });
    }
    {
        let shared = shared.clone();
        forward.connect_clicked(move |_| {
            if let Some(b) = shared.browser.borrow().as_ref() {
                b.go_forward();
            }
        });
    }
    {
        let shared = shared.clone();
        reload.connect_clicked(move |_| {
            if let Some(b) = shared.browser.borrow().as_ref() {
                b.reload();
            }
        });
    }

    let area = gtk::DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);

    {
        let shared = shared.clone();
        area.set_draw_func(move |_area, cr, w, h| {
            if let Some(frame) = shared.frame.borrow().as_ref() {
                let stride = frame.width * 4;
                if let Ok(surface) = gtk::cairo::ImageSurface::create_for_data(
                    frame.buf.clone(),
                    gtk::cairo::Format::ARgb32,
                    frame.width,
                    frame.height,
                    stride,
                ) {
                    // The buffer is at physical resolution (logical * scale);
                    // scale it down to fill the logical drawing area so the
                    // image stays crisp on HiDPI displays.
                    let sx = w as f64 / frame.width.max(1) as f64;
                    let sy = h as f64 / frame.height.max(1) as f64;
                    cr.scale(sx, sy);
                    if cr.set_source_surface(&surface, 0.0, 0.0).is_ok() {
                        let _ = cr.paint();
                    }
                }
            }
        });
    }

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&area));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(&title)
        .default_width(init_w)
        .default_height(init_h)
        .content(&toolbar)
        .build();
    if init_max {
        window.maximize();
    }
    // Persist geometry + zoom on close so the next launch restores them.
    {
        let shared = shared.clone();
        let app_id = app_id.clone();
        window.connect_close_request(move |win| {
            let (w, h) = win.default_size();
            save_window_state(&app_id, win.is_maximized(), w, h, shared.zoom.get());
            // Background apps stay alive: hide the window (and keep the CEF
            // browser running) instead of quitting, so notifications keep
            // arriving.
            if background {
                win.set_visible(false);
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
    }
    // Record a weak handle to the window so the MPRIS player (created lazily on
    // first media message) can present/close it for Raise/Quit.
    *shared.window.borrow_mut() = Some(window.downgrade());

    // Background-mode apps get a StatusNotifier tray item (Show / Quit) so they
    // can be re-shown after the window is hidden and genuinely quit.
    if background {
        tray_register(&webapp, &shared);
    }

    window.present();

    // Create the off-screen browser bound to this drawing area.
    let mut client = OsrClient::new(
        shared.clone(),
        area.clone(),
        back.clone(),
        forward.clone(),
        app_ctx.clone(),
    );
    let window_info = WindowInfo {
        windowless_rendering_enabled: 1,
        ..Default::default()
    };
    let browser_settings = BrowserSettings::default();
    let cef_url = CefString::from(url.as_str());
    browser_host_create_browser(
        Some(&window_info),
        Some(&mut client),
        Some(&cef_url),
        Some(&browser_settings),
        None,
        None,
    );

    // Tell CEF when the view is resized so it re-renders at the new size.
    {
        let shared = shared.clone();
        area.connect_resize(move |_area, _w, _h| {
            if let Some(browser) = shared.browser.borrow().as_ref() {
                if let Some(host) = browser.host() {
                    host.was_resized();
                }
            }
        });
    }

    // Re-query screen info (device scale) when the display scale changes,
    // e.g. moving the window between monitors of different DPI.
    {
        let shared = shared.clone();
        area.connect_scale_factor_notify(move |_area| {
            if let Some(browser) = shared.browser.borrow().as_ref() {
                if let Some(host) = browser.host() {
                    host.notify_screen_info_changed();
                    host.was_resized();
                }
            }
        });
    }

    // --- Input forwarding (#11 it.2): mouse, scroll, keyboard, focus. ---
    area.set_focusable(true);

    let motion = gtk::EventControllerMotion::new();
    {
        let shared = shared.clone();
        motion.connect_motion(move |_, x, y| {
            shared.mouse.set((x as i32, y as i32));
            with_host(&shared, |h| {
                h.send_mouse_move_event(Some(&mouse_event(x as i32, y as i32)), 0)
            });
        });
    }
    area.add_controller(motion);

    let click = gtk::GestureClick::new();
    click.set_button(0); // listen for all buttons
    {
        let shared = shared.clone();
        let area = area.clone();
        click.connect_pressed(move |gesture, n_press, x, y| {
            area.grab_focus();
            let button = map_button(gesture.current_button());
            with_host(&shared, |h| {
                h.set_focus(1);
                h.send_mouse_click_event(
                    Some(&mouse_event(x as i32, y as i32)),
                    button,
                    0,
                    n_press,
                );
            });
        });
    }
    {
        let shared = shared.clone();
        click.connect_released(move |gesture, n_press, x, y| {
            let button = map_button(gesture.current_button());
            with_host(&shared, |h| {
                h.send_mouse_click_event(
                    Some(&mouse_event(x as i32, y as i32)),
                    button,
                    1,
                    n_press,
                );
            });
        });
    }
    area.add_controller(click);

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    {
        let shared = shared.clone();
        scroll.connect_scroll(move |ctrl, dx, dy| {
            // Ctrl + scroll = zoom, like a browser.
            if ctrl
                .current_event_state()
                .contains(gtk::gdk::ModifierType::CONTROL_MASK)
            {
                set_zoom(&shared, shared.zoom.get() - dy * 0.5);
                return gtk::glib::Propagation::Stop;
            }
            let (mx, my) = shared.mouse.get();
            with_host(&shared, |h| {
                h.send_mouse_wheel_event(
                    Some(&mouse_event(mx, my)),
                    (-dx * 40.0) as i32,
                    (-dy * 40.0) as i32,
                )
            });
            gtk::glib::Propagation::Stop
        });
    }
    area.add_controller(scroll);

    let keys = gtk::EventControllerKey::new();
    {
        let shared = shared.clone();
        keys.connect_key_pressed(move |_, keyval, _code, state| {
            // Ctrl +/=/-/0 = zoom in / out / reset (handled here, not sent).
            if state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                use gtk::gdk::Key;
                let z = shared.zoom.get();
                if keyval == Key::plus || keyval == Key::equal || keyval == Key::KP_Add {
                    set_zoom(&shared, z + 0.5);
                    return gtk::glib::Propagation::Stop;
                }
                if keyval == Key::minus || keyval == Key::KP_Subtract {
                    set_zoom(&shared, z - 0.5);
                    return gtk::glib::Propagation::Stop;
                }
                if keyval == Key::_0 || keyval == Key::KP_0 {
                    set_zoom(&shared, 0.0);
                    return gtk::glib::Propagation::Stop;
                }
            }
            let vk = vk_from_keyval(keyval);
            let modifiers = cef_modifiers(state);
            with_host(&shared, |h| {
                h.send_key_event(Some(&KeyEvent {
                    type_: KeyEventType::RAWKEYDOWN,
                    windows_key_code: vk,
                    modifiers,
                    ..Default::default()
                }));
                if let Some(c) = keyval.to_unicode() {
                    if !c.is_control() {
                        h.send_key_event(Some(&KeyEvent {
                            type_: KeyEventType::CHAR,
                            character: c as u16,
                            windows_key_code: vk,
                            modifiers,
                            ..Default::default()
                        }));
                    }
                }
            });
            gtk::glib::Propagation::Proceed
        });
    }
    {
        let shared = shared.clone();
        keys.connect_key_released(move |_, keyval, _code, state| {
            let vk = vk_from_keyval(keyval);
            let modifiers = cef_modifiers(state);
            with_host(&shared, |h| {
                h.send_key_event(Some(&KeyEvent {
                    type_: KeyEventType::KEYUP,
                    windows_key_code: vk,
                    modifiers,
                    ..Default::default()
                }))
            });
        });
    }
    area.add_controller(keys);
    area.grab_focus();
}

/// Initialize CEF (off-screen) and run the GNOME window, pumping CEF from the
/// GTK loop. Returns when the window is closed.
pub fn run(main_args: &MainArgs, sandbox_info: *mut u8, webapp: WebApp) {
    // Attribute Chromium's desktop notifications to THIS app's launcher.
    // Chromium reads CHROME_DESKTOP for the org.freedesktop.Notifications
    // `desktop-entry` hint, so GNOME shows them with the app's name/icon and
    // keeps them in the notification list (sticky) instead of as "Chromium".
    // Subprocesses inherit this, so it must be set before CEF starts.
    std::env::set_var(
        "CHROME_DESKTOP",
        format!("{}.{}.desktop", qwa_core::APP_ID, webapp.id),
    );

    let settings = crate::app::build_settings(&webapp);

    let mut app = OsrApp::new();
    if initialize(
        Some(main_args),
        Some(&settings),
        Some(&mut app),
        sandbox_info,
    ) != 1
    {
        // initialize() != 1 means CEF forwarded our command line to the
        // already-running primary process that owns this profile's data dir
        // (CEF is a singleton per root_cache_path). That primary's
        // BrowserProcessHandler::on_already_running_app_relaunch opens a window
        // for us; this process has nothing more to do. This is the normal
        // same-profile path (Chrome's model), not an error.
        tracing::info!(
            "forwarded to existing instance for profile '{}'",
            webapp.profile_key()
        );
        return;
    }

    // Open the session bus once for the whole process, stashed for the badge
    // emitter. Best-effort: a missing bus just disables badge updates.
    let dbus = gtk::gio::bus_get_sync(gtk::gio::BusType::Session, gtk::gio::Cancellable::NONE).ok();
    if dbus.is_none() {
        tracing::debug!("session bus unavailable; dock badges disabled");
    }
    DBUS.with(|c| *c.borrow_mut() = dbus);

    // One GApplication per *profile* (not per app): same-profile apps share the
    // single CEF process, so they must also share one GApplication. The id is
    // sanitized to a valid application id (alphanumeric / '-' segments, never
    // leading with a digit or '-').
    let application = adw::Application::builder()
        .application_id(&format!(
            "{}.{}",
            qwa_core::APP_ID,
            sanitize_app_id(webapp.profile_key())
        ))
        .build();

    // If launched as a scheme handler (mailto:, webcal:, …), open the target
    // URL the matching handler expands to; otherwise the app's home page. This
    // override only applies to the INITIAL window — relaunch windows compute
    // their own from the forwarded command line.
    let url_override = crate::app::url_arg().and_then(|arg| {
        let scheme = arg.split(':').next().unwrap_or("").to_string();
        webapp
            .handlers
            .iter()
            .find(|h| h.scheme() == scheme)
            .map(|h| qwa_core::handlers::expand(&h.template, &arg))
    });

    application.connect_activate(move |app| {
        // Stash the running GApplication so the relaunch handler can open
        // windows on it for forwarded same-profile apps.
        GTK_APP.with(|cell| *cell.borrow_mut() = Some(app.clone()));

        open_window(app, webapp.clone(), url_override.clone());

        // Drive CEF's work from the GTK main loop (~60fps). Registered once for
        // the whole process; it services every window's browser.
        gtk::glib::timeout_add_local(Duration::from_millis(16), || {
            do_message_loop_work();
            gtk::glib::ControlFlow::Continue
        });
    });

    // Run with a controlled argv: our real argv carries the web-app id, which
    // GApplication would otherwise reject as an unknown argument.
    application.run_with_args::<&str>(&["gnome-quick-web-apps-runner"]);

    shutdown();
}

/// Sanitize a profile key into a valid GApplication id segment: keep ASCII
/// alphanumerics and '-', map everything else (notably '.') to '_', and ensure
/// it does not start with a digit or '-' (prefix an '_' if it would).
fn sanitize_app_id(key: &str) -> String {
    let mut s: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        s.push('_');
    } else if s.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
        s.insert(0, '_');
    }
    s
}
