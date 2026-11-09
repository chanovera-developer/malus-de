use anyhow::Result;
use clap::Parser;
use gio::prelude::*;
use gtk4::gdk::Key;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, Entry,
    EventControllerKey, Fixed, GestureClick, GestureDrag, Image, Label, Orientation,
    Overlay, Picture, Popover, Stack, StackTransitionType,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use malus_de_common::{
    config::{DesktopIconsConfig, DisplaysConfig},
    logging::init_logging,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(name = "malus-de-wallpaper")]
#[command(about = "Demonio de Fondo de Pantalla y Superficie de Escritorio de Malus DE (Layer Shell)")]
struct Args {
    /// Ruta a la imagen de fondo de pantalla
    #[arg(short, long)]
    image: Option<PathBuf>,
}

const DEFAULT_GRADIENT_CSS: &str = include_str!("styles/default-gradient.css");

/// Margen superior de la cuadrícula de iconos del escritorio para dejar respiración bajo la Top Bar (36px).
pub const DESKTOP_TOP_MARGIN: f64 = 56.0;
/// Margen superior estándar de la cuadrícula en pantallas secundarias (sin Top Bar).
pub const DESKTOP_SECONDARY_TOP_MARGIN: f64 = 16.0;

static CURRENT_IS_DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

thread_local! {
    static REGISTERED_POPOVERS: RefCell<Vec<glib::WeakRef<Popover>>> = RefCell::new(Vec::new());
    static DESKTOP_LAYOUT_ANIM_GEN: Cell<u64> = const { Cell::new(0) };
}

pub fn is_dark_mode_active() -> bool {
    CURRENT_IS_DARK.load(std::sync::atomic::Ordering::SeqCst)
}

pub fn set_is_dark_mode(is_dark: bool) {
    CURRENT_IS_DARK.store(is_dark, std::sync::atomic::Ordering::SeqCst);
    if let Some(gtk_settings) = gtk4::Settings::default() {
        gtk_settings.set_gtk_application_prefer_dark_theme(is_dark);
    }
    update_all_registered_popovers(is_dark);
}

pub fn register_wallpaper_popover(popover: &Popover) {
    let is_dark = is_dark_mode_active();
    apply_theme_to_popover(popover, is_dark);

    let pop_weak = popover.downgrade();
    popover.connect_show(move |_| {
        if let Some(pop) = pop_weak.upgrade() {
            apply_theme_to_popover(&pop, is_dark_mode_active());
        }
    });

    REGISTERED_POPOVERS.with(|list| {
        let mut l = list.borrow_mut();
        l.retain(|w| w.upgrade().is_some());
        l.push(popover.downgrade());
    });
}

pub fn update_all_registered_popovers(is_dark: bool) {
    REGISTERED_POPOVERS.with(|list| {
        let mut l = list.borrow_mut();
        l.retain(|w| {
            if let Some(pop) = w.upgrade() {
                apply_theme_to_popover(&pop, is_dark);
                true
            } else {
                false
            }
        });
    });
}

pub fn apply_theme_to_popover(popover: &Popover, is_dark: bool) {
    if is_dark {
        popover.remove_css_class("light-popover");
    } else {
        popover.add_css_class("light-popover");
    }
    update_menu_text_theme(popover.upcast_ref(), is_dark);
    popover.queue_draw();
}

fn update_menu_text_theme(widget: &gtk4::Widget, is_dark: bool) {
    if let Some(label) = widget.downcast_ref::<Label>() {
        label.remove_css_class("menu-light-text");
        label.remove_css_class("menu-dark-text");
        label.add_css_class(if is_dark {
            "menu-dark-text"
        } else {
            "menu-light-text"
        });
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        update_menu_text_theme(&current, is_dark);
    }
}

/// Curva de aceleración cúbica ease-out (estilo macOS) para transiciones fluidas.
pub fn ease_out_cubic(t: f64) -> f64 {
    let progress = t.clamp(0.0, 1.0);
    1.0 - (1.0 - progress).powi(3)
}

/// Anima suavemente la opacidad de un widget de 0.0 a 1.0 mediante el frame-clock nativo de GTK4
/// con aceleración cubic ease-out, garantizando 100% de alineación sin desfases de geometría.
pub fn animate_widget_fade_in<W: IsA<gtk4::Widget>>(widget: &W, duration_ms: i64) {
    widget.set_opacity(0.0);
    let start_time: Rc<Cell<Option<i64>>> = Rc::new(Cell::new(None));
    let start_time_clone = start_time.clone();

    widget.add_tick_callback(move |w, frame_clock| {
        // Esperar a que el widget esté efectivamente mapeado en pantalla antes de iniciar el conteo
        if !w.is_mapped() {
            if start_time_clone.get().is_some() {
                // Si ya había iniciado y se desmapeó (ej. popover cerrado por el usuario), cancelar
                return glib::ControlFlow::Break;
            }
            return glib::ControlFlow::Continue;
        }

        let now = frame_clock.frame_time() / 1000;
        let start = match start_time_clone.get() {
            Some(t) => t,
            None => {
                start_time_clone.set(Some(now));
                now
            }
        };

        let elapsed = now - start;
        if elapsed >= duration_ms {
            w.set_opacity(1.0);
            glib::ControlFlow::Break
        } else {
            let progress = elapsed as f64 / duration_ms as f64;
            let eased = ease_out_cubic(progress);
            w.set_opacity(eased);
            glib::ControlFlow::Continue
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StackCategory {
    Screenshots,
    Images,
    Documents,
    Videos,
    Audio,
    Code,
    Archives,
    Folders,
    Other,
}

impl StackCategory {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Screenshots => "Capturas de pantalla",
            Self::Images => "Imágenes",
            Self::Documents => "Documentos",
            Self::Videos => "Vídeos",
            Self::Audio => "Música y Audio",
            Self::Code => "Código y Scripts",
            Self::Archives => "Comprimidos",
            Self::Folders => "Carpetas",
            Self::Other => "Otros",
        }
    }

    pub fn icon_name(&self) -> &'static str {
        match self {
            Self::Screenshots => "camera-photo-symbolic",
            Self::Images => "folder-pictures-symbolic",
            Self::Documents => "folder-documents-symbolic",
            Self::Videos => "folder-videos-symbolic",
            Self::Audio => "folder-music-symbolic",
            Self::Code => "text-x-script-symbolic",
            Self::Archives => "package-x-generic-symbolic",
            Self::Folders => "folder-symbolic",
            Self::Other => "text-x-generic-symbolic",
        }
    }

    pub fn categorize(path: &Path, content_type: Option<&str>) -> Self {
        if path.is_dir() {
            return Self::Folders;
        }

        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        if file_name.starts_with("captura")
            || file_name.starts_with("screenshot")
            || file_name.contains("screengrab")
        {
            return Self::Screenshots;
        }

        if let Some(ct) = content_type {
            if ct.starts_with("image/") {
                return Self::Images;
            }
            if ct.starts_with("video/") {
                return Self::Videos;
            }
            if ct.starts_with("audio/") {
                return Self::Audio;
            }
            if ct == "application/pdf" {
                return Self::Documents;
            }
        }

        match extension.as_str() {
            "jpg" | "jpeg" | "png" | "webp" | "svg" | "gif" | "avif" | "heic" | "bmp" => {
                Self::Images
            }
            "pdf" | "doc" | "docx" | "txt" | "md" | "rtf" | "odt" | "xls" | "xlsx" | "ppt"
            | "pptx" | "csv" => Self::Documents,
            "mp4" | "mkv" | "webm" | "mov" | "avi" | "flv" => Self::Videos,
            "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" => Self::Audio,
            "rs" | "py" | "js" | "ts" | "c" | "cpp" | "h" | "html" | "css" | "json" | "toml"
            | "sh" | "bash" | "yml" | "yaml" => Self::Code,
            "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" => Self::Archives,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconLayoutMetrics {
    pub icon_size: i32,
    pub box_width: i32,
    pub box_height: i32,
    pub spacing: i32,
}

impl IconLayoutMetrics {
    pub fn from_config(config: &DesktopIconsConfig) -> Self {
        let icon_size = config.clamped_icon_size();
        let box_width = (icon_size + 44).max(72);
        let box_height = (icon_size + 48).max(76);
        let spacing = 8;
        Self {
            icon_size,
            box_width,
            box_height,
            spacing,
        }
    }

    pub fn cell_width(&self) -> i32 {
        self.box_width + self.spacing
    }

    pub fn cell_height(&self) -> i32 {
        self.box_height + self.spacing
    }
}

fn icon_positions_path() -> Option<PathBuf> {
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    if config_dir.ends_with("malus-de") {
        Some(config_dir.join("desktop-icons.json"))
    } else {
        Some(config_dir.join("malus-de/desktop-icons.json"))
    }
}

fn load_icon_positions() -> HashMap<PathBuf, (f64, f64)> {
    let Some(path) = icon_positions_path() else {
        warn!("No se pudo determinar dónde guardar las posiciones de los iconos.");
        return HashMap::new();
    };

    match fs::read(&path) {
        Ok(contents) => match serde_json::from_slice(&contents) {
            Ok(positions) => positions,
            Err(error) => {
                warn!(
                    "No se pudieron leer las posiciones de iconos {:?}: {}",
                    path, error
                );
                HashMap::new()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(error) => {
            warn!(
                "No se pudieron leer las posiciones de iconos {:?}: {}",
                path, error
            );
            HashMap::new()
        }
    }
}

fn save_icon_positions(positions: &HashMap<PathBuf, (f64, f64)>) {
    let Some(path) = icon_positions_path() else {
        warn!("No se pudo determinar dónde guardar las posiciones de los iconos.");
        return;
    };
    let Some(parent) = path.parent() else {
        warn!(
            "La ruta de posiciones de iconos no tiene carpeta padre: {:?}",
            path
        );
        return;
    };

    let result = (|| -> std::io::Result<()> {
        fs::create_dir_all(parent)?;
        let contents = serde_json::to_vec(positions).map_err(std::io::Error::other)?;
        let temporary_path = path.with_extension("json.tmp");
        fs::write(&temporary_path, contents)?;
        fs::rename(&temporary_path, &path)
    })();
    if let Err(error) = result {
        warn!(
            "No se pudieron guardar las posiciones de iconos {:?}: {}",
            path, error
        );
    }
}

fn find_icon_at(
    fixed: &Fixed,
    x: f64,
    y: f64,
    rubberband: &impl IsA<gtk4::Widget>,
) -> Option<(PathBuf, gtk4::Widget, (f64, f64))> {
    let mut child = fixed.last_child();
    let rb_widget = rubberband.as_ref();
    while let Some(item) = child {
        child = item.prev_sibling();
        if item == *rb_widget {
            continue;
        }
        let (ix, iy) = fixed.child_position(&item);
        let iw = item.width().max(item.width_request()).max(64) as f64;
        let ih = item.height().max(item.height_request()).max(64) as f64;
        if x >= ix && x <= ix + iw && y >= iy && y <= iy + ih {
            if let Some(path_str) = item.property::<Option<String>>("tooltip-text") {
                let p = PathBuf::from(path_str);
                if p.symlink_metadata().is_ok() {
                    let pointer_offset = (x - ix, y - iy);
                    return Some((p, item, pointer_offset));
                }
            }
        }
    }
    None
}

fn is_point_over_icon(fixed: &Fixed, x: f64, y: f64, rubberband: &impl IsA<gtk4::Widget>) -> bool {
    let mut child = fixed.first_child();
    let rb_widget = rubberband.as_ref();
    while let Some(item) = child {
        child = item.next_sibling();
        if item == *rb_widget {
            continue;
        }
        let (ix, iy) = fixed.child_position(&item);
        let iw = item.width().max(item.width_request()).max(64) as f64;
        let ih = item.height().max(item.height_request()).max(64) as f64;
        if x >= ix && x <= ix + iw && y >= iy && y <= iy + ih {
            return true;
        }
    }
    false
}

fn move_desktop_icon(fixed: &Fixed, path: &Path, position: (f64, f64)) -> bool {
    let expected_path = path.to_string_lossy();
    let mut child = fixed.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        let item_path = item.property::<Option<String>>("tooltip-text");
        if item_path.as_deref() == Some(expected_path.as_ref()) {
            fixed.move_(&item, position.0, position.1);
            return true;
        }
    }
    false
}

pub fn clamp_icon_position(
    x: f64,
    y: f64,
    pointer_offset: (f64, f64),
    metrics: &IconLayoutMetrics,
    desktop_size: (i32, i32),
    top_margin: f64,
) -> (f64, f64) {
    let min_x = 16.0;
    let max_x = (desktop_size.0 - metrics.box_width - 16).max(16) as f64;
    let min_y = top_margin;
    let max_y = (desktop_size.1 - metrics.box_height - 20).max(min_y as i32) as f64;

    (
        (x - pointer_offset.0).clamp(min_x, max_x),
        (y - pointer_offset.1).clamp(min_y, max_y),
    )
}

pub fn dragged_icon_position(
    original_position: (f64, f64),
    pointer_offset: (f64, f64),
    drag_offset: (f64, f64),
    metrics: &IconLayoutMetrics,
    desktop_size: (i32, i32),
    top_margin: f64,
) -> (f64, f64) {
    clamp_icon_position(
        original_position.0 + pointer_offset.0 + drag_offset.0,
        original_position.1 + pointer_offset.1 + drag_offset.1,
        pointer_offset,
        metrics,
        desktop_size,
        top_margin,
    )
}

pub fn snap_position_to_grid(
    pos: (f64, f64),
    metrics: &IconLayoutMetrics,
    align_left: bool,
    desktop_size: (i32, i32),
    top_margin: f64,
) -> (f64, f64) {
    let cell_w = metrics.cell_width() as f64;
    let cell_h = metrics.cell_height() as f64;
    let snapped_y = top_margin + ((pos.1 - top_margin).max(0.0) / cell_h).round() * cell_h;

    let snapped_x = if align_left {
        let base_x = 16.0;
        let col = ((pos.0 - base_x).max(0.0) / cell_w).round();
        base_x + col * cell_w
    } else {
        let base_x = (desktop_size.0 - metrics.box_width - 16).max(16) as f64;
        let col = ((base_x - pos.0).max(0.0) / cell_w).round();
        (base_x - col * cell_w).max(16.0)
    };

    (
        snapped_x.clamp(16.0, (desktop_size.0 - metrics.box_width - 16).max(16) as f64),
        snapped_y.clamp(top_margin, (desktop_size.1 - metrics.box_height - 20).max(top_margin as i32) as f64),
    )
}

pub fn calculate_grid_position(
    index: usize,
    rows: usize,
    metrics: &IconLayoutMetrics,
    align_left: bool,
    desktop_size: (i32, i32),
    top_margin: f64,
) -> (f64, f64) {
    let rows = rows.max(1);
    let column = index / rows;
    let row = index % rows;
    let y = top_margin + (row as i32 * metrics.cell_height()) as f64;
    let x = if align_left {
        16.0 + (column as i32 * metrics.cell_width()) as f64
    } else {
        (desktop_size.0 - metrics.box_width - 16 - column as i32 * metrics.cell_width()).max(16) as f64
    };
    (
        x.clamp(16.0, (desktop_size.0 - metrics.box_width - 16).max(16) as f64),
        y.clamp(top_margin, (desktop_size.1 - metrics.box_height - 20).max(top_margin as i32) as f64),
    )
}

fn resolve_desktop_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let user_dirs_file = config_home.join("user-dirs.dirs");

    let contents = match fs::read_to_string(&user_dirs_file) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            warn!("No se pudo leer {:?}: {}", user_dirs_file, error);
            String::new()
        }
    };

    Some(desktop_dir_from_config(&contents, &home))
}

fn desktop_dir_from_config(contents: &str, home: &Path) -> PathBuf {
    contents
        .lines()
        .find_map(|line| {
            let value = line.trim().strip_prefix("XDG_DESKTOP_DIR=")?;
            Some(path_from_user_dir_value(value, home))
        })
        .unwrap_or_else(|| home.join("Escritorio"))
}

fn path_from_user_dir_value(value: &str, home: &Path) -> PathBuf {
    let value = value.trim();
    let value = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value);

    if value == "$HOME" || value == "${HOME}" {
        return home.to_path_buf();
    }
    if let Some(relative) = value
        .strip_prefix("$HOME/")
        .or_else(|| value.strip_prefix("${HOME}/"))
    {
        return home.join(relative);
    }

    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        home.join(path)
    }
}

