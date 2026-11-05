// ==============================================================================
// Malus DE: Tests de Luminancia, Barra Transparente y Adaptabilidad de Color
// ==============================================================================

use anyhow::Result;
use malus_de_render::luminance::{calculate_luminance_rgba, AdaptivePalette};
use malus_de_render::renderer::SquircleRenderer;
use malus_de_render::text::TextSystem;
use malus_de_render::top_bar::{build_top_bar, TopBarConfig, TopBarModel};
use malus_de_render::layout::UiTree;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

#[test]
fn test_calculate_luminance_black_white_and_shades() {
    // 1. Negro absoluto
    let black = [0u8, 0, 0, 255];
    let lum_black = calculate_luminance_rgba(&black);
    assert!(lum_black < 0.001, "Luminancia de negro debe ser 0");

    // 2. Blanco puro
    let white = [255u8, 255, 255, 255];
    let lum_white = calculate_luminance_rgba(&white);
    assert!((lum_white - 1.0).abs() < 0.001, "Luminancia de blanco debe ser 1.0");

    // 3. Verde puro (debe tener mayor peso según ITU-R BT.709: 0.7152)
    let green = [0u8, 255, 0, 255];
    let lum_green = calculate_luminance_rgba(&green);
    assert!((lum_green - 0.7152).abs() < 0.01, "Luminancia de verde ≈ 0.7152");

    // 4. Azul puro (debe tener menor peso: 0.0722)
    let blue = [0u8, 0, 255, 255];
    let lum_blue = calculate_luminance_rgba(&blue);
    assert!((lum_blue - 0.0722).abs() < 0.01, "Luminancia de azul ≈ 0.0722");
}

#[test]
fn test_adaptive_palette_contrast_switch() {
    // Paleta para fondo oscuro
    let dark_bg_palette = AdaptivePalette::new(true);
    assert!(dark_bg_palette.is_dark_background);
    // El texto primario debe ser claro (R > 0.9)
    assert!(dark_bg_palette.text_primary[0] > 0.90);
    // El fondo de los botones en reposo debe ser 100% transparente
    assert_eq!(dark_bg_palette.button_idle_bg[3], 0.0);
    // El fondo del popover debe ser vidrio translúcido claro estilo macOS (menu-bar.jpg)
    assert!(dark_bg_palette.popover_bg[0] > 0.85);

    // Paleta para fondo claro
    let light_bg_palette = AdaptivePalette::new(false);
    assert!(!light_bg_palette.is_dark_background);
    // El texto primario debe ser grafito oscuro (R < 0.2)
    assert!(light_bg_palette.text_primary[0] < 0.20);
    // El fondo de los botones en reposo debe ser 100% transparente
    assert_eq!(light_bg_palette.button_idle_bg[3], 0.0);
    // El fondo del popover debe ser claro
    assert!(light_bg_palette.popover_bg[0] > 0.85);
}

#[test]
fn test_transparent_topbar_rendering_and_text_adaptation() -> Result<()> {
    let screen_w = 1280u32;
    let screen_h = 40u32;

    let mut renderer = SquircleRenderer::new_headless(wgpu::TextureFormat::Rgba8Unorm)?;
    let mut text_system = TextSystem::new(&renderer.device, &renderer.queue, renderer.target_format);

    // Configuración de barra 100% transparente sin bordes ni píldora opaca
    let config = TopBarConfig {
        height: 32.0,
        is_floating_pill: false,
        margin_x: 0.0,
        margin_top: 0.0,
        bg_color: [0.0, 0.0, 0.0, 0.0],
        border_color: [0.0, 0.0, 0.0, 0.0],
        border_width: 0.0,
    };

    // --------------------------------------------------------------------------
    // 1. Renderizar Barra Transparente sobre Fondo Oscuro (Texto Blanco)
    // --------------------------------------------------------------------------
    let mut ui_dark = UiTree::new();
    let model_dark = TopBarModel {
        app_name: "Finder".to_string(),
        menus: vec![
            "Archivo".to_string(),
            "Edición".to_string(),
            "Visualización".to_string(),
            "Ir".to_string(),
            "Ventana".to_string(),
            "Ayuda".to_string(),
        ],
        status_items: vec!["WiFi".into(), "100%".into()],
        clock_text: "Mié 4 Nov 22:45".to_string(),
        is_dark_background: true,
        active_popover: None,
    };

    let _nodes_dark = build_top_bar(&mut ui_dark, &config, &model_dark)?;
    ui_dark.compute_layout(screen_w as f32, screen_h as f32)?;

    let instances_dark = ui_dark.generate_instances();
    let labels_dark = ui_dark.generate_text_labels();

    // En una barra transparente en reposo, los botones no generan quads de fondo opaco
    // (solo el texto o elementos interactivos se dibujan)
    assert!(labels_dark.len() >= 9);

    let rgba_dark = renderer.render_scene_to_rgba_image(
        &mut text_system,
        screen_w,
        screen_h,
        1.0,
        &instances_dark,
        &labels_dark,
    )?;

    let out_dark = Path::new("/home/cristian/Proyectos/malus-de/target/malus_topbar_transparent_dark.png");
    let file_d = File::create(out_dark)?;
    let ref mut w_d = BufWriter::new(file_d);
    let mut enc_d = png::Encoder::new(w_d, screen_w, screen_h);
    enc_d.set_color(png::ColorType::Rgba);
    enc_d.set_depth(png::BitDepth::Eight);
    let mut writer_d = enc_d.write_header()?;
    writer_d.write_image_data(&rgba_dark)?;
    assert!(out_dark.exists());

    // --------------------------------------------------------------------------
    // 2. Renderizar Barra Transparente sobre Fondo Claro (Texto Grafito Oscuro)
    // --------------------------------------------------------------------------
    let mut ui_light = UiTree::new();
    let model_light = TopBarModel {
        app_name: "Safari".to_string(),
        menus: vec![
            "Archivo".to_string(),
            "Edición".to_string(),
            "Historial".to_string(),
            "Marcadores".to_string(),
            "Ventana".to_string(),
            "Ayuda".to_string(),
        ],
        status_items: vec!["WiFi".into(), "95%".into()],
        clock_text: "Mié 4 Nov 22:45".to_string(),
        is_dark_background: false, // Fondo claro detectado
        active_popover: None,
    };

    let _nodes_light = build_top_bar(&mut ui_light, &config, &model_light)?;
    ui_light.compute_layout(screen_w as f32, screen_h as f32)?;

    let instances_light = ui_light.generate_instances();
    let labels_light = ui_light.generate_text_labels();

    // Validar que las etiquetas cambiaron a color oscuro
    let safari_label = labels_light.iter().find(|(l, _, _)| l.text == "Safari").unwrap();
    assert!(safari_label.0.color[0] < 0.20, "El texto en fondo claro debe ser oscuro");

    let rgba_light = renderer.render_scene_to_rgba_image(
        &mut text_system,
        screen_w,
        screen_h,
        1.0,
        &instances_light,
        &labels_light,
    )?;

    let out_light = Path::new("/home/cristian/Proyectos/malus-de/target/malus_topbar_transparent_light.png");
    let file_l = File::create(out_light)?;
    let ref mut w_l = BufWriter::new(file_l);
    let mut enc_l = png::Encoder::new(w_l, screen_w, screen_h);
    enc_l.set_color(png::ColorType::Rgba);
    enc_l.set_depth(png::BitDepth::Eight);
    let mut writer_l = enc_l.write_header()?;
    writer_l.write_image_data(&rgba_light)?;
    assert!(out_light.exists());

    Ok(())
}
