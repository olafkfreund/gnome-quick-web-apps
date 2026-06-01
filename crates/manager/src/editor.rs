//! Create-web-app dialog with PWA manifest autofill (#6).
//!
//! Flow: the user pastes a URL and clicks the detect button. We fetch the
//! site's Web App Manifest on the background Tokio runtime and, back on the
//! GTK main thread, fill the name and stash scope / theme colour / icon
//! candidates. On save we build the `WebApp`, download the best icon (or
//! fall back to a lettered one) and install the launcher.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use qwa_core::manifest::SiteInfo;
use qwa_core::{icon, launcher, Category, WebApp};

/// Open the modal editor over `parent`; call `on_saved` after a successful
/// save so the caller can refresh its list.
pub fn present<F: Fn() + 'static>(parent: &adw::ApplicationWindow, on_saved: F) {
    let window = adw::Window::builder()
        .title("New Web App")
        .modal(true)
        .transient_for(parent)
        .default_width(480)
        .default_height(380)
        .build();

    let header = adw::HeaderBar::new();
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label("Add");
    save.add_css_class("suggested-action");
    header.pack_start(&cancel);
    header.pack_end(&save);

    // Detected manifest data, shared between the detect handler and save.
    let detected: Rc<RefCell<Option<SiteInfo>>> = Rc::new(RefCell::new(None));

    let url_row = adw::EntryRow::builder().title("URL").build();
    let detect_btn = gtk::Button::builder()
        .icon_name("folder-download-symbolic")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .tooltip_text("Detect site info")
        .build();
    let spinner = gtk::Spinner::new();
    url_row.add_suffix(&spinner);
    url_row.add_suffix(&detect_btn);

    let name_row = adw::EntryRow::builder().title("Name").build();

    let labels: Vec<&str> = Category::ALL.iter().map(|c| c.label()).collect();
    let cat_model = gtk::StringList::new(&labels);
    let cat_row = adw::ComboRow::builder()
        .title("Category")
        .model(&cat_model)
        .build();
    cat_row.set_selected((Category::ALL.len() - 1) as u32); // default: Utility

    let group = adw::PreferencesGroup::new();
    group.add(&url_row);
    group.add(&name_row);
    group.add(&cat_row);

    let page = adw::PreferencesPage::new();
    page.add(&group);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&page);
    window.set_content(Some(&content));

    cancel.connect_clicked(glib::clone!(
        #[weak]
        window,
        move |_| window.close()
    ));

    // --- Detect: fetch the manifest on the runtime, fill the form on main. ---
    detect_btn.connect_clicked(glib::clone!(
        #[weak] url_row,
        #[weak] name_row,
        #[weak] detect_btn,
        #[weak] spinner,
        #[strong] detected,
        move |_| {
            let url = url_row.text().to_string();
            if !is_http_url(&url) {
                url_row.add_css_class("error");
                return;
            }
            url_row.remove_css_class("error");
            detect_btn.set_sensitive(false);
            spinner.start();

            let (tx, rx) = async_channel::bounded(1);
            crate::runtime().spawn(async move {
                let _ = tx.send(qwa_core::manifest::detect(&url).await).await;
            });

            // Strong clones live only for this short-lived local future.
            let name_row = name_row.clone();
            let detect_btn = detect_btn.clone();
            let spinner = spinner.clone();
            let detected = detected.clone();
            glib::spawn_future_local(async move {
                if let Ok(result) = rx.recv().await {
                    match result {
                        Ok(info) => {
                            if name_row.text().trim().is_empty() {
                                if let Some(name) = &info.name {
                                    name_row.set_text(name);
                                }
                            }
                            *detected.borrow_mut() = Some(info);
                        }
                        Err(e) => tracing::warn!("manifest detect failed: {e}"),
                    }
                }
                spinner.stop();
                detect_btn.set_sensitive(true);
            });
        }
    ));

    // --- Save: build, persist, then download icon + install in background. ---
    save.connect_clicked(glib::clone!(
        #[weak] window,
        #[weak] url_row,
        #[weak] name_row,
        #[weak] cat_row,
        #[strong] detected,
        move |_| {
            let url = url_row.text().to_string();
            let name = name_row.text().trim().to_string();

            if !is_http_url(&url) {
                url_row.add_css_class("error");
                return;
            }
            if name.is_empty() {
                name_row.add_css_class("error");
                return;
            }

            let category = Category::from_index(cat_row.selected());
            let mut app = WebApp::new(name, url, category);

            // Apply anything we learned from the manifest.
            let icon_urls = match detected.borrow().as_ref() {
                Some(info) => {
                    app.scope = info.scope.clone();
                    app.theme_color = info.theme_color.clone();
                    info.icon_urls.clone()
                }
                None => Vec::new(),
            };

            // Guaranteed fallback icon, replaced by a downloaded one if possible.
            if let Ok(path) = icon::generate_lettered(&app.id, &app.name) {
                app.icon_path = Some(path);
            }
            if let Err(e) = app.save() {
                tracing::error!("failed to save web app: {e}");
                return;
            }
            tracing::info!("created web app {}", app.id);
            finalize_async(app, icon_urls);
            on_saved();
            window.close();
        }
    ));

    window.present();
}

fn is_http_url(url: &str) -> bool {
    url::Url::parse(url)
        .map(|u| matches!(u.scheme(), "http" | "https"))
        .unwrap_or(false)
}

/// Background tail of save: try to download a better icon (persisting it),
/// then install the launcher via the portal. Fire-and-forget with logging.
fn finalize_async(mut app: WebApp, icon_urls: Vec<String>) {
    crate::runtime().spawn(async move {
        if !icon_urls.is_empty() {
            match icon::download_best(&app.id, &icon_urls).await {
                Ok(path) => {
                    app.icon_path = Some(path);
                    let _ = app.save();
                }
                Err(e) => tracing::warn!("icon download failed for {}: {e}", app.id),
            }
        }

        let bytes = app
            .icon_path
            .as_ref()
            .and_then(|p| icon::read_bytes(p).ok());
        let Some(bytes) = bytes else {
            tracing::warn!("no icon bytes for {}, skipping install", app.id);
            return;
        };
        if let Err(e) = launcher::install(&app, bytes).await {
            tracing::error!("launcher install failed for {}: {e}", app.id);
        }
    });
}
