use gio::prelude::*;
use gdk_pixbuf::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, DrawingArea,
    EventControllerMotion, GestureClick, Image, Label, Orientation, Overlay, Picture, Popover,
    Separator, Window,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use libadwaita as adw;
use malus_de_common::config::{DockAppConfig, DockConfig, DockIconAppearance};
use malus_de_common::ipc::{ipc_socket_path, send_ipc_message, IpcMessage, MinimizedWindowInfo};
use malus_de_common::logging::init_logging;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;
use tracing::{error, info, warn};

fn dock_headroom_height(config: &DockConfig) -> i32 {
    config.headroom_height().min(i32::MAX as u32) as i32
}

const DOCK_CSS: &str = r#"
window.malus-dock-window,
window.malus-dock-window.background,
window.malus-dock-window decoration {
    background-color: transparent;
    background: transparent;
    border: none;
    outline: none;
    box-shadow: none;
}

.dock-container {
    background-color: alpha(@window_bg_color, 0.42);
    border: 1px solid alpha(@window_fg_color, 0.12);
    border-top: none;
    /* El compositor recorta esta superficie con la máscara squircle. */
    border-radius: 0;
    padding: 6px 10px 0;
    overflow: visible;
}

@media (prefers-color-scheme: dark) {
    .dock-container {
        border-color: alpha(@window_fg_color, 0.14);
    }
}

.dock-item-box {
    padding: 0px 2px;
}

.dock-bounce-target {
    transition: transform 260ms cubic-bezier(0.22, 0.61, 0.36, 1);
}

.dock-bounce-target.dock-bouncing {
    transition-duration: 280ms;
    transition-timing-function: cubic-bezier(0.16, 0.76, 0.32, 1);
    transform: translateY(-14px);
}

.dock-bounce-target.dock-bounce-falling {
    transition-duration: 190ms;
    transition-timing-function: cubic-bezier(0.65, 0, 0.9, 0.35);
    transform: translateY(2px);
}

.dock-bounce-target.dock-bounce-rebound {
    transition-duration: 130ms;
    transition-timing-function: cubic-bezier(0.2, 0.8, 0.3, 1);
    transform: translateY(-4px);
}

.dock-bounce-target.dock-bounce-settling {
    transition-duration: 280ms;
    transition-timing-function: cubic-bezier(0.34, 1.56, 0.64, 1);
    transform: translateY(0);
}

.dock-item-btn {
    background: transparent;
    border: none;
    border-radius: 14px;
    padding: 4px;
    box-shadow: none;
    transition: background-color 180ms cubic-bezier(0.2, 0.9, 0.3, 1.0);
    overflow: visible;
}

button.dock-item-btn,
button.dock-item-btn:hover,
button.dock-item-btn:active,
button.dock-item-btn:checked,
button.dock-item-btn:focus {
    background-color: transparent;
    background-image: none;
    border-color: transparent;
    box-shadow: none;
}

button.dock-item-btn:hover,
button.dock-item-btn:active {
    outline: none;
}

.dock-indicator {
    font-size: 8px;
    color: rgba(0, 0, 0, 0.85);
    margin-top: 2px;
    margin-bottom: -2px;
}

@media (prefers-color-scheme: dark) {
    .dock-indicator {
        color: rgba(255, 255, 255, 0.85);
    }
}

.dock-separator {
    background-color: rgba(255, 255, 255, 0.18);
    min-width: 1px;
    min-height: 44px;
    margin: 4px 6px 10px 6px;
}

/* Tarjeta miniatura de ventana minimizada (macOS Dock Window Thumbnail) */
.dock-window-card {
    padding: 1px 3px;
    margin: 0px 2px;
}

.dock-mini-window {
    background: linear-gradient(180deg, rgba(45, 45, 55, 0.90) 0%, rgba(25, 25, 32, 0.95) 100%);
    border: 1px solid rgba(255, 255, 255, 0.22);
    border-radius: 10px;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.48), 0 1px 3px rgba(255, 255, 255, 0.12);
    transition: all 180ms cubic-bezier(0.2, 0.9, 0.3, 1.0);
    padding: 0;
}

.dock-window-thumbnail {
    border-radius: 3px;
}

.dock-window-card-thumbnail {
    margin: 0;
    padding: 0;
}

.dock-mini-window.has-thumbnail {
    min-width: 0;
    min-height: 0;
    background: transparent;
    border: none;
    box-shadow: none;
}

.dock-mini-window:hover {
    background: linear-gradient(180deg, rgba(58, 58, 72, 0.95) 0%, rgba(32, 32, 42, 0.98) 100%);
    border-color: rgba(255, 255, 255, 0.45);
    box-shadow: 0 6px 16px rgba(0, 0, 0, 0.60), 0 0 8px rgba(59, 130, 246, 0.4);
}

.dock-mini-window.has-thumbnail:hover {
    background: transparent;
    border-color: transparent;
    box-shadow: none;
}

.dock-mini-window-titlebar {
    background: rgba(255, 255, 255, 0.08);
    border-bottom: 1px solid rgba(255, 255, 255, 0.10);
    padding: 2px 4px;
    border-top-left-radius: 7px;
    border-top-right-radius: 7px;
}

.dock-mini-dot {
    font-size: 6px;
    line-height: 6px;
    margin-right: 2px;
}

.dot-red { color: #ff5f56; }
.dot-yellow { color: #ffbd2e; }
.dot-green { color: #27c93f; }

.dock-mini-window-body {
    padding: 4px 6px;
}

.dock-mini-content-line {
    background-color: rgba(255, 255, 255, 0.18);
    border-radius: 2px;
    min-height: 3px;
    margin-bottom: 3px;
}

.dock-mini-content-line-short {
    background-color: rgba(255, 255, 255, 0.10);
    border-radius: 2px;
    min-height: 3px;
    margin-right: 14px;
}

.dock-mini-badge {
    background: rgba(24, 24, 28, 0.95);
    border: 1.5px solid rgba(255, 255, 255, 0.35);
    border-radius: 10px;
    padding: 2px;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.65);
    margin-right: -3px;
    margin-bottom: -3px;
}

.malus-popover-menu {
    background: transparent;
    border: none;
    box-shadow: none;
    padding: 0;
}

popover.malus-popover-menu,
popover.malus-popover-menu.background,
popover.malus-popover-menu > contents,
popover.malus-popover-menu > contents.background,
popover.malus-popover-menu > arrow {
    background-color: transparent;
    background-image: none;
    border: none;
    box-shadow: none;
}

popover.malus-popover-menu > contents,
popover.malus-popover-menu > contents.background {
    padding: 0;
    color: @window_fg_color;
}

.popover-surface,
.popover-surface.background,
.popover-surface > *,
.popover-menu-content,
.popover-menu-content.background {
    background-color: transparent;
    background-image: none;
    border: none;
    box-shadow: none;
}

.popover-menu-header {
    color: alpha(@window_fg_color, 0.72);
    font-size: 12px;
    font-weight: 600;
    padding: 5px 10px 3px;
}

.popover-separator {
    background-color: alpha(@window_fg_color, 0.14);
    min-height: 1px;
    margin: 5px 4px;
}

.popover-menu-item {
    background: transparent;
    background-image: none;
    border: none;
    border-radius: 7px;
    padding: 7px 12px;
    margin: 1px 2px;
    color: @window_fg_color;
    font-size: 14px;
    font-weight: 500;
    box-shadow: none;
}

.popover-menu-item:hover {
    background-color: @accent_bg_color;
    color: @accent_fg_color;
    box-shadow: none;
}

.popover-menu-item-danger:hover {
    background-color: #dc2626;
    color: #ffffff;
}

/* Selector Visual de Aplicaciones estilo macOS (⌘Tab App Switcher HUD) */
window.malus-switcher-window,
window.malus-switcher-window.background,
window.malus-switcher-window decoration,
.switcher-surface,
.switcher-container,
.switcher-card {
    background-color: transparent;
    background-image: none;
    border: none;
    border-radius: 0;
    box-shadow: none;
}

.switcher-container {
    background-color: transparent;
    border: none;
    border-radius: 0;
    box-shadow: none;
    padding: 12px 16px;
}

.switcher-container-background {
    color: @window_bg_color;
}

.switcher-card-background {
    color: @accent_bg_color;
}

.switcher-card {
    background-color: transparent;
    border: none;
    border-radius: 0;
    padding: 0;
    opacity: 0.65;
    transition: all 120ms cubic-bezier(0.2, 0.9, 0.3, 1.0);
}

.switcher-card.selected {
    opacity: 1.0;
}

.switcher-title {
    color: @window_fg_color;
    font-size: 12px;
    font-weight: 600;
    margin-top: 6px;
}

.switcher-card.selected .switcher-title {
    color: @accent_fg_color;
    font-weight: 700;
}
"#;

const DOCK_POPOVER_CSS: &str = r#"
popover.malus-popover-menu,
popover.malus-popover-menu.background,
popover.malus-popover-menu > contents,
popover.malus-popover-menu > contents.background,
popover.malus-popover-menu > arrow {
    background-color: transparent;
    background-image: none;
    border: none;
    box-shadow: none;
}

popover.malus-popover-menu > contents,
popover.malus-popover-menu > contents.background {
    padding: 0;
}
"#;

fn main() {
    init_logging("malus_de_dock");
    info!("Iniciando Malus DE Floating Dock (GTK4 / Layer Shell)...");

    // Esperar de forma activa a que el compositor esté listo y aceptando conexiones
    malus_de_common::wayland::wait_for_wayland(10);

    if let Err(err) = adw::init() {
        warn!("Aviso al inicializar libadwaita: {:?}", err);
    }

    let app = Application::builder()
        .application_id("org.malus_de.Dock")
        .build();

    app.connect_startup(|_| {
        load_css();
    });

    app.connect_activate(build_dock_ui);

    app.run();
}

fn load_css() {
    let provider = CssProvider::new();
    provider.load_from_string(DOCK_CSS);

    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let popover_provider = CssProvider::new();
        popover_provider.load_from_string(DOCK_POPOVER_CSS);
        gtk4::style_context_add_provider_for_display(
            &display,
            &popover_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER + 1,
        );
    }
}

fn create_popover_surface(menu_box: &GtkBox) -> Overlay {
    let overlay = Overlay::new();
    overlay.add_css_class("popover-surface");
    menu_box.add_css_class("popover-menu-content");
    if let Some(last_item) = menu_box.last_child() {
        last_item.set_margin_bottom(10);
    }

    let background = DrawingArea::new();
    background.add_css_class("popover-surface");
    background.set_can_target(false);
    background.set_hexpand(true);
    background.set_vexpand(true);

    match dock_popover_svg_path() {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(svg_template) => {
                background.set_draw_func(move |area, context, width, height| {
                    if width <= 0 || height <= 0 {
                        return;
                    }

                    match themed_popover_svg(&svg_template, area, width, height) {
                        Ok(svg) => {
                            gtk4::gdk::prelude::GdkCairoContextExt::set_source_pixbuf(
                                context, &svg, 0.0, 0.0,
                            );
                            if let Err(error) = context.paint() {
                                warn!("No se pudo dibujar el fondo SVG del popover: {error}");
                            }
                        }
                        Err(error) => {
                            warn!("No se pudo adaptar el SVG del popover al tema: {error}");
                        }
                    }
                });
            }
            Err(error) => {
                warn!("No se pudo leer el SVG del popover del dock: {error}");
            }
        },
        None => {
            warn!("No se encontró el SVG del popover del dock.");
        }
    }

    overlay.set_child(Some(&background));
    overlay.add_overlay(menu_box);
    overlay.set_measure_overlay(menu_box, true);
    overlay.set_clip_overlay(menu_box, false);

    menu_box.set_halign(gtk4::Align::Fill);
    menu_box.set_valign(gtk4::Align::Fill);
    menu_box.set_margin_start(8);
    menu_box.set_margin_end(8);
    menu_box.set_margin_top(6);
    menu_box.set_margin_bottom(35);

    overlay
}

