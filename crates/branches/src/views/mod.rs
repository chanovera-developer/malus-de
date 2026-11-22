pub mod column_view;
pub mod gallery_view;
pub mod icon_view;
pub mod list_view;

use std::path::PathBuf;

pub type NavigateCallback = std::rc::Rc<dyn Fn(PathBuf)>;
pub type SelectionCallback = std::rc::Rc<dyn Fn(usize)>;
pub type RenameCallback = std::rc::Rc<dyn Fn(PathBuf)>;

const TYPEAHEAD_TIMEOUT_MS: u64 = 850;

pub fn normalize_typeahead_str(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            other => other,
        })
        .collect()
}

pub struct TypeAheadState {
    buffer: String,
    last_key_at: Option<std::time::Instant>,
}

impl TypeAheadState {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            last_key_at: None,
        }
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.last_key_at = None;
    }

    pub fn is_active(&self) -> bool {
        !self.buffer.is_empty()
            && self.last_key_at.is_some_and(|t| {
                t.elapsed() <= std::time::Duration::from_millis(TYPEAHEAD_TIMEOUT_MS)
            })
    }

    pub fn step(
        &mut self,
        ch: char,
        candidates: &[String],
        current_name: Option<&str>,
    ) -> Option<usize> {
        if candidates.is_empty() {
            return None;
        }
        let now = std::time::Instant::now();
        if self.last_key_at.is_none_or(|t| {
            now.duration_since(t) > std::time::Duration::from_millis(TYPEAHEAD_TIMEOUT_MS)
        }) {
            self.buffer.clear();
        }
        self.last_key_at = Some(now);

        let norm_ch = normalize_typeahead_str(&ch.to_string());
        if norm_ch.is_empty() {
            return None;
        }
        self.buffer.push_str(&norm_ch);

        let normalized: Vec<String> = candidates
            .iter()
            .map(|c| normalize_typeahead_str(c))
            .collect();

        if let Some(idx) = normalized.iter().position(|n| n.starts_with(&self.buffer)) {
            return Some(idx);
        }

        let single_char = norm_ch.chars().next()?;
        if self.buffer.chars().count() >= 2 && self.buffer.chars().all(|c| c == single_char) {
            self.buffer = norm_ch.clone();
            let start = current_name
                .and_then(|cur| candidates.iter().position(|c| c == cur))
                .map(|i| (i + 1) % candidates.len())
                .unwrap_or(0);
            for offset in 0..candidates.len() {
                let idx = (start + offset) % candidates.len();
                if normalized[idx].starts_with(&norm_ch) {
                    return Some(idx);
                }
            }
            return None;
        }

        if self.buffer.chars().count() > 1 {
            if let Some(idx) = normalized.iter().position(|n| n.starts_with(&norm_ch)) {
                self.buffer = norm_ch;
                return Some(idx);
            }
        }

        None
    }
}

/// Devuelve `true` si el evento tiene un modificador de selección múltiple o rango (`Ctrl`, `Super`, `Meta` o `Shift`).
pub(super) fn is_multi_select_modifier(state: gtk4::gdk::ModifierType) -> bool {
    use gtk4::gdk::ModifierType;
    state.intersects(
        ModifierType::CONTROL_MASK
            | ModifierType::SUPER_MASK
            | ModifierType::META_MASK
            | ModifierType::SHIFT_MASK,
    )
}

/// Rango inclusivo normalizado entre dos índices en cualquier orden.
pub(super) fn inclusive_range(a: usize, b: usize) -> std::ops::RangeInclusive<usize> {
    if a <= b {
        a..=b
    } else {
        b..=a
    }
}

/// Carga el mapa completo de etiquetas por ruta en una única consulta SQLite.
pub(super) fn load_tags_map() -> std::collections::HashMap<PathBuf, Vec<(String, String)>> {
    crate::db::open_db()
        .map(|conn| crate::db::get_all_file_tags_map(&conn))
        .unwrap_or_default()
}

/// Construye el contenedor de puntos de color (máximo 3 visibles) si el archivo tiene etiquetas.
pub(super) fn create_item_tag_dots(
    path: &std::path::Path,
    tags_map: &std::collections::HashMap<PathBuf, Vec<(String, String)>>,
) -> Option<gtk4::Box> {
    use gtk4::prelude::*;

    let tags = tags_map.get(path)?;
    if tags.is_empty() {
        return None;
    }

    let box_dots = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
    box_dots.set_valign(gtk4::Align::Center);
    box_dots.set_halign(gtk4::Align::Center);
    box_dots.add_css_class("branches-item-tags");

    for (raw_id, _) in tags.iter().take(3) {
        let cid = crate::db::canonical_tag_id(raw_id);
        let dot = crate::sidebar::create_cairo_tag_dot(&cid, 9);
        box_dots.append(&dot);
    }

    Some(box_dots)
}

/// Extrae todos los `FileItem` seleccionados de un `gtk4::ListBox` respetando el orden visual.
pub(super) fn list_box_selected_items(
    list_box: &gtk4::ListBox,
    items: &[crate::fs_backend::FileItem],
) -> Vec<crate::fs_backend::FileItem> {
    use gtk4::prelude::*;

    let mut indices: Vec<usize> = list_box
        .selected_rows()
        .into_iter()
        .filter_map(|row| {
            unsafe { row.data::<usize>("list_item_idx").map(|p| *p.as_ref()) }
                .or_else(|| usize::try_from(row.index()).ok())
        })
        .collect();
    indices.sort_unstable();
    indices.dedup();
    indices
        .into_iter()
        .filter_map(|idx| items.get(idx).cloned())
        .collect()
}

/// Prepara el `ContentProvider` de arrastre para una fila de `ListBox`:
/// si la fila forma parte de una selección múltiple, arrastra todas las rutas seleccionadas.
pub(super) fn prepare_list_row_drag(
    row: &gtk4::ListBoxRow,
    item_path: &std::path::Path,
    items: &[crate::fs_backend::FileItem],
) -> gtk4::gdk::ContentProvider {
    use gtk4::prelude::*;

    if let Some(lb) = row
        .parent()
        .and_then(|p| p.downcast::<gtk4::ListBox>().ok())
    {
        if !row.is_selected() {
            lb.unselect_all();
            lb.select_row(Some(row));
        }
        let paths: Vec<PathBuf> = list_box_selected_items(&lb, items)
            .into_iter()
            .map(|it| it.path)
            .collect();
        if !paths.is_empty() {
            return build_files_drag_content_provider(&paths);
        }
    }
    build_file_drag_content_provider(item_path)
}

/// Aplica o retira `.branches-cut-item` (opacidad 0.5) sobre el widget del elemento según el portapapeles.
pub(super) fn sync_cut_css_class(
    widget: &impl gtk4::prelude::IsA<gtk4::Widget>,
    path: &std::path::Path,
) {
    use gtk4::prelude::*;
    if crate::operations::is_path_cut(path) {
        widget.add_css_class("branches-cut-item");
    } else {
        widget.remove_css_class("branches-cut-item");
    }
}

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
    crate::app_window::attach_squircle_menu_hover(&btn);
    btn
}

fn make_context_menu_icon_button(
    label: &str,
    left_icon: Option<&str>,
    right_arrow: bool,
) -> gtk4::Button {
    use gtk4::prelude::*;

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    if let Some(icon_name) = left_icon {
        let icon = gtk4::Image::from_icon_name(icon_name);
        icon.set_pixel_size(14);
        hbox.append(&icon);
    }
    let lbl = gtk4::Label::builder()
        .label(label)
        .halign(gtk4::Align::Start)
        .xalign(0.0)
        .hexpand(true)
        .build();
    hbox.append(&lbl);
    if right_arrow {
        let arrow = gtk4::Image::from_icon_name("pan-end-symbolic");
        arrow.set_pixel_size(12);
        arrow.set_opacity(0.7);
        hbox.append(&arrow);
    }

    let btn = gtk4::Button::builder()
        .child(&hbox)
        .has_frame(false)
        .css_classes(["popover-menu-item"])
        .build();
    crate::app_window::attach_squircle_menu_hover(&btn);
    btn
}

