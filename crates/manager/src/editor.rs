//! Create-web-app dialog. Phase 1: URL + name + category, validation, and
//! save to JSON with a generated lettered icon. The async portal launcher
//! install (#2/#4) and PWA manifest autofill (#6) hook in here next.

use adw::prelude::*;
use gtk::glib;
use qwa_core::{icon, launcher, Category, WebApp};

/// Open the modal editor over `parent`; call `on_saved` after a successful
/// save so the caller can refresh its list.
pub fn present<F: Fn() + 'static>(parent: &adw::ApplicationWindow, on_saved: F) {
    let window = adw::Window::builder()
        .title("New Web App")
        .modal(true)
        .transient_for(parent)
        .default_width(460)
        .default_height(360)
        .build();

    let header = adw::HeaderBar::new();
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label("Add");
    save.add_css_class("suggested-action");
    header.pack_start(&cancel);
    header.pack_end(&save);

    let url_row = adw::EntryRow::builder().title("URL").build();
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

    save.connect_clicked(glib::clone!(
        #[weak] window,
        #[weak] url_row,
        #[weak] name_row,
        #[weak] cat_row,
        move |_| {
            let url = url_row.text().to_string();
            let name = name_row.text().trim().to_string();

            let url_ok = url::Url::parse(&url)
                .map(|u| matches!(u.scheme(), "http" | "https"))
                .unwrap_or(false);
            if !url_ok {
                url_row.add_css_class("error");
                return;
            }
            if name.is_empty() {
                name_row.add_css_class("error");
                return;
            }

            let category = Category::from_index(cat_row.selected());
            let mut app = WebApp::new(name, url, category);
            if let Ok(path) = icon::generate_lettered(&app.id, &app.name) {
                app.icon_path = Some(path);
            }
            match app.save() {
                Ok(()) => {
                    tracing::info!("created web app {}", app.id);
                    install_launcher(&app);
                    on_saved();
                    window.close();
                }
                Err(e) => tracing::error!("failed to save web app: {e}"),
            }
        }
    ));

    window.present();
}

/// Install the `.desktop` launcher via the portal on the background runtime.
/// Fire-and-forget: the JSON config is already saved and the portal shows
/// its own confirmation dialog; failures are logged.
fn install_launcher(app: &WebApp) {
    let Some(icon_path) = app.icon_path.clone() else {
        tracing::warn!("no icon for {}, skipping launcher install", app.id);
        return;
    };
    let Ok(bytes) = icon::read_bytes(&icon_path) else {
        tracing::warn!("could not read icon {}", icon_path.display());
        return;
    };
    let app = app.clone();
    crate::runtime().spawn(async move {
        if let Err(e) = launcher::install(&app, bytes).await {
            tracing::error!("launcher install failed for {}: {e}", app.id);
        }
    });
}
