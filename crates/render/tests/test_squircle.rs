use anyhow::Result;
use malus_de_render::{SquircleInstance, SquircleRenderer, MACOS_SQUIRCLE_POWER};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

#[test]
fn test_squircle_rendering_and_png_export() -> Result<()> {
    // 1. Inicializar renderizador headless
    let mut renderer = SquircleRenderer::new_headless(wgpu::TextureFormat::Rgba8Unorm)?;

    let width = 800u32;
    let height = 260u32;

    // 2. Definir elementos de UI de prueba (simulando componentes de la Top-Bar y comparación G2 vs G1)
    let instances = vec![
        // Barra superior completa (Pill translúcida estilo macOS)
        SquircleInstance::new(20.0, 20.0, 760.0, 40.0, 20.0, [0.12, 0.12, 0.15, 0.88])
            .with_border(1.0, [1.0, 1.0, 1.0, 0.20])
            .with_power(MACOS_SQUIRCLE_POWER),

        // Botón Menú Apple (squircle n=4.2)
        SquircleInstance::new(32.0, 26.0, 48.0, 28.0, 8.0, [0.25, 0.25, 0.30, 0.60])
            .with_border(1.0, [1.0, 1.0, 1.0, 0.15])
            .with_power(MACOS_SQUIRCLE_POWER),

        // Botón de Aplicación Activa (Acento Azul macOS, squircle n=4.2)
        SquircleInstance::new(90.0, 26.0, 95.0, 28.0, 8.0, [0.0, 0.48, 1.0, 0.95])
            .with_border(1.0, [1.0, 1.0, 1.0, 0.30])
            .with_power(MACOS_SQUIRCLE_POWER),

        // Botón Menú Secundario
        SquircleInstance::new(195.0, 26.0, 70.0, 28.0, 8.0, [0.20, 0.20, 0.24, 0.50])
            .with_border(1.0, [1.0, 1.0, 1.0, 0.10])
            .with_power(MACOS_SQUIRCLE_POWER),

        // Widget de Centro de Control / Batería (derecha)
        SquircleInstance::new(670.0, 26.0, 98.0, 28.0, 14.0, [0.18, 0.18, 0.22, 0.75])
            .with_border(1.0, [1.0, 1.0, 1.0, 0.18])
            .with_power(MACOS_SQUIRCLE_POWER),

        // Tarjeta de Comparación 1: Curva Superelíptica macOS G2 (n=4.2)
        SquircleInstance::new(60.0, 90.0, 300.0, 140.0, 36.0, [0.10, 0.45, 0.85, 0.90])
            .with_border(2.0, [1.0, 1.0, 1.0, 0.40])
            .with_power(MACOS_SQUIRCLE_POWER),

        // Tarjeta de Comparación 2: Borde Circular Tradicional G1 (n=2.0)
        SquircleInstance::new(440.0, 90.0, 300.0, 140.0, 36.0, [0.85, 0.35, 0.15, 0.90])
            .with_border(2.0, [1.0, 1.0, 1.0, 0.40])
            .with_power(2.0),
    ];

    // 3. Renderizar a memoria (RGBA)
    let rgba_pixels = renderer.render_to_rgba_image(width, height, 1.0, &instances)?;
    assert_eq!(rgba_pixels.len(), (width * height * 4) as usize);

    // Verificar que hay píxeles renderizados (no está en blanco)
    let non_zero_pixels = rgba_pixels.iter().filter(|&&p| p > 0).count();
    assert!(non_zero_pixels > 1000, "La imagen renderizada no debe estar vacía");

    // 4. Guardar archivo PNG para inspección visual
    let output_dir = Path::new("/home/cristian/Proyectos/malus-de/target");
    std::fs::create_dir_all(output_dir)?;
    let output_path = output_dir.join("malus_squircle_poc.png");

    let file = File::create(&output_path)?;
    let ref mut w = BufWriter::new(file);

    let mut encoder = png::Encoder::new(w, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgba_pixels)?;

    println!("✅ Renderizado de squircle exitoso. Archivo guardado en: {:?}", output_path);
    assert!(output_path.exists());
    assert!(std::fs::metadata(&output_path)?.len() > 0);

    Ok(())
}
