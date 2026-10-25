use anyhow::{bail, Context, Result};
use async_channel::{Receiver, Sender};
use gio::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, Entry, Label, ListBox, Orientation,
    ScrolledWindow,
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
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tracing::{error, info, warn};

const MAX_HISTORY_ITEMS: usize = 50;
const MAX_CLIPBOARD_BYTES: usize = 10 * 1024 * 1024;
const CLIPBOARD_WATCH_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
struct ClipboardItem {
    id: u64,
    mime_type: String,
    data: Vec<u8>,
    pinned: bool,
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

        if self.items.len() >= MAX_HISTORY_ITEMS {
            let oldest_unpinned = self
                .items
                .iter()
                .rposition(|item| !item.pinned)
                .unwrap_or_else(|| {
                    warn!("Historial lleno: se quitará el elemento fijado más antiguo");
                    self.items.len() - 1
                });
            self.items.remove(oldest_unpinned);
        }

        self.next_id = self.next_id.wrapping_add(1).max(1);
        let item = ClipboardItem {
            id: self.next_id,
            mime_type,
            data,
            pinned: false,
        };
        self.items.push_front(item.clone());
        Some(item)
    }
}

#[derive(Deserialize, Serialize)]
struct CaptureHeader {
    mime_type: String,
    size: usize,
}

enum UiEvent {
    Refresh,
    ToggleWindow,
}

#[derive(Clone)]
struct HistoryUi {
    window: ApplicationWindow,
    search: Entry,
    list: ListBox,
    status: Label,
    history: Arc<Mutex<ClipboardHistory>>,
}

fn main() -> Result<()> {
    init_logging("malus_de_clipboard");

    match std::env::args().nth(1).as_deref() {
        Some("--capture") => capture_clipboard_change(),
        Some("--help") => {
            println!("malus-de-clipboard\n\nComponente de historial del portapapeles de Malus DE.");
            Ok(())
        }
        Some(argument) => bail!("Argumento no reconocido: {argument}"),
        None => run_manager(),
    }
}

fn run_manager() -> Result<()> {
    let history = Arc::new(Mutex::new(ClipboardHistory::default()));
    let (event_tx, event_rx) = async_channel::unbounded();

    let app = Application::builder()
        .application_id("org.malus.de.clipboard")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let _hold_guard = app.hold();

    let ui_state: Arc<Mutex<Option<HistoryUi>>> = Arc::new(Mutex::new(None));
    let ui_state_activate = ui_state.clone();
    let history_activate = history.clone();
    app.connect_activate(move |application| {
        if ui_state_activate.lock().unwrap().is_some() {
            return;
        }
        let ui = build_history_ui(application, history_activate.clone());
        *ui_state_activate.lock().unwrap() = Some(ui);
    });

    start_clipboard_service(history.clone(), event_tx.clone())?;
    start_compositor_listener(event_tx)?;
    poll_ui_events(event_rx, ui_state);

    info!("Gestor de portapapeles iniciado; historial en memoria de hasta {MAX_HISTORY_ITEMS} elementos.");
    app.run();
    Ok(())
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
    thread::Builder::new()
        .name("clipboard-history-ipc".to_string())
        .spawn(move || {
            for incoming in listener.incoming() {
                match incoming {
                    Ok(stream) => {
                        let history = history_for_listener.clone();
                        let event_tx = event_tx.clone();
                        thread::spawn(move || {
                            if let Err(error) = handle_capture(stream, history, event_tx) {
                                warn!("No se pudo procesar un cambio del portapapeles: {error:#}");
                            }
                        });
                    }
                    Err(error) => error!("Error al aceptar cliente del historial: {error}"),
                }
            }
        })
        .context("No se pudo iniciar el servidor IPC del historial")?;

    start_clipboard_watcher()?;
    Ok(())
}

fn start_clipboard_watcher() -> Result<()> {
    let executable = std::env::current_exe()
        .context("No se pudo localizar el binario del gestor de portapapeles")?;
    thread::Builder::new()
        .name("clipboard-watcher".to_string())
        .spawn(move || loop {
            match Command::new("wl-paste")
                .args([
                    "--watch",
                    executable.to_string_lossy().as_ref(),
                    "--capture",
                ])
                .spawn()
            {
                Ok(mut child) => match child.wait() {
                    Ok(status) => warn!("wl-paste --watch terminó ({status}); se reintentará"),
                    Err(error) => warn!("No se pudo esperar a wl-paste --watch: {error}"),
                },
                Err(error) => {
                    error!("No se pudo iniciar wl-paste --watch; instala wl-clipboard: {error}")
                }
            }
            thread::sleep(CLIPBOARD_WATCH_INTERVAL);
        })
        .context("No se pudo iniciar el monitor del portapapeles")?;
    Ok(())
}

