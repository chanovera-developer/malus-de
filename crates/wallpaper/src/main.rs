use anyhow::Result;
use clap::Parser;
use gio::prelude::*;
use gtk4::gdk::prelude::{DisplayExt, MonitorExt};
use gtk4::gdk::Key;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, Entry,
    EventControllerKey, Fixed, GestureClick, GestureDrag, Image, Label, Orientation, Overlay,
    Picture, Popover, Stack, StackTransitionType,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use malus_de_common::{
    config::{
        parse_wallpaper_hex_color, DesktopIconsConfig, DisplaysConfig, WallpaperConfig,
        WallpaperFitMode, WallpaperMode,
    },
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
#[command(
    about = "Demonio de Fondo de Pantalla y Superficie de Escritorio de Malus DE (Layer Shell)"
)]
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
/// Duración de la animación de reorganización del escritorio (alineada con `GRID_REORDER_ANIM_MS` de Branches).
const DESKTOP_ORGANIZE_ANIM_MS: i64 = 240;

#[inline]
pub fn compute_cover_crop_geometry(
    src_w: i32,
    src_h: i32,
    target_w: i32,
    target_h: i32,
) -> (i32, i32, i32, i32) {
    let target_w = target_w.max(1);
    let target_h = target_h.max(1);
    if src_w <= 0 || src_h <= 0 {
        return (target_w, target_h, 0, 0);
    }
    let scale = (target_w as f64 / src_w as f64).max(target_h as f64 / src_h as f64);
    let scaled_w = ((src_w as f64 * scale).ceil() as i32).max(target_w);
    let scaled_h = ((src_h as f64 * scale).ceil() as i32).max(target_h);
    let crop_x = ((scaled_w - target_w) / 2).max(0);
    let crop_y = ((scaled_h - target_h) / 2).max(0);
    (scaled_w, scaled_h, crop_x, crop_y)
}

fn build_scaled_wallpaper_texture(
    path: &Path,
    target_w: i32,
    target_h: i32,
) -> Option<gtk4::gdk::Texture> {
    let target_w = target_w.max(1);
    let target_h = target_h.max(1);
    let loaded = gdk_pixbuf::Pixbuf::from_file(path).ok()?;
    let pixbuf = loaded.apply_embedded_orientation().unwrap_or(loaded);
    let src_w = pixbuf.width();
    let src_h = pixbuf.height();
    let final_pixbuf = if src_w == target_w && src_h == target_h {
        pixbuf
    } else {
        let (scaled_w, scaled_h, crop_x, crop_y) =
            compute_cover_crop_geometry(src_w, src_h, target_w, target_h);
        let scaled = if src_w == scaled_w && src_h == scaled_h {
            pixbuf
        } else {
            pixbuf.scale_simple(scaled_w, scaled_h, gdk_pixbuf::InterpType::Bilinear)?
        };
        if scaled.width() == target_w && scaled.height() == target_h {
            scaled
        } else {
            let sub = scaled.new_subpixbuf(crop_x, crop_y, target_w, target_h);
            sub.copy()?
        }
    };
    let w = final_pixbuf.width() as usize;
    let h = final_pixbuf.height() as usize;
    let stride = final_pixbuf.rowstride() as usize;
    let channels = final_pixbuf.n_channels() as usize;
    if w == 0 || h == 0 || channels < 3 {
        return Some(gtk4::gdk::Texture::for_pixbuf(&final_pixbuf));
    }
    let src_bytes = final_pixbuf.read_pixel_bytes();
    let src = src_bytes.as_ref();
    let mut bgrx = vec![255u8; w * h * 4];
    for y in 0..h {
        let row_src = y * stride;
        let row_dst = y * w * 4;
        for x in 0..w {
            let s = row_src + x * channels;
            let d = row_dst + x * 4;
            if s + 3 > src.len() {
                return Some(gtk4::gdk::Texture::for_pixbuf(&final_pixbuf));
            }
            // Formato nativo Cairo RGB24 / GDK B8g8r8x8 (little-endian: B, G, R, 0xFF)
            bgrx[d] = src[s + 2];
            bgrx[d + 1] = src[s + 1];
            bgrx[d + 2] = src[s];
            bgrx[d + 3] = 255;
        }
    }
    let bytes = glib::Bytes::from_owned(bgrx);
    let tex = gtk4::gdk::MemoryTexture::new(
        w as i32,
        h as i32,
        gtk4::gdk::MemoryFormat::B8g8r8a8Premultiplied,
        &bytes,
        w * 4,
    );
    Some(tex.upcast())
}

const THUMB_SQUIRCLE_EXPONENT: f32 = 4.2;

fn thumb_squircle_sdf(px: f32, py: f32, half_w: f32, half_h: f32, radius: f32) -> f32 {
    let r = radius.min(half_w.min(half_h)).max(1.0);
    let dx = px.abs() - half_w + r;
    let dy = py.abs() - half_h + r;
    if dx <= 0.0 || dy <= 0.0 {
        dx.max(dy) - r
    } else {
        (dx.powf(THUMB_SQUIRCLE_EXPONENT) + dy.powf(THUMB_SQUIRCLE_EXPONENT))
            .powf(1.0 / THUMB_SQUIRCLE_EXPONENT)
            - r
    }
}

/// Mismo radio relativo que las miniaturas de Branches.
fn thumb_squircle_radius(width: i32, height: i32) -> f32 {
    let min_side = width.min(height).max(1) as f32;
    let max_r = (min_side * 0.45).max(0.5);
    let min_r = 3.0_f32.min(max_r);
    (min_side * 0.22).clamp(min_r, max_r)
}

/// Recorta a transparente las esquinas de un buffer RGBA (stride = width * 4) con squircle.
fn apply_thumb_squircle_clip_rgba(rgba: &mut [u8], width: usize, height: usize, radius: f32) {
    if width < 4 || height < 4 {
        return;
    }
    let half_w = width as f32 * 0.5;
    let half_h = height as f32 * 0.5;
    let r = radius.min(half_w.min(half_h)).max(1.0);
    let span = r.ceil() as usize + 1;
    let y_top_end = span.min(height);
    let y_bot_start = height.saturating_sub(span).max(y_top_end);
    let x_left_end = span.min(width);
    let x_right_start = width.saturating_sub(span).max(x_left_end);
    for y in (0..y_top_end).chain(y_bot_start..height) {
        let ly = (y as f32 + 0.5) - half_h;
        for x in (0..x_left_end).chain(x_right_start..width) {
            let lx = (x as f32 + 0.5) - half_w;
            let dist = thumb_squircle_sdf(lx, ly, half_w, half_h, r);
            if dist > -0.5 {
                let idx = (y * width + x) * 4 + 3;
                if dist >= 0.5 {
                    rgba[idx] = 0;
                } else {
                    let coverage = (0.5 - dist).clamp(0.0, 1.0);
                    rgba[idx] = (rgba[idx] as f32 * coverage).round() as u8;
                }
            }
        }
    }
}

/// Devuelve el pixbuf con las esquinas recortadas en squircle (como en Branches).
fn squircle_clip_pixbuf(pixbuf: &gdk_pixbuf::Pixbuf) -> Option<gdk_pixbuf::Pixbuf> {
    let width = pixbuf.width();
    let height = pixbuf.height();
    if width <= 0 || height <= 0 || pixbuf.n_channels() < 3 {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    let stride = pixbuf.rowstride() as usize;
    let channels = pixbuf.n_channels() as usize;
    let has_alpha = pixbuf.has_alpha();
    let bytes = pixbuf.read_pixel_bytes();
    let src = bytes.as_ref();
    let mut rgba = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let s = y * stride + x * channels;
            let d = (y * w + x) * 4;
            if s + channels > src.len() {
                return None;
            }
            rgba[d..d + 3].copy_from_slice(&src[s..s + 3]);
            if has_alpha {
                rgba[d + 3] = src[s + 3];
            }
        }
    }
    apply_thumb_squircle_clip_rgba(&mut rgba, w, h, thumb_squircle_radius(width, height));
    Some(gdk_pixbuf::Pixbuf::from_bytes(
        &glib::Bytes::from_owned(rgba),
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        width,
        height,
        width * 4,
    ))
}

fn build_default_gradient_texture(target_w: i32, target_h: i32) -> Option<gtk4::gdk::Texture> {
    build_aurora_gradient_texture(target_w, target_h, true)
}

fn build_aurora_gradient_texture(
    target_w: i32,
    target_h: i32,
    is_dark: bool,
) -> Option<gtk4::gdk::Texture> {
    let target_w = target_w.max(1);
    let target_h = target_h.max(1);
    let surface =
        gtk4::cairo::ImageSurface::create(gtk4::cairo::Format::ARgb32, target_w, target_h).ok()?;
    {
        let cr = gtk4::cairo::Context::new(&surface).ok()?;
        let cx = target_w as f64 * 0.35;
        let cy = target_h as f64 * 0.25;
        let radius = (target_w as f64 - cx).hypot(target_h as f64 - cy);
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
        let _ = cr.paint();
    }
    let data = surface.take_data().ok()?;
    let bytes = glib::Bytes::from_owned(data.to_vec());
    let tex = gtk4::gdk::MemoryTexture::new(
        target_w,
        target_h,
        gtk4::gdk::MemoryFormat::B8g8r8a8Premultiplied,
        &bytes,
        (target_w as usize) * 4,
    );
    Some(tex.upcast())
}

fn build_solid_color_texture(color_hex: &str) -> Option<gtk4::gdk::Texture> {
    let (r, g, b) = parse_wallpaper_hex_color(color_hex).unwrap_or((0x4D, 0xA6, 0xC2));
    // Cairo/GDK B8g8r8a8Premultiplied en little-endian: B, G, R, A
    let pixel = [b, g, r, 255u8];
    let mut buf = Vec::with_capacity(64 * 4);
    for _ in 0..64 {
        buf.extend_from_slice(&pixel);
    }
    let bytes = glib::Bytes::from_owned(buf);
    let tex = gtk4::gdk::MemoryTexture::new(
        8,
        8,
        gtk4::gdk::MemoryFormat::B8g8r8a8Premultiplied,
        &bytes,
        8 * 4,
    );
    Some(tex.upcast())
}

fn build_centered_wallpaper_texture(
    path: &Path,
    target_w: i32,
    target_h: i32,
) -> Option<gtk4::gdk::Texture> {
    let target_w = target_w.max(1);
    let target_h = target_h.max(1);
    let loaded = gdk_pixbuf::Pixbuf::from_file(path).ok()?;
    let pixbuf = loaded.apply_embedded_orientation().unwrap_or(loaded);
    let surface =
        gtk4::cairo::ImageSurface::create(gtk4::cairo::Format::ARgb32, target_w, target_h).ok()?;
    {
        let cr = gtk4::cairo::Context::new(&surface).ok()?;
        cr.set_source_rgb(0.05, 0.08, 0.14);
        let _ = cr.paint();
        let dx = (target_w - pixbuf.width()) as f64 / 2.0;
        let dy = (target_h - pixbuf.height()) as f64 / 2.0;
        gtk4::cairo::Context::set_source_pixbuf(&cr, &pixbuf, dx, dy);
        let _ = cr.paint();
    }
    let data = surface.take_data().ok()?;
    let bytes = glib::Bytes::from_owned(data.to_vec());
    let tex = gtk4::gdk::MemoryTexture::new(
        target_w,
        target_h,
        gtk4::gdk::MemoryFormat::B8g8r8a8Premultiplied,
        &bytes,
        (target_w as usize) * 4,
    );
    Some(tex.upcast())
}

fn apply_image_with_fit_mode(
    picture: &Picture,
    img_path: &Path,
    target_phys_w: i32,
    target_phys_h: i32,
    fit_mode: WallpaperFitMode,
) {
    match fit_mode {
        WallpaperFitMode::Fill => {
            if let Some(tex) = build_scaled_wallpaper_texture(img_path, target_phys_w, target_phys_h)
            {
                picture.set_paintable(Some(&tex));
                picture.set_content_fit(gtk4::ContentFit::Fill);
            } else {
                picture.set_filename(Some(img_path));
                picture.set_content_fit(gtk4::ContentFit::Cover);
            }
        }
        WallpaperFitMode::Fit => {
            if let Ok(loaded) = gdk_pixbuf::Pixbuf::from_file(img_path) {
                let pb = loaded.apply_embedded_orientation().unwrap_or(loaded);
                let tex = gtk4::gdk::Texture::for_pixbuf(&pb);
                picture.set_paintable(Some(&tex));
            } else {
                picture.set_filename(Some(img_path));
            }
            picture.set_content_fit(gtk4::ContentFit::Contain);
        }
        WallpaperFitMode::Stretch => {
            if let Ok(loaded) = gdk_pixbuf::Pixbuf::from_file(img_path) {
                let pb = loaded.apply_embedded_orientation().unwrap_or(loaded);
                let tex = gtk4::gdk::Texture::for_pixbuf(&pb);
                picture.set_paintable(Some(&tex));
            } else {
                picture.set_filename(Some(img_path));
            }
            picture.set_content_fit(gtk4::ContentFit::Fill);
        }
        WallpaperFitMode::Center => {
            if let Some(tex) =
                build_centered_wallpaper_texture(img_path, target_phys_w, target_phys_h)
            {
                picture.set_paintable(Some(&tex));
                picture.set_content_fit(gtk4::ContentFit::Fill);
            } else {
                picture.set_filename(Some(img_path));
                picture.set_content_fit(gtk4::ContentFit::ScaleDown);
            }
        }
    }
}

fn apply_wallpaper_to_picture(
    picture: &Picture,
    window: &ApplicationWindow,
    cli_image: &Option<PathBuf>,
    target_phys_w: i32,
    target_phys_h: i32,
) {
    if let Some(ref img_path) = cli_image.as_ref().filter(|p| p.exists()) {
        apply_image_with_fit_mode(
            picture,
            img_path,
            target_phys_w,
            target_phys_h,
            WallpaperFitMode::Fill,
        );
        window.add_css_class("has-wallpaper-texture");
        return;
    }

    let wp_cfg = WallpaperConfig::load();
    let is_dark = is_dark_mode_active();
    match wp_cfg.mode {
        WallpaperMode::SolidColor => {
            if let Some(tex) = build_solid_color_texture(&wp_cfg.color_hex) {
                picture.set_paintable(Some(&tex));
                picture.set_content_fit(gtk4::ContentFit::Fill);
                window.add_css_class("has-wallpaper-texture");
                return;
            }
        }
        WallpaperMode::Gradient => {
            if let Some(tex) =
                build_aurora_gradient_texture(target_phys_w, target_phys_h, is_dark)
            {
                picture.set_paintable(Some(&tex));
                picture.set_content_fit(gtk4::ContentFit::Fill);
                window.add_css_class("has-wallpaper-texture");
                return;
            }
        }
        WallpaperMode::DynamicPair => {
            let chosen = if is_dark {
                wp_cfg.dark_image_path.as_deref()
            } else {
                wp_cfg.light_image_path.as_deref()
            };
            if let Some(p) = chosen.map(PathBuf::from).filter(|p| p.is_file()) {
                info!("Cargando fondo dinámico ({}): {:?}", if is_dark { "obscuro" } else { "claro" }, p);
                apply_image_with_fit_mode(
                    picture,
                    &p,
                    target_phys_w,
                    target_phys_h,
                    wp_cfg.fit_mode,
                );
                window.add_css_class("has-wallpaper-texture");
                return;
            }
        }
        WallpaperMode::Image => {
            if let Some(p) = wp_cfg
                .image_path
                .as_deref()
                .map(PathBuf::from)
                .filter(|p| p.is_file())
            {
                info!("Cargando imagen configurada de fondo: {:?}", p);
                apply_image_with_fit_mode(
                    picture,
                    &p,
                    target_phys_w,
                    target_phys_h,
                    wp_cfg.fit_mode,
                );
                window.add_css_class("has-wallpaper-texture");
                return;
            }
        }
    }

    let fallback_path = resolve_wallpaper_image_path(&None);
    if let Some(ref img_path) = fallback_path.as_ref().filter(|p| p.exists()) {
        info!("Cargando imagen de fondo: {:?}", img_path);
        apply_image_with_fit_mode(
            picture,
            img_path,
            target_phys_w,
            target_phys_h,
            wp_cfg.fit_mode,
        );
        window.add_css_class("has-wallpaper-texture");
    } else if let Some(tex) = build_aurora_gradient_texture(target_phys_w, target_phys_h, is_dark) {
        picture.set_paintable(Some(&tex));
        picture.set_content_fit(gtk4::ContentFit::Fill);
        window.add_css_class("has-wallpaper-texture");
    }
}

