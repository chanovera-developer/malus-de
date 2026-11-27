use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, DropTarget, EventControllerKey, GestureClick, Grid, Image, Label,
    Orientation, Overlay, PolicyType, Scale, ScrolledWindow, Separator, Spinner, Stack,
    StackTransitionType, ToggleButton, Window,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use malus_de_common::config::{ControlCenterConfig, ControlWidgetKind};
use malus_de_common::ipc::{GlassRegion, IpcClient, IpcMessage};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::process::Command;
use std::rc::Rc;
use tracing::info;

use crate::services::audio::{AudioService, MprisService};
use crate::services::bluetooth::{BluetoothService, BtDeviceInfo};
use crate::services::brightness::BrightnessService;
use crate::services::network::{NetworkService, WifiApInfo};
use crate::ui::cc_dnd::{
    apply_squircle_glass_button, apply_squircle_glass_fill_button, attach_drag_source,
    unpin_widget_from_tray, wrap_squircle_glass_container, DragPayload,
};
use crate::ui::control_center_editor::{
    create_control_center_editor_window, ControlCenterEditorHandle,
};

struct ControlCenterRuntime {
    window: Window,
    toggle_btn: ToggleButton,
    edit_mode: Rc<Cell<bool>>,
    is_dragging: Rc<Cell<bool>>,
    editor: Rc<RefCell<Option<ControlCenterEditorHandle>>>,
    sync_glass: Rc<dyn Fn()>,
    rebuild_grid: Rc<dyn Fn()>,
    refresh_all: Rc<dyn Fn()>,
    set_tiling_state: Rc<dyn Fn(bool)>,
    sync_theme_fn: Rc<dyn Fn(bool)>,
    set_edit_mode_fn: Rc<dyn Fn(bool)>,
}

thread_local! {
    static CC_RUNTIME: RefCell<Option<ControlCenterRuntime>> = const { RefCell::new(None) };
}

/// Reconstruye la cuadrícula del Centro de Control y refresca la galería «Editar controles» si está abierta.
pub fn refresh_control_center_layout() {
    CC_RUNTIME.with(|cell| {
        if let Some(rt) = cell.borrow().as_ref() {
            (rt.rebuild_grid)();
            if let Some(ref ed) = *rt.editor.borrow() {
                ed.refresh();
            }
        }
    });
}

/// Redibuja las superficies squircle del Centro de Control (al cambiar opacidad Liquid Glass o tema).
pub fn redraw_control_center_surfaces() {
    CC_RUNTIME.with(|cell| {
        if let Some(rt) = cell.borrow().as_ref() {
            rt.window.queue_draw();
            if let Some(ref ed) = *rt.editor.borrow() {
                ed.window.queue_draw();
            }
        }
    });
}

/// Abre el Centro de Control si estaba cerrado (usado cuando el usuario arrastra un icono de la bandeja).
pub fn open_control_center_for_drag() {
    CC_RUNTIME.with(|cell| {
        if let Some(rt) = cell.borrow().as_ref() {
            if !rt.window.is_visible() {
                rt.toggle_btn.set_active(true);
            }
        }
    });
}

/// Alterna la visibilidad del Centro de Control (invocado por atajo o IPC).
#[allow(dead_code)]
pub fn toggle_control_center() {
    CC_RUNTIME.with(|cell| {
        if let Some(rt) = cell.borrow().as_ref() {
            let next = !rt.window.is_visible();
            rt.toggle_btn.set_active(next);
        }
    });
}

/// Cierra el Centro de Control cuando el usuario enfoca otra aplicación (salvo si está en modo edición o arrastrando).
pub fn close_control_center_if_not_editing() {
    CC_RUNTIME.with(|cell| {
        if let Some(rt) = cell.borrow().as_ref() {
            if rt.window.is_visible() && !rt.edit_mode.get() && !rt.is_dragging.get() {
                rt.toggle_btn.set_active(false);
            }
        }
    });
}

/// Sincroniza el tema claro/oscuro en el Centro de Control y su ventana «Editar controles».
pub fn sync_control_center_theme(is_dark: bool) {
    CC_RUNTIME.with(|cell| {
        if let Some(rt) = cell.borrow().as_ref() {
            (rt.sync_theme_fn)(is_dark);
            if let Some(ref ed) = *rt.editor.borrow() {
                ed.sync_theme(is_dark);
            }
        }
    });
}

/// Actualiza el estado visual del botón Organizador Visual / Auto-Tiling en el Centro de Control.
pub fn set_auto_tiling_cc_state(enabled: bool) {
    CC_RUNTIME.with(|cell| {
        if let Some(rt) = cell.borrow().as_ref() {
            (rt.set_tiling_state)(enabled);
        }
    });
}

pub fn create_control_center_widget() -> ToggleButton {
    let toggle_btn = ToggleButton::builder()
        .css_classes(["control-center-toggle", "topbar-btn"])
        .tooltip_text("Centro de Control")
        .has_frame(false)
        .build();

    let cc_icon = Image::builder()
        .icon_name("control-center-symbolic")
        .pixel_size(16)
        .halign(Align::Center)
        .valign(Align::Center)
        .build();
    toggle_btn.set_child(Some(&cc_icon));

    let initialized = Rc::new(Cell::new(false));
    let ensure_init: Rc<dyn Fn(&ToggleButton)> = {
        let init_flag = initialized.clone();
        Rc::new(move |btn: &ToggleButton| {
            if init_flag.replace(true) {
                return;
            }
            let app = btn
                .root()
                .and_then(|r| r.downcast::<gtk4::Window>().ok())
                .and_then(|w| w.application());
            build_control_center_overlay(btn, app.as_ref());
        })
    };

    let init_on_toggle = ensure_init.clone();
    toggle_btn.connect_toggled(move |btn| {
        init_on_toggle(btn);
        let active = btn.is_active();
        CC_RUNTIME.with(|cell| {
            if let Some(rt) = cell.borrow().as_ref() {
                if active {
                    sync_cc_top_margin(&rt.window);
                    (rt.refresh_all)();
                    rt.window.present();
                    (rt.sync_glass)();
                } else {
                    if rt.edit_mode.get() {
                        (rt.set_edit_mode_fn)(false);
                    }
                    rt.window.set_visible(false);
                    let _ = IpcClient::send_command(IpcMessage::SetControlCenterGlassRegions {
                        regions: Vec::new(),
                    });
                }
            }
        });
    });

    // Precalentar la ventana flotante oculta tras 350ms para que abrir el Centro de Control sea instantáneo
    let btn_warm = toggle_btn.clone();
    let init_warm = ensure_init;
    glib::timeout_add_local_once(std::time::Duration::from_millis(350), move || {
        init_warm(&btn_warm);
    });

    toggle_btn
}

const CC_TILE_SQUIRCLE_RADIUS: f64 = 32.0;
const CC_SUBVIEW_SQUIRCLE_RADIUS: f64 = 32.0;
const CC_EDIT_BTN_SQUIRCLE_RADIUS: f64 = 13.0;

/// Desactiva el cambio accidental por rueda de ratón o scroll de trackpad en un `GtkScale`,
/// dejando que el desplazamiento fluya normalmente hacia cualquier `ScrolledWindow` padre.
pub fn disable_scale_scroll(scale: &Scale) {
    let controllers = scale.observe_controllers();
    for idx in 0..controllers.n_items() {
        if let Some(obj) = controllers.item(idx) {
            if let Ok(scroll_ctrl) = obj.downcast::<gtk4::EventControllerScroll>() {
                scroll_ctrl.set_propagation_phase(gtk4::PropagationPhase::None);
            }
        }
    }
}

fn sync_cc_top_margin(cc_window: &Window) {
    if !gtk4_layer_shell::is_supported() {
        return;
    }
    let auto_hide = malus_de_common::config::MenuBarConfig::load().auto_hide_mode;
    // Alinear el borde superior del Centro de Control exactamente con los popovers de Wi-Fi/Bluetooth/Sonido (y ≈ 36 px).
    let top_margin = if auto_hide == 1 || auto_hide == 2 {
        36
    } else {
        0
    };
    cc_window.set_margin(Edge::Top, top_margin);
}