fn capture_clipboard_change() -> Result<()> {
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

    let mime_types = run_bounded("wl-paste", &["--list-types"], 64 * 1024)?;
    let mime_type = select_mime_type(&String::from_utf8_lossy(&mime_types))
        .context("El elemento del portapapeles no tiene un formato compatible")?;
    let data = match run_bounded(
        "wl-paste",
        &["--no-newline", "--type", &mime_type],
        MAX_CLIPBOARD_BYTES,
    ) {
        Ok(data) => data,
        Err(error) if !watched_data.is_empty() => {
            warn!("No se pudo volver a leer el MIME {mime_type}; se conservará el flujo del monitor: {error:#}");
            watched_data
        }
        Err(error) => return Err(error),
    };
    send_capture(&mime_type, &data)?;
    Ok(())
}

fn select_mime_type(types: &str) -> Option<String> {
    let offered: Vec<&str> = types.lines().map(str::trim).collect();
    [
        "image/png",
        "text/uri-list",
        "text/plain;charset=utf-8",
        "text/plain;charset=UTF-8",
        "text/plain",
        "UTF8_STRING",
        "TEXT",
        "STRING",
        "text/html",
    ]
    .into_iter()
    .find_map(|candidate| {
        offered
            .iter()
            .find(|mime| mime.eq_ignore_ascii_case(candidate))
            .map(|mime| (*mime).to_string())
    })
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

fn send_capture(mime_type: &str, data: &[u8]) -> Result<()> {
    let mut stream = UnixStream::connect(clipboard_socket_path()?)
        .context("El gestor de portapapeles no está disponible")?;
    serde_json::to_writer(
        &mut stream,
        &CaptureHeader {
            mime_type: mime_type.to_string(),
            size: data.len(),
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

    let item = history
        .lock()
        .map_err(|_| anyhow::anyhow!("El historial del portapapeles está bloqueado"))?
        .capture(header.mime_type, data);
    if let Some(item) = item {
        event_tx
            .send_blocking(UiEvent::Refresh)
            .map_err(|_| anyhow::anyhow!("La interfaz del historial ya no está disponible"))?;
        mark_manager_copy(&history, &item)?;
        if let Err(error) = copy_to_clipboard(&item.mime_type, &item.data) {
            error!("No se pudo conservar la selección capturada: {error:#}");
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
    state.manager_copy = Some((
        item.mime_type.clone(),
        item.data.clone(),
        Instant::now() + Duration::from_secs(10),
    ));
    Ok(())
}

fn copy_to_clipboard(mime_type: &str, data: &[u8]) -> Result<()> {
    let mut child = Command::new("wl-copy")
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

fn build_history_ui(application: &Application, history: Arc<Mutex<ClipboardHistory>>) -> HistoryUi {
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
    clear_button.connect_clicked(move |_| {
        ui_clear.history.lock().unwrap().items.clear();
        refresh_history(&ui_clear);
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
            if let Some(item) = history_for_pin
                .lock()
                .unwrap()
                .items
                .iter_mut()
                .find(|item| item.id == item_id)
            {
                item.pinned = !item.pinned;
            }
            refresh_history(&ui_for_pin);
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
    match copy_to_clipboard(&item.mime_type, &item.data) {
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
    match item.mime_type.as_str() {
        "image/png" => format!("Imagen PNG ({} KiB)", item.data.len().div_ceil(1024)),
        "text/uri-list" => String::from_utf8_lossy(&item.data)
            .lines()
            .find(|line| !line.trim().is_empty() && !line.starts_with('#'))
            .unwrap_or("Lista de archivos vacía")
            .to_string(),
        _ => {
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
    use super::{mark_manager_copy, select_mime_type, ClipboardHistory, MAX_HISTORY_ITEMS};
    use std::sync::{Arc, Mutex};

    #[test]
    fn picks_supported_types_in_priority_order() {
        let types = "text/plain\nimage/png\ntext/uri-list\n";
        assert_eq!(select_mime_type(types).as_deref(), Some("image/png"));
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
}
