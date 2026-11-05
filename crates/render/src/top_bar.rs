// ==============================================================================
// Malus DE: Constructor Declarativo de la Barra Superior (Top-Bar macOS)
// ==============================================================================

use crate::layout::{SquircleStyle, UiTree};
use crate::luminance::AdaptivePalette;
use crate::popover::{build_popover_menu, PopoverConfig, PopoverMenuModel, PopoverNodes};
use crate::text::{estimate_label_width, TextLabel, Weight};
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
            is_floating_pill: false, // Barra transparente continua por defecto
            margin_x: 0.0,
            margin_top: 0.0,
            bg_color: [0.0, 0.0, 0.0, 0.0], // 100% transparente para mostrar el fondo/wallpaper
            border_color: [0.0, 0.0, 0.0, 0.0],
            border_width: 0.0,
        }
    }
}

/// Elemento de estado o icono interactivo para la sección derecha de la barra superior.
#[derive(Clone, Debug, PartialEq)]
pub struct StatusItem {
    pub tag: String,
    pub text: String,
    pub font_size: f32,
    pub is_bold: bool,
}

impl StatusItem {
    pub fn new(tag: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            tag: tag.into(),
            text: text.into(),
            font_size: 13.0,
            is_bold: false,
        }
    }

    pub fn with_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    pub fn bold(mut self) -> Self {
        self.is_bold = true;
        self
    }
}

impl From<&str> for StatusItem {
    fn from(s: &str) -> Self {
        StatusItem::new(format!("status:{}", s), s)
    }
}

impl From<String> for StatusItem {
    fn from(s: String) -> Self {
        let tag = format!("status:{}", s);
        StatusItem::new(tag, s)
    }
}

/// Datos y estado del modelo del Menú Global y barra de estado.
#[derive(Clone, Debug)]
pub struct TopBarModel {
    pub app_name: String,
    pub menus: Vec<String>,
    pub status_items: Vec<StatusItem>,
    pub clock_text: String,
    pub is_dark_background: bool,
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
            status_items: vec![
                StatusItem::new("status:tray", "> +4").with_size(11.0).bold(),
                StatusItem::new("status:keyboard", "ES").with_size(11.0).bold(),
                StatusItem::new("status:wifi", "\u{F1EB}").with_size(13.0),
                StatusItem::new("status:volume", "\u{F028}").with_size(13.0),
                StatusItem::new("status:battery", "\u{F240}").with_size(13.0),
                StatusItem::new("status:spotlight", "\u{F002}").with_size(12.0),
            ],
            clock_text: "Mié 4 Nov 21:25".to_string(),
            is_dark_background: true,
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
    let palette = AdaptivePalette::new(model.is_dark_background);

    // Determinar qué menú o botón se encuentra activo en popover
    let active_menu_id = model
        .active_popover
        .as_ref()
        .map(|p| p.menu_id.as_str())
        .unwrap_or("");

    // Botón Logo Apple (resaltado si su popover está desplegado)
    let apple_style = if active_menu_id == "apple" {
        SquircleStyle::card(14.0, palette.topbar_active_bg).with_power(2.0)
    } else {
        SquircleStyle::button(palette.button_idle_bg)
    };

    let apple_w = (estimate_label_width("\u{F179}", 15.0, Weight::NORMAL) + 16.0).max(30.0);
    let apple_btn = ui.add_leaf(
        Style {
            size: Size {
                width: Dimension::length(apple_w),
                height: Dimension::length(24.0),
            },
            ..Default::default()
        },
        apple_style,
        Some("apple_menu"),
    )?;
    ui.set_text(
        apple_btn,
        TextLabel::new("\u{F179}")
            .with_size(15.0)
            .with_color(palette.text_primary),
    );
    left_children.push(apple_btn);

    // Botón Nombre de Aplicación Activa (Acento azul macOS / píldora activa)
    let is_app_active = active_menu_id == format!("app:{}", model.app_name) || active_menu_id == format!("menu:{}", model.app_name);
    let app_style = if is_app_active {
        SquircleStyle::card(14.0, palette.topbar_active_bg).with_power(2.0)
    } else {
        SquircleStyle::button(palette.button_idle_bg)
    };

