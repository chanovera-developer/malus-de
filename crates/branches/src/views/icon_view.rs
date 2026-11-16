use crate::db::{self, LayoutMode};
use crate::fs_backend::{open_file, FileItem};
use crate::views::{show_item_context_menu, NavigateCallback, SelectionCallback};
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Fixed, GestureClick, Image, Label,
    Orientation, PolicyType, Popover, ScrolledWindow,
};
use malus_de_common::config::{BranchesConfig, IconGridMetrics};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

const MARGIN_LEFT: i32 = 24;
const MARGIN_TOP: i32 = 24;
const FALLBACK_VIEWPORT_WIDTH: i32 = 760;

#[inline]
fn clamp_branches_icon_size(size: i32) -> i32 {
    size.clamp(
        BranchesConfig::MIN_ICON_SIZE as i32,
        BranchesConfig::MAX_ICON_SIZE as i32,
    )
}

#[inline]
fn clamp_grid_spacing(spacing: i32) -> i32 {
    spacing.clamp(
        BranchesConfig::MIN_GRID_SPACING as i32,
        BranchesConfig::MAX_GRID_SPACING as i32,
    )
}

/// Calcula cuántas columnas caben en el ancho visible del panel según el ancho de celda actual.
#[inline]
pub(crate) fn columns_for_width(available_width: i32, cell_w: i32) -> i32 {
    let cell_w = cell_w.max(1);
    let effective_w = if available_width > 0 {
        available_width
    } else {
        FALLBACK_VIEWPORT_WIDTH
    };
    ((effective_w - MARGIN_LEFT).max(cell_w) / cell_w).max(1)
}

#[inline]
fn snap_to_grid_cell(raw_x: i32, raw_y: i32, cell_w: i32, cell_h: i32) -> (i32, i32) {
    let col = ((raw_x - MARGIN_LEFT + cell_w / 2) / cell_w).max(0);
    let row = ((raw_y - MARGIN_TOP + cell_h / 2) / cell_h).max(0);
    (MARGIN_LEFT + col * cell_w, MARGIN_TOP + row * cell_h)
}

struct ItemWidgetData {
    widget: GtkBox,
    image: Image,
    item: FileItem,
    pos: Rc<Cell<(i32, i32)>>,
}

pub struct IconView {
    container: ScrolledWindow,
    fixed_canvas: Fixed,
    current_folder: Rc<RefCell<PathBuf>>,
    layout_mode: Rc<RefCell<LayoutMode>>,
    default_icon_size: Rc<Cell<i32>>,
    active_icon_size: Rc<Cell<i32>>,
    grid_spacing: Rc<Cell<i32>>,
    folder_icon_override: Rc<Cell<Option<i32>>>,
    last_auto_cols: Rc<Cell<i32>>,
    item_order: Rc<RefCell<Vec<String>>>,
    item_widgets: Rc<RefCell<HashMap<String, Rc<RefCell<ItemWidgetData>>>>>,
    selected_items: Rc<RefCell<HashSet<String>>>,
    on_navigate: NavigateCallback,
    on_selection_changed: SelectionCallback,
}

impl IconView {
    pub fn new(on_navigate: NavigateCallback, on_selection_changed: SelectionCallback) -> Self {
        let cfg = BranchesConfig::load();
        let initial_icon_size = cfg.clamped_icon_size();
        let initial_spacing = cfg.clamped_grid_spacing();

        let fixed_canvas = Fixed::builder()
            .hexpand(true)
            .vexpand(true)
            .css_classes(["branches-icon-canvas"])
            .build();

        let container = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vscrollbar_policy(PolicyType::Automatic)
            .css_classes(["branches-icon-view"])
            .child(&fixed_canvas)
            .build();

        let current_folder = Rc::new(RefCell::new(PathBuf::from("/")));
        let layout_mode = Rc::new(RefCell::new(LayoutMode::Auto));
        let default_icon_size = Rc::new(Cell::new(initial_icon_size));
        let active_icon_size = Rc::new(Cell::new(initial_icon_size));
        let grid_spacing = Rc::new(Cell::new(initial_spacing));
        let folder_icon_override = Rc::new(Cell::new(None));
        let last_auto_cols = Rc::new(Cell::new(0));
        let item_order = Rc::new(RefCell::new(Vec::new()));
        let item_widgets = Rc::new(RefCell::new(HashMap::new()));
        let selected_items = Rc::new(RefCell::new(HashSet::new()));

        let view = Self {
            container,
            fixed_canvas,
            current_folder,
            layout_mode,
            default_icon_size,
            active_icon_size,
            grid_spacing,
            folder_icon_override,
            last_auto_cols,
            item_order,
            item_widgets,
            selected_items,
            on_navigate,
            on_selection_changed,
        };

        // Gesto en el lienzo de fondo y reflow responsivo al redimensionar ventana o panel
        view.setup_canvas_gestures();
        view.setup_resize_reflow();

        view
    }

