use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Image, Label, MenuButton, Orientation, Popover, Scale,
    SearchEntry, Separator, Switch, Window,
};
use libadwaita as adw;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use tracing::{info, warn};

/// Colores de etiquetas de Branches / macOS Finder (Rojo, Naranja, Amarillo, Verde, Azul, Púrpura, Gris).
const TAG_DOT_COLORS: [(f64, f64, f64); 7] = [
    (1.00, 0.37, 0.34), // Rojo
    (1.00, 0.62, 0.04), // Naranja
    (1.00, 0.84, 0.04), // Amarillo
    (0.19, 0.82, 0.35), // Verde
    (0.04, 0.52, 1.00), // Azul
    (0.75, 0.35, 0.95), // Púrpura
    (0.56, 0.56, 0.58), // Gris
];

fn is_spanish_locale() -> bool {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default()
        .to_lowercase()
        .starts_with("es")
}

/// Determina si la ventana activa pertenece a la aplicación Branches.
pub fn is_branches_app(app_id: Option<&str>, title: Option<&str>) -> bool {
    if let Some(id) = app_id.map(str::trim).filter(|s| !s.is_empty()) {
        let lower = id.to_ascii_lowercase();
        if lower == "dev.malus.branches"
            || lower.starts_with("dev.malus.branches.")
            || lower == "malus-de-branches"
            || lower == "branches"
        {
            return true;
        }
        return false;
    }

    title
        .map(str::trim)
        .is_some_and(|t| t.eq_ignore_ascii_case("branches"))
}

/// Títulos de las categorías de la barra de menús de Branches (Finder1..Finder7).
pub fn branches_category_titles(es: bool) -> &'static [&'static str] {
    if es {
        &[
            "Archivo",
            "Edición",
            "Visualización",
            "Ir",
            "Ventana",
            "Ayuda",
        ]
    } else {
        &["File", "Edit", "View", "Go", "Window", "Help"]
    }
}

/// Etiquetas esperadas en el menú de aplicación «Branches» (Finder1.png).
pub fn branches_app_menu_labels(es: bool) -> [&'static str; 7] {
    if es {
        [
            "Acerca de Branches",
            "Configuración...",
            "Vaciar basurero...",
            "Servicios",
            "Ocultar Branches",
            "Ocultar otros",
            "Mostrar todo",
        ]
    } else {
        [
            "About Branches",
            "Settings...",
            "Empty Trash...",
            "Services",
            "Hide Branches",
            "Hide Others",
            "Show All",
        ]
    }
}

fn create_divider() -> Separator {
    let sep = Separator::new(Orientation::Horizontal);
    sep.add_css_class("menu-separator");
    sep
}

fn create_menu_item(
    icon_name: Option<&str>,
    label_text: &str,
    shortcut: Option<&str>,
    has_submenu: bool,
    enabled: bool,
) -> Button {
    let (btn, _) = crate::ui::system_menu::create_menu_item_widget(
        icon_name,
        label_text,
        shortcut,
        has_submenu,
    );
    btn.set_sensitive(enabled);
    btn
}

fn create_check_menu_item(
    checked: bool,
    icon_name: Option<&str>,
    label_text: &str,
    shortcut: Option<&str>,
    enabled: bool,
) -> Button {
    let btn = Button::builder()
        .css_classes(["popover-menu-item"])
        .has_frame(false)
        .sensitive(enabled)
        .build();

    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .valign(Align::Center)
        .build();

    if checked {
        let check = Image::builder()
            .icon_name("object-select-symbolic")
            .pixel_size(12)
            .width_request(14)
            .valign(Align::Center)
            .css_classes(["menu-item-icon"])
            .build();
        row.append(&check);
    } else {
        let spacer = GtkBox::builder().width_request(14).build();
        row.append(&spacer);
    }

    if let Some(icon_id) = icon_name {
        let icon = Image::builder()
            .icon_name(icon_id)
            .pixel_size(14)
            .valign(Align::Center)
            .css_classes(["menu-item-icon"])
            .build();
        row.append(&icon);
    }

    let label = Label::builder()
        .label(label_text)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["menu-row-label"])
        .build();
    row.append(&label);

    if let Some(sc) = shortcut {
        let sc_label = Label::builder()
            .label(sc)
            .xalign(1.0)
            .halign(Align::End)
            .css_classes(["menu-shortcut-hint"])
            .build();
        row.append(&sc_label);
    }

    btn.set_child(Some(&row));
    crate::ui::system_menu::attach_squircle_menu_hover(&btn);
    btn
}

fn create_trailing_glyph_menu_item(
    icon_name: Option<&str>,
    label_text: &str,
    trailing_glyph: &str,
    enabled: bool,
) -> Button {
    create_menu_item(icon_name, label_text, Some(trailing_glyph), false, enabled)
}

fn attach_cascading_submenu(
    parent_popover: &Popover,
    menu_vbox: &GtkBox,
    trigger_btn: &Button,
    submenu_content: &GtkBox,
    tracked_submenus: &Rc<RefCell<Vec<Popover>>>,
) -> Popover {
    submenu_content.set_vexpand(false);
    submenu_content.set_valign(Align::Start);

    let sub_pop = Popover::builder()
        .has_arrow(false)
        .autohide(false)
        .position(gtk4::PositionType::Right)
        .css_classes(["malus-popover-menu"])
        .build();
    sub_pop.set_offset(4, 0);
    sub_pop.set_vexpand(false);
    sub_pop.set_valign(Align::Start);
    sub_pop.set_child(Some(submenu_content));
    sub_pop.set_parent(menu_vbox);
    crate::ui::theme::register_popover(&sub_pop);

    tracked_submenus.borrow_mut().push(sub_pop.clone());

    let open_submenu: Rc<dyn Fn()> = {
        let sub_w = sub_pop.downgrade();
        let btn_w = trigger_btn.downgrade();
        let vbox_w = menu_vbox.downgrade();
        let parent_w = parent_popover.downgrade();
        let subs = tracked_submenus.clone();
        Rc::new(move || {
            let (Some(sp), Some(b), Some(vb), Some(parent)) = (
                sub_w.upgrade(),
                btn_w.upgrade(),
                vbox_w.upgrade(),
                parent_w.upgrade(),
            ) else {
                return;
            };
            if !parent.is_visible() || !b.is_sensitive() {
                return;
            }
            for other in subs.borrow().iter() {
                if *other != sp && other.is_visible() {
                    other.popdown();
                }
            }
            if let Some(bounds) = b.compute_bounds(&vb) {
                let rect = gdk::Rectangle::new(
                    bounds.x().round() as i32,
                    bounds.y().round() as i32,
                    bounds.width().round().max(1.0) as i32,
                    bounds.height().round().max(1.0) as i32,
                );
                sp.set_pointing_to(Some(&rect));
            }
            if !sp.is_visible() {
                sp.popup();
            }
        })
    };

    {
        let open_click = open_submenu.clone();
        trigger_btn.connect_clicked(move |_| {
            open_click();
        });
    }

    let motion = gtk4::EventControllerMotion::new();
    {
        let open_hover = open_submenu;
        motion.connect_enter(move |_, _, _| {
            open_hover();
        });
    }
    trigger_btn.add_controller(motion);

    let sub_close = sub_pop.downgrade();
    parent_popover.connect_closed(move |_| {
        if let Some(sp) = sub_close.upgrade() {
            if sp.is_visible() {
                sp.popdown();
            }
        }
    });

    sub_pop
}

