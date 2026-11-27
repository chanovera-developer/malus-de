use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

/// Tamaño máximo permitido por mensaje IPC (64 KiB) para prevenir DoS por agotamiento de memoria.
const MAX_IPC_LINE_BYTES: usize = 64 * 1024;
/// Número máximo de clientes IPC simultáneos para prevenir agotamiento de hilos/descriptores.
const MAX_IPC_CLIENTS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MinimizedWindowInfo {
    pub window_id: u64,
    pub app_id: String,
    pub title: String,
    pub icon_name: Option<String>,
    #[serde(default)]
    pub thumbnail_path: Option<String>,
    #[serde(default)]
    pub is_animating: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct GlassRegion {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub radius: f32,
    #[serde(default)]
    pub is_capsule: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSwitcherItem {
    pub window_id: u64,
    pub app_id: String,
    pub title: String,
    pub icon_name: Option<String>,
    #[serde(default)]
    pub is_minimized: bool,
}

/// Mensajes IPC transmitidos por Unix Domain Socket entre el compositor y los clientes shell
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcMessage {
    /// Invocación de Spotlight (abrir / cerrar modal)
    ToggleSpotlight,
    /// Cerrar Spotlight cuando se hace clic fuera de su interfaz
    HideSpotlight,
    /// Minimizar u ocultar la ventana activa actualmente (⌘H)
    HideActiveWindow,
    /// Minimizar u ocultar todas las demás ventanas excepto la aplicación activa (⌥⌘H)
    HideOtherWindows,
    /// Cerrar la ventana activa actualmente (⌘Q)
    CloseActiveWindow,
    /// Mostrar selector visual de aplicaciones estilo macOS (⌘Tab)
    ShowAppSwitcher {
        items: Vec<AppSwitcherItem>,
        selected_index: usize,
    },
    /// Actualizar índice seleccionado en el selector visual de aplicaciones
    UpdateAppSwitcher { selected_index: usize },
    /// Ocultar y descartar el selector visual de aplicaciones
    HideAppSwitcher,
    /// Conmutar a una ventana específica por su identificador
    SwitchToWindow { window_id: u64 },
    /// Activar o restaurar ventana de una aplicación desde el Dock o lanzador
    ActivateApp { app_id: String },
    /// Notificación de cambio de ventana activa / enfocada
    ActiveAppChanged {
        app_id: Option<String>,
        title: Option<String>,
        #[serde(default)]
        window_id: Option<u64>,
    },
    /// Notificación de lista de aplicaciones en ejecución (app_ids o títulos)
    RunningAppsChanged { running_apps: Vec<String> },
    /// Notificación de lista de ventanas minimizadas en el Dock
    MinimizedWindowsChanged { windows: Vec<MinimizedWindowInfo> },
    /// Notificación de coordenadas de slot en el Dock para animación Genie Effect
    RegisterDockSlot {
        window_id: u64,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    /// Solicitud de minimización con efecto Genie
    AnimateMinimize { window_id: u64 },
    /// Solicitud de restauración con efecto Genie inverso
    AnimateRestore { window_id: u64 },
    /// Cambio de espacio de trabajo virtual
    SwitchWorkspace { index: usize },
    /// Toggle Mission Control (Exposé)
    ToggleMissionControl,
    /// Toggle Dispersión de Ventanas para Mostrar / Ocultar Escritorio (Super + D)
    ToggleShowDesktop,
    /// Alternar Centro de Notificaciones / Centro de Control (esquina activa o atajo)
    ToggleNotificationCenter,
    /// Invocación del Historial Visual del Portapapeles (Super + V)
    ToggleClipboardHistory,
    /// Invocación del Selector de Emojis y Símbolos (Super + .)
    ToggleEmojiPicker,
    /// Invocación del Menú flotante Snap Layouts (Super + Z)
    ToggleSnapLayouts,
    /// Invocación de la herramienta de captura / grabación de pantalla
    TriggerScreenshot {
        interactive: bool,
        record_video: bool,
    },
    /// Inicia la grabación de pantalla con parámetros opcionales de recorte, audio y modo sin pérdidas
    StartScreenRecording {
        destination: String,
        rect: Option<(i32, i32, i32, i32)>,
        include_audio: bool,
        fps: u32,
        #[serde(default)]
        lossless: bool,
    },
    /// Detiene la grabación de pantalla activa
    StopScreenRecording,
    /// Notificación periódica de estado de grabación activa
    ScreenRecordingStatus {
        is_recording: bool,
        elapsed_secs: u64,
    },
    /// Notificación de finalización de grabación de pantalla
    ScreenRecordingFinished {
        destination: String,
        success: bool,
        error: Option<String>,
    },
    /// Solicita al compositor guardar el siguiente fotograma de pantalla como PNG.
    CaptureScreenshot { destination: String },
    /// Solicita una captura en una salida concreta del compositor.
    CaptureScreenshotForOutput {
        destination: String,
        output_name: String,
    },
    /// Solicita al compositor capturar una ventana aislada en PNG RGBA con fondo transparente (sin fondo ni ventanas traseras).
    CaptureWindowScreenshot {
        destination: String,
        window_id: u64,
        #[serde(default)]
        output_name: Option<String>,
    },
    /// Invocación del Extractor de Texto en Pantalla OCR (Super + Shift + T)
    TriggerOcrCapture,
    /// Conmutar entre Modo Flotante y Modo Mosaico Automático (Super + Y)
    ToggleAutoTiling,
    /// Notificación de estado activo de Auto-Tiling / Ordenamiento de ventanas
    AutoTilingChanged { enabled: bool },
    /// Conmutar Picture-in-Picture universal para la ventana activa (Super + P)
    TogglePictureInPicture,
    /// Filtrado difuso dinámico de ventanas en Mission Control al teclear
    MissionControlFilter { query: String },
    /// Disparar localizador acústico del teléfono (Hacer sonar teléfono)
    MalusLinkRingPhone,
    /// Enviar archivo al teléfono móvil emparejado (AirDrop Linux)
    MalusLinkSendFile { file_path: String },
    /// Solicitud de cambio de modo de rendimiento (power-profiles-daemon)
    SetPowerProfile {
        profile: crate::config::PowerProfile,
    },
    /// Notificación de cambio de modo de rendimiento activo
    PowerProfileChanged {
        profile: crate::config::PowerProfile,
    },
    /// Notificación de recarga de tema
    ThemeChanged { theme_name: String, is_dark: bool },
    /// Notificación de cambio del color de énfasis
    AccentColorChanged { index: usize },
    /// Notificación de cambio en el nivel de transparencia / opacidad de Liquid Glass (`[0.15, 0.95]`)
    LiquidGlassOpacityChanged { opacity: f64 },
    /// Notificación de cambio de lado de los botones de ventana (`false` = izquierda macOS, `true` = derecha)
    WindowButtonsLayoutChanged { right: bool },
    /// Solicita al compositor capturar el fotograma actual antes de cambiar el tema.
    PrepareThemeTransition { request_id: String },
    /// Confirmación enviada cuando el fotograma anterior quedó guardado.
    ThemeTransitionPrepared { request_id: String },
    /// Solicitud de anclar una aplicación al Dock
    PinAppToDock {
        name: String,
        icon: String,
        command: String,
        #[serde(default)]
        matchers: Vec<String>,
        #[serde(default)]
        insert_at_dock_x: Option<i32>,
    },
    /// Solicitud de crear un enlace directo de aplicación en el escritorio (arrastrando desde Spotlight)
    CreateDesktopAppShortcut {
        desktop_file: String,
        drop_x: f64,
        drop_y: f64,
    },
    /// Solicitud de desanclar una aplicación del Dock
    UnpinAppFromDock { name: String },
    /// Notificación de previsualización del hueco de inserción en el Dock al arrastrar un elemento (ej. desde Spotlight)
    PreviewDockDropGap {
        #[serde(default)]
        dock_x: Option<i32>,
    },
    /// Notificación de inicio de arrastre de un icono del Dock para que el compositor lo dibuje pegado al cursor
    StartDockIconDrag { name: String, icon: String },
    /// Notificación de fin de arrastre de un icono del Dock (`poof = true` reproduce la animación puff al soltar fuera)
    EndDockIconDrag { poof: bool },
    /// Notificación de estado de borde/hover del compositor para auto-hide del Dock
    DockAutoHideEdge { active: bool },
    /// Notificación o solicitud de recarga de configuración del Dock (dock.toml)
    ReloadDockConfig,
    /// Notificación o solicitud de recarga de configuración de Escritorio y Dock (desktop-dock.toml)
    ReloadDesktopDockConfig,
    /// Notificación o solicitud de recarga de configuración de entrada / esquinas activas (input.toml)
    ReloadInputConfig,
    /// Notificación o solicitud de recarga de configuración de la Barra de menús (menubar.toml)
    ReloadMenuBarConfig,
    /// Solicitud de recarga en caliente de la configuración de pantallas y escalado (monitors.toml)
    ReloadDisplaysConfig,
    /// Solicitud ligera de recarga de brillo por pantalla sin reconstruir superficies ni capas shell
    ReloadDisplayBrightness,
    /// Notificación o solicitud de recarga de configuración de Fondo de pantalla (wallpaper.toml)
    ReloadWallpaperConfig,
    /// Solicitud de cambio en vivo de la distribución de teclado XKB (ej. "latam", "es", "us")
    SetKeyboardLayout {
        layout: String,
        #[serde(default)]
        variant: String,
    },
    /// Notificación de cambio activo de la distribución de teclado
    KeyboardLayoutChanged {
        layout: String,
        #[serde(default)]
        variant: String,
    },
    /// Regiones individuales de cristal líquido con refracción GPU para el Centro de Control flotante
    SetControlCenterGlassRegions {
        regions: Vec<GlassRegion>,
    },
}

/// Obtiene la ruta canónica del socket IPC Unix de Malus DE.
/// Prefiere `XDG_RUNTIME_DIR`; si no está definido, usa un subdirectorio privado por UID (`0o700`)
/// en lugar de exponer un socket global en `/tmp`.
pub fn ipc_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let trimmed = runtime_dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("malus-de-ipc.sock");
        }
    }
    let uid = unsafe { libc::geteuid() };
    let private_dir = PathBuf::from(format!("/tmp/malus-de-runtime-{uid}"));
    if let Ok(meta) = std::fs::symlink_metadata(&private_dir) {
        if !meta.file_type().is_dir() || meta.uid() != uid {
            tracing::warn!(
                "⚠️ Directorio de respaldo IPC no confiable ({:?}); evitando sobrescribirlo",
                private_dir
            );
        } else {
            let _ = std::fs::set_permissions(&private_dir, std::fs::Permissions::from_mode(0o700));
        }
    } else if std::fs::create_dir_all(&private_dir).is_ok() {
        let _ = std::fs::set_permissions(&private_dir, std::fs::Permissions::from_mode(0o700));
    }
    private_dir.join("malus-de-ipc.sock")
}

