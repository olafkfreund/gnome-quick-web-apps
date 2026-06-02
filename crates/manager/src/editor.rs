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
use qwa_core::{icon, launcher, Category, ColorScheme, LinkScope, WebApp};

/// Map a `ColorScheme` to its dropdown row index (order matches the model).
fn color_scheme_to_index(scheme: ColorScheme) -> u32 {
    match scheme {
        ColorScheme::System => 0,
        ColorScheme::Light => 1,
        ColorScheme::Dark => 2,
    }
}

/// Map a dropdown row index back to a `ColorScheme`.
fn color_scheme_from_index(index: u32) -> ColorScheme {
    match index {
        1 => ColorScheme::Light,
        2 => ColorScheme::Dark,
        _ => ColorScheme::System,
    }
}

/// Map a `LinkScope` to its dropdown row index (order matches `link_model`).
fn link_scope_to_index(scope: LinkScope) -> u32 {
    match scope {
        LinkScope::InWindow => 0,
        LinkScope::SameSite => 1,
        LinkScope::ExactHost => 2,
    }
}

/// Map a dropdown row index back to a `LinkScope`.
fn link_scope_from_index(index: u32) -> LinkScope {
    match index {
        1 => LinkScope::SameSite,
        2 => LinkScope::ExactHost,
        _ => LinkScope::InWindow,
    }
}

