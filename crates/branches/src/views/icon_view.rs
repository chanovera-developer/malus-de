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
const GRID_REORDER_ANIM_MS: i64 = 240;

#[inline]
pub(crate) fn ease_out_cubic(t: f64) -> f64 {
    let p = t.clamp(0.0, 1.0);
    1.0 - (1.0 - p).powi(3)
}

#[inline]
pub(crate) fn interpolate_point(start: (f64, f64), target: (f64, f64), eased: f64) -> (f64, f64) {
    (
        start.0 + (target.0 - start.0) * eased,
        start.1 + (target.1 - start.1) * eased,
    )
}

#[inline]
fn current_canvas_pos(
    fixed: &Fixed,
    widget: &GtkBox,
    fallback: (i32, i32),
    is_animating: bool,
) -> (f64, f64) {
    if !is_animating {
        return (fallback.0 as f64, fallback.1 as f64);
    }
    let (cx, cy) = fixed.child_position(widget);
    if cx < 0.0 || cy < 0.0 {
        (fallback.0 as f64, fallback.1 as f64)
    } else {
        (cx, cy)
    }
}

fn animate_grid_moves(
    fixed: &Fixed,
    anim_gen: &Rc<Cell<u64>>,
    is_animating: &Rc<Cell<bool>>,
    moves: Vec<(GtkBox, (f64, f64), (f64, f64))>,
    final_size_request: (i32, i32),
    duration_ms: i64,
) {
    let next_gen = anim_gen.get().wrapping_add(1);
    anim_gen.set(next_gen);

    if moves.is_empty() || fixed.root().is_none() || duration_ms <= 0 {
        is_animating.set(false);
        for (widget, _, target) in &moves {
            fixed.move_(widget, target.0, target.1);
        }
        fixed.set_size_request(final_size_request.0, final_size_request.1);
        return;
    }

    let mut anim_max_y = final_size_request.1;
    for (_, start, target) in &moves {
        anim_max_y = anim_max_y
            .max(start.1.ceil() as i32 + 120)
            .max(target.1.ceil() as i32 + 120);
    }
    fixed.set_size_request(final_size_request.0, anim_max_y);
    is_animating.set(true);

    let start_time: Rc<Cell<Option<i64>>> = Rc::new(Cell::new(None));
    let fixed_clone = fixed.clone();
    let gen_cell = anim_gen.clone();
    let animating_cell = is_animating.clone();

    fixed.add_tick_callback(move |_, frame_clock| {
        if gen_cell.get() != next_gen {
            return glib::ControlFlow::Break;
        }

        let now = frame_clock.frame_time() / 1000;
        let start = match start_time.get() {
            Some(t) => t,
            None => {
                start_time.set(Some(now));
                now
            }
        };

        let elapsed = now - start;
        if elapsed >= duration_ms {
            for (widget, _, target) in &moves {
                fixed_clone.move_(widget, target.0, target.1);
            }
            fixed_clone.set_size_request(final_size_request.0, final_size_request.1);
            animating_cell.set(false);
            glib::ControlFlow::Break
        } else {
            let progress = elapsed as f64 / duration_ms as f64;
            let eased = ease_out_cubic(progress);
            for (widget, start_pos, target) in &moves {
                let (cx, cy) = interpolate_point(*start_pos, *target, eased);
                fixed_clone.move_(widget, cx, cy);
            }
            glib::ControlFlow::Continue
        }
    });
}

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
fn exact_drop_position(
    drop_x: f64,
    drop_y: f64,
    grab_offset: (f64, f64),
) -> (i32, i32) {
    (
        (drop_x - grab_offset.0).round().max(0.0) as i32,
        (drop_y - grab_offset.1).round().max(0.0) as i32,
    )
}

#[inline]
fn resolve_drop_coords(
    x: f64,
    y: f64,
    offset_idx: f64,
    grab_offset: (f64, f64),
    mode: LayoutMode,
    cell_w: i32,
    cell_h: i32,
) -> (i32, i32) {
    let (raw_x, raw_y) = exact_drop_position(x + offset_idx, y + offset_idx, grab_offset);
    match mode {
        LayoutMode::SnapToGrid => snap_to_grid_cell(raw_x, raw_y, cell_w, cell_h),
        LayoutMode::Auto | LayoutMode::Freeform => (raw_x, raw_y),
    }
}

