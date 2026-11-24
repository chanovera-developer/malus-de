use anyhow::{bail, Context, Result};
use calloop::channel::{self, Event as ChannelEvent, Sender as ChannelSender};
use calloop::EventLoop;
use calloop_wayland_source::WaylandSource;
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tracing::{debug, info, warn};
use wayland_client::{
    backend::ObjectId,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_registry, wl_seat::WlSeat},
    Connection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
    zwlr_data_control_source_v1::{self, ZwlrDataControlSourceV1},
};

use crate::{is_sensitive_mime_hint, select_mime_type_from_slice, MAX_CLIPBOARD_BYTES};

const MAX_OFFERED_MIMES: usize = 256;
const PIPE_IO_TIMEOUT: Duration = Duration::from_secs(5);

pub struct CapturedSelection {
    pub mime_type: String,
    pub data: Vec<u8>,
    pub primary_selection: bool,
    pub sensitive_hint: bool,
}

#[derive(Clone)]
pub struct DataControlHandle {
    cmd_tx: ChannelSender<DataControlCmd>,
}

impl DataControlHandle {
    pub fn set_selection(
        &self,
        mime_type: &str,
        data: &[u8],
        primary_selection: bool,
    ) -> Result<()> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.cmd_tx
            .send(DataControlCmd::SetSelection {
                mime_type: mime_type.to_string(),
                data: Arc::from(data),
                primary_selection,
                reply: reply_tx,
            })
            .map_err(|_| anyhow::anyhow!("El canal Wayland de portapapeles no está disponible"))?;
        reply_rx
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| anyhow::anyhow!("Tiempo de espera agotado al actualizar el portapapeles"))?
    }
}

enum DataControlCmd {
    SetSelection {
        mime_type: String,
        data: Arc<[u8]>,
        primary_selection: bool,
        reply: mpsc::SyncSender<Result<()>>,
    },
}

struct ActiveSource {
    source: ZwlrDataControlSourceV1,
    offered_mimes: Vec<String>,
    data: Arc<[u8]>,
}

type CaptureCallback = Arc<dyn Fn(CapturedSelection, DataControlHandle) + Send + Sync>;

struct DataControlState {
    conn: Connection,
    qh: QueueHandle<Self>,
    handle: DataControlHandle,
    manager: ZwlrDataControlManagerV1,
    device: ZwlrDataControlDeviceV1,
    sync_primary_selection: Arc<AtomicBool>,
    filter_sensitive_content: Arc<AtomicBool>,
    on_capture: CaptureCallback,

    /// MIME types accumulated for each newly announced offer until Selection / PrimarySelection.
    pending_offers: HashMap<ObjectId, Vec<String>>,
    /// Keep the latest offer alive until the next selection replaces it.
    current_selection_offer: Option<ZwlrDataControlOfferV1>,
    current_primary_offer: Option<ZwlrDataControlOfferV1>,

    /// Sources owned by this clipboard manager.
    sources: HashMap<ObjectId, ActiveSource>,
    own_selection_id: Option<ObjectId>,
    own_primary_id: Option<ObjectId>,
    /// Number of expected Selection/PrimarySelection echoes caused by our own set_selection calls.
    pending_own_selection_echoes: u32,
    pending_own_primary_echoes: u32,
    finished: bool,
}