fn make_context_action_button(
    label: &str,
    left_icon: Option<&str>,
    popover: &gtk4::Popover,
    anchor: &gtk4::Widget,
    action_name: &'static str,
    target_str: &str,
) -> gtk4::Button {
    use gtk4::prelude::*;

    let btn = if left_icon.is_some() {
        make_context_menu_icon_button(label, left_icon, false)
    } else {
        make_context_menu_button(label)
    };
    let pop = popover.downgrade();
    let aw = anchor.downgrade();
    let param = target_str.to_string();
    btn.connect_clicked(move |_| {
        if let Some(p) = pop.upgrade() {
            popdown_popover_lifo(&p);
        }
        if let Some(w) = aw.upgrade() {
            let _ = w.activate_action(action_name, Some(&param.to_variant()));
        }
    });
    btn
}

fn popdown_popover_lifo(popover: &gtk4::Popover) {
    use gtk4::prelude::*;

    if let Some(subs_ptr) =
        unsafe { popover.data::<std::rc::Rc<std::cell::RefCell<Vec<glib::WeakRef<gtk4::Popover>>>>>("cascading_submenus") }
    {
        let subs = unsafe { subs_ptr.as_ref() }.clone();
        for sub_w in subs.borrow().iter() {
            if let Some(sp) = sub_w.upgrade() {
                if sp.is_visible() {
                    sp.popdown();
                }
            }
        }
    }
    if popover.is_visible() {
        popover.popdown();
    }
}

fn attach_cascading_submenu(
    parent_popover: &gtk4::Popover,
    menu_vbox: &gtk4::Box,
    trigger_btn: &gtk4::Button,
    submenu_content: &gtk4::Box,
    all_submenus: &std::rc::Rc<std::cell::RefCell<Vec<glib::WeakRef<gtk4::Popover>>>>,
) {
    use gtk4::gdk;
    use gtk4::prelude::*;

    unsafe {
        parent_popover.set_data("cascading_submenus", all_submenus.clone());
    }

    submenu_content.set_vexpand(false);
    submenu_content.set_valign(gtk4::Align::Start);

    let sub_pop = gtk4::Popover::builder()
        .has_arrow(false)
        .autohide(false)
        .position(gtk4::PositionType::Right)
        .css_classes(["malus-popover-menu"])
        .build();
    sub_pop.set_vexpand(false);
    sub_pop.set_valign(gtk4::Align::Start);
    sub_pop.set_child(Some(submenu_content));
    sub_pop.set_parent(menu_vbox);

    all_submenus.borrow_mut().push(sub_pop.downgrade());

    let sub_w = sub_pop.downgrade();
    let btn_w = trigger_btn.downgrade();
    let vbox_w = menu_vbox.downgrade();
    let subs = all_submenus.clone();
    trigger_btn.connect_clicked(move |_| {
        let (Some(sp), Some(b), Some(vb)) = (sub_w.upgrade(), btn_w.upgrade(), vbox_w.upgrade())
        else {
            return;
        };
        for other_w in subs.borrow().iter() {
            if let Some(other) = other_w.upgrade() {
                if other != sp && other.is_visible() {
                    other.popdown();
                }
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
    });

    let sub_cleanup = sub_pop.downgrade();
    parent_popover.connect_closed(move |_| {
        if let Some(sp) = sub_cleanup.upgrade() {
            if sp.is_visible() {
                sp.popdown();
            }
            if sp.parent().is_some() {
                sp.unparent();
            }
        }
    });
}

fn is_desktop_picture_candidate(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" | "avif" | "tif" | "tiff"
            )
        })
}

fn set_desktop_picture(path: &std::path::Path) {
    let mut cfg = malus_de_common::config::WallpaperConfig::load();
    cfg.mode = malus_de_common::config::WallpaperMode::Image;
    cfg.image_path = Some(path.to_string_lossy().into_owned());
    let malus_dir = malus_de_common::config::malus_wallpaper_config_dir();
    let _ = std::fs::remove_file(malus_dir.join(".wallpaper-gradient"));
    let _ = cfg.save_in(&malus_dir);
    let _ = malus_de_common::ipc::IpcClient::send_command(
        malus_de_common::ipc::IpcMessage::ReloadWallpaperConfig,
    );
}

/// Construye el `ContentProvider` de arrastre exponiendo tanto `GdkFileList` como `text/uri-list`
/// para garantizar interoperabilidad Wayland con otras ventanas (como el Dock).
pub(super) fn build_file_drag_content_provider(
    path: &std::path::Path,
) -> gtk4::gdk::ContentProvider {
    build_files_drag_content_provider(&[path.to_path_buf()])
}

/// Construye un `ContentProvider` de arrastre para uno o varios archivos/carpetas seleccionados.
pub(super) fn build_files_drag_content_provider(paths: &[PathBuf]) -> gtk4::gdk::ContentProvider {
    use gtk4::prelude::*;

    let mut files = Vec::with_capacity(paths.len());
    let mut uri_payload = String::new();
    for path in paths {
        let file = gio::File::for_path(path);
        uri_payload.push_str(&file.uri());
        uri_payload.push_str("\r\n");
        files.push(file);
    }
    let file_list = gtk4::gdk::FileList::from_array(&files);
    gtk4::gdk::ContentProvider::new_union(&[
        gtk4::gdk::ContentProvider::for_value(&file_list.to_value()),
        gtk4::gdk::ContentProvider::for_bytes(
            "text/uri-list",
            &glib::Bytes::from_owned(uri_payload),
        ),
    ])
}

