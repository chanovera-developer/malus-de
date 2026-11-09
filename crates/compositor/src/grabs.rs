use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
        GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
        GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
        GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::{ResizeEdge, State as XdgState},
    utils::{Logical, Point, Rectangle, Size},
    wayland::seat::WaylandFocus,
};

use crate::state::MalusState;

pub struct ResizeSurfaceGrab {
    pub start_data: GrabStartData<MalusState>,
    pub window: Window,
    pub edges: ResizeEdge,
    pub initial_window_location: Point<i32, Logical>,
    pub initial_window_size: Size<i32, Logical>,
    pub last_window_size: Size<i32, Logical>,
}

impl PointerGrab<MalusState> for ResizeSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        _focus: Option<(
            <MalusState as smithay::input::SeatHandler>::PointerFocus,
            Point<f64, Logical>,
        )>,
        event: &MotionEvent,
    ) {
        if handle.current_pressed().is_empty() {
            tracing::info!("🛑 ResizeSurfaceGrab: ratón liberado durante motion, restaurando puntero");
            handle.unset_grab(self, data, event.serial, event.time, false);
            let cur_loc = event.location;
            let under_target = data.space.element_under(cur_loc).and_then(|(win, loc)| {
                let local_point = cur_loc - loc.to_f64();
                win.surface_under(local_point, smithay::desktop::WindowSurfaceType::all())
                    .map(|(surf, sub_loc)| (surf, (loc + sub_loc).to_f64()))
            });
            handle.motion(data, under_target, event);
            handle.frame(data);
            data.update_ssd_hover(cur_loc);
            return;
        }

        handle.motion(data, None, event);

        if let Some(surf) = self.window.wl_surface() {
            data.snap_transitions.remove(surf.as_ref());
        }

        let delta = event.location - self.start_data.location;
        let dx = delta.x as i32;
        let dy = delta.y as i32;

        let (min_w, min_h) = self
            .window
            .wl_surface()
            .map(|wl_surf| {
                smithay::wayland::compositor::with_states(&wl_surf, |states| {
                    let mut guard = states
                        .cached_state
                        .get::<smithay::wayland::shell::xdg::SurfaceCachedState>();
                    let cur = guard.current();
                    (cur.min_size.w.max(180), cur.min_size.h.max(120))
                })
            })
            .unwrap_or((180, 120));

        let mut new_width = self.initial_window_size.w;
        let mut new_height = self.initial_window_size.h;
        let mut new_x = self.initial_window_location.x;
        let mut new_y = self.initial_window_location.y;

        // Determinar el monitor de la ventana y el límite superior libre bajo la Top Bar
        let output = data
            .origin_output_for_window(&self.window)
            .or_else(|| {
                data.space
                    .outputs()
                    .find(|o| {
                        data.space
                            .output_geometry(o)
                            .map(|g| g.to_f64().contains(event.location))
                            .unwrap_or(false)
                    })
                    .cloned()
            })
            .or_else(|| data.space.outputs().next().cloned());

        let min_top = if let Some(ref o) = output {
            let usable = data.usable_zone_for_output(o);
            let ssd = if crate::state::window_uses_ssd(&self.window) {
                crate::ssd::SSD_TITLEBAR_HEIGHT
            } else {
                0
            };
            usable.loc.y + ssd
        } else {
            let ssd = if crate::state::window_uses_ssd(&self.window) {
                crate::ssd::SSD_TITLEBAR_HEIGHT
            } else {
                0
            };
            36 + ssd
        };

        match self.edges {
            ResizeEdge::Left => {
                new_width = (self.initial_window_size.w - dx).max(min_w);
                new_x = self.initial_window_location.x + (self.initial_window_size.w - new_width);
            }
            ResizeEdge::Right => {
                new_width = (self.initial_window_size.w + dx).max(min_w);
            }
            ResizeEdge::Top => {
                let initial_bottom = self.initial_window_location.y + self.initial_window_size.h;
                new_height = (self.initial_window_size.h - dy).max(min_h);
                new_y = self.initial_window_location.y + (self.initial_window_size.h - new_height);
                if new_y < min_top {
                    new_y = min_top;
                    new_height = (initial_bottom - min_top).max(min_h);
                }
            }
            ResizeEdge::Bottom => {
                new_height = (self.initial_window_size.h + dy).max(min_h);
            }
            ResizeEdge::TopLeft => {
                new_width = (self.initial_window_size.w - dx).max(min_w);
                new_x = self.initial_window_location.x + (self.initial_window_size.w - new_width);
                let initial_bottom = self.initial_window_location.y + self.initial_window_size.h;
                new_height = (self.initial_window_size.h - dy).max(min_h);
                new_y = self.initial_window_location.y + (self.initial_window_size.h - new_height);
                if new_y < min_top {
                    new_y = min_top;
                    new_height = (initial_bottom - min_top).max(min_h);
                }
            }
            ResizeEdge::TopRight => {
                new_width = (self.initial_window_size.w + dx).max(min_w);
                let initial_bottom = self.initial_window_location.y + self.initial_window_size.h;
                new_height = (self.initial_window_size.h - dy).max(min_h);
                new_y = self.initial_window_location.y + (self.initial_window_size.h - new_height);
                if new_y < min_top {
                    new_y = min_top;
                    new_height = (initial_bottom - min_top).max(min_h);
                }
            }
            ResizeEdge::BottomLeft => {
                new_width = (self.initial_window_size.w - dx).max(min_w);
                new_x = self.initial_window_location.x + (self.initial_window_size.w - new_width);
                new_height = (self.initial_window_size.h + dy).max(min_h);
            }
            ResizeEdge::BottomRight => {
                new_width = (self.initial_window_size.w + dx).max(min_w);
                new_height = (self.initial_window_size.h + dy).max(min_h);
            }
            _ => {}
        }

        // Jamás permitir que la coordenada Y de la ventana penetre bajo la Top Bar
        new_y = new_y.max(min_top);

        let new_size = Size::from((new_width, new_height));
        let new_loc = Point::from((new_x, new_y));

        data.cursor_status = smithay::input::pointer::CursorImageStatus::Named(
            cursor_icon_for_resize_edge(self.edges),
        );

        if new_size != self.last_window_size {
            self.last_window_size = new_size;
            if let Some(toplevel) = self.window.toplevel() {
                toplevel.with_pending_state(|top_state| {
                    top_state.size = Some(new_size);
                    top_state.states.unset(XdgState::Maximized);
                    top_state.states.unset(XdgState::TiledLeft);
                    top_state.states.unset(XdgState::TiledRight);
                    top_state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Resizing);
                });
                toplevel.send_configure();
            }
        }

        if data.space.element_location(&self.window) != Some(new_loc) {
            data.space.map_element(self.window.clone(), new_loc, true);
        }
    }

    fn relative_motion(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        focus: Option<(
            <MalusState as smithay::input::SeatHandler>::PointerFocus,
            Point<f64, Logical>,
        )>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            if let Some(toplevel) = self.window.toplevel() {
                toplevel.with_pending_state(|top_state| {
                    top_state.states.unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Resizing);
                });
                toplevel.send_configure();
            }

            handle.unset_grab(self, data, event.serial, event.time, false);

            let cur_loc = handle.current_location();
            let under_target = data.space.element_under(cur_loc).and_then(|(win, loc)| {
                let local_point = cur_loc - loc.to_f64();
                win.surface_under(local_point, smithay::desktop::WindowSurfaceType::all())
                    .map(|(surf, sub_loc)| (surf, (loc + sub_loc).to_f64()))
            });
            let motion_serial = smithay::utils::SERIAL_COUNTER.next_serial();
            handle.motion(
                data,
                under_target,
                &MotionEvent {
                    location: cur_loc,
                    serial: motion_serial,
                    time: event.time,
                },
            );
            handle.frame(data);
            data.update_ssd_hover(cur_loc);
            tracing::info!("✅ ResizeSurfaceGrab finalizado y foco restaurado");
        }
    }

    fn axis(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
    ) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &GrabStartData<MalusState> {
        &self.start_data
    }

    fn unset(&mut self, data: &mut MalusState) {
        data.ssd.grab_active = false;
        data.ssd.cursor_override = false;
    }
}

