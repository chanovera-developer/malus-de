use glib::ControlFlow;
use gtk4::gdk::{self, MemoryFormat, MemoryTexture, ModifierType, Texture};
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, ContentFit, CssProvider,
    DrawingArea, Entry, EventControllerKey, EventControllerMotion, GestureClick, GestureDrag,
    Image, Label, MenuButton, Orientation, Overflow, Overlay, Picture, Popover, PropagationPhase,
    Separator, ToggleButton, WindowHandle,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

const THUMBNAIL_CSS: &str = include_str!("styles/thumbnail.css");
const CAPTURE_FLASH_CSS: &str = include_str!("styles/capture-flash.css");

/// Duración exacta solicitada para la miniatura flotante inferior derecha (5 segundos).
pub const THUMBNAIL_DISPLAY_SECS: u64 = 5;

const THUMBNAIL_MAX_W: i32 = 248;
const THUMBNAIL_MAX_H: i32 = 152;

// Constantes de geometría idénticas a Quick Look en Branches (`crates/branches/src/quick_look.rs`)
const QL_HEADER_HEIGHT: i32 = 42;
const QL_IMAGE_INSET: i32 = 8;
const QL_MIN_CARD_W: i32 = 340;
const QL_MIN_CARD_H: i32 = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewKind {
    Image,
    Video,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarkupTool {
    Pen,
    Rect,
    Arrow,
}

#[derive(Clone, Debug)]
struct MarkupStroke {
    tool: MarkupTool,
    color: (f64, f64, f64),
    line_width: f64,
    /// Puntos normalizados [0.0..1.0] respecto al rectángulo renderizado de la imagen.
    points: Vec<(f64, f64)>,
}

#[derive(Clone)]
struct DecodedRgbaFrame {
    width: i32,
    height: i32,
    stride: usize,
    bytes: glib::Bytes,
}

impl DecodedRgbaFrame {
    fn to_texture(&self) -> Texture {
        MemoryTexture::new(
            self.width,
            self.height,
            MemoryFormat::R8g8b8a8,
            &self.bytes,
            self.stride,
        )
        .upcast()
    }
}

#[derive(Clone)]
struct MediaPlaybackHandle {
    stop_flag: Arc<AtomicBool>,
    children: Arc<Mutex<Vec<Child>>>,
}

impl MediaPlaybackHandle {
    fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            children: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn register_child(&self, mut child: Child) {
        if self.stop_flag.load(Ordering::SeqCst) {
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return;
        }
        if let Ok(mut guard) = self.children.lock() {
            guard.push(child);
        }
    }

    fn stop(&self) {
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
    }
}

impl Drop for MediaPlaybackHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone, Default)]
struct MediaMetadata {
    duration_secs: Option<f64>,
    duration_str: Option<String>,
    codec: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    has_audio: bool,
}

/// Geometría adaptativa de la tarjeta Quick Look idéntica a Branches (`fit_quicklook_card_size`).
pub fn fit_quicklook_card_size(
    img_w: i32,
    img_h: i32,
    max_card_w: i32,
    max_card_h: i32,
) -> (i32, i32, i32, i32) {
    let avail_w = (max_card_w - QL_IMAGE_INSET * 2).max(200) as f64;
    let avail_h = (max_card_h - QL_HEADER_HEIGHT - QL_IMAGE_INSET).max(160) as f64;
    let iw = img_w.max(1) as f64;
    let ih = img_h.max(1) as f64;

    let scale_down = (avail_w / iw).min(avail_h / ih).min(1.0);
    let mut fit_w = (iw * scale_down).round().max(1.0) as i32;
    let mut fit_h = (ih * scale_down).round().max(1.0) as i32;

    let min_long_edge = 260.0;
    let cur_long = (fit_w as f64).max(fit_h as f64);
    if cur_long < min_long_edge {
        let up = (min_long_edge / cur_long)
            .min(avail_w / fit_w.max(1) as f64)
            .min(avail_h / fit_h.max(1) as f64);
        fit_w = (fit_w as f64 * up).round().max(1.0) as i32;
        fit_h = (fit_h as f64 * up).round().max(1.0) as i32;
    }

    let card_w = (fit_w + QL_IMAGE_INSET * 2).clamp(QL_MIN_CARD_W.min(max_card_w), max_card_w);
    let card_h = (fit_h + QL_HEADER_HEIGHT + QL_IMAGE_INSET)
        .clamp(QL_MIN_CARD_H.min(max_card_h), max_card_h);
    (card_w, card_h, fit_w, fit_h)
}

fn primary_monitor_work_bounds() -> (i32, i32) {
    if let Some(display) = gdk::Display::default() {
        let monitors = display.monitors();
        if let Some(obj) = monitors.item(0) {
            if let Ok(mon) = obj.downcast::<gdk::Monitor>() {
                let geo = mon.geometry();
                if geo.width() > 0 && geo.height() > 0 {
                    let mw = ((geo.width() as f64) * 0.80).round() as i32;
                    let mh = ((geo.height() as f64) * 0.84).round() as i32;
                    return (mw.clamp(460, 1280), mh.clamp(360, 960));
                }
            }
        }
    }
    (920, 720)
}

fn fit_video_dimensions(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let sw = src_w.max(2) as f64;
    let sh = src_h.max(2) as f64;
    let scale = (max_w as f64 / sw).min(max_h as f64 / sh).min(1.0);
    let mut w = ((sw * scale).round() as u32).max(2);
    let mut h = ((sh * scale).round() as u32).max(2);
    if w % 2 != 0 {
        w += 1;
    }
    if h % 2 != 0 {
        h += 1;
    }
    (w, h)
}

fn rotate_rgba_frame_ccw(frame: &DecodedRgbaFrame) -> DecodedRgbaFrame {
    let w = frame.width.max(1) as usize;
    let h = frame.height.max(1) as usize;
    let src = frame.bytes.as_ref();
    let new_w = h;
    let new_h = w;
    let new_stride = new_w * 4;
    let mut dst = vec![0u8; new_stride * new_h];

    for y in 0..h {
        let src_row = y * frame.stride;
        for x in 0..w {
            let nx = y;
            let ny = w - 1 - x;
            let sp = src_row + x * 4;
            let dp = ny * new_stride + nx * 4;
            if sp + 4 <= src.len() && dp + 4 <= dst.len() {
                dst[dp..dp + 4].copy_from_slice(&src[sp..sp + 4]);
            }
        }
    }

    DecodedRgbaFrame {
        width: new_w as i32,
        height: new_h as i32,
        stride: new_stride,
        bytes: glib::Bytes::from_owned(dst),
    }
}

fn decode_png_to_rgba(path: &Path) -> Option<DecodedRgbaFrame> {
    let file = fs::File::open(path).ok()?;
    let mut decoder = png::Decoder::new(file);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let w = info.width as usize;
    let h = info.height as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let src = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => src.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(w * h * 4);
            for px in src.chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity(w * h * 4);
            for &g in src {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity(w * h * 4);
            for px in src.chunks_exact(2) {
                out.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
            out
        }
        _ => return None,
    };
    if rgba.len() != w * h * 4 {
        return None;
    }
    Some(DecodedRgbaFrame {
        width: w as i32,
        height: h as i32,
        stride: w * 4,
        bytes: glib::Bytes::from_owned(rgba),
    })
}

fn safe_ffmpeg_input_arg(path: &Path) -> std::ffi::OsString {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut s = std::ffi::OsString::from("file:");
    s.push(abs.as_os_str());
    s
}

fn unique_temp_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "malus_shot_ql_{}_{}_{}.{}",
        tag,
        std::process::id(),
        nanos,
        ext
    ))
}

fn probe_media_metadata(path: &Path) -> MediaMetadata {
    let mut meta = MediaMetadata::default();
    let Ok(output) = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_name,codec_type,width,height",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(safe_ffmpeg_input_arg(path))
        .output()
    else {
        return meta;
    };
    if !output.status.success() {
        return meta;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let val = v.trim();
        match k.trim() {
            "duration" => {
                if let Ok(secs) = val.parse::<f64>() {
                    meta.duration_secs = Some(secs);
                    let mins = (secs / 60.0).floor() as u64;
                    let rem = (secs % 60.0).round() as u64;
                    meta.duration_str = Some(format!("{:02}:{:02}", mins, rem));
                }
            }
            "codec_type" if val == "audio" => {
                meta.has_audio = true;
            }
            "codec_name" if meta.codec.is_none() => {
                meta.codec = Some(val.to_string());
            }
            "width" if meta.width.is_none() => {
                if let Ok(w) = val.parse::<u32>() {
                    if w > 0 {
                        meta.width = Some(w);
                    }
                }
            }
            "height" if meta.height.is_none() => {
                if let Ok(h) = val.parse::<u32>() {
                    if h > 0 {
                        meta.height = Some(h);
                    }
                }
            }
            _ => {}
        }
    }
    meta
}