/// Construye un `Paintable` compuesto que incluye todos los widgets arrastrados en su disposición espacial relativa,
/// devolviendo `(paintable, hotspot_x, hotspot_y)` para que el cursor arrastre visualmente todo el grupo.
pub(super) fn build_multi_widget_drag_paintable(
    items: &[(gtk4::Widget, (f64, f64))],
    cursor_x: f64,
    cursor_y: f64,
) -> Option<(gtk4::gdk::Paintable, i32, i32)> {
    use gtk4::prelude::*;

    if items.is_empty() {
        return None;
    }

    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for (w, (ix, iy)) in items {
        let ww = w.width().max(w.width_request()).max(72) as f64;
        let wh = w.height().max(w.height_request()).max(76) as f64;
        min_x = min_x.min(*ix);
        min_y = min_y.min(*iy);
        max_x = max_x.max(*ix + ww);
        max_y = max_y.max(*iy + wh);
    }

    if !min_x.is_finite() || !min_y.is_finite() {
        return None;
    }

    let snapshot = gtk4::Snapshot::new();
    for (w, (ix, iy)) in items {
        let wp = gtk4::WidgetPaintable::new(Some(w));
        let ww = wp
            .intrinsic_width()
            .max(w.width())
            .max(w.width_request())
            .max(72) as f64;
        let wh = wp
            .intrinsic_height()
            .max(w.height())
            .max(w.height_request())
            .max(76) as f64;
        snapshot.save();
        snapshot.translate(&gtk4::graphene::Point::new(
            (*ix - min_x) as f32,
            (*iy - min_y) as f32,
        ));
        wp.snapshot(&snapshot, ww, wh);
        snapshot.restore();
    }

    let total_w = (max_x - min_x).ceil().max(1.0) as f32;
    let total_h = (max_y - min_y).ceil().max(1.0) as f32;
    let paintable = snapshot.to_paintable(Some(&gtk4::graphene::Size::new(total_w, total_h)))?;
    let hot_x = (cursor_x - min_x).round() as i32;
    let hot_y = (cursor_y - min_y).round() as i32;
    Some((paintable, hot_x, hot_y))
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

fn make_check_menu_button(label: &str, checked: bool, right_arrow: bool) -> gtk4::Button {
    use gtk4::prelude::*;

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let check_lbl = gtk4::Label::builder()
        .label(if checked { "✓" } else { "" })
        .width_request(14)
        .halign(gtk4::Align::Center)
        .xalign(0.5)
        .build();
    hbox.append(&check_lbl);

    let lbl = gtk4::Label::builder()
        .label(label)
        .halign(gtk4::Align::Start)
        .xalign(0.0)
        .hexpand(true)
        .build();
    hbox.append(&lbl);

    if right_arrow {
        let arrow = gtk4::Image::from_icon_name("pan-end-symbolic");
        arrow.set_pixel_size(12);
        arrow.set_opacity(0.7);
        hbox.append(&arrow);
    }

    let btn = gtk4::Button::builder()
        .child(&hbox)
        .has_frame(false)
        .css_classes(["popover-menu-item"])
        .build();
    crate::app_window::attach_squircle_menu_hover(&btn);
    btn
}

/// Menú contextual de espacio vacío en Branches (Visualización, Usar grupos, Ordenar por, etc.).
pub(super) fn show_background_context_menu(
    anchor: &impl gtk4::prelude::IsA<gtk4::Widget>,
    x: f64,
    y: f64,
) {
    use gtk4::gdk;
    use gtk4::prelude::*;

    if anchor.root().is_none() {
        return;
    }

    let Some(state) = crate::app_window::background_menu_state_for_widget(anchor) else {
        return;
    };
    let es = malus_de_common::is_spanish_locale();

    let popover = gtk4::Popover::builder()
        .has_arrow(false)
        .autohide(true)
        .css_classes(["malus-popover-menu"])
        .build();
    popover.set_parent(anchor);
    let rect = gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1);
    popover.set_pointing_to(Some(&rect));
    popover.connect_closed(|p| p.unparent());

    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    let all_submenus: std::rc::Rc<std::cell::RefCell<Vec<glib::WeakRef<gtk4::Popover>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let anchor_widget: gtk4::Widget = anchor.clone().upcast();

    // 1. Mostrar / Ocultar criterios de búsqueda
    let search_lbl = if state.search_criteria_visible {
        if es {
            "Ocultar criterios de búsqueda"
        } else {
            "Hide Search Criteria"
        }
    } else if es {
        "Mostrar criterios de búsqueda"
    } else {
        "Show Search Criteria"
    };
    let btn_search = make_context_menu_button(search_lbl);
    {
        let pop = popover.downgrade();
        let aw = anchor_widget.downgrade();
        btn_search.connect_clicked(move |_| {
            if let Some(p) = pop.upgrade() {
                popdown_popover_lifo(&p);
            }
            if let Some(w) = aw.upgrade() {
                let _ = w.activate_action("win.toggle_search_criteria", None);
            }
        });
    }
    main_box.append(&btn_search);

    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    sep.add_css_class("popover-menu-separator");
    main_box.append(&sep);

    // 2. Visualización >
    let view_trigger = make_context_menu_icon_button(
        if es { "Visualización" } else { "View" },
        None,
        true,
    );
    let view_sub = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    for (id, lbl_es, lbl_en) in [
        ("icons", "Como íconos", "as Icons"),
        ("list", "Como lista", "as List"),
        ("columns", "Como columnas", "as Columns"),
        ("gallery", "Como galería", "as Gallery"),
    ] {
        let lbl = if es { lbl_es } else { lbl_en };
        let item_btn = make_check_menu_button(lbl, state.view_mode == id, false);
        let pop = popover.downgrade();
        let aw = anchor_widget.downgrade();
        item_btn.connect_clicked(move |_| {
            if let Some(p) = pop.upgrade() {
                popdown_popover_lifo(&p);
            }
            if let Some(w) = aw.upgrade() {
                let _ = w.activate_action("win.set_view_mode", Some(&id.to_variant()));
            }
        });
        view_sub.append(&item_btn);
    }
    main_box.append(&view_trigger);
    attach_cascading_submenu(&popover, &main_box, &view_trigger, &view_sub, &all_submenus);

    // 3. Usar grupos
    let groups_base = if es { "Usar grupos" } else { "Use Groups" };
    let groups_lbl = if state.use_groups {
        format!("✓ {groups_base}")
    } else {
        groups_base.to_string()
    };
    let btn_groups = make_context_menu_button(&groups_lbl);
    {
        let pop = popover.downgrade();
        let aw = anchor_widget.downgrade();
        btn_groups.connect_clicked(move |_| {
            if let Some(p) = pop.upgrade() {
                popdown_popover_lifo(&p);
            }
            if let Some(w) = aw.upgrade() {
                let _ = w.activate_action("win.toggle_use_groups", None);
            }
        });
    }
    main_box.append(&btn_groups);

    // 4. Ordenar por >
    let sort_trigger = make_context_menu_icon_button(
        if es { "Ordenar por" } else { "Sort By" },
        None,
        true,
    );
    let sort_sub = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    for (sort_variant, lbl_es, lbl_en) in [
        (crate::state::SortBy::Name, "Nombre", "Name"),
        (crate::state::SortBy::Kind, "Clase", "Kind"),
        (
            crate::state::SortBy::DateLastOpened,
            "Fecha de última apertura",
            "Date Last Opened",
        ),
        (
            crate::state::SortBy::DateAdded,
            "Fecha en que se agregó",
            "Date Added",
        ),
        (
            crate::state::SortBy::DateModified,
            "Fecha de modificación",
            "Date Modified",
        ),
        (
            crate::state::SortBy::DateCreated,
            "Fecha de creación",
            "Date Created",
        ),
        (crate::state::SortBy::Size, "Tamaño", "Size"),
        (crate::state::SortBy::Tags, "Etiquetas", "Tags"),
    ] {
        let lbl = if es { lbl_es } else { lbl_en };
        let item_btn = make_check_menu_button(lbl, state.sort_by == sort_variant, false);
        let id = sort_variant.as_str();
        let pop = popover.downgrade();
        let aw = anchor_widget.downgrade();
        item_btn.connect_clicked(move |_| {
            if let Some(p) = pop.upgrade() {
                popdown_popover_lifo(&p);
            }
            if let Some(w) = aw.upgrade() {
                let _ = w.activate_action("win.sort_by", Some(&id.to_variant()));
            }
        });
        sort_sub.append(&item_btn);
    }
    main_box.append(&sort_trigger);
    attach_cascading_submenu(&popover, &main_box, &sort_trigger, &sort_sub, &all_submenus);

    // 4b. Alinear (en Vista de íconos)
    if state.view_mode == "icons" {
        let btn_align = make_context_menu_button(if es { "Alinear" } else { "Clean Up" });
        let pop = popover.downgrade();
        let aw = anchor_widget.downgrade();
        btn_align.connect_clicked(move |_| {
            if let Some(p) = pop.upgrade() {
                popdown_popover_lifo(&p);
            }
            if let Some(w) = aw.upgrade() {
                let _ = w.activate_action("win.align_icons", None);
            }
        });
        main_box.append(&btn_align);
    }

    // 5. Mostrar opciones de visualización
    let btn_options = make_context_menu_button(if es {
        "Mostrar opciones de visualización"
    } else {
        "Show View Options"
    });
    {
        let pop = popover.downgrade();
        let aw = anchor_widget.downgrade();
        btn_options.connect_clicked(move |_| {
            if let Some(p) = pop.upgrade() {
                popdown_popover_lifo(&p);
            }
            if let Some(w) = aw.upgrade() {
                let _ = w.activate_action("win.show_view_options", None);
            }
        });
    }
    main_box.append(&btn_options);

    popover.set_child(Some(&main_box));
    popover.popup();
}

fn find_item_label_widget(root: &gtk4::Widget, expected_name: &str) -> Option<gtk4::Label> {
    use gtk4::prelude::*;

    const TARGET_CLASSES: &[&str] = &[
        "branches-icon-label",
        "branches-list-name-label",
        "branches-column-name-label",
        "branches-carousel-label",
    ];

    if let Some(lbl) = root.downcast_ref::<gtk4::Label>() {
        if TARGET_CLASSES.iter().any(|c| root.has_css_class(c)) {
            return Some(lbl.clone());
        }
    }

    let mut fallback: Option<gtk4::Label> = None;
    let mut stack = vec![root.clone()];
    while let Some(node) = stack.pop() {
        if let Some(lbl) = node.downcast_ref::<gtk4::Label>() {
            if TARGET_CLASSES.iter().any(|c| node.has_css_class(c)) {
                return Some(lbl.clone());
            }
            if fallback.is_none() && lbl.text().as_str() == expected_name {
                fallback = Some(lbl.clone());
            }
        }
        let mut child = node.first_child();
        while let Some(c) = child {
            stack.push(c.clone());
            child = c.next_sibling();
        }
    }

    fallback
}

