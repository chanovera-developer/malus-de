use gdk_pixbuf::prelude::*;
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, DrawingArea,
    EventControllerMotion, GestureClick, GestureDrag, Image, Label, Orientation, Overlay, Picture,
    Popover, Separator, Window,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use libadwaita as adw;
use malus_de_common::config::{DockAppConfig, DockConfig, DockIconAppearance, DockPosition};
use malus_de_common::ipc::{ipc_socket_path, send_ipc_message, IpcMessage, MinimizedWindowInfo};
use malus_de_common::logging::init_logging;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;
use tracing::{error, info, warn};

fn dock_headroom_height(config: &DockConfig) -> i32 {
    config.headroom_height().min(i32::MAX as u32) as i32
}

struct DockLayoutRefs {
    window: ApplicationWindow,
    dock_root: GtkBox,
    dock_bar: Overlay,
    edge_sensor: GtkBox,
    magnification_headroom: GtkBox,
    dock_content: GtkBox,
    apps_box: GtkBox,
    separator: Separator,
    minimized_box: GtkBox,
    dock_highlights: DrawingArea,
    trash_widget: GtkBox,
    dock_corner_radius: Rc<Cell<f64>>,
}

fn dock_bar_cross_size(cfg: &DockConfig) -> i32 {
    let edge_pad = ((cfg.icon_size as f64) * 0.308).round().clamp(10.0, 32.0) as i32;
    cfg.icon_size as i32 + edge_pad * 2
}

fn apply_popover_dock_placement(popover: &Popover, position: DockPosition) {
    match position {
        DockPosition::Bottom => {
            popover.set_position(gtk4::PositionType::Top);
            popover.set_offset(0, POPOVER_SHADOW_GUTTER);
        }
        DockPosition::Left => {
            popover.set_position(gtk4::PositionType::Right);
            popover.set_offset(-POPOVER_SHADOW_GUTTER, 0);
        }
        DockPosition::Right => {
            popover.set_position(gtk4::PositionType::Left);
            popover.set_offset(POPOVER_SHADOW_GUTTER, 0);
        }
    }
}

fn dock_item_cross_alignment(position: DockPosition) -> (gtk4::Align, gtk4::Align) {
    match position {
        DockPosition::Bottom => (gtk4::Align::Center, gtk4::Align::End),
        DockPosition::Left => (gtk4::Align::Start, gtk4::Align::Center),
        DockPosition::Right => (gtk4::Align::End, gtk4::Align::Center),
    }
}

fn sync_dock_item_child_alignment(widget: &gtk4::Widget, position: DockPosition) {
    let (cross_h, cross_v) = dock_item_cross_alignment(position);
    if widget.is::<Button>()
        || widget.is::<Overlay>()
        || widget.has_css_class("dock-icon-shadow")
        || widget.has_css_class("dock-window-card")
        || widget.has_css_class("dock-primary-icon")
    {
        widget.set_halign(cross_h);
        widget.set_valign(cross_v);
    }
    if widget.has_css_class("dock-icon-inset") {
        widget.set_margin_top(0);
        widget.set_margin_bottom(if position == DockPosition::Bottom { 5 } else { 0 });
        widget.set_margin_start(if position == DockPosition::Left { 5 } else { 0 });
        widget.set_margin_end(if position == DockPosition::Right { 5 } else { 0 });
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        sync_dock_item_child_alignment(&c, position);
        child = c.next_sibling();
    }
}

fn apply_dock_item_geometry(item: &GtkBox, position: DockPosition) {
    let (cross_h, cross_v) = dock_item_cross_alignment(position);
    item.set_orientation(if position == DockPosition::Bottom {
        Orientation::Vertical
    } else {
        Orientation::Horizontal
    });
    item.set_halign(cross_h);
    item.set_valign(cross_v);

    let mut main_child: Option<gtk4::Widget> = None;
    let mut indicator_child: Option<gtk4::Widget> = None;
    let mut child = item.first_child();
    while let Some(c) = child {
        if c.has_css_class("dock-indicator") {
            indicator_child = Some(c.clone());
        } else {
            main_child = Some(c.clone());
        }
        child = c.next_sibling();
    }

    if let (Some(main), Some(indicator)) = (main_child, indicator_child) {
        indicator.set_halign(cross_h);
        indicator.set_valign(cross_v);
        sync_dock_item_child_alignment(&main, position);
        match position {
            DockPosition::Left => {
                item.reorder_child_after(&main, Some(&indicator));
            }
            DockPosition::Bottom | DockPosition::Right => {
                item.reorder_child_after(&indicator, Some(&main));
            }
        }
    }
}

fn sync_all_dock_items_geometry(refs: &DockLayoutRefs, position: DockPosition) {
    let mut child = refs.apps_box.first_child();
    while let Some(c) = child {
        if let Ok(item) = c.clone().downcast::<GtkBox>() {
            apply_dock_item_geometry(&item, position);
        }
        child = c.next_sibling();
    }
    let mut min_child = refs.minimized_box.first_child();
    while let Some(c) = min_child {
        if let Ok(item) = c.clone().downcast::<GtkBox>() {
            apply_dock_item_geometry(&item, position);
        }
        min_child = c.next_sibling();
    }
    apply_dock_item_geometry(&refs.trash_widget, position);
}

fn apply_dock_layout_config(
    refs: &DockLayoutRefs,
    cfg: &DockConfig,
    auto_hide_revealed: bool,
    has_minimized_windows: bool,
) {
    let is_horizontal = cfg.position == DockPosition::Bottom;
    let is_revealed = !cfg.auto_hide || auto_hide_revealed;
    let edge_pad = ((cfg.icon_size as f64) * 0.308).round().clamp(10.0, 32.0) as i32;
    let dock_content_spacing = (edge_pad as f64 * 0.45).round().clamp(4.0, 14.0) as i32;

    DOCK_CURRENT_POSITION.set(cfg.position);
    apply_dock_scale_css(cfg);
    refs.dock_corner_radius.set(cfg.dock_corner_radius() as f64);
    refs.dock_highlights.queue_draw();

    if gtk4_layer_shell::is_supported() {
        refs.window
            .set_anchor(Edge::Bottom, cfg.position == DockPosition::Bottom);
        refs.window
            .set_anchor(Edge::Left, cfg.position == DockPosition::Left);
        refs.window
            .set_anchor(Edge::Right, cfg.position == DockPosition::Right);
        refs.window.set_anchor(Edge::Top, false);

        let gap = cfg.floating_gap as i32;
        refs.window.set_margin(
            Edge::Bottom,
            if cfg.position == DockPosition::Bottom {
                gap
            } else {
                0
            },
        );
        refs.window.set_margin(
            Edge::Left,
            if cfg.position == DockPosition::Left {
                gap
            } else {
                0
            },
        );
        refs.window.set_margin(
            Edge::Right,
            if cfg.position == DockPosition::Right {
                gap
            } else {
                0
            },
        );
        refs.window.set_margin(Edge::Top, 0);

        if cfg.auto_hide {
            refs.window.set_exclusive_zone(0);
        } else if is_horizontal {
            refs.window.auto_exclusive_zone_enable();
        } else {
            // Excluir únicamente la anchura visible de la barra (sin magnification_headroom)
            // para no desplazar ventanas ni agregar padding al top-bar.
            refs.window
                .set_exclusive_zone(dock_bar_cross_size(cfg) + gap);
        }
    }

    refs.dock_root.set_orientation(if is_horizontal {
        Orientation::Vertical
    } else {
        Orientation::Horizontal
    });
    let headroom = dock_headroom_height(cfg);
    match cfg.position {
        DockPosition::Bottom => {
            refs.magnification_headroom.set_size_request(-1, headroom);
            refs.dock_root
                .reorder_child_after(&refs.dock_bar, Some(&refs.magnification_headroom));
            refs.dock_root
                .reorder_child_after(&refs.edge_sensor, Some(&refs.dock_bar));
        }
        DockPosition::Left => {
            refs.magnification_headroom.set_size_request(headroom, -1);
            refs.dock_root
                .reorder_child_after(&refs.dock_bar, Some(&refs.edge_sensor));
            refs.dock_root
                .reorder_child_after(&refs.magnification_headroom, Some(&refs.dock_bar));
        }
        DockPosition::Right => {
            refs.magnification_headroom.set_size_request(headroom, -1);
            refs.dock_root
                .reorder_child_after(&refs.dock_bar, Some(&refs.magnification_headroom));
            refs.dock_root
                .reorder_child_after(&refs.edge_sensor, Some(&refs.dock_bar));
        }
    }
    refs.magnification_headroom.set_visible(is_revealed);

    match cfg.position {
        DockPosition::Bottom => {
            refs.dock_bar.set_halign(gtk4::Align::Center);
            refs.dock_bar.set_valign(gtk4::Align::End);
        }
        DockPosition::Left => {
            refs.dock_bar.set_halign(gtk4::Align::Start);
            refs.dock_bar.set_valign(gtk4::Align::Center);
        }
        DockPosition::Right => {
            refs.dock_bar.set_halign(gtk4::Align::End);
            refs.dock_bar.set_valign(gtk4::Align::Center);
        }
    }

    refs.dock_content.set_orientation(if is_horizontal {
        Orientation::Horizontal
    } else {
        Orientation::Vertical
    });
    refs.dock_content.set_spacing(dock_content_spacing);
    refs.apps_box.set_orientation(if is_horizontal {
        Orientation::Horizontal
    } else {
        Orientation::Vertical
    });
    refs.apps_box.set_spacing(edge_pad);
    match cfg.position {
        DockPosition::Bottom => {
            refs.apps_box.set_halign(gtk4::Align::Start);
            refs.apps_box.set_valign(gtk4::Align::End);
        }
        DockPosition::Left => {
            refs.apps_box.set_halign(gtk4::Align::Start);
            refs.apps_box.set_valign(gtk4::Align::Center);
        }
        DockPosition::Right => {
            refs.apps_box.set_halign(gtk4::Align::End);
            refs.apps_box.set_valign(gtk4::Align::Center);
        }
    }
    refs.apps_box.set_margin_bottom(0);

    refs.separator.set_orientation(if is_horizontal {
        Orientation::Vertical
    } else {
        Orientation::Horizontal
    });
    if is_horizontal {
        refs.separator.set_halign(gtk4::Align::Fill);
        refs.separator.set_valign(gtk4::Align::End);
    } else {
        refs.separator.set_halign(gtk4::Align::Fill);
        refs.separator.set_valign(gtk4::Align::Center);
    }

    refs.minimized_box.set_orientation(if is_horizontal {
        Orientation::Horizontal
    } else {
        Orientation::Vertical
    });
    match cfg.position {
        DockPosition::Bottom => {
            refs.minimized_box.set_halign(gtk4::Align::Center);
            refs.minimized_box.set_valign(gtk4::Align::End);
        }
        DockPosition::Left => {
            refs.minimized_box.set_halign(gtk4::Align::Start);
            refs.minimized_box.set_valign(gtk4::Align::Center);
        }
        DockPosition::Right => {
            refs.minimized_box.set_halign(gtk4::Align::End);
            refs.minimized_box.set_valign(gtk4::Align::Center);
        }
    }
    refs.minimized_box
        .set_visible(has_minimized_windows && !cfg.minimize_to_app_icon);

    sync_all_dock_items_geometry(refs, cfg.position);

    refs.dock_bar.set_visible(is_revealed);
    refs.edge_sensor.set_size_request(0, 0);
    refs.edge_sensor.set_visible(false);
    refs.window.set_opacity(if is_revealed { 1.0 } else { 0.0 });

    if is_revealed {
        queue_dock_resize_mode(&refs.window, &refs.dock_root, &refs.dock_bar, true);
    } else {
        if let Some(surface) = refs.window.surface() {
            let empty = gtk4::cairo::Region::create();
            surface.set_input_region(&empty);
        }
        refs.window.set_default_size(1, 1);
        refs.window.queue_resize();
    }
}

fn apply_dock_icons_size_in_place(
    refs: &DockLayoutRefs,
    base_size: i32,
    animations: &Rc<RefCell<Vec<IconSizeAnimation>>>,
    is_hovered: bool,
) {
    let pos = DOCK_CURRENT_POSITION.get();
    let mut anims = animations.borrow_mut();
    if is_hovered {
        for anim in anims.iter_mut() {
            anim.base_size = base_size;
            anim.target_size = anim.target_size.max(base_size as f64);
        }
    } else {
        anims.clear();
    }
    drop(anims);

    let mut child = refs.apps_box.first_child();
    while let Some(c) = child {
        update_icon_size_recursive(&c, base_size, base_size, pos);
        child = c.next_sibling();
    }
    let mut min_child = refs.minimized_box.first_child();
    while let Some(c) = min_child {
        update_icon_size_recursive(&c, base_size, base_size, pos);
        min_child = c.next_sibling();
    }
    update_icon_size_recursive(refs.trash_widget.upcast_ref(), base_size, base_size, pos);
    queue_dock_icon_backgrounds(refs.dock_content.upcast_ref());
    queue_dock_resize(&refs.window, &refs.dock_root, &refs.dock_bar);
    let win = refs.window.clone();
    let root = refs.dock_root.clone();
    let bar = refs.dock_bar.clone();
    glib::idle_add_local_once(move || {
        queue_dock_resize(&win, &root, &bar);
    });
}

fn sync_dock_indicators(
    indicators: &Rc<RefCell<Vec<(Vec<String>, Label)>>>,
    running_apps: &[String],
    show_indicators: bool,
) {
    for (matchers, indicator) in indicators.borrow().iter() {
        let is_running = show_indicators
            && running_apps.iter().any(|running| {
                let r_lower = running.to_lowercase();
                matchers.iter().any(|m| r_lower.contains(&m.to_lowercase()))
            });
        indicator.set_opacity(if is_running { 1.0 } else { 0.0 });
    }
}

fn find_matching_app_index_for_window(
    matcher_groups: &[Vec<String>],
    app_id: &str,
    title: &str,
) -> Option<usize> {
    if !app_id.trim().is_empty() {
        if let Some(idx) = matcher_groups
            .iter()
            .position(|matchers| !matchers.is_empty() && matches_dock_app(matchers, "", app_id))
        {
            return Some(idx);
        }
    }
    if !title.trim().is_empty() {
        if let Some(idx) = matcher_groups
            .iter()
            .position(|matchers| !matchers.is_empty() && matches_dock_app(matchers, "", title))
        {
            return Some(idx);
        }
    }
    None
}

fn find_app_icon_widget_for_window(
    indicators: &[(Vec<String>, Label)],
    app_id: &str,
    title: &str,
) -> Option<gtk4::Widget> {
    let groups: Vec<Vec<String>> = indicators.iter().map(|(m, _)| m.clone()).collect();
    let idx = find_matching_app_index_for_window(&groups, app_id, title)?;
    indicators.get(idx).and_then(|(_, label)| label.parent())
}

fn window_matches_app(
    matchers: &[String],
    app_name: &str,
    win_info: &malus_de_common::ipc::MinimizedWindowInfo,
) -> bool {
    matches_dock_app(matchers, app_name, &win_info.app_id)
        || matches_dock_app(matchers, app_name, &win_info.title)
}

#[allow(clippy::too_many_arguments)]
fn sync_minimized_windows_ui(
    windows: &[malus_de_common::ipc::MinimizedWindowInfo],
    minimize_to_app_icon: bool,
    icon_size: i32,
    min_box: &GtkBox,
    min_map: &Rc<RefCell<HashMap<u64, gtk4::Widget>>>,
    min_paths: &Rc<RefCell<HashMap<u64, Option<String>>>>,
    indicators: &Rc<RefCell<Vec<(Vec<String>, Label)>>>,
    win_ref: &ApplicationWindow,
    dock_root_ref: &GtkBox,
    dock_bar_ref: &Overlay,
) {
    let mut current_map = min_map.borrow_mut();
    let mut card_windows: Vec<&malus_de_common::ipc::MinimizedWindowInfo> = Vec::new();
    let mut app_icon_targets: Vec<(u64, gtk4::Widget)> = Vec::new();

    if minimize_to_app_icon {
        let ind_guard = indicators.borrow();
        for win_info in windows {
            if let Some(app_widget) =
                find_app_icon_widget_for_window(&ind_guard, &win_info.app_id, &win_info.title)
            {
                app_icon_targets.push((win_info.window_id, app_widget));
            } else {
                card_windows.push(win_info);
            }
        }
    } else {
        card_windows.extend(windows.iter());
    }

    let desired_card_ids: HashSet<u64> = card_windows.iter().map(|w| w.window_id).collect();
    let to_remove: Vec<u64> = current_map
        .keys()
        .filter(|id| !desired_card_ids.contains(id))
        .copied()
        .collect();

    if minimize_to_app_icon {
        for id in to_remove {
            if let Some(widget) = current_map.remove(&id) {
                min_box.remove(&widget);
                min_paths.borrow_mut().remove(&id);
            }
        }
    } else {
        for id in to_remove {
            if let Some(widget) = current_map.get(&id) {
                if let Ok(revealer) = widget.clone().downcast::<gtk4::Revealer>() {
                    widget.set_opacity(0.0);
                    widget.set_sensitive(false);
                    if revealer.reveals_child() {
                        revealer.set_reveal_child(false);
                        let widget = widget.clone();
                        let min_box = min_box.clone();
                        let min_map = min_map.clone();
                        let min_paths = min_paths.clone();
                        let win_ref = win_ref.clone();
                        let dock_root_ref = dock_root_ref.clone();
                        let dock_bar_ref = dock_bar_ref.clone();
                        glib::timeout_add_local_once(Duration::from_millis(240), move || {
                            let still_current = min_map
                                .borrow()
                                .get(&id)
                                .is_some_and(|current| *current == widget);
                            if !still_current || revealer.reveals_child() {
                                return;
                            }
                            min_box.remove(&widget);
                            min_map.borrow_mut().remove(&id);
                            min_paths.borrow_mut().remove(&id);
                            if min_box.first_child().is_none() {
                                min_box.set_visible(false);
                            }
                            queue_dock_resize(&win_ref, &dock_root_ref, &dock_bar_ref);
                        });
                    }
                }
            }
        }
    }

    for win_info in &card_windows {
        let thumbnail_changed =
            min_paths.borrow().get(&win_info.window_id) != Some(&win_info.thumbnail_path);
        if !current_map.contains_key(&win_info.window_id) || thumbnail_changed {
            if let Some(widget) = current_map.remove(&win_info.window_id) {
                min_box.remove(&widget);
            }
            let (card, id) = create_minimized_window_item(win_info, icon_size);
            let revealer = create_dock_item_revealer(&card);
            let widget = revealer.clone().upcast::<gtk4::Widget>();
            min_box.append(&revealer);
            revealer.set_reveal_child(true);
            current_map.insert(id, widget);
            min_paths
                .borrow_mut()
                .insert(id, win_info.thumbnail_path.clone());
        }
        if let Some(widget) = current_map.get(&win_info.window_id) {
            if let Ok(revealer) = widget.clone().downcast::<gtk4::Revealer>() {
                revealer.set_reveal_child(true);
            }
            widget.set_opacity(if win_info.is_animating { 0.0 } else { 1.0 });
            widget.set_sensitive(!win_info.is_animating);
        }
    }

    min_box.set_visible(!current_map.is_empty());
    let mut slot_targets: Vec<(u64, gtk4::Widget)> = current_map
        .iter()
        .filter(|(id, _)| desired_card_ids.contains(id))
        .map(|(id, widget)| (*id, widget.clone()))
        .collect();
    slot_targets.extend(app_icon_targets.iter().cloned());
    drop(current_map);

    queue_dock_resize(win_ref, dock_root_ref, dock_bar_ref);

    // Registrar de inmediato los iconos visibles en el Dock y reforzar tras el reajuste de layout.
    for (window_id, target_widget) in &app_icon_targets {
        register_minimized_dock_slot(target_widget, win_ref, *window_id);
    }
    let dock_window_weak = win_ref.downgrade();
    for delay in [16, 60, 180, 320] {
        let targets = slot_targets.clone();
        let dock_window_weak = dock_window_weak.clone();
        glib::timeout_add_local_once(Duration::from_millis(delay), move || {
            if let Some(dock_window) = dock_window_weak.upgrade() {
                for (window_id, widget) in targets {
                    register_minimized_dock_slot(&widget, &dock_window, window_id);
                }
            }
        });
    }
}

const DOCK_CSS: &str = include_str!("styles/dock.css");

const DOCK_POPOVER_CSS: &str = include_str!("styles/dock-popover.css");

fn main() {
    init_logging("malus_de_dock");
    info!("Iniciando Malus DE Floating Dock (GTK4 / Layer Shell)...");

    // Inicializar entorno aislado de Malus DE y limpiar variables residuales
    let _ = malus_de_common::config::setup_isolated_session_environment();

    // Esperar de forma activa a que el compositor esté listo y aceptando conexiones
    malus_de_common::wayland::wait_for_wayland(10);

    if let Err(err) = adw::init() {
        warn!("Aviso al inicializar libadwaita: {:?}", err);
    }
    adw::StyleManager::default().set_color_scheme(
        if malus_de_common::config::is_dark_mode_active() {
            adw::ColorScheme::ForceDark
        } else {
            adw::ColorScheme::ForceLight
        },
    );

    let app = Application::builder()
        .application_id("org.malus_de.Dock")
        .build();

    app.connect_startup(|_| {
        load_css();
    });

    app.connect_activate(build_dock_ui);

    app.run();
}

thread_local! {
    static DOCK_GLASS_PROVIDER: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
    static DOCK_ACCENT_PROVIDER: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
    static DOCK_SCALE_PROVIDER: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
    static DOCK_CURRENT_POSITION: Cell<DockPosition> = const { Cell::new(DockPosition::Bottom) };
    static DOCK_APPEARANCE_MONITOR: RefCell<Option<gtk4::gio::FileMonitor>> =
        const { RefCell::new(None) };
    static DOCK_REPAINT_TARGETS: RefCell<
        Option<(
            glib::WeakRef<GtkBox>,
            glib::WeakRef<GtkBox>,
            glib::WeakRef<gtk4::ApplicationWindow>,
        )>,
    > = const { RefCell::new(None) };
}

