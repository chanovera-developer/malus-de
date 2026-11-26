use crate::fs_backend::{format_modified, open_file, open_in_terminal, FileItem};
use crate::views::column_view::{
    format_dimensions, format_posix_permissions, format_preview_subtitle, is_preview_image_path,
    make_quick_action_button, read_image_dpi, rounded_rect_path, summarize_multi_selection,
};
use crate::views::{show_item_context_menu, NavigateCallback, SelectionCallback};
use gdk_pixbuf::{Pixbuf, PixbufRotation};
use gtk4::cairo::{self, Format, ImageSurface, LinearGradient};
use gtk4::gdk::{self, Key};
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, DrawingArea, Entry, EventControllerKey, EventControllerScroll,
    EventControllerScrollFlags, FlowBox, GestureClick, Image, Label, Orientation, PolicyType,
    Popover, ScrolledWindow, SelectionMode,
};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const GALLERY_INSPECTOR_WIDTH: i32 = 280;
const COVERFLOW_VISIBLE_RADIUS: i32 = 4;
const COVERFLOW_CACHE_KEEP_RADIUS: usize = 16;
const COVERFLOW_PERSPECTIVE_SLICES: usize = 24;
const COVERFLOW_ANIM_US: f64 = 280_000.0;
const FILMSTRIP_THUMB_W: i32 = 56;
const FILMSTRIP_THUMB_H: i32 = 44;