impl DataControlState {
    fn handle_set_selection(
        &mut self,
        mime_type: String,
        data: Arc<[u8]>,
        primary_selection: bool,
    ) -> Result<()> {
        if self.finished {
            bail!("El dispositivo zwlr_data_control_v1 fue finalizado por el compositor");
        }
        if primary_selection && self.device.version() < 2 {
            debug!(
                "El compositor expone zwlr_data_control_manager_v1 v{}; se omite set_primary_selection",
                self.device.version()
            );
            return Ok(());
        }

        let source = self.manager.create_data_source(&self.qh, ());
        let offered_mimes = expanded_offer_mimes(&mime_type);
        for mime in &offered_mimes {
            source.offer(mime.clone());
        }

        let source_id = source.id();
        self.sources.insert(
            source_id.clone(),
            ActiveSource {
                source: source.clone(),
                offered_mimes,
                data,
            },
        );

        if primary_selection {
            self.own_primary_id = Some(source_id);
            self.pending_own_primary_echoes = self.pending_own_primary_echoes.saturating_add(1);
            self.device.set_primary_selection(Some(&source));
        } else {
            self.own_selection_id = Some(source_id);
            self.pending_own_selection_echoes = self.pending_own_selection_echoes.saturating_add(1);
            self.device.set_selection(Some(&source));
        }

        self.conn
            .flush()
            .context("No se pudo enviar la selección al compositor Wayland")?;
        Ok(())
    }

    fn handle_selection_offer(
        &mut self,
        offer: Option<ZwlrDataControlOfferV1>,
        primary_selection: bool,
    ) {
        let prev_offer = if primary_selection {
            self.current_primary_offer.take()
        } else {
            self.current_selection_offer.take()
        };
        if let Some(old) = prev_offer {
            self.pending_offers.remove(&old.id());
            old.destroy();
        }

        let Some(offer) = offer else {
            return;
        };

        let offer_id = offer.id();
        let offered_mimes = self.pending_offers.remove(&offer_id).unwrap_or_default();

        // Discard our own selection echo without opening a pipe or copying bytes.
        if self.consume_own_echo(&offered_mimes, primary_selection) {
            if primary_selection {
                self.current_primary_offer = Some(offer);
            } else {
                self.current_selection_offer = Some(offer);
            }
            return;
        }

        let sync_primary = self.sync_primary_selection.load(Ordering::Relaxed);
        if primary_selection && !sync_primary {
            offer.destroy();
            return;
        }

        let offered_refs: Vec<&str> = offered_mimes.iter().map(String::as_str).collect();
        let sensitive_hint = offered_refs.iter().any(|mime| is_sensitive_mime_hint(mime));
        let selected_mime = select_mime_type_from_slice(&offered_refs);

        let Some(mime_type) = selected_mime else {
            if sensitive_hint {
                let hint = offered_refs
                    .iter()
                    .copied()
                    .find(|mime| is_sensitive_mime_hint(mime))
                    .unwrap_or("application/x-secret")
                    .to_string();
                let on_capture = self.on_capture.clone();
                let handle = self.handle.clone();
                on_capture(
                    CapturedSelection {
                        mime_type: hint,
                        data: Vec::new(),
                        primary_selection,
                        sensitive_hint: true,
                    },
                    handle,
                );
            }
            offer.destroy();
            return;
        };

        // If sensitive content is being filtered and primary sync is off, avoid even reading the
        // secret payload over the Wayland pipe.
        if sensitive_hint
            && self.filter_sensitive_content.load(Ordering::Relaxed)
            && !sync_primary
        {
            let on_capture = self.on_capture.clone();
            let handle = self.handle.clone();
            on_capture(
                CapturedSelection {
                    mime_type,
                    data: Vec::new(),
                    primary_selection,
                    sensitive_hint: true,
                },
                handle,
            );
            offer.destroy();
            return;
        }

        let Ok((read_fd, write_fd)) = create_cloexec_pipe() else {
            warn!("No se pudo crear tubería para leer la oferta de portapapeles");
            offer.destroy();
            return;
        };

        offer.receive(mime_type.clone(), write_fd.as_fd());
        drop(write_fd);
        let _ = self.conn.flush();

        if primary_selection {
            self.current_primary_offer = Some(offer);
        } else {
            self.current_selection_offer = Some(offer);
        }

        let on_capture = self.on_capture.clone();
        let handle = self.handle.clone();
        thread::Builder::new()
            .name("clipboard-offer-read".to_string())
            .spawn(move || {
                match read_fd_bounded(read_fd, MAX_CLIPBOARD_BYTES, PIPE_IO_TIMEOUT) {
                    Ok(Some(data)) => {
                        on_capture(
                            CapturedSelection {
                                mime_type,
                                data,
                                primary_selection,
                                sensitive_hint,
                            },
                            handle,
                        );
                    }
                    Ok(None) => {
                        warn!(
                            "Se omitió contenido del portapapeles mayor a {} MiB",
                            MAX_CLIPBOARD_BYTES / (1024 * 1024)
                        );
                    }
                    Err(error) => {
                        debug!("No se pudo leer la oferta del portapapeles ({mime_type}): {error:#}");
                    }
                }
            })
            .ok();
    }