    fn viewport_width(&self) -> i32 {
        let page_w = self.container.hadjustment().page_size().floor() as i32;
        if page_w > 0 {
            page_w
        } else {
            self.container.width()
        }
    }

    fn setup_resize_reflow(&self) {
        let weak_helper = self.make_weak();
        let hadj = self.container.hadjustment();
        hadj.connect_page_size_notify(move |adj| {
            let new_width = adj.page_size().floor() as i32;
            if new_width <= 0 {
                return;
            }
            if *weak_helper.layout_mode.borrow() != LayoutMode::Auto {
                return;
            }
            let metrics = IconGridMetrics::from_icon_size(
                weak_helper.active_icon_size.get(),
                weak_helper.grid_spacing.get(),
            );
            let new_cols = columns_for_width(new_width, metrics.cell_width());
            if new_cols != weak_helper.last_auto_cols.get() {
                weak_helper.last_auto_cols.set(new_cols);
                weak_helper.relayout_auto_grid_with_width(new_width);
            }
        });
    }

    fn make_weak(&self) -> IconViewWeak {
        IconViewWeak {
            container: self.container.clone(),
            fixed: self.fixed_canvas.clone(),
            current_folder: self.current_folder.clone(),
            layout_mode: self.layout_mode.clone(),
            active_icon_size: self.active_icon_size.clone(),
            grid_spacing: self.grid_spacing.clone(),
            last_auto_cols: self.last_auto_cols.clone(),
            item_order: self.item_order.clone(),
            item_widgets: self.item_widgets.clone(),
        }
    }

    pub fn current_metrics(&self) -> IconGridMetrics {
        IconGridMetrics::from_icon_size(self.active_icon_size.get(), self.grid_spacing.get())
    }

    /// Actualiza el tamaño predeterminado de iconos/carpetas y el espaciado de cuadrícula
    /// (listo para invocarse en vivo desde futuros ajustes o vistas de opciones de Branches).
    pub fn set_layout_preferences(&self, default_icon_size: i32, grid_spacing: i32) {
        let clamped_default = clamp_branches_icon_size(default_icon_size);
        let clamped_spacing = clamp_grid_spacing(grid_spacing);
        self.default_icon_size.set(clamped_default);
        self.grid_spacing.set(clamped_spacing);

        let effective_size = self
            .folder_icon_override
            .get()
            .map(clamp_branches_icon_size)
            .unwrap_or(clamped_default);
        self.apply_active_metrics(effective_size, clamped_spacing);
    }

    fn apply_active_metrics(&self, icon_size: i32, grid_spacing: i32) {
        self.active_icon_size.set(clamp_branches_icon_size(icon_size));
        self.grid_spacing.set(clamp_grid_spacing(grid_spacing));
        let metrics = self.current_metrics();
        let cell_w = metrics.cell_width();
        let cell_h = metrics.cell_height();
        let viewport_w = self.viewport_width();
        let cols = columns_for_width(viewport_w, cell_w);

        let widgets_map = self.item_widgets.borrow();
        if widgets_map.is_empty() {
            return;
        }

        let mode = *self.layout_mode.borrow();
        if mode == LayoutMode::Auto {
            drop(widgets_map);
            for data_rc in self.item_widgets.borrow().values() {
                let d = data_rc.borrow();
                d.image.set_pixel_size(metrics.icon_size);
                d.widget.set_width_request(metrics.box_width);
                d.widget.set_height_request(metrics.box_height);
            }
            self.last_auto_cols.set(if viewport_w > 0 { cols } else { 0 });
            self.make_weak().relayout_auto_grid_with_width(viewport_w);
            return;
        }

        let mut max_x = 400;
        let mut max_y = 300;
        let mut ordered: Vec<_> = widgets_map.values().cloned().collect();
        ordered.sort_by_key(|d| {
            let p = d.borrow().pos.get();
            (p.1, p.0)
        });

        let mut used_slots: HashSet<(i32, i32)> = HashSet::new();
        for data_rc in ordered {
            let d = data_rc.borrow();
            d.image.set_pixel_size(metrics.icon_size);
            d.widget.set_width_request(metrics.box_width);
            d.widget.set_height_request(metrics.box_height);
            let (mut px, mut py) = d.pos.get();

            if mode == LayoutMode::SnapToGrid {
                let mut col = ((px - MARGIN_LEFT + cell_w / 2) / cell_w).max(0);
                let mut row = ((py - MARGIN_TOP + cell_h / 2) / cell_h).max(0);
                while used_slots.contains(&(col, row)) {
                    col += 1;
                    if col >= cols {
                        col = 0;
                        row += 1;
                    }
                }
                used_slots.insert((col, row));
                px = MARGIN_LEFT + col * cell_w;
                py = MARGIN_TOP + row * cell_h;
                d.pos.set((px, py));
                self.fixed_canvas.move_(&d.widget, px as f64, py as f64);
            }

            max_x = max_x.max(px + cell_w);
            max_y = max_y.max(py + cell_h);
        }

        self.fixed_canvas.set_size_request(max_x + 40, max_y + 80);
    }

