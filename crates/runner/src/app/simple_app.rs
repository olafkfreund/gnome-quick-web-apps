//! CEF `App` + window/browser-view delegates. Port of upstream cefsimple,
//! reading window size and URL from our `WebApp` config.

use cef::{Rect, *};
use std::cell::RefCell;

use super::simple_handler::*;

wrap_window_delegate! {
    struct SimpleWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
        initial_show_state: ShowState,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            let (width, height) = crate::app::current_app()
                .map(|a| (a.window.0 as i32, a.window.1 as i32))
                .unwrap_or((960, 720));
            Size { width, height }
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let browser_view = self.browser_view.borrow();
            let (Some(window), Some(browser_view)) = (window, browser_view.as_ref()) else {
                return;
            };
            let mut view = View::from(browser_view);
            window.add_child_view(Some(&mut view));

            if self.initial_show_state != ShowState::HIDDEN {
                window.show();
            }
        }

        fn can_resize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn initial_bounds(&self, _window: Option<&mut Window>) -> Rect {
            if let Some(app) = crate::app::current_app() {
                return Rect {
                    width: app.window.0 as i32,
                    height: app.window.1 as i32,
                    ..Default::default()
                };
            }
            Default::default()
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            let mut browser_view = self.browser_view.borrow_mut();
            *browser_view = None;
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            let browser_view = self.browser_view.borrow();
            let browser_view = browser_view.as_ref().expect("BrowserView is None");
            if let Some(browser) = browser_view.browser() {
                let browser_host = browser.host().expect("BrowserHost is None");
                browser_host.try_close_browser()
            } else {
                1
            }
        }

        fn initial_show_state(&self, _window: Option<&mut Window>) -> ShowState {
            self.initial_show_state
        }

        fn window_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::ALLOY
        }
    }
}

wrap_browser_view_delegate! {
    struct SimpleBrowserViewDelegate {}

    impl ViewDelegate {}

    impl BrowserViewDelegate {
        fn on_popup_browser_view_created(
            &self,
            _browser_view: Option<&mut BrowserView>,
            popup_browser_view: Option<&mut BrowserView>,
            _is_devtools: i32,
        ) -> i32 {
            let mut window_delegate = SimpleWindowDelegate::new(
                RefCell::new(popup_browser_view.cloned()),
                ShowState::NORMAL,
            );
            window_create_top_level(Some(&mut window_delegate));
            1
        }

        fn browser_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::ALLOY
        }
    }
}

wrap_app! {
    pub struct SimpleApp;

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(SimpleBrowserProcessHandler::new(RefCell::new(None)))
        }
    }
}

wrap_browser_process_handler! {
    struct SimpleBrowserProcessHandler {
        client: RefCell<Option<Client>>,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            debug_assert_ne!(currently_on(ThreadId::UI), 0);

            let command_line = command_line_get_global().expect("Failed to get command line");

            {
                let mut client = self.client.borrow_mut();
                *client = Some(SimpleHandlerClient::new(SimpleHandler::new()));
            }

            let settings = BrowserSettings::default();

            let Some(webapp) = crate::app::current_app() else {
                return;
            };
            let url = CefString::from(webapp.url.as_str());

            let mut client = self.default_client();
            let mut delegate = SimpleBrowserViewDelegate::new();
            let browser_view = browser_view_create(
                client.as_mut(),
                Some(&url),
                Some(&settings),
                None,
                None,
                Some(&mut delegate),
            );

            let initial_show_state = CefString::from(
                &command_line.switch_value(Some(&CefString::from("initial-show-state"))),
            )
            .to_string();
            let initial_show_state = match initial_show_state.as_str() {
                "minimized" => ShowState::MINIMIZED,
                "maximized" => ShowState::MAXIMIZED,
                _ => ShowState::NORMAL,
            };

            let mut delegate =
                SimpleWindowDelegate::new(RefCell::new(browser_view), initial_show_state);
            window_create_top_level(Some(&mut delegate));
        }

        fn default_client(&self) -> Option<Client> {
            self.client.borrow().clone()
        }
    }
}