fn apply_dock_scale_css(cfg: &DockConfig) {
    let scale = (cfg.icon_size as f64 / 48.0).clamp(0.75, 2.0);
    // Calibrado a partir de macOS Tahoe (Dock3.png):
    // padding simétrico (~31% del icon_size) adaptado a la orientación (Bottom / Left / Right),
    // con el punto indicador en el borde exterior y el separador fino de 1px entre bloques.
    let edge_pad = ((cfg.icon_size as f64) * 0.308).round().clamp(10.0, 32.0) as i32;
    let btn_radius = (14.0 * scale).round().max(10.0) as i32;
    let sep_len = cfg.icon_size as i32;
    let sep_gap = (edge_pad as f64 * 0.25).round().clamp(2.0, 8.0) as i32;
    let ind_font = (7.0 * scale.sqrt()).round().clamp(6.0, 10.0) as i32;
    let (items_pad, sep_rule, ind_rule, card_margin) = match cfg.position {
        DockPosition::Bottom => (
            format!("{edge_pad}px {edge_pad}px 0px {edge_pad}px"),
            format!("min-width: 1px; min-height: {sep_len}px; margin: 0px {sep_gap}px {edge_pad}px {sep_gap}px;"),
            format!("font-size: {ind_font}px; min-width: 0px; min-height: {edge_pad}px; margin: 0px; padding: 0px;"),
            "0px 2px",
        ),
        DockPosition::Left => (
            format!("{edge_pad}px {edge_pad}px {edge_pad}px 0px"),
            format!("min-width: {sep_len}px; min-height: 1px; margin: {sep_gap}px 0px {sep_gap}px {edge_pad}px;"),
            format!("font-size: {ind_font}px; min-width: {edge_pad}px; min-height: 0px; margin: 0px; padding: 0px;"),
            "2px 0px",
        ),
        DockPosition::Right => (
            format!("{edge_pad}px 0px {edge_pad}px {edge_pad}px"),
            format!("min-width: {sep_len}px; min-height: 1px; margin: {sep_gap}px {edge_pad}px {sep_gap}px 0px;"),
            format!("font-size: {ind_font}px; min-width: {edge_pad}px; min-height: 0px; margin: 0px; padding: 0px;"),
            "2px 0px",
        ),
    };
    let css = format!(
        ".dock-items {{ padding: {items_pad}; }}\n\
         .dock-item-box {{ padding: 0px; }}\n\
         .dock-item-btn {{ padding: 0px; border-radius: {btn_radius}px; }}\n\
         .dock-separator {{ {sep_rule} }}\n\
         .dock-indicator {{ {ind_rule} }}\n\
         .dock-window-card {{ margin: {card_margin}; }}\n"
    );
    DOCK_SCALE_PROVIDER.with(|cell| {
        if let Some(provider) = cell.borrow().as_ref() {
            provider.load_from_string(&css);
        }
    });
}

fn apply_dock_accent_color(index: usize) {
    let css = malus_de_common::config::accent_color_css(index);
    DOCK_ACCENT_PROVIDER.with(|cell| {
        if let Some(provider) = cell.borrow().as_ref() {
            provider.load_from_string(&css);
        }
    });
}

fn apply_dock_liquid_glass_opacity(opacity: f64) {
    let scale = malus_de_common::config::liquid_glass_scale_for(opacity);
    let dock_a = malus_de_common::config::scaled_liquid_glass_alpha(0.42, scale, 0.10, 0.92);
    let css = if malus_de_common::config::load_reduce_transparency()
        && !malus_de_common::config::is_dark_mode_active()
    {
        ".dock-container { background-color: rgb(224, 224, 228); }".to_string()
    } else {
        format!(".dock-container {{ background-color: alpha(@window_bg_color, {dock_a:.3}); }}")
    };
    DOCK_GLASS_PROVIDER.with(|cell| {
        if let Some(provider) = cell.borrow().as_ref() {
            provider.load_from_string(&css);
        }
    });
}

fn load_css() {
    let provider = CssProvider::new();
    provider.load_from_string(DOCK_CSS);
    let glass_provider = CssProvider::new();
    let accent_provider = CssProvider::new();
    let scale_provider = CssProvider::new();

    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let popover_provider = CssProvider::new();
        popover_provider.load_from_string(DOCK_POPOVER_CSS);
        gtk4::style_context_add_provider_for_display(
            &display,
            &popover_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER + 1,
        );

        gtk4::style_context_add_provider_for_display(
            &display,
            &glass_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER + 2,
        );
        gtk4::style_context_add_provider_for_display(
            &display,
            &accent_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER + 3,
        );
        gtk4::style_context_add_provider_for_display(
            &display,
            &scale_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER + 4,
        );
    }

    DOCK_GLASS_PROVIDER.with(|cell| {
        *cell.borrow_mut() = Some(glass_provider);
    });
    DOCK_ACCENT_PROVIDER.with(|cell| {
        *cell.borrow_mut() = Some(accent_provider);
    });
    DOCK_SCALE_PROVIDER.with(|cell| {
        *cell.borrow_mut() = Some(scale_provider);
    });
    apply_dock_accent_color(malus_de_common::config::load_accent_color_index());
    apply_dock_liquid_glass_opacity(malus_de_common::config::load_liquid_glass_opacity());

    let config_file =
        gtk4::gio::File::for_path(malus_de_common::config::appearance_settings_path());
    if let Ok(monitor) = config_file.monitor_file(
        gtk4::gio::FileMonitorFlags::NONE,
        gtk4::gio::Cancellable::NONE,
    ) {
        monitor.connect_changed(|_, _, _, _| {
            malus_de_common::config::invalidate_appearance_settings_cache();
            apply_dock_accent_color(malus_de_common::config::load_accent_color_index());
            apply_dock_liquid_glass_opacity(malus_de_common::config::load_liquid_glass_opacity());
            DOCK_REPAINT_TARGETS.with(|cell| {
                if let Some((apps_w, root_w, win_w)) = cell.borrow().as_ref() {
                    if let Some(apps_box) = apps_w.upgrade() {
                        queue_dock_icon_backgrounds(apps_box.upcast_ref());
                    }
                    if let Some(dock_root) = root_w.upgrade() {
                        dock_root.queue_draw();
                    }
                    if let Some(win) = win_w.upgrade() {
                        win.queue_draw();
                    }
                }
            });
        });
        DOCK_APPEARANCE_MONITOR.with(|cell| {
            *cell.borrow_mut() = Some(monitor);
        });
    }
}

fn create_popover_surface(menu_box: &GtkBox) -> Overlay {
    let overlay = Overlay::new();
    overlay.add_css_class("popover-surface");
    overlay.set_vexpand(false);
    overlay.set_valign(gtk4::Align::Start);
    menu_box.add_css_class("popover-menu-content");
    menu_box.set_vexpand(false);
    if let Some(last_item) = menu_box.last_child() {
        last_item.set_margin_bottom(10);
    }

    let background = DrawingArea::new();
    background.add_css_class("popover-surface");
    background.set_can_target(false);
    background.set_hexpand(true);
    background.set_vexpand(false);

    static POPOVER_SVG_TEMPLATE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let svg_template_opt = POPOVER_SVG_TEMPLATE.get_or_init(|| {
        let path = dock_popover_svg_path()?;
        match std::fs::read_to_string(&path) {
            Ok(s) => Some(s),
            Err(error) => {
                warn!("No se pudo leer el SVG del popover del dock: {error}");
                None
            }
        }
    });

    if let Some(svg_template) = svg_template_opt.as_deref() {
        let svg_template = svg_template.to_string();
        let cached_pixbuf: RefCell<Option<((i32, i32, bool, i32), gtk4::gdk_pixbuf::Pixbuf)>> =
            RefCell::new(None);
        background.set_draw_func(move |area, context, width, height| {
            if width <= 0 || height <= 0 {
                return;
            }
            let is_dark = libadwaita::StyleManager::default().is_dark();
            let glass_milli =
                (malus_de_common::config::current_liquid_glass_scale() * 1000.0).round() as i32;
            let cache_key = (width, height, is_dark, glass_milli);
            let mut cache = cached_pixbuf.borrow_mut();
            let pixbuf = if let Some((k, ref pb)) = *cache {
                if k == cache_key {
                    Some(pb.clone())
                } else {
                    None
                }
            } else {
                None
            };
            let pixbuf = match pixbuf {
                Some(pb) => Ok(pb),
                None => themed_popover_svg(&svg_template, area, width, height).map(|pb| {
                    *cache = Some((cache_key, pb.clone()));
                    pb
                }),
            };
            match pixbuf {
                Ok(svg) => {
                    gtk4::gdk::prelude::GdkCairoContextExt::set_source_pixbuf(
                        context, &svg, 0.0, 0.0,
                    );
                    if let Err(error) = context.paint() {
                        warn!("No se pudo dibujar el fondo SVG del popover: {error}");
                    }
                }
                Err(error) => {
                    warn!("No se pudo adaptar el SVG del popover al tema: {error}");
                }
            }
        });
    } else {
        warn!("No se encontró el SVG del popover del dock.");
    }

    overlay.set_child(Some(&background));
    overlay.add_overlay(menu_box);
    overlay.set_measure_overlay(menu_box, true);
    overlay.set_clip_overlay(menu_box, false);

    menu_box.set_halign(gtk4::Align::Fill);
    menu_box.set_valign(gtk4::Align::Start);
    menu_box.set_margin_start(8 + POPOVER_SHADOW_GUTTER);
    menu_box.set_margin_end(8 + POPOVER_SHADOW_GUTTER);
    menu_box.set_margin_top(6 + POPOVER_SHADOW_GUTTER);
    menu_box.set_margin_bottom(35 + POPOVER_SHADOW_GUTTER);

    overlay
}

const POPOVER_TAIL_WIDTH: f64 = 44.0;
const POPOVER_TAIL_HEIGHT: f64 = 18.0;
const POPOVER_BODY_BOTTOM_INSET: f64 = 34.5;
const POPOVER_SHADOW_GUTTER: i32 = 32;
const SQUIRCLE_EXPONENT: f64 = 4.2;
const SQUIRCLE_SEGMENTS: usize = 24;

fn themed_popover_svg(
    template: &str,
    area: &DrawingArea,
    width: i32,
    height: i32,
) -> Result<gdk_pixbuf::Pixbuf, String> {
    let background = theme_color(area, "window_bg_color");
    let foreground = theme_color(area, "window_fg_color");
    let shadow_gutter = f64::from(POPOVER_SHADOW_GUTTER);
    let content_width = (f64::from(width) - shadow_gutter * 2.0).max(1.0);
    let content_height = (f64::from(height) - shadow_gutter * 2.0).max(1.0);
    let radius = 18.0_f64.min(content_width.min(content_height) / 2.0);
    let tail_width = POPOVER_TAIL_WIDTH.min((content_width - radius * 2.0).max(0.0));
    let tail_height = POPOVER_TAIL_HEIGHT.min(content_height / 4.0);
    let body_bottom = content_height - POPOVER_BODY_BOTTOM_INSET;
    let center_x = content_width / 2.0;
    let tail_half_width = tail_width / 2.0;
    let left_tail = center_x - tail_half_width;
    let right_tail = center_x + tail_half_width;
    let right_edge = content_width - 0.5;
    let corner_center_top = 0.5 + radius;
    let corner_center_right = right_edge - radius;
    let corner_center_left = 0.5 + radius;
    let mut path = format!(
        "M {left_top},{top_edge} H {top_right}",
        left_top = corner_center_left,
        top_edge = 0.5,
        top_right = corner_center_right
    );
    append_squircle_corner(
        &mut path,
        corner_center_right,
        corner_center_top,
        radius,
        -std::f64::consts::FRAC_PI_2,
        0.0,
    );
    path.push_str(&format!(" V {}", body_bottom - radius));
    append_squircle_corner(
        &mut path,
        corner_center_right,
        body_bottom - radius,
        radius,
        0.0,
        std::f64::consts::FRAC_PI_2,
    );
    path.push_str(&format!(
        " H {tail_right} \
         C {tail_right_curve},{body_bottom} {tail_right_inner},{tail_curve_y} {tail_slope_right},{tail_slope_y} \
         L {tail_tip_right},{tail_tip_curve_y} \
         Q {center_x},{tail_tip} {tail_tip_left},{tail_tip_curve_y} \
         L {tail_slope_left},{tail_slope_y} \
         C {tail_left_inner},{tail_curve_y} {tail_left_curve},{body_bottom} {tail_left},{body_bottom} \
         H {bottom_left}",
        tail_right = right_tail,
        tail_right_curve = right_tail - 2.5,
        tail_right_inner = right_tail - 3.5,
        tail_curve_y = body_bottom + 0.5,
        tail_slope_right = center_x + 2.0,
        tail_slope_y = body_bottom + tail_height - 2.0,
        tail_tip_right = center_x + 2.0,
        tail_tip_curve_y = body_bottom + tail_height - 2.0,
        tail_tip = body_bottom + tail_height,
        tail_tip_left = center_x - 2.0,
        tail_slope_left = center_x - 2.0,
        tail_left_inner = left_tail + 3.5,
        tail_left_curve = left_tail + 2.5,
        tail_left = left_tail,
        bottom_left = corner_center_left,
    ));
    append_squircle_corner(
        &mut path,
        corner_center_left,
        body_bottom - radius,
        radius,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    path.push_str(&format!(" V {corner_center_top}"));
    append_squircle_corner(
        &mut path,
        corner_center_left,
        corner_center_top,
        radius,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    path.push_str(" Z");
    let glass_scale = malus_de_common::config::current_liquid_glass_scale();
    let svg_opacity =
        malus_de_common::config::scaled_liquid_glass_alpha(0.65, glass_scale, 0.14, 0.96);
    let mut svg = template
        .replace("#f5f0ff", &blend_color_hex(background, foreground, 0.08))
        .replace("#e9e0f8", &blend_color_hex(background, foreground, 0.025))
        .replace("#d9cdec", &blend_color_hex(background, foreground, 0.06))
        .replace("#ffffff", &color_hex(foreground))
        .replace("opacity=\"0.65\"", &format!("opacity=\"{svg_opacity:.3}\""))
        .replace(
            "width=\"320\" height=\"264\" viewBox=\"0 0 320 264\"",
            &format!("width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\""),
        );
    let path_tag_start = svg
        .find("<path")
        .ok_or_else(|| "El SVG del popover no contiene trazado".to_string())?
        + "<path".len();
    svg.insert_str(
        path_tag_start,
        &format!(" transform=\"translate({shadow_gutter} {shadow_gutter})\""),
    );
    let path_start = svg
        .find(" d=\"")
        .ok_or_else(|| "El SVG del popover no contiene trazado".to_string())?
        + 4;
    let path_end = svg[path_start..]
        .find('"')
        .map(|offset| path_start + offset)
        .ok_or_else(|| "El trazado SVG del popover está incompleto".to_string())?;
    svg.replace_range(path_start..path_end, &path);

    let loader = gdk_pixbuf::PixbufLoader::with_type("svg").map_err(|error| error.to_string())?;
    loader
        .write(svg.as_bytes())
        .map_err(|error| error.to_string())?;
    loader.close().map_err(|error| error.to_string())?;
    loader
        .pixbuf()
        .ok_or_else(|| "El SVG no produjo una imagen".to_string())
}

fn append_squircle_corner(
    path: &mut String,
    center_x: f64,
    center_y: f64,
    radius: f64,
    start_angle: f64,
    end_angle: f64,
) {
    let power = 2.0 / SQUIRCLE_EXPONENT;
    for step in 1..=SQUIRCLE_SEGMENTS {
        let fraction = step as f64 / SQUIRCLE_SEGMENTS as f64;
        let angle = start_angle + (end_angle - start_angle) * fraction;
        let x = angle.cos().signum() * angle.cos().abs().powf(power);
        let y = angle.sin().signum() * angle.sin().abs().powf(power);
        path.push_str(&format!(
            " L {:.3},{:.3}",
            center_x + radius * x,
            center_y + radius * y
        ));
    }
}

fn blend_color_hex(
    background: gtk4::gdk::RGBA,
    foreground: gtk4::gdk::RGBA,
    foreground_weight: f32,
) -> String {
    let channel = |bg: f32, fg: f32| {
        ((bg * (1.0 - foreground_weight) + fg * foreground_weight) * 255.0).round() as u8
    };
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(background.red(), foreground.red()),
        channel(background.green(), foreground.green()),
        channel(background.blue(), foreground.blue())
    )
}

fn color_hex(color: gtk4::gdk::RGBA) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (color.red() * 255.0).round() as u8,
        (color.green() * 255.0).round() as u8,
        (color.blue() * 255.0).round() as u8
    )
}

fn dock_popover_svg_path() -> Option<std::path::PathBuf> {
    let installed_paths = [
        "/usr/local/share/malus-de/dock/macos-popover.svg",
        "/usr/share/malus-de/dock/macos-popover.svg",
    ];
    if let Some(path) = installed_paths
        .iter()
        .map(std::path::PathBuf::from)
        .find(|path| path.is_file())
    {
        return Some(path);
    }

    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent()?
                .parent()?
                .parent()
                .map(|root| root.join("assets/dock/macos-popover.svg"))
        })
        .filter(|path| path.is_file())
}

fn resize_switcher_window(
    window: &Window,
    surface: &Overlay,
    switcher_box: &GtkBox,
    background: &DrawingArea,
) {
    switcher_box.queue_resize();
    surface.queue_resize();

    let (_, width, _, _) = surface.measure(Orientation::Horizontal, -1);
    let width = width.max(1);
    let (_, height, _, _) = surface.measure(Orientation::Vertical, width);
    let height = height.max(1);

    window.set_default_size(width, height);
    window.queue_resize();
    background.queue_draw();
}

fn update_dock_surface_input_region(window: &ApplicationWindow, revealed: bool, width: i32, height: i32) {
    if let Some(surface) = window.surface() {
        if revealed {
            let region = gtk4::cairo::Region::create_rectangle(
                &gtk4::cairo::RectangleInt::new(0, 0, width.max(1), height.max(1)),
            );
            surface.set_input_region(&region);
        } else {
            let empty = gtk4::cairo::Region::create();
            surface.set_input_region(&empty);
        }
    }
}

fn queue_dock_resize_mode(
    window: &ApplicationWindow,
    dock_root: &GtkBox,
    dock_bar: &Overlay,
    allow_grow: bool,
) {
    if !dock_bar.is_visible() {
        update_dock_surface_input_region(window, false, 1, 1);
        if window.width() != 1 || window.height() != 1 {
            window.set_default_size(1, 1);
        }
        window.queue_resize();
        return;
    }

    dock_bar.queue_resize();
    dock_root.queue_resize();

    let (min_w, nat_w, _, _) = dock_root.measure(Orientation::Horizontal, -1);
    let width = nat_w.max(min_w);
    if (1..=16384).contains(&width) {
        let (min_h, nat_h, _, _) = dock_root.measure(Orientation::Vertical, width);
        let height = nat_h.max(min_h);
        let should_update = if allow_grow {
            width != window.width() || height != window.height()
        } else {
            width < window.width() || height < window.height()
        };
        if (1..=16384).contains(&height) && should_update {
            window.set_default_size(width, height);
        }
        update_dock_surface_input_region(window, true, width, height);
    }

    window.queue_resize();
}

fn queue_dock_resize(window: &ApplicationWindow, dock_root: &GtkBox, dock_bar: &Overlay) {
    queue_dock_resize_mode(window, dock_root, dock_bar, false);
}

fn has_visible_dock_popover(widget: &gtk4::Widget) -> bool {
    if let Ok(popover) = widget.clone().downcast::<Popover>() {
        if popover.is_visible() {
            return true;
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if has_visible_dock_popover(&c) {
            return true;
        }
        child = c.next_sibling();
    }
    false
}

fn schedule_auto_hide_check(
    delay_ms: u64,
    hovered_check: Rc<Cell<bool>>,
    edge_check: Rc<Cell<bool>>,
    auto_hide_check: Rc<Cell<bool>>,
    cfg_check: Rc<RefCell<DockConfig>>,
    layout_check: Rc<DockLayoutRefs>,
    min_cache_check: Rc<RefCell<Vec<malus_de_common::ipc::MinimizedWindowInfo>>>,
    dock_content_check: GtkBox,
) {
    glib::timeout_add_local_once(Duration::from_millis(delay_ms), move || {
        let cfg = cfg_check.borrow();
        if !cfg.auto_hide || hovered_check.get() || edge_check.get() || !auto_hide_check.get() {
            return;
        }
        if has_visible_dock_popover(dock_content_check.upcast_ref()) {
            drop(cfg);
            schedule_auto_hide_check(
                150,
                hovered_check,
                edge_check,
                auto_hide_check,
                cfg_check,
                layout_check,
                min_cache_check,
                dock_content_check,
            );
            return;
        }
        auto_hide_check.set(false);
        apply_dock_layout_config(
            &layout_check,
            &cfg,
            false,
            !min_cache_check.borrow().is_empty(),
        );
    });
}

fn sync_dock_apps_placeholder(apps_box: &GtkBox, placeholder: &GtkBox) {
    let (_, width, _, _) = apps_box.measure(Orientation::Horizontal, -1);
    if placeholder.width_request() != width {
        placeholder.set_size_request(width, -1);
    }
}

struct DockPlaceholderWidthAnimation {
    current_width: f64,
    target_width: f64,
}

fn start_dock_placeholder_width_animation(
    animation: &Rc<RefCell<DockPlaceholderWidthAnimation>>,
    animation_running: &Rc<Cell<bool>>,
    window: &ApplicationWindow,
    dock_root: &GtkBox,
    dock_bar: &Overlay,
    apps_box: &GtkBox,
    placeholder: &GtkBox,
) {
    let (_, target_width, _, _) = apps_box.measure(Orientation::Horizontal, -1);
    animation.borrow_mut().target_width = target_width.max(0) as f64;
    if animation_running.replace(true) {
        return;
    }

    let animation = animation.clone();
    let animation_running = animation_running.clone();
    let window = window.clone();
    let dock_root = dock_root.clone();
    let dock_bar = dock_bar.clone();
    let placeholder = placeholder.clone();
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let mut animation = animation.borrow_mut();
        let difference = animation.target_width - animation.current_width;
        if difference.abs() < 0.5 {
            animation.current_width = animation.target_width;
        } else {
            animation.current_width += difference * 0.18;
        }
        let width = animation.current_width.round().max(0.0) as i32;
        let settled = difference.abs() < 0.5;
        drop(animation);

        if placeholder.width_request() != width {
            placeholder.set_size_request(width, -1);
            queue_dock_resize(&window, &dock_root, &dock_bar);
        }
        if settled {
            animation_running.set(false);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn draw_dock_edge_highlights(
    context: &gtk4::cairo::Context,
    width: i32,
    height: i32,
    corner_radius: f64,
    position: DockPosition,
) {
    if width <= 0 || height <= 0 {
        return;
    }

    let is_horizontal = position == DockPosition::Bottom;
    let gradient = if is_horizontal {
        gtk4::cairo::LinearGradient::new(0.0, 0.0, width as f64, 0.0)
    } else {
        gtk4::cairo::LinearGradient::new(0.0, 0.0, 0.0, height as f64)
    };
    gradient.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.0);
    gradient.add_color_stop_rgba(0.08, 1.0, 1.0, 1.0, 0.42);
    gradient.add_color_stop_rgba(0.5, 1.0, 1.0, 1.0, 0.58);
    gradient.add_color_stop_rgba(0.92, 1.0, 1.0, 1.0, 0.42);
    gradient.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);

    let _ = context.save();
    squircle_path(context, width as f64, height as f64, corner_radius);
    let _ = context.clip();
    context.set_line_width(1.5);
    context.set_line_cap(gtk4::cairo::LineCap::Round);
    let _ = context.set_source(&gradient);
    if is_horizontal {
        context.move_to(3.0, 1.5);
        context.line_to(width as f64 - 3.0, 1.5);
        let _ = context.stroke();
        context.move_to(3.0, height as f64 - 1.5);
        context.line_to(width as f64 - 3.0, height as f64 - 1.5);
        let _ = context.stroke();
    } else {
        context.move_to(1.5, 3.0);
        context.line_to(1.5, height as f64 - 3.0);
        let _ = context.stroke();
        context.move_to(width as f64 - 1.5, 3.0);
        context.line_to(width as f64 - 1.5, height as f64 - 3.0);
        let _ = context.stroke();
    }
    let _ = context.restore();
}

fn check_trash_full() -> bool {
    let trash = gio::File::for_uri("trash:///");
    if let Ok(enumerator) = trash.enumerate_children(
        "standard::name",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    ) {
        enumerator.into_iter().next().is_some()
    } else {
        false
    }
}

fn should_ignore_app(identifier: &str) -> bool {
    let lower = identifier.to_lowercase();
    let ignored = [
        "top-bar",
        "layer-shell",
        "desktop",
        "gdm",
        "compositor",
        "xwayland",
        "malus-session",
        "xsettings",
        "pipewire",
        "wireplumber",
        "gnome-shell",
        "unknown",
        "malus-de-top-bar",
        "malus-de-dock",
        "malus-de-wallpaper",
        "malus-de-link",
        "malus-de-spotlight",
        "malus-de-notifications",
    ];
    ignored.iter().any(|ign| lower.contains(ign))
}

fn safe_app_info_executable(
    app_info: &impl glib::object::IsA<gio::AppInfo>,
) -> Option<std::path::PathBuf> {
    use glib::translate::ToGlibPtr;
    unsafe {
        let ptr = gio::ffi::g_app_info_get_executable(app_info.as_ref().to_glib_none().0);
        if ptr.is_null() {
            None
        } else {
            let cstr = std::ffi::CStr::from_ptr(ptr);
            cstr.to_str().ok().map(std::path::PathBuf::from)
        }
    }
}

fn resolve_desktop_app_info(identifier: &str) -> (String, String, String, Vec<String>) {
    let lower = identifier.to_lowercase();

    // 1. Mapeo específico para Ajustes del Sistema de Malus DE
    if lower == "dev.malus.settings"
        || lower == "malus-de-settings"
        || (lower.contains("malus") && lower.contains("settings"))
    {
        return (
            "Ajustes del Sistema".to_string(),
            "preferences-system".to_string(),
            "malus-de-settings".to_string(),
            vec![
                "malus-de-settings".to_string(),
                "dev.malus.settings".to_string(),
                "dev.malus.Settings".to_string(),
                "ajustes".to_string(),
                "settings".to_string(),
            ],
        );
    }

    // 1.1. Mapeo específico para Branches de Malus DE
    if lower == "dev.malus.branches"
        || lower == "malus-de-branches"
        || (lower.contains("malus") && lower.contains("branches"))
    {
        return (
            "Branches".to_string(),
            "system-file-manager".to_string(),
            "malus-de-branches".to_string(),
            vec![
                "dev.malus.Branches".to_string(),
                "dev.malus.branches".to_string(),
                "malus-de-branches".to_string(),
                "branches".to_string(),
                "files".to_string(),
            ],
        );
    }

    let candidates = [
        format!("{}.desktop", identifier),
        format!("{}.desktop", lower),
        format!("{}-ide.desktop", lower),
        format!("org.gnome.{}.desktop", identifier),
        format!("org.gnome.{}.desktop", lower),
        identifier.to_string(),
        lower.clone(),
    ];
    for cand in &candidates {
        if let Some(info) = gio::DesktopAppInfo::new(cand) {
            let name = info.name().to_string();
            let icon = info
                .icon()
                .and_then(|i| IconExt::to_string(&i))
                .map(|s| s.to_string())
                .unwrap_or_else(|| identifier.to_string());
            let exec = safe_app_info_executable(&info)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| identifier.to_string());
            let matchers = vec![identifier.to_string(), lower.clone(), name.to_lowercase()];
            return (name, icon, exec, matchers);
        }
    }

    for app_info in gio::AppInfo::all() {
        let id_matches = app_info.id().is_some_and(|id| {
            id.to_lowercase()
                .trim_end_matches(".desktop")
                .contains(&lower)
        });
        let name_matches = app_info.name().to_lowercase() == lower;
        let exec_matches = safe_app_info_executable(&app_info)
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .is_some_and(|exe| {
                exe.eq_ignore_ascii_case(identifier) || exe.to_lowercase().contains(&lower)
            });
        if id_matches || name_matches || exec_matches {
            let name = app_info.name().to_string();
            let icon = app_info
                .icon()
                .and_then(|i| IconExt::to_string(&i))
                .map(|s| s.to_string())
                .unwrap_or_else(|| identifier.to_string());
            let exec = safe_app_info_executable(&app_info)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| identifier.to_string());
            let matchers = vec![identifier.to_string(), lower.clone(), name.to_lowercase()];
            return (name, icon, exec, matchers);
        }
    }

    let name = capitalize(identifier);
    let icon = identifier.to_string();
    let exec = identifier.to_string();
    let matchers = vec![identifier.to_string(), lower];
    (name, icon, exec, matchers)
}