    pub fn widget(&self) -> &ScrolledWindow {
        &self.container
    }

    pub fn set_items(&self, items: &[FileItem]) {
        // Limpiar widgets anteriores
        while let Some(child) = self.fixed_canvas.first_child() {
            self.fixed_canvas.remove(&child);
        }
        self.item_widgets.borrow_mut().clear();
        self.item_order.borrow_mut().clear();
        self.selected_items.borrow_mut().clear();
        (self.on_selection_changed)(0);

        if items.is_empty() {
            self.fixed_canvas.set_size_request(1, 300);
            return;
        }

        let folder = items[0].path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("/"));
        *self.current_folder.borrow_mut() = folder.clone();

        // Cargar posiciones, modo de layout y tamaño personalizado de carpeta desde SQLite
        let mut db_positions = HashMap::new();
        let mut current_mode = LayoutMode::Auto;
        let mut folder_override = None;

        if let Ok(conn) = db::open_db() {
            db_positions = db::get_positions(&conn, &folder);
            current_mode = db::get_layout_mode(&conn, &folder);
            folder_override = db::get_folder_icon_size(&conn, &folder);
        }
        *self.layout_mode.borrow_mut() = current_mode;
        self.folder_icon_override.set(folder_override);
        let active_size = folder_override
            .map(clamp_branches_icon_size)
            .unwrap_or_else(|| self.default_icon_size.get());
        self.active_icon_size.set(active_size);

        let metrics = self.current_metrics();
        let cell_w = metrics.cell_width();
        let cell_h = metrics.cell_height();
        let viewport_w = self.viewport_width();
        let cols = columns_for_width(viewport_w, cell_w);
        self.last_auto_cols.set(if viewport_w > 0 { cols } else { 0 });

        // Calcular colocación para cada elemento
        let mut occupied_slots: HashSet<(i32, i32)> = HashSet::new();
        if current_mode != LayoutMode::Auto {
            for (_, (x, y)) in &db_positions {
                let col = ((*x - MARGIN_LEFT + cell_w / 2) / cell_w).max(0);
                let row = ((*y - MARGIN_TOP + cell_h / 2) / cell_h).max(0);
                occupied_slots.insert((col, row));
            }
        }

        let mut next_col = 0;
        let mut next_row = 0;
        let mut max_x = 400;
        let mut max_y = 300;

        let mut new_positions_to_save: HashMap<String, (i32, i32)> = HashMap::new();

        for item in items {
            let (pos_x, pos_y) = if current_mode != LayoutMode::Auto && db_positions.contains_key(&item.name) {
                let pos = db_positions[&item.name];
                if current_mode == LayoutMode::SnapToGrid {
                    snap_to_grid_cell(pos.0, pos.1, cell_w, cell_h)
                } else {
                    (pos.0, pos.1)
                }
            } else {
                // Asignar siguiente celda libre según el ancho actual de la ventana
                while occupied_slots.contains(&(next_col, next_row)) {
                    next_col += 1;
                    if next_col >= cols {
                        next_col = 0;
                        row_advance(&mut next_row);
                    }
                }
                let x = MARGIN_LEFT + next_col * cell_w;
                let y = MARGIN_TOP + next_row * cell_h;
                occupied_slots.insert((next_col, next_row));

                next_col += 1;
                if next_col >= cols {
                    next_col = 0;
                    row_advance(&mut next_row);
                }

                if current_mode != LayoutMode::Auto {
                    new_positions_to_save.insert(item.name.clone(), (x, y));
                }
                (x, y)
            };

            max_x = max_x.max(pos_x + cell_w);
            max_y = max_y.max(pos_y + cell_h);

            let pos_cell = Rc::new(Cell::new((pos_x, pos_y)));
            let (widget, image) = self.create_spatial_icon_widget(item, pos_cell.clone(), metrics);
            self.fixed_canvas.put(&widget, pos_x as f64, pos_y as f64);

            self.item_order.borrow_mut().push(item.name.clone());
            let data = Rc::new(RefCell::new(ItemWidgetData {
                widget,
                image,
                item: item.clone(),
                pos: pos_cell,
            }));
            self.item_widgets.borrow_mut().insert(item.name.clone(), data);
        }

        // Guardar en segundo plano las posiciones nuevas generadas (cuando no está en Auto)
        if !new_positions_to_save.is_empty() {
            if let Ok(mut conn) = db::open_db() {
                let _ = db::save_all_positions(&mut conn, &folder, &new_positions_to_save);
            }
        }

