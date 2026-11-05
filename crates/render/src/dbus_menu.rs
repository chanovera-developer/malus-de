// ==============================================================================
// Malus DE: Servicio D-Bus Menú Global (com.canonical.AppMenu.Registrar & dbusmenu)
// ==============================================================================

use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};
use zbus::interface;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// Información de un menú registrado en el sistema.
#[derive(Debug, Clone)]
pub struct RegisteredMenu {
    pub window_id: u32,
    pub service: String,
    pub path: String,
}

/// Elemento individual de menú obtenido vía D-Bus.
#[derive(Debug, Clone)]
pub struct DbusMenuItem {
    pub id: i32,
    pub label: String,
    pub shortcut: Option<String>,
    pub is_separator: bool,
    pub enabled: bool,
    pub visible: bool,
    pub children: Vec<DbusMenuItem>,
}

/// Categoría raíz del Menú Global (ej. "Archivo", "Edición", "Ver").
#[derive(Debug, Clone)]
pub struct DbusMenuCategory {
    pub id: i32,
    pub title: String,
    pub items: Vec<DbusMenuItem>,
}

/// Implementación del servidor D-Bus com.canonical.AppMenu.Registrar
pub struct AppMenuRegistrar {
    pub registrations_by_window: Arc<Mutex<HashMap<u32, RegisteredMenu>>>,
    pub registrations_by_service: Arc<Mutex<HashMap<String, RegisteredMenu>>>,
    pub update_tx: async_channel::Sender<RegisteredMenu>,
}

#[interface(name = "com.canonical.AppMenu.Registrar")]
impl AppMenuRegistrar {
    async fn register_window(
        &self,
        window_id: u32,
        menu_object_path: OwnedObjectPath,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let sender = header
            .sender()
            .map(|s| s.as_str().to_string())
            .unwrap_or_default();
        let path = menu_object_path.as_str().to_string();

        info!(
            "📥 [AppMenu.Registrar] Ventana {} registrada por '{}' (path '{}')",
            window_id, sender, path
        );

        let reg = RegisteredMenu {
            window_id,
            service: sender.clone(),
            path,
        };

        self.registrations_by_window
            .lock()
            .unwrap()
            .insert(window_id, reg.clone());
        self.registrations_by_service
            .lock()
            .unwrap()
            .insert(sender, reg.clone());

        let _ = self.update_tx.try_send(reg);
        Ok(())
    }

    async fn unregister_window(&self, window_id: u32) -> zbus::fdo::Result<()> {
        info!("📤 [AppMenu.Registrar] Ventana {} desregistrada", window_id);
        let mut by_win = self.registrations_by_window.lock().unwrap();
        if let Some(reg) = by_win.remove(&window_id) {
            let mut by_service = self.registrations_by_service.lock().unwrap();
            if by_service
                .get(&reg.service)
                .is_some_and(|current| current.window_id == window_id)
            {
                if let Some(replacement) = by_win
                    .values()
                    .find(|registered| registered.service == reg.service)
                    .cloned()
                {
                    by_service.insert(reg.service, replacement);
                } else {
                    by_service.remove(&reg.service);
                }
            }
        }
        Ok(())
    }

    async fn get_menu_for_window(
        &self,
        window_id: u32,
    ) -> zbus::fdo::Result<(String, OwnedObjectPath)> {
        let by_win = self.registrations_by_window.lock().unwrap();
        if let Some(reg) = by_win.get(&window_id) {
            let path = OwnedObjectPath::try_from(reg.path.clone())
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            Ok((reg.service.clone(), path))
        } else {
            Err(zbus::fdo::Error::Failed(format!(
                "No hay menú registrado para ventana {}",
                window_id
            )))
        }
    }

