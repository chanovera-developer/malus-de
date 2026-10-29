use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

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
    /// Cerrar la ventana activa actualmente (⌘Q)
    CloseActiveWindow,
    /// Mostrar selector visual de aplicaciones estilo macOS (⌘Tab)
    ShowAppSwitcher {
        items: Vec<AppSwitcherItem>,
        selected_index: usize,
    },
    /// Actualizar índice seleccionado en el selector visual de aplicaciones
    UpdateAppSwitcher {
        selected_index: usize,
    },
    /// Ocultar y descartar el selector visual de aplicaciones
    HideAppSwitcher,
    /// Conmutar a una ventana específica por su identificador
    SwitchToWindow {
        window_id: u64,
    },
    /// Activar o restaurar ventana de una aplicación desde el Dock o lanzador
    ActivateApp {
        app_id: String,
    },
    /// Notificación de cambio de ventana activa / enfocada
    ActiveAppChanged {
        app_id: Option<String>,
        title: Option<String>,
        #[serde(default)]
        window_id: Option<u64>,
    },
    /// Notificación de lista de aplicaciones en ejecución (app_ids o títulos)
    RunningAppsChanged {
        running_apps: Vec<String>,
    },
    /// Notificación de lista de ventanas minimizadas en el Dock
    MinimizedWindowsChanged {
        windows: Vec<MinimizedWindowInfo>,
    },
    /// Notificación de coordenadas de slot en el Dock para animación Genie Effect
    RegisterDockSlot {
        window_id: u64,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    /// Solicitud de minimización con efecto Genie
    AnimateMinimize {
        window_id: u64,
    },
    /// Solicitud de restauración con efecto Genie inverso
    AnimateRestore {
        window_id: u64,
    },
    /// Cambio de espacio de trabajo virtual
    SwitchWorkspace {
        index: usize,
    },
    /// Toggle Mission Control (Exposé)
    ToggleMissionControl,
    /// Toggle Dispersión de Ventanas para Mostrar / Ocultar Escritorio (Super + D)
    ToggleShowDesktop,
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
    /// Solicita al compositor guardar el siguiente fotograma de pantalla como PNG.
    CaptureScreenshot { destination: String },
    /// Solicita una captura en una salida concreta del compositor.
    CaptureScreenshotForOutput {
        destination: String,
        output_name: String,
    },
    /// Invocación del Extractor de Texto en Pantalla OCR (Super + Shift + T)
    TriggerOcrCapture,
    /// Conmutar entre Modo Flotante y Modo Mosaico Automático (Super + Y)
    ToggleAutoTiling,
    /// Conmutar Picture-in-Picture universal para la ventana activa (Super + P)
    TogglePictureInPicture,
    /// Filtrado difuso dinámico de ventanas en Mission Control al teclear
    MissionControlFilter {
        query: String,
    },
    /// Disparar localizador acústico del teléfono (Hacer sonar teléfono)
    MalusLinkRingPhone,
    /// Enviar archivo al teléfono móvil emparejado (AirDrop Linux)
    MalusLinkSendFile {
        file_path: String,
    },
    /// Solicitud de cambio de modo de rendimiento (power-profiles-daemon)
    SetPowerProfile {
        profile: crate::config::PowerProfile,
    },
    /// Notificación de cambio de modo de rendimiento activo
    PowerProfileChanged {
        profile: crate::config::PowerProfile,
    },
    /// Notificación de recarga de tema
    ThemeChanged {
        theme_name: String,
        is_dark: bool,
    },
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
    },
    /// Solicitud de desanclar una aplicación del Dock
    UnpinAppFromDock {
        name: String,
    },
    /// Notificación o solicitud de recarga de configuración del Dock (dock.toml)
    ReloadDockConfig,
    /// Solicitud de recarga en caliente de la configuración de pantallas y escalado (monitors.toml)
    ReloadDisplaysConfig,
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
}

/// Obtiene la ruta canónica del socket IPC Unix de Malus DE
pub fn ipc_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("malus-de-ipc.sock")
    } else {
        PathBuf::from("/tmp/malus-de-ipc.sock")
    }
}

/// Servidor y difusor de mensajes IPC para el compositor
#[derive(Clone)]
pub struct IpcBroadcaster {
    clients: Arc<Mutex<Vec<UnixStream>>>,
    last_active: Arc<Mutex<Option<String>>>,
    last_running: Arc<Mutex<Option<String>>>,
    last_minimized: Arc<Mutex<Option<String>>>,
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
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        let clients = self.clients.clone();
        let last_active = self.last_active.clone();
        let last_running = self.last_running.clone();
        let last_minimized = self.last_minimized.clone();
        let incoming_tx = self.incoming_tx.clone();

        thread::spawn(move || {
            tracing::info!("📡 Servidor IPC Malus DE escuchando en: {:?}", path);
            for stream in listener.incoming() {
                if let Ok(mut stream) = stream {
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

                    // Hilo lector para escuchar peticiones del cliente (ej. ActivateApp, ToggleSpotlight)
                    if let Ok(read_stream) = stream.try_clone() {
                        let tx = incoming_tx.clone();
                        thread::spawn(move || {
                            let reader = BufReader::new(read_stream);
                            for line in reader.lines() {
                                if let Ok(line) = line {
                                    if let Ok(msg) = serde_json::from_str::<IpcMessage>(&line) {
                                        let _ = tx.send(msg);
                                    }
                                } else {
                                    break;
                                }
                            }
                        });
                    }

                    let mut lock = clients.lock().unwrap();
                    lock.push(stream);
                }
            }
        });

        Ok(())
    }

    /// Difunde un mensaje JSON a todos los clientes shell conectados (Top Bar, Dock, etc.)
    pub fn broadcast(&self, msg: &IpcMessage) {
        let Ok(json) = serde_json::to_string(msg) else { return };
        let line = format!("{}\n", json);
        let bytes = line.as_bytes();

        match msg {
            IpcMessage::ActiveAppChanged { .. } => {
                *self.last_active.lock().unwrap() = Some(line.clone());
            }
            IpcMessage::RunningAppsChanged { .. } => {
                *self.last_running.lock().unwrap() = Some(line.clone());
            }
            IpcMessage::MinimizedWindowsChanged { .. } => {
                *self.last_minimized.lock().unwrap() = Some(line.clone());
            }
            _ => {}
        }

        let mut clients = self.clients.lock().unwrap();
        clients.retain_mut(|client| {
            client.write_all(bytes).is_ok()
        });
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
            windows: vec![
                MinimizedWindowInfo {
                    window_id: 42,
                    app_id: "org.gnome.Nautilus".to_string(),
                    title: "Archivos — Documentos".to_string(),
                    icon_name: Some("system-file-manager".to_string()),
                    thumbnail_path: None,
                    is_animating: true,
                },
            ],
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
        if let IpcMessage::ShowAppSwitcher { items, selected_index } = parsed {
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
}