fn dock_app_context_actions(
    app_cfg: &DockAppConfig,
    is_running: bool,
) -> Vec<(&'static str, &'static [&'static str])> {
    let identity = [
        app_cfg.name.as_str(),
        app_cfg.icon.as_str(),
        app_cfg.command.as_str(),
    ]
    .into_iter()
    .chain(app_cfg.matchers.iter().map(String::as_str))
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase();

    let mut actions = Vec::new();
    let is_branches = identity.contains("dev.malus.branches")
        || identity.contains("malus-de-branches")
        || identity
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|part| part == "branches");
    if identity.contains("nautilus") || (is_branches && is_running) {
        actions.push(("Abrir nueva ventana", &["--new-window"][..]));
    }

    if identity
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| part == "brave")
    {
        actions.push(("Abrir en modo incógnito", &["--incognito"][..]));
    }

    let is_code_editor = identity.contains("antigravity")
        || identity.contains("visual studio code")
        || identity.contains("vscode")
        || identity.contains("code-oss")
        || identity.contains("code-insiders")
        || identity
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|part| part == "code");
    if is_code_editor {
        actions.push(("Abrir nueva ventana vacía", &["--new-window"][..]));
    }

    actions
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn clear_dock_bounce_classes(widget: &gtk4::Widget) {
    widget.remove_css_class("dock-bounce-up");
    widget.remove_css_class("dock-bounce-down");
    widget.remove_css_class("dock-bounce-rebound-up");
    widget.remove_css_class("dock-bounce-rebound-down");
    widget.remove_css_class("dock-bounce-settle-up");
    widget.remove_css_class("dock-bounce-settle-down");
}

fn bounce_dock_icon(
    icon_container: &gtk4::Widget,
    animation_id: Rc<Cell<u64>>,
    running_cache: Option<Rc<RefCell<Vec<String>>>>,
    matchers: Vec<String>,
) {
    let current_id = animation_id.get().wrapping_add(1);
    animation_id.set(current_id);
    clear_dock_bounce_classes(icon_container);

    let target = icon_container.clone();
    let anim_id = animation_id.clone();

    glib::timeout_add_local_once(Duration::from_millis(1), move || {
        if anim_id.get() != current_id {
            return;
        }
        run_dock_bounce_cycle(target, anim_id, current_id, running_cache, matchers, 0);
    });
}

fn run_dock_bounce_cycle(
    target: gtk4::Widget,
    animation_id: Rc<Cell<u64>>,
    current_id: u64,
    cache: Option<Rc<RefCell<Vec<String>>>>,
    matchers: Vec<String>,
    cycle_count: u32,
) {
    if animation_id.get() != current_id {
        return;
    }

    // Fase 1: Impulso ascendente contra la gravedad (230ms con desaceleración hasta -24px)
    clear_dock_bounce_classes(&target);
    target.add_css_class("dock-bounce-up");

    let target_down = target.clone();
    let anim_id_down = animation_id.clone();
    let cache_down = cache.clone();
    let matchers_down = matchers.clone();

    glib::timeout_add_local_once(Duration::from_millis(230), move || {
        if anim_id_down.get() != current_id {
            return;
        }

        // Fase 2: Caída libre acelerada por gravedad hasta la base del dock (180ms hasta y=0)
        target_down.remove_css_class("dock-bounce-up");
        target_down.add_css_class("dock-bounce-down");

        let target_check = target_down.clone();
        let anim_id_check = anim_id_down.clone();

        glib::timeout_add_local_once(Duration::from_millis(180), move || {
            if anim_id_check.get() != current_id {
                return;
            }

            // Impacto firme con la base: verificar si la aplicación ya se encuentra en ejecución
            let is_running = if let Some(cache_ref) = &cache_down {
                let running = cache_ref.borrow();
                !matchers_down.is_empty()
                    && running.iter().any(|r| {
                        let r_lower = r.to_lowercase();
                        matchers_down
                            .iter()
                            .any(|m| r_lower.contains(&m.to_lowercase()))
                    })
            } else {
                true
            };

            // Permitir hasta 3 saltos completos si la aplicación continúa cargando
            if !is_running && cycle_count + 1 < 3 {
                run_dock_bounce_cycle(
                    target_check,
                    anim_id_check,
                    current_id,
                    cache_down,
                    matchers_down,
                    cycle_count + 1,
                );
            } else {
                run_dock_bounce_decay(target_check, anim_id_check, current_id);
            }
        });
    });
}

fn run_dock_bounce_decay(target: gtk4::Widget, animation_id: Rc<Cell<u64>>, current_id: u64) {
    if animation_id.get() != current_id {
        return;
    }

    // Fase 3: Rebote elástico secundario (135ms con desaceleración hasta -8.5px)
    clear_dock_bounce_classes(&target);
    target.add_css_class("dock-bounce-rebound-up");

    let target_reb_down = target.clone();
    let anim_id_reb_down = animation_id.clone();

    glib::timeout_add_local_once(Duration::from_millis(135), move || {
        if anim_id_reb_down.get() != current_id {
            return;
        }

        // Fase 4: Segunda caída libre acelerada hacia la superficie (115ms hasta y=0)
        target_reb_down.remove_css_class("dock-bounce-rebound-up");
        target_reb_down.add_css_class("dock-bounce-rebound-down");

        let target_settle = target_reb_down.clone();
        let anim_id_settle = anim_id_reb_down.clone();

        glib::timeout_add_local_once(Duration::from_millis(115), move || {
            if anim_id_settle.get() != current_id {
                return;
            }

            // Fase 5: Micro-asentamiento elástico (75ms hasta -2px)
            target_settle.remove_css_class("dock-bounce-rebound-down");
            target_settle.add_css_class("dock-bounce-settle-up");

            let target_final = target_settle.clone();
            let anim_id_final = anim_id_settle.clone();

            glib::timeout_add_local_once(Duration::from_millis(75), move || {
                if anim_id_final.get() != current_id {
                    return;
                }

                // Fase 6: Contacto final y descanso absoluto en la superficie (70ms hasta y=0)
                target_final.remove_css_class("dock-bounce-settle-up");
                target_final.add_css_class("dock-bounce-settle-down");

                let target_cleanup = target_final.clone();
                let anim_id_cleanup = anim_id_final.clone();

                glib::timeout_add_local_once(Duration::from_millis(70), move || {
                    if anim_id_cleanup.get() == current_id {
                        clear_dock_bounce_classes(&target_cleanup);
                    }
                });
            });
        });
    });
}

fn find_matching_running_app(
    running: &[String],
    matchers: &[String],
    app_name: &str,
) -> Option<String> {
    running
        .iter()
        .find(|identity| matches_dock_app(matchers, app_name, identity))
        .cloned()
}

fn matches_dock_app(matchers: &[String], app_name: &str, identity: &str) -> bool {
    let identity = identity.to_lowercase();
    matchers
        .iter()
        .filter(|matcher| !matcher.is_empty())
        .any(|matcher| identity.contains(&matcher.to_lowercase()))
        || (matchers.is_empty() && identity.contains(&app_name.to_lowercase()))
}

