use anyhow::{bail, Context, Result};
use async_channel::{Receiver, Sender};
use gio::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CheckButton, Entry, Label, ListBox,
    Orientation, ScrolledWindow,
};
use malus_de_common::{
    ipc::{ipc_socket_path, IpcMessage},
    logging::init_logging,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::FileTypeExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tracing::{error, info, warn};

const MAX_HISTORY_ITEMS: usize = 50;
const MAX_CLIPBOARD_BYTES: usize = 10 * 1024 * 1024;
const MAX_HISTORY_STORAGE_BYTES: usize = 50 * 1024 * 1024;

#[derive(Clone, Debug)]
struct ClipboardItem {
    id: u64,
    mime_type: String,
    data: Vec<u8>,
    pinned: bool,
}

#[derive(Deserialize, Serialize)]
struct PersistedItem {
    id: u64,
    mime_type: String,
    pinned: bool,
    size: usize,
}

#[derive(Deserialize, Serialize)]
struct PersistedHistory {
    items: Vec<PersistedItem>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
struct ClipboardSettings {
    sync_primary_selection: bool,
    filter_sensitive_content: bool,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            sync_primary_selection: false,
            filter_sensitive_content: true,
        }
    }
}

#[derive(Default)]
struct ClipboardHistory {
    items: VecDeque<ClipboardItem>,
    next_id: u64,
    manager_copy: Option<(String, Vec<u8>, Instant)>,
}

impl ClipboardHistory {
    fn capture(&mut self, mime_type: String, data: Vec<u8>) -> Option<ClipboardItem> {
        if let Some((owned_mime, owned_data, expires)) = &self.manager_copy {
            if *expires > Instant::now() && *owned_mime == mime_type && *owned_data == data {
                self.manager_copy = None;
                return None;
            }
        }
        self.manager_copy = None;

        if let Some(existing) = self
            .items
            .iter()
            .find(|item| item.mime_type == mime_type && item.data == data)
            .cloned()
        {
            self.items.retain(|item| item.id != existing.id);
            self.items.push_front(existing.clone());
            return Some(existing);
        }

        self.next_id = self.next_id.wrapping_add(1).max(1);
        let item = ClipboardItem {
            id: self.next_id,
            mime_type,
            data,
            pinned: false,
        };
        self.items.push_front(item.clone());
        self.enforce_limits();
        Some(item)
    }

    fn enforce_limits(&mut self) {
        while self.items.len() > MAX_HISTORY_ITEMS
            || self.items.iter().map(|item| item.data.len()).sum::<usize>()
                > MAX_HISTORY_STORAGE_BYTES
        {
            let oldest = self
                .items
                .iter()
                .rposition(|item| !item.pinned)
                .unwrap_or_else(|| {
                    warn!("Historial lleno: se quitará el elemento fijado más antiguo");
                    self.items.len() - 1
                });
            self.items.remove(oldest);
        }
    }
}

#[derive(Deserialize, Serialize)]
struct CaptureHeader {
    mime_type: String,
    size: usize,
    primary_selection: bool,
    sensitive_hint: bool,
}

enum UiEvent {
    Refresh,
    ToggleWindow,
    SensitiveSkipped,
}

#[derive(Clone)]
struct HistoryUi {
    window: ApplicationWindow,
    search: Entry,
    list: ListBox,
    status: Label,
    history: Arc<Mutex<ClipboardHistory>>,
    history_path: PathBuf,
    settings_path: PathBuf,
    sync_primary_selection: Arc<AtomicBool>,
    filter_sensitive_content: Arc<AtomicBool>,
}

fn main() -> Result<()> {
    init_logging("malus_de_clipboard");

    match std::env::args().nth(1).as_deref() {
        Some("--capture") => {
            let primary_selection = std::env::args().any(|argument| argument == "--primary");
            capture_clipboard_change(primary_selection)
        }
        Some("--help") => {
            println!("malus-de-clipboard\n\nComponente de historial del portapapeles de Malus DE.");
            Ok(())
        }
        Some(argument) => bail!("Argumento no reconocido: {argument}"),
        None => run_manager(),
    }
}

fn run_manager() -> Result<()> {
    let history_path = history_path()?;
    let history = Arc::new(Mutex::new(load_history(&history_path)?));
    let settings_path = settings_path()?;
    let settings = load_settings(&settings_path)?;
    let sync_primary_selection = Arc::new(AtomicBool::new(settings.sync_primary_selection));
    let filter_sensitive_content = Arc::new(AtomicBool::new(settings.filter_sensitive_content));
    let (event_tx, event_rx) = async_channel::unbounded();

    let app = Application::builder()
        .application_id("org.malus.de.clipboard")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let _hold_guard = app.hold();

    let ui_state: Arc<Mutex<Option<HistoryUi>>> = Arc::new(Mutex::new(None));
    let ui_state_activate = ui_state.clone();
    let history_activate = history.clone();
    let history_path_activate = history_path.clone();
    let settings_path_activate = settings_path.clone();
    let sync_primary_activate = sync_primary_selection.clone();
    let filter_sensitive_activate = filter_sensitive_content.clone();
    app.connect_activate(move |application| {
        if ui_state_activate.lock().unwrap().is_some() {
            return;
        }
        let ui = build_history_ui(
            application,
            history_activate.clone(),
            history_path_activate.clone(),
            settings_path_activate.clone(),
            sync_primary_activate.clone(),
            filter_sensitive_activate.clone(),
        );
        *ui_state_activate.lock().unwrap() = Some(ui);
    });

    start_clipboard_service(
        history.clone(),
        event_tx.clone(),
        sync_primary_selection.clone(),
        filter_sensitive_content.clone(),
    )?;
    start_compositor_listener(event_tx)?;
    poll_ui_events(event_rx, ui_state);

    info!("Gestor de portapapeles iniciado; historial persistente de hasta {MAX_HISTORY_ITEMS} elementos.");
    app.run();
    Ok(())
}

