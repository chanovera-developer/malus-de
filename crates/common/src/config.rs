use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

/// Configuración global de Malus DE
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MalusConfig {
    pub appearance: AppearanceConfig,
    pub dock: DockConfig,
    pub top_bar: TopBarConfig,
    pub input: InputConfig,
    pub tiling: TilingConfig,
    pub link: LinkConfig,
    pub power: PowerConfig,
    pub displays: DisplaysConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceConfig {
    pub theme: String,
    pub dark_mode: bool,
    pub button_layout_left: bool,
    pub acrylic_blur: bool,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: "default-light".to_string(),
            dark_mode: false,
            button_layout_left: true,
            acrylic_blur: true,
        }
    }
}

impl AppearanceConfig {
    /// Formato de botones de ventana para GNOME / GTK:
    /// - Izquierda (macOS): `close,minimize,maximize:`
    /// - Derecha (Windows/GNOME): `:minimize,maximize,close`
    pub fn button_layout_string(&self) -> &'static str {
        if self.button_layout_left {
            "close,minimize,maximize:"
        } else {
            ":minimize,maximize,close"
        }
    }
}

/// Reglas CSS de controles de ventana macOS (Traffic Lights) para tema Claro
pub const MALUS_GTK_CSS_WINDOWCONTROLS_LIGHT: &str = r#"/* Controles de ventana estilo macOS (Claro) */
windowcontrols {
    border-spacing: 8px;
    padding: 0;
    margin-top: 10px;
    margin-bottom: 10px;
}

windowcontrols.start {
    margin-left: 10px;
    margin-right: 12px;
}

windowcontrols.end {
    margin-right: 10px;
    margin-left: 12px;
}

windowcontrols > button,
headerbar button.titlebutton {
    min-width: 13px;
    min-height: 13px;
    padding: 0;
    margin: 0;
    border-radius: 9999px;
    border-style: solid;
    border-width: 0.5px;
    box-shadow: none;
    outline: none;
    background-clip: padding-box;
    transition: background-color 100ms ease, border-color 100ms ease, opacity 100ms ease;
}

windowcontrols > button > image,
headerbar button.titlebutton > image {
    padding: 0;
    margin: 0;
    min-width: 13px;
    min-height: 13px;
    border-radius: 9999px;
    opacity: 0;
    transition: opacity 120ms ease;
    background-color: transparent;
    box-shadow: none;
    -gtk-icon-size: 8px;
}

windowcontrols:hover > button > image,
windowcontrols > button:hover > image,
headerbar:hover button.titlebutton > image,
headerbar button.titlebutton:hover > image {
    opacity: 0.85;
}

windowcontrols > button.close,
headerbar button.titlebutton.close {
    background-color: #ff5f56;
    border-color: #e0443e;
}
windowcontrols > button.close:hover,
headerbar button.titlebutton.close:hover {
    background-color: #ff473d;
    border-color: #c93630;
}
windowcontrols > button.close:active,
headerbar button.titlebutton.close:active {
    background-color: #bf3933;
    border-color: #a82f2a;
}
windowcontrols > button.close > image,
headerbar button.titlebutton.close > image {
    color: #4c0000;
}

windowcontrols > button.minimize,
headerbar button.titlebutton.minimize {
    background-color: #ffbd2e;
    border-color: #dea123;
}
windowcontrols > button.minimize:hover,
headerbar button.titlebutton.minimize:hover {
    background-color: #ffb114;
    border-color: #c48c18;
}
windowcontrols > button.minimize:active,
headerbar button.titlebutton.minimize:active {
    background-color: #bf8516;
    border-color: #9e6d0f;
}
windowcontrols > button.minimize > image,
headerbar button.titlebutton.minimize > image {
    color: #593b00;
}

windowcontrols > button.maximize,
headerbar button.titlebutton.maximize {
    background-color: #27c93f;
    border-color: #1aab29;
}
windowcontrols > button.maximize:hover,
headerbar button.titlebutton.maximize:hover {
    background-color: #1fbd36;
    border-color: #159322;
}
windowcontrols > button.maximize:active,
headerbar button.titlebutton.maximize:active {
    background-color: #158a25;
    border-color: #0f6c1a;
}
windowcontrols > button.maximize > image,
headerbar button.titlebutton.maximize > image {
    color: #004d11;
}

window:backdrop windowcontrols > button,
.backdrop windowcontrols > button,
window:backdrop headerbar button.titlebutton,
.backdrop headerbar button.titlebutton {
    background-color: rgba(0, 0, 0, 0.12);
    border-color: rgba(0, 0, 0, 0.08);
}
window:backdrop windowcontrols > button > image,
.backdrop windowcontrols > button > image,
window:backdrop headerbar button.titlebutton > image,
.backdrop headerbar button.titlebutton > image {
    opacity: 0;
}
"#;

/// Reglas CSS de controles de ventana macOS (Traffic Lights) para tema Oscuro
pub const MALUS_GTK_CSS_WINDOWCONTROLS_DARK: &str = r#"/* Controles de ventana estilo macOS (Oscuro) */
windowcontrols {
    border-spacing: 8px;
    padding: 0;
    margin-top: 10px;
    margin-bottom: 10px;
}

windowcontrols.start {
    margin-left: 10px;
    margin-right: 12px;
}

windowcontrols.end {
    margin-right: 10px;
    margin-left: 12px;
}

windowcontrols > button,
headerbar button.titlebutton {
    min-width: 13px;
    min-height: 13px;
    padding: 0;
    margin: 0;
    border-radius: 9999px;
    border-style: solid;
    border-width: 0.5px;
    box-shadow: none;
    outline: none;
    background-clip: padding-box;
    transition: background-color 100ms ease, border-color 100ms ease, opacity 100ms ease;
}

windowcontrols > button > image,
headerbar button.titlebutton > image {
    padding: 0;
    margin: 0;
    min-width: 13px;
    min-height: 13px;
    border-radius: 9999px;
    opacity: 0;
    transition: opacity 120ms ease;
    background-color: transparent;
    box-shadow: none;
    -gtk-icon-size: 8px;
}

windowcontrols:hover > button > image,
windowcontrols > button:hover > image,
headerbar:hover button.titlebutton > image,
headerbar button.titlebutton:hover > image {
    opacity: 0.85;
}

windowcontrols > button.close,
headerbar button.titlebutton.close {
    background-color: #ff5f56;
    border-color: #e0443e;
}
windowcontrols > button.close:hover,
headerbar button.titlebutton.close:hover {
    background-color: #ff473d;
    border-color: #c93630;
}
windowcontrols > button.close:active,
headerbar button.titlebutton.close:active {
    background-color: #bf3933;
    border-color: #a82f2a;
}
windowcontrols > button.close > image,
headerbar button.titlebutton.close > image {
    color: #4c0000;
}

windowcontrols > button.minimize,
headerbar button.titlebutton.minimize {
    background-color: #ffbd2e;
    border-color: #dea123;
}
windowcontrols > button.minimize:hover,
headerbar button.titlebutton.minimize:hover {
    background-color: #ffb114;
    border-color: #c48c18;
}
windowcontrols > button.minimize:active,
headerbar button.titlebutton.minimize:active {
    background-color: #bf8516;
    border-color: #9e6d0f;
}
windowcontrols > button.minimize > image,
headerbar button.titlebutton.minimize > image {
    color: #593b00;
}

windowcontrols > button.maximize,
headerbar button.titlebutton.maximize {
    background-color: #27c93f;
    border-color: #1aab29;
}
windowcontrols > button.maximize:hover,
headerbar button.titlebutton.maximize:hover {
    background-color: #1fbd36;
    border-color: #159322;
}
windowcontrols > button.maximize:active,
headerbar button.titlebutton.maximize:active {
    background-color: #158a25;
    border-color: #0f6c1a;
}
windowcontrols > button.maximize > image,
headerbar button.titlebutton.maximize > image {
    color: #004d11;
}

window:backdrop windowcontrols > button,
.backdrop windowcontrols > button,
window:backdrop headerbar button.titlebutton,
.backdrop headerbar button.titlebutton {
    background-color: rgba(255, 255, 255, 0.18);
    border-color: rgba(255, 255, 255, 0.10);
}
window:backdrop windowcontrols > button > image,
.backdrop windowcontrols > button > image,
window:backdrop headerbar button.titlebutton > image,
.backdrop headerbar button.titlebutton > image {
    opacity: 0;
}
"#;

/// Reglas CSS de Malus DE para controles de ventana estilo macOS y recortes squircle
pub const MALUS_GTK_CSS_SQUIRCLE_RULES: &str = r#"/* Malus DE - Estilo macOS aislado para aplicaciones GTK3 y GTK4 */
windowtitle .title,
headerbar windowtitle .title {
    opacity: 0;
    font-size: 0px;
    min-height: 0px;
    margin: 0px;
    padding: 0px;
}

/* Ventanas GTK3, GTK4 y Libadwaita: esquinas rectas a nivel de widget para permitir
   que el compositor GPU de Malus DE aplique el recorte Squircle superelíptico continuo (n=4.2)
   sin doble recorte circular previo ni márgenes de sombra CSD */
window,
window.background,
window.csd,
window.unified,
window.solid-csd,
window.messagedialog,
window.dialog,
decoration,
.solid-csd,
headerbar,
headerbar.flat,
.titlebar,
tabbar,
tab-bar,
navigation-view,
navigation-view > widget,
leaflet,
flap,
toast-overlay,
adw-toolbar-view,
adw-window,
adw-application-window {
    border-radius: 0px;
    box-shadow: none;
    margin: 0px;
}

