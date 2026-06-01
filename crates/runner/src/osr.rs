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
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use cef::{Rect, *};
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
            }
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
                info.device_scale_factor = self.area.scale_factor().max(1) as f32;
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
                let app = crate::app::current_app();
                let external = app.as_ref().map(|a| a.external_links_in_browser).unwrap_or(false);
                let scope = app.as_ref().and_then(|a| a.scope.clone());
                let app_url = app.map(|a| a.url).unwrap_or_default();
                let home = crate::app::current_page_url(browser.as_deref_mut()).unwrap_or(app_url);

                // A user-opened popup to a DIFFERENT host means "leave the app"
                // (a link in an email, an off-site button). With external mode
                // on, hand those to the system browser. We match on host, not
                // registrable domain, so Gmail's same-domain link redirector
                // (www.google.com/url?q=...) also leaves — while same-host
                // pop-outs (compose, print) and in-scope pages stay in-window.
                let same_app = qwa_core::host_eq(&url, &home)
                    || scope
                        .as_deref()
                        .is_some_and(|s| qwa_core::is_in_scope(&url, Some(s), &home));
                if external && !same_app {
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
        }
    }
}

wrap_load_handler! {
    struct OsrLoadHandler {
        shared: Rc<Shared>,
        back: gtk::Button,
        forward: gtk::Button,
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

            // Record the settled home once the first load completes, after any
            // initial redirect chain (gmail.com -> mail.google.com).
            if is_loading == 0 && self.shared.home.borrow().is_none() {
                if let Some(url) = crate::app::current_page_url(browser) {
                    if url.starts_with("http") {
                        tracing::info!("settled home: {url}");
                        *self.shared.home.borrow_mut() = Some(url);
                    }
                }
            }
        }
    }
}

wrap_request_handler! {
    struct OsrRequestHandler {
        shared: Rc<Shared>,
        scope: Option<String>,
        external_in_browser: bool,
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

            // Diversion to the system browser is opt-in. Off by default so that
            // multi-domain logins (e.g. Microsoft: outlook.cloud.microsoft ->
            // login.microsoftonline.com -> login.live.com) stay in the window.
            if !self.external_in_browser {
                return 0;
            }
            // Until the app settles on its home, allow everything (initial load
            // + redirects, possibly across domains).
            let home = match self.shared.home.borrow().clone() {
                Some(h) => h,
                None => return 0,
            };
            if !qwa_core::is_in_scope(&url, self.scope.as_deref(), &home) {
                if let Err(e) = open::that(&url) {
                    tracing::warn!("failed to open external url {url}: {e}");
                }
                return 1;
            }
            0
        }
    }
}

wrap_permission_handler! {
    struct OsrPermissionHandler {}

    impl PermissionHandler {
        fn on_show_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            _prompt_id: u64,
            _requesting_origin: Option<&CefString>,
            _requested_permissions: u32,
            callback: Option<&mut PermissionPromptCallback>,
        ) -> i32 {
            // Grant permission prompts (desktop notifications, etc.) — these are
            // the user's own installed web apps. OSR has no prompt UI, so
            // without this the request would simply be denied.
            if let Some(callback) = callback {
                callback.cont(PermissionRequestResult::ACCEPT);
            }
            1 // handled
        }
    }
}

wrap_client! {
    struct OsrClient {
        shared: Rc<Shared>,
        area: gtk::DrawingArea,
        back: gtk::Button,
        forward: gtk::Button,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(OsrRenderHandler::new(self.shared.clone(), self.area.clone()))
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(OsrLifeSpanHandler::new(self.shared.clone()))
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            let (scope, external) = crate::app::current_app()
                .map(|a| (a.scope, a.external_links_in_browser))
                .unwrap_or((None, false));
            Some(OsrRequestHandler::new(self.shared.clone(), scope, external))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(OsrLoadHandler::new(
                self.shared.clone(),
                self.back.clone(),
                self.forward.clone(),
            ))
        }

        fn permission_handler(&self) -> Option<PermissionHandler> {
            Some(OsrPermissionHandler::new())
        }
    }
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
    assert_eq!(
        initialize(
            Some(main_args),
            Some(&settings),
            Some(&mut app),
            sandbox_info
        ),
        1,
        "CEF initialize failed"
    );

    let shared = Rc::new(Shared::default());

    let application = adw::Application::builder()
        .application_id(&format!("{}.{}", qwa_core::APP_ID, webapp.id))
        .build();

    // If launched as a scheme handler (mailto:, webcal:, …), open the target
    // URL the matching handler expands to; otherwise the app's home page.
    let url = crate::app::url_arg()
        .and_then(|arg| {
            let scheme = arg.split(':').next().unwrap_or("").to_string();
            webapp
                .handlers
                .iter()
                .find(|h| h.scheme() == scheme)
                .map(|h| qwa_core::handlers::expand(&h.template, &arg))
        })
        .unwrap_or_else(|| webapp.url.clone());
    let title = webapp.name.clone();
    let win_w = webapp.window.0 as i32;
    let win_h = webapp.window.1 as i32;

    application.connect_activate(move |app| {
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
            .default_width(win_w)
            .default_height(win_h)
            .content(&toolbar)
            .build();
        window.present();

        // Create the off-screen browser bound to this drawing area.
        let mut client =
            OsrClient::new(shared.clone(), area.clone(), back.clone(), forward.clone());
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

        // Drive CEF's work from the GTK main loop (~60fps).
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
