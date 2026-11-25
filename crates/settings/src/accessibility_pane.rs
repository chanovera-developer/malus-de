use gtk4::prelude::*;
use gtk4::{
    Adjustment, Align, ApplicationWindow, Box as GtkBox, Button, CheckButton, DrawingArea, Image,
    Label, MenuButton, Orientation, Overlay, PolicyType, Popover, Scale, ScrolledWindow, Stack,
};
use malus_de_common::mac_switch::MacSwitch as Switch;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use crate::general_pane::make_pending_badge;
use crate::wifi_pane::{
    make_help_circle_button, make_popover_action_row, make_squircle_badge, make_squircle_button,
    make_squircle_colored_badge, wrap_squircle_card, WIFI_BADGE_SQUIRCLE_RADIUS,
    WIFI_CARD_SQUIRCLE_RADIUS,
};

const DARK_TOP: (f64, f64, f64) = (0.20, 0.20, 0.22);
const DARK_BOT: (f64, f64, f64) = (0.10, 0.10, 0.12);
const GRAY_TOP: (f64, f64, f64) = (0.61, 0.62, 0.65);
const GRAY_BOT: (f64, f64, f64) = (0.48, 0.49, 0.52);
const GREEN_TOP: (f64, f64, f64) = (0.32, 0.82, 0.44);
const GREEN_BOT: (f64, f64, f64) = (0.20, 0.68, 0.32);
const RED_TOP: (f64, f64, f64) = (0.96, 0.30, 0.34);
const RED_BOT: (f64, f64, f64) = (0.84, 0.18, 0.24);
const PURPLE_TOP: (f64, f64, f64) = (0.58, 0.38, 0.94);
const PURPLE_BOT: (f64, f64, f64) = (0.44, 0.26, 0.82);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessibilityConfig {
    // Visión
    pub voiceover_enabled: bool,
    pub zoom_shortcuts: bool,
    pub zoom_trackpad_gesture: bool,
    pub zoom_scroll_gesture: bool,
    pub zoom_modifier_idx: usize,
    pub zoom_style_idx: usize,
    pub hover_text: bool,
    pub hover_color: bool,
    pub hover_typing: bool,
    pub invert_colors: bool,
    pub invert_mode_smart: bool,
    pub increase_contrast: bool,
    pub reduce_transparency: bool,
    pub show_borders: bool,
    pub differentiate_without_color: bool,
    pub dim_flashing_lights: bool,
    pub show_window_title_icons: bool,
    pub display_contrast: f64,
    pub text_scale_factor: f64,
    pub menubar_size_large: bool,
    pub prefer_horizontal_text: bool,
    pub shake_mouse_to_locate: bool,
    pub cursor_size: i32,
    pub color_filters_enabled: bool,
    pub color_filter_type_idx: usize,
    pub color_filter_intensity: f64,
    // Movimiento
    pub reduce_motion: bool,
    pub autoplay_animated_images: bool,
    pub prefer_non_blinking_cursor: bool,
    // Lectura y voz
    pub accessibility_reader: bool,
    pub speak_selection: bool,
    pub speak_item_under_pointer: bool,
    pub speak_announcements: bool,
    pub speak_while_typing: bool,
    pub voice_lang_idx: usize,
    pub system_voice_idx: usize,
    pub detect_languages: bool,
    pub custom_pronunciations: bool,
    pub audio_descriptions: bool,
    // Físico y motriz
    pub voice_control_enabled: bool,
    pub voice_control_show_hints: bool,
    pub voice_control_play_sound: bool,
    pub voice_control_overlay_idx: usize,
    pub full_keyboard_access: bool,
    pub sticky_keys: bool,
    pub slow_keys: bool,
    pub onscreen_keyboard: bool,
    pub double_click_speed: f64,
    pub spring_loading: bool,
    pub spring_loading_speed: f64,
    pub ignore_trackpad_with_mouse: bool,
    pub mouse_keys: bool,
    pub alternate_pointer_actions: bool,
    pub head_pointer: bool,
    pub switch_control_enabled: bool,
    pub switch_auto_scan: bool,
    // Audición
    pub flash_screen_on_alert: bool,
    pub mono_audio: bool,
    pub background_sounds: bool,
    pub background_sound_idx: usize,
    pub background_sound_volume: f64,
    pub headphone_accommodations: bool,
    pub headphone_tune_idx: usize,
    pub headphone_boost_level: f64,
    pub caption_style_idx: usize,
    pub prefer_sdh_captions: bool,
    pub auto_captions_on_lang_mismatch: bool,
    pub show_captions_on_mute: bool,
    pub show_captions_on_skip_back: bool,
    pub apply_captions_to_all_apps: bool,
    pub live_captions_enabled: bool,
    pub live_captions_font_pt: u32,
    // Lectura / General
    pub live_speech_enabled: bool,
    pub siri_detect_atypical_speech: bool,
    pub siri_spoken_responses_idx: usize,
    pub shortcut_enabled_mask: u32,
    pub shortcut_speak_menu: bool,
}

impl Default for AccessibilityConfig {
    fn default() -> Self {
        Self {
            voiceover_enabled: false,
            zoom_shortcuts: false,
            zoom_trackpad_gesture: false,
            zoom_scroll_gesture: false,
            zoom_modifier_idx: 0,
            zoom_style_idx: 0,
            hover_text: false,
            hover_color: false,
            hover_typing: false,
            invert_colors: false,
            invert_mode_smart: true,
            increase_contrast: false,
            reduce_transparency: false,
            show_borders: false,
            differentiate_without_color: false,
            dim_flashing_lights: false,
            show_window_title_icons: false,
            display_contrast: 0.0,
            text_scale_factor: 1.0,
            menubar_size_large: false,
            prefer_horizontal_text: false,
            shake_mouse_to_locate: true,
            cursor_size: 24,
            color_filters_enabled: false,
            color_filter_type_idx: 0,
            color_filter_intensity: 0.8,
            reduce_motion: false,
            autoplay_animated_images: true,
            prefer_non_blinking_cursor: false,
            accessibility_reader: false,
            speak_selection: false,
            speak_item_under_pointer: false,
            speak_announcements: false,
            speak_while_typing: false,
            voice_lang_idx: 0,
            system_voice_idx: 0,
            detect_languages: true,
            custom_pronunciations: false,
            audio_descriptions: false,
            voice_control_enabled: false,
            voice_control_show_hints: true,
            voice_control_play_sound: false,
            voice_control_overlay_idx: 0,
            full_keyboard_access: false,
            sticky_keys: false,
            slow_keys: false,
            onscreen_keyboard: false,
            double_click_speed: 0.7,
            spring_loading: true,
            spring_loading_speed: 0.5,
            ignore_trackpad_with_mouse: false,
            mouse_keys: false,
            alternate_pointer_actions: false,
            head_pointer: false,
            switch_control_enabled: false,
            switch_auto_scan: true,
            flash_screen_on_alert: false,
            mono_audio: false,
            background_sounds: false,
            background_sound_idx: 0,
            background_sound_volume: 0.65,
            headphone_accommodations: false,
            headphone_tune_idx: 0,
            headphone_boost_level: 0.0,
            caption_style_idx: 0,
            prefer_sdh_captions: false,
            auto_captions_on_lang_mismatch: true,
            show_captions_on_mute: true,
            show_captions_on_skip_back: true,
            apply_captions_to_all_apps: false,
            live_captions_enabled: false,
            live_captions_font_pt: 32,
            live_speech_enabled: false,
            siri_detect_atypical_speech: false,
            siri_spoken_responses_idx: 0,
            shortcut_enabled_mask: 0xFFFF_FFFF,
            shortcut_speak_menu: true,
        }
    }
}

impl AccessibilityConfig {
    fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config/malus-de/accessibility-settings.toml")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(raw) = fs::read_to_string(path) {
            if let Ok(parsed) = toml::from_str::<Self>(&raw) {
                return parsed;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(raw) = toml::to_string_pretty(self) {
            let _ = fs::write(path, raw);
        }
    }
}

pub fn is_accessibility_subpanel(id: &str) -> bool {
    matches!(
        id,
        "accessibility"
            | "a11y-voiceover"
            | "a11y-zoom"
            | "a11y-hover-text"
            | "a11y-display"
            | "a11y-motion"
            | "a11y-spoken"
            | "a11y-audio-desc"
            | "a11y-voice-control"
            | "a11y-keyboard"
            | "a11y-pointer"
            | "a11y-switch-control"
            | "a11y-audio"
            | "a11y-captions"
            | "a11y-live-captions"
            | "a11y-name-rec"
            | "a11y-live-speech"
            | "a11y-personal-voice"
            | "a11y-vocal-shortcuts"
            | "a11y-siri"
            | "a11y-shortcut"
    )
}

