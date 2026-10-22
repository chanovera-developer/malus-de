use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};
use zbus::interface;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

#[derive(Debug, Clone)]
pub struct RegisteredMenu {
    pub window_id: u32,
    pub service: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct DbusMenuItem {
    pub id: i32,
    pub label: String,
    pub shortcut: Option<String>,
    pub is_separator: bool,
    pub enabled: bool,
    pub visible: bool,
    pub toggle_type: Option<String>,
    pub toggle_state: i32,
    pub children: Vec<DbusMenuItem>,
}

#[derive(Debug, Clone)]
pub struct DbusMenuCategory {
    pub id: i32,
    pub title: String,
    pub items: Vec<DbusMenuItem>,
}

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
            "📥 [AppMenu.Registrar] Window {} registrado por servicio '{}' (path '{}')",
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
        info!("📤 [AppMenu.Registrar] Window {} desregistrado", window_id);
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

pub struct DbusMenuService {
    pub registrations_by_window: Arc<Mutex<HashMap<u32, RegisteredMenu>>>,
    pub registrations_by_service: Arc<Mutex<HashMap<String, RegisteredMenu>>>,
    pub update_rx: async_channel::Receiver<RegisteredMenu>,
}

impl DbusMenuService {
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

    /// Inicia el servidor D-Bus com.canonical.AppMenu.Registrar en un hilo en segundo plano
    pub fn start_registrar_server(registrar: AppMenuRegistrar) {
        std::thread::spawn(move || {
            let rt = zbus::block_on(async move {
                let _connection = match zbus::connection::Builder::session()?
                    .name("com.canonical.AppMenu.Registrar")?
                    .serve_at("/com/canonical/AppMenu/Registrar", registrar)?
                    .build()
                    .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        warn!(
                            "No se pudo iniciar com.canonical.AppMenu.Registrar: {:?}",
                            e
                        );
                        return Ok::<(), zbus::Error>(());
                    }
                };

                info!("🌐 Servicio D-Bus com.canonical.AppMenu.Registrar activo y escuchando.");
                std::future::pending::<()>().await;
                Ok::<(), zbus::Error>(())
            });

            if let Err(e) = rt {
                warn!("Aviso al registrar com.canonical.AppMenu.Registrar: {:?}", e);
            }
        });
    }

    /// Busca un menú D-Bus registrado asociado a la ventana o al app_id
    pub fn find_menu(
        &self,
        window_id: Option<u64>,
        app_id: Option<&str>,
    ) -> Option<(String, String)> {
        // 1. Buscar por window_id (si la ventana lo registró directamente)
        if let Some(wid) = window_id {
            let by_win = self.registrations_by_window.lock().unwrap();
            if let Some(reg) = by_win.get(&(wid as u32)) {
                return Some((reg.service.clone(), reg.path.clone()));
            }
        }

        // 2. Buscar por app_id cuando el publicador usa un nombre de servicio reconocible.
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

    /// Obtiene el árbol completo de menús de com.canonical.dbusmenu
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

        // Firma devuelta por GetLayout: (u32 revision, (i32 id, a{sv} props, av children))
        let (_revision, (root_id, _root_props, raw_categories)): (
            u32,
            (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>),
        ) = reply.body().deserialize()?;

        info!(
            "📋 [DbusMenu] Menú obtenido de {}:{} (root_id={}, categorías={})",
            service,
            path,
            root_id,
            raw_categories.len()
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

    /// Dispara el evento 'clicked' sobre un item de menú
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

    /// Notifica 'AboutToShow' antes de desplegar un submenú
    pub async fn about_to_show(service: &str, path: &str, item_id: i32) -> Result<()> {
        let connection = zbus::Connection::session().await?;
        connection
            .call_method(
                Some(service),
                path,
                Some("com.canonical.dbusmenu"),
                "AboutToShow",
                &(item_id,),
            )
            .await?;
        Ok(())
    }
}

fn normalize_app_id(id: &str) -> String {
    id.rsplit('/')
        .next()
        .unwrap_or(id)
        .trim_end_matches(".desktop")
        .to_lowercase()
}

fn unwrap_val<'a>(mut v: &'a Value<'a>) -> &'a Value<'a> {
    while let Value::Value(inner) = v {
        v = inner;
    }
    v
}