pub struct MoveSurfaceGrab {
    pub start_data: GrabStartData<MalusState>,
    pub window: Window,
    pub initial_window_location: Point<i32, Logical>,
    pub is_snapped: bool,
    pub has_dragged: bool,
}

impl PointerGrab<MalusState> for MoveSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        _focus: Option<(
            <MalusState as smithay::input::SeatHandler>::PointerFocus,
            Point<f64, Logical>,
        )>,
        event: &MotionEvent,
    ) {
        if handle.current_pressed().is_empty() {
            tracing::info!("🛑 MoveSurfaceGrab: ratón liberado durante motion, restaurando puntero");
            data.snap_preview = None;
            data.moving_window_surface = None;
            handle.unset_grab(self, data, event.serial, event.time, false);
            let cur_loc = event.location;
            let under_target = data.space.element_under(cur_loc).and_then(|(win, loc)| {
                let local_point = cur_loc - loc.to_f64();
                win.surface_under(local_point, smithay::desktop::WindowSurfaceType::all())
                    .map(|(surf, sub_loc)| (surf, (loc + sub_loc).to_f64()))
            });
            handle.motion(data, under_target, event);
            handle.frame(data);
            data.update_ssd_hover(cur_loc);
            return;
        }

        // Mientras el grab está activo, ningún cliente tiene foco del puntero
        handle.motion(data, None, event);

        if let Some(surf) = self.window.wl_surface() {
            data.snap_transitions.remove(surf.as_ref());
        }

        let delta = event.location - self.start_data.location;
        let dist_sq = delta.x * delta.x + delta.y * delta.y;

        if !self.has_dragged {
            let threshold_sq = if self.is_snapped { 64.0 } else { 16.0 };
            if dist_sq < threshold_sq {
                return;
            }
            self.has_dragged = true;

            if let Some(surf) = self.window.wl_surface() {
                data.snap_transitions.remove(surf.as_ref());
            }

            if self.is_snapped {
                let surface_ref = self.window.wl_surface();
                let saved = surface_ref.as_ref().and_then(|surf| {
                    data.window_restore_geometries.get(surf.as_ref()).copied()
                });
                let restore_size = saved.map(|(_, s)| s).filter(|s| s.w > 100 && s.h > 100);

                if let Some(toplevel) = self.window.toplevel() {
                    toplevel.with_pending_state(|top_state| {
                        top_state.size = restore_size;
                        top_state.states.unset(XdgState::Maximized);
                        top_state.states.unset(XdgState::TiledLeft);
                        top_state.states.unset(XdgState::TiledRight);
                    });
                    toplevel.send_configure();
                }

                let float_w = restore_size.map(|s| s.w).unwrap_or(400);
                let cursor_x = event.location.x as i32;
                let cursor_y = event.location.y as i32;
                let new_x = (cursor_x - float_w / 2).max(0);
                let new_y = if crate::state::window_uses_ssd(&self.window) {
                    (cursor_y + crate::ssd::SSD_TITLEBAR_HEIGHT / 2).max(32 + crate::ssd::SSD_TITLEBAR_HEIGHT)
                } else {
                    (cursor_y - 15).max(32)
                };
                let new_loc = Point::from((new_x, new_y));
                self.initial_window_location = new_loc;
                self.start_data.location = event.location;
                data.space.map_element(self.window.clone(), new_loc, true);
                self.is_snapped = false;
            }
        }

        let delta = event.location - self.start_data.location;
        let mut new_location = self.initial_window_location.to_f64() + delta;

        // Determinar el monitor actual y limitar el arrastre para que nunca quede bajo la Top Bar
        let output = data
            .origin_output_for_window(&self.window)
            .or_else(|| {
                data.space
                    .outputs()
                    .find(|o| {
                        data.space
                            .output_geometry(o)
                            .map(|g| g.to_f64().contains(event.location))
                            .unwrap_or(false)
                    })
                    .cloned()
            })
            .or_else(|| data.space.outputs().next().cloned());

        let min_top = if let Some(ref o) = output {
            let usable = data.usable_zone_for_output(o);
            let ssd = if crate::state::window_uses_ssd(&self.window) {
                crate::ssd::SSD_TITLEBAR_HEIGHT
            } else {
                0
            };
            usable.loc.y + ssd
        } else {
            let ssd = if crate::state::window_uses_ssd(&self.window) {
                crate::ssd::SSD_TITLEBAR_HEIGHT
            } else {
                0
            };
            36 + ssd
        };

        if new_location.y < min_top as f64 {
            new_location.y = min_top as f64;
        }

        data.space
            .map_element(self.window.clone(), new_location.to_i32_round(), true);

        let cur_loc = event.location;
        let active_output = data
            .space
            .outputs()
            .find(|o| {
                data.space
                    .output_geometry(o)
                    .map(|g| g.to_f64().contains(cur_loc))
                    .unwrap_or(false)
            })
            .or_else(|| data.space.outputs().next())
            .cloned();

        if let Some(ref output) = active_output {
            if let Some(wl_surf) = self.window.wl_surface() {
                let scale = output.current_scale().fractional_scale();
                smithay::wayland::compositor::with_states(&wl_surf, |states| {
                    smithay::wayland::fractional_scale::with_fractional_scale(states, |fs| {
                        fs.set_preferred_scale(scale);
                    });
                });
            }
        }

        // Actualizar overlay de previsualización Aero Snap en tiempo real
        let preview = active_output.and_then(|output| {
            let output_geo = data.space.output_geometry(&output).unwrap_or_default();
            let usable_zone = data.usable_zone_for_output(&output);

            evaluate_snap(cur_loc, output_geo, usable_zone)
                .and_then(|mode| preview_for_snap(mode, usable_zone))
        });

        data.snap_preview = preview;
    }

    fn relative_motion(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        focus: Option<(
            <MalusState as smithay::input::SeatHandler>::PointerFocus,
            Point<f64, Logical>,
        )>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            // Limpiar previsualización inmediatamente al soltar
            data.snap_preview = None;
            data.moving_window_surface = None;

            let cur_loc = handle.current_location();

            if !self.has_dragged {
                tracing::info!("👆 MoveSurfaceGrab finalizado sin arrastre (clic simple en título/menú)");
                handle.unset_grab(self, data, event.serial, event.time, false);

                let under_target = data.space.element_under(cur_loc).and_then(|(win, loc)| {
                    let local_point = cur_loc - loc.to_f64();
                    win.surface_under(local_point, smithay::desktop::WindowSurfaceType::all())
                        .map(|(surf, sub_loc)| (surf, (loc + sub_loc).to_f64()))
                });
                let motion_serial = smithay::utils::SERIAL_COUNTER.next_serial();
                handle.motion(
                    data,
                    under_target,
                    &MotionEvent {
                        location: cur_loc,
                        serial: motion_serial,
                        time: event.time,
                    },
                );
                handle.frame(data);
                data.update_ssd_hover(cur_loc);
                return;
            }

            let win_loc = data.space.element_location(&self.window).unwrap_or_default();
            let win_geo = self.window.geometry();
            let outputs_count = data.space.outputs().count();

            tracing::info!(
                "🛑 Soltado botón del ratón: cur_loc={:?}, win_loc={:?}, win_size={:?}, outputs_count={}",
                cur_loc, win_loc, win_geo.size, outputs_count
            );

            // Detección de Aero Snap al soltar la ventana en los bordes de la pantalla bajo el cursor
            let target_output = data
                .space
                .outputs()
                .find(|o| {
                    data.space
                        .output_geometry(o)
                        .map(|g| g.to_f64().contains(cur_loc))
                        .unwrap_or(false)
                })
                .or_else(|| data.space.outputs().next())
                .cloned();

            let snap_action = target_output.and_then(|output| {
                let output_geo = data.space.output_geometry(&output).unwrap_or_default();
                let usable_zone = data.usable_zone_for_output(&output);

                let mode = evaluate_snap(cur_loc, output_geo, usable_zone);
                tracing::info!(
                    "🔍 Aero Snap evaluación al soltar: modo={:?}, cur_loc={:?}, output_geo={:?}, usable_zone={:?}",
                    mode, cur_loc, output_geo, usable_zone
                );

                mode.map(|m| (m, usable_zone))
            });

            if let Some((mode, zone)) = snap_action {
                crate::state::snap_window(data, &self.window, mode, zone);
            } else if let Some(surf) = self.window.wl_surface() {
                if !self.is_snapped && win_geo.size.w > 150 && win_geo.size.h > 150 {
                    data.window_restore_geometries.insert(surf.into_owned(), (win_loc, win_geo.size));
                }
            }

            handle.unset_grab(self, data, event.serial, event.time, false);

            // Re-sincronizar foco del puntero inmediatamente sobre la superficie bajo el cursor
            let under_target = data.space.element_under(cur_loc).and_then(|(win, loc)| {
                let local_point = cur_loc - loc.to_f64();
                win.surface_under(local_point, smithay::desktop::WindowSurfaceType::all())
                    .map(|(surf, sub_loc)| (surf, (loc + sub_loc).to_f64()))
            });
            let motion_serial = smithay::utils::SERIAL_COUNTER.next_serial();
            handle.motion(
                data,
                under_target,
                &MotionEvent {
                    location: cur_loc,
                    serial: motion_serial,
                    time: event.time,
                },
            );
            handle.frame(data);
            data.update_ssd_hover(cur_loc);
        }
    }

    fn axis(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
    ) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut MalusState,
        handle: &mut PointerInnerHandle<'_, MalusState>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &GrabStartData<MalusState> {
        &self.start_data
    }

    fn unset(&mut self, data: &mut MalusState) {
        data.ssd.grab_active = false;
    }
}

