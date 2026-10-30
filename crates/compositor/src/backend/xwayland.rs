use anyhow::{Context, Result};
use smithay::{
    desktop::Window,
    reexports::{calloop::LoopHandle, wayland_server::DisplayHandle},
    utils::{Logical, Point, Rectangle},
    xwayland::{X11Surface, X11Wm, XWayland, XWaylandEvent},
};
use std::process::Stdio;
use tracing::{info, warn};

use crate::state::MalusState;

pub fn start(
    loop_handle: LoopHandle<'static, MalusState>,
    display_handle: &DisplayHandle,
) -> Result<()> {
    let (xwayland, client) = XWayland::spawn(
        display_handle,
        None,
        std::iter::empty::<(String, String)>(),
        true,
        Stdio::null(),
        Stdio::null(),
        |_| {},
    )
    .context("No se pudo iniciar XWayland; las aplicaciones X11 no estarán disponibles")?;

    let event_loop_handle = loop_handle.clone();
    loop_handle
        .insert_source(xwayland, move |event, _, state| match event {
            XWaylandEvent::Ready {
                x11_socket,
                display_number,
            } => {
                let display_name = format!(":{display_number}");
                std::env::set_var("DISPLAY", &display_name);
                info!("XWayland está listo en DISPLAY={display_name}");

                match X11Wm::start_wm(event_loop_handle.clone(), x11_socket, client.clone()) {
                    Ok(wm) => {
                        state.xwayland_wms.insert(wm.id(), wm);
                        import_display_environment(&display_name);
                    }
                    Err(error) => {
                        tracing::error!("No se pudo iniciar el gestor de ventanas X11: {error}");
                    }
                }
            }
            XWaylandEvent::Error => {
                tracing::error!("XWayland terminó con un error durante el inicio");
            }
        })
        .context("No se pudo registrar XWayland en el bucle de eventos")?;

    info!("Servidor XWayland iniciado");
    Ok(())
}

fn import_display_environment(display: &str) {
    let result = std::process::Command::new("dbus-update-activation-environment")
        .args(["--systemd", "DISPLAY"])
        .env("DISPLAY", display)
        .status();
    match result {
        Ok(status) if status.success() => {}
        Ok(status) => {
            warn!("No se pudo propagar DISPLAY al entorno D-Bus/systemd (salida {status})")
        }
        Err(error) => warn!("No se pudo importar DISPLAY al entorno D-Bus/systemd: {error}"),
    }
}

pub fn map_window(state: &mut MalusState, surface: X11Surface) {
    let Some(wl_surface) = surface.wl_surface() else {
        return;
    };
    if let Some(window_id) = state
        .minimized_windows
        .iter()
        .find(|(_, (window, _))| window.x11_surface() == Some(&surface))
        .and_then(|(surface, _)| state.window_ids.get(surface).copied())
    {
        state.restore_window_by_id(window_id);
        return;
    }
    if state
        .space
        .elements()
        .any(|window| window.x11_surface() == Some(&surface))
    {
        return;
    }

    let output = state.space.outputs().next().cloned();
    let (location, output_geometry) = output
        .as_ref()
        .and_then(|output| {
            let geometry = state.space.output_geometry(output)?;
            let offset = (state.space.elements().count() as i32 % 8) * 28;
            Some((
                Point::from((geometry.loc.x + 80 + offset, geometry.loc.y + 60 + offset)),
                geometry,
            ))
        })
        .unwrap_or_else(|| {
            (
                Point::from((80, 60)),
                Rectangle::new((0, 0).into(), (1920, 1080).into()),
            )
        });

    if !surface.is_override_redirect() {
        let current = surface.geometry();
        let max_width = (output_geometry.size.w - 80).max(320);
        let max_height = (output_geometry.size.h - 80).max(240);
        let geometry = Rectangle::new(
            location,
            (
                current.size.w.clamp(320, max_width),
                current.size.h.clamp(240, max_height),
            )
                .into(),
        );
        if let Err(error) = surface.configure(Some(geometry)) {
            warn!("No se pudo ubicar la ventana X11: {error}");
        }
    }

    if let Some(output) = output {
        output.enter(&wl_surface);
    }
    let window = Window::new_x11_window(surface.clone());
    window.set_activated(true);
    state.space.map_element(window, location, true);
    state.get_or_assign_window_id(&wl_surface);
    if let Some(keyboard) = state.seat.get_keyboard() {
        keyboard.set_focus(
            state,
            Some(crate::state::KeyboardFocusTarget::X11(surface.clone())),
            smithay::utils::SERIAL_COUNTER.next_serial(),
        );
    }
    state.notify_running_apps();
    state.notify_active_app(Some(&wl_surface));
    info!("Ventana X11 mapeada: {}", surface.title());
}

