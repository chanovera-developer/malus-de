//! Utilidades compartidas de miniaturas de video y reproducción en línea (estilo macOS)
//! para Branches y el Escritorio de Malus DE.

use gtk4::gdk;
use gtk4::gdk_pixbuf;
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "webm", "mov", "avi", "m4v", "wmv", "flv", "ogv", "mpeg", "mpg", "3gp", "ts",
];

const SQUIRCLE_EXPONENT: f32 = 4.2;
const MAX_DISK_CACHE_FILES: usize = 400;
const DISK_CACHE_KEEP_FILES: usize = 300;

static NEXT_TMP_SEQ: AtomicU64 = AtomicU64::new(1);
static NEXT_PLAYBACK_ID: AtomicU64 = AtomicU64::new(1);

const VIDEO_THUMB_CSS: &str = r#"
.malus-video-thumb-overlay {
    border-radius: 10px;
}

button.malus-video-play-btn {
    min-width: 24px;
    min-height: 24px;
    padding: 0;
    margin: 0;
    border-radius: 999px;
    background-color: rgba(18, 20, 26, 0.64);
    background-image: none;
    border: 1px solid rgba(255, 255, 255, 0.38);
    color: #ffffff;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.42);
    opacity: 0.94;
    transition: opacity 150ms ease, background-color 150ms ease, transform 120ms ease;
}

button.malus-video-play-btn:hover {
    background-color: rgba(18, 20, 26, 0.86);
    border-color: rgba(255, 255, 255, 0.62);
    opacity: 1.0;
}

button.malus-video-play-btn:active {
    background-color: rgba(10, 12, 16, 0.94);
    transform: scale(0.94);
}

button.malus-video-play-btn image {
    color: #ffffff;
    -gtk-icon-shadow: 0 1px 2px rgba(0, 0, 0, 0.45);
}

.malus-video-thumb-overlay.is-playing button.malus-video-play-btn {
    opacity: 0.18;
    background-color: rgba(18, 20, 26, 0.56);
}

.malus-video-thumb-overlay.is-playing:hover button.malus-video-play-btn,
.malus-video-thumb-overlay.is-playing button.malus-video-play-btn:hover {
    opacity: 0.95;
    background-color: rgba(18, 20, 26, 0.82);
}
"#;

pub struct DecodedVideoThumb {
    pub width: i32,
    pub height: i32,
    pub rowstride: usize,
    pub bytes: glib::Bytes,
}

/// Comprueba por extensión si la ruta corresponde a un formato de video soportado.
pub fn is_video_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            VIDEO_EXTENSIONS
                .iter()
                .any(|cand| ext.eq_ignore_ascii_case(cand))
        })
}

/// Comprueba por tipo MIME o extensión si un archivo es un video.
pub fn is_video_file(path: &Path, content_type: Option<&str>) -> bool {
    if content_type.is_some_and(|ct| ct.starts_with("video/")) {
        return true;
    }
    is_video_path(path)
}

/// Prefija `file:` para FFmpeg/FFprobe/FFplay evitando que nombres con `:` sean tratados como protocolos.
pub fn safe_ffmpeg_input_arg(path: &Path) -> OsString {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut s = OsString::from("file:");
    s.push(abs.as_os_str());
    s
}

pub fn squircle_sdf(px: f32, py: f32, half_w: f32, half_h: f32, radius: f32) -> f32 {
    let r = radius.min(half_w.min(half_h)).max(1.0);
    let dx = px.abs() - half_w + r;
    let dy = py.abs() - half_h + r;
    if dx <= 0.0 || dy <= 0.0 {
        dx.max(dy) - r
    } else {
        (dx.powf(SQUIRCLE_EXPONENT) + dy.powf(SQUIRCLE_EXPONENT)).powf(1.0 / SQUIRCLE_EXPONENT)
            - r
    }
}

pub fn squircle_radius(width: i32, height: i32) -> f32 {
    let min_side = width.min(height).max(1) as f32;
    let max_r = (min_side * 0.45).max(0.5);
    let min_r = 3.0_f32.min(max_r);
    (min_side * 0.22).clamp(min_r, max_r)
}

