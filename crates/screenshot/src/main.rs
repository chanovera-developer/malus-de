use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use glib::ControlFlow;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, Label, Orientation, Picture,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use libadwaita as adw;
use malus_de_common::ipc::{send_ipc_message, IpcMessage};
use malus_de_common::logging::init_logging;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
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
        help = "Modo de captura (por ahora solo full está implementado)"
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

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(15);

fn main() -> Result<()> {
    init_logging("malus_de_screenshot");
    let args = Args::parse();
    info!("Iniciando captura con modo: {:?}", args.mode);

    match args.mode {
        CaptureMode::Full => {
            let source = request_compositor_capture()?;
            if args.clipboard_only {
                let copy_result = copy_image_to_clipboard(&source);
                remove_temporary_capture(&source);
                copy_result?;
                info!("Captura copiada al portapapeles.");
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
            show_thumbnail_app(args.mode, path, clipboard_status);
        }
        CaptureMode::Area | CaptureMode::Window => {
            anyhow::bail!("Los modos de área y ventana aún no están implementados");
        }
        CaptureMode::Ocr => anyhow::bail!("El modo OCR aún no está implementado"),
    }

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
    send_ipc_message(&IpcMessage::CaptureScreenshot {
        destination: destination.to_string_lossy().into_owned(),
    })
    .context("No se pudo enviar la solicitud al compositor Malus")?;

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
        }

        let card = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .css_classes(["thumbnail-card"])
            .spacing(4)
            .build();

        let preview_img = Picture::for_filename(&capture_path);
        preview_img.set_content_fit(gtk4::ContentFit::Contain);
        preview_img.set_size_request(320, 180);
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