/// Verifica mediante `SO_PEERCRED` que el proceso conectado al socket Unix pertenece al mismo UID efectivo.
fn peer_matches_current_uid(stream: &UnixStream) -> bool {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if rc != 0 || len as usize != std::mem::size_of::<libc::ucred>() {
        return false;
    }
    let expected_uid = unsafe { libc::geteuid() };
    cred.uid == expected_uid
}

/// Lee una línea delimitada por `\n` acotando el tamaño máximo para prevenir agotamiento de memoria.
/// Devuelve `Ok(Some(line))` si se leyó una línea válida, `Ok(None)` en EOF, o `Err(InvalidData)` si excede `max_bytes`.
fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let mut limited = reader.by_ref().take((max_bytes + 1) as u64);
    let read = limited.read_until(b'\n', &mut buf)?;
    if read == 0 {
        return Ok(None);
    }
    if buf.len() > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Mensaje IPC excede el tamaño máximo permitido",
        ));
    }
    while matches!(buf.last(), Some(b'\n' | b'\r')) {
        buf.pop();
    }
    let line = String::from_utf8(buf).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "Mensaje IPC no es UTF-8")
    })?;
    Ok(Some(line))
}

/// Asegura que la ruta del socket no sea un archivo regular ni enlace simbólico antes de limpiarla.
fn remove_stale_socket_if_safe(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.file_type().is_socket() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "No se reemplazará una ruta IPC que no es socket: {}",
                        path.display()
                    ),
                ));
            }
            std::fs::remove_file(path)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