/// Activa la edición inline del nombre del archivo o carpeta directamente sobre su etiqueta
/// (estilo macOS Finder: `Enter` o perder foco confirma el cambio; `Escape` cancela).
pub(crate) fn show_rename_popover(
    anchor: &impl gtk4::prelude::IsA<gtk4::Widget>,
    path: PathBuf,
    is_dir: bool,
    _pointing_to: Option<(f64, f64)>,
    on_renamed: Option<RenameCallback>,
) {
    use gtk4::gdk;
    use gtk4::prelude::*;
    use std::cell::Cell;
    use std::rc::Rc;

    if anchor.root().is_none() || is_inside_trash(&path) {
        return;
    }

    let initial_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if initial_name.is_empty() {
        return;
    }

    let anchor_widget = anchor.as_ref().clone();
    let Some(label) = find_item_label_widget(&anchor_widget, &initial_name) else {
        return;
    };
    if !label.is_visible() {
        return;
    }
    let Some(parent_box) = label.parent().and_then(|p| p.downcast::<gtk4::Box>().ok()) else {
        return;
    };

    let is_icon_label = label.has_css_class("branches-icon-label")
        || label.has_css_class("branches-carousel-label");
    let label_w = label.width().max(24);
    let label_h = label.height().max(16);

    let entry = gtk4::Text::builder()
        .text(&initial_name)
        .width_chars(1)
        .max_width_chars(1)
        .xalign(if is_icon_label { 0.5 } else { 0.0 })
        .halign(if is_icon_label {
            gtk4::Align::Center
        } else {
            gtk4::Align::Fill
        })
        .valign(gtk4::Align::Center)
        .hexpand(!is_icon_label)
        .css_classes(if is_icon_label {
            vec![
                "branches-inline-rename-entry",
                "branches-inline-rename-icon",
            ]
        } else {
            vec!["branches-inline-rename-entry", "branches-inline-rename-row"]
        })
        .build();
    entry.set_size_request(if is_icon_label { label_w } else { -1 }, label_h);

    label.set_visible(false);
    parent_box.insert_child_after(&entry, Some(&label));

    let finished = Rc::new(Cell::new(false));
    let finish_edit: Rc<dyn Fn(bool)> = {
        let finished = finished.clone();
        let entry_ref = entry.clone();
        let label_ref = label.clone();
        let parent_ref = parent_box.clone();
        let old_path = path.clone();
        let initial = initial_name.clone();
        let on_renamed = on_renamed.clone();
        Rc::new(move |commit: bool| {
            if finished.replace(true) {
                return;
            }
            let candidate = entry_ref.text().to_string();
            let trimmed = candidate.trim();

            if entry_ref.parent().is_some() {
                parent_ref.remove(&entry_ref);
            }
            label_ref.set_visible(true);
            if let Some(canvas) = label_ref.ancestor(gtk4::Fixed::static_type()) {
                canvas.grab_focus();
            } else if let Some(row) = label_ref.ancestor(gtk4::ListBoxRow::static_type()) {
                row.grab_focus();
            }

            if !commit || trimmed.is_empty() || trimmed == initial {
                return;
            }

            if let Ok(new_path) = crate::operations::rename_item(&old_path, trimmed) {
                if new_path != old_path {
                    let old_str = old_path.to_string_lossy().to_string();
                    let new_str = new_path.to_string_lossy().to_string();
                    let _ = label_ref.activate_action(
                        "win.record_undo_rename",
                        Some(&(old_str, new_str).to_variant()),
                    );
                    if let Some(new_name) = new_path.file_name() {
                        label_ref.set_label(&new_name.to_string_lossy());
                    }
                    crate::sidebar::refresh_all_sidebars();
                    if let Some(ref cb) = on_renamed {
                        cb(new_path);
                    }
                }
            }
        })
    };

    // Confirmar con Enter
    {
        let finish = finish_edit.clone();
        entry.connect_activate(move |_| {
            finish(true);
        });
    }

    // Cancelar con Escape o detener propagación de teclas hacia la lista/ventana
    {
        let key_ctrl = gtk4::EventControllerKey::new();
        let finish = finish_edit.clone();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                finish(false);
                return glib::Propagation::Stop;
            }
            if matches!(
                key,
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter
            ) {
                finish(true);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        entry.add_controller(key_ctrl);
    }

    // Confirmar al hacer clic fuera (perder foco)
    {
        let focus_ctrl = gtk4::EventControllerFocus::new();
        let finish = finish_edit;
        focus_ctrl.connect_leave(move |_| {
            let f = finish.clone();
            glib::idle_add_local_once(move || {
                f(true);
            });
        });
        entry.add_controller(focus_ctrl);
    }

    // Reclamar clics primarios/secundarios sobre el Entry para no disparar navegación ni drag
    {
        let click_guard = gtk4::GestureClick::new();
        click_guard.set_button(0);
        click_guard.connect_pressed(|g, _n, _x, _y| {
            g.set_state(gtk4::EventSequenceState::Claimed);
        });
        entry.add_controller(click_guard);
    }

    let entry_focus = entry;
    let sel_end = crate::operations::rename_selection_end(&initial_name, is_dir);
    glib::idle_add_local_once(move || {
        entry_focus.grab_focus();
        entry_focus.select_region(0, sel_end);
    });
}