fn build_control_center_overlay(toggle_btn: &ToggleButton, app: Option<&gtk4::Application>) {
    let mut win_builder = Window::builder()
        .title("Centro de Control")
        .decorated(false)
        .resizable(false)
        .css_classes(["cc-floating-window"]);
    if let Some(a) = app {
        win_builder = win_builder.application(a);
    }
    let cc_window = win_builder.build();
    if !malus_de_common::config::is_dark_mode_active() {
        cc_window.add_css_class("light-mode");
    }

    if gtk4_layer_shell::is_supported() {
        cc_window.init_layer_shell();
        cc_window.set_layer(Layer::Overlay);
        cc_window.set_namespace("malus-control-center");
        cc_window.set_anchor(Edge::Top, true);
        cc_window.set_anchor(Edge::Right, true);
        sync_cc_top_margin(&cc_window);
        cc_window.set_margin(Edge::Right, 10);
        cc_window.set_keyboard_mode(KeyboardMode::OnDemand);
    }

    let stack = Stack::builder()
        .transition_type(StackTransitionType::SlideLeftRight)
        .transition_duration(200)
        .interpolate_size(true)
        .vhomogeneous(false)
        .hhomogeneous(false)
        .css_classes(["cc-floating-stack"])
        .build();

    let edit_mode = Rc::new(Cell::new(false));
    let is_dragging = Rc::new(Cell::new(false));
    let cell_coords: Rc<RefCell<HashMap<ControlWidgetKind, (u8, u8)>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let glass_targets: Rc<RefCell<Vec<(gtk4::Widget, f32, bool)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let editor_handle: Rc<RefCell<Option<ControlCenterEditorHandle>>> =
        Rc::new(RefCell::new(None));

    // ==========================================
    // Construcción de los 11 controles visibles (todos con bordes squircle n=4.2)
    // ==========================================
    let (wifi_inner, wifi_power_btn, wifi_nav_btn, wifi_icon, wifi_sub) =
        create_tahoe_connectivity_pill("network-wireless-symbolic", "Wi-Fi", "Buscando...", true);
    let wifi_surface = wrap_squircle_glass_container(&wifi_inner, CC_TILE_SQUIRCLE_RADIUS);
    let (bt_inner, bt_power_btn, bt_nav_btn, bt_icon, bt_sub) =
        create_tahoe_connectivity_pill("bluetooth-active-symbolic", "Bluetooth", "Activado", true);
    let bt_surface = wrap_squircle_glass_container(&bt_inner, CC_TILE_SQUIRCLE_RADIUS);
    let (airdrop_inner, airdrop_power_btn, airdrop_nav_btn, airdrop_icon, airdrop_sub) =
        create_tahoe_connectivity_pill(
            "network-wireless-hotspot-symbolic",
            "AirDrop",
            "Todos",
            false,
        );
    let airdrop_surface = wrap_squircle_glass_container(&airdrop_inner, CC_TILE_SQUIRCLE_RADIUS);

    let (media_inner, refresh_media) = create_tahoe_now_playing_card();
    let media_surface = wrap_squircle_glass_container(&media_inner, CC_TILE_SQUIRCLE_RADIUS);

    let (stage_btn, stage_icon, stage_active_flag) = create_tahoe_circle_control(
        "view-paged-symbolic",
        "Organizador visual / Auto-Tiling",
    );

    let (mirror_btn, _, _) = create_tahoe_circle_control(
        "video-display-symbolic",
        "Duplicar pantalla y disposición de monitores",
    );

    let (dark_btn, _, dark_state_flag) =
        create_tahoe_circle_control("dark-mode-symbolic", "Modo oscuro / claro");

    let (desktop_btn, _, _) = create_tahoe_circle_control(
        "user-desktop-symbolic",
        "Escritorio y Finder (Clic: Mostrar escritorio · Clic derecho: Branches)",
    );

    let (focus_btn, focus_icon_circle, focus_icon, focus_title, focus_sub) =
        create_tahoe_focus_pill();

    let current_disp_cfg = malus_de_common::config::DisplaysConfig::load();
    let (display_inner, display_header, brightness_scale, disp_pct, disp_icon) =
        create_tahoe_display_slider_pill(&current_disp_cfg);
    let display_surface = wrap_squircle_glass_container(&display_inner, CC_TILE_SQUIRCLE_RADIUS);

    let (
        sound_inner,
        sound_header,
        volume_scale,
        sound_pct,
        vol_icon,
        vol_icon_low,
        vol_mute_btn,
        sound_output_btn,
    ) = create_tahoe_sound_slider_pill();
    let sound_surface = wrap_squircle_glass_container(&sound_inner, CC_TILE_SQUIRCLE_RADIUS);

    // Controles auxiliares para la subvista de pantallas
    let monitors_info = Label::builder()
        .label(&format_monitors_summary(&current_disp_cfg))
        .css_classes(["cc-tile-subtitle"])
        .build();
    let link_btn = Button::builder()
        .css_classes(["cc-pill-btn"])
        .has_frame(false)
        .build();
    let link_icon_label = Label::builder()
        .label(if current_disp_cfg.linked_physical_1to1 {
            "🔗 TrueSize 1:1"
        } else {
            "🔓 Escala Libre"
        })
        .build();
    link_btn.set_child(Some(&link_icon_label));
    let master_scale_slider = Scale::with_range(Orientation::Horizontal, 100.0, 250.0, 5.0);
    disable_scale_scroll(&master_scale_slider);
    master_scale_slider.set_value((current_disp_cfg.master_scale * 100.0).clamp(100.0, 250.0));
    let scale_row_val = Label::builder()
        .label(&format!("{:.1}%", current_disp_cfg.master_scale * 100.0))
        .build();
    let is_updating_master_scale = Rc::new(RefCell::new(false));
    let is_updating_volume = Rc::new(RefCell::new(false));
    let is_updating_brightness = Rc::new(RefCell::new(false));

    // ==========================================
    // Vista Principal ("main") — Cuadrícula flotante de 4 columnas
    // ==========================================
    let main_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-main-floating"])
        .spacing(12)
        .build();

    let grid = Grid::builder()
        .column_homogeneous(true)
        .column_spacing(10)
        .row_spacing(10)
        .width_request(310)
        .halign(Align::Center)
        .build();

    let edit_controls_lbl = Label::builder()
        .label("Editar controles")
        .css_classes(["cc-glass-edit-label"])
        .halign(Align::Center)
        .valign(Align::Center)
        .build();
    let edit_controls_btn = Button::builder()
        .css_classes(["cc-glass-edit-pill"])
        .halign(Align::Center)
        .valign(Align::Center)
        .hexpand(false)
        .vexpand(false)
        .has_frame(false)
        .child(&edit_controls_lbl)
        .build();
    apply_squircle_glass_button(&edit_controls_btn, CC_EDIT_BTN_SQUIRCLE_RADIUS, 5, 14);

    main_box.append(&grid);
    main_box.append(&edit_controls_btn);
    stack.add_named(&main_box, Some("main"));

    // ==========================================
    // Subvistas envueltas en tarjeta semitransparente squircle
    // ==========================================
    let wrap_subview = |inner: &GtkBox| -> Overlay {
        let card = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .css_classes(["cc-subview-glass-card"])
            .width_request(318)
            .build();
        card.append(inner);
        wrap_squircle_glass_container(&card, CC_SUBVIEW_SQUIRCLE_RADIUS)
    };

    let (wifi_subview, refresh_wifi) =
        create_wifi_subview(&stack, &cc_window, &wifi_power_btn, &wifi_icon, &wifi_sub);
    let wifi_card = wrap_subview(&wifi_subview);
    stack.add_named(&wifi_card, Some("wifi"));

    let (bt_subview, refresh_bt) =
        create_bt_subview(&stack, &cc_window, &bt_power_btn, &bt_icon, &bt_sub);
    let bt_card = wrap_subview(&bt_subview);
    stack.add_named(&bt_card, Some("bluetooth"));

    let (airdrop_subview, refresh_airdrop) = create_airdrop_subview(
        &stack,
        &cc_window,
        &airdrop_power_btn,
        &airdrop_icon,
        &airdrop_sub,
    );
    let airdrop_card = wrap_subview(&airdrop_subview);
    stack.add_named(&airdrop_card, Some("airdrop"));

    let (displays_subview, refresh_displays_subview) = create_displays_subview(
        &stack,
        &cc_window,
        &brightness_scale,
        &disp_pct,
        &is_updating_brightness,
        &dark_btn,
        &monitors_info,
        &link_btn,
        &link_icon_label,
        &master_scale_slider,
        &scale_row_val,
        &is_updating_master_scale,
    );
    let displays_card = wrap_subview(&displays_subview);
    stack.add_named(&displays_card, Some("displays"));

    let (sound_subview, refresh_sound_subview) = create_sound_subview(
        &stack,
        &cc_window,
        &volume_scale,
        &sound_pct,
        &vol_icon,
        &is_updating_volume,
    );
    let sound_card = wrap_subview(&sound_subview);
    stack.add_named(&sound_card, Some("sound"));

    // 9. Estado visual del tile de Enfoque (compartido con la subvista selector de perfiles)
    let update_focus_ui: Rc<dyn Fn()> = {
        let btn_c = focus_btn.clone();
        let circle_c = focus_icon_circle.clone();
        let ico_c = focus_icon.clone();
        let title_c = focus_title.clone();
        let sub_c = focus_sub.clone();
        Rc::new(move || {
            let focus_cfg = malus_de_common::config::FocusConfig::load();
            if let Some(mode) = focus_cfg.active_mode_now() {
                btn_c.add_css_class("cc-tile-active");
                circle_c.add_css_class("cc-focus-circle-active");
                ico_c.set_icon_name(Some(&mode.icon));
                title_c.set_label(&mode.name);
                sub_c.set_label("Activado");
            } else if is_focus_active() {
                btn_c.add_css_class("cc-tile-active");
                circle_c.add_css_class("cc-focus-circle-active");
                ico_c.set_icon_name(Some("weather-clear-night-symbolic"));
                title_c.set_label("No\nmolestar");
                sub_c.set_label("Activado");
            } else {
                btn_c.remove_css_class("cc-tile-active");
                circle_c.remove_css_class("cc-focus-circle-active");
                ico_c.set_icon_name(Some("weather-clear-night-symbolic"));
                title_c.set_label("Enfoque");
                sub_c.set_label("Desactivado");
            }
        })
    };

    let (focus_subview, refresh_focus_subview) =
        create_focus_subview(&stack, &cc_window, &update_focus_ui);
    let focus_card = wrap_subview(&focus_subview);
    stack.add_named(&focus_card, Some("focus"));

    cc_window.set_child(Some(&stack));

    // ==========================================
    // Sincronización de regiones GPU Liquid Glass (refracción por cápsula)
    // ==========================================
    let last_sent_glass: Rc<RefCell<Option<Vec<GlassRegion>>>> = Rc::new(RefCell::new(None));
    let transition_ticker_active: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let sync_glass: Rc<dyn Fn()> = {
        let win_c = cc_window.clone();
        let stack_c = stack.clone();
        let targets_c = glass_targets.clone();
        let last_sent_c = last_sent_glass.clone();
        let ticker_c = transition_ticker_active.clone();
        let subview_cards: HashMap<&'static str, gtk4::Widget> = HashMap::from([
            ("wifi", wifi_card.clone().upcast()),
            ("bluetooth", bt_card.clone().upcast()),
            ("airdrop", airdrop_card.clone().upcast()),
            ("displays", displays_card.clone().upcast()),
            ("sound", sound_card.clone().upcast()),
            ("focus", focus_card.clone().upcast()),
        ]);

        let compute_and_send = Rc::new(move || {
            let mut regions = Vec::new();
            if win_c.is_visible() {
                let win_w = win_c.width();
                let win_h = win_c.height();
                let page = stack_c.visible_child_name();
                if page.as_deref() == Some("main") {
                    for (w, radius, is_capsule) in targets_c.borrow().iter() {
                        if !w.is_visible() || !w.is_mapped() {
                            continue;
                        }
                        if let Some(bounds) = w.compute_bounds(&win_c) {
                            if let Some(region) = clip_glass_region_to_window(
                                bounds.x(),
                                bounds.y(),
                                bounds.width(),
                                bounds.height(),
                                win_w,
                                win_h,
                                *radius,
                                *is_capsule,
                            ) {
                                regions.push(region);
                            }
                        }
                    }
                } else if let Some(name) = page.as_deref() {
                    if let Some(card) = subview_cards.get(name) {
                        if let Some(bounds) = card.compute_bounds(&win_c) {
                            if let Some(region) = clip_glass_region_to_window(
                                bounds.x(),
                                bounds.y(),
                                bounds.width(),
                                bounds.height(),
                                win_w,
                                win_h,
                                CC_SUBVIEW_SQUIRCLE_RADIUS as f32,
                                false,
                            ) {
                                regions.push(region);
                            }
                        }
                    }
                }
            }

            let unchanged = last_sent_c
                .borrow()
                .as_ref()
                .is_some_and(|prev| glass_regions_equal(prev, &regions));
            if unchanged {
                return;
            }
            *last_sent_c.borrow_mut() = Some(regions.clone());
            let _ = IpcClient::send_command(IpcMessage::SetControlCenterGlassRegions { regions });
        });

        let send_now = compute_and_send;
        let stack_sched = stack.clone();
        let win_sched = cc_window.clone();
        Rc::new(move || {
            let s1 = send_now.clone();
            glib::idle_add_local_once(move || s1());
            for delay_ms in [45u64, 120, 240] {
                let s = send_now.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(delay_ms), move || {
                    s();
                });
            }

            // Durante la transición SlideLeftRight / interpolate_size (200 ms), seguir el movimiento
            // fotograma a fotograma y enviar la geometría definitiva al detenerse la transición.
            if stack_sched.is_transition_running() && !ticker_c.get() {
                ticker_c.set(true);
                let s_tick = send_now.clone();
                let st_tick = stack_sched.clone();
                let w_tick = win_sched.clone();
                let flag_tick = ticker_c.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
                    s_tick();
                    if w_tick.is_visible() && st_tick.is_transition_running() {
                        glib::ControlFlow::Continue
                    } else {
                        flag_tick.set(false);
                        let s_post1 = s_tick.clone();
                        glib::idle_add_local_once(move || s_post1());
                        let s_post2 = s_tick.clone();
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(50),
                            move || s_post2(),
                        );
                        glib::ControlFlow::Break
                    }
                });
            }
        })
    };

    // ==========================================
    // Envolver cada uno de los 11 widgets con Overlay (badge "−" en edición + DnD)
    // ==========================================
    struct TileEntry {
        overlay: Overlay,
        glass_widget: gtk4::Widget,
        remove_btn: Button,
        radius: f32,
    }

    let rebuild_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let mut tile_map: HashMap<ControlWidgetKind, TileEntry> = HashMap::new();

    let tile_r = CC_TILE_SQUIRCLE_RADIUS as f32;
    let all_specs: Vec<(ControlWidgetKind, gtk4::Widget, gtk4::Widget, f32)> = vec![
        (
            ControlWidgetKind::Wifi,
            wifi_surface.clone().upcast(),
            wifi_surface.clone().upcast(),
            tile_r,
        ),
        (
            ControlWidgetKind::Bluetooth,
            bt_surface.clone().upcast(),
            bt_surface.clone().upcast(),
            tile_r,
        ),
        (
            ControlWidgetKind::AirDrop,
            airdrop_surface.clone().upcast(),
            airdrop_surface.clone().upcast(),
            tile_r,
        ),
        (
            ControlWidgetKind::NowPlaying,
            media_surface.clone().upcast(),
            media_surface.clone().upcast(),
            tile_r,
        ),
        (
            ControlWidgetKind::StageManager,
            stage_btn.clone().upcast(),
            stage_btn.clone().upcast(),
            tile_r,
        ),
        (
            ControlWidgetKind::ScreenMirroring,
            mirror_btn.clone().upcast(),
            mirror_btn.clone().upcast(),
            tile_r,
        ),
        (
            ControlWidgetKind::DarkMode,
            dark_btn.clone().upcast(),
            dark_btn.clone().upcast(),
            tile_r,
        ),
        (
            ControlWidgetKind::DesktopFinder,
            desktop_btn.clone().upcast(),
            desktop_btn.clone().upcast(),
            tile_r,
        ),
        (
            ControlWidgetKind::Focus,
            focus_btn.clone().upcast(),
            focus_btn.clone().upcast(),
            tile_r,
        ),
        (
            ControlWidgetKind::Display,
            display_surface.clone().upcast(),
            display_header.clone().upcast(),
            tile_r,
        ),
        (
            ControlWidgetKind::Sound,
            sound_surface.clone().upcast(),
            sound_header.clone().upcast(),
            tile_r,
        ),
    ];

    for (kind, glass_widget, drag_handle, radius) in all_specs {
        let overlay = Overlay::new();
        overlay.set_child(Some(&glass_widget));

        let remove_btn = Button::builder()
            .label("−")
            .css_classes(["cc-edit-remove-badge"])
            .halign(Align::Start)
            .valign(Align::Start)
            .has_frame(false)
            .visible(false)
            .tooltip_text("Quitar del Centro de Control")
            .build();
        overlay.add_overlay(&remove_btn);

        // Quitar control al pulsar el botón "−"
        {
            let rb_slot = rebuild_slot.clone();
            let ed_c = editor_handle.clone();
            remove_btn.connect_clicked(move |_| {
                let mut cfg = ControlCenterConfig::load();
                if cfg.remove_widget(kind) {
                    let _ = cfg.save();
                    if let Some(ref f) = *rb_slot.borrow() {
                        f();
                    }
                    if let Some(ref ed) = *ed_c.borrow() {
                        ed.refresh();
                    }
                }
            });
        }

        // DragSource bidireccional (arrastrar al tray o reordenar dentro del Centro de Control)
        {
            let drag_flag_b = is_dragging.clone();
            let drag_flag_e = is_dragging.clone();
            let on_begin: Rc<dyn Fn()> = Rc::new(move || {
                drag_flag_b.set(true);
                crate::ui::status_tray::set_tray_drag_highlight(true);
            });
            let on_end: Rc<dyn Fn()> = Rc::new(move || {
                drag_flag_e.set(false);
                crate::ui::status_tray::set_tray_drag_highlight(false);
            });
            attach_drag_source(
                &drag_handle,
                DragPayload::FromControlCenter(kind),
                Some(on_begin),
                Some(on_end),
            );
        }

        // DropTarget sobre cada celda para reordenar o recibir widgets desde la bandeja
        {
            let drop_target = DropTarget::new(
                glib::Type::STRING,
                gdk::DragAction::COPY | gdk::DragAction::MOVE,
            );
            let gw_enter = glass_widget.clone();
            drop_target.connect_enter(move |_, _, _| {
                gw_enter.add_css_class("cc-drop-hover");
                gdk::DragAction::MOVE
            });
            let gw_leave = glass_widget.clone();
            drop_target.connect_leave(move |_| {
                gw_leave.remove_css_class("cc-drop-hover");
            });
            let gw_drop = glass_widget.clone();
            let coords_c = cell_coords.clone();
            let rb_slot = rebuild_slot.clone();
            let ed_c = editor_handle.clone();
            drop_target.connect_drop(move |_, value, _, _| {
                gw_drop.remove_css_class("cc-drop-hover");
                let Ok(raw) = value.get::<String>() else {
                    return false;
                };
                let Some(payload) = DragPayload::decode(&raw) else {
                    return false;
                };
                let target_pos = coords_c.borrow().get(&kind).copied().unwrap_or((0, 0));
                let dropped_kind = payload.kind();
                let mut cfg = ControlCenterConfig::load();
                match payload {
                    DragPayload::FromControlCenter(src_kind) => {
                        if src_kind == kind {
                            return true;
                        }
                        cfg.place_widget_at(src_kind, target_pos.0, target_pos.1);
                    }
                    DragPayload::FromTray(src_kind) => {
                        unpin_widget_from_tray(src_kind);
                        if !cfg.contains(src_kind) {
                            cfg.place_widget_at(src_kind, target_pos.0, target_pos.1);
                        }
                    }
                }
                let _ = cfg.save();
                if let Some(ref f) = *rb_slot.borrow() {
                    f();
                }
                if let Some(ref ed) = *ed_c.borrow() {
                    ed.refresh();
                }
                info!(
                    "🎛 [ControlCenter] Soltado {:?} sobre ({}, {})",
                    dropped_kind, target_pos.0, target_pos.1
                );
                true
            });
            overlay.add_controller(drop_target);
        }

        tile_map.insert(
            kind,
            TileEntry {
                overlay,
                glass_widget,
                remove_btn,
                radius,
            },
        );
    }

    let tile_map = Rc::new(tile_map);

    // DropTarget global sobre todo el Centro de Control (para soltar iconos desde la bandeja en cualquier hueco)
    {
        let root_drop = DropTarget::new(
            glib::Type::STRING,
            gdk::DragAction::COPY | gdk::DragAction::MOVE,
        );
        let rb_slot = rebuild_slot.clone();
        let ed_c = editor_handle.clone();
        root_drop.connect_drop(move |_, value, _, _| {
            let Ok(raw) = value.get::<String>() else {
                return false;
            };
            let Some(payload) = DragPayload::decode(&raw) else {
                return false;
            };
            let mut cfg = ControlCenterConfig::load();
            match payload {
                DragPayload::FromTray(kind) => {
                    unpin_widget_from_tray(kind);
                    cfg.add_widget(kind);
                    let _ = cfg.save();
                }
                DragPayload::FromControlCenter(kind) => {
                    if cfg.add_widget(kind) {
                        let _ = cfg.save();
                    }
                }
            }
            if let Some(ref f) = *rb_slot.borrow() {
                f();
            }
            if let Some(ref ed) = *ed_c.borrow() {
                ed.refresh();
            }
            true
        });
        main_box.add_controller(root_drop);
    }

    // Función de reconstrucción del Grid (normal y modo edición)
    let rebuild_grid: Rc<dyn Fn()> = {
        let grid_c = grid.clone();
        let tiles_c = tile_map.clone();
        let coords_c = cell_coords.clone();
        let targets_c = glass_targets.clone();
        let edit_c = edit_mode.clone();
        let edit_btn_c = edit_controls_btn.clone();
        let br_scale_c = brightness_scale.clone();
        let vol_scale_c = volume_scale.clone();
        let sync_c = sync_glass.clone();
        let rb_self = rebuild_slot.clone();
        let ed_c = editor_handle.clone();

        Rc::new(move || {
            let mb_cfg = malus_de_common::config::MenuBarConfig::load();
            let mut cfg = ControlCenterConfig::load();
            if cfg.sync_with_menubar(&mb_cfg) {
                let _ = cfg.save();
            }
            let editing = edit_c.get();

            while let Some(child) = grid_c.first_child() {
                grid_c.remove(&child);
            }
            coords_c.borrow_mut().clear();
            targets_c.borrow_mut().clear();

            // En modo edición deshabilitamos sliders para facilitar el arrastre
            br_scale_c.set_sensitive(!editing);
            vol_scale_c.set_sensitive(!editing);

            for entry in &cfg.widgets {
                if let Some(tile) = tiles_c.get(&entry.kind) {
                    tile.remove_btn.set_visible(editing);
                    if editing {
                        tile.glass_widget.add_css_class("cc-tile-editing");
                    } else {
                        tile.glass_widget.remove_css_class("cc-tile-editing");
                    }
                    grid_c.attach(
                        &tile.overlay,
                        entry.col as i32,
                        entry.row as i32,
                        entry.col_span as i32,
                        entry.row_span as i32,
                    );
                    coords_c
                        .borrow_mut()
                        .insert(entry.kind, (entry.col, entry.row));
                    // Todos los controles del Centro de Control usan bordes squircle (is_capsule = false)
                    targets_c.borrow_mut().push((
                        tile.glass_widget.clone(),
                        tile.radius,
                        false,
                    ));
                }
            }

            if editing {
                edit_btn_c.set_visible(false);
                // 2 filas × 4 columnas de huecos circulares vacíos (como en captura de Edición)
                let base_row = cfg.max_row_end();
                for r in 0..2 {
                    for c in 0..4 {
                        let target_row = base_row + r;
                        let slot = GtkBox::builder()
                            .css_classes(["cc-edit-empty-slot"])
                            .width_request(70)
                            .height_request(70)
                            .halign(Align::Center)
                            .valign(Align::Center)
                            .build();

                        let slot_drop = DropTarget::new(
                            glib::Type::STRING,
                            gdk::DragAction::COPY | gdk::DragAction::MOVE,
                        );
                        let s_enter = slot.clone();
                        slot_drop.connect_enter(move |_, _, _| {
                            s_enter.add_css_class("cc-edit-empty-slot-hover");
                            gdk::DragAction::MOVE
                        });
                        let s_leave = slot.clone();
                        slot_drop.connect_leave(move |_| {
                            s_leave.remove_css_class("cc-edit-empty-slot-hover");
                        });
                        let s_drop = slot.clone();
                        let rb_inner = rb_self.clone();
                        let ed_inner = ed_c.clone();
                        slot_drop.connect_drop(move |_, value, _, _| {
                            s_drop.remove_css_class("cc-edit-empty-slot-hover");
                            let Ok(raw) = value.get::<String>() else {
                                return false;
                            };
                            let Some(payload) = DragPayload::decode(&raw) else {
                                return false;
                            };
                            let mut cc_cfg = ControlCenterConfig::load();
                            let dropped_kind = payload.kind();
                            if matches!(payload, DragPayload::FromTray(_)) {
                                unpin_widget_from_tray(dropped_kind);
                            }
                            cc_cfg.place_widget_at(dropped_kind, c, target_row);
                            let _ = cc_cfg.save();
                            if let Some(ref f) = *rb_inner.borrow() {
                                f();
                            }
                            if let Some(ref ed) = *ed_inner.borrow() {
                                ed.refresh();
                            }
                            true
                        });
                        slot.add_controller(slot_drop);
                        grid_c.attach(&slot, c as i32, target_row as i32, 1, 1);
                        targets_c.borrow_mut().push((
                            slot.upcast::<gtk4::Widget>(),
                            CC_TILE_SQUIRCLE_RADIUS as f32,
                            false,
                        ));
                    }
                }
            } else {
                edit_btn_c.set_visible(true);
                targets_c.borrow_mut().push((
                    edit_btn_c.clone().upcast::<gtk4::Widget>(),
                    CC_EDIT_BTN_SQUIRCLE_RADIUS as f32,
                    false,
                ));
            }

            sync_c();
        })
    };
    *rebuild_slot.borrow_mut() = Some(rebuild_grid.clone());

    // Activar / desactivar Modo Edición (abre o cierra «Editar controles» y el botón [+] en el tray)
    let set_edit_mode: Rc<dyn Fn(bool)> = {
        let edit_c = edit_mode.clone();
        let rb_c = rebuild_grid.clone();
        let ed_c = editor_handle.clone();
        let stack_c = stack.clone();
        let app_c = app.cloned();
        let edit_mode_self: Rc<RefCell<Option<Rc<dyn Fn(bool)>>>> = Rc::new(RefCell::new(None));
        let self_holder = edit_mode_self.clone();
        let f: Rc<dyn Fn(bool)> = Rc::new(move |active: bool| {
            edit_c.set(active);
            stack_c.set_visible_child_name("main");
            crate::ui::status_tray::set_tray_edit_mode(active);
            rb_c();

            if active {
                let mut ed_opt = ed_c.borrow_mut();
                if ed_opt.is_none() {
                    let rb_for_ed = rb_c.clone();
                    let self_for_done = self_holder.clone();
                    let handle = create_control_center_editor_window(
                        app_c.as_ref(),
                        rb_for_ed,
                        Rc::new(move || {
                            if let Some(ref set_ed) = *self_for_done.borrow() {
                                set_ed(false);
                            }
                        }),
                    );
                    *ed_opt = Some(handle);
                }
                if let Some(ref handle) = *ed_opt {
                    handle.show();
                }
            } else if let Some(ref handle) = *ed_c.borrow() {
                handle.hide();
            }
        });
        *edit_mode_self.borrow_mut() = Some(f.clone());
        f
    };

    // ==========================================
    // Conexiones de interactividad de los 11 controles
    // ==========================================

    // 1. Wi-Fi (clic círculo: encender/apagar; clic texto: subvista)
    {
        let w_btn = wifi_power_btn.clone();
        let w_ico = wifi_icon.clone();
        let w_sub = wifi_sub.clone();
        let em = edit_mode.clone();
        wifi_power_btn.connect_clicked(move |_| {
            if em.get() {
                return;
            }
            let btn = w_btn.clone();
            let ico = w_ico.clone();
            let sub = w_sub.clone();
            glib::MainContext::default().spawn_local(async move {
                let current = NetworkService::is_wifi_enabled().await;
                let _ = NetworkService::set_wifi_enabled(!current).await;
                update_wifi_ui_async(&btn, &ico, &sub).await;
            });
        });

        let st = stack.clone();
        let rf = refresh_wifi.clone();
        let em = edit_mode.clone();
        wifi_nav_btn.connect_clicked(move |_| {
            if em.get() {
                return;
            }
            st.set_visible_child_name("wifi");
            rf();
        });
    }

    // 2. Bluetooth (clic círculo: encender/apagar; clic texto: subvista)
    {
        let b_btn = bt_power_btn.clone();
        let b_ico = bt_icon.clone();
        let b_sub = bt_sub.clone();
        let em = edit_mode.clone();
        bt_power_btn.connect_clicked(move |_| {
            if em.get() {
                return;
            }
            let btn = b_btn.clone();
            let ico = b_ico.clone();
            let sub = b_sub.clone();
            glib::MainContext::default().spawn_local(async move {
                let current = BluetoothService::is_powered().await;
                let _ = BluetoothService::set_powered(!current).await;
                update_bluetooth_ui_async(&btn, &ico, &sub).await;
            });
        });

        let st = stack.clone();
        let rf = refresh_bt.clone();
        let em = edit_mode.clone();
        bt_nav_btn.connect_clicked(move |_| {
            if em.get() {
                return;
            }
            st.set_visible_child_name("bluetooth");
            rf();
        });
    }

    // 3. AirDrop (abre subvista AirDrop)
    {
        let st1 = stack.clone();
        let rf1 = refresh_airdrop.clone();
        let em1 = edit_mode.clone();
        airdrop_power_btn.connect_clicked(move |_| {
            if em1.get() {
                return;
            }
            st1.set_visible_child_name("airdrop");
            rf1();
        });
        let st2 = stack.clone();
        let rf2 = refresh_airdrop.clone();
        let em2 = edit_mode.clone();
        airdrop_nav_btn.connect_clicked(move |_| {
            if em2.get() {
                return;
            }
            st2.set_visible_child_name("airdrop");
            rf2();
        });
    }

    // 5. Organizador visual / Auto-Tiling
    let set_tiling_state: Rc<dyn Fn(bool)> = {
        let btn_c = stage_btn.clone();
        let ico_c = stage_icon.clone();
        let flag_c = stage_active_flag.clone();
        Rc::new(move |enabled: bool| {
            flag_c.set(enabled);
            if enabled {
                ico_c.set_icon_name(Some("view-grid-symbolic"));
                btn_c.add_css_class("cc-glass-circle-active");
            } else {
                ico_c.set_icon_name(Some("view-paged-symbolic"));
                btn_c.remove_css_class("cc-glass-circle-active");
            }
            btn_c.queue_draw();
        })
    };
    {
        let flag_c = stage_active_flag;
        let set_c = set_tiling_state.clone();
        let em = edit_mode.clone();
        stage_btn.connect_clicked(move |_| {
            if em.get() {
                return;
            }
            let next = !flag_c.get();
            set_c(next);
            let _ = IpcClient::send_command(IpcMessage::ToggleAutoTiling);
        });
    }

    // 6. Duplicar pantalla / Disposición de monitores
    {
        let st = stack.clone();
        let rf = refresh_displays_subview.clone();
        let em = edit_mode.clone();
        mirror_btn.connect_clicked(move |_| {
            if em.get() {
                return;
            }
            rf();
            st.set_visible_child_name("displays");
        });
    }

    // 7. Modo oscuro (glifo simbólico del sistema `dark-mode-symbolic`)
    let sync_theme_fn: Rc<dyn Fn(bool)> = {
        let win_c = cc_window.clone();
        let flag_c = dark_state_flag.clone();
        let btn_c = dark_btn.clone();
        Rc::new(move |is_dark: bool| {
            flag_c.set(is_dark);
            if is_dark {
                win_c.remove_css_class("light-mode");
                btn_c.add_css_class("cc-glass-circle-active");
            } else {
                win_c.add_css_class("light-mode");
                btn_c.remove_css_class("cc-glass-circle-active");
            }
            btn_c.queue_draw();
        })
    };
    {
        let sync_t = sync_theme_fn.clone();
        let em = edit_mode.clone();
        dark_btn.connect_clicked(move |_| {
            if em.get() {
                return;
            }
            let target_dark = !malus_de_common::config::is_dark_mode_active();
            apply_system_appearance_mode(target_dark);
            sync_t(target_dark);
            crate::ui::status_tray::refresh_tray_visibility();
        });
    }

    // 8. Escritorio y Finder (clic: Mostrar escritorio; clic derecho: Branches)
    {
        let em = edit_mode.clone();
        let win_c = cc_window.clone();
        let tgl_c = toggle_btn.clone();
        desktop_btn.connect_clicked(move |_| {
            if em.get() {
                return;
            }
            win_c.set_visible(false);
            tgl_c.set_active(false);
            let _ = IpcClient::send_command(IpcMessage::ToggleShowDesktop);
        });

        let rclick = GestureClick::new();
        rclick.set_button(3);
        let em_r = edit_mode.clone();
        let win_r = cc_window.clone();
        let tgl_r = toggle_btn.clone();
        rclick.connect_pressed(move |_, _, _, _| {
            if em_r.get() {
                return;
            }
            win_r.set_visible(false);
            tgl_r.set_active(false);
            let mut cmd = Command::new("malus-de-branches");
            malus_de_common::config::set_cursor_environment(&mut cmd);
            let _ = cmd.spawn();
        });
        desktop_btn.add_controller(rclick);
    }

    // 9. No molestar / Enfoque (clic abre selector de perfiles; clic derecho alterna rápido No molestar)
    {
        let st = stack.clone();
        let rf = refresh_focus_subview.clone();
        let em = edit_mode.clone();
        focus_btn.connect_clicked(move |_| {
            if em.get() {
                return;
            }
            rf();
            st.set_visible_child_name("focus");
        });
    }
    {
        let uf = update_focus_ui.clone();
        let em = edit_mode.clone();
        let rclick = GestureClick::new();
        rclick.set_button(3);
        rclick.connect_released(move |_, _, _, _| {
            if em.get() {
                return;
            }
            set_focus_active(!is_focus_active());
            uf();
            crate::ui::status_tray::refresh_tray_visibility();
        });
        focus_btn.add_controller(rclick);
    }

    // 10. Pantalla (slider de brillo)
    {
        let upd_b = is_updating_brightness.clone();
        let pct_b = disp_pct.clone();
        let ico_b = disp_icon.clone();
        brightness_scale.connect_value_changed(move |scale| {
            if *upd_b.borrow() {
                return;
            }
            let val = scale.value();
            pct_b.set_label(&format!("{}%", val.round() as i32));
            if val < 35.0 {
                ico_b.set_icon_name(Some("display-brightness-low-symbolic"));
            } else if val < 70.0 {
                ico_b.set_icon_name(Some("display-brightness-medium-symbolic"));
            } else {
                ico_b.set_icon_name(Some("display-brightness-symbolic"));
            }
            BrightnessService::set_brightness(val);
        });
    }

    // 11. Sonido (slider de volumen + silenciar + botón de salida)
    {
        let st = stack.clone();
        let rf = refresh_sound_subview.clone();
        let em = edit_mode.clone();
        sound_output_btn.connect_clicked(move |_| {
            if em.get() {
                return;
            }
            rf();
            st.set_visible_child_name("sound");
        });

        let v_sc = volume_scale.clone();
        let v_pc = sound_pct.clone();
        let v_ic = vol_icon.clone();
        let v_li = vol_icon_low.clone();
        let v_lk = is_updating_volume.clone();
        let em_m = edit_mode.clone();
        vol_mute_btn.connect_clicked(move |_| {
            if em_m.get() {
                return;
            }
            AudioService::toggle_mute();
            let sc = v_sc.clone();
            let pc = v_pc.clone();
            let ic = v_ic.clone();
            let li = v_li.clone();
            let lk = v_lk.clone();
            glib::MainContext::default().spawn_local(async move {
                let (vol, muted) = AudioService::get_volume().await;
                *lk.borrow_mut() = true;
                sc.set_value(if muted { 0.0 } else { vol });
                pc.set_label(&format!("{}%", if muted { 0 } else { vol.round() as i32 }));
                let icon_name = if muted || vol < 1.0 {
                    "audio-volume-muted-symbolic"
                } else if vol < 40.0 {
                    "audio-volume-low-symbolic"
                } else if vol < 75.0 {
                    "audio-volume-medium-symbolic"
                } else {
                    "audio-volume-high-symbolic"
                };
                ic.set_icon_name(Some(icon_name));
                li.set_icon_name(Some(if muted || vol < 1.0 {
                    "audio-volume-muted-symbolic"
                } else {
                    "audio-volume-low-symbolic"
                }));
                *lk.borrow_mut() = false;
                crate::ui::status_tray::refresh_volume_tray();
            });
        });

        let upd_v = is_updating_volume.clone();
        let pct_v = sound_pct.clone();
        let ico_v = vol_icon.clone();
        let low_v = vol_icon_low.clone();
        let last_fb = Rc::new(RefCell::new(
            std::time::Instant::now() - std::time::Duration::from_secs(1),
        ));
        volume_scale.connect_value_changed(move |scale| {
            if *upd_v.borrow() {
                return;
            }
            let val = scale.value();
            pct_v.set_label(&format!("{}%", val.round() as i32));
            if val < 1.0 {
                ico_v.set_icon_name(Some("audio-volume-muted-symbolic"));
                low_v.set_icon_name(Some("audio-volume-muted-symbolic"));
            } else if val < 40.0 {
                ico_v.set_icon_name(Some("audio-volume-low-symbolic"));
                low_v.set_icon_name(Some("audio-volume-low-symbolic"));
            } else if val < 75.0 {
                ico_v.set_icon_name(Some("audio-volume-medium-symbolic"));
                low_v.set_icon_name(Some("audio-volume-low-symbolic"));
            } else {
                ico_v.set_icon_name(Some("audio-volume-high-symbolic"));
                low_v.set_icon_name(Some("audio-volume-low-symbolic"));
            }
            AudioService::set_volume(val);
            let now = std::time::Instant::now();
            if now.duration_since(*last_fb.borrow()) >= std::time::Duration::from_millis(180) {
                *last_fb.borrow_mut() = now;
                AudioService::play_volume_feedback_if_enabled();
            }
        });
    }

    // Botón "Editar controles"
    {
        let set_ed = set_edit_mode.clone();
        edit_controls_btn.connect_clicked(move |_| {
            set_ed(true);
        });
    }

    // Tecla Escape: salir de subvista -> salir de modo edición -> cerrar Centro de Control
    {
        let key_ctrl = EventControllerKey::new();
        let st_k = stack.clone();
        let em_k = edit_mode.clone();
        let set_ed_k = set_edit_mode.clone();
        let tgl_k = toggle_btn.clone();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if st_k.visible_child_name().as_deref() != Some("main") {
                    st_k.set_visible_child_name("main");
                } else if em_k.get() {
                    set_ed_k(false);
                } else {
                    tgl_k.set_active(false);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        cc_window.add_controller(key_ctrl);
    }

    // Actualizar regiones GPU al cambiar de página, durante/al finalizar la transición del Stack o al ocultarse
    {
        let sg = sync_glass.clone();
        stack.connect_visible_child_name_notify(move |_| {
            sg();
        });
    }
    {
        let sg = sync_glass.clone();
        stack.connect_transition_running_notify(move |_| {
            sg();
        });
    }
    {
        let tgl_h = toggle_btn.clone();
        let last_h = last_sent_glass;
        cc_window.connect_hide(move |_| {
            if tgl_h.is_active() {
                tgl_h.set_active(false);
            }
            *last_h.borrow_mut() = Some(Vec::new());
            let _ = IpcClient::send_command(IpcMessage::SetControlCenterGlassRegions {
                regions: Vec::new(),
            });
        });
    }

    // Refresco completo al abrir el Centro de Control
    let refresh_all: Rc<dyn Fn()> = {
        let st_r = stack.clone();
        let rb_r = rebuild_grid.clone();
        let sync_t = sync_theme_fn.clone();
        let uf_r = update_focus_ui.clone();
        let rm_r = refresh_media.clone();
        let rd_r = refresh_displays_subview.clone();
        let wb = wifi_power_btn.clone();
        let wi = wifi_icon.clone();
        let ws = wifi_sub.clone();
        let bb = bt_power_btn.clone();
        let bi = bt_icon.clone();
        let bs = bt_sub.clone();
        let ab = airdrop_power_btn.clone();
        let ai = airdrop_icon.clone();
        let asub = airdrop_sub.clone();
        let vs = volume_scale.clone();
        let vp = sound_pct.clone();
        let vi = vol_icon.clone();
        let vl = is_updating_volume.clone();
        let brs = brightness_scale.clone();
        let brp = disp_pct.clone();
        let bri = disp_icon.clone();
        let brl = is_updating_brightness.clone();

        Rc::new(move || {
            st_r.set_visible_child_name("main");
            rb_r();
            sync_t(malus_de_common::config::is_dark_mode_active());
            uf_r();
            rm_r();
            rd_r();

            let wb_i = wb.clone();
            let wi_i = wi.clone();
            let ws_i = ws.clone();
            let bb_i = bb.clone();
            let bi_i = bi.clone();
            let bs_i = bs.clone();
            let ab_i = ab.clone();
            let ai_i = ai.clone();
            let asub_i = asub.clone();
            let vs_i = vs.clone();
            let vp_i = vp.clone();
            let vi_i = vi.clone();
            let vl_i = vl.clone();
            let brs_i = brs.clone();
            let brp_i = brp.clone();
            let bri_i = bri.clone();
            let brl_i = brl.clone();

            glib::MainContext::default().spawn_local(async move {
                update_wifi_ui_async(&wb_i, &wi_i, &ws_i).await;
                update_bluetooth_ui_async(&bb_i, &bi_i, &bs_i).await;
                let mode = get_airdrop_mode_dbus().await;
                update_airdrop_ui_mode(&ab_i, &ai_i, &asub_i, &mode);
                refresh_volume_ui_async(&vs_i, &vp_i, &vi_i, &vl_i).await;
                refresh_brightness_ui_async(&brs_i, &brp_i, &bri_i, &brl_i).await;
            });
        })
    };

    // Construcción inicial del Grid y estado
    rebuild_grid();
    (sync_theme_fn)(malus_de_common::config::is_dark_mode_active());
    update_focus_ui();

    CC_RUNTIME.with(|cell| {
        *cell.borrow_mut() = Some(ControlCenterRuntime {
            window: cc_window,
            toggle_btn: toggle_btn.clone(),
            edit_mode,
            is_dragging,
            editor: editor_handle,
            sync_glass,
            rebuild_grid,
            refresh_all,
            set_tiling_state,
            sync_theme_fn,
            set_edit_mode_fn: set_edit_mode,
        });
    });
}

