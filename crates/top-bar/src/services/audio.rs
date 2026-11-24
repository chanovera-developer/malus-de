use std::collections::HashMap;
use std::process::Command;
use tracing::info;
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

/// Dispositivo de salida de audio gestionado por PipeWire / PulseAudio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioOutputDevice {
    pub name: String,
    pub description: String,
    pub icon_name: &'static str,
    pub is_default: bool,
}

pub fn infer_output_device_icon(
    name: &str,
    description: &str,
    bus: &str,
    active_port: &str,
    form_factor: &str,
) -> &'static str {
    let combined = format!(
        "{} {} {} {} {}",
        name.to_ascii_lowercase(),
        description.to_ascii_lowercase(),
        bus.to_ascii_lowercase(),
        active_port.to_ascii_lowercase(),
        form_factor.to_ascii_lowercase()
    );
    if combined.contains("headphone")
        || combined.contains("headset")
        || combined.contains("auricular")
        || combined.contains("airpods")
        || combined.contains("buds")
        || combined.contains("space one")
    {
        "audio-headset-symbolic"
    } else if combined.contains("hdmi")
        || combined.contains("displayport")
        || combined.contains("monitor")
        || combined.contains("-dp")
        || combined.contains("dp-")
    {
        "video-display-symbolic"
    } else if combined.contains("bluetooth") || combined.contains("bluez") || combined.contains("usb")
    {
        "audio-speakers-symbolic"
    } else {
        "computer-symbolic"
    }
}

fn is_connected_real_sink(item: &serde_json::Value, name: &str, description: &str, is_default: bool) -> bool {
    let name_low = name.to_ascii_lowercase();
    let desc_low = description.to_ascii_lowercase();
    let props = item.get("properties");
    let str_prop = |k: &str| {
        props
            .and_then(|p| p.get(k))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
    };

    // Descartar salidas virtuales, ficticias (dummy/null), filtros y receptores de red RAOP
    if name_low.contains("auto_null")
        || name_low.contains("null-sink")
        || name_low.contains("null_sink")
        || name_low.contains("dummy")
        || name_low.starts_with("easyeffects_")
        || name_low.starts_with("effect_input.")
        || name_low.starts_with("effect_output.")
        || name_low.starts_with("loopback")
        || name_low.starts_with("raop_sink")
        || name_low.contains(".raop.")
        || desc_low.contains("dummy output")
        || desc_low.contains("salida ficticia")
    {
        return false;
    }

    if str_prop("node.virtual") == "true" {
        return false;
    }
    let dev_api = str_prop("device.api");
    if dev_api == "raop" {
        return false;
    }
    let factory = str_prop("factory.name");
    if factory.contains("null") || factory.contains("raop") {
        return false;
    }
    let dev_class = str_prop("device.class");
    if dev_class == "abstract" || dev_class == "virtual" {
        return false;
    }
    let media_class = str_prop("media.class");
    if !media_class.is_empty() && media_class != "audio/sink" {
        return false;
    }

    // Comprobar disponibilidad física de los puertos (HDMI/DP/auriculares desconectados)
    let active_port = item
        .get("active_port")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let mut active_availability: Option<String> = None;
    if let Some(ports) = item.get("ports").and_then(|v| v.as_array()) {
        if !ports.is_empty() {
            if !active_port.is_empty() {
                if let Some(port) = ports
                    .iter()
                    .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(active_port))
                {
                    let avail = port
                        .get("availability")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    if avail == "not available" {
                        return false;
                    }
                    active_availability = Some(avail);
                }
            } else {
                let all_unavailable = ports.iter().all(|p| {
                    p.get("availability")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .eq_ignore_ascii_case("not available")
                });
                if all_unavailable {
                    return false;
                }
            }
        }
    }

    // Salidas digitales S/PDIF (IEC958) sin detección física ni selección activa
    if !is_default
        && (name_low.contains("iec958")
            || name_low.contains("spdif")
            || active_port.to_ascii_lowercase().contains("iec958"))
        && active_availability.as_deref() != Some("available")
    {
        return false;
    }

    true
}

