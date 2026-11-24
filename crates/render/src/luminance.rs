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
    /// Píldora translúcida de selección en la barra superior al abrir un menú (estilo macOS).
    pub topbar_active_bg: [f32; 4],
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
        let scale = malus_de_common::config::current_liquid_glass_scale();
        let pop_alpha =
            malus_de_common::config::scaled_liquid_glass_alpha(0.35, scale, 0.08, 0.90) as f32;
        if is_dark_background {
            // Fondo oscuro (fondos oscuros, nocturnos o espaciales estilo macOS)
            // Barra 100% transparente con texto e iconos en blanco puro / brillante.
            // Popovers con vidrio translúcido vibrante claro estilo macOS Tahoe / Monterey (menu-bar.jpg).
            Self {
                is_dark_background: true,
                text_primary: [0.98, 0.98, 1.0, 1.0],
                text_secondary: [0.85, 0.85, 0.90, 0.85],
                button_idle_bg: [0.0, 0.0, 0.0, 0.0], // Totalmente transparente
                button_hover_bg: [1.0, 1.0, 1.0, 0.04], // Escarcha translúcida muy sutil
                button_active_bg: [0.0, 0.48, 1.0, 0.70], // Acento azul macOS activo semitransparente (#007AFF)
                topbar_active_bg: [1.0, 1.0, 1.0, 0.02], // Píldora translúcida extremadamente sutil (2% opacidad)
                popover_bg: [0.93, 0.95, 0.98, pop_alpha], // Vidrio claro translúcido enlazado a Liquid Glass
                popover_border: [0.0, 0.0, 0.0, 0.22],     // Línea exterior oscura semitransparente
                popover_text: [0.11, 0.11, 0.13, 1.0],     // Grafito oscuro nítido (#1D1D1F)
                popover_text_dimmed: [0.43, 0.43, 0.46, 0.80], // Atajos y subtítulos (#6E6E73)
                popover_separator: [0.0, 0.0, 0.0, 0.08],  // Línea divisoria fina sutil
            }
        } else {
            // Fondo claro (fondos diurnos, soleados o minimalistas blancos)
            // Barra 100% transparente con texto e iconos en grafito oscuro de alto contraste.
            Self {
                is_dark_background: false,
                text_primary: [0.10, 0.10, 0.14, 1.0], // Grafito oscuro de alta legibilidad
                text_secondary: [0.28, 0.28, 0.35, 0.88],
                button_idle_bg: [0.0, 0.0, 0.0, 0.0], // Totalmente transparente
                button_hover_bg: [0.0, 0.0, 0.0, 0.03], // Escarcha translúcida oscura muy sutil
                button_active_bg: [0.0, 0.48, 1.0, 0.70], // Acento azul macOS activo semitransparente (#007AFF)
                topbar_active_bg: [0.0, 0.0, 0.0, 0.02], // Píldora translúcida sutil oscura (2% opacidad)
                popover_bg: [0.95, 0.96, 0.98, pop_alpha], // Vidrio claro translúcido enlazado a Liquid Glass
                popover_border: [0.0, 0.0, 0.0, 0.22],     // Línea exterior oscura semitransparente
                popover_text: [0.11, 0.11, 0.13, 1.0],     // Grafito oscuro nítido (#1D1D1F)
                popover_text_dimmed: [0.43, 0.43, 0.46, 0.80], // Atajos y subtítulos (#6E6E73)
                popover_separator: [0.0, 0.0, 0.0, 0.08],  // Línea divisoria fina sutil
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
/// Cachea el resultado por ruta y `(mtime, len)` del wallpaper para evitar lanzar `python3`+PIL en cada segundo.
pub fn detect_system_is_dark() -> bool {
    // 1. Intentar analizar la imagen de fondo de pantalla activa (solo cuando cambia en disco)
    if let Some(is_dark) = detect_wallpaper_luminance_is_dark_cached() {
        return is_dark;
    }

    // 2. Respaldo sin subprocesos: configuración aislada de modo oscuro de Malus DE
    malus_de_common::config::is_dark_mode_active()
}

fn detect_wallpaper_luminance_is_dark_cached() -> Option<bool> {
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant, SystemTime};

    struct WallpaperLumCache {
        path: PathBuf,
        mtime: Option<SystemTime>,
        len: Option<u64>,
        checked_at: Instant,
        result: Option<bool>,
    }

    static LUM_CACHE: OnceLock<Mutex<Option<WallpaperLumCache>>> = OnceLock::new();
    const CHECK_INTERVAL: Duration = Duration::from_secs(5);

    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let path = home.join(".config/malus-de/wallpaper.jpg");

    let lock = LUM_CACHE.get_or_init(|| Mutex::new(None));
    {
        let guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(ref cached) = *guard {
            if cached.path == path && cached.checked_at.elapsed() < CHECK_INTERVAL {
                return cached.result;
            }
        }
    }

    let meta = std::fs::metadata(&path).ok();
    let (mtime, len) = meta
        .as_ref()
        .map(|m| (m.modified().ok(), Some(m.len())))
        .unwrap_or((None, None));

    {
        let mut guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(ref mut cached) = *guard {
            if cached.path == path && cached.mtime == mtime && cached.len == len {
                cached.checked_at = Instant::now();
                return cached.result;
            }
        }
    }

    let result = if meta.is_some() {
        detect_wallpaper_luminance_is_dark(&path)
    } else {
        None
    };

    let mut guard = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *guard = Some(WallpaperLumCache {
        path,
        mtime,
        len,
        checked_at: Instant::now(),
        result,
    });
    result
}

/// Analiza la tira superior del archivo de fondo de pantalla para calcular su luminancia matemática ITU-R BT.709.
fn detect_wallpaper_luminance_is_dark(path: &std::path::Path) -> Option<bool> {
    if let Ok(out) = std::process::Command::new("python3")
        .args([
            "-c",
            r#"
import sys, warnings
warnings.filterwarnings('ignore')
try:
    from PIL import Image
    im = Image.open(sys.argv[1]).convert('RGB')
    top = im.crop((0, 0, im.width, min(48, im.height)))
    top.thumbnail((128, 16))
    px = list(top.getdata())
    lum = sum(0.2126*(p[0]/255) + 0.7152*(p[1]/255) + 0.0722*(p[2]/255) for p in px) / max(len(px), 1)
    sys.exit(0 if lum < 0.50 else 1)
except Exception:
    pass
sys.exit(2)
"#,
            path.to_str()?,
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