fn app_config_dir() -> Result<PathBuf> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .context("No se pudo determinar el directorio de configuración del usuario")?;
    if config_home
        .file_name()
        .is_some_and(|name| name == "malus-de")
    {
        Ok(config_home)
    } else {
        Ok(config_home.join("malus-de"))
    }
}

fn settings_path() -> Result<PathBuf> {
    Ok(app_config_dir()?.join("clipboard.toml"))
}

fn history_path() -> Result<PathBuf> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .context("No se pudo determinar el directorio de datos del usuario")?;
    Ok(data_home.join("malus-de/clipboard-history.dat"))
}

fn load_settings(path: &Path) -> Result<ClipboardSettings> {
    match fs::read_to_string(path) {
        Ok(contents) => toml::from_str(&contents)
            .with_context(|| format!("No se pudo leer la configuración {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(ClipboardSettings::default())
        }
        Err(error) => Err(error).with_context(|| format!("No se pudo leer {}", path.display())),
    }
}

fn save_settings(path: &Path, settings: ClipboardSettings) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let parent = path
        .parent()
        .context("La ruta de configuración no tiene directorio padre")?;
    fs::create_dir_all(parent).with_context(|| format!("No se pudo crear {}", parent.display()))?;
    let temporary_path = path.with_extension("toml.tmp");
    let contents = toml::to_string_pretty(&settings)
        .context("No se pudo serializar la configuración del portapapeles")?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary_path)
        .with_context(|| format!("No se pudo abrir {}", temporary_path.display()))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .with_context(|| {
            format!(
                "No se pudieron proteger los permisos de {}",
                temporary_path.display()
            )
        })?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("No se pudo escribir {}", temporary_path.display()))?;
    file.sync_all()
        .with_context(|| format!("No se pudo sincronizar {}", temporary_path.display()))?;
    fs::rename(&temporary_path, path)
        .with_context(|| format!("No se pudo guardar {}", path.display()))
}

fn load_history(path: &Path) -> Result<ClipboardHistory> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ClipboardHistory::default())
        }
        Err(error) => {
            return Err(error).with_context(|| format!("No se pudo abrir {}", path.display()))
        }
    };
    let mut reader = BufReader::new(file);
    let mut header_line = String::new();
    (&mut reader)
        .take(1024 * 1024)
        .read_line(&mut header_line)
        .with_context(|| format!("No se pudo leer {}", path.display()))?;
    let persisted: PersistedHistory = serde_json::from_str(&header_line)
        .with_context(|| format!("El índice del historial {} no es válido", path.display()))?;
    if persisted.items.len() > MAX_HISTORY_ITEMS {
        bail!("El historial persistido excede el límite de {MAX_HISTORY_ITEMS} elementos");
    }

    let mut history = ClipboardHistory::default();
    let mut total_bytes = 0usize;
    for entry in persisted.items {
        if entry.size > MAX_CLIPBOARD_BYTES || entry.mime_type.len() > 256 {
            bail!("Un elemento del historial persistido excede los límites permitidos");
        }
        total_bytes = total_bytes
            .checked_add(entry.size)
            .context("El tamaño del historial persistido excede el límite")?;
        if total_bytes > MAX_HISTORY_STORAGE_BYTES {
            bail!("El historial persistido excede el límite de almacenamiento");
        }
        let mut data = vec![0; entry.size];
        reader.read_exact(&mut data).with_context(|| {
            format!(
                "Un elemento del historial {} está incompleto",
                path.display()
            )
        })?;
        history.next_id = history.next_id.max(entry.id);
        history.items.push_back(ClipboardItem {
            id: entry.id,
            mime_type: entry.mime_type,
            data,
            pinned: entry.pinned,
        });
    }
    Ok(history)
}

fn save_history(path: &Path, history: &ClipboardHistory) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let parent = path
        .parent()
        .context("La ruta del historial no tiene directorio padre")?;
    fs::create_dir_all(parent).with_context(|| format!("No se pudo crear {}", parent.display()))?;
    let temporary_path = path.with_extension("dat.tmp");
    let persisted = PersistedHistory {
        items: history
            .items
            .iter()
            .map(|item| PersistedItem {
                id: item.id,
                mime_type: item.mime_type.clone(),
                pinned: item.pinned,
                size: item.data.len(),
            })
            .collect(),
    };
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary_path)
        .with_context(|| format!("No se pudo abrir {}", temporary_path.display()))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .with_context(|| {
            format!(
                "No se pudieron proteger los permisos de {}",
                temporary_path.display()
            )
        })?;
    serde_json::to_writer(&mut file, &persisted)
        .context("No se pudo serializar el índice del historial")?;
    file.write_all(b"\n")
        .context("No se pudo terminar el índice del historial")?;
    for item in &history.items {
        file.write_all(&item.data)
            .context("No se pudo guardar un elemento del historial")?;
    }
    file.sync_all()
        .with_context(|| format!("No se pudo sincronizar {}", temporary_path.display()))?;
    fs::rename(&temporary_path, path)
        .with_context(|| format!("No se pudo guardar {}", path.display()))
}

fn clipboard_socket_path() -> Result<PathBuf> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .context("XDG_RUNTIME_DIR no está definido")?;
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "default".to_string());
    let display_key: String = display
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    Ok(runtime_dir.join(format!("malus-de-clipboard-{display_key}.sock")))
}

