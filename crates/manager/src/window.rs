//! Main manager window: header bar + a list of installed web apps.

use adw::prelude::*;
use qwa_core::WebApp;

pub fn build(app: &adw::Application) -> adw::ApplicationWindow {
    let header = adw::HeaderBar::new();

    let new_button = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("New Web App")
        .build();
    new_button.connect_clicked(|_| {
        // TODO(#editor): open the create/edit dialog with manifest autofill.
        tracing::info!("new web app requested");
    });
    header.pack_start(&new_button);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&build_list());

    adw::ApplicationWindow::builder()
        .application(app)
        .title("Quick Web Apps")
        .default_width(720)
        .default_height(560)
        .content(&content)
        .build()
}

fn build_list() -> gtk::Widget {
    let apps = WebApp::load_all();

    if apps.is_empty() {
        return adw::StatusPage::builder()
            .icon_name("application-x-addon-symbolic")
            .title("No Web Apps Yet")
            .description("Paste a URL to turn any site into a desktop app.")
            .vexpand(true)
            .build()
            .upcast();
    }

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    for app in apps {
        let row = adw::ActionRow::builder()
            .title(&app.name)
            .subtitle(&app.url)
            .build();
        list.append(&row);
    }

    let clamp = adw::Clamp::builder()
        .maximum_size(700)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(12)
        .margin_end(12)
        .child(&list)
        .build();

    gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&clamp)
        .build()
        .upcast()
}
