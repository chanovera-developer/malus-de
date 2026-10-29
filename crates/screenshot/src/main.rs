use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use glib::ControlFlow;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, DrawingArea,
    EventControllerKey, GestureDrag, Label, Orientation, Overlay, Picture,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use libadwaita as adw;
use malus_de_common::ipc::{send_ipc_message, IpcMessage};
use malus_de_common::logging::init_logging;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Duration;
use tracing::info;

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureMode {
    Area,
    Window,
    Full,
    Ocr,
}

#[derive(Parser, Debug)]
#[command(name = "malus-de-screenshot")]
#[command(about = "Herramienta de Captura de Pantalla y Extractor OCR de Malus DE")]
struct Args {
    #[arg(
        short,
        long,
        value_enum,
        default_value_t = CaptureMode::Full,
        help = "Modo de captura"
    )]
    mode: CaptureMode,

    #[arg(
        short,
        long,
        help = "Copia la captura al portapapeles sin guardarla en Imágenes"
    )]
    clipboard_only: bool,
}

const THUMBNAIL_CSS: &str = r#"
window.malus-screenshot-thumbnail {
    background-color: transparent;
}

.thumbnail-card {
    background-color: alpha(@window_bg_color, 0.90);
    border: 1px solid alpha(@window_fg_color, 0.16);
    border-radius: 14px;
    box-shadow: 0 16px 36px rgba(0, 0, 0, 0.55);
    padding: 10px;
    min-width: 240px;
}

.thumbnail-title {
    font-size: 13px;
    font-weight: 600;
    color: @window_fg_color;
    margin-bottom: 2px;
}

.thumbnail-subtitle {
    font-size: 11px;
    color: alpha(@window_fg_color, 0.68);
    margin-bottom: 6px;
}

.action-btn {
    border-radius: 6px;
    padding: 4px 10px;
    font-size: 12px;
    font-weight: 500;
}
"#;

const CAPTURE_FLASH_CSS: &str = r#"
window.malus-screenshot-flash {
    background-color: rgba(255, 255, 255, 0.82);
}
"#;

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(15);
const THUMBNAIL_PREVIEW_SIZE: (i32, i32) = (320, 180);

fn main() -> Result<()> {
    init_logging("malus_de_screenshot");
    let args = Args::parse();
    info!("Iniciando captura con modo: {:?}", args.mode);

    match args.mode {
        CaptureMode::Full => {
            let source = request_compositor_capture()?;
            finish_capture(args.mode, source, args.clipboard_only)?;
        }
        CaptureMode::Area => {
            let selection = select_area()?;
            let source = request_compositor_capture()?;
            let crop_result = crop_capture(&source, &selection);
            remove_temporary_capture(&source);
            finish_capture(CaptureMode::Area, crop_result?, args.clipboard_only)?;
        }
        CaptureMode::Window => {
            anyhow::bail!("El modo de ventana aún no está implementado");
        }
        CaptureMode::Ocr => anyhow::bail!("El modo OCR aún no está implementado"),
    }

    Ok(())
}

fn finish_capture(mode: CaptureMode, source: PathBuf, clipboard_only: bool) -> Result<()> {
    if clipboard_only {
        let copy_result = copy_image_to_clipboard(&source);
        remove_temporary_capture(&source);
        copy_result?;
        info!("Captura copiada al portapapeles.");
        show_capture_feedback();
        send_capture_notification("Captura lista", "La captura se copió al portapapeles.");
        return Ok(());
    }

    let save_result = save_capture(source.clone(), &screenshots_directory());
    remove_temporary_capture(&source);
    let path = save_result?;
    info!("Captura guardada en {}", path.display());
    let clipboard_status = match copy_image_to_clipboard(&path) {
        Ok(()) => {
            info!("Captura copiada al portapapeles.");
            "Captura copiada al portapapeles.".to_string()
        }
        Err(error) => {
            let message = format!("No se pudo copiar al portapapeles: {error:#}");
            tracing::warn!("{message}");
            message
        }
    };
    show_capture_feedback();
    send_capture_notification(
        "Captura de pantalla",
        &format!("Guardada en {}", path.display()),
    );
    show_thumbnail_app(mode, path, clipboard_status);
    Ok(())
}

