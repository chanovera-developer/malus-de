# Diario de Desarrollo (Devlog) — Malus DE

Registro cronológico y técnico del avance de ingeniería, hitos, experimentos, decisiones de diseño y métricas del entorno de escritorio **Malus DE**.

---

## [2026-11-08] — Corrección de Regresiones de Rendimiento en Compositor, Rediseño de Branches (macOS Finder) y Localización del Menú del Sistema

### Hitos Implementados

* **Previsualización Translúcida de Ajuste de Ventanas con Esquinas Squircle (`Snap Preview`):**
  * Sustitución del antiguo rectángulo opaco de esquinas rectas (`draw_solid` con borde tosco) por un pipeline de vidrio acrílico en GPU ([`GaussianBlur::render_snap_preview`](../crates/compositor/src/blur.rs)).
  * Integración de desenfoque de fondo Dual Kawase bajo el área de acoplamiento, tinte azul translúcido sutil estilo macOS (~14% opacidad) y borde vectorial continuo ($n = 4.2$) que respeta con precisión milimétrica el radio de curvatura de la ventana activa (`win_phys_r`).
  * Aplicación unificada tanto en el backend KMS/DRM nativo ([`drm.rs`](../crates/compositor/src/backend/drm.rs)) como en el backend de pruebas ([`winit.rs`](../crates/compositor/src/backend/winit.rs)).

* **Corrección de Regresiones de Rendimiento y Occlusion Culling en el Compositor (`crates/compositor`):**
  * **Occlusion Culling en Ventanas Maximizadas Redondeadas:** Se implementó [`OpaqueOccluder::contains_rounded_rect`](../crates/compositor/src/backend/mod.rs) para solventar la regresión provocada por la curvatura squircle continua ($n = 4.2$) en ventanas maximizadas. Anteriormente, la deducción de bandas interiores provocaba que una ventana maximizada no pudiera ocluir a otra detrás, desencadenando ráfagas de 60 Hz (`wl_surface.frame`) y overdraw completo de todas las ventanas en segundo plano.
  * **Optimización de Sombras Proyectadas SDF:** Se desactivó el renderizado innecesario de sombras en ventanas maximizadas (`is_maximized && !is_transitioning`) y se implementó [`clamp_rect_to_bounds`](../crates/compositor/src/backend/mod.rs) para recortar las sombras proyectadas al tamaño de la pantalla, evitando falsos negativos en `shadow_occluded` para ventanas en segundo plano.
  * **Resolución de Falso Positivo en Desenfoque Dual Kawase:** Se corrigió [`is_translucent_window`](../crates/compositor/src/state.rs) (que evaluaba por subcadena `"branches"` en el título de la ventana) para prevenir que editores de código o terminales abiertas en carpetas de `Branches` ejecutaran 4 pasadas de desenfoque Gaussiano a pantalla completa cada frame.
  * **Desenfoque Acotado en Branches:** Ahora solo se aplica desenfoque acrílico a la barra lateral izquierda (`212 px`), registrando el panel derecho de archivos como un `OpaqueOccluder` 100% opaco para acelerar el descarte de ventanas traseras.
  * **Eliminación de Hit-Tests Duplicados por Movimiento de Puntero:** Se suprimió la llamada redundante a `update_ssd_hover` previa a `pointer.motion` en [`drm.rs`](../crates/compositor/src/backend/drm.rs) y [`winit.rs`](../crates/compositor/src/backend/winit.rs), agregando retorno anticipado cuando el estado SSD ya se encuentra inactivo.

* **Rediseño Fiel de la Barra Superior de Branches (macOS Finder Sequoia/Sonoma):**
  * Reestructuración visual de la cabecera en [`crates/branches/src/header.rs`](../crates/branches/src/header.rs): disposición unificada con botones de navegación compactos (`<`, `>`), segmented view switcher cuádruple (Iconos, Lista, Columnas Miller, Galería), botón de agrupación/orden y campo de búsqueda integrado.
  * Sincronización e integración de capturas de pantalla de referencia en [`assets/screenshots/`](../assets/screenshots/).