        // En modo Auto dejamos min_width=1 para permitir reducir la ventana y que la cuadrícula haga reflow sin barra horizontal forzada
        let min_w = if current_mode == LayoutMode::Auto { 1 } else { max_x + 40 };
        self.fixed_canvas.set_size_request(min_w, max_y + 80);
    }

    fn create_spatial_icon_widget(
        &self,
        item: &FileItem,
        pos_cell: Rc<Cell<(i32, i32)>>,
        metrics: IconGridMetrics,
    ) -> (GtkBox, Image) {
        let vbox = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .halign(Align::Center)
            .valign(Align::Start)
            .width_request(metrics.box_width)
            .height_request(metrics.box_height)
            .css_classes(["branches-icon-item"])
            .build();

        let img = Image::from_gicon(&item.gicon);
        img.set_pixel_size(metrics.icon_size);
        vbox.append(&img);

        let label = Label::builder()
            .label(&item.name)
            .ellipsize(gtk4::pango::EllipsizeMode::Middle)
            .max_width_chars(12)
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            .lines(2)
            .halign(Align::Center)
            .css_classes(["branches-icon-label"])
            .build();
        vbox.append(&label);

        // 1. Doble clic para abrir y Clic derecho para menú contextual
        let click_gesture = GestureClick::new();
        click_gesture.set_button(0);
        let path = item.path.clone();
        let is_dir = item.is_dir;
        let on_nav = self.on_navigate.clone();
        let sel_items = self.selected_items.clone();
        let name_for_click = item.name.clone();
        let vbox_for_click = vbox.clone();
        let on_sel_cb = self.on_selection_changed.clone();

        click_gesture.connect_pressed(move |g, n_press, x, y| {
            let button = g.current_button();

            if button == gdk::BUTTON_PRIMARY {
                if n_press == 1 {
                    // Selección simple
                    sel_items.borrow_mut().clear();
                    sel_items.borrow_mut().insert(name_for_click.clone());
                    vbox_for_click.add_css_class("selected");
                    on_sel_cb(1);
                } else if n_press == 2 {
                    if is_dir {
                        on_nav(path.clone());
                    } else {
                        let _ = open_file(&path);
                    }
                }
            } else if button == gdk::BUTTON_SECONDARY {
                g.set_state(gtk4::EventSequenceState::Claimed);
                show_item_context_menu(&vbox_for_click, &path, is_dir, Some((x, y)), Some(&on_nav));
            }
        });
        vbox.add_controller(click_gesture);

        // 2. Drag & Drop entre ventanas/paneles (DragSource Wayland con icono translúcido estilo Finder)
        let drag_source = gtk4::DragSource::new();
        drag_source.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
        let path_for_drag = item.path.clone();

        // Pre-adjuntar la miniatura completa inicial para respuesta inmediata al cursor
        let initial_paintable = gtk4::WidgetPaintable::new(Some(&vbox));
        drag_source.set_icon(
            Some(&initial_paintable),
            metrics.box_width / 2,
            metrics.icon_size / 2,
        );

        let vbox_prep = vbox.clone();
        drag_source.connect_prepare(move |source, click_x, click_y| {
            vbox_prep.add_css_class("selected");
            let paintable = gtk4::WidgetPaintable::new(Some(&vbox_prep));
            source.set_icon(Some(&paintable), click_x.round() as i32, click_y.round() as i32);

            Some(crate::views::build_file_drag_content_provider(&path_for_drag))
        });

        let fixed_drag = self.fixed_canvas.clone();
        let vbox_drag = vbox.clone();
        drag_source.connect_drag_begin(move |_source, _drag| {
            fixed_drag.move_(&vbox_drag, -5000.0, -5000.0);
        });

        let fixed_end = self.fixed_canvas.clone();
        let vbox_end = vbox.clone();
        let pos_end = pos_cell;
        drag_source.connect_drag_end(move |_, _drag, _| {
            let (cx, _cy) = fixed_end.child_position(&vbox_end);
            if cx < 0.0 {
                let (px, py) = pos_end.get();
                fixed_end.move_(&vbox_end, px as f64, py as f64);
            }
        });
        vbox.add_controller(drag_source);

        // Si es carpeta, permitir soltar archivos encima con resaltado azul acrílico estilo Finder
        if is_dir {
            let folder_drop = gtk4::DropTarget::new(
                gdk::FileList::static_type(),
                gdk::DragAction::COPY | gdk::DragAction::MOVE,
            );
            let vbox_hover = vbox.clone();
            folder_drop.connect_enter(move |_, _x, _y| {
                vbox_hover.add_css_class("folder-drop-hover");
                gdk::DragAction::MOVE
            });
            folder_drop.connect_motion(move |_, _x, _y| {
                gdk::DragAction::MOVE
            });

            let vbox_leave = vbox.clone();
            folder_drop.connect_leave(move |_| {
                vbox_leave.remove_css_class("folder-drop-hover");
            });

            let vbox_drop = vbox.clone();
            let dest_dir = item.path.clone();
            let on_nav_reload = self.on_navigate.clone();
            let cur_folder = self.current_folder.clone();
            folder_drop.connect_drop(move |_, value, _, _| {
                vbox_drop.remove_css_class("folder-drop-hover");
                if let Ok(file_list) = value.get::<gdk::FileList>() {
                    let mut moved = false;
                    for file in file_list.files() {
                        if let Some(src) = file.path() {
                            if let Some(fname) = src.file_name() {
                                let target = dest_dir.join(fname);
                                if target != src {
                                    let dest_file = gio::File::for_path(&target);
                                    if file.move_(
                                        &dest_file,
                                        gio::FileCopyFlags::NONE,
                                        gio::Cancellable::NONE,
                                        None,
                                    ).is_ok() || std::fs::rename(&src, &target).is_ok() {
                                        moved = true;
                                    }
                                }
                            }
                        }
                    }
                    if moved {
                        let cf = cur_folder.borrow().clone();
                        on_nav_reload(cf);
                        return true;
                    }
                }
                false
            });
            vbox.add_controller(folder_drop);
        }

        (vbox, img)
    }

    fn setup_canvas_gestures(&self) {
        let click = GestureClick::new();
        click.set_button(0);
        let sel_items = self.selected_items.clone();
        let widgets = self.item_widgets.clone();
        let sel_cb = self.on_selection_changed.clone();
        let fixed = self.fixed_canvas.clone();
        let folder_ref = self.current_folder.clone();
        let layout_ref = self.layout_mode.clone();

        let self_weak: Rc<RefCell<Option<IconViewWeak>>> = Rc::new(RefCell::new(None));
        *self_weak.borrow_mut() = Some(self.make_weak());

        let weak_for_click = self_weak.clone();

        click.connect_pressed(move |g, _n, x, y| {
            let button = g.current_button();

            if button == gdk::BUTTON_PRIMARY {
                // Clic en fondo vacío: deseleccionar todo
                for (_, data) in widgets.borrow().iter() {
                    data.borrow().widget.remove_css_class("selected");
                }
                sel_items.borrow_mut().clear();
                sel_cb(0);
            } else if button == gdk::BUTTON_SECONDARY {
                // Menú contextual del lienzo de fondo
                if let Some(ref w) = *weak_for_click.borrow() {
                    show_canvas_context_menu(&fixed, x, y, folder_ref.clone(), layout_ref.clone(), w);
                }
            }
        });

        self.fixed_canvas.add_controller(click);

        // Destino para soltar archivos (tanto reorganización interna como desde Escritorio)
        let canvas_drop = gtk4::DropTarget::new(
            gdk::FileList::static_type(),
            gdk::DragAction::COPY | gdk::DragAction::MOVE,
        );
        canvas_drop.connect_enter(move |_, _x, _y| {
            gdk::DragAction::MOVE
        });
        canvas_drop.connect_motion(move |_, _x, _y| {
            gdk::DragAction::MOVE
        });
        let folder_drop_ref = self.current_folder.clone();
        let layout_drop_ref = self.layout_mode.clone();
        let icon_size_drop_ref = self.active_icon_size.clone();
        let spacing_drop_ref = self.grid_spacing.clone();
        let fixed_drop_canvas = self.fixed_canvas.clone();
        let widgets_drop_map = self.item_widgets.clone();
        let on_nav_drop = self.on_navigate.clone();

        canvas_drop.connect_drop(move |_, value, x, y| {
            if let Ok(file_list) = value.get::<gdk::FileList>() {
                let cur_folder = folder_drop_ref.borrow().clone();
                let mode = *layout_drop_ref.borrow();
                let metrics = IconGridMetrics::from_icon_size(
                    icon_size_drop_ref.get(),
                    spacing_drop_ref.get(),
                );
                let cell_w = metrics.cell_width();
                let cell_h = metrics.cell_height();
                let mut moved_external = false;
                let mut moved_internal = false;

                for file in file_list.files() {
                    if let Some(src) = file.path() {
                        let is_internal = src.parent().map(|p| p == cur_folder).unwrap_or(false);
                        if is_internal {
                            // Permitir mover iconos libremente dentro de la ventana incluso si estaba en Cuadrícula automática:
                            // si estaba en Auto, al soltar un icono manualmente se conservan las posiciones actuales y se pasa a SnapToGrid.
                            if let Some(fname) = src.file_name() {
                                let fname_str = fname.to_string_lossy().to_string();
                                let raw_x = (x - (metrics.box_width as f64 / 2.0)).max(10.0) as i32;
                                let raw_y = (y - (metrics.icon_size as f64 / 2.0)).max(10.0) as i32;
                                let (final_x, final_y) = match mode {
                                    LayoutMode::Auto | LayoutMode::SnapToGrid => {
                                        snap_to_grid_cell(raw_x, raw_y, cell_w, cell_h)
                                    }
                                    LayoutMode::Freeform => (raw_x, raw_y),
                                };

                                if let Some(item_data) = widgets_drop_map.borrow().get(&fname_str) {
                                    let d = item_data.borrow();
                                    d.pos.set((final_x, final_y));
                                    fixed_drop_canvas.move_(&d.widget, final_x as f64, final_y as f64);
                                    d.widget.add_css_class("landing");
                                    let w_clone = d.widget.clone();
                                    glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                                        w_clone.remove_css_class("landing");
                                    });
                                }

                                if mode == LayoutMode::Auto {
                                    *layout_drop_ref.borrow_mut() = LayoutMode::SnapToGrid;
                                    let mut all_positions = HashMap::new();
                                    for (name, data_rc) in widgets_drop_map.borrow().iter() {
                                        all_positions.insert(name.clone(), data_rc.borrow().pos.get());
                                    }
                                    if let Ok(mut conn) = db::open_db() {
                                        let _ = db::set_layout_mode(&conn, &cur_folder, LayoutMode::SnapToGrid);
                                        let _ = db::save_all_positions(&mut conn, &cur_folder, &all_positions);
                                    }
                                } else if let Ok(conn) = db::open_db() {
                                    let _ = db::save_position(&conn, &cur_folder, &fname_str, final_x, final_y);
                                }

                                let mut max_x = 400;
                                let mut max_y = 300;
                                for data_rc in widgets_drop_map.borrow().values() {
                                    let (px, py) = data_rc.borrow().pos.get();
                                    max_x = max_x.max(px + cell_w);
                                    max_y = max_y.max(py + cell_h);
                                }
                                fixed_drop_canvas.set_size_request(max_x + 40, max_y + 80);
                                moved_internal = true;
                            }
                        } else {
                            if let Some(fname) = src.file_name() {
                                let dest = cur_folder.join(fname);
                                if dest != src {
                                    let dest_file = gio::File::for_path(&dest);
                                    if file.move_(
                                        &dest_file,
                                        gio::FileCopyFlags::NONE,
                                        gio::Cancellable::NONE,
                                        None,
                                    ).is_ok() || std::fs::rename(&src, &dest).is_ok() {
                                        moved_external = true;
                                    }
                                }
                            }
                        }
                    }
                }
                if moved_external {
                    on_nav_drop(cur_folder);
                    return true;
                }
                if moved_internal {
                    return true;
                }
            }
            false
        });
        self.fixed_canvas.add_controller(canvas_drop);
    }
}

