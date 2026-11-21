use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, ColorDialog, DrawingArea, DropDown,
    EventControllerMotion, FileDialog, FileFilter, FlowBox, Image, Label, Orientation, Overlay,
    PolicyType, ScrolledWindow, StringList,
};
use malus_de_common::config::{
    default_bundled_wallpaper_path, is_dark_mode_active, malus_wallpaper_config_dir,
    normalize_wallpaper_hex_color, parse_wallpaper_hex_color, DEFAULT_WALLPAPER_FILENAME,
    WallpaperConfig, WallpaperFitMode, WallpaperMode,
};
use malus_de_common::ipc::{IpcClient, IpcMessage};
use malus_de_common::mac_switch::MacSwitch as Switch;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::squircle_path;
use crate::wifi_pane::{make_squircle_button, wrap_squircle_card, WIFI_CARD_SQUIRCLE_RADIUS};

const FIT_MODE_LABELS: [&str; 4] = ["Llenar pantalla", "Ajustar", "Estirar", "Centrar"];

const CURATED_SOLID_COLORS: [(&str, &str); 21] = [
    ("Negro", "#000000"),
    ("Lavanda pizarra", "#6C6EA0"),
    ("Azul océano", "#4DA6C2"),
    ("Rosa terracota", "#C86B6C"),
    ("Azul cobalto", "#445BD1"),
    ("Melocotón claro", "#ECD3C3"),
    ("Arena cálida", "#E8D5BD"),
    ("Ocre dorado", "#CFA85D"),
    ("Grafito", "#3A3A3C"),
    ("Gris niebla", "#8E8E93"),
    ("Azul medianoche", "#1C2B4D"),
    ("Verde salvia", "#6A8D73"),
    ("Verde bosque", "#2E5A44"),
    ("Ciruela", "#5B3256"),
    ("Borgoña", "#7A283B"),
    ("Terracota", "#B85C38"),
    ("Naranja coral", "#E07A5F"),
    ("Amarillo miel", "#D9A441"),
    ("Turquesa profundo", "#1D6F72"),
    ("Azul cielo", "#6CA6CD"),
    ("Blanco perlado", "#EDEBE6"),
];

const PENDING_AERIAL_SECTIONS: [(&str, &str); 6] = [
    (
        "Paisaje",
        "Colección de vistas aéreas de paisajes pendiente de contenido local.",
    ),
    (
        "Urbano",
        "Colección de vistas aéreas urbanas pendiente de contenido local.",
    ),
    (
        "Subacuático",
        "Colección de vistas aéreas submarinas pendiente de contenido local.",
    ),
    (
        "Tierra",
        "Colección de vistas orbitales de la Tierra pendiente de contenido local.",
    ),
    (
        "Mac",
        "Colección clásica Mac en color pendiente de contenido local.",
    ),
    (
        "Vistas aéreas aleatorias",
        "Rotación aleatoria de vistas aéreas pendiente de contenido local.",
    ),
];

#[derive(Clone, Debug, PartialEq, Eq)]
enum WallpaperKind {
    AuroraGradient,
    DynamicPair { light: PathBuf, dark: PathBuf },
    StaticImage(PathBuf),
}

#[derive(Clone, Debug)]
struct WallpaperItem {
    title: String,
    kind: WallpaperKind,
}

pub fn build_wallpaper_file_filter() -> FileFilter {
    let filter = FileFilter::new();
    filter.set_name(Some("Archivos de imagen (*.jpg, *.png, *.webp, *.svg)"));
    for mime in ["image/jpeg", "image/png", "image/webp", "image/svg+xml"] {
        filter.add_mime_type(mime);
    }
    filter
}

pub fn wallpaper_chooser_initial_folder_in(home: &Path) -> Option<PathBuf> {
    [
        "Imágenes/Wallpapers",
        "Pictures/Wallpapers",
        "Imágenes",
        "Pictures",
    ]
    .into_iter()
    .map(|rel| home.join(rel))
    .find(|p| p.is_dir())
}

pub fn wallpaper_chooser_initial_folder() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    wallpaper_chooser_initial_folder_in(&home)
}

fn friendly_stem_title(stem: &str) -> String {
    let clean = stem
        .replace('_', " ")
        .replace('-', " ")
        .trim()
        .to_string();
    let mut words = Vec::new();
    for word in clean.split_whitespace() {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            let cap = first.to_uppercase().collect::<String>() + chars.as_str();
            words.push(cap);
        }
    }
    if words.is_empty() {
        "Fondo".to_string()
    } else {
        words.join(" ")
    }
}

fn sync_legacy_wallpaper_files(
    malus_dir: &Path,
    image_src: Option<&Path>,
    is_gradient: bool,
) {
    let _ = std::fs::create_dir_all(malus_dir);
    let gradient_marker = malus_dir.join(".wallpaper-gradient");

    for ext in &["png", "jpg", "jpeg", "webp", "svg"] {
        let p = malus_dir.join(format!("wallpaper.{}", ext));
        let _ = std::fs::remove_file(&p);
    }

    if let Some(src) = image_src.filter(|p| p.is_file()) {
        let _ = std::fs::remove_file(&gradient_marker);
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_lowercase();
        let target = malus_dir.join(format!("wallpaper.{}", ext));
        let _ = std::fs::copy(src, &target);

        let uri = format!("file://{}", target.to_str().unwrap_or(""));
        let _ = Command::new("gsettings")
            .env("XDG_CONFIG_HOME", malus_dir)
            .env("GSETTINGS_BACKEND", "keyfile")
            .args(["set", "org.gnome.desktop.background", "picture-uri", &uri])
            .status();
        let _ = Command::new("gsettings")
            .env("XDG_CONFIG_HOME", malus_dir)
            .env("GSETTINGS_BACKEND", "keyfile")
            .args([
                "set",
                "org.gnome.desktop.background",
                "picture-uri-dark",
                &uri,
            ])
            .status();
    } else if is_gradient {
        let _ = std::fs::write(&gradient_marker, b"1");
        let _ = Command::new("gsettings")
            .env("XDG_CONFIG_HOME", malus_dir)
            .env("GSETTINGS_BACKEND", "keyfile")
            .args(["reset", "org.gnome.desktop.background", "picture-uri"])
            .status();
        let _ = Command::new("gsettings")
            .env("XDG_CONFIG_HOME", malus_dir)
            .env("GSETTINGS_BACKEND", "keyfile")
            .args(["reset", "org.gnome.desktop.background", "picture-uri-dark"])
            .status();
    } else {
        // Color sólido: marcar para evitar que ensure_default_wallpaper sobreescriba con foto default
        let _ = std::fs::write(&gradient_marker, b"solid");
        let _ = Command::new("gsettings")
            .env("XDG_CONFIG_HOME", malus_dir)
            .env("GSETTINGS_BACKEND", "keyfile")
            .args(["reset", "org.gnome.desktop.background", "picture-uri"])
            .status();
        let _ = Command::new("gsettings")
            .env("XDG_CONFIG_HOME", malus_dir)
            .env("GSETTINGS_BACKEND", "keyfile")
            .args(["reset", "org.gnome.desktop.background", "picture-uri-dark"])
            .status();
    }
}