pub fn accessibility_subpanel_title(id: &str) -> Option<&'static str> {
    match id {
        "accessibility" => Some("Accesibilidad"),
        "a11y-voiceover" => Some("VoiceOver"),
        "a11y-zoom" => Some("Zoom"),
        "a11y-hover-text" => Some("Texto flotante"),
        "a11y-display" => Some("Pantalla"),
        "a11y-motion" => Some("Movimiento"),
        "a11y-spoken" => Some("Lectura y voz"),
        "a11y-audio-desc" => Some("Descripciones de audio"),
        "a11y-voice-control" => Some("Control por voz"),
        "a11y-keyboard" => Some("Teclado"),
        "a11y-pointer" => Some("Control del puntero"),
        "a11y-switch-control" => Some("Control por botón"),
        "a11y-audio" => Some("Audio"),
        "a11y-captions" => Some("Subtítulos"),
        "a11y-live-captions" => Some("Subtítulos en vivo"),
        "a11y-name-rec" => Some("Reconocimiento de nombres"),
        "a11y-live-speech" => Some("Lectura en vivo"),
        "a11y-personal-voice" => Some("Voz personal"),
        "a11y-vocal-shortcuts" => Some("Atajos de voz"),
        "a11y-siri" => Some("Pip"),
        "a11y-shortcut" => Some("Atajo"),
        _ => None,
    }
}

/// Devuelve un cargador bajo demanda para `Accesibilidad` (`"accessibility"`) y sus 20 subvistas.
/// Cada página se construye solo la primera vez que se visita y queda conservada en el `Stack`.
pub fn create_accessibility_pane_loader(
    parent_window: &ApplicationWindow,
    stack: &Stack,
    navigate: Rc<dyn Fn(&str)>,
) -> Rc<dyn Fn(&str)> {
    let win = parent_window.clone();
    let stack_c = stack.clone();
    let cfg_cell: Rc<RefCell<Option<Rc<RefCell<AccessibilityConfig>>>>> =
        Rc::new(RefCell::new(None));

    Rc::new(move |target: &str| {
        if stack_c.child_by_name(target).is_some() {
            return;
        }
        let cfg = {
            let mut slot = cfg_cell.borrow_mut();
            slot.get_or_insert_with(|| Rc::new(RefCell::new(AccessibilityConfig::load())))
                .clone()
        };
        let pane: ScrolledWindow = match target {
            "accessibility" => build_a11y_hub_pane(&win, &navigate),
            "a11y-voiceover" => build_voiceover_pane(&cfg),
            "a11y-zoom" => build_zoom_pane(&cfg),
            "a11y-hover-text" => build_hover_text_pane(&cfg),
            "a11y-display" => build_display_pane(&cfg),
            "a11y-motion" => build_motion_pane(&cfg),
            "a11y-spoken" => build_spoken_content_pane(&cfg),
            "a11y-audio-desc" => build_audio_descriptions_pane(&cfg),
            "a11y-voice-control" => build_voice_control_pane(&cfg),
            "a11y-keyboard" => build_keyboard_a11y_pane(&cfg, &navigate),
            "a11y-pointer" => build_pointer_control_pane(&cfg),
            "a11y-switch-control" => build_switch_control_pane(&cfg, &navigate),
            "a11y-audio" => build_audio_a11y_pane(&cfg, &navigate),
            "a11y-captions" => build_captions_pane(&cfg),
            "a11y-live-captions" => build_live_captions_pane(&cfg),
            "a11y-name-rec" => build_name_recognition_pane(),
            "a11y-live-speech" => build_live_speech_pane(&cfg),
            "a11y-personal-voice" => build_personal_voice_pane(),
            "a11y-vocal-shortcuts" => build_vocal_shortcuts_pane(),
            "a11y-siri" => build_siri_a11y_pane(&cfg),
            "a11y-shortcut" => build_shortcut_a11y_pane(&cfg),
            _ => return,
        };
        crate::configure_right_pane_widget(pane.upcast_ref());
        stack_c.add_named(&pane, Some(target));
    })
}

fn make_scrolled_pane() -> (ScrolledWindow, GtkBox) {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();
    let pane = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .css_classes(["wifi-content-pane"])
        .valign(Align::Start)
        .vexpand(false)
        .build();
    scrolled.set_child(Some(&pane));
    (scrolled, pane)
}

fn hub_row(
    badge: Overlay,
    title: &str,
    pending: bool,
    target_id: &'static str,
    navigate: &Rc<dyn Fn(&str)>,
) -> Button {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    badge.set_valign(Align::Center);
    row.append(&badge);
    row.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .valign(Align::Center)
            .build(),
    );
    if pending {
        row.append(&make_pending_badge());
    }
    row.append(
        &Image::builder()
            .icon_name("go-next-symbolic")
            .pixel_size(14)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );
    let btn = Button::builder()
        .child(&row)
        .css_classes(["net-service-row"])
        .has_frame(false)
        .hexpand(true)
        .build();
    let nav = navigate.clone();
    btn.connect_clicked(move |_| nav(target_id));
    btn
}

// ============================================================================
// 1. Hub principal "Accesibilidad" (Accesibilidad1.png)
// ============================================================================