#[inline]
fn row_advance(row: &mut i32) {
    *row += 1;
}

#[derive(Clone)]
struct IconViewWeak {
    container: ScrolledWindow,
    fixed: Fixed,
    current_folder: Rc<RefCell<PathBuf>>,
    layout_mode: Rc<RefCell<LayoutMode>>,
    active_icon_size: Rc<Cell<i32>>,
    grid_spacing: Rc<Cell<i32>>,
    last_auto_cols: Rc<Cell<i32>>,
    item_order: Rc<RefCell<Vec<String>>>,
    item_widgets: Rc<RefCell<HashMap<String, Rc<RefCell<ItemWidgetData>>>>>,
}

impl IconViewWeak {
    fn viewport_width(&self) -> i32 {
        let page_w = self.container.hadjustment().page_size().floor() as i32;
        if page_w > 0 {
            page_w
        } else {
            self.container.width()
        }
    }

    fn relayout_auto_grid_with_width(&self, available_width: i32) {
        let metrics = IconGridMetrics::from_icon_size(
            self.active_icon_size.get(),
            self.grid_spacing.get(),
        );
        let cell_w = metrics.cell_width();
        let cell_h = metrics.cell_height();
        let cols = columns_for_width(available_width, cell_w);

        let order = self.item_order.borrow();
        let widgets = self.item_widgets.borrow();
        let mut max_y = 300;
        let mut idx: i32 = 0;

        for name in order.iter() {
            if let Some(data_rc) = widgets.get(name) {
                let col = idx % cols;
                let row = idx / cols;
                let x = MARGIN_LEFT + col * cell_w;
                let y = MARGIN_TOP + row * cell_h;

                let d = data_rc.borrow();
                d.pos.set((x, y));
                self.fixed.move_(&d.widget, x as f64, y as f64);
                max_y = max_y.max(y + cell_h);
                idx += 1;
            }
        }

        self.fixed.set_size_request(1, max_y + 80);
    }