// ==========================================
// Constructores de Controles Tahoe Liquid Glass
// ==========================================

fn is_focus_active() -> bool {
    let focus_cfg = malus_de_common::config::FocusConfig::load();
    if focus_cfg.is_active_now() {
        return true;
    }
    let output = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.notifications", "show-banners"])
        .output();
    if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
        text == "false"
    } else {
        false
    }
}

fn set_focus_mode_active(mode_id: Option<&str>) {
    let mut focus_cfg = malus_de_common::config::FocusConfig::load();
    focus_cfg.active_focus_id = mode_id.map(str::to_string);
    let _ = focus_cfg.save();

    let active = focus_cfg.is_active_now();
    let val = if active { "false" } else { "true" };
    let _ = Command::new("gsettings")
        .args([
            "set",
            "org.gnome.desktop.notifications",
            "show-banners",
            val,
        ])
        .spawn();
}

fn set_focus_active(active: bool) {
    set_focus_mode_active(if active { Some("do-not-disturb") } else { None });
}

/// Píldora 2×1 de conectividad (Wi-Fi, Bluetooth, AirDrop) con círculo blanco de icono + título + subtítulo.
fn create_tahoe_connectivity_pill(
    icon_name: &str,
    title: &str,
    subtitle: &str,
    wrap_subtitle: bool,
) -> (GtkBox, Button, Button, Image, Label) {
    let pill = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-glass-pill"])
        .spacing(10)
        .width_request(150)
        .height_request(70)
        .valign(Align::Fill)
        .halign(Align::Fill)
        .build();

    let power_btn = Button::builder()
        .css_classes(["cc-tahoe-icon-circle"])
        .has_frame(false)
        .width_request(44)
        .height_request(44)
        .valign(Align::Center)
        .halign(Align::Start)
        .tooltip_text(title)
        .build();

    let icon = Image::builder()
        .icon_name(icon_name)
        .pixel_size(18)
        .halign(Align::Center)
        .valign(Align::Center)
        .build();
    power_btn.set_child(Some(&icon));

    let nav_btn = Button::builder()
        .css_classes(["cc-tile-nav-btn"])
        .has_frame(false)
        .hexpand(true)
        .valign(Align::Fill)
        .build();

    let text_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .valign(Align::Center)
        .hexpand(true)
        .spacing(1)
        .build();

    let title_lbl = Label::builder()
        .label(title)
        .css_classes(["cc-glass-title"])
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();

    let sub_lbl = Label::builder()
        .label(subtitle)
        .css_classes(["cc-glass-subtitle"])
        .xalign(0.0)
        .wrap(wrap_subtitle)
        .lines(2)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(12)
        .build();

    text_box.append(&title_lbl);
    text_box.append(&sub_lbl);
    nav_btn.set_child(Some(&text_box));

    pill.append(&power_btn);
    pill.append(&nav_btn);

    (pill, power_btn, nav_btn, icon, sub_lbl)
}

/// Control cuadrado squircle 1×1 con glifo simbólico uniforme del sistema (`18px` de `*/symbolic/`).
fn create_tahoe_circle_control(icon_name: &str, tooltip: &str) -> (Button, Image, Rc<Cell<bool>>) {
    let btn = Button::builder()
        .css_classes(["cc-glass-circle"])
        .has_frame(false)
        .width_request(70)
        .height_request(70)
        .halign(Align::Fill)
        .valign(Align::Fill)
        .tooltip_text(tooltip)
        .build();

    let icon = Image::builder()
        .icon_name(icon_name)
        .pixel_size(18)
        .halign(Align::Center)
        .valign(Align::Center)
        .build();
    btn.set_child(Some(&icon));
    apply_squircle_glass_fill_button(&btn, CC_TILE_SQUIRCLE_RADIUS, 0, 0);

    (btn, icon, Rc::new(Cell::new(false)))
}

/// Tarjeta squircle 2×1 de No molestar / Enfoque con glifo simbólico uniforme (`18px`).
fn create_tahoe_focus_pill() -> (Button, GtkBox, Image, Label, Label) {
    let btn = Button::builder()
        .css_classes(["cc-glass-pill", "cc-glass-focus-pill"])
        .has_frame(false)
        .width_request(150)
        .height_request(70)
        .valign(Align::Fill)
        .halign(Align::Fill)
        .tooltip_text("Enfoque (Clic: elegir perfil · Clic derecho: alternar No molestar)")
        .build();

    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .halign(Align::Fill)
        .valign(Align::Center)
        .hexpand(true)
        .build();

    let icon_circle = GtkBox::builder()
        .css_classes(["cc-tahoe-icon-circle", "cc-focus-circle"])
        .width_request(44)
        .height_request(44)
        .halign(Align::Center)
        .valign(Align::Center)
        .build();

    let icon = Image::builder()
        .icon_name("weather-clear-night-symbolic")
        .pixel_size(18)
        .halign(Align::Center)
        .valign(Align::Center)
        .hexpand(true)
        .vexpand(true)
        .build();
    icon_circle.append(&icon);

    let text_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .valign(Align::Center)
        .hexpand(true)
        .spacing(1)
        .build();

    let title_lbl = Label::builder()
        .label("Enfoque")
        .css_classes(["cc-glass-title"])
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();

    let sub_lbl = Label::builder()
        .label("Desactivado")
        .css_classes(["cc-glass-subtitle"])
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();

    text_box.append(&title_lbl);
    text_box.append(&sub_lbl);

    let chevron = Image::builder()
        .icon_name("go-next-symbolic")
        .pixel_size(14)
        .opacity(0.65)
        .valign(Align::Center)
        .build();

    row.append(&icon_circle);
    row.append(&text_box);
    row.append(&chevron);
    btn.set_child(Some(&row));
    apply_squircle_glass_fill_button(&btn, CC_TILE_SQUIRCLE_RADIUS, 10, 12);

    (btn, icon_circle, icon, title_lbl, sub_lbl)
}

fn create_focus_subview(
    stack: &Stack,
    popover: &Window,
    update_focus_ui: &Rc<dyn Fn()>,
) -> (GtkBox, Rc<dyn Fn()>) {
    let view_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-subview"])
        .spacing(8)
        .build();

    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-subview-header"])
        .spacing(6)
        .build();

    let back_btn = Button::builder()
        .css_classes(["cc-back-btn"])
        .has_frame(false)
        .tooltip_text("Volver")
        .build();
    back_btn.set_child(Some(&Image::from_icon_name("go-previous-symbolic")));
    {
        let stack_back = stack.clone();
        back_btn.connect_clicked(move |_| {
            stack_back.set_visible_child_name("main");
        });
    }

    let title_lbl = Label::builder()
        .label("Enfoque")
        .css_classes(["cc-subview-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    header.append(&back_btn);
    header.append(&title_lbl);
    view_box.append(&header);

    let modes_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .build();
    view_box.append(&modes_box);

    view_box.append(&Separator::new(Orientation::Horizontal));

    let settings_btn = Button::builder()
        .label("Configuración de enfoque...")
        .css_classes(["cc-footer-btn"])
        .has_frame(false)
        .halign(Align::Fill)
        .build();
    {
        let pop_c = popover.clone();
        settings_btn.connect_clicked(move |_| {
            pop_c.set_visible(false);
            let mut cmd = Command::new("malus-de-settings");
            cmd.args(["--panel", "focus"]);
            malus_de_common::config::set_cursor_environment(&mut cmd);
            let _ = cmd.spawn();
        });
    }
    view_box.append(&settings_btn);

    let refresh_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh: Rc<dyn Fn()> = {
        let mb = modes_box;
        let uf = update_focus_ui.clone();
        let slot_c = refresh_slot.clone();
        Rc::new(move || {
            while let Some(child) = mb.first_child() {
                mb.remove(&child);
            }
            let focus_cfg = malus_de_common::config::FocusConfig::load();
            let active_id = focus_cfg.active_mode_now().map(|m| m.id.clone());

            for mode in focus_cfg.modes {
                let is_active = active_id.as_deref() == Some(mode.id.as_str());
                let subtitle = if is_active {
                    "Activado · Clic para desactivar".to_string()
                } else if mode.schedule.enabled {
                    format!("Horario {}", mode.schedule.summary_label())
                } else if mode.allow_urgent {
                    "Permite notificaciones urgentes".to_string()
                } else {
                    "Silencia todas las alertas".to_string()
                };

                let row_btn = Button::builder()
                    .css_classes(if is_active {
                        vec!["cc-list-row", "cc-list-row-active"]
                    } else {
                        vec!["cc-list-row"]
                    })
                    .has_frame(false)
                    .halign(Align::Fill)
                    .build();

                let hbox = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(10)
                    .build();
                let ico = Image::builder()
                    .icon_name(&mode.icon)
                    .pixel_size(18)
                    .valign(Align::Center)
                    .build();
                let tcol = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(1)
                    .hexpand(true)
                    .build();
                tcol.append(
                    &Label::builder()
                        .label(&mode.name)
                        .css_classes(["cc-item-title"])
                        .xalign(0.0)
                        .build(),
                );
                tcol.append(
                    &Label::builder()
                        .label(&subtitle)
                        .css_classes(["cc-item-subtitle"])
                        .xalign(0.0)
                        .build(),
                );
                hbox.append(&ico);
                hbox.append(&tcol);

                if is_active {
                    hbox.append(
                        &Image::builder()
                            .icon_name("object-select-symbolic")
                            .pixel_size(15)
                            .valign(Align::Center)
                            .build(),
                    );
                }

                row_btn.set_child(Some(&hbox));
                let mode_id = mode.id.clone();
                let uf_c = uf.clone();
                let slot_click = slot_c.clone();
                row_btn.connect_clicked(move |_| {
                    if is_active {
                        set_focus_mode_active(None);
                    } else {
                        set_focus_mode_active(Some(&mode_id));
                    }
                    uf_c();
                    crate::ui::status_tray::refresh_tray_visibility();
                    if let Some(ref rf) = *slot_click.borrow() {
                        rf();
                    }
                });
                mb.append(&row_btn);
            }
        })
    };
    *refresh_slot.borrow_mut() = Some(refresh.clone());

    (view_box, refresh)
}

/// Tarjeta 2×2 de Ahora suena (MPRIS D-Bus) estilo macOS Tahoe.
fn create_tahoe_now_playing_card() -> (GtkBox, Rc<dyn Fn()>) {
    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-glass-media-card"])
        .width_request(150)
        .height_request(150)
        .spacing(8)
        .build();

    let art_box = GtkBox::builder()
        .css_classes(["cc-glass-media-art"])
        .width_request(48)
        .height_request(48)
        .halign(Align::Start)
        .valign(Align::Start)
        .build();
    let art_icon = Image::builder()
        .icon_name("audio-x-generic-symbolic")
        .pixel_size(22)
        .halign(Align::Center)
        .valign(Align::Center)
        .hexpand(true)
        .vexpand(true)
        .build();
    art_box.append(&art_icon);
    card.append(&art_box);

    let info_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(1)
        .valign(Align::Center)
        .vexpand(true)
        .build();

    let title_lbl = Label::builder()
        .label("Sin reproducción")
        .css_classes(["cc-glass-media-title"])
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(16)
        .build();

    let artist_lbl = Label::builder()
        .label("")
        .css_classes(["cc-glass-subtitle"])
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(16)
        .visible(false)
        .build();

    info_box.append(&title_lbl);
    info_box.append(&artist_lbl);
    card.append(&info_box);

    let controls_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::Fill)
        .valign(Align::End)
        .homogeneous(true)
        .spacing(4)
        .build();

    let prev_btn = Button::builder()
        .css_classes(["cc-glass-media-btn"])
        .has_frame(false)
        .tooltip_text("Pista anterior")
        .build();
    let prev_ico = Image::from_icon_name("media-skip-backward-symbolic");
    prev_ico.set_pixel_size(18);
    prev_btn.set_child(Some(&prev_ico));

    let play_btn = Button::builder()
        .css_classes(["cc-glass-media-btn", "cc-glass-media-play"])
        .has_frame(false)
        .tooltip_text("Reproducir / Pausar")
        .build();
    let play_icon = Image::from_icon_name("media-playback-start-symbolic");
    play_icon.set_pixel_size(22);
    play_btn.set_child(Some(&play_icon));

    let next_btn = Button::builder()
        .css_classes(["cc-glass-media-btn"])
        .has_frame(false)
        .tooltip_text("Siguiente pista")
        .build();
    let next_ico = Image::from_icon_name("media-skip-forward-symbolic");
    next_ico.set_pixel_size(18);
    next_btn.set_child(Some(&next_ico));

    controls_box.append(&prev_btn);
    controls_box.append(&play_btn);
    controls_box.append(&next_btn);
    card.append(&controls_box);

    let current_bus = Rc::new(RefCell::new(String::new()));
    let t_c = title_lbl.clone();
    let a_c = artist_lbl.clone();
    let pi_c = play_icon.clone();
    let bus_c = current_bus.clone();

    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        let t = t_c.clone();
        let a = a_c.clone();
        let pi = pi_c.clone();
        let bus = bus_c.clone();

        glib::MainContext::default().spawn_local(async move {
            if let Some(media) = MprisService::get_current_media().await {
                *bus.borrow_mut() = media.player_bus.clone();
                t.set_label(&media.title);
                if media.artist.trim().is_empty() {
                    a.set_visible(false);
                } else {
                    a.set_label(&media.artist);
                    a.set_visible(true);
                }
                if media.is_playing {
                    pi.set_icon_name(Some("media-playback-pause-symbolic"));
                } else {
                    pi.set_icon_name(Some("media-playback-start-symbolic"));
                }
            } else {
                *bus.borrow_mut() = String::new();
                t.set_label("Sin reproducción");
                a.set_label("");
                a.set_visible(false);
                pi.set_icon_name(Some("media-playback-start-symbolic"));
            }
        });
    });

    for (btn, action) in [(&prev_btn, 0u8), (&play_btn, 1u8), (&next_btn, 2u8)] {
        let bus_h = current_bus.clone();
        let rf_h = refresh.clone();
        btn.connect_clicked(move |_| {
            let bus = bus_h.borrow().clone();
            if bus.is_empty() {
                return;
            }
            let r = rf_h.clone();
            glib::MainContext::default().spawn_local(async move {
                let _ = match action {
                    0 => MprisService::previous(&bus).await,
                    1 => MprisService::play_pause(&bus).await,
                    _ => MprisService::next(&bus).await,
                };
                glib::timeout_future(std::time::Duration::from_millis(150)).await;
                r();
            });
        });
    }

    refresh();
    (card, refresh)
}

