// ==============================================================================
// Malus DE: Tests de Integración para Menús Desplegables y Popovers WGPU
// ==============================================================================

use malus_de_render::layout::UiTree;
use malus_de_render::popover::{build_popover_menu, PopoverConfig, PopoverMenuModel};
use malus_de_render::renderer::SquircleRenderer;
use malus_de_render::text::TextSystem;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

#[test]
fn test_popover_layout_and_item_geometry() {
    let mut ui = UiTree::new();
    let config = PopoverConfig::default();
    let model = PopoverMenuModel::apple_menu(20.0, 36.0);

    let popover_nodes =
        build_popover_menu(&mut ui, &config, &model).expect("Error al construir popover");
    ui.set_root(popover_nodes.root_container);

    ui.compute_layout(800.0, 600.0)
        .expect("Error al computar layout de popover");

    let card_layout = ui
        .taffy
        .layout(popover_nodes.card)
        .expect("Layout de tarjeta popover no encontrado");

    assert!(
        (card_layout.size.width - config.width).abs() < 1.0,
        "El ancho de la tarjeta {} no coincide con la config {}",
        card_layout.size.width,
        config.width
    );
    assert!(
        card_layout.size.height > 150.0,
        "La altura de la tarjeta {} es demasiado pequeña",
        card_layout.size.height
    );

    assert_eq!(
        popover_nodes.items.len(),
        10,
        "Deben existir 10 ítems interactivos en el menú Apple estilo macOS"
    );

    // Verificar que cada ítem tiene altura correcta
    for (idx, item_node, item) in &popover_nodes.items {
        let item_layout = ui
            .taffy
            .layout(*item_node)
            .expect("Layout de ítem no encontrado");
        assert!(
            item_layout.size.height > 15.0,
            "El ítem [{}] '{}' tiene altura inválida {}",
            idx,
            item.label,
            item_layout.size.height
        );
    }
}

#[test]
fn test_popover_hit_testing_analytical() {
    let mut ui = UiTree::new();
    let config = PopoverConfig::default();
    let model = PopoverMenuModel::apple_menu(30.0, 40.0);

    let popover_nodes =
        build_popover_menu(&mut ui, &config, &model).expect("Error al construir popover");
    ui.set_root(popover_nodes.root_container);

    ui.compute_layout(800.0, 600.0)
        .expect("Error al computar layout de popover");

    // Probar clic dentro del primer ítem ("Acerca de Malus DE")
    // Anchor X = 30.0, Card padding_x = 6.0, ítem width ≈ 228.0
    // Anchor Y = 40.0, Card padding_y = 6.0, primer ítem top = 46.0, height = 24.0
    let hit_first = ui.hit_test([40.0, 52.0]);
    assert!(
        hit_first.is_some(),
        "El hit-test debe detectar el primer ítem"
    );
    let hit_node = hit_first.unwrap();

    let matched_item = popover_nodes.find_item_by_node(hit_node);
    assert!(
        matched_item.is_some(),
        "El nodo detectado debe mapearse a un PopoverItem"
    );
    assert_eq!(
        matched_item.unwrap().label,
        "Acerca de este Mac",
        "El ítem detectado debe ser 'Acerca de este Mac'"
    );

    // Probar clic fuera del popover (ej. en x = 500, y = 500)
    let hit_outside = ui.hit_test([500.0, 500.0]);
    assert!(
        hit_outside.is_none(),
        "El hit-test fuera del popover no debe interceptar eventos"
    );
}

