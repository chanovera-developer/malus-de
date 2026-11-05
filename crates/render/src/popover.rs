// ==============================================================================
// Malus DE: Menús Desplegables y Popovers Superelípticos G2 (WGPU + Taffy)
// ==============================================================================
// Implementa ventanas emergentes flotantes con curvatura superelíptica continua (n ≈ 4.2),
// sombras difusas multicapa por GPU, resaltado reactivo al pasar el cursor (hover)
// y compatibilidad directa con el menú del sistema Apple y categorías D-Bus Menu.

use crate::dbus_menu::DbusMenuCategory;
use crate::layout::{SquircleStyle, UiTree};
use crate::squircle::MACOS_SQUIRCLE_POWER;
use crate::text::{TextLabel, Weight};
use anyhow::Result;
use taffy::prelude::*;

/// Representación de un ítem individual dentro de un menú desplegable.
#[derive(Clone, Debug, PartialEq)]
pub struct PopoverItem {
    pub id: i32,
    pub label: String,
    pub shortcut: Option<String>,
    pub is_separator: bool,
    pub enabled: bool,
    pub icon: Option<String>,
}

impl PopoverItem {
    /// Crea un ítem de menú interactivo estándar con una etiqueta.
    pub fn new(id: i32, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            shortcut: None,
            is_separator: false,
            enabled: true,
            icon: None,
        }
    }

    /// Crea un separador horizontal divisorio.
    pub fn separator() -> Self {
        Self {
            id: -1,
            label: String::new(),
            shortcut: None,
            is_separator: true,
            enabled: false,
            icon: None,
        }
    }

    /// Asigna una combinación de teclas o atajo visible (ej. "⌘,", "⌥⌘⎋", "Ctrl+S").
    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// Configura si el ítem está activo o deshabilitado (grisado).
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Asigna un icono decorativo o glifo opcional.
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}

/// Parámetros visuales y de dimensionamiento para el popover.
#[derive(Clone, Debug)]
pub struct PopoverConfig {
    pub width: f32,
    pub corner_radius: f32,
    pub squircle_power: f32,
    pub bg_color: [f32; 4],
    pub border_color: [f32; 4],
    pub border_width: f32,
    pub item_height: f32,
    pub item_padding_x: f32,
    pub padding_y: f32,
    pub padding_x: f32,
    pub active_bg_color: [f32; 4],
    pub active_corner_radius: f32,
    pub shadow: bool,
}

impl Default for PopoverConfig {
    fn default() -> Self {
        Self {
            width: 240.0,
            corner_radius: 10.0,
            squircle_power: MACOS_SQUIRCLE_POWER, // n ≈ 4.2
            bg_color: [0.14, 0.14, 0.17, 0.94],   // Fondo translúcido profundo macOS
            border_color: [1.0, 1.0, 1.0, 0.16],  // Borde reflectante sutil
            border_width: 1.0,
            item_height: 24.0,
            item_padding_x: 8.0,
            padding_y: 6.0,
            padding_x: 6.0,
            active_bg_color: [0.0, 0.48, 1.0, 0.95], // Acento azul macOS activo
            active_corner_radius: 5.0,
            shadow: true,
        }
    }
}

/// Modelo declarativo de datos para un menú desplegable activo.
#[derive(Clone, Debug)]
pub struct PopoverMenuModel {
    pub menu_id: String,
    pub title: Option<String>,
    pub anchor_x: f32,
    pub anchor_y: f32,
    pub items: Vec<PopoverItem>,
    pub hovered_index: Option<usize>,
}

impl PopoverMenuModel {
    /// Menú principal de Malus DE / Apple con opciones del sistema.
    pub fn apple_menu(anchor_x: f32, anchor_y: f32) -> Self {
        Self {
            menu_id: "apple".to_string(),
            title: Some("Apple".to_string()),
            anchor_x,
            anchor_y,
            items: vec![
                PopoverItem::new(1, "Acerca de Malus DE"),
                PopoverItem::separator(),
                PopoverItem::new(2, "Preferencias del Sistema...").with_shortcut("⌘,"),
                PopoverItem::new(3, "Forzar salida de aplicaciones...").with_shortcut("⌥⌘⎋"),
                PopoverItem::new(4, "Recargar Shell (Hot-Reload)").with_shortcut("⌥⌘R"),
                PopoverItem::new(5, "Bloquear Pantalla").with_shortcut("⌃⌘L"),
                PopoverItem::separator(),
                PopoverItem::new(6, "Reiniciar..."),
                PopoverItem::new(7, "Apagar el Equipo..."),
            ],
            hovered_index: None,
        }
    }