fn extract_video_poster_rgba(
    path: &Path,
    duration_secs: Option<f64>,
) -> Option<DecodedRgbaFrame> {
    let seek_arg = match duration_secs {
        Some(d) if d > 2.0 => "0.5",
        _ => "0",
    };
    let tmp_png = unique_temp_path("vposter", "png");
    let ok = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-ss", seek_arg, "-i"])
        .arg(safe_ffmpeg_input_arg(path))
        .args(["-frames:v", "1", "-vf", "scale='min(760,iw)':-1"])
        .arg(&tmp_png)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let frame = if ok && tmp_png.exists() {
        decode_png_to_rgba(&tmp_png)
    } else {
        None
    };
    let _ = fs::remove_file(&tmp_png);
    frame
}

fn spawn_video_playback(
    path: &Path,
    width: u32,
    height: u32,
    play_audio: bool,
    picture: &Picture,
    media_handles: &Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>>,
    on_finished: Rc<dyn Fn()>,
) -> Rc<MediaPlaybackHandle> {
    let handle = Rc::new(MediaPlaybackHandle::new());
    media_handles.borrow_mut().push(handle.clone());

    let scale_filter = format!("scale={}:{}:flags=fast_bilinear,fps=24", width, height);
    let child_video = Command::new("ffmpeg")
        .args(["-re", "-loglevel", "error", "-i"])
        .arg(safe_ffmpeg_input_arg(path))
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

    if play_audio {
        if let Ok(audio_child) = Command::new("ffplay")
            .args(["-nodisp", "-autoexit", "-loglevel", "quiet", "-i"])
            .arg(safe_ffmpeg_input_arg(path))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            handle.register_child(audio_child);
        }
    }

    let Ok(mut child) = child_video else {
        on_finished();
        return handle;
    };

    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        on_finished();
        return handle;
    };
    handle.register_child(child);

    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(2);
    let stop_flag = handle.stop_flag.clone();
    let frame_bytes = (width as usize) * (height as usize) * 4;

    std::thread::spawn(move || {
        while !stop_flag.load(Ordering::SeqCst) {
            let mut buf = vec![0u8; frame_bytes];
            if stdout.read_exact(&mut buf).is_err() {
                break;
            }
            match tx.try_send(buf) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => break,
            }
        }
    });

    let stop_ui = handle.stop_flag.clone();
    let pic_weak = picture.downgrade();
    let stride = (width as usize) * 4;
    let w_i32 = width as i32;
    let h_i32 = height as i32;

    glib::timeout_add_local(Duration::from_millis(20), move || {
        if stop_ui.load(Ordering::SeqCst) {
            return ControlFlow::Break;
        }
        let Some(pic) = pic_weak.upgrade() else {
            return ControlFlow::Break;
        };

        let mut latest_frame = None;
        loop {
            match rx.try_recv() {
                Ok(frame) => latest_frame = Some(frame),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if let Some(frame) = latest_frame {
                        let bytes = glib::Bytes::from_owned(frame);
                        let tex = MemoryTexture::new(
                            w_i32,
                            h_i32,
                            MemoryFormat::R8g8b8a8,
                            &bytes,
                            stride,
                        );
                        pic.set_paintable(Some(&tex));
                    }
                    on_finished();
                    return ControlFlow::Break;
                }
            }
        }

        if let Some(frame) = latest_frame {
            let bytes = glib::Bytes::from_owned(frame);
            let tex = MemoryTexture::new(w_i32, h_i32, MemoryFormat::R8g8b8a8, &bytes, stride);
            pic.set_paintable(Some(&tex));
        }

        ControlFlow::Continue
    });

    handle
}

/// Sanitiza y renombra el archivo de captura dentro de su mismo directorio preservando la extensión.
pub fn rename_capture_file(current_path: &Path, requested_stem: &str) -> Result<PathBuf, String> {
    let cleaned: String = requested_stem
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '-',
            _ => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        return Err("El nombre no puede estar vacío".to_string());
    }

    let ext = current_path
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty());

    let cleaned_without_dup_ext = if let Some(e) = ext {
        let suffix = format!(".{e}");
        if cleaned.to_lowercase().ends_with(&suffix.to_lowercase()) && cleaned.len() > suffix.len()
        {
            cleaned[..cleaned.len() - suffix.len()].trim().to_string()
        } else {
            cleaned
        }
    } else {
        cleaned
    };

    if cleaned_without_dup_ext.is_empty() {
        return Err("El nombre no puede estar vacío".to_string());
    }

    let new_file_name = match ext {
        Some(e) => format!("{cleaned_without_dup_ext}.{e}"),
        None => cleaned_without_dup_ext,
    };

    let parent = current_path
        .parent()
        .ok_or_else(|| "No se pudo determinar la carpeta de la captura".to_string())?;
    let target = parent.join(&new_file_name);

    if target == current_path {
        return Ok(current_path.to_path_buf());
    }
    if target.exists() {
        return Err(format!("Ya existe un archivo llamado «{new_file_name}»"));
    }

    fs::rename(current_path, &target)
        .map_err(|err| format!("No se pudo renombrar la captura: {err}"))?;
    Ok(target)
}

fn move_file_to_trash(path: &Path) -> Result<(), String> {
    if let Ok(status) = Command::new("gio").arg("trash").arg(path).status() {
        if status.success() {
            return Ok(());
        }
    }
    fs::remove_file(path).map_err(|err| format!("No se pudo eliminar la captura: {err}"))
}

fn read_png_dimensions(path: &Path) -> Option<(u32, u32)> {
    let file = fs::File::open(path).ok()?;
    let decoder = png::Decoder::new(file);
    let reader = decoder.read_info().ok()?;
    let info = reader.info();
    Some((info.width, info.height))
}

fn compute_thumbnail_size(path: &Path, kind: PreviewKind) -> (i32, i32) {
    if kind == PreviewKind::Video {
        return (240, 144);
    }
    if let Some((w, h)) = read_png_dimensions(path) {
        if w > 0 && h > 0 {
            let ratio = w as f64 / h as f64;
            let mut tw = THUMBNAIL_MAX_W as f64;
            let mut th = tw / ratio;
            if th > THUMBNAIL_MAX_H as f64 {
                th = THUMBNAIL_MAX_H as f64;
                tw = th * ratio;
            }
            return (
                tw.round().clamp(120.0, THUMBNAIL_MAX_W as f64) as i32,
                th.round().clamp(84.0, THUMBNAIL_MAX_H as f64) as i32,
            );
        }
    }
    (240, 144)
}

fn draw_stroke_in_rect(
    cr: &gtk4::cairo::Context,
    stroke: &MarkupStroke,
    ox: f64,
    oy: f64,
    rw: f64,
    rh: f64,
    width_scale: f64,
) {
    if stroke.points.is_empty() {
        return;
    }
    let (r, g, b) = stroke.color;
    cr.set_source_rgba(r, g, b, 0.96);
    cr.set_line_width((stroke.line_width * width_scale).max(1.5));
    cr.set_line_cap(gtk4::cairo::LineCap::Round);
    cr.set_line_join(gtk4::cairo::LineJoin::Round);

    match stroke.tool {
        MarkupTool::Pen => {
            if stroke.points.len() == 1 {
                let (nx, ny) = stroke.points[0];
                cr.arc(
                    ox + nx * rw,
                    oy + ny * rh,
                    (stroke.line_width * width_scale) * 0.5,
                    0.0,
                    std::f64::consts::TAU,
                );
                let _ = cr.fill();
                return;
            }
            let (x0, y0) = stroke.points[0];
            cr.move_to(ox + x0 * rw, oy + y0 * rh);
            for &(nx, ny) in &stroke.points[1..] {
                cr.line_to(ox + nx * rw, oy + ny * rh);
            }
            let _ = cr.stroke();
        }
        MarkupTool::Rect => {
            let (x0, y0) = stroke.points[0];
            let (x1, y1) = *stroke.points.last().unwrap_or(&(x0, y0));
            let rx = ox + x0.min(x1) * rw;
            let ry = oy + y0.min(y1) * rh;
            let w = (x1 - x0).abs() * rw;
            let h = (y1 - y0).abs() * rh;
            cr.rectangle(rx, ry, w, h);
            let _ = cr.stroke();
        }
        MarkupTool::Arrow => {
            let (x0, y0) = stroke.points[0];
            let (x1, y1) = *stroke.points.last().unwrap_or(&(x0, y0));
            let sx = ox + x0 * rw;
            let sy = oy + y0 * rh;
            let ex = ox + x1 * rw;
            let ey = oy + y1 * rh;
            cr.move_to(sx, sy);
            cr.line_to(ex, ey);
            let _ = cr.stroke();

            let angle = (ey - sy).atan2(ex - sx);
            let head_len = (14.0 * width_scale).max(8.0);
            let spread = 0.48;
            cr.move_to(ex, ey);
            cr.line_to(
                ex - head_len * (angle - spread).cos(),
                ey - head_len * (angle - spread).sin(),
            );
            cr.move_to(ex, ey);
            cr.line_to(
                ex - head_len * (angle + spread).cos(),
                ey - head_len * (angle + spread).sin(),
            );
            let _ = cr.stroke();
        }
    }
}