/// Píldora 4×1 de Pantalla (brillo) estilo macOS Tahoe.
fn create_tahoe_display_slider_pill(
    cfg: &malus_de_common::config::DisplaysConfig,
) -> (GtkBox, GtkBox, Scale, Label, Image) {
    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-glass-slider-pill"])
        .width_request(310)
        .height_request(62)
        .spacing(3)
        .valign(Align::Center)
        .build();

    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();

    let title = Label::builder()
        .label("Pantalla")
        .css_classes(["cc-glass-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let pct = Label::builder()
        .label(&format!("{}%", cfg.brightness.round() as i32))
        .css_classes(["cc-glass-subtitle"])
        .visible(false)
        .build();

    header.append(&title);
    header.append(&pct);

    let slider_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .valign(Align::Center)
        .build();

    let low_ico = Image::from_icon_name("display-brightness-low-symbolic");
    low_ico.set_pixel_size(15);
    low_ico.add_css_class("cc-glass-slider-icon");

    let scale = Scale::with_range(Orientation::Horizontal, 5.0, 100.0, 1.0);
    disable_scale_scroll(&scale);
    scale.set_value(cfg.brightness.clamp(5.0, 100.0));
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.add_css_class("cc-tahoe-slider");

    let high_ico = Image::from_icon_name("display-brightness-symbolic");
    high_ico.set_pixel_size(18);
    high_ico.add_css_class("cc-glass-slider-icon");

    slider_row.append(&low_ico);
    slider_row.append(&scale);
    slider_row.append(&high_ico);

    card.append(&header);
    card.append(&slider_row);

    (card, header, scale, pct, high_ico)
}

/// Píldora 4×1 de Sonido (volumen + salida de audio) estilo macOS Tahoe.
#[allow(clippy::type_complexity)]
fn create_tahoe_sound_slider_pill() -> (
    GtkBox,
    GtkBox,
    Scale,
    Label,
    Image,
    Image,
    Button,
    Button,
) {
    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-glass-slider-pill"])
        .width_request(310)
        .height_request(62)
        .spacing(3)
        .valign(Align::Center)
        .build();

    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();

    let title = Label::builder()
        .label("Sonido")
        .css_classes(["cc-glass-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let pct = Label::builder()
        .label("75%")
        .css_classes(["cc-glass-subtitle"])
        .visible(false)
        .build();

    header.append(&title);
    header.append(&pct);

    let slider_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .valign(Align::Center)
        .build();

    let mute_btn = Button::builder()
        .css_classes(["cc-glass-inline-icon-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .tooltip_text("Silenciar / Reactivar audio")
        .build();
    let low_ico = Image::from_icon_name("audio-volume-low-symbolic");
    low_ico.set_pixel_size(16);
    low_ico.add_css_class("cc-glass-slider-icon");
    mute_btn.set_child(Some(&low_ico));

    let scale = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    disable_scale_scroll(&scale);
    scale.set_value(75.0);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.add_css_class("cc-tahoe-slider");

    let high_ico = Image::from_icon_name("audio-volume-high-symbolic");
    high_ico.set_pixel_size(18);
    high_ico.add_css_class("cc-glass-slider-icon");

    let output_btn = Button::builder()
        .css_classes(["cc-glass-sound-output-btn"])
        .has_frame(false)
        .width_request(30)
        .height_request(30)
        .valign(Align::Center)
        .tooltip_text("Salidas de audio y mezclador por aplicación")
        .build();
    let out_ico = Image::from_icon_name("audio-card-symbolic");
    out_ico.set_pixel_size(15);
    output_btn.set_child(Some(&out_ico));

    slider_row.append(&mute_btn);
    slider_row.append(&scale);
    slider_row.append(&high_ico);
    slider_row.append(&output_btn);

    card.append(&header);
    card.append(&slider_row);

    (
        card, header, scale, pct, high_ico, low_ico, mute_btn, output_btn,
    )
}

// ==========================================
// Subvista AirDrop (Link D-Bus)
// ==========================================

fn create_airdrop_subview(
    stack: &Stack,
    popover: &Window,
    main_ad_btn: &Button,
    main_ad_icon: &Image,
    main_ad_sub: &Label,
) -> (GtkBox, Rc<dyn Fn()>) {
    let view_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-subview"])
        .spacing(8)
        .build();

    // Header: Back btn + Title
    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-subview-header"])
        .spacing(6)
        .build();

    let back_btn = Button::builder()
        .css_classes(["cc-back-btn"])
        .has_frame(false)
        .tooltip_text("Volver")
        .build();
    let back_icon = Image::from_icon_name("go-previous-symbolic");
    back_btn.set_child(Some(&back_icon));

    let stack_back = stack.clone();
    back_btn.connect_clicked(move |_| {
        stack_back.set_visible_child_name("main");
    });

    let title_lbl = Label::builder()
        .label("AirDrop")
        .css_classes(["cc-subview-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    header.append(&back_btn);
    header.append(&title_lbl);
    view_box.append(&header);

    let desc_lbl = Label::builder()
        .label(
            "Comparte archivos vía AirDrop nativo (AWDL/owly) con MacBook, iPhone o iPad cercanos.",
        )
        .css_classes(["cc-airdrop-desc"])
        .wrap(true)
        .xalign(0.0)
        .build();
    view_box.append(&desc_lbl);

    let status_lbl = Label::builder()
        .label("Consultando estado del motor AirDrop...")
        .css_classes(["cc-item-subtitle"])
        .wrap(true)
        .xalign(0.0)
        .build();
    view_box.append(&status_lbl);

    // Opciones de visibilidad
    let (row_off, check_off) =
        create_airdrop_option_row("No recibir", "Desactiva AWDL y mantiene tu Wi-Fi normal");
    let (row_contacts, check_contacts) =
        create_airdrop_option_row("Solo contactos", "Requiere Apple ID (inactivo en Linux)");
    let (row_everyone, check_everyone) =
        create_airdrop_option_row("Todos durante 10 min", "Activa awdl0 y recibe en Descargas");

    view_box.append(&row_off);
    view_box.append(&row_contacts);
    view_box.append(&row_everyone);

    let peers_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .visible(false)
        .build();
    view_box.append(&peers_box);

    // Acciones reales: buscar y enviar
    let actions_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .homogeneous(true)
        .build();

    let scan_btn = Button::builder()
        .css_classes(["cc-airdrop-sim-btn"])
        .label("🔍 Buscar")
        .has_frame(false)
        .tooltip_text("Buscar dispositivos Apple cercanos en awdl0")
        .build();

    let send_btn = Button::builder()
        .css_classes(["cc-airdrop-sim-btn"])
        .label("📤 Enviar archivo")
        .has_frame(false)
        .tooltip_text("Seleccionar un archivo y enviarlo por AirDrop")
        .build();

    actions_row.append(&scan_btn);
    actions_row.append(&send_btn);
    view_box.append(&actions_row);

    // Footer: Separador y acceso a Descargas
    let footer_sep = Separator::new(Orientation::Horizontal);
    let downloads_btn = Button::builder()
        .css_classes(["cc-footer-btn"])
        .label("Abrir carpeta Descargas...")
        .has_frame(false)
        .build();

    let pop_c = popover.clone();
    downloads_btn.connect_clicked(move |_| {
        pop_c.set_visible(false);
        let home = glib::home_dir();
        let dl_dir = if home.join("Descargas").exists() {
            home.join("Descargas")
        } else {
            home.join("Downloads")
        };
        let mut command = Command::new("malus-de-branches");
        malus_de_common::config::set_cursor_environment(&mut command);
        if command.arg(&dl_dir).spawn().is_err() {
            let _ = Command::new("xdg-open").arg(&dl_dir).spawn();
        }
    });

    view_box.append(&footer_sep);
    view_box.append(&downloads_btn);

    // Manejador de cambio de opción
    let ro_c = row_off.clone();
    let rc_c = row_contacts.clone();
    let re_c = row_everyone.clone();
    let cho_c = check_off.clone();
    let chc_c = check_contacts.clone();
    let che_c = check_everyone.clone();
    let mb_c = main_ad_btn.clone();
    let mi_c = main_ad_icon.clone();
    let ms_c = main_ad_sub.clone();

    let update_selection = Rc::new(move |mode: &str| {
        ro_c.remove_css_class("cc-list-row-active");
        rc_c.remove_css_class("cc-list-row-active");
        re_c.remove_css_class("cc-list-row-active");

        cho_c.set_visible(false);
        chc_c.set_visible(false);
        che_c.set_visible(false);

        match mode {
            "contacts" => {
                rc_c.add_css_class("cc-list-row-active");
                chc_c.set_visible(true);
            }
            "everyone" => {
                re_c.add_css_class("cc-list-row-active");
                che_c.set_visible(true);
            }
            _ => {
                ro_c.add_css_class("cc-list-row-active");
                cho_c.set_visible(true);
            }
        }
        update_airdrop_ui_mode(&mb_c, &mi_c, &ms_c, mode);
    });

    let pop_send = popover.clone();
    send_btn.connect_clicked(move |_| {
        pop_send.set_visible(false);
        pick_and_send_airdrop_file_cc("auto".to_string());
    });

    let st_scan = status_lbl.clone();
    let pb_scan = peers_box.clone();
    let pop_scan = popover.clone();
    scan_btn.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        btn.set_label("⏳ Buscando...");
        st_scan.set_label("Buscando MacBook / iPhone cercanos vía AWDL...");
        let btn_c = btn.clone();
        let st_c = st_scan.clone();
        let pb_c = pb_scan.clone();
        let pop_peer = pop_scan.clone();
        glib::MainContext::default().spawn_local(async move {
            let peers = scan_airdrop_peers_cc_dbus(6).await;
            btn_c.set_sensitive(true);
            btn_c.set_label("🔍 Buscar");
            populate_airdrop_peers_cc(&pb_c, &peers, &pop_peer);
            if peers.is_empty() {
                st_c.set_label("Sin dispositivos encontrados. Asegúrate de tener 'Todos durante 10 min' activo aquí y en tu Mac.");
            } else {
                st_c.set_label(&format!("{} dispositivo(s) encontrado(s). Haz clic para enviar.", peers.len()));
            }
        });
    });

    // Callback de refresco al navegar a la subvista
    let sel_refresh = update_selection.clone();
    let st_refresh = status_lbl.clone();
    let pb_refresh = peers_box.clone();
    let pop_refresh = popover.clone();
    let refresh_closure: Rc<dyn Fn()> = Rc::new(move || {
        let sel = sel_refresh.clone();
        let st = st_refresh.clone();
        let pb = pb_refresh.clone();
        let pop_inner = pop_refresh.clone();
        glib::MainContext::default().spawn_local(async move {
            if let Some(v) = get_airdrop_status_json_cc_dbus().await {
                let mode = v["mode"].as_str().unwrap_or("off");
                sel(mode);
                st.set_label(&format_airdrop_status_cc(&v));
                let peers: Vec<(String, String, String)> = v["peers"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .map(|item| {
                                (
                                    item["id"].as_str().unwrap_or("").to_string(),
                                    item["name"].as_str().unwrap_or("Apple").to_string(),
                                    item["address"].as_str().unwrap_or("").to_string(),
                                )
                            })
                            .filter(|(id, _, _)| !id.is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
                populate_airdrop_peers_cc(&pb, &peers, &pop_inner);
            } else {
                let mode = get_airdrop_mode_dbus().await;
                sel(&mode);
                st.set_label("malus-de-link no disponible en D-Bus.");
            }
        });
    });

    // Conectar clics en filas
    let sel_off = update_selection.clone();
    let ref_off = refresh_closure.clone();
    row_off.connect_clicked(move |_| {
        sel_off("off");
        let r = ref_off.clone();
        glib::MainContext::default().spawn_local(async move {
            set_airdrop_mode_dbus("off").await;
            r();
        });
    });

    let sel_cont = update_selection.clone();
    let ref_cont = refresh_closure.clone();
    row_contacts.connect_clicked(move |_| {
        sel_cont("contacts");
        let r = ref_cont.clone();
        glib::MainContext::default().spawn_local(async move {
            set_airdrop_mode_dbus("contacts").await;
            r();
        });
    });

    let sel_every = update_selection.clone();
    let ref_every = refresh_closure.clone();
    let st_every = status_lbl.clone();
    row_everyone.connect_clicked(move |_| {
        sel_every("everyone");
        st_every.set_label(
            "Levantando radio AWDL (awdl0)... En Intel AX210 el Wi-Fi se pausa temporalmente.",
        );
        let r = ref_every.clone();
        glib::MainContext::default().spawn_local(async move {
            set_airdrop_mode_dbus("everyone").await;
            r();
            let r_later = r.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
                r_later();
            });
        });
    });

    (view_box, refresh_closure)
}

fn format_airdrop_status_cc(v: &serde_json::Value) -> String {
    let radio_state = v["radio_state"].as_str().unwrap_or("off");
    let rem = v["remaining_seconds"].as_u64().unwrap_or(0);
    let station = v["station"].as_str().unwrap_or("Wi-Fi");
    let rmode = v["radio_mode"].as_str().unwrap_or("");
    let hw_ready = v["hardware_ready"].as_bool().unwrap_or(false);
    if let Some(err) = v["last_error"].as_str() {
        if !err.trim().is_empty() {
            return format!("⚠️ {}", err);
        }
    }
    match radio_state {
        "starting" => format!("⏳ Iniciando awdl0 en {}...", station),
        "active" => {
            let name = v["device_name"].as_str().unwrap_or("Malus DE");
            let time_part = if rem > 0 {
                format!(" · {}:{:02}", rem / 60, rem % 60)
            } else {
                String::new()
            };
            format!("🟢 Activo como '{}' en awdl0{}", name, time_part)
        }
        "error" => "❌ Error activando radio AWDL.".to_string(),
        _ => {
            if hw_ready {
                if rmode == "exclusive" {
                    format!(
                        "Listo ({} · modo exclusivo: pausa Wi-Fi al activar)",
                        station
                    )
                } else {
                    format!("Listo ({} · modo compartido)", station)
                }
            } else {
                "⚠️ Motor owly/hardware AWDL no listo.".to_string()
            }
        }
    }
}

fn populate_airdrop_peers_cc(
    peers_box: &GtkBox,
    peers: &[(String, String, String)],
    popover: &Window,
) {
    while let Some(child) = peers_box.first_child() {
        peers_box.remove(&child);
    }
    if peers.is_empty() {
        peers_box.set_visible(false);
        return;
    }
    peers_box.set_visible(true);
    let hdr = Label::builder()
        .label("Dispositivos cercanos (clic para enviar):")
        .css_classes(["cc-item-subtitle"])
        .xalign(0.0)
        .build();
    peers_box.append(&hdr);

    for (id, name, addr) in peers {
        let subtitle = if addr.is_empty() {
            "Enviar archivo vía AirDrop".to_string()
        } else {
            format!("AWDL · {}", addr)
        };
        let (row, _) = create_airdrop_option_row(&format!("💻 {}", name), &subtitle);
        let target_id = id.clone();
        let pop_c = popover.clone();
        row.connect_clicked(move |_| {
            pop_c.set_visible(false);
            pick_and_send_airdrop_file_cc(target_id.clone());
        });
        peers_box.append(&row);
    }
}

fn pick_and_send_airdrop_file_cc(target: String) {
    let dialog = gtk4::FileDialog::builder()
        .title("Enviar archivo por AirDrop")
        .build();
    dialog.open(
        None::<&gtk4::Window>,
        None::<&gtk4::gio::Cancellable>,
        move |res| {
            if let Ok(file) = res {
                if let Some(path) = file.path() {
                    let path_str = path.to_string_lossy().into_owned();
                    let target_c = target.clone();
                    glib::MainContext::default().spawn_local(async move {
                        if let Ok(conn) = zbus::Connection::session().await {
                            let _ = conn
                                .call_method(
                                    Some("org.malus_de.Link"),
                                    "/org/malus_de/Link",
                                    Some("org.malus_de.Link"),
                                    "SendAirDropFile",
                                    &(target_c, path_str),
                                )
                                .await;
                        }
                    });
                }
            }
        },
    );
}

async fn get_airdrop_status_json_cc_dbus() -> Option<serde_json::Value> {
    let conn = zbus::Connection::session().await.ok()?;
    let reply = conn
        .call_method(
            Some("org.malus_de.Link"),
            "/org/malus_de/Link",
            Some("org.malus_de.Link"),
            "GetAirDropStatusJson",
            &(),
        )
        .await
        .ok()?;
    let raw: String = reply.body().deserialize().ok()?;
    serde_json::from_str(&raw).ok()
}

async fn scan_airdrop_peers_cc_dbus(seconds: u32) -> Vec<(String, String, String)> {
    if let Ok(conn) = zbus::Connection::session().await {
        if let Ok(reply) = conn
            .call_method(
                Some("org.malus_de.Link"),
                "/org/malus_de/Link",
                Some("org.malus_de.Link"),
                "ScanAirDropPeers",
                &(seconds,),
            )
            .await
        {
            if let Ok(peers) = reply.body().deserialize::<Vec<(String, String, String)>>() {
                return peers;
            }
        }
    }
    Vec::new()
}

fn create_airdrop_option_row(title: &str, subtitle: &str) -> (Button, Image) {
    let btn = Button::builder()
        .css_classes(["cc-list-row"])
        .has_frame(false)
        .build();

    let content = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();

    let text_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .hexpand(true)
        .spacing(1)
        .build();

    let title_lbl = Label::builder()
        .label(title)
        .css_classes(["cc-item-title"])
        .xalign(0.0)
        .build();

    let sub_lbl = Label::builder()
        .label(subtitle)
        .css_classes(["cc-item-subtitle"])
        .xalign(0.0)
        .build();

    text_box.append(&title_lbl);
    text_box.append(&sub_lbl);

    let check_icon = Image::builder()
        .icon_name("object-select-symbolic")
        .css_classes(["cc-item-check"])
        .pixel_size(14)
        .visible(false)
        .build();

    content.append(&text_box);
    content.append(&check_icon);
    btn.set_child(Some(&content));

    (btn, check_icon)
}

async fn get_airdrop_mode_dbus() -> String {
    if let Ok(connection) = zbus::Connection::session().await {
        if let Ok(reply) = connection
            .call_method(
                Some("org.malus_de.Link"),
                "/org/malus_de/Link",
                Some("org.malus_de.Link"),
                "GetAirDropMode",
                &(),
            )
            .await
        {
            if let Ok(mode) = reply.body().deserialize::<String>() {
                return mode;
            }
        }
    }
    "off".to_string()
}

async fn set_airdrop_mode_dbus(mode: &str) {
    if let Ok(connection) = zbus::Connection::session().await {
        let _ = connection
            .call_method(
                Some("org.malus_de.Link"),
                "/org/malus_de/Link",
                Some("org.malus_de.Link"),
                "SetAirDropMode",
                &(mode,),
            )
            .await;
    }
}

fn update_airdrop_ui_mode(btn: &Button, icon: &Image, sub: &Label, mode: &str) {
    match mode {
        "contacts" => {
            btn.add_css_class("cc-tile-active");
            if let Some(icon_box) = icon.parent() {
                icon_box.add_css_class("cc-icon-circle-active");
            }
            sub.set_label("Solo contactos");
        }
        "everyone" => {
            btn.add_css_class("cc-tile-active");
            if let Some(icon_box) = icon.parent() {
                icon_box.add_css_class("cc-icon-circle-active");
            }
            sub.set_label("Todos");
        }
        _ => {
            btn.remove_css_class("cc-tile-active");
            if let Some(icon_box) = icon.parent() {
                icon_box.remove_css_class("cc-icon-circle-active");
            }
            sub.set_label("Desactivado");
        }
    }
}

// ==========================================
// Subvista Wi-Fi (NetworkManager D-Bus)
// ==========================================

fn create_wifi_subview(
    stack: &Stack,
    popover: &Window,
    main_wifi_btn: &Button,
    main_wifi_icon: &Image,
    main_wifi_sub: &Label,
) -> (GtkBox, Rc<dyn Fn()>) {
    let view_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-subview"])
        .spacing(8)
        .build();

    // Header: Back btn + Title + Switch
    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-subview-header"])
        .spacing(6)
        .build();

    let back_btn = Button::builder()
        .css_classes(["cc-back-btn"])
        .has_frame(false)
        .tooltip_text("Volver")
        .build();
    let back_icon = Image::from_icon_name("go-previous-symbolic");
    back_btn.set_child(Some(&back_icon));

    let stack_back = stack.clone();
    back_btn.connect_clicked(move |_| {
        stack_back.set_visible_child_name("main");
    });

    let title_lbl = Label::builder()
        .label("Wi-Fi")
        .css_classes(["cc-subview-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let wifi_switch = Switch::builder().valign(Align::Center).build();

    header.append(&back_btn);
    header.append(&title_lbl);
    header.append(&*wifi_switch);
    view_box.append(&header);

    // Contenedor con Scroll para redes
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .min_content_height(180)
        .max_content_height(250)
        .propagate_natural_height(true)
        .build();

    let content_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();
    scrolled.set_child(Some(&content_box));
    view_box.append(&scrolled);

    // Footer: Separador y acceso a Configuración del Sistema
    let footer_sep = Separator::new(Orientation::Horizontal);
    let pref_btn = Button::builder()
        .css_classes(["cc-footer-btn"])
        .label("Preferencias de Wi-Fi...")
        .has_frame(false)
        .build();

    let pop_c = popover.clone();
    pref_btn.connect_clicked(move |_| {
        pop_c.set_visible(false);
        let settings_bin = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|dir| dir.join("malus-de-settings")))
            .filter(|p| p.exists())
            .unwrap_or_else(|| std::path::PathBuf::from("malus-de-settings"));
        let mut command = Command::new(settings_bin);
        malus_de_common::config::set_cursor_environment(&mut command);
        let _ = command.args(["--panel", "wifi"]).spawn();
    });

    view_box.append(&footer_sep);
    view_box.append(&pref_btn);

    // Switch connect state
    let is_updating_switch = Rc::new(RefCell::new(false));
    let is_upd_sw = is_updating_switch.clone();
    let m_btn = main_wifi_btn.clone();
    let m_ico = main_wifi_icon.clone();
    let m_sub = main_wifi_sub.clone();
    let cb_for_sw = content_box.clone();
    let sw_for_sw = wifi_switch.clone();
    let st_for_sw = stack.clone();
    let po_for_sw = popover.clone();

    wifi_switch.connect_state_set(move |_sw, state| {
        if *is_upd_sw.borrow() {
            return glib::Propagation::Proceed;
        }
        info!("Switch Wi-Fi cambiado a: {}", state);

        let mb_inner = m_btn.clone();
        let mi_inner = m_ico.clone();
        let ms_inner = m_sub.clone();
        let cb_inner = cb_for_sw.clone();
        let sw_inner = sw_for_sw.clone();
        let upd_inner = is_upd_sw.clone();
        let st_inner = st_for_sw.clone();
        let po_inner = po_for_sw.clone();

        glib::MainContext::default().spawn_local(async move {
            let _ = NetworkService::set_wifi_enabled(state).await;
            update_wifi_ui_async(&mb_inner, &mi_inner, &ms_inner).await;
            trigger_wifi_scan(
                &cb_inner, &sw_inner, &upd_inner, &mb_inner, &mi_inner, &ms_inner, &st_inner,
                &po_inner,
            );
        });

        glib::Propagation::Proceed
    });

    let cb_clone = content_box.clone();
    let sw_clone = wifi_switch.clone();
    let is_upd_clone = is_updating_switch.clone();
    let mb_clone = main_wifi_btn.clone();
    let mi_clone = main_wifi_icon.clone();
    let ms_clone = main_wifi_sub.clone();
    let st_clone = stack.clone();
    let po_clone = popover.clone();

    let refresh_closure: Rc<dyn Fn()> = Rc::new(move || {
        trigger_wifi_scan(
            &cb_clone,
            &sw_clone,
            &is_upd_clone,
            &mb_clone,
            &mi_clone,
            &ms_clone,
            &st_clone,
            &po_clone,
        );
    });

    (view_box, refresh_closure)
}

fn trigger_wifi_scan(
    content_box: &GtkBox,
    wifi_switch: &Switch,
    is_upd_sw: &Rc<RefCell<bool>>,
    m_btn: &Button,
    m_ico: &Image,
    m_sub: &Label,
    stack: &Stack,
    popover: &Window,
) {
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }

    let loading_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .halign(Align::Center)
        .valign(Align::Center)
        .margin_top(30)
        .margin_bottom(30)
        .build();

    let spinner = Spinner::builder().spinning(true).build();
    let lbl = Label::builder()
        .label("Buscando redes Wi-Fi...")
        .css_classes(["cc-empty-label"])
        .build();
    loading_box.append(&spinner);
    loading_box.append(&lbl);
    content_box.append(&loading_box);

    let cb_clone = content_box.clone();
    let sw_clone = wifi_switch.clone();
    let is_upd_clone = is_upd_sw.clone();
    let mb_clone = m_btn.clone();
    let mi_clone = m_ico.clone();
    let ms_clone = m_sub.clone();
    let st_clone = stack.clone();
    let po_clone = popover.clone();

    glib::MainContext::default().spawn_local(async move {
        let (enabled, networks) = NetworkService::scan_networks().await;

        *is_upd_clone.borrow_mut() = true;
        sw_clone.set_state(enabled);
        *is_upd_clone.borrow_mut() = false;

        update_wifi_ui_async(&mb_clone, &mi_clone, &ms_clone).await;

        populate_wifi_list(
            &cb_clone,
            enabled,
            networks,
            &sw_clone,
            &is_upd_clone,
            &mb_clone,
            &mi_clone,
            &ms_clone,
            &st_clone,
            &po_clone,
        );
    });
}

fn populate_wifi_list(
    content_box: &GtkBox,
    enabled: bool,
    networks: Vec<WifiApInfo>,
    wifi_switch: &Switch,
    is_upd_sw: &Rc<RefCell<bool>>,
    m_btn: &Button,
    m_ico: &Image,
    m_sub: &Label,
    stack: &Stack,
    popover: &Window,
) {
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }

    if !enabled {
        let disabled_lbl = Label::builder()
            .label("El Wi-Fi está desactivado")
            .css_classes(["cc-empty-label"])
            .halign(Align::Center)
            .margin_top(40)
            .margin_bottom(40)
            .build();
        content_box.append(&disabled_lbl);
        return;
    }

    // Red conectada actualmente (si existe)
    if let Some(active) = networks.iter().find(|n| n.in_use) {
        let active_header = Label::builder()
            .label("RED ACTUAL")
            .css_classes(["cc-section-label"])
            .xalign(0.0)
            .margin_start(4)
            .margin_top(4)
            .build();
        content_box.append(&active_header);

        let active_card = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .halign(Align::Fill)
            .valign(Align::Center)
            .hexpand(true)
            .spacing(8)
            .build();

        let sig_icon = Image::builder()
            .icon_name(get_wifi_signal_icon(active.signal))
            .pixel_size(16)
            .build();

        let ssid_lbl = Label::builder()
            .label(&active.ssid)
            .css_classes(["cc-item-title"])
            .hexpand(true)
            .halign(Align::Start)
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        let check_icon = Image::builder()
            .icon_name("object-select-symbolic")
            .css_classes(["cc-item-check"])
            .pixel_size(14)
            .build();

        active_card.append(&sig_icon);
        active_card.append(&ssid_lbl);
        active_card.append(&check_icon);
        let active_wrap =
            crate::ui::system_menu::wrap_tray_squircle_active_card(&active_card, 2, 8);
        content_box.append(&active_wrap);
    }

    // Cabecera de Otras Redes con botón de refresco
    let header_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-section-header-box"])
        .spacing(6)
        .build();

    let others_lbl = Label::builder()
        .label("OTRAS REDES")
        .css_classes(["cc-section-label"])
        .hexpand(true)
        .xalign(0.0)
        .margin_start(4)
        .build();

    let refresh_btn = Button::builder()
        .css_classes(["cc-refresh-btn"])
        .has_frame(false)
        .tooltip_text("Buscar redes Wi-Fi")
        .build();
    let ref_icon = Image::from_icon_name("view-refresh-symbolic");
    refresh_btn.set_child(Some(&ref_icon));

    let cb_ref = content_box.clone();
    let sw_ref = wifi_switch.clone();
    let upd_ref = is_upd_sw.clone();
    let mb_ref = m_btn.clone();
    let mi_ref = m_ico.clone();
    let ms_ref = m_sub.clone();
    let st_ref = stack.clone();
    let po_ref = popover.clone();
    refresh_btn.connect_clicked(move |_| {
        trigger_wifi_scan(
            &cb_ref, &sw_ref, &upd_ref, &mb_ref, &mi_ref, &ms_ref, &st_ref, &po_ref,
        );
    });

    header_box.append(&others_lbl);
    header_box.append(&refresh_btn);
    content_box.append(&header_box);

    let other_networks: Vec<&WifiApInfo> = networks.iter().filter(|n| !n.in_use).collect();
    if other_networks.is_empty() {
        let empty_lbl = Label::builder()
            .label("No se detectaron otras redes")
            .css_classes(["cc-empty-label"])
            .halign(Align::Center)
            .build();
        content_box.append(&empty_lbl);
    } else {
        for net in other_networks {
            let row_btn = Button::builder()
                .has_frame(false)
                .halign(Align::Fill)
                .hexpand(true)
                .build();

            let row_content = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .halign(Align::Fill)
                .valign(Align::Center)
                .hexpand(true)
                .spacing(8)
                .build();

            let sig_icon = Image::builder()
                .icon_name(get_wifi_signal_icon(net.signal))
                .pixel_size(16)
                .build();

            let ssid_lbl = Label::builder()
                .label(&net.ssid)
                .css_classes(["cc-item-title"])
                .hexpand(true)
                .halign(Align::Start)
                .xalign(0.0)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build();

            row_content.append(&sig_icon);
            row_content.append(&ssid_lbl);

            if net.is_encrypted {
                let lock_icon = Image::builder()
                    .icon_name("channel-secure-symbolic")
                    .css_classes(["cc-item-lock"])
                    .pixel_size(12)
                    .build();
                row_content.append(&lock_icon);
            }

            row_btn.set_child(Some(&row_content));
            crate::ui::system_menu::attach_tray_squircle_button(&row_btn, false, true, 2, 8);

            let target_ssid = net.ssid.clone();
            let cb_c = content_box.clone();
            let sw_c = wifi_switch.clone();
            let upd_c = is_upd_sw.clone();
            let mb_c = m_btn.clone();
            let mi_c = m_ico.clone();
            let ms_c = m_sub.clone();
            let st_c = stack.clone();
            let po_c = popover.clone();

            row_btn.connect_clicked(move |_| {
                info!("Conectando a red Wi-Fi: {}", target_ssid);
                let target = target_ssid.clone();
                let cb_inner = cb_c.clone();
                let sw_inner = sw_c.clone();
                let upd_inner = upd_c.clone();
                let mb_inner = mb_c.clone();
                let mi_inner = mi_c.clone();
                let ms_inner = ms_c.clone();
                let st_inner = st_c.clone();
                let po_inner = po_c.clone();

                glib::MainContext::default().spawn_local(async move {
                    let _ = NetworkService::connect_to_ssid(&target).await;
                    trigger_wifi_scan(
                        &cb_inner, &sw_inner, &upd_inner, &mb_inner, &mi_inner, &ms_inner,
                        &st_inner, &po_inner,
                    );
                });
            });

            content_box.append(&row_btn);
        }
    }
}

// ==========================================
// Subvista Bluetooth (BlueZ D-Bus)
// ==========================================

fn create_bt_subview(
    stack: &Stack,
    popover: &Window,
    main_bt_btn: &Button,
    main_bt_icon: &Image,
    main_bt_sub: &Label,
) -> (GtkBox, Rc<dyn Fn()>) {
    let view_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-subview"])
        .spacing(8)
        .build();

    // Header: Back btn + Title + Switch
    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-subview-header"])
        .spacing(6)
        .build();

    let back_btn = Button::builder()
        .css_classes(["cc-back-btn"])
        .has_frame(false)
        .tooltip_text("Volver")
        .build();
    let back_icon = Image::from_icon_name("go-previous-symbolic");
    back_btn.set_child(Some(&back_icon));

    let stack_back = stack.clone();
    back_btn.connect_clicked(move |_| {
        stack_back.set_visible_child_name("main");
    });

    let title_lbl = Label::builder()
        .label("Bluetooth")
        .css_classes(["cc-subview-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let bt_switch = Switch::builder().valign(Align::Center).build();

    header.append(&back_btn);
    header.append(&title_lbl);
    header.append(&*bt_switch);
    view_box.append(&header);

    // Contenedor con Scroll ajustado al contenido para dispositivos
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .max_content_height(250)
        .propagate_natural_height(true)
        .build();

    let content_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();
    scrolled.set_child(Some(&content_box));
    view_box.append(&scrolled);

    // Footer: Separador y acceso a Configuración del Sistema
    let footer_sep = Separator::new(Orientation::Horizontal);
    let pref_btn = Button::builder()
        .css_classes(["cc-footer-btn"])
        .has_frame(false)
        .halign(Align::Fill)
        .hexpand(true)
        .build();
    pref_btn.set_child(Some(
        &Label::builder()
            .label("Preferencias de Bluetooth...")
            .xalign(0.0)
            .halign(Align::Start)
            .hexpand(true)
            .build(),
    ));
    crate::ui::system_menu::attach_tray_squircle_button(&pref_btn, false, false, 4, 8);

    let pop_c = popover.clone();
    pref_btn.connect_clicked(move |_| {
        pop_c.set_visible(false);
        let settings_bin = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|dir| dir.join("malus-de-settings")))
            .filter(|p| p.exists())
            .unwrap_or_else(|| std::path::PathBuf::from("malus-de-settings"));
        let mut command = Command::new(settings_bin);
        malus_de_common::config::set_cursor_environment(&mut command);
        if let Err(err) = command.args(["--panel", "bluetooth"]).spawn() {
            tracing::error!("No se pudieron abrir los ajustes de Bluetooth de Malus DE: {err}");
        }
    });

    view_box.append(&footer_sep);
    view_box.append(&pref_btn);

    // Switch connect state
    let is_updating_switch = Rc::new(RefCell::new(false));
    let is_upd_sw = is_updating_switch.clone();
    let m_btn = main_bt_btn.clone();
    let m_ico = main_bt_icon.clone();
    let m_sub = main_bt_sub.clone();
    let cb_for_sw = content_box.clone();
    let sw_for_sw = bt_switch.clone();
    let st_for_sw = stack.clone();
    let po_for_sw = popover.clone();

    bt_switch.connect_state_set(move |_sw, state| {
        if *is_upd_sw.borrow() {
            return glib::Propagation::Proceed;
        }
        info!("Switch Bluetooth cambiado a: {}", state);

        let mb_inner = m_btn.clone();
        let mi_inner = m_ico.clone();
        let ms_inner = m_sub.clone();
        let cb_inner = cb_for_sw.clone();
        let sw_inner = sw_for_sw.clone();
        let upd_inner = is_upd_sw.clone();
        let st_inner = st_for_sw.clone();
        let po_inner = po_for_sw.clone();

        glib::MainContext::default().spawn_local(async move {
            let _ = BluetoothService::set_powered(state).await;
            update_bluetooth_ui_async(&mb_inner, &mi_inner, &ms_inner).await;
            trigger_bt_scan(
                &cb_inner, &sw_inner, &upd_inner, &mb_inner, &mi_inner, &ms_inner, &st_inner,
                &po_inner,
            );
        });

        glib::Propagation::Proceed
    });

    let cb_clone = content_box.clone();
    let sw_clone = bt_switch.clone();
    let is_upd_clone = is_updating_switch.clone();
    let mb_clone = main_bt_btn.clone();
    let mi_clone = main_bt_icon.clone();
    let ms_clone = main_bt_sub.clone();
    let st_clone = stack.clone();
    let po_clone = popover.clone();

    let refresh_closure: Rc<dyn Fn()> = Rc::new(move || {
        trigger_bt_scan(
            &cb_clone,
            &sw_clone,
            &is_upd_clone,
            &mb_clone,
            &mi_clone,
            &ms_clone,
            &st_clone,
            &po_clone,
        );
    });

    (view_box, refresh_closure)
}

fn trigger_bt_scan(
    content_box: &GtkBox,
    bt_switch: &Switch,
    is_upd_sw: &Rc<RefCell<bool>>,
    m_btn: &Button,
    m_ico: &Image,
    m_sub: &Label,
    stack: &Stack,
    popover: &Window,
) {
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }

    let loading_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .halign(Align::Center)
        .valign(Align::Center)
        .margin_top(30)
        .margin_bottom(30)
        .build();

    let spinner = Spinner::builder().spinning(true).build();
    let lbl = Label::builder()
        .label("Consultando dispositivos Bluetooth...")
        .css_classes(["cc-empty-label"])
        .build();
    loading_box.append(&spinner);
    loading_box.append(&lbl);
    content_box.append(&loading_box);

    let cb_clone = content_box.clone();
    let sw_clone = bt_switch.clone();
    let is_upd_clone = is_upd_sw.clone();
    let mb_clone = m_btn.clone();
    let mi_clone = m_ico.clone();
    let ms_clone = m_sub.clone();
    let st_clone = stack.clone();
    let po_clone = popover.clone();

    glib::MainContext::default().spawn_local(async move {
        let powered = BluetoothService::is_powered().await;
        let devices = BluetoothService::get_paired_devices().await;

        *is_upd_clone.borrow_mut() = true;
        sw_clone.set_state(powered);
        *is_upd_clone.borrow_mut() = false;

        update_bluetooth_ui_async(&mb_clone, &mi_clone, &ms_clone).await;

        populate_bt_list(
            &cb_clone,
            powered,
            devices,
            &sw_clone,
            &is_upd_clone,
            &mb_clone,
            &mi_clone,
            &ms_clone,
            &st_clone,
            &po_clone,
        );
    });
}

fn populate_bt_list(
    content_box: &GtkBox,
    enabled: bool,
    devices: Vec<BtDeviceInfo>,
    bt_switch: &Switch,
    is_upd_sw: &Rc<RefCell<bool>>,
    m_btn: &Button,
    m_ico: &Image,
    m_sub: &Label,
    stack: &Stack,
    popover: &Window,
) {
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }

    if !enabled {
        let disabled_lbl = Label::builder()
            .label("El Bluetooth está desactivado")
            .css_classes(["cc-empty-label"])
            .halign(Align::Center)
            .margin_top(40)
            .margin_bottom(40)
            .build();
        content_box.append(&disabled_lbl);
        return;
    }

    let header_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-section-header-box"])
        .spacing(6)
        .build();

    let dev_lbl = Label::builder()
        .label("DISPOSITIVOS VINCULADOS")
        .css_classes(["cc-section-label"])
        .hexpand(true)
        .xalign(0.0)
        .margin_start(4)
        .build();

    let refresh_btn = Button::builder()
        .css_classes(["cc-refresh-btn"])
        .has_frame(false)
        .tooltip_text("Actualizar dispositivos")
        .build();
    let ref_icon = Image::from_icon_name("view-refresh-symbolic");
    refresh_btn.set_child(Some(&ref_icon));

    let cb_ref = content_box.clone();
    let sw_ref = bt_switch.clone();
    let upd_ref = is_upd_sw.clone();
    let mb_ref = m_btn.clone();
    let mi_ref = m_ico.clone();
    let ms_ref = m_sub.clone();
    let st_ref = stack.clone();
    let po_ref = popover.clone();
    refresh_btn.connect_clicked(move |_| {
        trigger_bt_scan(
            &cb_ref, &sw_ref, &upd_ref, &mb_ref, &mi_ref, &ms_ref, &st_ref, &po_ref,
        );
    });

    header_box.append(&dev_lbl);
    header_box.append(&refresh_btn);
    content_box.append(&header_box);

    if devices.is_empty() {
        let empty_lbl = Label::builder()
            .label("No hay dispositivos vinculados")
            .css_classes(["cc-empty-label"])
            .halign(Align::Center)
            .build();
        content_box.append(&empty_lbl);
    } else {
        for dev in devices {
            let row_btn = Button::builder()
                .css_classes(["sound-menu-device-row"])
                .has_frame(false)
                .halign(Align::Fill)
                .hexpand(true)
                .build();

            let row_content = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .halign(Align::Fill)
                .valign(Align::Center)
                .hexpand(true)
                .vexpand(false)
                .spacing(8)
                .build();

            let circle = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .width_request(24)
                .height_request(24)
                .halign(Align::Start)
                .valign(Align::Center)
                .hexpand(false)
                .vexpand(false)
                .css_classes(if dev.connected {
                    ["sound-menu-device-circle", "sound-menu-device-circle-active"].as_slice()
                } else {
                    ["sound-menu-device-circle"].as_slice()
                })
                .build();
            let dev_icon = Image::builder()
                .icon_name(&dev.icon_name)
                .pixel_size(13)
                .width_request(24)
                .height_request(24)
                .halign(Align::Center)
                .valign(Align::Center)
                .hexpand(false)
                .vexpand(false)
                .build();
            circle.append(&dev_icon);

            let name_lbl = Label::builder()
                .label(&dev.name)
                .css_classes(["cc-item-title"])
                .xalign(0.0)
                .halign(Align::Start)
                .hexpand(true)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build();

            row_content.append(&circle);
            row_content.append(&name_lbl);

            if dev.connected {
                if let Some(bat) = dev.battery {
                    let bat_lbl = Label::builder()
                        .label(format!("{}%", bat))
                        .css_classes(["cc-item-subtitle"])
                        .xalign(1.0)
                        .halign(Align::End)
                        .hexpand(false)
                        .valign(Align::Center)
                        .build();
                    row_content.append(&bat_lbl);
                }
                let check_icon = Image::builder()
                    .icon_name("object-select-symbolic")
                    .css_classes(["cc-item-check"])
                    .pixel_size(14)
                    .hexpand(false)
                    .halign(Align::End)
                    .build();
                row_content.append(&check_icon);
            }

            row_btn.set_child(Some(&row_content));
            crate::ui::system_menu::attach_tray_squircle_button(&row_btn, false, false, 2, 6);

            let path = dev.path.clone();
            let is_connected = dev.connected;
            let cb_c = content_box.clone();
            let sw_c = bt_switch.clone();
            let upd_c = is_upd_sw.clone();
            let mb_c = m_btn.clone();
            let mi_c = m_ico.clone();
            let ms_c = m_sub.clone();
            let st_c = stack.clone();
            let po_c = popover.clone();

            row_btn.connect_clicked(move |_| {
                let p = path.clone();
                let cb_inner = cb_c.clone();
                let sw_inner = sw_c.clone();
                let upd_inner = upd_c.clone();
                let mb_inner = mb_c.clone();
                let mi_inner = mi_c.clone();
                let ms_inner = ms_c.clone();
                let st_inner = st_c.clone();
                let po_inner = po_c.clone();

                glib::MainContext::default().spawn_local(async move {
                    if is_connected {
                        BluetoothService::disconnect_device(&p).await;
                    } else {
                        BluetoothService::connect_device(&p).await;
                    }
                    trigger_bt_scan(
                        &cb_inner, &sw_inner, &upd_inner, &mb_inner, &mi_inner, &ms_inner,
                        &st_inner, &po_inner,
                    );
                });
            });

            content_box.append(&row_btn);
        }
    }
}

// ==========================================
// Utilidades Asíncronas de UI
// ==========================================

fn get_wifi_signal_icon(signal: u8) -> &'static str {
    if signal >= 80 {
        "network-wireless-signal-excellent-symbolic"
    } else if signal >= 55 {
        "network-wireless-signal-good-symbolic"
    } else if signal >= 30 {
        "network-wireless-signal-ok-symbolic"
    } else if signal >= 10 {
        "network-wireless-signal-weak-symbolic"
    } else {
        "network-wireless-signal-none-symbolic"
    }
}

// ==========================================
// Subvista Sonido (Salidas PipeWire + Mezclador por Aplicación)
// ==========================================

fn create_sound_subview(
    stack: &Stack,
    popover: &Window,
    main_vol_scale: &Scale,
    main_vol_pct: &Label,
    main_vol_icon: &Image,
    main_vol_lock: &Rc<RefCell<bool>>,
) -> (GtkBox, Rc<dyn Fn()>) {
    let view_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-subview"])
        .spacing(8)
        .build();

    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-subview-header"])
        .spacing(6)
        .build();

    let back_btn = Button::builder()
        .css_classes(["cc-back-btn"])
        .has_frame(false)
        .tooltip_text("Volver")
        .build();
    back_btn.set_child(Some(&Image::from_icon_name("go-previous-symbolic")));
    {
        let st = stack.clone();
        back_btn.connect_clicked(move |_| {
            st.set_visible_child_name("main");
        });
    }

    let title_lbl = Label::builder()
        .label("Sonido")
        .css_classes(["cc-subview-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let refresh_btn = Button::builder()
        .css_classes(["cc-refresh-btn"])
        .has_frame(false)
        .tooltip_text("Actualizar dispositivos y aplicaciones")
        .build();
    refresh_btn.set_child(Some(&Image::from_icon_name("view-refresh-symbolic")));

    header.append(&back_btn);
    header.append(&title_lbl);
    header.append(&refresh_btn);
    view_box.append(&header);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .max_content_height(300)
        .propagate_natural_height(true)
        .vexpand(false)
        .build();

    let content_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .vexpand(false)
        .build();
    scrolled.set_child(Some(&content_box));
    view_box.append(&scrolled);

    let footer_sep = Separator::new(Orientation::Horizontal);
    let pref_btn = Button::builder()
        .css_classes(["cc-footer-btn"])
        .has_frame(false)
        .halign(Align::Fill)
        .hexpand(true)
        .build();
    pref_btn.set_child(Some(
        &Label::builder()
            .label("Configuración de Sonido...")
            .xalign(0.0)
            .halign(Align::Start)
            .hexpand(true)
            .build(),
    ));
    crate::ui::system_menu::attach_tray_squircle_button(&pref_btn, false, false, 4, 8);
    {
        let pop_c = popover.clone();
        pref_btn.connect_clicked(move |_| {
            pop_c.set_visible(false);
            let settings_bin = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|dir| dir.join("malus-de-settings")))
                .filter(|p| p.exists())
                .unwrap_or_else(|| std::path::PathBuf::from("malus-de-settings"));
            let mut command = Command::new(settings_bin);
            malus_de_common::config::set_cursor_environment(&mut command);
            if let Err(err) = command.args(["--panel", "sound"]).spawn() {
                tracing::error!("No se pudieron abrir los ajustes de Sonido de Malus DE: {err}");
            }
        });
    }

    view_box.append(&footer_sep);
    view_box.append(&pref_btn);

    let cb = content_box.clone();
    let sc = main_vol_scale.clone();
    let pc = main_vol_pct.clone();
    let ic = main_vol_icon.clone();
    let lk = main_vol_lock.clone();
    let last_local_write = Rc::new(RefCell::new(
        std::time::Instant::now() - std::time::Duration::from_secs(5),
    ));
    let refresh_holder: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_fn: Rc<dyn Fn()> = {
        let lw = last_local_write.clone();
        let rf_h = refresh_holder.clone();
        Rc::new(move || {
            let cb_i = cb.clone();
            let sc_i = sc.clone();
            let pc_i = pc.clone();
            let ic_i = ic.clone();
            let lk_i = lk.clone();
            let lw_i = lw.clone();
            let rf_i = rf_h.clone();
            glib::MainContext::default().spawn_local(async move {
                let devices = AudioService::list_output_devices().await;
                let streams = AudioService::list_app_streams().await;
                populate_sound_subview_content(
                    &cb_i,
                    &devices,
                    &streams,
                    &sc_i,
                    &pc_i,
                    &ic_i,
                    &lk_i,
                    &lw_i,
                    &rf_i,
                );
            });
        })
    };
    *refresh_holder.borrow_mut() = Some(refresh_fn.clone());

    // Suscripción en tiempo real a PipeWire (`pactl subscribe`) solo mientras la subvista "sound" está abierta
    let sound_watcher: Rc<RefCell<Option<crate::services::audio::PactlWatcher>>> =
        Rc::new(RefCell::new(None));
    {
        let sw_c = sound_watcher.clone();
        let rf_c = refresh_fn.clone();
        let lw_c = last_local_write.clone();
        let pop_weak = popover.downgrade();
        let st_weak = stack.downgrade();
        let start_live_watcher = Rc::new(move || {
            if sw_c.borrow().is_some() {
                return;
            }
            let (tx, rx) = async_channel::bounded::<crate::services::audio::PactlEventKind>(32);
            let watcher = crate::services::audio::PactlWatcher::start(tx);
            *sw_c.borrow_mut() = Some(watcher);
            let rf_loop = rf_c.clone();
            let lw_loop = lw_c.clone();
            let pop_w = pop_weak.clone();
            let st_w = st_weak.clone();
            let pending_debounce = Rc::new(RefCell::new(false));
            glib::MainContext::default().spawn_local(async move {
                while let Ok(kind) = rx.recv().await {
                    let still_on_sound = pop_w
                        .upgrade()
                        .is_some_and(|p| p.is_visible())
                        && st_w
                            .upgrade()
                            .and_then(|s| s.visible_child_name())
                            .as_deref()
                            == Some("sound");
                    if !still_on_sound {
                        continue;
                    }
                    if matches!(kind, crate::services::audio::PactlEventKind::Change)
                        && std::time::Instant::now().duration_since(*lw_loop.borrow())
                            < std::time::Duration::from_millis(500)
                    {
                        continue;
                    }
                    if *pending_debounce.borrow() {
                        continue;
                    }
                    *pending_debounce.borrow_mut() = true;
                    let pd = pending_debounce.clone();
                    let rf_fire = rf_loop.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(180),
                        move || {
                            *pd.borrow_mut() = false;
                            rf_fire();
                        },
                    );
                }
            });
        });

        let sw_stop = sound_watcher.clone();
        let start_on_nav = start_live_watcher.clone();
        stack.connect_visible_child_name_notify(move |st| {
            if st.visible_child_name().as_deref() == Some("sound") {
                start_on_nav();
            } else {
                *sw_stop.borrow_mut() = None;
            }
        });

        let sw_close = sound_watcher.clone();
        popover.connect_hide(move |_| {
            *sw_close.borrow_mut() = None;
        });
    }

    {
        let rf = refresh_fn.clone();
        refresh_btn.connect_clicked(move |_| {
            rf();
        });
    }

    (view_box, refresh_fn)
}

