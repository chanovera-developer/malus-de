use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, Box as GtkBox, CssProvider, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use libadwaita as adw;
use malus_de_common::logging::init_logging;
use tracing::info;

mod ui;
pub mod services;

const STYLE_CSS: &str = r#"
window.malus-topbar,
window.malus-topbar.background,
window.malus-topbar decoration {
    background: transparent;
    background-color: transparent;
    color: #ffffff;
    border: none;
    outline: none;
    box-shadow: none;
    margin: 0;
    padding: 0;
    font-size: 13.5px;
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    font-weight: 500;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.85), 0 0 4px rgba(0, 0, 0, 0.65);
    -gtk-icon-shadow: 0 1px 3px rgba(0, 0, 0, 0.85), 0 0 4px rgba(0, 0, 0, 0.65);
}

.topbar-content {
    min-height: 36px;
    padding-left: 10px;
    padding-right: 12px;
}

/* Contenedor menubutton: transparente, sin bordes, sin sombras ni márgenes duplicados */
menubutton,
menubutton.topbar-btn {
    background: transparent;
    background-color: transparent;
    border: none;
    box-shadow: none;
    padding: 0;
    margin: 0;
}

menubutton:hover,
menubutton.topbar-btn:hover {
    background: transparent;
    background-color: transparent;
    box-shadow: none;
}

/* Ocultar chevron / flecha indicadora en todos los menús */
menubutton arrow,
menubutton image.arrow,
menubutton .arrow,
menubutton > button arrow,
menubutton > button image.arrow,
menubutton > button > box > arrow,
menubutton > button > box > image.arrow {
    opacity: 0;
    -gtk-icon-size: 0;
    min-width: 0;
    min-height: 0;
    padding: 0;
    margin: 0;
}

/* Botón interactivo único (limpio, sin recuadros superpuestos, estilo squircle) */
button.topbar-btn,
menubutton.topbar-btn > button,
menubutton > button,
label.clock-label {
    background: transparent;
    background-color: transparent;
    border: none;
    border-radius: 6px;
    padding: 3px 9px;
    color: #ffffff;
    box-shadow: none;
    outline: none;
    transition: background-color 100ms ease;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.85), 0 0 4px rgba(0, 0, 0, 0.65);
    -gtk-icon-shadow: 0 1px 3px rgba(0, 0, 0, 0.85), 0 0 4px rgba(0, 0, 0, 0.65);
    min-height: 28px;
}

button.control-center-toggle image,
menubutton.control-center-toggle image {
    color: #ffffff;
}

button.topbar-btn:hover,
menubutton.topbar-btn > button:hover,
menubutton > button:hover,
label.clock-label:hover {
    background-color: rgba(255, 255, 255, 0.18);
    border-radius: 6px;
    box-shadow: none;
}

button.topbar-btn:active,
button.topbar-btn:checked,
menubutton.topbar-btn > button:active,
menubutton.topbar-btn > button:checked,
menubutton > button:active,
menubutton > button:checked {
    background-color: rgba(255, 255, 255, 0.28);
    border-radius: 6px;
    box-shadow: none;
}

.apple-logo-btn > button {
    padding: 3px 8px;
}

.topbar-content image,
.apple-icon,
.apple-logo-btn image,
button.topbar-btn image,
menubutton.topbar-btn image {
    -gtk-icon-shadow: 0 1px 4px rgba(0, 0, 0, 0.65), 0 0 4px rgba(0, 0, 0, 0.65);
    filter: drop-shadow(0 1px 4px rgba(0, 0, 0, 0.60));
}

.apple-icon {
    margin-bottom: 5px;
}

.active-app-name {
    font-weight: 700;
    color: #ffffff;
    padding-left: 2px;
    padding-right: 4px;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.90), 0 0 4px rgba(0, 0, 0, 0.60);
}

.global-menu-item {
    font-weight: 600;
    color: #f4f4f5;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.85), 0 0 4px rgba(0, 0, 0, 0.65);
}

.clock-label {
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    color: #ffffff;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.85), 0 0 4px rgba(0, 0, 0, 0.65);
}

.active-pill {
    background-color: rgba(59, 130, 246, 0.35);
    border: 1px solid rgba(96, 165, 250, 0.5);
}