headerbar,
headerbar.flat,
.titlebar,
tabbar,
tab-bar {
    border-top-left-radius: 0px;
    border-top-right-radius: 0px;
    border-radius: 0px;
}

/* Ventanas maximizadas o en pantalla completa: esquinas rectas (0px) */
window.maximized,
window.fullscreen,
window.maximized.background,
window.fullscreen.background,
window.maximized headerbar,
window.fullscreen headerbar {
    border-radius: 0px;
}

/* Menús emergentes, popovers y context menus estilo Squircle macOS */
popover > contents,
popover.menu > contents,
popover.background > contents,
menu,
.menu {
    border-radius: 14px;
    padding: 0;
}

/* Elementos de menú con squircle highlight */
popover modelbutton,
popover .popover-menu-item,
menu menuitem,
.menu menuitem {
    border-radius: 8px;
    min-height: 26px;
    padding: 4px 10px;
    margin: 1px 2px;
}

/* Enlaces estilo macOS con squircle hover */
link,
link:hover,
link:active,
link:visited,
button.link,
button.link:hover {
    border-radius: 7px;
    padding: 2px 6px;
}
"#;

fn malus_gtk_css_popup(dark: bool) -> &'static str {
    if dark {
        r#"
popover > contents,
popover.menu > contents,
popover.background > contents,
menu,
.menu,
window.view popover,
window.view popover > contents,
window.view popover.menu > contents,
window.view popover.background > contents,
window.view menu,
window.view .menu {
    background-color: alpha(@window_bg_color, 0.62);
    border-color: alpha(@window_fg_color, 0.14);
}
"#
    } else {
        r#"
popover > contents,
popover.menu > contents,
popover.background > contents,
menu,
.menu,
window.view popover,
window.view popover > contents,
window.view popover.menu > contents,
window.view popover.background > contents,
window.view menu,
window.view .menu {
    background-color: alpha(@window_bg_color, 0.62);
    border-color: alpha(@window_fg_color, 0.12);
}
"#
    }
}

pub const MALUS_GTK_CSS_SIDEBAR_COMMON: &str = r#"/* Malus DE - Paneles laterales semitransparentes estilo macOS (Sidebar / Split View) */
window.view.background {
    background-color: transparent;
}

window.view popover,
window.view popover > contents,
window.view popover.menu > contents,
window.view popover.background > contents,
window.view menu,
window.view .menu {
    background-color: transparent;
    border-color: transparent;
    box-shadow: none;
}

/* Mantener fondo opaco sólido para vistas de contenido y ventanas generales */
adw-toolbar-view,
navigation-view,
preferencespage,
preferencesgroup,
.content-pane {
    background-color: @window_bg_color;
    color: @window_fg_color;
}

.content-pane headerbar,
navigation-split-view > .content-pane headerbar,
overlay-split-view > .content-pane headerbar {
    background-color: @headerbar_bg_color;
}

/* Diálogos modales y selectores de archivos/descargas: Fondo plano sólido estilo macOS */
window.dialog,
window.dialog.background,
window.background.dialog,
window.filechooser,
window.filechooser.background,
window.background.filechooser,
window.messagedialog,
window.messagedialog.background,
window.background.messagedialog,
window.modal,
window.sheet,
dialog,
dialog.background,
messagedialog,
messagedialog.background,
filechooser,
filechooser.background,
.file-chooser,
.dialog-vbox,
.dialog-content-area,
.dialog-action-box,
.dialog-action-area {
    background-color: @window_bg_color;
    color: @window_fg_color;
}

window.dialog headerbar,
window.filechooser headerbar,
dialog headerbar,
filechooser headerbar {
    background-color: @headerbar_bg_color;
    color: @headerbar_fg_color;
}

window.dialog actionbar,
window.dialog actionbar.background,
window.filechooser actionbar,
window.filechooser actionbar.background,
dialog actionbar,
dialog actionbar.background,
filechooser actionbar,
filechooser actionbar.background {
    background-color: @window_bg_color;
    color: @window_fg_color;
}

/* Barra lateral de lugares (placessidebar / navigation-sidebar / placesview) con fondo plano sólido en diálogos */
window.dialog placessidebar,
window.dialog placessidebar.view,
window.dialog placessidebar scrolledwindow,
window.dialog placessidebar viewport,
window.dialog placessidebar listview,
window.dialog placessidebar list,
window.dialog placessidebar listbox,
window.dialog placessidebar .view,
window.dialog placesview,
window.dialog placesview scrolledwindow,
window.dialog placesview viewport,
window.dialog placesview list,
window.dialog placesview stack,
window.filechooser placessidebar,
window.filechooser placessidebar.view,
window.filechooser placessidebar scrolledwindow,
window.filechooser placessidebar viewport,
window.filechooser placessidebar listview,
window.filechooser placessidebar list,
window.filechooser placessidebar listbox,
window.filechooser placessidebar .view,
window.filechooser placesview,
window.filechooser placesview scrolledwindow,
window.filechooser placesview viewport,
window.filechooser placesview list,
window.filechooser placesview stack,
dialog placessidebar,
dialog placessidebar.view,
dialog placessidebar scrolledwindow,
dialog placessidebar viewport,
dialog placessidebar listview,
dialog placessidebar list,
dialog placessidebar listbox,
dialog placessidebar .view,
dialog placesview,
dialog placesview scrolledwindow,
dialog placesview viewport,
dialog placesview list,
dialog placesview stack,
filechooser placessidebar,
filechooser placessidebar.view,
filechooser placessidebar scrolledwindow,
filechooser placessidebar viewport,
filechooser placessidebar listview,
filechooser placessidebar list,
filechooser placessidebar listbox,
filechooser placessidebar .view,
filechooser placesview,
filechooser placesview scrolledwindow,
filechooser placesview viewport,
filechooser placesview list,
filechooser placesview stack,
window.dialog .navigation-sidebar,
window.filechooser .navigation-sidebar,
dialog .navigation-sidebar,
filechooser .navigation-sidebar {
    background: @window_bg_color;
    background-color: @window_bg_color;
}

/* Transparencia de los componentes internos de la barra lateral de Nautilus */
window.view .sidebar-pane,
window.view .sidebar-pane *,
window.view .navigation-sidebar,
window.view .navigation-sidebar *,
window.view placessidebar,
window.view placessidebar *,
window.view .macos-sidebar,
window.view .macos-sidebar * {
    background: transparent;
    background-color: transparent;
    border-color: transparent;
    box-shadow: none;
}

window.view .sidebar-pane listview > row,
window.view .sidebar-pane list > row,
window.view .sidebar-pane listbox > row,
window.view .navigation-sidebar row,
window.view .navigation-sidebar > row,
window.view placessidebar row,
window.view placessidebar > row {
    background: transparent;
    background-color: transparent;
}

window.view .sidebar-pane listview > row:hover,
window.view .navigation-sidebar row:hover,
window.view placessidebar row:hover {
    background-color: alpha(currentColor, 0.08);
}

window.view .sidebar-pane listview > row:selected,
window.view .navigation-sidebar row:selected,
window.view placessidebar row:selected {
    background-color: @accent_bg_color;
    color: @accent_fg_color;
}
"#;

pub const MALUS_GTK_CSS_SIDEBAR_LIGHT: &str = r#"/* Estilos de panel lateral para Tema Claro */
@define-color sidebar_bg_color rgba(246, 246, 248, 0.82);
@define-color sidebar_backdrop_color rgba(240, 240, 242, 0.88);

:root {
    --sidebar-bg-color: rgba(246, 246, 248, 0.82);
    --sidebar-backdrop-color: rgba(240, 240, 242, 0.88);
}

window.view overlay-split-view > .sidebar-pane {
    background-color: rgba(246, 246, 248, 0.82);
    color: @window_fg_color;
}

window.view overlay-split-view > .sidebar-pane:backdrop {
    background-color: rgba(240, 240, 242, 0.88);
}

window.view overlay-split-view > .sidebar-pane:dir(ltr) {
    box-shadow: inset -1px 0 rgba(0, 0, 0, 0.08);
}

window.view overlay-split-view > .sidebar-pane:dir(rtl) {
    box-shadow: inset 1px 0 rgba(0, 0, 0, 0.08);
}

@media (prefers-color-scheme: dark) {
    window.view overlay-split-view > .sidebar-pane {
        background-color: rgba(30, 30, 34, 0.82);
        color: @window_fg_color;
    }

    window.view overlay-split-view > .sidebar-pane:backdrop {
        background-color: rgba(25, 25, 28, 0.88);
    }

    window.view overlay-split-view > .sidebar-pane:dir(ltr) {
        box-shadow: inset -1px 0 rgba(255, 255, 255, 0.08);
    }

    window.view overlay-split-view > .sidebar-pane:dir(rtl) {
        box-shadow: inset 1px 0 rgba(255, 255, 255, 0.08);
    }
}
"#;

pub const MALUS_GTK_CSS_SIDEBAR_DARK: &str = r#"/* Estilos de panel lateral para Tema Oscuro */
@define-color sidebar_bg_color rgba(30, 30, 34, 0.82);
@define-color sidebar_backdrop_color rgba(25, 25, 28, 0.88);

:root {
    --sidebar-bg-color: rgba(30, 30, 34, 0.82);
    --sidebar-backdrop-color: rgba(25, 25, 28, 0.88);
}

window.view overlay-split-view > .sidebar-pane {
    background-color: rgba(30, 30, 34, 0.82);
    color: @window_fg_color;
}

window.view overlay-split-view > .sidebar-pane:backdrop {
    background-color: rgba(25, 25, 28, 0.88);
}

window.view overlay-split-view > .sidebar-pane:dir(ltr) {
    box-shadow: inset -1px 0 rgba(255, 255, 255, 0.08);
}

