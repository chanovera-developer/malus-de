//! Live Text (Texto en vivo) estilo macOS para Quick Look en Malus DE.
//!
//! Ejecuta OCR local (`tesseract ... tsv`) de forma asíncrona en segundo plano,
//! obtiene las cajas normalizadas de cada palabra y línea, y superpone un
//! controlador GTK4 interactivo sobre la imagen para:
//! - Cambiar el cursor a `text` (I-beam) al pasar sobre texto detectado;
//! - Seleccionar palabras y párrafos directamente arrastrando sobre la imagen;
//! - Mostrar el botón flotante discreto de Live Text en la esquina inferior derecha
//!   (con resaltado suave de todos los bloques y acción rápida «Copiar» / «Copiar todo»);
//! - Copiar con `Ctrl+C` / `Super+C` o menú contextual, mostrando un toast «Texto copiado».

use gtk4::gdk;
use gtk4::glib::{self, ControlFlow};
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, DrawingArea, EventControllerMotion, GestureClick, GestureDrag,
    Label, Orientation, Overlay, Popover,
};
use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;

/// Palabra reconocida por Tesseract con caja normalizada `0.0..=1.0` respecto a la imagen original.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrWord {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub block_num: u32,
    pub par_num: u32,
    pub line_num: u32,
    pub word_num: u32,
    pub confidence: f32,
}

impl OcrWord {
    /// Transforma las coordenadas normalizadas de la palabra tras `turns` rotaciones de 90° antihorarias.
    pub fn rotated_rect(&self, turns: u8) -> (f64, f64, f64, f64) {
        match turns % 4 {
            0 => (self.x, self.y, self.w, self.h),
            // 90° CCW: (x, y) -> (y, 1 - x - w)
            1 => (self.y, (1.0 - self.x - self.w).max(0.0), self.h, self.w),
            // 180°: (x, y) -> (1 - x - w, 1 - y - h)
            2 => (
                (1.0 - self.x - self.w).max(0.0),
                (1.0 - self.y - self.h).max(0.0),
                self.w,
                self.h,
            ),
            // 270° CCW (90° CW): (x, y) -> (1 - y - h, x)
            _ => ((1.0 - self.y - self.h).max(0.0), self.x, self.h, self.w),
        }
    }
}

/// Documento OCR estructurado de una imagen.
#[derive(Debug, Clone, Default)]
pub struct LiveTextDocument {
    pub words: Vec<OcrWord>,
}

impl LiveTextDocument {
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Extrae el texto en el rango inclusivo `[start, end]` respetando espacios, saltos de línea y párrafos.
    pub fn extract_range(&self, start: usize, end: usize) -> String {
        if self.words.is_empty() {
            return String::new();
        }
        let lo = start.min(end).min(self.words.len() - 1);
        let hi = start.max(end).min(self.words.len() - 1);
        let mut out = String::new();
        let mut prev: Option<&OcrWord> = None;

        for word in &self.words[lo..=hi] {
            if let Some(p) = prev {
                if p.block_num != word.block_num || p.par_num != word.par_num {
                    out.push_str("\n\n");
                } else if p.line_num != word.line_num {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
            }
            out.push_str(&word.text);
            prev = Some(word);
        }
        out
    }

    pub fn all_text(&self) -> String {
        if self.words.is_empty() {
            String::new()
        } else {
            self.extract_range(0, self.words.len() - 1)
        }
    }

    /// Devuelve el índice de palabra bajo `(nx, ny)` en coordenadas normalizadas tras `turns` rotaciones CCW.
    pub fn hit_test(&self, nx: f64, ny: f64, pad_x: f64, pad_y: f64, turns: u8) -> Option<usize> {
        for (i, w) in self.words.iter().enumerate() {
            let (wx, wy, ww, wh) = w.rotated_rect(turns);
            if nx >= wx - pad_x
                && nx <= wx + ww + pad_x
                && ny >= wy - pad_y
                && ny <= wy + wh + pad_y
            {
                return Some(i);
            }
        }
        None
    }

    /// Encuentra la palabra más cercana al puntero durante un arrastre de selección,
    /// priorizando palabras en la misma banda horizontal de línea.
    pub fn nearest_word_index(&self, nx: f64, ny: f64, turns: u8) -> Option<usize> {
        if self.words.is_empty() {
            return None;
        }
        let mut best_idx = 0usize;
        let mut best_score = f64::INFINITY;

        for (i, w) in self.words.iter().enumerate() {
            let (wx, wy, ww, wh) = w.rotated_rect(turns);
            let cx = wx + ww * 0.5;
            let cy = wy + wh * 0.5;
            let dx = if nx < wx {
                wx - nx
            } else if nx > wx + ww {
                nx - (wx + ww)
            } else {
                0.0
            };
            let dy = if ny < wy {
                wy - ny
            } else if ny > wy + wh {
                ny - (wy + wh)
            } else {
                0.0
            };
            // Penalizar más la distancia vertical para que la selección recorra líneas limpiamente
            let score = dy * dy * 16.0 + dx * dx + ((cx - nx).abs() * 0.001) + ((cy - ny).abs() * 0.001);
            if score < best_score {
                best_score = score;
                best_idx = i;
            }
        }
        Some(best_idx)
    }
}

/// Parsea la salida TSV estándar de Tesseract (`tesseract <img.png> stdout tsv`).
pub fn parse_tesseract_tsv(tsv: &str, img_w: f64, img_h: f64) -> LiveTextDocument {
    if img_w <= 1.0 || img_h <= 1.0 {
        return LiveTextDocument::default();
    }
    let mut words = Vec::new();
    for line in tsv.lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 12 {
            continue;
        }
        let Ok(level) = cols[0].parse::<u32>() else {
            continue;
        };
        if level != 5 {
            continue;
        }
        let text = cols[11].trim();
        if text.is_empty() {
            continue;
        }
        let conf = cols[10].parse::<f32>().unwrap_or(-1.0);
        // Filtrar artefactos de muy baja confianza sobre fotografías o degradados
        if conf < 25.0 {
            continue;
        }
        let left = cols[6].parse::<f64>().unwrap_or(0.0);
        let top = cols[7].parse::<f64>().unwrap_or(0.0);
        let width = cols[8].parse::<f64>().unwrap_or(0.0);
        let height = cols[9].parse::<f64>().unwrap_or(0.0);
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        words.push(OcrWord {
            text: text.to_string(),
            x: (left / img_w).clamp(0.0, 1.0),
            y: (top / img_h).clamp(0.0, 1.0),
            w: (width / img_w).clamp(0.0, 1.0),
            h: (height / img_h).clamp(0.0, 1.0),
            block_num: cols[2].parse().unwrap_or(0),
            par_num: cols[3].parse().unwrap_or(0),
            line_num: cols[4].parse().unwrap_or(0),
            word_num: cols[5].parse().unwrap_or(0),
            confidence: conf,
        });
    }
    LiveTextDocument { words }
}