fn parse_raw_category(val: &OwnedValue) -> Option<DbusMenuCategory> {
    parse_raw_category_from_value(&**val)
}

fn parse_raw_category_from_value(val: &Value) -> Option<DbusMenuCategory> {
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

        let label_raw = extract_string_from_dict(&fields[1], "label")?;
        let title = clean_menu_label(&label_raw);

        let mut items = Vec::new();
        if let Value::Array(arr) = unwrap_val(&fields[2]) {
            for child_val in arr.iter() {
                if let Some(item) = parse_raw_item_from_value(child_val) {
                    items.push(item);
                }
            }
        }

        return Some(DbusMenuCategory { id, title, items });
    }
    None
}

fn parse_raw_item_from_value(val: &Value) -> Option<DbusMenuItem> {
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

        let item_type = extract_string_from_dict(&fields[1], "type")
            .unwrap_or_else(|| "standard".to_string());
        let is_separator = item_type == "separator";

        let label_raw = extract_string_from_dict(&fields[1], "label").unwrap_or_default();
        let label = clean_menu_label(&label_raw);

        let enabled = extract_bool_from_dict(&fields[1], "enabled").unwrap_or(true);
        let visible = extract_bool_from_dict(&fields[1], "visible").unwrap_or(true);

        if !visible && !is_separator {
            return None;
        }

        let shortcut = extract_value_from_dict(&fields[1], "shortcut")
            .and_then(parse_shortcut_value);
        let toggle_type = extract_string_from_dict(&fields[1], "toggle-type");
        let toggle_state = extract_i32_from_dict(&fields[1], "toggle-state").unwrap_or(0);

        let mut children = Vec::new();
        if let Value::Array(arr) = unwrap_val(&fields[2]) {
            for child_val in arr.iter() {
                if let Some(child_item) = parse_raw_item_from_value(child_val) {
                    children.push(child_item);
                }
            }
        }

        return Some(DbusMenuItem {
            id,
            label,
            shortcut,
            is_separator,
            enabled,
            visible,
            toggle_type,
            toggle_state,
            children,
        });
    }
    None
}

fn extract_value_from_dict<'a>(dict_val: &'a Value<'a>, key: &str) -> Option<&'a Value<'a>> {
    let dict_val = unwrap_val(dict_val);
    if let Value::Dict(dict) = dict_val {
        for (k, v) in dict.iter() {
            let k = unwrap_val(k);
            if let Value::Str(s) = k {
                if s.as_str() == key {
                    return Some(unwrap_val(v));
                }
            }
        }
    }
    None
}

fn extract_string_from_dict(dict_val: &Value<'_>, key: &str) -> Option<String> {
    extract_value_from_dict(dict_val, key).and_then(|v| match v {
        Value::Str(s) => Some(s.as_str().to_string()),
        _ => None,
    })
}

fn extract_bool_from_dict(dict_val: &Value<'_>, key: &str) -> Option<bool> {
    extract_value_from_dict(dict_val, key).and_then(|v| match v {
        Value::Bool(b) => Some(*b),
        _ => None,
    })
}

fn extract_i32_from_dict(dict_val: &Value<'_>, key: &str) -> Option<i32> {
    extract_value_from_dict(dict_val, key).and_then(|v| match v {
        Value::I32(i) => Some(*i),
        Value::U32(u) => Some(*u as i32),
        _ => None,
    })
}

/// Convierte mnemonicos de tipo `_Archivo` a `Archivo` y `__` a `_`
fn clean_menu_label(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                out.push('_');
                chars.next();
            }
            // Omitir guión bajo de nemónico
        } else {
            out.push(c);
        }
    }
    out
}

