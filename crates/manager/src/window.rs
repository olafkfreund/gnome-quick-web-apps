//! Main manager window: header bar + a live list of installed web apps with
//! create (header `+`) and per-row delete, refreshing on each change.

use adw::prelude::*;
use gtk::glib;
use qwa_core::{launcher, WebApp};

use crate::editor;

pub fn build(app: &adw::Application) -> adw::ApplicationWindow {
    let header = adw::HeaderBar::new();
    let new_button = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("New Web App")
        .build();
    header.pack_start(&new_button);

    let list_container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list_container.set_vexpand(true);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&list_container);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Quick Web Apps")
        .default_width(720)
        .default_height(560)
        .content(&content)
        .build();

    populate(&list_container, &window);

    new_button.connect_clicked(glib::clone!(
        #[weak] window,
        #[weak] list_container,
        move |_| {
            editor::present(
                &window,
                glib::clone!(
                    #[weak] window,
                    #[weak] list_container,
                    move || populate(&list_container, &window)
                ),
            );
        }
    ));

    window
}

/// Clear and rebuild the app list. Called on startup and after any change.
fn populate(container: &gtk::Box, window: &adw::ApplicationWindow) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let apps = WebApp::load_all();

    if apps.is_empty() {
        let status = adw::StatusPage::builder()
            .icon_name("application-x-addon-symbolic")
            .title("No Web Apps Yet")
            .description("Click + to turn any site into a desktop app.")
            .vexpand(true)
            .build();
        container.append(&status);
        return;
    }

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    for app in apps {
        let row = adw::ActionRow::builder()
            .title(app.name.as_str())
            .subtitle(app.url.as_str())
            .build();

        let delete = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .tooltip_text("Remove")
            .build();
        delete.connect_clicked(glib::clone!(
            #[weak] container,
            #[weak] window,
            move |_| {
                let dialog = adw::AlertDialog::new(
                    Some("Delete Web App?"),
                    Some(&format!(
                        "“{}” and its launcher will be removed. This cannot be undone.",
                        app.name
                    )),
                );
                dialog.add_responses(&[("cancel", "Cancel"), ("delete", "Delete")]);
                dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");

                let app = app.clone();
                dialog.connect_response(
                    Some("delete"),
                    glib::clone!(
                        #[weak] container,
                        #[weak] window,
                        move |_, _| {
                            // Remove the .desktop launcher via the portal...
                            let to_remove = app.clone();
                            crate::runtime().spawn(async move {
                                if let Err(e) = launcher::uninstall(&to_remove).await {
                                    tracing::error!(
                                        "launcher uninstall failed for {}: {e}",
                                        to_remove.id
                                    );
                                }
                            });
                            // ...and the local config, icon and profile.
                            app.remove_local();
                            populate(&container, &window);
                        }
                    ),
                );
                dialog.present(Some(&window));
            }
        ));
        row.add_suffix(&delete);
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

    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&clamp)
        .build();
    container.append(&scroller);
}
