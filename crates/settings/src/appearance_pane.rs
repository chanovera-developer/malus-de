use gtk4::prelude::*;
use gtk4::{
    Adjustment, Align, Box as GtkBox, Button, CheckButton, DrawingArea, GestureClick, Image, Label,
    MenuButton, Orientation, PolicyType, Popover, Scale, ScrolledWindow, Stack, Switch,
};
use malus_de_common::config::DesktopIconsConfig;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use crate::wifi_pane::{
    make_help_circle_button, make_popover_action_row, make_squircle_badge, make_squircle_button,
    wrap_squircle_card, WIFI_BADGE_SQUIRCLE_RADIUS, WIFI_CARD_SQUIRCLE_RADIUS,
};

pub const ACCENT_COLORS: [(&str, (f64, f64, f64)); 9] = [
    ("Multicolor", (0.04, 0.52, 1.0)),
    ("Azul", (0.0, 0.48, 1.0)),
    ("Morado", (0.58, 0.30, 0.86)),
    ("Rosa", (0.96, 0.31, 0.62)),
    ("Rojo", (0.88, 0.24, 0.24)),
    ("Naranja", (0.96, 0.52, 0.14)),
    ("Amarillo", (0.96, 0.78, 0.12)),
    ("Verde", (0.38, 0.73, 0.28)),
    ("Grafito", (0.55, 0.55, 0.58)),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearancePreferences {
    /// 0 = Claro, 1 = Obscuro, 2 = Automático
    pub mode_idx: usize,
    pub liquid_glass_opacity: f64,
    /// «Reducir transparencia»: superficies sólidas y sin blur.
    #[serde(default)]
    pub reduce_transparency: bool,
    pub accent_color_idx: usize,
    pub highlight_color_idx: usize,
    /// 0 = Predeterminado, 1 = Obscuro, 2 = Transparente, 3 = Teñido
    pub icon_widget_style_idx: usize,
    pub folder_color_idx: usize,
    /// 0 = Pequeños, 1 = Medianos, 2 = Grandes
    pub sidebar_icon_size_idx: usize,
    pub tint_windows_with_wallpaper: bool,
    /// 0 = Automáticamente, 1 = Durante el desplazamiento, 2 = Siempre
    pub show_scrollbars_idx: usize,
    /// 0 = Ir a la página siguiente, 1 = Ir a donde se hizo clic
    pub scrollbar_click_idx: usize,
}

impl Default for AppearancePreferences {
    fn default() -> Self {
        Self {
            mode_idx: if crate::is_dark_mode_active() { 1 } else { 0 },
            liquid_glass_opacity: 0.55,
            reduce_transparency: false,
            accent_color_idx: 0,
            highlight_color_idx: 0,
            icon_widget_style_idx: 0,
            folder_color_idx: 0,
            sidebar_icon_size_idx: 1,
            tint_windows_with_wallpaper: true,
            show_scrollbars_idx: 0,
            scrollbar_click_idx: 0,
        }
    }
}

impl AppearancePreferences {
    fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config/malus-de/appearance-settings.toml")
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

pub fn is_appearance_subpanel(id: &str) -> bool {
    matches!(id, "appearance" | "appearance-themes")
}

pub fn appearance_subpanel_title(id: &str) -> Option<&'static str> {
    match id {
        "appearance" => Some("Aspecto"),
        "appearance-themes" => Some("Temas"),
        _ => None,
    }
}

/// Registra el panel principal `Aspecto` (`"appearance"`, fiel a `assets/screenshots/Aspecto.png`)
/// y su subsección `Temas` (`"appearance-themes"`, con Tema de aplicaciones, Tema y estilo del puntero
/// y Superficie de escritorio en diseño Apple-like).
pub fn register_appearance_panes(stack: &Stack, navigate: Rc<dyn Fn(&str)>) -> Rc<dyn Fn()> {
    let prefs = Rc::new(RefCell::new(AppearancePreferences::load()));

    let (main_pane, refresh_main) = build_appearance_main_pane(&prefs, &navigate);
    stack.add_named(&main_pane, Some("appearance"));

    let (themes_pane, refresh_themes) = build_appearance_themes_subpane(&navigate);
    stack.add_named(&themes_pane, Some("appearance-themes"));

    Rc::new(move || {
        refresh_main();
        refresh_themes();
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

// ============================================================================
// 1. Vista principal "Aspecto" (assets/screenshots/Aspecto.png)
// ============================================================================

fn build_appearance_main_pane(
    prefs: &Rc<RefCell<AppearancePreferences>>,
    navigate: &Rc<dyn Fn(&str)>,
) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    // ------------------------------------------------------------------------
    // Tarjeta 1: Aspecto (Claro / Obscuro / Automático) + Liquid Glass
    // ------------------------------------------------------------------------
    let top_card = GtkBox::builder().orientation(Orientation::Vertical).build();

    let mode_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    mode_row.append(
        &Label::builder()
            .label("Aspecto")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .valign(Align::Start)
            .margin_top(6)
            .hexpand(true)
            .build(),
    );

    let thumbs_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .build();

    let selected_mode = Rc::new(Cell::new(if crate::is_dark_mode_active() {
        if prefs.borrow().mode_idx == 2 {
            2
        } else {
            1
        }
    } else if prefs.borrow().mode_idx == 2 {
        2
    } else {
        0
    }));

    let mut thumb_areas: Vec<(DrawingArea, Label)> = Vec::new();
    for (idx, title) in [(0usize, "Claro"), (1, "Obscuro"), (2, "Automático")] {
        let col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .halign(Align::Center)
            .build();
        let area = DrawingArea::new();
        area.set_content_width(68);
        area.set_content_height(44);
        crate::register_accent_draw_area(&area);
        let sel_c = selected_mode.clone();
        area.set_draw_func(move |_, cr, w, h| {
            draw_mode_thumbnail(cr, w as f64, h as f64, idx, sel_c.get() == idx);
        });
        let lbl = Label::builder()
            .label(title)
            .css_classes(if selected_mode.get() == idx {
                vec!["app-thumb-label", "app-thumb-label-active"]
            } else {
                vec!["app-thumb-label"]
            })
            .halign(Align::Center)
            .build();
        col.append(&area);
        col.append(&lbl);
        thumbs_box.append(&col);
        thumb_areas.push((area, lbl));
    }
    let thumb_areas_rc = Rc::new(thumb_areas);

    let update_mode_thumbs: Rc<dyn Fn()> = {
        let ta = thumb_areas_rc.clone();
        let sel = selected_mode.clone();
        Rc::new(move || {
            let cur = sel.get();
            for (i, (area, lbl)) in ta.iter().enumerate() {
                area.queue_draw();
                if i == cur {
                    lbl.add_css_class("app-thumb-label-active");
                } else {
                    lbl.remove_css_class("app-thumb-label-active");
                }
            }
        })
    };

    for (idx, (area, _)) in thumb_areas_rc.iter().enumerate() {
        let click = GestureClick::new();
        let sel_c = selected_mode.clone();
        let prefs_c = prefs.clone();
        let upd_c = update_mode_thumbs.clone();
        click.connect_released(move |_, _, _, _| {
            sel_c.set(idx);
            prefs_c.borrow_mut().mode_idx = idx;
            prefs_c.borrow().save();
            let to_dark = match idx {
                0 => false,
                1 => true,
                _ => {
                    let hour = glib::DateTime::now_local()
                        .map(|dt| dt.hour())
                        .unwrap_or(12);
                    !(7..19).contains(&hour)
                }
            };
            crate::set_system_appearance_mode(to_dark);
            upd_c();
        });
        area.add_controller(click);
    }

    mode_row.append(&thumbs_box);
    top_card.append(&mode_row);
    top_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Fila Liquid Glass
    let glass_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    glass_row.append(
        &Label::builder()
            .label("Liquid Glass")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .valign(Align::Start)
            .margin_top(4)
            .hexpand(true)
            .build(),
    );

    let glass_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(8)
        .halign(Align::End)
        .build();
    let glass_preview = DrawingArea::new();
    glass_preview.set_content_width(246);
    glass_preview.set_content_height(132);
    let glass_op = Rc::new(Cell::new(prefs.borrow().liquid_glass_opacity));
    {
        let op_c = glass_op.clone();
        glass_preview.set_draw_func(move |_, cr, w, h| {
            draw_liquid_glass_preview(cr, w as f64, h as f64, op_c.get());
        });
    }
    glass_col.append(&glass_preview);

    let glass_slider_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .valign(Align::Center)
        .build();
    glass_slider_row.append(
        &Image::builder()
            .icon_name("window-new-symbolic")
            .pixel_size(13)
            .css_classes(["bat-header-sub-icon"])
            .build(),
    );
    let glass_adj = Adjustment::new(glass_op.get(), 0.15, 0.95, 0.05, 0.05, 0.0);
    let glass_slider = Scale::builder()
        .orientation(Orientation::Horizontal)
        .adjustment(&glass_adj)
        .draw_value(false)
        .hexpand(true)
        .css_classes(["bat-limit-slider"])
        .build();
    let glass_preview_ref = glass_preview.clone();
    {
        let op_c = glass_op.clone();
        let prev_c = glass_preview;
        let prefs_c = prefs.clone();
        glass_slider.connect_value_changed(move |sc| {
            let v = sc.value().clamp(0.15, 0.95);
            op_c.set(v);
            prefs_c.borrow_mut().liquid_glass_opacity = v;
            prefs_c.borrow().save();
            prev_c.queue_draw();
            crate::apply_liquid_glass_opacity_css(v);
            malus_de_common::config::sync_system_appearance(crate::is_dark_mode_active());
            let _ = malus_de_common::ipc::IpcClient::send_command(
                malus_de_common::ipc::IpcMessage::LiquidGlassOpacityChanged { opacity: v },
            );
        });
    }
    glass_slider_row.append(&glass_slider);
    glass_slider_row.append(
        &Image::builder()
            .icon_name("view-dual-symbolic")
            .pixel_size(13)
            .css_classes(["bat-header-sub-icon"])
            .build(),
    );
    glass_col.append(&glass_slider_row);

    glass_row.append(&glass_col);
    top_card.append(&glass_row);

    // Fila «Reducir transparencia»: todo sólido y sin blur (sin detener ningún servicio).
    top_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
    glass_slider.set_sensitive(!prefs.borrow().reduce_transparency);
    {
        let prefs_c = prefs.clone();
        let slider_c = glass_slider.clone();
        let preview_c = glass_preview_ref.clone();
        let op_c = glass_op.clone();
        top_card.append(&apple_switch_row(
            "Reducir transparencia",
            Some("Muestra menús, Dock y barras laterales con fondos sólidos y desactiva el desenfoque."),
            prefs.borrow().reduce_transparency,
            move |active| {
                prefs_c.borrow_mut().reduce_transparency = active;
                prefs_c.borrow().save();
                slider_c.set_sensitive(!active);
                op_c.set(if active { 0.95 } else { prefs_c.borrow().liquid_glass_opacity });
                preview_c.queue_draw();
                let v = prefs_c.borrow().liquid_glass_opacity;
                crate::apply_liquid_glass_opacity_css(v);
                malus_de_common::config::sync_system_appearance(crate::is_dark_mode_active());
                let _ = malus_de_common::ipc::IpcClient::send_command(
                    malus_de_common::ipc::IpcMessage::LiquidGlassOpacityChanged { opacity: v },
                );
            },
        ));
    }
    pane.append(&wrap_squircle_card(&top_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ------------------------------------------------------------------------
    // Sección "Tema": Color, Resaltado, Estilo de íconos y widgets, Carpetas y Subsección Temas
    // ------------------------------------------------------------------------
    pane.append(
        &Label::builder()
            .label("Tema")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );

    let color_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let color_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    color_row.append(
        &Label::builder()
            .label("Color")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .valign(Align::Start)
            .margin_top(4)
            .hexpand(true)
            .build(),
    );

    let swatches_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .halign(Align::End)
        .build();
    let swatches_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(7)
        .build();
    let active_accent = Rc::new(Cell::new(prefs.borrow().accent_color_idx.min(8)));
    let accent_caption = Label::builder()
        .label(ACCENT_COLORS[active_accent.get()].0)
        .css_classes(["card-section-sub"])
        .xalign(0.0)
        .build();

    let mut swatch_areas: Vec<DrawingArea> = Vec::new();
    for idx in 0..ACCENT_COLORS.len() {
        let sw_area = DrawingArea::new();
        sw_area.set_content_width(22);
        sw_area.set_content_height(22);
        crate::register_accent_draw_area(&sw_area);
        let acc_c = active_accent.clone();
        sw_area.set_draw_func(move |_, cr, w, h| {
            draw_accent_swatch(cr, w as f64, h as f64, idx, acc_c.get() == idx);
        });
        swatches_row.append(&sw_area);
        swatch_areas.push(sw_area);
    }
    let swatch_areas_rc = Rc::new(swatch_areas);

    swatches_col.append(&swatches_row);
    swatches_col.append(&accent_caption);
    color_row.append(&swatches_col);
    color_card.append(&color_row);
    color_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let hl_names: Vec<&str> = std::iter::once("Automático")
        .chain(ACCENT_COLORS.iter().skip(1).map(|(n, _)| *n))
        .collect();
    color_card.append(&build_color_dot_selector_row(
        "Color de resaltado del texto",
        &hl_names,
        prefs.borrow().highlight_color_idx,
        {
            let p = prefs.clone();
            move |idx| {
                p.borrow_mut().highlight_color_idx = idx;
                p.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&color_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Tarjeta: Estilo de íconos y widgets + Color de carpetas
    let icon_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let icon_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    icon_row.append(
        &Label::builder()
            .label("Estilo de íconos y widgets")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .valign(Align::Start)
            .margin_top(6)
            .hexpand(true)
            .build(),
    );

    let icon_tiles_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    let active_icon_style = Rc::new(Cell::new(prefs.borrow().icon_widget_style_idx.min(3)));
    let mut icon_style_areas: Vec<(DrawingArea, Label)> = Vec::new();
    for (idx, name) in [
        (0usize, "Predeterminado"),
        (1, "Obscuro"),
        (2, "Transparente"),
        (3, "Teñido"),
    ] {
        let col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .halign(Align::Center)
            .build();
        let area = DrawingArea::new();
        area.set_content_width(44);
        area.set_content_height(44);
        crate::register_accent_draw_area(&area);
        let st_c = active_icon_style.clone();
        let acc_c = active_accent.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let tint = ACCENT_COLORS[acc_c.get()].1;
            draw_widget_style_tile(cr, w as f64, h as f64, idx, st_c.get() == idx, tint);
        });
        let lbl = Label::builder()
            .label(name)
            .css_classes(if active_icon_style.get() == idx {
                vec!["app-thumb-label", "app-thumb-label-active"]
            } else {
                vec!["app-thumb-label"]
            })
            .halign(Align::Center)
            .build();
        col.append(&area);
        col.append(&lbl);
        icon_tiles_box.append(&col);
        icon_style_areas.push((area, lbl));
    }
    let icon_style_areas_rc = Rc::new(icon_style_areas);

    for (idx, (area, _)) in icon_style_areas_rc.iter().enumerate() {
        let click = GestureClick::new();
        let st_c = active_icon_style.clone();
        let p_c = prefs.clone();
        let all_c = icon_style_areas_rc.clone();
        click.connect_released(move |_, _, _, _| {
            st_c.set(idx);
            p_c.borrow_mut().icon_widget_style_idx = idx;
            p_c.borrow().save();
            for (i, (a, l)) in all_c.iter().enumerate() {
                a.queue_draw();
                if i == idx {
                    l.add_css_class("app-thumb-label-active");
                } else {
                    l.remove_css_class("app-thumb-label-active");
                }
            }
        });
        area.add_controller(click);
    }

    // Conectar clic en muestras de color de acento (también repinta el ícono "Teñido")
    for (idx, area) in swatch_areas_rc.iter().enumerate() {
        let click = GestureClick::new();
        let acc_c = active_accent.clone();
        let cap_c = accent_caption.clone();
        let p_c = prefs.clone();
        let sw_all = swatch_areas_rc.clone();
        let ic_all = icon_style_areas_rc.clone();
        click.connect_released(move |_, _, _, _| {
            acc_c.set(idx);
            cap_c.set_label(ACCENT_COLORS[idx].0);
            p_c.borrow_mut().accent_color_idx = idx;
            p_c.borrow().save();
            crate::apply_accent_color_css(idx);
            malus_de_common::config::sync_system_appearance(crate::is_dark_mode_active());
            let _ = malus_de_common::ipc::IpcClient::send_command(
                malus_de_common::ipc::IpcMessage::AccentColorChanged { index: idx },
            );
            for a in sw_all.iter() {
                a.queue_draw();
            }
            for (ia, _) in ic_all.iter() {
                ia.queue_draw();
            }
        });
        area.add_controller(click);
    }

    icon_row.append(&icon_tiles_box);
    icon_card.append(&icon_row);
    icon_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    icon_card.append(&build_color_dot_selector_row(
        "Color de carpetas",
        &["Automático", "Azul clásico", "Color de énfasis", "Grafito"],
        prefs.borrow().folder_color_idx,
        {
            let p = prefs.clone();
            move |idx| {
                p.borrow_mut().folder_color_idx = idx;
                p.borrow().save();
            }
        },
    ));
    pane.append(&wrap_squircle_card(&icon_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Subsección "Temas" -> navega a "appearance-themes" (Tema de aplicaciones, Puntero y Superficie de escritorio)
    let themes_nav_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let themes_nav_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .build();
    let themes_badge = make_squircle_badge(
        30,
        "preferences-desktop-appearance-symbolic",
        16,
        WIFI_BADGE_SQUIRCLE_RADIUS,
    );
    themes_badge.set_valign(Align::Center);
    let themes_text = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(Align::Center)
        .build();
    themes_text.append(
        &Label::builder()
            .label("Temas")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    themes_text.append(
        &Label::builder()
            .label("Tema de aplicaciones, tema y estilo del puntero y superficie de escritorio")
            .css_classes(["card-section-sub"])
            .xalign(0.0)
            .build(),
    );
    themes_nav_row.append(&themes_badge);
    themes_nav_row.append(&themes_text);
    themes_nav_row.append(
        &Image::builder()
            .icon_name("go-next-symbolic")
            .pixel_size(14)
            .css_classes(["net-row-chevron"])
            .valign(Align::Center)
            .build(),
    );
    let themes_nav_btn = Button::builder()
        .child(&themes_nav_row)
        .css_classes(["net-service-row"])
        .has_frame(false)
        .hexpand(true)
        .build();
    let nav_themes = navigate.clone();
    themes_nav_btn.connect_clicked(move |_| nav_themes("appearance-themes"));
    themes_nav_card.append(&themes_nav_btn);
    pane.append(&wrap_squircle_card(
        &themes_nav_card,
        WIFI_CARD_SQUIRCLE_RADIUS,
    ));

    // ------------------------------------------------------------------------
    // Sección "Ventanas" (Aspecto.png)
    // ------------------------------------------------------------------------
    pane.append(
        &Label::builder()
            .label("Ventanas")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );

    let win_card1 = GtkBox::builder().orientation(Orientation::Vertical).build();
    win_card1.append(&build_simple_selector_row(
        "Íconos de la barra lateral",
        &["Pequeños", "Medianos", "Grandes"],
        prefs.borrow().sidebar_icon_size_idx,
        {
            let p = prefs.clone();
            move |idx| {
                p.borrow_mut().sidebar_icon_size_idx = idx;
                p.borrow().save();
            }
        },
    ));
    win_card1.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let tint_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    tint_row.append(
        &Label::builder()
            .label("Teñir el fondo de las ventanas con el color del fondo de pantalla")
            .css_classes(["wifi-row-title"])
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let tint_sw = Switch::builder()
        .active(prefs.borrow().tint_windows_with_wallpaper)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    {
        let p = prefs.clone();
        tint_sw.connect_state_set(move |_, active| {
            p.borrow_mut().tint_windows_with_wallpaper = active;
            p.borrow().save();
            glib::Propagation::Proceed
        });
    }
    tint_row.append(&tint_sw);
    win_card1.append(&tint_row);
    pane.append(&wrap_squircle_card(&win_card1, WIFI_CARD_SQUIRCLE_RADIUS));

    // Mostrar barras de desplazamiento + Hacer clic en la barra de desplazamiento
    let win_card2 = GtkBox::builder().orientation(Orientation::Vertical).build();
    let sb_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .css_classes(["wifi-pref-row"])
        .build();
    sb_box.append(
        &Label::builder()
            .label("Mostrar barras de desplazamiento")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    let rb_sb0 = CheckButton::with_label("Automáticamente según el mouse o trackpad");
    let rb_sb1 = CheckButton::with_label("Durante el desplazamiento");
    let rb_sb2 = CheckButton::with_label("Siempre");
    rb_sb1.set_group(Some(&rb_sb0));
    rb_sb2.set_group(Some(&rb_sb0));
    match prefs.borrow().show_scrollbars_idx {
        1 => rb_sb1.set_active(true),
        2 => rb_sb2.set_active(true),
        _ => rb_sb0.set_active(true),
    }
    for (idx, rb) in [(0usize, &rb_sb0), (1, &rb_sb1), (2, &rb_sb2)] {
        let p = prefs.clone();
        rb.connect_toggled(move |b| {
            if b.is_active() {
                p.borrow_mut().show_scrollbars_idx = idx;
                p.borrow().save();
                let gs = gtk4::gio::Settings::new("org.gnome.desktop.interface");
                let _ = gs.set_boolean("overlay-scrolling", idx != 2);
            }
        });
    }
    sb_box.append(&rb_sb0);
    sb_box.append(&rb_sb1);
    sb_box.append(&rb_sb2);
    win_card2.append(&sb_box);
    win_card2.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let sc_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .css_classes(["wifi-pref-row"])
        .build();
    sc_box.append(
        &Label::builder()
            .label("Hacer clic en la barra de desplazamiento para")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    let rb_sc0 = CheckButton::with_label("Ir a la página siguiente");
    let rb_sc1 = CheckButton::with_label("Ir a donde se hizo clic");
    rb_sc1.set_group(Some(&rb_sc0));
    if prefs.borrow().scrollbar_click_idx == 1 {
        rb_sc1.set_active(true);
    } else {
        rb_sc0.set_active(true);
    }
    for (idx, rb) in [(0usize, &rb_sc0), (1, &rb_sc1)] {
        let p = prefs.clone();
        rb.connect_toggled(move |b| {
            if b.is_active() {
                p.borrow_mut().scrollbar_click_idx = idx;
                p.borrow().save();
            }
        });
    }
    sc_box.append(&rb_sc0);
    sc_box.append(&rb_sc1);
    win_card2.append(&sc_box);
    pane.append(&wrap_squircle_card(&win_card2, WIFI_CARD_SQUIRCLE_RADIUS));

    // Pie inferior: [Más en la configuración de Accesibilidad...] [?]
    let footer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .halign(Align::End)
        .margin_top(2)
        .build();
    let (a11y_wrap, a11y_btn) =
        make_squircle_button("Más en la configuración de Accesibilidad...", false, None);
    let nav_a11y = navigate.clone();
    a11y_btn.connect_clicked(move |_| nav_a11y("a11y-display"));
    let (help_wrap, _) = make_help_circle_button("Ayuda sobre Aspecto");
    footer.append(&a11y_wrap);
    footer.append(&help_wrap);
    pane.append(&footer);

    let refresh: Rc<dyn Fn()> = {
        let sel = selected_mode;
        let prefs_c = prefs.clone();
        let upd = update_mode_thumbs;
        Rc::new(move || {
            if prefs_c.borrow().mode_idx != 2 {
                sel.set(if crate::is_dark_mode_active() { 1 } else { 0 });
            }
            upd();
        })
    };

    let ref_notify = refresh.clone();
    libadwaita::StyleManager::default().connect_dark_notify(move |_| {
        ref_notify();
    });

    (scrolled, refresh)
}

// ============================================================================
// 2. Subsección "Temas" ("appearance-themes") con diseño Apple-like:
//    - Tema de aplicaciones
//    - Tema y estilo del puntero
//    - Superficie de escritorio
// ============================================================================

fn build_appearance_themes_subpane(navigate: &Rc<dyn Fn(&str)>) -> (ScrolledWindow, Rc<dyn Fn()>) {
    let (scrolled, pane) = make_scrolled_pane();

    // ------------------------------------------------------------------------
    // A. Tema de aplicaciones (GTK3 / GTK4 / Libadwaita)
    // ------------------------------------------------------------------------
    pane.append(
        &Label::builder()
            .label("Tema de aplicaciones")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .build(),
    );

    let gtk_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let gtk_themes = crate::get_installed_gtk_themes();
    let gtk_check_labels: Rc<RefCell<Vec<(String, Label, DrawingArea)>>> =
        Rc::new(RefCell::new(Vec::new()));

    for (idx, (theme_id, label_txt, desc_txt)) in gtk_themes.iter().enumerate() {
        if idx > 0 {
            gtk_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        let is_dark_theme = theme_id.contains("Dark");
        let thumb = DrawingArea::new();
        thumb.set_content_width(48);
        thumb.set_content_height(32);
        crate::register_accent_draw_area(&thumb);
        let t_id_draw = theme_id.clone();
        thumb.set_draw_func(move |_, cr, w, h| {
            let active = crate::get_current_gtk_theme() == t_id_draw;
            draw_mode_thumbnail(
                cr,
                w as f64,
                h as f64,
                if is_dark_theme { 1 } else { 0 },
                active,
            );
        });

        let text_col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .valign(Align::Center)
            .build();
        text_col.append(
            &Label::builder()
                .label(label_txt)
                .css_classes(["wifi-row-title"])
                .xalign(0.0)
                .build(),
        );
        text_col.append(
            &Label::builder()
                .label(*desc_txt)
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );

        let check_lbl = Label::builder()
            .label("")
            .css_classes(["app-theme-active-tag"])
            .valign(Align::Center)
            .build();

        let row_inner = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .build();
        row_inner.append(&thumb);
        row_inner.append(&text_col);
        row_inner.append(&check_lbl);

        let row_btn = Button::builder()
            .child(&row_inner)
            .css_classes(["net-service-row"])
            .has_frame(false)
            .hexpand(true)
            .build();

        gtk_check_labels
            .borrow_mut()
            .push((theme_id.clone(), check_lbl, thumb));
        gtk_card.append(&row_btn);
    }
    pane.append(&wrap_squircle_card(&gtk_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ------------------------------------------------------------------------
    // B. Tema y estilo del puntero
    // ------------------------------------------------------------------------
    pane.append(
        &Label::builder()
            .label("Tema y estilo del puntero")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );

    let cursor_card = GtkBox::builder().orientation(Orientation::Vertical).build();
    let cursor_themes = crate::get_installed_cursor_themes();
    let (init_cursor_theme, init_cursor_size) = crate::get_current_cursor_config();
    let state_cursor_theme = Rc::new(RefCell::new(init_cursor_theme));
    let state_cursor_size = Rc::new(Cell::new(init_cursor_size));
    let cursor_rows: Rc<RefCell<Vec<(String, Label, DrawingArea, Button)>>> =
        Rc::new(RefCell::new(Vec::new()));

    for (idx, (theme_id, label_txt, desc_txt)) in cursor_themes.iter().enumerate() {
        if idx > 0 {
            cursor_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());
        }
        let is_white_variant = theme_id.contains("White") || theme_id == "Adwaita";
        let badge = DrawingArea::new();
        badge.set_content_width(34);
        badge.set_content_height(34);
        crate::register_accent_draw_area(&badge);
        let t_id_draw = theme_id.clone();
        let st_t_draw = state_cursor_theme.clone();
        badge.set_draw_func(move |_, cr, w, h| {
            draw_cursor_preview_badge(
                cr,
                w as f64,
                h as f64,
                is_white_variant,
                *st_t_draw.borrow() == t_id_draw,
            );
        });

        let text_col = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .valign(Align::Center)
            .build();
        text_col.append(
            &Label::builder()
                .label(label_txt)
                .css_classes(["wifi-row-title"])
                .xalign(0.0)
                .build(),
        );
        text_col.append(
            &Label::builder()
                .label(*desc_txt)
                .css_classes(["card-section-sub"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );

        let check_lbl = Label::builder()
            .label("")
            .css_classes(["app-theme-active-tag"])
            .valign(Align::Center)
            .build();

        let row_inner = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .build();
        row_inner.append(&badge);
        row_inner.append(&text_col);
        row_inner.append(&check_lbl);

        let row_btn = Button::builder()
            .child(&row_inner)
            .css_classes(["net-service-row"])
            .has_frame(false)
            .hexpand(true)
            .build();

        cursor_rows
            .borrow_mut()
            .push((theme_id.clone(), check_lbl, badge, row_btn.clone()));
        cursor_card.append(&row_btn);
    }

    cursor_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    // Selector segmentado Apple-like para Tamaño del puntero
    let size_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    size_row.append(
        &Label::builder()
            .label("Tamaño del puntero")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let size_seg = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(2)
        .css_classes(["bat-segmented-track"])
        .build();
    let size_btns: Rc<RefCell<Vec<(u32, Button)>>> = Rc::new(RefCell::new(Vec::new()));
    for (sz, txt) in [
        (24u32, "Estándar (24 px)"),
        (32, "Mediano (32 px)"),
        (48, "Grande (48 px)"),
    ] {
        let b = Button::builder()
            .label(txt)
            .has_frame(false)
            .css_classes(["bat-seg-tab"])
            .build();
        size_seg.append(&b);
        size_btns.borrow_mut().push((sz, b));
    }
    size_row.append(&size_seg);
    cursor_card.append(&size_row);
    pane.append(&wrap_squircle_card(&cursor_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // ------------------------------------------------------------------------
    // C. Superficie de escritorio (Archivos, Pilas, Cuadrícula y Fondo)
    // ------------------------------------------------------------------------
    pane.append(
        &Label::builder()
            .label("Superficie de escritorio")
            .css_classes(["wifi-section-title"])
            .xalign(0.0)
            .margin_top(6)
            .build(),
    );

    let desk_cfg = DesktopIconsConfig::load();
    let desk_card = GtkBox::builder().orientation(Orientation::Vertical).build();

    desk_card.append(&apple_switch_row(
        "Mostrar archivos en el escritorio",
        Some("Muestra los archivos, carpetas y accesos directos de ~/Escritorio."),
        desk_cfg.show_icons,
        |active| {
            let mut cfg = DesktopIconsConfig::load();
            cfg.show_icons = active;
            let _ = cfg.save();
        },
    ));
    desk_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    desk_card.append(&apple_switch_row(
        "Usar Pilas en el escritorio",
        Some("Agrupa automáticamente los elementos del escritorio por tipo de archivo."),
        desk_cfg.use_stacks,
        |active| {
            let mut cfg = DesktopIconsConfig::load();
            cfg.use_stacks = active;
            let _ = cfg.save();
        },
    ));
    desk_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    desk_card.append(&apple_switch_row(
        "Imantar íconos a la cuadrícula",
        Some("Alinea suavemente los elementos soltados en la cuadrícula del escritorio."),
        desk_cfg.snap_to_grid,
        |active| {
            let mut cfg = DesktopIconsConfig::load();
            cfg.snap_to_grid = active;
            let _ = cfg.save();
        },
    ));
    desk_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    desk_card.append(&build_simple_selector_row(
        "Alineación de la cuadrícula",
        &["Columna derecha (Estilo macOS)", "Columna izquierda"],
        if desk_cfg.align_left { 1 } else { 0 },
        |idx| {
            let mut cfg = DesktopIconsConfig::load();
            cfg.align_left = idx == 1;
            let _ = cfg.save();
        },
    ));
    desk_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let icon_sz_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let icon_sz_lbl = Label::builder()
        .label(&format!(
            "Tamaño de íconos del escritorio ({} px)",
            desk_cfg.clamped_icon_size()
        ))
        .css_classes(["wifi-row-title"])
        .xalign(0.0)
        .hexpand(true)
        .build();
    let icon_sz_adj = Adjustment::new(
        desk_cfg.clamped_icon_size() as f64,
        32.0,
        96.0,
        4.0,
        4.0,
        0.0,
    );
    let icon_sz_slider = Scale::builder()
        .orientation(Orientation::Horizontal)
        .adjustment(&icon_sz_adj)
        .draw_value(false)
        .width_request(190)
        .css_classes(["bat-limit-slider"])
        .valign(Align::Center)
        .build();
    icon_sz_slider.add_mark(32.0, gtk4::PositionType::Bottom, Some("32 px"));
    icon_sz_slider.add_mark(64.0, gtk4::PositionType::Bottom, Some("64 px"));
    icon_sz_slider.add_mark(96.0, gtk4::PositionType::Bottom, Some("96 px"));
    {
        let lbl_c = icon_sz_lbl.clone();
        icon_sz_slider.connect_value_changed(move |sc| {
            let val = sc.value().round() as u32;
            lbl_c.set_label(&format!("Tamaño de íconos del escritorio ({val} px)"));
            let mut cfg = DesktopIconsConfig::load();
            cfg.icon_size = val;
            let _ = cfg.save();
        });
    }
    icon_sz_row.append(&icon_sz_lbl);
    icon_sz_row.append(&icon_sz_slider);
    desk_card.append(&icon_sz_row);
    desk_card.append(&GtkBox::builder().css_classes(["wifi-hairline-sep"]).build());

    let wp_row = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(12)
        .css_classes(["wifi-pref-row"])
        .build();
    let wp_col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    wp_col.append(
        &Label::builder()
            .label("Fondo de pantalla del escritorio")
            .css_classes(["wifi-row-title"])
            .xalign(0.0)
            .build(),
    );
    wp_col.append(
        &Label::builder()
            .label("Personaliza el fondo con colecciones dinámicas, paisajes o imágenes propias.")
            .css_classes(["card-section-sub"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let (wp_btn_wrap, wp_btn) = make_squircle_button("Fondo de pantalla...", false, None);
    let nav_wp = navigate.clone();
    wp_btn.connect_clicked(move |_| nav_wp("wallpaper"));
    wp_row.append(&wp_col);
    wp_row.append(&wp_btn_wrap);
    desk_card.append(&wp_row);

    pane.append(&wrap_squircle_card(&desk_card, WIFI_CARD_SQUIRCLE_RADIUS));

    // Sincronización reactiva de selección de temas GTK y cursores
    let refresh_themes: Rc<dyn Fn()> = {
        let gtk_c = gtk_check_labels.clone();
        let cur_c = cursor_rows.clone();
        let sz_c = size_btns.clone();
        let st_t = state_cursor_theme.clone();
        let st_s = state_cursor_size.clone();
        Rc::new(move || {
            let cur_gtk = crate::get_current_gtk_theme();
            for (id, lbl, area) in gtk_c.borrow().iter() {
                lbl.set_label(if id == &cur_gtk { "✓ Activo" } else { "" });
                area.queue_draw();
            }

            let (cur_theme, cur_size) = crate::get_current_cursor_config();
            *st_t.borrow_mut() = cur_theme.clone();
            st_s.set(cur_size);
            for (id, lbl, area, _) in cur_c.borrow().iter() {
                lbl.set_label(if id == &cur_theme { "✓ Activo" } else { "" });
                area.queue_draw();
            }
            for (sz, btn) in sz_c.borrow().iter() {
                if *sz == cur_size {
                    btn.add_css_class("bat-seg-tab-active");
                } else {
                    btn.remove_css_class("bat-seg-tab-active");
                }
            }
        })
    };

    // Conectar clics de filas de tema GTK
    {
        let gtk_list = crate::get_installed_gtk_themes();
        let mut child = gtk_card.first_child();
        let mut idx = 0;
        while let Some(w) = child {
            if let Ok(btn) = w.clone().downcast::<Button>() {
                if let Some((t_id, _, _)) = gtk_list.get(idx) {
                    let target = t_id.clone();
                    let rf = refresh_themes.clone();
                    btn.connect_clicked(move |_| {
                        crate::set_gtk_theme(&target);
                        rf();
                    });
                }
                idx += 1;
            }
            child = w.next_sibling();
        }
    }

    // Conectar clics de filas de tema de cursor
    for (t_id, _, _, btn) in cursor_rows.borrow().iter() {
        let target = t_id.clone();
        let st_s = state_cursor_size.clone();
        let rf = refresh_themes.clone();
        btn.connect_clicked(move |_| {
            crate::set_cursor_theme_and_size(&target, st_s.get());
            rf();
        });
    }

    // Conectar clics de tamaño de cursor
    for (sz, btn) in size_btns.borrow().iter() {
        let target_sz = *sz;
        let st_t = state_cursor_theme.clone();
        let rf = refresh_themes.clone();
        btn.connect_clicked(move |_| {
            let cur_t = st_t.borrow().clone();
            crate::set_cursor_theme_and_size(&cur_t, target_sz);
            rf();
        });
    }

    refresh_themes();
    (scrolled, refresh_themes)
}

// ============================================================================
// Primitivas Cairo de previsualización (Aspecto.png y Temas)
// ============================================================================

const ASPECTO_CLARO_PNG: &[u8] = include_bytes!("../../../assets/screenshots/aspecto-claro.png");
const ASPECTO_OBSCURO_PNG: &[u8] =
    include_bytes!("../../../assets/screenshots/aspecto-obscuro.png");
const ASPECTO_AUTOMATICO_PNG: &[u8] =
    include_bytes!("../../../assets/screenshots/aspecto-automatico.png");
const ASPECTO_LIQUID_GLASS_WEBP: &[u8] =
    include_bytes!("../../../assets/screenshots/aspecto-liquid-glass.webp");

fn load_pixbuf_from_bytes(bytes: &[u8], image_type: &str) -> Option<gdk_pixbuf::Pixbuf> {
    let loader = gdk_pixbuf::PixbufLoader::with_type(image_type).ok()?;
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    loader.pixbuf()
}

fn with_liquid_glass_bg_pixbuf<F: FnOnce(&gdk_pixbuf::Pixbuf)>(f: F) -> bool {
    thread_local! {
        static BG_PIXBUF: RefCell<Option<gdk_pixbuf::Pixbuf>> = const { RefCell::new(None) };
    }
    BG_PIXBUF.with(|cell| {
        let mut opt = cell.borrow_mut();
        if opt.is_none() {
            *opt = load_pixbuf_from_bytes(ASPECTO_LIQUID_GLASS_WEBP, "webp");
        }
        if let Some(pb) = opt.as_ref() {
            f(pb);
            true
        } else {
            false
        }
    })
}

fn with_mode_thumbnail_pixbuf<F: FnOnce(&gdk_pixbuf::Pixbuf)>(mode_idx: usize, f: F) -> bool {
    thread_local! {
        static PIXBUFS: RefCell<[Option<gdk_pixbuf::Pixbuf>; 3]> =
            const { RefCell::new([None, None, None]) };
    }
    let idx = mode_idx.min(2);
    PIXBUFS.with(|cache| {
        let mut arr = cache.borrow_mut();
        if arr[idx].is_none() {
            let bytes = match idx {
                0 => ASPECTO_CLARO_PNG,
                1 => ASPECTO_OBSCURO_PNG,
                _ => ASPECTO_AUTOMATICO_PNG,
            };
            arr[idx] = load_pixbuf_from_bytes(bytes, "png");
        }
        if let Some(pb) = arr[idx].as_ref() {
            f(pb);
            true
        } else {
            false
        }
    })
}

fn draw_mode_thumbnail(cr: &gtk4::cairo::Context, w: f64, h: f64, mode_idx: usize, selected: bool) {
    let _ = cr.save();
    cr.translate(2.0, 2.0);
    let iw = (w - 4.0).max(10.0);
    let ih = (h - 4.0).max(10.0);
    crate::squircle_path(cr, iw, ih, 8.0);
    cr.clip();

    let painted = with_mode_thumbnail_pixbuf(mode_idx, |pb| {
        let sw = pb.width() as f64;
        let sh = pb.height() as f64;
        if sw > 0.0 && sh > 0.0 {
            let scale = (iw / sw).max(ih / sh);
            let dx = (iw - sw * scale) * 0.5;
            let dy = (ih - sh * scale) * 0.5;
            let _ = cr.save();
            cr.translate(dx, dy);
            cr.scale(scale, scale);
            cr.set_source_pixbuf(pb, 0.0, 0.0);
            cr.source().set_filter(gtk4::cairo::Filter::Bilinear);
            let _ = cr.paint();
            let _ = cr.restore();
        }
    });

    if !painted {
        match mode_idx {
            0 => cr.set_source_rgb(0.45, 0.72, 0.98),
            1 => cr.set_source_rgb(0.11, 0.16, 0.34),
            _ => {
                let g = gtk4::cairo::LinearGradient::new(0.0, 0.0, iw, 0.0);
                g.add_color_stop_rgb(0.49, 0.45, 0.72, 0.98);
                g.add_color_stop_rgb(0.51, 0.11, 0.16, 0.34);
                let _ = cr.set_source(&g);
            }
        }
        let _ = cr.paint();
    }
    cr.reset_clip();

    // Anillo de selección azul macOS
    crate::squircle_path(cr, iw, ih, 8.0);
    if selected {
        let (r, g, b) = crate::accent_color();
        cr.set_source_rgb(r, g, b);
        cr.set_line_width(2.2);
    } else {
        cr.set_source_rgba(0.5, 0.5, 0.54, 0.35);
        cr.set_line_width(1.0);
    }
    let _ = cr.stroke();
    let _ = cr.restore();
}

fn draw_liquid_glass_preview(cr: &gtk4::cairo::Context, w: f64, h: f64, glass_opacity: f64) {
    let _ = cr.save();
    crate::squircle_path(cr, w, h, 14.0);
    cr.clip();

    let painted_bg = with_liquid_glass_bg_pixbuf(|pb| {
        let sw = pb.width() as f64;
        let sh = pb.height() as f64;
        if sw > 0.0 && sh > 0.0 {
            let scale = (w / sw).max(h / sh);
            let dx = (w - sw * scale) * 0.5;
            let dy = (h - sh * scale) * 0.5;
            let _ = cr.save();
            cr.translate(dx, dy);
            cr.scale(scale, scale);
            cr.set_source_pixbuf(pb, 0.0, 0.0);
            cr.source().set_filter(gtk4::cairo::Filter::Bilinear);
            let _ = cr.paint();
            let _ = cr.restore();
        }
    });

    if !painted_bg {
        // Cielo y paisaje arquitectónico estilizado (respaldo)
        let sky = gtk4::cairo::LinearGradient::new(0.0, 0.0, 0.0, h * 0.72);
        sky.add_color_stop_rgb(0.0, 0.48, 0.69, 0.93);
        sky.add_color_stop_rgb(1.0, 0.78, 0.87, 0.96);
        let _ = cr.set_source(&sky);
        cr.rectangle(0.0, 0.0, w, h);
        let _ = cr.fill();

        cr.set_source_rgb(0.20, 0.38, 0.16);
        cr.arc(w * 0.22, h * 0.62, 46.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        cr.set_source_rgb(0.84, 0.86, 0.88);
        cr.arc(w * 0.95, h * 0.52, 64.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }

    // Píldoras superiores Liquid Glass (cambia su opacidad con el deslizador)
    let alpha = glass_opacity.clamp(0.18, 0.94);
    for (px, pw, txt) in [
        (10.0, 56.0, "↑   •••"),
        (72.0, 28.0, "⧉"),
        (106.0, w - 116.0, "🔍  Buscar"),
    ] {
        cr.set_source_rgba(0.96, 0.97, 0.99, alpha);
        let _ = cr.save();
        cr.translate(px, 10.0);
        crate::squircle_path(cr, pw, 24.0, 11.0);
        let _ = cr.restore();
        let _ = cr.fill_preserve();
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.45);
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        cr.set_source_rgba(0.12, 0.13, 0.16, 0.88);
        cr.set_font_size(10.5);
        cr.move_to(px + 9.0, 25.5);
        let _ = cr.show_text(txt);
    }

    // Faldón inferior translúcido
    cr.set_source_rgba(0.95, 0.95, 0.97, (alpha + 0.15).min(0.96));
    cr.rectangle(0.0, h - 26.0, w, 26.0);
    let _ = cr.fill();
    cr.set_source_rgba(0.10, 0.11, 0.13, 0.92);
    cr.select_font_face(
        "Sans",
        gtk4::cairo::FontSlant::Normal,
        gtk4::cairo::FontWeight::Bold,
    );
    cr.set_font_size(11.0);
    cr.move_to(14.0, h - 9.0);
    let _ = cr.show_text("Bienvenido a Malus DE");

    let _ = cr.restore();
}

fn draw_accent_swatch(cr: &gtk4::cairo::Context, w: f64, h: f64, idx: usize, selected: bool) {
    let cx = w * 0.5;
    let cy = h * 0.5;
    let r = (w.min(h) * 0.5) - 2.0;

    if idx == 0 {
        // Rueda multicolor
        let slices: [(f64, f64, f64); 6] = [
            (0.0, 0.48, 1.0),
            (0.62, 0.30, 0.88),
            (0.95, 0.28, 0.48),
            (0.98, 0.56, 0.12),
            (0.96, 0.82, 0.15),
            (0.32, 0.76, 0.34),
        ];
        let step = std::f64::consts::TAU / slices.len() as f64;
        for (i, &(cr_r, cr_g, cr_b)) in slices.iter().enumerate() {
            let a0 = i as f64 * step;
            let a1 = a0 + step;
            cr.set_source_rgb(cr_r, cr_g, cr_b);
            cr.move_to(cx, cy);
            cr.arc(cx, cy, r, a0, a1);
            cr.close_path();
            let _ = cr.fill();
        }
    } else {
        let (cr_r, cr_g, cr_b) = ACCENT_COLORS[idx].1;
        cr.set_source_rgb(cr_r, cr_g, cr_b);
        cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }

    if selected {
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.set_line_width(2.0);
        cr.arc(cx, cy, r - 2.2, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();

        let (r, g, b) = crate::accent_color();
        cr.set_source_rgb(r, g, b);
        cr.set_line_width(1.5);
        cr.arc(cx, cy, r + 1.0, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();
    }
}

fn draw_widget_style_tile(
    cr: &gtk4::cairo::Context,
    w: f64,
    h: f64,
    style_idx: usize,
    selected: bool,
    tint: (f64, f64, f64),
) {
    let _ = cr.save();
    cr.translate(3.0, 3.0);
    let iw = w - 6.0;
    let ih = h - 6.0;
    crate::squircle_path(cr, iw, ih, 11.0);

    match style_idx {
        0 => cr.set_source_rgb(0.24, 0.62, 0.98),
        1 => cr.set_source_rgb(0.14, 0.15, 0.18),
        2 => cr.set_source_rgba(0.55, 0.56, 0.60, 0.45),
        _ => cr.set_source_rgb(tint.0, tint.1, tint.2),
    }
    let _ = cr.fill_preserve();

    if selected {
        let (r, g, b) = crate::accent_color();
        cr.set_source_rgb(r, g, b);
        cr.set_line_width(2.2);
    } else {
        cr.set_source_rgba(0.5, 0.5, 0.54, 0.28);
        cr.set_line_width(1.0);
    }
    let _ = cr.stroke();

    // Sol + nube dentro del mosaico
    if style_idx == 2 {
        cr.set_source_rgba(0.95, 0.95, 0.97, 0.85);
    } else {
        cr.set_source_rgb(0.98, 0.78, 0.18);
    }
    cr.arc(iw * 0.40, ih * 0.44, 5.5, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();

    cr.set_source_rgba(1.0, 1.0, 1.0, 0.92);
    cr.arc(iw * 0.56, ih * 0.56, 5.5, 0.0, std::f64::consts::TAU);
    cr.arc(iw * 0.42, ih * 0.58, 4.5, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();

    let _ = cr.restore();
}

fn draw_cursor_preview_badge(
    cr: &gtk4::cairo::Context,
    w: f64,
    h: f64,
    white_fill: bool,
    active: bool,
) {
    let _ = cr.save();
    cr.translate(1.0, 1.0);
    crate::squircle_path(cr, w - 2.0, h - 2.0, 10.0);
    if active {
        let (r, g, b) = crate::accent_color();
        cr.set_source_rgba(r, g, b, 0.16);
    } else {
        cr.set_source_rgba(0.5, 0.5, 0.55, 0.12);
    }
    let _ = cr.fill_preserve();
    if active {
        let (r, g, b) = crate::accent_color();
        cr.set_source_rgb(r, g, b);
        cr.set_line_width(1.6);
    } else {
        cr.set_source_rgba(0.5, 0.5, 0.55, 0.25);
        cr.set_line_width(1.0);
    }
    let _ = cr.stroke();

    // Silueta de puntero estilo macOS
    cr.move_to(11.0, 7.0);
    cr.line_to(11.0, 23.0);
    cr.line_to(15.2, 19.2);
    cr.line_to(18.0, 25.0);
    cr.line_to(20.2, 24.0);
    cr.line_to(17.4, 18.2);
    cr.line_to(22.5, 18.2);
    cr.close_path();

    if white_fill {
        cr.set_source_rgb(0.97, 0.97, 0.99);
        let _ = cr.fill_preserve();
        cr.set_source_rgb(0.15, 0.16, 0.18);
    } else {
        cr.set_source_rgb(0.10, 0.11, 0.13);
        let _ = cr.fill_preserve();
        cr.set_source_rgb(1.0, 1.0, 1.0);
    }
    cr.set_line_width(1.3);
    let _ = cr.stroke();
    let _ = cr.restore();
}

// ============================================================================
// Constructores Apple-like reutilizables
// ============================================================================

fn apple_switch_row<F: Fn(bool) + 'static>(
    title: &str,
    subtitle: Option<&str>,
    initial: bool,
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
    let sw = Switch::builder()
        .active(initial)
        .css_classes(["macos-switch"])
        .valign(Align::Center)
        .build();
    sw.connect_state_set(move |_, active| {
        on_toggle(active);
        glib::Propagation::Proceed
    });
    row.append(&col);
    row.append(&sw);
    row
}

fn build_color_dot_selector_row<F: Fn(usize) + 'static>(
    title: &str,
    options: &[&str],
    initial_idx: usize,
    on_select: F,
) -> GtkBox {
    let row = build_simple_selector_row(title, options, initial_idx, on_select);
    row
}

fn build_simple_selector_row<F: Fn(usize) + 'static>(
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
    let dot = Label::builder()
        .label("●")
        .css_classes(["net-dot-blue"])
        .build();
    let cur_lbl = Label::builder()
        .label(opts.get(selected.get()).map(String::as_str).unwrap_or(""))
        .css_classes(["card-section-sub"])
        .build();
    inner.append(&dot);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_appearance_and_themes_subpanels() {
        assert!(is_appearance_subpanel("appearance"));
        assert!(is_appearance_subpanel("appearance-themes"));
        assert!(!is_appearance_subpanel("wifi"));
        assert_eq!(appearance_subpanel_title("appearance"), Some("Aspecto"));
        assert_eq!(
            appearance_subpanel_title("appearance-themes"),
            Some("Temas")
        );
        assert_eq!(ACCENT_COLORS.len(), 9);
    }
}