fn build_a11y_hub_pane(
    parent_window: &ApplicationWindow,
    navigate: &Rc<dyn Fn(&str)>,
) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();

    let hero_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let hero_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    hero_row.append(&make_squircle_badge(
        34,
        "preferences-desktop-accessibility-symbolic",
        18,
        WIFI_BADGE_SQUIRCLE_RADIUS,
    ));
    let hero_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    hero_text.append(
        &Label::builder()
            .label("Accesibilidad")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    let desc_lbl = Label::builder()
        .label("Personaliza las funciones de accesibilidad del equipo diseñadas para mejorar tu experiencia visual, auditiva, motriz, de lectura y cognición. <a href=\"details\">Obtén detalles...</a>")
        .use_markup(true)
        .css_classes(["card-section-sub", "bt-inline-desc"])
        .wrap(true)
        .xalign(0.0)
        .build();
    let win_c = parent_window.clone();
    desc_lbl.connect_activate_link(move |_, _| {
        let dlg = gtk4::Window::builder()
            .transient_for(&win_c)
            .destroy_with_parent(true)
            .modal(true)
            .resizable(false)
            .title("Accesibilidad en Malus DE")
            .default_width(420)
            .build();
        crate::attach_modal_scrim(&dlg, &win_c);
        let vb = GtkBox::builder().orientation(Orientation::Vertical).spacing(12).margin_top(18).margin_bottom(18).margin_start(20).margin_end(20).build();
        vb.append(&Label::builder().label("Malus DE integra los esquemas GSettings de accesibilidad de GNOME/GTK, el lector de pantalla Orca y el sintetizador speech-dispatcher. Las funciones exclusivas de macOS sin equivalente directo en Linux llevan la etiqueta «por implementar».").css_classes(["card-section-sub"]).wrap(true).xalign(0.0).build());
        let (ow, ob) = make_squircle_button("Listo", true, None);
        ow.set_halign(Align::End);
        let dc = dlg.clone();
        ob.connect_clicked(move |_| crate::close_attached_modal(&dc));
        vb.append(&ow);
        dlg.set_child(Some(&vb));
        crate::present_attached_modal(&dlg, &win_c);
        glib::Propagation::Stop
    });
    hero_text.append(&desc_lbl);
    hero_row.append(&hero_text);
    hero_card.append(&hero_row);
    pane.append(&wrap_squircle_card(&hero_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Visión
    pane.append(
        &Label::builder()
            .label("Visión")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(4)
            .build(),
    );
    let vision_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let vision_items: [(
        &str,
        (f64, f64, f64),
        (f64, f64, f64),
        &str,
        bool,
        &'static str,
    ); 7] = [
        (
            "preferences-desktop-accessibility-symbolic",
            DARK_TOP,
            DARK_BOT,
            "VoiceOver",
            false,
            "a11y-voiceover",
        ),
        (
            "edit-find-symbolic",
            DARK_TOP,
            DARK_BOT,
            "Zoom",
            false,
            "a11y-zoom",
        ),
        (
            "insert-text-symbolic",
            (0.259, 0.565, 0.976),
            (0.235, 0.518, 0.949),
            "Texto flotante",
            true,
            "a11y-hover-text",
        ),
        (
            "video-display-symbolic",
            (0.259, 0.565, 0.976),
            (0.235, 0.518, 0.949),
            "Pantalla",
            false,
            "a11y-display",
        ),
        (
            "media-playlist-repeat-symbolic",
            GREEN_TOP,
            GREEN_BOT,
            "Movimiento",
            false,
            "a11y-motion",
        ),
        (
            "user-available-symbolic",
            DARK_TOP,
            DARK_BOT,
            "Lectura y voz",
            false,
            "a11y-spoken",
        ),
        (
            "audio-speakers-symbolic",
            (0.259, 0.565, 0.976),
            (0.235, 0.518, 0.949),
            "Descripciones de audio",
            true,
            "a11y-audio-desc",
        ),
    ];
    append_hub_group(&vision_card, &vision_items, navigate);
    pane.append(&wrap_squircle_card(&vision_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Físico y motriz
    pane.append(
        &Label::builder()
            .label("Físico y motriz")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let motor_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let motor_items: [(
        &str,
        (f64, f64, f64),
        (f64, f64, f64),
        &str,
        bool,
        &'static str,
    ); 4] = [
        (
            "audio-input-microphone-symbolic",
            (0.259, 0.565, 0.976),
            (0.235, 0.518, 0.949),
            "Control por voz",
            true,
            "a11y-voice-control",
        ),
        (
            "input-keyboard-symbolic",
            GRAY_TOP,
            GRAY_BOT,
            "Teclado",
            false,
            "a11y-keyboard",
        ),
        (
            "input-mouse-symbolic",
            GRAY_TOP,
            GRAY_BOT,
            "Control del puntero",
            false,
            "a11y-pointer",
        ),
        (
            "view-grid-symbolic",
            DARK_TOP,
            DARK_BOT,
            "Control por botón",
            true,
            "a11y-switch-control",
        ),
    ];
    append_hub_group(&motor_card, &motor_items, navigate);
    pane.append(&wrap_squircle_card(&motor_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Audición
    pane.append(
        &Label::builder()
            .label("Audición")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let hearing_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let hearing_items: [(
        &str,
        (f64, f64, f64),
        (f64, f64, f64),
        &str,
        bool,
        &'static str,
    ); 4] = [
        (
            "audio-volume-high-symbolic",
            RED_TOP,
            RED_BOT,
            "Audio",
            false,
            "a11y-audio",
        ),
        (
            "media-view-subtitles-symbolic",
            (0.259, 0.565, 0.976),
            (0.235, 0.518, 0.949),
            "Subtítulos",
            false,
            "a11y-captions",
        ),
        (
            "user-available-symbolic",
            (0.259, 0.565, 0.976),
            (0.235, 0.518, 0.949),
            "Subtítulos en vivo",
            true,
            "a11y-live-captions",
        ),
        (
            "avatar-default-symbolic",
            (0.259, 0.565, 0.976),
            (0.235, 0.518, 0.949),
            "Reconocimiento de nombres",
            true,
            "a11y-name-rec",
        ),
    ];
    append_hub_group(&hearing_card, &hearing_items, navigate);
    pane.append(&wrap_squircle_card(
        &hearing_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // Lectura
    pane.append(
        &Label::builder()
            .label("Lectura")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let speech_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let speech_items: [(
        &str,
        (f64, f64, f64),
        (f64, f64, f64),
        &str,
        bool,
        &'static str,
    ); 3] = [
        (
            "input-keyboard-symbolic",
            DARK_TOP,
            DARK_BOT,
            "Lectura en vivo",
            false,
            "a11y-live-speech",
        ),
        (
            "avatar-default-symbolic",
            (0.259, 0.565, 0.976),
            (0.235, 0.518, 0.949),
            "Voz personal",
            true,
            "a11y-personal-voice",
        ),
        (
            "audio-input-microphone-symbolic",
            DARK_TOP,
            DARK_BOT,
            "Atajos de voz",
            true,
            "a11y-vocal-shortcuts",
        ),
    ];
    append_hub_group(&speech_card, &speech_items, navigate);
    pane.append(&wrap_squircle_card(&speech_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // General
    pane.append(
        &Label::builder()
            .label("General")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let gen_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let gen_items: [(
        &str,
        (f64, f64, f64),
        (f64, f64, f64),
        &str,
        bool,
        &'static str,
    ); 2] = [
        (
            "audio-input-microphone-symbolic",
            PURPLE_TOP,
            PURPLE_BOT,
            "Pip",
            true,
            "a11y-siri",
        ),
        (
            "preferences-desktop-accessibility-symbolic",
            (0.259, 0.565, 0.976),
            (0.235, 0.518, 0.949),
            "Atajo",
            false,
            "a11y-shortcut",
        ),
    ];
    append_hub_group(&gen_card, &gen_items, navigate);
    pane.append(&wrap_squircle_card(&gen_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .build();
    let (hw, _) = make_help_circle_button("Ayuda de Accesibilidad");
    footer.append(&hw);
    pane.append(&footer);

    scrolled
}

fn append_hub_group(
    card: &GtkBox,
    items: &[(
        &str,
        (f64, f64, f64),
        (f64, f64, f64),
        &str,
        bool,
        &'static str,
    )],
    navigate: &Rc<dyn Fn(&str)>,
) {
    for (idx, (ico, top, bot, title, pending, target)) in items.iter().enumerate() {
        if idx > 0 {
            card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        card.append(&hub_row(
            make_squircle_colored_badge(28, ico, 15, WIFI_BADGE_SQUIRCLE_RADIUS, *top, *bot),
            title,
            *pending,
            target,
            navigate,
        ));
    }
}

// ============================================================================
// 2. VoiceOver (Accesibilidad2.png) -> Orca + GSettings screen-reader-enabled
// ============================================================================

fn build_voiceover_pane(cfg: &Rc<RefCell<AccessibilityConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-card-compact", "wifi-header-section"])
        .build();
    row.append(&make_squircle_colored_badge(
        34,
        "preferences-desktop-accessibility-symbolic",
        18,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        DARK_TOP,
        DARK_BOT,
    ));
    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .build();
    col.append(
        &Label::builder()
            .label("VoiceOver (Lector de pantalla Orca)")
            .css_classes(["card-section-title"])
            .xalign(0.0)
            .build(),
    );
    col.append(
        &Label::builder()
            .label("Proporciona descripciones en voz alta y en braille de los elementos de la pantalla y permite controlar la computadora mediante el uso del teclado.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    col.append(
        &Label::builder()
            .label("Para activar o desactivar el lector rápidamente, usa Super+Alt+S.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let sw = Switch::builder()
        .active(cfg.borrow().voiceover_enabled)
        .css_classes(["macos-switch"])
        .valign(Align::Start)
        .build();
    {
        let c = cfg.clone();
        sw.connect_state_set(move |_, active| {
            c.borrow_mut().voiceover_enabled = active;
            c.borrow().save();
            set_gsetting_bool(
                "org.gnome.desktop.a11y.applications",
                "screen-reader-enabled",
                active,
            );
            if active {
                let _ = Command::new("orca").spawn();
            } else {
                let _ = Command::new("pkill").arg("-x").arg("orca").spawn();
            }
            glib::Propagation::Proceed
        });
    }
    row.append(&col);
    row.append(&*sw);
    card.append(&row);
    pane.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));

    let actions = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .build();
    let (tut_w, _) = make_squircle_button("Abrir tutorial de VoiceOver...", false, None);
    let (util_w, util_btn) = make_squircle_button("Abrir Utilidad VoiceOver...", false, None);
    util_btn.connect_clicked(|_| {
        let _ = Command::new("orca").arg("-s").spawn();
    });
    let (hw, _) = make_help_circle_button("Configuración de VoiceOver / Orca");
    actions.append(&tut_w);
    actions.append(&util_w);
    actions.append(&hw);
    pane.append(&actions);
    scrolled
}

// ============================================================================
// 3. Zoom (Accesibilidad3.png)
// ============================================================================

fn build_zoom_pane(cfg: &Rc<RefCell<AccessibilityConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let card = GtkBox::builder().orientation(Orientation::Vertical).build();

    card.append(&a11y_switch_row(
        "Usar atajos de teclado para cambiar el zoom",
        Some("Activar o desactivar zoom: ⌥⌘8\nAcercar: ⌥⌘=\nAlejar: ⌥⌘-"),
        cfg.borrow().zoom_shortcuts,
        false,
        {
            let c = cfg.clone();
            move |val| {
                c.borrow_mut().zoom_shortcuts = val;
                c.borrow().save();
                set_gsetting_bool(
                    "org.gnome.desktop.a11y.applications",
                    "screen-magnifier-enabled",
                    val,
                );
            }
        },
    ));
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card.append(&a11y_switch_row(
        "Usar gesto del trackpad para hacer zoom",
        Some("Toca dos veces con tres dedos para activar o desactivar el zoom."),
        cfg.borrow().zoom_trackpad_gesture,
        true,
        {
            let c = cfg.clone();
            move |val| {
                c.borrow_mut().zoom_trackpad_gesture = val;
                c.borrow().save();
            }
        },
    ));
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card.append(&a11y_switch_row(
        "Usar gesto de desplazamiento con teclas modificadoras para zoom",
        None,
        cfg.borrow().zoom_scroll_gesture,
        false,
        {
            let c = cfg.clone();
            move |val| {
                c.borrow_mut().zoom_scroll_gesture = val;
                c.borrow().save();
            }
        },
    ));
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card.append(&a11y_selector_row(
        "Tecla modificadora para gesto de desplazamiento",
        &["^ Control", "⌥ Opción (Alt)", "⌘ Comando (Super)"],
        cfg.borrow().zoom_modifier_idx,
        {
            let c = cfg.clone();
            move |idx| {
                c.borrow_mut().zoom_modifier_idx = idx;
                c.borrow().save();
            }
        },
    ));
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card.append(&a11y_selector_row(
        "Estilo de zoom",
        &[
            "Pantalla completa",
            "Pantalla dividida",
            "Imagen dentro de otra",
        ],
        cfg.borrow().zoom_style_idx,
        {
            let c = cfg.clone();
            move |idx| {
                c.borrow_mut().zoom_style_idx = idx;
                c.borrow().save();
            }
        },
    ));

    let adv_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .css_classes(["wifi-pref-row"])
        .build();
    let (adv_w, _) = make_squircle_button("Avanzado...", false, None);
    adv_row.append(&adv_w);
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card.append(&adv_row);
    pane.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

// ============================================================================
// 4. Texto flotante (Accesibilidad4.png)
// ============================================================================

fn build_hover_text_pane(cfg: &Rc<RefCell<AccessibilityConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    for (title, sub, val, key) in [
        ("Texto flotante", Some("Presiona ⌘ para mostrar una visualización con texto más grande del elemento debajo del puntero."), cfg.borrow().hover_text, 0u8),
        ("Color flotante", Some("Presiona ^ para describir el color que está debajo del puntero."), cfg.borrow().hover_color, 1u8),
        ("Escritura flotante", None, cfg.borrow().hover_typing, 2u8),
    ] {
        let c = GtkBox::builder().orientation(Orientation::Vertical).build();
        let c_ref = cfg.clone();
        c.append(&a11y_switch_row(title, sub, val, true, move |v| {
            match key {
                0 => c_ref.borrow_mut().hover_text = v,
                1 => c_ref.borrow_mut().hover_color = v,
                _ => c_ref.borrow_mut().hover_typing = v,
            }
            c_ref.borrow().save();
        }));
        pane.append(&wrap_squircle_card(&c, WIFI_CARD_SQUIRCLE_RADIUS));
    }
    scrolled
}

// ============================================================================
// 5. Pantalla (Accesibilidad5.png)
// ============================================================================

fn build_display_pane(cfg: &Rc<RefCell<AccessibilityConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();

    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c1.append(&a11y_switch_row(
        "Invertir colores",
        None,
        cfg.borrow().invert_colors,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().invert_colors = v;
                c.borrow().save();
                set_gsetting_bool("org.gnome.desktop.a11y.magnifier", "invert-lightness", v);
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let inv_mode = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    inv_mode.append(
        &Label::builder()
            .label("Modo de inversión de colores")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let rb_smart = CheckButton::with_label("Inteligente");
    let rb_classic = CheckButton::with_label("Clásico");
    rb_classic.set_group(Some(&rb_smart));
    rb_smart.set_active(true);
    inv_mode.append(&rb_smart);
    inv_mode.append(&rb_classic);
    c1.append(&inv_mode);
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    c1.append(&a11y_switch_row(
        "Aumentar contraste",
        None,
        cfg.borrow().increase_contrast,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().increase_contrast = v;
                c.borrow().save();
                set_gsetting_bool("org.gnome.desktop.a11y.interface", "high-contrast", v);
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_switch_row(
        "Reducir transparencia",
        None,
        cfg.borrow().reduce_transparency,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().reduce_transparency = v;
                c.borrow().save();
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_switch_row(
        "Mostrar bordes",
        None,
        cfg.borrow().show_borders,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().show_borders = v;
                c.borrow().save();
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_switch_row(
        "Diferenciar sin color",
        None,
        cfg.borrow().differentiate_without_color,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().differentiate_without_color = v;
                c.borrow().save();
                set_gsetting_bool("org.gnome.desktop.a11y.interface", "show-status-shapes", v);
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_switch_row(
        "Atenuar luces parpadeantes",
        Some("El contenido de vídeo que incluya luces parpadeantes o estroboscópicas se atenuará automáticamente."),
        cfg.borrow().dim_flashing_lights,
        true,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().dim_flashing_lights = v;
                c.borrow().save();
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_slider_row(
        "Contraste de la pantalla",
        "Normal",
        "Máximo",
        0.0,
        1.0,
        cfg.borrow().display_contrast,
        {
            let c = cfg.clone();
            move |val| {
                c.borrow_mut().display_contrast = val;
                c.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));

    // Texto
    pane.append(
        &Label::builder()
            .label("Texto")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let c2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c2.append(&a11y_selector_row(
        "Tamaño del texto",
        &["90%", "Predeterminado (100%)", "115%", "125%", "150%"],
        1,
        {
            let c = cfg.clone();
            move |idx| {
                let factor = match idx {
                    0 => 0.9,
                    2 => 1.15,
                    3 => 1.25,
                    4 => 1.5,
                    _ => 1.0,
                };
                c.borrow_mut().text_scale_factor = factor;
                c.borrow().save();
                let gs = gtk4::gio::Settings::new("org.gnome.desktop.interface");
                let _ = gs.set_double("text-scaling-factor", factor);
            }
        },
    ));
    c2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c2.append(&a11y_switch_row(
        "Preferir texto en horizontal",
        None,
        cfg.borrow().prefer_horizontal_text,
        true,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().prefer_horizontal_text = v;
                c.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c2, WIFI_CARD_SQUIRCLE_RADIUS));

    // Puntero
    pane.append(
        &Label::builder()
            .label("Puntero")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let c3 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c3.append(&a11y_switch_row(
        "Agitar puntero del mouse para encontrarlo",
        Some("Mueve rápidamente el puntero del mouse de un lado a otro para agrandarlo."),
        cfg.borrow().shake_mouse_to_locate,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().shake_mouse_to_locate = v;
                c.borrow().save();
            }
        },
    ));
    c3.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c3.append(&a11y_slider_row(
        "Tamaño del puntero",
        "Normal",
        "Grande",
        24.0,
        64.0,
        cfg.borrow().cursor_size as f64,
        {
            let c = cfg.clone();
            move |val| {
                let sz = val.round() as i32;
                c.borrow_mut().cursor_size = sz;
                c.borrow().save();
                let gs = gtk4::gio::Settings::new("org.gnome.desktop.interface");
                let _ = gs.set_int("cursor-size", sz);
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c3, WIFI_CARD_SQUIRCLE_RADIUS));

    // Filtros de colores con ilustración Cairo de 12 lápices de colores (Accesibilidad5.png)
    pane.append(
        &Label::builder()
            .label("Filtros de colores")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let c4 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let pencils = DrawingArea::new();
    pencils.set_content_height(110);
    pencils.set_hexpand(true);
    pencils.set_draw_func(|_, cr, w, h| {
        draw_color_pencils_preview(cr, w as f64, h as f64);
    });
    c4.append(&pencils);
    c4.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c4.append(&a11y_switch_row(
        "Filtros de colores",
        None,
        cfg.borrow().color_filters_enabled,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().color_filters_enabled = v;
                c.borrow().save();
                let gs = gtk4::gio::Settings::new("org.gnome.desktop.a11y.magnifier");
                let _ = gs.set_double("color-saturation", if v { 0.0 } else { 1.0 });
            }
        },
    ));
    c4.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c4.append(&a11y_selector_row(
        "Tipo de filtro",
        &[
            "Escala de grises",
            "Rojo/verde (Protanopía)",
            "Verde/rojo (Deuteranopía)",
            "Azul/amarillo (Tritanopía)",
        ],
        cfg.borrow().color_filter_type_idx,
        {
            let c = cfg.clone();
            move |idx| {
                c.borrow_mut().color_filter_type_idx = idx;
                c.borrow().save();
            }
        },
    ));
    c4.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c4.append(&a11y_slider_row(
        "Intensidad",
        "Baja",
        "Alta",
        0.0,
        1.0,
        cfg.borrow().color_filter_intensity,
        {
            let c = cfg.clone();
            move |val| {
                c.borrow_mut().color_filter_intensity = val;
                c.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c4, WIFI_CARD_SQUIRCLE_RADIUS));

    scrolled
}

fn draw_color_pencils_preview(cr: &gtk4::cairo::Context, w: f64, h: f64) {
    let colors: [(f64, f64, f64); 12] = [
        (0.89, 0.30, 0.24),
        (0.95, 0.56, 0.20),
        (0.96, 0.82, 0.22),
        (0.32, 0.78, 0.26),
        (0.16, 0.52, 0.26),
        (0.24, 0.68, 0.86),
        (0.18, 0.24, 0.88),
        (0.38, 0.20, 0.68),
        (0.78, 0.22, 0.82),
        (0.56, 0.14, 0.48),
        (0.38, 0.08, 0.26),
        (0.46, 0.28, 0.12),
    ];
    let count = colors.len() as f64;
    let pw = 18.0;
    let gap = 3.0;
    let total_w = count * pw + (count - 1.0) * gap;
    let start_x = ((w - total_w) * 0.5).max(10.0);
    let top_y = 14.0;
    let bot_y = h - 8.0;
    let tip_h = 24.0;

    for (i, &(r, g, b)) in colors.iter().enumerate() {
        let x = start_x + i as f64 * (pw + gap);
        // Cono de madera del lápiz
        cr.set_source_rgb(0.93, 0.80, 0.64);
        cr.move_to(x + pw * 0.5, top_y);
        cr.line_to(x + pw, top_y + tip_h);
        cr.line_to(x, top_y + tip_h);
        cr.close_path();
        let _ = cr.fill();

        // Punta coloreada
        cr.set_source_rgb(r, g, b);
        cr.move_to(x + pw * 0.5, top_y);
        cr.line_to(x + pw * 0.72, top_y + tip_h * 0.44);
        cr.line_to(x + pw * 0.28, top_y + tip_h * 0.44);
        cr.close_path();
        let _ = cr.fill();

        // Cuerpo cilíndrico del lápiz
        cr.set_source_rgb(r, g, b);
        cr.rectangle(x, top_y + tip_h, pw, (bot_y - (top_y + tip_h)).max(10.0));
        let _ = cr.fill();
    }
}

// ============================================================================
// 6. Movimiento (Accesibilidad6.png)
// ============================================================================

fn build_motion_pane(cfg: &Rc<RefCell<AccessibilityConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let card = GtkBox::builder().orientation(Orientation::Vertical).build();
    card.append(&a11y_switch_row(
        "Reducir movimiento",
        None,
        cfg.borrow().reduce_motion,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().reduce_motion = v;
                c.borrow().save();
                set_gsetting_bool("org.gnome.desktop.a11y.interface", "reduced-motion", v);
            }
        },
    ));
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card.append(&a11y_switch_row(
        "Reproducir automáticamente imágenes animadas",
        None,
        cfg.borrow().autoplay_animated_images,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().autoplay_animated_images = v;
                c.borrow().save();
            }
        },
    ));
    card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    card.append(&a11y_switch_row(
        "Preferir que el cursor no parpadee",
        None,
        cfg.borrow().prefer_non_blinking_cursor,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().prefer_non_blinking_cursor = v;
                c.borrow().save();
                set_gsetting_bool("org.gnome.desktop.interface", "cursor-blink", !v);
            }
        },
    ));
    pane.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

// ============================================================================
// 7. Lectura y voz (Accesibilidad7.png)
// ============================================================================

fn build_spoken_content_pane(cfg: &Rc<RefCell<AccessibilityConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();

    for (title, sub, val, pending) in [
        ("Lector de accesibilidad", "Lee o escucha texto en el lector de accesibilidad y personaliza los tipos de letra, el diseño y los colores de fondo.", cfg.borrow().accessibility_reader, true),
        ("Leer selección", "El texto seleccionado se leerá en voz alta mediante speech-dispatcher (spd-say).", cfg.borrow().speak_selection, false),
        ("Leer elemento situado bajo el puntero", "El sistema leerá las palabras que estén bajo el cursor.", cfg.borrow().speak_item_under_pointer, true),
        ("Leer avisos", "El sistema leerá en voz alta el texto de los cuadros de diálogo y avisos.", cfg.borrow().speak_announcements, false),
        ("Leer en voz alta al teclear", "El sistema leerá en voz alta las teclas que presiones y los caracteres que escribas.", cfg.borrow().speak_while_typing, false),
    ] {
        let c = GtkBox::builder().orientation(Orientation::Vertical).build();
        c.append(&a11y_switch_row(title, Some(sub), val, pending, |_| {}));
        pane.append(&wrap_squircle_card(&c, WIFI_CARD_SQUIRCLE_RADIUS));
    }

    let voice_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    voice_card.append(&a11y_selector_row(
        "Idioma de la voz del sistema",
        &[
            "Usar idioma del sistema (Español)",
            "Español (México)",
            "Español (España)",
            "English (US)",
        ],
        cfg.borrow().voice_lang_idx,
        {
            let c = cfg.clone();
            move |idx| {
                c.borrow_mut().voice_lang_idx = idx;
                c.borrow().save();
            }
        },
    ));
    voice_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    voice_card.append(&a11y_selector_row(
        "Voz del sistema",
        &[
            "Mónica (es-MX)",
            "Paulina (es-MX)",
            "Jorge (es-ES)",
            "eSpeak-NG Predeterminada",
        ],
        cfg.borrow().system_voice_idx,
        {
            let c = cfg.clone();
            move |idx| {
                c.borrow_mut().system_voice_idx = idx;
                c.borrow().save();
                let _ = Command::new("spd-say")
                    .args(["-l", "es", "Voz del sistema activada"])
                    .spawn();
            }
        },
    ));
    voice_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    voice_card.append(&a11y_switch_row(
        "Detectar idiomas",
        None,
        cfg.borrow().detect_languages,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().detect_languages = v;
                c.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&voice_card, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

// ============================================================================
// 8. Descripciones de audio (Accesibilidad8.png)
// ============================================================================

fn build_audio_descriptions_pane(cfg: &Rc<RefCell<AccessibilityConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let card = GtkBox::builder().orientation(Orientation::Vertical).build();
    card.append(&a11y_switch_row(
        "Reproducir descripciones de audio cuando estén disponibles",
        Some("Las descripciones de audio describen con voz la parte visual de los contenidos multimedia."),
        cfg.borrow().audio_descriptions,
        true,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().audio_descriptions = v;
                c.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

// ============================================================================
// 9. Control por voz (Accesibilidad9.png)
// ============================================================================

fn build_voice_control_pane(cfg: &Rc<RefCell<AccessibilityConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c1.append(&a11y_switch_row(
        "Control por voz",
        Some("Control por voz te permite usar tu voz para editar texto e interactuar con tu computadora."),
        cfg.borrow().voice_control_enabled,
        true,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().voice_control_enabled = v;
                c.borrow().save();
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_selector_row(
        "Idioma",
        &["Español (México)", "Español (España)", "English (US)"],
        0,
        |_| {},
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_selector_row(
        "Micrófono",
        &["Automático (Micrófono predeterminado PipeWire)"],
        0,
        |_| {},
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_switch_row(
        "Mostrar consejos",
        None,
        cfg.borrow().voice_control_show_hints,
        false,
        |_| {},
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_switch_row(
        "Reproducir sonido al reconocer un comando",
        None,
        cfg.borrow().voice_control_play_sound,
        false,
        |_| {},
    ));
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));

    let c2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c2.append(&a11y_selector_row(
        "Sobreposición",
        &[
            "Ninguna",
            "Números de elementos",
            "Nombres de elementos",
            "Cuadrícula numerada",
        ],
        0,
        |_| {},
    ));
    c2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c2.append(&a11y_switch_row(
        "Desvanecer sobreposición después de inactividad",
        None,
        false,
        false,
        |_| {},
    ));
    pane.append(&wrap_squircle_card(&c2, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

// ============================================================================
// 10. Teclado (Accesibilidad10.png) -> stickykeys, slowkeys, screen-keyboard
// ============================================================================

fn build_keyboard_a11y_pane(
    cfg: &Rc<RefCell<AccessibilityConfig>>,
    navigate: &Rc<dyn Fn(&str)>,
) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();

    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c1.append(&a11y_switch_row(
        "Acceso total con teclado",
        Some("El acceso total con teclado permite que se controle la computadora mediante un teclado. Usa Tab, Espacio y flechas para navegar por los controles."),
        cfg.borrow().full_keyboard_access,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().full_keyboard_access = v;
                c.borrow().save();
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    let kb_link_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    kb_link_row.append(
        &Label::builder()
            .label("Puedes establecer más atajos de teclado en la configuración de Teclado.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let (kb_w, kb_btn) = make_squircle_button("Configuración de Teclado...", false, None);
    let nav_kb = navigate.clone();
    kb_btn.connect_clicked(move |_| nav_kb("keyboard"));
    kb_link_row.append(&kb_w);
    c1.append(&kb_link_row);
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));

    let c2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c2.append(&a11y_switch_row(
        "Tecleado fácil (Sticky Keys)",
        Some("Permite utilizar las teclas modificadoras (Shift, Ctrl, Alt, Super) sin tener que mantenerlas presionadas."),
        cfg.borrow().sticky_keys,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().sticky_keys = v;
                c.borrow().save();
                set_gsetting_bool("org.gnome.desktop.a11y.keyboard", "stickykeys-enable", v);
            }
        },
    ));
    c2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c2.append(&a11y_switch_row(
        "Teclas lentas (Slow Keys)",
        Some("Establecen el intervalo de tiempo entre la presión de una tecla y su activación."),
        cfg.borrow().slow_keys,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().slow_keys = v;
                c.borrow().save();
                set_gsetting_bool("org.gnome.desktop.a11y.keyboard", "slowkeys-enable", v);
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c2, WIFI_CARD_SQUIRCLE_RADIUS));

    let c3 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c3.append(&a11y_switch_row(
        "Teclado de accesibilidad en pantalla",
        Some("Te deja escribir e interactuar sin usar un teclado físico."),
        cfg.borrow().onscreen_keyboard,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().onscreen_keyboard = v;
                c.borrow().save();
                set_gsetting_bool(
                    "org.gnome.desktop.a11y.applications",
                    "screen-keyboard-enabled",
                    v,
                );
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c3, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

// ============================================================================
// 11. Control del puntero (Accesibilidad11.png) -> mousekeys-enable
// ============================================================================

fn build_pointer_control_pane(cfg: &Rc<RefCell<AccessibilityConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    pane.append(
        &Label::builder()
            .label("Mouse y trackpad")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .build(),
    );
    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c1.append(&a11y_slider_row(
        "Velocidad de doble clic",
        "Lenta",
        "Rápida",
        0.1,
        1.0,
        cfg.borrow().double_click_speed,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().double_click_speed = v;
                c.borrow().save();
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_switch_row(
        "Apertura automática",
        None,
        cfg.borrow().spring_loading,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().spring_loading = v;
                c.borrow().save();
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_slider_row(
        "Velocidad de apertura automática",
        "Lenta",
        "Rápida",
        0.1,
        1.0,
        cfg.borrow().spring_loading_speed,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().spring_loading_speed = v;
                c.borrow().save();
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_switch_row(
        "Ignorar el trackpad integrado cuando haya un mouse o trackpad inalámbrico",
        None,
        cfg.borrow().ignore_trackpad_with_mouse,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().ignore_trackpad_with_mouse = v;
                c.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("Métodos de control alternativos")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let c2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c2.append(&a11y_switch_row(
        "Teclas para el mouse",
        Some("Permite que se controle el puntero usando las teclas del teclado o del teclado numérico."),
        cfg.borrow().mouse_keys,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().mouse_keys = v;
                c.borrow().save();
                set_gsetting_bool("org.gnome.desktop.a11y.keyboard", "mousekeys-enable", v);
            }
        },
    ));
    c2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c2.append(&a11y_switch_row(
        "Acciones del puntero alternativas",
        Some("Permite que se use un botón o expresión facial en lugar de los botones del mouse."),
        cfg.borrow().alternate_pointer_actions,
        true,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().alternate_pointer_actions = v;
                c.borrow().save();
            }
        },
    ));
    c2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c2.append(&a11y_switch_row(
        "Puntero con seguimiento de cabeza",
        Some("Permite que se controle el puntero usando el movimiento de tu cabeza capturado por la cámara."),
        cfg.borrow().head_pointer,
        true,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().head_pointer = v;
                c.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c2, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

// ============================================================================
// 12. Control por botón (Accesibilidad12.png)
// ============================================================================

fn build_switch_control_pane(
    cfg: &Rc<RefCell<AccessibilityConfig>>,
    navigate: &Rc<dyn Fn(&str)>,
) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c1.append(&a11y_switch_row(
        "Control por botón",
        Some("Permite que se controle la computadora usando uno o más botones dedicados, teclado o mando."),
        cfg.borrow().switch_control_enabled,
        true,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().switch_control_enabled = v;
                c.borrow().save();
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_switch_row(
        "Exploración automática",
        Some("Recorre secuencialmente los elementos de la interfaz."),
        cfg.borrow().switch_auto_scan,
        false,
        |_| {},
    ));
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));

    let c2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let r_kb = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    r_kb.append(
        &Label::builder()
            .label("En la configuración de Teclado puedes activar más funciones de escritura.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let (kb_w, kb_b) = make_squircle_button("Configuración de Teclado...", false, None);
    let nav = navigate.clone();
    kb_b.connect_clicked(move |_| nav("keyboard"));
    r_kb.append(&kb_w);
    c2.append(&r_kb);
    pane.append(&wrap_squircle_card(&c2, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

// ============================================================================
// 13. Audio (Accesibilidad13.png) -> visual-bell + enlace a Sonido
// ============================================================================

fn build_audio_a11y_pane(
    cfg: &Rc<RefCell<AccessibilityConfig>>,
    navigate: &Rc<dyn Fn(&str)>,
) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c1.append(&a11y_switch_row(
        "Activar el parpadeo de la pantalla al emitir sonidos de alerta",
        None,
        cfg.borrow().flash_screen_on_alert,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().flash_screen_on_alert = v;
                c.borrow().save();
                set_gsetting_bool("org.gnome.desktop.wm.preferences", "visual-bell", v);
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_switch_row(
        "Reproducir el audio estéreo como mono",
        None,
        cfg.borrow().mono_audio,
        true,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().mono_audio = v;
                c.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("Sonidos de fondo")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let c2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c2.append(&a11y_switch_row(
        "Sonidos de fondo",
        Some("Reproduce sonidos ambientales para enmascarar el ruido no deseado y ayudarte a concentrarte."),
        cfg.borrow().background_sounds,
        true,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().background_sounds = v;
                c.borrow().save();
            }
        },
    ));
    c2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c2.append(&a11y_selector_row(
        "Sonido de fondo",
        &[
            "Lluvia",
            "Océano",
            "Arroyo",
            "Ruido blanco",
            "Ruido equilibrado",
        ],
        cfg.borrow().background_sound_idx,
        {
            let c = cfg.clone();
            move |idx| {
                c.borrow_mut().background_sound_idx = idx;
                c.borrow().save();
            }
        },
    ));
    c2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c2.append(&a11y_slider_row(
        "Volumen de sonidos de fondo",
        "0%",
        "100%",
        0.0,
        1.0,
        cfg.borrow().background_sound_volume,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().background_sound_volume = v;
                c.borrow().save();
            }
        },
    ));
    c2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    let snd_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    snd_row.append(
        &Label::builder()
            .label("El volumen del sistema puede ajustarse en la configuración de Sonido.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let (sw, sb) = make_squircle_button("Configuración de Sonido...", false, None);
    let nav = navigate.clone();
    sb.connect_clicked(move |_| nav("sound"));
    snd_row.append(&sw);
    c2.append(&snd_row);
    pane.append(&wrap_squircle_card(&c2, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("Adaptaciones de audífonos")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let c3 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c3.append(&a11y_switch_row(
        "Adaptaciones de audífonos",
        Some("Personaliza el perfil de frecuencias de audio de tus audífonos."),
        cfg.borrow().headphone_accommodations,
        true,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().headphone_accommodations = v;
                c.borrow().save();
            }
        },
    ));
    c3.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c3.append(&a11y_selector_row(
        "Ajuste de audio",
        &["Tono balanceado", "Rango vocal", "Brillo"],
        cfg.borrow().headphone_tune_idx,
        |_| {},
    ));
    pane.append(&wrap_squircle_card(&c3, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

// ============================================================================
// 14. Subtítulos (Accesibilidad14.png) con vista previa Cairo en vivo
// ============================================================================

fn build_captions_pane(cfg: &Rc<RefCell<AccessibilityConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();

    let style_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let preview_area = DrawingArea::new();
    preview_area.set_content_height(100);
    preview_area.set_hexpand(true);
    let style_idx = Rc::new(Cell::new(cfg.borrow().caption_style_idx));
    {
        let st_c = style_idx.clone();
        preview_area.set_draw_func(move |_, cr, w, h| {
            draw_caption_preview(cr, w as f64, h as f64, st_c.get());
        });
    }
    style_card.append(&preview_area);
    style_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let styles_box = GtkBox::builder().orientation(Orientation::Vertical).build();
    let style_names = [
        "Fondo transparente",
        "Clásico",
        "Texto con contorno",
        "Texto grande",
    ];
    let rebuild_styles: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let reb_fn: Rc<dyn Fn()> = {
        let sbox = styles_box.clone();
        let st_c = style_idx.clone();
        let cfg_c = cfg.clone();
        let prev_c = preview_area;
        let self_ref = rebuild_styles.clone();
        Rc::new(move || {
            while let Some(c) = sbox.first_child() {
                sbox.remove(&c);
            }
            for (i, name) in style_names.iter().enumerate() {
                let r = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .css_classes(if st_c.get() == i {
                        vec!["fw-app-row", "fw-app-row-selected"]
                    } else {
                        vec!["fw-app-row"]
                    })
                    .build();
                r.append(
                    &Label::builder()
                        .label(*name)
                        .css_classes(["wifi-row-title"])
                        .xalign(0.0)
                        .hexpand(true)
                        .build(),
                );
                let click = gtk4::GestureClick::new();
                let sc = st_c.clone();
                let cc = cfg_c.clone();
                let pc = prev_c.clone();
                let rf = self_ref.clone();
                click.connect_released(move |_, _, _, _| {
                    sc.set(i);
                    cc.borrow_mut().caption_style_idx = i;
                    cc.borrow().save();
                    pc.queue_draw();
                    if let Some(f) = rf.borrow().as_ref() {
                        f();
                    }
                });
                r.add_controller(click);
                sbox.append(&r);
            }
        })
    };
    *rebuild_styles.borrow_mut() = Some(reb_fn.clone());
    reb_fn();
    style_card.append(&styles_box);
    pane.append(&wrap_squircle_card(&style_card, WIFI_CARD_SQUIRCLE_RADIUS));

    let opts_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    opts_card.append(&a11y_switch_row(
        "Preferir subtítulos opcionales y auxiliares para sordera (SDH)",
        Some("Si están disponibles, se utilizarán subtítulos SDH en lugar de los estándar."),
        cfg.borrow().prefer_sdh_captions,
        false,
        |_| {},
    ));
    opts_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    opts_card.append(&a11y_switch_row(
        "Mostrar automáticamente cuando los idiomas no coincidan",
        None,
        cfg.borrow().auto_captions_on_lang_mismatch,
        false,
        |_| {},
    ));
    opts_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    opts_card.append(&a11y_switch_row(
        "Mostrar al silenciar",
        None,
        cfg.borrow().show_captions_on_mute,
        false,
        |_| {},
    ));
    opts_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    opts_card.append(&a11y_switch_row(
        "Mostrar al retroceder",
        None,
        cfg.borrow().show_captions_on_skip_back,
        false,
        |_| {},
    ));
    opts_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    opts_card.append(&a11y_switch_row(
        "Aplicar a todas las apps",
        None,
        cfg.borrow().apply_captions_to_all_apps,
        false,
        |_| {},
    ));
    pane.append(&wrap_squircle_card(&opts_card, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

fn draw_caption_preview(cr: &gtk4::cairo::Context, w: f64, h: f64, style_idx: usize) {
    let grad = gtk4::cairo::LinearGradient::new(0.0, 0.0, w, h);
    grad.add_color_stop_rgb(0.0, 0.34, 0.44, 0.56);
    grad.add_color_stop_rgb(1.0, 0.78, 0.82, 0.88);
    let _ = cr.set_source(&grad);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();

    let font_sz = if style_idx == 3 { 16.5 } else { 13.5 };
    cr.select_font_face(
        "Sans",
        gtk4::cairo::FontSlant::Normal,
        gtk4::cairo::FontWeight::Bold,
    );
    cr.set_font_size(font_sz);
    let text = "Los subtítulos se verán así.";
    let ext = cr.text_extents(text).ok();
    let tw = ext.map(|e| e.width()).unwrap_or(180.0);
    let tx = (w - tw) * 0.5;
    let ty = h * 0.54;

    match style_idx {
        1 => {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.88);
            cr.rectangle(tx - 10.0, ty - font_sz - 4.0, tw + 20.0, font_sz + 10.0);
            let _ = cr.fill();
        }
        0 | 3 => {
            cr.set_source_rgba(0.12, 0.14, 0.18, 0.58);
            cr.rectangle(tx - 10.0, ty - font_sz - 4.0, tw + 20.0, font_sz + 10.0);
            let _ = cr.fill();
        }
        _ => {}
    }

    if style_idx == 2 {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.85);
        cr.move_to(tx + 1.0, ty + 1.0);
        let _ = cr.show_text(text);
    }
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.move_to(tx, ty);
    let _ = cr.show_text(text);
}

// ============================================================================
// 15–20. Subtítulos en vivo, Reconocimiento de nombres, Lectura en vivo,
//        Voz personal, Atajos de voz, Siri (Accesibilidad15..20.png)
// ============================================================================

fn build_live_captions_pane(cfg: &Rc<RefCell<AccessibilityConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c1.append(&a11y_switch_row(
        "Subtítulos en vivo",
        Some("Usa transcripción local en el dispositivo para mostrar subtítulos automáticamente en todas las apps."),
        cfg.borrow().live_captions_enabled,
        true,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().live_captions_enabled = v;
                c.borrow().save();
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_selector_row(
        "Familia tipográfica",
        &["Predeterminado (Sans)", "Monoespaciada", "Serif"],
        0,
        |_| {},
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_selector_row(
        "Idioma",
        &["español (México)", "español (España)", "English (US)"],
        0,
        |_| {},
    ));
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

fn build_name_recognition_pane() -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let r = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    r.append(&make_squircle_badge(
        30,
        "avatar-default-symbolic",
        16,
        WIFI_BADGE_SQUIRCLE_RADIUS,
    ));
    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    col.append(
        &Label::builder()
            .label("Reconocimiento de nombres")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    col.append(
        &Label::builder()
            .label("Enseña a tu equipo a reconocer tu nombre mediante el micrófono.")
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .build(),
    );
    r.append(&col);
    r.append(&make_pending_badge());
    let (cw, _) = make_squircle_button("Configurar", false, None);
    r.append(&cw);
    card.append(&r);
    pane.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

fn build_live_speech_pane(cfg: &Rc<RefCell<AccessibilityConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c1.append(&a11y_switch_row(
        "Lectura en vivo",
        Some("Permite ingresar texto para que se lea en voz alta mediante las bocinas o en llamadas."),
        cfg.borrow().live_speech_enabled,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().live_speech_enabled = v;
                c.borrow().save();
                if v {
                    let _ = Command::new("spd-say").args(["-l", "es", "Lectura en vivo activada"]).spawn();
                }
            }
        },
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_selector_row(
        "Idioma de la voz del sistema",
        &[
            "Usar idioma del sistema",
            "Español (México)",
            "English (US)",
        ],
        0,
        |_| {},
    ));
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    c1.append(&a11y_selector_row(
        "Voz",
        &["Usar la voz del sistema (speech-dispatcher)"],
        0,
        |_| {},
    ));
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));

    pane.append(
        &Label::builder()
            .label("Categorías")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );
    let c2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    for (idx, (ico, txt)) in [
        ("document-open-recent-symbolic", "Recientes"),
        ("user-bookmarks-symbolic", "Guardadas"),
    ]
    .into_iter()
    .enumerate()
    {
        if idx > 0 {
            c2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        let r = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .css_classes(["wifi-pref-row"])
            .build();
        r.append(&Image::builder().icon_name(ico).pixel_size(16).build());
        r.append(
            &Label::builder()
                .label(txt)
                .css_classes(["wifi-row-title"])
                .xalign(0.0)
                .hexpand(true)
                .build(),
        );
        c2.append(&r);
    }
    pane.append(&wrap_squircle_card(&c2, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

fn build_personal_voice_pane() -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let hdr = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    hdr.append(
        &Label::builder()
            .label("Voz personal")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .build(),
    );
    hdr.append(&make_pending_badge());
    pane.append(&hdr);
    pane.append(&Label::builder().label("Voz personal es una manera sencilla y segura de crear una voz sintetizada local que suene como tú.").css_classes(["card-section-sub"]).wrap(true).xalign(0.0).build());
    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c1.append(
        &Label::builder()
            .label("Sin voces personales")
            .css_classes(["vpn-empty-label"])
            .halign(Align::Center)
            .build(),
    );
    c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    let r = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .halign(Align::End)
        .css_classes(["wifi-pref-row"])
        .build();
    let (bw, _) = make_squircle_button("Crear una voz personal", false, None);
    r.append(&bw);
    c1.append(&r);
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

fn build_vocal_shortcuts_pane() -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let r = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    r.append(&make_squircle_colored_badge(
        30,
        "audio-input-microphone-symbolic",
        16,
        WIFI_BADGE_SQUIRCLE_RADIUS,
        DARK_TOP,
        DARK_BOT,
    ));
    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    col.append(
        &Label::builder()
            .label("Atajos de voz")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    col.append(&Label::builder().label("Enséñale a tu equipo a reconocer una frase personalizada que puedes decir para ejecutar rápidamente una acción.").css_classes(["card-section-sub"]).wrap(true).xalign(0.0).build());
    r.append(&col);
    r.append(&make_pending_badge());
    let (cw, _) = make_squircle_button("Configurar", false, None);
    r.append(&cw);
    card.append(&r);
    pane.append(&wrap_squircle_card(&card, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

fn build_siri_a11y_pane(cfg: &Rc<RefCell<AccessibilityConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();
    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c1.append(&a11y_switch_row(
        "Detectar variaciones del habla",
        Some("Permite que el asistente de voz detecte más patrones del habla a fin de mejorar el reconocimiento de voz."),
        cfg.borrow().siri_detect_atypical_speech,
        true,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().siri_detect_atypical_speech = v;
                c.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

// ============================================================================
// 21. Atajo de Accesibilidad (Accesibilidad21.png)
// ============================================================================

pub(crate) const SHORTCUT_FEATURE_NAMES: [&str; 20] = [
    "VoiceOver",
    "Zoom",
    "Texto flotante",
    "Escritura flotante",
    "Invertir colores de la pantalla",
    "Filtros de colores",
    "Lector de accesibilidad",
    "Tecleado fácil",
    "Teclas lentas",
    "Teclas para el mouse",
    "Acceso total con teclado",
    "Teclado de accesibilidad",
    "Aumentar el contraste",
    "Reducir transparencia",
    "Mostrar bordes",
    "Puntero con seguimiento de cabeza",
    "Control por voz",
    "Subtítulos en vivo",
    "Lectura en vivo",
    "Sonidos de fondo",
];

fn build_shortcut_a11y_pane(cfg: &Rc<RefCell<AccessibilityConfig>>) -> ScrolledWindow {
    let (scrolled, pane) = make_scrolled_pane();

    let c1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c1.append(
        &Label::builder()
            .label("Puedes activar o desactivar rápidamente las funciones de Accesibilidad al presionar ⌥⌘F5. Elige las funciones que quieres activar o desactivar:")
            .css_classes(["card-section-sub", "wifi-pref-row"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );

    let initial_mask = cfg.borrow().shortcut_enabled_mask;
    for (idx, name) in SHORTCUT_FEATURE_NAMES.iter().enumerate() {
        c1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .css_classes(["wifi-pref-row"])
            .build();
        let bit = 1u32 << idx;
        let chk = CheckButton::builder()
            .label(*name)
            .active((initial_mask & bit) != 0)
            .build();
        let cfg_c = cfg.clone();
        chk.connect_toggled(move |b| {
            let mut c = cfg_c.borrow_mut();
            if b.is_active() {
                c.shortcut_enabled_mask |= bit;
            } else {
                c.shortcut_enabled_mask &= !bit;
            }
            c.save();
        });
        row.append(&chk);
        c1.append(&row);
    }
    pane.append(&wrap_squircle_card(&c1, WIFI_CARD_SQUIRCLE_RADIUS));

    let c2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    c2.append(&a11y_switch_row(
        "Lectura",
        Some("Leer los elementos del menú de atajos de accesibilidad al navegar por ellos."),
        cfg.borrow().shortcut_speak_menu,
        false,
        {
            let c = cfg.clone();
            move |v| {
                c.borrow_mut().shortcut_speak_menu = v;
                c.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&c2, WIFI_CARD_SQUIRCLE_RADIUS));
    scrolled
}

// ============================================================================
// Constructores compartidos y sincronización con GSettings
// ============================================================================

fn set_gsetting_bool(schema: &str, key: &str, val: bool) {
    if let Some(source) = gtk4::gio::SettingsSchemaSource::default() {
        if source.lookup(schema, true).is_some() {
            let gs = gtk4::gio::Settings::new(schema);
            let _ = gs.set_boolean(key, val);
        }
    }
}

fn a11y_switch_row<F: Fn(bool) + 'static>(
    title: &str,
    subtitle: Option<&str>,
    initial: bool,
    pending: bool,
    on_toggle: F,
) -> GtkBox {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    col.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    if let Some(sub) = subtitle {
        col.append(
            &Label::builder()
                .label(sub)
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
    }
    row.append(&col);
    if pending {
        row.append(&make_pending_badge());
    }
    let sw = Switch::builder()
        .active(initial)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    sw.connect_state_set(move |_, active| {
        on_toggle(active);
        glib::Propagation::Proceed
    });
    row.append(&*sw);
    row
}

fn a11y_selector_row<F: Fn(usize) + 'static>(
    title: &str,
    options: &[&str],
    initial_idx: usize,
    on_select: F,
) -> GtkBox {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    row.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );

    let selected = Rc::new(Cell::new(initial_idx.min(options.len().saturating_sub(1))));
    let opts: Vec<String> = options.iter().map(|s| s.to_string()).collect();
    let cb = Rc::new(on_select);

    let inner = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .valign(Align::Center)
        .build();
    let cur_lbl = Label::builder()
        .label(opts.get(selected.get()).map(String::as_str).unwrap_or(""))
        .css_classes(["card-section-sub"])
        .build();
    inner.append(&cur_lbl);
    inner.append(
        &Image::builder()
            .icon_name("pan-down-symbolic")
            .pixel_size(12)
            .build(),
    );

    let btn = MenuButton::builder()
        .child(&inner)
        .has_frame(false)
        .always_show_arrow(false)
        .css_classes(["fw-rule-action-btn"])
        .valign(Align::Center)
        .build();
    let pop = Popover::builder()
        .has_arrow(false)
        .css_classes(["wifi-macos-popover"])
        .build();
    btn.set_popover(Some(&pop));

    let rebuild: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let reb_fn: Rc<dyn Fn()> = {
        let pop_c = pop;
        let sel_c = selected;
        let lbl_c = cur_lbl;
        let opts_c = opts;
        let cb_c = cb;
        let self_ref = rebuild.clone();
        Rc::new(move || {
            let mbox = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(1)
                .margin_top(4)
                .margin_bottom(4)
                .margin_start(4)
                .margin_end(4)
                .build();
            let cur = sel_c.get();
            for (idx, opt) in opts_c.iter().enumerate() {
                let (w, b) = make_popover_action_row(opt, true, (idx == cur).then_some("✓"));
                let pc = pop_c.clone();
                let sc = sel_c.clone();
                let lc = lbl_c.clone();
                let os = opt.clone();
                let cbc = cb_c.clone();
                let rfc = self_ref.clone();
                b.connect_clicked(move |_| {
                    pc.popdown();
                    sc.set(idx);
                    lc.set_label(&os);
                    if let Some(f) = rfc.borrow().as_ref() {
                        f();
                    }
                    cbc(idx);
                });
                mbox.append(&w);
            }
            pop_c.set_child(Some(&mbox));
        })
    };
    *rebuild.borrow_mut() = Some(reb_fn.clone());
    reb_fn();
    row.append(&btn);
    row
}

fn a11y_slider_row<F: Fn(f64) + 'static>(
    title: &str,
    min_label: &str,
    max_label: &str,
    min_val: f64,
    max_val: f64,
    initial: f64,
    on_change: F,
) -> GtkBox {
    let row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    row.append(
        &Label::builder()
            .label(title)
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let step = ((max_val - min_val) / 20.0).max(0.05);
    let adj = Adjustment::new(
        initial.clamp(min_val, max_val),
        min_val,
        max_val,
        step,
        step,
        0.0,
    );
    let slider = Scale::builder()
        .orientation(Orientation::Horizontal)
        .adjustment(&adj)
        .draw_value(false)
        .width_request(190)
        .css_classes(["bat-limit-slider"])
        .valign(Align::Center)
        .build();
    slider.add_mark(min_val, gtk4::PositionType::Bottom, Some(min_label));
    slider.add_mark(max_val, gtk4::PositionType::Bottom, Some(max_label));
    slider.connect_value_changed(move |sc| on_change(sc.value()));
    row.append(&slider);
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_all_accessibility_subpanels_and_titles() {
        assert!(is_accessibility_subpanel("accessibility"));
        assert!(is_accessibility_subpanel("a11y-voiceover"));
        assert!(is_accessibility_subpanel("a11y-shortcut"));
        assert!(!is_accessibility_subpanel("wifi"));
        assert_eq!(
            accessibility_subpanel_title("a11y-display"),
            Some("Pantalla")
        );
        assert_eq!(
            accessibility_subpanel_title("a11y-pointer"),
            Some("Control del puntero")
        );
        assert_eq!(SHORTCUT_FEATURE_NAMES.len(), 20);
    }
}
