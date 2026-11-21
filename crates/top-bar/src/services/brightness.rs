use malus_de_common::config::DisplaysConfig;
use std::process::Command;

/// Servicio para controlar el brillo de la pantalla y el Modo Noche (Night Shift)
pub struct BrightnessService;

impl BrightnessService {
    /// Obtiene de forma asíncrona el nivel de brillo de pantalla actual (5.0 a 100.0)
    pub async fn get_brightness() -> f64 {
        let (sender, receiver) = async_channel::bounded::<f64>(1);
        std::thread::spawn(move || {
            // 1. Intentar leer desde /sys/class/backlight (laptops o paneles con controlador sysfs)
            if let Ok(entries) = std::fs::read_dir("/sys/class/backlight") {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let cur_path = path.join("brightness");
                    let max_path = path.join("max_brightness");
                    if cur_path.exists() && max_path.exists() {
                        if let (Ok(cur_str), Ok(max_str)) = (
                            std::fs::read_to_string(&cur_path),
                            std::fs::read_to_string(&max_path),
                        ) {
                            if let (Ok(cur), Ok(max)) =
                                (cur_str.trim().parse::<f64>(), max_str.trim().parse::<f64>())
                            {
                                if max > 0.0 {
                                    let pct = (cur / max * 100.0).clamp(5.0, 100.0);
                                    let _ = sender.send_blocking(pct);
                                    return;
                                }
                            }
                        }
                    }
                }
            }

            // 2. Fallback persistente: cargar desde DisplaysConfig (~/.config/malus-de/monitors.toml)
            let cfg = DisplaysConfig::load();
            let _ = sender.send_blocking(cfg.brightness.clamp(5.0, 100.0));
        });

        receiver.recv().await.unwrap_or(80.0)
    }

    /// Establece el nivel de brillo de pantalla (5.0 a 100.0)
    pub fn set_brightness(pct: f64) {
        let clamped = pct.clamp(5.0, 100.0);

        // 1. Guardar en DisplaysConfig
        let mut cfg = DisplaysConfig::load();
        cfg.brightness = clamped;
        let _ = cfg.save();

        // 2. Intentar escribir en /sys/class/backlight o invocar brightnessctl en un hilo secundario
        std::thread::spawn(move || {
            if let Ok(entries) = std::fs::read_dir("/sys/class/backlight") {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let cur_path = path.join("brightness");
                    let max_path = path.join("max_brightness");
                    if max_path.exists() {
                        if let Ok(max_str) = std::fs::read_to_string(&max_path) {
                            if let Ok(max) = max_str.trim().parse::<f64>() {
                                let target = ((clamped / 100.0) * max).round() as u64;
                                let _ = std::fs::write(&cur_path, target.to_string());
                            }
                        }
                    }
                }
            }

            let _ = Command::new("brightnessctl")
                .args(["set", &format!("{}%", clamped.round() as u32)])
                .output();
        });
    }

    /// Obtiene de forma asíncrona si Modo Noche (Night Shift) está activado
    pub async fn get_night_shift() -> bool {
        let (sender, receiver) = async_channel::bounded::<bool>(1);
        std::thread::spawn(move || {
            let mut enabled = false;
            if let Ok(out) = Command::new("gsettings")
                .args([
                    "get",
                    "org.gnome.settings-daemon.plugins.color",
                    "night-light-enabled",
                ])
                .output()
            {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
                enabled = s == "true";
            }
            let _ = sender.send_blocking(enabled);
        });
        receiver.recv().await.unwrap_or(false)
    }

    /// Activa o desactiva Modo Noche (Night Shift)
    pub fn set_night_shift(enabled: bool) {
        let val_str = if enabled { "true" } else { "false" };
        let _ = Command::new("gsettings")
            .args([
                "set",
                "org.gnome.settings-daemon.plugins.color",
                "night-light-enabled",
                val_str,
            ])
            .spawn();
    }
}
