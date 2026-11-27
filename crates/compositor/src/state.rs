use crate::cpu_priority::FocusCpuPriority;
use malus_de_common::config::{AppSwitcherStyle, DockConfig};
use smithay::{
    backend::allocator::dmabuf::Dmabuf,
    backend::renderer::element::memory::MemoryRenderBuffer,
    delegate_compositor, delegate_data_control, delegate_data_device, delegate_dmabuf,
    delegate_fractional_scale, delegate_layer_shell, delegate_output, delegate_presentation,
    delegate_primary_selection, delegate_seat, delegate_session_lock, delegate_shm,
    delegate_viewporter, delegate_xdg_decoration, delegate_xdg_shell,
    desktop::{
        find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output, PopupGrab,
        PopupKeyboardGrab, PopupKind, PopupManager, PopupPointerGrab, Space,
    },
    input::{Seat, SeatHandler, SeatState},
    output::Output,
    reexports::{
        wayland_protocols::{
            ext::session_lock::v1::server::ext_session_lock_v1::ExtSessionLockV1,
            xdg::shell::server::xdg_toplevel::{ResizeEdge, State as XdgState},
        },
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{
                wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface,
            },
            Client, Display, DisplayHandle, Resource,
        },
    },
    utils::{IsAlive, Logical, Point, Rectangle, Serial, Size, SERIAL_COUNTER},
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        fractional_scale::{
            with_fractional_scale, FractionalScaleHandler, FractionalScaleManagerState,
        },
        output::{OutputHandler, OutputManagerState},
        seat::WaylandFocus,
        selection::{
            data_device::{
                set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
                ServerDndGrabHandler,
            },
            primary_selection::{
                set_primary_focus, PrimarySelectionHandler, PrimarySelectionState,
            },
            wlr_data_control::{DataControlHandler, DataControlState},
            SelectionHandler,
        },
        session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker},
        shell::{
            wlr_layer::{Layer, LayerSurface, WlrLayerShellHandler, WlrLayerShellState},
            xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState},
        },
        shm::{ShmHandler, ShmState},
        viewporter::ViewporterState,
    },
};
use std::collections::{HashMap, HashSet};
use std::os::unix::io::OwnedFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use malus_de_common::ipc::{AppSwitcherItem, IpcBroadcaster, IpcMessage};

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

/// Marcador en `SurfaceData::data_map` para evitar un error de protocolo falso (`InvalidSize`)
/// en Smithay 0.7.0 cuando GTK4/gtk4-layer-shell realiza `wl_surface.attach(null) + commit()`
/// inmediatamente después de destruir `zwlr_layer_surface_v1`.
struct LayerRoleDestroyedMarker(AtomicBool);

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
        if let DisconnectReason::ProtocolError(err) = reason {
            tracing::warn!(
                "⚠️ Cliente Wayland {:?} desconectado por error de protocolo: {}",
                client_id,
                err
            );
        }
    }
}

#[derive(Debug, Clone)]
pub struct GenieAnimation {
    pub window_id: u64,
    pub surface: WlSurface,
    pub window: smithay::desktop::Window,
    pub start_time: std::time::Instant,
    pub duration: std::time::Duration,
    pub is_minimizing: bool,
    pub waiting_for_dock_slot: bool,
    pub window_rect: Rectangle<i32, Logical>,
    pub dock_slot_rect: Rectangle<i32, Logical>,
    pub original_location: Point<i32, Logical>,
}

pub const SNAP_TRANSITION_DURATION: Duration = Duration::from_millis(220);

/// Animación suave de interpolación geométrica para transiciones de maximizar, restaurar y Aero Snap
#[derive(Debug, Clone)]
pub struct SnapTransition {
    pub start_time: Instant,
    pub duration: Duration,
    pub start_loc: Point<i32, Logical>,
    pub start_size: Size<i32, Logical>,
    pub target_loc: Point<i32, Logical>,
    pub target_size: Size<i32, Logical>,
}

impl SnapTransition {
    pub fn new(
        start_loc: Point<i32, Logical>,
        start_size: Size<i32, Logical>,
        target_loc: Point<i32, Logical>,
        target_size: Size<i32, Logical>,
        duration: Duration,
    ) -> Self {
        Self {
            start_time: Instant::now(),
            duration,
            start_loc,
            start_size: Size::from((start_size.w.max(1), start_size.h.max(1))),
            target_loc,
            target_size: Size::from((target_size.w.max(1), target_size.h.max(1))),
        }
    }

    /// Calcula la posición y tamaño interpolados usando aceleración ease-out cúbica estilo macOS
    pub fn current_geometry(&self, now: Instant) -> (Point<i32, Logical>, Size<i32, Logical>) {
        let elapsed = now.saturating_duration_since(self.start_time);
        let progress = (elapsed.as_secs_f64() / self.duration.as_secs_f64()).clamp(0.0, 1.0);

        // Curva cúbica ease-out: despegue reactivo con frenado suave y elegante al aterrizar
        let p = 1.0 - (1.0 - progress).powi(3);

        let cur_x = (self.start_loc.x as f64 + (self.target_loc.x - self.start_loc.x) as f64 * p)
            .round() as i32;
        let cur_y = (self.start_loc.y as f64 + (self.target_loc.y - self.start_loc.y) as f64 * p)
            .round() as i32;
        let cur_w = (self.start_size.w as f64 + (self.target_size.w - self.start_size.w) as f64 * p)
            .round() as i32;
        let cur_h = (self.start_size.h as f64 + (self.target_size.h - self.start_size.h) as f64 * p)
            .round() as i32;

        (
            Point::from((cur_x, cur_y)),
            Size::from((cur_w.max(1), cur_h.max(1))),
        )
    }

    pub fn is_finished(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start_time) >= self.duration
    }

    /// Calcula la geometría física manteniendo estrictamente escala **1:1 estática** durante toda
    /// la transición (sin ampliar el buffer ni deformar los botones del semáforo CSD) junto con el
    /// rectángulo de recorte `surface_clip_rect` dentro del área animada de la ventana.
    pub fn render_geometry(
        &self,
        window_geo: Rectangle<i32, Logical>,
        output_origin: Point<i32, Logical>,
        output_scale: f64,
        now: Instant,
    ) -> (
        Point<i32, Logical>,
        Size<i32, Logical>,
        Point<i32, smithay::utils::Physical>,
        Option<Rectangle<i32, smithay::utils::Physical>>,
        Point<i32, smithay::utils::Physical>,
        Size<i32, smithay::utils::Physical>,
        Rectangle<i32, smithay::utils::Physical>,
    ) {
        let (effective_loc, effective_size) = self.current_geometry(now);
        let surface_origin_x = effective_loc.x - window_geo.loc.x;
        let surface_origin_y = effective_loc.y - window_geo.loc.y;
        let phys_loc = Point::from((
            ((surface_origin_x - output_origin.x) as f64 * output_scale).round() as i32,
            ((surface_origin_y - output_origin.y) as f64 * output_scale).round() as i32,
        ));
        let content_offset = window_geo.loc.to_physical_precise_round(output_scale);
        let content_size = effective_size.to_physical_precise_round(output_scale);
        let win_screen_rect = Rectangle::new(
            Point::from((
                ((effective_loc.x - output_origin.x) as f64 * output_scale).round() as i32,
                ((effective_loc.y - output_origin.y) as f64 * output_scale).round() as i32,
            )),
            content_size,
        );
        let surface_screen_rect = Rectangle::new(
            win_screen_rect.loc,
            window_geo.size.to_physical_precise_round(output_scale),
        );
        let surface_clip_rect = win_screen_rect
            .intersection(surface_screen_rect)
            .unwrap_or_else(|| Rectangle::new(win_screen_rect.loc, Size::from((0, 0))));
        (
            effective_loc,
            effective_size,
            phys_loc,
            Some(surface_clip_rect),
            content_offset,
            content_size,
            win_screen_rect,
        )
    }
}

/// Estado en memoria del selector de aplicaciones visual (⌘Tab / App Switcher HUD)
#[derive(Debug, Default)]
pub struct AppSwitcherState {
    pub is_active: bool,
    pub items: Vec<AppSwitcherItem>,
    pub selected_index: usize,
    pub deck: crate::app_switcher::AppSwitcherDeckState,
    pub closed_dirty: bool,
}

#[derive(Debug, Clone)]
pub struct ThemeTransitionCapture {
    pub request_id: String,
    pub origin: Point<f64, Logical>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SwipeGestureKind {
    #[default]
    Undecided,
    ExposeVertical,
    WorkspaceHorizontal,
}

#[derive(Debug, Clone, Default)]
pub struct SwipeGestureState {
    pub active: bool,
    pub fingers: u32,
    pub accum_x: f64,
    pub accum_y: f64,
    pub last_dx: f64,
    pub last_dy: f64,
    pub kind: SwipeGestureKind,
    pub was_open_at_start: bool,
    pub app_expose_gesture: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PinchGestureState {
    pub active: bool,
    pub fingers: u32,
    pub last_scale: f64,
    pub triggered: bool,
}

pub struct CachedDndIcon {
    pub buffer: MemoryRenderBuffer,
    pub rel_offset_logical: (f64, f64),
}

pub struct MalusState {
    pub display_handle: DisplayHandle,
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: smithay::wayland::shell::xdg::decoration::XdgDecorationState,
    pub layer_shell_state: WlrLayerShellState,
    pub session_lock_state: SessionLockManagerState,
    pub ext_session_locked: bool,
    pub ext_lock_handle: Option<ExtSessionLockV1>,
    pub ext_lock_client_pid: Option<i32>,
    pub ext_lock_surfaces: HashMap<String, LockSurface>,
    pub malus_lock_layers: Vec<(Output, smithay::desktop::LayerSurface)>,
    pub lockscreen_respawn_at: Option<Instant>,
    pub session_locked_flag: Arc<AtomicBool>,
    pub fractional_scale_manager_state: FractionalScaleManagerState,
    pub viewporter_state: ViewporterState,
    pub presentation_state: smithay::wayland::presentation::PresentationState,
    pub shm_state: ShmState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub pending_dmabuf_imports: Vec<(Dmabuf, ImportNotifier)>,
    pub seat_state: SeatState<MalusState>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub data_control_state: DataControlState,
    pub output_manager_state: OutputManagerState,
    pub seat: Seat<MalusState>,
    pub focus_cpu_priority: FocusCpuPriority,
    pub space: Space<smithay::desktop::Window>,
    pub popups: PopupManager,
    pub active_popups: Vec<PopupSurface>,
    /// Grab explícito (`xdg_popup.grab`) activo de un popup de ventana de aplicación.
    pub popup_grab: Option<PopupGrab<MalusState>>,
    /// Evita enviar `xdg_toplevel.configure` de activación durante `ButtonState::Pressed` para no cancelar gestos en GTK4.
    pub defer_activation_configure: bool,
    /// Superficies cuyo `xdg_toplevel.configure` de activación quedó diferido hasta `ButtonState::Released`.
    pub pending_activation_configures: Vec<WlSurface>,
    pub window_restore_geometries: HashMap<WlSurface, (Point<i32, Logical>, Size<i32, Logical>)>,
    pub centered_transient_windows: HashMap<WlSurface, (Point<i32, Logical>, Size<i32, Logical>)>,
    /// Ventanas normales ya posicionadas en el centro del área útil tras su primer `commit` válido.
    pub initially_placed_windows: HashSet<WlSurface>,
    /// Ventanas a las que ya se les envió un `configure` de acotamiento para evitar bucles si exceden el escritorio.
    pub clamped_oversized_windows: HashSet<WlSurface>,
    pub minimized_windows: HashMap<WlSurface, (smithay::desktop::Window, Point<i32, Logical>)>,
    pub minimized_window_workspaces: HashMap<WlSurface, usize>,
    pub minimized_window_thumbnails: HashMap<u64, String>,
    pub window_ids: HashMap<WlSurface, u64>,
    pub next_window_id: u64,
    pub dock_slots: HashMap<u64, Rectangle<i32, Logical>>,
    pub active_animations: Vec<GenieAnimation>,
    pub close_button_hints: HashMap<WlSurface, (Point<f64, Logical>, Instant, bool)>,
    /// Desplazamientos `(dx_desde_borde, dy_desde_arriba)` aprendidos por `(app_id, botones_derecha)`
    /// al cerrar ventanas CSD haciendo clic en su botón de cerrar.
    pub learned_close_button_offsets: HashMap<(String, bool), (f64, f64)>,
    pub renderer_context_id: Option<
        smithay::backend::renderer::ContextId<smithay::backend::renderer::gles::GlesTexture>,
    >,
    pub cached_close_snapshots:
        HashMap<WlSurface, (Vec<crate::black_hole::CachedSurfaceLayer>, Instant)>,
    pub pending_black_holes: Vec<crate::black_hole::PendingBlackHoleClose>,
    pub snap_transitions: HashMap<WlSurface, SnapTransition>,
    pub snap_preview: Option<Rectangle<i32, Logical>>,
    pub moving_window_surface: Option<WlSurface>,
    /// Última posición conocida del puntero; evita leer `PointerHandle::current_location()`
    /// desde callbacks ejecutados bajo el mutex interno de Smithay (`PointerGrab`).
    pub last_pointer_location: Option<Point<f64, Logical>>,
    pub socket_name: String,
    pub is_running: bool,
    pub ipc_broadcaster: IpcBroadcaster,
    pub app_switcher: AppSwitcherState,
    pub app_switcher_style: AppSwitcherStyle,
    pub cursor_status: smithay::input::pointer::CursorImageStatus,
    pub pending_displays_reload: bool,
    pub pending_brightness_reload: bool,
    pub pending_drm_hotplug: bool,
    pub saved_app_window_sizes: HashMap<String, Size<i32, Logical>>,
    pub saved_app_window_positions: HashMap<String, Point<i32, Logical>>,
    pub toplevel_return_layer_focus: HashMap<WlSurface, WlSurface>,
    pub dock_blur_headroom: i32,
    pub pending_screenshot_requests:
        std::collections::VecDeque<(std::path::PathBuf, Option<String>)>,
    pub pending_window_screenshot_requests:
        std::collections::VecDeque<(std::path::PathBuf, u64, Option<String>)>,
    pub pending_theme_transition_capture: Option<ThemeTransitionCapture>,
    pub theme_transition_started_at: Option<Instant>,
    pub pending_workspace_switch: Option<crate::workspace_transition::PendingWorkspaceSwitch>,
    pub workspace_transition: Option<crate::workspace_transition::WorkspaceTransitionState>,
    pub spotlight: crate::spotlight_visual::SpotlightState,
    pub dnd_icon: Option<WlSurface>,
    pub dnd_icon_offset: Point<i32, Logical>,
    pub dnd_drop_hold: Option<(WlSurface, Point<f64, Logical>, Instant)>,
    pub dnd_icon_cache: Option<CachedDndIcon>,
    pub dnd_icon_cache_dirty: bool,
    pub active_screen_recording: Option<crate::recording::ScreenRecordingSession>,
    pub screenshot_ui: crate::screenshot_ui::ScreenshotUiState,
    pub snap_layouts: crate::snap_layouts::SnapLayoutsState,
    pub media_keys: crate::media_keys::MediaKeysState,
    pub ssd: crate::ssd::SsdManager,
    pub expose: crate::expose::ExposeState,
    pub workspaces: crate::workspaces::WorkspaceManager,
    pub workspace_revision: u64,
    pub swipe_gesture: SwipeGestureState,
    pub pinch_gesture: PinchGestureState,
    pub surface_commit_counter: usize,
    pub output_commit_counters: HashMap<String, usize>,
    pub background_commit_counter: usize,
    pub auto_tiling_enabled: bool,
    pub dock_cfg: DockConfig,
    pub desktop_dock_cfg: malus_de_common::config::DesktopDockConfig,
    pub control_center_glass_regions: Vec<malus_de_common::ipc::GlassRegion>,
    pub input_cfg: malus_de_common::config::InputConfig,
    pub input_config_dirty: bool,
    pub show_desktop_hidden_ids: Vec<u64>,
    pub hot_corner_hover: Option<(&'static str, Instant, bool)>,
    pub dock_auto_hide_edge_active: bool,
}

pub fn guess_icon_name(app_id: &str, title: &str) -> String {
    let app = app_id.to_lowercase();
    let tit = title.to_lowercase();

    if app.contains("nautilus")
        || app.contains("archivos")
        || tit.contains("archivos")
        || tit.contains("carpetas")
    {
        "org.gnome.Nautilus".to_string()
    } else if app.contains("ptyxis") || app.contains("terminal") || tit.contains("terminal") {
        "org.gnome.Ptyxis".to_string()
    } else if app.contains("calculator")
        || app.contains("calculadora")
        || tit.contains("calculadora")
    {
        "org.gnome.Calculator".to_string()
    } else if app.contains("texteditor")
        || app.contains("text-editor")
        || app.contains("gedit")
        || tit.contains("editor")
    {
        "org.gnome.TextEditor".to_string()
    } else if app.contains("control-center")
        || app.contains("settings")
        || app.contains("configuración")
        || app.contains("configuracion")
    {
        "preferences-system".to_string()
    } else if app.contains("firefox") {
        "firefox".to_string()
    } else if app.contains("chrome") {
        "google-chrome".to_string()
    } else if !app.is_empty() {
        app_id.to_string()
    } else {
        "application-x-executable".to_string()
    }
}

pub fn toplevel_info(surface: &WlSurface) -> (Option<String>, Option<String>) {
    smithay::wayland::compositor::with_states(surface, |states| {
        if let Some(mutex) = states
            .data_map
            .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
        {
            let role = mutex.lock().unwrap();
            (role.app_id.clone(), role.title.clone())
        } else {
            (None, None)
        }
    })
}

fn matches_app_target(target_lower: &str, app_id: Option<&str>, title: Option<&str>) -> bool {
    if target_lower.is_empty() {
        return false;
    }

    let id_lower = app_id.map(str::to_lowercase);
    let title_lower = title.map(str::to_lowercase);

    if id_lower
        .as_deref()
        .is_some_and(|id| id.contains(target_lower))
        || title_lower
            .as_deref()
            .is_some_and(|t| t.contains(target_lower))
    {
        return true;
    }

    if let Some(ref id) = id_lower {
        if let Some(suffix) = target_lower.strip_prefix("malus-de-") {
            if id.contains(&format!("malus.{suffix}")) {
                return true;
            }
        }
        let compact = |s: &str| -> String {
            s.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect()
        };
        let compact_target = compact(target_lower);
        if !compact_target.is_empty() && compact(id).ends_with(&compact_target) {
            return true;
        }
    }

    false
}

pub const PRIMARY_SHELL_LAYER_NAMESPACES: &[&str] =
    &["malus-top-bar", "malus-dock", "malus-app-switcher"];

pub fn is_primary_shell_layer_namespace(namespace: &str) -> bool {
    PRIMARY_SHELL_LAYER_NAMESPACES.contains(&namespace)
}

fn is_root_window_surface(window: &smithay::desktop::Window, surface: &WlSurface) -> bool {
    window.wl_surface().as_deref() == Some(surface)
}

fn is_system_action_confirmation_surface(surface: &WlSurface) -> bool {
    toplevel_info(surface).1.as_deref() == Some("Confirmación de acción del sistema")
}

/// Normaliza el identificador de aplicación para consistencia en la persistencia
pub fn normalize_app_id(app_id: &str) -> String {
    let lower = app_id.to_lowercase();
    if lower.contains("firefox") || lower.contains("mozilla") {
        "firefox".to_string()
    } else if lower.contains("brave") {
        "brave".to_string()
    } else if lower.contains("nautilus") || lower.contains("archivos") {
        "org.gnome.nautilus".to_string()
    } else if lower.contains("malus.settings") || lower.contains("malus-de-settings") {
        "malus-settings".to_string()
    } else {
        lower
    }
}

/// Obtiene el identificador de aplicación normalizado a partir de una superficie
/// Calcula el tamaño máximo disponible para una ventana dentro del área útil del escritorio,
/// descontando la barra de título SSD cuando corresponda.
pub fn max_window_size_in_usable_zone(
    usable: Rectangle<i32, Logical>,
    ssd_top: i32,
) -> Size<i32, Logical> {
    Size::from((
        usable.size.w.max(1),
        (usable.size.h - ssd_top.max(0)).max(1),
    ))
}

/// Recorta un tamaño recordado o solicitado para que nunca exceda `max_bounds`.
/// No impone mínimos artificiales a ventanas pequeñas que quepan en el escritorio.
pub fn clamp_size_to_bounds(
    size: Size<i32, Logical>,
    max_bounds: Size<i32, Logical>,
) -> Size<i32, Logical> {
    Size::from((
        size.w.clamp(1, max_bounds.w.max(1)),
        size.h.clamp(1, max_bounds.h.max(1)),
    ))
}

/// Calcula la ubicación que centra una ventana dentro de `usable`, compensando `win_geo.loc`
/// (sombras CSD) y `ssd_top` (barra de título SSD por encima de `loc.y`).
pub fn centered_window_location(
    usable: Rectangle<i32, Logical>,
    win_geo: Rectangle<i32, Logical>,
    ssd_top: i32,
) -> Point<i32, Logical> {
    let ssd = ssd_top.max(0);
    let total_h = win_geo.size.h + ssd;
    let x = usable.loc.x + (usable.size.w - win_geo.size.w).max(0) / 2 - win_geo.loc.x;
    let y = usable.loc.y + (usable.size.h - total_h).max(0) / 2 + ssd - win_geo.loc.y;
    Point::from((x, y))
}

/// Si existe una posición visible recordada (`saved_visible_loc = space_loc + win_geo.loc`),
/// la acota dentro del área útil `usable` (respetando la Top Bar y barra SSD); si es la primera vez
/// que se abre la aplicación (`None`), la centra en pantalla con `centered_window_location`.
pub fn clamped_or_centered_window_location(
    usable: Rectangle<i32, Logical>,
    win_geo: Rectangle<i32, Logical>,
    ssd_top: i32,
    saved_visible_loc: Option<Point<i32, Logical>>,
) -> Point<i32, Logical> {
    let Some(saved) = saved_visible_loc else {
        return centered_window_location(usable, win_geo, ssd_top);
    };
    let ssd = ssd_top.max(0);
    let min_x = usable.loc.x;
    let max_x = (usable.loc.x + usable.size.w - win_geo.size.w).max(min_x);
    let min_y = usable.loc.y + ssd;
    let max_y = (usable.loc.y + usable.size.h - win_geo.size.h).max(min_y);
    let vis_x = saved.x.clamp(min_x, max_x);
    let vis_y = saved.y.clamp(min_y, max_y);
    Point::from((vis_x - win_geo.loc.x, vis_y - win_geo.loc.y))
}

pub fn app_id_for_surface(surface: &WlSurface) -> Option<String> {
    let (app_id, title) = toplevel_info(surface);
    if let Some(id) = app_id {
        if !id.is_empty() {
            return Some(normalize_app_id(&id));
        }
    }
    if let Some(t) = title {
        if !t.is_empty() {
            return Some(normalize_app_id(&t));
        }
    }
    None
}

/// Identifica si una ventana pertenece a Firefox (para aplicar recorte squircle a subsuperficies
/// y redimensión del lado del servidor sin interferir con ventanas GTK o Electron).
pub fn is_firefox_window(window: &smithay::desktop::Window) -> bool {
    if let Some(surface) = window.wl_surface() {
        let (app_id, title) = toplevel_info(&surface);
        if let Some(ref id) = app_id {
            let lower = id.to_lowercase();
            if lower.contains("firefox") || lower.contains("mozilla") {
                return true;
            }
        }
        if let Some(ref title) = title {
            let lower = title.to_lowercase();
            if lower.contains("firefox") || lower.contains("mozilla") {
                return true;
            }
        }
    }
    if let Some(toplevel) = window.toplevel() {
        let (app_id, title) = toplevel_info(toplevel.wl_surface());
        if let Some(ref id) = app_id {
            let lower = id.to_lowercase();
            if lower.contains("firefox") || lower.contains("mozilla") {
                return true;
            }
        }
        if let Some(ref title) = title {
            let lower = title.to_lowercase();
            if lower.contains("firefox") || lower.contains("mozilla") {
                return true;
            }
        }
    }
    false
}

fn contains_isolated_token(text: &str, token: &str) -> bool {
    let token_lower = token.to_lowercase();
    for part in text.split(|c: char| !c.is_alphanumeric()) {
        if part.eq_ignore_ascii_case(&token_lower) {
            return true;
        }
    }
    false
}

/// Identifica si una cadena de texto corresponde a un navegador web conocido
pub fn is_browser_identifier(text: &str) -> bool {
    let lower = text.to_lowercase();

    // Excluir explícitamente aplicaciones esenciales de Malus DE y GNOME
    // para evitar cualquier falso positivo en sus identificadores.
    if lower.contains("nautilus")
        || lower.contains("malus")
        || lower.contains("settings")
        || lower.contains("ptyxis")
        || lower.contains("terminal")
        || lower.contains("texteditor")
        || lower.contains("text-editor")
        || lower.contains("gedit")
        || lower.contains("calculator")
        || lower.contains("systemmonitor")
        || lower.contains("system-monitor")
    {
        return false;
    }

    lower.contains("firefox")
        || lower.contains("mozilla")
        || lower.contains("chrome")
        || lower.contains("chromium")
        || lower.contains("brave")
        || lower.contains("vivaldi")
        || lower.contains("epiphany")
        || lower.contains("zen-browser")
        || lower.contains("zen_browser")
        || lower.contains("waterfox")
        || lower.contains("librewolf")
        || lower.contains("tor-browser")
        || lower.contains("torbrowser")
        || lower.contains("msedge")
        || lower.contains("microsoft-edge")
        || contains_isolated_token(&lower, "edge")
        || contains_isolated_token(&lower, "opera")
}

/// Identifica si una ventana pertenece a un navegador web (para excluir sus menús
/// contextuales del desenfoque y capas de fondo del compositor).
///
/// Solo evalúa `app_id` (identidad del proceso/aplicación) y nunca el título
/// de la ventana, ya que carpetas de archivos (ej. en Nautilus) o documentos
/// abiertos pueden contener palabras o nombres de navegadores sin serlo.
pub fn is_browser_window(window: &smithay::desktop::Window) -> bool {
    let check = |app_id: &Option<String>| -> bool {
        if let Some(ref id) = app_id {
            if is_browser_identifier(id) {
                return true;
            }
        }
        false
    };

    if let Some(surface) = window.wl_surface() {
        let (app_id, _) = toplevel_info(&surface);
        if check(&app_id) {
            return true;
        }
    }
    if let Some(toplevel) = window.toplevel() {
        let (app_id, _) = toplevel_info(toplevel.wl_surface());
        if check(&app_id) {
            return true;
        }
    }
    false
}

/// Identifica si un `app_id` (en minúsculas) corresponde a una ventana flotante Quick Look / Preview
/// que requiere desenfoque completo en toda la tarjeta y no el recorte de barra lateral de Branches.
pub fn is_quicklook_identifier(lower: &str) -> bool {
    lower.contains("branches.quicklook")
        || lower.contains("screenshot-ui")
        || lower == "dev.malus.preview"
        || lower == "malus-de-preview"
}

/// Determina si una aplicación debe persistir y restaurar su tamaño/posición en `window_sizes.json`.
/// Las ventanas transitorias Quick Look / Preview se dimensionan dinámicamente según el archivo
/// previsualizado y siempre deben abrirse centradas con su tamaño natural.
pub fn should_persist_window_geometry(app_id: &str) -> bool {
    let lower = app_id.trim().to_ascii_lowercase();
    !is_quicklook_identifier(&lower)
}

/// Identifica si una ventana corresponde a una tarjeta translúcida completa (p. ej. Quick Look).
pub fn is_full_translucent_window(window: &smithay::desktop::Window) -> bool {
    let check = |app_id: &Option<String>| -> bool {
        app_id
            .as_deref()
            .map(str::to_lowercase)
            .is_some_and(|lower| is_quicklook_identifier(&lower))
    };

    if let Some(surface) = window.wl_surface() {
        let (app_id, _) = toplevel_info(&surface);
        if check(&app_id) {
            return true;
        }
    }
    if let Some(toplevel) = window.toplevel() {
        let (app_id, _) = toplevel_info(toplevel.wl_surface());
        if check(&app_id) {
            return true;
        }
    }
    false
}

/// Identifica si una ventana corresponde al explorador nativo Branches (`malus-de-branches`).
pub fn is_branches_window(window: &smithay::desktop::Window) -> bool {
    let check = |app_id: &Option<String>, title: &Option<String>| -> bool {
        if let Some(ref id) = app_id {
            let lower = id.to_lowercase();
            if is_quicklook_identifier(&lower) {
                return false;
            }
            if lower == "branches"
                || lower.contains("malus.branches")
                || lower.contains("malus-de-branches")
                || lower.contains("malus_de_branches")
            {
                return true;
            }
        }
        if let Some(ref title) = title {
            if title.trim().eq_ignore_ascii_case("branches") {
                return true;
            }
        }
        false
    };

    if let Some(surface) = window.wl_surface() {
        let (app_id, title) = toplevel_info(&surface);
        if check(&app_id, &title) {
            return true;
        }
    }
    if let Some(toplevel) = window.toplevel() {
        let (app_id, title) = toplevel_info(toplevel.wl_surface());
        if check(&app_id, &title) {
            return true;
        }
    }
    false
}

/// Identifica si una ventana requiere desenfoque de fondo en GPU.
pub fn is_translucent_window(window: &smithay::desktop::Window) -> bool {
    let check = |app_id: &Option<String>, title: &Option<String>| -> bool {
        if let Some(ref id) = app_id {
            let lower = id.to_lowercase();
            if lower.contains("malus.settings")
                || lower.contains("malus-de-settings")
                || lower.contains("malus_de_settings")
                || lower == "branches"
                || lower.contains("malus.branches")
                || lower.contains("malus-de-branches")
                || lower.contains("malus_de_branches")
                || is_quicklook_identifier(&lower)
            {
                return true;
            }
        }
        if let Some(ref title) = title {
            let lower = title.trim().to_lowercase();
            if lower == "ajustes del sistema"
                || lower == "confirmación de acción del sistema"
                || lower == "system action confirmation"
                || lower == "branches"
            {
                return true;
            }
        }
        false
    };

    if let Some(surface) = window.wl_surface() {
        let (app_id, title) = toplevel_info(&surface);
        if check(&app_id, &title) {
            return true;
        }
    }
    if let Some(toplevel) = window.toplevel() {
        let (app_id, title) = toplevel_info(toplevel.wl_surface());
        if check(&app_id, &title) {
            return true;
        }
    }
    false
}

/// Clasificación consolidada de propiedades de renderizado de una ventana en una sola lectura
/// de `toplevel_info` (evita múltiples bloqueos de mutex y asignaciones `to_lowercase` por frame).
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowRenderFlags {
    pub is_firefox: bool,
    pub is_browser: bool,
    pub is_branches: bool,
    pub is_translucent: bool,
    pub is_full_translucent: bool,
    pub popup_effects_enabled: bool,
    pub uses_ssd: bool,
}

impl WindowRenderFlags {
    pub fn from_window(window: &smithay::desktop::Window, main_surface: &WlSurface) -> Self {
        let (app_id, title) = toplevel_info(main_surface);
        let id_lower = app_id.as_deref().map(str::to_lowercase);
        let title_lower = title.as_deref().map(|t| t.trim().to_lowercase());

        let is_firefox = id_lower
            .as_deref()
            .is_some_and(|s| s.contains("firefox") || s.contains("mozilla"))
            || title_lower
                .as_deref()
                .is_some_and(|s| s.contains("firefox") || s.contains("mozilla"));

        let is_browser = app_id
            .as_deref()
            .is_some_and(is_browser_identifier);

        let is_full_translucent = id_lower.as_deref().is_some_and(is_quicklook_identifier);

        let is_branches = !is_full_translucent
            && (id_lower.as_deref().is_some_and(|lower| {
                lower == "branches"
                    || lower.contains("malus.branches")
                    || lower.contains("malus-de-branches")
                    || lower.contains("malus_de_branches")
            }) || title_lower.as_deref().is_some_and(|lower| lower == "branches"));

        let is_translucent = is_branches
            || is_full_translucent
            || id_lower.as_deref().is_some_and(|lower| {
                lower.contains("malus.settings")
                    || lower.contains("malus-de-settings")
                    || lower.contains("malus_de_settings")
            })
            || title_lower.as_deref().is_some_and(|lower| {
                lower == "ajustes del sistema"
                    || lower == "confirmación de acción del sistema"
                    || lower == "system action confirmation"
            });

        let popup_effects_enabled = !is_browser
            && app_id.as_deref().map_or(true, |id| {
                !id.contains("org.malus_de.dock") && !id.contains("malus-de-dock")
            });

        let uses_ssd = if is_browser {
            false
        } else {
            let is_gnome = id_lower.as_deref().is_some_and(|lower| {
                lower.starts_with("org.gnome.")
                    || lower.contains("nautilus")
                    || lower.contains("malus.settings")
                    || lower.contains("malus-de-settings")
                    || lower.contains("malus_de_settings")
                    || lower == "branches"
                    || lower.contains("malus.branches")
                    || lower.contains("malus-de-branches")
                    || lower.contains("malus_de_branches")
                    || lower.contains("ptyxis")
                    || lower.contains("texteditor")
                    || lower.contains("calculator")
                    || lower.contains("systemmonitor")
            });
            if is_gnome {
                false
            } else if let Some(toplevel) = window.toplevel() {
                let state = toplevel.current_state();
                let is_fullscreen = state.states.contains(
                    smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen,
                );
                use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
                !is_fullscreen && state.decoration_mode == Some(Mode::ServerSide)
            } else {
                false
            }
        };

        Self {
            is_firefox,
            is_browser,
            is_branches,
            is_translucent,
            is_full_translucent,
            popup_effects_enabled,
            uses_ssd,
        }
    }
}

fn parse_color_scheme(settings: &str) -> Option<bool> {
    let mut in_interface_section = false;
    for line in settings.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            in_interface_section = line == "[org/gnome/desktop/interface]";
        } else if in_interface_section {
            if let Some(value) = line.strip_prefix("color-scheme=") {
                let value = value.trim().trim_matches(['\'', '\"']);
                return match value {
                    "prefer-dark" => Some(true),
                    "prefer-light" => Some(false),
                    _ => None,
                };
            }
        }
    }
    None
}

fn settings_keyfile_path() -> Option<PathBuf> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config_home.join("glib-2.0/settings/keyfile"))
}

fn read_system_dark_mode(settings_path: &std::path::Path) -> Option<bool> {
    match std::fs::read_to_string(settings_path) {
        Ok(settings) => parse_color_scheme(&settings),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            tracing::warn!(
                "No se pudo leer el tema GTK en {:?}: {error}",
                settings_path
            );
            None
        }
    }
}

/// Indica si el esquema de color GTK activo en la sesión es oscuro (`prefer-dark`).
pub fn is_dark_color_scheme() -> bool {
    static THEME_CACHE: OnceLock<Mutex<Option<(Instant, Option<std::time::SystemTime>, bool)>>> =
        OnceLock::new();
    let cache = THEME_CACHE.get_or_init(|| Mutex::new(None));
    let mut cached_theme = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let should_refresh = cached_theme
        .as_ref()
        .is_none_or(|(last_refresh, _, _)| last_refresh.elapsed() >= Duration::from_millis(500));
    if should_refresh {
        let keyfile_path = settings_keyfile_path();
        let current_mtime = keyfile_path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok());
        let prev_mtime = cached_theme.as_ref().and_then(|(_, mtime, _)| *mtime);
        let is_dark = if cached_theme.is_none() || current_mtime != prev_mtime {
            keyfile_path
                .as_deref()
                .and_then(read_system_dark_mode)
                .or_else(|| cached_theme.as_ref().map(|(_, _, is_dark)| *is_dark))
                .unwrap_or(false)
        } else {
            cached_theme
                .as_ref()
                .map(|(_, _, is_dark)| *is_dark)
                .unwrap_or(false)
        };
        *cached_theme = Some((Instant::now(), current_mtime, is_dark));
    }

    cached_theme
        .as_ref()
        .is_some_and(|(_, _, is_dark)| *is_dark)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapMode {
    LeftHalf,
    RightHalf,
    LeftTwoThirds,
    RightOneThird,
    LeftThird,
    CenterThird,
    RightThird,
    TopLeftQuarter,
    TopRightQuarter,
    BottomLeftQuarter,
    BottomRightQuarter,
    LeftQuarter,
    CenterHalf,
    RightQuarter,
    Maximize,
    CenterRestore,
}

/// Devuelve `true` si la ventana tiene actualmente algún estado XDG de maximizado o anclaje (tile).
pub fn is_window_snapped(window: &smithay::desktop::Window) -> bool {
    window.toplevel().is_some_and(|t| {
        let cur = &t.current_state().states;
        let cur_snapped = cur.contains(XdgState::Maximized)
            || cur.contains(XdgState::TiledLeft)
            || cur.contains(XdgState::TiledRight)
            || cur.contains(XdgState::TiledTop)
            || cur.contains(XdgState::TiledBottom)
            || cur.contains(XdgState::Fullscreen);
        let pend_snapped = t.with_pending_state(|p| {
            p.states.contains(XdgState::Maximized)
                || p.states.contains(XdgState::TiledLeft)
                || p.states.contains(XdgState::TiledRight)
                || p.states.contains(XdgState::TiledTop)
                || p.states.contains(XdgState::TiledBottom)
                || p.states.contains(XdgState::Fullscreen)
        });
        cur_snapped || pend_snapped
    })
}

/// Limpia todos los estados XDG de maximizado y anclaje en mosaico.
pub fn unset_snap_states(top_state: &mut smithay::wayland::shell::xdg::ToplevelState) {
    top_state.states.unset(XdgState::Maximized);
    top_state.states.unset(XdgState::TiledLeft);
    top_state.states.unset(XdgState::TiledRight);
    top_state.states.unset(XdgState::TiledTop);
    top_state.states.unset(XdgState::TiledBottom);
}

#[inline]
fn split_interval(start: i32, total: i32, num_start: i32, num_end: i32, den: i32) -> (i32, i32) {
    let p0 = start + (total * num_start) / den;
    let p1 = start + (total * num_end) / den;
    (p0, (p1 - p0).max(100))
}

/// Calcula ubicación destino, tamaño cliente y estados XDG para cualquier sección de Snap Layouts.
pub fn snap_target_geometry(
    mode: SnapMode,
    usable_zone: Rectangle<i32, Logical>,
    ssd_top: i32,
) -> (
    Point<i32, Logical>,
    Option<Size<i32, Logical>>,
    Vec<XdgState>,
) {
    let ux = usable_zone.loc.x;
    let uy = usable_zone.loc.y;
    let uw = usable_zone.size.w;
    let uh = usable_zone.size.h;
    let full_y = uy + ssd_top;
    let full_h = (uh - ssd_top).max(100);

    let (top_y, top_visual_h) = split_interval(uy, uh, 0, 1, 2);
    let top_row_y = top_y + ssd_top;
    let top_row_h = (top_visual_h - ssd_top).max(100);

    let (bot_y, bot_visual_h) = split_interval(uy, uh, 1, 2, 2);
    let bot_row_y = bot_y + ssd_top;
    let bot_row_h = (bot_visual_h - ssd_top).max(100);

    match mode {
        SnapMode::LeftHalf => {
            let (x, w) = split_interval(ux, uw, 0, 1, 2);
            (
                Point::from((x, full_y)),
                Some(Size::from((w, full_h))),
                vec![XdgState::TiledLeft],
            )
        }
        SnapMode::RightHalf => {
            let (x, w) = split_interval(ux, uw, 1, 2, 2);
            (
                Point::from((x, full_y)),
                Some(Size::from((w, full_h))),
                vec![XdgState::TiledRight],
            )
        }
        SnapMode::LeftTwoThirds => {
            let (x, w) = split_interval(ux, uw, 0, 2, 3);
            (
                Point::from((x, full_y)),
                Some(Size::from((w, full_h))),
                vec![XdgState::TiledLeft],
            )
        }
        SnapMode::RightOneThird => {
            let (x, w) = split_interval(ux, uw, 2, 3, 3);
            (
                Point::from((x, full_y)),
                Some(Size::from((w, full_h))),
                vec![XdgState::TiledRight],
            )
        }
        SnapMode::LeftThird => {
            let (x, w) = split_interval(ux, uw, 0, 1, 3);
            (
                Point::from((x, full_y)),
                Some(Size::from((w, full_h))),
                vec![XdgState::TiledLeft],
            )
        }
        SnapMode::CenterThird => {
            let (x, w) = split_interval(ux, uw, 1, 2, 3);
            (
                Point::from((x, full_y)),
                Some(Size::from((w, full_h))),
                vec![XdgState::TiledTop, XdgState::TiledBottom],
            )
        }
        SnapMode::RightThird => {
            let (x, w) = split_interval(ux, uw, 2, 3, 3);
            (
                Point::from((x, full_y)),
                Some(Size::from((w, full_h))),
                vec![XdgState::TiledRight],
            )
        }
        SnapMode::TopLeftQuarter => {
            let (x, w) = split_interval(ux, uw, 0, 1, 2);
            (
                Point::from((x, top_row_y)),
                Some(Size::from((w, top_row_h))),
                vec![XdgState::TiledLeft, XdgState::TiledTop],
            )
        }
        SnapMode::TopRightQuarter => {
            let (x, w) = split_interval(ux, uw, 1, 2, 2);
            (
                Point::from((x, top_row_y)),
                Some(Size::from((w, top_row_h))),
                vec![XdgState::TiledRight, XdgState::TiledTop],
            )
        }
        SnapMode::BottomLeftQuarter => {
            let (x, w) = split_interval(ux, uw, 0, 1, 2);
            (
                Point::from((x, bot_row_y)),
                Some(Size::from((w, bot_row_h))),
                vec![XdgState::TiledLeft, XdgState::TiledBottom],
            )
        }
        SnapMode::BottomRightQuarter => {
            let (x, w) = split_interval(ux, uw, 1, 2, 2);
            (
                Point::from((x, bot_row_y)),
                Some(Size::from((w, bot_row_h))),
                vec![XdgState::TiledRight, XdgState::TiledBottom],
            )
        }
        SnapMode::LeftQuarter => {
            let (x, w) = split_interval(ux, uw, 0, 1, 4);
            (
                Point::from((x, full_y)),
                Some(Size::from((w, full_h))),
                vec![XdgState::TiledLeft],
            )
        }
        SnapMode::CenterHalf => {
            let (x, w) = split_interval(ux, uw, 1, 3, 4);
            (
                Point::from((x, full_y)),
                Some(Size::from((w, full_h))),
                vec![XdgState::TiledTop, XdgState::TiledBottom],
            )
        }
        SnapMode::RightQuarter => {
            let (x, w) = split_interval(ux, uw, 3, 4, 4);
            (
                Point::from((x, full_y)),
                Some(Size::from((w, full_h))),
                vec![XdgState::TiledRight],
            )
        }
        SnapMode::Maximize => (
            Point::from((ux, full_y)),
            Some(Size::from((uw, full_h))),
            vec![XdgState::Maximized],
        ),
        SnapMode::CenterRestore => (Point::from((ux + 80, uy + 80 + ssd_top)), None, vec![]),
    }
}

impl MalusState {
    pub fn spawn_lockscreen_process(&self) {
        let lock_executable = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.parent()
                    .map(|parent| parent.join("malus-de-session-lock"))
            })
            .filter(|path| path.is_file())
            .unwrap_or_else(|| std::path::PathBuf::from("malus-de-session-lock"));
        let result = std::process::Command::new(lock_executable)
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .env("GDK_BACKEND", "wayland")
            .spawn();
        if let Err(error) = result {
            tracing::error!("No se pudo iniciar malus-de-session-lock: {error}");
        }
    }

    pub fn lock_session(&self) {
        if self.is_session_locked() {
            return;
        }
        self.spawn_lockscreen_process();
    }

    pub fn start_full_screenshot(&self) {
        if self.is_session_locked() {
            return;
        }
        let screenshot_executable = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.parent()
                    .map(|parent| parent.join("malus-de-screenshot"))
            })
            .filter(|path| path.is_file())
            .unwrap_or_else(|| std::path::PathBuf::from("malus-de-screenshot"));
        let result = std::process::Command::new(screenshot_executable)
            .args(["--mode", "full"])
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .env("GDK_BACKEND", "wayland")
            .spawn();
        if let Err(error) = result {
            tracing::error!("No se pudo iniciar la captura de pantalla completa: {error}");
        }
    }

    pub fn start_area_screenshot(&self) {
        if self.is_session_locked() {
            return;
        }
        let screenshot_executable = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.parent()
                    .map(|parent| parent.join("malus-de-screenshot"))
            })
            .filter(|path| path.is_file())
            .unwrap_or_else(|| std::path::PathBuf::from("malus-de-screenshot"));
        let result = std::process::Command::new(screenshot_executable)
            .args(["--mode", "area"])
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .env("GDK_BACKEND", "wayland")
            .spawn();
        if let Err(error) = result {
            tracing::error!("No se pudo iniciar la captura de pantalla de área: {error}");
        }
    }

    pub fn screenshot_ui_output_geometry(&self) -> Rectangle<i32, Logical> {
        self.screenshot_ui
            .target_output
            .as_deref()
            .and_then(|name| {
                self.space
                    .outputs()
                    .find(|o| o.name() == name)
                    .and_then(|o| self.space.output_geometry(o))
            })
            .or_else(|| {
                self.primary_output()
                    .and_then(|o| self.space.output_geometry(&o))
            })
            .or_else(|| {
                self.space
                    .outputs()
                    .next()
                    .and_then(|o| self.space.output_geometry(o))
            })
            .unwrap_or_default()
    }

    pub fn window_rect_under_point_in_output(
        &self,
        output_geo: Rectangle<i32, Logical>,
        point: Point<f64, Logical>,
    ) -> Option<Rectangle<i32, Logical>> {
        if output_geo.size.w <= 0 || output_geo.size.h <= 0 {
            return None;
        }
        let win = self
            .space
            .elements()
            .rev()
            .find(|w| {
                let Some(loc) = self.space.element_location(w) else {
                    return false;
                };
                let geo = w.geometry();
                let top_pad = if window_uses_ssd(w) {
                    crate::ssd::SSD_TITLEBAR_HEIGHT
                } else {
                    0
                };
                let rect = Rectangle::new(
                    (loc.x, loc.y - top_pad).into(),
                    (geo.size.w, geo.size.h + top_pad).into(),
                );
                rect.to_f64().contains(point)
            })
            .cloned()
            .or_else(|| self.get_active_window())?;

        let loc = self.space.element_location(&win)?;
        let geo = win.geometry();
        let top_pad = if window_uses_ssd(&win) {
            crate::ssd::SSD_TITLEBAR_HEIGHT
        } else {
            0
        };
        let win_rect = Rectangle::new(
            (loc.x, loc.y - top_pad).into(),
            (geo.size.w.max(1), (geo.size.h + top_pad).max(1)).into(),
        );
        let overlap = win_rect.intersection(output_geo)?;
        if overlap.size.w <= 0 || overlap.size.h <= 0 {
            return None;
        }
        let rel_x = (overlap.loc.x - output_geo.loc.x).max(0);
        let rel_y = (overlap.loc.y - output_geo.loc.y).max(0);
        let rel_w = overlap.size.w.min(output_geo.size.w - rel_x).max(1);
        let rel_h = overlap.size.h.min(output_geo.size.h - rel_y).max(1);
        Some(Rectangle::new((rel_x, rel_y).into(), (rel_w, rel_h).into()))
    }

    pub fn window_id_under_point_in_output(
        &self,
        output_geo: Rectangle<i32, Logical>,
        point: Point<f64, Logical>,
    ) -> Option<u64> {
        if output_geo.size.w <= 0 || output_geo.size.h <= 0 {
            return None;
        }
        let win = self
            .space
            .elements()
            .rev()
            .find(|w| {
                let Some(loc) = self.space.element_location(w) else {
                    return false;
                };
                let geo = w.geometry();
                let top_pad = if window_uses_ssd(w) {
                    crate::ssd::SSD_TITLEBAR_HEIGHT
                } else {
                    0
                };
                let rect = Rectangle::new(
                    (loc.x, loc.y - top_pad).into(),
                    (geo.size.w, geo.size.h + top_pad).into(),
                );
                rect.to_f64().contains(point)
            })
            .cloned()
            .or_else(|| self.get_active_window())?;
        let surf = win.wl_surface()?;
        self.window_ids.get(surf.as_ref()).copied()
    }

    pub fn window_by_id(&self, window_id: u64) -> Option<smithay::desktop::Window> {
        self.space
            .elements()
            .find(|w| {
                w.wl_surface()
                    .and_then(|s| self.window_ids.get(s.as_ref()).copied())
                    == Some(window_id)
            })
            .cloned()
    }

    pub fn start_ocr_screenshot(&mut self) {
        if self.is_session_locked() {
            return;
        }
        self.snap_layouts.close();
        let pointer_loc = self.seat.get_pointer().map(|p| p.current_location());
        let chosen_output = pointer_loc
            .and_then(|pt| {
                self.space
                    .outputs()
                    .find(|o| {
                        self.space
                            .output_geometry(o)
                            .is_some_and(|g| g.to_f64().contains(pt))
                    })
                    .cloned()
            })
            .or_else(|| self.primary_output())
            .or_else(|| self.space.outputs().next().cloned());

        let (output_geo, output_name) = chosen_output
            .map(|o| (self.space.output_geometry(&o).unwrap_or_default(), o.name()))
            .unwrap_or_else(|| {
                (
                    Rectangle::new((0, 0).into(), (1920, 1080).into()),
                    "default".to_string(),
                )
            });
        self.screenshot_ui.open_with_mode(
            output_geo,
            output_name,
            crate::screenshot_ui::ScreenshotUiMode::OcrArea,
        );
    }

    pub fn start_interactive_screenshot(&mut self) {
        if self.is_session_locked() {
            return;
        }
        self.snap_layouts.close();
        if self.screenshot_ui.is_open() {
            self.screenshot_ui.close();
            return;
        }
        let pointer_loc = self.seat.get_pointer().map(|p| p.current_location());
        let chosen_output = pointer_loc
            .and_then(|pt| {
                self.space
                    .outputs()
                    .find(|o| {
                        self.space
                            .output_geometry(o)
                            .is_some_and(|g| g.to_f64().contains(pt))
                    })
                    .cloned()
            })
            .or_else(|| self.primary_output())
            .or_else(|| self.space.outputs().next().cloned());

        let (output_geo, output_name) = chosen_output
            .map(|o| (self.space.output_geometry(&o).unwrap_or_default(), o.name()))
            .unwrap_or_else(|| {
                (
                    Rectangle::new((0, 0).into(), (1920, 1080).into()),
                    "default".to_string(),
                )
            });
        self.screenshot_ui.open(output_geo, output_name);
    }

    pub fn execute_screenshot_ui_action(&mut self) {
        if !self.screenshot_ui.is_open {
            return;
        }

        let target_output_name = self.screenshot_ui.target_output.clone();
        let target_output_geo = target_output_name
            .as_deref()
            .and_then(|name| {
                self.space
                    .outputs()
                    .find(|o| o.name() == name)
                    .and_then(|o| self.space.output_geometry(o))
            })
            .or_else(|| {
                self.space
                    .outputs()
                    .next()
                    .and_then(|o| self.space.output_geometry(o))
            })
            .unwrap_or_else(|| Rectangle::from_size((1920, 1080).into()));

        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        if self.screenshot_ui.mode.is_video() {
            let options = self.screenshot_ui.options.clone();
            let is_area = self.screenshot_ui.mode.is_area();
            let sel = self.screenshot_ui.clamped_selection(target_output_geo);
            self.screenshot_ui.close();

            let dest_dir = match options.save_to {
                crate::screenshot_ui::SaveTarget::Desktop => {
                    let es = home.join("Escritorio");
                    if es.exists() { es } else { home.join("Desktop") }
                }
                crate::screenshot_ui::SaveTarget::Documents => {
                    let es = home.join("Documentos");
                    if es.exists() { es } else { home.join("Documents") }
                }
                _ => {
                    let videos = home.join("Vídeos");
                    if videos.exists() {
                        videos.join("Grabaciones de pantalla")
                    } else {
                        home.join("Videos/Screen Recordings")
                    }
                }
            };
            let _ = std::fs::create_dir_all(&dest_dir);
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let dest = dest_dir.join(format!("Grabación de pantalla-{timestamp}.mp4"));

            let crop = if is_area {
                Some((
                    sel.x.round() as i32,
                    sel.y.round() as i32,
                    sel.width.round() as i32,
                    sel.height.round() as i32,
                ))
            } else {
                None
            };

            self.start_screen_recording_on_output(
                dest,
                target_output_name,
                crop,
                options.include_audio,
                0,
                options.lossless,
            );
        } else {
            let options = self.screenshot_ui.options.clone();
            let mode = self.screenshot_ui.mode;
            let sel = self.screenshot_ui.clamped_selection(target_output_geo);
            let pt = self
                .seat
                .get_pointer()
                .map(|p| p.current_location())
                .unwrap_or_else(|| Point::from((0.0, 0.0)));
            let win_rect = self
                .screenshot_ui
                .hovered_window_rect
                .or_else(|| self.window_rect_under_point_in_output(target_output_geo, pt));
            let win_id = self.window_id_under_point_in_output(target_output_geo, pt);
            self.screenshot_ui.close();

            let sw = target_output_geo.size.w as f64;
            let sh = target_output_geo.size.h as f64;

            let mut cmd = std::process::Command::new(
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.join("malus-de-screenshot")))
                    .filter(|p| p.is_file())
                    .unwrap_or_else(|| std::path::PathBuf::from("malus-de-screenshot")),
            );

            if mode == crate::screenshot_ui::ScreenshotUiMode::OcrArea {
                let crop_arg = format!(
                    "{},{},{},{},{},{}",
                    sel.x.max(0.0),
                    sel.y.max(0.0),
                    sel.width.max(10.0),
                    sel.height.max(10.0),
                    sw,
                    sh
                );
                cmd.args(["--mode", "ocr", "--crop", &crop_arg]);
            } else if mode == crate::screenshot_ui::ScreenshotUiMode::CaptureArea {
                let crop_arg = format!(
                    "{},{},{},{},{},{}",
                    sel.x.max(0.0),
                    sel.y.max(0.0),
                    sel.width.max(10.0),
                    sel.height.max(10.0),
                    sw,
                    sh
                );
                cmd.args(["--mode", "area", "--crop", &crop_arg]);
            } else if mode == crate::screenshot_ui::ScreenshotUiMode::CaptureWindow {
                cmd.args(["--mode", "window"]);
                if options.transparent_window_bg {
                    if let Some(wid) = win_id {
                        cmd.args(["--window-id", &wid.to_string()]);
                    }
                }
                if let Some(wr) = win_rect {
                    let crop_arg = format!(
                        "{},{},{},{},{},{}",
                        wr.loc.x.max(0),
                        wr.loc.y.max(0),
                        wr.size.w.max(16),
                        wr.size.h.max(16),
                        sw,
                        sh
                    );
                    cmd.args(["--crop", &crop_arg]);
                }
            } else {
                cmd.args(["--mode", "full"]);
            }

            if let Some(ref out_name) = target_output_name {
                cmd.args(["--output", out_name]);
            }

            if options.save_to == crate::screenshot_ui::SaveTarget::Clipboard {
                cmd.arg("--clipboard-only");
            } else {
                let dest_dir = match options.save_to {
                    crate::screenshot_ui::SaveTarget::Desktop => {
                        let es = home.join("Escritorio");
                        if es.exists() { es } else { home.join("Desktop") }
                    }
                    crate::screenshot_ui::SaveTarget::Documents => {
                        let es = home.join("Documentos");
                        if es.exists() { es } else { home.join("Documents") }
                    }
                    _ => {
                        let pics = home.join("Imágenes");
                        if pics.exists() {
                            pics.join("Capturas de pantalla")
                        } else {
                            home.join("Pictures/Screenshots")
                        }
                    }
                };
                cmd.args(["--save-dir", dest_dir.to_str().unwrap_or(".")]);
            }

            if options.timer_seconds > 0 {
                cmd.args(["--delay", &options.timer_seconds.to_string()]);
            }

            cmd.env("WAYLAND_DISPLAY", &self.socket_name);
            cmd.env("GDK_BACKEND", "wayland");
            let _ = cmd.spawn();
        }
    }

    pub fn start_screen_recording(
        &mut self,
        destination: std::path::PathBuf,
        rect: Option<(i32, i32, i32, i32)>,
        include_audio: bool,
        fps: u32,
        lossless: bool,
    ) {
        let out_name = self
            .primary_output()
            .or_else(|| self.space.outputs().next().cloned())
            .map(|o| o.name());
        self.start_screen_recording_on_output(
            destination,
            out_name,
            rect,
            include_audio,
            fps,
            lossless,
        );
    }

    pub fn start_screen_recording_on_output(
        &mut self,
        destination: std::path::PathBuf,
        output_name: Option<String>,
        rect: Option<(i32, i32, i32, i32)>,
        include_audio: bool,
        fps: u32,
        lossless: bool,
    ) {
        if self.active_screen_recording.is_some() {
            tracing::warn!("Ya hay una grabación de pantalla activa; deteniéndola primero");
            self.stop_screen_recording();
        }

        let target_output = output_name
            .as_deref()
            .and_then(|name| self.space.outputs().find(|o| o.name() == name).cloned())
            .or_else(|| self.primary_output())
            .or_else(|| self.space.outputs().next().cloned());

        let resolved_output_name = target_output.as_ref().map(|o| o.name()).or(output_name);
        let scale = target_output
            .as_ref()
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or(1.0);
        let (out_phys_w, out_phys_h, out_refresh_hz) = target_output
            .as_ref()
            .and_then(|o| {
                o.current_mode().map(|m| {
                    let hz = ((m.refresh as f64) / 1000.0).round() as u32;
                    (m.size.w, m.size.h, hz)
                })
            })
            .unwrap_or((1920, 1080, 60));

        // Si fps == 0 usamos la frecuencia nativa del monitor (tope 60 fps para máxima fluidez y compatibilidad MP4).
        let resolved_fps = if fps == 0 {
            out_refresh_hz.clamp(30, 60)
        } else {
            fps.clamp(10, 120)
        };

        let (width, height, phys_rect) = if let Some((x, y, w, h)) = rect {
            let phys_x = ((x as f64 * scale).round() as i32).clamp(0, (out_phys_w - 2).max(0));
            let phys_y = ((y as f64 * scale).round() as i32).clamp(0, (out_phys_h - 2).max(0));
            let max_w = (out_phys_w - phys_x).max(2);
            let max_h = (out_phys_h - phys_y).max(2);
            let phys_w = (((w as f64 * scale).round() as i32).min(max_w) & !1).max(2);
            let phys_h = (((h as f64 * scale).round() as i32).min(max_h) & !1).max(2);
            let r = Rectangle::new((phys_x, phys_y).into(), (phys_w, phys_h).into());
            (phys_w as u32, phys_h as u32, Some(r))
        } else {
            let w = (out_phys_w & !1).max(2) as u32;
            let h = (out_phys_h & !1).max(2) as u32;
            (w, h, None)
        };

        match crate::recording::ScreenRecordingSession::start(
            destination.clone(),
            resolved_output_name,
            phys_rect,
            width,
            height,
            include_audio,
            resolved_fps,
            lossless,
            self.ipc_broadcaster.clone(),
            self.socket_name.clone(),
        ) {
            Ok(session) => {
                self.screenshot_ui.is_recording = true;
                self.screenshot_ui.recording_start = Some(session.start_time);
                self.screenshot_ui.recording_dest = Some(session.destination.clone());
                self.active_screen_recording = Some(session);
                self.ipc_broadcaster
                    .broadcast(&IpcMessage::ScreenRecordingStatus {
                        is_recording: true,
                        elapsed_secs: 0,
                    });
            }
            Err(e) => {
                tracing::error!("No se pudo iniciar la grabación de pantalla: {e}");
                self.screenshot_ui.is_recording = false;
                self.screenshot_ui.recording_start = None;
                self.screenshot_ui.recording_dest = None;
                self.ipc_broadcaster
                    .broadcast(&IpcMessage::ScreenRecordingStatus {
                        is_recording: false,
                        elapsed_secs: 0,
                    });
            }
        }
    }

    pub fn stop_screen_recording(&mut self) {
        self.screenshot_ui.is_recording = false;
        self.screenshot_ui.recording_start = None;
        self.screenshot_ui.recording_dest = None;
        if let Some(session) = self.active_screen_recording.take() {
            tracing::info!(
                "⏹ Deteniendo grabación de pantalla en: {}",
                session.destination.display()
            );
            drop(session);
            self.ipc_broadcaster
                .broadcast(&IpcMessage::ScreenRecordingStatus {
                    is_recording: false,
                    elapsed_secs: 0,
                });
        }
    }
    pub fn new(display: &Display<MalusState>) -> Self {
        let dh = display.handle();
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state =
            smithay::wayland::shell::xdg::decoration::XdgDecorationState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let session_lock_state = SessionLockManagerState::new::<Self, _>(&dh, |_| true);
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let presentation_state =
            smithay::wayland::presentation::PresentationState::new::<Self>(&dh, 1);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let dmabuf_state = DmabufState::new();
        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, "malus-seat-0");
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let data_control_state = DataControlState::new::<Self, _>(&dh, None, |_| true);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);

        let ipc_broadcaster = IpcBroadcaster::new();
        // Evitar que `cargo test` dentro de una sesión activa elimine y sobrescriba el socket IPC vivo.
        #[cfg(not(test))]
        if let Err(e) = ipc_broadcaster.start_server() {
            tracing::error!("Error al iniciar servidor IPC: {:?}", e);
        }
        let dock_config = DockConfig::load_or_create_default();
        let desktop_dock_cfg = malus_de_common::config::DesktopDockConfig::load();
        let input_cfg = malus_de_common::config::InputConfig::load();
        let (saved_app_window_sizes, saved_app_window_positions) =
            Self::load_saved_window_geometries();
        let dock_blur_headroom = dock_config.headroom_height().min(i32::MAX as u32) as i32;
        let app_switcher_style = dock_config.app_switcher_style;

        Self {
            display_handle: dh,
            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            layer_shell_state,
            session_lock_state,
            ext_session_locked: false,
            ext_lock_handle: None,
            ext_lock_client_pid: None,
            ext_lock_surfaces: HashMap::new(),
            malus_lock_layers: Vec::new(),
            lockscreen_respawn_at: None,
            session_locked_flag: Arc::new(AtomicBool::new(false)),
            fractional_scale_manager_state,
            viewporter_state,
            presentation_state,
            shm_state,
            dmabuf_state,
            dmabuf_global: None,
            pending_dmabuf_imports: Vec::new(),
            seat_state,
            data_device_state,
            primary_selection_state,
            data_control_state,
            output_manager_state,
            seat,
            focus_cpu_priority: FocusCpuPriority::default(),
            space: Space::default(),
            popups: PopupManager::default(),
            active_popups: Vec::new(),
            popup_grab: None,
            defer_activation_configure: false,
            pending_activation_configures: Vec::new(),
            window_restore_geometries: HashMap::new(),
            centered_transient_windows: HashMap::new(),
            initially_placed_windows: HashSet::new(),
            clamped_oversized_windows: HashSet::new(),
            minimized_windows: HashMap::new(),
            minimized_window_workspaces: HashMap::new(),
            minimized_window_thumbnails: HashMap::new(),
            window_ids: HashMap::new(),
            next_window_id: 0,
            dock_slots: HashMap::new(),
            active_animations: Vec::new(),
            close_button_hints: HashMap::new(),
            learned_close_button_offsets: HashMap::new(),
            renderer_context_id: None,
            cached_close_snapshots: HashMap::new(),
            pending_black_holes: Vec::new(),
            snap_transitions: HashMap::new(),
            snap_preview: None,
            moving_window_surface: None,
            last_pointer_location: None,
            socket_name: String::new(),
            is_running: true,
            ipc_broadcaster,
            app_switcher: AppSwitcherState::default(),
            app_switcher_style,
            cursor_status: smithay::input::pointer::CursorImageStatus::default_named(),
            pending_displays_reload: false,
            pending_brightness_reload: false,
            pending_drm_hotplug: false,
            saved_app_window_sizes,
            saved_app_window_positions,
            toplevel_return_layer_focus: HashMap::new(),
            dock_blur_headroom,
            pending_screenshot_requests: std::collections::VecDeque::new(),
            pending_window_screenshot_requests: std::collections::VecDeque::new(),
            pending_theme_transition_capture: None,
            theme_transition_started_at: None,
            pending_workspace_switch: None,
            workspace_transition: None,
            spotlight: crate::spotlight_visual::SpotlightState::new(),
            dnd_icon: None,
            dnd_icon_offset: Point::from((0, 0)),
            dnd_drop_hold: None,
            dnd_icon_cache: None,
            dnd_icon_cache_dirty: false,
            active_screen_recording: None,
            screenshot_ui: crate::screenshot_ui::ScreenshotUiState::new(),
            snap_layouts: crate::snap_layouts::SnapLayoutsState::new(),
            media_keys: crate::media_keys::MediaKeysState::new(),
            ssd: crate::ssd::SsdManager::new_with_layout(
                malus_de_common::config::load_window_buttons_right(),
            ),
            expose: crate::expose::ExposeState::default(),
            workspaces: crate::workspaces::WorkspaceManager::default(),
            workspace_revision: 0,
            swipe_gesture: SwipeGestureState::default(),
            pinch_gesture: PinchGestureState::default(),
            surface_commit_counter: 0,
            output_commit_counters: HashMap::new(),
            background_commit_counter: 0,
            auto_tiling_enabled: false,
            dock_cfg: dock_config,
            desktop_dock_cfg,
            control_center_glass_regions: Vec::new(),
            input_cfg,
            input_config_dirty: false,
            show_desktop_hidden_ids: Vec::new(),
            hot_corner_hover: None,
            dock_auto_hide_edge_active: false,
        }
    }

    fn reload_dock_config_fields(&mut self) {
        let config = DockConfig::load_or_create_default();
        if let Some(parent) = DockConfig::config_path().parent() {
            let _ = malus_de_common::config::sync_titlebar_double_click_action(
                parent,
                config.titlebar_double_click_action,
            );
        }
        if !config.auto_hide && self.dock_auto_hide_edge_active {
            self.dock_auto_hide_edge_active = false;
            self.ipc_broadcaster
                .broadcast(&IpcMessage::DockAutoHideEdge { active: false });
        }
        self.dock_cfg = config.clone();
        self.dock_blur_headroom = config.headroom_height().min(i32::MAX as u32) as i32;
        if self.app_switcher_style != config.app_switcher_style
            && (self.app_switcher.is_active || self.app_switcher.deck.active)
        {
            self.dismiss_app_switcher_immediately();
        }
        self.app_switcher_style = config.app_switcher_style;
    }

    /// Carga los tamaños y posiciones de ventana guardados por aplicación desde ~/.config/malus-de/window_sizes.json,
    /// manteniendo compatibilidad con entradas antiguas de 2 elementos `[w, h]` y nuevas de 4 elementos `[w, h, x, y]`.
    pub fn load_saved_window_geometries() -> (
        HashMap<String, Size<i32, Logical>>,
        HashMap<String, Point<i32, Logical>>,
    ) {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let path = std::path::PathBuf::from(home).join(".config/malus-de/window_sizes.json");
        let mut sizes = HashMap::new();
        let mut positions = HashMap::new();
        if path.exists() {
            if let Ok(data) = std::fs::read_to_string(&path) {
                if let Ok(raw) = serde_json::from_str::<HashMap<String, Vec<i32>>>(&data) {
                    for (k, vals) in raw {
                        if !should_persist_window_geometry(&k) {
                            continue;
                        }
                        if vals.len() >= 2 && vals[0] >= 200 && vals[1] >= 200 {
                            sizes.insert(k.clone(), Size::from((vals[0], vals[1])));
                        }
                        if vals.len() >= 4 {
                            positions.insert(k, Point::from((vals[2], vals[3])));
                        }
                    }
                    tracing::info!(
                        "📂 Geometrías de ventana cargadas desde disco: {} tamaños, {} posiciones",
                        sizes.len(),
                        positions.len()
                    );
                }
            }
        }
        (sizes, positions)
    }

    /// Guarda el último tamaño y posición visible conocidos de una aplicación y los persiste en disco.
    pub fn save_app_window_geometry(
        &mut self,
        app_id: &str,
        size: Size<i32, Logical>,
        visible_loc: Option<Point<i32, Logical>>,
    ) {
        if size.w < 200 || size.h < 200 || !should_persist_window_geometry(app_id) {
            return;
        }
        let clean_id = normalize_app_id(app_id);
        self.saved_app_window_sizes.insert(clean_id.clone(), size);
        if let Some(loc) = visible_loc {
            self.saved_app_window_positions.insert(clean_id, loc);
        }
        self.persist_saved_window_geometries();
    }

    fn persist_saved_window_geometries(&self) {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let config_dir = std::path::PathBuf::from(home).join(".config/malus-de");
        let _ = std::fs::create_dir_all(&config_dir);
        let path = config_dir.join("window_sizes.json");
        let mut map: HashMap<String, Vec<i32>> = HashMap::new();
        for (k, s) in &self.saved_app_window_sizes {
            if let Some(p) = self.saved_app_window_positions.get(k) {
                map.insert(k.clone(), vec![s.w, s.h, p.x, p.y]);
            } else {
                map.insert(k.clone(), vec![s.w, s.h]);
            }
        }
        if let Ok(json) = serde_json::to_string_pretty(&map) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Obtiene el tamaño recordado de una aplicación si existe.
    pub fn get_saved_window_size(&self, app_id: &str) -> Option<Size<i32, Logical>> {
        if !should_persist_window_geometry(app_id) {
            return None;
        }
        let clean_id = normalize_app_id(app_id);
        self.saved_app_window_sizes.get(&clean_id).copied()
    }

    /// Obtiene la posición visible recordada de una aplicación si existe.
    pub fn get_saved_window_position(&self, app_id: &str) -> Option<Point<i32, Logical>> {
        if !should_persist_window_geometry(app_id) {
            return None;
        }
        let clean_id = normalize_app_id(app_id);
        self.saved_app_window_positions.get(&clean_id).copied()
    }

    pub fn get_or_assign_window_id(&mut self, surface: &WlSurface) -> u64 {
        if let Some(&id) = self.window_ids.get(surface) {
            id
        } else {
            self.next_window_id += 1;
            let id = self.next_window_id;
            self.window_ids.insert(surface.clone(), id);
            id
        }
    }

    pub fn notify_minimized_windows(&self) {
        let mut list = Vec::new();
        for (surface, _) in &self.minimized_windows {
            if is_system_action_confirmation_surface(surface) {
                continue;
            }
            if let Some(&window_id) = self.window_ids.get(surface) {
                let (app_id, title) = toplevel_info(surface);
                let app_id_str = app_id.unwrap_or_else(|| "app".to_string());
                let title_str = title.unwrap_or_else(|| app_id_str.clone());
                let icon_name = guess_icon_name(&app_id_str, &title_str);
                let is_animating = self
                    .active_animations
                    .iter()
                    .any(|anim| anim.window_id == window_id && anim.is_minimizing);
                list.push(malus_de_common::ipc::MinimizedWindowInfo {
                    window_id,
                    app_id: app_id_str,
                    title: title_str,
                    icon_name: Some(icon_name),
                    thumbnail_path: self.minimized_window_thumbnails.get(&window_id).cloned(),
                    is_animating,
                });
            }
        }
        self.ipc_broadcaster
            .broadcast(&IpcMessage::MinimizedWindowsChanged { windows: list });
    }

    pub fn remove_minimized_thumbnail(&mut self, window_id: u64) {
        if let Some(path) = self.minimized_window_thumbnails.remove(&window_id) {
            if let Err(error) = std::fs::remove_file(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!("No se pudo eliminar la miniatura {}: {error}", path);
                }
            }
        }
    }

    pub fn update_spotlight_drag_dock_hover(
        &mut self,
        point: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
        if !self.spotlight.is_dragging_app() {
            if self.spotlight.clear_dock_gap_preview() {
                self.ipc_broadcaster
                    .broadcast(&IpcMessage::PreviewDockDropGap { dock_x: None });
            }
            return;
        }

        let Some(output_geo) = self.spotlight_output_geometry() else {
            return;
        };
        let dock_geo = self.dock_surface_geometry().map(|(geo, _)| geo);
        let gap_x = self
            .spotlight
            .compute_dock_hover_gap(output_geo, point, dock_geo);

        if self.spotlight.set_dock_gap_preview(gap_x) {
            self.ipc_broadcaster
                .broadcast(&IpcMessage::PreviewDockDropGap { dock_x: gap_x });
        }
    }

    pub fn handle_spotlight_pointer_release(
        &self,
        spot_output_geo: Rectangle<i32, Logical>,
        outcome: crate::spotlight_visual::SpotlightPointerReleaseOutcome,
    ) {
        match outcome {
            crate::spotlight_visual::SpotlightPointerReleaseOutcome::PinToDock {
                app,
                insert_at_dock_x,
            } => {
                tracing::info!(
                    "📌 Aplicación arrastrada desde Spotlight al Dock: '{}' (dock_x={:?})",
                    app.name,
                    insert_at_dock_x
                );
                self.ipc_broadcaster.broadcast(&IpcMessage::PinAppToDock {
                    name: app.name,
                    icon: app.icon,
                    command: app.command,
                    matchers: app.matchers,
                    insert_at_dock_x,
                });
            }
            crate::spotlight_visual::SpotlightPointerReleaseOutcome::CreateDesktopShortcut {
                app,
                drop_loc,
            } => {
                if app.desktop_file.trim().is_empty()
                    || self.space.element_under(drop_loc).is_some()
                {
                    return;
                }
                let local_x = drop_loc.x - spot_output_geo.loc.x as f64;
                let local_y = drop_loc.y - spot_output_geo.loc.y as f64;
                if local_y < 36.0 {
                    return;
                }
                tracing::info!(
                    "🔗 Acceso directo creado en escritorio desde Spotlight: '{}' ({:.1}, {:.1})",
                    app.name,
                    local_x,
                    local_y
                );
                self.ipc_broadcaster
                    .broadcast(&IpcMessage::CreateDesktopAppShortcut {
                        desktop_file: app.desktop_file,
                        drop_x: local_x,
                        drop_y: local_y,
                    });
            }
            crate::spotlight_visual::SpotlightPointerReleaseOutcome::Consumed => {}
        }
    }

    pub fn notify_running_apps(&self) {
        let mut apps = Vec::new();
        for window in self.space.elements() {
            if let Some(surface) = window.wl_surface() {
                if is_system_action_confirmation_surface(&surface) {
                    continue;
                }
                let (app_id, title) = toplevel_info(&surface);
                let app = app_id
                    .filter(|id| !id.trim().is_empty())
                    .or_else(|| title.filter(|t| !t.trim().is_empty()));
                if let Some(app_name) = app {
                    if !apps.contains(&app_name) {
                        apps.push(app_name);
                    }
                }
            }
        }
        for (surface, _) in &self.minimized_windows {
            if is_system_action_confirmation_surface(surface) {
                continue;
            }
            let (app_id, title) = toplevel_info(surface);
            let app = app_id
                .filter(|id| !id.trim().is_empty())
                .or_else(|| title.filter(|t| !t.trim().is_empty()));
            if let Some(app_name) = app {
                if !apps.contains(&app_name) {
                    apps.push(app_name);
                }
            }
        }
        self.ipc_broadcaster
            .broadcast(&IpcMessage::RunningAppsChanged { running_apps: apps });
    }

    pub fn notify_active_app(&self, surface: Option<&WlSurface>) {
        let (app_id, title) = if let Some(surf) = surface {
            toplevel_info(surf)
        } else {
            (None, None)
        };
        let window_id = surface.and_then(|surf| self.window_ids.get(surf).copied());
        self.ipc_broadcaster
            .broadcast(&IpcMessage::ActiveAppChanged {
                app_id,
                title,
                window_id,
            });
    }

    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<smithay::desktop::Window> {
        self.space
            .elements()
            .find(|window| {
                if window.wl_surface().as_deref() == Some(surface) {
                    return true;
                }
                let mut found = false;
                window.with_surfaces(|s, _| {
                    if s == surface {
                        found = true;
                    }
                });
                found
            })
            .cloned()
            .or_else(|| {
                self.minimized_windows
                    .values()
                    .find(|(win, _)| win.wl_surface().as_deref() == Some(surface))
                    .map(|(win, _)| win.clone())
            })
            .or_else(|| self.workspaces.window_for_surface(surface))
    }

    pub fn output_commit_counter(&self, output_name: &str) -> usize {
        self.output_commit_counters
            .get(output_name)
            .copied()
            .unwrap_or(0)
    }

    pub fn bump_output_commit_counter(&mut self, output_name: &str) {
        let entry = self
            .output_commit_counters
            .entry(output_name.to_string())
            .or_insert(0);
        *entry = entry.wrapping_add(1);
    }

    pub fn dock_surface_geometry(&self) -> Option<(Rectangle<i32, Logical>, Output)> {
        for output in self.space.outputs() {
            let output_geo = self.space.output_geometry(output).unwrap_or_default();
            let layer_map = layer_map_for_output(output);
            for layer in layer_map.layers() {
                if layer.namespace() == "malus-dock" {
                    if let Some(mut geo) = layer_map.layer_geometry(layer) {
                        geo.loc += output_geo.loc;
                        return Some((geo, output.clone()));
                    }
                }
            }
        }
        None
    }

    pub fn resume_timed_out_dock_animations(&mut self, now: Instant) {
        const DOCK_SLOT_TIMEOUT: Duration = Duration::from_millis(300);
        let mut windows_to_unmap = Vec::new();
        for anim in &mut self.active_animations {
            if anim.waiting_for_dock_slot
                && now.duration_since(anim.start_time) >= DOCK_SLOT_TIMEOUT
            {
                tracing::warn!(
                    "No se recibió el slot del Dock para id={}; usando destino aproximado",
                    anim.window_id
                );
                anim.waiting_for_dock_slot = false;
                anim.start_time = now;
                windows_to_unmap.push(anim.window.clone());
            }
        }
        for window in windows_to_unmap {
            self.space.unmap_elem(&window);
        }
    }

    fn fallback_dock_slot(&self) -> Rectangle<i32, Logical> {
        use malus_de_common::config::DockPosition;
        if let Some((dock_geo, _)) = self.dock_surface_geometry() {
            let (slot_x, slot_y) = match self.dock_cfg.position {
                DockPosition::Bottom => (
                    (dock_geo.loc.x + dock_geo.size.w - 110).max(dock_geo.loc.x),
                    (dock_geo.loc.y + dock_geo.size.h - 58).max(dock_geo.loc.y),
                ),
                DockPosition::Left => (
                    dock_geo.loc.x + 12,
                    dock_geo.loc.y + (dock_geo.size.h / 2) - 22,
                ),
                DockPosition::Right => (
                    (dock_geo.loc.x + dock_geo.size.w - 66).max(dock_geo.loc.x),
                    dock_geo.loc.y + (dock_geo.size.h / 2) - 22,
                ),
            };
            Rectangle::new((slot_x, slot_y).into(), (54, 44).into())
        } else {
            let output_size = self
                .space
                .outputs()
                .next()
                .and_then(|o| self.space.output_geometry(o))
                .map(|g| g.size)
                .unwrap_or_else(|| (1920, 1080).into());
            let (slot_x, slot_y) = match self.dock_cfg.position {
                DockPosition::Bottom => ((output_size.w / 2) + 80, output_size.h - 58),
                DockPosition::Left => (12, (output_size.h / 2) - 22),
                DockPosition::Right => (output_size.w - 66, (output_size.h / 2) - 22),
            };
            Rectangle::new((slot_x, slot_y).into(), (54, 44).into())
        }
    }

    pub fn restore_window_by_id(&mut self, target_id: u64) -> bool {
        let entry = self
            .minimized_windows
            .iter()
            .find(|(surf, _)| self.window_ids.get(*surf) == Some(&target_id))
            .map(|(surf, _)| surf.clone());

        if let Some(surf) = entry {
            if self.active_animations.iter().any(|a| a.surface == surf) {
                tracing::warn!("⚠️ Ventana id={} ya tiene una animación activa; ignorando restauración duplicada", target_id);
                return false;
            }

            if let Some((window, loc)) = self.minimized_windows.remove(&surf) {
                self.minimized_window_workspaces.remove(&surf);
                self.remove_minimized_thumbnail(target_id);
                let geo = window.geometry();
                let dock_slot = self
                    .dock_slots
                    .remove(&target_id)
                    .unwrap_or_else(|| self.fallback_dock_slot());

                // Iniciar animación Genie Effect inversa (expansión desde el Dock)
                let anim = GenieAnimation {
                    window_id: target_id,
                    surface: surf.clone(),
                    window: window.clone(),
                    start_time: std::time::Instant::now(),
                    duration: std::time::Duration::from_millis(500),
                    is_minimizing: false,
                    waiting_for_dock_slot: false,
                    window_rect: Rectangle::new(loc, geo.size),
                    dock_slot_rect: dock_slot,
                    original_location: loc,
                };
                self.active_animations.push(anim);

                tracing::info!(
                    "✨ Iniciando animación de restauración Genie para ventana id={}",
                    target_id
                );
                self.notify_running_apps();
                self.notify_minimized_windows();
                return true;
            }
        }
        false
    }

    pub fn minimize_toplevel(&mut self, surface: &ToplevelSurface) {
        tracing::info!("🪟 [XdgShell] minimize_request recibido del cliente");
        self.snap_layouts.close();
        let surf = surface.wl_surface().clone();

        // 1. Evitar minimizar si ya está en la lista de minimizadas
        if self.minimized_windows.contains_key(&surf) {
            tracing::warn!("⚠️ Intento de minimizar ventana que ya está minimizada; ignorando");
            return;
        }

        // 2. Evitar minimizar si ya tiene una animación activa en curso
        if self.active_animations.iter().any(|a| a.surface == surf) {
            tracing::warn!("⚠️ Intento de minimizar ventana con animación en curso; ignorando");
            return;
        }

        // 3. Solo minimizar si está mapeada actualmente en Space
        let target_window = self
            .space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(&surf))
            .cloned();
        if let Some(window) = target_window {
            let loc = self.space.element_location(&window).unwrap_or_default();
            let geo = window.geometry();
            let window_id = self.get_or_assign_window_id(&surf);

            // Determinar slot de destino en el Dock (estimado en el extremo derecho de minimizadas
            // hasta que RegisterDockSlot entregue el centro exacto de la tarjeta).
            // Nunca dejar la ventana mapeada esperando al Dock: al haber perdido el foco, el cambio
            // de sombra/estado activo en la ventana viva causaría parpadeo antes de iniciar el Genie.
            let registered_dock_slot = self.dock_slots.get(&window_id).cloned();
            let waiting_for_dock_slot = false;
            let dock_slot = registered_dock_slot.unwrap_or_else(|| self.fallback_dock_slot());

            // Registrar animación Genie Effect (Lámpara de Aladino hacia el Dock)
            let anim = GenieAnimation {
                window_id,
                surface: surf.clone(),
                window: window.clone(),
                start_time: std::time::Instant::now(),
                duration: std::time::Duration::from_millis(520),
                is_minimizing: true,
                waiting_for_dock_slot,
                window_rect: Rectangle::new(loc, geo.size),
                dock_slot_rect: dock_slot,
                original_location: loc,
            };
            self.active_animations.push(anim);

            self.minimized_windows
                .insert(surf.clone(), (window.clone(), loc));
            self.minimized_window_workspaces
                .insert(surf.clone(), self.workspaces.active_index);
            self.space.unmap_elem(&window);

            let next_focus = self
                .space
                .elements()
                .filter(|candidate| candidate.wl_surface().as_deref() != Some(&surf))
                .last()
                .and_then(|w| w.wl_surface().map(|s| s.into_owned()));
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            self.set_keyboard_focus_with_sync(next_focus.clone(), serial);
            if let Some(next_win) = self.space.elements().last() {
                if let Some(toplevel) = next_win.toplevel() {
                    toplevel.send_configure();
                }
            }
            self.notify_running_apps();
            self.notify_minimized_windows();
            self.notify_active_app(next_focus.as_ref());
            if self.auto_tiling_enabled {
                self.relayout_auto_tiling();
            }
        }
    }

    pub fn activate_or_restore_app(&mut self, target: &str) -> bool {
        let target_lower = target.to_lowercase();

        // 1. Verificar si está entre las ventanas minimizadas
        let minimized_entry = self
            .minimized_windows
            .iter()
            .find(|(surf, _)| {
                let (app_id, title) = toplevel_info(surf);
                matches_app_target(&target_lower, app_id.as_deref(), title.as_deref())
            })
            .map(|(surf, _)| surf.clone());

        if let Some(surf) = minimized_entry {
            if let Some(&id) = self.window_ids.get(&surf) {
                return self.restore_window_by_id(id);
            }
        }

        // 2. Verificar si está entre las ventanas activas en Space
        let visible_window = self
            .space
            .elements()
            .find(|w| {
                if let Some(surf) = w.wl_surface() {
                    let (app_id, title) = toplevel_info(&surf);
                    matches_app_target(&target_lower, app_id.as_deref(), title.as_deref())
                } else {
                    false
                }
            })
            .cloned();

        if let Some(window) = visible_window {
            let is_already_focused = self
                .seat
                .get_keyboard()
                .and_then(|kb| kb.current_focus())
                .map(|focused| {
                    window
                        .wl_surface()
                        .map(|surface| surface.as_ref() == &focused)
                        .unwrap_or(false)
                })
                .unwrap_or(false);

            self.space.raise_element(&window, true);
            self.ensure_window_below_topbar(&window);

            if is_already_focused {
                self.sync_window_activation(window.wl_surface().as_deref());
                tracing::debug!("La app solicitada ya tiene el foco; asegurando orden superior");
                return true;
            }

            if let Some(surf) = window.wl_surface() {
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                self.set_keyboard_focus_with_sync(Some(surf.clone().into_owned()), serial);
                self.notify_active_app(Some(&surf));
            }
            return true;
        }

        false
    }

    pub fn ipc_message_allowed_when_locked(msg: &IpcMessage) -> bool {
        matches!(
            msg,
            IpcMessage::StopScreenRecording
                | IpcMessage::HideSpotlight
                | IpcMessage::HideAppSwitcher
                | IpcMessage::ReloadDisplaysConfig
                | IpcMessage::ReloadDisplayBrightness
                | IpcMessage::ThemeTransitionPrepared { .. }
        )
    }

    pub fn process_ipc_messages(&mut self) {
        while let Some(msg) = self.ipc_broadcaster.try_recv() {
            if self.is_session_locked() && !Self::ipc_message_allowed_when_locked(&msg) {
                tracing::warn!(
                    "🔒 [SECURITY] Mensaje IPC bloqueado mientras la sesión está bloqueada: {:?}",
                    std::mem::discriminant(&msg)
                );
                continue;
            }
            match msg {
                IpcMessage::ActivateApp { app_id } => {
                    tracing::info!("📥 IPC ActivateApp recibido para: {}", app_id);
                    self.activate_or_restore_app(&app_id);
                }
                IpcMessage::CaptureScreenshot { destination } => {
                    tracing::info!("Solicitud IPC de captura recibida");
                    self.pending_screenshot_requests
                        .push_back((std::path::PathBuf::from(destination), None));
                }
                IpcMessage::StartScreenRecording {
                    destination,
                    rect,
                    include_audio,
                    fps,
                    lossless,
                } => {
                    tracing::info!(
                        "📥 Solicitud IPC de inicio de grabación recibida: {} (lossless={})",
                        destination,
                        lossless
                    );
                    self.start_screen_recording(
                        std::path::PathBuf::from(destination),
                        rect,
                        include_audio,
                        fps,
                        lossless,
                    );
                }
                IpcMessage::StopScreenRecording => {
                    tracing::info!("📥 Solicitud IPC de parada de grabación recibida");
                    self.stop_screen_recording();
                }
                IpcMessage::TriggerScreenshot {
                    interactive: _,
                    record_video,
                } => {
                    self.start_interactive_screenshot();
                    if record_video {
                        self.screenshot_ui.mode =
                            crate::screenshot_ui::ScreenshotUiMode::RecordArea;
                    }
                }
                IpcMessage::CaptureScreenshotForOutput {
                    destination,
                    output_name,
                } => {
                    tracing::info!("Solicitud IPC de captura recibida para {output_name}");
                    self.pending_screenshot_requests
                        .push_back((std::path::PathBuf::from(destination), Some(output_name)));
                }
                IpcMessage::CaptureWindowScreenshot {
                    destination,
                    window_id,
                    output_name,
                } => {
                    tracing::info!(
                        "Solicitud IPC de captura aislada de ventana recibida (id={window_id})"
                    );
                    self.pending_window_screenshot_requests.push_back((
                        std::path::PathBuf::from(destination),
                        window_id,
                        output_name,
                    ));
                }
                IpcMessage::TriggerOcrCapture => {
                    tracing::info!("📥 IPC TriggerOcrCapture recibido");
                    self.start_ocr_screenshot();
                }
                IpcMessage::PrepareThemeTransition { request_id } => {
                    let origin = self
                        .seat
                        .get_pointer()
                        .map(|pointer| pointer.current_location())
                        .unwrap_or_else(|| Point::from((0.0, 0.0)));
                    self.theme_transition_started_at = None;
                    self.pending_theme_transition_capture =
                        Some(ThemeTransitionCapture { request_id, origin });
                }
                IpcMessage::ThemeChanged {
                    theme_name,
                    is_dark,
                } => {
                    tracing::info!(
                        "📥 IPC ThemeChanged recibido: tema='{}', oscuro={}",
                        theme_name,
                        is_dark
                    );
                    self.theme_transition_started_at = Some(Instant::now());
                    self.ipc_broadcaster.broadcast(&IpcMessage::ThemeChanged {
                        theme_name,
                        is_dark,
                    });
                }
                IpcMessage::AccentColorChanged { index } => {
                    tracing::info!("📥 IPC AccentColorChanged recibido: índice={}", index);
                    self.ipc_broadcaster
                        .broadcast(&IpcMessage::AccentColorChanged { index });
                }
                IpcMessage::LiquidGlassOpacityChanged { opacity } => {
                    tracing::info!(
                        "📥 IPC LiquidGlassOpacityChanged recibido: opacidad={:.2}",
                        opacity
                    );
                    self.surface_commit_counter = self.surface_commit_counter.wrapping_add(1);
                    crate::blur::set_reduce_transparency(
                        malus_de_common::config::load_reduce_transparency(),
                    );
                    self.expose.bump_revision();
                    self.ipc_broadcaster
                        .broadcast(&IpcMessage::LiquidGlassOpacityChanged { opacity });
                }
                IpcMessage::WindowButtonsLayoutChanged { right } => {
                    tracing::info!(
                        "📥 IPC WindowButtonsLayoutChanged recibido: derecha={}",
                        right
                    );
                    malus_de_common::config::invalidate_appearance_settings_cache();
                    self.ssd.set_buttons_right(right);
                    self.surface_commit_counter = self.surface_commit_counter.wrapping_add(1);
                    self.expose.bump_revision();
                    self.ipc_broadcaster
                        .broadcast(&IpcMessage::WindowButtonsLayoutChanged { right });
                }
                IpcMessage::AnimateRestore { window_id } => {
                    tracing::info!(
                        "📥 IPC AnimateRestore recibido para ventana id={}",
                        window_id
                    );
                    self.restore_window_by_id(window_id);
                }
                IpcMessage::RegisterDockSlot {
                    window_id,
                    x,
                    y,
                    width,
                    height,
                } => {
                    tracing::info!(
                        "📥 IPC RegisterDockSlot recibido: id={}, rect=({}, {}, {}, {})",
                        window_id,
                        x,
                        y,
                        width,
                        height
                    );
                    let dock_origin = self
                        .dock_surface_geometry()
                        .map(|(dock_geo, _)| dock_geo.loc)
                        .unwrap_or_default();
                    let final_rect = Rectangle::new(
                        (dock_origin.x + x, dock_origin.y + y).into(),
                        (width as i32, height as i32).into(),
                    );

                    tracing::info!(
                        "🎯 Dock slot ajustado con precisión: id={}, rect={:?}",
                        window_id,
                        final_rect
                    );
                    self.dock_slots.insert(window_id, final_rect);
                    let mut windows_to_unmap = Vec::new();
                    for anim in &mut self.active_animations {
                        if anim.window_id != window_id || !anim.is_minimizing {
                            continue;
                        }

                        anim.dock_slot_rect = final_rect;
                        if anim.waiting_for_dock_slot {
                            anim.start_time = Instant::now();
                            anim.waiting_for_dock_slot = false;
                            windows_to_unmap.push(anim.window.clone());
                            tracing::info!(
                                "🎯 Destino real del Dock recibido; iniciando minimización Genie para id={}",
                                window_id
                            );
                        }
                    }
                    for window in windows_to_unmap {
                        self.space.unmap_elem(&window);
                    }
                }
                IpcMessage::ToggleSpotlight => {
                    tracing::info!("📥 IPC ToggleSpotlight recibido");
                    self.toggle_spotlight();
                }
                IpcMessage::SwitchWorkspace { index } => {
                    tracing::info!("📥 IPC SwitchWorkspace recibido: {}", index);
                    self.switch_workspace(index);
                }
                IpcMessage::ToggleMissionControl => {
                    tracing::info!("📥 IPC ToggleMissionControl recibido");
                    self.toggle_expose(crate::expose::ExposeMode::Global);
                }
                IpcMessage::MissionControlFilter { query } => {
                    if self.expose.active {
                        self.expose.apply_filter(query);
                    }
                }
                IpcMessage::HideSpotlight => {
                    self.spotlight.close();
                    self.ipc_broadcaster.broadcast(&IpcMessage::HideSpotlight);
                }
                IpcMessage::HideActiveWindow => {
                    tracing::info!("📥 IPC HideActiveWindow recibido");
                    self.minimize_active_window();
                }
                IpcMessage::HideOtherWindows => {
                    tracing::info!("📥 IPC HideOtherWindows recibido");
                    self.minimize_other_windows();
                }
                IpcMessage::CloseActiveWindow => {
                    tracing::info!("📥 IPC CloseActiveWindow recibido");
                    self.close_active_window();
                }
                IpcMessage::ReloadDisplaysConfig => {
                    tracing::info!("📥 IPC ReloadDisplaysConfig recibido — marcando recarga inmediata de pantallas");
                    self.pending_displays_reload = true;
                    self.ipc_broadcaster
                        .broadcast(&IpcMessage::ReloadDisplaysConfig);
                }
                IpcMessage::ReloadDisplayBrightness => {
                    self.pending_brightness_reload = true;
                }
                IpcMessage::ThemeTransitionPrepared { .. } => {}
                IpcMessage::SetKeyboardLayout { layout, variant } => {
                    tracing::info!(
                        "📥 IPC SetKeyboardLayout recibido: layout='{}', variant='{}'",
                        layout,
                        variant
                    );
                    self.apply_keyboard_layout(&layout, &variant);
                }
                IpcMessage::SwitchToWindow { window_id } => {
                    tracing::info!("📥 IPC SwitchToWindow recibido para: {}", window_id);
                    self.dismiss_app_switcher_immediately();
                    self.switch_to_window(window_id);
                }
                IpcMessage::HideAppSwitcher => {
                    tracing::info!("📥 IPC HideAppSwitcher recibido");
                    self.dismiss_app_switcher_immediately();
                }
                IpcMessage::ReloadDockConfig => {
                    tracing::info!(
                        "📥 IPC ReloadDockConfig recibido — retransmitiendo a clientes shell"
                    );
                    self.reload_dock_config_fields();
                    self.ipc_broadcaster
                        .broadcast(&IpcMessage::ReloadDockConfig);
                }
                IpcMessage::ReloadMenuBarConfig => {
                    tracing::info!(
                        "📥 IPC ReloadMenuBarConfig recibido — retransmitiendo a clientes shell"
                    );
                    self.ipc_broadcaster
                        .broadcast(&IpcMessage::ReloadMenuBarConfig);
                }
                IpcMessage::SetControlCenterGlassRegions { regions } => {
                    self.control_center_glass_regions = regions;
                    self.surface_commit_counter = self.surface_commit_counter.wrapping_add(1);
                }
                IpcMessage::ToggleAutoTiling => {
                    tracing::info!("📥 IPC ToggleAutoTiling recibido — alternando mosaico de ventanas");
                    self.toggle_auto_tiling();
                }
                IpcMessage::ToggleShowDesktop => {
                    tracing::info!("📥 IPC ToggleShowDesktop recibido — alternando mostrar escritorio");
                    self.toggle_show_desktop();
                }
                IpcMessage::ReloadDesktopDockConfig => {
                    tracing::info!("📥 IPC ReloadDesktopDockConfig recibido");
                    self.desktop_dock_cfg = malus_de_common::config::DesktopDockConfig::load();
                    if !self.desktop_dock_cfg.black_hole_close_effect {
                        self.cached_close_snapshots.clear();
                        self.pending_black_holes.clear();
                    }
                    self.ipc_broadcaster
                        .broadcast(&IpcMessage::ReloadDesktopDockConfig);
                }
                IpcMessage::ReloadWallpaperConfig => {
                    tracing::info!(
                        "📥 IPC ReloadWallpaperConfig recibido — retransmitiendo a clientes shell"
                    );
                    self.ipc_broadcaster
                        .broadcast(&IpcMessage::ReloadWallpaperConfig);
                }
                IpcMessage::ReloadInputConfig => {
                    tracing::info!("📥 IPC ReloadInputConfig recibido");
                    self.input_cfg = malus_de_common::config::InputConfig::load();
                    self.input_config_dirty = true;
                    if let Some(kb) = self.seat.get_keyboard() {
                        kb.change_repeat_info(
                            self.input_cfg.keyboard_repeat_rate.clamp(0, 100),
                            self.input_cfg.keyboard_repeat_delay_ms.clamp(150, 1500),
                        );
                    }
                    self.ipc_broadcaster
                        .broadcast(&IpcMessage::ReloadInputConfig);
                }
                msg @ (IpcMessage::PinAppToDock { .. } | IpcMessage::UnpinAppFromDock { .. }) => {
                    self.ipc_broadcaster.broadcast(&msg);
                }
                IpcMessage::StartDockIconDrag { name, icon } => {
                    let loc = self
                        .seat
                        .get_pointer()
                        .map(|p| p.current_location())
                        .unwrap_or_default();
                    self.spotlight.start_dock_icon_drag(name, icon, loc);
                }
                IpcMessage::EndDockIconDrag { poof } => {
                    let loc = self.seat.get_pointer().map(|p| p.current_location());
                    self.spotlight.end_dock_icon_drag(poof, loc);
                }
                _ => {}
            }
        }
    }

    /// Alterna Spotlight vinculándolo exclusivamente al monitor donde fue invocado
    /// (el monitor bajo el cursor, o el de la ventana activa, o el primer monitor conectado).
    pub fn toggle_spotlight(&mut self) {
        self.snap_layouts.close();
        if self.spotlight.is_open() {
            self.spotlight.close();
            return;
        }
        let pointer_loc = self.seat.get_pointer().map(|p| p.current_location());
        let output_name = pointer_loc
            .and_then(|pt| {
                self.space
                    .outputs()
                    .find(|o| {
                        self.space
                            .output_geometry(o)
                            .map(|g| g.to_f64().contains(pt))
                            .unwrap_or(false)
                    })
                    .map(|o| o.name())
            })
            .or_else(|| {
                self.get_active_window()
                    .and_then(|w| self.space.outputs_for_element(&w).into_iter().next())
                    .map(|o| o.name())
            })
            .or_else(|| self.space.outputs().next().map(|o| o.name()));

        tracing::info!("🔍 Abriendo Spotlight en pantalla: {:?}", output_name);
        self.spotlight.open_on_output(output_name);
    }

    /// Devuelve la geometría lógica del monitor en el que Spotlight está abierto actualmente.
    pub fn spotlight_output_geometry(&self) -> Option<Rectangle<i32, Logical>> {
        if let Some(target_name) = self.spotlight.active_output_name() {
            if let Some(out) = self.space.outputs().find(|o| o.name() == target_name) {
                if let Some(geo) = self.space.output_geometry(out) {
                    return Some(geo);
                }
            }
        }
        self.primary_output()
            .and_then(|o| self.space.output_geometry(&o))
    }

    /// Cambia la distribución de teclado XKB en vivo para toda la sesión Wayland y persiste en input.toml
    pub fn apply_keyboard_layout(&mut self, layout: &str, variant: &str) {
        let mut input_cfg = malus_de_common::config::InputConfig::load();
        input_cfg.keyboard_layout = layout.to_string();
        input_cfg.keyboard_variant = variant.to_string();
        let _ = input_cfg.save();

        if let Some(kb) = self.seat.get_keyboard() {
            let xkb_config = smithay::input::keyboard::XkbConfig {
                rules: "",
                model: &input_cfg.keyboard_model,
                layout: &input_cfg.keyboard_layout,
                variant: &input_cfg.keyboard_variant,
                options: input_cfg.keyboard_options.clone(),
            };
            match kb.set_xkb_config(self, xkb_config) {
                Ok(()) => {
                    tracing::info!(
                        "⌨️ Distribución de teclado XKB aplicada en vivo: layout='{}' ({}), variant='{}'",
                        input_cfg.keyboard_layout,
                        input_cfg.layout_display_name(),
                        input_cfg.keyboard_variant
                    );
                }
                Err(e) => {
                    tracing::error!("❌ Error aplicando distribución XKB '{}': {:?}", layout, e);
                }
            }
        }

        self.ipc_broadcaster
            .broadcast(&IpcMessage::KeyboardLayoutChanged {
                layout: input_cfg.keyboard_layout,
                variant: input_cfg.keyboard_variant,
            });
    }

    pub fn get_active_window(&self) -> Option<smithay::desktop::Window> {
        if let Some(kb) = self.seat.get_keyboard() {
            if let Some(focus_surf) = kb.current_focus() {
                if let Some(win) = self.window_for_surface(&focus_surf) {
                    return Some(win);
                }
            }
        }
        self.space.elements().last().cloned()
    }

    fn close_button_offset_for_app_id(
        app_id: Option<&str>,
        is_firefox: bool,
        uses_ssd: bool,
    ) -> (f64, f64) {
        if uses_ssd {
            return (crate::ssd::BTN_CLOSE_X, crate::ssd::BTN_Y);
        }
        let app_lower = app_id.unwrap_or("").trim().to_ascii_lowercase();
        if app_lower == "dev.malus.branches" || app_lower == "dev.malus.settings" {
            return (20.5, 24.0);
        }
        if app_lower.contains("antigravity")
            || app_lower == "code"
            || app_lower.contains("vscode")
            || app_lower.contains("codium")
            || app_lower.contains("cursor")
        {
            return (20.5, 17.5);
        }
        if is_firefox {
            return (16.5, 21.0);
        }
        (22.0, 23.0)
    }

    fn close_button_center_logical(
        &self,
        window: &smithay::desktop::Window,
        loc: Point<i32, Logical>,
        uses_ssd: bool,
        is_fullscreen: bool,
        buttons_right: bool,
    ) -> Point<f64, Logical> {
        let top_y = if uses_ssd && !is_fullscreen {
            (loc.y - crate::ssd::SSD_TITLEBAR_HEIGHT) as f64
        } else {
            loc.y as f64
        };
        let app_id = window.wl_surface().and_then(|s| toplevel_info(&s).0);
        let learned = if !uses_ssd {
            app_id.as_deref().and_then(|id| {
                let key = id.trim().to_ascii_lowercase();
                if key.is_empty() {
                    None
                } else {
                    self.learned_close_button_offsets
                        .get(&(key, buttons_right))
                        .copied()
                }
            })
        } else {
            None
        };
        let (dx, dy) = learned.unwrap_or_else(|| {
            Self::close_button_offset_for_app_id(
                app_id.as_deref(),
                is_firefox_window(window),
                uses_ssd,
            )
        });
        let x = if buttons_right {
            let win_w = window.geometry().size.w.max(0) as f64;
            if win_w > dx {
                loc.x as f64 + (win_w - dx)
            } else {
                loc.x as f64 + dx
            }
        } else {
            loc.x as f64 + dx
        };
        Point::from((x, top_y + dy))
    }

    fn record_close_button_hint(&mut self, window: &smithay::desktop::Window) {
        let Some(surf) = window.wl_surface() else {
            return;
        };
        let Some(loc) = self.space.element_location(window) else {
            return;
        };
        let uses_ssd = window_uses_ssd(window);
        let is_fullscreen = window
            .toplevel()
            .map(|t| t.current_state().states.contains(XdgState::Fullscreen))
            .unwrap_or(false);
        let origin = self.close_button_center_logical(
            window,
            loc,
            uses_ssd,
            is_fullscreen,
            self.ssd.buttons_right,
        );
        self.close_button_hints
            .insert(surf.into_owned(), (origin, Instant::now(), false));
    }

    /// Determina si una coordenada relativa `(rel_x, rel_y)` dentro de una ventana CSD de ancho `win_w`
    /// cae dentro de la zona superior de botones de cierre (izquierda o derecha).
    fn is_in_csd_close_zone(rel_x: f64, rel_y: f64, win_w: f64) -> Option<bool> {
        if !(0.0..=56.0).contains(&rel_y) {
            return None;
        }
        let edge_span = 72.0_f64.min((win_w * 0.35).max(48.0));
        if (0.0..=edge_span).contains(&rel_x) {
            return Some(false);
        }
        if win_w >= edge_span && rel_x >= (win_w - edge_span) && rel_x <= win_w {
            return Some(true);
        }
        None
    }

    pub fn record_csd_close_click_hint(
        &mut self,
        window: &smithay::desktop::Window,
        click_pt: Point<f64, Logical>,
    ) {
        if window_uses_ssd(window) {
            return;
        }
        let Some(surf) = window.wl_surface() else {
            return;
        };
        let Some(win_loc) = self.space.element_location(window) else {
            return;
        };
        let geo = window.geometry();
        let rel_x = click_pt.x - win_loc.x as f64;
        let rel_y = click_pt.y - win_loc.y as f64;
        if Self::is_in_csd_close_zone(rel_x, rel_y, geo.size.w as f64).is_some() {
            self.close_button_hints
                .insert(surf.into_owned(), (click_pt, Instant::now(), true));
        }
    }

    pub fn close_active_window(&mut self) {
        if let Some(win) = self.get_active_window() {
            self.record_close_button_hint(&win);
            if let Some(toplevel) = win.toplevel() {
                tracing::info!("📤 Cerrando ventana activa vía toplevel.send_close()");
                toplevel.send_close();
            }
        }
    }

    pub fn minimize_active_window(&mut self) {
        if let Some(win) = self.get_active_window() {
            if let Some(toplevel) = win.toplevel().cloned() {
                tracing::info!("📤 Minimizando ventana activa vía minimize_toplevel()");
                self.minimize_toplevel(&toplevel);
            }
        }
    }

    pub fn minimize_other_windows(&mut self) {
        let Some(active) = self.get_active_window() else {
            return;
        };
        let active_surface = active.wl_surface().map(|s| s.into_owned());
        let active_app_id = active_surface
            .as_ref()
            .and_then(|s| toplevel_info(s).0)
            .map(|id| id.trim().to_ascii_lowercase())
            .filter(|id| !id.is_empty());

        let toplevels_to_hide: Vec<ToplevelSurface> = self
            .space
            .elements()
            .filter_map(|win| {
                let surf = win.wl_surface()?;
                if active_surface.as_ref() == Some(surf.as_ref()) {
                    return None;
                }
                if let Some(ref keep_app) = active_app_id {
                    if let Some(other_app) = toplevel_info(surf.as_ref()).0 {
                        if other_app.trim().to_ascii_lowercase() == *keep_app {
                            return None;
                        }
                    }
                }
                win.toplevel().cloned()
            })
            .collect();

        for toplevel in toplevels_to_hide {
            self.minimize_toplevel(&toplevel);
        }
    }

    pub fn dismiss_all_popups(&mut self) {
        if let Some(mut grab) = self.popup_grab.take() {
            let _ = grab.ungrab(smithay::desktop::PopupUngrabStrategy::All);
        }
        if self.active_popups.is_empty() {
            return;
        }
        tracing::info!("🧹 Descartando {} popups activos", self.active_popups.len());
        for popup in self.active_popups.drain(..).rev() {
            let _ = popup.send_popup_done();
            if let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) {
                let _ = PopupManager::dismiss_popup(&root, &PopupKind::Xdg(popup));
            }
        }
        self.popups.cleanup();
    }

    /// Libera grabs de popup (cuando el clic ocurre fuera de ellos) o grabs residuales antes de
    /// aplicar el foco a la ventana o superficie objetivo.
    pub fn prepare_pointer_press_grabs(
        &mut self,
        clicked_on_popup: bool,
        serial: Serial,
        time_msec: u32,
    ) {
        let has_any_popup = !self.active_popups.is_empty() || self.popup_grab.is_some();
        if !clicked_on_popup && has_any_popup {
            // Soltar primero el grab de teclado para que `PopupPointerGrab::unset` no devuelva
            // el foco a `self.root` y pise el foco de la ventana que se va a seleccionar.
            if let Some(kb) = self.seat.get_keyboard() {
                if kb.is_grabbed() {
                    kb.unset_grab(self);
                }
            }
            if let Some(pointer) = self.seat.get_pointer() {
                if pointer.is_grabbed() {
                    self.snap_preview = None;
                    pointer.unset_grab(self, serial, time_msec);
                }
            }
            self.dismiss_all_popups();
            return;
        }

        if !self.popup_grab_active() {
            if let Some(pointer) = self.seat.get_pointer() {
                if pointer.is_grabbed() {
                    self.snap_preview = None;
                    pointer.unset_grab(self, serial, time_msec);
                }
            }
            if let Some(kb) = self.seat.get_keyboard() {
                if kb.is_grabbed() {
                    kb.unset_grab(self);
                }
            }
        }
    }

    /// Sincroniza el estado `Activated` de las ventanas en `Space`, difiriendo el `send_configure()`
    /// de activación cuando estamos en medio de `ButtonState::Pressed`.
    pub fn sync_window_activation(&mut self, focused: Option<&WlSurface>) {
        for win in self.space.elements() {
            let is_focused = focused
                .map(|s| win.wl_surface().as_deref() == Some(s))
                .unwrap_or(false);
            // Nota: `Window::set_activated` (así como `Space::raise_element(_, true)` y
            // `Space::map_element(_, _, true)`) mutan `pending_state.states` sin enviar configure.
            // Usar `toplevel.has_pending_changes()` compara `server_pending` contra el último
            // estado enviado (`current_server_state`), garantizando que nunca se pierda el
            // `xdg_toplevel.configure` de activación o desactivación.
            win.set_activated(is_focused);
            if let Some(toplevel) = win.toplevel() {
                let surf = toplevel.wl_surface().clone();
                if !is_focused {
                    self.pending_activation_configures.retain(|s| s != &surf);
                }
                if toplevel.has_pending_changes() {
                    if self.defer_activation_configure && is_focused {
                        if !self.pending_activation_configures.contains(&surf) {
                            self.pending_activation_configures.push(surf);
                        }
                    } else {
                        let _ = toplevel.send_pending_configure();
                    }
                }
            }
        }
    }

    /// Determina si una ventana debe renderizarse visualmente como activa (barra SSD y sombra),
    /// basándose en el foco real de teclado (incluyendo popups hijos de esa ventana).
    pub fn is_window_visually_active(
        &self,
        window: &smithay::desktop::Window,
        kb_focus: Option<&WlSurface>,
    ) -> bool {
        let Some(main_surface) = window.wl_surface() else {
            return false;
        };
        let Some(focused) = kb_focus else {
            return false;
        };
        if focused == main_surface.as_ref() {
            return true;
        }
        if let Some(popup) = self.popups.find_popup(focused) {
            if let Ok(root) = find_popup_root_surface(&popup) {
                return &root == main_surface.as_ref();
            }
        }
        false
    }

    /// Cambia el foco de teclado y garantiza la actualización de `Activated` incluso cuando `focus` es `None`
    /// (Smithay no invoca `SeatHandler::focus_changed` al limpiar el foco con `None`).
    /// Indica si la sesión está bloqueada sin adquirir `layer_map_for_output` (evita interbloqueos
    /// cuando el bucle de renderizado mantiene bloqueado el `Mutex<LayerMap>` de una pantalla).
    pub fn is_session_locked(&self) -> bool {
        self.ext_session_locked
            || self
                .malus_lock_layers
                .iter()
                .any(|(_, layer)| layer.alive())
    }

    /// Verifica la salud del bloqueo `ext-session-lock-v1` y sincroniza el indicador atómico
    /// compartido con el watchdog evdev. Si el cliente lockscreen muere sin desbloquear,
    /// mantiene el estado bloqueado (pantalla negra) y relanza `malus-de-session-lock`.
    pub fn check_session_lock_liveness(&mut self) {
        self.ext_lock_surfaces.retain(|_, surf| surf.alive());
        self.malus_lock_layers.retain(|(_, layer)| layer.alive());
        if self.ext_session_locked {
            let owner_dead = self
                .ext_lock_handle
                .as_ref()
                .is_some_and(|handle| !handle.is_alive());
            if owner_dead {
                self.ext_lock_handle = None;
                self.ext_lock_client_pid = None;
                self.ext_lock_surfaces.clear();
            }
            if self.ext_lock_handle.is_none()
                && self
                    .lockscreen_respawn_at
                    .is_none_or(|ts| ts.elapsed() >= Duration::from_millis(800))
            {
                tracing::warn!(
                    "🔒 [SECURITY] Lockscreen cerrado sin unlock_and_destroy(); manteniendo sesión bloqueada y relanzando malus-de-session-lock"
                );
                self.lockscreen_respawn_at = Some(Instant::now());
                self.spawn_lockscreen_process();
            }
        }
        self.session_locked_flag
            .store(self.is_session_locked(), Ordering::SeqCst);
    }

    pub fn authorized_ext_lock_surface_for_output(&self, output_name: &str) -> Option<LockSurface> {
        self.ext_lock_surfaces
            .get(output_name)
            .filter(|ls| ls.alive())
            .cloned()
    }

    pub fn lock_focus_surface(&self) -> Option<WlSurface> {
        let mut fallback = None;
        for output in self.space.outputs() {
            if let Some(ls) = self.ext_lock_surfaces.get(&output.name()) {
                if ls.alive() {
                    if self.is_primary_output(output) {
                        return Some(ls.wl_surface().clone());
                    }
                    if fallback.is_none() {
                        fallback = Some(ls.wl_surface().clone());
                    }
                }
            }
        }
        for (output, layer) in &self.malus_lock_layers {
            if layer.alive() && layer.can_receive_keyboard_focus() {
                if self.is_primary_output(output) {
                    return Some(layer.wl_surface().clone());
                }
                if fallback.is_none() {
                    fallback = Some(layer.wl_surface().clone());
                }
            }
        }
        fallback
    }

    pub fn is_desktop_focusable_layer_surface(&self, surface: &WlSurface) -> bool {
        self.space.outputs().any(|output| {
            let layer_map = smithay::desktop::layer_map_for_output(output);
            let matched = layer_map.layers().any(|layer| {
                layer.wl_surface() == surface
                    && layer.can_receive_keyboard_focus()
                    && matches!(layer.namespace(), "malus-desktop-icons" | "malus-wallpaper")
            });
            matched
        })
    }

    pub fn desktop_icons_focus_surface(&self) -> Option<WlSurface> {
        let mut fallback = None;
        for output in self.space.outputs() {
            let layer_map = smithay::desktop::layer_map_for_output(output);
            for layer in layer_map.layers() {
                if layer.can_receive_keyboard_focus()
                    && layer.wl_surface().alive()
                    && matches!(layer.namespace(), "malus-desktop-icons" | "malus-wallpaper")
                {
                    if layer.namespace() == "malus-desktop-icons" && self.is_primary_output(output)
                    {
                        return Some(layer.wl_surface().clone());
                    }
                    if fallback.is_none() {
                        fallback = Some(layer.wl_surface().clone());
                    }
                }
            }
        }
        fallback
    }

    pub fn set_keyboard_focus_with_sync(&mut self, mut focus: Option<WlSurface>, serial: Serial) {
        if let Some(lock_surf) = self.lock_focus_surface() {
            let focusing_lock = focus.as_ref().is_some_and(|s| s == &lock_surf);
            if !focusing_lock {
                focus = Some(lock_surf);
            }
        }
        let Some(kb) = self.seat.get_keyboard() else {
            return;
        };
        let is_none = focus.is_none();
        kb.set_focus(self, focus.clone(), serial);
        if is_none {
            self.focus_cpu_priority.set_focused_pid(None);
            set_data_device_focus(&self.display_handle, &self.seat, None);
            set_primary_focus(&self.display_handle, &self.seat, None);
            self.cursor_status = smithay::input::pointer::CursorImageStatus::default_named();
        }
        // Garantizar siempre la sincronización de `Activated` incluso si `kb.set_focus`
        // no disparó `SeatHandler::focus_changed` (foco `None` o igual al foco previo).
        self.sync_window_activation(focus.as_ref());
    }

    /// Envía los `xdg_toplevel.configure` de activación que fueron diferidos durante `ButtonState::Pressed`.
    pub fn flush_pending_activation_configures(&mut self) {
        if self.pending_activation_configures.is_empty() {
            return;
        }
        use smithay::utils::IsAlive;
        let pending: Vec<WlSurface> = self.pending_activation_configures.drain(..).collect();
        for surf in pending {
            if !surf.alive() {
                continue;
            }
            if let Some(win) = self
                .space
                .elements()
                .find(|w| w.wl_surface().as_deref() == Some(&surf))
            {
                if let Some(toplevel) = win.toplevel() {
                    let _ = toplevel.send_pending_configure();
                }
            }
        }
    }

    /// Indica si hay un grab explícito de popup (`xdg_popup.grab`) todavía vigente.
    pub fn popup_grab_active(&self) -> bool {
        self.popup_grab
            .as_ref()
            .is_some_and(|grab| !grab.has_ended())
    }

    /// Libera el grab de popup cuando el cliente ya destruyó todos sus popups
    /// y devuelve el foco de teclado a la ventana raíz sin esperar a la siguiente tecla.
    pub fn refresh_popup_grab(&mut self) {
        let Some(grab) = self.popup_grab.clone() else {
            return;
        };
        if !grab.has_ended() {
            return;
        }
        if let Some(mut g) = self.popup_grab.take() {
            let _ = g.ungrab(smithay::desktop::PopupUngrabStrategy::All);
        }
        if let Some(keyboard) = self.seat.get_keyboard() {
            let previous = grab.previous_serial().unwrap_or(grab.serial());
            if keyboard.is_grabbed()
                && (keyboard.has_grab(grab.serial()) || keyboard.has_grab(previous))
            {
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                keyboard.unset_grab(self);
                keyboard.set_focus(self, grab.current_grab().filter(|s| s.alive()), serial);
            }
        }
        if let Some(pointer) = self.seat.get_pointer() {
            let previous = grab.previous_serial().unwrap_or(grab.serial());
            if pointer.is_grabbed()
                && (pointer.has_grab(grab.serial()) || pointer.has_grab(previous))
            {
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                pointer.unset_grab(self, serial, 0);
            }
        }
    }

    /// Ajusta la geometría de un popup de ventana para que no se salga de los monitores,
    /// respetando el `constraint_adjustment` (flip/slide/resize) pedido por el cliente
    /// en su `xdg_positioner`.
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let kind = PopupKind::Xdg(popup.clone());
        let Ok(root) = find_popup_root_surface(&kind) else {
            return;
        };
        let Some(window) = self.window_for_surface(&root) else {
            return;
        };
        let Some(window_loc) = self.space.element_location(&window) else {
            return;
        };

        let mut outputs = self.space.outputs_for_element(&window);
        if outputs.is_empty() {
            outputs = self.space.outputs().cloned().collect();
        }
        let Some(outputs_geo) = outputs
            .iter()
            .filter_map(|output| self.space.output_geometry(output))
            .reduce(|acc, geo| acc.merge(geo))
        else {
            return;
        };

        // El objetivo debe expresarse relativo a la geometría del padre directo del popup.
        let mut target = outputs_geo;
        target.loc -= window_loc;
        target.loc -= get_popup_toplevel_coords(&kind);

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }

    pub fn popup_under(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<(
        smithay::desktop::Window,
        Point<i32, Logical>,
        WlSurface,
        Point<i32, Logical>,
    )> {
        self.space.elements().rev().find_map(|window| {
            let location = self.space.element_location(window)?;
            let render_loc = location - window.geometry().loc;
            let (surface, surface_location) = window.surface_under(
                point - render_loc.to_f64(),
                smithay::desktop::WindowSurfaceType::POPUP,
            )?;
            Some((window.clone(), render_loc, surface, surface_location))
        })
    }

    fn app_switcher_item_for_surface(
        &mut self,
        surf: &WlSurface,
        is_minimized: bool,
    ) -> AppSwitcherItem {
        let window_id = self.get_or_assign_window_id(surf);
        let (app_id_opt, title_opt) = toplevel_info(surf);
        let app_id = app_id_opt.unwrap_or_else(|| "application-x-executable".to_string());
        let title = title_opt.unwrap_or_else(|| app_id.clone());
        let icon_name = guess_icon_name(&app_id, &title);
        AppSwitcherItem {
            window_id,
            app_id,
            title,
            icon_name: Some(icon_name),
            is_minimized,
        }
    }

    /// Construye la lista de elementos para el selector visual de aplicaciones (⌘Tab)
    /// filtrando estrictamente al monitor objetivo y al escritorio activo.
    pub fn build_app_switcher_items(
        &mut self,
        target_output: Option<&Output>,
    ) -> Vec<AppSwitcherItem> {
        let mut items = Vec::new();
        // 1. Ventanas activas en el escritorio actual mapeadas en `target_output` (de más reciente a más antigua)
        let windows: Vec<_> = self
            .space
            .elements()
            .filter(|win| {
                target_output
                    .map(|target| self.origin_output_for_window(win).as_ref() == Some(target))
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        for win in windows.into_iter().rev() {
            if let Some(surf) = win.wl_surface() {
                items.push(self.app_switcher_item_for_surface(&surf, false));
            }
        }
        // 2. Ventanas minimizadas del escritorio activo: solo aparecen en el Alt+Tab de la pantalla
        // principal, porque el Dock solo reside en dicha pantalla.
        let target_is_primary = target_output
            .map(|o| self.is_primary_output(o))
            .unwrap_or(true);
        let active_ws = self.workspaces.active_index;
        let min_surfaces: Vec<_> = self
            .minimized_windows
            .keys()
            .filter(|surf| {
                let win_ws = self
                    .minimized_window_workspaces
                    .get(*surf)
                    .copied()
                    .unwrap_or(active_ws);
                should_include_minimized_in_app_switcher(target_is_primary, win_ws, active_ws)
            })
            .cloned()
            .collect();
        for surf in min_surfaces {
            items.push(self.app_switcher_item_for_surface(&surf, true));
        }
        items
    }

    /// Conmuta el foco a la ventana especificada por su window_id (restaurando si está minimizada)
    pub fn switch_to_window(&mut self, window_id: u64) {
        tracing::info!("🔀 AppSwitcher: cambiando foco a ventana id={}", window_id);
        // Si la ventana está minimizada, restaurar con Genie Effect
        if self
            .minimized_windows
            .iter()
            .any(|(s, _)| self.window_ids.get(s) == Some(&window_id))
        {
            self.restore_window_by_id(window_id);
            return;
        }

        // Si está en space, traer al frente y enfocar
        let target = self
            .space
            .elements()
            .find(|w| {
                w.wl_surface()
                    .as_ref()
                    .and_then(|s| self.window_ids.get(s.as_ref()))
                    == Some(&window_id)
            })
            .cloned();

        if let Some(win) = target {
            self.space.raise_element(&win, true);
            if let Some(surf) = win.wl_surface() {
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                self.set_keyboard_focus_with_sync(Some(surf.clone().into_owned()), serial);
                self.notify_active_app(Some(&surf));
            }
        }
    }

    /// Alterna App Exposé filtrando por la aplicación actualmente enfocada (o Global si no hay app).
    pub fn toggle_app_expose(&mut self) {
        let app_id = self
            .get_active_window()
            .and_then(|w| w.wl_surface().and_then(|s| toplevel_info(&s).0))
            .unwrap_or_default();
        let mode = if app_id.is_empty() {
            crate::expose::ExposeMode::Global
        } else {
            crate::expose::ExposeMode::AppOnly { app_id }
        };
        self.toggle_expose(mode);
    }

    /// Alterna el modo Exposé / Mission Control (Global o filtrado por aplicación).
    pub fn toggle_expose(&mut self, mode: crate::expose::ExposeMode) {
        self.snap_layouts.close();
        if self.expose.active && self.expose.target_open {
            if self.expose.mode == mode {
                self.close_expose(None);
                return;
            }
        }
        self.open_expose(mode);
    }

    /// Calcula slots de Exposé y hitboxes de la Barra de Espacios para todos los monitores.
    fn compute_expose_slots_and_hitboxes(
        &self,
        mode: &crate::expose::ExposeMode,
    ) -> (
        Vec<crate::expose::ExposeSlot>,
        Vec<crate::expose::SpaceBarHitbox>,
    ) {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        let elements: Vec<smithay::desktop::Window> = self.space.elements().cloned().collect();
        let mut slots = Vec::new();
        let mut hitboxes = Vec::new();
        let space_count = self.workspaces.count();
        let can_add = space_count < crate::workspaces::MAX_WORKSPACES;

        for output in &outputs {
            let Some(out_geo) = self.space.output_geometry(output) else {
                continue;
            };
            let usable_raw = self.usable_zone_for_output(output);
            hitboxes.extend(crate::expose::compute_spaces_bar_hitboxes(
                &output.name(),
                usable_raw,
                space_count,
                can_add,
            ));
            let usable = crate::expose::expose_usable_zone_with_spaces_bar(usable_raw);
            let mut per_output: Vec<(
                usize,
                smithay::desktop::Window,
                u64,
                String,
                String,
                Rectangle<i32, Logical>,
            )> = Vec::new();

            for (idx, win) in elements.iter().enumerate() {
                let Some(loc) = self.space.element_location(win) else {
                    continue;
                };
                let geo = win.geometry();
                let center = Point::<i32, Logical>::from((
                    loc.x + geo.size.w.max(1) / 2,
                    loc.y + geo.size.h.max(1) / 2,
                ));
                let belongs = out_geo.contains(center)
                    || (outputs.len() == 1)
                    || (!outputs.iter().any(|o| {
                        self.space
                            .output_geometry(o)
                            .is_some_and(|g| g.contains(center))
                    }) && outputs.first() == Some(output));
                if !belongs {
                    continue;
                }

                let (raw_app, raw_title) = win
                    .wl_surface()
                    .map(|s| toplevel_info(&s))
                    .unwrap_or((None, None));
                let app_id = raw_app.unwrap_or_default();
                let norm_app = normalize_app_id(&app_id);

                if let crate::expose::ExposeMode::AppOnly {
                    app_id: ref target_app,
                } = mode
                {
                    if norm_app != normalize_app_id(target_app) {
                        continue;
                    }
                }

                let title = raw_title
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| {
                        if !app_id.is_empty() {
                            app_id.clone()
                        } else {
                            "Ventana".to_string()
                        }
                    });
                let window_id = win
                    .wl_surface()
                    .as_ref()
                    .and_then(|s| self.window_ids.get(s.as_ref()).copied())
                    .unwrap_or(0);
                let orig_rect = Rectangle::new(loc, geo.size);
                per_output.push((idx, win.clone(), window_id, app_id, title, orig_rect));
            }

            let layout_input: Vec<(usize, Rectangle<i32, Logical>)> = per_output
                .iter()
                .enumerate()
                .map(|(local_i, (_, _, _, _, _, r))| (local_i, *r))
                .collect();
            let laid_out = crate::expose::compute_expose_layout(usable, &layout_input);

            for (local_i, target_rect) in laid_out {
                let (_, win, window_id, app_id, title, original_rect) = &per_output[local_i];
                slots.push(crate::expose::ExposeSlot {
                    window: win.clone(),
                    window_id: *window_id,
                    output_name: output.name(),
                    app_id: app_id.clone(),
                    title: title.clone(),
                    original_rect: *original_rect,
                    target_rect,
                    matches_filter: true,
                });
            }
        }

        (slots, hitboxes)
    }

    fn refresh_expose_layout(&mut self) {
        if !self.expose.active {
            return;
        }
        let mode = self.expose.mode.clone();
        let (slots, hitboxes) = self.compute_expose_slots_and_hitboxes(&mode);
        self.expose.slots = slots;
        self.expose.spaces_bar_hitboxes = hitboxes;
        if let Some(sel) = self.expose.selected_index {
            if sel >= self.expose.slots.len() {
                self.expose.selected_index = (!self.expose.slots.is_empty()).then_some(0);
            }
        } else {
            self.expose.selected_index = (!self.expose.slots.is_empty()).then_some(0);
        }
        self.expose.hovered_index = None;
        self.expose.quick_look_index = None;
        if !self.expose.filter_query.is_empty() {
            let q = self.expose.filter_query.clone();
            self.expose.apply_filter(q);
        }
        self.workspace_revision = self.workspace_revision.wrapping_add(1);
        self.expose.bump_revision();
    }

    /// Abre Exposé calculando el mosaico de ventanas por monitor.
    pub fn open_expose(&mut self, mode: crate::expose::ExposeMode) {
        if self.spotlight.is_open() {
            self.spotlight.close();
            self.ipc_broadcaster.broadcast(&IpcMessage::HideSpotlight);
        }
        if self.app_switcher.is_active || self.app_switcher.deck.active {
            self.dismiss_app_switcher_immediately();
        }

        let previous_focus = self.get_active_window();
        let (slots, hitboxes) = self.compute_expose_slots_and_hitboxes(&mode);

        if slots.is_empty() && matches!(mode, crate::expose::ExposeMode::AppOnly { .. }) {
            if self.expose.active {
                self.close_expose(None);
            }
            return;
        }

        tracing::info!(
            "🪟 Exposé abierto ({:?}) con {} ventanas",
            mode,
            slots.len()
        );
        self.expose.open(mode, slots, previous_focus);
        self.expose.spaces_bar_hitboxes = hitboxes;
    }

    /// Cierra Exposé; si `activate_index` es `Some`, eleva y enfoca esa ventana.
    pub fn close_expose(&mut self, activate_index: Option<usize>) {
        if !self.expose.active {
            return;
        }
        let chosen_win = activate_index
            .and_then(|idx| self.expose.slots.get(idx))
            .map(|s| s.window.clone())
            .or_else(|| {
                self.expose
                    .previous_focus
                    .as_ref()
                    .filter(|w| self.space.elements().any(|e| e == *w))
                    .cloned()
            });

        self.expose.start_close();

        if let Some(win) = chosen_win {
            self.space.raise_element(&win, true);
            if let Some(surf) = win.wl_surface() {
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                self.set_keyboard_focus_with_sync(Some(surf.clone().into_owned()), serial);
            }
        }
    }

    /// Procesa eventos de teclado mientras Exposé está activo.
    pub fn handle_expose_key(
        &mut self,
        sym: smithay::input::keyboard::Keysym,
        modifiers: smithay::input::keyboard::ModifiersState,
    ) -> bool {
        if !self.expose.active || !self.expose.target_open {
            return false;
        }
        use smithay::input::keyboard::keysyms;

        if sym == keysyms::KEY_Escape.into() {
            if !self.expose.filter_query.is_empty() {
                self.expose.apply_filter(String::new());
            } else {
                self.close_expose(None);
            }
            return true;
        }

        if sym == keysyms::KEY_Return.into() || sym == keysyms::KEY_KP_Enter.into() {
            let target = self
                .expose
                .selected_index
                .or(self.expose.hovered_index)
                .filter(|&idx| self.expose.slots.get(idx).is_some_and(|s| s.matches_filter))
                .or_else(|| self.expose.slots.iter().position(|s| s.matches_filter));
            self.close_expose(target);
            return true;
        }

        if sym == keysyms::KEY_Left.into()
            || sym == keysyms::KEY_Up.into()
            || sym == keysyms::KEY_ISO_Left_Tab.into()
        {
            self.expose.move_selection(-1);
            return true;
        }
        if sym == keysyms::KEY_Right.into()
            || sym == keysyms::KEY_Down.into()
            || sym == keysyms::KEY_Tab.into()
        {
            self.expose.move_selection(1);
            return true;
        }

        if sym == keysyms::KEY_space.into() && self.expose.filter_query.is_empty() {
            let target = self.expose.selected_index.or(self.expose.hovered_index);
            if self.expose.quick_look_index == target {
                self.expose.quick_look_index = None;
            } else {
                self.expose.quick_look_index = target;
            }
            self.expose.bump_revision();
            return true;
        }

        if sym == keysyms::KEY_BackSpace.into() {
            let mut q = self.expose.filter_query.clone();
            q.pop();
            self.expose.apply_filter(q);
            return true;
        }

        if !modifiers.ctrl && !modifiers.alt && !modifiers.logo {
            if let Some(ch) = sym.key_char() {
                if !ch.is_control() {
                    let mut q = self.expose.filter_query.clone();
                    q.push(ch);
                    self.expose.apply_filter(q);
                    return true;
                }
            }
        }

        true
    }

    /// Actualiza la tarjeta resaltada o el arrastre en curso mientras Exposé está activo.
    pub fn handle_expose_pointer_motion(&mut self, point: Point<f64, Logical>) -> bool {
        if !self.expose.active || !self.expose.target_open {
            return false;
        }
        let space_hit = self.expose.hit_test_space_bar(point);
        if let Some(ref mut drag) = self.expose.dragging {
            drag.current_pointer = point;
            let dx = point.x - drag.start_pointer.x;
            let dy = point.y - drag.start_pointer.y;
            if (dx * dx + dy * dy).sqrt() >= 8.0 {
                drag.has_moved = true;
            }
            drag.hovered_target = space_hit.clone();
            self.expose.hovered_space_target = space_hit;
            self.expose.bump_revision();
            return true;
        }

        let hit = if space_hit.is_some() {
            None
        } else {
            self.expose.hit_test(point)
        };
        let prev_hover = self.expose.hovered_index;
        let prev_sel = self.expose.selected_index;
        let prev_space_hover = self.expose.hovered_space_target.clone();
        self.expose.hovered_index = hit;
        self.expose.hovered_space_target = space_hit;
        if hit.is_some() {
            self.expose.selected_index = hit;
        }
        if self.expose.hovered_index != prev_hover
            || self.expose.selected_index != prev_sel
            || self.expose.hovered_space_target != prev_space_hover
        {
            self.expose.bump_revision();
        }
        true
    }

    /// Procesa clics y Drag & Drop en Exposé y su Barra de Espacios superior.
    pub fn handle_expose_pointer_button(
        &mut self,
        point: Point<f64, Logical>,
        button_code: u32,
        is_pressed: bool,
    ) -> bool {
        if !self.expose.active || !self.expose.target_open {
            return false;
        }
        if button_code != 0x110 {
            return true;
        }

        if is_pressed {
            if let Some(space_target) = self.expose.hit_test_space_bar(point) {
                match space_target {
                    crate::expose::SpaceDropTarget::Workspace(idx) => {
                        if idx == self.workspaces.active_index {
                            self.close_expose(None);
                        } else {
                            self.switch_workspace(idx);
                        }
                    }
                    crate::expose::SpaceDropTarget::NewSpace => {
                        if let Some(new_idx) = self.workspaces.add_workspace() {
                            tracing::info!(
                                "🖥️ Nuevo espacio de trabajo creado: Escritorio {}",
                                new_idx + 1
                            );
                            self.refresh_expose_layout();
                        }
                    }
                }
                return true;
            }

            if let Some(hit) = self.expose.hit_test(point) {
                if let Some(slot) = self.expose.slots.get(hit) {
                    let t = self.expose.eased_progress();
                    let cur_rect = slot.current_rect(t);
                    self.expose.dragging = Some(crate::expose::ExposeDragState {
                        slot_index: hit,
                        window: slot.window.clone(),
                        window_id: slot.window_id,
                        orig_rect: slot.original_rect,
                        start_rect: cur_rect,
                        start_pointer: point,
                        current_pointer: point,
                        has_moved: false,
                        hovered_target: None,
                    });
                    self.expose.selected_index = Some(hit);
                    self.expose.bump_revision();
                    return true;
                }
            }

            self.close_expose(None);
            return true;
        }

        // ButtonState::Released
        if let Some(drag) = self.expose.dragging.take() {
            self.expose.hovered_space_target = None;
            if !drag.has_moved {
                self.close_expose(Some(drag.slot_index));
                return true;
            }
            let drop_target = drag
                .hovered_target
                .or_else(|| self.expose.hit_test_space_bar(point));
            match drop_target {
                Some(crate::expose::SpaceDropTarget::Workspace(target_idx)) => {
                    self.move_window_to_workspace(drag.window, drag.orig_rect.loc, target_idx);
                }
                Some(crate::expose::SpaceDropTarget::NewSpace) => {
                    if let Some(new_idx) = self.workspaces.add_workspace() {
                        self.move_window_to_workspace(drag.window, drag.orig_rect.loc, new_idx);
                    } else {
                        self.expose.bump_revision();
                    }
                }
                None => {
                    self.expose.bump_revision();
                }
            }
        }
        true
    }

    /// Solicita el cambio al espacio de trabajo indicado, capturando antes el cuadro saliente
    /// para animar la transición horizontal (incluyendo cambios desde la barra de Exposé).
    pub fn switch_workspace(&mut self, target_index: usize) {
        if target_index >= self.workspaces.count() {
            return;
        }
        if target_index == self.workspaces.active_index {
            self.pending_workspace_switch = None;
            return;
        }
        self.pending_workspace_switch = Some(crate::workspace_transition::PendingWorkspaceSwitch {
            target_index,
            close_expose_immediately: self.expose.active,
        });
    }

    /// Completa un cambio de espacio pendiente tras capturar el fotograma saliente e inicia
    /// la animación de deslizamiento horizontal si hubo captura válida.
    pub fn commit_pending_workspace_switch(&mut self, captured_any_output: bool) -> bool {
        let Some(pending) = self.pending_workspace_switch.take() else {
            return false;
        };
        if pending.target_index >= self.workspaces.count()
            || pending.target_index == self.workspaces.active_index
        {
            return false;
        }
        let direction = if pending.target_index > self.workspaces.active_index {
            1.0
        } else {
            -1.0
        };
        if pending.close_expose_immediately {
            self.expose.close_immediately();
        }
        self.apply_workspace_switch(pending.target_index);
        if captured_any_output {
            self.workspace_transition = Some(
                crate::workspace_transition::WorkspaceTransitionState::new(direction),
            );
        }
        true
    }

    /// Intercambia inmediatamente las ventanas del espacio saliente y entrante en `Space`.
    pub fn apply_workspace_switch(&mut self, target_index: usize) {
        if target_index >= self.workspaces.count() || target_index == self.workspaces.active_index {
            return;
        }
        let old_index = self.workspaces.active_index;
        let current_windows: Vec<(smithay::desktop::Window, Point<i32, Logical>)> = self
            .space
            .elements()
            .filter_map(|w| self.space.element_location(w).map(|loc| (w.clone(), loc)))
            .collect();

        for (win, _) in &current_windows {
            self.space.unmap_elem(win);
        }
        self.workspaces.workspaces[old_index].hidden_windows = current_windows
            .into_iter()
            .map(|(window, location)| crate::workspaces::WorkspaceWindow { window, location })
            .collect();

        let incoming = std::mem::take(&mut self.workspaces.workspaces[target_index].hidden_windows);
        let mut top_win: Option<smithay::desktop::Window> = None;
        for entry in incoming {
            self.space
                .map_element(entry.window.clone(), entry.location, false);
            top_win = Some(entry.window);
        }
        self.space.refresh();
        self.workspaces.active_index = target_index;
        self.workspace_revision = self.workspace_revision.wrapping_add(1);

        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        let next_surf = top_win.and_then(|w| w.wl_surface().map(|s| s.into_owned()));
        self.set_keyboard_focus_with_sync(next_surf.clone(), serial);
        self.notify_active_app(next_surf.as_ref());
        self.notify_running_apps();
        if self.expose.active {
            self.refresh_expose_layout();
        }
        if self.auto_tiling_enabled {
            self.relayout_auto_tiling();
        }
        tracing::info!(
            "🖥️ Espacio cambiado: Escritorio {} -> Escritorio {}",
            old_index + 1,
            target_index + 1
        );
    }

    /// Navega al espacio de trabajo relativo (`delta`: `-1` anterior, `+1` siguiente).
    pub fn switch_workspace_delta(&mut self, delta: isize) {
        let count = self.workspaces.count() as isize;
        if count <= 1 {
            return;
        }
        let current = self
            .pending_workspace_switch
            .map(|p| p.target_index)
            .unwrap_or(self.workspaces.active_index);
        let next = (current as isize + delta).clamp(0, count - 1) as usize;
        if next != current || next != self.workspaces.active_index {
            self.switch_workspace(next);
        }
    }

    /// Mueve una ventana desde el espacio activo hacia `target_index` (reubicándola en Exposé).
    pub fn move_window_to_workspace(
        &mut self,
        window: smithay::desktop::Window,
        orig_loc: Point<i32, Logical>,
        target_index: usize,
    ) {
        if target_index >= self.workspaces.count() || target_index == self.workspaces.active_index {
            self.expose.bump_revision();
            return;
        }
        let loc = self.space.element_location(&window).unwrap_or(orig_loc);
        self.space.unmap_elem(&window);
        self.space.refresh();
        self.workspaces.workspaces[target_index]
            .hidden_windows
            .push(crate::workspaces::WorkspaceWindow {
                window,
                location: loc,
            });
        self.workspace_revision = self.workspace_revision.wrapping_add(1);
        self.notify_running_apps();
        if self.expose.active {
            self.refresh_expose_layout();
        }
        tracing::info!("🪟 Ventana movida a Escritorio {}", target_index + 1);
    }

    /// Construye los datos resumidos de miniaturas para la Barra de Espacios en `output`.
    pub fn space_thumbnails_for_output(
        &self,
        output: &Output,
    ) -> Vec<crate::expose::SpaceThumbnailInfo> {
        let out_geo = self.space.output_geometry(output).unwrap_or_default();
        let ow = out_geo.size.w.max(1) as f64;
        let oh = out_geo.size.h.max(1) as f64;
        let to_norm =
            |loc: Point<i32, Logical>, size: Size<i32, Logical>| -> Option<(f64, f64, f64, f64)> {
                let cx = loc.x + size.w.max(1) / 2;
                let cy = loc.y + size.h.max(1) / 2;
                if self.space.outputs().count() > 1 && !out_geo.contains((cx, cy)) {
                    return None;
                }
                let nx = ((loc.x - out_geo.loc.x) as f64 / ow).clamp(0.0, 0.95);
                let ny = ((loc.y - out_geo.loc.y) as f64 / oh).clamp(0.0, 0.95);
                let nw = (size.w.max(60) as f64 / ow).clamp(0.08, 1.0);
                let nh = (size.h.max(40) as f64 / oh).clamp(0.08, 1.0);
                Some((nx, ny, nw, nh))
            };

        self.workspaces
            .workspaces
            .iter()
            .enumerate()
            .map(|(idx, ws)| {
                let is_active = idx == self.workspaces.active_index;
                let mini_rects: Vec<(f64, f64, f64, f64)> = if is_active {
                    self.space
                        .elements()
                        .filter_map(|w| {
                            let loc = self.space.element_location(w)?;
                            to_norm(loc, w.geometry().size)
                        })
                        .collect()
                } else {
                    ws.hidden_windows
                        .iter()
                        .filter_map(|entry| to_norm(entry.location, entry.window.geometry().size))
                        .collect()
                };
                crate::expose::SpaceThumbnailInfo {
                    index: idx,
                    name: ws.name.clone(),
                    is_active,
                    mini_rects,
                }
            })
            .collect()
    }

    /// Inicia el seguimiento de gesto multitáctil de deslizamiento (`libinput` 3+ dedos).
    pub fn handle_gesture_swipe_begin(&mut self, fingers: u32) -> bool {
        let matches_configured = fingers >= 3
            && (fingers == self.input_cfg.trackpad_swipe_spaces_fingers as u32
                || fingers == self.input_cfg.trackpad_mission_control_fingers as u32
                || fingers == self.input_cfg.trackpad_app_expose_fingers as u32
                || (self.expose.active && self.expose.target_open));
        if !matches_configured {
            self.swipe_gesture.active = false;
            return false;
        }
        self.swipe_gesture = SwipeGestureState {
            active: true,
            fingers,
            accum_x: 0.0,
            accum_y: 0.0,
            last_dx: 0.0,
            last_dy: 0.0,
            kind: SwipeGestureKind::Undecided,
            was_open_at_start: self.expose.active && self.expose.target_open,
            app_expose_gesture: false,
        };
        true
    }

    /// Actualiza 1:1 el progreso de Exposé o el cambio de espacio durante un deslizamiento de 3+ dedos.
    pub fn handle_gesture_swipe_update(&mut self, dx: f64, dy: f64) -> bool {
        if !self.swipe_gesture.active {
            return false;
        }
        self.swipe_gesture.accum_x += dx;
        self.swipe_gesture.accum_y += dy;
        self.swipe_gesture.last_dx = dx;
        self.swipe_gesture.last_dy = dy;

        if self.swipe_gesture.kind == SwipeGestureKind::Undecided {
            let ax = self.swipe_gesture.accum_x.abs();
            let ay = self.swipe_gesture.accum_y.abs();
            if ax.max(ay) < 10.0 {
                return true;
            }
            if ay >= ax {
                self.swipe_gesture.kind = SwipeGestureKind::ExposeVertical;
                if !self.swipe_gesture.was_open_at_start {
                    let f = self.swipe_gesture.fingers;
                    if self.swipe_gesture.accum_y < 0.0 {
                        // Deslizar arriba -> Mission Control (si coincide con el número de dedos configurado)
                        if f == self.input_cfg.trackpad_mission_control_fingers as u32 {
                            self.open_expose(crate::expose::ExposeMode::Global);
                        }
                    } else if f == self.input_cfg.trackpad_app_expose_fingers as u32 {
                        // Deslizar abajo -> App Exposé de la app activa
                        self.swipe_gesture.app_expose_gesture = true;
                        let app_id = self
                            .get_active_window()
                            .and_then(|w| w.wl_surface().and_then(|s| toplevel_info(&s).0))
                            .unwrap_or_default();
                        let mode = if app_id.is_empty() {
                            crate::expose::ExposeMode::Global
                        } else {
                            crate::expose::ExposeMode::AppOnly { app_id }
                        };
                        self.open_expose(mode);
                    }
                    if self.expose.active {
                        self.expose.begin_gesture_scrub();
                    }
                } else {
                    self.expose.begin_gesture_scrub();
                }
            } else {
                self.swipe_gesture.kind = SwipeGestureKind::WorkspaceHorizontal;
            }
        }

        if self.swipe_gesture.kind == SwipeGestureKind::ExposeVertical && self.expose.active {
            let drag_span = 140.0;
            let progress = if self.swipe_gesture.was_open_at_start {
                1.0 - (self.swipe_gesture.accum_y / drag_span)
            } else if self.swipe_gesture.app_expose_gesture {
                self.swipe_gesture.accum_y / drag_span
            } else {
                -self.swipe_gesture.accum_y / drag_span
            };
            self.expose.set_gesture_progress(progress as f32);
        }

        true
    }

    /// Finaliza el gesto de deslizamiento decidiendo confirmar o revertir según umbral y velocidad.
    pub fn handle_gesture_swipe_end(&mut self, cancelled: bool) -> bool {
        if !self.swipe_gesture.active {
            return false;
        }
        let g = std::mem::take(&mut self.swipe_gesture);
        if cancelled {
            if g.kind == SwipeGestureKind::ExposeVertical && self.expose.active {
                self.expose.finish_gesture_scrub(g.was_open_at_start);
            }
            return true;
        }

        match g.kind {
            SwipeGestureKind::ExposeVertical => {
                if self.expose.active {
                    let p = self.expose.progress;
                    let commit_open = if g.was_open_at_start {
                        !(p < 0.70 || g.last_dy > 4.0)
                    } else if g.app_expose_gesture {
                        p >= 0.30 || (g.last_dy > 4.0 && p >= 0.12)
                    } else {
                        p >= 0.30 || (g.last_dy < -4.0 && p >= 0.12)
                    };
                    if commit_open {
                        self.expose.finish_gesture_scrub(true);
                    } else {
                        self.close_expose(None);
                    }
                }
            }
            SwipeGestureKind::WorkspaceHorizontal => {
                if g.fingers == self.input_cfg.trackpad_swipe_spaces_fingers as u32 {
                    if g.accum_x <= -60.0 {
                        self.switch_workspace_delta(1);
                    } else if g.accum_x >= 60.0 {
                        self.switch_workspace_delta(-1);
                    }
                }
            }
            SwipeGestureKind::Undecided => {}
        }
        true
    }

    /// Inicia un gesto multitáctil de pellizco de 4 dedos (Apps / Mostrar escritorio).
    pub fn handle_gesture_pinch_begin(&mut self, fingers: u32) -> bool {
        if fingers >= 4
            && (self.input_cfg.trackpad_apps_gesture || self.input_cfg.show_desktop_gesture)
        {
            self.pinch_gesture = PinchGestureState {
                active: true,
                fingers,
                last_scale: 1.0,
                triggered: false,
            };
            return true;
        }
        self.pinch_gesture.active = false;
        false
    }

    /// Actualiza un gesto de pellizco de 4 dedos: juntar abre Apps/Canopy; separar muestra el escritorio.
    pub fn handle_gesture_pinch_update(&mut self, scale: f64) -> bool {
        if !self.pinch_gesture.active {
            return false;
        }
        self.pinch_gesture.last_scale = scale;
        if !self.pinch_gesture.triggered {
            if scale <= 0.72 && self.input_cfg.trackpad_apps_gesture {
                self.pinch_gesture.triggered = true;
                self.toggle_spotlight();
            } else if scale >= 1.28 && self.input_cfg.show_desktop_gesture {
                self.pinch_gesture.triggered = true;
                self.toggle_show_desktop();
            }
        }
        true
    }

    /// Finaliza el gesto de pellizco de 4 dedos.
    pub fn handle_gesture_pinch_end(&mut self, _cancelled: bool) -> bool {
        if !self.pinch_gesture.active {
            return false;
        }
        self.pinch_gesture = PinchGestureState::default();
        true
    }

    /// Cierra la ventana especificada por su window_id
    pub fn close_window_by_id(&mut self, window_id: u64) {
        tracing::info!("🚪 AppSwitcher: cerrando ventana id={}", window_id);
        let target = self
            .space
            .elements()
            .find(|w| {
                w.wl_surface()
                    .as_ref()
                    .and_then(|s| self.window_ids.get(s.as_ref()))
                    == Some(&window_id)
            })
            .cloned();

        if let Some(win) = target {
            self.record_close_button_hint(&win);
            if let Some(toplevel) = win.toplevel() {
                toplevel.send_close();
            }
        } else if let Some(surf) = self
            .minimized_windows
            .keys()
            .find(|s| self.window_ids.get(*s) == Some(&window_id))
            .cloned()
        {
            if let Some((win, _)) = self.minimized_windows.remove(&surf) {
                self.minimized_window_workspaces.remove(&surf);
                self.remove_minimized_thumbnail(window_id);
                if let Some(toplevel) = win.toplevel() {
                    toplevel.send_close();
                }
                self.notify_minimized_windows();
            }
        }
    }

    /// Oculta / minimiza la ventana especificada por su window_id vía Genie Effect
    pub fn hide_window_by_id(&mut self, window_id: u64) {
        tracing::info!("🪟 AppSwitcher: ocultando ventana id={}", window_id);
        let target = self
            .space
            .elements()
            .find(|w| {
                w.wl_surface()
                    .as_ref()
                    .and_then(|s| self.window_ids.get(s.as_ref()))
                    == Some(&window_id)
            })
            .cloned();

        if let Some(win) = target {
            if let Some(toplevel) = win.toplevel().cloned() {
                self.minimize_toplevel(&toplevel);
            }
        }
    }

    pub fn app_switcher_uses_deck(&self) -> bool {
        self.app_switcher_style == AppSwitcherStyle::Deck
    }

    pub fn app_switcher_classic_active(&self) -> bool {
        self.app_switcher.is_active && !self.app_switcher_uses_deck()
    }

    pub fn tick_app_switcher_deck(&mut self, now: Instant) {
        if self.app_switcher.deck.active {
            let _ = self.app_switcher.deck.tick(now);
        }
    }

    pub fn take_app_switcher_deck_closed_dirty(&mut self) -> bool {
        let dirty = self.app_switcher.closed_dirty || self.app_switcher.deck.take_closed_dirty();
        self.app_switcher.closed_dirty = false;
        dirty
    }

    pub fn deck_should_render_on_output(&self, output_name: &str) -> bool {
        self.app_switcher_uses_deck() && self.app_switcher.deck.should_render_on_output(output_name)
    }

    fn collect_deck_origin_rects(&self) -> HashMap<u64, Rectangle<i32, Logical>> {
        let mut rects = HashMap::new();
        for win in self.space.elements() {
            let Some(loc) = self.space.element_location(win) else {
                continue;
            };
            let Some(surf) = win.wl_surface() else {
                continue;
            };
            let Some(&wid) = self.window_ids.get(surf.as_ref()) else {
                continue;
            };
            let ssd_top = if window_uses_ssd(win) {
                crate::ssd::SSD_TITLEBAR_HEIGHT
            } else {
                0
            };
            let sz = win.geometry().size;
            rects.insert(
                wid,
                Rectangle::new(
                    (loc.x, loc.y - ssd_top).into(),
                    (sz.w.max(1), sz.h.max(1) + ssd_top).into(),
                ),
            );
        }
        rects
    }

    fn focus_or_unminimize_for_deck_close(&mut self, window_id: u64) {
        let min_surf = self
            .minimized_windows
            .iter()
            .find(|(s, _)| self.window_ids.get(*s) == Some(&window_id))
            .map(|(s, _)| s.clone());

        if let Some(surf) = min_surf {
            if let Some((win, loc)) = self.minimized_windows.remove(&surf) {
                self.minimized_window_workspaces.remove(&surf);
                self.remove_minimized_thumbnail(window_id);
                self.dock_slots.remove(&window_id);
                self.space.map_element(win.clone(), loc, true);
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                self.set_keyboard_focus_with_sync(Some(surf.clone()), serial);
                self.notify_active_app(Some(&surf));
                self.notify_running_apps();
                self.notify_minimized_windows();

                let ssd_top = if window_uses_ssd(&win) {
                    crate::ssd::SSD_TITLEBAR_HEIGHT
                } else {
                    0
                };
                let sz = win.geometry().size;
                self.app_switcher.deck.origin_rects.insert(
                    window_id,
                    Rectangle::new(
                        (loc.x, loc.y - ssd_top).into(),
                        (sz.w.max(1), sz.h.max(1) + ssd_top).into(),
                    ),
                );
                if let Some(item) = self
                    .app_switcher
                    .items
                    .iter_mut()
                    .find(|it| it.window_id == window_id)
                {
                    item.is_minimized = false;
                }
                return;
            }
        }

        self.switch_to_window(window_id);
    }

    fn target_app_switcher_output(
        &self,
        pointer_loc: Option<Point<f64, Logical>>,
    ) -> Option<Output> {
        if let Some(pt) = pointer_loc {
            if let Some(out) = self.space.outputs().find(|o| {
                self.space
                    .output_geometry(o)
                    .is_some_and(|g| g.to_f64().contains(pt))
            }) {
                return Some(out.clone());
            }
        }
        if let Some(win) = self.get_active_window() {
            if let Some(out) = self.origin_output_for_window(&win) {
                return Some(out);
            }
        }
        self.primary_output()
    }

    pub fn open_app_switcher(&mut self, reverse: bool, pointer_loc: Option<Point<f64, Logical>>) {
        let target_output = self.target_app_switcher_output(pointer_loc);
        let items = self.build_app_switcher_items(target_output.as_ref());
        if items.is_empty() {
            tracing::info!(
                "🔀 AppSwitcher: no hay ventanas abiertas en la pantalla objetivo, ignorando"
            );
            return;
        }
        let initial_idx = if items.len() > 1 {
            if reverse {
                items.len() - 1
            } else {
                1
            }
        } else {
            0
        };
        self.app_switcher.items = items;
        self.app_switcher.selected_index = initial_idx;
        self.app_switcher.is_active = true;
        self.app_switcher.closed_dirty = false;

        if self.app_switcher_uses_deck() {
            let origin_rects = self.collect_deck_origin_rects();
            self.app_switcher.deck.reset_on_open(
                target_output.map(|o| o.name()),
                initial_idx,
                origin_rects,
                Instant::now(),
            );
            self.ipc_broadcaster.broadcast(&IpcMessage::HideAppSwitcher);
        } else {
            self.app_switcher.deck.clear();
            self.ipc_broadcaster
                .broadcast(&IpcMessage::ShowAppSwitcher {
                    items: self.app_switcher.items.clone(),
                    selected_index: initial_idx,
                });
        }
    }

    pub fn step_app_switcher(&mut self, forward: bool) {
        let len = self.app_switcher.items.len();
        if len == 0 {
            return;
        }
        let idx = if forward {
            (self.app_switcher.selected_index + 1) % len
        } else {
            (self.app_switcher.selected_index + len - 1) % len
        };
        self.app_switcher.selected_index = idx;
        if self.app_switcher_uses_deck() {
            self.app_switcher.deck.retarget_focus(idx, Instant::now());
        } else {
            self.ipc_broadcaster
                .broadcast(&IpcMessage::UpdateAppSwitcher {
                    selected_index: idx,
                });
        }
    }

    pub fn commit_app_switcher(&mut self) {
        let idx = self.app_switcher.selected_index;
        self.commit_app_switcher_at(idx);
    }

    pub fn commit_app_switcher_at(&mut self, index: usize) {
        let target_id = self.app_switcher.items.get(index).map(|i| i.window_id);
        if self.app_switcher_uses_deck() {
            let now = Instant::now();
            self.app_switcher.is_active = false;
            self.app_switcher.selected_index =
                index.min(self.app_switcher.items.len().saturating_sub(1));
            self.app_switcher
                .deck
                .retarget_focus(self.app_switcher.selected_index, now);
            if let Some(window_id) = target_id {
                self.focus_or_unminimize_for_deck_close(window_id);
            }
            self.app_switcher.deck.begin_close(now);
            self.ipc_broadcaster.broadcast(&IpcMessage::HideAppSwitcher);
        } else {
            self.dismiss_app_switcher_immediately();
            if let Some(window_id) = target_id {
                self.switch_to_window(window_id);
            }
        }
    }

    pub fn dismiss_app_switcher(&mut self) {
        if self.app_switcher_uses_deck() {
            self.app_switcher.is_active = false;
            self.app_switcher.deck.begin_close(Instant::now());
            self.ipc_broadcaster.broadcast(&IpcMessage::HideAppSwitcher);
        } else {
            self.dismiss_app_switcher_immediately();
        }
    }

    pub fn dismiss_app_switcher_immediately(&mut self) {
        let was_deck = (self.app_switcher.is_active || self.app_switcher.deck.active)
            && self.app_switcher_uses_deck();
        self.app_switcher.is_active = false;
        self.app_switcher.deck.clear();
        if was_deck {
            self.app_switcher.closed_dirty = true;
        }
        self.ipc_broadcaster.broadcast(&IpcMessage::HideAppSwitcher);
    }

    pub fn close_selected_app_switcher_window(&mut self) {
        if self.app_switcher.items.is_empty() {
            return;
        }
        let idx = self
            .app_switcher
            .selected_index
            .min(self.app_switcher.items.len() - 1);
        let target_id = self.app_switcher.items[idx].window_id;
        self.close_window_by_id(target_id);
        self.app_switcher.items.remove(idx);
        if self.app_switcher.items.is_empty() {
            self.dismiss_app_switcher();
            return;
        }
        let new_idx = idx.min(self.app_switcher.items.len() - 1);
        self.app_switcher.selected_index = new_idx;
        if self.app_switcher_uses_deck() {
            self.app_switcher
                .deck
                .retarget_focus(new_idx, Instant::now());
        } else {
            self.ipc_broadcaster
                .broadcast(&IpcMessage::ShowAppSwitcher {
                    items: self.app_switcher.items.clone(),
                    selected_index: new_idx,
                });
        }
    }

    pub fn hide_selected_app_switcher_window(&mut self) {
        if self.app_switcher.items.is_empty() {
            return;
        }
        let idx = self
            .app_switcher
            .selected_index
            .min(self.app_switcher.items.len() - 1);
        let target_id = self.app_switcher.items[idx].window_id;
        self.hide_window_by_id(target_id);
        self.app_switcher.items[idx].is_minimized = true;
        self.app_switcher.deck.origin_rects.remove(&target_id);
        if !self.app_switcher_uses_deck() {
            self.ipc_broadcaster
                .broadcast(&IpcMessage::ShowAppSwitcher {
                    items: self.app_switcher.items.clone(),
                    selected_index: idx,
                });
        }
    }

    pub fn deck_windows_for_render(
        &self,
    ) -> Vec<(u64, smithay::desktop::Window, Size<i32, Logical>, bool)> {
        let mut out = Vec::new();
        for win in self.space.elements() {
            if let Some(surf) = win.wl_surface() {
                if let Some(&wid) = self.window_ids.get(surf.as_ref()) {
                    let uses_ssd = window_uses_ssd(win);
                    let tb = if uses_ssd {
                        crate::ssd::SSD_TITLEBAR_HEIGHT
                    } else {
                        0
                    };
                    let sz = win.geometry().size;
                    out.push((
                        wid,
                        win.clone(),
                        Size::from((sz.w.max(1), sz.h.max(1) + tb)),
                        uses_ssd,
                    ));
                }
            }
        }
        for (surf, (win, _)) in &self.minimized_windows {
            if let Some(&wid) = self.window_ids.get(surf) {
                let uses_ssd = window_uses_ssd(win);
                let tb = if uses_ssd {
                    crate::ssd::SSD_TITLEBAR_HEIGHT
                } else {
                    0
                };
                let sz = win.geometry().size;
                out.push((
                    wid,
                    win.clone(),
                    Size::from((sz.w.max(1), sz.h.max(1) + tb)),
                    uses_ssd,
                ));
            }
        }
        out
    }

    fn deck_active_output_geometry(&self) -> Option<Rectangle<i32, Logical>> {
        let target = self
            .app_switcher
            .deck
            .output_name
            .as_deref()
            .and_then(|name| self.space.outputs().find(|o| o.name() == name))
            .or_else(|| self.space.outputs().next())?;
        self.space.output_geometry(target)
    }

    fn deck_layout_size_inputs(&self) -> Vec<(u64, Size<i32, Logical>)> {
        let windows = self.deck_windows_for_render();
        self.app_switcher
            .items
            .iter()
            .map(|item| {
                let sz = windows
                    .iter()
                    .find(|(id, ..)| *id == item.window_id)
                    .map(|(_, _, sz, _)| *sz)
                    .unwrap_or_else(|| Size::from((1080, 700)));
                (item.window_id, sz)
            })
            .collect()
    }

    pub fn handle_app_switcher_deck_pointer_motion(&mut self, point: Point<f64, Logical>) -> bool {
        if !self.app_switcher.is_active || !self.app_switcher_uses_deck() {
            return false;
        }
        self.app_switcher.deck.hovered_index = self.deck_active_output_geometry().and_then(|geo| {
            let size_inputs = self.deck_layout_size_inputs();
            let layouts = crate::app_switcher::compute_deck_layout(
                geo,
                self.app_switcher.deck.visual_focus,
                &size_inputs,
            );
            crate::app_switcher::hit_test_deck_card(&layouts, point)
        });
        true
    }

    pub fn handle_app_switcher_deck_pointer_button(
        &mut self,
        point: Point<f64, Logical>,
        button_code: u32,
        is_pressed: bool,
    ) -> bool {
        if !self.app_switcher.is_active || !self.app_switcher_uses_deck() {
            return false;
        }
        if is_pressed && button_code == 0x110 {
            let hit = self.deck_active_output_geometry().and_then(|geo| {
                let size_inputs = self.deck_layout_size_inputs();
                let layouts = crate::app_switcher::compute_deck_layout(
                    geo,
                    self.app_switcher.deck.visual_focus,
                    &size_inputs,
                );
                crate::app_switcher::hit_test_deck_card(&layouts, point)
            });
            if let Some(idx) = hit {
                self.commit_app_switcher_at(idx);
            } else {
                self.dismiss_app_switcher();
            }
        }
        true
    }

    pub fn handle_app_switcher_deck_axis(&mut self, delta: f64) -> bool {
        if !self.app_switcher.is_active || !self.app_switcher_uses_deck() {
            return false;
        }
        if delta.abs() > 0.01 {
            self.step_app_switcher(delta > 0.0);
        }
        true
    }
}

/// Aplica un modo de Aero Snap / Tiling a una ventana de manera atómica
pub fn snap_window(
    state: &mut MalusState,
    window: &smithay::desktop::Window,
    mode: SnapMode,
    usable_zone: Rectangle<i32, Logical>,
) {
    let Some(surface) = window.wl_surface() else {
        return;
    };
    let surface = surface.into_owned();

    let cur_loc = state.space.element_location(window).unwrap_or_default();
    let cur_geo = window.geometry();

    // Si ya había una transición en vuelo para esta ventana, capturamos su geometría actual como punto de partida suave
    let (start_loc, start_size) = if let Some(existing) = state.snap_transitions.get(&surface) {
        existing.current_geometry(Instant::now())
    } else {
        (cur_loc, cur_geo.size)
    };

    let is_currently_snapped = is_window_snapped(window);

    // Guardar geometría previa flotante SOLO si la ventana no estaba ya snapped y su tamaño actual es válido
    if mode != SnapMode::CenterRestore
        && !is_currently_snapped
        && cur_geo.size.w > 150
        && cur_geo.size.h > 150
    {
        state
            .window_restore_geometries
            .insert(surface.clone(), (start_loc, start_size));
    }

    let ssd_top = if window_uses_ssd(window) {
        crate::ssd::SSD_TITLEBAR_HEIGHT
    } else {
        0
    };

    let (target_loc, target_size, xdg_states) = if mode == SnapMode::CenterRestore {
        if let Some((saved_loc, saved_size)) =
            state.window_restore_geometries.get(&surface).copied()
        {
            let size_opt = if saved_size.w > 100 && saved_size.h > 100 {
                Some(saved_size)
            } else {
                None
            };
            (saved_loc, size_opt, vec![])
        } else {
            snap_target_geometry(mode, usable_zone, ssd_top)
        }
    } else {
        snap_target_geometry(mode, usable_zone, ssd_top)
    };

    let min_y = usable_zone.loc.y + ssd_top;
    let mut target_loc = target_loc;
    if target_loc.y < min_y {
        target_loc.y = min_y;
    }

    let final_target_size = target_size.unwrap_or(start_size);

    tracing::info!(
        "🪟 Aero Snap aplicado: modo={:?}, loc={:?}, size={:?}",
        mode,
        target_loc,
        target_size
    );

    // Si hay un cambio perceptible en posición o tamaño, iniciamos la transición suave
    if (start_loc != target_loc || start_size != final_target_size)
        && start_size.w > 0
        && start_size.h > 0
    {
        state.snap_transitions.insert(
            surface.clone(),
            SnapTransition::new(
                start_loc,
                start_size,
                target_loc,
                final_target_size,
                SNAP_TRANSITION_DURATION,
            ),
        );
    } else {
        state.snap_transitions.remove(&surface);
    }

    if let Some(toplevel) = window.toplevel() {
        toplevel.with_pending_state(|top_state| {
            top_state.size = target_size;
            unset_snap_states(top_state);
            for s in xdg_states {
                top_state.states.set(s);
            }
        });
        toplevel.send_configure();
    }

    state.space.map_element(window.clone(), target_loc, true);
    state.space.raise_element(window, true);
}

/// Calcula el límite máximo de ventanas en mosaico para una pantalla según su resolución lógica.
/// - Pantallas pequeñas (`width < 2000`): 2 ventanas (ej. 1366×768, 1920×1080, 4K @ 200%)
/// - Pantallas grandes (`2000 <= width < 3200`): 3 ventanas (ej. 2560×1080, 2560×1440, 4K @ 150%)
/// - Pantallas enormes (`width >= 3200`): 6 ventanas (ej. 3440×1440, 3840×2160, 5120×1440)
pub fn auto_tiling_capacity_for_size(size: Size<i32, Logical>) -> usize {
    if size.w >= 3200 {
        6
    } else if size.w >= 2000 {
        3
    } else {
        2
    }
}

/// Destino elegido por el planificador de Auto-Tiling para una ventana recién abierta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewWindowTileTarget {
    /// Mantener en el escritorio activo y colocar en el monitor `output_idx`.
    CurrentWorkspace { output_idx: usize },
    /// Mover a un escritorio virtual existente `workspace_idx` (y cambiar a él).
    ExistingWorkspace { workspace_idx: usize },
    /// Crear un nuevo escritorio virtual en `workspace_idx` y mover ahí (y cambiar a él).
    CreateWorkspace { workspace_idx: usize },
    /// Todos los escritorios hasta `max_workspaces` están llenos: fallback en el escritorio actual
    /// sobre el monitor menos cargado para no perder la ventana.
    FallbackWorkspace { workspace_idx: usize, output_idx: usize },
}

/// Planifica dónde debe ubicarse una ventana recién abierta respetando:
/// 1) Capacidad individual de su monitor preferido;
/// 2) Resto de monitores conectados en el mismo escritorio activo;
/// 3) Desbordamiento a otro escritorio virtual (creando uno nuevo hasta `max_workspaces`).
pub fn plan_new_window_placement(
    output_capacities: &[usize],
    preferred_output_idx: usize,
    active_output_counts: &[usize],
    active_ws: usize,
    workspace_window_counts: &[usize],
    max_workspaces: usize,
) -> NewWindowTileTarget {
    if output_capacities.is_empty() {
        return NewWindowTileTarget::CurrentWorkspace { output_idx: 0 };
    }

    let pref = preferred_output_idx.min(output_capacities.len() - 1);
    if active_output_counts.get(pref).copied().unwrap_or(0) < output_capacities[pref] {
        return NewWindowTileTarget::CurrentWorkspace { output_idx: pref };
    }

    for (idx, &cap) in output_capacities.iter().enumerate() {
        if idx == pref {
            continue;
        }
        if active_output_counts.get(idx).copied().unwrap_or(0) < cap {
            return NewWindowTileTarget::CurrentWorkspace { output_idx: idx };
        }
    }

    let ws_cap: usize = output_capacities.iter().sum::<usize>().max(1);
    let ws_len = workspace_window_counts.len();

    // 1. Buscar un escritorio posterior existente con espacio disponible.
    for (ws_idx, &count) in workspace_window_counts
        .iter()
        .enumerate()
        .skip(active_ws.saturating_add(1))
    {
        if count < ws_cap {
            return NewWindowTileTarget::ExistingWorkspace {
                workspace_idx: ws_idx,
            };
        }
    }

    // 2. Si todavía podemos crear un escritorio virtual nuevo, crearlo hacia adelante.
    if ws_len < max_workspaces {
        return NewWindowTileTarget::CreateWorkspace {
            workspace_idx: ws_len,
        };
    }

    // 3. Con todos los escritorios creados, revisar escritorios anteriores con espacio libre.
    for (ws_idx, &count) in workspace_window_counts
        .iter()
        .enumerate()
        .take(active_ws.min(ws_len))
    {
        if count < ws_cap {
            return NewWindowTileTarget::ExistingWorkspace {
                workspace_idx: ws_idx,
            };
        }
    }

    // 4. Fallback seguro si todos los escritorios están llenos: monitor menos cargado del escritorio activo.
    let least_loaded = (0..output_capacities.len())
        .min_by_key(|&idx| active_output_counts.get(idx).copied().unwrap_or(0))
        .unwrap_or(0);
    NewWindowTileTarget::FallbackWorkspace {
        workspace_idx: active_ws,
        output_idx: least_loaded,
    }
}

/// Plan puro de rebalanceo del escritorio activo entre monitores y desbordamiento a otros escritorios.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebalancePlan {
    /// Para cada monitor `i`, índices de las ventanas asignadas a él.
    pub per_output: Vec<Vec<usize>>,
    /// Ventanas excedentes que deben moverse a otros escritorios: `(window_idx, target_ws_idx)`.
    pub overflow_to_workspaces: Vec<(usize, usize)>,
}

/// Rebalancea una lista ordenada de ventanas del escritorio activo respetando las capacidades
/// por pantalla (`capacities`), llenando primero monitores libres del mismo escritorio y enviando
/// el resto a escritorios virtuales disponibles (creando hasta `max_workspaces`).
pub fn plan_active_workspace_rebalance(
    capacities: &[usize],
    initial_assignments: &[usize],
    active_ws: usize,
    workspace_window_counts: &[usize],
    max_workspaces: usize,
) -> RebalancePlan {
    let mut per_output: Vec<Vec<usize>> = vec![Vec::new(); capacities.len()];
    if capacities.is_empty() || initial_assignments.is_empty() {
        return RebalancePlan {
            per_output,
            overflow_to_workspaces: Vec::new(),
        };
    }

    // Paso 1: conservar cada ventana en su monitor actual mientras no exceda su capacidad.
    let mut excess = Vec::new();
    for (win_idx, &mon_idx) in initial_assignments.iter().enumerate() {
        let m = if mon_idx < capacities.len() {
            mon_idx
        } else {
            0
        };
        if per_output[m].len() < capacities[m] {
            per_output[m].push(win_idx);
        } else {
            excess.push(win_idx);
        }
    }

    // Paso 2: repartir excedentes en otros monitores conectados del mismo escritorio.
    let mut ws_overflow = Vec::new();
    for win_idx in excess {
        if let Some(free_mon) = (0..capacities.len()).find(|&m| per_output[m].len() < capacities[m])
        {
            per_output[free_mon].push(win_idx);
        } else {
            ws_overflow.push(win_idx);
        }
    }

    // Paso 3: desbordar el remanente a otros escritorios virtuales.
    let mut overflow_to_workspaces = Vec::new();
    if !ws_overflow.is_empty() {
        let ws_cap: usize = capacities.iter().sum::<usize>().max(1);
        let mut simulated_counts = workspace_window_counts.to_vec();
        if simulated_counts.len() <= active_ws {
            simulated_counts.resize(active_ws + 1, 0);
        }

        for win_idx in ws_overflow {
            let target_ws = ((active_ws + 1)..simulated_counts.len())
                .find(|&ws| simulated_counts[ws] < ws_cap)
                .or_else(|| {
                    if simulated_counts.len() < max_workspaces {
                        simulated_counts.push(0);
                        Some(simulated_counts.len() - 1)
                    } else {
                        None
                    }
                })
                .or_else(|| (0..active_ws).find(|&ws| simulated_counts[ws] < ws_cap));

            if let Some(ws) = target_ws {
                simulated_counts[ws] += 1;
                overflow_to_workspaces.push((win_idx, ws));
            } else {
                // Todos los escritorios hasta `max_workspaces` llenos: degradar en monitor menos cargado.
                let least_loaded = (0..capacities.len())
                    .min_by_key(|&m| per_output[m].len())
                    .unwrap_or(0);
                per_output[least_loaded].push(win_idx);
            }
        }
    }

    RebalancePlan {
        per_output,
        overflow_to_workspaces,
    }
}

/// Calcula rectángulos mosaico no solapados dentro de `usable_zone` para `count` ventanas.
pub fn compute_auto_tiling_slots(
    count: usize,
    usable_zone: Rectangle<i32, Logical>,
    gap: i32,
) -> Vec<Rectangle<i32, Logical>> {
    if count == 0 {
        return Vec::new();
    }
    let ux = usable_zone.loc.x + gap;
    let uy = usable_zone.loc.y + gap;
    let uw = (usable_zone.size.w - 2 * gap).max(200);
    let uh = (usable_zone.size.h - 2 * gap).max(200);

    if count == 1 {
        return vec![Rectangle::new((ux, uy).into(), (uw, uh).into())];
    }
    if count == 2 {
        let lw = (uw - gap) / 2;
        let rw = uw - gap - lw;
        return vec![
            Rectangle::new((ux, uy).into(), (lw, uh).into()),
            Rectangle::new((ux + lw + gap, uy).into(), (rw, uh).into()),
        ];
    }
    if count == 3 {
        // En monitores ultrawide (relación >= 2.1:1) usar 3 columnas verticales equilibradas;
        // en pantallas 16:9 / 16:10 usar maestro (50% izq) + 2 apiladas (der).
        let aspect = uw as f64 / uh.max(1) as f64;
        if aspect >= 2.1 {
            let col_w = (uw - 2 * gap) / 3;
            let last_w = uw - 2 * gap - 2 * col_w;
            return vec![
                Rectangle::new((ux, uy).into(), (col_w, uh).into()),
                Rectangle::new((ux + col_w + gap, uy).into(), (col_w, uh).into()),
                Rectangle::new((ux + 2 * (col_w + gap), uy).into(), (last_w, uh).into()),
            ];
        }
        let lw = (uw - gap) / 2;
        let rw = uw - gap - lw;
        let th = (uh - gap) / 2;
        let bh = uh - gap - th;
        return vec![
            Rectangle::new((ux, uy).into(), (lw, uh).into()),
            Rectangle::new((ux + lw + gap, uy).into(), (rw, th).into()),
            Rectangle::new((ux + lw + gap, uy + th + gap).into(), (rw, bh).into()),
        ];
    }

    let cols = (count as f64).sqrt().ceil() as usize;
    let rows = count.div_ceil(cols);
    let mut rects = Vec::with_capacity(count);
    let cell_h = (uh - gap * (rows as i32 - 1)) / rows as i32;
    let mut remaining = count;
    for r in 0..rows {
        let row_cols = remaining.min(cols);
        remaining -= row_cols;
        let y = uy + r as i32 * (cell_h + gap);
        let h = if r + 1 == rows {
            uh - (y - uy)
        } else {
            cell_h
        };
        let cell_w = (uw - gap * (row_cols as i32 - 1)) / row_cols as i32;
        for c in 0..row_cols {
            let x = ux + c as i32 * (cell_w + gap);
            let w = if c + 1 == row_cols {
                uw - (x - ux)
            } else {
                cell_w
            };
            rects.push(Rectangle::new((x, y).into(), (w.max(120), h.max(120)).into()));
        }
    }
    rects
}

fn apply_tile_rect(
    state: &mut MalusState,
    window: &smithay::desktop::Window,
    rect: Rectangle<i32, Logical>,
) {
    let Some(surface) = window.wl_surface() else {
        return;
    };
    let surface = surface.into_owned();
    let cur_loc = state.space.element_location(window).unwrap_or_default();
    let cur_geo = window.geometry();
    let (start_loc, start_size) = if let Some(existing) = state.snap_transitions.get(&surface) {
        existing.current_geometry(Instant::now())
    } else {
        (cur_loc, cur_geo.size)
    };
    if !state.window_restore_geometries.contains_key(&surface)
        && !is_window_snapped(window)
        && cur_geo.size.w > 150
        && cur_geo.size.h > 150
    {
        state
            .window_restore_geometries
            .insert(surface.clone(), (start_loc, start_size));
    }

    let ssd_top = if window_uses_ssd(window) {
        crate::ssd::SSD_TITLEBAR_HEIGHT
    } else {
        0
    };
    let target_loc = Point::from((rect.loc.x, rect.loc.y + ssd_top));
    let target_size = Size::from((rect.size.w.max(120), (rect.size.h - ssd_top).max(120)));

    state.initially_placed_windows.insert(surface.clone());

    if (start_loc != target_loc || start_size != target_size)
        && start_size.w > 0
        && start_size.h > 0
    {
        state.snap_transitions.insert(
            surface.clone(),
            SnapTransition::new(
                start_loc,
                start_size,
                target_loc,
                target_size,
                SNAP_TRANSITION_DURATION,
            ),
        );
    } else {
        state.snap_transitions.remove(&surface);
    }

    if let Some(toplevel) = window.toplevel() {
        toplevel.with_pending_state(|top_state| {
            top_state.size = Some(target_size);
            unset_snap_states(top_state);
            top_state.states.set(XdgState::TiledLeft);
            top_state.states.set(XdgState::TiledRight);
            top_state.states.set(XdgState::TiledTop);
            top_state.states.set(XdgState::TiledBottom);
        });
        toplevel.send_configure();
    } else if let Some(x11) = window.x11_surface() {
        let _ = x11.set_maximized(false);
        let _ = x11.configure(Some(Rectangle::new(target_loc, target_size)));
    }
    state.space.map_element(window.clone(), target_loc, false);
}

fn sync_window_output_and_scale(
    state: &MalusState,
    window: &smithay::desktop::Window,
    target_output: &Output,
) {
    let Some(wl_surf) = window.wl_surface() else {
        return;
    };
    let target_scale = target_output.current_scale().fractional_scale();
    let other_outputs: Vec<Output> = state
        .space
        .outputs()
        .filter(|o| *o != target_output)
        .cloned()
        .collect();
    smithay::wayland::compositor::with_surface_tree_downward(
        wl_surf.as_ref(),
        (),
        |_, _, _| smithay::wayland::compositor::TraversalAction::DoChildren(()),
        |surf, states, _| {
            for old in &other_outputs {
                old.leave(surf);
            }
            target_output.enter(surf);
            with_fractional_scale(states, |fs| {
                fs.set_preferred_scale(target_scale);
            });
        },
        |_, _, _| true,
    );
}

fn is_window_tileable(state: &MalusState, window: &smithay::desktop::Window) -> bool {
    let Some(surf) = window.wl_surface() else {
        return false;
    };
    if state.minimized_windows.contains_key(surf.as_ref()) {
        return false;
    }
    let is_fullscreen = window
        .toplevel()
        .map(|t| t.current_state().states.contains(XdgState::Fullscreen))
        .or_else(|| window.x11_surface().map(|x| x.is_fullscreen()))
        .unwrap_or(false);
    if is_fullscreen {
        return false;
    }
    if let Some(toplevel) = window.toplevel() {
        if toplevel.parent().is_some() {
            return false;
        }
        let geo = window.geometry();
        if geo.size.w <= 1 || geo.size.h <= 1 {
            return false;
        }
    }
    if let Some(x11) = window.x11_surface() {
        if x11.is_override_redirect() || x11.is_popup() || x11.is_transient_for().is_some() {
            return false;
        }
    }
    if let Some(id) = app_id_for_surface(surf.as_ref()) {
        let lower = id.to_lowercase();
        if is_quicklook_identifier(&lower) || lower.contains("malus-de-dialogs") {
            return false;
        }
    }
    true
}

impl MalusState {
    fn ordered_auto_tiling_outputs(&self) -> Vec<Output> {
        let primary = self.primary_output();
        let mut outputs: Vec<Output> = self.space.outputs().cloned().collect();
        outputs.sort_by(|a, b| {
            let a_primary = primary.as_ref().is_some_and(|p| p == a);
            let b_primary = primary.as_ref().is_some_and(|p| p == b);
            b_primary
                .cmp(&a_primary)
                .then_with(|| {
                    let ga = self.space.output_geometry(a).unwrap_or_default();
                    let gb = self.space.output_geometry(b).unwrap_or_default();
                    (ga.loc.x, ga.loc.y).cmp(&(gb.loc.x, gb.loc.y))
                })
                .then_with(|| a.name().cmp(&b.name()))
        });
        outputs
    }

    fn auto_tiling_capacity_for_output(&self, output: &Output) -> usize {
        let size = self
            .space
            .output_geometry(output)
            .map(|g| g.size)
            .unwrap_or_else(|| Size::from((1920, 1080)));
        auto_tiling_capacity_for_size(size)
    }

    fn tileable_window_counts_per_workspace(
        &self,
        exclude_active: Option<&smithay::desktop::Window>,
    ) -> Vec<usize> {
        let active_ws = self.workspaces.active_index;
        self.workspaces
            .workspaces
            .iter()
            .enumerate()
            .map(|(idx, ws)| {
                if idx == active_ws {
                    self.space
                        .elements()
                        .filter(|w| {
                            exclude_active.is_none_or(|ex| ex != *w) && is_window_tileable(self, w)
                        })
                        .count()
                } else {
                    ws.hidden_windows
                        .iter()
                        .filter(|entry| is_window_tileable(self, &entry.window))
                        .count()
                }
            })
            .collect()
    }

    fn move_window_to_workspace_for_tiling(
        &mut self,
        window: smithay::desktop::Window,
        target_index: usize,
        target_loc: Point<i32, Logical>,
    ) {
        if target_index >= self.workspaces.count() || target_index == self.workspaces.active_index {
            return;
        }
        if let Some(surf) = window.wl_surface() {
            self.snap_transitions.remove(surf.as_ref());
            self.initially_placed_windows.insert(surf.into_owned());
        }
        self.space.unmap_elem(&window);
        self.workspaces.workspaces[target_index]
            .hidden_windows
            .push(crate::workspaces::WorkspaceWindow {
                window,
                location: target_loc,
            });
        self.workspace_revision = self.workspace_revision.wrapping_add(1);
    }

    /// Recalcula y aplica el mosaico automático en cada pantalla activa si `auto_tiling_enabled` está activo,
    /// respetando el límite por resolución de cada pantalla, llenando monitores secundarios antes de
    /// desbordar a otros escritorios virtuales.
    pub fn relayout_auto_tiling(&mut self) {
        if !self.auto_tiling_enabled {
            return;
        }
        let outputs = self.ordered_auto_tiling_outputs();
        if outputs.is_empty() {
            return;
        }
        let capacities: Vec<usize> = outputs
            .iter()
            .map(|o| self.auto_tiling_capacity_for_output(o))
            .collect();

        let mut tileable: Vec<smithay::desktop::Window> = self
            .space
            .elements()
            .filter(|w| is_window_tileable(self, w))
            .cloned()
            .collect();
        if tileable.is_empty() {
            return;
        }

        // Orden estable por window_id ascendente para no reordenar ventanas al cambiar el foco.
        tileable.sort_by_key(|w| {
            w.wl_surface()
                .and_then(|s| self.window_ids.get(s.as_ref()).copied())
                .unwrap_or(u64::MAX)
        });

        let initial_assignments: Vec<usize> = tileable
            .iter()
            .map(|w| {
                self.origin_output_for_window(w)
                    .and_then(|o| outputs.iter().position(|cand| cand == &o))
                    .unwrap_or(0)
            })
            .collect();

        let active_ws = self.workspaces.active_index;
        let workspace_counts = self.tileable_window_counts_per_workspace(None);
        let plan = plan_active_workspace_rebalance(
            &capacities,
            &initial_assignments,
            active_ws,
            &workspace_counts,
            crate::workspaces::MAX_WORKSPACES,
        );

        let primary_spawn_loc = self.usable_zone_for_output(&outputs[0]).loc;
        let mut moved_any_workspace = false;
        for (win_idx, target_ws) in plan.overflow_to_workspaces {
            while self.workspaces.count() <= target_ws {
                if self.workspaces.add_workspace().is_none() {
                    break;
                }
            }
            if target_ws < self.workspaces.count() && target_ws != active_ws {
                let win = tileable[win_idx].clone();
                sync_window_output_and_scale(self, &win, &outputs[0]);
                self.move_window_to_workspace_for_tiling(win, target_ws, primary_spawn_loc);
                moved_any_workspace = true;
            }
        }

        let gap = if self.desktop_dock_cfg.tiled_windows_have_margins {
            8
        } else {
            2
        };

        for (out_idx, win_indices) in plan.per_output.into_iter().enumerate() {
            if win_indices.is_empty() {
                continue;
            }
            let output = &outputs[out_idx];
            let usable = self.usable_zone_for_output(output);
            let slots = compute_auto_tiling_slots(win_indices.len(), usable, gap);
            for (win_idx, rect) in win_indices.into_iter().zip(slots.into_iter()) {
                let win = &tileable[win_idx];
                sync_window_output_and_scale(self, win, output);
                apply_tile_rect(self, win, rect);
            }
        }

        if moved_any_workspace {
            self.space.refresh();
            self.notify_running_apps();
            if self.expose.active {
                self.refresh_expose_layout();
            }
        }
        self.surface_commit_counter = self.surface_commit_counter.wrapping_add(1);
    }

    /// Coloca una ventana recién abierta aplicando el límite por resolución de cada pantalla,
    /// llenando pantallas secundarias antes de pasar a otro escritorio virtual (al cual cambia automáticamente).
    pub fn place_new_window_with_auto_tiling(&mut self, window: &smithay::desktop::Window) {
        if !self.auto_tiling_enabled || !is_window_tileable(self, window) {
            return;
        }
        if let Some(surf) = window.wl_surface() {
            self.get_or_assign_window_id(surf.as_ref());
            self.initially_placed_windows.insert(surf.into_owned());
        }

        let outputs = self.ordered_auto_tiling_outputs();
        if outputs.is_empty() {
            return;
        }
        let capacities: Vec<usize> = outputs
            .iter()
            .map(|o| self.auto_tiling_capacity_for_output(o))
            .collect();

        let preferred_output_idx = self
            .origin_output_for_window(window)
            .and_then(|o| outputs.iter().position(|cand| cand == &o))
            .unwrap_or(0);

        let mut active_output_counts = vec![0usize; outputs.len()];
        for other in self.space.elements() {
            if other == window || !is_window_tileable(self, other) {
                continue;
            }
            let idx = self
                .origin_output_for_window(other)
                .and_then(|o| outputs.iter().position(|cand| cand == &o))
                .unwrap_or(0);
            active_output_counts[idx] += 1;
        }

        let active_ws = self.workspaces.active_index;
        let workspace_counts = self.tileable_window_counts_per_workspace(Some(window));
        let target = plan_new_window_placement(
            &capacities,
            preferred_output_idx,
            &active_output_counts,
            active_ws,
            &workspace_counts,
            crate::workspaces::MAX_WORKSPACES,
        );

        match target {
            NewWindowTileTarget::CurrentWorkspace { output_idx }
            | NewWindowTileTarget::FallbackWorkspace { output_idx, .. } => {
                let clamped_idx = output_idx.min(outputs.len() - 1);
                if clamped_idx != preferred_output_idx {
                    let dest_output = &outputs[clamped_idx];
                    let dest_usable = self.usable_zone_for_output(dest_output);
                    let cur_size = window.geometry().size;
                    let dest_loc = Point::from((
                        dest_usable.loc.x + ((dest_usable.size.w - cur_size.w) / 2).max(0),
                        dest_usable.loc.y + ((dest_usable.size.h - cur_size.h) / 2).max(0),
                    ));
                    sync_window_output_and_scale(self, window, dest_output);
                    self.space.map_element(window.clone(), dest_loc, false);
                    self.space.refresh();
                }
                self.relayout_auto_tiling();
            }
            NewWindowTileTarget::ExistingWorkspace { workspace_idx } => {
                let primary_loc = self.usable_zone_for_output(&outputs[0]).loc;
                sync_window_output_and_scale(self, window, &outputs[0]);
                self.move_window_to_workspace_for_tiling(
                    window.clone(),
                    workspace_idx,
                    primary_loc,
                );
                self.space.refresh();
                self.notify_running_apps();
                self.relayout_auto_tiling();
                self.switch_workspace(workspace_idx);
                tracing::info!(
                    "🧩 Auto-Tiling: pantallas llenas en Escritorio {}, nueva ventana enviada a Escritorio {}",
                    active_ws + 1,
                    workspace_idx + 1
                );
            }
            NewWindowTileTarget::CreateWorkspace { workspace_idx: _ } => {
                if let Some(new_ws) = self.workspaces.add_workspace() {
                    let primary_loc = self.usable_zone_for_output(&outputs[0]).loc;
                    sync_window_output_and_scale(self, window, &outputs[0]);
                    self.move_window_to_workspace_for_tiling(window.clone(), new_ws, primary_loc);
                    self.space.refresh();
                    self.notify_running_apps();
                    self.relayout_auto_tiling();
                    self.switch_workspace(new_ws);
                    tracing::info!(
                        "🧩 Auto-Tiling: creado Escritorio {} por desbordamiento desde Escritorio {}",
                        new_ws + 1,
                        active_ws + 1
                    );
                } else {
                    self.relayout_auto_tiling();
                }
            }
        }
    }

    /// Alterna el ordenamiento automático de ventanas (Auto-Tiling) en las pantallas activas.
    pub fn toggle_auto_tiling(&mut self) {
        self.auto_tiling_enabled = !self.auto_tiling_enabled;
        let enabled = self.auto_tiling_enabled;

        if enabled {
            self.relayout_auto_tiling();
        } else {
            let outputs: Vec<smithay::output::Output> = self.space.outputs().cloned().collect();
            let all_windows: Vec<smithay::desktop::Window> = self
                .space
                .elements()
                .filter(|w| is_window_tileable(self, w))
                .cloned()
                .collect();
            let fallback = outputs
                .first()
                .map(|o| self.usable_zone_for_output(o))
                .unwrap_or_else(|| Rectangle::new((0, 32).into(), (1920, 1000).into()));
            for win in all_windows {
                let usable = self
                    .origin_output_for_window(&win)
                    .map(|o| self.usable_zone_for_output(&o))
                    .unwrap_or(fallback);
                if let Some(x11) = win.x11_surface() {
                    if let Some(surf) = win.wl_surface() {
                        if let Some((saved_loc, saved_size)) =
                            self.window_restore_geometries.get(surf.as_ref()).copied()
                        {
                            let _ = x11.set_maximized(false);
                            let _ = x11.configure(Some(Rectangle::new(saved_loc, saved_size)));
                            self.space.map_element(win.clone(), saved_loc, false);
                        }
                    }
                } else {
                    snap_window(self, &win, SnapMode::CenterRestore, usable);
                }
            }
            self.surface_commit_counter = self.surface_commit_counter.wrapping_add(1);
        }

        self.ipc_broadcaster
            .broadcast(&IpcMessage::AutoTilingChanged { enabled });
    }

    /// Muestra el escritorio minimizando temporalmente las ventanas visibles (o las restaura).
    pub fn toggle_show_desktop(&mut self) {
        let visible_wins: Vec<(u64, smithay::wayland::shell::xdg::ToplevelSurface)> = self
            .space
            .elements()
            .filter_map(|w| {
                let surf = w.wl_surface()?;
                if self.minimized_windows.contains_key(surf.as_ref()) {
                    return None;
                }
                let toplevel = w.toplevel()?.clone();
                let id = self.window_ids.get(surf.as_ref()).copied()?;
                Some((id, toplevel))
            })
            .collect();

        if !visible_wins.is_empty() {
            let mut ids = Vec::with_capacity(visible_wins.len());
            for (id, toplevel) in visible_wins {
                ids.push(id);
                self.minimize_toplevel(&toplevel);
            }
            self.show_desktop_hidden_ids = ids;
        } else {
            let to_restore = std::mem::take(&mut self.show_desktop_hidden_ids);
            for id in to_restore {
                self.restore_window_by_id(id);
            }
        }
    }

    /// Evalúa las esquinas activas (`InputConfig.hot_corners`) al mover el puntero.
    pub fn check_hot_corners(&mut self, point: Point<f64, Logical>) {
        use malus_de_common::config::HotCornerAction;
        if !self.input_cfg.hot_corners.enabled || self.is_session_locked() {
            self.hot_corner_hover = None;
            return;
        }
        let Some(output) = self
            .space
            .outputs()
            .find(|o| {
                self.space
                    .output_geometry(o)
                    .is_some_and(|g| g.to_f64().contains(point))
            })
            .cloned()
        else {
            self.hot_corner_hover = None;
            return;
        };
        let Some(geo) = self.space.output_geometry(&output) else {
            return;
        };
        let thr = 6.0;
        let near_l = (point.x - geo.loc.x as f64) <= thr;
        let near_r = ((geo.loc.x + geo.size.w) as f64 - point.x) <= thr;
        let near_t = (point.y - geo.loc.y as f64) <= thr;
        let near_b = ((geo.loc.y + geo.size.h) as f64 - point.y) <= thr;

        let hit: Option<(&'static str, HotCornerAction)> = if near_l && near_t {
            Some(("tl", self.input_cfg.hot_corners.top_left))
        } else if near_r && near_t {
            Some(("tr", self.input_cfg.hot_corners.top_right))
        } else if near_l && near_b {
            Some(("bl", self.input_cfg.hot_corners.bottom_left))
        } else if near_r && near_b {
            Some(("br", self.input_cfg.hot_corners.bottom_right))
        } else {
            None
        };

        let Some((tag, action)) = hit else {
            self.hot_corner_hover = None;
            return;
        };
        if action == HotCornerAction::None {
            self.hot_corner_hover = None;
            return;
        }

        let delay = Duration::from_millis(self.input_cfg.hot_corners.delay_ms.max(50) as u64);
        let now = Instant::now();
        match self.hot_corner_hover {
            Some((prev_tag, entered, fired)) if prev_tag == tag => {
                if !fired && now.saturating_duration_since(entered) >= delay {
                    self.hot_corner_hover = Some((tag, entered, true));
                    match action {
                        HotCornerAction::MissionControl => {
                            self.toggle_expose(crate::expose::ExposeMode::Global);
                        }
                        HotCornerAction::ShowDesktop => {
                            self.toggle_show_desktop();
                        }
                        HotCornerAction::LockScreen => {
                            self.lock_session();
                        }
                        HotCornerAction::NotificationCenter | HotCornerAction::None => {}
                    }
                }
            }
            _ => {
                self.hot_corner_hover = Some((tag, now, false));
            }
        }
    }
}

// Delegaciones de protocolos de Smithay
delegate_compositor!(MalusState);
delegate_shm!(MalusState);
delegate_dmabuf!(MalusState);
delegate_seat!(MalusState);
delegate_xdg_shell!(MalusState);
delegate_layer_shell!(MalusState);
delegate_output!(MalusState);
delegate_data_device!(MalusState);
delegate_primary_selection!(MalusState);
delegate_data_control!(MalusState);
delegate_fractional_scale!(MalusState);
delegate_viewporter!(MalusState);
delegate_presentation!(MalusState);
delegate_xdg_decoration!(MalusState);

impl smithay::wayland::shell::xdg::decoration::XdgDecorationHandler for MalusState {
    fn new_decoration(&mut self, toplevel: smithay::wayland::shell::xdg::ToplevelSurface) {
        use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        // Preferir barra nativa de Gnome (CSD) con controles semáforo.
        // Solo las ventanas que negocien y soliciten explícitamente ServerSide recibirán barra SSD.
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ClientSide);
        });
        toplevel.send_configure();
    }
    fn request_mode(
        &mut self,
        toplevel: smithay::wayland::shell::xdg::ToplevelSurface,
        mode: smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    ) {
        use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        let win_opt = self.window_for_surface(toplevel.wl_surface());
        let cannot_have_ssd = win_opt
            .as_ref()
            .is_some_and(|w| is_browser_window(w) || is_gnome_window(w));
        let effective_mode = if cannot_have_ssd {
            Mode::ClientSide
        } else {
            mode
        };
        if effective_mode == Mode::ServerSide {
            if let Some(ref win) = win_opt {
                if let Some(loc) = self.space.element_location(win) {
                    let min_y = 36 + crate::ssd::SSD_TITLEBAR_HEIGHT;
                    if loc.y < min_y {
                        self.space
                            .map_element(win.clone(), Point::from((loc.x, min_y + 8)), false);
                    }
                }
            }
        }
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(effective_mode);
        });
        toplevel.send_configure();
    }
    fn unset_mode(&mut self, toplevel: smithay::wayland::shell::xdg::ToplevelSurface) {
        use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ClientSide);
        });
        toplevel.send_configure();
    }
}

pub fn is_gnome_identity(app_id: Option<&str>, title: Option<&str>) -> bool {
    if let Some(id) = app_id {
        let lower = id.to_lowercase();
        if lower.starts_with("org.gnome.")
            || lower.contains(".gnome.")
            || lower.starts_with("gnome-")
            || lower.contains("nautilus")
            || lower.contains("control-center")
            || lower.contains("malus.settings")
            || lower.contains("malus-de-settings")
            || lower.contains("malus_de_settings")
            || lower.contains("ptyxis")
            || lower.contains("loupe")
            || lower.contains("baobab")
            || lower.contains("epiphany")
            || lower.contains("evince")
            || lower.contains("papers")
            || lower.contains("totem")
            || lower.contains("gedit")
            || lower.contains("texteditor")
            || lower.contains("calculator")
            || lower.contains("calendar")
            || lower.contains("systemmonitor")
            || lower.contains("weather")
            || lower.contains("clocks")
            || lower.contains("contacts")
            || lower.contains("characters")
            || lower.contains("diskutility")
            || lower.contains("file-roller")
            || lower.contains("seahorse")
        {
            return true;
        }
    }
    if let Some(t) = title {
        let lower = t.to_lowercase();
        if lower.contains("archivos")
            || lower.contains("carpetas")
            || lower.contains("ajustes del sistema")
            || lower.contains("configuración")
            || lower.contains("monitor del sistema")
            || lower.contains("calculadora")
            || lower.contains("calendario")
            || lower.contains("editor de texto")
        {
            return true;
        }
    }
    false
}

/// Identifica si una ventana pertenece al ecosistema GNOME / GTK con CSD nativo.
pub fn is_gnome_window(window: &smithay::desktop::Window) -> bool {
    let matches =
        |surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface| {
            let (app_id, title) = toplevel_info(surface);
            is_gnome_identity(app_id.as_deref(), title.as_deref())
        };
    window
        .wl_surface()
        .is_some_and(|surface| matches(surface.as_ref()))
        || window
            .toplevel()
            .is_some_and(|toplevel| matches(toplevel.wl_surface()))
}

pub fn is_nautilus_identity(app_id: Option<&str>, title: Option<&str>) -> bool {
    let app_matches = app_id.is_some_and(|id| {
        let lower = id.to_lowercase();
        lower.contains("nautilus") || lower.contains("org.gnome.nautilus")
    });
    let title_matches = title.is_some_and(|title| {
        let lower = title.to_lowercase();
        lower.contains("archivos") || lower.contains("carpetas")
    });
    app_matches || title_matches
}

/// Identifica si una ventana pertenece a Nautilus (Archivos / Finder).
pub fn is_nautilus_window(window: &smithay::desktop::Window) -> bool {
    let matches =
        |surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface| {
            let (app_id, title) = toplevel_info(surface);
            is_nautilus_identity(app_id.as_deref(), title.as_deref())
        };
    window
        .wl_surface()
        .is_some_and(|surface| matches(surface.as_ref()))
        || window
            .toplevel()
            .is_some_and(|toplevel| matches(toplevel.wl_surface()))
}

pub fn is_settings_identity(app_id: Option<&str>, title: Option<&str>) -> bool {
    let app_matches = app_id.is_some_and(|id| {
        let lower = id.to_lowercase();
        lower.contains("malus.settings")
            || lower.contains("malus-de-settings")
            || lower.contains("malus_de_settings")
            || lower.contains("gnome-control-center")
            || lower.contains("control-center")
    });
    let title_matches = title.is_some_and(|title| {
        let lower = title.to_lowercase();
        lower.contains("ajustes del sistema")
            || lower.contains("configuración")
            || lower.contains("settings")
    });
    app_matches || title_matches
}

/// Identifica si una ventana pertenece a Ajustes del Sistema (Configuración).
pub fn is_settings_window(window: &smithay::desktop::Window) -> bool {
    let matches =
        |surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface| {
            let (app_id, title) = toplevel_info(surface);
            is_settings_identity(app_id.as_deref(), title.as_deref())
        };
    window
        .wl_surface()
        .is_some_and(|surface| matches(surface.as_ref()))
        || window
            .toplevel()
            .is_some_and(|toplevel| matches(toplevel.wl_surface()))
}

/// Determina si una ventana tiene activadas las decoraciones del lado del servidor (SSD) estilo macOS.
pub fn window_uses_ssd(window: &smithay::desktop::Window) -> bool {
    // Aplicaciones de GNOME, Nautilus, Ajustes y navegadores NUNCA llevan barra SSD superior (usan su propia barra nativa u oculta)
    if is_browser_window(window) || is_gnome_window(window) {
        return false;
    }
    let Some(toplevel) = window.toplevel() else {
        return false;
    };
    let is_fullscreen = toplevel.current_state().states.contains(
        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen,
    );
    if is_fullscreen {
        return false;
    }
    use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
    // Solo si la ventana negoció y solicitó explícitamente Mode::ServerSide
    toplevel.current_state().decoration_mode == Some(Mode::ServerSide)
}

/// Obtiene el título visible que debe mostrarse en la barra SSD de la ventana.
pub fn window_title_for_ssd(window: &smithay::desktop::Window) -> String {
    if let Some(surf) = window.wl_surface() {
        let (app_id, title) = toplevel_info(&surf);
        if let Some(t) = title {
            if !t.trim().is_empty() {
                return t;
            }
        }
        if let Some(id) = app_id {
            if !id.trim().is_empty() {
                return id;
            }
        }
    }
    String::new()
}

impl FractionalScaleHandler for MalusState {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let scale = self.preferred_scale_for_surface(&surface);
        smithay::wayland::compositor::with_states(&surface, |states| {
            with_fractional_scale(states, |fractional_scale| {
                fractional_scale.set_preferred_scale(scale);
            });
        });
    }
}

impl MalusState {
    /// Detecta si el punto global cae sobre la barra de título SSD de alguna ventana (respetando orden Z).
    pub fn ssd_titlebar_under(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<(
        smithay::desktop::Window,
        Point<i32, Logical>,
        crate::ssd::SsdHit,
        f64,
        f64,
        f64,
    )> {
        for window in self.space.elements().rev() {
            let Some(loc) = self.space.element_location(window) else {
                continue;
            };
            let geo = window.geometry();
            if geo.size.w <= 0 || geo.size.h <= 0 {
                continue;
            }
            if window_uses_ssd(window) {
                let tb_x = loc.x as f64;
                let tb_y = (loc.y - crate::ssd::SSD_TITLEBAR_HEIGHT) as f64;
                let tb_w = geo.size.w as f64;
                let tb_h = crate::ssd::SSD_TITLEBAR_HEIGHT as f64;
                if point.x >= tb_x
                    && point.x <= tb_x + tb_w
                    && point.y >= tb_y
                    && point.y <= tb_y + tb_h
                {
                    let lx = point.x - tb_x;
                    let ly = point.y - tb_y;
                    if let Some(hit) = crate::ssd::hit_test_titlebar_with_layout(
                        lx,
                        ly,
                        tb_w,
                        self.ssd.buttons_right,
                    ) {
                        return Some((window.clone(), loc, hit, lx, ly, tb_w));
                    }
                }
            }
            let body_rect = Rectangle::new(loc, geo.size).to_f64();
            if body_rect.contains(point) {
                return None;
            }
        }
        None
    }

    pub fn is_pointer_on_dock_auto_hide_zone(
        point: Point<f64, Logical>,
        output_geo: Rectangle<i32, Logical>,
        position: malus_de_common::config::DockPosition,
        floating_gap: i32,
        top_bar_reserved: i32,
        dock_geo: Option<Rectangle<i32, Logical>>,
    ) -> bool {
        use malus_de_common::config::DockPosition;
        let ox = output_geo.loc.x as f64;
        let oy = output_geo.loc.y as f64;
        let ow = output_geo.size.w as f64;
        let oh = output_geo.size.h as f64;
        let edge_px = 5.0;
        let gap = floating_gap.max(0) as f64;
        let top_h = top_bar_reserved.max(0) as f64;

        let on_edge = match position {
            DockPosition::Bottom => {
                point.x >= ox && point.x <= ox + ow && point.y >= oy + oh - edge_px
            }
            DockPosition::Left => {
                point.x >= ox - 1.0
                    && point.x <= ox + edge_px
                    && point.y >= oy + top_h
                    && point.y <= oy + oh
            }
            DockPosition::Right => {
                point.x >= ox + ow - edge_px
                    && point.x <= ox + ow + 1.0
                    && point.y >= oy + top_h
                    && point.y <= oy + oh
            }
        };
        if on_edge {
            return true;
        }

        if let Some(dg) = dock_geo {
            let is_collapsed = match position {
                DockPosition::Bottom => dg.size.h <= 12,
                DockPosition::Left | DockPosition::Right => dg.size.w <= 12,
            };
            if !is_collapsed {
                let mut x1 = dg.loc.x as f64;
                let y1 = dg.loc.y as f64;
                let mut x2 = x1 + dg.size.w as f64;
                let mut y2 = y1 + dg.size.h as f64;
                match position {
                    DockPosition::Bottom => y2 += gap + 2.0,
                    DockPosition::Left => x1 -= gap + 2.0,
                    DockPosition::Right => x2 += gap + 2.0,
                }
                if point.x >= x1 && point.x <= x2 && point.y >= y1 && point.y <= y2 {
                    return true;
                }
            }
        }
        false
    }

    pub fn update_dock_auto_hide_edge(&mut self, point: Point<f64, Logical>) {
        let should_reveal = if !self.dock_cfg.auto_hide
            || self.is_session_locked()
            || self.ssd.grab_active
            || self.spotlight.is_open()
            || self.screenshot_ui.is_open()
            || self.expose.active
        {
            false
        } else {
            let target_output = self
                .space
                .outputs()
                .find(|o| {
                    self.space
                        .output_geometry(o)
                        .map(|g| g.to_f64().contains(point))
                        .unwrap_or(false)
                })
                .cloned()
                .or_else(|| self.primary_output());
            if let Some(output) = target_output {
                if self.has_fullscreen_window_on_output(&output) {
                    false
                } else if let Some(output_geo) = self.space.output_geometry(&output) {
                    let dock_geo = self.dock_surface_geometry().map(|(geo, _)| geo);
                    let top_bar_h = (self.usable_zone_for_output(&output).loc.y - output_geo.loc.y)
                        .max(0);
                    Self::is_pointer_on_dock_auto_hide_zone(
                        point,
                        output_geo,
                        self.dock_cfg.position,
                        self.dock_cfg.floating_gap as i32,
                        top_bar_h,
                        dock_geo,
                    )
                } else {
                    false
                }
            } else {
                false
            }
        };

        if should_reveal != self.dock_auto_hide_edge_active {
            self.dock_auto_hide_edge_active = should_reveal;
            self.ipc_broadcaster
                .broadcast(&IpcMessage::DockAutoHideEdge {
                    active: should_reveal,
                });
        }
    }

    /// Actualiza el estado de hover de los botones semáforo SSD y el icono de redimensión perimetral.
    pub fn update_ssd_hover(&mut self, point: Point<f64, Logical>) {
        self.update_dock_auto_hide_edge(point);
        if self.ssd.grab_active {
            self.snap_layouts.close();
            return;
        }
        let occluded_by_top_ui = self.spotlight.is_open()
            || self.screenshot_ui.is_open()
            || self.expose.active
            || self.popup_grab_active()
            || self.popup_under(point).is_some()
            || self.space.outputs().any(|output| {
                let Some(output_geo) = self.space.output_geometry(output) else {
                    return false;
                };
                if !output_geo.to_f64().contains(point) {
                    return false;
                }
                let output_local_point = point - output_geo.loc.to_f64();
                let layer_map = smithay::desktop::layer_map_for_output(output);
                let under_layer = layer_map
                    .layer_under(
                        smithay::wayland::shell::wlr_layer::Layer::Overlay,
                        output_local_point,
                    )
                    .filter(|l| {
                        !(l.namespace() == "malus-app-switcher"
                            && !self.app_switcher_classic_active())
                    })
                    .or_else(|| {
                        if !self.has_fullscreen_window_on_output(output) {
                            layer_map.layer_under(
                                smithay::wayland::shell::wlr_layer::Layer::Top,
                                output_local_point,
                            )
                        } else {
                            None
                        }
                    });
                under_layer.is_some_and(|layer| {
                    layer_map.layer_geometry(layer).is_some_and(|geo| {
                        let local_point = output_local_point - geo.loc.to_f64();
                        if layer.namespace() == "malus-top-bar"
                            && local_point.y >= (geo.size.h as f64).max(36.0)
                        {
                            false
                        } else {
                            layer
                                .surface_under(
                                    local_point,
                                    smithay::desktop::WindowSurfaceType::all(),
                                )
                                .is_some()
                        }
                    })
                })
            });

        self.snap_layouts.update_pointer_motion(
            &self.space,
            point,
            occluded_by_top_ui,
            self.ssd.buttons_right,
            Instant::now(),
        );
        self.check_hot_corners(point);

        if !occluded_by_top_ui {
            if self.snap_layouts.contains_point(point) {
                self.cursor_status = smithay::input::pointer::CursorImageStatus::default_named();
                self.ssd.cursor_override = false;
                self.ssd.update_hover(None, 0.0, 0.0, 0.0);
                return;
            }
            if let Some((_win, edge, _loc)) = crate::grabs::find_window_resize_edge_under(
                &self.space,
                point,
                8.0,
                self.ssd.buttons_right,
            ) {
                self.ssd.cursor_override = true;
                self.ssd.update_hover(None, 0.0, 0.0, 0.0);
                self.cursor_status = smithay::input::pointer::CursorImageStatus::Named(
                    crate::grabs::cursor_icon_for_resize_edge(edge),
                );
                return;
            }
            if let Some((win, _, _, lx, ly, w)) = self.ssd_titlebar_under(point) {
                let surf = win.wl_surface().map(|s| s.into_owned());
                self.ssd.cursor_override = true;
                self.ssd.update_hover(surf, lx, ly, w);
                self.cursor_status = smithay::input::pointer::CursorImageStatus::default_named();
                return;
            }
        }

        if self.ssd.cursor_override || self.ssd.hovered_surface.is_some() {
            self.cursor_status = smithay::input::pointer::CursorImageStatus::default_named();
            self.ssd.cursor_override = false;
            self.ssd.update_hover(None, 0.0, 0.0, 0.0);
        }
    }

    /// Avanza los temporizadores de apertura/cierre por hover del menú Snap Layouts.
    pub fn tick_snap_layouts(&mut self, now: Instant) {
        if self.spotlight.is_open()
            || self.screenshot_ui.is_open()
            || self.expose.active
            || self.ssd.grab_active
        {
            self.snap_layouts.close();
            return;
        }
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        let output_zones: Vec<(String, Rectangle<i32, Logical>, Rectangle<i32, Logical>)> = outputs
            .iter()
            .filter_map(|o| {
                self.space
                    .output_geometry(o)
                    .map(|geo| (o.name(), geo, self.usable_zone_for_output(o)))
            })
            .collect();
        let space = &self.space;
        self.snap_layouts.tick(space, now, |window, anchor_center| {
            let win_output = space.outputs_for_element(window).into_iter().next();
            let chosen = output_zones
                .iter()
                .find(|(_, geo, _)| geo.contains(anchor_center))
                .or_else(|| {
                    win_output.and_then(|wo| {
                        let wname = wo.name();
                        output_zones.iter().find(|(n, _, _)| *n == wname)
                    })
                })
                .or_else(|| output_zones.first())?;
            Some((chosen.0.clone(), chosen.2))
        });
    }

    /// Determina el nombre del monitor donde debe mostrarse el HUD OSD (bajo el cursor o monitor primario).
    pub fn preferred_osd_output_name(&self) -> String {
        if let Some(pt) = self.last_pointer_location {
            if let Some(out) = self.space.outputs().find(|o| {
                self.space
                    .output_geometry(o)
                    .is_some_and(|geo| geo.to_f64().contains(pt))
            }) {
                return out.name();
            }
        }
        if let Some(win) = self.get_active_window() {
            if let Some(out) = self.space.outputs_for_element(&win).into_iter().next() {
                return out.name();
            }
        }
        self.primary_output()
            .or_else(|| self.space.outputs().next().cloned())
            .map(|o| o.name())
            .unwrap_or_else(|| "winit".to_string())
    }

    /// Avanza los temporizadores del HUD OSD y la auto-repetición de teclas de volumen/brillo mantenidas.
    pub fn tick_media_keys(&mut self, now: Instant) {
        if let Some(repeated_action) = self.media_keys.tick(now) {
            if !self.is_session_locked() || repeated_action.is_allowed_when_locked() {
                self.execute_media_key_action(repeated_action, now);
            }
        }
    }

    /// Ejecuta una acción de tecla multimedia o de función de hardware y muestra el HUD OSD correspondiente.
    pub fn execute_media_key_action(
        &mut self,
        action: crate::media_keys::MediaKeyAction,
        now: Instant,
    ) {
        use crate::media_keys::{MediaKeyAction, OsdIconKind};

        let output_name = self.preferred_osd_output_name();
        match action {
            MediaKeyAction::VolumeUp { fine } => {
                let step = if fine { 2.0 } else { 5.0 };
                tracing::info!("🔊 [MEDIA KEY] Subir volumen (+{:.0}%)", step);
                self.media_keys.step_volume(
                    step,
                    output_name,
                    now,
                    self.ipc_broadcaster.clone(),
                );
            }
            MediaKeyAction::VolumeDown { fine } => {
                let step = if fine { -2.0 } else { -5.0 };
                tracing::info!("🔉 [MEDIA KEY] Bajar volumen ({:.0}%)", step);
                self.media_keys.step_volume(
                    step,
                    output_name,
                    now,
                    self.ipc_broadcaster.clone(),
                );
            }
            MediaKeyAction::VolumeMute => {
                tracing::info!("🔇 [MEDIA KEY] Alternar silencio de audio");
                self.media_keys
                    .toggle_volume_mute(output_name, now, self.ipc_broadcaster.clone());
            }
            MediaKeyAction::MicMute => {
                tracing::info!("🎙️ [MEDIA KEY] Alternar silencio de micrófono");
                self.media_keys.toggle_mic_mute(output_name, now);
            }
            MediaKeyAction::BrightnessUp { fine } => {
                let step = if fine { 2.0 } else { 5.0 };
                let next = self.media_keys.step_brightness(step, output_name, now);
                tracing::info!("☀️ [MEDIA KEY] Subir brillo (+{:.0}% -> {:.0}%)", step, next);
                self.pending_brightness_reload = true;
                self.ipc_broadcaster
                    .broadcast(&IpcMessage::ReloadDisplayBrightness);
            }
            MediaKeyAction::BrightnessDown { fine } => {
                let step = if fine { -2.0 } else { -5.0 };
                let next = self.media_keys.step_brightness(step, output_name, now);
                tracing::info!("🔅 [MEDIA KEY] Bajar brillo ({:.0}% -> {:.0}%)", step, next);
                self.pending_brightness_reload = true;
                self.ipc_broadcaster
                    .broadcast(&IpcMessage::ReloadDisplayBrightness);
            }
            MediaKeyAction::BrightnessCycle => {
                let next = self.media_keys.cycle_brightness(output_name, now);
                tracing::info!("☀️ [MEDIA KEY] Ciclar brillo (-> {:.0}%)", next);
                self.pending_brightness_reload = true;
                self.ipc_broadcaster
                    .broadcast(&IpcMessage::ReloadDisplayBrightness);
            }
            MediaKeyAction::KbdBrightnessUp { fine } => {
                let step = if fine { 10.0 } else { 25.0 };
                tracing::info!("⌨️ [MEDIA KEY] Subir luz de teclado (+{:.0}%)", step);
                self.media_keys.step_kbd_brightness(step, output_name, now);
            }
            MediaKeyAction::KbdBrightnessDown { fine } => {
                let step = if fine { -10.0 } else { -25.0 };
                tracing::info!("⌨️ [MEDIA KEY] Bajar luz de teclado ({:.0}%)", step);
                self.media_keys.step_kbd_brightness(step, output_name, now);
            }
            MediaKeyAction::KbdBrightnessToggle => {
                tracing::info!("⌨️ [MEDIA KEY] Alternar luz de teclado");
                self.media_keys.toggle_kbd_brightness(output_name, now);
            }
            MediaKeyAction::MediaPlayPause => {
                tracing::info!("⏯️ [MEDIA KEY] Reproducir / Pausar (MPRIS)");
                self.media_keys.trigger_mpris_command(
                    "PlayPause",
                    OsdIconKind::MediaPlayPause,
                    "Reproducir / Pausar",
                    output_name,
                    now,
                );
            }
            MediaKeyAction::MediaPause => {
                tracing::info!("⏸️ [MEDIA KEY] Pausar (MPRIS)");
                self.media_keys.trigger_mpris_command(
                    "Pause",
                    OsdIconKind::MediaPlayPause,
                    "Pausa",
                    output_name,
                    now,
                );
            }
            MediaKeyAction::MediaStop => {
                tracing::info!("⏹️ [MEDIA KEY] Detener (MPRIS)");
                self.media_keys.trigger_mpris_command(
                    "Stop",
                    OsdIconKind::MediaStop,
                    "Detenido",
                    output_name,
                    now,
                );
            }
            MediaKeyAction::MediaNext => {
                tracing::info!("⏭️ [MEDIA KEY] Siguiente pista (MPRIS)");
                self.media_keys.trigger_mpris_command(
                    "Next",
                    OsdIconKind::MediaNext,
                    "Siguiente",
                    output_name,
                    now,
                );
            }
            MediaKeyAction::MediaPrev => {
                tracing::info!("⏮️ [MEDIA KEY] Pista anterior (MPRIS)");
                self.media_keys.trigger_mpris_command(
                    "Previous",
                    OsdIconKind::MediaPrev,
                    "Anterior",
                    output_name,
                    now,
                );
            }
            MediaKeyAction::Calculator => {
                tracing::info!("🧮 [MEDIA KEY] Lanzando Calculadora / Spotlight");
                let socket = self.socket_name.clone();
                std::thread::spawn(move || {
                    for bin in ["gnome-calculator", "qalculate-gtk", "kcalc", "galculator"] {
                        let mut cmd = std::process::Command::new(bin);
                        cmd.env("WAYLAND_DISPLAY", &socket);
                        malus_de_common::config::set_cursor_environment(&mut cmd);
                        if cmd.spawn().is_ok() {
                            return;
                        }
                    }
                });
            }
            MediaKeyAction::Explorer => {
                tracing::info!("📁 [MEDIA KEY] Lanzando gestor de archivos Branches");
                let socket = self.socket_name.clone();
                std::thread::spawn(move || {
                    let local_bin = std::env::current_exe()
                        .ok()
                        .and_then(|p| p.parent().map(|d| d.join("malus-de-branches")))
                        .filter(|p| p.is_file());
                    let mut cmd = local_bin
                        .map(std::process::Command::new)
                        .unwrap_or_else(|| std::process::Command::new("malus-de-branches"));
                    cmd.env("WAYLAND_DISPLAY", &socket);
                    malus_de_common::config::set_cursor_environment(&mut cmd);
                    let _ = cmd.spawn();
                });
            }
            MediaKeyAction::Search => {
                tracing::info!("🔍 [MEDIA KEY] Tecla Search -> Alternando Spotlight");
                self.toggle_spotlight();
            }
            MediaKeyAction::MissionControl => {
                tracing::info!("🪟 [MEDIA KEY] Tecla Mission Control -> Alternando Exposé");
                self.toggle_expose(crate::expose::ExposeMode::Global);
            }
            MediaKeyAction::ScreenSaver => {
                tracing::info!("🔒 [MEDIA KEY] Tecla Bloqueo / Suspensión -> Bloqueando sesión");
                self.lock_session();
            }
            MediaKeyAction::TouchpadToggle
            | MediaKeyAction::TouchpadOn
            | MediaKeyAction::TouchpadOff => {
                let mut cfg = malus_de_common::config::InputConfig::load();
                let next_enabled = match action {
                    MediaKeyAction::TouchpadOn => true,
                    MediaKeyAction::TouchpadOff => false,
                    _ => !cfg.touchpad_enabled,
                };
                cfg.touchpad_enabled = next_enabled;
                let _ = cfg.save();
                self.input_cfg = cfg;
                self.input_config_dirty = true;
                tracing::info!(
                    "🖱️ [MEDIA KEY] Touchpad {}",
                    if next_enabled { "activado" } else { "desactivado" }
                );
                self.media_keys.show_osd(
                    output_name,
                    if next_enabled {
                        OsdIconKind::TouchpadOn
                    } else {
                        OsdIconKind::TouchpadOff
                    },
                    "Trackpad",
                    if next_enabled { "Activado" } else { "Desactivado" },
                    None,
                    !next_enabled,
                    now,
                );
            }
        }
    }

    /// Procesa clics del ratón sobre el menú flotante Snap Layouts.
    /// Devuelve `true` si el evento fue consumido por el menú.
    pub fn handle_snap_layouts_pointer_button(
        &mut self,
        point: Point<f64, Logical>,
        button_code: u32,
        is_pressed: bool,
    ) -> bool {
        match self
            .snap_layouts
            .handle_pointer_button(point, button_code, is_pressed)
        {
            crate::snap_layouts::SnapLayoutClickResult::Ignored => false,
            crate::snap_layouts::SnapLayoutClickResult::Consumed => true,
            crate::snap_layouts::SnapLayoutClickResult::Snap { window, mode } => {
                let target_output = self
                    .space
                    .outputs_for_element(&window)
                    .into_iter()
                    .next()
                    .or_else(|| {
                        self.space
                            .outputs()
                            .find(|o| {
                                self.space
                                    .output_geometry(o)
                                    .is_some_and(|g| g.to_f64().contains(point))
                            })
                            .cloned()
                    })
                    .or_else(|| self.space.outputs().next().cloned());
                let usable_zone = target_output
                    .as_ref()
                    .map(|o| self.usable_zone_for_output(o))
                    .unwrap_or_else(|| Rectangle::new((0, 32).into(), (1920, 1000).into()));
                snap_window(self, &window, mode, usable_zone);
                true
            }
        }
    }

    /// Procesa clics sobre la barra de título SSD (botones de cerrar, minimizar, maximizar, doble clic y arrastre).
    /// Devuelve `true` si el evento fue consumido por la decoración SSD.
    pub fn handle_ssd_pointer_button(
        &mut self,
        point: Point<f64, Logical>,
        button_code: u32,
        serial: Serial,
    ) -> bool {
        if button_code != 0x110 || self.popup_grab_active() {
            return false;
        }
        let Some((window, loc, hit, _, _, _)) = self.ssd_titlebar_under(point) else {
            return false;
        };
        self.snap_layouts.close();

        let was_already_top = self
            .space
            .elements()
            .last()
            .map(|w| w == &window)
            .unwrap_or(false);
        if !was_already_top {
            self.space.raise_element(&window, true);
        }
        if let Some(surface) = window.wl_surface() {
            if let Some(keyboard) = self.seat.get_keyboard() {
                if keyboard.current_focus().as_ref() != Some(surface.as_ref()) {
                    keyboard.set_focus(self, Some(surface.clone().into_owned()), serial);
                    self.notify_active_app(Some(&surface));
                } else {
                    self.sync_window_activation(Some(surface.as_ref()));
                }
            }
        }

        let target_output = self
            .space
            .outputs()
            .find(|o| {
                self.space
                    .output_geometry(o)
                    .map(|g| g.to_f64().contains(point))
                    .unwrap_or(false)
            })
            .or_else(|| self.space.outputs().next())
            .cloned();
        let usable_zone = target_output
            .as_ref()
            .map(|o| self.usable_zone_for_output(o))
            .unwrap_or_else(|| Rectangle::new((0, 32).into(), (1920, 1000).into()));

        match hit {
            crate::ssd::SsdHit::Button(crate::ssd::SsdButton::Close) => {
                self.record_close_button_hint(&window);
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_close();
                }
                true
            }
            crate::ssd::SsdHit::Button(crate::ssd::SsdButton::Minimize) => {
                if let Some(toplevel) = window.toplevel().cloned() {
                    self.minimize_toplevel(&toplevel);
                }
                true
            }
            crate::ssd::SsdHit::Button(crate::ssd::SsdButton::Maximize) => {
                let is_maximized = window
                    .toplevel()
                    .map(|t| t.current_state().states.contains(XdgState::Maximized))
                    .unwrap_or(false);
                let mode = if is_maximized {
                    SnapMode::CenterRestore
                } else {
                    SnapMode::Maximize
                };
                snap_window(self, &window, mode, usable_zone);
                true
            }
            crate::ssd::SsdHit::Titlebar => {
                let now = Instant::now();
                let is_double_click = if let Some((ref prev_surf, prev_time, prev_pt)) =
                    self.ssd.last_titlebar_click
                {
                    window.wl_surface().as_deref() == Some(prev_surf)
                        && now.saturating_duration_since(prev_time) <= Duration::from_millis(400)
                        && (point.x - prev_pt.x).hypot(point.y - prev_pt.y) <= 10.0
                } else {
                    false
                };

                if is_double_click {
                    self.ssd.last_titlebar_click = None;
                    match self.dock_cfg.titlebar_double_click_action {
                        1 => {
                            if let Some(toplevel) = window.toplevel().cloned() {
                                self.minimize_toplevel(&toplevel);
                            }
                        }
                        2 => {}
                        _ => {
                            let is_maximized = window
                                .toplevel()
                                .map(|t| t.current_state().states.contains(XdgState::Maximized))
                                .unwrap_or(false);
                            let mode = if is_maximized {
                                SnapMode::CenterRestore
                            } else {
                                SnapMode::Maximize
                            };
                            snap_window(self, &window, mode, usable_zone);
                        }
                    }
                    return true;
                }

                if let Some(surf) = window.wl_surface() {
                    self.ssd.last_titlebar_click = Some((surf.into_owned(), now, point));
                }

                if let Some(pointer) = self.seat.get_pointer() {
                    let is_snapped = is_window_snapped(&window);
                    let start_data = smithay::input::pointer::GrabStartData {
                        focus: None,
                        button: button_code,
                        location: point,
                    };
                    let grab = crate::grabs::MoveSurfaceGrab {
                        start_data,
                        window: window.clone(),
                        initial_window_location: loc,
                        is_snapped,
                        has_dragged: false,
                    };
                    self.ssd.grab_active = true;
                    self.moving_window_surface = window.wl_surface().map(|s| s.into_owned());
                    pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
                }
                true
            }
        }
    }

    /// Limpia referencias a `WlSurface` que ya han sido destruidas para evitar fugas de memoria a lo largo de la sesión
    pub fn cleanup_stale_surfaces(&mut self) {
        use smithay::utils::IsAlive;
        self.ssd.retain_alive();
        self.window_restore_geometries.retain(|s, _| s.alive());
        self.snap_transitions.retain(|s, _| s.alive());
        self.centered_transient_windows.retain(|s, _| s.alive());
        self.initially_placed_windows.retain(|s| s.alive());
        self.clamped_oversized_windows.retain(|s| s.alive());
        self.minimized_windows.retain(|s, _| s.alive());
        self.minimized_window_workspaces.retain(|s, _| s.alive());
        self.cached_close_snapshots.retain(|s, _| s.alive());
        self.ext_lock_surfaces.retain(|_, surf| surf.alive());
        self.malus_lock_layers.retain(|(_, layer)| layer.alive());
        self.workspaces.retain_alive();
        let dock_slots = &mut self.dock_slots;
        let minimized_thumbnails = &mut self.minimized_window_thumbnails;
        let minimized_windows = &self.minimized_windows;
        self.window_ids.retain(|s, win_id| {
            let alive = s.alive();
            if !alive {
                dock_slots.remove(win_id);
                if let Some(path) = minimized_thumbnails.remove(win_id) {
                    let _ = std::fs::remove_file(path);
                }
            } else if !minimized_windows.contains_key(s) {
                if let Some(path) = minimized_thumbnails.remove(win_id) {
                    let _ = std::fs::remove_file(path);
                }
            }
            alive
        });
    }

    /// Determina la escala fraccionaria preferida para una superficie según el monitor en el que reside
    pub fn preferred_scale_for_surface(&self, surface: &WlSurface) -> f64 {
        let window = self.window_for_surface(surface);
        self.preferred_scale_for_surface_with_window(surface, window.as_ref())
    }

    pub fn preferred_scale_for_surface_with_window(
        &self,
        surface: &WlSurface,
        window: Option<&smithay::desktop::Window>,
    ) -> f64 {
        if let Some(window) = window {
            if let Some(win_loc) = self.space.element_location(window) {
                let win_geo = window.geometry();
                let center = Point::<i32, Logical>::from((
                    win_loc.x + (win_geo.size.w.max(1) / 2),
                    win_loc.y + (win_geo.size.h.max(1) / 2),
                ));
                for output in self.space.outputs() {
                    if let Some(geo) = self.space.output_geometry(output) {
                        if geo.contains(center) {
                            return output.current_scale().fractional_scale();
                        }
                    }
                }
                for output in self.space.outputs() {
                    if let Some(geo) = self.space.output_geometry(output) {
                        if geo.contains(win_loc) {
                            return output.current_scale().fractional_scale();
                        }
                    }
                }
            }
            if let Some(output) = self.space.outputs_for_element(window).first() {
                return output.current_scale().fractional_scale();
            }
        }

        for output in self.space.outputs() {
            let map = smithay::desktop::layer_map_for_output(output);
            if map
                .layer_for_surface(surface, smithay::desktop::WindowSurfaceType::all())
                .is_some()
            {
                return output.current_scale().fractional_scale();
            }
        }

        if let Some(output) = self.space.outputs().next() {
            output.current_scale().fractional_scale()
        } else {
            1.0
        }
    }

    /// Obtiene el monitor configurado como principal en `displays_cfg` (`primary = true`),
    /// o el primer monitor disponible en Space si no se encuentra configurado.
    pub fn primary_output_for_config(
        &self,
        displays_cfg: &malus_de_common::config::DisplaysConfig,
    ) -> Option<Output> {
        if let Some(primary_cfg) = displays_cfg.outputs.iter().find(|o| o.primary) {
            // 1. Por connector name
            if let Some(output) = self
                .space
                .outputs()
                .find(|o| o.name() == primary_cfg.connector)
            {
                return Some(output.clone());
            }
            // 2. Por descripción o modelo
            if let Some(output) = self.space.outputs().find(|o| {
                let desc = o.description();
                desc.contains(&primary_cfg.name) || desc.contains(&primary_cfg.connector)
            }) {
                return Some(output.clone());
            }
            // 3. Por geometría
            if let Some(output) = self.space.outputs().find(|o| {
                if let Some(geo) = self.space.output_geometry(o) {
                    geo.loc.x == primary_cfg.x && geo.loc.y == primary_cfg.y
                } else {
                    false
                }
            }) {
                return Some(output.clone());
            }
        }
        self.space.outputs().next().cloned()
    }

    /// Obtiene el monitor configurado como principal (`primary = true en DisplaysConfig`),
    /// o el primer monitor disponible en Space si no se encuentra configurado.
    pub fn primary_output(&self) -> Option<Output> {
        let displays_cfg = malus_de_common::config::DisplaysConfig::load();
        self.primary_output_for_config(&displays_cfg)
    }

    /// Reubica de inmediato las capas principales del shell (`malus-top-bar`, `malus-dock` y
    /// `malus-app-switcher`) al monitor principal definido en `displays_cfg`.
    pub fn relocate_primary_shell_layers_for_config(
        &mut self,
        displays_cfg: &malus_de_common::config::DisplaysConfig,
        preserved_layers: Vec<smithay::desktop::LayerSurface>,
    ) -> bool {
        let Some(primary_output) = self.primary_output_for_config(displays_cfg) else {
            for layer in preserved_layers {
                layer.layer_surface().send_close();
            }
            return false;
        };

        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        let mut to_move: Vec<(Output, smithay::desktop::LayerSurface)> = Vec::new();
        for output in &outputs {
            if output == &primary_output {
                continue;
            }
            let map = layer_map_for_output(output);
            for layer in map.layers() {
                if is_primary_shell_layer_namespace(layer.namespace()) {
                    to_move.push((output.clone(), layer.clone()));
                }
            }
        }

        if to_move.is_empty() && preserved_layers.is_empty() {
            return false;
        }

        for (old_output, layer) in &to_move {
            let mut old_map = layer_map_for_output(old_output);
            old_map.unmap_layer(layer);
            smithay::wayland::compositor::with_surface_tree_downward(
                layer.wl_surface(),
                (),
                |_, _, _| smithay::wayland::compositor::TraversalAction::DoChildren(()),
                |wl_surf, _, _| {
                    old_output.leave(wl_surf);
                },
                |_, _, _| true,
            );
        }

        let target_scale = primary_output.current_scale().fractional_scale();
        let mut moved_layers: Vec<smithay::desktop::LayerSurface> = to_move
            .into_iter()
            .map(|(_, l)| l)
            .chain(preserved_layers)
            .collect();

        {
            let mut new_map = layer_map_for_output(&primary_output);
            for layer in &mut moved_layers {
                if layer.namespace() == "malus-top-bar" {
                    layer.layer_surface().with_pending_state(|s| {
                        s.size = None;
                    });
                }
                if let Err(err) = new_map.map_layer(layer) {
                    tracing::warn!(
                        "⚠️ No se pudo reubicar layer '{}' en '{}': {:?}",
                        layer.namespace(),
                        primary_output.name(),
                        err
                    );
                }
            }
            new_map.arrange();
            for layer in &moved_layers {
                smithay::wayland::compositor::with_surface_tree_downward(
                    layer.wl_surface(),
                    (),
                    |_, _, _| smithay::wayland::compositor::TraversalAction::DoChildren(()),
                    |wl_surf, states, _| {
                        primary_output.enter(wl_surf);
                        with_fractional_scale(states, |fs| {
                            fs.set_preferred_scale(target_scale);
                        });
                    },
                    |_, _, _| true,
                );
                layer.layer_surface().send_configure();
            }
        }

        tracing::info!(
            "🖥 Capas principales (Dock / Top Bar / Switcher) trasladadas a '{}'",
            primary_output.name()
        );
        true
    }

    pub fn relocate_primary_shell_layers(&mut self) -> bool {
        let displays_cfg = malus_de_common::config::DisplaysConfig::load();
        self.relocate_primary_shell_layers_for_config(&displays_cfg, Vec::new())
    }

    /// Migra ventanas y puntero que hayan quedado fuera de las salidas activas tras desconectar
    /// un monitor, reubicándolos dentro del área útil del monitor primario y sincronizando escala.
    pub fn migrate_orphaned_windows_and_pointer(&mut self) {
        let active_geos: Vec<Rectangle<i32, Logical>> = self
            .space
            .outputs()
            .filter_map(|o| self.space.output_geometry(o))
            .collect();
        if active_geos.is_empty() {
            return;
        }
        let Some(target_output) = self
            .primary_output()
            .or_else(|| self.space.outputs().next().cloned())
        else {
            return;
        };
        let Some(target_geo) = self.space.output_geometry(&target_output) else {
            return;
        };
        let usable = self.usable_zone_for_output(&target_output);

        let windows: Vec<_> = self.space.elements().cloned().collect();
        for win in windows {
            let Some(win_loc) = self.space.element_location(&win) else {
                continue;
            };
            let geo = win.geometry();
            let center = Point::<i32, Logical>::from((
                win_loc.x + geo.loc.x + geo.size.w.max(1) / 2,
                win_loc.y + geo.loc.y + geo.size.h.max(1) / 2,
            ));
            let on_active_output = active_geos.iter().any(|g| g.contains(center));
            if !on_active_output {
                let ssd_top = if window_uses_ssd(&win) {
                    crate::ssd::SSD_TITLEBAR_HEIGHT
                } else {
                    0
                };
                let max_bounds = max_window_size_in_usable_zone(usable, ssd_top);
                let target_size = clamp_size_to_bounds(geo.size, max_bounds);
                let centered_loc =
                    centered_window_location(usable, Rectangle::new(geo.loc, target_size), ssd_top);
                self.space.map_element(win.clone(), centered_loc, false);

                if let Some(wl_surf) = win.wl_surface() {
                    if let Some(entry) = self.window_restore_geometries.get_mut(wl_surf.as_ref()) {
                        *entry = (centered_loc, target_size);
                    }
                    let new_scale = self.preferred_scale_for_surface(&wl_surf);
                    smithay::wayland::compositor::with_states(&wl_surf, |states| {
                        smithay::wayland::fractional_scale::with_fractional_scale(states, |fs| {
                            fs.set_preferred_scale(new_scale);
                        });
                    });
                }
                if let Some(toplevel) = win.toplevel() {
                    let needs_resize = target_size.w != geo.size.w || target_size.h != geo.size.h;
                    toplevel.with_pending_state(|s| {
                        s.bounds = Some(max_bounds);
                        if needs_resize {
                            s.size = Some(target_size);
                        }
                    });
                    toplevel.send_configure();
                } else if let Some(x11) = win.x11_surface() {
                    let _ = x11.configure(Some(Rectangle::new(centered_loc, target_size)));
                }
            }
        }

        // Evitar que ventanas minimizadas en el monitor desconectado se restauren fuera de pantalla
        for (win, prev_loc) in self.minimized_windows.values_mut() {
            let geo = win.geometry();
            let center = Point::<i32, Logical>::from((
                prev_loc.x + geo.loc.x + geo.size.w.max(1) / 2,
                prev_loc.y + geo.loc.y + geo.size.h.max(1) / 2,
            ));
            if !active_geos.iter().any(|g| g.contains(center)) {
                let ssd_top = if window_uses_ssd(win) {
                    crate::ssd::SSD_TITLEBAR_HEIGHT
                } else {
                    0
                };
                let max_bounds = max_window_size_in_usable_zone(usable, ssd_top);
                let target_size = clamp_size_to_bounds(geo.size, max_bounds);
                *prev_loc =
                    centered_window_location(usable, Rectangle::new(geo.loc, target_size), ssd_top);
            }
        }

        let ptr_outside = self
            .last_pointer_location
            .is_none_or(|loc| !active_geos.iter().any(|g| g.to_f64().contains(loc)));
        if ptr_outside {
            let center = target_geo.loc.to_f64() + (target_geo.size.to_f64() / 2.0);
            self.last_pointer_location = Some(center);
            if let Some(pointer) = self.seat.get_pointer() {
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                pointer.motion(
                    self,
                    None,
                    &smithay::input::pointer::MotionEvent {
                        location: center,
                        serial,
                        time: 0,
                    },
                );
                pointer.frame(self);
            }
        }
    }

    /// Determina si un monitor dado corresponde a la pantalla principal del entorno.
    pub fn is_primary_output(&self, output: &Output) -> bool {
        self.primary_output().as_ref() == Some(output)
    }

    /// Calcula la zona utilizable global (en coordenadas de Space) para un monitor dado
    pub fn usable_zone_for_output(&self, output: &Output) -> Rectangle<i32, Logical> {
        let layer_map = smithay::desktop::layer_map_for_output(output);
        self.usable_zone_from_layer_map(output, &layer_map)
    }

    /// Calcula la zona utilizable reutilizando un `LayerMap` ya bloqueado.
    pub fn usable_zone_from_layer_map(
        &self,
        output: &Output,
        layer_map: &smithay::desktop::LayerMap,
    ) -> Rectangle<i32, Logical> {
        let output_geo = self.space.output_geometry(output).unwrap_or_default();
        let zone = layer_map.non_exclusive_zone();

        // En Malus DE, la Top Bar solo existe en la pantalla primaria.
        // En la pantalla primaria ocupa de manera estricta al menos 36 píxeles lógicos superiores.
        // En pantallas secundarias no hay Top Bar, por lo que no se impone margen superior artificial.
        let is_primary = self.is_primary_output(output);
        let top_bar_min_height = if is_primary { 36 } else { 0 };
        let top_offset = if zone.size.w > 0 && zone.size.h > 0 {
            if is_primary {
                zone.loc.y.max(top_bar_min_height)
            } else {
                zone.loc.y
            }
        } else {
            top_bar_min_height
        };

        let has_dock = layer_map
            .layers()
            .any(|layer| layer.namespace() == "malus-dock");
        let headroom = if has_dock {
            self.dock_blur_headroom.max(0)
        } else {
            0
        };

        let width = if zone.size.w > 0 {
            zone.size.w
        } else {
            output_geo.size.w
        };

        let raw_height = if zone.size.h > 0 {
            zone.size.h - (top_offset - zone.loc.y).max(0)
        } else {
            output_geo.size.h - top_offset
        };
        let usable_height = (raw_height + headroom)
            .min(output_geo.size.h - top_offset)
            .max(100);

        Rectangle::new(
            (
                output_geo.loc.x + zone.loc.x.max(0),
                output_geo.loc.y + top_offset,
            )
                .into(),
            (width, usable_height).into(),
        )
    }

    /// Determina la pantalla (Output) de origen donde reside principalmente una ventana.
    /// Prioriza el monitor con mayor área de intersección física con la ventana (incluyendo SSD),
    /// seguido del monitor que contiene su centro o su posición actual, o la última posición del puntero.
    pub fn origin_output_for_window(&self, window: &smithay::desktop::Window) -> Option<Output> {
        let win_loc = self.space.element_location(window).unwrap_or_default();
        let win_geo = window.geometry();
        let ssd_top = if window_uses_ssd(window) {
            crate::ssd::SSD_TITLEBAR_HEIGHT
        } else {
            0
        };
        let outputs: Vec<(Output, Rectangle<i32, Logical>)> = self
            .space
            .outputs()
            .filter_map(|o| self.space.output_geometry(o).map(|g| (o.clone(), g)))
            .collect();

        if let Some(output) = resolve_origin_output_from_bounds(
            &outputs,
            win_loc,
            win_geo.size,
            ssd_top,
            self.last_pointer_location,
        ) {
            return Some(output);
        }

        // 5. Fallback a la lista de monitores de Smithay o monitor primario
        self.space
            .outputs_for_element(window)
            .into_iter()
            .next()
            .or_else(|| self.primary_output())
    }

    /// Determina si hay una ventana en pantalla completa visible sobre un monitor específico.
    pub fn has_fullscreen_window_on_output(&self, output: &Output) -> bool {
        let Some(_output_geo) = self.space.output_geometry(output) else {
            return false;
        };
        for window in self.space.elements() {
            let is_fullscreen = window
                .toplevel()
                .map(|t| t.current_state().states.contains(XdgState::Fullscreen))
                .unwrap_or_else(|| {
                    window
                        .x11_surface()
                        .map(|x| x.is_fullscreen())
                        .unwrap_or(false)
                });
            if is_fullscreen {
                // La ventana en pantalla completa debe pertenecer estrictamente a este monitor específico.
                // Usamos `origin_output_for_window` para que una ventana en pantalla completa en un monitor
                // secundario nunca oculte la Top Bar ni el Dock del monitor principal por rozar bordes limítrofes.
                if self.origin_output_for_window(window).as_ref() == Some(output) {
                    return true;
                }
            }
        }
        false
    }

    /// Asegura que una ventana (que no esté en pantalla completa) nunca ocupe el espacio de la Top Bar.
    /// Si su coordenada Y es menor que la parte inferior de la Top Bar del monitor donde reside,
    /// ajusta su posición atómicamente para que su barra de título quede completamente visible.
    pub fn ensure_window_below_topbar(&mut self, window: &smithay::desktop::Window) {
        let is_fullscreen = window
            .toplevel()
            .map(|t| t.current_state().states.contains(XdgState::Fullscreen))
            .or_else(|| window.x11_surface().map(|s| s.is_fullscreen()))
            .unwrap_or(false);

        if is_fullscreen {
            return;
        }

        if let Some(loc) = self.space.element_location(window) {
            if let Some(output) = self.origin_output_for_window(window) {
                if !self.is_primary_output(&output) {
                    return;
                }
                let usable_zone = self.usable_zone_for_output(&output);
                let ssd_top = if window_uses_ssd(window) {
                    crate::ssd::SSD_TITLEBAR_HEIGHT
                } else {
                    0
                };
                let min_y = usable_zone.loc.y + ssd_top;
                if loc.y < min_y {
                    let clamped = Point::from((loc.x, min_y));
                    self.space.map_element(window.clone(), clamped, false);
                }
            }
        }
    }
}

/// Resuelve el monitor de origen a partir de la geometría cliente, barra SSD y última posición del puntero,
/// sin acceder al `Mutex<PointerInternal>` de Smithay (seguro bajo `PointerGrab`).
pub fn resolve_origin_output_from_bounds(
    outputs: &[(Output, Rectangle<i32, Logical>)],
    win_loc: Point<i32, Logical>,
    win_size: Size<i32, Logical>,
    ssd_top: i32,
    last_pointer_location: Option<Point<f64, Logical>>,
) -> Option<Output> {
    let win_rect = Rectangle::new(
        Point::from((win_loc.x, win_loc.y - ssd_top)),
        Size::from((win_size.w, win_size.h + ssd_top)),
    );

    // 1. Monitor con mayor área de solapamiento (cliente + barra SSD superior)
    let mut best_output = None;
    let mut max_area = 0i64;
    for (output, out_geo) in outputs {
        if let Some(intersection) = win_rect.intersection(*out_geo) {
            let area = intersection.size.w as i64 * intersection.size.h as i64;
            if area > max_area {
                max_area = area;
                best_output = Some(output.clone());
            }
        }
    }
    if best_output.is_some() {
        return best_output;
    }

    // 2. Monitor que contiene el centro de la ventana
    let center = Point::from((
        win_rect.loc.x + win_rect.size.w / 2,
        win_rect.loc.y + win_rect.size.h / 2,
    ));
    for (output, out_geo) in outputs {
        if out_geo.contains(center) {
            return Some(output.clone());
        }
    }

    // 3. Monitor que contiene la esquina superior izquierda (cliente o barra SSD)
    let ssd_top_left = Point::from((win_loc.x, win_loc.y - ssd_top));
    for (output, out_geo) in outputs {
        if out_geo.contains(win_loc) || (ssd_top > 0 && out_geo.contains(ssd_top_left)) {
            return Some(output.clone());
        }
    }

    // 4. Monitor de la última posición conocida del puntero (sin bloquear `PointerHandle`)
    if let Some(cur_loc) = last_pointer_location {
        for (output, out_geo) in outputs {
            if out_geo.to_f64().contains(cur_loc) {
                return Some(output.clone());
            }
        }
    }

    None
}

impl CompositorHandler for MalusState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn new_surface(&mut self, surface: &WlSurface) {
        // Este hook se registra al crearse la `wl_surface`, antes que el `pre_commit_hook` de
        // `WlrLayerShellState` en Smithay 0.7.0. Si el cliente ya destruyó su
        // `zwlr_layer_surface_v1` (como hace GTK4 4.24 + gtk4-layer-shell al cerrar ventanas
        // mediante `destroy` seguido de `attach(null) + commit`), Smithay reinicia el estado
        // pendiente a tamaño `(0, 0)` sin anclajes y dispara un falso `Error::InvalidSize` que
        // mata la conexión Wayland del cliente (p. ej. `malus-de-notifications`).
        smithay::wayland::compositor::add_pre_commit_hook::<Self, _>(
            surface,
            |_state, _dh, surface| {
                smithay::wayland::compositor::with_states(surface, |states| {
                    let role_destroyed = states
                        .data_map
                        .get::<LayerRoleDestroyedMarker>()
                        .is_some_and(|marker| marker.0.load(Ordering::Acquire));
                    if role_destroyed {
                        let mut cached = states
                            .cached_state
                            .get::<smithay::wayland::shell::wlr_layer::LayerSurfaceCachedState>();
                        let pending = cached.pending();
                        if pending.size.w == 0 {
                            pending.size.w = 1;
                        }
                        if pending.size.h == 0 {
                            pending.size.h = 1;
                        }
                    }
                });
            },
        );
    }

    fn commit(&mut self, surface: &WlSurface) {
        let mut matched_outputs: Vec<String> = Vec::new();
        let mut is_background = false;
        for output in self.space.outputs() {
            let layer_map = smithay::desktop::layer_map_for_output(output);
            if let Some(layer) = layer_map.layers().find(|l| l.wl_surface() == surface) {
                matched_outputs.push(output.name());
                if matches!(
                    layer.layer(),
                    smithay::wayland::shell::wlr_layer::Layer::Background
                        | smithay::wayland::shell::wlr_layer::Layer::Bottom
                ) {
                    is_background = true;
                }
            };
        }
        for (out_name, ls) in &self.ext_lock_surfaces {
            if ls.wl_surface() == surface {
                matched_outputs.push(out_name.clone());
            }
        }
        if is_background {
            self.background_commit_counter = self.background_commit_counter.wrapping_add(1);
        }

        let (is_buffer_removed, has_new_buffer) =
            smithay::wayland::compositor::with_states(surface, |states| {
                let mut guard = states
                    .cached_state
                    .get::<smithay::wayland::compositor::SurfaceAttributes>();
                let current = guard.current();
                (
                    matches!(
                        current.buffer,
                        Some(smithay::wayland::compositor::BufferAssignment::Removed)
                    ),
                    matches!(
                        current.buffer,
                        Some(smithay::wayland::compositor::BufferAssignment::NewBuffer(_))
                    ),
                )
            });
        if is_buffer_removed {
            if let Some(ctx_id) = self.renderer_context_id.clone() {
                if let Some(win) = self.window_for_surface(surface) {
                    if is_root_window_surface(&win, surface)
                        && self.space.element_location(&win).is_some()
                    {
                        if let Some(root_surf) = win.wl_surface() {
                            if !self.cached_close_snapshots.contains_key(root_surf.as_ref()) {
                                let layers = crate::black_hole::capture_surface_tree_snapshot(
                                    root_surf.as_ref(),
                                    ctx_id,
                                );
                                if !layers.is_empty() {
                                    self.cached_close_snapshots
                                        .insert(root_surf.into_owned(), (layers, Instant::now()));
                                }
                            }
                        }
                    }
                }
            }
        }

        smithay::backend::renderer::utils::on_commit_buffer_handler::<Self>(surface);
        self.popups.commit(surface);

        if self.dnd_icon.as_ref() == Some(surface) {
            self.dnd_icon_cache_dirty = true;
            let delta = smithay::wayland::compositor::with_states(surface, |states| {
                states
                    .cached_state
                    .get::<smithay::wayland::compositor::SurfaceAttributes>()
                    .current()
                    .buffer_delta
                    .take()
            });
            if let Some(delta) = delta {
                self.dnd_icon_offset += delta;
            }
        }
        if has_new_buffer
            && self
                .dnd_drop_hold
                .as_ref()
                .is_some_and(|(target, _, started)| {
                    target == surface && started.elapsed() >= std::time::Duration::from_millis(4)
                })
        {
            self.clear_dnd_icon();
        }

        let window = self.window_for_surface(surface);
        if matched_outputs.is_empty() {
            if let Some(ref win) = window {
                for output in self.space.outputs_for_element(win) {
                    matched_outputs.push(output.name());
                }
            }
        }
        if !matched_outputs.is_empty() && !self.expose.active {
            for out_name in matched_outputs {
                self.bump_output_commit_counter(&out_name);
            }
        } else {
            self.surface_commit_counter = self.surface_commit_counter.wrapping_add(1);
        }

        // Sincronizar escala fraccionaria preferida según el monitor físico en el que reside la superficie
        let preferred_scale =
            self.preferred_scale_for_surface_with_window(surface, window.as_ref());
        smithay::wayland::compositor::with_states(surface, |states| {
            smithay::wayland::fractional_scale::with_fractional_scale(states, |fractional_scale| {
                fractional_scale.set_preferred_scale(preferred_scale);
            });
        });

        // Si es una ventana regular en Space, actualizamos sus dimensiones internas únicamente para esa ventana.
        // Solo el commit de la superficie raíz del toplevel debe evaluar colocación inicial, transients o topbar;
        // los commits de popups (menús contextuales) o subsurfaces pertenecen a `window_for_surface` vía `with_surfaces`,
        // pero nunca deben recentrar la ventana padre.
        if let Some(window) = window {
            window.on_commit();
            if is_root_window_surface(&window, surface) {
                if let Some(toplevel) = window.toplevel() {
                    let is_system_action_confirmation = toplevel.parent().is_some()
                        || is_system_action_confirmation_surface(surface);
                    let geometry = window.geometry();
                    if is_system_action_confirmation
                        && self.centered_transient_windows.get(surface)
                            != Some(&(geometry.loc, geometry.size))
                    {
                        if geometry.size.w > 1 && geometry.size.h > 1 {
                            let parent_win = toplevel.parent().and_then(|parent| {
                                let parent_surface = parent.wl_surface();
                                parent_surface
                                    .as_deref()
                                    .and_then(|surface| self.window_for_surface(surface))
                            });
                            let parent_rect = parent_win.as_ref().and_then(|parent| {
                                let location = self.space.element_location(parent)?;
                                let parent_geometry = parent.geometry();
                                Some(Rectangle::new(
                                    Point::<i32, Logical>::from((
                                        location.x + parent_geometry.loc.x,
                                        location.y + parent_geometry.loc.y,
                                    )),
                                    parent_geometry.size,
                                ))
                            });
                            let parent_output = parent_rect.and_then(|pr| {
                                let center = Point::<i32, Logical>::from((
                                    pr.loc.x + pr.size.w / 2,
                                    pr.loc.y + pr.size.h / 2,
                                ));
                                self.space.outputs().find(|output| {
                                    self.space
                                        .output_geometry(output)
                                        .map(|output_geometry| output_geometry.contains(center))
                                        .unwrap_or(false)
                                })
                            });
                            let window_location =
                                self.space.element_location(&window).unwrap_or_default();
                            let current_center = Point::<i32, Logical>::from((
                                window_location.x + geometry.loc.x + geometry.size.w / 2,
                                window_location.y + geometry.loc.y + geometry.size.h / 2,
                            ));
                            let output = parent_output
                                .or_else(|| {
                                    self.space.outputs().find(|output| {
                                        self.space
                                            .output_geometry(output)
                                            .map(|output_geometry| {
                                                output_geometry.contains(current_center)
                                            })
                                            .unwrap_or(false)
                                    })
                                })
                                .cloned();

                            if let Some(output) = output {
                                if let Some(output_geometry) = self.space.output_geometry(&output) {
                                    let usable = self.usable_zone_for_output(&output);
                                    let mut centered_location = if let Some(pr) = parent_rect {
                                        Point::<i32, Logical>::from((
                                            pr.loc.x + (pr.size.w - geometry.size.w) / 2
                                                - geometry.loc.x,
                                            pr.loc.y + (pr.size.h - geometry.size.h) / 2
                                                - geometry.loc.y,
                                        ))
                                    } else {
                                        Point::<i32, Logical>::from((
                                            output_geometry.loc.x
                                                + (output_geometry.size.w - geometry.size.w) / 2
                                                - geometry.loc.x,
                                            output_geometry.loc.y
                                                + (output_geometry.size.h - geometry.size.h) / 2
                                                - geometry.loc.y,
                                        ))
                                    };
                                    let min_x = usable.loc.x - geometry.loc.x;
                                    let max_x = (usable.loc.x + usable.size.w - geometry.size.w
                                        - geometry.loc.x)
                                        .max(min_x);
                                    let min_y = usable.loc.y - geometry.loc.y;
                                    let max_y = (usable.loc.y + usable.size.h - geometry.size.h
                                        - geometry.loc.y)
                                        .max(min_y);
                                    centered_location.x = centered_location.x.clamp(min_x, max_x);
                                    centered_location.y = centered_location.y.clamp(min_y, max_y);

                                    self.space
                                        .map_element(window.clone(), centered_location, true);
                                    self.space.raise_element(&window, true);
                                    self.centered_transient_windows
                                        .insert(surface.clone(), (geometry.loc, geometry.size));
                                }
                            }
                        }
                    } else if !is_system_action_confirmation
                        && !self.initially_placed_windows.contains(surface)
                        && geometry.size.w > 1
                        && geometry.size.h > 1
                    {
                        let states = &toplevel.current_state().states;
                        let is_snapped_state = states.contains(XdgState::Maximized)
                            || states.contains(XdgState::Fullscreen)
                            || states.contains(XdgState::TiledLeft)
                            || states.contains(XdgState::TiledRight)
                            || states.contains(XdgState::TiledTop)
                            || states.contains(XdgState::TiledBottom);
                        if self.auto_tiling_enabled && is_window_tileable(self, &window) {
                            self.initially_placed_windows.insert(surface.clone());
                            self.place_new_window_with_auto_tiling(&window);
                        } else if is_window_snapped(&window)
                            || is_snapped_state
                            || self.snap_transitions.contains_key(surface)
                            || self.minimized_windows.contains_key(surface)
                        {
                            self.initially_placed_windows.insert(surface.clone());
                        } else {
                            let saved_pos = if toplevel.parent().is_none() {
                                app_id_for_surface(surface)
                                    .and_then(|id| self.get_saved_window_position(&id))
                            } else {
                                None
                            };
                            let output = saved_pos
                                .and_then(|pt| {
                                    self.space
                                        .outputs()
                                        .find(|o| {
                                            self.space
                                                .output_geometry(o)
                                                .is_some_and(|g| g.contains(pt))
                                        })
                                        .cloned()
                                })
                                .or_else(|| self.origin_output_for_window(&window))
                                .or_else(|| self.primary_output());
                            if let Some(output) = output {
                                let usable = self.usable_zone_for_output(&output);
                                let ssd_top = if window_uses_ssd(&window) {
                                    crate::ssd::SSD_TITLEBAR_HEIGHT
                                } else {
                                    0
                                };
                                let max_bounds = max_window_size_in_usable_zone(usable, ssd_top);
                                if (geometry.size.w > max_bounds.w || geometry.size.h > max_bounds.h)
                                    && !self.clamped_oversized_windows.contains(surface)
                                {
                                    let clamped = clamp_size_to_bounds(geometry.size, max_bounds);
                                    tracing::info!(
                                        "📐 [commit] Ventana excede el área útil ({:?} > {:?}); acotando a {:?}",
                                        geometry.size,
                                        max_bounds,
                                        clamped
                                    );
                                    self.clamped_oversized_windows.insert(surface.clone());
                                    toplevel.with_pending_state(|s| {
                                        s.bounds = Some(max_bounds);
                                        s.size = Some(clamped);
                                    });
                                    toplevel.send_configure();
                                    let provisional_loc = clamped_or_centered_window_location(
                                        usable,
                                        geometry,
                                        ssd_top,
                                        saved_pos,
                                    );
                                    self.space
                                        .map_element(window.clone(), provisional_loc, true);
                                } else {
                                    let target_loc = clamped_or_centered_window_location(
                                        usable,
                                        geometry,
                                        ssd_top,
                                        saved_pos,
                                    );
                                    self.space.map_element(window.clone(), target_loc, true);
                                    self.initially_placed_windows.insert(surface.clone());
                                }
                            }
                        }
                    }
                }

                let is_currently_snapped = is_window_snapped(&window);

                if !is_currently_snapped {
                    self.ensure_window_below_topbar(&window);
                }

                if !is_currently_snapped && !self.snap_transitions.contains_key(surface) {
                    let geo = window.geometry();
                    let loc = self.space.element_location(&window).unwrap_or_default();
                    if geo.size.w > 150 && geo.size.h > 150 {
                        self.window_restore_geometries
                            .insert(surface.clone(), (loc, geo.size));
                    }
                }
            }
        }

        // Si la superficie pertenece a una Layer Surface (Top Bar, Dock, Notificaciones, etc.)
        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        let target_output = outputs.into_iter().find(|o| {
            let map = smithay::desktop::layer_map_for_output(o);
            map.layer_for_surface(surface, smithay::desktop::WindowSurfaceType::all())
                .is_some()
        });

        if let Some(output) = target_output {
            let mut should_focus_overlay = None;
            {
                let mut map = smithay::desktop::layer_map_for_output(&output);
                map.arrange();

                if let Some(layer) =
                    map.layer_for_surface(surface, smithay::desktop::WindowSurfaceType::TOPLEVEL)
                {
                    let initial_configure_sent =
                        smithay::wayland::compositor::with_states(layer.wl_surface(), |states| {
                            states
                                .data_map
                                .get::<smithay::wayland::shell::wlr_layer::LayerSurfaceData>()
                                .map(|d| d.lock().unwrap().initial_configure_sent)
                                .unwrap_or(false)
                        });
                    if !initial_configure_sent {
                        tracing::info!("📤 Enviando initial configure a layer surface");
                        layer.layer_surface().send_configure();
                    } else {
                        layer.layer_surface().send_pending_configure();
                    }
                    if layer.layer() == smithay::wayland::shell::wlr_layer::Layer::Overlay
                        && layer.can_receive_keyboard_focus()
                    {
                        should_focus_overlay = Some(layer.wl_surface().clone());
                    }
                }
            }

            if let Some(focus_surf) = should_focus_overlay {
                if let Some(kb) = self.seat.get_keyboard() {
                    if kb.current_focus().as_ref() != Some(&focus_surf) {
                        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                        kb.set_focus(self, Some(focus_surf), serial);
                    }
                }
            }
        }
    }
}

impl BufferHandler for MalusState {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for MalusState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl DmabufHandler for MalusState {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        self.pending_dmabuf_imports.push((dmabuf, notifier));
    }
}

impl MalusState {
    /// Importa en el renderer los DMA-BUFs recién solicitados por clientes Wayland y notifica el resultado.
    pub fn process_pending_dmabuf_imports(
        &mut self,
        renderer: &mut impl smithay::backend::renderer::ImportDma,
    ) -> bool {
        if self.pending_dmabuf_imports.is_empty() {
            return false;
        }
        for (dmabuf, notifier) in self.pending_dmabuf_imports.drain(..) {
            match renderer.import_dmabuf(&dmabuf, None) {
                Ok(_) => {
                    let _ = notifier.successful::<Self>();
                }
                Err(err) => {
                    tracing::warn!("⚠️ Falló importación DMA-BUF en renderer GPU: {:?}", err);
                    notifier.failed();
                }
            }
        }
        true
    }
}

impl SeatHandler for MalusState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| {
            dh.get_client(smithay::reexports::wayland_server::Resource::id(s))
                .ok()
        });
        let is_regular_window = focused.and_then(|s| self.window_for_surface(s)).is_some();
        if focused.is_none() || is_regular_window {
            let focused_pid = client.as_ref().and_then(|client| {
                let credentials = client.get_credentials(dh).ok()?;
                (credentials.uid == rustix::process::getuid().as_raw()).then_some(credentials.pid)
            });
            self.focus_cpu_priority.set_focused_pid(focused_pid);
        }
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);

        if focused.is_none() {
            self.cursor_status = smithay::input::pointer::CursorImageStatus::default_named();
        }

        // Actualizar el estado Activated de las ventanas cuyo foco haya cambiado realmente
        self.sync_window_activation(focused);
    }
    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        image: smithay::input::pointer::CursorImageStatus,
    ) {
        // IMPORTANTE: Nunca invocar métodos de `PointerHandle` (`is_grabbed`, `current_location`, etc.)
        // aquí porque Smithay llama a `SeatHandler::cursor_image` desde `PointerInternal::motion` /
        // `PointerTarget::replace` mientras el `Mutex<PointerInternal>` ya está bloqueado en el mismo hilo.
        if self.ssd.grab_active || self.ssd.cursor_override {
            return;
        }
        self.cursor_status = image;
    }
}

impl SelectionHandler for MalusState {
    type SelectionUserData = ();
}

impl DataDeviceHandler for MalusState {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl PrimarySelectionHandler for MalusState {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

impl DataControlHandler for MalusState {
    fn data_control_state(&self) -> &DataControlState {
        &self.data_control_state
    }
}

impl ClientDndGrabHandler for MalusState {
    fn started(
        &mut self,
        _source: Option<smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource>,
        icon: Option<WlSurface>,
        _seat: Seat<Self>,
    ) {
        tracing::info!("✨ [DnD] Drag iniciado con icono: {}", icon.is_some());
        self.dnd_drop_hold = None;
        self.dnd_icon = icon.clone();
        self.dnd_icon_offset = Point::from((0, 0));
        self.dnd_icon_cache = None;
        self.dnd_icon_cache_dirty = icon.is_some();
        if let Some(ref surf) = icon {
            let delta = smithay::wayland::compositor::with_states(surf, |states| {
                states
                    .cached_state
                    .get::<smithay::wayland::compositor::SurfaceAttributes>()
                    .current()
                    .buffer_delta
                    .take()
            });
            if let Some(delta) = delta {
                self.dnd_icon_offset += delta;
            }
        }
    }

    fn dropped(&mut self, target: Option<WlSurface>, validated: bool, _seat: Seat<Self>) {
        tracing::info!("✨ [DnD] Drag finalizado");
        if validated {
            if let (Some(target_surf), true) = (target, self.dnd_icon.is_some()) {
                // Smithay invoca `dropped` desde PointerGrab::button con PointerInternal bloqueado;
                // consultar `last_pointer_location` evita deadlock re-entrante en `seat.get_pointer()`.
                let drop_loc = self.last_pointer_location.unwrap_or_default();
                self.dnd_drop_hold = Some((target_surf, drop_loc, Instant::now()));
                return;
            }
        }
        self.clear_dnd_icon();
    }
}

impl MalusState {
    pub fn clear_dnd_icon(&mut self) {
        self.dnd_icon = None;
        self.dnd_icon_offset = Point::from((0, 0));
        self.dnd_drop_hold = None;
        self.dnd_icon_cache = None;
        self.dnd_icon_cache_dirty = false;
    }

    pub fn active_dnd_icon_draw_location(&mut self) -> Option<Point<f64, Logical>> {
        if let Some((_, drop_loc, started)) = self.dnd_drop_hold {
            if started.elapsed() >= std::time::Duration::from_millis(200) {
                self.clear_dnd_icon();
                return None;
            }
            if self.dnd_icon.as_ref().is_some_and(|s| s.alive()) || self.dnd_icon_cache.is_some() {
                return Some(drop_loc);
            }
            self.clear_dnd_icon();
            return None;
        }
        let is_dnd_alive = self.dnd_icon.as_ref().map(|s| s.alive()).unwrap_or(false);
        if !is_dnd_alive {
            if self.dnd_icon.is_some() {
                self.clear_dnd_icon();
            }
            return None;
        }
        self.seat.get_pointer().map(|p| p.current_location())
    }
}

impl ServerDndGrabHandler for MalusState {
    fn send(&mut self, _mime_type: String, _fd: OwnedFd, _seat: Seat<Self>) {}
}

impl OutputHandler for MalusState {}

impl XdgShellHandler for MalusState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        tracing::info!("✨ Nueva ventana XDG Toplevel recibida y mapeada en Space!");
        let target_output = self
            .seat
            .get_pointer()
            .map(|p| p.current_location())
            .and_then(|loc| {
                self.space
                    .outputs()
                    .find(|o| {
                        self.space
                            .output_geometry(o)
                            .map(|g| g.to_f64().contains(loc))
                            .unwrap_or(false)
                    })
                    .cloned()
            })
            .or_else(|| self.primary_output());

        let usable_zone = if let Some(ref output) = target_output {
            output.enter(surface.wl_surface());
            let scale = output.current_scale().fractional_scale();
            smithay::wayland::compositor::with_states(surface.wl_surface(), |states| {
                with_fractional_scale(states, |fractional_scale| {
                    fractional_scale.set_preferred_scale(scale);
                });
            });
            self.usable_zone_for_output(output)
        } else {
            Rectangle::new((0, 36).into(), (1920, 1044).into())
        };

        let window = smithay::desktop::Window::new_wayland_window(surface.clone());
        window.set_activated(true);

        let max_bounds = max_window_size_in_usable_zone(usable_zone, 0);

        // Si tenemos un tamaño y/o posición recordados para esta aplicación, aplicarlos acotados al área útil;
        // si es la primera vez que se abre, se centra en pantalla y elige su tamaño natural sin exceder `bounds`.
        let (initial_size, saved_pos) = if surface.parent().is_none() {
            let app_id = app_id_for_surface(surface.wl_surface());
            let size = app_id
                .as_deref()
                .and_then(|id| self.get_saved_window_size(id))
                .map(|saved| clamp_size_to_bounds(saved, max_bounds));
            let pos = app_id
                .as_deref()
                .and_then(|id| self.get_saved_window_position(id));
            (size, pos)
        } else {
            (None, None)
        };

        let spawn_usable = saved_pos
            .and_then(|pt| {
                self.space.outputs().find_map(|o| {
                    let g = self.space.output_geometry(o)?;
                    g.contains(pt).then(|| self.usable_zone_for_output(o))
                })
            })
            .unwrap_or(usable_zone);

        let spawn_loc = clamped_or_centered_window_location(
            spawn_usable,
            Rectangle::from_size(initial_size.unwrap_or_else(|| Size::from((400, 300)))),
            0,
            saved_pos,
        );

        surface.with_pending_state(|state| {
            state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated);
            use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
            // Preferir decoraciones del lado del cliente (CSD) nativas estilo Gnome por defecto.
            // Solo las ventanas que soliciten explícitamente ServerSide recibirán SSD.
            state.decoration_mode = Some(Mode::ClientSide);
            state.bounds = Some(max_bounds);
            if let Some(size) = initial_size {
                tracing::info!("📐 [State] Aplicando tamaño de ventana recordado a nueva ventana: {:?}", size);
                state.size = Some(size);
            }
        });
        surface.send_configure();
        self.space.map_element(window, spawn_loc, true);
        let prev_layer_focus = self
            .seat
            .get_keyboard()
            .and_then(|kb| kb.current_focus())
            .filter(|prev| self.is_desktop_focusable_layer_surface(prev));
        if let Some(layer_surf) = prev_layer_focus {
            self.toplevel_return_layer_focus
                .insert(surface.wl_surface().clone(), layer_surf);
        }
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        if let Some(kb) = self.seat.get_keyboard() {
            kb.set_focus(self, Some(surface.wl_surface().clone()), serial);
        }
        self.notify_running_apps();
        self.notify_active_app(Some(surface.wl_surface()));
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        tracing::info!("✨ [XdgShell] Nueva PopupSurface recibida; registrando en PopupManager");
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        if self.popups.find_popup(surface.wl_surface()).is_none() {
            if let Err(err) = self.popups.track_popup(PopupKind::Xdg(surface.clone())) {
                tracing::warn!("⚠️ Error al registrar popup en PopupManager: {:?}", err);
            }
        }
        if surface.get_parent_surface().is_some() {
            if let Err(err) = surface.send_configure() {
                tracing::warn!("⚠️ Error al enviar configure a popup: {:?}", err);
            }
        }
        if !self
            .active_popups
            .iter()
            .any(|p| p.wl_surface() == surface.wl_surface())
        {
            self.active_popups.push(surface);
        }
    }

    fn grab(&mut self, surface: PopupSurface, seat: WlSeat, serial: Serial) {
        tracing::info!(
            "🎯 [XdgShell] grab request recibido para popup surface: {:?}",
            smithay::reexports::wayland_server::Resource::id(surface.wl_surface())
        );
        if !self
            .active_popups
            .iter()
            .any(|p| p.wl_surface() == surface.wl_surface())
        {
            self.active_popups.push(surface.clone());
        }

        let kind = PopupKind::Xdg(surface.clone());
        let root = match find_popup_root_surface(&kind) {
            Ok(root) => root,
            Err(err) => {
                tracing::warn!("⚠️ No se encontró root surface para popup grab: {:?}", err);
                return;
            }
        };

        let Some(seat) = Seat::<Self>::from_resource(&seat) else {
            tracing::warn!("⚠️ No se pudo resolver Seat desde WlSeat para popup grab");
            return;
        };

        let parent_surface = surface.get_parent_surface();
        if let Some(mut existing_grab) = self.popup_grab.clone() {
            if parent_surface.as_ref() == Some(&root) {
                let _ = existing_grab.ungrab(smithay::desktop::PopupUngrabStrategy::All);
                self.popup_grab = None;
            } else {
                while existing_grab.current_grab().as_ref() != parent_surface.as_ref() {
                    let before = existing_grab.current_grab();
                    let _ = existing_grab.ungrab(smithay::desktop::PopupUngrabStrategy::Topmost);
                    if existing_grab.current_grab() == before {
                        let _ = existing_grab.ungrab(smithay::desktop::PopupUngrabStrategy::All);
                        self.popup_grab = None;
                        break;
                    }
                }
            }
        }
        self.popups.cleanup();

        match self.popups.grab_popup::<Self>(root, kind, &seat, serial) {
            Ok(popup_grab) => {
                if let Some(keyboard) = seat.get_keyboard() {
                    keyboard.set_grab(self, PopupKeyboardGrab::new(&popup_grab), serial);
                }
                if let Some(pointer) = seat.get_pointer() {
                    pointer.set_grab(
                        self,
                        PopupPointerGrab::new(&popup_grab),
                        serial,
                        smithay::input::pointer::Focus::Keep,
                    );
                }
                self.popup_grab = Some(popup_grab);
                tracing::info!(
                    "✅ [XdgShell] Popup grab (teclado + puntero) establecido exitosamente"
                );
            }
            Err(err) => {
                tracing::warn!(
                    "⚠️ Error al registrar popup grab en PopupManager: {:?}",
                    err
                );
            }
        }
    }

    fn popup_destroyed(&mut self, surface: PopupSurface) {
        tracing::info!("🗑️ [XdgShell] Popup surface destruida");
        let surf = surface.wl_surface();
        self.active_popups.retain(|p| p.wl_surface() != surf);
        if let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(surface.clone())) {
            let _ = PopupManager::dismiss_popup(&root, &PopupKind::Xdg(surface));
        }
        self.popups.cleanup();
        self.refresh_popup_grab();
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: WlSeat, serial: Serial) {
        tracing::info!(
            "🖱️ [XdgShell] move_request recibido del cliente: serial={:?}",
            serial
        );
        let Some(seat) = Seat::<Self>::from_resource(&seat) else {
            return;
        };
        let Some(pointer) = seat.get_pointer() else {
            return;
        };

        if !pointer.has_grab(serial) {
            tracing::warn!(
                "⚠️ [XdgShell] move_request rechazado: el puntero no tiene un grab con este serial"
            );
            return;
        }

        let Some(start_data) = pointer.grab_start_data() else {
            tracing::warn!("⚠️ [XdgShell] move_request ignorado: no hay clic activo de ratón (evitando bloqueo del puntero)");
            return;
        };

        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            tracing::warn!("⚠️ [XdgShell] move_request: no se encontró ventana para la superficie");
            return;
        };

        let is_snapped = is_window_snapped(&window);

        let initial_window_location = self.space.element_location(&window).unwrap_or_default();

        let grab = crate::grabs::MoveSurfaceGrab {
            start_data,
            window: window.clone(),
            initial_window_location,
            is_snapped,
            has_dragged: false,
        };

        self.space.raise_element(&window, true);
        self.flush_pending_activation_configures();
        self.ssd.grab_active = true;
        self.moving_window_surface = window.wl_surface().map(|s| s.into_owned());
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
        tracing::info!(
            "✅ [XdgShell] MoveSurfaceGrab activado exitosamente para arrastre por título!"
        );
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: WlSeat,
        serial: Serial,
        edges: ResizeEdge,
    ) {
        tracing::info!(
            "📐 [XdgShell] resize_request recibido del cliente: edges={:?}",
            edges
        );
        let Some(seat) = Seat::from_resource(&seat) else {
            return;
        };
        let Some(pointer) = seat.get_pointer() else {
            return;
        };

        if !pointer.has_grab(serial) {
            tracing::warn!(
                "⚠️ resize_request rechazado: el puntero no tiene un grab con este serial"
            );
            return;
        }

        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            return;
        };

        let is_maximized_or_snapped = is_window_snapped(&window);

        if is_maximized_or_snapped {
            tracing::info!("⚠️ [XdgShell] resize_request ignorado: la ventana está maximizada, a pantalla completa o anclada");
            return;
        }

        let start_data = pointer.grab_start_data().unwrap();

        let initial_window_location = self.space.element_location(&window).unwrap_or_default();
        let initial_window_size = window.geometry().size;

        let grab = crate::grabs::ResizeSurfaceGrab {
            start_data,
            window: window.clone(),
            edges,
            initial_window_location,
            initial_window_size,
            last_window_size: initial_window_size,
        };

        self.space.raise_element(&window, true);
        self.flush_pending_activation_configures();
        self.ssd.grab_active = true;
        self.ssd.cursor_override = true;
        self.cursor_status = smithay::input::pointer::CursorImageStatus::Named(
            crate::grabs::cursor_icon_for_resize_edge(edges),
        );
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
        tracing::info!(
            "✅ [XdgShell] ResizeSurfaceGrab activado exitosamente para redimensión de ventana!"
        );
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        let _ = surface.send_repositioned(token);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        tracing::info!("🪟 [XdgShell] maximize_request recibido del cliente");
        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            return;
        };
        let output = self
            .origin_output_for_window(&window)
            .or_else(|| self.primary_output());

        if let Some(output) = output {
            let usable_zone = self.usable_zone_for_output(&output);
            snap_window(self, &window, SnapMode::Maximize, usable_zone);
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        tracing::info!("🪟 [XdgShell] unmaximize_request recibido del cliente");
        let surf = surface.wl_surface();
        let Some(window) = self.window_for_surface(surf) else {
            return;
        };
        let output = self
            .origin_output_for_window(&window)
            .or_else(|| self.primary_output());

        if let Some(output) = output {
            let usable_zone = self.usable_zone_for_output(&output);
            snap_window(self, &window, SnapMode::CenterRestore, usable_zone);
        }
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, requested_output: Option<WlOutput>) {
        tracing::info!("🖥️ [XdgShell] fullscreen_request recibido del cliente");
        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            tracing::warn!("⚠️ fullscreen_request ignorado: no se encontró la ventana");
            return;
        };
        // Siempre respetar la pantalla de origen donde reside la ventana para que los vídeos
        // o aplicaciones en pantalla completa no salten arbitrariamente al monitor primario.
        let output = self
            .origin_output_for_window(&window)
            .or_else(|| requested_output.as_ref().and_then(Output::from_resource))
            .or_else(|| self.primary_output());
        let Some(output) = output else {
            tracing::warn!("⚠️ fullscreen_request ignorado: no hay una pantalla disponible");
            return;
        };
        let Some(output_geometry) = self.space.output_geometry(&output) else {
            tracing::warn!(
                "⚠️ fullscreen_request ignorado: no se pudo obtener la geometría de pantalla"
            );
            return;
        };

        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let states = &toplevel.current_state().states;
        if !states.contains(XdgState::Fullscreen) {
            if let Some(surface) = window.wl_surface() {
                let geometry = window.geometry();
                let loc = self.space.element_location(&window).unwrap_or_default();
                self.window_restore_geometries
                    .insert(surface.into_owned(), (loc, geometry.size));
            }
        }

        toplevel.with_pending_state(|state| {
            state.size = Some(output_geometry.size);
            unset_snap_states(state);
            state.states.set(XdgState::Fullscreen);
        });
        self.space
            .map_element(window.clone(), output_geometry.loc, true);
        self.space.raise_element(&window, true);
        toplevel.send_configure();
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        tracing::info!("🖥️ [XdgShell] unfullscreen_request recibido del cliente");
        let surf = surface.wl_surface();
        let Some(window) = self.window_for_surface(surf) else {
            tracing::warn!("⚠️ unfullscreen_request ignorado: no se encontró la ventana");
            return;
        };
        let Some(toplevel) = window.toplevel() else {
            return;
        };
        let output = self
            .origin_output_for_window(&window)
            .or_else(|| self.primary_output());
        let restore = self.window_restore_geometries.get(surf).copied();
        let (location, size) = if let Some((saved_loc, saved_size)) = restore {
            // Asegurar que la posición restaurada permanezca dentro del monitor actual
            // donde el usuario estaba visualizando la ventana a pantalla completa.
            let output_geo = output.as_ref().and_then(|o| self.space.output_geometry(o));
            let loc = if let Some(geo) = output_geo {
                if geo.contains(saved_loc) {
                    saved_loc
                } else {
                    Point::from((geo.loc.x + 80, geo.loc.y + 60))
                }
            } else {
                saved_loc
            };
            (loc, Some(saved_size))
        } else if let Some(ref output) = output {
            let output_geometry = self.space.output_geometry(output).unwrap_or_default();
            (
                Point::from((output_geometry.loc.x + 80, output_geometry.loc.y + 60)),
                None,
            )
        } else {
            (
                self.space.element_location(&window).unwrap_or_default(),
                None,
            )
        };

        toplevel.with_pending_state(|state| {
            state.size = size;
            state.states.unset(XdgState::Fullscreen);
        });
        self.space.map_element(window.clone(), location, true);
        self.space.raise_element(&window, true);
        toplevel.send_configure();
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        self.minimize_toplevel(&surface);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        tracing::info!("🗑️ [XdgShell] Ventana toplevel destruida");
        self.snap_layouts.close();
        let surf = surface.wl_surface();
        self.centered_transient_windows.remove(surf);
        self.initially_placed_windows.remove(surf);
        self.clamped_oversized_windows.remove(surf);

        let window = self
            .space
            .elements()
            .find(|w| w.toplevel() == Some(&surface) || w.wl_surface().as_deref() == Some(surf))
            .cloned();

        // Guardar el tamaño final no maximizado/no tiled de la aplicación
        if surface.parent().is_none() {
            if let Some(id) = app_id_for_surface(surf) {
                let states = &surface.current_state().states;
                let is_snapped = states.contains(XdgState::Maximized)
                    || states.contains(XdgState::TiledLeft)
                    || states.contains(XdgState::TiledRight)
                    || states.contains(XdgState::TiledTop)
                    || states.contains(XdgState::TiledBottom)
                    || states.contains(XdgState::Fullscreen);

                if !is_snapped {
                    if let Some(ref win) = window {
                        let geo = win.geometry();
                        if geo.size.w >= 200 && geo.size.h >= 200 {
                            let vis_loc = self.space.element_location(win).map(|loc| loc + geo.loc);
                            tracing::info!(
                                "💾 Recordando geometría de ventana para {}: size={:?}, pos={:?}",
                                id,
                                geo.size,
                                vis_loc
                            );
                            self.save_app_window_geometry(&id, geo.size, vis_loc);
                        }
                    }
                } else if let Some((saved_loc, saved_size)) =
                    self.window_restore_geometries.get(surf).copied()
                {
                    if saved_size.w >= 200 && saved_size.h >= 200 {
                        let vis_loc = window
                            .as_ref()
                            .map(|w| saved_loc + w.geometry().loc)
                            .or(Some(saved_loc));
                        tracing::info!(
                            "💾 Recordando geometría previa no maximizada para {}: size={:?}, pos={:?}",
                            id,
                            saved_size,
                            vis_loc
                        );
                        self.save_app_window_geometry(&id, saved_size, vis_loc);
                    }
                }
            }
        }

        let hint_entry = self.close_button_hints.remove(surf);
        let skip_black_hole = !self.desktop_dock_cfg.black_hole_close_effect
            || should_skip_black_hole_for_app_id(app_id_for_surface(surf).as_deref());
        if let Some(ref win) = window {
            let geo = win.geometry();
            if !skip_black_hole
                && !self.expose.active
                && !self.minimized_windows.contains_key(surf)
                && geo.size.w >= 80
                && geo.size.h >= 48
            {
                if let Some(loc) = self.space.element_location(win) {
                    let uses_ssd = window_uses_ssd(win);
                    let states = &surface.current_state().states;
                    let is_fullscreen = states.contains(XdgState::Fullscreen);
                    let is_active = states.contains(XdgState::Activated)
                        || self
                            .seat
                            .get_keyboard()
                            .and_then(|k| k.current_focus())
                            .as_ref()
                            == Some(surf);
                    let fallback_origin = self.close_button_center_logical(
                        win,
                        loc,
                        uses_ssd,
                        is_fullscreen,
                        self.ssd.buttons_right,
                    );

                    let (origin_logical, origin_source) =
                        if let Some((hint_pt, hint_at, from_click)) = hint_entry {
                            if hint_at.elapsed() < std::time::Duration::from_secs(5) {
                                if from_click && !uses_ssd {
                                    let rel_x = hint_pt.x - loc.x as f64;
                                    let rel_y = hint_pt.y - loc.y as f64;
                                    if let Some(is_right) =
                                        Self::is_in_csd_close_zone(rel_x, rel_y, geo.size.w as f64)
                                    {
                                        if let Some(app_id) = app_id_for_surface(surf) {
                                            let key = app_id.trim().to_ascii_lowercase();
                                            if !key.is_empty() {
                                                let dx = if is_right {
                                                    (geo.size.w as f64 - rel_x).clamp(4.0, 72.0)
                                                } else {
                                                    rel_x.clamp(4.0, 72.0)
                                                };
                                                let dy = rel_y.clamp(4.0, 56.0);
                                                self.learned_close_button_offsets
                                                    .insert((key, is_right), (dx, dy));
                                            }
                                        }
                                    }
                                }
                                (hint_pt, if from_click { "click" } else { "hint" })
                            } else {
                                (fallback_origin, "fallback-expired")
                            }
                        } else {
                            (fallback_origin, "fallback")
                        };

                    let cached_layers = self
                        .cached_close_snapshots
                        .remove(surf)
                        .map(|(layers, _)| layers)
                        .or_else(|| {
                            self.renderer_context_id.clone().map(|ctx_id| {
                                crate::black_hole::capture_surface_tree_snapshot(surf, ctx_id)
                            })
                        })
                        .unwrap_or_default();

                    tracing::info!(
                        "🕳️ [BlackHole] Encolando cierre de '{}' ({}x{}, origin=({:.1},{:.1}) [{}], loc=({},{}), cached_layers={})",
                        window_title_for_ssd(win),
                        geo.size.w,
                        geo.size.h,
                        origin_logical.x,
                        origin_logical.y,
                        origin_source,
                        loc.x,
                        loc.y,
                        cached_layers.len()
                    );

                    self.pending_black_holes
                        .push(crate::black_hole::PendingBlackHoleClose {
                            surface: surf.clone(),
                            geometry: geo,
                            location: loc,
                            origin_logical,
                            uses_ssd,
                            is_fullscreen,
                            is_active,
                            is_firefox: is_firefox_window(win),
                            is_translucent: is_translucent_window(win),
                            custom_corner_radius: None,
                            blur_insets: (0, 0, 0, 0),
                            title: window_title_for_ssd(win),
                            cached_layers,
                        });
                }
            }
        }
        self.cached_close_snapshots.remove(surf);
        self.close_button_hints
            .retain(|_, (_, at, _)| at.elapsed() < std::time::Duration::from_secs(10));

        self.window_restore_geometries.remove(surf);
        self.snap_transitions.remove(surf);
        self.minimized_windows.remove(surf);
        self.minimized_window_workspaces.remove(surf);
        let removed_from_ws = self.workspaces.remove_surface(surf);
        if let Some(window_id) = self.window_ids.remove(surf) {
            self.dock_slots.remove(&window_id);
            self.remove_minimized_thumbnail(window_id);
        }
        self.active_animations.retain(|a| &a.surface != surf);
        self.notify_minimized_windows();

        if let Some(window) = window.or(removed_from_ws) {
            self.space.unmap_elem(&window);
            if self.expose.active {
                self.refresh_expose_layout();
            }
            tracing::info!("🧹 Ventana desmapeada de Space tras destruirse");
        } else {
            tracing::warn!("⚠️ No se encontró la ventana en Space al destruir toplevel");
        }

        let return_layer_focus = self
            .toplevel_return_layer_focus
            .remove(surf)
            .filter(|s| s.alive() && self.is_desktop_focusable_layer_surface(s));
        let destroyed_is_quicklook = app_id_for_surface(surf)
            .as_deref()
            .map(str::to_lowercase)
            .is_some_and(|lower| is_quicklook_identifier(&lower));

        self.space.refresh();
        if self.auto_tiling_enabled {
            self.relayout_auto_tiling();
        }

        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        let (_next_win, next_surface) = if destroyed_is_quicklook {
            if let Some(layer_surf) = return_layer_focus {
                (None, Some(layer_surf))
            } else {
                let next_win = self.space.elements().rev().next().cloned();
                let next_surface = next_win
                    .as_ref()
                    .and_then(|w| w.wl_surface().map(|s| s.into_owned()))
                    .or_else(|| self.desktop_icons_focus_surface());
                (next_win, next_surface)
            }
        } else {
            let next_win = self.space.elements().rev().next().cloned();
            let next_surface = next_win
                .as_ref()
                .and_then(|w| w.wl_surface().map(|s| s.into_owned()))
                .or(return_layer_focus);
            (next_win, next_surface)
        };
        self.set_keyboard_focus_with_sync(next_surface, serial);

        if let Some(pointer) = self.seat.get_pointer() {
            if pointer.is_grabbed() {
                pointer.unset_grab(self, serial, 0);
            }
            let loc = pointer.current_location();
            let new_target = self
                .space
                .element_under(loc)
                .and_then(|(win, win_loc)| {
                    let local_point = loc - win_loc.to_f64();
                    win.surface_under(local_point, smithay::desktop::WindowSurfaceType::all())
                        .map(|(s, sub_loc)| (s, (win_loc + sub_loc).to_f64()))
                })
                .or_else(|| {
                    self.space.outputs().find_map(|output| {
                        let output_geo = self.space.output_geometry(output)?;
                        if !output_geo.to_f64().contains(loc) {
                            return None;
                        }
                        let output_local = loc - output_geo.loc.to_f64();
                        let layer_map = smithay::desktop::layer_map_for_output(output);
                        let layer = layer_map
                            .layer_under(Layer::Bottom, output_local)
                            .or_else(|| layer_map.layer_under(Layer::Background, output_local))?;
                        let geo = layer_map.layer_geometry(layer)?;
                        let local = output_local - geo.loc.to_f64();
                        layer
                            .surface_under(local, smithay::desktop::WindowSurfaceType::TOPLEVEL)
                            .map(|(s, sub_loc)| (s, (output_geo.loc + geo.loc + sub_loc).to_f64()))
                    })
                });
            let motion_serial = smithay::utils::SERIAL_COUNTER.next_serial();
            pointer.motion(
                self,
                new_target,
                &smithay::input::pointer::MotionEvent {
                    location: loc,
                    serial: motion_serial,
                    time: 0,
                },
            );
            pointer.frame(self);
            tracing::info!("🧹 Foco de puntero resincronizado tras destruirse ventana");
        }

        self.notify_running_apps();
        let focused = self.seat.get_keyboard().and_then(|kb| kb.current_focus());
        self.notify_active_app(focused.as_ref());
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        if surface.parent().is_none() {
            if let Some(id) = app_id_for_surface(surface.wl_surface()) {
                if let Some(saved) = self.get_saved_window_size(&id) {
                    let win_opt = self.window_for_surface(surface.wl_surface());
                    let cur_geo = win_opt.as_ref().map(|w| w.geometry());
                    let is_unconfigured = cur_geo
                        .map(|g| g.size.w <= 200 || g.size.h <= 200)
                        .unwrap_or(true);
                    if is_unconfigured {
                        let output = win_opt
                            .as_ref()
                            .and_then(|w| self.origin_output_for_window(w))
                            .or_else(|| self.primary_output());
                        let usable = output
                            .as_ref()
                            .map(|o| self.usable_zone_for_output(o))
                            .unwrap_or_else(|| Rectangle::new((0, 36).into(), (1920, 1044).into()));
                        let ssd_top = win_opt
                            .as_ref()
                            .filter(|w| window_uses_ssd(w))
                            .map(|_| crate::ssd::SSD_TITLEBAR_HEIGHT)
                            .unwrap_or(0);
                        let max_bounds = max_window_size_in_usable_zone(usable, ssd_top);
                        let clamped = clamp_size_to_bounds(saved, max_bounds);
                        tracing::info!(
                            "📐 [app_id_changed] Aplicando tamaño recordado para {}: {:?}",
                            id,
                            clamped
                        );
                        self.initially_placed_windows.remove(surface.wl_surface());
                        surface.with_pending_state(|state| {
                            state.bounds = Some(max_bounds);
                            state.size = Some(clamped);
                        });
                        surface.send_configure();
                    }
                }
            }
        }
        self.notify_running_apps();
        let focused = self.seat.get_keyboard().and_then(|kb| kb.current_focus());
        if focused.as_ref() == Some(surface.wl_surface()) {
            self.notify_active_app(Some(surface.wl_surface()));
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        self.notify_running_apps();
        let focused = self.seat.get_keyboard().and_then(|kb| kb.current_focus());
        if focused.as_ref() == Some(surface.wl_surface()) {
            self.notify_active_app(Some(surface.wl_surface()));
        }
    }
}

impl WlrLayerShellHandler for MalusState {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: LayerSurface,
        output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        tracing::info!("✨ Nueva Layer Surface recibida: namespace={}", namespace);
        smithay::wayland::compositor::with_states(surface.wl_surface(), |states| {
            if let Some(marker) = states.data_map.get::<LayerRoleDestroyedMarker>() {
                marker.0.store(false, Ordering::Release);
            } else {
                states
                    .data_map
                    .insert_if_missing_threadsafe(|| LayerRoleDestroyedMarker(AtomicBool::new(false)));
            }
        });
        if namespace == "malus-lock" {
            let layer_client_pid = self
                .display_handle
                .get_client(smithay::reexports::wayland_server::Resource::id(
                    surface.wl_surface(),
                ))
                .ok()
                .and_then(|c| c.get_credentials(&self.display_handle).ok())
                .map(|cred| cred.pid);
            if let (Some(expected_pid), Some(got_pid)) =
                (self.ext_lock_client_pid, layer_client_pid)
            {
                if expected_pid != got_pid {
                    tracing::warn!(
                        "🚨 [SECURITY] Rechazada capa 'malus-lock' no autorizada del PID {} (esperado {})",
                        got_pid,
                        expected_pid
                    );
                    surface.send_close();
                    return;
                }
            }
            self.snap_layouts.close();
            self.dismiss_app_switcher_immediately();
            self.spotlight.close();
            if self.expose.active {
                self.expose.active = false;
            }
            self.session_locked_flag.store(true, Ordering::SeqCst);
        }
        if namespace == "malus-top-bar" {
            surface.with_pending_state(|state| {
                state.size = None;
            });
        }
        let output = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.primary_output());

        if let Some(output) = output {
            output.enter(surface.wl_surface());
            let scale = output.current_scale().fractional_scale();
            smithay::wayland::compositor::with_states(surface.wl_surface(), |states| {
                with_fractional_scale(states, |fractional_scale| {
                    fractional_scale.set_preferred_scale(scale);
                });
            });
            let is_malus_lock = namespace == "malus-lock";
            let is_top_bar = namespace == "malus-top-bar";
            let desktop_surface = smithay::desktop::LayerSurface::new(surface, namespace);
            if is_malus_lock {
                self.malus_lock_layers
                    .push((output.clone(), desktop_surface.clone()));
            }
            let mut map = layer_map_for_output(&output);
            if is_top_bar {
                // Asegurar que `malus-top-bar` se organice siempre antes que `malus-dock` en LayerMap
                // para que un Dock lateral no recorte ni desplace horizontalmente la Top Bar.
                let docked: Vec<_> = map
                    .layers()
                    .filter(|l| l.namespace() == "malus-dock")
                    .cloned()
                    .collect();
                for d in &docked {
                    map.unmap_layer(d);
                }
                let _ = map.map_layer(&desktop_surface);
                for d in &docked {
                    let _ = map.map_layer(d);
                }
            } else {
                let _ = map.map_layer(&desktop_surface);
            }
            map.arrange();
        }
    }

    fn new_popup(
        &mut self,
        _parent: LayerSurface,
        popup: smithay::wayland::shell::xdg::PopupSurface,
    ) {
        tracing::info!(
            "✨ [LayerShell] Nuevo Popup para Layer Surface recibido; registrando en PopupManager"
        );
        if self.popups.find_popup(popup.wl_surface()).is_none() {
            if let Err(err) = self.popups.track_popup(PopupKind::Xdg(popup.clone())) {
                tracing::warn!(
                    "⚠️ Error al registrar layer popup en PopupManager: {:?}",
                    err
                );
            }
        }
        if let Err(err) = popup.send_configure() {
            tracing::warn!("⚠️ Error al enviar configure a layer popup: {:?}", err);
        }
        if !self
            .active_popups
            .iter()
            .any(|p| p.wl_surface() == popup.wl_surface())
        {
            self.active_popups.push(popup);
        }
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        use smithay::utils::IsAlive;
        tracing::info!("🗑️ [LayerShell] Layer surface destruida");
        let destroyed_wl = surface.wl_surface().clone();
        smithay::wayland::compositor::with_states(&destroyed_wl, |states| {
            if let Some(marker) = states.data_map.get::<LayerRoleDestroyedMarker>() {
                marker.0.store(true, Ordering::Release);
            } else {
                states
                    .data_map
                    .insert_if_missing_threadsafe(|| LayerRoleDestroyedMarker(AtomicBool::new(true)));
            }
        });
        self.malus_lock_layers
            .retain(|(_, layer)| layer.layer_surface() != &surface && layer.alive());
        self.session_locked_flag
            .store(self.is_session_locked(), Ordering::SeqCst);
        let mut notif_black_hole_rect: Option<(Point<i32, Logical>, Size<i32, Logical>)> = None;
        for output in self.space.outputs() {
            let mut map = layer_map_for_output(output);
            let layer = map
                .layers()
                .find(|&layer| layer.layer_surface() == &surface)
                .cloned();
            if let Some(layer) = layer {
                if layer.namespace() == "malus-notifications" {
                    if let Some(geo) = map.layer_geometry(&layer) {
                        let out_geo = self
                            .space
                            .output_geometry(output)
                            .unwrap_or_else(|| Rectangle::from_size((1920, 1080).into()));
                        // Solo activar Agujero Negro cuando el globo sigue visible en pantalla
                        // (cierre manual con ✕), no tras deslizarse fuera del borde derecho.
                        let visible_in_output = geo.loc.x < out_geo.size.w - 24
                            && geo.loc.x + geo.size.w > 24
                            && geo.loc.y < out_geo.size.h
                            && geo.loc.y + geo.size.h > 0;
                        if visible_in_output && geo.size.w >= 40 && geo.size.h >= 30 {
                            notif_black_hole_rect = Some((out_geo.loc + geo.loc, geo.size));
                        }
                    }
                }
                map.unmap_layer(&layer);
                tracing::info!(
                    "🧹 Layer surface desmapeada de LayerMap: namespace={}",
                    layer.namespace()
                );
            }
            map.cleanup();
        }

        if let Some((global_loc, size)) = notif_black_hole_rect {
            if self.desktop_dock_cfg.black_hole_close_effect {
                let cached_layers = self
                    .cached_close_snapshots
                    .remove(&destroyed_wl)
                    .map(|(layers, _)| layers)
                    .or_else(|| {
                        self.renderer_context_id.clone().map(|ctx_id| {
                            crate::black_hole::capture_surface_tree_snapshot(&destroyed_wl, ctx_id)
                        })
                    })
                    .unwrap_or_default();
                // Centro del botón circular ✕ en la esquina superior izquierda del globo de notificación.
                let origin_logical =
                    Point::from((global_loc.x as f64 + 11.0, global_loc.y as f64 + 11.0));
                self.pending_black_holes
                    .push(crate::black_hole::PendingBlackHoleClose {
                        surface: destroyed_wl.clone(),
                        geometry: Rectangle::from_size(size),
                        location: global_loc,
                        origin_logical,
                        uses_ssd: false,
                        is_fullscreen: false,
                        is_active: true,
                        is_firefox: false,
                        // No hornear el blur opaco del fondo detrás de la tarjeta (arrastraría
                        // un bloque blanco de ventanas claras): capturar únicamente la superficie
                        // premultiplicada del globo con canal alfa limpio.
                        is_translucent: false,
                        custom_corner_radius: Some(
                            crate::media_keys::NOTIFICATION_CORNER_RADIUS as f32,
                        ),
                        blur_insets: (8, 8, 0, 0),
                        title: "Notificación".to_string(),
                        cached_layers,
                    });
            }
        }
        self.cached_close_snapshots.remove(&destroyed_wl);

        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        let should_restore_focus = self
            .seat
            .get_keyboard()
            .and_then(|kb| kb.current_focus())
            .is_some_and(|focused| {
                if focused == destroyed_wl || !focused.alive() {
                    return true;
                }
                if let Some(popup) = self.popups.find_popup(&focused) {
                    return find_popup_root_surface(&popup).ok().as_ref() == Some(&destroyed_wl);
                }
                false
            });
        if should_restore_focus {
            let top_win_surface = self
                .space
                .elements()
                .last()
                .and_then(|w| w.wl_surface().map(|s| s.into_owned()));
            self.set_keyboard_focus_with_sync(top_win_surface.clone(), serial);
            self.notify_active_app(top_win_surface.as_ref());
        }

        if let Some(pointer) = self.seat.get_pointer() {
            if pointer.is_grabbed() {
                pointer.unset_grab(self, serial, 0);
            }
            if pointer.current_focus().as_ref() == Some(surface.wl_surface()) {
                let loc = pointer.current_location();
                pointer.motion(
                    self,
                    None,
                    &smithay::input::pointer::MotionEvent {
                        location: loc,
                        serial,
                        time: 0,
                    },
                );
                pointer.frame(self);
                tracing::info!("🧹 Foco de puntero limpiado tras destruirse layer surface");
            }
        }
    }
}

/// Determina si una ventana minimizada debe incluirse en el App Switcher de la pantalla destino:
/// las minimizadas solo viven en el Dock (pantalla principal) y deben pertenecer al escritorio activo.
fn should_include_minimized_in_app_switcher(
    target_is_primary: bool,
    window_workspace: usize,
    active_workspace: usize,
) -> bool {
    target_is_primary && window_workspace == active_workspace
}

impl SessionLockHandler for MalusState {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        if self.ext_session_locked
            && self
                .ext_lock_handle
                .as_ref()
                .is_some_and(|handle| handle.is_alive())
        {
            tracing::warn!(
                "🔒 [SECURITY] Solicitud ext-session-lock-v1 rechazada: ya existe un bloqueo activo"
            );
            drop(confirmation);
            return;
        }

        let lock_res = confirmation.ext_session_lock().clone();
        let client_pid = self
            .display_handle
            .get_client(lock_res.id())
            .ok()
            .and_then(|client| client.get_credentials(&self.display_handle).ok())
            .map(|cred| cred.pid);

        self.ext_session_locked = true;
        self.ext_lock_handle = Some(lock_res);
        self.ext_lock_client_pid = client_pid;
        self.lockscreen_respawn_at = None;
        self.session_locked_flag.store(true, Ordering::SeqCst);

        self.snap_layouts.close();
        self.dismiss_app_switcher_immediately();
        self.spotlight.close();
        if self.expose.active {
            self.expose.active = false;
        }
        self.clear_dnd_icon();
        let serial = SERIAL_COUNTER.next_serial();
        self.set_keyboard_focus_with_sync(None, serial);

        confirmation.lock();
        self.surface_commit_counter = self.surface_commit_counter.wrapping_add(1);
        tracing::info!(
            "🔒 Sesión bloqueada mediante protocolo ext-session-lock-v1 (pid={:?})",
            client_pid
        );
    }

    fn unlock(&mut self) {
        self.ext_session_locked = false;
        self.ext_lock_handle = None;
        self.ext_lock_client_pid = None;
        self.ext_lock_surfaces.clear();
        self.lockscreen_respawn_at = None;
        self.session_locked_flag.store(false, Ordering::SeqCst);
        self.surface_commit_counter = self.surface_commit_counter.wrapping_add(1);

        if let Some(top_win) = self.space.elements().last().cloned() {
            if let Some(surf) = top_win.wl_surface().map(|s| s.into_owned()) {
                self.set_keyboard_focus_with_sync(Some(surf), SERIAL_COUNTER.next_serial());
            }
        }
        tracing::info!("🔓 Sesión desbloqueada vía ext-session-lock-v1");
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(out) = Output::from_resource(&output).or_else(|| self.primary_output()) else {
            return;
        };
        let size = self
            .space
            .output_geometry(&out)
            .map(|geo| Size::from((geo.size.w.max(1) as u32, geo.size.h.max(1) as u32)))
            .unwrap_or_else(|| Size::from((1920, 1080)));

        surface.with_pending_state(|state| {
            state.size = Some(size);
        });
        surface.send_configure();

        let wl_surf = surface.wl_surface().clone();
        self.ext_lock_surfaces.insert(out.name(), surface);

        if self.is_primary_output(&out) || self.lock_focus_surface().is_none() {
            self.set_keyboard_focus_with_sync(Some(wl_surf), SERIAL_COUNTER.next_serial());
        }
    }
}

delegate_session_lock!(MalusState);

fn should_skip_black_hole_for_app_id(app_id: Option<&str>) -> bool {
    let Some(id) = app_id else {
        return false;
    };
    let lower = id.to_ascii_lowercase();
    lower.contains("branches.quicklook")
        || lower.starts_with("org.malus.de.screenshot-ui")
        || lower == "dev.malus.preview"
        || lower == "malus-de-preview"
}

#[cfg(test)]
mod tests {
    use super::{is_browser_identifier, parse_color_scheme};

    #[test]
    fn quicklook_windows_do_not_persist_or_restore_geometry() {
        assert!(!super::should_persist_window_geometry(
            "dev.malus.Branches.QuickLook"
        ));
        assert!(!super::should_persist_window_geometry(
            "dev.malus.branches.quicklook"
        ));
        assert!(!super::should_persist_window_geometry("dev.malus.Preview"));
        assert!(super::should_persist_window_geometry("dev.malus.Branches"));
        assert!(super::should_persist_window_geometry("org.gnome.Ptyxis"));
    }

    #[test]
    fn recognizes_browser_identifiers() {
        assert!(is_browser_identifier("google-chrome"));
        assert!(is_browser_identifier("google-chrome-stable"));
        assert!(is_browser_identifier("chromium"));
        assert!(is_browser_identifier("org.chromium.Chromium"));
        assert!(is_browser_identifier("brave-browser"));
        assert!(is_browser_identifier("firefox"));
        assert!(is_browser_identifier("org.mozilla.firefox"));
        assert!(is_browser_identifier("microsoft-edge"));
        assert!(is_browser_identifier("msedge"));
        assert!(is_browser_identifier("edge"));
        assert!(is_browser_identifier("opera"));
        assert!(is_browser_identifier("opera-stable"));
        assert!(is_browser_identifier("com.opera.browser"));
        assert!(is_browser_identifier("zen-browser"));
        assert!(is_browser_identifier("vivaldi"));
        assert!(is_browser_identifier("org.gnome.Epiphany"));

        // Asegurar que aplicaciones del sistema y palabras en español/inglés no se confundan
        assert!(!is_browser_identifier("org.gnome.Nautilus"));
        assert!(!is_browser_identifier("nautilus"));
        assert!(!is_browser_identifier("malus-de-settings"));
        assert!(!is_browser_identifier("antigravity"));
        assert!(!is_browser_identifier("code"));
        assert!(!is_browser_identifier("org.gnome.Ptyxis"));
        assert!(!is_browser_identifier("org.gnome.TextEditor"));
        assert!(!is_browser_identifier("operaciones"));
        assert!(!is_browser_identifier("knowledge"));
        assert!(!is_browser_identifier("privilege"));
        assert!(!is_browser_identifier("recuperacion"));
    }

    #[test]
    fn recognizes_gnome_identities() {
        use super::is_gnome_identity;

        // Apps comunes de GNOME
        assert!(is_gnome_identity(Some("org.gnome.TextEditor"), None));
        assert!(is_gnome_identity(Some("org.gnome.Ptyxis"), None));
        assert!(is_gnome_identity(Some("org.gnome.Calculator"), None));
        assert!(is_gnome_identity(Some("org.gnome.Calendar"), None));
        assert!(is_gnome_identity(Some("org.gnome.Loupe"), None));
        assert!(is_gnome_identity(Some("org.gnome.SystemMonitor"), None));
        assert!(is_gnome_identity(Some("org.gnome.DiskUtility"), None));
        assert!(is_gnome_identity(Some("gnome-terminal"), None));
        assert!(is_gnome_identity(Some("gnome-calculator"), None));
        assert!(is_gnome_identity(Some("org.gnome.Nautilus"), None));
        assert!(is_gnome_identity(Some("dev.malus.Settings"), None));
        assert!(is_gnome_identity(None, Some("Editor de texto")));
        assert!(is_gnome_identity(None, Some("Calculadora")));

        // Aplicaciones que NO son GNOME y SÍ pueden recibir SSD si lo solicitan
        assert!(!is_gnome_identity(Some("zapfast"), Some("ZapFast")));
        assert!(!is_gnome_identity(Some("alacritty"), Some("Alacritty")));
        assert!(!is_gnome_identity(Some("foot"), Some("Foot")));
        assert!(!is_gnome_identity(Some("code"), Some("Visual Studio Code")));
    }

    #[test]
    fn reads_dark_and_light_color_scheme_from_session_keyfile() {
        assert_eq!(
            parse_color_scheme("[org/gnome/desktop/interface]\ncolor-scheme='prefer-dark'\n"),
            Some(true)
        );
        assert_eq!(
            parse_color_scheme("[org/gnome/desktop/interface]\ncolor-scheme='prefer-light'\n"),
            Some(false)
        );
        assert_eq!(
            parse_color_scheme("[org/gnome/desktop/interface]\ncolor-scheme='default'\n"),
            None
        );
        assert_eq!(
            parse_color_scheme("[org/gnome/desktop/wm/preferences]\ncolor-scheme='prefer-light'\n"),
            None
        );
    }

    #[test]
    fn snap_transition_interpolates_smoothly_with_cubic_ease_out() {
        use smithay::utils::{Point, Size};
        use std::time::Duration;

        let start_time = std::time::Instant::now();
        let trans = super::SnapTransition {
            start_time,
            duration: Duration::from_millis(220),
            start_loc: Point::from((100, 100)),
            start_size: Size::from((800, 600)),
            target_loc: Point::from((0, 32)),
            target_size: Size::from((1920, 1048)),
        };

        // Inicio exacto (t = 0)
        let (loc_0, sz_0) = trans.current_geometry(start_time);
        assert_eq!(loc_0, Point::from((100, 100)));
        assert_eq!(sz_0, Size::from((800, 600)));

        // Mitad de la transición (t = 110ms): curva ease-out cúbica p = 1 - (1 - 0.5)^3 = 0.875
        let (loc_half, sz_half) = trans.current_geometry(start_time + Duration::from_millis(110));
        let expected_x = (100.0_f64 + (0.0_f64 - 100.0_f64) * 0.875_f64).round() as i32;
        let expected_y = (100.0_f64 + (32.0_f64 - 100.0_f64) * 0.875_f64).round() as i32;
        let expected_w = (800.0_f64 + (1920.0_f64 - 800.0_f64) * 0.875_f64).round() as i32;
        let expected_h = (600.0_f64 + (1048.0_f64 - 600.0_f64) * 0.875_f64).round() as i32;
        assert_eq!(loc_half, Point::from((expected_x, expected_y)));
        assert_eq!(sz_half, Size::from((expected_w, expected_h)));

        // Final de la transición (t >= 220ms)
        let (loc_end, sz_end) = trans.current_geometry(start_time + Duration::from_millis(220));
        assert_eq!(loc_end, Point::from((0, 32)));
        assert_eq!(sz_end, Size::from((1920, 1048)));
        assert!(trans.is_finished(start_time + Duration::from_millis(220)));
    }

    #[test]
    fn cursor_image_and_update_ssd_hover_do_not_deadlock_while_pointer_mutex_is_locked() {
        use smithay::{
            input::{
                pointer::{
                    AxisFrame, ButtonEvent, CursorIcon, CursorImageStatus, GestureHoldBeginEvent,
                    GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
                    GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
                    GestureSwipeUpdateEvent, GrabStartData, MotionEvent, PointerGrab,
                    PointerInnerHandle, RelativeMotionEvent,
                },
                SeatHandler,
            },
            reexports::wayland_server::Display,
            utils::{Logical, Point, SERIAL_COUNTER},
        };

        struct ReentrantTestGrab {
            start_data: GrabStartData<super::MalusState>,
        }

        impl PointerGrab<super::MalusState> for ReentrantTestGrab {
            fn motion(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                _focus: Option<(
                    <super::MalusState as SeatHandler>::PointerFocus,
                    Point<f64, Logical>,
                )>,
                event: &MotionEvent,
            ) {
                handle.motion(data, None, event);
                let seat = data.seat.clone();
                data.cursor_image(&seat, CursorImageStatus::Named(CursorIcon::Pointer));
                data.update_ssd_hover(event.location);
            }
            fn relative_motion(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                focus: Option<(
                    <super::MalusState as SeatHandler>::PointerFocus,
                    Point<f64, Logical>,
                )>,
                event: &RelativeMotionEvent,
            ) {
                handle.relative_motion(data, focus, event);
            }
            fn button(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &ButtonEvent,
            ) {
                handle.unset_grab(self, data, event.serial, event.time, false);
                data.update_ssd_hover(handle.current_location());
            }
            fn axis(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                details: AxisFrame,
            ) {
                handle.axis(data, details);
            }
            fn frame(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
            ) {
                handle.frame(data);
            }
            fn gesture_swipe_begin(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GestureSwipeBeginEvent,
            ) {
                handle.gesture_swipe_begin(data, event);
            }
            fn gesture_swipe_update(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GestureSwipeUpdateEvent,
            ) {
                handle.gesture_swipe_update(data, event);
            }
            fn gesture_swipe_end(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GestureSwipeEndEvent,
            ) {
                handle.gesture_swipe_end(data, event);
            }
            fn gesture_pinch_begin(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GesturePinchBeginEvent,
            ) {
                handle.gesture_pinch_begin(data, event);
            }
            fn gesture_pinch_update(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GesturePinchUpdateEvent,
            ) {
                handle.gesture_pinch_update(data, event);
            }
            fn gesture_pinch_end(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GesturePinchEndEvent,
            ) {
                handle.gesture_pinch_end(data, event);
            }
            fn gesture_hold_begin(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GestureHoldBeginEvent,
            ) {
                handle.gesture_hold_begin(data, event);
            }
            fn gesture_hold_end(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GestureHoldEndEvent,
            ) {
                handle.gesture_hold_end(data, event);
            }
            fn start_data(&self) -> &GrabStartData<super::MalusState> {
                &self.start_data
            }
            fn unset(&mut self, data: &mut super::MalusState) {
                data.ssd.grab_active = false;
                data.ssd.cursor_override = false;
                let seat = data.seat.clone();
                data.cursor_image(&seat, CursorImageStatus::default_named());
            }
        }

        let display: Display<super::MalusState> = Display::new().unwrap();
        let mut state = super::MalusState::new(&display);
        let pointer = state.seat.add_pointer();
        let serial = SERIAL_COUNTER.next_serial();

        state.ssd.grab_active = true;
        pointer.set_grab(
            &mut state,
            ReentrantTestGrab {
                start_data: GrabStartData {
                    focus: None,
                    button: 0x110,
                    location: (100.0, 100.0).into(),
                },
            },
            serial,
            smithay::input::pointer::Focus::Clear,
        );

        pointer.motion(
            &mut state,
            None,
            &MotionEvent {
                location: (120.0, 120.0).into(),
                serial: SERIAL_COUNTER.next_serial(),
                time: 10,
            },
        );
        pointer.button(
            &mut state,
            &ButtonEvent {
                button: 0x110,
                state: smithay::backend::input::ButtonState::Released,
                serial: SERIAL_COUNTER.next_serial(),
                time: 20,
            },
        );
        assert!(!state.ssd.grab_active);
    }

    #[test]
    fn usable_zone_respects_top_bar_height() {
        use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
        use smithay::reexports::wayland_server::Display;
        use smithay::utils::Transform;

        let display: Display<super::MalusState> = Display::new().unwrap();
        let mut state = super::MalusState::new(&display);

        let output = Output::new(
            "test_output".to_string(),
            PhysicalProperties {
                size: (1920, 1080).into(),
                subpixel: Subpixel::Unknown,
                make: "Malus".to_string(),
                model: "Test Display".to_string(),
            },
        );
        let mode = Mode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            None,
            Some((0, 0).into()),
        );
        state.space.map_output(&output, (0, 0));

        let usable = state.usable_zone_for_output(&output);
        assert!(
            usable.loc.y >= 36,
            "La zona utilizable debe comenzar debajo de la Top Bar (esperado >= 36, actual = {})",
            usable.loc.y
        );
    }

    #[test]
    fn fullscreen_on_secondary_does_not_affect_primary_fullscreen_state() {
        use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
        use smithay::reexports::wayland_server::Display;
        use smithay::utils::Transform;

        let display: Display<super::MalusState> = Display::new().unwrap();
        let mut state = super::MalusState::new(&display);

        // Monitor 1: Primary (0, 0) 1920x1080
        let output1 = Output::new(
            "DP-1".to_string(),
            PhysicalProperties {
                size: (1920, 1080).into(),
                subpixel: Subpixel::Unknown,
                make: "Malus".to_string(),
                model: "Primary Display".to_string(),
            },
        );
        let mode1 = Mode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        output1.change_current_state(
            Some(mode1),
            Some(Transform::Normal),
            None,
            Some((0, 0).into()),
        );
        state.space.map_output(&output1, (0, 0));

        // Monitor 2: Secondary (1920, 0) 1920x1080
        let output2 = Output::new(
            "HDMI-A-1".to_string(),
            PhysicalProperties {
                size: (1920, 1080).into(),
                subpixel: Subpixel::Unknown,
                make: "Malus".to_string(),
                model: "Secondary Display".to_string(),
            },
        );
        let mode2 = Mode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        output2.change_current_state(
            Some(mode2),
            Some(Transform::Normal),
            None,
            Some((1920, 0).into()),
        );
        state.space.map_output(&output2, (1920, 0));

        // Sin ventanas en fullscreen, ambos monitores deben reportar false
        assert!(!state.has_fullscreen_window_on_output(&output1));
        assert!(!state.has_fullscreen_window_on_output(&output2));
    }

    #[test]
    fn test_dnd_icon_offset_calculation() {
        use smithay::utils::{Logical, Point};
        // Verificar que el cálculo de ubicación física del icono de arrastre
        // posicione la superficie compensando exactamente el hotspot (buffer_delta)
        let loc = Point::<f64, Logical>::from((250.0, 350.0));
        let hotspot_offset = Point::<i32, Logical>::from((-32, -40));
        let output_loc = Point::<i32, Logical>::from((0, 0));
        let scale = 1.0;

        let draw_x = loc.x + hotspot_offset.x as f64 - output_loc.x as f64;
        let draw_y = loc.y + hotspot_offset.y as f64 - output_loc.y as f64;
        let phys_x = (draw_x * scale).round() as i32;
        let phys_y = (draw_y * scale).round() as i32;

        // Si el puntero presionó a 32px del borde izquierdo y 40px del superior del icono (hotspot 32, 40),
        // la esquina superior izquierda del icono debe renderizarse en (250 - 32, 350 - 40) = (218, 310)
        assert_eq!(phys_x, 218);
        assert_eq!(phys_y, 310);
    }

    #[test]
    fn prepare_pointer_press_grabs_clears_residual_grabs_and_sync_none_clears_focus() {
        use smithay::{
            input::{
                keyboard::{GrabStartData as KbGrabStartData, KeyboardGrab, KeyboardInnerHandle},
                pointer::{GrabStartData, PointerGrab, PointerInnerHandle},
                SeatHandler,
            },
            reexports::wayland_server::Display,
            utils::SERIAL_COUNTER,
        };

        struct DummyKbGrab(KbGrabStartData<super::MalusState>);
        impl KeyboardGrab<super::MalusState> for DummyKbGrab {
            fn input(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut KeyboardInnerHandle<'_, super::MalusState>,
                keycode: smithay::backend::input::Keycode,
                key_state: smithay::backend::input::KeyState,
                modifiers: Option<smithay::input::keyboard::ModifiersState>,
                serial: smithay::utils::Serial,
                time: u32,
            ) {
                handle.input(data, keycode, key_state, modifiers, serial, time);
            }
            fn set_focus(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut KeyboardInnerHandle<'_, super::MalusState>,
                focus: Option<<super::MalusState as SeatHandler>::KeyboardFocus>,
                serial: smithay::utils::Serial,
            ) {
                handle.set_focus(data, focus, serial);
            }
            fn start_data(&self) -> &KbGrabStartData<super::MalusState> {
                &self.0
            }
            fn unset(&mut self, _data: &mut super::MalusState) {}
        }

        struct DummyPtrGrab(GrabStartData<super::MalusState>);
        impl PointerGrab<super::MalusState> for DummyPtrGrab {
            fn motion(
                &mut self,
                _data: &mut super::MalusState,
                _handle: &mut PointerInnerHandle<'_, super::MalusState>,
                _focus: Option<(
                    <super::MalusState as SeatHandler>::PointerFocus,
                    smithay::utils::Point<f64, smithay::utils::Logical>,
                )>,
                _event: &smithay::input::pointer::MotionEvent,
            ) {
            }
            fn relative_motion(
                &mut self,
                _data: &mut super::MalusState,
                _handle: &mut PointerInnerHandle<'_, super::MalusState>,
                _focus: Option<(
                    <super::MalusState as SeatHandler>::PointerFocus,
                    smithay::utils::Point<f64, smithay::utils::Logical>,
                )>,
                _event: &smithay::input::pointer::RelativeMotionEvent,
            ) {
            }
            fn button(
                &mut self,
                _data: &mut super::MalusState,
                _handle: &mut PointerInnerHandle<'_, super::MalusState>,
                _event: &smithay::input::pointer::ButtonEvent,
            ) {
            }
            fn axis(
                &mut self,
                _data: &mut super::MalusState,
                _handle: &mut PointerInnerHandle<'_, super::MalusState>,
                _details: smithay::input::pointer::AxisFrame,
            ) {
            }
            fn frame(
                &mut self,
                _data: &mut super::MalusState,
                _handle: &mut PointerInnerHandle<'_, super::MalusState>,
            ) {
            }
            fn gesture_swipe_begin(
                &mut self,
                _data: &mut super::MalusState,
                _handle: &mut PointerInnerHandle<'_, super::MalusState>,
                _event: &smithay::input::pointer::GestureSwipeBeginEvent,
            ) {
            }
            fn gesture_swipe_update(
                &mut self,
                _data: &mut super::MalusState,
                _handle: &mut PointerInnerHandle<'_, super::MalusState>,
                _event: &smithay::input::pointer::GestureSwipeUpdateEvent,
            ) {
            }
            fn gesture_swipe_end(
                &mut self,
                _data: &mut super::MalusState,
                _handle: &mut PointerInnerHandle<'_, super::MalusState>,
                _event: &smithay::input::pointer::GestureSwipeEndEvent,
            ) {
            }
            fn gesture_pinch_begin(
                &mut self,
                _data: &mut super::MalusState,
                _handle: &mut PointerInnerHandle<'_, super::MalusState>,
                _event: &smithay::input::pointer::GesturePinchBeginEvent,
            ) {
            }
            fn gesture_pinch_update(
                &mut self,
                _data: &mut super::MalusState,
                _handle: &mut PointerInnerHandle<'_, super::MalusState>,
                _event: &smithay::input::pointer::GesturePinchUpdateEvent,
            ) {
            }
            fn gesture_pinch_end(
                &mut self,
                _data: &mut super::MalusState,
                _handle: &mut PointerInnerHandle<'_, super::MalusState>,
                _event: &smithay::input::pointer::GesturePinchEndEvent,
            ) {
            }
            fn gesture_hold_begin(
                &mut self,
                _data: &mut super::MalusState,
                _handle: &mut PointerInnerHandle<'_, super::MalusState>,
                _event: &smithay::input::pointer::GestureHoldBeginEvent,
            ) {
            }
            fn gesture_hold_end(
                &mut self,
                _data: &mut super::MalusState,
                _handle: &mut PointerInnerHandle<'_, super::MalusState>,
                _event: &smithay::input::pointer::GestureHoldEndEvent,
            ) {
            }
            fn start_data(&self) -> &GrabStartData<super::MalusState> {
                &self.0
            }
            fn unset(&mut self, _data: &mut super::MalusState) {}
        }

        let display: Display<super::MalusState> = Display::new().unwrap();
        let mut state = super::MalusState::new(&display);
        let kb = state
            .seat
            .add_keyboard(Default::default(), 200, 25)
            .unwrap();
        let ptr = state.seat.add_pointer();
        let serial = SERIAL_COUNTER.next_serial();

        kb.set_grab(
            &mut state,
            DummyKbGrab(KbGrabStartData { focus: None }),
            serial,
        );
        ptr.set_grab(
            &mut state,
            DummyPtrGrab(GrabStartData {
                focus: None,
                button: 0x110,
                location: (50.0, 50.0).into(),
            }),
            serial,
            smithay::input::pointer::Focus::Clear,
        );
        assert!(kb.is_grabbed());
        assert!(ptr.is_grabbed());

        state.prepare_pointer_press_grabs(false, SERIAL_COUNTER.next_serial(), 100);
        assert!(!kb.is_grabbed());
        assert!(!ptr.is_grabbed());

        state.cursor_status = smithay::input::pointer::CursorImageStatus::Named(
            smithay::input::pointer::CursorIcon::Pointer,
        );
        state.set_keyboard_focus_with_sync(None, SERIAL_COUNTER.next_serial());
        assert!(kb.current_focus().is_none());
        assert!(matches!(
            state.cursor_status,
            smithay::input::pointer::CursorImageStatus::Named(
                smithay::input::pointer::CursorIcon::Default
            )
        ));
    }

    #[test]
    fn snap_target_geometry_partitions_usable_zone_seamlessly() {
        let usable = smithay::utils::Rectangle::new((0, 32).into(), (1920, 1000).into());
        let ssd_top = crate::ssd::SSD_TITLEBAR_HEIGHT;

        // 2/3 + 1/3
        let (l_loc, l_sz, _) =
            super::snap_target_geometry(super::SnapMode::LeftTwoThirds, usable, 0);
        let (r_loc, r_sz, _) =
            super::snap_target_geometry(super::SnapMode::RightOneThird, usable, 0);
        assert_eq!(l_loc.x, 0);
        assert_eq!(l_sz.unwrap().w + r_sz.unwrap().w, 1920);
        assert_eq!(l_loc.x + l_sz.unwrap().w, r_loc.x);

        // 3 columnas iguales
        let (c1, s1, _) = super::snap_target_geometry(super::SnapMode::LeftThird, usable, 0);
        let (c2, s2, _) = super::snap_target_geometry(super::SnapMode::CenterThird, usable, 0);
        let (c3, s3, _) = super::snap_target_geometry(super::SnapMode::RightThird, usable, 0);
        assert_eq!(c1.x + s1.unwrap().w, c2.x);
        assert_eq!(c2.x + s2.unwrap().w, c3.x);
        assert_eq!(s1.unwrap().w + s2.unwrap().w + s3.unwrap().w, 1920);

        // 25 / 50 / 25
        let (q1, qs1, _) = super::snap_target_geometry(super::SnapMode::LeftQuarter, usable, 0);
        let (q2, qs2, _) = super::snap_target_geometry(super::SnapMode::CenterHalf, usable, 0);
        let (q3, qs3, _) = super::snap_target_geometry(super::SnapMode::RightQuarter, usable, 0);
        assert_eq!(q1.x + qs1.unwrap().w, q2.x);
        assert_eq!(q2.x + qs2.unwrap().w, q3.x);
        assert_eq!(qs1.unwrap().w, 480);
        assert_eq!(qs2.unwrap().w, 960);
        assert_eq!(qs3.unwrap().w, 480);

        // Cuartos verticales con barra SSD (sin solapamiento de barra de título)
        let (tl_loc, tl_sz, _) =
            super::snap_target_geometry(super::SnapMode::TopLeftQuarter, usable, ssd_top);
        let (bl_loc, bl_sz, _) =
            super::snap_target_geometry(super::SnapMode::BottomLeftQuarter, usable, ssd_top);
        assert_eq!(tl_loc.y - ssd_top, 32);
        assert_eq!(tl_loc.y + tl_sz.unwrap().h, bl_loc.y - ssd_top);
        assert_eq!(bl_loc.y + bl_sz.unwrap().h, 1032);
    }

    #[test]
    fn matches_app_target_handles_malus_branches_and_reverse_dns_ids() {
        assert!(super::matches_app_target(
            "malus-de-branches",
            Some("dev.malus.Branches"),
            Some("Escritorio")
        ));
        assert!(super::matches_app_target(
            "dev.malus.branches",
            Some("dev.malus.Branches"),
            Some("Descargas")
        ));
        assert!(super::matches_app_target(
            "gnome-text-editor",
            Some("org.gnome.TextEditor"),
            Some("notas.txt")
        ));
        assert!(!super::matches_app_target(
            "malus-de-branches",
            Some("org.gnome.Ptyxis"),
            Some("Terminal")
        ));
    }

    #[test]
    fn snap_transition_render_geometry_keeps_static_1_to_1_scale() {
        use smithay::utils::{Logical, Point, Rectangle, Size};
        use std::time::Duration;

        let start_time = std::time::Instant::now();
        let trans = super::SnapTransition {
            start_time,
            duration: Duration::from_millis(220),
            start_loc: Point::from((100, 100)),
            start_size: Size::from((800, 600)),
            target_loc: Point::from((0, 32)),
            target_size: Size::from((1920, 1048)),
        };

        let mid = start_time + Duration::from_millis(110);

        // Antes de que el cliente envíe el buffer maximizado (geo = 800x600 con márgenes CSD 26px):
        // la superficie se posiciona 1:1 (phys_loc = effective_loc - geo.loc) y se recorta a 800x600.
        let uncommited_geo =
            Rectangle::<i32, Logical>::new(Point::from((26, 26)), Size::from((800, 600)));
        let (eff_loc, eff_sz, phys_loc_before, clip_before, _, _, win_rect_before) =
            trans.render_geometry(uncommited_geo, Point::from((0, 0)), 1.0, mid);
        assert_eq!(
            phys_loc_before,
            Point::from((eff_loc.x - 26, eff_loc.y - 26))
        );
        let clip_before = clip_before.expect("debe devolver recorte en transición");
        assert_eq!(clip_before.loc, win_rect_before.loc);
        assert_eq!(clip_before.size, Size::from((800, 600)));
        assert!(eff_sz.w > 800 && eff_sz.h > 600);

        // Una vez que el cliente confirma el tamaño maximizado (geo = 1920x1048 >= effective_size):
        // el recorte coincide exactamente con win_screen_rect y mantiene escala 1:1.
        let committed_geo =
            Rectangle::<i32, Logical>::new(Point::from((0, 0)), Size::from((1920, 1048)));
        let (eff_loc2, _, phys_loc_after, clip_after, _, _, win_rect_after) =
            trans.render_geometry(committed_geo, Point::from((0, 0)), 1.0, mid);
        assert_eq!(phys_loc_after, eff_loc2.to_physical_precise_round(1.0));
        assert_eq!(clip_after, Some(win_rect_after));
    }

    #[test]
    fn app_switcher_isolates_minimized_windows_to_primary_output_and_active_workspace() {
        // Pantalla principal + mismo escritorio -> sí se incluye
        assert!(super::should_include_minimized_in_app_switcher(true, 0, 0));
        assert!(super::should_include_minimized_in_app_switcher(true, 1, 1));

        // Pantalla principal + otro escritorio -> NO se incluye
        assert!(!super::should_include_minimized_in_app_switcher(true, 0, 1));

        // Pantalla secundaria (sin Dock) -> nunca se incluyen minimizadas
        assert!(!super::should_include_minimized_in_app_switcher(
            false, 0, 0
        ));
        assert!(!super::should_include_minimized_in_app_switcher(
            false, 1, 0
        ));
    }

    #[test]
    fn ssd_corner_drag_resolves_output_without_locking_pointer_mutex() {
        use smithay::{
            input::{
                pointer::{
                    AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
                    GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
                    GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
                    GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
                    RelativeMotionEvent,
                },
                SeatHandler,
            },
            output::{Mode, Output, PhysicalProperties, Subpixel},
            reexports::wayland_server::Display,
            utils::{Logical, Point, Rectangle, Size, Transform, SERIAL_COUNTER},
        };

        let display: Display<super::MalusState> = Display::new().unwrap();
        let mut state = super::MalusState::new(&display);
        let pointer = state.seat.add_pointer();

        let output = Output::new(
            "DP-1".to_string(),
            PhysicalProperties {
                size: (1920, 1080).into(),
                subpixel: Subpixel::Unknown,
                make: "Malus".to_string(),
                model: "Test".to_string(),
            },
        );
        output.change_current_state(
            Some(Mode {
                size: (1920, 1080).into(),
                refresh: 60_000,
            }),
            Some(Transform::Normal),
            None,
            Some((0, 0).into()),
        );
        state.space.map_output(&output, (0, 0));

        let outputs = vec![(
            output.clone(),
            Rectangle::new((0, 0).into(), (1920, 1080).into()),
        )];
        // Ventana SSD arrastrada a esquina inferior derecha: cliente en y=1100 >= 1080, barra SSD en [1062, 1100).
        let resolved_bottom = super::resolve_origin_output_from_bounds(
            &outputs,
            Point::from((1400, 1100)),
            Size::from((800, 600)),
            crate::ssd::SSD_TITLEBAR_HEIGHT,
            None,
        );
        assert_eq!(resolved_bottom, Some(output.clone()));

        // Ventana arrastrada completamente fuera del borde izquierdo: cae al fallback de `last_pointer_location`.
        let resolved_left = super::resolve_origin_output_from_bounds(
            &outputs,
            Point::from((-900, 50)),
            Size::from((800, 600)),
            crate::ssd::SSD_TITLEBAR_HEIGHT,
            Some(Point::from((0.0, 50.0))),
        );
        assert_eq!(resolved_left, Some(output.clone()));

        struct CornerGrab {
            start_data: GrabStartData<super::MalusState>,
            output: Output,
        }
        impl PointerGrab<super::MalusState> for CornerGrab {
            fn motion(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                _focus: Option<(
                    <super::MalusState as SeatHandler>::PointerFocus,
                    Point<f64, Logical>,
                )>,
                event: &MotionEvent,
            ) {
                data.last_pointer_location = Some(event.location);
                handle.motion(data, None, event);
                let _ = data.has_fullscreen_window_on_output(&self.output);
                data.update_ssd_hover(event.location);
            }
            fn relative_motion(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                focus: Option<(
                    <super::MalusState as SeatHandler>::PointerFocus,
                    Point<f64, Logical>,
                )>,
                event: &RelativeMotionEvent,
            ) {
                handle.relative_motion(data, focus, event);
            }
            fn button(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &ButtonEvent,
            ) {
                data.last_pointer_location = Some(handle.current_location());
                smithay::wayland::selection::data_device::ClientDndGrabHandler::dropped(
                    data,
                    None,
                    true,
                    data.seat.clone(),
                );
                handle.unset_grab(self, data, event.serial, event.time, false);
                data.update_ssd_hover(handle.current_location());
            }
            fn axis(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                details: AxisFrame,
            ) {
                handle.axis(data, details);
            }
            fn frame(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
            ) {
                handle.frame(data);
            }
            fn gesture_swipe_begin(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GestureSwipeBeginEvent,
            ) {
                handle.gesture_swipe_begin(data, event);
            }
            fn gesture_swipe_update(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GestureSwipeUpdateEvent,
            ) {
                handle.gesture_swipe_update(data, event);
            }
            fn gesture_swipe_end(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GestureSwipeEndEvent,
            ) {
                handle.gesture_swipe_end(data, event);
            }
            fn gesture_pinch_begin(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GesturePinchBeginEvent,
            ) {
                handle.gesture_pinch_begin(data, event);
            }
            fn gesture_pinch_update(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GesturePinchUpdateEvent,
            ) {
                handle.gesture_pinch_update(data, event);
            }
            fn gesture_pinch_end(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GesturePinchEndEvent,
            ) {
                handle.gesture_pinch_end(data, event);
            }
            fn gesture_hold_begin(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GestureHoldBeginEvent,
            ) {
                handle.gesture_hold_begin(data, event);
            }
            fn gesture_hold_end(
                &mut self,
                data: &mut super::MalusState,
                handle: &mut PointerInnerHandle<'_, super::MalusState>,
                event: &GestureHoldEndEvent,
            ) {
                handle.gesture_hold_end(data, event);
            }
            fn start_data(&self) -> &GrabStartData<super::MalusState> {
                &self.start_data
            }
            fn unset(&mut self, data: &mut super::MalusState) {
                data.ssd.grab_active = false;
            }
        }

        state.ssd.grab_active = true;
        pointer.set_grab(
            &mut state,
            CornerGrab {
                start_data: GrabStartData {
                    focus: None,
                    button: 0x110,
                    location: (1800.0, 1000.0).into(),
                },
                output: output.clone(),
            },
            SERIAL_COUNTER.next_serial(),
            smithay::input::pointer::Focus::Clear,
        );
        pointer.motion(
            &mut state,
            None,
            &MotionEvent {
                location: (1919.0, 1079.0).into(),
                serial: SERIAL_COUNTER.next_serial(),
                time: 10,
            },
        );
        pointer.button(
            &mut state,
            &ButtonEvent {
                button: 0x110,
                state: smithay::backend::input::ButtonState::Released,
                serial: SERIAL_COUNTER.next_serial(),
                time: 20,
            },
        );
        assert_eq!(
            state.last_pointer_location,
            Some(Point::from((1919.0, 1079.0)))
        );
        assert!(!state.ssd.grab_active);
    }

    #[test]
    fn quick_look_and_preview_windows_skip_black_hole_close_effect() {
        assert!(super::should_skip_black_hole_for_app_id(Some(
            "dev.malus.Branches.QuickLook"
        )));
        assert!(super::should_skip_black_hole_for_app_id(Some(
            "org.malus.de.screenshot-ui-1234"
        )));
        assert!(super::should_skip_black_hole_for_app_id(Some(
            "dev.malus.Preview"
        )));
        assert!(!super::should_skip_black_hole_for_app_id(Some(
            "dev.malus.Branches"
        )));
        assert!(!super::should_skip_black_hole_for_app_id(None));
    }

    #[test]
    fn close_button_offset_matches_ssd_and_csd_window_families() {
        assert_eq!(
            super::MalusState::close_button_offset_for_app_id(Some("org.test.App"), false, true),
            (crate::ssd::BTN_CLOSE_X, crate::ssd::BTN_Y)
        );
        assert_eq!(
            super::MalusState::close_button_offset_for_app_id(
                Some("dev.malus.Branches"),
                false,
                false
            ),
            (20.5, 24.0)
        );
        assert_eq!(
            super::MalusState::close_button_offset_for_app_id(
                Some("dev.malus.Settings"),
                false,
                false
            ),
            (20.5, 24.0)
        );
        assert_eq!(
            super::MalusState::close_button_offset_for_app_id(Some("antigravity"), false, false),
            (20.5, 17.5)
        );
        assert_eq!(
            super::MalusState::close_button_offset_for_app_id(Some("firefox"), true, false),
            (16.5, 21.0)
        );
        assert_eq!(
            super::MalusState::close_button_offset_for_app_id(
                Some("org.gnome.TextEditor"),
                false,
                false
            ),
            (22.0, 23.0)
        );
    }

    #[test]
    fn csd_close_zone_covers_left_and_right_headerbars() {
        assert_eq!(
            super::MalusState::is_in_csd_close_zone(20.0, 24.0, 900.0),
            Some(false)
        );
        assert_eq!(
            super::MalusState::is_in_csd_close_zone(878.0, 24.0, 900.0),
            Some(true)
        );
        assert_eq!(
            super::MalusState::is_in_csd_close_zone(450.0, 24.0, 900.0),
            None
        );
        assert_eq!(
            super::MalusState::is_in_csd_close_zone(20.0, 120.0, 900.0),
            None
        );
    }

    #[test]
    fn initial_bounds_and_clamping_respect_usable_zone_without_min_inflation() {
        use smithay::utils::{Logical, Rectangle, Size};
        let usable = Rectangle::<i32, Logical>::new((0, 36).into(), (1920, 960).into());
        assert_eq!(
            super::max_window_size_in_usable_zone(usable, 0),
            Size::from((1920, 960))
        );
        let ssd_max =
            super::max_window_size_in_usable_zone(usable, crate::ssd::SSD_TITLEBAR_HEIGHT);
        assert_eq!(
            ssd_max,
            Size::from((1920, 960 - crate::ssd::SSD_TITLEBAR_HEIGHT))
        );

        // Ventanas pequeñas mantienen el tamaño de su contenido (no se inflan a 320x240)
        assert_eq!(
            super::clamp_size_to_bounds(Size::from((210, 140)), ssd_max),
            Size::from((210, 140))
        );
        // Ventanas enormes o tamaños recordados de un monitor mayor quedan acotados al área útil
        assert_eq!(
            super::clamp_size_to_bounds(Size::from((3840, 2160)), ssd_max),
            ssd_max
        );
    }

    #[test]
    fn centered_window_location_centers_csd_and_ssd_inside_usable_zone() {
        use smithay::utils::{Logical, Point, Rectangle};
        let usable = Rectangle::<i32, Logical>::new((100, 36).into(), (1600, 900).into());

        // Ventana CSD con sombra de 24px (geometry.loc = (24, 24))
        let csd_geo = Rectangle::new((24, 24).into(), (800, 500).into());
        assert_eq!(
            super::centered_window_location(usable, csd_geo, 0),
            Point::from((100 + 400 - 24, 36 + 200 - 24))
        );

        // Ventana SSD cuya barra de título mide SSD_TITLEBAR_HEIGHT por encima de loc.y
        let ssd_h = crate::ssd::SSD_TITLEBAR_HEIGHT;
        let ssd_geo = Rectangle::new((0, 0).into(), (800, 900 - ssd_h).into());
        let loc = super::centered_window_location(usable, ssd_geo, ssd_h);
        assert_eq!(loc, Point::from((100 + 400, 36 + ssd_h)));

        // Sin posición guardada (`None`), usa la posición centrada
        assert_eq!(
            super::clamped_or_centered_window_location(usable, csd_geo, 0, None),
            Point::from((100 + 400 - 24, 36 + 200 - 24))
        );

        // Con posición visible guardada dentro del área útil, la restaura compensando sombra CSD
        assert_eq!(
            super::clamped_or_centered_window_location(
                usable,
                csd_geo,
                0,
                Some(Point::from((260, 120))),
            ),
            Point::from((260 - 24, 120 - 24))
        );

        // Si la posición guardada excede los bordes del escritorio o invade la Top Bar, se acota a `usable`
        assert_eq!(
            super::clamped_or_centered_window_location(
                usable,
                csd_geo,
                0,
                Some(Point::from((5000, -40))),
            ),
            Point::from((100 + 1600 - 800 - 24, 36 - 24))
        );
    }

    #[test]
    fn migrate_orphaned_windows_and_pointer_when_secondary_output_is_removed() {
        use smithay::{
            output::{Mode, Output, PhysicalProperties, Subpixel},
            reexports::wayland_server::Display,
            utils::{Point, Size, Transform},
        };

        let display: Display<super::MalusState> = Display::new().unwrap();
        let mut state = super::MalusState::new(&display);
        let _pointer = state.seat.add_pointer();

        let primary = Output::new(
            "DP-1".to_string(),
            PhysicalProperties {
                size: (600, 340).into(),
                subpixel: Subpixel::Unknown,
                make: "Malus".to_string(),
                model: "Primary".to_string(),
            },
        );
        let p_mode = Mode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        primary.change_current_state(
            Some(p_mode),
            Some(Transform::Normal),
            None,
            Some((0, 0).into()),
        );
        primary.set_preferred(p_mode);
        state.space.map_output(&primary, (0, 0));

        // Puntero estaba en el segundo monitor (x=2400) ya desmapeado de Space
        state.last_pointer_location = Some(Point::from((2400.0, 500.0)));
        state.migrate_orphaned_windows_and_pointer();

        assert_eq!(
            state.last_pointer_location,
            Some(Point::from((960.0, 540.0)))
        );
        assert_eq!(state.space.outputs().count(), 1);
        let _ = Size::<i32, smithay::utils::Logical>::from((0, 0));
    }

    #[test]
    fn session_lock_queries_do_not_deadlock_while_output_layer_map_is_locked() {
        use smithay::{
            output::{Mode, Output, PhysicalProperties, Subpixel},
            reexports::wayland_server::Display,
            utils::{Transform, SERIAL_COUNTER},
        };

        let display: Display<super::MalusState> = Display::new().unwrap();
        let mut state = super::MalusState::new(&display);
        let _keyboard = state
            .seat
            .add_keyboard(Default::default(), 200, 200)
            .unwrap();

        let output = Output::new(
            "DP-1".to_string(),
            PhysicalProperties {
                size: (600, 340).into(),
                subpixel: Subpixel::Unknown,
                make: "Malus".to_string(),
                model: "Primary".to_string(),
            },
        );
        let mode = Mode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            None,
            Some((0, 0).into()),
        );
        output.set_preferred(mode);
        state.space.map_output(&output, (0, 0));

        // Simula el estado del bucle de renderizado DRM/Winit manteniendo bloqueado `LayerMap`
        let held_layer_map = smithay::desktop::layer_map_for_output(&output);
        assert!(!state.is_session_locked());
        assert!(state.lock_focus_surface().is_none());

        // Verificar que preparar el OSD de teclas multimedia con `LayerMap` bloqueado no cause deadlock
        let now = std::time::Instant::now();
        state.media_keys.show_osd(
            "DP-1".to_string(),
            crate::media_keys::OsdIconKind::VolumeMedium,
            "Volumen",
            "50%",
            Some(50.0),
            false,
            now,
        );
        let output_geo = state.space.output_geometry(&output).unwrap_or_default();
        let usable_zone = state.usable_zone_from_layer_map(&output, &held_layer_map);
        let osd_prepared = state.media_keys.prepare_frame(
            "DP-1",
            output_geo,
            usable_zone,
            1.0,
            false,
            now,
        );
        assert!(osd_prepared.is_some());

        state.ext_session_locked = true;
        assert!(state.is_session_locked());
        state.set_keyboard_focus_with_sync(None, SERIAL_COUNTER.next_serial());
    }

    #[test]
    fn commit_ignores_non_root_surfaces_for_window_centering() {
        let state_src = include_str!("state.rs");
        assert!(state_src.contains("fn is_root_window_surface"));
        assert!(state_src.contains("if is_root_window_surface(&window, surface)"));
        assert!(state_src.contains("if is_root_window_surface(&win, surface)"));
    }

    #[test]
    fn primary_output_for_config_selects_configured_primary_display() {
        use malus_de_common::config::{DisplayOutputConfig, DisplaysConfig};
        use smithay::{
            output::{Mode, Output, PhysicalProperties, Subpixel},
            reexports::wayland_server::Display,
            utils::Transform,
        };

        let display: Display<super::MalusState> = Display::new().unwrap();
        let mut state = super::MalusState::new(&display);

        let make_out = |name: &str, x: i32| {
            let out = Output::new(
                name.to_string(),
                PhysicalProperties {
                    size: (600, 340).into(),
                    subpixel: Subpixel::Unknown,
                    make: "Malus".to_string(),
                    model: name.to_string(),
                },
            );
            let mode = Mode {
                size: (1920, 1080).into(),
                refresh: 60_000,
            };
            out.change_current_state(
                Some(mode),
                Some(Transform::Normal),
                None,
                Some((x, 0).into()),
            );
            out.set_preferred(mode);
            out
        };

        let out1 = make_out("DP-1", 0);
        let out2 = make_out("DP-2", 1920);
        state.space.map_output(&out1, (0, 0));
        state.space.map_output(&out2, (1920, 0));

        let mut cfg = DisplaysConfig::default();
        cfg.outputs = vec![
            DisplayOutputConfig {
                connector: "DP-1".to_string(),
                name: "DP-1".to_string(),
                width: 1920,
                height: 1080,
                refresh_rate: 60.0,
                modes: vec![],
                x: 0,
                y: 0,
                scale: 1.0,
                primary: false,
                brightness: 100.0,
                diagonal_inches: None,
            },
            DisplayOutputConfig {
                connector: "DP-2".to_string(),
                name: "DP-2".to_string(),
                width: 1920,
                height: 1080,
                refresh_rate: 60.0,
                modes: vec![],
                x: 1920,
                y: 0,
                scale: 1.0,
                primary: true,
                brightness: 100.0,
                diagonal_inches: None,
            },
        ];

        assert_eq!(
            state.primary_output_for_config(&cfg).map(|o| o.name()),
            Some("DP-2".to_string())
        );
        // Si DP-2 ya no está mapeado (desconectado), el monitor sobreviviente pasa a ser primario.
        state.space.unmap_output(&out2);
        assert_eq!(
            state.primary_output_for_config(&cfg).map(|o| o.name()),
            Some("DP-1".to_string())
        );
        assert!(super::is_primary_shell_layer_namespace("malus-top-bar"));
        assert!(super::is_primary_shell_layer_namespace("malus-dock"));
        assert!(super::is_primary_shell_layer_namespace(
            "malus-app-switcher"
        ));
        assert!(!super::is_primary_shell_layer_namespace("malus-wallpaper"));
    }

    #[test]
    fn compute_auto_tiling_slots_partitions_without_overlap() {
        let usable = smithay::utils::Rectangle::new((0, 36).into(), (1920, 1000).into());
        assert!(super::compute_auto_tiling_slots(0, usable, 8).is_empty());

        for count in 1..=6 {
            let slots = super::compute_auto_tiling_slots(count, usable, 8);
            assert_eq!(slots.len(), count);
            for (i, a) in slots.iter().enumerate() {
                assert!(
                    usable.contains_rect(*a),
                    "slot {i} out of usable bounds: {a:?}"
                );
                for (j, b) in slots.iter().enumerate().skip(i + 1) {
                    assert!(
                        !a.overlaps(*b),
                        "slots {i} and {j} overlap for count={count}: {a:?} vs {b:?}"
                    );
                }
            }
        }

        // Ultrawide (relación >= 2.1:1): con 3 ventanas deben ser 3 columnas de altura completa.
        let ultrawide = smithay::utils::Rectangle::new((0, 36).into(), (3440, 1400).into());
        let uw_slots = super::compute_auto_tiling_slots(3, ultrawide, 8);
        assert_eq!(uw_slots.len(), 3);
        assert_eq!(uw_slots[0].size.h, uw_slots[1].size.h);
        assert_eq!(uw_slots[1].size.h, uw_slots[2].size.h);
        assert!(uw_slots[0].loc.x < uw_slots[1].loc.x);
        assert!(uw_slots[1].loc.x < uw_slots[2].loc.x);
    }

    #[test]
    fn auto_tiling_capacity_scales_with_logical_resolution() {
        use smithay::utils::Size;

        // Pantallas pequeñas (< 2000 lógico) -> 2 ventanas
        assert_eq!(super::auto_tiling_capacity_for_size(Size::from((1366, 768))), 2);
        assert_eq!(super::auto_tiling_capacity_for_size(Size::from((1920, 1080))), 2);
        assert_eq!(super::auto_tiling_capacity_for_size(Size::from((1999, 1080))), 2);

        // Pantallas grandes (2000..3199 lógico) -> 3 ventanas
        assert_eq!(super::auto_tiling_capacity_for_size(Size::from((2000, 1080))), 3);
        assert_eq!(super::auto_tiling_capacity_for_size(Size::from((2560, 1440))), 3);
        assert_eq!(super::auto_tiling_capacity_for_size(Size::from((2880, 1620))), 3);
        assert_eq!(super::auto_tiling_capacity_for_size(Size::from((3199, 1800))), 3);

        // Pantallas enormes (>= 3200 lógico) -> 6 ventanas
        assert_eq!(super::auto_tiling_capacity_for_size(Size::from((3200, 1800))), 6);
        assert_eq!(super::auto_tiling_capacity_for_size(Size::from((3440, 1440))), 6);
        assert_eq!(super::auto_tiling_capacity_for_size(Size::from((3840, 2160))), 6);
        assert_eq!(super::auto_tiling_capacity_for_size(Size::from((5120, 1440))), 6);
    }

    #[test]
    fn auto_tiling_single_monitor_overflows_by_resolution_tier() {
        use super::NewWindowTileTarget;

        // Pantalla pequeña (cap = 2): 1ª y 2ª en escritorio 0; 3ª -> escritorio 1; 5ª -> crea escritorio 2.
        assert_eq!(
            super::plan_new_window_placement(&[2], 0, &[1], 0, &[1, 0], 8),
            NewWindowTileTarget::CurrentWorkspace { output_idx: 0 }
        );
        assert_eq!(
            super::plan_new_window_placement(&[2], 0, &[2], 0, &[2, 0], 8),
            NewWindowTileTarget::ExistingWorkspace { workspace_idx: 1 }
        );
        assert_eq!(
            super::plan_new_window_placement(&[2], 0, &[2], 1, &[2, 2], 8),
            NewWindowTileTarget::CreateWorkspace { workspace_idx: 2 }
        );

        // Pantalla grande (cap = 3): hasta 3 en escritorio 0; 4ª -> escritorio 1.
        assert_eq!(
            super::plan_new_window_placement(&[3], 0, &[2], 0, &[2, 0], 8),
            NewWindowTileTarget::CurrentWorkspace { output_idx: 0 }
        );
        assert_eq!(
            super::plan_new_window_placement(&[3], 0, &[3], 0, &[3, 0], 8),
            NewWindowTileTarget::ExistingWorkspace { workspace_idx: 1 }
        );

        // Pantalla enorme (cap = 6): hasta 6 en escritorio 0; 7ª -> escritorio 1.
        assert_eq!(
            super::plan_new_window_placement(&[6], 0, &[5], 0, &[5, 0], 8),
            NewWindowTileTarget::CurrentWorkspace { output_idx: 0 }
        );
        assert_eq!(
            super::plan_new_window_placement(&[6], 0, &[6], 0, &[6, 0], 8),
            NewWindowTileTarget::ExistingWorkspace { workspace_idx: 1 }
        );
    }

    #[test]
    fn auto_tiling_fills_secondary_monitor_before_second_workspace() {
        use super::NewWindowTileTarget;

        // Monitor principal grande (cap=3) + monitor secundario pequeño (cap=2) => 5 por escritorio.
        let caps = [3, 2];

        // Ventana 4 cuando el principal está lleno (3/3) y el secundario vacío (0/2) -> monitor secundario en mismo escritorio.
        assert_eq!(
            super::plan_new_window_placement(&caps, 0, &[3, 0], 0, &[3, 0], 8),
            NewWindowTileTarget::CurrentWorkspace { output_idx: 1 }
        );
        // Ventana 5 -> llena segundo hueco del monitor secundario en el mismo escritorio.
        assert_eq!(
            super::plan_new_window_placement(&caps, 0, &[3, 1], 0, &[4, 0], 8),
            NewWindowTileTarget::CurrentWorkspace { output_idx: 1 }
        );
        // Ventana 6 -> ambos monitores llenos (3/3 y 2/2) => desborda a Escritorio 2.
        assert_eq!(
            super::plan_new_window_placement(&caps, 0, &[3, 2], 0, &[5, 0], 8),
            NewWindowTileTarget::ExistingWorkspace { workspace_idx: 1 }
        );

        // Con los 8 escritorios completamente llenos, degrada al monitor menos cargado sin perder la ventana.
        let all_full = [5; 8];
        assert_eq!(
            super::plan_new_window_placement(&caps, 0, &[3, 2], 7, &all_full, 8),
            NewWindowTileTarget::FallbackWorkspace {
                workspace_idx: 7,
                output_idx: 1
            }
        );
    }

    #[test]
    fn auto_tiling_rebalance_distributes_across_monitors_and_workspaces() {
        // Principal grande (3) + secundario pequeño (2), 7 ventanas abiertas todas en monitor 0.
        let plan = super::plan_active_workspace_rebalance(
            &[3, 2],
            &[0, 0, 0, 0, 0, 0, 0],
            0,
            &[7, 0],
            8,
        );
        assert_eq!(plan.per_output, vec![vec![0, 1, 2], vec![3, 4]]);
        assert_eq!(plan.overflow_to_workspaces, vec![(5, 1), (6, 1)]);

        // Rebalanceo masivo (12 ventanas en 2+2 monitores, 2 escritorios iniciales):
        // 4 en ws 0, 4 en ws 1, 4 en nuevo ws 2.
        let plan_many = super::plan_active_workspace_rebalance(
            &[2, 2],
            &[0; 12],
            0,
            &[12, 0],
            8,
        );
        assert_eq!(plan_many.per_output, vec![vec![0, 1], vec![2, 3]]);
        assert_eq!(
            plan_many.overflow_to_workspaces,
            vec![
                (4, 1),
                (5, 1),
                (6, 1),
                (7, 1),
                (8, 2),
                (9, 2),
                (10, 2),
                (11, 2),
            ]
        );
    }

    #[test]
    fn dock_auto_hide_zone_detects_screen_edges_and_dock_body() {
        use malus_de_common::config::DockPosition;
        let out = smithay::utils::Rectangle::new((0, 0).into(), (2560, 1440).into());
        let dock_bottom = Some(smithay::utils::Rectangle::new(
            (1000, 1320).into(),
            (560, 116).into(),
        ));
        assert!(super::MalusState::is_pointer_on_dock_auto_hide_zone(
            smithay::utils::Point::from((100.0, 1438.0)),
            out,
            DockPosition::Bottom,
            4,
            36,
            None,
        ));
        assert!(super::MalusState::is_pointer_on_dock_auto_hide_zone(
            smithay::utils::Point::from((1200.0, 1380.0)),
            out,
            DockPosition::Bottom,
            4,
            36,
            dock_bottom,
        ));
        assert!(!super::MalusState::is_pointer_on_dock_auto_hide_zone(
            smithay::utils::Point::from((1200.0, 900.0)),
            out,
            DockPosition::Bottom,
            4,
            36,
            dock_bottom,
        ));

        assert!(super::MalusState::is_pointer_on_dock_auto_hide_zone(
            smithay::utils::Point::from((2.0, 700.0)),
            out,
            DockPosition::Left,
            4,
            36,
            None,
        ));
        assert!(!super::MalusState::is_pointer_on_dock_auto_hide_zone(
            smithay::utils::Point::from((2.0, 20.0)),
            out,
            DockPosition::Left,
            4,
            36,
            None,
        ));
        assert!(super::MalusState::is_pointer_on_dock_auto_hide_zone(
            smithay::utils::Point::from((2558.0, 700.0)),
            out,
            DockPosition::Right,
            4,
            36,
            None,
        ));
    }

    #[test]
    fn layer_destroy_commit_guard_prevents_smithay_invalid_size_disconnect() {
        let state_src = include_str!("state.rs");
        assert!(state_src.contains("struct LayerRoleDestroyedMarker"));
        assert!(state_src.contains("fn new_surface(&mut self, surface: &WlSurface)"));
        assert!(state_src.contains("marker.0.store(true, Ordering::Release)"));
        assert!(state_src.contains("marker.0.store(false, Ordering::Release)"));
        assert!(state_src.contains("pending.size.w = 1"));
        assert!(state_src.contains("pending.size.h = 1"));
    }

    #[test]
    fn activation_sync_uses_has_pending_changes_and_move_request_validates_serial() {
        let state_src = include_str!("state.rs");
        assert!(state_src.contains("if toplevel.has_pending_changes()"));
        assert!(state_src.contains("toplevel.send_pending_configure()"));
        assert!(state_src.contains("if !pointer.has_grab(serial)"));
        assert!(state_src.contains("fn is_window_visually_active("));
    }
}