    fn snap_existing_to_grid(&self) {
        let metrics = IconGridMetrics::from_icon_size(
            self.active_icon_size.get(),
            self.grid_spacing.get(),
        );
        let cell_w = metrics.cell_width();
        let cell_h = metrics.cell_height();
        let cols = columns_for_width(self.viewport_width(), cell_w);

        let mut ordered: Vec<_> = self.item_widgets.borrow().values().cloned().collect();
        ordered.sort_by_key(|d| {
            let p = d.borrow().pos.get();
            (p.1, p.0)
        });

        let mut used_slots: HashSet<(i32, i32)> = HashSet::new();
        let mut positions = HashMap::new();
        let mut max_x = 400;
        let mut max_y = 300;

        for data_rc in ordered {
            let d = data_rc.borrow();
            let (cur_x, cur_y) = d.pos.get();
            let mut col = ((cur_x - MARGIN_LEFT + cell_w / 2) / cell_w).max(0);
            let mut row = ((cur_y - MARGIN_TOP + cell_h / 2) / cell_h).max(0);
            while used_slots.contains(&(col, row)) {
                col += 1;
                if col >= cols {
                    col = 0;
                    row += 1;
                }
            }
            used_slots.insert((col, row));
            let x = MARGIN_LEFT + col * cell_w;
            let y = MARGIN_TOP + row * cell_h;
            d.pos.set((x, y));
            self.fixed.move_(&d.widget, x as f64, y as f64);
            positions.insert(d.item.name.clone(), (x, y));
            max_x = max_x.max(x + cell_w);
            max_y = max_y.max(y + cell_h);
        }

        self.fixed.set_size_request(max_x + 40, max_y + 80);
        let folder = self.current_folder.borrow().clone();
        if let Ok(mut conn) = db::open_db() {
            let _ = db::save_all_positions(&mut conn, &folder, &positions);
        }
    }