static CURRENT_IS_DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

thread_local! {
    static REGISTERED_POPOVERS: RefCell<Vec<glib::WeakRef<Popover>>> = RefCell::new(Vec::new());
    static DESKTOP_LAYOUT_ANIM_GEN: Cell<u64> = const { Cell::new(0) };
    static WALLPAPER_GLASS_PROVIDER: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
}

fn apply_wallpaper_liquid_glass_opacity(opacity: f64) {
    let scale = malus_de_common::config::liquid_glass_scale_for(opacity);
    let dark_a = malus_de_common::config::scaled_liquid_glass_alpha(0.78, scale, 0.14, 0.96);
    let light_a = malus_de_common::config::scaled_liquid_glass_alpha(0.76, scale, 0.14, 0.96);
    let css = format!(
        "popover.desktop-popover-menu > contents, \
         popover.desktop-popover-menu > contents.background {{ \
             background-color: rgba(30, 30, 36, {dark_a:.3}); \
         }} \
         popover.desktop-popover-menu.light-popover > contents, \
         popover.desktop-popover-menu.light-popover > contents.background, \
         popover.light-popover.desktop-popover-menu > contents, \
         popover.light-popover > contents {{ \
             background-color: rgba(244, 246, 250, {light_a:.3}); \
         }}"
    );
    WALLPAPER_GLASS_PROVIDER.with(|cell| {
        if let Some(provider) = cell.borrow().as_ref() {
            provider.load_from_string(&css);
        }
    });
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
    if is_dark {
        popover.remove_css_class("light-popover");
    } else {
        popover.add_css_class("light-popover");
    }

    let pop_weak = popover.downgrade();
    popover.connect_show(move |_| {
        if let Some(pop) = pop_weak.upgrade() {
            let is_dark = is_dark_mode_active();
            if pop.has_css_class("light-popover") == is_dark {
                apply_theme_to_popover(&pop, is_dark);
            }
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

fn close_all_registered_popovers() {
    REGISTERED_POPOVERS.with(|list| {
        let pops: Vec<Popover> = list
            .borrow_mut()
            .drain(..)
            .filter_map(|w| w.upgrade())
            .collect();
        for pop in pops {
            pop.popdown();
            if pop.parent().is_some() {
                pop.unparent();
            }
        }
    });
}

fn slot_needs_role_rebuild(
    prev_is_primary: bool,
    prev_desktop_size: (i32, i32),
    next_is_primary: bool,
    next_desktop_size: (i32, i32),
) -> bool {
    prev_is_primary != next_is_primary
        || (next_is_primary && prev_desktop_size != next_desktop_size)
}

fn monitor_geometry_ready(monitor: &gtk4::gdk::Monitor) -> bool {
    let geo = monitor.geometry();
    geo.width() > 0 && geo.height() > 0
}

/// Desconecta el manejador interno `invalidate` de `gtk4-layer-shell` sobre este monitor.
/// De lo contrario, al quitar un monitor físico (`wl_registry.global_remove`), `gtk4-layer-shell`
/// llama a `gtk_layer_set_monitor(win, NULL)` y remapea/destruye `zwlr_layer_surface_v1`,
/// provocando que GTK4 envíe `commit()` tras destruir y Smithay 0.7.0 cierre el proceso con `InvalidSize`.
fn disconnect_layer_shell_monitor_invalidate(monitor: &gtk4::gdk::Monitor) {
    use glib::translate::{IntoGlib, ToGlibPtr};
    let mon_type = gtk4::gdk::Monitor::static_type().into_glib();
    let mon_ptr: *mut gtk4::gdk::ffi::GdkMonitor = monitor.to_glib_none().0;
    unsafe {
        let sig_id = glib::gobject_ffi::g_signal_lookup(c"invalidate".as_ptr(), mon_type);
        if sig_id != 0 {
            glib::gobject_ffi::g_signal_handlers_disconnect_matched(
                mon_ptr as *mut glib::gobject_ffi::GObject,
                glib::gobject_ffi::G_SIGNAL_MATCH_ID,
                sig_id,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
        }
    }
}

fn ensure_monitor_sync_hooks(
    monitor: &gtk4::gdk::Monitor,
    sync_handler: &Rc<RefCell<Option<Rc<dyn Fn()>>>>,
) {
    let already_hooked = unsafe { monitor.data::<bool>("malus-wp-mon-hooked") }.is_some();
    if already_hooked {
        return;
    }
    unsafe {
        monitor.set_data("malus-wp-mon-hooked", true);
    }
    let trigger = {
        let sync_c = sync_handler.clone();
        Rc::new(move || {
            if let Some(sched) = sync_c.borrow().as_ref() {
                sched();
            }
        })
    };
    let t1 = trigger.clone();
    monitor.connect_geometry_notify(move |_| t1());
    let t2 = trigger.clone();
    monitor.connect_scale_factor_notify(move |_| t2());
    let t3 = trigger;
    monitor.connect_connector_notify(move |_| t3());
}

struct WallpaperWindowSlot {
    monitor: Option<gtk4::gdk::Monitor>,
    window: ApplicationWindow,
    root_overlay: Overlay,
    picture: Picture,
    role_host: Option<Overlay>,
    key_controller: Option<EventControllerKey>,
    phys_w: i32,
    phys_h: i32,
    desktop_w: i32,
    desktop_h: i32,
    is_primary: bool,
}

fn park_wallpaper_slot(mut slot: WallpaperWindowSlot) -> ApplicationWindow {
    if let Some(host) = slot.role_host.take() {
        slot.root_overlay.remove_overlay(&host);
    }
    if let Some(kc) = slot.key_controller.take() {
        slot.window.remove_controller(&kc);
    }
    slot.picture.set_paintable(gtk4::gdk::Paintable::NONE);
    slot.root_overlay.set_visible(false);
    slot.window.set_opacity(0.0);
    slot.window.set_can_target(false);
    if gtk4_layer_shell::is_supported() {
        slot.window.set_keyboard_mode(KeyboardMode::None);
    }
    slot.window.set_default_size(1, 1);
    slot.window
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
        let (target, stale) = if is_dark {
            ("menu-dark-text", "menu-light-text")
        } else {
            ("menu-light-text", "menu-dark-text")
        };
        if label.has_css_class(stale) {
            label.remove_css_class(stale);
        }
        if !label.has_css_class(target) {
            label.add_css_class(target);
        }
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
#[allow(dead_code)]
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
        if elapsed == 0 {
            return glib::ControlFlow::Continue;
        }
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
        let shared = config.icon_grid_metrics();
        Self {
            icon_size: shared.icon_size,
            box_width: shared.box_width,
            box_height: shared.box_height,
            spacing: shared.spacing,
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
            item.remove_css_class("desktop-icon-dragging");
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
        (x - pointer_offset.0).round().clamp(min_x, max_x),
        (y - pointer_offset.1).round().clamp(min_y, max_y),
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
        snapped_x.clamp(
            16.0,
            (desktop_size.0 - metrics.box_width - 16).max(16) as f64,
        ),
        snapped_y.clamp(
            top_margin,
            (desktop_size.1 - metrics.box_height - 20).max(top_margin as i32) as f64,
        ),
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
        (desktop_size.0 - metrics.box_width - 16 - column as i32 * metrics.cell_width()).max(16)
            as f64
    };
    (
        x.clamp(
            16.0,
            (desktop_size.0 - metrics.box_width - 16).max(16) as f64,
        ),
        y.clamp(
            top_margin,
            (desktop_size.1 - metrics.box_height - 20).max(top_margin as i32) as f64,
        ),
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
    if let Some(path) = file.path() {
        if path.extension().and_then(|s| s.to_str()) == Some("desktop") {
            if malus_de_common::apps::spawn_desktop_entry_path(&path).is_ok() {
                return;
            }
            if let Some(app_info) = gio::DesktopAppInfo::from_filename(&path) {
                if app_info.launch(&[], None::<&gio::AppLaunchContext>).is_ok() {
                    return;
                }
            }
            warn!(
                "No se pudo iniciar la aplicación {:?} en el escritorio",
                path
            );
            return;
        }
    }
    let target = file
        .path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.uri().to_string());
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(&target);
    malus_de_common::config::apply_isolated_environment(&mut cmd);
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum DesktopClipboardAction {
    Copy,
    Cut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopClipboardPayload {
    action: DesktopClipboardAction,
    paths: Vec<PathBuf>,
}

thread_local! {
    static DESKTOP_FILE_CLIPBOARD: RefCell<Option<DesktopClipboardPayload>> = const { RefCell::new(None) };
    static EXTERNAL_CUT_PATHS: RefCell<HashSet<PathBuf>> = RefCell::new(HashSet::new());
}

fn encode_gnome_copied_files(action: &DesktopClipboardAction, paths: &[PathBuf]) -> String {
    let op = match action {
        DesktopClipboardAction::Copy => "copy",
        DesktopClipboardAction::Cut => "cut",
    };
    let mut lines = Vec::with_capacity(paths.len() + 1);
    lines.push(op.to_string());
    for p in paths {
        lines.push(gio::File::for_path(p).uri().to_string());
    }
    lines.join("\n")
}

fn parse_gnome_copied_files(text: &str) -> Option<DesktopClipboardPayload> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let op = lines.next()?;
    let action = match op {
        "copy" => DesktopClipboardAction::Copy,
        "cut" => DesktopClipboardAction::Cut,
        _ => return None,
    };
    let paths: Vec<PathBuf> = lines
        .filter_map(|uri| {
            if uri.starts_with('#') {
                return None;
            }
            if uri.contains("://") {
                gio::File::for_uri(uri).path()
            } else {
                Some(PathBuf::from(uri))
            }
        })
        .collect();
    if paths.is_empty() {
        None
    } else {
        Some(DesktopClipboardPayload { action, paths })
    }
}

fn parse_uri_list(text: &str) -> Option<DesktopClipboardPayload> {
    let paths: Vec<PathBuf> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| {
            if line.contains("://") {
                gio::File::for_uri(line).path()
            } else if line.starts_with('/') {
                Some(PathBuf::from(line))
            } else {
                None
            }
        })
        .collect();
    if paths.is_empty() {
        None
    } else {
        Some(DesktopClipboardPayload {
            action: DesktopClipboardAction::Copy,
            paths,
        })
    }
}

fn set_desktop_clipboard(action: DesktopClipboardAction, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let gnome_payload = encode_gnome_copied_files(&action, &paths);
    let uri_list: String = paths
        .iter()
        .map(|p| format!("{}\r\n", gio::File::for_path(p).uri()))
        .collect();

    EXTERNAL_CUT_PATHS.with(|slot| slot.borrow_mut().clear());
    DESKTOP_FILE_CLIPBOARD.with(|slot| {
        *slot.borrow_mut() = Some(DesktopClipboardPayload { action, paths });
    });

    if let Some(display) = gtk4::gdk::Display::default() {
        let provider = gtk4::gdk::ContentProvider::new_union(&[
            gtk4::gdk::ContentProvider::for_bytes(
                "x-special/gnome-copied-files",
                &glib::Bytes::from_owned(gnome_payload),
            ),
            gtk4::gdk::ContentProvider::for_bytes(
                "text/uri-list",
                &glib::Bytes::from_owned(uri_list),
            ),
        ]);
        let _ = display.clipboard().set_content(Some(&provider));
    }
}

fn clear_desktop_clipboard() {
    EXTERNAL_CUT_PATHS.with(|slot| slot.borrow_mut().clear());
    let had_local = DESKTOP_FILE_CLIPBOARD.with(|slot| slot.borrow_mut().take().is_some());
    if had_local {
        if let Some(display) = gtk4::gdk::Display::default() {
            let formats = display.clipboard().formats();
            if formats.contain_mime_type("x-special/gnome-copied-files") {
                let _ = display
                    .clipboard()
                    .set_content(None::<&gtk4::gdk::ContentProvider>);
            }
        }
    }
}

fn has_desktop_clipboard_files() -> bool {
    if let Some(display) = gtk4::gdk::Display::default() {
        let formats = display.clipboard().formats();
        return formats.contain_mime_type("x-special/gnome-copied-files")
            || formats.contain_mime_type("text/uri-list");
    }
    DESKTOP_FILE_CLIPBOARD.with(|slot| slot.borrow().is_some())
}

fn is_desktop_path_cut(path: &Path) -> bool {
    let local_cut = DESKTOP_FILE_CLIPBOARD.with(|slot| {
        slot.borrow().as_ref().is_some_and(|p| {
            p.action == DesktopClipboardAction::Cut && p.paths.iter().any(|cut_p| cut_p == path)
        })
    });
    if local_cut {
        return true;
    }
    EXTERNAL_CUT_PATHS.with(|slot| slot.borrow().contains(path))
}

fn sync_desktop_cut_states(fixed: &Fixed) {
    let mut sibling = fixed.first_child();
    while let Some(child) = sibling {
        sibling = child.next_sibling();
        if !child.has_css_class("desktop-icon") {
            continue;
        }
        let Some(item_path) = (unsafe { child.data::<PathBuf>("desktop_item_path") }) else {
            continue;
        };
        let cut = is_desktop_path_cut(unsafe { item_path.as_ref() });
        if cut {
            child.add_css_class("desktop-cut-item");
        } else {
            child.remove_css_class("desktop-cut-item");
        }
    }
}

fn refresh_cut_paths_from_display_clipboard(fixed: &Fixed) {
    let Some(display) = gtk4::gdk::Display::default() else {
        return;
    };
    let clipboard = display.clipboard();
    let formats = clipboard.formats();
    if !formats.contain_mime_type("x-special/gnome-copied-files") {
        EXTERNAL_CUT_PATHS.with(|slot| slot.borrow_mut().clear());
        if !clipboard.is_local() {
            DESKTOP_FILE_CLIPBOARD.with(|slot| *slot.borrow_mut() = None);
        }
        sync_desktop_cut_states(fixed);
        return;
    }

    if clipboard.is_local() {
        EXTERNAL_CUT_PATHS.with(|slot| slot.borrow_mut().clear());
        sync_desktop_cut_states(fixed);
        return;
    }

    let fixed_weak = fixed.downgrade();
    glib::MainContext::default().spawn_local(async move {
        let mut new_cuts = HashSet::new();
        if let Ok((stream, _mime)) = clipboard
            .read_future(&["x-special/gnome-copied-files"], glib::Priority::DEFAULT)
            .await
        {
            let mut raw = Vec::new();
            loop {
                let buf = vec![0u8; 65536];
                match stream.read_future(buf, glib::Priority::DEFAULT).await {
                    Ok((chunk, n)) if n > 0 => {
                        raw.extend_from_slice(&chunk[..n]);
                        if raw.len() >= 4 * 1024 * 1024 {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            let text = String::from_utf8_lossy(&raw);
            if let Some(payload) = parse_gnome_copied_files(&text) {
                if payload.action == DesktopClipboardAction::Cut {
                    new_cuts.extend(payload.paths);
                }
            }
        }
        EXTERNAL_CUT_PATHS.with(|slot| *slot.borrow_mut() = new_cuts);
        if let Some(fixed) = fixed_weak.upgrade() {
            sync_desktop_cut_states(&fixed);
        }
    });
}

fn watch_clipboard_for_desktop(fixed: &Fixed) {
    let Some(display) = gtk4::gdk::Display::default() else {
        return;
    };
    refresh_cut_paths_from_display_clipboard(fixed);
    let fixed_weak = fixed.downgrade();
    display.clipboard().connect_changed(move |_| {
        if let Some(fixed) = fixed_weak.upgrade() {
            refresh_cut_paths_from_display_clipboard(&fixed);
        }
    });
}

fn copy_dir_recursive_desktop(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)?.flatten() {
        let entry_path = entry.path();
        let target_path = dst.join(entry.file_name());
        if entry_path.is_dir() {
            copy_dir_recursive_desktop(&entry_path, &target_path)?;
        } else {
            fs::copy(&entry_path, &target_path)?;
        }
    }
    Ok(())
}

fn unique_destination_in_desktop_folder(src: &Path, dest_folder: &Path) -> Option<PathBuf> {
    let file_name = src.file_name()?;
    let mut dest = dest_folder.join(file_name);
    if !dest.exists() && dest.symlink_metadata().is_err() {
        return Some(dest);
    }

    let is_dir = src
        .symlink_metadata()
        .map(|m| m.is_dir())
        .unwrap_or_else(|_| src.is_dir());

    let (stem, ext) = if is_dir {
        (file_name.to_string_lossy().to_string(), String::new())
    } else {
        (
            src.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| file_name.to_string_lossy().to_string()),
            src.extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default(),
        )
    };

    let mut counter = 2usize;
    dest = dest_folder.join(format!("{} copia{}", stem, ext));
    while dest.exists() || dest.symlink_metadata().is_ok() {
        dest = dest_folder.join(format!("{} copia {}{}", stem, counter, ext));
        counter += 1;
    }
    Some(dest)
}

fn paste_desktop_payload_into(
    payload: &DesktopClipboardPayload,
    dest_folder: &Path,
) -> Vec<PathBuf> {
    if !dest_folder.is_dir() {
        return Vec::new();
    }
    let mut pasted = Vec::new();
    for src in &payload.paths {
        if !src.exists() && src.symlink_metadata().is_err() {
            continue;
        }
        if src == dest_folder || (src.is_dir() && dest_folder.starts_with(src)) {
            continue;
        }
        match payload.action {
            DesktopClipboardAction::Copy => {
                let Some(dest) = unique_destination_in_desktop_folder(src, dest_folder) else {
                    continue;
                };
                let ok = if src.is_dir() {
                    copy_dir_recursive_desktop(src, &dest).is_ok()
                } else {
                    fs::copy(src, &dest).is_ok()
                };
                if ok {
                    pasted.push(dest);
                }
            }
            DesktopClipboardAction::Cut => {
                if src.parent() == Some(dest_folder) {
                    continue;
                }
                let Some(dest) = unique_destination_in_desktop_folder(src, dest_folder) else {
                    continue;
                };
                let src_file = gio::File::for_path(src);
                let dest_file = gio::File::for_path(&dest);
                let ok = src_file
                    .move_(
                        &dest_file,
                        gio::FileCopyFlags::NONE,
                        gio::Cancellable::NONE,
                        None,
                    )
                    .is_ok()
                    || fs::rename(src, &dest).is_ok();
                if ok {
                    pasted.push(dest);
                }
            }
        }
    }
    pasted
}

fn paste_from_desktop_clipboard_async<F: Fn(Vec<PathBuf>) + 'static>(
    dest_dir: PathBuf,
    on_done: F,
) {
    let Some(display) = gtk4::gdk::Display::default() else {
        if let Some(payload) = DESKTOP_FILE_CLIPBOARD.with(|slot| slot.borrow().clone()) {
            let is_cut = payload.action == DesktopClipboardAction::Cut;
            let pasted = paste_desktop_payload_into(&payload, &dest_dir);
            if is_cut && !pasted.is_empty() {
                clear_desktop_clipboard();
            }
            on_done(pasted);
        } else {
            on_done(Vec::new());
        }
        return;
    };

    let clipboard = display.clipboard();
    let formats = clipboard.formats();
    let has_gnome = formats.contain_mime_type("x-special/gnome-copied-files");
    let has_uri = formats.contain_mime_type("text/uri-list");
    if !has_gnome && !has_uri {
        on_done(Vec::new());
        return;
    }

    if clipboard.is_local() {
        if let Some(payload) = DESKTOP_FILE_CLIPBOARD.with(|slot| slot.borrow().clone()) {
            let is_cut = payload.action == DesktopClipboardAction::Cut;
            let pasted = paste_desktop_payload_into(&payload, &dest_dir);
            if is_cut && !pasted.is_empty() {
                clear_desktop_clipboard();
            }
            on_done(pasted);
            return;
        }
    }

    glib::MainContext::default().spawn_local(async move {
        let payload = if let Ok((stream, mime)) = clipboard
            .read_future(
                &["x-special/gnome-copied-files", "text/uri-list"],
                glib::Priority::DEFAULT,
            )
            .await
        {
            let mut raw = Vec::new();
            loop {
                let buf = vec![0u8; 65536];
                match stream.read_future(buf, glib::Priority::DEFAULT).await {
                    Ok((chunk, n)) if n > 0 => {
                        raw.extend_from_slice(&chunk[..n]);
                        if raw.len() >= 4 * 1024 * 1024 {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            let text = String::from_utf8_lossy(&raw);
            if mime.as_str() == "x-special/gnome-copied-files" {
                parse_gnome_copied_files(&text)
            } else {
                parse_uri_list(&text)
            }
        } else {
            None
        };

        if let Some(payload) = payload {
            let is_cut = payload.action == DesktopClipboardAction::Cut;
            let pasted = paste_desktop_payload_into(&payload, &dest_dir);
            if is_cut && !pasted.is_empty() {
                clear_desktop_clipboard();
            }
            on_done(pasted);
        } else {
            on_done(Vec::new());
        }
    });
}

fn trash_file(path: &Path) {
    let file = gio::File::for_path(path);
    if let Err(err) = file.trash(gio::Cancellable::NONE) {
        warn!("Error al enviar archivo a la papelera {:?}: {}", path, err);
    }
}

fn duplicate_file(path: &Path) {
    if let Some(parent) = path.parent() {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("archivo");
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

    let entry = Entry::builder().text(&initial_name).hexpand(true).build();
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

fn get_unique_text_document_name(dir: &Path, base_stem: &str) -> PathBuf {
    let candidate = dir.join(format!("{}.txt", base_stem));
    if !candidate.exists() {
        return candidate;
    }
    let mut i = 2;
    loop {
        let candidate = dir.join(format!("{} {}.txt", base_stem, i));
        if !candidate.exists() {
            return candidate;
        }
        i += 1;
    }
}

fn desktop_rename_selection_end(file_name: &str, is_dir: bool) -> i32 {
    if is_dir {
        return -1;
    }
    match file_name.rfind('.') {
        Some(pos) if pos > 0 => file_name[..pos].chars().count() as i32,
        _ => -1,
    }
}

fn find_desktop_icon_widget(fixed: &Fixed, path: &Path) -> Option<gtk4::Widget> {
    let expected = path.to_string_lossy();
    let mut child = fixed.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if item.has_css_class("desktop-rubberband") {
            continue;
        }
        if item.property::<Option<String>>("tooltip-text").as_deref() == Some(expected.as_ref()) {
            return Some(item);
        }
    }
    None
}

fn show_desktop_inline_rename(
    item_widget: &gtk4::Widget,
    old_path: PathBuf,
    is_dir: bool,
    icon_positions: Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    selected_paths: Rc<RefCell<HashSet<PathBuf>>>,
    focused_path: Rc<RefCell<Option<PathBuf>>>,
    trigger_refresh: Rc<dyn Fn()>,
) {
    let Some(label) = get_icon_label(item_widget) else {
        return;
    };
    if !label.is_visible() {
        return;
    }
    let Some(parent_box) = label.parent().and_then(|p| p.downcast::<GtkBox>().ok()) else {
        return;
    };
    let Some(initial_name) = old_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    else {
        return;
    };
    if initial_name.is_empty() {
        return;
    }

    let label_w = label.width().max(68);
    let label_h = label.height().max(18);
    let entry = gtk4::Text::builder()
        .text(&initial_name)
        .width_chars(1)
        .max_width_chars(1)
        .xalign(0.5)
        .halign(Align::Center)
        .valign(Align::Center)
        .css_classes(["desktop-inline-rename-entry"])
        .build();
    entry.set_size_request(label_w, label_h);

    label.set_visible(false);
    parent_box.insert_child_after(&entry, Some(&label));

    let finished = Rc::new(Cell::new(false));
    let finish_edit: Rc<dyn Fn(bool)> = {
        let finished = finished.clone();
        let entry_ref = entry.clone();
        let label_ref = label.clone();
        let parent_ref = parent_box.clone();
        let old_p = old_path.clone();
        let initial = initial_name.clone();
        let positions_ref = icon_positions;
        let sel_ref = selected_paths;
        let foc_ref = focused_path;
        let trig_ref = trigger_refresh;
        Rc::new(move |commit: bool| {
            if finished.replace(true) {
                return;
            }
            let candidate = entry_ref.text().to_string();
            let trimmed = candidate.trim().to_string();

            if entry_ref.parent().is_some() {
                parent_ref.remove(&entry_ref);
            }
            label_ref.set_visible(true);
            if let Some(icon_root) = label_ref.ancestor(GtkBox::static_type()) {
                icon_root.grab_focus();
            }

            if !commit || trimmed.is_empty() || trimmed == initial || trimmed.contains('/') {
                return;
            }

            let Some(parent_dir) = old_p.parent() else {
                return;
            };
            let dest = parent_dir.join(&trimmed);
            if dest.exists() {
                return;
            }
            if let Err(err) = fs::rename(&old_p, &dest) {
                warn!("No se pudo renombrar {:?} a {:?}: {}", old_p, dest, err);
                return;
            }

            {
                let mut pos_map = positions_ref.borrow_mut();
                if let Some(pos) = pos_map.remove(&old_p) {
                    pos_map.insert(dest.clone(), pos);
                    save_icon_positions(&pos_map);
                }
            }
            {
                let mut sel = sel_ref.borrow_mut();
                if sel.remove(&old_p) || sel.is_empty() {
                    sel.insert(dest.clone());
                }
            }
            *foc_ref.borrow_mut() = Some(dest);
            trig_ref();
        })
    };

    {
        let finish = finish_edit.clone();
        entry.connect_activate(move |_| {
            finish(true);
        });
    }

    {
        let key_ctrl = EventControllerKey::new();
        let finish = finish_edit.clone();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == Key::Escape {
                finish(false);
                return glib::Propagation::Stop;
            }
            if matches!(key, Key::Return | Key::KP_Enter | Key::ISO_Enter) {
                finish(true);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        entry.add_controller(key_ctrl);
    }

    {
        let focus_ctrl = gtk4::EventControllerFocus::new();
        let finish = finish_edit;
        focus_ctrl.connect_leave(move |_| {
            let f = finish.clone();
            glib::idle_add_local_once(move || {
                f(true);
            });
        });
        entry.add_controller(focus_ctrl);
    }

    let entry_focus = entry;
    let sel_end = desktop_rename_selection_end(&initial_name, is_dir);
    glib::idle_add_local_once(move || {
        entry_focus.grab_focus();
        entry_focus.select_region(0, sel_end);
    });
}

fn schedule_desktop_inline_rename(
    fixed: Fixed,
    target_path: PathBuf,
    is_dir: bool,
    icon_positions: Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    selected_paths: Rc<RefCell<HashSet<PathBuf>>>,
    focused_path: Rc<RefCell<Option<PathBuf>>>,
    trigger_refresh: Rc<dyn Fn()>,
) {
    let mut attempts = 0u8;
    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        attempts = attempts.saturating_add(1);
        if let Some(widget) = find_desktop_icon_widget(&fixed, &target_path) {
            let ready = widget.root().is_some()
                && get_icon_label(&widget)
                    .is_some_and(|lbl| lbl.is_visible() && lbl.width() > 0 && lbl.height() > 0);
            if ready || attempts >= 12 {
                show_desktop_inline_rename(
                    &widget,
                    target_path.clone(),
                    is_dir,
                    icon_positions.clone(),
                    selected_paths.clone(),
                    focused_path.clone(),
                    trigger_refresh.clone(),
                );
                return glib::ControlFlow::Break;
            }
        } else if attempts >= 12 {
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

#[allow(clippy::too_many_arguments)]
fn create_desktop_item_and_edit(
    fixed: &Fixed,
    desktop_dir: &Path,
    desktop_size: (i32, i32),
    config: &Rc<RefCell<DesktopIconsConfig>>,
    icon_positions: &Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    selected_paths: &Rc<RefCell<HashSet<PathBuf>>>,
    focused_path: &Rc<RefCell<Option<PathBuf>>>,
    trigger_refresh: &Rc<dyn Fn()>,
    click_pos: (f64, f64),
    top_margin: f64,
    is_dir: bool,
) {
    let target_path = if is_dir {
        get_unique_folder_name(desktop_dir, "Nueva carpeta")
    } else {
        get_unique_text_document_name(desktop_dir, "Nuevo documento")
    };

    let create_result = if is_dir {
        fs::create_dir_all(&target_path)
    } else {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target_path)
            .map(|_| ())
    };

    if let Err(err) = create_result {
        warn!("No se pudo crear {:?}: {}", target_path, err);
        return;
    }

    let cfg = config.borrow().clone();
    let metrics = IconLayoutMetrics::from_config(&cfg);
    let mut pos = clamp_icon_position(
        click_pos.0,
        click_pos.1,
        (
            f64::from(metrics.box_width) / 2.0,
            f64::from(metrics.box_height) / 2.0,
        ),
        &metrics,
        desktop_size,
        top_margin,
    );
    if cfg.snap_to_grid {
        pos = snap_position_to_grid(pos, &metrics, cfg.align_left, desktop_size, top_margin);
    }

    {
        let mut pos_map = icon_positions.borrow_mut();
        pos_map.insert(target_path.clone(), pos);
        save_icon_positions(&pos_map);
    }
    {
        let mut sel = selected_paths.borrow_mut();
        sel.clear();
        sel.insert(target_path.clone());
    }
    *focused_path.borrow_mut() = Some(target_path.clone());
    trigger_refresh();

    schedule_desktop_inline_rename(
        fixed.clone(),
        target_path,
        is_dir,
        icon_positions.clone(),
        selected_paths.clone(),
        focused_path.clone(),
        trigger_refresh.clone(),
    );
}

#[allow(clippy::too_many_arguments)]
fn build_create_new_submenu(
    popover: &Popover,
    stack: &Stack,
    fixed: &Fixed,
    desktop_dir: PathBuf,
    desktop_size: (i32, i32),
    config: Rc<RefCell<DesktopIconsConfig>>,
    icon_positions: Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    selected_paths: Rc<RefCell<HashSet<PathBuf>>>,
    focused_path: Rc<RefCell<Option<PathBuf>>>,
    trigger_refresh: Rc<dyn Fn()>,
    click_pos: (f64, f64),
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

    let folder_btn = create_menu_button("Carpeta", false);
    {
        let p_c = popover.clone();
        let f_c = fixed.clone();
        let d_c = desktop_dir.clone();
        let cfg_c = config.clone();
        let pos_c = icon_positions.clone();
        let sel_c = selected_paths.clone();
        let foc_c = focused_path.clone();
        let trig_c = trigger_refresh.clone();
        folder_btn.connect_clicked(move |_| {
            p_c.popdown();
            create_desktop_item_and_edit(
                &f_c,
                &d_c,
                desktop_size,
                &cfg_c,
                &pos_c,
                &sel_c,
                &foc_c,
                &trig_c,
                click_pos,
                top_margin,
                true,
            );
        });
    }
    container.append(&folder_btn);

    let doc_btn = create_menu_button("Documento de texto", false);
    {
        let p_c = popover.clone();
        let f_c = fixed.clone();
        let d_c = desktop_dir;
        let cfg_c = config;
        let pos_c = icon_positions;
        let sel_c = selected_paths;
        let foc_c = focused_path;
        let trig_c = trigger_refresh;
        doc_btn.connect_clicked(move |_| {
            p_c.popdown();
            create_desktop_item_and_edit(
                &f_c,
                &d_c,
                desktop_size,
                &cfg_c,
                &pos_c,
                &sel_c,
                &foc_c,
                &trig_c,
                click_pos,
                top_margin,
                false,
            );
        });
    }
    container.append(&doc_btn);

    container
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
    struct SortEntry {
        path: PathBuf,
        is_dir: bool,
        name_lower: String,
        ext_lower: String,
        modified: std::time::SystemTime,
    }

    let mut entries_list = Vec::new();
    if let Ok(entries) = fs::read_dir(desktop_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let Some(file_name) = p.file_name() else {
                continue;
            };
            let name_str = file_name.to_string_lossy();
            if name_str.starts_with('.') {
                continue;
            }
            let is_dir = matches!(entry.file_type(), Ok(ft) if ft.is_dir())
                || (matches!(entry.file_type(), Ok(ft) if ft.is_symlink()) && p.is_dir());
            let modified = if by == DesktopOrganizeBy::DateModified {
                entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH)
            } else {
                std::time::UNIX_EPOCH
            };
            let ext_lower = if by == DesktopOrganizeBy::Kind {
                p.extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase()
            } else {
                String::new()
            };
            entries_list.push(SortEntry {
                name_lower: name_str.to_lowercase(),
                path: p,
                is_dir,
                ext_lower,
                modified,
            });
        }
    }

    match by {
        DesktopOrganizeBy::Name => {
            entries_list.sort_by(|a, b| {
                b.is_dir
                    .cmp(&a.is_dir)
                    .then_with(|| a.name_lower.cmp(&b.name_lower))
            });
        }
        DesktopOrganizeBy::Kind => {
            entries_list.sort_by(|a, b| {
                b.is_dir
                    .cmp(&a.is_dir)
                    .then_with(|| a.ext_lower.cmp(&b.ext_lower))
                    .then_with(|| a.name_lower.cmp(&b.name_lower))
            });
        }
        DesktopOrganizeBy::DateModified => {
            entries_list.sort_by(|a, b| b.modified.cmp(&a.modified));
        }
    }

    let metrics = IconLayoutMetrics::from_config(config);
    let rows = ((desktop_size.1 - (top_margin as i32) - 80).max(1) / metrics.cell_height()).max(1)
        as usize;

    let mut targets = HashMap::with_capacity(entries_list.len());
    for (index, entry) in entries_list.into_iter().enumerate() {
        let pos = calculate_grid_position(
            index,
            rows,
            &metrics,
            config.align_left,
            desktop_size,
            top_margin,
        );
        targets.insert(entry.path, pos);
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
    animate_desktop_layout_transition_with_completion(fixed, targets, duration_ms, None);
}

fn animate_desktop_layout_transition_with_completion(
    fixed: &Fixed,
    targets: &HashMap<PathBuf, (f64, f64)>,
    duration_ms: i64,
    on_complete: Option<Box<dyn FnOnce()>>,
) {
    let mut moves: Vec<(gtk4::Widget, (f64, f64), (f64, f64))> = Vec::new();
    let mut child = fixed.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if item.has_css_class("desktop-rubberband") {
            continue;
        }
        let current = fixed
            .child_transform(&item)
            .map(|t| t.to_translate())
            .map(|(tx, ty)| (f64::from(tx), f64::from(ty)))
            .unwrap_or_else(|| fixed.child_position(&item));
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
        if let Some(cb) = on_complete {
            cb();
        }
        return;
    }

    if fixed.root().is_none() || duration_ms <= 0 {
        for (widget, _, target) in &moves {
            fixed.move_(widget, target.0, target.1);
        }
        if let Some(cb) = on_complete {
            cb();
        }
        return;
    }

    let anim_gen = DESKTOP_LAYOUT_ANIM_GEN.with(|g| {
        let next = g.get().wrapping_add(1);
        g.set(next);
        next
    });

    let start_instant: Rc<Cell<Option<std::time::Instant>>> = Rc::new(Cell::new(None));
    let fixed_clone = fixed.clone();
    let completion = Rc::new(RefCell::new(on_complete));
    let duration_f = duration_ms as f64;

    fixed.add_tick_callback(move |_, _frame_clock| {
        if DESKTOP_LAYOUT_ANIM_GEN.with(|g| g.get()) != anim_gen {
            return glib::ControlFlow::Break;
        }

        let now = std::time::Instant::now();
        let start = match start_instant.get() {
            Some(t) => t,
            None => {
                start_instant.set(Some(now));
                // Primer fotograma: aplicar paso mínimo para mantener el frame clock activo a 60 Hz sin saltar frames
                for (widget, start_pos, target) in &moves {
                    let cx = start_pos.0 + (target.0 - start_pos.0) * 0.04;
                    let cy = start_pos.1 + (target.1 - start_pos.1) * 0.04;
                    fixed_clone.move_(widget, cx, cy);
                }
                return glib::ControlFlow::Continue;
            }
        };

        let elapsed_ms = now.saturating_duration_since(start).as_secs_f64() * 1000.0;
        if elapsed_ms >= duration_f {
            for (widget, _, target) in &moves {
                fixed_clone.move_(widget, target.0, target.1);
            }
            if let Some(cb) = completion.borrow_mut().take() {
                cb();
            }
            glib::ControlFlow::Break
        } else {
            let progress = (elapsed_ms / duration_f).clamp(0.0, 1.0);
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
    }
    let pos_for_save = icon_positions.clone();
    animate_desktop_layout_transition_with_completion(
        fixed,
        &targets,
        duration_ms,
        Some(Box::new(move || {
            save_icon_positions(&pos_for_save.borrow());
        })),
    );
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
            let snapped =
                snap_position_to_grid(*pt, &metrics, align_left, desktop_size, top_margin);
            targets.insert(p.clone(), snapped);
        }
    }
    {
        let mut positions = icon_positions.borrow_mut();
        for (p, pt) in &targets {
            positions.insert(p.clone(), *pt);
        }
    }
    let pos_for_save = icon_positions.clone();
    animate_desktop_layout_transition_with_completion(
        fixed,
        &targets,
        duration_ms,
        Some(Box::new(move || {
            save_icon_positions(&pos_for_save.borrow());
        })),
    );
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
            organize_desktop_icons(
                &dir_c,
                desktop_size,
                &cfg,
                &pos_c,
                DesktopOrganizeBy::Name,
                top_margin,
            );
            trig_c();
        } else {
            animate_desktop_organization(
                &fixed_c,
                &dir_c,
                desktop_size,
                &cfg,
                &pos_c,
                DesktopOrganizeBy::Name,
                DESKTOP_ORGANIZE_ANIM_MS,
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
            organize_desktop_icons(
                &dir_c,
                desktop_size,
                &cfg,
                &pos_c,
                DesktopOrganizeBy::Kind,
                top_margin,
            );
            trig_c();
        } else {
            animate_desktop_organization(
                &fixed_c,
                &dir_c,
                desktop_size,
                &cfg,
                &pos_c,
                DesktopOrganizeBy::Kind,
                DESKTOP_ORGANIZE_ANIM_MS,
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
            organize_desktop_icons(
                &dir_c,
                desktop_size,
                &cfg,
                &pos_c,
                DesktopOrganizeBy::DateModified,
                top_margin,
            );
            trig_c();
        } else {
            animate_desktop_organization(
                &fixed_c,
                &dir_c,
                desktop_size,
                &cfg,
                &pos_c,
                DesktopOrganizeBy::DateModified,
                DESKTOP_ORGANIZE_ANIM_MS,
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

fn has_desktop_icon_widget(fixed: &Fixed, path: &Path) -> bool {
    let expected = path.to_string_lossy();
    let mut child = fixed.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if item.has_css_class("desktop-rubberband") {
            continue;
        }
        if item.property::<Option<String>>("tooltip-text").as_deref() == Some(expected.as_ref()) {
            return true;
        }
    }
    false
}

fn remove_desktop_icon_widget(fixed: &Fixed, path: &Path) -> bool {
    let expected = path.to_string_lossy();
    let mut removed = false;
    let mut child = fixed.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if item.has_css_class("desktop-rubberband") {
            continue;
        }
        if item.property::<Option<String>>("tooltip-text").as_deref() == Some(expected.as_ref()) {
            fixed.remove(&item);
            removed = true;
        }
    }
    removed
}

fn resolve_desktop_app_gicon(icon: &str) -> gio::Icon {
    let trimmed = icon.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() {
        return gio::ThemedIcon::new("application-x-executable").upcast();
    }
    let path = Path::new(trimmed);
    if path.is_absolute() && path.is_file() {
        return gio::FileIcon::new(&gio::File::for_path(path)).upcast();
    }
    if !trimmed.ends_with("-symbolic") {
        if let Some(resolved_path) = malus_de_common::apps::resolve_icon_file_path(trimmed) {
            return gio::FileIcon::new(&gio::File::for_path(&resolved_path)).upcast();
        }
    }
    gio::ThemedIcon::from_names(&[trimmed, "application-x-executable"]).upcast()
}

fn build_desktop_item_data(path: &Path) -> Option<DesktopItemData> {
    if path.extension().and_then(|s| s.to_str()) == Some("desktop") {
        if let Some(entry) = malus_de_common::apps::parse_desktop_file_any(path) {
            return Some(DesktopItemData {
                path: path.to_path_buf(),
                display_name: entry.name,
                icon: Some(resolve_desktop_app_gicon(&entry.icon)),
                content_type: Some("application/x-desktop".to_string()),
                category: StackCategory::categorize(path, Some("application/x-desktop")),
            });
        }
    }

    let file = gio::File::for_path(path);
    let file_info = file.query_info(
        "standard::display-name,standard::icon,standard::content-type",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    );
    match file_info {
        Ok(info) => {
            let display_name = info.display_name().to_string();
            let icon = info.icon();
            let content_type = info.content_type().map(|value| value.to_string());
            let category = StackCategory::categorize(path, content_type.as_deref());
            Some(DesktopItemData {
                path: path.to_path_buf(),
                display_name,
                icon,
                content_type,
                category,
            })
        }
        Err(error) => {
            warn!("No se pudo obtener información de {:?}: {}", path, error);
            None
        }
    }
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
        if let Some(item) = build_desktop_item_data(&path) {
            items.push(item);
        }
    }

    let metrics = IconLayoutMetrics::from_config(config);
    let rows = ((desktop_size.1 - (top_margin as i32) - 80).max(1) / metrics.cell_height()).max(1)
        as usize;

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

            let (x, y) = calculate_grid_position(
                slot_index,
                rows,
                &metrics,
                config.align_left,
                desktop_size,
                top_margin,
            );
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

fn squircle_path(context: &gtk4::cairo::Context, width: f64, height: f64, radius: f64) {
    let radius = radius.min(width / 2.0).min(height / 2.0).max(1.0);
    let power = 2.0 / 4.2;
    let quarter_turn = std::f64::consts::FRAC_PI_2;
    const SEGMENTS: usize = 20;

    context.move_to(radius, 0.0);
    context.line_to(width - radius, 0.0);
    for step in 1..=SEGMENTS {
        let angle = quarter_turn * step as f64 / SEGMENTS as f64;
        context.line_to(
            width - radius + radius * angle.sin().powf(power),
            radius - radius * angle.cos().powf(power),
        );
    }
    context.line_to(width, height - radius);
    for step in 1..=SEGMENTS {
        let angle = quarter_turn * step as f64 / SEGMENTS as f64;
        context.line_to(
            width - radius + radius * angle.cos().powf(power),
            height - radius + radius * angle.sin().powf(power),
        );
    }
    context.line_to(radius, height);
    for step in 1..=SEGMENTS {
        let angle = quarter_turn * step as f64 / SEGMENTS as f64;
        context.line_to(
            radius - radius * angle.sin().powf(power),
            height - radius + radius * angle.cos().powf(power),
        );
    }
    context.line_to(0.0, radius);
    for step in 1..=SEGMENTS {
        let angle = quarter_turn * step as f64 / SEGMENTS as f64;
        context.line_to(
            radius - radius * angle.cos().powf(power),
            radius - radius * angle.sin().powf(power),
        );
    }
    context.close_path();
}

fn attach_squircle_menu_hover(btn: &Button) {
    let Some(inner) = btn.child() else {
        return;
    };
    btn.add_css_class("malus-squircle-menu-item");
    btn.set_child(gtk4::Widget::NONE);
    inner.set_margin_top(3);
    inner.set_margin_bottom(3);
    inner.set_margin_start(8);
    inner.set_margin_end(8);

    let bg = gtk4::DrawingArea::new();
    bg.set_can_target(false);
    bg.set_hexpand(true);
    bg.set_vexpand(true);

    let btn_weak = btn.downgrade();
    bg.set_draw_func(move |_, cr, w, h| {
        let width = w as f64;
        let height = h as f64;
        if width <= 2.0 || height <= 2.0 {
            return;
        }
        let Some(b) = btn_weak.upgrade() else {
            return;
        };
        let flags = b.state_flags();
        if flags.contains(gtk4::StateFlags::INSENSITIVE) {
            return;
        }
        let hovered = flags.contains(gtk4::StateFlags::PRELIGHT);
        let active = flags.contains(gtk4::StateFlags::ACTIVE);
        if !hovered && !active {
            return;
        }
        let _ = cr.save();
        squircle_path(cr, width, height, 14.0);
        let (red, green, blue) = malus_de_common::config::load_accent_color();
        if active {
            cr.set_source_rgba(red * 0.78, green * 0.78, blue * 0.78, 0.86);
        } else {
            cr.set_source_rgba(red, green, blue, 0.74);
        }
        let _ = cr.fill();
        let _ = cr.restore();
    });

    let bg_state = bg.clone();
    btn.connect_state_flags_changed(move |_, _| {
        bg_state.queue_draw();
    });

    let overlay = Overlay::builder().hexpand(true).build();
    overlay.set_child(Some(&bg));
    overlay.add_overlay(&inner);
    overlay.set_measure_overlay(&inner, true);
    btn.set_child(Some(&overlay));
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
    attach_squircle_menu_hover(&btn);
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
            if let Ok(pixbuf) =
                gdk_pixbuf::Pixbuf::from_file_at_scale(path, icon_size, icon_size, true)
            {
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

fn build_multi_desktop_drag_paintable(
    items: &[(gtk4::Widget, (f64, f64))],
    cursor_x: f64,
    cursor_y: f64,
) -> Option<(gtk4::gdk::Paintable, i32, i32)> {
    if items.is_empty() {
        return None;
    }

    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for (w, (ix, iy)) in items {
        let ww = w.width().max(w.width_request()).max(72) as f64;
        let wh = w.height().max(w.height_request()).max(76) as f64;
        min_x = min_x.min(*ix);
        min_y = min_y.min(*iy);
        max_x = max_x.max(*ix + ww);
        max_y = max_y.max(*iy + wh);
    }

    if !min_x.is_finite() || !min_y.is_finite() {
        return None;
    }

    let snapshot = gtk4::Snapshot::new();
    for (w, (ix, iy)) in items {
        let wp = gtk4::WidgetPaintable::new(Some(w));
        let ww = wp
            .intrinsic_width()
            .max(w.width())
            .max(w.width_request())
            .max(72) as f64;
        let wh = wp
            .intrinsic_height()
            .max(w.height())
            .max(w.height_request())
            .max(76) as f64;
        snapshot.save();
        snapshot.translate(&gtk4::graphene::Point::new(
            (*ix - min_x) as f32,
            (*iy - min_y) as f32,
        ));
        wp.snapshot(&snapshot, ww, wh);
        snapshot.restore();
    }

    let total_w = (max_x - min_x).ceil().max(1.0) as f32;
    let total_h = (max_y - min_y).ceil().max(1.0) as f32;
    let paintable = snapshot.to_paintable(Some(&gtk4::graphene::Size::new(total_w, total_h)))?;
    let hot_x = (cursor_x - min_x).round() as i32;
    let hot_y = (cursor_y - min_y).round() as i32;
    Some((paintable, hot_x, hot_y))
}

fn build_single_desktop_drag_paintable(widget: &GtkBox) -> gtk4::gdk::Paintable {
    let wp = gtk4::WidgetPaintable::new(Some(widget));
    let w = wp
        .intrinsic_width()
        .max(widget.width())
        .max(widget.width_request())
        .max(72) as f32;
    let h = wp
        .intrinsic_height()
        .max(widget.height())
        .max(widget.height_request())
        .max(76) as f32;
    let snapshot = gtk4::Snapshot::new();
    wp.snapshot(&snapshot, w as f64, h as f64);
    snapshot
        .to_paintable(Some(&gtk4::graphene::Size::new(w, h)))
        .unwrap_or_else(|| wp.upcast())
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
            let clipped = squircle_clip_pixbuf(pixbuf);
            let texture = gtk4::gdk::Texture::for_pixbuf(clipped.as_ref().unwrap_or(pixbuf));
            Image::from_paintable(Some(&texture))
        })
        .unwrap_or_else(|| match icon.as_ref() {
            Some(i) => {
                let img = Image::new();
                if !(path.is_dir()
                    && malus_de_common::folder_icon::apply_tinted_folder_icon(
                        &img,
                        i,
                        metrics.icon_size,
                    ))
                {
                    img.set_from_gicon(i);
                }
                img
            }
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

    unsafe {
        item_box.set_data("desktop_item_path", path.clone());
    }
    if path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
    {
        item_box.add_css_class("desktop-hidden-item");
    }
    if is_desktop_path_cut(&path) {
        item_box.add_css_class("desktop-cut-item");
    }

    let is_selected = selected_paths.borrow().contains(&path);
    set_desktop_icon_selected(&item_box, is_selected);

    let (x, mut y) = *positions.entry(path.clone()).or_insert_with(|| {
        calculate_grid_position(
            index,
            rows,
            metrics,
            config.align_left,
            desktop_size,
            top_margin,
        )
    });
    if y < top_margin {
        y = top_margin;
        positions.insert(path.clone(), (x, y));
    }
    fixed.put(&item_box, x, y);
    let _ = recently_dropped_paths.borrow_mut().remove(&path);

    // Arrastrar archivo/carpeta (Drag & Drop Wayland con icono mapeado al punto exacto de clic estilo macOS)
    let stored_click = Rc::new(Cell::new((
        f64::from(metrics.box_width) / 2.0,
        f64::from(metrics.box_height) / 2.0,
    )));
    let drag_source = gtk4::DragSource::new();
    drag_source.set_actions(gtk4::gdk::DragAction::MOVE | gtk4::gdk::DragAction::COPY);

    // Clic con botón 1 (Selección simple / doble clic para abrir)
    let click = GestureClick::new();
    click.set_button(1);
    let open_file = file.clone();
    let path_click = path.clone();
    let sel_paths_click = selected_paths.clone();
    let foc_path_click = focused_path.clone();
    let drag_fixed_press = fixed.clone();
    let drag_item_press = item_box.clone();

    click.connect_pressed(move |gesture, presses, _click_x, _click_y| {
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

    // 5b. Copiar / Cortar / Pegar en carpeta
    let copy_button = create_menu_button("Copiar", false);
    {
        let copy_path = path.clone();
        let sel_copy = selected_paths.clone();
        let fixed_copy = fixed.clone();
        let popover_c = popover.clone();
        copy_button.connect_clicked(move |_| {
            let sel = sel_copy.borrow();
            let paths = if sel.contains(&copy_path) && !sel.is_empty() {
                sel.iter().cloned().collect()
            } else {
                vec![copy_path.clone()]
            };
            drop(sel);
            set_desktop_clipboard(DesktopClipboardAction::Copy, paths);
            sync_desktop_cut_states(&fixed_copy);
            popover_c.popdown();
        });
    }
    menu.append(&copy_button);

    let cut_button = create_menu_button("Cortar", false);
    {
        let cut_path = path.clone();
        let sel_cut = selected_paths.clone();
        let fixed_cut = fixed.clone();
        let popover_c = popover.clone();
        cut_button.connect_clicked(move |_| {
            let sel = sel_cut.borrow();
            let paths = if sel.contains(&cut_path) && !sel.is_empty() {
                sel.iter().cloned().collect()
            } else {
                vec![cut_path.clone()]
            };
            drop(sel);
            set_desktop_clipboard(DesktopClipboardAction::Cut, paths);
            sync_desktop_cut_states(&fixed_cut);
            popover_c.popdown();
        });
    }
    menu.append(&cut_button);

    let is_dir_item = path.is_dir();
    let paste_into_button = create_menu_button("Pegar en carpeta", false);
    paste_into_button.set_visible(false);
    if is_dir_item {
        let paste_dir = path.clone();
        let fixed_paste = fixed.clone();
        let popover_c = popover.clone();
        paste_into_button.connect_clicked(move |_| {
            let fixed_done = fixed_paste.clone();
            popover_c.popdown();
            paste_from_desktop_clipboard_async(paste_dir.clone(), move |_pasted| {
                sync_desktop_cut_states(&fixed_done);
            });
        });
        menu.append(&paste_into_button);
    }

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
    let paste_btn_vis = paste_into_button.clone();
    context_click.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk4::EventSequenceState::Claimed);
        if is_dir_item {
            paste_btn_vis.set_visible(has_desktop_clipboard_files());
        }
        let rect = gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        p_click.set_pointing_to(Some(&rect));
        p_click.popup();
    });
    item_box.add_controller(context_click);

    let path_drag = path.clone();
    let sel_drag = selected_paths.clone();
    let pos_drag = icon_positions.clone();
    let fixed_prep = fixed.clone();
    let item_box_prep = item_box.clone();
    let stored_click_prep = stored_click.clone();
    drag_source.connect_prepare(move |source, click_x, click_y| {
        let rounded_click = (click_x.round(), click_y.round());
        stored_click_prep.set(rounded_click);
        set_desktop_icon_selected(&item_box_prep, true);

        let sel = sel_drag.borrow();
        let pos_map = pos_drag.borrow();
        let (p0x, p0y) = pos_map
            .get(&path_drag)
            .copied()
            .unwrap_or_else(|| fixed_prep.child_position(&item_box_prep));
        let cursor_desktop_x = p0x + rounded_click.0;
        let cursor_desktop_y = p0y + rounded_click.1;

        let files: Vec<gio::File> = if sel.contains(&path_drag) && sel.len() > 1 {
            let ordered_paths: Vec<PathBuf> = std::iter::once(path_drag.clone())
                .chain(sel.iter().filter(|p| *p != &path_drag).cloned())
                .collect();
            let anchors: Vec<(PathBuf, (f64, f64))> = ordered_paths
                .iter()
                .map(|p| {
                    let (ix, iy) = pos_map.get(p).copied().unwrap_or((p0x, p0y));
                    (p.clone(), (cursor_desktop_x - ix, cursor_desktop_y - iy))
                })
                .collect();
            malus_de_common::write_drag_anchors(&anchors);

            let mut drag_widgets = Vec::new();
            let mut sibling = fixed_prep.first_child();
            while let Some(child) = sibling {
                sibling = child.next_sibling();
                if let Some(p_str) = child.property::<Option<String>>("tooltip-text") {
                    let p = PathBuf::from(p_str);
                    if sel.contains(&p) {
                        let pos = pos_map
                            .get(&p)
                            .copied()
                            .unwrap_or_else(|| fixed_prep.child_position(&child));
                        drag_widgets.push((child, pos));
                    }
                }
            }
            if let Some((multi_paintable, hot_x, hot_y)) = build_multi_desktop_drag_paintable(
                &drag_widgets,
                cursor_desktop_x,
                cursor_desktop_y,
            ) {
                source.set_icon(Some(&multi_paintable), hot_x, hot_y);
            } else {
                let paintable = gtk4::WidgetPaintable::new(Some(&item_box_prep));
                source.set_icon(
                    Some(&paintable),
                    rounded_click.0 as i32,
                    rounded_click.1 as i32,
                );
            }

            ordered_paths
                .iter()
                .map(|p| gio::File::for_path(p))
                .collect()
        } else {
            let paintable = build_single_desktop_drag_paintable(&item_box_prep);
            source.set_icon(
                Some(&paintable),
                rounded_click.0 as i32,
                rounded_click.1 as i32,
            );
            malus_de_common::write_drag_anchor(&path_drag, rounded_click.0, rounded_click.1);
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
                        child.add_css_class("desktop-icon-dragging");
                    }
                }
            }
        } else {
            item_box_drag.add_css_class("desktop-icon-dragging");
        }
        *act_drag_begin.borrow_mut() =
            Some((path_drag_begin.clone(), (0.0, 0.0), (click_x, click_y)));
    });

    let fixed_drag_end = fixed.clone();
    let act_drag_end = active_icon_drag.clone();
    let icon_pos_drag_end = icon_positions.clone();
    let sel_drag_end = selected_paths.clone();
    let internal_drag_end = internal_drag_paths.clone();
    let orig_pos = (x, y);
    drag_source.connect_drag_end(move |_, drag, _| {
        *act_drag_end.borrow_mut() = None;
        let is_move = drag.selected_action().contains(gtk4::gdk::DragAction::MOVE);
        let mut removed_moved_out = false;
        let mut sibling = fixed_drag_end.first_child();
        while let Some(child) = sibling {
            sibling = child.next_sibling();
            let (cx, cy) = fixed_drag_end.child_position(&child);
            let is_dragging = child.has_css_class("desktop-icon-dragging") || cx < 0.0 || cy < 0.0;
            if is_dragging {
                if let Some(p_str) = child.property::<Option<String>>("tooltip-text") {
                    let p = PathBuf::from(p_str);
                    if p.symlink_metadata().is_err() {
                        fixed_drag_end.remove(&child);
                        icon_pos_drag_end.borrow_mut().remove(&p);
                        sel_drag_end.borrow_mut().remove(&p);
                        removed_moved_out = true;
                        continue;
                    }
                    let is_internal = internal_drag_end.borrow_mut().remove(&p);
                    if !is_move || is_internal {
                        let target_pos = icon_pos_drag_end
                            .borrow()
                            .get(&p)
                            .cloned()
                            .unwrap_or(orig_pos);
                        if cx < 0.0 || cy < 0.0 {
                            fixed_drag_end.move_(&child, target_pos.0, target_pos.1);
                        }
                        child.remove_css_class("desktop-icon-dragging");
                    }
                }
            }
        }
        if removed_moved_out {
            save_icon_positions(&icon_pos_drag_end.borrow());
        } else if is_move {
            let fixed_check = fixed_drag_end.clone();
            let pos_check = icon_pos_drag_end.clone();
            let sel_check = sel_drag_end.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || {
                let mut changed = false;
                let mut child_opt = fixed_check.first_child();
                while let Some(child) = child_opt {
                    child_opt = child.next_sibling();
                    if child.has_css_class("desktop-rubberband") {
                        continue;
                    }
                    if let Some(p_str) = child.property::<Option<String>>("tooltip-text") {
                        let p = PathBuf::from(p_str);
                        if p.symlink_metadata().is_err() {
                            fixed_check.remove(&child);
                            pos_check.borrow_mut().remove(&p);
                            sel_check.borrow_mut().remove(&p);
                            changed = true;
                        } else {
                            let (cx, cy) = fixed_check.child_position(&child);
                            if cx < 0.0 || cy < 0.0 {
                                let target_pos =
                                    pos_check.borrow().get(&p).cloned().unwrap_or(orig_pos);
                                fixed_check.move_(&child, target_pos.0, target_pos.1);
                            }
                            child.remove_css_class("desktop-icon-dragging");
                        }
                    }
                }
                if changed {
                    save_icon_positions(&pos_check.borrow());
                }
            });
        }
    });

    item_box.add_controller(drag_source);

    // Si es carpeta, permitir soltar archivos encima con resaltado azul acrílico estilo Finder
    if path.is_dir() {
        let folder_drop = gtk4::DropTarget::new(
            gtk4::gdk::FileList::static_type(),
            gtk4::gdk::DragAction::MOVE | gtk4::gdk::DragAction::COPY,
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
                                if src_file
                                    .move_(
                                        &dest_file,
                                        gio::FileCopyFlags::NONE,
                                        gio::Cancellable::NONE,
                                        None,
                                    )
                                    .is_ok()
                                {
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

type DesktopShortcutHandler = Rc<RefCell<Option<Rc<dyn Fn(String, f64, f64)>>>>;

fn monitor_target_sizes(monitor: Option<&gtk4::gdk::Monitor>) -> ((i32, i32), (i32, i32)) {
    let phys = monitor
        .map(|m| {
            let geometry = m.geometry();
            let scale = m.scale_factor().max(1);
            (
                geometry.width().max(1) * scale,
                geometry.height().max(1) * scale,
            )
        })
        .unwrap_or((1920, 1080));
    let desktop = monitor
        .map(|m| {
            let geometry = m.geometry();
            (geometry.width().max(320), geometry.height().max(240))
        })
        .unwrap_or((1920, 1080));
    (phys, desktop)
}

#[allow(clippy::too_many_arguments)]
fn create_wallpaper_window(
    application: &Application,
    monitor: Option<&gtk4::gdk::Monitor>,
    cli_image: &Option<PathBuf>,
    desktop_dir: Option<&Path>,
    desktop_monitors: &Rc<RefCell<Vec<gio::FileMonitor>>>,
    icon_positions: &Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    internal_drag_paths: &Rc<RefCell<HashSet<PathBuf>>>,
    active_icon_drag: &Rc<RefCell<Option<(PathBuf, (f64, f64), (f64, f64))>>>,
    shortcut_handler: &DesktopShortcutHandler,
    is_primary: bool,
) -> WallpaperWindowSlot {
    let window = ApplicationWindow::builder()
        .application(application)
        .title("Fondo de Pantalla Malus DE")
        .decorated(false)
        .css_classes(["malus-wallpaper-window"])
        .build();
    window.connect_close_request(|_| glib::Propagation::Stop);

    if gtk4_layer_shell::is_supported() {
        info!("Anclando Fondo de Pantalla en capa Background a pantalla completa...");
        window.init_layer_shell();
        if let Some(mon) = monitor {
            window.set_monitor(mon);
            disconnect_layer_shell_monitor_invalidate(mon);
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

    let root_overlay = Overlay::new();
    let ((target_phys_w, target_phys_h), (desktop_w, desktop_h)) = monitor_target_sizes(monitor);

    let picture = Picture::new();
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_halign(Align::Fill);
    picture.set_valign(Align::Fill);
    apply_wallpaper_to_picture(&picture, &window, cli_image, target_phys_w, target_phys_h);
    root_overlay.set_child(Some(&picture));

    let (role_host, key_controller) = populate_wallpaper_role(
        &window,
        desktop_dir,
        (desktop_w, desktop_h),
        desktop_monitors,
        icon_positions,
        internal_drag_paths,
        active_icon_drag,
        shortcut_handler,
        is_primary,
    );
    root_overlay.add_overlay(&role_host);

    window.set_child(Some(&root_overlay));
    window.present();
    WallpaperWindowSlot {
        monitor: monitor.cloned(),
        window,
        root_overlay,
        picture,
        role_host: Some(role_host),
        key_controller,
        phys_w: target_phys_w,
        phys_h: target_phys_h,
        desktop_w,
        desktop_h,
        is_primary,
    }
}

#[allow(clippy::too_many_arguments)]
fn populate_wallpaper_role(
    window: &ApplicationWindow,
    desktop_dir: Option<&Path>,
    desktop_size: (i32, i32),
    desktop_monitors: &Rc<RefCell<Vec<gio::FileMonitor>>>,
    icon_positions: &Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    internal_drag_paths: &Rc<RefCell<HashSet<PathBuf>>>,
    active_icon_drag: &Rc<RefCell<Option<(PathBuf, (f64, f64), (f64, f64))>>>,
    shortcut_handler: &DesktopShortcutHandler,
    is_primary: bool,
) -> (Overlay, Option<EventControllerKey>) {
    let overlay = Overlay::builder()
        .hexpand(true)
        .vexpand(true)
        .halign(Align::Fill)
        .valign(Align::Fill)
        .build();
    let hit_target = GtkBox::builder()
        .hexpand(true)
        .vexpand(true)
        .halign(Align::Fill)
        .valign(Align::Fill)
        .build();
    overlay.set_child(Some(&hit_target));
    let mut attached_key_ctrl: Option<EventControllerKey> = None;

    if let Some(desktop_dir) = desktop_dir {

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
        let expanded_stacks: Rc<RefCell<HashSet<StackCategory>>> =
            Rc::new(RefCell::new(HashSet::new()));
        let recently_dropped_paths: Rc<RefCell<HashSet<PathBuf>>> =
            Rc::new(RefCell::new(HashSet::new()));

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
        watch_clipboard_for_desktop(&icons);

        // Controladores de teclado en la ventana del escritorio:
        // Espacio -> Quick Look, Enter -> Abrir, Delete -> Papelera, Ctrl/Cmd+C/X/V/D/A
        let key_controller = EventControllerKey::new();
        let win_key = window.clone();
        let foc_path_key = focused_path.clone();
        let sel_paths_key = selected_paths.clone();
        let icons_key = icons.clone();
        let dir_key = desktop_dir.to_path_buf();
        let trigger_key = trigger_refresh.clone();
        key_controller.connect_key_pressed(move |_, keyval, _, state| {
            if gtk4::prelude::GtkWindowExt::focus(&win_key)
                .is_some_and(|w| w.is::<gtk4::Editable>() || w.is::<gtk4::Text>())
            {
                return glib::Propagation::Proceed;
            }

            let is_alt = state.contains(gtk4::gdk::ModifierType::ALT_MASK);
            let is_ctrl = state.contains(gtk4::gdk::ModifierType::CONTROL_MASK);
            let is_command = state.intersects(
                gtk4::gdk::ModifierType::SUPER_MASK | gtk4::gdk::ModifierType::META_MASK,
            );
            let is_shift = state.contains(gtk4::gdk::ModifierType::SHIFT_MASK);

            if (is_ctrl || is_command) && !is_alt && !is_shift {
                match keyval {
                    Key::c | Key::C => {
                        let sel = sel_paths_key.borrow();
                        let paths: Vec<PathBuf> = if !sel.is_empty() {
                            sel.iter().cloned().collect()
                        } else if let Some(p) = foc_path_key.borrow().as_ref() {
                            vec![p.clone()]
                        } else {
                            Vec::new()
                        };
                        drop(sel);
                        if !paths.is_empty() {
                            set_desktop_clipboard(DesktopClipboardAction::Copy, paths);
                            sync_desktop_cut_states(&icons_key);
                            return glib::Propagation::Stop;
                        }
                    }
                    Key::x | Key::X => {
                        let sel = sel_paths_key.borrow();
                        let paths: Vec<PathBuf> = if !sel.is_empty() {
                            sel.iter().cloned().collect()
                        } else if let Some(p) = foc_path_key.borrow().as_ref() {
                            vec![p.clone()]
                        } else {
                            Vec::new()
                        };
                        drop(sel);
                        if !paths.is_empty() {
                            set_desktop_clipboard(DesktopClipboardAction::Cut, paths);
                            sync_desktop_cut_states(&icons_key);
                            return glib::Propagation::Stop;
                        }
                    }
                    Key::v | Key::V => {
                        let trig_done = trigger_key.clone();
                        let icons_done = icons_key.clone();
                        paste_from_desktop_clipboard_async(dir_key.clone(), move |pasted| {
                            if !pasted.is_empty() {
                                trig_done();
                            }
                            sync_desktop_cut_states(&icons_done);
                        });
                        return glib::Propagation::Stop;
                    }
                    Key::d | Key::D => {
                        let sel = sel_paths_key.borrow();
                        let paths: Vec<PathBuf> = if !sel.is_empty() {
                            sel.iter().cloned().collect()
                        } else if let Some(p) = foc_path_key.borrow().as_ref() {
                            vec![p.clone()]
                        } else {
                            Vec::new()
                        };
                        drop(sel);
                        if !paths.is_empty() {
                            for p in paths {
                                duplicate_file(&p);
                            }
                            trigger_key();
                            return glib::Propagation::Stop;
                        }
                    }
                    Key::a | Key::A => {
                        let mut sel = sel_paths_key.borrow_mut();
                        sel.clear();
                        let mut sibling = icons_key.first_child();
                        while let Some(child) = sibling {
                            sibling = child.next_sibling();
                            if !child.has_css_class("desktop-icon") {
                                continue;
                            }
                            if let Some(p) = unsafe { child.data::<PathBuf>("desktop_item_path") } {
                                let p_clone = unsafe { p.as_ref() }.clone();
                                set_desktop_icon_selected(&child, true);
                                sel.insert(p_clone);
                            }
                        }
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }

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
        window.add_controller(key_controller.clone());
        attached_key_ctrl = Some(key_controller);

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
        let press_origin = Rc::new(Cell::new((0.0_f64, 0.0_f64)));
        let press_origin_p = press_origin.clone();
        bg_click.connect_pressed(move |_, _, x, y| {
            press_origin_p.set((x, y));
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
        let icons_bg_rel = icons.clone();
        let rb_check_rel = rubberband.clone();
        bg_click.connect_released(move |_, n_press, x, y| {
            if n_press != 1 || rb_check_rel.is_visible() {
                return;
            }
            let (px, py) = press_origin.get();
            if (x - px).hypot(y - py) > 6.0 {
                return;
            }
            if is_point_over_icon(&icons_bg_rel, x, y, &rb_check_rel) {
                return;
            }
            let dd_cfg = malus_de_common::config::DesktopDockConfig::load();
            if dd_cfg.click_wallpaper_shows_desktop == 0 {
                let _ = malus_de_common::ipc::IpcClient::send_command(
                    malus_de_common::ipc::IpcMessage::ToggleShowDesktop,
                );
            }
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
        let sel_paths_ctx = selected_paths.clone();
        let foc_path_ctx = focused_path.clone();
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
                .transition_duration(120)
                .interpolate_size(false)
                .css_classes(["desktop-context-menu"])
                .build();

            let create_new_box = build_create_new_submenu(
                &popover,
                &stack,
                &icons_context,
                desktop_dir_menu.clone(),
                desktop_size,
                cfg_menu.clone(),
                icon_pos_snap.clone(),
                sel_paths_ctx.clone(),
                foc_path_ctx.clone(),
                trigger_refresh_menu.clone(),
                (x, y),
                top_margin,
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

            // 1. Crear nuevo ▸ (Carpeta / Documento de texto con edición inline inmediata)
            let create_new_btn = create_menu_button("Crear nuevo ▸", false);
            let stack_cn = stack.clone();
            create_new_btn.connect_clicked(move |_| {
                stack_cn.set_visible_child_name("create_new");
            });
            menu.append(&create_new_btn);

            if has_desktop_clipboard_files() {
                let paste_btn = create_menu_button("Pegar", false);
                let popover_c = popover.clone();
                let paste_dir = desktop_dir_menu.clone();
                let pos_paste = icon_pos_snap.clone();
                let trig_paste = trigger_refresh_menu.clone();
                let icons_paste = icons_context.clone();
                paste_btn.connect_clicked(move |_| {
                    popover_c.popdown();
                    let pos_map_done = pos_paste.clone();
                    let trig_done = trig_paste.clone();
                    let icons_done = icons_paste.clone();
                    paste_from_desktop_clipboard_async(paste_dir.clone(), move |pasted| {
                        if !pasted.is_empty() {
                            {
                                let mut pos_map = pos_map_done.borrow_mut();
                                let base_y = y.max(top_margin);
                                for (idx, p) in pasted.iter().enumerate() {
                                    pos_map.insert(
                                        p.clone(),
                                        (x + (idx as f64) * 24.0, base_y + (idx as f64) * 24.0),
                                    );
                                }
                                save_icon_positions(&pos_map);
                            }
                            trig_done();
                        }
                        sync_desktop_cut_states(&icons_done);
                    });
                });
                menu.append(&paste_btn);
            }

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
                        DESKTOP_ORGANIZE_ANIM_MS,
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
                        DESKTOP_ORGANIZE_ANIM_MS,
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
                            DESKTOP_ORGANIZE_ANIM_MS,
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
                malus_de_common::config::apply_isolated_environment(&mut cmd);
                let _ = cmd.spawn();
                popover_c.popdown();
            });
            menu.append(&wp_btn);

            stack.add_named(&menu, Some("main"));
            stack.add_named(&organize_box, Some("organize"));
            stack.add_named(&create_new_box, Some("create_new"));
            stack.set_visible_child_name("main");

            popover.set_child(Some(&stack));
            popover.connect_closed(|p| p.unparent());
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
            gtk4::gdk::DragAction::MOVE | gtk4::gdk::DragAction::COPY,
        );
        let drop_dir = desktop_dir.to_path_buf();
        let drop_positions = icon_positions.clone();
        let internal_paths_drop = internal_drag_paths.clone();

        // En Wayland el cursor lleva consigo la miniatura flotante (WidgetPaintable).
        // No movemos el widget del escritorio durante connect_motion para evitar cualquier
        // estela o duplicación semitransparente detrás del puntero.
        drop_target.connect_enter(move |_, _x, _y| gtk4::gdk::DragAction::MOVE);
        drop_target.connect_motion(move |_, _x, _y| gtk4::gdk::DragAction::MOVE);

        let active_drag_drop = active_icon_drag.clone();
        let drop_icons = icons.clone();
        let cfg_drop = current_config.clone();
        let rec_drop = recently_dropped_paths.clone();
        let sel_drop = selected_paths.clone();
        let foc_drop = focused_path.clone();

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

            let cfg_snapshot = cfg_drop.borrow().clone();
            let metrics = IconLayoutMetrics::from_config(&cfg_snapshot);
            let snap = cfg_snapshot.snap_to_grid;
            let align_left = cfg_snapshot.align_left;
            let rows = ((desktop_size.1 - (top_margin as i32) - 80).max(1) / metrics.cell_height())
                .max(1) as usize;

            let mut moved_any = false;
            let mut cleared_selection_for_external = false;
            let mut snap_anim_targets: HashMap<PathBuf, (f64, f64)> = HashMap::new();
            for (index, source) in files.files().into_iter().enumerate() {
                let Some(name) = source.basename() else {
                    warn!(
                        "No se pudo mover al escritorio un elemento sin nombre: {:?}",
                        source
                    );
                    continue;
                };
                let destination = gio::File::for_path(&drop_dir).child(&name);
                let shared_anchor = source
                    .path()
                    .and_then(|src_path| malus_de_common::read_drag_anchor(&src_path));
                let offset_idx = if shared_anchor.is_some() {
                    0.0
                } else {
                    (index % 4) as f64 * 24.0
                };
                if source.equal(&destination) {
                    if let Some(path) = source.path() {
                        let pointer_offset = shared_anchor
                            .or_else(|| {
                                active_drag_drop
                                    .borrow()
                                    .as_ref()
                                    .filter(|(active_path, _, _)| active_path == &path)
                                    .map(|(_, _, pointer_offset)| *pointer_offset)
                            })
                            .unwrap_or((
                                f64::from(metrics.box_width) / 2.0,
                                f64::from(metrics.box_height) / 2.0,
                            ));
                        let raw_pos = clamp_icon_position(
                            x + offset_idx,
                            y + offset_idx,
                            pointer_offset,
                            &metrics,
                            desktop_size,
                            top_margin,
                        );
                        let final_pos = if snap {
                            snap_position_to_grid(
                                raw_pos,
                                &metrics,
                                align_left,
                                desktop_size,
                                top_margin,
                            )
                        } else {
                            raw_pos
                        };
                        // Colocar primero exactamente en el punto de suelta (1:1 con la miniatura del cursor)
                        // y luego deslizar suavemente hacia la celda de la cuadrícula si `snap_to_grid` está activo.
                        if move_desktop_icon(&drop_icons, &path, raw_pos) {
                            drop_positions.borrow_mut().insert(path.clone(), final_pos);
                            internal_paths_drop.borrow_mut().insert(path.clone());
                            if snap
                                && ((final_pos.0 - raw_pos.0).abs() > 0.5
                                    || (final_pos.1 - raw_pos.1).abs() > 0.5)
                            {
                                snap_anim_targets.insert(path.clone(), final_pos);
                            }
                            moved_any = true;
                        }
                    }
                    continue;
                }

                let pointer_offset = shared_anchor.unwrap_or((
                    f64::from(metrics.box_width) / 2.0,
                    f64::from(metrics.icon_size) / 2.0,
                ));
                let mut position = clamp_icon_position(
                    x + offset_idx,
                    y + offset_idx,
                    pointer_offset,
                    &metrics,
                    desktop_size,
                    top_margin,
                );
                if snap {
                    position = snap_position_to_grid(
                        position,
                        &metrics,
                        align_left,
                        desktop_size,
                        top_margin,
                    );
                }

                let is_app_shortcut = source
                    .path()
                    .as_deref()
                    .is_some_and(malus_de_common::apps::is_application_desktop_file);

                let transfer_result: Result<(), String> = if is_app_shortcut {
                    if let (Some(src_path), Some(dest_path)) = (source.path(), destination.path()) {
                        if dest_path.symlink_metadata().is_ok() {
                            if has_desktop_icon_widget(&drop_icons, &dest_path)
                                && move_desktop_icon(&drop_icons, &dest_path, position)
                            {
                                drop_positions
                                    .borrow_mut()
                                    .insert(dest_path.clone(), position);
                                internal_paths_drop.borrow_mut().insert(dest_path);
                                moved_any = true;
                                continue;
                            }
                            Err(
                                "Ya existe un acceso directo con ese nombre en el escritorio"
                                    .to_string(),
                            )
                        } else {
                            std::os::unix::fs::symlink(&src_path, &dest_path)
                                .map_err(|e| e.to_string())
                        }
                    } else {
                        Err("Ruta inválida para el acceso directo de aplicación".to_string())
                    }
                } else {
                    source
                        .move_(
                            &destination,
                            gio::FileCopyFlags::NONE,
                            gio::Cancellable::NONE,
                            None,
                        )
                        .map_err(|e| e.to_string())
                };

                match transfer_result {
                    Ok(()) => {
                        if let Some(path) = destination.path() {
                            drop_positions.borrow_mut().insert(path.clone(), position);
                            internal_paths_drop.borrow_mut().insert(path.clone());
                            rec_drop.borrow_mut().insert(path.clone());

                            if !cfg_snapshot.use_stacks {
                                if !cleared_selection_for_external {
                                    let mut sibling = drop_icons.first_child();
                                    while let Some(child) = sibling {
                                        sibling = child.next_sibling();
                                        set_desktop_icon_selected(&child, false);
                                    }
                                    sel_drop.borrow_mut().clear();
                                    cleared_selection_for_external = true;
                                }
                                sel_drop.borrow_mut().insert(path.clone());
                                *foc_drop.borrow_mut() = Some(path.clone());
                                if !has_desktop_icon_widget(&drop_icons, &path) {
                                    if let Some(item_data) = build_desktop_item_data(&path) {
                                        let mut pos_map = drop_positions.borrow_mut();
                                        render_single_icon(
                                            &drop_icons,
                                            0,
                                            rows,
                                            item_data,
                                            &metrics,
                                            &cfg_snapshot,
                                            desktop_size,
                                            &mut pos_map,
                                            &drop_positions,
                                            &internal_paths_drop,
                                            &active_drag_drop,
                                            &sel_drop,
                                            &foc_drop,
                                            &rec_drop,
                                            top_margin,
                                        );
                                    }
                                }
                            }
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
                if !snap_anim_targets.is_empty() {
                    let pos_for_save = drop_positions.clone();
                    animate_desktop_layout_transition_with_completion(
                        &drop_icons,
                        &snap_anim_targets,
                        180,
                        Some(Box::new(move || {
                            save_icon_positions(&pos_for_save.borrow());
                        })),
                    );
                } else {
                    save_icon_positions(&drop_positions.borrow());
                }
                drop_icons.queue_draw();
            }
            moved_any
        });

        overlay.add_controller(drop_target);
        overlay.add_overlay(&icons);

        // Handler IPC para accesos directos arrastrados desde Spotlight al escritorio
        {
            let sc_dir = desktop_dir.to_path_buf();
            let sc_positions = icon_positions.clone();
            let sc_internal = internal_drag_paths.clone();
            let sc_active_drag = active_icon_drag.clone();
            let sc_icons = icons.clone();
            let sc_cfg = current_config.clone();
            let sc_rec = recently_dropped_paths.clone();
            let sc_sel = selected_paths.clone();
            let sc_foc = focused_path.clone();
            *shortcut_handler.borrow_mut() = Some(Rc::new(move |desktop_file, drop_x, drop_y| {
                let src_path = PathBuf::from(&desktop_file);
                let Some(name) = src_path.file_name() else {
                    return;
                };
                let dest_path = sc_dir.join(name);
                let cfg_snapshot = sc_cfg.borrow().clone();
                let metrics = IconLayoutMetrics::from_config(&cfg_snapshot);
                let rows = ((desktop_size.1 - (top_margin as i32) - 80).max(1)
                    / metrics.cell_height())
                .max(1) as usize;
                let pointer_offset = (
                    f64::from(metrics.box_width) / 2.0,
                    f64::from(metrics.icon_size) / 2.0,
                );
                let mut position = clamp_icon_position(
                    drop_x,
                    drop_y,
                    pointer_offset,
                    &metrics,
                    desktop_size,
                    top_margin,
                );
                if cfg_snapshot.snap_to_grid {
                    position = snap_position_to_grid(
                        position,
                        &metrics,
                        cfg_snapshot.align_left,
                        desktop_size,
                        top_margin,
                    );
                }

                if dest_path.symlink_metadata().is_ok() {
                    if has_desktop_icon_widget(&sc_icons, &dest_path)
                        && move_desktop_icon(&sc_icons, &dest_path, position)
                    {
                        sc_positions
                            .borrow_mut()
                            .insert(dest_path.clone(), position);
                        sc_internal.borrow_mut().insert(dest_path);
                        save_icon_positions(&sc_positions.borrow());
                    }
                    return;
                }

                if let Err(err) = std::os::unix::fs::symlink(&src_path, &dest_path) {
                    warn!(
                        "No se pudo crear acceso directo en el escritorio {:?} -> {:?}: {}",
                        dest_path, src_path, err
                    );
                    return;
                }

                sc_positions
                    .borrow_mut()
                    .insert(dest_path.clone(), position);
                sc_internal.borrow_mut().insert(dest_path.clone());
                sc_rec.borrow_mut().insert(dest_path.clone());

                if !cfg_snapshot.use_stacks {
                    let mut sibling = sc_icons.first_child();
                    while let Some(child) = sibling {
                        sibling = child.next_sibling();
                        set_desktop_icon_selected(&child, false);
                    }
                    sc_sel.borrow_mut().clear();
                    sc_sel.borrow_mut().insert(dest_path.clone());
                    *sc_foc.borrow_mut() = Some(dest_path.clone());
                    if !has_desktop_icon_widget(&sc_icons, &dest_path) {
                        if let Some(item_data) = build_desktop_item_data(&dest_path) {
                            let mut pos_map = sc_positions.borrow_mut();
                            render_single_icon(
                                &sc_icons,
                                0,
                                rows,
                                item_data,
                                &metrics,
                                &cfg_snapshot,
                                desktop_size,
                                &mut pos_map,
                                &sc_positions,
                                &sc_internal,
                                &sc_active_drag,
                                &sc_sel,
                                &sc_foc,
                                &sc_rec,
                                top_margin,
                            );
                        }
                    }
                }
                save_icon_positions(&sc_positions.borrow());
            }));
        }

        // Monitor en tiempo real para reflejar cambios en la carpeta ~/Escritorio
        let desktop_file = gio::File::for_path(desktop_dir);
        match desktop_file.monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
            Ok(monitor) => {
                let trigger_c = trigger_refresh.clone();
                let icons_mon = icons.clone();
                let rec_mon = recently_dropped_paths.clone();
                let pos_mon = icon_positions.clone();
                let sel_mon = selected_paths.clone();
                let foc_mon = focused_path.clone();
                let internal_mon = internal_drag_paths.clone();
                let desktop_dir_mon = desktop_dir.to_path_buf();
                monitor.connect_changed(move |_, file, _other_file, event_type| {
                    let event_path = file.path();
                    if event_path.as_ref() == Some(&desktop_dir_mon) {
                        return;
                    }
                    match event_type {
                        gio::FileMonitorEvent::Deleted | gio::FileMonitorEvent::MovedOut => {
                            if let Some(ref p) = event_path {
                                let removed_widget = remove_desktop_icon_widget(&icons_mon, p);
                                let removed_pos = pos_mon.borrow_mut().remove(p).is_some();
                                sel_mon.borrow_mut().remove(p);
                                internal_mon.borrow_mut().remove(p);
                                if foc_mon.borrow().as_ref() == Some(p) {
                                    *foc_mon.borrow_mut() = None;
                                }
                                if removed_pos {
                                    save_icon_positions(&pos_mon.borrow());
                                }
                                if removed_widget || !has_desktop_icon_widget(&icons_mon, p) {
                                    return;
                                }
                            }
                        }
                        gio::FileMonitorEvent::Created
                        | gio::FileMonitorEvent::MovedIn
                        | gio::FileMonitorEvent::ChangesDoneHint
                        | gio::FileMonitorEvent::AttributeChanged
                        | gio::FileMonitorEvent::Changed => {
                            if let Some(ref p) = event_path {
                                if rec_mon.borrow_mut().remove(p)
                                    || has_desktop_icon_widget(&icons_mon, p)
                                {
                                    return;
                                }
                            }
                        }
                        _ => {}
                    }
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
        if let Ok(cfg_monitor) =
            config_file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            let cfg_holder = current_config.clone();
            let trigger_c = trigger_refresh.clone();
            cfg_monitor.connect_changed(move |_, _, _, _| {
                let new_cfg = DesktopIconsConfig::load();
                let needs_rebuild = {
                    let prev = cfg_holder.borrow();
                    prev.show_icons != new_cfg.show_icons
                        || prev.icon_size != new_cfg.icon_size
                        || prev.align_left != new_cfg.align_left
                        || prev.use_stacks != new_cfg.use_stacks
                };
                *cfg_holder.borrow_mut() = new_cfg;
                if needs_rebuild {
                    trigger_c();
                }
            });
            desktop_monitors.borrow_mut().push(cfg_monitor);
        }

        let app_cfg_file = gio::File::for_path(malus_de_common::config::appearance_settings_path());
        if let Ok(app_monitor) =
            app_cfg_file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            let last_hex =
                Rc::new(RefCell::new(malus_de_common::config::load_folder_tint_hex()));
            let trigger_c = trigger_refresh.clone();
            app_monitor.connect_changed(move |_, _, _, _| {
                let next_hex = malus_de_common::config::load_folder_tint_hex();
                if *last_hex.borrow() != next_hex {
                    *last_hex.borrow_mut() = next_hex;
                    trigger_c();
                }
            });
            desktop_monitors.borrow_mut().push(app_monitor);
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
                .spacing(1)
                .margin_top(0)
                .margin_bottom(0)
                .margin_start(0)
                .margin_end(0)
                .css_classes(["desktop-context-menu"])
                .build();

            let wp_btn = create_menu_button("Fondo de pantalla y Estilo...", false);
            let popover_c = popover.clone();
            wp_btn.connect_clicked(move |_| {
                let mut cmd = std::process::Command::new("malus-de-settings");
                cmd.arg("--panel").arg("wallpaper");
                malus_de_common::config::apply_isolated_environment(&mut cmd);
                let _ = cmd.spawn();
                popover_c.popdown();
            });
            menu.append(&wp_btn);

            popover.set_child(Some(&menu));
            popover.connect_closed(|p| p.unparent());
            popover.popup();
        });
        overlay.add_controller(bg_context);
    }

    (overlay, attached_key_ctrl)
}

fn resolve_wallpaper_image_path(image_path: &Option<PathBuf>) -> Option<PathBuf> {
    image_path.clone().or_else(|| {
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
        candidates.into_iter().find(|p| p.exists()).or_else(|| {
            if config_dir.join(".wallpaper-gradient").exists() {
                None
            } else {
                malus_de_common::config::default_bundled_wallpaper_path()
            }
        })
    })
}

fn select_primary_monitor_index(
    monitor_list: &[gtk4::gdk::Monitor],
    displays_cfg: &DisplaysConfig,
) -> usize {
    displays_cfg
        .primary_output()
        .and_then(|cfg| {
            monitor_list.iter().position(|monitor| {
                let geo = monitor.geometry();
                cfg.matches_monitor_identity(
                    monitor.connector().as_deref(),
                    monitor.model().as_deref(),
                    monitor.description().as_deref(),
                    geo.x(),
                    geo.y(),
                )
            })
        })
        .unwrap_or(0)
}

fn main() -> Result<()> {
    init_logging("malus_de_wallpaper");
    let args = Args::parse();
    info!("Iniciando Malus DE Wallpaper Daemon...");

    // Inicializar entorno aislado de Malus DE y limpiar variables residuales
    let _ = malus_de_common::config::setup_isolated_session_environment();

    // Esperar de forma activa a que el compositor esté listo y aceptando conexiones
    malus_de_common::wayland::wait_for_wayland(10);

    let app = Application::builder()
        .application_id("org.malus_de.Wallpaper")
        .build();

    let image_path = args.image;
    let desktop_dir = resolve_desktop_dir();
    let desktop_monitors = Rc::new(RefCell::new(Vec::new()));
    let wallpaper_surfaces: Rc<RefCell<Vec<WallpaperWindowSlot>>> =
        Rc::new(RefCell::new(Vec::new()));
    let parked_windows: Rc<RefCell<Vec<ApplicationWindow>>> = Rc::new(RefCell::new(Vec::new()));
    let wallpaper_cfg_monitors: Rc<RefCell<Vec<gio::FileMonitor>>> =
        Rc::new(RefCell::new(Vec::new()));
    let hold_guard: Rc<RefCell<Option<gio::ApplicationHoldGuard>>> = Rc::new(RefCell::new(None));
    let sync_handler: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let sync_handler_ipc = sync_handler.clone();
    let bg_refresh_handler: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let bg_refresh_ipc = bg_refresh_handler.clone();
    let icon_positions = Rc::new(RefCell::new(load_icon_positions()));
    let internal_drag_paths = Rc::new(RefCell::new(HashSet::new()));
    let active_icon_drag = Rc::new(RefCell::new(None));
    let shortcut_handler: DesktopShortcutHandler = Rc::new(RefCell::new(None));
    let shortcut_handler_ipc = shortcut_handler.clone();

    app.connect_startup(move |_| {
        let initial_dark = malus_de_common::config::is_dark_mode_active();
        set_is_dark_mode(initial_dark);

        // 1. Escuchar mensajes IPC del compositor (ThemeChanged, ReloadWallpaperConfig, ReloadDisplaysConfig y accesos de Spotlight)
        let (tx, rx) = async_channel::unbounded::<malus_de_common::ipc::IpcMessage>();
        let shortcut_handler_rx = shortcut_handler_ipc.clone();
        let sync_handler_rx = sync_handler_ipc.clone();
        let bg_refresh_rx = bg_refresh_ipc.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                match msg {
                    malus_de_common::ipc::IpcMessage::ThemeChanged { is_dark, .. } => {
                        info!("🎨 [Wallpaper] IPC ThemeChanged recibido: is_dark={}", is_dark);
                        set_is_dark_mode(is_dark);
                        if let Some(ref refresh_bg) = *bg_refresh_rx.borrow() {
                            refresh_bg();
                        }
                    }
                    malus_de_common::ipc::IpcMessage::ReloadWallpaperConfig => {
                        info!("🖼 [Wallpaper] IPC ReloadWallpaperConfig recibido — actualizando fondo");
                        if let Some(ref refresh_bg) = *bg_refresh_rx.borrow() {
                            refresh_bg();
                        }
                    }
                    malus_de_common::ipc::IpcMessage::LiquidGlassOpacityChanged { opacity } => {
                        info!("✨ [Wallpaper] IPC LiquidGlassOpacityChanged recibido: opacity={:.2}", opacity);
                        apply_wallpaper_liquid_glass_opacity(opacity);
                    }
                    malus_de_common::ipc::IpcMessage::CreateDesktopAppShortcut {
                        desktop_file,
                        drop_x,
                        drop_y,
                    } => {
                        if let Some(ref handler) = *shortcut_handler_rx.borrow() {
                            handler(desktop_file, drop_x, drop_y);
                        }
                    }
                    malus_de_common::ipc::IpcMessage::ReloadDisplaysConfig => {
                        info!("🖥 [Wallpaper] IPC ReloadDisplaysConfig recibido — sincronizando monitores");
                        if let Some(ref trigger) = *sync_handler_rx.borrow() {
                            trigger();
                        }
                    }
                    _ => {}
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
            let bg_refresh_gs = bg_refresh_ipc.clone();
            gsettings.connect_changed(Some("color-scheme"), move |settings, _| {
                let scheme = settings.string("color-scheme");
                let to_dark = scheme.contains("dark");
                info!("🎨 [Wallpaper] GSettings color-scheme: {}", scheme);
                set_is_dark_mode(to_dark);
                if let Some(ref refresh_bg) = *bg_refresh_gs.borrow() {
                    refresh_bg();
                }
            });
            Box::leak(Box::new(gsettings));
        }

        // 3. Registrar hoja de estilo CSS
        let provider = CssProvider::new();
        provider.load_from_string(DEFAULT_GRADIENT_CSS);
        let glass_provider = CssProvider::new();
        if let Some(display) = gtk4::gdk::Display::default() {
            let icon_theme = gtk4::IconTheme::for_display(&display);
            for icon_dir in malus_de_common::apps::extra_icon_search_paths() {
                if icon_dir.is_dir() {
                    icon_theme.add_search_path(&icon_dir);
                }
            }
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_USER + 100,
            );
            gtk4::style_context_add_provider_for_display(
                &display,
                &glass_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_USER + 101,
            );
        }
        WALLPAPER_GLASS_PROVIDER.with(|cell| {
            *cell.borrow_mut() = Some(glass_provider);
        });
        apply_wallpaper_liquid_glass_opacity(malus_de_common::config::load_liquid_glass_opacity());
    });

    let monitors_hooked = Rc::new(Cell::new(false));
    app.connect_activate(move |application| {
        if hold_guard.borrow().is_none() {
            *hold_guard.borrow_mut() = Some(application.hold());
        }

        if let Some(desktop_dir) = &desktop_dir {
            info!(
                "Mostrando contenido de la carpeta de escritorio: {:?}",
                desktop_dir
            );
        } else {
            warn!("No se pudo determinar la carpeta de escritorio del usuario.");
        }

        let app_sync = application.clone();
        let image_path_sync = image_path.clone();
        let image_path_bg = image_path.clone();
        let desktop_dir_sync = desktop_dir.clone();
        let desktop_monitors_sync = desktop_monitors.clone();
        let wallpaper_surfaces_sync = wallpaper_surfaces.clone();
        let wallpaper_surfaces_bg = wallpaper_surfaces.clone();
        let parked_windows_sync = parked_windows.clone();
        let icon_positions_sync = icon_positions.clone();
        let internal_drag_sync = internal_drag_paths.clone();
        let active_drag_sync = active_icon_drag.clone();
        let shortcut_handler_sync = shortcut_handler.clone();

        let refresh_backgrounds: Rc<dyn Fn()> = Rc::new(move || {
            for slot in wallpaper_surfaces_bg.borrow().iter() {
                apply_wallpaper_to_picture(
                    &slot.picture,
                    &slot.window,
                    &image_path_bg,
                    slot.phys_w,
                    slot.phys_h,
                );
            }
        });
        let pending_bg_refresh: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        let schedule_bg_refresh: Rc<dyn Fn()> = {
            let refresh_c = refresh_backgrounds.clone();
            let pending_c = pending_bg_refresh.clone();
            Rc::new(move || {
                if let Some(source) = pending_c.borrow_mut().take() {
                    source.remove();
                }
                let cb = refresh_c.clone();
                let done_c = pending_c.clone();
                let source = glib::timeout_add_local_once(
                    std::time::Duration::from_millis(60),
                    move || {
                        *done_c.borrow_mut() = None;
                        cb();
                    },
                );
                *pending_c.borrow_mut() = Some(source);
            })
        };
        *bg_refresh_handler.borrow_mut() = Some(schedule_bg_refresh.clone());

        if wallpaper_cfg_monitors.borrow().is_empty() {
            let wp_cfg_file = gio::File::for_path(WallpaperConfig::config_path());
            if let Ok(mon) =
                wp_cfg_file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
            {
                let sched = schedule_bg_refresh.clone();
                mon.connect_changed(move |_, _, _, _| {
                    sched();
                });
                wallpaper_cfg_monitors.borrow_mut().push(mon);
            }
        }

        let geometry_retry_count: Rc<Cell<u8>> = Rc::new(Cell::new(0));
        let sync_handler_mon = sync_handler.clone();

        let sync_windows: Rc<dyn Fn()> = Rc::new(move || {
            let displays_cfg = DisplaysConfig::load();
            let primary_cfg = displays_cfg.primary_output().cloned();
            let monitor_list: Vec<gtk4::gdk::Monitor> = gtk4::gdk::Display::default()
                .map(|display| {
                    let monitors = display.monitors();
                    (0..monitors.n_items())
                        .filter_map(|i| monitors.item(i))
                        .filter_map(|monitor| monitor.downcast::<gtk4::gdk::Monitor>().ok())
                        .filter(|monitor| monitor.is_valid())
                        .collect()
                })
                .unwrap_or_default();

            for monitor in &monitor_list {
                ensure_monitor_sync_hooks(monitor, &sync_handler_mon);
            }

            let pending_geometry = monitor_list.iter().any(|m| !monitor_geometry_ready(m));
            if pending_geometry && geometry_retry_count.get() < 10 {
                geometry_retry_count.set(geometry_retry_count.get() + 1);
                info!(
                    "⏳ [Wallpaper] Esperando geometría inicial de monitor tras hotplug (intento {}/10)",
                    geometry_retry_count.get()
                );
                if let Some(sched) = sync_handler_mon.borrow().as_ref() {
                    sched();
                }
                return;
            }
            geometry_retry_count.set(0);

            let primary_index = select_primary_monitor_index(&monitor_list, &displays_cfg);
            let mut prev_slots: Vec<WallpaperWindowSlot> =
                wallpaper_surfaces_sync.borrow_mut().drain(..).collect();

            if monitor_list.is_empty() {
                let mut kept_fallback: Option<WallpaperWindowSlot> = None;
                for slot in prev_slots {
                    if kept_fallback.is_none() && slot.monitor.is_none() {
                        kept_fallback = Some(slot);
                    } else {
                        close_all_registered_popovers();
                        parked_windows_sync
                            .borrow_mut()
                            .push(park_wallpaper_slot(slot));
                    }
                }
                let fallback_slot = kept_fallback.unwrap_or_else(|| {
                    create_wallpaper_window(
                        &app_sync,
                        None,
                        &image_path_sync,
                        desktop_dir_sync.as_deref(),
                        &desktop_monitors_sync,
                        &icon_positions_sync,
                        &internal_drag_sync,
                        &active_drag_sync,
                        &shortcut_handler_sync,
                        true,
                    )
                });
                wallpaper_surfaces_sync.borrow_mut().push(fallback_slot);
                return;
            }

            let mut matched_indices: Vec<Option<usize>> = Vec::with_capacity(monitor_list.len());
            let mut used_prev = vec![false; prev_slots.len()];
            for monitor in &monitor_list {
                let pos = prev_slots
                    .iter()
                    .enumerate()
                    .position(|(i, slot)| !used_prev[i] && slot.monitor.as_ref() == Some(monitor));
                if let Some(i) = pos {
                    used_prev[i] = true;
                }
                matched_indices.push(pos);
            }

            let primary_slot_changing = monitor_list
                .iter()
                .enumerate()
                .any(|(index, monitor)| {
                    let is_primary = index == primary_index;
                    let ((_, _), (dw, dh)) = monitor_target_sizes(Some(monitor));
                    match matched_indices[index].and_then(|pi| prev_slots.get(pi)) {
                        Some(slot) => slot_needs_role_rebuild(
                            slot.is_primary,
                            (slot.desktop_w, slot.desktop_h),
                            is_primary,
                            (dw, dh),
                        ),
                        None => is_primary,
                    }
                });

            let has_removed_slots = used_prev.iter().any(|&used| !used);
            if has_removed_slots || primary_slot_changing {
                close_all_registered_popovers();
            }

            // Aparcar ventanas de monitores desconectados sin llamar a `.close()` (evita Error::InvalidSize en Smithay).
            let mut remaining_slots: Vec<Option<WallpaperWindowSlot>> =
                prev_slots.drain(..).map(Some).collect();
            for (i, used) in used_prev.iter().enumerate() {
                if !used {
                    if let Some(removed_slot) = remaining_slots[i].take() {
                        if removed_slot.is_primary {
                            desktop_monitors_sync.borrow_mut().clear();
                            *shortcut_handler_sync.borrow_mut() = None;
                        }
                        parked_windows_sync
                            .borrow_mut()
                            .push(park_wallpaper_slot(removed_slot));
                    }
                }
            }

            if primary_slot_changing {
                desktop_monitors_sync.borrow_mut().clear();
                *shortcut_handler_sync.borrow_mut() = None;
                for (index, _) in monitor_list.iter().enumerate() {
                    if let Some(pi) = matched_indices[index] {
                        if let Some(slot) = remaining_slots[pi].as_mut() {
                            let is_primary = index == primary_index;
                            let ((_, _), (dw, dh)) = monitor_target_sizes(slot.monitor.as_ref());
                            if slot_needs_role_rebuild(
                                slot.is_primary,
                                (slot.desktop_w, slot.desktop_h),
                                is_primary,
                                (dw, dh),
                            ) {
                                if let Some(old_host) = slot.role_host.take() {
                                    slot.root_overlay.remove_overlay(&old_host);
                                }
                                if let Some(old_kc) = slot.key_controller.take() {
                                    slot.window.remove_controller(&old_kc);
                                }
                            }
                        }
                    }
                }
            }

            let mut next_slots = Vec::with_capacity(monitor_list.len());
            let had_primary_logged = primary_slot_changing
                || matched_indices
                    .get(primary_index)
                    .copied()
                    .flatten()
                    .is_none();
            if had_primary_logged {
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
            }

            for (index, monitor) in monitor_list.iter().enumerate() {
                let is_primary = index == primary_index;
                if let Some(pi) = matched_indices[index] {
                    if let Some(mut slot) = remaining_slots[pi].take() {
                        let ((phys_w, phys_h), (dw, dh)) = monitor_target_sizes(Some(monitor));
                        if slot.phys_w != phys_w || slot.phys_h != phys_h {
                            slot.phys_w = phys_w;
                            slot.phys_h = phys_h;
                            apply_wallpaper_to_picture(
                                &slot.picture,
                                &slot.window,
                                &image_path_sync,
                                phys_w,
                                phys_h,
                            );
                        }
                        if slot.role_host.is_none() {
                            let (new_host, new_kc) = populate_wallpaper_role(
                                &slot.window,
                                if is_primary {
                                    desktop_dir_sync.as_deref()
                                } else {
                                    None
                                },
                                (dw, dh),
                                &desktop_monitors_sync,
                                &icon_positions_sync,
                                &internal_drag_sync,
                                &active_drag_sync,
                                &shortcut_handler_sync,
                                is_primary,
                            );
                            slot.root_overlay.add_overlay(&new_host);
                            slot.role_host = Some(new_host);
                            slot.key_controller = new_kc;
                            slot.desktop_w = dw;
                            slot.desktop_h = dh;
                            slot.is_primary = is_primary;
                        }
                        next_slots.push(slot);
                        continue;
                    }
                }

                let slot = create_wallpaper_window(
                    &app_sync,
                    Some(monitor),
                    &image_path_sync,
                    if is_primary {
                        desktop_dir_sync.as_deref()
                    } else {
                        None
                    },
                    &desktop_monitors_sync,
                    &icon_positions_sync,
                    &internal_drag_sync,
                    &active_drag_sync,
                    &shortcut_handler_sync,
                    is_primary,
                );
                next_slots.push(slot);
            }

            *wallpaper_surfaces_sync.borrow_mut() = next_slots;
        });

        let pending_sync: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        let schedule_sync: Rc<dyn Fn()> = {
            let sync_windows_c = sync_windows.clone();
            let pending_c = pending_sync.clone();
            Rc::new(move || {
                if let Some(source) = pending_c.borrow_mut().take() {
                    source.remove();
                }
                let sync_cb = sync_windows_c.clone();
                let pending_done = pending_c.clone();
                let source = glib::timeout_add_local_once(
                    std::time::Duration::from_millis(120),
                    move || {
                        *pending_done.borrow_mut() = None;
                        sync_cb();
                    },
                );
                *pending_c.borrow_mut() = Some(source);
            })
        };

        *sync_handler.borrow_mut() = Some(schedule_sync.clone());
        sync_windows();

        if !monitors_hooked.get() {
            monitors_hooked.set(true);
            if let Some(display) = gtk4::gdk::Display::default() {
                let monitors = display.monitors();
                let schedule_mon = schedule_sync.clone();
                monitors.connect_items_changed(move |_, _, _, _| {
                    info!("🔌 [Wallpaper] Cambio en monitores GDK detectado; programando resincronización");
                    schedule_mon();
                });
            }
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
            clamp_icon_position(
                150.0,
                100.0,
                (20.0, 30.0),
                &metrics,
                (800, 600),
                DESKTOP_TOP_MARGIN
            ),
            (130.0, 70.0)
        );
        assert_eq!(
            clamp_icon_position(
                -20.0,
                -30.0,
                (20.0, 30.0),
                &metrics,
                (800, 600),
                DESKTOP_TOP_MARGIN
            ),
            (16.0, DESKTOP_TOP_MARGIN)
        );
        assert_eq!(
            clamp_icon_position(
                150.7,
                100.4,
                (20.0, 30.0),
                &metrics,
                (800, 600),
                DESKTOP_TOP_MARGIN
            ),
            (131.0, 70.0)
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
            dragged_icon_position(
                (130.0, 70.0),
                (20.0, 30.0),
                (15.0, -10.0),
                &metrics,
                (800, 600),
                DESKTOP_TOP_MARGIN
            ),
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

        let d1 = super::get_unique_text_document_name(&temp_dir, "Nuevo documento");
        assert_eq!(d1, temp_dir.join("Nuevo documento.txt"));
        std::fs::write(&d1, "").unwrap();

        let d2 = super::get_unique_text_document_name(&temp_dir, "Nuevo documento");
        assert_eq!(d2, temp_dir.join("Nuevo documento 2.txt"));

        assert_eq!(
            super::desktop_rename_selection_end("Nueva carpeta", true),
            -1
        );
        assert_eq!(
            super::desktop_rename_selection_end("Nuevo documento.txt", false),
            "Nuevo documento".chars().count() as i32
        );
        assert_eq!(super::desktop_rename_selection_end(".gitignore", false), -1);

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
        let icon_positions =
            std::rc::Rc::new(std::cell::RefCell::new(std::collections::HashMap::new()));

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
        let rows = ((1080 - (DESKTOP_TOP_MARGIN as i32) - 80).max(1) / metrics.cell_height()).max(1)
            as usize;

        let slot_0 =
            calculate_grid_position(0, rows, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);
        let slot_1 =
            calculate_grid_position(1, rows, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);
        let slot_2 =
            calculate_grid_position(2, rows, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);
        let slot_3 =
            calculate_grid_position(3, rows, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);

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
        let snapped_left =
            snap_position_to_grid(pos_left, &metrics, true, (1920, 1080), DESKTOP_TOP_MARGIN);
        assert_eq!(snapped_left.0, 16.0);
        assert_eq!(snapped_left.1, DESKTOP_TOP_MARGIN);
    }

    #[test]
    fn test_snap_position_to_grid_secondary_display() {
        let config = DesktopIconsConfig::default();
        let metrics = IconLayoutMetrics::from_config(&config);

        // En pantalla secundaria no hay Top Bar, el margen superior es 16.0
        let pos = (1800.0, 5.0);
        let snapped = snap_position_to_grid(
            pos,
            &metrics,
            false,
            (1920, 1080),
            DESKTOP_SECONDARY_TOP_MARGIN,
        );
        assert!(snapped.0 <= 1920.0 - metrics.box_width as f64);
        assert_eq!(snapped.1, DESKTOP_SECONDARY_TOP_MARGIN);

        let pos_left = (25.0, 5.0);
        let snapped_left = snap_position_to_grid(
            pos_left,
            &metrics,
            true,
            (1920, 1080),
            DESKTOP_SECONDARY_TOP_MARGIN,
        );
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
        // Fondo oscuro acrílico de popover unificado con Top Bar
        assert!(css.contains("rgba(30, 30, 36, 0.78)"));
        // Fondo claro acrílico de popover unificado con Top Bar
        assert!(css.contains("rgba(244, 246, 250, 0.76)"));
        // Clase reactiva light-popover y selectores
        assert!(css.contains("popover.desktop-popover-menu.light-popover > contents"));
        assert!(css.contains("popover.light-popover > contents"));
        assert!(css.contains(".desktop-context-btn"));
        assert!(css.contains(".desktop-context-separator"));
        // Colores de texto oscuro (#f4f4f5 / #e4e4e7) y claro (#1d1d1f)
        assert!(css.contains("#f4f4f5"));
        assert!(css.contains("#1d1d1f"));
        // Hover azul semitransparente y altura compacta del Top Bar
        assert!(css.contains("rgba(0, 122, 255, 0.70)"));
        assert!(css.contains("padding: 3px 8px;"));
        assert!(css.contains("min-height: 24px;"));
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
        assert!(css.contains(".desktop-icon.desktop-hidden-item"));
        assert!(css.contains(".desktop-icon.desktop-cut-item"));
        assert!(css.contains("opacity: 0.5;"));
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
        assert_eq!(
            fixed.child_transform(&item_box).map(|t| t.to_translate()),
            Some((100.0, 100.0))
        );

        let paintable = gtk4::WidgetPaintable::new(Some(&item_box));
        let src = gtk4::DragSource::new();
        src.set_icon(Some(&paintable), 35, 40);

        // Al comenzar el arrastre se desplaza fuera de pantalla (-5000)
        fixed.move_(&item_box, -5000.0, -5000.0);
        assert_eq!(
            fixed.child_transform(&item_box).map(|t| t.to_translate()),
            Some((-5000.0, -5000.0))
        );
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
        let rows = ((1080 - (DESKTOP_TOP_MARGIN as i32) - 80).max(1) / metrics.cell_height()).max(1)
            as usize;

        let slot_0 =
            calculate_grid_position(0, rows, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);
        let slot_1 =
            calculate_grid_position(1, rows, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);
        let slot_2 =
            calculate_grid_position(2, rows, &metrics, false, (1920, 1080), DESKTOP_TOP_MARGIN);

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
        let long_file =
            super::truncate_desktop_name("Documento_De_Investigacion_Final_2026.pdf", 24);
        assert!(long_file.contains('…'));
        assert!(long_file.ends_with(".pdf"));
        assert!(long_file.chars().count() <= 24);

        // 3. Extensión compuesta .tar.gz: preserva la extensión completa
        let tar_file = super::truncate_desktop_name("malus_de_backup_noviembre_2026.tar.gz", 24);
        assert!(tar_file.contains('…'));
        assert!(tar_file.ends_with(".tar.gz"));
        assert!(tar_file.chars().count() <= 24);

        // 4. Carpeta larga sin extensión: divide equitativamente por el centro
        let long_folder =
            super::truncate_desktop_name("Carpeta con fotos de las vacaciones 2026", 24);
        assert!(long_folder.contains('…'));
        assert!(long_folder.starts_with("Carpeta"));
        assert!(long_folder.ends_with("2026"));
        assert!(long_folder.chars().count() <= 24);

        // 5. Caracteres UTF-8 multibyte (español, acentos, eñe)
        let utf8_file =
            super::truncate_desktop_name("Diseño_gráfico_año_2026_españa_final.png", 24);
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

    #[test]
    fn test_compute_cover_crop_geometry_and_gradient_texture() {
        assert_eq!(
            super::compute_cover_crop_geometry(3840, 2160, 1920, 1080),
            (1920, 1080, 0, 0)
        );
        assert_eq!(
            super::compute_cover_crop_geometry(2000, 1000, 1920, 1080),
            (2160, 1080, 120, 0)
        );
        assert_eq!(
            super::compute_cover_crop_geometry(1000, 2000, 1920, 1080),
            (1920, 3840, 0, 1380)
        );
        let tex = super::build_default_gradient_texture(64, 36)
            .expect("Debe poder pre-rasterizar el fondo degradado");
        assert_eq!(tex.width(), 64);
        assert_eq!(tex.height(), 36);
    }
}

#[cfg(test)]
mod squircle_thumb_tests {
    use super::*;

    #[test]
    fn clip_makes_corners_transparent_and_keeps_center_opaque() {
        let (w, h) = (64usize, 64usize);
        let mut rgba = vec![255u8; w * h * 4];
        apply_thumb_squircle_clip_rgba(&mut rgba, w, h, thumb_squircle_radius(64, 64));
        assert_eq!(rgba[3], 0);
        assert_eq!(rgba[((h / 2) * w + w / 2) * 4 + 3], 255);
        assert_eq!(rgba[(w / 2) * 4 + 3], 255);
    }

    #[test]
    fn desktop_clipboard_cut_and_copy_roundtrip() {
        let dir = std::env::temp_dir().join("malus_wallpaper_clipboard_test");
        let _ = std::fs::remove_dir_all(&dir);
        let src_dir = dir.join("src");
        let dest_dir = dir.join("dest");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::create_dir_all(&dest_dir).unwrap();

        let file = src_dir.join("nota.txt");
        std::fs::write(&file, "hola escritorio").unwrap();

        let encoded = encode_gnome_copied_files(&DesktopClipboardAction::Cut, &[file.clone()]);
        let parsed = parse_gnome_copied_files(&encoded).unwrap();
        assert_eq!(parsed.action, DesktopClipboardAction::Cut);
        assert_eq!(parsed.paths, vec![file.clone()]);

        let copied = paste_desktop_payload_into(
            &DesktopClipboardPayload {
                action: DesktopClipboardAction::Copy,
                paths: vec![file.clone()],
            },
            &src_dir,
        );
        assert_eq!(copied, vec![src_dir.join("nota copia.txt")]);
        assert!(src_dir.join("nota copia.txt").is_file());

        let moved = paste_desktop_payload_into(&parsed, &dest_dir);
        assert_eq!(moved, vec![dest_dir.join("nota.txt")]);
        assert!(!file.exists());
        assert!(dest_dir.join("nota.txt").is_file());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn slot_role_rebuild_detects_primary_flip_and_desktop_resize() {
        assert!(!super::slot_needs_role_rebuild(
            true,
            (2560, 1440),
            true,
            (2560, 1440)
        ));
        assert!(!super::slot_needs_role_rebuild(
            false,
            (1920, 1080),
            false,
            (2560, 1440)
        ));
        assert!(super::slot_needs_role_rebuild(
            false,
            (1920, 1080),
            true,
            (1920, 1080)
        ));
        assert!(super::slot_needs_role_rebuild(
            true,
            (2560, 1440),
            true,
            (1920, 1080)
        ));
    }
}
