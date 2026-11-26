use anyhow::{Context, Result};
use gtk4::prelude::*;
use gtk4::{Align, ApplicationWindow, Box as GtkBox, Label, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use malus_de_common::ipc::{ipc_socket_path, send_ipc_message, IpcMessage};
use std::cell::Cell;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use tracing::{info, warn};

use crate::toolbar::{SelectionRect, ToolbarOptions};

pub fn run_countdown(seconds: u32) -> Result<()> {
    if seconds == 0 {
        return Ok(());
    }

    let display = gtk4::gdk::Display::default()
        .context("No se pudo acceder a la pantalla para la cuenta regresiva")?;
    let monitor = display
        .monitors()
        .item(0)
        .and_then(|item| item.downcast::<gtk4::gdk::Monitor>().ok())
        .context("No se encontró monitor")?;

    let window = ApplicationWindow::builder()
        .title("Cuenta regresiva de captura")
        .decorated(false)
        .css_classes(["malus-screenshot-selector"])
        .build();

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.set_halign(Align::Center);
    vbox.set_valign(Align::Center);

    let label = Label::new(Some(&seconds.to_string()));
    label.add_css_class("screenshot-countdown-badge");
    vbox.append(&label);
    window.set_child(Some(&vbox));

    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace("malus-screenshot-countdown");
        window.set_monitor(&monitor);
        window.set_keyboard_mode(KeyboardMode::None);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
    } else {
        window.fullscreen();
    }

    window.present();

    let main_loop = glib::MainLoop::new(None, false);
    let remaining = Rc::new(Cell::new(seconds));

    {
        let main_loop = main_loop.clone();
        let window = window.clone();
        let label = label.clone();
        let remaining = remaining.clone();

        glib::timeout_add_local(Duration::from_secs(1), move || {
            let cur = remaining.get();
            if cur <= 1 {
                window.close();
                main_loop.quit();
                glib::ControlFlow::Break
            } else {
                let next = cur - 1;
                remaining.set(next);
                label.set_label(&next.to_string());
                glib::ControlFlow::Continue
            }
        });
    }

    main_loop.run();
    std::thread::sleep(Duration::from_millis(100));
    Ok(())
}

pub fn start_screen_recording(
    rect: Option<SelectionRect>,
    options: &ToolbarOptions,
) -> Result<PathBuf> {
    let destination_dir = recordings_directory();
    fs::create_dir_all(&destination_dir)
        .with_context(|| format!("No se pudo crear {}", destination_dir.display()))?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("Reloj del sistema antes de UNIX_EPOCH")?
        .as_secs();

    let destination = destination_dir.join(format!("Grabación de pantalla-{timestamp}.mp4"));

    let crop_tuple = rect.map(|r| {
        (
            r.x.round() as i32,
            r.y.round() as i32,
            r.width.round() as i32,
            r.height.round() as i32,
        )
    });

    let msg = IpcMessage::StartScreenRecording {
        destination: destination.to_string_lossy().to_string(),
        rect: crop_tuple,
        include_audio: options.record_audio,
        fps: 0,
        lossless: options.lossless,
    };

    send_ipc_message(&msg)
        .context("No se pudo enviar solicitud de inicio de grabación al compositor Malus")?;
    info!(
        "Grabación de pantalla iniciada en {} (detener desde la top-bar o con ⌃⌘⎋ / ⇧⌘R)",
        destination.display()
    );

    wait_for_recording_finish(&destination);
    Ok(destination)
}

/// Espera de forma pasiva sobre el socket IPC del compositor hasta que la grabación finalice
/// (detenida desde el botón de la top-bar, mediante ⌃⌘⎋ / ⇧⌘R o `malus-de-ctl recording stop`).
fn wait_for_recording_finish(expected_dest: &Path) {
    let sock_path = ipc_socket_path();
    let Ok(stream) = UnixStream::connect(&sock_path) else {
        warn!("No se pudo suscribir al socket IPC para esperar fin de grabación");
        return;
    };
    let reader = BufReader::new(stream);
    for line in reader.lines().map_while(Result::ok) {
        if let Ok(IpcMessage::ScreenRecordingFinished {
            destination,
            success,
            error,
        }) = serde_json::from_str::<IpcMessage>(&line)
        {
            if Path::new(&destination) == expected_dest || !success {
                if let Some(err) = error {
                    warn!("La grabación de pantalla terminó con error: {err}");
                }
                break;
            }
        }
    }
}

pub fn recordings_directory() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let spanish = std::env::var_os("LC_ALL")
        .or_else(|| std::env::var_os("LC_MESSAGES"))
        .or_else(|| std::env::var_os("LANG"))
        .is_some_and(|locale| locale.to_string_lossy().to_lowercase().starts_with("es"));

    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let videos_dir = std::env::var_os("XDG_VIDEOS_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| read_xdg_dir(&config_dir.join("user-dirs.dirs"), "XDG_VIDEOS_DIR", &home))
        .unwrap_or_else(|| home.join(if spanish { "Vídeos" } else { "Videos" }));

    videos_dir.join(if spanish {
        "Grabaciones de pantalla"
    } else {
        "Screen Recordings"
    })
}

fn read_xdg_dir(path: &Path, key: &str, home: &Path) -> Option<PathBuf> {
    let contents = fs::read_to_string(path).ok()?;
    let prefix = format!("{key}=\"");
    let value = contents
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))?
        .strip_suffix('"')?;

    let resolved = value
        .strip_prefix("$HOME")
        .map(|suffix| home.join(suffix.strip_prefix('/').unwrap_or(suffix)))
        .unwrap_or_else(|| PathBuf::from(value));
    resolved.is_absolute().then_some(resolved)
}
