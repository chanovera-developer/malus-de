use anyhow::{bail, Context, Result};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Color32F, ExportMem, Frame, Offscreen, Renderer,
            element::{
                Id, Kind,
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            },
            gles::{GlesRenderer, GlesTexture},
        },
    },
    desktop::Window,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{Buffer as BufferCoord, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
    wayland::seat::WaylandFocus,
};
use std::{
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

/// Captura las miniaturas limpias de las ventanas en curso de minimización
/// renderizando directamente su árbol de superficies en un FBO offscreen,
/// evitando la deformación del efecto Genie o elementos superpuestos del escritorio.
#[allow(clippy::too_many_arguments)]
pub fn capture_pending_minimized_thumbnails(
    state: &mut crate::state::MalusState,
    renderer: &mut GlesRenderer,
    squircle_shader: Option<&crate::window_corners::WindowSquircleShader>,
    output_geo_filter: Option<Rectangle<i32, Logical>>,
    scale: f64,
    win_phys_r: Option<i32>,
    is_dark_theme: bool,
    now: std::time::Instant,
) {
    let mut pending = Vec::new();
    for anim in &state.active_animations {
        if !anim.is_minimizing
            || state.minimized_window_thumbnails.contains_key(&anim.window_id)
            || now.duration_since(anim.start_time) > Duration::from_millis(250)
        {
            continue;
        }
        if let Some(output_geo) = output_geo_filter {
            if !output_geo.overlaps(anim.window_rect) {
                continue;
            }
        }
        pending.push((anim.window_id, anim.window.clone()));
    }

    let mut captured_any = false;
    for (window_id, window) in pending {
        let result = window_thumbnail_path(window_id).and_then(|path| {
            capture_window_thumbnail_offscreen(
                renderer,
                &mut state.ssd,
                squircle_shader,
                &window,
                scale,
                win_phys_r,
                is_dark_theme,
                &path,
            )
            .map(|()| path)
        });
        match result {
            Ok(path) => {
                state
                    .minimized_window_thumbnails
                    .insert(window_id, path.display().to_string());
                captured_any = true;
            }
            Err(error) => {
                tracing::warn!(
                    "No se pudo capturar la miniatura offscreen de la ventana {}: {error:#}",
                    window_id
                );
            }
        }
    }

    if captured_any {
        state.notify_minimized_windows();
    }
}

fn capture_window_thumbnail_offscreen(
    renderer: &mut GlesRenderer,
    ssd: &mut crate::ssd::SsdManager,
    squircle_shader: Option<&crate::window_corners::WindowSquircleShader>,
    window: &Window,
    scale: f64,
    win_phys_r: Option<i32>,
    is_dark_theme: bool,
    destination: &Path,
) -> Result<()> {
    if !scale.is_finite() || scale <= 0.0 {
        bail!("La escala de salida no es válida para capturar la miniatura");
    }

    let geo = window.geometry();
    if geo.size.w <= 0 || geo.size.h <= 0 {
        bail!("La geometría de la ventana no es válida para capturar su miniatura");
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
    let base_win_rect =
        Rectangle::new(Point::from((0, 0)), geo.size.to_physical_precise_round(scale));

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
        bail!("Las dimensiones calculadas de la miniatura son inválidas");
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

    let pixels = {
        let mut target = renderer
            .bind(&mut texture)
            .map_err(|error| anyhow::anyhow!("No se pudo enlazar FBO offscreen: {error:?}"))?;
        {
            let mut frame = renderer
                .render(&mut target, final_rect.size, Transform::Normal)
                .map_err(|error| anyhow::anyhow!("No se pudo iniciar frame offscreen: {error:?}"))?;
            let _ = frame.clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[offscreen_rect]);
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
            let _ = frame
                .finish()
                .map_err(|error| anyhow::anyhow!("No se pudo finalizar frame offscreen: {error:?}"))?;
        }

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
        unpremultiply_rgba(&mapped)
    };

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
            format!("No se pudo finalizar la miniatura {}", destination.display())
        });
    }
    Ok(())
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