fn build_dock_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Malus DE Dock")
        .decorated(false)
        .css_classes(["malus-dock-window"])
        .build();

    window.set_decorated(false);

    // 1. Cargar configuración persistida desde dock.toml
    let dock_config = Rc::new(RefCell::new(DockConfig::load_or_create_default()));
    let initial_cfg = dock_config.borrow().clone();

    if gtk4_layer_shell::is_supported() {
        info!("Layer Shell soportado. Configurando Dock flotante centrado inferior...");
        window.init_layer_shell();
        if let Some(display) = gtk4::gdk::Display::default() {
            let displays_cfg = malus_de_common::config::DisplaysConfig::load();
            if let Some(primary_mon) = find_primary_gdk_monitor(&display, &displays_cfg) {
                window.set_monitor(&primary_mon);
            }
        }
        window.set_layer(Layer::Top);
        window.set_namespace("malus-dock");
        window.set_anchor(Edge::Bottom, true);
        window.set_margin(Edge::Bottom, initial_cfg.floating_gap as i32);
        window.auto_exclusive_zone_enable();
    } else {
        info!("Layer Shell no detectado; ejecutando Dock en ventana flotante de desarrollo.");
    }

    let dock_root = GtkBox::new(Orientation::Vertical, 0);
    dock_root.add_css_class("dock-root");
    dock_root.set_overflow(gtk4::Overflow::Visible);
    let magnification_headroom = GtkBox::new(Orientation::Horizontal, 0);
    magnification_headroom.set_size_request(-1, dock_headroom_height(&initial_cfg));
    dock_root.append(&magnification_headroom);

    let dock_bar = Overlay::builder()
        .valign(gtk4::Align::End)
        .halign(gtk4::Align::Center)
        .build();
    dock_bar.set_overflow(gtk4::Overflow::Visible);

    // Fondo del dock: contenedor acrílico + línea de brillo de bordes (en segundo plano absoluto)
    let dock_background = Overlay::new();
    dock_background.set_overflow(gtk4::Overflow::Visible);
    let dock_bg_box = GtkBox::builder()
        .css_classes(["dock-container"])
        .hexpand(true)
        .vexpand(true)
        .build();
    dock_bg_box.set_overflow(gtk4::Overflow::Visible);

    let dock_corner_radius = Rc::new(Cell::new(initial_cfg.dock_corner_radius() as f64));
    let highlights = DrawingArea::new();
    highlights.set_can_target(false);
    highlights.set_hexpand(true);
    highlights.set_vexpand(true);
    {
        let radius_draw = dock_corner_radius.clone();
        highlights.set_draw_func(move |_, context, width, height| {
            draw_dock_edge_highlights(
                context,
                width,
                height,
                radius_draw.get(),
                DOCK_CURRENT_POSITION.get(),
            );
        });
    }
    dock_background.set_child(Some(&dock_bg_box));
    dock_background.add_overlay(&highlights);
    dock_background.set_measure_overlay(&highlights, false);

    dock_bar.set_child(Some(&dock_background));

    // Capa frontal de contenido: contiene apps_box, separator, miniaturas y papelera.
    // Al ser el overlay superior de dock_bar, TODOS los iconos y sus contenedores
    // se dibujan por encima del fondo y de la línea de brillo, garantizando que nunca se corten
    // al hacer el efecto zoom o al saltar en rebote.
    let dock_content = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["dock-items"])
        .spacing(4)
        .build();
    dock_content.set_overflow(gtk4::Overflow::Visible);

    // Contenedor modular de aplicaciones (fijadas y dinámicas en ejecución)
    let apps_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(2)
        .valign(gtk4::Align::End)
        .build();
    apps_box.set_overflow(gtk4::Overflow::Visible);
    apps_box.set_halign(gtk4::Align::Start);
    apps_box.set_margin_bottom(1);

    dock_content.append(&apps_box);

    let apps_placeholder = GtkBox::new(Orientation::Horizontal, 0);

    // Separador de cristal translúcido
    let separator = Separator::new(Orientation::Vertical);
    separator.add_css_class("dock-separator");
    separator.set_valign(gtk4::Align::End);
    dock_content.append(&separator);

    // Contenedor dinámico de miniaturas de ventanas minimizadas (Bloque Derecho)
    let minimized_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .valign(gtk4::Align::End)
        .build();
    minimized_box.set_overflow(gtk4::Overflow::Visible);
    minimized_box.set_visible(false);
    dock_content.append(&minimized_box);

    // Papelera de Reciclaje (Trash) con estado reactivo y menú contextual
    let (trash_widget, trash_img) = create_trash_item(initial_cfg.icon_size as i32);
    dock_content.append(&trash_widget);

    dock_bar.add_overlay(&dock_content);
    dock_bar.set_measure_overlay(&dock_content, true);
    dock_bar.set_clip_overlay(&dock_content, false);

    let edge_sensor = GtkBox::new(Orientation::Horizontal, 0);
    edge_sensor.set_visible(false);

    dock_root.append(&dock_bar);
    dock_root.append(&edge_sensor);

    let dock_hovered = Rc::new(Cell::new(false));
    let dock_edge_active = Rc::new(Cell::new(false));
    let auto_hide_revealed = Rc::new(Cell::new(!initial_cfg.auto_hide));
    let icon_size_animations = Rc::new(RefCell::new(Vec::<IconSizeAnimation>::new()));
    let icon_size_animation_running = Rc::new(Cell::new(false));
    let layout_refs = Rc::new(DockLayoutRefs {
        window: window.clone(),
        dock_root: dock_root.clone(),
        dock_bar: dock_bar.clone(),
        edge_sensor: edge_sensor.clone(),
        magnification_headroom: magnification_headroom.clone(),
        dock_content: dock_content.clone(),
        apps_box: apps_box.clone(),
        separator: separator.clone(),
        minimized_box: minimized_box.clone(),
        dock_highlights: highlights.clone(),
        trash_widget: trash_widget.clone(),
        dock_corner_radius,
    });
    apply_dock_layout_config(
        &layout_refs,
        &initial_cfg,
        auto_hide_revealed.get(),
        false,
    );

    // Ventana modal Overlay del Selector Visual de Aplicaciones (⌘Tab App Switcher HUD)
    let switcher_window = Window::builder()
        .title("Malus DE App Switcher")
        .css_classes(["malus-switcher-window"])
        .build();

    if gtk4_layer_shell::is_supported() {
        switcher_window.init_layer_shell();
        switcher_window.set_layer(Layer::Overlay);
        switcher_window.set_namespace("malus-app-switcher");
        switcher_window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::None);
        switcher_window.set_exclusive_zone(0);
    } else {
        switcher_window.set_decorated(false);
    }

    let switcher_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["switcher-container"])
        .spacing(8)
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .build();

    let win_dismiss_click = GestureClick::new();
    win_dismiss_click.connect_released(|_, _, _, _| {
        let _ = send_ipc_message(&IpcMessage::HideAppSwitcher);
    });
    switcher_box.set_visible(false);
    let switcher_background = DrawingArea::new();
    switcher_background.add_css_class("switcher-container-background");
    switcher_background.set_can_target(false);
    switcher_background.set_draw_func(|area, context, width, height| {
        draw_switcher_squircle(
            area,
            context,
            width as f64,
            height as f64,
            32.0,
            false,
            true,
        );
    });
    let switcher_surface = Overlay::new();
    switcher_surface.add_css_class("switcher-surface");
    switcher_surface.set_child(Some(&switcher_background));
    switcher_surface.add_overlay(&switcher_box);
    switcher_surface.set_measure_overlay(&switcher_box, true);
    switcher_window.set_child(Some(&switcher_surface));

    // Canales y estado compartido
    let running_cache = Rc::new(RefCell::new(Vec::<String>::new()));
    let active_app_cache = Rc::new(RefCell::new(Vec::<String>::new()));
    let minimized_cache: Rc<RefCell<Vec<malus_de_common::ipc::MinimizedWindowInfo>>> =
        Rc::new(RefCell::new(Vec::new()));
    let dock_indicators: Rc<RefCell<Vec<(Vec<String>, Label)>>> = Rc::new(RefCell::new(Vec::new()));

    // Canal de actualización del layout de aplicaciones del Dock
    let (refresh_tx, refresh_rx) = async_channel::unbounded::<bool>();
    let r_tx = refresh_tx.clone();
    let on_refresh: Rc<dyn Fn()> = Rc::new(move || {
        let _ = r_tx.send_blocking(true);
    });
    let running_tx = refresh_tx.clone();
    let on_running_refresh: Rc<dyn Fn()> = Rc::new(move || {
        let _ = running_tx.send_blocking(false);
    });

    // Render inicial de aplicaciones desde dock.toml
    render_dock_apps(
        &apps_box,
        &dock_config,
        &running_cache,
        &active_app_cache,
        &minimized_cache,
        &dock_indicators,
        &on_refresh,
        true,
    );
    sync_dock_apps_placeholder(&apps_box, &apps_placeholder);
    let placeholder_width_animation = Rc::new(RefCell::new(DockPlaceholderWidthAnimation {
        current_width: apps_placeholder.width_request().max(0) as f64,
        target_width: apps_placeholder.width_request().max(0) as f64,
    }));
    let placeholder_width_animation_running = Rc::new(Cell::new(false));

    let apps_box_theme = apps_box.clone();
    let dock_root_theme = dock_root.clone();
    let win_theme = window.clone();
    adw::StyleManager::default().connect_dark_notify(move |_| {
        queue_dock_icon_backgrounds(apps_box_theme.upcast_ref());
        dock_root_theme.queue_draw();
        win_theme.queue_draw();
    });

    // Loop de refresco reactivo de las aplicaciones
    let apps_box_rf = apps_box.clone();
    let dock_cfg_rf = dock_config.clone();
    let run_cache_rf = running_cache.clone();
    let active_app_rf = active_app_cache.clone();
    let minimized_rf = minimized_cache.clone();
    let indicators_rf = dock_indicators.clone();
    let on_rf_loop = on_refresh.clone();
    let dock_bar_resize = dock_bar.clone();
    let apps_placeholder_resize = apps_placeholder.clone();
    let dock_root_resize = dock_root.clone();
    let win_resize = window.clone();
    let placeholder_animation_rf = placeholder_width_animation.clone();
    let placeholder_animation_running_rf = placeholder_width_animation_running.clone();

    let drop_placeholder: Rc<RefCell<Option<GtkBox>>> = Rc::new(RefCell::new(None));
    let drop_placeholder_ipc = drop_placeholder.clone();
    let drop_placeholder_rf = drop_placeholder.clone();

    attach_dock_drop_target(
        &window,
        &dock_root,
        &dock_bar,
        &apps_box,
        &trash_widget,
        &trash_img,
        &dock_config,
        &drop_placeholder,
        &on_refresh,
    );

    glib::spawn_future_local(async move {
        while let Ok(refresh_pinned) = refresh_rx.recv().await {
            *drop_placeholder_rf.borrow_mut() = None;
            render_dock_apps(
                &apps_box_rf,
                &dock_cfg_rf,
                &run_cache_rf,
                &active_app_rf,
                &minimized_rf,
                &indicators_rf,
                &on_rf_loop,
                refresh_pinned,
            );
            start_dock_placeholder_width_animation(
                &placeholder_animation_rf,
                &placeholder_animation_running_rf,
                &win_resize,
                &dock_root_resize,
                &dock_bar_resize,
                &apps_box_rf,
                &apps_placeholder_resize,
            );
            queue_dock_resize(&win_resize, &dock_root_resize, &dock_bar_resize);
            if dock_cfg_rf.borrow().minimize_to_app_icon {
                let ind_guard = indicators_rf.borrow();
                for win_info in minimized_rf.borrow().iter() {
                    if let Some(app_widget) = find_app_icon_widget_for_window(
                        &ind_guard,
                        &win_info.app_id,
                        &win_info.title,
                    ) {
                        register_minimized_dock_slot(&app_widget, &win_resize, win_info.window_id);
                    }
                }
            }
        }
    });

    // Monitoreo reactivo de cambios en dock.toml en disco (GFileMonitor)
    let config_path = DockConfig::config_path();
    let gfile = gio::File::for_path(&config_path);
    let min_map: Rc<RefCell<HashMap<u64, gtk4::Widget>>> = Rc::new(RefCell::new(HashMap::new()));
    let min_paths: Rc<RefCell<HashMap<u64, Option<String>>>> =
        Rc::new(RefCell::new(HashMap::new()));
    if let Ok(monitor) = gfile.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
        let cfg_clone = dock_config.clone();
        let on_rf_mon = on_refresh.clone();
        let layout_mon = layout_refs.clone();
        let auto_hide_mon = auto_hide_revealed.clone();
        let hovered_mon = dock_hovered.clone();
        let edge_mon = dock_edge_active.clone();
        let indicators_mon = dock_indicators.clone();
        let running_mon = running_cache.clone();
        let min_cache_mon = minimized_cache.clone();
        let min_map_mon = min_map.clone();
        let min_paths_mon = min_paths.clone();
        let min_box_mon = minimized_box.clone();
        let win_mon = window.clone();
        let root_mon = dock_root.clone();
        let bar_mon = dock_bar.clone();
        let anims_mon = icon_size_animations.clone();
        monitor.connect_changed(move |_, _, _, event_type| {
            if event_type == gio::FileMonitorEvent::ChangesDoneHint
                || event_type == gio::FileMonitorEvent::Created
            {
                let new_cfg = DockConfig::load_or_create_default();
                let old_cfg = cfg_clone.borrow().clone();
                if new_cfg == old_cfg {
                    return;
                }
                info!("🔄 dock.toml modificado externamente en disco. Recargando...");
                let base_size = new_cfg.icon_size as i32;
                let rebuild_needed = old_cfg.apps != new_cfg.apps
                    || old_cfg.show_recent_apps != new_cfg.show_recent_apps
                    || old_cfg.indicator_style != new_cfg.indicator_style;
                if !new_cfg.auto_hide {
                    auto_hide_mon.set(true);
                } else {
                    auto_hide_mon.set(hovered_mon.get() || edge_mon.get());
                }
                let min_to_icon = new_cfg.minimize_to_app_icon;
                apply_dock_layout_config(
                    &layout_mon,
                    &new_cfg,
                    auto_hide_mon.get(),
                    !min_cache_mon.borrow().is_empty(),
                );
                if old_cfg.position != new_cfg.position {
                    anims_mon.borrow_mut().clear();
                }
                sync_dock_indicators(
                    &indicators_mon,
                    &running_mon.borrow(),
                    new_cfg.show_indicators,
                );
                *cfg_clone.borrow_mut() = new_cfg;
                if rebuild_needed {
                    on_rf_mon();
                } else {
                    apply_dock_icons_size_in_place(
                        &layout_mon,
                        base_size,
                        &anims_mon,
                        hovered_mon.get(),
                    );
                }
                sync_minimized_windows_ui(
                    &min_cache_mon.borrow(),
                    min_to_icon,
                    base_size,
                    &min_box_mon,
                    &min_map_mon,
                    &min_paths_mon,
                    &indicators_mon,
                    &win_mon,
                    &root_mon,
                    &bar_mon,
                );
            }
        });
    }

    // Magnificación Analítica macOS (Efecto zoom suave con curva gaussiana)
    let apps_box_motion = apps_box.clone();
    let min_box_motion = minimized_box.clone();
    let trash_widget_motion = trash_widget.clone();
    let dock_bar_motion = dock_bar.clone();
    let apps_placeholder_motion = apps_placeholder.clone();
    let dock_root_motion = dock_root.clone();
    let window_motion = window.clone();
    let cfg_motion = dock_config.clone();
    let icon_size_animations_motion = icon_size_animations.clone();
    let icon_size_animation_running_motion = icon_size_animation_running.clone();

    let motion_controller = EventControllerMotion::new();
    let hovered_enter = dock_hovered.clone();
    let auto_hide_enter = auto_hide_revealed.clone();
    let cfg_enter = dock_config.clone();
    let layout_enter = layout_refs.clone();
    let min_cache_enter = minimized_cache.clone();
    motion_controller.connect_enter(move |_, _, _| {
        hovered_enter.set(true);
        let cfg = cfg_enter.borrow();
        if cfg.auto_hide && !auto_hide_enter.get() {
            auto_hide_enter.set(true);
            apply_dock_layout_config(
                &layout_enter,
                &cfg,
                true,
                !min_cache_enter.borrow().is_empty(),
            );
        }
    });
    let hovered_motion = dock_hovered.clone();
    let auto_hide_motion = auto_hide_revealed.clone();
    let layout_motion = layout_refs.clone();
    let min_cache_motion = minimized_cache.clone();
    motion_controller.connect_motion(move |_, x, y| {
        hovered_motion.set(true);
        let cfg = cfg_motion.borrow();
        if cfg.auto_hide && !auto_hide_motion.get() {
            auto_hide_motion.set(true);
            apply_dock_layout_config(
                &layout_motion,
                &cfg,
                true,
                !min_cache_motion.borrow().is_empty(),
            );
        }
        if !cfg.magnification_enabled {
            return;
        }
        let Some(pointer_in_dock) = dock_root_motion.compute_point(
            &dock_bar_motion,
            &gtk4::graphene::Point::new(x as f32, y as f32),
        ) else {
            return;
        };
        let is_horizontal = cfg.position == DockPosition::Bottom;
        let pointer_coord = if is_horizontal {
            pointer_in_dock.x() as f64
        } else {
            pointer_in_dock.y() as f64
        };
        let base_size = cfg.icon_size as f64;
        let max_size = cfg.magnification_max_size as f64;
        let max_scale_diff = (max_size - base_size).max(0.0);
        let max_zoom_dist = 110.0;
        let mut targets = Vec::new();

        let mut update_item = |widget: &gtk4::Widget| {
            if let Some(p) = widget.compute_point(&dock_bar_motion, &gtk4::graphene::Point::zero())
            {
                let item_center = if is_horizontal {
                    p.x() as f64 + widget.width() as f64 / 2.0
                } else {
                    p.y() as f64 + widget.height() as f64 / 2.0
                };
                let dist = (pointer_coord - item_center).abs();
                let factor = if dist < max_zoom_dist {
                    let norm = dist / max_zoom_dist;
                    (-norm * norm * 2.2).exp()
                } else {
                    0.0
                };
                let target_size = (base_size + factor * max_scale_diff) as i32;
                targets.push((widget.clone(), target_size, base_size as i32));
            }
        };

        // Magnificar cada app en apps_box
        let mut child = apps_box_motion.first_child();
        while let Some(c) = child {
            update_item(&c);
            child = c.next_sibling();
        }

        // Magnificar cada miniatura en minimized_box si está visible y activa
        if min_box_motion.is_visible() {
            let mut min_child = min_box_motion.first_child();
            while let Some(c) = min_child {
                let is_closing = c.opacity() <= 0.01
                    || c.clone()
                        .downcast::<gtk4::Revealer>()
                        .is_ok_and(|r| !r.reveals_child());
                if !is_closing {
                    update_item(&c);
                }
                min_child = c.next_sibling();
            }
        }

        // Magnificar papelera
        update_item(trash_widget_motion.upcast_ref());
        update_icon_size_animation_targets(&icon_size_animations_motion, targets);
        start_icon_size_animation(
            &icon_size_animations_motion,
            &icon_size_animation_running_motion,
            &window_motion,
            &dock_root_motion,
            &dock_bar_motion,
            &apps_box_motion,
            &apps_placeholder_motion,
        );
    });

    let cfg_leave = dock_config.clone();
    let apps_box_leave = apps_box.clone();
    let apps_placeholder_leave = apps_placeholder.clone();
    let icon_size_animations_leave = icon_size_animations.clone();
    let icon_size_animation_running_leave = icon_size_animation_running.clone();
    let window_leave = window.clone();
    let dock_root_leave = dock_root.clone();
    let dock_bar_leave = dock_bar.clone();

    let hovered_leave = dock_hovered.clone();
    let edge_leave = dock_edge_active.clone();
    let auto_hide_leave = auto_hide_revealed.clone();
    let cfg_leave_ah = dock_config.clone();
    let layout_leave = layout_refs.clone();
    let min_cache_leave = minimized_cache.clone();
    let dock_content_leave = dock_content.clone();
    motion_controller.connect_leave(move |_| {
        hovered_leave.set(false);
        let base_size = cfg_leave.borrow().icon_size as i32;
        let mut animations = icon_size_animations_leave.borrow_mut();
        for animation in animations.iter_mut() {
            animation.target_size = base_size as f64;
            animation.base_size = base_size;
        }
        drop(animations);
        start_icon_size_animation(
            &icon_size_animations_leave,
            &icon_size_animation_running_leave,
            &window_leave,
            &dock_root_leave,
            &dock_bar_leave,
            &apps_box_leave,
            &apps_placeholder_leave,
        );

        schedule_auto_hide_check(
            320,
            hovered_leave.clone(),
            edge_leave.clone(),
            auto_hide_leave.clone(),
            cfg_leave_ah.clone(),
            layout_leave.clone(),
            min_cache_leave.clone(),
            dock_content_leave.clone(),
        );
    });
    dock_root.add_controller(motion_controller);
    setup_dock_apps_drag_controller(&apps_box, &dock_config, &on_refresh);

    // Escucha de eventos IPC del compositor
    let (tx, rx) = async_channel::unbounded::<IpcMessage>();
    let cache_ipc = running_cache.clone();
    let apps_box_ipc = apps_box.clone();
    let min_box = minimized_box.clone();
    let dock_bar_ref = dock_bar.clone();
    let dock_root_ref = dock_root.clone();
    let win_ref = window.clone();
    let indicators_ipc = dock_indicators.clone();
    let active_app_ipc = active_app_cache.clone();
    let minimized_ipc = minimized_cache.clone();
    let cfg_ipc = dock_config.clone();
    let on_rf_ipc = on_refresh.clone();
    let on_rf_running_ipc = on_running_refresh.clone();
    let refresh_pending_ipc = Rc::new(Cell::new(false));
    let switcher_win_ipc = switcher_window.clone();
    let switcher_surface_ipc = switcher_surface.clone();
    let switcher_bg_ipc = switcher_background.clone();
    let switcher_box_ipc = switcher_box.clone();
    let switcher_cards_ipc: Rc<RefCell<Vec<(DrawingArea, Rc<std::cell::Cell<bool>>)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let switcher_cards_show = switcher_cards_ipc.clone();

    let layout_ipc = layout_refs.clone();
    let auto_hide_ipc = auto_hide_revealed.clone();
    let hovered_ipc = dock_hovered.clone();
    let edge_ipc = dock_edge_active.clone();
    let dock_content_ipc = dock_content.clone();
    let anims_ipc = icon_size_animations.clone();
    glib::spawn_future_local(async move {
        while let Ok(msg) = rx.recv().await {
            match msg {
                IpcMessage::DockAutoHideEdge { active } => {
                    edge_ipc.set(active);
                    let cfg = cfg_ipc.borrow();
                    if cfg.auto_hide {
                        if active && !auto_hide_ipc.get() {
                            auto_hide_ipc.set(true);
                            apply_dock_layout_config(
                                &layout_ipc,
                                &cfg,
                                true,
                                !minimized_ipc.borrow().is_empty(),
                            );
                        } else if !active && auto_hide_ipc.get() && !hovered_ipc.get() {
                            schedule_auto_hide_check(
                                260,
                                hovered_ipc.clone(),
                                edge_ipc.clone(),
                                auto_hide_ipc.clone(),
                                cfg_ipc.clone(),
                                layout_ipc.clone(),
                                minimized_ipc.clone(),
                                dock_content_ipc.clone(),
                            );
                        }
                    }
                }
                IpcMessage::ActiveAppChanged { app_id, title, .. } => {
                    let mut active_app = active_app_ipc.borrow_mut();
                    active_app.clear();
                    active_app.extend(app_id.into_iter().chain(title));
                }
                IpcMessage::RunningAppsChanged { running_apps } => {
                    let prev_apps = cache_ipc.borrow().clone();
                    *cache_ipc.borrow_mut() = running_apps.clone();

                    // 1. Actualizar indicadores de las apps visibles
                    let show_ind = cfg_ipc.borrow().show_indicators;
                    sync_dock_indicators(&indicators_ipc, &running_apps, show_ind);

                    // 2. Si la lista de aplicaciones ejecutándose cambió, refrescar apps unpinned
                    if prev_apps != running_apps && !refresh_pending_ipc.replace(true) {
                        let on_refresh = on_rf_running_ipc.clone();
                        let refresh_pending = refresh_pending_ipc.clone();
                        glib::timeout_add_local_once(Duration::from_millis(900), move || {
                            refresh_pending.set(false);
                            on_refresh();
                        });
                    }
                }
                IpcMessage::PinAppToDock {
                    name,
                    icon,
                    command,
                    matchers,
                    insert_at_dock_x,
                } => {
                    info!(
                        "📌 IPC PinAppToDock recibido para: {} (dock_x={:?})",
                        name, insert_at_dock_x
                    );
                    if let Some(placeholder) = drop_placeholder_ipc.borrow_mut().take() {
                        if placeholder.parent().as_ref() == Some(apps_box_ipc.as_ref()) {
                            apps_box_ipc.remove(&placeholder);
                        }
                    }
                    let app_cfg = DockAppConfig {
                        name,
                        icon,
                        command,
                        matchers,
                        icon_appearance: DockIconAppearance::Auto,
                    };
                    let mut cfg = cfg_ipc.borrow_mut();
                    let target_idx = insert_at_dock_x.and_then(|dock_x| {
                        win_ref
                            .compute_point(
                                &apps_box_ipc,
                                &gtk4::graphene::Point::new(dock_x as f32, 0.0),
                            )
                            .map(|pt| {
                                let coord = if apps_box_ipc.orientation() == Orientation::Horizontal
                                {
                                    pt.x() as f64
                                } else {
                                    pt.y() as f64
                                };
                                compute_pinned_insert_index(
                                    &apps_box_ipc,
                                    None,
                                    cfg.apps.len(),
                                    coord,
                                )
                            })
                    });
                    if cfg.pin_app_at(app_cfg, target_idx) {
                        let _ = cfg.save();
                        drop(cfg);
                        on_rf_ipc();
                    }
                }
                IpcMessage::PreviewDockDropGap { dock_x } => {
                    let local_x = dock_x.and_then(|x| {
                        win_ref
                            .compute_point(
                                &apps_box_ipc,
                                &gtk4::graphene::Point::new(x as f32, 0.0),
                            )
                            .map(|p| p.x() as f64)
                    });
                    if dock_x.is_some() {
                        update_dock_drop_placeholder(
                            &apps_box_ipc,
                            &drop_placeholder_ipc,
                            &cfg_ipc,
                            local_x,
                        );
                    } else {
                        clear_dock_drop_placeholder(&apps_box_ipc, &drop_placeholder_ipc);
                    }
                }
                IpcMessage::UnpinAppFromDock { name } => {
                    info!("📌 IPC UnpinAppFromDock recibido para: {}", name);
                    let mut cfg = cfg_ipc.borrow_mut();
                    if cfg.unpin_app(&name) {
                        let _ = cfg.save();
                        drop(cfg);
                        on_rf_ipc();
                    }
                }
                IpcMessage::ReloadDockConfig => {
                    info!("🔄 IPC ReloadDockConfig recibido");
                    let new_cfg = DockConfig::load_or_create_default();
                    let old_cfg = cfg_ipc.borrow().clone();
                    let base_size = new_cfg.icon_size as i32;
                    let rebuild_needed = old_cfg.apps != new_cfg.apps
                        || old_cfg.show_recent_apps != new_cfg.show_recent_apps
                        || old_cfg.indicator_style != new_cfg.indicator_style;
                    if !new_cfg.auto_hide {
                        auto_hide_ipc.set(true);
                    } else {
                        auto_hide_ipc.set(hovered_ipc.get() || edge_ipc.get());
                    }
                    let has_min = !minimized_ipc.borrow().is_empty();
                    let position_changed = old_cfg.position != new_cfg.position;
                    let new_pos = new_cfg.position;
                    let min_to_icon = new_cfg.minimize_to_app_icon;
                    apply_dock_layout_config(&layout_ipc, &new_cfg, auto_hide_ipc.get(), has_min);
                    if position_changed {
                        anims_ipc.borrow_mut().clear();
                    }
                    sync_dock_indicators(
                        &indicators_ipc,
                        &cache_ipc.borrow(),
                        new_cfg.show_indicators,
                    );
                    *cfg_ipc.borrow_mut() = new_cfg;
                    for widget in min_map.borrow().values() {
                        update_icon_size_recursive(widget, base_size, base_size, new_pos);
                    }
                    if rebuild_needed {
                        update_icon_size_recursive(
                            layout_ipc.trash_widget.upcast_ref(),
                            base_size,
                            base_size,
                            new_pos,
                        );
                        queue_dock_resize(&win_ref, &dock_root_ref, &dock_bar_ref);
                        on_rf_ipc();
                    } else {
                        apply_dock_icons_size_in_place(
                            &layout_ipc,
                            base_size,
                            &anims_ipc,
                            hovered_ipc.get(),
                        );
                    }
                    sync_minimized_windows_ui(
                        &minimized_ipc.borrow(),
                        min_to_icon,
                        base_size,
                        &min_box,
                        &min_map,
                        &min_paths,
                        &indicators_ipc,
                        &win_ref,
                        &dock_root_ref,
                        &dock_bar_ref,
                    );
                }
                IpcMessage::MinimizedWindowsChanged { windows } => {
                    *minimized_ipc.borrow_mut() = windows.clone();
                    let cfg_now = cfg_ipc.borrow();
                    let min_to_icon = cfg_now.minimize_to_app_icon;
                    let base_size = cfg_now.icon_size as i32;
                    drop(cfg_now);
                    sync_minimized_windows_ui(
                        &windows,
                        min_to_icon,
                        base_size,
                        &min_box,
                        &min_map,
                        &min_paths,
                        &indicators_ipc,
                        &win_ref,
                        &dock_root_ref,
                        &dock_bar_ref,
                    );
                }
                IpcMessage::ShowAppSwitcher {
                    items,
                    selected_index,
                } => {
                    while let Some(c) = switcher_box_ipc.first_child() {
                        switcher_box_ipc.remove(&c);
                    }
                    switcher_cards_show.borrow_mut().clear();
                    for (i, item) in items.iter().enumerate() {
                        let content = GtkBox::builder()
                            .orientation(Orientation::Vertical)
                            .spacing(6)
                            .margin_top(18)
                            .margin_bottom(18)
                            .margin_start(20)
                            .margin_end(20)
                            .halign(gtk4::Align::Center)
                            .valign(gtk4::Align::Center)
                            .build();
                        let selected = Rc::new(std::cell::Cell::new(i == selected_index));
                        let card_background = DrawingArea::new();
                        card_background.add_css_class("switcher-card-background");
                        card_background.set_can_target(false);
                        let selected_for_draw = selected.clone();
                        card_background.set_draw_func(move |area, context, width, height| {
                            draw_switcher_squircle(
                                area,
                                context,
                                width as f64,
                                height as f64,
                                22.0,
                                selected_for_draw.get(),
                                false,
                            );
                        });
                        let card = Overlay::new();
                        card.add_css_class("switcher-card");
                        if selected.get() {
                            card.add_css_class("selected");
                        }
                        card.set_child(Some(&card_background));
                        card.add_overlay(&content);
                        card.set_measure_overlay(&content, true);

                        let icon_name = item
                            .icon_name
                            .as_deref()
                            .unwrap_or("application-x-executable");
                        let icon = Image::builder()
                            .icon_name(icon_name)
                            .pixel_size(64)
                            .halign(gtk4::Align::Center)
                            .build();
                        content.append(&icon);

                        let title_label = Label::builder()
                            .label(&item.title)
                            .css_classes(["switcher-title"])
                            .ellipsize(gtk4::pango::EllipsizeMode::End)
                            .max_width_chars(14)
                            .halign(gtk4::Align::Center)
                            .build();
                        content.append(&title_label);

                        // Conmutar a la ventana al hacer clic sobre su tarjeta
                        let target_wid = item.window_id;
                        let card_click = GestureClick::new();
                        card_click.connect_released(move |_, _, _, _| {
                            let _ = send_ipc_message(&IpcMessage::SwitchToWindow {
                                window_id: target_wid,
                            });
                        });
                        card.add_controller(card_click);

                        switcher_box_ipc.append(&card);
                        switcher_cards_show
                            .borrow_mut()
                            .push((card_background, selected));
                    }
                    switcher_box_ipc.set_visible(true);
                    resize_switcher_window(
                        &switcher_win_ipc,
                        &switcher_surface_ipc,
                        &switcher_box_ipc,
                        &switcher_bg_ipc,
                    );
                    if !switcher_win_ipc.is_visible() {
                        switcher_win_ipc.set_visible(true);
                    }
                    switcher_win_ipc.present();
                }
                IpcMessage::UpdateAppSwitcher { selected_index } => {
                    for (index, (background, selected)) in
                        switcher_cards_show.borrow().iter().enumerate()
                    {
                        selected.set(index == selected_index);
                        background.queue_draw();
                    }
                    let mut idx = 0;
                    let mut child = switcher_box_ipc.first_child();
                    while let Some(c) = child {
                        if idx == selected_index {
                            c.add_css_class("selected");
                        } else {
                            c.remove_css_class("selected");
                        }
                        idx += 1;
                        child = c.next_sibling();
                    }
                }
                IpcMessage::HideAppSwitcher => {
                    switcher_box_ipc.set_visible(false);
                    switcher_cards_show.borrow_mut().clear();
                    while let Some(c) = switcher_box_ipc.first_child() {
                        switcher_box_ipc.remove(&c);
                    }
                    // No llamar set_visible(false) sobre una LayerSurface de gtk4-layer-shell:
                    // GTK4 envía wl_surface.attach(nil)+commit() tras destruir zwlr_layer_surface_v1,
                    // y Smithay 0.7.0 dispara Error::InvalidSize cerrando todo el cliente (Dock + Switcher).
                    switcher_win_ipc.set_default_size(1, 1);
                    switcher_win_ipc.queue_resize();
                    switcher_bg_ipc.queue_draw();
                }
                IpcMessage::ThemeChanged { is_dark, .. } => {
                    info!("🎨 [Dock] IPC ThemeChanged recibido: is_dark={}", is_dark);
                    adw::StyleManager::default().set_color_scheme(if is_dark {
                        adw::ColorScheme::ForceDark
                    } else {
                        adw::ColorScheme::ForceLight
                    });
                    queue_dock_icon_backgrounds(apps_box_ipc.upcast_ref());
                    dock_root_ref.queue_draw();
                    win_ref.queue_draw();
                }
                IpcMessage::AccentColorChanged { index } => {
                    info!("🎨 [Dock] IPC AccentColorChanged recibido: index={}", index);
                    apply_dock_accent_color(index);
                    queue_dock_icon_backgrounds(apps_box_ipc.upcast_ref());
                    dock_root_ref.queue_draw();
                    win_ref.queue_draw();
                }
                IpcMessage::LiquidGlassOpacityChanged { opacity } => {
                    info!(
                        "✨ [Dock] IPC LiquidGlassOpacityChanged recibido: opacity={:.2}",
                        opacity
                    );
                    apply_dock_liquid_glass_opacity(opacity);
                    switcher_bg_ipc.queue_draw();
                    dock_root_ref.queue_draw();
                    win_ref.queue_draw();
                }
                IpcMessage::ReloadDisplaysConfig => {
                    info!("🖥 [Dock] IPC ReloadDisplaysConfig recibido — reajustando dock y miniaturas");
                    queue_dock_resize(&win_ref, &dock_root_ref, &dock_bar_ref);
                    queue_dock_icon_backgrounds(apps_box_ipc.upcast_ref());
                    dock_root_ref.queue_draw();
                    win_ref.queue_draw();
                    let cards: Vec<(u64, gtk4::Widget)> = min_map
                        .borrow()
                        .iter()
                        .map(|(id, widget)| (*id, widget.clone()))
                        .collect();
                    let dock_window_weak = win_ref.downgrade();
                    for delay in [60, 180] {
                        let cards = cards.clone();
                        let dock_window_weak = dock_window_weak.clone();
                        glib::timeout_add_local_once(Duration::from_millis(delay), move || {
                            if let Some(dock_window) = dock_window_weak.upgrade() {
                                for (window_id, card) in cards {
                                    register_minimized_dock_slot(&card, &dock_window, window_id);
                                }
                            }
                        });
                    }
                }
                _ => {}
            }
        }
    });

    // Hilo lector del socket IPC del compositor
    std::thread::spawn(move || {
        let sock_path = ipc_socket_path();
        loop {
            if let Ok(stream) = std::os::unix::net::UnixStream::connect(&sock_path) {
                let reader = std::io::BufReader::new(stream);
                for line in std::io::BufRead::lines(reader) {
                    if let Ok(line) = line {
                        if let Ok(msg) = serde_json::from_str::<IpcMessage>(&line) {
                            let _ = tx.send_blocking(msg);
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });

    // Monitor reactivo de papelera (GVFS monitor en vez de polling cada 3 s en hilo GTK)
    let trash_img_ref = trash_img;
    let last_trash_full: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
    let sync_trash_icon: Rc<dyn Fn()> = {
        let img = trash_img_ref.clone();
        let last = last_trash_full.clone();
        Rc::new(move || {
            let is_full = check_trash_full();
            if last.get() != Some(is_full) {
                last.set(Some(is_full));
                img.set_icon_name(Some(if is_full {
                    "user-trash-full"
                } else {
                    "user-trash"
                }));
            }
        })
    };
    sync_trash_icon();
    let trash_file = gtk4::gio::File::for_uri("trash:///");
    match trash_file.monitor_directory(
        gtk4::gio::FileMonitorFlags::NONE,
        gtk4::gio::Cancellable::NONE,
    ) {
        Ok(monitor) => {
            let sync_cb = sync_trash_icon.clone();
            monitor.connect_changed(move |_, _, _, _| {
                sync_cb();
            });
            // Mantener vivo el monitor durante toda la vida de la app
            std::mem::forget(monitor);
        }
        Err(_) => {
            let sync_fallback = sync_trash_icon.clone();
            glib::timeout_add_local(std::time::Duration::from_secs(15), move || {
                sync_fallback();
                glib::ControlFlow::Continue
            });
        }
    }

    if let Some(display) = gtk4::gdk::Display::default() {
        let win_mon = window.clone();
        let dock_root_mon = dock_root.clone();
        let dock_bar_mon = dock_bar.clone();
        display.monitors().connect_items_changed(move |_, _, _, _| {
            queue_dock_resize(&win_mon, &dock_root_mon, &dock_bar_mon);
            dock_root_mon.queue_draw();
            win_mon.queue_draw();
        });
    }

    window.set_child(Some(&dock_root));
    DOCK_REPAINT_TARGETS.with(|cell| {
        use glib::object::ObjectExt;
        *cell.borrow_mut() = Some((
            apps_box.downgrade(),
            dock_root.downgrade(),
            window.downgrade(),
        ));
    });
    let ah_map = auto_hide_revealed.clone();
    let cfg_map = dock_config.clone();
    window.connect_map(move |win| {
        if cfg_map.borrow().auto_hide && !ah_map.get() {
            update_dock_surface_input_region(win, false, 1, 1);
        }
    });
    window.present();
    if initial_cfg.auto_hide && !auto_hide_revealed.get() {
        update_dock_surface_input_region(&window, false, 1, 1);
    }
}

fn find_primary_gdk_monitor(
    display: &gtk4::gdk::Display,
    cfg: &malus_de_common::config::DisplaysConfig,
) -> Option<gtk4::gdk::Monitor> {
    let monitors = display.monitors();
    let monitor_list: Vec<gtk4::gdk::Monitor> = (0..monitors.n_items())
        .filter_map(|i| monitors.item(i))
        .filter_map(|m| m.downcast::<gtk4::gdk::Monitor>().ok())
        .collect();
    if let Some(primary_cfg) = cfg.primary_output() {
        if let Some(matched) = monitor_list.iter().find(|monitor| {
            let geo = monitor.geometry();
            primary_cfg.matches_monitor_identity(
                monitor.connector().as_deref(),
                monitor.model().as_deref(),
                monitor.description().as_deref(),
                geo.x(),
                geo.y(),
            )
        }) {
            return Some(matched.clone());
        }
    }
    monitor_list.into_iter().next()
}

fn clear_dock_drop_placeholder(apps_box: &GtkBox, drop_placeholder: &Rc<RefCell<Option<GtkBox>>>) {
    if let Some(placeholder) = drop_placeholder.borrow_mut().take() {
        if placeholder.parent().as_ref() == Some(apps_box.as_ref()) {
            apps_box.remove(&placeholder);
        }
    }
}

fn update_dock_drop_placeholder(
    apps_box: &GtkBox,
    drop_placeholder: &Rc<RefCell<Option<GtkBox>>>,
    dock_config: &Rc<RefCell<DockConfig>>,
    local_x: Option<f64>,
) {
    let mut placeholder_guard = drop_placeholder.borrow_mut();
    let cfg = dock_config.borrow();
    let total_pinned = cfg.apps.len();
    let icon_size = cfg.icon_size as i32;
    let target_idx = local_x
        .map(|x| {
            compute_pinned_insert_index(
                apps_box,
                placeholder_guard
                    .as_ref()
                    .map(|w| w.upcast_ref::<gtk4::Widget>()),
                total_pinned,
                x,
            )
        })
        .unwrap_or(total_pinned);
    drop(cfg);

    if let Some(ref placeholder) = *placeholder_guard {
        insert_or_reorder_box_child(apps_box, placeholder.upcast_ref(), target_idx);
    } else {
        let placeholder = create_drop_placeholder(icon_size);
        insert_or_reorder_box_child(apps_box, placeholder.upcast_ref(), target_idx);
        *placeholder_guard = Some(placeholder);
    }
}

fn extract_dropped_desktop_apps(file_list: &gdk::FileList) -> Vec<DockAppConfig> {
    file_list
        .files()
        .into_iter()
        .filter_map(|file| file.path())
        .filter_map(|path| malus_de_common::apps::desktop_file_to_dock_app(&path))
        .collect()
}

fn drop_has_desktop_file(target: &gtk4::DropTarget) -> Option<bool> {
    let value = target.value()?;
    let file_list = value.get::<gdk::FileList>().ok()?;
    let has_desktop = file_list.files().into_iter().any(|f| {
        f.path()
            .and_then(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|s| s == "desktop")
            })
            .unwrap_or(false)
    });
    Some(has_desktop)
}

fn pin_desktop_apps_at(
    cfg: &mut DockConfig,
    apps: Vec<DockAppConfig>,
    mut insert_idx: usize,
) -> bool {
    let mut changed = false;
    for app in apps {
        if cfg.pin_app_at(app, Some(insert_idx)) {
            changed = true;
        }
        insert_idx = insert_idx.saturating_add(1).min(cfg.apps.len());
    }
    changed
}

fn is_trashable_path(path: &Path) -> bool {
    if path.as_os_str().is_empty() || path == Path::new("/") {
        return false;
    }
    if std::env::var_os("HOME").is_some_and(|home| path == Path::new(&home)) {
        return false;
    }
    let path_str = path.to_string_lossy();
    if path_str.contains("/.local/share/Trash") {
        return false;
    }
    path.symlink_metadata().is_ok()
}

fn extract_trashable_paths(file_list: &gdk::FileList) -> Vec<PathBuf> {
    file_list
        .files()
        .into_iter()
        .filter_map(|file| file.path())
        .filter(|path| is_trashable_path(path))
        .collect()
}

fn drop_has_trashable_file(target: &gtk4::DropTarget) -> Option<bool> {
    let value = target.value()?;
    let file_list = value.get::<gdk::FileList>().ok()?;
    Some(!extract_trashable_paths(&file_list).is_empty())
}

fn is_point_over_trash(dock_root: &GtkBox, trash_widget: &GtkBox, x: f64, y: f64) -> bool {
    let Some(bounds) = trash_widget.compute_bounds(dock_root) else {
        return false;
    };
    let bx = f64::from(bounds.x());
    let by = f64::from(bounds.y());
    let bw = f64::from(bounds.width());
    let bh = f64::from(bounds.height());
    x >= bx - 6.0 && x <= bx + bw + 6.0 && y >= by - 12.0 && y <= by + bh + 12.0
}

fn trash_dropped_paths(paths: &[PathBuf]) -> usize {
    let mut trashed = 0usize;
    for path in paths {
        let file = gio::File::for_path(path);
        match file.trash(gio::Cancellable::NONE) {
            Ok(()) => {
                trashed += 1;
            }
            Err(err) => {
                warn!("🗑️ Error al enviar {:?} a la papelera: {}", path, err);
            }
        }
    }
    trashed
}

#[allow(clippy::too_many_arguments)]
fn attach_dock_drop_target(
    window: &ApplicationWindow,
    dock_root: &GtkBox,
    dock_bar: &Overlay,
    apps_box: &GtkBox,
    trash_widget: &GtkBox,
    trash_img: &Image,
    dock_config: &Rc<RefCell<DockConfig>>,
    drop_placeholder: &Rc<RefCell<Option<GtkBox>>>,
    on_refresh: &Rc<dyn Fn()>,
) {
    let drop_target = gtk4::DropTarget::new(
        gdk::FileList::static_type(),
        gdk::DragAction::COPY | gdk::DragAction::MOVE,
    );
    drop_target.set_propagation_phase(gtk4::PropagationPhase::Capture);
    drop_target.set_preload(true);

    let update_hover = {
        let window = window.clone();
        let dock_root = dock_root.clone();
        let dock_bar = dock_bar.clone();
        let apps_box = apps_box.clone();
        let trash_widget = trash_widget.clone();
        let dock_config = dock_config.clone();
        let drop_placeholder = drop_placeholder.clone();
        move |target: &gtk4::DropTarget, x: f64, y: f64| -> gdk::DragAction {
            if is_point_over_trash(&dock_root, &trash_widget, x, y) {
                clear_dock_drop_placeholder(&apps_box, &drop_placeholder);
                queue_dock_resize(&window, &dock_root, &dock_bar);
                if drop_has_trashable_file(target) == Some(false) {
                    trash_widget.remove_css_class("dock-trash-drop-hover");
                    return gdk::DragAction::empty();
                }
                trash_widget.add_css_class("dock-trash-drop-hover");
                return gdk::DragAction::MOVE;
            }

            trash_widget.remove_css_class("dock-trash-drop-hover");
            if drop_has_desktop_file(target) == Some(false) {
                clear_dock_drop_placeholder(&apps_box, &drop_placeholder);
                queue_dock_resize(&window, &dock_root, &dock_bar);
                return gdk::DragAction::empty();
            }
            let local_x = dock_root
                .compute_point(&apps_box, &gtk4::graphene::Point::new(x as f32, y as f32))
                .map(|p| p.x() as f64);
            update_dock_drop_placeholder(&apps_box, &drop_placeholder, &dock_config, local_x);
            queue_dock_resize(&window, &dock_root, &dock_bar);
            gdk::DragAction::COPY
        }
    };

    let enter_hover = update_hover.clone();
    drop_target.connect_enter(move |target, x, y| {
        info!("📥 [Dock DnD] Enter sobre el Dock en ({:.1}, {:.1})", x, y);
        enter_hover(target, x, y)
    });

    let motion_hover = update_hover;
    drop_target.connect_motion(move |target, x, y| motion_hover(target, x, y));

    let window_leave = window.clone();
    let dock_root_leave = dock_root.clone();
    let dock_bar_leave = dock_bar.clone();
    let apps_box_leave = apps_box.clone();
    let trash_widget_leave = trash_widget.clone();
    let placeholder_leave = drop_placeholder.clone();
    drop_target.connect_leave(move |_| {
        trash_widget_leave.remove_css_class("dock-trash-drop-hover");
        clear_dock_drop_placeholder(&apps_box_leave, &placeholder_leave);
        queue_dock_resize(&window_leave, &dock_root_leave, &dock_bar_leave);
    });

    let dock_root_drop = dock_root.clone();
    let apps_box_drop = apps_box.clone();
    let trash_widget_drop = trash_widget.clone();
    let trash_img_drop = trash_img.clone();
    let cfg_drop = dock_config.clone();
    let placeholder_drop = drop_placeholder.clone();
    let on_refresh_drop = on_refresh.clone();
    drop_target.connect_drop(move |_, value, x, y| {
        trash_widget_drop.remove_css_class("dock-trash-drop-hover");
        clear_dock_drop_placeholder(&apps_box_drop, &placeholder_drop);
        let Ok(file_list) = value.get::<gdk::FileList>() else {
            return false;
        };

        if is_point_over_trash(&dock_root_drop, &trash_widget_drop, x, y) {
            let paths = extract_trashable_paths(&file_list);
            if paths.is_empty() {
                return false;
            }
            let trashed = trash_dropped_paths(&paths);
            if trashed > 0 {
                trash_img_drop.set_icon_name(Some(if check_trash_full() {
                    "user-trash-full"
                } else {
                    "user-trash"
                }));
                info!("🗑️ [Dock DnD] Enviados {} elementos a la papelera", trashed);
                return true;
            }
            return false;
        }

        let apps = extract_dropped_desktop_apps(&file_list);
        info!(
            "📥 [Dock DnD] Drop recibido en ({:.1}, {:.1}) con {} apps .desktop",
            x,
            y,
            apps.len()
        );
        if apps.is_empty() {
            return false;
        }

        let mut cfg = cfg_drop.borrow_mut();
        let target_idx = dock_root_drop
            .compute_point(
                &apps_box_drop,
                &gtk4::graphene::Point::new(x as f32, y as f32),
            )
            .map(|pt| {
                let coord = if apps_box_drop.orientation() == Orientation::Horizontal {
                    pt.x() as f64
                } else {
                    pt.y() as f64
                };
                compute_pinned_insert_index(&apps_box_drop, None, cfg.apps.len(), coord)
            })
            .unwrap_or(cfg.apps.len());

        if pin_desktop_apps_at(&mut cfg, apps, target_idx) {
            let _ = cfg.save();
            drop(cfg);
            on_refresh_drop();
        }
        true
    });

    dock_root.add_controller(drop_target);
}

struct DockItemDragState {
    widget: gtk4::Widget,
    app_name: String,
    icon_name: String,
    current_idx: usize,
    total_pinned: usize,
    start_x: f64,
    start_y: f64,
    active: bool,
    will_remove: bool,
    order_changed: bool,
}

fn create_drop_placeholder(icon_size: i32) -> GtkBox {
    let container = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["dock-item-box", "dock-drop-placeholder"])
        .valign(gtk4::Align::End)
        .build();
    container.set_overflow(gtk4::Overflow::Visible);

    let inner = GtkBox::builder()
        .css_classes(["dock-drop-placeholder-inner"])
        .width_request(icon_size + 8)
        .height_request(icon_size + 8)
        .valign(gtk4::Align::End)
        .build();
    inner.set_overflow(gtk4::Overflow::Visible);

    container.append(&inner);
    container
}

fn compute_pinned_insert_index(
    apps_box: &GtkBox,
    ignore_child: Option<&gtk4::Widget>,
    total_pinned: usize,
    local_coord: f64,
) -> usize {
    let is_horizontal = apps_box.orientation() == Orientation::Horizontal;
    let mut child = apps_box.first_child();
    let mut idx = 0usize;
    while let Some(c) = child {
        if let Some(ignored) = ignore_child {
            if c == *ignored {
                child = c.next_sibling();
                continue;
            }
        }
        if idx >= total_pinned {
            break;
        }
        if let Some(bounds) = c.compute_bounds(apps_box) {
            let center = if is_horizontal {
                bounds.x() as f64 + (bounds.width() as f64) * 0.5
            } else {
                bounds.y() as f64 + (bounds.height() as f64) * 0.5
            };
            if local_coord < center {
                return idx;
            }
        }
        idx += 1;
        child = c.next_sibling();
    }
    total_pinned
}

fn insert_or_reorder_box_child(apps_box: &GtkBox, widget: &gtk4::Widget, target_idx: usize) {
    if widget.parent().as_ref() == Some(apps_box.as_ref()) {
        reorder_box_child_to_index(apps_box, widget, target_idx);
    } else {
        let mut others = Vec::new();
        let mut child = apps_box.first_child();
        while let Some(c) = child {
            let next = c.next_sibling();
            others.push(c);
            child = next;
        }

        if target_idx == 0 {
            apps_box.insert_child_after(widget, None::<&gtk4::Widget>);
        } else if let Some(prev) = others.get(target_idx - 1) {
            apps_box.insert_child_after(widget, Some(prev));
        } else {
            apps_box.append(widget);
        }
    }
}

fn reorder_box_child_to_index(apps_box: &GtkBox, widget: &gtk4::Widget, target_idx: usize) {
    let mut others = Vec::new();
    let mut child = apps_box.first_child();
    while let Some(c) = child {
        let next = c.next_sibling();
        if c != *widget {
            others.push(c);
        }
        child = next;
    }

    if target_idx == 0 {
        apps_box.reorder_child_after(widget, None::<&gtk4::Widget>);
    } else if let Some(prev) = others.get(target_idx - 1) {
        apps_box.reorder_child_after(widget, Some(prev));
    } else if let Some(last) = others.last() {
        apps_box.reorder_child_after(widget, Some(last));
    }
}

fn setup_dock_apps_drag_controller(
    apps_box: &GtkBox,
    dock_config: &Rc<RefCell<DockConfig>>,
    on_refresh: &Rc<dyn Fn()>,
) {
    let drag = GestureDrag::new();
    drag.set_button(1);
    drag.set_propagation_phase(gtk4::PropagationPhase::Capture);

    let drag_state: Rc<RefCell<Option<DockItemDragState>>> = Rc::new(RefCell::new(None));

    let apps_box_begin = apps_box.clone();
    let cfg_begin = dock_config.clone();
    let state_begin = drag_state.clone();
    drag.connect_drag_begin(move |_, start_x, start_y| {
        let cfg = cfg_begin.borrow();
        let total_pinned = cfg.apps.len();
        let mut child = apps_box_begin.first_child();
        let mut idx = 0usize;
        while let Some(c) = child {
            if idx >= total_pinned {
                break;
            }
            if let Some(bounds) = c.compute_bounds(&apps_box_begin) {
                let bx = bounds.x() as f64;
                let bw = bounds.width() as f64;
                if start_x >= bx - 2.0 && start_x <= bx + bw + 2.0 {
                    if let Some(app_cfg) = cfg.apps.get(idx) {
                        let resolved_icon = match app_cfg.icon.as_str() {
                            "system-file-manager" => "org.gnome.Nautilus",
                            "utilities-terminal" => "org.gnome.Ptyxis",
                            "accessories-text-editor" => "org.gnome.TextEditor",
                            other => other,
                        };
                        *state_begin.borrow_mut() = Some(DockItemDragState {
                            widget: c,
                            app_name: app_cfg.name.clone(),
                            icon_name: resolved_icon.to_string(),
                            current_idx: idx,
                            total_pinned,
                            start_x,
                            start_y,
                            active: false,
                            will_remove: false,
                            order_changed: false,
                        });
                    }
                    return;
                }
            }
            idx += 1;
            child = c.next_sibling();
        }
        *state_begin.borrow_mut() = None;
    });

    let apps_box_update = apps_box.clone();
    let cfg_update = dock_config.clone();
    let state_update = drag_state.clone();
    drag.connect_drag_update(move |gesture, offset_x, offset_y| {
        let mut guard = state_update.borrow_mut();
        let Some(ref mut state) = *guard else {
            return;
        };

        if !state.active {
            if offset_x.hypot(offset_y) < 6.0 {
                return;
            }
            state.active = true;
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            state.widget.add_css_class("dock-item-dragging");
            state.widget.set_opacity(0.0);
            let _ = send_ipc_message(&IpcMessage::StartDockIconDrag {
                name: state.app_name.clone(),
                icon: state.icon_name.clone(),
            });
        }

        let cur_x = state.start_x + offset_x;
        let cur_y = state.start_y + offset_y;
        let outside_dock = cur_y < -52.0
            || cur_y > (apps_box_update.height() as f64 + 42.0)
            || cur_x < -72.0
            || cur_x > (apps_box_update.width() as f64 + 72.0);

        if outside_dock {
            state.will_remove = true;
        } else {
            state.will_remove = false;

            let mut target_idx = state.current_idx;
            let mut child = apps_box_update.first_child();
            let mut idx = 0usize;
            while let Some(c) = child {
                if idx >= state.total_pinned {
                    break;
                }
                if let Some(bounds) = c.compute_bounds(&apps_box_update) {
                    let center_x = bounds.x() as f64 + (bounds.width() as f64) * 0.5;
                    if idx < state.current_idx && cur_x < center_x {
                        target_idx = idx;
                        break;
                    } else if idx > state.current_idx && cur_x > center_x {
                        target_idx = idx;
                    }
                }
                idx += 1;
                child = c.next_sibling();
            }

            if target_idx != state.current_idx
                && cfg_update
                    .borrow_mut()
                    .move_app(state.current_idx, target_idx)
            {
                reorder_box_child_to_index(&apps_box_update, &state.widget, target_idx);
                state.current_idx = target_idx;
                state.order_changed = true;
            }
        }
    });

    let cfg_end = dock_config.clone();
    let on_rf_end = on_refresh.clone();
    let state_end = drag_state;
    drag.connect_drag_end(move |_, _, _| {
        let Some(state) = state_end.borrow_mut().take() else {
            return;
        };
        state.widget.remove_css_class("dock-item-dragging");

        if !state.active {
            state.widget.set_opacity(1.0);
            return;
        }

        let _ = send_ipc_message(&IpcMessage::EndDockIconDrag {
            poof: state.will_remove,
        });

        if state.will_remove {
            info!(
                "🗑️ Icono '{}' arrastrado fuera del Dock; eliminando con efecto Poof",
                state.app_name
            );
            let mut cfg = cfg_end.borrow_mut();
            if cfg.unpin_app(&state.app_name) {
                let _ = cfg.save();
                drop(cfg);
                on_rf_end();
            } else {
                state.widget.set_opacity(1.0);
            }
        } else {
            state.widget.set_opacity(1.0);
            if state.order_changed {
                info!(
                    "🔀 Orden de iconos del Dock actualizado tras arrastrar '{}'",
                    state.app_name
                );
                let _ = cfg_end.borrow().save();
                on_rf_end();
            }
        }
    });

    apps_box.add_controller(drag);
}

fn render_dock_apps(
    apps_box: &GtkBox,
    dock_config: &Rc<RefCell<DockConfig>>,
    running_cache: &Rc<RefCell<Vec<String>>>,
    active_app_cache: &Rc<RefCell<Vec<String>>>,
    minimized_cache: &Rc<RefCell<Vec<malus_de_common::ipc::MinimizedWindowInfo>>>,
    dock_indicators: &Rc<RefCell<Vec<(Vec<String>, Label)>>>,
    on_refresh: &Rc<dyn Fn()>,
    refresh_pinned: bool,
) {
    let config = dock_config.borrow().clone();
    let running = running_cache.borrow().clone();
    let icon_size = config.icon_size as i32;
    let total_pinned = config.apps.len();

    if refresh_pinned {
        while let Some(child) = apps_box.first_child() {
            apps_box.remove(&child);
        }
        dock_indicators.borrow_mut().clear();
    } else {
        let mut child = apps_box.first_child();
        let mut index = 0;
        while let Some(current) = child {
            child = current.next_sibling();
            if index >= total_pinned {
                apps_box.remove(&current);
            }
            index += 1;
        }
        dock_indicators.borrow_mut().truncate(total_pinned);
    }

    // 1. Aplicaciones fijadas (Pinned Apps)
    if refresh_pinned {
        for (idx, app) in config.apps.iter().enumerate() {
            let is_running = running.iter().any(|r| app.matches(r));
            let (widget, ind) = create_dock_item(
                app,
                true,
                idx,
                total_pinned,
                icon_size,
                is_running,
                running_cache.clone(),
                active_app_cache.clone(),
                minimized_cache.clone(),
                dock_config.clone(),
                on_refresh.clone(),
            );
            apps_box.append(&widget);
            let ind_matchers = if app.matchers.is_empty() {
                vec![app.name.clone()]
            } else {
                app.matchers.clone()
            };
            dock_indicators.borrow_mut().push((ind_matchers, ind));
        }
    }

    // 2. Aplicaciones en ejecución no fijadas (Unpinned Running Apps)
    if config.show_recent_apps {
        let mut unpinned_candidates: Vec<String> = Vec::new();
        for r in &running {
            if should_ignore_app(r) {
                continue;
            }
            if config.apps.iter().any(|a| a.matches(r)) {
                continue;
            }
            let r_lower = r.to_lowercase();
            if !unpinned_candidates
                .iter()
                .any(|c| c.to_lowercase() == r_lower)
            {
                unpinned_candidates.push(r.clone());
            }
        }

        for cand in unpinned_candidates {
            let (name, icon, command, matchers) = resolve_desktop_app_info(&cand);
            let app_cfg = DockAppConfig {
                name,
                icon,
                command,
                matchers,
                icon_appearance: DockIconAppearance::Auto,
            };
            let (widget, ind) = create_dock_item(
                &app_cfg,
                false,
                0,
                0,
                icon_size,
                true,
                running_cache.clone(),
                active_app_cache.clone(),
                minimized_cache.clone(),
                dock_config.clone(),
                on_refresh.clone(),
            );
            apps_box.append(&widget);
            let ind_matchers = if app_cfg.matchers.is_empty() {
                vec![app_cfg.name.clone()]
            } else {
                app_cfg.matchers.clone()
            };
            dock_indicators.borrow_mut().push((ind_matchers, ind));
        }
    }
}

fn update_icon_size_recursive(
    widget: &gtk4::Widget,
    size: i32,
    base_size: i32,
    position: DockPosition,
) -> bool {
    let mut changed = false;
    let clamped_size = size.clamp(1, 512);
    let clamped_base = base_size.clamp(1, 512);
    let is_horizontal = position == DockPosition::Bottom;
    let (slot_w, slot_h) = if is_horizontal {
        (clamped_size, clamped_base)
    } else {
        (clamped_base, clamped_size)
    };
    if (widget.has_css_class("dock-icon-placeholder")
        || widget.has_css_class("dock-min-thumb-slot"))
        && (widget.width_request() != slot_w || widget.height_request() != slot_h)
    {
        widget.set_size_request(slot_w, slot_h);
        changed = true;
    }
    if let Ok(overlay) = widget.clone().downcast::<Overlay>() {
        let (target_w, target_h) = if overlay.has_css_class("dock-window-card") {
            if is_horizontal {
                (clamped_size, -1)
            } else {
                (-1, clamped_size)
            }
        } else {
            (slot_w, slot_h)
        };
        if overlay.width_request() != target_w || overlay.height_request() != target_h {
            overlay.set_size_request(target_w, target_h);
            changed = true;
        }
    }
    if let Ok(picture) = widget.clone().downcast::<Picture>() {
        let current_width = picture.width_request().max(1);
        let current_height = picture.height_request().max(1);
        let (target_w, target_h) = if is_horizontal {
            let w = clamped_size;
            let h = ((w as f64 * current_height as f64) / current_width as f64)
                .round()
                .clamp(1.0, 512.0) as i32;
            (w, h)
        } else {
            let h = clamped_size;
            let w = ((h as f64 * current_width as f64) / current_height as f64)
                .round()
                .clamp(1.0, 512.0) as i32;
            (w, h)
        };
        if picture.width_request() != target_w || picture.height_request() != target_h {
            picture.set_size_request(target_w, target_h);
            changed = true;
        }
    }
    if widget.has_css_class("dock-icon-shadow")
        && (widget.width_request() != size || widget.height_request() != size)
    {
        widget.set_size_request(size, size);
        changed = true;
    }
    if let Ok(img) = widget.clone().downcast::<Image>() {
        // Solo ajustar iconos principales del dock, preservando insignias secundarias
        if img.has_css_class("dock-primary-icon") || img.pixel_size() >= (base_size - 10) {
            let target_size = (size
                + if img.has_css_class("dock-icon-inset") {
                    -10
                } else {
                    0
                })
            .max(1);
            if img.pixel_size() != target_size {
                img.set_pixel_size(target_size);
                changed = true;
            }
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        changed |= update_icon_size_recursive(&c, size, base_size, position);
        child = c.next_sibling();
    }
    changed
}

struct IconSizeAnimation {
    widget: gtk4::Widget,
    current_size: f64,
    target_size: f64,
    base_size: i32,
}

fn update_icon_size_animation_targets(
    animations: &Rc<RefCell<Vec<IconSizeAnimation>>>,
    targets: Vec<(gtk4::Widget, i32, i32)>,
) {
    let mut animations = animations.borrow_mut();
    animations.retain(|animation| animation.widget.parent().is_some());

    for (widget, target_size, base_size) in targets {
        if let Some(animation) = animations
            .iter_mut()
            .find(|animation| animation.widget == widget)
        {
            animation.target_size = target_size as f64;
            animation.base_size = base_size;
        } else {
            animations.push(IconSizeAnimation {
                widget,
                current_size: base_size as f64,
                target_size: target_size as f64,
                base_size,
            });
        }
    }
}

fn start_icon_size_animation(
    animations: &Rc<RefCell<Vec<IconSizeAnimation>>>,
    animation_running: &Rc<Cell<bool>>,
    window: &ApplicationWindow,
    dock_root: &GtkBox,
    dock_bar: &Overlay,
    apps_box: &GtkBox,
    apps_placeholder: &GtkBox,
) {
    if animation_running.replace(true) {
        return;
    }

    let animations = animations.clone();
    let animation_running = animation_running.clone();
    let window = window.clone();
    let dock_root = dock_root.clone();
    let dock_bar = dock_bar.clone();
    let apps_box = apps_box.clone();
    let apps_placeholder = apps_placeholder.clone();
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let mut layout_changed = false;
        let mut settled = true;
        let mut animations = animations.borrow_mut();
        animations.retain_mut(|animation| {
            if animation.widget.parent().is_none()
                || animation.widget.opacity() <= 0.01
                || animation
                    .widget
                    .clone()
                    .downcast::<gtk4::Revealer>()
                    .is_ok_and(|r| !r.reveals_child())
            {
                return false;
            }

            let difference = animation.target_size - animation.current_size;
            if difference.abs() < 0.5 {
                animation.current_size = animation.target_size;
            } else {
                animation.current_size += difference * 0.22;
                settled = false;
            }
            layout_changed |= update_icon_size_recursive(
                &animation.widget,
                animation.current_size.round() as i32,
                animation.base_size,
                DOCK_CURRENT_POSITION.get(),
            );
            true
        });
        drop(animations);

        if layout_changed {
            sync_dock_apps_placeholder(&apps_box, &apps_placeholder);
            queue_dock_resize(&window, &dock_root, &dock_bar);
        }
        if settled {
            animation_running.set(false);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

enum DockIconRenderMode {
    /// Icono con canal alfa / forma libre (ej. Antigravity, Brave, VS Code):
    /// se dibuja el contenedor squircle del Dock y el icono reducido (`icon_size - 10`) centrado dentro.
    ContainerBackdrop,
    /// Icono que ya posee un bloque con relleno sólido cuadrado/redondeado:
    /// se rellenan los márgenes/esquinas con el color del borde del propio icono y se recorta con la curvatura squircle del Dock.
    FilledSquircle(gtk4::cairo::ImageSurface),
    /// Modo nativo sin contenedor ni recorte (`icon_appearance = "native"`).
    RawNative,
}

fn resolve_dock_icon_render_mode(
    appearance: DockIconAppearance,
    icon_name: &str,
    icon: &Image,
    icon_size: i32,
) -> DockIconRenderMode {
    match appearance {
        DockIconAppearance::DockBackground => DockIconRenderMode::ContainerBackdrop,
        DockIconAppearance::Native => DockIconRenderMode::RawNative,
        DockIconAppearance::Auto => match inspect_filled_icon_surface(icon_name, icon, icon_size) {
            Some(filled_surface) => DockIconRenderMode::FilledSquircle(filled_surface),
            None => DockIconRenderMode::ContainerBackdrop,
        },
    }
}

fn resolve_icon_file_path(
    icon_name: &str,
    icon: &Image,
    icon_size: i32,
) -> Option<std::path::PathBuf> {
    if icon_name.starts_with('/') {
        let p = std::path::PathBuf::from(icon_name);
        if p.is_file() {
            return Some(p);
        }
    }

    let icon_theme = gtk4::IconTheme::for_display(&icon.display());
    if icon_theme.has_icon(icon_name) {
        if let Some(path) = icon_theme
            .lookup_icon(
                icon_name,
                &[],
                icon_size.max(64),
                1,
                icon.direction(),
                gtk4::IconLookupFlags::empty(),
            )
            .file()
            .and_then(|file| file.path())
            .filter(|p| p.is_file())
        {
            return Some(path);
        }
    }

    let mut bases = vec![
        std::path::PathBuf::from("/usr/share/icons/hicolor"),
        std::path::PathBuf::from("/usr/share/icons/Adwaita"),
        std::path::PathBuf::from("/var/lib/flatpak/exports/share/icons/hicolor"),
        std::path::PathBuf::from("/usr/local/share/icons/hicolor"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        bases.insert(0, home.join(".local/share/icons/hicolor"));
        bases.insert(
            1,
            home.join(".local/share/flatpak/exports/share/icons/hicolor"),
        );
    }

    let subdirs = [
        "scalable/apps",
        "256x256/apps",
        "128x128/apps",
        "64x64/apps",
        "48x48/apps",
    ];
    for base in &bases {
        for sub in &subdirs {
            for ext in ["svg", "png"] {
                let candidate = base.join(sub).join(format!("{icon_name}.{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    for ext in ["svg", "png"] {
        let pixmap = std::path::PathBuf::from(format!("/usr/share/pixmaps/{icon_name}.{ext}"));
        if pixmap.is_file() {
            return Some(pixmap);
        }
    }

    None
}

fn inspect_filled_icon_surface(
    icon_name: &str,
    icon: &Image,
    icon_size: i32,
) -> Option<gtk4::cairo::ImageSurface> {
    let icon_path = resolve_icon_file_path(icon_name, icon, icon_size)?;
    let pixbuf = gdk_pixbuf::Pixbuf::from_file_at_scale(&icon_path, 128, 128, true).ok()?;
    let bytes = pixbuf.read_pixel_bytes();
    let pixels = bytes.as_ref();
    let channels = pixbuf.n_channels() as usize;
    let rowstride = pixbuf.rowstride() as usize;
    let width = pixbuf.width() as usize;
    let height = pixbuf.height() as usize;

    analyze_dock_icon_pixels(
        pixels,
        width,
        height,
        channels,
        rowstride,
        pixbuf.has_alpha(),
    )
}

/// Analiza los píxeles de un icono para determinar si ya es una baldosa cuadrada/squircle con relleno sólido.
/// - Si es un símbolo con canal alfa, círculo o logo con forma libre (como Antigravity), devuelve `None`
///   para que el Dock le cree el contenedor squircle estándar y reduzca su tamaño.
/// - Si ya es un icono con relleno sólido, recorta el margen exterior transparente, rellena el resto de las
///   esquinas extrapolando los colores del borde de cada fila y devuelve la superficie lista para recortar con `squircle_path`.
fn analyze_dock_icon_pixels(
    pixels: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    rowstride: usize,
    has_alpha: bool,
) -> Option<gtk4::cairo::ImageSurface> {
    if channels < 3 || width < 16 || height < 16 {
        return None;
    }

    let sample_rgba = |x: usize, y: usize| -> (u8, u8, u8, u8) {
        let idx = y * rowstride + x * channels;
        let r = pixels.get(idx).copied().unwrap_or(0);
        let g = pixels.get(idx + 1).copied().unwrap_or(0);
        let b = pixels.get(idx + 2).copied().unwrap_or(0);
        let a = if has_alpha && channels >= 4 {
            pixels.get(idx + 3).copied().unwrap_or(0)
        } else {
            255
        };
        (r, g, b, a)
    };

    // 1. Hallar el bounding box de los píxeles opacos (alpha >= 225)
    let mut min_x = width;
    let mut max_x = 0usize;
    let mut min_y = height;
    let mut max_y = 0usize;
    let mut opaque_count = 0usize;

    for y in 0..height {
        for x in 0..width {
            let (_, _, _, a) = sample_rgba(x, y);
            if a >= 225 {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                opaque_count += 1;
            }
        }
    }

    if min_x >= max_x || min_y >= max_y || opaque_count == 0 {
        return None;
    }

    let bw = max_x - min_x + 1;
    let bh = max_y - min_y + 1;

    // La baldosa rellena debe tener proporción prácticamente cuadrada (1:1) y cubrir una fracción significativa del lienzo
    let aspect = bw as f64 / bh as f64;
    if bw < width * 55 / 100 || bh < height * 55 / 100 || !(0.85..=1.18).contains(&aspect) {
        return None;
    }

    // 2. Relación de relleno dentro de su propio bounding box:
    // Un círculo inscrito tiene pi/4 ≈ 0.785 (78.5%) y un logo en forma de 'A' o triángulo tiene < 75%.
    // Un bloque cuadrado o squircle relleno supera siempre el 86% de su bounding box.
    let mut bbox_opaque = 0usize;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            if sample_rgba(x, y).3 >= 225 {
                bbox_opaque += 1;
            }
        }
    }
    let bbox_fill_ratio = bbox_opaque as f64 / (bw * bh) as f64;
    if bbox_fill_ratio < 0.86 {
        return None;
    }

    // 3. El núcleo central (20%..80%) debe ser completamente sólido sin huecos internos transparentes (>= 98.5%)
    let core_x0 = min_x + bw * 20 / 100;
    let core_x1 = max_x.saturating_sub(bw * 20 / 100).max(core_x0);
    let core_y0 = min_y + bh * 20 / 100;
    let core_y1 = max_y.saturating_sub(bh * 20 / 100).max(core_y0);
    let mut core_total = 0usize;
    let mut core_opaque = 0usize;
    for y in core_y0..=core_y1 {
        for x in core_x0..=core_x1 {
            core_total += 1;
            if sample_rgba(x, y).3 >= 240 {
                core_opaque += 1;
            }
        }
    }
    if core_total == 0 || (core_opaque as f64 / core_total as f64) < 0.985 {
        return None;
    }

    // 4. Los 4 «hombros» diagonales del squircle (u, v en {0.13, 0.87}) deben ser opacos:
    // En un círculo o logo libre, (0.13, 0.13) está a distancia sqrt(0.37^2 + 0.37^2) = 0.523 > 0.50 del centro (transparente),
    // mientras que en un icono cuadrado o squircle está dentro del cuerpo sólido.
    for (u, v) in [(0.13, 0.13), (0.87, 0.13), (0.13, 0.87), (0.87, 0.87)] {
        let sx = min_x + ((bw - 1) as f64 * u).round() as usize;
        let sy = min_y + ((bh - 1) as f64 * v).round() as usize;
        if sample_rgba(sx, sy).3 < 225 {
            return None;
        }
    }

    // 5. Las 4 bandas rectas cardinales (arriba, abajo, izquierda, derecha, a 6% del borde entre 28%..72%) deben ser opacas
    let edge_inset_x = (bw * 6 / 100).max(1);
    let edge_inset_y = (bh * 6 / 100).max(1);
    let span_x0 = min_x + bw * 28 / 100;
    let span_x1 = min_x + bw * 72 / 100;
    let span_y0 = min_y + bh * 28 / 100;
    let span_y1 = min_y + bh * 72 / 100;

    let mut cardinal_total = 0usize;
    let mut cardinal_opaque = 0usize;
    for x in span_x0..=span_x1 {
        cardinal_total += 2;
        if sample_rgba(x, min_y + edge_inset_y).3 >= 225 {
            cardinal_opaque += 1;
        }
        if sample_rgba(x, max_y - edge_inset_y).3 >= 225 {
            cardinal_opaque += 1;
        }
    }
    for y in span_y0..=span_y1 {
        cardinal_total += 2;
        if sample_rgba(min_x + edge_inset_x, y).3 >= 225 {
            cardinal_opaque += 1;
        }
        if sample_rgba(max_x - edge_inset_x, y).3 >= 225 {
            cardinal_opaque += 1;
        }
    }
    if cardinal_total == 0 || (cardinal_opaque as f64 / cardinal_total as f64) < 0.95 {
        return None;
    }

    // 6. Construir una superficie Cairo (bw × bh) rellenando cualquier esquina transparente
    // con el color opaco del borde de cada fila para que al aplicar `squircle_path` quede 100% relleno.
    let mut surface =
        gtk4::cairo::ImageSurface::create(gtk4::cairo::Format::ARgb32, bw as i32, bh as i32)
            .ok()?;
    let dst_stride = surface.stride() as usize;
    {
        let mut dst = surface.data().ok()?;
        let mut fallback_left = (48u8, 48u8, 52u8);
        let mut fallback_right = (48u8, 48u8, 52u8);

        // Pre-muestrear la primera fila con píxeles opacos para inicializar el color de respaldo
        for sy in min_y..=max_y {
            let mut first = None;
            let mut last = None;
            for sx in min_x..=max_x {
                let (r, g, b, a) = sample_rgba(sx, sy);
                if a >= 240 {
                    if first.is_none() {
                        first = Some((r, g, b));
                    }
                    last = Some((r, g, b));
                }
            }
            if let (Some(l), Some(r)) = (first, last) {
                fallback_left = l;
                fallback_right = r;
                break;
            }
        }

        for ry in 0..bh {
            let sy = min_y + ry;
            let mut row_left_rgb = None;
            let mut row_right_rgb = None;
            for rx in 0..bw {
                let (r, g, b, a) = sample_rgba(min_x + rx, sy);
                if a >= 240 {
                    if row_left_rgb.is_none() {
                        row_left_rgb = Some((r, g, b));
                    }
                    row_right_rgb = Some((r, g, b));
                }
            }
            let left_fill = row_left_rgb.unwrap_or(fallback_left);
            let right_fill = row_right_rgb.unwrap_or(fallback_right);
            fallback_left = left_fill;
            fallback_right = right_fill;

            let dst_row = &mut dst[ry * dst_stride..];
            for rx in 0..bw {
                let (sr, sg, sb, sa) = sample_rgba(min_x + rx, sy);
                let bg = if rx < bw / 2 { left_fill } else { right_fill };
                let alpha_f = u32::from(sa);
                let inv_a = 255 - alpha_f;
                let out_r = ((u32::from(sr) * alpha_f + u32::from(bg.0) * inv_a + 127) / 255) as u8;
                let out_g = ((u32::from(sg) * alpha_f + u32::from(bg.1) * inv_a + 127) / 255) as u8;
                let out_b = ((u32::from(sb) * alpha_f + u32::from(bg.2) * inv_a + 127) / 255) as u8;

                let di = rx * 4;
                dst_row[di] = out_b;
                dst_row[di + 1] = out_g;
                dst_row[di + 2] = out_r;
                dst_row[di + 3] = 255;
            }
        }
    }
    surface.flush();
    Some(surface)
}

fn create_magnifiable_icon(
    icon: &Image,
    icon_size: i32,
    render_mode: DockIconRenderMode,
) -> Overlay {
    let placeholder = GtkBox::new(Orientation::Horizontal, 0);
    placeholder.add_css_class("dock-icon-placeholder");
    placeholder.set_size_request(icon_size, icon_size);

    let overlay = Overlay::new();
    overlay.set_child(Some(&placeholder));
    match render_mode {
        DockIconRenderMode::ContainerBackdrop => {
            let shadow = DrawingArea::new();
            shadow.add_css_class("dock-icon-shadow");
            shadow.set_size_request(icon_size, icon_size);
            shadow.set_halign(gtk4::Align::Center);
            shadow.set_valign(gtk4::Align::End);
            shadow.set_can_target(false);
            shadow.set_overflow(gtk4::Overflow::Visible);
            shadow.set_draw_func(|_, context, width, height| {
                draw_squircle(context, width as f64, height as f64);
            });
            overlay.add_overlay(&shadow);
            overlay.set_measure_overlay(&shadow, false);

            overlay.add_overlay(icon);
            overlay.set_measure_overlay(icon, false);
        }
        DockIconRenderMode::FilledSquircle(filled_surface) => {
            let filled_area = DrawingArea::new();
            filled_area.add_css_class("dock-icon-shadow");
            filled_area.set_size_request(icon_size, icon_size);
            filled_area.set_halign(gtk4::Align::Center);
            filled_area.set_valign(gtk4::Align::End);
            filled_area.set_can_target(false);
            filled_area.set_overflow(gtk4::Overflow::Visible);
            filled_area.set_draw_func(move |_, context, width, height| {
                draw_filled_squircle_icon(context, &filled_surface, width as f64, height as f64);
            });
            overlay.add_overlay(&filled_area);
            overlay.set_measure_overlay(&filled_area, false);
        }
        DockIconRenderMode::RawNative => {
            overlay.add_overlay(icon);
            overlay.set_measure_overlay(icon, false);
        }
    }
    overlay.set_overflow(gtk4::Overflow::Visible);

    icon.set_halign(gtk4::Align::Center);
    icon.set_valign(gtk4::Align::End);
    icon.set_overflow(gtk4::Overflow::Visible);

    overlay
}

fn queue_dock_icon_backgrounds(widget: &gtk4::Widget) {
    if let Ok(area) = widget.clone().downcast::<DrawingArea>() {
        if area.has_css_class("dock-icon-shadow") {
            area.queue_draw();
        }
    }

    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        queue_dock_icon_backgrounds(&current);
    }
}

fn dock_squircle_radius(width: f64, height: f64) -> f64 {
    let side = width.min(height).max(1.0);
    (side * (22.0 / 48.0)).min(side * 0.5)
}

fn draw_squircle(context: &gtk4::cairo::Context, width: f64, height: f64) {
    let style_idx = malus_de_common::config::load_icon_widget_style_index();
    let is_dark = adw::StyleManager::default().is_dark();
    let radius = dock_squircle_radius(width, height);

    match style_idx {
        // 1 = Obscuro: baldosa oscura estilo macOS incluso en modo claro
        1 => {
            squircle_path(context, width, height, radius);
            let grad = gtk4::cairo::LinearGradient::new(0.0, 0.0, 0.0, height);
            grad.add_color_stop_rgb(0.0, 0.15, 0.16, 0.19);
            grad.add_color_stop_rgb(1.0, 0.07, 0.08, 0.10);
            let _ = context.set_source(&grad);
            let _ = context.fill_preserve();
            context.set_source_rgba(1.0, 1.0, 1.0, 0.12);
            context.set_line_width(1.0);
            let _ = context.stroke();
        }
        // 2 = Transparente: cristal translúcido Liquid Glass (o gris sólido si Reducir transparencia está activo)
        2 => {
            squircle_path(context, width, height, radius);
            if malus_de_common::config::load_reduce_transparency() {
                if is_dark {
                    context.set_source_rgb(0.24, 0.25, 0.28);
                } else {
                    context.set_source_rgb(0.56, 0.58, 0.62);
                }
                let _ = context.fill_preserve();
                context.set_source_rgba(1.0, 1.0, 1.0, 0.22);
                context.set_line_width(1.0);
                let _ = context.stroke();
            } else {
                let glass_scale = malus_de_common::config::current_liquid_glass_scale();
                let base_alpha = malus_de_common::config::scaled_liquid_glass_alpha(
                    if is_dark { 0.44 } else { 0.36 },
                    glass_scale,
                    0.16,
                    0.88,
                );
                if is_dark {
                    context.set_source_rgba(0.14, 0.15, 0.19, base_alpha);
                } else {
                    context.set_source_rgba(0.95, 0.96, 0.99, base_alpha);
                }
                let _ = context.fill_preserve();

                let sheen = gtk4::cairo::LinearGradient::new(0.0, 0.0, 0.0, height);
                sheen.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, if is_dark { 0.20 } else { 0.34 });
                sheen.add_color_stop_rgba(0.5, 1.0, 1.0, 1.0, 0.04);
                sheen.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
                let _ = context.set_source(&sheen);
                let _ = context.fill_preserve();

                context.set_source_rgba(1.0, 1.0, 1.0, if is_dark { 0.26 } else { 0.52 });
                context.set_line_width(1.0);
                let _ = context.stroke();
            }
        }
        // 3 = Teñido: fondo teñido con el color de énfasis activo
        3 => {
            let (r, g, b) = malus_de_common::config::load_accent_color();
            squircle_path(context, width, height, radius);
            let grad = gtk4::cairo::LinearGradient::new(0.0, 0.0, 0.0, height);
            grad.add_color_stop_rgb(
                0.0,
                (r * 1.12).min(1.0),
                (g * 1.12).min(1.0),
                (b * 1.12).min(1.0),
            );
            grad.add_color_stop_rgb(1.0, r * 0.80, g * 0.80, b * 0.80);
            let _ = context.set_source(&grad);
            let _ = context.fill_preserve();

            context.set_source_rgba(1.0, 1.0, 1.0, 0.26);
            context.set_line_width(1.0);
            let _ = context.stroke();
        }
        // 0 = Predeterminado: blanco en modo claro, oscuro en modo oscuro
        _ => {
            squircle_path(context, width, height, radius);
            if is_dark {
                context.set_source_rgb(0.06, 0.06, 0.07);
            } else {
                context.set_source_rgb(1.0, 1.0, 1.0);
            }
            let _ = context.fill();
        }
    }
}

fn draw_filled_squircle_icon(
    context: &gtk4::cairo::Context,
    surface: &gtk4::cairo::ImageSurface,
    width: f64,
    height: f64,
) {
    let sw = surface.width() as f64;
    let sh = surface.height() as f64;
    if sw <= 0.0 || sh <= 0.0 || width <= 0.0 || height <= 0.0 {
        return;
    }

    let _ = context.save();
    squircle_path(context, width, height, dock_squircle_radius(width, height));
    context.clip();

    context.scale(width / sw, height / sh);
    let _ = context.set_source_surface(surface, 0.0, 0.0);
    let _ = context.paint();
    let _ = context.restore();
}

fn draw_switcher_squircle(
    area: &DrawingArea,
    context: &gtk4::cairo::Context,
    width: f64,
    height: f64,
    radius: f64,
    selected: bool,
    container: bool,
) {
    let background_color = theme_color(
        area,
        if container {
            "window_bg_color"
        } else if selected {
            "accent_bg_color"
        } else {
            "window_fg_color"
        },
    );
    let foreground = theme_color(area, "window_fg_color");
    let accent_color = theme_color(area, "accent_bg_color");

    squircle_path(context, width, height, radius);
    let fill_color = if container {
        background_color
    } else if selected {
        accent_color
    } else {
        foreground
    };
    let glass_scale = malus_de_common::config::current_liquid_glass_scale();
    let container_alpha =
        malus_de_common::config::scaled_liquid_glass_alpha(0.42, glass_scale, 0.10, 0.92) as f32;
    let fill_alpha = if container {
        container_alpha
    } else if selected {
        0.22
    } else {
        0.055
    };
    context.set_source_rgba(
        fill_color.red() as f64,
        fill_color.green() as f64,
        fill_color.blue() as f64,
        (fill_color.alpha() * fill_alpha) as f64,
    );
    let _ = context.fill_preserve();

    let border_color = if selected { accent_color } else { foreground };
    let border_alpha = if container {
        0.18
    } else if selected {
        0.95
    } else {
        0.14
    };
    context.set_source_rgba(
        border_color.red() as f64,
        border_color.green() as f64,
        border_color.blue() as f64,
        (border_color.alpha() * border_alpha) as f64,
    );
    context.set_line_width(if selected { 1.5 } else { 1.0 });
    let _ = context.stroke();
}

#[allow(deprecated)]
fn theme_color(area: &DrawingArea, name: &str) -> gtk4::gdk::RGBA {
    area.style_context()
        .lookup_color(name)
        .unwrap_or_else(|| area.color())
}

fn squircle_path(context: &gtk4::cairo::Context, width: f64, height: f64, radius: f64) {
    let radius = radius.min(width / 2.0).min(height / 2.0);
    let power = 2.0 / SQUIRCLE_EXPONENT;
    let quarter_turn = std::f64::consts::FRAC_PI_2;

    context.move_to(radius, 0.0);
    context.line_to(width - radius, 0.0);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            width - radius + radius * angle.sin().powf(power),
            radius - radius * angle.cos().powf(power),
        );
    }
    context.line_to(width, height - radius);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            width - radius + radius * angle.cos().powf(power),
            height - radius + radius * angle.sin().powf(power),
        );
    }
    context.line_to(radius, height);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            radius - radius * angle.sin().powf(power),
            height - radius + radius * angle.cos().powf(power),
        );
    }
    context.line_to(0.0, radius);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            radius - radius * angle.cos().powf(power),
            radius - radius * angle.sin().powf(power),
        );
    }

    context.close_path();
}

