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
    pub text_color: [f32; 4],
    pub text_dimmed_color: [f32; 4],
    pub separator_color: [f32; 4],
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
            width: 265.0,                         // Ancho generoso con mayor margen lateral (~265px)
            corner_radius: 24.0,                 // Curvatura idéntica al dock de Malus DE (24px)
            squircle_power: MACOS_SQUIRCLE_POWER, // n ≈ 4.2
            bg_color: [0.93, 0.95, 0.98, 0.35],   // Vidrio claro translúcido al 35% de opacidad
            border_color: [0.0, 0.0, 0.0, 0.22],  // Línea exterior oscura semitransparente
            border_width: 2.0,                   // Ancho 2.0 activa el doble borde (1px oscuro + 1px claro)
            text_color: [0.11, 0.11, 0.13, 1.0],  // Grafito oscuro nítido (#1D1D1F)
            text_dimmed_color: [0.43, 0.43, 0.46, 0.80], // Atajos y subtítulos (#6E6E73)
            separator_color: [0.0, 0.0, 0.0, 0.08], // Línea divisoria fina sutil
            item_height: 32.0,                   // Altura cómoda estilo macOS Sonoma/Tahoe
            item_padding_x: 14.0,                // Mayor padding a la izquierda y derecha dentro del ítem
            padding_y: 6.0,
            padding_x: 7.0,                      // Mayor padding horizontal de la tarjeta del menú
            active_bg_color: [0.0, 0.48, 1.0, 0.70], // Acento azul macOS activo semitransparente (#007AFF al 70%)
            active_corner_radius: 5.5,           // Border-radius suave exacto para el hover (menu-bar.jpg)
            shadow: true,                        // Sombra exterior difuminada suave y sutil
        }
    }
}

impl PopoverConfig {
    /// Genera la configuración de popover adaptada a la paleta de contraste del fondo.
    pub fn for_palette(palette: &crate::luminance::AdaptivePalette) -> Self {
        Self {
            bg_color: palette.popover_bg,
            border_color: palette.popover_border,
            border_width: 2.0,
            text_color: palette.popover_text,
            text_dimmed_color: palette.popover_text_dimmed,
            separator_color: palette.popover_separator,
            active_bg_color: palette.button_active_bg,
            shadow: true,
            ..Default::default()
        }
    }
}

/// Estado y configuración del panel modular de Centro de Control (estilo macOS Tahoe / Sonoma).
#[derive(Clone, Debug, PartialEq)]
pub struct ControlCenterModel {
    pub wifi_enabled: bool,
    pub wifi_ssid: String,
    pub bluetooth_enabled: bool,
    pub airdrop_enabled: bool,
    pub media_title: String,
    pub media_artist: String,
    pub media_playing: bool,
    pub focus_enabled: bool,
    pub display_brightness: u32,
    pub sound_volume: u32,
    pub sound_muted: bool,
    pub dark_mode: bool,
}