pub fn parse_output_devices_json(raw_json: &str, default_sink: &str) -> Vec<AudioOutputDevice> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(raw_json)
    else {
        return Vec::new();
    };
    let mut devices = Vec::new();
    for item in items {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let description = item
            .get("description")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&name)
            .to_string();
        let is_default = !default_sink.is_empty() && name == default_sink;
        if !is_connected_real_sink(&item, &name, &description, is_default) {
            continue;
        }
        let props = item.get("properties");
        let bus = props
            .and_then(|p| p.get("device.bus"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let form_factor = props
            .and_then(|p| p.get("device.form_factor"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let active_port = item
            .get("active_port")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        devices.push(AudioOutputDevice {
            is_default,
            icon_name: infer_output_device_icon(&name, &description, bus, active_port, form_factor),
            name,
            description,
        });
    }
    if !devices.is_empty() && !devices.iter().any(|d| d.is_default) {
        devices[0].is_default = true;
    }
    devices
}

/// Servicio para controlar el volumen del sistema gestionado por PipeWire / WirePlumber
pub struct AudioService;

impl AudioService {
    /// Obtiene de forma asíncrona (sin bloquear el hilo de GTK) el volumen actual y si está silenciado.
    pub async fn get_volume() -> (f64, bool) {
        let (sender, receiver) = async_channel::bounded::<(f64, bool)>(1);

        std::thread::spawn(move || {
            let mut vol_val = 75.0;
            let mut is_muted = false;

            if let Ok(out) = Command::new("wpctl")
                .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
                .output()
            {
                let s = String::from_utf8_lossy(&out.stdout);
                is_muted = s.contains("[MUTED]");

                for part in s.split_whitespace() {
                    if let Ok(v) = part.parse::<f64>() {
                        vol_val = (v * 100.0).clamp(0.0, 100.0);
                        break;
                    }
                }
            }

            let _ = sender.send_blocking((vol_val, is_muted));
        });

        receiver.recv().await.unwrap_or((75.0, false))
    }

    /// Establece el volumen en WirePlumber/PipeWire (0.0 a 100.0)
    pub fn set_volume(pct: f64) {
        let normalized = (pct / 100.0).clamp(0.0, 1.5);
        let arg = format!("{:.2}", normalized);
        std::thread::spawn(move || {
            let _ = Command::new("wpctl")
                .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &arg])
                .status();
        });
    }

    /// Alterna el estado de silencio (mute) en WirePlumber/PipeWire
    pub fn toggle_mute() {
        std::thread::spawn(|| {
            let _ = Command::new("wpctl")
                .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
                .status();
        });
    }

    /// Obtiene el nombre del dispositivo de salida de audio activo
    pub async fn get_active_sink_name() -> String {
        let (sender, receiver) = async_channel::bounded::<String>(1);
        std::thread::spawn(move || {
            let mut name = "Altavoces".to_string();
            if let Ok(out) = Command::new("wpctl")
                .args(["inspect", "@DEFAULT_AUDIO_SINK@"])
                .output()
            {
                let s = String::from_utf8_lossy(&out.stdout);
                for line in s.lines() {
                    let trimmed = line.trim();
                    if trimmed.contains("node.description =") {
                        if let Some((_, val)) = trimmed.split_once('=') {
                            let clean = val.trim().trim_matches('"');
                            if !clean.is_empty() {
                                name = clean.to_string();
                                break;
                            }
                        }
                    }
                }
            }
            let _ = sender.send_blocking(name);
        });
        receiver
            .recv()
            .await
            .unwrap_or_else(|_| "Altavoces".to_string())
    }

    /// Lista todos los dispositivos de salida de audio disponibles en PipeWire / PulseAudio.
    pub async fn list_output_devices() -> Vec<AudioOutputDevice> {
        let (sender, receiver) = async_channel::bounded::<Vec<AudioOutputDevice>>(1);
        std::thread::spawn(move || {
            let default_sink = Command::new("pactl")
                .arg("get-default-sink")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();

            let mut devices = Command::new("pactl")
                .args(["--format=json", "list", "sinks"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| {
                    let raw = String::from_utf8_lossy(&o.stdout);
                    parse_output_devices_json(&raw, &default_sink)
                })
                .unwrap_or_default();

            if devices.is_empty() {
                let mut fallback_name = "Altavoces integrados".to_string();
                if let Ok(out) = Command::new("wpctl")
                    .args(["inspect", "@DEFAULT_AUDIO_SINK@"])
                    .output()
                {
                    for line in String::from_utf8_lossy(&out.stdout).lines() {
                        let trimmed = line.trim();
                        if trimmed.contains("node.description =") {
                            if let Some((_, val)) = trimmed.split_once('=') {
                                let clean = val.trim().trim_matches('"');
                                if !clean.is_empty() {
                                    fallback_name = clean.to_string();
                                    break;
                                }
                            }
                        }
                    }
                }
                devices.push(AudioOutputDevice {
                    icon_name: infer_output_device_icon("", &fallback_name, "", "", ""),
                    name: "@DEFAULT_AUDIO_SINK@".to_string(),
                    description: fallback_name,
                    is_default: true,
                });
            }

            let _ = sender.send_blocking(devices);
        });
        receiver.recv().await.unwrap_or_default()
    }

    /// Selecciona el dispositivo de salida predeterminado en PipeWire / PulseAudio.
    pub async fn set_default_sink(sink_name: String) {
        if sink_name.is_empty() || sink_name.starts_with('@') {
            return;
        }
        let (sender, receiver) = async_channel::bounded::<()>(1);
        std::thread::spawn(move || {
            let _ = Command::new("pactl")
                .args(["set-default-sink", &sink_name])
                .status();
            let _ = sender.send_blocking(());
        });
        let _ = receiver.recv().await;
    }
}

/// Información del reproductor multimedia activo vía MPRIS D-Bus
#[derive(Clone, Debug)]
pub struct MprisMediaInfo {
    pub player_bus: String,
    pub player_name: String,
    pub title: String,
    pub artist: String,
    pub is_playing: bool,
}

/// Servicio D-Bus para integración con reproductores multimedia (MPRIS)
pub struct MprisService;

impl MprisService {
    async fn shared_session_connection() -> Option<Connection> {
        static SESSION_CONN: std::sync::OnceLock<std::sync::Mutex<Option<Connection>>> =
            std::sync::OnceLock::new();
        let lock = SESSION_CONN.get_or_init(|| std::sync::Mutex::new(None));
        if let Ok(guard) = lock.lock() {
            if let Some(conn) = guard.as_ref() {
                return Some(conn.clone());
            }
        }
        let conn = Connection::session().await.ok()?;
        if let Ok(mut guard) = lock.lock() {
            *guard = Some(conn.clone());
        }
        Some(conn)
    }

    /// Obtiene información de la pista y estado del reproductor multimedia activo
    pub async fn get_current_media() -> Option<MprisMediaInfo> {
        let conn = Self::shared_session_connection().await?;

        // Listar nombres en el bus de sesión
        let reply = conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "ListNames",
                &(),
            )
            .await
            .ok()?;

        let names: Vec<String> = reply.body().deserialize().ok()?;
        let mut candidates = Vec::new();

        for name in names {
            if name.starts_with("org.mpris.MediaPlayer2.") {
                if let Some(info) = Self::get_player_info(&conn, &name).await {
                    // Si está reproduciendo, darle máxima prioridad inmediata
                    if info.is_playing {
                        return Some(info);
                    }
                    candidates.push(info);
                }
            }
        }

        candidates.into_iter().next()
    }

    async fn get_player_info(conn: &Connection, bus_name: &str) -> Option<MprisMediaInfo> {
        // Consultar PlaybackStatus
        let status_reply = conn
            .call_method(
                Some(bus_name),
                "/org/mpris/MediaPlayer2",
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.mpris.MediaPlayer2.Player", "PlaybackStatus"),
            )
            .await
            .ok()?;

        let status_val: OwnedValue = status_reply.body().deserialize().ok()?;
        let is_playing = match &*status_val {
            Value::Str(s) => s.as_str() == "Playing",
            _ => false,
        };

        // Consultar Metadata
        let meta_reply = conn
            .call_method(
                Some(bus_name),
                "/org/mpris/MediaPlayer2",
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.mpris.MediaPlayer2.Player", "Metadata"),
            )
            .await
            .ok()?;

        let meta_val: OwnedValue = meta_reply.body().deserialize().ok()?;
        let meta_map = match &*meta_val {
            Value::Dict(d) => {
                let mut map = HashMap::new();
                for (k, v) in d.iter() {
                    if let Value::Str(ks) = k {
                        map.insert(ks.as_str(), v);
                    }
                }
                map
            }
            _ => HashMap::new(),
        };

        let title = meta_map
            .get("xesam:title")
            .and_then(|v| match v {
                Value::Str(s) => Some(s.as_str().to_string()),
                _ => None,
            })
            .unwrap_or_default();

        let artist = meta_map
            .get("xesam:artist")
            .and_then(|v| match v {
                Value::Array(arr) => {
                    let artists: Vec<String> = arr
                        .iter()
                        .filter_map(|item| match item {
                            Value::Str(s) => Some(s.as_str().to_string()),
                            _ => None,
                        })
                        .collect();
                    if artists.is_empty() {
                        None
                    } else {
                        Some(artists.join(", "))
                    }
                }
                Value::Str(s) => Some(s.as_str().to_string()),
                _ => None,
            })
            .unwrap_or_default();

        if title.is_empty() && artist.is_empty() {
            return None;
        }

        // Nombre amigable del reproductor (ej: "org.mpris.MediaPlayer2.brave.instance..." -> "Brave")
        let raw_suffix = bus_name.trim_start_matches("org.mpris.MediaPlayer2.");
        let clean_name = raw_suffix.split('.').next().unwrap_or(raw_suffix);
        let player_name = capitalize_first_letter(clean_name);

        Some(MprisMediaInfo {
            player_bus: bus_name.to_string(),
            player_name,
            title,
            artist,
            is_playing,
        })
    }

    /// Alterna reproducción/pausa en el reproductor vía D-Bus
    pub async fn play_pause(player_bus: &str) -> bool {
        if let Ok(conn) = Connection::session().await {
            info!("Enviando PlayPause a {}", player_bus);
            let res = conn
                .call_method(
                    Some(player_bus),
                    "/org/mpris/MediaPlayer2",
                    Some("org.mpris.MediaPlayer2.Player"),
                    "PlayPause",
                    &(),
                )
                .await;
            return res.is_ok();
        }
        false
    }

    /// Avanza a la siguiente pista vía D-Bus
    pub async fn next(player_bus: &str) -> bool {
        if let Ok(conn) = Connection::session().await {
            let res = conn
                .call_method(
                    Some(player_bus),
                    "/org/mpris/MediaPlayer2",
                    Some("org.mpris.MediaPlayer2.Player"),
                    "Next",
                    &(),
                )
                .await;
            return res.is_ok();
        }
        false
    }

    /// Retrocede a la pista anterior vía D-Bus
    pub async fn previous(player_bus: &str) -> bool {
        if let Ok(conn) = Connection::session().await {
            let res = conn
                .call_method(
                    Some(player_bus),
                    "/org/mpris/MediaPlayer2",
                    Some("org.mpris.MediaPlayer2.Player"),
                    "Previous",
                    &(),
                )
                .await;
            return res.is_ok();
        }
        false
    }
}

