// ==============================================================================
// Malus DE: Adaptabilidad de Luminancia y Paleta Dinámica para Barra Transparente
// ==============================================================================
// Calcula la luminancia percibida del fondo (ITU-R BT.709) y provee paletas de
// contraste adaptativo para texto, iconos, acentos y popovers sobre fondo transparente.

/// Paleta adaptativa con colores optimizados según el contraste del fondo.
#[derive(Clone, Debug, PartialEq)]
pub struct AdaptivePalette {
    /// `true` si el fondo es oscuro (texto blanco), `false` si es claro (texto oscuro).
    pub is_dark_background: bool,
    /// Color primario para títulos y botones de la barra (ej. "Finder", "Archivo", reloj).
    pub text_primary: [f32; 4],
    /// Color secundario para iconos de estado y subtítulos (ej. "WiFi", batería).
    pub text_secondary: [f32; 4],
    /// Fondo de botón en reposo (completamente transparente en la barra).
    pub button_idle_bg: [f32; 4],
    /// Fondo sutil al pasar el cursor (hover) sobre un botón de la barra.
    pub button_hover_bg: [f32; 4],
    /// Fondo activo cuando un menú o botón está presionado o su popover desplegado.
    pub button_active_bg: [f32; 4],
    /// Fondo de la tarjeta flotante del popover (vidrio translúcido esmerilado).
    pub popover_bg: [f32; 4],
    /// Borde perimetral reflectante del popover.
    pub popover_border: [f32; 4],
    /// Color de texto normal para los ítems dentro del popover.
    pub popover_text: [f32; 4],
    /// Color atenuado para atajos de teclado o ítems deshabilitados en popovers.
    pub popover_text_dimmed: [f32; 4],
    /// Color de las líneas divisorias en los menús popover.
    pub popover_separator: [f32; 4],
}

impl AdaptivePalette {
    /// Genera la paleta de contraste adaptativo para fondo oscuro o fondo claro.
    pub fn new(is_dark_background: bool) -> Self {
        if is_dark_background {
            // Fondo oscuro (fondos oscuros, nocturnos o espaciales estilo macOS)
            // Barra 100% transparente con texto e iconos en blanco puro / brillante.
            Self {
                is_dark_background: true,
                text_primary: [0.98, 0.98, 1.0, 1.0],
                text_secondary: [0.85, 0.85, 0.90, 0.85],
                button_idle_bg: [0.0, 0.0, 0.0, 0.0], // Totalmente transparente
                button_hover_bg: [1.0, 1.0, 1.0, 0.18], // Escarcha translúcida clara
                button_active_bg: [0.0, 0.48, 1.0, 0.90], // Acento azul macOS activo
                popover_bg: [0.12, 0.12, 0.15, 0.92],   // Vidrio oscuro translúcido
                popover_border: [1.0, 1.0, 1.0, 0.14],  // Borde reflectante sutil
                popover_text: [0.95, 0.95, 0.98, 1.0],
                popover_text_dimmed: [0.60, 0.60, 0.68, 0.75],
                popover_separator: [1.0, 1.0, 1.0, 0.10],
            }
        } else {
            // Fondo claro (fondos diurnos, soleados o minimalistas blancos)
            // Barra 100% transparente con texto e iconos en grafito oscuro de alto contraste.
            Self {
                is_dark_background: false,
                text_primary: [0.10, 0.10, 0.14, 1.0], // Grafito oscuro de alta legibilidad
                text_secondary: [0.28, 0.28, 0.35, 0.85],
                button_idle_bg: [0.0, 0.0, 0.0, 0.0], // Totalmente transparente
                button_hover_bg: [0.0, 0.0, 0.0, 0.12], // Escarcha translúcida oscura
                button_active_bg: [0.0, 0.48, 1.0, 0.90], // Acento azul macOS activo
                popover_bg: [0.95, 0.95, 0.97, 0.94],   // Vidrio claro translúcido
                popover_border: [0.0, 0.0, 0.0, 0.12],  // Borde oscuro fino
                popover_text: [0.12, 0.12, 0.16, 1.0],
                popover_text_dimmed: [0.45, 0.45, 0.50, 0.70],
                popover_separator: [0.0, 0.0, 0.0, 0.10],
            }
        }
    }

    /// Determina la paleta a partir del valor de luminancia percibida [0.0, 1.0].
    /// Luminancia < 0.50 indica fondo oscuro; >= 0.50 indica fondo claro.
    pub fn from_luminance(luminance: f32) -> Self {
        Self::new(luminance < 0.50)
    }
}

/// Calcula la luminancia percibida promedio de un búfer de píxeles RGBA según ITU-R BT.709.
/// Retorna un valor entre 0.0 (negro absoluto) y 1.0 (blanco puro).
pub fn calculate_luminance_rgba(rgba_data: &[u8]) -> f32 {
    if rgba_data.len() < 4 {
        return 0.0;
    }

    let mut total_lum = 0.0f64;
    let mut pixel_count = 0usize;

    for chunk in rgba_data.chunks_exact(4) {
        let r = chunk[0] as f64 / 255.0;
        let g = chunk[1] as f64 / 255.0;
        let b = chunk[2] as f64 / 255.0;

        // Fórmula estándar ITU-R BT.709
        let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        total_lum += lum;
        pixel_count += 1;
    }

    if pixel_count == 0 {
        0.0
    } else {
        (total_lum / pixel_count as f64) as f32
    }
}

/// Detecta si el fondo bajo la barra superior es oscuro (usando luminancia real del wallpaper) o el tema del sistema.
pub fn detect_system_is_dark() -> bool {
    // 1. Intentar analizar la imagen de fondo de pantalla activa en Malus DE / GNOME
    if let Some(is_dark) = detect_wallpaper_luminance_is_dark() {
        return is_dark;
    }

    // 2. Si no se puede leer el fondo de pantalla, recurrir al tema del sistema
    if let Ok(out) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
    {
        let scheme = String::from_utf8_lossy(&out.stdout).to_lowercase();
        if scheme.contains("prefer-dark") || scheme.contains("dark") {
            return true;
        }
    }

    true // Por defecto fondo oscuro en Malus DE (macOS dark vibrancy)
}

/// Analiza la tira superior del archivo de fondo de pantalla para calcular su luminancia matemática ITU-R BT.709.
fn detect_wallpaper_luminance_is_dark() -> Option<bool> {
    if let Ok(out) = std::process::Command::new("python3")
        .args([
            "-c",
            r#"
import os, sys
try:
    from PIL import Image
    home = os.environ.get('HOME', '')
    path = os.path.join(home, '.config/malus-de/wallpaper.jpg')
    if not os.path.exists(path):
        import subprocess
        uri = subprocess.check_output(['gsettings', 'get', 'org.gnome.desktop.background', 'picture-uri']).decode().strip().strip("'")
        if uri.startswith('file://'):
            path = uri[7:]
    if os.path.exists(path):
        im = Image.open(path)
        top = im.crop((0, 0, im.width, min(48, im.height)))
        px = list(top.getdata())
        lum = sum(0.2126*(p[0]/255) + 0.7152*(p[1]/255) + 0.0722*(p[2]/255) for p in px) / len(px)
        sys.exit(0 if lum < 0.50 else 1)
except Exception:
    pass
sys.exit(2)
"#,
        ])
        .status()
    {
        if out.code() == Some(0) {
            return Some(true); // Fondo oscuro -> texto blanco
        } else if out.code() == Some(1) {
            return Some(false); // Fondo claro -> texto oscuro
        }
    }
    None
}