const POPOVER_TAIL_WIDTH: f64 = 44.0;
const POPOVER_TAIL_HEIGHT: f64 = 18.0;
const POPOVER_BODY_BOTTOM_INSET: f64 = 34.5;
const SQUIRCLE_EXPONENT: f64 = 4.2;
const SQUIRCLE_SEGMENTS: usize = 24;

fn themed_popover_svg(
    template: &str,
    area: &DrawingArea,
    width: i32,
    height: i32,
) -> Result<gdk_pixbuf::Pixbuf, String> {
    let background = theme_color(area, "window_bg_color");
    let foreground = theme_color(area, "window_fg_color");
    let radius = 18.0_f64.min(f64::from(width.min(height)) / 2.0);
    let tail_width = POPOVER_TAIL_WIDTH.min((f64::from(width) - radius * 2.0).max(0.0));
    let tail_height = POPOVER_TAIL_HEIGHT.min(f64::from(height) / 4.0);
    let body_bottom = f64::from(height) - POPOVER_BODY_BOTTOM_INSET;
    let center_x = f64::from(width) / 2.0;
    let tail_half_width = tail_width / 2.0;
    let left_tail = center_x - tail_half_width;
    let right_tail = center_x + tail_half_width;
    let right_edge = f64::from(width) - 0.5;
    let corner_center_top = 0.5 + radius;
    let corner_center_right = right_edge - radius;
    let corner_center_left = 0.5 + radius;
    let mut path = format!(
        "M {left_top},{top_edge} H {top_right}",
        left_top = corner_center_left,
        top_edge = 0.5,
        top_right = corner_center_right
    );
    append_squircle_corner(
        &mut path,
        corner_center_right,
        corner_center_top,
        radius,
        -std::f64::consts::FRAC_PI_2,
        0.0,
    );
    path.push_str(&format!(" V {}", body_bottom - radius));
    append_squircle_corner(
        &mut path,
        corner_center_right,
        body_bottom - radius,
        radius,
        0.0,
        std::f64::consts::FRAC_PI_2,
    );
    path.push_str(&format!(
        " H {tail_right} \
         C {tail_right_curve},{body_bottom} {tail_right_inner},{tail_curve_y} {tail_slope_right},{tail_slope_y} \
         L {tail_tip_right},{tail_tip_curve_y} \
         Q {center_x},{tail_tip} {tail_tip_left},{tail_tip_curve_y} \
         L {tail_slope_left},{tail_slope_y} \
         C {tail_left_inner},{tail_curve_y} {tail_left_curve},{body_bottom} {tail_left},{body_bottom} \
         H {bottom_left}",
        tail_right = right_tail,
        tail_right_curve = right_tail - 2.5,
        tail_right_inner = right_tail - 3.5,
        tail_curve_y = body_bottom + 0.5,
        tail_slope_right = center_x + 2.0,
        tail_slope_y = body_bottom + tail_height - 2.0,
        tail_tip_right = center_x + 2.0,
        tail_tip_curve_y = body_bottom + tail_height - 2.0,
        tail_tip = body_bottom + tail_height,
        tail_tip_left = center_x - 2.0,
        tail_slope_left = center_x - 2.0,
        tail_left_inner = left_tail + 3.5,
        tail_left_curve = left_tail + 2.5,
        tail_left = left_tail,
        bottom_left = corner_center_left,
    ));
    append_squircle_corner(
        &mut path,
        corner_center_left,
        body_bottom - radius,
        radius,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    path.push_str(&format!(" V {corner_center_top}"));
    append_squircle_corner(
        &mut path,
        corner_center_left,
        corner_center_top,
        radius,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    path.push_str(" Z");
    let mut svg = template
        .replace("#f5f0ff", &blend_color_hex(background, foreground, 0.08))
        .replace("#e9e0f8", &blend_color_hex(background, foreground, 0.025))
        .replace("#d9cdec", &blend_color_hex(background, foreground, 0.06))
        .replace("#ffffff", &color_hex(foreground))
        .replace(
            "width=\"320\" height=\"264\" viewBox=\"0 0 320 264\"",
            &format!("width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\""),
        );
    let path_start = svg
        .find(" d=\"")
        .ok_or_else(|| "El SVG del popover no contiene trazado".to_string())?
        + 4;
    let path_end = svg[path_start..]
        .find('"')
        .map(|offset| path_start + offset)
        .ok_or_else(|| "El trazado SVG del popover está incompleto".to_string())?;
    svg.replace_range(path_start..path_end, &path);

    let loader = gdk_pixbuf::PixbufLoader::with_type("svg").map_err(|error| error.to_string())?;
    loader
        .write(svg.as_bytes())
        .map_err(|error| error.to_string())?;
    loader.close().map_err(|error| error.to_string())?;
    loader
        .pixbuf()
        .ok_or_else(|| "El SVG no produjo una imagen".to_string())
}

fn append_squircle_corner(
    path: &mut String,
    center_x: f64,
    center_y: f64,
    radius: f64,
    start_angle: f64,
    end_angle: f64,
) {
    let power = 2.0 / SQUIRCLE_EXPONENT;
    for step in 1..=SQUIRCLE_SEGMENTS {
        let fraction = step as f64 / SQUIRCLE_SEGMENTS as f64;
        let angle = start_angle + (end_angle - start_angle) * fraction;
        let x = angle.cos().signum() * angle.cos().abs().powf(power);
        let y = angle.sin().signum() * angle.sin().abs().powf(power);
        path.push_str(&format!(
            " L {:.3},{:.3}",
            center_x + radius * x,
            center_y + radius * y
        ));
    }
}

fn blend_color_hex(
    background: gtk4::gdk::RGBA,
    foreground: gtk4::gdk::RGBA,
    foreground_weight: f32,
) -> String {
    let channel = |bg: f32, fg: f32| {
        ((bg * (1.0 - foreground_weight) + fg * foreground_weight) * 255.0).round() as u8
    };
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(background.red(), foreground.red()),
        channel(background.green(), foreground.green()),
        channel(background.blue(), foreground.blue())
    )
}

fn color_hex(color: gtk4::gdk::RGBA) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (color.red() * 255.0).round() as u8,
        (color.green() * 255.0).round() as u8,
        (color.blue() * 255.0).round() as u8
    )
}

fn dock_popover_svg_path() -> Option<std::path::PathBuf> {
    let installed_paths = [
        "/usr/local/share/malus-de/dock/macos-popover.svg",
        "/usr/share/malus-de/dock/macos-popover.svg",
    ];
    if let Some(path) = installed_paths
        .iter()
        .map(std::path::PathBuf::from)
        .find(|path| path.is_file())
    {
        return Some(path);
    }

    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent()?
                .parent()?
                .parent()
                .map(|root| root.join("assets/dock/macos-popover.svg"))
        })
        .filter(|path| path.is_file())
}

fn queue_dock_resize(window: &ApplicationWindow, dock_root: &GtkBox, dock_bar: &GtkBox) {
    dock_bar.queue_resize();
    dock_root.queue_resize();

    let (_, width, _, _) = dock_root.measure(Orientation::Horizontal, -1);
    if width > 0 {
        let (_, height, _, _) = dock_root.measure(Orientation::Vertical, width);
        if height > 0 && (width < window.width() || height < window.height()) {
            window.set_default_size(width, height);
        }
    }

    window.queue_resize();
}

fn check_trash_full() -> bool {
    let trash = gio::File::for_uri("trash:///");
    if let Ok(enumerator) = trash.enumerate_children(
        "standard::name",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    ) {
        enumerator.into_iter().next().is_some()
    } else {
        false
    }
}

fn should_ignore_app(identifier: &str) -> bool {
    let lower = identifier.to_lowercase();
    let ignored = [
        "top-bar", "layer-shell", "desktop", "gdm", "compositor",
        "xwayland", "malus-session", "xsettings", "pipewire", "wireplumber",
        "gnome-shell", "unknown", "malus-de-top-bar", "malus-de-dock",
        "malus-de-wallpaper", "malus-de-link", "malus-de-spotlight",
        "malus-de-notifications"
    ];
    ignored.iter().any(|ign| lower.contains(ign))
}