pub fn evaluate_snap(
    cur_loc: Point<f64, Logical>,
    output_geo: Rectangle<i32, Logical>,
    usable_zone: Rectangle<i32, Logical>,
) -> Option<crate::state::SnapMode> {
    let edge_margin = 35.0;

    let is_left = cur_loc.x <= output_geo.loc.x as f64 + edge_margin;
    let is_right = cur_loc.x >= (output_geo.loc.x + output_geo.size.w) as f64 - edge_margin;
    let is_top = cur_loc.y <= usable_zone.loc.y as f64 + edge_margin;

    if is_left {
        Some(crate::state::SnapMode::LeftHalf)
    } else if is_right {
        Some(crate::state::SnapMode::RightHalf)
    } else if is_top {
        Some(crate::state::SnapMode::Maximize)
    } else {
        None
    }
}

pub fn preview_for_snap(
    mode: crate::state::SnapMode,
    usable_zone: Rectangle<i32, Logical>,
) -> Option<Rectangle<i32, Logical>> {
    match mode {
        crate::state::SnapMode::Maximize => Some(Rectangle::new(
            (usable_zone.loc.x + 8, usable_zone.loc.y + 8).into(),
            (usable_zone.size.w - 16, usable_zone.size.h - 16).into(),
        )),
        crate::state::SnapMode::LeftHalf => Some(Rectangle::new(
            (usable_zone.loc.x + 8, usable_zone.loc.y + 8).into(),
            (usable_zone.size.w / 2 - 12, usable_zone.size.h - 16).into(),
        )),
        crate::state::SnapMode::RightHalf => Some(Rectangle::new(
            (usable_zone.loc.x + usable_zone.size.w / 2 + 4, usable_zone.loc.y + 8).into(),
            (usable_zone.size.w - usable_zone.size.w / 2 - 12, usable_zone.size.h - 16).into(),
        )),
        crate::state::SnapMode::CenterRestore => None,
    }
}

