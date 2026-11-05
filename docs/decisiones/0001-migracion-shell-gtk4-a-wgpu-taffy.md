# ADR 0001: Migración de la Shell de GTK4 a Stack Nativo en Rust (SCTK + WGPU + Taffy + Cosmic-Text)

* **Fecha:** 2026-11-04  
* **Estado:** Aceptado  
* **Decisores:** Equipo Malus DE  
* **Ámbito:** `malus-de-top-bar`, `malus-de-spotlight`, `malus-de-notifications`, `malus-de-dock`

---

## 1. Contexto y Problemática

El entorno **Malus DE** tiene como objetivo fundamental recrear la ergonomía, coherencia visual y excelencia de interacción de **macOS** sobre Linux y Wayland.

Hasta la fecha, los componentes de la interfaz de usuario de la shell (`top-bar`, `dock`, `spotlight`, `notifications`) fueron construidos sobre **GTK4** (`gtk4-rs`), **Libadwaita** y **`gtk4-layer-shell`**. Si bien este stack permitió un prototipado inicial rápido, impone barreras arquitectónicas insalvables para la fidelidad estética y el rendimiento buscados:

### A. La limitación matemática de CSS y el radio de curvatura
* El diseño de macOS se basa en **superelipses con continuidad de curvatura $G^2$ (squircles)**:
  $$\left(\frac{|x|}{a}\right)^n + \left(\frac{|y|}{b}\right)^n \le 1 \quad \text{con } n \approx 4.2$$
* En contraste, el subsistema de estilizado de GTK4 está basado en CSS estándar, cuyo atributo `border-radius` genera arcos circulares o elípticos ($n=2$).
* En la unión entre una línea recta y un arco circular existe una discontinuidad abrupta en la segunda derivada ($G^1$, no $G^2$). Visualmente, esto crea esquinas con "vértices perceptibles" y el aspecto geométrico plano característico de GNOME/Adwaita, incompatible con la suavidad continua de los controles de macOS.
* Forzar squircles dentro de GTK4 requiere hacks como máscaras SVG pesadas vía `clip-path` o nodos personalizados con `GskGLShaderNode`, lo cual degrada el rendimiento del pipeline de GTK y produce artefactos de aliasing.

### B. Desajuste de ecosistema (Rust vs. GObject en C)
* El núcleo del compositor (`malus-de-compositor`) está programado en Rust puro sobre **Smithay**.
* Desarrollar la shell con `gtk4-rs` obliga a interactuar con una jerarquía de objetos en C (GObject, `glib::Object`, `Rc<RefCell<...>>`, macros de subclases y señales) que añade una gran sobrecarga de desarrollo, limita el paralelismo y contradice el modelo de propiedad y concurrencia de Rust.

### C. Shaders de materiales ópticos (Liquid Glass & Vibrancy)
* La estética macOS requiere desenfoque dual en tiempo real (Dual Kawase), saturación cromática, aberración sutil y bordes con reflexión especular.
* En GTK4, aplicar shaders directos a widgets individuales y componerlos dinámicamente con el fondo del escritorio es inviable de manera nativa sin recurrir a costosos volcados de pantalla o capas intermedias.
* Malus DE ya cuenta con shaders analíticos de alta precisión en `shaders/` (`squircle_sdf.frag`, `dual_kawase_down.frag`, `dual_kawase_up.frag`) que GTK no puede aprovechar de forma nativa en su árbol de widgets.

---

## 2. Decisión Arquitectónica

Se aprueba la **sustitución progresiva de GTK4** en los componentes de la shell del sistema por una arquitectura nativa en Rust compuesta por:

```
+-------------------------------------------------------------+
|                     Malus Shell Client                      |
+-------------------------------------------------------------+
| Layout Engine:        Taffy (Flexbox & Grid puro en Rust)   |
| Text & Typography:    cosmic-text (Shaping, Hinting, Fallback)|
| Vector & SDF Shaders: wgpu (Pipeline GPU con squircle_sdf)  |
| Window & Protocol:    smithay-client-toolkit (Layer Shell)  |
| IPC & Global Menu:    zbus (D-Bus: com.canonical.dbusmenu)  |
+-------------------------------------------------------------+
                              | Wayland Layer-Shell Protocol
                              v
+-------------------------------------------------------------+
|             Compositor Smithay (malus-de-compositor)        |
+-------------------------------------------------------------+
```

