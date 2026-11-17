use crate::db::{self, LayoutMode};
use crate::fs_backend::{open_file, FileItem};
use crate::views::{show_item_context_menu, NavigateCallback, SelectionCallback};
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Fixed, GestureClick, GestureDrag, Image, Label,
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

#[inline]
pub(crate) fn rects_intersect(
    ax1: f64,
    ay1: f64,
    ax2: f64,
    ay2: f64,
    bx1: f64,
    by1: f64,
    bx2: f64,
    by2: f64,
) -> bool {
    ax1 <= bx2 && ax2 >= bx1 && ay1 <= by2 && ay2 >= by1
}

#[inline]
pub(crate) fn has_multi_select_modifier(modifiers: gdk::ModifierType) -> bool {
    modifiers.intersects(
        gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::SUPER_MASK
            | gdk::ModifierType::META_MASK,
    )
}

#[inline]
pub(crate) fn has_additive_select_modifier(modifiers: gdk::ModifierType) -> bool {
    modifiers.intersects(
        gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::SUPER_MASK
            | gdk::ModifierType::META_MASK
            | gdk::ModifierType::SHIFT_MASK,
    )
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

fn toggle_spatial_item_selection(
    widgets: &HashMap<String, Rc<RefCell<ItemWidgetData>>>,
    selected_items: &RefCell<HashSet<String>>,
    target_name: &str,
    on_selection_changed: &SelectionCallback,
) {
    let mut sel = selected_items.borrow_mut();
    let now_selected = if sel.remove(target_name) {
        false
    } else {
        sel.insert(target_name.to_string());
        true
    };
    if let Some(data) = widgets.get(target_name) {
        let w = &data.borrow().widget;
        if now_selected {
            w.add_css_class("selected");
        } else {
            w.remove_css_class("selected");
        }
    }
    let count = sel.len();
    drop(sel);
    on_selection_changed(count);
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

#[derive(Clone)]
pub struct IconView {
    container: ScrolledWindow,
    fixed_canvas: Fixed,
    current_folder: Rc<RefCell<PathBuf>>,
    can_create_here: Rc<Cell<bool>>,
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
            .focusable(true)
            .css_classes(["branches-icon-canvas"])
            .build();

        let container = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vscrollbar_policy(PolicyType::Automatic)
            .css_classes(["branches-icon-view"])
            .child(&fixed_canvas)
            .build();

        let current_folder = Rc::new(RefCell::new(PathBuf::from("/")));
        let can_create_here = Rc::new(Cell::new(false));
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
            can_create_here,
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

    pub fn set_current_folder(&self, folder: Option<&std::path::Path>) {
        if let Some(dir) = folder {
            *self.current_folder.borrow_mut() = dir.to_path_buf();
            self.can_create_here
                .set(crate::views::can_create_in_folder(dir));
        } else {
            self.can_create_here.set(false);
        }
    }

    pub fn queue_select_name(&self, name: &str) {
        self.pending_select_names.borrow_mut().insert(name.to_string());
    }

    /// Navega entre iconos de la rejilla con las teclas de dirección (`←`, `→`, `↑`, `↓`).
    pub fn navigate_by_arrow(&self, key: gdk::Key) -> bool {
        let widgets = self.item_widgets.borrow();
        let order = self.item_order.borrow();
        if order.is_empty() || widgets.is_empty() {
            return false;
        }

        let current_name = {
            let sel = self.selected_items.borrow();
            order
                .iter()
                .find(|name| sel.contains(*name))
                .or_else(|| sel.iter().next())
                .cloned()
        };

        let target_name = if let Some(ref cur) = current_name {
            if let Some(cur_data) = widgets.get(cur) {
                let (cx, cy) = cur_data.borrow().pos.get();
                let metrics = self.current_metrics();
                let cell_h = metrics.cell_height().max(1);
                let row_tolerance = (cell_h / 2).max(16);

                match key {
                    gdk::Key::Right | gdk::Key::Left => {
                        let dir = if key == gdk::Key::Right { 1 } else { -1 };
                        // 1. Buscar vecino más cercano en la misma fila visual
                        let same_row = widgets
                            .iter()
                            .filter(|(name, _)| *name != cur)
                            .filter_map(|(name, d)| {
                                let (px, py) = d.borrow().pos.get();
                                let dx = (px - cx) * dir;
                                let dy = (py - cy).abs();
                                if dx > 0 && dy <= row_tolerance {
                                    Some((dx, dy, name.clone()))
                                } else {
                                    None
                                }
                            })
                            .min_by_key(|(dx, dy, _)| (*dx, *dy))
                            .map(|(_, _, n)| n);

                        same_row.or_else(|| {
                            // 2. Si está al borde de la fila, avanzar/retroceder en orden visual (y, x)
                            let mut sorted: Vec<(i32, i32, String)> = widgets
                                .iter()
                                .map(|(n, d)| {
                                    let (px, py) = d.borrow().pos.get();
                                    let row_bucket = (py - MARGIN_TOP + row_tolerance / 2)
                                        .div_euclid(row_tolerance.max(1));
                                    (row_bucket, px, n.clone())
                                })
                                .collect();
                            sorted.sort_by(|a, b| {
                                a.0.cmp(&b.0)
                                    .then_with(|| a.1.cmp(&b.1))
                                    .then_with(|| a.2.cmp(&b.2))
                            });
                            let pos_idx = sorted.iter().position(|(_, _, n)| n == cur)?;
                            let next_idx = if dir > 0 {
                                pos_idx.checked_add(1)?
                            } else {
                                pos_idx.checked_sub(1)?
                            };
                            sorted.get(next_idx).map(|(_, _, n)| n.clone())
                        })
                    }
                    gdk::Key::Down | gdk::Key::Up => {
                        let dir = if key == gdk::Key::Down { 1 } else { -1 };
                        widgets
                            .iter()
                            .filter(|(name, _)| *name != cur)
                            .filter_map(|(name, d)| {
                                let (px, py) = d.borrow().pos.get();
                                let dy = (py - cy) * dir;
                                let dx = (px - cx).abs();
                                if dy > row_tolerance / 2 {
                                    // Priorizar la fila inmediata siguiente/anterior y la columna más alineada
                                    let row_steps = (dy + cell_h / 2) / cell_h;
                                    Some((row_steps.max(1), dx, dy, name.clone()))
                                } else {
                                    None
                                }
                            })
                            .min_by_key(|(row_steps, dx, dy, _)| (*row_steps, *dx, *dy))
                            .map(|(_, _, _, n)| n)
                    }
                    _ => None,
                }
                .unwrap_or_else(|| cur.clone())
            } else {
                order[0].clone()
            }
        } else {
            order[0].clone()
        };

        select_single_spatial_item(
            &widgets,
            &self.selected_items,
            &target_name,
            &self.on_selection_changed,
        );

        // Asegurar que el icono seleccionado sea visible dentro del ScrolledWindow y mantener el foco en el lienzo
        if let Some(data_rc) = widgets.get(&target_name) {
            let d = data_rc.borrow();
            let (_, py) = d.pos.get();
            let cell_h = self.current_metrics().cell_height() as f64;
            let vadj = self.container.vadjustment();
            let top = vadj.value();
            let page = vadj.page_size();
            if page > 0.0 {
                let item_top = (py - MARGIN_TOP).max(0) as f64;
                let item_bottom = py as f64 + cell_h + 16.0;
                if item_top < top {
                    vadj.set_value(item_top);
                } else if item_bottom > top + page {
                    vadj.set_value((item_bottom - page).max(0.0));
                }
            }
        }
        self.fixed_canvas.grab_focus();
        true
    }

    pub fn get_selected_item_and_widget(&self, items: &[FileItem]) -> Option<(FileItem, gtk4::Widget)> {
        let sel = self.selected_items.borrow();
        if sel.is_empty() {
            return None;
        }
        let order = self.item_order.borrow();
        let selected_name = order
            .iter()
            .find(|name| sel.contains(*name))
            .or_else(|| sel.iter().next())?;
        let item = items.iter().find(|it| &it.name == selected_name)?.clone();
        let widget = self
            .item_widgets
            .borrow()
            .get(selected_name)
            .map(|data| data.borrow().widget.clone().upcast())?;
        Some((item, widget))
    }

    pub fn get_selected_items(&self, items: &[FileItem]) -> Vec<FileItem> {
        let sel = self.selected_items.borrow();
        if sel.is_empty() {
            return Vec::new();
        }
        items
            .iter()
            .filter(|it| sel.contains(&it.name))
            .cloned()
            .collect()
    }

    pub fn select_all(&self) {
        let widgets = self.item_widgets.borrow();
        let mut sel = self.selected_items.borrow_mut();
        sel.clear();
        for (name, data) in widgets.iter() {
            data.borrow().widget.add_css_class("selected");
            sel.insert(name.clone());
        }
        let count = sel.len();
        drop(sel);
        (self.on_selection_changed)(count);
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
            can_create_here: self.can_create_here.clone(),
            layout_mode: self.layout_mode.clone(),
            active_icon_size: self.active_icon_size.clone(),
            grid_spacing: self.grid_spacing.clone(),
            last_auto_cols: self.last_auto_cols.clone(),
            anim_gen: self.anim_gen.clone(),
            is_animating: self.is_animating.clone(),
            item_order: self.item_order.clone(),
            item_widgets: self.item_widgets.clone(),
            pending_select_names: self.pending_select_names.clone(),
            on_navigate: self.on_navigate.clone(),
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

        // Limpiar widgets anteriores (excepto el rectángulo de selección elástica si existe)
        let mut child = self.fixed_canvas.first_child();
        while let Some(c) = child {
            let next = c.next_sibling();
            if !c.has_css_class("branches-selection-marquee") {
                self.fixed_canvas.remove(&c);
            }
            child = next;
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

        // 1. Doble clic para abrir, segundo clic lento sobre etiqueta para renombrar (estilo Finder)
        // y Clic derecho para menú contextual
        let click_gesture = GestureClick::new();
        click_gesture.set_button(0);
        let path = item.path.clone();
        let is_dir = item.is_dir;
        let on_nav = self.on_navigate.clone();
        let sel_items = self.selected_items.clone();
        let widgets_for_click = self.item_widgets.clone();
        let name_for_click = item.name.clone();
        let vbox_for_click = vbox.clone();
        let label_for_click = label.clone();
        let on_sel_cb = self.on_selection_changed.clone();
        let pending_for_menu = self.pending_select_names.clone();
        let folder_for_menu = self.current_folder.clone();
        let fixed_for_click = self.fixed_canvas.clone();
        let pending_slow_rename: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
        let pending_slow_click = pending_slow_rename.clone();
        let icon_h_for_click = metrics.icon_size as f64;

        click_gesture.connect_pressed(move |g, n_press, x, y| {
            let button = g.current_button();
            if let Some(source_id) = pending_slow_click.take() {
                source_id.remove();
            }

            if button == gdk::BUTTON_PRIMARY {
                fixed_for_click.grab_focus();
                let modifiers = g.current_event_state();
                let is_multi_mod = has_multi_select_modifier(modifiers);
                if n_press == 1 {
                    if is_multi_mod {
                        g.set_state(gtk4::EventSequenceState::Claimed);
                        toggle_spatial_item_selection(
                            &widgets_for_click.borrow(),
                            &sel_items,
                            &name_for_click,
                            &on_sel_cb,
                        );
                        return;
                    }
                    let was_sole_selected = {
                        let sel = sel_items.borrow();
                        sel.len() == 1 && sel.contains(&name_for_click)
                    };
                    if !sel_items.borrow().contains(&name_for_click) || was_sole_selected {
                        select_single_spatial_item(
                            &widgets_for_click.borrow(),
                            &sel_items,
                            &name_for_click,
                            &on_sel_cb,
                        );
                    }
                    let clicked_on_label = label_for_click
                        .compute_bounds(&vbox_for_click)
                        .map(|b| y >= b.y() as f64 - 2.0)
                        .unwrap_or(y >= icon_h_for_click);
                    if was_sole_selected && clicked_on_label {
                        let vbox_ren = vbox_for_click.clone();
                        let path_ren = path.clone();
                        let pending_cb = pending_for_menu.clone();
                        let folder_cb = folder_for_menu.clone();
                        let on_nav_ren = on_nav.clone();
                        let slot = pending_slow_click.clone();
                        let id = glib::timeout_add_local_once(
                            std::time::Duration::from_millis(450),
                            move || {
                                slot.set(None);
                                let on_renamed: crate::views::RenameCallback =
                                    Rc::new(move |new_path| {
                                        if let Some(new_name) = new_path.file_name() {
                                            pending_cb
                                                .borrow_mut()
                                                .insert(new_name.to_string_lossy().into_owned());
                                        }
                                        let reload_dir = folder_cb.borrow().clone();
                                        on_nav_ren(reload_dir);
                                    });
                                crate::views::show_rename_popover(
                                    &vbox_ren,
                                    path_ren,
                                    is_dir,
                                    None,
                                    Some(on_renamed),
                                );
                            },
                        );
                        pending_slow_click.set(Some(id));
                    }
                } else if n_press >= 2 {
                    g.set_state(gtk4::EventSequenceState::Claimed);
                    if is_dir {
                        on_nav(path.clone());
                    } else {
                        let _ = open_file(&path);
                    }
                }
            } else if button == gdk::BUTTON_SECONDARY {
                g.set_state(gtk4::EventSequenceState::Claimed);
                if !sel_items.borrow().contains(&name_for_click) {
                    select_single_spatial_item(
                        &widgets_for_click.borrow(),
                        &sel_items,
                        &name_for_click,
                        &on_sel_cb,
                    );
                }
                let pending_cb = pending_for_menu.clone();
                let folder_cb = folder_for_menu.clone();
                let on_nav_rename = on_nav.clone();
                let on_renamed: crate::views::RenameCallback = Rc::new(move |new_path| {
                    if let Some(new_name) = new_path.file_name() {
                        pending_cb
                            .borrow_mut()
                            .insert(new_name.to_string_lossy().into_owned());
                    }
                    let reload_dir = folder_cb.borrow().clone();
                    on_nav_rename(reload_dir);
                });
                show_item_context_menu(
                    &vbox_for_click,
                    &path,
                    is_dir,
                    Some((x, y)),
                    Some(&on_nav),
                    Some(on_renamed),
                );
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
        let pos_prep = pos_cell.clone();
        let widgets_prep = self.item_widgets.clone();
        let order_prep = self.item_order.clone();
        let sel_items_prep = self.selected_items.clone();
        let name_prep = item.name.clone();
        let on_sel_prep = self.on_selection_changed.clone();
        drag_source.connect_prepare(move |source, click_x, click_y| {
            let rounded_offset = (click_x.round(), click_y.round());
            *active_drag_prep.borrow_mut() = Some((item_path_drag.clone(), rounded_offset));
            if !sel_items_prep.borrow().contains(&name_prep) {
                select_single_spatial_item(
                    &widgets_prep.borrow(),
                    &sel_items_prep,
                    &name_prep,
                    &on_sel_prep,
                );
            }

            let (p0x, p0y) = pos_prep.get();
            let cursor_canvas_x = p0x as f64 + rounded_offset.0;
            let cursor_canvas_y = p0y as f64 + rounded_offset.1;

            let (selected_paths, anchors, drag_widgets): (
                Vec<PathBuf>,
                Vec<(PathBuf, (f64, f64))>,
                Vec<(gtk4::Widget, (f64, f64))>,
            ) = {
                let sel = sel_items_prep.borrow();
                let widgets = widgets_prep.borrow();
                let mut paths = Vec::new();
                let mut anchor_list = Vec::new();
                let mut widget_list = Vec::new();
                for n in order_prep.borrow().iter().filter(|n| sel.contains(*n)) {
                    if let Some(d_rc) = widgets.get(n) {
                        let d = d_rc.borrow();
                        let (ix, iy) = d.pos.get();
                        let grab_rel = (cursor_canvas_x - ix as f64, cursor_canvas_y - iy as f64);
                        paths.push(d.item.path.clone());
                        anchor_list.push((d.item.path.clone(), grab_rel));
                        widget_list.push((d.widget.clone().upcast(), (ix as f64, iy as f64)));
                    }
                }
                (paths, anchor_list, widget_list)
            };

            if drag_widgets.len() > 1 {
                if let Some((multi_paintable, hot_x, hot_y)) =
                    crate::views::build_multi_widget_drag_paintable(
                        &drag_widgets,
                        cursor_canvas_x,
                        cursor_canvas_y,
                    )
                {
                    source.set_icon(Some(&multi_paintable), hot_x, hot_y);
                } else {
                    let paintable = gtk4::WidgetPaintable::new(Some(&vbox_prep));
                    source.set_icon(
                        Some(&paintable),
                        rounded_offset.0 as i32,
                        rounded_offset.1 as i32,
                    );
                }
            } else {
                let paintable = gtk4::WidgetPaintable::new(Some(&vbox_prep));
                source.set_icon(
                    Some(&paintable),
                    rounded_offset.0 as i32,
                    rounded_offset.1 as i32,
                );
            }

            if anchors.is_empty() {
                malus_de_common::write_drag_anchor(
                    &item_path_drag,
                    rounded_offset.0,
                    rounded_offset.1,
                );
                Some(crate::views::build_file_drag_content_provider(&path_for_drag))
            } else {
                malus_de_common::write_drag_anchors(&anchors);
                Some(crate::views::build_files_drag_content_provider(&selected_paths))
            }
        });

        let fixed_drag = self.fixed_canvas.clone();
        let vbox_drag = vbox.clone();
        let widgets_drag_begin = self.item_widgets.clone();
        let sel_drag_begin = self.selected_items.clone();
        let name_drag_begin = item.name.clone();
        let path_drag_begin = item.path.clone();
        let cancel_slow_drag = pending_slow_rename;
        drag_source.connect_drag_begin(move |_source, _drag| {
            if let Some(source_id) = cancel_slow_drag.take() {
                source_id.remove();
            }
            if malus_de_common::apps::is_application_desktop_file(&path_drag_begin) {
                return;
            }
            let sel = sel_drag_begin.borrow();
            if sel.contains(&name_drag_begin) && sel.len() > 1 {
                let widgets = widgets_drag_begin.borrow();
                for sel_name in sel.iter() {
                    if let Some(d_rc) = widgets.get(sel_name) {
                        fixed_drag.move_(&d_rc.borrow().widget, -5000.0, -5000.0);
                    }
                }
            } else {
                fixed_drag.move_(&vbox_drag, -5000.0, -5000.0);
            }
        });

        let fixed_end = self.fixed_canvas.clone();
        let vbox_end = vbox.clone();
        let pos_end = pos_cell;
        let widgets_drag_end = self.item_widgets.clone();
        let order_drag_end = self.item_order.clone();
        let sel_drag_end = self.selected_items.clone();
        let on_sel_drag_end = self.on_selection_changed.clone();
        let active_drag_end = self.active_drag_offset.clone();
        let path_end = item.path.clone();
        let cur_folder_end = self.current_folder.clone();
        drag_source.connect_drag_end(move |_, drag, _| {
            *active_drag_end.borrow_mut() = None;
            if malus_de_common::apps::is_application_desktop_file(&path_end) {
                return;
            }
            let folder = cur_folder_end.borrow().clone();
            let remove_missing_in_place = || {
                let missing_names: Vec<(String, GtkBox)> = {
                    let widgets = widgets_drag_end.borrow();
                    widgets
                        .iter()
                        .filter_map(|(name, d_rc)| {
                            let d = d_rc.borrow();
                            if !d.item.path.exists() {
                                Some((name.clone(), d.widget.clone()))
                            } else {
                                None
                            }
                        })
                        .collect()
                };
                if missing_names.is_empty() {
                    return false;
                }
                let missing_set: HashSet<String> =
                    missing_names.iter().map(|(n, _)| n.clone()).collect();
                for (name, widget) in missing_names {
                    if widget.parent().is_some() {
                        fixed_end.remove(&widget);
                    }
                    widgets_drag_end.borrow_mut().remove(&name);
                    sel_drag_end.borrow_mut().remove(&name);
                }
                order_drag_end
                    .borrow_mut()
                    .retain(|n| !missing_set.contains(n));
                let sel_count = sel_drag_end.borrow().len();
                on_sel_drag_end(sel_count);
                true
            };

            if !path_end.exists() {
                if vbox_end.parent().is_some() {
                    fixed_end.remove(&vbox_end);
                }
                remove_missing_in_place();
                return;
            }
            if drag.selected_action().contains(gdk::DragAction::MOVE) {
                let cur_folder_async = cur_folder_end.clone();
                let widgets_async = widgets_drag_end.clone();
                let order_async = order_drag_end.clone();
                let sel_async = sel_drag_end.clone();
                let on_sel_async = on_sel_drag_end.clone();
                let fixed_async = fixed_end.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || {
                    if *cur_folder_async.borrow() != folder {
                        return;
                    }
                    let missing_names: Vec<(String, GtkBox)> = {
                        let widgets = widgets_async.borrow();
                        widgets
                            .iter()
                            .filter_map(|(name, d_rc)| {
                                let d = d_rc.borrow();
                                if !d.item.path.exists() {
                                    Some((name.clone(), d.widget.clone()))
                                } else {
                                    None
                                }
                            })
                            .collect()
                    };
                    if !missing_names.is_empty() {
                        let missing_set: HashSet<String> =
                            missing_names.iter().map(|(n, _)| n.clone()).collect();
                        for (name, widget) in missing_names {
                            if widget.parent().is_some() {
                                fixed_async.remove(&widget);
                            }
                            widgets_async.borrow_mut().remove(&name);
                            sel_async.borrow_mut().remove(&name);
                        }
                        order_async
                            .borrow_mut()
                            .retain(|n| !missing_set.contains(n));
                        let sel_count = sel_async.borrow().len();
                        on_sel_async(sel_count);
                    }
                    for d_rc in widgets_async.borrow().values() {
                        let d = d_rc.borrow();
                        let (cx, _cy) = fixed_async.child_position(&d.widget);
                        if cx < 0.0 {
                            let (px, py) = d.pos.get();
                            fixed_async.move_(&d.widget, px as f64, py as f64);
                        }
                    }
                });
                return;
            }
            for d_rc in widgets_drag_end.borrow().values() {
                let d = d_rc.borrow();
                let (cx, _cy) = fixed_end.child_position(&d.widget);
                if cx < 0.0 {
                    let (px, py) = d.pos.get();
                    fixed_end.move_(&d.widget, px as f64, py as f64);
                }
            }
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
            let fixed_folder_drop = self.fixed_canvas.clone();
            let widgets_folder_drop = self.item_widgets.clone();
            let order_folder_drop = self.item_order.clone();
            let sel_folder_drop = self.selected_items.clone();
            let on_sel_folder_drop = self.on_selection_changed.clone();
            folder_drop.connect_drop(move |_, value, _, _| {
                vbox_drop.remove_css_class("folder-drop-hover");
                if let Ok(file_list) = value.get::<gdk::FileList>() {
                    if crate::operations::drop_files_into_folder(&file_list.files(), &dest_dir) {
                        let missing_names: Vec<(String, GtkBox)> = {
                            let widgets = widgets_folder_drop.borrow();
                            widgets
                                .iter()
                                .filter_map(|(name, d_rc)| {
                                    let d = d_rc.borrow();
                                    if !d.item.path.exists() {
                                        Some((name.clone(), d.widget.clone()))
                                    } else {
                                        None
                                    }
                                })
                                .collect()
                        };
                        if !missing_names.is_empty() {
                            let missing_set: HashSet<String> =
                                missing_names.iter().map(|(n, _)| n.clone()).collect();
                            for (name, widget) in missing_names {
                                if widget.parent().is_some() {
                                    fixed_folder_drop.remove(&widget);
                                }
                                widgets_folder_drop.borrow_mut().remove(&name);
                                sel_folder_drop.borrow_mut().remove(&name);
                            }
                            order_folder_drop
                                .borrow_mut()
                                .retain(|n| !missing_set.contains(n));
                            let sel_count = sel_folder_drop.borrow().len();
                            on_sel_folder_drop(sel_count);
                        }
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
                fixed.grab_focus();
                let modifiers = g.current_event_state();
                let is_additive = has_additive_select_modifier(modifiers);
                if !is_additive {
                    // Clic en fondo vacío: deseleccionar todo
                    for (_, data) in widgets.borrow().iter() {
                        data.borrow().widget.remove_css_class("selected");
                    }
                    sel_items.borrow_mut().clear();
                    sel_cb(0);
                }
            } else if button == gdk::BUTTON_SECONDARY {
                // Menú contextual del lienzo de fondo
                if let Some(ref w) = *weak_for_click.borrow() {
                    show_canvas_context_menu(&fixed, x, y, folder_ref.clone(), layout_ref.clone(), w);
                }
            }
        });

        self.fixed_canvas.add_controller(click);

        // Selección de área elástica (marquee / rubberband) arrastrando el cursor sobre el lienzo
        let marquee_box = GtkBox::builder()
            .visible(false)
            .can_target(false)
            .css_classes(["branches-selection-marquee"])
            .build();
        self.fixed_canvas.put(&marquee_box, 0.0, 0.0);

        let marquee_drag = GestureDrag::new();
        marquee_drag.set_button(gdk::BUTTON_PRIMARY);
        let marquee_start: Rc<Cell<Option<(f64, f64)>>> = Rc::new(Cell::new(None));
        let marquee_base_sel: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(HashSet::new()));

        {
            let fixed_m = self.fixed_canvas.clone();
            let widgets_m = self.item_widgets.clone();
            let sel_m = self.selected_items.clone();
            let start_m = marquee_start.clone();
            let base_m = marquee_base_sel.clone();
            let m_box = marquee_box.clone();
            marquee_drag.connect_drag_begin(move |g, x, y| {
                if is_point_over_spatial_icon(&fixed_m, &widgets_m.borrow(), x, y) {
                    start_m.set(None);
                    return;
                }
                start_m.set(Some((x, y)));
                let modifiers = g.current_event_state();
                let is_additive = has_additive_select_modifier(modifiers);
                if is_additive {
                    *base_m.borrow_mut() = sel_m.borrow().clone();
                } else {
                    base_m.borrow_mut().clear();
                }
                m_box.set_size_request(1, 1);
                fixed_m.move_(&m_box, x, y);
                m_box.set_visible(false);
            });
        }

        {
            let fixed_m = self.fixed_canvas.clone();
            let widgets_m = self.item_widgets.clone();
            let sel_m = self.selected_items.clone();
            let sel_cb_m = self.on_selection_changed.clone();
            let start_m = marquee_start.clone();
            let base_m = marquee_base_sel.clone();
            let m_box = marquee_box.clone();
            marquee_drag.connect_drag_update(move |_, offset_x, offset_y| {
                let Some((sx, sy)) = start_m.get() else {
                    return;
                };
                if offset_x.abs() < 3.0 && offset_y.abs() < 3.0 && !m_box.is_visible() {
                    return;
                }
                let rx1 = sx.min(sx + offset_x).max(0.0);
                let ry1 = sy.min(sy + offset_y).max(0.0);
                let rx2 = sx.max(sx + offset_x);
                let ry2 = sy.max(sy + offset_y);
                let rw = (rx2 - rx1).round().max(1.0) as i32;
                let rh = (ry2 - ry1).round().max(1.0) as i32;

                m_box.set_size_request(rw, rh);
                fixed_m.move_(&m_box, rx1, ry1);
                m_box.set_visible(true);

                let base = base_m.borrow();
                let widgets = widgets_m.borrow();
                let mut new_sel = base.clone();

                for (name, data_rc) in widgets.iter() {
                    let d = data_rc.borrow();
                    let (px, py) = fixed_m.child_position(&d.widget);
                    let iw = d.widget.width().max(d.widget.width_request()).max(1) as f64;
                    let ih = d.widget.height().max(d.widget.height_request()).max(1) as f64;
                    let hit = rects_intersect(rx1, ry1, rx2, ry2, px, py, px + iw, py + ih);
                    if hit {
                        if base.contains(name) {
                            new_sel.remove(name);
                        } else {
                            new_sel.insert(name.clone());
                        }
                    }
                }

                for (name, data_rc) in widgets.iter() {
                    let w = &data_rc.borrow().widget;
                    if new_sel.contains(name) {
                        w.add_css_class("selected");
                    } else {
                        w.remove_css_class("selected");
                    }
                }
                let count = new_sel.len();
                *sel_m.borrow_mut() = new_sel;
                sel_cb_m(count);
            });
        }

        {
            let start_m = marquee_start;
            let base_m = marquee_base_sel;
            let m_box = marquee_box;
            marquee_drag.connect_drag_end(move |_, _, _| {
                start_m.set(None);
                base_m.borrow_mut().clear();
                m_box.set_visible(false);
            });
        }

        self.fixed_canvas.add_controller(marquee_drag);

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
        let view_drop = self.clone();

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
                        let shared_anchor = malus_de_common::read_drag_anchor(&src);
                        let offset_idx = if shared_anchor.is_some() {
                            0.0
                        } else {
                            (index % 4) as f64 * 24.0
                        };
                        if is_internal {
                            // Colocar en la posición exacta donde se soltó respetando la disposición espacial entre elementos seleccionados
                            if let Some(fname) = src.file_name() {
                                let fname_str = fname.to_string_lossy().to_string();
                                let grab_offset = shared_anchor
                                    .or_else(|| {
                                        active_drag_drop_ref
                                            .borrow()
                                            .as_ref()
                                            .filter(|(dragged_path, _)| dragged_path == &src)
                                            .map(|(_, offset)| *offset)
                                    })
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
                                let grab_offset = shared_anchor.unwrap_or((
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
                                let transferred = if malus_de_common::apps::is_application_desktop_file(&src) {
                                    dest.symlink_metadata().is_err()
                                        && std::os::unix::fs::symlink(&src, &dest).is_ok()
                                } else {
                                    let dest_file = gio::File::for_path(&dest);
                                    file.move_(
                                        &dest_file,
                                        gio::FileCopyFlags::NONE,
                                        gio::Cancellable::NONE,
                                        None,
                                    )
                                    .is_ok()
                                        || std::fs::rename(&src, &dest).is_ok()
                                };
                                if transferred {
                                    dropped_positions.insert(fname_str.clone(), (final_x, final_y));
                                    external_selected_names.push(fname_str);
                                    moved_external = true;
                                }
                            }
                        }
                    }
                }
                let saved_drop_positions = dropped_positions.clone();
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
                    for d_rc in view_drop.item_widgets.borrow().values() {
                        d_rc.borrow().widget.remove_css_class("selected");
                    }
                    view_drop.selected_items.borrow_mut().clear();

                    for name in external_selected_names {
                        if let Some(&(fx, fy)) = saved_drop_positions.get(&name) {
                            let dest = cur_folder.join(&name);
                            if let Some(file_item) = FileItem::from_path(&dest) {
                                if !view_drop.item_widgets.borrow().contains_key(&name) {
                                    let pos_cell = Rc::new(Cell::new((fx, fy)));
                                    let (vbox, img) = view_drop.create_spatial_icon_widget(
                                        &file_item,
                                        pos_cell.clone(),
                                        metrics,
                                    );
                                    vbox.add_css_class("selected");
                                    view_drop.fixed_canvas.put(&vbox, fx as f64, fy as f64);
                                    view_drop.item_order.borrow_mut().push(name.clone());
                                    view_drop.item_widgets.borrow_mut().insert(
                                        name.clone(),
                                        Rc::new(RefCell::new(ItemWidgetData {
                                            widget: vbox,
                                            image: img,
                                            item: file_item,
                                            pos: pos_cell,
                                        })),
                                    );
                                }
                                view_drop.selected_items.borrow_mut().insert(name);
                            }
                        }
                    }
                    let mut max_x = 400;
                    let mut max_y = 300;
                    for data_rc in view_drop.item_widgets.borrow().values() {
                        let (px, py) = data_rc.borrow().pos.get();
                        max_x = max_x.max(px + cell_w);
                        max_y = max_y.max(py + cell_h);
                    }
                    let vw = container_drop_ref.hadjustment().page_size().floor() as i32;
                    let min_w = if vw > 0 && max_x <= vw { 1 } else { max_x + 24 };
                    view_drop.fixed_canvas.set_size_request(min_w, max_y + 80);
                    let sel_count = view_drop.selected_items.borrow().len();
                    (view_drop.on_selection_changed)(sel_count);
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
    can_create_here: Rc<Cell<bool>>,
    layout_mode: Rc<RefCell<LayoutMode>>,
    active_icon_size: Rc<Cell<i32>>,
    grid_spacing: Rc<Cell<i32>>,
    last_auto_cols: Rc<Cell<i32>>,
    anim_gen: Rc<Cell<u64>>,
    is_animating: Rc<Cell<bool>>,
    item_order: Rc<RefCell<Vec<String>>>,
    item_widgets: Rc<RefCell<HashMap<String, Rc<RefCell<ItemWidgetData>>>>>,
    pending_select_names: Rc<RefCell<HashSet<String>>>,
    on_navigate: NavigateCallback,
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

    let stack = gtk4::Stack::builder()
        .transition_type(gtk4::StackTransitionType::SlideLeftRight)
        .transition_duration(160)
        .vhomogeneous(false)
        .build();

    let vbox = GtkBox::new(Orientation::Vertical, 2);
    stack.add_named(&vbox, Some("main"));

    let target_dir = folder_ref.borrow().clone();
    if view_weak.can_create_here.get() && crate::views::can_create_in_folder(&target_dir) {
        let pending_create = view_weak.pending_select_names.clone();
        let nav_create = view_weak.on_navigate.clone();
        let layout_create = *layout_ref.borrow();
        let metrics = IconGridMetrics::from_icon_size(
            view_weak.active_icon_size.get(),
            view_weak.grid_spacing.get(),
        );
        let dir_create = target_dir.clone();
        let on_created: crate::views::RenameCallback = Rc::new(move |created_path| {
            if let Some(new_name) = created_path.file_name().map(|n| n.to_string_lossy().into_owned()) {
                if layout_create != LayoutMode::Auto {
                    let (px, py) = if layout_create == LayoutMode::SnapToGrid {
                        snap_to_grid_cell(
                            x.round() as i32,
                            y.round() as i32,
                            metrics.cell_width(),
                            metrics.cell_height(),
                        )
                    } else {
                        ((x.round() as i32).max(MARGIN_LEFT), (y.round() as i32).max(MARGIN_TOP))
                    };
                    if let Ok(conn) = db::open_db() {
                        let _ = db::save_position(&conn, &dir_create, &new_name, px, py);
                    }
                }
                pending_create.borrow_mut().insert(new_name);
            }
            nav_create(dir_create.clone());
        });
        crate::views::append_create_new_submenu(&popover, &vbox, &stack, target_dir, on_created);
        vbox.append(&gtk4::Separator::new(Orientation::Horizontal));
    }

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

    stack.set_visible_child_name("main");
    popover.set_child(Some(&stack));
    popover.popup();
}

#[cfg(test)]
mod tests {
    use super::{
        columns_for_width, ease_out_cubic, exact_drop_position, has_additive_select_modifier,
        has_multi_select_modifier, interpolate_point, point_in_icon_bounds, rects_intersect,
        snap_to_grid_cell, MARGIN_LEFT, MARGIN_TOP,
    };
    use gtk4::gdk;
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

        // Selección de área por rectángulo elástico (marquee)
        assert!(rects_intersect(10.0, 10.0, 80.0, 80.0, 24.0, 24.0, 132.0, 136.0));
        assert!(!rects_intersect(200.0, 200.0, 300.0, 300.0, 24.0, 24.0, 132.0, 136.0));

        // Modificadores individuales (Ctrl solo, Super solo, Shift solo)
        assert!(has_multi_select_modifier(gdk::ModifierType::CONTROL_MASK));
        assert!(has_multi_select_modifier(gdk::ModifierType::SUPER_MASK));
        assert!(!has_multi_select_modifier(gdk::ModifierType::empty()));
        assert!(has_additive_select_modifier(gdk::ModifierType::SHIFT_MASK));
    }
}