/// Aplica rotaciones (en cuartos de vuelta antihorarios) y aplana las anotaciones sobre el PNG en disco.
fn save_image_edits_to_png(
    path: &Path,
    rotation_ccw_turns: u8,
    strokes: &[MarkupStroke],
) -> Result<(), String> {
    let turns = rotation_ccw_turns % 4;
    if turns == 0 && strokes.is_empty() {
        return Ok(());
    }
    let mut frame =
        decode_png_to_rgba(path).ok_or_else(|| "No se pudo decodificar la imagen PNG".to_string())?;
    for _ in 0..turns {
        frame = rotate_rgba_frame_ccw(&frame);
    }

    let w_u32 = frame.width.max(1) as u32;
    let h_u32 = frame.height.max(1) as u32;
    let expected = (w_u32 as usize) * (h_u32 as usize) * 4;
    let rgba = frame.bytes.as_ref();

    let out_rgba = if strokes.is_empty() {
        rgba[..expected].to_vec()
    } else {
        // Convertir RGBA (recto) -> ARGB32 little-endian (BGRA premultiplicado) para Cairo
        let mut bgra = vec![0u8; expected];
        for (src, dst) in rgba[..expected].chunks_exact(4).zip(bgra.chunks_exact_mut(4)) {
            let a = u16::from(src[3]);
            dst[0] = ((u16::from(src[2]) * a + 127) / 255) as u8;
            dst[1] = ((u16::from(src[1]) * a + 127) / 255) as u8;
            dst[2] = ((u16::from(src[0]) * a + 127) / 255) as u8;
            dst[3] = src[3];
        }

        let stride = (w_u32 as i32) * 4;
        let mut surface = gtk4::cairo::ImageSurface::create_for_data(
            bgra,
            gtk4::cairo::Format::ARgb32,
            w_u32 as i32,
            h_u32 as i32,
            stride,
        )
        .map_err(|e| format!("No se pudo crear superficie Cairo: {e}"))?;

        let w = w_u32 as f64;
        let h = h_u32 as f64;
        let scale_factor = (w / 900.0).clamp(0.8, 3.0);
        {
            let cr = gtk4::cairo::Context::new(&surface)
                .map_err(|e| format!("No se pudo crear contexto Cairo: {e}"))?;
            for stroke in strokes {
                draw_stroke_in_rect(&cr, stroke, 0.0, 0.0, w, h, scale_factor);
            }
        }
        surface.flush();

        let rendered = surface
            .data()
            .map_err(|e| format!("No se pudo leer superficie anotada: {e}"))?;
        let mut out = vec![0u8; expected];
        for (src, dst) in rendered.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
            let a = src[3];
            if a == 0 {
                dst.copy_from_slice(&[0, 0, 0, 0]);
            } else if a == 255 {
                dst[0] = src[2];
                dst[1] = src[1];
                dst[2] = src[0];
                dst[3] = 255;
            } else {
                let a_u16 = u16::from(a);
                dst[0] = ((u16::from(src[2]) * 255 + a_u16 / 2) / a_u16).min(255) as u8;
                dst[1] = ((u16::from(src[1]) * 255 + a_u16 / 2) / a_u16).min(255) as u8;
                dst[2] = ((u16::from(src[0]) * 255 + a_u16 / 2) / a_u16).min(255) as u8;
                dst[3] = a;
            }
        }
        drop(rendered);
        out
    };

    let tmp = path.with_extension("markup.tmp");
    let out_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp)
        .map_err(|e| format!("No se pudo crear archivo temporal: {e}"))?;
    let mut encoder = png::Encoder::new(out_file, w_u32, h_u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("No se pudo escribir cabecera PNG: {e}"))?;
    writer
        .write_image_data(&out_rgba)
        .map_err(|e| format!("No se pudo escribir PNG editado: {e}"))?;
    drop(writer);

    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("No se pudo actualizar captura editada: {e}")
    })?;
    Ok(())
}

fn image_display_rect(canvas_w: f64, canvas_h: f64, img_w: f64, img_h: f64) -> (f64, f64, f64, f64) {
    if canvas_w <= 1.0 || canvas_h <= 1.0 || img_w <= 1.0 || img_h <= 1.0 {
        return (0.0, 0.0, canvas_w.max(1.0), canvas_h.max(1.0));
    }
    let scale = (canvas_w / img_w).min(canvas_h / img_h);
    let rw = img_w * scale;
    let rh = img_h * scale;
    let ox = (canvas_w - rw) * 0.5;
    let oy = (canvas_h - rh) * 0.5;
    (ox, oy, rw, rh)
}

/// Muestra el destello (para fotos), la miniatura flotante inferior derecha (5 s, `Captura-pantalla1.png`)
/// y, al pulsarla, despliega la ventana de vista previa idéntica a Quick Look en Branches.
pub fn show_capture_preview_flow(
    kind: PreviewKind,
    capture_path: Option<PathBuf>,
    show_flash: bool,
    on_copy_clipboard: fn(&Path) -> anyhow::Result<()>,
) {
    let _ = malus_de_common::config::setup_isolated_session_environment();
    if let Err(err) = adw::init() {
        warn!("Aviso al inicializar libadwaita: {:?}", err);
    }
    let is_dark = malus_de_common::config::is_dark_mode_active();
    adw::StyleManager::default().set_color_scheme(if is_dark {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::ForceLight
    });

    let app = Application::builder()
        .application_id(format!(
            "org.malus.de.screenshot-ui-{}",
            std::process::id()
        ))
        .build();

    app.connect_startup(|_| {
        let flash_provider = CssProvider::new();
        flash_provider.load_from_string(CAPTURE_FLASH_CSS);
        let thumb_provider = CssProvider::new();
        thumb_provider.load_from_string(THUMBNAIL_CSS);
        let accent_provider = CssProvider::new();
        accent_provider.load_from_string(&malus_de_common::config::accent_color_css(
            malus_de_common::config::load_accent_color_index(),
        ));

        if let Some(display) = gdk::Display::default() {
            let icon_theme = gtk4::IconTheme::for_display(&display);
            for icon_dir in malus_de_common::apps::extra_icon_search_paths() {
                if icon_dir.is_dir() {
                    icon_theme.add_search_path(&icon_dir);
                }
            }
            gtk4::style_context_add_provider_for_display(
                &display,
                &flash_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            gtk4::style_context_add_provider_for_display(
                &display,
                &thumb_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_USER + 1,
            );
            gtk4::style_context_add_provider_for_display(
                &display,
                &accent_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_USER + 3,
            );
        }
    });

    let capture_path_clone = capture_path.clone();
    app.connect_activate(move |application| {
        let Some(display) = gdk::Display::default() else {
            warn!("No se pudo mostrar la interfaz de captura: no hay pantalla GTK");
            application.quit();
            return;
        };

        // 1. Destello blanco visual en cada pantalla conectada (solo fotos)
        let mut flash_windows = Vec::new();
        if show_flash && kind == PreviewKind::Image {
            let monitors = display.monitors();
            for index in 0..monitors.n_items() {
                let Some(monitor) = monitors
                    .item(index)
                    .and_then(|item| item.downcast::<gdk::Monitor>().ok())
                else {
                    continue;
                };

                let flash_window = ApplicationWindow::builder()
                    .application(application)
                    .title("Destello de captura")
                    .decorated(false)
                    .css_classes(["malus-screenshot-flash"])
                    .opacity(0.85)
                    .build();

                if gtk4_layer_shell::is_supported() {
                    flash_window.init_layer_shell();
                    flash_window.set_layer(Layer::Overlay);
                    flash_window.set_namespace("malus-screenshot-flash");
                    flash_window.set_monitor(&monitor);
                    flash_window.set_exclusive_zone(-1);
                    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                        flash_window.set_anchor(edge, true);
                    }
                } else {
                    flash_window.fullscreen();
                }

                flash_window.set_child(Some(&GtkBox::new(Orientation::Vertical, 0)));
                flash_window.present();
                flash_windows.push(flash_window);
            }
        }

        let is_clipboard_only = capture_path_clone.is_none();
        let app_for_flash = application.clone();
        let mut step = 0u8;
        if !flash_windows.is_empty() {
            glib::timeout_add_local(Duration::from_millis(25), move || {
                step += 1;
                let opacity = 0.85 * (1.0 - f64::from(step) / 8.0);
                for win in &flash_windows {
                    win.set_opacity(opacity.max(0.0));
                }
                if step >= 8 {
                    for win in &flash_windows {
                        win.close();
                    }
                    if is_clipboard_only {
                        app_for_flash.quit();
                    }
                    ControlFlow::Break
                } else {
                    ControlFlow::Continue
                }
            });
        } else if is_clipboard_only {
            application.quit();
        }

        // 2. Miniatura flotante estilo macOS (Captura-pantalla1.png) durante 5 segundos
        if let Some(ref path) = capture_path_clone {
            show_floating_thumbnail(application, path.clone(), kind, on_copy_clipboard);
        }
    });

    app.run_with_args(&["malus-de-screenshot"]);
}

