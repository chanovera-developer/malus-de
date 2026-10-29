use std::collections::HashMap;
use std::os::unix::io::OwnedFd;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use crate::cpu_priority::FocusCpuPriority;
use malus_de_common::config::DockConfig;
use smithay::{
    delegate_compositor, delegate_data_control, delegate_data_device, delegate_fractional_scale,
    delegate_layer_shell, delegate_output, delegate_primary_selection, delegate_seat, delegate_shm,
    delegate_viewporter, delegate_xdg_decoration, delegate_xdg_shell,
    desktop::{
        find_popup_root_surface, layer_map_for_output, PopupKind, PopupManager, Space,
    },
    input::{Seat, SeatHandler, SeatState},
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel::{ResizeEdge, State as XdgState},
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{
                wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface,
            },
            Client, Display, DisplayHandle,
        },
    },
    utils::{Logical, Point, Rectangle, Serial, Size},
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        fractional_scale::{with_fractional_scale, FractionalScaleHandler, FractionalScaleManagerState},
        output::{OutputHandler, OutputManagerState},
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
        shell::{
            wlr_layer::{
                Layer, LayerSurface, WlrLayerShellHandler, WlrLayerShellState,
            },
            xdg::{
                PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            },
        },
        seat::WaylandFocus,
        shm::{ShmHandler, ShmState},
        viewporter::ViewporterState,
    },
};

use malus_de_common::ipc::{AppSwitcherItem, IpcBroadcaster, IpcMessage};

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
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

/// Estado en memoria del selector de aplicaciones visual (⌘Tab / App Switcher HUD)
#[derive(Debug, Default, Clone)]
pub struct AppSwitcherState {
    pub is_active: bool,
    pub items: Vec<AppSwitcherItem>,
    pub selected_index: usize,
}

#[derive(Debug, Clone)]
pub struct ThemeTransitionCapture {
    pub request_id: String,
    pub origin: Point<f64, Logical>,
}

pub struct MalusState {
    pub display_handle: DisplayHandle,
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: smithay::wayland::shell::xdg::decoration::XdgDecorationState,
    pub layer_shell_state: WlrLayerShellState,
    pub fractional_scale_manager_state: FractionalScaleManagerState,
    pub viewporter_state: ViewporterState,
    pub shm_state: ShmState,
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
    pub window_restore_geometries: HashMap<WlSurface, (Point<i32, Logical>, Size<i32, Logical>)>,
    pub centered_transient_windows: HashMap<WlSurface, (Point<i32, Logical>, Size<i32, Logical>)>,
    pub minimized_windows: HashMap<WlSurface, (smithay::desktop::Window, Point<i32, Logical>)>,
    pub minimized_window_thumbnails: HashMap<u64, String>,
    pub window_ids: HashMap<WlSurface, u64>,
    pub next_window_id: u64,
    pub dock_slots: HashMap<u64, Rectangle<i32, Logical>>,
    pub active_animations: Vec<GenieAnimation>,
    pub snap_preview: Option<Rectangle<i32, Logical>>,
    pub socket_name: String,
    pub is_running: bool,
    pub ipc_broadcaster: IpcBroadcaster,
    pub app_switcher: AppSwitcherState,
    pub cursor_status: smithay::input::pointer::CursorImageStatus,
    pub pending_displays_reload: bool,
    pub saved_app_window_sizes: HashMap<String, Size<i32, Logical>>,
    pub dock_blur_headroom: i32,
    pub pending_screenshot_requests: std::collections::VecDeque<(std::path::PathBuf, Option<String>)>,
    pub pending_theme_transition_capture: Option<ThemeTransitionCapture>,
    pub theme_transition_started_at: Option<Instant>,
}

pub fn guess_icon_name(app_id: &str, title: &str) -> String {
    let app = app_id.to_lowercase();
    let tit = title.to_lowercase();

    if app.contains("nautilus") || app.contains("archivos") || tit.contains("archivos") || tit.contains("carpetas") {
        "org.gnome.Nautilus".to_string()
    } else if app.contains("ptyxis") || app.contains("terminal") || tit.contains("terminal") {
        "org.gnome.Ptyxis".to_string()
    } else if app.contains("calculator") || app.contains("calculadora") || tit.contains("calculadora") {
        "org.gnome.Calculator".to_string()
    } else if app.contains("texteditor") || app.contains("text-editor") || app.contains("gedit") || tit.contains("editor") {
        "org.gnome.TextEditor".to_string()
    } else if app.contains("control-center") || app.contains("settings") || app.contains("configuración") || app.contains("configuracion") {
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
        if let Some(mutex) = states.data_map.get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>() {
            let role = mutex.lock().unwrap();
            (role.app_id.clone(), role.title.clone())
        } else {
            (None, None)
        }
    })
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