    let app_w = (estimate_label_width(&model.app_name, 13.0, Weight::BOLD) + 16.0).max(36.0);
    let app_btn = ui.add_leaf(
        Style {
            size: Size {
                width: Dimension::length(app_w),
                height: Dimension::length(24.0),
            },
            ..Default::default()
        },
        app_style,
        Some(format!("app:{}", model.app_name)),
    )?;
    ui.set_text(
        app_btn,
        TextLabel::new(&model.app_name)
            .with_weight(Weight::BOLD)
            .with_color(palette.text_primary)
            .with_size(13.0),
    );
    left_children.push(app_btn);

    // Filtrar menús para evitar duplicar el nombre de la app activa en la barra (estilo macOS)
    let filtered_menus: Vec<String> = model
        .menus
        .iter()
        .filter(|m| !m.eq_ignore_ascii_case(&model.app_name))
        .cloned()
        .collect();

    // Elementos del menú activo de la aplicación
    let mut menu_items = Vec::new();
    for menu in &filtered_menus {
        let menu_id_tag = format!("menu:{}", menu);
        let is_menu_active = active_menu_id == menu_id_tag;

        let menu_style = if is_menu_active {
            SquircleStyle::card(14.0, palette.topbar_active_bg).with_power(2.0)
        } else {
            SquircleStyle::button(palette.button_idle_bg)
        };

        let menu_w = (estimate_label_width(menu, 13.0, Weight::NORMAL) + 16.0).max(36.0);
        let menu_node = ui.add_leaf(
            Style {
                size: Size {
                    width: Dimension::length(menu_w),
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
                .with_color(palette.text_primary)
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
        let weight = if item.is_bold { Weight::BOLD } else { Weight::NORMAL };
        let status_w = (estimate_label_width(&item.text, item.font_size, weight) + 16.0).max(28.0);
        let is_active = active_menu_id == item.tag;
        let item_style = if is_active {
            SquircleStyle::card(14.0, palette.topbar_active_bg).with_power(2.0)
        } else {
            SquircleStyle::button(palette.button_idle_bg)
        };

        let item_node = ui.add_leaf(
            Style {
                size: Size {
                    width: Dimension::length(status_w),
                    height: Dimension::length(24.0),
                },
                ..Default::default()
            },
            item_style,
            Some(item.tag.clone()),
        )?;
        ui.set_text(
            item_node,
            TextLabel::new(&item.text)
                .with_size(item.font_size)
                .with_weight(weight)
                .with_color(palette.text_primary),
        );
        right_children.push(item_node);
    }

    // Botón Centro de Control (Pill compacta con icono \u{F1DE})
    let is_cc_active = active_menu_id == "control_center";
    let cc_style = if is_cc_active {
        SquircleStyle::card(14.0, palette.topbar_active_bg).with_power(2.0)
    } else {
        SquircleStyle::button(palette.button_idle_bg)
    };
    let cc_w = (estimate_label_width("\u{F1DE}", 13.0, Weight::NORMAL) + 16.0).max(28.0);
    let control_center_btn = ui.add_leaf(
        Style {
            size: Size {
                width: Dimension::length(cc_w),
                height: Dimension::length(24.0),
            },
            ..Default::default()
        },
        cc_style,
        Some("control_center"),
    )?;
    ui.set_text(
        control_center_btn,
        TextLabel::new("\u{F1DE}")
            .with_color(palette.text_primary)
            .with_size(13.0),
    );
    right_children.push(control_center_btn);

    // Botón Reloj
    let is_clock_active = active_menu_id == "clock";
    let clock_style = if is_clock_active {
        SquircleStyle::card(14.0, palette.topbar_active_bg).with_power(2.0)
    } else {
        SquircleStyle::button(palette.button_idle_bg)
    };
    let clock_w = (estimate_label_width(&model.clock_text, 12.5, Weight::SEMIBOLD) + 16.0).max(48.0);
    let clock_btn = ui.add_leaf(
        Style {
            size: Size {
                width: Dimension::length(clock_w),
                height: Dimension::length(24.0),
            },
            ..Default::default()
        },
        clock_style,
        Some("clock"),
    )?;
    ui.set_text(
        clock_btn,
        TextLabel::new(&model.clock_text)
            .with_weight(Weight::SEMIBOLD)
            .with_color(palette.text_primary)
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
        let popover_cfg = PopoverConfig::for_palette(&palette);
        build_popover_menu(ui, &popover_cfg, popover_model).ok()
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
