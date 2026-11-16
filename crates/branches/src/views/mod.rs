pub mod column_view;
pub mod gallery_view;
pub mod icon_view;
pub mod list_view;

use std::path::PathBuf;

pub type NavigateCallback = std::rc::Rc<dyn Fn(PathBuf)>;
pub type SelectionCallback = std::rc::Rc<dyn Fn(usize)>;
pub type RenameCallback = std::rc::Rc<dyn Fn(PathBuf)>;

/// Crea un botón de menú contextual estilizado como ítem unificado de Malus DE.
pub(super) fn make_context_menu_button(label: &str) -> gtk4::Button {
    use gtk4::prelude::*;

    let btn = gtk4::Button::builder()
        .label(label)
        .has_frame(false)
        .css_classes(["popover-menu-item"])
        .build();
    if let Some(child) = btn.child() {
        if let Some(lbl) = child.downcast_ref::<gtk4::Label>() {
            lbl.set_xalign(0.0);
        }
    }
    btn
}

/// Construye el `ContentProvider` de arrastre exponiendo tanto `GdkFileList` como `text/uri-list`
/// para garantizar interoperabilidad Wayland con otras ventanas (como el Dock).
pub(super) fn build_file_drag_content_provider(path: &std::path::Path) -> gtk4::gdk::ContentProvider {
    use gtk4::prelude::*;

    let file = gio::File::for_path(path);
    let uri_payload = format!("{}\r\n", file.uri());
    let file_list = gtk4::gdk::FileList::from_array(&[file]);
    gtk4::gdk::ContentProvider::new_union(&[
        gtk4::gdk::ContentProvider::for_value(&file_list.to_value()),
        gtk4::gdk::ContentProvider::for_bytes(
            "text/uri-list",
            &glib::Bytes::from_owned(uri_payload),
        ),
    ])
}

/// Refresca la vista origen si un archivo arrastrado fue movido fuera de la carpeta actual
/// (por ejemplo, hacia el escritorio o la otra vista en Split View).
pub(super) fn refresh_if_moved_out(
    drag: &gtk4::gdk::Drag,
    item_path: &std::path::Path,
    reload_dir: &std::path::Path,
    on_navigate: &NavigateCallback,
) -> bool {
    use gtk4::prelude::*;

    if !item_path.exists() {
        on_navigate(reload_dir.to_path_buf());
        return true;
    }

    if drag.selected_action() == gtk4::gdk::DragAction::MOVE {
        let path_check = item_path.to_path_buf();
        let dir_reload = reload_dir.to_path_buf();
        let nav_cb = on_navigate.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || {
            if !path_check.exists() {
                nav_cb(dir_reload);
            }
        });
    }
    false
}

pub(super) fn is_dock_app_pinned(
    cfg: &malus_de_common::config::DockConfig,
    app: &malus_de_common::config::DockAppConfig,
) -> bool {
    cfg.apps.iter().any(|existing| {
        existing.name.eq_ignore_ascii_case(&app.name)
            || (!existing.command.is_empty() && existing.command == app.command)
    })
}

fn set_app_pinned_in_dock(app: malus_de_common::config::DockAppConfig, pin: bool) {
    use malus_de_common::config::DockConfig;
    use malus_de_common::ipc::{send_ipc_message, IpcMessage};

    let ipc_msg = if pin {
        IpcMessage::PinAppToDock {
            name: app.name.clone(),
            icon: app.icon.clone(),
            command: app.command.clone(),
            matchers: app.matchers.clone(),
            insert_at_dock_x: None,
        }
    } else {
        IpcMessage::UnpinAppFromDock {
            name: app.name.clone(),
        }
    };

    if send_ipc_message(&ipc_msg).is_err() {
        let mut cfg = DockConfig::load_or_create_default();
        let changed = if pin {
            cfg.pin_app(app)
        } else {
            cfg.unpin_app(&app.name)
        };
        if changed {
            let _ = cfg.save();
        }
    }
}