fn resolve_desktop_app_info(identifier: &str) -> (String, String, String, Vec<String>) {
    let lower = identifier.to_lowercase();
    let candidates = [
        format!("{}.desktop", identifier),
        format!("{}.desktop", lower),
        format!("org.gnome.{}.desktop", identifier),
        format!("org.gnome.{}.desktop", lower),
        identifier.to_string(),
        lower.clone(),
    ];
    for cand in &candidates {
        if let Some(info) = gio::DesktopAppInfo::new(cand) {
            let name = info.name().to_string();
            let icon = info.icon()
                .and_then(|i| i.to_string())
                .map(|s| s.to_string())
                .unwrap_or_else(|| identifier.to_string());
            let exec = info.executable().to_string_lossy().to_string();
            let matchers = vec![identifier.to_string(), lower.clone(), name.to_lowercase()];
            return (name, icon, exec, matchers);
        }
    }

    let name = capitalize(identifier);
    let icon = identifier.to_string();
    let exec = identifier.to_string();
    let matchers = vec![identifier.to_string(), lower];
    (name, icon, exec, matchers)
}

fn dock_app_context_actions(app_cfg: &DockAppConfig) -> Vec<(&'static str, &'static [&'static str])> {
    let identity = [
        app_cfg.name.as_str(),
        app_cfg.icon.as_str(),
        app_cfg.command.as_str(),
    ]
    .into_iter()
    .chain(app_cfg.matchers.iter().map(String::as_str))
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase();

    let mut actions = Vec::new();
    if identity.contains("nautilus") {
        actions.push(("Abrir nueva ventana", &["--new-window"][..]));
    }

    if identity
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| part == "brave")
    {
        actions.push(("Abrir en modo incógnito", &["--incognito"][..]));
    }

    let is_code_editor = identity.contains("antigravity")
        || identity.contains("visual studio code")
        || identity.contains("vscode")
        || identity.contains("code-oss")
        || identity.contains("code-insiders")
        || identity
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|part| part == "code");
    if is_code_editor {
        actions.push(("Abrir nueva ventana vacía", &["--new-window"][..]));
    }

    actions
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn bounce_dock_icon(
    icon_container: &gtk4::Widget,
    animation_id: Rc<Cell<u64>>,
) {
    let icon_container = icon_container.clone();
    let current_id = animation_id.get().wrapping_add(1);
    animation_id.set(current_id);
    icon_container.remove_css_class("dock-bouncing");
    icon_container.remove_css_class("dock-bounce-falling");
    icon_container.remove_css_class("dock-bounce-rebound");
    icon_container.remove_css_class("dock-bounce-settling");

    let target = icon_container.clone();
    glib::timeout_add_local_once(Duration::from_millis(1), move || {
        if animation_id.get() != current_id {
            return;
        }
        target.add_css_class("dock-bouncing");
        let target_falling = target.clone();
        glib::timeout_add_local_once(Duration::from_millis(280), move || {
            if animation_id.get() != current_id {
                return;
            }
            target_falling.remove_css_class("dock-bouncing");
            target_falling.add_css_class("dock-bounce-falling");
            let target_rebound = target_falling.clone();
            glib::timeout_add_local_once(Duration::from_millis(190), move || {
                if animation_id.get() != current_id {
                    return;
                }
                target_rebound.remove_css_class("dock-bounce-falling");
                target_rebound.add_css_class("dock-bounce-rebound");
                let target_settling = target_rebound.clone();
                glib::timeout_add_local_once(Duration::from_millis(130), move || {
                    if animation_id.get() != current_id {
                        return;
                    }
                    target_settling.remove_css_class("dock-bounce-rebound");
                    target_settling.add_css_class("dock-bounce-settling");
                    glib::timeout_add_local_once(Duration::from_millis(280), move || {
                        if animation_id.get() == current_id {
                            target_settling.remove_css_class("dock-bounce-settling");
                        }
                    });
                });
            });
        });
    });
}

fn matches_dock_app(matchers: &[String], app_name: &str, identity: &str) -> bool {
    let identity = identity.to_lowercase();
    matchers
        .iter()
        .filter(|matcher| !matcher.is_empty())
        .any(|matcher| identity.contains(&matcher.to_lowercase()))
        || (matchers.is_empty() && identity.contains(&app_name.to_lowercase()))
}

