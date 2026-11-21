use anyhow::Result;
use malus_de_common::logging::init_logging;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{info, warn};
use zbus::interface;

mod airdrop;
mod protocol;

use airdrop::{
    scan_airdrop_peers, send_airdrop_file, send_airdrop_url, set_airdrop_mode, AirDropEngine,
    AirDropMode,
};
use protocol::NetworkPacket;

const KDE_CONNECT_PORT: u16 = 1716;

#[derive(Clone)]
pub struct LinkServer {
    pub airdrop: Arc<Mutex<AirDropEngine>>,
    #[allow(dead_code)]
    pub is_running: Arc<AtomicBool>,
}

#[interface(name = "org.malus_de.Link")]
impl LinkServer {
    #[zbus(name = "GetAirDropMode")]
    async fn get_airdrop_mode(&self) -> String {
        let mut st = self.airdrop.lock().unwrap();
        st.refresh_expiration_if_needed();
        st.mode.as_str().to_string()
    }

    #[zbus(name = "SetAirDropMode")]
    async fn set_airdrop_mode(&self, mode: String) -> bool {
        let parsed = AirDropMode::from_str(&mode);
        let engine = Arc::clone(&self.airdrop);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let ok = set_airdrop_mode(&engine, parsed);
            let _ = tx.send_blocking(ok);
        });
        rx.recv().await.unwrap_or(false)
    }

    #[zbus(name = "GetAirDropRemainingSeconds")]
    async fn get_airdrop_remaining_seconds(&self) -> u32 {
        let mut st = self.airdrop.lock().unwrap();
        st.refresh_expiration_if_needed();
        st.remaining_seconds()
    }

    #[zbus(name = "GetAirDropStatusJson")]
    async fn get_airdrop_status_json(&self) -> String {
        let engine = Arc::clone(&self.airdrop);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let snapshot = {
                let mut st = engine.lock().unwrap();
                st.status_snapshot(false)
            };
            let json = serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".to_string());
            let _ = tx.send_blocking(json);
        });
        rx.recv().await.unwrap_or_else(|_| "{}".to_string())
    }

    #[zbus(name = "ScanAirDropPeers")]
    async fn scan_airdrop_peers(&self, seconds: u32) -> Vec<(String, String, String)> {
        let engine = Arc::clone(&self.airdrop);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let res = scan_airdrop_peers(&engine, seconds).unwrap_or_default();
            let list: Vec<(String, String, String)> =
                res.into_iter().map(|p| (p.id, p.name, p.address)).collect();
            let _ = tx.send_blocking(list);
        });
        rx.recv().await.unwrap_or_default()
    }

    #[zbus(name = "SendAirDropFile")]
    async fn send_airdrop_file(&self, target: String, path: String) -> (bool, String) {
        let engine = Arc::clone(&self.airdrop);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = match send_airdrop_file(&engine, &target, &path) {
                Ok(msg) => (true, msg),
                Err(err) => (false, err),
            };
            let _ = tx.send_blocking(result);
        });
        rx.recv()
            .await
            .unwrap_or_else(|_| (false, "Canal interno cerrado.".to_string()))
    }

    #[zbus(name = "SendAirDropUrl")]
    async fn send_airdrop_url(&self, target: String, url: String) -> (bool, String) {
        let engine = Arc::clone(&self.airdrop);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = match send_airdrop_url(&engine, &target, &url) {
                Ok(msg) => (true, msg),
                Err(err) => (false, err),
            };
            let _ = tx.send_blocking(result);
        });
        rx.recv()
            .await
            .unwrap_or_else(|_| (false, "Canal interno cerrado.".to_string()))
    }

    #[zbus(name = "SimulateAirDropReceive")]
    async fn simulate_airdrop_receive(&self, sender_name: String, file_name: String) -> bool {
        info!(
            "📲 [AirDrop] Notificación de prueba '{}' desde '{}'...",
            file_name, sender_name
        );
        let summary = format!("AirDrop de {}", sender_name);
        let body = format!(
            "Compartió el archivo '{}'\nGuardado en ~/Descargas",
            file_name
        );
        airdrop::notify_airdrop(&summary, &body);
        true
    }

    async fn get_connected_devices(&self) -> Vec<(String, String, u32)> {
        let mut st = self.airdrop.lock().unwrap();
        st.refresh_expiration_if_needed();
        st.discovered_peers
            .iter()
            .map(|p| {
                let display = if p.model.is_empty() {
                    p.name.clone()
                } else {
                    format!("{} ({})", p.name, p.model)
                };
                (p.id.clone(), display, 0)
            })
            .collect()
    }

    async fn ring_device(&self, device_id: String) {
        info!("Enviando solicitud de llamada a dispositivo: {}", device_id);
    }

    async fn share_file(&self, device_id: String, path: String) {
        info!(
            "📤 [AirDrop] Compartiendo archivo '{}' con '{}' vía AWDL/owly...",
            path, device_id
        );
        let engine = Arc::clone(&self.airdrop);
        std::thread::spawn(move || {
            if let Err(err) = send_airdrop_file(&engine, &device_id, &path) {
                warn!(
                    "⚠️ [AirDrop] Fallo en ShareFile('{}', '{}'): {}",
                    device_id, path, err
                );
            }
        });
    }

    async fn sync_clipboard(&self, content: String) {
        info!(
            "Sincronizando portapapeles hacia dispositivo móvil (longitud: {})",
            content.len()
        );
    }
}

