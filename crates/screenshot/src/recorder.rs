use anyhow::{Context, Result};
use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, Label, Orientation,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use malus_de_common::ipc::{send_ipc_message, IpcMessage};
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};
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

    let destination = destination_dir.join(format!("Grabación de pantalla-{timestamp}.webm"));

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
        fps: 30,
    };

    send_ipc_message(&msg)
        .context("No se pudo enviar solicitud de inicio de grabación al compositor Malus")?;
    info!("Grabación de pantalla iniciada en {}", destination.display());

    show_recording_indicator(destination.clone())?;
    Ok(destination)
}

fn show_recording_indicator(_destination: PathBuf) -> Result<()> {
    let display = gtk4::gdk::Display::default()
        .context("No se pudo acceder a la pantalla para el indicador de grabación")?;
    let monitor = display
        .monitors()
        .item(0)
        .and_then(|item| item.downcast::<gtk4::gdk::Monitor>().ok())
        .context("No se encontró monitor")?;

    let window = ApplicationWindow::builder()
        .title("Grabando pantalla")
        .decorated(false)
        .css_classes(["screenshot-recording-indicator"])
        .build();

    let hbox = GtkBox::new(Orientation::Horizontal, 8);
    hbox.set_margin_start(4);
    hbox.set_margin_end(4);
    hbox.set_margin_top(2);
    hbox.set_margin_bottom(2);

    let dot = Label::new(Some("●"));
    dot.add_css_class("screenshot-rec-dot");
    hbox.append(&dot);

    let timer_label = Label::new(Some("00:00"));
    timer_label.add_css_class("screenshot-timer-label");
    hbox.append(&timer_label);

    let stop_btn = Button::with_label("⏹ Detener");
    stop_btn.add_css_class("screenshot-stop-btn");
    hbox.append(&stop_btn);

    window.set_child(Some(&hbox));

    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_layer(Layer::Top);
        window.set_namespace("malus-recording-indicator");
        window.set_monitor(&monitor);
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Top, 10);
        window.set_margin(Edge::Right, 24);
        window.set_exclusive_zone(0);
        window.set_keyboard_mode(KeyboardMode::None);
    }

    window.present();

    let main_loop = glib::MainLoop::new(None, false);
    let start_time = Instant::now();

    // Actualización de reloj 00:00
    {
        let timer_label = timer_label.clone();
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let elapsed = start_time.elapsed().as_secs();
            let mins = elapsed / 60;
            let secs = elapsed % 60;
            timer_label.set_label(&format!("{mins:02}:{secs:02}"));
            glib::ControlFlow::Continue
        });
    }

    // Detener grabación al pulsar botón
    {
        let window = window.clone();
        let loop_quit = main_loop.clone();
        stop_btn.connect_clicked(move |_| {
            info!("Deteniendo grabación de pantalla por usuario...");
            if let Err(e) = send_ipc_message(&IpcMessage::StopScreenRecording) {
                warn!("Error al enviar parada de grabación: {e}");
            }
            window.close();
            loop_quit.quit();
        });
    }

    main_loop.run();
    // Pausa breve para que ffmpeg cierre el contenedor MP4
    std::thread::sleep(Duration::from_millis(400));
    Ok(())
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