#[allow(clippy::too_many_arguments)]
fn populate_sound_subview_content(
    content_box: &GtkBox,
    devices: &[crate::services::audio::AudioOutputDevice],
    streams: &[crate::services::audio::AppAudioStream],
    main_vol_scale: &Scale,
    main_vol_pct: &Label,
    main_vol_icon: &Image,
    main_vol_lock: &Rc<RefCell<bool>>,
    last_local_write: &Rc<RefCell<std::time::Instant>>,
    refresh_holder: &Rc<RefCell<Option<Rc<dyn Fn()>>>>,
) {
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }

    // 1. Salidas de audio PipeWire
    content_box.append(
        &Label::builder()
            .label("SALIDA")
            .css_classes(["cc-section-label"])
            .xalign(0.0)
            .halign(Align::Start)
            .margin_start(4)
            .build(),
    );

    let outputs_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .vexpand(false)
        .build();

    for dev in devices {
        let row_btn = Button::builder()
            .css_classes(["sound-menu-device-row"])
            .has_frame(false)
            .halign(Align::Fill)
            .hexpand(true)
            .build();

        let row_content = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .halign(Align::Fill)
            .valign(Align::Center)
            .hexpand(true)
            .vexpand(false)
            .spacing(8)
            .build();

        let circle = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .width_request(24)
            .height_request(24)
            .halign(Align::Start)
            .valign(Align::Center)
            .hexpand(false)
            .vexpand(false)
            .css_classes(if dev.is_default {
                ["sound-menu-device-circle", "sound-menu-device-circle-active"].as_slice()
            } else {
                ["sound-menu-device-circle"].as_slice()
            })
            .build();
        let dev_icon = Image::builder()
            .icon_name(dev.icon_name)
            .pixel_size(13)
            .width_request(24)
            .height_request(24)
            .halign(Align::Center)
            .valign(Align::Center)
            .hexpand(false)
            .vexpand(false)
            .build();
        circle.append(&dev_icon);

        let name_lbl = Label::builder()
            .label(&dev.description)
            .css_classes(["sound-menu-device-name"])
            .xalign(0.0)
            .halign(Align::Start)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        row_content.append(&circle);
        row_content.append(&name_lbl);
        row_btn.set_child(Some(&row_content));
        crate::ui::system_menu::attach_tray_squircle_button(&row_btn, false, false, 2, 6);

        let sink_name = dev.name.clone();
        let cb = content_box.clone();
        let sc = main_vol_scale.clone();
        let pc = main_vol_pct.clone();
        let ic = main_vol_icon.clone();
        let lk = main_vol_lock.clone();
        let lw = last_local_write.clone();
        let rf_h = refresh_holder.clone();
        row_btn.connect_clicked(move |_| {
            let target = sink_name.clone();
            let cb_i = cb.clone();
            let sc_i = sc.clone();
            let pc_i = pc.clone();
            let ic_i = ic.clone();
            let lk_i = lk.clone();
            let lw_i = lw.clone();
            let rf_i = rf_h.clone();
            glib::MainContext::default().spawn_local(async move {
                AudioService::set_default_sink(target).await;
                let (vol, muted) = AudioService::get_volume().await;
                *lk_i.borrow_mut() = true;
                sc_i.set_value(if muted { 0.0 } else { vol });
                pc_i.set_label(&format!("{}%", if muted { 0 } else { vol.round() as i32 }));
                ic_i.set_icon_name(Some(if muted || vol < 1.0 {
                    "audio-volume-muted-symbolic"
                } else if vol < 40.0 {
                    "audio-volume-low-symbolic"
                } else if vol < 75.0 {
                    "audio-volume-medium-symbolic"
                } else {
                    "audio-volume-high-symbolic"
                }));
                *lk_i.borrow_mut() = false;
                super::status_tray::refresh_volume_tray();
                let updated_devs = AudioService::list_output_devices().await;
                let updated_streams = AudioService::list_app_streams().await;
                populate_sound_subview_content(
                    &cb_i,
                    &updated_devs,
                    &updated_streams,
                    &sc_i,
                    &pc_i,
                    &ic_i,
                    &lk_i,
                    &lw_i,
                    &rf_i,
                );
            });
        });

        outputs_box.append(&row_btn);
    }
    content_box.append(&outputs_box);

    // 2. Mezclador de Audio por Aplicación (PipeWire sink-inputs: volumen, canales L/R y salida por app)
    content_box.append(
        &Separator::builder()
            .orientation(Orientation::Horizontal)
            .css_classes(["sound-menu-sep"])
            .margin_top(2)
            .build(),
    );
    content_box.append(
        &Label::builder()
            .label("MEZCLADOR POR APLICACIÓN")
            .css_classes(["cc-section-label"])
            .xalign(0.0)
            .halign(Align::Start)
            .margin_start(4)
            .build(),
    );

    if streams.is_empty() {
        content_box.append(
            &Label::builder()
                .label("No hay aplicaciones reproduciendo audio")
                .css_classes(["sound-menu-section"])
                .xalign(0.0)
                .halign(Align::Start)
                .margin_start(6)
                .margin_top(2)
                .margin_bottom(2)
                .build(),
        );
    } else {
        let on_routed: Rc<dyn Fn()> = {
            let rf_h = refresh_holder.clone();
            Rc::new(move || {
                if let Some(rf) = rf_h.borrow().as_ref() {
                    rf();
                }
            })
        };
        for stream in streams {
            content_box.append(&build_per_app_mixer_row(
                stream,
                devices,
                last_local_write,
                on_routed.clone(),
            ));
        }
    }
}