/// Abre la edición inline sobre un elemento recién creado una vez que el menú se ha cerrado
/// y el widget ya cuenta con geometría asignada en la vista activa.
pub(crate) fn schedule_inline_rename<F>(find_target: F, on_renamed: Option<RenameCallback>)
where
    F: Fn() -> Option<(PathBuf, bool, gtk4::Widget)> + 'static,
{
    use gtk4::prelude::*;

    let mut attempts = 0u8;
    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        attempts = attempts.saturating_add(1);
        if let Some((path, is_dir, widget)) = find_target() {
            let expected_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let ready = !expected_name.is_empty()
                && widget.root().is_some()
                && find_item_label_widget(&widget, &expected_name)
                    .is_some_and(|lbl| lbl.is_visible() && lbl.width() > 0 && lbl.height() > 0);
            if ready || attempts >= 12 {
                show_rename_popover(&widget, path, is_dir, None, on_renamed.clone());
                return glib::ControlFlow::Break;
            }
        } else if attempts >= 12 {
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
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

    let anchor_widget = anchor.clone().upcast::<gtk4::Widget>();
    let target_str = path.to_string_lossy().to_string();
    let item_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| target_str.clone());

    if is_inside_trash(path) {
        vbox.append(&make_context_action_button(
            if es {
                "Restaurar a ubicación original"
            } else {
                "Put Back"
            },
            None,
            &popover,
            &anchor_widget,
            "win.restore_item",
            &target_str,
        ));
        vbox.append(&make_context_action_button(
            if es {
                "Obtener información"
            } else {
                "Get Info"
            },
            None,
            &popover,
            &anchor_widget,
            "win.get_info_item",
            &target_str,
        ));
        vbox.append(&make_context_action_button(
            if es {
                "Eliminar inmediatamente…"
            } else {
                "Delete Immediately…"
            },
            None,
            &popover,
            &anchor_widget,
            "win.delete_permanent_item",
            &target_str,
        ));
        popover.set_child(Some(&vbox));
        popover.popup();
        return;
    }

    // 1. Abrir
    let open_label = if desktop_app.is_some() {
        if es {
            "Abrir aplicación"
        } else {
            "Open Application"
        }
    } else if es {
        "Abrir"
    } else {
        "Open"
    };
    let open_btn = make_context_menu_button(open_label);
    {
        let p = path.to_path_buf();
        let nav_opt = on_navigate.cloned();
        let pop = popover.downgrade();
        open_btn.connect_clicked(move |_| {
            if let Some(pop_w) = pop.upgrade() {
                pop_w.popdown();
            }
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

    vbox.set_vexpand(false);
    vbox.set_valign(gtk4::Align::Start);
    let all_submenus: std::rc::Rc<std::cell::RefCell<Vec<glib::WeakRef<gtk4::Popover>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));

    if is_dir {
        vbox.append(&make_context_action_button(
            if es {
                "Abrir en una pestaña nueva"
            } else {
                "Open in New Tab"
            },
            None,
            &popover,
            &anchor_widget,
            "win.open_in_new_tab",
            &target_str,
        ));
    } else {
        // 2. Abrir con >
        let open_with_btn = make_context_menu_icon_button(
            if es { "Abrir con" } else { "Open With" },
            None,
            true,
        );
        let ow_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);

        let gfile = gio::File::for_path(path);
        let mime_type = gfile
            .query_info(
                "standard::content-type",
                gio::FileQueryInfoFlags::NONE,
                gio::Cancellable::NONE,
            )
            .ok()
            .and_then(|info| info.content_type())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let apps = gio::AppInfo::all_for_type(&mime_type);
        if apps.is_empty() {
            let none_lbl = gtk4::Label::builder()
                .label(if es {
                    "Sin aplicaciones compatibles"
                } else {
                    "No Compatible Applications"
                })
                .margin_start(8)
                .margin_end(8)
                .margin_top(4)
                .margin_bottom(4)
                .build();
            ow_box.append(&none_lbl);
        } else {
            for app in apps.into_iter().take(12) {
                let app_btn = make_context_menu_button(&app.display_name());
                let pop_main = popover.downgrade();
                let gf = gfile.clone();
                app_btn.connect_clicked(move |_| {
                    if let Some(m) = pop_main.upgrade() {
                        popdown_popover_lifo(&m);
                    }
                    let _ = app.launch(&[gf.clone()], gio::AppLaunchContext::NONE);
                });
                ow_box.append(&app_btn);
            }
        }
        vbox.append(&open_with_btn);
        attach_cascading_submenu(&popover, &vbox, &open_with_btn, &ow_box, &all_submenus);
    }

    // 3. Transferir al basurero
    if path != std::path::Path::new("/") && path.parent().is_some() {
        vbox.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
        vbox.append(&make_context_action_button(
            if es {
                "Transferir al basurero"
            } else {
                "Move to Trash"
            },
            Some("user-trash-symbolic"),
            &popover,
            &anchor_widget,
            "win.trash_item",
            &target_str,
        ));
    }

    vbox.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

    // 4. Obtener información
    vbox.append(&make_context_action_button(
        if es {
            "Obtener información"
        } else {
            "Get Info"
        },
        None,
        &popover,
        &anchor_widget,
        "win.get_info_item",
        &target_str,
    ));

    // 5. Renombrar
    if path.parent().is_some() {
        let rename_btn = make_context_menu_button(if es { "Renombrar" } else { "Rename" });
        let aw = anchor_widget.clone();
        let p_rename = path.to_path_buf();
        let pop = popover.downgrade();
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
            if let Some(p) = pop.upgrade() {
                p.popdown();
            }
            let aw_inner = aw.clone();
            let pr = p_rename.clone();
            let cb = rename_cb.clone();
            glib::idle_add_local_once(move || {
                show_rename_popover(&aw_inner, pr, is_dir, pointing_to, cb);
            });
        });
        vbox.append(&rename_btn);
    }

    if path != std::path::Path::new("/") && path.parent().is_some() {
        // 6. Comprimir “<Nombre>”
        let compress_label = if es {
            format!("Comprimir “{}”", item_name)
        } else {
            format!("Compress “{}”", item_name)
        };
        vbox.append(&make_context_action_button(
            &compress_label,
            None,
            &popover,
            &anchor_widget,
            "win.compress_item",
            &target_str,
        ));

        if crate::jobs::is_supported_archive(path) {
            vbox.append(&make_context_action_button(
                if es {
                    "Extraer aquí"
                } else {
                    "Extract Here"
                },
                None,
                &popover,
                &anchor_widget,
                "win.extract_item",
                &target_str,
            ));
        }

        // 7. Duplicar
        vbox.append(&make_context_action_button(
            if es { "Duplicar" } else { "Duplicate" },
            None,
            &popover,
            &anchor_widget,
            "win.duplicate_item",
            &target_str,
        ));

        // 8. Crear alias
        vbox.append(&make_context_action_button(
            if es {
                "Crear alias"
            } else {
                "Make Alias"
            },
            None,
            &popover,
            &anchor_widget,
            "win.make_alias_item",
            &target_str,
        ));
    }

    // 9. Vista rápida
    vbox.append(&make_context_action_button(
        if es {
            "Vista rápida"
        } else {
            "Quick Look"
        },
        None,
        &popover,
        &anchor_widget,
        "win.quick_look_item",
        &target_str,
    ));

    // 10. Copiar
    if path != std::path::Path::new("/") {
        vbox.append(&make_context_action_button(
            if es { "Copiar" } else { "Copy" },
            None,
            &popover,
            &anchor_widget,
            "win.copy_item",
            &target_str,
        ));
    }

    // 11. Compartir...
    vbox.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
    let share_btn = make_context_menu_icon_button(
        if es {
            "Compartir..."
        } else {
            "Share..."
        },
        Some("emblem-shared-symbolic"),
        false,
    );
    {
        let pop = popover.downgrade();
        let aw = anchor_widget.downgrade();
        share_btn.connect_clicked(move |_| {
            if let Some(p) = pop.upgrade() {
                p.popdown();
            }
            if let Some(w) = aw.upgrade() {
                let _ = w.activate_action("win.share_malus_link", None);
            }
        });
    }
    vbox.append(&share_btn);

    // 12. Puntos de color + Etiquetas...
    if path != std::path::Path::new("/") && path.parent().is_some() {
        vbox.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
        let tag_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        tag_row.add_css_class("branches-context-tag-row");
        tag_row.set_halign(gtk4::Align::Start);
        for def in crate::db::TAG_CATALOG.iter() {
            let dot_area = crate::sidebar::create_cairo_tag_dot(def.id, 16);
            let dot_btn = gtk4::Button::builder()
                .child(&dot_area)
                .has_frame(false)
                .valign(gtk4::Align::Center)
                .tooltip_text(if es { def.label_es } else { def.label_en })
                .css_classes(["flat", "branches-context-tag-dot-btn"])
                .build();
            let aw = anchor_widget.downgrade();
            let t_str = target_str.clone();
            let tag_id = def.id.to_string();
            let pop = popover.downgrade();
            dot_btn.connect_clicked(move |_| {
                if let Some(p) = pop.upgrade() {
                    p.popdown();
                }
                if let Some(w) = aw.upgrade() {
                    let _ = w.activate_action(
                        "win.toggle_tag_item",
                        Some(&(tag_id.clone(), t_str.clone()).to_variant()),
                    );
                }
            });
            tag_row.append(&dot_btn);
        }
        vbox.append(&tag_row);

        let tags_btn = make_context_menu_icon_button(
            if es {
                "Etiquetas..."
            } else {
                "Tags..."
            },
            Some("tag-symbolic"),
            false,
        );
        let tags_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);

        let sidebar_tags = crate::db::open_db()
            .map(|c| crate::db::get_all_sidebar_tags(&c, es))
            .unwrap_or_default();
        if sidebar_tags.is_empty() {
            for def in crate::db::TAG_CATALOG.iter() {
                let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                hbox.append(&crate::sidebar::create_cairo_tag_dot(def.id, 12));
                let lbl = gtk4::Label::builder()
                    .label(if es { def.label_es } else { def.label_en })
                    .halign(gtk4::Align::Start)
                    .xalign(0.0)
                    .hexpand(true)
                    .build();
                hbox.append(&lbl);
                let item_btn = gtk4::Button::builder()
                    .child(&hbox)
                    .has_frame(false)
                    .css_classes(["popover-menu-item"])
                    .build();
                crate::app_window::attach_squircle_menu_hover(&item_btn);
                let pop_main = popover.downgrade();
                let aw = anchor_widget.downgrade();
                let t_str = target_str.clone();
                let tid = def.id.to_string();
                item_btn.connect_clicked(move |_| {
                    if let Some(m) = pop_main.upgrade() {
                        popdown_popover_lifo(&m);
                    }
                    if let Some(w) = aw.upgrade() {
                        let _ = w.activate_action(
                            "win.toggle_tag_item",
                            Some(&(tid.clone(), t_str.clone()).to_variant()),
                        );
                    }
                });
                tags_box.append(&item_btn);
            }
        } else {
            for meta in sidebar_tags {
                let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                hbox.append(&crate::sidebar::create_cairo_tag_dot(&meta.color_id, 12));
                let lbl = gtk4::Label::builder()
                    .label(&meta.label)
                    .halign(gtk4::Align::Start)
                    .xalign(0.0)
                    .hexpand(true)
                    .build();
                hbox.append(&lbl);
                let item_btn = gtk4::Button::builder()
                    .child(&hbox)
                    .has_frame(false)
                    .css_classes(["popover-menu-item"])
                    .build();
                crate::app_window::attach_squircle_menu_hover(&item_btn);
                let pop_main = popover.downgrade();
                let aw = anchor_widget.downgrade();
                let t_str = target_str.clone();
                let tid = meta.tag_id;
                item_btn.connect_clicked(move |_| {
                    if let Some(m) = pop_main.upgrade() {
                        popdown_popover_lifo(&m);
                    }
                    if let Some(w) = aw.upgrade() {
                        let _ = w.activate_action(
                            "win.toggle_tag_item",
                            Some(&(tid.clone(), t_str.clone()).to_variant()),
                        );
                    }
                });
                tags_box.append(&item_btn);
            }
        }
        tags_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
        tags_box.append(&make_context_action_button(
            if es {
                "Quitar etiquetas"
            } else {
                "Clear Tags"
            },
            None,
            &popover,
            &anchor_widget,
            "win.clear_tags_item",
            &target_str,
        ));
        vbox.append(&tags_btn);
        attach_cascading_submenu(&popover, &vbox, &tags_btn, &tags_box, &all_submenus);
    }

    // 13. Menú de acciones rápidas >
    vbox.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
    let quick_actions_btn = make_context_menu_icon_button(
        if es {
            "Menú de acciones rápidas"
        } else {
            "Quick Actions"
        },
        Some("emblem-system-symbolic"),
        true,
    );
    let qa_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);

    if is_dir || path.extension().and_then(|e| e.to_str()) != Some("desktop") {
        let term_btn = make_context_menu_button(if es {
            "Abrir en Terminal"
        } else {
            "Open in Terminal"
        });
        let p_term = path.to_path_buf();
        let pop_main = popover.downgrade();
        term_btn.connect_clicked(move |_| {
            if let Some(m) = pop_main.upgrade() {
                popdown_popover_lifo(&m);
            }
            crate::fs_backend::open_in_terminal(&p_term);
        });
        qa_box.append(&term_btn);
    }

    if let (Some(parent), Some(nav)) = (path.parent(), on_navigate) {
        if !is_dir {
            let enclosing_btn = make_context_menu_button(if es {
                "Mostrar en la carpeta contenedora"
            } else {
                "Show in Enclosing Folder"
            });
            let parent_path = parent.to_path_buf();
            let nav_cb = nav.clone();
            let pop_main = popover.downgrade();
            enclosing_btn.connect_clicked(move |_| {
                if let Some(m) = pop_main.upgrade() {
                    m.popdown();
                }
                nav_cb(parent_path.clone());
            });
            qa_box.append(&enclosing_btn);
        }
    }

    if path != std::path::Path::new("/") {
        if path.parent().is_some() {
            qa_box.append(&make_context_action_button(
                if es { "Cortar" } else { "Cut" },
                None,
                &popover,
                &anchor_widget,
                "win.cut_item",
                &target_str,
            ));
        }
        if is_dir && can_create_in_folder(path) && crate::operations::has_clipboard_files() {
            qa_box.append(&make_context_action_button(
                if es {
                    "Pegar en carpeta"
                } else {
                    "Paste into Folder"
                },
                None,
                &popover,
                &anchor_widget,
                "win.paste_into",
                &target_str,
            ));
        }

        let copy_path_btn = make_context_menu_button(if es {
            "Copiar ruta"
        } else {
            "Copy Path"
        });
        let p_copy = path.to_path_buf();
        let pop_main = popover.downgrade();
        copy_path_btn.connect_clicked(move |_| {
            if let Some(m) = pop_main.upgrade() {
                popdown_popover_lifo(&m);
            }
            if let Some(display) = gdk::Display::default() {
                display.clipboard().set_text(&p_copy.to_string_lossy());
            }
        });
        qa_box.append(&copy_path_btn);

        let already_bookmarked = crate::db::open_db()
            .map(|conn| crate::db::is_bookmark(&conn, path))
            .unwrap_or(false);
        let bm_btn = make_context_menu_button(if already_bookmarked {
            if es {
                "Quitar de marcadores"
            } else {
                "Remove Bookmark"
            }
        } else if es {
            "Añadir a marcadores"
        } else {
            "Add to Bookmarks"
        });
        let p_bm = path.to_path_buf();
        let pop_main = popover.downgrade();
        bm_btn.connect_clicked(move |_| {
            if let Some(m) = pop_main.upgrade() {
                popdown_popover_lifo(&m);
            }
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
        qa_box.append(&bm_btn);
    }

    if let Some(app_cfg) = desktop_app {
        let dock_cfg = malus_de_common::config::DockConfig::load_or_create_default();
        let currently_pinned = is_dock_app_pinned(&dock_cfg, &app_cfg);
        let dock_btn = make_context_menu_button(if currently_pinned {
            if es {
                "Quitar del Dock"
            } else {
                "Remove from Dock"
            }
        } else if es {
            "Agregar al Dock"
        } else {
            "Add to Dock"
        });
        let pop_main = popover.downgrade();
        dock_btn.connect_clicked(move |_| {
            if let Some(m) = pop_main.upgrade() {
                popdown_popover_lifo(&m);
            }
            set_app_pinned_in_dock(app_cfg.clone(), !currently_pinned);
        });
        qa_box.append(&dock_btn);
    }

    if path != std::path::Path::new("/") && path.parent().is_some() {
        qa_box.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
        qa_box.append(&make_context_action_button(
            if es {
                "Eliminar inmediatamente…"
            } else {
                "Delete Immediately…"
            },
            None,
            &popover,
            &anchor_widget,
            "win.delete_permanent_item",
            &target_str,
        ));
    }

    vbox.append(&quick_actions_btn);
    attach_cascading_submenu(&popover, &vbox, &quick_actions_btn, &qa_box, &all_submenus);

    // 14. Establecer como imagen del escritorio (para imágenes)
    if !is_dir && is_desktop_picture_candidate(path) {
        let wp_btn = make_context_menu_button(if es {
            "Establecer como imagen del escritorio"
        } else {
            "Set Desktop Picture"
        });
        let p_wp = path.to_path_buf();
        let pop = popover.downgrade();
        wp_btn.connect_clicked(move |_| {
            if let Some(p) = pop.upgrade() {
                p.popdown();
            }
            set_desktop_picture(&p_wp);
        });
        vbox.append(&wp_btn);
    }

    popover.set_child(Some(&vbox));
    popover.popup();
}

const IMAGE_THUMB_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "svg", "bmp", "ico", "tif", "tiff", "avif", "heic", "heif",
];
const MAX_THUMBNAIL_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_THUMBNAIL_CACHE_ENTRIES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ThumbnailCacheKey {
    path: PathBuf,
    modified_secs: u64,
    size: u64,
    decode_px: i32,
}

