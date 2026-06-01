//! Create / edit web-app dialog.
//!
//! Paste a URL and "Detect" to autofill name + icon candidates from the PWA
//! manifest (#6). Icons: a chosen file wins; otherwise we download the best
//! manifest/apple-touch icon, falling back to a favicon service and finally a
//! generated lettered icon. Opening with `Some(app)` edits in place.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use qwa_core::manifest::SiteInfo;
use qwa_core::{icon, launcher, Category, WebApp};

/// Open the editor over `parent`. `existing` = None creates a new app; Some
/// edits it in place. `on_saved` is called after a successful save.
pub fn present<F: Fn() + 'static>(
    parent: &adw::ApplicationWindow,
    existing: Option<WebApp>,
    on_saved: F,
) {
    let editing = existing.is_some();
    let window = adw::Window::builder()
        .title(if editing { "Edit Web App" } else { "New Web App" })
        .modal(true)
        .transient_for(parent)
        .default_width(480)
        .default_height(440)
        .build();

    let header = adw::HeaderBar::new();
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label(if editing { "Save" } else { "Add" });
    save.add_css_class("suggested-action");
    header.pack_start(&cancel);
    header.pack_end(&save);

    let detected: Rc<RefCell<Option<SiteInfo>>> = Rc::new(RefCell::new(None));
    // The user's chosen icon file (starts from the existing app's icon).
    let chosen_icon: Rc<RefCell<Option<PathBuf>>> =
        Rc::new(RefCell::new(existing.as_ref().and_then(|a| a.icon_path.clone())));
    let icon_picked = Rc::new(Cell::new(false));

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
    cat_row.set_selected((Category::ALL.len() - 1) as u32);

    // Icon row: preview + "Choose File…".
    let icon_img = gtk::Image::builder().pixel_size(32).build();
    set_icon_preview(&icon_img, chosen_icon.borrow().as_deref());
    let choose_btn = gtk::Button::with_label("Choose File…");
    let icon_row = adw::ActionRow::builder().title("Icon").build();
    icon_row.add_prefix(&icon_img);
    icon_row.add_suffix(&choose_btn);

    // Pre-fill when editing.
    if let Some(app) = &existing {
        url_row.set_text(&app.url);
        name_row.set_text(&app.name);
        if let Some(idx) = Category::ALL.iter().position(|c| c == &app.category) {
            cat_row.set_selected(idx as u32);
        }
    }

    let group = adw::PreferencesGroup::new();
    group.add(&url_row);
    group.add(&name_row);
    group.add(&cat_row);
    group.add(&icon_row);

    let page = adw::PreferencesPage::new();
    page.add(&group);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&page);
    window.set_content(Some(&content));

    cancel.connect_clicked(glib::clone!(#[weak] window, move |_| window.close()));

    // --- Choose an icon file. ---
    choose_btn.connect_clicked(glib::clone!(
        #[weak] window,
        #[weak] icon_img,
        #[strong] chosen_icon,
        #[strong] icon_picked,
        move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Images"));
            for p in ["*.png", "*.svg", "*.jpg", "*.jpeg", "*.ico", "*.webp"] {
                filter.add_pattern(p);
            }
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);

            let dialog = gtk::FileDialog::builder()
                .title("Choose Icon")
                .filters(&filters)
                .build();
            dialog.open(
                Some(&window),
                gio::Cancellable::NONE,
                glib::clone!(
                    #[weak] icon_img,
                    #[strong] chosen_icon,
                    #[strong] icon_picked,
                    move |res| {
                        if let Some(path) = res.ok().and_then(|f| f.path()) {
                            set_icon_preview(&icon_img, Some(&path));
                            *chosen_icon.borrow_mut() = Some(path);
                            icon_picked.set(true);
                        }
                    }
                ),
            );
        }
    ));

    // --- Detect manifest info. ---
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

            let name_row = name_row.clone();
            let detect_btn = detect_btn.clone();
            let spinner = spinner.clone();
            let detected = detected.clone();
            glib::spawn_future_local(async move {
                if let Ok(Ok(info)) = rx.recv().await {
                    if name_row.text().trim().is_empty() {
                        if let Some(name) = &info.name {
                            name_row.set_text(name);
                        }
                    }
                    *detected.borrow_mut() = Some(info);
                }
                spinner.stop();
                detect_btn.set_sensitive(true);
            });
        }
    ));

    // --- Save. ---
    let existing_for_save = existing.clone();
    save.connect_clicked(glib::clone!(
        #[weak] window,
        #[weak] url_row,
        #[weak] name_row,
        #[weak] cat_row,
        #[strong] detected,
        #[strong] chosen_icon,
        #[strong] icon_picked,
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

            // Build the app: keep id (and other fields) when editing.
            let mut app = match &existing_for_save {
                Some(a) => {
                    let mut a = a.clone();
                    a.name = name;
                    a.url = url;
                    a.category = category;
                    a
                }
                None => WebApp::new(name, url, category),
            };

            // Manifest-derived scope/theme + icon candidates (if detected).
            let mut candidates = match detected.borrow().as_ref() {
                Some(info) => {
                    app.scope = info.scope.clone();
                    app.theme_color = info.theme_color.clone();
                    info.icon_urls.clone()
                }
                None => Vec::new(),
            };
            candidates.extend(icon::favicon_service_urls(&app.url));

            app.icon_path = chosen_icon.borrow().clone();

            // Auto-fetch an icon unless the user picked one. Always ensure a
            // lettered fallback so the launcher install has something.
            let auto = !icon_picked.get();
            if app.icon_path.is_none() {
                if let Ok(p) = icon::generate_lettered(&app.id, &app.name) {
                    app.icon_path = Some(p);
                }
            }
            if let Err(e) = app.save() {
                tracing::error!("failed to save web app: {e}");
                return;
            }
            tracing::info!("{} web app {}", if editing { "edited" } else { "created" }, app.id);
            finalize_async(app, candidates, auto);
            on_saved();
            window.close();
        }
    ));

    window.present();
}

fn set_icon_preview(img: &gtk::Image, path: Option<&std::path::Path>) {
    match path {
        Some(p) if p.exists() => img.set_from_file(Some(p)),
        _ => img.set_icon_name(Some("application-x-addon-symbolic")),
    }
}

fn is_http_url(url: &str) -> bool {
    url::Url::parse(url)
        .map(|u| matches!(u.scheme(), "http" | "https"))
        .unwrap_or(false)
}

/// Background tail of save: optionally download a better icon, then (re)install
/// the launcher via the portal.
fn finalize_async(mut app: WebApp, candidates: Vec<String>, auto: bool) {
    crate::runtime().spawn(async move {
        if auto && !candidates.is_empty() {
            match icon::download_best(&app.id, &candidates).await {
                Ok(path) => {
                    app.icon_path = Some(path);
                    let _ = app.save();
                }
                Err(e) => tracing::warn!("icon download failed for {}: {e}", app.id),
            }
        }

        let bytes = app.icon_path.as_ref().and_then(|p| icon::read_bytes(p).ok());
        let Some(bytes) = bytes else {
            tracing::warn!("no icon bytes for {}, skipping install", app.id);
            return;
        };
        if let Err(e) = launcher::install(&app, bytes).await {
            tracing::error!("launcher install failed for {}: {e}", app.id);
        }
    });
}