/// Determina si un punto global de ratón cae dentro de la zona de borde perimetral de redimensión (8 px) de una ventana.
/// NOTA IMPORTANTE: Para no interferir con aplicaciones GTK/Electron (que manejan su propia redimensión y controles),
/// esta detección del lado del servidor se aplica EXCLUSIVAMENTE a Firefox.
pub fn check_window_resize_edge(
    window: &Window,
    loc: Point<i32, Logical>,
    point: Point<f64, Logical>,
    thickness: f64,
) -> Option<ResizeEdge> {
    let uses_ssd = crate::state::window_uses_ssd(window);
    // Solo aplicar a Firefox y a ventanas con decoración SSD del compositor
    if !crate::state::is_firefox_window(window) && !uses_ssd {
        return None;
    }

    let is_maximized_or_fullscreen = window.toplevel().map(|t| {
        let states = &t.current_state().states;
        states.contains(XdgState::Fullscreen) || states.contains(XdgState::Maximized)
    }).unwrap_or(false);

    if is_maximized_or_fullscreen {
        return None;
    }

    let geo = window.geometry();
    if geo.size.w <= 0 || geo.size.h <= 0 {
        return None;
    }

    let ssd_top = if uses_ssd {
        crate::ssd::SSD_TITLEBAR_HEIGHT
    } else {
        0
    };
    let win_x = loc.x as f64;
    let win_y = (loc.y - ssd_top) as f64;
    let win_w = geo.size.w as f64;
    let win_h = (geo.size.h + ssd_top) as f64;

    let rel_x = point.x - win_x;
    let rel_y = point.y - win_y;

    // Fuera de los límites exteriores extendidos por thickness
    if rel_x < -thickness || rel_x > win_w + thickness || rel_y < -thickness || rel_y > win_h + thickness {
        return None;
    }

    let top_inner = if uses_ssd { 5.0 } else { thickness };
    let on_left = rel_x <= thickness;
    let on_right = rel_x >= win_w - thickness;
    let on_top = rel_y <= top_inner;
    let on_bottom = rel_y >= win_h - thickness;

    if !on_left && !on_right && !on_top && !on_bottom {
        return None;
    }

    // Proteger zona de controles de ventana (semáforo macOS en esquina superior izquierda x in [8, 72])
    // para garantizar que los clics en los botones de cerrar/minimizar/maximizar nunca se confundan con resize
    if on_top && rel_x > thickness && rel_x < 72.0 && (!uses_ssd || rel_y > 2.0) {
        return None;
    }
    if uses_ssd && on_left && rel_y >= 8.0 && rel_y <= 30.0 && rel_x >= 10.0 {
        return None;
    }

    let corner_span = 16.0;
    let in_left_corner = rel_x <= corner_span;
    let in_right_corner = rel_x >= win_w - corner_span;
    let in_top_corner = rel_y <= corner_span;
    let in_bottom_corner = rel_y >= win_h - corner_span;

    match (on_left, on_right, on_top, on_bottom) {
        (true, _, _, _) if in_top_corner => Some(ResizeEdge::TopLeft),
        (_, _, true, _) if in_left_corner => Some(ResizeEdge::TopLeft),
        (_, true, _, _) if in_top_corner => Some(ResizeEdge::TopRight),
        (_, _, true, _) if in_right_corner => Some(ResizeEdge::TopRight),
        (true, _, _, _) if in_bottom_corner => Some(ResizeEdge::BottomLeft),
        (_, _, _, true) if in_left_corner => Some(ResizeEdge::BottomLeft),
        (_, true, _, _) if in_bottom_corner => Some(ResizeEdge::BottomRight),
        (_, _, _, true) if in_right_corner => Some(ResizeEdge::BottomRight),
        (true, _, _, _) => Some(ResizeEdge::Left),
        (_, true, _, _) => Some(ResizeEdge::Right),
        (_, _, true, _) => Some(ResizeEdge::Top),
        (_, _, _, true) => Some(ResizeEdge::Bottom),
        _ => None,
    }
}

