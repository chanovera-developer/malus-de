use anyhow::{bail, Context, Result};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{ExportMem, TextureMapping},
    },
    utils::{Buffer as BufferCoord, Physical, Rectangle, Size},
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

    let image = orient_pixels(&pixels, size.w as usize * 4, mapping.flipped());
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

fn orient_pixels(pixels: &[u8], row_len: usize, flipped: bool) -> Vec<u8> {
    if flipped {
        pixels
            .chunks_exact(row_len)
            .rev()
            .flatten()
            .copied()
            .collect()
    } else {
        pixels.to_vec()
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

    #[test]
    fn flips_rows_without_changing_pixel_order() {
        let pixels = [1, 2, 3, 4, 5, 6, 7, 8];

        assert_eq!(orient_pixels(&pixels, 4, true), [5, 6, 7, 8, 1, 2, 3, 4]);
        assert_eq!(orient_pixels(&pixels, 4, false), pixels);
    }
}