fn send_capture_notification(summary: &str, body: &str) {
    let result = zbus::block_on(async {
        let connection = zbus::Connection::session().await?;
        connection
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &(
                    "Malus DE",
                    0u32,
                    "camera-photo-symbolic",
                    summary,
                    body,
                    Vec::<String>::new(),
                    HashMap::<String, zbus::zvariant::Value<'_>>::new(),
                    5000i32,
                ),
            )
            .await?;
        Ok::<(), zbus::Error>(())
    });
    if let Err(error) = result {
        tracing::warn!("No se pudo mostrar el aviso de captura: {error}");
    }
}

fn show_capture_feedback() {
    play_capture_sound();

    if let Err(error) = show_capture_flash() {
        tracing::warn!("No se pudo mostrar el destello de captura: {error:#}");
    }
}

fn play_capture_sound() {
    std::thread::spawn(|| {
        match Command::new("canberra-gtk-play")
            .args(["--id", "camera-shutter"])
            .status()
        {
            Ok(status) if status.success() => return,
            Ok(status) => {
                tracing::warn!("El sonido de cámara con libcanberra terminó con error: {status}")
            }
            Err(error) => {
                tracing::warn!("No se pudo iniciar el sonido de cámara con libcanberra: {error}")
            }
        }

        let sound_file = "/usr/share/sounds/freedesktop/stereo/camera-shutter.oga";
        match Command::new("paplay").arg(sound_file).status() {
            Ok(status) if status.success() => {}
            Ok(status) => tracing::warn!("El sonido de captura con paplay terminó con error: {status}"),
            Err(error) => tracing::warn!("No se pudo iniciar paplay para el sonido de captura: {error}"),
        }
    });
}

fn show_capture_flash() -> Result<()> {
    let app = Application::builder()
        .application_id("org.malus.de.screenshot-flash")
        .build();

    app.connect_startup(|_| {
        let provider = CssProvider::new();
        provider.load_from_string(CAPTURE_FLASH_CSS);
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });

    app.connect_activate(|application| {
        let Some(display) = gtk4::gdk::Display::default() else {
            tracing::warn!("No se pudo mostrar el destello: no hay pantalla GTK disponible");
            application.quit();
            return;
        };
        let monitors = display.monitors();
        let mut windows = Vec::new();

        for index in 0..monitors.n_items() {
            let Some(monitor) = monitors
                .item(index)
                .and_then(|item| item.downcast::<gtk4::gdk::Monitor>().ok())
            else {
                continue;
            };

            let window = ApplicationWindow::builder()
                .application(application)
                .title("Destello de captura")
                .decorated(false)
                .css_classes(["malus-screenshot-flash"])
                .opacity(0.82)
                .build();

            if gtk4_layer_shell::is_supported() {
                window.init_layer_shell();
                window.set_layer(Layer::Overlay);
                window.set_namespace("malus-screenshot-flash");
                window.set_monitor(&monitor);
                window.set_exclusive_zone(-1);
                for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                    window.set_anchor(edge, true);
                }
            } else {
                window.fullscreen();
            }

            window.set_child(Some(&GtkBox::new(Orientation::Vertical, 0)));
            window.present();
            windows.push(window);
        }

        if windows.is_empty() {
            tracing::warn!("No se pudo mostrar el destello: no se encontraron pantallas");
            application.quit();
            return;
        }

        let app_for_timeout = application.clone();
        let mut step = 0u8;
        glib::timeout_add_local(Duration::from_millis(25), move || {
            step += 1;
            let opacity = 0.82 * (1.0 - f64::from(step) / 8.0);
            for window in &windows {
                window.set_opacity(opacity.max(0.0));
            }
            if step >= 8 {
                for window in &windows {
                    window.close();
                }
                app_for_timeout.quit();
                ControlFlow::Break
            } else {
                ControlFlow::Continue
            }
        });
    });

    app.run_with_args(&["malus-de-screenshot"]);
    Ok(())
}