/// Geometría calculada de una tarjeta dentro del escenario Cover Flow 3D.
#[derive(Debug, Clone, Copy)]
struct CoverSlot {
    center_x: f64,
    center_y: f64,
    scale: f64,
    yaw_rad: f64,
    opacity: f64,
    shade_alpha: f64,
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Calcula posición, escala, rotación 3D (eje Y), opacidad y sombreado de profundidad
/// para una portada situada a `offset` posiciones respecto al índice central activo.
fn compute_cover_slot(
    offset: f64,
    stage_w: f64,
    stage_h: f64,
    card_w: f64,
    _card_h: f64,
) -> CoverSlot {
    let abs_off = offset.abs();
    let sign = if offset < 0.0 {
        -1.0
    } else if offset > 0.0 {
        1.0
    } else {
        0.0
    };

    let t_wing = smoothstep(0.0, 1.0, abs_off);
    let first_wing_dx = (card_w * 0.58)
        .max(stage_w * 0.25)
        .min(stage_w * 0.36);
    let extra_step_dx = (stage_w * 0.115).clamp(48.0, 108.0);
    let dx = if abs_off <= 1.0 {
        sign * first_wing_dx * abs_off
    } else {
        sign * (first_wing_dx + (abs_off - 1.0) * extra_step_dx)
    };

    let center_x = stage_w * 0.5 + dx;
    let center_y = stage_h * 0.49;
    let scale = (1.0 - 0.22 * t_wing - 0.045 * (abs_off - 1.0).max(0.0)).max(0.50);
    // Las portadas izquierdas giran hacia la derecha (+yaw) y las derechas hacia la izquierda (-yaw)
    let yaw_rad = -sign * t_wing * 0.90;
    let opacity = if abs_off <= 3.0 {
        1.0
    } else {
        (1.0 - (abs_off - 3.0) / 1.25).clamp(0.0, 1.0)
    };
    let shade_alpha = (t_wing * 0.15 + (abs_off - 1.0).max(0.0) * 0.06).min(0.38);

    CoverSlot {
        center_x,
        center_y,
        scale,
        yaw_rad,
        opacity,
        shade_alpha,
    }
}

struct CoverSurface {
    surface: ImageSurface,
    width: f64,
    height: f64,
}

pub struct GalleryView {
    container: GtkBox,
    coverflow_area: DrawingArea,
    empty_box: GtkBox,
    inspector_holder: GtkBox,
    carousel_box: GtkBox,
    carousel_scrolled: ScrolledWindow,
    items: RefCell<Vec<FileItem>>,
    cover_cache: RefCell<HashMap<usize, CoverSurface>>,
    hit_rects: RefCell<Vec<(usize, f64, f64, f64, f64)>>,
    scroll_pos: Cell<f64>,
    target_pos: Cell<f64>,
    anim_gen: Cell<u64>,
    scroll_accum: Cell<f64>,
    current_folder: RefCell<Option<PathBuf>>,
    pending_select_name: RefCell<Option<String>>,
    current_index: Cell<Option<usize>>,
    selected_indices: RefCell<BTreeSet<usize>>,
    selection_anchor: Cell<Option<usize>>,
    carousel_widgets: RefCell<Vec<GtkBox>>,
    on_navigate: NavigateCallback,
    on_selection_changed: SelectionCallback,
}

impl GalleryView {
    pub fn new(on_navigate: NavigateCallback, on_selection_changed: SelectionCallback) -> Rc<Self> {
        // Contenedor principal horizontal: izquierda (Cover Flow + filmstrip) | derecha (Inspector)
        let container = GtkBox::new(Orientation::Horizontal, 0);
        container.add_css_class("branches-gallery-view");
        container.set_focusable(true);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Columna izquierda: Escenario Cover Flow arriba + Filmstrip de miniaturas abajo
        let left_pane = GtkBox::new(Orientation::Vertical, 0);
        left_pane.add_css_class("branches-gallery-left-pane");
        left_pane.set_hexpand(true);
        left_pane.set_vexpand(true);

        let stage_overlay = gtk4::Overlay::new();
        stage_overlay.add_css_class("branches-gallery-canvas");
        stage_overlay.set_hexpand(true);
        stage_overlay.set_vexpand(true);

        let coverflow_area = DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .build();
        stage_overlay.set_child(Some(&coverflow_area));

        let empty_box = GtkBox::new(Orientation::Vertical, 10);
        empty_box.set_valign(Align::Center);
        empty_box.set_halign(Align::Center);
        empty_box.set_visible(false);
        let empty_icon = Image::from_icon_name("folder-open-symbolic");
        empty_icon.set_pixel_size(64);
        empty_icon.add_css_class("dim-label");
        let empty_lbl = Label::builder()
            .label(if malus_de_common::is_spanish_locale() {
                "Carpeta vacía"
            } else {
                "Empty Folder"
            })
            .css_classes(["title-3", "dim-label"])
            .build();
        empty_box.append(&empty_icon);
        empty_box.append(&empty_lbl);
        stage_overlay.add_overlay(&empty_box);

        left_pane.append(&stage_overlay);

        // Tira inferior de miniaturas estilo macOS Tahoe Finder
        let carousel_box = GtkBox::new(Orientation::Horizontal, 6);
        carousel_box.add_css_class("branches-gallery-carousel");
        carousel_box.set_valign(Align::Center);
        carousel_box.set_halign(Align::Start);
        carousel_box.set_margin_top(6);
        carousel_box.set_margin_bottom(10);
        carousel_box.set_margin_start(14);
        carousel_box.set_margin_end(14);

        let carousel_scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vscrollbar_policy(PolicyType::Never)
            .css_classes(["branches-gallery-scrolled"])
            .min_content_height(66)
            .hexpand(true)
            .vexpand(false)
            .child(&carousel_box)
            .build();
        left_pane.append(&carousel_scrolled);

        container.append(&left_pane);

        // Panel inspector derecho fijo estilo macOS Tahoe Finder
        let inspector_holder = GtkBox::new(Orientation::Vertical, 0);
        inspector_holder.add_css_class("branches-gallery-inspector");
        inspector_holder.set_size_request(GALLERY_INSPECTOR_WIDTH, -1);
        inspector_holder.set_hexpand(false);
        inspector_holder.set_vexpand(true);
        container.append(&inspector_holder);

        let gallery = Rc::new(Self {
            container,
            coverflow_area,
            empty_box,
            inspector_holder,
            carousel_box,
            carousel_scrolled,
            items: RefCell::new(Vec::new()),
            cover_cache: RefCell::new(HashMap::new()),
            hit_rects: RefCell::new(Vec::new()),
            scroll_pos: Cell::new(0.0),
            target_pos: Cell::new(0.0),
            anim_gen: Cell::new(0),
            scroll_accum: Cell::new(0.0),
            current_folder: RefCell::new(None),
            pending_select_name: RefCell::new(None),
            current_index: Cell::new(None),
            selected_indices: RefCell::new(BTreeSet::new()),
            selection_anchor: Cell::new(None),
            carousel_widgets: RefCell::new(Vec::new()),
            on_navigate,
            on_selection_changed,
        });

        // Dibujo Cairo 3D Cover Flow
        {
            let g_weak = Rc::downgrade(&gallery);
            gallery
                .coverflow_area
                .set_draw_func(move |_area, cr, width, height| {
                    if let Some(g) = g_weak.upgrade() {
                        g.draw_coverflow(cr, width as f64, height as f64);
                    }
                });
        }

        // Clic directo sobre el escenario Cover Flow (seleccionar portada lateral, doble clic abrir, clic derecho menú)
        {
            let click = GestureClick::new();
            click.set_button(0);
            let g_click = gallery.clone();
            click.connect_pressed(move |g, n_press, x, y| {
                g_click.container.grab_focus();
                let button = g.current_button();
                let hit = g_click.hit_test_cover(x, y);
                if button == gdk::BUTTON_SECONDARY {
                    g.set_state(gtk4::EventSequenceState::Claimed);
                    if let Some(idx) = hit {
                        let already_selected = g_click.selected_indices.borrow().contains(&idx);
                        if !already_selected {
                            g_click.select_index(idx);
                        }
                        let item_opt = g_click.items.borrow().get(idx).cloned();
                        if let Some(item) = item_opt {
                            show_item_context_menu(
                                &g_click.coverflow_area,
                                &item.path,
                                item.is_dir,
                                Some((x, y)),
                                Some(&g_click.on_navigate),
                                None,
                            );
                            return;
                        }
                    }
                    crate::views::show_background_context_menu(&g_click.coverflow_area, x, y);
                    return;
                }
                if button == gdk::BUTTON_PRIMARY {
                    if let Some(idx) = hit {
                        let state = g.current_event_state();
                        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
                        let ctrl = crate::views::is_multi_select_modifier(state);
                        if n_press >= 2 && g_click.current_index.get() == Some(idx) {
                            let item_opt = g_click.items.borrow().get(idx).cloned();
                            if let Some(item) = item_opt {
                                if item.is_dir {
                                    (g_click.on_navigate)(item.path);
                                } else {
                                    let _ = open_file(&item.path);
                                }
                            }
                        } else if shift {
                            g_click.select_range_to(idx);
                        } else if ctrl {
                            g_click.toggle_select_index(idx);
                        } else {
                            g_click.select_index(idx);
                        }
                        g.set_state(gtk4::EventSequenceState::Claimed);
                    }
                }
            });
            gallery.coverflow_area.add_controller(click);
        }

        // Desplazamiento con rueda / gesto táctil horizontal o vertical sobre Cover Flow
        {
            let scroll = EventControllerScroll::new(EventControllerScrollFlags::BOTH_AXES);
            let g_scroll = gallery.clone();
            scroll.connect_scroll(move |_, dx, dy| {
                let delta = if dx.abs() >= dy.abs() { dx } else { dy };
                let acc = g_scroll.scroll_accum.get() + delta;
                if acc >= 0.65 {
                    g_scroll.scroll_accum.set(0.0);
                    g_scroll.select_next();
                } else if acc <= -0.65 {
                    g_scroll.scroll_accum.set(0.0);
                    g_scroll.select_prev();
                } else {
                    g_scroll.scroll_accum.set(acc);
                }
                glib::Propagation::Stop
            });
            gallery.coverflow_area.add_controller(scroll);
        }

        // Controlador de teclado: Flecha Izq / Der navega el Cover Flow (Shift extiende rango)
        let key_controller = EventControllerKey::new();
        let g_weak = gallery.clone();
        key_controller.connect_key_pressed(move |_, keyval, _, state| {
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            match keyval {
                Key::Left => {
                    if shift {
                        if let Some(curr) = g_weak.current_index.get() {
                            if curr > 0 {
                                g_weak.select_range_to(curr - 1);
                            }
                        }
                    } else {
                        g_weak.select_prev();
                    }
                    glib::Propagation::Stop
                }
                Key::Right => {
                    if shift {
                        let total = g_weak.items.borrow().len();
                        if let Some(curr) = g_weak.current_index.get() {
                            if curr + 1 < total {
                                g_weak.select_range_to(curr + 1);
                            }
                        }
                    } else {
                        g_weak.select_next();
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        gallery.container.add_controller(key_controller);

        gallery
    }

    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn set_current_folder(&self, folder: Option<&Path>) {
        *self.current_folder.borrow_mut() = folder.map(Path::to_path_buf);
    }

    pub fn get_selected_item_and_widget(&self) -> Option<(FileItem, gtk4::Widget)> {
        let idx = self
            .current_index
            .get()
            .or_else(|| self.selected_indices.borrow().iter().next().copied())?;
        let item = self.items.borrow().get(idx).cloned()?;
        let widget = self
            .carousel_widgets
            .borrow()
            .get(idx)
            .map(|w| w.clone().upcast())
            .unwrap_or_else(|| self.container.clone().upcast());
        Some((item, widget))
    }

    pub fn get_selected_items(&self) -> Vec<FileItem> {
        let items = self.items.borrow();
        self.selected_indices
            .borrow()
            .iter()
            .filter_map(|&idx| items.get(idx).cloned())
            .collect()
    }

    pub fn select_all(self: &Rc<Self>) {
        let total = self.items.borrow().len();
        if total == 0 {
            return;
        }
        {
            let mut sel = self.selected_indices.borrow_mut();
            sel.clear();
            for idx in 0..total {
                sel.insert(idx);
            }
        }
        if self.current_index.get().is_none() {
            self.current_index.set(Some(0));
        }
        if self.selection_anchor.get().is_none() {
            self.selection_anchor.set(Some(0));
        }
        self.sync_selection_visuals(true);
    }

    pub fn queue_select_name(&self, name: &str) {
        *self.pending_select_name.borrow_mut() = Some(name.to_string());
    }

    pub fn item_names_ordered(&self) -> Vec<String> {
        self.items.borrow().iter().map(|it| it.name.clone()).collect()
    }

    pub fn selected_item_name(&self) -> Option<String> {
        let idx = self.current_index.get()?;
        self.items.borrow().get(idx).map(|it| it.name.clone())
    }

    pub fn select_item_by_name(self: &Rc<Self>, name: &str) -> bool {
        let idx = self.items.borrow().iter().position(|it| it.name == name);
        if let Some(i) = idx {
            self.select_index(i);
            true
        } else {
            false
        }
    }

    pub fn refresh_cut_states(&self) {
        let items = self.items.borrow();
        let cards = self.carousel_widgets.borrow();
        for (item, card) in items.iter().zip(cards.iter()) {
            crate::views::sync_cut_css_class(card, &item.path);
        }
    }

    pub fn set_items(self: &Rc<Self>, items: &[FileItem]) {
        *self.items.borrow_mut() = items.to_vec();
        self.cover_cache.borrow_mut().clear();
        self.hit_rects.borrow_mut().clear();
        self.current_index.set(None);
        self.selected_indices.borrow_mut().clear();
        self.selection_anchor.set(None);
        self.scroll_accum.set(0.0);

        while let Some(child) = self.carousel_box.first_child() {
            self.carousel_box.remove(&child);
        }
        self.carousel_widgets.borrow_mut().clear();

        if items.is_empty() {
            self.pending_select_name.borrow_mut().take();
            self.empty_box.set_visible(true);
            self.coverflow_area.queue_draw();
            self.clear_inspector();
            (self.on_selection_changed)(0);
            return;
        }

        self.empty_box.set_visible(false);

        for (idx, item) in items.iter().enumerate() {
            let card = self.create_carousel_card(idx, item);
            self.carousel_box.append(&card);
            self.carousel_widgets.borrow_mut().push(card);
        }

        let target_idx = self
            .pending_select_name
            .borrow_mut()
            .take()
            .and_then(|target| items.iter().position(|it| it.name == target))
            .unwrap_or(0);

        // Posicionar inicialmente sin animación brusca desde 0 si se abre en otro índice
        self.scroll_pos.set(target_idx as f64);
        self.target_pos.set(target_idx as f64);
        self.select_index_inner(target_idx, false);
    }

    pub fn select_prev(self: &Rc<Self>) {
        if let Some(curr) = self.current_index.get() {
            if curr > 0 {
                self.select_index(curr - 1);
            }
        }
    }

    pub fn select_next(self: &Rc<Self>) {
        let total = self.items.borrow().len();
        if let Some(curr) = self.current_index.get() {
            if curr + 1 < total {
                self.select_index(curr + 1);
            }
        }
    }

    pub fn select_index(self: &Rc<Self>, index: usize) {
        self.select_index_inner(index, true);
    }

    fn select_index_inner(self: &Rc<Self>, index: usize, animate: bool) {
        let total = self.items.borrow().len();
        if index < total {
            self.current_index.set(Some(index));
            self.selection_anchor.set(Some(index));
            {
                let mut sel = self.selected_indices.borrow_mut();
                sel.clear();
                sel.insert(index);
            }
            self.sync_selection_visuals(animate);
        }
    }

    pub fn toggle_select_index(self: &Rc<Self>, index: usize) {
        let total = self.items.borrow().len();
        if index >= total {
            return;
        }
        let next_focus = {
            let mut sel = self.selected_indices.borrow_mut();
            if sel.contains(&index) {
                sel.remove(&index);
                sel.iter().next_back().copied()
            } else {
                sel.insert(index);
                Some(index)
            }
        };
        if next_focus.is_some() {
            self.current_index.set(next_focus);
            self.selection_anchor.set(next_focus);
        } else {
            self.current_index.set(None);
            self.selection_anchor.set(None);
        }
        self.sync_selection_visuals(true);
    }

    pub fn select_range_to(self: &Rc<Self>, index: usize) {
        let total = self.items.borrow().len();
        if index >= total {
            return;
        }
        let anchor = self
            .selection_anchor
            .get()
            .or_else(|| self.current_index.get())
            .unwrap_or(index);
        {
            let mut sel = self.selected_indices.borrow_mut();
            sel.clear();
            for i in crate::views::inclusive_range(anchor, index) {
                sel.insert(i);
            }
        }
        self.selection_anchor.set(Some(anchor));
        self.current_index.set(Some(index));
        self.sync_selection_visuals(true);
    }

    fn sync_selection_visuals(self: &Rc<Self>, animate: bool) {
        let sel = self.selected_indices.borrow();
        let widgets = self.carousel_widgets.borrow();
        for (i, widget) in widgets.iter().enumerate() {
            if sel.contains(&i) {
                widget.add_css_class("branches-carousel-active");
            } else {
                widget.remove_css_class("branches-carousel-active");
            }
        }
        let selected_list: Vec<FileItem> = {
            let items = self.items.borrow();
            sel.iter()
                .filter_map(|&idx| items.get(idx).cloned())
                .collect()
        };
        let count = sel.len();
        drop(sel);
        drop(widgets);

        if let Some(idx) = self.current_index.get() {
            self.animate_coverflow_to(idx, animate);
            self.ensure_filmstrip_visible(idx);
        } else {
            self.coverflow_area.queue_draw();
        }

        if selected_list.is_empty() {
            self.clear_inspector();
        } else if selected_list.len() == 1 {
            self.rebuild_single_inspector(&selected_list[0]);
        } else {
            self.rebuild_multi_inspector(&selected_list);
        }

        (self.on_selection_changed)(count);
    }

    fn animate_coverflow_to(self: &Rc<Self>, target_idx: usize, animate: bool) {
        let target = target_idx as f64;
        self.target_pos.set(target);
        self.preload_covers_around(target_idx);

        let start_pos = self.scroll_pos.get();
        if !animate || (target - start_pos).abs() < 0.001 || !self.coverflow_area.is_realized() {
            self.scroll_pos.set(target);
            self.coverflow_area.queue_draw();
            return;
        }

        let gen = self.anim_gen.get().wrapping_add(1);
        self.anim_gen.set(gen);
        let g_weak = Rc::downgrade(self);
        let start_time: Cell<Option<i64>> = Cell::new(None);

        self.coverflow_area.add_tick_callback(move |area, clock| {
            let Some(g) = g_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if g.anim_gen.get() != gen {
                return glib::ControlFlow::Break;
            }
            let now = clock.frame_time();
            let t0 = match start_time.get() {
                Some(v) => v,
                None => {
                    start_time.set(Some(now));
                    now
                }
            };
            let t = ((now - t0) as f64 / COVERFLOW_ANIM_US).clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - t).powi(3);
            let cur = start_pos + (target - start_pos) * eased;
            g.scroll_pos.set(cur);
            area.queue_draw();
            if t >= 1.0 {
                g.scroll_pos.set(target);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn ensure_filmstrip_visible(&self, index: usize) {
        let widgets = self.carousel_widgets.borrow();
        let Some(card) = widgets.get(index) else {
            return;
        };
        let Some(bounds) = card.compute_bounds(&self.carousel_box) else {
            return;
        };
        let hadj = self.carousel_scrolled.hadjustment();
        let view_start = hadj.value();
        let view_w = hadj.page_size();
        if view_w <= 1.0 {
            return;
        }
        let card_left = bounds.x() as f64;
        let card_right = (bounds.x() + bounds.width()) as f64;
        let pad = 28.0;
        let max_val = (hadj.upper() - view_w).max(0.0);
        if card_left < view_start + pad {
            hadj.set_value((card_left - pad).clamp(0.0, max_val));
        } else if card_right > view_start + view_w - pad {
            hadj.set_value((card_right - view_w + pad).clamp(0.0, max_val));
        }
    }

    fn preload_covers_around(&self, center: usize) {
        let items = self.items.borrow();
        if items.is_empty() {
            return;
        }
        let lo = center.saturating_sub(COVERFLOW_VISIBLE_RADIUS as usize + 1);
        let hi = (center + COVERFLOW_VISIBLE_RADIUS as usize + 1).min(items.len() - 1);
        let mut cache = self.cover_cache.borrow_mut();
        cache.retain(|&idx, _| idx.abs_diff(center) <= COVERFLOW_CACHE_KEEP_RADIUS);
        for idx in lo..=hi {
            if !cache.contains_key(&idx) {
                if let Some(surface) = render_cover_surface(&items[idx]) {
                    cache.insert(idx, surface);
                }
            }
        }
    }

    fn hit_test_cover(&self, x: f64, y: f64) -> Option<usize> {
        let rects = self.hit_rects.borrow();
        // `hit_rects` se guarda en orden de dibujo (de fondo a frente); buscar en reversa toma la portada más frontal
        for &(idx, x0, y0, x1, y1) in rects.iter().rev() {
            if x >= x0 && x <= x1 && y >= y0 && y <= y1 {
                return Some(idx);
            }
        }
        None
    }

    fn draw_coverflow(&self, cr: &cairo::Context, stage_w: f64, stage_h: f64) {
        let items_len = self.items.borrow().len();
        self.hit_rects.borrow_mut().clear();
        if items_len == 0 || stage_w <= 20.0 || stage_h <= 20.0 {
            return;
        }

        let pos = self.scroll_pos.get().clamp(0.0, (items_len - 1) as f64);
        let center_idx = pos.round() as usize;
        self.preload_covers_around(center_idx);

        // Ordenar índices visibles de mayor distancia al centro hacia el frente (painter's algorithm)
        let lo = (center_idx as i32 - COVERFLOW_VISIBLE_RADIUS).max(0) as usize;
        let hi = (center_idx as i32 + COVERFLOW_VISIBLE_RADIUS).min(items_len as i32 - 1) as usize;
        let mut draw_order: Vec<(usize, f64)> = (lo..=hi)
            .map(|i| (i, i as f64 - pos))
            .filter(|(_, off)| off.abs() <= COVERFLOW_VISIBLE_RADIUS as f64 + 0.35)
            .collect();
        draw_order.sort_by(|a, b| {
            b.1.abs()
                .partial_cmp(&a.1.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let max_center_w = (stage_w * 0.76).clamp(160.0, 680.0);
        let max_center_h = (stage_h * 0.80).clamp(140.0, 520.0);
        let cache = self.cover_cache.borrow();
        let mut new_hits = Vec::with_capacity(draw_order.len());

        for (idx, offset) in draw_order {
            let Some(cover) = cache.get(&idx) else {
                continue;
            };
            let base_scale = (max_center_w / cover.width)
                .min(max_center_h / cover.height)
                .min(1.0);
            let card_w = (cover.width * base_scale).max(48.0);
            let card_h = (cover.height * base_scale).max(48.0);
            let slot = compute_cover_slot(offset, stage_w, stage_h, card_w, card_h);
            if slot.opacity <= 0.01 {
                continue;
            }

            let draw_w = card_w * slot.scale;
            let draw_h = card_h * slot.scale;
            let bounds = if slot.yaw_rad.abs() < 0.012 {
                draw_flat_center_cover(cr, cover, &slot, draw_w, draw_h)
            } else {
                draw_perspective_wing_cover(cr, cover, &slot, draw_w, draw_h)
            };
            new_hits.push((idx, bounds.0, bounds.1, bounds.2, bounds.3));
        }

        *self.hit_rects.borrow_mut() = new_hits;
    }

    fn clear_inspector(&self) {
        while let Some(child) = self.inspector_holder.first_child() {
            self.inspector_holder.remove(&child);
        }
    }

    fn rebuild_single_inspector(self: &Rc<Self>, item: &FileItem) {
        self.clear_inspector();
        let es = malus_de_common::is_spanish_locale();

        let content_vbox = GtkBox::new(Orientation::Vertical, 0);
        content_vbox.set_margin_top(16);
        content_vbox.set_margin_bottom(12);
        content_vbox.set_margin_start(16);
        content_vbox.set_margin_end(16);

        let ext_lower = item
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let is_img = is_preview_image_path(&item.path);
        let can_save_rotation = matches!(ext_lower.as_str(), "png" | "jpg" | "jpeg" | "bmp");

        // 1. Cabecera compacta (miniatura 42x42 a la izquierda + título y subtítulo a la derecha)
        let header_row = GtkBox::new(Orientation::Horizontal, 10);
        header_row.set_valign(Align::Start);

        let mini_thumb = crate::views::create_item_image(item, 42, Some("branches-gallery-mini-thumb"));
        mini_thumb.set_size_request(42, 42);
        mini_thumb.set_valign(Align::Start);
        header_row.append(&mini_thumb);

        let title_col = GtkBox::new(Orientation::Vertical, 2);
        title_col.set_hexpand(true);
        title_col.set_valign(Align::Center);

        let name_lbl = Label::builder()
            .label(&item.name)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .lines(2)
            .ellipsize(pango::EllipsizeMode::Middle)
            .max_width_chars(22)
            .xalign(0.0)
            .halign(Align::Fill)
            .css_classes(["branches-gallery-inspector-title"])
            .build();
        title_col.append(&name_lbl);

        let subtitle_text = format_preview_subtitle(&item.kind_str, &item.size_str);
        let sub_lbl = Label::builder()
            .label(&subtitle_text)
            .ellipsize(pango::EllipsizeMode::End)
            .max_width_chars(24)
            .xalign(0.0)
            .halign(Align::Fill)
            .css_classes(["branches-preview-subtitle"])
            .build();
        title_col.append(&sub_lbl);
        header_row.append(&title_col);
        content_vbox.append(&header_row);

        // 2. Encabezado «Información» + enlace azul «Mostrar más»
        let info_header = GtkBox::new(Orientation::Horizontal, 8);
        info_header.set_margin_top(16);
        info_header.set_margin_bottom(6);
        info_header.append(
            &Label::builder()
                .label(if es { "Información" } else { "Information" })
                .xalign(0.0)
                .halign(Align::Start)
                .hexpand(true)
                .css_classes(["branches-preview-section-title"])
                .build(),
        );
        let show_more_btn = Button::builder()
            .label(if es { "Mostrar más" } else { "Show More" })
            .has_frame(false)
            .css_classes(["branches-preview-more-link"])
            .build();
        info_header.append(&show_more_btn);
        content_vbox.append(&info_header);

        // 3. Metadatos clave-valor con divisores sutiles
        let meta_box = GtkBox::new(Orientation::Vertical, 0);
        let mut first_row = true;
        let append_meta_row = |parent: &GtkBox, first: &mut bool, key: &str, val: &str| -> Label {
            if !*first {
                let div = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .css_classes(["branches-preview-meta-divider"])
                    .hexpand(true)
                    .build();
                parent.append(&div);
            }
            *first = false;

            let row = GtkBox::new(Orientation::Horizontal, 8);
            row.add_css_class("branches-preview-meta-row");
            row.append(
                &Label::builder()
                    .label(key)
                    .halign(Align::Start)
                    .xalign(0.0)
                    .hexpand(true)
                    .css_classes(["branches-preview-meta-key"])
                    .build(),
            );
            let val_lbl = Label::builder()
                .label(val)
                .halign(Align::End)
                .xalign(1.0)
                .ellipsize(pango::EllipsizeMode::Middle)
                .max_width_chars(18)
                .selectable(true)
                .css_classes(["branches-preview-meta-val"])
                .build();
            row.append(&val_lbl);
            parent.append(&row);
            val_lbl
        };

        let created_str = item
            .created
            .map(format_modified)
            .unwrap_or_else(|| item.modified_str.clone());
        let accessed_str = item
            .accessed
            .map(format_modified)
            .unwrap_or_else(|| item.modified_str.clone());

        append_meta_row(
            &meta_box,
            &mut first_row,
            if es { "Creación" } else { "Created" },
            &created_str,
        );
        append_meta_row(
            &meta_box,
            &mut first_row,
            if es { "Modificación" } else { "Modified" },
            &item.modified_str,
        );
        append_meta_row(
            &meta_box,
            &mut first_row,
            if es {
                "Última apertura"
            } else {
                "Last opened"
            },
            &accessed_str,
        );

        let mut dims_val_label: Option<Label> = None;
        if is_img {
            if let Some((_, w, h)) = Pixbuf::file_info(&item.path) {
                let lbl = append_meta_row(
                    &meta_box,
                    &mut first_row,
                    if es { "Dimensiones" } else { "Dimensions" },
                    &format_dimensions(w, h),
                );
                dims_val_label = Some(lbl);
            }
            let (dpi_x, dpi_y) = read_image_dpi(&item.path).unwrap_or((72, 72));
            append_meta_row(
                &meta_box,
                &mut first_row,
                if es { "Resolución" } else { "Resolution" },
                &format_dimensions(dpi_x as i32, dpi_y as i32),
            );
        }
        content_vbox.append(&meta_box);

        // Metadatos adicionales («Mostrar más»)
        let extra_meta_box = GtkBox::new(Orientation::Vertical, 0);
        extra_meta_box.set_visible(false);
        let mut extra_first = false;
        append_meta_row(
            &extra_meta_box,
            &mut extra_first,
            if es { "Permisos" } else { "Permissions" },
            &format_posix_permissions(&item.path),
        );
        let parent_dir = item
            .path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());
        append_meta_row(
            &extra_meta_box,
            &mut extra_first,
            if es { "Ubicación" } else { "Where" },
            &parent_dir,
        );
        append_meta_row(
            &extra_meta_box,
            &mut extra_first,
            if es { "Tamaño exacto" } else { "Exact size" },
            &format!("{} bytes", item.size),
        );
        content_vbox.append(&extra_meta_box);

        {
            let extra_c = extra_meta_box.clone();
            let expanded = Cell::new(false);
            show_more_btn.connect_clicked(move |btn| {
                let next = !expanded.get();
                expanded.set(next);
                extra_c.set_visible(next);
                btn.set_label(if next {
                    if es {
                        "Mostrar menos"
                    } else {
                        "Show Less"
                    }
                } else if es {
                    "Mostrar más"
                } else {
                    "Show More"
                });
            });
        }

        // 4. Sección «Etiquetas» (idéntica a la captura de macOS Tahoe Finder)
        let tags_section = build_tags_editor_section(&item.path, es);
        content_vbox.append(&tags_section);

        let scrolled_insp = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .propagate_natural_width(false)
            .max_content_width(GALLERY_INSPECTOR_WIDTH)
            .vexpand(true)
            .child(&content_vbox)
            .build();
        self.inspector_holder.append(&scrolled_insp);

        // 5. Barra inferior fija de acciones rápidas
        let actions_bar = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .halign(Align::Fill)
            .hexpand(true)
            .css_classes(["branches-preview-actions-bar"])
            .build();
        let actions_inner = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .halign(Align::Center)
            .hexpand(true)
            .build();

        if is_img {
            let rotate_btn = make_quick_action_button(
                "object-rotate-left-symbolic",
                if es {
                    "Girar a la\nizquierda"
                } else {
                    "Rotate\nLeft"
                },
            );
            let g_weak = Rc::downgrade(self);
            let path_rot = item.path.clone();
            let ext_rot = ext_lower.clone();
            let dims_lbl_c = dims_val_label.clone();
            let cur_idx = self.current_index.get();
            rotate_btn.connect_clicked(move |_| {
                if can_save_rotation {
                    if let Ok(full_pb) = Pixbuf::from_file(&path_rot) {
                        if let Some(rot_full) =
                            full_pb.rotate_simple(PixbufRotation::Counterclockwise)
                        {
                            let fmt = match ext_rot.as_str() {
                                "jpg" | "jpeg" => "jpeg",
                                "bmp" => "bmp",
                                _ => "png",
                            };
                            if rot_full.savev(&path_rot, fmt, &[]).is_ok() {
                                if let Some(ref lbl) = dims_lbl_c {
                                    lbl.set_label(&format_dimensions(
                                        rot_full.width(),
                                        rot_full.height(),
                                    ));
                                }
                            }
                        }
                    }
                }
                if let Some(g) = g_weak.upgrade() {
                    if let Some(idx) = cur_idx {
                        g.cover_cache.borrow_mut().remove(&idx);
                        g.coverflow_area.queue_draw();
                    }
                }
            });
            actions_inner.append(&rotate_btn);

            let markup_btn = make_quick_action_button(
                "document-edit-symbolic",
                if es { "Marcado" } else { "Markup" },
            );
            let path_m = item.path.clone();
            markup_btn.connect_clicked(move |_| {
                let _ = open_file(&path_m);
            });
            actions_inner.append(&markup_btn);
        } else {
            let open_btn = make_quick_action_button(
                "document-open-symbolic",
                if es { "Abrir" } else { "Open" },
            );
            let path_o = item.path.clone();
            let is_dir = item.is_dir;
            let on_nav = self.on_navigate.clone();
            open_btn.connect_clicked(move |_| {
                if is_dir {
                    on_nav(path_o.clone());
                } else {
                    let _ = open_file(&path_o);
                }
            });
            actions_inner.append(&open_btn);

            let ql_btn = make_quick_action_button(
                "view-reveal-symbolic",
                if es { "Vista rápida" } else { "Quick Look" },
            );
            let p_str = item.path.to_string_lossy().to_string();
            ql_btn.connect_clicked(move |b| {
                let _ = b.activate_action("win.quick_look_item", Some(&p_str.to_variant()));
            });
            actions_inner.append(&ql_btn);
        }

        let more_btn = build_single_more_action_button(item, es);
        actions_inner.append(&more_btn);
        actions_bar.append(&actions_inner);
        self.inspector_holder.append(&actions_bar);
    }

    fn rebuild_multi_inspector(self: &Rc<Self>, items: &[FileItem]) {
        self.clear_inspector();
        let es = malus_de_common::is_spanish_locale();
        let summary = summarize_multi_selection(items, es);

        let content_vbox = GtkBox::new(Orientation::Vertical, 0);
        content_vbox.set_margin_top(16);
        content_vbox.set_margin_bottom(12);
        content_vbox.set_margin_start(16);
        content_vbox.set_margin_end(16);

        let title_lbl = Label::builder()
            .label(&summary.title)
            .xalign(0.0)
            .halign(Align::Fill)
            .css_classes(["branches-gallery-inspector-title"])
            .build();
        content_vbox.append(&title_lbl);

        let sub_lbl = Label::builder()
            .label(&summary.subtitle)
            .ellipsize(pango::EllipsizeMode::End)
            .max_width_chars(26)
            .xalign(0.0)
            .halign(Align::Fill)
            .margin_top(2)
            .css_classes(["branches-preview-subtitle"])
            .build();
        content_vbox.append(&sub_lbl);

        let info_lbl = Label::builder()
            .label(if es { "Información" } else { "Information" })
            .xalign(0.0)
            .halign(Align::Start)
            .margin_top(16)
            .margin_bottom(6)
            .css_classes(["branches-preview-section-title"])
            .build();
        content_vbox.append(&info_lbl);

        let meta_box = GtkBox::new(Orientation::Vertical, 0);
        let mut first = true;
        let append_row = |parent: &GtkBox, first: &mut bool, key: &str, val: &str| {
            if !*first {
                parent.append(
                    &GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .css_classes(["branches-preview-meta-divider"])
                        .hexpand(true)
                        .build(),
                );
            }
            *first = false;
            let row = GtkBox::new(Orientation::Horizontal, 8);
            row.add_css_class("branches-preview-meta-row");
            row.append(
                &Label::builder()
                    .label(key)
                    .halign(Align::Start)
                    .xalign(0.0)
                    .hexpand(true)
                    .css_classes(["branches-preview-meta-key"])
                    .build(),
            );
            row.append(
                &Label::builder()
                    .label(val)
                    .halign(Align::End)
                    .xalign(1.0)
                    .ellipsize(pango::EllipsizeMode::Middle)
                    .max_width_chars(18)
                    .css_classes(["branches-preview-meta-val"])
                    .build(),
            );
            parent.append(&row);
        };

        append_row(
            &meta_box,
            &mut first,
            if es { "Elementos" } else { "Items" },
            &summary.breakdown,
        );
        append_row(
            &meta_box,
            &mut first,
            if es { "Tamaño total" } else { "Total size" },
            &summary.total_size_str,
        );
        append_row(
            &meta_box,
            &mut first,
            if es { "Tipo" } else { "Kind" },
            &summary.kind_label,
        );
        if let Some(created) = summary.earliest_created {
            append_row(
                &meta_box,
                &mut first,
                if es { "Creación" } else { "Created" },
                &format_modified(created),
            );
        }
        if let Some(modified) = summary.latest_modified {
            append_row(
                &meta_box,
                &mut first,
                if es { "Modificación" } else { "Modified" },
                &format_modified(modified),
            );
        }
        content_vbox.append(&meta_box);

        let scrolled_insp = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .propagate_natural_width(false)
            .max_content_width(GALLERY_INSPECTOR_WIDTH)
            .vexpand(true)
            .child(&content_vbox)
            .build();
        self.inspector_holder.append(&scrolled_insp);

        let actions_bar = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .halign(Align::Fill)
            .hexpand(true)
            .css_classes(["branches-preview-actions-bar"])
            .build();
        let actions_inner = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .halign(Align::Center)
            .hexpand(true)
            .build();

        let file_paths: Vec<PathBuf> = items
            .iter()
            .filter(|it| !it.is_dir)
            .map(|it| it.path.clone())
            .collect();
        let all_paths_joined = items
            .iter()
            .map(|it| it.path.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");

        let open_btn = make_quick_action_button(
            "document-open-symbolic",
            if es { "Abrir" } else { "Open" },
        );
        open_btn.connect_clicked(move |_| {
            for p in &file_paths {
                let _ = open_file(p);
            }
        });
        actions_inner.append(&open_btn);

        let copy_btn = make_quick_action_button(
            "edit-copy-symbolic",
            if es { "Copiar\nrutas" } else { "Copy\nPaths" },
        );
        copy_btn.connect_clicked(move |_| {
            if let Some(display) = gdk::Display::default() {
                display.clipboard().set_text(&all_paths_joined);
            }
        });
        actions_inner.append(&copy_btn);

        actions_bar.append(&actions_inner);
        self.inspector_holder.append(&actions_bar);
    }

    fn create_carousel_card(self: &Rc<Self>, index: usize, item: &FileItem) -> GtkBox {
        let card = GtkBox::new(Orientation::Vertical, 0);
        card.add_css_class("branches-carousel-card");
        card.set_tooltip_text(Some(&item.name));
        if item.is_hidden {
            card.add_css_class("branches-hidden-item");
        }
        if crate::operations::is_path_cut(&item.path) {
            card.add_css_class("branches-cut-item");
        }
        card.set_valign(Align::Center);

        let thumb_frame = GtkBox::new(Orientation::Vertical, 0);
        thumb_frame.add_css_class("branches-carousel-thumb-frame");
        thumb_frame.set_overflow(gtk4::Overflow::Hidden);
        thumb_frame.set_size_request(FILMSTRIP_THUMB_W, FILMSTRIP_THUMB_H);
        thumb_frame.set_halign(Align::Center);
        thumb_frame.set_valign(Align::Center);

        let img = crate::views::create_item_image(item, FILMSTRIP_THUMB_H, Some("branches-carousel-thumb"));
        img.set_size_request(FILMSTRIP_THUMB_W, FILMSTRIP_THUMB_H);
        img.set_halign(Align::Center);
        img.set_valign(Align::Center);
        thumb_frame.append(&img);
        card.append(&thumb_frame);

        let click = GestureClick::new();
        click.set_button(0);
        let self_click = self.clone();
        let p_for_dbl = item.path.clone();
        let is_dir = item.is_dir;
        let on_nav = self.on_navigate.clone();
        let card_for_menu = card.clone();

        click.connect_pressed(move |g, n_press, x, y| {
            self_click.container.grab_focus();
            let button = g.current_button();
            let state = g.current_event_state();
            if button == gdk::BUTTON_SECONDARY {
                if !self_click.selected_indices.borrow().contains(&index) {
                    self_click.select_index(index);
                }
                show_item_context_menu(
                    &card_for_menu,
                    &p_for_dbl,
                    is_dir,
                    Some((x, y)),
                    Some(&on_nav),
                    None,
                );
            } else if n_press == 1 {
                let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
                let ctrl = crate::views::is_multi_select_modifier(state);
                if shift {
                    self_click.select_range_to(index);
                } else if ctrl {
                    self_click.toggle_select_index(index);
                } else {
                    self_click.select_index(index);
                }
            } else if n_press >= 2 {
                if is_dir {
                    on_nav(p_for_dbl.clone());
                } else {
                    let _ = open_file(&p_for_dbl);
                }
            }
            g.set_state(gtk4::EventSequenceState::Claimed);
        });
        card.add_controller(click);

        let drag_source = gtk4::DragSource::new();
        drag_source.set_actions(gdk::DragAction::MOVE | gdk::DragAction::COPY);
        let self_drag = self.clone();
        let path_drag_fallback = item.path.clone();
        drag_source.connect_prepare(move |_, _x, _y| {
            if !self_drag.selected_indices.borrow().contains(&index) {
                self_drag.select_index(index);
            }
            let paths: Vec<PathBuf> = self_drag
                .get_selected_items()
                .into_iter()
                .map(|it| it.path)
                .collect();
            if paths.is_empty() {
                Some(crate::views::build_file_drag_content_provider(
                    &path_drag_fallback,
                ))
            } else {
                Some(crate::views::build_files_drag_content_provider(&paths))
            }
        });
        let path_end = item.path.clone();
        let on_nav_end = self.on_navigate.clone();
        drag_source.connect_drag_end(move |_, drag, _| {
            if let Some(parent) = path_end.parent() {
                let _ = crate::views::refresh_if_moved_out(drag, &path_end, parent, &on_nav_end);
            }
        });
        if let Some(display) = gdk::Display::default() {
            let theme = gtk4::IconTheme::for_display(&display);
            let paintable = theme.lookup_by_gicon(
                &item.gicon,
                32,
                1,
                gtk4::TextDirection::Ltr,
                gtk4::IconLookupFlags::empty(),
            );
            drag_source.set_icon(Some(&paintable), 16, 16);
        }
        card.add_controller(drag_source);

        card
    }
}

const COVER_SQUIRCLE_RATIO: f64 = 0.056;
const COVER_SQUIRCLE_EXPONENT: f64 = 4.2;
const COVER_SQUIRCLE_SEGMENTS: usize = 16;

/// Radio squircle proporcional e invariante a escala para que el recorte en la superficie
/// coincida exactamente con el borde y la sombra proyectados en Cover Flow.
fn cover_squircle_radius(w: f64, h: f64) -> f64 {
    (w.min(h) * COVER_SQUIRCLE_RATIO).max(6.0)
}

/// Traza un contorno squircle (`n = 4.2`) proyectado en perspectiva 3D con cámara pinhole.
#[allow(clippy::too_many_arguments)]
fn trace_projected_squircle_path(
    cr: &cairo::Context,
    slot: &CoverSlot,
    draw_w: f64,
    draw_h: f64,
    expand: f64,
    off_y: f64,
    radius: f64,
    d: f64,
) {
    let w = draw_w + expand * 2.0;
    let h = draw_h + expand * 2.0;
    let r = (radius + expand * 0.4).min(w * 0.5).min(h * 0.5).max(1.0);
    let power = 2.0 / COVER_SQUIRCLE_EXPONENT;
    let quarter = std::f64::consts::FRAC_PI_2;
    let cos_y = slot.yaw_rad.cos();
    let sin_y = slot.yaw_rad.sin();

    let project_pt = |px_local: f64, py_local: f64| -> (f64, f64) {
        let lx = px_local - w * 0.5;
        let ly = py_local - h * 0.5;
        let x3 = lx * cos_y;
        let z3 = lx * sin_y;
        let f = d / (d + z3).max(40.0);
        (slot.center_x + x3 * f, slot.center_y + ly * f + off_y)
    };

    cr.new_path();
    let (sx, sy) = project_pt(r, 0.0);
    cr.move_to(sx, sy);

    // Borde superior y esquina superior derecha
    let (p1x, p1y) = project_pt(w - r, 0.0);
    cr.line_to(p1x, p1y);
    for step in 1..=COVER_SQUIRCLE_SEGMENTS {
        let a = quarter * step as f64 / COVER_SQUIRCLE_SEGMENTS as f64;
        let (qx, qy) = project_pt(
            w - r + r * a.sin().powf(power),
            r - r * a.cos().powf(power),
        );
        cr.line_to(qx, qy);
    }

    // Borde derecho y esquina inferior derecha
    let (p2x, p2y) = project_pt(w, h - r);
    cr.line_to(p2x, p2y);
    for step in 1..=COVER_SQUIRCLE_SEGMENTS {
        let a = quarter * step as f64 / COVER_SQUIRCLE_SEGMENTS as f64;
        let (qx, qy) = project_pt(
            w - r + r * a.cos().powf(power),
            h - r + r * a.sin().powf(power),
        );
        cr.line_to(qx, qy);
    }

    // Borde inferior y esquina inferior izquierda
    let (p3x, p3y) = project_pt(r, h);
    cr.line_to(p3x, p3y);
    for step in 1..=COVER_SQUIRCLE_SEGMENTS {
        let a = quarter * step as f64 / COVER_SQUIRCLE_SEGMENTS as f64;
        let (qx, qy) = project_pt(
            r - r * a.sin().powf(power),
            h - r + r * a.cos().powf(power),
        );
        cr.line_to(qx, qy);
    }

    // Borde izquierdo y esquina superior izquierda
    let (p4x, p4y) = project_pt(0.0, r);
    cr.line_to(p4x, p4y);
    for step in 1..=COVER_SQUIRCLE_SEGMENTS {
        let a = quarter * step as f64 / COVER_SQUIRCLE_SEGMENTS as f64;
        let (qx, qy) = project_pt(
            r - r * a.cos().powf(power),
            r - r * a.sin().powf(power),
        );
        cr.line_to(qx, qy);
    }

    cr.close_path();
}

/// Dibuja la portada central en posición plana con esquinas squircle (`n = 4.2`), sombra suave y reflejo sutil.
fn draw_flat_center_cover(
    cr: &cairo::Context,
    cover: &CoverSurface,
    slot: &CoverSlot,
    draw_w: f64,
    draw_h: f64,
) -> (f64, f64, f64, f64) {
    let dx = (slot.center_x - draw_w * 0.5).round();
    let dy = (slot.center_y - draw_h * 0.5).round();
    let radius = cover_squircle_radius(draw_w, draw_h);

    // Reflejo sutil en el suelo (efecto Cover Flow)
    let refl_h = (draw_h * 0.16).min(56.0);
    if refl_h > 4.0 {
        let _ = cr.save();
        cr.rectangle(dx, dy + draw_h + 2.0, draw_w, refl_h);
        cr.clip();
        cr.translate(dx, dy + draw_h * 2.0 + 2.0);
        cr.scale(draw_w / cover.width, -draw_h / cover.height);
        let _ = cr.set_source_surface(&cover.surface, 0.0, 0.0);
        cr.source().set_filter(cairo::Filter::Good);
        let mask = LinearGradient::new(0.0, cover.height, 0.0, cover.height * (1.0 - refl_h / draw_h));
        mask.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.14 * slot.opacity);
        mask.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.0);
        let _ = cr.mask(&mask);
        let _ = cr.restore();
    }

    // Sombra suave multicapa con contorno squircle
    for (expand, off_y, alpha) in [(5.0, 7.0, 0.05), (2.5, 4.0, 0.08), (1.0, 1.5, 0.07)] {
        crate::app_window::squircle_path_corners(
            cr,
            dx - expand,
            dy - expand + off_y,
            draw_w + expand * 2.0,
            draw_h + expand * 2.0,
            radius + expand * 0.4,
            true,
            true,
        );
        cr.set_source_rgba(0.0, 0.0, 0.0, alpha * slot.opacity);
        let _ = cr.fill();
    }

    // Imagen principal recortada en squircle continuo (n = 4.2)
    let _ = cr.save();
    crate::app_window::squircle_path_corners(cr, dx, dy, draw_w, draw_h, radius, true, true);
    cr.clip();
    cr.translate(dx, dy);
    cr.scale(draw_w / cover.width, draw_h / cover.height);
    let _ = cr.set_source_surface(&cover.surface, 0.0, 0.0);
    cr.source().set_filter(cairo::Filter::Good);
    let _ = cr.paint_with_alpha(slot.opacity);
    let _ = cr.restore();

    // Borde hairline sutil en squircle
    crate::app_window::squircle_path_corners(
        cr,
        dx + 0.5,
        dy + 0.5,
        draw_w - 1.0,
        draw_h - 1.0,
        radius,
        true,
        true,
    );
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.10 * slot.opacity);
    cr.set_line_width(1.0);
    let _ = cr.stroke();

    (dx, dy, dx + draw_w, dy + draw_h)
}

/// Dibuja una portada lateral en perspectiva 3D Cover Flow proyectando en franjas verticales con esquinas squircle.
fn draw_perspective_wing_cover(
    cr: &cairo::Context,
    cover: &CoverSurface,
    slot: &CoverSlot,
    draw_w: f64,
    draw_h: f64,
) -> (f64, f64, f64, f64) {
    let d = (draw_w * 2.35).max(360.0);
    let cos_y = slot.yaw_rad.cos();
    let sin_y = slot.yaw_rad.sin();
    let radius = cover_squircle_radius(draw_w, draw_h);

    let project = |u: f64| -> (f64, f64, f64) {
        let lx = (u - 0.5) * draw_w;
        let x3 = lx * cos_y;
        let z3 = lx * sin_y;
        let f = d / (d + z3).max(40.0);
        let px = slot.center_x + x3 * f;
        let half_h = (draw_h * 0.5) * f;
        (px, slot.center_y - half_h, slot.center_y + half_h)
    };

    let (x_l, top_l, bot_l) = project(0.0);
    let (x_r, top_r, bot_r) = project(1.0);
    let min_x = x_l.min(x_r);
    let max_x = x_l.max(x_r);
    let min_y = top_l.min(top_r);
    let max_y = bot_l.max(bot_r);

    // Sombra squircle proyectada en perspectiva
    for (expand, off_y, alpha) in [(4.0, 7.0, 0.05), (1.5, 3.5, 0.08)] {
        trace_projected_squircle_path(cr, slot, draw_w, draw_h, expand, off_y, radius, d);
        cr.set_source_rgba(0.0, 0.0, 0.0, alpha * slot.opacity);
        let _ = cr.fill();
    }

    // Franjas verticales proyectadas con cámara pinhole para perspectiva real
    let slices = COVERFLOW_PERSPECTIVE_SLICES;
    for s in 0..slices {
        let u0 = s as f64 / slices as f64;
        let u1 = (s + 1) as f64 / slices as f64;
        let um = (u0 + u1) * 0.5;
        let (px0, t0, b0) = project(u0);
        let (px1, t1, b1) = project(u1);
        let (_, tm, bm) = project(um);
        let seg_w = px1 - px0;
        if seg_w.abs() < 0.2 {
            continue;
        }
        let seg_h = (bm - tm).max(1.0);
        let sx = u0 * cover.width;
        let sw = (cover.width / slices as f64).max(0.5);

        let _ = cr.save();
        // Solapamiento de 0.6 px para evitar grietas subpíxel entre franjas
        let pad = if seg_w > 0.0 { 0.6 } else { -0.6 };
        cr.new_path();
        cr.move_to(px0, t0);
        cr.line_to(px1 + pad, t1);
        cr.line_to(px1 + pad, b1);
        cr.line_to(px0, b0);
        cr.close_path();
        cr.clip();

        cr.translate(px0, tm);
        cr.scale(seg_w / sw, seg_h / cover.height);
        let _ = cr.set_source_surface(&cover.surface, -sx, 0.0);
        cr.source().set_filter(cairo::Filter::Good);
        let _ = cr.paint_with_alpha(slot.opacity);
        let _ = cr.restore();

        // Reflejo en perspectiva debajo de cada franja
        let refl_h = (seg_h * 0.14).min(42.0);
        if refl_h > 3.0 {
            let _ = cr.save();
            cr.new_path();
            cr.move_to(px0, b0 + 2.0);
            cr.line_to(px1 + pad, b1 + 2.0);
            cr.line_to(px1 + pad, b1 + 2.0 + refl_h);
            cr.line_to(px0, b0 + 2.0 + refl_h);
            cr.close_path();
            cr.clip();

            cr.translate(px0, bm + seg_h + 2.0);
            cr.scale(seg_w / sw, -seg_h / cover.height);
            let _ = cr.set_source_surface(&cover.surface, -sx, 0.0);
            cr.source().set_filter(cairo::Filter::Good);
            let mask = LinearGradient::new(
                0.0,
                cover.height,
                0.0,
                cover.height * (1.0 - refl_h / seg_h),
            );
            mask.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.11 * slot.opacity);
            mask.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.0);
            let _ = cr.mask(&mask);
            let _ = cr.restore();
        }
    }

    // Sombreado de profundidad direccional y borde hairline sobre el contorno squircle proyectado
    trace_projected_squircle_path(cr, slot, draw_w, draw_h, 0.0, 0.0, radius, d);
    let shade = if slot.yaw_rad > 0.0 {
        LinearGradient::new(x_l, 0.0, x_r, 0.0)
    } else {
        LinearGradient::new(x_r, 0.0, x_l, 0.0)
    };
    shade.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, (slot.shade_alpha * 0.45) * slot.opacity);
    shade.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, slot.shade_alpha * slot.opacity);
    let _ = cr.set_source(&shade);
    let _ = cr.fill_preserve();

    cr.set_source_rgba(0.0, 0.0, 0.0, 0.12 * slot.opacity);
    cr.set_line_width(1.0);
    let _ = cr.stroke();

    (min_x, min_y, max_x, max_y)
}

/// Construye la superficie Cairo en caché para una portada de Cover Flow (imagen, texto o icono).
fn render_cover_surface(item: &FileItem) -> Option<CoverSurface> {
    if !item.is_dir && is_preview_image_path(&item.path) {
        if let Ok(pb) = Pixbuf::from_file_at_scale(&item.path, 760, 560, true) {
            return pixbuf_to_cover_surface(&pb);
        }
    }

    // Miniatura de video si ya existe en caché (sin bloquear el hilo UI)
    if !item.is_dir && malus_de_common::video_thumb::is_video_path(&item.path) {
        let mod_secs = item
            .modified
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        for bucket in [512, 256, 128] {
            let cache_path = malus_de_common::video_thumb::video_thumb_cache_path(
                &item.path,
                mod_secs,
                item.size,
                bucket,
            );
            if cache_path.is_file() {
                if let Ok(pb) = Pixbuf::from_file_at_scale(&cache_path, 640, 440, true) {
                    return pixbuf_to_cover_surface(&pb);
                }
            }
        }
    }

    let ext = item
        .path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let is_text = !item.is_dir
        && matches!(
            ext.as_str(),
            "rs" | "toml"
                | "json"
                | "md"
                | "txt"
                | "sh"
                | "py"
                | "c"
                | "cpp"
                | "h"
                | "js"
                | "ts"
                | "html"
                | "css"
                | "yaml"
                | "yml"
                | "xml"
                | "log"
                | "ini"
                | "conf"
        );
    if is_text {
        return render_text_cover_surface(item);
    }

    render_icon_cover_surface(item)
}

fn pixbuf_to_cover_surface(pb: &Pixbuf) -> Option<CoverSurface> {
    let w = pb.width().max(1);
    let h = pb.height().max(1);
    let surface = ImageSurface::create(Format::ARgb32, w, h).ok()?;
    let cr = cairo::Context::new(&surface).ok()?;
    let r = cover_squircle_radius(w as f64, h as f64);
    crate::app_window::squircle_path_corners(&cr, 0.0, 0.0, w as f64, h as f64, r, true, true);
    cr.clip();
    gtk4::prelude::GdkCairoContextExt::set_source_pixbuf(&cr, pb, 0.0, 0.0);
    let _ = cr.paint();
    Some(CoverSurface {
        surface,
        width: w as f64,
        height: h as f64,
    })
}

fn render_text_cover_surface(item: &FileItem) -> Option<CoverSurface> {
    let w = 400;
    let h = 480;
    let surface = ImageSurface::create(Format::ARgb32, w, h).ok()?;
    let cr = cairo::Context::new(&surface).ok()?;
    let r = cover_squircle_radius(w as f64, h as f64);
    crate::app_window::squircle_path_corners(&cr, 0.0, 0.0, w as f64, h as f64, r, true, true);
    cr.clip();

    // Hoja blanca de documento con cabecera limpia
    cr.set_source_rgb(0.99, 0.99, 0.995);
    let _ = cr.paint();

    cr.set_source_rgba(0.0, 0.0, 0.0, 0.05);
    cr.rectangle(0.0, 0.0, w as f64, 42.0);
    let _ = cr.fill();

    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(13.0);
    cr.set_source_rgb(0.15, 0.15, 0.18);
    cr.move_to(18.0, 26.0);
    let short_name: String = item.name.chars().take(36).collect();
    let _ = cr.show_text(&short_name);

    let snippet = match fs::File::open(&item.path) {
        Ok(file) => {
            use std::io::Read;
            let mut buf = Vec::with_capacity(2048);
            let _ = file.take(2048).read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        }
        Err(_) => String::new(),
    };

    cr.select_font_face(
        "Monospace",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Normal,
    );
    cr.set_font_size(10.5);
    cr.set_source_rgb(0.24, 0.25, 0.29);
    let mut y = 66.0;
    for line in snippet.lines().take(24) {
        let trimmed: String = line.chars().take(48).collect();
        cr.move_to(18.0, y);
        let _ = cr.show_text(&trimmed);
        y += 16.5;
        if y > h as f64 - 20.0 {
            break;
        }
    }

    Some(CoverSurface {
        surface,
        width: w as f64,
        height: h as f64,
    })
}

fn render_icon_cover_surface(item: &FileItem) -> Option<CoverSurface> {
    let w = 360;
    let h = 360;
    let surface = ImageSurface::create(Format::ARgb32, w, h).ok()?;
    let cr = cairo::Context::new(&surface).ok()?;
    let r = cover_squircle_radius(w as f64, h as f64);
    crate::app_window::squircle_path_corners(&cr, 0.0, 0.0, w as f64, h as f64, r, true, true);
    cr.clip();

    // Fondo de tarjeta luminosa limpia
    let bg = LinearGradient::new(0.0, 0.0, 0.0, h as f64);
    bg.add_color_stop_rgb(0.0, 0.97, 0.975, 0.985);
    bg.add_color_stop_rgb(1.0, 0.92, 0.93, 0.95);
    let _ = cr.set_source(&bg);
    let _ = cr.paint();

    // Intentar cargar el icono del tema como Pixbuf para renderizarlo nítido en la tarjeta
    let mut drew_icon = false;
    if let Some(display) = gdk::Display::default() {
        let theme = gtk4::IconTheme::for_display(&display);
        let paintable = theme.lookup_by_gicon(
            &item.gicon,
            160,
            1,
            gtk4::TextDirection::Ltr,
            gtk4::IconLookupFlags::empty(),
        );
        if let Some(file) = paintable.file() {
            if let Some(path) = file.path() {
                if let Ok(pb) = Pixbuf::from_file_at_scale(&path, 160, 160, true) {
                    let ix = (w as f64 - pb.width() as f64) * 0.5;
                    let iy = (h as f64 - pb.height() as f64) * 0.44;
                    gtk4::prelude::GdkCairoContextExt::set_source_pixbuf(&cr, &pb, ix, iy);
                    let _ = cr.paint();
                    drew_icon = true;
                }
            }
        }
    }

    if !drew_icon {
        // Respaldo vectorial elegante (carpeta azul macOS o documento)
        if item.is_dir {
            rounded_rect_path(&cr, 100.0, 104.0, 160.0, 118.0, 14.0);
            cr.set_source_rgb(0.32, 0.67, 0.97);
            let _ = cr.fill();
        } else {
            rounded_rect_path(&cr, 122.0, 86.0, 116.0, 148.0, 10.0);
            cr.set_source_rgb(1.0, 1.0, 1.0);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.14);
            cr.set_line_width(1.5);
            let _ = cr.stroke();
        }
    }

    // Pie con el nombre del archivo para identificarlo en Cover Flow
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(14.0);
    cr.set_source_rgb(0.18, 0.19, 0.22);
    let label_txt: String = item.name.chars().take(28).collect();
    if let Ok(extents) = cr.text_extents(&label_txt) {
        let tx = ((w as f64 - extents.width()) * 0.5).max(16.0);
        cr.move_to(tx, h as f64 - 36.0);
        let _ = cr.show_text(&label_txt);
    }

    Some(CoverSurface {
        surface,
        width: w as f64,
        height: h as f64,
    })
}

/// Construye la sección «Etiquetas» del panel inspector de Galería con persistencia real en BD/xattr.
fn build_tags_editor_section(file_path: &Path, es: bool) -> GtkBox {
    let section = GtkBox::new(Orientation::Vertical, 6);
    section.set_margin_top(16);

    section.append(
        &Label::builder()
            .label(if es { "Etiquetas" } else { "Tags" })
            .xalign(0.0)
            .halign(Align::Start)
            .css_classes(["branches-preview-section-title"])
            .build(),
    );

    let box_frame = GtkBox::new(Orientation::Vertical, 4);
    box_frame.add_css_class("branches-gallery-tags-box");

    let chips_flow = FlowBox::builder()
        .selection_mode(SelectionMode::None)
        .row_spacing(4)
        .column_spacing(4)
        .homogeneous(false)
        .build();
    box_frame.append(&chips_flow);

    let entry = Entry::builder()
        .placeholder_text(if es {
            "Agregar etiquetas..."
        } else {
            "Add Tags..."
        })
        .has_frame(false)
        .css_classes(["branches-gallery-tags-entry"])
        .build();
    box_frame.append(&entry);
    section.append(&box_frame);

    let populate_chips: Rc<dyn Fn()> = {
        let chips_c = chips_flow.clone();
        let path_c = file_path.to_path_buf();
        Rc::new(move || {
            while let Some(ch) = chips_c.first_child() {
                chips_c.remove(&ch);
            }
            let Ok(conn) = crate::db::open_db() else {
                chips_c.set_visible(false);
                return;
            };
            let tags = crate::db::get_file_tags(&conn, &path_c);
            chips_c.set_visible(!tags.is_empty());
            for (tag_id, _color_hex) in tags {
                let chip = GtkBox::new(Orientation::Horizontal, 4);
                chip.add_css_class("branches-gallery-tag-chip");

                let meta = crate::db::get_tag_metadata(&conn, &tag_id, es);
                let dot = crate::sidebar::create_cairo_tag_dot(&meta.color_id, 8);
                dot.set_valign(Align::Center);
                chip.append(&dot);

                chip.append(
                    &Label::builder()
                        .label(&meta.label)
                        .css_classes(["branches-gallery-tag-chip-label"])
                        .build(),
                );

                let rm_btn = Button::builder()
                    .label("×")
                    .has_frame(false)
                    .css_classes(["branches-gallery-tag-remove"])
                    .build();
                let p_rm = path_c.clone();
                let tid_rm = tag_id.clone();
                let chip_rm = chip.clone();
                let flow_rm = chips_c.clone();
                rm_btn.connect_clicked(move |_| {
                    if let Ok(c) = crate::db::open_db() {
                        let _ = crate::db::remove_file_tag(&c, &p_rm, &tid_rm);
                    }
                    if let Some(fb_child) = chip_rm.parent() {
                        flow_rm.remove(&fb_child);
                    }
                    if flow_rm.first_child().is_none() {
                        flow_rm.set_visible(false);
                    }
                });
                chip.append(&rm_btn);
                chips_c.insert(&chip, -1);
            }
        })
    };

    populate_chips();

    {
        let path_add = file_path.to_path_buf();
        let pop_c = populate_chips.clone();
        entry.connect_activate(move |e| {
            let raw = e.text().trim().to_string();
            if raw.is_empty() {
                return;
            }
            if let Ok(conn) = crate::db::open_db() {
                let color = crate::db::tag_definition(&crate::db::canonical_tag_id(&raw))
                    .map(|d| d.color_hex)
                    .unwrap_or("#8e8e93");
                let _ = crate::db::set_file_tag(&conn, &path_add, &raw, color);
            }
            e.set_text("");
            pop_c();
        });
    }

    section
}

fn build_single_more_action_button(item: &FileItem, es: bool) -> Button {
    let more_btn = make_quick_action_button(
        "view-more-horizontal-symbolic",
        if es { "Más..." } else { "More..." },
    );
    let more_pop = Popover::builder()
        .has_arrow(false)
        .css_classes(["malus-popover-menu"])
        .build();
    more_pop.set_parent(&more_btn);

    let more_menu = GtkBox::new(Orientation::Vertical, 1);
    more_menu.set_margin_top(4);
    more_menu.set_margin_bottom(4);
    more_menu.set_margin_start(4);
    more_menu.set_margin_end(4);

    let add_pop_item = |menu: &GtkBox, label: &str| -> Button {
        let b = Button::builder()
            .label(label)
            .has_frame(false)
            .css_classes(["popover-menu-item"])
            .build();
        if let Some(l) = b.child().and_then(|c| c.downcast::<Label>().ok()) {
            l.set_xalign(0.0);
        }
        crate::app_window::attach_squircle_menu_hover(&b);
        menu.append(&b);
        b
    };

    let pop_open = add_pop_item(
        &more_menu,
        if es { "Abrir archivo" } else { "Open File" },
    );
    let pop_ql = add_pop_item(
        &more_menu,
        if es { "Vista rápida" } else { "Quick Look" },
    );
    let pop_info = add_pop_item(
        &more_menu,
        if es {
            "Obtener información"
        } else {
            "Get Info"
        },
    );
    let pop_term = add_pop_item(
        &more_menu,
        if es {
            "Abrir en Terminal"
        } else {
            "Open in Terminal"
        },
    );
    let pop_copy = add_pop_item(
        &more_menu,
        if es { "Copiar ruta" } else { "Copy Path" },
    );
    more_pop.set_child(Some(&more_menu));

    {
        let p = item.path.clone();
        let pop = more_pop.clone();
        pop_open.connect_clicked(move |_| {
            pop.popdown();
            let _ = open_file(&p);
        });
    }
    {
        let p_str = item.path.to_string_lossy().to_string();
        let pop = more_pop.clone();
        let anchor = more_btn.clone();
        pop_ql.connect_clicked(move |_| {
            pop.popdown();
            let _ = anchor.activate_action("win.quick_look_item", Some(&p_str.to_variant()));
        });
    }
    {
        let p_str = item.path.to_string_lossy().to_string();
        let pop = more_pop.clone();
        let anchor = more_btn.clone();
        pop_info.connect_clicked(move |_| {
            pop.popdown();
            let _ = anchor.activate_action("win.get_info_item", Some(&p_str.to_variant()));
        });
    }
    {
        let p = item.path.clone();
        let pop = more_pop.clone();
        pop_term.connect_clicked(move |_| {
            pop.popdown();
            open_in_terminal(&p);
        });
    }
    {
        let p_str = item.path.to_string_lossy().to_string();
        let pop = more_pop.clone();
        pop_copy.connect_clicked(move |_| {
            pop.popdown();
            if let Some(display) = gdk::Display::default() {
                display.clipboard().set_text(&p_str);
            }
        });
    }
    {
        let pop = more_pop.clone();
        more_btn.connect_clicked(move |_| {
            pop.popup();
        });
    }
    {
        let pop_weak = more_pop.downgrade();
        more_btn.connect_unrealize(move |_| {
            if let Some(p) = pop_weak.upgrade() {
                p.popdown();
                if p.parent().is_some() {
                    p.unparent();
                }
            }
        });
    }

    more_btn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverflow_slot_geometry_centers_active_and_tilts_wings_symmetrically() {
        let center = compute_cover_slot(0.0, 800.0, 500.0, 420.0, 320.0);
        assert!((center.center_x - 400.0).abs() < 1e-6);
        assert!((center.scale - 1.0).abs() < 1e-6);
        assert!(center.yaw_rad.abs() < 1e-6);
        assert!((center.opacity - 1.0).abs() < 1e-6);

        let left1 = compute_cover_slot(-1.0, 800.0, 500.0, 420.0, 320.0);
        let right1 = compute_cover_slot(1.0, 800.0, 500.0, 420.0, 320.0);
        assert!(left1.center_x < center.center_x);
        assert!(right1.center_x > center.center_x);
        assert!(((center.center_x - left1.center_x) - (right1.center_x - center.center_x)).abs() < 1e-6);
        assert!(left1.yaw_rad > 0.5);
        assert!(right1.yaw_rad < -0.5);
        assert!((left1.yaw_rad + right1.yaw_rad).abs() < 1e-6);
        assert!(left1.scale < center.scale);

        let right2 = compute_cover_slot(2.0, 800.0, 500.0, 420.0, 320.0);
        assert!(right2.center_x > right1.center_x);
        assert!(right2.scale <= right1.scale);
    }
}