/* Quitar fondo/borde del contenedor popover general para evitar recuadros superpuestos */
popover.malus-popover-menu,
popover.control-center-popover,
popover.light-popover {
    background-color: transparent;
    background: transparent;
    border: none;
    box-shadow: 0 14px 34px rgba(0, 0, 0, 0.60);
    padding: 0;
    border-radius: 0;
}

popover.malus-popover-menu > arrow,
popover.control-center-popover > arrow,
popover.light-popover > arrow {
    background: transparent;
    border: none;
}

/* El compositor aplica el recorte squircle a los popovers; CSS deja las esquinas rectas. */
popover.malus-popover-menu > contents {
    background-color: alpha(@window_bg_color, 0.94);
    border: 1px solid alpha(@window_fg_color, 0.14);
    border-radius: 0;
    padding: 6px;
    color: #f4f4f5;
    text-shadow: none;
}

popover.control-center-popover > contents {
    background-color: alpha(@window_bg_color, 0.95);
    border: 1px solid alpha(@window_fg_color, 0.16);
    border-radius: 0;
    padding: 12px;
    color: #f4f4f5;
    text-shadow: none;
}

/* Soporte para Modo Claro en Popovers */
popover.light-popover {
    box-shadow: 0 14px 34px rgba(0, 0, 0, 0.20);
}

popover.control-center-popover {
    box-shadow: 0 18px 44px rgba(0, 0, 0, 0.65);
}

popover.control-center-popover.light-popover {
    box-shadow: 0 18px 44px rgba(0, 0, 0, 0.22);
}

popover.light-popover.malus-popover-menu > contents,
popover.light-popover > contents,
.light-mode popover.malus-popover-menu > contents {
    background-color: alpha(@window_bg_color, 0.82);
    border: 1px solid alpha(@window_fg_color, 0.12);
    border-radius: 0;
    padding: 6px;
    color: #1d1d1f;
}

popover.light-popover.control-center-popover > contents,
.light-mode popover.control-center-popover > contents {
    background-color: alpha(@window_bg_color, 0.82);
    border: 1px solid alpha(@window_fg_color, 0.14);
    border-radius: 0;
    padding: 12px;
    color: #1d1d1f;
}

.malus-popover-menu .menu-row-label {
    color: #1d1d1f;
}

popover.malus-popover-menu.light-popover .menu-row-label {
    color: #1d1d1f;
}

popover.light-popover .popover-menu-item,
popover.light-popover button.popover-menu-item,
popover.light-popover button.flat,
popover.light-popover modelbutton,
.light-mode .popover-menu-item,
.light-mode button.popover-menu-item,
.light-mode button.flat,
.light-mode modelbutton,
popover.light-popover .menu-row-label,
.light-mode .menu-row-label {
    color: #1d1d1f;
    border-radius: 6px;
}

popover.light-popover .popover-menu-item:hover,
popover.light-popover button.popover-menu-item:hover,
popover.light-popover button.flat:hover,
popover.light-popover modelbutton:hover,
.light-mode .popover-menu-item:hover,
.light-mode button.popover-menu-item:hover,
.light-mode button.flat:hover,
.light-mode modelbutton:hover {
    background-color: #007aff;
    border-radius: 6px;
    color: #ffffff;
}

popover.light-popover .popover-menu-item:hover .menu-row-label,
popover.light-popover button.popover-menu-item:hover .menu-row-label,
.light-mode .popover-menu-item:hover .menu-row-label {
    color: #ffffff;
}

popover.light-popover .menu-shortcut-hint,
.light-mode .menu-shortcut-hint {
    color: rgba(0, 0, 0, 0.40);
}

popover.light-popover .popover-menu-item:hover .menu-shortcut-hint,
.light-mode .popover-menu-item:hover .menu-shortcut-hint {
    color: rgba(255, 255, 255, 0.90);
}

popover.light-popover .menu-separator,
.light-mode .menu-separator {
    background-color: rgba(0, 0, 0, 0.08);
}

popover.light-popover .cc-tile,
.light-mode .cc-tile {
    background-color: rgba(0, 0, 0, 0.05);
    border: 1px solid rgba(0, 0, 0, 0.06);
}

popover.light-popover .cc-tile:hover,
.light-mode .cc-tile:hover {
    background-color: rgba(0, 0, 0, 0.08);
}

