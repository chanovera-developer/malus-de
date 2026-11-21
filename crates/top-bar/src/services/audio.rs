use std::collections::HashMap;
use std::process::Command;
use tracing::info;
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

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
        let _ = Command::new("wpctl")
            .args([
                "set-volume",
                "@DEFAULT_AUDIO_SINK@",
                &format!("{:.2}", normalized),
            ])
            .spawn();
    }

    /// Alterna el estado de silencio (mute) en WirePlumber/PipeWire
    pub fn toggle_mute() {
        let _ = Command::new("wpctl")
            .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
            .spawn();
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
    /// Obtiene información de la pista y estado del reproductor multimedia activo
    pub async fn get_current_media() -> Option<MprisMediaInfo> {
        let conn = Connection::session().await.ok()?;

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