### Componentes del nuevo stack:

1. **Protocolo Wayland & Gestión de Capas:**
   * **`smithay-client-toolkit` (SCTK):** Manejo directo del protocolo `zwlr_layer_shell_v1` (anclaje de barra superior, reserva de márgenes exclusivos en el escritorio, ventanas emergentes `xdg_popup` para menús desplegables).
2. **Pipeline Gráfico y Shaders:**
   * **`wgpu`:** Renderizado acelerado por hardware multiplataforma y seguro en Rust.
   * Ejecución directa de los shaders GLSL/WGSL del proyecto:
     * `squircle_sdf.frag` para botones, menús, links y paneles con antialiasing analítico subpíxel (`fwidth`).
     * `dual_kawase_*.frag` para el material translúcido *Liquid Glass*.
3. **Motor de Layout:**
   * **[`taffy`](https://github.com/DioxusLabs/taffy):** Motor de cálculo de layout Flexbox y CSS Grid escrito en Rust puro, sin dependencias de navegadores ni motores web. Permite definir la estructura de la barra superior (Menú Apple, Global Menu, espaciadores elásticos, Centro de Control y Reloj) con velocidad de microsegundos.
4. **Tipografía y Renderizado de Texto:**
   * **[`cosmic-text`](https://github.com/pop-os/cosmic-text):** Manejo completo de fuentes tipográficas, selección automática de fallbacks (Inter, SF Pro, Noto Sans, fuentes CJK, emojis de color), renderizado nítido con hinting y subpíxel antialiasing integrado con WGPU.
5. **Menú Global y Servicios de Sistema:**
   * **`zbus`:** Comunicación D-Bus nativa y asíncrona para implementar el cliente del Menú Global (`com.canonical.dbusmenu` / `org.ayatana.bamf`) e integración con el Centro de Control (PipeWire, NetworkManager, BlueZ).

---

## 3. Consecuencias y Beneficios

### Positivas:
* **Perfección Visual $G^2$:** Todos los botones, links, contenedores y ventanas emergentes contarán con esquinas superelípticas matemáticamente exactas mediante shaders SDF analíticos.
* **Rendimiento Extremo y Reducción de Huella:**
  * El consumo de memoria por proceso de shell descenderá de ~60–80 MB (GTK4 + Libadwaita + WebKit/Cairo) a **~10–15 MB** en Rust puro.
  * Tiempos de arranque inmediatos (sin inicialización del runtime GObject/GTK).
* **Control Absoluto:** Capacidad de animar libremente parámetros de shaders (radio, exponente $n$, opacidad, desenfoque) con interpolaciones físicas y elásticas sin luchar contra hojas de estilo CSS.
* **Ecosistema Homogéneo:** Tanto el compositor (`malus-de-compositor`) como la shell compartirán librerías comunes de Smithay, WGPU y Rust puro.

### Desafíos y Mitigaciones:
* **Manejo de Eventos de Entrada:** Al no haber un framework de widgets monolítico, los eventos de ratón (hover, click, arrastre) y teclado se despachan contra el árbol de nodos de Taffy mediante detección de colisiones 2D simple (bounding boxes + SDF).
* **Transición Gradual:** No se reescribirá todo en un solo paso. Se creará un prototipo inicial enfocado en `malus-de-top-bar`, validando la arquitectura antes de migrar el Dock o Spotlight.

---

## 4. Plan de Ejecución

1. **Fase 1 (Prueba de Concepto - Shell UI Core):**
   * Crear módulo/crate común de renderizado ligero basado en WGPU + Taffy + SCTK.
   * Portar `squircle_sdf.frag` al pipeline de WGPU para renderizado instanciado de quads con superelipse.
2. **Fase 2 (Prototipo Top-Bar):**
   * Montar la barra superior sobre `zwlr_layer_shell_v1`.
   * Integrar layout Flexbox con Taffy: logo Apple, menús de prueba, reloj con `cosmic-text`.
   * Integrar cliente `zbus` para D-Bus Menu.
3. **Fase 3 (Despliegue a la Shell Completa):**
   * Migración de `malus-de-spotlight`.
   * Migración de `malus-de-notifications`.
   * Evaluación de `malus-de-dock` para unificación de shaders con el efecto Genie.
