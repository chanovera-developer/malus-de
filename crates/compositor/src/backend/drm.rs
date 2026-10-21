use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use anyhow::{Context, Result};
use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Fourcc,
        },
        drm::{DrmDevice, DrmDeviceFd, DrmEvent, GbmBufferedSurface},
        egl::{EGLContext, EGLDisplay},
        input::{
            AbsolutePositionEvent, Axis, ButtonState, Event, InputEvent, KeyState,
            KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
        },
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            element::{
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement, WaylandSurfaceTexture},
                Element, Kind,
            },
            gles::{GlesRenderer, GlesTexture},
            utils::draw_render_elements,
            Bind, Color32F, Frame, ImportDma, ImportMem, Renderer, Texture,
        },
        session::{
            libseat::LibSeatSession,
            Event as SessionEvent, Session,
        },
        udev::{all_gpus, primary_gpu},
    },

    input::{
        keyboard::{keysyms, FilterResult},
        pointer::{AxisFrame, ButtonEvent, CursorImageStatus, MotionEvent},
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        calloop::EventLoop,
        drm::control::{connector, crtc, Device as ControlDevice},
        input::Libinput,
        wayland_protocols::xdg::shell::server::xdg_toplevel::{ResizeEdge, State as XdgState},
        wayland_server::{Display, ListeningSocket},
    },
    utils::{DeviceFd, Logical, Physical, Point, Rectangle, Transform, SERIAL_COUNTER, Size},
    wayland::{
        seat::WaylandFocus,
        shell::wlr_layer::Layer,
    },
};
use tracing::{error, info, warn};

use malus_de_common::config::{DisplayModeConfig, DisplayOutputConfig, DisplaysConfig};
use malus_de_common::ipc::IpcMessage;
use crate::backend::winit::{send_frames_surface_tree, toggle_spotlight};
use crate::state::{ClientState, MalusState};

struct DrmOutputSurface {
    output: Output,
    surface: GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>,
    crtc: crtc::Handle,
    modes: Vec<smithay::reexports::drm::control::Mode>,
    size: Size<i32, Physical>,
    damage: Rectangle<i32, Physical>,
}

struct MacosCursor {
    texture: GlesTexture,
    size: u32,
    width: i32,
    height: i32,
    xhot: i32,
    yhot: i32,
}

fn create_macos_cursor(renderer: &mut GlesRenderer) -> Option<MacosCursor> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let possible_paths = [
        format!("{}/.icons/macOS/cursors/default", home),
        format!("{}/.local/share/icons/macOS/cursors/default", home),
        format!("{}/.icons/macOS/cursors/left_ptr", home),
        format!("{}/.local/share/icons/macOS/cursors/left_ptr", home),
        "/usr/share/icons/macOS/cursors/default".to_string(),
        "/usr/share/icons/macOS/cursors/left_ptr".to_string(),
    ];

    for path in &possible_paths {
        if let Ok(content) = std::fs::read(path) {
            if let Some(images) = xcursor::parser::parse_xcursor(&content) {
                // Seleccionar tamaño 48 o 32 para máxima nitidez en pantallas 4K con escalado fraccionario
                if let Some(best) = images.iter().find(|img| img.size == 48)
                    .or_else(|| images.iter().find(|img| img.size == 32))
                    .or_else(|| images.iter().find(|img| img.size == 24))
                    .or_else(|| images.first())
                {
                    let size = Size::from((best.width as i32, best.height as i32));
                    if let Ok(texture) = renderer.import_memory(&best.pixels_rgba, Fourcc::Argb8888, size, false) {
                        info!(
                            "✨ Cursor nativo de macOS cargado desde '{}' ({}x{}, size={}, hotspot: {}, {})",
                            path, best.width, best.height, best.size, best.xhot, best.yhot
                        );
                        return Some(MacosCursor {
                            texture,
                            size: best.size,
                            width: best.width as i32,
                            height: best.height as i32,
                            xhot: best.xhot as i32,
                            yhot: best.yhot as i32,
                        });
                    }
                }
            }
        }
    }

    // Fallback procedimental en caso de no encontrar el tema en disco
    let w = 16;
    let h = 16;
    let mut data = vec![0u8; w * h * 4];
    let pattern = [
        "X               ",
        "XX              ",
        "X.X             ",
        "X..X            ",
        "X...X           ",
        "X....X          ",
        "X.....X         ",
        "X......X        ",
        "X.......X       ",
        "X.....XXXX      ",
        "X..X..X         ",
        "X.X X..X        ",
        "XX   X..X       ",
        "X     X..X      ",
        "       XX       ",
        "                ",
    ];

    for (y, row) in pattern.iter().enumerate() {
        for (x, ch) in row.chars().enumerate() {
            let idx = (y * w + x) * 4;
            match ch {
                'X' => {
                    data[idx] = 0;
                    data[idx + 1] = 0;
                    data[idx + 2] = 0;
                    data[idx + 3] = 255;
                }
                '.' => {
                    data[idx] = 255;
                    data[idx + 1] = 255;
                    data[idx + 2] = 255;
                    data[idx + 3] = 255;
                }
                _ => {}
            }
        }
    }

    let size = Size::from((w as i32, h as i32));
    let texture = renderer.import_memory(&data, Fourcc::Argb8888, size, false).ok()?;
    Some(MacosCursor {
        texture,
        size: 16,
        width: 16,
        height: 16,
        xhot: 0,
        yhot: 0,
    })
}

/// Genera las 4 texturas de recorte squircle (n=4.2) para esquinas de pantalla estilo display moderno
pub fn create_squircle_corner_textures(renderer: &mut GlesRenderer, radius_px: i32) -> Option<[GlesTexture; 4]> {
    let r = radius_px.max(2);
    let mut corners = Vec::with_capacity(4);
    let n = 4.2f64;
    let inv_n = 1.0 / n;

    for corner in 0..4 {
        let mut data = vec![0u8; (r * r * 4) as usize];
        let r_f = r as f64;

        for y in 0..r {
            for x in 0..r {
                let idx = ((y * r + x) * 4) as usize;
                let (u, v) = match corner {
                    0 => (r_f - (x as f64 + 0.5), r_f - (y as f64 + 0.5)), // Superior Izquierda
                    1 => (x as f64 + 0.5, r_f - (y as f64 + 0.5)),        // Superior Derecha
                    2 => (r_f - (x as f64 + 0.5), y as f64 + 0.5),        // Inferior Izquierda
                    _ => (x as f64 + 0.5, y as f64 + 0.5),                // Inferior Derecha
                };

                let dist_px = (u.powf(n) + v.powf(n)).powf(inv_n) - r_f;
                let alpha = (dist_px + 0.5).clamp(0.0, 1.0);

                if alpha > 0.0 {
                    let a_byte = (alpha * 255.0).round() as u8;
                    data[idx] = 0;
                    data[idx + 1] = 0;
                    data[idx + 2] = 0;
                    data[idx + 3] = a_byte;
                }
            }
        }

        let size = Size::from((r, r));
        let tex = match renderer.import_memory(&data, Fourcc::Abgr8888, size, false) {
            Ok(t) => t,
            Err(e) => {
                warn!("❌ [Squircle Corners] Error al importar memoria en OpenGL (esquina {}): {:?}", corner, e);
                return None;
            }
        };
        corners.push(tex);
    }

    let arr: [GlesTexture; 4] = corners.try_into().ok()?;
    info!("✅ [Squircle Corners] 4 texturas de recorte squircle (n=4.2) creadas exitosamente para radio {} px", r);
    Some(arr)
}

fn find_layer_under(
    state: &MalusState,
    point: Point<f64, Logical>,
) -> Option<(
    smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    Point<f64, Logical>,
    String,
    bool,
)> {
    for output in state.space.outputs() {
        let Some(output_geo) = state.space.output_geometry(output) else {
            continue;
        };
        if !output_geo.to_f64().contains(point) {
            continue;
        }
        let output_local_point = point - output_geo.loc.to_f64();
        let layer_map = smithay::desktop::layer_map_for_output(output);
        let under_layer = layer_map
            .layer_under(Layer::Overlay, output_local_point)
            .filter(|l| !(l.namespace() == "malus-app-switcher" && !state.app_switcher.is_active))
            .or_else(|| layer_map.layer_under(Layer::Top, output_local_point));

        if let Some(layer) = under_layer {
            if let Some(geo) = layer_map.layer_geometry(layer) {
                let local_point = output_local_point - geo.loc.to_f64();
                let surface_hit = if !state.active_popups.is_empty() {
                    layer.surface_under(local_point, smithay::desktop::WindowSurfaceType::POPUP)
                } else {
                    None
                };
                let surface_hit = surface_hit.or_else(|| {
                    if layer.namespace() == "malus-top-bar" && local_point.y >= (geo.size.h as f64).max(36.0) {
                        None
                    } else {
                        layer.surface_under(
                            local_point,
                            smithay::desktop::WindowSurfaceType::TOPLEVEL,
                        )
                    }
                });

                if let Some((surf, sub_loc)) = surface_hit {
                    return Some((
                        surf,
                        (output_geo.loc + geo.loc + sub_loc).to_f64(),
                        layer.namespace().to_string(),
                        layer.can_receive_keyboard_focus(),
                    ));
                }
            }
        }
    }
    None
}