struct DecodedThumbnail {
    width: i32,
    height: i32,
    rowstride: usize,
    has_alpha: bool,
    bytes: glib::Bytes,
}

thread_local! {
    static THUMBNAIL_CACHE: std::cell::RefCell<std::collections::HashMap<ThumbnailCacheKey, gtk4::gdk::Texture>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

static NEXT_THUMB_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(super) fn is_image_thumbnail_candidate(item: &crate::fs_backend::FileItem) -> bool {
    if item.is_dir || item.size > MAX_THUMBNAIL_FILE_BYTES {
        return false;
    }
    item.path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            IMAGE_THUMB_EXTENSIONS
                .iter()
                .any(|cand| ext.eq_ignore_ascii_case(cand))
        })
        .unwrap_or(false)
}

fn thumbnail_decode_target_px(pixel_size: i32) -> i32 {
    (pixel_size.max(16) * 2).clamp(32, 512)
}

const SQUIRCLE_EXPONENT: f32 = 4.2;

fn squircle_sdf(px: f32, py: f32, half_w: f32, half_h: f32, radius: f32) -> f32 {
    let r = radius.min(half_w.min(half_h)).max(1.0);
    let dx = px.abs() - half_w + r;
    let dy = py.abs() - half_h + r;
    if dx <= 0.0 || dy <= 0.0 {
        dx.max(dy) - r
    } else {
        (dx.powf(SQUIRCLE_EXPONENT) + dy.powf(SQUIRCLE_EXPONENT)).powf(1.0 / SQUIRCLE_EXPONENT) - r
    }
}