fn ensure_wallpaper_daemon_running() {
    let already_running = Command::new("pgrep")
        .args(["-x", "malus-de-wallpaper"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if already_running {
        return;
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let wayland_disp = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".to_string());
    let bin_candidates = [
        std::env::var_os("MALUS_DE_BIN_DIR")
            .map(|dir| PathBuf::from(dir).join("malus-de-wallpaper")),
        Some(PathBuf::from(&home).join("Proyectos/malus-de/target/release/malus-de-wallpaper")),
        Some(PathBuf::from(&home).join("Apps/malus-de/target/release/malus-de-wallpaper")),
        Some(PathBuf::from("/usr/local/bin/malus-de-wallpaper")),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.exists())
    .map(|p| p.to_string_lossy().to_string())
    .unwrap_or_else(|| "malus-de-wallpaper".to_string());

    let mut cmd = Command::new(&bin_candidates);
    cmd.env("WAYLAND_DISPLAY", &wayland_disp);
    cmd.env("GDK_BACKEND", "wayland");
    cmd.env("XDG_CONFIG_HOME", format!("{}/.config/malus-de", home));
    let _ = cmd.spawn();
}

fn save_and_apply_wallpaper_config(cfg: &WallpaperConfig) {
    let malus_dir = malus_wallpaper_config_dir();
    let _ = cfg.save_in(&malus_dir);
    let is_dark = is_dark_mode_active();
    match cfg.mode {
        WallpaperMode::Image => {
            let src = cfg
                .image_path
                .as_deref()
                .map(PathBuf::from)
                .filter(|p| p.is_file());
            sync_legacy_wallpaper_files(&malus_dir, src.as_deref(), false);
        }
        WallpaperMode::DynamicPair => {
            let chosen = if is_dark {
                cfg.dark_image_path.as_deref()
            } else {
                cfg.light_image_path.as_deref()
            };
            let src = chosen.map(PathBuf::from).filter(|p| p.is_file());
            sync_legacy_wallpaper_files(&malus_dir, src.as_deref(), false);
        }
        WallpaperMode::Gradient => {
            sync_legacy_wallpaper_files(&malus_dir, None, true);
        }
        WallpaperMode::SolidColor => {
            sync_legacy_wallpaper_files(&malus_dir, None, false);
        }
    }

    let _ = IpcClient::send_command(IpcMessage::ReloadWallpaperConfig);
    ensure_wallpaper_daemon_running();
}

fn discover_dynamic_wallpapers() -> Vec<WallpaperItem> {
    let mut items = vec![WallpaperItem {
        title: "Degradado Aurora".to_string(),
        kind: WallpaperKind::AuroraGradient,
    }];

    let mut pairs: HashMap<String, (Option<PathBuf>, Option<PathBuf>)> = HashMap::new();
    for dir in &[
        "/usr/share/backgrounds/fedora-workstation",
        "/usr/share/backgrounds/gnome",
        "/usr/share/backgrounds/f45",
    ] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if !p.is_file() {
                    continue;
                }
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if !matches!(ext.as_str(), "webp" | "jpg" | "jpeg" | "png") {
                    continue;
                }
                let base_key = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|stem| {
                        stem.strip_suffix("_light")
                            .or_else(|| stem.strip_suffix("-l"))
                            .map(|b| (true, b.to_string()))
                            .or_else(|| {
                                stem.strip_suffix("_dark")
                                    .or_else(|| stem.strip_suffix("-d"))
                                    .map(|b| (false, b.to_string()))
                            })
                    });
                if let Some((is_light, base)) = base_key {
                    let entry_pair = pairs.entry(base).or_default();
                    if is_light {
                        entry_pair.0 = Some(p);
                    } else {
                        entry_pair.1 = Some(p);
                    }
                }
            }
        }
    }

    let mut sorted_keys: Vec<String> = pairs.keys().cloned().collect();
    sorted_keys.sort();
    for base in sorted_keys {
        if let Some((Some(light), Some(dark))) = pairs.get(&base) {
            items.push(WallpaperItem {
                title: friendly_stem_title(&base),
                kind: WallpaperKind::DynamicPair {
                    light: light.clone(),
                    dark: dark.clone(),
                },
            });
        }
    }
    items
}

fn discover_user_photos(cfg: &WallpaperConfig) -> Vec<WallpaperItem> {
    let mut items = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    for raw in &cfg.custom_photos {
        let p = PathBuf::from(raw);
        if p.is_file() && seen.insert(p.clone()) {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Foto");
            items.push(WallpaperItem {
                title: friendly_stem_title(stem),
                kind: WallpaperKind::StaticImage(p),
            });
        }
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    for rel in [
        "Imágenes/Wallpapers",
        "Imágenes/Wallpaper",
        "Pictures/Wallpapers",
        "Pictures/Wallpaper",
    ] {
        let dir = PathBuf::from(&home).join(rel);
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    if p.file_name().and_then(|n| n.to_str()) == Some(DEFAULT_WALLPAPER_FILENAME) {
                        return false;
                    }
                    let ext = p
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    p.is_file() && matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "svg")
                })
                .collect();
            files.sort();
            for p in files {
                if seen.insert(p.clone()) {
                    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Foto");
                    items.push(WallpaperItem {
                        title: friendly_stem_title(stem),
                        kind: WallpaperKind::StaticImage(p),
                    });
                }
            }
        }
    }

    items
}

fn discover_system_images() -> Vec<WallpaperItem> {
    let mut items = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    if let Some(default_wp) = default_bundled_wallpaper_path() {
        seen.insert(default_wp.clone());
        items.push(WallpaperItem {
            title: "Playa de Pacasmayo".to_string(),
            kind: WallpaperKind::StaticImage(default_wp),
        });
    }

    for dir in &[
        "/usr/share/backgrounds/fedora-workstation",
        "/usr/share/backgrounds/gnome",
        "/usr/share/backgrounds/f45",
    ] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    if !p.is_file() {
                        return false;
                    }
                    let ext = p
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    matches!(ext.as_str(), "webp" | "jpg" | "jpeg" | "png")
                })
                .collect();
            files.sort();
            for p in files {
                if !seen.insert(p.clone()) {
                    continue;
                }
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Fondo");
                let clean = stem
                    .trim_end_matches("_light")
                    .trim_end_matches("_dark")
                    .trim_end_matches("-l")
                    .trim_end_matches("-d");
                let suffix = if stem.ends_with("_dark") || stem.ends_with("-d") {
                    " (Obscuro)"
                } else if stem.ends_with("_light") || stem.ends_with("-l") {
                    " (Claro)"
                } else {
                    ""
                };
                items.push(WallpaperItem {
                    title: format!("{}{}", friendly_stem_title(clean), suffix),
                    kind: WallpaperKind::StaticImage(p),
                });
            }
        }
    }

    items
}

fn draw_aurora_gradient_in_context(
    cr: &gtk4::cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    is_dark: bool,
) {
    let cx = x + w * 0.35;
    let cy = y + h * 0.25;
    let radius = (w * 0.8).hypot(h * 0.8).max(1.0);
    let grad = gtk4::cairo::RadialGradient::new(cx, cy, 0.0, cx, cy, radius);
    if is_dark {
        grad.add_color_stop_rgba(0.0, 0x2e as f64 / 255.0, 0x10 as f64 / 255.0, 0x65 as f64 / 255.0, 1.0);
        grad.add_color_stop_rgba(0.35, 0x17 as f64 / 255.0, 0x25 as f64 / 255.0, 0x54 as f64 / 255.0, 1.0);
        grad.add_color_stop_rgba(0.70, 0x0f as f64 / 255.0, 0x17 as f64 / 255.0, 0x2a as f64 / 255.0, 1.0);
        grad.add_color_stop_rgba(1.0, 0x0d as f64 / 255.0, 0x15 as f64 / 255.0, 0x26 as f64 / 255.0, 1.0);
    } else {
        grad.add_color_stop_rgba(0.0, 0x7c as f64 / 255.0, 0xb8 as f64 / 255.0, 0xff as f64 / 255.0, 1.0);
        grad.add_color_stop_rgba(0.38, 0x4f as f64 / 255.0, 0x8a as f64 / 255.0, 0xe8 as f64 / 255.0, 1.0);
        grad.add_color_stop_rgba(0.72, 0x5a as f64 / 255.0, 0x67 as f64 / 255.0, 0xd8 as f64 / 255.0, 1.0);
        grad.add_color_stop_rgba(1.0, 0x2d as f64 / 255.0, 0x3a as f64 / 255.0, 0x8c as f64 / 255.0, 1.0);
    }
    let _ = cr.set_source(&grad);
    cr.rectangle(x, y, w, h);
    let _ = cr.fill();
}

struct DecodedThumb {
    key: (PathBuf, i32),
    width: i32,
    height: i32,
    rowstride: i32,
    has_alpha: bool,
    pixels: Option<Vec<u8>>,
}

thread_local! {
    static THUMB_PIXBUF_CACHE: RefCell<HashMap<(PathBuf, i32), Option<gdk_pixbuf::Pixbuf>>> =
        RefCell::new(HashMap::new());
    static THUMB_WAITERS: RefCell<HashMap<(PathBuf, i32), Vec<gtk4::glib::WeakRef<DrawingArea>>>> =
        RefCell::new(HashMap::new());
    static THUMB_REQ_TX: RefCell<Option<std::sync::mpsc::Sender<(PathBuf, i32, i32)>>> =
        const { RefCell::new(None) };
}

fn disk_thumb_cache_path(path: &Path, req_w: i32) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in path.to_string_lossy().as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    for v in [meta.len(), mtime, req_w as u64] {
        for b in v.to_le_bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    let cache_root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".cache")
        });
    Some(
        cache_root
            .join("malus-de")
            .join("wallpaper-thumbs")
            .join(format!("{hash:016x}.png")),
    )
}

fn decode_thumbnail_off_main(path: &Path, req_w: i32, req_h: i32) -> Option<gdk_pixbuf::Pixbuf> {
    if let Some(cache_path) = disk_thumb_cache_path(path, req_w) {
        if cache_path.is_file() {
            if let Ok(pb) = gdk_pixbuf::Pixbuf::from_file(&cache_path) {
                return Some(pb);
            }
        }
        let loaded = gdk_pixbuf::Pixbuf::from_file_at_scale(path, req_w, req_h, true).ok()?;
        let pb = loaded.apply_embedded_orientation().unwrap_or(loaded);
        if let Some(parent) = cache_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = pb.savev(&cache_path, "png", &[]);
        return Some(pb);
    }
    let loaded = gdk_pixbuf::Pixbuf::from_file_at_scale(path, req_w, req_h, true).ok()?;
    Some(loaded.apply_embedded_orientation().unwrap_or(loaded))
}