fn launch_file(file: &gio::File) {
    let target = file
        .path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.uri().to_string());
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(&target);
    malus_de_common::config::set_cursor_environment(&mut cmd);
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let malus_config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&home).join(".config/malus-de"));
    cmd.env("XDG_CONFIG_HOME", malus_config_dir);
    cmd.env("GSETTINGS_BACKEND", "keyfile");
    cmd.env("GTK_USE_PORTAL", "1");
    cmd.env("XDG_CURRENT_DESKTOP", "Malus:GNOME");
    cmd.env("GDK_BACKEND", "wayland");
    if let Err(error) = cmd.spawn() {
        warn!("No se pudo abrir {:?}: {}", file, error);
    }
}

fn launch_quick_look(path: &Path) {
    let mut cmd = std::process::Command::new("malus-de-branches");
    cmd.arg("-q").arg(path);
    malus_de_common::config::set_cursor_environment(&mut cmd);
    if let Err(err) = cmd.spawn() {
        warn!(
            "No se pudo invocar Quick Look para {:?}: {}. Fallback a xdg-open...",
            path, err
        );
        let file = gio::File::for_path(path);
        launch_file(&file);
    }
}

fn trash_file(path: &Path) {
    let file = gio::File::for_path(path);
    if let Err(err) = file.trash(gio::Cancellable::NONE) {
        warn!("Error al enviar archivo a la papelera {:?}: {}", path, err);
    }
}

fn duplicate_file(path: &Path) {
    if let Some(parent) = path.parent() {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("archivo");
        let ext = path.extension().and_then(|e| e.to_str());
        let new_name = match ext {
            Some(e) => format!("{} copia.{}", stem, e),
            None => format!("{} copia", stem),
        };
        let dest = parent.join(new_name);
        let src_file = gio::File::for_path(path);
        let dest_file = gio::File::for_path(&dest);
        let _ = src_file.copy(
            &dest_file,
            gio::FileCopyFlags::NONE,
            gio::Cancellable::NONE,
            None,
        );
    }
}

fn show_rename_dialog(parent_widget: &impl IsA<gtk4::Widget>, current_path: PathBuf) {
    let popover = Popover::new();
    popover.add_css_class("desktop-popover-menu");
    register_wallpaper_popover(&popover);
    popover.set_parent(parent_widget);
    popover.set_autohide(true);

    let container = GtkBox::new(Orientation::Vertical, 8);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);
    container.add_css_class("desktop-context-menu");

    let title_lbl = Label::builder()
        .label("Renombrar")
        .halign(Align::Start)
        .css_classes(["card-section-title"])
        .build();
    container.append(&title_lbl);

    let initial_name = current_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let entry = Entry::builder()
        .text(&initial_name)
        .hexpand(true)
        .build();
    container.append(&entry);

    let btn_box = GtkBox::new(Orientation::Horizontal, 6);
    btn_box.set_halign(Align::End);

    let cancel_btn = Button::with_label("Cancelar");
    cancel_btn.add_css_class("desktop-context-btn");
    let popover_close = popover.clone();
    cancel_btn.connect_clicked(move |_| {
        popover_close.popdown();
    });
    btn_box.append(&cancel_btn);

    let save_btn = Button::with_label("Aceptar");
    save_btn.add_css_class("desktop-context-btn");
    save_btn.add_css_class("suggested-action");

    let popover_save = popover.clone();
    let entry_save = entry.clone();
    let path_save = current_path;
    save_btn.connect_clicked(move |_| {
        let new_name = entry_save.text().trim().to_string();
        if !new_name.is_empty() && new_name != initial_name {
            if let Some(parent) = path_save.parent() {
                let dest = parent.join(&new_name);
                if let Err(err) = fs::rename(&path_save, &dest) {
                    warn!("No se pudo renombrar {:?} a {:?}: {}", path_save, dest, err);
                }
            }
        }
        popover_save.popdown();
    });
    btn_box.append(&save_btn);
    container.append(&btn_box);

    popover.set_child(Some(&container));
    apply_theme_to_popover(&popover, is_dark_mode_active());
    popover.connect_closed(|p| p.unparent());
    animate_widget_fade_in(&container, 140);
    popover.popup();
}

fn get_unique_folder_name(dir: &Path, base_name: &str) -> PathBuf {
    let candidate = dir.join(base_name);
    if !candidate.exists() {
        return candidate;
    }
    let mut i = 2;
    loop {
        let name = format!("{} {}", base_name, i);
        let candidate = dir.join(&name);
        if !candidate.exists() {
            return candidate;
        }
        i += 1;
    }
}

fn build_new_folder_view(
    popover: &Popover,
    stack: &Stack,
    desktop_dir: PathBuf,
    click_pos_cell: Rc<Cell<(f64, f64)>>,
    icon_positions: Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    trigger_refresh: Rc<dyn Fn()>,
) -> (GtkBox, Entry) {
    let container = GtkBox::new(Orientation::Vertical, 8);
    container.set_margin_top(10);
    container.set_margin_bottom(10);
    container.set_margin_start(10);
    container.set_margin_end(10);
    container.add_css_class("desktop-context-menu");

    let title_lbl = Label::builder()
        .label("Nueva Carpeta")
        .halign(Align::Start)
        .css_classes(["card-section-title"])
        .build();
    container.append(&title_lbl);

    let default_path = get_unique_folder_name(&desktop_dir, "Nueva carpeta");
    let default_str = default_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Nueva carpeta");

    let entry = Entry::builder()
        .text(default_str)
        .hexpand(true)
        .build();
    container.append(&entry);

    let btn_box = GtkBox::new(Orientation::Horizontal, 6);
    btn_box.set_halign(Align::End);

    let cancel_btn = Button::with_label("Cancelar");
    cancel_btn.add_css_class("desktop-context-btn");
    let stack_back = stack.clone();
    cancel_btn.connect_clicked(move |_| {
        stack_back.set_visible_child_name("main");
    });
    btn_box.append(&cancel_btn);

    let save_btn = Button::with_label("Crear");
    save_btn.add_css_class("desktop-context-btn");
    save_btn.add_css_class("suggested-action");

    let popover_save = popover.clone();
    let entry_save = entry.clone();
    let desktop_dir_save = desktop_dir.clone();
    let icon_positions_save = icon_positions.clone();
    let trigger_save = trigger_refresh.clone();
    let click_pos_save = click_pos_cell.clone();

    let create_action = move || {
        let new_name = entry_save.text().trim().to_string();
        if !new_name.is_empty() {
            let dest = desktop_dir_save.join(&new_name);
            if let Err(err) = fs::create_dir_all(&dest) {
                warn!("No se pudo crear carpeta {:?}: {}", dest, err);
            } else {
                icon_positions_save.borrow_mut().insert(dest, click_pos_save.get());
                save_icon_positions(&icon_positions_save.borrow());
                trigger_save();
            }
        }
        popover_save.popdown();
    };

    let create_action_rc = Rc::new(create_action);
    let create_for_btn = create_action_rc.clone();
    save_btn.connect_clicked(move |_| {
        create_for_btn();
    });
    btn_box.append(&save_btn);
    container.append(&btn_box);

    let create_for_entry = create_action_rc.clone();
    entry.connect_activate(move |_| {
        create_for_entry();
    });

    (container, entry)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopOrganizeBy {
    Name,
    Kind,
    DateModified,
}

pub fn calculate_organize_targets(
    desktop_dir: &Path,
    desktop_size: (i32, i32),
    config: &DesktopIconsConfig,
    by: DesktopOrganizeBy,
    top_margin: f64,
) -> HashMap<PathBuf, (f64, f64)> {
    let mut raw_paths = Vec::new();
    if let Ok(entries) = fs::read_dir(desktop_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if let Some(file_name) = p.file_name() {
                if file_name.to_string_lossy().starts_with('.') {
                    continue;
                }
            }
            raw_paths.push(p);
        }
    }

    match by {
        DesktopOrganizeBy::Name => {
            raw_paths.sort_by(|a, b| {
                let a_is_dir = a.is_dir();
                let b_is_dir = b.is_dir();
                // Carpetas primero (is_dir true antes de false)
                if a_is_dir != b_is_dir {
                    return b_is_dir.cmp(&a_is_dir);
                }
                let a_name = a.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
                let b_name = b.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
                a_name.cmp(&b_name)
            });
        }
        DesktopOrganizeBy::Kind => {
            raw_paths.sort_by(|a, b| {
                let a_is_dir = a.is_dir();
                let b_is_dir = b.is_dir();
                if a_is_dir != b_is_dir {
                    return b_is_dir.cmp(&a_is_dir);
                }
                let a_ext = a.extension().unwrap_or_default().to_string_lossy().to_lowercase();
                let b_ext = b.extension().unwrap_or_default().to_string_lossy().to_lowercase();
                let ext_cmp = a_ext.cmp(&b_ext);
                if ext_cmp != std::cmp::Ordering::Equal {
                    return ext_cmp;
                }
                let a_name = a.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
                let b_name = b.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
                a_name.cmp(&b_name)
            });
        }
        DesktopOrganizeBy::DateModified => {
            raw_paths.sort_by(|a, b| {
                let a_time = fs::metadata(a).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                let b_time = fs::metadata(b).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                // Más recientes primero
                b_time.cmp(&a_time)
            });
        }
    }

    let metrics = IconLayoutMetrics::from_config(config);
    let rows = ((desktop_size.1 - (top_margin as i32) - 80).max(1) / metrics.cell_height()).max(1) as usize;

    let mut targets = HashMap::new();
    for (index, path) in raw_paths.iter().enumerate() {
        let pos = calculate_grid_position(index, rows, &metrics, config.align_left, desktop_size, top_margin);
        targets.insert(path.clone(), pos);
    }
    targets
}

pub fn organize_desktop_icons(
    desktop_dir: &Path,
    desktop_size: (i32, i32),
    config: &DesktopIconsConfig,
    icon_positions: &Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    by: DesktopOrganizeBy,
    top_margin: f64,
) {
    let targets = calculate_organize_targets(desktop_dir, desktop_size, config, by, top_margin);
    let mut positions = icon_positions.borrow_mut();
    positions.clear();
    positions.extend(targets);
    save_icon_positions(&positions);
}

