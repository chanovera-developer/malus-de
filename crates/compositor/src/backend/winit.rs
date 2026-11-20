use anyhow::{Context, Result};
use malus_de_common::ipc::IpcMessage;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::{
    ResizeEdge, State as XdgState,
};
use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, Axis, ButtonState, Event, InputEvent, KeyState,
            KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
        },
        renderer::{
            element::{
                surface::{
                    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
                },
                Element, Kind,
            },
            gles::GlesRenderer,
            utils::draw_render_elements,
            Color32F, ExportMem, Frame, ImportDma, Renderer, Texture,
        },
        winit::{self, WinitEvent},
    },
    input::{
        keyboard::{keysyms, FilterResult},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        wayland_server::{protocol::wl_surface::WlSurface, Display, ListeningSocket},
        winit::platform::pump_events::PumpStatus,
    },
    utils::{Logical, Physical, Point, Rectangle, Scale, Transform, SERIAL_COUNTER},
    wayland::{
        compositor::{with_surface_tree_downward, SurfaceAttributes, TraversalAction},
        seat::WaylandFocus,
        shell::wlr_layer::Layer,
    },
};
use std::{sync::Arc, time::Instant};
use tracing::{info, warn};

use crate::state::{ClientState, MalusState};

pub fn send_frames_surface_tree(surface: &WlSurface, time: u32) {
    with_surface_tree_downward(
        surface,
        (),
        |_, _, &()| TraversalAction::DoChildren(()),
        |_surf, states, &()| {
            for callback in states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .frame_callbacks
                .drain(..)
            {
                callback.done(time);
            }
        },
        |_, _, &()| true,
    );
}

fn find_background_layer_under(
    state: &MalusState,
    output: &Output,
    point: Point<f64, Logical>,
) -> Option<(WlSurface, Point<f64, Logical>, String, bool)> {
    let layer_map = smithay::desktop::layer_map_for_output(output);
    let layer = layer_map
        .layer_under(Layer::Background, point)
        .filter(|layer| {
            !(layer.namespace() == "malus-app-switcher" && !state.app_switcher_classic_active())
        })?;
    let geometry = layer_map.layer_geometry(layer)?;
    let local_point = point - geometry.loc.to_f64();
    let hit = if !state.active_popups.is_empty() {
        layer.surface_under(local_point, smithay::desktop::WindowSurfaceType::POPUP)
    } else {
        None
    };
    let (surface, sub_location) = hit.or_else(|| {
        layer.surface_under(local_point, smithay::desktop::WindowSurfaceType::TOPLEVEL)
    })?;

    Some((
        surface,
        (geometry.loc + sub_location).to_f64(),
        layer.namespace().to_string(),
        layer.can_receive_keyboard_focus(),
    ))
}

fn find_layer_popup_under(
    state: &MalusState,
    output: &smithay::output::Output,
    point: Point<f64, Logical>,
) -> Option<(
    smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    Point<f64, Logical>,
)> {
    if state.active_popups.is_empty() {
        return None;
    }
    let layer_map = smithay::desktop::layer_map_for_output(output);
    for layer in layer_map.layers() {
        if let Some(geo) = layer_map.layer_geometry(layer) {
            let local_point = point - geo.loc.to_f64();
            if let Some((surf, sub_loc)) =
                layer.surface_under(local_point, smithay::desktop::WindowSurfaceType::POPUP)
            {
                return Some((
                    surf,
                    (geo.loc + sub_loc).to_f64(),
                ));
            }
        }
    }
    None
}

