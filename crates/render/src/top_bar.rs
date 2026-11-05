// ==============================================================================
// Malus DE: Constructor Declarativo de la Barra Superior (Top-Bar macOS)
// ==============================================================================

use crate::layout::{SquircleStyle, UiTree};
use crate::popover::{build_popover_menu, PopoverConfig, PopoverMenuModel, PopoverNodes};
use crate::text::{TextLabel, Weight};
use anyhow::Result;
use taffy::prelude::*;

/// Configuración visual y de geometría para la barra superior.
#[derive(Clone, Debug)]
pub struct TopBarConfig {
    pub height: f32,
    pub is_floating_pill: bool,
    pub margin_x: f32,
    pub margin_top: f32,
    pub bg_color: [f32; 4],
    pub border_color: [f32; 4],
    pub border_width: f32,
}

impl Default for TopBarConfig {
    fn default() -> Self {
        Self {
            height: 32.0,
            is_floating_pill: true,
            margin_x: 16.0,
            margin_top: 6.0,
            bg_color: [0.12, 0.12, 0.15, 0.85],
            border_color: [1.0, 1.0, 1.0, 0.18],
            border_width: 1.0,
        }
    }
}

/// Datos y estado del modelo del Menú Global y barra de estado.
#[derive(Clone, Debug)]
pub struct TopBarModel {
    pub app_name: String,
    pub menus: Vec<String>,
    pub status_items: Vec<String>,
    pub clock_text: String,
    pub active_popover: Option<PopoverMenuModel>,
}

impl Default for TopBarModel {
    fn default() -> Self {
        Self {
            app_name: "Finder".to_string(),
            menus: vec![
                "Archivo".to_string(),
                "Edición".to_string(),
                "Visualización".to_string(),
                "Ir".to_string(),
                "Ventana".to_string(),
                "Ayuda".to_string(),
            ],
            status_items: vec!["WiFi".to_string(), "Batería".to_string()],
            clock_text: "Mié 4 Nov 21:25".to_string(),
            active_popover: None,
        }
    }
}

/// Identificadores de nodos creados en el árbol de layout para interacción.
pub struct TopBarNodes {
    pub root: NodeId,
    pub pill: NodeId,
    pub apple_btn: NodeId,
    pub app_btn: NodeId,
    pub menu_items: Vec<(String, NodeId)>,
    pub control_center_btn: NodeId,
    pub clock_btn: NodeId,
    pub popover: Option<PopoverNodes>,
}