pub fn animate_desktop_layout_transition(
    fixed: &Fixed,
    targets: &HashMap<PathBuf, (f64, f64)>,
    duration_ms: i64,
) {
    let mut moves: Vec<(gtk4::Widget, (f64, f64), (f64, f64))> = Vec::new();
    let mut child = fixed.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if item.has_css_class("desktop-rubberband") {
            continue;
        }
        let current = fixed.child_position(&item);
        if current.0 < 0.0 || current.1 < 0.0 {
            continue;
        }
        if let Some(path_str) = item.property::<Option<String>>("tooltip-text") {
            let path = PathBuf::from(path_str);
            if let Some(&target) = targets.get(&path) {
                let dx = target.0 - current.0;
                let dy = target.1 - current.1;
                if dx.abs() > 0.5 || dy.abs() > 0.5 {
                    moves.push((item, current, target));
                }
            }
        }
    }

    if moves.is_empty() {
        return;
    }

    if fixed.root().is_none() {
        for (widget, _, target) in &moves {
            fixed.move_(widget, target.0, target.1);
        }
        return;
    }

    let anim_gen = DESKTOP_LAYOUT_ANIM_GEN.with(|g| {
        let next = g.get().wrapping_add(1);
        g.set(next);
        next
    });

    let start_time: Rc<Cell<Option<i64>>> = Rc::new(Cell::new(None));
    let fixed_clone = fixed.clone();

    fixed.add_tick_callback(move |_, frame_clock| {
        if DESKTOP_LAYOUT_ANIM_GEN.with(|g| g.get()) != anim_gen {
            return glib::ControlFlow::Break;
        }

        let now = frame_clock.frame_time() / 1000;
        let start = match start_time.get() {
            Some(t) => t,
            None => {
                start_time.set(Some(now));
                now
            }
        };

        let elapsed = now - start;
        if elapsed >= duration_ms {
            for (widget, _, target) in &moves {
                fixed_clone.move_(widget, target.0, target.1);
            }
            glib::ControlFlow::Break
        } else {
            let progress = (elapsed as f64 / duration_ms as f64).clamp(0.0, 1.0);
            let eased = ease_out_cubic(progress);
            for (widget, start_pos, target) in &moves {
                let cx = start_pos.0 + (target.0 - start_pos.0) * eased;
                let cy = start_pos.1 + (target.1 - start_pos.1) * eased;
                fixed_clone.move_(widget, cx, cy);
            }
            glib::ControlFlow::Continue
        }
    });
}

pub fn animate_desktop_organization(
    fixed: &Fixed,
    desktop_dir: &Path,
    desktop_size: (i32, i32),
    config: &DesktopIconsConfig,
    icon_positions: &Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    by: DesktopOrganizeBy,
    duration_ms: i64,
    top_margin: f64,
) {
    let targets = calculate_organize_targets(desktop_dir, desktop_size, config, by, top_margin);
    {
        let mut positions = icon_positions.borrow_mut();
        positions.clear();
        positions.extend(targets.clone());
        save_icon_positions(&positions);
    }
    animate_desktop_layout_transition(fixed, &targets, duration_ms);
}

pub fn animate_desktop_snap_to_grid(
    fixed: &Fixed,
    desktop_size: (i32, i32),
    config: &DesktopIconsConfig,
    icon_positions: &Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    duration_ms: i64,
    top_margin: f64,
) {
    let metrics = IconLayoutMetrics::from_config(config);
    let align_left = config.align_left;
    let mut targets = HashMap::new();
    {
        let pos = icon_positions.borrow();
        for (p, pt) in pos.iter() {
            let snapped = snap_position_to_grid(*pt, &metrics, align_left, desktop_size, top_margin);
            targets.insert(p.clone(), snapped);
        }
    }
    {
        let mut positions = icon_positions.borrow_mut();
        for (p, pt) in &targets {
            positions.insert(p.clone(), *pt);
        }
        save_icon_positions(&positions);
    }
    animate_desktop_layout_transition(fixed, &targets, duration_ms);
}

fn build_organize_submenu(
    popover: &Popover,
    stack: &Stack,
    fixed: &Fixed,
    desktop_dir: PathBuf,
    desktop_size: (i32, i32),
    config: Rc<RefCell<DesktopIconsConfig>>,
    icon_positions: Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    trigger_refresh: Rc<dyn Fn()>,
    top_margin: f64,
) -> GtkBox {
    let container = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .css_classes(["desktop-context-menu"])
        .build();

    let back_btn = create_menu_button("← Volver", false);
    let stack_back = stack.clone();
    back_btn.connect_clicked(move |_| {
        stack_back.set_visible_child_name("main");
    });
    container.append(&back_btn);

    let sep = GtkBox::new(Orientation::Horizontal, 0);
    sep.add_css_class("desktop-context-separator");
    container.append(&sep);

    let by_name_btn = create_menu_button("Por Nombre (Carpetas primero)", false);
    let p_c = popover.clone();
    let dir_c = desktop_dir.clone();
    let cfg_c = config.clone();
    let pos_c = icon_positions.clone();
    let trig_c = trigger_refresh.clone();
    let fixed_c = fixed.clone();
    by_name_btn.connect_clicked(move |_| {
        let cfg = cfg_c.borrow().clone();
        if cfg.use_stacks {
            organize_desktop_icons(&dir_c, desktop_size, &cfg, &pos_c, DesktopOrganizeBy::Name, top_margin);
            trig_c();
        } else {
            animate_desktop_organization(
                &fixed_c,
                &dir_c,
                desktop_size,
                &cfg,
                &pos_c,
                DesktopOrganizeBy::Name,
                320,
                top_margin,
            );
        }
        p_c.popdown();
    });
    container.append(&by_name_btn);

    let by_kind_btn = create_menu_button("Por Tipo / Clase", false);
    let p_c = popover.clone();
    let dir_c = desktop_dir.clone();
    let cfg_c = config.clone();
    let pos_c = icon_positions.clone();
    let trig_c = trigger_refresh.clone();
    let fixed_c = fixed.clone();
    by_kind_btn.connect_clicked(move |_| {
        let cfg = cfg_c.borrow().clone();
        if cfg.use_stacks {
            organize_desktop_icons(&dir_c, desktop_size, &cfg, &pos_c, DesktopOrganizeBy::Kind, top_margin);
            trig_c();
        } else {
            animate_desktop_organization(
                &fixed_c,
                &dir_c,
                desktop_size,
                &cfg,
                &pos_c,
                DesktopOrganizeBy::Kind,
                320,
                top_margin,
            );
        }
        p_c.popdown();
    });
    container.append(&by_kind_btn);

    let by_date_btn = create_menu_button("Por Fecha de modificación", false);
    let p_c = popover.clone();
    let dir_c = desktop_dir;
    let cfg_c = config;
    let pos_c = icon_positions;
    let trig_c = trigger_refresh;
    let fixed_c = fixed.clone();
    by_date_btn.connect_clicked(move |_| {
        let cfg = cfg_c.borrow().clone();
        if cfg.use_stacks {
            organize_desktop_icons(&dir_c, desktop_size, &cfg, &pos_c, DesktopOrganizeBy::DateModified, top_margin);
            trig_c();
        } else {
            animate_desktop_organization(
                &fixed_c,
                &dir_c,
                desktop_size,
                &cfg,
                &pos_c,
                DesktopOrganizeBy::DateModified,
                320,
                top_margin,
            );
        }
        p_c.popdown();
    });
    container.append(&by_date_btn);

    container
}

struct DesktopItemData {
    path: PathBuf,
    display_name: String,
    icon: Option<gio::Icon>,
    content_type: Option<String>,
    category: StackCategory,
}

fn refresh_desktop_icons(
    fixed: &Fixed,
    desktop_dir: &Path,
    desktop_size: (i32, i32),
    config: &DesktopIconsConfig,
    icon_positions: &Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    internal_drag_paths: &Rc<RefCell<HashSet<PathBuf>>>,
    active_icon_drag: &Rc<RefCell<Option<(PathBuf, (f64, f64), (f64, f64))>>>,
    selected_paths: &Rc<RefCell<HashSet<PathBuf>>>,
    focused_path: &Rc<RefCell<Option<PathBuf>>>,
    expanded_stacks: &Rc<RefCell<HashSet<StackCategory>>>,
    recently_dropped_paths: &Rc<RefCell<HashSet<PathBuf>>>,
    top_margin: f64,
) {
    let mut child = fixed.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if current.has_css_class("desktop-rubberband") {
            continue;
        }
        fixed.remove(&current);
    }

    if !config.show_icons {
        return;
    }

    let entries = match fs::read_dir(desktop_dir) {
        Ok(entries) => entries,
        Err(error) => {
            warn!(
                "No se pudo leer la carpeta de escritorio {:?}: {}",
                desktop_dir, error
            );
            return;
        }
    };

    let mut raw_paths: Vec<PathBuf> = entries
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry.path()),
            Err(error) => {
                warn!("No se pudo leer una entrada del escritorio: {}", error);
                None
            }
        })
        .collect();
    raw_paths.sort_by_key(|path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    });

    let mut items = Vec::new();
    for path in raw_paths {
        let file = gio::File::for_path(&path);
        let file_info = file.query_info(
            "standard::display-name,standard::icon,standard::content-type",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        );
        let (display_name, icon, content_type) = match file_info {
            Ok(info) => {
                let display_name = info.display_name().to_string();
                let icon = info.icon();
                let content_type = info.content_type().map(|value| value.to_string());
                (display_name, icon, content_type)
            }
            Err(error) => {
                warn!("No se pudo obtener información de {:?}: {}", path, error);
                continue;
            }
        };

        let category = StackCategory::categorize(&path, content_type.as_deref());
        items.push(DesktopItemData {
            path,
            display_name,
            icon,
            content_type,
            category,
        });
    }

    let metrics = IconLayoutMetrics::from_config(config);
    let rows = ((desktop_size.1 - (top_margin as i32) - 80).max(1) / metrics.cell_height()).max(1) as usize;

    let mut positions = icon_positions.borrow_mut();
    let current_paths_set: HashSet<PathBuf> = items.iter().map(|it| it.path.clone()).collect();
    positions.retain(|path, _| current_paths_set.contains(path));

    if config.use_stacks {
        // Modo Pilas de macOS (macOS Desktop Stacks)
        let mut grouped: HashMap<StackCategory, Vec<DesktopItemData>> = HashMap::new();
        for item in items {
            grouped.entry(item.category).or_default().push(item);
        }

        let mut sorted_categories: Vec<StackCategory> = grouped.keys().copied().collect();
        sorted_categories.sort_by_key(|c| c.title());

        let mut slot_index = 0;
        for cat in sorted_categories {
            let cat_items = grouped.remove(&cat).unwrap_or_default();
            let is_expanded = expanded_stacks.borrow().contains(&cat);
            let count = cat_items.len();

            // Renderizar encabezado de la pila
            let stack_box = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(4)
                .halign(Align::Center)
                .width_request(metrics.box_width)
                .height_request(metrics.box_height)
                .focusable(true)
                .css_classes(["desktop-icon"])
                .tooltip_text(format!("{} ({} elementos)", cat.title(), count))
                .build();

            let stack_image = Image::from_icon_name(cat.icon_name());
            stack_image.set_pixel_size(metrics.icon_size);

            let stack_badge = Label::builder()
                .label(&format!("{}", count))
                .css_classes(["desktop-stack-badge"])
                .halign(Align::End)
                .valign(Align::Start)
                .build();

            let icon_overlay = Overlay::new();
            icon_overlay.set_child(Some(&stack_image));
            icon_overlay.add_overlay(&stack_badge);

            let label = Label::builder()
                .label(cat.title())
                .wrap(true)
                .max_width_chars(14)
                .justify(gtk4::Justification::Center)
                .css_classes(["desktop-icon-label"])
                .build();

            stack_box.append(&icon_overlay);
            stack_box.append(&label);

            let (x, y) = calculate_grid_position(slot_index, rows, &metrics, config.align_left, desktop_size, top_margin);
            fixed.put(&stack_box, x, y);
            slot_index += 1;

            let expanded_ref = expanded_stacks.clone();
            let click = GestureClick::new();
            click.set_button(1);
            let fixed_refresh = fixed.clone();
            let desktop_dir_refresh = desktop_dir.to_path_buf();
            let config_refresh = config.clone();
            let icon_pos_refresh = icon_positions.clone();
            let int_drag_refresh = internal_drag_paths.clone();
            let act_drag_refresh = active_icon_drag.clone();
            let sel_paths_refresh = selected_paths.clone();
            let foc_path_refresh = focused_path.clone();

            let rec_refresh = recently_dropped_paths.clone();

            click.connect_pressed(move |_, _, _, _| {
                let mut exp = expanded_ref.borrow_mut();
                if exp.contains(&cat) {
                    exp.remove(&cat);
                } else {
                    exp.insert(cat);
                }
                drop(exp);

                refresh_desktop_icons(
                    &fixed_refresh,
                    &desktop_dir_refresh,
                    desktop_size,
                    &config_refresh,
                    &icon_pos_refresh,
                    &int_drag_refresh,
                    &act_drag_refresh,
                    &sel_paths_refresh,
                    &foc_path_refresh,
                    &expanded_ref,
                    &rec_refresh,
                    top_margin,
                );
            });
            stack_box.add_controller(click);

            // Si la pila está expandida, renderizar los archivos hijos secuencialmente
            if is_expanded {
                for child_item in cat_items {
                    render_single_icon(
                        fixed,
                        slot_index,
                        rows,
                        child_item,
                        &metrics,
                        config,
                        desktop_size,
                        &mut positions,
                        icon_positions,
                        internal_drag_paths,
                        active_icon_drag,
                        selected_paths,
                        focused_path,
                        recently_dropped_paths,
                        top_margin,
                    );
                    slot_index += 1;
                }
            }
        }
    } else {
        // Modo Normal: visualización directa en cuadrícula o coordenadas libres persistentes
        for (index, item) in items.into_iter().enumerate() {
            render_single_icon(
                fixed,
                index,
                rows,
                item,
                &metrics,
                config,
                desktop_size,
                &mut positions,
                icon_positions,
                internal_drag_paths,
                active_icon_drag,
                selected_paths,
                focused_path,
                recently_dropped_paths,
                top_margin,
            );
        }
    }
}

fn create_menu_button(label: &str, destructive: bool) -> Button {
    let btn = Button::with_label(label);
    btn.add_css_class("desktop-context-btn");
    if destructive {
        btn.add_css_class("destructive");
    }
    btn.set_halign(Align::Fill);
    if let Some(child) = btn.child() {
        if let Some(lbl) = child.downcast_ref::<Label>() {
            lbl.set_xalign(0.0);
            let is_dark = is_dark_mode_active();
            lbl.add_css_class(if is_dark {
                "menu-dark-text"
            } else {
                "menu-light-text"
            });
        }
    }
    btn
}