/// Open the editor over `parent`. `existing` = None creates a new app; Some
/// edits it in place. `on_saved` is called after a successful save.
pub fn present<F: Fn() + 'static>(
    parent: &adw::ApplicationWindow,
    prefill: Option<WebApp>,
    editing: bool,
    on_saved: F,
) {
    // `prefill` pre-populates fields; `editing` true keeps the existing id
    // (edit), false creates a new app (templates / blank new).
    let existing = prefill;
    let window = adw::Window::builder()
        .title(if editing {
            "Edit Web App"
        } else {
            "New Web App"
        })
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
    let chosen_icon: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(
        existing.as_ref().and_then(|a| a.icon_path.clone()),
    ));
    // A pre-filled icon (template or edit) counts as chosen, so finalize won't
    // overwrite it with an auto-downloaded favicon.
    let icon_picked = Rc::new(Cell::new(
        existing
            .as_ref()
            .and_then(|a| a.icon_path.as_ref())
            .is_some(),
    ));

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

    // Login profile dropdown: Private + detected browser profiles + shared.
    // Apps that pick the same profile share cookies/logins (sign in once).
    let (profile_labels, profile_values, profile_sel) =
        profile_options(existing.as_ref().and_then(|a| a.profile.as_deref()));
    let profile_strs: Vec<&str> = profile_labels.iter().map(String::as_str).collect();
    let profile_combo = adw::ComboRow::builder()
        .title("Login profile")
        .subtitle("Apps sharing a profile sign in once")
        .model(&gtk::StringList::new(&profile_strs))
        .build();
    profile_combo.set_selected(profile_sel);
    let profile_values = Rc::new(profile_values);

    // Icon row: preview + "Choose File…".
    let icon_img = gtk::Image::builder().pixel_size(32).build();
    set_icon_preview(&icon_img, chosen_icon.borrow().as_deref());
    let search_icon_btn = gtk::Button::builder()
        .icon_name("system-search-symbolic")
        .valign(gtk::Align::Center)
        .tooltip_text("Search icons online")
        .build();
    let choose_btn = gtk::Button::with_label("Choose File…");
    choose_btn.set_valign(gtk::Align::Center);
    let icon_row = adw::ActionRow::builder().title("Icon").build();
    icon_row.add_prefix(&icon_img);
    icon_row.add_suffix(&search_icon_btn);
    icon_row.add_suffix(&choose_btn);

    // Link handling (tri-state). InWindow is the safe default for multi-domain
    // logins (Microsoft/Slack SSO). Order must match `link_scope_from_index`.
    let link_model = gtk::StringList::new(&[
        "Open everything in this window",
        "Open other sites in browser (keep sibling subdomains)",
        "Open other sites in browser (this exact site only)",
    ]);
    let link_row = adw::ComboRow::builder()
        .title("Links to other sites")
        .model(&link_model)
        .build();
    link_row.set_selected(link_scope_to_index(
        existing
            .as_ref()
            .map(|a| a.link_scope())
            .unwrap_or_default(),
    ));

    // Appearance override (forces the site's prefers-color-scheme + chrome).
    let appearance_model = gtk::StringList::new(&["Follow system", "Light", "Dark"]);
    let appearance_row = adw::ComboRow::builder()
        .title("Appearance")
        .model(&appearance_model)
        .build();
    appearance_row.set_selected(color_scheme_to_index(
        existing
            .as_ref()
            .map(|a| a.color_scheme)
            .unwrap_or_default(),
    ));

    // "Set as default for…" toggles, rebuilt from the URL (email for web mail,
    // calendar for web calendars, nothing otherwise).
    let handlers_group = adw::PreferencesGroup::builder()
        .title("Set as default for…")
        .build();
    let role_switches: Rc<RefCell<Vec<RoleSwitch>>> = Rc::new(RefCell::new(Vec::new()));
    let existing_handlers: Vec<qwa_core::webapp::UrlHandler> = existing
        .as_ref()
        .map(|a| a.handlers.clone())
        .unwrap_or_default();

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
    group.add(&profile_combo);
    group.add(&icon_row);
    group.add(&link_row);
    group.add(&appearance_row);

    let page = adw::PreferencesPage::new();
    page.add(&group);
    page.add(&handlers_group);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&page);
    window.set_content(Some(&content));

    // Build the default-handler toggles from the current URL, and keep them in
    // sync as the URL changes.
    rebuild_handler_rows(
        &handlers_group,
        &role_switches,
        &url_row.text(),
        &existing_handlers,
    );
    url_row.connect_changed(glib::clone!(
        #[weak]
        handlers_group,
        #[strong]
        role_switches,
        #[strong]
        existing_handlers,
        move |row| {
            rebuild_handler_rows(
                &handlers_group,
                &role_switches,
                &row.text(),
                &existing_handlers,
            );
        }
    ));

    cancel.connect_clicked(glib::clone!(
        #[weak]
        window,
        move |_| window.close()
    ));

    // --- Choose an icon file. ---
    choose_btn.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[weak]
        icon_img,
        #[strong]
        chosen_icon,
        #[strong]
        icon_picked,
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
                    #[weak]
                    icon_img,
                    #[strong]
                    chosen_icon,
                    #[strong]
                    icon_picked,
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

    // --- Search icons online (Iconify). ---
    search_icon_btn.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[weak]
        icon_img,
        #[weak]
        name_row,
        #[strong]
        chosen_icon,
        #[strong]
        icon_picked,
        move |_| {
            present_icon_search(
                &window,
                &name_row.text(),
                glib::clone!(
                    #[weak]
                    icon_img,
                    #[strong]
                    chosen_icon,
                    #[strong]
                    icon_picked,
                    move |path: PathBuf| {
                        set_icon_preview(&icon_img, Some(&path));
                        *chosen_icon.borrow_mut() = Some(path);
                        icon_picked.set(true);
                    }
                ),
            );
        }
    ));

    // --- Detect manifest info. ---
    detect_btn.connect_clicked(glib::clone!(
        #[weak]
        url_row,
        #[weak]
        name_row,
        #[weak]
        detect_btn,
        #[weak]
        spinner,
        #[strong]
        detected,
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
    // When editing, keep the original (its id); for templates/new, start fresh.
    let existing_for_save = if editing { existing.clone() } else { None };
    save.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[weak]
        url_row,
        #[weak]
        name_row,
        #[weak]
        cat_row,
        #[weak]
        profile_combo,
        #[weak]
        link_row,
        #[weak]
        appearance_row,
        #[strong]
        role_switches,
        #[strong]
        profile_values,
        #[strong]
        detected,
        #[strong]
        chosen_icon,
        #[strong]
        icon_picked,
        move |_| {
            let url = url_row.text().to_string();
            let name = name_row.text().trim().to_string();
            let (profile, import_from) = profile_values
                .get(profile_combo.selected() as usize)
                .cloned()
                .unwrap_or((None, None));
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
                    a.profile = profile;
                    a
                }
                None => {
                    let mut a = WebApp::new(name, url, category);
                    a.profile = profile;
                    a
                }
            };
            let scope = link_scope_from_index(link_row.selected());
            app.link_scope = Some(scope);
            // Keep the legacy bool in sync so older runner builds still behave.
            app.external_links_in_browser = scope != qwa_core::LinkScope::InWindow;
            app.color_scheme = color_scheme_from_index(appearance_row.selected());
            app.handlers = role_switches
                .borrow()
                .iter()
                .filter(|(_, _, sw)| sw.is_active())
                .map(|(mime, template, _)| qwa_core::webapp::UrlHandler {
                    mime: mime.clone(),
                    template: template.clone(),
                })
                .collect();

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
            tracing::info!(
                "{} web app {}",
                if editing { "edited" } else { "created" },
                app.id
            );
            finalize_async(app, candidates, auto, import_from.clone());
            on_saved();
            window.close();
        }
    ));

    window.present();
}