pub(crate) fn build_per_app_mixer_row(
    stream: &crate::services::audio::AppAudioStream,
    devices: &[crate::services::audio::AudioOutputDevice],
    last_local_write: &Rc<RefCell<std::time::Instant>>,
    on_routed: Rc<dyn Fn()>,
) -> GtkBox {
    let container = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(3)
        .css_classes(["sound-mixer-row"])
        .vexpand(false)
        .build();

    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .halign(Align::Fill)
        .valign(Align::Center)
        .hexpand(true)
        .build();

    let app_icon = Image::builder()
        .icon_name(&stream.icon_name)
        .pixel_size(14)
        .hexpand(false)
        .vexpand(false)
        .build();

    let title_text = if let Some(ref media) = stream.media_name {
        if media.chars().count() <= 22 {
            format!("{} — {}", stream.app_name, media)
        } else {
            stream.app_name.clone()
        }
    } else {
        stream.app_name.clone()
    };
    let app_lbl = Label::builder()
        .label(&title_text)
        .css_classes(["sound-mixer-app-name"])
        .xalign(0.0)
        .halign(Align::Start)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    if let Some(ref media) = stream.media_name {
        app_lbl.set_tooltip_text(Some(&format!("{} — {}", stream.app_name, media)));
    }

    let pct_lbl = Label::builder()
        .label(if stream.muted {
            "Silencio".to_string()
        } else {
            format!("{}%", stream.volume_pct.round() as i32)
        })
        .css_classes(["sound-mixer-pct"])
        .xalign(1.0)
        .halign(Align::End)
        .hexpand(false)
        .build();

    header.append(&app_icon);
    header.append(&app_lbl);
    header.append(&pct_lbl);

    // Botón para desplegar canales individuales (Izquierdo / Derecho) estilo KDE
    let has_multi_channels = stream.channels.len() >= 2;
    let channels_unbalanced = has_multi_channels
        && (stream
            .channels
            .iter()
            .map(|(_, v)| *v)
            .fold(f64::NEG_INFINITY, f64::max)
            - stream
                .channels
                .iter()
                .map(|(_, v)| *v)
                .fold(f64::INFINITY, f64::min))
            >= 2.0;

    let channels_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .margin_start(18)
        .visible(channels_unbalanced)
        .css_classes(["sound-mixer-channels-box"])
        .build();

    if has_multi_channels {
        let ch_toggle = Button::builder()
            .css_classes(["sound-menu-vol-btn", "sound-mixer-sub-toggle"])
            .has_frame(false)
            .valign(Align::Center)
            .tooltip_text("Separar canales (Izquierdo / Derecho)")
            .build();
        let ch_ico = Image::builder()
            .icon_name(if channels_unbalanced {
                "pan-up-symbolic"
            } else {
                "pan-down-symbolic"
            })
            .pixel_size(12)
            .build();
        ch_toggle.set_child(Some(&ch_ico));
        let cb = channels_box.clone();
        let ico = ch_ico.clone();
        ch_toggle.connect_clicked(move |_| {
            let next = !cb.is_visible();
            cb.set_visible(next);
            ico.set_icon_name(Some(if next {
                "pan-up-symbolic"
            } else {
                "pan-down-symbolic"
            }));
        });
        header.append(&ch_toggle);
    }

    container.append(&header);

    let slider_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();

    let muted_state = Rc::new(RefCell::new(stream.muted));
    let mute_btn = Button::builder()
        .css_classes(["sound-menu-vol-btn", "sound-mixer-mute-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .tooltip_text("Silenciar / Reactivar aplicación")
        .build();
    let mute_icon = Image::builder()
        .icon_name(if stream.muted {
            "audio-volume-muted-symbolic"
        } else {
            "audio-volume-medium-symbolic"
        })
        .pixel_size(13)
        .build();
    mute_btn.set_child(Some(&mute_icon));

    let slider = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    disable_scale_scroll(&slider);
    slider.set_value(stream.volume_pct.clamp(0.0, 100.0));
    slider.set_draw_value(false);
    slider.set_hexpand(true);
    slider.set_valign(Align::Center);
    slider.set_css_classes(&["sound-menu-scale"]);

    slider_row.append(&mute_btn);
    slider_row.append(&slider);
    container.append(&slider_row);

    let idx = stream.index;
    let syncing_sliders = Rc::new(RefCell::new(false));
    let channel_values: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(
        stream
            .channels
            .iter()
            .map(|(_, v)| v.clamp(0.0, 100.0))
            .collect(),
    ));
    let channel_widgets: Rc<RefCell<Vec<(Scale, Label)>>> = Rc::new(RefCell::new(Vec::new()));

    if has_multi_channels {
        for (ch_idx, (ch_name, ch_val)) in stream.channels.iter().enumerate() {
            let ch_row = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(6)
                .valign(Align::Center)
                .build();
            let ch_lbl = Label::builder()
                .label(&crate::services::audio::format_channel_label(ch_name))
                .css_classes(["sound-mixer-ch-label"])
                .xalign(0.0)
                .width_request(58)
                .build();
            let ch_scale = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
            disable_scale_scroll(&ch_scale);
            ch_scale.set_value(ch_val.clamp(0.0, 100.0));
            ch_scale.set_draw_value(false);
            ch_scale.set_hexpand(true);
            ch_scale.set_valign(Align::Center);
            ch_scale.set_css_classes(&["sound-menu-scale"]);
            let ch_pct = Label::builder()
                .label(format!("{}%", ch_val.round() as i32))
                .css_classes(["sound-mixer-pct"])
                .xalign(1.0)
                .width_request(32)
                .build();

            ch_row.append(&ch_lbl);
            ch_row.append(&ch_scale);
            ch_row.append(&ch_pct);
            channels_box.append(&ch_row);

            channel_widgets
                .borrow_mut()
                .push((ch_scale.clone(), ch_pct.clone()));

            let vals_c = channel_values.clone();
            let sync_c = syncing_sliders.clone();
            let master_s = slider.clone();
            let master_pct = pct_lbl.clone();
            let master_ico = mute_icon.clone();
            let m_state = muted_state.clone();
            let lw = last_local_write.clone();
            let ch_pct_c = ch_pct.clone();
            ch_scale.connect_value_changed(move |s| {
                if *sync_c.borrow() {
                    return;
                }
                let val = s.value().clamp(0.0, 100.0);
                *lw.borrow_mut() = std::time::Instant::now();
                ch_pct_c.set_label(&format!("{}%", val.round() as i32));
                let updated = {
                    let mut guard = vals_c.borrow_mut();
                    if let Some(slot) = guard.get_mut(ch_idx) {
                        *slot = val;
                    }
                    guard.clone()
                };
                if *m_state.borrow() && val > 0.5 {
                    *m_state.borrow_mut() = false;
                    AudioService::set_app_stream_mute(idx, false);
                }
                let avg = if updated.is_empty() {
                    val
                } else {
                    updated.iter().sum::<f64>() / updated.len() as f64
                };
                *sync_c.borrow_mut() = true;
                master_s.set_value(avg.clamp(0.0, 100.0));
                *sync_c.borrow_mut() = false;
                master_pct.set_label(&format!("{}%", avg.round() as i32));
                master_ico.set_icon_name(Some(if avg < 1.0 {
                    "audio-volume-muted-symbolic"
                } else if avg < 40.0 {
                    "audio-volume-low-symbolic"
                } else {
                    "audio-volume-medium-symbolic"
                }));
                AudioService::set_app_stream_channel_volumes(idx, updated);
            });
        }
        container.append(&channels_box);
    }

    // Selector inline de salida por aplicación (cuando hay 2+ salidas conectadas reales)
    if devices.len() >= 2 {
        let current_sink_desc = stream
            .sink_index
            .and_then(|s_idx| devices.iter().find(|d| d.index == Some(s_idx)))
            .or_else(|| devices.iter().find(|d| d.is_default))
            .map(|d| d.description.as_str())
            .unwrap_or("Salida predeterminada");

        let route_picker = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .margin_start(14)
            .visible(false)
            .build();

        let route_btn = Button::builder()
            .css_classes(["sound-mixer-route-btn"])
            .has_frame(false)
            .halign(Align::Start)
            .tooltip_text("Elegir salida de audio para esta aplicación")
            .build();
        let route_btn_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(5)
            .valign(Align::Center)
            .build();
        route_btn_box.append(
            &Image::builder()
                .icon_name("audio-speakers-symbolic")
                .pixel_size(11)
                .build(),
        );
        route_btn_box.append(
            &Label::builder()
                .label(current_sink_desc)
                .css_classes(["sound-mixer-route-label"])
                .xalign(0.0)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .max_width_chars(24)
                .build(),
        );
        let route_chev = Image::builder()
            .icon_name("pan-down-symbolic")
            .pixel_size(10)
            .build();
        route_btn_box.append(&route_chev);
        route_btn.set_child(Some(&route_btn_box));

        {
            let rp = route_picker.clone();
            let rc = route_chev.clone();
            route_btn.connect_clicked(move |_| {
                let next = !rp.is_visible();
                rp.set_visible(next);
                rc.set_icon_name(Some(if next {
                    "pan-up-symbolic"
                } else {
                    "pan-down-symbolic"
                }));
            });
        }

        for dev in devices {
            let is_current = stream
                .sink_index
                .is_some_and(|s_idx| dev.index == Some(s_idx));
            let opt_btn = Button::builder()
                .css_classes(["sound-menu-device-row"])
                .has_frame(false)
                .halign(Align::Fill)
                .hexpand(true)
                .build();
            let opt_box = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(6)
                .halign(Align::Fill)
                .valign(Align::Center)
                .hexpand(true)
                .build();
            let opt_ico = Image::builder()
                .icon_name(dev.icon_name)
                .pixel_size(12)
                .hexpand(false)
                .build();
            let opt_lbl = Label::builder()
                .label(&dev.description)
                .css_classes(["sound-mixer-ch-label"])
                .xalign(0.0)
                .halign(Align::Start)
                .hexpand(true)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build();
            opt_box.append(&opt_ico);
            opt_box.append(&opt_lbl);
            if is_current {
                opt_box.append(
                    &Image::builder()
                        .icon_name("object-select-symbolic")
                        .css_classes(["cc-item-check"])
                        .pixel_size(11)
                        .hexpand(false)
                        .build(),
                );
            }
            opt_btn.set_child(Some(&opt_box));
            crate::ui::system_menu::attach_tray_squircle_button(&opt_btn, false, false, 2, 6);

            let target_sink = dev.name.clone();
            let on_r = on_routed.clone();
            let lw = last_local_write.clone();
            opt_btn.connect_clicked(move |_| {
                *lw.borrow_mut() = std::time::Instant::now();
                let sink_bg = target_sink.clone();
                let on_r_c = on_r.clone();
                glib::MainContext::default().spawn_local(async move {
                    AudioService::move_app_stream_to_sink(idx, sink_bg).await;
                    on_r_c();
                });
            });
            route_picker.append(&opt_btn);
        }

        container.append(&route_btn);
        container.append(&route_picker);
    }

    {
        let pct_c = pct_lbl.clone();
        let ico_c = mute_icon.clone();
        let m_state = muted_state.clone();
        let sync_c = syncing_sliders.clone();
        let vals_c = channel_values.clone();
        let ch_ws = channel_widgets.clone();
        let lw = last_local_write.clone();
        slider.connect_value_changed(move |s| {
            if *sync_c.borrow() {
                return;
            }
            let val = s.value().clamp(0.0, 100.0);
            *lw.borrow_mut() = std::time::Instant::now();
            if *m_state.borrow() && val > 0.5 {
                *m_state.borrow_mut() = false;
                AudioService::set_app_stream_mute(idx, false);
            }
            pct_c.set_label(&format!("{}%", val.round() as i32));
            ico_c.set_icon_name(Some(if val < 1.0 {
                "audio-volume-muted-symbolic"
            } else if val < 40.0 {
                "audio-volume-low-symbolic"
            } else {
                "audio-volume-medium-symbolic"
            }));
            *sync_c.borrow_mut() = true;
            for slot in vals_c.borrow_mut().iter_mut() {
                *slot = val;
            }
            for (ch_s, ch_p) in ch_ws.borrow().iter() {
                ch_s.set_value(val);
                ch_p.set_label(&format!("{}%", val.round() as i32));
            }
            *sync_c.borrow_mut() = false;
            AudioService::set_app_stream_volume(idx, val);
        });
    }

    {
        let pct_c = pct_lbl.clone();
        let ico_c = mute_icon.clone();
        let m_state = muted_state.clone();
        let s_c = slider.clone();
        let lw = last_local_write.clone();
        mute_btn.connect_clicked(move |_| {
            *lw.borrow_mut() = std::time::Instant::now();
            let next_muted = !*m_state.borrow();
            *m_state.borrow_mut() = next_muted;
            AudioService::set_app_stream_mute(idx, next_muted);
            ico_c.set_icon_name(Some(if next_muted {
                "audio-volume-muted-symbolic"
            } else {
                "audio-volume-medium-symbolic"
            }));
            pct_c.set_label(&if next_muted {
                "Silencio".to_string()
            } else {
                format!("{}%", s_c.value().round() as i32)
            });
        });
    }

    container
}

async fn update_wifi_ui_async(btn: &Button, icon: &Image, sub: &Label) {
    let enabled = NetworkService::is_wifi_enabled().await;
    if enabled {
        btn.add_css_class("cc-tile-active");
        if let Some(icon_box) = icon.parent() {
            icon_box.add_css_class("cc-icon-circle-active");
        }
        if let Some(ssid) = NetworkService::get_active_wifi_ssid().await {
            sub.set_label(&ssid);
        } else {
            sub.set_label("Activado");
        }
    } else {
        btn.remove_css_class("cc-tile-active");
        if let Some(icon_box) = icon.parent() {
            icon_box.remove_css_class("cc-icon-circle-active");
        }
        sub.set_label("Desactivado");
    }
}

async fn update_bluetooth_ui_async(btn: &Button, icon: &Image, sub: &Label) {
    let enabled = BluetoothService::is_powered().await;
    if enabled {
        btn.add_css_class("cc-tile-active");
        if let Some(icon_box) = icon.parent() {
            icon_box.add_css_class("cc-icon-circle-active");
        }
        if let Some(dev_name) = BluetoothService::get_active_device_name().await {
            sub.set_label(&dev_name);
        } else {
            sub.set_label("Activado");
        }
    } else {
        btn.remove_css_class("cc-tile-active");
        if let Some(icon_box) = icon.parent() {
            icon_box.remove_css_class("cc-icon-circle-active");
        }
        sub.set_label("Desactivado");
    }
}

pub fn apply_system_appearance_mode(to_dark: bool) {
    let target = if to_dark {
        "prefer-dark"
    } else {
        "prefer-light"
    };
    info!("🎨 Aplicando modo de apariencia de Malus DE: {}", target);

    match malus_de_common::ipc::IpcClient::prepare_theme_transition() {
        Ok(true) => info!("✨ Fotograma anterior guardado para la transición de tema"),
        Ok(false) => tracing::warn!(
            "No se preparó la transición de tema; se aplicará el cambio sin animación"
        ),
        Err(error) => tracing::warn!(
            "No se pudo preparar la transición de tema: {}; se aplicará el cambio sin animación",
            error
        ),
    }

    // 1. Sincronizar las preferencias aisladas de GSettings, GTK y CSS de Malus DE.
    malus_de_common::config::sync_system_appearance(to_dark);

    // 2. Actualizar AdwStyleManager y popovers en el proceso actual sin doble recálculo CSS.
    libadwaita::StyleManager::default().set_color_scheme(if to_dark {
        libadwaita::ColorScheme::ForceDark
    } else {
        libadwaita::ColorScheme::ForceLight
    });
    crate::ui::theme::update_all_popovers_theme(to_dark);
    sync_control_center_theme(to_dark);

    // 3. Notificar a otros módulos mediante IPC
    let _ = malus_de_common::ipc::IpcClient::send_command(
        malus_de_common::ipc::IpcMessage::ThemeChanged {
            theme_name: "macOS".to_string(),
            is_dark: to_dark,
        },
    );
}

async fn refresh_volume_ui_async(
    scale: &Scale,
    pct_lbl: &Label,
    icon: &Image,
    is_updating: &Rc<RefCell<bool>>,
) {
    let (vol_val, is_muted) = AudioService::get_volume().await;

    *is_updating.borrow_mut() = true;
    scale.set_value(vol_val);
    *is_updating.borrow_mut() = false;

    if is_muted {
        pct_lbl.set_label("Silenciado");
        icon.set_icon_name(Some("audio-volume-muted-symbolic"));
    } else {
        pct_lbl.set_label(&format!("{}%", vol_val.round() as i32));
        if vol_val < 1.0 {
            icon.set_icon_name(Some("audio-volume-muted-symbolic"));
        } else if vol_val < 40.0 {
            icon.set_icon_name(Some("audio-volume-low-symbolic"));
        } else if vol_val < 75.0 {
            icon.set_icon_name(Some("audio-volume-medium-symbolic"));
        } else {
            icon.set_icon_name(Some("audio-volume-high-symbolic"));
        }
    }
}

async fn refresh_brightness_ui_async(
    scale: &Scale,
    pct_lbl: &Label,
    icon: &Image,
    is_updating: &Rc<RefCell<bool>>,
) {
    let b = BrightnessService::get_brightness().await;

    *is_updating.borrow_mut() = true;
    scale.set_value(b);
    *is_updating.borrow_mut() = false;

    pct_lbl.set_label(&format!("{}%", b.round() as i32));
    if b < 35.0 {
        icon.set_icon_name(Some("display-brightness-low-symbolic"));
    } else if b < 70.0 {
        icon.set_icon_name(Some("display-brightness-medium-symbolic"));
    } else {
        icon.set_icon_name(Some("display-brightness-symbolic"));
    }
}

#[allow(dead_code)]
async fn refresh_night_shift_ui_async(btn: &Button, lbl: &Label) {
    let enabled = BrightnessService::get_night_shift().await;
    if enabled {
        btn.add_css_class("cc-pill-btn-active");
        lbl.set_label("🌙 Modo Noche: Activo");
    } else {
        btn.remove_css_class("cc-pill-btn-active");
        lbl.set_label("🌙 Modo Noche: Inactivo");
    }
}

fn format_monitors_summary(cfg: &malus_de_common::config::DisplaysConfig) -> String {
    if cfg.outputs.is_empty() {
        return format!("Escala global: {:.1}%", cfg.master_scale * 100.0);
    }
    let mut sorted: Vec<_> = cfg.outputs.iter().collect();
    sorted.sort_by_key(|o| o.x);
    let parts: Vec<String> = sorted
        .iter()
        .map(|o| {
            let (lw, lh) = o.logical_size();
            let short_name = if let Some(diag) = o.effective_diagonal() {
                format!("{} {:.0}\"", o.connector, diag)
            } else if !o.name.is_empty() && o.name != o.connector {
                o.name.clone()
            } else {
                o.connector.clone()
            };
            format!(
                "{}{} ({}): {:.1}% · {}×{} @({}, {})",
                if o.primary { "★ " } else { "" },
                short_name,
                o.connector,
                o.scale * 100.0,
                lw,
                lh,
                o.x,
                o.y
            )
        })
        .collect();
    parts.join("  |  ")
}

// ==========================================
// Tarjeta de Teclado / Idioma (Español Latinoamericano 'latam')
// ==========================================

#[allow(dead_code)]
fn create_keyboard_card() -> (GtkBox, Rc<dyn Fn()>) {
    let card = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-slider-card"])
        .spacing(6)
        .build();

    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();

    let icon_lbl = Label::builder()
        .label("⌨️ Teclado")
        .css_classes(["cc-tile-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let input_cfg = malus_de_common::config::InputConfig::load();
    let active_lbl = Label::builder()
        .label(input_cfg.layout_display_name())
        .css_classes(["cc-tile-subtitle"])
        .build();

    header.append(&icon_lbl);
    header.append(&active_lbl);

    let pills_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .homogeneous(true)
        .build();

    let btn_latam = Button::builder()
        .css_classes(["cc-pill-btn"])
        .has_frame(false)
        .tooltip_text("Español (Latinoamérica) — tecla ñ, acentos y símbolos latam")
        .build();
    btn_latam.set_child(Some(
        &Label::builder()
            .label("🇲🇽 Latam (ñ)")
            .css_classes(["cc-tile-subtitle"])
            .build(),
    ));

    let btn_es = Button::builder()
        .css_classes(["cc-pill-btn"])
        .has_frame(false)
        .tooltip_text("Español (España / ISO)")
        .build();
    btn_es.set_child(Some(
        &Label::builder()
            .label("🇪🇸 España")
            .css_classes(["cc-tile-subtitle"])
            .build(),
    ));

    let btn_us = Button::builder()
        .css_classes(["cc-pill-btn"])
        .has_frame(false)
        .tooltip_text("Inglés (EE. UU. / ANSI)")
        .build();
    btn_us.set_child(Some(
        &Label::builder()
            .label("🇺🇸 Inglés US")
            .css_classes(["cc-tile-subtitle"])
            .build(),
    ));

    pills_row.append(&btn_latam);
    pills_row.append(&btn_es);
    pills_row.append(&btn_us);

    card.append(&header);
    card.append(&pills_row);

    let b_latam = btn_latam.clone();
    let b_es = btn_es.clone();
    let b_us = btn_us.clone();
    let a_lbl = active_lbl.clone();

    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        let cfg = malus_de_common::config::InputConfig::load();
        a_lbl.set_label(cfg.layout_display_name());
        b_latam.remove_css_class("cc-pill-btn-active");
        b_es.remove_css_class("cc-pill-btn-active");
        b_us.remove_css_class("cc-pill-btn-active");
        match cfg.keyboard_layout.as_str() {
            "latam" => b_latam.add_css_class("cc-pill-btn-active"),
            "es" => b_es.add_css_class("cc-pill-btn-active"),
            "us" => b_us.add_css_class("cc-pill-btn-active"),
            _ => b_latam.add_css_class("cc-pill-btn-active"),
        }
    });

    refresh();

    let apply_layout = |layout_code: &'static str, refresh_fn: Rc<dyn Fn()>| {
        move |_: &Button| {
            let mut cfg = malus_de_common::config::InputConfig::load();
            cfg.keyboard_layout = layout_code.to_string();
            cfg.keyboard_variant = String::new();
            let _ = cfg.save();
            let _ = malus_de_common::ipc::IpcClient::send_command(
                malus_de_common::ipc::IpcMessage::SetKeyboardLayout {
                    layout: layout_code.to_string(),
                    variant: String::new(),
                },
            );
            refresh_fn();
        }
    };

    btn_latam.connect_clicked(apply_layout("latam", refresh.clone()));
    btn_es.connect_clicked(apply_layout("es", refresh.clone()));
    btn_us.connect_clicked(apply_layout("us", refresh.clone()));

    (card, refresh)
}

