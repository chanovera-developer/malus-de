//! ==============================================================================
//! Malus DE: Pipeline de Renderizado GPU Nativo y Sistema de Layout Reactivo
//! ==============================================================================

pub mod dbus_menu;
pub mod layout;
pub mod luminance;
pub mod popover;
pub mod renderer;
pub mod squircle;
pub mod status_notifier;
pub mod text;
pub mod top_bar;
pub mod wayland;

pub use dbus_menu::{DbusMenuCategory, DbusMenuItem, DbusMenuService, RegisteredMenu};
pub use layout::{squircle_sdf_cpu, SquircleStyle, UiNodeData, UiTree};
pub use luminance::{calculate_luminance_rgba, detect_system_is_dark, AdaptivePalette};
pub use popover::{
    build_control_center_panel, build_popover_menu, ControlCenterModel, PopoverConfig, PopoverItem,
    PopoverMenuModel, PopoverNodes,
};
pub use renderer::SquircleRenderer;
pub use squircle::{
    SquircleInstance, SquirclePipeline, SquircleUniforms, MACOS_SQUIRCLE_POWER,
};
pub use status_notifier::{SniService, SniUpdate, StatusNotifierWatcher};
pub use text::{FontSystem, TextBounds, TextLabel, TextSystem, Weight};
pub use top_bar::{build_top_bar, StatusItem, TopBarConfig, TopBarModel, TopBarNodes};
pub use wayland::WaylandTopBarApp;
