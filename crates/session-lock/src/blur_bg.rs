//! Carga del fondo de pantalla activo del escritorio en resolución completa
//! y versión difuminada para el blur de fondo del input de contraseña.

use std::path::{Path, PathBuf};

/// Fondo nítido de pantalla completa más su versión difuminada para el vidrio del input.
#[derive(Clone)]
pub struct LockWallpaperAssets {
    pub crisp_texture: gtk4::gdk::Texture,
    pub blurred_surface: gtk4::cairo::ImageSurface,
    pub blur_width: i32,
    pub blur_height: i32,
}

/// Resuelve el fondo de pantalla configurado actualmente en Malus DE,
/// respetando `XDG_CONFIG_HOME` / `~/.config/malus-de` y `.wallpaper-gradient`.
pub fn resolve_desktop_wallpaper_path() -> Option<PathBuf> {
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".config/malus-de"))
                .unwrap_or_else(|_| PathBuf::from("/etc/malus-de"))
        });

    resolve_wallpaper_in_dir(&config_dir).or_else(|| {
        if config_dir.join(".wallpaper-gradient").exists() {
            None
        } else {
            malus_de_common::config::default_bundled_wallpaper_path()
        }
    })
}

fn resolve_wallpaper_in_dir(config_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        config_dir.join("wallpaper.png"),
        config_dir.join("wallpaper.jpg"),
        config_dir.join("wallpaper.jpeg"),
        config_dir.join("wallpaper.webp"),
        config_dir.join("wallpaper.svg"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

/// Calcula el rectángulo de recorte *cover* (aspect-fill) centrado para cubrir `target_w x target_h`.
pub fn compute_cover_crop_geometry(
    src_w: i32,
    src_h: i32,
    target_w: i32,
    target_h: i32,
) -> (i32, i32, i32, i32) {
    let src_w = src_w.max(1) as f64;
    let src_h = src_h.max(1) as f64;
    let target_w = target_w.max(1) as f64;
    let target_h = target_h.max(1) as f64;

    let scale = (target_w / src_w).max(target_h / src_h);
    let scaled_w = (src_w * scale).round().max(target_w) as i32;
    let scaled_h = (src_h * scale).round().max(target_h) as i32;
    let crop_x = ((scaled_w - target_w as i32) / 2).max(0);
    let crop_y = ((scaled_h - target_h as i32) / 2).max(0);
    (scaled_w, scaled_h, crop_x, crop_y)
}

fn build_gradient_pixbuf(target_w: i32, target_h: i32) -> Option<gdk_pixbuf::Pixbuf> {
    let w = target_w.max(1);
    let h = target_h.max(1);
    let surface = gtk4::cairo::ImageSurface::create(gtk4::cairo::Format::ARgb32, w, h).ok()?;
    {
        let cr = gtk4::cairo::Context::new(&surface).ok()?;
        let cx = w as f64 * 0.35;
        let cy = h as f64 * 0.25;
        let radius = (w as f64 - cx).hypot(h as f64 - cy);
        let grad = gtk4::cairo::RadialGradient::new(cx, cy, 0.0, cx, cy, radius);
        grad.add_color_stop_rgba(
            0.0,
            0x2e as f64 / 255.0,
            0x10 as f64 / 255.0,
            0x65 as f64 / 255.0,
            1.0,
        );
        grad.add_color_stop_rgba(
            0.35,
            0x17 as f64 / 255.0,
            0x25 as f64 / 255.0,
            0x54 as f64 / 255.0,
            1.0,
        );
        grad.add_color_stop_rgba(
            0.70,
            0x0f as f64 / 255.0,
            0x17 as f64 / 255.0,
            0x2a as f64 / 255.0,
            1.0,
        );
        grad.add_color_stop_rgba(
            1.0,
            0x0d as f64 / 255.0,
            0x15 as f64 / 255.0,
            0x26 as f64 / 255.0,
            1.0,
        );
        let _ = cr.set_source(&grad);
        let _ = cr.paint();
    }
    let data = surface.take_data().ok()?;
    let mut rgba = vec![255u8; data.len()];
    for (dst, src) in rgba.chunks_exact_mut(4).zip(data.chunks_exact(4)) {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
    }
    Some(gdk_pixbuf::Pixbuf::from_bytes(
        &glib::Bytes::from_owned(rgba),
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        w,
        h,
        w * 4,
    ))
}

fn build_solid_color_pixbuf(
    target_w: i32,
    target_h: i32,
    r: u8,
    g: u8,
    b: u8,
) -> Option<gdk_pixbuf::Pixbuf> {
    let w = target_w.max(1);
    let h = target_h.max(1);
    let mut rgba = vec![255u8; (w as usize) * (h as usize) * 4];
    for px in rgba.chunks_exact_mut(4) {
        px[0] = r;
        px[1] = g;
        px[2] = b;
        px[3] = 255;
    }
    Some(gdk_pixbuf::Pixbuf::from_bytes(
        &glib::Bytes::from_owned(rgba),
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        w,
        h,
        w * 4,
    ))
}

fn load_cover_cropped_pixbuf(target_w: i32, target_h: i32) -> Option<gdk_pixbuf::Pixbuf> {
    let target_w = target_w.max(1);
    let target_h = target_h.max(1);
    let wp_cfg = malus_de_common::config::WallpaperConfig::load();
    match wp_cfg.mode {
        malus_de_common::config::WallpaperMode::SolidColor => {
            let (r, g, b) = malus_de_common::config::parse_wallpaper_hex_color(&wp_cfg.color_hex)
                .unwrap_or((0x4D, 0xA6, 0xC2));
            return build_solid_color_pixbuf(target_w, target_h, r, g, b);
        }
        malus_de_common::config::WallpaperMode::Gradient => {
            return build_gradient_pixbuf(target_w, target_h);
        }
        malus_de_common::config::WallpaperMode::DynamicPair => {
            let is_dark = malus_de_common::config::is_dark_mode_active();
            let chosen = if is_dark {
                wp_cfg.dark_image_path.as_deref()
            } else {
                wp_cfg.light_image_path.as_deref()
            };
            if let Some(p) = chosen.map(PathBuf::from).filter(|p| p.is_file()) {
                if let Ok(loaded) = gdk_pixbuf::Pixbuf::from_file(p) {
                    let pixbuf = loaded.apply_embedded_orientation().unwrap_or(loaded);
                    let (sw, sh, cx, cy) = compute_cover_crop_geometry(
                        pixbuf.width(),
                        pixbuf.height(),
                        target_w,
                        target_h,
                    );
                    if let Some(scaled) = pixbuf.scale_simple(sw, sh, gdk_pixbuf::InterpType::Bilinear) {
                        if scaled.width() == target_w && scaled.height() == target_h {
                            return Some(scaled);
                        }
                        if let Some(cropped) = scaled.new_subpixbuf(cx, cy, target_w, target_h).copy() {
                            return Some(cropped);
                        }
                    }
                }
            }
        }
        malus_de_common::config::WallpaperMode::Image => {}
    }

    let explicit_image = wp_cfg
        .image_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(resolve_desktop_wallpaper_path);

    explicit_image
        .and_then(|path| {
            let loaded = gdk_pixbuf::Pixbuf::from_file(path).ok()?;
            let pixbuf = loaded.apply_embedded_orientation().unwrap_or(loaded);
            let src_w = pixbuf.width();
            let src_h = pixbuf.height();
            if src_w == target_w && src_h == target_h {
                Some(pixbuf)
            } else {
                let (sw, sh, cx, cy) =
                    compute_cover_crop_geometry(src_w, src_h, target_w, target_h);
                let scaled = if src_w == sw && src_h == sh {
                    pixbuf
                } else {
                    pixbuf.scale_simple(sw, sh, gdk_pixbuf::InterpType::Bilinear)?
                };
                if scaled.width() == target_w && scaled.height() == target_h {
                    Some(scaled)
                } else {
                    scaled.new_subpixbuf(cx, cy, target_w, target_h).copy()
                }
            }
        })
        .or_else(|| build_gradient_pixbuf(target_w, target_h))
}

/// Aplica un desenfoque separable de 3 pasadas sobre un buffer RGBA (stride = `width * 4`).
pub fn box_blur_rgba_3_pass(rgba: &mut [u8], width: usize, height: usize, radius: usize) {
    if width == 0 || height == 0 || radius == 0 || rgba.len() < width * height * 4 {
        return;
    }
    let mut scratch = vec![0u8; width * height * 4];
    for _ in 0..3 {
        box_blur_horizontal(rgba, &mut scratch, width, height, radius);
        box_blur_vertical(&scratch, rgba, width, height, radius);
    }
}

fn box_blur_horizontal(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
    let window = (radius * 2 + 1) as u32;
    for y in 0..height {
        let row = y * width * 4;
        let mut acc = [0u32; 3];
        for k in -(radius as isize)..=(radius as isize) {
            let sx = k.clamp(0, width as isize - 1) as usize;
            let idx = row + sx * 4;
            acc[0] += src[idx] as u32;
            acc[1] += src[idx + 1] as u32;
            acc[2] += src[idx + 2] as u32;
        }
        for x in 0..width {
            let out = row + x * 4;
            dst[out] = (acc[0] / window) as u8;
            dst[out + 1] = (acc[1] / window) as u8;
            dst[out + 2] = (acc[2] / window) as u8;
            dst[out + 3] = 255;

            let left = (x as isize - radius as isize).clamp(0, width as isize - 1) as usize;
            let right = (x as isize + radius as isize + 1).clamp(0, width as isize - 1) as usize;
            let li = row + left * 4;
            let ri = row + right * 4;
            acc[0] = acc[0] + src[ri] as u32 - src[li] as u32;
            acc[1] = acc[1] + src[ri + 1] as u32 - src[li + 1] as u32;
            acc[2] = acc[2] + src[ri + 2] as u32 - src[li + 2] as u32;
        }
    }
}

fn box_blur_vertical(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
    let window = (radius * 2 + 1) as u32;
    let stride = width * 4;
    for x in 0..width {
        let col = x * 4;
        let mut acc = [0u32; 3];
        for k in -(radius as isize)..=(radius as isize) {
            let sy = k.clamp(0, height as isize - 1) as usize;
            let idx = sy * stride + col;
            acc[0] += src[idx] as u32;
            acc[1] += src[idx + 1] as u32;
            acc[2] += src[idx + 2] as u32;
        }
        for y in 0..height {
            let out = y * stride + col;
            dst[out] = (acc[0] / window) as u8;
            dst[out + 1] = (acc[1] / window) as u8;
            dst[out + 2] = (acc[2] / window) as u8;
            dst[out + 3] = 255;

            let top = (y as isize - radius as isize).clamp(0, height as isize - 1) as usize;
            let bot = (y as isize + radius as isize + 1).clamp(0, height as isize - 1) as usize;
            let ti = top * stride + col;
            let bi = bot * stride + col;
            acc[0] = acc[0] + src[bi] as u32 - src[ti] as u32;
            acc[1] = acc[1] + src[bi + 1] as u32 - src[ti + 1] as u32;
            acc[2] = acc[2] + src[bi + 2] as u32 - src[ti + 2] as u32;
        }
    }
}

/// Aumenta saturación y luminosidad tras el blur para reproducir el efecto *Vibrancy* de macOS.
pub fn apply_vibrancy_rgba(rgba: &mut [u8], saturation: f32, brightness: f32) {
    for px in rgba.chunks_exact_mut(4) {
        let r = px[0] as f32 / 255.0;
        let g = px[1] as f32 / 255.0;
        let b = px[2] as f32 / 255.0;
        let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let nr = ((lum + (r - lum) * saturation) * brightness).clamp(0.0, 1.0);
        let ng = ((lum + (g - lum) * saturation) * brightness).clamp(0.0, 1.0);
        let nb = ((lum + (b - lum) * saturation) * brightness).clamp(0.0, 1.0);
        px[0] = (nr * 255.0).round() as u8;
        px[1] = (ng * 255.0).round() as u8;
        px[2] = (nb * 255.0).round() as u8;
        px[3] = 255;
    }
}

fn build_blurred_surface_from_pixbuf(
    crisp: &gdk_pixbuf::Pixbuf,
    target_w: i32,
    target_h: i32,
) -> Option<(gtk4::cairo::ImageSurface, i32, i32)> {
    let aspect = (target_w.max(1) as f64) / (target_h.max(1) as f64);
    let (bw, bh) = if aspect >= 1.0 {
        (720, (720.0 / aspect).round().max(180.0) as i32)
    } else {
        ((720.0 * aspect).round().max(180.0) as i32, 720)
    };
    let small = crisp.scale_simple(bw, bh, gdk_pixbuf::InterpType::Bilinear)?;
    let w = small.width() as usize;
    let h = small.height() as usize;
    let stride = small.rowstride() as usize;
    let ch = small.n_channels() as usize;
    let bytes = small.read_pixel_bytes();
    let src = bytes.as_ref();
    let mut rgba = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let s = y * stride + x * ch;
            let d = (y * w + x) * 4;
            if s + 3 <= src.len() {
                rgba[d] = src[s];
                rgba[d + 1] = src[s + 1];
                rgba[d + 2] = src[s + 2];
            }
        }
    }
    // Blur medio-fino estilo macOS Vibrancy + impulso de saturación para conservar color y textura
    box_blur_rgba_3_pass(&mut rgba, w, h, 9);
    apply_vibrancy_rgba(&mut rgba, 1.60, 1.04);

    let mut surface =
        gtk4::cairo::ImageSurface::create(gtk4::cairo::Format::ARgb32, bw, bh).ok()?;
    {
        let mut data = surface.data().ok()?;
        for (dst, src) in data.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
            // Cairo ARgb32 little-endian: B, G, R, A
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = 255;
        }
    }
    Some((surface, bw, bh))
}