fn close_submenus_on_hover(row_btn: &Button, tracked_submenus: &Rc<RefCell<Vec<Popover>>>) {
    let subs = tracked_submenus.clone();
    let motion = gtk4::EventControllerMotion::new();
    motion.connect_enter(move |_, _, _| {
        for sub in subs.borrow().iter() {
            if sub.is_visible() {
                sub.popdown();
            }
        }
    });
    row_btn.add_controller(motion);
}

fn popdown_lifo(sub_popover: &Popover, parent_popover: &Popover) {
    if sub_popover.is_visible() {
        sub_popover.popdown();
    }
    if parent_popover.is_visible() {
        parent_popover.popdown();
    }
}

fn resolve_sibling_binary(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join(name)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn spawn_branches(args: &[&str]) {
    let exe = resolve_sibling_binary("malus-de-branches");
    let wayland_display =
        std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".to_string());
    let mut command = Command::new(&exe);
    malus_de_common::config::set_cursor_environment(&mut command);
    if let Err(err) = command
        .args(args)
        .env("WAYLAND_DISPLAY", &wayland_display)
        .env("GDK_BACKEND", "wayland")
        .env("GSK_RENDERER", "cairo")
        .spawn()
    {
        warn!("No se pudo lanzar malus-de-branches {:?}: {}", args, err);
    }
}

fn spawn_settings_panel(panel: &str) {
    let exe = resolve_sibling_binary("malus-de-settings");
    let wayland_display =
        std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".to_string());
    let mut command = Command::new(&exe);
    malus_de_common::config::set_cursor_environment(&mut command);
    if let Err(err) = command
        .args(["--panel", panel])
        .env("WAYLAND_DISPLAY", &wayland_display)
        .env("GDK_BACKEND", "wayland")
        .env("GSK_RENDERER", "cairo")
        .spawn()
    {
        warn!("No se pudo lanzar malus-de-settings (--panel {}): {}", panel, err);
    }
}

fn resolve_user_dir(home: &Path, xdg_key: &str, es_name: &str, en_name: &str) -> PathBuf {
    let xdg_cfg = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".config"));
    for candidate in [
        xdg_cfg.join("user-dirs.dirs"),
        home.join(".config/user-dirs.dirs"),
    ] {
        if let Ok(content) = std::fs::read_to_string(&candidate) {
            for line in content.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix(&format!("{xdg_key}=")) {
                    let unquoted = rest.trim().trim_matches('"');
                    let resolved = if let Some(rel) = unquoted.strip_prefix("$HOME/") {
                        home.join(rel)
                    } else if unquoted == "$HOME" {
                        home.to_path_buf()
                    } else {
                        PathBuf::from(unquoted)
                    };
                    if resolved.exists() {
                        return resolved;
                    }
                }
            }
        }
    }
    let es_path = home.join(es_name);
    if es_path.exists() {
        return es_path;
    }
    let en_path = home.join(en_name);
    if en_path.exists() {
        return en_path;
    }
    es_path
}

fn trash_base_dir() -> PathBuf {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".local/share")
        });
    data_home.join("Trash")
}

fn show_about_branches_dialog() {
    let es = is_spanish_locale();
    let comments = if es {
        "Explorador de archivos nativo de Malus DE, diseñado con estética Liquid Glass y flujo de trabajo estilo macOS."
    } else {
        "Native file manager for Malus DE, crafted with Liquid Glass aesthetics and macOS-style workflow."
    };
    let dev = if es {
        "Equipo Malus DE"
    } else {
        "Malus DE Team"
    };

    let about = adw::AboutDialog::builder()
        .application_name("Branches")
        .application_icon("system-file-manager")
        .version("0.1.0-alpha")
        .developer_name(dev)
        .comments(comments)
        .website("https://github.com/cristian/malus-de")
        .issue_url("https://github.com/cristian/malus-de/issues")
        .license_type(gtk4::License::Gpl30)
        .build();

    about.present(None as Option<&gtk4::Window>);
}

thread_local! {
    static BRANCHES_SETTINGS_WINDOW: RefCell<Option<glib::WeakRef<Window>>> =
        const { RefCell::new(None) };
}