// ==========================================
// Subvista 4: Disposición de Pantallas, Escalado Individual y Enlace 1:1 Pixel a Pixel
// ==========================================

#[allow(clippy::too_many_arguments)]
fn create_displays_subview(
    stack: &Stack,
    popover: &Window,
    main_brightness_scale: &Scale,
    main_disp_pct: &Label,
    is_updating_brightness: &Rc<RefCell<bool>>,
    main_dark_btn: &Button,
    main_monitors_info: &Label,
    main_link_btn: &Button,
    main_link_lbl: &Label,
    main_master_slider: &Scale,
    main_master_val_lbl: &Label,
    is_updating_master_scale: &Rc<RefCell<bool>>,
) -> (GtkBox, Rc<dyn Fn()>) {
    let subview = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-subview", "cc-displays-subview"])
        .spacing(7)
        .build();

    // 1. Cabecera limpia estilo macOS Tahoe
    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-subview-header"])
        .spacing(6)
        .build();

    let back_btn = Button::builder()
        .css_classes(["cc-back-btn"])
        .has_frame(false)
        .tooltip_text("Volver al Centro de Control")
        .build();
    back_btn.set_child(Some(&Image::from_icon_name("go-previous-symbolic")));
    {
        let stack_back = stack.clone();
        back_btn.connect_clicked(move |_| {
            stack_back.set_visible_child_name("main");
        });
    }

    let title_lbl = Label::builder()
        .label("Pantalla")
        .css_classes(["cc-subview-title"])
        .hexpand(true)
        .xalign(0.0)
        .build();

    let count_lbl = Label::builder()
        .label("1 pantalla")
        .css_classes(["cc-display-value"])
        .valign(Align::Center)
        .build();

    header.append(&back_btn);
    header.append(&title_lbl);
    header.append(&count_lbl);
    subview.append(&header);

    // 2. Contenido desplazable acotado al ancho exacto del Centro de Control (sin saltos horizontales)
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .max_content_height(430)
        .propagate_natural_height(true)
        .build();

    let content = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(7)
        .build();

    // --- BLOQUE 1: BRILLO ESTILO APPLE ---
    let bright_section = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-display-section"])
        .spacing(6)
        .build();

    let bright_hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    bright_hdr.append(
        &Label::builder()
            .label("Brillo")
            .css_classes(["cc-display-label"])
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    let sub_bright_pct = Label::builder()
        .label("80%")
        .css_classes(["cc-display-value"])
        .build();
    bright_hdr.append(&sub_bright_pct);

    let bright_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .valign(Align::Center)
        .build();
    bright_row.append(
        &Image::builder()
            .icon_name("display-brightness-low-symbolic")
            .pixel_size(14)
            .css_classes(["cc-glass-slider-icon"])
            .build(),
    );
    let sub_bright_scale = Scale::with_range(Orientation::Horizontal, 5.0, 100.0, 1.0);
    disable_scale_scroll(&sub_bright_scale);
    sub_bright_scale.set_draw_value(false);
    sub_bright_scale.set_hexpand(true);
    sub_bright_scale.add_css_class("cc-tahoe-slider");
    bright_row.append(&sub_bright_scale);
    bright_row.append(
        &Image::builder()
            .icon_name("display-brightness-high-symbolic")
            .pixel_size(16)
            .css_classes(["cc-glass-slider-icon"])
            .build(),
    );

    bright_section.append(&bright_hdr);
    bright_section.append(&bright_row);
    content.append(&bright_section);

    // --- BLOQUE 2: MODOS RÁPIDOS ESTILO MACOS (Modo oscuro · Night Shift · TrueSize 1:1) ---
    let make_quick_tile = |icon_name: &str, caption: &str, tooltip: &str| -> (Button, Label) {
        let btn = Button::builder()
            .css_classes(["cc-display-quick-tile"])
            .has_frame(false)
            .tooltip_text(tooltip)
            .build();
        let vbox = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(5)
            .halign(Align::Center)
            .valign(Align::Center)
            .build();
        let circle = GtkBox::builder()
            .css_classes(["cc-display-quick-circle"])
            .halign(Align::Center)
            .valign(Align::Center)
            .build();
        circle.append(
            &Image::builder()
                .icon_name(icon_name)
                .pixel_size(15)
                .halign(Align::Center)
                .valign(Align::Center)
                .hexpand(true)
                .vexpand(true)
                .build(),
        );
        let lbl = Label::builder()
            .label(caption)
            .css_classes(["cc-display-quick-caption"])
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        vbox.append(&circle);
        vbox.append(&lbl);
        btn.set_child(Some(&vbox));
        (btn, lbl)
    };

    let quick_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .homogeneous(true)
        .build();

    let (dark_tile_btn, _) = make_quick_tile(
        "dark-mode-symbolic",
        "Modo oscuro",
        "Alternar entre aspecto claro y oscuro",
    );
    let (night_tile_btn, _) = make_quick_tile(
        "night-light-symbolic",
        "Night Shift",
        "Activar o desactivar tono cálido nocturno",
    );
    let (truesize_tile_btn, _) = make_quick_tile(
        "view-dual-symbolic",
        "TrueSize 1:1",
        "Enlazar escala física por densidad PPI entre ambos monitores",
    );
    quick_row.append(&dark_tile_btn);
    quick_row.append(&night_tile_btn);
    quick_row.append(&truesize_tile_btn);
    content.append(&quick_row);

    // --- BLOQUE 3: DISPOSICIÓN DE PANTALLAS (solo si hay ≥2 monitores) ---
    let arrange_section = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-display-section"])
        .spacing(8)
        .build();

    let arrange_hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    arrange_hdr.append(
        &Label::builder()
            .label("DISPOSICIÓN")
            .css_classes(["cc-section-label"])
            .hexpand(true)
            .xalign(0.0)
            .valign(Align::Center)
            .build(),
    );
    let swap_order_btn = Button::builder()
        .css_classes(["cc-display-inline-btn"])
        .has_frame(false)
        .tooltip_text("Intercambiar monitor izquierdo y derecho")
        .build();
    {
        let r = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .build();
        r.append(
            &Image::builder()
                .icon_name("object-flip-horizontal-symbolic")
                .pixel_size(12)
                .build(),
        );
        r.append(&Label::new(Some("Invertir")));
        swap_order_btn.set_child(Some(&r));
    }
    arrange_hdr.append(&swap_order_btn);
    arrange_section.append(&arrange_hdr);

    let make_monitor_chip = || -> (Button, Label, Label, Label) {
        let btn = Button::builder()
            .css_classes(["cc-display-chip"])
            .has_frame(false)
            .tooltip_text("Clic para establecer como pantalla principal")
            .build();
        let vbox = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .halign(Align::Fill)
            .build();
        let top = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(5)
            .build();
        top.append(
            &Image::builder()
                .icon_name("video-display-symbolic")
                .pixel_size(14)
                .css_classes(["cc-glass-slider-icon"])
                .build(),
        );
        let name_lbl = Label::builder()
            .css_classes(["cc-display-label"])
            .hexpand(true)
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        top.append(&name_lbl);
        let res_lbl = Label::builder()
            .css_classes(["cc-display-value"])
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        let badge_lbl = Label::builder()
            .css_classes(["cc-display-chip-badge"])
            .xalign(0.0)
            .build();
        vbox.append(&top);
        vbox.append(&res_lbl);
        vbox.append(&badge_lbl);
        btn.set_child(Some(&vbox));
        (btn, name_lbl, res_lbl, badge_lbl)
    };

    let chips_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .homogeneous(true)
        .build();
    let (left_chip_btn, left_chip_name, left_chip_res, left_chip_badge) = make_monitor_chip();
    let (right_chip_btn, right_chip_name, right_chip_res, right_chip_badge) = make_monitor_chip();
    chips_row.append(&left_chip_btn);
    chips_row.append(&right_chip_btn);
    arrange_section.append(&chips_row);

    // Control segmentado de alineación vertical (Arriba · Centro · Abajo · Escritorio)
    let align_seg = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-segmented-group"])
        .spacing(2)
        .homogeneous(true)
        .build();
    let align_top_btn = Button::builder()
        .label("Arriba")
        .css_classes(["cc-segmented-btn"])
        .has_frame(false)
        .tooltip_text("Alinear bordes superiores")
        .build();
    let align_center_btn = Button::builder()
        .label("Centro")
        .css_classes(["cc-segmented-btn"])
        .has_frame(false)
        .tooltip_text("Centrar verticalmente")
        .build();
    let align_bottom_btn = Button::builder()
        .label("Abajo")
        .css_classes(["cc-segmented-btn"])
        .has_frame(false)
        .tooltip_text("Alinear bordes inferiores")
        .build();
    let align_desk_btn = Button::builder()
        .label("Mesa")
        .css_classes(["cc-segmented-btn"])
        .has_frame(false)
        .tooltip_text("Alineación física de escritorio")
        .build();
    align_seg.append(&align_top_btn);
    align_seg.append(&align_center_btn);
    align_seg.append(&align_bottom_btn);
    align_seg.append(&align_desk_btn);
    arrange_section.append(&align_seg);

    // Ajuste fino vertical compacto
    let fine_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    let offset_lbl = Label::builder()
        .label("Desplazamiento vertical: 0 px")
        .css_classes(["cc-display-value"])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let nudge_up_btn = Button::builder()
        .label("−50")
        .css_classes(["cc-display-inline-btn"])
        .has_frame(false)
        .tooltip_text("Subir segundo monitor 50 px")
        .build();
    let nudge_down_btn = Button::builder()
        .label("+50")
        .css_classes(["cc-display-inline-btn"])
        .has_frame(false)
        .tooltip_text("Bajar segundo monitor 50 px")
        .build();
    fine_row.append(&offset_lbl);
    fine_row.append(&nudge_up_btn);
    fine_row.append(&nudge_down_btn);
    arrange_section.append(&fine_row);
    content.append(&arrange_section);

    // --- BLOQUE 4: ESCALADO DE PANTALLAS ---
    let scale_section = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["cc-display-section"])
        .spacing(8)
        .build();

    let scale_hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    scale_hdr.append(
        &Label::builder()
            .label("ESCALADO")
            .css_classes(["cc-section-label"])
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    let link_status_lbl = Label::builder()
        .label("TrueSize 1:1")
        .css_classes(["cc-display-chip-badge"])
        .build();
    scale_hdr.append(&link_status_lbl);
    scale_section.append(&scale_hdr);

    // Selector segmentado de presets enlazados (visible en multi-monitor)
    let linked_presets_seg = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-segmented-group"])
        .spacing(2)
        .homogeneous(true)
        .build();
    let link_p1_btn = Button::builder()
        .label("Óptimo · 171 / 150%")
        .css_classes(["cc-segmented-btn"])
        .has_frame(false)
        .tooltip_text("Texto nítido 1:1 (171% y 150%)")
        .build();
    let link_p2_btn = Button::builder()
        .label("Espacio · 152 / 133%")
        .css_classes(["cc-segmented-btn"])
        .has_frame(false)
        .tooltip_text("Más espacio 1:1 (152.5% y 133.3%)")
        .build();
    linked_presets_seg.append(&link_p1_btn);
    linked_presets_seg.append(&link_p2_btn);
    scale_section.append(&linked_presets_seg);

    // Monitor 1
    let mon0_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(5)
        .build();
    let mon0_hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    let mon0_title = Label::builder()
        .label("Pantalla 1")
        .css_classes(["cc-display-label"])
        .hexpand(true)
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    let mon0_val_lbl = Label::builder()
        .label("171% · 2246×1263")
        .css_classes(["cc-display-value"])
        .build();
    mon0_hdr.append(&mon0_title);
    mon0_hdr.append(&mon0_val_lbl);

    let mon0_slider = Scale::with_range(Orientation::Horizontal, 100.0, 250.0, 5.0);
    disable_scale_scroll(&mon0_slider);
    mon0_slider.set_draw_value(false);
    mon0_slider.add_css_class("cc-tahoe-slider");

    let mon0_presets = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-segmented-group"])
        .spacing(2)
        .homogeneous(true)
        .build();
    let preset_scales_m0: [(&str, f64); 5] = [
        ("100%", 1.0),
        ("125%", 1.25),
        ("152%", 1.525),
        ("171%", 1.71),
        ("200%", 2.0),
    ];
    let mut mon0_preset_btns: Vec<(Button, f64)> = Vec::new();
    for &(pct_label, val) in &preset_scales_m0 {
        let p_btn = Button::builder()
            .css_classes(["cc-segmented-btn"])
            .label(pct_label)
            .has_frame(false)
            .build();
        mon0_presets.append(&p_btn);
        mon0_preset_btns.push((p_btn, val));
    }
    mon0_box.append(&mon0_hdr);
    mon0_box.append(&mon0_slider);
    mon0_box.append(&mon0_presets);
    scale_section.append(&mon0_box);

    // Separador + Monitor 2
    let mon1_sep = Separator::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-display-sep"])
        .build();
    scale_section.append(&mon1_sep);

    let mon1_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(5)
        .build();
    let mon1_hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    let mon1_title = Label::builder()
        .label("Pantalla 2")
        .css_classes(["cc-display-label"])
        .hexpand(true)
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    let mon1_val_lbl = Label::builder()
        .label("150% · 2560×1440")
        .css_classes(["cc-display-value"])
        .build();
    mon1_hdr.append(&mon1_title);
    mon1_hdr.append(&mon1_val_lbl);

    let mon1_slider = Scale::with_range(Orientation::Horizontal, 100.0, 250.0, 5.0);
    disable_scale_scroll(&mon1_slider);
    mon1_slider.set_draw_value(false);
    mon1_slider.add_css_class("cc-tahoe-slider");

    let mon1_presets = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["cc-segmented-group"])
        .spacing(2)
        .homogeneous(true)
        .build();
    let preset_scales_m1: [(&str, f64); 5] = [
        ("100%", 1.0),
        ("125%", 1.25),
        ("133%", 1.333333),
        ("150%", 1.5),
        ("200%", 2.0),
    ];
    let mut mon1_preset_btns: Vec<(Button, f64)> = Vec::new();
    for &(pct_label, val) in &preset_scales_m1 {
        let p_btn = Button::builder()
            .css_classes(["cc-segmented-btn"])
            .label(pct_label)
            .has_frame(false)
            .build();
        mon1_presets.append(&p_btn);
        mon1_preset_btns.push((p_btn, val));
    }
    mon1_box.append(&mon1_hdr);
    mon1_box.append(&mon1_slider);
    mon1_box.append(&mon1_presets);
    scale_section.append(&mon1_box);

    content.append(&scale_section);
    scrolled.set_child(Some(&content));
    subview.append(&scrolled);

    // 3. Pie estilo macOS: acceso directo a Ajustes > Pantallas
    subview.append(
        &Separator::builder()
            .orientation(Orientation::Horizontal)
            .css_classes(["cc-display-sep"])
            .build(),
    );
    let open_settings_btn = Button::builder()
        .label("Configuración de pantallas...")
        .css_classes(["cc-footer-btn"])
        .has_frame(false)
        .halign(Align::Fill)
        .valign(Align::Center)
        .vexpand(false)
        .build();
    {
        let pop_c = popover.clone();
        open_settings_btn.connect_clicked(move |_| {
            pop_c.set_visible(false);
            let mut cmd = Command::new("malus-de-settings");
            cmd.args(["--panel", "displays"]);
            malus_de_common::config::set_cursor_environment(&mut cmd);
            let _ = cmd.spawn();
        });
    }
    subview.append(&open_settings_btn);

    // Estado interno
    let is_syncing = Rc::new(RefCell::new(false));
    let night_shift_active = Rc::new(Cell::new(false));
    let left_output_idx = Rc::new(Cell::new(0usize));
    let right_output_idx = Rc::new(Cell::new(1usize));

    let refresh_ui: Rc<dyn Fn()> = {
        let count_c = count_lbl.clone();
        let sb_scale = sub_bright_scale.clone();
        let sb_pct = sub_bright_pct.clone();
        let dark_btn_c = dark_tile_btn.clone();
        let night_btn_c = night_tile_btn.clone();
        let night_flag_c = night_shift_active.clone();
        let ts_btn_c = truesize_tile_btn.clone();
        let arr_sec = arrange_section.clone();
        let l_chip = left_chip_btn.clone();
        let l_name = left_chip_name.clone();
        let l_res = left_chip_res.clone();
        let l_badge = left_chip_badge.clone();
        let r_chip = right_chip_btn.clone();
        let r_name = right_chip_name.clone();
        let r_res = right_chip_res.clone();
        let r_badge = right_chip_badge.clone();
        let l_idx_cell = left_output_idx.clone();
        let r_idx_cell = right_output_idx.clone();
        let off_lbl = offset_lbl.clone();
        let a_top = align_top_btn.clone();
        let a_ctr = align_center_btn.clone();
        let a_bot = align_bottom_btn.clone();
        let a_dsk = align_desk_btn.clone();
        let lk_status = link_status_lbl.clone();
        let lk_seg = linked_presets_seg.clone();
        let l_p1 = link_p1_btn.clone();
        let l_p2 = link_p2_btn.clone();
        let m0_t = mon0_title.clone();
        let m0_v = mon0_val_lbl.clone();
        let m0_s = mon0_slider.clone();
        let m0_btns = mon0_preset_btns.clone();
        let m1_sep_c = mon1_sep.clone();
        let m1_box_c = mon1_box.clone();
        let m1_t = mon1_title.clone();
        let m1_v = mon1_val_lbl.clone();
        let m1_s = mon1_slider.clone();
        let m1_btns = mon1_preset_btns.clone();
        let main_info_c = main_monitors_info.clone();
        let main_l_btn_c = main_link_btn.clone();
        let main_l_lbl_c = main_link_lbl.clone();
        let main_ms_sld_c = main_master_slider.clone();
        let main_ms_val_c = main_master_val_lbl.clone();
        let is_upd_ms_c = is_updating_master_scale.clone();
        let sync_lock = is_syncing.clone();

        Rc::new(move || {
            let cfg = malus_de_common::config::DisplaysConfig::load();
            *sync_lock.borrow_mut() = true;

            let n_outs = cfg.outputs.len().max(1);
            let has_multi = cfg.outputs.len() > 1;
            count_c.set_label(&if n_outs == 1 {
                "1 pantalla".to_string()
            } else {
                format!("{n_outs} pantallas")
            });

            // Brillo actual
            let b_val = cfg.brightness.clamp(5.0, 100.0);
            sb_scale.set_value(b_val);
            sb_pct.set_label(&format!("{}%", b_val.round() as i32));

            // Estado visual de Modo Oscuro
            if malus_de_common::config::is_dark_mode_active() {
                dark_btn_c.add_css_class("cc-quick-active");
            } else {
                dark_btn_c.remove_css_class("cc-quick-active");
            }

            // Estado visual de Night Shift (consulta asíncrona sin bloquear UI)
            {
                let nb = night_btn_c.clone();
                let nf = night_flag_c.clone();
                glib::MainContext::default().spawn_local(async move {
                    let enabled = BrightnessService::get_night_shift().await;
                    nf.set(enabled);
                    if enabled {
                        nb.add_css_class("cc-quick-warm");
                    } else {
                        nb.remove_css_class("cc-quick-warm");
                    }
                });
            }

            ts_btn_c.set_visible(has_multi);
            arr_sec.set_visible(has_multi);
            lk_seg.set_visible(has_multi);
            m1_sep_c.set_visible(has_multi);
            m1_box_c.set_visible(has_multi);

            // Disposición visual izquierda / derecha
            let mut sorted: Vec<_> = cfg.outputs.iter().enumerate().collect();
            sorted.sort_by_key(|(_, o)| o.x);

            if let Some(&(idx0, left_o)) = sorted.first() {
                l_idx_cell.set(idx0);
                let (lw, lh) = left_o.logical_size();
                let display_name = if left_o.name.trim().is_empty() {
                    left_o.connector.clone()
                } else {
                    left_o.name.clone()
                };
                l_name.set_label(&display_name);
                l_res.set_label(&format!("{}×{} · {:.0}%", lw, lh, left_o.scale * 100.0));
                if left_o.primary {
                    l_chip.add_css_class("cc-display-chip-primary");
                    l_badge.set_label("Principal · Izquierda");
                } else {
                    l_chip.remove_css_class("cc-display-chip-primary");
                    l_badge.set_label("Izquierda");
                }
            }

            if let Some(&(idx1, right_o)) = sorted.get(1) {
                r_idx_cell.set(idx1);
                let (rw, rh) = right_o.logical_size();
                let display_name = if right_o.name.trim().is_empty() {
                    right_o.connector.clone()
                } else {
                    right_o.name.clone()
                };
                r_name.set_label(&display_name);
                r_res.set_label(&format!("{}×{} · {:.0}%", rw, rh, right_o.scale * 100.0));
                if right_o.primary {
                    r_chip.add_css_class("cc-display-chip-primary");
                    r_badge.set_label("Principal · Derecha");
                } else {
                    r_chip.remove_css_class("cc-display-chip-primary");
                    r_badge.set_label("Derecha");
                }

                // Desplazamiento vertical y resaltado de alineación activa
                let dy = right_o.y - sorted.first().map(|(_, o)| o.y).unwrap_or(0);
                off_lbl.set_label(&format!("Desplazamiento vertical: {dy:+} px"));
                let (_, lh) = sorted.first().map(|(_, o)| o.logical_size()).unwrap_or((0, 0));
                let diff_h = lh - rh;
                for b in [&a_top, &a_ctr, &a_bot, &a_dsk] {
                    b.remove_css_class("cc-segmented-btn-active");
                }
                if dy == 0 {
                    a_top.add_css_class("cc-segmented-btn-active");
                } else if (dy - diff_h / 2).abs() <= 4 {
                    a_ctr.add_css_class("cc-segmented-btn-active");
                } else if (dy - diff_h).abs() <= 4 {
                    a_bot.add_css_class("cc-segmented-btn-active");
                } else if (dy - 519).abs() <= 8 {
                    a_dsk.add_css_class("cc-segmented-btn-active");
                }
            }

            // Estado TrueSize 1:1
            if cfg.linked_physical_1to1 {
                ts_btn_c.add_css_class("cc-quick-active");
                lk_status.set_label("TrueSize 1:1");
                lk_status.set_visible(has_multi);
                main_l_btn_c.add_css_class("cc-pill-btn-active");
                main_l_lbl_c.set_label("TrueSize 1:1");
            } else {
                ts_btn_c.remove_css_class("cc-quick-active");
                lk_status.set_label("Independiente");
                lk_status.set_visible(has_multi);
                main_l_btn_c.remove_css_class("cc-pill-btn-active");
                main_l_lbl_c.set_label("Escala libre");
            }

            let curr_preset = cfg.current_linked_preset();
            if curr_preset == Some(1) {
                l_p1.add_css_class("cc-segmented-btn-active");
                l_p2.remove_css_class("cc-segmented-btn-active");
            } else if curr_preset == Some(2) {
                l_p2.add_css_class("cc-segmented-btn-active");
                l_p1.remove_css_class("cc-segmented-btn-active");
            } else {
                l_p1.remove_css_class("cc-segmented-btn-active");
                l_p2.remove_css_class("cc-segmented-btn-active");
            }

            // Monitor 1
            if let Some(o0) = cfg.outputs.first() {
                let (w0, h0) = o0.logical_size();
                let name = if o0.name.trim().is_empty() {
                    o0.connector.clone()
                } else {
                    o0.name.clone()
                };
                m0_t.set_label(&name);
                m0_v.set_label(&format!("{:.0}% · {}×{}", o0.scale * 100.0, w0, h0));
                m0_s.set_value((o0.scale * 100.0).clamp(100.0, 250.0));
                for (btn, target) in &m0_btns {
                    if (o0.scale - *target).abs() < 0.02 {
                        btn.add_css_class("cc-segmented-btn-active");
                    } else {
                        btn.remove_css_class("cc-segmented-btn-active");
                    }
                }
            }

            // Monitor 2
            if let Some(o1) = cfg.outputs.get(1) {
                let (w1, h1) = o1.logical_size();
                let name = if o1.name.trim().is_empty() {
                    o1.connector.clone()
                } else {
                    o1.name.clone()
                };
                m1_t.set_label(&name);
                m1_v.set_label(&format!("{:.0}% · {}×{}", o1.scale * 100.0, w1, h1));
                m1_s.set_value((o1.scale * 100.0).clamp(100.0, 250.0));
                for (btn, target) in &m1_btns {
                    if (o1.scale - *target).abs() < 0.02 {
                        btn.add_css_class("cc-segmented-btn-active");
                    } else {
                        btn.remove_css_class("cc-segmented-btn-active");
                    }
                }
            }

            // Sincronizar tarjeta principal
            *is_upd_ms_c.borrow_mut() = true;
            main_ms_sld_c.set_value((cfg.master_scale * 100.0).clamp(100.0, 250.0));
            *is_upd_ms_c.borrow_mut() = false;
            main_ms_val_c.set_label(&format!("{:.1}%", cfg.master_scale * 100.0));
            main_info_c.set_label(&format_monitors_summary(&cfg));

            *sync_lock.borrow_mut() = false;
        })
    };

    refresh_ui();

    // Sincronizar brillo desde la subvista hacia el sistema y el slider principal
    {
        let sync_b = is_syncing.clone();
        let main_b_lock = is_updating_brightness.clone();
        let main_b_scale = main_brightness_scale.clone();
        let main_b_lbl = main_disp_pct.clone();
        let sub_b_lbl = sub_bright_pct;
        sub_bright_scale.connect_value_changed(move |sc| {
            if *sync_b.borrow() {
                return;
            }
            let val = sc.value().clamp(5.0, 100.0);
            let txt = format!("{}%", val.round() as i32);
            sub_b_lbl.set_label(&txt);
            *main_b_lock.borrow_mut() = true;
            main_b_scale.set_value(val);
            main_b_lbl.set_label(&txt);
            *main_b_lock.borrow_mut() = false;
            BrightnessService::set_brightness(val);
        });
    }

    // Modo oscuro rápido
    {
        let main_dark_c = main_dark_btn.clone();
        let rf_c = refresh_ui.clone();
        dark_tile_btn.connect_clicked(move |_| {
            main_dark_c.emit_clicked();
            let rf_later = rf_c.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || {
                rf_later();
            });
        });
    }

    // Night Shift rápido
    {
        let nf = night_shift_active;
        let nb = night_tile_btn.clone();
        night_tile_btn.connect_clicked(move |_| {
            let next = !nf.get();
            nf.set(next);
            BrightnessService::set_night_shift(next);
            if next {
                nb.add_css_class("cc-quick-warm");
            } else {
                nb.remove_css_class("cc-quick-warm");
            }
        });
    }

    let ref_map = refresh_ui.clone();
    subview.connect_map(move |_| {
        ref_map();
    });
    if let Some(display) = gtk4::gdk::Display::default() {
        let ref_mon = refresh_ui.clone();
        display.monitors().connect_items_changed(move |_, _, _, _| {
            let ref_later = ref_mon.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
                ref_later();
            });
        });
    }
    if let Ok(home) = std::env::var("HOME") {
        let toml_path = std::path::PathBuf::from(home).join(".config/malus-de/monitors.toml");
        if let Ok(file_mon) = gtk4::gio::File::for_path(toml_path).monitor_file(
            gtk4::gio::FileMonitorFlags::NONE,
            gtk4::gio::Cancellable::NONE,
        ) {
            let ref_file = refresh_ui.clone();
            file_mon.connect_changed(move |_, _, _, ev| {
                if matches!(
                    ev,
                    gtk4::gio::FileMonitorEvent::Changed
                        | gtk4::gio::FileMonitorEvent::Created
                        | gtk4::gio::FileMonitorEvent::ChangesDoneHint
                ) {
                    ref_file();
                }
            });
            Box::leak(Box::new(file_mon));
        }
    }

    let apply_and_notify = |cfg: malus_de_common::config::DisplaysConfig, ref_fn: &Rc<dyn Fn()>| {
        let _ = cfg.save();
        let _ = malus_de_common::ipc::IpcClient::send_command(
            malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig,
        );
        ref_fn();
    };

    // Clic en tarjeta de monitor izquierdo o derecho -> marcar como principal
    {
        let ref_c = refresh_ui.clone();
        let idx_c = left_output_idx;
        left_chip_btn.connect_clicked(move |_| {
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            let target = idx_c.get();
            if target < cfg.outputs.len() {
                cfg.set_primary_output(target);
                apply_and_notify(cfg, &ref_c);
            }
        });
    }
    {
        let ref_c = refresh_ui.clone();
        let idx_c = right_output_idx;
        right_chip_btn.connect_clicked(move |_| {
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            let target = idx_c.get();
            if target < cfg.outputs.len() {
                cfg.set_primary_output(target);
                apply_and_notify(cfg, &ref_c);
            }
        });
    }

    // Invertir orden izquierda <-> derecha
    {
        let ref_swap = refresh_ui.clone();
        swap_order_btn.connect_clicked(move |_| {
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            cfg.swap_horizontal_order();
            apply_and_notify(cfg, &ref_swap);
        });
    }

    // Alineación vertical
    for (btn, mode) in [
        (&align_top_btn, "top"),
        (&align_center_btn, "center"),
        (&align_bottom_btn, "bottom"),
        (&align_desk_btn, "desk"),
    ] {
        let ref_align = refresh_ui.clone();
        let mode_str = mode.to_string();
        btn.connect_clicked(move |_| {
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            cfg.set_vertical_alignment(&mode_str);
            apply_and_notify(cfg, &ref_align);
        });
    }

    // Ajuste fino vertical (-50 / +50 px)
    {
        let ref_up = refresh_ui.clone();
        nudge_up_btn.connect_clicked(move |_| {
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            let target_idx = if cfg.outputs.len() > 1 { 1 } else { 0 };
            cfg.adjust_vertical_offset(target_idx, -50);
            apply_and_notify(cfg, &ref_up);
        });
    }
    {
        let ref_down = refresh_ui.clone();
        nudge_down_btn.connect_clicked(move |_| {
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            let target_idx = if cfg.outputs.len() > 1 { 1 } else { 0 };
            cfg.adjust_vertical_offset(target_idx, 50);
            apply_and_notify(cfg, &ref_down);
        });
    }

    // TrueSize 1:1 toggle
    {
        let ref_link = refresh_ui.clone();
        truesize_tile_btn.connect_clicked(move |_| {
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            cfg.linked_physical_1to1 = !cfg.linked_physical_1to1;
            if cfg.linked_physical_1to1 {
                if cfg.current_linked_preset().is_none() {
                    cfg.apply_linked_preset(1);
                } else {
                    cfg.apply_1to1_link();
                }
            } else {
                cfg.recalculate_layout();
            }
            apply_and_notify(cfg, &ref_link);
        });
    }

    // Presets enlazados 1 y 2
    {
        let ref_lp1 = refresh_ui.clone();
        link_p1_btn.connect_clicked(move |_| {
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            cfg.apply_linked_preset(1);
            apply_and_notify(cfg, &ref_lp1);
        });
    }
    {
        let ref_lp2 = refresh_ui.clone();
        link_p2_btn.connect_clicked(move |_| {
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            cfg.apply_linked_preset(2);
            apply_and_notify(cfg, &ref_lp2);
        });
    }

    // Monitor 1 slider & presets
    {
        let sync_m0 = is_syncing.clone();
        let ref_m0 = refresh_ui.clone();
        mon0_slider.connect_value_changed(move |sld| {
            if *sync_m0.borrow() {
                return;
            }
            let pct = (sld.value() / 2.5).round() * 2.5;
            let new_scale = (pct / 100.0).clamp(0.5, 3.0);
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            cfg.set_output_scale_independent(0, new_scale);
            apply_and_notify(cfg, &ref_m0);
        });
    }
    for (btn, target_scale) in mon0_preset_btns {
        let ref_p0 = refresh_ui.clone();
        btn.connect_clicked(move |_| {
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            cfg.set_output_scale_independent(0, target_scale);
            apply_and_notify(cfg, &ref_p0);
        });
    }

    // Monitor 2 slider & presets
    {
        let sync_m1 = is_syncing.clone();
        let ref_m1 = refresh_ui.clone();
        mon1_slider.connect_value_changed(move |sld| {
            if *sync_m1.borrow() {
                return;
            }
            let pct = (sld.value() / 2.5).round() * 2.5;
            let new_scale = (pct / 100.0).clamp(0.5, 3.0);
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            cfg.set_output_scale_independent(1, new_scale);
            apply_and_notify(cfg, &ref_m1);
        });
    }
    for (btn, target_scale) in mon1_preset_btns {
        let ref_p1 = refresh_ui.clone();
        btn.connect_clicked(move |_| {
            let mut cfg = malus_de_common::config::DisplaysConfig::load();
            cfg.set_output_scale_independent(1, target_scale);
            apply_and_notify(cfg, &ref_p1);
        });
    }

    (subview, refresh_ui)
}