/// Construye la estructura Flexbox completa de la barra superior inspirada en macOS.
pub fn build_top_bar(
    ui: &mut UiTree,
    config: &TopBarConfig,
    model: &TopBarModel,
) -> Result<TopBarNodes> {
    // --------------------------------------------------------------------------
    // 1. Sección Izquierda: [Logo Apple] + [Nombre App en Negrita] + [Menús Globales]
    // --------------------------------------------------------------------------
    let mut left_children = Vec::new();

    // Determinar qué menú o botón se encuentra activo en popover
    let active_menu_id = model
        .active_popover
        .as_ref()
        .map(|p| p.menu_id.as_str())
        .unwrap_or("");

    // Botón Logo Apple (resaltado si su popover está desplegado)
    let apple_style = if active_menu_id == "apple" {
        SquircleStyle::button([0.0, 0.48, 1.0, 0.90]).with_border(1.0, [1.0, 1.0, 1.0, 0.35])
    } else {
        SquircleStyle::button([0.25, 0.25, 0.30, 0.60]).with_border(1.0, [1.0, 1.0, 1.0, 0.12])
    };

    let apple_btn = ui.add_leaf(
        Style {
            size: Size {
                width: Dimension::length(28.0),
                height: Dimension::length(24.0),
            },
            ..Default::default()
        },
        apple_style,
        Some("apple_menu"),
    )?;
    ui.set_text(apple_btn, TextLabel::new("🍎").with_size(13.0));
    left_children.push(apple_btn);

    // Botón Nombre de Aplicación Activa (Acento azul macOS)
    let app_btn = ui.add_leaf(
        Style {
            size: Size {
                width: Dimension::length(75.0),
                height: Dimension::length(24.0),
            },
            ..Default::default()
        },
        SquircleStyle::button([0.0, 0.48, 1.0, 0.90]).with_border(1.0, [1.0, 1.0, 1.0, 0.25]),
        Some(format!("app:{}", model.app_name)),
    )?;
    ui.set_text(
        app_btn,
        TextLabel::new(&model.app_name)
            .with_weight(Weight::BOLD)
            .with_size(13.0),
    );
    left_children.push(app_btn);

    // Elementos del menú activo de la aplicación
    let mut menu_items = Vec::new();
    for menu in &model.menus {
        let menu_id_tag = format!("menu:{}", menu);
        let is_menu_active = active_menu_id == menu_id_tag;

        let menu_style = if is_menu_active {
            SquircleStyle::button([0.0, 0.48, 1.0, 0.90]).with_border(1.0, [1.0, 1.0, 1.0, 0.35])
        } else {
            SquircleStyle::button([0.20, 0.20, 0.24, 0.45]).with_border(1.0, [1.0, 1.0, 1.0, 0.08])
        };

        let menu_node = ui.add_leaf(
            Style {
                size: Size {
                    width: Dimension::length(58.0),
                    height: Dimension::length(24.0),
                },
                ..Default::default()
            },
            menu_style,
            Some(menu_id_tag),
        )?;
        ui.set_text(
            menu_node,
            TextLabel::new(menu)
                .with_weight(if is_menu_active { Weight::MEDIUM } else { Weight::NORMAL })
                .with_size(13.0),
        );
        menu_items.push((menu.clone(), menu_node));
        left_children.push(menu_node);
    }

    let left_section = ui.add_container(
        Style {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::CENTER,
            gap: Size {
                width: LengthPercentage::length(4.0),
                height: LengthPercentage::length(0.0),
            },
            ..Default::default()
        },
        SquircleStyle::transparent(),
        &left_children,
        Some("left_section"),
    )?;

    // --------------------------------------------------------------------------
    // 2. Sección Central: Espaciador Flexible (FlexGrow = 1.0)
    // --------------------------------------------------------------------------
    let center_spacer = ui.add_leaf(
        Style {
            flex_grow: 1.0,
            ..Default::default()
        },
        SquircleStyle::transparent(),
        Some("center_spacer"),
    )?;

    // --------------------------------------------------------------------------
    // 3. Sección Derecha: [Iconos de Estado] + [Control Center] + [Reloj]
    // --------------------------------------------------------------------------
    let mut right_children = Vec::new();

    for item in &model.status_items {
        let item_node = ui.add_leaf(
            Style {
                size: Size {
                    width: Dimension::length(36.0),
                    height: Dimension::length(24.0),
                },
                ..Default::default()
            },
            SquircleStyle::button([0.18, 0.18, 0.22, 0.50]).with_border(1.0, [1.0, 1.0, 1.0, 0.10]),
            Some(format!("status:{}", item)),
        )?;
        ui.set_text(
            item_node,
            TextLabel::new(item)
                .with_size(11.0)
                .with_color([0.80, 0.80, 0.85, 1.0]),
        );
        right_children.push(item_node);
    }

    // Botón Centro de Control (Pill compacta)
    let control_center_btn = ui.add_leaf(
        Style {
            size: Size {
                width: Dimension::length(38.0),
                height: Dimension::length(24.0),
            },
            ..Default::default()
        },
        SquircleStyle::pill([0.22, 0.22, 0.26, 0.70]).with_border(1.0, [1.0, 1.0, 1.0, 0.15]),
        Some("control_center"),
    )?;
    ui.set_text(
        control_center_btn,
        TextLabel::new("CC")
            .with_weight(Weight::BOLD)
            .with_size(11.0),
    );
    right_children.push(control_center_btn);

    // Botón Reloj
    let clock_btn = ui.add_leaf(
        Style {
            size: Size {
                width: Dimension::length(110.0),
                height: Dimension::length(24.0),
            },
            ..Default::default()
        },
        SquircleStyle::button([0.16, 0.16, 0.20, 0.60]).with_border(1.0, [1.0, 1.0, 1.0, 0.12]),
        Some("clock"),
    )?;
    ui.set_text(
        clock_btn,
        TextLabel::new(&model.clock_text)
            .with_weight(Weight::SEMIBOLD)
            .with_size(12.5),
    );
    right_children.push(clock_btn);

    let right_section = ui.add_container(
        Style {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::CENTER,
            gap: Size {
                width: LengthPercentage::length(6.0),
                height: LengthPercentage::length(0.0),
            },
            ..Default::default()
        },
        SquircleStyle::transparent(),
        &right_children,
        Some("right_section"),
    )?;

    // --------------------------------------------------------------------------
    // 4. Contenedor Principal (Pill o Barra de Borde a Borde)
    // --------------------------------------------------------------------------
    let pill_style = if config.is_floating_pill {
        SquircleStyle::pill(config.bg_color).with_border(config.border_width, config.border_color)
    } else {
        SquircleStyle::card(0.0, config.bg_color).with_border(config.border_width, config.border_color)
    };

    let pill = ui.add_container(
        Style {
            size: Size {
                width: Dimension::auto(),
                height: Dimension::length(config.height),
            },
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::CENTER,
            margin: Rect {
                left: LengthPercentageAuto::length(if config.is_floating_pill { config.margin_x } else { 0.0 }),
                right: LengthPercentageAuto::length(if config.is_floating_pill { config.margin_x } else { 0.0 }),
                top: LengthPercentageAuto::length(if config.is_floating_pill { config.margin_top } else { 0.0 }),
                bottom: LengthPercentageAuto::length(0.0),
            },
            padding: Rect {
                left: LengthPercentage::length(8.0),
                right: LengthPercentage::length(8.0),
                top: LengthPercentage::length(2.0),
                bottom: LengthPercentage::length(2.0),
            },
            ..Default::default()
        },
        pill_style,
        &[left_section, center_spacer, right_section],
        Some("top_bar_pill"),
    )?;

    // --------------------------------------------------------------------------
    // 5. Construir Popover activo si está presente en el modelo
    // --------------------------------------------------------------------------
    let popover_nodes = if let Some(ref popover_model) = model.active_popover {
        build_popover_menu(ui, &PopoverConfig::default(), popover_model).ok()
    } else {
        None
    };

    let mut screen_children = vec![pill];
    if let Some(ref pn) = popover_nodes {
        screen_children.push(pn.root_container);
    }

    // Envoltorio de pantalla (Screen Root) que estira la píldora respetando sus márgenes horizontales
    let root = ui.add_container(
        Style {
            size: Size {
                width: Dimension::percent(1.0),
                height: Dimension::percent(1.0),
            },
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::STRETCH,
            ..Default::default()
        },
        SquircleStyle::transparent(),
        &screen_children,
        Some("top_bar_screen_root"),
    )?;

    ui.set_root(root);

    Ok(TopBarNodes {
        root,
        pill,
        apple_btn,
        app_btn,
        menu_items,
        control_center_btn,
        clock_btn,
        popover: popover_nodes,
    })
}