    fn rearrange(&self, by_name: bool) {
        let mut list: Vec<_> = self.item_widgets.borrow().values().cloned().collect();
        if by_name {
            list.sort_by(|a, b| a.borrow().item.name.to_lowercase().cmp(&b.borrow().item.name.to_lowercase()));
        }

        let metrics = IconGridMetrics::from_icon_size(
            self.active_icon_size.get(),
            self.grid_spacing.get(),
        );
        let cell_w = metrics.cell_width();
        let cell_h = metrics.cell_height();
        let viewport_w = self.viewport_width();
        let cols = columns_for_width(viewport_w, cell_w);

        let mut positions = HashMap::new();
        let mut new_order = Vec::with_capacity(list.len());
        let folder = self.current_folder.borrow().clone();
        let mut max_x = 400;
        let mut max_y = 300;

        for (idx, data_rc) in list.iter().enumerate() {
            let col = (idx as i32) % cols;
            let row = (idx as i32) / cols;
            let x = MARGIN_LEFT + col * cell_w;
            let y = MARGIN_TOP + row * cell_h;

            let d = data_rc.borrow();
            d.pos.set((x, y));
            self.fixed.move_(&d.widget, x as f64, y as f64);
            positions.insert(d.item.name.clone(), (x, y));
            new_order.push(d.item.name.clone());
            max_x = max_x.max(x + cell_w);
            max_y = max_y.max(y + cell_h);
        }

        *self.item_order.borrow_mut() = new_order;
        let mode = *self.layout_mode.borrow();
        let min_w = if mode == LayoutMode::Auto { 1 } else { max_x + 40 };
        self.fixed.set_size_request(min_w, max_y + 80);

        if mode != LayoutMode::Auto {
            if let Ok(mut conn) = db::open_db() {
                let _ = db::save_all_positions(&mut conn, &folder, &positions);
            }
        }
    }
}

use super::make_context_menu_button;