static CACHED_TESSERACT_LANG: OnceLock<Option<String>> = OnceLock::new();

fn detect_tesseract_lang() -> Option<String> {
    CACHED_TESSERACT_LANG
        .get_or_init(|| {
            let out = Command::new("tesseract").arg("--list-langs").output().ok()?;
            let combined = format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            let langs: Vec<&str> = combined
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with("List of available"))
                .collect();
            let has_spa = langs.iter().any(|&l| l == "spa");
            let has_eng = langs.iter().any(|&l| l == "eng");
            match (has_spa, has_eng) {
                (true, true) => Some("spa+eng".to_string()),
                (true, false) => Some("spa".to_string()),
                (false, true) => Some("eng".to_string()),
                (false, false) => langs.first().map(|s| (*s).to_string()),
            }
        })
        .clone()
}

/// Ejecuta Tesseract TSV sobre un archivo de imagen (con escalado 2× automático si la imagen es pequeña).
fn run_tesseract_tsv_for_file(path: &Path, img_w: i32, img_h: i32) -> Option<LiveTextDocument> {
    if img_w <= 2 || img_h <= 2 || !path.is_file() {
        return None;
    }
    let lang = detect_tesseract_lang()?;

    // Si la imagen es pequeña (< 1000 px), escalarla 2× mejora notablemente la precisión en fuentes UI
    let mut temp_file: Option<PathBuf> = None;
    let (ocr_path, eff_w, eff_h) = if img_w < 1000 && img_h < 1000 {
        if let Ok(pb) = gtk4::gdk_pixbuf::Pixbuf::from_file(path) {
            if let Some(scaled) =
                pb.scale_simple(img_w * 2, img_h * 2, gtk4::gdk_pixbuf::InterpType::Bilinear)
            {
                let nonce = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                let tmp = std::env::temp_dir()
                    .join(format!("malus-livetext-{}-{nonce}.png", std::process::id()));
                if scaled.savev(&tmp, "png", &[]).is_ok() {
                    temp_file = Some(tmp.clone());
                    (tmp, (img_w * 2) as f64, (img_h * 2) as f64)
                } else {
                    (path.to_path_buf(), img_w as f64, img_h as f64)
                }
            } else {
                (path.to_path_buf(), img_w as f64, img_h as f64)
            }
        } else {
            (path.to_path_buf(), img_w as f64, img_h as f64)
        }
    } else {
        (path.to_path_buf(), img_w as f64, img_h as f64)
    };

    let output = Command::new("tesseract")
        .arg(&ocr_path)
        .arg("stdout")
        .args(["-l", &lang, "--psm", "3", "tsv"])
        .stderr(Stdio::null())
        .output()
        .ok();

    if let Some(tmp) = temp_file {
        let _ = std::fs::remove_file(tmp);
    }

    let output = output?;
    if !output.status.success() {
        return None;
    }
    let tsv = String::from_utf8_lossy(&output.stdout);
    let doc = parse_tesseract_tsv(&tsv, eff_w, eff_h);
    (!doc.is_empty()).then_some(doc)
}