* **Navegación del Sidebar y Soporte de Rutas en Español (`crates/branches/src/sidebar.rs`):**
  * Resolución robusta de nombres de directorios de usuario XDG (`XDG_DESKTOP_DIR`, `XDG_DOCUMENTS_DIR`, `XDG_DOWNLOAD_DIR`, `XDG_PICTURES_DIR`, `XDG_MUSIC_DIR`, `XDG_VIDEOS_DIR`) tanto en inglés como en español (`Escritorio`, `Documentos`, `Descargas`, etc.).
  * Conexión funcional de los eventos de clic en la barra lateral para navegar a las carpetas correspondientes y filtrar archivos por etiquetas de color SQLite.
  * Incorporación de [`is_spanish_locale`](../crates/common/src/lib.rs) para detección unificada de idioma en la sesión.

* **Localización y Nomenclatura del Menú del Sistema (`crates/top-bar/src/menu.rs`):**
  * Actualización de los elementos principales a sus homónimos en Linux: *"Acerca de este equipo"* / *"About This Computer"* y *"Software"* para la apertura de la tienda de aplicaciones con soporte multidioma.

---

## [2026-11-07] — Eliminación de Doble Barra en Apps GNOME, Agente Polkit para GNOME Software y Monitoreo Reactivo de Spotlight

### Hitos Implementados
* **Eliminación Definitiva de Doble Barra de Título en Aplicaciones GNOME:**
  * Identificación exhaustiva del ecosistema GNOME mediante [`is_gnome_window`](../crates/compositor/src/state.rs) e [`is_gnome_identity`](../crates/compositor/src/state.rs) (abarcando `org.gnome.*`, `gnome-*`, Nautilus, Ajustes, Ptyxis, Text Editor, Loupe, etc.).
  * Forzado incondicional de `Mode::ClientSide` en [`XdgDecorationHandler`](../crates/compositor/src/state.rs) e invalidación de `window_uses_ssd`, garantizando que ninguna app GNOME reciba barra SSD del compositor sobre su barra nativa.
  * Preservación del estilo semáforo estilo macOS en GTK 3 y GTK 4 (`close,minimize,maximize:`) sobre las cabeceras nativas (CSD).
  * Concesión estricta de decoraciones del servidor (SSD) solo a clientes que negocien y soliciten explícitamente `Mode::ServerSide` (como ZapFast y herramientas Wayland sin CSD).
* **Integración del Agente de Autenticación Polkit y Asociaciones de Instaladores:**
  * Incorporación del arranque y terminación automática de `mate-polkit` (`/usr/libexec/polkit-mate-authentication-agent-1`) en los scripts de sesión ([`scripts/malus-session`](../scripts/malus-session) y [`scripts/malus-session.sh`](../scripts/malus-session.sh)).
  * Inclusión de `mate-polkit` en el script instalador de sesión ([`scripts/install-session.sh`](../scripts/install-session.sh)) y en el manifiesto de paquetes ([`docs/PACKAGE_MANIFEST.md`](./PACKAGE_MANIFEST.md)).
  * Implementación de [`setup_default_mime_applications`](../crates/common/src/config.rs) para asociar archivos `.rpm`, `.flatpakref`, `.flatpak` y `.deb` a GNOME Software en `mimeapps.list` al hacer doble clic.

### Incidencias Pendientes (Backlog de Depuración)
* **Actualización Dinámica de Aplicaciones en Spotlight (`malus-de-spotlight`):**
  * El indexador de aplicaciones de Spotlight (`crates/spotlight/src/indexer.rs`) escanea los directorios de archivos `.desktop` (`/usr/share/applications`, `~/.local/share/applications`) únicamente durante el arranque inicial del proceso.
  * Al instalar o desinstalar aplicaciones durante la sesión activa (por ejemplo, vía GNOME Software o `dnf`), el catálogo en memoria no se refresca automáticamente, requiriendo cerrar e iniciar sesión para que la nueva app sea visible en los resultados de búsqueda.
  * **Acción para el próximo sprint:** Integrar un observador de cambios en el sistema de archivos (`notify` / `inotify`) sobre los directorios XDG para recargar la lista de aplicaciones de manera reactiva en segundo plano sin reiniciar la sesión.