pub fn unmap_window(state: &mut MalusState, surface: &X11Surface) {
    let window = state
        .space
        .elements()
        .find(|window| window.x11_surface() == Some(surface))
        .cloned()
        .or_else(|| {
            state
                .minimized_windows
                .values()
                .find(|(window, _)| window.x11_surface() == Some(surface))
                .map(|(window, _)| window.clone())
        });

    let wl_surface = surface.wl_surface();
    if let Some(window) = window {
        state.space.unmap_elem(&window);
    }
    if let Some(wl_surface) = wl_surface {
        state.minimized_windows.remove(&wl_surface);
        state.window_restore_geometries.remove(&wl_surface);
        state.x11_window_info.remove(&wl_surface);
        if let Some(window_id) = state.window_ids.remove(&wl_surface) {
            state.dock_slots.remove(&window_id);
            state.remove_minimized_thumbnail(window_id);
            state
                .active_animations
                .retain(|animation| animation.surface != wl_surface);
        }
    }
    state.space.refresh();
    state.notify_running_apps();
    info!("Ventana X11 desmapeada: {}", surface.title());
}

pub fn update_window_location(
    state: &mut MalusState,
    surface: &X11Surface,
    location: Point<i32, Logical>,
) {
    let Some(window) = state
        .space
        .elements()
        .find(|window| window.x11_surface() == Some(surface))
        .cloned()
    else {
        return;
    };
    state.space.map_element(window, location, true);
}

pub fn set_window_state(state: &mut MalusState, surface: &X11Surface, fullscreen: bool) {
    let Some(wl_surface) = surface.wl_surface() else {
        return;
    };
    let window = state.window_for_surface(&wl_surface);
    if let Some(window) = &window {
        if !surface.is_maximized() && !surface.is_fullscreen() {
            let location = state
                .space
                .element_location(window)
                .unwrap_or_else(|| surface.geometry().loc);
            state
                .window_restore_geometries
                .entry(wl_surface.clone())
                .or_insert((location, surface.geometry().size));
        }
    }

    let Some(output) = window
        .as_ref()
        .and_then(|window| state.space.outputs_for_element(window).into_iter().next())
        .or_else(|| state.space.outputs().next().cloned())
    else {
        return;
    };
    let geometry = if fullscreen {
        state.space.output_geometry(&output)
    } else {
        Some(state.usable_zone_for_output(&output))
    };
    let Some(geometry) = geometry else {
        return;
    };

    if let Err(error) = surface.configure(Some(geometry)) {
        warn!("No se pudo actualizar el estado de la ventana X11: {error}");
    } else {
        update_window_location(state, surface, geometry.loc);
    }
    if let Err(error) = surface.set_maximized(!fullscreen) {
        warn!("No se pudo cambiar el estado maximizado de la ventana X11: {error}");
    }
    if let Err(error) = surface.set_fullscreen(fullscreen) {
        warn!("No se pudo cambiar el estado de pantalla completa de la ventana X11: {error}");
    }
}

pub fn restore_window_state(state: &mut MalusState, surface: &X11Surface) {
    let Some(wl_surface) = surface.wl_surface() else {
        return;
    };
    if let Some((location, size)) = state.window_restore_geometries.remove(&wl_surface) {
        let geometry = Rectangle::new(location, size);
        if let Err(error) = surface.configure(Some(geometry)) {
            warn!("No se pudo restaurar la geometría de la ventana X11: {error}");
        } else {
            update_window_location(state, surface, geometry.loc);
        }
    }
    if let Err(error) = surface.set_maximized(false) {
        warn!("No se pudo restaurar la ventana X11: {error}");
    }
    if let Err(error) = surface.set_fullscreen(false) {
        warn!("No se pudo salir de pantalla completa en la ventana X11: {error}");
    }
}