/// Recorta las esquinas de un buffer RGBA (stride = width * 4) con curva squircle (`n = 4.2`).
pub fn apply_squircle_clip_rgba(rgba: &mut [u8], width: usize, height: usize, radius: f32) {
    if width < 4 || height < 4 {
        return;
    }
    let half_w = width as f32 * 0.5;
    let half_h = height as f32 * 0.5;
    let r = radius.min(half_w.min(half_h)).max(1.0);
    let span = (r.ceil() as usize + 1).min(width).min(height);
    let y_top_end = span.min(height);
    let y_bot_start = height.saturating_sub(span).max(y_top_end);
    let x_left_end = span.min(width);
    let x_right_start = width.saturating_sub(span).max(x_left_end);

    for y in (0..y_top_end).chain(y_bot_start..height) {
        let ly = (y as f32 + 0.5) - half_h;
        let row_offset = y * width * 4;
        for x in (0..x_left_end).chain(x_right_start..width) {
            let lx = (x as f32 + 0.5) - half_w;
            let dist = squircle_sdf(lx, ly, half_w, half_h, r);
            if dist > -0.5 {
                let alpha_idx = row_offset + x * 4 + 3;
                if dist >= 0.5 {
                    rgba[alpha_idx] = 0;
                } else {
                    let coverage = (0.5 - dist).clamp(0.0, 1.0);
                    rgba[alpha_idx] = (rgba[alpha_idx] as f32 * coverage).round() as u8;
                }
            }
        }
    }
}

/// Precalcula los píxeles de esquina `(byte_alpha_index, coverage_u8)` para recortar frames de video a 24fps sin coste.
fn precompute_squircle_corner_mask(width: usize, height: usize) -> Vec<(usize, u8)> {
    let mut ops = Vec::new();
    if width < 4 || height < 4 {
        return ops;
    }
    let radius = squircle_radius(width as i32, height as i32);
    let half_w = width as f32 * 0.5;
    let half_h = height as f32 * 0.5;
    let r = radius.min(half_w.min(half_h)).max(1.0);
    let span = (r.ceil() as usize + 1).min(width).min(height);
    let y_top_end = span.min(height);
    let y_bot_start = height.saturating_sub(span).max(y_top_end);
    let x_left_end = span.min(width);
    let x_right_start = width.saturating_sub(span).max(x_left_end);

    for y in (0..y_top_end).chain(y_bot_start..height) {
        let ly = (y as f32 + 0.5) - half_h;
        let row_offset = y * width * 4;
        for x in (0..x_left_end).chain(x_right_start..width) {
            let lx = (x as f32 + 0.5) - half_w;
            let dist = squircle_sdf(lx, ly, half_w, half_h, r);
            if dist > -0.5 {
                let alpha_idx = row_offset + x * 4 + 3;
                let alpha = if dist >= 0.5 {
                    0
                } else {
                    ((0.5 - dist).clamp(0.0, 1.0) * 255.0).round() as u8
                };
                ops.push((alpha_idx, alpha));
            }
        }
    }
    ops
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn bucket_decode_px(target_px: i32) -> i32 {
    if target_px <= 144 {
        128
    } else if target_px <= 288 {
        256
    } else {
        512
    }
}

pub fn video_thumb_cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("malus-de").join("video-thumbs")
}

pub fn video_thumb_cache_path(
    path: &Path,
    modified_secs: u64,
    file_size: u64,
    bucket_px: i32,
) -> PathBuf {
    let key = format!(
        "{}\0{modified_secs}\0{file_size}\0{bucket_px}",
        path.to_string_lossy()
    );
    let h1 = fnv1a_64(key.as_bytes());
    let h2 = fnv1a_64(&key.as_bytes().iter().rev().copied().collect::<Vec<u8>>());
    video_thumb_cache_dir().join(format!("{h1:016x}{h2:016x}.png"))
}

fn prune_disk_cache_if_needed(cache_dir: &Path) {
    let Ok(entries) = fs::read_dir(cache_dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("png") {
                return None;
            }
            let modified = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(UNIX_EPOCH);
            Some((modified, p))
        })
        .collect();
    if files.len() <= MAX_DISK_CACHE_FILES {
        return;
    }
    files.sort_unstable_by_key(|(t, _)| *t);
    let to_remove = files.len().saturating_sub(DISK_CACHE_KEEP_FILES);
    for (_, old) in files.into_iter().take(to_remove) {
        let _ = fs::remove_file(old);
    }
}

