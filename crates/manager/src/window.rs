//! Main manager window: header bar + a live list of installed web apps with
//! create (header `+`) and per-row delete, refreshing on each change.

use std::rc::Rc;

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

    let templates_button = gtk::Button::builder()
        .icon_name("view-app-grid-symbolic")
        .tooltip_text("Add from Template")
        .build();
    header.pack_start(&templates_button);

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
        #[weak]
        window,
        #[weak]
        list_container,
        move |_| {
            editor::present(
                &window,
                None,
                false,
                glib::clone!(
                    #[weak]
                    window,
                    #[weak]
                    list_container,
                    move || populate(&list_container, &window)
                ),
            );
        }
    ));

    templates_button.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[weak]
        list_container,
        move |_| {
            present_templates(
                &window,
                glib::clone!(
                    #[weak]
                    window,
                    #[weak]
                    list_container,
                    move || populate(&list_container, &window)
                ),
            );
        }
    ));

    window
}

/// Grid of curated templates; clicking one fetches its icon and opens the
/// editor pre-filled so the user can pick a profile and Save.
fn present_templates<F: Fn() + 'static>(parent: &adw::ApplicationWindow, on_saved: F) {
    let on_saved = Rc::new(on_saved);

    let dialog = adw::Window::builder()
        .title("Add from Template")
        .modal(true)
        .transient_for(parent)
        .default_width(580)
        .default_height(620)
        .build();
    let header = adw::HeaderBar::new();
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .max_children_per_line(4)
        .column_spacing(12)
        .row_spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&flow)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&scroller);
    dialog.set_content(Some(&content));

    for tpl in qwa_core::templates::all() {
        let img = gtk::Image::builder().pixel_size(48).build();
        img.set_icon_name(Some("content-loading-symbolic"));
        let label = gtk::Label::builder()
            .label(tpl.name)
            .wrap(true)
            .max_width_chars(12)
            .justify(gtk::Justification::Center)
            .build();
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 6);
        vbox.set_width_request(96);
        vbox.append(&img);
        vbox.append(&label);
        let btn = gtk::Button::builder()
            .css_classes(["flat"])
            .child(&vbox)
            .build();
        flow.insert(&btn, -1);

        // Thumbnail.
        let (ttx, trx) = async_channel::bounded(1);
        let icon_id = tpl.icon.to_string();
        crate::runtime().spawn(async move {
            let _ = ttx
                .send(qwa_core::icon::iconify_png(&icon_id, 48).await)
                .await;
        });
        glib::spawn_future_local(glib::clone!(
            #[weak]
            img,
            async move {
                if let Ok(Some(png)) = trx.recv().await {
                    let bytes = glib::Bytes::from(&png[..]);
                    if let Ok(tex) = gtk::gdk::Texture::from_bytes(&bytes) {
                        img.set_paintable(Some(&tex));
                    }
                }
            }
        ));

        // Click: download icon (Iconify, favicon fallback) then open editor.
        let name = tpl.name.to_string();
        let url = tpl.url.to_string();
        let category = tpl.category;
        let icon_id = tpl.icon.to_string();
        let external_links = tpl.external_links;
        let show_badge = tpl.show_badge;
        btn.connect_clicked(glib::clone!(
            #[weak]
            parent,
            #[weak]
            dialog,
            #[strong]
            on_saved,
            move |_| {
                let (stx, srx) = async_channel::bounded(1);
                let (name_c, url_c, icon_c) = (name.clone(), url.clone(), icon_id.clone());
                crate::runtime().spawn(async move {
                    let path = match qwa_core::icon::iconify_png(&icon_c, 256).await {
                        Some(png) => qwa_core::icon::save_png(&name_c, &png),
                        None => {
                            let cands = qwa_core::icon::favicon_service_urls(&url_c);
                            qwa_core::icon::download_best(&name_c, &cands).await.ok()
                        }
                    };
                    let _ = stx.send(path).await;
                });
                glib::spawn_future_local(glib::clone!(
                    #[weak]
                    parent,
                    #[weak]
                    dialog,
                    #[strong]
                    on_saved,
                    #[strong]
                    name,
                    #[strong]
                    url,
                    async move {
                        let path = srx.recv().await.ok().flatten();
                        let mut app = WebApp::new(name.clone(), url.clone(), category);
                        app.icon_path = path;
                        app.external_links_in_browser = external_links;
                        app.show_badge = show_badge;
                        dialog.close();
                        editor::present(&parent, Some(app), false, move || (*on_saved)());
                    }
                ));
            }
        ));
    }

    dialog.present();
}

/// Clear and rebuild the app list. Called on startup and after any change.
/// A small (12x12) vertically-centered colored circle used to indicate an
/// app's login profile at a glance.
fn profile_dot(color: (u8, u8, u8)) -> gtk::DrawingArea {
    let (r, g, b) = color;
    let dot = gtk::DrawingArea::builder()
        .content_width(12)
        .content_height(12)
        .valign(gtk::Align::Center)
        .build();
    dot.set_draw_func(move |_area, cr, w, h| {
        let d = (w.min(h) as f64) - 2.0;
        let radius = (d / 2.0).max(0.0);
        cr.arc(
            w as f64 / 2.0,
            h as f64 / 2.0,
            radius,
            0.0,
            std::f64::consts::TAU,
        );
        cr.set_source_rgb(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
        let _ = cr.fill();
    });
    dot
}

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
        // Friendly profile label: the shared session key, or "Private" for a
        // per-app profile (None / blank).
        let profile_label = app
            .profile
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Private")
            .to_string();

        let row = adw::ActionRow::builder()
            .title(app.name.as_str())
            .subtitle(profile_label.as_str())
            .build();

        // A small colored dot indicating which login profile this app uses,
        // so the same profile is the same color across all its apps.
        let dot = profile_dot(qwa_core::profile_color(app.profile.as_deref()));
        row.add_prefix(&dot);

        // App icon on the left for quick visual scanning.
        let icon = gtk::Image::builder().pixel_size(32).build();
        match app.icon_path.as_ref().filter(|p| p.exists()) {
            Some(path) => icon.set_from_file(Some(path)),
            None => icon.set_icon_name(Some("application-x-addon-symbolic")),
        }
        row.add_prefix(&icon);

        let edit = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .tooltip_text("Edit")
            .build();
        let app_for_edit = app.clone();
        edit.connect_clicked(glib::clone!(
            #[weak]
            container,
            #[weak]
            window,
            move |_| {
                editor::present(
                    &window,
                    Some(app_for_edit.clone()),
                    true,
                    glib::clone!(
                        #[weak]
                        container,
                        #[weak]
                        window,
                        move || populate(&container, &window)
                    ),
                );
            }
        ));
        row.add_suffix(&edit);

        let delete = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .tooltip_text("Remove")
            .build();
        delete.connect_clicked(glib::clone!(
            #[weak]
            container,
            #[weak]
            window,
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
                        #[weak]
                        container,
                        #[weak]
                        window,
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
