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
            user_gesture: i32,
            is_redirect: i32,
        ) -> i32 {
            let Some(request) = request else {
                return 0;
            };
            let url = CefString::from(&request.url()).to_string();

            // Only divert a DELIBERATE user click to an off-scope site. Never
            // touch redirects or programmatic navigation — diverting those
            // breaks login/OAuth flows (e.g. mail.google.com -> accounts...)
            // and would leave the window blank.
            let deliberate = user_gesture == 1 && is_redirect == 0;
            if deliberate && !qwa_core::is_in_scope(&url, self.scope.as_deref(), &self.app_url) {
                if let Err(e) = open::that(&url) {
                    tracing::warn!("failed to open external url {url}: {e}");
                }
                1 // cancel in-app navigation; opened externally
            } else {
                0 // allow in-app
            }
        }
    }
}