fn build_dock_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Malus DE Dock")
        .decorated(false)
        .css_classes(["malus-dock-window"])
        .build();

    window.set_decorated(false);

    // 1. Cargar configuración persistida desde dock.toml
    let dock_config = Rc::new(RefCell::new(DockConfig::load_or_create_default()));
    let initial_cfg = dock_config.borrow().clone();

    if gtk4_layer_shell::is_supported() {
        info!("Layer Shell soportado. Configurando Dock flotante centrado inferior...");
        window.init_layer_shell();
        window.set_layer(Layer::Top);
        window.set_namespace("malus-dock");
        window.set_anchor(Edge::Bottom, true);
        window.set_margin(Edge::Bottom, initial_cfg.floating_gap as i32);
        window.auto_exclusive_zone_enable();
    } else {
        info!("Layer Shell no detectado; ejecutando Dock en ventana flotante de desarrollo.");
    }

    let dock_root = GtkBox::new(Orientation::Vertical, 0);
    dock_root.set_overflow(gtk4::Overflow::Visible);
    let magnification_headroom = GtkBox::new(Orientation::Horizontal, 0);
    magnification_headroom.set_size_request(-1, dock_headroom_height(&initial_cfg));
    dock_root.append(&magnification_headroom);

    let dock_bar = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["dock-container"])
        .spacing(4)
        .valign(gtk4::Align::End)
        .halign(gtk4::Align::Center)
        .build();
    dock_bar.set_overflow(gtk4::Overflow::Visible);

    // Contenedor modular de aplicaciones (fijadas y dinámicas en ejecución)
    let apps_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(2)
        .valign(gtk4::Align::End)
        .build();
    apps_box.set_overflow(gtk4::Overflow::Visible);
    dock_bar.append(&apps_box);

    // Separador de cristal translúcido
    let separator = Separator::new(Orientation::Vertical);
    separator.add_css_class("dock-separator");
    separator.set_valign(gtk4::Align::End);
    dock_bar.append(&separator);

    // Contenedor dinámico de miniaturas de ventanas minimizadas (Bloque Derecho)
    let minimized_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .valign(gtk4::Align::Center)
        .build();
    minimized_box.set_overflow(gtk4::Overflow::Visible);
    minimized_box.set_visible(false);
    dock_bar.append(&minimized_box);

    // Papelera de Reciclaje (Trash) con estado reactivo y menú contextual
    let (trash_widget, trash_img) = create_trash_item(initial_cfg.icon_size as i32);
    dock_bar.append(&trash_widget);
    dock_root.append(&dock_bar);

    // Ventana modal Overlay del Selector Visual de Aplicaciones (⌘Tab App Switcher HUD)
    let switcher_window = Window::builder()
        .title("Malus DE App Switcher")
        .css_classes(["malus-switcher-window"])
        .build();

    if gtk4_layer_shell::is_supported() {
        switcher_window.init_layer_shell();
        switcher_window.set_layer(Layer::Overlay);
        switcher_window.set_namespace("malus-app-switcher");
        switcher_window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::None);
        switcher_window.set_exclusive_zone(0);
    } else {
        switcher_window.set_decorated(false);
    }

    let switcher_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["switcher-container"])
        .spacing(8)
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .build();

    let win_dismiss_click = GestureClick::new();
    win_dismiss_click.connect_released(|_, _, _, _| {
        let _ = send_ipc_message(&IpcMessage::HideAppSwitcher);
    });
    switcher_box.set_visible(false);
    let switcher_background = DrawingArea::new();
    switcher_background.add_css_class("switcher-container-background");
    switcher_background.set_can_target(false);
    switcher_background.set_draw_func(|area, context, width, height| {
        draw_switcher_squircle(area, context, width as f64, height as f64, 32.0, false, true);
    });
    let switcher_surface = Overlay::new();
    switcher_surface.add_css_class("switcher-surface");
    switcher_surface.set_child(Some(&switcher_background));
    switcher_surface.add_overlay(&switcher_box);
    switcher_surface.set_measure_overlay(&switcher_box, true);
    switcher_window.set_child(Some(&switcher_surface));

    // Canales y estado compartido
    let running_cache = Rc::new(RefCell::new(Vec::<String>::new()));
    let active_app_cache = Rc::new(RefCell::new(Vec::<String>::new()));
    let dock_indicators: Rc<RefCell<Vec<(Vec<String>, Label)>>> = Rc::new(RefCell::new(Vec::new()));

    // Canal de actualización del layout de aplicaciones del Dock
    let (refresh_tx, refresh_rx) = async_channel::unbounded::<bool>();
    let r_tx = refresh_tx.clone();
    let on_refresh: Rc<dyn Fn()> = Rc::new(move || {
        let _ = r_tx.send_blocking(true);
    });
    let running_tx = refresh_tx.clone();
    let on_running_refresh: Rc<dyn Fn()> = Rc::new(move || {
        let _ = running_tx.send_blocking(false);
    });

    // Render inicial de aplicaciones desde dock.toml
    render_dock_apps(
        &apps_box,
        &dock_config,
        &running_cache,
        &active_app_cache,
        &dock_indicators,
        &on_refresh,
        true,
    );

    let apps_box_theme = apps_box.clone();
    adw::StyleManager::default().connect_dark_notify(move |_| {
        queue_dock_icon_backgrounds(apps_box_theme.upcast_ref());
    });

    // Loop de refresco reactivo de las aplicaciones
    let apps_box_rf = apps_box.clone();
    let dock_cfg_rf = dock_config.clone();
    let run_cache_rf = running_cache.clone();
    let active_app_rf = active_app_cache.clone();
    let indicators_rf = dock_indicators.clone();
    let on_rf_loop = on_refresh.clone();
    let dock_bar_resize = dock_bar.clone();
    let dock_root_resize = dock_root.clone();
    let win_resize = window.clone();

    glib::spawn_future_local(async move {
        while let Ok(refresh_pinned) = refresh_rx.recv().await {
            render_dock_apps(
                &apps_box_rf,
                &dock_cfg_rf,
                &run_cache_rf,
                &active_app_rf,
                &indicators_rf,
                &on_rf_loop,
                refresh_pinned,
            );
            queue_dock_resize(&win_resize, &dock_root_resize, &dock_bar_resize);
        }
    });

    // Monitoreo reactivo de cambios en dock.toml en disco (GFileMonitor)
    let config_path = DockConfig::config_path();
    let gfile = gio::File::for_path(&config_path);
    if let Ok(monitor) = gfile.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
        let cfg_clone = dock_config.clone();
        let on_rf_mon = on_refresh.clone();
        let win_mon = window.clone();
        let headroom_mon = magnification_headroom.clone();
        monitor.connect_changed(move |_, _, _, event_type| {
            if event_type == gio::FileMonitorEvent::ChangesDoneHint || event_type == gio::FileMonitorEvent::Created {
                info!("🔄 dock.toml modificado externamente en disco. Recargando...");
                let new_cfg = DockConfig::load_or_create_default();
                if gtk4_layer_shell::is_supported() {
                    win_mon.set_margin(Edge::Bottom, new_cfg.floating_gap as i32);
                }
                headroom_mon.set_size_request(-1, dock_headroom_height(&new_cfg));
                *cfg_clone.borrow_mut() = new_cfg;
                on_rf_mon();
            }
        });
    }

    // Magnificación Analítica macOS (Efecto zoom suave con curva gaussiana)
    let apps_box_motion = apps_box.clone();
    let min_box_motion = minimized_box.clone();
    let trash_widget_motion = trash_widget.clone();
    let dock_bar_motion = dock_bar.clone();
    let dock_root_motion = dock_root.clone();
    let window_motion = window.clone();
    let cfg_motion = dock_config.clone();
    let icon_size_animations = Rc::new(RefCell::new(Vec::<IconSizeAnimation>::new()));
    let icon_size_animation_running = Rc::new(Cell::new(false));
    let icon_size_animations_motion = icon_size_animations.clone();
    let icon_size_animation_running_motion = icon_size_animation_running.clone();

    let motion_controller = EventControllerMotion::new();
    motion_controller.connect_motion(move |_, x, y| {
        let cfg = cfg_motion.borrow();
        if !cfg.magnification_enabled {
            return;
        }
        let Some(pointer_in_dock) = dock_root_motion.compute_point(
            &dock_bar_motion,
            &gtk4::graphene::Point::new(x as f32, y as f32),
        ) else {
            return;
        };
        let x = pointer_in_dock.x() as f64;
        let base_size = cfg.icon_size as f64;
        let max_size = cfg.magnification_max_size as f64;
        let max_scale_diff = (max_size - base_size).max(0.0);
        let max_zoom_dist = 110.0;
        let mut targets = Vec::new();

        let mut update_item = |widget: &gtk4::Widget| {
            if let Some(p) = widget.compute_point(&dock_bar_motion, &gtk4::graphene::Point::zero())
            {
                let cx = p.x() as f64;
                let cw = widget.width() as f64;
                let center_x = cx + cw / 2.0;
                let dist = (x - center_x).abs();
                let factor = if dist < max_zoom_dist {
                    let norm = dist / max_zoom_dist;
                    (-norm * norm * 2.2).exp()
                } else {
                    0.0
                };
                let target_size = (base_size + factor * max_scale_diff) as i32;
                targets.push((widget.clone(), target_size, base_size as i32));
            }
        };

        // Magnificar cada app en apps_box
        let mut child = apps_box_motion.first_child();
        while let Some(c) = child {
            update_item(&c);
            child = c.next_sibling();
        }

        // Magnificar cada miniatura en minimized_box si está visible
        if min_box_motion.is_visible() {
            let mut min_child = min_box_motion.first_child();
            while let Some(c) = min_child {
                update_item(&c);
                min_child = c.next_sibling();
            }
        }

        // Magnificar papelera
        update_item(trash_widget_motion.upcast_ref());
        update_icon_size_animation_targets(&icon_size_animations_motion, targets);
        start_icon_size_animation(
            &icon_size_animations_motion,
            &icon_size_animation_running_motion,
            &window_motion,
            &dock_root_motion,
            &dock_bar_motion,
        );
    });

    let cfg_leave = dock_config.clone();
    let icon_size_animations_leave = icon_size_animations.clone();
    let icon_size_animation_running_leave = icon_size_animation_running.clone();
    let window_leave = window.clone();
    let dock_root_leave = dock_root.clone();
    let dock_bar_leave = dock_bar.clone();

    motion_controller.connect_leave(move |_| {
        let base_size = cfg_leave.borrow().icon_size as i32;
        let mut animations = icon_size_animations_leave.borrow_mut();
        for animation in animations.iter_mut() {
            animation.target_size = base_size as f64;
            animation.base_size = base_size;
        }
        drop(animations);
        start_icon_size_animation(
            &icon_size_animations_leave,
            &icon_size_animation_running_leave,
            &window_leave,
            &dock_root_leave,
            &dock_bar_leave,
        );
    });
    dock_root.add_controller(motion_controller);

    // Escucha de eventos IPC del compositor
    let (tx, rx) = async_channel::unbounded::<IpcMessage>();
    let cache_ipc = running_cache.clone();
    let min_box = minimized_box.clone();
    let dock_bar_ref = dock_bar.clone();
    let dock_root_ref = dock_root.clone();
    let min_map: Rc<RefCell<HashMap<u64, gtk4::Widget>>> = Rc::new(RefCell::new(HashMap::new()));
    let min_paths: Rc<RefCell<HashMap<u64, Option<String>>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let win_ref = window.clone();
    let indicators_ipc = dock_indicators.clone();
    let active_app_ipc = active_app_cache.clone();
    let cfg_ipc = dock_config.clone();
    let on_rf_ipc = on_refresh.clone();
    let on_rf_running_ipc = on_running_refresh.clone();
    let refresh_pending_ipc = Rc::new(Cell::new(false));
    let headroom_ipc = magnification_headroom.clone();
    let switcher_win_ipc = switcher_window.clone();
    let switcher_box_ipc = switcher_box.clone();
    let switcher_cards_ipc: Rc<RefCell<Vec<(DrawingArea, Rc<std::cell::Cell<bool>>)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let switcher_cards_show = switcher_cards_ipc.clone();

    glib::spawn_future_local(async move {
        while let Ok(msg) = rx.recv().await {
            match msg {
                IpcMessage::ActiveAppChanged { app_id, title, .. } => {
                    let mut active_app = active_app_ipc.borrow_mut();
                    active_app.clear();
                    active_app.extend(app_id.into_iter().chain(title));
                }
                IpcMessage::RunningAppsChanged { running_apps } => {
                    let prev_apps = cache_ipc.borrow().clone();
                    *cache_ipc.borrow_mut() = running_apps.clone();

                    // 1. Actualizar indicadores de las apps visibles
                    for (matchers, indicator) in indicators_ipc.borrow().iter() {
                        let is_running = running_apps.iter().any(|running| {
                            let r_lower = running.to_lowercase();
                            matchers.iter().any(|m| r_lower.contains(&m.to_lowercase()))
                        });
                        indicator.set_opacity(if is_running { 1.0 } else { 0.0 });
                    }

                    // 2. Si la lista de aplicaciones ejecutándose cambió, refrescar apps unpinned
                    if prev_apps != running_apps && !refresh_pending_ipc.replace(true) {
                        let on_refresh = on_rf_running_ipc.clone();
                        let refresh_pending = refresh_pending_ipc.clone();
                        glib::timeout_add_local_once(Duration::from_millis(900), move || {
                            refresh_pending.set(false);
                            on_refresh();
                        });
                    }
                }
                IpcMessage::PinAppToDock { name, icon, command, matchers } => {
                    info!("📌 IPC PinAppToDock recibido para: {}", name);
                    let app_cfg = DockAppConfig {
                        name,
                        icon,
                        command,
                        matchers,
                        icon_appearance: DockIconAppearance::Auto,
                    };
                    let mut cfg = cfg_ipc.borrow_mut();
                    if cfg.pin_app(app_cfg) {
                        let _ = cfg.save();
                        drop(cfg);
                        on_rf_ipc();
                    }
                }
                IpcMessage::UnpinAppFromDock { name } => {
                    info!("📌 IPC UnpinAppFromDock recibido para: {}", name);
                    let mut cfg = cfg_ipc.borrow_mut();
                    if cfg.unpin_app(&name) {
                        let _ = cfg.save();
                        drop(cfg);
                        on_rf_ipc();
                    }
                }
                IpcMessage::ReloadDockConfig => {
                    info!("🔄 IPC ReloadDockConfig recibido");
                    let new_cfg = DockConfig::load_or_create_default();
                    let base_size = new_cfg.icon_size as i32;
                    if gtk4_layer_shell::is_supported() {
                        win_ref.set_margin(Edge::Bottom, new_cfg.floating_gap as i32);
                    }
                    headroom_ipc.set_size_request(-1, dock_headroom_height(&new_cfg));
                    *cfg_ipc.borrow_mut() = new_cfg;
                    let cards: Vec<(u64, gtk4::Widget)> = min_map
                        .borrow()
                        .iter()
                        .map(|(id, widget)| (*id, widget.clone()))
                        .collect();
                    for (_, card) in &cards {
                        update_icon_size_recursive(card, base_size, base_size);
                    }
                    queue_dock_resize(&win_ref, &dock_root_ref, &dock_bar_ref);
                    let dock_window_weak = win_ref.downgrade();
                    for delay in [60, 180] {
                        let cards = cards.clone();
                        let dock_window_weak = dock_window_weak.clone();
                        glib::timeout_add_local_once(Duration::from_millis(delay), move || {
                            if let Some(dock_window) = dock_window_weak.upgrade() {
                                for (window_id, card) in cards {
                                    register_minimized_dock_slot(&card, &dock_window, window_id);
                                }
                            }
                        });
                    }
                    on_rf_ipc();
                }
                IpcMessage::MinimizedWindowsChanged { windows } => {
                    let mut current_map = min_map.borrow_mut();
                    let new_ids: HashSet<u64> = windows.iter().map(|w| w.window_id).collect();

                    // Eliminar miniaturas de ventanas ya restauradas o cerradas
                    let to_remove: Vec<u64> = current_map.keys().filter(|id| !new_ids.contains(id)).copied().collect();
                    for id in to_remove {
                        if let Some(widget) = current_map.remove(&id) {
                            min_box.remove(&widget);
                            info!("🗑️ Miniatura removida del Dock: window_id={}", id);
                        }
                        min_paths.borrow_mut().remove(&id);
                    }

                    // Agregar o mantener miniaturas activas
                    for win_info in windows {
                        let thumbnail_changed = min_paths
                            .borrow()
                            .get(&win_info.window_id)
                            != Some(&win_info.thumbnail_path);
                        if !current_map.contains_key(&win_info.window_id) || thumbnail_changed {
                            if let Some(widget) = current_map.remove(&win_info.window_id) {
                                min_box.remove(&widget);
                            }
                            let icon_size = cfg_ipc.borrow().icon_size as i32;
                            let (widget, id) = create_minimized_window_item(&win_info, icon_size);
                            min_box.append(&widget);
                            current_map.insert(id, widget);
                            min_paths
                                .borrow_mut()
                                .insert(id, win_info.thumbnail_path.clone());
                            info!("🖼️ Miniatura agregada al Dock: window_id={} ('{}')", id, win_info.title);
                        }
                        if let Some(widget) = current_map.get(&win_info.window_id) {
                            widget.set_opacity(if win_info.is_animating { 0.0 } else { 1.0 });
                            widget.set_sensitive(!win_info.is_animating);
                        }
                    }

                    // Actualizar visibilidad del contenedor para colapsar espacio si está vacío
                    let has_minimized = !current_map.is_empty();
                    min_box.set_visible(has_minimized);
                    let cards: Vec<(u64, gtk4::Widget)> = current_map
                        .iter()
                        .map(|(id, widget)| (*id, widget.clone()))
                        .collect();
                    drop(current_map);

                    queue_dock_resize(&win_ref, &dock_root_ref, &dock_bar_ref);
                    let dock_window_weak = win_ref.downgrade();
                    for delay in [60, 180] {
                        let cards = cards.clone();
                        let dock_window_weak = dock_window_weak.clone();
                        glib::timeout_add_local_once(Duration::from_millis(delay), move || {
                            if let Some(dock_window) = dock_window_weak.upgrade() {
                                for (window_id, card) in cards {
                                    register_minimized_dock_slot(
                                        &card,
                                        &dock_window,
                                        window_id,
                                    );
                                }
                            }
                        });
                    }
                }
                IpcMessage::ShowAppSwitcher { items, selected_index } => {
                    while let Some(c) = switcher_box_ipc.first_child() {
                        switcher_box_ipc.remove(&c);
                    }
                    switcher_cards_show.borrow_mut().clear();
                    for (i, item) in items.iter().enumerate() {
                        let content = GtkBox::builder()
                            .orientation(Orientation::Vertical)
                            .spacing(6)
                            .margin_top(18)
                            .margin_bottom(18)
                            .margin_start(20)
                            .margin_end(20)
                            .halign(gtk4::Align::Center)
                            .valign(gtk4::Align::Center)
                            .build();
                        let selected = Rc::new(std::cell::Cell::new(i == selected_index));
                        let card_background = DrawingArea::new();
                        card_background.add_css_class("switcher-card-background");
                        card_background.set_can_target(false);
                        let selected_for_draw = selected.clone();
                        card_background.set_draw_func(move |area, context, width, height| {
                            draw_switcher_squircle(
                                area,
                                context,
                                width as f64,
                                height as f64,
                                22.0,
                                selected_for_draw.get(),
                                false,
                            );
                        });
                        let card = Overlay::new();
                        card.add_css_class("switcher-card");
                        if selected.get() {
                            card.add_css_class("selected");
                        }
                        card.set_child(Some(&card_background));
                        card.add_overlay(&content);
                        card.set_measure_overlay(&content, true);

                        let icon_name = item.icon_name.as_deref().unwrap_or("application-x-executable");
                        let icon = Image::builder()
                            .icon_name(icon_name)
                            .pixel_size(64)
                            .halign(gtk4::Align::Center)
                            .build();
                        content.append(&icon);

                        let title_label = Label::builder()
                            .label(&item.title)
                            .css_classes(["switcher-title"])
                            .ellipsize(gtk4::pango::EllipsizeMode::End)
                            .max_width_chars(14)
                            .halign(gtk4::Align::Center)
                            .build();
                        content.append(&title_label);

                        // Conmutar a la ventana al hacer clic sobre su tarjeta
                        let target_wid = item.window_id;
                        let card_click = GestureClick::new();
                        card_click.connect_released(move |_, _, _, _| {
                            let _ = send_ipc_message(&IpcMessage::SwitchToWindow { window_id: target_wid });
                        });
                        card.add_controller(card_click);

                        switcher_box_ipc.append(&card);
                        switcher_cards_show
                            .borrow_mut()
                            .push((card_background, selected));
                    }
                    switcher_box_ipc.set_visible(true);
                    switcher_win_ipc.set_visible(true);
                    switcher_win_ipc.present();
                }
                IpcMessage::UpdateAppSwitcher { selected_index } => {
                    for (index, (background, selected)) in
                        switcher_cards_show.borrow().iter().enumerate()
                    {
                        selected.set(index == selected_index);
                        background.queue_draw();
                    }
                    let mut idx = 0;
                    let mut child = switcher_box_ipc.first_child();
                    while let Some(c) = child {
                        if idx == selected_index {
                            c.add_css_class("selected");
                        } else {
                            c.remove_css_class("selected");
                        }
                        idx += 1;
                        child = c.next_sibling();
                    }
                }
                IpcMessage::HideAppSwitcher => {
                    switcher_box_ipc.set_visible(false);
                    switcher_cards_show.borrow_mut().clear();
                    while let Some(c) = switcher_box_ipc.first_child() {
                        switcher_box_ipc.remove(&c);
                    }
                }
                _ => {}
            }
        }
    });

    // Hilo lector del socket IPC del compositor
    std::thread::spawn(move || {
        let sock_path = ipc_socket_path();
        loop {
            if let Ok(stream) = std::os::unix::net::UnixStream::connect(&sock_path) {
                let reader = std::io::BufReader::new(stream);
                for line in std::io::BufRead::lines(reader) {
                    if let Ok(line) = line {
                        if let Ok(msg) = serde_json::from_str::<IpcMessage>(&line) {
                            let _ = tx.send_blocking(msg);
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });

    // Monitor periódico para actualizar icono de papelera (vacía vs llena)
    let trash_img_ref = trash_img;
    glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
        let is_full = check_trash_full();
        trash_img_ref.set_icon_name(Some(if is_full {
            "user-trash-full"
        } else {
            "user-trash"
        }));
        glib::ControlFlow::Continue
    });

    window.set_child(Some(&dock_root));
    window.present();
}

fn render_dock_apps(
    apps_box: &GtkBox,
    dock_config: &Rc<RefCell<DockConfig>>,
    running_cache: &Rc<RefCell<Vec<String>>>,
    active_app_cache: &Rc<RefCell<Vec<String>>>,
    dock_indicators: &Rc<RefCell<Vec<(Vec<String>, Label)>>>,
    on_refresh: &Rc<dyn Fn()>,
    refresh_pinned: bool,
) {
    let config = dock_config.borrow().clone();
    let running = running_cache.borrow().clone();
    let icon_size = config.icon_size as i32;
    let total_pinned = config.apps.len();

    if refresh_pinned {
        while let Some(child) = apps_box.first_child() {
            apps_box.remove(&child);
        }
        dock_indicators.borrow_mut().clear();
    } else {
        let mut child = apps_box.first_child();
        let mut index = 0;
        while let Some(current) = child {
            child = current.next_sibling();
            if index >= total_pinned {
                apps_box.remove(&current);
            }
            index += 1;
        }
        dock_indicators.borrow_mut().truncate(total_pinned);
    }

    // 1. Aplicaciones fijadas (Pinned Apps)
    if refresh_pinned {
        for (idx, app) in config.apps.iter().enumerate() {
            let is_running = running.iter().any(|r| app.matches(r));
            let (widget, ind) = create_dock_item(
                app,
                true,
                idx,
                total_pinned,
                icon_size,
                is_running,
                running_cache.clone(),
                active_app_cache.clone(),
                dock_config.clone(),
                on_refresh.clone(),
            );
            apps_box.append(&widget);
            dock_indicators.borrow_mut().push((app.matchers.clone(), ind));
        }
    }

    // 2. Aplicaciones en ejecución no fijadas (Unpinned Running Apps)
    let mut unpinned_candidates: Vec<String> = Vec::new();
    for r in &running {
        if should_ignore_app(r) {
            continue;
        }
        if config.apps.iter().any(|a| a.matches(r)) {
            continue;
        }
        let r_lower = r.to_lowercase();
        if !unpinned_candidates.iter().any(|c| c.to_lowercase() == r_lower) {
            unpinned_candidates.push(r.clone());
        }
    }

    for cand in unpinned_candidates {
        let (name, icon, command, matchers) = resolve_desktop_app_info(&cand);
        let app_cfg = DockAppConfig {
            name,
            icon,
            command,
            matchers,
            icon_appearance: DockIconAppearance::Auto,
        };
        let (widget, ind) = create_dock_item(
            &app_cfg,
            false,
            0,
            0,
            icon_size,
            true,
            running_cache.clone(),
            active_app_cache.clone(),
            dock_config.clone(),
            on_refresh.clone(),
        );
        apps_box.append(&widget);
        dock_indicators.borrow_mut().push((app_cfg.matchers.clone(), ind));
    }
}

fn update_icon_size_recursive(widget: &gtk4::Widget, size: i32, base_size: i32) -> bool {
    let mut changed = false;
    if let Ok(overlay) = widget.clone().downcast::<Overlay>() {
        let height = if overlay.has_css_class("dock-window-card-thumbnail") {
            -1
        } else {
            base_size
        };
        if overlay.width_request() != size || overlay.height_request() != height {
            overlay.set_size_request(size, height);
            changed = true;
        }
    }
    if let Ok(picture) = widget.clone().downcast::<Picture>() {
        let current_width = picture.width_request().max(1);
        let current_height = picture.height_request().max(1);
        let height = ((size as f64 * current_height as f64) / current_width as f64)
            .round()
            .max(1.0) as i32;
        if picture.width_request() != size || picture.height_request() != height {
            picture.set_size_request(size, height);
            changed = true;
        }
    }
    if widget.has_css_class("dock-icon-shadow")
        && (widget.width_request() != size || widget.height_request() != size)
    {
        widget.set_size_request(size, size);
        changed = true;
    }
    if let Ok(img) = widget.clone().downcast::<Image>() {
        // Solo ajustar iconos principales del dock, preservando insignias secundarias
        let target_size = (size
            + if img.has_css_class("dock-trash-icon") {
                8
            } else if img.has_css_class("dock-icon-inset") {
                -10
            } else {
                0
            })
        .max(1);
        if img.pixel_size() >= (base_size - 10) && img.pixel_size() != target_size {
            img.set_pixel_size(target_size);
            changed = true;
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        changed |= update_icon_size_recursive(&c, size, base_size);
        child = c.next_sibling();
    }
    changed
}

struct IconSizeAnimation {
    widget: gtk4::Widget,
    current_size: f64,
    target_size: f64,
    base_size: i32,
}

fn update_icon_size_animation_targets(
    animations: &Rc<RefCell<Vec<IconSizeAnimation>>>,
    targets: Vec<(gtk4::Widget, i32, i32)>,
) {
    let mut animations = animations.borrow_mut();
    animations.retain(|animation| animation.widget.parent().is_some());

    for (widget, target_size, base_size) in targets {
        if let Some(animation) = animations
            .iter_mut()
            .find(|animation| animation.widget == widget)
        {
            animation.target_size = target_size as f64;
            animation.base_size = base_size;
        } else {
            animations.push(IconSizeAnimation {
                widget,
                current_size: base_size as f64,
                target_size: target_size as f64,
                base_size,
            });
        }
    }
}

fn start_icon_size_animation(
    animations: &Rc<RefCell<Vec<IconSizeAnimation>>>,
    animation_running: &Rc<Cell<bool>>,
    window: &ApplicationWindow,
    dock_root: &GtkBox,
    dock_bar: &GtkBox,
) {
    if animation_running.replace(true) {
        return;
    }

    let animations = animations.clone();
    let animation_running = animation_running.clone();
    let window = window.clone();
    let dock_root = dock_root.clone();
    let dock_bar = dock_bar.clone();
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let mut layout_changed = false;
        let mut settled = true;
        let mut animations = animations.borrow_mut();
        animations.retain_mut(|animation| {
            if animation.widget.parent().is_none() {
                return false;
            }

            let difference = animation.target_size - animation.current_size;
            if difference.abs() < 0.5 {
                animation.current_size = animation.target_size;
            } else {
                animation.current_size += difference * 0.22;
                settled = false;
            }
            layout_changed |= update_icon_size_recursive(
                &animation.widget,
                animation.current_size.round() as i32,
                animation.base_size,
            );
            true
        });
        drop(animations);

        if layout_changed {
            queue_dock_resize(&window, &dock_root, &dock_bar);
        }
        if settled {
            animation_running.set(false);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn should_show_dock_icon_background(
    appearance: DockIconAppearance,
    icon_name: &str,
    icon: &Image,
    icon_size: i32,
) -> bool {
    match appearance {
        DockIconAppearance::DockBackground => true,
        DockIconAppearance::Native => false,
        DockIconAppearance::Auto => match inspect_icon_background(icon_name, icon, icon_size) {
            Some(has_background) => !has_background,
            None => {
                warn!(
                    "No se pudo analizar el fondo del icono '{}'; se usará el fondo del Dock",
                    icon_name
                );
                true
            }
        },
    }
}

fn inspect_icon_background(icon_name: &str, icon: &Image, icon_size: i32) -> Option<bool> {
    let icon_path = if icon_name.starts_with('/') {
        Some(std::path::PathBuf::from(icon_name))
    } else {
        gtk4::IconTheme::for_display(&icon.display())
            .lookup_icon(
                icon_name,
                &[],
                icon_size.max(1),
                1,
                icon.direction(),
                gtk4::IconLookupFlags::empty(),
            )
            .file()
            .and_then(|file| file.path())
    }?;

    let pixbuf = gdk_pixbuf::Pixbuf::from_file_at_scale(&icon_path, 64, 64, true).ok()?;
    let bytes = pixbuf.read_pixel_bytes();
    let pixels = bytes.as_ref();
    let channels = pixbuf.n_channels() as usize;
    let rowstride = pixbuf.rowstride() as usize;
    let width = pixbuf.width() as usize;
    let height = pixbuf.height() as usize;
    if channels < 3 || width == 0 || height == 0 {
        return None;
    }

    let alpha_index = if pixbuf.has_alpha() {
        channels - 1
    } else {
        0
    };
    let mut opaque_pixels = 0usize;
    for y in 0..height {
        for x in 0..width {
            let pixel_start = y * rowstride + x * channels;
            let alpha = if pixbuf.has_alpha() {
                pixels.get(pixel_start + alpha_index).copied()?
            } else {
                255
            };
            if alpha >= 230 {
                opaque_pixels += 1;
            }
        }
    }

    Some(opaque_pixels as f64 / (width * height) as f64 >= 0.72)
}

fn create_magnifiable_icon(icon: &Image, icon_size: i32, show_backdrop: bool) -> Overlay {
    let placeholder = GtkBox::new(Orientation::Horizontal, 0);
    placeholder.set_size_request(icon_size, icon_size);

    let overlay = Overlay::new();
    overlay.set_child(Some(&placeholder));
    if show_backdrop {
        let shadow = DrawingArea::new();
        shadow.add_css_class("dock-icon-shadow");
        shadow.set_size_request(icon_size, icon_size);
        shadow.set_halign(gtk4::Align::Center);
        shadow.set_valign(gtk4::Align::End);
        shadow.set_can_target(false);
        shadow.set_overflow(gtk4::Overflow::Visible);
        shadow.set_draw_func(|_, context, width, height| {
            draw_squircle(context, width as f64, height as f64);
        });
        overlay.add_overlay(&shadow);
        overlay.set_measure_overlay(&shadow, false);
    }
    overlay.add_overlay(icon);
    overlay.set_measure_overlay(icon, false);
    overlay.set_overflow(gtk4::Overflow::Visible);

    icon.set_halign(gtk4::Align::Center);
    icon.set_valign(gtk4::Align::End);
    icon.set_overflow(gtk4::Overflow::Visible);

    overlay
}

fn queue_dock_icon_backgrounds(widget: &gtk4::Widget) {
    if let Ok(area) = widget.clone().downcast::<DrawingArea>() {
        if area.has_css_class("dock-icon-shadow") {
            area.queue_draw();
        }
    }

    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        queue_dock_icon_backgrounds(&current);
    }
}

fn draw_squircle(context: &gtk4::cairo::Context, width: f64, height: f64) {
    squircle_path(context, width, height, 22.0);
    if adw::StyleManager::default().is_dark() {
        context.set_source_rgb(0.06, 0.06, 0.07);
    } else {
        context.set_source_rgb(1.0, 1.0, 1.0);
    }
    let _ = context.fill();
}

fn draw_switcher_squircle(
    area: &DrawingArea,
    context: &gtk4::cairo::Context,
    width: f64,
    height: f64,
    radius: f64,
    selected: bool,
    container: bool,
) {
    let background_color = theme_color(
        area,
        if container {
            "window_bg_color"
        } else if selected {
            "accent_bg_color"
        } else {
            "window_fg_color"
        },
    );
    let foreground = theme_color(area, "window_fg_color");
    let accent_color = theme_color(area, "accent_bg_color");

    squircle_path(context, width, height, radius);
    let fill_color = if container {
        background_color
    } else if selected {
        accent_color
    } else {
        foreground
    };
    let fill_alpha = if container {
        0.42
    } else if selected {
        0.22
    } else {
        0.055
    };
    context.set_source_rgba(
        fill_color.red() as f64,
        fill_color.green() as f64,
        fill_color.blue() as f64,
        (fill_color.alpha() * fill_alpha) as f64,
    );
    let _ = context.fill_preserve();

    let border_color = if selected {
        accent_color
    } else {
        foreground
    };
    let border_alpha = if container { 0.18 } else if selected { 0.95 } else { 0.14 };
    context.set_source_rgba(
        border_color.red() as f64,
        border_color.green() as f64,
        border_color.blue() as f64,
        (border_color.alpha() * border_alpha) as f64,
    );
    context.set_line_width(if selected { 1.5 } else { 1.0 });
    let _ = context.stroke();
}

#[allow(deprecated)]
fn theme_color(area: &DrawingArea, name: &str) -> gtk4::gdk::RGBA {
    area.style_context()
        .lookup_color(name)
        .unwrap_or_else(|| area.color())
}

fn squircle_path(context: &gtk4::cairo::Context, width: f64, height: f64, radius: f64) {
    let radius = radius.min(width / 2.0).min(height / 2.0);
    let power = 2.0 / SQUIRCLE_EXPONENT;
    let quarter_turn = std::f64::consts::FRAC_PI_2;

    context.move_to(radius, 0.0);
    context.line_to(width - radius, 0.0);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            width - radius + radius * angle.sin().powf(power),
            radius - radius * angle.cos().powf(power),
        );
    }
    context.line_to(width, height - radius);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            width - radius + radius * angle.cos().powf(power),
            height - radius + radius * angle.sin().powf(power),
        );
    }
    context.line_to(radius, height);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            radius - radius * angle.sin().powf(power),
            height - radius + radius * angle.cos().powf(power),
        );
    }
    context.line_to(0.0, radius);

    for step in 1..=SQUIRCLE_SEGMENTS {
        let angle = quarter_turn * step as f64 / SQUIRCLE_SEGMENTS as f64;
        context.line_to(
            radius - radius * angle.cos().powf(power),
            radius - radius * angle.sin().powf(power),
        );
    }

    context.close_path();
}