#[inline]
pub(crate) fn point_in_icon_bounds(
    x: f64,
    y: f64,
    icon_x: f64,
    icon_y: f64,
    icon_w: f64,
    icon_h: f64,
) -> bool {
    x >= icon_x && x <= icon_x + icon_w && y >= icon_y && y <= icon_y + icon_h
}

fn select_single_spatial_item(
    widgets: &HashMap<String, Rc<RefCell<ItemWidgetData>>>,
    selected_items: &RefCell<HashSet<String>>,
    target_name: &str,
    on_selection_changed: &SelectionCallback,
) {
    for (name, data) in widgets.iter() {
        let w = &data.borrow().widget;
        if name == target_name {
            w.add_css_class("selected");
        } else {
            w.remove_css_class("selected");
        }
    }
    let mut sel = selected_items.borrow_mut();
    sel.clear();
    sel.insert(target_name.to_string());
    drop(sel);
    on_selection_changed(1);
}

fn is_point_over_spatial_icon(
    fixed: &Fixed,
    widgets: &HashMap<String, Rc<RefCell<ItemWidgetData>>>,
    x: f64,
    y: f64,
) -> bool {
    if let Some(mut cur) = fixed.pick(x, y, gtk4::PickFlags::DEFAULT) {
        let fixed_widget: gtk4::Widget = fixed.clone().upcast();
        while cur != fixed_widget {
            if cur.has_css_class("branches-icon-item") {
                return true;
            }
            let Some(parent) = cur.parent() else {
                break;
            };
            cur = parent;
        }
    }
    widgets.values().any(|data_rc| {
        let d = data_rc.borrow();
        let (px, py) = fixed.child_position(&d.widget);
        if px < 0.0 || py < 0.0 {
            return false;
        }
        let w = d.widget.width().max(d.widget.width_request()).max(1) as f64;
        let h = d.widget.height().max(d.widget.height_request()).max(1) as f64;
        point_in_icon_bounds(x, y, px, py, w, h)
    })
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
    anim_gen: Rc<Cell<u64>>,
    is_animating: Rc<Cell<bool>>,
    active_drag_offset: Rc<RefCell<Option<(PathBuf, (f64, f64))>>>,
    item_order: Rc<RefCell<Vec<String>>>,
    item_widgets: Rc<RefCell<HashMap<String, Rc<RefCell<ItemWidgetData>>>>>,
    selected_items: Rc<RefCell<HashSet<String>>>,
    pending_select_names: Rc<RefCell<HashSet<String>>>,
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
        let anim_gen = Rc::new(Cell::new(0));
        let is_animating = Rc::new(Cell::new(false));
        let active_drag_offset = Rc::new(RefCell::new(None));
        let item_order = Rc::new(RefCell::new(Vec::new()));
        let item_widgets = Rc::new(RefCell::new(HashMap::new()));
        let selected_items = Rc::new(RefCell::new(HashSet::new()));
        let pending_select_names = Rc::new(RefCell::new(HashSet::new()));

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
            anim_gen,
            is_animating,
            active_drag_offset,
            item_order,
            item_widgets,
            selected_items,
            pending_select_names,
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
            let prev_cols = weak_helper.last_auto_cols.get();
            if new_cols != prev_cols {
                weak_helper.last_auto_cols.set(new_cols);
                weak_helper.relayout_auto_grid_with_width(new_width, prev_cols > 0);
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
            anim_gen: self.anim_gen.clone(),
            is_animating: self.is_animating.clone(),
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
            self.make_weak().relayout_auto_grid_with_width(viewport_w, true);
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
        let mut moves = Vec::new();
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
                let target_x = MARGIN_LEFT + col * cell_w;
                let target_y = MARGIN_TOP + row * cell_h;
                let start_pos = current_canvas_pos(
                    &self.fixed_canvas,
                    &d.widget,
                    (px, py),
                    self.is_animating.get(),
                );
                px = target_x;
                py = target_y;
                d.pos.set((px, py));
                if (start_pos.0 - px as f64).abs() >= 1.0 || (start_pos.1 - py as f64).abs() >= 1.0 {
                    moves.push((d.widget.clone(), start_pos, (px as f64, py as f64)));
                }
            }

            max_x = max_x.max(px + cell_w);
            max_y = max_y.max(py + cell_h);
        }

        let min_w = if max_x <= viewport_w { 1 } else { max_x + 24 };
        animate_grid_moves(
            &self.fixed_canvas,
            &self.anim_gen,
            &self.is_animating,
            moves,
            (min_w, max_y + 80),
            GRID_REORDER_ANIM_MS,
        );
    }

    pub fn widget(&self) -> &ScrolledWindow {
        &self.container
    }

    pub fn set_items(&self, items: &[FileItem]) {
        // Cancelar cualquier animación de cuadrícula previa al cambiar/recargar carpeta
        self.anim_gen.set(self.anim_gen.get().wrapping_add(1));
        self.is_animating.set(false);
        *self.active_drag_offset.borrow_mut() = None;

        // Limpiar widgets anteriores
        while let Some(child) = self.fixed_canvas.first_child() {
            self.fixed_canvas.remove(&child);
        }
        self.item_widgets.borrow_mut().clear();
        self.item_order.borrow_mut().clear();
        self.selected_items.borrow_mut().clear();
        let pending_select = std::mem::take(&mut *self.pending_select_names.borrow_mut());
        if pending_select.is_empty() {
            (self.on_selection_changed)(0);
        }

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
            if pending_select.contains(&item.name) {
                widget.add_css_class("selected");
                self.selected_items.borrow_mut().insert(item.name.clone());
            }
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

        let sel_count = self.selected_items.borrow().len();
        if sel_count > 0 {
            (self.on_selection_changed)(sel_count);
        }

        // Guardar en segundo plano las posiciones nuevas generadas (cuando no está en Auto)
        if !new_positions_to_save.is_empty() {
            if let Ok(mut conn) = db::open_db() {
                let _ = db::save_all_positions(&mut conn, &folder, &new_positions_to_save);
            }
        }

        // En modo Auto (o cuando todo cabe en el ancho visible) dejamos min_width=1 para evitar scroll horizontal espurio
        let min_w = if current_mode == LayoutMode::Auto || (viewport_w > 0 && max_x <= viewport_w) {
            1
        } else {
            max_x + 24
        };
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
        let widgets_for_click = self.item_widgets.clone();
        let name_for_click = item.name.clone();
        let vbox_for_click = vbox.clone();
        let on_sel_cb = self.on_selection_changed.clone();

        click_gesture.connect_pressed(move |g, n_press, x, y| {
            let button = g.current_button();

            if button == gdk::BUTTON_PRIMARY {
                if n_press == 1 {
                    select_single_spatial_item(
                        &widgets_for_click.borrow(),
                        &sel_items,
                        &name_for_click,
                        &on_sel_cb,
                    );
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

        // 2. Drag & Drop entre ventanas/paneles (DragSource Wayland con icono mapeado al punto exacto de agarre)
        let drag_source = gtk4::DragSource::new();
        drag_source.set_actions(gdk::DragAction::MOVE | gdk::DragAction::COPY);
        let path_for_drag = item.path.clone();

        let vbox_prep = vbox.clone();
        let active_drag_prep = self.active_drag_offset.clone();
        let item_path_drag = item.path.clone();
        let widgets_prep = self.item_widgets.clone();
        let sel_items_prep = self.selected_items.clone();
        let name_prep = item.name.clone();
        let on_sel_prep = self.on_selection_changed.clone();
        drag_source.connect_prepare(move |source, click_x, click_y| {
            let rounded_offset = (click_x.round(), click_y.round());
            *active_drag_prep.borrow_mut() = Some((item_path_drag.clone(), rounded_offset));
            malus_de_common::write_drag_anchor(&item_path_drag, rounded_offset.0, rounded_offset.1);
            select_single_spatial_item(
                &widgets_prep.borrow(),
                &sel_items_prep,
                &name_prep,
                &on_sel_prep,
            );
            let paintable = gtk4::WidgetPaintable::new(Some(&vbox_prep));
            source.set_icon(Some(&paintable), rounded_offset.0 as i32, rounded_offset.1 as i32);

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
        let active_drag_end = self.active_drag_offset.clone();
        let path_end = item.path.clone();
        let cur_folder_end = self.current_folder.clone();
        let on_nav_end = self.on_navigate.clone();
        drag_source.connect_drag_end(move |_, drag, _| {
            *active_drag_end.borrow_mut() = None;
            let folder = cur_folder_end.borrow().clone();
            if !path_end.exists() {
                if vbox_end.parent().is_some() {
                    fixed_end.remove(&vbox_end);
                }
                on_nav_end(folder);
                return;
            }
            let (cx, _cy) = fixed_end.child_position(&vbox_end);
            if cx < 0.0 {
                let (px, py) = pos_end.get();
                fixed_end.move_(&vbox_end, px as f64, py as f64);
            }
            let _ = crate::views::refresh_if_moved_out(drag, &path_end, &folder, &on_nav_end);
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
            if is_point_over_spatial_icon(&fixed, &widgets.borrow(), x, y) {
                return;
            }
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
        let container_drop_ref = self.container.clone();
        let fixed_drop_canvas = self.fixed_canvas.clone();
        let active_drag_drop_ref = self.active_drag_offset.clone();
        let widgets_drop_map = self.item_widgets.clone();
        let pending_select_drop_ref = self.pending_select_names.clone();
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
                let mut dropped_positions: HashMap<String, (i32, i32)> = HashMap::new();
                let mut external_selected_names: Vec<String> = Vec::new();

                for (index, file) in file_list.files().into_iter().enumerate() {
                    if let Some(src) = file.path() {
                        let is_internal = src.parent().map(|p| p == cur_folder).unwrap_or(false);
                        let offset_idx = (index % 4) as f64 * 24.0;
                        if is_internal {
                            // Colocar en la posición exacta donde se soltó respetando el punto de agarre (sin saltos ni rebotes)
                            if let Some(fname) = src.file_name() {
                                let fname_str = fname.to_string_lossy().to_string();
                                let grab_offset = active_drag_drop_ref
                                    .borrow()
                                    .as_ref()
                                    .filter(|(dragged_path, _)| dragged_path == &src)
                                    .map(|(_, offset)| *offset)
                                    .unwrap_or((
                                        metrics.box_width as f64 / 2.0,
                                        metrics.icon_size as f64 / 2.0,
                                    ));
                                let (final_x, final_y) = resolve_drop_coords(
                                    x,
                                    y,
                                    offset_idx,
                                    grab_offset,
                                    mode,
                                    cell_w,
                                    cell_h,
                                );

                                if let Some(item_data) = widgets_drop_map.borrow().get(&fname_str) {
                                    let d = item_data.borrow();
                                    d.pos.set((final_x, final_y));
                                    fixed_drop_canvas.move_(&d.widget, final_x as f64, final_y as f64);
                                }
                                dropped_positions.insert(fname_str, (final_x, final_y));

                                let mut max_x = 400;
                                let mut max_y = 300;
                                for data_rc in widgets_drop_map.borrow().values() {
                                    let (px, py) = data_rc.borrow().pos.get();
                                    max_x = max_x.max(px + cell_w);
                                    max_y = max_y.max(py + cell_h);
                                }
                                let vw = container_drop_ref.hadjustment().page_size().floor() as i32;
                                let min_w = if vw > 0 && max_x <= vw { 1 } else { max_x + 24 };
                                fixed_drop_canvas.set_size_request(min_w, max_y + 80);
                                moved_internal = true;
                            }
                        } else if let Some(fname) = src.file_name() {
                            let fname_str = fname.to_string_lossy().to_string();
                            let dest = cur_folder.join(fname);
                            if dest != src {
                                let grab_offset = malus_de_common::read_drag_anchor(&src)
                                    .unwrap_or((
                                        metrics.box_width as f64 / 2.0,
                                        metrics.icon_size as f64 / 2.0,
                                    ));
                                let (final_x, final_y) = resolve_drop_coords(
                                    x,
                                    y,
                                    offset_idx,
                                    grab_offset,
                                    mode,
                                    cell_w,
                                    cell_h,
                                );
                                let dest_file = gio::File::for_path(&dest);
                                if file
                                    .move_(
                                        &dest_file,
                                        gio::FileCopyFlags::NONE,
                                        gio::Cancellable::NONE,
                                        None,
                                    )
                                    .is_ok()
                                    || std::fs::rename(&src, &dest).is_ok()
                                {
                                    dropped_positions.insert(fname_str.clone(), (final_x, final_y));
                                    external_selected_names.push(fname_str);
                                    moved_external = true;
                                }
                            }
                        }
                    }
                }
                if !dropped_positions.is_empty() {
                    if mode == LayoutMode::Auto {
                        *layout_drop_ref.borrow_mut() = LayoutMode::Freeform;
                        let mut all_positions = HashMap::new();
                        for (name, data_rc) in widgets_drop_map.borrow().iter() {
                            all_positions.insert(name.clone(), data_rc.borrow().pos.get());
                        }
                        for (name, pos) in dropped_positions {
                            all_positions.insert(name, pos);
                        }
                        if let Ok(mut conn) = db::open_db() {
                            let _ = db::set_layout_mode(&conn, &cur_folder, LayoutMode::Freeform);
                            let _ = db::save_all_positions(&mut conn, &cur_folder, &all_positions);
                        }
                    } else if let Ok(conn) = db::open_db() {
                        for (name, (fx, fy)) in dropped_positions {
                            let _ = db::save_position(&conn, &cur_folder, &name, fx, fy);
                        }
                    }
                }
                if moved_external {
                    *pending_select_drop_ref.borrow_mut() =
                        external_selected_names.into_iter().collect();
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
    anim_gen: Rc<Cell<u64>>,
    is_animating: Rc<Cell<bool>>,
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

    fn relayout_auto_grid_with_width(&self, available_width: i32, animate: bool) {
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
        let mut moves = Vec::new();
        let animating = self.is_animating.get();

        for name in order.iter() {
            if let Some(data_rc) = widgets.get(name) {
                let col = idx % cols;
                let row = idx / cols;
                let x = MARGIN_LEFT + col * cell_w;
                let y = MARGIN_TOP + row * cell_h;

                let d = data_rc.borrow();
                let start_pos = current_canvas_pos(&self.fixed, &d.widget, d.pos.get(), animating);
                d.pos.set((x, y));
                if !animate {
                    self.fixed.move_(&d.widget, x as f64, y as f64);
                } else if (start_pos.0 - x as f64).abs() >= 1.0 || (start_pos.1 - y as f64).abs() >= 1.0 {
                    moves.push((d.widget.clone(), start_pos, (x as f64, y as f64)));
                }
                max_y = max_y.max(y + cell_h);
                idx += 1;
            }
        }

        if animate {
            animate_grid_moves(
                &self.fixed,
                &self.anim_gen,
                &self.is_animating,
                moves,
                (1, max_y + 80),
                GRID_REORDER_ANIM_MS,
            );
        } else {
            self.anim_gen.set(self.anim_gen.get().wrapping_add(1));
            self.is_animating.set(false);
            self.fixed.set_size_request(1, max_y + 80);
        }
    }

    fn snap_existing_to_grid(&self) {
        let metrics = IconGridMetrics::from_icon_size(
            self.active_icon_size.get(),
            self.grid_spacing.get(),
        );
        let cell_w = metrics.cell_width();
        let cell_h = metrics.cell_height();
        let viewport_w = self.viewport_width();
        let cols = columns_for_width(viewport_w, cell_w);

        let mut ordered: Vec<_> = self.item_widgets.borrow().values().cloned().collect();
        ordered.sort_by_key(|d| {
            let p = d.borrow().pos.get();
            (p.1, p.0)
        });

        let mut used_slots: HashSet<(i32, i32)> = HashSet::new();
        let mut positions = HashMap::new();
        let mut moves = Vec::new();
        let mut max_x = 400;
        let mut max_y = 300;
        let animating = self.is_animating.get();

        for data_rc in ordered {
            let d = data_rc.borrow();
            let fallback_pos = d.pos.get();
            let start_pos = current_canvas_pos(&self.fixed, &d.widget, fallback_pos, animating);
            let mut col = ((fallback_pos.0 - MARGIN_LEFT + cell_w / 2) / cell_w).max(0);
            let mut row = ((fallback_pos.1 - MARGIN_TOP + cell_h / 2) / cell_h).max(0);
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
            if (start_pos.0 - x as f64).abs() >= 1.0 || (start_pos.1 - y as f64).abs() >= 1.0 {
                moves.push((d.widget.clone(), start_pos, (x as f64, y as f64)));
            }
            positions.insert(d.item.name.clone(), (x, y));
            max_x = max_x.max(x + cell_w);
            max_y = max_y.max(y + cell_h);
        }

        let min_w = if max_x <= viewport_w { 1 } else { max_x + 24 };
        animate_grid_moves(
            &self.fixed,
            &self.anim_gen,
            &self.is_animating,
            moves,
            (min_w, max_y + 80),
            GRID_REORDER_ANIM_MS,
        );
        let folder = self.current_folder.borrow().clone();
        if let Ok(mut conn) = db::open_db() {
            let _ = db::save_all_positions(&mut conn, &folder, &positions);
        }
    }

    fn rearrange(&self, by_name: bool) {
        let mut list: Vec<_> = self.item_widgets.borrow().values().cloned().collect();
        if by_name {
            // Igual que fs_backend::load_directory: carpetas primero, luego orden alfabético
            list.sort_by(|a, b| {
                let a_ref = a.borrow();
                let b_ref = b.borrow();
                b_ref
                    .item
                    .is_dir
                    .cmp(&a_ref.item.is_dir)
                    .then_with(|| a_ref.item.name.to_lowercase().cmp(&b_ref.item.name.to_lowercase()))
            });
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
        let mut moves = Vec::new();
        let folder = self.current_folder.borrow().clone();
        let mut max_x = 400;
        let mut max_y = 300;
        let animating = self.is_animating.get();

        for (idx, data_rc) in list.iter().enumerate() {
            let col = (idx as i32) % cols;
            let row = (idx as i32) / cols;
            let x = MARGIN_LEFT + col * cell_w;
            let y = MARGIN_TOP + row * cell_h;

            let d = data_rc.borrow();
            let start_pos = current_canvas_pos(&self.fixed, &d.widget, d.pos.get(), animating);
            d.pos.set((x, y));
            if (start_pos.0 - x as f64).abs() >= 1.0 || (start_pos.1 - y as f64).abs() >= 1.0 {
                moves.push((d.widget.clone(), start_pos, (x as f64, y as f64)));
            }
            positions.insert(d.item.name.clone(), (x, y));
            new_order.push(d.item.name.clone());
            max_x = max_x.max(x + cell_w);
            max_y = max_y.max(y + cell_h);
        }

        *self.item_order.borrow_mut() = new_order;
        let mode = *self.layout_mode.borrow();
        let min_w = if mode == LayoutMode::Auto || max_x <= viewport_w {
            1
        } else {
            max_x + 24
        };
        animate_grid_moves(
            &self.fixed,
            &self.anim_gen,
            &self.is_animating,
            moves,
            (min_w, max_y + 80),
            GRID_REORDER_ANIM_MS,
        );

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
        w_auto.relayout_auto_grid_with_width(vw, true);
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
    use super::{
        columns_for_width, ease_out_cubic, exact_drop_position, interpolate_point,
        point_in_icon_bounds, snap_to_grid_cell, MARGIN_LEFT, MARGIN_TOP,
    };
    use malus_de_common::config::BranchesConfig;

    #[test]
    fn exact_drop_position_preserves_pointer_grab_offset() {
        // Si el usuario tomó el icono en (42, 70) respecto a su esquina superior izquierda
        // y lo soltó con el cursor en (342, 270), la esquina del icono queda en (300, 200).
        assert_eq!(exact_drop_position(342.0, 270.0, (42.0, 70.0)), (300, 200));
        assert_eq!(exact_drop_position(10.0, 10.0, (42.0, 70.0)), (0, 0));
    }

    #[test]
    fn ease_out_cubic_and_interpolation_follow_macos_curve() {
        assert_eq!(ease_out_cubic(-0.5), 0.0);
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert!((ease_out_cubic(0.5) - 0.875).abs() < 1e-6);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert_eq!(ease_out_cubic(1.5), 1.0);

        let mid = interpolate_point((24.0, 24.0), (140.0, 144.0), ease_out_cubic(0.5));
        assert!((mid.0 - 125.5).abs() < 1e-6);
        assert!((mid.1 - 129.0).abs() < 1e-6);
    }

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

    #[test]
    fn canvas_hit_test_distinguishes_icons_from_empty_space() {
        assert!(point_in_icon_bounds(50.0, 60.0, 24.0, 24.0, 108.0, 112.0));
        assert!(!point_in_icon_bounds(150.0, 60.0, 24.0, 24.0, 108.0, 112.0));
        assert!(!point_in_icon_bounds(10.0, 60.0, 24.0, 24.0, 108.0, 112.0));
    }
}
