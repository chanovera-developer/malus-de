# Diario de Desarrollo (Devlog) — Malus DE

Registro cronológico y técnico del avance de ingeniería, hitos, experimentos, decisiones de diseño y métricas del entorno de escritorio **Malus DE**.

---

## [2026-11-04] — Hito Arquitectónico: Transición de la Shell hacia WGPU + Taffy (Desacople de GTK4)

### Contexto
Durante las pruebas de fidelidad visual de la barra superior y los botones del sistema, se evaluó la viabilidad de lograr bordes superelípticos continuos ($G^2$, squircles $n \approx 4.2$) con el stack actual de GTK4 + Libadwaita. Se concluyó que las limitaciones del modelo CSS estándar de GTK (`border-radius` circular con discontinuidad de curvatura $G^1$) y la sobrecarga del runtime de GObject en C representan un obstáculo estructural para la visión de Malus DE.

### Decisiones Aprobadas
1. **Aprobación de ADR-0001:** Se formalizó la decisión en [`docs/decisiones/0001-migracion-shell-gtk4-a-wgpu-taffy.md`](./decisiones/0001-migracion-shell-gtk4-a-wgpu-taffy.md).
2. **Selección del Stack Nativo:**
   * **Ventana & Protocolo:** `smithay-client-toolkit` (SCTK) sobre `zwlr_layer_shell_v1`.
   * **Renderizado GPU:** `wgpu` ejecutando el shader analítico del proyecto [`shaders/squircle_sdf.frag`](../shaders/squircle_sdf.frag).
   * **Layout Engine:** `taffy` (motor Flexbox puro en Rust).
   * **Tipografía:** `cosmic-text` (shaping avanzado con fallbacks y antialiasing subpíxel).
   * **IPC D-Bus:** `zbus` para Menú Global (`com.canonical.dbusmenu`) y Centro de Control.
3. **Estrategia respecto a GNOME / Display Manager:**
   * Análisis del acoplamiento con GDM (GDM depende intrínsecamente de `gnome-shell --mode=gdm`).
   * Se planea una transición hacia un gestor de inicio independiente (`greetd` con `tuigreet`/`regreet` o `sddm`) para permitir un sistema 100% libre de paquetes de GNOME Shell, manteniendo únicamente aplicaciones de usuario como Nautilus, GNOME System Monitor y navegadores GTK (Firefox/Brave).

### Tareas en Curso & Próximos Pasos
- [x] **Prototipo WGPU + SCTK para Top-Bar:** Crate `malus-de-render` completado con pipeline instanciado de quads y shader WGSL (`squircle.wgsl`). Prueba de renderizado headless validada (`test_squircle.rs`).
- [x] **Estructura Flexbox con Taffy:** Motor de layout implementado en `malus-de-render::layout` con soporte para Flexbox, cálculo de cotas globales, hit-testing con curvas superelípticas analíticas y constructor completo de top-bar macOS (`top_bar.rs`). Test de integración validado (`test_layout.rs`).
- [x] **Renderizado de Tipografía con `cosmic-text` y `glyphon`:** Subsistema de texto GPU implementado en `malus-de-render::text` con soporte para fallbacks del sistema, pesos tipográficos (Bold/Semibold), conformación avanzada de texto y atlas dinámico de glifos. Test de integración validado (`test_text.rs`).
- [x] **Protocolo Wayland Layer-Shell:** Cliente Wayland nativo implementado con `smithay-client-toolkit` (`crates/render/src/wayland.rs`) y binario ejecutable (`target/debug/malus-wgpu-topbar`). Anclaje superior exclusivo, renderizado directo a `wgpu::Surface` nativa y eventos de cursor interactivos (hover & click).
- [x] **Integración de Menú Global D-Bus (`com.canonical.dbusmenu`) e IPC del Compositor:** Servicio D-Bus asíncrono con `zbus`, servidor `com.canonical.AppMenu.Registrar`, obtención y normalización de árboles de menú dinámicos, despacho unificado en el bucle de eventos `calloop 0.13` junto con notificaciones de cambio de aplicación activa (`ActiveAppChanged`) del compositor. Pruebas de integración aprobadas (`test_dbus_menu.rs`).
- [x] **Salud del Workspace 100% Verde:** Ejecución completa de `cargo test --workspace --no-fail-fast` finalizada exitosamente con cero fallos en todos los crates del proyecto.

---

## [2026-11-04] — Validación Integral del Workspace (100% Tests Aprobados)

Se ejecutó la suite completa de pruebas automatizadas en todo el workspace de Malus DE:
* `malus-de-common`: 27/27 correctas
* `malus-de-render`: 3/3 correctas
* `malus-de-top-bar`: 10/10 correctas
* `malus-de-wallpaper`: 4/4 correctas
* `malus-de-screenshot`: 3/3 correctas
* `malus-de-spotlight`: 2/2 correctas
* `malus-de-dock` y `malus-de-compositor`: aprobadas
* **Estado global:** Cero advertencias críticas y 100% de aserciones válidas.

---