fn start_clipboard_service(
    history: Arc<Mutex<ClipboardHistory>>,
    event_tx: Sender<UiEvent>,
    sync_primary_selection: Arc<AtomicBool>,
    filter_sensitive_content: Arc<AtomicBool>,
) -> Result<()> {
    let socket_path = clipboard_socket_path()?;
    if let Ok(metadata) = fs::symlink_metadata(&socket_path) {
        if !metadata.file_type().is_socket() {
            bail!(
                "No se reemplazará la ruta no-socket del portapapeles: {}",
                socket_path.display()
            );
        }
        match UnixStream::connect(&socket_path) {
            Ok(_) => bail!(
                "Ya hay un gestor de portapapeles activo en {}",
                socket_path.display()
            ),
            Err(_) => fs::remove_file(&socket_path).with_context(|| {
                format!(
                    "No se pudo limpiar el socket obsoleto {}",
                    socket_path.display()
                )
            })?,
        }
    }
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("No se pudo abrir el socket {}", socket_path.display()))?;
    info!("Socket local del historial: {}", socket_path.display());

    let history_for_listener = history.clone();
    let sync_primary_for_listener = sync_primary_selection.clone();
    let filter_sensitive_for_listener = filter_sensitive_content.clone();
    thread::Builder::new()
        .name("clipboard-history-ipc".to_string())
        .spawn(move || {
            for incoming in listener.incoming() {
                match incoming {
                    Ok(stream) => {
                        let history = history_for_listener.clone();
                        let event_tx = event_tx.clone();
                        let sync_primary_selection = sync_primary_for_listener.clone();
                        let filter_sensitive_content = filter_sensitive_for_listener.clone();
                        thread::spawn(move || {
                            if let Err(error) = handle_capture(
                                stream,
                                history,
                                event_tx,
                                sync_primary_selection,
                                filter_sensitive_content,
                            ) {
                                warn!("No se pudo procesar un cambio del portapapeles: {error:#}");
                            }
                        });
                    }
                    Err(error) => error!("Error al aceptar cliente del historial: {error}"),
                }
            }
        })
        .context("No se pudo iniciar el servidor IPC del historial")?;

    start_clipboard_watcher(sync_primary_selection)?;
    Ok(())
}

fn start_clipboard_watcher(sync_primary_selection: Arc<AtomicBool>) -> Result<()> {
    let executable = std::env::current_exe()
        .context("No se pudo localizar el binario del gestor de portapapeles")?;
    thread::Builder::new()
        .name("clipboard-watcher".to_string())
        .spawn(move || {
            let mut clipboard_watcher: Option<std::process::Child> = None;
            let mut primary_watcher: Option<std::process::Child> = None;
            let mut clipboard_retry_at = Instant::now();
            let mut primary_retry_at = Instant::now();
            loop {
                let now = Instant::now();
                if clipboard_watcher.is_none() && now >= clipboard_retry_at {
                    clipboard_watcher = spawn_watcher(&executable, false);
                    clipboard_retry_at = now + Duration::from_secs(10);
                }
                match primary_watcher.as_mut() {
                    Some(child) if !sync_primary_selection.load(Ordering::Relaxed) => {
                        if let Err(error) = child.kill() {
                            warn!("No se pudo detener el monitor de selección primaria: {error}");
                        }
                        if let Err(error) = child.wait() {
                            warn!("No se pudo esperar al monitor de selección primaria: {error}");
                        }
                        primary_watcher = None;
                    }
                    None if sync_primary_selection.load(Ordering::Relaxed)
                        && now >= primary_retry_at =>
                    {
                        primary_watcher = spawn_watcher(&executable, true);
                        primary_retry_at = now + Duration::from_secs(10);
                    }
                    _ => {}
                }
                if reap_watcher(&mut clipboard_watcher, "portapapeles") {
                    clipboard_retry_at = Instant::now() + Duration::from_secs(10);
                }
                if reap_watcher(&mut primary_watcher, "selección primaria") {
                    primary_retry_at = Instant::now() + Duration::from_secs(10);
                }
                thread::sleep(Duration::from_millis(250));
            }
        })
        .context("No se pudo iniciar el monitor del portapapeles")?;
    Ok(())
}

