//! Gestión de teclas multimedia y de hardware (XF86Audio*, XF86MonBrightness*,
//! XF86KbdBrightness*, MPRIS, MicMute, Touchpad, Calculadora, Explorador) y
//! renderizado del HUD OSD estilo macOS con Liquid Glass en el compositor.

use std::f64::consts::FRAC_PI_2;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cairo::{Context, Format, ImageSurface, LinearGradient, Operator};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind,
            },
            gles::GlesRenderer,
        },
    },
    input::keyboard::{keysyms, Keysym, ModifiersState},
    utils::{Logical, Physical, Point, Rectangle, Size},
};
use tracing::info;

/// Tiempo total durante el cual el HUD OSD permanece visible tras pulsar una tecla multimedia.
pub const OSD_VISIBLE_DURATION: Duration = Duration::from_millis(1650);
/// Duración de la animación de entrada (fade-in) del HUD OSD.
pub const OSD_FADE_IN_DURATION: Duration = Duration::from_millis(95);
/// Duración de la animación de salida (fade-out) del HUD OSD.
pub const OSD_FADE_OUT_DURATION: Duration = Duration::from_millis(220);

/// Retardo inicial antes de iniciar auto-repetición al mantener pulsada una tecla de volumen/brillo.
pub const REPEAT_INITIAL_DELAY: Duration = Duration::from_millis(280);
/// Intervalo entre pasos sucesivos al mantener pulsada una tecla de volumen/brillo.
pub const REPEAT_INTERVAL: Duration = Duration::from_millis(65);

pub const OSD_WIDTH: i32 = 228;
pub const OSD_HEIGHT: i32 = 60;
pub const OSD_CORNER_RADIUS: f64 = 18.0;
const SQUIRCLE_EXPONENT: f64 = 4.2;

/// Acción asociada a una tecla multimedia o de función de hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKeyAction {
    VolumeUp { fine: bool },
    VolumeDown { fine: bool },
    VolumeMute,
    MicMute,
    BrightnessUp { fine: bool },
    BrightnessDown { fine: bool },
    BrightnessCycle,
    KbdBrightnessUp { fine: bool },
    KbdBrightnessDown { fine: bool },
    KbdBrightnessToggle,
    MediaPlayPause,
    MediaPause,
    MediaStop,
    MediaNext,
    MediaPrev,
    Calculator,
    Explorer,
    Search,
    MissionControl,
    ScreenSaver,
    TouchpadToggle,
    TouchpadOn,
    TouchpadOff,
}

impl MediaKeyAction {
    /// Indica si la tecla puede ejecutarse incluso con la pantalla bloqueada (`session-lock`).
    pub fn is_allowed_when_locked(self) -> bool {
        matches!(
            self,
            Self::VolumeUp { .. }
                | Self::VolumeDown { .. }
                | Self::VolumeMute
                | Self::MicMute
                | Self::BrightnessUp { .. }
                | Self::BrightnessDown { .. }
                | Self::BrightnessCycle
                | Self::KbdBrightnessUp { .. }
                | Self::KbdBrightnessDown { .. }
                | Self::KbdBrightnessToggle
                | Self::MediaPlayPause
                | Self::MediaPause
                | Self::MediaStop
                | Self::MediaNext
                | Self::MediaPrev
                | Self::TouchpadToggle
                | Self::TouchpadOn
                | Self::TouchpadOff
        )
    }

    /// Indica si mantener presionada la tecla debe repetir la acción progresivamente.
    pub fn is_repeatable(self) -> bool {
        matches!(
            self,
            Self::VolumeUp { .. }
                | Self::VolumeDown { .. }
                | Self::BrightnessUp { .. }
                | Self::BrightnessDown { .. }
                | Self::KbdBrightnessUp { .. }
                | Self::KbdBrightnessDown { .. }
        )
    }
}