pub fn run_winit() -> Result<()> {
    info!("Inicializando Display Wayland de Malus DE...");
    let mut display: Display<MalusState> =
        Display::new().context("Error al crear Display Wayland")?;
    let mut state = MalusState::new(&display);

    let input_cfg = malus_de_common::config::InputConfig::load();
    let xkb_cfg = smithay::input::keyboard::XkbConfig {
        rules: "",
        model: &input_cfg.keyboard_model,
        layout: &input_cfg.keyboard_layout,
        variant: &input_cfg.keyboard_variant,
        options: input_cfg.keyboard_options.clone(),
    };
    let keyboard = match state.seat.add_keyboard(xkb_cfg, 200, 200) {
        Ok(kb) => {
            info!(
                "⌨️ Teclado XKB inicializado (winit): layout='{}' ({}), variant='{}'",
                input_cfg.keyboard_layout,
                input_cfg.layout_display_name(),
                input_cfg.keyboard_variant
            );
            kb
        }
        Err(e) => {
            warn!(
                "⚠️ Falló carga de XKB layout '{}': {:?}. Usando fallback US.",
                input_cfg.keyboard_layout, e
            );
            state
                .seat
                .add_keyboard(Default::default(), 200, 200)
                .map_err(|e| anyhow::anyhow!("Error al inicializar teclado: {e:?}"))?
        }
    };
    let pointer = state.seat.add_pointer();

    // Crear socket de escucha de clientes Wayland (wayland-1 .. 32)
    let listener = ListeningSocket::bind_auto("wayland", 1..33)
        .map_err(|e| anyhow::anyhow!("Error al bindear socket de Wayland: {e:?}"))?;
    let socket_name = listener
        .socket_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    info!(
        "Socket Wayland escuchando activamente en: WAYLAND_DISPLAY={}",
        socket_name
    );
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);
    state.socket_name = socket_name.clone();
    let _clipboard_manager = crate::start_clipboard_manager(&socket_name);

    // Inicializar backend gráfico de Winit en ventana anidada
    let (mut backend, mut winit_event_loop) = winit::init::<GlesRenderer>()
        .map_err(|e| anyhow::anyhow!("Error al inicializar WinitGraphicsBackend: {e:?}"))?;

    let dmabuf_formats = backend.renderer().dmabuf_formats();
    if dmabuf_formats.iter().next().is_some() {
        let global = state
            .dmabuf_state
            .create_global::<MalusState>(&state.display_handle, dmabuf_formats);
        state.dmabuf_global = Some(global);
    }

    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };

    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Malus".to_string(),
            model: "Virtual Display (Nested)".to_string(),
        },
    );
    let _global = output.create_global::<MalusState>(&state.display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));

    info!("============================================================");
    info!("✨ MALUS DE: Compositor Wayland Smithay (Modo Anidado Activo)");
    info!("   Socket activo: WAYLAND_DISPLAY={}", socket_name);
    info!("   Prueba lanzar una app cliente en otra terminal:");
    info!(
        "   WAYLAND_DISPLAY={} GDK_BACKEND=wayland gnome-calculator",
        socket_name
    );
    info!("============================================================");

    // Color acrílico oscuro de Malus DE (#18181b = rgba(24, 24, 27, 1.0))
    let dark_acrylic = Color32F::new(0.094, 0.094, 0.106, 1.0);
    let start_time = Instant::now();
    let mut current_displays_cfg = malus_de_common::config::DisplaysConfig::load();
    let mut squircle_corner_cache: std::collections::HashMap<
        i32,
        [smithay::backend::renderer::gles::GlesTexture; 4],
    > = std::collections::HashMap::new();
    let mut blur_pipeline: Option<crate::blur::GaussianBlur> = None;
    let mut window_squircle_shader: Option<crate::window_corners::WindowSquircleShader> = None;
    let mut shadow_pipeline: Option<crate::shadow::WindowShadowPipeline> = None;
    let mut black_hole_pipeline: Option<crate::black_hole::BlackHolePipeline> = None;
    let mut active_black_holes: Vec<crate::black_hole::ActiveBlackHoleClose> = Vec::new();
    let mut genie_pipeline: Option<crate::magic_lamp::GeniePipeline> = None;
    let mut genie_snapshots: std::collections::HashMap<
        u64,
        crate::capture::CapturedWindowOffscreen,
    > = std::collections::HashMap::new();
    let mut theme_transition_pipeline: Option<crate::theme_transition::ThemeTransitionPipeline> =
        None;
    let mut workspace_transition_pipeline: Option<
        crate::workspace_transition::WorkspaceTransitionPipeline,
    > = None;

    while state.is_running {
        let frame_start = Instant::now();
        state.check_session_lock_liveness();
        state.resume_timed_out_dock_animations(frame_start);
        state.tick_snap_layouts(frame_start);
        state.tick_app_switcher_deck(frame_start);
        let _ = state.snap_layouts.take_closed_dirty();
        let _ = state.take_app_switcher_deck_closed_dirty();

        let status = winit_event_loop.dispatch_new_events(|event| match event {
            WinitEvent::CloseRequested => {
                info!("Cierre de ventana anidada solicitado por el usuario.");
                state.is_running = false;
            }
            WinitEvent::Resized { size, .. } => {
                output.change_current_state(
                    Some(Mode { size, refresh: 60_000 }),
                    None,
                    None,
                    None,
                );
            }
            WinitEvent::Input(event) => match event {
                InputEvent::Keyboard { event } => {
                    let key = event.key_code().raw();
                    let is_pressed = event.state() == KeyState::Pressed;

                    keyboard.input::<(), _>(
                        &mut state,
                        event.key_code(),
                        event.state(),
                        SERIAL_COUNTER.next_serial(),
                        event.time_msec(),
                        |state, modifiers, keysym_handle| {
                            let sym = keysym_handle.modified_sym();
                            let is_modifier_sym = sym == keysyms::KEY_Alt_L.into()
                                || sym == keysyms::KEY_Alt_R.into()
                                || sym == keysyms::KEY_Super_L.into()
                                || sym == keysyms::KEY_Super_R.into()
                                || sym == keysyms::KEY_Meta_L.into()
                                || sym == keysyms::KEY_Meta_R.into()
                                || keysym_handle.raw_syms().iter().any(|&s| {
                                    s == keysyms::KEY_Alt_L.into()
                                        || s == keysyms::KEY_Alt_R.into()
                                        || s == keysyms::KEY_Super_L.into()
                                        || s == keysyms::KEY_Super_R.into()
                                        || s == keysyms::KEY_Meta_L.into()
                                        || s == keysyms::KEY_Meta_R.into()
                                });

                            let has_super = modifiers.logo;
                            let has_shift = modifiers.shift;
                            let has_alt = modifiers.alt;
                            let has_ctrl = modifiers.ctrl;

                            let is_space = sym == keysyms::KEY_space.into();
                            let is_k = sym == keysyms::KEY_k.into() || sym == keysyms::KEY_K.into();
                            let is_tab = sym == keysyms::KEY_Tab.into() || sym == keysyms::KEY_ISO_Left_Tab.into() || key == 23;
                            let is_return = sym == keysyms::KEY_Return.into() || sym == keysyms::KEY_KP_Enter.into();
                            let is_w = sym == keysyms::KEY_w.into() || sym == keysyms::KEY_W.into();
                            let is_q = sym == keysyms::KEY_q.into() || sym == keysyms::KEY_Q.into();
                            let is_h = sym == keysyms::KEY_h.into() || sym == keysyms::KEY_H.into();
                            let is_m = sym == keysyms::KEY_m.into() || sym == keysyms::KEY_M.into();
                            let is_f4 = sym == keysyms::KEY_F4.into();
                            let is_f9 = sym == keysyms::KEY_F9.into();
                            let is_esc = sym == keysyms::KEY_Escape.into();
                            let is_left = sym == keysyms::KEY_Left.into() || key == 105 || key == 113;
                            let is_right = sym == keysyms::KEY_Right.into() || key == 106 || key == 114;

                            if is_pressed {
                                if state.is_session_locked() {
                                    return FilterResult::Forward;
                                }
                                let is_print = sym == keysyms::KEY_Print.into() || key == 99;
                                let is_s = sym == keysyms::KEY_s.into() || sym == keysyms::KEY_S.into();
                                let is_3 = sym == keysyms::KEY_3.into() || sym == keysyms::KEY_numbersign.into();
                                let is_4 = sym == keysyms::KEY_4.into() || sym == keysyms::KEY_dollar.into();
                                let is_5 = sym == keysyms::KEY_5.into() || sym == keysyms::KEY_percent.into();

                                if state.screenshot_ui.is_open() {
                                    if state.screenshot_ui.handle_key_press(sym, *modifiers) {
                                        state.execute_screenshot_ui_action();
                                    }
                                    return FilterResult::Intercept(());
                                }

                                if is_esc && state.snap_layouts.handle_escape() {
                                    return FilterResult::Intercept(());
                                }

                                if (is_print && has_super) || (has_super && has_shift && is_5) {
                                    info!("📸 Atajo de barra interactiva de captura detectado (Super+Shift+5, Super+Print)");
                                    state.start_interactive_screenshot();
                                    return FilterResult::Intercept(());
                                }

                                if (is_print && has_shift) || (has_super && has_shift && (is_s || is_4)) {
                                    info!("📸 Atajo de captura de área detectado (Shift+Print, Super+Shift+S, Super+Shift+4)");
                                    state.start_area_screenshot();
                                    return FilterResult::Intercept(());
                                }

                                if (is_print && !has_shift) || (has_super && has_shift && is_3) {
                                    info!("📸 Atajo de captura completa detectado (Print, Super+Shift+3)");
                                    state.start_full_screenshot();
                                    return FilterResult::Intercept(());
                                }

                                // 1. Atajos para alternar Spotlight:
                                // Super + Espacio (macOS nativo), Alt + Espacio, Ctrl + Espacio, Super + K
                                if (is_space && (has_super || has_alt || has_ctrl)) || (is_k && has_super) {
                                    info!("🔍 Atajo de Spotlight detectado (Super={}, Alt={}, Ctrl={}): alternando...",
                                        has_super, has_alt, has_ctrl);
                                    state.toggle_spotlight();
                                    return FilterResult::Intercept(());
                                }

                                if state.spotlight.is_open() {
                                    state.spotlight.handle_key_press(sym, *modifiers);
                                    return FilterResult::Intercept(());
                                }

                                let is_up = sym == keysyms::KEY_Up.into();
                                let is_down = sym == keysyms::KEY_Down.into();
                                if has_ctrl && !has_super && !has_alt && is_up {
                                    state.toggle_expose(crate::expose::ExposeMode::Global);
                                    return FilterResult::Intercept(());
                                }
                                if has_ctrl && !has_super && !has_alt && is_down {
                                    state.toggle_app_expose();
                                    return FilterResult::Intercept(());
                                }

                                if state.handle_expose_key(sym, *modifiers) {
                                    return FilterResult::Intercept(());
                                }

                                if has_ctrl && !has_super && !has_alt && is_left {
                                    state.switch_workspace_delta(-1);
                                    return FilterResult::Intercept(());
                                }
                                if has_ctrl && !has_super && !has_alt && is_right {
                                    state.switch_workspace_delta(1);
                                    return FilterResult::Intercept(());
                                }

                                // 2. Navegación en el App Switcher (Selector Visual ⌘Tab) mientras está activo
                                if state.app_switcher.is_active {
                                    if is_return {
                                        info!("🎯 AppSwitcher commit vía Return/Enter, enfocando ventana");
                                        state.commit_app_switcher();
                                        return FilterResult::Intercept(());
                                    } else if is_tab {
                                        state.step_app_switcher(!modifiers.shift);
                                        info!(
                                            "🔀 AppSwitcher ciclo: idx={}/{}",
                                            state.app_switcher.selected_index,
                                            state.app_switcher.items.len()
                                        );
                                        return FilterResult::Intercept(());
                                    } else if is_left {
                                        state.step_app_switcher(false);
                                        return FilterResult::Intercept(());
                                    } else if is_right {
                                        state.step_app_switcher(true);
                                        return FilterResult::Intercept(());
                                    } else if is_q {
                                        state.close_selected_app_switcher_window();
                                        return FilterResult::Intercept(());
                                    } else if is_h {
                                        state.hide_selected_app_switcher_window();
                                        return FilterResult::Intercept(());
                                    } else if is_esc {
                                        info!("❌ AppSwitcher cancelado con Escape");
                                        state.dismiss_app_switcher();
                                        return FilterResult::Intercept(());
                                    }
                                }

                                // 3. Invocación inicial de ⌘Tab / Super+Tab / Alt+Tab
                                if (has_super || has_alt) && is_tab {
                                    let pointer_loc =
                                        state.seat.get_pointer().map(|p| p.current_location());
                                    state.open_app_switcher(modifiers.shift, pointer_loc);
                                    if state.app_switcher.is_active {
                                        info!(
                                            "🔀 AppSwitcher activado con {} ventanas (idx={})",
                                            state.app_switcher.items.len(),
                                            state.app_switcher.selected_index
                                        );
                                        return FilterResult::Intercept(());
                                    }
                                }

                                // Bloqueo de pantalla (Super+L, y Ctrl+Super+Q estilo macOS)
                                let is_l = sym == keysyms::KEY_l.into() || sym == keysyms::KEY_L.into();
                                if (has_super && !has_alt && is_l) || (has_super && has_ctrl && !has_alt && is_q) {
                                    info!("🔒 Bloqueando pantalla mediante atajo de teclado");
                                    state.lock_session();
                                    return FilterResult::Intercept(());
                                }

                                // 4. Atajos universales de macOS: ⌘Q / ⌘W / Alt+F4 para cerrar ventana activa
                                if (has_super && (is_w || is_q)) || (has_alt && is_f4) {
                                    info!("🚪 Atajo macOS ⌘Q / ⌘W / Alt+F4: cerrando ventana activa");
                                    state.close_active_window();
                                    return FilterResult::Intercept(());
                                }

                                // 5. Atajos universales de macOS: ⌘H / ⌘M / Alt+F9 para ocultar / minimizar con Genie Effect
                                if (has_super && (is_m || is_h)) || (has_alt && is_f9) {
                                    info!("🪟 Atajo macOS ⌘H / ⌘M / Alt+F9: minimizando ventana activa vía Genie Effect");
                                    state.minimize_active_window();
                                    return FilterResult::Intercept(());
                                }

                                // 6. Atajos de Aero Snap / Tiling (Super o Alt + Flechas)
                                if has_super || has_alt {
                                    let is_up = sym == keysyms::KEY_Up.into() || key == 103 || key == 111;
                                    let is_down = sym == keysyms::KEY_Down.into() || key == 108 || key == 116;

                                    if is_left || is_right || is_up || is_down {
                                        let active_window = state.get_active_window();
                                        let snap_info = active_window.and_then(|win| {
                                            state.space.outputs().next().map(|output| {
                                                let layer_map = smithay::desktop::layer_map_for_output(output);
                                                let zone = layer_map.non_exclusive_zone();
                                                let output_geo = state.space.output_geometry(output).unwrap_or_default();
                                                let usable_zone = if zone.size.w > 0 && zone.size.h > 0 {
                                                    let has_dock = layer_map
                                                        .layers()
                                                        .any(|layer| layer.namespace() == "malus-dock");
                                                    let headroom = if has_dock {
                                                        state.dock_blur_headroom.max(0)
                                                    } else {
                                                        0
                                                    };
                                                    let usable_height = (zone.size.h + headroom)
                                                        .min(output_geo.size.h - zone.loc.y);
                                                    Rectangle::new(
                                                        zone.loc,
                                                        (
                                                            zone.size.w,
                                                            usable_height.max(zone.size.h),
                                                        )
                                                            .into(),
                                                    )
                                                } else {
                                                    Rectangle::new(
                                                        (0, 32).into(),
                                                        (output_geo.size.w, output_geo.size.h - 32).into(),
                                                    )
                                                };

                                                let snap_mode = if is_left {
                                                    crate::state::SnapMode::LeftHalf
                                                } else if is_right {
                                                    crate::state::SnapMode::RightHalf
                                                } else if is_up {
                                                    crate::state::SnapMode::Maximize
                                                } else {
                                                    crate::state::SnapMode::CenterRestore
                                                };

                                                (win, snap_mode, usable_zone)
                                            })
                                        });

                                        if let Some((win, snap_mode, usable_zone)) = snap_info {
                                            info!("🪟 Atajo Aero Snap activado: modo={:?}, ventana activa encontrada", snap_mode);
                                            crate::state::snap_window(state, &win, snap_mode, usable_zone);
                                            return FilterResult::Intercept(());
                                        }
                                    }
                                }
                            } else {
                                // 7. Liberación de teclas (Key Release)
                                // Cuando el usuario suelta Super o Alt, confirmar selección en App Switcher
                                if state.app_switcher.is_active && ((!modifiers.logo && !modifiers.alt) || is_modifier_sym) {
                                    info!("🎯 AppSwitcher commit: soltado Super/Alt (is_mod={}), enfocando ventana", is_modifier_sym);
                                    state.commit_app_switcher();
                                    return FilterResult::Intercept(());
                                }
                            }
                            FilterResult::Forward
                        },
                    );
                }
                InputEvent::PointerMotionAbsolute { event } => {
                    let output_geo = state.space.output_geometry(&output).unwrap_or_default();
                    let point = event.position_transformed(output_geo.size);
                    state.last_pointer_location = Some(point);
                    let serial = SERIAL_COUNTER.next_serial();
                    state.spotlight.update_dock_drag_location(point);

                    if state.spotlight.is_open() {
                        state.spotlight.update_pointer_motion(output_geo, point);
                    }
                    if state.screenshot_ui.is_open() {
                        state.screenshot_ui.update_pointer_motion(output_geo, point);
                    }
                    state.update_spotlight_drag_dock_hover(point);
                    let _ = state.handle_app_switcher_deck_pointer_motion(point);

                    if state.handle_expose_pointer_motion(point) {
                        let serial = SERIAL_COUNTER.next_serial();
                        pointer.motion(
                            &mut state,
                            None,
                            &MotionEvent {
                                location: point,
                                serial,
                                time: event.time_msec(),
                            },
                        );
                        pointer.frame(&mut state);
                        return;
                    }

                    let has_fullscreen = state.has_fullscreen_window_on_output(&output);
                    let layer_target = {
                        let layer_map = smithay::desktop::layer_map_for_output(&output);
                        let under_layer = layer_map.layer_under(Layer::Overlay, point)
                            .filter(|l| !(l.namespace() == "malus-app-switcher" && !state.app_switcher.is_active))
                            .or_else(|| {
                                if !has_fullscreen {
                                    layer_map.layer_under(Layer::Top, point)
                                } else {
                                    None
                                }
                            });

                        under_layer.and_then(|layer| {
                            layer_map.layer_geometry(layer).and_then(|geo| {
                                let local_point = point - geo.loc.to_f64();
                                let hit = if !state.active_popups.is_empty() {
                                    layer.surface_under(local_point, smithay::desktop::WindowSurfaceType::POPUP)
                                } else {
                                    None
                                };
                                let hit = hit.or_else(|| {
                                    if layer.namespace() == "malus-top-bar" && local_point.y >= 32.0 {
                                        None
                                    } else {
                                        layer.surface_under(local_point, smithay::desktop::WindowSurfaceType::TOPLEVEL)
                                    }
                                });

                                hit.map(|(surf, sub_loc)| (surf, (geo.loc + sub_loc).to_f64()))
                            })
                        })
                    };

                    let under_target = if state.is_session_locked() {
                        if let Some(ls) = state.authorized_ext_lock_surface_for_output(&output.name()) {
                            Some((ls.wl_surface().clone(), Point::from((0.0, 0.0))))
                        } else {
                            let layer_map = smithay::desktop::layer_map_for_output(&output);
                            let res = layer_map
                                .layers()
                                .rev()
                                .find(|l| l.namespace() == "malus-lock")
                                .and_then(|layer| {
                                    layer_map.layer_geometry(layer).and_then(|geo| {
                                        let local_point = point - geo.loc.to_f64();
                                        layer
                                            .surface_under(
                                                local_point,
                                                smithay::desktop::WindowSurfaceType::TOPLEVEL
                                                    | smithay::desktop::WindowSurfaceType::SUBSURFACE,
                                            )
                                            .map(|(surf, sub_loc)| (surf, (geo.loc + sub_loc).to_f64()))
                                    })
                                });
                            res
                        }
                    } else if let Some((surf, focus_loc)) = find_layer_popup_under(&state, &output, point) {
                        Some((surf, focus_loc))
                    } else if let Some((surf, focus_loc)) = layer_target {
                        Some((surf, focus_loc))
                    } else if let Some((_, loc, surf, sub_loc)) = state.popup_under(point) {
                        Some((surf, (loc + sub_loc).to_f64()))
                    } else if crate::grabs::find_window_resize_edge_under(&state.space, point, 8.0)
                        .is_some()
                    {
                        None
                    } else if state.ssd_titlebar_under(point).is_some() {
                        None
                    } else if let Some((window, loc)) = state.space.element_under(point) {
                        let local_point = point - loc.to_f64();
                        window
                            .surface_under(
                                local_point,
                                smithay::desktop::WindowSurfaceType::TOPLEVEL
                                    | smithay::desktop::WindowSurfaceType::SUBSURFACE,
                            )
                            .map(|(surf, sub_loc)| (surf, (loc + sub_loc).to_f64()))
                    } else if let Some((surf, focus_loc, _, _)) =
                        find_background_layer_under(&state, &output, point)
                    {
                        Some((surf, focus_loc))
                    } else {
                        None
                    };

                    pointer.motion(
                        &mut state,
                        under_target,
                        &MotionEvent {
                            location: point,
                            serial,
                            time: event.time_msec(),
                        },
                    );
                    pointer.frame(&mut state);
                    state.update_ssd_hover(point);
                }
                InputEvent::PointerButton { event } => {
                    let is_pressed = event.state() == ButtonState::Pressed;
                    let point = pointer.current_location();
                    state.last_pointer_location = Some(point);
                    if state.handle_expose_pointer_button(point, event.button_code(), is_pressed) {
                        return;
                    }

                    let has_fullscreen = state.has_fullscreen_window_on_output(&output);
                    let layer_target = {
                        let layer_map = smithay::desktop::layer_map_for_output(&output);
                        let under_layer = layer_map.layer_under(Layer::Overlay, point)
                            .filter(|l| !(l.namespace() == "malus-app-switcher" && !state.app_switcher.is_active))
                            .or_else(|| {
                                if !has_fullscreen {
                                    layer_map.layer_under(Layer::Top, point)
                                } else {
                                    None
                                }
                            });

                        under_layer.and_then(|layer| {
                            layer_map.layer_geometry(layer).and_then(|geo| {
                                let local_point = point - geo.loc.to_f64();
                                let hit = if !state.active_popups.is_empty() {
                                    layer.surface_under(local_point, smithay::desktop::WindowSurfaceType::POPUP)
                                } else {
                                    None
                                };
                                let hit = hit.or_else(|| {
                                    if layer.namespace() == "malus-top-bar" && local_point.y >= 32.0 {
                                        None
                                    } else {
                                        layer.surface_under(local_point, smithay::desktop::WindowSurfaceType::TOPLEVEL)
                                    }
                                });

                                hit.map(|(surf, sub_loc)| {
                                    (
                                        surf.clone(),
                                        (geo.loc + sub_loc).to_f64(),
                                        layer.namespace().to_string(),
                                        layer.can_receive_keyboard_focus(),
                                    )
                                })
                            })
                        })
                    };

                    let (under_target, layer_info, regular_win, is_popup) = if state.is_session_locked() {
                        if let Some(ls) = state.authorized_ext_lock_surface_for_output(&output.name()) {
                            let surf = ls.wl_surface().clone();
                            (
                                Some((surf.clone(), Point::from((0.0, 0.0)))),
                                Some((surf, "ext-session-lock".to_string(), true)),
                                None,
                                false,
                            )
                        } else {
                            let layer_map = smithay::desktop::layer_map_for_output(&output);
                            let hit = layer_map
                                .layers()
                                .rev()
                                .find(|l| l.namespace() == "malus-lock")
                                .and_then(|layer| {
                                    layer_map.layer_geometry(layer).and_then(|geo| {
                                        let local_point = point - geo.loc.to_f64();
                                        layer
                                            .surface_under(
                                                local_point,
                                                smithay::desktop::WindowSurfaceType::TOPLEVEL
                                                    | smithay::desktop::WindowSurfaceType::SUBSURFACE,
                                            )
                                            .map(|(surf, sub_loc)| {
                                                (
                                                    surf.clone(),
                                                    (geo.loc + sub_loc).to_f64(),
                                                    layer.namespace().to_string(),
                                                    layer.can_receive_keyboard_focus(),
                                                )
                                            })
                                    })
                                });
                            if let Some((surf, focus_loc, ns, can_kb)) = hit {
                                (Some((surf.clone(), focus_loc)), Some((surf, ns, can_kb)), None, false)
                            } else {
                                (None, None, None, false)
                            }
                        }
                    } else if let Some((surf, focus_loc)) = find_layer_popup_under(&state, &output, point) {
                        (Some((surf, focus_loc)), None, None, true)
                    } else if let Some((surf, focus_loc, ns, can_kb)) = layer_target {
                        (Some((surf.clone(), focus_loc)), Some((surf, ns, can_kb)), None, false)
                    } else if let Some((_window, loc, surf, sub_loc)) = state.popup_under(point) {
                        (
                            Some((surf, (loc + sub_loc).to_f64())),
                            None,
                            None,
                            true,
                        )
                    } else if let Some((win, _edge, loc)) =
                        crate::grabs::find_window_resize_edge_under(&state.space, point, 8.0)
                    {
                        (None, None, Some((win, loc)), false)
                    } else if let Some((win, loc, _, _, _, _)) = state.ssd_titlebar_under(point) {
                        (None, None, Some((win, loc)), false)
                    } else if let Some((window, loc)) = state.space.element_under(point) {
                        let local_point = point - loc.to_f64();
                        let target = window
                            .surface_under(
                                local_point,
                                smithay::desktop::WindowSurfaceType::TOPLEVEL
                                    | smithay::desktop::WindowSurfaceType::SUBSURFACE,
                            )
                            .map(|(surf, sub_loc)| (surf, (loc + sub_loc).to_f64()));
                        (target, None, Some((window.clone(), loc)), false)
                    } else if let Some((surf, focus_loc, ns, can_kb)) =
                        find_background_layer_under(&state, &output, point)
                    {
                        (
                            Some((surf.clone(), focus_loc)),
                            Some((surf, ns, can_kb)),
                            None,
                            false,
                        )
                    } else {
                        (None, None, None, false)
                    };

                    // Sincronizar foco del puntero antes de procesar el botón para que el cliente reciba enter/button
                    let motion_serial = SERIAL_COUNTER.next_serial();
                    pointer.motion(
                        &mut state,
                        under_target.clone(),
                        &MotionEvent {
                            location: point,
                            serial: motion_serial,
                            time: event.time_msec(),
                        },
                    );

                    let btn_serial = SERIAL_COUNTER.next_serial();
                    if is_pressed {
                        if state.screenshot_ui.is_open() {
                            let output_geo = state.space.output_geometry(&output).unwrap_or_default();
                            match state.screenshot_ui.handle_pointer_button(
                                output_geo,
                                point,
                                event.button_code(),
                                event.state(),
                            ) {
                                crate::screenshot_ui::ScreenshotPointerResult::TriggerAction => {
                                    state.execute_screenshot_ui_action();
                                    return;
                                }
                                crate::screenshot_ui::ScreenshotPointerResult::Consumed => {
                                    return;
                                }
                                crate::screenshot_ui::ScreenshotPointerResult::Ignored => {}
                            }
                        }

                        if state.spotlight.is_open() {
                            let output_geo = state.space.output_geometry(&output).unwrap_or_default();
                            if state.spotlight.handle_pointer_button(
                                output_geo,
                                point,
                                event.button_code(),
                            ) {
                                return;
                            }
                        }

                        if state.handle_snap_layouts_pointer_button(point, event.button_code(), true) {
                            return;
                        }

                        if state.handle_app_switcher_deck_pointer_button(point, event.button_code(), true) {
                            return;
                        }

                        if state.app_switcher.is_active {
                            info!("🖱️ Click detectado mientras AppSwitcher está activo -> cerrando AppSwitcher");
                            state.dismiss_app_switcher();
                        }

                        // Si hay popups activos y el clic NO fue sobre el popup, descartamos los popups
                        let clicked_on_popup = is_popup
                            || under_target
                                .as_ref()
                                .map(|(s, _)| {
                                    state.active_popups.iter().any(|p| p.wl_surface() == s)
                                        || state.popups.find_popup(s).is_some()
                                })
                                .unwrap_or(false);

                        if is_popup {
                            if let Some((win, _, _, _)) = state.popup_under(point) {
                                let was_already_top = state.space.elements().last().map(|w| w == &win).unwrap_or(false);
                                if !was_already_top {
                                    state.space.raise_element(&win, true);
                                }
                            }
                        }

                        state.prepare_pointer_press_grabs(
                            clicked_on_popup,
                            btn_serial,
                            event.time_msec(),
                        );

                        let has_super = keyboard.modifier_state().logo;
                        let has_alt = keyboard.modifier_state().alt;

                        if event.button_code() == 0x110
                            && !has_super
                            && !has_alt
                            && layer_info.is_none()
                            && !clicked_on_popup
                            && !state.popup_grab_active()
                        {
                            if let Some((resize_win, edge, win_loc)) =
                                crate::grabs::find_window_resize_edge_under(&state.space, point, 8.0)
                            {
                                let was_already_top = state
                                    .space
                                    .elements()
                                    .last()
                                    .map(|w| w == &resize_win)
                                    .unwrap_or(false);
                                if !was_already_top {
                                    state.space.raise_element(&resize_win, true);
                                }
                                if let Some(surface) = resize_win.wl_surface() {
                                    if keyboard.current_focus().as_ref() != Some(surface.as_ref()) {
                                        keyboard.set_focus(
                                            &mut state,
                                            Some(surface.clone().into_owned()),
                                            btn_serial,
                                        );
                                        state.notify_active_app(Some(&surface));
                                    }
                                }
                                let start_data = smithay::input::pointer::GrabStartData {
                                    focus: None,
                                    button: event.button_code(),
                                    location: point,
                                };
                                let initial_window_size = resize_win.geometry().size;
                                let grab = crate::grabs::ResizeSurfaceGrab {
                                    start_data,
                                    window: resize_win.clone(),
                                    edges: edge,
                                    initial_window_location: win_loc,
                                    initial_window_size,
                                    last_window_size: initial_window_size,
                                };
                                state.ssd.grab_active = true;
                                state.ssd.cursor_override = true;
                                state.cursor_status =
                                    smithay::input::pointer::CursorImageStatus::Named(
                                        crate::grabs::cursor_icon_for_resize_edge(edge),
                                    );
                                pointer.set_grab(
                                    &mut state,
                                    grab,
                                    btn_serial,
                                    smithay::input::pointer::Focus::Clear,
                                );
                                pointer.button(
                                    &mut state,
                                    &ButtonEvent {
                                        button: event.button_code(),
                                        state: event.state(),
                                        serial: btn_serial,
                                        time: event.time_msec(),
                                    },
                                );
                                pointer.frame(&mut state);
                                info!(
                                    "📐 [ServerResize] Redimensión perimetral activada: edge={:?}, win={:?}",
                                    edge,
                                    resize_win.geometry()
                                );
                                return;
                            }
                        }

                        if !has_super
                            && !has_alt
                            && layer_info.is_none()
                            && !clicked_on_popup
                            && state.handle_ssd_pointer_button(
                                point,
                                event.button_code(),
                                btn_serial,
                            )
                        {
                            pointer.button(
                                &mut state,
                                &ButtonEvent {
                                    button: event.button_code(),
                                    state: event.state(),
                                    serial: btn_serial,
                                    time: event.time_msec(),
                                },
                            );
                            pointer.frame(&mut state);
                            return;
                        }

                        if let Some((surf, ns, can_kb)) = layer_info {
                            info!("🎯 Clic de ratón sobre layer surface: namespace={}, pos={:?}", ns, point);
                            if can_kb {
                                keyboard.set_focus(&mut state, Some(surf), btn_serial);
                            }
                        } else if let Some((window, loc)) = regular_win {
                            info!("🖱️ Clic en ventana regular: button={:#x}, pos={:?}, win_loc={:?}, geo={:?}", event.button_code(), point, loc, window.geometry());
                            let was_already_top = state.space.elements().last().map(|w| w == &window).unwrap_or(false);
                            if !was_already_top {
                                state.space.raise_element(&window, true);
                                // NOTA: No enviamos send_configure() durante Pressed para evitar que GTK4
                                // cancele el GtkGestureClick de los botones de título (cerrar/minimizar/maximizar).
                            }
                            if let Some(surface) = window.wl_surface() {
                                if keyboard.current_focus().as_ref() != Some(surface.as_ref()) {
                                    state.defer_activation_configure = true;
                                    keyboard.set_focus(&mut state, Some(surface.clone().into_owned()), btn_serial);
                                    state.defer_activation_configure = false;
                                    state.notify_active_app(Some(&surface));
                                }
                            }

                            if !has_super && !has_alt && event.button_code() == 0x110 {
                                state.record_csd_close_click_hint(&window, loc, point);
                            }

                            if (has_super || has_alt) && event.button_code() == 0x110 {
                                info!("🚀 Atajo Super/Alt + Clic: iniciando arrastre directo de ventana desde {:?}", loc);
                                let initial_window_location = loc;
                                let start_data = smithay::input::pointer::GrabStartData {
                                    focus: None,
                                    button: event.button_code(),
                                    location: point,
                                };
                                let is_snapped = window.toplevel().map(|t| {
                                    let states = &t.current_state().states;
                                    states.contains(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized)
                                        || states.contains(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::TiledLeft)
                                        || states.contains(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::TiledRight)
                                }).unwrap_or(false);
                                let grab = crate::grabs::MoveSurfaceGrab {
                                    start_data,
                                    window: window.clone(),
                                    initial_window_location,
                                    is_snapped,
                                    has_dragged: false,
                                };
                                state.ssd.grab_active = true;
                                state.moving_window_surface = window.wl_surface().map(|s| s.into_owned());
                                pointer.set_grab(&mut state, grab, btn_serial, smithay::input::pointer::Focus::Clear);
                            }

                            // Si Super o Alt está presionado y es clic derecho (0x111 = BTN_RIGHT): redimensionar ventana directamente desde cualquier punto
                            if (has_super || has_alt) && event.button_code() == 0x111 {
                                let initial_window_location = loc;
                                let initial_window_size = window.geometry().size;

                                let rel_x = point.x - loc.x as f64;
                                let rel_y = point.y - loc.y as f64;
                                let w = initial_window_size.w as f64;
                                let h = initial_window_size.h as f64;

                                let is_right = rel_x >= w / 2.0;
                                let is_bottom = rel_y >= h / 2.0;

                                let edges = match (is_right, is_bottom) {
                                    (true, true) => ResizeEdge::BottomRight,
                                    (false, true) => ResizeEdge::BottomLeft,
                                    (true, false) => ResizeEdge::TopRight,
                                    (false, false) => ResizeEdge::TopLeft,
                                };

                                info!("📐 Atajo Super/Alt + Clic Derecho: iniciando redimensión de ventana ({:?})", edges);
                                let start_data = smithay::input::pointer::GrabStartData {
                                    focus: None,
                                    button: event.button_code(),
                                    location: point,
                                };
                                let grab = crate::grabs::ResizeSurfaceGrab {
                                    start_data,
                                    window: window.clone(),
                                    edges,
                                    initial_window_location,
                                    initial_window_size,
                                    last_window_size: initial_window_size,
                                };
                                state.ssd.grab_active = true;
                                state.ssd.cursor_override = true;
                                state.cursor_status =
                                    smithay::input::pointer::CursorImageStatus::Named(
                                        crate::grabs::cursor_icon_for_resize_edge(edges),
                                    );
                                pointer.set_grab(&mut state, grab, btn_serial, smithay::input::pointer::Focus::Clear);
                            }
                        } else {
                            info!("🖱️ Clic en espacio libre del escritorio: button={:#x}, pos={:?}", event.button_code(), point);
                            state.notify_active_app(None);
                        }
                    } else {
                        if state.screenshot_ui.is_open() {
                            let output_geo = state.space.output_geometry(&output).unwrap_or_default();
                            match state.screenshot_ui.handle_pointer_button(
                                output_geo,
                                point,
                                event.button_code(),
                                event.state(),
                            ) {
                                crate::screenshot_ui::ScreenshotPointerResult::TriggerAction => {
                                    state.execute_screenshot_ui_action();
                                    return;
                                }
                                crate::screenshot_ui::ScreenshotPointerResult::Consumed => {
                                    return;
                                }
                                crate::screenshot_ui::ScreenshotPointerResult::Ignored => {}
                            }
                        }

                        if state.spotlight.is_open() {
                            let output_geo = state.space.output_geometry(&output).unwrap_or_default();
                            let dock_geo = state.dock_surface_geometry().map(|(geo, _)| geo);
                            if let Some(outcome) =
                                state.spotlight.handle_pointer_button_release(
                                    output_geo,
                                    point,
                                    event.button_code(),
                                    dock_geo,
                                )
                            {
                                state.update_spotlight_drag_dock_hover(point);
                    let _ = state.handle_app_switcher_deck_pointer_motion(point);

                    if state.handle_expose_pointer_motion(point) {
                        let serial = SERIAL_COUNTER.next_serial();
                        pointer.motion(
                            &mut state,
                            None,
                            &MotionEvent {
                                location: point,
                                serial,
                                time: event.time_msec(),
                            },
                        );
                        pointer.frame(&mut state);
                        return;
                    }
                    state.update_ssd_hover(point);
                                state.handle_spotlight_pointer_release(output_geo, outcome);
                                return;
                            }
                        }
                        if state.handle_snap_layouts_pointer_button(point, event.button_code(), false) {
                            return;
                        }
                        if state.handle_app_switcher_deck_pointer_button(point, event.button_code(), false) {
                            return;
                        }
                        state.snap_preview = None;
                    }

                    pointer.button(
                        &mut state,
                        &ButtonEvent {
                            serial: btn_serial,
                            time: event.time_msec(),
                            button: event.button_code(),
                            state: event.state(),
                        },
                    );
                    if !pointer.is_grabbed() {
                        state.snap_preview = None;
                    }
                    pointer.frame(&mut state);
                    if !is_pressed {
                        state.flush_pending_activation_configures();
                    }
                }
                InputEvent::PointerAxis { event } => {
                    if let Some(vertical) = event.amount(Axis::Vertical) {
                        if state.handle_app_switcher_deck_axis(vertical) {
                            return;
                        }
                    }
                    if state.spotlight.is_open() {
                        if let Some(vertical) = event.amount(Axis::Vertical) {
                            if vertical.abs() > 0.001 && state.spotlight.handle_scroll(vertical) {
                                return;
                            }
                        }
                    }
                    let mut frame = AxisFrame::new(event.time_msec()).source(event.source());
                    if let Some(horizontal) = event.amount(Axis::Horizontal) {
                        frame = frame.value(Axis::Horizontal, horizontal);
                        if let Some(discrete) = event.amount_v120(Axis::Horizontal) {
                            frame = frame.v120(Axis::Horizontal, discrete as i32);
                        }
                    }
                    if let Some(vertical) = event.amount(Axis::Vertical) {
                        frame = frame.value(Axis::Vertical, vertical);
                        if let Some(discrete) = event.amount_v120(Axis::Vertical) {
                            frame = frame.v120(Axis::Vertical, discrete as i32);
                        }
                    }
                    if event.amount(Axis::Horizontal).is_some() || event.amount(Axis::Vertical).is_some() {
                        pointer.axis(&mut state, frame);
                        pointer.frame(&mut state);
                    }
                }
                _ => {}
            },
            _ => {}
        });

        if !state.is_running {
            break;
        }

        match status {
            PumpStatus::Continue => (),
            PumpStatus::Exit(_) => break,
        }

        let size = backend.window_size();
        let damage = Rectangle::new((0, 0).into(), size);

        // Renderizado del frame
        let mut visible_surface_ids = std::collections::HashSet::new();
        {
            if let Ok((renderer, mut framebuffer)) = backend.bind() {
                let now = Instant::now();
                let active_genies: Vec<crate::state::GenieAnimation> = state
                    .active_animations
                    .iter()
                    .filter(|anim| !anim.waiting_for_dock_slot)
                    .cloned()
                    .collect();
                for anim in &active_genies {
                    visible_surface_ids.insert(
                        smithay::backend::renderer::element::Id::from_wayland_resource(
                            &anim.surface,
                        ),
                    );
                }

                if state.pending_displays_reload {
                    state.pending_displays_reload = false;
                    current_displays_cfg = malus_de_common::config::DisplaysConfig::load();
                    state.relocate_primary_shell_layers_for_config(&current_displays_cfg, Vec::new());
                    squircle_corner_cache.clear();
                }

                let corner_radius_pt = current_displays_cfg.screen_corner_radius;
                let phys_r = if corner_radius_pt > 0.0 {
                    let r = corner_radius_pt.round() as i32;
                    if r > 2 {
                        if !squircle_corner_cache.contains_key(&r) {
                            if let Some(textures) =
                                super::drm::create_squircle_corner_textures(renderer, r)
                            {
                                squircle_corner_cache.insert(r, textures);
                            }
                        }
                        Some(r)
                    } else {
                        None
                    }
                } else {
                    None
                };

                let win_corner_radius_pt = current_displays_cfg.window_corner_radius;
                let win_phys_r = if win_corner_radius_pt > 0.0 {
                    let r = win_corner_radius_pt.round() as i32;
                    if r > 2 {
                        Some(r)
                    } else {
                        None
                    }
                } else {
                    None
                };

                let session_locked = state.is_session_locked();
                let layer_map = smithay::desktop::layer_map_for_output(&output);
                let has_fullscreen_window = state.has_fullscreen_window_on_output(&output);
                let mut lower_elements = Vec::new();
                let mut upper_elements = Vec::new();
                let mut lock_elements = Vec::new();
                let mut dock_clip_roots = Vec::new();
                let mut spotlight_backdrop_elements = Vec::new();
                let mut spotlight_shadow_rects = Vec::new();
                let mut dock_popups = Vec::new();
                let mut topbar_popups = Vec::new();
                let mut wallpaper_popups = Vec::new();
                let mut dnd_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();
                let mut dnd_fallback_elem: Option<
                    smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement<
                        GlesRenderer,
                    >,
                > = None;
                if let Some(loc) = state.active_dnd_icon_draw_location() {
                    if let Some(dnd_surf) = state.dnd_icon.clone() {
                        if smithay::utils::IsAlive::alive(&dnd_surf) {
                            if state.dnd_icon_cache_dirty {
                                super::capture_dnd_icon_cache(renderer, &mut state, &dnd_surf, 1.0);
                            }
                            let offset = state.dnd_icon_offset;
                            let phys_loc = Point::from((
                                (loc.x + offset.x as f64).round() as i32,
                                (loc.y + offset.y as f64).round() as i32,
                            ));
                            dnd_elems = render_elements_from_surface_tree::<
                                GlesRenderer,
                                WaylandSurfaceRenderElement<GlesRenderer>,
                            >(
                                renderer,
                                &dnd_surf,
                                phys_loc,
                                Scale::from(1.0),
                                1.0,
                                Kind::Unspecified,
                            );
                        }
                    }
                    if dnd_elems.is_empty() && state.dnd_drop_hold.is_some() {
                        dnd_fallback_elem = super::build_cached_dnd_fallback(
                            renderer,
                            &state,
                            loc.x,
                            loc.y,
                            1.0,
                        );
                    }
                }
                let spotlight_active = layer_map
                    .layers()
                    .any(|surface| surface.namespace() == "malus-spotlight-results");
                for surface in layer_map.layers().rev() {
                    if surface.namespace() == "malus-app-switcher" && !state.app_switcher.is_active
                    {
                        continue;
                    }
                    if let Some(geo) = layer_map.layer_geometry(surface) {
                        let namespace = surface.namespace();
                        if spotlight_active && namespace == "malus-spotlight-results" {
                            spotlight_shadow_rects.push(geo.to_physical_precise_round(1.0));
                        }
                        let layer_loc = geo.loc.to_physical_precise_round(1.0);
                        let s_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                            render_elements_from_surface_tree(
                                renderer,
                                surface.wl_surface(),
                                layer_loc,
                                smithay::utils::Scale::from(1.0),
                                1.0,
                                Kind::Unspecified,
                            );
                        if surface.namespace() == "malus-dock" && !has_fullscreen_window {
                            let root_id =
                                smithay::backend::renderer::element::Id::from_wayland_resource(
                                    surface.wl_surface(),
                                );
                            if let Some(root_elem) =
                                s_elems.iter().find(|elem| *elem.id() == root_id)
                            {
                                dock_clip_roots.push((
                                    root_id,
                                    super::dock_clip_rect(
                                        root_elem.geometry(smithay::utils::Scale::from(1.0)),
                                        state.dock_blur_headroom,
                                        1.0,
                                    ),
                                ));
                            }
                        }
                        let mut p_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                            Vec::new();
                        for (popup, popup_offset) in
                            smithay::desktop::PopupManager::popups_for_surface(surface.wl_surface())
                        {
                            let popup_geometry = popup.geometry();
                            let offset = (popup_offset - popup_geometry.loc)
                                .to_f64()
                                .to_physical(smithay::utils::Scale::from(1.0))
                                .to_i32_round();
                            let elems = render_elements_from_surface_tree(
                                renderer,
                                popup.wl_surface(),
                                layer_loc + offset,
                                smithay::utils::Scale::from(1.0),
                                1.0,
                                Kind::Unspecified,
                            );
                            if surface.namespace() == "malus-top-bar" {
                                if !has_fullscreen_window {
                                    let popup_rect = Rectangle::new(
                                        layer_loc + popup_offset.to_physical_precise_round(1.0),
                                        popup_geometry.size.to_physical_precise_round(1.0),
                                    );
                                    topbar_popups.push((elems, popup_rect));
                                }
                            } else if surface.namespace() == "malus-dock" {
                                if !has_fullscreen_window {
                                    let popup_rect = Rectangle::new(
                                        layer_loc + offset,
                                        popup_geometry.size.to_physical_precise_round(1.0),
                                    );
                                    dock_popups.push((elems, popup_rect));
                                }
                            } else if surface.namespace() == "malus-wallpaper" {
                                let popup_rect = Rectangle::new(
                                    layer_loc + popup_offset.to_physical_precise_round(1.0),
                                    popup_geometry.size.to_physical_precise_round(1.0),
                                );
                                wallpaper_popups.push((elems, popup_rect));
                            } else {
                                p_elems.extend(elems);
                            }
                        }

                        if namespace == "malus-spotlight-backdrop" {
                            spotlight_backdrop_elements
                                .push((s_elems, geo.to_physical_precise_round(1.0)));
                        } else if namespace == "malus-lock" {
                            lock_elements.extend(s_elems);
                            lock_elements.extend(p_elems);
                        } else if matches!(surface.layer(), Layer::Background | Layer::Bottom) {
                            lower_elements.extend(s_elems);
                            upper_elements.extend(p_elems);
                        } else if surface.layer() == Layer::Top {
                            // Por protocolo wlr-layer-shell, Layer::Top está por debajo de ventanas a pantalla completa.
                            // Solo se dibuja en capas superiores cuando NO hay una ventana a pantalla completa en este monitor.
                            if !has_fullscreen_window {
                                upper_elements.extend(s_elems);
                                upper_elements.extend(p_elems);
                            }
                        } else {
                            upper_elements.extend(s_elems);
                            upper_elements.extend(p_elems);
                        }
                    }
                }
                if let Some(ext_lock_surf) =
                    state.authorized_ext_lock_surface_for_output(&output.name())
                {
                    let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                        render_elements_from_surface_tree(
                            renderer,
                            ext_lock_surf.wl_surface(),
                            (0, 0),
                            smithay::utils::Scale::from(1.0),
                            1.0,
                            Kind::Unspecified,
                        );
                    lock_elements.extend(elems);
                }

                let mut window_render_data = Vec::new();
                let is_dark_theme = crate::state::is_dark_color_scheme();
                let output_geo = state
                    .space
                    .output_geometry(&output)
                    .unwrap_or_else(|| Rectangle::new((0, 0).into(), (size.w, size.h).into()));
                for window in state.space.elements() {
                    if let Some(loc) = state.space.element_location(window) {
                        let geo = window.geometry();
                        let Some(main_surface) = window.wl_surface() else {
                            continue;
                        };

                        let trans_opt = state.snap_transitions.get(main_surface.as_ref()).cloned();
                        let transition_layout = trans_opt
                            .as_ref()
                            .map(|trans| trans.render_geometry(geo, output_geo.loc, 1.0, now));
                        let (effective_loc, effective_size, is_transitioning) =
                            if let Some((eloc, esize, ..)) = transition_layout {
                                (eloc, esize, true)
                            } else {
                                (loc, geo.size, false)
                            };

                        let win_global_rect = Rectangle::new(effective_loc, effective_size);
                        if output_geo.overlaps(win_global_rect) {
                            let (phys_loc, transition_clip, content_offset, content_size, win_screen_rect) =
                                if let Some((_, _, pl, clip_rect, c_off, c_sz, w_rect)) = transition_layout {
                                    (pl, clip_rect, c_off, c_sz, w_rect)
                                } else {
                                    let pl = (loc - geo.loc - output_geo.loc).to_physical_precise_round(1.0);
                                    let c_off = geo.loc.to_physical_precise_round(1.0);
                                    let c_sz = geo.size.to_physical_precise_round(1.0);
                                    let w_rect = Rectangle::new(
                                        (loc - output_geo.loc).to_physical_precise_round(1.0),
                                        geo.size.to_physical_precise_round(1.0),
                                    );
                                    (pl, None, c_off, c_sz, w_rect)
                                };

                            let win_surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                                render_elements_from_surface_tree(
                                    renderer,
                                    main_surface.as_ref(),
                                    phys_loc,
                                    smithay::utils::Scale::from(1.0),
                                    1.0,
                                    Kind::Unspecified,
                                );
                            if !is_transitioning {
                                let snap_root_id =
                                    smithay::backend::renderer::element::Id::from_wayland_resource(
                                        main_surface.as_ref(),
                                    );
                                crate::black_hole::update_cached_window_snapshot(
                                    &mut state.cached_close_snapshots,
                                    main_surface.as_ref(),
                                    &win_surfaces,
                                    &snap_root_id,
                                    phys_loc,
                                    1.0,
                                );
                            }

                            let mut popup_elems: Vec<(
                                Vec<WaylandSurfaceRenderElement<GlesRenderer>>,
                                Rectangle<i32, Physical>,
                                bool,
                            )> = Vec::new();
                            let mut popup_shadow_rects = Vec::new();
                            let popup_effects_enabled =
                                crate::state::app_id_for_surface(main_surface.as_ref())
                                    .map(|app_id| {
                                        !app_id.contains("org.malus_de.dock")
                                            && !app_id.contains("malus-de-dock")
                                            && !crate::state::is_browser_identifier(&app_id)
                                    })
                                    .unwrap_or(true)
                                    && !crate::state::is_browser_window(&window);
                            let mut raw_popups: Vec<_> =
                                smithay::desktop::PopupManager::popups_for_surface(
                                    main_surface.as_ref(),
                                )
                                .collect();
                            raw_popups.reverse();
                            for (popup, popup_offset) in raw_popups {
                                let popup_geometry = popup.geometry();
                                let popup_rect = Rectangle::new(
                                    phys_loc
                                        + (geo.loc + popup_offset).to_physical_precise_round(1.0),
                                    popup_geometry.size.to_physical_precise_round(1.0),
                                );
                                if popup_effects_enabled {
                                    popup_shadow_rects.push(popup_rect);
                                }
                                let offset = (geo.loc + popup_offset - popup_geometry.loc)
                                    .to_physical_precise_round(1.0);
                                let p_elems = render_elements_from_surface_tree(
                                    renderer,
                                    popup.wl_surface(),
                                    phys_loc + offset,
                                    smithay::utils::Scale::from(1.0),
                                    1.0,
                                    Kind::Unspecified,
                                );
                                popup_elems.push((p_elems, popup_rect, popup_effects_enabled));
                            }

                            let (is_fullscreen, is_maximized) = window
                                .toplevel()
                                .map(|t| {
                                    let states = &t.current_state().states;
                                    (
                                        states.contains(XdgState::Fullscreen),
                                        states.contains(XdgState::Maximized),
                                    )
                                })
                                .unwrap_or((false, false));

                            let is_active = window.toplevel().map(|t| {
                                let states = &t.current_state().states;
                                states.contains(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated)
                            }).unwrap_or(false) || (state.seat.get_keyboard().and_then(|k| k.current_focus()).as_ref() == Some(main_surface.as_ref()));

                            let is_firefox = crate::state::is_firefox_window(&window);
                            let is_translucent = crate::state::is_translucent_window(&window);
                            let is_branches = crate::state::is_branches_window(&window);
                            let uses_ssd = crate::state::window_uses_ssd(window);
                            let root_id =
                                smithay::backend::renderer::element::Id::from_wayland_resource(
                                    main_surface.as_ref(),
                                );
                            let should_round = win_phys_r.is_some() && !is_fullscreen;

                            let win_screen_rect = if transition_clip.is_none() && !uses_ssd {
                                crate::window_corners::effective_shadow_rect_for_window(
                                    &win_surfaces,
                                    &root_id,
                                    win_screen_rect,
                                    1.0,
                                    is_translucent,
                                    uses_ssd,
                                )
                            } else {
                                win_screen_rect
                            };

                            let (win_screen_rect, ssd_elem) = if uses_ssd && !is_fullscreen {
                                let tb_phys_h = crate::ssd::SSD_TITLEBAR_HEIGHT;
                                let tb_loc = Point::from((
                                    win_screen_rect.loc.x,
                                    win_screen_rect.loc.y - tb_phys_h,
                                ));
                                let full_rect = Rectangle::new(
                                    tb_loc,
                                    smithay::utils::Size::from((
                                        win_screen_rect.size.w,
                                        win_screen_rect.size.h + tb_phys_h,
                                    )),
                                );
                                let ssd_title = crate::state::window_title_for_ssd(window);
                                let elem = state.ssd.render_element_for_window(
                                    renderer,
                                    main_surface.as_ref(),
                                    tb_loc,
                                    win_screen_rect.size.w,
                                    tb_phys_h,
                                    1.0,
                                    &ssd_title,
                                    is_active,
                                    is_dark_theme,
                                );
                                (full_rect, elem)
                            } else {
                                (win_screen_rect, None)
                            };

                            window_render_data.push((
                                root_id,
                                win_surfaces,
                                popup_elems,
                                popup_shadow_rects,
                                should_round,
                                is_active,
                                win_screen_rect,
                                content_offset,
                                content_size,
                                is_firefox,
                                is_translucent,
                                transition_clip,
                                ssd_elem,
                                is_maximized,
                                is_branches,
                            ));
                        }
                    }
                }

                // Inicializar shader de recorte squircle para ventanas si aún no existe
                if state.renderer_context_id.is_none() {
                    state.renderer_context_id = Some(renderer.context_id());
                }
                if window_squircle_shader.is_none() {
                    window_squircle_shader =
                        crate::window_corners::WindowSquircleShader::new(renderer);
                }
                if black_hole_pipeline.is_none() {
                    black_hole_pipeline = renderer
                        .with_context(|gl| crate::black_hole::BlackHolePipeline::new(gl))
                        .ok()
                        .flatten();
                }
                if genie_pipeline.is_none() {
                    genie_pipeline = renderer
                        .with_context(|gl| crate::magic_lamp::GeniePipeline::new(gl))
                        .ok()
                        .flatten();
                }

                let output_geo = state.space.output_geometry(&output).unwrap_or_default();
                if !state.pending_black_holes.is_empty() {
                    let out_name = output.name();
                    for pending in &state.pending_black_holes {
                        if let Some(active) = crate::black_hole::capture_closing_window_texture(
                            renderer,
                            &mut state.ssd,
                            window_squircle_shader.as_ref(),
                            blur_pipeline.as_mut(),
                            pending,
                            &out_name,
                            output_geo,
                            size,
                            1.0,
                            win_phys_r,
                            is_dark_theme,
                        ) {
                            active_black_holes.push(active);
                        }
                    }
                    state.pending_black_holes.clear();
                }
                crate::capture::capture_pending_genie_snapshots(
                    &mut state,
                    renderer,
                    window_squircle_shader.as_ref(),
                    blur_pipeline.as_mut(),
                    &mut genie_snapshots,
                    output_geo,
                    size,
                    false,
                    1.0,
                    win_phys_r,
                    is_dark_theme,
                    now,
                );
                let spotlight_frame = if state.spotlight.is_open() {
                    if let Some((spot_rect, mask_opt, mask_sz, spot_alpha)) =
                        state.spotlight.prepare_frame(output_geo, 1.0, is_dark_theme)
                    {
                        let mask_vec = mask_opt.map(|s| s.to_vec());
                        state
                            .spotlight
                            .create_render_element(renderer, spot_rect.loc.to_f64(), spot_alpha)
                            .map(|elem| (spot_rect, mask_vec, mask_sz, spot_alpha, elem))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let spotlight_drag_elem = if state.spotlight.has_active_drag_or_poof() {
                    state.spotlight.prepare_drag_render_element(
                        renderer,
                        output_geo,
                        1.0,
                        is_dark_theme,
                    )
                } else {
                    None
                };

                let screenshot_ui_frame = if state.screenshot_ui.is_open() {
                    if let Some((ui_rect, mask_opt, mask_sz, ui_alpha)) =
                        state.screenshot_ui.prepare_frame(output_geo, 1.0, is_dark_theme)
                    {
                        let mask_vec = mask_opt.map(|s| s.to_vec());
                        state
                            .screenshot_ui
                            .create_render_element(renderer, ui_rect.loc.to_f64())
                            .map(|elem| (ui_rect, mask_vec, mask_sz, ui_alpha, elem))
                    } else {
                        None
                    }
                } else {
                    None
                };

                let snap_layouts_frame = if state.snap_layouts.should_render_on_output(&output.name()) {
                    if let Some((card_rect, mask_opt, mask_sz, snap_alpha)) =
                        state.snap_layouts.prepare_frame(&output.name(), output_geo, 1.0, is_dark_theme)
                    {
                        let mask_vec = mask_opt.map(|s| s.to_vec());
                        state
                            .snap_layouts
                            .create_render_element(renderer, card_rect.loc, snap_alpha)
                            .map(|elem| (card_rect, mask_vec, mask_sz, snap_alpha, elem))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let snap_layouts_preview = state.snap_layouts.hovered_preview_rect(&output.name());
                let app_switcher_deck_frame = if state.deck_should_render_on_output(&output.name()) {
                    let deck_windows = state.deck_windows_for_render();
                    let app_switcher = &mut state.app_switcher;
                    let ssd = &mut state.ssd;
                    crate::app_switcher::prepare_deck_cards(
                        renderer,
                        &mut app_switcher.deck,
                        ssd,
                        &app_switcher.items,
                        &deck_windows,
                        output_geo,
                        1.0,
                        is_dark_theme,
                        now,
                    )
                } else {
                    None
                };

                let screenshot_request = state
                    .pending_screenshot_requests
                    .pop_front()
                    .map(|(destination, _)| destination);
                let theme_capture_request = state.pending_theme_transition_capture.clone();
                let workspace_switch_pending = state.pending_workspace_switch.is_some();
                let mut frame_rendered = false;
                if let Ok(mut frame) =
                    renderer.render(&mut framebuffer, size, Transform::Flipped180)
                {
                    // Precomputar oclusión (Occlusion Culling) de arriba hacia abajo para evitar overdraw masivo
                    let mut body_occluded = vec![false; window_render_data.len()];
                    let mut shadow_occluded = vec![false; window_render_data.len()];
                    let mut occluders: Vec<super::OpaqueOccluder> = Vec::new();

                    let screen_bounds = Rectangle::from_size(size);
                    let shadow_pad = 80;
                    for i in (0..window_render_data.len()).rev() {
                        let win = &window_render_data[i];
                        let win_rect = win.6;
                        let should_round = win.4;
                        let is_translucent = win.10;
                        let is_transitioning = win.11.is_some();
                        let is_maximized = win.13;
                        let is_branches = win.14;
                        let target_r = if should_round {
                            win_phys_r.unwrap_or(0)
                        } else {
                            0
                        };

                        if is_maximized && !is_transitioning {
                            shadow_occluded[i] = true;
                        }

                        let raw_shadow_rect = Rectangle::new(
                            (win_rect.loc.x - shadow_pad, win_rect.loc.y - shadow_pad).into(),
                            (win_rect.size.w + shadow_pad * 2, win_rect.size.h + shadow_pad * 2).into(),
                        );
                        let shadow_rect = super::clamp_rect_to_bounds(raw_shadow_rect, screen_bounds);

                        for occluder in &occluders {
                            if occluder.contains_rounded_rect(win_rect, target_r) {
                                body_occluded[i] = true;
                            }
                            if !shadow_occluded[i]
                                && occluder.contains_rounded_rect(shadow_rect, target_r)
                            {
                                shadow_occluded[i] = true;
                            }
                            if body_occluded[i] && shadow_occluded[i] {
                                break;
                            }
                        }

                        if !is_translucent && !is_transitioning {
                            occluders.push(super::OpaqueOccluder {
                                rect: win_rect,
                                corner_radius: target_r,
                            });
                        } else if is_branches && !is_transitioning {
                            let sidebar_w = 212;
                            if win_rect.size.w > sidebar_w {
                                let opaque_right_rect = Rectangle::new(
                                    (win_rect.loc.x + sidebar_w, win_rect.loc.y).into(),
                                    (win_rect.size.w - sidebar_w, win_rect.size.h).into(),
                                );
                                occluders.push(super::OpaqueOccluder {
                                    rect: opaque_right_rect,
                                    corner_radius: target_r,
                                });
                            }
                        }
                    }

                    let wallpaper_occluded = app_switcher_deck_frame.is_none()
                        && occluders
                            .iter()
                            .any(|occ| occ.contains_rect(Rectangle::from_size(size)));

                    frame_rendered = true;
                    let clear_color = if session_locked {
                        Color32F::new(0.0, 0.0, 0.0, 1.0)
                    } else {
                        dark_acrylic
                    };
                    let _ = frame.clear(clear_color, &[damage]);
                    if session_locked {
                        if !lock_elements.is_empty() {
                            let _ = draw_render_elements(
                                &mut frame,
                                1.0,
                                &lock_elements,
                                &[damage],
                            );
                        }
                    } else {
                    // 1. Renderizar fondo (wallpaper y capas inferiores) solo si no está 100% ocluido por una ventana pantalla completa
                    if !wallpaper_occluded && !lower_elements.is_empty() {
                        let _ = draw_render_elements(&mut frame, 1.0, &lower_elements, &[damage]);
                    }

                    // Inicializar pipeline de sombras squircle si aún no existe
                    if shadow_pipeline.is_none() {
                        shadow_pipeline = frame
                            .with_context(|gl| crate::shadow::WindowShadowPipeline::new(gl))
                            .ok()
                            .flatten();
                    }

                    // Inicializar pipeline de desenfoque Dual Kawase si aún no existe
                    if blur_pipeline.is_none() {
                        blur_pipeline = frame
                            .with_context(|gl| crate::blur::GaussianBlur::new(gl))
                            .ok()
                            .flatten();
                    }

                    let projection = *frame.projection();
                    if !wallpaper_occluded {
                        if let Some(ref mut blur) = blur_pipeline {
                            let _ = frame.with_context(|gl| {
                                blur.capture_background(gl, &projection, size);
                            });
                        }
                    }

                    let is_pointer_grabbed = state
                        .seat
                        .get_pointer()
                        .map(|p| p.is_grabbed())
                        .unwrap_or(false);
                    if !is_pointer_grabbed {
                        state.snap_preview = None;
                        state.moving_window_surface = None;
                    }
                    let mut snap_preview_drawn = false;

                    // 2. Renderizar superficies de cada ventana con sus sombras proyectadas, desenfoque de fondo y esquinas squircle
                    for (
                        idx,
                        (
                            root_id,
                            win_elems,
                            _popups,
                            _popup_shadow_rects,
                            should_round,
                            is_active,
                            win_rect,
                            _content_offset,
                            _content_size,
                            is_firefox,
                            is_translucent,
                            transition_clip,
                            ssd_elem,
                            _is_maximized,
                            is_branches,
                        ),
                    ) in window_render_data.iter().enumerate()
                    {
                        if app_switcher_deck_frame.is_some() || win_elems.is_empty() {
                            continue;
                        }

                        // Dibujar la previsualización de ajuste (Snap Preview) INMEDIATAMENTE ANTES
                        // de la ventana activa que se está arrastrando, de modo que la ventana quede POR ENCIMA
                        if !snap_preview_drawn && is_pointer_grabbed {
                            let is_target_window = state.moving_window_surface.as_ref().map_or(
                                idx == window_render_data.len() - 1,
                                |moving_surf| {
                                    let moving_id = smithay::backend::renderer::element::Id::from_wayland_resource(moving_surf);
                                    *root_id == moving_id
                                },
                            );
                            if is_target_window {
                                if let Some(preview) = state.snap_preview {
                                    crate::blur::render_snap_preview_overlay(
                                        &mut frame,
                                        blur_pipeline.as_mut(),
                                        &projection,
                                        size,
                                        preview,
                                        output_geo.loc,
                                        1.0,
                                        win_phys_r,
                                    );
                                }
                                snap_preview_drawn = true;
                            }
                        }

                        let is_body_occluded = body_occluded[idx];
                        let is_shadow_occluded = shadow_occluded[idx];

                        if !is_body_occluded {
                            visible_surface_ids.insert(root_id.clone());
                        }

                        // 2.1. SOMBRA PROYECTADA (Window Drop Shadow con Squircle SDF n=4.2)
                        // Se dibuja para ventanas no en pantalla completa inmediatamente antes de su superficie
                        if *should_round && !is_shadow_occluded {
                            if let (Some(r), Some(ref shadow)) =
                                (win_phys_r, shadow_pipeline.as_ref())
                            {
                                let _ = frame.with_context(|gl| {
                                    shadow.render_shadow(
                                        gl,
                                        &projection,
                                        *win_rect,
                                        r as f32,
                                        *is_active,
                                        1.0,
                                    );
                                });
                            }
                        }

                        // 2.1.5. DESENFOQUE DE FONDO DUAL KAWASE (Branches y Ajustes del Sistema)
                        if *is_translucent && !is_body_occluded {
                            if let Some(ref mut blur) = blur_pipeline {
                                let corner_radius = if *should_round {
                                    win_phys_r.unwrap_or(0) as f32
                                } else {
                                    0.0
                                };
                                let blur_rect = if *is_branches {
                                    let sidebar_blur_w =
                                        (212 + corner_radius as i32 + 4).min(win_rect.size.w);
                                    Rectangle::new(
                                        win_rect.loc,
                                        (sidebar_blur_w, win_rect.size.h).into(),
                                    )
                                } else {
                                    *win_rect
                                };
                                let screen_size = size;
                                let _ = frame.with_context(|gl| {
                                    if idx > 0 {
                                        blur.capture_background_region(
                                            gl,
                                            &projection,
                                            screen_size,
                                            blur_rect,
                                        );
                                    }
                                    blur.render_blur(
                                        gl,
                                        &projection,
                                        screen_size,
                                        blur_rect,
                                        corner_radius,
                                    );
                                });
                            }
                        }

                        // 2.2. SUPERFICIES DE LA VENTANA (con respaldo 1:1 durante SnapTransition y recorte Squircle)
                        let needs_transition_backdrop = transition_clip.is_some_and(|c| {
                            c.size.w < win_rect.size.w
                                || (c.loc.y + c.size.h) < (win_rect.loc.y + win_rect.size.h)
                        });
                        if needs_transition_backdrop && !is_body_occluded {
                            let corner_radius = if *should_round {
                                win_phys_r.unwrap_or(0) as f32
                            } else {
                                0.0
                            };
                            crate::blur::render_snap_transition_backdrop(
                                &mut frame,
                                blur_pipeline.as_mut(),
                                &projection,
                                size,
                                *win_rect,
                                corner_radius,
                                idx > 0,
                                *is_translucent,
                                *is_branches,
                                is_dark_theme,
                            );
                        }
                        if !is_body_occluded {
                            crate::window_corners::render_window_elements(
                                &mut frame,
                                1.0,
                                &[damage],
                                window_squircle_shader.as_ref(),
                                root_id,
                                win_elems,
                                ssd_elem.as_ref(),
                                *win_rect,
                                win_phys_r,
                                *should_round,
                                *is_firefox,
                                *transition_clip,
                            );
                        }
                    }

                    // Si no se dibujó antes (por ejemplo, si no hay ventanas en este monitor), dibujarlo sobre el fondo
                    if !snap_preview_drawn && is_pointer_grabbed {
                        if let Some(preview) = state.snap_preview {
                            crate::blur::render_snap_preview_overlay(
                                &mut frame,
                                blur_pipeline.as_mut(),
                                &projection,
                                size,
                                preview,
                                output_geo.loc,
                                1.0,
                                win_phys_r,
                            );
                        }
                    }

                    // 2. Aplicar desenfoque de fondo Dual Kawase por GPU a las capas y popups translúcidos
                    if blur_pipeline.is_none() {
                        blur_pipeline = frame
                            .with_context(|gl| crate::blur::GaussianBlur::new(gl))
                            .ok()
                            .flatten();
                    }

                    if let Some(ref mut blur) = blur_pipeline {
                        let screen_size = size;
                        let mut layer_blur_items: Vec<(Rectangle<i32, Physical>, f32, bool)> =
                            Vec::new();
                        for (_, rect) in &spotlight_backdrop_elements {
                            layer_blur_items.push((*rect, 26.0, false));
                        }
                        for surface in layer_map.layers() {
                            if matches!(surface.layer(), Layer::Top | Layer::Overlay) {
                                if has_fullscreen_window && surface.layer() == Layer::Top {
                                    continue;
                                }
                                let ns = surface.namespace();
                                if ns == "malus-app-switcher" && !state.app_switcher.is_active {
                                    continue;
                                }
                                if ns == "malus-top-bar" {
                                    continue;
                                }
                                if let Some(geo) = layer_map.layer_geometry(surface) {
                                    let radius = match ns.as_ref() {
                                        "malus-dock" => 32.0,
                                        "malus-app-switcher" => 32.0,
                                        "malus-lock" => 0.0,
                                        "malus-spotlight-controls"
                                        | "malus-spotlight-input"
                                        | "malus-spotlight-apps"
                                        | "malus-spotlight-files" => 27.0,
                                        "malus-notifications" => 16.0,
                                        _ => 0.0,
                                    };
                                    if radius > 0.0 {
                                        let is_spotlight_comp = matches!(
                                            ns.as_ref(),
                                            "malus-spotlight-controls"
                                                | "malus-spotlight-input"
                                                | "malus-spotlight-apps"
                                                | "malus-spotlight-files"
                                        );
                                        let phys_rect = if ns == "malus-dock" {
                                            let root_id =
                                                smithay::backend::renderer::element::Id::from_wayland_resource(
                                                    surface.wl_surface(),
                                                );
                                            let Some(rect) = dock_clip_roots
                                                .iter()
                                                .find(|(id, _)| *id == root_id)
                                                .map(|(_, rect)| *rect)
                                            else {
                                                continue;
                                            };
                                            rect
                                        } else {
                                            let rect = geo.to_physical_precise_round(1.0);
                                            if is_spotlight_comp {
                                                super::spotlight_component_blur_rect(rect, 1.0)
                                            } else {
                                                rect
                                            }
                                        };
                                        if phys_rect.size.w > 0 && phys_rect.size.h > 0 {
                                            layer_blur_items.push((
                                                phys_rect,
                                                radius,
                                                is_spotlight_comp,
                                            ));
                                        }
                                    }
                                }
                            }
                        }

                        if !layer_blur_items.is_empty() {
                            let mut min_x = i32::MAX;
                            let mut min_y = i32::MAX;
                            let mut max_x = i32::MIN;
                            let mut max_y = i32::MIN;
                            for (rect, _, _) in &layer_blur_items {
                                min_x = min_x.min(rect.loc.x);
                                min_y = min_y.min(rect.loc.y);
                                max_x = max_x.max(rect.loc.x + rect.size.w);
                                max_y = max_y.max(rect.loc.y + rect.size.h);
                            }
                            let capture_region = Rectangle::new(
                                (min_x, min_y).into(),
                                ((max_x - min_x).max(0), (max_y - min_y).max(0)).into(),
                            );
                            let _ = frame.with_context(|gl| {
                                blur.capture_background_region(
                                    gl,
                                    &projection,
                                    screen_size,
                                    capture_region,
                                );
                                for (phys_rect, radius, is_round) in layer_blur_items {
                                    if is_round {
                                        blur.render_captured_blur_round(
                                            gl,
                                            &projection,
                                            screen_size,
                                            phys_rect,
                                            radius,
                                        );
                                    } else {
                                        blur.render_captured_blur(
                                            gl,
                                            &projection,
                                            screen_size,
                                            phys_rect,
                                            radius,
                                        );
                                    }
                                }
                            });
                        }
                    }

                    for (elements, rect) in &spotlight_backdrop_elements {
                        for elem in elements {
                            if let Some(shader) = window_squircle_shader.as_ref() {
                                let elem_geo = elem.geometry(Scale::from(1.0));
                                shader.apply_clip_to_frame(
                                    &mut frame,
                                    elem_geo.size,
                                    rect.loc - elem_geo.loc,
                                    rect.size,
                                    26,
                                );
                            }
                            let _ = draw_render_elements(
                                &mut frame,
                                1.0,
                                std::slice::from_ref(elem),
                                &[damage],
                            );
                            frame.clear_tex_program_override();
                        }
                    }

                    if let Some(shadow) = shadow_pipeline.as_ref() {
                        let _ = frame.with_context(|gl| {
                            for rect in &spotlight_shadow_rects {
                                shadow.render_shadow(gl, &projection, *rect, 26.0, true, 1.0);
                            }
                        });
                    }

                    // 3. Renderizar menús contextuales y popups de ventanas (siempre ENCIMA de las superficies de ventana)
                    let menu_radius = super::POPUP_MENU_CORNER_RADIUS_LOGICAL.round() as i32;
                    for (idx, (_, _, _, popup_shadow_rects, _, is_active, ..)) in
                        window_render_data.iter().enumerate()
                    {
                        if app_switcher_deck_frame.is_some() || body_occluded[idx] {
                            continue;
                        }
                        if let (Some(radius), Some(shadow)) = (win_phys_r, shadow_pipeline.as_ref())
                        {
                            for popup_rect in popup_shadow_rects {
                                let _ = frame.with_context(|gl| {
                                    shadow.render_shadow(
                                        gl,
                                        &projection,
                                        *popup_rect,
                                        radius as f32,
                                        *is_active,
                                        1.0,
                                    );
                                });
                            }
                        }
                    }

                    for (idx, (_, _, popups, ..)) in window_render_data.iter().enumerate() {
                        if app_switcher_deck_frame.is_some() || body_occluded[idx] {
                            continue;
                        }
                        for (popup_elems, popup_rect, popup_effects_enabled) in popups {
                            if *popup_effects_enabled {
                                if let Some(blur) = blur_pipeline.as_mut() {
                                    let _ = frame.with_context(|gl| {
                                        blur.render_blur(
                                            gl,
                                            &projection,
                                            size,
                                            *popup_rect,
                                            menu_radius as f32,
                                        );
                                    });
                                }
                            }

                            for elem in popup_elems.iter().rev() {
                                if *popup_effects_enabled {
                                    if let Some(shader) = window_squircle_shader.as_ref() {
                                        let elem_geo = elem.geometry(Scale::from(1.0));
                                        shader.apply_to_frame(
                                            &mut frame,
                                            elem_geo.size,
                                            popup_rect.loc - elem_geo.loc,
                                            popup_rect.size,
                                            menu_radius,
                                        );
                                    }
                                }
                                let _ = draw_render_elements(
                                    &mut frame,
                                    1.0,
                                    std::slice::from_ref(elem),
                                    &[damage],
                                );
                                if *popup_effects_enabled {
                                    frame.clear_tex_program_override();
                                }
                            }
                        }
                    }

                    // Previsualización de destino al pasar el cursor por las plantillas de Snap Layouts
                    if !is_pointer_grabbed {
                        if let Some(preview) = snap_layouts_preview {
                            crate::blur::render_snap_preview_overlay(
                                &mut frame,
                                blur_pipeline.as_mut(),
                                &projection,
                                size,
                                preview,
                                output_geo.loc,
                                1.0,
                                win_phys_r,
                            );
                        }
                    }

                    // 4. Renderizar capas superiores translúcidas (Dock, Top Bar, Popovers)
                    if !upper_elements.is_empty() {
                        for elem in &upper_elements {
                            let dock_rect = dock_clip_roots
                                .iter()
                                .find(|(root_id, _)| *elem.id() == *root_id)
                                .map(|(_, rect)| *rect);
                            if let (Some(rect), Some(shadow)) =
                                (dock_rect, shadow_pipeline.as_ref())
                            {
                                let _ = frame.with_context(|gl| {
                                    shadow.render_dock_shadow(
                                        gl,
                                        &projection,
                                        rect,
                                        32.0,
                                        1.0,
                                    );
                                });
                            }
                            if let Some(rect) = dock_rect {
                                let elem_geo = elem.geometry(Scale::from(1.0));
                                if let Some(shader) = window_squircle_shader.as_ref() {
                                    shader.apply_dock_clip_to_frame(
                                        &mut frame,
                                        elem_geo.size,
                                        rect.loc - elem_geo.loc,
                                        rect.size,
                                        32,
                                    );
                                }
                            }
                            let _ = draw_render_elements(
                                &mut frame,
                                1.0,
                                std::slice::from_ref(elem),
                                &[damage],
                            );
                            if dock_rect.is_some() {
                                frame.clear_tex_program_override();
                            }
                        }
                    }

                    for (popup_elems, popup_rect) in &dock_popups {
                        if let Some(blur) = blur_pipeline.as_mut() {
                            let blur_rect = super::dock_popover_blur_rect(*popup_rect, 1.0);
                            let _ = frame.with_context(|gl| {
                                blur.render_blur(
                                    gl,
                                    &projection,
                                    size,
                                    blur_rect,
                                    menu_radius as f32,
                                );
                            });
                        }

                        for elem in popup_elems.iter().rev() {
                            if let Some(shader) = window_squircle_shader.as_ref() {
                                let elem_geo = elem.geometry(Scale::from(1.0));
                                shader.apply_clip_to_frame(
                                    &mut frame,
                                    elem_geo.size,
                                    popup_rect.loc - elem_geo.loc,
                                    popup_rect.size,
                                    8,
                                );
                            }
                            let _ = draw_render_elements(
                                &mut frame,
                                1.0,
                                std::slice::from_ref(elem),
                                &[damage],
                            );
                            frame.clear_tex_program_override();
                        }
                    }

                    for (popup_elems, popup_rect) in &topbar_popups {
                        if let Some(blur) = blur_pipeline.as_mut() {
                            let _ = frame.with_context(|gl| {
                                blur.render_blur(
                                    gl,
                                    &projection,
                                    size,
                                    *popup_rect,
                                    menu_radius as f32,
                                );
                            });
                        }

                        if let Some(shadow) = shadow_pipeline.as_ref() {
                            let _ = frame.with_context(|gl| {
                                shadow.render_menu_shadow(
                                    gl,
                                    &projection,
                                    *popup_rect,
                                    menu_radius as f32,
                                    1.0,
                                );
                            });
                        }

                        for elem in popup_elems.iter().rev() {
                            if let Some(shader) = window_squircle_shader.as_ref() {
                                let elem_geo = elem.geometry(Scale::from(1.0));
                                shader.apply_to_frame(
                                    &mut frame,
                                    elem_geo.size,
                                    popup_rect.loc - elem_geo.loc,
                                    popup_rect.size,
                                    menu_radius,
                                );
                            }
                            let _ = draw_render_elements(
                                &mut frame,
                                1.0,
                                std::slice::from_ref(elem),
                                &[damage],
                            );
                            frame.clear_tex_program_override();
                        }
                    }

                    for (popup_elems, popup_rect) in &wallpaper_popups {
                        if let Some(blur) = blur_pipeline.as_mut() {
                            let _ = frame.with_context(|gl| {
                                blur.render_blur(
                                    gl,
                                    &projection,
                                    size,
                                    *popup_rect,
                                    menu_radius as f32,
                                );
                            });
                        }

                        if let Some(shadow) = shadow_pipeline.as_ref() {
                            let _ = frame.with_context(|gl| {
                                shadow.render_menu_shadow(
                                    gl,
                                    &projection,
                                    *popup_rect,
                                    menu_radius as f32,
                                    1.0,
                                );
                            });
                        }

                        for elem in popup_elems.iter().rev() {
                            if let Some(shader) = window_squircle_shader.as_ref() {
                                let elem_geo = elem.geometry(Scale::from(1.0));
                                shader.apply_to_frame(
                                    &mut frame,
                                    elem_geo.size,
                                    popup_rect.loc - elem_geo.loc,
                                    popup_rect.size,
                                    menu_radius,
                                );
                            }
                            let _ = draw_render_elements(
                                &mut frame,
                                1.0,
                                std::slice::from_ref(elem),
                                &[damage],
                            );
                            frame.clear_tex_program_override();
                        }
                    }

                    // 5. Renderizar Spotlight nativo del compositor (blur enmascarado por silueta exacta + capa Liquid Glass)
                    if let Some((
                        spot_rect,
                        ref mask_vec,
                        (mask_w, mask_h),
                        spot_alpha,
                        ref spot_elem,
                    )) = spotlight_frame
                    {
                        if let Some(blur) = blur_pipeline.as_mut() {
                            let _ = frame.with_context(|gl| {
                                blur.render_blur_masked(
                                    gl,
                                    &projection,
                                    size,
                                    spot_rect,
                                    mask_vec.as_deref(),
                                    mask_w,
                                    mask_h,
                                    spot_alpha,
                                );
                            });
                        }
                        let _ = draw_render_elements(
                            &mut frame,
                            1.0,
                            std::slice::from_ref(spot_elem),
                            &[damage],
                        );
                    }
                    if let Some(ref drag_elem) = spotlight_drag_elem {
                        let _ = draw_render_elements(
                            &mut frame,
                            1.0,
                            std::slice::from_ref(drag_elem),
                            &[damage],
                        );
                    }

                    // Renderizar Screenshot UI nativa con curvas squircle y Dual-Kawase blur
                    if let Some((
                        ui_rect,
                        ref mask_vec,
                        (mask_w, mask_h),
                        ui_alpha,
                        ref ui_elem,
                    )) = screenshot_ui_frame
                    {
                        if let Some(blur) = blur_pipeline.as_mut() {
                            let _ = frame.with_context(|gl| {
                                blur.render_blur_masked(
                                    gl,
                                    &projection,
                                    size,
                                    ui_rect,
                                    mask_vec.as_deref(),
                                    mask_w,
                                    mask_h,
                                    ui_alpha,
                                );
                            });
                        }
                        let _ = draw_render_elements(
                            &mut frame,
                            1.0,
                            std::slice::from_ref(ui_elem),
                            &[damage],
                        );
                    }

                    // Renderizar menú flotante Snap Layouts (hover sobre maximizar estilo Windows 11)
                    if let Some((
                        snap_rect,
                        ref mask_vec,
                        (mask_w, mask_h),
                        snap_alpha,
                        ref snap_elem,
                    )) = snap_layouts_frame
                    {
                        if let Some(shadow) = shadow_pipeline.as_ref() {
                            let _ = frame.with_context(|gl| {
                                shadow.render_menu_shadow_alpha(
                                    gl,
                                    &projection,
                                    snap_rect,
                                    crate::snap_layouts::POPUP_CORNER_RADIUS as f32,
                                    1.0,
                                    snap_alpha,
                                );
                            });
                        }
                        if let Some(blur) = blur_pipeline.as_mut() {
                            let _ = frame.with_context(|gl| {
                                blur.render_blur_masked(
                                    gl,
                                    &projection,
                                    size,
                                    snap_rect,
                                    mask_vec.as_deref(),
                                    mask_w,
                                    mask_h,
                                    snap_alpha,
                                );
                            });
                        }
                        let _ = draw_render_elements(
                            &mut frame,
                            1.0,
                            std::slice::from_ref(snap_elem),
                            &[damage],
                        );
                    }

                    // Renderizar Selector Visual App Switcher estilo Deck (sin blur global en el fondo)
                    if let Some((deck_eased, ref deck_cards)) = app_switcher_deck_frame {
                        crate::app_switcher::render_deck_frame(
                            &mut frame,
                            &projection,
                            size,
                            1.0,
                            &[damage],
                            blur_pipeline.as_mut(),
                            shadow_pipeline.as_ref(),
                            window_squircle_shader.as_ref(),
                            deck_eased,
                            deck_cards,
                        );
                    }

                    // Dibujar el icono de Drag and Drop (dnd_icon) si hay una operación activa
                    if !dnd_elems.is_empty() {
                        let _ = draw_render_elements(
                            &mut frame,
                            1.0,
                            &dnd_elems,
                            &[damage],
                        );
                    } else if let Some(ref fallback_elem) = dnd_fallback_elem {
                        let fallback_slice = std::slice::from_ref(fallback_elem);
                        let _ = draw_render_elements(
                            &mut frame,
                            1.0,
                            fallback_slice,
                            &[damage],
                        );
                    }

                    // Efecto Genie: malla GPU única sobre snapshot offscreen (incluyendo SSD y squircle).
                    if !active_genies.is_empty() {
                        if let Some(ref gp) = genie_pipeline {
                            let _ = frame.with_context(|gl| {
                                for anim in &active_genies {
                                    let Some(snap) = genie_snapshots.get(&anim.window_id) else {
                                        continue;
                                    };
                                    let elapsed = now.duration_since(anim.start_time);
                                    let raw_t = (elapsed.as_secs_f32()
                                        / anim.duration.as_secs_f32())
                                    .clamp(0.0, 1.0);
                                    let win_rect = crate::genie::GenieRect {
                                        x: (snap.win_rect_logical.loc.x - output_geo.loc.x) as f32,
                                        y: (snap.win_rect_logical.loc.y - output_geo.loc.y) as f32,
                                        width: (snap.win_rect_logical.size.w as f32).max(40.0),
                                        height: (snap.win_rect_logical.size.h as f32).max(40.0),
                                    };
                                    let target = crate::genie::GenieRect {
                                        x: (anim.dock_slot_rect.loc.x - output_geo.loc.x) as f32,
                                        y: (anim.dock_slot_rect.loc.y - output_geo.loc.y) as f32,
                                        width: anim.dock_slot_rect.size.w as f32,
                                        height: anim.dock_slot_rect.size.h as f32,
                                    };
                                    let (k, j, alpha) = crate::magic_lamp::MagicLampAnimation::new(
                                        win_rect,
                                        target,
                                        raw_t,
                                        anim.is_minimizing,
                                    )
                                    .params();
                                    let is_fullscreen_or_max = anim
                                        .window
                                        .toplevel()
                                        .map(|t| {
                                            let s = &t.current_state().states;
                                            s.contains(
                                                smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen,
                                            ) || s.contains(
                                                smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized,
                                            )
                                        })
                                        .unwrap_or(false);
                                    if let (Some(r), Some(ref shadow)) =
                                        (win_phys_r, shadow_pipeline.as_ref())
                                    {
                                        shadow.render_genie_shadow(
                                            gl,
                                            &projection,
                                            win_rect,
                                            r as f32,
                                            1.0,
                                            k,
                                            j,
                                            is_fullscreen_or_max,
                                        );
                                    }
                                    gp.render(
                                        gl,
                                        &projection,
                                        &snap.texture,
                                        win_rect,
                                        target,
                                        k,
                                        j,
                                        alpha,
                                    );
                                }
                            });
                        }
                    }

                    // 🕳️ EFECTO AGUJERO NEGRO AL CERRAR VENTANAS
                    if !active_black_holes.is_empty() {
                        if let Some(ref bh_pipeline) = black_hole_pipeline {
                            let bh_now = Instant::now();
                            let _ = frame.with_context(|gl| {
                                for bh in &active_black_holes {
                                    bh_pipeline.render_effect(gl, &projection, bh, bh_now);
                                }
                            });
                        }
                    }
                    }

                    // 🖥 ESQUINAS SQUIRCLE SUPERELÍPTICAS (n=4.2) DEL ESCRITORIO
                    if let Some(r) = phys_r {
                        if let Some(textures) = squircle_corner_cache.get(&r) {
                            let (w, h) = (size.w, size.h);
                            let corner_rects = [
                                Rectangle::new((0, 0).into(), (r, r).into()), // Superior Izquierda
                                Rectangle::new((w - r, 0).into(), (r, r).into()), // Superior Derecha
                                Rectangle::new((0, h - r).into(), (r, r).into()), // Inferior Izquierda
                                Rectangle::new((w - r, h - r).into(), (r, r).into()), // Inferior Derecha
                            ];

                            for (i, rect) in corner_rects.iter().enumerate() {
                                let _ = frame.render_texture_from_to(
                                    &textures[i],
                                    Rectangle::from_size(textures[i].size()).to_f64(),
                                    *rect,
                                    &[Rectangle::from_size(rect.size)],
                                    &[],
                                    Transform::Normal,
                                    1.0,
                                    None,
                                    &[],
                                );
                            }
                        }
                    }

                    let mut theme_capture_succeeded = false;
                    if theme_transition_pipeline.is_none() {
                        theme_transition_pipeline = frame
                            .with_context(crate::theme_transition::ThemeTransitionPipeline::new)
                            .ok()
                            .flatten();
                    }
                    if let Some(pipeline) = theme_transition_pipeline.as_mut() {
                        let output_geo = state.space.output_geometry(&output).unwrap_or_default();
                        let output_name = output.name();
                        let origin = theme_capture_request.as_ref().map(|request| {
                            let origin = crate::theme_transition::output_origin(
                                (request.origin.x, request.origin.y),
                                (output_geo.loc.x, output_geo.loc.y),
                                1.0,
                                size,
                            );
                            Point::from((size.w - origin.x, origin.y))
                        });
                        let result = frame.with_context(|gl| {
                            if let Some(origin) = origin {
                                theme_capture_succeeded = pipeline.capture_snapshot(
                                    gl,
                                    &output_name,
                                    size,
                                    false,
                                    origin,
                                );
                            }
                            if let Some(started_at) = state.theme_transition_started_at {
                                let now = Instant::now();
                                let drawn = pipeline.draw(gl, &output_name, size, started_at, now);
                                if !drawn && now.duration_since(started_at) >= crate::theme_transition::DURATION {
                                    state.theme_transition_started_at = None;
                                }
                            }
                        });
                        if let Err(error) = result {
                            warn!("No se pudo dibujar la transición de tema: {error:?}");
                        }
                    }

                    let mut ws_capture_succeeded = false;
                    if workspace_transition_pipeline.is_none() {
                        workspace_transition_pipeline = frame
                            .with_context(
                                crate::workspace_transition::WorkspaceTransitionPipeline::new,
                            )
                            .ok()
                            .flatten();
                    }
                    if let Some(ws_pipeline) = workspace_transition_pipeline.as_mut() {
                        let output_name = output.name();
                        let ws_result = frame.with_context(|gl| {
                            if let Some(mut ws_transition) = state.workspace_transition {
                                let now = Instant::now();
                                if ws_pipeline.draw(
                                    gl,
                                    &output_name,
                                    size,
                                    false,
                                    &mut ws_transition,
                                    now,
                                ) {
                                    state.workspace_transition = Some(ws_transition);
                                } else {
                                    state.workspace_transition = None;
                                }
                            }
                            if workspace_switch_pending {
                                ws_capture_succeeded =
                                    ws_pipeline.capture_outgoing(gl, &output_name, size, false);
                            }
                        });
                        if let Err(error) = ws_result {
                            warn!("No se pudo dibujar la transición de escritorio: {error:?}");
                        }
                    }

                    let _ = frame.finish();
                    if workspace_switch_pending {
                        state.commit_pending_workspace_switch(ws_capture_succeeded);
                    }
                    if let Some(request) = theme_capture_request {
                        state.pending_theme_transition_capture = None;
                        if theme_capture_succeeded {
                            state
                                .ipc_broadcaster
                                .broadcast(&IpcMessage::ThemeTransitionPrepared {
                                    request_id: request.request_id,
                                });
                        } else {
                            warn!("No se pudo guardar el fotograma para la transición de tema");
                        }
                    }

                    let mut finished = Vec::new();
                    active_black_holes.retain(|bh| !bh.is_finished(now));
                    state.active_animations.retain(|anim| {
                        if anim.waiting_for_dock_slot {
                            return true;
                        }
                        if now.duration_since(anim.start_time) >= anim.duration {
                            finished.push(anim.clone());
                            false
                        } else {
                            true
                        }
                    });
                    for anim in &finished {
                        genie_snapshots.remove(&anim.window_id);
                    }

                    for anim in finished {
                        if !anim.is_minimizing {
                            info!("✨ Restauración Genie completada: mapeando ventana a Space");
                            state.space.map_element(
                                anim.window.clone(),
                                anim.original_location,
                                true,
                            );
                            state.space.raise_element(&anim.window, true);
                            if let Some(toplevel) = anim.window.toplevel() {
                                toplevel.send_configure();
                            }
                            if let Some(kb) = state.seat.get_keyboard() {
                                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                                kb.set_focus(&mut state, Some(anim.surface.clone()), serial);
                            }
                            if let Some(pointer) = state.seat.get_pointer() {
                                let loc = pointer.current_location();
                                let target =
                                    state.space.element_under(loc).and_then(|(win, win_loc)| {
                                        let local_point = loc - win_loc.to_f64();
                                        win.surface_under(
                                            local_point,
                                            smithay::desktop::WindowSurfaceType::all(),
                                        )
                                        .map(|(s, sub_loc)| (s, (win_loc + sub_loc).to_f64()))
                                    });
                                let motion_serial = smithay::utils::SERIAL_COUNTER.next_serial();
                                pointer.motion(
                                    &mut state,
                                    target,
                                    &smithay::input::pointer::MotionEvent {
                                        location: loc,
                                        serial: motion_serial,
                                        time: 0,
                                    },
                                );
                                pointer.frame(&mut state);
                            }
                            state.notify_active_app(Some(&anim.surface));
                            state.notify_running_apps();
                        } else {
                            state.notify_minimized_windows();
                        }
                    }
                }
                if let Some(destination) = screenshot_request {
                    info!("Procesando solicitud de captura de pantalla");
                    if frame_rendered {
                        if let Err(error) = crate::capture::capture_frame(
                            renderer,
                            &framebuffer,
                            size,
                            &destination,
                            Transform::Flipped180,
                        ) {
                            warn!("Error al capturar el framebuffer: {error:#}");
                            crate::capture::write_capture_error(&destination, &error);
                        }
                    } else {
                        let error =
                            anyhow::anyhow!("No se pudo renderizar el fotograma de captura");
                        crate::capture::write_capture_error(&destination, &error);
                    }
                }

                if !session_locked {
                if let Some(ref mut session) = state.active_screen_recording {
                    let interval = std::time::Duration::from_millis(
                        (1000 / session.target_fps.max(1)) as u64,
                    );
                    if now.duration_since(session.last_frame) >= interval {
                        session.last_frame = now;
                        let crop = session.rect.unwrap_or_else(|| {
                            Rectangle::from_size(size)
                        });
                        let cl_x = crop.loc.x.clamp(0, size.w);
                        let cl_y = crop.loc.y.clamp(0, size.h);
                        let cl_w = (crop.size.w.min(size.w - cl_x) & !1).max(2);
                        let cl_h = (crop.size.h.min(size.h - cl_y) & !1).max(2);
                        let region = Rectangle::new((cl_x, cl_y).into(), (cl_w, cl_h).into());

                        if let Ok(mapping) = renderer.copy_framebuffer(&framebuffer, region, smithay::backend::allocator::Fourcc::Abgr8888) {
                            if let Ok(pixels) = renderer.map_texture(&mapping) {
                                if pixels.len() == (cl_w as usize * cl_h as usize * 4) {
                                    session.push_frame(pixels.to_vec());
                                }
                            }
                        }
                    }
                }
                }
            }
        }

        // Limpiar transiciones de maximizado/restauración finalizadas
        let now = Instant::now();
        state.snap_transitions.retain(|_, trans| !trans.is_finished(now));

        // Notificar fotogramas a los clientes de ventanas activas (evita despertar apps minimizadas u ocluidas a 60 Hz)
        let now_ms = start_time.elapsed().as_millis() as u32;
        for window in state.space.elements() {
            if let Some(surface) = window.wl_surface() {
                let sid = smithay::backend::renderer::element::Id::from_wayland_resource(
                    surface.as_ref(),
                );
                if !visible_surface_ids.contains(&sid) {
                    continue;
                }
                send_frames_surface_tree(surface.as_ref(), now_ms);
                for (popup, _) in
                    smithay::desktop::PopupManager::popups_for_surface(surface.as_ref())
                {
                    send_frames_surface_tree(popup.wl_surface(), now_ms);
                }
            }
        }

        {
            let layer_map = smithay::desktop::layer_map_for_output(&output);
            for layer in layer_map.layers() {
                send_frames_surface_tree(layer.wl_surface(), now_ms);
                for (popup, _) in
                    smithay::desktop::PopupManager::popups_for_surface(layer.wl_surface())
                {
                    send_frames_surface_tree(popup.wl_surface(), now_ms);
                }
            }
            if let Some(ext_ls) = state.authorized_ext_lock_surface_for_output(&output.name()) {
                send_frames_surface_tree(ext_ls.wl_surface(), now_ms);
            }
        }

        // Aceptar nuevos clientes de forma no bloqueante
        while let Ok(Some(stream)) = listener.accept() {
            match display
                .handle()
                .insert_client(stream, Arc::new(ClientState::default()))
            {
                Ok(_client) => {
                    info!("🎉 Nuevo cliente Wayland conectado exitosamente al compositor!");
                }
                Err(err) => warn!("Error al registrar nuevo cliente Wayland: {:?}", err),
            }
        }

        state.process_ipc_messages();
        display.dispatch_clients(&mut state)?;
        state.process_pending_dmabuf_imports(backend.renderer());
        state.space.refresh();
        for output in state.space.outputs() {
            let mut map = smithay::desktop::layer_map_for_output(output);
            map.cleanup();
        }
        state.popups.cleanup();
        state.active_popups.retain(|p| p.alive());
        state.cleanup_stale_surfaces();
        display.flush_clients()?;

        let _ = backend.submit(Some(&[damage]));

        let refresh_millihz = output
            .current_mode()
            .map(|mode| mode.refresh)
            .unwrap_or(60_000);
        let remaining =
            super::frame_interval(refresh_millihz).saturating_sub(frame_start.elapsed());
        if !remaining.is_zero() {
            std::thread::sleep(remaining);
        }
    }

    info!("Compositor Malus DE finalizado limpiamente.");
    Ok(())
}

pub fn toggle_spotlight(_socket_name: &str) {
    let _ = malus_de_common::ipc::IpcClient::send_command(
        malus_de_common::ipc::IpcMessage::ToggleSpotlight,
    );
}