---

## [2026-11-07] — Decoraciones SSD Nativas macOS, Tema de Cursores `macOS` Integrado y Llavero del Sistema

### Hitos Implementados
* **Decoraciones del Servidor (Server-Side Decorations / SSD) Nativas Estilo macOS (`zxdg_decoration_manager_v1`):**
  * Implementación de [`crates/compositor/src/ssd.rs`](../crates/compositor/src/ssd.rs) (`SSD_TITLEBAR_HEIGHT = 38`), renderizando una barra de título nativa estilo macOS para aplicaciones que negocian `Mode::ServerSide` (como ZapFast y clientes Electron/Wayland sin CSD propio).
  * Incluye los 3 botones semáforo a la izquierda (Cerrar `#ff5f57`, Minimizar `#febc2e` con integración al efecto Genie, Maximizar/Restaurar `#28c840`) con glifos vectoriales al pasar el cursor, título centrado y adaptación automática a modo claro/oscuro.
  * **Recorte Squircle Unificado ($n = 4.2$):** El compositor agrupa la barra SSD y el árbol de superficies de la ventana dentro de un mismo contenedor (`win_screen_rect`), aplicando el shader de esquinas squircle y la sombra proyectada al conjunto completo.
  * **Movimiento y Redimensión Perimetral Interactiva:** Soporte completo de arrastre desde la barra SSD (`MoveSurfaceGrab`), doble clic para maximizar y redimensión desde los 8 bordes y esquinas (`ResizeSurfaceGrab` y `find_window_resize_edge_under` en [`crates/compositor/src/grabs.rs`](../crates/compositor/src/grabs.rs)) con actualización dinámica de iconos de cursor y respeto al tamaño mínimo de ventana.