fn run_ffmpeg_extract_frame(
    video_path: &Path,
    seek_secs: &str,
    bucket_px: i32,
    out_png: &Path,
) -> bool {
    let scale_filter = format!(
        "scale='min({0},iw)':'min({0},ih)':force_original_aspect_ratio=decrease",
        bucket_px
    );
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-ss", seek_secs, "-i"])
        .arg(safe_ffmpeg_input_arg(video_path))
        .args(["-frames:v", "1", "-vf", &scale_filter])
        .arg(out_png)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    status.is_ok_and(|s| s.success())
        && fs::metadata(out_png).is_ok_and(|m| m.is_file() && m.len() > 0)
}

fn decode_pixbuf_squircle_rgba(path: &Path, target_px: i32) -> Option<DecodedVideoThumb> {
    let pixbuf =
        gdk_pixbuf::Pixbuf::from_file_at_scale(path, target_px, target_px, true).ok()?;
    let width = pixbuf.width();
    let height = pixbuf.height();
    let stride = pixbuf.rowstride() as usize;
    let channels = pixbuf.n_channels() as usize;
    let has_alpha = pixbuf.has_alpha();
    if width <= 0 || height <= 0 || channels < 3 {
        return None;
    }
    let w = width as usize;
    let h = height as usize;
    let src_bytes = pixbuf.read_pixel_bytes();
    let src = src_bytes.as_ref();
    let out_stride = w * 4;
    let mut rgba = vec![255u8; out_stride * h];

    for y in 0..h {
        let row_start = y * stride;
        let out_start = y * out_stride;
        if row_start + w * channels > src.len() {
            return None;
        }
        if has_alpha && channels == 4 {
            rgba[out_start..out_start + out_stride]
                .copy_from_slice(&src[row_start..row_start + out_stride]);
        } else {
            for x in 0..w {
                let s = row_start + x * channels;
                let d = out_start + x * 4;
                rgba[d..d + 3].copy_from_slice(&src[s..s + 3]);
                if has_alpha {
                    rgba[d + 3] = src[s + 3];
                }
            }
        }
    }

    apply_squircle_clip_rgba(&mut rgba, w, h, squircle_radius(width, height));
    Some(DecodedVideoThumb {
        width,
        height,
        rowstride: out_stride,
        bytes: glib::Bytes::from_owned(rgba),
    })
}

/// Extrae (con caché persistente en disco) una miniatura RGBA recortada en squircle para un archivo de video.
/// Pensada para ejecutarse en hilos worker fuera del hilo principal de GTK.
pub fn extract_video_thumbnail_rgba(path: &Path, target_px: i32) -> Option<DecodedVideoThumb> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 {
        return None;
    }
    let modified_secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let target_px = target_px.clamp(32, 512);
    let bucket_px = bucket_decode_px(target_px);
    let cache_path = video_thumb_cache_path(path, modified_secs, meta.len(), bucket_px);

    if cache_path.is_file() {
        if let Some(decoded) = decode_pixbuf_squircle_rgba(&cache_path, target_px) {
            return Some(decoded);
        }
        let _ = fs::remove_file(&cache_path);
    }

    let cache_dir = cache_path.parent()?;
    let _ = fs::create_dir_all(cache_dir);
    let seq = NEXT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_png = cache_dir.join(format!("tmp-{}-{seq}.png", std::process::id()));

    let extracted = run_ffmpeg_extract_frame(path, "0.5", bucket_px, &tmp_png)
        || run_ffmpeg_extract_frame(path, "0", bucket_px, &tmp_png);

    if !extracted {
        let _ = fs::remove_file(&tmp_png);
        return None;
    }

    let final_path = if fs::rename(&tmp_png, &cache_path).is_ok() {
        prune_disk_cache_if_needed(cache_dir);
        cache_path
    } else {
        tmp_png.clone()
    };

    let decoded = decode_pixbuf_squircle_rgba(&final_path, target_px);
    if final_path == tmp_png {
        let _ = fs::remove_file(&tmp_png);
    }
    decoded
}

/// Ajusta dimensiones pares manteniendo la relación de aspecto para `ffmpeg rawvideo`.
pub fn fit_even_dimensions(src_w: i32, src_h: i32, max_w: i32, max_h: i32) -> (i32, i32) {
    let sw = src_w.max(2) as f64;
    let sh = src_h.max(2) as f64;
    let scale = (max_w.max(2) as f64 / sw).min(max_h.max(2) as f64 / sh);
    let mut w = (sw * scale).round().max(2.0) as i32;
    let mut h = (sh * scale).round().max(2.0) as i32;
    if w % 2 != 0 {
        w = (w + 1).min(max_w.max(2));
        if w % 2 != 0 {
            w = (w - 1).max(2);
        }
    }
    if h % 2 != 0 {
        h = (h + 1).min(max_h.max(2));
        if h % 2 != 0 {
            h = (h - 1).max(2);
        }
    }
    (w, h)
}