popover.light-popover .cc-tile-title,
.light-mode .cc-tile-title {
    color: #1d1d1f;
}

popover.light-popover .cc-tile-subtitle,
.light-mode .cc-tile-subtitle {
    color: rgba(0, 0, 0, 0.55);
}

popover.light-popover .cc-icon-circle,
.light-mode .cc-icon-circle {
    background-color: rgba(0, 0, 0, 0.08);
    color: #1d1d1f;
}

popover.light-popover .cc-slider-card,
.light-mode .cc-slider-card {
    background-color: rgba(0, 0, 0, 0.05);
    border: 1px solid rgba(0, 0, 0, 0.06);
}

popover.light-popover .cc-subview-title,
.light-mode .cc-subview-title {
    color: #1d1d1f;
}

popover.light-popover .cc-section-label,
.light-mode .cc-section-label {
    color: rgba(0, 0, 0, 0.55);
}

popover.light-popover .cc-list-row,
.light-mode .cc-list-row {
    background-color: rgba(0, 0, 0, 0.04);
    border: 1px solid rgba(0, 0, 0, 0.05);
}

popover.light-popover .cc-list-row:hover,
.light-mode .cc-list-row:hover {
    background-color: rgba(0, 0, 0, 0.08);
    border-color: rgba(0, 0, 0, 0.10);
}

popover.light-popover .cc-list-row-active,
.light-mode .cc-list-row-active {
    background-color: rgba(0, 122, 255, 0.15);
    border: 1px solid rgba(0, 122, 255, 0.35);
}

popover.light-popover .cc-list-row-active:hover,
.light-mode .cc-list-row-active:hover {
    background-color: rgba(0, 122, 255, 0.22);
}

popover.light-popover .cc-item-title,
.light-mode .cc-item-title {
    color: #1d1d1f;
}

popover.light-popover .cc-item-subtitle,
.light-mode .cc-item-subtitle {
    color: rgba(0, 0, 0, 0.55);
}

popover.light-popover .cc-item-lock,
.light-mode .cc-item-lock {
    color: rgba(0, 0, 0, 0.40);
}

popover.light-popover .cc-footer-btn,
.light-mode .cc-footer-btn {
    color: #52525b;
}

popover.light-popover .cc-footer-btn:hover,
.light-mode .cc-footer-btn:hover {
    background-color: rgba(0, 0, 0, 0.06);
    color: #1d1d1f;
}

popover.light-popover .cc-empty-label,
.light-mode .cc-empty-label {
    color: rgba(0, 0, 0, 0.45);
}

popover.light-popover .cc-airdrop-desc,
.light-mode .cc-airdrop-desc {
    color: rgba(0, 0, 0, 0.55);
}

popover.light-popover .cc-subview-back-btn,
.light-mode .cc-subview-back-btn {
    color: #1d1d1f;
}

popover.light-popover .cc-subview-back-btn:hover,
.light-mode .cc-subview-back-btn:hover {
    background-color: rgba(0, 0, 0, 0.06);
}

popover.light-popover .cc-switch-card,
.light-mode .cc-switch-card {
    background-color: rgba(0, 0, 0, 0.04);
    border: 1px solid rgba(0, 0, 0, 0.06);
}

popover.light-popover .cc-refresh-btn,
.light-mode .cc-refresh-btn {
    color: #52525b;
}

popover.light-popover .cc-refresh-btn:hover,
.light-mode .cc-refresh-btn:hover {
    background-color: rgba(0, 0, 0, 0.06);
    color: #1d1d1f;
}

.popover-menu-item,
button.popover-menu-item,
popover.malus-popover-menu button,
popover.malus-popover-menu button.flat,
popover modelbutton {
    background: transparent;
    background-color: transparent;
    border: none;
    border-radius: 6px;
    padding: 5px 10px;
    margin: 1px 3px;
    color: #e4e4e7;
    font-size: 13px;
    font-weight: 500;
    transition: background-color 100ms ease;
    box-shadow: none;
}

.popover-menu-item:hover,
button.popover-menu-item:hover,
popover.malus-popover-menu button:hover,
popover.malus-popover-menu button.flat:hover,
popover modelbutton:hover {
    background-color: #007aff;
    border-radius: 6px;
    color: #ffffff;
    box-shadow: none;
}

