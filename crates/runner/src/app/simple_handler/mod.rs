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
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            user_gesture: i32,
            is_redirect: i32,
        ) -> i32 {
            let Some(request) = request else {
                return 0;
            };
            let url = CefString::from(&request.url()).to_string();

            // Judge scope against the page we're actually on (post-redirect),
            // falling back to the configured app URL.
            let home = crate::app::current_page_url(browser)
                .unwrap_or_else(|| self.app_url.clone());

            // Only divert a DELIBERATE user click to a genuinely external site.
            // Redirects / programmatic navigation always stay in-window so
            // login/OAuth flows (mail.google.com -> accounts.google.com) work.
            let deliberate = user_gesture == 1 && is_redirect == 0;
            if deliberate && !qwa_core::is_in_scope(&url, self.scope.as_deref(), &home) {
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
