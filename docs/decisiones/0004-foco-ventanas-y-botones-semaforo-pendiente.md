# ADR 0004: Resuelto — Ventanas fuera de foco y botones del semáforo sin respuesta al clic

## Estado
Resuelto el `2026-11-26` (mitigaciones iniciales el `2026-11-11`; causa raíz localizada y corregida en `malus-de-compositor`).

## Síntomas Reportados
1. **Ventana elevada sin foco visual (`Activated`):**
   Al seleccionar con el cursor una ventana en segundo plano, ésta pasa al frente de la pila en `Space`, pero en muchas ocasiones conserva los colores y el diseño de ventana inactiva.
2. **Botones del semáforo sin acción al hacer clic:**
   Con frecuencia los botones de cerrar/minimizar/maximizar cambian visualmente al hacer *hover*, pero al hacer clic no ejecutan ninguna acción hasta mover ligeramente la ventana.

## Causas Raíz Identificadas y Corregidas (`2026-11-26`)
En [`crates/compositor/src/state.rs`](../../crates/compositor/src/state.rs), [`crates/compositor/src/grabs.rs`](../../crates/compositor/src/grabs.rs), [`crates/compositor/src/ssd.rs`](../../crates/compositor/src/ssd.rs), [`crates/compositor/src/backend/drm.rs`](../../crates/compositor/src/backend/drm.rs) y [`crates/compositor/src/backend/winit.rs`](../../crates/compositor/src/backend/winit.rs):
- **Configure de `Activated` nunca enviado (`sync_window_activation`):** en Smithay 0.7.0, `Window::set_activated` y `Space::raise_element(_, true)` ya mutan `pending_state.states` sin enviar configure. Comprobar `state.states.contains(Activated)` después de `win.set_activated(is_focused)` devolvía siempre `changed = false`. Se corrigió usando `toplevel.has_pending_changes()` y `toplevel.send_pending_configure()`, sincronizando siempre desde `set_keyboard_focus_with_sync` y vaciando `pending_activation_configures` en toda ruta de `Released`, inicio de `move_request`/`resize_request` y `unset` de grabs.
- **Estado visual activo inmediato:** las barras SSD y sombras ahora evalúan `is_window_visually_active` basado en el foco real de teclado (incluyendo popups hijos) en lugar de mantener activas ventanas ya desenfocadas por un `||` con `current_state`.
- **Orden de grabs/puntero y validación de serial:** `prepare_pointer_press_grabs` ahora corre antes de `pointer.motion` en `ButtonState::Pressed`, emitiendo `pointer.frame` al cambiar el foco del puntero; `move_request` valida `pointer.has_grab(serial)` para evitar secuestros de clic por peticiones tardías.
- **Paridad hover/clic en semáforo y bordes de redimensión:** `update_ssd_hover` usa coordenadas locales por monitor y el mismo umbral de `malus-top-bar` (`max(geo.size.h, 36.0)`) que `find_layer_under_layer`; `check_window_resize_edge` protege el semáforo en ambos lados (`buttons_right` izquierda/derecha) mediante `is_in_traffic_light_resize_exclusion`, y `BTN_HIT_RADIUS` SSD sube a `9.0 px`.