/// Parsea la propiedad `shortcut` de dbusmenu (Array of Arrays of Strings) en atajos macOS
fn parse_shortcut_value(val: &Value<'_>) -> Option<String> {
    let val = unwrap_val(val);
    if let Value::Array(outer) = val {
        if let Some(first_combo) = outer.first() {
            let combo = unwrap_val(first_combo);
            if let Value::Array(keys) = combo {
                let mut modifiers = Vec::new();
                let mut key_str = String::new();

                for k in keys.iter() {
                    let k = unwrap_val(k);
                    if let Value::Str(s) = k {
                        let name = s.as_str();
                        match name.to_lowercase().as_str() {
                            "control" | "ctrl" => modifiers.push("⌘"),
                            "shift" => modifiers.push("⇧"),
                            "alt" => modifiers.push("⌥"),
                            "super" => modifiers.push("⌘"),
                            other => {
                                key_str = match other {
                                    "return" | "enter" => "↩".to_string(),
                                    "backspace" => "⌫".to_string(),
                                    "tab" => "⇥".to_string(),
                                    "space" => "Espacio".to_string(),
                                    "escape" | "esc" => "⎋".to_string(),
                                    "delete" => "⌦".to_string(),
                                    "up" => "↑".to_string(),
                                    "down" => "↓".to_string(),
                                    "left" => "←".to_string(),
                                    "right" => "→".to_string(),
                                    c => c.to_uppercase(),
                                };
                            }
                        }
                    }
                }

                if !key_str.is_empty() || !modifiers.is_empty() {
                    return Some(format!("{}{}", modifiers.join(""), key_str));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_menu_label() {
        assert_eq!(clean_menu_label("_Archivo"), "Archivo");
        assert_eq!(clean_menu_label("Guardar _como..."), "Guardar como...");
        assert_eq!(clean_menu_label("P_referencias del Sistema"), "Preferencias del Sistema");
        assert_eq!(clean_menu_label("Escape__Under"), "Escape_Under");
    }

    #[test]
    fn test_parse_shortcut_value() {
        let keys_val = Value::Array(vec![
            Value::from("Control"),
            Value::from("s"),
        ].into());
        let outer_val = Value::Array(vec![keys_val].into());
        assert_eq!(parse_shortcut_value(&outer_val), Some("⌘S".to_string()));

        let keys_val2 = Value::Array(vec![
            Value::from("Control"),
            Value::from("Shift"),
            Value::from("p"),
        ].into());
        let outer_val2 = Value::Array(vec![keys_val2].into());
        assert_eq!(parse_shortcut_value(&outer_val2), Some("⌘⇧P".to_string()));
    }

    #[test]
    fn test_registrar_mapping() {
        let (service, _registrar) = DbusMenuService::new();
        let path = OwnedObjectPath::try_from("/MenuBar/1").unwrap();
        
        let reg = RegisteredMenu {
            window_id: 1234,
            service: ":1.99".to_string(),
            path: path.as_str().to_string(),
        };

        service.registrations_by_window.lock().unwrap().insert(1234, reg.clone());
        service.registrations_by_service.lock().unwrap().insert(":1.99".to_string(), reg);

        let found = service.find_menu(Some(1234), Some("my-app"));
        assert_eq!(found, Some((":1.99".to_string(), "/MenuBar/1".to_string())));
    }

    #[test]
    fn does_not_return_an_unrelated_menu_for_a_different_app() {
        let (service, _registrar) = DbusMenuService::new();
        let reg = RegisteredMenu {
            window_id: 1234,
            service: ":1.99".to_string(),
            path: "/MenuBar/1".to_string(),
        };
        service.registrations_by_service.lock().unwrap().insert(reg.service.clone(), reg);

        assert_eq!(service.find_menu(None, Some("code")), None);
    }

    #[test]
    fn matches_explicit_desktop_service_names() {
        let (service, _registrar) = DbusMenuService::new();
        let reg = RegisteredMenu {
            window_id: 1234,
            service: "org.example.Editor".to_string(),
            path: "/MenuBar/1".to_string(),
        };
        service.registrations_by_service.lock().unwrap().insert(reg.service.clone(), reg);

        assert_eq!(
            service.find_menu(None, Some("org.example.Editor.desktop")),
            Some(("org.example.Editor".to_string(), "/MenuBar/1".to_string()))
        );
    }
}