fn is_inside_trash(path: &std::path::Path) -> bool {
    path.parent().is_some_and(|p| p.ends_with("Trash/files"))
}

pub(super) fn can_create_in_folder(folder: &std::path::Path) -> bool {
    !folder.ends_with("Trash/files") && folder.is_dir()
}

fn make_submenu_trigger_button(label: &str) -> gtk4::Button {
    use gtk4::prelude::*;

    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    let title = gtk4::Label::builder()
        .label(label)
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .build();
    let chevron = gtk4::Label::builder()
        .label("›")
        .css_classes(["dim-label"])
        .build();
    row.append(&title);
    row.append(&chevron);

    gtk4::Button::builder()
        .has_frame(false)
        .css_classes(["popover-menu-item"])
        .child(&row)
        .build()
}

/// Agrega la entrada «Crear nuevo ›» y su submenú (Carpeta / Documento de texto) al Stack del menú contextual.
pub(super) fn append_create_new_submenu(
    popover: &gtk4::Popover,
    main_box: &gtk4::Box,
    stack: &gtk4::Stack,
    target_dir: PathBuf,
    on_created: RenameCallback,
) {
    use gtk4::prelude::*;

    let es = malus_de_common::is_spanish_locale();
    let trigger_btn = make_submenu_trigger_button(if es { "Crear nuevo" } else { "Create New" });
    {
        let st = stack.clone();
        trigger_btn.connect_clicked(move |_| {
            st.set_visible_child_name("create_new");
        });
    }
    main_box.append(&trigger_btn);

    let sub_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);

    let back_btn = make_context_menu_button(if es { "← Volver" } else { "← Back" });
    {
        let st = stack.clone();
        back_btn.connect_clicked(move |_| {
            st.set_visible_child_name("main");
        });
    }
    sub_box.append(&back_btn);
    sub_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

    let folder_btn = make_context_menu_button(if es { "Carpeta" } else { "Folder" });
    {
        let pop = popover.clone();
        let dir = target_dir.clone();
        let cb = on_created.clone();
        let default_name = if es { "Nueva carpeta" } else { "New Folder" };
        folder_btn.connect_clicked(move |_| {
            pop.popdown();
            if let Ok(created) = crate::operations::create_new_folder(&dir, default_name) {
                cb(created);
            }
        });
    }
    sub_box.append(&folder_btn);

    let doc_btn = make_context_menu_button(if es {
        "Documento de texto"
    } else {
        "Text Document"
    });
    {
        let pop = popover.clone();
        let dir = target_dir;
        let cb = on_created;
        let default_stem = if es {
            "Nuevo documento"
        } else {
            "Untitled Document"
        };
        doc_btn.connect_clicked(move |_| {
            pop.popdown();
            if let Ok(created) = crate::operations::create_new_text_document(&dir, default_stem) {
                cb(created);
            }
        });
    }
    sub_box.append(&doc_btn);

    stack.add_named(&sub_box, Some("create_new"));
}

/// Menú contextual genérico de fondo para vistas sin opciones espaciales (Lista, Columnas, Galería).
pub(super) fn show_background_create_menu(
    anchor: &impl gtk4::prelude::IsA<gtk4::Widget>,
    x: f64,
    y: f64,
    target_dir: PathBuf,
    on_created: RenameCallback,
) {
    use gtk4::gdk;
    use gtk4::prelude::*;

    if anchor.root().is_none() || !can_create_in_folder(&target_dir) {
        return;
    }

    let popover = gtk4::Popover::builder()
        .has_arrow(false)
        .css_classes(["malus-popover-menu"])
        .build();
    popover.set_parent(anchor);
    let rect = gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1);
    popover.set_pointing_to(Some(&rect));
    popover.connect_closed(|p| p.unparent());

    let stack = gtk4::Stack::builder()
        .transition_type(gtk4::StackTransitionType::SlideLeftRight)
        .transition_duration(160)
        .vhomogeneous(false)
        .build();
    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    stack.add_named(&main_box, Some("main"));

    append_create_new_submenu(&popover, &main_box, &stack, target_dir, on_created);

    stack.set_visible_child_name("main");
    popover.set_child(Some(&stack));
    popover.popup();
}