fn make_dock_menu_button(label: &str, danger: bool) -> Button {
    let classes: &[&str] = if danger {
        &[
            "popover-menu-item",
            "popover-menu-item-danger",
            "malus-squircle-menu-item",
        ]
    } else {
        &["popover-menu-item", "malus-squircle-menu-item"]
    };
    let btn = Button::builder()
        .label(label)
        .has_frame(false)
        .css_classes(classes)
        .build();
    if let Some(inner) = btn.child() {
        if let Some(lbl) = inner.downcast_ref::<Label>() {
            lbl.set_xalign(0.0);
        }
        btn.set_child(gtk4::Widget::NONE);
        inner.set_margin_top(4);
        inner.set_margin_bottom(4);
        inner.set_margin_start(8);
        inner.set_margin_end(8);

        let bg = DrawingArea::new();
        bg.set_can_target(false);
        bg.set_hexpand(true);
        bg.set_vexpand(false);

        let btn_weak = btn.downgrade();
        bg.set_draw_func(move |_, cr, w, h| {
            let width = w as f64;
            let height = h as f64;
            if width <= 2.0 || height <= 2.0 {
                return;
            }
            let Some(b) = btn_weak.upgrade() else {
                return;
            };
            let flags = b.state_flags();
            if flags.contains(gtk4::StateFlags::INSENSITIVE) {
                return;
            }
            let hovered = flags.contains(gtk4::StateFlags::PRELIGHT);
            let active = flags.contains(gtk4::StateFlags::ACTIVE);
            if !hovered && !active {
                return;
            }
            let _ = cr.save();
            squircle_path(cr, width, height, 14.0);
            let (red, green, blue) = malus_de_common::config::load_accent_color();
            if active {
                cr.set_source_rgba(red * 0.78, green * 0.78, blue * 0.78, 0.86);
            } else {
                cr.set_source_rgba(red, green, blue, 0.74);
            }
            let _ = cr.fill();
            let _ = cr.restore();
        });

        let bg_state = bg.clone();
        btn.connect_state_flags_changed(move |_, _| {
            bg_state.queue_draw();
        });

        let overlay = Overlay::builder()
            .hexpand(true)
            .vexpand(false)
            .valign(gtk4::Align::Center)
            .build();
        overlay.set_child(Some(&bg));
        overlay.add_overlay(&inner);
        overlay.set_measure_overlay(&inner, true);
        btn.set_vexpand(false);
        btn.set_valign(gtk4::Align::Start);
        btn.set_child(Some(&overlay));
    }
    btn
}