    fn consume_own_echo(&mut self, offered_mimes: &[String], primary_selection: bool) -> bool {
        let (own_id, pending_echoes) = if primary_selection {
            (&self.own_primary_id, &mut self.pending_own_primary_echoes)
        } else {
            (&self.own_selection_id, &mut self.pending_own_selection_echoes)
        };
        if *pending_echoes == 0 {
            return false;
        }
        let Some(source_id) = own_id.as_ref() else {
            *pending_echoes = 0;
            return false;
        };
        let Some(active) = self.sources.get(source_id) else {
            *pending_echoes = 0;
            return false;
        };
        if offered_mimes.len() == active.offered_mimes.len()
            && offered_mimes
                .iter()
                .all(|m| active.offered_mimes.iter().any(|own| own == m))
        {
            *pending_echoes -= 1;
            true
        } else {
            false
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for DataControlState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for DataControlState {
    fn event(
        _state: &mut Self,
        _proxy: &WlSeat,
        _event: <WlSeat as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlManagerV1, ()> for DataControlState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrDataControlManagerV1,
        _event: <ZwlrDataControlManagerV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for DataControlState {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id } => {
                // Prevent unbounded growth if a buggy compositor announces offers without Selection.
                if state.pending_offers.len() > 64 {
                    state.pending_offers.clear();
                }
                state.pending_offers.insert(id.id(), Vec::new());
            }
            zwlr_data_control_device_v1::Event::Selection { id } => {
                state.handle_selection_offer(id, false);
            }
            zwlr_data_control_device_v1::Event::PrimarySelection { id } => {
                state.handle_selection_offer(id, true);
            }
            zwlr_data_control_device_v1::Event::Finished => {
                warn!("El dispositivo zwlr_data_control_v1 recibió Finished");
                state.finished = true;
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(DataControlState, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for DataControlState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            if let Some(list) = state.pending_offers.get_mut(&proxy.id()) {
                if list.len() < MAX_OFFERED_MIMES {
                    list.push(mime_type);
                }
            }
        }
    }
}

impl Dispatch<ZwlrDataControlSourceV1, ()> for DataControlState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_source_v1::Event::Send { mime_type: _, fd } => {
                let Some(active) = state.sources.get(&proxy.id()) else {
                    return;
                };
                let data = active.data.clone();
                thread::Builder::new()
                    .name("clipboard-source-write".to_string())
                    .spawn(move || {
                        let _ = write_fd_with_timeout(fd, &data, PIPE_IO_TIMEOUT);
                    })
                    .ok();
            }
            zwlr_data_control_source_v1::Event::Cancelled => {
                let id = proxy.id();
                if state.own_selection_id.as_ref() == Some(&id) {
                    state.own_selection_id = None;
                    state.pending_own_selection_echoes = 0;
                }
                if state.own_primary_id.as_ref() == Some(&id) {
                    state.own_primary_id = None;
                    state.pending_own_primary_echoes = 0;
                }
                if let Some(removed) = state.sources.remove(&id) {
                    removed.source.destroy();
                } else {
                    proxy.destroy();
                }
            }
            _ => {}
        }
    }
}

/// Attempts to start the native `zwlr_data_control_manager_v1` Wayland backend.
///
/// Returns `Ok(None)` if the compositor does not advertise `zwlr_data_control_manager_v1`
/// or `wl_seat` (allowing the caller to fall back cleanly to `wl-paste`/`wl-copy`).
pub fn try_start_data_control<F>(
    sync_primary_selection: Arc<AtomicBool>,
    filter_sensitive_content: Arc<AtomicBool>,
    on_capture: F,
) -> Result<Option<DataControlHandle>>
where
    F: Fn(CapturedSelection, DataControlHandle) + Send + Sync + 'static,
{
    let conn = match Connection::connect_to_env() {
        Ok(conn) => conn,
        Err(error) => {
            warn!("No se pudo conectar a Wayland para zwlr_data_control_v1: {error}");
            return Ok(None);
        }
    };

    let (globals, event_queue) = match registry_queue_init::<DataControlState>(&conn) {
        Ok(pair) => pair,
        Err(error) => {
            warn!("No se pudo consultar el registro Wayland: {error}");
            return Ok(None);
        }
    };
    let qh = event_queue.handle();

    let manager = match globals.bind::<ZwlrDataControlManagerV1, _, _>(&qh, 1..=2, ()) {
        Ok(manager) => manager,
        Err(error) => {
            warn!(
                "El compositor no expone zwlr_data_control_manager_v1 ({error}); se usará respaldo wl-clipboard"
            );
            return Ok(None);
        }
    };
    let seat = match globals.bind::<WlSeat, _, _>(&qh, 1..=9, ()) {
        Ok(seat) => seat,
        Err(error) => {
            warn!("No se encontró wl_seat en Wayland ({error}); se usará respaldo wl-clipboard");
            return Ok(None);
        }
    };

    let device = manager.get_data_device(&seat, &qh, ());
    let manager_version = manager.version();
    conn.flush()
        .context("No se pudo enviar la inicialización de zwlr_data_control_v1")?;

    let (cmd_tx, cmd_rx) = channel::channel::<DataControlCmd>();
    let control_handle = DataControlHandle { cmd_tx };
    let state = DataControlState {
        conn: conn.clone(),
        qh,
        handle: control_handle.clone(),
        manager,
        device,
        sync_primary_selection,
        filter_sensitive_content,
        on_capture: Arc::new(on_capture),
        pending_offers: HashMap::new(),
        current_selection_offer: None,
        current_primary_offer: None,
        sources: HashMap::new(),
        own_selection_id: None,
        own_primary_id: None,
        pending_own_selection_echoes: 0,
        pending_own_primary_echoes: 0,
        finished: false,
    };

    thread::Builder::new()
        .name("clipboard-data-control".to_string())
        .spawn(move || {
            let Ok(mut event_loop) = EventLoop::<DataControlState>::try_new() else {
                warn!("No se pudo crear el bucle calloop para zwlr_data_control_v1");
                return;
            };
            let handle = event_loop.handle();
            if let Err(error) = WaylandSource::new(conn, event_queue).insert(handle.clone()) {
                warn!("No se pudo registrar WaylandSource en calloop: {error}");
                return;
            }
            if let Err(error) = handle.insert_source(cmd_rx, |event, _, state| {
                if let ChannelEvent::Msg(DataControlCmd::SetSelection {
                    mime_type,
                    data,
                    primary_selection,
                    reply,
                }) = event
                {
                    let result = state.handle_set_selection(mime_type, data, primary_selection);
                    let _ = reply.send(result);
                }
            }) {
                warn!("No se pudo registrar el canal de comandos de portapapeles: {error}");
                return;
            }

            let mut state = state;
            loop {
                if let Err(error) = event_loop.dispatch(None, &mut state) {
                    warn!("El bucle zwlr_data_control_v1 terminó con error: {error}");
                    break;
                }
                if state.finished {
                    break;
                }
            }
        })
        .context("No se pudo iniciar el hilo zwlr_data_control_v1")?;

    info!(
        "Monitor de portapapeles nativo Wayland activo (zwlr_data_control_manager_v1 v{manager_version})."
    );
    Ok(Some(control_handle))
}

pub(crate) fn expanded_offer_mimes(mime_type: &str) -> Vec<String> {
    let mut mimes = vec![mime_type.to_string()];
    let lower = mime_type.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "text/plain" | "text/plain;charset=utf-8" | "utf8_string" | "text" | "string"
    ) {
        for alias in ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"] {
            if !mimes.iter().any(|existing| existing == alias) {
                mimes.push(alias.to_string());
            }
        }
    }
    mimes
}