/// Detecta si una pulsación de teclado corresponde a una tecla multimedia/hardware.
///
/// Comprueba tanto `modified_sym` como `raw_syms` (para que combinaciones con `Shift` o `Alt`
/// funcionen para ajuste fino de 2% aunque el mapa XKB no defina nivel 2 para `XF86*`)
/// y utiliza los códigos de tecla físicos XKB (evdev + 8) como respaldo directo.
pub fn detect_media_key_action(
    modified_sym: Keysym,
    raw_syms: &[Keysym],
    raw_keycode: u32,
    modifiers: &ModifiersState,
) -> Option<MediaKeyAction> {
    let fine = modifiers.shift || modifiers.alt;
    let has_sym = |target: u32| -> bool {
        let target_sym: Keysym = target.into();
        modified_sym == target_sym || raw_syms.contains(&target_sym)
    };

    // 1. Volumen de salida (PipeWire / WirePlumber)
    if has_sym(keysyms::KEY_XF86AudioRaiseVolume) || raw_keycode == 123 {
        return Some(MediaKeyAction::VolumeUp { fine });
    }
    if has_sym(keysyms::KEY_XF86AudioLowerVolume) || raw_keycode == 122 {
        return Some(MediaKeyAction::VolumeDown { fine });
    }
    if has_sym(keysyms::KEY_XF86AudioMute) || raw_keycode == 121 {
        return Some(MediaKeyAction::VolumeMute);
    }
    if has_sym(keysyms::KEY_XF86AudioMicMute) || raw_keycode == 256 {
        return Some(MediaKeyAction::MicMute);
    }

    // 2. Brillo de pantalla (Backlight sysfs / login1 / DRM CRTC Gamma / Software Dim)
    if has_sym(keysyms::KEY_XF86MonBrightnessUp) || raw_keycode == 233 {
        return Some(MediaKeyAction::BrightnessUp { fine });
    }
    if has_sym(keysyms::KEY_XF86MonBrightnessDown) || raw_keycode == 232 {
        return Some(MediaKeyAction::BrightnessDown { fine });
    }
    if has_sym(keysyms::KEY_XF86BrightnessAdjust) || raw_keycode == 251 {
        return Some(MediaKeyAction::BrightnessCycle);
    }

    // 3. Retroiluminación del teclado (/sys/class/leds/*kbd_backlight*)
    if has_sym(keysyms::KEY_XF86KbdBrightnessUp) || raw_keycode == 238 {
        return Some(MediaKeyAction::KbdBrightnessUp { fine });
    }
    if has_sym(keysyms::KEY_XF86KbdBrightnessDown) || raw_keycode == 237 {
        return Some(MediaKeyAction::KbdBrightnessDown { fine });
    }
    if has_sym(keysyms::KEY_XF86KbdLightOnOff) || raw_keycode == 236 {
        return Some(MediaKeyAction::KbdBrightnessToggle);
    }

    // 4. Control de reproducción multimedia (MPRIS2 D-Bus)
    if has_sym(keysyms::KEY_XF86AudioPlay)
        || raw_keycode == 172
        || raw_keycode == 215
    {
        return Some(MediaKeyAction::MediaPlayPause);
    }
    if has_sym(keysyms::KEY_XF86AudioPause) || raw_keycode == 209 {
        return Some(MediaKeyAction::MediaPause);
    }
    if has_sym(keysyms::KEY_XF86AudioStop) || raw_keycode == 174 {
        return Some(MediaKeyAction::MediaStop);
    }
    if has_sym(keysyms::KEY_XF86AudioNext) || raw_keycode == 171 {
        return Some(MediaKeyAction::MediaNext);
    }
    if has_sym(keysyms::KEY_XF86AudioPrev) || raw_keycode == 173 {
        return Some(MediaKeyAction::MediaPrev);
    }

    // 5. Teclas rápidas de hardware adicionales (Calculadora, Explorador, Spotlight, Exposé, Lock, Touchpad)
    if has_sym(keysyms::KEY_XF86Calculator)
        || has_sym(keysyms::KEY_XF86Calculater)
        || raw_keycode == 148
    {
        return Some(MediaKeyAction::Calculator);
    }
    if has_sym(keysyms::KEY_XF86Explorer) || raw_keycode == 152 {
        return Some(MediaKeyAction::Explorer);
    }
    if has_sym(keysyms::KEY_XF86Search) || raw_keycode == 225 {
        return Some(MediaKeyAction::Search);
    }
    if has_sym(keysyms::KEY_XF86LaunchA) || has_sym(keysyms::KEY_XF86LaunchB) {
        return Some(MediaKeyAction::MissionControl);
    }
    if has_sym(keysyms::KEY_XF86ScreenSaver) || has_sym(keysyms::KEY_XF86Sleep) {
        return Some(MediaKeyAction::ScreenSaver);
    }
    if has_sym(keysyms::KEY_XF86TouchpadToggle) || raw_keycode == 199 {
        return Some(MediaKeyAction::TouchpadToggle);
    }
    if has_sym(keysyms::KEY_XF86TouchpadOn) {
        return Some(MediaKeyAction::TouchpadOn);
    }
    if has_sym(keysyms::KEY_XF86TouchpadOff) {
        return Some(MediaKeyAction::TouchpadOff);
    }

    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsdIconKind {
    VolumeHigh,
    VolumeMedium,
    VolumeLow,
    VolumeMuted,
    MicActive,
    MicMuted,
    Brightness,
    KbdBrightness,
    MediaPlayPause,
    MediaNext,
    MediaPrev,
    MediaStop,
    TouchpadOn,
    TouchpadOff,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveOsd {
    pub output_name: String,
    pub icon: OsdIconKind,
    pub title: String,
    pub subtitle: String,
    /// Nivel normalizado `0.0..=100.0` si el OSD muestra barra de progreso.
    pub level_pct: Option<f64>,
    pub muted_style: bool,
    pub first_shown_at: Instant,
    pub updated_at: Instant,
}

#[derive(Debug, Clone, Copy)]
struct ActiveRepeat {
    raw_keycode: u32,
    action: MediaKeyAction,
    pressed_at: Instant,
    last_repeat_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OsdCacheKey {
    phys_w: i32,
    phys_h: i32,
    scale_milli: i32,
    glass_milli: i32,
    is_dark: bool,
    icon: OsdIconKind,
    title: String,
    subtitle: String,
    level_permille: Option<i32>,
    muted_style: bool,
}

/// Estado compartido para sincronización asíncrona con PipeWire / WirePlumber (`wpctl`).
#[derive(Debug)]
struct SharedAudioSnapshot {
    volume_pct: f64,
    muted: bool,
    mic_muted: bool,
    dirty_for_osd: bool,
}

pub struct MediaKeysState {
    audio_state: Arc<Mutex<SharedAudioSnapshot>>,
    cached_brightness_pct: f64,
    cached_kbd_brightness_pct: f64,
    active_repeat: Option<ActiveRepeat>,
    active_osd: Option<ActiveOsd>,
    closed_dirty: bool,
    cache_key: Option<OsdCacheKey>,
    render_buffer: Option<MemoryRenderBuffer>,
    render_buffer_size: (i32, i32),
    mask_buffer: Vec<u8>,
    mask_size: (i32, i32),
}

impl Default for MediaKeysState {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaKeysState {
    pub fn new() -> Self {
        let initial_brightness = malus_de_common::config::read_hardware_or_config_brightness();
        let initial_kbd = malus_de_common::config::read_keyboard_backlight_pct().unwrap_or(50.0);
        let audio_state = Arc::new(Mutex::new(SharedAudioSnapshot {
            volume_pct: 75.0,
            muted: false,
            mic_muted: false,
            dirty_for_osd: false,
        }));

        // Consultar el volumen inicial de PipeWire en segundo plano sin bloquear el arranque
        let audio_init = Arc::clone(&audio_state);
        std::thread::spawn(move || {
            if let Some((vol, muted)) = query_wpctl_volume("@DEFAULT_AUDIO_SINK@") {
                if let Ok(mut guard) = audio_init.lock() {
                    guard.volume_pct = vol;
                    guard.muted = muted;
                }
            }
            if let Some((_, mic_muted)) = query_wpctl_volume("@DEFAULT_AUDIO_SOURCE@") {
                if let Ok(mut guard) = audio_init.lock() {
                    guard.mic_muted = mic_muted;
                }
            }
        });

        Self {
            audio_state,
            cached_brightness_pct: initial_brightness,
            cached_kbd_brightness_pct: initial_kbd,
            active_repeat: None,
            active_osd: None,
            closed_dirty: false,
            cache_key: None,
            render_buffer: None,
            render_buffer_size: (0, 0),
            mask_buffer: Vec::new(),
            mask_size: (0, 0),
        }
    }

    pub fn is_osd_open(&self) -> bool {
        self.active_osd.is_some()
    }

    pub fn has_active_repeat(&self) -> bool {
        self.active_repeat.is_some()
    }

    pub fn should_render_on_output(&self, output_name: &str) -> bool {
        self.active_osd
            .as_ref()
            .is_some_and(|osd| osd.output_name == output_name)
    }

    pub fn take_closed_dirty(&mut self) -> bool {
        std::mem::take(&mut self.closed_dirty)
    }

    /// Registra la liberación de una tecla física para detener cualquier auto-repetición activa.
    pub fn handle_key_release(&mut self, raw_keycode: u32) {
        if self
            .active_repeat
            .is_some_and(|rep| rep.raw_keycode == raw_keycode)
        {
            self.active_repeat = None;
        }
    }

    /// Inicia la acción de una tecla multimedia pulsada y configura su auto-repetición si aplica.
    pub fn register_key_press(
        &mut self,
        raw_keycode: u32,
        action: MediaKeyAction,
        now: Instant,
    ) {
        if action.is_repeatable() {
            self.active_repeat = Some(ActiveRepeat {
                raw_keycode,
                action,
                pressed_at: now,
                last_repeat_at: now,
            });
        } else {
            self.active_repeat = None;
        }
    }

    /// Avanza la auto-repetición de teclas mantenidas y la expiración del HUD OSD.
    /// Devuelve `Some(MediaKeyAction)` si corresponde disparar un paso repetido en este fotograma.
    pub fn tick(&mut self, now: Instant) -> Option<MediaKeyAction> {
        // Sincronizar lectura real de wpctl si terminó un comando de audio en segundo plano
        if let Ok(mut guard) = self.audio_state.lock() {
            if guard.dirty_for_osd {
                guard.dirty_for_osd = false;
                let vol = guard.volume_pct;
                let muted = guard.muted;
                if let Some(ref mut osd) = self.active_osd {
                    if matches!(
                        osd.icon,
                        OsdIconKind::VolumeHigh
                            | OsdIconKind::VolumeMedium
                            | OsdIconKind::VolumeLow
                            | OsdIconKind::VolumeMuted
                    ) {
                        osd.icon = volume_icon_for(vol, muted);
                        osd.subtitle = if muted || vol < 0.5 {
                            "Silenciado".to_string()
                        } else {
                            format!("{}%", vol.round() as i32)
                        };
                        osd.level_pct = Some(if muted { 0.0 } else { vol });
                        osd.muted_style = muted || vol < 0.5;
                    }
                }
            }
        }

        if let Some(ref osd) = self.active_osd {
            if now.saturating_duration_since(osd.updated_at) >= OSD_VISIBLE_DURATION {
                self.active_osd = None;
                self.closed_dirty = true;
            }
        }

        if let Some(ref mut rep) = self.active_repeat {
            if now.saturating_duration_since(rep.pressed_at) >= REPEAT_INITIAL_DELAY
                && now.saturating_duration_since(rep.last_repeat_at) >= REPEAT_INTERVAL
            {
                rep.last_repeat_at = now;
                return Some(rep.action);
            }
        }

        None
    }

    pub fn show_osd(
        &mut self,
        output_name: String,
        icon: OsdIconKind,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        level_pct: Option<f64>,
        muted_style: bool,
        now: Instant,
    ) {
        let first_shown_at = self
            .active_osd
            .as_ref()
            .map(|prev| prev.first_shown_at)
            .unwrap_or(now);
        self.active_osd = Some(ActiveOsd {
            output_name,
            icon,
            title: title.into(),
            subtitle: subtitle.into(),
            level_pct: level_pct.map(|v| v.clamp(0.0, 100.0)),
            muted_style,
            first_shown_at,
            updated_at: now,
        });
    }

    /// Ajusta el volumen de salida en `delta_pct` (ej. `+5.0` o `-5.0`) sin bloquear el compositor.
    pub fn step_volume(
        &mut self,
        delta_pct: f64,
        output_name: String,
        now: Instant,
        ipc_broadcaster: malus_de_common::ipc::IpcBroadcaster,
    ) {
        let (next_vol, next_muted) = {
            let mut guard = self.audio_state.lock().unwrap();
            let base = if guard.muted && delta_pct > 0.0 {
                guard.volume_pct
            } else {
                guard.volume_pct
            };
            let updated = (base + delta_pct).clamp(0.0, 100.0);
            let muted = updated < 0.5;
            guard.volume_pct = updated;
            guard.muted = muted;
            (updated, muted)
        };

        let subtitle = if next_muted {
            "Silenciado".to_string()
        } else {
            format!("{}%", next_vol.round() as i32)
        };
        self.show_osd(
            output_name,
            volume_icon_for(next_vol, next_muted),
            "Volumen",
            subtitle,
            Some(if next_muted { 0.0 } else { next_vol }),
            next_muted,
            now,
        );

        let shared = Arc::clone(&self.audio_state);
        let step_abs = delta_pct.abs();
        let is_up = delta_pct >= 0.0;
        std::thread::spawn(move || {
            if is_up {
                let _ = Command::new("wpctl")
                    .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "0"])
                    .status();
                let arg = format!("{:.0}%+", step_abs.max(1.0));
                let status = Command::new("wpctl")
                    .args(["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", &arg])
                    .status();
                if status.is_err() {
                    let _ = Command::new("pactl")
                        .args(["set-sink-mute", "@DEFAULT_SINK@", "0"])
                        .status();
                    let _ = Command::new("pactl")
                        .args([
                            "set-sink-volume",
                            "@DEFAULT_SINK@",
                            &format!("+{}%", step_abs.round() as i32),
                        ])
                        .status();
                }
            } else {
                let arg = format!("{:.0}%-", step_abs.max(1.0));
                let status = Command::new("wpctl")
                    .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &arg])
                    .status();
                if status.is_err() {
                    let _ = Command::new("pactl")
                        .args([
                            "set-sink-volume",
                            "@DEFAULT_SINK@",
                            &format!("-{}%", step_abs.round() as i32),
                        ])
                        .status();
                }
            }

            if let Some((vol, muted)) = query_wpctl_volume("@DEFAULT_AUDIO_SINK@") {
                if let Ok(mut guard) = shared.lock() {
                    guard.volume_pct = vol;
                    guard.muted = muted;
                    guard.dirty_for_osd = true;
                }
            }
            ipc_broadcaster.broadcast(&malus_de_common::ipc::IpcMessage::ReloadMenuBarConfig);
        });
    }

    /// Alterna el silencio (mute) de la salida de audio principal.
    pub fn toggle_volume_mute(
        &mut self,
        output_name: String,
        now: Instant,
        ipc_broadcaster: malus_de_common::ipc::IpcBroadcaster,
    ) {
        let (vol, next_muted) = {
            let mut guard = self.audio_state.lock().unwrap();
            guard.muted = !guard.muted;
            (guard.volume_pct, guard.muted)
        };

        let subtitle = if next_muted || vol < 0.5 {
            "Silenciado".to_string()
        } else {
            format!("{}%", vol.round() as i32)
        };
        self.show_osd(
            output_name,
            volume_icon_for(vol, next_muted),
            "Volumen",
            subtitle,
            Some(if next_muted { 0.0 } else { vol }),
            next_muted,
            now,
        );

        let shared = Arc::clone(&self.audio_state);
        std::thread::spawn(move || {
            let status = Command::new("wpctl")
                .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
                .status();
            if status.is_err() {
                let _ = Command::new("pactl")
                    .args(["set-sink-mute", "@DEFAULT_SINK@", "toggle"])
                    .status();
            }
            if let Some((vol, muted)) = query_wpctl_volume("@DEFAULT_AUDIO_SINK@") {
                if let Ok(mut guard) = shared.lock() {
                    guard.volume_pct = vol;
                    guard.muted = muted;
                    guard.dirty_for_osd = true;
                }
            }
            ipc_broadcaster.broadcast(&malus_de_common::ipc::IpcMessage::ReloadMenuBarConfig);
        });
    }

    /// Alterna el silencio (mute) del micrófono principal (`@DEFAULT_AUDIO_SOURCE@`).
    pub fn toggle_mic_mute(&mut self, output_name: String, now: Instant) {
        let next_muted = {
            let mut guard = self.audio_state.lock().unwrap();
            guard.mic_muted = !guard.mic_muted;
            guard.mic_muted
        };

        self.show_osd(
            output_name,
            if next_muted {
                OsdIconKind::MicMuted
            } else {
                OsdIconKind::MicActive
            },
            "Micrófono",
            if next_muted { "Silenciado" } else { "Activo" },
            Some(if next_muted { 0.0 } else { 100.0 }),
            next_muted,
            now,
        );

        let shared = Arc::clone(&self.audio_state);
        std::thread::spawn(move || {
            let status = Command::new("wpctl")
                .args(["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"])
                .status();
            if status.is_err() {
                let _ = Command::new("pactl")
                    .args(["set-source-mute", "@DEFAULT_SOURCE@", "toggle"])
                    .status();
            }
            if let Some((_, muted)) = query_wpctl_volume("@DEFAULT_AUDIO_SOURCE@") {
                if let Ok(mut guard) = shared.lock() {
                    guard.mic_muted = muted;
                }
            }
        });
    }

    /// Ajusta el brillo de pantalla (`delta_pct`), persiste en `monitors.toml` y aplica por hardware/logind/DRM.
    pub fn step_brightness(&mut self, delta_pct: f64, output_name: String, now: Instant) -> f64 {
        // Sincronizar con hardware o monitors.toml si no hay OSD reciente activo
        if self.active_osd.is_none() {
            self.cached_brightness_pct =
                malus_de_common::config::read_hardware_or_config_brightness();
        }
        let target = (self.cached_brightness_pct + delta_pct).clamp(5.0, 100.0);
        self.cached_brightness_pct = target;

        malus_de_common::config::set_global_and_outputs_brightness(target);

        self.show_osd(
            output_name,
            OsdIconKind::Brightness,
            "Brillo",
            format!("{}%", target.round() as i32),
            Some(target),
            false,
            now,
        );
        target
    }

    /// Cicla el brillo de pantalla en pasos predefinidos (`25% -> 50% -> 75% -> 100% -> 25%`).
    pub fn cycle_brightness(&mut self, output_name: String, now: Instant) -> f64 {
        let cur = self.cached_brightness_pct;
        let next = if cur < 25.0 {
            25.0
        } else if cur < 50.0 {
            50.0
        } else if cur < 75.0 {
            75.0
        } else if cur < 98.0 {
            100.0
        } else {
            25.0
        };
        let delta = next - cur;
        self.step_brightness(delta, output_name, now)
    }

    /// Ajusta el brillo de la retroiluminación del teclado (`/sys/class/leds/*kbd_backlight*`).
    pub fn step_kbd_brightness(&mut self, delta_pct: f64, output_name: String, now: Instant) {
        let next = malus_de_common::config::step_keyboard_backlight_pct(delta_pct)
            .unwrap_or_else(|| (self.cached_kbd_brightness_pct + delta_pct).clamp(0.0, 100.0));
        self.cached_kbd_brightness_pct = next;
        let subtitle = if next < 0.5 {
            "Apagado".to_string()
        } else {
            format!("{}%", next.round() as i32)
        };
        self.show_osd(
            output_name,
            OsdIconKind::KbdBrightness,
            "Luz de teclado",
            subtitle,
            Some(next),
            next < 0.5,
            now,
        );
    }

    /// Alterna el encendido/apagado de la retroiluminación del teclado.
    pub fn toggle_kbd_brightness(&mut self, output_name: String, now: Instant) {
        let next = malus_de_common::config::toggle_keyboard_backlight_pct().unwrap_or_else(|| {
            if self.cached_kbd_brightness_pct > 1.0 {
                0.0
            } else {
                100.0
            }
        });
        self.cached_kbd_brightness_pct = next;
        let subtitle = if next < 0.5 {
            "Apagado".to_string()
        } else {
            format!("{}%", next.round() as i32)
        };
        self.show_osd(
            output_name,
            OsdIconKind::KbdBrightness,
            "Luz de teclado",
            subtitle,
            Some(next),
            next < 0.5,
            now,
        );
    }

    /// Envía un comando MPRIS2 (`PlayPause`, `Pause`, `Stop`, `Next`, `Previous`) al reproductor activo.
    pub fn trigger_mpris_command(
        &mut self,
        method: &'static str,
        icon: OsdIconKind,
        label: &'static str,
        output_name: String,
        now: Instant,
    ) {
        self.show_osd(output_name, icon, "Multimedia", label, None, false, now);
        std::thread::spawn(move || {
            dispatch_mpris_method(method);
        });
    }

    /// Prepara el buffer Cairo y la máscara squircle del HUD OSD para el monitor indicado.
    pub fn prepare_frame(
        &mut self,
        output_name: &str,
        output_geo: Rectangle<i32, Logical>,
        usable_zone: Rectangle<i32, Logical>,
        scale: f64,
        is_dark: bool,
        now: Instant,
    ) -> Option<(Rectangle<i32, Physical>, Option<&[u8]>, (i32, i32), f32)> {
        let osd = self.active_osd.as_ref()?;
        if osd.output_name != output_name {
            return None;
        }

        let elapsed_since_update = now.saturating_duration_since(osd.updated_at);
        if elapsed_since_update >= OSD_VISIBLE_DURATION {
            return None;
        }

        // Fade-in suave al aparecer por primera vez y fade-out en los últimos 220ms
        let elapsed_since_first = now.saturating_duration_since(osd.first_shown_at);
        let fade_in = (elapsed_since_first.as_secs_f32() / OSD_FADE_IN_DURATION.as_secs_f32())
            .clamp(0.0, 1.0);
        let remaining = OSD_VISIBLE_DURATION.saturating_sub(elapsed_since_update);
        let fade_out =
            (remaining.as_secs_f32() / OSD_FADE_OUT_DURATION.as_secs_f32()).clamp(0.0, 1.0);
        let raw_t = fade_in.min(fade_out);
        let alpha = 1.0 - (1.0 - raw_t).powi(3);

        // Posicionar centrado horizontalmente en la parte inferior del área útil (encima del Dock)
        let local_usable_x = usable_zone.loc.x - output_geo.loc.x;
        let local_usable_y = usable_zone.loc.y - output_geo.loc.y;
        let x = local_usable_x + (usable_zone.size.w - OSD_WIDTH) / 2;
        let y = (local_usable_y + usable_zone.size.h - OSD_HEIGHT - 44).max(local_usable_y + 24);
        let local_rect = Rectangle::new(Point::from((x, y)), Size::from((OSD_WIDTH, OSD_HEIGHT)));
        let phys_rect: Rectangle<i32, Physical> = local_rect.to_physical_precise_round(scale);
        let phys_w = phys_rect.size.w.max(2);
        let phys_h = phys_rect.size.h.max(2);

        let glass_scale = malus_de_common::config::current_liquid_glass_scale();
        let key = OsdCacheKey {
            phys_w,
            phys_h,
            scale_milli: (scale * 1000.0).round() as i32,
            glass_milli: (glass_scale * 1000.0).round() as i32,
            is_dark,
            icon: osd.icon,
            title: osd.title.clone(),
            subtitle: osd.subtitle.clone(),
            level_permille: osd.level_pct.map(|v| (v * 10.0).round() as i32),
            muted_style: osd.muted_style,
        };

        let needs_rebuild = self.cache_key.as_ref() != Some(&key)
            || self.render_buffer.is_none()
            || self.mask_size != (phys_w, phys_h);

        if needs_rebuild {
            let osd_clone = osd.clone();
            self.rebuild_buffers(phys_w, phys_h, scale, is_dark, &osd_clone)?;
            self.cache_key = Some(key);
        }

        Some((
            phys_rect,
            Some(self.mask_buffer.as_slice()),
            self.mask_size,
            alpha,
        ))
    }

    pub fn create_render_element(
        &self,
        renderer: &mut GlesRenderer,
        loc_phys: Point<i32, Physical>,
        alpha: f32,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let buf = self.render_buffer.as_ref()?;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            loc_phys.to_f64(),
            buf,
            Some(alpha),
            None,
            None,
            Kind::Unspecified,
        )
        .ok()
    }

    fn rebuild_buffers(
        &mut self,
        phys_w: i32,
        phys_h: i32,
        scale: f64,
        is_dark: bool,
        osd: &ActiveOsd,
    ) -> Option<()> {
        let mut surface = ImageSurface::create(Format::ARgb32, phys_w, phys_h).ok()?;
        let mut mask_surface = ImageSurface::create(Format::ARgb32, phys_w, phys_h).ok()?;

        let cw = OSD_WIDTH as f64;
        let ch = OSD_HEIGHT as f64;

        {
            let ctx = Context::new(&surface).ok()?;
            ctx.scale(scale, scale);
            ctx.set_operator(Operator::Source);
            ctx.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            let _ = ctx.paint();
            ctx.set_operator(Operator::Over);

            // Fondo Liquid Glass del pill OSD
            let glass_scale = malus_de_common::config::current_liquid_glass_scale();
            let a_top =
                malus_de_common::config::scaled_liquid_glass_alpha(0.80, glass_scale, 0.18, 0.95);
            let a_bot =
                malus_de_common::config::scaled_liquid_glass_alpha(0.86, glass_scale, 0.22, 0.96);
            append_squircle_path(&ctx, 0.0, 0.0, cw, ch, OSD_CORNER_RADIUS, SQUIRCLE_EXPONENT);
            let bg_grad = LinearGradient::new(0.0, 0.0, 0.0, ch);
            if is_dark {
                bg_grad.add_color_stop_rgba(0.0, 0.16, 0.16, 0.19, a_top);
                bg_grad.add_color_stop_rgba(1.0, 0.11, 0.11, 0.14, a_bot);
            } else {
                bg_grad.add_color_stop_rgba(0.0, 0.98, 0.98, 0.99, a_top);
                bg_grad.add_color_stop_rgba(1.0, 0.93, 0.94, 0.96, a_bot);
            }
            let _ = ctx.set_source(&bg_grad);
            let _ = ctx.fill();

            // Borde especular interno
            append_squircle_path(
                &ctx,
                0.5,
                0.5,
                (cw - 1.0).max(2.0),
                (ch - 1.0).max(2.0),
                (OSD_CORNER_RADIUS - 0.5).max(1.0),
                SQUIRCLE_EXPONENT,
            );
            ctx.set_line_width(1.0);
            if is_dark {
                ctx.set_source_rgba(1.0, 1.0, 1.0, 0.18);
            } else {
                ctx.set_source_rgba(1.0, 1.0, 1.0, 0.78);
            }
            let _ = ctx.stroke();

            let (fg_r, fg_g, fg_b) = if is_dark {
                (0.96, 0.97, 0.99)
            } else {
                (0.10, 0.11, 0.14)
            };

            // Icono vectorial a la izquierda
            draw_osd_vector_icon(&ctx, osd.icon, 26.0, ch * 0.5, fg_r, fg_g, fg_b, osd.muted_style);

            // Título y porcentaje/subtítulo
            ctx.select_font_face(
                "Inter",
                cairo::FontSlant::Normal,
                cairo::FontWeight::Bold,
            );
            ctx.set_font_size(12.5);
            ctx.set_source_rgba(fg_r, fg_g, fg_b, 0.94);
            let text_y = if osd.level_pct.is_some() { 23.0 } else { 34.0 };
            ctx.move_to(50.0, text_y);
            let _ = ctx.show_text(&osd.title);

            ctx.select_font_face(
                "Inter",
                cairo::FontSlant::Normal,
                cairo::FontWeight::Normal,
            );
            ctx.set_font_size(12.0);
            ctx.set_source_rgba(fg_r, fg_g, fg_b, 0.68);
            if let Ok(ext) = ctx.text_extents(&osd.subtitle) {
                let sub_x = (cw - 16.0 - ext.width()).max(125.0);
                ctx.move_to(sub_x, text_y);
                let _ = ctx.show_text(&osd.subtitle);
            }

            // Barra de nivel segmentada/continua estilo macOS Tahoe
            if let Some(pct) = osd.level_pct {
                let bar_x = 50.0;
                let bar_y = 35.0;
                let bar_w = cw - bar_x - 16.0;
                let bar_h = 7.0;
                let bar_r = 3.5;

                // Track de fondo
                append_rounded_rect_path(&ctx, bar_x, bar_y, bar_w, bar_h, bar_r);
                if is_dark {
                    ctx.set_source_rgba(1.0, 1.0, 1.0, 0.14);
                } else {
                    ctx.set_source_rgba(0.08, 0.10, 0.14, 0.12);
                }
                let _ = ctx.fill();

                // Relleno activo
                let frac = (pct / 100.0).clamp(0.0, 1.0);
                if frac > 0.005 {
                    let fill_w = (bar_w * frac).max(bar_h);
                    append_rounded_rect_path(&ctx, bar_x, bar_y, fill_w, bar_h, bar_r);
                    if osd.muted_style {
                        ctx.set_source_rgba(0.95, 0.36, 0.34, 0.92);
                    } else {
                        let fill_grad = LinearGradient::new(bar_x, bar_y, bar_x + fill_w, bar_y);
                        fill_grad.add_color_stop_rgba(0.0, 0.18, 0.60, 1.0, 0.96);
                        fill_grad.add_color_stop_rgba(1.0, 0.32, 0.74, 1.0, 0.98);
                        let _ = ctx.set_source(&fill_grad);
                    }
                    let _ = ctx.fill();
                }
            }
        }

        {
            let mctx = Context::new(&mask_surface).ok()?;
            mctx.scale(scale, scale);
            mctx.set_operator(Operator::Source);
            mctx.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            let _ = mctx.paint();
            mctx.set_operator(Operator::Over);

            append_squircle_path(
                &mctx,
                0.0,
                0.0,
                cw,
                ch,
                OSD_CORNER_RADIUS,
                SQUIRCLE_EXPONENT,
            );
            mctx.set_source_rgba(1.0, 1.0, 1.0, 1.0);
            let _ = mctx.fill();
        }

        surface.flush();
        mask_surface.flush();

        let mask_data = mask_surface.data().ok()?;
        self.mask_buffer = mask_data.to_vec();
        self.mask_size = (phys_w, phys_h);

        let data = surface.data().ok()?;
        if self.render_buffer.is_none() || self.render_buffer_size != (phys_w, phys_h) {
            self.render_buffer = Some(MemoryRenderBuffer::new(
                Fourcc::Argb8888,
                Size::from((phys_w, phys_h)),
                1,
                smithay::utils::Transform::Normal,
                None,
            ));
            self.render_buffer_size = (phys_w, phys_h);
        }
        if let Some(ref mut buf) = self.render_buffer {
            let mut rctx = buf.render();
            let _ = rctx.draw(|dst| {
                if dst.len() == data.len() {
                    dst.copy_from_slice(&data);
                }
                Result::<_, ()>::Ok(vec![Rectangle::from_size((phys_w, phys_h).into())])
            });
        }

        Some(())
    }
}