/// Copia texto UTF-8 al portapapeles de Wayland (usando el portapapeles de GDK y `wl-copy` como respaldo).
fn copy_plain_text_to_clipboard(text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(display) = gdk::Display::default() {
        display.clipboard().set_text(text);
    }
    let owned = text.to_string();
    std::thread::spawn(move || {
        if let Ok(mut child) = Command::new("wl-copy")
            .args(["--type", "text/plain;charset=utf-8"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(owned.as_bytes());
                let _ = stdin.flush();
            }
            let _ = child.wait();
        }
    });
}

/// Calcula el rectángulo efectivo de la imagen renderizada dentro del lienzo con `ContentFit::ScaleDown`.
pub fn fitted_image_rect(
    canvas_w: f64,
    canvas_h: f64,
    img_w: f64,
    img_h: f64,
    turns: u8,
) -> (f64, f64, f64, f64) {
    if canvas_w <= 1.0 || canvas_h <= 1.0 || img_w <= 1.0 || img_h <= 1.0 {
        return (0.0, 0.0, canvas_w.max(1.0), canvas_h.max(1.0));
    }
    let (ew, eh) = if turns % 2 == 1 {
        (img_h, img_w)
    } else {
        (img_w, img_h)
    };
    let scale = (canvas_w / ew).min(canvas_h / eh).min(1.0);
    let rw = ew * scale;
    let rh = eh * scale;
    let ox = (canvas_w - rw) * 0.5;
    let oy = (canvas_h - rh) * 0.5;
    (ox, oy, rw, rh)
}

fn draw_rounded_pill(cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -pi * 0.5, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, pi * 0.5);
    cr.arc(x + r, y + h - r, r, pi * 0.5, pi);
    cr.arc(x + r, y + r, r, pi, pi * 1.5);
    cr.close_path();
}

/// Controlador interactivo de Live Text acoplado a un `Overlay` de imagen en Quick Look.
#[derive(Clone)]
pub struct LiveTextController {
    doc: Rc<RefCell<LiveTextDocument>>,
    selection: Rc<Cell<Option<(usize, usize)>>>,
    highlight_all: Rc<Cell<bool>>,
    interaction_enabled: Rc<Cell<bool>>,
    rotation_turns: Rc<Cell<u8>>,
    canvas: DrawingArea,
    copy_pill_btn: Button,
    live_btn: Button,
    toast_label: Label,
    toast_gen: Rc<Cell<u64>>,
}

