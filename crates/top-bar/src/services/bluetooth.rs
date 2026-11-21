use std::collections::HashMap;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

#[derive(Clone, Debug)]
pub struct BtDeviceInfo {
    pub path: String,
    pub address: String,
    pub name: String,
    pub connected: bool,
    pub icon_name: String,
    pub battery: Option<u8>,
}

pub struct BluetoothService;

impl BluetoothService {
    /// Consulta si el adaptador Bluetooth está encendido vía D-Bus
    pub async fn is_powered() -> bool {
        if let Ok(conn) = Connection::system().await {
            let adapter = Self::find_adapter(&conn).await;
            if let Ok(reply) = conn
                .call_method(
                    Some("org.bluez"),
                    adapter.as_str(),
                    Some("org.freedesktop.DBus.Properties"),
                    "Get",
                    &("org.bluez.Adapter1", "Powered"),
                )
                .await
            {
                if let Ok(val) = reply.body().deserialize::<OwnedValue>() {
                    if let Some(powered) = extract_bool(&val) {
                        return powered;
                    }
                }
            }
        }
        false
    }

    /// Enciende o apaga el adaptador Bluetooth vía D-Bus
    pub async fn set_powered(powered: bool) -> bool {
        if let Ok(conn) = Connection::system().await {
            let adapter = Self::find_adapter(&conn).await;
            let res = conn
                .call_method(
                    Some("org.bluez"),
                    adapter.as_str(),
                    Some("org.freedesktop.DBus.Properties"),
                    "Set",
                    &("org.bluez.Adapter1", "Powered", Value::from(powered)),
                )
                .await;
            return res.is_ok();
        }
        false
    }

    /// Obtiene el nombre del primer dispositivo conectado (para el subtítulo del tile)
    pub async fn get_active_device_name() -> Option<String> {
        let devices = Self::get_paired_devices().await;
        for dev in devices {
            if dev.connected && !dev.name.is_empty() {
                return Some(dev.name);
            }
        }
        None
    }

    /// Obtiene la lista completa de dispositivos emparejados usando ObjectManager
    pub async fn get_paired_devices() -> Vec<BtDeviceInfo> {
        let conn = match Connection::system().await {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let reply = match conn
            .call_method(
                Some("org.bluez"),
                "/",
                Some("org.freedesktop.DBus.ObjectManager"),
                "GetManagedObjects",
                &(),
            )
            .await
        {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        type ManagedObjects =
            HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

        let objects: ManagedObjects = match reply.body().deserialize() {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };

        let mut devices = Vec::new();

        for (path, ifaces) in objects {
            if let Some(dev_props) = ifaces.get("org.bluez.Device1") {
                let paired = dev_props
                    .get("Paired")
                    .and_then(extract_bool)
                    .unwrap_or(false);

                if !paired {
                    continue;
                }

                let address = dev_props
                    .get("Address")
                    .and_then(extract_string)
                    .unwrap_or_default();

                let name = dev_props
                    .get("Name")
                    .or_else(|| dev_props.get("Alias"))
                    .and_then(extract_string)
                    .unwrap_or_else(|| address.clone());

                let connected = dev_props
                    .get("Connected")
                    .and_then(extract_bool)
                    .unwrap_or(false);

                let raw_icon = dev_props
                    .get("Icon")
                    .and_then(extract_string)
                    .unwrap_or_default();

                let icon_name = Self::resolve_icon(&raw_icon, &name);

                let battery = ifaces
                    .get("org.bluez.Battery1")
                    .and_then(|bat_props| bat_props.get("Percentage").and_then(extract_u8));

                devices.push(BtDeviceInfo {
                    path: path.as_str().to_string(),
                    address,
                    name,
                    connected,
                    icon_name,
                    battery,
                });
            }
        }

        devices.sort_by(|a, b| b.connected.cmp(&a.connected));
        devices
    }

    /// Conecta un dispositivo vía D-Bus
    pub async fn connect_device(path: &str) -> bool {
        if let Ok(conn) = Connection::system().await {
            let res = conn
                .call_method(
                    Some("org.bluez"),
                    path,
                    Some("org.bluez.Device1"),
                    "Connect",
                    &(),
                )
                .await;
            return res.is_ok();
        }
        false
    }

    /// Desconecta un dispositivo vía D-Bus
    pub async fn disconnect_device(path: &str) -> bool {
        if let Ok(conn) = Connection::system().await {
            let res = conn
                .call_method(
                    Some("org.bluez"),
                    path,
                    Some("org.bluez.Device1"),
                    "Disconnect",
                    &(),
                )
                .await;
            return res.is_ok();
        }
        false
    }

    fn resolve_icon(raw_icon: &str, name: &str) -> String {
        if raw_icon.contains("audio-headset") || raw_icon.contains("audio-card") {
            "audio-headset-symbolic".to_string()
        } else if raw_icon.contains("input-keyboard") {
            "input-keyboard-symbolic".to_string()
        } else if raw_icon.contains("input-mouse") {
            "input-mouse-symbolic".to_string()
        } else if raw_icon.contains("phone") {
            "phone-symbolic".to_string()
        } else {
            let n = name.to_lowercase();
            if n.contains("buds")
                || n.contains("headset")
                || n.contains("space one")
                || n.contains("airpods")
                || n.contains("soundcore")
            {
                "audio-headset-symbolic".to_string()
            } else if n.contains("keyboard") || n.contains("teclado") {
                "input-keyboard-symbolic".to_string()
            } else if n.contains("mouse") || n.contains("raton") || n.contains("ratón") {
                "input-mouse-symbolic".to_string()
            } else {
                "bluetooth-active-symbolic".to_string()
            }
        }
    }

    async fn find_adapter(conn: &Connection) -> String {
        type ManagedObjects =
            HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

        if let Ok(reply) = conn
            .call_method(
                Some("org.bluez"),
                "/",
                Some("org.freedesktop.DBus.ObjectManager"),
                "GetManagedObjects",
                &(),
            )
            .await
        {
            if let Ok(objects) = reply.body().deserialize::<ManagedObjects>() {
                for (path, ifaces) in objects {
                    if ifaces.contains_key("org.bluez.Adapter1") {
                        return path.as_str().to_string();
                    }
                }
            }
        }
        "/org/bluez/hci0".to_string()
    }
}

fn extract_string(val: &OwnedValue) -> Option<String> {
    match &**val {
        Value::Str(s) => Some(s.as_str().to_string()),
        _ => None,
    }
}

fn extract_bool(val: &OwnedValue) -> Option<bool> {
    match &**val {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

fn extract_u8(val: &OwnedValue) -> Option<u8> {
    match &**val {
        Value::U8(u) => Some(*u),
        _ => None,
    }
}