#[test]
fn test_popover_rendering_and_png_export() {
    let width = 340u32;
    let height = 380u32;

    let mut renderer =
        SquircleRenderer::new_headless(wgpu::TextureFormat::Rgba8Unorm)
            .expect("Error al inicializar SquircleRenderer headless");
    let mut text_system =
        TextSystem::new(&renderer.device, &renderer.queue, renderer.target_format);

    // 1. Renderizar Popover del Menú Apple
    let mut ui_apple = UiTree::new();
    let config = PopoverConfig::default();
    let mut model_apple = PopoverMenuModel::apple_menu(30.0, 20.0);
    model_apple.hovered_index = Some(10); // "Apagar el Equipo..." (índice 10) resaltado en azul como en menu-bar.jpg

    let nodes_apple =
        build_popover_menu(&mut ui_apple, &config, &model_apple)
            .expect("Error al construir popover Apple");
    ui_apple.set_root(nodes_apple.root_container);

    ui_apple
        .compute_layout(width as f32, height as f32)
        .expect("Error al computar layout");

    let instances_apple = ui_apple.generate_instances();
    let text_labels_apple = ui_apple.generate_text_labels();

    println!("Instances count: {}", instances_apple.len());
    for (i, inst) in instances_apple.iter().enumerate() {
        println!("inst[{}]: pos={:?} size={:?} bg={:?} border_w={}", i, inst.position, inst.size, inst.bg_color, inst.border_width);
    }

    assert!(nodes_apple.outer_shadow.is_some(), "La sombra debe estar activada para profundidad macOS");
    assert!(nodes_apple.inner_shadow.is_none(), "La sombra debe ser una sola sombra, no multicapa");
    assert!(
        !instances_apple.is_empty(),
        "Debe haber instancias generadas para el popover (tarjeta + ítems)"
    );
    assert!(
        !text_labels_apple.is_empty(),
        "Debe haber etiquetas tipográficas generadas para el popover"
    );

    let rgba_apple = renderer
        .render_scene_to_rgba_image(
            &mut text_system,
            width,
            height,
            1.0,
            &instances_apple,
            &text_labels_apple,
        )
        .expect("Error al renderizar escena a imagen RGBA");

    let out_apple = Path::new("/home/cristian/Proyectos/malus-de/target/malus_popover_apple.png");
    let _ = std::fs::create_dir_all("/home/cristian/Proyectos/malus-de/target");
    let file = File::create(out_apple).expect("Error al crear archivo PNG Apple");
    let ref mut w = BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("Error al escribir cabecera PNG");
    writer
        .write_image_data(&rgba_apple)
        .expect("Error al escribir datos PNG");

    assert!(out_apple.exists(), "El archivo PNG del menú Apple debe existir");

    // 2. Renderizar Popover con Ítem Activo en Hover ("Archivo" -> "Guardar")
    let mut ui_file = UiTree::new();
    let mut model_file = PopoverMenuModel::sample_file_menu(30.0, 20.0);
    // Simular hover sobre el ítem 4 ("Guardar")
    model_file.hovered_index = Some(4);

    let nodes_file =
        build_popover_menu(&mut ui_file, &config, &model_file)
            .expect("Error al construir popover Archivo");
    ui_file.set_root(nodes_file.root_container);

    ui_file
        .compute_layout(width as f32, height as f32)
        .expect("Error al computar layout");

    let instances_file = ui_file.generate_instances();
    let text_labels_file = ui_file.generate_text_labels();

    let rgba_file = renderer
        .render_scene_to_rgba_image(
            &mut text_system,
            width,
            height,
            1.0,
            &instances_file,
            &text_labels_file,
        )
        .expect("Error al renderizar escena con hover");

    let out_file = Path::new("/home/cristian/Proyectos/malus-de/target/malus_popover_file.png");
    let file_f = File::create(out_file).expect("Error al crear archivo PNG File");
    let ref mut w_f = BufWriter::new(file_f);
    let mut encoder_f = png::Encoder::new(w_f, width, height);
    encoder_f.set_color(png::ColorType::Rgba);
    encoder_f.set_depth(png::BitDepth::Eight);
    let mut writer_f = encoder_f.write_header().expect("Error al escribir cabecera PNG");
    writer_f
        .write_image_data(&rgba_file)
        .expect("Error al escribir datos PNG");

    assert!(out_file.exists(), "El archivo PNG del menú Archivo debe existir");
}