.popover-menu-item:hover .menu-shortcut-hint,
button.popover-menu-item:hover .menu-shortcut-hint {
    color: rgba(255, 255, 255, 0.85);
}

/* Enlaces y botones estilo Link con bordes squircle */
link,
link:hover,
link:active,
link:visited,
button.link,
button.link:hover,
.link-btn {
    color: #007aff;
    text-decoration: none;
    border-radius: 6px;
    padding: 3px 6px;
    transition: background-color 120ms ease, color 120ms ease;
}

link:hover,
button.link:hover,
.link-btn:hover {
    background-color: rgba(0, 122, 255, 0.15);
    border-radius: 6px;
    color: #ffffff;
}

.menu-item-bold {
    font-weight: 700;
}

.menu-row-label {
    font-weight: 500;
    font-size: 13px;
}

.menu-shortcut-hint {
    font-size: 12px;
    font-weight: 400;
    color: rgba(255, 255, 255, 0.45);
    margin-left: 20px;
}

.menu-separator {
    background-color: rgba(255, 255, 255, 0.10);
    min-height: 1px;
    margin: 4px 6px;
}

.cc-tile {
    background-color: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 14px;
    padding: 8px 10px;
    transition: background-color 150ms ease;
}

.cc-tile:hover {
    background-color: rgba(255, 255, 255, 0.14);
}

.cc-tile-active {
    background-color: rgba(255, 255, 255, 0.12);
    border-color: rgba(59, 130, 246, 0.40);
}

.cc-icon-circle {
    padding: 7px;
    border-radius: 16px;
    background-color: rgba(255, 255, 255, 0.12);
    color: #e4e4e7;
}

.cc-icon-circle-active {
    background-color: #007aff;
    color: #ffffff;
}

.cc-tile-title {
    font-size: 12px;
    font-weight: 700;
    color: #ffffff;
}

.cc-tile-subtitle {
    font-size: 11px;
    font-weight: 400;
    color: rgba(255, 255, 255, 0.65);
}

.cc-slider-card {
    background-color: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 14px;
    padding: 10px 12px;
}

.cc-mute-btn {
    background: transparent;
    border: none;
    padding: 2px;
    color: #ffffff;
    border-radius: 6px;
}

.cc-mute-btn:hover {
    background-color: rgba(255, 255, 255, 0.12);
}

.cc-quick-action-btn {
    background-color: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 10px;
    padding: 7px 12px;
    font-size: 12px;
    font-weight: 600;
    color: #e4e4e7;
}

.cc-quick-action-btn:hover {
    background-color: rgba(255, 255, 255, 0.15);
    color: #ffffff;
}

scale.horizontal trough {
    min-height: 20px;
    border-radius: 10px;
    background-color: rgba(255, 255, 255, 0.16);
}

scale.horizontal highlight {
    min-height: 20px;
    border-radius: 10px;
    background-color: #ffffff;
}

scale.horizontal slider {
    min-width: 18px;
    min-height: 18px;
    border-radius: 9px;
    background-color: #ffffff;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.5);
    margin: -1px;
}

.cc-pill-btn {
    background-color: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 14px;
    padding: 4px 10px;
    font-size: 11px;
    font-weight: 600;
    color: #e4e4e7;
    transition: all 120ms ease;
}

.cc-pill-btn:hover {
    background-color: rgba(255, 255, 255, 0.16);
    color: #ffffff;
}

.cc-pill-btn-active {
    background-color: #007aff;
    border-color: #007aff;
    color: #ffffff;
}

/* ======================================================== */
/* Subvistas de navegación del Centro de Control (Wi-Fi, BT) */
/* ======================================================== */
.cc-nav-tile {
    padding: 6px 8px;
}

.cc-tile-power-btn {
    border: none;
    padding: 0;
    transition: background-color 150ms ease, transform 100ms ease;
}

.cc-tile-power-btn:hover {
    transform: scale(1.05);
}

.cc-tile-nav-btn {
    background: transparent;
    border: none;
    padding: 0 4px;
    border-radius: 6px;
    transition: background-color 120ms ease;
}

.cc-tile-nav-btn:hover {
    background-color: rgba(255, 255, 255, 0.08);
}

.cc-chevron {
    color: rgba(255, 255, 255, 0.40);
    margin-right: 2px;
}

.cc-subview {
    padding: 2px 4px;
}