fn main() -> Result<()> {
    init_logging("malus_de_link");
    info!("Iniciando Malus Link Daemon (Motor AirDrop AWDL/owly & Mobile Integration)...");

    let is_running = Arc::new(AtomicBool::new(true));
    let airdrop_engine = Arc::new(Mutex::new(AirDropEngine::new()));

    // Diagnóstico inicial no invasivo del hardware AWDL (sin activar el radio al arranque)
    {
        let mut st = airdrop_engine.lock().unwrap();
        let snap = st.status_snapshot(true);
        info!(
            "📡 [AirDrop] Backend owly={}, hardware_ready={}, modo={:?}, estación={:?}, carpeta={}",
            snap.backend_installed,
            snap.hardware_ready,
            snap.radio_mode,
            snap.station,
            snap.downloads_dir
        );
    }

    // Descubrimiento UDP en puerto 1716 (KDE Connect)
    let run_udp = is_running.clone();
    std::thread::spawn(
        move || match UdpSocket::bind(("0.0.0.0", KDE_CONNECT_PORT)) {
            Ok(socket) => {
                info!(
                    "Socket UDP KDE Connect escuchando en 0.0.0.0:{}",
                    KDE_CONNECT_PORT
                );
                let _ = socket.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = [0u8; 4096];
                while run_udp.load(Ordering::SeqCst) {
                    if let Ok((len, src)) = socket.recv_from(&mut buf) {
                        if let Ok(text) = std::str::from_utf8(&buf[..len]) {
                            if let Ok(packet) = serde_json::from_str::<NetworkPacket>(text) {
                                info!("Paquete recibido de {}: {}", src, packet.packet_type);
                            }
                        }
                    }
                }
            }
            Err(err) => {
                warn!(
                    "Puerto UDP {} no disponible ({:?}). Modo pasivo activado.",
                    KDE_CONNECT_PORT, err
                );
            }
        },
    );

    // Publicar servicio D-Bus org.malus_de.Link
    let s_server = LinkServer {
        airdrop: airdrop_engine.clone(),
        is_running: is_running.clone(),
    };

    zbus::block_on(async {
        let _connection = zbus::connection::Builder::session()?
            .name("org.malus_de.Link")?
            .serve_at("/org/malus_de/Link", s_server)?
            .build()
            .await?;

        info!("Servicio D-Bus org.malus_de.Link listo y escuchando solicitudes.");
        std::future::pending::<()>().await;
        Ok::<(), zbus::Error>(())
    })?;

    Ok(())
}