fn spawn_watcher(executable: &Path, primary_selection: bool) -> Option<std::process::Child> {
    let mut command = Command::new("wl-paste");
    if primary_selection {
        command.arg("--primary");
    }
    match command
        .args(["--watch"])
        .arg(executable)
        .arg("--capture")
        .args(if primary_selection {
            &["--primary"][..]
        } else {
            &[][..]
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => Some(child),
        Err(error) => {
            error!("No se pudo iniciar wl-paste --watch; instala wl-clipboard: {error}");
            None
        }
    }
}

fn reap_watcher(watcher: &mut Option<std::process::Child>, name: &str) -> bool {
    let Some(child) = watcher.as_mut() else {
        return false;
    };
    match child.try_wait() {
        Ok(Some(status)) => {
            warn!("El monitor de {name} terminó ({status}); se reintentará");
            *watcher = None;
            true
        }
        Ok(None) => false,
        Err(error) => {
            warn!("No se pudo comprobar el monitor de {name}: {error}");
            *watcher = None;
            true
        }
    }
}

fn capture_clipboard_change(primary_selection: bool) -> Result<()> {
    if std::env::var("CLIPBOARD_STATE").as_deref() != Ok("data") {
        return Ok(());
    }

    let mut watched_data = Vec::new();
    std::io::stdin()
        .take((MAX_CLIPBOARD_BYTES + 1) as u64)
        .read_to_end(&mut watched_data)
        .context("No se pudo leer el contenido del portapapeles observado")?;
    if watched_data.len() > MAX_CLIPBOARD_BYTES {
        warn!("Se omitió contenido del portapapeles mayor a 10 MiB");
        return Ok(());
    }

    let mut selection_args = Vec::new();
    if primary_selection {
        selection_args.push("--primary");
    }
    selection_args.push("--list-types");
    let mime_types = run_bounded("wl-paste", &selection_args, 64 * 1024)?;
    let mime_types = String::from_utf8_lossy(&mime_types);
    let sensitive_hint = mime_types.lines().any(is_sensitive_mime_hint);
    let mime_type = match select_mime_type(&mime_types) {
        Some(mime_type) => mime_type,
        None if sensitive_hint => {
            let hint = mime_types
                .lines()
                .find(|mime_type| is_sensitive_mime_hint(mime_type))
                .unwrap_or("application/x-secret")
                .trim();
            send_capture(hint, &[], primary_selection, true)?;
            return Ok(());
        }
        None => {
            warn!("Se omitió un formato MIME no compatible del portapapeles");
            return Ok(());
        }
    };
    let mut data_args = Vec::new();
    if primary_selection {
        data_args.push("--primary");
    }
    data_args.extend(["--no-newline", "--type", &mime_type]);
    let data = match run_bounded("wl-paste", &data_args, MAX_CLIPBOARD_BYTES) {
        Ok(data) => data,
        Err(error) if !watched_data.is_empty() => {
            warn!("No se pudo volver a leer el MIME {mime_type}; se conservará el flujo del monitor: {error:#}");
            watched_data
        }
        Err(error) => return Err(error),
    };
    send_capture(&mime_type, &data, primary_selection, sensitive_hint)?;
    Ok(())
}

fn select_mime_type(types: &str) -> Option<String> {
    let offered: Vec<&str> = types.lines().map(str::trim).collect();
    let preferred = [
        "image/png",
        "image/jpeg",
        "image/webp",
        "image/gif",
        "image/bmp",
        "image/tiff",
        "image/avif",
        "image/heic",
        "image/heif",
        "image/svg+xml",
        "text/uri-list",
        "text/plain;charset=utf-8",
        "text/plain;charset=UTF-8",
        "text/plain",
        "UTF8_STRING",
        "TEXT",
        "STRING",
        "text/markdown",
        "text/csv",
        "text/xml",
        "text/html",
        "application/rtf",
        "text/rtf",
        "application/pdf",
        "application/vnd.oasis.opendocument.text",
        "application/vnd.oasis.opendocument.spreadsheet",
        "application/vnd.oasis.opendocument.presentation",
        "application/msword",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "application/vnd.ms-excel",
        "application/vnd.ms-powerpoint",
        "application/epub+zip",
    ];
    preferred
        .into_iter()
        .find_map(|candidate| {
            offered
                .iter()
                .find(|mime| mime.eq_ignore_ascii_case(candidate))
                .map(|mime| (*mime).to_string())
        })
        .or_else(|| {
            offered
                .iter()
                .find(|mime| {
                    !is_sensitive_mime_hint(mime)
                        && (mime.to_ascii_lowercase().starts_with("text/")
                            || mime.to_ascii_lowercase().starts_with("image/"))
                })
                .map(|mime| (*mime).to_string())
        })
}

fn is_sensitive_mime_hint(mime_type: &str) -> bool {
    let mime_type = mime_type.to_ascii_lowercase();
    mime_type.contains("passwordmanagerhint")
        || mime_type.contains("password-manager-hint")
        || mime_type.contains("keepass")
        || mime_type.contains("password-manager")
        || mime_type == "application/x-secret"
        || mime_type == "application/x-clipboard-manager-secret"
}

fn is_sensitive_content(mime_type: &str, data: &[u8]) -> bool {
    if is_sensitive_mime_hint(mime_type) {
        return true;
    }
    let Ok(text) = std::str::from_utf8(data) else {
        return false;
    };
    let lowercase = text.to_ascii_lowercase();
    if lowercase.contains("-----begin") && lowercase.contains("private key")
        || [
            "ghp_",
            "github_pat_",
            "glpat-",
            "xoxb-",
            "xoxp-",
            "sk_live_",
        ]
        .iter()
        .any(|prefix| lowercase.contains(prefix))
    {
        return true;
    }
    if let Some(credentials) = lowercase.split_once("://").map(|(_, rest)| rest) {
        if let Some(authority) = credentials.split('/').next() {
            if authority
                .split_once(':')
                .is_some_and(|(_, password)| password.contains('@') && password.len() >= 10)
            {
                return true;
            }
        }
    }

    for assignment in text.lines().flat_map(|line| line.split(',')) {
        let line = assignment.trim();
        if line.starts_with("export ") {
            let Some((key, value)) = line[7..].split_once('=') else {
                continue;
            };
            if is_secret_assignment(key, value) {
                return true;
            }
            continue;
        }
        let Some(separator) = line.find(['=', ':']) else {
            continue;
        };
        if is_secret_assignment(&line[..separator], &line[separator + 1..]) {
            return true;
        }
    }
    false
}

fn is_secret_assignment(key: &str, value: &str) -> bool {
    let key = key
        .trim()
        .trim_matches(|character: char| matches!(character, '"' | '\'' | '`' | '{' | '}' | ','))
        .to_ascii_lowercase()
        .replace('-', "_");
    let key = key.rsplit('.').next().unwrap_or(&key);
    let secret_key = [
        "password",
        "passwd",
        "pwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "access_key",
        "client_secret",
        "credential",
        "authorization",
    ]
    .iter()
    .any(|name| key == *name || key.ends_with(name));
    if !secret_key {
        return false;
    }
    let value = value
        .trim()
        .trim_matches(['"', '\'', '`'])
        .trim_end_matches(',');
    let normalized = value.to_ascii_lowercase();
    let placeholder = [
        "placeholder",
        "example",
        "changeme",
        "change_me",
        "redacted",
        "null",
        "none",
        "password",
        "secret",
    ]
    .iter()
    .any(|placeholder| normalized == *placeholder)
        || normalized.starts_with("your_")
        || normalized.starts_with("your-");
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .count()
        >= 8
        && !placeholder
}

fn run_bounded(program: &str, args: &[&str], max_bytes: usize) -> Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("No se pudo iniciar {program}"))?;
    let stdout = child
        .stdout
        .take()
        .context("El comando no abrió su salida estándar")?;
    let mut output = Vec::new();
    stdout
        .take((max_bytes + 1) as u64)
        .read_to_end(&mut output)
        .with_context(|| format!("No se pudo leer la salida de {program}"))?;
    if output.len() > max_bytes {
        let _ = child.kill();
        let _ = child.wait();
        bail!("La salida de {program} excede el límite de {max_bytes} bytes");
    }
    let status = child
        .wait()
        .with_context(|| format!("No se pudo esperar a {program}"))?;
    if !status.success() {
        bail!("{program} terminó con error: {status}");
    }
    Ok(output)
}