fn create_minimized_window_item(info: &MinimizedWindowInfo, icon_size: i32) -> (gtk4::Widget, u64) {
    let window_id = info.window_id;

    // Mini ventana: captura real cuando está disponible, con la tarjeta simulada como respaldo.
    let mini_win = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["dock-mini-window"])
        .valign(gtk4::Align::Center)
        .build();

    let has_thumbnail = info.thumbnail_path.as_deref().is_some_and(|path| {
        match gdk_pixbuf::Pixbuf::from_file_at_scale(path, icon_size.max(1), icon_size.max(1), true)
        {
            Ok(pixbuf) => {
                let texture = gtk4::gdk::Texture::for_pixbuf(&pixbuf);
                let preview_height = ((icon_size as f64 * pixbuf.height() as f64)
                    / pixbuf.width().max(1) as f64)
                    .round()
                    .max(1.0) as i32;
                let image = Picture::for_paintable(&texture);
                image.set_content_fit(gtk4::ContentFit::Contain);
                image.set_can_shrink(true);
                image.set_size_request(icon_size, preview_height);
                image.set_hexpand(true);
                image.add_css_class("dock-window-thumbnail");
                mini_win.add_css_class("has-thumbnail");
                mini_win.append(&image);
                true
            }
            Err(error) => {
                warn!("No se pudo cargar la miniatura de '{}': {error}", path);
                false
            }
        }
    });

    if !has_thumbnail {
        mini_win.set_size_request(icon_size.max(1), -1);

        // Barra superior simulada, visible solo mientras no haya una captura real.
        let titlebar = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .css_classes(["dock-mini-window-titlebar"])
            .spacing(2)
            .build();

        let dot_red = Label::builder().label("●").css_classes(["dock-mini-dot", "dot-red"]).build();
        let dot_yellow = Label::builder().label("●").css_classes(["dock-mini-dot", "dot-yellow"]).build();
        let dot_green = Label::builder().label("●").css_classes(["dock-mini-dot", "dot-green"]).build();

        titlebar.append(&dot_red);
        titlebar.append(&dot_yellow);
        titlebar.append(&dot_green);
        mini_win.append(&titlebar);

        let body = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .css_classes(["dock-mini-window-body"])
            .valign(gtk4::Align::Center)
            .build();

        let line1 = GtkBox::builder().css_classes(["dock-mini-content-line"]).build();
        let line2 = GtkBox::builder().css_classes(["dock-mini-content-line-short"]).build();
        body.append(&line1);
        body.append(&line2);
        mini_win.append(&body);
    }

    // 3. Insignia circular de la app en la esquina inferior derecha (macOS style)
    let icon_name = info.icon_name.as_deref().unwrap_or("application-x-executable");
    let badge_img = Image::builder()
        .icon_name(icon_name)
        .pixel_size(18)
        .build();

    let badge_box = GtkBox::builder()
        .css_classes(["dock-mini-badge"])
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::End)
        .build();
    badge_box.append(&badge_img);

    // Contenedor Overlay para superponer la insignia sobre la mini ventana
    let overlay = Overlay::builder()
        .child(&mini_win)
        .css_classes(["dock-window-card"])
        .tooltip_text(&info.title)
        .focusable(false)
        .valign(gtk4::Align::Center)
        .build();
    if has_thumbnail {
        overlay.add_css_class("dock-window-card-thumbnail");
    }
    overlay.add_overlay(&badge_box);

    // Clic para restaurar la ventana vía Genie Effect inverso
    let gesture = GestureClick::new();
    let id = window_id;
    gesture.connect_pressed(move |_, _, _, _| {
        info!("✨ Clic en miniatura de Dock: restaurando ventana id={}", id);
        let _ = send_ipc_message(&IpcMessage::AnimateRestore { window_id: id });
    });
    overlay.add_controller(gesture);

    (overlay.upcast::<gtk4::Widget>(), window_id)
}

