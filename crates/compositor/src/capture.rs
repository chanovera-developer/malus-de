use anyhow::{bail, Context, Result};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
                Id, Kind,
            },
            gles::{GlesRenderer, GlesTexture},
            Bind, Color32F, ExportMem, Frame, Offscreen, Renderer,
        },
    },
    desktop::Window,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{Buffer as BufferCoord, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
    wayland::seat::WaylandFocus,
};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    path::Path,
    time::Duration,
};

pub fn capture_frame<R>(
    renderer: &mut R,
    target: &R::Framebuffer<'_>,
    size: Size<i32, Physical>,
    destination: &Path,
    transform: Transform,
) -> Result<()>
where
    R: ExportMem,
{
    validate_destination(destination)?;
    let buffer_size = Size::<i32, BufferCoord>::from((size.w, size.h));
    let region: Rectangle<i32, BufferCoord> = Rectangle::from_size(buffer_size);
    let mapping = renderer
        .copy_framebuffer(target, region, Fourcc::Abgr8888)
        .map_err(|error| {
            anyhow::anyhow!("No se pudo leer el framebuffer del compositor: {error:?}")
        })?;
    let pixels = renderer.map_texture(&mapping).map_err(|error| {
        anyhow::anyhow!("No se pudo mapear el framebuffer capturado: {error:?}")
    })?;
    let expected_len = (size.w as usize)
        .checked_mul(size.h as usize)
        .and_then(|pixel_count| pixel_count.checked_mul(4))
        .context("La resolución de pantalla excede el tamaño permitido")?;
    if pixels.len() != expected_len {
        bail!(
            "El framebuffer tiene un tamaño inesperado: {} bytes, se esperaban {expected_len}",
            pixels.len()
        );
    }

    let image = orient_pixels(&pixels, size.w as usize * 4, transform);
    let temporary = destination.with_extension("pending");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .with_context(|| format!("No se pudo crear {}", temporary.display()))?;
    let mut encoder = png::Encoder::new(file, size.w as u32, size.h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = match encoder.write_header() {
        Ok(writer) => writer,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error).context("No se pudo iniciar la escritura de la captura PNG");
        }
    };
    if let Err(error) = writer.write_image_data(&image) {
        drop(writer);
        let _ = fs::remove_file(&temporary);
        return Err(error).context("No se pudo escribir la captura PNG");
    }
    drop(writer);
    if let Err(error) = fs::hard_link(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error)
            .with_context(|| format!("No se pudo finalizar {}", destination.display()));
    }
    if let Err(error) = fs::remove_file(&temporary) {
        tracing::warn!(
            "No se pudo limpiar el archivo temporal {}: {error}",
            temporary.display()
        );
    }
    Ok(())
}

#[derive(Clone)]
pub struct CapturedWindowOffscreen {
    pub texture: GlesTexture,
    /// Rectángulo lógico completo de la textura capturada respecto a `state.space`
    /// (incluye `SSD_TITLEBAR_HEIGHT` en la parte superior cuando usa barra SSD).
    pub win_rect_logical: Rectangle<i32, Logical>,
}