fn show_branches_settings_dialog() {
    let existing = BRANCHES_SETTINGS_WINDOW.with(|cell| {
        cell.borrow().as_ref().and_then(|w| w.upgrade())
    });
    if let Some(win) = existing {
        win.present();
        return;
    }

    let es = is_spanish_locale();
    let cfg = malus_de_common::config::BranchesConfig::load();

    let win = Window::builder()
        .title(if es {
            "Configuración de Branches"
        } else {
            "Branches Settings"
        })
        .default_width(360)
        .resizable(false)
        .build();

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();

    let add_slider = |parent: &GtkBox,
                      title: &str,
                      min: f64,
                      max: f64,
                      step: f64,
                      value: f64,
                      on_change: Box<dyn Fn(u32)>| {
        let section = GtkBox::new(Orientation::Vertical, 4);
        let header = GtkBox::new(Orientation::Horizontal, 8);
        header.append(
            &Label::builder()
                .label(title)
                .halign(Align::Start)
                .hexpand(true)
                .build(),
        );
        let val_lbl = Label::builder()
            .label(format!("{} px", value.round() as u32))
            .halign(Align::End)
            .css_classes(["dim-label"])
            .build();
        header.append(&val_lbl);

        let scale = Scale::with_range(Orientation::Horizontal, min, max, step);
        scale.set_value(value);
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        let val_c = val_lbl.clone();
        scale.connect_value_changed(move |s| {
            let v = s.value().round() as u32;
            val_c.set_label(&format!("{v} px"));
            on_change(v);
        });
        section.append(&header);
        section.append(&scale);
        parent.append(&section);
    };

    add_slider(
        &vbox,
        if es { "Tamaño de los íconos" } else { "Icon Size" },
        malus_de_common::config::BranchesConfig::MIN_ICON_SIZE as f64,
        malus_de_common::config::BranchesConfig::MAX_ICON_SIZE as f64,
        4.0,
        cfg.icon_size as f64,
        Box::new(|v| {
            let mut c = malus_de_common::config::BranchesConfig::load();
            c.icon_size = v;
            let _ = c.save();
        }),
    );

    add_slider(
        &vbox,
        if es {
            "Espacio en la cuadrícula"
        } else {
            "Grid Spacing"
        },
        malus_de_common::config::BranchesConfig::MIN_GRID_SPACING as f64,
        malus_de_common::config::BranchesConfig::MAX_GRID_SPACING as f64,
        2.0,
        cfg.grid_spacing as f64,
        Box::new(|v| {
            let mut c = malus_de_common::config::BranchesConfig::load();
            c.grid_spacing = v;
            let _ = c.save();
        }),
    );

    add_slider(
        &vbox,
        if es { "Tamaño de lista" } else { "List Icon Size" },
        malus_de_common::config::BranchesConfig::MIN_LIST_ICON_SIZE as f64,
        malus_de_common::config::BranchesConfig::MAX_LIST_ICON_SIZE as f64,
        2.0,
        cfg.list_icon_size as f64,
        Box::new(|v| {
            let mut c = malus_de_common::config::BranchesConfig::load();
            c.list_icon_size = v;
            let _ = c.save();
        }),
    );

    let hidden_row = GtkBox::new(Orientation::Horizontal, 8);
    hidden_row.append(
        &Label::builder()
            .label(if es {
                "Mostrar archivos ocultos"
            } else {
                "Show Hidden Files"
            })
            .halign(Align::Start)
            .hexpand(true)
            .build(),
    );
    let sw_hidden = Switch::builder()
        .active(cfg.show_hidden)
        .valign(Align::Center)
        .build();
    sw_hidden.connect_state_set(|_, active| {
        let mut c = malus_de_common::config::BranchesConfig::load();
        c.show_hidden = active;
        let _ = c.save();
        glib::Propagation::Proceed
    });
    hidden_row.append(&sw_hidden);
    vbox.append(&hidden_row);

    win.set_child(Some(&vbox));
    win.connect_close_request(|_| {
        BRANCHES_SETTINGS_WINDOW.with(|cell| {
            *cell.borrow_mut() = None;
        });
        glib::Propagation::Proceed
    });
    BRANCHES_SETTINGS_WINDOW.with(|cell| {
        *cell.borrow_mut() = Some(win.downgrade());
    });
    win.present();
}

fn confirm_and_empty_trash() {
    let es = is_spanish_locale();
    let base = trash_base_dir();
    let files_dir = base.join("files");
    let count = std::fs::read_dir(&files_dir)
        .map(|iter| iter.count())
        .unwrap_or(0);

    if count == 0 {
        info!("🗑️ [Branches Menu] El basurero ya está vacío.");
        return;
    }

    let heading = if es {
        "¿Seguro que quieres eliminar permanentemente los elementos del basurero?"
    } else {
        "Are you sure you want to permanently erase the items in the Trash?"
    };
    let body = if es {
        format!("No podrás deshacer esta acción ({count} elemento(s) en el basurero).")
    } else {
        format!("You can't undo this action ({count} item(s) in the Trash).")
    };

    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(&body)
        .build();
    dialog.add_response("cancel", if es { "Cancelar" } else { "Cancel" });
    dialog.add_response(
        "empty",
        if es {
            "Vaciar basurero"
        } else {
            "Empty Trash"
        },
    );
    dialog.set_response_appearance("empty", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("empty"));
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |_, response| {
        if response == "empty" {
            info!("🗑️ [Branches Menu] Vaciando basurero...");
            let gio_ok = Command::new("gio")
                .args(["trash", "--empty"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !gio_ok {
                let trash = trash_base_dir();
                for sub in ["files", "info"] {
                    if let Ok(entries) = std::fs::read_dir(trash.join(sub)) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.is_dir() && !path.is_symlink() {
                                let _ = std::fs::remove_dir_all(&path);
                            } else {
                                let _ = std::fs::remove_file(&path);
                            }
                        }
                    }
                }
            }
        }
    });

    dialog.present(None as Option<&gtk4::Window>);
}

/// Rellena el popover del nombre de la app («Branches») según `Finder1.png`:
/// - Acerca de Branches
/// - \[glifo\] Configuración... (⌘,)
/// - \[glifo\] Vaciar basurero... (⇧⌘⌫)
/// - Servicios (submenú)
/// - Ocultar Branches (⌘H)
/// - Ocultar otros (⌥⌘H)
/// - Mostrar todo (inhabilitado)
pub fn populate_branches_app_menu(
    popover: &Popover,
    content_box: &GtkBox,
    tracked_submenus: &Rc<RefCell<Vec<Popover>>>,
) {
    let es = is_spanish_locale();
    let labels = branches_app_menu_labels(es);
    content_box.set_width_request(254);

    // 1. Acerca de Branches
    let about_btn = create_menu_item(None, labels[0], None, false, true);
    close_submenus_on_hover(&about_btn, tracked_submenus);
    {
        let pop = popover.clone();
        about_btn.connect_clicked(move |_| {
            pop.popdown();
            show_about_branches_dialog();
        });
    }
    content_box.append(&about_btn);

    content_box.append(&create_divider());

    // 2. [glifo] Configuración... (⌘,)
    let settings_btn = create_menu_item(
        Some("apple-settings-symbolic"),
        labels[1],
        Some("⌘,"),
        false,
        true,
    );
    close_submenus_on_hover(&settings_btn, tracked_submenus);
    {
        let pop = popover.clone();
        settings_btn.connect_clicked(move |_| {
            pop.popdown();
            show_branches_settings_dialog();
        });
    }
    content_box.append(&settings_btn);

    content_box.append(&create_divider());

    // 3. [glifo] Vaciar basurero... (⇧⌘⌫)
    let empty_trash_btn = create_menu_item(
        Some("user-trash-symbolic"),
        labels[2],
        Some("⇧⌘⌫"),
        false,
        true,
    );
    close_submenus_on_hover(&empty_trash_btn, tracked_submenus);
    {
        let pop = popover.clone();
        empty_trash_btn.connect_clicked(move |_| {
            pop.popdown();
            confirm_and_empty_trash();
        });
    }
    content_box.append(&empty_trash_btn);

    content_box.append(&create_divider());

    // 4. Submenú Servicios
    let services_btn = create_menu_item(None, labels[3], None, true, true);
    let services_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .width_request(230)
        .margin_top(2)
        .margin_bottom(2)
        .build();
    let no_services_btn = create_menu_item(
        None,
        if es {
            "No se pueden aplicar servicios"
        } else {
            "No Services Apply"
        },
        None,
        false,
        false,
    );
    services_box.append(&no_services_btn);
    services_box.append(&create_divider());
    let services_settings_btn = create_menu_item(
        None,
        if es {
            "Configuración de Servicios..."
        } else {
            "Services Settings..."
        },
        None,
        false,
        true,
    );
    services_box.append(&services_settings_btn);

    content_box.append(&services_btn);
    let services_sub = attach_cascading_submenu(
        popover,
        content_box,
        &services_btn,
        &services_box,
        tracked_submenus,
    );
    {
        let sub_c = services_sub.clone();
        let pop_c = popover.clone();
        services_settings_btn.connect_clicked(move |_| {
            popdown_lifo(&sub_c, &pop_c);
            spawn_settings_panel("keyboard");
        });
    }

    content_box.append(&create_divider());

    // 5. Ocultar Branches (⌘H)
    let hide_btn = create_menu_item(None, labels[4], Some("⌘H"), false, true);
    close_submenus_on_hover(&hide_btn, tracked_submenus);
    {
        let pop = popover.clone();
        hide_btn.connect_clicked(move |_| {
            pop.popdown();
            let _ = malus_de_common::ipc::send_ipc_message(
                &malus_de_common::ipc::IpcMessage::HideActiveWindow,
            );
        });
    }
    content_box.append(&hide_btn);

    // 6. Ocultar otros (⌥⌘H)
    let hide_others_btn = create_menu_item(None, labels[5], Some("⌥⌘H"), false, true);
    close_submenus_on_hover(&hide_others_btn, tracked_submenus);
    {
        let pop = popover.clone();
        hide_others_btn.connect_clicked(move |_| {
            pop.popdown();
            let _ = malus_de_common::ipc::send_ipc_message(
                &malus_de_common::ipc::IpcMessage::HideOtherWindows,
            );
        });
    }
    content_box.append(&hide_others_btn);

    // 7. Mostrar todo (inhabilitado)
    let show_all_btn = create_menu_item(None, labels[6], None, false, false);
    content_box.append(&show_all_btn);
}

