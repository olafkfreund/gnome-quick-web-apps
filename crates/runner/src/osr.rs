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
}

wrap_app! {
    pub struct OsrApp;
    impl App {}
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

wrap_client! {
    struct OsrClient {
        shared: Rc<Shared>,
        area: gtk::DrawingArea,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(OsrRenderHandler::new(self.shared.clone(), self.area.clone()))
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(OsrLifeSpanHandler::new(self.shared.clone()))
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            // Preserve #9 scope confinement in the OSR path.
            let (scope, app_url) = crate::app::current_app()
                .map(|a| (a.scope, a.url))
                .unwrap_or((None, String::new()));
            Some(crate::app::simple_handler::ScopeRequestHandler::new(scope, app_url))
        }
    }
}

/// Initialize CEF (off-screen) and run the GNOME window, pumping CEF from the
/// GTK loop. Returns when the window is closed.
pub fn run(main_args: &MainArgs, sandbox_info: *mut u8, webapp: WebApp) {
    let settings = crate::app::build_settings(&webapp);

    let mut app = OsrApp::new();
    assert_eq!(
        initialize(Some(main_args), Some(&settings), Some(&mut app), sandbox_info),
        1,
        "CEF initialize failed"
    );

    let shared = Rc::new(Shared::default());

    let application = adw::Application::builder()
        .application_id(&format!("{}.{}", qwa_core::APP_ID, webapp.id))
        .build();

    let url = webapp.url.clone();
    let title = webapp.name.clone();
    let win_w = webapp.window.0 as i32;
    let win_h = webapp.window.1 as i32;

    application.connect_activate(move |app| {
        let header = adw::HeaderBar::new();

        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);

        {
            let shared = shared.clone();
            area.set_draw_func(move |_area, cr, _w, _h| {
                if let Some(frame) = shared.frame.borrow().as_ref() {
                    let stride = frame.width * 4;
                    if let Ok(surface) = gtk::cairo::ImageSurface::create_for_data(
                        frame.buf.clone(),
                        gtk::cairo::Format::ARgb32,
                        frame.width,
                        frame.height,
                        stride,
                    ) {
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
        let mut client = OsrClient::new(shared.clone(), area.clone());
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