fn thumbnail_squircle_radius(width: i32, height: i32) -> f32 {
    let min_side = width.min(height).max(1) as f32;
    let max_r = (min_side * 0.45).max(0.5);
    let min_r = 3.0_f32.min(max_r);
    (min_side * 0.22).clamp(min_r, max_r)
}

fn apply_squircle_clip_rgba(rgba: &mut [u8], width: usize, height: usize, radius: f32) {
    if width < 4 || height < 4 {
        return;
    }
    let half_w = width as f32 * 0.5;
    let half_h = height as f32 * 0.5;
    let r = radius.min(half_w.min(half_h)).max(1.0);
    let corner_span = (r.ceil() as usize + 1).min(width).min(height);
    let y_top_end = corner_span.min(height);
    let y_bot_start = height.saturating_sub(corner_span).max(y_top_end);
    let x_left_end = corner_span.min(width);
    let x_right_start = width.saturating_sub(corner_span).max(x_left_end);

    for y in (0..y_top_end).chain(y_bot_start..height) {
        let ly = (y as f32 + 0.5) - half_h;
        let row_offset = y * width * 4;
        for x in (0..x_left_end).chain(x_right_start..width) {
            let lx = (x as f32 + 0.5) - half_w;
            let dist = squircle_sdf(lx, ly, half_w, half_h, r);
            if dist > -0.5 {
                let alpha_idx = row_offset + x * 4 + 3;
                if dist >= 0.5 {
                    rgba[alpha_idx] = 0;
                } else {
                    let coverage = (0.5 - dist).clamp(0.0, 1.0);
                    rgba[alpha_idx] = (rgba[alpha_idx] as f32 * coverage).round() as u8;
                }
            }
        }
    }
}

fn decode_thumbnail_bytes(path: &std::path::Path, target_px: i32) -> Option<DecodedThumbnail> {
    let pixbuf = gdk_pixbuf::Pixbuf::from_file_at_scale(path, target_px, target_px, true).ok()?;
    let width = pixbuf.width();
    let height = pixbuf.height();
    let rowstride = pixbuf.rowstride();
    if width <= 0 || height <= 0 || rowstride <= 0 {
        return None;
    }
    let src_alpha = pixbuf.has_alpha();
    let src_bytes = pixbuf.read_pixel_bytes();
    let src = src_bytes.as_ref();
    let w = width as usize;
    let h = height as usize;
    let stride = rowstride as usize;
    let channels = if src_alpha { 4 } else { 3 };
    let out_stride = w * 4;
    let mut rgba = vec![0u8; out_stride * h];

    for y in 0..h {
        let row_start = y * stride;
        let out_start = y * out_stride;
        if row_start + w * channels > src.len() {
            return None;
        }
        if src_alpha {
            rgba[out_start..out_start + out_stride]
                .copy_from_slice(&src[row_start..row_start + out_stride]);
        } else {
            for x in 0..w {
                let s = row_start + x * 3;
                let d = out_start + x * 4;
                rgba[d] = src[s];
                rgba[d + 1] = src[s + 1];
                rgba[d + 2] = src[s + 2];
                rgba[d + 3] = 255;
            }
        }
    }

    let radius = thumbnail_squircle_radius(width, height);
    apply_squircle_clip_rgba(&mut rgba, w, h, radius);

    Some(DecodedThumbnail {
        width,
        height,
        rowstride: out_stride,
        has_alpha: true,
        bytes: glib::Bytes::from_owned(rgba),
    })
}

fn apply_thumbnail_texture(
    img: &gtk4::Image,
    texture: &gtk4::gdk::Texture,
    pixel_size: i32,
    thumb_css_class: Option<&str>,
) {
    use gtk4::prelude::*;

    img.set_paintable(Some(texture));
    img.set_pixel_size(pixel_size);
    if let Some(css) = thumb_css_class {
        img.add_css_class(css);
    }
}

pub(super) fn update_item_image_thumbnail(
    img: &gtk4::Image,
    item: &crate::fs_backend::FileItem,
    pixel_size: i32,
    thumb_css_class: Option<&str>,
) {
    use gtk4::prelude::*;

    apply_folder_icon_tint(img, item);
    img.set_pixel_size(pixel_size);
    if !is_image_thumbnail_candidate(item) {
        if let Some(css) = thumb_css_class {
            img.remove_css_class(css);
        }
        img.set_widget_name("");
        apply_item_gicon_or_tinted_folder(img, item, pixel_size);
        return;
    }

    let decode_px = thumbnail_decode_target_px(pixel_size);
    let modified_secs = item
        .modified
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let cache_key = ThumbnailCacheKey {
        path: item.path.clone(),
        modified_secs,
        size: item.size,
        decode_px,
    };

    if let Some(cached) = THUMBNAIL_CACHE.with(|cache| cache.borrow().get(&cache_key).cloned()) {
        img.set_widget_name("");
        apply_thumbnail_texture(img, &cached, pixel_size, thumb_css_class);
        return;
    }

    let token = NEXT_THUMB_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let marker = format!("thumb:{token}");
    img.set_widget_name(&marker);

    let img_weak = img.downgrade();
    let path = item.path.clone();
    let thumb_css = thumb_css_class.map(str::to_string);
    glib::spawn_future_local(async move {
        let decoded = gio::spawn_blocking(move || decode_thumbnail_bytes(&path, decode_px)).await;
        let Ok(Some(data)) = decoded else {
            return;
        };

        let format = if data.has_alpha {
            gtk4::gdk::MemoryFormat::R8g8b8a8
        } else {
            gtk4::gdk::MemoryFormat::R8g8b8
        };
        let texture: gtk4::gdk::Texture = gtk4::gdk::MemoryTexture::new(
            data.width,
            data.height,
            format,
            &data.bytes,
            data.rowstride,
        )
        .upcast();

        THUMBNAIL_CACHE.with(|cache| {
            let mut map = cache.borrow_mut();
            if map.len() >= MAX_THUMBNAIL_CACHE_ENTRIES {
                map.clear();
            }
            map.insert(cache_key, texture.clone());
        });

        if let Some(target_img) = img_weak.upgrade() {
            if target_img.widget_name().as_str() == marker {
                apply_thumbnail_texture(&target_img, &texture, pixel_size, thumb_css.as_deref());
            }
        }
    });
}

pub(super) fn create_item_image(
    item: &crate::fs_backend::FileItem,
    pixel_size: i32,
    thumb_css_class: Option<&str>,
) -> gtk4::Image {
    let img = gtk4::Image::new();
    apply_folder_icon_tint(&img, item);
    apply_item_gicon_or_tinted_folder(&img, item, pixel_size);
    img.set_pixel_size(pixel_size);
    if is_image_thumbnail_candidate(item) {
        update_item_image_thumbnail(&img, item, pixel_size, thumb_css_class);
    }
    img
}

pub(crate) fn apply_item_gicon_or_tinted_folder(
    img: &gtk4::Image,
    item: &crate::fs_backend::FileItem,
    pixel_size: i32,
) {
    if item.is_dir
        && malus_de_common::folder_icon::apply_tinted_folder_icon(img, &item.gicon, pixel_size)
    {
        return;
    }
    img.set_from_gicon(&item.gicon);
}

pub(crate) fn apply_folder_icon_tint(img: &gtk4::Image, item: &crate::fs_backend::FileItem) {
    use gtk4::prelude::WidgetExt;
    if item.is_dir {
        img.add_css_class("branches-folder-icon");
    } else {
        img.remove_css_class("branches-folder-icon");
    }
}

pub(crate) fn create_folder_aware_icon(
    item: &crate::fs_backend::FileItem,
    pixel_size: i32,
) -> gtk4::Image {
    let img = gtk4::Image::new();
    apply_folder_icon_tint(&img, item);
    apply_item_gicon_or_tinted_folder(&img, item, pixel_size);
    img.set_pixel_size(pixel_size);
    img
}

#[cfg(test)]
mod tests {
    use super::{
        decode_thumbnail_bytes, is_dock_app_pinned, is_image_thumbnail_candidate,
        thumbnail_decode_target_px,
    };
    use crate::fs_backend::FileItem;
    use gio::prelude::Cast;
    use malus_de_common::config::{DockAppConfig, DockConfig, DockIconAppearance};
    use std::path::PathBuf;

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