fn send_capture(
    mime_type: &str,
    data: &[u8],
    primary_selection: bool,
    sensitive_hint: bool,
) -> Result<()> {
    let mut stream = UnixStream::connect(clipboard_socket_path()?)
        .context("El gestor de portapapeles no está disponible")?;
    serde_json::to_writer(
        &mut stream,
        &CaptureHeader {
            mime_type: mime_type.to_string(),
            size: data.len(),
            primary_selection,
            sensitive_hint,
        },
    )
    .context("No se pudo serializar el cambio del portapapeles")?;
    stream
        .write_all(b"\n")
        .context("No se pudo enviar el encabezado")?;
    stream
        .write_all(data)
        .context("No se pudo enviar el contenido copiado")?;
    Ok(())
}

fn handle_capture(
    stream: UnixStream,
    history: Arc<Mutex<ClipboardHistory>>,
    event_tx: Sender<UiEvent>,
    sync_primary_selection: Arc<AtomicBool>,
    filter_sensitive_content: Arc<AtomicBool>,
) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("No se pudo leer el encabezado del portapapeles")?;
    let header: CaptureHeader =
        serde_json::from_str(&line).context("El encabezado del portapapeles no es válido")?;
    if header.size > MAX_CLIPBOARD_BYTES || header.mime_type.len() > 256 {
        bail!("El contenido del portapapeles excede los límites permitidos");
    }
    let mut data = vec![0; header.size];
    reader
        .read_exact(&mut data)
        .context("El contenido del portapapeles llegó incompleto")?;

    let sync_primary = sync_primary_selection.load(Ordering::Relaxed);
    if header.primary_selection && !sync_primary {
        return Ok(());
    }

    if header.sensitive_hint && data.is_empty() {
        event_tx
            .send_blocking(UiEvent::SensitiveSkipped)
            .map_err(|_| anyhow::anyhow!("La interfaz del historial ya no está disponible"))?;
        return Ok(());
    }

    if filter_sensitive_content.load(Ordering::Relaxed)
        && (header.sensitive_hint || is_sensitive_content(&header.mime_type, &data))
    {
        event_tx
            .send_blocking(UiEvent::SensitiveSkipped)
            .map_err(|_| anyhow::anyhow!("La interfaz del historial ya no está disponible"))?;
        if sync_primary {
            let target_primary = !header.primary_selection;
            let mut state = history
                .lock()
                .map_err(|_| anyhow::anyhow!("El historial del portapapeles está bloqueado"))?;
            mark_manager_payload(&mut state, &header.mime_type, &data);
            drop(state);
            if let Err(error) = copy_to_selection(&header.mime_type, &data, target_primary) {
                error!("No se pudo sincronizar la selección sensible: {error:#}");
            }
        }
        return Ok(());
    }

    let item = history
        .lock()
        .map_err(|_| anyhow::anyhow!("El historial del portapapeles está bloqueado"))?
        .capture(header.mime_type.clone(), data.clone());
    if let Some(item) = item {
        let save_result = {
            let history = history
                .lock()
                .map_err(|_| anyhow::anyhow!("El historial del portapapeles está bloqueado"))?;
            save_history(&history_path()?, &history)
        };
        if let Err(error) = save_result {
            error!("No se pudo persistir el historial: {error:#}");
        }
        event_tx
            .send_blocking(UiEvent::Refresh)
            .map_err(|_| anyhow::anyhow!("La interfaz del historial ya no está disponible"))?;
        mark_manager_copy(&history, &item)?;
        let copy_result = if sync_primary {
            let target_primary = !header.primary_selection;
            copy_to_selection(&item.mime_type, &item.data, target_primary)
        } else if header.primary_selection {
            copy_to_selection(&item.mime_type, &item.data, false)
        } else {
            Ok(())
        };
        if let Err(error) = copy_result {
            error!("No se pudo sincronizar la selección capturada: {error:#}");
        }
    }
    Ok(())
}

fn mark_manager_copy(history: &Arc<Mutex<ClipboardHistory>>, item: &ClipboardItem) -> Result<()> {
    let mut state = history
        .lock()
        .map_err(|_| anyhow::anyhow!("El historial del portapapeles está bloqueado"))?;
    if let Some(index) = state
        .items
        .iter()
        .position(|existing| existing.id == item.id)
    {
        let recent = state
            .items
            .remove(index)
            .expect("El elemento se encontró arriba");
        state.items.push_front(recent);
    }
    mark_manager_payload(&mut state, &item.mime_type, &item.data);
    Ok(())
}