* **Integración del Tema de Cursores `macOS` (`apple_cursor`) con Respaldo Infalible:**
  * Incorporación de los temas de cursores `macOS` y `macOS-White` (`v2.0.1`) en [`assets/cursors/`](../assets/cursors/) —creados por **Abdulkaiz Khatri ([@ful1e5](https://github.com/ful1e5))** en el proyecto [`apple_cursor`](https://github.com/ful1e5/apple_cursor) bajo licencia GPL-3.0 ([`assets/cursors/LICENSE`](../assets/cursors/LICENSE))—, con instalación automática en `/usr/share/icons`, `/usr/local/share/malus-de/cursors`, `~/.local/share/icons` y `~/.icons`.
  * Respaldo en memoria compilado dentro del compositor (`EMBEDDED_MACOS_LEFT_PTR` en [`crates/compositor/src/backend/drm.rs`](../crates/compositor/src/backend/drm.rs)) y recuperación automática ante superficies de cursor muertas o vacías, garantizando que el cursor `macOS` se muestre en todos los casos incluso cuando ocurran errores de cursor en aplicaciones cliente.
* **Servicio de Llavero (`org.freedesktop.secrets`) en la Sesión:**
  * Integración de `gnome-keyring-daemon --start --components=secrets,pkcs11,ssh` y `libsecret` en [`scripts/malus-session`](../scripts/malus-session), [`scripts/malus-session.sh`](../scripts/malus-session.sh), [`scripts/install-session.sh`](../scripts/install-session.sh) y [`docs/PACKAGE_MANIFEST.md`](./PACKAGE_MANIFEST.md) para dar soporte a aplicaciones que almacenan credenciales seguras (como ZapFast).

---

## [2026-11-07] — Controles Semáforo para Firefox y Soporte XDG Popups en Compositor

### Hitos Implementados
* **Integración Nativa de Mozilla Firefox con `userChrome.css`:**
  * Archivo de estilos [`crates/common/src/styles/firefox-userchrome.css`](../crates/common/src/styles/firefox-userchrome.css) con botones semáforo (cerrar, minimizar, maximizar) a la izquierda, círculos de colores fieles a macOS y glifos vectoriales que se revelan en hover.
  * Inyección automática en perfiles: [`crates/common/src/config.rs`](../crates/common/src/config.rs#L411) detecta perfiles locales y del host (`~/.mozilla/firefox`), genera carpetas `chrome/`, inyecta `userChrome.css` y activa `toolkit.legacyUserProfileCustomizations.stylesheets = true` en `user.js`.
  * Políticas de sistema: [`scripts/install-session.sh`](../scripts/install-session.sh) instala `/etc/firefox/policies/policies.json` para garantizar el soporte de hojas de estilo en cualquier perfil nuevo.
* **Manejo de Popups XDG en Compositor Wayland (Smithay):**
  * Desahogo de límites geométricos mediante `unconstrain_popup` en [`crates/compositor/src/state.rs`](../crates/compositor/src/state.rs) relativo a las coordenadas del padre directo y del monitor.
  * Implementación de retención de foco mediante `popup_grab` (`PopupPointerGrab` y `PopupKeyboardGrab`), canalizando eventos de puntero y teclado exclusivamente a los menús emergentes mientras permanecen abiertos.
  * Corrección de hit-testing en backends DRM ([`crates/compositor/src/backend/drm.rs`](../crates/compositor/src/backend/drm.rs)) y Winit ([`crates/compositor/src/backend/winit.rs`](../crates/compositor/src/backend/winit.rs)) tomando en cuenta el desplazamiento de geometría (`render_loc = location - window.geometry().loc`).

### Incidencias Pendientes (Backlog de Depuración)
* **Desfase en Popups y Menús Desplegables de Navegador (Firefox):**
  * Aunque los popups en Firefox ya responden e interactúan adecuadamente, se observa que los menús emergentes y desplegables (`<select>`) siguen presentando un desfase relativo respecto a su posición de anclaje esperada.
  * En elementos `<select>`, las listas de opciones se expanden en ocasiones por detrás del popup o no respetan el orden $Z$ de la superficie hija.
  * **Acción para la próxima sesión:** Ajustar el cálculo de coordenadas relativas de anclaje (`positioner.get_unconstrained_geometry` vs. posición de toplevel / subsurfaces) y verificar el orden de composición/sub-renderizado de popups jerárquicos en Smithay.
* **Fallo de Popups en Navegadores Chromium (Brave Browser):**
  * A diferencia de Firefox, en **Brave Browser** los menús emergentes (menú hamburguesa, menús contextuales de clic derecho y desplegables) siguen sin funcionar o no se muestran.
  * **Causa a investigar en la próxima sesión:** Brave/Chromium opera mediante el backend Ozone/Wayland y posee requerimientos estrictos sobre el ciclo de configuración de `xdg_popup` (`configure` inicial con dimensiones y token de reposicionamiento, tratamiento de buffers de subsuperficies y respuesta a grabs de puntero). Será necesario revisar en `crates/compositor/src/state.rs` si los popups de Chromium están siendo rechazados por un cálculo de geometría erróneo o si se ejecutan bajo XWayland / Ozone con parámetros específicos.

---

## [2026-11-06] — Editores Electron, Menú Global DBusMenu y Sincronización de Temas

### Hitos Implementados
* **Soporte de Controles Semáforo para Editores Electron (Antigravity IDE & VS Code):**
  * Script wrapper [`scripts/malus-ide-wrapper.sh`](../scripts/malus-ide-wrapper.sh) con inyección de variables de entorno y parámetros de ejecución.
  * Inyección de estilos y configuración de ventana en [`crates/common/src/config.rs`](../crates/common/src/config.rs): controles nativos a la izquierda para editores basados en Electron.
  * Lanzamiento transparente desde Dock ([`crates/dock/src/main.rs`](../crates/dock/src/main.rs)) y Spotlight ([`crates/spotlight/src/main.rs`](../crates/spotlight/src/main.rs)).
* **Menú Global para Firefox y LibreOffice:**
  * Integración en [`crates/top-bar/src/services/dbusmenu.rs`](../crates/top-bar/src/services/dbusmenu.rs) para exportar e interpretar árboles de menú dinámicos de aplicaciones GTK/GDK vía `com.canonical.dbusmenu`.
* **Sincronización Dinámica de Tema y Efecto Ripple:**
  * Detección y aplicación inmediata de `org.gnome.desktop.interface color-scheme` en tiempo de ejecución.
  * Transición visual fluida entre modo oscuro y claro coordinada entre top-bar, dock y compositor.
* **Ajuste Tipográfico y Contraste en Menú Superior:**
  * Atenuación de glifos de atajos de teclado (`.menu-shortcut-hint`) con `rgba(0, 0, 0, 0.28)` en modo claro y `rgba(255, 255, 255, 0.30)` en modo oscuro en [`crates/top-bar/src/styles/style.css`](../crates/top-bar/src/styles/style.css).
  * Hover con contraste blanco puro (`#ffffff`) sobre el fondo de selección azul.

---

## [2026-11-05] — Menú del Sistema macOS, Idioma y Radios Squircle

### Hitos Implementados
* **Menú del Sistema Tipo Apple:**
  * Implementación de opciones completas: *Acerca de este Mac / Acerca de Malus*, *Ajustes del Sistema...*, *App Store*, *Ítems Recientes*, *Forzar salida...*, *Reposo*, *Reiniciar...*, *Apagar equipo...*, *Bloquear pantalla* y *Cerrar sesión*.
  * Soporte de localización e idioma (Español / Inglés).
* **Fidelidad Visual:**
  * Esquinas redondeadas squircle continuas (radio 32px en ventanas y paneles flotantes).
  * Sombras suaves y eliminación de bordes duros.

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
- [x] **Menús Desplegables y Popovers Superelípticos G2:** Módulo `malus-de-render::popover` implementado con sombras difusas multicapa (outer e inner squircle), hit-testing analítico, soporte para menú del sistema Apple y categorías D-Bus, interactividad reactiva en Wayland (hover dinámico, apertura/cierre sin parpadeos y transición fluida entre menús contiguos). Suite `test_popover.rs` validada (3/3 tests OK).
- [x] **Barra Transparente, Eliminación de Sombras y Adaptabilidad de Luminancia:** Módulo `malus-de-render::luminance` implementado con cálculo analítico ITU-R BT.709, paleta adaptativa (`AdaptivePalette`) que ajusta automáticamente los colores tipográficos (blanco para fondos oscuros, grafito oscuro para fondos claros), barra 100% transparente sin píldora opaca de fondo, eliminación de sombras por defecto en popovers y shader Liquid Glass con brillo especular perimetral en WGSL. Suite `test_luminance.rs` validada (3/3 tests OK).
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

## [2026-11-04] — Barra Transparente, Eliminación de Sombras y Adaptabilidad de Luminancia

### Contexto y Requerimientos de Diseño
Siguiendo las pautas de fidelidad visual con macOS y los requerimientos del usuario:
1. **Barra 100% Transparente:** La barra superior se desacopla de fondos tipo píldora opacos (`bg_color = [0, 0, 0, 0]`, `border_width = 0.0`), permitiendo que el wallpaper o el desenfoque del compositor se perciban directamente tras el texto y los iconos.
2. **Eliminación Total de Sombras:** Se desactivan las sombras por defecto (`shadow: false`), garantizando una estética limpia, ligera y sin recuadros oscuros adicionales.
3. **Adaptabilidad del Texto a la Luminancia:** Los colores tipográficos y los resaltados de botones se adaptan en tiempo real al brillo percibido del fondo.

### Componentes Implementados
* **Módulo de Luminancia:** [`crates/render/src/luminance.rs`](../crates/render/src/luminance.rs)
  * Cálculo analítico de luminancia percibida bajo el estándar **ITU-R BT.709**:
    $$L = 0.2126 R + 0.7152 G + 0.0722 B$$
  * Estructura [`AdaptivePalette`](../crates/render/src/luminance.rs#L8-L45) con conmutación dinámica:
    * **Fondo Oscuro ($L < 0.50$):** Texto primario en blanco brillante (`rgba(0.98, 0.98, 1.0, 1.0)`), hover en escarcha translúcida clara (`rgba(1.0, 1.0, 1.0, 0.18)`), popover en vidrio oscuro.
    * **Fondo Claro ($L \ge 0.50$):** Texto primario en grafito oscuro (`rgba(0.10, 0.10, 0.14, 1.0)`), hover en escarcha oscura (`rgba(0.0, 0.0, 0.0, 0.12)`), popover en vidrio claro.
* **Shader WGSL de Liquid Glass Vibrancy:** [`crates/render/src/shader/squircle.wgsl`](../crates/render/src/shader/squircle.wgsl)
  * Brillo especular sutil en el borde superior de los elementos translúcidos (`rim_specular`), simulando la refracción de luz sobre vidrio esmerilado sin costo computacional de pases adicionales.
* **Resultados de Validación:**
  * Suite [`crates/render/tests/test_luminance.rs`](../crates/render/tests/test_luminance.rs) (3/3 tests aprobados).
  * Exportación de capturas verificadas en `target/malus_topbar_transparent_dark.png` y `target/malus_topbar_transparent_light.png`.

---

## [2026-11-04] — Menús Desplegables y Popovers Superelípticos G2 (`malus-de-render::popover`)

### Componentes Implementados
* **Módulo Popover:** [`crates/render/src/popover.rs`](../crates/render/src/popover.rs)
  * Tarjeta flotante superelíptica continua ($n=4.2$, radio $10.0\text{px}$) con borde reflectante sutil y fondo translúcido profundo estilo macOS.
  * **Sombras difusas multicapa (GPU Drop Shadows):**
    * Generación analítica de dos capas de squircle exterior e interior con degradado de opacidad y radio expandido sin costo de multipass en GPU.
  * **Filas Interactivas y Tipografía:**
    * Resaltado en tiempo real al pasar el puntero (hover) cambiando a color de acento macOS (`rgba(0, 122, 255, 0.95)`), squircle de $5.0\text{px}$ y borde blanco fino.
    * Atajos de teclado alineados a la derecha (`TextAlign::Right`) mediante la extensión del motor tipográfico [`TextLabel`](../crates/render/src/text.rs).
    * Separadores de 1px con opacidad sutil entre grupos lógicos.
  * **Menú del Sistema Apple y D-Bus Menu:**
    * Soporte nativo para opciones de sistema (Acerca de Malus DE, Preferencias, Monitor del Sistema, Recarga de Shell, Bloqueo de Sesión, Reiniciar, Apagar).
    * Conversión automática de árboles [`DbusMenuCategory`](../crates/render/src/dbus_menu.rs) a ítems de popover interactivos.
* **Integración Wayland e Interactividad (`WaylandTopBarApp`):**
  * Despliegue con ajuste dinámico de tamaño de superficie en `smithay-client-toolkit` preservando la zona de exclusión original (para evitar que las ventanas del escritorio se desplacen).
  * Manejo reactivo de `PointerHandler`: clic para abrir/cerrar, selección de ítem con ejecución de comandos del sistema, descarte automático al hacer clic fuera del popover y navegación fluida entre menús adyacentes al deslizar el mouse con un menú ya abierto.
* **Resultados de Validación:**
  * Suite [`crates/render/tests/test_popover.rs`](../crates/render/tests/test_popover.rs) aprobada (3/3 tests OK: geometría, hit-testing analítico y exportación de imágenes PNG).
  * Archivos de artefacto generados y verificados: `target/malus_popover_apple.png` y `target/malus_popover_file.png`.

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