fn ensure_thumb_worker() -> std::sync::mpsc::Sender<(PathBuf, i32, i32)> {
    THUMB_REQ_TX.with(|cell| {
        if let Some(ref tx) = *cell.borrow() {
            return tx.clone();
        }
        let (req_tx, req_rx) = std::sync::mpsc::channel::<(PathBuf, i32, i32)>();
        let (res_tx, res_rx) = async_channel::unbounded::<DecodedThumb>();

        // Pool ligero de 2 hilos para decodificar miniaturas en paralelo fuera del hilo GTK
        let shared_rx = std::sync::Arc::new(std::sync::Mutex::new(req_rx));
        for _ in 0..2 {
            let rx_c = shared_rx.clone();
            let tx_c = res_tx.clone();
            std::thread::spawn(move || {
                while let Ok(item) = {
                    let guard = rx_c.lock().ok();
                    guard.and_then(|g| g.recv().ok()).ok_or(())
                } {
                    let (path, req_w, req_h) = item;
                    let decoded = decode_thumbnail_off_main(&path, req_w, req_h).map(|pb| {
                        let width = pb.width();
                        let height = pb.height();
                        let rowstride = pb.rowstride();
                        let has_alpha = pb.has_alpha();
                        let pixels = pb.read_pixel_bytes().as_ref().to_vec();
                        (width, height, rowstride, has_alpha, pixels)
                    });
                    let msg = match decoded {
                        Some((width, height, rowstride, has_alpha, pixels)) => DecodedThumb {
                            key: (path, req_w),
                            width,
                            height,
                            rowstride,
                            has_alpha,
                            pixels: Some(pixels),
                        },
                        None => DecodedThumb {
                            key: (path, req_w),
                            width: 0,
                            height: 0,
                            rowstride: 0,
                            has_alpha: false,
                            pixels: None,
                        },
                    };
                    let _ = tx_c.send_blocking(msg);
                }
            });
        }

        gtk4::glib::spawn_future_local(async move {
            while let Ok(msg) = res_rx.recv().await {
                let pb = msg.pixels.map(|bytes| {
                    gdk_pixbuf::Pixbuf::from_bytes(
                        &gtk4::glib::Bytes::from_owned(bytes),
                        gdk_pixbuf::Colorspace::Rgb,
                        msg.has_alpha,
                        8,
                        msg.width,
                        msg.height,
                        msg.rowstride,
                    )
                });
                THUMB_PIXBUF_CACHE.with(|cache| {
                    cache.borrow_mut().insert(msg.key.clone(), pb);
                });
                let waiters = THUMB_WAITERS.with(|w| w.borrow_mut().remove(&msg.key));
                if let Some(list) = waiters {
                    for weak in list {
                        if let Some(area) = weak.upgrade() {
                            area.queue_draw();
                        }
                    }
                }
            }
        });

        *cell.borrow_mut() = Some(req_tx.clone());
        req_tx
    })
}

fn get_or_request_thumbnail_pixbuf(
    path: &Path,
    target_w: i32,
    target_h: i32,
    area: &DrawingArea,
) -> Option<gdk_pixbuf::Pixbuf> {
    let req_w = target_w.max(64) * 2;
    let req_h = target_h.max(64) * 2;
    let key = (path.to_path_buf(), req_w);

    if let Some(cached) = THUMB_PIXBUF_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return cached;
    }

    // Si la miniatura ya está precalculada en disco (~0.3 ms), cargarla directo sin esperar
    if let Some(cache_path) = disk_thumb_cache_path(path, req_w) {
        if cache_path.is_file() {
            if let Ok(pb) = gdk_pixbuf::Pixbuf::from_file(&cache_path) {
                THUMB_PIXBUF_CACHE.with(|c| {
                    c.borrow_mut().insert(key, Some(pb.clone()));
                });
                return Some(pb);
            }
        }
    }

    let should_enqueue = THUMB_WAITERS.with(|w| {
        let mut map = w.borrow_mut();
        let entry = map.entry(key.clone()).or_default();
        let is_first = entry.is_empty();
        entry.push(area.downgrade());
        is_first
    });

    if should_enqueue {
        let tx = ensure_thumb_worker();
        let _ = tx.send((path.to_path_buf(), req_w, req_h));
    }
    None
}

fn draw_pixbuf_cover(
    cr: &gtk4::cairo::Context,
    pb: &gdk_pixbuf::Pixbuf,
    w: f64,
    h: f64,
) {
    let sw = pb.width().max(1) as f64;
    let sh = pb.height().max(1) as f64;
    let scale = (w / sw).max(h / sh);
    let draw_w = sw * scale;
    let draw_h = sh * scale;
    let ox = (w - draw_w) * 0.5;
    let oy = (h - draw_h) * 0.5;
    let _ = cr.save();
    cr.translate(ox, oy);
    cr.scale(scale, scale);
    gtk4::cairo::Context::set_source_pixbuf(cr, pb, 0.0, 0.0);
    let _ = cr.paint();
    let _ = cr.restore();
}

fn draw_pixbuf_with_fit(
    cr: &gtk4::cairo::Context,
    pb: &gdk_pixbuf::Pixbuf,
    w: f64,
    h: f64,
    fit: WallpaperFitMode,
) {
    cr.set_source_rgb(0.05, 0.08, 0.14);
    let _ = cr.paint();

    let sw = pb.width().max(1) as f64;
    let sh = pb.height().max(1) as f64;
    let _ = cr.save();
    match fit {
        WallpaperFitMode::Fill => {
            let scale = (w / sw).max(h / sh);
            cr.translate((w - sw * scale) * 0.5, (h - sh * scale) * 0.5);
            cr.scale(scale, scale);
        }
        WallpaperFitMode::Fit => {
            let scale = (w / sw).min(h / sh);
            cr.translate((w - sw * scale) * 0.5, (h - sh * scale) * 0.5);
            cr.scale(scale, scale);
        }
        WallpaperFitMode::Stretch => {
            cr.scale(w / sw, h / sh);
        }
        WallpaperFitMode::Center => {
            let scale = (w / sw).min(h / sh) * 0.68;
            cr.translate((w - sw * scale) * 0.5, (h - sh * scale) * 0.5);
            cr.scale(scale, scale);
        }
    }
    gtk4::cairo::Context::set_source_pixbuf(cr, pb, 0.0, 0.0);
    let _ = cr.paint();
    let _ = cr.restore();
}

fn draw_dynamic_badge_icon(cr: &gtk4::cairo::Context, w: f64, h: f64) {
    let cx = 12.0;
    let cy = h - 12.0;
    let r = 6.2;
    let _ = cr.save();
    cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
    let _ = cr.fill();

    cr.arc(cx, cy, r - 0.8, 0.0, std::f64::consts::TAU);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.92);
    cr.set_line_width(1.2);
    let _ = cr.stroke();

    cr.arc(
        cx,
        cy,
        r - 1.4,
        -std::f64::consts::FRAC_PI_2,
        std::f64::consts::FRAC_PI_2,
    );
    cr.close_path();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.92);
    let _ = cr.fill();
    let _ = cr.restore();
    let _ = w;
}

fn update_show_all_label(btn: &Button, expanded: bool, count: usize) {
    let label = if expanded {
        "Mostrar menos".to_string()
    } else {
        format!("Mostrar todo ({})", count)
    };
    btn.set_label(&label);
}

const WP_CONTENT_LEFT_MARGIN: i32 = crate::SETTINGS_SIDEBAR_WIDTH + 22;
const WP_CONTENT_RIGHT_MARGIN: i32 = 22;

fn align_content_block<W: IsA<gtk4::Widget>>(w: W) -> W {
    w.set_margin_start(WP_CONTENT_LEFT_MARGIN);
    w.set_margin_end(WP_CONTENT_RIGHT_MARGIN);
    w
}

struct CarouselSection {
    root: GtkBox,
    slide_overlay: Overlay,
    hbox: GtkBox,
    flow: FlowBox,
}

fn animate_horizontal_scroll(hadj: gtk4::Adjustment, delta: f64) {
    let start = hadj.value();
    let max_val = (hadj.upper() - hadj.page_size()).max(0.0);
    let target = (start + delta).clamp(0.0, max_val);
    if (target - start).abs() < 1.0 {
        return;
    }
    let start_us = gtk4::glib::monotonic_time();
    let duration_us: f64 = 220_000.0;
    gtk4::glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        let elapsed = (gtk4::glib::monotonic_time() - start_us) as f64;
        let t = (elapsed / duration_us).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        hadj.set_value(start + (target - start) * eased);
        if t >= 1.0 {
            gtk4::glib::ControlFlow::Break
        } else {
            gtk4::glib::ControlFlow::Continue
        }
    });
}

