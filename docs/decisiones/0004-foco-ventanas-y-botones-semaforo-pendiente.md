# ADR 0004: Pendiente — Ventanas fuera de foco y botones del semáforo sin respuesta al clic

## Estado
Pendiente de resolución definitiva (mitigaciones iniciales aplicadas el `2026-11-11`; el síntoma persiste en sesión en vivo).

## Síntomas Reportados
1. **Ventana elevada sin foco visual (`Activated`):**
   Al seleccionar con el cursor una ventana en segundo plano, ésta pasa al frente de la pila en `Space`, pero en muchas ocasiones conserva los colores y el diseño de ventana inactiva.
2. **Botones del semáforo sin acción al hacer clic:**
   Con frecuencia los botones de cerrar/minimizar/maximizar cambian visualmente al hacer *hover*, pero al hacer clic no ejecutan ninguna acción.

## Mitigaciones Iniciales Ya Aplicadas
En [`crates/compositor/src/state.rs`](../../crates/compositor/src/state.rs), [`crates/compositor/src/backend/drm.rs`](../../crates/compositor/src/backend/drm.rs) y [`crates/compositor/src/backend/winit.rs`](../../crates/compositor/src/backend/winit.rs):
- Sincronización de estado `Activated` y foco de teclado mediante `sync_window_activation` y `set_keyboard_focus_with_sync` (incluyendo cuando el foco pasa a `None` o se minimiza/destruye la ventana).
- Aplazamiento de `xdg_toplevel.configure` de activación (`defer_activation_configure` / `flush_pending_activation_configures`) hasta `ButtonState::Released` para no cancelar gestos en curso durante `Pressed`.
- Limpieza unificada de grabs residuales y popups en `prepare_pointer_press_grabs` y `refresh_popup_grab`.
- Restauración condicional de foco en `layer_destroyed` (solo si la capa/popup destruida retenía el foco, seleccionando `.last()` como ventana superior).

## Qué Investigar Cuando se Retome
- Registrar en sesión DRM en vivo la secuencia exacta de eventos al hacer clic sobre ventanas inactivas y botones del semáforo: `window_under`, `pointer.current_focus()` (incluyendo subsuperficies), `keyboard.current_focus()`, envío de `xdg_toplevel.configure(Activated)` y `PointerButton` (`Pressed`/`Released`).
- Verificar si el clic sobre los botones CSD/SSD está siendo interceptado por un `MoveSurfaceGrab`, por un filtro de región/subsuperficie en `surface_under`, o si el foco de teclado/activación se pierde o queda desfasado frente al orden de apilamiento de `Space` (p. ej. tras usar Spotlight, Alt+Tab, popups o notificaciones).
- Verificar tanto aplicaciones GTK4/Libadwaita (Nautilus, Ptyxis) como aplicaciones Electron/Chromium (Antigravity IDE, Brave) y clientes con decoración SSD.
