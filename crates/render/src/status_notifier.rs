// ==============================================================================
// Malus DE: Implementación de la Bandeja del Sistema (StatusNotifierWatcher / SNI)
// ==============================================================================
// Provee el servicio estándar org.kde.StatusNotifierWatcher para recibir y registrar
// los iconos de aplicaciones en segundo plano (Telegram, Discord, Steam, Dropbox, etc.)

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tracing::info;
use zbus::interface;

/// Mensaje de evento emitido cuando un ítem de bandeja se registra o desregistra.
#[derive(Clone, Debug)]
pub enum SniUpdate {
    ItemRegistered(String),
    ItemUnregistered(String),
}

/// Servidor D-Bus org.kde.StatusNotifierWatcher
pub struct StatusNotifierWatcher {
    pub registered_items: Arc<Mutex<HashSet<String>>>,
    pub update_tx: async_channel::Sender<SniUpdate>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl StatusNotifierWatcher {
    async fn register_status_notifier_item(
        &mut self,
        service: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(signal_context)] ctxt: zbus::SignalContext<'_>,
    ) -> zbus::fdo::Result<()> {
        let sender = header
            .sender()
            .map(|s| s.as_str().to_string())
            .unwrap_or_default();

        let item_path = if service.starts_with('/') {
            format!("{}{}", sender, service)
        } else {
            service.to_string()
        };

        info!("📥 [SNI Watcher] Nuevo ítem registrado: {}", item_path);
        {
            let mut items = self.registered_items.lock().unwrap();
            items.insert(item_path.clone());
        }

        let _ = self.update_tx.try_send(SniUpdate::ItemRegistered(item_path.clone()));
        Self::status_notifier_item_registered(&ctxt, &item_path).await?;
        Ok(())
    }

    async fn register_status_notifier_host(&self, _service: &str) -> zbus::fdo::Result<()> {
        info!("📥 [SNI Watcher] Host de bandeja registrado");
        Ok(())
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        let items = self.registered_items.lock().unwrap();
        items.iter().cloned().collect()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        ctxt: &zbus::SignalContext<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        ctxt: &zbus::SignalContext<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(ctxt: &zbus::SignalContext<'_>) -> zbus::Result<()>;
}

/// Gestor del servicio de la bandeja del sistema SNI.
pub struct SniService {
    pub registered_items: Arc<Mutex<HashSet<String>>>,
    pub update_rx: async_channel::Receiver<SniUpdate>,
}

impl SniService {
    /// Inicializa las estructuras del servicio de bandeja y el canal de actualización.
    pub fn new() -> (Self, StatusNotifierWatcher) {
        let registered_items = Arc::new(Mutex::new(HashSet::new()));
        let (update_tx, update_rx) = async_channel::unbounded();

        let watcher = StatusNotifierWatcher {
            registered_items: registered_items.clone(),
            update_tx,
        };

        let service = Self {
            registered_items,
            update_rx,
        };

        (service, watcher)
    }

    /// Inicia el servidor D-Bus en segundo plano.
    pub fn start_watcher(watcher: StatusNotifierWatcher) {
        std::thread::spawn(move || {
            let res = zbus::block_on(async move {
                let _conn = match zbus::connection::Builder::session()?
                    .name("org.kde.StatusNotifierWatcher")?
                    .serve_at("/StatusNotifierWatcher", watcher)?
                    .build()
                    .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!("Aviso: org.kde.StatusNotifierWatcher ya está activo o no disponible: {:?}", e);
                        return Ok::<(), zbus::Error>(());
                    }
                };

                info!("🛡️ [SNI Watcher] Servidor org.kde.StatusNotifierWatcher activo en el bus de sesión");
                std::future::pending::<()>().await;
                Ok(())
            });
            if let Err(e) = res {
                tracing::warn!("Error en servidor SNI Watcher: {:?}", e);
            }
        });
    }
}