fn show_canvas_context_menu(
    parent: &impl IsA<gtk4::Widget>,
    x: f64,
    y: f64,
    folder_ref: Rc<RefCell<PathBuf>>,
    layout_ref: Rc<RefCell<LayoutMode>>,
    view_weak: &IconViewWeak,
) {
    let popover = Popover::builder()
        .has_arrow(false)
        .css_classes(["malus-popover-menu"])
        .build();
    popover.set_parent(parent);
    popover.connect_closed(|p| p.unparent());

    let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
    popover.set_pointing_to(Some(&rect));

    let vbox = GtkBox::new(Orientation::Vertical, 2);

    let lbl_layout = Label::builder()
        .label("Disposición de iconos")
        .halign(Align::Start)
        .css_classes(["dim-label", "bold"])
        .build();
    vbox.append(&lbl_layout);

    let current_mode = *layout_ref.borrow();
    let label_with_check = |active: bool, text: &str| -> String {
        if active {
            format!("✓ {}", text)
        } else {
            text.to_string()
        }
    };

    // 1. Cuadrícula automática adaptada al ancho de ventana
    let pop_auto = popover.clone();
    let l_auto = layout_ref.clone();
    let f_auto = folder_ref.clone();
    let w_auto = view_weak.clone();
    let btn_auto = make_context_menu_button(&label_with_check(
        current_mode == LayoutMode::Auto,
        "Cuadrícula automática",
    ));
    btn_auto.connect_clicked(move |_| {
        pop_auto.popdown();
        *l_auto.borrow_mut() = LayoutMode::Auto;
        let vw = w_auto.viewport_width();
        let metrics = IconGridMetrics::from_icon_size(
            w_auto.active_icon_size.get(),
            w_auto.grid_spacing.get(),
        );
        w_auto.last_auto_cols.set(columns_for_width(vw, metrics.cell_width()));
        w_auto.relayout_auto_grid_with_width(vw);
        if let Ok(conn) = db::open_db() {
            let _ = db::set_layout_mode(&conn, &f_auto.borrow(), LayoutMode::Auto);
        }
    });
    vbox.append(&btn_auto);

    // 2. Alinear a la cuadrícula (Snap to Grid con movimiento libre)
    let pop_clone = popover.clone();
    let l_ref = layout_ref.clone();
    let f_ref = folder_ref.clone();
    let w_snap = view_weak.clone();
    let btn_snap = make_context_menu_button(&label_with_check(
        current_mode == LayoutMode::SnapToGrid,
        "Alinear a la cuadrícula (Snap to Grid)",
    ));
    btn_snap.connect_clicked(move |_| {
        pop_clone.popdown();
        *l_ref.borrow_mut() = LayoutMode::SnapToGrid;
        if let Ok(conn) = db::open_db() {
            let _ = db::set_layout_mode(&conn, &f_ref.borrow(), LayoutMode::SnapToGrid);
        }
        w_snap.snap_existing_to_grid();
    });
    vbox.append(&btn_snap);

    // 3. Colocación libre
    let pop_clone2 = popover.clone();
    let l_ref2 = layout_ref.clone();
    let f_ref2 = folder_ref.clone();
    let btn_free = make_context_menu_button(&label_with_check(
        current_mode == LayoutMode::Freeform,
        "Colocación libre (Freeform)",
    ));
    btn_free.connect_clicked(move |_| {
        pop_clone2.popdown();
        *l_ref2.borrow_mut() = LayoutMode::Freeform;
        if let Ok(conn) = db::open_db() {
            let _ = db::set_layout_mode(&conn, &f_ref2.borrow(), LayoutMode::Freeform);
        }
    });
    vbox.append(&btn_free);

    let sep = gtk4::Separator::new(Orientation::Horizontal);
    vbox.append(&sep);

    // Limpiar / Reorganizar automáticamente según ancho de ventana
    let pop_clone3 = popover.clone();
    let w_clean = view_weak.clone();
    let btn_clean = make_context_menu_button("Reordenar cuadrícula");
    btn_clean.connect_clicked(move |_| {
        pop_clone3.popdown();
        w_clean.rearrange(true);
    });
    vbox.append(&btn_clean);

    popover.set_child(Some(&vbox));
    popover.popup();
}

#[cfg(test)]
mod tests {
    use super::{columns_for_width, snap_to_grid_cell, MARGIN_LEFT, MARGIN_TOP};
    use malus_de_common::config::BranchesConfig;

    #[test]
    fn grid_columns_adapt_to_window_and_pane_width() {
        let cell_w = BranchesConfig::default().icon_grid_metrics().cell_width();
        assert_eq!(cell_w, 116);

        // Panel estrecho (Split View ~420px): 3 columnas
        assert_eq!(columns_for_width(420, cell_w), 3);
        // Ventana estándar (~860px tras restar sidebar): 7 columnas
        assert_eq!(columns_for_width(860, cell_w), 7);
        // Ventana maximizada (~1700px): 14 columnas
        assert_eq!(columns_for_width(1700, cell_w), 14);
        // Mínimo garantizado de 1 columna aunque sea muy estrecha
        assert_eq!(columns_for_width(80, cell_w), 1);
    }

    #[test]
    fn snap_to_grid_cell_aligns_freely_dragged_icon_to_nearest_slot() {
        let metrics = BranchesConfig::default().icon_grid_metrics();
        let (x, y) = snap_to_grid_cell(
            MARGIN_LEFT + metrics.cell_width() * 2 + 15,
            MARGIN_TOP + metrics.cell_height() - 10,
            metrics.cell_width(),
            metrics.cell_height(),
        );
        assert_eq!(x, MARGIN_LEFT + metrics.cell_width() * 2);
        assert_eq!(y, MARGIN_TOP + metrics.cell_height());
    }
}