.cc-subview-header {
    padding-bottom: 6px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.10);
}

.cc-back-btn {
    background: transparent;
    border: none;
    padding: 4px;
    border-radius: 6px;
    color: #e4e4e7;
    transition: background-color 120ms ease;
}

.cc-back-btn:hover {
    background-color: rgba(255, 255, 255, 0.15);
    color: #ffffff;
}

.cc-subview-title {
    font-size: 14px;
    font-weight: 700;
    color: #ffffff;
    margin-left: 6px;
}

.cc-section-header-box {
    margin-top: 8px;
    margin-bottom: 4px;
}

.cc-section-label {
    font-size: 10px;
    font-weight: 700;
    color: rgba(255, 255, 255, 0.50);
    letter-spacing: 0.5px;
}

.cc-refresh-btn {
    background: transparent;
    border: none;
    padding: 2px 4px;
    border-radius: 6px;
    color: rgba(255, 255, 255, 0.60);
    transition: all 120ms ease;
}

.cc-refresh-btn:hover {
    background-color: rgba(255, 255, 255, 0.12);
    color: #ffffff;
}

.cc-list-row {
    background-color: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.04);
    border-radius: 10px;
    padding: 6px 8px;
    transition: all 120ms ease;
}

.cc-list-row:hover {
    background-color: rgba(255, 255, 255, 0.12);
    border-color: rgba(255, 255, 255, 0.15);
}

.cc-list-row-active {
    background-color: rgba(0, 122, 255, 0.20);
    border: 1px solid rgba(0, 122, 255, 0.40);
}

.cc-list-row-active:hover {
    background-color: rgba(0, 122, 255, 0.28);
}

.cc-item-title {
    font-size: 12px;
    font-weight: 600;
    color: #f4f4f5;
}

.cc-item-subtitle {
    font-size: 10px;
    font-weight: 400;
    color: rgba(255, 255, 255, 0.60);
}

.cc-item-check {
    color: #007aff;
    margin-left: 6px;
}

.cc-item-lock {
    color: rgba(255, 255, 255, 0.45);
    margin-right: 4px;
}

.cc-footer-btn {
    background: transparent;
    border: none;
    padding: 6px 10px;
    border-radius: 6px;
    font-size: 11px;
    font-weight: 500;
    color: #a1a1aa;
    transition: all 120ms ease;
}

.cc-footer-btn:hover {
    background-color: rgba(255, 255, 255, 0.08);
    color: #ffffff;
}

.cc-empty-label {
    font-size: 11px;
    font-style: italic;
    color: rgba(255, 255, 255, 0.45);
    padding: 10px 0;
}

.cc-airdrop-desc {
    font-size: 11px;
    color: rgba(255, 255, 255, 0.65);
    line-height: 1.35;
    margin-bottom: 6px;
    padding: 0 4px;
}

.cc-airdrop-sim-btn {
    background-color: rgba(0, 122, 255, 0.15);
    border: 1px solid rgba(0, 122, 255, 0.35);
    border-radius: 10px;
    padding: 6px 10px;
    font-size: 11px;
    font-weight: 600;
    color: #60a5fa;
    transition: all 120ms ease;
    margin-top: 4px;
}

.cc-airdrop-sim-btn:hover {
    background-color: rgba(0, 122, 255, 0.28);
    color: #93c5fd;
}

.cc-media-card {
    background-color: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 14px;
    padding: 8px 10px;
}

.cc-media-art {
    background-color: rgba(255, 255, 255, 0.12);
    border-radius: 10px;
    min-width: 38px;
    min-height: 38px;
    color: #e4e4e7;
}

.cc-media-title {
    font-size: 12px;
    font-weight: 700;
    color: #ffffff;
}

.cc-media-subtitle {
    font-size: 10px;
    color: rgba(255, 255, 255, 0.65);
}

.cc-media-btn {
    background: transparent;
    border: none;
    border-radius: 6px;
    padding: 4px;
    color: rgba(255, 255, 255, 0.75);
    transition: all 120ms ease;
}

.cc-media-btn:hover {
    background-color: rgba(255, 255, 255, 0.15);
    color: #ffffff;
}

.cc-media-play-btn {
    background-color: rgba(255, 255, 255, 0.15);
    color: #ffffff;
    border-radius: 50%;
    min-width: 28px;
    min-height: 28px;
}