    async fn register_window_nick(
        &self,
        window_id: u32,
        service: String,
        menu_object_path: OwnedObjectPath,
    ) -> zbus::fdo::Result<()> {
        let path = menu_object_path.as_str().to_string();
        info!(
            "📥 [AppMenu.Registrar] WindowNick {} registrado con service '{}', path '{}'",
            window_id, service, path
        );

        let reg = RegisteredMenu {
            window_id,
            service: service.clone(),
            path,
        };

        self.registrations_by_window
            .lock()
            .unwrap()
            .insert(window_id, reg.clone());
        self.registrations_by_service
            .lock()
            .unwrap()
            .insert(service, reg.clone());

        let _ = self.update_tx.try_send(reg);
        Ok(())
    }
}

/// Gestor del servicio de menús D-Bus para la barra superior.
pub struct DbusMenuService {
    pub registrations_by_window: Arc<Mutex<HashMap<u32, RegisteredMenu>>>,
    pub registrations_by_service: Arc<Mutex<HashMap<String, RegisteredMenu>>>,
    pub update_rx: async_channel::Receiver<RegisteredMenu>,
}

impl DbusMenuService {
    /// Inicializa las estructuras del servicio de menús y el canal de actualización.
    pub fn new() -> (Self, AppMenuRegistrar) {
        let registrations_by_window = Arc::new(Mutex::new(HashMap::new()));
        let registrations_by_service = Arc::new(Mutex::new(HashMap::new()));
        let (update_tx, update_rx) = async_channel::unbounded();

        let registrar = AppMenuRegistrar {
            registrations_by_window: registrations_by_window.clone(),
            registrations_by_service: registrations_by_service.clone(),
            update_tx,
        };

        let service = Self {
            registrations_by_window,
            registrations_by_service,
            update_rx,
        };

        (service, registrar)
    }

    /// Inicia el servidor D-Bus en segundo plano.
    pub fn start_registrar(registrar: AppMenuRegistrar) {
        std::thread::spawn(move || {
            let res = zbus::block_on(async move {
                let _conn = match zbus::connection::Builder::session()?
                    .name("com.canonical.AppMenu.Registrar")?
                    .serve_at("/com/canonical/AppMenu/Registrar", registrar)?
                    .build()
                    .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("Aviso: com.canonical.AppMenu.Registrar ya está activo o no disponible: {:?}", e);
                        return Ok::<(), zbus::Error>(());
                    }
                };

                info!("🌐 Servicio D-Bus com.canonical.AppMenu.Registrar activo y escuchando.");
                std::future::pending::<()>().await;
                Ok::<(), zbus::Error>(())
            });

            if let Err(e) = res {
                warn!("Error en hilo de AppMenu.Registrar: {:?}", e);
            }
        });
    }

    /// Busca un menú D-Bus registrado asociado a la ventana o al app_id.
    pub fn find_menu(&self, window_id: Option<u64>, app_id: Option<&str>) -> Option<(String, String)> {
        if let Some(wid) = window_id {
            let by_win = self.registrations_by_window.lock().unwrap();
            if let Some(reg) = by_win.get(&(wid as u32)) {
                return Some((reg.service.clone(), reg.path.clone()));
            }
        }

        if let Some(id) = app_id {
            let id_normalized = normalize_app_id(id);
            let by_srv = self.registrations_by_service.lock().unwrap();
            for (srv, reg) in by_srv.iter() {
                if normalize_app_id(srv) == id_normalized {
                    return Some((reg.service.clone(), reg.path.clone()));
                }
            }
        }

        None
    }

    /// Consulta el árbol de categorías de com.canonical.dbusmenu.
    pub async fn fetch_layout(service: &str, path: &str) -> Result<Vec<DbusMenuCategory>> {
        let connection = zbus::Connection::session().await?;
        let reply = connection
            .call_method(
                Some(service),
                path,
                Some("com.canonical.dbusmenu"),
                "GetLayout",
                &(0i32, -1i32, Vec::<String>::new()),
            )
            .await?;

        let (_revision, (root_id, _root_props, raw_categories)): (
            u32,
            (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>),
        ) = reply.body().deserialize()?;

        info!(
            "📋 [DbusMenu] Menú obtenido de {}:{} (root_id={}, categorías={})",
            service, path, root_id, raw_categories.len()
        );

        let mut categories = Vec::new();
        for cat_val in raw_categories {
            if let Some(node) = parse_raw_category(&cat_val) {
                if !node.items.is_empty() {
                    categories.push(node);
                }
            }
        }

        Ok(categories)
    }

    /// Dispara el evento 'clicked' sobre un item de menú.
    pub async fn trigger_event(service: &str, path: &str, item_id: i32) -> Result<()> {
        let connection = zbus::Connection::session().await?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u32;

        info!(
            "⚡ [DbusMenu] Disparando evento clicked: service={}, path={}, item_id={}",
            service, path, item_id
        );

        let _ = connection
            .call_method(
                Some(service),
                path,
                Some("com.canonical.dbusmenu"),
                "Event",
                &(item_id, "clicked", Value::from(0u32), timestamp),
            )
            .await?;

        Ok(())
    }
}