fn create_category_menu_button(title: &str, width_request: i32) -> (MenuButton, Popover, GtkBox) {
    let label = Label::builder()
        .label(title)
        .css_classes(["global-menu-item"])
        .build();

    let menu_btn = MenuButton::builder()
        .css_classes(["topbar-btn"])
        .has_frame(false)
        .always_show_arrow(false)
        .child(&label)
        .build();

    let popover = Popover::builder()
        .css_classes(["malus-popover-menu"])
        .has_arrow(false)
        .position(gtk4::PositionType::Bottom)
        .halign(Align::Start)
        .build();
    popover.set_offset(0, 4);
    crate::ui::theme::register_popover(&popover);

    let menu_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .width_request(width_request)
        .margin_top(2)
        .margin_bottom(2)
        .build();

    popover.set_child(Some(&menu_box));
    menu_btn.set_popover(Some(&popover));
    (menu_btn, popover, menu_box)
}

fn create_tag_dots_row() -> GtkBox {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_start(10)
        .margin_end(10)
        .margin_top(4)
        .margin_bottom(4)
        .valign(Align::Center)
        .build();

    for (r, g, b) in TAG_DOT_COLORS {
        let dot = gtk4::DrawingArea::builder()
            .content_width(14)
            .content_height(14)
            .valign(Align::Center)
            .build();
        dot.set_draw_func(move |_, cr, w, h| {
            let cx = w as f64 / 2.0;
            let cy = h as f64 / 2.0;
            let radius = (w.min(h) as f64 / 2.0) - 1.0;
            cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
            cr.set_source_rgba(r, g, b, 0.55);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(r * 0.78, g * 0.78, b * 0.78, 0.65);
            cr.set_line_width(0.8);
            let _ = cr.stroke();
        });
        row.append(&dot);
    }

    row
}

/// Construye las 6 categorías de menú de Branches en la barra superior (`Finder2.png`..`Finder7.png`):
/// Archivo, Edición, Visualización, Ir, Ventana y Ayuda.
pub fn render_branches_category_menus(menus_container: &GtkBox) {
    let es = is_spanish_locale();
    let titles = branches_category_titles(es);

    menus_container.append(&build_file_menu(titles[0], es));
    menus_container.append(&build_edit_menu(titles[1], es));
    menus_container.append(&build_view_menu(titles[2], es));
    menus_container.append(&build_go_menu(titles[3], es));
    menus_container.append(&build_window_menu(titles[4], es));
    menus_container.append(&build_help_menu(titles[5], es));
}