/// Captura los snapshots GPU offscreen para las animaciones Genie activas (minimizar y restaurar)
/// y, en minimización, exporta desde el mismo renderizado la miniatura PNG del Dock.
#[allow(clippy::too_many_arguments)]
pub fn capture_pending_genie_snapshots(
    state: &mut crate::state::MalusState,
    renderer: &mut GlesRenderer,
    squircle_shader: Option<&crate::window_corners::WindowSquircleShader>,
    mut blur_pipeline: Option<&mut crate::blur::GaussianBlur>,
    snapshots: &mut HashMap<u64, CapturedWindowOffscreen>,
    output_geo: Rectangle<i32, Logical>,
    screen_size: Size<i32, Physical>,
    filter_by_output: bool,
    scale: f64,
    win_phys_r: Option<i32>,
    is_dark_theme: bool,
    now: std::time::Instant,
) {
    let active_ids: HashSet<u64> = state
        .active_animations
        .iter()
        .map(|anim| anim.window_id)
        .collect();
    snapshots.retain(|id, _| active_ids.contains(id));

    let mut pending = Vec::new();
    for anim in &state.active_animations {
        let needs_snapshot = !snapshots.contains_key(&anim.window_id);
        let needs_png = anim.is_minimizing
            && !state
                .minimized_window_thumbnails
                .contains_key(&anim.window_id);
        if (!needs_snapshot && !needs_png)
            || now.duration_since(anim.start_time) > Duration::from_millis(250)
        {
            continue;
        }
        if filter_by_output && !output_geo.overlaps(anim.window_rect) {
            continue;
        }
        pending.push((
            anim.window_id,
            anim.window.clone(),
            anim.window_rect.loc,
            needs_snapshot,
            needs_png,
        ));
    }

    let mut captured_any_png = false;
    for (window_id, window, window_loc, needs_snapshot, needs_png) in pending {
        let png_path_opt = if needs_png {
            match window_thumbnail_path(window_id) {
                Ok(p) => Some(p),
                Err(error) => {
                    tracing::warn!(
                        "No se pudo resolver la ruta de miniatura para {}: {error:#}",
                        window_id
                    );
                    None
                }
            }
        } else {
            None
        };

        match capture_window_offscreen(
            renderer,
            &mut state.ssd,
            squircle_shader,
            blur_pipeline.as_deref_mut(),
            &window,
            window_loc,
            output_geo.loc,
            screen_size,
            scale,
            win_phys_r,
            is_dark_theme,
            png_path_opt.as_deref(),
        ) {
            Ok(captured) => {
                if needs_snapshot {
                    snapshots.insert(window_id, captured);
                }
                if let Some(path) = png_path_opt {
                    state
                        .minimized_window_thumbnails
                        .insert(window_id, path.display().to_string());
                    captured_any_png = true;
                }
            }
            Err(error) => {
                tracing::warn!(
                    "No se pudo capturar snapshot offscreen de la ventana {}: {error:#}",
                    window_id
                );
            }
        }
    }

    if captured_any_png {
        state.notify_minimized_windows();
    }
}