fn request_compositor_capture() -> Result<PathBuf> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .context("XDG_RUNTIME_DIR no está definido")?;
    let token = format!(
        "malus-screenshot-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .context("El reloj del sistema está antes de UNIX_EPOCH")?
            .as_nanos()
    );
    let destination = runtime_dir.join(format!("{token}.png"));
    let message = IpcMessage::CaptureScreenshot {
        destination: destination.to_string_lossy().into_owned(),
    };
    send_ipc_message(&message).context("No se pudo enviar la solicitud al compositor Malus")?;

    info!("Solicitud enviada al compositor; esperando el fotograma capturado...");
    let error_path = destination.with_extension("error");
    let deadline = std::time::Instant::now() + CAPTURE_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if destination.is_file() {
            return Ok(destination);
        }
        if let Ok(error) = fs::read_to_string(&error_path) {
            let _ = fs::remove_file(&error_path);
            anyhow::bail!("El compositor no pudo capturar la pantalla: {error}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!(
        "El compositor no respondió a la solicitud de captura en {} segundos. \
         Recompila y reinicia el compositor Malus para habilitar la captura IPC",
        CAPTURE_TIMEOUT.as_secs()
    );
}

#[derive(Clone, Copy)]
struct SelectionRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    surface_width: f64,
    surface_height: f64,
}

struct AreaSelection {
    rect: SelectionRect,
}