// =========================================================================
// 1. Archivo (Finder2.png)
// =========================================================================
fn build_file_menu(title: &str, es: bool) -> MenuButton {
    let (menu_btn, popover, menu_box) = create_category_menu_button(title, 286);

    let new_win = create_menu_item(
        None,
        if es {
            "Nueva ventana de Branches"
        } else {
            "New Branches Window"
        },
        Some("⌘N"),
        false,
        true,
    );
    {
        let pop = popover.clone();
        new_win.connect_clicked(move |_| {
            pop.popdown();
            spawn_branches(&["--new-window"]);
        });
    }
    menu_box.append(&new_win);

    menu_box.append(&create_menu_item(
        None,
        if es { "Nueva carpeta" } else { "New Folder" },
        Some("⇧⌘N"),
        false,
        true,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es {
            "Nueva carpeta con la selección"
        } else {
            "New Folder with Selection"
        },
        Some("⌃⌘N"),
        false,
        false,
    ));

    let smart_folder = create_menu_item(
        None,
        if es {
            "Nueva carpeta inteligente"
        } else {
            "New Smart Folder"
        },
        None,
        false,
        true,
    );
    {
        let pop = popover.clone();
        smart_folder.connect_clicked(move |_| {
            pop.popdown();
            spawn_branches(&["--new-window"]);
        });
    }
    menu_box.append(&smart_folder);

    menu_box.append(&create_menu_item(
        None,
        if es { "Nueva pestaña" } else { "New Tab" },
        Some("⌘T"),
        false,
        true,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es { "Abrir" } else { "Open" },
        Some("⌘O"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es { "Abrir con" } else { "Open With" },
        None,
        true,
        false,
    ));

    let close_win = create_menu_item(
        None,
        if es {
            "Cerrar ventana"
        } else {
            "Close Window"
        },
        Some("⌘W"),
        false,
        true,
    );
    {
        let pop = popover.clone();
        close_win.connect_clicked(move |_| {
            pop.popdown();
            let _ = malus_de_common::ipc::send_ipc_message(
                &malus_de_common::ipc::IpcMessage::CloseActiveWindow,
            );
        });
    }
    menu_box.append(&close_win);

    menu_box.append(&create_divider());

    menu_box.append(&create_menu_item(
        None,
        if es {
            "Obtener información"
        } else {
            "Get Info"
        },
        Some("⌘I"),
        false,
        true,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es { "Renombrar" } else { "Rename" },
        None,
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es { "Comprimir" } else { "Compress" },
        None,
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es { "Duplicar" } else { "Duplicate" },
        Some("⌘D"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es { "Crear alias" } else { "Make Alias" },
        Some("⌃⌘A"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es { "Vista rápida" } else { "Quick Look" },
        Some("⌘Y"),
        false,
        false,
    ));

    menu_box.append(&create_divider());

    menu_box.append(&create_menu_item(
        Some("document-send-symbolic"),
        if es { "Compartir..." } else { "Share..." },
        None,
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        Some("system-users-symbolic"),
        if es {
            "Administrar archivo compartido..."
        } else {
            "Manage Shared File..."
        },
        None,
        false,
        false,
    ));

    menu_box.append(&create_divider());

    menu_box.append(&create_menu_item(
        None,
        if es {
            "Mostrar original"
        } else {
            "Show Original"
        },
        Some("⌘R"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es {
            "Agregar al Dock"
        } else {
            "Add to Dock"
        },
        Some("⌃⇧⌘T"),
        false,
        false,
    ));

    menu_box.append(&create_divider());

    menu_box.append(&create_menu_item(
        Some("user-trash-symbolic"),
        if es {
            "Transferir al basurero"
        } else {
            "Move to Trash"
        },
        Some("⌘⌫"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        Some("media-eject-symbolic"),
        if es { "Expulsar" } else { "Eject" },
        Some("⌘E"),
        false,
        false,
    ));

    menu_box.append(&create_divider());

    menu_box.append(&create_tag_dots_row());
    menu_box.append(&create_menu_item(
        Some("tag-symbolic"),
        if es { "Etiquetas..." } else { "Tags..." },
        None,
        false,
        false,
    ));

    menu_box.append(&create_divider());

    menu_box.append(&create_menu_item(
        Some("document-print-symbolic"),
        if es { "Imprimir" } else { "Print" },
        Some("⌘P"),
        false,
        false,
    ));

    crate::ui::theme::apply_theme_to_popover(&popover, crate::ui::theme::is_dark_mode_enabled());
    menu_btn
}

// =========================================================================
// 2. Edición (Finder3.png)
// =========================================================================
fn build_edit_menu(title: &str, es: bool) -> MenuButton {
    let (menu_btn, popover, menu_box) = create_category_menu_button(title, 266);
    let submenus: Rc<RefCell<Vec<Popover>>> = Rc::new(RefCell::new(Vec::new()));

    let undo_btn = create_menu_item(
        None,
        if es { "Deshacer" } else { "Undo" },
        Some("⌘Z"),
        false,
        true,
    );
    close_submenus_on_hover(&undo_btn, &submenus);
    menu_box.append(&undo_btn);

    menu_box.append(&create_menu_item(
        None,
        if es { "Rehacer" } else { "Redo" },
        Some("⇧⌘Z"),
        false,
        false,
    ));

    menu_box.append(&create_divider());

    menu_box.append(&create_menu_item(
        None,
        if es { "Cortar" } else { "Cut" },
        Some("⌘X"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es { "Copiar" } else { "Copy" },
        Some("⌘C"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es { "Pegar" } else { "Paste" },
        Some("⌘V"),
        false,
        false,
    ));

    let select_all_btn = create_menu_item(
        None,
        if es {
            "Seleccionar todo"
        } else {
            "Select All"
        },
        Some("⌘A"),
        false,
        true,
    );
    close_submenus_on_hover(&select_all_btn, &submenus);
    menu_box.append(&select_all_btn);

    menu_box.append(&create_divider());

    let clip_btn = create_menu_item(
        None,
        if es {
            "Mostrar portapapeles"
        } else {
            "Show Clipboard"
        },
        None,
        false,
        true,
    );
    close_submenus_on_hover(&clip_btn, &submenus);
    {
        let pop = popover.clone();
        clip_btn.connect_clicked(move |_| {
            pop.popdown();
            let _ = malus_de_common::ipc::send_ipc_message(
                &malus_de_common::ipc::IpcMessage::ToggleClipboardHistory,
            );
        });
    }
    menu_box.append(&clip_btn);

    menu_box.append(&create_divider());

    let find_btn = create_menu_item(
        Some("system-search-symbolic"),
        if es { "Buscar" } else { "Find" },
        Some("⌘F"),
        false,
        true,
    );
    close_submenus_on_hover(&find_btn, &submenus);
    {
        let pop = popover.clone();
        find_btn.connect_clicked(move |_| {
            pop.popdown();
            let _ = malus_de_common::ipc::send_ipc_message(
                &malus_de_common::ipc::IpcMessage::ToggleSpotlight,
            );
        });
    }
    menu_box.append(&find_btn);

    menu_box.append(&create_divider());

    let writing_btn = create_menu_item(
        None,
        if es {
            "Herramientas de escritura"
        } else {
            "Writing Tools"
        },
        None,
        true,
        true,
    );
    let writing_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .width_request(230)
        .margin_top(2)
        .margin_bottom(2)
        .build();
    writing_box.append(&create_menu_item(
        None,
        if es {
            "Mostrar herramientas de escritura"
        } else {
            "Show Writing Tools"
        },
        None,
        false,
        false,
    ));
    menu_box.append(&writing_btn);
    attach_cascading_submenu(&popover, &menu_box, &writing_btn, &writing_box, &submenus);

    menu_box.append(&create_divider());

    let autofill_btn = create_menu_item(
        Some("input-keyboard-symbolic"),
        if es { "Autorrelleno" } else { "AutoFill" },
        None,
        true,
        true,
    );
    let autofill_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .width_request(190)
        .margin_top(2)
        .margin_bottom(2)
        .build();
    autofill_box.append(&create_menu_item(
        None,
        if es { "Contactos..." } else { "Contacts..." },
        None,
        false,
        false,
    ));
    autofill_box.append(&create_menu_item(
        None,
        if es { "Contraseñas..." } else { "Passwords..." },
        None,
        false,
        false,
    ));
    menu_box.append(&autofill_btn);
    attach_cascading_submenu(&popover, &menu_box, &autofill_btn, &autofill_box, &submenus);

    let dictation_btn = create_trailing_glyph_menu_item(
        Some("audio-input-microphone-symbolic"),
        if es {
            "Iniciar dictado..."
        } else {
            "Start Dictation..."
        },
        "🎙",
        true,
    );
    close_submenus_on_hover(&dictation_btn, &submenus);
    menu_box.append(&dictation_btn);

    let emoji_btn = create_trailing_glyph_menu_item(
        Some("face-smile-symbolic"),
        if es {
            "Emojis y símbolos"
        } else {
            "Emoji & Symbols"
        },
        "🌐",
        true,
    );
    close_submenus_on_hover(&emoji_btn, &submenus);
    {
        let pop = popover.clone();
        emoji_btn.connect_clicked(move |_| {
            pop.popdown();
            let _ = malus_de_common::ipc::send_ipc_message(
                &malus_de_common::ipc::IpcMessage::ToggleEmojiPicker,
            );
        });
    }
    menu_box.append(&emoji_btn);

    crate::ui::theme::apply_theme_to_popover(&popover, crate::ui::theme::is_dark_mode_enabled());
    menu_btn
}

// =========================================================================
// 3. Visualización (Finder4.png)
// =========================================================================
fn build_view_menu(title: &str, es: bool) -> MenuButton {
    let (menu_btn, popover, menu_box) = create_category_menu_button(title, 294);
    let submenus: Rc<RefCell<Vec<Popover>>> = Rc::new(RefCell::new(Vec::new()));

    for (checked, icon, label_es, label_en, shortcut) in [
        (false, "view-grid-symbolic", "Como íconos", "as Icons", "⌘1"),
        (true, "view-list-symbolic", "Como lista", "as List", "⌘2"),
        (
            false,
            "view-dual-symbolic",
            "Como columnas",
            "as Columns",
            "⌘3",
        ),
        (
            false,
            "view-paged-symbolic",
            "Como galería",
            "as Gallery",
            "⌘4",
        ),
    ] {
        let btn = create_check_menu_item(
            checked,
            Some(icon),
            if es { label_es } else { label_en },
            Some(shortcut),
            true,
        );
        close_submenus_on_hover(&btn, &submenus);
        menu_box.append(&btn);
    }

    menu_box.append(&create_divider());

    let stack_btn = create_menu_item(
        None,
        if es { "Apilar" } else { "Use Stacks" },
        Some("⌃⌘0"),
        false,
        true,
    );
    close_submenus_on_hover(&stack_btn, &submenus);
    menu_box.append(&stack_btn);

    let sort_btn = create_menu_item(
        None,
        if es { "Ordenar por" } else { "Sort By" },
        None,
        true,
        true,
    );
    let sort_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .width_request(210)
        .margin_top(2)
        .margin_bottom(2)
        .build();
    for (es_lbl, en_lbl) in [
        ("Ninguno", "None"),
        ("Nombre", "Name"),
        ("Tipo", "Kind"),
        ("Fecha de apertura", "Date Last Opened"),
        ("Fecha de creación", "Date Created"),
        ("Fecha de modificación", "Date Modified"),
        ("Tamaño", "Size"),
        ("Etiquetas", "Tags"),
    ] {
        sort_box.append(&create_menu_item(
            None,
            if es { es_lbl } else { en_lbl },
            None,
            false,
            true,
        ));
    }
    menu_box.append(&sort_btn);
    attach_cascading_submenu(&popover, &menu_box, &sort_btn, &sort_box, &submenus);

    let clean_up_btn = create_menu_item(
        None,
        if es { "Alinear" } else { "Clean Up" },
        None,
        false,
        true,
    );
    close_submenus_on_hover(&clean_up_btn, &submenus);
    menu_box.append(&clean_up_btn);

    let clean_up_by_btn = create_menu_item(
        None,
        if es { "Alinear por" } else { "Clean Up By" },
        None,
        true,
        true,
    );
    let clean_by_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .width_request(210)
        .margin_top(2)
        .margin_bottom(2)
        .build();
    for (es_lbl, en_lbl) in [
        ("Nombre", "Name"),
        ("Tipo", "Kind"),
        ("Fecha de modificación", "Date Modified"),
        ("Fecha de creación", "Date Created"),
        ("Tamaño", "Size"),
        ("Etiquetas", "Tags"),
    ] {
        clean_by_box.append(&create_menu_item(
            None,
            if es { es_lbl } else { en_lbl },
            None,
            false,
            true,
        ));
    }
    menu_box.append(&clean_up_by_btn);
    attach_cascading_submenu(
        &popover,
        &menu_box,
        &clean_up_by_btn,
        &clean_by_box,
        &submenus,
    );

    menu_box.append(&create_divider());

    menu_box.append(&create_menu_item(
        None,
        if es {
            "Mostrar barra de pestañas"
        } else {
            "Show Tab Bar"
        },
        Some("⇧⌘T"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es {
            "Mostrar todas las pestañas"
        } else {
            "Show All Tabs"
        },
        Some("⇧⌘¿"),
        false,
        false,
    ));

    menu_box.append(&create_divider());

    menu_box.append(&create_menu_item(
        None,
        if es {
            "Ocultar barra lateral"
        } else {
            "Hide Sidebar"
        },
        Some("⌃⌘S"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es {
            "Mostrar vista previa"
        } else {
            "Show Preview"
        },
        Some("⇧⌘P"),
        false,
        false,
    ));

    menu_box.append(&create_divider());

    menu_box.append(&create_menu_item(
        None,
        if es {
            "Ocultar barra de herramientas"
        } else {
            "Hide Toolbar"
        },
        Some("⌥⌘T"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es {
            "Mostrar barra de ruta"
        } else {
            "Show Path Bar"
        },
        Some("⌥⌘P"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es {
            "Mostrar barra de estado"
        } else {
            "Show Status Bar"
        },
        Some("⌘'"),
        false,
        false,
    ));

    menu_box.append(&create_divider());

    menu_box.append(&create_menu_item(
        None,
        if es {
            "Personalizar barra de herramientas..."
        } else {
            "Customize Toolbar..."
        },
        None,
        false,
        false,
    ));

    menu_box.append(&create_divider());

    let view_opts_btn = create_menu_item(
        None,
        if es {
            "Mostrar opciones de visualización"
        } else {
            "Show View Options"
        },
        Some("⌘J"),
        false,
        true,
    );
    close_submenus_on_hover(&view_opts_btn, &submenus);
    {
        let pop = popover.clone();
        view_opts_btn.connect_clicked(move |_| {
            pop.popdown();
            show_branches_settings_dialog();
        });
    }
    menu_box.append(&view_opts_btn);

    menu_box.append(&create_menu_item(
        None,
        if es {
            "Mostrar opciones de vista previa"
        } else {
            "Show Preview Options"
        },
        None,
        false,
        false,
    ));

    menu_box.append(&create_divider());

    menu_box.append(&create_menu_item(
        None,
        if es {
            "Ver en pantalla completa"
        } else {
            "Enter Full Screen"
        },
        Some("🌐F"),
        false,
        false,
    ));

    crate::ui::theme::apply_theme_to_popover(&popover, crate::ui::theme::is_dark_mode_enabled());
    menu_btn
}

// =========================================================================
// 4. Ir (Finder5.png)
// =========================================================================
fn build_go_menu(title: &str, es: bool) -> MenuButton {
    let (menu_btn, popover, menu_box) = create_category_menu_button(title, 286);
    let submenus: Rc<RefCell<Vec<Popover>>> = Rc::new(RefCell::new(Vec::new()));

    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));
    let docs_path = resolve_user_dir(&home, "XDG_DOCUMENTS_DIR", "Documentos", "Documents");
    let desktop_path = resolve_user_dir(&home, "XDG_DESKTOP_DIR", "Escritorio", "Desktop");
    let downloads_path = resolve_user_dir(&home, "XDG_DOWNLOAD_DIR", "Descargas", "Downloads");
    let public_path = resolve_user_dir(&home, "XDG_PUBLICSHARE_DIR", "Público", "Public");

    menu_box.append(&create_menu_item(
        None,
        if es { "Atrás" } else { "Back" },
        Some("⌘{"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es { "Adelante" } else { "Forward" },
        Some("⌘}"),
        false,
        false,
    ));

    let enclosing_btn = create_menu_item(
        None,
        if es {
            "Seleccionar disco de arranque"
        } else {
            "Enclosing Folder"
        },
        Some("⇧⌘▲"),
        false,
        true,
    );
    close_submenus_on_hover(&enclosing_btn, &submenus);
    {
        let pop = popover.clone();
        enclosing_btn.connect_clicked(move |_| {
            pop.popdown();
            spawn_branches(&["/"]);
        });
    }
    menu_box.append(&enclosing_btn);

    menu_box.append(&create_divider());

    let places: Vec<(&str, &str, &str, &str, String)> = vec![
        (
            "document-open-recent-symbolic",
            "Recientes",
            "Recents",
            "⇧⌘F",
            home.to_string_lossy().to_string(),
        ),
        (
            "folder-documents-symbolic",
            "Documentos",
            "Documents",
            "⇧⌘O",
            docs_path.to_string_lossy().to_string(),
        ),
        (
            "user-desktop-symbolic",
            "Escritorio",
            "Desktop",
            "⇧⌘D",
            desktop_path.to_string_lossy().to_string(),
        ),
        (
            "folder-download-symbolic",
            "Descargas",
            "Downloads",
            "⌥⌘L",
            downloads_path.to_string_lossy().to_string(),
        ),
        (
            "user-home-symbolic",
            "Inicio",
            "Home",
            "⇧⌘H",
            home.to_string_lossy().to_string(),
        ),
        (
            "computer-symbolic",
            "Computadora",
            "Computer",
            "⇧⌘C",
            "/".to_string(),
        ),
        (
            "network-wireless-symbolic",
            "AirDrop",
            "AirDrop",
            "⇧⌘R",
            home.to_string_lossy().to_string(),
        ),
        (
            "network-workgroup-symbolic",
            "Red",
            "Network",
            "⇧⌘K",
            "/mnt".to_string(),
        ),
        (
            "weather-overcast-symbolic",
            "Nube",
            "Cloud Drive",
            "⇧⌘I",
            home.to_string_lossy().to_string(),
        ),
        (
            "system-users-symbolic",
            "Compartido",
            "Shared",
            "⇧⌘S",
            public_path.to_string_lossy().to_string(),
        ),
        (
            "apple-appstore-symbolic",
            "Aplicaciones",
            "Applications",
            "⇧⌘A",
            "applications://".to_string(),
        ),
        (
            "applications-utilities-symbolic",
            "Utilidades",
            "Utilities",
            "⇧⌘U",
            "applications://".to_string(),
        ),
    ];

    for (icon, label_es, label_en, shortcut, target) in places {
        let btn = create_menu_item(
            Some(icon),
            if es { label_es } else { label_en },
            Some(shortcut),
            false,
            true,
        );
        close_submenus_on_hover(&btn, &submenus);
        let pop = popover.clone();
        btn.connect_clicked(move |_| {
            pop.popdown();
            spawn_branches(&[target.as_str()]);
        });
        menu_box.append(&btn);
    }

    menu_box.append(&create_divider());

    let recent_folders_btn = create_menu_item(
        None,
        if es {
            "Carpetas recientes"
        } else {
            "Recent Folders"
        },
        None,
        true,
        true,
    );
    let recent_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .width_request(210)
        .margin_top(2)
        .margin_bottom(2)
        .build();
    menu_box.append(&recent_folders_btn);
    let recent_sub = attach_cascading_submenu(
        &popover,
        &menu_box,
        &recent_folders_btn,
        &recent_box,
        &submenus,
    );

    for (icon, lbl, path) in [
        ("user-home-symbolic", if es { "Inicio" } else { "Home" }, home),
        (
            "folder-documents-symbolic",
            if es { "Documentos" } else { "Documents" },
            docs_path,
        ),
        (
            "folder-download-symbolic",
            if es { "Descargas" } else { "Downloads" },
            downloads_path,
        ),
        (
            "user-desktop-symbolic",
            if es { "Escritorio" } else { "Desktop" },
            desktop_path,
        ),
    ] {
        let item = create_menu_item(Some(icon), lbl, None, false, true);
        let sub_c = recent_sub.clone();
        let pop_c = popover.clone();
        let target = path.to_string_lossy().to_string();
        item.connect_clicked(move |_| {
            popdown_lifo(&sub_c, &pop_c);
            spawn_branches(&[target.as_str()]);
        });
        recent_box.append(&item);
    }
    recent_box.append(&create_divider());
    recent_box.append(&create_menu_item(
        None,
        if es { "Borrar menú" } else { "Clear Menu" },
        None,
        false,
        false,
    ));

    menu_box.append(&create_divider());

    let go_folder_btn = create_menu_item(
        None,
        if es {
            "Ir a la carpeta..."
        } else {
            "Go to Folder..."
        },
        Some("⇧⌘G"),
        false,
        true,
    );
    close_submenus_on_hover(&go_folder_btn, &submenus);
    menu_box.append(&go_folder_btn);

    let connect_srv_btn = create_menu_item(
        None,
        if es {
            "Conectarse al servidor..."
        } else {
            "Connect to Server..."
        },
        Some("⌘K"),
        false,
        true,
    );
    close_submenus_on_hover(&connect_srv_btn, &submenus);
    menu_box.append(&connect_srv_btn);

    crate::ui::theme::apply_theme_to_popover(&popover, crate::ui::theme::is_dark_mode_enabled());
    menu_btn
}

// =========================================================================
// 5. Ventana (Finder6.png)
// =========================================================================
fn build_window_menu(title: &str, es: bool) -> MenuButton {
    let (menu_btn, popover, menu_box) = create_category_menu_button(title, 286);
    let submenus: Rc<RefCell<Vec<Popover>>> = Rc::new(RefCell::new(Vec::new()));

    let min_btn = create_menu_item(
        None,
        if es { "Minimizar" } else { "Minimize" },
        Some("⌘M"),
        false,
        true,
    );
    close_submenus_on_hover(&min_btn, &submenus);
    {
        let pop = popover.clone();
        min_btn.connect_clicked(move |_| {
            pop.popdown();
            let _ = malus_de_common::ipc::send_ipc_message(
                &malus_de_common::ipc::IpcMessage::HideActiveWindow,
            );
        });
    }
    menu_box.append(&min_btn);

    menu_box.append(&create_menu_item(
        None,
        if es { "Maximizar" } else { "Zoom" },
        None,
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es { "Rellenar" } else { "Fill" },
        Some("⌃🌐F"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es { "Centrar" } else { "Center" },
        Some("⌃🌐C"),
        false,
        false,
    ));

    menu_box.append(&create_divider());

    let move_resize_btn = create_menu_item(
        Some("sidebar-show-right-symbolic"),
        if es {
            "Desplazar y redimensionar"
        } else {
            "Move & Resize"
        },
        None,
        true,
        true,
    );
    let move_resize_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .width_request(196)
        .margin_top(2)
        .margin_bottom(2)
        .build();
    menu_box.append(&move_resize_btn);
    let move_sub = attach_cascading_submenu(
        &popover,
        &menu_box,
        &move_resize_btn,
        &move_resize_box,
        &submenus,
    );
    for (es_lbl, en_lbl) in [
        ("Izquierda", "Left"),
        ("Derecha", "Right"),
        ("Arriba", "Top"),
        ("Abajo", "Bottom"),
    ] {
        let item = create_menu_item(None, if es { es_lbl } else { en_lbl }, None, false, true);
        let sub_c = move_sub.clone();
        let pop_c = popover.clone();
        item.connect_clicked(move |_| {
            popdown_lifo(&sub_c, &pop_c);
            let _ = malus_de_common::ipc::send_ipc_message(
                &malus_de_common::ipc::IpcMessage::ToggleSnapLayouts,
            );
        });
        move_resize_box.append(&item);
    }

    menu_box.append(&create_menu_item(
        Some("view-dual-symbolic"),
        if es {
            "Ventana en pantalla completa"
        } else {
            "Full Screen Tile"
        },
        None,
        true,
        false,
    ));

    menu_box.append(&create_divider());

    let bring_front_btn = create_menu_item(
        None,
        if es {
            "Traer todo al frente"
        } else {
            "Bring All to Front"
        },
        None,
        false,
        true,
    );
    close_submenus_on_hover(&bring_front_btn, &submenus);
    {
        let pop = popover.clone();
        bring_front_btn.connect_clicked(move |_| {
            pop.popdown();
            let _ = malus_de_common::ipc::send_ipc_message(
                &malus_de_common::ipc::IpcMessage::ActivateApp {
                    app_id: "dev.malus.Branches".to_string(),
                },
            );
        });
    }
    menu_box.append(&bring_front_btn);

    let cycle_btn = create_menu_item(
        None,
        if es {
            "Recorrer ventanas"
        } else {
            "Cycle Through Windows"
        },
        Some("⌘<"),
        false,
        true,
    );
    close_submenus_on_hover(&cycle_btn, &submenus);
    menu_box.append(&cycle_btn);

    menu_box.append(&create_menu_item(
        None,
        if es {
            "Quitar ventana del conjunto"
        } else {
            "Remove Window from Set"
        },
        None,
        false,
        false,
    ));

    menu_box.append(&create_divider());

    menu_box.append(&create_menu_item(
        None,
        if es {
            "Mostrar pestaña anterior"
        } else {
            "Show Previous Tab"
        },
        Some("⌃⇧⇥"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es {
            "Mostrar pestaña siguiente"
        } else {
            "Show Next Tab"
        },
        Some("⌃⇥"),
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es {
            "Mover pestaña a una ventana nueva"
        } else {
            "Move Tab to New Window"
        },
        None,
        false,
        false,
    ));
    menu_box.append(&create_menu_item(
        None,
        if es {
            "Unir todas las ventanas"
        } else {
            "Merge All Windows"
        },
        None,
        false,
        false,
    ));

    menu_box.append(&create_divider());

    menu_box.append(&create_menu_item(
        None,
        if es {
            "Mostrar ventana de progreso"
        } else {
            "Show Progress Window"
        },
        None,
        false,
        false,
    ));

    crate::ui::theme::apply_theme_to_popover(&popover, crate::ui::theme::is_dark_mode_enabled());
    menu_btn
}

// =========================================================================
// 6. Ayuda (Finder7.png)
// =========================================================================
fn build_help_menu(title: &str, es: bool) -> MenuButton {
    let (menu_btn, popover, menu_box) = create_category_menu_button(title, 286);

    let search_entry = SearchEntry::builder()
        .placeholder_text(if es { "Buscar" } else { "Search" })
        .css_classes(["help-menu-search"])
        .hexpand(true)
        .margin_top(2)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();
    {
        let pop = popover.clone();
        search_entry.connect_activate(move |entry| {
            entry.set_text("");
            pop.popdown();
            let _ = malus_de_common::ipc::send_ipc_message(
                &malus_de_common::ipc::IpcMessage::ToggleSpotlight,
            );
        });
    }
    menu_box.append(&search_entry);

    let manual_btn = create_menu_item(
        Some("help-contents-symbolic"),
        if es {
            "Manual de uso de Malus DE"
        } else {
            "Malus DE User Guide"
        },
        Some("⌘?"),
        false,
        true,
    );
    {
        let pop = popover.clone();
        manual_btn.connect_clicked(move |_| {
            pop.popdown();
            show_about_branches_dialog();
        });
    }
    menu_box.append(&manual_btn);

    let tips_btn = create_menu_item(
        Some("weather-clear-symbolic"),
        if es {
            "Consejos para Malus DE"
        } else {
            "Tips for Malus DE"
        },
        None,
        false,
        true,
    );
    {
        let pop = popover.clone();
        tips_btn.connect_clicked(move |_| {
            pop.popdown();
            spawn_settings_panel("about");
        });
    }
    menu_box.append(&tips_btn);

    crate::ui::theme::apply_theme_to_popover(&popover, crate::ui::theme::is_dark_mode_enabled());
    menu_btn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_branches_app_ids_without_false_positives() {
        assert!(is_branches_app(Some("dev.malus.Branches"), None));
        assert!(is_branches_app(Some("dev.malus.Branches.Info"), Some("Foto.png")));
        assert!(is_branches_app(Some("malus-de-branches"), None));
        assert!(is_branches_app(None, Some("Branches")));
        assert!(!is_branches_app(Some("firefox"), Some("git branches")));
        assert!(!is_branches_app(Some("org.gnome.Nautilus"), Some("Archivos")));
        assert!(!is_branches_app(None, None));
    }

    #[test]
    fn branches_app_menu_and_categories_match_finder_spec() {
        assert_eq!(
            branches_app_menu_labels(true),
            [
                "Acerca de Branches",
                "Configuración...",
                "Vaciar basurero...",
                "Servicios",
                "Ocultar Branches",
                "Ocultar otros",
                "Mostrar todo",
            ]
        );
        assert_eq!(
            branches_category_titles(true),
            ["Archivo", "Edición", "Visualización", "Ir", "Ventana", "Ayuda"]
        );
    }
}