window.view overlay-split-view > .sidebar-pane:dir(rtl) {
    box-shadow: inset 1px 0 rgba(255, 255, 255, 0.08);
}
"#;

/// Genera la hoja de estilos CSS completa de Malus DE (controles macOS + recortes squircle + paneles translúcidos) para GTK 4 / Libadwaita
pub fn get_malus_gtk_css(dark: bool) -> String {
    let (controls, sidebar) = if dark {
        (
            MALUS_GTK_CSS_WINDOWCONTROLS_DARK,
            MALUS_GTK_CSS_SIDEBAR_DARK,
        )
    } else {
        (
            MALUS_GTK_CSS_WINDOWCONTROLS_LIGHT,
            MALUS_GTK_CSS_SIDEBAR_LIGHT,
        )
    };
    format!(
        "{}\n{}\n{}\n{}\n{}",
        controls,
        MALUS_GTK_CSS_SQUIRCLE_RULES,
        MALUS_GTK_CSS_SIDEBAR_COMMON,
        malus_gtk_css_popup(dark),
        sidebar
    )
}

/// Hoja de estilos CSS nativa y válida para GTK 3 (Firefox, LibreOffice, GIMP, diálogos GTK3)
pub fn get_malus_gtk3_css(dark: bool) -> String {
    let (bg_color, fg_color, sidebar_bg, border_color, popup_opacity, popup_border_opacity) =
        if dark {
            (
                "rgb(30, 30, 34)",
                "rgb(255, 255, 255)",
                "rgb(30, 30, 34)",
                "rgba(255, 255, 255, 0.08)",
                "0.94",
                "0.14",
            )
        } else {
            (
                "rgb(246, 246, 248)",
                "rgb(0, 0, 0)",
                "rgb(246, 246, 248)",
                "rgba(0, 0, 0, 0.08)",
                "0.82",
                "0.12",
            )
        };

    format!(
        r#"/* Malus DE - Hoja de estilos GTK 3 nativa (macOS Style) */
@define-color window_bg_color {bg_color};
@define-color window_fg_color {fg_color};
@define-color headerbar_bg_color {bg_color};
@define-color headerbar_fg_color {fg_color};
@define-color sidebar_bg_color {sidebar_bg};
@define-color accent_bg_color @theme_selected_bg_color;
@define-color accent_fg_color @theme_selected_fg_color;

popover,
popover.background,
popover > contents,
menu,
.menu {{
    background-color: alpha(@window_bg_color, {popup_opacity});
    border-color: alpha(@window_fg_color, {popup_border_opacity});
    padding: 0;
}}

/* Controles de ventana macOS para GTK 3 */
headerbar button.titlebutton.close,
headerbar button.titlebutton.minimize,
headerbar button.titlebutton.maximize {{
    min-width: 13px;
    min-height: 13px;
    border-radius: 9999px;
    padding: 0;
    margin: 0 4px;
}}

/* Fondo plano sólido e impenetrable para diálogos modales y selectores de archivos en GTK 3 */
dialog,
dialog.background,
dialog.csd,
filechooser,
.file-chooser,
placesview,
placesview.vertical,
placesview frame,
placesview stack,
placesview scrolledwindow,
placesview viewport,
placesview list,
placesview listbox,
dialog placessidebar,
dialog placessidebar.view,
dialog placessidebar viewport,
dialog placessidebar list,
dialog placesview,
filechooser placessidebar,
filechooser placessidebar.view,
filechooser placessidebar viewport,
filechooser placessidebar list,
filechooser placesview,
.dialog-vbox,
.dialog-content-area,
.dialog-action-box,
.dialog-action-area {{
    background-color: @theme_bg_color;
    color: @theme_fg_color;
}}

dialog headerbar,
filechooser headerbar {{
    background-color: @theme_bg_color;
    color: @theme_fg_color;
}}

dialog actionbar,
dialog actionbar.background,
filechooser actionbar,
filechooser actionbar.background {{
    background-color: @theme_bg_color;
    color: @theme_fg_color;
}}

/* Paneles laterales GTK 3 */
placessidebar,
placessidebar.view {{
    background-color: {sidebar_bg};
    box-shadow: inset -1px 0 {border_color};
}}

"#,
        bg_color = bg_color,
        fg_color = fg_color,
        sidebar_bg = sidebar_bg,
        border_color = border_color,
        popup_opacity = popup_opacity,
        popup_border_opacity = popup_border_opacity
    )
}

fn set_gtk_decoration_layout(config_dir: &Path) -> std::io::Result<()> {
    const LAYOUT: &str = "gtk-decoration-layout=close,minimize,maximize:";

    for version in ["gtk-3.0", "gtk-4.0"] {
        let settings_path = config_dir.join(version).join("settings.ini");
        let content = match std::fs::read_to_string(&settings_path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error),
        };
        let mut lines = content.lines().map(str::to_string).collect::<Vec<_>>();

        let settings_start = lines.iter().position(|line| line.trim() == "[Settings]");
        let Some(mut start) = settings_start else {
            if !lines.is_empty() && !lines.last().map(|line| line.is_empty()).unwrap_or(false) {
                lines.push(String::new());
            }
            lines.push("[Settings]".to_string());
            lines.push(LAYOUT.to_string());
            std::fs::write(settings_path, lines.join("\n") + "\n")?;
            continue;
        };

        start += 1;
        let end = lines[start..]
            .iter()
            .position(|line| line.trim().starts_with('['))
            .map(|offset| start + offset)
            .unwrap_or(lines.len());
        if let Some(index) = (start..end).find(|&index| {
            lines[index].trim_start().starts_with("gtk-decoration-layout=")
        }) {
            lines[index] = LAYOUT.to_string();
        } else {
            lines.insert(end, LAYOUT.to_string());
        }
        std::fs::write(settings_path, lines.join("\n") + "\n")?;
    }

    Ok(())
}

fn set_gsettings_button_layout() {
    match std::process::Command::new("gsettings")
        .args([
            "set",
            "org.gnome.desktop.wm.preferences",
            "button-layout",
            "close,minimize,maximize:",
        ])
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::warn!(
            "No se pudo aplicar el layout aislado de botones en GSettings (código: {:?})",
            status.code()
        ),
        Err(error) => tracing::warn!("No se pudo ejecutar gsettings para el layout de botones: {error}"),
    }
}

/// Inicializa el entorno de configuración aislado para Malus DE en `$HOME/.config/malus-de`.
/// Configura el backend `keyfile` de GSettings y los archivos `settings.ini` de GTK3/GTK4
/// con el layout de botones a la macOS (`close,minimize,maximize:`) sin alterar dconf del host.
pub fn setup_isolated_session_environment() -> std::io::Result<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let malus_config_dir = PathBuf::from(&home).join(".config").join("malus-de");

    std::env::set_var("XDG_CONFIG_HOME", &malus_config_dir);
    std::env::set_var("GSETTINGS_BACKEND", "keyfile");
    std::env::set_var("GTK_USE_PORTAL", "1");

    // 1. Directorio glib-2.0/settings para el backend GSettings keyfile (preservar si ya existe)
    let keyfile_dir = malus_config_dir.join("glib-2.0").join("settings");
    std::fs::create_dir_all(&keyfile_dir)?;
    let keyfile_path = keyfile_dir.join("keyfile");
    if !keyfile_path.exists() {
        let keyfile_content = r#"[org/gnome/desktop/wm/preferences]
button-layout='close,minimize,maximize:'

[org/gnome/desktop/interface]
color-scheme='prefer-dark'
cursor-theme='macOS'
cursor-size=24
icon-theme='Adwaita'
gtk-theme='Adwaita'

[org/gnome/settings-daemon/plugins/xsettings]
overrides={'Gtk/ShellShowsAppMenu': <1>, 'Gtk/ShellShowsMenubar': <1>}
"#;
        std::fs::write(&keyfile_path, keyfile_content)?;
    }
    let (cursor_theme, cursor_size) = load_cursor_config();

    // 2. Archivos settings.ini para GTK3 y GTK4 (preservar configuraciones existentes del usuario)
    let gtk3_dir = malus_config_dir.join("gtk-3.0");
    std::fs::create_dir_all(&gtk3_dir)?;
    let gtk3_ini = gtk3_dir.join("settings.ini");
    if !gtk3_ini.exists() {
        let gtk_ini_content = format!(
            "[Settings]\ngtk-decoration-layout=close,minimize,maximize:\ngtk-application-prefer-dark-theme=1\ngtk-cursor-theme-name={}\ngtk-cursor-theme-size={}\ngtk-theme-name=Adwaita\n",
            cursor_theme, cursor_size
        );
        std::fs::write(&gtk3_ini, gtk_ini_content)?;
    }

    let gtk4_dir = malus_config_dir.join("gtk-4.0");
    std::fs::create_dir_all(&gtk4_dir)?;
    let gtk4_ini = gtk4_dir.join("settings.ini");
    if !gtk4_ini.exists() {
        let gtk_ini_content = format!(
            "[Settings]\ngtk-decoration-layout=close,minimize,maximize:\ngtk-application-prefer-dark-theme=1\ngtk-cursor-theme-name={}\ngtk-cursor-theme-size={}\ngtk-theme-name=Adwaita\n",
            cursor_theme, cursor_size
        );
        std::fs::write(&gtk4_ini, gtk_ini_content)?;
    }

    set_gtk_decoration_layout(&malus_config_dir)?;
    set_gsettings_button_layout();

    // 2.1. Hojas de estilo GTK exclusivas de Malus DE
    let light_css = get_malus_gtk_css(false);
    let dark_css = get_malus_gtk_css(true);
    let light_gtk3_css = get_malus_gtk3_css(false);
    let dark_gtk3_css = get_malus_gtk3_css(true);

    let is_dark = if let Ok(out) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
    {
        String::from_utf8_lossy(&out.stdout).contains("prefer-dark")
    } else {
        true
    };

    let active_css = if is_dark { &dark_css } else { &light_css };
    let active_gtk3 = if is_dark { &dark_gtk3_css } else { &light_gtk3_css };

    // GTK 3
    let _ = std::fs::write(gtk3_dir.join("gtk.css"), active_gtk3);

    // GTK 4
    let _ = std::fs::write(gtk4_dir.join("gtk.css"), active_css);
    let _ = std::fs::write(gtk4_dir.join("gtk-dark.css"), &dark_css);

    // 3. Configuración de monitores y escalado fraccionario (~/.config/malus-de/monitors.toml)
    let monitors_path = malus_config_dir.join("monitors.toml");
    if !monitors_path.exists() {
        let displays_cfg = DisplaysConfig::load();
        if let Ok(toml_str) = toml::to_string_pretty(&displays_cfg) {
            let header = "# Configuración de Pantallas y Monitores para Malus DE\n# Soporte nativo para múltiples pantallas 4K con escalado fraccionario independiente.\n\n";
            let _ = std::fs::write(&monitors_path, format!("{}{}", header, toml_str));
        }
    }

    // 4. Configuración del Dock (~/.config/malus-de/dock.toml)
    let dock_path = malus_config_dir.join("dock.toml");
    if !dock_path.exists() {
        let dock_cfg = DockConfig::load_or_create_default();
        let _ = dock_cfg.save();
    }

    // 5. Configuración de Teclado y Entrada (~/.config/malus-de/input.toml)
    let input_path = malus_config_dir.join("input.toml");
    if !input_path.exists() {
        let input_cfg = InputConfig::load();
        let _ = input_cfg.save();
    }

    // Exportar variables de cursor para los procesos iniciados desde Malus DE
    std::env::set_var("XCURSOR_THEME", &cursor_theme);
    std::env::set_var("XCURSOR_SIZE", cursor_size.to_string());

    Ok(malus_config_dir)
}