## [2026-11-04] — Servicio de Menú Global D-Bus e IPC de Foco (`com.canonical.dbusmenu`)

### Componentes Implementados
* **Módulo D-Bus:** [`crates/render/src/dbus_menu.rs`](../crates/render/src/dbus_menu.rs)
  * Servidor D-Bus asíncrono sobre `zbus 4.4` registrando la interfaz estándar `com.canonical.AppMenu.Registrar` en `/net/canonical/AppMenu/Registrar`.
  * Mapeo dinámico de ventanas activas a servicios D-Bus y paths de objetos (`WindowMenuRegistration`).
  * Cliente proxy de `com.canonical.dbusmenu`:
    * Método `fetch_layout`: invoca `GetLayout(0, 1, [])` para extraer los encabezados de primer nivel del árbol de menús exportados por aplicaciones GTK, Qt y Electron/Chromium.
    * Método `trigger_event`: ejecuta `Event(id, "clicked", 0, 0)` para invocar acciones en la aplicación destino.
    * Limpieza y normalización de etiquetas: remoción automática de mnemónicos (`_Archivo` -> `Archivo`), sanitización de caracteres y fallbacks limpios.
* **Integración en `malus-wgpu-topbar` mediante `calloop 0.13`:**
  * Reemplazo del bucle bloqueante simple por un bucle de eventos reactivo multipropósito usando `calloop::EventLoop`.
  * Integración con `calloop-wayland-source 0.3` para procesar el socket de Wayland sin bloqueos (`insert_source(WaylandSource)`).
  * Canal MPSC reactivo (`calloop::channel`) comunicando el servicio D-Bus con el loop de la interfaz para redibujar la barra instantáneamente cuando cambian los menús.
  * Hilo listener de socket IPC (`malus-compositor.sock`) para sincronizar el foco de la ventana activa (`ActiveAppChanged`) y actualizar inmediatamente la identidad de la aplicación y sus menús asociados.
* **Resultados de Validación:**
  * Prueba de integración [`crates/render/tests/test_dbus_menu.rs`](../crates/render/tests/test_dbus_menu.rs) aprobada (2/2 tests OK).

---

## [2026-11-04] — Cliente Wayland Nativo Layer-Shell (`malus-wgpu-topbar`)