#[test]
fn test_control_center_and_tray_rendering_and_png_export() {
    let width = 360u32;
    let height = 480u32;

    let mut renderer =
        SquircleRenderer::new_headless(wgpu::TextureFormat::Rgba8Unorm)
            .expect("Error al inicializar SquircleRenderer headless");
    let mut text_system =
        TextSystem::new(&renderer.device, &renderer.queue, renderer.target_format);

    // 1. Renderizar Centro de Control modular estilo macOS Sonoma / Tahoe
    let mut ui_cc = UiTree::new();
    let config = PopoverConfig::default();
    let model_cc = PopoverMenuModel::control_center_popover(20.0, 20.0);

    let nodes_cc =
        build_popover_menu(&mut ui_cc, &config, &model_cc)
            .expect("Error al construir Centro de Control");
    ui_cc.set_root(nodes_cc.root_container);

    ui_cc
        .compute_layout(width as f32, height as f32)
        .expect("Error al computar layout del Centro de Control");

    let instances_cc = ui_cc.generate_instances();
    let text_labels_cc = ui_cc.generate_text_labels();

    assert!(
        instances_cc.len() > 10,
        "Debe haber múltiples tarjetas y squircles para el Centro de Control modular"
    );

    let rgba_cc = renderer
        .render_scene_to_rgba_image(
            &mut text_system,
            width,
            height,
            1.0,
            &instances_cc,
            &text_labels_cc,
        )
        .expect("Error al renderizar Centro de Control a imagen RGBA");

    let out_cc = Path::new("/home/cristian/Proyectos/malus-de/target/malus_control_center.png");
    let file_cc = File::create(out_cc).expect("Error al crear archivo PNG Control Center");
    let ref mut w_cc = BufWriter::new(file_cc);
    let mut encoder_cc = png::Encoder::new(w_cc, width, height);
    encoder_cc.set_color(png::ColorType::Rgba);
    encoder_cc.set_depth(png::BitDepth::Eight);
    let mut writer_cc = encoder_cc.write_header().expect("Error al escribir cabecera PNG");
    writer_cc
        .write_image_data(&rgba_cc)
        .expect("Error al escribir datos PNG");

    assert!(out_cc.exists(), "El archivo PNG del Centro de Control debe existir");

    // 2. Renderizar Bandeja del Sistema flotante (estilo Bartender)
    let mut ui_tray = UiTree::new();
    let model_tray = PopoverMenuModel::tray_popover(20.0, 20.0, &[]);

    let nodes_tray =
        build_popover_menu(&mut ui_tray, &config, &model_tray)
            .expect("Error al construir Bandeja del Sistema");
    ui_tray.set_root(nodes_tray.root_container);

    ui_tray
        .compute_layout(width as f32, height as f32)
        .expect("Error al computar layout de la Bandeja");

    let instances_tray = ui_tray.generate_instances();
    let text_labels_tray = ui_tray.generate_text_labels();

    let rgba_tray = renderer
        .render_scene_to_rgba_image(
            &mut text_system,
            width,
            height,
            1.0,
            &instances_tray,
            &text_labels_tray,
        )
        .expect("Error al renderizar Bandeja a imagen RGBA");

    let out_tray = Path::new("/home/cristian/Proyectos/malus-de/target/malus_tray_popover.png");
    let file_tray = File::create(out_tray).expect("Error al crear archivo PNG Bandeja");
    let ref mut w_tray = BufWriter::new(file_tray);
    let mut encoder_tray = png::Encoder::new(w_tray, width, height);
    encoder_tray.set_color(png::ColorType::Rgba);
    encoder_tray.set_depth(png::BitDepth::Eight);
    let mut writer_tray = encoder_tray.write_header().expect("Error al escribir cabecera PNG");
    writer_tray
        .write_image_data(&rgba_tray)
        .expect("Error al escribir datos PNG");

    assert!(out_tray.exists(), "El archivo PNG de la Bandeja debe existir");
}