fn volume_icon_for(vol: f64, muted: bool) -> OsdIconKind {
    if muted || vol < 0.5 {
        OsdIconKind::VolumeMuted
    } else if vol < 34.0 {
        OsdIconKind::VolumeLow
    } else if vol < 68.0 {
        OsdIconKind::VolumeMedium
    } else {
        OsdIconKind::VolumeHigh
    }
}

/// Consulta `wpctl get-volume <target>` y devuelve `(porcentaje_0_a_100, is_muted)`.
pub fn query_wpctl_volume(target: &str) -> Option<(f64, bool)> {
    let out = Command::new("wpctl")
        .args(["get-volume", target])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    parse_wpctl_volume_output(&s)
}

pub fn parse_wpctl_volume_output(output: &str) -> Option<(f64, bool)> {
    let is_muted = output.contains("[MUTED]");
    for part in output.split_whitespace() {
        let cleaned = part.trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.');
        if !cleaned.is_empty() {
            if let Ok(v) = cleaned.parse::<f64>() {
                return Some(((v * 100.0).clamp(0.0, 150.0), is_muted));
            }
        }
    }
    None
}

/// Busca reproductores MPRIS2 activos en el bus de sesión D-Bus y ejecuta el método solicitado.
fn dispatch_mpris_method(method: &str) {
    // 1. Si playerctl está disponible, usarlo directamente
    if Command::new("playerctl")
        .arg(match method {
            "PlayPause" => "play-pause",
            "Pause" => "pause",
            "Stop" => "stop",
            "Next" => "next",
            "Previous" => "previous",
            _ => "play-pause",
        })
        .status()
        .is_ok_and(|s| s.success())
    {
        return;
    }

    // 2. Descubrimiento nativo MPRIS2 sobre D-Bus vía zbus
    let _ = zbus::block_on(async move {
        let Ok(conn) = zbus::Connection::session().await else {
            return;
        };
        let Ok(reply) = conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "ListNames",
                &(),
            )
            .await
        else {
            return;
        };
        let Ok(names) = reply.body().deserialize::<Vec<String>>() else {
            return;
        };

        let players: Vec<String> = names
            .into_iter()
            .filter(|n| n.starts_with("org.mpris.MediaPlayer2."))
            .collect();

        // Priorizar el reproductor que esté actualmente en estado "Playing"
        let mut chosen_bus: Option<String> = None;
        for bus in &players {
            let is_playing = conn
                .call_method(
                    Some(bus.as_str()),
                    "/org/mpris/MediaPlayer2",
                    Some("org.freedesktop.DBus.Properties"),
                    "Get",
                    &("org.mpris.MediaPlayer2.Player", "PlaybackStatus"),
                )
                .await
                .ok()
                .and_then(|r| r.body().deserialize::<zbus::zvariant::OwnedValue>().ok())
                .is_some_and(|val| {
                    matches!(&*val, zbus::zvariant::Value::Str(s) if s.as_str() == "Playing")
                });
            if is_playing {
                chosen_bus = Some(bus.clone());
                break;
            }
        }
        if chosen_bus.is_none() {
            chosen_bus = players.into_iter().next();
        }

        if let Some(target_bus) = chosen_bus {
            info!("🎵 [MPRIS] Enviando '{}' a {}", method, target_bus);
            let _ = conn
                .call_method(
                    Some(target_bus.as_str()),
                    "/org/mpris/MediaPlayer2",
                    Some("org.mpris.MediaPlayer2.Player"),
                    method,
                    &(),
                )
                .await;
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_osd_vector_icon(
    ctx: &Context,
    icon: OsdIconKind,
    cx: f64,
    cy: f64,
    r: f64,
    g: f64,
    b: f64,
    muted_style: bool,
) {
    let _ = ctx.save();
    if muted_style {
        ctx.set_source_rgba(0.95, 0.36, 0.34, 0.95);
    } else {
        ctx.set_source_rgba(r, g, b, 0.92);
    }
    ctx.set_line_cap(cairo::LineCap::Round);
    ctx.set_line_join(cairo::LineJoin::Round);

    match icon {
        OsdIconKind::VolumeHigh
        | OsdIconKind::VolumeMedium
        | OsdIconKind::VolumeLow
        | OsdIconKind::VolumeMuted => {
            // Cono de altavoz
            ctx.new_sub_path();
            ctx.move_to(cx - 9.0, cy - 3.5);
            ctx.line_to(cx - 5.0, cy - 3.5);
            ctx.line_to(cx - 0.5, cy - 8.0);
            ctx.line_to(cx - 0.5, cy + 8.0);
            ctx.line_to(cx - 5.0, cy + 3.5);
            ctx.line_to(cx - 9.0, cy + 3.5);
            ctx.close_path();
            let _ = ctx.fill();

            ctx.set_line_width(1.8);
            if matches!(
                icon,
                OsdIconKind::VolumeLow | OsdIconKind::VolumeMedium | OsdIconKind::VolumeHigh
            ) {
                ctx.new_sub_path();
                ctx.arc(cx + 1.0, cy, 4.0, -0.65, 0.65);
                let _ = ctx.stroke();
            }
            if matches!(icon, OsdIconKind::VolumeMedium | OsdIconKind::VolumeHigh) {
                ctx.new_sub_path();
                ctx.arc(cx + 1.0, cy, 7.2, -0.70, 0.70);
                let _ = ctx.stroke();
            }
            if matches!(icon, OsdIconKind::VolumeHigh) {
                ctx.new_sub_path();
                ctx.arc(cx + 1.0, cy, 10.4, -0.75, 0.75);
                let _ = ctx.stroke();
            }
            if matches!(icon, OsdIconKind::VolumeMuted) {
                ctx.set_line_width(2.0);
                ctx.move_to(cx + 3.0, cy - 4.5);
                ctx.line_to(cx + 9.5, cy + 4.5);
                ctx.move_to(cx + 9.5, cy - 4.5);
                ctx.line_to(cx + 3.0, cy + 4.5);
                let _ = ctx.stroke();
            }
        }
        OsdIconKind::MicActive | OsdIconKind::MicMuted => {
            append_rounded_rect_path(ctx, cx - 3.5, cy - 9.0, 7.0, 11.5, 3.5);
            let _ = ctx.fill();
            ctx.set_line_width(1.8);
            ctx.new_sub_path();
            ctx.arc(cx, cy - 2.0, 6.2, 0.0, std::f64::consts::PI);
            let _ = ctx.stroke();
            ctx.move_to(cx, cy + 4.2);
            ctx.line_to(cx, cy + 8.5);
            ctx.move_to(cx - 4.0, cy + 8.5);
            ctx.line_to(cx + 4.0, cy + 8.5);
            let _ = ctx.stroke();
            if matches!(icon, OsdIconKind::MicMuted) {
                ctx.set_line_width(2.0);
                ctx.move_to(cx - 8.5, cy - 8.5);
                ctx.line_to(cx + 8.5, cy + 8.5);
                let _ = ctx.stroke();
            }
        }
        OsdIconKind::Brightness => {
            // Sol estilo macOS
            ctx.new_sub_path();
            ctx.arc(cx, cy, 4.6, 0.0, std::f64::consts::TAU);
            let _ = ctx.fill();
            ctx.set_line_width(1.9);
            for i in 0..8 {
                let angle = (i as f64) * std::f64::consts::TAU / 8.0;
                let r0 = 7.2;
                let r1 = 10.2;
                ctx.move_to(cx + angle.cos() * r0, cy + angle.sin() * r0);
                ctx.line_to(cx + angle.cos() * r1, cy + angle.sin() * r1);
            }
            let _ = ctx.stroke();
        }
        OsdIconKind::KbdBrightness => {
            // Rayos superiores + base de teclado
            ctx.set_line_width(1.8);
            ctx.move_to(cx - 8.5, cy + 5.5);
            ctx.line_to(cx + 8.5, cy + 5.5);
            let _ = ctx.stroke();
            for offset in [-6.0, 0.0, 6.0] {
                ctx.arc(cx + offset, cy + 2.2, 1.1, 0.0, std::f64::consts::TAU);
                let _ = ctx.fill();
            }
            let ray_angles = [-2.35_f64, -1.85, -1.57, -1.29, -0.79];
            for &ang in &ray_angles {
                ctx.move_to(cx + ang.cos() * 4.5, cy + 1.0 + ang.sin() * 4.5);
                ctx.line_to(cx + ang.cos() * 8.5, cy + 1.0 + ang.sin() * 8.5);
            }
            let _ = ctx.stroke();
        }
        OsdIconKind::MediaPlayPause => {
            ctx.new_sub_path();
            ctx.move_to(cx - 7.0, cy - 7.5);
            ctx.line_to(cx + 2.5, cy);
            ctx.line_to(cx - 7.0, cy + 7.5);
            ctx.close_path();
            let _ = ctx.fill();
            append_rounded_rect_path(ctx, cx + 4.5, cy - 7.0, 2.4, 14.0, 1.0);
            append_rounded_rect_path(ctx, cx + 8.5, cy - 7.0, 2.4, 14.0, 1.0);
            let _ = ctx.fill();
        }
        OsdIconKind::MediaNext => {
            ctx.new_sub_path();
            ctx.move_to(cx - 7.5, cy - 7.0);
            ctx.line_to(cx + 2.0, cy);
            ctx.line_to(cx - 7.5, cy + 7.0);
            ctx.close_path();
            let _ = ctx.fill();
            append_rounded_rect_path(ctx, cx + 4.0, cy - 7.0, 2.8, 14.0, 1.2);
            let _ = ctx.fill();
        }
        OsdIconKind::MediaPrev => {
            append_rounded_rect_path(ctx, cx - 6.8, cy - 7.0, 2.8, 14.0, 1.2);
            let _ = ctx.fill();
            ctx.new_sub_path();
            ctx.move_to(cx + 7.5, cy - 7.0);
            ctx.line_to(cx - 2.0, cy);
            ctx.line_to(cx + 7.5, cy + 7.0);
            ctx.close_path();
            let _ = ctx.fill();
        }
        OsdIconKind::MediaStop => {
            append_rounded_rect_path(ctx, cx - 6.5, cy - 6.5, 13.0, 13.0, 2.5);
            let _ = ctx.fill();
        }
        OsdIconKind::TouchpadOn | OsdIconKind::TouchpadOff => {
            ctx.set_line_width(1.7);
            append_rounded_rect_path(ctx, cx - 8.5, cy - 6.5, 17.0, 13.0, 2.8);
            let _ = ctx.stroke();
            ctx.move_to(cx - 8.0, cy + 2.0);
            ctx.line_to(cx + 8.0, cy + 2.0);
            ctx.move_to(cx, cy + 2.0);
            ctx.line_to(cx, cy + 6.5);
            let _ = ctx.stroke();
            if matches!(icon, OsdIconKind::TouchpadOff) {
                ctx.set_line_width(2.0);
                ctx.move_to(cx - 9.0, cy - 8.0);
                ctx.line_to(cx + 9.0, cy + 8.0);
                let _ = ctx.stroke();
            }
        }
    }

    let _ = ctx.restore();
}

fn append_rounded_rect_path(ctx: &Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    let r = radius.min(w * 0.5).min(h * 0.5).max(0.0);
    ctx.new_sub_path();
    ctx.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    ctx.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    ctx.arc(x + r, y + h - r, r, FRAC_PI_2, std::f64::consts::PI);
    ctx.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * FRAC_PI_2);
    ctx.close_path();
}

fn append_squircle_path(ctx: &Context, x: f64, y: f64, w: f64, h: f64, radius: f64, exponent: f64) {
    let r = radius.min(w * 0.5).min(h * 0.5).max(0.0);
    if r <= 0.5 {
        ctx.rectangle(x, y, w, h);
        return;
    }

    let steps = 14usize;
    let inv_n = 1.0 / exponent;
    ctx.new_sub_path();
    ctx.move_to(x + r, y);
    ctx.line_to(x + w - r, y);

    let corner_point = |cx: f64, cy: f64, sx: f64, sy: f64, t: f64| -> (f64, f64) {
        let angle = t * FRAC_PI_2;
        let dx = angle.cos().abs().powf(2.0 * inv_n) * r * sx;
        let dy = angle.sin().abs().powf(2.0 * inv_n) * r * sy;
        (cx + dx, cy + dy)
    };

    for i in 0..=steps {
        let (px, py) = corner_point(x + w - r, y + r, 1.0, -1.0, 1.0 - (i as f64 / steps as f64));
        ctx.line_to(px, py);
    }
    ctx.line_to(x + w, y + h - r);

    for i in 0..=steps {
        let (px, py) = corner_point(x + w - r, y + h - r, 1.0, 1.0, i as f64 / steps as f64);
        ctx.line_to(px, py);
    }
    ctx.line_to(x + r, y + h);

    for i in 0..=steps {
        let (px, py) = corner_point(x + r, y + h - r, -1.0, 1.0, 1.0 - (i as f64 / steps as f64));
        ctx.line_to(px, py);
    }
    ctx.line_to(x, y + r);

    for i in 0..=steps {
        let (px, py) = corner_point(x + r, y + r, -1.0, -1.0, i as f64 / steps as f64);
        ctx.line_to(px, py);
    }
    ctx.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_volume_and_brightness_keys_by_keysym_and_raw_keycode() {
        let no_mods = ModifiersState::default();
        let shift_mods = ModifiersState {
            shift: true,
            ..Default::default()
        };

        // Por keysym directo
        assert_eq!(
            detect_media_key_action(
                keysyms::KEY_XF86AudioRaiseVolume.into(),
                &[],
                0,
                &no_mods
            ),
            Some(MediaKeyAction::VolumeUp { fine: false })
        );
        assert_eq!(
            detect_media_key_action(
                keysyms::KEY_XF86AudioLowerVolume.into(),
                &[],
                0,
                &shift_mods
            ),
            Some(MediaKeyAction::VolumeDown { fine: true })
        );
        assert_eq!(
            detect_media_key_action(keysyms::KEY_XF86AudioMute.into(), &[], 0, &no_mods),
            Some(MediaKeyAction::VolumeMute)
        );
        assert_eq!(
            detect_media_key_action(
                keysyms::KEY_XF86MonBrightnessUp.into(),
                &[],
                0,
                &no_mods
            ),
            Some(MediaKeyAction::BrightnessUp { fine: false })
        );
        assert_eq!(
            detect_media_key_action(
                keysyms::KEY_XF86MonBrightnessDown.into(),
                &[],
                0,
                &shift_mods
            ),
            Some(MediaKeyAction::BrightnessDown { fine: true })
        );

        // Cuando Shift hace que modified_sym sea NoSymbol pero raw_syms conserva el XF86 keysym
        assert_eq!(
            detect_media_key_action(
                keysyms::KEY_NoSymbol.into(),
                &[keysyms::KEY_XF86MonBrightnessUp.into()],
                0,
                &shift_mods
            ),
            Some(MediaKeyAction::BrightnessUp { fine: true })
        );

        // Por raw_keycode (evdev + 8) incluso sin keysym
        assert_eq!(
            detect_media_key_action(keysyms::KEY_NoSymbol.into(), &[], 123, &no_mods),
            Some(MediaKeyAction::VolumeUp { fine: false })
        );
        assert_eq!(
            detect_media_key_action(keysyms::KEY_NoSymbol.into(), &[], 122, &no_mods),
            Some(MediaKeyAction::VolumeDown { fine: false })
        );
        assert_eq!(
            detect_media_key_action(keysyms::KEY_NoSymbol.into(), &[], 233, &no_mods),
            Some(MediaKeyAction::BrightnessUp { fine: false })
        );
        assert_eq!(
            detect_media_key_action(keysyms::KEY_NoSymbol.into(), &[], 232, &no_mods),
            Some(MediaKeyAction::BrightnessDown { fine: false })
        );
    }

    #[test]
    fn parse_wpctl_volume_handles_normal_and_muted_output() {
        assert_eq!(
            parse_wpctl_volume_output("Volume: 0.65\n"),
            Some((65.0, false))
        );
        assert_eq!(
            parse_wpctl_volume_output("Volume: 0.40 [MUTED]\n"),
            Some((40.0, true))
        );
        assert_eq!(
            parse_wpctl_volume_output("Volume: 0.00 [MUTED]"),
            Some((0.0, true))
        );
    }

    #[test]
    fn key_repeat_fires_after_initial_delay_and_stops_on_release() {
        let mut state = MediaKeysState::new();
        let t0 = Instant::now();
        let action = MediaKeyAction::VolumeUp { fine: false };
        state.register_key_press(123, action, t0);
        assert!(state.has_active_repeat());

        // Antes del retardo inicial no repite
        assert_eq!(state.tick(t0 + Duration::from_millis(100)), None);

        // Tras superar REPEAT_INITIAL_DELAY dispara repetición
        assert_eq!(
            state.tick(t0 + REPEAT_INITIAL_DELAY + Duration::from_millis(5)),
            Some(action)
        );

        // Al soltar la tecla se detiene la repetición
        state.handle_key_release(123);
        assert!(!state.has_active_repeat());
        assert_eq!(
            state.tick(t0 + REPEAT_INITIAL_DELAY + Duration::from_millis(200)),
            None
        );
    }
}