.cc-media-play-btn:hover {
    background-color: rgba(255, 255, 255, 0.28);
}

popover.malus-popover-menu.light-popover > contents label.menu-row-label,
popover.malus-popover-menu.light-popover > contents button.popover-menu-item label.menu-row-label {
    color: #1d1d1f;
}

popover.malus-popover-menu.light-popover > contents button.popover-menu-item:hover label.menu-row-label {
    color: #ffffff;
}

label.menu-row-label.menu-light-text {
    color: #1d1d1f;
}

label.menu-row-label.menu-dark-text {
    color: #f4f4f5;
}

label.menu-shortcut-hint.menu-light-text {
    color: rgba(0, 0, 0, 0.40);
}

label.menu-shortcut-hint.menu-dark-text {
    color: rgba(255, 255, 255, 0.45);
}

button.popover-menu-item:hover label.menu-row-label.menu-light-text,
button.popover-menu-item:hover label.menu-row-label.menu-dark-text {
    color: #ffffff;
}

"#;

fn main() {
    init_logging("malus_de_top_bar");
    info!("Iniciando Malus DE Top Bar (GTK4 / Layer Shell)...");

    // Esperar de forma activa a que el compositor esté listo y aceptando conexiones
    malus_de_common::wayland::wait_for_wayland(10);

    if let Err(err) = adw::init() {
        tracing::warn!("Aviso al inicializar libadwaita: {:?}", err);
    }

    let app = Application::builder()
        .application_id("org.malus_de.TopBar")
        .build();

    app.connect_startup(|_| {
        load_css();
    });

    app.connect_activate(build_ui);

    app.run();
}

fn load_css() {
    let provider = CssProvider::new();
    provider.load_from_string(STYLE_CSS);

    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER,
        );

        let icon_theme = gtk4::IconTheme::for_display(&display);
        icon_theme.add_search_path("/usr/local/share/malus-de/icons");
        icon_theme.add_search_path("/usr/share/malus-de/icons");
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                icon_theme.add_search_path(parent.join("assets/icons"));
                icon_theme.add_search_path(parent.join("../../assets/icons"));
            }
        }
    }
}