fn resolve_desktop_icon_paintable(
    path: &Path,
    content_type: Option<&str>,
    icon: Option<&gio::Icon>,
    icon_size: i32,
) -> Option<gtk4::gdk::Paintable> {
    if let Some(ct) = content_type {
        if ct.starts_with("image/") {
            if let Ok(pixbuf) = gdk_pixbuf::Pixbuf::from_file_at_scale(path, icon_size, icon_size, true) {
                let texture = gtk4::gdk::Texture::for_pixbuf(&pixbuf);
                return Some(texture.upcast());
            }
        }
    }

    let display = gtk4::gdk::Display::default()?;
    let icon_theme = gtk4::IconTheme::for_display(&display);
    if let Some(gicon) = icon {
        let icon_paintable = icon_theme.lookup_by_gicon(
            gicon,
            icon_size,
            1,
            gtk4::TextDirection::Ltr,
            gtk4::IconLookupFlags::empty(),
        );
        Some(icon_paintable.upcast())
    } else {
        let icon_paintable = icon_theme.lookup_icon(
            "text-x-generic",
            &[],
            icon_size,
            1,
            gtk4::TextDirection::Ltr,
            gtk4::IconLookupFlags::empty(),
        );
        Some(icon_paintable.upcast())
    }
}

pub fn truncate_desktop_name(name: &str, max_chars: usize) -> String {
    let chars: Vec<char> = name.chars().collect();
    let n = chars.len();
    if n <= max_chars || max_chars < 6 {
        return name.to_string();
    }

    // Identificar si tiene extensión de archivo (ej. .png, .pdf, .tar.gz)
    let dot_pos = chars.iter().rposition(|&c| c == '.');
    let has_extension = match dot_pos {
        Some(pos) => pos > 0 && (n - pos) <= 8 && (n - pos) < max_chars / 2,
        None => false,
    };

    let avail = max_chars.saturating_sub(1); // Descontar 1 carácter para el elipsis '…'

    if let (true, Some(last_dot)) = (has_extension, dot_pos) {
        // Detectar si es una extensión compuesta como .tar.gz o .tar.xz
        let effective_dot = if last_dot >= 4 {
            let prev_slice: String = chars[last_dot.saturating_sub(4)..last_dot].iter().collect();
            if prev_slice == ".tar" {
                last_dot - 4
            } else {
                last_dot
            }
        } else {
            last_dot
        };

        let ext_len = n - effective_dot;
        let extra_tail = ((avail.saturating_sub(ext_len)) / 3).clamp(1, 4);
        let tail_len = (ext_len + extra_tail).min(avail.saturating_sub(3));
        let head_len = avail - tail_len;

        let head: String = chars[..head_len].iter().collect();
        let tail: String = chars[n - tail_len..].iter().collect();
        format!("{}…{}", head.trim_end(), tail.trim_start())
    } else {
        let head_len = (avail + 1) / 2;
        let tail_len = avail - head_len;
        let head: String = chars[..head_len].iter().collect();
        let tail: String = chars[n - tail_len..].iter().collect();
        format!("{}…{}", head.trim_end(), tail.trim_start())
    }
}

pub fn get_icon_label<W: IsA<gtk4::Widget>>(item_box: &W) -> Option<gtk4::Label> {
    if let Some(box_widget) = item_box.as_ref().downcast_ref::<gtk4::Box>() {
        let mut child = box_widget.first_child();
        while let Some(c) = child {
            if let Some(label) = c.downcast_ref::<gtk4::Label>() {
                if label.has_css_class("desktop-icon-label") {
                    return Some(label.clone());
                }
            }
            child = c.next_sibling();
        }
    }
    None
}

pub fn set_desktop_icon_selected<W: IsA<gtk4::Widget>>(item_box: &W, selected: bool) {
    let widget = item_box.as_ref();
    if widget.has_css_class("desktop-rubberband") {
        return;
    }

    if selected {
        widget.add_css_class("selected");
    } else {
        widget.remove_css_class("selected");
    }

    if let Some(label) = get_icon_label(item_box) {
        let full_name = label.widget_name();
        let full_name_str = full_name.as_str();
        if full_name_str.is_empty() || full_name_str == "GtkLabel" {
            return;
        }

        let max_chars = if widget.width_request() > 0 {
            (widget.width_request() * 2 / 9).clamp(20, 36) as usize
        } else {
            24
        };

        if selected {
            label.set_lines(0);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::None);
            label.set_text(full_name_str);
        } else {
            let truncated = truncate_desktop_name(full_name_str, max_chars);
            label.set_lines(2);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            label.set_text(&truncated);
        }
    }
}

#[allow(unused_variables)]
fn render_single_icon(
    fixed: &Fixed,
    index: usize,
    rows: usize,
    item: DesktopItemData,
    metrics: &IconLayoutMetrics,
    config: &DesktopIconsConfig,
    desktop_size: (i32, i32),
    positions: &mut HashMap<PathBuf, (f64, f64)>,
    icon_positions: &Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    internal_drag_paths: &Rc<RefCell<HashSet<PathBuf>>>,
    active_icon_drag: &Rc<RefCell<Option<(PathBuf, (f64, f64), (f64, f64))>>>,
    selected_paths: &Rc<RefCell<HashSet<PathBuf>>>,
    focused_path: &Rc<RefCell<Option<PathBuf>>>,
    recently_dropped_paths: &Rc<RefCell<HashSet<PathBuf>>>,
    top_margin: f64,
) {
    let path = item.path;
    let file = gio::File::for_path(&path);
    let display_name = item.display_name;
    let icon = item.icon;
    let content_type = item.content_type;

    let icon_paintable = resolve_desktop_icon_paintable(
        &path,
        content_type.as_deref(),
        icon.as_ref(),
        metrics.icon_size,
    );

    let thumbnail = content_type
        .as_deref()
        .filter(|ct| ct.starts_with("image/"))
        .and_then(|_| {
            gdk_pixbuf::Pixbuf::from_file_at_scale(
                &path,
                metrics.icon_size,
                metrics.icon_size,
                true,
            )
            .ok()
        });

    let icon_image = thumbnail
        .as_ref()
        .map(|pixbuf| {
            let texture = gtk4::gdk::Texture::for_pixbuf(pixbuf);
            Image::from_paintable(Some(&texture))
        })
        .unwrap_or_else(|| match icon.as_ref() {
            Some(i) => Image::from_gicon(i),
            None => Image::from_icon_name("text-x-generic"),
        });
    icon_image.set_pixel_size(metrics.icon_size);

    let label = Label::builder()
        .wrap(true)
        .wrap_mode(gtk4::pango::WrapMode::WordChar)
        .max_width_chars(14)
        .justify(gtk4::Justification::Center)
        .css_classes(["desktop-icon-label"])
        .build();
    label.set_widget_name(&display_name);

    let item_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .halign(Align::Center)
        .width_request(metrics.box_width)
        .height_request(metrics.box_height)
        .focusable(true)
        .css_classes(["desktop-icon"])
        .tooltip_text(path.to_string_lossy())
        .build();

    item_box.append(&icon_image);
    item_box.append(&label);

    let is_selected = selected_paths.borrow().contains(&path);
    set_desktop_icon_selected(&item_box, is_selected);

    let (x, mut y) = *positions.entry(path.clone()).or_insert_with(|| {
        calculate_grid_position(index, rows, metrics, config.align_left, desktop_size, top_margin)
    });
    if y < top_margin {
        y = top_margin;
        positions.insert(path.clone(), (x, y));
    }
    fixed.put(&item_box, x, y);

    // Animación de aterrizaje suave estilo Finder si el archivo acaba de ser soltado
    if recently_dropped_paths.borrow().contains(&path) {
        item_box.add_css_class("landing");
        let box_landing = item_box.clone();
        let path_landing = path.clone();
        let rec_landing = recently_dropped_paths.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
            box_landing.remove_css_class("landing");
            rec_landing.borrow_mut().remove(&path_landing);
        });
    }

    // Clic con botón 1 (Selección simple / doble clic para abrir)
    let click = GestureClick::new();
    click.set_button(1);
    let open_file = file.clone();
    let path_click = path.clone();
    let sel_paths_click = selected_paths.clone();
    let foc_path_click = focused_path.clone();
    let drag_fixed_press = fixed.clone();
    let drag_item_press = item_box.clone();

    click.connect_pressed(move |gesture, presses, _, _| {
        if presses == 2 {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            launch_file(&open_file);
            return;
        }

        let is_ctrl = gesture
            .current_event_state()
            .contains(gtk4::gdk::ModifierType::CONTROL_MASK);

        let mut sel = sel_paths_click.borrow_mut();
        if is_ctrl {
            if sel.contains(&path_click) {
                sel.remove(&path_click);
                set_desktop_icon_selected(&drag_item_press, false);
            } else {
                sel.insert(path_click.clone());
                set_desktop_icon_selected(&drag_item_press, true);
            }
        } else if !sel.contains(&path_click) {
            // Deseleccionar hermanos si este elemento no estaba seleccionado
            let mut sibling = drag_fixed_press.first_child();
            while let Some(child) = sibling {
                sibling = child.next_sibling();
                set_desktop_icon_selected(&child, false);
            }
            sel.clear();
            sel.insert(path_click.clone());
            set_desktop_icon_selected(&drag_item_press, true);
        }
        *foc_path_click.borrow_mut() = Some(path_click.clone());
    });
    item_box.add_controller(click);

    // Menú contextual de archivo estilo macOS (Popover dedicado)
    let popover = Popover::builder()
        .has_arrow(false)
        .autohide(true)
        .position(gtk4::PositionType::Bottom)
        .css_classes(["desktop-popover-menu"])
        .build();

    let menu = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .css_classes(["desktop-context-menu"])
        .build();

    // 1. Abrir
    let open_button = create_menu_button("Abrir", false);
    let open_file_btn = file.clone();
    let popover_c = popover.clone();
    open_button.connect_clicked(move |_| {
        launch_file(&open_file_btn);
        popover_c.popdown();
    });
    menu.append(&open_button);

    // 2. Vista rápida (Quick Look)
    let ql_button = create_menu_button("Vista rápida (Espacio)", false);
    let ql_path = path.clone();
    let popover_c = popover.clone();
    ql_button.connect_clicked(move |_| {
        launch_quick_look(&ql_path);
        popover_c.popdown();
    });
    menu.append(&ql_button);

    // 3. Mostrar en gestor de archivos
    let reveal_button = create_menu_button("Mostrar en el gestor de archivos", false);
    let reveal_file = file.clone();
    let popover_c = popover.clone();
    reveal_button.connect_clicked(move |_| {
        let parent = reveal_file.parent().unwrap_or_else(|| reveal_file.clone());
        launch_file(&parent);
        popover_c.popdown();
    });
    menu.append(&reveal_button);

    let sep1 = GtkBox::new(Orientation::Horizontal, 0);
    sep1.add_css_class("desktop-context-separator");
    menu.append(&sep1);

    // 4. Renombrar
    let rename_button = create_menu_button("Renombrar...", false);
    let rename_path = path.clone();
    let rename_item = item_box.clone();
    let popover_c = popover.clone();
    rename_button.connect_clicked(move |_| {
        popover_c.popdown();
        show_rename_dialog(&rename_item, rename_path.clone());
    });
    menu.append(&rename_button);

    // 5. Duplicar
    let dup_button = create_menu_button("Duplicar", false);
    let dup_path = path.clone();
    let popover_c = popover.clone();
    dup_button.connect_clicked(move |_| {
        duplicate_file(&dup_path);
        popover_c.popdown();
    });
    menu.append(&dup_button);

    let sep2 = GtkBox::new(Orientation::Horizontal, 0);
    sep2.add_css_class("desktop-context-separator");
    menu.append(&sep2);

    // 6. Mover a la papelera
    let trash_button = create_menu_button("Mover a la papelera", true);
    let trash_path = path.clone();
    let popover_c = popover.clone();
    trash_button.connect_clicked(move |_| {
        trash_file(&trash_path);
        popover_c.popdown();
    });
    menu.append(&trash_button);

    popover.set_child(Some(&menu));
    popover.set_parent(&item_box);
    register_wallpaper_popover(&popover);
    let p_unroot = popover.clone();
    item_box.connect_unrealize(move |_| {
        p_unroot.unparent();
    });

    let context_click = GestureClick::new();
    context_click.set_button(3);
    let p_click = popover.clone();
    let menu_click = menu.clone();
    context_click.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk4::EventSequenceState::Claimed);
        let is_dark = is_dark_mode_active();
        apply_theme_to_popover(&p_click, is_dark);
        let rect = gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        p_click.set_pointing_to(Some(&rect));
        animate_widget_fade_in(&menu_click, 140);
        p_click.popup();
    });
    item_box.add_controller(context_click);

    // Arrastrar archivo/carpeta (Drag & Drop Wayland con icono translúcido estilo Finder)
    let drag_source = gtk4::DragSource::new();
    drag_source.set_actions(gtk4::gdk::DragAction::COPY | gtk4::gdk::DragAction::MOVE);
    let path_drag = path.clone();
    let sel_drag = selected_paths.clone();
    let item_box_prep = item_box.clone();
    let stored_click = Rc::new(Cell::new((
        f64::from(metrics.box_width) / 2.0,
        f64::from(metrics.box_height) / 2.0,
    )));

    // Pre-adjuntar un paintable inicial para respuesta visual inmediata al iniciar el arrastre
    let initial_paintable = gtk4::WidgetPaintable::new(Some(&item_box));
    drag_source.set_icon(
        Some(&initial_paintable),
        metrics.box_width / 2,
        metrics.box_height / 2,
    );

    let stored_click_prep = stored_click.clone();
    drag_source.connect_prepare(move |source, click_x, click_y| {
        stored_click_prep.set((click_x, click_y));
        set_desktop_icon_selected(&item_box_prep, true);
        let paintable = gtk4::WidgetPaintable::new(Some(&item_box_prep));
        source.set_icon(Some(&paintable), click_x as i32, click_y as i32);

        let sel = sel_drag.borrow();
        let files: Vec<gio::File> = if sel.contains(&path_drag) && sel.len() > 1 {
            sel.iter().map(|p| gio::File::for_path(p)).collect()
        } else {
            vec![gio::File::for_path(&path_drag)]
        };
        let file_list = gtk4::gdk::FileList::from_array(&files);
        Some(gtk4::gdk::ContentProvider::for_value(&file_list.to_value()))
    });

    let item_box_drag = item_box.clone();
    let fixed_drag = fixed.clone();
    let path_drag_begin = path.clone();
    let sel_drag_begin = selected_paths.clone();
    let act_drag_begin = active_icon_drag.clone();
    let stored_click_begin = stored_click.clone();
    drag_source.connect_drag_begin(move |_source, _drag| {
        let (click_x, click_y) = stored_click_begin.get();
        let sel = sel_drag_begin.borrow();
        if sel.contains(&path_drag_begin) && sel.len() > 1 {
            let mut sibling = fixed_drag.first_child();
            while let Some(child) = sibling {
                sibling = child.next_sibling();
                if let Some(p_str) = child.property::<Option<String>>("tooltip-text") {
                    if sel.contains(&PathBuf::from(p_str)) {
                        fixed_drag.move_(&child, -5000.0, -5000.0);
                    }
                }
            }
        } else {
            fixed_drag.move_(&item_box_drag, -5000.0, -5000.0);
        }
        *act_drag_begin.borrow_mut() = Some((
            path_drag_begin.clone(),
            (0.0, 0.0),
            (click_x, click_y),
        ));
    });

    let fixed_drag_end = fixed.clone();
    let act_drag_end = active_icon_drag.clone();
    let icon_pos_drag_end = icon_positions.clone();
    let orig_pos = (x, y);
    drag_source.connect_drag_end(move |_, _drag, _| {
        *act_drag_end.borrow_mut() = None;
        let mut sibling = fixed_drag_end.first_child();
        while let Some(child) = sibling {
            sibling = child.next_sibling();
            let (cx, cy) = fixed_drag_end.child_position(&child);
            if cx < 0.0 || cy < 0.0 {
                if let Some(p_str) = child.property::<Option<String>>("tooltip-text") {
                    let p = PathBuf::from(p_str);
                    let target_pos = icon_pos_drag_end
                        .borrow()
                        .get(&p)
                        .cloned()
                        .unwrap_or(orig_pos);
                    fixed_drag_end.move_(&child, target_pos.0, target_pos.1);
                }
            }
        }
    });

    item_box.add_controller(drag_source);

    // Si es carpeta, permitir soltar archivos encima con resaltado azul acrílico estilo Finder
    if path.is_dir() {
        let folder_drop = gtk4::DropTarget::new(
            gtk4::gdk::FileList::static_type(),
            gtk4::gdk::DragAction::COPY | gtk4::gdk::DragAction::MOVE,
        );
        let item_box_hover = item_box.clone();
        folder_drop.connect_enter(move |_, _, _| {
            item_box_hover.add_css_class("folder-drop-hover");
            gtk4::gdk::DragAction::MOVE
        });

        let item_box_leave = item_box.clone();
        folder_drop.connect_leave(move |_| {
            item_box_leave.remove_css_class("folder-drop-hover");
        });

        let item_box_drop = item_box.clone();
        let target_dir = path.clone();
        let pos_map_drop = icon_positions.clone();
        folder_drop.connect_drop(move |_, value, _, _| {
            item_box_drop.remove_css_class("folder-drop-hover");
            if let Ok(file_list) = value.get::<gtk4::gdk::FileList>() {
                let mut moved_any = false;
                for src_file in file_list.files() {
                    if let Some(src_path) = src_file.path() {
                        if let Some(fname) = src_path.file_name() {
                            let dest_path = target_dir.join(fname);
                            if dest_path != src_path {
                                let dest_file = gio::File::for_path(&dest_path);
                                if src_file.move_(
                                    &dest_file,
                                    gio::FileCopyFlags::NONE,
                                    gio::Cancellable::NONE,
                                    None,
                                ).is_ok() {
                                    pos_map_drop.borrow_mut().remove(&src_path);
                                    moved_any = true;
                                }
                            }
                        }
                    }
                }
                if moved_any {
                    save_icon_positions(&pos_map_drop.borrow());
                    return true;
                }
            }
            false
        });
        item_box.add_controller(folder_drop);
    }
}

