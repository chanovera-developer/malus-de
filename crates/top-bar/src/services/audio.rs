use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tracing::info;
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

/// Dispositivo de salida de audio gestionado por PipeWire / PulseAudio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioOutputDevice {
    pub index: Option<u32>,
    pub name: String,
    pub description: String,
    pub icon_name: &'static str,
    pub is_default: bool,
}

/// Flujo de reproducción activo de una aplicación (mezclador por aplicación sobre PipeWire).
#[derive(Clone, Debug, PartialEq)]
pub struct AppAudioStream {
    pub index: u32,
    pub sink_index: Option<u32>,
    pub app_name: String,
    pub media_name: Option<String>,
    pub icon_name: String,
    pub volume_pct: f64,
    pub muted: bool,
    /// Canales individuales en el orden exacto de `channel_map` (ej. `[("front-left", 80.0), ("front-right", 65.0)]`).
    pub channels: Vec<(String, f64)>,
}

/// Etiqueta legible en español para un canal PipeWire/PulseAudio.
pub fn format_channel_label(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "front-left" | "left" => "Izquierdo".to_string(),
        "front-right" | "right" => "Derecho".to_string(),
        "front-center" | "center" => "Centro".to_string(),
        "mono" => "Mono".to_string(),
        "lfe" | "subwoofer" => "Subwoofer".to_string(),
        "rear-left" => "Trasero izq.".to_string(),
        "rear-right" => "Trasero der.".to_string(),
        "rear-center" => "Trasero centro".to_string(),
        "side-left" => "Lateral izq.".to_string(),
        "side-right" => "Lateral der.".to_string(),
        other => other.replace('-', " "),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PactlEventKind {
    Structural,
    Change,
}

/// Clasifica una línea de `pactl subscribe` para refresco reactivo en vivo.
pub fn classify_pactl_event(line: &str) -> Option<PactlEventKind> {
    let low = line.to_ascii_lowercase();
    let relevant_target = low.contains(" on sink-input ")
        || low.contains(" on sink ")
        || low.contains(" on source ")
        || low.contains(" on card ")
        || low.contains(" on server ");
    if !relevant_target {
        return None;
    }
    if low.contains("'new'") || low.contains("'remove'") || low.contains(" on server ") {
        Some(PactlEventKind::Structural)
    } else if low.contains("'change'") {
        Some(PactlEventKind::Change)
    } else {
        None
    }
}

/// Suscriptor ligero a `pactl subscribe` que detiene y mata el subproceso limpiamente al soltarse.
pub struct PactlWatcher {
    stop: Arc<AtomicBool>,
    child_pid: Arc<Mutex<Option<u32>>>,
}

impl PactlWatcher {
    pub fn start(on_event: async_channel::Sender<PactlEventKind>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let child_pid: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
        let stop_bg = stop.clone();
        let pid_bg = child_pid.clone();

        std::thread::spawn(move || {
            let Ok(mut child) = Command::new("pactl")
                .arg("subscribe")
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            else {
                return;
            };
            let pid = child.id();
            if let Ok(mut guard) = pid_bg.lock() {
                *guard = Some(pid);
            }
            if stop_bg.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            if let Some(stdout) = child.stdout.take() {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    if stop_bg.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(text) = line else { break };
                    if let Some(kind) = classify_pactl_event(&text) {
                        if on_event.send_blocking(kind).is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            if let Ok(mut guard) = pid_bg.lock() {
                *guard = None;
            }
        });

        Self { stop, child_pid }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut guard) = self.child_pid.lock() {
            if let Some(pid) = guard.take() {
                let _ = Command::new("kill")
                    .args(["-TERM", &pid.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
    }
}

impl Drop for PactlWatcher {
    fn drop(&mut self) {
        self.stop();
    }
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

fn is_null_placeholder(raw: &str) -> bool {
    let low = raw.trim().to_ascii_lowercase();
    low.is_empty()
        || matches!(
            low.as_str(),
            "null" | "(null)" | "<null>" | "none" | "(none)" | "n/a"
        )
}

fn is_null_audio_name(raw: &str) -> bool {
    if is_null_placeholder(raw) {
        return true;
    }
    let low = raw.trim().to_ascii_lowercase();
    if low.contains("auto_null")
        || low.contains("null-sink")
        || low.contains("null_sink")
        || low.contains("null-output")
        || low.contains("null_output")
        || low.contains("null output")
        || low.contains("null sink")
        || low.contains("null audio")
        || low.contains("salida nula")
        || low.contains("dummy")
        || low.contains("salida ficticia")
    {
        return true;
    }
    low.split(|c: char| matches!(c, ' ' | '-' | '_' | '.' | '/' | ':' | '(' | ')'))
        .any(|tok| tok == "null")
}

fn is_connected_real_sink(item: &serde_json::Value, name: &str, description: &str, is_default: bool) -> bool {
    let name_low = name.to_ascii_lowercase();
    let desc_low = description.to_ascii_lowercase();
    let driver = item
        .get("driver")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let props = item.get("properties");
    let str_prop = |k: &str| {
        props
            .and_then(|p| p.get(k))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
    };

    // Descartar salidas nulas, virtuales, ficticias (dummy/null), filtros y receptores de red RAOP
    if is_null_audio_name(name)
        || is_null_audio_name(description)
        || (!driver.trim().is_empty() && is_null_audio_name(driver))
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
    let node_name = str_prop("node.name");
    let node_desc = str_prop("node.description");
    if (!node_name.is_empty() && is_null_audio_name(&node_name))
        || (!node_desc.is_empty() && is_null_audio_name(&node_desc))
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

        let index = item.get("index").and_then(|v| v.as_u64()).map(|n| n as u32);
        devices.push(AudioOutputDevice {
            index,
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

fn parse_percent_str(raw: &str) -> Option<f64> {
    raw.trim().trim_end_matches('%').trim().parse::<f64>().ok()
}

fn infer_app_stream_icon(raw_icon: &str, app_name: &str) -> String {
    let trimmed = raw_icon.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    let low = app_name.to_ascii_lowercase();
    if low.contains("firefox")
        || low.contains("chrome")
        || low.contains("chromium")
        || low.contains("brave")
        || low.contains("zen")
        || low.contains("webkit")
    {
        "web-browser-symbolic".to_string()
    } else if low.contains("spotify")
        || low.contains("vlc")
        || low.contains("mpv")
        || low.contains("rhythmbox")
        || low.contains("music")
    {
        "audio-x-generic-symbolic".to_string()
    } else if low.contains("discord")
        || low.contains("telegram")
        || low.contains("slack")
        || low.contains("zoom")
        || low.contains("meet")
    {
        "chat-symbolic".to_string()
    } else if low.contains("steam") || low.contains("wine") || low.contains("proton") {
        "input-gaming-symbolic".to_string()
    } else {
        "multimedia-volume-control-symbolic".to_string()
    }
}

/// Parsea `pactl --format=json list sink-inputs` para el mezclador de volumen por aplicación.
pub fn parse_sink_inputs_json(raw_json: &str) -> Vec<AppAudioStream> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(raw_json)
    else {
        return Vec::new();
    };
    let mut streams = Vec::new();
    for item in items {
        let Some(index) = item.get("index").and_then(|v| v.as_u64()).map(|n| n as u32) else {
            continue;
        };
        let props = item.get("properties");
        let str_prop = |k: &str| {
            props
                .and_then(|p| p.get(k))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !is_null_placeholder(s))
        };

        // Omitir flujos nulos, virtuales o de feedback del propio sistema
        if str_prop("node.virtual").as_deref() == Some("true") {
            continue;
        }
        let driver = item
            .get("driver")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if !driver.trim().is_empty() && is_null_audio_name(driver) {
            continue;
        }
        let role = str_prop("media.role")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if role == "event" || role == "test" {
            continue;
        }
        let binary = str_prop("application.process.binary")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(binary.as_str(), "pw-play" | "paplay" | "pw-cat") {
            continue;
        }
        if let Some(node_name) = str_prop("node.name") {
            let nl = node_name.to_ascii_lowercase();
            if is_null_audio_name(&nl)
                || nl.starts_with("loopback")
                || nl.starts_with("easyeffects_")
            {
                continue;
            }
        }

        let Some(app_name) = str_prop("application.name")
            .or_else(|| str_prop("node.name"))
            .or_else(|| str_prop("media.name"))
        else {
            continue;
        };
        if is_null_audio_name(&app_name) {
            continue;
        }

        let media_name = str_prop("media.name")
            .filter(|m| !m.eq_ignore_ascii_case(&app_name) && !is_null_audio_name(m));
        let raw_icon = str_prop("application.icon_name")
            .or_else(|| str_prop("media.icon_name"))
            .unwrap_or_default();
        let icon_name = infer_app_stream_icon(&raw_icon, &app_name);

        let muted = item.get("mute").and_then(|v| v.as_bool()).unwrap_or(false);
        let sink_index = item.get("sink").and_then(|v| v.as_u64()).map(|n| n as u32);
        let mut channels: Vec<(String, f64)> = Vec::new();
        if let Some(vol_map) = item.get("volume").and_then(|v| v.as_object()) {
            let ordered_keys: Vec<String> = item
                .get("channel_map")
                .and_then(|v| v.as_str())
                .map(|cm| {
                    cm.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                })
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| vol_map.keys().cloned().collect());

            for key in ordered_keys {
                if let Some(pct) = vol_map
                    .get(&key)
                    .and_then(|ch| ch.get("value_percent"))
                    .and_then(|v| v.as_str())
                    .and_then(parse_percent_str)
                {
                    channels.push((key, pct.clamp(0.0, 150.0)));
                }
            }
            // Fallback por si channel_map tenía nombres distintos a las claves de volume
            if channels.is_empty() {
                for (key, ch) in vol_map {
                    if let Some(pct) = ch
                        .get("value_percent")
                        .and_then(|v| v.as_str())
                        .and_then(parse_percent_str)
                    {
                        channels.push((key.clone(), pct.clamp(0.0, 150.0)));
                    }
                }
            }
        }
        let volume_pct = if channels.is_empty() {
            100.0
        } else {
            (channels.iter().map(|(_, v)| *v).sum::<f64>() / channels.len() as f64)
                .clamp(0.0, 150.0)
        };

        streams.push(AppAudioStream {
            index,
            sink_index,
            app_name,
            media_name,
            icon_name,
            volume_pct,
            muted,
            channels,
        });
    }
    streams.sort_by(|a, b| {
        a.app_name
            .to_ascii_lowercase()
            .cmp(&b.app_name.to_ascii_lowercase())
            .then_with(|| a.index.cmp(&b.index))
    });
    streams
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
                    index: None,
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

    /// Lista los flujos activos de reproducción por aplicación (PipeWire `sink-inputs`).
    pub async fn list_app_streams() -> Vec<AppAudioStream> {
        let (sender, receiver) = async_channel::bounded::<Vec<AppAudioStream>>(1);
        std::thread::spawn(move || {
            let streams = Command::new("pactl")
                .args(["--format=json", "list", "sink-inputs"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| {
                    let raw = String::from_utf8_lossy(&o.stdout);
                    parse_sink_inputs_json(&raw)
                })
                .unwrap_or_default();
            let _ = sender.send_blocking(streams);
        });
        receiver.recv().await.unwrap_or_default()
    }

    /// Ajusta el volumen independiente de un flujo de aplicación (`0..=100%`).
    pub fn set_app_stream_volume(index: u32, pct: f64) {
        let clamped = pct.round().clamp(0.0, 150.0) as u32;
        std::thread::spawn(move || {
            let _ = Command::new("pactl")
                .args([
                    "set-sink-input-volume",
                    &index.to_string(),
                    &format!("{clamped}%"),
                ])
                .status();
        });
    }

    /// Ajusta el volumen independiente por canal de un flujo de aplicación (`pactl set-sink-input-volume <idx> <c0%> <c1%> ...`).
    pub fn set_app_stream_channel_volumes(index: u32, channel_pcts: Vec<f64>) {
        if channel_pcts.is_empty() {
            return;
        }
        let args_pct: Vec<String> = channel_pcts
            .into_iter()
            .map(|v| format!("{}%", v.round().clamp(0.0, 150.0) as u32))
            .collect();
        std::thread::spawn(move || {
            let mut cmd = Command::new("pactl");
            cmd.arg("set-sink-input-volume").arg(index.to_string());
            for pct in &args_pct {
                cmd.arg(pct);
            }
            let _ = cmd.status();
        });
    }

    /// Mueve un flujo de aplicación a otro dispositivo de salida PipeWire (`pactl move-sink-input <idx> <sink>`).
    pub async fn move_app_stream_to_sink(index: u32, sink_name: String) {
        if sink_name.is_empty() || sink_name.starts_with('@') {
            return;
        }
        let (sender, receiver) = async_channel::bounded::<()>(1);
        std::thread::spawn(move || {
            let _ = Command::new("pactl")
                .args(["move-sink-input", &index.to_string(), &sink_name])
                .status();
            let _ = sender.send_blocking(());
        });
        let _ = receiver.recv().await;
    }

    /// Silencia o reactiva un flujo de aplicación independiente.
    pub fn set_app_stream_mute(index: u32, muted: bool) {
        std::thread::spawn(move || {
            let _ = Command::new("pactl")
                .args([
                    "set-sink-input-mute",
                    &index.to_string(),
                    if muted { "1" } else { "0" },
                ])
                .status();
        });
    }

    /// Reproduce el sonido sutil de cambio de volumen si está habilitado en `sound.toml`.
    pub fn play_volume_feedback_if_enabled() {
        let cfg = malus_de_common::config::SoundConfig::load();
        if !cfg.volume_change_feedback {
            return;
        }
        let alert_vol = (cfg.alert_volume.clamp(0.0, 100.0) / 100.0).max(0.25);
        std::thread::spawn(move || {
            let path = "/usr/share/sounds/freedesktop/stereo/audio-volume-change.oga";
            if !std::path::Path::new(path).exists() {
                return;
            }
            let pa_vol = (65536.0 * alert_vol).round() as u32;
            let status = Command::new("paplay")
                .args(["--volume", &pa_vol.to_string(), path])
                .status();
            if status.map(|s| !s.success()).unwrap_or(true) {
                let _ = Command::new("pw-play")
                    .args(["--volume", &format!("{alert_vol:.2}"), path])
                    .status();
            }
        });
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
            },
            {
                "name": "my_null_output",
                "description": "Null Output",
                "properties": {}
            },
            {
                "name": "alsa_output.0",
                "description": "(null)",
                "properties": {}
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

    #[test]
    fn parse_sink_inputs_extracts_active_apps_and_filters_system_events() {
        let json = r#"[
            {
                "index": 42,
                "mute": false,
                "volume": {
                    "front-left": { "value_percent": "80%" },
                    "front-right": { "value_percent": "80%" }
                },
                "properties": {
                    "application.name": "Spotify",
                    "media.name": "Lofi Beats",
                    "application.icon_name": "spotify"
                }
            },
            {
                "index": 43,
                "mute": true,
                "volume": {
                    "front-left": { "value_percent": "50%" },
                    "front-right": { "value_percent": "60%" }
                },
                "properties": {
                    "application.name": "Firefox",
                    "media.name": "YouTube"
                }
            },
            {
                "index": 44,
                "mute": false,
                "volume": { "mono": { "value_percent": "100%" } },
                "properties": {
                    "application.name": "paplay",
                    "application.process.binary": "paplay",
                    "media.role": "event"
                }
            },
            {
                "index": 45,
                "mute": false,
                "volume": { "mono": { "value_percent": "100%" } },
                "properties": {
                    "application.name": "(null)",
                    "media.name": "null"
                }
            },
            {
                "index": 46,
                "mute": false,
                "volume": { "mono": { "value_percent": "100%" } },
                "properties": {
                    "node.name": "null-sink"
                }
            }
        ]"#;

        let streams = parse_sink_inputs_json(json);
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].app_name, "Firefox");
        assert_eq!(streams[0].media_name.as_deref(), Some("YouTube"));
        assert_eq!(streams[0].icon_name, "web-browser-symbolic");
        assert!(streams[0].muted);
        assert!((streams[0].volume_pct - 55.0).abs() < 0.01);
        assert_eq!(streams[0].channels.len(), 2);
        assert_eq!(streams[0].channels[0].0, "front-left");
        assert!((streams[0].channels[0].1 - 50.0).abs() < 0.01);
        assert_eq!(streams[0].channels[1].0, "front-right");
        assert!((streams[0].channels[1].1 - 60.0).abs() < 0.01);
        assert_eq!(streams[1].app_name, "Spotify");
        assert_eq!(streams[1].icon_name, "spotify");
        assert!(!streams[1].muted);
        assert!((streams[1].volume_pct - 80.0).abs() < 0.01);

        assert_eq!(format_channel_label("front-left"), "Izquierdo");
        assert_eq!(format_channel_label("front-right"), "Derecho");
        assert_eq!(
            classify_pactl_event("Event 'new' on sink-input #42"),
            Some(PactlEventKind::Structural)
        );
        assert_eq!(
            classify_pactl_event("Event 'change' on sink-input #42"),
            Some(PactlEventKind::Change)
        );
        assert_eq!(classify_pactl_event("Event 'change' on client #10"), None);
    }
}