impl LiveTextController {
    /// Adjunta Live Text al `img_frame: Overlay` y lanza el reconocimiento OCR asíncrono en segundo plano.
    pub fn attach(img_frame: &Overlay, image_path: &Path, img_w: i32, img_h: i32) -> Self {
        let es = crate::is_spanish_locale();
        let doc = Rc::new(RefCell::new(LiveTextDocument::default()));
        let selection: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
        let highlight_all = Rc::new(Cell::new(false));
        let interaction_enabled = Rc::new(Cell::new(true));
        let rotation_turns = Rc::new(Cell::new(0u8));
        let toast_gen = Rc::new(Cell::new(0u64));
        let base_dims = (img_w.max(1) as f64, img_h.max(1) as f64);

        let canvas = DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .can_target(false) // Se habilita en cuanto el OCR detecte texto
            .build();

        // Dibujo de resaltados Live Text (bloques sutiles al activar botón + selección con color de acento)
        {
            let doc_d = doc.clone();
            let sel_d = selection.clone();
            let hi_d = highlight_all.clone();
            let rot_d = rotation_turns.clone();
            canvas.set_draw_func(move |_, cr, cw, ch| {
                let d = doc_d.borrow();
                if d.is_empty() {
                    return;
                }
                let turns = rot_d.get();
                let (ox, oy, rw, rh) =
                    fitted_image_rect(cw as f64, ch as f64, base_dims.0, base_dims.1, turns);
                let (ar, ag, ab) = crate::config::load_accent_color();

                // 1. Si el usuario pulsó el botón Live Text, resaltar suavemente cada línea detectada
                if hi_d.get() {
                    let mut i = 0;
                    while i < d.words.len() {
                        let first = &d.words[i];
                        let (mut min_x, mut min_y, mut max_x, mut max_y) = {
                            let (x, y, w, h) = first.rotated_rect(turns);
                            (x, y, x + w, y + h)
                        };
                        let key = (first.block_num, first.par_num, first.line_num);
                        let mut j = i + 1;
                        while j < d.words.len() {
                            let w = &d.words[j];
                            if (w.block_num, w.par_num, w.line_num) != key {
                                break;
                            }
                            let (wx, wy, ww, wh) = w.rotated_rect(turns);
                            min_x = min_x.min(wx);
                            min_y = min_y.min(wy);
                            max_x = max_x.max(wx + ww);
                            max_y = max_y.max(wy + wh);
                            j += 1;
                        }
                        let rx = ox + min_x * rw - 2.5;
                        let ry = oy + min_y * rh - 1.5;
                        let r_w = (max_x - min_x) * rw + 5.0;
                        let r_h = (max_y - min_y) * rh + 3.0;
                        draw_rounded_pill(cr, rx, ry, r_w, r_h, 4.0);
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.24);
                        let _ = cr.fill_preserve();
                        cr.set_source_rgba(ar, ag, ab, 0.45);
                        cr.set_line_width(1.0);
                        let _ = cr.stroke();
                        i = j;
                    }
                }

                // 2. Resaltado continuo por línea de la selección activa (estilo macOS Live Text)
                if let Some((a, b)) = sel_d.get() {
                    let lo = a.min(b).min(d.words.len().saturating_sub(1));
                    let hi = a.max(b).min(d.words.len().saturating_sub(1));
                    let mut i = lo;
                    while i <= hi {
                        let first = &d.words[i];
                        let (mut min_x, mut min_y, mut max_x, mut max_y) = {
                            let (x, y, w, h) = first.rotated_rect(turns);
                            (x, y, x + w, y + h)
                        };
                        let key = (first.block_num, first.par_num, first.line_num);
                        let mut j = i + 1;
                        while j <= hi {
                            let w = &d.words[j];
                            if (w.block_num, w.par_num, w.line_num) != key {
                                break;
                            }
                            let (wx, wy, ww, wh) = w.rotated_rect(turns);
                            min_x = min_x.min(wx);
                            min_y = min_y.min(wy);
                            max_x = max_x.max(wx + ww);
                            max_y = max_y.max(wy + wh);
                            j += 1;
                        }
                        let rx = ox + min_x * rw - 2.0;
                        let ry = oy + min_y * rh - 1.5;
                        let r_w = (max_x - min_x) * rw + 4.0;
                        let r_h = (max_y - min_y) * rh + 3.0;
                        draw_rounded_pill(cr, rx, ry, r_w, r_h, 4.0);
                        cr.set_source_rgba(ar, ag, ab, 0.36);
                        let _ = cr.fill_preserve();
                        cr.set_source_rgba(ar, ag, ab, 0.72);
                        cr.set_line_width(1.0);
                        let _ = cr.stroke();
                        i = j;
                    }
                }
            });
        }

        // Controles flotantes estilo macOS en la esquina inferior derecha (ocultos hasta detectar texto)
        let hud_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .halign(Align::End)
            .valign(Align::End)
            .margin_end(12)
            .margin_bottom(12)
            .visible(false)
            .build();

        let copy_pill_btn = Button::builder()
            .label(if es { "Copiar todo" } else { "Copy All" })
            .has_frame(false)
            .can_focus(false)
            .visible(false)
            .build();
        copy_pill_btn.set_cursor_from_name(Some("pointer"));
        apply_inline_hud_pill_css(&copy_pill_btn, false);