fn capitalize_first_letter(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_output_devices_selects_default_and_assigns_macos_icons() {
        let json = r#"[
            {
                "name": "alsa_output.pci-0000_00_1f.3.analog-stereo",
                "description": "Altavoces del MacBook Air",
                "active_port": "analog-output-speaker",
                "properties": { "device.bus": "pci" }
            },
            {
                "name": "bluez_output.AA_BB_CC.1",
                "description": "Space One",
                "active_port": "headset-output",
                "properties": { "device.bus": "bluetooth" }
            },
            {
                "name": "alsa_output.pci-0000_01_00.1.hdmi-stereo",
                "description": "Mi Monitor",
                "active_port": "hdmi-output-0",
                "ports": [
                    { "name": "hdmi-output-0", "availability": "available" }
                ],
                "properties": { "device.bus": "pci" }
            },
            {
                "name": "alsa_output.pci-0000_01_00.1.hdmi-stereo-extra1",
                "description": "HDMI desconectado",
                "active_port": "hdmi-output-1",
                "ports": [
                    { "name": "hdmi-output-1", "availability": "not available" }
                ],
                "properties": { "device.bus": "pci" }
            },
            {
                "name": "raop_sink.HomePod.local.7000",
                "description": "HomePod AirPlay",
                "properties": { "device.api": "raop" }
            },
            {
                "name": "auto_null",
                "description": "Dummy Output",
                "properties": { "node.virtual": "true" }
            }
        ]"#;

        let devs = parse_output_devices_json(json, "alsa_output.pci-0000_00_1f.3.analog-stereo");
        assert_eq!(devs.len(), 3);
        assert!(devs[0].is_default);
        assert_eq!(devs[0].icon_name, "computer-symbolic");
        assert!(!devs[1].is_default);
        assert_eq!(devs[1].icon_name, "audio-headset-symbolic");
        assert_eq!(devs[2].icon_name, "video-display-symbolic");
    }
}