/// Modal icon-search dialog: type a keyword, browse Iconify results, click one
/// to set it as the app icon (downloaded + rasterized to PNG).
fn present_icon_search<F: Fn(PathBuf) + 'static>(parent: &adw::Window, initial: &str, on_pick: F) {
    let on_pick = Rc::new(on_pick);

    let dialog = adw::Window::builder()
        .title("Search Icons")
        .modal(true)
        .transient_for(parent)
        .default_width(540)
        .default_height(560)
        .build();

    let header = adw::HeaderBar::new();
    let search = gtk::SearchEntry::new();
    search.set_hexpand(true);
    search.set_placeholder_text(Some("Search icons…"));
    header.set_title_widget(Some(&search));

    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .max_children_per_line(8)
        .column_spacing(6)
        .row_spacing(6)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&flow)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&scroller);
    dialog.set_content(Some(&content));

    let run_search = Rc::new(glib::clone!(
        #[weak]
        flow,
        #[weak]
        dialog,
        #[strong]
        on_pick,
        move |query: String| {
            while let Some(child) = flow.first_child() {
                flow.remove(&child);
            }
            if query.trim().len() < 2 {
                return;
            }

            let (tx, rx) = async_channel::bounded(1);
            crate::runtime().spawn(async move {
                let _ = tx.send(qwa_core::icon::search_iconify(&query).await).await;
            });

            glib::spawn_future_local(glib::clone!(
                #[weak]
                flow,
                #[weak]
                dialog,
                #[strong]
                on_pick,
                async move {
                    let Ok(ids) = rx.recv().await else {
                        return;
                    };
                    for id in ids {
                        let img = gtk::Image::builder().pixel_size(48).build();
                        img.set_icon_name(Some("content-loading-symbolic"));
                        let btn = gtk::Button::builder()
                            .css_classes(["flat"])
                            .tooltip_text(&id)
                            .child(&img)
                            .build();
                        flow.insert(&btn, -1);

                        // Thumbnail (rasterized PNG -> texture).
                        let (ttx, trx) = async_channel::bounded(1);
                        let id_t = id.clone();
                        crate::runtime().spawn(async move {
                            let _ = ttx.send(qwa_core::icon::iconify_png(&id_t, 48).await).await;
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

                        // Selection: download at full size, save, hand back.
                        btn.connect_clicked(glib::clone!(
                            #[weak]
                            dialog,
                            #[strong]
                            on_pick,
                            move |_| {
                                let id_s = id.clone();
                                let (stx, srx) = async_channel::bounded(1);
                                crate::runtime().spawn(async move {
                                    let path = match qwa_core::icon::iconify_png(&id_s, 256).await {
                                        Some(png) => qwa_core::icon::save_png(&id_s, &png),
                                        None => None,
                                    };
                                    let _ = stx.send(path).await;
                                });
                                glib::spawn_future_local(glib::clone!(
                                    #[weak]
                                    dialog,
                                    #[strong]
                                    on_pick,
                                    async move {
                                        if let Ok(Some(path)) = srx.recv().await {
                                            (*on_pick)(path);
                                            dialog.close();
                                        }
                                    }
                                ));
                            }
                        ));
                    }
                }
            ));
        }
    ));

    search.connect_search_changed(glib::clone!(
        #[strong]
        run_search,
        move |entry| (*run_search)(entry.text().to_string())
    ));

    search.set_text(initial);
    (*run_search)(initial.trim().to_string());

    dialog.present();
}

fn set_icon_preview(img: &gtk::Image, path: Option<&std::path::Path>) {
    match path {
        Some(p) if p.exists() => img.set_from_file(Some(p)),
        _ => img.set_icon_name(Some("application-x-addon-symbolic")),
    }
}

/// Build the login-profile dropdown: "Private", detected browser profiles,
/// and any shared profiles already used by other apps. Returns the labels,
/// the matching profile values (None = private), and the index to preselect
/// for `current`.
/// Each option carries (profile key, optional session-import source dir).
type ProfileChoice = (Option<String>, Option<PathBuf>);

fn profile_options(current: Option<&str>) -> (Vec<String>, Vec<ProfileChoice>, u32) {
    use std::collections::HashSet;

    let mut labels = vec!["Private (this app only)".to_string()];
    let mut values: Vec<ProfileChoice> = vec![(None, None)];

    for p in qwa_core::profiles::detect() {
        labels.push(format!("{} — {}", p.browser, p.display));
        // Only Chromium-family sessions can be imported into the CEF runner.
        let import = p.chromium.then_some(p.path);
        values.push((Some(p.key), import));
    }

    let mut seen: HashSet<String> = values.iter().filter_map(|(k, _)| k.clone()).collect();
    for app in WebApp::load_all() {
        if let Some(pr) = app.profile.as_deref() {
            if !pr.is_empty() && seen.insert(pr.to_string()) {
                labels.push(format!("Shared: {pr}"));
                values.push((Some(pr.to_string()), None));
            }
        }
    }

    let mut selected = 0;
    if let Some(cur) = current.filter(|c| !c.is_empty()) {
        match values.iter().position(|(k, _)| k.as_deref() == Some(cur)) {
            Some(idx) => selected = idx as u32,
            None => {
                labels.push(format!("Shared: {cur}"));
                values.push((Some(cur.to_string()), None));
                selected = (labels.len() - 1) as u32;
            }
        }
    }

    (labels, values, selected)
}

fn is_http_url(url: &str) -> bool {
    url::Url::parse(url)
        .map(|u| matches!(u.scheme(), "http" | "https"))
        .unwrap_or(false)
}

/// Background tail of save: optionally download a better icon, then (re)install
/// the launcher via the portal.
fn finalize_async(
    mut app: WebApp,
    candidates: Vec<String>,
    auto: bool,
    import_from: Option<PathBuf>,
) {
    crate::runtime().spawn(async move {
        // Best-effort: seed this app's (shared) profile from a browser profile.
        if let Some(src) = import_from {
            let dest = qwa_core::paths::profile_dir(app.profile_key());
            qwa_core::profiles::import_session(&src, &dest);
        }
        if auto && !candidates.is_empty() {
            match icon::download_best(&app.id, &candidates).await {
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
            return;
        }
        // Register as the system default for any selected scheme handlers.
        if !app.handlers.is_empty() {
            launcher::set_as_default_handlers(&app);
        }
    });
}

/// (mime, template, switch) for one default-handler role currently shown.
type RoleSwitch = (String, String, adw::SwitchRow);

/// Rebuild the "Set as default for…" toggles for `url`. Hidden when the URL
/// isn't a default handler for anything (e.g. Google Drive).
fn rebuild_handler_rows(
    group: &adw::PreferencesGroup,
    switches: &Rc<RefCell<Vec<RoleSwitch>>>,
    url: &str,
    existing: &[qwa_core::webapp::UrlHandler],
) {
    for (_, _, sw) in switches.borrow().iter() {
        group.remove(sw);
    }
    switches.borrow_mut().clear();

    let roles = qwa_core::handlers::roles_for(url);
    group.set_visible(!roles.is_empty());
    for role in roles {
        let active = existing.iter().any(|h| h.mime == role.mime);
        let sw = adw::SwitchRow::builder()
            .title(&role.label)
            .subtitle(&role.subtitle)
            .active(active)
            .build();
        group.add(&sw);
        switches.borrow_mut().push((role.mime, role.template, sw));
    }
}