        // Botón icono Live Text (visor + 3 líneas de texto, estilo macOS)
        let live_icon = DrawingArea::builder()
            .content_width(18)
            .content_height(18)
            .halign(Align::Center)
            .valign(Align::Center)
            .build();
        {
            let hi_i = highlight_all.clone();
            live_icon.set_draw_func(move |_, cr, w, h| {
                let cx = w as f64 * 0.5;
                let cy = h as f64 * 0.5;
                let half = 6.8;
                let leg = 2.8;
                let active = hi_i.get();
                if active {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.98);
                } else {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.92);
                }
                cr.set_line_width(1.45);
                cr.set_line_cap(gtk4::cairo::LineCap::Round);
                cr.set_line_join(gtk4::cairo::LineJoin::Round);
                for (sx, sy) in [(-1.0_f64, -1.0_f64), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    let ex = cx + sx * half;
                    let ey = cy + sy * half;
                    cr.move_to(ex - sx * leg, ey);
                    cr.line_to(ex, ey);
                    cr.line_to(ex, ey - sy * leg);
                    let _ = cr.stroke();
                }
                // 3 líneas horizontales estilo Live Text de macOS
                cr.set_line_width(1.3);
                for (dy, lw) in [(-2.6_f64, 3.6_f64), (0.0, 3.6), (2.6, 2.2)] {
                    cr.move_to(cx - 3.6, cy + dy);
                    cr.line_to(cx - 3.6 + lw * 2.0, cy + dy);
                    let _ = cr.stroke();
                }
            });
        }

        let live_btn = Button::builder()
            .child(&live_icon)
            .has_frame(false)
            .can_focus(false)
            .tooltip_text(if es {
                "Texto en vivo (Live Text): resalta y copia texto de la imagen"
            } else {
                "Live Text: highlight and copy text from image"
            })
            .build();
        live_btn.set_size_request(30, 30);
        live_btn.set_cursor_from_name(Some("pointer"));
        apply_inline_hud_pill_css(&live_btn, false);

        hud_box.append(&copy_pill_btn);
        hud_box.append(&live_btn);

        // Toast discreto «Texto copiado» en la parte inferior central de la imagen
        let toast_label = Label::builder()
            .label(if es { "Texto copiado" } else { "Text Copied" })
            .halign(Align::Center)
            .valign(Align::End)
            .margin_bottom(14)
            .can_target(false)
            .visible(false)
            .build();
        apply_toast_css(&toast_label);

        let controller = Self {
            doc: doc.clone(),
            selection: selection.clone(),
            highlight_all: highlight_all.clone(),
            interaction_enabled: interaction_enabled.clone(),
            rotation_turns: rotation_turns.clone(),
            canvas: canvas.clone(),
            copy_pill_btn: copy_pill_btn.clone(),
            live_btn: live_btn.clone(),
            toast_label: toast_label.clone(),
            toast_gen: toast_gen.clone(),
        };

        // Hover: cursor I-beam cuando el puntero pasa sobre texto detectado
        let motion = EventControllerMotion::new();
        {
            let doc_m = doc.clone();
            let canvas_m = canvas.clone();
            let enabled_m = interaction_enabled.clone();
            let rot_m = rotation_turns.clone();
            motion.connect_motion(move |_, x, y| {
                if !enabled_m.get() {
                    canvas_m.set_cursor_from_name(None);
                    return;
                }
                let d = doc_m.borrow();
                if d.is_empty() {
                    canvas_m.set_cursor_from_name(None);
                    return;
                }
                let cw = canvas_m.width() as f64;
                let ch = canvas_m.height() as f64;
                let turns = rot_m.get();
                let (ox, oy, rw, rh) = fitted_image_rect(cw, ch, base_dims.0, base_dims.1, turns);
                let nx = (x - ox) / rw.max(1.0);
                let ny = (y - oy) / rh.max(1.0);
                let pad_x = 5.0 / rw.max(1.0);
                let pad_y = 4.0 / rh.max(1.0);
                if d.hit_test(nx, ny, pad_x, pad_y, turns).is_some() {
                    canvas_m.set_cursor_from_name(Some("text"));
                } else {
                    canvas_m.set_cursor_from_name(None);
                }
            });
        }
        {
            let canvas_l = canvas.clone();
            motion.connect_leave(move |_| {
                canvas_l.set_cursor_from_name(None);
            });
        }
        canvas.add_controller(motion);

        // Arrastre para seleccionar texto directamente sobre la imagen
        let drag = GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let drag_anchor: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let drag_origin: Rc<Cell<(f64, f64)>> = Rc::new(Cell::new((0.0, 0.0)));
        {
            let doc_b = doc.clone();
            let sel_b = selection.clone();
            let anchor_b = drag_anchor.clone();
            let origin_b = drag_origin.clone();
            let canvas_b = canvas.clone();
            let enabled_b = interaction_enabled.clone();
            let rot_b = rotation_turns.clone();
            let ctrl_b = controller.clone();
            drag.connect_drag_begin(move |_, sx, sy| {
                if !enabled_b.get() {
                    anchor_b.set(None);
                    return;
                }
                origin_b.set((sx, sy));
                let d = doc_b.borrow();
                if d.is_empty() {
                    anchor_b.set(None);
                    return;
                }
                let cw = canvas_b.width() as f64;
                let ch = canvas_b.height() as f64;
                let turns = rot_b.get();
                let (ox, oy, rw, rh) = fitted_image_rect(cw, ch, base_dims.0, base_dims.1, turns);
                let nx = (sx - ox) / rw.max(1.0);
                let ny = (sy - oy) / rh.max(1.0);
                let pad_x = 8.0 / rw.max(1.0);
                let pad_y = 6.0 / rh.max(1.0);
                if let Some(idx) = d.hit_test(nx, ny, pad_x, pad_y, turns) {
                    anchor_b.set(Some(idx));
                    sel_b.set(Some((idx, idx)));
                    ctrl_b.sync_copy_pill();
                    canvas_b.queue_draw();
                } else {
                    anchor_b.set(None);
                    if sel_b.take().is_some() {
                        ctrl_b.sync_copy_pill();
                        canvas_b.queue_draw();
                    }
                }
            });
        }
        {
            let doc_u = doc.clone();
            let sel_u = selection.clone();
            let anchor_u = drag_anchor.clone();
            let origin_u = drag_origin;
            let canvas_u = canvas.clone();
            let rot_u = rotation_turns.clone();
            let ctrl_u = controller.clone();
            drag.connect_drag_update(move |_, dx, dy| {
                let Some(start_idx) = anchor_u.get() else {
                    return;
                };
                let (sx, sy) = origin_u.get();
                let d = doc_u.borrow();
                let cw = canvas_u.width() as f64;
                let ch = canvas_u.height() as f64;
                let turns = rot_u.get();
                let (ox, oy, rw, rh) = fitted_image_rect(cw, ch, base_dims.0, base_dims.1, turns);
                let nx = ((sx + dx) - ox) / rw.max(1.0);
                let ny = ((sy + dy) - oy) / rh.max(1.0);
                if let Some(end_idx) = d.nearest_word_index(nx, ny, turns) {
                    if sel_u.get() != Some((start_idx, end_idx)) {
                        sel_u.set(Some((start_idx, end_idx)));
                        ctrl_u.sync_copy_pill();
                        canvas_u.queue_draw();
                    }
                }
            });
        }
        {
            let anchor_e = drag_anchor;
            drag.connect_drag_end(move |_, _, _| {
                anchor_e.set(None);
            });
        }
        canvas.add_controller(drag);

        // Menú contextual con clic derecho sobre texto detectado («Copiar texto» / «Seleccionar todo»)
        let right_click = GestureClick::new();
        right_click.set_button(gdk::BUTTON_SECONDARY);
        {
            let doc_r = doc.clone();
            let sel_r = selection.clone();
            let canvas_r = canvas.clone();
            let enabled_r = interaction_enabled.clone();
            let rot_r = rotation_turns.clone();
            let ctrl_r = controller.clone();
            right_click.connect_pressed(move |_, _, x, y| {
                if !enabled_r.get() {
                    return;
                }
                let d = doc_r.borrow();
                if d.is_empty() {
                    return;
                }
                let cw = canvas_r.width() as f64;
                let ch = canvas_r.height() as f64;
                let turns = rot_r.get();
                let (ox, oy, rw, rh) = fitted_image_rect(cw, ch, base_dims.0, base_dims.1, turns);
                let nx = (x - ox) / rw.max(1.0);
                let ny = (y - oy) / rh.max(1.0);
                let pad_x = 8.0 / rw.max(1.0);
                let pad_y = 6.0 / rh.max(1.0);
                if sel_r.get().is_none() {
                    if let Some(hit) = d.hit_test(nx, ny, pad_x, pad_y, turns) {
                        sel_r.set(Some((hit, hit)));
                        ctrl_r.sync_copy_pill();
                        canvas_r.queue_draw();
                    }
                }
                drop(d);

                let pop = Popover::new();
                pop.set_parent(&canvas_r);
                pop.set_has_arrow(false);
                pop.set_pointing_to(Some(&gdk::Rectangle::new(
                    x.round() as i32,
                    y.round() as i32,
                    1,
                    1,
                )));
                let menu_box = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(2)
                    .margin_top(4)
                    .margin_bottom(4)
                    .margin_start(4)
                    .margin_end(4)
                    .build();

                let copy_btn = Button::builder()
                    .label(if sel_r.get().is_some() {
                        if es { "Copiar texto" } else { "Copy Text" }
                    } else if es {
                        "Copiar todo el texto"
                    } else {
                        "Copy All Text"
                    })
                    .has_frame(false)
                    .build();
                {
                    let pop_c = pop.clone();
                    let ctrl_c = ctrl_r.clone();
                    copy_btn.connect_clicked(move |_| {
                        pop_c.popdown();
                        ctrl_c.copy_active_or_all();
                    });
                }
                menu_box.append(&copy_btn);

                let sel_all_btn = Button::builder()
                    .label(if es {
                        "Seleccionar todo"
                    } else {
                        "Select All"
                    })
                    .has_frame(false)
                    .build();
                {
                    let pop_s = pop.clone();
                    let ctrl_s = ctrl_r.clone();
                    sel_all_btn.connect_clicked(move |_| {
                        pop_s.popdown();
                        ctrl_s.select_all();
                    });
                }
                menu_box.append(&sel_all_btn);

                pop.set_child(Some(&menu_box));
                pop.connect_closed(|p| {
                    p.unparent();
                });
                pop.popup();
            });
        }
        canvas.add_controller(right_click);

        // Acción del botón Live Text (alterna resaltado de todos los bloques + píldora Copiar)
        {
            let ctrl_l = controller.clone();
            let icon_l = live_icon;
            live_btn.connect_clicked(move |_| {
                let next = !ctrl_l.highlight_all.get();
                ctrl_l.highlight_all.set(next);
                if !next {
                    ctrl_l.selection.set(None);
                }
                apply_inline_hud_pill_css(&ctrl_l.live_btn, next);
                ctrl_l.sync_copy_pill();
                icon_l.queue_draw();
                ctrl_l.canvas.queue_draw();
            });
        }

        // Acción del botón píldora «Copiar» / «Copiar todo»
        {
            let ctrl_c = controller.clone();
            copy_pill_btn.connect_clicked(move |_| {
                ctrl_c.copy_active_or_all();
            });
        }

        img_frame.add_overlay(&canvas);
        img_frame.set_measure_overlay(&canvas, false);
        img_frame.add_overlay(&hud_box);
        img_frame.set_measure_overlay(&hud_box, false);
        img_frame.add_overlay(&toast_label);
        img_frame.set_measure_overlay(&toast_label, false);

        // Lanzar Tesseract TSV en un hilo en segundo plano sin bloquear Quick Look
        let path_owned = image_path.to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel::<LiveTextDocument>();
        std::thread::spawn(move || {
            if let Some(parsed) = run_tesseract_tsv_for_file(&path_owned, img_w, img_h) {
                let _ = tx.send(parsed);
            }
        });

        let canvas_weak = canvas.downgrade();
        let hud_weak = hud_box.downgrade();
        let doc_async = doc;
        let enabled_async = interaction_enabled;
        glib::timeout_add_local(Duration::from_millis(45), move || {
            let (Some(c), Some(hud)) = (canvas_weak.upgrade(), hud_weak.upgrade()) else {
                return ControlFlow::Break;
            };
            match rx.try_recv() {
                Ok(parsed) => {
                    if !parsed.is_empty() {
                        *doc_async.borrow_mut() = parsed;
                        if enabled_async.get() {
                            c.set_can_target(true);
                            hud.set_visible(true);
                        }
                        c.queue_draw();
                    }
                    ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => ControlFlow::Break,
            }
        });

        controller
    }

    /// Habilita o pausa la interacción Live Text (p. ej. al activar herramientas de marcación en Capturas).
    pub fn set_interaction_enabled(&self, enabled: bool) {
        self.interaction_enabled.set(enabled);
        let has_words = !self.doc.borrow().is_empty();
        self.canvas.set_can_target(enabled && has_words);
        if let Some(hud) = self.live_btn.parent() {
            hud.set_visible(enabled && has_words);
        }
        if !enabled {
            self.canvas.set_cursor_from_name(None);
        }
    }

    /// Actualiza la rotación acumulada (en cuartos de vuelta antihorarios) para alinear las cajas con la imagen.
    pub fn set_rotation_turns(&self, turns: u8) {
        self.rotation_turns.set(turns % 4);
        self.canvas.queue_draw();
    }

    pub fn rotate_ccw(&self) {
        self.set_rotation_turns((self.rotation_turns.get() + 1) % 4);
    }

    pub fn has_selection(&self) -> bool {
        self.selection.get().is_some()
    }

    pub fn has_detected_text(&self) -> bool {
        !self.doc.borrow().is_empty()
    }

    pub fn select_all(&self) {
        let len = self.doc.borrow().words.len();
        if len > 0 {
            self.selection.set(Some((0, len - 1)));
            self.sync_copy_pill();
            self.canvas.queue_draw();
        }
    }

    /// Copia la selección activa (o todo el texto si el modo resaltado está activo) y devuelve `true` si copió algo.
    pub fn copy_selection_or_highlighted(&self) -> bool {
        let d = self.doc.borrow();
        if d.is_empty() {
            return false;
        }
        let text = if let Some((a, b)) = self.selection.get() {
            d.extract_range(a, b)
        } else if self.highlight_all.get() {
            d.all_text()
        } else {
            return false;
        };
        drop(d);
        if text.trim().is_empty() {
            return false;
        }
        copy_plain_text_to_clipboard(&text);
        self.show_copied_toast();
        true
    }

    pub fn copy_active_or_all(&self) {
        let d = self.doc.borrow();
        if d.is_empty() {
            return;
        }
        let text = if let Some((a, b)) = self.selection.get() {
            d.extract_range(a, b)
        } else {
            d.all_text()
        };
        drop(d);
        if !text.trim().is_empty() {
            copy_plain_text_to_clipboard(&text);
            self.show_copied_toast();
        }
    }

    fn sync_copy_pill(&self) {
        let es = crate::is_spanish_locale();
        let has_sel = self.selection.get().is_some();
        let show = has_sel || self.highlight_all.get();
        self.copy_pill_btn.set_visible(show);
        self.copy_pill_btn.set_label(if has_sel {
            if es { "Copiar" } else { "Copy" }
        } else if es {
            "Copiar todo"
        } else {
            "Copy All"
        });
    }

    fn show_copied_toast(&self) {
        let next_gen = self.toast_gen.get().wrapping_add(1);
        self.toast_gen.set(next_gen);
        self.toast_label.set_visible(true);
        let lbl_weak = self.toast_label.downgrade();
        let gen_ref = self.toast_gen.clone();
        glib::timeout_add_local_once(Duration::from_millis(1450), move || {
            if gen_ref.get() == next_gen {
                if let Some(lbl) = lbl_weak.upgrade() {
                    lbl.set_visible(false);
                }
            }
        });
    }
}

