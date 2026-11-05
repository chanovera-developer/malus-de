use anyhow::Result;
use malus_de_render::{
    build_top_bar, SquircleRenderer, TopBarConfig, TopBarModel, UiTree,
};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

#[test]
fn test_taffy_flexbox_topbar_layout_and_render() -> Result<()> {
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
    let model = TopBarModel::default();

    // 1. Construir la estructura reactiva de la Top Bar
    let nodes = build_top_bar(&mut ui, &config, &model)?;

    // 2. Calcular layout para resolución de 1280x80
    let screen_w = 1280.0f32;
    let screen_h = 80.0f32;
    ui.compute_layout(screen_w, screen_h)?;

    // 3. Validar geometría calculada por Taffy
    let pill_layout = ui.taffy.layout(nodes.pill).unwrap();
    assert_eq!(pill_layout.size.height, 36.0);
    // El ancho de la píldora es 1280 - (20 * 2 de márgenes laterales) = 1240 px
    assert_eq!(pill_layout.size.width, 1240.0);
    // Su posición x es el margen izquierdo = 20 px
    assert_eq!(pill_layout.location.x, 20.0);

    let apple_layout = ui.taffy.layout(nodes.apple_btn).unwrap();
    assert_eq!(apple_layout.size.width, 28.0);
    assert_eq!(apple_layout.size.height, 24.0);

    let clock_layout = ui.taffy.layout(nodes.clock_btn).unwrap();
    assert_eq!(clock_layout.size.width, 110.0);
    assert_eq!(clock_layout.size.height, 24.0);

    // 4. Hit-Testing matemático
    // Margen pill X=20 + padding=8 + dentro del botón Apple (28px ancho) -> (35, 18)
    let hit_apple = ui.hit_test([35.0, 18.0]);
    assert_eq!(hit_apple, Some(nodes.apple_btn), "El clic debe acertar el botón Apple");

    // Clic en la esquina vacía superior izquierda de la pantalla (fuera de la píldora)
    let hit_corner = ui.hit_test([2.0, 2.0]);
    assert_eq!(hit_corner, None, "Un clic fuera de la píldora no debe colisionar con ningún botón");

    // 5. Generar instancias para la GPU
    let instances = ui.generate_instances();
    assert!(!instances.is_empty(), "Debe haber instancias generadas");

    // 6. Renderizar a imagen PNG con WGPU
    let mut renderer = SquircleRenderer::new_headless(wgpu::TextureFormat::Rgba8Unorm)?;
    let rgba_pixels = renderer.render_to_rgba_image(screen_w as u32, screen_h as u32, 1.0, &instances)?;

    let output_path = Path::new("/home/cristian/Proyectos/malus-de/target/malus_topbar_taffy.png");
    let file = File::create(output_path)?;
    let ref mut w = BufWriter::new(file);

    let mut encoder = png::Encoder::new(w, screen_w as u32, screen_h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgba_pixels)?;

    println!("✅ Top-Bar con Flexbox Taffy generada con éxito en: {:?}", output_path);
    assert!(output_path.exists());
    assert!(std::fs::metadata(output_path)?.len() > 0);

    Ok(())
}