fn probe_video_dimensions(path: &Path) -> Option<(i32, i32)> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0:s=x",
        ])
        .arg(safe_ffmpeg_input_arg(path))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next()?.trim();
    let (ws, hs) = line.split_once('x')?;
    let w = ws.trim().parse::<i32>().ok()?;
    let h = hs.trim().parse::<i32>().ok()?;
    if w > 0 && h > 0 {
        Some((w, h))
    } else {
        None
    }
}

struct InlinePlaybackSession {
    id: u64,
    stop_flag: Arc<AtomicBool>,
    children: Arc<Mutex<Vec<Child>>>,
    overlay_weak: glib::WeakRef<gtk4::Overlay>,
    image_weak: glib::WeakRef<gtk4::Image>,
    play_icon_weak: glib::WeakRef<gtk4::Image>,
    poster: Option<gdk::Paintable>,
}

impl InlinePlaybackSession {
    fn register_child(stop_flag: &AtomicBool, children: &Mutex<Vec<Child>>, mut child: Child) {
        if stop_flag.load(Ordering::SeqCst) {
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return;
        }
        if let Ok(mut guard) = children.lock() {
            guard.push(child);
        }
    }

    fn stop_and_restore(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        let mut drained = Vec::new();
        if let Ok(mut guard) = self.children.lock() {
            for mut child in guard.drain(..) {
                let _ = child.kill();
                drained.push(child);
            }
        }
        if !drained.is_empty() {
            std::thread::spawn(move || {
                for mut child in drained {
                    let _ = child.wait();
                }
            });
        }
        if let Some(overlay) = self.overlay_weak.upgrade() {
            overlay.remove_css_class("is-playing");
        }
        if let Some(play_icon) = self.play_icon_weak.upgrade() {
            play_icon.set_icon_name(Some("media-playback-start-symbolic"));
        }
        if let Some(img) = self.image_weak.upgrade() {
            if let Some(ref poster) = self.poster {
                img.set_paintable(Some(poster));
            }
        }
    }
}

thread_local! {
    static VIDEO_CSS_INSTALLED: RefCell<bool> = const { RefCell::new(false) };
    static ACTIVE_INLINE_PLAYBACK: RefCell<Option<Arc<InlinePlaybackSession>>> =
        const { RefCell::new(None) };
}

/// Instala una sola vez el CSS del botón ▶/⏸ sobre el Display activo.
pub fn ensure_video_thumb_css() {
    let already = VIDEO_CSS_INSTALLED.with(|c| *c.borrow());
    if already {
        return;
    }
    if let Some(display) = gdk::Display::default() {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(VIDEO_THUMB_CSS);
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 5,
        );
        VIDEO_CSS_INSTALLED.with(|c| *c.borrow_mut() = true);
    }
}

/// Detiene cualquier reproducción en línea activa en el proceso actual (restaurando la miniatura).
pub fn stop_active_inline_video_playback() {
    let active = ACTIVE_INLINE_PLAYBACK.with(|slot| slot.borrow_mut().take());
    if let Some(session) = active {
        session.stop_and_restore();
    }
}

fn is_overlay_currently_playing(overlay: &gtk4::Overlay) -> bool {
    ACTIVE_INLINE_PLAYBACK.with(|slot| {
        slot.borrow().as_ref().is_some_and(|s| {
            s.overlay_weak
                .upgrade()
                .is_some_and(|active_ov| active_ov == *overlay)
        })
    })
}

fn button_metrics_for_icon_size(icon_size: i32) -> (i32, i32) {
    let btn_px = ((icon_size as f32 * 0.40).round() as i32).clamp(22, 46);
    let glyph_px = ((btn_px as f32 * 0.50).round() as i32).clamp(11, 22);
    (btn_px, glyph_px)
}

