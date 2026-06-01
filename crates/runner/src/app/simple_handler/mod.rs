//! Browser-level request handler: URL scope confinement (#9).
//!
//! Off-scope navigation is opened in the system browser and cancelled in-app.
//! Used by the OSR client (`crate::osr`).

use cef::*;

wrap_request_handler! {
    pub struct ScopeRequestHandler {
        scope: Option<String>,
        app_url: String,
    }

    impl RequestHandler {
        fn on_before_browse(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _user_gesture: i32,
            _is_redirect: i32,
        ) -> i32 {
            let Some(request) = request else {
                return 0;
            };
            let url = CefString::from(&request.url()).to_string();
            if qwa_core::is_in_scope(&url, self.scope.as_deref(), &self.app_url) {
                0 // allow in-app
            } else {
                if let Err(e) = open::that(&url) {
                    tracing::warn!("failed to open external url {url}: {e}");
                }
                1 // cancel in-app navigation
            }
        }
    }
}
