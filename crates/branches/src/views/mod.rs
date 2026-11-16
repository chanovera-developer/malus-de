pub mod column_view;
pub mod gallery_view;
pub mod icon_view;
pub mod list_view;

use std::path::PathBuf;

pub type NavigateCallback = std::rc::Rc<dyn Fn(PathBuf)>;
pub type SelectionCallback = std::rc::Rc<dyn Fn(usize)>;

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

/// Menú contextual unificado para elementos en Branches (archivos, carpetas y aplicaciones `.desktop`).
pub(super) fn show_item_context_menu(
    anchor: &impl gtk4::prelude::IsA<gtk4::Widget>,
    path: &std::path::Path,
    is_dir: bool,
    pointing_to: Option<(f64, f64)>,
    on_navigate: Option<&NavigateCallback>,
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

    // 5. Copiar ruta
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