pub fn load_cursor_config() -> (String, u32) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(home).join(".config"));
    let settings_path = config_home.join("glib-2.0/settings/keyfile");
    let settings = std::fs::read_to_string(settings_path).unwrap_or_default();
    let (configured_theme, configured_size) = parse_cursor_config(&settings);
    let theme = configured_theme
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var("XCURSOR_THEME").ok())
        .unwrap_or_else(|| "macOS".to_string());
    let size = configured_size
        .or_else(|| std::env::var("XCURSOR_SIZE").ok()?.parse::<u32>().ok())
        .unwrap_or(24)
        .clamp(1, 128);
    (theme, size)
}

fn parse_cursor_config(settings: &str) -> (Option<String>, Option<u32>) {
    let mut in_interface_section = false;
    let mut theme = None;
    let mut size = None;

    for line in settings.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            in_interface_section = line == "[org/gnome/desktop/interface]";
        } else if in_interface_section {
            if let Some(value) = line.strip_prefix("cursor-theme=") {
                theme = Some(value.trim().trim_matches('\'').to_string());
            } else if let Some(value) = line.strip_prefix("cursor-size=") {
                size = value.trim().parse::<u32>().ok();
            }
        }
    }
    (theme, size)
}

pub fn set_cursor_environment(command: &mut std::process::Command) {
    let (theme, size) = load_cursor_config();
    command
        .env("XCURSOR_THEME", theme)
        .env("XCURSOR_SIZE", size.to_string());
}

/// Sincroniza el modo claro/oscuro dentro de la configuración aislada de Malus DE.
pub fn sync_system_appearance(to_dark: bool) {
    let target = if to_dark { "prefer-dark" } else { "prefer-light" };
    let dark_val = if to_dark { "1" } else { "0" };
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let malus_config_dir = std::path::PathBuf::from(&home).join(".config/malus-de");
    std::env::set_var("XDG_CONFIG_HOME", &malus_config_dir);
    std::env::set_var("GSETTINGS_BACKEND", "keyfile");

    // GSettings usa el backend keyfile aislado de Malus DE.
    let _ = std::process::Command::new("gsettings")
        .args(["set", "org.gnome.desktop.interface", "color-scheme", target])
        .status();
    let _ = std::process::Command::new("gsettings")
        .args(["set", "org.gnome.desktop.interface", "gtk-theme", "Adwaita"])
        .status();

    // Keyfile de sesión aislada de Malus DE (~/.config/malus-de/glib-2.0/settings/keyfile)
    let keyfile_path = std::path::PathBuf::from(&home).join(".config/malus-de/glib-2.0/settings/keyfile");
    if keyfile_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&keyfile_path) {
            let mut lines = Vec::new();
            let mut found_color = false;
            let mut found_theme = false;
            for line in content.lines() {
                if line.starts_with("color-scheme=") {
                    lines.push(format!("color-scheme='{}'", target));
                    found_color = true;
                } else if line.starts_with("gtk-theme=") {
                    lines.push("gtk-theme='Adwaita'".to_string());
                    found_theme = true;
                } else {
                    lines.push(line.to_string());
                }
            }
            if !found_color {
                lines.push(format!("color-scheme='{}'", target));
            }
            if !found_theme {
                lines.push("gtk-theme='Adwaita'".to_string());
            }
            let _ = std::fs::write(&keyfile_path, lines.join("\n") + "\n");
        }
    }

    // Archivos settings.ini (GTK3 y GTK4) de Malus DE
    let ini_paths = [
        format!("{}/.config/malus-de/gtk-3.0/settings.ini", home),
        format!("{}/.config/malus-de/gtk-4.0/settings.ini", home),
    ];
    if let Err(error) = set_gtk_decoration_layout(&malus_config_dir) {
        tracing::warn!("No se pudo actualizar la posición de botones GTK de Malus DE: {error}");
    }
    set_gsettings_button_layout();

    for path_str in &ini_paths {
        let p = std::path::PathBuf::from(path_str);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if p.exists() {
            if let Ok(content) = std::fs::read_to_string(&p) {
                let mut lines: Vec<String> = Vec::new();
                let mut found_dark = false;
                let mut found_theme = false;
                for line in content.lines() {
                    if line.starts_with("gtk-application-prefer-dark-theme=") {
                        lines.push(format!("gtk-application-prefer-dark-theme={}", dark_val));
                        found_dark = true;
                    } else if line.starts_with("gtk-theme-name=") {
                        lines.push("gtk-theme-name=Adwaita".to_string());
                        found_theme = true;
                    } else {
                        lines.push(line.to_string());
                    }
                }
                if !found_dark {
                    lines.push(format!("gtk-application-prefer-dark-theme={}", dark_val));
                }
                if !found_theme {
                    lines.push("gtk-theme-name=Adwaita".to_string());
                }
                let _ = std::fs::write(&p, lines.join("\n") + "\n");
            }
        } else {
            let content = format!(
                "[Settings]\ngtk-decoration-layout=close,minimize,maximize:\ngtk-application-prefer-dark-theme={}\ngtk-cursor-theme-name=macOS\ngtk-cursor-theme-size=24\ngtk-theme-name=Adwaita\n",
                dark_val
            );
            let _ = std::fs::write(&p, content);
        }
    }

    // Generar y escribir CSS actualizado solo en la configuración de Malus DE.
    let light_css = get_malus_gtk_css(false);
    let dark_css = get_malus_gtk_css(true);
    let light_gtk3_css = get_malus_gtk3_css(false);
    let dark_gtk3_css = get_malus_gtk3_css(true);

    let active_gtk3 = if to_dark { &dark_gtk3_css } else { &light_gtk3_css };

    let gtk3_paths = [
        format!("{}/.config/malus-de/gtk-3.0/gtk.css", home),
    ];
    for p in &gtk3_paths {
        let _ = std::fs::write(p, active_gtk3);
    }

    let gtk4_css_paths = [
        format!("{}/.config/malus-de/gtk-4.0/gtk.css", home),
    ];
    for p in &gtk4_css_paths {
        let _ = std::fs::write(p, if to_dark { &dark_css } else { &light_css });
    }

    let gtk4_dark_paths = [
        format!("{}/.config/malus-de/gtk-4.0/gtk-dark.css", home),
    ];
    for p in &gtk4_dark_paths {
        let _ = std::fs::write(p, &dark_css);
    }

    // Notificar a GTK tocando los archivos CSS
    for p in gtk3_paths.iter().chain(gtk4_css_paths.iter()).chain(gtk4_dark_paths.iter()) {
        let _ = std::process::Command::new("touch").arg(p).status();
    }

}

