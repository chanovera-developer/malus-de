use anyhow::Result;
use malus_de_render::{
    build_top_bar, SquircleRenderer, TextSystem, TopBarConfig, TopBarModel, UiTree,
};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

#[test]
fn test_cosmic_text_typography_and_topbar_rendering() -> Result<()> {
    let mut ui = UiTree::new();
    let config = TopBarConfig {
        height: 36.0,
        is_floating_pill: true,
        margin_x: 20.0,
        margin_top: 8.0,
        bg_color: [0.12, 0.12, 0.15, 0.88],
        border_color: [1.0, 1.0, 1.0, 0.20],
        border_width: 1.0,
    };
    let model = TopBarModel {
        app_name: "Finder".to_string(),
        menus: vec![
            "Archivo".to_string(),
            "Edición".to_string(),
            "Visualización".to_string(),
            "Ir".to_string(),
            "Ventana".to_string(),
            "Ayuda".to_string(),
        ],
        status_items: vec!["WiFi".to_string(), "100%".to_string()],
        clock_text: "Mié 4 Nov 21:30".to_string(),
    };

    // 1. Construir la barra superior con Flexbox y etiquetas de texto
    let _nodes = build_top_bar(&mut ui, &config, &model)?;

    // 2. Calcular layout espacial con Taffy
    let screen_w = 1280.0f32;
    let screen_h = 80.0f32;
    ui.compute_layout(screen_w, screen_h)?;

    // 3. Obtener instancias geométricas y etiquetas de texto posicionadas
    let instances = ui.generate_instances();
    let labels = ui.generate_text_labels();

    assert!(!instances.is_empty(), "Deben generarse instancias de squircle");
    assert!(labels.len() >= 10, "Deben generarse al menos 10 etiquetas de texto para los botones");

    // Verificar que existen las etiquetas clave
    let has_finder = labels.iter().any(|(lbl, _, _)| lbl.text == "Finder");
    let has_clock = labels.iter().any(|(lbl, _, _)| lbl.text == "Mié 4 Nov 21:30");
    let has_apple = labels.iter().any(|(lbl, _, _)| lbl.text == "🍎");
    assert!(has_finder, "Debe contener la etiqueta de la app activa 'Finder'");
    assert!(has_clock, "Debe contener la etiqueta del reloj");
    assert!(has_apple, "Debe contener el icono de Apple");

    // 4. Inicializar WGPU y el subsistema de texto cosmic-text / glyphon
    let mut renderer = SquircleRenderer::new_headless(wgpu::TextureFormat::Rgba8Unorm)?;
    let mut text_system = TextSystem::new(&renderer.device, &renderer.queue, renderer.target_format);

    // 5. Renderizar escena completa (geometría + texto)
    let rgba_pixels = renderer.render_scene_to_rgba_image(
        &mut text_system,
        screen_w as u32,
        screen_h as u32,
        1.0,
        &instances,
        &labels,
    )?;

    assert_eq!(rgba_pixels.len(), (screen_w as u32 * screen_h as u32 * 4) as usize);

    // 6. Guardar imagen compuesta como PNG
    let output_path = Path::new("/home/cristian/Proyectos/malus-de/target/malus_topbar_complete.png");
    let file = File::create(output_path)?;
    let ref mut w = BufWriter::new(file);

    let mut encoder = png::Encoder::new(w, screen_w as u32, screen_h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgba_pixels)?;

    println!("✅ Top-Bar completa con texto y squircles renderizada en: {:?}", output_path);
    assert!(output_path.exists());
    assert!(std::fs::metadata(output_path)?.len() > 0);

    Ok(())
}