fn apply_inline_hud_pill_css(btn: &Button, active: bool) {
    let provider = gtk4::CssProvider::new();
    let (ar, ag, ab) = crate::config::load_accent_color();
    let r = (ar * 255.0).round() as u8;
    let g = (ag * 255.0).round() as u8;
    let b = (ab * 255.0).round() as u8;
    let bg = if active {
        format!("rgba({r}, {g}, {b}, 0.94)")
    } else {
        "rgba(24, 26, 30, 0.72)".to_string()
    };
    let hover_bg = if active {
        format!("rgba({r}, {g}, {b}, 1.0)")
    } else {
        "rgba(32, 34, 40, 0.86)".to_string()
    };
    provider.load_from_string(&format!(
        "button {{ \
            min-height: 30px; min-width: 30px; padding: 0 10px; \
            border-radius: 10px; border: 1px solid rgba(255, 255, 255, 0.22); \
            background-image: none; background-color: {bg}; color: #ffffff; \
            font-size: 12px; font-weight: 600; \
            box-shadow: 0 4px 12px rgba(0, 0, 0, 0.28); \
        }}\n\
        button:hover {{ background-color: {hover_bg}; }}"
    ));
    #[allow(deprecated)]
    btn.style_context()
        .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 10);
}

fn apply_toast_css(label: &Label) {
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(
        "label { \
            padding: 5px 14px; border-radius: 999px; \
            background-color: rgba(24, 26, 30, 0.86); \
            border: 1px solid rgba(255, 255, 255, 0.18); \
            color: #ffffff; font-size: 12px; font-weight: 600; \
            box-shadow: 0 4px 14px rgba(0, 0, 0, 0.30); \
        }",
    );
    #[allow(deprecated)]
    label
        .style_context()
        .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 10);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TSV: &str = "\
level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext
1\t1\t0\t0\t0\t0\t0\t0\t1000\t500\t-1\t
5\t1\t1\t1\t1\t1\t100\t50\t80\t30\t95.0\tHola
5\t1\t1\t1\t1\t2\t200\t50\t100\t30\t92.5\tMalus
5\t1\t1\t1\t2\t1\t100\t100\t120\t30\t88.0\tDesktop
5\t1\t2\t1\t1\t1\t100\t220\t150\t30\t91.0\tNuevo
5\t1\t2\t1\t1\t2\t300\t220\t40\t20\t12.0\tRuido
";

    #[test]
    fn parses_tesseract_tsv_filters_low_confidence_and_preserves_paragraphs() {
        let doc = parse_tesseract_tsv(SAMPLE_TSV, 1000.0, 500.0);
        assert_eq!(doc.words.len(), 4);
        assert_eq!(doc.words[0].text, "Hola");
        assert!((doc.words[0].x - 0.10).abs() < 1e-6);
        assert!((doc.words[0].y - 0.10).abs() < 1e-6);

        assert_eq!(doc.extract_range(0, 1), "Hola Malus");
        assert_eq!(doc.extract_range(1, 0), "Hola Malus");
        assert_eq!(doc.extract_range(0, 2), "Hola Malus\nDesktop");
        assert_eq!(doc.all_text(), "Hola Malus\nDesktop\n\nNuevo");
    }

    #[test]
    fn hit_test_and_rotation_transform_word_boxes_accurately() {
        let doc = parse_tesseract_tsv(SAMPLE_TSV, 1000.0, 500.0);
        // Palabra 0: x=0.10..0.18, y=0.10..0.16
        assert_eq!(doc.hit_test(0.14, 0.13, 0.0, 0.0, 0), Some(0));
        assert_eq!(doc.hit_test(0.95, 0.95, 0.0, 0.0, 0), None);

        // Tras 1 rotación 90° CCW: x' = y = 0.10..0.16, y' = 1 - x - w = 0.82..0.90
        let (rx, ry, rw, rh) = doc.words[0].rotated_rect(1);
        assert!((rx - 0.10).abs() < 1e-6);
        assert!((ry - 0.82).abs() < 1e-6);
        assert!((rw - 0.06).abs() < 1e-6);
        assert!((rh - 0.08).abs() < 1e-6);
        assert_eq!(doc.hit_test(0.13, 0.86, 0.0, 0.0, 1), Some(0));

        // 4 rotaciones vuelven exactamente al origen
        assert_eq!(doc.words[0].rotated_rect(4), doc.words[0].rotated_rect(0));
    }

    #[test]
    fn nearest_word_tracks_line_flow_smoothly() {
        let doc = parse_tesseract_tsv(SAMPLE_TSV, 1000.0, 500.0);
        assert_eq!(doc.nearest_word_index(0.25, 0.13, 0), Some(1));
        assert_eq!(doc.nearest_word_index(0.15, 0.23, 0), Some(2));
        assert_eq!(doc.nearest_word_index(0.18, 0.46, 0), Some(3));
    }
}
