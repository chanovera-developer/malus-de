use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use anyhow::{Context, Result};
use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, Axis, ButtonState, Event, InputEvent, KeyState, KeyboardKeyEvent,
            PointerAxisEvent, PointerButtonEvent,
        },
        renderer::{
            gles::GlesRenderer,
            utils::draw_render_elements,
            element::{
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement, WaylandSurfaceTexture},
                Element, Kind,
            },
            Color32F, Frame, Renderer, Texture,
        },
        winit::{self, WinitEvent},
    },

    input::{
        keyboard::{keysyms, FilterResult},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        wayland_server::{
            protocol::wl_surface::WlSurface,
            Display, ListeningSocket,
        },
        winit::platform::pump_events::PumpStatus,
    },
    utils::{Logical, Physical, Point, Rectangle, Scale, Transform, SERIAL_COUNTER},
    wayland::{
        compositor::{with_surface_tree_downward, SurfaceAttributes, TraversalAction},
        seat::WaylandFocus,
        shell::wlr_layer::Layer,
    },
};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::{ResizeEdge, State as XdgState};
use tracing::{info, warn};
use malus_de_common::ipc::IpcMessage;

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
            !(layer.namespace() == "malus-app-switcher" && !state.app_switcher.is_active)
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

pub fn run_winit() -> Result<()> {
    info!("Inicializando Display Wayland de Malus DE...");
    let mut display: Display<MalusState> = Display::new().context("Error al crear Display Wayland")?;
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
            warn!("⚠️ Falló carga de XKB layout '{}': {:?}. Usando fallback US.", input_cfg.keyboard_layout, e);
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
    info!("Socket Wayland escuchando activamente en: WAYLAND_DISPLAY={}", socket_name);
    state.socket_name = socket_name.clone();
    let _clipboard_manager = crate::start_clipboard_manager(&socket_name);

    // Inicializar backend gráfico de Winit en ventana anidada
    let (mut backend, mut winit_event_loop) = winit::init::<GlesRenderer>()
        .map_err(|e| anyhow::anyhow!("Error al inicializar WinitGraphicsBackend: {e:?}"))?;

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
    output.change_current_state(Some(mode), Some(Transform::Normal), None, Some((0, 0).into()));
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));

    info!("============================================================");
    info!("✨ MALUS DE: Compositor Wayland Smithay (Modo Anidado Activo)");
    info!("   Socket activo: WAYLAND_DISPLAY={}", socket_name);
    info!("   Prueba lanzar una app cliente en otra terminal:");
    info!("   WAYLAND_DISPLAY={} GDK_BACKEND=wayland gnome-calculator", socket_name);
    info!("============================================================");

    // Color acrílico oscuro de Malus DE (#18181b = rgba(24, 24, 27, 1.0))
    let dark_acrylic = Color32F::new(0.094, 0.094, 0.106, 1.0);
    let start_time = Instant::now();
    let mut clients = Vec::new();
    let mut current_displays_cfg = malus_de_common::config::DisplaysConfig::load();
    let mut squircle_corner_cache: std::collections::HashMap<i32, [smithay::backend::renderer::gles::GlesTexture; 4]> = std::collections::HashMap::new();
    let mut blur_pipeline: Option<crate::blur::DualKawaseBlur> = None;
    let mut window_squircle_shader: Option<crate::window_corners::WindowSquircleShader> = None;
    let mut shadow_pipeline: Option<crate::shadow::WindowShadowPipeline> = None;

    while state.is_running {
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
                            let has_alt = modifiers.alt;
                            let has_ctrl = modifiers.ctrl;

                            let is_space = sym == keysyms::KEY_space.into();
                            let is_k = sym == keysyms::KEY_k.into() || sym == keysyms::KEY_K.into();
                            let is_v = sym == keysyms::KEY_v.into() || sym == keysyms::KEY_V.into();
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
                                if is_v && has_super {
                                    state
                                        .ipc_broadcaster
                                        .broadcast(&IpcMessage::ToggleClipboardHistory);
                                    return FilterResult::Intercept(());
                                }

                                // 1. Atajos para alternar Spotlight:
                                // Super + Espacio (macOS nativo), Alt + Espacio, Ctrl + Espacio, Super + K
                                if (is_space && (has_super || has_alt || has_ctrl)) || (is_k && has_super) {
                                    info!("🔍 Atajo de Spotlight detectado (Super={}, Alt={}, Ctrl={}): alternando...",
                                        has_super, has_alt, has_ctrl);
                                    toggle_spotlight(&state.socket_name);
                                    return FilterResult::Intercept(());
                                }

                                // 2. Navegación en el App Switcher (Selector Visual ⌘Tab) mientras está activo
                                if state.app_switcher.is_active {
                                    if is_return {
                                        info!("🎯 AppSwitcher commit vía Return/Enter, enfocando ventana");
                                        state.app_switcher.is_active = false;
                                        state.ipc_broadcaster.broadcast(&IpcMessage::HideAppSwitcher);
                                        if !state.app_switcher.items.is_empty() {
                                            let target_id = state.app_switcher.items[state.app_switcher.selected_index].window_id;
                                            state.switch_to_window(target_id);
                                        }
                                        return FilterResult::Intercept(());
                                    } else if is_tab {
                                        let has_shift = modifiers.shift;
                                        let len = state.app_switcher.items.len();
                                        if len > 0 {
                                            if has_shift {
                                                state.app_switcher.selected_index = (state.app_switcher.selected_index + len - 1) % len;
                                            } else {
                                                state.app_switcher.selected_index = (state.app_switcher.selected_index + 1) % len;
                                            }
                                            info!("🔀 AppSwitcher ciclo: idx={}/{}", state.app_switcher.selected_index, len);
                                            state.ipc_broadcaster.broadcast(&IpcMessage::UpdateAppSwitcher {
                                                selected_index: state.app_switcher.selected_index,
                                            });
                                        }
                                        return FilterResult::Intercept(());
                                    } else if is_left {
                                        let len = state.app_switcher.items.len();
                                        if len > 0 {
                                            state.app_switcher.selected_index = (state.app_switcher.selected_index + len - 1) % len;
                                            state.ipc_broadcaster.broadcast(&IpcMessage::UpdateAppSwitcher {
                                                selected_index: state.app_switcher.selected_index,
                                            });
                                        }
                                        return FilterResult::Intercept(());
                                    } else if is_right {
                                        let len = state.app_switcher.items.len();
                                        if len > 0 {
                                            state.app_switcher.selected_index = (state.app_switcher.selected_index + 1) % len;
                                            state.ipc_broadcaster.broadcast(&IpcMessage::UpdateAppSwitcher {
                                                selected_index: state.app_switcher.selected_index,
                                            });
                                        }
                                        return FilterResult::Intercept(());
                                    } else if is_q {
                                        // Atajo secreto de macOS: ⌘Q mientras ⌘Tab está activo cierra la app seleccionada
                                        if !state.app_switcher.items.is_empty() {
                                            let target_id = state.app_switcher.items[state.app_switcher.selected_index].window_id;
                                            info!("🚪 AppSwitcher ⌘Q: cerrando ventana seleccionada id={}", target_id);
                                            state.close_window_by_id(target_id);
                                            state.app_switcher.items.remove(state.app_switcher.selected_index);
                                            if state.app_switcher.items.is_empty() {
                                                state.app_switcher.is_active = false;
                                                state.ipc_broadcaster.broadcast(&IpcMessage::HideAppSwitcher);
                                            } else {
                                                state.app_switcher.selected_index = state.app_switcher.selected_index.min(state.app_switcher.items.len() - 1);
                                                state.ipc_broadcaster.broadcast(&IpcMessage::ShowAppSwitcher {
                                                    items: state.app_switcher.items.clone(),
                                                    selected_index: state.app_switcher.selected_index,
                                                });
                                            }
                                        }
                                        return FilterResult::Intercept(());
                                    } else if is_h {
                                        // Atajo secreto de macOS: ⌘H mientras ⌘Tab está activo oculta/minimiza la app seleccionada
                                        if !state.app_switcher.items.is_empty() {
                                            let target_id = state.app_switcher.items[state.app_switcher.selected_index].window_id;
                                            info!("🪟 AppSwitcher ⌘H: ocultando ventana seleccionada id={}", target_id);
                                            state.hide_window_by_id(target_id);
                                            state.app_switcher.items[state.app_switcher.selected_index].is_minimized = true;
                                            state.ipc_broadcaster.broadcast(&IpcMessage::ShowAppSwitcher {
                                                items: state.app_switcher.items.clone(),
                                                selected_index: state.app_switcher.selected_index,
                                            });
                                        }
                                        return FilterResult::Intercept(());
                                    } else if is_esc {
                                        info!("❌ AppSwitcher cancelado con Escape");
                                        state.app_switcher.is_active = false;
                                        state.ipc_broadcaster.broadcast(&IpcMessage::HideAppSwitcher);
                                        return FilterResult::Intercept(());
                                    }
                                }

                                // 3. Invocación inicial de ⌘Tab / Super+Tab / Alt+Tab
                                if (has_super || has_alt) && is_tab {
                                    let items = state.build_app_switcher_items();
                                    if !items.is_empty() {
                                        let has_shift = modifiers.shift;
                                        let initial_idx = if items.len() > 1 {
                                            if has_shift { items.len() - 1 } else { 1 }
                                        } else {
                                            0
                                        };
                                        state.app_switcher.is_active = true;
                                        state.app_switcher.items = items.clone();
                                        state.app_switcher.selected_index = initial_idx;
                                        info!("🔀 AppSwitcher activado con {} ventanas (idx={})", items.len(), initial_idx);
                                        state.ipc_broadcaster.broadcast(&IpcMessage::ShowAppSwitcher {
                                            items,
                                            selected_index: initial_idx,
                                        });
                                        return FilterResult::Intercept(());
                                    }
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
                                    state.app_switcher.is_active = false;
                                    state.ipc_broadcaster.broadcast(&IpcMessage::HideAppSwitcher);
                                    if !state.app_switcher.items.is_empty() {
                                        let target_id = state.app_switcher.items[state.app_switcher.selected_index].window_id;
                                        state.switch_to_window(target_id);
                                    }
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
                    let serial = SERIAL_COUNTER.next_serial();

                    let layer_target = {
                        let layer_map = smithay::desktop::layer_map_for_output(&output);
                        let under_layer = layer_map.layer_under(Layer::Overlay, point)
                            .filter(|l| !(l.namespace() == "malus-app-switcher" && !state.app_switcher.is_active))
                            .or_else(|| layer_map.layer_under(Layer::Top, point));

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

                    let under_target = if let Some((surf, focus_loc)) = layer_target {
                        Some((surf, focus_loc))
                    } else if let Some((window, loc)) = state.space.element_under(point) {
                        let local_point = point - loc.to_f64();
                        window
                            .surface_under(local_point, smithay::desktop::WindowSurfaceType::all())
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
                }
                InputEvent::PointerButton { event } => {
                    let is_pressed = event.state() == ButtonState::Pressed;
                    let point = pointer.current_location();

                    let layer_target = {
                        let layer_map = smithay::desktop::layer_map_for_output(&output);
                        let under_layer = layer_map.layer_under(Layer::Overlay, point)
                            .filter(|l| !(l.namespace() == "malus-app-switcher" && !state.app_switcher.is_active))
                            .or_else(|| layer_map.layer_under(Layer::Top, point));

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

                    let (under_target, layer_info, regular_win) = if let Some((surf, focus_loc, ns, can_kb)) = layer_target {
                        (Some((surf.clone(), focus_loc)), Some((surf, ns, can_kb)), None)
                    } else if let Some((window, loc)) = state.space.element_under(point) {
                        let local_point = point - loc.to_f64();
                        let target = window
                            .surface_under(local_point, smithay::desktop::WindowSurfaceType::all())
                            .map(|(surf, sub_loc)| (surf, (loc + sub_loc).to_f64()));
                        (target, None, Some((window.clone(), loc)))
                    } else if let Some((surf, focus_loc, ns, can_kb)) =
                        find_background_layer_under(&state, &output, point)
                    {
                        (
                            Some((surf.clone(), focus_loc)),
                            Some((surf, ns, can_kb)),
                            None,
                        )
                    } else {
                        (None, None, None)
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
                        if state.app_switcher.is_active {
                            info!("🖱️ Click detectado mientras AppSwitcher está activo -> cerrando AppSwitcher");
                            state.app_switcher.is_active = false;
                            state.ipc_broadcaster.broadcast(&IpcMessage::HideAppSwitcher);
                        }

                        // Si hay popups activos y el clic NO fue sobre el popup, descartamos los popups
                        let clicked_on_popup = under_target
                            .as_ref()
                            .map(|(s, _)| {
                                state.active_popups.iter().any(|p| p.wl_surface() == s)
                                    || state.popups.find_popup(s).is_some()
                            })
                            .unwrap_or(false);

                        if !clicked_on_popup && !state.active_popups.is_empty() {
                            state.dismiss_all_popups();
                        }

                        // Liberar cualquier grab residual para que los clics nunca se queden congelados
                        if pointer.is_grabbed() {
                            tracing::warn!("⚠️ Grab de puntero residual detectado al hacer clic; liberando grab");
                            state.snap_preview = None;
                            pointer.unset_grab(&mut state, btn_serial, event.time_msec());
                        }
                        if keyboard.is_grabbed() {
                            tracing::warn!("⚠️ Grab de teclado residual detectado al hacer clic; liberando grab");
                            keyboard.unset_grab(&mut state);
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
                                    keyboard.set_focus(&mut state, Some(surface.clone().into_owned()), btn_serial);
                                    state.notify_active_app(Some(&surface));
                                }
                            }

                            // Si Super o Alt está presionado y es clic izquierdo (0x110 = BTN_LEFT): arrastrar ventana inmediatamente desde cualquier punto
                            let has_super = keyboard.modifier_state().logo;
                            let has_alt = keyboard.modifier_state().alt;

                            // Detección perimetral de redimensión del lado del servidor (8 px) con Clic Izquierdo (0x110 = BTN_LEFT)
                            // APLICADO EXCLUSIVAMENTE A FIREFOX (para no alterar umbrales ni controles en ventanas GTK/Electron)
                            if event.button_code() == 0x110 && !has_super && !has_alt {
                                if let Some((resize_win, edge, win_loc)) = crate::grabs::find_window_resize_edge_under(&state.space, point, 8.0) {
                                    let was_already_top = state.space.elements().last().map(|w| w == &resize_win).unwrap_or(false);
                                    if !was_already_top {
                                        state.space.raise_element(&resize_win, true);
                                    }
                                    if let Some(surface) = resize_win.wl_surface() {
                                        if keyboard.current_focus().as_ref() != Some(surface.as_ref()) {
                                            keyboard.set_focus(&mut state, Some(surface.clone().into_owned()), btn_serial);
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
                                    pointer.set_grab(&mut state, grab, btn_serial, smithay::input::pointer::Focus::Clear);
                                    info!("📐 [ServerResize] Redimensión perimetral activada para Firefox: edge={:?}, win={:?}", edge, resize_win.geometry());
                                }
                            }

                            if (has_super || has_alt) && event.button_code() == 0x110 {
                                info!("🚀 Atajo Super/Alt + Clic: iniciando arrastre directo de ventana desde {:?}", loc);
                                let initial_window_location = loc;
                                let start_data = smithay::input::pointer::GrabStartData {
                                    focus: None,
                                    button: event.button_code(),
                                    location: point,
                                };
                                let grab = crate::grabs::MoveSurfaceGrab {
                                    start_data,
                                    window: window.clone(),
                                    initial_window_location,
                                };
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
                                pointer.set_grab(&mut state, grab, btn_serial, smithay::input::pointer::Focus::Clear);
                            }
                        } else {
                            info!("🖱️ Clic en espacio libre del escritorio: button={:#x}, pos={:?}", event.button_code(), point);
                            state.notify_active_app(None);
                        }
                    } else {
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
                }
                InputEvent::PointerAxis { event } => {
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
        {
            if let Ok((renderer, mut framebuffer)) = backend.bind() {

                let now = Instant::now();
                let mut anim_data = Vec::new();
                for anim in &state.active_animations {
                    let elems = render_elements_from_surface_tree::<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>(
                        renderer,
                        &anim.surface,
                        (0, 0),
                        Scale::from(1.0),
                        1.0,
                        Kind::Unspecified,
                    );
                    if elems.is_empty() {
                        warn!("⚠️ [Genie] anim_data: elems está VACÍO para anim id={}", anim.window_id);
                    }
                    anim_data.push((anim.clone(), elems));
                }

                if state.pending_displays_reload {
                    state.pending_displays_reload = false;
                    current_displays_cfg = malus_de_common::config::DisplaysConfig::load();
                    squircle_corner_cache.clear();
                }

                let corner_radius_pt = current_displays_cfg.screen_corner_radius;
                let phys_r = if corner_radius_pt > 0.0 {
                    let r = corner_radius_pt.round() as i32;
                    if r > 2 {
                        if !squircle_corner_cache.contains_key(&r) {
                            if let Some(textures) = super::drm::create_squircle_corner_textures(renderer, r) {
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
                    if r > 2 { Some(r) } else { None }
                } else {
                    None
                };

                let layer_map = smithay::desktop::layer_map_for_output(&output);
                let mut lower_elements = Vec::new();
                let mut upper_elements = Vec::new();
                let mut dock_clip_roots = Vec::new();
                let mut topbar_popups = Vec::new();
                for surface in layer_map.layers().rev() {
                    if surface.namespace() == "malus-app-switcher" && !state.app_switcher.is_active {
                        continue;
                    }
                    if let Some(geo) = layer_map.layer_geometry(surface) {
                        let layer_loc = geo.loc.to_physical_precise_round(1.0);
                        let s_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = render_elements_from_surface_tree(
                            renderer,
                            surface.wl_surface(),
                            layer_loc,
                            smithay::utils::Scale::from(1.0),
                            1.0,
                            Kind::Unspecified,
                        );
                        if surface.namespace() == "malus-dock" {
                            let root_id =
                                smithay::backend::renderer::element::Id::from_wayland_resource(
                                    surface.wl_surface(),
                                );
                            if let Some(root_elem) = s_elems.iter().find(|elem| *elem.id() == root_id) {
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
                        let mut p_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();
                        for (popup, popup_offset) in smithay::desktop::PopupManager::popups_for_surface(surface.wl_surface()) {
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
                                let popup_rect = Rectangle::new(
                                    layer_loc + popup_offset.to_physical_precise_round(1.0),
                                    popup_geometry.size.to_physical_precise_round(1.0),
                                );
                                topbar_popups.push((elems, popup_rect));
                            } else {
                                p_elems.extend(elems);
                            }
                        }

                        if matches!(surface.layer(), Layer::Background | Layer::Bottom) {
                            lower_elements.extend(s_elems);
                            lower_elements.extend(p_elems);
                        } else {
                            upper_elements.extend(s_elems);
                            upper_elements.extend(p_elems);
                        }
                    }
                }

                let mut window_render_data = Vec::new();
                let output_geo = state.space.output_geometry(&output).unwrap_or_else(|| Rectangle::new((0, 0).into(), (size.w, size.h).into()));
                for window in state.space.elements() {
                    if let Some(loc) = state.space.element_location(window) {
                        let geo = window.geometry();
                        let win_global_rect = Rectangle::new(loc, geo.size);
                        if output_geo.overlaps(win_global_rect) {
                            let phys_loc = (loc - geo.loc - output_geo.loc).to_physical_precise_round(1.0);
                            let Some(main_surface) = window.wl_surface() else {
                                continue;
                            };

                            let win_surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = render_elements_from_surface_tree(
                                renderer,
                                main_surface.as_ref(),
                                phys_loc,
                                smithay::utils::Scale::from(1.0),
                                1.0,
                                Kind::Unspecified,
                            );

                            let mut popup_elems: Vec<(
                                Vec<WaylandSurfaceRenderElement<GlesRenderer>>,
                                Rectangle<i32, Physical>,
                            )> = Vec::new();
                            let mut popup_shadow_rects = Vec::new();
                            for (popup, popup_offset) in smithay::desktop::PopupManager::popups_for_surface(main_surface.as_ref()) {
                                let popup_geometry = popup.geometry();
                                let popup_rect = Rectangle::new(
                                    phys_loc + (geo.loc + popup_offset).to_physical_precise_round(1.0),
                                    popup_geometry.size.to_physical_precise_round(1.0),
                                );
                                popup_shadow_rects.push(popup_rect);
                                let offset = (geo.loc + popup_offset - popup_geometry.loc).to_physical_precise_round(1.0);
                                let p_elems = render_elements_from_surface_tree(
                                    renderer,
                                    popup.wl_surface(),
                                    phys_loc + offset,
                                    smithay::utils::Scale::from(1.0),
                                    1.0,
                                    Kind::Unspecified,
                                );
                                popup_elems.push((p_elems, popup_rect));
                            }

                            let is_fullscreen = window.toplevel().map(|t| {
                                let states = &t.current_state().states;
                                states.contains(XdgState::Fullscreen)
                            }).unwrap_or(false);

                            let is_active = window.toplevel().map(|t| {
                                let states = &t.current_state().states;
                                states.contains(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated)
                            }).unwrap_or(false) || (state.seat.get_keyboard().and_then(|k| k.current_focus()).as_ref() == Some(main_surface.as_ref()));

                            let is_firefox = crate::state::is_firefox_window(&window);
                            let is_translucent = crate::state::is_translucent_window(&window);
                            let root_id = smithay::backend::renderer::element::Id::from_wayland_resource(main_surface.as_ref());
                            let should_round = win_phys_r.is_some() && !is_fullscreen;
                            let content_offset = geo.loc.to_physical_precise_round(1.0);
                            let content_size = geo.size.to_physical_precise_round(1.0);
                            let win_screen_rect = Rectangle::new(
                                (loc - output_geo.loc).to_physical_precise_round(1.0),
                                geo.size.to_physical_precise_round(1.0),
                            );

                            window_render_data.push((root_id, win_surfaces, popup_elems, popup_shadow_rects, should_round, is_active, win_screen_rect, content_offset, content_size, is_firefox, is_translucent));
                        }
                    }
                }

                // Inicializar shader de recorte squircle para ventanas si aún no existe
                if window_squircle_shader.is_none() {
                    window_squircle_shader = crate::window_corners::WindowSquircleShader::new(renderer);
                }

                let screenshot_request = state.pending_screenshot_requests.pop_front();
                let mut frame_rendered = false;
                if let Ok(mut frame) = renderer.render(&mut framebuffer, size, Transform::Flipped180) {
                    frame_rendered = true;
                    let _ = frame.clear(dark_acrylic, &[damage]);
                    // 1. Renderizar fondo (wallpaper y capas inferiores)
                    if !lower_elements.is_empty() {
                        let _ = draw_render_elements(&mut frame, 1.0, &lower_elements, &[damage]);
                    }

                    // Inicializar pipeline de sombras squircle si aún no existe
                    if shadow_pipeline.is_none() {
                        shadow_pipeline = frame.with_context(|gl| crate::shadow::WindowShadowPipeline::new(gl)).ok().flatten();
                    }

                    // Inicializar pipeline de desenfoque Dual Kawase si aún no existe
                    if blur_pipeline.is_none() {
                        blur_pipeline = frame.with_context(|gl| crate::blur::DualKawaseBlur::new(gl)).ok().flatten();
                    }

                    let projection = *frame.projection();

                    // 2. Renderizar superficies de cada ventana con sus sombras proyectadas, desenfoque de fondo y esquinas squircle
                    for (root_id, win_elems, _popups, _popup_shadow_rects, should_round, is_active, win_rect, content_offset, content_size, is_firefox, is_translucent) in &window_render_data {
                        if win_elems.is_empty() {
                            continue;
                        }

                        // 2.1. SOMBRA PROYECTADA (Window Drop Shadow con Squircle SDF n=4.2)
                        // Se dibuja para ventanas no en pantalla completa inmediatamente antes de su superficie
                        if *should_round {
                            if let (Some(r), Some(ref shadow)) = (win_phys_r, shadow_pipeline.as_ref()) {
                                let _ = frame.with_context(|gl| {
                                    shadow.render_shadow(gl, &projection, *win_rect, r as f32, *is_active, 1.0);
                                });
                            }

                            // 2.1.5. DESENFOQUE DE FONDO DUAL KAWASE (Exclusivo para Nautilus y Ajustes del Sistema)
                            if *is_translucent {
                                if let (Some(r), Some(ref mut blur)) = (win_phys_r, blur_pipeline.as_mut()) {
                                    let screen_size = size;
                                    let _ = frame.with_context(|gl| {
                                        blur.render_blur(gl, &projection, screen_size, *win_rect, r as f32);
                                    });
                                }
                            }
                        }

                        // 2.2. SUPERFICIES DE LA VENTANA (con recorte Squircle)
                        if *should_round {
                            if *is_firefox {
                                // Para Firefox: se aplica el shader a todo el árbol de subsuperficies (WebRender)
                                // para evitar esquinas cuadradas en el contenido web interno
                                if let (Some(r), Some(ref shader)) = (win_phys_r, window_squircle_shader.as_ref()) {
                                    for elem in win_elems {
                                        let elem_geo = elem.geometry(smithay::utils::Scale::from(1.0));
                                        let sub_offset = smithay::utils::Point::from((
                                            win_rect.loc.x - elem_geo.loc.x,
                                            win_rect.loc.y - elem_geo.loc.y,
                                        ));
                                        shader.apply_to_frame(&mut frame, elem_geo.size, sub_offset, win_rect.size, r);

                                        let _ = draw_render_elements(&mut frame, 1.0, std::slice::from_ref(elem), &[damage]);

                                        frame.clear_tex_program_override();
                                    }
                                } else {
                                    let _ = draw_render_elements(&mut frame, 1.0, win_elems, &[damage]);
                                }
                            } else {
                                // Para las demás apps (GTK, Electron, etc.): comportamiento intacto original,
                                // SOLO se aplica el recorte a la superficie raíz para no distorsionar widgets GTK
                                for elem in win_elems {
                                    let is_root = elem.id() == root_id;
                                    if is_root {
                                        if let (Some(r), Some(ref shader)) = (win_phys_r, window_squircle_shader.as_ref()) {
                                            let elem_geo = elem.geometry(smithay::utils::Scale::from(1.0));
                                            shader.apply_to_frame(&mut frame, elem_geo.size, *content_offset, *content_size, r);
                                        }
                                    }

                                    let _ = draw_render_elements(&mut frame, 1.0, std::slice::from_ref(elem), &[damage]);

                                    if is_root {
                                        frame.clear_tex_program_override();
                                    }
                                }
                            }
                        } else {
                            let _ = draw_render_elements(&mut frame, 1.0, win_elems, &[damage]);
                        }
                    }

                    // 2. Aplicar desenfoque de fondo Dual Kawase por GPU a las capas y popups translúcidos
                    if blur_pipeline.is_none() {
                        blur_pipeline = frame.with_context(|gl| crate::blur::DualKawaseBlur::new(gl)).ok().flatten();
                    }

                    if let Some(ref mut blur) = blur_pipeline {
                        let screen_size = size;
                        let _ = frame.with_context(|gl| {
                            for surface in layer_map.layers() {
                                if matches!(surface.layer(), Layer::Top | Layer::Overlay) {
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
                                            "malus-spotlight" => 18.0,
                                            "malus-notifications" => 16.0,
                                            _ => 0.0,
                                        };
                                        if radius > 0.0 {
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
                                                geo.to_physical_precise_round(1.0)
                                            };
                                            blur.render_blur(gl, &projection, screen_size, phys_rect, radius);
                                        }
                                    }
                                }
                            }
                        });
                    }

                    // 3. Renderizar menús contextuales y popups de ventanas (siempre ENCIMA de las superficies de ventana)
                    let menu_radius = 22;
                    for (_, _, _, popup_shadow_rects, _, is_active, ..) in &window_render_data {
                        if let (Some(radius), Some(shadow)) = (win_phys_r, shadow_pipeline.as_ref()) {
                            for popup_rect in popup_shadow_rects {
                                let _ = frame.with_context(|gl| {
                                    shadow.render_shadow(gl, &projection, *popup_rect, radius as f32, *is_active, 1.0);
                                });
                            }
                        }
                    }

                    for (_, _, popups, ..) in &window_render_data {
                        for (popup_elems, popup_rect) in popups {
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

                            for elem in popup_elems {
                                if let Some(shader) = window_squircle_shader.as_ref() {
                                    let elem_geo = elem.geometry(Scale::from(1.0));
                                    shader.apply_to_frame(
                                        &mut frame,
                                        elem_geo.size,
                                        popup_rect.loc - elem_geo.loc,
                                        popup_rect.size,
                                        22,
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
                    }

                    // 4. Renderizar capas superiores translúcidas (Dock, Top Bar, Popovers)
                    if !upper_elements.is_empty() {
                        for elem in &upper_elements {
                            let dock_rect = dock_clip_roots
                                .iter()
                                .find(|(root_id, _)| *elem.id() == *root_id)
                                .map(|(_, rect)| *rect);
                            if let (Some(rect), Some(shader)) = (dock_rect, window_squircle_shader.as_ref()) {
                                let elem_geo = elem.geometry(Scale::from(1.0));
                                let offset = Point::from((
                                    rect.loc.x - elem_geo.loc.x,
                                    rect.loc.y - elem_geo.loc.y,
                                ));
                                shader.apply_dock_clip_to_frame(&mut frame, elem_geo.size, offset, rect.size, 32);
                            }
                            let _ = draw_render_elements(&mut frame, 1.0, std::slice::from_ref(elem), &[damage]);
                            if dock_rect.is_some() {
                                frame.clear_tex_program_override();
                            }
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
                                shadow.render_shadow(
                                    gl,
                                    &projection,
                                    *popup_rect,
                                    menu_radius as f32,
                                    true,
                                    1.0,
                                );
                            });
                        }

                        for elem in popup_elems {
                            if let Some(shader) = window_squircle_shader.as_ref() {
                                let elem_geo = elem.geometry(Scale::from(1.0));
                                shader.apply_clip_to_frame(
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

                    // Dibujar overlay translúcido Aero Glass de previsualización al arrastrar cerca del borde
                    let is_pointer_grabbed = state.seat.get_pointer().map(|p| p.is_grabbed()).unwrap_or(false);
                    if is_pointer_grabbed {
                        if let Some(preview) = state.snap_preview {
                            let preview_phys = preview.to_physical_precise_round(1.0);
                            // Fondo azul translúcido (#0078d4 con 35% de opacidad)
                            let _ = frame.draw_solid(
                                preview_phys,
                                &[Rectangle::from_size(preview_phys.size)],
                                Color32F::new(0.0, 0.47, 0.83, 0.35),
                            );

                            // Borde resaltado azul de 3px
                            let border_color = Color32F::new(0.38, 0.72, 1.0, 0.85);
                            let b_thickness = 3;
                            let b_top = Rectangle::new(preview_phys.loc, (preview_phys.size.w, b_thickness).into());
                            let b_bottom = Rectangle::new(
                                (preview_phys.loc.x, preview_phys.loc.y + preview_phys.size.h - b_thickness).into(),
                                (preview_phys.size.w, b_thickness).into(),
                            );
                            let b_left = Rectangle::new(preview_phys.loc, (b_thickness, preview_phys.size.h).into());
                            let b_right = Rectangle::new(
                                (preview_phys.loc.x + preview_phys.size.w - b_thickness, preview_phys.loc.y).into(),
                                (b_thickness, preview_phys.size.h).into(),
                            );

                            let _ = frame.draw_solid(b_top, &[Rectangle::from_size(b_top.size)], border_color);
                            let _ = frame.draw_solid(b_bottom, &[Rectangle::from_size(b_bottom.size)], border_color);
                            let _ = frame.draw_solid(b_left, &[Rectangle::from_size(b_left.size)], border_color);
                            let _ = frame.draw_solid(b_right, &[Rectangle::from_size(b_right.size)], border_color);
                        }
                    } else {
                        state.snap_preview = None;
                    }

                    // Renderizado auténtico de Efecto Lámpara de Aladino (Genie Effect / Magic Lamp)
                    // Renderizado auténtico de Efecto Lámpara de Aladino (Genie Effect / Magic Lamp)
                    // Implementación canónica con física elástica, sombra volumétrica y volumen 3D
                    for (anim, elems) in &anim_data {
                        let elapsed = now.duration_since(anim.start_time);
                        let raw_t = (elapsed.as_secs_f32() / anim.duration.as_secs_f32()).clamp(0.0, 1.0);

                        // Curva de física orgánica con inercia elástica (Spring Easing)
                        let t = if anim.is_minimizing {
                            if raw_t < 0.65 {
                                (raw_t / 0.65).clamp(0.0, 1.0).powf(1.35) * 0.65
                            } else {
                                let p = ((raw_t - 0.65) / 0.35).clamp(0.0, 1.0);
                                0.65 + (1.0 - (1.0 - p).powf(2.6)) * 0.35
                            }
                        } else {
                            if raw_t < 0.75 {
                                (raw_t / 0.75).clamp(0.0, 1.0).powf(0.85) * 0.75
                            } else {
                                let p = ((raw_t - 0.75) / 0.25).clamp(0.0, 1.0);
                                0.75 + (1.0 - (1.0 - p).powf(2.2)) * 0.25
                            }
                        };

                        let split = 0.30f32;
                        let (k, j) = if anim.is_minimizing {
                            let k = if t <= split { t / split } else { 1.0 };
                            let j = if t > split { (t - split) / (1.0 - split) } else { 0.0 };
                            (k, j)
                        } else {
                            // Restauración: emerge de la lámpara (j: 1 -> 0), luego su base se asienta (k: 1 -> 0)
                            let j = 1.0 - if t <= (1.0 - split) { t / (1.0 - split) } else { 1.0 };
                            let k = 1.0 - if t > (1.0 - split) { (t - (1.0 - split)) / split } else { 0.0 };
                            (k, j)
                        };

                        info!("🎬 [Genie Frame] id={}, is_min={}, t={:.2}, k={:.2}, j={:.2}, elems={}",
                            anim.window_id, anim.is_minimizing, t, k, j, elems.len());

                        let win_x = anim.window_rect.loc.x as f32;
                        let win_y = anim.window_rect.loc.y as f32;
                        let win_w = (anim.window_rect.size.w as f32).max(40.0);
                        let win_h = (anim.window_rect.size.h as f32).max(40.0);

                        let slot_x = anim.dock_slot_rect.loc.x as f32;
                        let slot_y = anim.dock_slot_rect.loc.y as f32;
                        let slot_w = (anim.dock_slot_rect.size.w as f32).max(24.0);
                        let slot_h = (anim.dock_slot_rect.size.h as f32).max(24.0);

                        // Coordenadas objetivo en el Dock (alineación precisa al slot registrado)
                        let target_w = slot_w.max(32.0);
                        let target_x = slot_x;
                        let target_y = slot_y + slot_h * 0.40;

                        // Altura de extensión y desplazamiento de fase Compiz
                        let expand_h = (target_y - win_y - win_h).max(10.0);
                        let full_height = (target_y - win_y) - expand_h * (1.0 - k);
                        let current_height = (full_height * (1.0 - j)).max(1.0);
                        let offset_y = (target_y - win_y) * j;

                        // Función de deformación para una rebanada normalizada v en [0.0, 1.0]
                        // (v = 0.0: borde superior de la ventana; v = 1.0: borde inferior)
                        let deform_slice = |v: f32| -> (f32, f32, f32) {
                            let local_y = v * current_height;
                            let screen_y = win_y + local_y + offset_y;

                            // Ancho de la rebanada (estrechamiento hacia el embudo del Dock)
                            let slice_w = target_w
                                + (win_w - target_w) * (1.0 - j) * (1.0 - v)
                                + (win_w - target_w) * (1.0 - k) * v;
                            let slice_w = slice_w.max(2.0);

                            // Desplazamiento horizontal progresivo hacia el slot del dock (acotado a 1.0 para evitar desfase)
                            let norm_y = if full_height > 1.0 { (local_y / full_height).clamp(0.0, 1.0) } else { v };
                            let norm_x = (norm_y * k + j).min(1.0);
                            let offset_x = (target_x - win_x) * norm_x;

                            // Bucle sinusoidal ondulante en "S" (estilo Compiz / GNOME Shell Magic Lamp predeterminado)
                            let s_phase = (1.0 - norm_y) * 2.0 * std::f32::consts::PI + std::f32::consts::PI;
                            let delta_x = (win_x + win_w * 0.5) - (target_x + target_w * 0.5);
                            let effect_x = s_phase.sin() * (delta_x / 7.0) * k * (1.0 - j * 0.7);

                            let left_x = win_x + offset_x + effect_x;
                            (screen_y, left_x, slice_w)
                        };

                        let alpha = if j > 0.85 {
                            ((1.0 - j) / 0.15).clamp(0.0, 1.0)
                        } else {
                            1.0
                        };

                        // Ultra-alta resolución de malla adaptativa (160 - 240 rebanadas)
                        let n_strips = ((win_h / 2.5) as usize).clamp(160, 240);
                        let sampled_slices: Vec<(f32, f32, f32)> = (0..=n_strips)
                            .map(|i| {
                                let v = i as f32 / n_strips as f32;
                                deform_slice(v)
                            })
                            .collect();

                        // Renderizado de la textura de la ventana con traslape continuo anti-fisuras
                        for elem in elems {
                            let src = elem.src();

                            for i in 0..n_strips {
                                let local_v0 = i as f32 / n_strips as f32;
                                let local_v1 = (i + 1) as f32 / n_strips as f32;

                                let y_top = sampled_slices[i].0.round() as i32;
                                let y_bottom = sampled_slices[i + 1].0.round() as i32;
                                let strip_y = y_top.min(y_bottom);
                                let strip_h = (y_bottom - y_top).abs().max(1) + 1; // Traslape de 1px anti-fisuras

                                let left_x = (sampled_slices[i].1 + sampled_slices[i + 1].1) * 0.5;
                                let width = (sampled_slices[i].2 + sampled_slices[i + 1].2) * 0.5;

                                let strip_dst = Rectangle::new(
                                    (left_x.round() as i32, strip_y).into(),
                                    (width.round().max(1.0) as i32, strip_h).into(),
                                );

                                let src_y0 = src.loc.y + (local_v0 as f64) * src.size.h;
                                let src_y1 = src.loc.y + (local_v1 as f64) * src.size.h;
                                let strip_src = Rectangle::new(
                                    (src.loc.x, src_y0).into(),
                                    (src.size.w, (src_y1 - src_y0).max(0.1)).into(),
                                );

                                match elem.texture() {
                                    WaylandSurfaceTexture::Texture(ref texture) => {
                                        if let Err(err) = frame.render_texture_from_to(
                                            texture,
                                            strip_src,
                                            strip_dst,
                                            &[Rectangle::from_size(strip_dst.size)],
                                            &[],
                                            elem.transform(),
                                            alpha,
                                            None,
                                            &[],
                                        ) {
                                            warn!("❌ [Genie Frame] render_texture_from_to falló: {:?}", err);
                                        }
                                    }
                                    WaylandSurfaceTexture::SolidColor(color) => {
                                        let solid = *color * alpha;
                                        let _ = frame.draw_solid(
                                            strip_dst,
                                            &[Rectangle::from_size(strip_dst.size)],
                                            solid,
                                        );
                                    }
                                }
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

                    let _ = frame.finish();

                    let mut finished = Vec::new();
                    state.active_animations.retain(|anim| {
                        if now.duration_since(anim.start_time) >= anim.duration {
                            finished.push(anim.clone());
                            false
                        } else {
                            true
                        }
                    });

                    for anim in finished {
                        if !anim.is_minimizing {
                            info!("✨ Restauración Genie completada: mapeando ventana a Space");
                            state.space.map_element(anim.window.clone(), anim.original_location, true);
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
                                let target = state.space.element_under(loc).and_then(|(win, win_loc)| {
                                    let local_point = loc - win_loc.to_f64();
                                    win.surface_under(local_point, smithay::desktop::WindowSurfaceType::all())
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
                        }
                    }
                }
                if let Some(destination) = screenshot_request {
                    info!("Procesando solicitud de captura de pantalla");
                    if frame_rendered {
                        if let Err(error) =
                            crate::capture::capture_frame(
                                renderer,
                                &framebuffer,
                                size,
                                &destination,
                            )
                        {
                            warn!("Error al capturar el framebuffer: {error:#}");
                            crate::capture::write_capture_error(&destination, &error);
                        }
                    } else {
                        let error = anyhow::anyhow!("No se pudo renderizar el fotograma de captura");
                        crate::capture::write_capture_error(&destination, &error);
                    }
                }
            }
        }

        let now_ms = start_time.elapsed().as_millis() as u32;
        for surface in state.xdg_shell_state.toplevel_surfaces() {
            send_frames_surface_tree(surface.wl_surface(), now_ms);
            for (popup, _) in smithay::desktop::PopupManager::popups_for_surface(surface.wl_surface()) {
                send_frames_surface_tree(popup.wl_surface(), now_ms);
            }
        }

        {
            let layer_map = smithay::desktop::layer_map_for_output(&output);
            for layer in layer_map.layers() {
                send_frames_surface_tree(layer.wl_surface(), now_ms);
                for (popup, _) in smithay::desktop::PopupManager::popups_for_surface(layer.wl_surface()) {
                    send_frames_surface_tree(popup.wl_surface(), now_ms);
                }
            }
        }

        // Aceptar nuevos clientes de forma no bloqueante
        while let Ok(Some(stream)) = listener.accept() {
            match display.handle().insert_client(
                stream,
                Arc::new(ClientState::default()),
            ) {
                Ok(client) => {
                    info!("🎉 Nuevo cliente Wayland conectado exitosamente al compositor!");
                    clients.push(client);
                }
                Err(err) => warn!("Error al registrar nuevo cliente Wayland: {:?}", err),
            }
        }

        state.process_ipc_messages();
        display.dispatch_clients(&mut state)?;
        state.space.refresh();
        for output in state.space.outputs() {
            let mut map = smithay::desktop::layer_map_for_output(output);
            map.cleanup();
        }
        state.popups.cleanup();
        state.active_popups.retain(|p| p.alive());
        display.flush_clients()?;

        let _ = backend.submit(Some(&[damage]));

        std::thread::sleep(Duration::from_millis(16));
    }

    info!("Compositor Malus DE finalizado limpiamente.");
    Ok(())
}

pub fn toggle_spotlight(socket_name: &str) {
    let output = std::process::Command::new("pkill")
        .args(["-f", "malus-de-spotlight"])
        .status();

    if let Ok(status) = output {
        if status.success() {
            info!("Spotlight cerrado por alternancia (toggle).");
            return;
        }
    }

    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join("malus-de-spotlight")))
        .unwrap_or_else(|| std::path::PathBuf::from("malus-de-spotlight"));

    info!("Lanzando Spotlight desde: {:?}", exe);
    let mut command = std::process::Command::new(&exe);
    malus_de_common::config::set_cursor_environment(&mut command);
    command.env("WAYLAND_DISPLAY", socket_name)
        .env("GDK_BACKEND", "wayland")
        .env("GSK_RENDERER", "cairo");
    if let Ok(cfg_home) = std::env::var("XDG_CONFIG_HOME") {
        command.env("XDG_CONFIG_HOME", cfg_home);
    } else if let Ok(home) = std::env::var("HOME") {
        command.env("XDG_CONFIG_HOME", format!("{}/.config/malus-de", home));
    }
    command.env("GSETTINGS_BACKEND", "keyfile");

    match command.spawn() {
        Ok(mut child) => {
            info!("Spotlight lanzado con éxito, PID={}", child.id());
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::error!("Error al lanzar Spotlight: {:?}", e),
    }
}