fn register_minimized_dock_slot(
    card: &gtk4::Widget,
    dock_window: &ApplicationWindow,
    window_id: u64,
) {
    if let Some(point) = card.compute_point(dock_window, &gtk4::graphene::Point::zero()) {
        let x = point.x().round() as i32;
        let y = point.y().round() as i32;
        let width = card.width().max(1) as u32;
        let height = card.height().max(1) as u32;

        info!(
            "📍 Registrando slot de miniatura en Dock: id={}, pos=({}, {}, {}, {})",
            window_id, x, y, width, height
        );
        let _ = send_ipc_message(&IpcMessage::RegisterDockSlot {
            window_id,
            x,
            y,
            width,
            height,
        });
    }
}

fn create_trash_item(icon_size: i32) -> (GtkBox, Image) {
    let item_container = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["dock-item-box"])
        .spacing(1)
        .valign(gtk4::Align::End)
        .build();
    item_container.set_overflow(gtk4::Overflow::Visible);

    let btn = Button::builder()
        .css_classes(["dock-item-btn", "dock-trash-btn"])
        .tooltip_text("Papelera")
        .focusable(false)
        .valign(gtk4::Align::End)
        .build();

    let trash_icon_name = if check_trash_full() {
        "user-trash-full"
    } else {
        "user-trash"
    };

    let trash_img = Image::builder()
        .icon_name(trash_icon_name)
        .pixel_size(icon_size + 8)
        .css_classes(["dock-trash-icon"])
        .valign(gtk4::Align::End)
        .build();
    let trash_overlay = create_magnifiable_icon(&trash_img, icon_size, false);
    trash_img.set_valign(gtk4::Align::Center);
    btn.set_child(Some(&trash_overlay));

    btn.connect_clicked(|_| {
        let _ = Command::new("nautilus").arg("trash://").spawn();
    });

    // Menú Popover de Clic Derecho para la Papelera
    let popover = Popover::builder()
        .css_classes(["malus-popover-menu"])
        .has_arrow(false)
        .position(gtk4::PositionType::Top)
        .build();

    let menu_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .build();

    let header = Label::builder()
        .label("Papelera")
        .css_classes(["popover-menu-header"])
        .halign(gtk4::Align::Start)
        .build();
    menu_box.append(&header);

    let sep = Separator::new(Orientation::Horizontal);
    sep.add_css_class("popover-separator");
    menu_box.append(&sep);

    let btn_open = Button::builder()
        .label("Abrir en Archivos")
        .css_classes(["popover-menu-item"])
        .build();
    let p_open = popover.clone();
    btn_open.connect_clicked(move |_| {
        p_open.popdown();
        let _ = Command::new("nautilus").arg("trash://").spawn();
    });

    let btn_empty = Button::builder()
        .label("Vaciar Papelera")
        .css_classes(["popover-menu-item"])
        .build();
    let img_clone = trash_img.clone();
    let popover_clone = popover.clone();
    btn_empty.connect_clicked(move |_| {
        info!("🗑️ Vaciando papelera...");
        let _ = Command::new("gio").args(["trash", "--empty"]).status();
        img_clone.set_icon_name(Some("user-trash"));
        popover_clone.popdown();
    });

    menu_box.append(&btn_open);
    menu_box.append(&btn_empty);
    let popover_surface = create_popover_surface(&menu_box);
    popover.set_child(Some(&popover_surface));
    popover.set_parent(&btn);
    let p_unroot = popover.clone();
    btn.connect_unrealize(move |_| {
        p_unroot.unparent();
    });

    let right_click = GestureClick::new();
    right_click.set_button(3);
    let p = popover.clone();
    right_click.connect_pressed(move |_, _, _, _| {
        p.popup();
    });
    btn.add_controller(right_click);

    item_container.append(&btn);

    let spacer = Label::builder()
        .label("●")
        .css_classes(["dock-indicator"])
        .opacity(0.0)
        .valign(gtk4::Align::End)
        .build();
    item_container.append(&spacer);

    (item_container, trash_img)
}