impl CarouselSection {
    fn new(max_per_line: u32, min_per_line: u32) -> Self {
        let root = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .hexpand(true)
            .build();

        let hbox = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .margin_start(WP_CONTENT_LEFT_MARGIN)
            .margin_end(WP_CONTENT_RIGHT_MARGIN)
            .valign(Align::Start)
            .build();

        let hscroll = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::External)
            .vscrollbar_policy(PolicyType::Never)
            .kinetic_scrolling(true)
            .propagate_natural_width(false)
            .propagate_natural_height(true)
            .hexpand(true)
            .child(&hbox)
            .build();

        let slide_overlay = Overlay::builder()
            .hexpand(true)
            .child(&hscroll)
            .build();

        let prev_btn = Button::builder()
            .icon_name("go-previous-symbolic")
            .css_classes(["wp-slide-nav-btn"])
            .has_frame(false)
            .halign(Align::Start)
            .valign(Align::Center)
            .margin_start(WP_CONTENT_LEFT_MARGIN + 6)
            .visible(false)
            .build();
        let next_btn = Button::builder()
            .icon_name("go-next-symbolic")
            .css_classes(["wp-slide-nav-btn"])
            .has_frame(false)
            .halign(Align::End)
            .valign(Align::Center)
            .margin_end(10)
            .visible(false)
            .build();

        slide_overlay.add_overlay(&prev_btn);
        slide_overlay.add_overlay(&next_btn);

        let hovered = Rc::new(Cell::new(false));
        let hadj = hscroll.hadjustment();

        let sync_arrows: Rc<dyn Fn()> = {
            let hadj_c = hadj.clone();
            let hov_c = hovered.clone();
            let prev_c = prev_btn.clone();
            let next_c = next_btn.clone();
            Rc::new(move || {
                let is_hov = hov_c.get();
                let val = hadj_c.value();
                let upper = hadj_c.upper();
                let page = hadj_c.page_size();
                prev_c.set_visible(is_hov && val > 2.0);
                next_c.set_visible(is_hov && page > 1.0 && val + page < upper - 2.0);
            })
        };

        let motion = EventControllerMotion::new();
        {
            let hov_c = hovered.clone();
            let sync_c = sync_arrows.clone();
            motion.connect_enter(move |_, _, _| {
                hov_c.set(true);
                sync_c();
            });
        }
        {
            let hov_c = hovered.clone();
            let sync_c = sync_arrows.clone();
            motion.connect_leave(move |_| {
                hov_c.set(false);
                sync_c();
            });
        }
        slide_overlay.add_controller(motion);

        {
            let sync_c = sync_arrows.clone();
            hadj.connect_value_changed(move |_| sync_c());
        }
        {
            let sync_c = sync_arrows;
            hadj.connect_changed(move |_| sync_c());
        }

        {
            let hadj_p = hadj.clone();
            prev_btn.connect_clicked(move |_| animate_horizontal_scroll(hadj_p.clone(), -280.0));
        }
        {
            let hadj_n = hadj;
            next_btn.connect_clicked(move |_| animate_horizontal_scroll(hadj_n.clone(), 280.0));
        }

        let flow = align_content_block(
            FlowBox::builder()
                .valign(Align::Start)
                .max_children_per_line(max_per_line)
                .min_children_per_line(min_per_line)
                .selection_mode(gtk4::SelectionMode::None)
                .row_spacing(10)
                .column_spacing(10)
                .homogeneous(false)
                .visible(false)
                .build(),
        );

        root.append(&slide_overlay);
        root.append(&flow);

        Self {
            root,
            slide_overlay,
            hbox,
            flow,
        }
    }

    fn clear(&self, detach_widgets: &[&gtk4::Widget]) {
        for w in detach_widgets {
            if let Some(parent) = w.parent() {
                if let Some(fb_child) = parent.downcast_ref::<gtk4::FlowBoxChild>() {
                    fb_child.set_child(Option::<&gtk4::Widget>::None);
                } else if let Some(box_parent) = parent.downcast_ref::<GtkBox>() {
                    box_parent.remove(*w);
                }
            }
        }
        while let Some(child) = self.hbox.first_child() {
            self.hbox.remove(&child);
        }
        while let Some(child) = self.flow.first_child() {
            self.flow.remove(&child);
        }
    }

    fn populate_widgets(&self, expanded: bool, widgets: &[gtk4::Widget]) {
        self.slide_overlay.set_visible(!expanded);
        self.flow.set_visible(expanded);
        if expanded {
            for w in widgets {
                self.flow.append(w);
            }
        } else {
            for w in widgets {
                self.hbox.append(w);
            }
        }
    }
}

fn build_section_header(title: &str, right_widget: Option<&impl IsA<gtk4::Widget>>) -> GtkBox {
    let header = align_content_block(
        GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .margin_top(6)
            .build(),
    );
    let lbl = Label::builder()
        .label(title)
        .css_classes(["wp-section-title"])
        .xalign(0.0)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    header.append(&lbl);
    if let Some(rw) = right_widget {
        header.append(rw);
    }
    header
}

fn make_thumbnail_tile(
    item: &WallpaperItem,
    is_active_fn: Rc<dyn Fn(&WallpaperItem) -> bool>,
    on_select: Rc<dyn Fn(&WallpaperItem)>,
) -> (Button, Rc<dyn Fn()>) {
    let btn = Button::builder()
        .css_classes(["wp-tile-btn"])
        .has_frame(false)
        .build();

    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(5)
        .width_request(116)
        .build();

    let area = DrawingArea::builder()
        .content_width(114)
        .content_height(70)
        .css_classes(["wp-thumb-area"])
        .build();

    let is_dynamic = matches!(
        item.kind,
        WallpaperKind::AuroraGradient | WallpaperKind::DynamicPair { .. }
    );

    let item_for_draw = item.clone();
    let is_active_draw = is_active_fn.clone();
    area.set_draw_func(move |area_self, cr, width, height| {
        let w = width as f64;
        let h = height as f64;
        if w <= 2.0 || h <= 2.0 {
            return;
        }
        let _ = cr.save();
        squircle_path(cr, w, h, 8.0);
        cr.clip();

        match &item_for_draw.kind {
            WallpaperKind::AuroraGradient => {
                let _ = cr.save();
                cr.rectangle(0.0, 0.0, w * 0.5, h);
                cr.clip();
                draw_aurora_gradient_in_context(cr, 0.0, 0.0, w, h, false);
                let _ = cr.restore();

                let _ = cr.save();
                cr.rectangle(w * 0.5, 0.0, w * 0.5, h);
                cr.clip();
                draw_aurora_gradient_in_context(cr, 0.0, 0.0, w, h, true);
                let _ = cr.restore();
            }
            WallpaperKind::DynamicPair { light, dark } => {
                let light_pb = get_or_request_thumbnail_pixbuf(light, 114, 70, area_self);
                let dark_pb = get_or_request_thumbnail_pixbuf(dark, 114, 70, area_self);
                cr.set_source_rgba(0.18, 0.21, 0.27, 0.45);
                let _ = cr.paint();
                if let Some(ref lpb) = light_pb {
                    let _ = cr.save();
                    cr.rectangle(0.0, 0.0, w * 0.5, h);
                    cr.clip();
                    draw_pixbuf_cover(cr, lpb, w, h);
                    let _ = cr.restore();
                }
                if let Some(ref dpb) = dark_pb {
                    let _ = cr.save();
                    cr.rectangle(w * 0.5, 0.0, w * 0.5, h);
                    cr.clip();
                    draw_pixbuf_cover(cr, dpb, w, h);
                    let _ = cr.restore();
                }
            }
            WallpaperKind::StaticImage(p) => {
                if let Some(pb) = get_or_request_thumbnail_pixbuf(p, 114, 70, area_self) {
                    draw_pixbuf_cover(cr, &pb, w, h);
                } else {
                    cr.set_source_rgba(0.18, 0.21, 0.27, 0.45);
                    let _ = cr.paint();
                }
            }
        }

        if is_dynamic {
            draw_dynamic_badge_icon(cr, w, h);
        }
        let _ = cr.restore();

        // Borde / anillo activo
        let active = is_active_draw(&item_for_draw);
        let _ = cr.save();
        if active {
            squircle_path(cr, w - 2.0, h - 2.0, 7.5);
            let _ = cr.restore();
            let _ = cr.save();
            cr.translate(1.0, 1.0);
            squircle_path(cr, w - 2.0, h - 2.0, 7.5);
            cr.set_source_rgba(0.04, 0.52, 1.0, 0.96);
            cr.set_line_width(2.2);
            let _ = cr.stroke();
        } else {
            cr.translate(0.5, 0.5);
            squircle_path(cr, w - 1.0, h - 1.0, 8.0);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.14);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        }
        let _ = cr.restore();
    });

    let title_lbl = Label::builder()
        .label(&item.title)
        .css_classes(["wp-tile-label"])
        .xalign(0.5)
        .max_width_chars(16)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();

    vbox.append(&area);
    vbox.append(&title_lbl);
    btn.set_child(Some(&vbox));

    let item_click = item.clone();
    btn.connect_clicked(move |_| {
        on_select(&item_click);
    });

    let area_ref = area.clone();
    let btn_ref = btn.clone();
    let title_ref = title_lbl.clone();
    let item_sync = item.clone();
    let is_active_sync = is_active_fn;
    let refresh_tile: Rc<dyn Fn()> = Rc::new(move || {
        let active = is_active_sync(&item_sync);
        if active {
            btn_ref.add_css_class("wp-tile-btn-active");
            title_ref.add_css_class("app-thumb-label-active");
        } else {
            btn_ref.remove_css_class("wp-tile-btn-active");
            title_ref.remove_css_class("app-thumb-label-active");
        }
        area_ref.queue_draw();
    });

    (btn, refresh_tile)
}