fn show_floating_thumbnail(
    application: &Application,
    path: PathBuf,
    kind: PreviewKind,
    on_copy_clipboard: fn(&Path) -> anyhow::Result<()>,
) {
    let (thumb_w, thumb_h) = compute_thumbnail_size(&path, kind);

    let window = ApplicationWindow::builder()
        .application(application)
        .title("Miniatura de captura")
        .decorated(false)
        .css_classes(["malus-screenshot-thumbnail"])
        .default_width(thumb_w + 20)
        .default_height(thumb_h + 20)
        .build();

    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace("malus-screenshot-thumbnail");
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Bottom, 18);
        window.set_margin(Edge::Right, 18);
    }

    let frame_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["thumbnail-photo-frame"])
        .halign(Align::Center)
        .valign(Align::Center)
        .build();

    let overlay = Overlay::new();
    if kind == PreviewKind::Image {
        let pic = Picture::for_filename(&path);
        pic.set_content_fit(ContentFit::Cover);
        pic.set_can_shrink(true);
        pic.set_size_request(thumb_w, thumb_h);
        pic.add_css_class("thumbnail-photo-image");
        overlay.set_child(Some(&pic));
    } else {
        let video_bg = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(6)
            .halign(Align::Fill)
            .valign(Align::Fill)
            .width_request(thumb_w)
            .height_request(thumb_h)
            .css_classes(["thumbnail-video-placeholder"])
            .build();
        let play_icon = Image::builder()
            .icon_name("media-playback-start-symbolic")
            .pixel_size(34)
            .halign(Align::Center)
            .valign(Align::Center)
            .vexpand(true)
            .css_classes(["thumbnail-video-icon"])
            .build();
        let lbl = Label::builder()
            .label("Grabación lista")
            .css_classes(["thumbnail-video-caption"])
            .margin_bottom(10)
            .build();
        video_bg.append(&play_icon);
        video_bg.append(&lbl);
        overlay.set_child(Some(&video_bg));
    }

    frame_box.append(&overlay);
    frame_box.set_cursor_from_name(Some("pointer"));
    window.set_child(Some(&frame_box));

    let opened_editor = Rc::new(Cell::new(false));
    let hovered = Rc::new(Cell::new(false));

    // Al hacer clic en la miniatura se abre inmediatamente la ventana de previsualización estilo Branches
    let click = GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    {
        let app_c = application.clone();
        let path_c = path.clone();
        let win_c = window.clone();
        let opened_c = opened_editor.clone();
        click.connect_released(move |_, _, _, _| {
            if opened_c.replace(true) {
                return;
            }
            win_c.close();
            open_preview_editor_window(&app_c, path_c.clone(), kind, on_copy_clipboard);
        });
    }
    frame_box.add_controller(click);

    let motion = EventControllerMotion::new();
    {
        let hov = hovered.clone();
        motion.connect_enter(move |_, _, _| hov.set(true));
    }
    {
        let hov = hovered.clone();
        motion.connect_leave(move |_| hov.set(false));
    }
    frame_box.add_controller(motion);

    window.present();

    // Duración de 5 segundos (pausando mientras el puntero esté encima para facilitar el clic)
    let remaining_ticks = Rc::new(Cell::new(THUMBNAIL_DISPLAY_SECS));
    let win_auto = window.clone();
    let opened_auto = opened_editor;
    glib::timeout_add_local(Duration::from_secs(1), move || {
        if opened_auto.get() || win_auto.root().is_none() {
            return ControlFlow::Break;
        }
        if hovered.get() {
            return ControlFlow::Continue;
        }
        let rem = remaining_ticks.get();
        if rem <= 1 {
            win_auto.close();
            ControlFlow::Break
        } else {
            remaining_ticks.set(rem - 1);
            ControlFlow::Continue
        }
    });
}

#[derive(Clone)]
struct ImageViewSlot {
    frame: Rc<RefCell<Option<DecodedRgbaFrame>>>,
    picture: Picture,
    img_frame: Overlay,
    size_spacer: GtkBox,
}