fn clip_glass_region_to_window(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    win_w: i32,
    win_h: i32,
    radius: f32,
    is_capsule: bool,
) -> Option<GlassRegion> {
    let mut x0 = x.round() as i32;
    let mut y0 = y.round() as i32;
    let mut x1 = (x + w).round() as i32;
    let mut y1 = (y + h).round() as i32;

    if win_w > 0 && win_h > 0 {
        x0 = x0.clamp(0, win_w);
        y0 = y0.clamp(0, win_h);
        x1 = x1.clamp(0, win_w);
        y1 = y1.clamp(0, win_h);
    }

    let rw = x1 - x0;
    let rh = y1 - y0;
    if rw < 8 || rh < 8 {
        return None;
    }

    let r = if is_capsule {
        (rh as f32 * 0.5).min(rw as f32 * 0.5)
    } else {
        radius
    };

    Some(GlassRegion {
        x: x0,
        y: y0,
        width: rw,
        height: rh,
        radius: r,
        is_capsule,
    })
}

fn glass_regions_equal(a: &[GlassRegion], b: &[GlassRegion]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b.iter()).all(|(lhs, rhs)| {
            lhs.x == rhs.x
                && lhs.y == rhs.y
                && lhs.width == rhs.width
                && lhs.height == rhs.height
                && (lhs.radius - rhs.radius).abs() < 0.05
                && lhs.is_capsule == rhs.is_capsule
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_sliding_glass_regions_to_window_and_deduplicates_cleanly() {
        // Región parcialmente deslizada fuera del borde izquierdo durante SlideLeftRight
        let clipped = clip_glass_region_to_window(-40.0, 12.0, 150.0, 70.0, 340, 480, 26.0, false)
            .expect("debe recortar la porción visible");
        assert_eq!(clipped.x, 0);
        assert_eq!(clipped.y, 12);
        assert_eq!(clipped.width, 110);
        assert_eq!(clipped.height, 70);

        // Región completamente fuera de la ventana -> descartada
        assert!(clip_glass_region_to_window(-200.0, 12.0, 150.0, 70.0, 340, 480, 26.0, false).is_none());

        // Deduplicación exacta
        let a = vec![clipped.clone()];
        let b = vec![clipped];
        assert!(glass_regions_equal(&a, &b));
    }
}