fn create_dock_item(
    app_cfg: &DockAppConfig,
    is_pinned: bool,
    idx: usize,
    total_pinned: usize,
    icon_size: i32,
    is_running: bool,
    running_cache: Rc<RefCell<Vec<String>>>,
    active_app_cache: Rc<RefCell<Vec<String>>>,
    dock_config: Rc<RefCell<DockConfig>>,
    on_refresh: Rc<dyn Fn()>,
) -> (GtkBox, Label) {
    let item_container = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .css_classes(["dock-item-box"])
        .spacing(1)
        .valign(gtk4::Align::End)
        .build();
    item_container.set_overflow(gtk4::Overflow::Visible);

    let btn = Button::builder()
        .css_classes(["dock-item-btn"])
        .focusable(false)
        .valign(gtk4::Align::End)
        .build();
    btn.set_overflow(gtk4::Overflow::Visible);

    let icon_name = match app_cfg.icon.as_str() {
        "system-file-manager" => "org.gnome.Nautilus",
        "utilities-terminal" => "org.gnome.Ptyxis",
        "accessories-text-editor" => "org.gnome.TextEditor",
        icon_name => icon_name,
    };
    let icon = if icon_name.starts_with('/') {
        Image::builder()
            .file(icon_name)
            .pixel_size(icon_size)
            .valign(gtk4::Align::End)
            .build()
    } else {
        Image::builder()
            .icon_name(icon_name)
            .pixel_size(icon_size)
            .valign(gtk4::Align::End)
            .build()
    };
    let show_backdrop = should_show_dock_icon_background(
        app_cfg.icon_appearance,
        icon_name,
        &icon,
        icon_size,
    );
    if show_backdrop {
        icon.add_css_class("dock-icon-inset");
        icon.set_pixel_size((icon_size - 10).max(1));
        icon.set_margin_bottom(5);
    }
    let icon_container = create_magnifiable_icon(
        &icon,
        icon_size,
        show_backdrop,
    );
    btn.set_child(Some(&icon_container));
    icon_container.add_css_class("dock-bounce-target");
    let animation_id = Rc::new(Cell::new(0_u64));
    let cmd = app_cfg.command.clone();
    let name = app_cfg.name.clone();
    let matchers = app_cfg.matchers.clone();
    let cache = running_cache;

    let activate_or_spawn = {
        let cmd = cmd.clone();
        let name = name.clone();
        let matchers = matchers.clone();
        let cache = cache.clone();
        let active_app_cache = active_app_cache.clone();
        let icon_container = icon_container.clone().upcast::<gtk4::Widget>();
        let animation_id = animation_id.clone();
        Rc::new(move || {
            let active_app_matches = active_app_cache
                .borrow()
                .iter()
                .any(|identity| matches_dock_app(&matchers, &name, identity));
            if active_app_matches {
                info!("{} ya tiene el foco; no se envía acción de dock", name);
                return;
            }

            let running = cache.borrow();
            let is_currently_running = running.iter().any(|r| {
                let r_lower = r.to_lowercase();
                matchers.iter().any(|m| r_lower.contains(&m.to_lowercase()))
            });
            drop(running);

            if is_currently_running && !matchers.is_empty() {
                info!("App ya en ejecución, enviando IPC ActivateApp para {}", name);
                let _ = send_ipc_message(&IpcMessage::ActivateApp {
                    app_id: matchers[0].clone(),
                });
                return;
            }

            bounce_dock_icon(
                &icon_container,
                animation_id.clone(),
            );
            info!("Lanzando aplicación desde Dock: {} (`{}`)", name, cmd);
            spawn_dock_app(&name, &cmd, &[]);
        })
    };

    let act_click = activate_or_spawn.clone();
    btn.connect_clicked(move |_| {
        act_click();
    });

    // Menú Popover de Clic Derecho (macOS Style Context Menu)
    let popover = Popover::builder()
        .css_classes(["malus-popover-menu"])
        .has_arrow(false)
        .position(gtk4::PositionType::Top)
        .build();

    let menu_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(2)
        .build();

    // 1. Título de la app en la cabecera
    let header = Label::builder()
        .label(&app_cfg.name)
        .css_classes(["popover-menu-header"])
        .halign(gtk4::Align::Start)
        .build();
    menu_box.append(&header);

    let sep1 = Separator::new(Orientation::Horizontal);
    sep1.add_css_class("popover-separator");
    menu_box.append(&sep1);

    // 2. Acción Abrir / Traer al frente
    let btn_open = Button::builder()
        .label(if is_running { "Traer al frente" } else { "Abrir" })
        .css_classes(["popover-menu-item"])
        .build();
    let p_open = popover.clone();
    let act_open = activate_or_spawn;
    btn_open.connect_clicked(move |_| {
        p_open.popdown();
        act_open();
    });
    menu_box.append(&btn_open);

    for (label, args) in dock_app_context_actions(app_cfg) {
        let btn_action = Button::builder()
            .label(label)
            .css_classes(["popover-menu-item"])
            .build();
        let p_action = popover.clone();
        let action_name = name.clone();
        let action_command = cmd.clone();
        btn_action.connect_clicked(move |_| {
            p_action.popdown();
            spawn_dock_app(&action_name, &action_command, args);
        });
        menu_box.append(&btn_action);
    }

    // 3. Opciones de anclado / organización estilo macOS
    if is_pinned {
        if idx > 0 {
            let btn_move_left = Button::builder()
                .label("Mover a la izquierda")
                .css_classes(["popover-menu-item"])
                .build();
            let p_mv_l = popover.clone();
            let cfg_mv_l = dock_config.clone();
            let on_rf_mv_l = on_refresh.clone();
            btn_move_left.connect_clicked(move |_| {
                p_mv_l.popdown();
                let mut cfg = cfg_mv_l.borrow_mut();
                if cfg.move_app(idx, idx - 1) {
                    let _ = cfg.save();
                    drop(cfg);
                    on_rf_mv_l();
                }
            });
            menu_box.append(&btn_move_left);
        }

        if idx + 1 < total_pinned {
            let btn_move_right = Button::builder()
                .label("Mover a la derecha")
                .css_classes(["popover-menu-item"])
                .build();
            let p_mv_r = popover.clone();
            let cfg_mv_r = dock_config.clone();
            let on_rf_mv_r = on_refresh.clone();
            btn_move_right.connect_clicked(move |_| {
                p_mv_r.popdown();
                let mut cfg = cfg_mv_r.borrow_mut();
                if cfg.move_app(idx, idx + 1) {
                    let _ = cfg.save();
                    drop(cfg);
                    on_rf_mv_r();
                }
            });
            menu_box.append(&btn_move_right);
        }

        let btn_unpin = Button::builder()
            .label("Eliminar del Dock")
            .css_classes(["popover-menu-item"])
            .build();
        let p_unpin = popover.clone();
        let cfg_unpin = dock_config.clone();
        let name_unpin = app_cfg.name.clone();
        let on_rf_unpin = on_refresh.clone();
        btn_unpin.connect_clicked(move |_| {
            p_unpin.popdown();
            let mut cfg = cfg_unpin.borrow_mut();
            if cfg.unpin_app(&name_unpin) {
                let _ = cfg.save();
                drop(cfg);
                on_rf_unpin();
            }
        });
        menu_box.append(&btn_unpin);
    } else {
        let btn_pin = Button::builder()
            .label("Mantener en el Dock")
            .css_classes(["popover-menu-item"])
            .build();
        let p_pin = popover.clone();
        let cfg_pin = dock_config.clone();
        let app_to_pin = app_cfg.clone();
        let on_rf_pin = on_refresh.clone();
        btn_pin.connect_clicked(move |_| {
            p_pin.popdown();
            let mut cfg = cfg_pin.borrow_mut();
            if cfg.pin_app(app_to_pin.clone()) {
                let _ = cfg.save();
                drop(cfg);
                on_rf_pin();
            }
        });
        menu_box.append(&btn_pin);
    }

    // 4. Forzar salida si está en ejecución
    if is_running {
        let sep2 = Separator::new(Orientation::Horizontal);
        sep2.add_css_class("popover-separator");
        menu_box.append(&sep2);

        let btn_force_quit = Button::builder()
            .label("Forzar salida")
            .css_classes(["popover-menu-item", "popover-menu-item-danger"])
            .build();
        let p_fq = popover.clone();
        let matchers_fq = matchers.clone();
        let cmd_fq = cmd.clone();
        let app_name_fq = name.clone();
        btn_force_quit.connect_clicked(move |_| {
            p_fq.popdown();
            info!("🛑 Forzando salida de {}", app_name_fq);
            for m in &matchers_fq {
                let _ = Command::new("pkill").args(["-f", m]).status();
            }
            let parts: Vec<&str> = cmd_fq.split_whitespace().collect();
            if let Some((exe, _)) = parts.split_first() {
                let _ = Command::new("killall").arg(exe).status();
            }
        });
        menu_box.append(&btn_force_quit);
    }

    let popover_surface = create_popover_surface(&menu_box);
    popover.set_child(Some(&popover_surface));
    popover.set_parent(&btn);
    let p_unroot = popover.clone();
    btn.connect_unrealize(move |_| {
        p_unroot.unparent();
    });

    let right_click = GestureClick::new();
    right_click.set_button(3); // Clic derecho
    let p_rc = popover.clone();
    right_click.connect_pressed(move |_, _, _, _| {
        p_rc.popup();
    });
    btn.add_controller(right_click);

    item_container.append(&btn);

    // Indicador de app en ejecución (Punto iluminado `●`)
    let indicator = Label::builder()
        .label("●")
        .css_classes(["dock-indicator"])
        .opacity(if is_running { 1.0 } else { 0.0 })
        .valign(gtk4::Align::End)
        .build();
    item_container.append(&indicator);

    (item_container, indicator)
}