/// Configuración de una aplicación anclada en el Dock
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DockAppConfig {
    pub name: String,
    pub icon: String,
    pub command: String,
    #[serde(default)]
    pub matchers: Vec<String>,
    #[serde(default)]
    pub icon_appearance: DockIconAppearance,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockIconAppearance {
    #[default]
    Auto,
    DockBackground,
    Native,
}

impl DockAppConfig {
    pub fn matches(&self, identifier: &str) -> bool {
        let id_lower = identifier.to_lowercase();
        if self.name.to_lowercase() == id_lower {
            return true;
        }
        for m in &self.matchers {
            if id_lower.contains(&m.to_lowercase()) {
                return true;
            }
        }
        false
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DockConfig {
    pub position: DockPosition,
    pub style: DockStyle,
    pub icon_size: u32,
    pub magnification_enabled: bool,
    pub magnification_max_size: u32,
    pub floating_gap: u32,
    pub corner_radius: u32,
    pub background_opacity: f32,
    pub blur_strength: u32,
    pub genie_effect: bool,
    pub indicator_style: DockIndicatorStyle,
    pub apps: Vec<DockAppConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockPosition {
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockStyle {
    Floating,
    Panel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockIndicatorStyle {
    Dot,
    Dash,
    Glow,
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            position: DockPosition::Bottom,
            style: DockStyle::Floating,
            icon_size: 48,
            magnification_enabled: true,
            magnification_max_size: 62,
            floating_gap: 4,
            corner_radius: 24,
            background_opacity: 0.70,
            blur_strength: 24,
            genie_effect: true,
            indicator_style: DockIndicatorStyle::Dot,
            apps: vec![
                DockAppConfig {
                    name: "Archivos".to_string(),
                    icon: "org.gnome.Nautilus".to_string(),
                    command: "nautilus --new-window".to_string(),
                    matchers: vec!["nautilus".into(), "archivos".into(), "org.gnome.nautilus".into()],
                    icon_appearance: DockIconAppearance::Auto,
                },
                DockAppConfig {
                    name: "Terminal".to_string(),
                    icon: "org.gnome.Ptyxis".to_string(),
                    command: "ptyxis -s".to_string(),
                    matchers: vec!["ptyxis".into(), "terminal".into(), "org.gnome.ptyxis".into()],
                    icon_appearance: DockIconAppearance::Auto,
                },
                DockAppConfig {
                    name: "Editor de Texto".to_string(),
                    icon: "org.gnome.TextEditor".to_string(),
                    command: "gnome-text-editor -s".to_string(),
                    matchers: vec!["texteditor".into(), "text-editor".into(), "gedit".into(), "org.gnome.texteditor".into()],
                    icon_appearance: DockIconAppearance::Auto,
                },
                DockAppConfig {
                    name: "Calculadora".to_string(),
                    icon: "org.gnome.Calculator".to_string(),
                    command: "gnome-calculator".to_string(),
                    matchers: vec!["calculator".into(), "calculadora".into(), "org.gnome.calculator".into()],
                    icon_appearance: DockIconAppearance::Auto,
                },
                DockAppConfig {
                    name: "Ajustes del Sistema".to_string(),
                    icon: "preferences-system".to_string(),
                    command: "malus-de-settings".to_string(),
                    matchers: vec!["malus-de-settings".into(), "dev.malus.settings".into(), "ajustes".into(), "settings".into(), "configuración".into(), "configuracion".into()],
                    icon_appearance: DockIconAppearance::Auto,
                },
            ],
        }
    }
}

impl DockConfig {
    /// Determina la ruta canónica del archivo `dock.toml`
    pub fn config_path() -> PathBuf {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            let p1 = PathBuf::from(&xdg).join("dock.toml");
            let p2 = PathBuf::from(&xdg).join("malus-de").join("dock.toml");
            if p1.exists() {
                return p1;
            }
            if p2.exists() {
                return p2;
            }
            if PathBuf::from(&xdg).ends_with("malus-de") {
                return p1;
            } else {
                return p2;
            }
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config").join("malus-de").join("dock.toml")
    }

    /// Carga la configuración desde `dock.toml`, o crea el archivo con la configuración predeterminada si no existe
    pub fn load_or_create_default() -> Self {
        let path = Self::config_path();
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(cfg) = toml::from_str::<DockConfig>(&content) {
                    return cfg;
                }
            }
        }
        let default_cfg = Self::default();
        let _ = default_cfg.save();
        default_cfg
    }

    /// Guarda la configuración actual en el archivo `dock.toml`
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let header = "# Configuración del Dock de Malus DE\n# Personalización de tamaño de iconos, magnificación, gap flotante y aplicaciones ancladas.\n# icon_appearance por aplicación: auto, dock_background o native.\n\n";
        std::fs::write(path, format!("{}{}", header, toml_str))
    }

    /// Ancla una aplicación al final del dock si no está ya presente
    pub fn pin_app(&mut self, app: DockAppConfig) -> bool {
        if self.apps.iter().any(|a| a.name.eq_ignore_ascii_case(&app.name) || (!a.command.is_empty() && a.command == app.command)) {
            return false;
        }
        self.apps.push(app);
        true
    }

    /// Desancla una aplicación del dock por su nombre
    pub fn unpin_app(&mut self, name: &str) -> bool {
        let prev_len = self.apps.len();
        self.apps.retain(|a| !a.name.eq_ignore_ascii_case(name));
        self.apps.len() < prev_len
    }

    /// Mueve una aplicación de posición (reordenar)
    pub fn move_app(&mut self, from_idx: usize, to_idx: usize) -> bool {
        if from_idx < self.apps.len() && to_idx < self.apps.len() && from_idx != to_idx {
            let item = self.apps.remove(from_idx);
            self.apps.insert(to_idx, item);
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopBarConfig {
    pub height: u32,
    pub global_menu: bool,
    pub clock_show_seconds: bool,
}

impl Default for TopBarConfig {
    fn default() -> Self {
        Self {
            height: 28,
            global_menu: true,
            clock_show_seconds: false,
        }
    }
}

fn default_keyboard_layout() -> String {
    "latam".to_string()
}

fn default_keyboard_model() -> String {
    "pc105".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InputConfig {
    /// Distribución de teclado XKB activa ("latam" = Español Latinoamericano, "es" = Español España, "us" = Inglés EE. UU.)
    #[serde(default = "default_keyboard_layout")]
    pub keyboard_layout: String,
    /// Variante XKB opcional (ej. "" o "deadtilde")
    #[serde(default)]
    pub keyboard_variant: String,
    /// Modelo físico de teclado XKB (por defecto "pc105")
    #[serde(default = "default_keyboard_model")]
    pub keyboard_model: String,
    /// Opciones XKB adicionales (ej. Some("grp:alt_shift_toggle"))
    #[serde(default)]
    pub keyboard_options: Option<String>,
    pub natural_scrolling: bool,
    pub shake_to_find_cursor: bool,
    pub show_desktop_gesture: bool,
    pub hot_corners: HotCornersConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotCornersConfig {
    pub enabled: bool,
    pub delay_ms: u32,
    pub top_left: HotCornerAction,
    pub top_right: HotCornerAction,
    pub bottom_left: HotCornerAction,
    pub bottom_right: HotCornerAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotCornerAction {
    None,
    MissionControl,
    NotificationCenter,
    ShowDesktop,
    LockScreen,
}

impl Default for HotCornersConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            delay_ms: 150,
            top_left: HotCornerAction::MissionControl,
            top_right: HotCornerAction::NotificationCenter,
            bottom_left: HotCornerAction::LockScreen,
            bottom_right: HotCornerAction::ShowDesktop,
        }
    }
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            keyboard_layout: default_keyboard_layout(),
            keyboard_variant: String::new(),
            keyboard_model: default_keyboard_model(),
            keyboard_options: None,
            natural_scrolling: true,
            shake_to_find_cursor: true,
            show_desktop_gesture: true,
            hot_corners: HotCornersConfig::default(),
        }
    }
}

impl InputConfig {
    pub fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config/malus-de/input.toml")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(mut cfg) = toml::from_str::<InputConfig>(&content) {
                    if cfg.keyboard_layout.trim().is_empty() {
                        cfg.keyboard_layout = "latam".to_string();
                    }
                    return cfg;
                }
            }
        }
        let cfg = Self::default();
        let _ = cfg.save();
        cfg
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let header = "# Configuración de Teclado y Entrada para Malus DE\n# Soporte nativo para Español Latinoamericano (latam), Español (es) e Inglés (us).\n\n";
        std::fs::write(path, format!("{}{}", header, toml_str))
    }

    pub fn layout_display_name(&self) -> &'static str {
        match self.keyboard_layout.as_str() {
            "latam" => "🇲🇽 Español (Latinoamérica)",
            "es" => "🇪🇸 Español (España)",
            "us" => "🇺🇸 Inglés (EE. UU.)",
            _ => "⌨️ Personalizado",
        }
    }

    pub fn layout_short_badge(&self) -> &'static str {
        match self.keyboard_layout.as_str() {
            "latam" => "LAT",
            "es" => "ES",
            "us" => "US",
            _ => "KB",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TilingConfig {
    pub enabled: bool,
    pub gaps: u32,
    pub default_layout: TilingLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TilingLayout {
    BinaryTree,
    Columns,
}

impl Default for TilingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            gaps: 12,
            default_layout: TilingLayout::BinaryTree,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkConfig {
    pub device_name: String,
    pub auto_clipboard: bool,
    pub pause_on_call: bool,
    pub sync_notifications: bool,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            device_name: "Malus Linux".to_string(),
            auto_clipboard: true,
            pause_on_call: true,
            sync_notifications: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerConfig {
    pub default_profile: PowerProfile,
    pub auto_power_saver_on_low_battery: bool,
    pub low_battery_threshold: u8,
    pub reduce_effects_on_power_saver: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerProfile {
    Performance,
    Balanced,
    PowerSaver,
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            default_profile: PowerProfile::Balanced,
            auto_power_saver_on_low_battery: true,
            low_battery_threshold: 20,
            reduce_effects_on_power_saver: true,
        }
    }
}

impl Default for MalusConfig {
    fn default() -> Self {
        Self {
            appearance: AppearanceConfig::default(),
            dock: DockConfig::default(),
            top_bar: TopBarConfig::default(),
            input: InputConfig::default(),
            tiling: TilingConfig::default(),
            link: LinkConfig::default(),
            power: PowerConfig::default(),
            displays: DisplaysConfig::default(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_master_scale() -> f64 {
    1.0
}

fn default_brightness() -> f64 {
    80.0
}

fn default_screen_corner_radius() -> f64 {
    32.0
}

fn default_window_corner_radius() -> f64 {
    32.0
}

/// Configuración multi-monitor con escalado fraccionario independiente y Enlace Físico 1:1
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplaysConfig {
    /// Activa el enlace matemático 1:1 para que ventanas y textos midan los mismos milímetros físicos en todos los monitores
    #[serde(default = "default_true")]
    pub linked_physical_1to1: bool,
    /// Escala maestra aplicada al monitor principal (1.0 = 100%, 1.666667 = 166.7%, 2.0 = 200%)
    #[serde(default = "default_master_scale")]
    pub master_scale: f64,
    /// Nivel de brillo de pantalla configurado (5.0 a 100.0)
    #[serde(default = "default_brightness")]
    pub brightness: f64,
    /// Radio de curvatura squircle de las esquinas del escritorio (en puntos lógicos, ej. 24.0 o 32.0). 0.0 desactiva el recorte.
    #[serde(default = "default_screen_corner_radius")]
    pub screen_corner_radius: f64,
    /// Radio de curvatura squircle de las ventanas (en puntos lógicos, ej. 32.0). 0.0 desactiva el recorte.
    #[serde(default = "default_window_corner_radius")]
    pub window_corner_radius: f64,
    /// Lista de pantallas configuradas
    pub outputs: Vec<DisplayOutputConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayOutputConfig {
    pub connector: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub refresh_rate: f64,
    #[serde(default)]
    pub modes: Vec<DisplayModeConfig>,
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    pub primary: bool,
    /// Diagonal física real del panel en pulgadas (permite corregir datos genéricos de EDID)
    #[serde(default)]
    pub diagonal_inches: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DisplayModeConfig {
    pub width: u32,
    pub height: u32,
    pub refresh_rate: f64,
}

impl DisplayOutputConfig {
    /// Obtiene la diagonal real o estimada en pulgadas
    pub fn effective_diagonal(&self) -> f64 {
        if let Some(d) = self.diagonal_inches {
            return d;
        }
        let n = self.name.to_lowercase();
        if n.contains("27") {
            27.0
        } else if n.contains("24") || n.contains("mi monitor") {
            24.0
        } else if n.contains("32") {
            32.0
        } else {
            27.0
        }
    }

    /// Calcula la densidad física de píxeles por pulgada (PPI)
    pub fn ppi(&self) -> f64 {
        let diag = self.effective_diagonal();
        if diag <= 0.0 {
            return 163.0;
        }
        let diag_px = ((self.width as f64).powi(2) + (self.height as f64).powi(2)).sqrt();
        diag_px / diag
    }

    /// Dimensiones físicas aproximadas del panel en milímetros
    pub fn physical_size_mm(&self) -> (f64, f64) {
        let ppi = self.ppi();
        let w_mm = (self.width as f64 / ppi) * 25.4;
        let h_mm = (self.height as f64 / ppi) * 25.4;
        (w_mm, h_mm)
    }

    /// Resolución lógica en el espacio virtual del compositor
    pub fn logical_size(&self) -> (i32, i32) {
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        (
            (self.width as f64 / s).round() as i32,
            (self.height as f64 / s).round() as i32,
        )
    }
}

impl Default for DisplaysConfig {
    fn default() -> Self {
        Self {
            linked_physical_1to1: true,
            master_scale: 1.0,
            brightness: 80.0,
            screen_corner_radius: 32.0,
            window_corner_radius: 32.0,
            outputs: Vec::new(),
        }
    }
}

impl DisplaysConfig {
    /// Carga la configuración de monitores desde `~/.config/malus-de/monitors.toml`,
    /// o como fallback parsea `~/.config/monitors.xml` de GNOME.
    pub fn load() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let toml_path = PathBuf::from(&home).join(".config/malus-de/monitors.toml");
        if toml_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&toml_path) {
                if let Ok(mut config) = toml::from_str::<DisplaysConfig>(&content) {
                    config.recalculate_layout();
                    return config;
                }
            }
        }

        // Fallback: intentar parsear ~/.config/monitors.xml
        let xml_path = PathBuf::from(&home).join(".config/monitors.xml");
        if xml_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&xml_path) {
                if let Some(mut config) = Self::from_gnome_xml(&content) {
                    if config.linked_physical_1to1 {
                        config.apply_1to1_link();
                    } else {
                        config.recalculate_layout();
                    }
                    return config;
                }
            }
        }

        Self::default()
    }

    /// Recalcula las coordenadas lógicas X e Y de las pantallas para mantenerlas adyacentes
    /// sin huecos muertos ni solapamientos cuando cambia la escala o el orden horizontal.
    pub fn recalculate_layout(&mut self) {
        if self.outputs.is_empty() {
            return;
        }

        // Ordenar índices de izquierda a derecha según su coordenada X actual
        let mut order: Vec<usize> = (0..self.outputs.len()).collect();
        order.sort_by_key(|&idx| (self.outputs[idx].x, !self.outputs[idx].primary));

        let mut cursor_x: i32 = 0;
        for idx in order {
            self.outputs[idx].x = cursor_x;
            let (lw, _) = self.outputs[idx].logical_size();
            cursor_x += lw.max(100);
        }

        // Normalizar coordenadas verticales Y para que al menos un monitor toque el borde superior (y = 0)
        if let Some(min_y) = self.outputs.iter().map(|o| o.y).min() {
            for o in &mut self.outputs {
                o.y = (o.y - min_y).max(0);
            }
        }
    }

    /// Devuelve el preset enlazado activo (Some(1), Some(2), o None)
    pub fn current_linked_preset(&self) -> Option<u8> {
        if !self.linked_physical_1to1 || self.outputs.is_empty() {
            return None;
        }
        let primary_scale = self.outputs.iter().find(|output| output.primary)?.scale;
        if (primary_scale - 1.5).abs() < 0.015 {
            Some(1)
        } else if (primary_scale - 1.333333).abs() < 0.015 {
            Some(2)
        } else {
            None
        }
    }

    /// Aplica un preset de escala enlazada a partir de la pantalla principal.
    pub fn apply_linked_preset(&mut self, preset: u8) {
        let scale = match preset {
            1 => 1.5,
            2 => 1.333333,
            _ => return,
        };
        self.linked_physical_1to1 = true;
        if self.outputs.is_empty() {
            self.master_scale = scale;
            return;
        }
        let primary_idx = self.outputs.iter().position(|output| output.primary).unwrap_or(0);
        let primary_ppi = self.outputs[primary_idx].ppi();
        self.master_scale = scale;
        for (idx, output) in self.outputs.iter_mut().enumerate() {
            output.scale = if idx == primary_idx {
                scale
            } else {
                (scale * (output.ppi() / primary_ppi)).clamp(0.75, 3.0)
            };
        }
        self.recalculate_layout();
    }

    /// Aplica el enlace físico 1:1 sincronizando la escala de todos los monitores
    /// según su densidad de píxeles (PPI) real respecto al monitor primario.
    pub fn apply_1to1_link(&mut self) {
        if !self.linked_physical_1to1 || self.outputs.is_empty() {
            self.recalculate_layout();
            return;
        }

        // Si ya coincide con un preset, mantenerlo exacto
        if let Some(preset) = self.current_linked_preset() {
            self.apply_linked_preset(preset);
            return;
        }

        let primary_idx = self.outputs.iter().position(|o| o.primary).unwrap_or(0);
        let primary_ppi = self.outputs[primary_idx].ppi();
        let primary_scale = self.master_scale.clamp(0.75, 3.0);
        self.master_scale = (primary_scale * 10000.0).round() / 10000.0;
        self.outputs[primary_idx].scale = self.master_scale;

        for (i, output) in self.outputs.iter_mut().enumerate() {
            if i == primary_idx {
                continue;
            }
            let out_ppi = output.ppi();
            // Escala 1:1 exacta en base a PPI: Escala_i = Escala_primaria * (PPI_i / PPI_primaria)
            let linked_scale = (primary_scale * (out_ppi / primary_ppi)).clamp(0.75, 3.0);
            output.scale = (linked_scale * 10000.0).round() / 10000.0;
        }

        self.recalculate_layout();
    }

    /// Actualiza la escala maestra unificada y recalcula ambas pantallas 1:1
    pub fn set_master_scale(&mut self, new_scale: f64) {
        let clamped = (new_scale.clamp(0.75, 3.0) * 10000.0).round() / 10000.0;
        self.master_scale = clamped;
        if self.linked_physical_1to1 {
            self.apply_1to1_link();
        } else {
            if let Some(p) = self.outputs.iter_mut().find(|o| o.primary) {
                p.scale = clamped;
            }
            self.recalculate_layout();
        }
    }

    /// Ajusta la escala de una pantalla específica por su índice.
    /// Si `linked_physical_1to1` está activo, ajusta proporcionalmente todas las demás pantallas según su PPI.
    pub fn set_output_scale(&mut self, output_idx: usize, new_scale: f64) {
        if output_idx >= self.outputs.len() {
            return;
        }
        let clamped = (new_scale.clamp(0.75, 3.0) * 10000.0).round() / 10000.0;
        self.outputs[output_idx].scale = clamped;

        if self.linked_physical_1to1 {
            let ref_ppi = self.outputs[output_idx].ppi();
            for i in 0..self.outputs.len() {
                if i != output_idx {
                    let i_ppi = self.outputs[i].ppi();
                    let linked = (clamped * (i_ppi / ref_ppi)).clamp(0.75, 3.0);
                    self.outputs[i].scale = (linked * 10000.0).round() / 10000.0;
                }
            }
            let primary_idx = self.outputs.iter().position(|o| o.primary).unwrap_or(0);
            self.master_scale = self.outputs[primary_idx].scale;
        } else if self.outputs[output_idx].primary {
            self.master_scale = clamped;
        }

        self.recalculate_layout();
    }

    /// Ajusta una pantalla sin modificar la escala de las demás.
    pub fn set_output_scale_independent(&mut self, output_idx: usize, new_scale: f64) {
        if output_idx >= self.outputs.len() {
            return;
        }
        self.linked_physical_1to1 = false;
        self.set_output_scale(output_idx, new_scale);
    }

    /// Intercambia el orden horizontal (Izquierda <-> Derecha) de los monitores
    pub fn swap_horizontal_order(&mut self) {
        if self.outputs.len() < 2 {
            return;
        }
        let mut order: Vec<usize> = (0..self.outputs.len()).collect();
        order.sort_by_key(|&idx| (self.outputs[idx].x, !self.outputs[idx].primary));
        order.reverse();

        let mut cursor_x = 0;
        for idx in order {
            self.outputs[idx].x = cursor_x;
            let (lw, _) = self.outputs[idx].logical_size();
            cursor_x += lw.max(100);
        }
    }

    /// Aplica un modo de alineación vertical entre las pantallas ("top", "center", "bottom", "desk")
    pub fn set_vertical_alignment(&mut self, mode: &str) {
        if self.outputs.is_empty() {
            return;
        }
        let max_h = self
            .outputs
            .iter()
            .map(|o| o.logical_size().1)
            .max()
            .unwrap_or(1080);

        match mode {
            "top" => {
                for o in &mut self.outputs {
                    o.y = 0;
                }
            }
            "center" => {
                for o in &mut self.outputs {
                    let (_, lh) = o.logical_size();
                    o.y = ((max_h - lh) / 2).max(0);
                }
            }
            "bottom" => {
                for o in &mut self.outputs {
                    let (_, lh) = o.logical_size();
                    o.y = (max_h - lh).max(0);
                }
            }
            "desk" => {
                for (i, o) in self.outputs.iter_mut().enumerate() {
                    if o.primary || i == 0 {
                        o.y = 519;
                    } else {
                        o.y = 0;
                    }
                }
            }
            _ => {}
        }
        self.recalculate_layout();
    }

    /// Desplaza verticalmente un monitor específico en `delta_y` píxeles lógicos
    pub fn adjust_vertical_offset(&mut self, output_idx: usize, delta_y: i32) {
        if output_idx >= self.outputs.len() {
            return;
        }
        self.outputs[output_idx].y += delta_y;
        if let Some(min_y) = self.outputs.iter().map(|o| o.y).min() {
            for o in &mut self.outputs {
                o.y = (o.y - min_y).clamp(0, 1200);
            }
        }
    }

    /// Define qué monitor es la pantalla principal
    pub fn set_primary_output(&mut self, primary_idx: usize) {
        if primary_idx >= self.outputs.len() {
            return;
        }
        for (i, o) in self.outputs.iter_mut().enumerate() {
            o.primary = i == primary_idx;
        }
        self.master_scale = self.outputs[primary_idx].scale;
        if self.linked_physical_1to1 {
            self.apply_1to1_link();
        } else {
            self.recalculate_layout();
        }
    }

    /// Guarda la configuración actual en `$HOME/.config/malus-de/monitors.toml`
    pub fn save(&self) -> std::io::Result<()> {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let toml_path = PathBuf::from(&home).join(".config/malus-de/monitors.toml");
        if let Some(parent) = toml_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let header = "# Configuración de Pantallas y Monitores para Malus DE\n# Soporte nativo para múltiples pantallas 4K con Enlace Físico 1:1.\n\n";
        std::fs::write(toml_path, format!("{}{}", header, toml_str))
    }

    /// Parsea la configuración de pantallas desde el formato XML de GNOME
    pub fn from_gnome_xml(xml: &str) -> Option<Self> {
        let config_blocks: Vec<&str> = xml.split("<configuration>").collect();
        let active_block = config_blocks.last()?;
        let parts: Vec<&str> = active_block.split("<logicalmonitor>").collect();
        if parts.len() <= 1 {
            return None;
        }

        let mut outputs = Vec::new();
        let mut master_scale = 1.666667;

        for part in &parts[1..] {
            let x = extract_tag_value(part, "x").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
            let y = extract_tag_value(part, "y").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
            let scale = extract_tag_value(part, "scale").and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0);
            let primary = extract_tag_value(part, "primary").map(|v| v == "yes").unwrap_or(false);
            let connector = extract_tag_value(part, "connector").unwrap_or_else(|| "default".to_string());
            let product = extract_tag_value(part, "product").unwrap_or_else(|| connector.clone());
            let width = extract_tag_value(part, "width").and_then(|v| v.parse::<u32>().ok()).unwrap_or(3840);
            let height = extract_tag_value(part, "height").and_then(|v| v.parse::<u32>().ok()).unwrap_or(2160);
            let rate = extract_tag_value(part, "rate").and_then(|v| v.parse::<f64>().ok()).unwrap_or(60.0);

            let diagonal_inches = if product.contains("27") || product.contains("LG") {
                Some(27.0)
            } else if product.contains("24") || product.contains("Mi") {
                Some(24.0)
            } else {
                None
            };

            if primary {
                master_scale = scale;
            }

            outputs.push(DisplayOutputConfig {
                connector,
                name: product,
                width,
                height,
                refresh_rate: rate,
                modes: vec![DisplayModeConfig {
                    width,
                    height,
                    refresh_rate: rate,
                }],
                x,
                y,
                scale,
                primary,
                diagonal_inches,
            });
        }

        if outputs.is_empty() {
            None
        } else {
            let mut cfg = DisplaysConfig {
                linked_physical_1to1: true,
                master_scale,
                brightness: 80.0,
                screen_corner_radius: 32.0,
                window_corner_radius: 32.0,
                outputs,
            };
            cfg.apply_1to1_link();
            Some(cfg)
        }
    }

    /// Busca la configuración de un monitor por nombre de conector (p. ej. "DP-1" o "HDMI-A-1")
    pub fn find_for_connector(&self, connector: &str) -> Option<&DisplayOutputConfig> {
        let norm = connector.trim_start_matches("card1-").trim_start_matches("card0-");
        self.outputs.iter().find(|o| {
            let o_norm = o.connector.trim_start_matches("card1-").trim_start_matches("card0-");
            o_norm.eq_ignore_ascii_case(norm)
        })
    }
}

fn extract_tag_value(xml_snippet: &str, tag: &str) -> Option<String> {
    let open_tag = format!("<{}>", tag);
    let close_tag = format!("</{}>", tag);
    let start = xml_snippet.find(&open_tag)? + open_tag.len();
    let end = xml_snippet[start..].find(&close_tag)? + start;
    Some(xml_snippet[start..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_config_reads_values_from_interface_section_only() {
        let settings = "[org/example]\ncursor-theme='wrong'\ncursor-size=99\n\
[org/gnome/desktop/interface]\ncursor-theme='Adwaita'\ncursor-size=32\n";

        assert_eq!(
            parse_cursor_config(settings),
            (Some("Adwaita".to_string()), Some(32))
        );
    }

    #[test]
    fn test_gtk_decoration_layout_is_set_left_without_dropping_settings() {
        let config_dir = std::env::temp_dir().join(format!(
            "malus-de-gtk-settings-test-{}",
            std::process::id()
        ));
        let gtk3_dir = config_dir.join("gtk-3.0");
        let gtk4_dir = config_dir.join("gtk-4.0");
        std::fs::create_dir_all(&gtk3_dir).unwrap();
        std::fs::create_dir_all(&gtk4_dir).unwrap();
        std::fs::write(
            gtk3_dir.join("settings.ini"),
            "[Settings]\ngtk-decoration-layout=:close\ncustom-option=true\n",
        )
        .unwrap();
        std::fs::write(gtk4_dir.join("settings.ini"), "[Settings]\ncustom-option=true\n").unwrap();

        set_gtk_decoration_layout(&config_dir).unwrap();

        for version in ["gtk-3.0", "gtk-4.0"] {
            let settings = std::fs::read_to_string(config_dir.join(version).join("settings.ini")).unwrap();
            assert!(settings.contains("gtk-decoration-layout=close,minimize,maximize:"));
            assert!(settings.contains("custom-option=true"));
        }
        std::fs::remove_dir_all(config_dir).unwrap();
    }

    #[test]
    fn test_gtk4_transparency_is_scoped_to_nautilus_and_menus_keep_their_background() {
        let css = get_malus_gtk_css(false);

        assert!(css.contains("window.view.background"));
        assert!(css.contains("window.view overlay-split-view > .sidebar-pane"));
        assert!(css.contains("window.view .sidebar-pane *"));
        assert!(css.contains("window.view popover > contents"));
        assert!(css.contains(
            "background-color: transparent;\n    border-color: transparent;\n    box-shadow: none;"
        ));
        assert!(
            css.rfind("window.view popover > contents").unwrap()
                > css.find("window.view popover > contents").unwrap()
        );
        assert!(css.contains("border-radius: 14px;\n    padding: 0;"));
        assert!(!css.contains("background-color: alpha(@window_bg_color, 0.84);"));
        assert!(!css.contains("\nwindow.background {\n"));
        assert!(!css.contains("\n.sidebar-pane,\n"));
    }

    #[test]
    fn nautilus_popup_background_overrides_transparency_in_both_themes() {
        for (dark, background) in [
            (true, "background-color: alpha(@window_bg_color, 0.94);"),
            (false, "background-color: alpha(@window_bg_color, 0.82);"),
        ] {
            let css = get_malus_gtk_css(dark);
            let transparent_rule = css.find("window.view popover > contents").unwrap();
            let themed_rule = css.rfind("window.view popover > contents").unwrap();

            assert!(themed_rule > transparent_rule);
            assert!(css[themed_rule..].contains(background));
        }
    }

    #[test]
    fn popup_background_and_border_follow_the_active_theme_in_both_gtk_versions() {
        let dark_gtk4 = get_malus_gtk_css(true);
        let light_gtk4 = get_malus_gtk_css(false);
        let dark_gtk3 = get_malus_gtk3_css(true);
        let light_gtk3 = get_malus_gtk3_css(false);

        for (css, opacity, border_opacity) in [
            (&dark_gtk4, "0.94", "0.14"),
            (&light_gtk4, "0.82", "0.12"),
            (&dark_gtk3, "0.94", "0.14"),
            (&light_gtk3, "0.82", "0.12"),
        ] {
            assert!(css.contains(&format!(
                "background-color: alpha(@window_bg_color, {opacity});"
            )));
            assert!(css.contains(&format!(
                "border-color: alpha(@window_fg_color, {border_opacity});"
            )));
            assert!(css.contains("padding: 0;"));
        }
    }

    #[test]
    fn test_gtk3_sidebar_background_is_opaque() {
        let css = get_malus_gtk3_css(false);

        assert!(css.contains("@define-color sidebar_bg_color rgb(246, 246, 248);"));
        assert!(css.contains(
            "placessidebar,\nplacessidebar.view {\n    background-color: rgb(246, 246, 248);"
        ));
    }

    #[test]
    fn test_dock_config_default_serialization() {
        let default_cfg = DockConfig::default();
        let toml_str = toml::to_string_pretty(&default_cfg).expect("Serialización falló");
        let parsed: DockConfig = toml::from_str(&toml_str).expect("Deserialización falló");
        assert_eq!(parsed.apps.len(), 5);
        assert_eq!(parsed.icon_size, 48);
        assert_eq!(parsed.magnification_max_size, 62);
        assert_eq!(parsed.floating_gap, 10);
        assert_eq!(parsed.position, DockPosition::Bottom);
    }

    #[test]
    fn test_dock_config_partial_deserialization() {
        let partial_toml = r#"
            icon_size = 56
            magnification_enabled = false
            floating_gap = 14

            [[apps]]
            name = "Test App"
            icon = "test-icon"
            command = "test-cmd"
            matchers = ["testapp"]
        "#;
        let parsed: DockConfig = toml::from_str(partial_toml).expect("Deserialización parcial falló");
        assert_eq!(parsed.icon_size, 56);
        assert!(!parsed.magnification_enabled);
        assert_eq!(parsed.floating_gap, 14);
        assert_eq!(parsed.corner_radius, 24); // default
        assert_eq!(parsed.apps.len(), 1);
        assert_eq!(parsed.apps[0].name, "Test App");
        assert_eq!(parsed.apps[0].icon_appearance, DockIconAppearance::Auto);
    }

    #[test]
    fn test_dock_app_icon_appearance_override() {
        let config = r#"
            [[apps]]
            name = "Native Icon"
            icon = "native-icon"
            command = "native-app"
            icon_appearance = "native"
        "#;

        let parsed: DockConfig = toml::from_str(config).expect("Deserialización falló");
        assert_eq!(parsed.apps[0].icon_appearance, DockIconAppearance::Native);

        let serialized = toml::to_string(&parsed).expect("Serialización falló");
        let reparsed: DockConfig = toml::from_str(&serialized).expect("Deserialización falló");
        assert_eq!(reparsed.apps[0].icon_appearance, DockIconAppearance::Native);
    }

    #[test]
    fn test_dock_config_pin_unpin_move() {
        let mut cfg = DockConfig::default();
        let initial_count = cfg.apps.len();

        // Pin new app
        let new_app = DockAppConfig {
            name: "Spotify".to_string(),
            icon: "spotify".to_string(),
            command: "spotify".to_string(),
            matchers: vec!["spotify".into()],
            icon_appearance: DockIconAppearance::Auto,
        };
        assert!(cfg.pin_app(new_app.clone()));
        assert_eq!(cfg.apps.len(), initial_count + 1);
        // Duplicate pin should return false
        assert!(!cfg.pin_app(new_app));

        // Move app
        let last_idx = cfg.apps.len() - 1;
        assert!(cfg.move_app(last_idx, 0));
        assert_eq!(cfg.apps[0].name, "Spotify");

        // Unpin app
        assert!(cfg.unpin_app("Spotify"));
        assert_eq!(cfg.apps.len(), initial_count);
        assert!(!cfg.unpin_app("NonExistentApp"));
    }

    #[test]
    fn test_dock_app_config_matches() {
        let app = DockAppConfig {
            name: "Terminal".to_string(),
            icon: "utilities-terminal".to_string(),
            command: "ptyxis -s".to_string(),
            matchers: vec!["ptyxis".into(), "terminal".into(), "org.gnome.ptyxis".into()],
            icon_appearance: DockIconAppearance::Auto,
        };

        assert!(app.matches("Terminal"));
        assert!(app.matches("ptyxis"));
        assert!(app.matches("org.gnome.ptyxis.something"));
        assert!(!app.matches("firefox"));
    }

    #[test]
    fn test_dock_config_load_or_create_default() {
        let cfg = DockConfig::load_or_create_default();
        let path = DockConfig::config_path();
        assert!(path.exists());
        assert_eq!(cfg.apps.len(), 5);
        assert!(cfg.icon_size > 0);
    }

    #[test]
    fn test_displays_config_brightness() {
        let mut cfg = DisplaysConfig::default();
        assert_eq!(cfg.brightness, 80.0);
        cfg.brightness = 95.0;
        let toml_str = toml::to_string(&cfg).unwrap();
        let parsed: DisplaysConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.brightness, 95.0);
    }

    #[test]
    fn test_displays_config_1to1_and_arrangement() {
        let mut cfg = DisplaysConfig::default();
        cfg.outputs = vec![
            DisplayOutputConfig {
                connector: "DP-1".to_string(),
                name: "Monitor 27".to_string(),
                width: 3840,
                height: 2160,
                refresh_rate: 60.0,
                modes: Vec::new(),
                x: 0,
                y: 0,
                scale: 1.5,
                primary: true,
                diagonal_inches: Some(27.0),
            },
            DisplayOutputConfig {
                connector: "HDMI-A-1".to_string(),
                name: "Monitor 24".to_string(),
                width: 3840,
                height: 2160,
                refresh_rate: 60.0,
                modes: Vec::new(),
                x: 2560,
                y: 0,
                scale: 1.5,
                primary: false,
                diagonal_inches: Some(24.0),
            },
        ];
        cfg.outputs[0].diagonal_inches = Some(27.0);
        cfg.outputs[1].diagonal_inches = Some(24.0);
        assert!(cfg.linked_physical_1to1);
        cfg.set_output_scale(0, 1.5);
        // Monitor 0 is 27" 4K (163.18 PPI) -> scale 1.50 (logical width 2560)
        // Monitor 1 is 24" 4K (183.58 PPI) -> scale 1.50 * (27/24) = 1.6875 (logical width 2276)
        assert!((cfg.outputs[0].scale - 1.5).abs() < 0.001);
        assert!((cfg.outputs[1].scale - 1.6875).abs() < 0.001);
        assert_eq!(cfg.outputs[0].x, 0);
        assert_eq!(cfg.outputs[1].x, 2560);

        // Swap horizontal order
        cfg.swap_horizontal_order();
        assert_eq!(cfg.outputs[1].x, 0);
        assert_eq!(cfg.outputs[0].x, 2276);

        // Editing an individual output automatically unlinks the scales.
        cfg.set_output_scale_independent(1, 2.0);
        assert!(!cfg.linked_physical_1to1);
        assert!((cfg.outputs[1].scale - 2.0).abs() < 0.001);
        assert!((cfg.outputs[0].scale - 1.5).abs() < 0.001);
    }

    #[test]
    fn test_input_config_latam_default() {
        let cfg = InputConfig::default();
        assert_eq!(cfg.keyboard_layout, "latam");
        assert_eq!(cfg.layout_short_badge(), "LAT");
    }

    #[test]
    fn test_displays_config_screen_corner_radius_default() {
        let cfg = DisplaysConfig::default();
        assert!(cfg.outputs.is_empty());
        assert_eq!(cfg.screen_corner_radius, 32.0);
        assert_eq!(cfg.window_corner_radius, 32.0);
    }

    #[test]
    fn test_display_connector_matching_is_exact() {
        let mut cfg = DisplaysConfig::default();
        cfg.outputs.push(DisplayOutputConfig {
            connector: "DP-1".to_string(),
            name: "DisplayPort".to_string(),
            width: 1920,
            height: 1080,
            refresh_rate: 60.0,
            modes: Vec::new(),
            x: 0,
            y: 0,
            scale: 1.0,
            primary: true,
            diagonal_inches: None,
        });
        assert!(cfg.find_for_connector("card0-DP-1").is_some());
        assert!(cfg.find_for_connector("DP-2").is_none());
    }

    #[test]
    fn test_legacy_display_config_without_modes_still_loads() {
        let legacy = r#"
connector = "DP-1"
name = "DisplayPort"
width = 3840
height = 2160
refresh_rate = 60.0
x = 0
y = 0
scale = 2.0
primary = true
"#;
        let output: DisplayOutputConfig = toml::from_str(legacy).unwrap();
        assert!(output.modes.is_empty());
        assert_eq!(output.width, 3840);
    }
}
