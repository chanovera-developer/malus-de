use std::collections::HashSet;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

#[derive(Clone, Debug)]
pub struct WifiApInfo {
    pub in_use: bool,
    pub ssid: String,
    pub signal: u8,
    pub is_encrypted: bool,
}

pub struct NetworkService;

impl NetworkService {
    /// Obtiene si el Wi-Fi está activado en NetworkManager vía D-Bus
    pub async fn is_wifi_enabled() -> bool {
        if let Ok(conn) = Connection::system().await {
            if let Ok(reply) = conn
                .call_method(
                    Some("org.freedesktop.NetworkManager"),
                    "/org/freedesktop/NetworkManager",
                    Some("org.freedesktop.DBus.Properties"),
                    "Get",
                    &("org.freedesktop.NetworkManager", "WirelessEnabled"),
                )
                .await
            {
                if let Ok(val) = reply.body().deserialize::<OwnedValue>() {
                    if let Some(enabled) = extract_bool(&val) {
                        return enabled;
                    }
                }
            }
        }
        false
    }

    /// Activa o desactiva el Wi-Fi en NetworkManager vía D-Bus
    pub async fn set_wifi_enabled(enabled: bool) -> bool {
        if let Ok(conn) = Connection::system().await {
            let res = conn
                .call_method(
                    Some("org.freedesktop.NetworkManager"),
                    "/org/freedesktop/NetworkManager",
                    Some("org.freedesktop.DBus.Properties"),
                    "Set",
                    &(
                        "org.freedesktop.NetworkManager",
                        "WirelessEnabled",
                        Value::from(enabled),
                    ),
                )
                .await;
            return res.is_ok();
        }
        false
    }

    /// Obtiene el SSID de la red Wi-Fi activa actualmente
    pub async fn get_active_wifi_ssid() -> Option<String> {
        let conn = Connection::system().await.ok()?;
        let wifi_dev_path = Self::find_wifi_device(&conn).await?;

        let reply = conn
            .call_method(
                Some("org.freedesktop.NetworkManager"),
                wifi_dev_path.as_str(),
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(
                    "org.freedesktop.NetworkManager.Device.Wireless",
                    "ActiveAccessPoint",
                ),
            )
            .await
            .ok()?;

        let body: OwnedValue = reply.body().deserialize().ok()?;
        let ap_path: OwnedObjectPath = match &*body {
            Value::ObjectPath(p) => OwnedObjectPath::from(p.clone()),
            _ => return None,
        };

        if ap_path.as_str() == "/" || ap_path.as_str().is_empty() {
            return None;
        }

        Self::get_ap_ssid(&conn, ap_path.as_str()).await
    }

    /// Escanea y lista las redes Wi-Fi disponibles usando D-Bus
    pub async fn scan_networks() -> (bool, Vec<WifiApInfo>) {
        let is_enabled = Self::is_wifi_enabled().await;
        if !is_enabled {
            return (false, Vec::new());
        }

        let conn = match Connection::system().await {
            Ok(c) => c,
            Err(_) => return (is_enabled, Vec::new()),
        };

        let wifi_dev = match Self::find_wifi_device(&conn).await {
            Some(d) => d,
            None => return (is_enabled, Vec::new()),
        };

        // Solicitar escaneo (asíncrono)
        let _ = conn
            .call_method(
                Some("org.freedesktop.NetworkManager"),
                wifi_dev.as_str(),
                Some("org.freedesktop.NetworkManager.Device.Wireless"),
                "RequestScan",
                &(std::collections::HashMap::<String, Value>::new(),),
            )
            .await;

        let active_ap = Self::get_active_ap_path(&conn, wifi_dev.as_str()).await;

        let mut ap_list: Vec<OwnedObjectPath> = Vec::new();
        if let Ok(reply) = conn
            .call_method(
                Some("org.freedesktop.NetworkManager"),
                wifi_dev.as_str(),
                Some("org.freedesktop.NetworkManager.Device.Wireless"),
                "GetAllAccessPoints",
                &(),
            )
            .await
        {
            if let Ok(paths) = reply.body().deserialize::<Vec<OwnedObjectPath>>() {
                ap_list = paths;
            }
        }

        let mut seen_ssids: HashSet<String> = HashSet::new();
        let mut networks = Vec::new();

        for ap in ap_list {
            let ap_str = ap.as_str();
            let is_active = active_ap.as_deref() == Some(ap_str);

            if let Some(ap_info) = Self::get_ap_details(&conn, ap_str, is_active).await {
                if !ap_info.ssid.is_empty() && seen_ssids.insert(ap_info.ssid.clone()) {
                    networks.push(ap_info);
                }
            }
        }

        networks.sort_by(|a, b| {
            b.in_use
                .cmp(&a.in_use)
                .then_with(|| b.signal.cmp(&a.signal))
        });
        (true, networks)
    }