fn create_minimized_window_item(info: &MinimizedWindowInfo, icon_size: i32) -> (gtk4::Widget, u64) {
    let window_id = info.window_id;

    // Mini ventana: captura real cuando está disponible, con la tarjeta simulada como respaldo.
    let mini_win = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["dock-mini-window"])
        .valign(gtk4::Align::Center)
        .build();

    let has_thumbnail = info.thumbnail_path.as_deref().is_some_and(|path| {
        match gdk_pixbuf::Pixbuf::from_file_at_scale(path, icon_size.max(1), icon_size.max(1), true)
        {
            Ok(pixbuf) => {
                let texture = gtk4::gdk::Texture::for_pixbuf(&pixbuf);
                let preview_height = ((icon_size as f64 * pixbuf.height() as f64)
                    / pixbuf.width().max(1) as f64)
                    .round()
                    .max(1.0) as i32;
                let image = Picture::for_paintable(&texture);
                image.set_content_fit(gtk4::ContentFit::Contain);
                image.set_can_shrink(true);
                image.set_size_request(icon_size, preview_height);
                image.set_hexpand(true);
                image.add_css_class("dock-window-thumbnail");
                mini_win.add_css_class("has-thumbnail");
                mini_win.append(&image);
                true
            }
            Err(error) => {
                warn!("No se pudo cargar la miniatura de '{}': {error}", path);
                false
            }
        }
    });

    if !has_thumbnail {
        mini_win.set_size_request(icon_size.max(1), -1);

        // Barra superior simulada, visible solo mientras no haya una captura real.
        let titlebar = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .css_classes(["dock-mini-window-titlebar"])
            .spacing(2)
            .build();

        let dot_red = Label::builder()
            .label("●")
            .css_classes(["dock-mini-dot", "dot-red"])
            .build();
        let dot_yellow = Label::builder()
            .label("●")
            .css_classes(["dock-mini-dot", "dot-yellow"])
            .build();
        let dot_green = Label::builder()
            .label("●")
            .css_classes(["dock-mini-dot", "dot-green"])
            .build();

        titlebar.append(&dot_red);
        titlebar.append(&dot_yellow);
        titlebar.append(&dot_green);
        mini_win.append(&titlebar);

        let body = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .css_classes(["dock-mini-window-body"])
            .valign(gtk4::Align::Center)
            .build();

        let line1 = GtkBox::builder()
            .css_classes(["dock-mini-content-line"])
            .build();
        let line2 = GtkBox::builder()
            .css_classes(["dock-mini-content-line-short"])
            .build();
        body.append(&line1);
        body.append(&line2);
        mini_win.append(&body);
    }

    // 3. Insignia circular de la app en la esquina inferior derecha (macOS style)
    let icon_name = info
        .icon_name
        .as_deref()
        .unwrap_or("application-x-executable");
    let badge_img = Image::builder().icon_name(icon_name).pixel_size(18).build();

    let badge_box = GtkBox::builder()
        .css_classes(["dock-mini-badge"])
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::End)
        .build();
    badge_box.append(&badge_img);

    // Contenedor Overlay para superponer la insignia sobre la mini ventana
    let overlay = Overlay::builder()
        .child(&mini_win)
        .css_classes(["dock-window-card"])
        .tooltip_text(&info.title)
        .focusable(false)
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::End)
        .build();
    overlay.set_overflow(gtk4::Overflow::Visible);
    if has_thumbnail {
        overlay.add_css_class("dock-window-card-thumbnail");
    }
    overlay.add_overlay(&badge_box);

    // Banda vertical de la altura fija de los iconos + banda inferior de indicadores.
    // `slot_spacer` fija la altura en `icon_size` (nunca empuja el alto del Dock al magnificar),
    // mientras `overlay` crece libremente hacia arriba como overlay no medido igual que los iconos.
    let item_container = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["dock-item-box"])
        .spacing(0)
        .valign(gtk4::Align::End)
        .build();
    item_container.set_overflow(gtk4::Overflow::Visible);

    let slot_spacer = GtkBox::new(Orientation::Horizontal, 0);
    slot_spacer.add_css_class("dock-min-thumb-slot");
    slot_spacer.set_size_request(icon_size.max(1), icon_size.max(1));

    let icon_band = Overlay::new();
    icon_band.set_overflow(gtk4::Overflow::Visible);
    icon_band.set_valign(gtk4::Align::End);
    icon_band.set_size_request(icon_size.max(1), icon_size.max(1));
    icon_band.set_child(Some(&slot_spacer));
    icon_band.add_overlay(&overlay);
    icon_band.set_measure_overlay(&overlay, false);
    icon_band.set_clip_overlay(&overlay, false);

    let spacer = Label::builder()
        .label("●")
        .css_classes(["dock-indicator"])
        .opacity(0.0)
        .valign(gtk4::Align::End)
        .build();

    item_container.append(&icon_band);
    item_container.append(&spacer);
    apply_dock_item_geometry(&item_container, DOCK_CURRENT_POSITION.get());

    // Clic para restaurar la ventana vía animación inversa desde el Dock
    let gesture = GestureClick::new();
    let id = window_id;
    let item_for_click = item_container.clone();
    gesture.connect_pressed(move |_, _, _, _| {
        info!(
            "✨ Clic en miniatura de Dock: restaurando ventana id={}",
            id
        );
        // Ocultar la tarjeta en el instante del clic para que no quede encimada ni se aplaste
        // mientras el compositor expande la captura real desde este slot.
        if let Some(revealer) = item_for_click.parent() {
            revealer.set_opacity(0.0);
            revealer.set_sensitive(false);
        } else {
            item_for_click.set_opacity(0.0);
            item_for_click.set_sensitive(false);
        }
        if let Err(err) = send_ipc_message(&IpcMessage::AnimateRestore { window_id: id }) {
            warn!(
                "No se pudo enviar AnimateRestore (id={}) por IPC: {}",
                id, err
            );
        }
    });
    overlay.add_controller(gesture);

    (item_container.upcast::<gtk4::Widget>(), window_id)
}