/// Muestra un popover compacto estilo macOS sobre el elemento para cambiar su nombre.
pub(crate) fn show_rename_popover(
    anchor: &impl gtk4::prelude::IsA<gtk4::Widget>,
    path: PathBuf,
    is_dir: bool,
    pointing_to: Option<(f64, f64)>,
    on_renamed: Option<RenameCallback>,
) {
    use gtk4::gdk;
    use gtk4::prelude::*;

    if anchor.root().is_none() || is_inside_trash(&path) {
        return;
    }

    let es = malus_de_common::is_spanish_locale();
    let initial_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if initial_name.is_empty() {
        return;
    }

    let popover = gtk4::Popover::builder()
        .has_arrow(false)
        .autohide(true)
        .css_classes(["malus-popover-menu"])
        .build();
    popover.set_parent(anchor);
    if let Some((x, y)) = pointing_to {
        let rect = gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1);
        popover.set_pointing_to(Some(&rect));
    }
    popover.connect_closed(|p| p.unparent());

    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);
    container.set_width_request(240);

    let title_lbl = gtk4::Label::builder()
        .label(if es { "Renombrar" } else { "Rename" })
        .halign(gtk4::Align::Start)
        .css_classes(["popover-menu-header"])
        .build();
    container.append(&title_lbl);

    let entry = gtk4::Entry::builder()
        .text(&initial_name)
        .hexpand(true)
        .width_chars(24)
        .build();
    container.append(&entry);

    let error_lbl = gtk4::Label::builder()
        .halign(gtk4::Align::Start)
        .wrap(true)
        .visible(false)
        .css_classes(["error"])
        .build();
    container.append(&error_lbl);

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    btn_box.set_halign(gtk4::Align::End);

    let cancel_btn = gtk4::Button::builder()
        .label(if es { "Cancelar" } else { "Cancel" })
        .css_classes(["popover-menu-item"])
        .build();
    {
        let pop = popover.clone();
        cancel_btn.connect_clicked(move |_| pop.popdown());
    }
    btn_box.append(&cancel_btn);

    let save_btn = gtk4::Button::builder()
        .label(if es { "Renombrar" } else { "Rename" })
        .css_classes(["popover-menu-item", "suggested-action"])
        .build();
    {
        let pop = popover.clone();
        let entry_ref = entry.clone();
        let err_ref = error_lbl.clone();
        let old_path = path.clone();
        save_btn.connect_clicked(move |_| {
            let candidate = entry_ref.text();
            match crate::operations::rename_item(&old_path, candidate.as_str()) {
                Ok(new_path) => {
                    pop.popdown();
                    if new_path != old_path {
                        crate::sidebar::refresh_all_sidebars();
                        if let Some(ref cb) = on_renamed {
                            cb(new_path);
                        }
                    }
                }
                Err(msg) => {
                    err_ref.set_text(&msg);
                    err_ref.set_visible(true);
                    entry_ref.grab_focus();
                }
            }
        });
    }
    btn_box.append(&save_btn);
    container.append(&btn_box);

    {
        let err_clear = error_lbl.clone();
        entry.connect_changed(move |_| {
            err_clear.set_visible(false);
        });
    }
    {
        let submit_btn = save_btn.clone();
        entry.connect_activate(move |_| {
            submit_btn.emit_clicked();
        });
    }

    popover.set_child(Some(&container));
    popover.popup();

    let entry_focus = entry.clone();
    let sel_end = crate::operations::rename_selection_end(&initial_name, is_dir);
    glib::idle_add_local_once(move || {
        entry_focus.grab_focus();
        entry_focus.select_region(0, sel_end);
    });
}