fn is_item_active(item: &WallpaperItem, cfg: &WallpaperConfig) -> bool {
    match (&item.kind, cfg.mode) {
        (WallpaperKind::AuroraGradient, WallpaperMode::Gradient) => true,
        (
            WallpaperKind::DynamicPair { light, dark },
            WallpaperMode::DynamicPair,
        ) => {
            cfg.light_image_path.as_deref() == light.to_str()
                && cfg.dark_image_path.as_deref() == dark.to_str()
        }
        (WallpaperKind::StaticImage(path), WallpaperMode::Image) => {
            if let Some(ref active) = cfg.image_path {
                Path::new(active) == path
            } else {
                default_bundled_wallpaper_path().as_deref() == Some(path.as_path())
            }
        }
        _ => false,
    }
}

fn active_wallpaper_display_title(cfg: &WallpaperConfig) -> String {
    if let Some(ref title) = cfg.active_title {
        if !title.trim().is_empty() {
            return title.clone();
        }
    }
    match cfg.mode {
        WallpaperMode::Gradient => "Degradado Aurora".to_string(),
        WallpaperMode::SolidColor => {
            let norm = normalize_wallpaper_hex_color(&cfg.color_hex)
                .unwrap_or_else(|| cfg.color_hex.clone());
            let named = CURATED_SOLID_COLORS
                .iter()
                .find(|(_, hex)| hex.eq_ignore_ascii_case(&norm))
                .map(|(name, _)| (*name).to_string());
            named.unwrap_or_else(|| format!("Color sólido ({})", norm))
        }
        WallpaperMode::DynamicPair => cfg
            .light_image_path
            .as_deref()
            .and_then(|p| Path::new(p).file_stem()?.to_str())
            .map(|s| friendly_stem_title(s.trim_end_matches("_light").trim_end_matches("-l")))
            .unwrap_or_else(|| "Fondo dinámico".to_string()),
        WallpaperMode::Image => cfg
            .image_path
            .as_deref()
            .and_then(|p| Path::new(p).file_stem()?.to_str())
            .map(|s| {
                if s.contains("pexels-jbarbozameca-41027443") {
                    "Playa de Pacasmayo".to_string()
                } else {
                    friendly_stem_title(s)
                }
            })
            .unwrap_or_else(|| "Playa de Pacasmayo".to_string()),
    }
}

fn resolve_active_image_for_preview(cfg: &WallpaperConfig) -> Option<PathBuf> {
    let is_dark = is_dark_mode_active();
    match cfg.mode {
        WallpaperMode::DynamicPair => {
            let chosen = if is_dark {
                cfg.dark_image_path.as_deref()
            } else {
                cfg.light_image_path.as_deref()
            };
            chosen.map(PathBuf::from).filter(|p| p.is_file())
        }
        WallpaperMode::Image => cfg
            .image_path
            .as_deref()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(default_bundled_wallpaper_path),
        _ => None,
    }
}

fn populate_carousel_tiles(
    carousel: &CarouselSection,
    items: &[WallpaperItem],
    expanded: bool,
    leading_widget: Option<&gtk4::Widget>,
    cfg_state: &Rc<RefCell<WallpaperConfig>>,
    on_select: &Rc<dyn Fn(&WallpaperItem)>,
    tile_refreshers: &Rc<RefCell<Vec<Rc<dyn Fn()>>>>,
) {
    if let Some(lead) = leading_widget {
        carousel.clear(&[lead]);
    } else {
        carousel.clear(&[]);
    }

    let mut widgets: Vec<gtk4::Widget> = Vec::with_capacity(items.len() + 1);
    if let Some(lead) = leading_widget {
        widgets.push(lead.clone());
    }

    for item in items {
        let cfg_check = cfg_state.clone();
        let is_active_fn: Rc<dyn Fn(&WallpaperItem) -> bool> =
            Rc::new(move |it| is_item_active(it, &cfg_check.borrow()));
        let (btn, refresh_fn) = make_thumbnail_tile(item, is_active_fn, on_select.clone());
        refresh_fn();
        tile_refreshers.borrow_mut().push(refresh_fn);
        widgets.push(btn.upcast());
    }

    carousel.populate_widgets(expanded, &widgets);
}