pub fn cursor_icon_for_resize_edge(edge: ResizeEdge) -> smithay::input::pointer::CursorIcon {
    use smithay::input::pointer::CursorIcon;
    match edge {
        ResizeEdge::Top => CursorIcon::NResize,
        ResizeEdge::Bottom => CursorIcon::SResize,
        ResizeEdge::Left => CursorIcon::WResize,
        ResizeEdge::Right => CursorIcon::EResize,
        ResizeEdge::TopLeft => CursorIcon::NwResize,
        ResizeEdge::BottomRight => CursorIcon::SeResize,
        ResizeEdge::TopRight => CursorIcon::NeResize,
        ResizeEdge::BottomLeft => CursorIcon::SwResize,
        _ => CursorIcon::Default,
    }
}

/// Busca en la pila Z de ventanas si el punto del cursor se encuentra sobre el borde de redimensión de alguna ventana visible.
pub fn find_window_resize_edge_under(
    space: &smithay::desktop::Space<Window>,
    point: Point<f64, Logical>,
    thickness: f64,
) -> Option<(Window, ResizeEdge, Point<i32, Logical>)> {
    for window in space.elements().rev() {
        let Some(loc) = space.element_location(window) else { continue };
        if let Some(edge) = check_window_resize_edge(window, loc, point, thickness) {
            return Some((window.clone(), edge, loc));
        }
        let geo = window.geometry();
        let ssd_top = if crate::state::window_uses_ssd(window) {
            crate::ssd::SSD_TITLEBAR_HEIGHT
        } else {
            0
        };
        let full_rect = Rectangle::new(
            Point::from((loc.x, loc.y - ssd_top)),
            Size::from((geo.size.w, geo.size.h + ssd_top)),
        )
        .to_f64();
        if full_rect.contains(point) {
            return None;
        }
    }
    None
}


