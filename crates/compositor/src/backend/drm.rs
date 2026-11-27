use anyhow::{Context, Result};
use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Format, Fourcc,
        },
        drm::{DrmDevice, DrmDeviceFd, DrmEvent, GbmBufferedSurface},
        egl::{EGLContext, EGLDisplay},
        input::{
            AbsolutePositionEvent, Axis, ButtonState, Event, GestureBeginEvent, GestureEndEvent,
            GesturePinchUpdateEvent, GestureSwipeUpdateEvent, InputEvent, KeyState,
            KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent, TouchEvent,
        },
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            element::{
                surface::{
                    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
                    WaylandSurfaceTexture,
                },
                Element, Kind,
            },
            gles::{GlesRenderer, GlesTexture},
            utils::draw_render_elements,
            Bind, Color32F, Frame, ImportDma, ImportMem, Renderer, Texture,
        },
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent},
    },
    input::{
        keyboard::{keysyms, FilterResult},
        pointer::{AxisFrame, ButtonEvent, CursorImageStatus, MotionEvent},
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        calloop::EventLoop,
        drm::control::{connector, crtc, Device as ControlDevice, ResourceHandles},
        input::{
            AccelProfile, ClickMethod, Device as LibinputDevice, DeviceCapability, Libinput,
            SendEventsMode,
        },
        wayland_protocols::xdg::shell::server::xdg_toplevel::{ResizeEdge, State as XdgState},
        wayland_server::{backend::GlobalId, Display, ListeningSocket},
    },
    utils::{
        DeviceFd, IsAlive, Logical, Physical, Point, Rectangle, Size, Transform, SERIAL_COUNTER,
    },
    wayland::{seat::WaylandFocus, shell::wlr_layer::Layer},
};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tracing::{error, info, warn};

use crate::backend::winit::send_frames_surface_tree;
use crate::state::{ClientState, MalusState};
use malus_de_common::config::{DisplayModeConfig, DisplayOutputConfig, DisplaysConfig};
use malus_de_common::ipc::IpcMessage;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::render_elements;

render_elements! {
    pub MalusDrmElement<='a, GlesRenderer>;
    Wayland = &'a WaylandSurfaceRenderElement<GlesRenderer>,
    WaylandSquircle = crate::window_corners::SquircleSurfaceRenderElement<'a>,
    Memory = &'a MemoryRenderBufferRenderElement<GlesRenderer>,
    MemorySquircle = crate::window_corners::SquircleMemoryRenderElement<'a>,
    Shadow = crate::shadow::WindowShadowRenderElement,
    DockShadow = crate::shadow::DockShadowRenderElement,
    MenuShadow = crate::shadow::MenuShadowRenderElement,
    Blur = crate::blur::BlurRenderElement,
}

/// Aplica los ajustes de Trackpad y Mouse de `InputConfig` a un dispositivo libinput.
fn apply_libinput_device_config(
    device: &mut LibinputDevice,
    cfg: &malus_de_common::config::InputConfig,
) {
    if device.config_tap_finger_count() > 0 {
        let _ = device.config_tap_set_enabled(cfg.trackpad_tap_to_click);
        let _ = device.config_tap_set_drag_enabled(cfg.trackpad_tap_to_click);
        if device.config_dwt_is_available() {
            let _ = device.config_dwt_set_enabled(cfg.trackpad_disable_while_typing);
        }
        if device.config_scroll_has_natural_scroll() {
            let _ = device.config_scroll_set_natural_scroll_enabled(cfg.natural_scrolling);
        }
        if device.config_accel_is_available() {
            let _ = device.config_accel_set_speed(cfg.trackpad_speed.clamp(-1.0, 1.0));
        }
        let click_methods = device.config_click_methods();
        let desired_click = match cfg.trackpad_secondary_click.as_str() {
            "bottom_right" | "bottom_left" => Some(ClickMethod::ButtonAreas),
            "off" => None,
            _ => Some(ClickMethod::Clickfinger),
        };
        if let Some(method) = desired_click {
            if click_methods.contains(&method) {
                let _ = device.config_click_set_method(method);
            }
        }
        if device
            .config_send_events_modes()
            .contains(SendEventsMode::DISABLED)
        {
            let mode = if cfg.touchpad_enabled {
                SendEventsMode::ENABLED
            } else {
                SendEventsMode::DISABLED
            };
            let _ = device.config_send_events_set_mode(mode);
        }
        return;
    }

    if !device.has_capability(DeviceCapability::Pointer) {
        return;
    }
    if device.config_accel_is_available() {
        let _ = device.config_accel_set_speed(cfg.mouse_speed.clamp(-1.0, 1.0));
        let desired_profile = if cfg.mouse_pointer_acceleration {
            AccelProfile::Adaptive
        } else {
            AccelProfile::Flat
        };
        if device.config_accel_profiles().contains(&desired_profile) {
            let _ = device.config_accel_set_profile(desired_profile);
        }
    }
    if device.config_scroll_has_natural_scroll() {
        let _ = device.config_scroll_set_natural_scroll_enabled(cfg.mouse_natural_scrolling);
    }
    if device.config_left_handed_is_available() {
        let _ = device.config_left_handed_set(cfg.mouse_secondary_click == "left");
    }
}

fn send_presentation_feedback_surface_tree(
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    output: &Output,
    time: std::time::Duration,
    refresh: smithay::wayland::presentation::Refresh,
    seq: u64,
) {
    smithay::wayland::compositor::with_surface_tree_downward(
        surface,
        (),
        |_, _, &()| smithay::wayland::compositor::TraversalAction::DoChildren(()),
        |_surf, states, &()| {
            let callbacks = std::mem::take(
                &mut states
                    .cached_state
                    .get::<smithay::wayland::presentation::PresentationFeedbackCachedState>()
                    .current()
                    .callbacks,
            );
            for callback in callbacks {
                callback.presented(
                    output,
                    time,
                    refresh,
                    seq,
                    smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind::Vsync,
                );
            }
        },
        |_, _, &()| true,
    );
}

fn trigger_emergency_compositor_exit(reason: &str) {
    warn!(
        "🛑 [ZAP] Cierre de emergencia del compositor activado: {}",
        reason
    );
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(600));
        std::process::exit(0);
    });
}

#[derive(Default, Debug, Clone, Copy)]
struct EmergencyZapKeys {
    left_ctrl: bool,
    right_ctrl: bool,
    left_alt: bool,
    right_alt: bool,
}

impl EmergencyZapKeys {
    fn reset(&mut self) {
        *self = Self::default();
    }

    /// Actualiza el estado con un evento evdev (`EV_KEY`) y devuelve `true` si se pulsó `Ctrl + Alt + BackSpace`.
    fn update(&mut self, code: u16, value: i32) -> bool {
        const KEY_BACKSPACE: u16 = 14;
        const KEY_LEFTCTRL: u16 = 29;
        const KEY_LEFTALT: u16 = 56;
        const KEY_RIGHTCTRL: u16 = 97;
        const KEY_RIGHTALT: u16 = 100;

        let down = value != 0;
        match code {
            KEY_LEFTCTRL => self.left_ctrl = down,
            KEY_RIGHTCTRL => self.right_ctrl = down,
            KEY_LEFTALT => self.left_alt = down,
            KEY_RIGHTALT => self.right_alt = down,
            KEY_BACKSPACE if value == 1 => {
                return (self.left_ctrl || self.right_ctrl) && (self.left_alt || self.right_alt);
            }
            _ => {}
        }
        false
    }
}

fn parse_evdev_key_event(chunk: &[u8]) -> Option<(u16, i32)> {
    const EV_KEY: u16 = 1;
    #[cfg(target_pointer_width = "64")]
    const HEADER_BYTES: usize = 16;
    #[cfg(not(target_pointer_width = "64"))]
    const HEADER_BYTES: usize = 8;

    if chunk.len() < HEADER_BYTES + 8 {
        return None;
    }
    let ev_type = u16::from_ne_bytes([chunk[HEADER_BYTES], chunk[HEADER_BYTES + 1]]);
    if ev_type != EV_KEY {
        return None;
    }
    let code = u16::from_ne_bytes([chunk[HEADER_BYTES + 2], chunk[HEADER_BYTES + 3]]);
    let value = i32::from_ne_bytes([
        chunk[HEADER_BYTES + 4],
        chunk[HEADER_BYTES + 5],
        chunk[HEADER_BYTES + 6],
        chunk[HEADER_BYTES + 7],
    ]);
    Some((code, value))
}

fn sysfs_key_caps_has_backspace(caps: &str) -> bool {
    let Some(last_word) = caps.split_whitespace().last() else {
        return false;
    };
    u64::from_str_radix(last_word, 16).is_ok_and(|mask| (mask & (1u64 << 14)) != 0)
}

fn open_readable_evdev_keyboards() -> Vec<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    let Ok(entries) = std::fs::read_dir("/sys/class/input") else {
        return Vec::new();
    };
    let nonblock = smithay::reexports::rustix::fs::OFlags::NONBLOCK.bits() as i32;
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("event") {
            continue;
        }
        let key_caps_path = entry.path().join("device/capabilities/key");
        let Ok(caps) = std::fs::read_to_string(key_caps_path) else {
            continue;
        };
        if !sysfs_key_caps_has_backspace(&caps) {
            continue;
        }
        let dev_path = std::path::Path::new("/dev/input").join(&name);
        if let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nonblock)
            .open(dev_path)
        {
            files.push(file);
        }
    }
    files
}

/// Hilo independiente de emergencia: si el usuario tiene permiso de lectura en `/dev/input/event*`
/// (p. ej. grupo `input`), permite cerrar el entorno con `Ctrl+Alt+BackSpace` aunque el hilo
/// principal del compositor esté completamente bloqueado.
fn spawn_emergency_zap_watchdog(session_active: Arc<AtomicBool>, session_locked: Arc<AtomicBool>) {
    std::thread::Builder::new()
        .name("malus-zap-watchdog".to_string())
        .spawn(move || {
            use std::io::Read;

            #[cfg(target_pointer_width = "64")]
            const EVENT_SIZE: usize = 24;
            #[cfg(not(target_pointer_width = "64"))]
            const EVENT_SIZE: usize = 16;

            let mut keyboards = open_readable_evdev_keyboards();
            if keyboards.is_empty() {
                return;
            }
            let mut key_state = EmergencyZapKeys::default();
            let mut buf = [0u8; EVENT_SIZE * 16];
            let mut last_rescan = Instant::now();

            loop {
                if !session_active.load(Ordering::Relaxed)
                    || session_locked.load(Ordering::Relaxed)
                {
                    key_state.reset();
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                }

                // Esperar pasivamente eventos del kernel con `poll()` (timeout 500 ms) en lugar
                // de despertar el hilo 28 veces por segundo en reposo.
                let mut needs_rescan = last_rescan.elapsed() >= Duration::from_secs(30);
                let mut readable_indices = Vec::new();
                {
                    let mut poll_fds: Vec<rustix::event::PollFd<'_>> = keyboards
                        .iter()
                        .map(|kb| {
                            rustix::event::PollFd::new(kb, rustix::event::PollFlags::IN)
                        })
                        .collect();
                    let ready = rustix::event::poll(&mut poll_fds, 500).unwrap_or(0);
                    if ready > 0 {
                        for (idx, pfd) in poll_fds.iter().enumerate() {
                            let revents = pfd.revents();
                            if revents.intersects(
                                rustix::event::PollFlags::ERR
                                    | rustix::event::PollFlags::HUP
                                    | rustix::event::PollFlags::NVAL,
                            ) {
                                needs_rescan = true;
                            } else if revents.contains(rustix::event::PollFlags::IN) {
                                readable_indices.push(idx);
                            }
                        }
                    }
                }
                for idx in readable_indices {
                    let kb = &mut keyboards[idx];
                    loop {
                        match kb.read(&mut buf) {
                            Ok(n) if n >= EVENT_SIZE => {
                                for chunk in buf[..n].chunks_exact(EVENT_SIZE) {
                                    if let Some((code, value)) = parse_evdev_key_event(chunk) {
                                        if key_state.update(code, value) {
                                            warn!("🛑 [ZAP-EVDEV] Ctrl+Alt+BackSpace detectado en hilo independiente; cerrando compositor");
                                            std::process::exit(0);
                                        }
                                    }
                                }
                            }
                            _ => break,
                        }
                    }
                }

                if needs_rescan {
                    last_rescan = Instant::now();
                    let refreshed = open_readable_evdev_keyboards();
                    if !refreshed.is_empty() {
                        keyboards = refreshed;
                    }
                }
            }
        })
        .ok();
}

const ENABLE_DAMAGE_TRACKING: bool = false;

fn scan_dev_dri_cards() -> Vec<std::path::PathBuf> {
    let mut cards = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/dev/dri") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some(rest) = name_str.strip_prefix("card") {
                if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                    cards.push(entry.path());
                }
            }
        }
    }
    cards.sort();
    for fallback in ["/dev/dri/card0", "/dev/dri/card1"] {
        let p = std::path::PathBuf::from(fallback);
        if !cards.contains(&p) {
            cards.push(p);
        }
    }
    cards
}

/// Puntúa una tarjeta DRM vía sysfs (`/sys/class/drm/cardN*`):
/// `(tiene_monitor_conectado, es_driver_kms_real)` sin requerir abrir el nodo DRM.
fn sysfs_drm_card_score(path: &std::path::Path) -> (bool, bool) {
    let Some(card_name) = path.file_name().and_then(|n| n.to_str()) else {
        return (false, true);
    };
    let drm_sysfs = std::path::Path::new("/sys/class/drm");
    let uevent_path = drm_sysfs.join(card_name).join("device/uevent");
    let is_real_kms = std::fs::read_to_string(uevent_path)
        .map(|content| {
            !content.lines().any(|line| {
                matches!(
                    line.trim(),
                    "DRIVER=simple-framebuffer" | "DRIVER=vgem" | "DRIVER=vkms"
                )
            })
        })
        .unwrap_or(true);

    let prefix = format!("{card_name}-");
    let mut has_connected = false;
    if let Ok(entries) = std::fs::read_dir(drm_sysfs) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if !name.to_string_lossy().starts_with(&prefix) {
                continue;
            }
            if let Ok(status) = std::fs::read_to_string(entry.path().join("status")) {
                if status.trim() == "connected" {
                    has_connected = true;
                    break;
                }
            }
        }
    }
    (has_connected, is_real_kms)
}

/// Deduplica las rutas candidatas de GPU conservando el orden de descubrimiento y prioriza
/// GPUs KMS reales con monitores conectados (útil en laptops híbridas o sin panel eDP interno).
fn prioritize_drm_gpu_candidates(
    primary: Option<std::path::PathBuf>,
    udev_gpus: Vec<std::path::PathBuf>,
    dev_dri_cards: Vec<std::path::PathBuf>,
    score_fn: impl Fn(&std::path::Path) -> (bool, bool),
) -> Vec<std::path::PathBuf> {
    let mut ordered = Vec::new();
    let mut seen_canonical = HashSet::new();
    for path in primary.into_iter().chain(udev_gpus).chain(dev_dri_cards) {
        let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen_canonical.insert(key) {
            ordered.push(path);
        }
    }
    let mut indexed: Vec<(usize, (bool, bool), std::path::PathBuf)> = ordered
        .into_iter()
        .enumerate()
        .map(|(idx, p)| {
            let score = score_fn(&p);
            (idx, score, p)
        })
        .collect();
    indexed.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    indexed.into_iter().map(|(_, _, p)| p).collect()
}

fn open_first_working_drm_gpu(
    session: &mut LibSeatSession,
    candidates: &[std::path::PathBuf],
) -> Result<(
    DrmDevice,
    smithay::backend::drm::DrmDeviceNotifier,
    GbmDevice<DrmDeviceFd>,
    GlesRenderer,
    std::path::PathBuf,
)> {
    let mut errors = Vec::new();
    for path in candidates {
        let raw_fd = match session.open(
            path,
            smithay::reexports::rustix::fs::OFlags::RDWR
                | smithay::reexports::rustix::fs::OFlags::CLOEXEC
                | smithay::reexports::rustix::fs::OFlags::NOCTTY,
        ) {
            Ok(fd) => fd,
            Err(err) => {
                warn!(
                    "⚠️ No se pudo abrir nodo DRM {:?} mediante libseat: {:?}",
                    path, err
                );
                errors.push(format!("{}: libseat open ({err:?})", path.display()));
                continue;
            }
        };

        let drm_device_fd = DrmDeviceFd::new(DeviceFd::from(raw_fd));
        let (drm_device, drm_notifier) = match DrmDevice::new(drm_device_fd.clone(), false) {
            Ok(pair) => pair,
            Err(err) => {
                warn!("⚠️ {:?} no soportó DrmDevice KMS: {:?}", path, err);
                errors.push(format!("{}: DrmDevice ({err:?})", path.display()));
                continue;
            }
        };
        let gbm_device = match GbmDevice::new(drm_device_fd) {
            Ok(gbm) => gbm,
            Err(err) => {
                warn!("⚠️ {:?} no soportó GbmDevice: {:?}", path, err);
                errors.push(format!("{}: GbmDevice ({err:?})", path.display()));
                continue;
            }
        };
        let egl_display = match unsafe { EGLDisplay::new(gbm_device.clone()) } {
            Ok(dpy) => dpy,
            Err(err) => {
                warn!("⚠️ {:?} no soportó EGLDisplay sobre GBM: {:?}", path, err);
                errors.push(format!("{}: EGLDisplay ({err:?})", path.display()));
                continue;
            }
        };
        let egl_context = match EGLContext::new(&egl_display) {
            Ok(ctx) => ctx,
            Err(err) => {
                warn!("⚠️ {:?} no soportó EGLContext: {:?}", path, err);
                errors.push(format!("{}: EGLContext ({err:?})", path.display()));
                continue;
            }
        };
        let renderer = match unsafe { GlesRenderer::new(egl_context) } {
            Ok(r) => r,
            Err(err) => {
                warn!("⚠️ {:?} no soportó GlesRenderer: {:?}", path, err);
                errors.push(format!("{}: GlesRenderer ({err:?})", path.display()));
                continue;
            }
        };

        return Ok((drm_device, drm_notifier, gbm_device, renderer, path.clone()));
    }

    anyhow::bail!(
        "Ningún nodo DRM pudo inicializarse ({})",
        if errors.is_empty() {
            "sin candidatos".to_string()
        } else {
            errors.join("; ")
        }
    );
}

struct DrmOutputSurface {
    output: Output,
    connector: connector::Handle,
    global: GlobalId,
    surface: GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>,
    crtc: crtc::Handle,
    modes: Vec<smithay::reexports::drm::control::Mode>,
    size: Size<i32, Physical>,
    damage_tracker: OutputDamageTracker,
    cursor_history: std::collections::VecDeque<Rectangle<i32, Physical>>,
    extra_damage_history: std::collections::VecDeque<Vec<Rectangle<i32, Physical>>>,
    last_window_geometries: std::collections::HashMap<
        smithay::backend::renderer::element::Id,
        (Rectangle<i32, Physical>, bool),
    >,
    last_window_logical_geoms: std::collections::HashMap<
        smithay::backend::renderer::element::Id,
        (Rectangle<i32, Logical>, bool),
    >,
    window_redraw_shadow_blur:
        std::collections::HashMap<smithay::backend::renderer::element::Id, usize>,
    popup_effect_ids: std::collections::HashMap<
        smithay::backend::renderer::element::Id,
        (
            smithay::backend::renderer::element::Id,
            smithay::backend::renderer::element::Id,
            Rectangle<i32, Physical>,
            smithay::backend::renderer::utils::CommitCounter,
        ),
    >,
    last_popup_count: usize,
    last_dock_rect: Option<Rectangle<i32, Physical>>,
    dock_redraw_buffers: usize,
    full_damage_buffers: usize,
    last_commit_counter: usize,
    last_output_commit_counter: usize,
    pending_damage_buffers: usize,
    flip_pending: Option<Instant>,
    last_cursor_draw_point: Option<(i32, i32)>,
    last_cursor_status: smithay::input::pointer::CursorImageStatus,
    last_cursor_scale_milli: i32,
    last_background_commit_counter: usize,
    last_background_size: (i32, i32),
    background_dirty: bool,
    background_snapshot_stale: bool,
    last_expose_revision: u64,
    last_workspace_revision: u64,
    hw_gamma_brightness_applied: bool,
}

fn popup_effect_ids_for(
    popup_effect_ids: &mut std::collections::HashMap<
        smithay::backend::renderer::element::Id,
        (
            smithay::backend::renderer::element::Id,
            smithay::backend::renderer::element::Id,
            Rectangle<i32, Physical>,
            smithay::backend::renderer::utils::CommitCounter,
        ),
    >,
    key: smithay::backend::renderer::element::Id,
    rect: Rectangle<i32, Physical>,
) -> (
    smithay::backend::renderer::element::Id,
    smithay::backend::renderer::element::Id,
    smithay::backend::renderer::utils::CommitCounter,
) {
    let entry = popup_effect_ids.entry(key).or_insert_with(|| {
        (
            smithay::backend::renderer::element::Id::new(),
            smithay::backend::renderer::element::Id::new(),
            rect,
            smithay::backend::renderer::utils::CommitCounter::default(),
        )
    });
    if entry.2 != rect {
        entry.2 = rect;
        entry.3.increment();
    }
    (entry.0.clone(), entry.1.clone(), entry.3)
}

impl DrmOutputSurface {
    fn is_flip_pending(&self) -> bool {
        self.flip_pending.map_or(false, |submitted_at| {
            submitted_at.elapsed() < Duration::from_millis(250)
        })
    }

    fn recover_expired_flip(&mut self) {
        if let Some(submitted_at) = self.flip_pending {
            if submitted_at.elapsed() >= Duration::from_millis(250) {
                warn!(
                    "⏱️ Timeout de VBlank DRM en {} ({:?}); liberando frame pendiente en swapchain",
                    self.output.name(),
                    submitted_at.elapsed()
                );
                let _ = self.surface.frame_submitted();
                self.flip_pending = None;
                self.pending_damage_buffers = self.pending_damage_buffers.max(1);
            }
        }
    }
}

struct ThemedCursor {
    texture: GlesTexture,
    size: u32,
    width: i32,
    height: i32,
    xhot: i32,
    yhot: i32,
}

fn create_gbm_surface_with_fallback(
    drm_device: &mut DrmDevice,
    gbm_device: &GbmDevice<DrmDeviceFd>,
    connector: connector::Handle,
    crtc: crtc::Handle,
    preferred_mode: smithay::reexports::drm::control::Mode,
    available_modes: &[smithay::reexports::drm::control::Mode],
    renderer_formats: &[Format],
    output_name: &str,
) -> Option<(
    GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>,
    smithay::reexports::drm::control::Mode,
)> {
    let color_formats = [Fourcc::Argb8888, Fourcc::Xrgb8888];
    let mut format_candidates = vec![None];
    for color_format in color_formats {
        for format in renderer_formats
            .iter()
            .filter(|format| format.code == color_format)
        {
            if !format_candidates.contains(&Some(*format)) {
                format_candidates.push(Some(*format));
            }
        }
    }

    let mut mode_candidates = vec![preferred_mode];
    let mut fallback_modes = available_modes
        .iter()
        .copied()
        .filter(|mode| {
            mode.size() != preferred_mode.size() || mode.vrefresh() != preferred_mode.vrefresh()
        })
        .collect::<Vec<_>>();
    fallback_modes.sort_by(|a, b| {
        let same_resolution = b
            .size()
            .eq(&preferred_mode.size())
            .cmp(&a.size().eq(&preferred_mode.size()));
        if same_resolution != std::cmp::Ordering::Equal {
            return same_resolution;
        }

        let area_a = u64::from(a.size().0) * u64::from(a.size().1);
        let area_b = u64::from(b.size().0) * u64::from(b.size().1);
        area_b
            .cmp(&area_a)
            .then_with(|| b.vrefresh().cmp(&a.vrefresh()))
    });
    for mode in fallback_modes {
        if !mode_candidates.iter().any(|candidate| {
            candidate.size() == mode.size() && candidate.vrefresh() == mode.vrefresh()
        }) {
            mode_candidates.push(mode);
        }
    }

    for (round, delay) in [Duration::ZERO, Duration::from_millis(500)]
        .into_iter()
        .enumerate()
    {
        if !delay.is_zero() {
            info!(
                "Reintentando inicializar {} tras una pausa de {} ms",
                output_name,
                delay.as_millis()
            );
            std::thread::sleep(delay);
        }

        let candidates_to_try = if round == 0 {
            format_candidates.as_slice()
        } else {
            &format_candidates[..1]
        };
        for mode in mode_candidates.iter().copied() {
            for candidate in candidates_to_try {
                let drm_surface = match drm_device.create_surface(crtc, mode, &[connector]) {
                    Ok(surface) => surface,
                    Err(err) => {
                        warn!(
                            "No se pudo crear DrmSurface para {} en {}x{} @ {} Hz (intento {}): {:?}",
                            output_name,
                            mode.size().0,
                            mode.size().1,
                            mode.vrefresh(),
                            round + 1,
                            err
                        );
                        break;
                    }
                };

                let (formats, formats_to_try): (&[Fourcc], Vec<Format>) = match candidate {
                    Some(format) => (std::slice::from_ref(&format.code), vec![*format]),
                    None => (&color_formats, renderer_formats.to_vec()),
                };
                let allocator = GbmAllocator::new(
                    gbm_device.clone(),
                    GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
                );

                match GbmBufferedSurface::new(drm_surface, allocator, formats, formats_to_try) {
                    Ok(surface) => {
                        if mode.size() != preferred_mode.size()
                            || mode.vrefresh() != preferred_mode.vrefresh()
                        {
                            warn!(
                                "KMS rechazó el modo preferido de {}. Se usará {}x{} @ {} Hz.",
                                output_name,
                                mode.size().0,
                                mode.size().1,
                                mode.vrefresh()
                            );
                        }
                        if let Some(format) = candidate {
                            info!(
                                "Superficie DRM de {} inicializada con formato alternativo {:?}",
                                output_name, format
                            );
                        }
                        return Some((surface, mode));
                    }
                    Err(err) => {
                        warn!(
                            "Falló la inicialización de {} en {}x{} @ {} Hz (intento {}, formato {:?}): {:?}",
                            output_name,
                            mode.size().0,
                            mode.size().1,
                            mode.vrefresh(),
                            round + 1,
                            candidate,
                            err
                        );
                    }
                }
            }
        }
    }

    error!(
        "No se pudo inicializar la superficie DRM de {} tras probar los modos y formatos compatibles",
        output_name
    );
    None
}

fn scan_connected_drm_connectors(
    drm_device: &mut DrmDevice,
    res_handles: &ResourceHandles,
    displays_cfg: &DisplaysConfig,
) -> Vec<(connector::Handle, connector::Info, String, bool)> {
    let mut connected_list = Vec::new();
    for &conn_handle in res_handles.connectors() {
        let conn_info = match drm_device.get_connector(conn_handle, true) {
            Ok(info) => info,
            Err(e) => {
                warn!("Error al leer connector {:?}: {:?}", conn_handle, e);
                continue;
            }
        };

        if conn_info.state() != connector::State::Connected {
            continue;
        }

        let conn_name = format!(
            "{}-{}",
            conn_info.interface().as_str(),
            conn_info.interface_id()
        );
        let is_primary = displays_cfg
            .find_for_connector(&conn_name)
            .map(|c| c.primary)
            .unwrap_or(false);
        connected_list.push((conn_handle, conn_info, conn_name, is_primary));
    }
    connected_list
}

fn select_hotplug_primary_index(
    outputs: &[DisplayOutputConfig],
    pinned_primary: Option<&str>,
) -> usize {
    pinned_primary
        .and_then(|pin| outputs.iter().position(|output| output.connector == pin))
        .or_else(|| outputs.iter().position(|output| output.primary))
        .unwrap_or(0)
}

fn sync_displays_config_from_connectors(
    connected_list: &[(connector::Handle, connector::Info, String, bool)],
    displays_cfg: &mut DisplaysConfig,
) {
    sync_displays_config_from_connectors_with_pinned_primary(connected_list, displays_cfg, None);
}

fn sync_displays_config_from_connectors_with_pinned_primary(
    connected_list: &[(connector::Handle, connector::Info, String, bool)],
    displays_cfg: &mut DisplaysConfig,
    pinned_primary: Option<&str>,
) {
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
        let scale = previous
            .as_ref()
            .map(|output| output.scale)
            .unwrap_or_else(|| {
                DisplayOutputConfig::recommended_scale(
                    width as u32,
                    height as u32,
                    detected_diagonal,
                )
            });
        let logical_width = (width as f64 / scale.max(0.5)).round() as i32;
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
            x: previous
                .as_ref()
                .map(|output| output.x)
                .unwrap_or_else(|| {
                    if pinned_primary.is_some() {
                        100_000 + detected_x
                    } else {
                        detected_x
                    }
                }),
            y: previous.as_ref().map(|output| output.y).unwrap_or(0),
            scale,
            primary: previous
                .as_ref()
                .map(|output| output.primary)
                .unwrap_or(idx == 0),
            brightness: previous
                .as_ref()
                .map(|output| output.brightness)
                .unwrap_or(100.0),
            diagonal_inches: previous
                .as_ref()
                .and_then(|output| output.diagonal_inches)
                .or(detected_diagonal),
        });
        detected_x += logical_width.max(100);
    }
    let primary_idx = select_hotplug_primary_index(&detected_outputs, pinned_primary);
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
}

