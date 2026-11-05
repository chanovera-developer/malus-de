// ==============================================================================
// Malus DE: Sistema Tipográfico Acelerado por GPU con cosmic-text y glyphon
// ==============================================================================

use anyhow::Result;
pub use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Weight,
};
pub use glyphon::{Cache, Color, Resolution, TextAtlas, TextArea, TextBounds, TextRenderer, Viewport};

/// Estima el ancho en píxeles de una etiqueta de texto tipográfico con alta precisión.
pub fn estimate_label_width(text: &str, font_size: f32, weight: Weight) -> f32 {
    let is_bold = weight == Weight::BOLD || weight == Weight::SEMIBOLD;
    let base_factor = if is_bold { 0.65 } else { 0.58 };
    let mut total_w = 0.0f32;
    for c in text.chars() {
        if c == '\u{F179}' || c == '' || c == '\u{F8FF}' {
            total_w += font_size * 1.1;
        } else if c == '%' || c == '@' || c == '#' || c == '&' || c == 'm' || c == 'w' || c == 'M' || c == 'W' {
            total_w += font_size * 0.90;
        } else if c.is_ascii_uppercase() {
            total_w += font_size * 0.74;
        } else if c == 'i' || c == 'l' || c == 'j' || c == '!' || c == '|' || c == ':' || c == '.' || c == '\'' {
            total_w += font_size * 0.32;
        } else if c.is_ascii_punctuation() || c == ' ' {
            total_w += font_size * 0.45;
        } else if c.is_ascii_digit() {
            total_w += font_size * 0.65;
        } else {
            total_w += font_size * base_factor;
        }
    }
    total_w.ceil()
}

/// Alineación horizontal del texto dentro de las dimensiones del nodo.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Center,
    Left,
    Right,
}

/// Etiqueta de texto decorativa para un botón o elemento de la interfaz de usuario.
#[derive(Clone, Debug)]
pub struct TextLabel {
    pub text: String,
    pub font_size: f32,
    pub line_height: f32,
    pub color: [f32; 4],
    pub weight: Weight,
    pub align: TextAlign,
}

impl Default for TextLabel {
    fn default() -> Self {
        Self {
            text: String::new(),
            font_size: 13.0,
            line_height: 16.0,
            color: [0.95, 0.95, 0.98, 1.0],
            weight: Weight::NORMAL,
            align: TextAlign::Center,
        }
    }
}

impl TextLabel {
    /// Crea una etiqueta de texto estándar para barra de menús o botones.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }

    /// Modifica el tamaño de fuente y la altura de línea proporcionalmente.
    pub fn with_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self.line_height = size * 1.25;
        self
    }

    /// Establece el color de la tipografía [R, G, B, A].
    pub fn with_color(mut self, color: [f32; 4]) -> Self {
        self.color = color;
        self
    }

    /// Aplica peso tipográfico (ej. `Weight::BOLD` o `Weight::SEMIBOLD`).
    pub fn with_weight(mut self, weight: Weight) -> Self {
        self.weight = weight;
        self
    }

    /// Alinea el texto a la izquierda en lugar de centrarlo dentro del contenedor.
    pub fn left_aligned(mut self) -> Self {
        self.align = TextAlign::Left;
        self
    }

    /// Alinea el texto a la derecha dentro del contenedor (útil para atajos de teclado en menús).
    pub fn right_aligned(mut self) -> Self {
        self.align = TextAlign::Right;
        self
    }

    /// Centra el texto dentro del contenedor.
    pub fn centered(mut self) -> Self {
        self.align = TextAlign::Center;
        self
    }
}

/// Instancia interna preparada de un búfer de texto con su posición en pantalla y color.
pub struct PreparedTextItem {
    pub buffer: Buffer,
    pub left: f32,
    pub top: f32,
    pub color: Color,
}

/// Subsistema gráfico de texto para Malus DE impulsado por `cosmic-text` y `glyphon`.
pub struct TextSystem {
    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
    pub cache: Cache,
    pub viewport: Viewport,
    pub atlas: TextAtlas,
    pub text_renderer: TextRenderer,
    pub prepared_items: Vec<PreparedTextItem>,
}