fn spawn_dock_app(name: &str, cmd: &str, extra_args: &[&str]) {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if let Some((exe, args)) = parts.split_first() {
        let target_exe = if *exe == "gnome-control-center" {
            "malus-de-settings"
        } else {
            *exe
        };
        let resolved_exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join(target_exe)))
            .filter(|p| p.exists())
            .or_else(|| {
                let rel = std::path::PathBuf::from("/home/cristian/Apps/malus-de/target/release")
                    .join(target_exe);
                if rel.exists() {
                    Some(rel)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| std::path::PathBuf::from(target_exe));
        let mut command = Command::new(&resolved_exe);
        command.args(args).args(extra_args);
        malus_de_common::config::set_cursor_environment(&mut command);
        if let Ok(wayland_disp) = std::env::var("WAYLAND_DISPLAY") {
            command.env("WAYLAND_DISPLAY", wayland_disp);
        }
        command.env("XDG_CURRENT_DESKTOP", "Malus");
        command.env("GDK_BACKEND", "wayland");
        if let Ok(xdg_rt) = std::env::var("XDG_RUNTIME_DIR") {
            command.env("XDG_RUNTIME_DIR", xdg_rt);
        }
        match command.spawn() {
            Ok(mut child) => {
                let pid = child.id();
                info!("🚀 Proceso iniciado para {}: PID={}", name, pid);
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(err) => {
                error!("❌ Error al lanzar aplicación {} desde Dock: {:?}", name, err);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, command: &str) -> DockAppConfig {
        DockAppConfig {
            name: name.to_string(),
            icon: String::new(),
            command: command.to_string(),
            matchers: Vec::new(),
            icon_appearance: DockIconAppearance::Auto,
        }
    }

    #[test]
    fn brave_has_incognito_action() {
        assert_eq!(
            dock_app_context_actions(&app("Brave", "brave-browser")),
            vec![("Abrir en modo incógnito", &["--incognito"][..])]
        );
    }

    #[test]
    fn code_editors_have_empty_window_action() {
        for app in [
            app("Visual Studio Code", "code"),
            app("Antigravity", "antigravity"),
        ] {
            assert_eq!(
                dock_app_context_actions(&app),
                vec![("Abrir nueva ventana vacía", &["--new-window"][..])]
            );
        }
    }

    #[test]
    fn nautilus_has_new_window_action() {
        assert_eq!(
            dock_app_context_actions(&app("Archivos", "nautilus")),
            vec![("Abrir nueva ventana", &["--new-window"][..])]
        );
    }

    #[test]
    fn active_app_identity_matches_dock_matchers_case_insensitively() {
        assert!(matches_dock_app(
            &["org.gnome.nautilus".to_string()],
            "Archivos",
            "org.gnome.Nautilus"
        ));
        assert!(!matches_dock_app(
            &["org.gnome.nautilus".to_string()],
            "Archivos",
            "org.gnome.Calculator"
        ));
    }
}