use gtk4::prelude::*;
use gtk4::{Button, Image};
use malus_de_common::ipc::{IpcClient, IpcMessage};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use tracing::info;

thread_local! {
    static TILING_BTN_REF: RefCell<Option<(glib::WeakRef<Button>, Rc<Cell<bool>>)>> =
        const { RefCell::new(None) };
}

pub fn refresh_auto_tiling_visibility() {
    let visible = malus_de_common::config::MenuBarConfig::load().window_tiling;
    TILING_BTN_REF.with(|cell| {
        if let Some((weak, _)) = cell.borrow().as_ref() {
            if let Some(btn) = weak.upgrade() {
                btn.set_visible(visible);
            }
        }
    });
}

pub fn set_auto_tiling_visual(enabled: bool) {
    TILING_BTN_REF.with(|cell| {
        if let Some((weak, state)) = cell.borrow().as_ref() {
            if let Some(btn) = weak.upgrade() {
                apply_tiling_visual(&btn, state, enabled);
            }
        }
    });
}

fn apply_tiling_visual(btn: &Button, state: &Cell<bool>, enabled: bool) {
    state.set(enabled);
    if enabled {
        btn.set_child(Some(&Image::from_icon_name("view-grid-symbolic")));
        btn.add_css_class("active-pill");
        btn.set_tooltip_text(Some("Ordenamiento de ventanas: Mosaico activo (Super+Y)"));
    } else {
        btn.set_child(Some(&Image::from_icon_name("view-paged-symbolic")));
        btn.remove_css_class("active-pill");
        btn.set_tooltip_text(Some("Ordenamiento de ventanas: Flotante (Clic o Super+Y)"));
    }
}

pub fn create_auto_tiling_widget() -> Button {
    let is_tiling = Rc::new(Cell::new(false));

    let btn = Button::builder()
        .css_classes(["tiling-pill", "topbar-btn"])
        .tooltip_text("Ordenamiento de ventanas: Flotante (Clic o Super+Y)")
        .has_frame(false)
        .build();
    btn.set_visible(malus_de_common::config::MenuBarConfig::load().window_tiling);

    let icon = Image::from_icon_name("view-paged-symbolic");
    btn.set_child(Some(&icon));

    TILING_BTN_REF.with(|cell| {
        *cell.borrow_mut() = Some((btn.downgrade(), is_tiling.clone()));
    });

    let btn_clone = btn.clone();
    let state_clone = is_tiling;

    btn.connect_clicked(move |_| {
        let next = !state_clone.get();
        apply_tiling_visual(&btn_clone, &state_clone, next);
        info!("Solicitando ToggleAutoTiling al compositor (activo={next})");
        if let Err(e) = IpcClient::send_command(IpcMessage::ToggleAutoTiling) {
            tracing::warn!("Error al enviar ToggleAutoTiling al compositor: {:?}", e);
        }
    });

    btn
}