fn clamp_pointer_location(state: &MalusState, mut point: Point<f64, Logical>) -> Point<f64, Logical> {
    let geos: Vec<Rectangle<i32, Logical>> = state
        .space
        .outputs()
        .filter_map(|o| state.space.output_geometry(o))
        .collect();

    if geos.is_empty() {
        point.x = point.x.clamp(0.0, 1919.0);
        point.y = point.y.clamp(0.0, 1079.0);
        return point;
    }

    // Si el punto ya cae dentro de cualquier monitor activo, permitir movimiento directo
    for geo in &geos {
        if geo.to_f64().contains(point) {
            return point;
        }
    }

    // Acotar coordenada X global al rango combinado de todos los monitores
    let min_x = geos.iter().map(|g| g.loc.x as f64).fold(f64::INFINITY, f64::min);
    let max_x = geos.iter().map(|g| (g.loc.x + g.size.w) as f64).fold(f64::NEG_INFINITY, f64::max);
    point.x = point.x.clamp(min_x, (max_x - 1.0).max(min_x));

    // Encontrar el monitor que cubre esta franja horizontal X (o el más cercano en X)
    // y acotar Y estrictamente al borde superior/inferior real de ese monitor para evitar zonas muertas
    let target_geo = geos
        .iter()
        .find(|g| {
            let x0 = g.loc.x as f64;
            let x1 = (g.loc.x + g.size.w) as f64;
            point.x >= x0 && point.x < x1
        })
        .or_else(|| {
            geos.iter().min_by(|a, b| {
                let da = if point.x < a.loc.x as f64 {
                    a.loc.x as f64 - point.x
                } else {
                    point.x - (a.loc.x + a.size.w) as f64
                };
                let db = if point.x < b.loc.x as f64 {
                    b.loc.x as f64 - point.x
                } else {
                    point.x - (b.loc.x + b.size.w) as f64
                };
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
        });

    if let Some(geo) = target_geo {
        let y0 = geo.loc.y as f64;
        let y1 = ((geo.loc.y + geo.size.h) as f64 - 1.0).max(y0);
        point.y = point.y.clamp(y0, y1);
    }

    point
}

fn resolve_target_under(
    state: &MalusState,
    point: Point<f64, Logical>,
) -> (
    Option<(smithay::reexports::wayland_server::protocol::wl_surface::WlSurface, Point<f64, Logical>)>,
    Option<(smithay::reexports::wayland_server::protocol::wl_surface::WlSurface, String, bool)>,
    Option<(smithay::desktop::Window, Point<i32, Logical>)>,
) {
    let layer_hit = find_layer_under(state, point);
    if let Some((surf, focus_loc, ns, can_kb)) = layer_hit {
        (
            Some((surf.clone(), focus_loc)),
            Some((surf, ns, can_kb)),
            None,
        )
    } else if let Some((window, loc)) = state.space.element_under(point) {
        let local_point = point - loc.to_f64();
        let target = window
            .surface_under(local_point, smithay::desktop::WindowSurfaceType::all())
            .map(|(surf, sub_loc)| (surf, (loc + sub_loc).to_f64()));
        (target, None, Some((window.clone(), loc)))
    } else {
        (None, None, None)
    }
}

pub fn run_drm() -> Result<()> {
    info!("============================================================");
    info!("🚀 MALUS DE: Iniciando en modo DRM/KMS nativo (Direct TTY/GDM)");
    info!("============================================================");

    let mut event_loop: EventLoop<MalusState> = EventLoop::try_new()
        .context("Error al inicializar EventLoop de Calloop")?;
    let mut display: Display<MalusState> = Display::new()
        .context("Error al crear Display Wayland")?;
    let mut state = MalusState::new(&display);

    // 1. Inicializar sesión de hardware (LibSeat / logind)
    let (mut session, notifier) = match LibSeatSession::new() {
        Ok(res) => res,
        Err(err) => {
            error!("❌ Error fatal: no se pudo inicializar LibSeatSession: {:?}", err);
            eprintln!("⚠️ LibSeat no pudo tomar control del asiento TTY ({err:?}).");
            eprintln!("   Si estás dentro de una sesión gráfica existente, ejecuta Malus DE en modo anidado:");
            eprintln!("   malus-de-compositor --backend winit");
            anyhow::bail!("LibSeatSession initialization failed: {:?}", err);
        }
    };
    let seat_name = session.seat();
    info!("✅ Sesión de hardware activa en asiento: '{}'", seat_name);

    // 2. Detectar y abrir nodo DRM de la GPU primaria vía LibSeat
    let gpu_path = primary_gpu(&seat_name)
        .context("Error al buscar GPU primaria")?
        .or_else(|| all_gpus(&seat_name).ok()?.into_iter().next())
        .unwrap_or_else(|| std::path::PathBuf::from("/dev/dri/card1"));
    info!("🎮 GPU seleccionada: {:?}", gpu_path);

    let raw_fd = session.open(
        &gpu_path,
        smithay::reexports::rustix::fs::OFlags::RDWR
            | smithay::reexports::rustix::fs::OFlags::CLOEXEC
            | smithay::reexports::rustix::fs::OFlags::NOCTTY,
    ).context("Error al abrir nodo DRM mediante libseat")?;
    let drm_device_fd = DrmDeviceFd::new(DeviceFd::from(raw_fd));

    // 3. Inicializar DrmDevice, GBM, EGL y GlesRenderer
    let (mut drm_device, drm_notifier) = DrmDevice::new(drm_device_fd.clone(), false)
        .context("Error al inicializar DrmDevice")?;
    let gbm_device = GbmDevice::new(drm_device_fd.clone())
        .context("Error al inicializar GbmDevice")?;
    let egl_display = unsafe { EGLDisplay::new(gbm_device.clone()) }
        .context("Error al crear EGLDisplay sobre GBM")?;
    let egl_context = EGLContext::new(&egl_display)
        .context("Error al crear EGLContext")?;
    let mut renderer = unsafe { GlesRenderer::new(egl_context) }
        .context("Error al inicializar GlesRenderer")?;

    info!("🎨 Renderer OpenGL ES acelerado por hardware listo");

    // Crear textura del cursor macOS
    let cursor_texture = create_macos_cursor(&mut renderer);

    // 4. Configurar dispositivos de entrada (teclado con layout de InputConfig, por defecto 'latam', y puntero)
    let input_cfg = malus_de_common::config::InputConfig::load();
    let xkb_cfg = smithay::input::keyboard::XkbConfig {
        rules: "",
        model: &input_cfg.keyboard_model,
        layout: &input_cfg.keyboard_layout,
        variant: &input_cfg.keyboard_variant,
        options: input_cfg.keyboard_options.clone(),
    };
    let _keyboard = match state.seat.add_keyboard(xkb_cfg, 200, 200) {
        Ok(kb) => {
            info!(
                "⌨️ Teclado XKB inicializado: layout='{}' ({}), variant='{}', model='{}'",
                input_cfg.keyboard_layout,
                input_cfg.layout_display_name(),
                input_cfg.keyboard_variant,
                input_cfg.keyboard_model
            );
            kb
        }
        Err(e) => {
            warn!("⚠️ Falló carga de XKB layout '{}': {:?}. Usando fallback US.", input_cfg.keyboard_layout, e);
            state
                .seat
                .add_keyboard(Default::default(), 200, 200)
                .map_err(|e| anyhow::anyhow!("Error al inicializar teclado fallback: {e:?}"))?
        }
    };
    let _pointer = state.seat.add_pointer();

    // 5. Descubrir conectores físicos y configurar superficies de pantalla
    let res_handles = drm_device.resource_handles()
        .context("Error al consultar resource handles de DRM")?;

    // Desactivar cualquier cursor por hardware residual de GDM o compositores previos en todos los CRTCs
    for &crtc_handle in res_handles.crtcs() {
        #[allow(deprecated)]
        if let Err(e) = drm_device.set_cursor::<smithay::reexports::drm::control::dumbbuffer::DumbBuffer>(crtc_handle, None) {
            tracing::debug!("Aviso al limpiar hardware cursor en CRTC {:?}: {:?}", crtc_handle, e);
        }
    }

    let mut displays_cfg = DisplaysConfig::load();
    let mut used_crtcs = HashSet::new();
    let mut surfaces = Vec::new();

    // Recopilar conectores conectados y ordenarlos de modo que el monitor primario (DP-1) sea el primero
    let mut connected_list = Vec::new();
    for &conn_handle in res_handles.connectors() {
        let conn_info = match drm_device.get_connector(conn_handle, false) {
            Ok(info) => info,
            Err(e) => {
                warn!("Error al leer connector {:?}: {:?}", conn_handle, e);
                continue;
            }
        };

        if conn_info.state() != connector::State::Connected {
            continue;
        }

        let conn_name = format!("{}-{}", conn_info.interface().as_str(), conn_info.interface_id());
        let is_primary = displays_cfg
            .find_for_connector(&conn_name)
            .map(|c| c.primary)
            .unwrap_or(false);
        connected_list.push((conn_handle, conn_info, conn_name, is_primary));
    }

    // Mantener solo las salidas conectadas y refrescar sus modos a partir de DRM.
    let mut detected_outputs = Vec::with_capacity(connected_list.len());
    let mut detected_x = 0;
    for (idx, (_, conn_info, conn_name, _)) in connected_list.iter().enumerate() {
        let modes: Vec<DisplayModeConfig> = conn_info
            .modes()
            .iter()
            .map(|mode| {
                let (width, height) = mode.size();
                DisplayModeConfig {
                    width: width as u32,
                    height: height as u32,
                    refresh_rate: mode.vrefresh() as f64,
                }
            })
            .collect();
        let previous = displays_cfg.find_for_connector(conn_name).cloned();
        let selected_mode = previous
            .as_ref()
            .and_then(|output| {
                conn_info.modes().iter().find(|mode| {
                    let (width, height) = mode.size();
                    width as u32 == output.width
                        && height as u32 == output.height
                        && (mode.vrefresh() as f64 - output.refresh_rate).abs() < 1.0
                })
            })
            .or_else(|| conn_info.modes().first());
        let Some(selected_mode) = selected_mode else {
            continue;
        };
        let (width, height) = selected_mode.size();
        let scale = previous.as_ref().map(|output| output.scale).unwrap_or(2.0);
        let logical_width = (width as f64 / scale.max(0.5)).round() as i32;
        let (physical_width_mm, physical_height_mm) = conn_info.size().unwrap_or((0, 0));
        let detected_diagonal = if physical_width_mm > 0 && physical_height_mm > 0 {
            Some(
                (((physical_width_mm as f64).powi(2) + (physical_height_mm as f64).powi(2)).sqrt()
                    / 25.4
                    * 10.0)
                    .round()
                    / 10.0,
            )
        } else {
            None
        };
        detected_outputs.push(DisplayOutputConfig {
            connector: conn_name.clone(),
            name: previous
                .as_ref()
                .map(|output| output.name.clone())
                .unwrap_or_else(|| conn_name.clone()),
            width: width as u32,
            height: height as u32,
            refresh_rate: selected_mode.vrefresh() as f64,
            modes,
            x: previous.as_ref().map(|output| output.x).unwrap_or(detected_x),
            y: previous.as_ref().map(|output| output.y).unwrap_or(0),
            scale,
            primary: previous
                .as_ref()
                .map(|output| output.primary)
                .unwrap_or(idx == 0),
            diagonal_inches: previous
                .as_ref()
                .and_then(|output| output.diagonal_inches)
                .or(detected_diagonal),
        });
        detected_x += logical_width.max(100);
    }
    let primary_idx = detected_outputs
        .iter()
        .position(|output| output.primary)
        .unwrap_or(0);
    for (idx, output) in detected_outputs.iter_mut().enumerate() {
        output.primary = idx == primary_idx;
    }
    displays_cfg.outputs = detected_outputs;
    if let Some(primary) = displays_cfg.outputs.get(primary_idx) {
        displays_cfg.master_scale = primary.scale;
    }
    displays_cfg.recalculate_layout();
    if let Err(err) = displays_cfg.save() {
        warn!("No se pudo guardar la detección de pantallas: {}", err);
    }

    // Monitor primario siempre en el índice 0 de state.space.outputs()
    connected_list.sort_by_key(|(_, _, name, _)| {
        !displays_cfg
            .find_for_connector(name)
            .map(|output| output.primary)
            .unwrap_or(false)
    });

    let mut next_auto_x: i32 = 0;

    for (conn_handle, conn_info, conn_name, _) in connected_list {
        info!("🔌 Monitor físico conectado detectado: {}", conn_name);

        let cfg_output = displays_cfg.find_for_connector(&conn_name);
        let mode = if let Some(cfg) = cfg_output {
            conn_info
                .modes()
                .iter()
                .find(|m| {
                    m.size().0 == cfg.width as u16
                        && m.size().1 == cfg.height as u16
                        && (m.vrefresh() as f64 - cfg.refresh_rate).abs() < 1.0
                })
                .or_else(|| {
                    conn_info
                        .modes()
                        .iter()
                        .find(|m| m.size().0 == cfg.width as u16 && m.size().1 == cfg.height as u16)
                })
                .copied()
                .or_else(|| conn_info.modes().first().copied())
        } else {
            conn_info.modes().first().copied()
        };

        let mode = match mode {
            Some(m) => m,
            None => {
                warn!("No hay modos de vídeo para {}", conn_name);
                continue;
            }
        };

        let possible_crtcs: Vec<crtc::Handle> = conn_info
            .encoders()
            .iter()
            .filter_map(|&enc_handle| drm_device.get_encoder(enc_handle).ok())
            .flat_map(|enc| res_handles.filter_crtcs(enc.possible_crtcs()))
            .collect();

        let crtc = match possible_crtcs.into_iter().find(|c| !used_crtcs.contains(c)) {
            Some(c) => {
                used_crtcs.insert(c);
                c
            }
            None => {
                warn!("No se encontró CRTC libre para el monitor {}", conn_name);
                continue;
            }
        };

        info!("   🖥 Asignando CRTC {:?} a {} con modo {}x{} @ {}Hz",
            crtc, conn_name, mode.size().0, mode.size().1, mode.vrefresh());

        let drm_surface = match drm_device.create_surface(crtc, mode, &[conn_handle]) {
            Ok(s) => s,
            Err(e) => {
                error!("Error al crear DrmSurface para {}: {:?}", conn_name, e);
                continue;
            }
        };

        let gbm_allocator = GbmAllocator::new(
            gbm_device.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        );

        let gbm_surface = match GbmBufferedSurface::new(
            drm_surface,
            gbm_allocator,
            &[Fourcc::Argb8888, Fourcc::Xrgb8888],
            renderer.dmabuf_formats(),
        ) {
            Ok(s) => s,
            Err(e) => {
                error!("Error al inicializar GbmBufferedSurface para {}: {:?}", conn_name, e);
                continue;
            }
        };

        let (mode_w, mode_h) = mode.size();
        let (loc_x, loc_y, scale, phys_mm, model_label) = if let Some(cfg) = cfg_output {
            let (w_mm, h_mm) = cfg.physical_size_mm();
            (
                cfg.x,
                cfg.y,
                cfg.scale.max(0.5),
                (w_mm.round() as i32, h_mm.round() as i32),
                cfg.name.clone(),
            )
        } else {
            let fallback_scale = displays_cfg.master_scale.max(1.0);
            let raw_mm = conn_info.size().unwrap_or((600, 340));
            (
                next_auto_x,
                0,
                fallback_scale,
                (raw_mm.0 as i32, raw_mm.1 as i32),
                conn_name.clone(),
            )
        };

        let logical_w = (mode_w as f64 / scale).round() as i32;
        let logical_h = (mode_h as f64 / scale).round() as i32;
        next_auto_x = next_auto_x.max(loc_x + logical_w);

        info!(
            "   📐 Monitor {} ({}) extendido: pos=({}, {}), escala={:.4}x ({:.1}%), lógico={}x{}, físico={}x{}mm",
            conn_name,
            model_label,
            loc_x,
            loc_y,
            scale,
            scale * 100.0,
            logical_w,
            logical_h,
            phys_mm.0,
            phys_mm.1
        );

        let output = Output::new(
            conn_name.clone(),
            PhysicalProperties {
                size: phys_mm.into(),
                subpixel: Subpixel::Unknown,
                make: "Apple/Malus".to_string(),
                model: model_label,
            },
        );

        let smithay_mode = Mode {
            size: (mode_w as i32, mode_h as i32).into(),
            refresh: (mode.vrefresh() * 1000) as i32,
        };
        let _global = output.create_global::<MalusState>(&state.display_handle);
        output.set_preferred(smithay_mode);
        output.change_current_state(
            Some(smithay_mode),
            Some(Transform::Normal),
            Some(Scale::Fractional(scale)),
            Some((loc_x, loc_y).into()),
        );
        state.space.map_output(&output, (loc_x, loc_y));

        surfaces.push(DrmOutputSurface {
            output,
            surface: gbm_surface,
            crtc,
            modes: conn_info.modes().to_vec(),
            size: (mode_w as i32, mode_h as i32).into(),
            damage: Rectangle::from_size((mode_w as i32, mode_h as i32).into()),
        });
    }

    if surfaces.is_empty() {
        warn!("⚠️ No se detectó ninguna pantalla activa conectada en DRM.");
    }

    // Centrar el puntero inmediatamente en el centro de la pantalla primaria para evitar coordenadas vacías
    let primary_center = state.space.outputs().next().and_then(|primary| {
        let geo = state.space.output_geometry(primary)?;
        Some(geo.loc.to_f64() + (geo.size.to_f64() / 2.0))
    });

    if let Some(center) = primary_center {
        if let Some(pointer) = state.seat.get_pointer() {
            let serial = SERIAL_COUNTER.next_serial();
            pointer.motion(
                &mut state,
                None,
                &MotionEvent {
                    location: center,
                    serial,
                    time: 0,
                },
            );
            pointer.frame(&mut state);
            info!("🎯 Puntero DRM inicializado en el centro de la pantalla: {:?}", center);
        }
    }

    // 6. Crear socket de escucha de clientes Wayland (wayland-1 .. 32)
    // Se crea una vez que los monitores están descubiertos, dimensionados y mapeados en Space
    let listener = ListeningSocket::bind_auto("wayland", 1..33)
        .map_err(|e| anyhow::anyhow!("Error al bindear socket de Wayland: {e:?}"))?;
    let socket_name = listener
        .socket_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    info!("🌐 Socket Wayland escuchando activamente en: WAYLAND_DISPLAY={}", socket_name);
    state.socket_name = socket_name.clone();

    let shared_surfaces = Arc::new(Mutex::new(surfaces));
    let surfaces_for_drm = shared_surfaces.clone();
    let surfaces_for_vt = shared_surfaces.clone();

    // 7. Inicializar Libinput para teclado y ratón
    let mut libinput_context = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(
        session.clone().into(),
    );
    if let Err(e) = libinput_context.udev_assign_seat(&seat_name) {
        warn!("Aviso al asignar asiento en libinput: {:?}", e);
    }
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    // 8. Registrar fuentes de eventos en el EventLoop de Calloop
    let loop_handle = event_loop.handle();

    // Notificador de pausa / reactivación de sesión TTY (VT Switch)
    loop_handle.insert_source(notifier, move |event, &mut (), _state: &mut MalusState| {
        match event {
            SessionEvent::PauseSession => {
                info!("⏸️ Sesión TTY pausada (VT switch away)");
                libinput_context.suspend();
            }
            SessionEvent::ActivateSession => {
                info!("▶️ Sesión TTY reactivada (VT switch back)");
                if let Err(e) = libinput_context.resume() {
                    error!("Error al reanudar libinput tras VT switch: {:?}", e);
                }
                if let Ok(mut surfs) = surfaces_for_vt.lock() {
                    for s in surfs.iter_mut() {
                        s.damage = Rectangle::from_size(s.size);
                    }
                }
            }
        }
    }).map_err(|e| anyhow::anyhow!("Error al registrar notificador de sesión: {e:?}"))?;

    // Notificador de eventos de sincronización DRM (VBlank y Page Flip)
    loop_handle.insert_source(drm_notifier, move |event, _metadata, _state: &mut MalusState| {
        match event {
            DrmEvent::VBlank(crtc) => {
                if let Ok(mut surfs) = surfaces_for_drm.lock() {
                    for s in surfs.iter_mut() {
                        if s.crtc == crtc {
                            let _ = s.surface.frame_submitted();
                        }
                    }
                }
            }
            DrmEvent::Error(err) => {
                warn!("Aviso en evento DRM: {:?}", err);
            }
        }
    }).map_err(|e| anyhow::anyhow!("Error al registrar notificador DRM: {e:?}"))?;

    // Despacho de eventos de Libinput con soporte para ratón, touchpad y atajos completos
    let mut session_vt = session.clone();

    loop_handle.insert_source(libinput_backend, move |event, _, state: &mut MalusState| {
        match event {
            InputEvent::Keyboard { event } => {
                let _key = event.key_code().raw();
                let is_pressed = event.state() == KeyState::Pressed;

                if let Some(keyboard) = state.seat.get_keyboard() {
                    keyboard.input::<(), _>(
                        state,
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

                            if is_pressed {
                                let has_super = modifiers.logo;
                                let has_alt = modifiers.alt;
                                let has_ctrl = modifiers.ctrl;

                                // 🚨 RED DE SEGURIDAD CRÍTICA: CAMBIO DE TTY CON CTRL + ALT + F1..F12
                                if has_ctrl && has_alt {
                                    let vt_target = if sym == keysyms::KEY_F1.into() { Some(1) }
                                    else if sym == keysyms::KEY_F2.into() { Some(2) }
                                    else if sym == keysyms::KEY_F3.into() { Some(3) }
                                    else if sym == keysyms::KEY_F4.into() { Some(4) }
                                    else if sym == keysyms::KEY_F5.into() { Some(5) }
                                    else if sym == keysyms::KEY_F6.into() { Some(6) }
                                    else if sym == keysyms::KEY_F7.into() { Some(7) }
                                    else if sym == keysyms::KEY_F8.into() { Some(8) }
                                    else if sym == keysyms::KEY_F9.into() { Some(9) }
                                    else if sym == keysyms::KEY_F10.into() { Some(10) }
                                    else if sym == keysyms::KEY_F11.into() { Some(11) }
                                    else if sym == keysyms::KEY_F12.into() { Some(12) }
                                    else { None };

                                    if let Some(vt) = vt_target {
                                        info!("🚨 [VT SWITCH] Solicitando cambio de terminal virtual a tty{} vía LibSeat", vt);
                                        if let Err(err) = session_vt.change_vt(vt) {
                                            error!("Error al cambiar a VT {}: {:?}", vt, err);
                                        }
                                        return FilterResult::Intercept(());
                                    }
                                }

                                let is_tab = sym == keysyms::KEY_Tab.into();
                                let is_return = sym == keysyms::KEY_Return.into() || sym == keysyms::KEY_KP_Enter.into();
                                let is_left = sym == keysyms::KEY_Left.into();
                                let is_right = sym == keysyms::KEY_Right.into();
                                let is_q = sym == keysyms::KEY_q.into() || sym == keysyms::KEY_Q.into();
                                let is_w = sym == keysyms::KEY_w.into() || sym == keysyms::KEY_W.into();
                                let is_h = sym == keysyms::KEY_h.into() || sym == keysyms::KEY_H.into();
                                let is_m = sym == keysyms::KEY_m.into() || sym == keysyms::KEY_M.into();
                                let is_esc = sym == keysyms::KEY_Escape.into();

                                // 2. Navegación activa dentro del App Switcher (⌘Tab HUD)
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

                                let is_space = sym == keysyms::KEY_space.into();
                                let is_k = sym == keysyms::KEY_k.into() || sym == keysyms::KEY_K.into();

                                // Spotlight (Super+Espacio / Alt+Espacio / Super+K)
                                if (is_space && (has_super || has_alt || has_ctrl)) || (is_k && has_super) {
                                    info!("🔍 Spotlight alternado vía atajo de teclado");
                                    toggle_spotlight(&state.socket_name);
                                    return FilterResult::Intercept(());
                                }

                                // Cerrar ventana activa (Super+W / Super+Q / Alt+F4)
                                let is_f4 = sym == keysyms::KEY_F4.into();
                                if (has_super && (is_w || is_q)) || (has_alt && is_f4) {
                                    state.close_active_window();
                                    return FilterResult::Intercept(());
                                }

                                // Minimizar ventana activa con Genie Effect (Super+M / Super+H / Alt+F9)
                                let is_f9 = sym == keysyms::KEY_F9.into();
                                if (has_super && (is_m || is_h)) || (has_alt && is_f9) {
                                    state.minimize_active_window();
                                    return FilterResult::Intercept(());
                                }

                                // Atajos de Aero Snap / Tiling (Super o Alt + Flechas)
                                if has_super || has_alt {
                                    let is_up = sym == keysyms::KEY_Up.into();
                                    let is_down = sym == keysyms::KEY_Down.into();

                                    if is_left || is_right || is_up || is_down {
                                        if let Some(win) = state.get_active_window() {
                                            let target_out = state
                                                .space
                                                .outputs_for_element(&win)
                                                .into_iter()
                                                .next()
                                                .or_else(|| state.space.outputs().next().cloned());

                                            if let Some(output) = target_out {
                                                let usable_zone = state.usable_zone_for_output(&output);
                                                let snap_mode = if is_left {
                                                    crate::state::SnapMode::LeftHalf
                                                } else if is_right {
                                                    crate::state::SnapMode::RightHalf
                                                } else if is_up {
                                                    crate::state::SnapMode::Maximize
                                                } else {
                                                    crate::state::SnapMode::CenterRestore
                                                };
                                                crate::state::snap_window(state, &win, snap_mode, usable_zone);
                                                return FilterResult::Intercept(());
                                            }
                                        }
                                    }
                                }
                            } else {
                                // Liberación de modificadores (Super/Alt) -> Commit de App Switcher
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
            }
            InputEvent::PointerMotion { event } => {
                if let Some(pointer) = state.seat.get_pointer() {
                    let mut current_pos = pointer.current_location();
                    current_pos.x += event.delta_x();
                    current_pos.y += event.delta_y();
                    let point = clamp_pointer_location(state, current_pos);

                    let (under_target, _, _) = resolve_target_under(state, point);

                    let serial = SERIAL_COUNTER.next_serial();
                    pointer.motion(
                        state,
                        under_target,
                        &MotionEvent {
                            location: point,
                            serial,
                            time: event.time_msec(),
                        },
                    );
                    pointer.frame(state);
                }
            }
            InputEvent::PointerMotionAbsolute { event } => {
                if let Some(pointer) = state.seat.get_pointer() {
                    let cur = pointer.current_location();
                    let output = state
                        .space
                        .outputs()
                        .find(|o| {
                            state
                                .space
                                .output_geometry(o)
                                .map(|g| g.to_f64().contains(cur))
                                .unwrap_or(false)
                        })
                        .or_else(|| state.space.outputs().next())
                        .cloned();

                    if let Some(output) = output {
                        let output_geo = state.space.output_geometry(&output).unwrap_or_default();
                        let mut point = event.position_transformed(output_geo.size);
                        point.x += output_geo.loc.x as f64;
                        point.y += output_geo.loc.y as f64;
                        let point = clamp_pointer_location(state, point);

                        let (under_target, _, _) = resolve_target_under(state, point);

                        let serial = SERIAL_COUNTER.next_serial();
                        pointer.motion(
                            state,
                            under_target,
                            &MotionEvent {
                                location: point,
                                serial,
                                time: event.time_msec(),
                            },
                        );
                        pointer.frame(state);
                    }
                }
            }
            InputEvent::PointerButton { event } => {
                if let Some(pointer) = state.seat.get_pointer() {
                    let point = pointer.current_location();
                    let is_pressed = event.state() == ButtonState::Pressed;

                    let (under_target, layer_info, regular_win) = resolve_target_under(state, point);

                    let motion_serial = SERIAL_COUNTER.next_serial();
                    pointer.motion(
                        state,
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

                        if pointer.is_grabbed() {
                            state.snap_preview = None;
                            pointer.unset_grab(state, btn_serial, event.time_msec());
                        }

                        if let Some(keyboard) = state.seat.get_keyboard() {
                            if keyboard.is_grabbed() {
                                keyboard.unset_grab(state);
                            }

                            if let Some((surf, _ns, can_kb)) = layer_info {
                                if can_kb {
                                    keyboard.set_focus(state, Some(surf), btn_serial);
                                }
                            } else if let Some((window, loc)) = regular_win {
                                let was_already_top = state.space.elements().last().map(|w| w == &window).unwrap_or(false);
                                if !was_already_top {
                                    state.space.raise_element(&window, true);
                                }
                                if let Some(surface) = window.wl_surface() {
                                    if keyboard.current_focus().as_ref() != Some(surface.as_ref()) {
                                        keyboard.set_focus(state, Some(surface.clone().into_owned()), btn_serial);
                                        state.notify_active_app(Some(&surface));
                                    }
                                }

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
                                                keyboard.set_focus(state, Some(surface.clone().into_owned()), btn_serial);
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
                                        pointer.set_grab(state, grab, btn_serial, smithay::input::pointer::Focus::Clear);
                                        info!("📐 [ServerResize] Redimensión perimetral activada para Firefox: edge={:?}, win={:?}", edge, resize_win.geometry());
                                    }
                                }

                                if (has_super || has_alt) && event.button_code() == 0x110 {
                                    let start_data = smithay::input::pointer::GrabStartData {
                                        focus: None,
                                        button: event.button_code(),
                                        location: point,
                                    };
                                    let grab = crate::grabs::MoveSurfaceGrab {
                                        start_data,
                                        window: window.clone(),
                                        initial_window_location: loc,
                                    };
                                    pointer.set_grab(state, grab, btn_serial, smithay::input::pointer::Focus::Clear);
                                }

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
                                    pointer.set_grab(state, grab, btn_serial, smithay::input::pointer::Focus::Clear);
                                }
                            }
                        }
                    } else {
                        state.snap_preview = None;
                    }

                    pointer.button(
                        state,
                        &ButtonEvent {
                            button: event.button_code(),
                            state: event.state(),
                            serial: btn_serial,
                            time: event.time_msec(),
                        },
                    );
                    if !pointer.is_grabbed() {
                        state.snap_preview = None;
                    }
                    pointer.frame(state);
                }
            }
            InputEvent::PointerAxis { event } => {
                if let Some(pointer) = state.seat.get_pointer() {
                    let mut frame = AxisFrame::new(event.time_msec()).source(event.source());
                    let horiz = event.amount(Axis::Horizontal).unwrap_or(0.0);
                    let vert = event.amount(Axis::Vertical).unwrap_or(0.0);
                    if horiz != 0.0 {
                        frame = frame.value(Axis::Horizontal, horiz);
                    }
                    if vert != 0.0 {
                        frame = frame.value(Axis::Vertical, vert);
                    }
                    pointer.axis(state, frame);
                    pointer.frame(state);
                }
            }
            _ => {}
        }
    }).map_err(|e| anyhow::anyhow!("Error al registrar libinput event source: {e:?}"))?;

    info!("============================================================");
    info!("✨ MALUS DE: Bucle de renderizado DRM/KMS y GPU activo");
    info!("   WAYLAND_DISPLAY={}", socket_name);
    info!("============================================================");

    let dark_acrylic = Color32F::new(0.094, 0.094, 0.106, 1.0);
    let start_time = Instant::now();
    let mut clients = Vec::new();

    let monitors_toml_path = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".config/malus-de/monitors.toml"))
        .unwrap_or_else(|_| std::path::PathBuf::from(".config/malus-de/monitors.toml"));
    let mut last_monitors_mtime = std::fs::metadata(&monitors_toml_path)
        .and_then(|m| m.modified())
        .ok();
    let input_toml_path = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".config/malus-de/input.toml"))
        .unwrap_or_else(|_| std::path::PathBuf::from(".config/malus-de/input.toml"));
    let mut last_input_mtime = std::fs::metadata(&input_toml_path)
        .and_then(|m| m.modified())
        .ok();
    let mut last_config_check = Instant::now();
    let mut current_displays_cfg = DisplaysConfig::load();
    let mut squircle_corner_cache: std::collections::HashMap<i32, [GlesTexture; 4]> = std::collections::HashMap::new();
    let mut blur_pipeline: Option<crate::blur::DualKawaseBlur> = None;
    let mut window_squircle_shader: Option<crate::window_corners::WindowSquircleShader> = None;
    let mut shadow_pipeline: Option<crate::shadow::WindowShadowPipeline> = None;

    while state.is_running {
        // Si la sesión TTY no está activa (VT switch away), pausar renderizado para no saturar GPU
        if !session.is_active() {
            std::thread::sleep(Duration::from_millis(30));
            let _ = event_loop.dispatch(Some(Duration::from_millis(1)), &mut state);
            continue;
        }

        // Hot-reload automático de monitors.toml e input.toml (inmediato vía IPC o cada 250ms por mtime)
        let check_now = state.pending_displays_reload || last_config_check.elapsed() >= Duration::from_millis(250);
        if check_now {
            last_config_check = Instant::now();
            let current_mtime = std::fs::metadata(&monitors_toml_path)
                .and_then(|m| m.modified())
                .ok();
            if state.pending_displays_reload || (current_mtime.is_some() && current_mtime != last_monitors_mtime) {
                state.pending_displays_reload = false;
                last_monitors_mtime = current_mtime;
                current_displays_cfg = DisplaysConfig::load();
                current_displays_cfg.recalculate_layout();
                let new_cfg = &current_displays_cfg;
                squircle_corner_cache.clear();
                info!("🔄 Recargando disposición y escalas de pantallas en vivo desde monitors.toml...");

                // Capturar geometrías previas de cada monitor para migrar ventanas si los monitores se intercambian o desplazan
                let mut prev_output_geos: std::collections::HashMap<String, Rectangle<i32, Logical>> =
                    std::collections::HashMap::new();
                for out in state.space.outputs() {
                    if let Some(geo) = state.space.output_geometry(out) {
                        prev_output_geos.insert(out.name(), geo);
                    }
                }

                let mut new_output_locs: std::collections::HashMap<String, (Point<i32, Logical>, f64)> =
                    std::collections::HashMap::new();

                if let Ok(mut surfs) = shared_surfaces.lock() {
                    let mut auto_x = 0;
                    for s in surfs.iter_mut() {
                        let out_name = s.output.name();
                        let (loc_x, loc_y, scale, requested_mode) = if let Some(cfg) = new_cfg.find_for_connector(&out_name) {
                            let requested_mode = s.modes.iter().find(|mode| {
                                let (width, height) = mode.size();
                                width as u32 == cfg.width
                                    && height as u32 == cfg.height
                                    && (mode.vrefresh() as f64 - cfg.refresh_rate).abs() < 1.0
                            }).copied();
                            (cfg.x, cfg.y, cfg.scale.clamp(0.5, 4.0), requested_mode)
                        } else {
                            (auto_x, 0, new_cfg.master_scale.clamp(0.5, 4.0), None)
                        };
                        let mut mode_update = None;
                        if let Some(mode) = requested_mode {
                            let current_mode = s.surface.current_mode();
                            if mode.size() != current_mode.size()
                                || mode.vrefresh() != current_mode.vrefresh()
                            {
                                match s.surface.use_mode(mode) {
                                    Ok(()) => {
                                        let (width, height) = mode.size();
                                        s.size = (width as i32, height as i32).into();
                                        s.damage = Rectangle::from_size(s.size);
                                        mode_update = Some(Mode {
                                            size: (width as i32, height as i32).into(),
                                            refresh: (mode.vrefresh() * 1000) as i32,
                                        });
                                        info!(
                                            "   🖥 Resolución de {} cambiada en vivo a {}x{} @ {}Hz",
                                            out_name,
                                            width,
                                            height,
                                            mode.vrefresh()
                                        );
                                    }
                                    Err(err) => warn!(
                                        "No se pudo cambiar {} al modo {}x{} @ {}Hz: {:?}",
                                        out_name,
                                        mode.size().0,
                                        mode.size().1,
                                        mode.vrefresh(),
                                        err
                                    ),
                                }
                            }
                        }
                        let log_w = (s.size.w as f64 / scale).round() as i32;
                        auto_x = auto_x.max(loc_x + log_w);

                        s.output.change_current_state(
                            mode_update,
                            None,
                            Some(Scale::Fractional(scale)),
                            Some((loc_x, loc_y).into()),
                        );
                        if let Some(mode) = mode_update {
                            s.output.set_preferred(mode);
                        }
                        state.space.map_output(&s.output, (loc_x, loc_y));
                        new_output_locs.insert(out_name.clone(), (Point::from((loc_x, loc_y)), scale));

                        // Notificar nueva escala fraccionaria y reconfigurar todas las Layer Surfaces del monitor
                        let mut lmap = smithay::desktop::layer_map_for_output(&s.output);
                        lmap.arrange();
                        for layer in lmap.layers() {
                            let wl_surf = layer.wl_surface();
                            smithay::wayland::compositor::with_states(wl_surf, |states| {
                                smithay::wayland::fractional_scale::with_fractional_scale(states, |fs| {
                                    fs.set_preferred_scale(scale);
                                });
                            });
                            layer.layer_surface().send_configure();
                        }

                        s.damage = Rectangle::from_size(s.size);
                        info!(
                            "   ✅ Monitor {} actualizado en vivo: pos=({}, {}), escala={:.4}x ({:.1}%)",
                            out_name,
                            loc_x,
                            loc_y,
                            scale,
                            scale * 100.0
                        );
                    }
                }

                // Migrar ventanas cuyos monitores cambiaron de posición y actualizar su escala fraccionaria
                let windows: Vec<_> = state.space.elements().cloned().collect();
                for win in windows {
                    if let Some(win_loc) = state.space.element_location(&win) {
                        let win_geo = win.geometry();
                        let center = Point::<i32, Logical>::from((
                            win_loc.x + (win_geo.size.w.max(1) / 2),
                            win_loc.y + (win_geo.size.h.max(1) / 2),
                        ));
                        // Identificar en qué monitor estaba antes del cambio
                        let owner_out = prev_output_geos
                            .iter()
                            .find(|(_, geo)| geo.contains(center))
                            .or_else(|| prev_output_geos.iter().find(|(_, geo)| geo.contains(win_loc)))
                            .map(|(name, geo)| (name.clone(), *geo));

                        if let Some((out_name, old_geo)) = owner_out {
                            if let Some(&(new_loc, _new_scale)) = new_output_locs.get(&out_name) {
                                let delta = new_loc - old_geo.loc;
                                if delta.x != 0 || delta.y != 0 {
                                    let updated_loc = win_loc + delta;
                                    state.space.map_element(win.clone(), updated_loc, false);
                                }
                            }
                        }
                    }

                    if let Some(wl_surf) = win.wl_surface() {
                        let new_scale = state.preferred_scale_for_surface(&wl_surf);
                        smithay::wayland::compositor::with_states(&wl_surf, |states| {
                            smithay::wayland::fractional_scale::with_fractional_scale(states, |fs| {
                                fs.set_preferred_scale(new_scale);
                            });
                        });
                    }
                    if let Some(toplevel) = win.toplevel() {
                        toplevel.send_configure();
                    }
                }
            }

            // Hot-reload de input.toml si cambió en disco (p.ej. desde CLI)
            let cur_input_mtime = std::fs::metadata(&input_toml_path)
                .and_then(|m| m.modified())
                .ok();
            if cur_input_mtime.is_some() && cur_input_mtime != last_input_mtime {
                last_input_mtime = cur_input_mtime;
                let input_cfg = malus_de_common::config::InputConfig::load();
                state.apply_keyboard_layout(&input_cfg.keyboard_layout, &input_cfg.keyboard_variant);
            }
        }

        // Aceptar nuevos clientes de forma no bloqueante
        while let Ok(Some(stream)) = listener.accept() {
            match display.handle().insert_client(
                stream,
                Arc::new(ClientState::default()),
            ) {
                Ok(client) => {
                    info!("🎉 Nuevo cliente Wayland conectado al compositor DRM!");
                    clients.push(client);
                }
                Err(err) => warn!("Error al registrar cliente Wayland en DRM: {:?}", err),
            }
        }

        // Despachar eventos de hardware (libinput, sesión, vblank)
        let _ = event_loop.dispatch(Some(Duration::from_millis(1)), &mut state);

        state.process_ipc_messages();
        display.dispatch_clients(&mut state)?;
        state.space.refresh();

        for output in state.space.outputs() {
            let mut map = smithay::desktop::layer_map_for_output(output);
            map.cleanup();
        }
        state.popups.cleanup();
        state.active_popups.retain(|p| p.alive());


        // 🎨 RENDERIZADO GPU REAL A CADA MONITOR FÍSICO
        if let Ok(mut surfs) = shared_surfaces.lock() {
            for out_surface in surfs.iter_mut() {
                let scale = out_surface.output.current_scale().fractional_scale();
                let output_geo = state.space.output_geometry(&out_surface.output).unwrap_or_default();

                let (mut dmabuf, _age) = match out_surface.surface.next_buffer() {
                    Ok(buf) => buf,
                    Err(e) => {
                        // Si no hay slots libres, la GPU aún está en escaneo de buffer anterior (esperar vblank)
                        warn!("Buffer no disponible para {}: {:?}", out_surface.output.name(), e);
                        continue;
                    }
                };

                let damage = out_surface.damage;

                // Preparar animaciones Genie activas que intersecten este monitor
                let now = Instant::now();
                let mut anim_data = Vec::new();
                for anim in &state.active_animations {
                    let anim_bbox = anim.window_rect.merge(anim.dock_slot_rect);
                    if output_geo.overlaps(anim_bbox) {
                        let elems = render_elements_from_surface_tree::<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>(
                            &mut renderer,
                            &anim.surface,
                            (0, 0),
                            1.0,
                            1.0,
                            Kind::Unspecified,
                        );
                        anim_data.push((anim.clone(), elems));
                    }
                }

                // Preparar elementos de cursor antes de comenzar el pase de renderizado
                let mut cursor_elems = Vec::new();
                let mut cursor_draw_point = None;
                let mut cursor_hotspot = Point::from((0, 0));

                if let Some(pointer) = state.seat.get_pointer() {
                    let loc = pointer.current_location();
                    if output_geo.to_f64().contains(loc) {
                        let phys_x = ((loc.x - output_geo.loc.x as f64) * scale).round() as i32;
                        let phys_y = ((loc.y - output_geo.loc.y as f64) * scale).round() as i32;
                        cursor_draw_point = Some((phys_x, phys_y));

                        if let CursorImageStatus::Surface(ref surf) = state.cursor_status {
                            cursor_hotspot = smithay::wayland::compositor::with_states(surf, |states| {
                                states
                                    .data_map
                                    .get::<smithay::input::pointer::CursorImageSurfaceData>()
                                    .map(|attrs| attrs.lock().unwrap().hotspot)
                                    .unwrap_or_default()
                            });

                            cursor_elems = render_elements_from_surface_tree::<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>(
                                &mut renderer,
                                surf,
                                Point::from((0, 0)),
                                smithay::utils::Scale::from(1.0),
                                1.0,
                                Kind::Unspecified,
                            );
                        }
                    }
                }

                let corner_radius_pt = current_displays_cfg.screen_corner_radius;
                let phys_r = if corner_radius_pt > 0.0 {
                    let r = (corner_radius_pt * scale).round() as i32;
                    if r > 2 {
                        if !squircle_corner_cache.contains_key(&r) {
                            if let Some(textures) = create_squircle_corner_textures(&mut renderer, r) {
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
                    let r = (win_corner_radius_pt * scale).round() as i32;
                    if r > 2 { Some(r) } else { None }
                } else {
                    None
                };

                let layer_map = smithay::desktop::layer_map_for_output(&out_surface.output);
                let mut lower_elements = Vec::new();
                let mut upper_elements = Vec::new();
                for surface in layer_map.layers().rev() {
                    if surface.namespace() == "malus-app-switcher" && !state.app_switcher.is_active {
                        continue;
                    }
                    if let Some(geo) = layer_map.layer_geometry(surface) {
                        let layer_loc = geo.loc.to_physical_precise_round(scale);
                        let s_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = render_elements_from_surface_tree(
                            &mut renderer,
                            surface.wl_surface(),
                            layer_loc,
                            smithay::utils::Scale::from(scale),
                            1.0,
                            Kind::Unspecified,
                        );
                        let mut p_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();
                        for (popup, popup_offset) in smithay::desktop::PopupManager::popups_for_surface(surface.wl_surface()) {
                            let offset = (popup_offset - popup.geometry().loc)
                                .to_f64()
                                .to_physical(scale)
                                .to_i32_round();
                            let elems = render_elements_from_surface_tree(
                                &mut renderer,
                                popup.wl_surface(),
                                layer_loc + offset,
                                smithay::utils::Scale::from(scale),
                                1.0,
                                Kind::Unspecified,
                            );
                            p_elems.extend(elems);
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
                for window in state.space.elements() {
                    if let Some(loc) = state.space.element_location(window) {
                        let geo = window.geometry();
                        let win_global_rect = Rectangle::new(loc, geo.size);
                        if output_geo.overlaps(win_global_rect) {
                            let phys_loc = (loc - geo.loc - output_geo.loc).to_physical_precise_round(scale);
                            let Some(main_surface) = window.wl_surface() else {
                                continue;
                            };

                            let win_surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = render_elements_from_surface_tree(
                                &mut renderer,
                                main_surface.as_ref(),
                                phys_loc,
                                smithay::utils::Scale::from(scale),
                                1.0,
                                Kind::Unspecified,
                            );

                            let mut popup_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();
                            for (popup, popup_offset) in smithay::desktop::PopupManager::popups_for_surface(main_surface.as_ref()) {
                                let offset = (geo.loc + popup_offset - popup.geometry().loc).to_physical_precise_round(scale);
                                let p_elems = render_elements_from_surface_tree(
                                    &mut renderer,
                                    popup.wl_surface(),
                                    phys_loc + offset,
                                    smithay::utils::Scale::from(scale),
                                    1.0,
                                    Kind::Unspecified,
                                );
                                popup_elems.extend(p_elems);
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
                            let content_offset = geo.loc.to_physical_precise_round(scale);
                            let content_size = geo.size.to_physical_precise_round(scale);
                            let win_screen_rect = Rectangle::new(
                                (loc - output_geo.loc).to_physical_precise_round(scale),
                                geo.size.to_physical_precise_round(scale),
                            );

                            window_render_data.push((root_id, win_surfaces, popup_elems, should_round, is_active, win_screen_rect, content_offset, content_size, is_firefox, is_translucent));
                        }
                    }
                }

                // Inicializar shader de recorte squircle para ventanas si aún no existe
                if window_squircle_shader.is_none() {
                    window_squircle_shader = crate::window_corners::WindowSquircleShader::new(&mut renderer);
                }

                if let Ok(mut target) = renderer.bind(&mut dmabuf) {
                    if let Ok(mut frame) = renderer.render(&mut target, out_surface.size, Transform::Normal) {
                        let _ = frame.clear(dark_acrylic, &[damage]);
                        // 1. Renderizar fondo (wallpaper y capas inferiores)
                        if !lower_elements.is_empty() {
                            let _ = draw_render_elements(&mut frame, scale, &lower_elements, &[damage]);
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
                        for (root_id, win_elems, _popups, should_round, is_active, win_rect, content_offset, content_size, is_firefox, is_translucent) in &window_render_data {
                            if win_elems.is_empty() {
                                continue;
                            }

                            // 2.1. SOMBRA PROYECTADA (Window Drop Shadow con Squircle SDF n=4.2)
                            // Se dibuja para ventanas no en pantalla completa inmediatamente antes de su superficie
                            if *should_round {
                                if let (Some(r), Some(ref shadow)) = (win_phys_r, shadow_pipeline.as_ref()) {
                                    let _ = frame.with_context(|gl| {
                                        shadow.render_shadow(gl, &projection, *win_rect, r as f32, *is_active, scale);
                                    });
                                }

                                // 2.1.5. DESENFOQUE DE FONDO DUAL KAWASE (Exclusivo para Nautilus y Ajustes del Sistema)
                                if *is_translucent {
                                    if let (Some(r), Some(ref mut blur)) = (win_phys_r, blur_pipeline.as_mut()) {
                                        let screen_size = out_surface.size;
                                        let _ = frame.with_context(|gl| {
                                            blur.render_blur(gl, &projection, screen_size, *win_rect, r as f32);
                                        });
                                    }
                                }
                            }

                            // 2.2. SUPERFICIES DE LA VENTANA (con recorte Squircle)
                            if *should_round {
                                if *is_firefox {
                                    // Para Firefox: aplicar recorte a todo su árbol de subsuperficies (WebRender)
                                    // para evitar esquinas cuadradas en el lienzo interno
                                    if let (Some(r), Some(ref shader)) = (win_phys_r, window_squircle_shader.as_ref()) {
                                        for elem in win_elems {
                                            let elem_geo = elem.geometry(smithay::utils::Scale::from(scale));
                                            let sub_offset = smithay::utils::Point::from((
                                                win_rect.loc.x - elem_geo.loc.x,
                                                win_rect.loc.y - elem_geo.loc.y,
                                            ));
                                            shader.apply_to_frame(&mut frame, elem_geo.size, sub_offset, win_rect.size, r);

                                            let _ = draw_render_elements(&mut frame, scale, std::slice::from_ref(elem), &[damage]);

                                            frame.clear_tex_program_override();
                                        }
                                    } else {
                                        let _ = draw_render_elements(&mut frame, scale, win_elems, &[damage]);
                                    }
                                } else {
                                    // Para el resto de ventanas (GTK, Electron, etc.): comportamiento intacto original.
                                    // Solo se recorta la superficie raíz (is_root), sin alterar widgets ni subsuperficies GTK.
                                    for elem in win_elems {
                                        let is_root = elem.id() == root_id;
                                        if is_root {
                                            if let (Some(r), Some(ref shader)) = (win_phys_r, window_squircle_shader.as_ref()) {
                                                let elem_geo = elem.geometry(smithay::utils::Scale::from(scale));
                                                shader.apply_to_frame(&mut frame, elem_geo.size, *content_offset, *content_size, r);
                                            }
                                        }

                                        let _ = draw_render_elements(&mut frame, scale, std::slice::from_ref(elem), &[damage]);

                                        if is_root {
                                            frame.clear_tex_program_override();
                                        }
                                    }
                                }
                            } else {
                                let _ = draw_render_elements(&mut frame, scale, win_elems, &[damage]);
                            }
                        }

                        // 2. Aplicar desenfoque de fondo Dual Kawase por GPU a las capas y popups translúcidos
                        if blur_pipeline.is_none() {
                            blur_pipeline = frame.with_context(|gl| crate::blur::DualKawaseBlur::new(gl)).ok().flatten();
                        }

                        if let Some(ref mut blur) = blur_pipeline {
                            let screen_size = out_surface.size;
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
                                                "malus-dock" => 22.0 * scale as f32,
                                                "malus-spotlight" => 18.0 * scale as f32,
                                                "malus-notifications" => 16.0 * scale as f32,
                                                _ => 0.0,
                                            };
                                            if radius > 0.0 {
                                                let phys_rect = geo.to_physical_precise_round(scale);
                                                blur.render_blur(gl, &projection, screen_size, phys_rect, radius);
                                            }
                                        }
                                    }
                                }
                            });
                        }

                        // 3. Renderizar menús contextuales y popups de ventanas (siempre ENCIMA de las superficies de ventana)
                        for (_, _, popup_elems, ..) in &window_render_data {
                            if !popup_elems.is_empty() {
                                let _ = draw_render_elements(&mut frame, scale, popup_elems, &[damage]);
                            }
                        }

                        // 4. Renderizar capas superiores translúcidas (Dock, Top Bar, Popovers)
                        if !upper_elements.is_empty() {
                            let _ = draw_render_elements(&mut frame, scale, &upper_elements, &[damage]);
                        }

                        // Dibujar previsualización translúcida de Aero Snap si el puntero está agarrado y la previsualización intersecta este monitor
                        let is_pointer_grabbed = state.seat.get_pointer().map(|p| p.is_grabbed()).unwrap_or(false);
                        if is_pointer_grabbed {
                            if let Some(preview) = state.snap_preview {
                                if output_geo.overlaps(preview) {
                                    let local_preview = Rectangle::new(
                                        preview.loc - output_geo.loc,
                                        preview.size,
                                    );
                                    let preview_phys = local_preview.to_physical_precise_round(scale);
                                    let _ = frame.draw_solid(
                                        preview_phys,
                                        &[Rectangle::from_size(preview_phys.size)],
                                        Color32F::new(0.0, 0.47, 0.83, 0.35),
                                    );
                                }
                            }
                        } else {
                            state.snap_preview = None;
                        }

                        // 🧞 EFECTO LÁMPARA DE ALADINO (GENIE EFFECT / MAGIC LAMP)
                        for (anim, elems) in &anim_data {
                            let elapsed = now.duration_since(anim.start_time);
                            let raw_t = (elapsed.as_secs_f32() / anim.duration.as_secs_f32()).clamp(0.0, 1.0);

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
                                let j = 1.0 - if t <= (1.0 - split) { t / (1.0 - split) } else { 1.0 };
                                let k = 1.0 - if t > (1.0 - split) { (t - (1.0 - split)) / split } else { 0.0 };
                                (k, j)
                            };

                            let win_x = ((anim.window_rect.loc.x - output_geo.loc.x) as f64 * scale) as f32;
                            let win_y = ((anim.window_rect.loc.y - output_geo.loc.y) as f64 * scale) as f32;
                            let win_w = ((anim.window_rect.size.w as f64 * scale) as f32).max(40.0);
                            let win_h = ((anim.window_rect.size.h as f64 * scale) as f32).max(40.0);

                            let slot_x = ((anim.dock_slot_rect.loc.x - output_geo.loc.x) as f64 * scale) as f32;
                            let slot_y = ((anim.dock_slot_rect.loc.y - output_geo.loc.y) as f64 * scale) as f32;
                            let slot_w = ((anim.dock_slot_rect.size.w as f64 * scale) as f32).max(24.0);
                            let slot_h = ((anim.dock_slot_rect.size.h as f64 * scale) as f32).max(24.0);

                            let target_w = slot_w.max(32.0);
                            let target_x = slot_x;
                            let target_y = slot_y + slot_h * 0.40;

                            let expand_h = (target_y - win_y - win_h).max(10.0);
                            let full_height = (target_y - win_y) - expand_h * (1.0 - k);
                            let current_height = (full_height * (1.0 - j)).max(1.0);
                            let offset_y = (target_y - win_y) * j;

                            let deform_slice = |v: f32| -> (f32, f32, f32) {
                                let local_y = v * current_height;
                                let screen_y = win_y + local_y + offset_y;

                                let slice_w = target_w
                                    + (win_w - target_w) * (1.0 - j) * (1.0 - v)
                                    + (win_w - target_w) * (1.0 - k) * v;
                                let slice_w = slice_w.max(2.0);

                                let norm_y = if full_height > 1.0 { (local_y / full_height).clamp(0.0, 1.0) } else { v };
                                let norm_x = (norm_y * k + j).min(1.0);
                                let offset_x = (target_x - win_x) * norm_x;

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

                            let n_strips = ((win_h / 2.5) as usize).clamp(160, 240);
                            let sampled_slices: Vec<(f32, f32, f32)> = (0..=n_strips)
                                .map(|i| {
                                    let v = i as f32 / n_strips as f32;
                                    deform_slice(v)
                                })
                                .collect();

                            for elem in elems {
                                let src = elem.src();

                                for i in 0..n_strips {
                                    let local_v0 = i as f32 / n_strips as f32;
                                    let local_v1 = (i + 1) as f32 / n_strips as f32;

                                    let y_top = sampled_slices[i].0.round() as i32;
                                    let y_bottom = sampled_slices[i + 1].0.round() as i32;
                                    let strip_y = y_top.min(y_bottom);
                                    let strip_h = (y_bottom - y_top).abs().max(1) + 1;

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
                                            let _ = frame.render_texture_from_to(
                                                texture,
                                                strip_src,
                                                strip_dst,
                                                &[Rectangle::from_size(strip_dst.size)],
                                                &[],
                                                elem.transform(),
                                                alpha,
                                                None,
                                                &[],
                                            );
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

                        // Dibujar puntero del ratón con escala física precisa, tamaño macOS 24pt normalizado y hotspot exacto
                        if let Some((phys_x, phys_y)) = cursor_draw_point {
                            if !cursor_elems.is_empty() {
                                for elem in &cursor_elems {
                                    let view_dst = elem.view().dst;
                                    if view_dst.w > 0 && view_dst.h > 0 {
                                        // Normalizar tamaño estrictamente a 24 puntos lógicos (evita cursores gigantes de clientes HiDPI)
                                        let target_logical_w = if view_dst.w > 24 { 24.0 } else { view_dst.w as f64 };
                                        let target_logical_h = if view_dst.h > 24 {
                                            24.0 * (view_dst.h as f64 / view_dst.w as f64)
                                        } else {
                                            view_dst.h as f64
                                        };
                                        let ratio_x = target_logical_w / view_dst.w as f64;
                                        let ratio_y = target_logical_h / view_dst.h as f64;

                                        let hot_phys_x = (cursor_hotspot.x as f64 * ratio_x * scale).round() as i32;
                                        let hot_phys_y = (cursor_hotspot.y as f64 * ratio_y * scale).round() as i32;
                                        let cursor_phys_w = (target_logical_w * scale).round() as i32;
                                        let cursor_phys_h = (target_logical_h * scale).round() as i32;

                                        let cursor_rect = Rectangle::new(
                                            (phys_x - hot_phys_x, phys_y - hot_phys_y).into(),
                                            (cursor_phys_w, cursor_phys_h).into(),
                                        );

                                        match elem.texture() {
                                            WaylandSurfaceTexture::Texture(ref texture) => {
                                                let _ = frame.render_texture_from_to(
                                                    texture,
                                                    elem.src(),
                                                    cursor_rect,
                                                    &[Rectangle::from_size(cursor_rect.size)],
                                                    &[],
                                                    elem.transform(),
                                                    elem.alpha(),
                                                    None,
                                                    &[],
                                                );
                                            }
                                            WaylandSurfaceTexture::SolidColor(color) => {
                                                let _ = frame.draw_solid(
                                                    cursor_rect,
                                                    &[Rectangle::from_size(cursor_rect.size)],
                                                    *color * elem.alpha(),
                                                );
                                            }
                                        }
                                    }
                                }
                            } else if let Some(ref cur) = cursor_texture {
                                let scale_factor = (24.0 * scale) / cur.size as f64;
                                let cursor_w = (cur.width as f64 * scale_factor).round() as i32;
                                let cursor_h = (cur.height as f64 * scale_factor).round() as i32;
                                let hot_x = (cur.xhot as f64 * scale_factor).round() as i32;
                                let hot_y = (cur.yhot as f64 * scale_factor).round() as i32;
                                let cursor_rect = Rectangle::new(
                                    (phys_x - hot_x, phys_y - hot_y).into(),
                                    (cursor_w, cursor_h).into(),
                                );
                                let _ = frame.render_texture_from_to(
                                    &cur.texture,
                                    Rectangle::from_size(cur.texture.size()).to_f64(),
                                    cursor_rect,
                                    &[Rectangle::from_size(cursor_rect.size)],
                                    &[],
                                    Transform::Normal,
                                    1.0,
                                    None,
                                    &[],
                                );
                            }
                        }

                        // 🖥 ESQUINAS SQUIRCLE SUPERELÍPTICAS (n=4.2) DEL ESCRITORIO (macOS Display Corner Cutout)
                        if let Some(r) = phys_r {
                            if let Some(textures) = squircle_corner_cache.get(&r) {
                                let (w, h) = (out_surface.size.w, out_surface.size.h);
                                let corner_rects = [
                                    Rectangle::new((0, 0).into(), (r, r).into()), // Superior Izquierda
                                    Rectangle::new((w - r, 0).into(), (r, r).into()), // Superior Derecha
                                    Rectangle::new((0, h - r).into(), (r, r).into()), // Inferior Izquierda
                                    Rectangle::new((w - r, h - r).into(), (r, r).into()), // Inferior Derecha
                                ];

                                for (i, rect) in corner_rects.iter().enumerate() {
                                    if let Err(e) = frame.render_texture_from_to(
                                        &textures[i],
                                        Rectangle::from_size(textures[i].size()).to_f64(),
                                        *rect,
                                        &[Rectangle::from_size(rect.size)],
                                        &[],
                                        Transform::Normal,
                                        1.0,
                                        None,
                                        &[],
                                    ) {
                                        warn!("❌ Error al renderizar textura squircle esquina {}: {:?}", i, e);
                                    }
                                }
                            }
                        }

                        let _ = frame.finish();
                    }
                }

                if let Err(e) = out_surface.surface.queue_buffer(None, None, ()) {
                    warn!("Error al encolar buffer en DRM: {:?}", e);
                }
            }
        }

        // Completar animaciones Genie (minimizar / restaurar) tras renderizar el frame
        let now = Instant::now();
        let mut finished_anims = Vec::new();
        state.active_animations.retain(|anim| {
            if now.duration_since(anim.start_time) >= anim.duration {
                finished_anims.push(anim.clone());
                false
            } else {
                true
            }
        });

        for anim in finished_anims {
            if !anim.is_minimizing {
                info!("✨ Restauración Genie completada en DRM: mapeando ventana a Space");
                state.space.map_element(anim.window.clone(), anim.original_location, true);
                state.space.raise_element(&anim.window, true);
                if let Some(toplevel) = anim.window.toplevel() {
                    toplevel.send_configure();
                }
                if let Some(kb) = state.seat.get_keyboard() {
                    let serial = SERIAL_COUNTER.next_serial();
                    kb.set_focus(&mut state, Some(anim.surface.clone()), serial);
                }
                state.notify_active_app(Some(&anim.surface));
                state.notify_running_apps();
            }
        }

        // Notificar fotogramas a los clientes
        let now_ms = start_time.elapsed().as_millis() as u32;
        for surface in state.xdg_shell_state.toplevel_surfaces() {
            send_frames_surface_tree(surface.wl_surface(), now_ms);
            for (popup, _) in smithay::desktop::PopupManager::popups_for_surface(surface.wl_surface()) {
                send_frames_surface_tree(popup.wl_surface(), now_ms);
            }
        }

        for output in state.space.outputs() {
            let layer_map = smithay::desktop::layer_map_for_output(output);
            for layer in layer_map.layers() {
                send_frames_surface_tree(layer.wl_surface(), now_ms);
                for (popup, _) in smithay::desktop::PopupManager::popups_for_surface(layer.wl_surface()) {
                    send_frames_surface_tree(popup.wl_surface(), now_ms);
                }
            }
        }

        display.flush_clients()?;
        std::thread::sleep(Duration::from_millis(16));
    }

    info!("Compositor Malus DE DRM finalizado limpiamente.");
    Ok(())
}