/// Actualiza el tamaño del botón ▶/⏸ dentro de un overlay creado por `wrap_video_thumbnail_with_play_button`.
pub fn update_video_play_overlay_size(overlay: &gtk4::Overlay, icon_size: i32) {
    let (btn_px, glyph_px) = button_metrics_for_icon_size(icon_size);
    let mut child = overlay.first_child();
    while let Some(c) = child {
        child = c.next_sibling();
        if let Some(btn) = c.downcast_ref::<gtk4::Button>() {
            if btn.has_css_class("malus-video-play-btn") {
                btn.set_size_request(btn_px, btn_px);
                if let Some(icon) = btn.child().and_then(|w| w.downcast::<gtk4::Image>().ok()) {
                    icon.set_pixel_size(glyph_px);
                }
            }
        }
    }
}

fn start_inline_video_playback(
    overlay: &gtk4::Overlay,
    img: &gtk4::Image,
    play_icon: &gtk4::Image,
    path: &Path,
    play_audio: bool,
) {
    stop_active_inline_video_playback();

    let poster = img.paintable();
    let target_max_px = (img.pixel_size().max(img.width()).max(64) * 2).clamp(96, 512);
    let hint_dims = poster.as_ref().and_then(|p| {
        // Solo usar las dimensiones del paintable si es una textura de miniatura (MemoryTexture/Texture),
        // no un ícono simbólico o genérico.
        if p.is::<gdk::MemoryTexture>() {
            let iw = p.intrinsic_width();
            let ih = p.intrinsic_height();
            if iw > 4 && ih > 4 {
                return Some((iw, ih));
            }
        }
        None
    });

    let id = NEXT_PLAYBACK_ID.fetch_add(1, Ordering::Relaxed);
    let stop_flag = Arc::new(AtomicBool::new(false));
    let children = Arc::new(Mutex::new(Vec::new()));

    let session = Arc::new(InlinePlaybackSession {
        id,
        stop_flag: Arc::clone(&stop_flag),
        children: Arc::clone(&children),
        overlay_weak: overlay.downgrade(),
        image_weak: img.downgrade(),
        play_icon_weak: play_icon.downgrade(),
        poster,
    });

    overlay.add_css_class("is-playing");
    play_icon.set_icon_name(Some("media-playback-pause-symbolic"));
    ACTIVE_INLINE_PLAYBACK.with(|slot| {
        *slot.borrow_mut() = Some(Arc::clone(&session));
    });

    let (tx, rx) = mpsc::sync_channel::<(i32, i32, Vec<u8>)>(2);
    let path_owned = path.to_path_buf();
    let stop_worker = Arc::clone(&stop_flag);
    let children_worker = Arc::clone(&children);

    std::thread::Builder::new()
        .name("malus-video-inline".to_string())
        .spawn(move || {
            if stop_worker.load(Ordering::SeqCst) {
                return;
            }
            let (src_w, src_h) = hint_dims
                .or_else(|| probe_video_dimensions(&path_owned))
                .unwrap_or((16, 9));
            let (play_w, play_h) =
                fit_even_dimensions(src_w, src_h, target_max_px, target_max_px);
            let corner_mask = precompute_squircle_corner_mask(play_w as usize, play_h as usize);
            let scale_filter = format!("scale={play_w}:{play_h}:flags=fast_bilinear,fps=24");

            let child_video = Command::new("ffmpeg")
                .args(["-re", "-loglevel", "error", "-i"])
                .arg(safe_ffmpeg_input_arg(&path_owned))
                .args([
                    "-vf",
                    &scale_filter,
                    "-an",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "rgba",
                    "-",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();

            if play_audio && !stop_worker.load(Ordering::SeqCst) {
                if let Ok(audio_child) = Command::new("ffplay")
                    .args(["-nodisp", "-autoexit", "-loglevel", "quiet", "-i"])
                    .arg(safe_ffmpeg_input_arg(&path_owned))
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    InlinePlaybackSession::register_child(
                        &stop_worker,
                        &children_worker,
                        audio_child,
                    );
                }
            }

            let Ok(mut child) = child_video else {
                return;
            };
            let Some(mut stdout) = child.stdout.take() else {
                let _ = child.kill();
                let _ = child.wait();
                return;
            };
            InlinePlaybackSession::register_child(&stop_worker, &children_worker, child);

            let frame_bytes = (play_w as usize) * (play_h as usize) * 4;
            while !stop_worker.load(Ordering::SeqCst) {
                let mut buf = vec![0u8; frame_bytes];
                if stdout.read_exact(&mut buf).is_err() {
                    break;
                }
                for &(alpha_idx, alpha_val) in &corner_mask {
                    buf[alpha_idx] = alpha_val;
                }
                match tx.try_send((play_w, play_h, buf)) {
                    Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                    Err(mpsc::TrySendError::Disconnected(_)) => break,
                }
            }
        })
        .ok();

    let stop_ui = Arc::clone(&stop_flag);
    let img_weak = img.downgrade();
    let session_id = id;
    glib::timeout_add_local(Duration::from_millis(20), move || {
        if stop_ui.load(Ordering::SeqCst) {
            return glib::ControlFlow::Break;
        }
        let Some(target_img) = img_weak.upgrade() else {
            ACTIVE_INLINE_PLAYBACK.with(|slot| {
                if slot.borrow().as_ref().is_some_and(|s| s.id == session_id) {
                    if let Some(s) = slot.borrow_mut().take() {
                        s.stop_and_restore();
                    }
                }
            });
            return glib::ControlFlow::Break;
        };

        let mut latest = None;
        loop {
            match rx.try_recv() {
                Ok(frame) => latest = Some(frame),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if let Some((w, h, buf)) = latest {
                        let bytes = glib::Bytes::from_owned(buf);
                        let tex = gdk::MemoryTexture::new(
                            w,
                            h,
                            gdk::MemoryFormat::R8g8b8a8,
                            &bytes,
                            (w as usize) * 4,
                        );
                        target_img.set_paintable(Some(&tex));
                    }
                    ACTIVE_INLINE_PLAYBACK.with(|slot| {
                        if slot.borrow().as_ref().is_some_and(|s| s.id == session_id) {
                            if let Some(s) = slot.borrow_mut().take() {
                                s.stop_and_restore();
                            }
                        }
                    });
                    return glib::ControlFlow::Break;
                }
            }
        }

        if let Some((w, h, buf)) = latest {
            let bytes = glib::Bytes::from_owned(buf);
            let tex = gdk::MemoryTexture::new(
                w,
                h,
                gdk::MemoryFormat::R8g8b8a8,
                &bytes,
                (w as usize) * 4,
            );
            target_img.set_paintable(Some(&tex));
        }

        glib::ControlFlow::Continue
    });
}