    /// Menú de prueba representativo ("Archivo").
    pub fn sample_file_menu(anchor_x: f32, anchor_y: f32) -> Self {
        Self {
            menu_id: "menu:Archivo".to_string(),
            title: Some("Archivo".to_string()),
            anchor_x,
            anchor_y,
            items: vec![
                PopoverItem::new(101, "Nueva Ventana").with_shortcut("⌘N"),
                PopoverItem::new(102, "Nueva Pestaña").with_shortcut("⌘T"),
                PopoverItem::new(103, "Abrir...").with_shortcut("⌘O"),
                PopoverItem::separator(),
                PopoverItem::new(104, "Guardar").with_shortcut("⌘S"),
                PopoverItem::new(105, "Guardar como...").with_shortcut("⇧⌘S"),
                PopoverItem::separator(),
                PopoverItem::new(106, "Cerrar Ventana").with_shortcut("⌘W"),
            ],
            hovered_index: None,
        }
    }

    /// Construye el modelo de popover a partir de una categoría obtenida vía D-Bus Menu.
    pub fn from_dbus_category(anchor_x: f32, anchor_y: f32, cat: &DbusMenuCategory) -> Self {
        let items = cat
            .items
            .iter()
            .map(|item| {
                if item.is_separator {
                    PopoverItem::separator()
                } else {
                    let mut p_item =
                        PopoverItem::new(item.id, &item.label).with_enabled(item.enabled);
                    if let Some(ref sc) = item.shortcut {
                        p_item = p_item.with_shortcut(sc);
                    }
                    p_item
                }
            })
            .collect();

        Self {
            menu_id: format!("menu:{}", cat.title),
            title: Some(cat.title.clone()),
            anchor_x,
            anchor_y,
            items,
            hovered_index: None,
        }
    }

    /// Calcula la altura total requerida por el popover.
    pub fn total_height(&self, config: &PopoverConfig) -> f32 {
        let mut height = config.padding_y * 2.0;
        for item in &self.items {
            if item.is_separator {
                height += 9.0; // 1px línea + 4px top + 4px bottom
            } else {
                height += config.item_height;
            }
        }
        height
    }
}

/// Identificadores de nodos creados en el árbol Taffy para el popover.
pub struct PopoverNodes {
    pub root_container: NodeId,
    pub card: NodeId,
    pub outer_shadow: Option<NodeId>,
    pub inner_shadow: Option<NodeId>,
    pub items: Vec<(usize, NodeId, PopoverItem)>,
}

impl PopoverNodes {
    /// Busca el ítem de popover correspondiente a un NodeId del árbol.
    pub fn find_item_by_node(&self, node: NodeId) -> Option<&PopoverItem> {
        self.items
            .iter()
            .find(|(_, item_node, _)| *item_node == node)
            .map(|(_, _, item)| item)
    }

    /// Obtiene el índice de posición en el menú para un NodeId dado.
    pub fn find_index_by_node(&self, node: NodeId) -> Option<usize> {
        self.items
            .iter()
            .find(|(_, item_node, _)| *item_node == node)
            .map(|(idx, _, _)| *idx)
    }
}