impl Default for ControlCenterModel {
    fn default() -> Self {
        Self {
            wifi_enabled: true,
            wifi_ssid: "BT-J6A5PF".to_string(),
            bluetooth_enabled: true,
            airdrop_enabled: true,
            media_title: "Useless Desires".to_string(),
            media_artist: "Patty Griffin – Impo...".to_string(),
            media_playing: true,
            focus_enabled: false,
            display_brightness: 85,
            sound_volume: 62,
            sound_muted: false,
            dark_mode: true,
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
    pub control_center: Option<ControlCenterModel>,
}

impl PopoverMenuModel {
    /// Menú principal de Malus DE / Apple con opciones del sistema exactas de la captura de macOS.
    pub fn apple_menu(anchor_x: f32, anchor_y: f32) -> Self {
        Self {
            menu_id: "apple".to_string(),
            title: Some("Apple".to_string()),
            anchor_x,
            anchor_y,
            items: vec![
                PopoverItem::new(1, "Acerca de este Mac").with_icon("\u{F108}"),
                PopoverItem::separator(),
                PopoverItem::new(2, "Ajustes del Sistema...")
                    .with_shortcut("⌘,")
                    .with_icon("\u{F013}"),
                PopoverItem::new(10, "App Store...")
                    .with_icon("\u{F290}"),
                PopoverItem::new(11, "Ítems recientes")
                    .with_shortcut(">")
                    .with_icon("\u{F017}"),
                PopoverItem::separator(),
                PopoverItem::new(3, "Forzar salida...")
                    .with_shortcut("⌥⌘⎋")
                    .with_icon("\u{F05E}"),
                PopoverItem::separator(),
                PopoverItem::new(12, "Reposo")
                    .with_icon("\u{F186}"),
                PopoverItem::new(6, "Reiniciar...")
                    .with_icon("\u{F01E}"),
                PopoverItem::new(7, "Apagar el Equipo...")
                    .with_icon("\u{F011}"),
                PopoverItem::separator(),
                PopoverItem::new(5, "Bloquear Pantalla")
                    .with_shortcut("⌃⌘Q")
                    .with_icon("\u{F023}"),
                PopoverItem::new(13, "Cerrar sesión cristian...")
                    .with_shortcut("⇧⌘Q")
                    .with_icon("\u{F2F5}"),
            ],
            hovered_index: None,
            control_center: None,
        }
    }

    /// Popover de la Bandeja del Sistema flotante (estilo Bartender / Hidden by the notch).
    pub fn tray_popover(anchor_x: f32, anchor_y: f32, apps: &[(&str, &str)]) -> Self {
        let mut items = vec![
            PopoverItem::new(-10, "Bandeja del sistema")
                .with_enabled(false),
            PopoverItem::separator(),
        ];
        if apps.is_empty() {
            let defaults = [
                ("Screen Studio", "\u{F03D}"),
                ("ChatGPT", "\u{F075}"),
                ("Docker Desktop", "\u{F24D}"),
                ("Wispr Flow", "\u{F130}"),
            ];
            for (i, (name, icon)) in defaults.iter().enumerate() {
                items.push(PopoverItem::new(300 + i as i32, *name).with_icon(*icon));
            }
        } else {
            for (i, (name, icon)) in apps.iter().enumerate() {
                items.push(PopoverItem::new(300 + i as i32, *name).with_icon(*icon));
            }
        }
        items.push(PopoverItem::separator());
        items.push(PopoverItem::new(399, "Gestionar aplicaciones...").with_icon("\u{F013}"));

        Self {
            menu_id: "status:tray".to_string(),
            title: Some("Bandeja del sistema".to_string()),
            anchor_x,
            anchor_y,
            items,
            hovered_index: None,
            control_center: None,
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
                PopoverItem::new(101, "Nueva Ventana").with_shortcut("⌘N").with_icon("\u{F044}"),
                PopoverItem::separator(),
                PopoverItem::new(102, "Cerrar Ventana").with_shortcut("⌘W").with_icon("\u{F00D}"),
                PopoverItem::new(103, "Abrir en Nueva Ventana").with_icon("\u{F2D0}"),
                PopoverItem::separator(),
                PopoverItem::new(104, "Guardar").with_shortcut("⌘S").with_icon("\u{F0C7}"),
                PopoverItem::new(105, "Guardar como...").with_shortcut("⇧⌘S"),
                PopoverItem::separator(),
                PopoverItem::new(106, "Imprimir...").with_shortcut("⌘P").with_icon("\u{F02F}").with_enabled(false),
            ],
            hovered_index: None,
            control_center: None,
        }
    }

    /// Popover de estado y gestión de Red Wi-Fi.
    pub fn wifi_popover(anchor_x: f32, anchor_y: f32, ssid: &str, signal: u8) -> Self {
        let is_connected = !ssid.is_empty();
        let status_text = if is_connected {
            format!("Wi-Fi: {}", ssid)
        } else {
            "Wi-Fi: Desconectado".to_string()
        };

        Self {
            menu_id: "status:wifi".to_string(),
            title: Some("Wi-Fi".to_string()),
            anchor_x,
            anchor_y,
            items: vec![
                PopoverItem::new(200, status_text)
                    .with_icon("\u{F1EB}")
                    .with_shortcut(if is_connected { format!("{}%", signal) } else { "".to_string() }),
                PopoverItem::separator(),
                PopoverItem::new(201, "Activar / Desactivar Wi-Fi").with_icon("\u{F011}"),
                PopoverItem::separator(),
                PopoverItem::new(202, "Preferencias de Red...")
                    .with_shortcut("⌘,")
                    .with_icon("\u{F013}"),
            ],
            hovered_index: None,
            control_center: None,
        }
    }

    /// Popover de ajuste de Sonido / Salida de Audio.
    pub fn volume_popover(anchor_x: f32, anchor_y: f32, volume: u32, muted: bool) -> Self {
        let status = if muted { "Silenciado" } else { "Altavoces integrados" };
        Self {
            menu_id: "status:volume".to_string(),
            title: Some("Sonido".to_string()),
            anchor_x,
            anchor_y,
            items: vec![
                PopoverItem::new(210, format!("Salida: {}", status)).with_icon("\u{F028}"),
                PopoverItem::separator(),
                PopoverItem::new(211, format!("Volumen: {}%", volume)).with_icon("\u{F027}"),
                PopoverItem::new(212, if muted { "Reactivar Sonido" } else { "Silenciar Volumen" })
                    .with_icon("\u{F6A9}"),
                PopoverItem::separator(),
                PopoverItem::new(213, "Preferencias de Sonido...").with_icon("\u{F013}"),
            ],
            hovered_index: None,
            control_center: None,
        }
    }

    /// Popover de información de Batería y Periféricos.
    pub fn battery_popover(anchor_x: f32, anchor_y: f32, pct: u32, is_charging: bool) -> Self {
        let pwr = if is_charging { "Cargando (Adaptador)" } else { "Alimentación por Batería" };
        Self {
            menu_id: "status:battery".to_string(),
            title: Some("Batería".to_string()),
            anchor_x,
            anchor_y,
            items: vec![
                PopoverItem::new(220, format!("Fuente: {}", pwr)).with_icon("\u{F0E7}"),
                PopoverItem::separator(),
                PopoverItem::new(221, format!("Nivel: {}%", pct)).with_icon("\u{F240}"),
                PopoverItem::new(222, "Dispositivo: Apple Mouse (13%)").with_icon("\u{F87C}"),
                PopoverItem::separator(),
                PopoverItem::new(223, "Preferencias de Batería...").with_icon("\u{F013}"),
            ],
            hovered_index: None,
            control_center: None,
        }
    }

    /// Popover del Centro de Control modular estilo macOS Tahoe / Sonoma con tarjetas y sliders.
    pub fn control_center_popover(anchor_x: f32, anchor_y: f32) -> Self {
        Self {
            menu_id: "control_center".to_string(),
            title: Some("Centro de Control".to_string()),
            anchor_x,
            anchor_y,
            items: Vec::new(),
            hovered_index: None,
            control_center: Some(ControlCenterModel::default()),
        }
    }

    /// Popover del Centro de Control a partir de un estado existente.
    pub fn control_center_panel(anchor_x: f32, anchor_y: f32, cc: ControlCenterModel) -> Self {
        Self {
            menu_id: "control_center".to_string(),
            title: Some("Centro de Control".to_string()),
            anchor_x,
            anchor_y,
            items: Vec::new(),
            hovered_index: None,
            control_center: Some(cc),
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
            control_center: None,
        }
    }

    /// Calcula la altura total requerida por el popover.
    pub fn total_height(&self, config: &PopoverConfig) -> f32 {
        if self.control_center.is_some() {
            return 420.0;
        }
        let mut height = config.padding_y * 2.0;
        for item in &self.items {
            if item.is_separator {
                height += 9.0;
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
    if let Some(ref cc) = model.control_center {
        return build_control_center_panel(ui, config, model, cc);
    }

    let popover_w = if model.menu_id == "status:tray" { 240.0 } else { config.width };
    let total_height = model.total_height(config);

    // --------------------------------------------------------------------------
    // 1. Sombra única difusa central analítica (falloff suave, concéntrica, sutil)
    // --------------------------------------------------------------------------
    let outer_shadow = if config.shadow {
        let blur_r = 8.0f32;
        let shadow_node = ui.add_leaf(
            Style {
                position: Position::Absolute,
                inset: Rect {
                    left: LengthPercentageAuto::length(model.anchor_x - blur_r),
                    top: LengthPercentageAuto::length(model.anchor_y - blur_r),
                    right: LengthPercentageAuto::auto(),
                    bottom: LengthPercentageAuto::auto(),
                },
                size: Size {
                    width: Dimension::length(popover_w + blur_r * 2.0),
                    height: Dimension::length(total_height + blur_r * 2.0),
                },
                ..Default::default()
            },
            SquircleStyle::card(config.corner_radius + blur_r, [0.0, 0.0, 0.0, 0.08])
                .with_power(config.squircle_power)
                .with_border(-blur_r, [0.0, 0.0, 0.0, 0.0])
                .with_z(70.0),
            Some("popover_shadow"),
        )?;
        Some(shadow_node)
    } else {
        None
    };

    let inner_shadow = None;

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
                        left: LengthPercentageAuto::length(6.0),
                        right: LengthPercentageAuto::length(6.0),
                    },
                    ..Default::default()
                },
                SquircleStyle::card(0.5, config.separator_color).with_z(85.0),
                Some(format!("popover_sep:{}", idx)),
            )?;
            card_children.push(sep_node);
        } else {
            let is_hovered = model.hovered_index == Some(idx) && item.enabled;

            // Hijos del contenedor de fila (icono opcional + etiqueta a la izquierda + atajo a la derecha)
            let mut row_children = Vec::new();

            // 1. Icono decorativo o glifo opcional alineado a la izquierda
            if let Some(ref icon_str) = item.icon {
                let icon_node = ui.add_leaf(
                    Style {
                        size: Size {
                            width: Dimension::length(20.0),
                            height: Dimension::percent(1.0),
                        },
                        margin: Rect {
                            left: LengthPercentageAuto::length(0.0),
                            right: LengthPercentageAuto::length(6.0),
                            top: LengthPercentageAuto::length(0.0),
                            bottom: LengthPercentageAuto::length(0.0),
                        },
                        ..Default::default()
                    },
                    SquircleStyle::transparent(),
                    Some(format!("popover_icon:{}", idx)),
                )?;

                let icon_color = if is_hovered {
                    [1.0, 1.0, 1.0, 1.0]
                } else if item.enabled {
                    [
                        config.text_color[0],
                        config.text_color[1],
                        config.text_color[2],
                        0.85,
                    ]
                } else {
                    [
                        config.text_color[0] * 0.5,
                        config.text_color[1] * 0.5,
                        config.text_color[2] * 0.5,
                        0.40,
                    ]
                };

                ui.set_text(
                    icon_node,
                    TextLabel::new(icon_str)
                        .centered()
                        .with_color(icon_color)
                        .with_weight(Weight::BLACK)
                        .with_size(12.5),
                );
                row_children.push(icon_node);
            }

            // 2. Etiqueta del ítem
            let label_node = ui.add_leaf(
                Style {
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
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
                config.text_color
            } else {
                [
                    config.text_color[0] * 0.5,
                    config.text_color[1] * 0.5,
                    config.text_color[2] * 0.5,
                    0.50,
                ]
            };

            let label_weight = Weight::NORMAL;

            ui.set_text(
                label_node,
                TextLabel::new(&item.label)
                    .left_aligned()
                    .with_color(label_color)
                    .with_weight(label_weight)
                    .with_size(13.0),
            );
            row_children.push(label_node);

            // 3. Atajo de teclado opcional alineado a la derecha
            if let Some(ref shortcut) = item.shortcut {
                let shortcut_node = ui.add_leaf(
                    Style {
                        flex_shrink: 0.0,
                        size: Size {
                            width: Dimension::auto(),
                            height: Dimension::percent(1.0),
                        },
                        min_size: Size {
                            width: LengthPercentageAuto::length(45.0),
                            height: LengthPercentageAuto::auto(),
                        },
                        ..Default::default()
                    },
                    SquircleStyle::transparent(),
                    Some(format!("popover_shortcut:{}", idx)),
                )?;

                let sc_color = if is_hovered {
                    [1.0, 1.0, 1.0, 0.85]
                } else if item.enabled {
                    config.text_dimmed_color
                } else {
                    [
                        config.text_dimmed_color[0] * 0.6,
                        config.text_dimmed_color[1] * 0.6,
                        config.text_dimmed_color[2] * 0.6,
                        0.40,
                    ]
                };

                ui.set_text(
                    shortcut_node,
                    TextLabel::new(shortcut)
                        .right_aligned()
                        .with_color(sc_color)
                        .with_weight(Weight::NORMAL)
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
                    justify_content: JustifyContent::SPACE_BETWEEN,
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
                width: Dimension::length(popover_w),
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

fn margin_right(val: f32) -> Rect<LengthPercentageAuto> {
    Rect {
        left: LengthPercentageAuto::length(0.0),
        right: LengthPercentageAuto::length(val),
        top: LengthPercentageAuto::length(0.0),
        bottom: LengthPercentageAuto::length(0.0),
    }
}

fn margin_left(val: f32) -> Rect<LengthPercentageAuto> {
    Rect {
        left: LengthPercentageAuto::length(val),
        right: LengthPercentageAuto::length(0.0),
        top: LengthPercentageAuto::length(0.0),
        bottom: LengthPercentageAuto::length(0.0),
    }
}

fn margin_bottom(val: f32) -> Rect<LengthPercentageAuto> {
    Rect {
        left: LengthPercentageAuto::length(0.0),
        right: LengthPercentageAuto::length(0.0),
        top: LengthPercentageAuto::length(0.0),
        bottom: LengthPercentageAuto::length(val),
    }
}

/// Construye el panel modular del Centro de Control estilo macOS Tahoe / Sonoma.
pub fn build_control_center_panel(
    ui: &mut UiTree,
    config: &PopoverConfig,
    model: &PopoverMenuModel,
    cc: &ControlCenterModel,
) -> Result<PopoverNodes> {
    let panel_w = 320.0f32;
    let panel_h = 420.0f32;

    // Colores para las tarjetas y controles interiores (estilo macOS Tahoe flotante)
    let is_dark = config.bg_color[0] < 0.5;
    let tile_bg = if is_dark {
        [0.20, 0.20, 0.25, 0.82]
    } else {
        [0.96, 0.96, 0.98, 0.85]
    };
    let tile_border = if is_dark {
        [1.0, 1.0, 1.0, 0.20]
    } else {
        [0.0, 0.0, 0.0, 0.12]
    };
    let active_blue = [0.0f32, 0.48, 1.0, 0.95];
    let inactive_circle = if is_dark {
        [1.0, 1.0, 1.0, 0.15]
    } else {
        [0.0, 0.0, 0.0, 0.10]
    };
    let slider_track_bg = if is_dark {
        [0.0, 0.0, 0.0, 0.35]
    } else {
        [0.0, 0.0, 0.0, 0.15]
    };
    let slider_fill_bg = if is_dark {
        [1.0f32, 1.0, 1.0, 0.95]
    } else {
        [0.10f32, 0.10, 0.14, 0.95]
    };

    let mut panel_children = Vec::new();

    // --------------------------------------------------------------------------
    // FILA 1: [Módulo Conectividad 143x130] + [Módulo Reproductor 143x130]
    // --------------------------------------------------------------------------
    let mut conn_items = Vec::new();

    // 1. Wi-Fi Row
    {
        let circle = ui.add_leaf(
            Style {
                size: Size { width: Dimension::length(28.0), height: Dimension::length(28.0) },
                margin: margin_right(8.0),
                ..Default::default()
            },
            SquircleStyle::card(14.0, if cc.wifi_enabled { active_blue } else { inactive_circle }).with_z(85.0),
            Some("cc:wifi_icon"),
        )?;
        ui.set_text(
            circle,
            TextLabel::new("\u{F1EB}")
                .centered()
                .with_size(12.0)
                .with_color(if cc.wifi_enabled { [1.0, 1.0, 1.0, 1.0] } else { config.text_color }),
        );

        let title = ui.add_leaf(
            Style { size: Size { width: Dimension::auto(), height: Dimension::length(14.0) }, ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:wifi_title"),
        )?;
        ui.set_text(title, TextLabel::new("Wi-Fi").left_aligned().with_size(11.5).with_weight(Weight::BOLD).with_color(config.text_color));

        let sub = ui.add_leaf(
            Style { size: Size { width: Dimension::auto(), height: Dimension::length(12.0) }, ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:wifi_sub"),
        )?;
        let ssid_text = if cc.wifi_enabled { &cc.wifi_ssid } else { "Desactivado" };
        ui.set_text(sub, TextLabel::new(ssid_text).left_aligned().with_size(9.5).with_color(config.text_dimmed_color));

        let label_col = ui.add_container(
            Style { flex_direction: FlexDirection::Column, flex_grow: 1.0, ..Default::default() },
            SquircleStyle::transparent(),
            &[title, sub],
            Some("cc:wifi_text_col"),
        )?;

        let wifi_row = ui.add_container(
            Style {
                size: Size { width: Dimension::percent(1.0), height: Dimension::length(34.0) },
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                ..Default::default()
            },
            SquircleStyle::card(8.0, [0.0, 0.0, 0.0, 0.002]).with_z(84.0),
            &[circle, label_col],
            Some("cc:wifi"),
        )?;
        conn_items.push(wifi_row);
    }

    // 2. Bluetooth Row
    {
        let circle = ui.add_leaf(
            Style {
                size: Size { width: Dimension::length(28.0), height: Dimension::length(28.0) },
                margin: margin_right(8.0),
                ..Default::default()
            },
            SquircleStyle::card(14.0, if cc.bluetooth_enabled { [1.0, 1.0, 1.0, 0.95] } else { inactive_circle }).with_z(85.0),
            Some("cc:bluetooth_icon"),
        )?;
        ui.set_text(
            circle,
            TextLabel::new("\u{F293}")
                .centered()
                .with_size(12.0)
                .with_color(if cc.bluetooth_enabled { active_blue } else { config.text_color }),
        );

        let title = ui.add_leaf(
            Style { size: Size { width: Dimension::auto(), height: Dimension::length(14.0) }, ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:bt_title"),
        )?;
        ui.set_text(title, TextLabel::new("Bluetooth").left_aligned().with_size(11.5).with_weight(Weight::BOLD).with_color(config.text_color));

        let sub = ui.add_leaf(
            Style { size: Size { width: Dimension::auto(), height: Dimension::length(12.0) }, ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:bt_sub"),
        )?;
        let bt_text = if cc.bluetooth_enabled { "Activado" } else { "Desactivado" };
        ui.set_text(sub, TextLabel::new(bt_text).left_aligned().with_size(9.5).with_color(config.text_dimmed_color));

        let label_col = ui.add_container(
            Style { flex_direction: FlexDirection::Column, flex_grow: 1.0, ..Default::default() },
            SquircleStyle::transparent(),
            &[title, sub],
            Some("cc:bt_text_col"),
        )?;

        let bt_row = ui.add_container(
            Style {
                size: Size { width: Dimension::percent(1.0), height: Dimension::length(34.0) },
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                ..Default::default()
            },
            SquircleStyle::card(8.0, [0.0, 0.0, 0.0, 0.002]).with_z(84.0),
            &[circle, label_col],
            Some("cc:bluetooth"),
        )?;
        conn_items.push(bt_row);
    }

    // 3. AirDrop / Malus Link Row
    {
        let circle = ui.add_leaf(
            Style {
                size: Size { width: Dimension::length(28.0), height: Dimension::length(28.0) },
                margin: margin_right(8.0),
                ..Default::default()
            },
            SquircleStyle::card(14.0, if cc.airdrop_enabled { [1.0, 1.0, 1.0, 0.95] } else { inactive_circle }).with_z(85.0),
            Some("cc:airdrop_icon"),
        )?;
        ui.set_text(
            circle,
            TextLabel::new("\u{F0C1}")
                .centered()
                .with_size(11.0)
                .with_color(if cc.airdrop_enabled { active_blue } else { config.text_color }),
        );

        let title = ui.add_leaf(
            Style { size: Size { width: Dimension::auto(), height: Dimension::length(14.0) }, ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:airdrop_title"),
        )?;
        ui.set_text(title, TextLabel::new("Malus Link").left_aligned().with_size(11.5).with_weight(Weight::BOLD).with_color(config.text_color));

        let sub = ui.add_leaf(
            Style { size: Size { width: Dimension::auto(), height: Dimension::length(12.0) }, ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:airdrop_sub"),
        )?;
        let ad_text = if cc.airdrop_enabled { "Todos" } else { "Inactivo" };
        ui.set_text(sub, TextLabel::new(ad_text).left_aligned().with_size(9.5).with_color(config.text_dimmed_color));

        let label_col = ui.add_container(
            Style { flex_direction: FlexDirection::Column, flex_grow: 1.0, ..Default::default() },
            SquircleStyle::transparent(),
            &[title, sub],
            Some("cc:airdrop_text_col"),
        )?;

        let airdrop_row = ui.add_container(
            Style {
                size: Size { width: Dimension::percent(1.0), height: Dimension::length(34.0) },
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                ..Default::default()
            },
            SquircleStyle::card(8.0, [0.0, 0.0, 0.0, 0.002]).with_z(84.0),
            &[circle, label_col],
            Some("cc:airdrop"),
        )?;
        conn_items.push(airdrop_row);
    }

    let conn_card = ui.add_container(
        Style {
            size: Size { width: Dimension::length(143.0), height: Dimension::length(130.0) },
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::SPACE_AROUND,
            padding: Rect { top: LengthPercentage::length(6.0), bottom: LengthPercentage::length(6.0), left: LengthPercentage::length(8.0), right: LengthPercentage::length(8.0) },
            ..Default::default()
        },
        SquircleStyle::card(14.0, tile_bg).with_border(0.5, tile_border).with_power(config.squircle_power).with_z(82.0),
        &conn_items,
        Some("cc_card:connectivity"),
    )?;

    // Módulo Reproductor de Medios
    let mut media_children = Vec::new();
    {
        let cover = ui.add_leaf(
            Style {
                size: Size { width: Dimension::length(36.0), height: Dimension::length(36.0) },
                margin: margin_right(8.0),
                ..Default::default()
            },
            SquircleStyle::card(8.0, [0.85, 0.45, 0.35, 0.90]).with_z(84.0),
            Some("cc:media_cover"),
        )?;
        ui.set_text(cover, TextLabel::new("\u{F001}").centered().with_size(14.0).with_color([1.0, 1.0, 1.0, 0.95]));

        let m_title = ui.add_leaf(
            Style { size: Size { width: Dimension::length(80.0), height: Dimension::length(14.0) }, ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:media_title"),
        )?;
        ui.set_text(m_title, TextLabel::new(&cc.media_title).left_aligned().with_size(11.0).with_weight(Weight::BOLD).with_color(config.text_color));

        let m_artist = ui.add_leaf(
            Style { size: Size { width: Dimension::length(80.0), height: Dimension::length(12.0) }, ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:media_artist"),
        )?;
        ui.set_text(m_artist, TextLabel::new(&cc.media_artist).left_aligned().with_size(9.5).with_color(config.text_dimmed_color));

        let m_text_col = ui.add_container(
            Style { flex_direction: FlexDirection::Column, flex_grow: 1.0, ..Default::default() },
            SquircleStyle::transparent(),
            &[m_title, m_artist],
            Some("cc:media_text_col"),
        )?;

        let m_info_row = ui.add_container(
            Style {
                size: Size { width: Dimension::percent(1.0), height: Dimension::length(44.0) },
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                ..Default::default()
            },
            SquircleStyle::transparent(),
            &[cover, m_text_col],
            Some("cc:media_info"),
        )?;
        media_children.push(m_info_row);

        // Botones de reproducción (Prev, Play/Pause, Next)
        let btn_prev = ui.add_leaf(
            Style { size: Size { width: Dimension::length(30.0), height: Dimension::length(28.0) }, ..Default::default() },
            SquircleStyle::card(6.0, [0.0, 0.0, 0.0, 0.002]).with_z(84.0),
            Some("cc:media_prev"),
        )?;
        ui.set_text(btn_prev, TextLabel::new("\u{F048}").centered().with_size(12.0).with_color(config.text_color));

        let btn_play = ui.add_leaf(
            Style { size: Size { width: Dimension::length(34.0), height: Dimension::length(28.0) }, ..Default::default() },
            SquircleStyle::card(6.0, [0.0, 0.0, 0.0, 0.002]).with_z(84.0),
            Some("cc:media_play"),
        )?;
        let play_icon = if cc.media_playing { "\u{F04C}" } else { "\u{F04B}" };
        ui.set_text(btn_play, TextLabel::new(play_icon).centered().with_size(14.0).with_color(config.text_color));

        let btn_next = ui.add_leaf(
            Style { size: Size { width: Dimension::length(30.0), height: Dimension::length(28.0) }, ..Default::default() },
            SquircleStyle::card(6.0, [0.0, 0.0, 0.0, 0.002]).with_z(84.0),
            Some("cc:media_next"),
        )?;
        ui.set_text(btn_next, TextLabel::new("\u{F051}").centered().with_size(12.0).with_color(config.text_color));

        let m_ctrl_row = ui.add_container(
            Style {
                size: Size { width: Dimension::percent(1.0), height: Dimension::length(32.0) },
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                justify_content: JustifyContent::SPACE_AROUND,
                ..Default::default()
            },
            SquircleStyle::transparent(),
            &[btn_prev, btn_play, btn_next],
            Some("cc:media_ctrls"),
        )?;
        media_children.push(m_ctrl_row);
    }

    let media_card = ui.add_container(
        Style {
            size: Size { width: Dimension::length(143.0), height: Dimension::length(130.0) },
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::SPACE_BETWEEN,
            padding: Rect { top: LengthPercentage::length(10.0), bottom: LengthPercentage::length(10.0), left: LengthPercentage::length(8.0), right: LengthPercentage::length(8.0) },
            ..Default::default()
        },
        SquircleStyle::card(14.0, tile_bg).with_border(0.5, tile_border).with_power(config.squircle_power).with_z(82.0),
        &media_children,
        Some("cc_card:media"),
    )?;

    let row1 = ui.add_container(
        Style {
            size: Size { width: Dimension::percent(1.0), height: Dimension::length(130.0) },
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SPACE_BETWEEN,
            ..Default::default()
        },
        SquircleStyle::transparent(),
        &[conn_card, media_card],
        Some("cc:row1"),
    )?;
    panel_children.push(row1);

    // --------------------------------------------------------------------------
    // FILA 2: [Concentración / Focus] + [Stage Manager / Duplicar] (44px)
    // --------------------------------------------------------------------------
    let focus_icon = ui.add_leaf(
        Style { size: Size { width: Dimension::length(24.0), height: Dimension::length(24.0) }, margin: margin_right(6.0), ..Default::default() },
        SquircleStyle::card(12.0, if cc.focus_enabled { active_blue } else { inactive_circle }).with_z(85.0),
        Some("cc:focus_icon"),
    )?;
    ui.set_text(focus_icon, TextLabel::new("\u{F186}").centered().with_size(11.0).with_color(if cc.focus_enabled { [1.0, 1.0, 1.0, 1.0] } else { config.text_color }));

    let focus_label = ui.add_leaf(
        Style { size: Size { width: Dimension::auto(), height: Dimension::length(16.0) }, ..Default::default() },
        SquircleStyle::transparent(),
        Some("cc:focus_label"),
    )?;
    ui.set_text(focus_label, TextLabel::new("Concentración").left_aligned().with_size(11.5).with_weight(Weight::BOLD).with_color(config.text_color));

    let focus_card = ui.add_container(
        Style {
            size: Size { width: Dimension::length(143.0), height: Dimension::length(44.0) },
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::CENTER,
            padding: Rect { left: LengthPercentage::length(10.0), right: LengthPercentage::length(10.0), top: LengthPercentage::length(0.0), bottom: LengthPercentage::length(0.0) },
            ..Default::default()
        },
        SquircleStyle::card(14.0, tile_bg).with_border(0.5, tile_border).with_power(config.squircle_power).with_z(82.0),
        &[focus_icon, focus_label],
        Some("cc:focus"),
    )?;

    let sm_btn = ui.add_leaf(
        Style { size: Size { width: Dimension::length(40.0), height: Dimension::length(30.0) }, ..Default::default() },
        SquircleStyle::card(8.0, inactive_circle).with_z(84.0),
        Some("cc:stage_manager"),
    )?;
    ui.set_text(sm_btn, TextLabel::new("\u{F009}").centered().with_size(12.0).with_color(config.text_color));

    let mirror_btn = ui.add_leaf(
        Style { size: Size { width: Dimension::length(40.0), height: Dimension::length(30.0) }, ..Default::default() },
        SquircleStyle::card(8.0, inactive_circle).with_z(84.0),
        Some("cc:mirror"),
    )?;
    ui.set_text(mirror_btn, TextLabel::new("\u{F26C}").centered().with_size(12.0).with_color(config.text_color));

    let modes_card = ui.add_container(
        Style {
            size: Size { width: Dimension::length(143.0), height: Dimension::length(44.0) },
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::CENTER,
            justify_content: JustifyContent::SPACE_AROUND,
            ..Default::default()
        },
        SquircleStyle::card(14.0, tile_bg).with_border(0.5, tile_border).with_power(config.squircle_power).with_z(82.0),
        &[sm_btn, mirror_btn],
        Some("cc:modes_card"),
    )?;

    let row2 = ui.add_container(
        Style {
            size: Size { width: Dimension::percent(1.0), height: Dimension::length(44.0) },
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SPACE_BETWEEN,
            ..Default::default()
        },
        SquircleStyle::transparent(),
        &[focus_card, modes_card],
        Some("cc:row2"),
    )?;
    panel_children.push(row2);

    // --------------------------------------------------------------------------
    // FILA 3: [Slider Pantalla / Brillo] (56px)
    // --------------------------------------------------------------------------
    {
        let disp_label = ui.add_leaf(
            Style { size: Size { width: Dimension::auto(), height: Dimension::length(14.0) }, margin: margin_bottom(4.0), ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:disp_label"),
        )?;
        ui.set_text(disp_label, TextLabel::new("Pantalla").left_aligned().with_size(11.5).with_weight(Weight::BOLD).with_color(config.text_color));

        let sun_low = ui.add_leaf(
            Style { size: Size { width: Dimension::length(18.0), height: Dimension::length(18.0) }, margin: margin_right(4.0), ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:sun_low"),
        )?;
        ui.set_text(sun_low, TextLabel::new("\u{F185}").centered().with_size(11.0).with_color(config.text_dimmed_color));

        let pct = (cc.display_brightness as f32 / 100.0).clamp(0.05, 1.0);
        let slider_fill = ui.add_leaf(
            Style { size: Size { width: Dimension::percent(pct), height: Dimension::percent(1.0) }, ..Default::default() },
            SquircleStyle::card(10.0, slider_fill_bg).with_z(85.0),
            Some("slider_fill:display"),
        )?;

        let slider_track = ui.add_container(
            Style {
                size: Size { width: Dimension::auto(), height: Dimension::length(20.0) },
                flex_grow: 1.0,
                flex_direction: FlexDirection::Row,
                ..Default::default()
            },
            SquircleStyle::card(10.0, slider_track_bg).with_z(84.0),
            &[slider_fill],
            Some("slider:display"),
        )?;

        let sun_high = ui.add_leaf(
            Style { size: Size { width: Dimension::length(18.0), height: Dimension::length(18.0) }, margin: margin_left(4.0), ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:sun_high"),
        )?;
        ui.set_text(sun_high, TextLabel::new("\u{F185}").centered().with_size(13.0).with_color(config.text_color));

        let slider_row = ui.add_container(
            Style {
                size: Size { width: Dimension::percent(1.0), height: Dimension::length(24.0) },
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                ..Default::default()
            },
            SquircleStyle::transparent(),
            &[sun_low, slider_track, sun_high],
            Some("slider_row:display"),
        )?;

        let disp_card = ui.add_container(
            Style {
                size: Size { width: Dimension::percent(1.0), height: Dimension::length(60.0) },
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::CENTER,
                padding: Rect { left: LengthPercentage::length(12.0), right: LengthPercentage::length(12.0), top: LengthPercentage::length(6.0), bottom: LengthPercentage::length(6.0) },
                ..Default::default()
            },
            SquircleStyle::card(16.0, tile_bg).with_border(0.5, tile_border).with_power(config.squircle_power).with_z(82.0),
            &[disp_label, slider_row],
            Some("slider_box:display"),
        )?;
        panel_children.push(disp_card);
    }

    // --------------------------------------------------------------------------
    // FILA 4: [Slider Sonido / Volumen] (60px)
    // --------------------------------------------------------------------------
    {
        let sound_label = ui.add_leaf(
            Style { size: Size { width: Dimension::auto(), height: Dimension::length(14.0) }, margin: margin_bottom(4.0), ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:sound_label"),
        )?;
        let sound_text = if cc.sound_muted { "Sonido (Silenciado)" } else { "Sonido" };
        ui.set_text(sound_label, TextLabel::new(sound_text).left_aligned().with_size(11.5).with_weight(Weight::BOLD).with_color(config.text_color));

        let vol_low = ui.add_leaf(
            Style { size: Size { width: Dimension::length(18.0), height: Dimension::length(18.0) }, margin: margin_right(4.0), ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:vol_low"),
        )?;
        ui.set_text(vol_low, TextLabel::new("\u{F027}").centered().with_size(11.0).with_color(config.text_dimmed_color));

        let vol_pct = if cc.sound_muted { 0.05 } else { (cc.sound_volume as f32 / 100.0).clamp(0.05, 1.0) };
        let vol_fill = ui.add_leaf(
            Style { size: Size { width: Dimension::percent(vol_pct), height: Dimension::percent(1.0) }, ..Default::default() },
            SquircleStyle::card(10.0, slider_fill_bg).with_z(85.0),
            Some("slider_fill:sound"),
        )?;

        let vol_track = ui.add_container(
            Style {
                size: Size { width: Dimension::auto(), height: Dimension::length(20.0) },
                flex_grow: 1.0,
                flex_direction: FlexDirection::Row,
                ..Default::default()
            },
            SquircleStyle::card(10.0, slider_track_bg).with_z(84.0),
            &[vol_fill],
            Some("slider:sound"),
        )?;

        let vol_high = ui.add_leaf(
            Style { size: Size { width: Dimension::length(18.0), height: Dimension::length(18.0) }, margin: margin_left(4.0), ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:vol_high"),
        )?;
        ui.set_text(vol_high, TextLabel::new("\u{F028}").centered().with_size(13.0).with_color(config.text_color));

        let output_btn = ui.add_leaf(
            Style { size: Size { width: Dimension::length(22.0), height: Dimension::length(20.0) }, margin: margin_left(6.0), ..Default::default() },
            SquircleStyle::card(6.0, inactive_circle).with_z(84.0),
            Some("cc:sound_output"),
        )?;
        ui.set_text(output_btn, TextLabel::new("\u{F025}").centered().with_size(11.0).with_color(config.text_color));

        let vol_row = ui.add_container(
            Style {
                size: Size { width: Dimension::percent(1.0), height: Dimension::length(24.0) },
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                ..Default::default()
            },
            SquircleStyle::transparent(),
            &[vol_low, vol_track, vol_high, output_btn],
            Some("slider_row:sound"),
        )?;

        let sound_card = ui.add_container(
            Style {
                size: Size { width: Dimension::percent(1.0), height: Dimension::length(60.0) },
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::CENTER,
                padding: Rect { left: LengthPercentage::length(12.0), right: LengthPercentage::length(12.0), top: LengthPercentage::length(6.0), bottom: LengthPercentage::length(6.0) },
                ..Default::default()
            },
            SquircleStyle::card(16.0, tile_bg).with_border(0.5, tile_border).with_power(config.squircle_power).with_z(82.0),
            &[sound_label, vol_row],
            Some("slider_box:sound"),
        )?;
        panel_children.push(sound_card);
    }

    // --------------------------------------------------------------------------
    // FILA 5: [Toggles Rápidos Circulares: Modo Oscuro, Captura, Teclado, Bloqueo] (38px)
    // --------------------------------------------------------------------------
    {
        let t_dark = ui.add_leaf(
            Style { size: Size { width: Dimension::length(38.0), height: Dimension::length(38.0) }, ..Default::default() },
            SquircleStyle::card(19.0, if cc.dark_mode { active_blue } else { tile_bg }).with_border(0.5, tile_border).with_z(84.0),
            Some("cc:dark_mode"),
        )?;
        ui.set_text(t_dark, TextLabel::new("\u{F186}").centered().with_size(13.0).with_color(if cc.dark_mode { [1.0, 1.0, 1.0, 1.0] } else { config.text_color }));

        let t_shot = ui.add_leaf(
            Style { size: Size { width: Dimension::length(38.0), height: Dimension::length(38.0) }, ..Default::default() },
            SquircleStyle::card(19.0, tile_bg).with_border(0.5, tile_border).with_z(84.0),
            Some("cc:screenshot"),
        )?;
        ui.set_text(t_shot, TextLabel::new("\u{F030}").centered().with_size(13.0).with_color(config.text_color));

        let t_kb = ui.add_leaf(
            Style { size: Size { width: Dimension::length(38.0), height: Dimension::length(38.0) }, ..Default::default() },
            SquircleStyle::card(19.0, tile_bg).with_border(0.5, tile_border).with_z(84.0),
            Some("cc:keyboard"),
        )?;
        ui.set_text(t_kb, TextLabel::new("\u{F11C}").centered().with_size(13.0).with_color(config.text_color));

        let t_lock = ui.add_leaf(
            Style { size: Size { width: Dimension::length(38.0), height: Dimension::length(38.0) }, ..Default::default() },
            SquircleStyle::card(19.0, tile_bg).with_border(0.5, tile_border).with_z(84.0),
            Some("cc:lock"),
        )?;
        ui.set_text(t_lock, TextLabel::new("\u{F023}").centered().with_size(13.0).with_color(config.text_color));

        let toggles_row = ui.add_container(
            Style {
                size: Size { width: Dimension::percent(1.0), height: Dimension::length(38.0) },
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                justify_content: JustifyContent::SPACE_BETWEEN,
                ..Default::default()
            },
            SquircleStyle::transparent(),
            &[t_dark, t_shot, t_kb, t_lock],
            Some("cc:toggles_row"),
        )?;
        panel_children.push(toggles_row);
    }

    // --------------------------------------------------------------------------
    // FILA 6: [Botón Editar Controles] (24px)
    // --------------------------------------------------------------------------
    {
        let edit_label = ui.add_leaf(
            Style { size: Size { width: Dimension::auto(), height: Dimension::length(14.0) }, ..Default::default() },
            SquircleStyle::transparent(),
            Some("cc:edit_label"),
        )?;
        ui.set_text(edit_label, TextLabel::new("Editar controles").centered().with_size(11.0).with_color(config.text_dimmed_color));

        let edit_btn = ui.add_container(
            Style {
                size: Size { width: Dimension::length(120.0), height: Dimension::length(24.0) },
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                justify_content: JustifyContent::CENTER,
                ..Default::default()
            },
            SquircleStyle::card(12.0, tile_bg).with_border(0.5, tile_border).with_z(84.0),
            &[edit_label],
            Some("cc:edit_controls"),
        )?;

        let edit_row = ui.add_container(
            Style {
                size: Size { width: Dimension::percent(1.0), height: Dimension::length(24.0) },
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::CENTER,
                justify_content: JustifyContent::CENTER,
                ..Default::default()
            },
            SquircleStyle::transparent(),
            &[edit_btn],
            Some("cc:edit_row"),
        )?;
        panel_children.push(edit_row);
    }

    // Tarjeta contenedora principal (transparente para que cada tarjeta flote individualmente estilo macOS Tahoe)
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
                width: Dimension::length(panel_w),
                height: Dimension::length(panel_h),
            },
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::SPACE_BETWEEN,
            padding: Rect {
                top: LengthPercentage::length(6.0),
                bottom: LengthPercentage::length(6.0),
                left: LengthPercentage::length(6.0),
                right: LengthPercentage::length(6.0),
            },
            ..Default::default()
        },
        SquircleStyle::transparent(),
        &panel_children,
        Some("popover_card"),
    )?;

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
        &[card],
        Some("popover_root_overlay"),
    )?;

    Ok(PopoverNodes {
        root_container,
        card,
        outer_shadow: None,
        inner_shadow: None,
        items: Vec::new(),
    })
}