/// Captura una ventana individual de forma aislada en un FBO offscreen RGBA con fondo
/// 100% transparente (`A = 0`) y esquinas squircle analíticas, escribiendo el PNG atómicamente.
#[allow(clippy::too_many_arguments)]
pub fn capture_isolated_window_to_png(
    renderer: &mut GlesRenderer,
    ssd: &mut crate::ssd::SsdManager,
    squircle_shader: Option<&crate::window_corners::WindowSquircleShader>,
    window: &Window,
    window_loc_logical: Point<i32, Logical>,
    output_origin_logical: Point<i32, Logical>,
    screen_size: Size<i32, Physical>,
    scale: f64,
    win_phys_r: Option<i32>,
    is_dark_theme: bool,
    destination: &Path,
) -> Result<()> {
    validate_destination(destination)?;
    let temporary = destination.with_extension("pending");
    let capture_res = capture_window_offscreen(
        renderer,
        ssd,
        squircle_shader,
        None,
        window,
        window_loc_logical,
        output_origin_logical,
        screen_size,
        scale,
        win_phys_r,
        is_dark_theme,
        Some(&temporary),
    );
    if let Err(err) = capture_res {
        let _ = fs::remove_file(&temporary);
        return Err(err);
    }
    if let Err(error) = fs::hard_link(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error)
            .with_context(|| format!("No se pudo finalizar {}", destination.display()));
    }
    if let Err(error) = fs::remove_file(&temporary) {
        tracing::warn!(
            "No se pudo limpiar el archivo temporal {}: {error}",
            temporary.display()
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn capture_window_offscreen(
    renderer: &mut GlesRenderer,
    ssd: &mut crate::ssd::SsdManager,
    squircle_shader: Option<&crate::window_corners::WindowSquircleShader>,
    blur_pipeline: Option<&mut crate::blur::GaussianBlur>,
    window: &Window,
    window_loc_logical: Point<i32, Logical>,
    output_origin_logical: Point<i32, Logical>,
    screen_size: Size<i32, Physical>,
    scale: f64,
    win_phys_r: Option<i32>,
    is_dark_theme: bool,
    png_destination: Option<&Path>,
) -> Result<CapturedWindowOffscreen> {
    if !scale.is_finite() || scale <= 0.0 {
        bail!("La escala de salida no es válida para capturar el snapshot");
    }

    let geo = window.geometry();
    if geo.size.w <= 0 || geo.size.h <= 0 {
        bail!("La geometría de la ventana no es válida para capturar el snapshot");
    }

    let main_surface = window
        .wl_surface()
        .context("La ventana no tiene superficie Wayland activa")?;
    let root_id = Id::from_wayland_resource(main_surface.as_ref());

    let uses_ssd = crate::state::window_uses_ssd(window);
    let (is_fullscreen, is_active) = window
        .toplevel()
        .map(|t| {
            let states = &t.current_state().states;
            (
                states.contains(xdg_toplevel::State::Fullscreen),
                states.contains(xdg_toplevel::State::Activated),
            )
        })
        .unwrap_or((false, false));
    let is_firefox = crate::state::is_firefox_window(window);
    let is_translucent = crate::state::is_translucent_window(window);

    let phys_loc = Point::from((-geo.loc.x, -geo.loc.y)).to_physical_precise_round(scale);
    let base_win_rect = Rectangle::new(
        Point::from((0, 0)),
        geo.size.to_physical_precise_round(scale),
    );

    let probe_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
        render_elements_from_surface_tree(
            renderer,
            main_surface.as_ref(),
            phys_loc,
            Scale::from(scale),
            1.0,
            Kind::Unspecified,
        );
    if probe_elems.is_empty() {
        bail!("La ventana aún no tiene superficies listas para capturar");
    }

    let content_rect = if !uses_ssd {
        crate::window_corners::effective_shadow_rect_for_window(
            &probe_elems,
            &root_id,
            base_win_rect,
            scale,
            is_translucent,
            uses_ssd,
        )
    } else {
        base_win_rect
    };

    let (final_rect, tb_loc_opt, tb_phys_h) = if uses_ssd && !is_fullscreen {
        let tb_h = (crate::ssd::SSD_TITLEBAR_HEIGHT as f64 * scale).round() as i32;
        let tb_loc = Point::from((content_rect.loc.x, content_rect.loc.y - tb_h));
        let full_rect = Rectangle::new(
            tb_loc,
            Size::from((content_rect.size.w, content_rect.size.h + tb_h)),
        );
        (full_rect, Some(tb_loc), tb_h)
    } else {
        (content_rect, None, 0)
    };

    if final_rect.size.w <= 0 || final_rect.size.h <= 0 {
        bail!("Las dimensiones calculadas del snapshot son inválidas");
    }

    let shift = Point::from((-final_rect.loc.x, -final_rect.loc.y));
    let offscreen_win_elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
        render_elements_from_surface_tree(
            renderer,
            main_surface.as_ref(),
            phys_loc + shift,
            Scale::from(scale),
            1.0,
            Kind::Unspecified,
        );
    if offscreen_win_elems.is_empty() {
        bail!("La ventana no produjo elementos renderizables");
    }

    let ssd_elem = tb_loc_opt.and_then(|tb_loc| {
        let ssd_title = crate::state::window_title_for_ssd(window);
        ssd.render_element_for_window(
            renderer,
            main_surface.as_ref(),
            tb_loc + shift,
            final_rect.size.w,
            tb_phys_h,
            scale,
            &ssd_title,
            is_active,
            is_dark_theme,
        )
    });

    let buf_size = Size::<i32, BufferCoord>::from((final_rect.size.w, final_rect.size.h));
    let mut texture: GlesTexture = renderer
        .create_buffer(Fourcc::Abgr8888, buf_size)
        .map_err(|error| anyhow::anyhow!("No se pudo crear textura offscreen: {error:?}"))?;

    let offscreen_rect = Rectangle::from_size(final_rect.size);
    let should_round = win_phys_r.is_some() && !is_fullscreen;

    let pixels_opt = {
        let mut target = renderer
            .bind(&mut texture)
            .map_err(|error| anyhow::anyhow!("No se pudo enlazar FBO offscreen: {error:?}"))?;
        {
            let mut frame = renderer
                .render(&mut target, final_rect.size, Transform::Normal)
                .map_err(|error| {
                    anyhow::anyhow!("No se pudo iniciar frame offscreen: {error:?}")
                })?;
            let _ = frame.clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[offscreen_rect]);
            if is_translucent {
                if let Some(blur) = blur_pipeline {
                    let win_screen_origin = (window_loc_logical - output_origin_logical)
                        .to_physical_precise_round(scale);
                    let screen_win_rect =
                        Rectangle::new(win_screen_origin + final_rect.loc, final_rect.size);
                    blur.render_translucent_offscreen_blur(
                        &mut frame,
                        screen_size,
                        screen_win_rect,
                        offscreen_rect,
                        scale,
                        win_phys_r,
                        should_round,
                    );
                }
            }
            crate::window_corners::render_window_elements(
                &mut frame,
                scale,
                &[offscreen_rect],
                squircle_shader,
                &root_id,
                &offscreen_win_elems,
                ssd_elem.as_ref(),
                offscreen_rect,
                win_phys_r,
                should_round,
                is_firefox,
                None,
            );
            let _ = frame.finish().map_err(|error| {
                anyhow::anyhow!("No se pudo finalizar frame offscreen: {error:?}")
            })?;
        }

        if png_destination.is_some() {
            let region = Rectangle::from_size(buf_size);
            let mapping = renderer
                .copy_framebuffer(&target, region, Fourcc::Abgr8888)
                .map_err(|error| {
                    anyhow::anyhow!("No se pudo leer el framebuffer de la ventana: {error:?}")
                })?;
            let mapped = renderer.map_texture(&mapping).map_err(|error| {
                anyhow::anyhow!("No se pudo mapear la miniatura capturada: {error:?}")
            })?;
            let expected_len = (final_rect.size.w as usize)
                .checked_mul(final_rect.size.h as usize)
                .and_then(|pixel_count| pixel_count.checked_mul(4))
                .context("El tamaño de la miniatura excede el tamaño permitido")?;
            if mapped.len() != expected_len {
                bail!(
                    "La miniatura tiene un tamaño inesperado: {} bytes, se esperaban {expected_len}",
                    mapped.len()
                );
            }
            Some(unpremultiply_rgba(&mapped))
        } else {
            None
        }
    };

    if let (Some(destination), Some(pixels)) = (png_destination, pixels_opt) {
        let temporary = destination.with_extension("pending");
        let file = File::create(&temporary)
            .with_context(|| format!("No se pudo crear {}", temporary.display()))?;
        let mut encoder =
            png::Encoder::new(file, final_rect.size.w as u32, final_rect.size.h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = match encoder.write_header() {
            Ok(writer) => writer,
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(error).context("No se pudo iniciar la escritura de la miniatura PNG");
            }
        };
        if let Err(error) = writer.write_image_data(&pixels) {
            drop(writer);
            let _ = fs::remove_file(&temporary);
            return Err(error).context("No se pudo escribir la miniatura PNG");
        }
        drop(writer);
        if let Err(error) = fs::rename(&temporary, destination) {
            let _ = fs::remove_file(&temporary);
            return Err(error).with_context(|| {
                format!(
                    "No se pudo finalizar la miniatura {}",
                    destination.display()
                )
            });
        }
    }

    let logical_dx = (final_rect.loc.x as f64 / scale).round() as i32;
    let logical_dy = (final_rect.loc.y as f64 / scale).round() as i32;
    let logical_w = (final_rect.size.w as f64 / scale).round().max(1.0) as i32;
    let logical_h = (final_rect.size.h as f64 / scale).round().max(1.0) as i32;
    let win_rect_logical = Rectangle::new(
        (
            window_loc_logical.x + logical_dx,
            window_loc_logical.y + logical_dy,
        )
            .into(),
        (logical_w, logical_h).into(),
    );

    Ok(CapturedWindowOffscreen {
        texture,
        win_rect_logical,
    })
}

fn unpremultiply_rgba(pixels: &[u8]) -> Vec<u8> {
    let mut out = pixels.to_vec();
    for px in out.chunks_exact_mut(4) {
        let a = px[3];
        if a > 0 && a < 255 {
            let a_u16 = u16::from(a);
            px[0] = ((u16::from(px[0]) * 255 + a_u16 / 2) / a_u16).min(255) as u8;
            px[1] = ((u16::from(px[1]) * 255 + a_u16 / 2) / a_u16).min(255) as u8;
            px[2] = ((u16::from(px[2]) * 255 + a_u16 / 2) / a_u16).min(255) as u8;
        }
    }
    out
}

pub fn window_thumbnail_path(window_id: u64) -> Result<std::path::PathBuf> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .context("XDG_RUNTIME_DIR no está definido")?;
    Ok(runtime_dir.join(format!(
        "malus-window-thumbnail-{}-{window_id}.png",
        std::process::id()
    )))
}