    #[test]
    fn thumbnail_candidate_detects_images_and_ignores_directories_and_non_images() {
        let make_item = |name: &str, is_dir: bool, size: u64| FileItem {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/{name}")),
            is_dir,
            is_hidden: false,
            is_symlink: false,
            size,
            size_str: "1 KB".to_string(),
            modified: None,
            accessed: None,
            added: None,
            created: None,
            modified_str: "--".to_string(),
            kind_str: "Imagen".to_string(),
            gicon: gio::ThemedIcon::new("image-x-generic").upcast(),
        };

        assert!(is_image_thumbnail_candidate(&make_item(
            "foto.PNG", false, 1024
        )));
        assert!(is_image_thumbnail_candidate(&make_item(
            "fondo.webp",
            false,
            2048
        )));
        assert!(!is_image_thumbnail_candidate(&make_item(
            "carpeta.png",
            true,
            0
        )));
        assert!(!is_image_thumbnail_candidate(&make_item(
            "notas.txt",
            false,
            128
        )));
        assert_eq!(thumbnail_decode_target_px(64), 128);
        assert_eq!(thumbnail_decode_target_px(512), 512);
    }

    #[test]
    fn decode_thumbnail_bytes_decodes_valid_png_thumbnail() {
        let temp_dir = std::env::temp_dir().join("malus_branches_thumb_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let img_path = temp_dir.join("sample.png");

        let pixbuf =
            gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 240, 120).unwrap();
        pixbuf.fill(0xff44_88ff);
        pixbuf.savev(&img_path, "png", &[]).unwrap();

        let thumb = decode_thumbnail_bytes(&img_path, 128).expect("Debe decodificar miniatura PNG");
        assert_eq!(thumb.width, 128);
        assert_eq!(thumb.height, 64);
        assert!(thumb.has_alpha);
        assert!(!thumb.bytes.is_empty());

        let raw = thumb.bytes.as_ref();
        // Esquina extrema recortada a transparente por el squircle y centro completamente opaco
        assert_eq!(raw[3], 0);
        assert_eq!(raw[(32 * 128 + 64) * 4 + 3], 255);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn multi_selection_helpers_and_views_work_across_list_columns_and_gallery() {
        assert_eq!(
            super::inclusive_range(2, 5).collect::<Vec<_>>(),
            vec![2, 3, 4, 5]
        );
        assert_eq!(
            super::inclusive_range(5, 2).collect::<Vec<_>>(),
            vec![2, 3, 4, 5]
        );
        assert!(super::is_multi_select_modifier(
            gtk4::gdk::ModifierType::CONTROL_MASK
        ));
        assert!(super::is_multi_select_modifier(
            gtk4::gdk::ModifierType::SHIFT_MASK
        ));
        assert!(super::is_multi_select_modifier(
            gtk4::gdk::ModifierType::SUPER_MASK
        ));
        assert!(!super::is_multi_select_modifier(
            gtk4::gdk::ModifierType::empty()
        ));

        if gtk4::init().is_err() {
            return;
        }

        let make_item = |name: &str| FileItem {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/{name}")),
            is_dir: false,
            is_hidden: false,
            is_symlink: false,
            size: 128,
            size_str: "128 B".to_string(),
            modified: None,
            accessed: None,
            added: None,
            created: None,
            modified_str: "--".to_string(),
            kind_str: "Texto".to_string(),
            gicon: gio::ThemedIcon::new("text-x-generic").upcast(),
        };
        let items = vec![make_item("a.txt"), make_item("b.txt"), make_item("c.txt")];
        let nav: super::NavigateCallback = std::rc::Rc::new(|_| {});
        let last_sel_count = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let sel_count_cb = last_sel_count.clone();
        let sel_cb: super::SelectionCallback = std::rc::Rc::new(move |n| sel_count_cb.set(n));

        // 1. ListView: selección múltiple y select_all
        let list_view = super::list_view::ListView::new(nav.clone(), sel_cb.clone());
        list_view.set_items(&items);
        list_view.select_all();
        let list_sel = list_view.get_selected_items();
        assert_eq!(list_sel.len(), 3);
        assert_eq!(last_sel_count.get(), 3);

        // 2. ColumnView: selección múltiple en columna activa y select_all
        let col_view = super::column_view::ColumnView::new(nav.clone(), sel_cb.clone());
        col_view.set_root(
            std::path::Path::new("/tmp"),
            &items,
            false,
            crate::state::SortBy::Name,
            crate::state::SortOrder::Ascending,
        );
        col_view.select_all();
        let col_sel = col_view.get_selected_items();
        assert_eq!(col_sel.len(), 3);
        assert_eq!(last_sel_count.get(), 3);

        // 3. GalleryView: selección única, rango Shift, toggle Ctrl y select_all
        let gallery = super::gallery_view::GalleryView::new(nav, sel_cb);
        gallery.set_items(&items);
        assert_eq!(gallery.get_selected_items().len(), 1);
        gallery.select_range_to(2);
        assert_eq!(gallery.get_selected_items().len(), 3);
        gallery.toggle_select_index(1);
        let gal_after_toggle: Vec<String> = gallery
            .get_selected_items()
            .into_iter()
            .map(|i| i.name)
            .collect();
        assert_eq!(gal_after_toggle, vec!["a.txt", "c.txt"]);
        gallery.select_all();
        assert_eq!(gallery.get_selected_items().len(), 3);
    }

    #[test]
    fn thin_and_tiny_thumbnails_do_not_panic_in_squircle_radius() {
        // Reproducción del caso real: imagen muy delgada escalada a ancho/alto <= 5 px (min_side * 0.45 = 2.25 < 3.0)
        for &(w, h) in &[(512, 5), (5, 512), (1, 200), (0, 0), (-4, 12), (64, 64)] {
            let r = super::thumbnail_squircle_radius(w, h);
            assert!(r.is_finite() && r > 0.0);
        }
        let mut thin = vec![255u8; 512 * 3 * 4];
        super::apply_squircle_clip_rgba(
            &mut thin,
            512,
            3,
            super::thumbnail_squircle_radius(512, 3),
        );
        assert!(thin.iter().all(|&b| b == 255));
    }

    #[test]
    fn typeahead_matches_prefix_accents_and_cycles_single_letter() {
        let items: Vec<String> = vec![
            "Documentos",
            "Finder1",
            "Finder2",
            "Finder3",
            "Imágenes",
            "Notas rápidas.txt",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let mut ta = super::TypeAheadState::new();
        // Prefijo sin acentos coincide con "Imágenes"
        assert_eq!(ta.step('i', &items, None), Some(4));
        assert_eq!(ta.step('m', &items, Some("Imágenes")), Some(4));
        assert_eq!(ta.step('a', &items, Some("Imágenes")), Some(4));
        assert!(ta.is_active());

        // Prefijo con espacio ("notas r")
        ta.clear();
        assert_eq!(ta.step('n', &items, None), Some(5));
        assert_eq!(ta.step('o', &items, Some("Notas rápidas.txt")), Some(5));
        assert_eq!(ta.step('t', &items, Some("Notas rápidas.txt")), Some(5));
        assert_eq!(ta.step('a', &items, Some("Notas rápidas.txt")), Some(5));
        assert_eq!(ta.step('s', &items, Some("Notas rápidas.txt")), Some(5));
        assert_eq!(ta.step(' ', &items, Some("Notas rápidas.txt")), Some(5));
        assert_eq!(ta.step('r', &items, Some("Notas rápidas.txt")), Some(5));

        // Repetir la misma letra cicla entre coincidencias
        ta.clear();
        assert_eq!(ta.step('f', &items, None), Some(1));
        assert_eq!(ta.step('f', &items, Some("Finder1")), Some(2));
        assert_eq!(ta.step('f', &items, Some("Finder2")), Some(3));
        assert_eq!(ta.step('f', &items, Some("Finder3")), Some(1));

        // Si un prefijo combinado no coincide, reinicia con la nueva letra
        ta.clear();
        assert_eq!(ta.step('f', &items, None), Some(1));
        assert_eq!(ta.step('d', &items, Some("Finder1")), Some(0));
    }
}