fn select_area() -> Result<AreaSelection> {
    gtk4::init().context(
        "No se pudo inicializar GTK para seleccionar el área; verifica que haya una sesión gráfica activa",
    )?;
    let display = gtk4::gdk::Display::default()
        .context("No se pudo acceder a la pantalla para seleccionar el área")?;
    let monitors = display.monitors();
    let monitor = monitors
        .item(0)
        .and_then(|monitor| monitor.downcast::<gtk4::gdk::Monitor>().ok())
        .context("No se encontró una pantalla para seleccionar el área")?;
    let selected = Rc::new(RefCell::new(None));
    let selected_for_activate = selected.clone();
    let monitor_for_activate = monitor.clone();

    let app = Application::builder()
        .application_id("org.malus.de.screenshot-selector")
        .build();
    app.connect_startup(|_| {
        let provider = CssProvider::new();
        provider.load_from_string(
            "window.malus-screenshot-selector { background-color: transparent; } \
             .screenshot-selector-hint { \
                 color: white; background-color: alpha(#171717, 0.88); \
                 border-radius: 10px; padding: 10px 16px; font-size: 14px; \
             }",
        );
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
    app.connect_activate(move |application| {
        let window = ApplicationWindow::builder()
            .application(application)
            .title("Seleccionar área de captura")
            .decorated(false)
            .css_classes(["malus-screenshot-selector"])
            .build();
        let state = Rc::new(RefCell::new(None::<(f64, f64, f64, f64)>));
        let drawing_area = DrawingArea::new();
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);
        drawing_area.set_draw_func({
            let state = state.clone();
            move |_, context, _, _| {
                context.set_source_rgba(0.0, 0.0, 0.0, 0.42);
                let _ = context.paint();
                if let Some((start_x, start_y, end_x, end_y)) = *state.borrow() {
                    let x = start_x.min(end_x);
                    let y = start_y.min(end_y);
                    let rect_width = (start_x - end_x).abs();
                    let rect_height = (start_y - end_y).abs();
                    context.set_operator(gtk4::cairo::Operator::Clear);
                    context.rectangle(x, y, rect_width, rect_height);
                    let _ = context.fill();
                    context.set_operator(gtk4::cairo::Operator::Over);
                    context.set_source_rgba(1.0, 1.0, 1.0, 0.95);
                    context.set_line_width(2.0);
                    context.rectangle(x, y, rect_width, rect_height);
                    let _ = context.stroke();
                }
            }
        });

        let overlay = Overlay::new();
        overlay.set_child(Some(&drawing_area));
        let hint = Label::new(Some("Arrastra para seleccionar · Esc para cancelar"));
        hint.set_halign(Align::Center);
        hint.set_valign(Align::Start);
        hint.set_margin_top(28);
        hint.set_can_target(false);
        hint.add_css_class("screenshot-selector-hint");
        overlay.add_overlay(&hint);
        window.set_child(Some(&overlay));

        let drag = GestureDrag::new();
        let state_begin = state.clone();
        drag.connect_drag_begin(move |_, x, y| {
            *state_begin.borrow_mut() = Some((x, y, x, y));
        });
        let state_update = state.clone();
        let area_update = drawing_area.clone();
        drag.connect_drag_update(move |_, offset_x, offset_y| {
            let current = *state_update.borrow();
            if let Some((start_x, start_y, _, _)) = current {
                *state_update.borrow_mut() =
                    Some((start_x, start_y, start_x + offset_x, start_y + offset_y));
                area_update.queue_draw();
            }
        });
        let state_end = state.clone();
        let area_end = drawing_area.clone();
        let selected = selected_for_activate.clone();
        let window_end = window.clone();
        drag.connect_drag_end(move |_, offset_x, offset_y| {
            let current = *state_end.borrow();
            let Some((start_x, start_y, _, _)) = current else {
                return;
            };
            let end_x = start_x + offset_x;
            let end_y = start_y + offset_y;
            let surface_width = area_end.width().max(0) as f64;
            let surface_height = area_end.height().max(0) as f64;
            let x = start_x.min(end_x).clamp(0.0, surface_width);
            let y = start_y.min(end_y).clamp(0.0, surface_height);
            let right = start_x.max(end_x).clamp(0.0, surface_width);
            let bottom = start_y.max(end_y).clamp(0.0, surface_height);
            if right > x && bottom > y {
                *selected.borrow_mut() = Some(AreaSelection {
                    rect: SelectionRect {
                        x,
                        y,
                        width: right - x,
                        height: bottom - y,
                        surface_width,
                        surface_height,
                    },
                });
                window_end.close();
            }
        });
        drawing_area.add_controller(drag);

        let keys = EventControllerKey::new();
        let window_escape = window.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                window_escape.close();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        window.add_controller(keys);

        if gtk4_layer_shell::is_supported() {
            window.init_layer_shell();
            window.set_layer(Layer::Overlay);
            window.set_namespace("malus-screenshot-selector");
            window.set_monitor(&monitor_for_activate);
            window.set_keyboard_mode(KeyboardMode::Exclusive);
            window.set_exclusive_zone(-1);
            for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                window.set_anchor(edge, true);
            }
        } else {
            window.fullscreen();
        }
        window.present();
    });

    app.run_with_args(&["malus-de-screenshot"]);
    let selection = selected
        .borrow_mut()
        .take()
        .context("Se canceló la selección del área de captura")?;
    // Let the selector unmap before the compositor captures the next frame.
    std::thread::sleep(Duration::from_millis(100));
    Ok(selection)
}