/// Carga el wallpaper nítido y su versión difuminada para el efecto vidrio del input.
pub fn build_lock_wallpaper_assets(target_w: i32, target_h: i32) -> Option<LockWallpaperAssets> {
    let crisp = load_cover_cropped_pixbuf(target_w, target_h)?;
    let crisp_texture = gtk4::gdk::Texture::for_pixbuf(&crisp);
    let (blurred_surface, blur_width, blur_height) =
        build_blurred_surface_from_pixbuf(&crisp, target_w, target_h)?;
    Some(LockWallpaperAssets {
        crisp_texture,
        blurred_surface,
        blur_width,
        blur_height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_crop_geometry_preserves_aspect_fill_and_centers_crop() {
        let (sw, sh, cx, cy) = compute_cover_crop_geometry(1920, 1080, 1000, 1000);
        assert_eq!(sh, 1000);
        assert!(sw > 1000);
        assert_eq!(cx, (sw - 1000) / 2);
        assert_eq!(cy, 0);
    }

    #[test]
    fn box_blur_smooths_sharp_impulse_across_neighbors() {
        let (w, h) = (32usize, 32usize);
        let mut rgba = vec![0u8; w * h * 4];
        for px in rgba.chunks_exact_mut(4) {
            px[3] = 255;
        }
        for y in 12..20 {
            for x in 12..20 {
                let i = (y * w + x) * 4;
                rgba[i] = 255;
                rgba[i + 1] = 255;
                rgba[i + 2] = 255;
            }
        }
        box_blur_rgba_3_pass(&mut rgba, w, h, 8);
        assert!(rgba[(2 * w + 2) * 4] > 0);
        assert!(rgba[(16 * w + 16) * 4] < 220);
    }

    #[test]
    fn resolve_wallpaper_in_dir_prefers_user_image() {
        let dir = std::env::temp_dir().join(format!("malus-lock-wp-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wp = dir.join("wallpaper.webp");
        std::fs::write(&wp, b"fake").unwrap();
        assert_eq!(resolve_wallpaper_in_dir(&dir), Some(wp));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vibrancy_boosts_color_saturation_cleanly() {
        let mut rgba = [180u8, 120, 110, 255];
        apply_vibrancy_rgba(&mut rgba, 1.6, 1.04);
        assert!(rgba[0] - rgba[2] > 70);
    }
}