fn build_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Malus DE Top Bar")
        .default_height(36)
        .css_classes(["malus-topbar"])
        .build();

    window.set_size_request(-1, 36);
    window.set_default_size(-1, 36);

    window.set_decorated(false);

    if gtk4_layer_shell::is_supported() {
        info!("Protocolo Layer Shell soportado. Anclando Top Bar en capa Top...");
        window.init_layer_shell();
        window.set_layer(Layer::Top);
        window.set_namespace("malus-top-bar");
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Right, true);
        window.auto_exclusive_zone_enable();
        window.set_keyboard_mode(KeyboardMode::OnDemand);
    } else {
        info!("Layer Shell no detectado en este display; ejecutando ventana flotante.");
    }

    let root_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["topbar-content"])
        .valign(gtk4::Align::Center)
        .spacing(8)
        .build();

    // 1. Zona Izquierda: Apple Menu + Global Menu (con soporte com.canonical.AppMenu.Registrar)
    let left_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .valign(gtk4::Align::Center)
        .spacing(4)
        .build();

    let (dbus_menu_service, registrar) = crate::services::dbusmenu::DbusMenuService::new();
    crate::services::dbusmenu::DbusMenuService::start_registrar_server(registrar);
    let dbus_service_arc = std::sync::Arc::new(dbus_menu_service);

    let menu_handles = ui::global_menu::create_global_menu_widget(dbus_service_arc.clone());
    left_box.append(&ui::apple_menu::create_apple_menu());
    left_box.append(&menu_handles.container);

    // Escuchar eventos de registro de ventanas con menús D-Bus
    let handles_for_reg = menu_handles.clone();
    let reg_rx = dbus_service_arc.update_rx.clone();
    glib::spawn_future_local(async move {
        while let Ok(reg) = reg_rx.recv().await {
            let (app_id, title, window_id) = {
                let cur = handles_for_reg.current_active.borrow();
                (cur.0.clone(), cur.1.clone(), cur.2)
            };
            let should_update = match window_id {
                Some(wid) => (wid as u32) == reg.window_id,
                None => true,
            };
            if should_update {
                info!("🔄 [TopBar] Actualizando menú dinámico tras registro D-Bus de ventana {}", reg.window_id);
                ui::global_menu::update_global_menu(
                    &handles_for_reg,
                    app_id.as_deref(),
                    title.as_deref(),
                    window_id,
                );
            }
        }
    });

    // Escuchar eventos IPC del compositor para actualizar el nombre de la aplicación activa y acciones
    let (tx, rx) = async_channel::unbounded::<malus_de_common::ipc::IpcMessage>();
    let title_label = menu_handles.app_title_label.clone();
    let hide_btn = menu_handles.hide_btn.clone();
    let quit_btn = menu_handles.quit_btn.clone();
    let handles_clone = menu_handles.clone();

    glib::spawn_future_local(async move {
        while let Ok(msg) = rx.recv().await {
            match msg {
                malus_de_common::ipc::IpcMessage::ActiveAppChanged { app_id, title, window_id } => {
                    let display = format_app_name(app_id.as_deref(), title.as_deref());
                    title_label.set_label(&display);
                    hide_btn.set_label(&format!("Ocultar {}          ⌘H", display));
                    quit_btn.set_label(&format!("Salir de {}        ⌘Q", display));
                    ui::global_menu::update_global_menu(&handles_clone, app_id.as_deref(), title.as_deref(), window_id);
                }
                malus_de_common::ipc::IpcMessage::ThemeChanged { is_dark, .. } => {
                    ui::theme::update_all_popovers_theme(is_dark);
                }
                _ => {}
            }
        }
    });

    std::thread::spawn(move || {
        let sock_path = malus_de_common::ipc::ipc_socket_path();
        loop {
            if let Ok(stream) = std::os::unix::net::UnixStream::connect(&sock_path) {
                let reader = std::io::BufReader::new(stream);
                for line in std::io::BufRead::lines(reader) {
                    if let Ok(line) = line {
                        if let Ok(msg) = serde_json::from_str::<malus_de_common::ipc::IpcMessage>(&line) {
                            let _ = tx.send_blocking(msg);
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });

    // 2. Zona Central: Espaciador expansible
    let center_spacer = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .hexpand(true)
        .build();

    // 3. Zona Derecha: Auto-Tiling + Power Mode + Tray + Clock
    let right_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .valign(gtk4::Align::Center)
        .spacing(6)
        .build();
    right_box.append(&ui::auto_tiling::create_auto_tiling_widget());
    right_box.append(&ui::power_profile::create_power_profile_widget());
    right_box.append(&ui::status_tray::create_status_tray_widget());
    right_box.append(&ui::clock::create_clock_widget());

    root_box.append(&left_box);
    root_box.append(&center_spacer);
    root_box.append(&right_box);

    window.set_child(Some(&root_box));
    window.present();
}

fn format_app_name(app_id: Option<&str>, title: Option<&str>) -> String {
    if let Some(id) = app_id {
        let lower = id.to_lowercase();
        if lower.contains("firefox") || lower.contains("mozilla") {
            return "Firefox".to_string();
        } else if lower.contains("nautilus") || lower.contains("archivos") {
            return "Archivos".to_string();
        } else if lower.contains("calculator") || lower.contains("calculadora") {
            return "Calculadora".to_string();
        } else if lower.contains("ptyxis") || lower.contains("terminal") {
            return "Terminal".to_string();
        } else if lower.contains("texteditor") || lower.contains("text-editor") || lower.contains("gedit") {
            return "Editor de Texto".to_string();
        } else if lower.contains("control-center") || lower.contains("settings") {
            return "Configuración".to_string();
        }
    }

    if let Some(t) = title {
        let trimmed = t.trim();
        let lower = trimmed.to_lowercase();
        if lower.contains("firefox") {
            return "Firefox".to_string();
        }
        if !trimmed.is_empty() {
            if let Some((_, last)) = trimmed.rsplit_once(" — ") {
                return last.trim().to_string();
            }
            if let Some((_, last)) = trimmed.rsplit_once(" - ") {
                return last.trim().to_string();
            }
            return trimmed.to_string();
        }
    }

    if let Some(id) = app_id {
        if let Some(last) = id.rsplit('.').next() {
            if !last.is_empty() {
                return last.to_string();
            }
        }
    }

    "Malus DE".to_string()
}