pub fn normalize_app_id(id: &str) -> String {
    id.rsplit('/')
        .next()
        .unwrap_or(id)
        .trim_end_matches(".desktop")
        .to_lowercase()
}

pub fn clean_menu_label(label: &str) -> String {
    label.replace('_', "")
}

fn unwrap_val<'a>(mut v: &'a Value<'a>) -> &'a Value<'a> {
    while let Value::Value(inner) = v {
        v = inner;
    }
    v
}

fn parse_raw_category(val: &OwnedValue) -> Option<DbusMenuCategory> {
    let val = unwrap_val(&**val);
    if let Value::Structure(s) = val {
        let fields = s.fields();
        if fields.len() < 3 {
            return None;
        }
        let id = match unwrap_val(&fields[0]) {
            Value::I32(i) => *i,
            Value::U32(u) => *u as i32,
            _ => return None,
        };

        let mut title = String::new();
        if let Value::Dict(dict) = unwrap_val(&fields[1]) {
            for (k, v) in dict.iter() {
                if let Value::Str(key_str) = unwrap_val(k) {
                    if key_str.as_str() == "label" {
                        if let Value::Str(val_str) = unwrap_val(v) {
                            title = clean_menu_label(val_str.as_str());
                        }
                    }
                }
            }
        }

        if title.is_empty() {
            return None;
        }

        let mut items = Vec::new();
        if let Value::Array(arr) = unwrap_val(&fields[2]) {
            for item_val in arr.iter() {
                if let Some(item) = parse_raw_item(item_val) {
                    items.push(item);
                }
            }
        }

        return Some(DbusMenuCategory { id, title, items });
    }
    None
}

fn parse_raw_item(val: &Value) -> Option<DbusMenuItem> {
    let val = unwrap_val(val);
    if let Value::Structure(s) = val {
        let fields = s.fields();
        if fields.len() < 3 {
            return None;
        }
        let id = match unwrap_val(&fields[0]) {
            Value::I32(i) => *i,
            Value::U32(u) => *u as i32,
            _ => return None,
        };

        let mut label = String::new();
        let mut enabled = true;
        let mut visible = true;
        let mut is_separator = false;

        if let Value::Dict(dict) = unwrap_val(&fields[1]) {
            for (k, v) in dict.iter() {
                if let Value::Str(key_str) = unwrap_val(k) {
                    match key_str.as_str() {
                        "label" => {
                            if let Value::Str(val_str) = unwrap_val(v) {
                                label = clean_menu_label(val_str.as_str());
                            }
                        }
                        "enabled" => {
                            if let Value::Bool(b) = unwrap_val(v) {
                                enabled = *b;
                            }
                        }
                        "visible" => {
                            if let Value::Bool(b) = unwrap_val(v) {
                                visible = *b;
                            }
                        }
                        "type" => {
                            if let Value::Str(t) = unwrap_val(v) {
                                if t.as_str() == "separator" {
                                    is_separator = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let mut children = Vec::new();
        if let Value::Array(arr) = unwrap_val(&fields[2]) {
            for child_val in arr.iter() {
                if let Some(child) = parse_raw_item(child_val) {
                    children.push(child);
                }
            }
        }

        return Some(DbusMenuItem {
            id,
            label,
            shortcut: None,
            is_separator,
            enabled,
            visible,
            children,
        });
    }
    None
}