#[allow(clippy::too_many_arguments)]
fn create_drm_output_surface_for_connector(
    drm_device: &mut DrmDevice,
    gbm_device: &GbmDevice<DrmDeviceFd>,
    renderer: &mut GlesRenderer,
    res_handles: &ResourceHandles,
    conn_handle: connector::Handle,
    conn_info: &connector::Info,
    conn_name: &str,
    displays_cfg: &DisplaysConfig,
    used_crtcs: &mut HashSet<crtc::Handle>,
    next_auto_x: &mut i32,
    state: &mut MalusState,
) -> Option<DrmOutputSurface> {
    info!("🔌 Monitor físico conectado detectado: {}", conn_name);

    let cfg_output = displays_cfg.find_for_connector(conn_name);
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
            return None;
        }
    };

    let possible_crtcs: Vec<crtc::Handle> = conn_info
        .encoders()
        .iter()
        .filter_map(|&enc_handle| drm_device.get_encoder(enc_handle).ok())
        .flat_map(|enc| res_handles.filter_crtcs(enc.possible_crtcs()))
        .collect();

    let active_crtc = conn_info
        .current_encoder()
        .and_then(|encoder| drm_device.get_encoder(encoder).ok())
        .and_then(|encoder| encoder.crtc())
        .filter(|crtc| possible_crtcs.contains(crtc));
    let crtc = match active_crtc
        .filter(|crtc| !used_crtcs.contains(crtc))
        .or_else(|| possible_crtcs.into_iter().find(|c| !used_crtcs.contains(c)))
    {
        Some(c) => {
            used_crtcs.insert(c);
            c
        }
        None => {
            warn!("No se encontró CRTC libre para el monitor {}", conn_name);
            return None;
        }
    };

    if active_crtc == Some(crtc) {
        info!("   ♻️ Reutilizando CRTC activo {:?} de {}", crtc, conn_name);
    }

    info!(
        "   🖥 Asignando CRTC {:?} a {} con modo {}x{} @ {}Hz",
        crtc,
        conn_name,
        mode.size().0,
        mode.size().1,
        mode.vrefresh()
    );

    let renderer_formats = renderer.dmabuf_formats().into_iter().collect::<Vec<_>>();
    let (gbm_surface, mode) = create_gbm_surface_with_fallback(
        drm_device,
        gbm_device,
        conn_handle,
        crtc,
        mode,
        conn_info.modes(),
        &renderer_formats,
        conn_name,
    )?;

    let (mode_w, mode_h) = mode.size();
    let edid_mm = conn_info
        .size()
        .filter(|&(w, h)| w > 0 && h > 0)
        .map(|(w, h)| (w as i32, h as i32));
    let edid_diag = edid_mm.map(|(w, h)| ((w as f64).powi(2) + (h as f64).powi(2)).sqrt() / 25.4);
    let (loc_x, loc_y, scale, phys_mm, model_label) = if let Some(cfg) = cfg_output {
        let use_edid_mm = match (edid_mm, edid_diag, cfg.diagonal_inches) {
            (Some(_), Some(ed), Some(manual_diag)) if (ed - manual_diag).abs() > 0.5 => false,
            (Some(_), _, _) => true,
            (None, _, _) => false,
        };
        let phys_mm = if use_edid_mm {
            edid_mm.unwrap()
        } else {
            cfg.physical_size_mm()
                .map(|(w_mm, h_mm)| (w_mm.round() as i32, h_mm.round() as i32))
                .unwrap_or((0, 0))
        };
        (cfg.x, cfg.y, cfg.scale.max(0.5), phys_mm, cfg.name.clone())
    } else {
        let fallback_scale = displays_cfg.master_scale.max(1.0);
        (
            *next_auto_x,
            0,
            fallback_scale,
            edid_mm.unwrap_or((0, 0)),
            conn_name.to_string(),
        )
    };

    let logical_w = (mode_w as f64 / scale).round() as i32;
    let logical_h = (mode_h as f64 / scale).round() as i32;
    *next_auto_x = (*next_auto_x).max(loc_x + logical_w);

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
        conn_name.to_string(),
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
    let global = output.create_global::<MalusState>(&state.display_handle);
    output.set_preferred(smithay_mode);
    output.change_current_state(
        Some(smithay_mode),
        Some(Transform::Normal),
        Some(Scale::Fractional(scale)),
        Some((loc_x, loc_y).into()),
    );
    state.space.map_output(&output, (loc_x, loc_y));

    let damage_tracker = OutputDamageTracker::from_output(&output);
    Some(DrmOutputSurface {
        output,
        connector: conn_handle,
        global,
        surface: gbm_surface,
        crtc,
        modes: conn_info.modes().to_vec(),
        size: (mode_w as i32, mode_h as i32).into(),
        damage_tracker,
        cursor_history: std::collections::VecDeque::with_capacity(4),
        extra_damage_history: std::collections::VecDeque::with_capacity(4),
        last_window_geometries: std::collections::HashMap::new(),
        last_window_logical_geoms: std::collections::HashMap::new(),
        window_redraw_shadow_blur: std::collections::HashMap::new(),
        popup_effect_ids: std::collections::HashMap::new(),
        last_popup_count: 0,
        last_dock_rect: None,
        dock_redraw_buffers: if ENABLE_DAMAGE_TRACKING { 3 } else { 0 },
        full_damage_buffers: if ENABLE_DAMAGE_TRACKING { 3 } else { 1 },
        last_commit_counter: 0,
        last_output_commit_counter: 0,
        pending_damage_buffers: 1,
        flip_pending: None,
        last_cursor_draw_point: None,
        last_cursor_status: smithay::input::pointer::CursorImageStatus::default_named(),
        last_cursor_scale_milli: 1000,
        last_background_commit_counter: usize::MAX,
        last_background_size: (0, 0),
        background_dirty: true,
        background_snapshot_stale: true,
        last_expose_revision: 0,
        last_workspace_revision: 0,
        hw_gamma_brightness_applied: false,
    })
}

fn apply_drm_crtc_brightness_to_surface(
    drm_device: &DrmDevice,
    s: &mut DrmOutputSurface,
    cfg: &DisplaysConfig,
) -> bool {
    let out_name = s.output.name();
    let factor = if malus_de_common::config::connector_has_hardware_backlight(&out_name) {
        1.0
    } else {
        cfg.find_for_connector(&out_name)
            .map(|o| o.brightness)
            .unwrap_or(cfg.brightness)
            .clamp(5.0, 100.0)
            / 100.0
    };
    let Ok(crtc_info) = drm_device.get_crtc(s.crtc) else {
        s.hw_gamma_brightness_applied = false;
        return false;
    };
    let gamma_len = crtc_info.gamma_length() as usize;
    if gamma_len <= 1 {
        s.hw_gamma_brightness_applied = false;
        return false;
    }
    let denom = (gamma_len - 1) as f64;
    let lut: Vec<u16> = (0..gamma_len)
        .map(|i| {
            ((i as f64 / denom) * factor * 65535.0)
                .round()
                .clamp(0.0, 65535.0) as u16
        })
        .collect();
    if drm_device.set_gamma(s.crtc, &lut, &lut, &lut).is_ok() {
        s.hw_gamma_brightness_applied = true;
        true
    } else {
        s.hw_gamma_brightness_applied = false;
        false
    }
}

fn apply_output_config_to_surface(
    s: &mut DrmOutputSurface,
    new_cfg: &DisplaysConfig,
    state: &mut MalusState,
    auto_x: &mut i32,
) -> (Point<i32, Logical>, f64) {
    let out_name = s.output.name();
    let (loc_x, loc_y, scale, requested_mode) =
        if let Some(cfg) = new_cfg.find_for_connector(&out_name) {
            let requested_mode = s
                .modes
                .iter()
                .find(|mode| {
                    let (width, height) = mode.size();
                    width as u32 == cfg.width
                        && height as u32 == cfg.height
                        && (mode.vrefresh() as f64 - cfg.refresh_rate).abs() < 1.0
                })
                .copied();
            (cfg.x, cfg.y, cfg.scale.clamp(0.5, 4.0), requested_mode)
        } else {
            (*auto_x, 0, new_cfg.master_scale.clamp(0.5, 4.0), None)
        };
    let mut mode_update = None;
    if let Some(mode) = requested_mode {
        let current_mode = s.surface.current_mode();
        if mode.size() != current_mode.size() || mode.vrefresh() != current_mode.vrefresh() {
            match s.surface.use_mode(mode) {
                Ok(()) => {
                    let (width, height) = mode.size();
                    s.size = (width as i32, height as i32).into();
                    s.full_damage_buffers = if ENABLE_DAMAGE_TRACKING { 3 } else { 1 };
                    s.flip_pending = None;
                    s.cursor_history.clear();
                    s.extra_damage_history.clear();
                    s.last_window_geometries.clear();
                    s.window_redraw_shadow_blur.clear();
                    s.dock_redraw_buffers = if ENABLE_DAMAGE_TRACKING { 3 } else { 0 };
                    s.pending_damage_buffers = 1;
                    s.background_dirty = true;
                    s.damage_tracker = OutputDamageTracker::from_output(&s.output);
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
    let log_h = (s.size.h as f64 / scale).round() as i32;
    *auto_x = (*auto_x).max(loc_x + log_w);

    if let Some(lock_surf) = state.ext_lock_surfaces.get(&out_name).cloned() {
        lock_surf.with_pending_state(|ls| {
            ls.size = Some(Size::from((log_w.max(1) as u32, log_h.max(1) as u32)));
        });
        lock_surf.send_configure();
    }

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

    s.full_damage_buffers = if ENABLE_DAMAGE_TRACKING { 3 } else { 1 };
    s.flip_pending = None;
    s.pending_damage_buffers = 1;
    s.cursor_history.clear();
    s.extra_damage_history.clear();
    s.last_window_geometries.clear();
    s.window_redraw_shadow_blur.clear();
    s.dock_redraw_buffers = if ENABLE_DAMAGE_TRACKING { 3 } else { 0 };
    s.damage_tracker = OutputDamageTracker::from_output(&s.output);
    (Point::from((loc_x, loc_y)), scale)
}

fn reconcile_drm_outputs(
    drm_device: &mut DrmDevice,
    gbm_device: &GbmDevice<DrmDeviceFd>,
    renderer: &mut GlesRenderer,
    shared_surfaces: &Arc<Mutex<Vec<DrmOutputSurface>>>,
    state: &mut MalusState,
) -> Option<DisplaysConfig> {
    let res_handles = match drm_device.resource_handles() {
        Ok(res) => res,
        Err(err) => {
            warn!("No se pudieron leer resource_handles en hotplug DRM: {err:?}");
            return None;
        }
    };
    let mut displays_cfg = DisplaysConfig::load();
    let mut connected_list = scan_connected_drm_connectors(drm_device, &res_handles, &displays_cfg);
    let connected_handles: HashSet<connector::Handle> =
        connected_list.iter().map(|(h, ..)| *h).collect();

    let mut removed_outputs = Vec::new();
    let (has_added, surviving_primary_connector) = {
        let Ok(mut surfs) = shared_surfaces.lock() else {
            return None;
        };
        if connected_list.is_empty() && !surfs.is_empty() {
            warn!("Hotplug DRM reportó 0 conectores momentáneamente; conservando salidas activas.");
            return None;
        }
        let existing_handles: HashSet<connector::Handle> =
            surfs.iter().map(|s| s.connector).collect();
        let any_added = connected_handles
            .iter()
            .any(|h| !existing_handles.contains(h));
        let any_removed = surfs
            .iter()
            .any(|s| !connected_handles.contains(&s.connector));
        if !any_added && !any_removed {
            return None;
        }

        let active_primary_name = state.primary_output().map(|o| o.name());
        let mut remaining = Vec::with_capacity(surfs.len());
        for surf in surfs.drain(..) {
            if connected_handles.contains(&surf.connector) {
                remaining.push(surf);
            } else {
                removed_outputs.push(surf);
            }
        }
        let pinned_primary = if any_added && !remaining.is_empty() {
            active_primary_name
                .filter(|name| remaining.iter().any(|s| s.output.name() == *name))
                .or_else(|| remaining.first().map(|s| s.output.name()))
        } else {
            None
        };
        *surfs = remaining;
        (any_added, pinned_primary)
    };

    let mut preserved_shell_layers = Vec::new();
    let mut removed_globals = Vec::with_capacity(removed_outputs.len());
    for removed in removed_outputs {
        info!("🔌 Monitor físico desconectado: {}", removed.output.name());
        {
            let mut lmap = smithay::desktop::layer_map_for_output(&removed.output);
            let layers: Vec<_> = lmap.layers().cloned().collect();
            for layer in layers {
                smithay::wayland::compositor::with_surface_tree_downward(
                    layer.wl_surface(),
                    (),
                    |_, _, _| smithay::wayland::compositor::TraversalAction::DoChildren(()),
                    |wl_surf, _, _| {
                        removed.output.leave(wl_surf);
                    },
                    |_, _, _| true,
                );
                if crate::state::is_primary_shell_layer_namespace(layer.namespace())
                    && !connected_list.is_empty()
                {
                    lmap.unmap_layer(&layer);
                    preserved_shell_layers.push(layer);
                } else if matches!(
                    layer.namespace(),
                    "malus-wallpaper" | "malus-desktop-icons"
                ) && !connected_list.is_empty()
                {
                    // Evita que gtk4-layer-shell cierre la ventana y GTK4 envíe commit tras destroy,
                    // lo que dispara Error::InvalidSize en Smithay 0.7.0 y mata a malus-de-wallpaper.
                    lmap.unmap_layer(&layer);
                } else {
                    layer.layer_surface().send_close();
                }
            }
        }
        state.space.unmap_output(&removed.output);
        let DrmOutputSurface {
            global, surface, ..
        } = removed;
        drop(surface);
        removed_globals.push(global);
    }

    sync_displays_config_from_connectors_with_pinned_primary(
        &connected_list,
        &mut displays_cfg,
        surviving_primary_connector.as_deref(),
    );
    connected_list.sort_by_key(|(_, _, name, _)| {
        !displays_cfg
            .find_for_connector(name)
            .map(|output| output.primary)
            .unwrap_or(false)
    });

    if let Ok(mut surfs) = shared_surfaces.lock() {
        if has_added {
            let mut used_crtcs: HashSet<crtc::Handle> = surfs.iter().map(|s| s.crtc).collect();
            let existing_handles: HashSet<connector::Handle> =
                surfs.iter().map(|s| s.connector).collect();
            let mut next_auto_x = 0;
            for (conn_handle, conn_info, conn_name, _) in &connected_list {
                if existing_handles.contains(conn_handle) {
                    continue;
                }
                if let Some(new_surf) = create_drm_output_surface_for_connector(
                    drm_device,
                    gbm_device,
                    renderer,
                    &res_handles,
                    *conn_handle,
                    conn_info,
                    conn_name,
                    &displays_cfg,
                    &mut used_crtcs,
                    &mut next_auto_x,
                    state,
                ) {
                    surfs.push(new_surf);
                }
            }
        }

        surfs.sort_by_key(|s| {
            !displays_cfg
                .find_for_connector(&s.output.name())
                .is_some_and(|o| o.primary)
        });
        let mut auto_x = 0;
        for s in surfs.iter_mut() {
            apply_output_config_to_surface(s, &displays_cfg, state, &mut auto_x);
            let _ = apply_drm_crtc_brightness_to_surface(drm_device, s, &displays_cfg);
            s.background_dirty = true;
            s.background_snapshot_stale = true;
        }
    }

    state.relocate_primary_shell_layers_for_config(&displays_cfg, preserved_shell_layers);
    state.migrate_orphaned_windows_and_pointer();
    if state.auto_tiling_enabled {
        state.relayout_auto_tiling();
    }
    for global in removed_globals {
        state.display_handle.remove_global::<MalusState>(global);
    }
    if let Ok(mut surfs) = shared_surfaces.lock() {
        for s in surfs.iter_mut() {
            s.full_damage_buffers = if ENABLE_DAMAGE_TRACKING { 3 } else { 1 };
            s.pending_damage_buffers = 1;
            s.dock_redraw_buffers = if ENABLE_DAMAGE_TRACKING { 3 } else { 0 };
            s.background_dirty = true;
            s.background_snapshot_stale = true;
        }
    }
    let _ = state.display_handle.flush_clients();
    state
        .ipc_broadcaster
        .broadcast(&IpcMessage::ReloadDisplaysConfig);
    Some(displays_cfg)
}

fn cursor_names_for_icon(icon: smithay::input::pointer::CursorIcon) -> &'static [&'static str] {
    use smithay::input::pointer::CursorIcon;
    match icon {
        CursorIcon::NResize | CursorIcon::NsResize => &[
            "n-resize",
            "ns-resize",
            "sb_v_double_arrow",
            "v_double_arrow",
            "size_ver",
            "top_side",
        ],
        CursorIcon::SResize => &[
            "s-resize",
            "ns-resize",
            "sb_v_double_arrow",
            "v_double_arrow",
            "size_ver",
            "bottom_side",
        ],
        CursorIcon::EResize | CursorIcon::EwResize => &[
            "e-resize",
            "ew-resize",
            "sb_h_double_arrow",
            "h_double_arrow",
            "size_hor",
            "right_side",
        ],
        CursorIcon::WResize => &[
            "w-resize",
            "ew-resize",
            "sb_h_double_arrow",
            "h_double_arrow",
            "size_hor",
            "left_side",
        ],
        CursorIcon::NwResize | CursorIcon::NwseResize => &[
            "nw-resize",
            "nwse-resize",
            "bd_double_arrow",
            "size_fdiag",
            "top_left_corner",
        ],
        CursorIcon::SeResize => &[
            "se-resize",
            "nwse-resize",
            "bd_double_arrow",
            "size_fdiag",
            "bottom_right_corner",
        ],
        CursorIcon::NeResize | CursorIcon::NeswResize => &[
            "ne-resize",
            "nesw-resize",
            "fd_double_arrow",
            "size_bdiag",
            "top_right_corner",
        ],
        CursorIcon::SwResize => &[
            "sw-resize",
            "nesw-resize",
            "fd_double_arrow",
            "size_bdiag",
            "bottom_left_corner",
        ],
        CursorIcon::Pointer => &["pointer", "hand2", "hand1", "pointing_hand"],
        CursorIcon::Text => &["text", "xterm", "ibeam"],
        CursorIcon::Grab => &["grab", "openhand"],
        CursorIcon::Grabbing => &["grabbing", "closedhand", "fleur"],
        _ => &["default", "left_ptr", "arrow"],
    }
}

const EMBEDDED_MACOS_LEFT_PTR: &[u8] =
    include_bytes!("../../../../assets/cursors/macOS/cursors/left_ptr");

fn parse_xcursor_bytes_to_texture(
    renderer: &mut GlesRenderer,
    content: &[u8],
    cursor_size: u32,
) -> Option<ThemedCursor> {
    let images = xcursor::parser::parse_xcursor(content)?;
    let best = images
        .iter()
        .min_by_key(|image| (image.size.abs_diff(cursor_size), image.size < cursor_size))?;
    let size = Size::from((best.width as i32, best.height as i32));
    let texture = renderer
        .import_memory(&best.pixels_rgba, Fourcc::Argb8888, size, false)
        .ok()?;
    Some(ThemedCursor {
        texture,
        size: best.size,
        width: best.width as i32,
        height: best.height as i32,
        xhot: best.xhot as i32,
        yhot: best.yhot as i32,
    })
}

fn load_cursor_texture_by_names(
    renderer: &mut GlesRenderer,
    theme: &str,
    cursor_size: u32,
    names: &[&str],
) -> Option<ThemedCursor> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let mut icon_dirs = vec![
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/cursors").to_string(),
        "/usr/local/share/malus-de/cursors".to_string(),
        format!("{}/.icons", home),
        format!("{}/.local/share/icons", home),
        "/usr/share/icons".to_string(),
        "/usr/share/pixmaps".to_string(),
    ];
    if let Ok(xcursor_path) = std::env::var("XCURSOR_PATH") {
        icon_dirs.extend(
            xcursor_path
                .split(':')
                .filter(|dir| !dir.is_empty())
                .map(str::to_string),
        );
    }
    let primary_theme = if theme.is_empty() || theme == "default" || theme == "Adwaita" {
        "macOS"
    } else {
        theme
    };
    let mut theme_candidates = vec![primary_theme];
    for fallback in [
        "macOS",
        "macOS-White",
        "Adwaita",
        "default",
        "AdwaitaLegacy",
    ] {
        if !theme_candidates.contains(&fallback) {
            theme_candidates.push(fallback);
        }
    }
    let possible_paths = theme_candidates
        .iter()
        .flat_map(|candidate_theme| {
            let dirs = &icon_dirs;
            dirs.iter().flat_map(move |dir| {
                names
                    .iter()
                    .map(move |name| format!("{}/{}/cursors/{}", dir, candidate_theme, name))
            })
        })
        .collect::<Vec<_>>();

    for path in &possible_paths {
        if let Ok(content) = std::fs::read(path) {
            if let Some(cur) = parse_xcursor_bytes_to_texture(renderer, &content, cursor_size) {
                return Some(cur);
            }
        }
    }

    // Si algún nombre de cursor específico falla o no existe en el tema, usar siempre el cursor macOS integrado
    for dir in &icon_dirs {
        for fallback_name in ["default", "left_ptr", "arrow"] {
            let macos_fallback_path = format!("{}/macOS/cursors/{}", dir, fallback_name);
            if let Ok(content) = std::fs::read(&macos_fallback_path) {
                if let Some(cur) = parse_xcursor_bytes_to_texture(renderer, &content, cursor_size) {
                    return Some(cur);
                }
            }
        }
    }

    parse_xcursor_bytes_to_texture(renderer, EMBEDDED_MACOS_LEFT_PTR, cursor_size)
}

fn create_cursor_texture(
    renderer: &mut GlesRenderer,
    theme: &str,
    cursor_size: u32,
) -> Option<ThemedCursor> {
    if let Some(cur) = load_cursor_texture_by_names(
        renderer,
        theme,
        cursor_size,
        &["default", "left_ptr", "arrow"],
    ) {
        return Some(cur);
    }

    if let Some(cur) =
        parse_xcursor_bytes_to_texture(renderer, EMBEDDED_MACOS_LEFT_PTR, cursor_size)
    {
        return Some(cur);
    }

    warn!(
        "No se encontró el cursor predeterminado del tema '{}'; usando el cursor de respaldo",
        theme
    );
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
    let texture = renderer
        .import_memory(&data, Fourcc::Argb8888, size, false)
        .ok()?;
    Some(ThemedCursor {
        texture,
        size: 16,
        width: 16,
        height: 16,
        xhot: 0,
        yhot: 0,
    })
}

/// Genera las 4 texturas de recorte squircle (n=4.2) para esquinas de pantalla estilo display moderno
pub fn create_squircle_corner_textures(
    renderer: &mut GlesRenderer,
    radius_px: i32,
) -> Option<[GlesTexture; 4]> {
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
                    1 => (x as f64 + 0.5, r_f - (y as f64 + 0.5)),         // Superior Derecha
                    2 => (r_f - (x as f64 + 0.5), y as f64 + 0.5),         // Inferior Izquierda
                    _ => (x as f64 + 0.5, y as f64 + 0.5),                 // Inferior Derecha
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
                warn!(
                    "❌ [Squircle Corners] Error al importar memoria en OpenGL (esquina {}): {:?}",
                    corner, e
                );
                return None;
            }
        };
        corners.push(tex);
    }

    let arr: [GlesTexture; 4] = corners.try_into().ok()?;
    info!("✅ [Squircle Corners] 4 texturas de recorte squircle (n=4.2) creadas exitosamente para radio {} px", r);
    Some(arr)
}