fn find_dock_widget_with_class(widget: &gtk4::Widget, class_name: &str) -> Option<gtk4::Widget> {
    if widget.has_css_class(class_name) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(found) = find_dock_widget_with_class(&c, class_name) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

fn create_dock_item_revealer(child: &gtk4::Widget) -> gtk4::Revealer {
    let revealer = gtk4::Revealer::new();
    revealer.set_transition_type(gtk4::RevealerTransitionType::SlideLeft);
    revealer.set_transition_duration(180);
    revealer.set_child(Some(child));
    revealer
}

fn register_minimized_dock_slot(
    card: &gtk4::Widget,
    dock_window: &ApplicationWindow,
    window_id: u64,
) {
    let target = find_dock_widget_with_class(card, "dock-window-card")
        .or_else(|| find_dock_widget_with_class(card, "dock-bounce-target"))
        .unwrap_or_else(|| card.clone());
    if !target.is_mapped() || !target.is_visible() || target.width() <= 1 || target.height() <= 1
    {
        return;
    }
    if let Some(point) = target.compute_point(dock_window, &gtk4::graphene::Point::zero()) {
        let x = point.x().round() as i32;
        let y = point.y().round() as i32;
        let width = target.width().max(1) as u32;
        let height = target.height().max(1) as u32;

        info!(
            "📍 Registrando slot de miniatura en Dock: id={}, pos=({}, {}, {}, {})",
            window_id, x, y, width, height
        );
        if let Err(err) = send_ipc_message(&IpcMessage::RegisterDockSlot {
            window_id,
            x,
            y,
            width,
            height,
        }) {
            warn!(
                "No se pudo enviar RegisterDockSlot (id={}) por IPC: {}",
                window_id, err
            );
        }
    }
}

fn create_trash_item(icon_size: i32) -> (GtkBox, Image) {
    let item_container = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["dock-item-box"])
        .spacing(0)
        .valign(gtk4::Align::End)
        .build();
    item_container.set_overflow(gtk4::Overflow::Visible);

    let btn = Button::builder()
        .css_classes(["dock-item-btn", "dock-trash-btn"])
        .tooltip_text("Papelera")
        .focusable(false)
        .valign(gtk4::Align::End)
        .build();

    let trash_icon_name = if check_trash_full() {
        "user-trash-full"
    } else {
        "user-trash"
    };

    let trash_img = Image::builder()
        .icon_name(trash_icon_name)
        .pixel_size(icon_size)
        .css_classes(["dock-trash-icon", "dock-primary-icon"])
        .valign(gtk4::Align::End)
        .build();
    let trash_overlay =
        create_magnifiable_icon(&trash_img, icon_size, DockIconRenderMode::RawNative);
    btn.set_child(Some(&trash_overlay));

    btn.connect_clicked(|_| {
        spawn_dock_app("Branches", "malus-de-branches", &["trash:///"]);
    });

    // Menú Popover de Clic Derecho para la Papelera
    let popover = Popover::builder()
        .css_classes(["malus-popover-menu"])
        .has_arrow(false)
        .position(gtk4::PositionType::Top)
        .build();
    popover.set_offset(0, POPOVER_SHADOW_GUTTER);

    let menu_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .build();

    let header = Label::builder()
        .label("Papelera")
        .css_classes(["popover-menu-header"])
        .halign(gtk4::Align::Start)
        .build();
    menu_box.append(&header);

    let sep = Separator::new(Orientation::Horizontal);
    sep.add_css_class("popover-separator");
    menu_box.append(&sep);

    let btn_open = make_dock_menu_button("Abrir en Branches", false);
    let p_open = popover.clone();
    btn_open.connect_clicked(move |_| {
        p_open.popdown();
        spawn_dock_app("Branches", "malus-de-branches", &["trash:///"]);
    });

    let btn_empty = make_dock_menu_button("Vaciar Papelera", false);
    let img_clone = trash_img.clone();
    let popover_clone = popover.clone();
    btn_empty.connect_clicked(move |_| {
        info!("🗑️ Vaciando papelera...");
        let _ = Command::new("gio").args(["trash", "--empty"]).status();
        img_clone.set_icon_name(Some("user-trash"));
        popover_clone.popdown();
    });

    menu_box.append(&btn_open);
    menu_box.append(&btn_empty);
    let popover_surface = create_popover_surface(&menu_box);
    popover.set_child(Some(&popover_surface));
    popover.set_parent(&btn);
    let p_unroot = popover.clone();
    btn.connect_unrealize(move |_| {
        p_unroot.unparent();
    });

    let right_click = GestureClick::new();
    right_click.set_button(3);
    let p = popover.clone();
    right_click.connect_pressed(move |_, _, _, _| {
        apply_popover_dock_placement(&p, DOCK_CURRENT_POSITION.get());
        p.popup();
    });
    btn.add_controller(right_click);

    item_container.append(&btn);

    let spacer = Label::builder()
        .label("●")
        .css_classes(["dock-indicator"])
        .opacity(0.0)
        .valign(gtk4::Align::End)
        .build();
    item_container.append(&spacer);
    apply_dock_item_geometry(&item_container, DOCK_CURRENT_POSITION.get());

    (item_container, trash_img)
}

fn is_dock_item_running(running_cache: &Rc<RefCell<Vec<String>>>, app_cfg: &DockAppConfig) -> bool {
    find_matching_running_app(&running_cache.borrow(), &app_cfg.matchers, &app_cfg.name).is_some()
}