/// Ventana de previsualización idéntica a Quick Look en Branches (`crates/branches/src/quick_look.rs`):
/// - Tarjeta flotante de radio 20 px, margen exterior de 16 px para sombra, animaciones `ql-open` / `ql-closing`
/// - Geometría adaptativa `fit_quicklook_card_size` + cabecera arrastrable de 42 px (`WindowHandle`)
/// - Botones HUD circulares de 18 px (Cerrar + Ampliar/Restaurar), nombre editable inline con tipografía
///   `.branches-quicklook-title`, botones de icono de 26 px (Girar, Marcación, Basurero, Compartir) y píldora OK de 26 px
/// - Escenario de imagen redondeado a 12 px con margen de 8 px sin divisor, o reproductor de video embebido estilo Branches.
fn open_preview_editor_window(
    application: &Application,
    initial_path: PathBuf,
    kind: PreviewKind,
    on_copy_clipboard: fn(&Path) -> anyhow::Result<()>,
) {
    let es = malus_de_common::is_spanish_locale();
    let current_path = Rc::new(RefCell::new(initial_path.clone()));
    let strokes: Rc<RefCell<Vec<MarkupStroke>>> = Rc::new(RefCell::new(Vec::new()));
    let active_tool = Rc::new(Cell::new(MarkupTool::Pen));
    let active_color = Rc::new(Cell::new((0.96, 0.24, 0.22)));
    let markup_enabled = Rc::new(Cell::new(false));
    let rotation_turns = Rc::new(Cell::new(0u8));
    let expanded = Rc::new(Cell::new(false));
    let closing = Rc::new(Cell::new(false));
    let media_handles: Rc<RefCell<Vec<Rc<MediaPlaybackHandle>>>> =
        Rc::new(RefCell::new(Vec::new()));

    let initial_frame = if kind == PreviewKind::Image {
        decode_png_to_rgba(&initial_path)
    } else {
        None
    };
    let initial_img_dims = if kind == PreviewKind::Image {
        initial_frame
            .as_ref()
            .map(|f| (f.width, f.height))
            .or_else(|| read_png_dimensions(&initial_path).map(|(w, h)| (w as i32, h as i32)))
    } else {
        None
    };
    let current_image_dims = Rc::new(Cell::new(initial_img_dims));

    let (max_w, max_h) = primary_monitor_work_bounds();
    let (init_w, init_h) = if let Some((iw, ih)) = initial_img_dims {
        let (cw, ch, _, _) = fit_quicklook_card_size(iw, ih, max_w, max_h);
        (cw, ch)
    } else {
        (max_w.clamp(480, 820), max_h.clamp(380, 600))
    };

    let title_name = initial_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Captura de pantalla")
        .to_string();

    let window = ApplicationWindow::builder()
        .application(application)
        .title(&title_name)
        .decorated(false)
        .css_classes(["branches-quicklook-window"])
        .default_width(init_w + 32)
        .default_height(init_h + 32)
        .build();

    let root_box = GtkBox::new(Orientation::Vertical, 0);
    root_box.add_css_class("branches-quicklook-container");
    root_box.set_overflow(Overflow::Hidden);
    root_box.set_margin_top(16);
    root_box.set_margin_bottom(16);
    root_box.set_margin_start(16);
    root_box.set_margin_end(16);

    let stop_all_media: Rc<dyn Fn()> = {
        let mh = media_handles.clone();
        Rc::new(move || {
            for handle in mh.borrow_mut().drain(..) {
                handle.stop();
            }
        })
    };

    let close_with_animation: Rc<dyn Fn()> = {
        let closing_flag = closing.clone();
        let card = root_box.clone();
        let win = window.clone();
        let stop_media = stop_all_media.clone();
        Rc::new(move || {
            if closing_flag.replace(true) {
                return;
            }
            stop_media();
            card.remove_css_class("ql-open");
            card.add_css_class("ql-closing");
            let win_c = win.clone();
            glib::timeout_add_local_once(Duration::from_millis(140), move || {
                win_c.close();
            });
        })
    };

    let image_slot: Rc<RefCell<Option<ImageViewSlot>>> = Rc::new(RefCell::new(None));

    let apply_card_geometry: Rc<dyn Fn()> = {
        let card = root_box.clone();
        let win = window.clone();
        let exp = expanded.clone();
        let dims_cell = current_image_dims.clone();
        let slot_ref = image_slot.clone();
        Rc::new(move || {
            let (mw, mh) = primary_monitor_work_bounds();
            let is_exp = exp.get();
            if let Some((iw, ih)) = dims_cell.get() {
                if is_exp {
                    card.set_size_request(-1, -1);
                    if let Some(slot) = slot_ref.borrow().as_ref() {
                        slot.size_spacer.set_size_request(-1, -1);
                        slot.img_frame.set_halign(Align::Fill);
                        slot.img_frame.set_valign(Align::Fill);
                    }
                } else {
                    let (cw, ch, fit_w, fit_h) = fit_quicklook_card_size(iw, ih, mw, mh);
                    card.set_size_request(cw, ch);
                    if let Some(slot) = slot_ref.borrow().as_ref() {
                        slot.img_frame.set_halign(Align::Center);
                        slot.img_frame.set_valign(Align::Center);
                        slot.size_spacer.set_size_request(fit_w, fit_h);
                    }
                    win.set_default_size(cw + 32, ch + 32);
                }
            } else if is_exp {
                card.set_size_request(-1, -1);
            } else {
                let cw = mw.clamp(480, 820);
                let ch = mh.clamp(380, 600);
                card.set_size_request(cw, ch);
                win.set_default_size(cw + 32, ch + 32);
            }
        })
    };

    // =====================================================================
    // 1. Barra superior compacta estilo Branches Quick Look (42 px, WindowHandle)
    // =====================================================================
    let header_box = GtkBox::new(Orientation::Horizontal, 6);
    header_box.add_css_class("branches-quicklook-header");

    // Botón HUD Cerrar (18×18 px, igual que Branches)
    let close_icon = Image::from_icon_name("window-close-symbolic");
    close_icon.set_pixel_size(10);
    let close_btn = Button::builder()
        .child(&close_icon)
        .has_frame(false)
        .can_focus(false)
        .valign(Align::Center)
        .css_classes(["branches-quicklook-close-btn", "branches-quicklook-hud-btn"])
        .tooltip_text(if es {
            "Cerrar Vista Rápida (Espacio / Esc)"
        } else {
            "Close Quick Look (Space / Esc)"
        })
        .build();
    {
        let close_fn = close_with_animation.clone();
        close_btn.connect_clicked(move |_| close_fn());
    }
    header_box.append(&close_btn);

    // Botón HUD Ampliar / Restaurar (18×18 px, igual que Branches)
    let expand_icon = Image::from_icon_name("view-fullscreen-symbolic");
    expand_icon.set_pixel_size(10);
    let expand_btn = Button::builder()
        .child(&expand_icon)
        .has_frame(false)
        .can_focus(false)
        .valign(Align::Center)
        .css_classes(["branches-quicklook-hud-btn"])
        .tooltip_text(if es {
            "Ampliar / restaurar"
        } else {
            "Expand / Restore"
        })
        .build();
    {
        let exp = expanded.clone();
        let win = window.clone();
        let card = root_box.clone();
        let icon = expand_icon.clone();
        let apply_geo = apply_card_geometry.clone();
        expand_btn.connect_clicked(move |_| {
            let next = !exp.get();
            exp.set(next);
            icon.set_icon_name(Some(if next {
                "view-restore-symbolic"
            } else {
                "view-fullscreen-symbolic"
            }));
            if next {
                card.set_margin_top(0);
                card.set_margin_bottom(0);
                card.set_margin_start(0);
                card.set_margin_end(0);
                win.fullscreen();
            } else {
                win.unfullscreen();
                card.set_margin_top(16);
                card.set_margin_bottom(16);
                card.set_margin_start(16);
                card.set_margin_end(16);
            }
            apply_geo();
        });
    }
    header_box.append(&expand_btn);

    // Campo editable inline del nombre de archivo con estética idéntica a `.branches-quicklook-title`
    let initial_stem = initial_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Captura de pantalla")
        .to_string();
    let initial_ext = initial_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();

    let name_entry = Entry::builder()
        .text(&initial_stem)
        .placeholder_text("Nombre de la captura")
        .tooltip_text("Haz clic para cambiar el nombre de la captura y pulsa Enter")
        .css_classes(["branches-quicklook-title-entry"])
        .width_chars(initial_stem.chars().count().clamp(10, 34) as i32)
        .max_width_chars(38)
        .margin_start(4)
        .valign(Align::Center)
        .build();
    name_entry.connect_changed(|entry| {
        let len = entry.text().chars().count().clamp(8, 36) as i32;
        entry.set_width_chars(len);
    });
    header_box.append(&name_entry);

    if !initial_ext.is_empty() {
        let ext_lbl = Label::builder()
            .label(&initial_ext)
            .css_classes(["screenshot-editor-ext-label"])
            .valign(Align::Center)
            .build();
        header_box.append(&ext_lbl);
    }

    let rename_feedback = Label::builder()
        .label("")
        .css_classes(["screenshot-editor-feedback"])
        .valign(Align::Center)
        .visible(false)
        .build();
    header_box.append(&rename_feedback);

    // Espaciador elástico dentro de `WindowHandle` para arrastrar la ventana desde el título
    let drag_spacer = GtkBox::new(Orientation::Horizontal, 0);
    drag_spacer.set_hexpand(true);
    header_box.append(&drag_spacer);

    let commit_rename: Rc<dyn Fn() -> bool> = {
        let cur_path = current_path.clone();
        let entry = name_entry.clone();
        let feedback = rename_feedback.clone();
        let win_t = window.clone();
        Rc::new(move || {
            let requested = entry.text().to_string();
            let old = cur_path.borrow().clone();
            match rename_capture_file(&old, &requested) {
                Ok(new_path) => {
                    let changed = new_path != old;
                    *cur_path.borrow_mut() = new_path.clone();
                    if let Some(stem) = new_path.file_stem().and_then(|s| s.to_str()) {
                        if entry.text().as_str() != stem {
                            entry.set_text(stem);
                        }
                    }
                    if let Some(fname) = new_path.file_name().and_then(|s| s.to_str()) {
                        win_t.set_title(Some(fname));
                    }
                    if changed {
                        info!("✏️ Captura renombrada a {}", new_path.display());
                        feedback.set_label("Renombrado");
                        feedback.remove_css_class("error");
                        feedback.set_visible(true);
                        let fb = feedback.clone();
                        glib::timeout_add_local_once(Duration::from_secs(2), move || {
                            fb.set_visible(false);
                        });
                    }
                    true
                }
                Err(err) => {
                    warn!("No se pudo renombrar la captura: {err}");
                    feedback.set_label(&err);
                    feedback.add_css_class("error");
                    feedback.set_visible(true);
                    false
                }
            }
        })
    };

    {
        let commit = commit_rename.clone();
        let win_f = window.clone();
        name_entry.connect_activate(move |_| {
            if commit() {
                GtkWindowExt::set_focus(&win_f, gtk4::Widget::NONE);
            }
        });
    }
    {
        let commit = commit_rename.clone();
        let focus_ctrl = gtk4::EventControllerFocus::new();
        focus_ctrl.connect_leave(move |_| {
            commit();
        });
        name_entry.add_controller(focus_ctrl);
    }

    // Botón Girar a la izquierda (26×26 px, solo imágenes, igual que Branches)
    let rotate_icon = Image::from_icon_name("object-rotate-left-symbolic");
    rotate_icon.set_pixel_size(15);
    let rotate_btn = Button::builder()
        .child(&rotate_icon)
        .has_frame(false)
        .can_focus(false)
        .valign(Align::Center)
        .visible(kind == PreviewKind::Image)
        .css_classes(["branches-quicklook-icon-btn"])
        .tooltip_text(if es {
            "Girar a la izquierda"
        } else {
            "Rotate Left"
        })
        .build();
    header_box.append(&rotate_btn);

    // Botón Marcación / Anotaciones (26×26 px, solo imágenes)
    let markup_icon = Image::from_icon_name("document-edit-symbolic");
    markup_icon.set_pixel_size(15);
    let markup_btn = ToggleButton::builder()
        .child(&markup_icon)
        .has_frame(false)
        .can_focus(false)
        .valign(Align::Center)
        .visible(kind == PreviewKind::Image)
        .css_classes(["branches-quicklook-icon-btn"])
        .tooltip_text("Marcación y anotaciones")
        .build();
    header_box.append(&markup_btn);

    // Botón Basurero (26×26 px)
    let trash_icon = Image::from_icon_name("user-trash-symbolic");
    trash_icon.set_pixel_size(15);
    let trash_btn = Button::builder()
        .child(&trash_icon)
        .has_frame(false)
        .can_focus(false)
        .valign(Align::Center)
        .css_classes(["branches-quicklook-icon-btn"])
        .tooltip_text("Transferir al basurero")
        .build();
    {
        let cur_path = current_path.clone();
        let close_fn = close_with_animation.clone();
        let stop_media = stop_all_media.clone();
        trash_btn.connect_clicked(move |_| {
            stop_media();
            let p = cur_path.borrow().clone();
            match move_file_to_trash(&p) {
                Ok(()) => {
                    info!("🗑️ Captura enviada al basurero: {}", p.display());
                    close_fn();
                }
                Err(err) => error!("{err}"),
            }
        });
    }
    header_box.append(&trash_btn);

    // Botón Compartir (26×26 px con icono `send-to-symbolic` igual que Branches)
    let share_icon = Image::from_icon_name("send-to-symbolic");
    share_icon.set_pixel_size(15);
    let share_btn = MenuButton::builder()
        .child(&share_icon)
        .has_frame(false)
        .can_focus(false)
        .valign(Align::Center)
        .css_classes(["branches-quicklook-icon-btn"])
        .tooltip_text(if es { "Compartir" } else { "Share" })
        .build();

    let share_pop = Popover::builder()
        .has_arrow(false)
        .css_classes(["screenshot-share-popover"])
        .build();
    let share_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();

    let make_share_row = |icon: &str, text: &str| -> Button {
        let b = Button::builder()
            .has_frame(false)
            .css_classes(["screenshot-share-row"])
            .build();
        let row = GtkBox::new(Orientation::Horizontal, 8);
        row.append(&Image::from_icon_name(icon));
        row.append(
            &Label::builder()
                .label(text)
                .xalign(0.0)
                .hexpand(true)
                .build(),
        );
        b.set_child(Some(&row));
        b
    };

    if kind == PreviewKind::Image {
        let copy_row = make_share_row("edit-copy-symbolic", "Copiar imagen");
        let cur_path = current_path.clone();
        let commit = commit_rename.clone();
        let pop = share_pop.clone();
        copy_row.connect_clicked(move |_| {
            pop.popdown();
            commit();
            let p = cur_path.borrow().clone();
            if let Err(err) = on_copy_clipboard(&p) {
                error!("Error al copiar imagen: {err:#}");
            }
        });
        share_box.append(&copy_row);
    }

    let open_row = make_share_row(
        if kind == PreviewKind::Video {
            "media-playback-start-symbolic"
        } else {
            "image-x-generic-symbolic"
        },
        if kind == PreviewKind::Video {
            "Abrir con reproductor predeterminado"
        } else {
            "Abrir con visor predeterminado"
        },
    );
    {
        let cur_path = current_path.clone();
        let commit = commit_rename.clone();
        let pop = share_pop.clone();
        open_row.connect_clicked(move |_| {
            pop.popdown();
            commit();
            let p = cur_path.borrow().clone();
            let _ = Command::new("xdg-open").arg(&p).spawn();
        });
    }
    share_box.append(&open_row);

    let folder_row = make_share_row("folder-symbolic", "Mostrar en Branches");
    {
        let cur_path = current_path.clone();
        let commit = commit_rename.clone();
        let pop = share_pop.clone();
        folder_row.connect_clicked(move |_| {
            pop.popdown();
            commit();
            let p = cur_path.borrow().clone();
            if let Some(parent) = p.parent() {
                let _ = Command::new("malus-de-branches")
                    .arg(parent)
                    .spawn()
                    .or_else(|_| Command::new("xdg-open").arg(parent).spawn());
            }
        });
    }
    share_box.append(&folder_row);

    share_box.append(&Separator::new(Orientation::Horizontal));

    let airdrop_row = make_share_row("network-wireless-symbolic", "AirDrop / Malus Link...");
    {
        let cur_path = current_path.clone();
        let commit = commit_rename.clone();
        let pop = share_pop.clone();
        airdrop_row.connect_clicked(move |_| {
            pop.popdown();
            commit();
            let p = cur_path.borrow().clone();
            let _ = Command::new("malus-de-branches")
                .arg("--share")
                .arg(&p)
                .spawn();
        });
    }
    share_box.append(&airdrop_row);

    share_pop.set_child(Some(&share_box));
    share_btn.set_popover(Some(&share_pop));
    header_box.append(&share_btn);

    // Botón píldora OK (idéntico a `.branches-quicklook-open-pill` de Branches)
    let ok_btn = Button::builder()
        .label("OK")
        .has_frame(false)
        .can_focus(false)
        .css_classes(["branches-quicklook-open-pill"])
        .valign(Align::Center)
        .build();
    header_box.append(&ok_btn);

    let handle = WindowHandle::builder().child(&header_box).build();
    root_box.append(&handle);

    // =====================================================================
    // 2. Sub-barra de Marcación (oculta hasta pulsar el botón de Marcación)
    // =====================================================================
    let markup_bar = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::Center)
        .css_classes(["screenshot-markup-bar"])
        .visible(false)
        .build();

    let pen_btn = ToggleButton::builder()
        .label("Lápiz")
        .active(true)
        .css_classes(["screenshot-markup-tool"])
        .build();
    let rect_btn = ToggleButton::builder()
        .label("Rectángulo")
        .group(&pen_btn)
        .css_classes(["screenshot-markup-tool"])
        .build();
    let arrow_btn = ToggleButton::builder()
        .label("Flecha")
        .group(&pen_btn)
        .css_classes(["screenshot-markup-tool"])
        .build();
    {
        let tool = active_tool.clone();
        pen_btn.connect_toggled(move |b| {
            if b.is_active() {
                tool.set(MarkupTool::Pen);
            }
        });
    }
    {
        let tool = active_tool.clone();
        rect_btn.connect_toggled(move |b| {
            if b.is_active() {
                tool.set(MarkupTool::Rect);
            }
        });
    }
    {
        let tool = active_tool.clone();
        arrow_btn.connect_toggled(move |b| {
            if b.is_active() {
                tool.set(MarkupTool::Arrow);
            }
        });
    }
    markup_bar.append(&pen_btn);
    markup_bar.append(&rect_btn);
    markup_bar.append(&arrow_btn);
    markup_bar.append(&Separator::new(Orientation::Vertical));

    for (css_cls, rgb) in [
        ("markup-dot-red", (0.96, 0.24, 0.22)),
        ("markup-dot-yellow", (0.99, 0.80, 0.08)),
        ("markup-dot-green", (0.20, 0.80, 0.35)),
        ("markup-dot-blue", (0.04, 0.52, 1.00)),
        ("markup-dot-white", (1.0, 1.0, 1.0)),
    ] {
        let dot_btn = Button::builder()
            .css_classes(["screenshot-markup-color-dot", css_cls])
            .valign(Align::Center)
            .build();
        let col = active_color.clone();
        dot_btn.connect_clicked(move |_| col.set(rgb));
        markup_bar.append(&dot_btn);
    }

    markup_bar.append(&Separator::new(Orientation::Vertical));

    let undo_btn = Button::builder()
        .label("Deshacer")
        .css_classes(["screenshot-markup-tool"])
        .build();
    markup_bar.append(&undo_btn);
    root_box.append(&markup_bar);

    // =====================================================================
    // 3. Contenedor de contenido idéntico a Quick Look en Branches
    // =====================================================================
    let content_holder = GtkBox::new(Orientation::Vertical, 0);
    content_holder.set_vexpand(true);
    content_holder.set_hexpand(true);
    content_holder.add_css_class("branches-quicklook-content");

    let markup_canvas = DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .can_target(false)
        .build();

    if kind == PreviewKind::Image {
        let stage = GtkBox::new(Orientation::Vertical, 0);
        stage.set_vexpand(true);
        stage.set_hexpand(true);
        stage.add_css_class("branches-quicklook-image-stage");

        let size_spacer = GtkBox::new(Orientation::Vertical, 0);
        size_spacer.set_halign(Align::Fill);
        size_spacer.set_valign(Align::Fill);

        let img_frame = Overlay::new();
        img_frame.set_halign(Align::Center);
        img_frame.set_valign(Align::Center);
        img_frame.set_vexpand(true);
        img_frame.set_hexpand(true);
        img_frame.set_overflow(Overflow::Hidden);
        img_frame.add_css_class("branches-quicklook-image-frame");
        img_frame.set_child(Some(&size_spacer));

        let picture = if let Some(ref frame) = initial_frame {
            let tex = frame.to_texture();
            Picture::for_paintable(&tex)
        } else {
            Picture::for_filename(&initial_path)
        };
        picture.set_content_fit(ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_vexpand(true);
        picture.set_hexpand(true);
        picture.add_css_class("branches-quicklook-image");
        img_frame.add_overlay(&picture);
        img_frame.set_measure_overlay(&picture, false);

        {
            let strokes_draw = strokes.clone();
            let dims_draw = current_image_dims.clone();
            markup_canvas.set_draw_func(move |_, cr, w, h| {
                let (iw, ih) = dims_draw.get().unwrap_or((w.max(1), h.max(1)));
                let (ox, oy, rw, rh) =
                    image_display_rect(w as f64, h as f64, iw as f64, ih as f64);
                for stroke in strokes_draw.borrow().iter() {
                    draw_stroke_in_rect(cr, stroke, ox, oy, rw, rh, 1.0);
                }
            });
        }

        let drag = GestureDrag::new();
        let drag_origin = Rc::new(Cell::new((0.0_f64, 0.0_f64)));
        {
            let enabled = markup_enabled.clone();
            let strokes_b = strokes.clone();
            let tool = active_tool.clone();
            let color = active_color.clone();
            let canvas = markup_canvas.clone();
            let origin = drag_origin.clone();
            let dims_b = current_image_dims.clone();
            drag.connect_drag_begin(move |_, sx, sy| {
                if !enabled.get() {
                    return;
                }
                origin.set((sx, sy));
                let cw = canvas.width().max(1) as f64;
                let ch = canvas.height().max(1) as f64;
                let (iw, ih) = dims_b.get().unwrap_or((cw as i32, ch as i32));
                let (ox, oy, rw, rh) = image_display_rect(cw, ch, iw as f64, ih as f64);
                let nx = ((sx - ox) / rw).clamp(0.0, 1.0);
                let ny = ((sy - oy) / rh).clamp(0.0, 1.0);
                strokes_b.borrow_mut().push(MarkupStroke {
                    tool: tool.get(),
                    color: color.get(),
                    line_width: 3.5,
                    points: vec![(nx, ny)],
                });
                canvas.queue_draw();
            });
        }
        {
            let enabled = markup_enabled.clone();
            let strokes_u = strokes.clone();
            let canvas = markup_canvas.clone();
            let origin = drag_origin;
            let dims_u = current_image_dims.clone();
            drag.connect_drag_update(move |_, dx, dy| {
                if !enabled.get() {
                    return;
                }
                let (sx, sy) = origin.get();
                let cw = canvas.width().max(1) as f64;
                let ch = canvas.height().max(1) as f64;
                let (iw, ih) = dims_u.get().unwrap_or((cw as i32, ch as i32));
                let (ox, oy, rw, rh) = image_display_rect(cw, ch, iw as f64, ih as f64);
                let nx = ((sx + dx - ox) / rw).clamp(0.0, 1.0);
                let ny = ((sy + dy - oy) / rh).clamp(0.0, 1.0);
                if let Some(last) = strokes_u.borrow_mut().last_mut() {
                    match last.tool {
                        MarkupTool::Pen => last.points.push((nx, ny)),
                        MarkupTool::Rect | MarkupTool::Arrow => {
                            if last.points.len() == 1 {
                                last.points.push((nx, ny));
                            } else if let Some(end) = last.points.last_mut() {
                                *end = (nx, ny);
                            }
                        }
                    }
                }
                canvas.queue_draw();
            });
        }
        markup_canvas.add_controller(drag);
        img_frame.add_overlay(&markup_canvas);
        img_frame.set_measure_overlay(&markup_canvas, false);

        stage.append(&img_frame);
        content_holder.append(&stage);

        *image_slot.borrow_mut() = Some(ImageViewSlot {
            frame: Rc::new(RefCell::new(initial_frame)),
            picture,
            img_frame,
            size_spacer,
        });
    } else {
        // Reproductor de video embebido estilo Quick Look de Branches
        let meta = probe_media_metadata(&initial_path);
        let poster_frame = extract_video_poster_rgba(&initial_path, meta.duration_secs);
        let (play_w, play_h) = fit_video_dimensions(
            meta.width.unwrap_or(720),
            meta.height.unwrap_or(420),
            760,
            440,
        );

        let video_root = GtkBox::new(Orientation::Vertical, 0);
        video_root.set_vexpand(true);
        video_root.set_hexpand(true);

        let stage = GtkBox::new(Orientation::Vertical, 0);
        stage.set_vexpand(true);
        stage.set_hexpand(true);
        stage.set_margin_top(12);
        stage.set_margin_bottom(8);
        stage.set_margin_start(14);
        stage.set_margin_end(14);
        stage.add_css_class("branches-quicklook-video-stage");

        let picture = Picture::new();
        picture.set_content_fit(ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_vexpand(true);
        picture.set_hexpand(true);
        if let Some(frame) = poster_frame {
            let poster = frame.to_texture();
            picture.set_paintable(Some(&poster));
        }
        stage.append(&picture);
        video_root.append(&stage);

        let footer = GtkBox::new(Orientation::Horizontal, 10);
        footer.add_css_class("branches-quicklook-footer");
        footer.set_margin_top(4);
        footer.set_margin_bottom(12);
        footer.set_margin_start(16);
        footer.set_margin_end(16);

        let is_playing = Rc::new(Cell::new(true));
        let is_muted = Rc::new(Cell::new(false));
        let current_handle: Rc<RefCell<Option<Rc<MediaPlaybackHandle>>>> =
            Rc::new(RefCell::new(None));

        let play_btn = Button::builder()
            .icon_name("media-playback-stop-symbolic")
            .label("Detener")
            .has_frame(true)
            .can_focus(false)
            .css_classes(["branches-quicklook-btn"])
            .build();

        let mute_btn = Button::builder()
            .icon_name("audio-volume-high-symbolic")
            .has_frame(false)
            .can_focus(false)
            .tooltip_text("Silenciar / Activar audio")
            .visible(meta.has_audio)
            .build();

        let mut info_parts = Vec::new();
        if let (Some(w), Some(h)) = (meta.width, meta.height) {
            info_parts.push(format!("{}×{}", w, h));
        }
        if let Some(ref d) = meta.duration_str {
            info_parts.push(d.clone());
        }
        if let Some(ref c) = meta.codec {
            info_parts.push(c.clone());
        }
        let meta_lbl = Label::builder()
            .label(&info_parts.join(" • "))
            .halign(Align::Start)
            .hexpand(true)
            .css_classes(["dim-label", "caption"])
            .build();

        footer.append(&play_btn);
        footer.append(&mute_btn);
        footer.append(&meta_lbl);
        video_root.append(&footer);

        let start_playback: Rc<dyn Fn()> = Rc::new({
            let cur_path = current_path.clone();
            let picture = picture.clone();
            let media_handles = media_handles.clone();
            let is_playing = is_playing.clone();
            let is_muted = is_muted.clone();
            let current_handle = current_handle.clone();
            let play_btn = play_btn.clone();
            let has_audio = meta.has_audio;
            move || {
                if let Some(h) = current_handle.borrow_mut().take() {
                    h.stop();
                }
                is_playing.set(true);
                play_btn.set_icon_name("media-playback-stop-symbolic");
                play_btn.set_label("Detener");

                let is_playing_end = is_playing.clone();
                let play_btn_end = play_btn.clone();
                let on_end: Rc<dyn Fn()> = Rc::new(move || {
                    is_playing_end.set(false);
                    play_btn_end.set_icon_name("media-playback-start-symbolic");
                    play_btn_end.set_label("Reproducir");
                });

                let path = cur_path.borrow().clone();
                let h = spawn_video_playback(
                    &path,
                    play_w,
                    play_h,
                    has_audio && !is_muted.get(),
                    &picture,
                    &media_handles,
                    on_end,
                );
                *current_handle.borrow_mut() = Some(h);
            }
        });

        // Diferir arranque de ffmpeg/ffplay hasta completar la transición de apertura (185 ms, igual que Branches)
        {
            let sp_delayed = start_playback.clone();
            let closing_check = closing.clone();
            glib::timeout_add_local_once(Duration::from_millis(185), move || {
                if !closing_check.get() {
                    sp_delayed();
                }
            });
        }

        {
            let sp_toggle = start_playback.clone();
            let ch_toggle = current_handle.clone();
            let ip_toggle = is_playing.clone();
            let pb_toggle = play_btn.clone();
            play_btn.connect_clicked(move |_| {
                if ip_toggle.get() {
                    if let Some(h) = ch_toggle.borrow_mut().take() {
                        h.stop();
                    }
                    ip_toggle.set(false);
                    pb_toggle.set_icon_name("media-playback-start-symbolic");
                    pb_toggle.set_label("Reproducir");
                } else {
                    sp_toggle();
                }
            });
        }

        {
            let im_mute = is_muted.clone();
            let mb_mute = mute_btn.clone();
            let sp_mute = start_playback;
            let ip_mute = is_playing;
            mute_btn.connect_clicked(move |_| {
                let next_muted = !im_mute.get();
                im_mute.set(next_muted);
                mb_mute.set_icon_name(if next_muted {
                    "audio-volume-muted-symbolic"
                } else {
                    "audio-volume-high-symbolic"
                });
                if ip_mute.get() {
                    sp_mute();
                }
            });
        }

        content_holder.append(&video_root);
    }

    root_box.append(&content_holder);
    window.set_child(Some(&root_box));

    // Acción Girar a la izquierda (90° CCW, igual que Branches + rota trazos de marcación)
    {
        let slot_rot = image_slot.clone();
        let dims_rot = current_image_dims.clone();
        let turns_rot = rotation_turns.clone();
        let strokes_rot = strokes.clone();
        let canvas_rot = markup_canvas.clone();
        let apply_geo = apply_card_geometry.clone();
        rotate_btn.connect_clicked(move |_| {
            let new_dims = {
                let slot_opt = slot_rot.borrow();
                let Some(slot) = slot_opt.as_ref() else {
                    return;
                };
                let mut frame_guard = slot.frame.borrow_mut();
                let Some(cur_frame) = frame_guard.as_ref() else {
                    return;
                };
                let rotated = rotate_rgba_frame_ccw(cur_frame);
                let dims = (rotated.width, rotated.height);
                let tex = rotated.to_texture();
                slot.picture.set_paintable(Some(&tex));
                *frame_guard = Some(rotated);
                dims
            };
            turns_rot.set((turns_rot.get() + 1) % 4);
            for stroke in strokes_rot.borrow_mut().iter_mut() {
                for pt in &mut stroke.points {
                    let (nx, ny) = *pt;
                    *pt = (ny, 1.0 - nx);
                }
            }
            dims_rot.set(Some(new_dims));
            apply_geo();
            canvas_rot.queue_draw();
        });
    }

    // Toggle barra de Marcación
    {
        let bar = markup_bar.clone();
        let enabled = markup_enabled.clone();
        let canvas = markup_canvas.clone();
        markup_btn.connect_toggled(move |b| {
            let active = b.is_active();
            enabled.set(active);
            bar.set_visible(active);
            canvas.set_can_target(active);
            canvas.set_cursor_from_name(if active { Some("crosshair") } else { None });
        });
    }

    // Deshacer último trazo
    {
        let strokes_undo = strokes.clone();
        let canvas = markup_canvas.clone();
        undo_btn.connect_clicked(move |_| {
            strokes_undo.borrow_mut().pop();
            canvas.queue_draw();
        });
    }

    // Botón OK: guarda renombre + rotaciones + anotaciones en disco, actualiza portapapeles y cierra
    {
        let commit = commit_rename.clone();
        let cur_path = current_path.clone();
        let strokes_ok = strokes.clone();
        let turns_ok = rotation_turns.clone();
        let close_fn = close_with_animation.clone();
        ok_btn.connect_clicked(move |_| {
            if !commit() {
                return;
            }
            let p = cur_path.borrow().clone();
            let drawn = strokes_ok.borrow().clone();
            let turns = turns_ok.get() % 4;
            if kind == PreviewKind::Image && (turns != 0 || !drawn.is_empty()) {
                if let Err(err) = save_image_edits_to_png(&p, turns, &drawn) {
                    error!("Error al guardar ediciones: {err}");
                } else {
                    let _ = on_copy_clipboard(&p);
                }
            }
            close_fn();
        });
    }

    // Atajos de teclado estilo Quick Look: Esc / Espacio (cuando no se edita el nombre) y Ctrl/Super+Z
    let keys = EventControllerKey::new();
    keys.set_propagation_phase(PropagationPhase::Capture);
    {
        let close_fn = close_with_animation.clone();
        let strokes_k = strokes;
        let canvas_k = markup_canvas;
        let enabled_k = markup_enabled;
        let entry_k = name_entry.clone();
        keys.connect_key_pressed(move |_, key, _, state| {
            let has_mod = state.intersects(ModifierType::CONTROL_MASK | ModifierType::ALT_MASK);
            if !has_mod && key == gdk::Key::Escape {
                close_fn();
                return glib::Propagation::Stop;
            }
            if !has_mod
                && matches!(key, gdk::Key::space | gdk::Key::KP_Space)
                && !entry_k.has_focus()
            {
                close_fn();
                return glib::Propagation::Stop;
            }
            let is_mod = state.intersects(
                ModifierType::CONTROL_MASK | ModifierType::SUPER_MASK | ModifierType::META_MASK,
            );
            if is_mod && enabled_k.get() && (key == gdk::Key::z || key == gdk::Key::Z) {
                strokes_k.borrow_mut().pop();
                canvas_k.queue_draw();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    window.add_controller(keys);

    {
        let stop_media = stop_all_media;
        window.connect_close_request(move |_| {
            stop_media();
            glib::Propagation::Proceed
        });
    }

    apply_card_geometry();
    window.present();
    // Evitar que el Entry capture foco al abrir para lucir idéntico al título de Branches Quick Look
    GtkWindowExt::set_focus(&window, gtk4::Widget::NONE);

    // Animación de entrada macOS idéntica a Branches Quick Look
    root_box.add_tick_callback(move |widget, _| {
        widget.add_css_class("ql-open");
        ControlFlow::Break
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn thumbnail_duration_is_5_seconds() {
        assert_eq!(THUMBNAIL_DISPLAY_SECS, 5);
    }

    #[test]
    fn quicklook_preview_matches_branches_styles_and_adaptive_geometry() {
        assert!(THUMBNAIL_CSS.contains("window.branches-quicklook-window"));
        assert!(THUMBNAIL_CSS.contains(".branches-quicklook-container"));
        assert!(THUMBNAIL_CSS.contains("border-radius: 20px;"));
        assert!(THUMBNAIL_CSS.contains(".branches-quicklook-container.ql-open"));
        assert!(THUMBNAIL_CSS.contains(".branches-quicklook-container.ql-closing"));
        assert!(THUMBNAIL_CSS.contains(".branches-quicklook-header"));
        assert!(THUMBNAIL_CSS.contains("min-height: 42px;"));
        assert!(THUMBNAIL_CSS.contains("button.branches-quicklook-hud-btn"));
        assert!(THUMBNAIL_CSS.contains("button.branches-quicklook-icon-btn"));
        assert!(THUMBNAIL_CSS.contains("button.branches-quicklook-open-pill"));
        assert!(THUMBNAIL_CSS.contains(".branches-quicklook-image-stage"));
        assert!(THUMBNAIL_CSS.contains(".branches-quicklook-image-frame"));
        assert!(THUMBNAIL_CSS.contains(".branches-quicklook-video-stage"));

        let (cw, ch, fw, fh) = fit_quicklook_card_size(1200, 750, 900, 700);
        assert!(cw <= 900 && ch <= 700);
        assert!(fw > fh);
        let (pw, ph, _, _) = fit_quicklook_card_size(520, 900, 900, 720);
        assert!(ph > pw);
    }

    #[test]
    fn rename_capture_file_preserves_extension_and_sanitizes_name() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("malus-rename-test-{nonce}"));
        fs::create_dir_all(&dir).unwrap();
        let original = dir.join("Captura de pantalla-1.png");
        fs::write(&original, b"png-data").unwrap();

        let renamed = rename_capture_file(&original, "Mi Captura/Final").unwrap();
        assert_eq!(renamed, dir.join("Mi Captura-Final.png"));
        assert!(renamed.is_file());
        assert!(!original.exists());

        // Evita duplicar la extensión si el usuario la escribe
        let renamed2 = rename_capture_file(&renamed, "Reporte.png").unwrap();
        assert_eq!(renamed2, dir.join("Reporte.png"));

        // Rechaza nombres vacíos
        assert!(rename_capture_file(&renamed2, "   ").is_err());

        let _ = fs::remove_dir_all(&dir);
    }
}