fn find_layer_under_layer(
    state: &MalusState,
    point: Point<f64, Logical>,
    target_layer: Layer,
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
            .layer_under(target_layer, output_local_point)
            .filter(|l| {
                !(l.namespace() == "malus-app-switcher" && !state.app_switcher_classic_active())
            });

        if let Some(layer) = under_layer {
            if let Some(geo) = layer_map.layer_geometry(layer) {
                let local_point = output_local_point - geo.loc.to_f64();
                let surface_hit = if !state.active_popups.is_empty() {
                    layer.surface_under(local_point, smithay::desktop::WindowSurfaceType::POPUP)
                } else {
                    None
                };
                let surface_hit = surface_hit.or_else(|| {
                    if layer.namespace() == "malus-top-bar"
                        && local_point.y >= (geo.size.h as f64).max(36.0)
                    {
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

fn find_layer_under(
    state: &MalusState,
    point: Point<f64, Logical>,
) -> Option<(
    smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    Point<f64, Logical>,
    String,
    bool,
)> {
    if state.is_session_locked() {
        for output in state.space.outputs() {
            let Some(output_geo) = state.space.output_geometry(output) else {
                continue;
            };
            if !output_geo.to_f64().contains(point) {
                continue;
            }
            if let Some(ls) = state.authorized_ext_lock_surface_for_output(&output.name()) {
                return Some((
                    ls.wl_surface().clone(),
                    output_geo.loc.to_f64(),
                    "ext-session-lock".to_string(),
                    true,
                ));
            }
            let layer_map = smithay::desktop::layer_map_for_output(output);
            let matched = layer_map
                .layers()
                .rev()
                .find(|l| l.namespace() == "malus-lock")
                .and_then(|layer| {
                    layer_map.layer_geometry(layer).and_then(|geo| {
                        let local_point = point - output_geo.loc.to_f64() - geo.loc.to_f64();
                        layer
                            .surface_under(
                                local_point,
                                smithay::desktop::WindowSurfaceType::TOPLEVEL
                                    | smithay::desktop::WindowSurfaceType::SUBSURFACE,
                            )
                            .map(|(surf, sub_loc)| {
                                (
                                    surf,
                                    (output_geo.loc + geo.loc + sub_loc).to_f64(),
                                    layer.namespace().to_string(),
                                    layer.can_receive_keyboard_focus(),
                                )
                            })
                    })
                });
            if let Some(hit) = matched {
                return Some(hit);
            }
        }
        return None;
    }

    if let Some(hit) = find_layer_under_layer(state, point, Layer::Overlay) {
        return Some(hit);
    }

    // Para Layer::Top (Top Bar, Dock): si el monitor donde cae el puntero tiene una ventana
    // en pantalla completa, Layer::Top queda por debajo de esa ventana (wlr-layer-shell spec).
    let output_has_fullscreen = state
        .space
        .outputs()
        .find(|o| {
            state
                .space
                .output_geometry(o)
                .map(|g| g.to_f64().contains(point))
                .unwrap_or(false)
        })
        .map(|o| state.has_fullscreen_window_on_output(o))
        .unwrap_or(false);

    if !output_has_fullscreen {
        if let Some(hit) = find_layer_under_layer(state, point, Layer::Top) {
            return Some(hit);
        }
    }
    None
}

fn find_layer_popup_under(
    state: &MalusState,
    point: Point<f64, Logical>,
) -> Option<(
    smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    Point<f64, Logical>,
)> {
    if state.active_popups.is_empty() {
        return None;
    }
    for output in state.space.outputs() {
        let Some(output_geo) = state.space.output_geometry(output) else {
            continue;
        };
        if !output_geo.to_f64().contains(point) {
            continue;
        }
        let output_local_point = point - output_geo.loc.to_f64();
        let layer_map = smithay::desktop::layer_map_for_output(output);
        for layer in layer_map.layers() {
            if let Some(geo) = layer_map.layer_geometry(layer) {
                let local_point = output_local_point - geo.loc.to_f64();
                if let Some((surf, sub_loc)) =
                    layer.surface_under(local_point, smithay::desktop::WindowSurfaceType::POPUP)
                {
                    return Some((surf, (output_geo.loc + geo.loc + sub_loc).to_f64()));
                }
            }
        }
    }
    None
}

fn clamp_pointer_location(
    state: &MalusState,
    mut point: Point<f64, Logical>,
) -> Point<f64, Logical> {
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
    let min_x = geos
        .iter()
        .map(|g| g.loc.x as f64)
        .fold(f64::INFINITY, f64::min);
    let max_x = geos
        .iter()
        .map(|g| (g.loc.x + g.size.w) as f64)
        .fold(f64::NEG_INFINITY, f64::max);
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
    Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<f64, Logical>,
    )>,
    Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        String,
        bool,
    )>,
    Option<(smithay::desktop::Window, Point<i32, Logical>)>,
    bool,
) {
    let layer_hit = find_layer_under(state, point);
    if state.is_session_locked() {
        return if let Some((surf, focus_loc, ns, can_kb)) = layer_hit {
            (
                Some((surf.clone(), focus_loc)),
                Some((surf, ns, can_kb)),
                None,
                false,
            )
        } else {
            (None, None, None, false)
        };
    }
    if let Some((surf, focus_loc, ns, can_kb)) = layer_hit {
        (
            Some((surf.clone(), focus_loc)),
            Some((surf, ns, can_kb)),
            None,
            false,
        )
    } else if let Some((surf, focus_loc)) = find_layer_popup_under(state, point) {
        (Some((surf, focus_loc)), None, None, true)
    } else if let Some((_window, loc, surf, sub_loc)) = state.popup_under(point) {
        (Some((surf, (loc + sub_loc).to_f64())), None, None, true)
    } else if let Some((win, _edge, loc)) = crate::grabs::find_window_resize_edge_under(
        &state.space,
        point,
        8.0,
        state.ssd.buttons_right,
    ) {
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
        find_layer_under_layer(state, point, Layer::Bottom)
            .or_else(|| find_layer_under_layer(state, point, Layer::Background))
    {
        (
            Some((surf.clone(), focus_loc)),
            Some((surf, ns, can_kb)),
            None,
            false,
        )
    } else {
        (None, None, None, false)
    }
}

pub fn run_drm() -> Result<()> {
    info!("============================================================");
    info!("🚀 MALUS DE: Iniciando en modo DRM/KMS nativo (Direct TTY/GDM)");
    info!("============================================================");

    let mut event_loop: EventLoop<MalusState> =
        EventLoop::try_new().context("Error al inicializar EventLoop de Calloop")?;
    let mut display: Display<MalusState> =
        Display::new().context("Error al crear Display Wayland")?;
    let mut state = MalusState::new(&display);

    // 1. Inicializar sesión de hardware (LibSeat / logind)
    let (mut session, notifier) = match LibSeatSession::new() {
        Ok(res) => res,
        Err(err) => {
            error!(
                "❌ Error fatal: no se pudo inicializar LibSeatSession: {:?}",
                err
            );
            eprintln!("⚠️ LibSeat no pudo tomar control del asiento TTY ({err:?}).");
            eprintln!("   Si estás dentro de una sesión gráfica existente, ejecuta Malus DE en modo anidado:");
            eprintln!("   malus-de-compositor --backend winit");
            anyhow::bail!("LibSeatSession initialization failed: {:?}", err);
        }
    };
    let seat_name = session.seat();
    info!("✅ Sesión de hardware activa en asiento: '{}'", seat_name);

    // 2. Detectar y abrir el mejor nodo DRM disponible vía LibSeat (probando todas las GPUs)
    let primary_candidate = primary_gpu(&seat_name).ok().flatten();
    let udev_candidates = all_gpus(&seat_name).unwrap_or_default();
    let dev_dri_candidates = scan_dev_dri_cards();
    let gpu_candidates = prioritize_drm_gpu_candidates(
        primary_candidate,
        udev_candidates,
        dev_dri_candidates,
        sysfs_drm_card_score,
    );
    info!(
        "🎮 Candidatos DRM detectados (orden de prioridad): {:?}",
        gpu_candidates
    );

    let (mut drm_device, drm_notifier, gbm_device, mut renderer, chosen_gpu) =
        open_first_working_drm_gpu(&mut session, &gpu_candidates)
            .context("Error al abrir nodo DRM mediante libseat")?;
    info!("🎮 GPU seleccionada y operativa: {:?}", chosen_gpu);

    info!("🎨 Renderer OpenGL ES acelerado por hardware listo");

    // Obtener el dev_id del nodo DRM primario (/dev/dri/cardX) para filtrar eventos udev hotplug
    let primary_drm_dev_id =
        smithay::backend::drm::DrmNode::from_file(std::os::fd::AsFd::as_fd(&drm_device))
            .ok()
            .map(|node| node.dev_id());

    // Exponer protocolo Wayland zwp_linux_dmabuf_v1 para compartición zero-copy en GPU (Chrome, Firefox, Electron)
    let dmabuf_formats = renderer.dmabuf_formats();
    let drm_render_node =
        smithay::backend::drm::DrmNode::from_file(std::os::fd::AsFd::as_fd(&drm_device))
            .ok()
            .and_then(|node| {
                node.node_with_type(smithay::backend::drm::NodeType::Render)
                    .and_then(Result::ok)
                    .or(Some(node))
            });
    if let Some(render_node) = drm_render_node {
        match smithay::wayland::dmabuf::DmabufFeedbackBuilder::new(
            render_node.dev_id(),
            dmabuf_formats.iter().copied(),
        )
        .build()
        {
            Ok(default_feedback) => {
                let global = state
                    .dmabuf_state
                    .create_global_with_default_feedback::<MalusState>(
                        &state.display_handle,
                        &default_feedback,
                    );
                state.dmabuf_global = Some(global);
                info!(
                    "🚀 linux-dmabuf v5 (zero-copy GPU) habilitado en {:?}",
                    render_node
                );
            }
            Err(err) => {
                warn!(
                    "⚠️ No se pudo construir DmabufFeedback ({:?}); usando linux-dmabuf v3",
                    err
                );
                let global = state
                    .dmabuf_state
                    .create_global::<MalusState>(&state.display_handle, dmabuf_formats);
                state.dmabuf_global = Some(global);
            }
        }
    } else {
        let global = state
            .dmabuf_state
            .create_global::<MalusState>(&state.display_handle, dmabuf_formats);
        state.dmabuf_global = Some(global);
        info!("🚀 linux-dmabuf v3 (zero-copy GPU) habilitado");
    }

    let (mut cursor_theme, mut cursor_size) = malus_de_common::config::load_cursor_config();
    let mut cursor_texture = create_cursor_texture(&mut renderer, &cursor_theme, cursor_size);

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
            warn!(
                "⚠️ Falló carga de XKB layout '{}': {:?}. Usando fallback US.",
                input_cfg.keyboard_layout, e
            );
            state
                .seat
                .add_keyboard(Default::default(), 200, 200)
                .map_err(|e| anyhow::anyhow!("Error al inicializar teclado fallback: {e:?}"))?
        }
    };
    let _pointer = state.seat.add_pointer();
    let _touch = state.seat.add_touch();

    // 5. Descubrir conectores físicos y configurar superficies de pantalla
    let res_handles = drm_device
        .resource_handles()
        .context("Error al consultar resource handles de DRM")?;

    // Desactivar cualquier cursor por hardware residual de GDM o compositores previos en todos los CRTCs
    for &crtc_handle in res_handles.crtcs() {
        #[allow(deprecated)]
        if let Err(e) = drm_device
            .set_cursor::<smithay::reexports::drm::control::dumbbuffer::DumbBuffer>(
                crtc_handle,
                None,
            )
        {
            tracing::debug!(
                "Aviso al limpiar hardware cursor en CRTC {:?}: {:?}",
                crtc_handle,
                e
            );
        }
    }

    let mut displays_cfg = DisplaysConfig::load();
    let mut used_crtcs = HashSet::new();
    let mut surfaces = Vec::new();

    // Recopilar conectores conectados y ordenarlos de modo que el monitor primario sea el primero
    let mut connected_list =
        scan_connected_drm_connectors(&mut drm_device, &res_handles, &displays_cfg);
    sync_displays_config_from_connectors(&connected_list, &mut displays_cfg);

    // Monitor primario siempre en el índice 0 de state.space.outputs()
    connected_list.sort_by_key(|(_, _, name, _)| {
        !displays_cfg
            .find_for_connector(name)
            .map(|output| output.primary)
            .unwrap_or(false)
    });

    let mut next_auto_x: i32 = 0;
    for (conn_handle, conn_info, conn_name, _) in connected_list {
        if let Some(mut surface) = create_drm_output_surface_for_connector(
            &mut drm_device,
            &gbm_device,
            &mut renderer,
            &res_handles,
            conn_handle,
            &conn_info,
            &conn_name,
            &displays_cfg,
            &mut used_crtcs,
            &mut next_auto_x,
            &mut state,
        ) {
            let _ = apply_drm_crtc_brightness_to_surface(&drm_device, &mut surface, &displays_cfg);
            surfaces.push(surface);
        }
    }

    if surfaces.is_empty() {
        warn!("⚠️ No se detectó ninguna pantalla activa conectada en DRM.");
    }

    // Centrar el puntero inmediatamente en el centro de la pantalla primaria para evitar coordenadas vacías
    let primary_center = state.primary_output().and_then(|primary| {
        let geo = state.space.output_geometry(&primary)?;
        Some(geo.loc.to_f64() + (geo.size.to_f64() / 2.0))
    });

    if let Some(center) = primary_center {
        state.last_pointer_location = Some(center);
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
            info!(
                "🎯 Puntero DRM inicializado en el centro de la pantalla: {:?}",
                center
            );
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
    info!(
        "🌐 Socket Wayland escuchando activamente en: WAYLAND_DISPLAY={}",
        socket_name
    );
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);
    state.socket_name = socket_name.clone();
    let _clipboard_manager = crate::start_clipboard_manager(&socket_name);

    let shared_surfaces = Arc::new(Mutex::new(surfaces));
    let surfaces_for_drm = shared_surfaces.clone();
    let surfaces_for_vt = shared_surfaces.clone();

    // 7. Inicializar Libinput para teclado y ratón
    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    if let Err(e) = libinput_context.udev_assign_seat(&seat_name) {
        warn!("Aviso al asignar asiento en libinput: {:?}", e);
    }
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    // 8. Registrar fuentes de eventos en el EventLoop de Calloop
    let loop_handle = event_loop.handle();

    let session_active = Arc::new(AtomicBool::new(session.is_active()));
    let session_active_vt = session_active.clone();

    // Notificador de pausa / reactivación de sesión TTY (VT Switch)
    loop_handle
        .insert_source(
            notifier,
            move |event, &mut (), state: &mut MalusState| match event {
                SessionEvent::PauseSession => {
                    info!("⏸️ Sesión TTY pausada (VT switch away)");
                    session_active_vt.store(false, Ordering::Relaxed);
                    libinput_context.suspend();
                }
                SessionEvent::ActivateSession => {
                    info!("▶️ Sesión TTY reactivada (VT switch back)");
                    session_active_vt.store(true, Ordering::Relaxed);
                    state.pending_drm_hotplug = true;
                    if let Err(e) = libinput_context.resume() {
                        error!("Error al reanudar libinput tras VT switch: {:?}", e);
                    }
                    if let Ok(mut surfs) = surfaces_for_vt.lock() {
                        for s in surfs.iter_mut() {
                            s.full_damage_buffers = if ENABLE_DAMAGE_TRACKING { 3 } else { 1 };
                            s.flip_pending = None;
                            s.pending_damage_buffers = 1;
                            s.cursor_history.clear();
                            s.extra_damage_history.clear();
                            s.last_window_geometries.clear();
                            s.window_redraw_shadow_blur.clear();
                            s.dock_redraw_buffers = if ENABLE_DAMAGE_TRACKING { 3 } else { 0 };
                        }
                    }
                }
            },
        )
        .map_err(|e| anyhow::anyhow!("Error al registrar notificador de sesión: {e:?}"))?;

    // Notificador de eventos de sincronización DRM (VBlank y Page Flip)
    loop_handle
        .insert_source(
            drm_notifier,
            move |event, _metadata, _state: &mut MalusState| match event {
                DrmEvent::VBlank(crtc) => {
                    if let Ok(mut surfs) = surfaces_for_drm.lock() {
                        for s in surfs.iter_mut() {
                            if s.crtc == crtc {
                                let _ = s.surface.frame_submitted();
                                s.flip_pending = None;
                            }
                        }
                    }
                }
                DrmEvent::Error(err) => {
                    warn!("Aviso en evento DRM: {:?}", err);
                }
            },
        )
        .map_err(|e| anyhow::anyhow!("Error al registrar notificador DRM: {e:?}"))?;

    // Notificador udev de conexión/desconexión en caliente de monitores (DRM hotplug)
    match UdevBackend::new(&seat_name) {
        Ok(udev_backend) => {
            if let Err(err) = loop_handle.insert_source(
                udev_backend,
                move |event, _, state: &mut MalusState| {
                    let matches_gpu = match event {
                        UdevEvent::Changed { device_id }
                        | UdevEvent::Added { device_id, .. }
                        | UdevEvent::Removed { device_id } => {
                            primary_drm_dev_id.is_none_or(|id| id == device_id)
                        }
                    };
                    if matches_gpu {
                        info!(
                            "🔌 Evento udev DRM recibido ({:?}); programando reconciliación de pantallas",
                            event
                        );
                        state.pending_drm_hotplug = true;
                    }
                },
            ) {
                warn!("No se pudo registrar UdevBackend para hotplug de monitores: {err:?}");
            }
        }
        Err(err) => {
            warn!("No se pudo inicializar UdevBackend para hotplug de monitores: {err:?}");
        }
    }

    // Despertar inmediatamente cuando algún cliente Wayland escriba commits o frame requests
    let display_poll_fd = display
        .backend()
        .poll_fd()
        .try_clone_to_owned()
        .context("No se pudo clonar fd de Wayland Display para calloop")?;
    loop_handle
        .insert_source(
            smithay::reexports::calloop::generic::Generic::new(
                display_poll_fd,
                smithay::reexports::calloop::Interest::READ,
                smithay::reexports::calloop::Mode::Level,
            ),
            |_readiness, _fd, _state| Ok(smithay::reexports::calloop::PostAction::Continue),
        )
        .map_err(|e| anyhow::anyhow!("Error al registrar Wayland Display en calloop: {e:?}"))?;

    // Despacho de eventos de Libinput con soporte para ratón, touchpad y atajos completos
    spawn_emergency_zap_watchdog(session_active, state.session_locked_flag.clone());
    let mut session_vt = session.clone();
    let libinput_devices: std::rc::Rc<std::cell::RefCell<Vec<LibinputDevice>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let libinput_devices_ev = libinput_devices.clone();

    loop_handle.insert_source(libinput_backend, move |event, _, state: &mut MalusState| {
        match event {
            InputEvent::DeviceAdded { mut device } => {
                apply_libinput_device_config(&mut device, &state.input_cfg);
                libinput_devices_ev.borrow_mut().push(device);
            }
            InputEvent::DeviceRemoved { device } => {
                libinput_devices_ev.borrow_mut().retain(|d| d != &device);
            }
            InputEvent::Keyboard { event } => {
                let key = event.key_code().raw();
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
                                let has_shift = modifiers.shift;
                                let has_alt = modifiers.alt;
                                let has_ctrl = modifiers.ctrl;

                                // 🚨 RED DE SEGURIDAD CRÍTICA (deshabilitado Ctrl+Alt+BackSpace si la sesión está bloqueada):
                                // 1. Ctrl + Alt + BackSpace: fuerza el cierre limpio del compositor/entorno (solo desbloqueado).
                                // 2. Ctrl + Alt + F1..F12: cambio inmediato de TTY vía LibSeat.
                                if has_ctrl && has_alt {
                                    let is_bksp = sym == keysyms::KEY_BackSpace.into()
                                        || sym == keysyms::KEY_Terminate_Server.into()
                                        || keysym_handle.raw_syms().iter().any(|&s| {
                                            s == keysyms::KEY_BackSpace.into()
                                                || s == keysyms::KEY_Terminate_Server.into()
                                        });
                                    if is_bksp {
                                        if state.is_session_locked() {
                                            tracing::warn!("🔒 [SECURITY] Ctrl+Alt+BackSpace bloqueado mientras la sesión está bloqueada");
                                            return FilterResult::Intercept(());
                                        }
                                        state.is_running = false;
                                        trigger_emergency_compositor_exit("Ctrl+Alt+BackSpace");
                                        return FilterResult::Intercept(());
                                    }

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

                                // Teclas multimedia y de hardware (Volumen, Silencio, Micrófono, Brillo, Teclado, MPRIS, etc.)
                                if let Some(media_action) = crate::media_keys::detect_media_key_action(
                                    sym,
                                    &keysym_handle.raw_syms(),
                                    key,
                                    modifiers,
                                ) {
                                    if !state.is_session_locked() || media_action.is_allowed_when_locked() {
                                        let now = Instant::now();
                                        state.media_keys.register_key_press(key, media_action, now);
                                        state.execute_media_key_action(media_action, now);
                                        return FilterResult::Intercept(());
                                    }
                                }

                                // Mientras la sesión está bloqueada, ningún atajo del entorno debe abrir
                                // Spotlight, AppSwitcher, Exposé ni capturas sobre el lockscreen.
                                if state.is_session_locked() {
                                    return FilterResult::Forward;
                                }

                                let is_print = sym == keysyms::KEY_Print.into() || key == 99;
                                let is_s = sym == keysyms::KEY_s.into() || sym == keysyms::KEY_S.into();
                                let is_r = sym == keysyms::KEY_r.into() || sym == keysyms::KEY_R.into();
                                let is_t = sym == keysyms::KEY_t.into() || sym == keysyms::KEY_T.into();
                                let is_3 = sym == keysyms::KEY_3.into() || sym == keysyms::KEY_numbersign.into();
                                let is_4 = sym == keysyms::KEY_4.into() || sym == keysyms::KEY_dollar.into();
                                let is_5 = sym == keysyms::KEY_5.into() || sym == keysyms::KEY_percent.into();
                                let is_esc_early = sym == keysyms::KEY_Escape.into();

                                // Detener grabación de pantalla activa con ⌃⌘⎋ (macOS nativo) o ⇧⌘R
                                if state.active_screen_recording.is_some()
                                    && ((has_ctrl && has_super && is_esc_early)
                                        || (has_super && has_shift && is_r))
                                {
                                    info!("⏹ Atajo para detener grabación de pantalla detectado");
                                    state.stop_screen_recording();
                                    return FilterResult::Intercept(());
                                }

                                if state.screenshot_ui.is_open() {
                                    if state.screenshot_ui.handle_key_press(sym, *modifiers) {
                                        state.execute_screenshot_ui_action();
                                    }
                                    return FilterResult::Intercept(());
                                }

                                if sym == keysyms::KEY_Escape.into() && state.snap_layouts.handle_escape() {
                                    return FilterResult::Intercept(());
                                }

                                if has_super && has_shift && !has_alt && !has_ctrl && is_t {
                                    info!("🔤 Atajo de Extractor OCR en pantalla detectado (Super+Shift+T)");
                                    state.start_ocr_screenshot();
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

                                let is_tab = sym == keysyms::KEY_Tab.into()
                                    || sym == keysyms::KEY_ISO_Left_Tab.into()
                                    || key == 23;
                                let is_return = sym == keysyms::KEY_Return.into() || sym == keysyms::KEY_KP_Enter.into();
                                let is_left = sym == keysyms::KEY_Left.into();
                                let is_right = sym == keysyms::KEY_Right.into();
                                let is_q = sym == keysyms::KEY_q.into() || sym == keysyms::KEY_Q.into();
                                let is_w = sym == keysyms::KEY_w.into() || sym == keysyms::KEY_W.into();
                                let is_h = sym == keysyms::KEY_h.into() || sym == keysyms::KEY_H.into();
                                let is_m = sym == keysyms::KEY_m.into() || sym == keysyms::KEY_M.into();
                                let is_esc = sym == keysyms::KEY_Escape.into();

                                // 2. Navegación activa dentro del App Switcher (⌘Tab HUD / Deck)
                                if state.app_switcher.is_active {
                                    if is_return {
                                        info!("🎯 AppSwitcher commit vía Return/Enter, enfocando ventana");
                                        state.commit_app_switcher();
                                        return FilterResult::Intercept(());
                                    } else if is_tab {
                                        state.step_app_switcher(!modifiers.shift);
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

                                let is_space = sym == keysyms::KEY_space.into();
                                let is_k = sym == keysyms::KEY_k.into() || sym == keysyms::KEY_K.into();

                                // Spotlight (Super+Espacio / Alt+Espacio / Super+K)
                                if (is_space && (has_super || has_alt || has_ctrl)) || (is_k && has_super) {
                                    info!("🔍 Spotlight alternado vía atajo de teclado");
                                    state.toggle_spotlight();
                                    return FilterResult::Intercept(());
                                }

                                let is_y = sym == keysyms::KEY_y.into() || sym == keysyms::KEY_Y.into();
                                if has_super && !has_alt && !has_ctrl && is_y {
                                    info!("🪟 Auto-Tiling alternado vía Super+Y");
                                    state.toggle_auto_tiling();
                                    return FilterResult::Intercept(());
                                }

                                if state.spotlight.is_open() {
                                    state.spotlight.handle_key_press(sym, *modifiers);
                                    return FilterResult::Intercept(());
                                }

                                let is_up = sym == keysyms::KEY_Up.into();
                                let is_down = sym == keysyms::KEY_Down.into();

                                // Exposé / Mission Control (Ctrl+Flecha Arriba) y App Exposé (Ctrl+Flecha Abajo)
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

                                // Cambiar espacio de trabajo virtual (Ctrl+Flecha Izquierda / Derecha)
                                if has_ctrl && !has_super && !has_alt && is_left {
                                    state.switch_workspace_delta(-1);
                                    return FilterResult::Intercept(());
                                }
                                if has_ctrl && !has_super && !has_alt && is_right {
                                    state.switch_workspace_delta(1);
                                    return FilterResult::Intercept(());
                                }

                                // Cerrar ventana activa (Super+W / Super+Q / Alt+F4) y bloqueo de pantalla (Super+L / Ctrl+Super+Q)
                                let is_l = sym == keysyms::KEY_l.into() || sym == keysyms::KEY_L.into();
                                if (has_super && !has_alt && is_l) || (has_super && has_ctrl && !has_alt && is_q) {
                                    info!("🔒 Bloqueando pantalla mediante atajo de teclado");
                                    state.lock_session();
                                    return FilterResult::Intercept(());
                                }

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
                                state.media_keys.handle_key_release(key);
                                // Liberación de modificadores (Super/Alt) -> Commit de App Switcher
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
            }
            InputEvent::PointerMotion { event } => {
                if let Some(pointer) = state.seat.get_pointer() {
                    let mut current_pos = pointer.current_location();
                    current_pos.x += event.delta_x();
                    current_pos.y += event.delta_y();
                    let point = clamp_pointer_location(state, current_pos);
                    state.last_pointer_location = Some(point);
                    state.shake_cursor.record_motion(
                        event.delta_x(),
                        event.delta_y(),
                        Instant::now(),
                        state.input_cfg.shake_to_find_cursor,
                    );
                    state.spotlight.update_dock_drag_location(point);

                    if state.spotlight.is_open() {
                        if let Some(spot_output_geo) = state.spotlight_output_geometry() {
                            state.spotlight.update_pointer_motion(spot_output_geo, point);
                        }
                    }
                    if state.screenshot_ui.is_open() {
                        let output_geo = state.screenshot_ui_output_geometry();
                        let win_rect =
                            state.window_rect_under_point_in_output(output_geo, point);
                        state.screenshot_ui.update_hovered_window(win_rect);
                        state.screenshot_ui.update_pointer_motion(output_geo, point);
                    }
                    state.update_spotlight_drag_dock_hover(point);
                    let _ = state.handle_app_switcher_deck_pointer_motion(point);

                    if state.handle_expose_pointer_motion(point) {
                        let serial = SERIAL_COUNTER.next_serial();
                        pointer.motion(
                            state,
                            None,
                            &MotionEvent {
                                location: point,
                                serial,
                                time: event.time_msec(),
                            },
                        );
                        pointer.frame(state);
                        return;
                    }

                    let (under_target, _, _, _) = resolve_target_under(state, point);

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
                    state.update_ssd_hover(point);
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
                        state.last_pointer_location = Some(point);
                        state.shake_cursor.record_motion(
                            point.x - cur.x,
                            point.y - cur.y,
                            Instant::now(),
                            state.input_cfg.shake_to_find_cursor,
                        );
                        state.spotlight.update_dock_drag_location(point);

                        if state.spotlight.is_open() {
                            let spot_output_geo = state
                                .spotlight_output_geometry()
                                .unwrap_or(output_geo);
                            state.spotlight.update_pointer_motion(spot_output_geo, point);
                        }
                        if state.screenshot_ui.is_open() {
                            let ui_output_geo = state.screenshot_ui_output_geometry();
                            let win_rect =
                                state.window_rect_under_point_in_output(ui_output_geo, point);
                            state.screenshot_ui.update_hovered_window(win_rect);
                            state
                                .screenshot_ui
                                .update_pointer_motion(ui_output_geo, point);
                        }
                        state.update_spotlight_drag_dock_hover(point);
                        let _ = state.handle_app_switcher_deck_pointer_motion(point);

                        if state.handle_expose_pointer_motion(point) {
                            let serial = SERIAL_COUNTER.next_serial();
                            pointer.motion(
                                state,
                                None,
                                &MotionEvent {
                                    location: point,
                                    serial,
                                    time: event.time_msec(),
                                },
                            );
                            pointer.frame(state);
                            return;
                        }

                        let (under_target, _, _, _) = resolve_target_under(state, point);

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
                        state.update_ssd_hover(point);
                    }
                }
            }
            InputEvent::PointerButton { event } => {
                if let Some(pointer) = state.seat.get_pointer() {
                    let point = pointer.current_location();
                    state.last_pointer_location = Some(point);
                    let is_pressed = event.state() == ButtonState::Pressed;

                    let (under_target, layer_info, regular_win, is_popup) = resolve_target_under(state, point);

                    let btn_serial = SERIAL_COUNTER.next_serial();
                    if state.handle_expose_pointer_button(point, event.button_code(), is_pressed) {
                        return;
                    }
                    if is_pressed {
                        if state.screenshot_ui.is_open() {
                            let output_geo = state.screenshot_ui_output_geometry();
                            let win_rect =
                                state.window_rect_under_point_in_output(output_geo, point);
                            state.screenshot_ui.update_hovered_window(win_rect);
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
                            if let Some(spot_output_geo) = state.spotlight_output_geometry() {
                                if state.spotlight.handle_pointer_button(
                                    spot_output_geo,
                                    point,
                                    event.button_code(),
                                ) {
                                    return;
                                }
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

                        // Liberar grabs residuales / popups ANTES de sincronizar el foco de puntero
                        // para que `pointer.motion` no entregue el evento a un grab antiguo.
                        let pre_grab_serial = SERIAL_COUNTER.next_serial();
                        state.prepare_pointer_press_grabs(
                            clicked_on_popup,
                            pre_grab_serial,
                            event.time_msec(),
                        );

                        let focus_changed =
                            pointer.current_focus().as_ref() != under_target.as_ref().map(|(s, _)| s);
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
                        if focus_changed {
                            pointer.frame(state);
                        }

                        let (has_super, has_alt) = state
                            .seat
                            .get_keyboard()
                            .map(|kb| {
                                let mods = kb.modifier_state();
                                (mods.logo, mods.alt)
                            })
                            .unwrap_or((false, false));

                        if event.button_code() == 0x110
                            && !has_super
                            && !has_alt
                            && layer_info.is_none()
                            && !clicked_on_popup
                            && !state.popup_grab_active()
                        {
                            if let Some((resize_win, edge, win_loc)) =
                                crate::grabs::find_window_resize_edge_under(
                                    &state.space,
                                    point,
                                    8.0,
                                    state.ssd.buttons_right,
                                )
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
                                    if let Some(keyboard) = state.seat.get_keyboard() {
                                        if keyboard.current_focus().as_ref() != Some(surface.as_ref()) {
                                            let kb_serial = SERIAL_COUNTER.next_serial();
                                            keyboard.set_focus(
                                                state,
                                                Some(surface.clone().into_owned()),
                                                kb_serial,
                                            );
                                            state.notify_active_app(Some(&surface));
                                        } else {
                                            state.sync_window_activation(Some(surface.as_ref()));
                                        }
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
                                    state,
                                    grab,
                                    btn_serial,
                                    smithay::input::pointer::Focus::Clear,
                                );
                                pointer.button(
                                    state,
                                    &ButtonEvent {
                                        button: event.button_code(),
                                        state: event.state(),
                                        serial: btn_serial,
                                        time: event.time_msec(),
                                    },
                                );
                                pointer.frame(state);
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
                                state,
                                &ButtonEvent {
                                    button: event.button_code(),
                                    state: event.state(),
                                    serial: btn_serial,
                                    time: event.time_msec(),
                                },
                            );
                            pointer.frame(state);
                            return;
                        }

                        if let Some((surf, ns, can_kb)) = layer_info {
                            if can_kb {
                                let kb_serial = SERIAL_COUNTER.next_serial();
                                state.set_keyboard_focus_with_sync(Some(surf), kb_serial);
                                if matches!(ns.as_str(), "malus-desktop-icons" | "malus-wallpaper")
                                {
                                    state.notify_active_app(None);
                                }
                            }
                        } else if let Some(keyboard) = state.seat.get_keyboard() {
                            if let Some((window, loc)) = regular_win {
                                let was_already_top = state.space.elements().last().map(|w| w == &window).unwrap_or(false);
                                if !was_already_top {
                                    state.space.raise_element(&window, true);
                                }
                                if let Some(surface) = window.wl_surface() {
                                    let was_focused =
                                        keyboard.current_focus().as_ref() == Some(surface.as_ref());
                                    let kb_serial = SERIAL_COUNTER.next_serial();
                                    state.set_keyboard_focus_with_sync(
                                        Some(surface.clone().into_owned()),
                                        kb_serial,
                                    );
                                    if !was_focused {
                                        state.notify_active_app(Some(&surface));
                                    }
                                }

                                if !has_super && !has_alt && event.button_code() == 0x110 {
                                    state.record_csd_close_click_hint(&window, point);
                                }

                                if (has_super || has_alt) && event.button_code() == 0x110 {
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
                                        initial_window_location: loc,
                                        is_snapped,
                                        has_dragged: false,
                                    };
                                    state.ssd.grab_active = true;
                                    state.moving_window_surface = window.wl_surface().map(|s| s.into_owned());
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
                                    state.ssd.grab_active = true;
                                    state.ssd.cursor_override = true;
                                    state.cursor_status =
                                        smithay::input::pointer::CursorImageStatus::Named(
                                            crate::grabs::cursor_icon_for_resize_edge(edges),
                                        );
                                    pointer.set_grab(state, grab, btn_serial, smithay::input::pointer::Focus::Clear);
                                }
                            }
                        }
                    } else {
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
                        if state.screenshot_ui.is_open() {
                            let output_geo = state.screenshot_ui_output_geometry();
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
                            if let Some(spot_output_geo) = state.spotlight_output_geometry() {
                                let dock_geo = state.dock_surface_geometry().map(|(geo, _)| geo);
                                if let Some(outcome) =
                                    state.spotlight.handle_pointer_button_release(
                                        spot_output_geo,
                                        point,
                                        event.button_code(),
                                        dock_geo,
                                    )
                                {
                                    state.update_spotlight_drag_dock_hover(point);
                                    state.handle_spotlight_pointer_release(spot_output_geo, outcome);
                                    return;
                                }
                            }
                        }
                        if state.handle_snap_layouts_pointer_button(point, event.button_code(), false) {
                            return;
                        }
                        if state.handle_app_switcher_deck_pointer_button(point, event.button_code(), false) {
                            return;
                        }
                        if event.button_code() == 0x110 && !pointer.is_grabbed() {
                            if let Some((ref window, _)) = regular_win {
                                state.record_csd_close_click_hint(window, point);
                            }
                        }
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
                if let Some(pointer) = state.seat.get_pointer() {
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
                    if event.amount(Axis::Horizontal).is_some()
                        || event.amount(Axis::Vertical).is_some()
                    {
                        pointer.axis(state, frame);
                        pointer.frame(state);
                    }
                }
            }
            InputEvent::GestureSwipeBegin { event } => {
                if !state.handle_gesture_swipe_begin(event.fingers()) {
                    if let Some(pointer) = state.seat.get_pointer() {
                        pointer.gesture_swipe_begin(
                            state,
                            &smithay::input::pointer::GestureSwipeBeginEvent {
                                serial: SERIAL_COUNTER.next_serial(),
                                time: event.time_msec(),
                                fingers: event.fingers(),
                            },
                        );
                    }
                }
            }
            InputEvent::GestureSwipeUpdate { event } => {
                if !state.handle_gesture_swipe_update(event.delta_x(), event.delta_y()) {
                    if let Some(pointer) = state.seat.get_pointer() {
                        pointer.gesture_swipe_update(
                            state,
                            &smithay::input::pointer::GestureSwipeUpdateEvent {
                                time: event.time_msec(),
                                delta: (event.delta_x(), event.delta_y()).into(),
                            },
                        );
                    }
                }
            }
            InputEvent::GestureSwipeEnd { event } => {
                if !state.handle_gesture_swipe_end(event.cancelled()) {
                    if let Some(pointer) = state.seat.get_pointer() {
                        pointer.gesture_swipe_end(
                            state,
                            &smithay::input::pointer::GestureSwipeEndEvent {
                                serial: SERIAL_COUNTER.next_serial(),
                                time: event.time_msec(),
                                cancelled: event.cancelled(),
                            },
                        );
                    }
                }
            }
            InputEvent::GesturePinchBegin { event } => {
                if !state.handle_gesture_pinch_begin(event.fingers())
                    && (state.input_cfg.trackpad_pinch_zoom
                        || state.input_cfg.trackpad_rotate_gesture)
                {
                    if let Some(pointer) = state.seat.get_pointer() {
                        pointer.gesture_pinch_begin(
                            state,
                            &smithay::input::pointer::GesturePinchBeginEvent {
                                serial: SERIAL_COUNTER.next_serial(),
                                time: event.time_msec(),
                                fingers: event.fingers(),
                            },
                        );
                    }
                }
            }
            InputEvent::GesturePinchUpdate { event } => {
                if !state.handle_gesture_pinch_update(event.scale())
                    && (state.input_cfg.trackpad_pinch_zoom
                        || state.input_cfg.trackpad_rotate_gesture)
                {
                    if let Some(pointer) = state.seat.get_pointer() {
                        pointer.gesture_pinch_update(
                            state,
                            &smithay::input::pointer::GesturePinchUpdateEvent {
                                time: event.time_msec(),
                                delta: (event.delta_x(), event.delta_y()).into(),
                                scale: if state.input_cfg.trackpad_pinch_zoom {
                                    event.scale()
                                } else {
                                    1.0
                                },
                                rotation: if state.input_cfg.trackpad_rotate_gesture {
                                    event.rotation()
                                } else {
                                    0.0
                                },
                            },
                        );
                    }
                }
            }
            InputEvent::GesturePinchEnd { event } => {
                if !state.handle_gesture_pinch_end(event.cancelled())
                    && (state.input_cfg.trackpad_pinch_zoom
                        || state.input_cfg.trackpad_rotate_gesture)
                {
                    if let Some(pointer) = state.seat.get_pointer() {
                        pointer.gesture_pinch_end(
                            state,
                            &smithay::input::pointer::GesturePinchEndEvent {
                                serial: SERIAL_COUNTER.next_serial(),
                                time: event.time_msec(),
                                cancelled: event.cancelled(),
                            },
                        );
                    }
                }
            }
            InputEvent::GestureHoldBegin { event } => {
                if let Some(pointer) = state.seat.get_pointer() {
                    pointer.gesture_hold_begin(
                        state,
                        &smithay::input::pointer::GestureHoldBeginEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time: event.time_msec(),
                            fingers: event.fingers(),
                        },
                    );
                }
            }
            InputEvent::GestureHoldEnd { event } => {
                if let Some(pointer) = state.seat.get_pointer() {
                    pointer.gesture_hold_end(
                        state,
                        &smithay::input::pointer::GestureHoldEndEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time: event.time_msec(),
                            cancelled: event.cancelled(),
                        },
                    );
                }
            }
            InputEvent::TouchDown { event } => {
                if let Some(touch) = state.seat.get_touch() {
                    let output = state
                        .primary_output()
                        .or_else(|| state.space.outputs().next().cloned());
                    if let Some(output) = output {
                        let output_geo = state.space.output_geometry(&output).unwrap_or_default();
                        let mut point = event.position_transformed(output_geo.size);
                        point.x += output_geo.loc.x as f64;
                        point.y += output_geo.loc.y as f64;
                        let point = clamp_pointer_location(state, point);
                        state.last_pointer_location = Some(point);

                        let (under_target, layer_info, regular_win, _) =
                            resolve_target_under(state, point);
                        if let Some((ref win, _)) = regular_win {
                            let was_already_top = state
                                .space
                                .elements()
                                .last()
                                .map(|w| w == win)
                                .unwrap_or(false);
                            if !was_already_top {
                                state.space.raise_element(win, true);
                            }
                            if let Some(surface) = win.wl_surface() {
                                if let Some(keyboard) = state.seat.get_keyboard() {
                                    if keyboard.current_focus().as_ref() != Some(surface.as_ref()) {
                                        let kb_serial = SERIAL_COUNTER.next_serial();
                                        keyboard.set_focus(
                                            state,
                                            Some(surface.clone().into_owned()),
                                            kb_serial,
                                        );
                                        state.notify_active_app(Some(&surface));
                                    } else {
                                        state.sync_window_activation(Some(surface.as_ref()));
                                    }
                                }
                            }
                        } else if let Some((ref surf, _, can_kb)) = layer_info {
                            if can_kb {
                                if let Some(keyboard) = state.seat.get_keyboard() {
                                    if keyboard.current_focus().as_ref() != Some(surf) {
                                        let kb_serial = SERIAL_COUNTER.next_serial();
                                        keyboard.set_focus(state, Some(surf.clone()), kb_serial);
                                    }
                                }
                            }
                        }

                        touch.down(
                            state,
                            under_target,
                            &smithay::input::touch::DownEvent {
                                slot: event.slot(),
                                location: point,
                                serial: SERIAL_COUNTER.next_serial(),
                                time: event.time_msec(),
                            },
                        );
                    }
                }
            }
            InputEvent::TouchMotion { event } => {
                if let Some(touch) = state.seat.get_touch() {
                    let output = state
                        .primary_output()
                        .or_else(|| state.space.outputs().next().cloned());
                    if let Some(output) = output {
                        let output_geo = state.space.output_geometry(&output).unwrap_or_default();
                        let mut point = event.position_transformed(output_geo.size);
                        point.x += output_geo.loc.x as f64;
                        point.y += output_geo.loc.y as f64;
                        let point = clamp_pointer_location(state, point);
                        state.last_pointer_location = Some(point);
                        let (under_target, _, _, _) = resolve_target_under(state, point);
                        touch.motion(
                            state,
                            under_target,
                            &smithay::input::touch::MotionEvent {
                                slot: event.slot(),
                                location: point,
                                time: event.time_msec(),
                            },
                        );
                    }
                }
            }
            InputEvent::TouchUp { event } => {
                if let Some(touch) = state.seat.get_touch() {
                    touch.up(
                        state,
                        &smithay::input::touch::UpEvent {
                            slot: event.slot(),
                            serial: SERIAL_COUNTER.next_serial(),
                            time: event.time_msec(),
                        },
                    );
                }
            }
            InputEvent::TouchFrame { .. } => {
                if let Some(touch) = state.seat.get_touch() {
                    touch.frame(state);
                }
            }
            InputEvent::TouchCancel { .. } => {
                if let Some(touch) = state.seat.get_touch() {
                    touch.cancel(state);
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
    let _start_time = Instant::now();

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
    let keyfile_path = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from(".config"))
        .join("glib-2.0/settings/keyfile");
    let mut last_keyfile_mtime = std::fs::metadata(&keyfile_path)
        .and_then(|m| m.modified())
        .ok();
    let mut last_config_check = Instant::now();
    let mut current_displays_cfg = DisplaysConfig::load();
    let mut squircle_corner_cache: std::collections::HashMap<i32, [GlesTexture; 4]> =
        std::collections::HashMap::new();
    let mut named_cursor_cache: std::collections::HashMap<
        smithay::input::pointer::CursorIcon,
        Option<ThemedCursor>,
    > = std::collections::HashMap::new();
    let mut blur_pipeline: Option<std::sync::Arc<std::sync::Mutex<crate::blur::GaussianBlur>>> =
        None;
    let mut blur_init_attempted = false;
    let mut window_squircle_shader: Option<
        std::sync::Arc<crate::window_corners::WindowSquircleShader>,
    > = None;
    let mut window_squircle_init_attempted = false;
    let mut shadow_pipeline: Option<std::sync::Arc<crate::shadow::WindowShadowPipeline>> = None;
    let mut shadow_init_attempted = false;
    let mut black_hole_pipeline: Option<crate::black_hole::BlackHolePipeline> = None;
    let mut black_hole_init_attempted = false;
    let mut active_black_holes: Vec<crate::black_hole::ActiveBlackHoleClose> = Vec::new();
    let mut genie_pipeline: Option<crate::magic_lamp::GeniePipeline> = None;
    let mut genie_init_attempted = false;
    let mut genie_snapshots: std::collections::HashMap<
        u64,
        crate::capture::CapturedWindowOffscreen,
    > = std::collections::HashMap::new();
    let mut theme_transition_pipeline: Option<crate::theme_transition::ThemeTransitionPipeline> =
        None;
    let mut theme_transition_init_attempted = false;
    let mut workspace_transition_pipeline: Option<
        crate::workspace_transition::WorkspaceTransitionPipeline,
    > = None;
    let mut workspace_transition_init_attempted = false;
    let mut was_session_locked = false;

    while state.is_running {
        let frame_start = Instant::now();
        state.check_session_lock_liveness();
        let session_locked_now = state.is_session_locked();
        if session_locked_now != was_session_locked {
            was_session_locked = session_locked_now;
            let full_bufs = if ENABLE_DAMAGE_TRACKING { 3 } else { 1 };
            if let Ok(mut surfs) = shared_surfaces.lock() {
                for s in surfs.iter_mut() {
                    s.full_damage_buffers = s.full_damage_buffers.max(full_bufs);
                    s.pending_damage_buffers = s.pending_damage_buffers.max(full_bufs);
                }
            }
        }
        state.resume_timed_out_dock_animations(frame_start);
        state.tick_snap_layouts(frame_start);
        state.tick_media_keys(frame_start);
        state.tick_app_switcher_deck(frame_start);
        if state.snap_layouts.take_closed_dirty()
            || state.media_keys.take_closed_dirty()
            || state.take_app_switcher_deck_closed_dirty()
        {
            let full_bufs = if ENABLE_DAMAGE_TRACKING { 3 } else { 1 };
            if let Ok(mut surfs) = shared_surfaces.lock() {
                for s in surfs.iter_mut() {
                    s.full_damage_buffers = s.full_damage_buffers.max(full_bufs);
                    s.pending_damage_buffers = s.pending_damage_buffers.max(full_bufs);
                }
            }
        }

        // Si la sesión TTY no está activa (VT switch away), pausar renderizado para no saturar GPU
        if !session.is_active() {
            std::thread::sleep(Duration::from_millis(30));
            let _ = event_loop.dispatch(Some(Duration::from_millis(1)), &mut state);
            continue;
        }

        // Reconciliación en caliente (hotplug) de monitores conectados/desconectados
        if state.pending_drm_hotplug {
            state.pending_drm_hotplug = false;
            if let Some(updated_cfg) = reconcile_drm_outputs(
                &mut drm_device,
                &gbm_device,
                &mut renderer,
                &shared_surfaces,
                &mut state,
            ) {
                current_displays_cfg = updated_cfg;
                last_monitors_mtime = std::fs::metadata(&monitors_toml_path)
                    .and_then(|m| m.modified())
                    .ok();
                squircle_corner_cache.clear();
            }
        }

        // Hot-reload automático de monitors.toml e input.toml (inmediato vía IPC o cada 250ms por mtime)
        let check_now = state.pending_displays_reload
            || state.pending_brightness_reload
            || last_config_check.elapsed() >= Duration::from_millis(250);
        if check_now {
            last_config_check = Instant::now();
            let cur_keyfile_mtime = std::fs::metadata(&keyfile_path)
                .and_then(|m| m.modified())
                .ok();
            if cur_keyfile_mtime != last_keyfile_mtime {
                last_keyfile_mtime = cur_keyfile_mtime;
                let (configured_cursor_theme, configured_cursor_size) =
                    malus_de_common::config::load_cursor_config();
                if configured_cursor_theme != cursor_theme || configured_cursor_size != cursor_size
                {
                    cursor_theme = configured_cursor_theme;
                    cursor_size = configured_cursor_size;
                    cursor_texture =
                        create_cursor_texture(&mut renderer, &cursor_theme, cursor_size);
                    named_cursor_cache.clear();
                }
            }
            let current_mtime = std::fs::metadata(&monitors_toml_path)
                .and_then(|m| m.modified())
                .ok();
            if state.pending_displays_reload
                || state.pending_brightness_reload
                || (current_mtime.is_some() && current_mtime != last_monitors_mtime)
            {
                state.pending_displays_reload = false;
                state.pending_brightness_reload = false;
                last_monitors_mtime = current_mtime;
                let prev_cfg = current_displays_cfg.clone();
                current_displays_cfg = DisplaysConfig::load();
                current_displays_cfg.recalculate_layout();
                let new_cfg = &current_displays_cfg;
                let structural_unchanged = prev_cfg.outputs.len() == new_cfg.outputs.len()
                    && (prev_cfg.screen_corner_radius - new_cfg.screen_corner_radius).abs()
                        < 0.01
                    && (prev_cfg.window_corner_radius - new_cfg.window_corner_radius).abs()
                        < 0.01
                    && prev_cfg.outputs.iter().zip(&new_cfg.outputs).all(|(a, b)| {
                        a.connector == b.connector
                            && a.width == b.width
                            && a.height == b.height
                            && (a.refresh_rate - b.refresh_rate).abs() < 0.1
                            && a.x == b.x
                            && a.y == b.y
                            && (a.scale - b.scale).abs() < 0.0005
                            && a.primary == b.primary
                    });
                if structural_unchanged {
                    if let Ok(mut surfs) = shared_surfaces.lock() {
                        for s in surfs.iter_mut() {
                            if !apply_drm_crtc_brightness_to_surface(&drm_device, s, new_cfg) {
                                s.full_damage_buffers = if ENABLE_DAMAGE_TRACKING { 3 } else { 1 };
                                s.pending_damage_buffers = 1;
                            }
                        }
                    }
                } else {
                squircle_corner_cache.clear();
                info!("🔄 Recargando disposición y escalas de pantallas en vivo desde monitors.toml...");

                // Capturar geometrías previas de cada monitor para migrar ventanas si los monitores se intercambian o desplazan
                let mut prev_output_geos: std::collections::HashMap<
                    String,
                    Rectangle<i32, Logical>,
                > = std::collections::HashMap::new();
                for out in state.space.outputs() {
                    if let Some(geo) = state.space.output_geometry(out) {
                        prev_output_geos.insert(out.name(), geo);
                    }
                }

                let mut new_output_locs: std::collections::HashMap<
                    String,
                    (Point<i32, Logical>, f64),
                > = std::collections::HashMap::new();

                if let Ok(mut surfs) = shared_surfaces.lock() {
                    let mut auto_x = 0;
                    for s in surfs.iter_mut() {
                        let out_name = s.output.name();
                        let (loc, scale) =
                            apply_output_config_to_surface(s, new_cfg, &mut state, &mut auto_x);
                        let _ = apply_drm_crtc_brightness_to_surface(&drm_device, s, new_cfg);
                        new_output_locs.insert(out_name.clone(), (loc, scale));
                        s.full_damage_buffers = if ENABLE_DAMAGE_TRACKING { 3 } else { 1 };
                        s.pending_damage_buffers = 1;
                        info!(
                            "   ✅ Monitor {} actualizado en vivo: pos=({}, {}), escala={:.4}x ({:.1}%)",
                            out_name,
                            loc.x,
                            loc.y,
                            scale,
                            scale * 100.0
                        );
                    }
                }

                let moved_shell_layers =
                    state.relocate_primary_shell_layers_for_config(new_cfg, Vec::new());
                if moved_shell_layers {
                    if let Ok(mut surfs) = shared_surfaces.lock() {
                        for s in surfs.iter_mut() {
                            s.background_dirty = true;
                            s.background_snapshot_stale = true;
                            s.full_damage_buffers = if ENABLE_DAMAGE_TRACKING { 3 } else { 1 };
                            s.dock_redraw_buffers = if ENABLE_DAMAGE_TRACKING { 3 } else { 0 };
                            s.pending_damage_buffers = 1;
                        }
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
                            .or_else(|| {
                                prev_output_geos
                                    .iter()
                                    .find(|(_, geo)| geo.contains(win_loc))
                            })
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

                    state.ensure_window_below_topbar(&win);

                    if let Some(wl_surf) = win.wl_surface() {
                        let new_scale = state.preferred_scale_for_surface(&wl_surf);
                        smithay::wayland::compositor::with_states(&wl_surf, |states| {
                            smithay::wayland::fractional_scale::with_fractional_scale(
                                states,
                                |fs| {
                                    fs.set_preferred_scale(new_scale);
                                },
                            );
                        });
                    }
                    if let Some(toplevel) = win.toplevel() {
                        toplevel.send_configure();
                    }
                }
                if state.auto_tiling_enabled {
                    state.relayout_auto_tiling();
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
                state
                    .apply_keyboard_layout(&input_cfg.keyboard_layout, &input_cfg.keyboard_variant);
                state.input_cfg = input_cfg;
                state.input_config_dirty = true;
            }
        }

        if state.input_config_dirty {
            state.input_config_dirty = false;
            for dev in libinput_devices.borrow_mut().iter_mut() {
                apply_libinput_device_config(dev, &state.input_cfg);
            }
        }

        // Aceptar nuevos clientes de forma no bloqueante
        while let Ok(Some(stream)) = listener.accept() {
            match display
                .handle()
                .insert_client(stream, Arc::new(ClientState::default()))
            {
                Ok(_client) => {
                    info!("🎉 Nuevo cliente Wayland conectado al compositor DRM!");
                }
                Err(err) => warn!("Error al registrar cliente Wayland en DRM: {:?}", err),
            }
        }

        // Despachar eventos de hardware (libinput, sesión, VBlank, socket Wayland) dirigidos por VBlank por monitor.
        // Si algún monitor NO tiene un flip pendiente y ya acumula trabajo listo para pintar, se despacha sin espera.
        // En caso contrario, calloop duerme sin consumir CPU y despierta al instante cuando llega un VBlank, entrada o commit.
        let compositor_anim_active = !state.active_animations.is_empty()
            || !state.pending_black_holes.is_empty()
            || !active_black_holes.is_empty()
            || !state.snap_transitions.is_empty()
            || state.expose.is_animating()
            || state.snap_layouts.is_open()
            || state.media_keys.is_osd_open()
            || state.media_keys.has_active_repeat()
            || state.app_switcher.deck.active
            || state.theme_transition_started_at.is_some()
            || state.pending_theme_transition_capture.is_some()
            || state.workspace_transition.is_some()
            || state.pending_workspace_switch.is_some()
            || state.shake_cursor.is_animating(Instant::now());
        let has_ready_output = if let Ok(mut surfs) = shared_surfaces.lock() {
            surfs.iter_mut().any(|s| {
                s.recover_expired_flip();
                if s.is_flip_pending() {
                    return false;
                }
                let out_name = s.output.name();
                compositor_anim_active
                    || state.active_screen_recording.as_ref().is_some_and(|rec| {
                        rec.matches_output(&out_name) && rec.needs_render_wakeup(Instant::now())
                    })
                    || s.pending_damage_buffers > 0
                    || s.full_damage_buffers > 0
                    || s.last_commit_counter != state.surface_commit_counter
                    || s.last_output_commit_counter != state.output_commit_counter(&out_name)
                    || s.last_expose_revision != state.expose.render_revision
                    || s.last_workspace_revision != state.workspace_revision
            })
        } else {
            false
        };
        let dispatch_timeout = if has_ready_output {
            Duration::ZERO
        } else {
            Duration::from_millis(16)
        };
        let _ = event_loop.dispatch(Some(dispatch_timeout), &mut state);
        let _ = display.flush_clients();

        state.process_ipc_messages();
        state.tick_pending_hot_corner();
        let shake_frame_now = Instant::now();
        state.shake_cursor.prune_if_finished(shake_frame_now);
        let cursor_shake_scale = state.shake_cursor.scale_at(shake_frame_now);
        let cursor_scale_milli = (cursor_shake_scale * 1000.0).round() as i32;
        let effective_cursor_size = cursor_size as f64 * cursor_shake_scale;
        let shake_cursor_animating = state.shake_cursor.is_animating(shake_frame_now);
        display.dispatch_clients(&mut state)?;
        if state.process_pending_dmabuf_imports(&mut renderer) {
            let _ = display.flush_clients();
        }
        state.space.refresh();
        let expose_was_active = state.expose.active;
        let expose_animating = state.expose.tick();
        let expose_just_closed = expose_was_active && !state.expose.active;

        for output in state.space.outputs() {
            let mut map = smithay::desktop::layer_map_for_output(output);
            map.cleanup();
        }
        state.popups.cleanup();
        state.active_popups.retain(|p| p.alive());
        state.cleanup_stale_surfaces();

        if let Some(ref mut session) = state.active_screen_recording {
            let rec_now = Instant::now();
            if session.pending_mapping_ready(rec_now) {
                session.flush_pending_mapping(&mut renderer);
            }
            if rec_now.duration_since(session.last_status_broadcast) >= Duration::from_secs(1) {
                session.last_status_broadcast = rec_now;
                let elapsed = session.start_time.elapsed().as_secs();
                state
                    .ipc_broadcaster
                    .broadcast(&malus_de_common::ipc::IpcMessage::ScreenRecordingStatus {
                        is_recording: true,
                        elapsed_secs: elapsed,
                    });
            }
        }

        // 🎨 RENDERIZADO GPU REAL A CADA MONITOR FÍSICO
        let mut visible_surface_ids = std::collections::HashSet::new();
        let mut rendered_outputs = std::collections::HashSet::new();
        if let Ok(mut surfs) = shared_surfaces.lock() {
            let theme_capture_request = state.pending_theme_transition_capture.clone();
            let mut theme_capture_succeeded = theme_capture_request.is_some();
            let mut theme_capture_outputs = 0usize;
            let expected_theme_outputs = surfs.len();
            let workspace_switch_pending = state.pending_workspace_switch.is_some();
            let mut ws_capture_outputs = 0usize;
            let mut ws_transition_finished = false;
            let mut ws_transition_attempted = false;
            let mut ws_transition_drawn = false;
            for out_surface in surfs.iter_mut() {
                out_surface.recover_expired_flip();
                let scale = out_surface.output.current_scale().fractional_scale();
                let output_geo = state
                    .space
                    .output_geometry(&out_surface.output)
                    .unwrap_or_default();
                let out_name = out_surface.output.name();

                // Fast-path en reposo / flip pendiente: evitar construir todos los elementos de
                // escena (`render_elements_from_surface_tree`, SSD, oclusión, overlays) cuando
                // este monitor no necesita renderizar en este ciclo.
                let has_pending_captures = !state.pending_black_holes.is_empty()
                    || !state.active_animations.is_empty()
                    || !state.pending_window_screenshot_requests.is_empty()
                    || theme_capture_request.is_some()
                    || workspace_switch_pending;
                if out_surface.is_flip_pending() && !has_pending_captures {
                    continue;
                }
                if !has_pending_captures {
                    let quick_cursor_point = state.seat.get_pointer().and_then(|pointer| {
                        let loc = pointer.current_location();
                        if output_geo.to_f64().contains(loc)
                            && !matches!(&state.cursor_status, CursorImageStatus::Hidden)
                        {
                            let px = ((loc.x - output_geo.loc.x as f64) * scale).round() as i32;
                            let py = ((loc.y - output_geo.loc.y as f64) * scale).round() as i32;
                            Some((px, py))
                        } else {
                            None
                        }
                    });
                    let quick_has_commits = state.surface_commit_counter
                        != out_surface.last_commit_counter
                        || state.output_commit_counter(&out_name)
                            != out_surface.last_output_commit_counter;
                    let quick_cursor_changed = quick_cursor_point
                        != out_surface.last_cursor_draw_point
                        || state.cursor_status != out_surface.last_cursor_status
                        || cursor_scale_milli != out_surface.last_cursor_scale_milli;
                    let quick_has_effects = out_surface.pending_damage_buffers > 0
                        || out_surface.full_damage_buffers > 0
                        || out_surface.dock_redraw_buffers > 0
                        || quick_has_commits
                        || quick_cursor_changed
                        || shake_cursor_animating
                        || state.dnd_drop_hold.is_some()
                        || state.active_dnd_icon_draw_location().is_some()
                        || expose_animating
                        || expose_just_closed
                        || out_surface.last_expose_revision != state.expose.render_revision
                        || out_surface.last_workspace_revision != state.workspace_revision
                        || active_black_holes.iter().any(|bh| bh.output_name == out_name)
                        || !state.snap_transitions.is_empty()
                        || state.snap_preview.is_some()
                        || state.theme_transition_started_at.is_some()
                        || state.workspace_transition.is_some()
                        || !state.pending_screenshot_requests.is_empty()
                        || !state.pending_window_screenshot_requests.is_empty()
                        || state
                            .active_screen_recording
                            .as_ref()
                            .is_some_and(|s| s.matches_output(&out_name) && !s.first_frame_captured)
                        || state.spotlight.should_render_on_output(&out_name)
                        || state.spotlight.has_active_drag_or_poof()
                        || state.screenshot_ui.should_render_on_output(&out_name)
                        || state.snap_layouts.should_render_on_output(&out_name)
                        || state.media_keys.should_render_on_output(&out_name)
                        || state.deck_should_render_on_output(&out_name);

                    if !quick_has_effects {
                        // Verificar rápidamente si cambiaron popups o geometrías/foco de ventanas en este monitor
                        let kb_focus = state.seat.get_keyboard().and_then(|k| k.current_focus());
                        let mut quick_win_count = 0usize;
                        let mut quick_win_changed = false;
                        for window in state.space.elements() {
                            let Some(loc) = state.space.element_location(window) else {
                                continue;
                            };
                            let Some(main_surface) = window.wl_surface() else {
                                continue;
                            };
                            let geo = window.geometry();
                            let win_rect = Rectangle::new(loc, geo.size);
                            if !output_geo.overlaps(win_rect) {
                                continue;
                            }
                            quick_win_count += 1;
                            let root_id =
                                smithay::backend::renderer::element::Id::from_wayland_resource(
                                    main_surface.as_ref(),
                                );
                            let is_active =
                                state.is_window_visually_active(window, kb_focus.as_ref());
                            if out_surface.last_window_logical_geoms.get(&root_id)
                                != Some(&(win_rect, is_active))
                            {
                                quick_win_changed = true;
                                break;
                            }
                        }
                        if !quick_win_changed
                            && quick_win_count == out_surface.last_window_logical_geoms.len()
                        {
                            let quick_layer_map =
                                smithay::desktop::layer_map_for_output(&out_surface.output);
                            let mut quick_popups = state.active_popups.len();
                            for surface in quick_layer_map.layers() {
                                if matches!(
                                    surface.namespace(),
                                    "malus-top-bar"
                                        | "malus-dock"
                                        | "malus-wallpaper"
                                        | "malus-desktop-icons"
                                ) {
                                    quick_popups +=
                                        smithay::desktop::PopupManager::popups_for_surface(
                                            surface.wl_surface(),
                                        )
                                        .count();
                                }
                            }
                            if quick_popups == out_surface.last_popup_count {
                                continue;
                            }
                        }
                    }
                }

                // Preparar animaciones Genie activas que intersecten este monitor
                let now = Instant::now();
                let active_genies: Vec<crate::state::GenieAnimation> = state
                    .active_animations
                    .iter()
                    .filter(|anim| {
                        if anim.waiting_for_dock_slot {
                            return false;
                        }
                        let win_rect = genie_snapshots
                            .get(&anim.window_id)
                            .map_or(anim.window_rect, |snap| snap.win_rect_logical);
                        let anim_bbox = win_rect.merge(anim.dock_slot_rect);
                        output_geo.overlaps(anim_bbox)
                    })
                    .cloned()
                    .collect();

                // Preparar elementos de cursor antes de comenzar el pase de renderizado
                let mut cursor_elems = Vec::new();
                let mut cursor_draw_point = None;
                let mut cursor_hotspot = Point::from((0, 0));

                if let Some(pointer) = state.seat.get_pointer() {
                    let loc = pointer.current_location();
                    if output_geo.to_f64().contains(loc) {
                        let phys_x = ((loc.x - output_geo.loc.x as f64) * scale).round() as i32;
                        let phys_y = ((loc.y - output_geo.loc.y as f64) * scale).round() as i32;
                        if !matches!(&state.cursor_status, CursorImageStatus::Hidden) {
                            cursor_draw_point = Some((phys_x, phys_y));
                        }

                        if let CursorImageStatus::Surface(ref surf) = state.cursor_status {
                            if !surf.alive() {
                                state.cursor_status = CursorImageStatus::default_named();
                            }
                        }

                        if let CursorImageStatus::Surface(ref surf) = state.cursor_status {
                            cursor_hotspot =
                                smithay::wayland::compositor::with_states(surf, |states| {
                                    states
                                        .data_map
                                        .get::<smithay::input::pointer::CursorImageSurfaceData>()
                                        .map(|attrs| attrs.lock().unwrap().hotspot)
                                        .unwrap_or_default()
                                });

                            cursor_elems = render_elements_from_surface_tree::<
                                GlesRenderer,
                                WaylandSurfaceRenderElement<GlesRenderer>,
                            >(
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

                let mut dnd_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();
                let mut dnd_fallback_elem: Option<MemoryRenderBufferRenderElement<GlesRenderer>> =
                    None;
                if let Some(loc) = state.active_dnd_icon_draw_location() {
                    if output_geo.to_f64().contains(loc) {
                        if let Some(dnd_surf) = state.dnd_icon.clone() {
                            if dnd_surf.alive() {
                                if state.dnd_icon_cache_dirty {
                                    super::capture_dnd_icon_cache(
                                        &mut renderer,
                                        &mut state,
                                        &dnd_surf,
                                        scale,
                                    );
                                }
                                let offset = state.dnd_icon_offset;
                                let draw_x = loc.x + offset.x as f64 - output_geo.loc.x as f64;
                                let draw_y = loc.y + offset.y as f64 - output_geo.loc.y as f64;
                                let phys_x = (draw_x * scale).round() as i32;
                                let phys_y = (draw_y * scale).round() as i32;
                                dnd_elems = render_elements_from_surface_tree::<
                                    GlesRenderer,
                                    WaylandSurfaceRenderElement<GlesRenderer>,
                                >(
                                    &mut renderer,
                                    &dnd_surf,
                                    Point::from((phys_x, phys_y)),
                                    smithay::utils::Scale::from(scale),
                                    1.0,
                                    Kind::Unspecified,
                                );
                            }
                        }
                        if dnd_elems.is_empty() && state.dnd_drop_hold.is_some() {
                            let draw_x = loc.x - output_geo.loc.x as f64;
                            let draw_y = loc.y - output_geo.loc.y as f64;
                            dnd_fallback_elem = super::build_cached_dnd_fallback(
                                &mut renderer,
                                &state,
                                draw_x,
                                draw_y,
                                scale,
                            );
                        }
                    }
                }

                let corner_radius_pt = current_displays_cfg.screen_corner_radius;
                let phys_r = if corner_radius_pt > 0.0 {
                    let r = (corner_radius_pt * scale).round() as i32;
                    if r > 2 {
                        if !squircle_corner_cache.contains_key(&r) {
                            if let Some(textures) =
                                create_squircle_corner_textures(&mut renderer, r)
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
                    let r = (win_corner_radius_pt * scale).round() as i32;
                    if r > 2 {
                        Some(r)
                    } else {
                        None
                    }
                } else {
                    None
                };

                let session_locked = state.is_session_locked();
                let layer_map = smithay::desktop::layer_map_for_output(&out_surface.output);
                let has_fullscreen_window =
                    state.has_fullscreen_window_on_output(&out_surface.output);
                let mut lower_elements = Vec::new();
                let mut upper_elements = Vec::new();
                let mut lock_elements = Vec::new();
                let mut dock_clip_roots = Vec::new();
                let mut spotlight_backdrop_elements = Vec::new();
                let mut spotlight_shadow_rects = Vec::new();
                let mut dock_popups = Vec::new();
                let mut topbar_popups = Vec::new();
                let mut wallpaper_popups = Vec::new();
                let spotlight_active = layer_map
                    .layers()
                    .any(|surface| surface.namespace() == "malus-spotlight-results");
                for surface in layer_map.layers().rev() {
                    if surface.namespace() == "malus-app-switcher"
                        && !state.app_switcher_classic_active()
                    {
                        continue;
                    }
                    if let Some(geo) = layer_map.layer_geometry(surface) {
                        let namespace = surface.namespace();
                        if spotlight_active && namespace == "malus-spotlight-results" {
                            spotlight_shadow_rects.push(geo.to_physical_precise_round(scale));
                        }
                        let layer_loc = geo.loc.to_physical_precise_round(scale);
                        let s_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                            render_elements_from_surface_tree(
                                &mut renderer,
                                surface.wl_surface(),
                                layer_loc,
                                smithay::utils::Scale::from(scale),
                                1.0,
                                Kind::Unspecified,
                            );
                        if namespace == "malus-notifications"
                            && state.desktop_dock_cfg.black_hole_close_effect
                        {
                            let root_id =
                                smithay::backend::renderer::element::Id::from_wayland_resource(
                                    surface.wl_surface(),
                                );
                            crate::black_hole::update_cached_window_snapshot(
                                &mut state.cached_close_snapshots,
                                surface.wl_surface(),
                                &s_elems,
                                &root_id,
                                layer_loc,
                                scale,
                            );
                        }
                        let is_collapsed_dock = namespace == "malus-dock"
                            && super::dock_is_collapsed(
                                geo.to_physical_precise_round(scale),
                                scale,
                                state.dock_cfg.position,
                            );
                        if surface.namespace() == "malus-dock"
                            && !has_fullscreen_window
                            && !is_collapsed_dock
                        {
                            let root_id =
                                smithay::backend::renderer::element::Id::from_wayland_resource(
                                    surface.wl_surface(),
                                );
                            if let Some(root_elem) =
                                s_elems.iter().find(|elem| *elem.id() == root_id)
                            {
                                let root_geo =
                                    root_elem.geometry(smithay::utils::Scale::from(scale));
                                dock_clip_roots.push((
                                    root_id,
                                    super::dock_clip_rect(
                                        root_geo,
                                        state.dock_blur_headroom,
                                        scale,
                                        state.dock_cfg.position,
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
                            if surface.namespace() == "malus-top-bar" {
                                if !has_fullscreen_window {
                                    let popup_rect = Rectangle::new(
                                        layer_loc + popup_offset.to_physical_precise_round(scale),
                                        popup_geometry.size.to_physical_precise_round(scale),
                                    );
                                    topbar_popups.push((elems, popup_rect));
                                }
                            } else if surface.namespace() == "malus-dock" {
                                if !has_fullscreen_window {
                                    let popup_rect = Rectangle::new(
                                        layer_loc + offset,
                                        popup_geometry.size.to_physical_precise_round(scale),
                                    );
                                    dock_popups.push((elems, popup_rect));
                                }
                            } else if matches!(
                                surface.namespace(),
                                "malus-wallpaper" | "malus-desktop-icons"
                            ) {
                                let popup_rect = Rectangle::new(
                                    layer_loc + popup_offset.to_physical_precise_round(scale),
                                    popup_geometry.size.to_physical_precise_round(scale),
                                );
                                wallpaper_popups.push((elems, popup_rect));
                            } else {
                                p_elems.extend(elems);
                            }
                        }

                        if namespace == "malus-spotlight-backdrop" {
                            spotlight_backdrop_elements
                                .push((s_elems, geo.to_physical_precise_round(scale)));
                        } else if namespace == "malus-lock" {
                            lock_elements.extend(s_elems);
                            lock_elements.extend(p_elems);
                        } else if matches!(surface.layer(), Layer::Background | Layer::Bottom) {
                            lower_elements.extend(s_elems);
                            upper_elements.extend(p_elems);
                        } else if surface.layer() == Layer::Top {
                            // Por protocolo wlr-layer-shell, Layer::Top está por debajo de ventanas a pantalla completa.
                            // Solo se dibuja en capas superiores cuando NO hay una ventana a pantalla completa en este monitor.
                            if !has_fullscreen_window && !is_collapsed_dock {
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
                    state.authorized_ext_lock_surface_for_output(&out_surface.output.name())
                {
                    let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                        render_elements_from_surface_tree(
                            &mut renderer,
                            ext_lock_surf.wl_surface(),
                            (0, 0),
                            smithay::utils::Scale::from(scale),
                            1.0,
                            Kind::Unspecified,
                        );
                    lock_elements.extend(elems);
                }

                let is_dark_theme = crate::state::is_dark_color_scheme();
                let mut window_render_data = Vec::new();
                for window in state.space.elements() {
                    if let Some(loc) = state.space.element_location(window) {
                        let geo = window.geometry();
                        let Some(main_surface) = window.wl_surface() else {
                            continue;
                        };

                        let trans_opt = state.snap_transitions.get(main_surface.as_ref()).cloned();
                        let transition_layout = trans_opt
                            .as_ref()
                            .map(|trans| trans.render_geometry(geo, output_geo.loc, scale, now));
                        let (effective_loc, effective_size, is_transitioning) =
                            if let Some((eloc, esize, ..)) = transition_layout {
                                (eloc, esize, true)
                            } else {
                                (loc, geo.size, false)
                            };

                        let win_flags = crate::state::WindowRenderFlags::from_window(
                            window,
                            main_surface.as_ref(),
                        );
                        let uses_ssd = win_flags.uses_ssd;
                        let ssd_top = if uses_ssd {
                            crate::ssd::SSD_TITLEBAR_HEIGHT
                        } else {
                            0
                        };
                        let win_global_rect = Rectangle::new(
                            Point::from((effective_loc.x, effective_loc.y - ssd_top)),
                            Size::from((effective_size.w, effective_size.h + ssd_top)),
                        );
                        if output_geo.overlaps(win_global_rect) {
                            let (
                                phys_loc,
                                transition_clip,
                                content_offset,
                                content_size,
                                win_screen_rect,
                            ) = if let Some((_, _, pl, clip_rect, c_off, c_sz, w_rect)) =
                                transition_layout
                            {
                                (pl, clip_rect, c_off, c_sz, w_rect)
                            } else {
                                let pl = (loc - geo.loc - output_geo.loc)
                                    .to_physical_precise_round(scale);
                                let c_off = geo.loc.to_physical_precise_round(scale);
                                let c_sz = geo.size.to_physical_precise_round(scale);
                                let w_rect = Rectangle::new(
                                    (loc - output_geo.loc).to_physical_precise_round(scale),
                                    geo.size.to_physical_precise_round(scale),
                                );
                                (pl, None, c_off, c_sz, w_rect)
                            };

                            let win_surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                                render_elements_from_surface_tree(
                                    &mut renderer,
                                    main_surface.as_ref(),
                                    phys_loc,
                                    smithay::utils::Scale::from(scale),
                                    1.0,
                                    Kind::Unspecified,
                                );
                            if !is_transitioning && state.desktop_dock_cfg.black_hole_close_effect {
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
                                    scale,
                                );
                            }

                            let mut popup_elems: Vec<(
                                Vec<WaylandSurfaceRenderElement<GlesRenderer>>,
                                Rectangle<i32, Physical>,
                                bool,
                            )> = Vec::new();
                            let mut popup_shadow_rects = Vec::new();
                            let popup_effects_enabled = win_flags.popup_effects_enabled;
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
                                        + (geo.loc + popup_offset).to_physical_precise_round(scale),
                                    popup_geometry.size.to_physical_precise_round(scale),
                                );
                                if popup_effects_enabled {
                                    popup_shadow_rects.push(popup_rect);
                                }
                                let offset = (geo.loc + popup_offset - popup_geometry.loc)
                                    .to_physical_precise_round(scale);
                                let p_elems = render_elements_from_surface_tree(
                                    &mut renderer,
                                    popup.wl_surface(),
                                    phys_loc + offset,
                                    smithay::utils::Scale::from(scale),
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

                            let kb_focus_now = state.seat.get_keyboard().and_then(|k| k.current_focus());
                            let is_active =
                                state.is_window_visually_active(window, kb_focus_now.as_ref());

                            let is_firefox = win_flags.is_firefox;
                            let is_translucent = win_flags.is_translucent;
                            let is_branches = win_flags.is_branches;
                            let is_full_translucent = win_flags.is_full_translucent;
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
                                    scale,
                                    is_translucent,
                                    uses_ssd,
                                )
                            } else {
                                win_screen_rect
                            };

                            let (win_screen_rect, ssd_elem) = if uses_ssd {
                                let tb_phys_h =
                                    (crate::ssd::SSD_TITLEBAR_HEIGHT as f64 * scale).round() as i32;
                                let tb_loc = Point::from((
                                    win_screen_rect.loc.x,
                                    win_screen_rect.loc.y - tb_phys_h,
                                ));
                                let full_rect = Rectangle::new(
                                    tb_loc,
                                    Size::from((
                                        win_screen_rect.size.w,
                                        win_screen_rect.size.h + tb_phys_h,
                                    )),
                                );
                                let ssd_title = crate::state::window_title_for_ssd(window);
                                let elem = state.ssd.render_element_for_window(
                                    &mut renderer,
                                    main_surface.as_ref(),
                                    tb_loc,
                                    win_screen_rect.size.w,
                                    tb_phys_h,
                                    scale,
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
                                is_full_translucent,
                            ));
                        }
                    }
                }

                // Inicializar shaders y pipelines declarativos de renderizado una sola vez por contexto GL
                if state.renderer_context_id.is_none() {
                    state.renderer_context_id = Some(renderer.context_id());
                }
                if !window_squircle_init_attempted {
                    window_squircle_init_attempted = true;
                    window_squircle_shader =
                        crate::window_corners::WindowSquircleShader::new(&mut renderer)
                            .map(std::sync::Arc::new);
                }
                if !shadow_init_attempted {
                    shadow_init_attempted = true;
                    shadow_pipeline = renderer
                        .with_context(|gl| crate::shadow::WindowShadowPipeline::new(gl))
                        .ok()
                        .flatten()
                        .map(std::sync::Arc::new);
                }
                if !blur_init_attempted {
                    blur_init_attempted = true;
                    blur_pipeline = renderer
                        .with_context(|gl| crate::blur::GaussianBlur::new(gl))
                        .ok()
                        .flatten()
                        .map(|b| std::sync::Arc::new(std::sync::Mutex::new(b)));
                }
                if !black_hole_init_attempted {
                    black_hole_init_attempted = true;
                    black_hole_pipeline = renderer
                        .with_context(|gl| crate::black_hole::BlackHolePipeline::new(gl))
                        .ok()
                        .flatten();
                }
                if !genie_init_attempted {
                    genie_init_attempted = true;
                    genie_pipeline = renderer
                        .with_context(|gl| crate::magic_lamp::GeniePipeline::new(gl))
                        .ok()
                        .flatten();
                }
                if !state.pending_black_holes.is_empty()
                    || !state.active_animations.is_empty()
                    || !genie_snapshots.is_empty()
                {
                    let mut blur_guard = blur_pipeline.as_ref().and_then(|b| b.lock().ok());
                    if !state.pending_black_holes.is_empty() {
                        let out_name = out_surface.output.name();
                        for pending in &state.pending_black_holes {
                            if let Some(active) = crate::black_hole::capture_closing_window_texture(
                                &mut renderer,
                                &mut state.ssd,
                                window_squircle_shader.as_deref(),
                                blur_guard.as_deref_mut(),
                                pending,
                                &out_name,
                                output_geo,
                                out_surface.size,
                                scale,
                                win_phys_r,
                                is_dark_theme,
                            ) {
                                active_black_holes.push(active);
                            }
                        }
                    }
                    crate::capture::capture_pending_genie_snapshots(
                        &mut state,
                        &mut renderer,
                        window_squircle_shader.as_deref(),
                        blur_guard.as_deref_mut(),
                        &mut genie_snapshots,
                        output_geo,
                        out_surface.size,
                        true,
                        scale,
                        win_phys_r,
                        is_dark_theme,
                        now,
                    );
                }

                // Captura aislada de ventana individual en PNG RGBA transparente (sin fondo ni ventanas traseras)
                while let Some(req_idx) =
                    state
                        .pending_window_screenshot_requests
                        .iter()
                        .position(|(_, _, out_req)| {
                            out_req
                                .as_deref()
                                .map_or(true, |name| name == out_surface.output.name())
                        })
                {
                    if let Some((destination, window_id, _)) =
                        state.pending_window_screenshot_requests.remove(req_idx)
                    {
                        if let Some(window) = state.window_by_id(window_id) {
                            let win_loc =
                                state.space.element_location(&window).unwrap_or_default();
                            if let Err(error) = crate::capture::capture_isolated_window_to_png(
                                &mut renderer,
                                &mut state.ssd,
                                window_squircle_shader.as_deref(),
                                &window,
                                win_loc,
                                output_geo.loc,
                                out_surface.size,
                                scale,
                                win_phys_r,
                                is_dark_theme,
                                &destination,
                            ) {
                                warn!(
                                    "Error al capturar ventana aislada {window_id} en DRM: {error:#}"
                                );
                                crate::capture::write_capture_error(&destination, &error);
                            }
                        } else {
                            let error = anyhow::anyhow!(
                                "No se encontró la ventana {window_id} para captura aislada"
                            );
                            crate::capture::write_capture_error(&destination, &error);
                        }
                    }
                }

                let spotlight_frame = if state
                    .spotlight
                    .should_render_on_output(&out_surface.output.name())
                {
                    if let Some((spot_rect, mask_opt, mask_sz, spot_alpha)) = state
                        .spotlight
                        .prepare_frame(output_geo, scale, is_dark_theme)
                    {
                        let mask_vec = mask_opt.map(|s| s.to_vec());
                        state
                            .spotlight
                            .create_render_element(
                                &mut renderer,
                                spot_rect.loc.to_f64(),
                                spot_alpha,
                            )
                            .map(|elem| (spot_rect, mask_vec, mask_sz, spot_alpha, elem))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let spotlight_drag_elem = if state.spotlight.has_active_drag_or_poof() {
                    state.spotlight.prepare_drag_render_element(
                        &mut renderer,
                        output_geo,
                        scale,
                        is_dark_theme,
                    )
                } else {
                    None
                };

                let screenshot_ui_frame = if state
                    .screenshot_ui
                    .should_render_on_output(&out_surface.output.name())
                {
                    if let Some((ui_rect, mask_opt, mask_sz, ui_alpha)) = state
                        .screenshot_ui
                        .prepare_frame(output_geo, scale, is_dark_theme)
                    {
                        let mask_vec = mask_opt.map(|s| s.to_vec());
                        state
                            .screenshot_ui
                            .create_render_element(&mut renderer, ui_rect.loc.to_f64())
                            .map(|elem| (ui_rect, mask_vec, mask_sz, ui_alpha, elem))
                    } else {
                        None
                    }
                } else {
                    None
                };

                let snap_layouts_frame = if state
                    .snap_layouts
                    .should_render_on_output(&out_surface.output.name())
                {
                    if let Some((card_rect, mask_opt, mask_sz, snap_alpha)) = state
                        .snap_layouts
                        .prepare_frame(&out_surface.output.name(), output_geo, scale, is_dark_theme)
                    {
                        let mask_vec = mask_opt.map(|s| s.to_vec());
                        state
                            .snap_layouts
                            .create_render_element(&mut renderer, card_rect.loc, snap_alpha)
                            .map(|elem| (card_rect, mask_vec, mask_sz, snap_alpha, elem))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let snap_layouts_preview = state
                    .snap_layouts
                    .hovered_preview_rect(&out_surface.output.name())
                    .filter(|preview| output_geo.overlaps(*preview));
                let media_osd_frame = if state
                    .media_keys
                    .should_render_on_output(&out_surface.output.name())
                {
                    let usable_zone =
                        state.usable_zone_from_layer_map(&out_surface.output, &layer_map);
                    if let Some((osd_rect, mask_opt, mask_sz, osd_alpha)) = state
                        .media_keys
                        .prepare_frame(
                            &out_surface.output.name(),
                            output_geo,
                            usable_zone,
                            scale,
                            is_dark_theme,
                            now,
                        )
                    {
                        let mask_vec = mask_opt.map(|s| s.to_vec());
                        state
                            .media_keys
                            .create_render_element(&mut renderer, osd_rect.loc, osd_alpha)
                            .map(|elem| (osd_rect, mask_vec, mask_sz, osd_alpha, elem))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let app_switcher_deck_frame =
                    if state.deck_should_render_on_output(&out_surface.output.name()) {
                        let deck_windows = state.deck_windows_for_render();
                        let app_switcher = &mut state.app_switcher;
                        let ssd = &mut state.ssd;
                        crate::app_switcher::prepare_deck_cards(
                            &mut renderer,
                            &mut app_switcher.deck,
                            ssd,
                            &app_switcher.items,
                            &deck_windows,
                            output_geo,
                            scale,
                            is_dark_theme,
                            now,
                        )
                    } else {
                        None
                    };

                if let CursorImageStatus::Named(icon) = state.cursor_status {
                    if icon != smithay::input::pointer::CursorIcon::Default
                        && !named_cursor_cache.contains_key(&icon)
                    {
                        let loaded = load_cursor_texture_by_names(
                            &mut renderer,
                            &cursor_theme,
                            cursor_size,
                            cursor_names_for_icon(icon),
                        );
                        named_cursor_cache.insert(icon, loaded);
                    }
                }

                let damage_buffers_per_event = if ENABLE_DAMAGE_TRACKING { 3 } else { 1 };
                let expose_rev_changed =
                    out_surface.last_expose_revision != state.expose.render_revision;
                let workspace_rev_changed =
                    out_surface.last_workspace_revision != state.workspace_revision;
                if expose_just_closed || expose_rev_changed || workspace_rev_changed {
                    out_surface.full_damage_buffers = out_surface
                        .full_damage_buffers
                        .max(damage_buffers_per_event);
                    out_surface.pending_damage_buffers = out_surface
                        .pending_damage_buffers
                        .max(damage_buffers_per_event);
                }

                // ─── CÁLCULO DE DAMAGE TRACKING POR PANTALLA CON SMITHAY ───
                let tracker_elements: Vec<MalusDrmElement> = if ENABLE_DAMAGE_TRACKING {
                    let mut tracker_elements: Vec<MalusDrmElement> = Vec::new();

                    for elem in &cursor_elems {
                        tracker_elements.push(MalusDrmElement::Wayland(elem));
                    }
                    for elem in &dnd_elems {
                        tracker_elements.push(MalusDrmElement::Wayland(elem));
                    }
                    if let Some(ref fallback_elem) = dnd_fallback_elem {
                        tracker_elements.push(MalusDrmElement::Memory(fallback_elem));
                    }
                    let popup_corner_radius =
                        (super::POPUP_MENU_CORNER_RADIUS_LOGICAL * scale) as f32;
                    let mut live_popup_keys = std::collections::HashSet::new();
                    for (popups, popup_rect) in &topbar_popups {
                        let topbar_popup_r =
                            super::topbar_popup_corner_radius(*popup_rect, out_surface.size, scale);
                        let popup_key = popups
                            .first()
                            .map(|elem| elem.id().clone())
                            .unwrap_or_else(smithay::backend::renderer::element::Id::new);
                        live_popup_keys.insert(popup_key.clone());
                        let (blur_id, shadow_id, effect_commit) = popup_effect_ids_for(
                            &mut out_surface.popup_effect_ids,
                            popup_key,
                            *popup_rect,
                        );
                        for elem in popups {
                            tracker_elements.push(MalusDrmElement::Wayland(elem));
                        }
                        if let Some(blur) = blur_pipeline.as_ref() {
                            let menu_blur = crate::blur::BlurRenderElement::for_popup(
                                blur_id,
                                effect_commit,
                                blur.clone(),
                                *popup_rect,
                                topbar_popup_r.round(),
                                out_surface.size,
                            );
                            tracker_elements.push(MalusDrmElement::Blur(menu_blur));
                        }
                        if let Some(pipeline) = shadow_pipeline.as_ref() {
                            let menu_shadow = crate::shadow::MenuShadowRenderElement::new(
                                shadow_id,
                                effect_commit,
                                pipeline.clone(),
                                *popup_rect,
                                topbar_popup_r,
                                scale,
                            );
                            tracker_elements.push(MalusDrmElement::MenuShadow(menu_shadow));
                        }
                    }
                    for (popups, popup_rect) in &dock_popups {
                        let blur_rect = super::dock_popover_blur_rect(*popup_rect, scale);
                        let dock_popup_radius =
                            super::dock_popover_corner_radius(blur_rect, scale);
                        let popup_key = popups
                            .first()
                            .map(|elem| elem.id().clone())
                            .unwrap_or_else(smithay::backend::renderer::element::Id::new);
                        live_popup_keys.insert(popup_key.clone());
                        let (blur_id, shadow_id, effect_commit) = popup_effect_ids_for(
                            &mut out_surface.popup_effect_ids,
                            popup_key,
                            *popup_rect,
                        );
                        for elem in popups {
                            tracker_elements.push(MalusDrmElement::Wayland(elem));
                        }
                        if let Some(blur) = blur_pipeline.as_ref() {
                            let menu_blur = crate::blur::BlurRenderElement::for_popup(
                                blur_id,
                                effect_commit,
                                blur.clone(),
                                blur_rect,
                                dock_popup_radius.round(),
                                out_surface.size,
                            );
                            tracker_elements.push(MalusDrmElement::Blur(menu_blur));
                        }
                        if let Some(pipeline) = shadow_pipeline.as_ref() {
                            let menu_shadow = crate::shadow::MenuShadowRenderElement::new(
                                shadow_id,
                                effect_commit,
                                pipeline.clone(),
                                *popup_rect,
                                dock_popup_radius,
                                scale,
                            );
                            tracker_elements.push(MalusDrmElement::MenuShadow(menu_shadow));
                        }
                    }
                    for (popups, popup_rect) in &wallpaper_popups {
                        let popup_key = popups
                            .first()
                            .map(|elem| elem.id().clone())
                            .unwrap_or_else(smithay::backend::renderer::element::Id::new);
                        live_popup_keys.insert(popup_key.clone());
                        let (blur_id, shadow_id, effect_commit) = popup_effect_ids_for(
                            &mut out_surface.popup_effect_ids,
                            popup_key,
                            *popup_rect,
                        );
                        for elem in popups {
                            tracker_elements.push(MalusDrmElement::Wayland(elem));
                        }
                        if let Some(blur) = blur_pipeline.as_ref() {
                            let menu_blur = crate::blur::BlurRenderElement::for_popup(
                                blur_id,
                                effect_commit,
                                blur.clone(),
                                *popup_rect,
                                popup_corner_radius.round(),
                                out_surface.size,
                            );
                            tracker_elements.push(MalusDrmElement::Blur(menu_blur));
                        }
                        if let Some(pipeline) = shadow_pipeline.as_ref() {
                            let menu_shadow = crate::shadow::MenuShadowRenderElement::new(
                                shadow_id,
                                effect_commit,
                                pipeline.clone(),
                                *popup_rect,
                                popup_corner_radius,
                                scale,
                            );
                            tracker_elements.push(MalusDrmElement::MenuShadow(menu_shadow));
                        }
                    }
                    out_surface
                        .popup_effect_ids
                        .retain(|k, _| live_popup_keys.contains(k));
                    for elem in &upper_elements {
                        let dock_rect = dock_clip_roots
                            .iter()
                            .find(|(root_id, _)| *elem.id() == *root_id)
                            .map(|(_, rect)| *rect);
                        if let Some(rect) = dock_rect {
                            let squircle_dock =
                                crate::window_corners::SquircleSurfaceRenderElement::with_dock_clip(
                                    elem,
                                    window_squircle_shader.clone(),
                                    elem.geometry(smithay::utils::Scale::from(scale)).size,
                                    smithay::utils::Point::from((0, 0)),
                                    rect.size,
                                    (32.0 * scale).round() as i32,
                                );
                            tracker_elements.push(MalusDrmElement::WaylandSquircle(squircle_dock));

                            if let Some(blur) = blur_pipeline.as_ref() {
                                let dock_blur = crate::blur::BlurRenderElement::for_dock(
                                    smithay::backend::renderer::element::Id::new(),
                                    smithay::backend::renderer::utils::CommitCounter::default(),
                                    blur.clone(),
                                    rect,
                                    32.0 * scale as f32,
                                    out_surface.size,
                                );
                                tracker_elements.push(MalusDrmElement::Blur(dock_blur));
                            }

                            if let Some(pipeline) = shadow_pipeline.as_ref() {
                                let dock_shadow = crate::shadow::DockShadowRenderElement::new(
                                    smithay::backend::renderer::element::Id::new(),
                                    smithay::backend::renderer::utils::CommitCounter::default(),
                                    pipeline.clone(),
                                    rect,
                                    32.0 * scale as f32,
                                    scale,
                                );
                                tracker_elements.push(MalusDrmElement::DockShadow(dock_shadow));
                            }
                        } else {
                            tracker_elements.push(MalusDrmElement::Wayland(elem));
                        }
                    }
                    if app_switcher_deck_frame.is_none() {
                        for (
                            root_id,
                            win_elems,
                            popups,
                            popup_shadow_rects,
                            should_round,
                            is_active,
                            win_screen_rect,
                            content_offset,
                            content_size,
                            is_firefox,
                            is_translucent,
                            _,
                            ssd_elem,
                            _,
                            is_branches,
                            is_full_translucent,
                        ) in window_render_data.iter().rev()
                        {
                            for (popup_elems, popup_rect, popup_effects) in popups {
                                for elem in popup_elems {
                                    tracker_elements.push(MalusDrmElement::Wayland(elem));
                                }
                                if *popup_effects {
                                    if let Some(blur) = blur_pipeline.as_ref() {
                                        let menu_blur = crate::blur::BlurRenderElement::for_popup(
                                    smithay::backend::renderer::element::Id::new(),
                                    smithay::backend::renderer::utils::CommitCounter::default(),
                                    blur.clone(),
                                    *popup_rect,
                                    popup_corner_radius.round(),
                                    out_surface.size,
                                );
                                        tracker_elements.push(MalusDrmElement::Blur(menu_blur));
                                    }
                                }
                            }
                            if let Some(pipeline) = shadow_pipeline.as_ref() {
                                for popup_rect in popup_shadow_rects {
                                    let menu_shadow = crate::shadow::MenuShadowRenderElement::new(
                                        smithay::backend::renderer::element::Id::new(),
                                        smithay::backend::renderer::utils::CommitCounter::default(),
                                        pipeline.clone(),
                                        *popup_rect,
                                        popup_corner_radius,
                                        scale,
                                    );
                                    tracker_elements.push(MalusDrmElement::MenuShadow(menu_shadow));
                                }
                            }

                            if let Some(ssd) = ssd_elem {
                                if *should_round && (*is_firefox || ssd_elem.is_some()) {
                                    let tb_geo = ssd.geometry(smithay::utils::Scale::from(1.0));
                                    let sub_offset = smithay::utils::Point::from((
                                        win_screen_rect.loc.x - tb_geo.loc.x,
                                        win_screen_rect.loc.y - tb_geo.loc.y,
                                    ));
                                    let ssd_squircle =
                                        crate::window_corners::SquircleMemoryRenderElement::new(
                                            ssd,
                                            window_squircle_shader.clone(),
                                            tb_geo.size,
                                            sub_offset,
                                            win_screen_rect.size,
                                            win_phys_r.unwrap_or(0),
                                        );
                                    tracker_elements
                                        .push(MalusDrmElement::MemorySquircle(ssd_squircle));
                                } else {
                                    tracker_elements.push(MalusDrmElement::Memory(ssd));
                                }
                            }

                            for elem in win_elems {
                                if *should_round {
                                    if *is_firefox || ssd_elem.is_some() {
                                        let elem_geo =
                                            elem.geometry(smithay::utils::Scale::from(scale));
                                        let sub_offset = smithay::utils::Point::from((
                                            win_screen_rect.loc.x - elem_geo.loc.x,
                                            win_screen_rect.loc.y - elem_geo.loc.y,
                                        ));
                                        let win_squircle = crate::window_corners::SquircleSurfaceRenderElement::new(
                                    elem,
                                    window_squircle_shader.clone(),
                                    elem_geo.size,
                                    sub_offset,
                                    win_screen_rect.size,
                                    win_phys_r.unwrap_or(0),
                                );
                                        tracker_elements
                                            .push(MalusDrmElement::WaylandSquircle(win_squircle));
                                    } else if elem.id() == root_id {
                                        let elem_geo =
                                            elem.geometry(smithay::utils::Scale::from(scale));
                                        let win_squircle = crate::window_corners::SquircleSurfaceRenderElement::new(
                                    elem,
                                    window_squircle_shader.clone(),
                                    elem_geo.size,
                                    *content_offset,
                                    *content_size,
                                    win_phys_r.unwrap_or(0),
                                );
                                        tracker_elements
                                            .push(MalusDrmElement::WaylandSquircle(win_squircle));
                                    } else {
                                        tracker_elements.push(MalusDrmElement::Wayland(elem));
                                    }
                                } else {
                                    tracker_elements.push(MalusDrmElement::Wayland(elem));
                                }
                            }

                            if *is_translucent {
                                let blur_rect = super::translucent_window_blur_rect(
                                    *win_screen_rect,
                                    *is_branches,
                                    *is_translucent,
                                    *is_full_translucent,
                                    win_phys_r,
                                    scale,
                                );
                                let corner_radius = if *should_round {
                                    win_phys_r.unwrap_or(0) as f32
                                } else {
                                    0.0
                                };
                                if let Some(blur) = blur_pipeline.as_ref() {
                                    let win_blur = crate::blur::BlurRenderElement::for_window(
                                        root_id.clone(),
                                        smithay::backend::renderer::utils::CommitCounter::default(),
                                        blur.clone(),
                                        blur_rect,
                                        corner_radius,
                                        out_surface.size,
                                    );
                                    tracker_elements.push(MalusDrmElement::Blur(win_blur));
                                }
                            }

                            // Sombra proyectada como RenderElement declarativo en el tracker de daño
                            if *should_round {
                                if let (Some(r), Some(pipeline)) =
                                    (win_phys_r, shadow_pipeline.as_ref())
                                {
                                    let shadow_elem = crate::shadow::WindowShadowRenderElement::new(
                                        root_id.clone(),
                                        smithay::backend::renderer::utils::CommitCounter::default(),
                                        pipeline.clone(),
                                        *win_screen_rect,
                                        r as f32,
                                        *is_active,
                                        scale,
                                    );
                                    tracker_elements.push(MalusDrmElement::Shadow(shadow_elem));
                                }
                            }
                        }
                    }
                    for elem in &lower_elements {
                        tracker_elements.push(MalusDrmElement::Wayland(elem));
                    }
                    tracker_elements
                } else {
                    Vec::new()
                };

                // Precomputar oclusión (Occlusion Culling) de arriba hacia abajo para evitar overdraw masivo
                let screen_bounds = Rectangle::from_size(out_surface.size);
                let shadow_pad = (80.0 * scale).round() as i32;
                let mut body_occluded = vec![false; window_render_data.len()];
                let mut shadow_occluded = vec![false; window_render_data.len()];
                let mut occluders: Vec<super::OpaqueOccluder> = Vec::new();
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

                    // Las ventanas maximizadas no proyectan sombra exterior fuera del área útil
                    if is_maximized && !is_transitioning {
                        shadow_occluded[i] = true;
                    }

                    let raw_shadow_rect = Rectangle::new(
                        (win_rect.loc.x - shadow_pad, win_rect.loc.y - shadow_pad).into(),
                        (
                            win_rect.size.w + shadow_pad * 2,
                            win_rect.size.h + shadow_pad * 2,
                        )
                            .into(),
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
                        // En Branches solo la barra lateral izquierda (212px lógicos) es translúcida;
                        // el panel derecho de contenido es 100% opaco y ocluye ventanas detrás.
                        let sidebar_w = (212.0 * scale).round() as i32;
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

                // Verificación y sincronización por monitor (evita que commits de una pantalla redibujen la otra)
                let current_output_commit_counter =
                    state.output_commit_counter(&out_surface.output.name());
                let has_commits = state.surface_commit_counter != out_surface.last_commit_counter
                    || current_output_commit_counter != out_surface.last_output_commit_counter;
                if has_commits {
                    out_surface.pending_damage_buffers = out_surface
                        .pending_damage_buffers
                        .max(damage_buffers_per_event);
                }

                let cursor_changed = cursor_draw_point != out_surface.last_cursor_draw_point
                    || state.cursor_status != out_surface.last_cursor_status
                    || cursor_scale_milli != out_surface.last_cursor_scale_milli;
                if cursor_changed {
                    out_surface.pending_damage_buffers = out_surface.pending_damage_buffers.max(1);
                }
                if state.dnd_drop_hold.is_some() {
                    out_surface.pending_damage_buffers = out_surface
                        .pending_damage_buffers
                        .max(damage_buffers_per_event);
                }

                let mut window_geoms_changed =
                    window_render_data.len() != out_surface.last_window_geometries.len();
                if !window_geoms_changed {
                    for (root_id, _, _, _, _, is_active, win_rect, ..) in &window_render_data {
                        match out_surface.last_window_geometries.get(root_id) {
                            Some((prev_rect, prev_active)) => {
                                if *prev_rect != *win_rect || *prev_active != *is_active {
                                    window_geoms_changed = true;
                                    break;
                                }
                            }
                            None => {
                                window_geoms_changed = true;
                                break;
                            }
                        }
                    }
                }
                if window_geoms_changed {
                    out_surface.pending_damage_buffers = out_surface
                        .pending_damage_buffers
                        .max(damage_buffers_per_event);
                }

                let screenshot_active =
                    state
                        .pending_screenshot_requests
                        .iter()
                        .any(|(_, output_name)| {
                            output_name
                                .as_deref()
                                .map_or(true, |name| name == out_surface.output.name())
                        });

                let black_hole_active_on_output = active_black_holes
                    .iter()
                    .any(|bh| bh.output_name == out_surface.output.name());

                let current_popup_count = topbar_popups.len()
                    + dock_popups.len()
                    + wallpaper_popups.len()
                    + state.active_popups.len();
                let popup_count_changed = current_popup_count != out_surface.last_popup_count;
                if popup_count_changed {
                    out_surface.last_popup_count = current_popup_count;
                    out_surface.pending_damage_buffers = out_surface
                        .pending_damage_buffers
                        .max(damage_buffers_per_event);
                }

                let recording_due_on_output = state
                    .active_screen_recording
                    .as_ref()
                    .is_some_and(|s| s.matches_output(&out_name) && !s.first_frame_captured);

                let has_active_effects = !active_genies.is_empty()
                    || black_hole_active_on_output
                    || !state.snap_transitions.is_empty()
                    || state.snap_preview.is_some()
                    || state.theme_transition_started_at.is_some()
                    || theme_capture_request.is_some()
                    || state.workspace_transition.is_some()
                    || workspace_switch_pending
                    || screenshot_active
                    || recording_due_on_output
                    || spotlight_frame.is_some()
                    || screenshot_ui_frame.is_some()
                    || snap_layouts_frame.is_some()
                    || media_osd_frame.is_some()
                    || app_switcher_deck_frame.is_some()
                    || expose_animating
                    || expose_just_closed
                    || expose_rev_changed
                    || workspace_rev_changed
                    || popup_count_changed
                    || out_surface.dock_redraw_buffers > 0
                    || out_surface.full_damage_buffers > 0;

                let needs_render = has_active_effects || out_surface.pending_damage_buffers > 0;
                if !needs_render || out_surface.is_flip_pending() {
                    continue;
                }

                // 🌟 IDLE SLEEPING: Solo si hay cambios reales en esta pantalla física solicitamos un búfer a DRM
                let (mut dmabuf, age) = match out_surface.surface.next_buffer() {
                    Ok(buf) => buf,
                    Err(e) => {
                        if theme_capture_request.is_some() {
                            theme_capture_succeeded = false;
                        }
                        warn!(
                            "Buffer no disponible para {}: {:?}",
                            out_surface.output.name(),
                            e
                        );
                        continue;
                    }
                };

                // Registrar las ventanas y animaciones visibles en el monitor una vez confirmado el render
                for anim in &active_genies {
                    visible_surface_ids.insert(
                        smithay::backend::renderer::element::Id::from_wayland_resource(
                            &anim.surface,
                        ),
                    );
                }
                for (idx, (root_id, ..)) in window_render_data.iter().enumerate() {
                    if !body_occluded[idx] {
                        visible_surface_ids.insert(root_id.clone());
                    }
                }

                let wallpaper_occluded = !state.expose.active
                    && app_switcher_deck_frame.is_none()
                    && occluders.iter().any(|occ| occ.contains_rect(screen_bounds));

                let age_to_use = if out_surface.full_damage_buffers > 0 {
                    0
                } else {
                    age as usize
                };

                if age_to_use == 0 {
                    out_surface.cursor_history.clear();
                    out_surface.extra_damage_history.clear();
                    out_surface.dock_redraw_buffers = if ENABLE_DAMAGE_TRACKING { 3 } else { 0 };
                    for (root_id, ..) in &window_render_data {
                        out_surface
                            .window_redraw_shadow_blur
                            .insert(root_id.clone(), if ENABLE_DAMAGE_TRACKING { 3 } else { 0 });
                    }
                }

                let expose_render_bundle = if state.expose.active {
                    let eased = state.expose.eased_progress();
                    let usable = state.usable_zone_from_layer_map(&out_surface.output, &layer_map);
                    let usable_cards = crate::expose::expose_usable_zone_with_spaces_bar(usable);
                    let out_name = out_surface.output.name();
                    let hovered_idx = state.expose.hovered_index;
                    let selected_idx = state.expose.selected_index;
                    let quick_look_idx = state.expose.quick_look_index;
                    let show_overlays = !expose_animating || eased >= 0.85;
                    let active_drag = state
                        .expose
                        .dragging
                        .as_ref()
                        .filter(|d| d.has_moved)
                        .map(|d| (d.slot_index, crate::expose::drag_card_rect(d)));
                    let spaces_info = state.space_thumbnails_for_output(&out_surface.output);
                    let active_ws = state.workspaces.active_index;
                    let can_add_ws = state.workspaces.count() < crate::workspaces::MAX_WORKSPACES;
                    let ws_rev = state.workspace_revision;

                    let slots_snapshot: Vec<_> = state
                        .expose
                        .slots
                        .iter()
                        .enumerate()
                        .filter(|(_, s)| s.output_name == out_name)
                        .map(|(idx, s)| {
                            let is_dragged = active_drag.map_or(false, |(di, _)| di == idx);
                            let slot_rect = if let Some((di, dr)) = active_drag {
                                if di == idx {
                                    dr
                                } else {
                                    s.current_rect(eased)
                                }
                            } else {
                                let target = if quick_look_idx == Some(idx) {
                                    crate::expose::quick_look_rect(usable_cards, s.original_rect)
                                } else {
                                    s.target_rect
                                };
                                let t = eased as f64;
                                Rectangle::new(
                                    (
                                        (s.original_rect.loc.x as f64
                                            + (target.loc.x - s.original_rect.loc.x) as f64 * t)
                                            .round() as i32,
                                        (s.original_rect.loc.y as f64
                                            + (target.loc.y - s.original_rect.loc.y) as f64 * t)
                                            .round() as i32,
                                    )
                                        .into(),
                                    (
                                        (s.original_rect.size.w as f64
                                            + (target.size.w - s.original_rect.size.w) as f64 * t)
                                            .round()
                                            .max(1.0)
                                            as i32,
                                        (s.original_rect.size.h as f64
                                            + (target.size.h - s.original_rect.size.h) as f64 * t)
                                            .round()
                                            .max(1.0)
                                            as i32,
                                    )
                                        .into(),
                                )
                            };
                            (
                                idx,
                                is_dragged,
                                s.window.clone(),
                                s.original_rect,
                                slot_rect,
                                s.title.clone(),
                                s.matches_filter,
                            )
                        })
                        .collect();

                    let spaces_bar_elem = if show_overlays || active_drag.is_some() {
                        state.expose.render_spaces_bar(
                            &mut renderer,
                            output_geo,
                            usable,
                            scale,
                            &spaces_info,
                            active_ws,
                            can_add_ws,
                            ws_rev,
                        )
                    } else {
                        None
                    };

                    let filter_elem = if show_overlays {
                        state
                            .expose
                            .render_filter_pill(&mut renderer, out_surface.size, scale)
                    } else {
                        None
                    };

                    let mut cards = Vec::with_capacity(slots_snapshot.len());
                    for (idx, is_dragged, win, orig_rect, slot_rect, title, matches_filter) in
                        slots_snapshot
                    {
                        let win_geo_loc = win.geometry().loc;
                        let orig_phys_loc = (orig_rect.loc - output_geo.loc)
                            .to_f64()
                            .to_physical(scale)
                            .to_i32_round();
                        let surf_phys_loc = (orig_rect.loc - win_geo_loc - output_geo.loc)
                            .to_f64()
                            .to_physical(scale)
                            .to_i32_round();
                        let card_phys_loc = (slot_rect.loc - output_geo.loc)
                            .to_f64()
                            .to_physical(scale)
                            .to_i32_round();
                        let card_phys_size =
                            slot_rect.size.to_f64().to_physical(scale).to_i32_round();
                        let card_phys = Rectangle::new(card_phys_loc, card_phys_size);
                        let k = ((slot_rect.size.w as f64 / orig_rect.size.w.max(1) as f64)
                            .min(slot_rect.size.h as f64 / orig_rect.size.h.max(1) as f64))
                        .clamp(0.02, 4.0);
                        let alpha = if is_dragged {
                            0.92
                        } else if matches_filter {
                            1.0
                        } else {
                            0.28
                        };

                        let raw_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                            if let Some(surf) = win.wl_surface() {
                                render_elements_from_surface_tree(
                                    &mut renderer,
                                    &surf,
                                    surf_phys_loc,
                                    smithay::utils::Scale::from(scale),
                                    alpha,
                                    Kind::Unspecified,
                                )
                            } else {
                                Vec::new()
                            };
                        let scaled_elems: Vec<_> = raw_elems
                            .into_iter()
                            .map(|elem| {
                                let tex_params =
                                    crate::window_corners::SurfaceTexParams::from_wayland_element(
                                        &elem,
                                    );
                                let rescaled = smithay::backend::renderer::element::utils::RescaleRenderElement::from_element(
                                    elem,
                                    orig_phys_loc,
                                    smithay::utils::Scale::from(k),
                                );
                                let relocated = smithay::backend::renderer::element::utils::RelocateRenderElement::from_element(
                                    rescaled,
                                    card_phys_loc - orig_phys_loc,
                                    smithay::backend::renderer::element::utils::Relocate::Relative,
                                );
                                (relocated, tex_params)
                            })
                            .collect();

                        let is_selected = selected_idx == Some(idx);
                        let is_highlighted =
                            !is_dragged && (is_selected || hovered_idx == Some(idx));
                        let highlight_elem = if show_overlays && is_highlighted {
                            state.expose.render_highlight(
                                &mut renderer,
                                card_phys,
                                scale,
                                is_selected,
                            )
                        } else {
                            None
                        };
                        let label_elem = if show_overlays && !is_dragged {
                            state.expose.render_card_label(
                                &mut renderer,
                                card_phys,
                                scale,
                                &title,
                                is_highlighted,
                            )
                        } else {
                            None
                        };
                        cards.push((
                            idx == quick_look_idx.unwrap_or(usize::MAX),
                            is_dragged,
                            card_phys,
                            scaled_elems,
                            highlight_elem,
                            label_elem,
                        ));
                    }
                    cards.sort_by_key(|(is_ql, is_dragged, ..)| (*is_dragged, *is_ql));
                    Some((eased, cards, filter_elem, spaces_bar_elem))
                } else {
                    None
                };

                let mut this_frame_extra_damage: Vec<Rectangle<i32, Physical>> = Vec::new();

                // Daño del cursor físico sincronizado exactamente con la textura activa y el historial de buffers DRM
                let current_cursor_rect = if let Some((phys_x, phys_y)) = cursor_draw_point {
                    if !cursor_elems.is_empty() {
                        let elem = &cursor_elems[0];
                        let view_dst = elem.view().dst;
                        if view_dst.w > 0 && view_dst.h > 0 {
                            let target_logical_w = effective_cursor_size;
                            let target_logical_h =
                                target_logical_w * (view_dst.h as f64 / view_dst.w as f64);
                            let ratio_x = target_logical_w / view_dst.w as f64;
                            let ratio_y = target_logical_h / view_dst.h as f64;
                            let hot_phys_x =
                                (cursor_hotspot.x as f64 * ratio_x * scale).round() as i32;
                            let hot_phys_y =
                                (cursor_hotspot.y as f64 * ratio_y * scale).round() as i32;
                            let cursor_phys_w = (target_logical_w * scale).round() as i32;
                            let cursor_phys_h = (target_logical_h * scale).round() as i32;
                            Some(Rectangle::new(
                                (phys_x - hot_phys_x - 2, phys_y - hot_phys_y - 2).into(),
                                (cursor_phys_w + 4, cursor_phys_h + 4).into(),
                            ))
                        } else {
                            None
                        }
                    } else {
                        let active_cursor = match state.cursor_status {
                            CursorImageStatus::Named(icon)
                                if icon != smithay::input::pointer::CursorIcon::Default =>
                            {
                                named_cursor_cache
                                    .get(&icon)
                                    .and_then(|c| c.as_ref())
                                    .or(cursor_texture.as_ref())
                            }
                            _ => cursor_texture.as_ref(),
                        };
                        if let Some(cur) = active_cursor {
                            let scale_factor = (effective_cursor_size * scale) / cur.size as f64;
                            let cursor_w = (cur.width as f64 * scale_factor).round() as i32;
                            let cursor_h = (cur.height as f64 * scale_factor).round() as i32;
                            let hot_x = (cur.xhot as f64 * scale_factor).round() as i32;
                            let hot_y = (cur.yhot as f64 * scale_factor).round() as i32;
                            Some(Rectangle::new(
                                (phys_x - hot_x - 2, phys_y - hot_y - 2).into(),
                                (cursor_w + 4, cursor_h + 4).into(),
                            ))
                        } else {
                            None
                        }
                    }
                } else {
                    None
                };

                if ENABLE_DAMAGE_TRACKING {
                    let buffers_to_check = age_to_use.max(1).min(out_surface.cursor_history.len());
                    let cursor_needs_render = out_surface
                        .cursor_history
                        .iter()
                        .take(buffers_to_check)
                        .any(|r| Some(*r) != current_cursor_rect)
                        || (out_surface.cursor_history.is_empty() && current_cursor_rect.is_some());

                    if cursor_needs_render {
                        if let Some(new_rect) = current_cursor_rect {
                            this_frame_extra_damage.push(new_rect);
                        }
                        for old_rect in out_surface.cursor_history.iter().take(buffers_to_check) {
                            if !old_rect.is_empty() {
                                this_frame_extra_damage.push(*old_rect);
                            }
                        }
                    }
                }

                // Daño por animaciones activas (Genie, Snap Preview, transiciones de tema)
                if !active_genies.is_empty()
                    || !state.snap_transitions.is_empty()
                    || state.snap_preview.is_some()
                    || state.theme_transition_started_at.is_some()
                {
                    this_frame_extra_damage.push(Rectangle::from_size(out_surface.size));
                }

                // Daño por Spotlight, UI de Captura, Snap Layouts o App Switcher Deck
                if spotlight_frame.is_some()
                    || screenshot_ui_frame.is_some()
                    || snap_layouts_frame.is_some()
                    || media_osd_frame.is_some()
                    || app_switcher_deck_frame.is_some()
                {
                    this_frame_extra_damage.push(Rectangle::from_size(out_surface.size));
                }

                if screenshot_active
                    || recording_due_on_output
                    || theme_capture_request.is_some()
                    || state.theme_transition_started_at.is_some()
                    || workspace_switch_pending
                    || state.workspace_transition.is_some()
                {
                    this_frame_extra_damage.push(Rectangle::from_size(out_surface.size));
                }

                // Detección estricta de cambios geométricos en ventanas sincronizada con los 3 buffers DRM
                let mut window_needs_shadow_blur = if ENABLE_DAMAGE_TRACKING {
                    vec![false; window_render_data.len()]
                } else {
                    vec![true; window_render_data.len()]
                };
                for (idx, (root_id, _, _, _, _, is_active, win_rect, ..)) in
                    window_render_data.iter().enumerate()
                {
                    let last = out_surface.last_window_geometries.get(root_id);
                    let geom_changed = match last {
                        Some((prev_rect, prev_active)) => {
                            *prev_rect != *win_rect || *prev_active != *is_active
                        }
                        None => true,
                    };
                    if geom_changed || age_to_use == 0 {
                        out_surface
                            .window_redraw_shadow_blur
                            .insert(root_id.clone(), 3);
                        let raw_shadow_rect = Rectangle::new(
                            (win_rect.loc.x - shadow_pad, win_rect.loc.y - shadow_pad).into(),
                            (
                                win_rect.size.w + shadow_pad * 2,
                                win_rect.size.h + shadow_pad * 2,
                            )
                                .into(),
                        );
                        this_frame_extra_damage.push(raw_shadow_rect);
                        if let Some((prev_rect, _)) = last {
                            let prev_shadow = Rectangle::new(
                                (prev_rect.loc.x - shadow_pad, prev_rect.loc.y - shadow_pad).into(),
                                (
                                    prev_rect.size.w + shadow_pad * 2,
                                    prev_rect.size.h + shadow_pad * 2,
                                )
                                    .into(),
                            );
                            this_frame_extra_damage.push(prev_shadow);
                        }
                    }

                    if out_surface
                        .window_redraw_shadow_blur
                        .get(root_id)
                        .copied()
                        .unwrap_or(0)
                        > 0
                    {
                        window_needs_shadow_blur[idx] = true;
                        let raw_shadow_rect = Rectangle::new(
                            (win_rect.loc.x - shadow_pad, win_rect.loc.y - shadow_pad).into(),
                            (
                                win_rect.size.w + shadow_pad * 2,
                                win_rect.size.h + shadow_pad * 2,
                            )
                                .into(),
                        );
                        this_frame_extra_damage.push(raw_shadow_rect);
                    }
                }

                // Si alguna ventana se cerró o minimizó, invalidar su sombra previa
                let current_ids: HashSet<_> = window_render_data.iter().map(|w| &w.0).collect();
                out_surface.last_window_geometries.retain(|id, (rect, _)| {
                    if !current_ids.contains(id) {
                        let old_shadow = Rectangle::new(
                            (rect.loc.x - shadow_pad, rect.loc.y - shadow_pad).into(),
                            (rect.size.w + shadow_pad * 2, rect.size.h + shadow_pad * 2).into(),
                        );
                        this_frame_extra_damage.push(old_shadow);
                        false
                    } else {
                        true
                    }
                });
                out_surface
                    .window_redraw_shadow_blur
                    .retain(|id, _| current_ids.contains(id));

                // Sincronización robusta del Dock con el ciclo de triple-buffering de DRM
                let dock_shadow_pad = (80.0 * scale).round() as i32;
                let mut current_dock_phys: Option<Rectangle<i32, Physical>> = None;
                for surface in layer_map.layers() {
                    if surface.namespace() == "malus-dock" {
                        if let Some(geo) = layer_map.layer_geometry(surface) {
                            let dock_phys = geo.to_physical_precise_round(scale);
                            if super::dock_is_collapsed(dock_phys, scale, state.dock_cfg.position) {
                                continue;
                            }
                            current_dock_phys = Some(dock_phys);
                            let dock_shadow_rect = Rectangle::new(
                                (
                                    dock_phys.loc.x - dock_shadow_pad,
                                    dock_phys.loc.y - dock_shadow_pad,
                                )
                                    .into(),
                                (
                                    dock_phys.size.w + dock_shadow_pad * 2,
                                    dock_phys.size.h + dock_shadow_pad * 2,
                                )
                                    .into(),
                            );
                            let dock_touched = this_frame_extra_damage
                                .iter()
                                .any(|d| d.overlaps(dock_phys))
                                || current_cursor_rect
                                    .map_or(false, |c| c.overlaps(dock_shadow_rect));
                            if dock_touched {
                                out_surface.dock_redraw_buffers = 3;
                            }
                            if out_surface.dock_redraw_buffers > 0 {
                                this_frame_extra_damage.push(dock_shadow_rect);
                            }
                        }
                    }
                }
                if out_surface.last_dock_rect != current_dock_phys {
                    if let Some(old_dock) = out_surface.last_dock_rect {
                        this_frame_extra_damage.push(Rectangle::new(
                            (
                                old_dock.loc.x - dock_shadow_pad,
                                old_dock.loc.y - dock_shadow_pad,
                            )
                                .into(),
                            (
                                old_dock.size.w + dock_shadow_pad * 2,
                                old_dock.size.h + dock_shadow_pad * 2,
                            )
                                .into(),
                        ));
                    }
                    out_surface.dock_redraw_buffers = 3;
                    out_surface.last_dock_rect = current_dock_phys;
                }

                // Expansión de daño de globos de notificación y Control Center flotante (sombra/refracción GPU)
                let notif_shadow_pad = (48.0 * scale).round() as i32;
                for surface in layer_map.layers() {
                    if matches!(
                        surface.namespace(),
                        "malus-notifications"
                            | "malus-notification-center"
                            | "malus-control-center"
                            | "malus-control-center-editor"
                    ) {
                        if let Some(geo) = layer_map.layer_geometry(surface) {
                            let r: Rectangle<i32, Physical> =
                                geo.to_physical_precise_round(scale);
                            this_frame_extra_damage.push(Rectangle::new(
                                (r.loc.x - notif_shadow_pad, r.loc.y - notif_shadow_pad).into(),
                                (
                                    r.size.w + notif_shadow_pad * 2,
                                    r.size.h + notif_shadow_pad * 2,
                                )
                                    .into(),
                            ));
                        }
                    }
                }

                // Expansión de daño de popups (Top Bar menus, Control Center, Calendario)
                for (_, popup_rect) in &topbar_popups {
                    let pad = (32.0 * scale).round() as i32;
                    this_frame_extra_damage.push(Rectangle::new(
                        (popup_rect.loc.x - pad, popup_rect.loc.y - pad).into(),
                        (popup_rect.size.w + pad * 2, popup_rect.size.h + pad * 2).into(),
                    ));
                }
                for (_, popup_rect) in &dock_popups {
                    let pad = (32.0 * scale).round() as i32;
                    this_frame_extra_damage.push(Rectangle::new(
                        (popup_rect.loc.x - pad, popup_rect.loc.y - pad).into(),
                        (popup_rect.size.w + pad * 2, popup_rect.size.h + pad * 2).into(),
                    ));
                }

                let mut render_damage = if ENABLE_DAMAGE_TRACKING {
                    if age_to_use == 0 {
                        vec![Rectangle::from_size(out_surface.size)]
                    } else {
                        let mut dmg = match out_surface
                            .damage_tracker
                            .damage_output(age_to_use, &tracker_elements)
                        {
                            Ok((Some(d), _)) => d.clone(),
                            Ok((None, _)) => Vec::new(),
                            Err(_) => vec![Rectangle::from_size(out_surface.size)],
                        };
                        dmg.extend_from_slice(&this_frame_extra_damage);
                        for old_extra in
                            out_surface.extra_damage_history.iter().take(age_to_use - 1)
                        {
                            dmg.extend_from_slice(old_extra);
                        }
                        dmg
                    }
                } else {
                    vec![Rectangle::from_size(out_surface.size)]
                };

                render_damage.retain_mut(|rect| {
                    if let Some(clamped) = rect.intersection(screen_bounds) {
                        *rect = clamped;
                        !rect.is_empty()
                    } else {
                        false
                    }
                });

                // Consolidación y fusión de rectángulos superpuestos en regiones disjuntas
                // para evitar sobre-dibujo o doble blending en quads translúcidos
                render_damage =
                    render_damage
                        .into_iter()
                        .fold(Vec::new(), |new_damage, mut rect| {
                            let (overlapping, mut new_damage): (Vec<_>, Vec<_>) = new_damage
                                .into_iter()
                                .partition(|other| other.overlaps_or_touches(rect));
                            for overlap in overlapping {
                                rect = rect.merge(overlap);
                            }
                            new_damage.push(rect);
                            new_damage
                        });

                // Si no hay cambios en esta pantalla física, ¡evitar por completo el sobre-dibujo GPU!
                if render_damage.is_empty() {
                    continue;
                }

                let damage = render_damage.as_slice();

                let mut frame_rendered = false;
                if let Ok(mut target) = renderer.bind(&mut dmabuf) {
                    let screenshot_request = state
                        .pending_screenshot_requests
                        .iter()
                        .position(|(_, output_name)| {
                            output_name
                                .as_deref()
                                .map_or(true, |name| name == out_surface.output.name())
                        })
                        .and_then(|index| state.pending_screenshot_requests.remove(index))
                        .map(|(destination, _)| destination);
                    if let Ok(mut frame) =
                        renderer.render(&mut target, out_surface.size, Transform::Normal)
                    {
                        frame_rendered = true;

                        let clear_color = if session_locked {
                            Color32F::new(0.0, 0.0, 0.0, 1.0)
                        } else {
                            dark_acrylic
                        };
                        let _ = frame.clear(clear_color, damage);
                        if session_locked {
                            if !lock_elements.is_empty() {
                                let _ =
                                    draw_render_elements(&mut frame, scale, &lock_elements, damage);
                            }
                            if let Some((
                                osd_rect,
                                ref mask_vec,
                                (mask_w, mask_h),
                                osd_alpha,
                                ref osd_elem,
                            )) = media_osd_frame
                            {
                                let projection = *frame.projection();
                                if let Some(shadow) = shadow_pipeline.as_ref() {
                                    let _ = frame.with_context(|gl| {
                                        shadow.render_menu_shadow_alpha(
                                            gl,
                                            &projection,
                                            osd_rect,
                                            (crate::media_keys::OSD_CORNER_RADIUS * scale) as f32,
                                            scale,
                                            osd_alpha,
                                        );
                                    });
                                }
                                if let Some(mut blur) =
                                    blur_pipeline.as_ref().and_then(|b| b.lock().ok())
                                {
                                    let _ = frame.with_context(|gl| {
                                        blur.render_blur_masked(
                                            gl,
                                            &projection,
                                            out_surface.size,
                                            osd_rect,
                                            mask_vec.as_deref(),
                                            mask_w,
                                            mask_h,
                                            osd_alpha,
                                        );
                                    });
                                }
                                let _ = draw_render_elements(
                                    &mut frame,
                                    1.0,
                                    std::slice::from_ref(osd_elem),
                                    damage,
                                );
                            }
                        } else {
                            // 1. Renderizar fondo (wallpaper y capas inferiores) solo si no está 100% ocluido por una ventana pantalla completa
                            if !wallpaper_occluded && !lower_elements.is_empty() {
                                let _ = draw_render_elements(
                                    &mut frame,
                                    scale,
                                    &lower_elements,
                                    damage,
                                );
                            }

                            // Inicializar pipeline de sombras squircle si aún no se intentó
                            if !shadow_init_attempted {
                                shadow_init_attempted = true;
                                shadow_pipeline = frame
                                    .with_context(|gl| crate::shadow::WindowShadowPipeline::new(gl))
                                    .ok()
                                    .flatten()
                                    .map(std::sync::Arc::new);
                            }

                            // Inicializar pipeline de desenfoque Dual Kawase si aún no se intentó
                            if !blur_init_attempted {
                                blur_init_attempted = true;
                                blur_pipeline = frame
                                    .with_context(|gl| crate::blur::GaussianBlur::new(gl))
                                    .ok()
                                    .flatten()
                                    .map(|b| std::sync::Arc::new(std::sync::Mutex::new(b)));
                            }

                            let projection = *frame.projection();

                            // 1.1. Capturar el fondo limpio DESPUÉS de dibujar el wallpaper y ANTES de dibujar ventanas o sombras.
                            // Esto garantiza que el desenfoque Dual-Kawase nunca lea fragmentos de ventanas ni sombras.
                            // Cuando solo cambian iconos del escritorio (`malus-wallpaper`), omitimos la copia completa de pantalla
                            // salvo en el área dañada de la ventana translúcida inferior (si existe), evitando stalls de GPU por cuadro.
                            let force_full_bg_capture = out_surface.background_dirty
                                || out_surface.last_background_size
                                    != (out_surface.size.w, out_surface.size.h)
                                || expose_render_bundle.is_some()
                                || app_switcher_deck_frame.is_some()
                                || state.theme_transition_started_at.is_some()
                                || theme_capture_request.is_some()
                                || state.workspace_transition.is_some()
                                || workspace_switch_pending;
                            let bg_committed = out_surface.last_background_commit_counter
                                != state.background_commit_counter;

                            let (bottom_translucent_rect, bottom_needs_shadow_blur) =
                                if app_switcher_deck_frame.is_none() {
                                    if let Some((
                                        _,
                                        win_elems,
                                        _,
                                        _,
                                        _,
                                        _,
                                        win_rect,
                                        _,
                                        _,
                                        _,
                                        is_translucent,
                                        transition_clip,
                                        _,
                                        _,
                                        is_branches,
                                        is_full_translucent,
                                    )) = window_render_data.first()
                                    {
                                        let needs_transition_backdrop = transition_clip
                                            .is_some_and(|c| {
                                                c.size.w < win_rect.size.w
                                                    || (c.loc.y + c.size.h)
                                                        < (win_rect.loc.y + win_rect.size.h)
                                            });
                                        let is_visible = !win_elems.is_empty() && !body_occluded[0];
                                        if is_visible
                                            && (*is_translucent || needs_transition_backdrop)
                                        {
                                            let rect =
                                                if needs_transition_backdrop && !*is_translucent {
                                                    *win_rect
                                                } else {
                                                    super::translucent_window_blur_rect(
                                                        *win_rect,
                                                        *is_branches,
                                                        *is_translucent,
                                                        *is_full_translucent,
                                                        win_phys_r,
                                                        scale,
                                                    )
                                                };
                                            (Some(rect), window_needs_shadow_blur[0])
                                        } else {
                                            (None, false)
                                        }
                                    } else {
                                        (None, false)
                                    }
                                } else {
                                    (None, false)
                                };

                            let capture_plan = super::plan_background_blur_capture(
                                force_full_bg_capture,
                                bg_committed,
                                out_surface.background_snapshot_stale,
                                bottom_translucent_rect,
                                bottom_needs_shadow_blur,
                                damage,
                            );

                            if !wallpaper_occluded {
                                match capture_plan {
                                    super::BackgroundBlurCapturePlan::FullScreen => {
                                        if let Some(mut blur) =
                                            blur_pipeline.as_ref().and_then(|b| b.lock().ok())
                                        {
                                            let _ = frame.with_context(|gl| {
                                                blur.capture_background(
                                                    gl,
                                                    &projection,
                                                    out_surface.size,
                                                );
                                            });
                                            out_surface.background_dirty = false;
                                            out_surface.background_snapshot_stale = false;
                                            out_surface.last_background_commit_counter =
                                                state.background_commit_counter;
                                            out_surface.last_background_size =
                                                (out_surface.size.w, out_surface.size.h);
                                        }
                                    }
                                    super::BackgroundBlurCapturePlan::Region(capture_rect) => {
                                        if let Some(mut blur) =
                                            blur_pipeline.as_ref().and_then(|b| b.lock().ok())
                                        {
                                            let _ = frame.with_context(|gl| {
                                                blur.capture_background_region(
                                                    gl,
                                                    &projection,
                                                    out_surface.size,
                                                    capture_rect,
                                                );
                                            });
                                            if bg_committed {
                                                out_surface.background_snapshot_stale = true;
                                                out_surface.last_background_commit_counter =
                                                    state.background_commit_counter;
                                            }
                                        }
                                    }
                                    super::BackgroundBlurCapturePlan::None => {
                                        if bg_committed {
                                            out_surface.background_snapshot_stale = true;
                                            out_surface.last_background_commit_counter =
                                                state.background_commit_counter;
                                        }
                                    }
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

                            let menu_radius =
                                (super::POPUP_MENU_CORNER_RADIUS_LOGICAL * scale).round() as i32;
                            if let Some((
                                eased,
                                ref expose_cards,
                                ref filter_elem,
                                ref spaces_bar_elem,
                            )) = expose_render_bundle
                            {
                                if let Some(mut blur) =
                                    blur_pipeline.as_ref().and_then(|b| b.lock().ok())
                                {
                                    let _ = frame.with_context(|gl| {
                                        blur.render_captured_blur_round(
                                            gl,
                                            &projection,
                                            out_surface.size,
                                            Rectangle::from_size(out_surface.size),
                                            0.0,
                                        );
                                    });
                                }
                                let _ = frame.draw_solid(
                                    screen_bounds,
                                    damage,
                                    Color32F::new(0.04, 0.05, 0.08, 0.44 * eased),
                                );
                                let card_radius = crate::expose::card_corner_radius_phys(scale);
                                for draw_dragged in [false, true] {
                                    if draw_dragged {
                                        if let Some(ref sb) = spaces_bar_elem {
                                            let _ = draw_render_elements(
                                                &mut frame,
                                                1.0,
                                                std::slice::from_ref(sb),
                                                damage,
                                            );
                                        }
                                    }
                                    for (
                                        _,
                                        _,
                                        card_phys,
                                        scaled_elems,
                                        highlight_elem,
                                        label_elem,
                                    ) in
                                        expose_cards.iter().filter(|(_, d, ..)| *d == draw_dragged)
                                    {
                                        if let Some(ref hl) = highlight_elem {
                                            let _ = draw_render_elements(
                                                &mut frame,
                                                1.0,
                                                std::slice::from_ref(hl),
                                                damage,
                                            );
                                        }
                                        if let Some(ref shader) = window_squircle_shader.as_ref() {
                                            for (elem, tex_params) in scaled_elems.iter().rev() {
                                                let elem_geo = elem
                                                    .geometry(smithay::utils::Scale::from(scale));
                                                let sub_offset = Point::from((
                                                    card_phys.loc.x - elem_geo.loc.x,
                                                    card_phys.loc.y - elem_geo.loc.y,
                                                ));
                                                shader.apply_surface_to_frame(
                                                    &mut frame,
                                                    *tex_params,
                                                    elem_geo.size,
                                                    sub_offset,
                                                    card_phys.size,
                                                    card_radius,
                                                    1.0,
                                                    false,
                                                );
                                                let _ = draw_render_elements(
                                                    &mut frame,
                                                    scale,
                                                    std::slice::from_ref(elem),
                                                    damage,
                                                );
                                                frame.clear_tex_program_override();
                                            }
                                        } else {
                                            for (elem, _) in scaled_elems.iter().rev() {
                                                let _ = draw_render_elements(
                                                    &mut frame,
                                                    scale,
                                                    std::slice::from_ref(elem),
                                                    damage,
                                                );
                                            }
                                        }
                                        if let Some(ref lbl) = label_elem {
                                            let _ = draw_render_elements(
                                                &mut frame,
                                                1.0,
                                                std::slice::from_ref(lbl),
                                                damage,
                                            );
                                        }
                                    }
                                }
                                if let Some(ref fp) = filter_elem {
                                    let _ = draw_render_elements(
                                        &mut frame,
                                        1.0,
                                        std::slice::from_ref(fp),
                                        damage,
                                    );
                                }
                            } else {
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
                                        is_maximized,
                                        is_branches,
                                        is_full_translucent,
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
                                                if output_geo.overlaps(preview) {
                                                    let mut blur_guard = blur_pipeline
                                                        .as_ref()
                                                        .and_then(|b| b.lock().ok());
                                                    crate::blur::render_snap_preview_overlay(
                                                        &mut frame,
                                                        blur_guard.as_deref_mut(),
                                                        &projection,
                                                        out_surface.size,
                                                        preview,
                                                        output_geo.loc,
                                                        scale,
                                                        win_phys_r,
                                                    );
                                                }
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
                                    let raw_shadow_rect = Rectangle::new(
                                        (win_rect.loc.x - shadow_pad, win_rect.loc.y - shadow_pad)
                                            .into(),
                                        (
                                            win_rect.size.w + shadow_pad * 2,
                                            win_rect.size.h + shadow_pad * 2,
                                        )
                                            .into(),
                                    );
                                    let shadow_is_damaged = window_needs_shadow_blur[idx]
                                        || damage.iter().any(|d| d.overlaps(raw_shadow_rect));
                                    if *should_round && !is_shadow_occluded && shadow_is_damaged {
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
                                                    scale,
                                                );
                                            });
                                        }
                                    }

                                    let win_is_damaged =
                                        damage.iter().any(|d| d.overlaps(*win_rect));

                                    // 2.1.5. DESENFOQUE DE FONDO DUAL KAWASE (Branches y Ajustes del Sistema)
                                    let blur_rect = super::translucent_window_blur_rect(
                                        *win_rect,
                                        *is_branches,
                                        *is_translucent,
                                        *is_full_translucent,
                                        win_phys_r,
                                        scale,
                                    );
                                    let blur_is_damaged = window_needs_shadow_blur[idx]
                                        || damage.iter().any(|d| d.overlaps(blur_rect));
                                    if *is_translucent && !is_body_occluded && blur_is_damaged {
                                        if let Some(mut blur) =
                                            blur_pipeline.as_ref().and_then(|b| b.lock().ok())
                                        {
                                            let corner_radius = if *should_round {
                                                win_phys_r.unwrap_or(0) as f32
                                            } else {
                                                0.0
                                            };
                                            let screen_size = out_surface.size;
                                            let _ = frame.with_context(|gl| {
                                                // Si hay ventanas dibujadas debajo de esta ventana translúcida (idx > 0),
                                                // actualizar la región de snapshot con el contenido de las ventanas inferiores
                                                // para que el desenfoque refleje tanto el wallpaper como las ventanas subyacentes.
                                                if idx > 0 {
                                                    blur.capture_background_region(
                                                        gl,
                                                        &projection,
                                                        screen_size,
                                                        blur_rect,
                                                    );
                                                }
                                                blur.render_captured_blur(
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
                                    let needs_transition_backdrop =
                                        transition_clip.is_some_and(|c| {
                                            c.size.w < win_rect.size.w
                                                || (c.loc.y + c.size.h)
                                                    < (win_rect.loc.y + win_rect.size.h)
                                        });
                                    if needs_transition_backdrop
                                        && !is_body_occluded
                                        && (win_is_damaged || window_needs_shadow_blur[idx])
                                    {
                                        let corner_radius = if *should_round {
                                            win_phys_r.unwrap_or(0) as f32
                                        } else {
                                            0.0
                                        };
                                        let mut blur_guard =
                                            blur_pipeline.as_ref().and_then(|b| b.lock().ok());
                                        crate::blur::render_snap_transition_backdrop(
                                            &mut frame,
                                            blur_guard.as_deref_mut(),
                                            &projection,
                                            out_surface.size,
                                            *win_rect,
                                            corner_radius,
                                            idx > 0,
                                            *is_translucent,
                                            *is_branches,
                                            is_dark_theme,
                                        );
                                    }
                                    if !is_body_occluded && win_is_damaged {
                                        crate::window_corners::render_window_elements(
                                            &mut frame,
                                            scale,
                                            damage,
                                            window_squircle_shader.as_deref(),
                                            root_id,
                                            win_elems,
                                            ssd_elem.as_ref(),
                                            *win_rect,
                                            win_phys_r,
                                            *should_round,
                                            *is_firefox,
                                            *is_maximized,
                                            *transition_clip,
                                        );
                                    }
                                }

                                // Si no se dibujó antes (por ejemplo, si no hay ventanas en este monitor), dibujarlo sobre el fondo
                                if !snap_preview_drawn && is_pointer_grabbed {
                                    if let Some(preview) = state.snap_preview {
                                        if output_geo.overlaps(preview) {
                                            let mut blur_guard =
                                                blur_pipeline.as_ref().and_then(|b| b.lock().ok());
                                            crate::blur::render_snap_preview_overlay(
                                                &mut frame,
                                                blur_guard.as_deref_mut(),
                                                &projection,
                                                out_surface.size,
                                                preview,
                                                output_geo.loc,
                                                scale,
                                                win_phys_r,
                                            );
                                        }
                                    }
                                }

                                // 2. Aplicar desenfoque de fondo Dual Kawase por GPU a las capas y popups translúcidos
                                if !blur_init_attempted {
                                    blur_init_attempted = true;
                                    blur_pipeline = frame
                                        .with_context(|gl| crate::blur::GaussianBlur::new(gl))
                                        .ok()
                                        .flatten()
                                        .map(|b| std::sync::Arc::new(std::sync::Mutex::new(b)));
                                }

                                if let Some(mut blur) =
                                    blur_pipeline.as_ref().and_then(|b| b.lock().ok())
                                {
                                    let screen_size = out_surface.size;
                                    let mut layer_blur_items: Vec<(
                                        Rectangle<i32, Physical>,
                                        f32,
                                        bool,
                                        u8,
                                    )> = Vec::new();
                                    for (_, rect) in &spotlight_backdrop_elements {
                                        if damage.iter().any(|d| d.overlaps(*rect)) {
                                            layer_blur_items.push((
                                                *rect,
                                                26.0 * scale as f32,
                                                false,
                                                0,
                                            ));
                                        }
                                    }
                                    let mut cc_refract_items: Vec<(
                                        Rectangle<i32, Physical>,
                                        f32,
                                        bool,
                                    )> = Vec::new();
                                    for surface in layer_map.layers() {
                                        if matches!(surface.layer(), Layer::Top | Layer::Overlay) {
                                            if has_fullscreen_window
                                                && surface.layer() == Layer::Top
                                            {
                                                continue;
                                            }
                                            let ns = surface.namespace();
                                            if ns == "malus-app-switcher"
                                                && !state.app_switcher_classic_active()
                                            {
                                                continue;
                                            }
                                            if ns == "malus-top-bar" {
                                                continue;
                                            }
                                            if ns == "malus-control-center" {
                                                if let Some(geo) =
                                                    layer_map.layer_geometry(surface)
                                                {
                                                    let base: Rectangle<i32, Physical> =
                                                        geo.to_physical_precise_round(scale);
                                                    for r in &state.control_center_glass_regions {
                                                        if r.width < 4 || r.height < 4 {
                                                            continue;
                                                        }
                                                        let rx = base.loc.x
                                                            + (r.x as f64 * scale).round() as i32;
                                                        let ry = base.loc.y
                                                            + (r.y as f64 * scale).round() as i32;
                                                        let rw = (r.width as f64 * scale).round()
                                                            as i32;
                                                        let rh = (r.height as f64 * scale).round()
                                                            as i32;
                                                        let rect = Rectangle::new(
                                                            Point::from((rx, ry)),
                                                            Size::from((rw, rh)),
                                                        );
                                                        let pad = (24.0 * scale).round() as i32;
                                                        let check = Rectangle::new(
                                                            Point::from((rx - pad, ry - pad)),
                                                            Size::from((
                                                                rw + pad * 2,
                                                                rh + pad * 2,
                                                            )),
                                                        );
                                                        if damage.iter().any(|d| d.overlaps(check))
                                                        {
                                                            let eff_r = (r.radius * scale as f32)
                                                                .min(rw as f32 * 0.5)
                                                                .min(rh as f32 * 0.5);
                                                            cc_refract_items.push((
                                                                rect,
                                                                eff_r,
                                                                r.is_capsule,
                                                            ));
                                                        }
                                                    }
                                                }
                                                continue;
                                            }
                                            if let Some(geo) = layer_map.layer_geometry(surface) {
                                                let dock_radius = state.dock_cfg.dock_corner_radius();
                                                let radius = match ns.as_ref() {
                                                    "malus-dock" => dock_radius * scale as f32,
                                                    "malus-app-switcher" => 32.0 * scale as f32,
                                                    "malus-control-center-editor"
                                                    | "malus-notification-center" => {
                                                        34.0 * scale as f32
                                                    }
                                                    "malus-spotlight-controls"
                                                    | "malus-spotlight-input"
                                                    | "malus-spotlight-apps"
                                                    | "malus-spotlight-files" => {
                                                        27.0 * scale as f32
                                                    }
                                                    "malus-notifications" => {
                                                        (crate::media_keys::NOTIFICATION_CORNER_RADIUS
                                                            * scale)
                                                            as f32
                                                    }
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
                                                    let is_notification =
                                                        ns == "malus-notifications";
                                                    let is_cc_editor = matches!(
                                                        ns.as_ref(),
                                                        "malus-control-center-editor"
                                                            | "malus-notification-center"
                                                    );
                                                    let with_menu_shadow =
                                                        is_notification || is_cc_editor;
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
                                                        let rect =
                                                            geo.to_physical_precise_round(scale);
                                                        if is_spotlight_comp {
                                                            super::spotlight_component_blur_rect(
                                                                rect, scale,
                                                            )
                                                        } else if is_notification {
                                                            let inset = (8.0 * scale).round() as i32;
                                                            Rectangle::new(
                                                                Point::from((
                                                                    rect.loc.x + inset,
                                                                    rect.loc.y + inset,
                                                                )),
                                                                Size::from((
                                                                    (rect.size.w - inset).max(0),
                                                                    (rect.size.h - inset).max(0),
                                                                )),
                                                            )
                                                        } else {
                                                            rect
                                                        }
                                                    };
                                                    if phys_rect.size.w > 0 && phys_rect.size.h > 0
                                                    {
                                                        let check_rect = if with_menu_shadow {
                                                            let pad = (36.0 * scale).round() as i32;
                                                            Rectangle::new(
                                                                Point::from((
                                                                    phys_rect.loc.x - pad,
                                                                    phys_rect.loc.y - pad,
                                                                )),
                                                                Size::from((
                                                                    phys_rect.size.w + pad * 2,
                                                                    phys_rect.size.h + pad * 2,
                                                                )),
                                                            )
                                                        } else {
                                                            phys_rect
                                                        };
                                                        if damage
                                                            .iter()
                                                            .any(|d| d.overlaps(check_rect))
                                                        {
                                                            let effective_radius = if with_menu_shadow
                                                            {
                                                                radius
                                                                    .min(phys_rect.size.w as f32 * 0.5)
                                                                    .min(phys_rect.size.h as f32 * 0.5)
                                                            } else {
                                                                radius
                                                            };
                                                            let shadow_kind: u8 = if is_cc_editor {
                                                                2
                                                            } else if is_notification {
                                                                1
                                                            } else {
                                                                0
                                                            };
                                                            layer_blur_items.push((
                                                                phys_rect,
                                                                effective_radius,
                                                                is_spotlight_comp,
                                                                shadow_kind,
                                                            ));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    if !layer_blur_items.is_empty() || !cc_refract_items.is_empty()
                                    {
                                        let mut min_x = i32::MAX;
                                        let mut min_y = i32::MAX;
                                        let mut max_x = i32::MIN;
                                        let mut max_y = i32::MIN;
                                        for (rect, _, _, _) in &layer_blur_items {
                                            min_x = min_x.min(rect.loc.x);
                                            min_y = min_y.min(rect.loc.y);
                                            max_x = max_x.max(rect.loc.x + rect.size.w);
                                            max_y = max_y.max(rect.loc.y + rect.size.h);
                                        }
                                        for (rect, _, _) in &cc_refract_items {
                                            min_x = min_x.min(rect.loc.x);
                                            min_y = min_y.min(rect.loc.y);
                                            max_x = max_x.max(rect.loc.x + rect.size.w);
                                            max_y = max_y.max(rect.loc.y + rect.size.h);
                                        }
                                        let clamped_min_x = min_x.clamp(0, screen_size.w);
                                        let clamped_min_y = min_y.clamp(0, screen_size.h);
                                        let clamped_max_x = max_x.clamp(0, screen_size.w);
                                        let clamped_max_y = max_y.clamp(0, screen_size.h);
                                        let capture_region = Rectangle::new(
                                            (clamped_min_x, clamped_min_y).into(),
                                            (
                                                (clamped_max_x - clamped_min_x).max(0),
                                                (clamped_max_y - clamped_min_y).max(0),
                                            )
                                                .into(),
                                        );
                                        let _ = frame.with_context(|gl| {
                                            blur.capture_background_region(
                                                gl,
                                                &projection,
                                                screen_size,
                                                capture_region,
                                            );
                                            for (phys_rect, radius, is_round, shadow_kind) in
                                                layer_blur_items
                                            {
                                                if shadow_kind == 2 {
                                                    if let Some(shadow) = shadow_pipeline.as_ref() {
                                                        shadow.render_shadow_alpha(
                                                            gl,
                                                            &projection,
                                                            phys_rect,
                                                            radius,
                                                            true,
                                                            scale,
                                                            1.0,
                                                        );
                                                        shadow.render_menu_shadow_alpha(
                                                            gl,
                                                            &projection,
                                                            phys_rect,
                                                            radius,
                                                            scale,
                                                            0.9,
                                                        );
                                                    }
                                                } else if shadow_kind == 1 {
                                                    if let Some(shadow) = shadow_pipeline.as_ref() {
                                                        shadow.render_menu_shadow_alpha(
                                                            gl,
                                                            &projection,
                                                            phys_rect,
                                                            radius,
                                                            scale,
                                                            1.0,
                                                        );
                                                    }
                                                }
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
                                            for (phys_rect, radius, is_capsule) in cc_refract_items
                                            {
                                                if let Some(shadow) = shadow_pipeline.as_ref() {
                                                    shadow.render_menu_shadow_alpha(
                                                        gl,
                                                        &projection,
                                                        phys_rect,
                                                        radius,
                                                        scale,
                                                        0.85,
                                                    );
                                                }
                                                if is_capsule {
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
                                    for elem in elements.iter().rev() {
                                        if let Some(shader) = window_squircle_shader.as_ref() {
                                            let elem_geo =
                                                elem.geometry(smithay::utils::Scale::from(scale));
                                            shader.apply_clip_to_frame(
                                                &mut frame,
                                                elem_geo.size,
                                                rect.loc - elem_geo.loc,
                                                rect.size,
                                                (26.0 * scale).round() as i32,
                                            );
                                        }
                                        let _ = draw_render_elements(
                                            &mut frame,
                                            scale,
                                            std::slice::from_ref(elem),
                                            damage,
                                        );
                                        frame.clear_tex_program_override();
                                    }
                                }

                                if let Some(shadow) = shadow_pipeline.as_ref() {
                                    let _ = frame.with_context(|gl| {
                                        for rect in &spotlight_shadow_rects {
                                            shadow.render_shadow(
                                                gl,
                                                &projection,
                                                *rect,
                                                26.0 * scale as f32,
                                                true,
                                                scale,
                                            );
                                        }
                                    });
                                }

                                // 3. Renderizar menús contextuales y popups de ventanas (siempre ENCIMA de las superficies de ventana)
                                for (idx, (_, _, _, popup_shadow_rects, ..)) in
                                    window_render_data.iter().enumerate()
                                {
                                    if app_switcher_deck_frame.is_some() || body_occluded[idx] {
                                        continue;
                                    }
                                    if let Some(shadow) = shadow_pipeline.as_ref() {
                                        for popup_rect in popup_shadow_rects {
                                            let _ = frame.with_context(|gl| {
                                                shadow.render_menu_shadow(
                                                    gl,
                                                    &projection,
                                                    *popup_rect,
                                                    menu_radius as f32,
                                                    scale,
                                                );
                                            });
                                        }
                                    }
                                }

                                for (idx, (_, _, popups, ..)) in
                                    window_render_data.iter().enumerate()
                                {
                                    if app_switcher_deck_frame.is_some() || body_occluded[idx] {
                                        continue;
                                    }
                                    for (popup_elems, popup_rect, popup_effects_enabled) in popups {
                                        if !damage.iter().any(|d| d.overlaps(*popup_rect)) {
                                            continue;
                                        }
                                        if *popup_effects_enabled {
                                            if let Some(mut blur) =
                                                blur_pipeline.as_ref().and_then(|b| b.lock().ok())
                                            {
                                                let _ = frame.with_context(|gl| {
                                                    blur.capture_background_region(
                                                        gl,
                                                        &projection,
                                                        out_surface.size,
                                                        *popup_rect,
                                                    );
                                                    blur.render_captured_blur_strong(
                                                        gl,
                                                        &projection,
                                                        out_surface.size,
                                                        *popup_rect,
                                                        menu_radius as f32,
                                                    );
                                                });
                                            }
                                        }

                                        for elem in popup_elems.iter().rev() {
                                            if *popup_effects_enabled {
                                                if let Some(shader) =
                                                    window_squircle_shader.as_ref()
                                                {
                                                    let elem_geo = elem.geometry(
                                                        smithay::utils::Scale::from(scale),
                                                    );
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
                                                scale,
                                                std::slice::from_ref(elem),
                                                damage,
                                            );
                                            if *popup_effects_enabled {
                                                frame.clear_tex_program_override();
                                            }
                                        }
                                    }
                                }
                            }

                            // Previsualización de destino al pasar el cursor por las plantillas de Snap Layouts
                            if !is_pointer_grabbed {
                                if let Some(preview) = snap_layouts_preview {
                                    let mut blur_guard =
                                        blur_pipeline.as_ref().and_then(|b| b.lock().ok());
                                    crate::blur::render_snap_preview_overlay(
                                        &mut frame,
                                        blur_guard.as_deref_mut(),
                                        &projection,
                                        out_surface.size,
                                        preview,
                                        output_geo.loc,
                                        scale,
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
                                    let dock_radius = state.dock_cfg.dock_corner_radius();
                                    if let (Some(rect), Some(shadow)) =
                                        (dock_rect, shadow_pipeline.as_ref())
                                    {
                                        let dock_shadow_pad = (80.0 * scale).round() as i32;
                                        let dock_shadow_rect = Rectangle::new(
                                            (
                                                rect.loc.x - dock_shadow_pad,
                                                rect.loc.y - dock_shadow_pad,
                                            )
                                                .into(),
                                            (
                                                rect.size.w + dock_shadow_pad * 2,
                                                rect.size.h + dock_shadow_pad * 2,
                                            )
                                                .into(),
                                        );
                                        if damage.iter().any(|d| d.overlaps(dock_shadow_rect)) {
                                            let _ = frame.with_context(|gl| {
                                                shadow.render_dock_shadow(
                                                    gl,
                                                    &projection,
                                                    rect,
                                                    dock_radius * scale as f32,
                                                    scale,
                                                );
                                            });
                                        }
                                    }
                                    if let Some(rect) = dock_rect {
                                        let elem_geo =
                                            elem.geometry(smithay::utils::Scale::from(scale));
                                        if let Some(shader) = window_squircle_shader.as_ref() {
                                            shader.apply_wayland_element_to_frame(
                                                &mut frame,
                                                elem,
                                                elem_geo.size,
                                                rect.loc - elem_geo.loc,
                                                rect.size,
                                                (dock_radius as f64 * scale).round() as i32,
                                                0.0,
                                                true,
                                            );
                                        }
                                    }
                                    let _ = draw_render_elements(
                                        &mut frame,
                                        scale,
                                        std::slice::from_ref(elem),
                                        damage,
                                    );
                                    if dock_rect.is_some() {
                                        frame.clear_tex_program_override();
                                    }
                                }
                            }

                            for (popup_elems, popup_rect) in &dock_popups {
                                if !damage.iter().any(|d| d.overlaps(*popup_rect)) {
                                    continue;
                                }
                                if let Some(mut blur) =
                                    blur_pipeline.as_ref().and_then(|b| b.lock().ok())
                                {
                                    let blur_rect =
                                        super::dock_popover_blur_rect(*popup_rect, scale);
                                    let dock_popup_radius =
                                        super::dock_popover_corner_radius(blur_rect, scale);
                                    let _ = frame.with_context(|gl| {
                                        blur.capture_background_region(
                                            gl,
                                            &projection,
                                            out_surface.size,
                                            blur_rect,
                                        );
                                        blur.render_captured_blur(
                                            gl,
                                            &projection,
                                            out_surface.size,
                                            blur_rect,
                                            dock_popup_radius,
                                        );
                                    });
                                }

                                for elem in popup_elems.iter().rev() {
                                    if let Some(shader) = window_squircle_shader.as_ref() {
                                        let elem_geo =
                                            elem.geometry(smithay::utils::Scale::from(scale));
                                        shader.apply_clip_to_frame(
                                            &mut frame,
                                            elem_geo.size,
                                            popup_rect.loc - elem_geo.loc,
                                            popup_rect.size,
                                            (8.0 * scale).round() as i32,
                                        );
                                    }
                                    let _ = draw_render_elements(
                                        &mut frame,
                                        scale,
                                        std::slice::from_ref(elem),
                                        damage,
                                    );
                                    frame.clear_tex_program_override();
                                }
                            }

                            for (popup_elems, popup_rect) in &topbar_popups {
                                if !damage.iter().any(|d| d.overlaps(*popup_rect)) {
                                    continue;
                                }
                                let topbar_popup_r = super::topbar_popup_corner_radius(
                                    *popup_rect,
                                    out_surface.size,
                                    scale,
                                );
                                let topbar_popup_r_i32 = topbar_popup_r.round() as i32;
                                if let Some(mut blur) =
                                    blur_pipeline.as_ref().and_then(|b| b.lock().ok())
                                {
                                    let _ = frame.with_context(|gl| {
                                        blur.capture_background_region(
                                            gl,
                                            &projection,
                                            out_surface.size,
                                            *popup_rect,
                                        );
                                        blur.render_captured_blur(
                                            gl,
                                            &projection,
                                            out_surface.size,
                                            *popup_rect,
                                            topbar_popup_r,
                                        );
                                    });
                                }

                                if let Some(shadow) = shadow_pipeline.as_ref() {
                                    let _ = frame.with_context(|gl| {
                                        shadow.render_menu_shadow(
                                            gl,
                                            &projection,
                                            *popup_rect,
                                            topbar_popup_r,
                                            scale,
                                        );
                                    });
                                }

                                for elem in popup_elems.iter().rev() {
                                    if let Some(shader) = window_squircle_shader.as_ref() {
                                        let elem_geo =
                                            elem.geometry(smithay::utils::Scale::from(scale));
                                        shader.apply_to_frame(
                                            &mut frame,
                                            elem_geo.size,
                                            popup_rect.loc - elem_geo.loc,
                                            popup_rect.size,
                                            topbar_popup_r_i32,
                                        );
                                    }
                                    let _ = draw_render_elements(
                                        &mut frame,
                                        scale,
                                        std::slice::from_ref(elem),
                                        damage,
                                    );
                                    frame.clear_tex_program_override();
                                }
                            }

                            for (popup_elems, popup_rect) in &wallpaper_popups {
                                if !damage.iter().any(|d| d.overlaps(*popup_rect)) {
                                    continue;
                                }
                                if let Some(mut blur) =
                                    blur_pipeline.as_ref().and_then(|b| b.lock().ok())
                                {
                                    let _ = frame.with_context(|gl| {
                                        blur.capture_background_region(
                                            gl,
                                            &projection,
                                            out_surface.size,
                                            *popup_rect,
                                        );
                                        blur.render_captured_blur(
                                            gl,
                                            &projection,
                                            out_surface.size,
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
                                            scale,
                                        );
                                    });
                                }

                                for elem in popup_elems.iter().rev() {
                                    if let Some(shader) = window_squircle_shader.as_ref() {
                                        let elem_geo =
                                            elem.geometry(smithay::utils::Scale::from(scale));
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
                                        scale,
                                        std::slice::from_ref(elem),
                                        damage,
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
                                if let Some(mut blur) =
                                    blur_pipeline.as_ref().and_then(|b| b.lock().ok())
                                {
                                    let _ = frame.with_context(|gl| {
                                        blur.render_blur_masked(
                                            gl,
                                            &projection,
                                            out_surface.size,
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
                                    damage,
                                );
                            }
                            if let Some(ref drag_elem) = spotlight_drag_elem {
                                let _ = draw_render_elements(
                                    &mut frame,
                                    1.0,
                                    std::slice::from_ref(drag_elem),
                                    damage,
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
                                if let Some(mut blur) =
                                    blur_pipeline.as_ref().and_then(|b| b.lock().ok())
                                {
                                    let _ = frame.with_context(|gl| {
                                        blur.render_blur_masked(
                                            gl,
                                            &projection,
                                            out_surface.size,
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
                                    damage,
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
                                            (crate::snap_layouts::POPUP_CORNER_RADIUS * scale)
                                                as f32,
                                            scale,
                                            snap_alpha,
                                        );
                                    });
                                }
                                if let Some(mut blur) =
                                    blur_pipeline.as_ref().and_then(|b| b.lock().ok())
                                {
                                    let _ = frame.with_context(|gl| {
                                        blur.render_blur_masked(
                                            gl,
                                            &projection,
                                            out_surface.size,
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
                                    damage,
                                );
                            }

                            if let Some((
                                osd_rect,
                                ref mask_vec,
                                (mask_w, mask_h),
                                osd_alpha,
                                ref osd_elem,
                            )) = media_osd_frame
                            {
                                if let Some(shadow) = shadow_pipeline.as_ref() {
                                    let _ = frame.with_context(|gl| {
                                        shadow.render_menu_shadow_alpha(
                                            gl,
                                            &projection,
                                            osd_rect,
                                            (crate::media_keys::OSD_CORNER_RADIUS * scale) as f32,
                                            scale,
                                            osd_alpha,
                                        );
                                    });
                                }
                                if let Some(mut blur) =
                                    blur_pipeline.as_ref().and_then(|b| b.lock().ok())
                                {
                                    let _ = frame.with_context(|gl| {
                                        blur.render_blur_masked(
                                            gl,
                                            &projection,
                                            out_surface.size,
                                            osd_rect,
                                            mask_vec.as_deref(),
                                            mask_w,
                                            mask_h,
                                            osd_alpha,
                                        );
                                    });
                                }
                                let _ = draw_render_elements(
                                    &mut frame,
                                    1.0,
                                    std::slice::from_ref(osd_elem),
                                    damage,
                                );
                            }

                            // Selector Visual App Switcher estilo Deck (sin blur global en el fondo)
                            if let Some((deck_eased, ref deck_cards)) = app_switcher_deck_frame {
                                let mut blur_guard =
                                    blur_pipeline.as_ref().and_then(|b| b.lock().ok());
                                crate::app_switcher::render_deck_frame(
                                    &mut frame,
                                    &projection,
                                    out_surface.size,
                                    scale,
                                    damage,
                                    blur_guard.as_deref_mut(),
                                    shadow_pipeline.as_deref(),
                                    window_squircle_shader.as_deref(),
                                    deck_eased,
                                    deck_cards,
                                );
                            }

                            // 🧞 EFECTO LÁMPARA DE ALADINO (GENIE EFFECT / MAGIC LAMP)
                            // Efecto Genie: malla GPU única sobre snapshot offscreen (incluyendo SSD y squircle).
                            if !active_genies.is_empty() {
                                if let Some(ref gp) = genie_pipeline {
                                    let _ = frame.with_context(|gl| {
                                    for anim in &active_genies {
                                        let Some(snap) = genie_snapshots.get(&anim.window_id)
                                        else {
                                            continue;
                                        };
                                        let elapsed = now.duration_since(anim.start_time);
                                        let raw_t = (elapsed.as_secs_f32()
                                            / anim.duration.as_secs_f32())
                                        .clamp(0.0, 1.0);
                                        let win_rect = crate::genie::GenieRect {
                                            x: ((snap.win_rect_logical.loc.x - output_geo.loc.x)
                                                as f64
                                                * scale)
                                                as f32,
                                            y: ((snap.win_rect_logical.loc.y - output_geo.loc.y)
                                                as f64
                                                * scale)
                                                as f32,
                                            width: ((snap.win_rect_logical.size.w as f64 * scale)
                                                as f32)
                                                .max(40.0),
                                            height: ((snap.win_rect_logical.size.h as f64 * scale)
                                                as f32)
                                                .max(40.0),
                                        };
                                        let target = crate::genie::GenieRect {
                                            x: ((anim.dock_slot_rect.loc.x - output_geo.loc.x)
                                                as f64
                                                * scale)
                                                as f32,
                                            y: ((anim.dock_slot_rect.loc.y - output_geo.loc.y)
                                                as f64
                                                * scale)
                                                as f32,
                                            width: (anim.dock_slot_rect.size.w as f64 * scale)
                                                as f32,
                                            height: (anim.dock_slot_rect.size.h as f64 * scale)
                                                as f32,
                                        };
                                        let mf = crate::magic_lamp::compute_minimize_frame(
                                            win_rect,
                                            target,
                                            raw_t,
                                            anim.is_minimizing,
                                            state.dock_cfg.genie_effect,
                                            state.dock_cfg.position,
                                        );
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
                                                mf.win_rect,
                                                r as f32,
                                                scale,
                                                mf.shadow_k,
                                                mf.j,
                                                is_fullscreen_or_max,
                                            );
                                        }
                                        gp.render(
                                            gl,
                                            &projection,
                                            &snap.texture,
                                            mf.win_rect,
                                            mf.target_rect,
                                            mf.direction,
                                            mf.k,
                                            mf.j,
                                            mf.alpha,
                                        );
                                    }
                                });
                                }
                            }

                            // 🕳️ EFECTO AGUJERO NEGRO AL CERRAR VENTANAS
                            if black_hole_active_on_output {
                                if let Some(ref bh_pipeline) = black_hole_pipeline {
                                    let out_name = out_surface.output.name();
                                    let bh_now = Instant::now();
                                    let mut blur_guard =
                                        blur_pipeline.as_ref().and_then(|b| b.lock().ok());
                                    let _ = frame.with_context(|gl| {
                                        for bh in &active_black_holes {
                                            if bh.output_name == out_name {
                                                if let Some(glass) = bh.live_backdrop {
                                                    let alpha = bh.live_backdrop_alpha(bh_now);
                                                    if alpha > 0.002 {
                                                        if let Some(ref mut blur) = blur_guard {
                                                            blur.capture_background_region(
                                                                gl,
                                                                &projection,
                                                                out_surface.size,
                                                                glass.rect,
                                                            );
                                                            if let Some(shadow) =
                                                                shadow_pipeline.as_ref()
                                                            {
                                                                shadow.render_menu_shadow_alpha(
                                                                    gl,
                                                                    &projection,
                                                                    glass.rect,
                                                                    glass.radius,
                                                                    scale,
                                                                    alpha,
                                                                );
                                                            }
                                                            blur.render_captured_blur_alpha(
                                                                gl,
                                                                &projection,
                                                                out_surface.size,
                                                                glass.rect,
                                                                glass.radius,
                                                                alpha,
                                                            );
                                                        }
                                                    }
                                                }
                                                bh_pipeline.render_effect(
                                                    gl,
                                                    &projection,
                                                    bh,
                                                    bh_now,
                                                );
                                            }
                                        }
                                    });
                                }
                            }

                            // Dibujar el icono de Drag and Drop (dnd_icon) si hay una operación activa
                            if !dnd_elems.is_empty() {
                                let _ = draw_render_elements(&mut frame, scale, &dnd_elems, damage);
                            } else if let Some(ref fallback_elem) = dnd_fallback_elem {
                                let fallback_slice = std::slice::from_ref(fallback_elem);
                                let _ =
                                    draw_render_elements(&mut frame, 1.0, fallback_slice, damage);
                            }
                        }

                        // Dibujar el puntero con el tamaño lógico configurado y el hotspot exacto
                        if let Some((phys_x, phys_y)) = cursor_draw_point {
                            let mut rendered_surface_cursor = false;
                            if !cursor_elems.is_empty() {
                                for elem in &cursor_elems {
                                    let view_dst = elem.view().dst;
                                    if view_dst.w > 0 && view_dst.h > 0 {
                                        rendered_surface_cursor = true;
                                        let target_logical_w = effective_cursor_size;
                                        let target_logical_h = target_logical_w
                                            * (view_dst.h as f64 / view_dst.w as f64);
                                        let ratio_x = target_logical_w / view_dst.w as f64;
                                        let ratio_y = target_logical_h / view_dst.h as f64;

                                        let hot_phys_x = (cursor_hotspot.x as f64 * ratio_x * scale)
                                            .round()
                                            as i32;
                                        let hot_phys_y = (cursor_hotspot.y as f64 * ratio_y * scale)
                                            .round()
                                            as i32;
                                        let cursor_phys_w =
                                            (target_logical_w * scale).round() as i32;
                                        let cursor_phys_h =
                                            (target_logical_h * scale).round() as i32;

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
                            }
                            if !rendered_surface_cursor {
                                let active_cursor = match state.cursor_status {
                                    CursorImageStatus::Named(icon)
                                        if icon != smithay::input::pointer::CursorIcon::Default =>
                                    {
                                        named_cursor_cache
                                            .get(&icon)
                                            .and_then(|c| c.as_ref())
                                            .or(cursor_texture.as_ref())
                                    }
                                    _ => cursor_texture.as_ref(),
                                };
                                if let Some(cur) = active_cursor {
                                    let scale_factor =
                                        (effective_cursor_size * scale) / cur.size as f64;
                                    let cursor_w = (cur.width as f64 * scale_factor).round() as i32;
                                    let cursor_h =
                                        (cur.height as f64 * scale_factor).round() as i32;
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

                        if !theme_transition_init_attempted {
                            theme_transition_init_attempted = true;
                            theme_transition_pipeline = frame
                                .with_context(crate::theme_transition::ThemeTransitionPipeline::new)
                                .ok()
                                .flatten();
                        }
                        if let Some(pipeline) = theme_transition_pipeline.as_mut() {
                            let output_name = out_surface.output.name();
                            let origin = theme_capture_request.as_ref().map(|request| {
                                crate::theme_transition::output_origin(
                                    (request.origin.x, request.origin.y),
                                    (output_geo.loc.x, output_geo.loc.y),
                                    scale,
                                    out_surface.size,
                                )
                            });
                            let result = frame.with_context(|gl| {
                                if let Some(origin) = origin {
                                    if pipeline.capture_snapshot(
                                        gl,
                                        &output_name,
                                        out_surface.size,
                                        true,
                                        origin,
                                    ) {
                                        theme_capture_outputs += 1;
                                    } else {
                                        theme_capture_succeeded = false;
                                    }
                                }
                                if let Some(started_at) = state.theme_transition_started_at {
                                    let now = Instant::now();
                                    let drawn = pipeline.draw(
                                        gl,
                                        &output_name,
                                        out_surface.size,
                                        started_at,
                                        now,
                                    );
                                    if !drawn
                                        && now.duration_since(started_at)
                                            >= crate::theme_transition::DURATION
                                    {
                                        state.theme_transition_started_at = None;
                                    }
                                }
                            });
                            if let Err(error) = result {
                                warn!("No se pudo dibujar la transición de tema: {error:?}");
                                if theme_capture_request.is_some() {
                                    theme_capture_succeeded = false;
                                }
                            }
                        } else if theme_capture_request.is_some() {
                            theme_capture_succeeded = false;
                        }

                        if !workspace_transition_init_attempted {
                            workspace_transition_init_attempted = true;
                            workspace_transition_pipeline = frame
                                .with_context(
                                    crate::workspace_transition::WorkspaceTransitionPipeline::new,
                                )
                                .ok()
                                .flatten();
                        }
                        if let Some(ws_pipeline) = workspace_transition_pipeline.as_mut() {
                            let output_name = out_surface.output.name();
                            let ws_result = frame.with_context(|gl| {
                                if let Some(mut ws_transition) = state.workspace_transition {
                                    ws_transition_attempted = true;
                                    let now = Instant::now();
                                    let drawn = ws_pipeline.draw(
                                        gl,
                                        &output_name,
                                        out_surface.size,
                                        true,
                                        &mut ws_transition,
                                        now,
                                    );
                                    if drawn {
                                        ws_transition_drawn = true;
                                        state.workspace_transition = Some(ws_transition);
                                    } else if ws_transition.is_finished(now) {
                                        ws_transition_finished = true;
                                    }
                                }
                                if workspace_switch_pending
                                    && ws_pipeline.capture_outgoing(
                                        gl,
                                        &output_name,
                                        out_surface.size,
                                        true,
                                    )
                                {
                                    ws_capture_outputs += 1;
                                }
                            });
                            if let Err(error) = ws_result {
                                warn!("No se pudo dibujar la transición de escritorio: {error:?}");
                            }
                        }

                        let output_name = out_surface.output.name();
                        if !out_surface.hw_gamma_brightness_applied
                            && !malus_de_common::config::connector_has_hardware_backlight(
                                &output_name,
                            )
                        {
                            let out_brightness = current_displays_cfg
                                .find_for_connector(&output_name)
                                .map(|o| o.brightness)
                                .unwrap_or(current_displays_cfg.brightness)
                                .clamp(5.0, 100.0);
                            if out_brightness < 99.9 {
                                let dim_alpha =
                                    (((100.0 - out_brightness) / 100.0) * 0.85).clamp(0.0, 0.85)
                                        as f32;
                                if dim_alpha > 0.002 {
                                    let dim_bounds = Rectangle::from_size(out_surface.size);
                                    let _ = frame.draw_solid(
                                        dim_bounds,
                                        damage,
                                        Color32F::new(0.0, 0.0, 0.0, dim_alpha),
                                    );
                                }
                            }
                        }

                        let _ = frame.finish();
                    }
                    if let Some(destination) = screenshot_request {
                        info!("Procesando solicitud de captura de pantalla DRM");
                        if frame_rendered {
                            if let Err(error) = crate::capture::capture_frame(
                                &mut renderer,
                                &target,
                                out_surface.size,
                                &destination,
                                Transform::Normal,
                            ) {
                                warn!("Error al capturar el framebuffer DRM: {error:#}");
                                crate::capture::write_capture_error(&destination, &error);
                            }
                        } else {
                            let error = anyhow::anyhow!(
                                "No se pudo renderizar el fotograma de captura DRM"
                            );
                            crate::capture::write_capture_error(&destination, &error);
                        }
                    }

                    if !session_locked {
                        let out_name_rec = out_surface.output.name();
                        if let Some(ref mut session) = state.active_screen_recording {
                            if session.matches_output(&out_name_rec) && session.frame_due(now) {
                                let crop = session.rect.unwrap_or_else(|| {
                                    Rectangle::from_size(
                                        (session.frame_width as i32, session.frame_height as i32)
                                            .into(),
                                    )
                                });
                                let cl_x = crop.loc.x.clamp(0, (out_surface.size.w - 2).max(0));
                                let cl_y = crop.loc.y.clamp(0, (out_surface.size.h - 2).max(0));
                                let cl_w = (session.frame_width as i32)
                                    .min(out_surface.size.w - cl_x)
                                    .max(2);
                                let cl_h = (session.frame_height as i32)
                                    .min(out_surface.size.h - cl_y)
                                    .max(2);
                                let region =
                                    Rectangle::new((cl_x, cl_y).into(), (cl_w, cl_h).into());

                                session.capture_frame_async(&mut renderer, &target, region, now);
                            }
                        }
                    }
                }

                if let Err(e) = out_surface.surface.queue_buffer(None, None, ()) {
                    warn!("Error al encolar buffer en DRM: {:?}", e);
                    let _ = out_surface.surface.frame_submitted();
                    out_surface.flip_pending = None;
                    out_surface.pending_damage_buffers = out_surface.pending_damage_buffers.max(1);
                } else if frame_rendered {
                    out_surface.flip_pending = Some(Instant::now());
                    out_surface.last_commit_counter = state.surface_commit_counter;
                    out_surface.last_output_commit_counter = current_output_commit_counter;
                    out_surface.last_cursor_draw_point = cursor_draw_point;
                    out_surface.last_cursor_status = state.cursor_status.clone();
                    out_surface.last_cursor_scale_milli = cursor_scale_milli;
                    out_surface.last_expose_revision = state.expose.render_revision;
                    out_surface.last_workspace_revision = state.workspace_revision;
                    if out_surface.pending_damage_buffers > 0 {
                        out_surface.pending_damage_buffers -= 1;
                    }
                    if out_surface.full_damage_buffers > 0 {
                        out_surface.full_damage_buffers -= 1;
                    }

                    if let Some(new_rect) = current_cursor_rect {
                        out_surface.cursor_history.push_front(new_rect);
                    } else if !out_surface.cursor_history.is_empty() {
                        out_surface.cursor_history.push_front(Rectangle::default());
                    }
                    if out_surface.cursor_history.len() > 4 {
                        out_surface.cursor_history.truncate(4);
                    }

                    out_surface
                        .extra_damage_history
                        .push_front(this_frame_extra_damage);
                    if out_surface.extra_damage_history.len() > 4 {
                        out_surface.extra_damage_history.truncate(4);
                    }

                    if out_surface.dock_redraw_buffers > 0 {
                        out_surface.dock_redraw_buffers -= 1;
                    }

                    for count in out_surface.window_redraw_shadow_blur.values_mut() {
                        if *count > 0 {
                            *count -= 1;
                        }
                    }
                    out_surface
                        .window_redraw_shadow_blur
                        .retain(|_, count| *count > 0);

                    rendered_outputs.insert(out_surface.output.clone());

                    out_surface.last_window_geometries.clear();
                    for (root_id, _, _, _, _, is_active, win_rect, ..) in &window_render_data {
                        out_surface
                            .last_window_geometries
                            .insert(root_id.clone(), (*win_rect, *is_active));
                    }
                    out_surface.last_window_logical_geoms.clear();
                    let kb_focus = state.seat.get_keyboard().and_then(|k| k.current_focus());
                    for window in state.space.elements() {
                        if let (Some(loc), Some(main_surface)) =
                            (state.space.element_location(window), window.wl_surface())
                        {
                            let win_rect = Rectangle::new(loc, window.geometry().size);
                            if output_geo.overlaps(win_rect) {
                                let root_id =
                                    smithay::backend::renderer::element::Id::from_wayland_resource(
                                        main_surface.as_ref(),
                                    );
                                let is_active =
                                    state.is_window_visually_active(window, kb_focus.as_ref());
                                out_surface
                                    .last_window_logical_geoms
                                    .insert(root_id, (win_rect, is_active));
                            }
                        }
                    }
                }
            }
            if let Some(request) = theme_capture_request {
                state.pending_theme_transition_capture = None;
                if theme_capture_succeeded && theme_capture_outputs == expected_theme_outputs {
                    state
                        .ipc_broadcaster
                        .broadcast(&IpcMessage::ThemeTransitionPrepared {
                            request_id: request.request_id,
                        });
                } else {
                    warn!("No se guardaron todos los fotogramas DRM para la transición de tema");
                }
            }
            if ws_transition_finished || (ws_transition_attempted && !ws_transition_drawn) {
                let full_bufs = if ENABLE_DAMAGE_TRACKING { 3 } else { 1 };
                state.workspace_transition = None;
                for s in surfs.iter_mut() {
                    s.full_damage_buffers = s.full_damage_buffers.max(full_bufs);
                }
            }
            if workspace_switch_pending && (ws_capture_outputs > 0 || surfs.is_empty()) {
                let full_bufs = if ENABLE_DAMAGE_TRACKING { 3 } else { 1 };
                state.commit_pending_workspace_switch(ws_capture_outputs > 0);
                for s in surfs.iter_mut() {
                    s.full_damage_buffers = s.full_damage_buffers.max(full_bufs);
                }
            }
        }
        state.pending_black_holes.clear();

        // Completar animaciones Genie (minimizar / restaurar) tras renderizar el frame
        let now = Instant::now();
        active_black_holes.retain(|bh| !bh.is_finished(now));
        let mut finished_anims = Vec::new();
        state.active_animations.retain(|anim| {
            if anim.waiting_for_dock_slot {
                return true;
            }
            if now.duration_since(anim.start_time) >= anim.duration {
                finished_anims.push(anim.clone());
                false
            } else {
                true
            }
        });

        let active_genie_ids: std::collections::HashSet<u64> = state
            .active_animations
            .iter()
            .map(|anim| anim.window_id)
            .collect();
        genie_snapshots.retain(|id, _| active_genie_ids.contains(id));

        for anim in finished_anims {
            if !anim.is_minimizing {
                info!("✨ Restauración Genie completada en DRM: mapeando ventana a Space");
                state
                    .space
                    .map_element(anim.window.clone(), anim.original_location, true);
                state.space.raise_element(&anim.window, true);
                let serial = SERIAL_COUNTER.next_serial();
                state.set_keyboard_focus_with_sync(Some(anim.surface.clone()), serial);
                state.notify_active_app(Some(&anim.surface));
                state.notify_running_apps();
                if state.auto_tiling_enabled {
                    state.relayout_auto_tiling();
                }
            } else {
                state.notify_minimized_windows();
            }
        }

        // Limpiar transiciones de maximizado/restauración finalizadas
        state
            .snap_transitions
            .retain(|_, trans| !trans.is_finished(now));

        // Notificar fotogramas a los clientes de ventanas activas (evita despertar apps minimizadas u ocluidas a 60 Hz)
        let ts = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        let now_ms = (ts.tv_sec as u64 * 1000 + (ts.tv_nsec as u64 / 1_000_000)) as u32;
        let pres_time =
            std::time::Duration::from_nanos(ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64);

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

                if let Some(output) = state.space.outputs_for_element(window).first() {
                    if rendered_outputs.contains(output) {
                        let refresh_nanos = output
                            .current_mode()
                            .map(|m| 1_000_000_000_000 / m.refresh.max(1) as u64)
                            .unwrap_or(16_666_666);
                        let refresh = smithay::wayland::presentation::Refresh::fixed(
                            std::time::Duration::from_nanos(refresh_nanos),
                        );
                        send_presentation_feedback_surface_tree(
                            surface.as_ref(),
                            output,
                            pres_time,
                            refresh,
                            0,
                        );
                        for (popup, _) in
                            smithay::desktop::PopupManager::popups_for_surface(surface.as_ref())
                        {
                            send_presentation_feedback_surface_tree(
                                popup.wl_surface(),
                                output,
                                pres_time,
                                refresh,
                                0,
                            );
                        }
                    }
                }
            }
        }

        for output in state.space.outputs() {
            let output_rendered = rendered_outputs.contains(output);
            if !output_rendered {
                continue;
            }
            let has_fullscreen = state.has_fullscreen_window_on_output(output);
            let refresh_nanos = output
                .current_mode()
                .map(|m| 1_000_000_000_000 / m.refresh.max(1) as u64)
                .unwrap_or(16_666_666);
            let refresh = smithay::wayland::presentation::Refresh::fixed(
                std::time::Duration::from_nanos(refresh_nanos),
            );
            let layer_map = smithay::desktop::layer_map_for_output(output);
            for layer in layer_map.layers() {
                if has_fullscreen
                    && !matches!(
                        layer.layer(),
                        smithay::wayland::shell::wlr_layer::Layer::Overlay
                    )
                {
                    continue;
                }
                send_frames_surface_tree(layer.wl_surface(), now_ms);
                if output_rendered {
                    send_presentation_feedback_surface_tree(
                        layer.wl_surface(),
                        output,
                        pres_time,
                        refresh,
                        0,
                    );
                }
                for (popup, _) in
                    smithay::desktop::PopupManager::popups_for_surface(layer.wl_surface())
                {
                    send_frames_surface_tree(popup.wl_surface(), now_ms);
                    if output_rendered {
                        send_presentation_feedback_surface_tree(
                            popup.wl_surface(),
                            output,
                            pres_time,
                            refresh,
                            0,
                        );
                    }
                }
            }
            if let Some(ext_ls) = state.authorized_ext_lock_surface_for_output(&output.name()) {
                send_frames_surface_tree(ext_ls.wl_surface(), now_ms);
                if output_rendered {
                    send_presentation_feedback_surface_tree(
                        ext_ls.wl_surface(),
                        output,
                        pres_time,
                        refresh,
                        0,
                    );
                }
            }
        }

        display.flush_clients()?;
    }

    info!("Compositor Malus DE DRM finalizado limpiamente.");
    Ok(())
}

#[cfg(test)]
mod drm_emergency_tests {
    use super::{parse_evdev_key_event, sysfs_key_caps_has_backspace, EmergencyZapKeys};

    #[test]
    fn sysfs_key_caps_identifies_keyboards_with_backspace() {
        assert!(sysfs_key_caps_has_backspace("10000 7 fffffffffffffffe"));
        assert!(!sysfs_key_caps_has_backspace("10000002"));
        assert!(!sysfs_key_caps_has_backspace("0"));
    }

    #[test]
    fn evdev_zap_triggers_only_on_ctrl_alt_backspace_press() {
        let mut keys = EmergencyZapKeys::default();
        assert!(!keys.update(29, 1)); // LeftCtrl down
        assert!(!keys.update(14, 1)); // Backspace without Alt
        assert!(!keys.update(56, 1)); // LeftAlt down
        assert!(keys.update(14, 1)); // Ctrl+Alt+Backspace press -> ZAP
        assert!(!keys.update(14, 2)); // key repeat ignored
        assert!(!keys.update(56, 0)); // LeftAlt released
        assert!(!keys.update(14, 1));
    }

    #[test]
    fn parse_evdev_key_event_extracts_type_code_and_value() {
        #[cfg(target_pointer_width = "64")]
        let mut buf = [0u8; 24];
        #[cfg(not(target_pointer_width = "64"))]
        let mut buf = [0u8; 16];
        let off = buf.len() - 8;
        buf[off..off + 2].copy_from_slice(&1u16.to_ne_bytes()); // EV_KEY
        buf[off + 2..off + 4].copy_from_slice(&14u16.to_ne_bytes()); // KEY_BACKSPACE
        buf[off + 4..off + 8].copy_from_slice(&1i32.to_ne_bytes());
        assert_eq!(parse_evdev_key_event(&buf), Some((14, 1)));
    }

    #[test]
    fn prioritize_drm_gpu_candidates_prefers_real_kms_with_connected_outputs() {
        use std::path::PathBuf;
        let ordered = super::prioritize_drm_gpu_candidates(
            Some(PathBuf::from("/dev/dri/card0")),
            vec![
                PathBuf::from("/dev/dri/card0"),
                PathBuf::from("/dev/dri/card1"),
            ],
            vec![
                PathBuf::from("/dev/dri/card0"),
                PathBuf::from("/dev/dri/card1"),
            ],
            |p| match p.to_str() {
                // Caso laptop con panel interno roto/desconectado en card0 y monitor externo en card1
                Some("/dev/dri/card0") => (false, true),
                Some("/dev/dri/card1") => (true, true),
                _ => (false, false),
            },
        );
        assert_eq!(
            ordered,
            vec![
                PathBuf::from("/dev/dri/card1"),
                PathBuf::from("/dev/dri/card0")
            ]
        );
    }

    #[test]
    fn select_hotplug_primary_index_keeps_surviving_primary_over_new_connector() {
        use malus_de_common::config::DisplayOutputConfig;

        let make_out = |connector: &str, primary: bool| DisplayOutputConfig {
            connector: connector.to_string(),
            name: connector.to_string(),
            width: 1920,
            height: 1080,
            refresh_rate: 60.0,
            modes: vec![],
            x: 0,
            y: 0,
            scale: 1.0,
            primary,
            brightness: 100.0,
            diagonal_inches: Some(24.0),
        };
        // Simula el caso en que el conector nuevo (`DP-1`) aparece antes que el activo (`eDP-1`)
        // e incluso traía `primary = true` en un `monitors.toml` previo.
        let outputs = vec![make_out("DP-1", true), make_out("eDP-1", true)];
        assert_eq!(
            super::select_hotplug_primary_index(&outputs, Some("eDP-1")),
            1
        );
        assert_eq!(super::select_hotplug_primary_index(&outputs, None), 0);
    }
}