/// Envuelve un `gtk4::Image` de video en un `gtk4::Overlay` con un botón interactivo ▶/⏸ estilo macOS
/// que reproduce el video en línea dentro de la propia miniatura.
pub fn wrap_video_thumbnail_with_play_button(
    img: &gtk4::Image,
    path: &Path,
    icon_size: i32,
    play_audio: bool,
) -> gtk4::Overlay {
    ensure_video_thumb_css();

    let overlay = gtk4::Overlay::new();
    overlay.add_css_class("malus-video-thumb-overlay");
    overlay.set_halign(gtk4::Align::Center);
    overlay.set_valign(gtk4::Align::Center);
    overlay.set_child(Some(img));

    let (btn_px, glyph_px) = button_metrics_for_icon_size(icon_size);
    let play_icon = gtk4::Image::from_icon_name("media-playback-start-symbolic");
    play_icon.set_pixel_size(glyph_px);
    play_icon.set_halign(gtk4::Align::Center);
    play_icon.set_valign(gtk4::Align::Center);

    let play_btn = gtk4::Button::builder()
        .child(&play_icon)
        .has_frame(false)
        .focusable(false)
        .can_focus(false)
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .width_request(btn_px)
        .height_request(btn_px)
        .css_classes(["malus-video-play-btn"])
        .tooltip_text(if crate::is_spanish_locale() {
            "Reproducir / Pausar vista previa"
        } else {
            "Play / Pause preview"
        })
        .build();

    let ov_click = overlay.downgrade();
    let img_click = img.downgrade();
    let icon_click = play_icon.downgrade();
    let path_click = path.to_path_buf();
    play_btn.connect_clicked(move |_| {
        let (Some(ov), Some(target_img), Some(p_icon)) =
            (ov_click.upgrade(), img_click.upgrade(), icon_click.upgrade())
        else {
            return;
        };
        if is_overlay_currently_playing(&ov) {
            stop_active_inline_video_playback();
        } else {
            start_inline_video_playback(&ov, &target_img, &p_icon, &path_click, play_audio);
        }
    });

    // Asegurar que al desmontar o destruir la miniatura (cambio de carpeta/recarga) se detenga audio y video
    let ov_unmap = overlay.downgrade();
    overlay.connect_unmap(move |_| {
        if let Some(ov) = ov_unmap.upgrade() {
            if is_overlay_currently_playing(&ov) {
                stop_active_inline_video_playback();
            }
        }
    });
    let ov_destroy = overlay.downgrade();
    overlay.connect_destroy(move |_| {
        if let Some(ov) = ov_destroy.upgrade() {
            if is_overlay_currently_playing(&ov) {
                stop_active_inline_video_playback();
            }
        }
    });

    overlay.add_overlay(&play_btn);
    overlay
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_video_extensions_and_mime_types() {
        assert!(is_video_path(Path::new("/tmp/clip.MP4")));
        assert!(is_video_path(Path::new("/tmp/movie.mkv")));
        assert!(is_video_path(Path::new("/tmp/screen.webm")));
        assert!(is_video_path(Path::new("/tmp/demo.mov")));
        assert!(!is_video_path(Path::new("/tmp/photo.png")));
        assert!(!is_video_path(Path::new("/tmp/notes.txt")));

        assert!(is_video_file(Path::new("/tmp/sin_ext"), Some("video/mp4")));
        assert!(is_video_file(
            Path::new("/tmp/grabacion.mp4"),
            Some("application/x-zerosize")
        ));
        assert!(!is_video_file(
            Path::new("/tmp/imagen.webp"),
            Some("image/webp")
        ));
    }

    #[test]
    fn cache_path_is_deterministic_and_invalidates_on_mtime_or_size() {
        let p = Path::new("/home/user/Escritorio/demo.mp4");
        let a = video_thumb_cache_path(p, 1000, 5000, 128);
        let b = video_thumb_cache_path(p, 1000, 5000, 128);
        let c = video_thumb_cache_path(p, 1001, 5000, 128);
        let d = video_thumb_cache_path(p, 1000, 5001, 128);
        let e = video_thumb_cache_path(p, 1000, 5000, 256);

        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_ne!(a, e);
        assert_eq!(a.extension().and_then(|s| s.to_str()), Some("png"));
    }

    #[test]
    fn fit_even_dimensions_keeps_aspect_and_even_numbers() {
        let (w, h) = fit_even_dimensions(1920, 1080, 128, 128);
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
        assert!(w <= 128 && h <= 128);
        assert_eq!(w, 128);
        assert_eq!(h, 72);

        let (pw, ph) = fit_even_dimensions(1080, 1920, 128, 128);
        assert_eq!(pw % 2, 0);
        assert_eq!(ph % 2, 0);
        assert_eq!(pw, 72);
        assert_eq!(ph, 128);
    }

    #[test]
    fn squircle_corner_mask_clears_extreme_corners_and_keeps_center() {
        let w = 64usize;
        let h = 64usize;
        let mask = precompute_squircle_corner_mask(w, h);
        let mut buf = vec![255u8; w * h * 4];
        for &(idx, alpha) in &mask {
            buf[idx] = alpha;
        }
        // Esquina (0, 0) transparente; centro (32, 32) opaco
        assert_eq!(buf[3], 0);
        assert_eq!(buf[(32 * w + 32) * 4 + 3], 255);
    }

    #[test]
    fn extracts_and_caches_real_mp4_thumbnail_when_ffmpeg_is_available() {
        let has_ffmpeg = Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !has_ffmpeg {
            return;
        }

        let temp_dir = std::env::temp_dir().join("malus_video_thumb_unit_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let mp4_path = temp_dir.join("sample.mp4");

        let created = Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:d=1",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&mp4_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        assert!(created, "ffmpeg debe generar un MP4 de prueba");

        let thumb = extract_video_thumbnail_rgba(&mp4_path, 128)
            .expect("debe extraer miniatura squircle del MP4");
        assert_eq!(thumb.width, 128);
        assert_eq!(thumb.height, 72);
        let raw = thumb.bytes.as_ref();
        assert_eq!(raw[3], 0, "esquina debe quedar recortada a transparente");
        assert_eq!(
            raw[(36 * 128 + 64) * 4 + 3],
            255,
            "centro debe quedar opaco"
        );

        // Segunda llamada debe reutilizar la caché en disco con el mismo resultado
        let cached = extract_video_thumbnail_rgba(&mp4_path, 128)
            .expect("debe cargar desde caché en disco");
        assert_eq!(cached.width, thumb.width);
        assert_eq!(cached.height, thumb.height);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
