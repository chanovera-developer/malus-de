//! Sincronización compartida del tamaño de íconos y filas de barra lateral (`Pequeños`, `Medianos`, `Grandes`).
//!
//! Tanto Ajustes como Branches y cualquier aplicación futura de Malus DE que use las clases
//! `.malus-sidebar-row`, `.malus-sidebar-icon` y `.malus-sidebar-label` (o registre sus
//! widgets con [`register_sidebar_icon`] / [`register_sidebar_row`]) obtienen escalado
//! consistente y actualización en vivo al cambiar «Íconos de la barra lateral» en Aspecto.

use crate::config::{
    appearance_settings_path, load_sidebar_icon_size_index, sidebar_icon_metrics,
    sidebar_icon_size_css,
};
use gtk4::gdk::Display;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{CssProvider, Image, Widget};
use std::cell::{Cell, RefCell};

thread_local! {
    static SIDEBAR_CSS_PROVIDER: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
    static SIDEBAR_MONITOR: RefCell<Option<gio::FileMonitor>> = const { RefCell::new(None) };
    static LAST_SIDEBAR_IDX: Cell<usize> = const { Cell::new(usize::MAX) };
    static SIDEBAR_ICONS: RefCell<Vec<glib::WeakRef<Image>>> = const { RefCell::new(Vec::new()) };
    static SIDEBAR_ROWS: RefCell<Vec<glib::WeakRef<Widget>>> = const { RefCell::new(Vec::new()) };
}

/// Instala el proveedor CSS y el monitor sobre `appearance-settings.toml` para esta ventana/app.
pub fn install_sidebar_size_sync(display: &Display) {
    let already_installed = SIDEBAR_CSS_PROVIDER.with(|c| c.borrow().is_some());
    if !already_installed {
        let provider = CssProvider::new();
        gtk4::style_context_add_provider_for_display(
            display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_USER + 3,
        );
        SIDEBAR_CSS_PROVIDER.with(|c| {
            *c.borrow_mut() = Some(provider);
        });

        let cfg_file = gio::File::for_path(appearance_settings_path());
        if let Ok(monitor) =
            cfg_file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            monitor.connect_changed(|_, _, _, _| {
                let idx = load_sidebar_icon_size_index();
                if LAST_SIDEBAR_IDX.with(|c| c.get()) != idx {
                    apply_sidebar_size_index(idx);
                }
            });
            SIDEBAR_MONITOR.with(|c| {
                *c.borrow_mut() = Some(monitor);
            });
        }
    }
    apply_sidebar_size_now();
}

/// Registra un `gtk4::Image` de barra lateral para que escale automáticamente (`14` / `16` / `20` px).
pub fn register_sidebar_icon(img: &Image) {
    img.add_css_class("malus-sidebar-icon");
    let metrics = sidebar_icon_metrics(load_sidebar_icon_size_index());
    img.set_pixel_size(metrics.icon_px);
    SIDEBAR_ICONS.with(|cell| {
        let mut list = cell.borrow_mut();
        list.retain(|w| w.upgrade().is_some());
        list.push(img.downgrade());
    });
}

/// Registra una fila o botón de barra lateral para mantener su altura acorde al tamaño activo.
pub fn register_sidebar_row(widget: &impl IsA<Widget>) {
    let w = widget.as_ref();
    w.add_css_class("malus-sidebar-row");
    let metrics = sidebar_icon_metrics(load_sidebar_icon_size_index());
    w.set_height_request(metrics.row_height);
    SIDEBAR_ROWS.with(|cell| {
        let mut list = cell.borrow_mut();
        list.retain(|item| item.upgrade().is_some());
        list.push(w.downgrade());
    });
}

/// Relee `sidebar_icon_size_idx` desde disco y aplica CSS e íconos registrados de inmediato.
pub fn apply_sidebar_size_now() {
    apply_sidebar_size_index(load_sidebar_icon_size_index());
}

/// Aplica un índice explícito (`0 = Pequeños`, `1 = Medianos`, `2 = Grandes`) al instante.
pub fn apply_sidebar_size_index(index: usize) {
    let idx = index.min(2);
    LAST_SIDEBAR_IDX.with(|c| c.set(idx));
    let metrics = sidebar_icon_metrics(idx);
    let css = sidebar_icon_size_css(idx);

    SIDEBAR_CSS_PROVIDER.with(|c| {
        if let Some(provider) = c.borrow().as_ref() {
            provider.load_from_string(&css);
        }
    });

    SIDEBAR_ICONS.with(|cell| {
        let mut list = cell.borrow_mut();
        list.retain(|weak| {
            if let Some(img) = weak.upgrade() {
                img.set_pixel_size(metrics.icon_px);
                true
            } else {
                false
            }
        });
    });

    SIDEBAR_ROWS.with(|cell| {
        let mut list = cell.borrow_mut();
        list.retain(|weak| {
            if let Some(row) = weak.upgrade() {
                row.set_height_request(metrics.row_height);
                true
            } else {
                false
            }
        });
    });
}