fn mark_manager_payload(history: &mut ClipboardHistory, mime_type: &str, data: &[u8]) {
    history.manager_copy = Some((
        mime_type.to_string(),
        data.to_vec(),
        Instant::now() + Duration::from_secs(10),
    ));
}

fn copy_to_selection(mime_type: &str, data: &[u8], primary_selection: bool) -> Result<()> {
    let mut command = Command::new("wl-copy");
    if primary_selection {
        command.arg("--primary");
    }
    let mut child = command
        .args(["--type", mime_type])
        .stdin(Stdio::piped())
        .spawn()
        .context("No se pudo iniciar wl-copy; instala wl-clipboard")?;
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        bail!("wl-copy no abrió la entrada de datos");
    };
    stdin
        .write_all(data)
        .context("No se pudo enviar el contenido a wl-copy")?;
    drop(stdin);
    let status = child.wait().context("No se pudo esperar a wl-copy")?;
    if !status.success() {
        bail!("wl-copy terminó con error: {status}");
    }
    Ok(())
}

fn start_compositor_listener(event_tx: Sender<UiEvent>) -> Result<()> {
    thread::Builder::new()
        .name("clipboard-compositor-ipc".to_string())
        .spawn(move || loop {
            match UnixStream::connect(ipc_socket_path()) {
                Ok(stream) => {
                    let reader = BufReader::new(stream);
                    for line in reader.lines() {
                        let Ok(line) = line else {
                            break;
                        };
                        if matches!(
                            serde_json::from_str::<IpcMessage>(&line),
                            Ok(IpcMessage::ToggleClipboardHistory)
                        ) {
                            if event_tx.send_blocking(UiEvent::ToggleWindow).is_err() {
                                return;
                            }
                        }
                    }
                    warn!("Se desconectó el canal IPC del compositor");
                }
                Err(error) => warn!("No se pudo conectar con el compositor: {error}"),
            }
            thread::sleep(Duration::from_secs(1));
        })
        .context("No se pudo iniciar el receptor de atajos")?;
    Ok(())
}

fn build_history_ui(
    application: &Application,
    history: Arc<Mutex<ClipboardHistory>>,
    history_path: PathBuf,
    settings_path: PathBuf,
    sync_primary_selection: Arc<AtomicBool>,
    filter_sensitive_content: Arc<AtomicBool>,
) -> HistoryUi {
    let window = ApplicationWindow::builder()
        .application(application)
        .title("Historial del portapapeles")
        .default_width(460)
        .default_height(540)
        .build();
    window.connect_close_request(|window| {
        window.set_visible(false);
        glib::Propagation::Stop
    });
    let layout = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    let heading = Label::builder()
        .label("Historial del portapapeles")
        .xalign(0.0)
        .css_classes(["title-2"])
        .build();
    layout.append(&heading);

    let search = Entry::builder()
        .placeholder_text("Buscar en el historial")
        .hexpand(true)
        .build();
    layout.append(&search);

    let sync_primary_button = CheckButton::with_label("Sincronizar con la selección primaria");
    sync_primary_button.set_active(sync_primary_selection.load(Ordering::Relaxed));
    layout.append(&sync_primary_button);
    let filter_sensitive_button =
        CheckButton::with_label("Omitir contraseñas y credenciales detectadas del historial");
    filter_sensitive_button.set_active(filter_sensitive_content.load(Ordering::Relaxed));
    layout.append(&filter_sensitive_button);
    let privacy_note = Label::builder()
        .label("El historial se guarda localmente en este dispositivo. El filtro automático detecta señales comunes de credenciales, pero no puede identificar todos los secretos. Borrar historial elimina también la copia persistente.")
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label", "caption"])
        .build();
    layout.append(&privacy_note);

    let list = ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .build();
    let scroller = ScrolledWindow::builder().child(&list).vexpand(true).build();
    layout.append(&scroller);

    let status = Label::builder()
        .label("Selecciona un elemento para copiarlo de nuevo.")
        .xalign(0.0)
        .wrap(true)
        .build();
    layout.append(&status);

    let clear_button = Button::with_label("Borrar historial");
    layout.append(&clear_button);
    window.set_child(Some(&layout));

    let ui = HistoryUi {
        window: window.clone(),
        search: search.clone(),
        list: list.clone(),
        status: status.clone(),
        history: history.clone(),
        history_path,
        settings_path,
        sync_primary_selection: sync_primary_selection.clone(),
        filter_sensitive_content: filter_sensitive_content.clone(),
    };
    let ui_search = ui.clone();
    search.connect_changed(move |_| refresh_history(&ui_search));

    let ui_enter = ui.clone();
    search.connect_activate(move |_| {
        if let Some(item) = filtered_items(&ui_enter).into_iter().next() {
            restore_clipboard_item(&ui_enter, &item);
        }
    });

    let ui_clear = ui.clone();
    clear_button.connect_clicked(move |_| match ui_clear.history.lock() {
        Ok(mut history) => {
            history.items.clear();
            history.manager_copy = None;
            let save_result = save_history(&ui_clear.history_path, &history);
            drop(history);
            match save_result {
                Ok(()) => refresh_history(&ui_clear),
                Err(error) => {
                    refresh_history(&ui_clear);
                    ui_clear
                        .status
                        .set_label(&format!("No se pudo guardar el historial vacío: {error:#}"));
                    error!("No se pudo persistir el borrado del historial: {error:#}");
                }
            }
        }
        Err(error) => {
            ui_clear
                .status
                .set_label("No se pudo acceder al historial.");
            error!("El historial del portapapeles está bloqueado: {error}");
        }
    });

    let ui_sync = ui.clone();
    sync_primary_button.connect_toggled(move |button| {
        let enabled = button.is_active();
        ui_sync
            .sync_primary_selection
            .store(enabled, Ordering::Relaxed);
        match save_settings(
            &ui_sync.settings_path,
            ClipboardSettings {
                sync_primary_selection: enabled,
                filter_sensitive_content: ui_sync.filter_sensitive_content.load(Ordering::Relaxed),
            },
        ) {
            Ok(()) => ui_sync.status.set_label(if enabled {
                "La selección primaria y el portapapeles ahora se sincronizan."
            } else {
                "La sincronización con la selección primaria está desactivada."
            }),
            Err(error) => {
                ui_sync.status.set_label(&format!(
                    "No se pudo guardar la preferencia de sincronización: {error:#}"
                ));
                error!("No se pudo guardar la preferencia del portapapeles: {error:#}");
            }
        }
    });

    let ui_filter = ui.clone();
    filter_sensitive_button.connect_toggled(move |button| {
        let enabled = button.is_active();
        ui_filter
            .filter_sensitive_content
            .store(enabled, Ordering::Relaxed);
        match save_settings(
            &ui_filter.settings_path,
            ClipboardSettings {
                sync_primary_selection: ui_filter.sync_primary_selection.load(Ordering::Relaxed),
                filter_sensitive_content: enabled,
            },
        ) {
            Ok(()) => ui_filter.status.set_label(if enabled {
                "El filtro automático está activo para el nuevo contenido."
            } else {
                "Aviso: el contenido sensible nuevo puede guardarse en el historial."
            }),
            Err(error) => {
                ui_filter.status.set_label(&format!(
                    "No se pudo guardar la preferencia del filtro: {error:#}"
                ));
                error!("No se pudo guardar la preferencia del filtro sensible: {error:#}");
            }
        }
    });

    window.set_visible(false);
    refresh_history(&ui);
    ui
}