#[derive(Clone, Debug)]
enum DesktopDragMode {
    None,
    Rubberband {
        start: (f64, f64),
    },
    #[allow(dead_code)]
    Icon {
        clicked_path: PathBuf,
        start_positions: HashMap<PathBuf, (f64, f64)>,
        had_multiple_selected: bool,
    },
}

fn create_wallpaper_window(
    application: &Application,
    monitor: Option<&gtk4::gdk::Monitor>,
    resolved_path: &Option<PathBuf>,
    desktop_dir: Option<&Path>,
    desktop_monitors: &Rc<RefCell<Vec<gio::FileMonitor>>>,
    icon_positions: &Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    internal_drag_paths: &Rc<RefCell<HashSet<PathBuf>>>,
    active_icon_drag: &Rc<RefCell<Option<(PathBuf, (f64, f64), (f64, f64))>>>,
    is_primary: bool,
) {
    let window = ApplicationWindow::builder()
        .application(application)
        .title("Fondo de Pantalla Malus DE")
        .decorated(false)
        .css_classes(["malus-wallpaper-window"])
        .build();

    if gtk4_layer_shell::is_supported() {
        info!("Anclando Fondo de Pantalla en capa Background a pantalla completa...");
        window.init_layer_shell();
        if let Some(mon) = monitor {
            window.set_monitor(mon);
        }
        window.set_layer(Layer::Background);
        window.set_namespace("malus-wallpaper");
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Right, true);
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::OnDemand);
    } else {
        window.maximize();
    }

    let overlay = Overlay::new();
    if let Some(ref img_path) = resolved_path {
        if img_path.exists() {
            info!("Cargando imagen de fondo: {:?}", img_path);
            let picture = Picture::for_filename(img_path);
            picture.set_content_fit(gtk4::ContentFit::Cover);
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            picture.set_halign(Align::Fill);
            picture.set_valign(Align::Fill);
            overlay.set_child(Some(&picture));
        }
    }

    if let Some(desktop_dir) = desktop_dir {
        let desktop_size = monitor
            .map(|monitor| {
                let geometry = monitor.geometry();
                (geometry.width(), geometry.height())
            })
            .unwrap_or((1920, 1080));

        let top_margin = if is_primary {
            DESKTOP_TOP_MARGIN
        } else {
            DESKTOP_SECONDARY_TOP_MARGIN
        };

        let icons = Fixed::builder()
            .hexpand(true)
            .vexpand(true)
            .halign(Align::Fill)
            .valign(Align::Fill)
            .build();

        let selected_paths: Rc<RefCell<HashSet<PathBuf>>> = Rc::new(RefCell::new(HashSet::new()));
        let focused_path: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let expanded_stacks: Rc<RefCell<HashSet<StackCategory>>> = Rc::new(RefCell::new(HashSet::new()));
        let recently_dropped_paths: Rc<RefCell<HashSet<PathBuf>>> = Rc::new(RefCell::new(HashSet::new()));

        let current_config = Rc::new(RefCell::new(DesktopIconsConfig::load()));

        let icons_c = icons.clone();
        let dir_c = desktop_dir.to_path_buf();
        let icon_pos_c = icon_positions.clone();
        let int_drag_c = internal_drag_paths.clone();
        let act_drag_c = active_icon_drag.clone();
        let sel_paths_c = selected_paths.clone();
        let foc_path_c = focused_path.clone();
        let exp_stacks_c = expanded_stacks.clone();
        let rec_c = recently_dropped_paths.clone();
        let cfg_c = current_config.clone();

        let trigger_refresh: Rc<dyn Fn()> = Rc::new(move || {
            let cfg = cfg_c.borrow().clone();
            refresh_desktop_icons(
                &icons_c,
                &dir_c,
                desktop_size,
                &cfg,
                &icon_pos_c,
                &int_drag_c,
                &act_drag_c,
                &sel_paths_c,
                &foc_path_c,
                &exp_stacks_c,
                &rec_c,
                top_margin,
            );
        });

        trigger_refresh();

        // Controladores de teclado en la ventana del escritorio:
        // Espacio -> Quick Look, Enter -> Abrir, Delete -> Papelera
        let key_controller = EventControllerKey::new();
        let foc_path_key = focused_path.clone();
        let sel_paths_key = selected_paths.clone();
        key_controller.connect_key_pressed(move |_, keyval, _, _| {
            match keyval {
                Key::space => {
                    if let Some(path) = foc_path_key.borrow().as_ref() {
                        launch_quick_look(path);
                        return glib::Propagation::Stop;
                    }
                }
                Key::Return | Key::KP_Enter => {
                    if let Some(path) = foc_path_key.borrow().as_ref() {
                        let file = gio::File::for_path(path);
                        launch_file(&file);
                        return glib::Propagation::Stop;
                    }
                }
                Key::Delete | Key::BackSpace => {
                    let sel = sel_paths_key.borrow().clone();
                    for path in sel {
                        trash_file(&path);
                    }
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
            glib::Propagation::Proceed
        });
        window.add_controller(key_controller);

        // Selección elástica rectangular (Marquee / Rubberband)
        let rubberband = GtkBox::new(Orientation::Horizontal, 0);
        rubberband.add_css_class("desktop-rubberband");
        rubberband.set_visible(false);
        icons.put(&rubberband, 0.0, 0.0);

        // Clic en fondo de pantalla vacío:
        // Botón 1: Deseleccionar todo si no se hace clic sobre un icono
        let bg_click = GestureClick::new();
        bg_click.set_button(1);
        let icons_bg_click = icons.clone();
        let sel_paths_bg = selected_paths.clone();
        let foc_path_bg = focused_path.clone();
        let rb_check_click = rubberband.clone();
        bg_click.connect_pressed(move |_, _, x, y| {
            if is_point_over_icon(&icons_bg_click, x, y, &rb_check_click) {
                return;
            }
            let mut sibling = icons_bg_click.first_child();
            while let Some(child) = sibling {
                sibling = child.next_sibling();
                set_desktop_icon_selected(&child, false);
            }
            sel_paths_bg.borrow_mut().clear();
            *foc_path_bg.borrow_mut() = None;
        });
        overlay.add_controller(bg_click);

        // Botón 3 en fondo de pantalla vacío: Menú contextual del escritorio
        let bg_context = GestureClick::new();
        bg_context.set_button(3);
        let overlay_context = overlay.clone();
        let icons_context = icons.clone();
        let desktop_dir_menu = desktop_dir.to_path_buf();
        let cfg_menu = current_config.clone();
        let trigger_refresh_menu = trigger_refresh.clone();
        let icon_pos_snap = icon_positions.clone();
        let rb_check_ctx = rubberband.clone();

        bg_context.connect_pressed(move |gesture, _, x, y| {
            if is_point_over_icon(&icons_context, x, y, &rb_check_ctx) {
                return;
            }
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            let popover = Popover::builder()
                .has_arrow(false)
                .autohide(true)
                .css_classes(["desktop-popover-menu"])
                .build();
            popover.set_parent(&overlay_context);
            register_wallpaper_popover(&popover);

            let pointing_rect = gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
            popover.set_pointing_to(Some(&pointing_rect));

            let stack = Stack::builder()
                .transition_type(StackTransitionType::SlideLeftRight)
                .transition_duration(180)
                .interpolate_size(true)
                .css_classes(["desktop-context-menu"])
                .build();

            let click_pos_cell = Rc::new(Cell::new((x, y)));

            let (new_folder_box, new_folder_entry) = build_new_folder_view(
                &popover,
                &stack,
                desktop_dir_menu.clone(),
                click_pos_cell.clone(),
                icon_pos_snap.clone(),
                trigger_refresh_menu.clone(),
            );

            let organize_box = build_organize_submenu(
                &popover,
                &stack,
                &icons_context,
                desktop_dir_menu.clone(),
                desktop_size,
                cfg_menu.clone(),
                icon_pos_snap.clone(),
                trigger_refresh_menu.clone(),
                top_margin,
            );

            let menu = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(2)
                .css_classes(["desktop-context-menu"])
                .build();

            // 1. Nueva Carpeta
            let new_folder_btn = create_menu_button("Nueva carpeta", false);
            let stack_nf = stack.clone();
            let entry_nf = new_folder_entry.clone();
            let d_dir = desktop_dir_menu.clone();
            new_folder_btn.connect_clicked(move |_| {
                let default_path = get_unique_folder_name(&d_dir, "Nueva carpeta");
                let default_str = default_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Nueva carpeta");
                entry_nf.set_text(default_str);
                stack_nf.set_visible_child_name("new_folder");
                entry_nf.grab_focus();
                entry_nf.select_region(0, -1);
            });
            menu.append(&new_folder_btn);

            let sep1 = GtkBox::new(Orientation::Horizontal, 0);
            sep1.add_css_class("desktop-context-separator");
            menu.append(&sep1);

            // 2. Usar Pilas (macOS Stacks Toggle)
            let is_stacks = cfg_menu.borrow().use_stacks;
            let stacks_label = if is_stacks {
                "✓ Usar Pilas"
            } else {
                "Usar Pilas"
            };
            let stacks_btn = create_menu_button(stacks_label, false);
            let popover_c = popover.clone();
            let cfg_c2 = cfg_menu.clone();
            let trigger_c = trigger_refresh_menu.clone();
            stacks_btn.connect_clicked(move |_| {
                let mut c = cfg_c2.borrow_mut();
                c.use_stacks = !c.use_stacks;
                let _ = c.save();
                drop(c);
                trigger_c();
                popover_c.popdown();
            });
            menu.append(&stacks_btn);

            // 3. Organizar escritorio (Acción directa: limpia y anima el orden de carpetas y archivos en la cuadrícula)
            let organize_btn = create_menu_button("Organizar escritorio", false);
            let popover_c = popover.clone();
            let d_dir_org = desktop_dir_menu.clone();
            let cfg_org = cfg_menu.clone();
            let pos_org = icon_pos_snap.clone();
            let trigger_org = trigger_refresh_menu.clone();
            let icons_org = icons_context.clone();
            organize_btn.connect_clicked(move |_| {
                let cfg = cfg_org.borrow().clone();
                if cfg.use_stacks {
                    organize_desktop_icons(
                        &d_dir_org,
                        desktop_size,
                        &cfg,
                        &pos_org,
                        DesktopOrganizeBy::Name,
                        top_margin,
                    );
                    trigger_org();
                } else {
                    animate_desktop_organization(
                        &icons_org,
                        &d_dir_org,
                        desktop_size,
                        &cfg,
                        &pos_org,
                        DesktopOrganizeBy::Name,
                        320,
                        top_margin,
                    );
                }
                popover_c.popdown();
            });
            menu.append(&organize_btn);

            // 4. Organizar por ▸ (Submenú animado con criterios de orden)
            let organize_by_btn = create_menu_button("Organizar por ▸", false);
            let stack_org = stack.clone();
            organize_by_btn.connect_clicked(move |_| {
                stack_org.set_visible_child_name("organize");
            });
            menu.append(&organize_by_btn);

            // 5. Alinear a la cuadrícula (Snap animado individual de cada elemento a la celda más cercana)
            let snap_all_btn = create_menu_button("Alinear a la cuadrícula", false);
            let popover_c = popover.clone();
            let cfg_c3 = cfg_menu.clone();
            let trigger_c = trigger_refresh_menu.clone();
            let icon_pos_c2 = icon_pos_snap.clone();
            let icons_w = icons_context.clone();
            snap_all_btn.connect_clicked(move |_| {
                let cfg = cfg_c3.borrow().clone();
                if cfg.use_stacks {
                    trigger_c();
                } else {
                    animate_desktop_snap_to_grid(
                        &icons_w,
                        desktop_size,
                        &cfg,
                        &icon_pos_c2,
                        280,
                        top_margin,
                    );
                }
                popover_c.popdown();
            });
            menu.append(&snap_all_btn);

            // 6. Alineación automática (Toggle snap_to_grid + auto-organiza en cuadrícula de forma animada al activarse)
            let is_snap = cfg_menu.borrow().snap_to_grid;
            let snap_label = if is_snap {
                "✓ Alineación automática"
            } else {
                "Alineación automática"
            };
            let snap_btn = create_menu_button(snap_label, false);
            let popover_c = popover.clone();
            let cfg_c4 = cfg_menu.clone();
            let d_dir_snap = desktop_dir_menu.clone();
            let pos_snap_c = icon_pos_snap.clone();
            let trigger_c = trigger_refresh_menu.clone();
            let icons_snap = icons_context.clone();
            snap_btn.connect_clicked(move |_| {
                let mut c = cfg_c4.borrow_mut();
                let will_snap = !c.snap_to_grid;
                c.snap_to_grid = will_snap;
                let _ = c.save();
                let cfg_clone = c.clone();
                drop(c);
                if will_snap {
                    if cfg_clone.use_stacks {
                        organize_desktop_icons(
                            &d_dir_snap,
                            desktop_size,
                            &cfg_clone,
                            &pos_snap_c,
                            DesktopOrganizeBy::Name,
                            top_margin,
                        );
                        trigger_c();
                    } else {
                        animate_desktop_organization(
                            &icons_snap,
                            &d_dir_snap,
                            desktop_size,
                            &cfg_clone,
                            &pos_snap_c,
                            DesktopOrganizeBy::Name,
                            320,
                            top_margin,
                        );
                    }
                }
                popover_c.popdown();
            });
            menu.append(&snap_btn);

            let sep2 = GtkBox::new(Orientation::Horizontal, 0);
            sep2.add_css_class("desktop-context-separator");
            menu.append(&sep2);

            // 7. Cambiar fondo de pantalla...
            let wp_btn = create_menu_button("Fondo de pantalla y Estilo...", false);
            let popover_c = popover.clone();
            wp_btn.connect_clicked(move |_| {
                let mut cmd = std::process::Command::new("malus-de-settings");
                cmd.arg("--panel").arg("wallpaper");
                let _ = cmd.spawn();
                popover_c.popdown();
            });
            menu.append(&wp_btn);

            stack.add_named(&menu, Some("main"));
            stack.add_named(&organize_box, Some("organize"));
            stack.add_named(&new_folder_box, Some("new_folder"));
            stack.set_visible_child_name("main");

            popover.set_child(Some(&stack));
            apply_theme_to_popover(&popover, is_dark_mode_active());
            popover.connect_closed(|p| p.unparent());
            animate_widget_fade_in(&stack, 150);
            popover.popup();
        });
        overlay.add_controller(bg_context);

        let drag_mode = Rc::new(RefCell::new(DesktopDragMode::None));
        let desktop_drag = GestureDrag::new();
        desktop_drag.set_button(1);

        let drag_mode_begin = drag_mode.clone();
        let icons_begin = icons.clone();
        let rb_begin = rubberband.clone();
        let sel_begin = selected_paths.clone();
        let foc_begin = focused_path.clone();

        desktop_drag.connect_drag_begin(move |gesture, start_x, start_y| {
            let is_ctrl = gesture
                .current_event_state()
                .contains(gtk4::gdk::ModifierType::CONTROL_MASK);

            if let Some((clicked_path, item_box, _)) =
                find_icon_at(&icons_begin, start_x, start_y, &rb_begin)
            {
                let mut sel = sel_begin.borrow_mut();
                if is_ctrl {
                    if !sel.contains(&clicked_path) {
                        sel.insert(clicked_path.clone());
                        set_desktop_icon_selected(&item_box, true);
                    }
                } else if !sel.contains(&clicked_path) {
                    let mut sibling = icons_begin.first_child();
                    while let Some(child) = sibling {
                        sibling = child.next_sibling();
                        set_desktop_icon_selected(&child, false);
                    }
                    sel.clear();
                    sel.insert(clicked_path.clone());
                    set_desktop_icon_selected(&item_box, true);
                }
                *foc_begin.borrow_mut() = Some(clicked_path.clone());

                let mut start_positions = HashMap::new();
                let mut child = icons_begin.first_child();
                while let Some(c) = child {
                    child = c.next_sibling();
                    if c == rb_begin {
                        continue;
                    }
                    if let Some(p_str) = c.property::<Option<String>>("tooltip-text") {
                        let p = PathBuf::from(p_str);
                        if sel.contains(&p) {
                            start_positions.insert(p, icons_begin.child_position(&c));
                        }
                    }
                }

                // DragSource en el icono se encarga del arrastre Wayland y del feedback visual
                *drag_mode_begin.borrow_mut() = DesktopDragMode::None;
            } else {
                if !is_ctrl {
                    let mut sibling = icons_begin.first_child();
                    while let Some(child) = sibling {
                        sibling = child.next_sibling();
                        set_desktop_icon_selected(&child, false);
                    }
                    sel_begin.borrow_mut().clear();
                    *foc_begin.borrow_mut() = None;
                }

                *drag_mode_begin.borrow_mut() = DesktopDragMode::Rubberband {
                    start: (start_x, start_y),
                };
            }
        });

        let drag_mode_update = drag_mode.clone();
        let icons_update = icons.clone();
        let rb_update = rubberband.clone();
        let sel_update = selected_paths.clone();
        let cfg_update = current_config.clone();

        desktop_drag.connect_drag_update(move |gesture, offset_x, offset_y| {
            let mode = drag_mode_update.borrow().clone();
            match mode {
                DesktopDragMode::Icon {
                    ref start_positions,
                    ..
                } => {
                    if offset_x.abs() > 2.0 || offset_y.abs() > 2.0 {
                        gesture.set_state(gtk4::EventSequenceState::Claimed);
                        let metrics = IconLayoutMetrics::from_config(&cfg_update.borrow());
                        for (p, (orig_x, orig_y)) in start_positions.iter() {
                            let new_pos = clamp_icon_position(
                                orig_x + offset_x,
                                orig_y + offset_y,
                                (0.0, 0.0),
                                &metrics,
                                desktop_size,
                                top_margin,
                            );
                            move_desktop_icon(&icons_update, p, new_pos);
                        }
                    }
                }
                DesktopDragMode::Rubberband { start: (sx, sy) } => {
                    if offset_x.abs() > 3.0 || offset_y.abs() > 3.0 {
                        gesture.set_state(gtk4::EventSequenceState::Claimed);
                        let min_x = if offset_x >= 0.0 { sx } else { sx + offset_x };
                        let min_y = if offset_y >= 0.0 { sy } else { sy + offset_y };
                        let width = offset_x.abs().max(1.0) as i32;
                        let height = offset_y.abs().max(1.0) as i32;

                        icons_update.move_(&rb_update, min_x, min_y);
                        rb_update.set_size_request(width, height);
                        if !rb_update.is_visible() {
                            rb_update.set_visible(true);
                        }

                        let mut sibling = icons_update.first_child();
                        let mut sel = sel_update.borrow_mut();
                        while let Some(child) = sibling {
                            sibling = child.next_sibling();
                            if child == rb_update {
                                continue;
                            }
                            let (ix, iy) = icons_update.child_position(&child);
                            let iw = child.width().max(child.width_request()).max(64) as f64;
                            let ih = child.height().max(child.height_request()).max(64) as f64;

                            let intersects = min_x < ix + iw
                                && min_x + (width as f64) > ix
                                && min_y < iy + ih
                                && min_y + (height as f64) > iy;

                            if intersects {
                                set_desktop_icon_selected(&child, true);
                                if let Some(path_str) =
                                    child.property::<Option<String>>("tooltip-text")
                                {
                                    sel.insert(PathBuf::from(path_str));
                                }
                            }
                        }
                    }
                }
                DesktopDragMode::None => {}
            }
        });

        let drag_mode_end = drag_mode.clone();
        let icons_end = icons.clone();
        let rb_end = rubberband.clone();
        let pos_end = icon_positions.clone();
        let cfg_end = current_config.clone();
        let sel_end = selected_paths.clone();

        desktop_drag.connect_drag_end(move |gesture, offset_x, offset_y| {
            let mode = drag_mode_end.borrow().clone();
            match mode {
                DesktopDragMode::Rubberband { .. } => {
                    rb_end.set_visible(false);
                }
                DesktopDragMode::Icon {
                    ref clicked_path,
                    ref start_positions,
                    had_multiple_selected,
                } => {
                    let is_drag = offset_x.abs() > 3.0 || offset_y.abs() > 3.0;
                    if is_drag {
                        let cfg = cfg_end.borrow();
                        let metrics = IconLayoutMetrics::from_config(&cfg);
                        let mut positions_map = pos_end.borrow_mut();

                        for (p, (orig_x, orig_y)) in start_positions.iter() {
                            let mut final_pos = clamp_icon_position(
                                orig_x + offset_x,
                                orig_y + offset_y,
                                (0.0, 0.0),
                                &metrics,
                                desktop_size,
                                top_margin,
                            );
                            if cfg.snap_to_grid {
                                final_pos = snap_position_to_grid(
                                    final_pos,
                                    &metrics,
                                    cfg.align_left,
                                    desktop_size,
                                    top_margin,
                                );
                            }
                            move_desktop_icon(&icons_end, p, final_pos);
                            positions_map.insert(p.clone(), final_pos);
                        }

                        save_icon_positions(&positions_map);
                    } else {
                        for (p, (orig_x, orig_y)) in start_positions.iter() {
                            move_desktop_icon(&icons_end, p, (*orig_x, *orig_y));
                        }

                        let is_ctrl = gesture
                            .current_event_state()
                            .contains(gtk4::gdk::ModifierType::CONTROL_MASK);

                        if !is_ctrl && had_multiple_selected {
                            let mut sibling = icons_end.first_child();
                            while let Some(child) = sibling {
                                sibling = child.next_sibling();
                                if child == rb_end {
                                    continue;
                                }
                                if let Some(p_str) =
                                    child.property::<Option<String>>("tooltip-text")
                                {
                                    if p_str == clicked_path.to_string_lossy() {
                                        set_desktop_icon_selected(&child, true);
                                    } else {
                                        set_desktop_icon_selected(&child, false);
                                    }
                                }
                            }
                            let mut sel = sel_end.borrow_mut();
                            sel.clear();
                            sel.insert(clicked_path.clone());
                        }
                    }
                }
                DesktopDragMode::None => {}
            }
            *drag_mode_end.borrow_mut() = DesktopDragMode::None;
        });

        overlay.add_controller(desktop_drag);

        // Destino para soltar archivos arrastrados al escritorio (DropTarget)
        let drop_target = gtk4::DropTarget::new(
            gtk4::gdk::FileList::static_type(),
            gtk4::gdk::DragAction::COPY | gtk4::gdk::DragAction::MOVE,
        );
        let drop_dir = desktop_dir.to_path_buf();
        let drop_positions = icon_positions.clone();
        let internal_paths_drop = internal_drag_paths.clone();

        // En Wayland el cursor lleva consigo la miniatura flotante (WidgetPaintable).
        // No movemos el widget del escritorio durante connect_motion para evitar cualquier
        // estela o duplicación semitransparente detrás del puntero.
        drop_target.connect_enter(move |_, _x, _y| {
            gtk4::gdk::DragAction::MOVE
        });
        drop_target.connect_motion(move |_, _x, _y| {
            gtk4::gdk::DragAction::MOVE
        });

        let active_drag_drop = active_icon_drag.clone();
        let drop_icons = icons.clone();
        let cfg_drop = current_config.clone();
        let rec_drop = recently_dropped_paths.clone();

        drop_target.connect_drop(move |_, value, x, y| {
            let files = match value.get::<gtk4::gdk::FileList>() {
                Ok(files) => files,
                Err(error) => {
                    warn!(
                        "El contenido arrastrado al escritorio no es una lista de archivos: {}",
                        error
                    );
                    return false;
                }
            };

            let metrics = IconLayoutMetrics::from_config(&cfg_drop.borrow());
            let snap = cfg_drop.borrow().snap_to_grid;
            let align_left = cfg_drop.borrow().align_left;

            let mut moved_any = false;
            for (index, source) in files.files().into_iter().enumerate() {
                let Some(name) = source.basename() else {
                    warn!(
                        "No se pudo mover al escritorio un elemento sin nombre: {:?}",
                        source
                    );
                    continue;
                };
                let destination = gio::File::for_path(&drop_dir).child(&name);
                if source.equal(&destination) {
                    if let Some(path) = source.path() {
                        let pointer_offset = active_drag_drop
                            .borrow()
                            .as_ref()
                            .filter(|(active_path, _, _)| active_path == &path)
                            .map(|(_, _, pointer_offset)| *pointer_offset)
                            .unwrap_or((
                                f64::from(metrics.box_width) / 2.0,
                                f64::from(metrics.box_height) / 2.0,
                            ));
                        let offset_idx = (index % 4) as f64 * 24.0;
                        let mut position = clamp_icon_position(x + offset_idx, y + offset_idx, pointer_offset, &metrics, desktop_size, top_margin);
                        if snap {
                            position = snap_position_to_grid(position, &metrics, align_left, desktop_size, top_margin);
                        }
                        if move_desktop_icon(&drop_icons, &path, position) {
                            drop_positions.borrow_mut().insert(path.clone(), position);
                            internal_paths_drop.borrow_mut().insert(path.clone());
                            let mut sibling = drop_icons.first_child();
                            while let Some(child) = sibling {
                                sibling = child.next_sibling();
                                if let Some(p_str) = child.property::<Option<String>>("tooltip-text") {
                                    if p_str == path.to_string_lossy() {
                                        child.add_css_class("landing");
                                        let c = child.clone();
                                        glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                                            c.remove_css_class("landing");
                                        });
                                        break;
                                    }
                                }
                            }
                            moved_any = true;
                        }
                    }
                    continue;
                }

                let mut position = (
                    (x + (index % 4) as f64 * 24.0)
                        .clamp(16.0, f64::from((desktop_size.0 - metrics.box_width - 16).max(16))),
                    (y + (index % 4) as f64 * 24.0).clamp(
                        top_margin,
                        f64::from((desktop_size.1 - metrics.box_height - 20).max(top_margin as i32)),
                    ),
                );
                if snap {
                    position = snap_position_to_grid(position, &metrics, align_left, desktop_size, top_margin);
                }

                match source.move_(
                    &destination,
                    gio::FileCopyFlags::NONE,
                    gio::Cancellable::NONE,
                    None,
                ) {
                    Ok(()) => {
                        if let Some(path) = destination.path() {
                            drop_positions.borrow_mut().insert(path.clone(), position);
                            internal_paths_drop.borrow_mut().insert(path.clone());
                            rec_drop.borrow_mut().insert(path);
                        }
                        moved_any = true;
                    }
                    Err(error) => {
                        warn!(
                            "No se pudo mover {:?} al escritorio {:?}: {}",
                            source, drop_dir, error
                        );
                    }
                }
            }
            if moved_any {
                save_icon_positions(&drop_positions.borrow());
            }
            moved_any
        });

        overlay.add_controller(drop_target);
        overlay.add_overlay(&icons);

        // Monitor en tiempo real para reflejar cambios en la carpeta ~/Escritorio
        let desktop_file = gio::File::for_path(desktop_dir);
        match desktop_file.monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
            Ok(monitor) => {
                let trigger_c = trigger_refresh.clone();
                monitor.connect_changed(move |_, _, _, _| {
                    trigger_c();
                });
                desktop_monitors.borrow_mut().push(monitor);
            }
            Err(error) => {
                warn!(
                    "No se pudo monitorear la carpeta de escritorio {:?}: {}",
                    desktop_dir, error
                );
            }
        }

        // Monitor reactivo para el archivo de configuración desktop.toml
        let config_file = gio::File::for_path(DesktopIconsConfig::config_path());
        if let Ok(cfg_monitor) = config_file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
            let cfg_holder = current_config.clone();
            let trigger_c = trigger_refresh.clone();
            cfg_monitor.connect_changed(move |_, _, _, _| {
                *cfg_holder.borrow_mut() = DesktopIconsConfig::load();
                trigger_c();
            });
            desktop_monitors.borrow_mut().push(cfg_monitor);
        }
    } else {
        // Monitor secundario: menú contextual básico en fondo de pantalla
        let bg_context = GestureClick::new();
        bg_context.set_button(3);
        let overlay_context = overlay.clone();

        bg_context.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            let popover = Popover::builder()
                .has_arrow(false)
                .autohide(true)
                .css_classes(["desktop-popover-menu"])
                .build();
            popover.set_parent(&overlay_context);
            register_wallpaper_popover(&popover);

            let pointing_rect = gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
            popover.set_pointing_to(Some(&pointing_rect));

            let menu = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(2)
                .margin_top(6)
                .margin_bottom(6)
                .margin_start(6)
                .margin_end(6)
                .css_classes(["desktop-context-menu"])
                .build();

            let wp_btn = create_menu_button("Fondo de pantalla y Estilo...", false);
            let popover_c = popover.clone();
            wp_btn.connect_clicked(move |_| {
                let mut cmd = std::process::Command::new("malus-de-settings");
                cmd.arg("--panel").arg("wallpaper");
                let _ = cmd.spawn();
                popover_c.popdown();
            });
            menu.append(&wp_btn);

            popover.set_child(Some(&menu));
            apply_theme_to_popover(&popover, is_dark_mode_active());
            popover.connect_closed(|p| p.unparent());
            animate_widget_fade_in(&menu, 140);
            popover.popup();
        });
        overlay.add_controller(bg_context);
    }

    window.set_child(Some(&overlay));
    window.present();
}

