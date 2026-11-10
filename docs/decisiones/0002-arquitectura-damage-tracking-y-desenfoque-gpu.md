# ADR 0002: Incompatibilidad de Damage Tracking por Scissor en Modo Inmediato y Hoja de Ruta con Offscreen FBO Render Cache

## Estado
Aceptado / Documentado (Damage tracking desactivado en modo inmediato; hoja de ruta establecida).

## Contexto
En el commit `b812250` se intentó implementar Output Damage Tracking en el backend KMS/DRM nativo (`crates/compositor/src/backend/drm.rs`) utilizando `OutputDamageTracker` de Smithay.
El objetivo era reducir el consumo de GPU/batería evitando redibujar áreas estáticas de la pantalla.

Sin embargo, su activación introdujo fallos visuales críticos:
1. **Parpadeo a 20 Hz (Flickering):** En DRM/GBM la GPU opera con un swapchain de triple-buffering (Buffer 0 $\rightarrow$ Buffer 1 $\rightarrow$ Buffer 2). Al recortar el pase con tijera OpenGL (`glScissor`) usando daño parcial, cada búfer recibía fragmentos desincronizados en distintos instantes de tiempo, alternando imágenes discordantes a 60 Hz (detectables en pantalla y grabaciones externas de celular).
2. **Pérdida de desenfoque Dual-Kawase y sombras volumétricas:** El pipeline `GaussianBlur::render_blur` captura el fondo con `glCopyTexSubImage2D`. Si se ejecuta cada frame sobre el daño, se produce un bucle de retroalimentación (feedback loop) que oscurece el fondo. Si se restringe a cambios de geometría (`geom_changed`), cualquier actualización adyacente (como un video de YouTube en Brave o un cursor parpadeante) limpia la zona con `frame.clear` y la ventana estática pierde instantáneamente su sombra y desenfoque (dejando ver el texto de ventanas traseras a través del sidebar).
3. **Desaparición del Dock y elementos de la Top Bar:** Cuando el daño no intersecaba el área de capas superiores de `wlr-layer-shell`, estas no se dibujaban en el búfer rotado, provocando que se cortaran o desaparecieran según la rotación del swapchain.

## Decisión
1. **Desactivar Damage Tracking directo en modo inmediato (`ENABLE_DAMAGE_TRACKING = false`):**
   * El compositor se mantiene en renderizado completo y limpio en cada ciclo de vblank.
   * Se garantiza la estabilidad visual absoluta: sombras volumétricas SDF $n=4.2$, esquinas squircle suaves y desenfoque Dual-Kawase acrílico sin parpadeos ni corrupción en multimonitor.
2. **Requisitos arquitectónicos para retomar Damage Tracking a futuro:**
   * **Opción A (Offscreen FBO Render Cache):** Para ventanas con sombras grandes y desenfoques, la sombra y el blur deben renderizarse en texturas fuera de pantalla (FBOs dedicados por superficie) y actualizarse únicamente cuando la ventana o el fondo tras ella cambie. En el bucle de composición principal, estas texturas se dibujan como quads normales sin invocar shaders pesados ni lecturas directas del framebuffer.
   * **Opción B (Idle Output Sleeping / Frame Throttling):** Si se desea ahorrar energía sin alterar el pipeline de shaders, el compositor puede usar `OutputDamageTracker` únicamente como detector booleano: si la pantalla física tiene 0 cambios (sin commits de clientes ni movimientos de cursor), se salta completamente el renderizado (0% GPU). Si hay cambios (movimiento, video o cursor), se renderiza la pantalla completa de forma limpia.

## Consecuencias
* El compositor Malus DE es completamente estable, fluido a 60 Hz y fiel al diseño gráfico macOS Tahoe.
* Todo agente de IA o desarrollador futuro debe consultar este registro antes de reintentar Damage Tracking directo sobre el framebuffer principal.
