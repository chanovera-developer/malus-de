// ==============================================================================
// Malus DE: Sistema de Layout Reactivo con Taffy (Flexbox & Squircles)
// ==============================================================================

use crate::squircle::{SquircleInstance, MACOS_SQUIRCLE_POWER};
use crate::text::TextLabel;
use anyhow::{Context, Result};
use std::collections::HashMap;
pub use taffy::prelude::*;

/// Estilo visual decorativo para un nodo del árbol de UI.
#[derive(Clone, Debug)]
pub struct SquircleStyle {
    pub bg_color: Option<[f32; 4]>,
    pub border_color: Option<[f32; 4]>,
    pub border_width: f32,
    pub corner_radius: f32,
    pub squircle_power: f32,
    pub z_index: f32,
}

impl Default for SquircleStyle {
    fn default() -> Self {
        Self {
            bg_color: None,
            border_color: None,
            border_width: 0.0,
            corner_radius: 0.0,
            squircle_power: MACOS_SQUIRCLE_POWER,
            z_index: 0.0,
        }
    }
}

impl SquircleStyle {
    /// Nodo transparente sin representación visual (ideal para contenedores o espaciadores Flexbox).
    pub fn transparent() -> Self {
        Self::default()
    }

    /// Botón estándar con curvatura continua estilo macOS.
    pub fn button(bg_color: [f32; 4]) -> Self {
        Self {
            bg_color: Some(bg_color),
            corner_radius: 7.0,
            squircle_power: MACOS_SQUIRCLE_POWER,
            ..Default::default()
        }
    }

    /// Estilo tipo "Pill" o cápsula completamente redondeada en sus extremos.
    pub fn pill(bg_color: [f32; 4]) -> Self {
        Self {
            bg_color: Some(bg_color),
            corner_radius: 999.0, // El shader calcula automáticamente min(radius, min(w, h)/2)
            squircle_power: MACOS_SQUIRCLE_POWER,
            ..Default::default()
        }
    }

    /// Tarjeta o panel con radio de curvatura y color de fondo explícitos.
    pub fn card(corner_radius: f32, bg_color: [f32; 4]) -> Self {
        Self {
            bg_color: Some(bg_color),
            corner_radius,
            squircle_power: MACOS_SQUIRCLE_POWER,
            ..Default::default()
        }
    }

    /// Añade un contorno perimetral.
    pub fn with_border(mut self, width: f32, color: [f32; 4]) -> Self {
        self.border_width = width;
        self.border_color = Some(color);
        self
    }

    /// Sobrescribe el radio de curvatura de las esquinas.
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    /// Sobrescribe la potencia superelíptica (ej. 2.0 para círculo clásico).
    pub fn with_power(mut self, power: f32) -> Self {
        self.squircle_power = power;
        self
    }

    /// Ajusta la profundidad z en el orden de composición.
    pub fn with_z(mut self, z: f32) -> Self {
        self.z_index = z;
        self
    }

    /// Indica si el elemento tiene contenido visible que deba ser enviado al pipeline de GPU.
    pub fn is_visible(&self) -> bool {
        let has_bg = self.bg_color.map_or(false, |c| c[3] > 0.001);
        let has_border = self.border_width > 0.0 && self.border_color.map_or(false, |c| c[3] > 0.001);
        has_bg || has_border
    }
}

/// Metadatos asociados a un nodo de la interfaz de usuario.
#[derive(Clone, Debug)]
pub struct UiNodeData {
    pub style: SquircleStyle,
    pub tag: Option<String>,
    pub text: Option<TextLabel>,
}

/// Distancia con signo analítica para hit-testing idéntico al shader de la GPU.
pub fn squircle_sdf_cpu(p: [f32; 2], half_size: [f32; 2], r: f32, n: f32) -> f32 {
    let r = r.min(half_size[0].min(half_size[1]));
    let n = n.max(1.0);
    let dx = p[0].abs() - half_size[0] + r;
    let dy = p[1].abs() - half_size[1] + r;

    if dx <= 0.0 || dy <= 0.0 {
        dx.max(dy) - r
    } else {
        (dx.powf(n) + dy.powf(n)).powf(1.0 / n) - r
    }
}

/// Árbol jerárquico de UI que combina Taffy (para Flexbox/Grid) con Squircles WGPU.
pub struct UiTree {
    pub taffy: TaffyTree<()>,
    node_data: HashMap<NodeId, UiNodeData>,
    root: Option<NodeId>,
}