    /// Conecta a una red Wi-Fi
    pub async fn connect_to_ssid(ssid: &str) -> bool {
        let (sender, receiver) = async_channel::bounded::<bool>(1);
        let ssid_owned = ssid.to_string();
        std::thread::spawn(move || {
            let status = std::process::Command::new("nmcli")
                .args(["dev", "wifi", "connect", &ssid_owned])
                .status();
            let _ = sender.send_blocking(status.map(|s| s.success()).unwrap_or(false));
        });
        receiver.recv().await.unwrap_or(false)
    }

    // Auxiliares privados
    async fn find_wifi_device(conn: &Connection) -> Option<String> {
        let reply = conn
            .call_method(
                Some("org.freedesktop.NetworkManager"),
                "/org/freedesktop/NetworkManager",
                Some("org.freedesktop.NetworkManager"),
                "GetDevices",
                &(),
            )
            .await
            .ok()?;

        let devices: Vec<OwnedObjectPath> = reply.body().deserialize().ok()?;
        for dev in devices {
            if let Ok(dev_type_reply) = conn
                .call_method(
                    Some("org.freedesktop.NetworkManager"),
                    dev.as_str(),
                    Some("org.freedesktop.DBus.Properties"),
                    "Get",
                    &("org.freedesktop.NetworkManager.Device", "DeviceType"),
                )
                .await
            {
                if let Ok(val) = dev_type_reply.body().deserialize::<OwnedValue>() {
                    if let Some(t) = extract_u32(&val) {
                        if t == 2 {
                            // NM_DEVICE_TYPE_WIFI = 2
                            return Some(dev.as_str().to_string());
                        }
                    }
                }
            }
        }
        None
    }

    async fn get_active_ap_path(conn: &Connection, dev_path: &str) -> Option<String> {
        let reply = conn
            .call_method(
                Some("org.freedesktop.NetworkManager"),
                dev_path,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(
                    "org.freedesktop.NetworkManager.Device.Wireless",
                    "ActiveAccessPoint",
                ),
            )
            .await
            .ok()?;

        let val: OwnedValue = reply.body().deserialize().ok()?;
        let path: Option<OwnedObjectPath> = match &*val {
            Value::ObjectPath(p) => Some(OwnedObjectPath::from(p.clone())),
            _ => None,
        };

        if let Some(p) = path {
            if p.as_str() != "/" && !p.as_str().is_empty() {
                return Some(p.as_str().to_string());
            }
        }
        None
    }

    async fn get_ap_ssid(conn: &Connection, ap_path: &str) -> Option<String> {
        let reply = conn
            .call_method(
                Some("org.freedesktop.NetworkManager"),
                ap_path,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.freedesktop.NetworkManager.AccessPoint", "Ssid"),
            )
            .await
            .ok()?;

        let val: OwnedValue = reply.body().deserialize().ok()?;
        match &*val {
            Value::Array(arr) => {
                let bytes: Vec<u8> = arr
                    .iter()
                    .filter_map(|v| match v {
                        Value::U8(b) => Some(*b),
                        _ => None,
                    })
                    .collect();
                let ssid = String::from_utf8_lossy(&bytes).trim().to_string();
                if !ssid.is_empty() {
                    Some(ssid)
                } else {
                    None
                }
            }
            Value::Str(s) => {
                let ssid = s.as_str().trim().to_string();
                if !ssid.is_empty() {
                    Some(ssid)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    async fn get_ap_details(
        conn: &Connection,
        ap_path: &str,
        is_active: bool,
    ) -> Option<WifiApInfo> {
        let ssid = Self::get_ap_ssid(conn, ap_path).await?;

        let mut signal = 0u8;
        let mut is_encrypted = false;

        if let Ok(reply) = conn
            .call_method(
                Some("org.freedesktop.NetworkManager"),
                ap_path,
                Some("org.freedesktop.DBus.Properties"),
                "GetAll",
                &("org.freedesktop.NetworkManager.AccessPoint",),
            )
            .await
        {
            if let Ok(props) = reply
                .body()
                .deserialize::<std::collections::HashMap<String, OwnedValue>>()
            {
                if let Some(s_val) = props.get("Strength") {
                    if let Some(s) = extract_u8(s_val) {
                        signal = s;
                    }
                }
                let wpa = props.get("WpaFlags").and_then(extract_u32).unwrap_or(0);
                let rsn = props.get("RsnFlags").and_then(extract_u32).unwrap_or(0);
                is_encrypted = wpa > 0 || rsn > 0;
            }
        }

        Some(WifiApInfo {
            in_use: is_active,
            ssid,
            signal,
            is_encrypted,
        })
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

fn extract_u32(val: &OwnedValue) -> Option<u32> {
    match &**val {
        Value::U32(u) => Some(*u),
        _ => None,
    }
}