/// Construye el árbol de layout y estilo para un menú popover flotante.
pub fn build_popover_menu(
    ui: &mut UiTree,
    config: &PopoverConfig,
    model: &PopoverMenuModel,
) -> Result<PopoverNodes> {
    let total_height = model.total_height(config);

    // --------------------------------------------------------------------------
    // 1. Sombras difusas multicapa (Drop Shadow con Squircles WGPU)
    // --------------------------------------------------------------------------
    let outer_shadow = if config.shadow {
        let shadow_outer = ui.add_leaf(
            Style {
                position: Position::Absolute,
                inset: Rect {
                    left: LengthPercentageAuto::length(model.anchor_x - 6.0),
                    top: LengthPercentageAuto::length(model.anchor_y - 2.0),
                    right: LengthPercentageAuto::auto(),
                    bottom: LengthPercentageAuto::auto(),
                },
                size: Size {
                    width: Dimension::length(config.width + 12.0),
                    height: Dimension::length(total_height + 12.0),
                },
                ..Default::default()
            },
            SquircleStyle::card(config.corner_radius + 4.0, [0.0, 0.0, 0.0, 0.12])
                .with_power(config.squircle_power)
                .with_z(70.0),
            Some("popover_shadow_outer"),
        )?;
        Some(shadow_outer)
    } else {
        None
    };

    let inner_shadow = if config.shadow {
        let shadow_inner = ui.add_leaf(
            Style {
                position: Position::Absolute,
                inset: Rect {
                    left: LengthPercentageAuto::length(model.anchor_x - 3.0),
                    top: LengthPercentageAuto::length(model.anchor_y),
                    right: LengthPercentageAuto::auto(),
                    bottom: LengthPercentageAuto::auto(),
                },
                size: Size {
                    width: Dimension::length(config.width + 6.0),
                    height: Dimension::length(total_height + 6.0),
                },
                ..Default::default()
            },
            SquircleStyle::card(config.corner_radius + 2.0, [0.0, 0.0, 0.0, 0.20])
                .with_power(config.squircle_power)
                .with_z(75.0),
            Some("popover_shadow_inner"),
        )?;
        Some(shadow_inner)
    } else {
        None
    };

    // --------------------------------------------------------------------------
    // 2. Construcción de los ítems internos del menú
    // --------------------------------------------------------------------------
    let mut card_children = Vec::new();
    let mut item_node_pairs = Vec::new();

    for (idx, item) in model.items.iter().enumerate() {
        if item.is_separator {
            let sep_node = ui.add_leaf(
                Style {
                    size: Size {
                        width: Dimension::percent(1.0),
                        height: Dimension::length(1.0),
                    },
                    margin: Rect {
                        top: LengthPercentageAuto::length(4.0),
                        bottom: LengthPercentageAuto::length(4.0),
                        left: LengthPercentageAuto::length(2.0),
                        right: LengthPercentageAuto::length(2.0),
                    },
                    ..Default::default()
                },
                SquircleStyle::card(0.5, [1.0, 1.0, 1.0, 0.10]).with_z(85.0),
                Some(format!("popover_sep:{}", idx)),
            )?;
            card_children.push(sep_node);
        } else {
            let is_hovered = model.hovered_index == Some(idx) && item.enabled;

            // Hijos del contenedor de fila (etiqueta a la izquierda + atajo a la derecha)
            let mut row_children = Vec::new();

            // Etiqueta del ítem
            let label_node = ui.add_leaf(
                Style {
                    flex_grow: 1.0,
                    size: Size {
                        width: Dimension::auto(),
                        height: Dimension::percent(1.0),
                    },
                    ..Default::default()
                },
                SquircleStyle::transparent(),
                Some(format!("popover_label:{}", idx)),
            )?;

            let label_color = if is_hovered {
                [1.0, 1.0, 1.0, 1.0]
            } else if item.enabled {
                [0.92, 0.92, 0.94, 1.0]
            } else {
                [0.48, 0.48, 0.52, 0.60]
            };

            let label_weight = if is_hovered {
                Weight::MEDIUM
            } else {
                Weight::NORMAL
            };

            ui.set_text(
                label_node,
                TextLabel::new(&item.label)
                    .left_aligned()
                    .with_color(label_color)
                    .with_weight(label_weight)
                    .with_size(13.0),
            );
            row_children.push(label_node);

            // Atajo de teclado opcional alineado a la derecha
            if let Some(ref shortcut) = item.shortcut {
                let shortcut_node = ui.add_leaf(
                    Style {
                        size: Size {
                            width: Dimension::length(55.0),
                            height: Dimension::percent(1.0),
                        },
                        ..Default::default()
                    },
                    SquircleStyle::transparent(),
                    Some(format!("popover_shortcut:{}", idx)),
                )?;

                let sc_color = if is_hovered {
                    [1.0, 1.0, 1.0, 0.85]
                } else if item.enabled {
                    [0.62, 0.62, 0.70, 0.80]
                } else {
                    [0.40, 0.40, 0.45, 0.45]
                };

                ui.set_text(
                    shortcut_node,
                    TextLabel::new(shortcut)
                        .right_aligned()
                        .with_color(sc_color)
                        .with_size(12.0),
                );
                row_children.push(shortcut_node);
            }

            // Contenedor interactivo del ítem
            let item_visual_style = if is_hovered {
                SquircleStyle::card(config.active_corner_radius, config.active_bg_color)
                    .with_power(config.squircle_power)
                    .with_border(0.5, [1.0, 1.0, 1.0, 0.25])
                    .with_z(86.0)
            } else {
                // Nodo con fondo transparente visible para permitir hit-testing analítico
                SquircleStyle::card(config.active_corner_radius, [0.0, 0.0, 0.0, 0.002])
                    .with_power(config.squircle_power)
                    .with_z(86.0)
            };

            let item_row = ui.add_container(
                Style {
                    size: Size {
                        width: Dimension::percent(1.0),
                        height: Dimension::length(config.item_height),
                    },
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::CENTER,
                    padding: Rect {
                        left: LengthPercentage::length(config.item_padding_x),
                        right: LengthPercentage::length(config.item_padding_x),
                        top: LengthPercentage::length(0.0),
                        bottom: LengthPercentage::length(0.0),
                    },
                    ..Default::default()
                },
                item_visual_style,
                &row_children,
                Some(format!("popover_item:{}:{}", idx, item.id)),
            )?;

            card_children.push(item_row);
            item_node_pairs.push((idx, item_row, item.clone()));
        }
    }

    // --------------------------------------------------------------------------
    // 3. Tarjeta contenedora principal (Squircle con G2)
    // --------------------------------------------------------------------------
    let card = ui.add_container(
        Style {
            position: Position::Absolute,
            inset: Rect {
                left: LengthPercentageAuto::length(model.anchor_x),
                top: LengthPercentageAuto::length(model.anchor_y),
                right: LengthPercentageAuto::auto(),
                bottom: LengthPercentageAuto::auto(),
            },
            size: Size {
                width: Dimension::length(config.width),
                height: Dimension::length(total_height),
            },
            flex_direction: FlexDirection::Column,
            padding: Rect {
                top: LengthPercentage::length(config.padding_y),
                bottom: LengthPercentage::length(config.padding_y),
                left: LengthPercentage::length(config.padding_x),
                right: LengthPercentage::length(config.padding_x),
            },
            ..Default::default()
        },
        SquircleStyle::card(config.corner_radius, config.bg_color)
            .with_power(config.squircle_power)
            .with_border(config.border_width, config.border_color)
            .with_z(80.0),
        &card_children,
        Some("popover_card"),
    )?;

    // Agrupar sombras y tarjeta en el contenedor raíz
    let mut all_elements = Vec::new();
    if let Some(s_outer) = outer_shadow {
        all_elements.push(s_outer);
    }
    if let Some(s_inner) = inner_shadow {
        all_elements.push(s_inner);
    }
    all_elements.push(card);

    let root_container = ui.add_container(
        Style {
            position: Position::Absolute,
            inset: Rect {
                left: LengthPercentageAuto::length(0.0),
                top: LengthPercentageAuto::length(0.0),
                right: LengthPercentageAuto::length(0.0),
                bottom: LengthPercentageAuto::length(0.0),
            },
            size: Size {
                width: Dimension::percent(1.0),
                height: Dimension::percent(1.0),
            },
            ..Default::default()
        },
        SquircleStyle::transparent(),
        &all_elements,
        Some("popover_root_overlay"),
    )?;

    Ok(PopoverNodes {
        root_container,
        card,
        outer_shadow,
        inner_shadow,
        items: item_node_pairs,
    })
}