#[allow(clippy::too_many_arguments)]
fn populate_dock_app_menu_box(
    menu_box: &GtkBox,
    popover: &Popover,
    app_cfg: &DockAppConfig,
    is_pinned: bool,
    idx: usize,
    total_pinned: usize,
    is_running: bool,
    activate_or_spawn: &Rc<dyn Fn()>,
    minimized_cache: &Rc<RefCell<Vec<malus_de_common::ipc::MinimizedWindowInfo>>>,
    dock_config: &Rc<RefCell<DockConfig>>,
    on_refresh: &Rc<dyn Fn()>,
) {
    while let Some(child) = menu_box.first_child() {
        menu_box.remove(&child);
    }

    // 1. Título de la app en la cabecera
    let header = Label::builder()
        .label(&app_cfg.name)
        .css_classes(["popover-menu-header"])
        .halign(gtk4::Align::Start)
        .build();
    menu_box.append(&header);

    let sep1 = Separator::new(Orientation::Horizontal);
    sep1.add_css_class("popover-separator");
    menu_box.append(&sep1);

    // 2. Acción Abrir / Traer al frente
    let btn_open = make_dock_menu_button(
        if is_running {
            "Traer al frente"
        } else {
            "Abrir"
        },
        false,
    );
    let p_open = popover.clone();
    let act_open = activate_or_spawn.clone();
    btn_open.connect_clicked(move |_| {
        p_open.popdown();
        act_open();
    });
    menu_box.append(&btn_open);

    for (label, args) in dock_app_context_actions(app_cfg, is_running) {
        let btn_action = make_dock_menu_button(label, false);
        let p_action = popover.clone();
        let action_name = app_cfg.name.clone();
        let action_command = app_cfg.command.clone();
        btn_action.connect_clicked(move |_| {
            p_action.popdown();
            spawn_dock_app(&action_name, &action_command, args);
        });
        menu_box.append(&btn_action);
    }

    // 2b. Ventanas minimizadas en el icono de la app
    let app_minimized: Vec<malus_de_common::ipc::MinimizedWindowInfo> = minimized_cache
        .borrow()
        .iter()
        .filter(|w| window_matches_app(&app_cfg.matchers, &app_cfg.name, w))
        .cloned()
        .collect();
    if !app_minimized.is_empty() && dock_config.borrow().minimize_to_app_icon {
        let sep_min = Separator::new(Orientation::Horizontal);
        sep_min.add_css_class("popover-separator");
        menu_box.append(&sep_min);

        for win in app_minimized {
            let label_txt = format!("Mostrar: {}", win.title);
            let btn_win = make_dock_menu_button(&label_txt, false);
            let p_win = popover.clone();
            let win_id = win.window_id;
            btn_win.connect_clicked(move |_| {
                p_win.popdown();
                let _ = send_ipc_message(&IpcMessage::AnimateRestore { window_id: win_id });
            });
            menu_box.append(&btn_win);
        }
    }

    // 3. Opciones de anclado / organización estilo macOS
    if is_pinned {
        if idx > 0 {
            let btn_move_left = make_dock_menu_button("Mover a la izquierda", false);
            let p_mv_l = popover.clone();
            let cfg_mv_l = dock_config.clone();
            let on_rf_mv_l = on_refresh.clone();
            btn_move_left.connect_clicked(move |_| {
                p_mv_l.popdown();
                let mut cfg = cfg_mv_l.borrow_mut();
                if cfg.move_app(idx, idx - 1) {
                    let _ = cfg.save();
                    drop(cfg);
                    on_rf_mv_l();
                }
            });
            menu_box.append(&btn_move_left);
        }

        if idx + 1 < total_pinned {
            let btn_move_right = make_dock_menu_button("Mover a la derecha", false);
            let p_mv_r = popover.clone();
            let cfg_mv_r = dock_config.clone();
            let on_rf_mv_r = on_refresh.clone();
            btn_move_right.connect_clicked(move |_| {
                p_mv_r.popdown();
                let mut cfg = cfg_mv_r.borrow_mut();
                if cfg.move_app(idx, idx + 1) {
                    let _ = cfg.save();
                    drop(cfg);
                    on_rf_mv_r();
                }
            });
            menu_box.append(&btn_move_right);
        }

        let btn_unpin = make_dock_menu_button("Eliminar del Dock", false);
        let p_unpin = popover.clone();
        let cfg_unpin = dock_config.clone();
        let name_unpin = app_cfg.name.clone();
        let on_rf_unpin = on_refresh.clone();
        btn_unpin.connect_clicked(move |_| {
            p_unpin.popdown();
            let mut cfg = cfg_unpin.borrow_mut();
            if cfg.unpin_app(&name_unpin) {
                let _ = cfg.save();
                drop(cfg);
                on_rf_unpin();
            }
        });
        menu_box.append(&btn_unpin);
    } else {
        let btn_pin = make_dock_menu_button("Mantener en el Dock", false);
        let p_pin = popover.clone();
        let cfg_pin = dock_config.clone();
        let app_to_pin = app_cfg.clone();
        let on_rf_pin = on_refresh.clone();
        btn_pin.connect_clicked(move |_| {
            p_pin.popdown();
            let mut cfg = cfg_pin.borrow_mut();
            if cfg.pin_app(app_to_pin.clone()) {
                let _ = cfg.save();
                drop(cfg);
                on_rf_pin();
            }
        });
        menu_box.append(&btn_pin);
    }

    // 4. Forzar salida si está en ejecución
    if is_running {
        let sep2 = Separator::new(Orientation::Horizontal);
        sep2.add_css_class("popover-separator");
        menu_box.append(&sep2);

        let btn_force_quit = make_dock_menu_button("Forzar salida", true);
        let p_fq = popover.clone();
        let matchers_fq = app_cfg.matchers.clone();
        let cmd_fq = app_cfg.command.clone();
        let app_name_fq = app_cfg.name.clone();
        btn_force_quit.connect_clicked(move |_| {
            p_fq.popdown();
            info!("🛑 Forzando salida de {}", app_name_fq);
            for m in &matchers_fq {
                let pattern = m.trim();
                if pattern.len() < 3
                    || pattern.starts_with('-')
                    || !pattern
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                {
                    continue;
                }
                let _ = Command::new("pkill").args(["-x", "--", pattern]).status();
            }
            let parts: Vec<&str> = cmd_fq.split_whitespace().collect();
            if let Some((exe, _)) = parts.split_first() {
                if let Some(base) = std::path::Path::new(exe).file_name().and_then(|n| n.to_str())
                {
                    if !base.is_empty() && !base.starts_with('-') {
                        let _ = Command::new("killall").args(["--", base]).status();
                    }
                }
            }
        });
        menu_box.append(&btn_force_quit);
    }

    if let Some(last_item) = menu_box.last_child() {
        last_item.set_margin_bottom(10);
    }
}

fn create_dock_item(
    app_cfg: &DockAppConfig,
    is_pinned: bool,
    idx: usize,
    total_pinned: usize,
    icon_size: i32,
    is_running: bool,
    running_cache: Rc<RefCell<Vec<String>>>,
    active_app_cache: Rc<RefCell<Vec<String>>>,
    minimized_cache: Rc<RefCell<Vec<malus_de_common::ipc::MinimizedWindowInfo>>>,
    dock_config: Rc<RefCell<DockConfig>>,
    on_refresh: Rc<dyn Fn()>,
) -> (GtkBox, Label) {
    let item_container = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["dock-item-box"])
        .spacing(0)
        .valign(gtk4::Align::End)
        .build();
    item_container.set_overflow(gtk4::Overflow::Visible);

    let btn = Button::builder()
        .css_classes(["dock-item-btn"])
        .focusable(false)
        .valign(gtk4::Align::End)
        .build();
    btn.set_overflow(gtk4::Overflow::Visible);

    let icon_name = match app_cfg.icon.as_str() {
        "system-file-manager" => "org.gnome.Nautilus",
        "utilities-terminal" => "org.gnome.Ptyxis",
        "accessories-text-editor" => "org.gnome.TextEditor",
        icon_name => icon_name,
    };
    let icon = if icon_name.starts_with('/') {
        Image::builder()
            .file(icon_name)
            .pixel_size(icon_size)
            .css_classes(["dock-primary-icon"])
            .valign(gtk4::Align::End)
            .build()
    } else {
        Image::builder()
            .icon_name(icon_name)
            .pixel_size(icon_size)
            .css_classes(["dock-primary-icon"])
            .valign(gtk4::Align::End)
            .build()
    };
    let render_mode =
        resolve_dock_icon_render_mode(app_cfg.icon_appearance, icon_name, &icon, icon_size);
    if matches!(render_mode, DockIconRenderMode::ContainerBackdrop) {
        icon.add_css_class("dock-icon-inset");
        icon.set_pixel_size((icon_size - 10).max(1));
        icon.set_margin_bottom(5);
    }
    let icon_container = create_magnifiable_icon(&icon, icon_size, render_mode);
    btn.set_child(Some(&icon_container));
    icon_container.add_css_class("dock-bounce-target");
    let animation_id = Rc::new(Cell::new(0_u64));
    let cmd = app_cfg.command.clone();
    let name = app_cfg.name.clone();
    let matchers = app_cfg.matchers.clone();
    let cache = running_cache;

    let activate_or_spawn: Rc<dyn Fn()> = {
        let cmd = cmd.clone();
        let name = name.clone();
        let matchers = matchers.clone();
        let cache = cache.clone();
        let active_app_cache = active_app_cache.clone();
        let minimized_cache = minimized_cache.clone();
        let icon_container = icon_container.clone().upcast::<gtk4::Widget>();
        let animation_id = animation_id.clone();
        let dock_config_click = dock_config.clone();
        Rc::new(move || {
            let has_minimized_for_app = minimized_cache
                .borrow()
                .iter()
                .any(|w| window_matches_app(&matchers, &name, w));

            let active_app_matches = active_app_cache
                .borrow()
                .iter()
                .any(|identity| matches_dock_app(&matchers, &name, identity));
            if active_app_matches && !has_minimized_for_app {
                info!("{} ya tiene el foco; no se envía acción de dock", name);
                return;
            }

            let running = cache.borrow();
            let matched_running = find_matching_running_app(&running, &matchers, &name);
            drop(running);

            if let Some(running_id) = matched_running.or_else(|| has_minimized_for_app.then(|| name.clone())) {
                info!(
                    "App ya en ejecución, enviando IPC ActivateApp para {}",
                    name
                );
                let _ = send_ipc_message(&IpcMessage::ActivateApp { app_id: running_id });
                return;
            }

            if dock_config_click.borrow().animate_opening {
                bounce_dock_icon(
                    &icon_container,
                    animation_id.clone(),
                    Some(cache.clone()),
                    matchers.clone(),
                );
            }
            info!("Lanzando aplicación desde Dock: {} (`{}`)", name, cmd);
            spawn_dock_app(&name, &cmd, &[]);
        })
    };

    let act_click = activate_or_spawn.clone();
    btn.connect_clicked(move |_| {
        act_click();
    });

    // Menú Popover de Clic Derecho (macOS Style Context Menu)
    let popover = Popover::builder()
        .css_classes(["malus-popover-menu"])
        .has_arrow(false)
        .position(gtk4::PositionType::Top)
        .build();
    popover.set_offset(0, POPOVER_SHADOW_GUTTER);

    let menu_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .build();

    let app_cfg_menu = app_cfg.clone();
    populate_dock_app_menu_box(
        &menu_box,
        &popover,
        &app_cfg_menu,
        is_pinned,
        idx,
        total_pinned,
        is_running,
        &activate_or_spawn,
        &minimized_cache,
        &dock_config,
        &on_refresh,
    );

    let popover_surface = create_popover_surface(&menu_box);
    popover.set_child(Some(&popover_surface));
    popover.set_parent(&btn);
    let p_unroot = popover.clone();
    btn.connect_unrealize(move |_| {
        p_unroot.unparent();
    });

    let right_click = GestureClick::new();
    right_click.set_button(3); // Clic derecho
    let p_rc = popover.clone();
    let menu_box_rc = menu_box.clone();
    let cache_rc = cache.clone();
    let min_rc = minimized_cache.clone();
    let cfg_rc = dock_config.clone();
    let on_rf_rc = on_refresh.clone();
    let act_rc = activate_or_spawn.clone();
    right_click.connect_pressed(move |_, _, _, _| {
        let live_running = is_dock_item_running(&cache_rc, &app_cfg_menu);
        populate_dock_app_menu_box(
            &menu_box_rc,
            &p_rc,
            &app_cfg_menu,
            is_pinned,
            idx,
            total_pinned,
            live_running,
            &act_rc,
            &min_rc,
            &cfg_rc,
            &on_rf_rc,
        );
        apply_popover_dock_placement(&p_rc, DOCK_CURRENT_POSITION.get());
        p_rc.popup();
    });
    btn.add_controller(right_click);

    item_container.append(&btn);

    // Indicador de app en ejecución (Punto iluminado `●`)
    let show_indicator = is_running && dock_config.borrow().show_indicators;
    let indicator = Label::builder()
        .label("●")
        .css_classes(["dock-indicator"])
        .opacity(if show_indicator { 1.0 } else { 0.0 })
        .valign(gtk4::Align::End)
        .build();
    item_container.append(&indicator);
    apply_dock_item_geometry(&item_container, DOCK_CURRENT_POSITION.get());

    (item_container, indicator)
}

fn spawn_dock_app(name: &str, cmd: &str, extra_args: &[&str]) {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if let Some((exe, args)) = parts.split_first() {
        let target_path_buf;
        let mut wrapped_target_bin: Option<&str> = None;
        let target_exe = if *exe == "gnome-control-center" {
            "malus-de-settings"
        } else if let Some(wrapper) = malus_de_common::config::resolve_electron_ide_wrapper(exe) {
            wrapped_target_bin = Some(*exe);
            target_path_buf = wrapper.to_string_lossy().to_string();
            &target_path_buf
        } else {
            *exe
        };
        let resolved_exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join(target_exe)))
            .filter(|p| p.exists())
            .unwrap_or_else(|| std::path::PathBuf::from(target_exe));
        let mut command = Command::new(&resolved_exe);
        command.args(args).args(extra_args);
        malus_de_common::config::apply_isolated_environment(&mut command);
        if let Some(orig_exe) = wrapped_target_bin {
            command.env("MALUS_IDE_BIN", orig_exe);
        }
        match command.spawn() {
            Ok(mut child) => {
                let pid = child.id();
                info!("🚀 Proceso iniciado para {}: PID={}", name, pid);
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(err) => {
                error!(
                    "❌ Error al lanzar aplicación {} desde Dock: {:?}",
                    name, err
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, command: &str) -> DockAppConfig {
        DockAppConfig {
            name: name.to_string(),
            icon: String::new(),
            command: command.to_string(),
            matchers: Vec::new(),
            icon_appearance: DockIconAppearance::Auto,
        }
    }

    #[test]
    fn brave_has_incognito_action() {
        assert_eq!(
            dock_app_context_actions(&app("Brave", "brave-browser"), false),
            vec![("Abrir en modo incógnito", &["--incognito"][..])]
        );
    }

    #[test]
    fn code_editors_have_empty_window_action() {
        for app in [
            app("Visual Studio Code", "code"),
            app("Antigravity", "antigravity"),
        ] {
            assert_eq!(
                dock_app_context_actions(&app, false),
                vec![("Abrir nueva ventana vacía", &["--new-window"][..])]
            );
        }
    }

    #[test]
    fn nautilus_has_new_window_action() {
        assert_eq!(
            dock_app_context_actions(&app("Archivos", "nautilus"), false),
            vec![("Abrir nueva ventana", &["--new-window"][..])]
        );
    }

    #[test]
    fn branches_has_new_window_action_when_running() {
        let branches = app("Branches", "malus-de-branches");
        assert!(dock_app_context_actions(&branches, false).is_empty());
        assert_eq!(
            dock_app_context_actions(&branches, true),
            vec![("Abrir nueva ventana", &["--new-window"][..])]
        );
    }

    #[test]
    fn is_dock_item_running_tracks_live_running_cache() {
        let branches = DockAppConfig {
            name: "Branches".to_string(),
            icon: "system-file-manager".to_string(),
            command: "malus-de-branches".to_string(),
            matchers: vec!["dev.malus.Branches".to_string()],
            icon_appearance: DockIconAppearance::Auto,
        };
        let running_cache = Rc::new(RefCell::new(Vec::new()));

        assert!(!is_dock_item_running(&running_cache, &branches));
        assert!(dock_app_context_actions(
            &branches,
            is_dock_item_running(&running_cache, &branches)
        )
        .is_empty());

        running_cache
            .borrow_mut()
            .push("dev.malus.Branches".to_string());
        let live_running = is_dock_item_running(&running_cache, &branches);
        assert!(live_running);
        assert_eq!(
            dock_app_context_actions(&branches, live_running),
            vec![("Abrir nueva ventana", &["--new-window"][..])]
        );
    }

    #[test]
    fn active_app_identity_matches_dock_matchers_case_insensitively() {
        assert!(matches_dock_app(
            &["org.gnome.nautilus".to_string()],
            "Archivos",
            "org.gnome.Nautilus"
        ));
        assert!(!matches_dock_app(
            &["org.gnome.nautilus".to_string()],
            "Archivos",
            "org.gnome.Calculator"
        ));
    }

    #[test]
    fn resolves_actual_running_identity_for_branches_activation() {
        let (_name, _icon, _cmd, matchers) = resolve_desktop_app_info("dev.malus.Branches");
        let running = vec![
            "org.gnome.Ptyxis".to_string(),
            "dev.malus.Branches".to_string(),
        ];
        assert_eq!(
            find_matching_running_app(&running, &matchers, "Branches").as_deref(),
            Some("dev.malus.Branches")
        );
    }

    #[test]
    fn circular_or_freeform_alpha_icon_requires_container_backdrop() {
        let size = 64usize;
        let stride = size * 4;
        let mut pixels = vec![0u8; size * stride];
        let center = (size as f64 - 1.0) * 0.5;
        let radius = size as f64 * 0.49;

        for y in 0..size {
            for x in 0..size {
                let dx = x as f64 - center;
                let dy = y as f64 - center;
                if dx * dx + dy * dy <= radius * radius {
                    let offset = y * stride + x * 4;
                    pixels[offset] = 52;
                    pixels[offset + 1] = 120;
                    pixels[offset + 2] = 246;
                    pixels[offset + 3] = 255;
                }
            }
        }

        // Although a circle covers ~75% of the canvas (>72%), its shoulders are transparent
        // so it must NOT be treated as a pre-filled dock tile.
        assert!(
            analyze_dock_icon_pixels(&pixels, size, size, stride, 4, true).is_none(),
            "Un icono circular o con canal alfa libre debe recibir el contenedor del Dock"
        );
    }

    #[test]
    fn hollow_symbol_icon_requires_container_backdrop() {
        let size = 64usize;
        let stride = size * 4;
        let mut pixels = vec![0u8; size * stride];

        for y in 4..(size - 4) {
            for x in 4..(size - 4) {
                // Agujero transparente en el centro (como un símbolo o letra calada)
                if (24..40).contains(&x) && (24..40).contains(&y) {
                    continue;
                }
                let offset = y * stride + x * 4;
                pixels[offset] = 240;
                pixels[offset + 1] = 90;
                pixels[offset + 2] = 40;
                pixels[offset + 3] = 255;
            }
        }

        assert!(
            analyze_dock_icon_pixels(&pixels, size, size, stride, 4, true).is_none(),
            "Un icono con huecos internos transparentes debe recibir el contenedor del Dock"
        );
    }

    #[test]
    fn solid_rounded_tile_is_detected_and_corner_padded_for_squircle_clip() {
        let size = 64usize;
        let stride = size * 4;
        let mut pixels = vec![0u8; size * stride];
        let margin = 4isize;
        let corner_r = 10isize;
        let min_c = margin + corner_r;
        let max_c = (size as isize - 1 - margin) - corner_r;

        for y in margin..(size as isize - margin) {
            for x in margin..(size as isize - margin) {
                let dx = if x < min_c {
                    min_c - x
                } else if x > max_c {
                    x - max_c
                } else {
                    0
                };
                let dy = if y < min_c {
                    min_c - y
                } else if y > max_c {
                    y - max_c
                } else {
                    0
                };
                if dx * dx + dy * dy <= corner_r * corner_r {
                    let offset = (y as usize) * stride + (x as usize) * 4;
                    pixels[offset] = 38;
                    pixels[offset + 1] = 162;
                    pixels[offset + 2] = 105;
                    pixels[offset + 3] = 255;
                }
            }
        }

        let mut surface = analyze_dock_icon_pixels(&pixels, size, size, stride, 4, true)
            .expect("Un tile sólido cuadrado/redondeado debe detectarse como icono con relleno");
        assert_eq!(surface.width(), 56);
        assert_eq!(surface.height(), 56);

        let dst_stride = surface.stride() as usize;
        let data = surface.data().expect("surface data");
        // Verificamos que las 4 esquinas del bounding box hayan sido rellenadas con alfa 255
        for (cx, cy) in [(0usize, 0usize), (55, 0), (0, 55), (55, 55)] {
            let idx = cy * dst_stride + cx * 4;
            let pixel =
                u32::from_ne_bytes([data[idx], data[idx + 1], data[idx + 2], data[idx + 3]]);
            let alpha = (pixel >> 24) & 0xff;
            assert_eq!(
                alpha, 255,
                "La esquina ({cx}, {cy}) debe quedar rellena para el recorte squircle"
            );
        }
    }

    #[test]
    fn resolve_desktop_app_info_maps_malus_settings() {
        let (name, icon, exec, matchers) = resolve_desktop_app_info("dev.malus.Settings");
        assert_eq!(name, "Ajustes del Sistema");
        assert_eq!(icon, "preferences-system");
        assert_eq!(exec, "malus-de-settings");
        assert!(matchers.iter().any(|m| m.contains("malus")));
    }

    #[test]
    fn resolve_desktop_app_info_does_not_crash_on_unknown_app_with_null_executables() {
        // En Linux/GNOME existen archivos desktop (como qemu.desktop) sin clave Exec
        // donde g_app_info_get_executable devuelve NULL. La función debe manejarlo
        // de forma segura sin provocar un segfault en strlen().
        let (name, icon, exec, _) = resolve_desktop_app_info("nonexistent-unpinned-app-test");
        assert_eq!(name, "Nonexistent-unpinned-app-test");
        assert_eq!(icon, "nonexistent-unpinned-app-test");
        assert_eq!(exec, "nonexistent-unpinned-app-test");
    }

    #[test]
    fn pin_desktop_apps_from_branches_drop_inserts_into_dock_config() {
        let temp_dir = std::env::temp_dir().join("malus_dock_drop_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let desktop_path = temp_dir.join("org.example.BranchesDrop.desktop");
        std::fs::write(
            &desktop_path,
            "[Desktop Entry]\nType=Application\nName=Drop App\nExec=drop-app %u\nIcon=drop-app-icon\n",
        )
        .unwrap();

        let file_list = gdk::FileList::from_array(&[gio::File::for_path(&desktop_path)]);
        let apps = extract_dropped_desktop_apps(&file_list);
        assert_eq!(apps.len(), 1);

        let mut cfg = DockConfig {
            apps: vec![DockAppConfig {
                name: "Existing".to_string(),
                icon: "existing".to_string(),
                command: "existing".to_string(),
                matchers: vec!["existing".to_string()],
                icon_appearance: DockIconAppearance::Auto,
            }],
            ..DockConfig::default()
        };

        assert!(pin_desktop_apps_at(&mut cfg, apps, 0));
        assert_eq!(cfg.apps.len(), 2);
        assert_eq!(cfg.apps[0].name, "Drop App");
        assert_eq!(cfg.apps[0].command, "drop-app");
        assert_eq!(cfg.apps[0].icon, "drop-app-icon");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dock_css_suppresses_gtk_drop_active_outline_and_border() {
        assert!(DOCK_CSS.contains(".dock-root:drop(active)"));
        let block = DOCK_CSS
            .split(".dock-root:drop(active)")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .expect("Debe existir regla :drop(active) en dock.css");
        assert!(block.contains("border: none;"));
        assert!(block.contains("outline: none;"));
        assert!(block.contains("box-shadow: none;"));
    }

    #[test]
    fn extract_trashable_paths_filters_root_home_and_trash_and_keeps_regular_files() {
        let temp_dir = std::env::temp_dir().join("malus_dock_trash_extract_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("borrar.txt");
        let folder_path = temp_dir.join("carpeta");
        std::fs::write(&file_path, "hola").unwrap();
        std::fs::create_dir_all(&folder_path).unwrap();

        assert!(!is_trashable_path(Path::new("/")));
        assert!(!is_trashable_path(Path::new(
            "/home/user/.local/share/Trash/files/a"
        )));
        assert!(is_trashable_path(&file_path));
        assert!(is_trashable_path(&folder_path));

        let file_list = gdk::FileList::from_array(&[
            gio::File::for_path(Path::new("/")),
            gio::File::for_path(&file_path),
            gio::File::for_path(&folder_path),
        ]);
        let extracted = extract_trashable_paths(&file_list);
        assert_eq!(extracted, vec![file_path, folder_path]);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn finds_app_icon_matcher_for_minimized_window_preferring_app_id() {
        let groups = vec![
            vec!["malus-de-branches".to_string(), "branches".to_string()],
            vec!["ptyxis".to_string(), "org.gnome.ptyxis".to_string()],
            vec!["antigravity".to_string()],
        ];
        // Aunque el título mencione otra app ("Antigravity"), debe coincidir primero por app_id.
        assert_eq!(
            find_matching_app_index_for_window(&groups, "org.gnome.Ptyxis", "nvim Antigravity"),
            Some(1)
        );
        assert_eq!(
            find_matching_app_index_for_window(&groups, "", "Branches"),
            Some(0)
        );
        assert_eq!(
            find_matching_app_index_for_window(&groups, "org.gnome.Calculator", "Calculadora"),
            None
        );
    }
}