fn main() -> Result<()> {
    init_logging("malus_de_wallpaper");
    let args = Args::parse();
    info!("Iniciando Malus DE Wallpaper Daemon...");

    // Esperar de forma activa a que el compositor esté listo y aceptando conexiones
    malus_de_common::wayland::wait_for_wayland(10);

    let app = Application::builder()
        .application_id("org.malus_de.Wallpaper")
        .build();

    let image_path = args.image;
    let desktop_dir = resolve_desktop_dir();
    let desktop_monitors = Rc::new(RefCell::new(Vec::new()));
    let icon_positions = Rc::new(RefCell::new(load_icon_positions()));
    let internal_drag_paths = Rc::new(RefCell::new(HashSet::new()));
    let active_icon_drag = Rc::new(RefCell::new(None));

    app.connect_startup(|_| {
        let initial_dark = malus_de_common::config::is_dark_mode_active();
        set_is_dark_mode(initial_dark);

        // 1. Escuchar mensajes IPC del compositor (ThemeChanged entre procesos en tiempo real)
        let (tx, rx) = async_channel::unbounded::<malus_de_common::ipc::IpcMessage>();
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                if let malus_de_common::ipc::IpcMessage::ThemeChanged { is_dark, .. } = msg {
                    info!("🎨 [Wallpaper] IPC ThemeChanged recibido: is_dark={}", is_dark);
                    set_is_dark_mode(is_dark);
                }
            }
        });

        std::thread::spawn(move || {
            let sock_path = malus_de_common::ipc::ipc_socket_path();
            loop {
                if let Ok(stream) = std::os::unix::net::UnixStream::connect(&sock_path) {
                    let reader = std::io::BufReader::new(stream);
                    for line in std::io::BufRead::lines(reader) {
                        if let Ok(line) = line {
                            if let Ok(msg) = serde_json::from_str::<malus_de_common::ipc::IpcMessage>(&line) {
                                let _ = tx.send_blocking(msg);
                            }
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });

        // 2. Monitor fallback de GSettings (para sesiones GNOME estándar)
        let has_schema = gio::SettingsSchemaSource::default()
            .map(|s| s.lookup("org.gnome.desktop.interface", true).is_some())
            .unwrap_or(false);
        if has_schema {
            let gsettings = gio::Settings::new("org.gnome.desktop.interface");
            gsettings.connect_changed(Some("color-scheme"), move |settings, _| {
                let scheme = settings.string("color-scheme");
                let to_dark = scheme.contains("dark");
                info!("🎨 [Wallpaper] GSettings color-scheme: {}", scheme);
                set_is_dark_mode(to_dark);
            });
            Box::leak(Box::new(gsettings));
        }

        // 3. Registrar hoja de estilo CSS
        let provider = CssProvider::new();
        provider.load_from_string(DEFAULT_GRADIENT_CSS);
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_USER + 100,
            );
        }
    });

    app.connect_activate(move |application| {
        let resolved_path = image_path.clone().or_else(|| {
            let config_dir = std::env::var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    std::env::var("HOME")
                        .map(|h| PathBuf::from(h).join(".config/malus-de"))
                        .unwrap_or_else(|_| PathBuf::from("/etc/malus-de"))
                });
            let candidates = [
                config_dir.join("wallpaper.png"),
                config_dir.join("wallpaper.jpg"),
                config_dir.join("wallpaper.jpeg"),
                config_dir.join("wallpaper.webp"),
                config_dir.join("wallpaper.svg"),
            ];
            candidates.into_iter().find(|p| p.exists())
        });

        if let Some(desktop_dir) = &desktop_dir {
            info!(
                "Mostrando contenido de la carpeta de escritorio: {:?}",
                desktop_dir
            );
        } else {
            warn!("No se pudo determinar la carpeta de escritorio del usuario.");
        }

        let mut created = 0;
        if let Some(display) = gtk4::gdk::Display::default() {
            let monitors = display.monitors();
            let displays_cfg = DisplaysConfig::load();
            let primary_cfg = displays_cfg.outputs.iter().find(|output| output.primary).cloned();
            let monitor_list: Vec<gtk4::gdk::Monitor> = (0..monitors.n_items())
                .filter_map(|i| monitors.item(i))
                .filter_map(|monitor| monitor.downcast().ok())
                .collect();
            let primary_index = primary_cfg
                .as_ref()
                .and_then(|cfg| {
                    monitor_list.iter().position(|monitor| {
                        if let Some(connector) = monitor.connector() {
                            if connector.as_str() == cfg.connector {
                                return true;
                            }
                        }
                        if let Some(model) = monitor.model() {
                            if model.as_str() == cfg.name || model.as_str() == cfg.connector {
                                return true;
                            }
                        }
                        if let Some(desc) = monitor.description() {
                            if desc.contains(&cfg.name) || desc.contains(&cfg.connector) {
                                return true;
                            }
                        }
                        let geo = monitor.geometry();
                        geo.x() == cfg.x && geo.y() == cfg.y
                    })
                })
                .unwrap_or(0);

            match (primary_cfg.as_ref(), monitor_list.get(primary_index)) {
                (Some(cfg), Some(_)) => info!(
                    "Mostrando iconos del escritorio en la pantalla principal: {} ({})",
                    cfg.connector, cfg.name
                ),
                (_, Some(monitor)) => info!(
                    "No se encontró la pantalla principal configurada; usando {} para los iconos del escritorio.",
                    monitor
                        .connector()
                        .map(|name| name.to_string())
                        .unwrap_or_else(|| "la primera pantalla".to_string())
                ),
                (_, None) => {}
            }

            for (index, monitor) in monitor_list.iter().enumerate() {
                let is_primary = index == primary_index;
                create_wallpaper_window(
                    application,
                    Some(monitor),
                    &resolved_path,
                    if is_primary {
                        desktop_dir.as_deref()
                    } else {
                        None
                    },
                    &desktop_monitors,
                    &icon_positions,
                    &internal_drag_paths,
                    &active_icon_drag,
                    is_primary,
                );
                created += 1;
            }
        }

        if created == 0 {
            create_wallpaper_window(
                application,
                None,
                &resolved_path,
                desktop_dir.as_deref(),
                &desktop_monitors,
                &icon_positions,
                &internal_drag_paths,
                &active_icon_drag,
                true,
            );
        }
    });

    app.run_with_args(&["malus-de-wallpaper"]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        calculate_grid_position, clamp_icon_position, desktop_dir_from_config,
        dragged_icon_position, snap_position_to_grid, IconLayoutMetrics, StackCategory,
        DESKTOP_SECONDARY_TOP_MARGIN, DESKTOP_TOP_MARGIN,
    };
    use gtk4::prelude::*;
    use malus_de_common::config::DesktopIconsConfig;
    use std::path::{Path, PathBuf};

    #[test]
    fn keeps_pointer_offset_and_clamps_icon_to_desktop() {
        let metrics = IconLayoutMetrics {
            icon_size: 64,
            box_width: 108,
            box_height: 112,
            spacing: 8,
        };
        assert_eq!(
            clamp_icon_position(150.0, 100.0, (20.0, 30.0), &metrics, (800, 600), DESKTOP_TOP_MARGIN),
            (130.0, 70.0)
        );
        assert_eq!(
            clamp_icon_position(-20.0, -30.0, (20.0, 30.0), &metrics, (800, 600), DESKTOP_TOP_MARGIN),
            (16.0, DESKTOP_TOP_MARGIN)
        );
    }

    #[test]
    fn follows_drag_offset_without_shifting_the_pointer_anchor() {
        let metrics = IconLayoutMetrics {
            icon_size: 64,
            box_width: 108,
            box_height: 112,
            spacing: 8,
        };
        assert_eq!(
            dragged_icon_position((130.0, 70.0), (20.0, 30.0), (15.0, -10.0), &metrics, (800, 600), DESKTOP_TOP_MARGIN),
            (145.0, 60.0)
        );
    }

    #[test]
    fn resolves_spanish_desktop_directory_from_user_dirs() {
        let path = desktop_dir_from_config(
            "XDG_DESKTOP_DIR=\"$HOME/Escritorio\"\n",
            Path::new("/home/user"),
        );
        assert_eq!(path, PathBuf::from("/home/user/Escritorio"));
    }

    #[test]
    fn defaults_to_escritorio_when_user_dir_is_not_configured() {
        let path = desktop_dir_from_config("", Path::new("/home/user"));
        assert_eq!(path, PathBuf::from("/home/user/Escritorio"));
    }

    #[test]
    fn test_get_unique_folder_name() {
        let temp_dir = std::env::temp_dir().join("malus_test_new_folder");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let _ = std::fs::create_dir_all(&temp_dir);

        let f1 = super::get_unique_folder_name(&temp_dir, "Nueva carpeta");
        assert_eq!(f1, temp_dir.join("Nueva carpeta"));
        std::fs::create_dir_all(&f1).unwrap();

        let f2 = super::get_unique_folder_name(&temp_dir, "Nueva carpeta");
        assert_eq!(f2, temp_dir.join("Nueva carpeta 2"));
        std::fs::create_dir_all(&f2).unwrap();

        let f3 = super::get_unique_folder_name(&temp_dir, "Nueva carpeta");
        assert_eq!(f3, temp_dir.join("Nueva carpeta 3"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_organize_desktop_icons_folders_first_and_alphabetical() {
        let temp_dir = std::env::temp_dir().join("malus_test_organize_desktop");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let _ = std::fs::create_dir_all(&temp_dir);

        let folder_b = temp_dir.join("Carpeta B");
        let folder_a = temp_dir.join("Carpeta A");
        let file_z = temp_dir.join("archivo_z.txt");
        let file_a = temp_dir.join("archivo_a.txt");

        std::fs::create_dir_all(&folder_b).unwrap();
        std::fs::create_dir_all(&folder_a).unwrap();
        std::fs::write(&file_z, "test").unwrap();
        std::fs::write(&file_a, "test").unwrap();

        let config = DesktopIconsConfig::default();
        let icon_positions = std::rc::Rc::new(std::cell::RefCell::new(std::collections::HashMap::new()));

        super::organize_desktop_icons(
            &temp_dir,
            (1920, 1080),
            &config,
            &icon_positions,
            super::DesktopOrganizeBy::Name,
            DESKTOP_TOP_MARGIN,
        );

        let pos = icon_positions.borrow();
        assert_eq!(pos.len(), 4);

        // Carpetas primero (Carpeta A en slot 0, Carpeta B en slot 1, archivo_a en slot 2, archivo_z en slot 3)
        let metrics = IconLayoutMetrics::from_config(&config);
        let rows = ((1080 - (DESKTOP_TOP_MARGIN as i32) - 80).max(1) / metrics.cell_height()).max(1) as usize;

        let slot_0 = calculate_grid_position(0, rows, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);
        let slot_1 = calculate_grid_position(1, rows, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);
        let slot_2 = calculate_grid_position(2, rows, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);
        let slot_3 = calculate_grid_position(3, rows, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);

        assert_eq!(pos.get(&folder_a).copied(), Some(slot_0));
        assert_eq!(pos.get(&folder_b).copied(), Some(slot_1));
        assert_eq!(pos.get(&file_a).copied(), Some(slot_2));
        assert_eq!(pos.get(&file_z).copied(), Some(slot_3));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_snap_position_to_grid_mac_and_left() {
        let config = DesktopIconsConfig::default();
        let metrics = IconLayoutMetrics::from_config(&config);

        // Snap a la derecha estilo macOS
        let pos = (1800.0, 45.0);
        let snapped = snap_position_to_grid(pos, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);
        assert!(snapped.0 <= 1920.0 - metrics.box_width as f64);
        assert_eq!(snapped.1, DESKTOP_TOP_MARGIN);

        // Snap a la izquierda
        let pos_left = (25.0, 45.0);
        let snapped_left = snap_position_to_grid(pos_left, &metrics, true, (1920, 1080), DESKTOP_TOP_MARGIN);
        assert_eq!(snapped_left.0, 16.0);
        assert_eq!(snapped_left.1, DESKTOP_TOP_MARGIN);
    }

    #[test]
    fn test_snap_position_to_grid_secondary_display() {
        let config = DesktopIconsConfig::default();
        let metrics = IconLayoutMetrics::from_config(&config);

        // En pantalla secundaria no hay Top Bar, el margen superior es 16.0
        let pos = (1800.0, 5.0);
        let snapped = snap_position_to_grid(pos, &metrics, false, (1920, 1080), DESKTOP_SECONDARY_TOP_MARGIN);
        assert!(snapped.0 <= 1920.0 - metrics.box_width as f64);
        assert_eq!(snapped.1, DESKTOP_SECONDARY_TOP_MARGIN);

        let pos_left = (25.0, 5.0);
        let snapped_left = snap_position_to_grid(pos_left, &metrics, true, (1920, 1080), DESKTOP_SECONDARY_TOP_MARGIN);
        assert_eq!(snapped_left.0, 16.0);
        assert_eq!(snapped_left.1, DESKTOP_SECONDARY_TOP_MARGIN);
    }

    #[test]
    fn test_stack_category_categorization() {
        assert_eq!(
            StackCategory::categorize(Path::new("Captura de pantalla 2026.png"), None),
            StackCategory::Screenshots
        );
        assert_eq!(
            StackCategory::categorize(Path::new("foto.jpg"), Some("image/jpeg")),
            StackCategory::Images
        );
        assert_eq!(
            StackCategory::categorize(Path::new("informe.pdf"), Some("application/pdf")),
            StackCategory::Documents
        );
        assert_eq!(
            StackCategory::categorize(Path::new("video.mp4"), Some("video/mp4")),
            StackCategory::Videos
        );
        assert_eq!(
            StackCategory::categorize(Path::new("cancion.mp3"), Some("audio/mpeg")),
            StackCategory::Audio
        );
        assert_eq!(
            StackCategory::categorize(Path::new("main.rs"), Some("text/rust")),
            StackCategory::Code
        );
        assert_eq!(
            StackCategory::categorize(Path::new("archivo.zip"), None),
            StackCategory::Archives
        );
    }

    #[test]
    fn test_calculate_grid_position_order() {
        let config = DesktopIconsConfig::default();
        let metrics = IconLayoutMetrics::from_config(&config);
        let pos0 = calculate_grid_position(0, 5, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);
        let pos1 = calculate_grid_position(1, 5, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);
        assert_eq!(pos0.0, pos1.0); // Misma columna
        assert!(pos1.1 > pos0.1); // Siguiente fila hacia abajo
    }

    #[test]
    fn test_desktop_context_menu_css_has_acrylic_colors_for_both_themes() {
        let css = super::DEFAULT_GRADIENT_CSS;
        // Fondo oscuro acrílico de popover
        assert!(css.contains("rgba(30, 30, 36, 0.82)"));
        // Fondo claro acrílico de popover
        assert!(css.contains("rgba(246, 246, 248, 0.85)"));
        // Clase reactiva light-popover y selectores
        assert!(css.contains("popover.desktop-popover-menu.light-popover > contents"));
        assert!(css.contains("popover.light-popover > contents"));
        assert!(css.contains(".desktop-context-btn"));
        assert!(css.contains(".desktop-context-separator"));
        // Colores de texto oscuro (#f1f5f9) y claro (#1e293b)
        assert!(css.contains("#f1f5f9"));
        assert!(css.contains("#1e293b"));
        // Hover azul macOS
        assert!(css.contains("#007aff"));
        // Transiciones suaves de hover y activación en botones estilo macOS
        assert!(css.contains("cubic-bezier(0.16, 1, 0.3, 1)"));
        // GTK4 CSS no soporta !important ni queries @media condicionales
        assert!(!css.contains("!important"));
        assert!(!css.contains("@media"));
    }

    #[test]
    fn test_ease_out_cubic() {
        assert_eq!(super::ease_out_cubic(0.0), 0.0);
        assert_eq!(super::ease_out_cubic(1.0), 1.0);
        assert!((super::ease_out_cubic(0.5) - 0.875).abs() < 1e-6);
        assert_eq!(super::ease_out_cubic(-0.5), 0.0);
        assert_eq!(super::ease_out_cubic(1.5), 1.0);
    }

    #[test]
    fn test_desktop_drag_and_drop_macos_css_classes() {
        let css = include_str!("styles/default-gradient.css");
        assert!(css.contains(".desktop-icon.desktop-icon-dragging"));
        assert!(css.contains("opacity: 0.0;"));
        assert!(css.contains("transition: none;"));
        assert!(css.contains(".desktop-icon.folder-drop-hover"));
        assert!(css.contains(".desktop-icon.landing"));
        assert!(css.contains("@keyframes macos-drop-landing"));
        assert!(css.contains("cubic-bezier(0.16, 1, 0.3, 1)"));
        assert!(!css.contains("!important"));
    }

    #[test]
    fn test_drag_preview_widget_paintable() {
        if gtk4::init().is_err() {
            return;
        }
        let fixed = gtk4::Fixed::new();
        let item_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        item_box.add_css_class("desktop-icon");
        item_box.add_css_class("selected");
        let lbl = gtk4::Label::new(Some("Archivo"));
        item_box.append(&lbl);
        fixed.put(&item_box, 100.0, 100.0);

        let paintable = gtk4::WidgetPaintable::new(Some(&item_box));
        let src = gtk4::DragSource::new();
        src.set_icon(Some(&paintable), 35, 40);

        // Al comenzar el arrastre se desplaza fuera de pantalla (-5000)
        fixed.move_(&item_box, -5000.0, -5000.0);
        assert_eq!(fixed.child_position(&item_box), (-5000.0, -5000.0));
    }

    #[test]
    fn test_calculate_organize_targets_by_kind() {
        let temp_dir = std::env::temp_dir().join("malus_test_organize_by_kind");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let _ = std::fs::create_dir_all(&temp_dir);

        let folder = temp_dir.join("Subcarpeta");
        let txt_file = temp_dir.join("documento.txt");
        let jpg_file = temp_dir.join("imagen.jpg");

        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(&txt_file, "texto").unwrap();
        std::fs::write(&jpg_file, "foto").unwrap();

        let config = DesktopIconsConfig::default();
        let targets = super::calculate_organize_targets(
            &temp_dir,
            (1920, 1080),
            &config,
            super::DesktopOrganizeBy::Kind,
            DESKTOP_TOP_MARGIN,
        );

        assert_eq!(targets.len(), 3);
        let metrics = IconLayoutMetrics::from_config(&config);
        let rows = ((1080 - (DESKTOP_TOP_MARGIN as i32) - 80).max(1) / metrics.cell_height()).max(1) as usize;

        let slot_0 = calculate_grid_position(0, rows, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);
        let slot_1 = calculate_grid_position(1, rows, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);
        let slot_2 = calculate_grid_position(2, rows, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);

        // Subcarpeta primero (slot 0), luego jpg (slot 1 por orden alfabético de extensión), luego txt (slot 2)
        assert_eq!(targets.get(&folder).copied(), Some(slot_0));
        assert_eq!(targets.get(&jpg_file).copied(), Some(slot_1));
        assert_eq!(targets.get(&txt_file).copied(), Some(slot_2));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_truncate_desktop_name() {
        // 1. Archivo corto: no se corta
        let short = super::truncate_desktop_name("Notas.txt", 24);
        assert_eq!(short, "Notas.txt");

        // 2. Archivo largo con extensión: corta por el centro y preserva la extensión
        let long_file = super::truncate_desktop_name("Documento_De_Investigacion_Final_2026.pdf", 24);
        assert!(long_file.contains('…'));
        assert!(long_file.ends_with(".pdf"));
        assert!(long_file.chars().count() <= 24);

        // 3. Extensión compuesta .tar.gz: preserva la extensión completa
        let tar_file = super::truncate_desktop_name("malus_de_backup_noviembre_2026.tar.gz", 24);
        assert!(tar_file.contains('…'));
        assert!(tar_file.ends_with(".tar.gz"));
        assert!(tar_file.chars().count() <= 24);

        // 4. Carpeta larga sin extensión: divide equitativamente por el centro
        let long_folder = super::truncate_desktop_name("Carpeta con fotos de las vacaciones 2026", 24);
        assert!(long_folder.contains('…'));
        assert!(long_folder.starts_with("Carpeta"));
        assert!(long_folder.ends_with("2026"));
        assert!(long_folder.chars().count() <= 24);

        // 5. Caracteres UTF-8 multibyte (español, acentos, eñe)
        let utf8_file = super::truncate_desktop_name("Diseño_gráfico_año_2026_españa_final.png", 24);
        assert!(utf8_file.contains('…'));
        assert!(utf8_file.ends_with(".png"));
        assert!(utf8_file.chars().count() <= 24);
    }

    #[test]
    fn test_resolve_desktop_icon_paintable() {
        if gtk4::init().is_err() {
            return;
        }
        let paintable = super::resolve_desktop_icon_paintable(
            Path::new("/dummy/nonexistent.png"),
            None,
            None,
            48,
        );
        let src = gtk4::DragSource::new();
        if let Some(ref p) = paintable {
            src.set_icon(Some(p), 24, 24);
        }
    }
}
