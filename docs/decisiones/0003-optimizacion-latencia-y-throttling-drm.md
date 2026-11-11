# ADR 0003: Optimización de Latencia de Entrada, Bucle de Eventos y Throttling en Backend DRM

## Estado
Implementado (mergeado en `master`; detalles en [DEVLOG.md](../DEVLOG.md)).

## Contexto
Tras la implementación de Idle Sleeping y Frame Throttling en el backend DRM ([commit 18f9e17](../DEVLOG.md#2026-11-10--implementación-de-idle-sleeping-y-frame-throttling-en-backend-drm-resolución-de-lag-y-regresiones)) y la reversión a la implementación limpia y directa de sombras procedurales squircle ($n=4.2$), el compositor eliminó por completo el parpadeo a 20 Hz, las caídas de búferes en KMS y los textos fantasma en las sombras.

Sin embargo, las pruebas de usuario reportaron una sensación de latencia de entrada (*input latency*) ligeramente superior a la de GNOME Shell (Mutter).

Una auditoría exhaustiva del ciclo de vida de fotogramas y eventos en `crates/compositor/src/backend/drm.rs` identificó tres causas raíz de este desfasaje:

1. **Retardo Artificial en `event_loop.dispatch` (8 ms):**
   En ausencia de animaciones de ventanas (`active_animations.is_empty()`), el bucle de llamadas `calloop`/`epoll` esperaba hasta 8 ms. A 60 Hz (16.6 ms por fotograma), esto inyecta hasta medio cuadro de retraso; a 120 Hz (8.3 ms), equivale a un cuadro completo. Asimismo, tras registrarse daño, el bucle dormía 8 ms antes de poder sincronizar los búferes DRM restantes.
2. **Copia de Pantalla Completa en GPU (`glCopyTexSubImage2D`):**
   En cada fotograma con daño o movimiento de cursor, `blur.capture_background` leía todo el framebuffer (8.3 MB a 33.2 MB) para almacenarlo en `snapshot_tex`, provocando bloqueos síncronos de GPU (2–5 ms de GPU stall), a pesar de que el fondo de pantalla (wallpaper) no sufre alteraciones.
3. **Saturación del Triple-Buffering en Eventos de Cursor:**
   Al moverse el puntero, se asignaba `out_surface.pending_damage_buffers = 3`, forzando tres redibujados completos y redundantes de pantalla por cada micro-evento de libinput.

## Decisión y Plan de Mejoras

Para igualar y superar la fluidez y respuesta de GNOME sin comprometer la fidelidad visual de las sombras squircle ni del desenfoque Dual-Kawase acrílico, se define la siguiente hoja de ruta:

### 1. Despacho Ultra-Reactivo en el Bucle de Eventos
* **Evaluación de Búferes Pendientes:**
  Si cualquier salida DRM tiene trabajo en cola (`pending_damage_buffers > 0`), el tiempo de espera de `event_loop.dispatch` debe ser estrictamente `Duration::ZERO` (0 ms), garantizando la presentación inmediata del swapchain sin esperas pasivas.
* **Reducción de Latencia en Reposo:**
  En estado de reposo absoluto (sin animaciones ni búferes pendientes), reducir el timeout de despacho de `8 ms` a `1 ms`. Esto mantiene un consumo de CPU insignificante (< 0.2%) y asegura que cualquier evento de ratón o teclado sea atendido en < 1 ms.

### 2. Caché Persistente del Fondo Limpio en `GaussianBlur`
* **Invalidación por Eventos de Fondo:**
  Incorporar un indicador `background_dirty` (o sincronización con `state.background_commit_counter`), el cual se activa únicamente ante:
  - Inicialización o cambio de resolución/modo de pantalla.
  - Commits en superficies de `Layer::Background | Layer::Bottom` (cliente `malus-wallpaper`).
  - Transición o cambio de tema de color.
* **Supresión de Lecturas Innecesarias del Framebuffer:**
  Al mover el ratón o interactuar con ventanas, `background_dirty` permanece en `false`. Se omite `blur.capture_background`, reutilizando la textura existente en `snapshot_tex`. El desenfoque acrílico del Dock, Top Bar y ventanas translúcidas continúa operando con fidelidad total, liberando de 2 a 5 ms de GPU por fotograma.

### 3. Throttling Responsivo de Cursor
* **Presentación Inmediata de Fotograma Único:**
  Cuando únicamente cambia la posición del cursor (`cursor_changed` sin commits Wayland ni cambios de geometría), actualizar el búfer actual estableciendo `pending_damage_buffers = pending_damage_buffers.max(1)`.
* **Beneficio:**
  El cursor se refleja en pantalla en el siguiente refresco vertical sin generar una cola de 3 cuadros completos en DRM KMS, evitando saturación del pipeline gráfico.

## Consecuencias
* Eliminación inmediata de la latencia percibida, igualando la respuesta inmediata de Mutter/GNOME Shell.
* Reducción radical del consumo de ancho de banda de memoria GPU en movimiento de puntero.
* Cero regresiones en la geometría squircle de sombras volumétricas y cero parpadeos en KMS.