fn create_cloexec_pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is a valid 2-element array for pipe2(2).
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: pipe2 succeeded and returned two newly opened unique file descriptors.
    unsafe { Ok((OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))) }
}

/// Reads all bytes from `fd` up to `max_bytes` within `timeout`.
/// Returns `Ok(None)` if the sender produces more than `max_bytes` bytes.
pub(crate) fn read_fd_bounded(
    fd: OwnedFd,
    max_bytes: usize,
    timeout: Duration,
) -> Result<Option<Vec<u8>>> {
    let raw_fd = fd.as_raw_fd();
    let mut file = File::from(fd);
    let deadline = Instant::now() + timeout;
    let mut data = Vec::new();
    let mut chunk = [0u8; 16 * 1024];

    loop {
        let now = Instant::now();
        if now >= deadline {
            bail!("Tiempo de espera agotado al leer la selección del portapapeles");
        }
        let remaining_ms = deadline
            .saturating_duration_since(now)
            .as_millis()
            .min(i32::MAX as u128) as libc::c_int;
        let mut pfd = libc::pollfd {
            fd: raw_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pfd` points to a single valid pollfd struct.
        let ready = unsafe { libc::poll(&mut pfd, 1, remaining_ms) };
        if ready < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err).context("Error en poll() al leer la oferta del portapapeles");
        }
        if ready == 0 {
            bail!("Tiempo de espera agotado al esperar datos del portapapeles");
        }

        match file.read(&mut chunk) {
            Ok(0) => return Ok(Some(data)),
            Ok(n) => {
                if data.len().saturating_add(n) > max_bytes {
                    return Ok(None);
                }
                data.extend_from_slice(&chunk[..n]);
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err).context("Error al leer la oferta del portapapeles"),
        }
    }
}

pub(crate) fn write_fd_with_timeout(fd: OwnedFd, data: &[u8], timeout: Duration) -> Result<()> {
    let raw_fd = fd.as_raw_fd();
    // Set O_NONBLOCK so individual writes never stall indefinitely on a hung reader.
    // SAFETY: fcntl on a valid OwnedFd.
    unsafe {
        let flags = libc::fcntl(raw_fd, libc::F_GETFL);
        if flags >= 0 {
            let _ = libc::fcntl(raw_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
    let mut file = File::from(fd);
    let deadline = Instant::now() + timeout;
    let mut offset = 0usize;

    while offset < data.len() {
        let now = Instant::now();
        if now >= deadline {
            bail!("Tiempo de espera agotado al entregar datos del portapapeles");
        }
        let remaining_ms = deadline
            .saturating_duration_since(now)
            .as_millis()
            .min(i32::MAX as u128) as libc::c_int;
        let mut pfd = libc::pollfd {
            fd: raw_fd,
            events: libc::POLLOUT,
            revents: 0,
        };
        // SAFETY: `pfd` points to a single valid pollfd struct.
        let ready = unsafe { libc::poll(&mut pfd, 1, remaining_ms) };
        if ready < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err).context("Error en poll() al escribir datos del portapapeles");
        }
        if ready == 0 {
            bail!("Tiempo de espera agotado al esperar al receptor del portapapeles");
        }
        if (pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
            return Ok(());
        }
        match file.write(&data[offset..]) {
            Ok(0) => return Ok(()),
            Ok(n) => offset += n,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
            Err(err) => return Err(err).context("Error al escribir en el receptor del portapapeles"),
        }
    }
    Ok(())
}