/// Identifica si una ventana requiere desenfoque de fondo en GPU.
pub fn is_translucent_window(window: &smithay::desktop::Window) -> bool {
    let check = |app_id: &Option<String>, title: &Option<String>| -> bool {
        if let Some(ref id) = app_id {
            let lower = id.to_lowercase();
            if lower.contains("nautilus")
                || lower.contains("malus.settings")
                || lower.contains("malus-de-settings")
                || lower.contains("malus_de_settings")
            {
                return true;
            }
        }
        if let Some(ref title) = title {
            let lower = title.to_lowercase();
            if lower.contains("ajustes del sistema")
                || lower.contains("confirmación de acción del sistema")
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


fn is_system_monitor_identity(app_id: Option<&str>, title: Option<&str>) -> bool {
    let app_matches = app_id.is_some_and(|id| {
        let lower = id.to_lowercase();
        lower.contains("gnome-system-monitor")
            || lower.contains("org.gnome.systemmonitor")
            || lower.contains("systemmonitor")
    });
    let title_matches = title.is_some_and(|title| {
        let lower = title.to_lowercase();
        lower.contains("system monitor") || lower.contains("monitor del sistema")
    });
    app_matches || title_matches
}

/// Identifica el Monitor del Sistema para dibujarle un fondo opaco sin modificar otras ventanas.
pub fn is_system_monitor_window(window: &smithay::desktop::Window) -> bool {
    let matches = |surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface| {
        let (app_id, title) = toplevel_info(surface);
        is_system_monitor_identity(app_id.as_deref(), title.as_deref())
    };

    window.wl_surface().is_some_and(|surface| matches(surface.as_ref()))
        || window
            .toplevel()
            .is_some_and(|toplevel| matches(toplevel.wl_surface()))
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

fn read_system_dark_mode() -> Option<bool> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    let settings_path = config_home.join("glib-2.0/settings/keyfile");
    match std::fs::read_to_string(&settings_path) {
        Ok(settings) => parse_color_scheme(&settings),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            tracing::warn!("No se pudo leer el tema GTK para el fondo del Monitor del Sistema en {:?}: {error}", settings_path);
            None
        }
    }
}

/// Devuelve el fondo base de Adwaita para el modo de color GTK activo en la sesión.
pub fn system_monitor_backdrop_color() -> [f32; 3] {
    static THEME_CACHE: OnceLock<Mutex<Option<(Instant, bool)>>> = OnceLock::new();
    let cache = THEME_CACHE.get_or_init(|| Mutex::new(None));
    let mut cached_theme = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

    let should_refresh = cached_theme
        .as_ref()
        .is_none_or(|(last_refresh, _)| last_refresh.elapsed() >= Duration::from_millis(500));
    if should_refresh {
        let is_dark = read_system_dark_mode()
            .or_else(|| cached_theme.as_ref().map(|(_, is_dark)| *is_dark))
            .unwrap_or(true);
        *cached_theme = Some((Instant::now(), is_dark));
    }

    let is_dark = cached_theme.as_ref().is_some_and(|(_, is_dark)| *is_dark);
    if is_dark {
        [36.0 / 255.0, 36.0 / 255.0, 36.0 / 255.0]
    } else {
        [250.0 / 255.0, 250.0 / 255.0, 250.0 / 255.0]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapMode {
    LeftHalf,
    RightHalf,
    Maximize,
    CenterRestore,
}

impl MalusState {
    pub fn start_full_screenshot(&self) {
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
    pub fn new(display: &Display<MalusState>) -> Self {
        let dh = display.handle();
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state = smithay::wayland::shell::xdg::decoration::XdgDecorationState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, "malus-seat-0");
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let data_control_state = DataControlState::new::<Self, _>(&dh, None, |_| true);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);

        let ipc_broadcaster = IpcBroadcaster::new();
        if let Err(e) = ipc_broadcaster.start_server() {
            tracing::error!("Error al iniciar servidor IPC: {:?}", e);
        }
        let dock_blur_headroom = Self::load_dock_blur_headroom();

        Self {
            display_handle: dh,
            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            layer_shell_state,
            fractional_scale_manager_state,
            viewporter_state,
            shm_state,
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
            window_restore_geometries: HashMap::new(),
            centered_transient_windows: HashMap::new(),
            minimized_windows: HashMap::new(),
            minimized_window_thumbnails: HashMap::new(),
            window_ids: HashMap::new(),
            next_window_id: 0,
            dock_slots: HashMap::new(),
            active_animations: Vec::new(),
            snap_preview: None,
            socket_name: String::new(),
            is_running: true,
            ipc_broadcaster,
            app_switcher: AppSwitcherState::default(),
            cursor_status: smithay::input::pointer::CursorImageStatus::default_named(),
            pending_displays_reload: false,
            saved_app_window_sizes: Self::load_saved_window_sizes(),
            dock_blur_headroom,
            pending_screenshot_requests: std::collections::VecDeque::new(),
            pending_theme_transition_capture: None,
            theme_transition_started_at: None,
        }
    }

    fn load_dock_blur_headroom() -> i32 {
        let config = DockConfig::load_or_create_default();
        config
            .magnification_max_size
            .saturating_sub(config.icon_size)
            .min(i32::MAX as u32) as i32
    }

    /// Carga los tamaños de ventana guardados por aplicación desde ~/.config/malus-de/window_sizes.json
    pub fn load_saved_window_sizes() -> HashMap<String, Size<i32, Logical>> {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let path = std::path::PathBuf::from(home).join(".config/malus-de/window_sizes.json");
        if path.exists() {
            if let Ok(data) = std::fs::read_to_string(&path) {
                if let Ok(map) = serde_json::from_str::<HashMap<String, (i32, i32)>>(&data) {
                    tracing::info!("📂 Tamaños de ventana cargados desde disco: {} aplicaciones", map.len());
                    return map.into_iter().map(|(k, (w, h))| (k, Size::from((w, h)))).collect();
                }
            }
        }
        HashMap::new()
    }

    /// Guarda el último tamaño conocido de una aplicación y lo persiste en disco
    pub fn save_app_window_size(&mut self, app_id: &str, size: Size<i32, Logical>) {
        if size.w < 200 || size.h < 200 {
            return;
        }
        let clean_id = normalize_app_id(app_id);
        self.saved_app_window_sizes.insert(clean_id, size);

        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let config_dir = std::path::PathBuf::from(home).join(".config/malus-de");
        let _ = std::fs::create_dir_all(&config_dir);
        let path = config_dir.join("window_sizes.json");
        let map: HashMap<String, (i32, i32)> = self
            .saved_app_window_sizes
            .iter()
            .map(|(k, s)| (k.clone(), (s.w, s.h)))
            .collect();
        if let Ok(json) = serde_json::to_string_pretty(&map) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Obtiene el tamaño recordado de una aplicación si existe
    pub fn get_saved_window_size(&self, app_id: &str) -> Option<Size<i32, Logical>> {
        let clean_id = normalize_app_id(app_id);
        self.saved_app_window_sizes.get(&clean_id).copied()
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
            if let Some(&window_id) = self.window_ids.get(surface) {
                let (app_id, title) = toplevel_info(surface);
                let app_id_str = app_id.unwrap_or_else(|| "app".to_string());
                let title_str = title.unwrap_or_else(|| app_id_str.clone());
                let icon_name = guess_icon_name(&app_id_str, &title_str);
                list.push(malus_de_common::ipc::MinimizedWindowInfo {
                    window_id,
                    app_id: app_id_str,
                    title: title_str,
                    icon_name: Some(icon_name),
                    thumbnail_path: self.minimized_window_thumbnails.get(&window_id).cloned(),
                });
            }
        }
        self.ipc_broadcaster.broadcast(&IpcMessage::MinimizedWindowsChanged { windows: list });
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

    pub fn notify_running_apps(&self) {
        let mut apps = Vec::new();
        for window in self.space.elements() {
            if let Some(surface) = window.wl_surface() {
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
        self.ipc_broadcaster.broadcast(&IpcMessage::RunningAppsChanged { running_apps: apps });
    }

    pub fn notify_active_app(&self, surface: Option<&WlSurface>) {
        let (app_id, title) = if let Some(surf) = surface {
            toplevel_info(surf)
        } else {
            (None, None)
        };
        let window_id = surface.and_then(|surf| self.window_ids.get(surf).copied());
        self.ipc_broadcaster.broadcast(&IpcMessage::ActiveAppChanged { app_id, title, window_id });
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

    pub fn restore_window_by_id(&mut self, target_id: u64) -> bool {
        let entry = self.minimized_windows.iter().find(|(surf, _)| {
            self.window_ids.get(*surf) == Some(&target_id)
        }).map(|(surf, _)| surf.clone());

        if let Some(surf) = entry {
            if self.active_animations.iter().any(|a| a.surface == surf) {
                tracing::warn!("⚠️ Ventana id={} ya tiene una animación activa; ignorando restauración duplicada", target_id);
                return false;
            }

            if let Some((window, loc)) = self.minimized_windows.remove(&surf) {
                self.remove_minimized_thumbnail(target_id);
                let geo = window.geometry();
                let dock_slot = self.dock_slots.remove(&target_id).unwrap_or_else(|| {
                    if let Some((dock_geo, _)) = self.dock_surface_geometry() {
                        let slot_x = dock_geo.loc.x + (dock_geo.size.w / 2) + 20;
                        let slot_y = dock_geo.loc.y + (dock_geo.size.h / 2) - 22;
                        Rectangle::new((slot_x, slot_y).into(), (54, 44).into())
                    } else {
                        let output_size = self.space.outputs().next()
                            .and_then(|o| self.space.output_geometry(o))
                            .map(|g| g.size)
                            .unwrap_or_else(|| (1920, 1080).into());
                        let slot_x = (output_size.w / 2) + 80;
                        let slot_y = output_size.h - 58;
                        Rectangle::new((slot_x, slot_y).into(), (54, 44).into())
                    }
                });

                // Iniciar animación Genie Effect inversa (expansión desde el Dock)
                let anim = GenieAnimation {
                    window_id: target_id,
                    surface: surf.clone(),
                    window: window.clone(),
                    start_time: std::time::Instant::now(),
                    duration: std::time::Duration::from_millis(480),
                    is_minimizing: false,
                    waiting_for_dock_slot: false,
                    window_rect: Rectangle::new(loc, geo.size),
                    dock_slot_rect: dock_slot,
                    original_location: loc,
                };
                self.active_animations.push(anim);

                tracing::info!("✨ Iniciando animación de restauración Genie para ventana id={}", target_id);
                self.notify_running_apps();
                self.notify_minimized_windows();
                return true;
            }
        }
        false
    }

    pub fn minimize_toplevel(&mut self, surface: &ToplevelSurface) {
        tracing::info!("🪟 [XdgShell] minimize_request recibido del cliente");
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
        let target_window = self.space.elements().find(|w| w.wl_surface().as_deref() == Some(&surf)).cloned();
        if let Some(window) = target_window {
            let loc = self.space.element_location(&window).unwrap_or_default();
            let geo = window.geometry();
            let window_id = self.get_or_assign_window_id(&surf);

            // Determinar slot de destino en el Dock
            let registered_dock_slot = self.dock_slots.get(&window_id).cloned();
            let waiting_for_dock_slot =
                registered_dock_slot.is_none() && self.dock_surface_geometry().is_some();
            let dock_slot = registered_dock_slot.unwrap_or_else(|| {
                if let Some((dock_geo, _)) = self.dock_surface_geometry() {
                    let slot_x = dock_geo.loc.x + (dock_geo.size.w / 2) + 20;
                    let slot_y = dock_geo.loc.y + (dock_geo.size.h / 2) - 22;
                    Rectangle::new((slot_x, slot_y).into(), (54, 44).into())
                } else {
                    let output_size = self.space.outputs().next()
                        .and_then(|o| self.space.output_geometry(o))
                        .map(|g| g.size)
                        .unwrap_or_else(|| (1920, 1080).into());
                    let slot_x = (output_size.w / 2) + 90;
                    let slot_y = output_size.h - 58;
                    Rectangle::new((slot_x, slot_y).into(), (54, 44).into())
                }
            });

            // Registrar animación Genie Effect (Lámpara de Aladino hacia el Dock)
            let anim = GenieAnimation {
                window_id,
                surface: surf.clone(),
                window: window.clone(),
                start_time: std::time::Instant::now(),
                duration: std::time::Duration::from_millis(480),
                is_minimizing: true,
                waiting_for_dock_slot,
                window_rect: Rectangle::new(loc, geo.size),
                dock_slot_rect: dock_slot,
                original_location: loc,
            };
            self.active_animations.push(anim);

            self.minimized_windows.insert(surf.clone(), (window.clone(), loc));
            if !waiting_for_dock_slot {
                self.space.unmap_elem(&window);
            }

            let next_focus = self
                .space
                .elements()
                .filter(|candidate| candidate.wl_surface().as_deref() != Some(&surf))
                .last()
                .and_then(|w| w.wl_surface().map(|s| s.into_owned()));
            if let Some(kb) = self.seat.get_keyboard() {
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                kb.set_focus(self, next_focus.clone(), serial);
            }
            if let Some(next_win) = self.space.elements().last() {
                if let Some(toplevel) = next_win.toplevel() {
                    toplevel.send_configure();
                }
            }
            self.notify_running_apps();
            self.notify_minimized_windows();
            self.notify_active_app(next_focus.as_ref());
        }
    }

    pub fn activate_or_restore_app(&mut self, target: &str) -> bool {
        let target_lower = target.to_lowercase();

        // 1. Verificar si está entre las ventanas minimizadas
        let minimized_entry = self.minimized_windows.iter().find(|(surf, _)| {
            let (app_id, title) = toplevel_info(surf);
            let id_match = app_id.map(|id| id.to_lowercase().contains(&target_lower)).unwrap_or(false);
            let title_match = title.map(|t| t.to_lowercase().contains(&target_lower)).unwrap_or(false);
            id_match || title_match
        }).map(|(surf, _)| surf.clone());

        if let Some(surf) = minimized_entry {
            if let Some(&id) = self.window_ids.get(&surf) {
                return self.restore_window_by_id(id);
            }
        }

        // 2. Verificar si está entre las ventanas activas en Space
        let visible_window = self.space.elements().find(|w| {
            if let Some(surf) = w.wl_surface() {
                let (app_id, title) = toplevel_info(&surf);
                let id_match = app_id.map(|id| id.to_lowercase().contains(&target_lower)).unwrap_or(false);
                let title_match = title.map(|t| t.to_lowercase().contains(&target_lower)).unwrap_or(false);
                id_match || title_match
            } else {
                false
            }
        }).cloned();

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

            if is_already_focused {
                tracing::debug!("La app solicitada ya tiene el foco; no se minimiza ni se altera");
                return true;
            }

            self.space.raise_element(&window, true);
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_configure();
            }
            if let Some(surf) = window.wl_surface() {
                if let Some(kb) = self.seat.get_keyboard() {
                    let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                    kb.set_focus(self, Some(surf.clone().into_owned()), serial);
                }
                self.notify_active_app(Some(&surf));
            }
            return true;
        }

        false
    }

    pub fn process_ipc_messages(&mut self) {
        while let Some(msg) = self.ipc_broadcaster.try_recv() {
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
                IpcMessage::CaptureScreenshotForOutput {
                    destination,
                    output_name,
                } => {
                    tracing::info!("Solicitud IPC de captura recibida para {output_name}");
                    self.pending_screenshot_requests
                        .push_back((std::path::PathBuf::from(destination), Some(output_name)));
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
                IpcMessage::ThemeChanged { theme_name, is_dark } => {
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
                IpcMessage::AnimateRestore { window_id } => {
                    tracing::info!("📥 IPC AnimateRestore recibido para ventana id={}", window_id);
                    self.restore_window_by_id(window_id);
                }
                IpcMessage::RegisterDockSlot { window_id, x, y, width, height } => {
                    tracing::info!("📥 IPC RegisterDockSlot recibido: id={}, rect=({}, {}, {}, {})", window_id, x, y, width, height);
                    let dock_origin = self
                        .dock_surface_geometry()
                        .map(|(dock_geo, _)| dock_geo.loc)
                        .unwrap_or_default();
                    let final_rect = Rectangle::new(
                        (dock_origin.x + x, dock_origin.y + y).into(),
                        (width as i32, height as i32).into(),
                    );

                    tracing::info!("🎯 Dock slot ajustado con precisión: id={}, rect={:?}", window_id, final_rect);
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
                    crate::backend::winit::toggle_spotlight(&self.socket_name);
                }
                IpcMessage::HideSpotlight => {
                    self.ipc_broadcaster.broadcast(&IpcMessage::HideSpotlight);
                }
                IpcMessage::HideActiveWindow => {
                    tracing::info!("📥 IPC HideActiveWindow recibido");
                    self.minimize_active_window();
                }
                IpcMessage::CloseActiveWindow => {
                    tracing::info!("📥 IPC CloseActiveWindow recibido");
                    self.close_active_window();
                }
                IpcMessage::ReloadDisplaysConfig => {
                    tracing::info!("📥 IPC ReloadDisplaysConfig recibido — marcando recarga inmediata de pantallas");
                    self.pending_displays_reload = true;
                    self.ipc_broadcaster.broadcast(&IpcMessage::ReloadDisplaysConfig);
                }
                IpcMessage::ThemeTransitionPrepared { .. } => {}
                IpcMessage::SetKeyboardLayout { layout, variant } => {
                    tracing::info!("📥 IPC SetKeyboardLayout recibido: layout='{}', variant='{}'", layout, variant);
                    self.apply_keyboard_layout(&layout, &variant);
                }
                IpcMessage::SwitchToWindow { window_id } => {
                    tracing::info!("📥 IPC SwitchToWindow recibido para: {}", window_id);
                    self.app_switcher.is_active = false;
                    self.ipc_broadcaster.broadcast(&IpcMessage::HideAppSwitcher);
                    self.switch_to_window(window_id);
                }
                IpcMessage::HideAppSwitcher => {
                    tracing::info!("📥 IPC HideAppSwitcher recibido");
                    self.app_switcher.is_active = false;
                    self.ipc_broadcaster.broadcast(&IpcMessage::HideAppSwitcher);
                }
                IpcMessage::ReloadDockConfig => {
                    tracing::info!("📥 IPC ReloadDockConfig recibido — retransmitiendo a clientes shell");
                    self.dock_blur_headroom = Self::load_dock_blur_headroom();
                    self.ipc_broadcaster.broadcast(&IpcMessage::ReloadDockConfig);
                }
                _ => {}
            }
        }
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

        self.ipc_broadcaster.broadcast(&IpcMessage::KeyboardLayoutChanged {
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

    pub fn close_active_window(&mut self) {
        if let Some(win) = self.get_active_window() {
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

    pub fn dismiss_all_popups(&mut self) {
        if self.active_popups.is_empty() {
            return;
        }
        tracing::info!("🧹 Descartando {} popups activos", self.active_popups.len());
        for popup in self.active_popups.drain(..) {
            let _ = popup.send_popup_done();
            if let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) {
                let _ = PopupManager::dismiss_popup(&root, &PopupKind::Xdg(popup));
            }
        }
        self.popups.cleanup();
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
            let (surface, surface_location) = window.surface_under(
                point - location.to_f64(),
                smithay::desktop::WindowSurfaceType::POPUP,
            )?;
            Some((window.clone(), location, surface, surface_location))
        })
    }

    /// Construye la lista de elementos para el selector visual de aplicaciones (⌘Tab)
    pub fn build_app_switcher_items(&mut self) -> Vec<AppSwitcherItem> {
        let mut items = Vec::new();
        // 1. Ventanas activas en space, ordenadas de más reciente a más antigua
        let windows: Vec<_> = self.space.elements().cloned().collect();
        for win in windows.into_iter().rev() {
            if let Some(surf) = win.wl_surface() {
                let window_id = self.get_or_assign_window_id(&surf);
                let (app_id_opt, title_opt) = toplevel_info(&surf);
                let app_id = app_id_opt.unwrap_or_else(|| "application-x-executable".to_string());
                let title = title_opt.unwrap_or_else(|| app_id.clone());
                let icon_name = guess_icon_name(&app_id, &title);
                items.push(AppSwitcherItem {
                    window_id,
                    app_id,
                    title,
                    icon_name: Some(icon_name),
                    is_minimized: false,
                });
            }
        }
        // 2. Ventanas minimizadas
        let min_surfaces: Vec<_> = self.minimized_windows.keys().cloned().collect();
        for surf in min_surfaces {
            let window_id = self.get_or_assign_window_id(&surf);
            let (app_id_opt, title_opt) = toplevel_info(&surf);
            let app_id = app_id_opt.unwrap_or_else(|| "application-x-executable".to_string());
            let title = title_opt.unwrap_or_else(|| app_id.clone());
            let icon_name = guess_icon_name(&app_id, &title);
            items.push(AppSwitcherItem {
                window_id,
                app_id,
                title,
                icon_name: Some(icon_name),
                is_minimized: true,
            });
        }
        items
    }

    /// Conmuta el foco a la ventana especificada por su window_id (restaurando si está minimizada)
    pub fn switch_to_window(&mut self, window_id: u64) {
        tracing::info!("🔀 AppSwitcher: cambiando foco a ventana id={}", window_id);
        // Si la ventana está minimizada, restaurar con Genie Effect
        if self.minimized_windows.iter().any(|(s, _)| self.window_ids.get(s) == Some(&window_id)) {
            self.restore_window_by_id(window_id);
            return;
        }

        // Si está en space, traer al frente y enfocar
        let target = self.space.elements().find(|w| {
            w.wl_surface().as_ref().and_then(|s| self.window_ids.get(s.as_ref())) == Some(&window_id)
        }).cloned();

        if let Some(win) = target {
            self.space.raise_element(&win, true);
            if let Some(toplevel) = win.toplevel() {
                toplevel.send_configure();
            }
            if let Some(surf) = win.wl_surface() {
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                if let Some(kb) = self.seat.get_keyboard() {
                    kb.set_focus(self, Some(surf.clone().into_owned()), serial);
                }
                self.notify_active_app(Some(&surf));
            }
        }
    }

    /// Cierra la ventana especificada por su window_id
    pub fn close_window_by_id(&mut self, window_id: u64) {
        tracing::info!("🚪 AppSwitcher: cerrando ventana id={}", window_id);
        let target = self.space.elements().find(|w| {
            w.wl_surface().as_ref().and_then(|s| self.window_ids.get(s.as_ref())) == Some(&window_id)
        }).cloned();

        if let Some(win) = target {
            if let Some(toplevel) = win.toplevel() {
                toplevel.send_close();
            }
        } else if let Some(surf) = self.minimized_windows.keys().find(|s| self.window_ids.get(*s) == Some(&window_id)).cloned() {
            if let Some((win, _)) = self.minimized_windows.remove(&surf) {
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
        let target = self.space.elements().find(|w| {
            w.wl_surface().as_ref().and_then(|s| self.window_ids.get(s.as_ref())) == Some(&window_id)
        }).cloned();

        if let Some(win) = target {
            if let Some(toplevel) = win.toplevel().cloned() {
                self.minimize_toplevel(&toplevel);
            }
        }
    }
}

/// Aplica un modo de Aero Snap / Tiling a una ventana de manera atómica
pub fn snap_window(
    state: &mut MalusState,
    window: &smithay::desktop::Window,
    mode: SnapMode,
    usable_zone: Rectangle<i32, Logical>,
) {
    let Some(surface) = window.wl_surface() else { return };
    let surface = surface.into_owned();

    let cur_loc = state.space.element_location(window).unwrap_or_default();
    let cur_geo = window.geometry();

    let is_currently_snapped = window.toplevel().map(|t| {
        let states = &t.current_state().states;
        states.contains(XdgState::Maximized)
            || states.contains(XdgState::TiledLeft)
            || states.contains(XdgState::TiledRight)
            || states.contains(XdgState::Fullscreen)
    }).unwrap_or(false);

    // Guardar geometría previa flotante SOLO si la ventana no estaba ya snapped y su tamaño actual es válido
    if mode != SnapMode::CenterRestore && !is_currently_snapped && cur_geo.size.w > 150 && cur_geo.size.h > 150 {
        state.window_restore_geometries.insert(surface.clone(), (cur_loc, cur_geo.size));
    }

    let (target_loc, target_size, xdg_states) = match mode {
        SnapMode::LeftHalf => (
            Point::from((usable_zone.loc.x, usable_zone.loc.y)),
            Some(Size::from((usable_zone.size.w / 2, usable_zone.size.h))),
            vec![XdgState::TiledLeft],
        ),
        SnapMode::RightHalf => (
            Point::from((usable_zone.loc.x + usable_zone.size.w / 2, usable_zone.loc.y)),
            Some(Size::from((usable_zone.size.w - usable_zone.size.w / 2, usable_zone.size.h))),
            vec![XdgState::TiledRight],
        ),
        SnapMode::Maximize => (
            Point::from((usable_zone.loc.x, usable_zone.loc.y)),
            Some(Size::from((usable_zone.size.w, usable_zone.size.h))),
            vec![XdgState::Maximized],
        ),
        SnapMode::CenterRestore => {
            if let Some((saved_loc, saved_size)) = state.window_restore_geometries.get(&surface).copied() {
                let size_opt = if saved_size.w > 100 && saved_size.h > 100 {
                    Some(saved_size)
                } else {
                    None
                };
                (saved_loc, size_opt, vec![])
            } else {
                let default_loc = Point::from((usable_zone.loc.x + 80, usable_zone.loc.y + 80));
                (default_loc, None, vec![])
            }
        }
    };

    tracing::info!(
        "🪟 Aero Snap aplicado: modo={:?}, loc={:?}, size={:?}",
        mode,
        target_loc,
        target_size
    );

    if let Some(toplevel) = window.toplevel() {
        toplevel.with_pending_state(|top_state| {
            top_state.size = target_size;
            top_state.states.unset(XdgState::Maximized);
            top_state.states.unset(XdgState::TiledLeft);
            top_state.states.unset(XdgState::TiledRight);
            for s in xdg_states {
                top_state.states.set(s);
            }
        });
        toplevel.send_configure();
    }

    state.space.map_element(window.clone(), target_loc, true);
    state.space.raise_element(window, true);
}

// Delegaciones de protocolos de Smithay
delegate_compositor!(MalusState);
delegate_shm!(MalusState);
delegate_seat!(MalusState);
delegate_xdg_shell!(MalusState);
delegate_layer_shell!(MalusState);
delegate_output!(MalusState);
delegate_data_device!(MalusState);
delegate_primary_selection!(MalusState);
delegate_data_control!(MalusState);
delegate_fractional_scale!(MalusState);
delegate_viewporter!(MalusState);
delegate_xdg_decoration!(MalusState);

impl smithay::wayland::shell::xdg::decoration::XdgDecorationHandler for MalusState {
    fn new_decoration(&mut self, toplevel: smithay::wayland::shell::xdg::ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode::ClientSide);
        });
        toplevel.send_configure();
    }
    fn request_mode(
        &mut self,
        toplevel: smithay::wayland::shell::xdg::ToplevelSurface,
        mode: smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    ) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(mode);
        });
        toplevel.send_configure();
    }
    fn unset_mode(&mut self, toplevel: smithay::wayland::shell::xdg::ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode::ClientSide);
        });
        toplevel.send_configure();
    }
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
    /// Determina la escala fraccionaria preferida para una superficie según el monitor en el que reside
    pub fn preferred_scale_for_surface(&self, surface: &WlSurface) -> f64 {
        if let Some(window) = self.window_for_surface(surface) {
            if let Some(win_loc) = self.space.element_location(&window) {
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
            if let Some(output) = self.space.outputs_for_element(&window).first() {
                return output.current_scale().fractional_scale();
            }
        }

        for output in self.space.outputs() {
            let map = smithay::desktop::layer_map_for_output(output);
            if map.layer_for_surface(surface, smithay::desktop::WindowSurfaceType::all()).is_some() {
                return output.current_scale().fractional_scale();
            }
        }

        if let Some(output) = self.space.outputs().next() {
            output.current_scale().fractional_scale()
        } else {
            1.0
        }
    }

    /// Calcula la zona utilizable global (en coordenadas de Space) para un monitor dado
    pub fn usable_zone_for_output(&self, output: &Output) -> Rectangle<i32, Logical> {
        let output_geo = self.space.output_geometry(output).unwrap_or_default();
        let layer_map = smithay::desktop::layer_map_for_output(output);
        let zone = layer_map.non_exclusive_zone();
        if zone.size.w > 0 && zone.size.h > 0 {
            let has_dock = layer_map
                .layers()
                .any(|layer| layer.namespace() == "malus-dock");
            let headroom = if has_dock {
                self.dock_blur_headroom.max(0)
            } else {
                0
            };
            let usable_height = (zone.size.h + headroom).min(output_geo.size.h - zone.loc.y);
            Rectangle::new(
                output_geo.loc + zone.loc,
                (zone.size.w, usable_height.max(zone.size.h)).into(),
            )
        } else {
            Rectangle::new(
                (output_geo.loc.x, output_geo.loc.y + 36).into(),
                (output_geo.size.w, (output_geo.size.h - 36).max(100)).into(),
            )
        }
    }
}

impl CompositorHandler for MalusState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        smithay::backend::renderer::utils::on_commit_buffer_handler::<Self>(surface);
        self.popups.commit(surface);

        // Sincronizar escala fraccionaria preferida según el monitor físico en el que reside la superficie
        let preferred_scale = self.preferred_scale_for_surface(surface);
        smithay::wayland::compositor::with_states(surface, |states| {
            smithay::wayland::fractional_scale::with_fractional_scale(states, |fractional_scale| {
                fractional_scale.set_preferred_scale(preferred_scale);
            });
        });

        // Si es una ventana regular en Space, actualizamos sus dimensiones internas
        for window in self.space.elements() {
            window.on_commit();
        }

        if let Some(window) = self.window_for_surface(surface) {
            if let Some(toplevel) = window.toplevel() {
                let is_system_action_confirmation = toplevel.parent().is_some()
                    || toplevel_info(surface).1.as_deref()
                        == Some("Confirmación de acción del sistema");
                let geometry = window.geometry();
                if is_system_action_confirmation
                    && self.centered_transient_windows.get(surface)
                        != Some(&(geometry.loc, geometry.size))
                {
                    if geometry.size.w > 1 && geometry.size.h > 1 {
                        let parent_output = toplevel
                            .parent()
                            .and_then(|parent| {
                                let parent_surface = parent.wl_surface();
                                parent_surface
                                    .as_deref()
                                    .and_then(|surface| self.window_for_surface(surface))
                            })
                            .and_then(|parent| {
                                let location = self.space.element_location(&parent)?;
                                let parent_geometry = parent.geometry();
                                let center = Point::<i32, Logical>::from((
                                    location.x + parent_geometry.loc.x + parent_geometry.size.w / 2,
                                    location.y + parent_geometry.loc.y + parent_geometry.size.h / 2,
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
                        let output = parent_output.or_else(|| {
                            self.space.outputs().find(|output| {
                                self.space
                                    .output_geometry(output)
                                    .map(|output_geometry| output_geometry.contains(current_center))
                                    .unwrap_or(false)
                            })
                        });

                        if let Some(output) = output {
                            if let Some(output_geometry) = self.space.output_geometry(output) {
                                let centered_location = Point::<i32, Logical>::from((
                                    output_geometry.loc.x
                                        + (output_geometry.size.w - geometry.size.w) / 2
                                        - geometry.loc.x,
                                    output_geometry.loc.y
                                        + (output_geometry.size.h - geometry.size.h) / 2
                                        - geometry.loc.y,
                                ));
                                self.space
                                    .map_element(window.clone(), centered_location, true);
                                self.space.raise_element(&window, true);
                                self.centered_transient_windows
                                    .insert(surface.clone(), (geometry.loc, geometry.size));
                            }
                        }
                    }
                }
            }

            let is_currently_snapped = window.toplevel().map(|t| {
                let states = &t.current_state().states;
                states.contains(XdgState::Maximized)
                    || states.contains(XdgState::TiledLeft)
                    || states.contains(XdgState::TiledRight)
                    || states.contains(XdgState::Fullscreen)
            }).unwrap_or(false);

            if !is_currently_snapped {
                let geo = window.geometry();
                let loc = self.space.element_location(&window).unwrap_or_default();
                if geo.size.w > 150 && geo.size.h > 150 {
                    self.window_restore_geometries.insert(surface.clone(), (loc, geo.size));
                }
            }
        }

        // Si la superficie pertenece a una Layer Surface (Top Bar, Dock, Notificaciones, etc.)
        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        let target_output = outputs.into_iter().find(|o| {
            let map = smithay::desktop::layer_map_for_output(o);
            map.layer_for_surface(surface, smithay::desktop::WindowSurfaceType::all()).is_some()
        });

        if let Some(output) = target_output {
            let mut should_focus_overlay = None;
            {
                let mut map = smithay::desktop::layer_map_for_output(&output);
                map.arrange();

                if let Some(layer) = map.layer_for_surface(surface, smithay::desktop::WindowSurfaceType::TOPLEVEL) {
                    let initial_configure_sent = smithay::wayland::compositor::with_states(layer.wl_surface(), |states| {
                        states
                            .data_map
                            .get::<smithay::wayland::shell::wlr_layer::LayerSurfaceData>()
                            .map(|d| d.lock().unwrap().initial_configure_sent)
                            .unwrap_or(false)
                    });
                    if !initial_configure_sent {
                        tracing::info!("📤 Enviando initial configure a layer surface");
                        layer.layer_surface().send_configure();
                    }
                    if layer.layer() == smithay::wayland::shell::wlr_layer::Layer::Overlay && layer.can_receive_keyboard_focus() {
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

impl SeatHandler for MalusState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(smithay::reexports::wayland_server::Resource::id(s)).ok());
        let focused_pid = client.as_ref().and_then(|client| {
            let credentials = client.get_credentials(dh).ok()?;
            (credentials.uid == rustix::process::getuid().as_raw()).then_some(credentials.pid)
        });
        self.focus_cpu_priority.set_focused_pid(focused_pid);
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);

        if focused.is_none() {
            self.cursor_status = smithay::input::pointer::CursorImageStatus::default_named();
        }

        // Actualizar el estado Activated de todas las ventanas para sincronizar clientes Wayland (Brave, Chrome, Electron, etc.)
        for win in self.space.elements() {
            let is_focused = focused.map(|s| win.wl_surface().as_deref() == Some(s)).unwrap_or(false);
            win.set_activated(is_focused);
            if let Some(toplevel) = win.toplevel() {
                toplevel.with_pending_state(|state| {
                    if is_focused {
                        state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated);
                    } else {
                        state.states.unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated);
                    }
                });
                toplevel.send_configure();
            }
        }
    }
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: smithay::input::pointer::CursorImageStatus) {
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

impl ClientDndGrabHandler for MalusState {}

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
            .or_else(|| self.space.outputs().next().cloned());

        let (spawn_loc, usable_zone) = if let Some(ref output) = target_output {
            output.enter(surface.wl_surface());
            let scale = output.current_scale().fractional_scale();
            smithay::wayland::compositor::with_states(surface.wl_surface(), |states| {
                with_fractional_scale(states, |fractional_scale| {
                    fractional_scale.set_preferred_scale(scale);
                });
            });
            let geo = self.space.output_geometry(output).unwrap_or_default();
            let usable = self.usable_zone_for_output(output);
            let offset = ((self.space.elements().count() % 6) as i32) * 28;
            ((geo.loc.x + 80 + offset, geo.loc.y + 60 + offset), usable)
        } else {
            ((80, 60), Rectangle::new((0, 0).into(), (1920, 1080).into()))
        };

        let window = smithay::desktop::Window::new_wayland_window(surface.clone());
        window.set_activated(true);

        // Si tenemos un tamaño recordado para esta aplicación, sugerirlo en el configure inicial
        let initial_size = if surface.parent().is_none() {
            let app_id = app_id_for_surface(surface.wl_surface());
            app_id.as_deref().and_then(|id| self.get_saved_window_size(id)).map(|saved| {
                let max_w = (usable_zone.size.w - 80).max(400);
                let max_h = (usable_zone.size.h - 80).max(300);
                Size::from((saved.w.min(max_w).max(320), saved.h.min(max_h).max(240)))
            })
        } else {
            None
        };

        surface.with_pending_state(|state| {
            state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated);
            if let Some(size) = initial_size {
                tracing::info!("📐 [State] Aplicando tamaño de ventana recordado a nueva ventana: {:?}", size);
                state.size = Some(size);
            }
        });
        surface.send_configure();
        self.space.map_element(window, spawn_loc, true);
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
        if !self.active_popups.iter().any(|p| p.wl_surface() == surface.wl_surface()) {
            self.active_popups.push(surface);
        }
    }

    fn grab(&mut self, surface: PopupSurface, _seat: WlSeat, _serial: Serial) {
        tracing::info!("🎯 [XdgShell] grab request recibido para popup surface: {:?}", smithay::reexports::wayland_server::Resource::id(surface.wl_surface()));
        if !self.active_popups.iter().any(|p| p.wl_surface() == surface.wl_surface()) {
            self.active_popups.push(surface);
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
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: WlSeat, serial: Serial) {
        tracing::info!("🖱️ [XdgShell] move_request recibido del cliente: serial={:?}", serial);
        let seat: Seat<Self> = Seat::from_resource(&seat).unwrap();
        let pointer = seat.get_pointer().unwrap();

        let Some(start_data) = pointer.grab_start_data() else {
            tracing::warn!("⚠️ [XdgShell] move_request ignorado: no hay clic activo de ratón (evitando bloqueo del puntero)");
            return;
        };

        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            tracing::warn!("⚠️ [XdgShell] move_request: no se encontró ventana para la superficie");
            return;
        };

        let is_snapped = window.toplevel().map(|t| {
            let states = &t.current_state().states;
            states.contains(XdgState::Maximized)
                || states.contains(XdgState::TiledLeft)
                || states.contains(XdgState::TiledRight)
        }).unwrap_or(false);

        let initial_window_location = if is_snapped {
            let saved = self.window_restore_geometries.get(surface.wl_surface()).copied();
            let restore_size = saved.map(|(_, s)| s).filter(|s| s.w > 100 && s.h > 100);

            if let Some(toplevel) = window.toplevel() {
                toplevel.with_pending_state(|top_state| {
                    top_state.size = restore_size;
                    top_state.states.unset(XdgState::Maximized);
                    top_state.states.unset(XdgState::TiledLeft);
                    top_state.states.unset(XdgState::TiledRight);
                });
                toplevel.send_configure();
            }

            let float_w = restore_size.map(|s| s.w).unwrap_or(400);
            let cursor_x = start_data.location.x as i32;
            let cursor_y = start_data.location.y as i32;
            let new_x = (cursor_x - float_w / 2).max(0);
            let new_loc = Point::from((new_x, (cursor_y - 15).max(32)));
            self.space.map_element(window.clone(), new_loc, true);
            new_loc
        } else {
            self.space.element_location(&window).unwrap_or_default()
        };

        let grab = crate::grabs::MoveSurfaceGrab {
            start_data,
            window: window.clone(),
            initial_window_location,
        };

        self.space.raise_element(&window, true);
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
        tracing::info!("✅ [XdgShell] MoveSurfaceGrab activado exitosamente para arrastre por título!");
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: WlSeat,
        serial: Serial,
        edges: ResizeEdge,
    ) {
        tracing::info!("📐 [XdgShell] resize_request recibido del cliente: edges={:?}", edges);
        let Some(seat) = Seat::from_resource(&seat) else { return };
        let Some(pointer) = seat.get_pointer() else { return };

        if !pointer.has_grab(serial) {
            tracing::warn!("⚠️ resize_request rechazado: el puntero no tiene un grab con este serial");
            return;
        }

        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
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
        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
        tracing::info!("✅ [XdgShell] ResizeSurfaceGrab activado exitosamente para redimensión de ventana!");
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
        let _ = surface.send_repositioned(token);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        tracing::info!("🪟 [XdgShell] maximize_request recibido del cliente");
        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
        let output = self
            .space
            .outputs_for_element(&window)
            .into_iter()
            .next()
            .or_else(|| self.space.outputs().next().cloned());

        if let Some(output) = output {
            let usable_zone = self.usable_zone_for_output(&output);
            snap_window(self, &window, SnapMode::Maximize, usable_zone);
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        tracing::info!("🪟 [XdgShell] unmaximize_request recibido del cliente");
        let surf = surface.wl_surface();
        let Some(window) = self.window_for_surface(surf) else { return };
        let output = self
            .space
            .outputs_for_element(&window)
            .into_iter()
            .next()
            .or_else(|| self.space.outputs().next().cloned());

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
        let output = requested_output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.space.outputs_for_element(&window).into_iter().next())
            .or_else(|| self.space.outputs().next().cloned());
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
        if !states.contains(XdgState::Fullscreen)
            && !states.contains(XdgState::Maximized)
            && !states.contains(XdgState::TiledLeft)
            && !states.contains(XdgState::TiledRight)
        {
            if let Some(surface) = window.wl_surface() {
                let geometry = window.geometry();
                if geometry.size.w > 150 && geometry.size.h > 150 {
                    self.window_restore_geometries.insert(
                        surface.into_owned(),
                        (
                            self.space.element_location(&window).unwrap_or_default(),
                            geometry.size,
                        ),
                    );
                }
            }
        }

        toplevel.with_pending_state(|state| {
            state.size = Some(output_geometry.size);
            state.states.unset(XdgState::Maximized);
            state.states.unset(XdgState::TiledLeft);
            state.states.unset(XdgState::TiledRight);
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
            .space
            .outputs_for_element(&window)
            .into_iter()
            .next()
            .or_else(|| self.space.outputs().next().cloned());
        let restore = self.window_restore_geometries.get(surf).copied();
        let (location, size) = if let Some((location, size)) = restore {
            (location, Some(size))
        } else if let Some(output) = output {
            let output_geometry = self.space.output_geometry(&output).unwrap_or_default();
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
        let surf = surface.wl_surface();
        self.centered_transient_windows.remove(surf);

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
                    || states.contains(XdgState::Fullscreen);

                if !is_snapped {
                    if let Some(ref win) = window {
                        let geo = win.geometry();
                        if geo.size.w >= 200 && geo.size.h >= 200 {
                            tracing::info!("💾 Recordando tamaño de ventana para {}: {:?}", id, geo.size);
                            self.save_app_window_size(&id, geo.size);
                        }
                    }
                } else if let Some((_, saved_size)) = self.window_restore_geometries.get(surf).copied() {
                    if saved_size.w >= 200 && saved_size.h >= 200 {
                        tracing::info!("💾 Recordando tamaño previo no maximizado para {}: {:?}", id, saved_size);
                        self.save_app_window_size(&id, saved_size);
                    }
                }
            }
        }

        self.window_restore_geometries.remove(surf);
        self.minimized_windows.remove(surf);
        if let Some(window_id) = self.window_ids.remove(surf) {
            self.dock_slots.remove(&window_id);
            self.remove_minimized_thumbnail(window_id);
        }
        self.active_animations.retain(|a| &a.surface != surf);
        self.notify_minimized_windows();

        if let Some(window) = window {
            self.space.unmap_elem(&window);
            tracing::info!("🧹 Ventana desmapeada de Space tras destruirse");
        } else {
            tracing::warn!("⚠️ No se encontró la ventana en Space al destruir toplevel");
        }

        self.space.refresh();

        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        if let Some(kb) = self.seat.get_keyboard() {
            let next_win = self.space.elements().rev().next().cloned();
            let next_surface = next_win.as_ref().and_then(|w| w.wl_surface().map(|s| s.into_owned()));
            if let Some(ref win) = next_win {
                if let Some(toplevel) = win.toplevel() {
                    toplevel.send_configure();
                }
            }
            kb.set_focus(self, next_surface, serial);
        }

        if let Some(pointer) = self.seat.get_pointer() {
            if pointer.is_grabbed() {
                pointer.unset_grab(self, serial, 0);
            }
            let loc = pointer.current_location();
            let new_target = self.space.element_under(loc).and_then(|(win, win_loc)| {
                let local_point = loc - win_loc.to_f64();
                win.surface_under(local_point, smithay::desktop::WindowSurfaceType::all())
                    .map(|(s, sub_loc)| (s, (win_loc + sub_loc).to_f64()))
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
                    let cur_geo = self.window_for_surface(surface.wl_surface()).map(|w| w.geometry());
                    let is_unconfigured = cur_geo.map(|g| g.size.w <= 200 || g.size.h <= 200).unwrap_or(true);
                    if is_unconfigured {
                        tracing::info!("📐 [app_id_changed] Aplicando tamaño recordado para {}: {:?}", id, saved);
                        surface.with_pending_state(|state| {
                            state.size = Some(saved);
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
        if namespace == "malus-top-bar" {
            surface.with_pending_state(|state| {
                state.size = Some((0, 36).into());
            });
        }
        let output = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.space.outputs().next().cloned());

        if let Some(output) = output {
            output.enter(surface.wl_surface());
            let scale = output.current_scale().fractional_scale();
            smithay::wayland::compositor::with_states(surface.wl_surface(), |states| {
                with_fractional_scale(states, |fractional_scale| {
                    fractional_scale.set_preferred_scale(scale);
                });
            });
            let desktop_surface = smithay::desktop::LayerSurface::new(surface, namespace);
            let mut map = layer_map_for_output(&output);
            let _ = map.map_layer(&desktop_surface);
            map.arrange();
        }
    }

    fn new_popup(&mut self, _parent: LayerSurface, popup: smithay::wayland::shell::xdg::PopupSurface) {
        tracing::info!("✨ [LayerShell] Nuevo Popup para Layer Surface recibido; registrando en PopupManager");
        if self.popups.find_popup(popup.wl_surface()).is_none() {
            if let Err(err) = self.popups.track_popup(PopupKind::Xdg(popup.clone())) {
                tracing::warn!("⚠️ Error al registrar layer popup en PopupManager: {:?}", err);
            }
        }
        if let Err(err) = popup.send_configure() {
            tracing::warn!("⚠️ Error al enviar configure a layer popup: {:?}", err);
        }
        if !self.active_popups.iter().any(|p| p.wl_surface() == popup.wl_surface()) {
            self.active_popups.push(popup);
        }
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        tracing::info!("🗑️ [LayerShell] Layer surface destruida");
        for output in self.space.outputs() {
            let mut map = layer_map_for_output(output);
            let layer = map
                .layers()
                .find(|&layer| layer.layer_surface() == &surface)
                .cloned();
            if let Some(layer) = layer {
                map.unmap_layer(&layer);
                tracing::info!("🧹 Layer surface desmapeada de LayerMap: namespace={}", layer.namespace());
            }
            map.cleanup();
        }

        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        if let Some(kb) = self.seat.get_keyboard() {
            let top_win_surface = self.space.elements().next().and_then(|w| w.wl_surface().map(|s| s.into_owned()));
            kb.set_focus(self, top_win_surface, serial);
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


#[cfg(test)]
mod tests {
    use super::{is_system_monitor_identity, parse_color_scheme};

    #[test]
    fn recognizes_system_monitor_app_ids_and_localized_titles() {
        assert!(is_system_monitor_identity(Some("org.gnome.SystemMonitor"), None));
        assert!(is_system_monitor_identity(Some("gnome-system-monitor"), None));
        assert!(is_system_monitor_identity(None, Some("Monitor del sistema")));
        assert!(is_system_monitor_identity(None, Some("System Monitor")));
        assert!(!is_system_monitor_identity(Some("org.gnome.Nautilus"), Some("Archivos")));
    }

    #[test]
    fn reads_dark_and_light_color_scheme_from_session_keyfile() {
        assert_eq!(parse_color_scheme("[org/gnome/desktop/interface]\ncolor-scheme='prefer-dark'\n"), Some(true));
        assert_eq!(parse_color_scheme("[org/gnome/desktop/interface]\ncolor-scheme='prefer-light'\n"), Some(false));
        assert_eq!(parse_color_scheme("[org/gnome/desktop/interface]\ncolor-scheme='default'\n"), None);
        assert_eq!(parse_color_scheme("[org/gnome/desktop/wm/preferences]\ncolor-scheme='prefer-light'\n"), None);
    }
}