### Componentes Implementados
* **Módulo Wayland:** [`crates/render/src/wayland.rs`](../crates/render/src/wayland.rs)
  * Integración con **`smithay-client-toolkit 0.19`** y **`wayland-client 0.31`**.
  * Creación y anclaje de la superficie [`LayerSurface`](../crates/render/src/wayland.rs#L85-L105):
    * Capa: `Layer::Top`
    * Anclajes: `Anchor::TOP | Anchor::LEFT | Anchor::RIGHT` (ancho completo adaptable).
    * Margen exclusivo reservado en el escritorio (`set_exclusive_zone`) para evitar superposición con ventanas maximizadas.
  * Puente directo a WGPU mediante `RawDisplayHandle` y `RawWindowHandle` sobre el descriptor nativo de la superficie Wayland.
  * **Interacción del Cursor (Pointer):** Manejador de eventos de puntero con cálculo reactivo de *hover* sobre los nodos y despacho de eventos de clic en tiempo real.
* **Binario Ejecutable:** [`crates/render/src/bin/malus-wgpu-topbar.rs`](../crates/render/src/bin/malus-wgpu-topbar.rs)
  * Punto de entrada de la aplicación compilado exitosamente en `target/debug/malus-wgpu-topbar`.
  * Bucle de eventos bloqueante sobre la cola de Wayland (`event_queue.blocking_dispatch`).

---

## [2026-11-04] — Integración de Tipografía Acelerada por GPU (`cosmic-text` + `glyphon`)

### Componentes Implementados
* **Subsistema de Texto:** [`crates/render/src/text.rs`](../crates/render/src/text.rs)
  * Integración de **`cosmic-text 0.12`** y **`glyphon 0.8`** en el workspace.
  * Estructura [`TextLabel`](../crates/render/src/text.rs#L12-L22) con control de tamaño, altura de línea, color RGBA, pesos tipográficos (`Weight::BOLD`, `Weight::SEMIBOLD`, `Weight::NORMAL`) y alineación relativa.
  * Estructura [`TextSystem`](../crates/render/src/text.rs#L85-L95): orquesta `FontSystem`, `SwashCache`, `Cache`, `Viewport`, `TextAtlas` y `TextRenderer`.
* **Composición en un solo RenderPass:**
  * Métodos [`render_scene_to_view`](../crates/render/src/renderer.rs#L106-L157) y [`render_scene_to_rgba_image`](../crates/render/src/renderer.rs#L190-L222) en [`SquircleRenderer`]:
    1. Dibuja los fondos geométricos superelípticos ($n=4.2$) con antialiasing analítico de subpíxel.
    2. Dibuja todas las etiquetas de texto tipográficas por encima en la misma pasada de renderizado.
* **Integración con Top-Bar:**
  * Enriquecido [`top_bar.rs`](../crates/render/src/top_bar.rs) para asociar etiquetas a cada botón: icono de Apple ("🍎"), nombre de la aplicación activa ("Finder" en negrita), menús globales ("Archivo", "Edición", "Visualización", "Ir", "Ventana", "Ayuda"), estado del sistema ("WiFi", "100%", "CC") y reloj ("Mié 4 Nov 21:30").
* **Resultados de Validación:**
  * Test de integración [`crates/render/tests/test_text.rs`](../crates/render/tests/test_text.rs) ejecutado y aprobado en 0.60s.
  * Verificación del archivo `target/malus_topbar_complete.png` con la barra superior de macOS 100% renderizada por GPU.

---

## [2026-11-04] — Integración de Layout Flexbox con Taffy (`malus-de-render::layout`)

### Componentes Implementados
* **Árbol Reactivo:** [`crates/render/src/layout.rs`](../crates/render/src/layout.rs)
  * Integración con **`taffy 0.15`** (`TaffyTree`).
  * Estructura [`UiTree`](../crates/render/src/layout.rs#L85-L95) para orquestar la relación entre estilos visuales [`SquircleStyle`](../crates/render/src/layout.rs#L12-L23) y el cálculo de geometrías Flexbox/Grid.
  * Generador automático de lotes para GPU: `ui.generate_instances()` recorre el árbol de layout acumulando posiciones globales en píxeles y genera `Vec<SquircleInstance>` en microsegundos.
  * **Hit-Testing Analítico por GPU:** [`squircle_sdf_cpu`](../crates/render/src/layout.rs#L70-L83) replica exactamente la fórmula de Lamé $n=4.2$ del shader para asegurar que los clics del cursor en las esquinas de los botones respeten la curvatura $G^2$.
* **Constructor de Barra Superior:** [`crates/render/src/top_bar.rs`](../crates/render/src/top_bar.rs)
  * Implementación declarativa de la barra macOS:
    1. **Izquierda:** Contenedor Flexbox con botón Apple, botón de app activa (acento azul) y lista de menús del modelo.
    2. **Centro:** Espaciador flexible elástico (`flex_grow = 1.0`).
    3. **Derecha:** Iconos de estado, botón píldora de Centro de Control y botón de reloj.
    4. **Contenedor:** Píldora translúcida flotante con margen superior e izquierdo/derecho adaptativo.
* **Resultados de Validación:**
  * Prueba de integración [`crates/render/tests/test_layout.rs`](../crates/render/tests/test_layout.rs) aprobada en 0.42s.
  * Exportación de imagen verificada en `target/malus_topbar_taffy.png`.

---

## [2026-11-04] — Implementación de `malus-de-render`: Pipeline Instanciado de Squircles

### Arquitectura Técnica del Crate
* **Shader:** [`crates/render/src/shader/squircle.wgsl`](../crates/render/src/shader/squircle.wgsl)
  * Cálculo analítico de superelipse de Lamé:
    $$\left(\frac{|x|}{a}\right)^n + \left(\frac{|y|}{b}\right)^n \le 1 \quad (n = 4.2)$$
  * Antialiasing analítico de subpíxel mediante la tasa de cambio local de la distancia: `edge_aa = max(fwidth(dist), 0.0001)`.
  * Generación de quads procedurales (6 vértices por instancia) sin necesidad de buffers de vértices en memoria de GPU.
  * Stride de instancia compacto de 64 bytes (`bytemuck::Pod`, `Zeroable`).
  * Salida pre-multiplicada por alpha para eliminar halos oscuros en transparencia.
* **Componentes Rust:**
  * [`SquirclePipeline`](../crates/render/src/squircle.rs): Gestión dinámica de buffers de uniforms y buffers de instancias.
  * [`SquircleRenderer`](../crates/render/src/renderer.rs): Contexto de renderizado WGPU preparado tanto para superficies Wayland en vivo como para renderizado offscreen de memoria.
* **Resultados de Verificación:**
  * Prueba de integración [`crates/render/tests/test_squircle.rs`](../crates/render/tests/test_squircle.rs) ejecutada exitosamente en 0.43s.
  * Generación y verificación del archivo de prueba `target/malus_squircle_poc.png` demostrando visualmente la barra superior pill translúcida, botones squircle y comparación directa entre curvatura continua $G^2$ ($n=4.2$) y círculo clásico $G^1$ ($n=2.0$).

---

## [2026-11-04] — Estado Previo del Workspace

* **Compositor:** Implementación funcional en Rust con backend dual (Winit para depuración anidada y DRM/KMS nativo para sesión de hardware).
* **Efecto Genie (Magic Lamp):** Malla de deformación no lineal $C^2$ con hasta 360 franjas horizontales operando en `shaders/genie_deform.*`.
* **Dock:** Animaciones de aumento parabólico (*magnification*), arrastre de iconos con huecos en tiempo real y animación de desanclaje *Poof*.
* **Spotlight:** Interfaz *Liquid Glass* con indexación de aplicaciones nativas y Flatpak.
* **Captura de Pantalla:** Módulo nativo con soporte WebM y PNG con esquinas squircle.