fn orient_pixels(pixels: &[u8], row_len: usize, transform: Transform) -> Vec<u8> {
    match transform {
        Transform::Flipped180 => pixels
            .chunks_exact(row_len)
            .rev()
            .flat_map(|row| row.iter().rev().copied())
            .collect(),
        Transform::Flipped => pixels
            .chunks_exact(row_len)
            .rev()
            .flatten()
            .copied()
            .collect(),
        _ => pixels.to_vec(),
    }
}

pub fn write_capture_error(destination: &Path, error: &anyhow::Error) {
    if validate_destination(destination).is_err() {
        return;
    }
    let error_path = destination.with_extension("error");
    if let Err(write_error) = fs::write(&error_path, error.to_string()) {
        tracing::warn!(
            "No se pudo informar el fallo de captura en {}: {write_error}",
            error_path.display()
        );
    }
}

fn validate_destination(destination: &Path) -> Result<()> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .context("XDG_RUNTIME_DIR no está definido")?;
    let filename = destination
        .file_name()
        .and_then(|name| name.to_str())
        .context("La ruta de captura no tiene un nombre de archivo válido")?;
    if !destination.is_absolute()
        || destination.parent() != Some(runtime_dir.as_path())
        || !filename.starts_with("malus-screenshot-")
        || destination.extension().and_then(|ext| ext.to_str()) != Some("png")
    {
        bail!("La ruta de captura debe ser un PNG dentro de XDG_RUNTIME_DIR");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{orient_pixels, unpremultiply_rgba};
    use smithay::utils::Transform;

    #[test]
    fn flips_image_vertically_without_rotating_it() {
        let pixels = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

        assert_eq!(
            orient_pixels(&pixels, 8, Transform::Flipped),
            [9, 10, 11, 12, 13, 14, 15, 16, 1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert_eq!(orient_pixels(&pixels, 8, Transform::Normal), pixels);
    }

    #[test]
    fn rotates_image_180_degrees() {
        let pixels = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

        assert_eq!(
            orient_pixels(&pixels, 8, Transform::Flipped180),
            [16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1]
        );
    }

    #[test]
    fn unpremultiplies_alpha_without_overflow_or_halo() {
        let input = [
            0, 0, 0, 0, // transparente
            10, 20, 30, 255, // opaco
            128, 64, 0, 128, // semitransparente
            255, 255, 255, 1, // clamp a 255 sin desbordamiento
        ];
        let out = unpremultiply_rgba(&input);
        assert_eq!(&out[0..4], &[0, 0, 0, 0]);
        assert_eq!(&out[4..8], &[10, 20, 30, 255]);
        assert_eq!(&out[8..12], &[255, 128, 0, 128]);
        assert_eq!(&out[12..16], &[255, 255, 255, 1]);
    }
}