impl Default for UiTree {
    fn default() -> Self {
        Self::new()
    }
}

impl UiTree {
    /// Crea un nuevo árbol de UI vacío.
    pub fn new() -> Self {
        Self {
            taffy: TaffyTree::new(),
            node_data: HashMap::new(),
            root: None,
        }
    }

    /// Define el nodo raíz del árbol de UI.
    pub fn set_root(&mut self, root: NodeId) {
        self.root = Some(root);
    }

    /// Obtiene el nodo raíz si existe.
    pub fn root(&self) -> Option<NodeId> {
        self.root
    }

    /// Crea un nodo hoja (botón, etiqueta o elemento sin hijos).
    pub fn add_leaf(&mut self, layout_style: Style, visual_style: SquircleStyle, tag: Option<impl Into<String>>) -> Result<NodeId> {
        let node = self.taffy.new_leaf(layout_style).context("Error al crear nodo hoja en Taffy")?;
        self.node_data.insert(node, UiNodeData {
            style: visual_style,
            tag: tag.map(Into::into),
            text: None,
        });
        Ok(node)
    }

    /// Crea un contenedor Flexbox con hijos.
    pub fn add_container(
        &mut self,
        layout_style: Style,
        visual_style: SquircleStyle,
        children: &[NodeId],
        tag: Option<impl Into<String>>,
    ) -> Result<NodeId> {
        let node = self.taffy.new_with_children(layout_style, children).context("Error al crear contenedor en Taffy")?;
        self.node_data.insert(node, UiNodeData {
            style: visual_style,
            tag: tag.map(Into::into),
            text: None,
        });
        Ok(node)
    }

    /// Asigna una etiqueta de texto tipográfica a un nodo.
    pub fn set_text(&mut self, node: NodeId, text: TextLabel) {
        if let Some(data) = self.node_data.get_mut(&node) {
            data.text = Some(text);
        }
    }

    /// Genera la lista de etiquetas de texto con sus posiciones absolutas resueltas en el layout.
    pub fn generate_text_labels(&self) -> Vec<(TextLabel, [f32; 2], [f32; 2])> {
        let mut labels = Vec::new();
        if let Some(root) = self.root {
            self.collect_text_labels(root, (0.0, 0.0), &mut labels);
        }
        labels
    }

    fn collect_text_labels(&self, node: NodeId, parent_offset: (f32, f32), out: &mut Vec<(TextLabel, [f32; 2], [f32; 2])>) {
        let layout = match self.taffy.layout(node) {
            Ok(l) => l,
            Err(_) => return,
        };

        let abs_x = parent_offset.0 + layout.location.x;
        let abs_y = parent_offset.1 + layout.location.y;

        if let Some(data) = self.node_data.get(&node) {
            if let Some(ref text) = data.text {
                out.push((text.clone(), [abs_x, abs_y], [layout.size.width, layout.size.height]));
            }
        }

        if let Ok(children) = self.taffy.children(node) {
            for child in children {
                self.collect_text_labels(child, (abs_x, abs_y), out);
            }
        }
    }

    /// Actualiza el estilo visual de un nodo existente (ej. al pasar el mouse por encima / hover).
    pub fn set_visual_style(&mut self, node: NodeId, style: SquircleStyle) {
        if let Some(data) = self.node_data.get_mut(&node) {
            data.style = style;
        }
    }

    /// Obtiene los metadatos de un nodo.
    pub fn node_data(&self, node: NodeId) -> Option<&UiNodeData> {
        self.node_data.get(&node)
    }

    /// Calcula la disposición espacial en todo el árbol de acuerdo a las dimensiones dadas.
    pub fn compute_layout(&mut self, available_width: f32, available_height: f32) -> Result<()> {
        let root = self.root.context("No se ha definido un nodo raíz en el árbol de UI")?;
        self.taffy.compute_layout(
            root,
            Size {
                width: AvailableSpace::Definite(available_width),
                height: AvailableSpace::Definite(available_height),
            },
        ).context("Error al calcular layout con Taffy")?;
        Ok(())
    }

