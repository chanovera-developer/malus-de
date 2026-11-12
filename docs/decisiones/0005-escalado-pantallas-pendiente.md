# ADR 0005: Pendiente — Consecuencias y problemas por resolver del escalado de pantallas

## Estado
Parcialmente resuelto el `2026-11-11`: **Problema 4 (heurística de diagonal/PPI y tamaño físico `mm`) corregido**; los puntos 2–7 siguen pendientes.

## Resuelto

### 1. Heurística frágil de PPI/diagonal y alteración del tamaño físico (`mm`) *(Problema 4 — Resuelto el `2026-11-11`)*
- **Dónde ocurría:** [`crates/common/src/config.rs`](../../crates/common/src/config.rs), [`crates/compositor/src/backend/drm.rs`](../../crates/compositor/src/backend/drm.rs), [`crates/ctl/src/main.rs`](../../crates/ctl/src/main.rs), [`crates/settings/src/main.rs`](../../crates/settings/src/main.rs) y [`crates/top-bar/src/ui/control_center.rs`](../../crates/top-bar/src/ui/control_center.rs).
- **Solución aplicada:**
  - `DisplayOutputConfig::effective_diagonal()`, `ppi()` y `physical_size_mm()` devuelven `Option` cuando no existe `diagonal_inches` válido, eliminando la suposición de `27.0"` por defecto y la inferencia por nombre en `from_gnome_xml` / `effective_diagonal`.
  - Se unificó la sincronización proporcional 1:1 en `DisplaysConfig::sync_linked_scales_from`: los paneles sin diagonal conocida conservan su escala actual en lugar de recibir proporciones inventadas como de 27".
  - En `drm.rs`, `Output::new` preserva los milímetros reales reportados por EDID (`conn_info.size()`) salvo corrección manual explícita (`diagonal_inches` diferente en `> 0.5"`), o `(0, 0)` si faltan.
  - Se añadió `DisplayOutputConfig::recommended_scale` (~109 PPI base en pasos de `0.25`) para inicializar pantallas nuevas sin entrada previa en lugar de fijar `2.0x`.
  - Verificado con `cargo test` (`malus-de-common` 36/36, `malus-de-compositor` 41/41, `malus-de-top-bar` 10/10).

---

## Otros Problemas Detectados

### 2. Apps XWayland (X11) sin escalado HiDPI *(Problema 1)*
- **Dónde ocurre:** [`crates/compositor/src/backend/xwayland.rs`](../../crates/compositor/src/backend/xwayland.rs) (`start`).
- **Problema:** Se inicia `XWayland::spawn` y `X11Wm::start_wm`, pero no se configura escala de XWayland ni `Xft.dpi`. Las apps X11 renderizan a `1x` y el compositor las estira por GPU (`Scale::Fractional`), viéndose borrosas en pantallas HiDPI o fraccionarias.
- **Acciones sugeridas:** Evaluar soporte de escalado nativo en XWayland / propagación de `Xft.dpi` y escala X11 según la configuración de monitores.

### 3. Clientes Wayland sin `wp_fractional_scale_v1` y estado de `malus-wgpu-topbar` *(Problema 2)*
- **Dónde ocurre:** [`crates/compositor/src/backend/drm.rs`](../../crates/compositor/src/backend/drm.rs) (`Scale::Fractional`) y [`crates/render/src/wayland.rs`](../../crates/render/src/wayland.rs) / [`crates/render/src/text.rs`](../../crates/render/src/text.rs).
- **Problema:** Los clientes que solo implementan `wl_output.scale` (entero) renderizan a escala entera y son reescalados por el compositor con pérdida de nitidez. Además, `crates/render` (`malus-wgpu-topbar`) solo escucha `CompositorHandler::scale_factor_changed` entero e ignora `_scale_factor` en `TextSystem::prepare`.
- **Acciones sugeridas:** Implementar `wp_fractional_scale_v1` + `wp_viewporter` en `crates/render` (`malus-de-render`) y aplicar correctamente `scale_factor` al texto/viewport.

### 4. Imprecisiones de ±1 px por escalas fraccionarias arbitrarias y redondeo lógico *(Problema 3)*
- **Dónde ocurre:** [`crates/common/src/config.rs`](../../crates/common/src/config.rs) (`DisplayOutputConfig::logical_size`, `set_output_scale`), [`crates/compositor/src/backend/drm.rs`](../../crates/compositor/src/backend/drm.rs) y [`crates/top-bar/src/ui/control_center.rs`](../../crates/top-bar/src/ui/control_center.rs).
- **Problema:** `logical_size()` redondea `(width / scale).round() as i32` y los presets/cálculos 1:1 producen factores de 4 decimales (`1.333333`, `1.525`, `1.71`, `1.6875`) que no dividen exactamente la resolución física, produciendo discrepancias de ±1 px en bordes, líneas finas o regiones y un costo extra de reescalado en GPU.
- **Acciones sugeridas:** Ajustar los factores permitidos o alinear el cálculo a divisores exactos/múltiplos de 1/120 (protocolo `wp_fractional_scale_v1`) cuando sea posible, minimizando errores de redondeo lógico ↔ físico.

### 5. Layout forzado al recargar o cambiar escala (`recalculate_layout`) *(Problema 5)*
- **Dónde ocurre:** [`crates/common/src/config.rs`](../../crates/common/src/config.rs) (`DisplaysConfig::load`, `recalculate_layout`).
- **Problema:** `recalculate_layout()` se invoca incluso al cargar `monitors.toml`, forzando los monitores de izquierda a derecha desde `x = 0` sin huecos y normalizando `y >= 0`. Esto impide disposiciones verticales puras o coordenadas negativas respecto al primario.
- **Acciones sugeridas:** Preservar arreglos verticales/personalizados y recalcular solo colisiones o deltas de ancho/alto al cambiar la escala de un monitor ya posicionado.

### 6. Grabación de pantalla en multimonitor con escalas distintas *(Problema 6)*
- **Dónde ocurre:** [`crates/compositor/src/state.rs`](../../crates/compositor/src/state.rs) (`start_screen_recording` / cálculo de `phys_rect`).
- **Problema:** Al grabar una región (`rect: Some((x, y, w, h))`), la escala se toma siempre del primer monitor (`self.space.outputs().next()`) en lugar del monitor que contiene la región seleccionada, produciendo regiones físicas erróneas al grabar en pantallas secundarias con distinta escala.
- **Acciones sugeridas:** Detectar el `Output` que contiene el centro o la intersección principal de `rect` (similar a `preferred_scale_for_surface_with_window`) y convertir a coordenadas físicas relativas a ese monitor (o manejar regiones que crucen monitores).

### 7. Desalineación entre el backend DRM y el backend Winit *(Problema 7)*
- **Dónde ocurre:** [`crates/compositor/src/backend/winit.rs`](../../crates/compositor/src/backend/winit.rs) frente a [`crates/compositor/src/backend/drm.rs`](../../crates/compositor/src/backend/drm.rs).
- **Problema:** `drm.rs` implementa `Scale::Fractional`, recarga en vivo de `monitors.toml`, reconfiguración de capas y migración de ventanas, mientras `winit.rs` mantiene `Scale::from(1.0)` fijo en geometría y renderizado.
- **Acciones sugeridas:** Unificar o propagar el soporte de escala fraccionaria en `winit.rs` para poder depurar cambios de HiDPI en sesión anidada.

