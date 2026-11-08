//! Biblioteca compartida para Malus DE (`malus-de-common`)
//!
//! Contiene estructuras de configuración serializables, tipos de mensajes IPC
//! para sockets Unix, gestión de temas y configuración de logging estructurado.

pub mod config;
pub mod ipc;
pub mod theme;
pub mod logging;
pub mod wayland;

/// Detecta si el locale activo del sistema corresponde a idioma español (`es_*`).
pub fn is_spanish_locale() -> bool {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default()
        .to_lowercase()
        .starts_with("es")
}