    /// Recorre el árbol y genera la lista plana de instancias `SquircleInstance` listas para la GPU.
    pub fn generate_instances(&self) -> Vec<SquircleInstance> {
        let mut instances = Vec::new();
        if let Some(root) = self.root {
            self.collect_instances(root, (0.0, 0.0), &mut instances);
        }
        instances
    }

    fn collect_instances(&self, node: NodeId, parent_offset: (f32, f32), out: &mut Vec<SquircleInstance>) {
        let layout = match self.taffy.layout(node) {
            Ok(l) => l,
            Err(_) => return,
        };

        let abs_x = parent_offset.0 + layout.location.x;
        let abs_y = parent_offset.1 + layout.location.y;

        if let Some(data) = self.node_data.get(&node) {
            if data.style.is_visible() {
                let bg = data.style.bg_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
                let border = data.style.border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);

                out.push(SquircleInstance {
                    position: [abs_x, abs_y],
                    size: [layout.size.width, layout.size.height],
                    bg_color: bg,
                    border_color: border,
                    corner_radius: data.style.corner_radius,
                    border_width: data.style.border_width,
                    squircle_power: data.style.squircle_power,
                    z_index: data.style.z_index,
                });
            }
        }

        if let Ok(children) = self.taffy.children(node) {
            for child in children {
                self.collect_instances(child, (abs_x, abs_y), out);
            }
        }
    }

    /// Realiza hit-testing matemático para determinar qué nodo se encuentra bajo las coordenadas (x, y).
    pub fn hit_test(&self, point: [f32; 2]) -> Option<NodeId> {
        self.root.and_then(|root| self.hit_test_recursive(root, (0.0, 0.0), point))
    }

    fn hit_test_recursive(&self, node: NodeId, parent_offset: (f32, f32), point: [f32; 2]) -> Option<NodeId> {
        let layout = self.taffy.layout(node).ok()?;
        let abs_x = parent_offset.0 + layout.location.x;
        let abs_y = parent_offset.1 + layout.location.y;

        // Comprobar primero los hijos en orden inverso (los últimos hijos se dibujan encima)
        if let Ok(children) = self.taffy.children(node) {
            for &child in children.iter().rev() {
                if let Some(hit) = self.hit_test_recursive(child, (abs_x, abs_y), point) {
                    return Some(hit);
                }
            }
        }

        // Comprobar si el punto está dentro del área del nodo actual
        let width = layout.size.width;
        let height = layout.size.height;
        if point[0] >= abs_x && point[0] <= abs_x + width && point[1] >= abs_y && point[1] <= abs_y + height {
            if let Some(data) = self.node_data.get(&node) {
                // Los contenedores estructurales transparentes no interceptan eventos,
                // excepto si son elementos interactivos identificados con texto o ítems de popover.
                let is_interactive = data.text.is_some()
                    || data
                        .tag
                        .as_deref()
                        .map_or(false, |t| t.starts_with("popover_item:"));
                if !data.style.is_visible() && !is_interactive {
                    return None;
                }

                // Comprobación analítica con la superelipse para respetar esquinas redondeadas
                let half_size = [width * 0.5, height * 0.5];
                let local_p = [point[0] - abs_x - half_size[0], point[1] - abs_y - half_size[1]];
                let dist = squircle_sdf_cpu(local_p, half_size, data.style.corner_radius, data.style.squircle_power);
                if dist <= 0.0 {
                    return Some(node);
                }
            }
        }

        None
    }

    /// Obtiene las coordenadas absolutas en pantalla (x, y, width, height) de un nodo del árbol.
    pub fn absolute_layout(&self, target_node: NodeId) -> Option<(f32, f32, f32, f32)> {
        self.root.and_then(|root| self.find_absolute_layout(root, (0.0, 0.0), target_node))
    }

    fn find_absolute_layout(
        &self,
        current: NodeId,
        parent_offset: (f32, f32),
        target: NodeId,
    ) -> Option<(f32, f32, f32, f32)> {
        let layout = self.taffy.layout(current).ok()?;
        let abs_x = parent_offset.0 + layout.location.x;
        let abs_y = parent_offset.1 + layout.location.y;

        if current == target {
            return Some((abs_x, abs_y, layout.size.width, layout.size.height));
        }

        if let Ok(children) = self.taffy.children(current) {
            for child in children {
                if let Some(res) = self.find_absolute_layout(child, (abs_x, abs_y), target) {
                    return Some(res);
                }
            }
        }
        None
    }
}