struct ActiveClientGuard(Arc<AtomicUsize>);

impl Drop for ActiveClientGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Servidor y difusor de mensajes IPC para el compositor
#[derive(Clone)]
pub struct IpcBroadcaster {
    clients: Arc<Mutex<Vec<UnixStream>>>,
    last_active: Arc<Mutex<Option<String>>>,
    last_running: Arc<Mutex<Option<String>>>,
    last_minimized: Arc<Mutex<Option<String>>>,
    last_recording_status: Arc<Mutex<Option<String>>>,
    incoming_tx: std::sync::mpsc::Sender<IpcMessage>,
    incoming_rx: Arc<Mutex<std::sync::mpsc::Receiver<IpcMessage>>>,
}

impl Default for IpcBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl IpcBroadcaster {
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            clients: Arc::new(Mutex::new(Vec::new())),
            last_active: Arc::new(Mutex::new(None)),
            last_running: Arc::new(Mutex::new(None)),
            last_minimized: Arc::new(Mutex::new(None)),
            last_recording_status: Arc::new(Mutex::new(None)),
            incoming_tx: tx,
            incoming_rx: Arc::new(Mutex::new(rx)),
        }
    }

    /// Obtiene el siguiente mensaje IPC recibido desde algún cliente shell
    pub fn try_recv(&self) -> Option<IpcMessage> {
        self.incoming_rx.lock().unwrap().try_recv().ok()
    }

    /// Inicializa el servidor Unix Listener en un hilo dedicado en segundo plano
    pub fn start_server(&self) -> std::io::Result<()> {
        let path = ipc_socket_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        remove_stale_socket_if_safe(&path)?;
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let clients = self.clients.clone();
        let last_active = self.last_active.clone();
        let last_running = self.last_running.clone();
        let last_minimized = self.last_minimized.clone();
        let last_recording_status = self.last_recording_status.clone();
        let incoming_tx = self.incoming_tx.clone();
        let active_readers = Arc::new(AtomicUsize::new(0));

        thread::spawn(move || {
            tracing::info!("📡 Servidor IPC Malus DE escuchando en: {:?}", path);
            for stream in listener.incoming() {
                if let Ok(mut stream) = stream {
                    if !peer_matches_current_uid(&stream) {
                        tracing::warn!("🔒 Conexión IPC rechazada: UID de cliente no coincide");
                        continue;
                    }

                    let current = active_readers.fetch_add(1, Ordering::SeqCst);
                    if current >= MAX_IPC_CLIENTS {
                        active_readers.fetch_sub(1, Ordering::SeqCst);
                        tracing::warn!(
                            "⚠️ Conexión IPC rechazada: límite de clientes concurrentes ({}) alcanzado",
                            MAX_IPC_CLIENTS
                        );
                        continue;
                    }
                    let guard = ActiveClientGuard(active_readers.clone());

                    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(20)));
                    // Enviar los últimos estados conocidos al nuevo cliente conectado
                    if let Some(ref act) = *last_active.lock().unwrap() {
                        let _ = stream.write_all(act.as_bytes());
                    }
                    if let Some(ref run) = *last_running.lock().unwrap() {
                        let _ = stream.write_all(run.as_bytes());
                    }
                    if let Some(ref min) = *last_minimized.lock().unwrap() {
                        let _ = stream.write_all(min.as_bytes());
                    }
                    if let Some(ref rec) = *last_recording_status.lock().unwrap() {
                        let _ = stream.write_all(rec.as_bytes());
                    }

                    // Hilo lector para escuchar peticiones del cliente (ej. ActivateApp, ToggleSpotlight)
                    if let Ok(read_stream) = stream.try_clone() {
                        let tx = incoming_tx.clone();
                        thread::spawn(move || {
                            let _guard = guard;
                            let mut reader = BufReader::new(read_stream);
                            loop {
                                match read_bounded_line(&mut reader, MAX_IPC_LINE_BYTES) {
                                    Ok(Some(line)) => {
                                        if let Ok(msg) = serde_json::from_str::<IpcMessage>(&line) {
                                            let _ = tx.send(msg);
                                        }
                                    }
                                    Ok(None) => break,
                                    Err(err) => {
                                        tracing::warn!(
                                            "⚠️ Cerrando cliente IPC por error de lectura: {}",
                                            err
                                        );
                                        break;
                                    }
                                }
                            }
                        });
                    } else {
                        drop(guard);
                    }

                    let mut lock = clients.lock().unwrap();
                    if lock.len() >= MAX_IPC_CLIENTS {
                        lock.remove(0);
                    }
                    lock.push(stream);
                }
            }
        });

        Ok(())
    }

    /// Difunde un mensaje JSON a todos los clientes shell conectados (Top Bar, Dock, etc.)
    pub fn broadcast(&self, msg: &IpcMessage) {
        let Ok(json) = serde_json::to_string(msg) else {
            return;
        };
        let line = format!("{}\n", json);
        let bytes = line.as_bytes();

        match msg {
            IpcMessage::ActiveAppChanged { .. } => {
                let mut guard = self.last_active.lock().unwrap();
                if guard.as_deref() == Some(line.as_str()) {
                    return;
                }
                *guard = Some(line.clone());
            }
            IpcMessage::RunningAppsChanged { .. } => {
                let mut guard = self.last_running.lock().unwrap();
                if guard.as_deref() == Some(line.as_str()) {
                    return;
                }
                *guard = Some(line.clone());
            }
            IpcMessage::MinimizedWindowsChanged { .. } => {
                let mut guard = self.last_minimized.lock().unwrap();
                if guard.as_deref() == Some(line.as_str()) {
                    return;
                }
                *guard = Some(line.clone());
            }
            IpcMessage::ScreenRecordingStatus { is_recording, .. } => {
                let mut guard = self.last_recording_status.lock().unwrap();
                if *is_recording {
                    *guard = Some(line.clone());
                } else {
                    *guard = None;
                }
            }
            IpcMessage::ScreenRecordingFinished { .. } => {
                let mut guard = self.last_recording_status.lock().unwrap();
                *guard = None;
            }
            _ => {}
        }

        let mut clients = self.clients.lock().unwrap();
        clients.retain_mut(|client| client.write_all(bytes).is_ok());
    }
}