fn refresh_history(ui: &HistoryUi) {
    while let Some(child) = ui.list.first_child() {
        ui.list.remove(&child);
    }
    let items = filtered_items(ui);

    ui.status.set_label(if items.is_empty() {
        "El historial está vacío. Copia texto, una imagen o archivos para empezar."
    } else {
        "Selecciona un elemento para copiarlo de nuevo."
    });

    for item in items {
        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(4)
            .margin_end(4)
            .build();
        let content = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(3)
            .hexpand(true)
            .build();
        let preview = Label::builder()
            .label(item_preview(&item))
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(52)
            .build();
        content.append(&preview);
        let mime = Label::builder()
            .label(&item.mime_type)
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .build();
        content.append(&mime);

        let copy_button = Button::with_label("Copiar");
        let item_for_copy = item.clone();
        let ui_for_copy = ui.clone();
        copy_button.connect_clicked(move |_| {
            restore_clipboard_item(&ui_for_copy, &item_for_copy);
        });

        let pin_button = Button::with_label(if item.pinned { "★" } else { "☆" });
        let history_for_pin = ui.history.clone();
        let ui_for_pin = ui.clone();
        let item_id = item.id;
        pin_button.connect_clicked(move |_| {
            let mut save_error = None;
            match history_for_pin.lock() {
                Ok(mut history) => {
                    if let Some(item) = history.items.iter_mut().find(|item| item.id == item_id) {
                        item.pinned = !item.pinned;
                    }
                    if let Err(error) = save_history(&ui_for_pin.history_path, &history) {
                        error!("No se pudo persistir el estado fijado: {error:#}");
                        save_error = Some(error);
                    }
                }
                Err(error) => {
                    error!("El historial del portapapeles está bloqueado: {error}");
                    save_error = Some(anyhow::anyhow!("El historial está bloqueado: {error}"));
                }
            }
            refresh_history(&ui_for_pin);
            if let Some(error) = save_error {
                ui_for_pin
                    .status
                    .set_label(&format!("No se pudo guardar el elemento fijado: {error:#}"));
            }
        });

        row.append(&content);
        row.append(&copy_button);
        row.append(&pin_button);
        ui.list.append(&row);
    }
}

fn filtered_items(ui: &HistoryUi) -> Vec<ClipboardItem> {
    let query = ui.search.text().to_string().to_lowercase();
    ui.history
        .lock()
        .unwrap()
        .items
        .iter()
        .filter(|item| {
            let preview = item_preview(item);
            query.is_empty()
                || preview.to_lowercase().contains(&query)
                || item.mime_type.to_lowercase().contains(&query)
        })
        .cloned()
        .collect()
}

fn restore_clipboard_item(ui: &HistoryUi, item: &ClipboardItem) {
    if let Err(error) = mark_manager_copy(&ui.history, item) {
        ui.status
            .set_label(&format!("No se pudo preparar el portapapeles: {error:#}"));
        error!("No se pudo preparar la restauración del historial: {error:#}");
        return;
    }
    if let Ok(history) = ui.history.lock() {
        if let Err(error) = save_history(&ui.history_path, &history) {
            ui.status
                .set_label(&format!("No se pudo guardar el historial: {error:#}"));
            error!("No se pudo persistir el historial al restaurar: {error:#}");
        }
    }
    match copy_to_selection(&item.mime_type, &item.data, false) {
        Ok(()) => {
            ui.window.set_visible(false);
            info!("Elemento del historial copiado al portapapeles");
        }
        Err(error) => {
            ui.status
                .set_label(&format!("No se pudo copiar: {error:#}"));
            error!("No se pudo restaurar un elemento del historial: {error:#}");
        }
    }
}