fn crop_capture(source: &std::path::Path, selection: &AreaSelection) -> Result<PathBuf> {
    let file = File::open(source)
        .with_context(|| format!("No se pudo abrir la captura temporal {}", source.display()))?;
    let decoder = png::Decoder::new(file);
    let mut reader = decoder
        .read_info()
        .context("No se pudo leer la captura PNG")?;
    let mut input = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut input)
        .context("No se pudo decodificar la captura PNG")?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        anyhow::bail!("El formato de la captura PNG no es RGBA de 8 bits");
    }
    let expected_len = (info.width as usize)
        .checked_mul(info.height as usize)
        .and_then(|pixel_count| pixel_count.checked_mul(4))
        .context("La resolución de pantalla excede el tamaño permitido")?;
    if info.buffer_size() != expected_len {
        anyhow::bail!("La captura PNG tiene un tamaño de píxeles inesperado");
    }

    let (left, top, right, bottom) = pixel_bounds(selection.rect, info.width, info.height)?;
    let crop_width = right - left;
    let crop_height = bottom - top;
    let row_bytes = crop_width as usize * 4;
    let mut pixels = Vec::with_capacity(row_bytes * crop_height as usize);
    for row in top..bottom {
        let start = (row as usize * info.width as usize + left as usize) * 4;
        pixels.extend_from_slice(&input[start..start + row_bytes]);
    }

    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .context("El nombre de la captura temporal no es válido")?;
    let destination = source.with_file_name(format!("{stem}-area.png"));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .with_context(|| format!("No se pudo crear {}", destination.display()))?;
    let write_result = (|| -> Result<()> {
        let mut encoder = png::Encoder::new(file, crop_width, crop_height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .context("No se pudo iniciar la escritura de la captura recortada")?;
        writer
            .write_image_data(&pixels)
            .context("No se pudo escribir la captura recortada")?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&destination);
        return Err(error);
    }
    Ok(destination)
}

fn pixel_bounds(
    rect: SelectionRect,
    image_width: u32,
    image_height: u32,
) -> Result<(u32, u32, u32, u32)> {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || rect.surface_width <= 0.0
        || rect.surface_height <= 0.0
        || image_width == 0
        || image_height == 0
    {
        anyhow::bail!("La selección del área de captura no tiene dimensiones válidas");
    }
    let scale_x = image_width as f64 / rect.surface_width;
    let scale_y = image_height as f64 / rect.surface_height;
    let left = (rect.x * scale_x).floor().clamp(0.0, image_width as f64) as u32;
    let top = (rect.y * scale_y).floor().clamp(0.0, image_height as f64) as u32;
    let right = ((rect.x + rect.width) * scale_x)
        .ceil()
        .clamp(0.0, image_width as f64) as u32;
    let bottom = ((rect.y + rect.height) * scale_y)
        .ceil()
        .clamp(0.0, image_height as f64) as u32;
    if right <= left || bottom <= top {
        anyhow::bail!("La selección del área de captura está vacía");
    }
    Ok((left, top, right, bottom))
}

#[cfg(test)]
mod tests {
    use super::{crop_capture, pixel_bounds, AreaSelection, SelectionRect};
    use std::fs::{self, File, OpenOptions};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_png_path(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "malus-screenshot-{label}-{}-{nonce}.png",
            std::process::id()
        ))
    }

    #[test]
    fn pixel_bounds_scales_selection_to_capture_resolution() {
        let selection = SelectionRect {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
            surface_width: 100.0,
            surface_height: 100.0,
        };

        assert_eq!(
            pixel_bounds(selection, 200, 300).unwrap(),
            (20, 60, 80, 180)
        );
    }

    #[test]
    fn pixel_bounds_rejects_empty_selection() {
        let selection = SelectionRect {
            x: 10.0,
            y: 20.0,
            width: 0.0,
            height: 40.0,
            surface_width: 100.0,
            surface_height: 100.0,
        };

        assert!(pixel_bounds(selection, 200, 300).is_err());
    }

    #[test]
    fn crop_capture_encodes_only_selected_pixels() {
        let source = temporary_png_path("source");
        let mut encoder = png::Encoder::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&source)
                .unwrap(),
            2,
            2,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255])
            .unwrap();

        let selection = AreaSelection {
            rect: SelectionRect {
                x: 1.0,
                y: 1.0,
                width: 1.0,
                height: 1.0,
                surface_width: 2.0,
                surface_height: 2.0,
            },
        };
        let cropped = crop_capture(&source, &selection).unwrap();
        let mut reader = png::Decoder::new(File::open(&cropped).unwrap())
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut pixels).unwrap();

        assert_eq!((info.width, info.height), (1, 1));
        assert_eq!(&pixels[..info.buffer_size()], &[10, 11, 12, 255]);

        fs::remove_file(cropped).unwrap();
        fs::remove_file(source).unwrap();
    }
}