/// Envía un mensaje IPC de un solo disparo al socket del compositor
pub fn send_ipc_message(msg: &IpcMessage) -> std::io::Result<()> {
    let path = ipc_socket_path();
    let mut stream = UnixStream::connect(path)?;
    let json = serde_json::to_string(msg)?;
    writeln!(stream, "{}", json)?;
    Ok(())
}

pub struct IpcClient;

impl IpcClient {
    pub fn send_command(msg: IpcMessage) -> std::io::Result<()> {
        send_ipc_message(&msg)
    }

    pub fn prepare_theme_transition() -> std::io::Result<bool> {
        let request_id = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let mut stream = UnixStream::connect(ipc_socket_path())?;
        stream.set_read_timeout(Some(std::time::Duration::from_millis(250)))?;
        let request = IpcMessage::PrepareThemeTransition {
            request_id: request_id.clone(),
        };
        let json = serde_json::to_string(&request)?;
        writeln!(stream, "{json}")?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => return Ok(false),
                Ok(_) => {
                    if let Ok(IpcMessage::ThemeTransitionPrepared {
                        request_id: prepared_id,
                    }) = serde_json::from_str(&line)
                    {
                        if prepared_id == request_id {
                            return Ok(true);
                        }
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(false);
                }
                Err(error) => return Err(error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_transition_messages_round_trip() {
        let request = IpcMessage::PrepareThemeTransition {
            request_id: "request-1".to_string(),
        };
        let serialized = serde_json::to_string(&request).expect("Debe serializar");
        assert!(matches!(
            serde_json::from_str::<IpcMessage>(&serialized),
            Ok(IpcMessage::PrepareThemeTransition { request_id }) if request_id == "request-1"
        ));

        let response = IpcMessage::ThemeTransitionPrepared {
            request_id: "request-1".to_string(),
        };
        let serialized = serde_json::to_string(&response).expect("Debe serializar");
        assert!(matches!(
            serde_json::from_str::<IpcMessage>(&serialized),
            Ok(IpcMessage::ThemeTransitionPrepared { request_id }) if request_id == "request-1"
        ));
    }

    #[test]
    fn test_minimized_windows_serialization() {
        let msg = IpcMessage::MinimizedWindowsChanged {
            windows: vec![MinimizedWindowInfo {
                window_id: 42,
                app_id: "org.gnome.Nautilus".to_string(),
                title: "Archivos — Documentos".to_string(),
                icon_name: Some("system-file-manager".to_string()),
                thumbnail_path: None,
                is_animating: true,
            }],
        };

        let json = serde_json::to_string(&msg).expect("Debe serializar");
        let parsed: IpcMessage = serde_json::from_str(&json).expect("Debe deserializar");

        if let IpcMessage::MinimizedWindowsChanged { windows } = parsed {
            assert_eq!(windows.len(), 1);
            assert_eq!(windows[0].window_id, 42);
            assert_eq!(windows[0].app_id, "org.gnome.Nautilus");
            assert_eq!(windows[0].icon_name.as_deref(), Some("system-file-manager"));
            assert!(windows[0].is_animating);
        } else {
            panic!("Tipo de mensaje incorrecto");
        }

        let legacy_json = r#"{"window_id":43,"app_id":"app","title":"Ventana","icon_name":null}"#;
        let legacy_info: MinimizedWindowInfo =
            serde_json::from_str(legacy_json).expect("Debe aceptar la lista de ventanas anterior");
        assert_eq!(legacy_info.thumbnail_path, None);
        assert!(!legacy_info.is_animating);
    }

    #[test]
    fn test_dock_slot_and_genie_messages() {
        let slot_msg = IpcMessage::RegisterDockSlot {
            window_id: 10,
            x: 500,
            y: 980,
            width: 48,
            height: 44,
        };
        let json = serde_json::to_string(&slot_msg).unwrap();
        assert!(json.contains("RegisterDockSlot"));

        let restore_msg = IpcMessage::AnimateRestore { window_id: 10 };
        let json_restore = serde_json::to_string(&restore_msg).unwrap();
        assert!(json_restore.contains("AnimateRestore"));
    }

    #[test]
    fn test_app_switcher_serialization() {
        let msg = IpcMessage::ShowAppSwitcher {
            items: vec![
                AppSwitcherItem {
                    window_id: 1,
                    app_id: "org.gnome.Ptyxis".to_string(),
                    title: "Terminal".to_string(),
                    icon_name: Some("utilities-terminal".to_string()),
                    is_minimized: false,
                },
                AppSwitcherItem {
                    window_id: 2,
                    app_id: "org.gnome.Nautilus".to_string(),
                    title: "Archivos".to_string(),
                    icon_name: Some("system-file-manager".to_string()),
                    is_minimized: true,
                },
            ],
            selected_index: 1,
        };

        let json = serde_json::to_string(&msg).expect("Serialización falló");
        let parsed: IpcMessage = serde_json::from_str(&json).expect("Deserialización falló");
        if let IpcMessage::ShowAppSwitcher {
            items,
            selected_index,
        } = parsed
        {
            assert_eq!(items.len(), 2);
            assert_eq!(selected_index, 1);
            assert!(items[1].is_minimized);
        } else {
            panic!("Tipo incorrecto");
        }
    }

    #[test]
    fn test_screenshot_request_serialization() {
        let msg = IpcMessage::CaptureScreenshot {
            destination: "/run/user/1000/malus-screenshot-test.png".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("Serialización falló");
        let parsed: IpcMessage = serde_json::from_str(&json).expect("Deserialización falló");
        assert!(matches!(
            parsed,
            IpcMessage::CaptureScreenshot { destination }
                if destination == "/run/user/1000/malus-screenshot-test.png"
        ));
    }

    #[test]
    fn test_screenshot_output_request_serialization() {
        let msg = IpcMessage::CaptureScreenshotForOutput {
            destination: "/run/user/1000/malus-screenshot-area.png".to_string(),
            output_name: "DP-1".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("Serialización falló");
        let parsed: IpcMessage = serde_json::from_str(&json).expect("Deserialización falló");
        assert!(matches!(
            parsed,
            IpcMessage::CaptureScreenshotForOutput {
                destination,
                output_name
            } if destination == "/run/user/1000/malus-screenshot-area.png"
                && output_name == "DP-1"
        ));
    }

    #[test]
    fn test_screen_recording_serialization() {
        let msg = IpcMessage::StartScreenRecording {
            destination: "/home/user/video.mp4".to_string(),
            rect: Some((100, 200, 800, 600)),
            include_audio: true,
            fps: 60,
            lossless: true,
        };
        let json = serde_json::to_string(&msg).expect("Serialización falló");
        let parsed: IpcMessage = serde_json::from_str(&json).expect("Deserialización falló");
        assert!(matches!(
            parsed,
            IpcMessage::StartScreenRecording {
                destination,
                rect: Some((100, 200, 800, 600)),
                include_audio: true,
                fps: 60,
                lossless: true,
            } if destination == "/home/user/video.mp4"
        ));

        // Retrocompatibilidad: payloads antiguos sin campo `lossless` deben deserializarse con false
        let legacy_json = r#"{"StartScreenRecording":{"destination":"/home/user/old.mp4","rect":null,"include_audio":false,"fps":30}}"#;
        let legacy_parsed: IpcMessage =
            serde_json::from_str(legacy_json).expect("Deserialización legacy falló");
        assert!(matches!(
            legacy_parsed,
            IpcMessage::StartScreenRecording {
                lossless: false,
                fps: 30,
                ..
            }
        ));

        let stop = IpcMessage::StopScreenRecording;
        let json = serde_json::to_string(&stop).expect("Serialización falló");
        let parsed: IpcMessage = serde_json::from_str(&json).expect("Deserialización falló");
        assert!(matches!(parsed, IpcMessage::StopScreenRecording));
    }

    #[test]
    fn test_read_bounded_line_rejects_oversized_payload() {
        let mut cursor = std::io::Cursor::new(b"hola\n");
        let ok = read_bounded_line(&mut cursor, 16).expect("Debe leer línea corta");
        assert_eq!(ok.as_deref(), Some("hola"));
        assert_eq!(read_bounded_line(&mut cursor, 16).unwrap(), None);

        let oversized = vec![b'A'; 128];
        let mut big_cursor = std::io::Cursor::new(oversized);
        let err = read_bounded_line(&mut big_cursor, 64).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_peer_matches_current_uid_on_unix_pair() {
        let (a, _b) = UnixStream::pair().expect("pair");
        assert!(peer_matches_current_uid(&a));
    }

    #[test]
    fn test_hide_other_windows_ipc_round_trip() {
        let msg = IpcMessage::HideOtherWindows;
        let json = serde_json::to_string(&msg).expect("Debe serializar HideOtherWindows");
        assert!(matches!(
            serde_json::from_str::<IpcMessage>(&json),
            Ok(IpcMessage::HideOtherWindows)
        ));
    }

    #[test]
    fn test_remove_stale_socket_rejects_regular_file() {
        let tmp = std::env::temp_dir().join(format!(
            "malus-ipc-non-socket-{}",
            std::process::id()
        ));
        std::fs::write(&tmp, b"not a socket").unwrap();
        let res = remove_stale_socket_if_safe(&tmp);
        assert!(res.is_err());
        assert!(tmp.exists());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_control_center_glass_regions_ipc_round_trip() {
        let msg = IpcMessage::SetControlCenterGlassRegions {
            regions: vec![
                GlassRegion {
                    x: 8,
                    y: 8,
                    width: 150,
                    height: 70,
                    radius: 35.0,
                    is_capsule: true,
                },
                GlassRegion {
                    x: 168,
                    y: 8,
                    width: 150,
                    height: 150,
                    radius: 26.0,
                    is_capsule: false,
                },
            ],
        };
        let json = serde_json::to_string(&msg).expect("serializar SetControlCenterGlassRegions");
        let parsed: IpcMessage =
            serde_json::from_str(&json).expect("deserializar SetControlCenterGlassRegions");
        match parsed {
            IpcMessage::SetControlCenterGlassRegions { regions } => {
                assert_eq!(regions.len(), 2);
                assert!(regions[0].is_capsule);
                assert!(!regions[1].is_capsule);
                assert_eq!((regions[1].width, regions[1].height), (150, 150));
            }
            other => panic!("mensaje inesperado: {other:?}"),
        }
    }
}
