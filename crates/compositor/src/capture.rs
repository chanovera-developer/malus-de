use anyhow::{bail, Context, Result};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::ExportMem,
    },
    utils::{Buffer as BufferCoord, Logical, Physical, Point, Rectangle, Size, Transform},
};
use std::{
    fs::{self, OpenOptions},
    path::Path,
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

pub fn capture_window_thumbnail<R>(
    renderer: &mut R,
    target: &R::Framebuffer<'_>,
    framebuffer_size: Size<i32, Physical>,
    window_location: Point<i32, Logical>,
    window_size: Size<i32, Logical>,
    output_origin: Point<i32, Logical>,
    scale: f64,
    destination: &Path,
    transform: Transform,
) -> Result<()>
where
    R: ExportMem,
{
    let region_x = ((window_location.x - output_origin.x) as f64 * scale).round();
    let region_y = ((window_location.y - output_origin.y) as f64 * scale).round();
    let region_w = (window_size.w as f64 * scale).round();
    let region_h = (window_size.h as f64 * scale).round();
    if !scale.is_finite()
        || scale <= 0.0
        || [region_x, region_y, region_w, region_h]
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0 || *value > i32::MAX as f64)
    {
        bail!("La geometría de la ventana no es válida para capturar su miniatura");
    }

    let region_x = region_x as i32;
    let region_y = region_y as i32;
    let region_w = region_w as i32;
    let region_h = region_h as i32;
    if region_w == 0
        || region_h == 0
        || i64::from(region_x) + i64::from(region_w) > i64::from(framebuffer_size.w)
        || i64::from(region_y) + i64::from(region_h) > i64::from(framebuffer_size.h)
    {
        bail!("La ventana no cabe dentro del fotograma de salida para capturar su miniatura");
    }

    let region = Rectangle::new((region_x, region_y).into(), (region_w, region_h).into());
    let mapping = renderer
        .copy_framebuffer(target, region, Fourcc::Abgr8888)
        .map_err(|error| {
            anyhow::anyhow!("No se pudo leer el framebuffer de la ventana: {error:?}")
        })?;
    let pixels = renderer.map_texture(&mapping).map_err(|error| {
        anyhow::anyhow!("No se pudo mapear la miniatura capturada: {error:?}")
    })?;
    let expected_len = (region_w as usize)
        .checked_mul(region_h as usize)
        .and_then(|pixel_count| pixel_count.checked_mul(4))
        .context("El tamaño de la miniatura excede el tamaño permitido")?;
    if pixels.len() != expected_len {
        bail!(
            "La miniatura tiene un tamaño inesperado: {} bytes, se esperaban {expected_len}",
            pixels.len()
        );
    }

    let image = orient_pixels(&pixels, region_w as usize * 4, transform);
    let temporary = destination.with_extension("pending");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .with_context(|| format!("No se pudo crear {}", temporary.display()))?;
    let mut encoder = png::Encoder::new(file, region_w as u32, region_h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = match encoder.write_header() {
        Ok(writer) => writer,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error).context("No se pudo iniciar la escritura de la miniatura PNG");
        }
    };
    if let Err(error) = writer.write_image_data(&image) {
        drop(writer);
        let _ = fs::remove_file(&temporary);
        return Err(error).context("No se pudo escribir la miniatura PNG");
    }
    drop(writer);
    if let Err(error) = fs::hard_link(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| {
            format!("No se pudo finalizar la miniatura {}", destination.display())
        });
    }
    if let Err(error) = fs::remove_file(&temporary) {
        tracing::warn!(
            "No se pudo limpiar el archivo temporal {}: {error}",
            temporary.display()
        );
    }
    Ok(())
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
    use super::orient_pixels;
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
}