pub fn create_wallpaper_pane(window: &ApplicationWindow) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .css_classes(["wp-bleed-pane"])
        .hexpand(true)
        .vexpand(true)
        .build();

    let pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["wp-bleed-content-pane"])
        .spacing(12)
        .valign(Align::Start)
        .vexpand(false)
        .build();

    let cfg_state = Rc::new(RefCell::new(WallpaperConfig::load()));
    let syncing_ui = Rc::new(Cell::new(false));
    let tile_refreshers: Rc<RefCell<Vec<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(Vec::new()));

    // ========================================================================
    // 1. Tarjeta Superior (Preview + Título + Encuadre + Spaces + Acciones)
    // ========================================================================
    let hero_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(14)
        .css_classes(["wp-hero-box"])
        .build();

    let hero_preview = DrawingArea::builder()
        .content_width(164)
        .content_height(100)
        .valign(Align::Center)
        .build();

    {
        let cfg_draw = cfg_state.clone();
        hero_preview.set_draw_func(move |area_self, cr, width, height| {
            let w = width as f64;
            let h = height as f64;
            if w <= 2.0 || h <= 2.0 {
                return;
            }
            let cfg = cfg_draw.borrow().clone();
            let _ = cr.save();
            squircle_path(cr, w, h, 10.0);
            cr.clip();

            match cfg.mode {
                WallpaperMode::SolidColor => {
                    let (r, g, b) =
                        parse_wallpaper_hex_color(&cfg.color_hex).unwrap_or((0x4D, 0xA6, 0xC2));
                    cr.set_source_rgb(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
                    let _ = cr.paint();
                }
                WallpaperMode::Gradient => {
                    draw_aurora_gradient_in_context(cr, 0.0, 0.0, w, h, is_dark_mode_active());
                }
                WallpaperMode::Image | WallpaperMode::DynamicPair => {
                    if let Some(img_path) = resolve_active_image_for_preview(&cfg) {
                        if let Some(pb) =
                            get_or_request_thumbnail_pixbuf(&img_path, 164, 100, area_self)
                        {
                            draw_pixbuf_with_fit(cr, &pb, w, h, cfg.fit_mode);
                        } else {
                            draw_aurora_gradient_in_context(
                                cr,
                                0.0,
                                0.0,
                                w,
                                h,
                                is_dark_mode_active(),
                            );
                        }
                    } else {
                        draw_aurora_gradient_in_context(
                            cr,
                            0.0,
                            0.0,
                            w,
                            h,
                            is_dark_mode_active(),
                        );
                    }
                }
            }
            let _ = cr.restore();

            let _ = cr.save();
            cr.translate(0.5, 0.5);
            squircle_path(cr, w - 1.0, h - 1.0, 10.0);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.16);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            let _ = cr.restore();
        });
    }
    hero_row.append(&hero_preview);

    let hero_right = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .hexpand(true)
        .valign(Align::Center)
        .build();

    let hero_title_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();

    let hero_title_lbl = Label::builder()
        .label("Playa de Pacasmayo")
        .css_classes(["wp-hero-title"])
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .valign(Align::Center)
        .build();
    hero_title_row.append(&hero_title_lbl);

    let fit_model = StringList::new(&FIT_MODE_LABELS);
    let fit_dropdown = DropDown::builder()
        .model(&fit_model)
        .selected(cfg_state.borrow().fit_mode.as_index() as u32)
        .css_classes(["wp-fit-dropdown"])
        .valign(Align::Center)
        .build();
    hero_title_row.append(&fit_dropdown);
    hero_right.append(&hero_title_row);

    // Sub-fila dentro de tarjeta blanca estilo macOS: "Mostrar en todos los Spaces"
    let spaces_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .css_classes(["wp-spaces-row"])
        .build();
    spaces_row.append(
        &Label::builder()
            .label("Mostrar en todos los Spaces")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    spaces_row.append(&make_pending_badge());
    let spaces_switch = Switch::new(true);
    spaces_switch.set_sensitive(false);
    spaces_switch.set_valign(Align::Center);
    spaces_row.append(spaces_switch.widget());
    hero_right.append(&wrap_squircle_card(
        &spaces_row,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // Botones de acciones inferiores del hero ("Protector de pantalla..." y "Aspecto del reloj...")
    let hero_actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .halign(Align::End)
        .build();

    for btn_title in ["Protector de pantalla...", "Aspecto del reloj..."] {
        let holder = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .valign(Align::Center)
            .build();
        let (wrapped, btn) = make_squircle_button(btn_title, false, Some("Por implementar"));
        btn.set_sensitive(false);
        holder.append(&wrapped);
        holder.append(&make_pending_badge());
        hero_actions.append(&holder);
    }
    hero_right.append(&hero_actions);

    hero_row.append(&hero_right);
    pane.append(&align_content_block(wrap_squircle_card(
        &hero_row,
        WIFI_CARD_SQUIRCLE_RADIUS,
    )));

    // ========================================================================
    // 2. Fondos de pantalla dinámicos
    // ========================================================================
    let dyn_items = Rc::new(RefCell::new(discover_dynamic_wallpapers()));
    let dyn_expanded = Rc::new(Cell::new(false));
    let dyn_show_btn = Button::builder()
        .label(&format!("Mostrar todo ({})", dyn_items.borrow().len()))
        .css_classes(["wp-show-all-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .build();
    dyn_show_btn.set_visible(dyn_items.borrow().len() > 5);

    pane.append(&build_section_header(
        "Fondos de pantalla dinámicos",
        Some(&dyn_show_btn),
    ));

    let dyn_carousel = Rc::new(CarouselSection::new(5, 2));
    pane.append(&dyn_carousel.root);

    // ========================================================================
    // 3. Tus fotos ("Agregar foto..." + fotos añadidas/usuario)
    // ========================================================================
    let user_photos = Rc::new(RefCell::new(discover_user_photos(&cfg_state.borrow())));
    let photos_expanded = Rc::new(Cell::new(false));
    let photos_show_btn = Button::builder()
        .label(&format!("Mostrar todo ({})", user_photos.borrow().len()))
        .css_classes(["wp-show-all-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .build();
    photos_show_btn.set_visible(user_photos.borrow().len() > 4);

    pane.append(&build_section_header("Tus fotos", Some(&photos_show_btn)));

    let photos_carousel = Rc::new(CarouselSection::new(5, 2));
    pane.append(&photos_carousel.root);

    let add_photo_btn = Button::builder()
        .css_classes(["wp-tile-btn", "wp-add-photo-tile"])
        .has_frame(false)
        .build();
    let add_photo_inner = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .width_request(114)
        .height_request(70)
        .halign(Align::Center)
        .valign(Align::Center)
        .css_classes(["wp-add-photo-card"])
        .build();
    add_photo_inner.append(
        &Image::builder()
            .icon_name("insert-image-symbolic")
            .pixel_size(24)
            .css_classes(["wp-add-photo-icon"])
            .valign(Align::Center)
            .build(),
    );
    add_photo_inner.append(
        &Label::builder()
            .label("Agregar foto...")
            .css_classes(["wp-tile-label"])
            .valign(Align::Center)
            .build(),
    );
    add_photo_btn.set_child(Some(&add_photo_inner));

    // ========================================================================
    // 4-9. Colecciones de vistas aéreas / Mac sin contenido local ("por implementar")
    // ========================================================================
    for (sec_title, sec_desc) in PENDING_AERIAL_SECTIONS {
        let badge = make_pending_badge();
        pane.append(&build_section_header(sec_title, Some(&badge)));
        let pending_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .css_classes(["wp-pending-card"])
            .build();
        pending_box.append(
            &Image::builder()
                .icon_name("image-x-generic-symbolic")
                .pixel_size(18)
                .css_classes(["wp-pending-icon"])
                .valign(Align::Center)
                .build(),
        );
        pending_box.append(
            &Label::builder()
                .label(sec_desc)
                .css_classes(["card-section-sub"])
                .xalign(0.0)
                .hexpand(true)
                .wrap(true)
                .valign(Align::Center)
                .build(),
        );
        pane.append(&align_content_block(wrap_squircle_card(
            &pending_box,
            WIFI_CARD_SQUIRCLE_RADIUS,
        )));
    }

    // ========================================================================
    // 10. Imágenes
    // ========================================================================
    let image_items = Rc::new(RefCell::new(discover_system_images()));
    let images_expanded = Rc::new(Cell::new(false));
    let images_show_btn = Button::builder()
        .label(&format!("Mostrar todo ({})", image_items.borrow().len()))
        .css_classes(["wp-show-all-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .build();
    images_show_btn.set_visible(image_items.borrow().len() > 5);

    pane.append(&build_section_header("Imágenes", Some(&images_show_btn)));

    let images_carousel = Rc::new(CarouselSection::new(5, 2));
    pane.append(&images_carousel.root);

    // ========================================================================
    // 11. Colores (Botón rotar + '+' color personalizado + muestras circulares)
    // ========================================================================
    let colors_expanded = Rc::new(Cell::new(false));
    let colors_show_btn = Button::builder()
        .label(&format!("Mostrar todo ({})", CURATED_SOLID_COLORS.len()))
        .css_classes(["wp-show-all-btn"])
        .has_frame(false)
        .valign(Align::Center)
        .build();

    pane.append(&build_section_header("Colores", Some(&colors_show_btn)));

    let colors_carousel = Rc::new(CarouselSection::new(12, 6));
    pane.append(&colors_carousel.root);

    // ========================================================================
    // Estado compartido de refresco y callbacks
    // ========================================================================
    let sync_visuals_ref: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let rebuild_colors_ref: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let rebuild_photos_ref: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let apply_item: Rc<dyn Fn(&WallpaperItem)> = {
        let cfg_c = cfg_state.clone();
        let sync_ref = sync_visuals_ref.clone();
        Rc::new(move |item: &WallpaperItem| {
            {
                let mut cfg = cfg_c.borrow_mut();
                cfg.active_title = Some(item.title.clone());
                match &item.kind {
                    WallpaperKind::AuroraGradient => {
                        cfg.mode = WallpaperMode::Gradient;
                        cfg.image_path = None;
                        cfg.light_image_path = None;
                        cfg.dark_image_path = None;
                    }
                    WallpaperKind::DynamicPair { light, dark } => {
                        cfg.mode = WallpaperMode::DynamicPair;
                        cfg.light_image_path = light.to_str().map(str::to_string);
                        cfg.dark_image_path = dark.to_str().map(str::to_string);
                        cfg.image_path = None;
                    }
                    WallpaperKind::StaticImage(path) => {
                        cfg.mode = WallpaperMode::Image;
                        cfg.image_path = path.to_str().map(str::to_string);
                        cfg.light_image_path = None;
                        cfg.dark_image_path = None;
                    }
                }
                save_and_apply_wallpaper_config(&cfg);
            }
            if let Some(ref f) = *sync_ref.borrow() {
                f();
            }
        })
    };

    let apply_solid_color: Rc<dyn Fn(&str, Option<&str>, bool)> = {
        let cfg_c = cfg_state.clone();
        let sync_ref = sync_visuals_ref.clone();
        let rebuild_colors = rebuild_colors_ref.clone();
        Rc::new(move |hex: &str, title_opt: Option<&str>, add_custom: bool| {
            let norm = normalize_wallpaper_hex_color(hex).unwrap_or_else(|| "#4DA6C2".to_string());
            let mut added_new = false;
            {
                let mut cfg = cfg_c.borrow_mut();
                cfg.mode = WallpaperMode::SolidColor;
                cfg.color_hex = norm.clone();
                cfg.image_path = None;
                cfg.light_image_path = None;
                cfg.dark_image_path = None;
                let resolved_title = title_opt
                    .map(str::to_string)
                    .or_else(|| {
                        CURATED_SOLID_COLORS
                            .iter()
                            .find(|(_, h)| h.eq_ignore_ascii_case(&norm))
                            .map(|(n, _)| (*n).to_string())
                    })
                    .unwrap_or_else(|| format!("Color sólido ({})", norm));
                cfg.active_title = Some(resolved_title);
                if add_custom
                    && !CURATED_SOLID_COLORS
                        .iter()
                        .any(|(_, h)| h.eq_ignore_ascii_case(&norm))
                    && !cfg
                        .custom_colors
                        .iter()
                        .any(|h| h.eq_ignore_ascii_case(&norm))
                {
                    cfg.custom_colors.insert(0, norm);
                    added_new = true;
                }
                save_and_apply_wallpaper_config(&cfg);
            }
            if added_new {
                if let Some(ref rc) = *rebuild_colors.borrow() {
                    rc();
                }
            }
            if let Some(ref f) = *sync_ref.borrow() {
                f();
            }
        })
    };

    let rebuild_dynamic_grid: Rc<dyn Fn()> = {
        let carousel_c = dyn_carousel.clone();
        let items_c = dyn_items.clone();
        let exp_c = dyn_expanded.clone();
        let cfg_c = cfg_state.clone();
        let apply_c = apply_item.clone();
        let refreshers_c = tile_refreshers.clone();
        let btn_c = dyn_show_btn.clone();
        Rc::new(move || {
            let items = items_c.borrow();
            let expanded = exp_c.get();
            update_show_all_label(&btn_c, expanded, items.len());
            populate_carousel_tiles(
                &carousel_c,
                &items,
                expanded,
                None,
                &cfg_c,
                &apply_c,
                &refreshers_c,
            );
        })
    };

    let rebuild_photos_grid: Rc<dyn Fn()> = {
        let carousel_c = photos_carousel.clone();
        let items_c = user_photos.clone();
        let exp_c = photos_expanded.clone();
        let lead_c = add_photo_btn.clone();
        let cfg_c = cfg_state.clone();
        let apply_c = apply_item.clone();
        let refreshers_c = tile_refreshers.clone();
        let btn_c = photos_show_btn.clone();
        Rc::new(move || {
            let items = items_c.borrow();
            let expanded = exp_c.get();
            btn_c.set_visible(items.len() > 4);
            update_show_all_label(&btn_c, expanded, items.len());
            populate_carousel_tiles(
                &carousel_c,
                &items,
                expanded,
                Some(lead_c.upcast_ref()),
                &cfg_c,
                &apply_c,
                &refreshers_c,
            );
        })
    };
    *rebuild_photos_ref.borrow_mut() = Some(rebuild_photos_grid.clone());

    let rebuild_images_grid: Rc<dyn Fn()> = {
        let carousel_c = images_carousel.clone();
        let items_c = image_items.clone();
        let exp_c = images_expanded.clone();
        let cfg_c = cfg_state.clone();
        let apply_c = apply_item.clone();
        let refreshers_c = tile_refreshers.clone();
        let btn_c = images_show_btn.clone();
        Rc::new(move || {
            let items = items_c.borrow();
            let expanded = exp_c.get();
            update_show_all_label(&btn_c, expanded, items.len());
            populate_carousel_tiles(
                &carousel_c,
                &items,
                expanded,
                None,
                &cfg_c,
                &apply_c,
                &refreshers_c,
            );
        })
    };

    let color_swatch_areas: Rc<RefCell<Vec<(String, DrawingArea)>>> =
        Rc::new(RefCell::new(Vec::new()));

    let rebuild_colors_grid: Rc<dyn Fn()> = {
        let carousel_c = colors_carousel.clone();
        let exp_c = colors_expanded.clone();
        let cfg_c = cfg_state.clone();
        let apply_col = apply_solid_color.clone();
        let win_c = window.clone();
        let areas_c = color_swatch_areas.clone();
        let btn_c = colors_show_btn.clone();
        Rc::new(move || {
            carousel_c.clear(&[]);
            areas_c.borrow_mut().clear();

            let mut col_widgets: Vec<gtk4::Widget> = Vec::new();

            // Botón 1: Rotar / siguiente color
            let rotate_btn = Button::builder()
                .css_classes(["wp-color-circle-btn"])
                .has_frame(false)
                .tooltip_text("Elegir siguiente color")
                .valign(Align::Center)
                .child(
                    &Image::builder()
                        .icon_name("view-refresh-symbolic")
                        .pixel_size(16)
                        .build(),
                )
                .build();
            {
                let cfg_r = cfg_c.clone();
                let apply_r = apply_col.clone();
                rotate_btn.connect_clicked(move |_| {
                    let cur_hex = cfg_r.borrow().color_hex.clone();
                    let cur_idx = CURATED_SOLID_COLORS
                        .iter()
                        .position(|(_, h)| h.eq_ignore_ascii_case(&cur_hex));
                    let next_idx = match cur_idx {
                        Some(i) => (i + 1) % CURATED_SOLID_COLORS.len(),
                        None => 0,
                    };
                    let (name, hex) = CURATED_SOLID_COLORS[next_idx];
                    apply_r(hex, Some(name), false);
                });
            }
            col_widgets.push(rotate_btn.upcast());

            // Botón 2: '+' selector personalizado ColorDialog
            let add_color_btn = Button::builder()
                .css_classes(["wp-color-circle-btn"])
                .has_frame(false)
                .tooltip_text("Elegir color personalizado...")
                .valign(Align::Center)
                .child(
                    &Image::builder()
                        .icon_name("list-add-symbolic")
                        .pixel_size(16)
                        .build(),
                )
                .build();
            {
                let win_d = win_c.clone();
                let cfg_d = cfg_c.clone();
                let apply_d = apply_col.clone();
                add_color_btn.connect_clicked(move |_| {
                    let dialog = ColorDialog::builder()
                        .title("Elegir color de fondo")
                        .modal(true)
                        .with_alpha(false)
                        .build();
                    let (r, g, b) = parse_wallpaper_hex_color(&cfg_d.borrow().color_hex)
                        .unwrap_or((0x4D, 0xA6, 0xC2));
                    let initial = gtk4::gdk::RGBA::new(
                        r as f32 / 255.0,
                        g as f32 / 255.0,
                        b as f32 / 255.0,
                        1.0,
                    );
                    let apply_cb = apply_d.clone();
                    dialog.choose_rgba(
                        Some(&win_d),
                        Some(&initial),
                        gtk4::gio::Cancellable::NONE,
                        move |res| {
                            if let Ok(rgba) = res {
                                let hex = format!(
                                    "#{:02X}{:02X}{:02X}",
                                    (rgba.red().clamp(0.0, 1.0) * 255.0).round() as u8,
                                    (rgba.green().clamp(0.0, 1.0) * 255.0).round() as u8,
                                    (rgba.blue().clamp(0.0, 1.0) * 255.0).round() as u8
                                );
                                apply_cb(&hex, None, true);
                            }
                        },
                    );
                });
            }
            col_widgets.push(add_color_btn.upcast());

            // Lista de muestras: custom + curadas
            let mut swatches: Vec<(String, String)> = cfg_c
                .borrow()
                .custom_colors
                .iter()
                .filter_map(|hex| {
                    normalize_wallpaper_hex_color(hex)
                        .map(|h| (format!("Color ({})", h), h))
                })
                .collect();
            for (name, hex) in CURATED_SOLID_COLORS {
                swatches.push((name.to_string(), hex.to_string()));
            }

            let expanded = exp_c.get();
            update_show_all_label(&btn_c, expanded, swatches.len());

            for (name, hex) in swatches {
                let area = DrawingArea::builder()
                    .content_width(36)
                    .content_height(36)
                    .build();
                let hex_draw = hex.clone();
                let cfg_draw = cfg_c.clone();
                area.set_draw_func(move |_, cr, width, height| {
                    let cx = width as f64 * 0.5;
                    let cy = height as f64 * 0.5;
                    let outer_r = (cx.min(cy) - 2.0).max(6.0);
                    let cfg = cfg_draw.borrow();
                    let is_active = cfg.mode == WallpaperMode::SolidColor
                        && cfg.color_hex.eq_ignore_ascii_case(&hex_draw);
                    let (r, g, b) = parse_wallpaper_hex_color(&hex_draw).unwrap_or((80, 120, 180));

                    if is_active {
                        cr.arc(cx, cy, outer_r, 0.0, std::f64::consts::TAU);
                        cr.set_source_rgba(0.04, 0.52, 1.0, 0.95);
                        cr.set_line_width(2.2);
                        let _ = cr.stroke();
                    }

                    let fill_r = if is_active { outer_r - 3.2 } else { outer_r - 1.0 };
                    cr.arc(cx, cy, fill_r, 0.0, std::f64::consts::TAU);
                    cr.set_source_rgb(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
                    let _ = cr.fill();

                    cr.arc(cx, cy, fill_r, 0.0, std::f64::consts::TAU);
                    cr.set_source_rgba(0.0, 0.0, 0.0, 0.14);
                    cr.set_line_width(1.0);
                    let _ = cr.stroke();
                });

                let swatch_btn = Button::builder()
                    .css_classes(["wp-swatch-btn"])
                    .has_frame(false)
                    .tooltip_text(&name)
                    .valign(Align::Center)
                    .child(&area)
                    .build();

                let hex_click = hex.clone();
                let name_click = name.clone();
                let apply_click = apply_col.clone();
                swatch_btn.connect_clicked(move |_| {
                    apply_click(&hex_click, Some(&name_click), false);
                });

                areas_c.borrow_mut().push((hex, area));
                col_widgets.push(swatch_btn.upcast());
            }

            carousel_c.populate_widgets(expanded, &col_widgets);
        })
    };
    *rebuild_colors_ref.borrow_mut() = Some(rebuild_colors_grid.clone());

    let sync_visuals: Rc<dyn Fn()> = {
        let cfg_c = cfg_state.clone();
        let hero_prev_c = hero_preview.clone();
        let hero_title_c = hero_title_lbl.clone();
        let fit_c = fit_dropdown.clone();
        let syncing_c = syncing_ui.clone();
        let refreshers_c = tile_refreshers.clone();
        let swatches_c = color_swatch_areas.clone();
        Rc::new(move || {
            let cfg = cfg_c.borrow().clone();
            hero_title_c.set_label(&active_wallpaper_display_title(&cfg));
            let is_image_mode = matches!(
                cfg.mode,
                WallpaperMode::Image | WallpaperMode::DynamicPair
            );
            fit_c.set_sensitive(is_image_mode);
            syncing_c.set(true);
            fit_c.set_selected(cfg.fit_mode.as_index() as u32);
            syncing_c.set(false);
            hero_prev_c.queue_draw();

            for r in refreshers_c.borrow().iter() {
                r();
            }
            for (_, area) in swatches_c.borrow().iter() {
                area.queue_draw();
            }
        })
    };
    *sync_visuals_ref.borrow_mut() = Some(sync_visuals.clone());

    // Cambio de modo de encuadre (Llenar / Ajustar / Estirar / Centrar)
    {
        let cfg_c = cfg_state.clone();
        let syncing_c = syncing_ui.clone();
        let sync_c = sync_visuals.clone();
        fit_dropdown.connect_selected_notify(move |dd| {
            if syncing_c.get() {
                return;
            }
            let next_fit = WallpaperFitMode::from_index(dd.selected() as usize);
            {
                let mut cfg = cfg_c.borrow_mut();
                if cfg.fit_mode == next_fit {
                    return;
                }
                cfg.fit_mode = next_fit;
                save_and_apply_wallpaper_config(&cfg);
            }
            sync_c();
        });
    }

    // "Agregar foto..."
    {
        let win_c = window.clone();
        let cfg_c = cfg_state.clone();
        let photos_c = user_photos.clone();
        let rebuild_p = rebuild_photos_grid.clone();
        let sync_c = sync_visuals.clone();
        add_photo_btn.connect_clicked(move |_| {
            let filter = build_wallpaper_file_filter();
            let filters = gtk4::gio::ListStore::new::<FileFilter>();
            filters.append(&filter);

            let chooser = FileDialog::builder()
                .title("Seleccionar fondo de pantalla")
                .accept_label("Establecer Fondo")
                .modal(true)
                .filters(&filters)
                .default_filter(&filter)
                .build();

            if let Some(folder) = wallpaper_chooser_initial_folder() {
                chooser.set_initial_folder(Some(&gtk4::gio::File::for_path(folder)));
            }

            let cfg_sub = cfg_c.clone();
            let photos_sub = photos_c.clone();
            let rebuild_sub = rebuild_p.clone();
            let sync_sub = sync_c.clone();
            chooser.open(Some(&win_c), gtk4::gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        if let Some(path_str) = path.to_str().map(str::to_string) {
                            let stem = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Foto");
                            {
                                let mut cfg = cfg_sub.borrow_mut();
                                cfg.custom_photos.retain(|p| p != &path_str);
                                cfg.custom_photos.insert(0, path_str.clone());
                                cfg.mode = WallpaperMode::Image;
                                cfg.image_path = Some(path_str);
                                cfg.light_image_path = None;
                                cfg.dark_image_path = None;
                                cfg.active_title = Some(friendly_stem_title(stem));
                                save_and_apply_wallpaper_config(&cfg);
                                *photos_sub.borrow_mut() = discover_user_photos(&cfg);
                            }
                            rebuild_sub();
                            sync_sub();
                        }
                    }
                }
            });
        });
    }

    // Botones "Mostrar todo (N)"
    {
        let exp_c = dyn_expanded.clone();
        let rebuild_d = rebuild_dynamic_grid.clone();
        let rebuild_p = rebuild_photos_grid.clone();
        let rebuild_i = rebuild_images_grid.clone();
        let sync_c = sync_visuals.clone();
        let ref_c = tile_refreshers.clone();
        dyn_show_btn.connect_clicked(move |_| {
            exp_c.set(!exp_c.get());
            ref_c.borrow_mut().clear();
            rebuild_d();
            rebuild_p();
            rebuild_i();
            sync_c();
        });
    }
    {
        let exp_c = photos_expanded.clone();
        let rebuild_c = rebuild_photos_grid.clone();
        let rebuild_d = rebuild_dynamic_grid.clone();
        let rebuild_i = rebuild_images_grid.clone();
        let sync_c = sync_visuals.clone();
        let ref_c = tile_refreshers.clone();
        photos_show_btn.connect_clicked(move |_| {
            exp_c.set(!exp_c.get());
            ref_c.borrow_mut().clear();
            rebuild_d();
            rebuild_c();
            rebuild_i();
            sync_c();
        });
    }
    {
        let exp_c = images_expanded.clone();
        let rebuild_d = rebuild_dynamic_grid.clone();
        let rebuild_p = rebuild_photos_grid.clone();
        let rebuild_i = rebuild_images_grid.clone();
        let sync_c = sync_visuals.clone();
        let ref_c = tile_refreshers.clone();
        images_show_btn.connect_clicked(move |_| {
            exp_c.set(!exp_c.get());
            ref_c.borrow_mut().clear();
            rebuild_d();
            rebuild_p();
            rebuild_i();
            sync_c();
        });
    }
    {
        let exp_c = colors_expanded.clone();
        let rebuild_col = rebuild_colors_grid.clone();
        colors_show_btn.connect_clicked(move |_| {
            exp_c.set(!exp_c.get());
            rebuild_col();
        });
    }

    let built_once = Rc::new(Cell::new(false));
    let last_item_counts: Rc<Cell<(usize, usize, usize, usize)>> =
        Rc::new(Cell::new((0, 0, 0, 0)));

    let refresh_pane: Rc<dyn Fn()> = {
        let cfg_c = cfg_state.clone();
        let dyn_items_c = dyn_items.clone();
        let photos_c = user_photos.clone();
        let img_items_c = image_items.clone();
        let ref_c = tile_refreshers.clone();
        let rebuild_d = rebuild_dynamic_grid.clone();
        let rebuild_p = rebuild_photos_grid.clone();
        let rebuild_i = rebuild_images_grid.clone();
        let rebuild_col = rebuild_colors_grid.clone();
        let sync_c = sync_visuals.clone();
        let built_c = built_once.clone();
        let counts_c = last_item_counts.clone();
        Rc::new(move || {
            let loaded = WallpaperConfig::load();
            let custom_col_count = loaded.custom_colors.len();
            let custom_photo_count = loaded.custom_photos.len();
            *cfg_c.borrow_mut() = loaded;
            let next_counts = (
                dyn_items_c.borrow().len(),
                custom_photo_count,
                img_items_c.borrow().len(),
                custom_col_count,
            );
            if !built_c.get() || counts_c.get() != next_counts {
                built_c.set(true);
                counts_c.set(next_counts);
                let cfg_snap = cfg_c.borrow().clone();
                *photos_c.borrow_mut() = discover_user_photos(&cfg_snap);
                ref_c.borrow_mut().clear();
                rebuild_d();
                rebuild_p();
                rebuild_i();
                rebuild_col();
            }
            sync_c();
        })
    };

    refresh_pane();
    scrolled.set_child(Some(&pane));
    (scrolled, refresh_pane)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_colors_match_macos_palette_count_and_hex() {
        assert_eq!(CURATED_SOLID_COLORS.len(), 21);
        for (name, hex) in CURATED_SOLID_COLORS {
            assert!(!name.is_empty());
            assert!(
                parse_wallpaper_hex_color(hex).is_some(),
                "Hex inválido en paleta curada: {hex}"
            );
        }
    }

    #[test]
    fn formats_friendly_wallpaper_titles() {
        assert_eq!(friendly_stem_title("golden_gate-sunset"), "Golden Gate Sunset");

        let mut cfg = WallpaperConfig::default();
        cfg.mode = WallpaperMode::Gradient;
        assert_eq!(active_wallpaper_display_title(&cfg), "Degradado Aurora");

        cfg.mode = WallpaperMode::SolidColor;
        cfg.color_hex = "#445bd1".to_string();
        assert_eq!(active_wallpaper_display_title(&cfg), "Azul cobalto");
    }

    #[test]
    fn wallpaper_pane_builds_under_gtk_init() {
        if gtk4::init().is_err() {
            return;
        }
        let _ = libadwaita::init();
        let app = gtk4::Application::builder()
            .application_id("org.malus_de.SettingsTestWallpaper")
            .build();
        let win = ApplicationWindow::builder().application(&app).build();
        let (scrolled, refresh) = create_wallpaper_pane(&win);
        refresh();
        assert!(scrolled.child().is_some());
    }
}