fn save_capture(source: PathBuf, destination_dir: &std::path::Path) -> Result<PathBuf> {
    fs::create_dir_all(destination_dir)
        .with_context(|| format!("No se pudo crear {}", destination_dir.display()))?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("El reloj del sistema está antes de UNIX_EPOCH")?
        .as_secs();
    let stem = format!("Captura de pantalla-{timestamp}");
    for index in 0..=u32::MAX {
        let name = if index == 0 {
            format!("{stem}.png")
        } else {
            format!("{stem}-{index}.png")
        };
        let destination = destination_dir.join(name);
        let mut target = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
        {
            Ok(target) => target,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("No se pudo crear {}", destination.display()));
            }
        };
        let mut input = fs::File::open(&source).with_context(|| {
            format!("No se pudo abrir la captura temporal {}", source.display())
        })?;
        if let Err(error) = std::io::copy(&mut input, &mut target).and_then(|_| target.flush()) {
            let _ = fs::remove_file(&destination);
            return Err(error).with_context(|| {
                format!("No se pudo guardar la captura en {}", destination.display())
            });
        }
        return Ok(destination);
    }
    anyhow::bail!("No se encontró un nombre libre para guardar la captura")
}

fn screenshots_directory() -> PathBuf {
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
    let pictures_dir = std::env::var_os("XDG_PICTURES_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| read_xdg_pictures_dir(&config_dir.join("user-dirs.dirs"), &home))
        .unwrap_or_else(|| home.join(if spanish { "Imágenes" } else { "Pictures" }));
    pictures_dir.join(if spanish {
        "Capturas de pantalla"
    } else {
        "Screenshots"
    })
}

fn read_xdg_pictures_dir(path: &std::path::Path, home: &std::path::Path) -> Option<PathBuf> {
    let contents = fs::read_to_string(path).ok()?;
    let value = contents
        .lines()
        .find_map(|line| line.strip_prefix("XDG_PICTURES_DIR=\""))?
        .strip_suffix('"')?;

    let resolved = value
        .strip_prefix("$HOME")
        .map(|suffix| home.join(suffix.strip_prefix('/').unwrap_or(suffix)))
        .unwrap_or_else(|| PathBuf::from(value));
    resolved.is_absolute().then_some(resolved)
}

fn remove_temporary_capture(path: &std::path::Path) {
    if let Err(error) = fs::remove_file(path) {
        tracing::warn!(
            "No se pudo eliminar el archivo temporal {}: {error}",
            path.display()
        );
    }
}

fn copy_image_to_clipboard(path: &std::path::Path) -> Result<()> {
    let mut source = fs::File::open(path)
        .with_context(|| format!("No se pudo abrir la captura {}", path.display()))?;
    let mut child = Command::new("wl-copy")
        .args(["--type", "image/png"])
        .stdin(Stdio::piped())
        .spawn()
        .context("No se pudo iniciar wl-copy; instala wl-clipboard para copiar capturas")?;
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!("wl-copy no abrió la entrada de datos");
    };
    let write_result = std::io::copy(&mut source, &mut stdin).and_then(|_| stdin.flush());
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error).context("No se pudo transferir la captura a wl-copy");
    }
    let status = child.wait().context("No se pudo esperar a wl-copy")?;
    if !status.success() {
        anyhow::bail!("wl-copy terminó con error: {status}");
    }
    Ok(())
}