fn item_preview(item: &ClipboardItem) -> String {
    if item.mime_type.starts_with("image/") {
        return format!(
            "Imagen {} ({} KiB)",
            item.mime_type.trim_start_matches("image/"),
            item.data.len().div_ceil(1024)
        );
    }
    match item.mime_type.as_str() {
        "application/pdf" => format!("Documento PDF ({} KiB)", item.data.len().div_ceil(1024)),
        "text/uri-list" => String::from_utf8_lossy(&item.data)
            .lines()
            .find(|line| !line.trim().is_empty() && !line.starts_with('#'))
            .unwrap_or("Lista de archivos vacía")
            .to_string(),
        _ => {
            if !item.mime_type.starts_with("text/")
                && !matches!(
                    item.mime_type.as_str(),
                    "application/rtf"
                        | "application/vnd.oasis.opendocument.text"
                        | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                        | "application/vnd.ms-word"
                )
            {
                return format!(
                    "{} ({} KiB)",
                    item.mime_type,
                    item.data.len().div_ceil(1024)
                );
            }
            let text = String::from_utf8_lossy(&item.data);
            let first_line = text.lines().next().unwrap_or_default();
            let preview: String = first_line.chars().take(160).collect();
            if preview.is_empty() {
                "(contenido vacío)".to_string()
            } else {
                preview
            }
        }
    }
}

fn poll_ui_events(receiver: Receiver<UiEvent>, ui_state: Arc<Mutex<Option<HistoryUi>>>) {
    glib::timeout_add_local(Duration::from_millis(100), move || {
        while let Ok(event) = receiver.try_recv() {
            let mut ui_guard = ui_state.lock().unwrap();
            if ui_guard.is_none() {
                continue;
            }
            let ui = ui_guard.as_mut().unwrap();
            match event {
                UiEvent::Refresh => refresh_history(ui),
                UiEvent::SensitiveSkipped => ui
                    .status
                    .set_label("Se omitió del historial contenido que parece sensible."),
                UiEvent::ToggleWindow => {
                    if ui.window.is_visible() {
                        ui.window.set_visible(false);
                    } else {
                        ui.window.present();
                        ui.search.grab_focus();
                        refresh_history(ui);
                    }
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::{
        is_sensitive_content, is_sensitive_mime_hint, load_history, mark_manager_copy,
        save_history, select_mime_type, ClipboardHistory, ClipboardSettings, MAX_HISTORY_ITEMS,
    };
    use std::{
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn picks_supported_types_in_priority_order() {
        let types = "text/plain\nimage/png\ntext/uri-list\n";
        assert_eq!(select_mime_type(types).as_deref(), Some("image/png"));
    }

    #[test]
    fn supports_additional_image_and_text_mime_types() {
        assert_eq!(
            select_mime_type("image/jpeg\ntext/plain").as_deref(),
            Some("image/jpeg")
        );
        assert_eq!(
            select_mime_type("text/calendar\n").as_deref(),
            Some("text/calendar")
        );
    }

    #[test]
    fn old_settings_keep_sensitive_filter_enabled_by_default() {
        let settings: ClipboardSettings = toml::from_str("sync_primary_selection = true").unwrap();
        assert!(settings.sync_primary_selection);
        assert!(settings.filter_sensitive_content);
    }

    #[test]
    fn detects_sensitive_mime_hints_and_common_credential_text() {
        assert!(is_sensitive_mime_hint(
            "application/x-kde-passwordManagerHint"
        ));
        assert!(is_sensitive_content(
            "text/plain",
            b"password = \"correct-horse-battery-staple\""
        ));
        assert!(is_sensitive_content(
            "text/plain",
            b"-----BEGIN OPENSSH PRIVATE KEY-----"
        ));
        assert!(is_sensitive_content(
            "text/plain",
            br#"{"username":"alice","userPassword":"high-entropy-secret"}"#
        ));
        assert!(is_sensitive_content(
            "text/plain",
            b"https://alice:very-secret-pass@example.test/"
        ));
        assert!(!is_sensitive_content(
            "text/plain",
            b"Remember to update your password settings."
        ));
        assert!(!is_sensitive_content("image/png", &[0, 159, 146, 150]));
    }

    #[test]
    fn history_deduplicates_and_caps_unpinned_items() {
        let mut history = ClipboardHistory::default();
        history.capture("text/plain".to_string(), b"first".to_vec());
        history.capture("text/plain".to_string(), b"first".to_vec());
        assert_eq!(history.items.len(), 1);

        for index in 0..=MAX_HISTORY_ITEMS {
            history.capture("text/plain".to_string(), index.to_string().into_bytes());
        }
        assert_eq!(history.items.len(), MAX_HISTORY_ITEMS);
    }

    #[test]
    fn restored_item_moves_to_front_without_creating_a_copy_loop() {
        let history = Arc::new(Mutex::new(ClipboardHistory::default()));
        let first = history
            .lock()
            .unwrap()
            .capture("text/plain".to_string(), b"first".to_vec())
            .unwrap();
        history
            .lock()
            .unwrap()
            .capture("text/plain".to_string(), b"second".to_vec());

        mark_manager_copy(&history, &first).unwrap();
        let mut history = history.lock().unwrap();
        assert_eq!(history.items.front().unwrap().data, b"first");
        assert!(history
            .capture("text/plain".to_string(), b"first".to_vec())
            .is_none());
    }

    #[test]
    fn persisted_history_round_trips_items_and_pins() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "malus-de-clipboard-test-{}-{unique}",
            std::process::id()
        ));
        let path = directory.join("history.dat");
        let mut history = ClipboardHistory::default();
        let item = history
            .capture("text/plain".to_string(), b"persisted entry".to_vec())
            .unwrap();
        history.items.front_mut().unwrap().pinned = true;

        save_history(&path, &history).unwrap();
        let loaded = load_history(&path).unwrap();
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items.front().unwrap().data, item.data);
        assert!(loaded.items.front().unwrap().pinned);
        assert_eq!(loaded.next_id, item.id);

        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