/// Menú contextual unificado para elementos en Branches (archivos, carpetas y aplicaciones `.desktop`).
pub(super) fn show_item_context_menu(
    anchor: &impl gtk4::prelude::IsA<gtk4::Widget>,
    path: &std::path::Path,
    is_dir: bool,
    pointing_to: Option<(f64, f64)>,
    on_navigate: Option<&NavigateCallback>,
    on_renamed: Option<RenameCallback>,
) {
    use gtk4::gdk;
    use gtk4::prelude::*;

    let es = malus_de_common::is_spanish_locale();
    let popover = gtk4::Popover::builder()
        .has_arrow(false)
        .css_classes(["malus-popover-menu"])
        .build();
    popover.set_parent(anchor);
    if let Some((x, y)) = pointing_to {
        let rect = gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1);
        popover.set_pointing_to(Some(&rect));
    }
    popover.connect_closed(|p| p.unparent());

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);

    let desktop_app = if !is_dir {
        malus_de_common::apps::desktop_file_to_dock_app(path)
    } else {
        None
    };

    // 1. Acción principal: Abrir carpeta / Abrir aplicación / Abrir archivo
    let open_label = if is_dir {
        if es { "Abrir carpeta" } else { "Open Folder" }
    } else if desktop_app.is_some() {
        if es { "Abrir aplicación" } else { "Open Application" }
    } else if es {
        "Abrir"
    } else {
        "Open"
    };
    let open_btn = make_context_menu_button(open_label);
    {
        let p = path.to_path_buf();
        let nav_opt = on_navigate.cloned();
        let pop = popover.clone();
        open_btn.connect_clicked(move |_| {
            pop.popdown();
            if is_dir {
                if let Some(ref nav) = nav_opt {
                    nav(p.clone());
                    return;
                }
            }
            let _ = crate::fs_backend::open_file(&p);
        });
    }
    vbox.append(&open_btn);

    // 2. Acciones exclusivas para lanzadores .desktop (Aplicaciones)
    if let Some(app_cfg) = desktop_app {
        let dock_cfg = malus_de_common::config::DockConfig::load_or_create_default();
        let currently_pinned = is_dock_app_pinned(&dock_cfg, &app_cfg);
        let dock_label = if currently_pinned {
            if es { "Quitar del Dock" } else { "Remove from Dock" }
        } else if es {
            "Fijar en el Dock"
        } else {
            "Keep in Dock"
        };
        let dock_btn = make_context_menu_button(dock_label);
        {
            let pop = popover.clone();
            dock_btn.connect_clicked(move |_| {
                pop.popdown();
                set_app_pinned_in_dock(app_cfg.clone(), !currently_pinned);
            });
        }
        vbox.append(&dock_btn);
    }

    // 3. Mostrar carpeta contenedora (útil en Aplicaciones y Recientes)
    if let (Some(parent), Some(nav)) = (path.parent(), on_navigate) {
        if !is_dir {
            let enclosing_btn = make_context_menu_button(if es {
                "Mostrar carpeta contenedora"
            } else {
                "Show Enclosing Folder"
            });
            let parent_path = parent.to_path_buf();
            let nav_cb = nav.clone();
            let pop = popover.clone();
            enclosing_btn.connect_clicked(move |_| {
                pop.popdown();
                nav_cb(parent_path.clone());
            });
            vbox.append(&enclosing_btn);
        }
    }

    // 4. Abrir en Terminal (para carpetas y archivos ordinarios)
    if is_dir || path.extension().and_then(|e| e.to_str()) != Some("desktop") {
        let term_btn = make_context_menu_button(if es {
            "Abrir en Terminal"
        } else {
            "Open in Terminal"
        });
        let p_term = path.to_path_buf();
        let pop = popover.clone();
        term_btn.connect_clicked(move |_| {
            pop.popdown();
            crate::fs_backend::open_in_terminal(&p_term);
        });
        vbox.append(&term_btn);
    }

    // 5. Renombrar carpeta, archivo o icono
    if !is_inside_trash(path) && path.parent().is_some() {
        let rename_btn = make_context_menu_button(if es { "Renombrar…" } else { "Rename…" });
        let anchor_widget = anchor.clone().upcast::<gtk4::Widget>();
        let p_rename = path.to_path_buf();
        let pop = popover.clone();
        let nav_opt = on_navigate.cloned();
        let rename_cb = on_renamed.or_else(|| {
            let parent = p_rename.parent().map(|p| p.to_path_buf());
            match (parent, nav_opt) {
                (Some(parent_dir), Some(nav)) => Some(std::rc::Rc::new(move |_new_path| {
                    nav(parent_dir.clone());
                }) as RenameCallback),
                _ => None,
            }
        });
        rename_btn.connect_clicked(move |_| {
            pop.popdown();
            let aw = anchor_widget.clone();
            let pr = p_rename.clone();
            let cb = rename_cb.clone();
            glib::idle_add_local_once(move || {
                show_rename_popover(&aw, pr, is_dir, pointing_to, cb);
            });
        });
        vbox.append(&rename_btn);
    }

    // 6. Añadir / Quitar de marcadores (para carpetas y archivos)
    if !is_inside_trash(path) && path != std::path::Path::new("/") {
        let already_bookmarked = crate::db::open_db()
            .map(|conn| crate::db::is_bookmark(&conn, path))
            .unwrap_or(false);
        let bm_label = if already_bookmarked {
            if es {
                "Quitar de marcadores"
            } else {
                "Remove Bookmark"
            }
        } else if es {
            "Añadir a marcadores"
        } else {
            "Add to Bookmarks"
        };
        let bm_btn = make_context_menu_button(bm_label);
        let p_bm = path.to_path_buf();
        let pop = popover.clone();
        bm_btn.connect_clicked(move |_| {
            pop.popdown();
            if let Ok(conn) = crate::db::open_db() {
                let changed = if already_bookmarked {
                    crate::db::remove_bookmark(&conn, &p_bm).unwrap_or(false)
                } else {
                    crate::db::add_bookmark(&conn, &p_bm, None).unwrap_or(false)
                };
                if changed {
                    crate::sidebar::refresh_all_sidebars();
                }
            }
        });
        vbox.append(&bm_btn);
    }

    // 7. Copiar ruta
    let copy_btn = make_context_menu_button(if es { "Copiar ruta" } else { "Copy Path" });
    {
        let p_copy = path.to_path_buf();
        let pop = popover.clone();
        copy_btn.connect_clicked(move |_| {
            pop.popdown();
            if let Some(display) = gdk::Display::default() {
                display.clipboard().set_text(&p_copy.to_string_lossy());
            }
        });
    }
    vbox.append(&copy_btn);

    popover.set_child(Some(&vbox));
    popover.popup();
}

#[cfg(test)]
mod tests {
    use super::is_dock_app_pinned;
    use malus_de_common::config::{DockAppConfig, DockConfig, DockIconAppearance};

    #[test]
    fn dock_app_pinned_detection_matches_by_name_or_command() {
        let app = DockAppConfig {
            name: "Firefox".to_string(),
            icon: "firefox".to_string(),
            command: "/usr/bin/firefox".to_string(),
            matchers: vec!["firefox".to_string()],
            icon_appearance: DockIconAppearance::Auto,
        };
        let mut cfg = DockConfig {
            apps: Vec::new(),
            ..DockConfig::default()
        };
        assert!(!is_dock_app_pinned(&cfg, &app));
        assert!(cfg.pin_app(app.clone()));
        assert!(is_dock_app_pinned(&cfg, &app));
    }
}
