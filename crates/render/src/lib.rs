//! ==============================================================================
//! Malus DE: Pipeline de Renderizado GPU Nativo y Sistema de Layout Reactivo
//! ==============================================================================

pub mod dbus_menu;
pub mod layout;
pub mod renderer;
pub mod squircle;
pub mod text;
pub mod top_bar;
pub mod wayland;

pub use dbus_menu::{DbusMenuCategory, DbusMenuItem, DbusMenuService, RegisteredMenu};
pub use layout::{squircle_sdf_cpu, SquircleStyle, UiNodeData, UiTree};
pub use renderer::SquircleRenderer;
pub use squircle::{
    SquircleInstance, SquirclePipeline, SquircleUniforms, MACOS_SQUIRCLE_POWER,
};
pub use text::{FontSystem, TextBounds, TextLabel, TextSystem, Weight};
pub use top_bar::{build_top_bar, TopBarConfig, TopBarModel, TopBarNodes};
pub use wayland::WaylandTopBarApp;