fn show_thumbnail_app(mode: CaptureMode, capture_path: PathBuf, clipboard_status: String) {
    if let Err(err) = adw::init() {
        tracing::warn!("Aviso al inicializar libadwaita: {:?}", err);
    }

    let app = Application::builder()
        .application_id("org.malus.de.screenshot-thumbnail")
        .build();

    app.connect_startup(|_| {
        let provider = CssProvider::new();
        provider.load_from_string(THUMBNAIL_CSS);
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });

    app.connect_activate(move |application| {
        let window = ApplicationWindow::builder()
            .application(application)
            .title("Captura de pantalla")
            .css_classes(["malus-screenshot-thumbnail"])
            .default_width(360)
            .default_height(300)
            .build();

        if gtk4_layer_shell::is_supported() {
            window.init_layer_shell();
            window.set_layer(Layer::Overlay);
            window.set_namespace("malus-screenshot-thumbnail");
            // Anclar a esquina inferior derecha estilo macOS
            window.set_anchor(Edge::Bottom, true);
            window.set_anchor(Edge::Right, true);
            window.set_margin(Edge::Bottom, 20);
            window.set_margin(Edge::Right, 20);
        } else {
            window.set_decorated(false);
            window.set_default_size(360, 300);
        }

        let card = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .css_classes(["thumbnail-card"])
            .spacing(4)
            .build();

        let preview_img = Picture::for_filename(&capture_path);
        preview_img.set_content_fit(gtk4::ContentFit::Contain);
        preview_img.set_can_shrink(true);
        preview_img.set_size_request(THUMBNAIL_PREVIEW_SIZE.0, THUMBNAIL_PREVIEW_SIZE.1);
        card.append(&preview_img);

        let title_label = Label::builder()
            .label(match mode {
                CaptureMode::Area => "Captura de Área",
                CaptureMode::Window => "Captura de Ventana",
                CaptureMode::Full => "Captura de Pantalla Completa",
                CaptureMode::Ocr => "Texto Extraído",
            })
            .css_classes(["thumbnail-title"])
            .build();
        card.append(&title_label);

        let sub_label = Label::builder()
            .label(&capture_path.display().to_string())
            .css_classes(["thumbnail-subtitle"])
            .build();
        sub_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        sub_label.set_max_width_chars(42);
        card.append(&sub_label);

        let status_label = Label::builder()
            .label(&clipboard_status)
            .wrap(true)
            .css_classes(["thumbnail-subtitle"])
            .build();
        card.append(&status_label);

        let btn_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .build();

        let copy_btn = Button::builder()
            .label("Copiar")
            .css_classes(["action-btn", "suggested-action"])
            .build();
        let win_clone = window.clone();
        let status_label_for_copy = status_label.clone();
        let capture_path_for_copy = capture_path.clone();
        copy_btn.connect_clicked(
            move |_| match copy_image_to_clipboard(&capture_path_for_copy) {
                Ok(()) => {
                    info!("Captura copiada al portapapeles.");
                    win_clone.close();
                }
                Err(error) => {
                    let message = format!("No se pudo copiar al portapapeles: {error:#}");
                    status_label_for_copy.set_label(&message);
                    tracing::error!("{message}");
                }
            },
        );
        btn_box.append(&copy_btn);

        let dismiss_btn = Button::builder()
            .label("Cerrar")
            .css_classes(["action-btn"])
            .build();
        let win_clone2 = window.clone();
        dismiss_btn.connect_clicked(move |_| {
            win_clone2.close();
        });
        btn_box.append(&dismiss_btn);

        card.append(&btn_box);
        window.set_child(Some(&card));
        window.present();

        // Auto-cerrar miniatura tras 5 segundos si el usuario no interactúa
        let win_auto = window.clone();
        glib::timeout_add_local(Duration::from_secs(5), move || {
            win_auto.close();
            ControlFlow::Break
        });
    });

    app.run_with_args(&["malus-de-screenshot"]);
}