impl TextSystem {
    /// Inicializa el atlas de glifos GPU, el viewport y el motor de rasterización.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, target_format: wgpu::TextureFormat) -> Self {
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, target_format);
        let text_renderer = TextRenderer::new(
            &mut atlas,
            device,
            wgpu::MultisampleState::default(),
            None,
        );

        Self {
            font_system,
            swash_cache,
            cache,
            viewport,
            atlas,
            text_renderer,
            prepared_items: Vec::new(),
        }
    }

    /// Prepara y conforma los búferes de texto a partir de una lista de etiquetas y sus posiciones/dimensiones.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_size: [f32; 2],
        _scale_factor: f32,
        labels: &[(TextLabel, [f32; 2], [f32; 2])], // (Label, position [x, y], size [w, h])
    ) -> Result<()> {
        let width = screen_size[0] as u32;
        let height = screen_size[1] as u32;

        self.viewport.update(
            queue,
            Resolution {
                width: width.max(1),
                height: height.max(1),
            },
        );

        self.prepared_items.clear();
        self.prepared_items.reserve(labels.len());

        for (label, pos, size) in labels {
            let metrics = Metrics::new(label.font_size, label.line_height);
            let mut buffer = Buffer::new(&mut self.font_system, metrics);

            let family = if label.text.contains('\u{F179}') || label.text.contains('') || label.text.contains('\u{F8FF}') {
                Family::Name("Font Awesome 6 Brands")
            } else {
                Family::SansSerif
            };

            buffer.set_text(
                &mut self.font_system,
                &label.text,
                Attrs::new()
                    .family(family)
                    .weight(label.weight),
                Shaping::Advanced,
            );
            buffer.shape_until_scroll(&mut self.font_system, false);

            // Medir el texto conformado
            let text_w = buffer
                .layout_runs()
                .map(|run| run.line_w)
                .fold(0.0f32, f32::max);
            let text_h = label.line_height;

            // Calcular posición horizontal y vertical según alineación
            let (draw_x, draw_y) = match label.align {
                TextAlign::Center => {
                    let offset_x = ((size[0] - text_w) * 0.5).max(0.0);
                    let offset_y = ((size[1] - text_h) * 0.5).max(0.0);
                    (pos[0] + offset_x, pos[1] + offset_y)
                }
                TextAlign::Left => {
                    (pos[0] + 6.0, pos[1] + ((size[1] - text_h) * 0.5).max(0.0))
                }
                TextAlign::Right => {
                    let offset_x = (size[0] - text_w - 6.0).max(0.0);
                    (pos[0] + offset_x, pos[1] + ((size[1] - text_h) * 0.5).max(0.0))
                }
            };

            let r = (label.color[0] * 255.0).clamp(0.0, 255.0) as u8;
            let g = (label.color[1] * 255.0).clamp(0.0, 255.0) as u8;
            let b = (label.color[2] * 255.0).clamp(0.0, 255.0) as u8;
            let a = (label.color[3] * 255.0).clamp(0.0, 255.0) as u8;

            self.prepared_items.push(PreparedTextItem {
                buffer,
                left: draw_x,
                top: draw_y,
                color: Color::rgba(r, g, b, a),
            });
        }

        // Construir la lista de áreas de texto para glyphon
        let text_areas = self.prepared_items.iter().map(|item| TextArea {
            buffer: &item.buffer,
            left: item.left,
            top: item.top,
            scale: 1.0, // La escala 1.0 respeta exactamente las coordenadas métricas del buffer
            bounds: TextBounds {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: height as i32,
            },
            default_color: item.color,
            custom_glyphs: &[],
        });

        self.text_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                text_areas,
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("Error al preparar atlas de texto en GPU: {:?}", e))?;

        Ok(())
    }

    /// Dibuja todos los glifos de texto preparados dentro del render pass activo de WGPU.
    pub fn render<'rp>(&'rp self, rpass: &mut wgpu::RenderPass<'rp>) -> Result<()> {
        self.text_renderer
            .render(&self.atlas, &self.viewport, rpass)
            .map_err(|e| anyhow::anyhow!("Error al renderizar texto en GPU: {:?}", e))?;
        Ok(())
    }

    /// Limpia glifos que ya no estén en uso en el atlas de texturas.
    pub fn trim(&mut self) {
        self.atlas.trim();
    }
}
