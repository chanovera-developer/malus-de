# Diario de Desarrollo (Devlog) — Malus DE

Registro cronológico y técnico del avance de ingeniería, hitos, experimentos, decisiones de diseño y métricas del entorno de escritorio **Malus DE**.

---

## [2026-11-08] — Cero Latencia en Miniatura Flotante de Arrastre y Desaparición Completa del Elemento de Origen (Sensación Física Real estilo macOS Finder)

### Diagnóstico de Causa Raíz
* **Latencia y Lapso de Cursor Vacío (*Empty Cursor Latency*):**
  * Al configurar la miniatura de arrastre dinámicamente dentro del callback `connect_drag_begin` mediante `gtk4::WidgetPaintable::new(Some(&widget))`, el compositor Wayland iniciaba la sesión de arrastre (`wl_data_device.start_drag`) sin que la superficie del cursor tuviese aún un `wl_buffer` asociado. GTK4 debía esperar al siguiente ciclo de snapshot/commit de la ventana para renderizar el widget, provocando que el cursor estuviera vacío durante los primeros 1 a 3 fotogramas del arrastre.
* **Silueta o "Recorte" Residual en la Posición de Origen:**
  * Las clases CSS `.desktop-icon-dragging` y `.branches-icon-dragging` aplicaban `opacity: 0.35` con una transición de 150ms (`transition: opacity 150ms ease`). Esto dejaba una silueta o "recorte" translúcido en la posición original del archivo en el escritorio o explorador, arruinando la ilusión óptica de que el archivo fue "levantado" físicamente de la superficie.
* **Conflicto de Acoplamiento de Opacidad en `WidgetPaintable`:**
  * En GTK4, si se intenta forzar `opacity: 0.0` sobre el widget de origen mientras se usa `WidgetPaintable::new(Some(&widget))`, la función de snapshot de GTK descarta el renderizado del widget (`if (gtk_widget_get_opacity (widget) < 0.001) return;`), haciendo que la miniatura flotante bajo el cursor también desaparezca por completo.

### Solución Implementada (`crates/wallpaper`, `crates/branches`)
* **Pre-configuración Temprana e Inmediata del Icono de Arrastre:**
  * Se desacopló la miniatura flotante del widget de la interfaz mediante la función `resolve_desktop_icon_paintable` (en `wallpaper`) y `lookup_by_gicon` (en `icon_view` y `list_view` de `branches`).
  * Para imágenes con miniatura se genera directamente un `gdk::Texture::for_pixbuf`, y para directorios o archivos generales se consulta `gtk4::IconTheme::for_display(...).lookup_by_gicon(...)`, obteniendo un `gdk::Paintable` independiente y persistente.
  * Se asigna el icono con `drag_source.set_icon(Some(&paintable), icon_size / 2, icon_size / 2)` inmediatamente al instanciar el `DragSource`, antes de que comience cualquier interacción. Al activarse el arrastre, el búfer ya está listo en Wayland, logrando respuesta visual instantánea (0 ms, cero lapso de cursor vacío).
* **Desaparición Instantánea del Elemento de Origen (Sin Recortes ni Siluetas):**
  * Se actualizaron las reglas CSS en [`crates/wallpaper/src/styles/default-gradient.css`](../crates/wallpaper/src/styles/default-gradient.css) y [`crates/branches/src/styles/macos-branches.css`](../crates/branches/src/styles/macos-branches.css):
    * `.desktop-icon.desktop-icon-dragging { opacity: 0.0; transition: none; }`
    * `.branches-icon-dragging, .branches-icon-item.branches-icon-dragging, .branches-list-row.branches-icon-dragging { opacity: 0.0 !important; transition: none !important; }`
  * Como el icono del cursor utiliza una textura o `IconPaintable` autónomo, permanece 100% visible, nítido y opaco, mientras que la posición original desaparece al instante en cuanto el puntero se mueve, transmitiendo la sensación física idéntica a macOS Finder de que el archivo ha sido despegado y tomado por el cursor.
* **Cobertura con Pruebas Unitarias:**
  * Se agregaron aserciones y tests (`test_resolve_desktop_icon_paintable`, `test_desktop_drag_and_drop_macos_css_classes` y `test_branches_macos_drag_and_drop_styles`) verificando la presencia de `opacity: 0.0` y `transition: none`, garantizando que futuras modificaciones no reintroduzcan siluetas residuales ni dependencias circulares de opacidad.

---

## [2026-11-08] — Efecto de Mover Carpetas Estilo macOS Finder (Drag & Drop Wayland Bidireccional, Drag Ghost Translúcido, Resaltado de Carpeta Receptora y Animación Elástica de Aterrizaje)

### Diagnóstico de Causa Raíz
* **Inexistencia de `DragSource` en los Iconos del Escritorio:**
  * Anteriormente, los iconos en [`crates/wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs) sólo implementaban un `GestureDrag` local sobre el widget `Fixed`. Al arrastrar un icono para moverlo a otra ventana (como Branches o Nautilus), el puntero jamás iniciaba una sesión Wayland de Drag & Drop (`wl_data_device.start_drag`), haciendo físicamente imposible mover carpetas o archivos desde el escritorio hacia el gestor de archivos.
* **Ausencia de Previsualización Gráfica (`Drag Ghost Icon`):**
  * En [`crates/branches/src/views/icon_view.rs`](../crates/branches/src/views/icon_view.rs) y [`crates/branches/src/views/list_view.rs`](../crates/branches/src/views/list_view.rs), se utilizaba `gtk4::DragSource` pero sin conectar `connect_drag_begin` ni suministrar `drag_source.set_icon(&paintable, hot_x, hot_y)`. En Wayland, el compositor recibía una superficie nula (`icon: None`), dejando al cursor sin miniatura flotante durante el vuelo.
* **Falta de Detección de Carpetas Receptoras y Aterrizaje Brusco:**
  * Al arrastrar un elemento sobre una carpeta existente en el escritorio o en el explorador, la carpeta no reaccionaba visualmente ni aceptaba soltar dentro de ella; únicamente se permitía soltar en el fondo plano. Al soltar en el escritorio, el icono aparecía instantáneamente sin suavidad física.

### Solución Implementada (`crates/wallpaper`, `crates/branches`)
* **`DragSource` Nativo y Bidireccional en el Escritorio (`crates/wallpaper/src/main.rs`):**
  * Se equipó a cada icono de escritorio (`render_single_icon`) con un `gtk4::DragSource` configurado con `gdk::DragAction::COPY | gdk::DragAction::MOVE` y empaquetado en `gdk::FileList`.
  * Soporte de selección múltiple: al arrastrar un icono seleccionado, se transfieren todos los elementos activos simultáneamente.
  * Al comenzar el arrastre, se captura un snapshot exacto del widget con `gtk4::WidgetPaintable::new(Some(&item_box))` y se asigna al cursor con `source.set_icon`, mientras el elemento original se atenúa a `opacity: 0.35` (`.desktop-icon-dragging`).
* **Resaltado Acrílico de Carpeta Receptora (`Drop Hover Glow`):**
  * Si el elemento es un directorio (`path.is_dir()`), se añade un `gtk4::DropTarget` dedicado al icono de la carpeta (tanto en el Escritorio como en las vistas de Iconos y Lista de Branches).
  * Al pasar por encima mientras se arrastran archivos (`connect_enter`), se activa la clase `.folder-drop-hover`: halo azul acrílico translúcido (`rgba(0, 122, 255, 0.28)`), borde redondeado brillante de 2px, sombra exterior de 16px y ligera expansión física (`transform: scale(1.05 / 1.06)`).
  * Al soltar sobre la carpeta, los archivos se mueven automáticamente a esa subcarpeta (`target_dir.join(name)`).
* **Física y Microanimación de Aterrizaje Suave (`macos-drop-landing`):**
  * Se implementó `@keyframes macos-drop-landing` con curva elástica Apple `cubic-bezier(0.16, 1, 0.3, 1)` de 240ms (`scale(1.12) -> scale(0.98) -> scale(1.0)` y `opacity: 0.6 -> 1.0`).
  * Los elementos soltados en el escritorio reciben la clase `.landing` temporalmente mediante `glib::timeout_add_local_once`, logrando un asentamiento con peso físico idéntico al de macOS Finder.
* **Eliminación de la Estela Semitransparente en Movimiento:**
  * Se eliminó el callback `move_desktop_icon` que se ejecutaba en cada evento de `drop_target.connect_motion` en [`crates/wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs), el cual desplazaba el widget de origen paralelamente al puntero generando un efecto de estela o duplicación semitransparente.
  * Se desacopló de [`crates/branches/src/views/icon_view.rs`](../crates/branches/src/views/icon_view.rs) el antiguo `GestureDrag` que movía el widget del lienzo en tiempo real, delegando el reordenamiento libre y snap al manejador `connect_drop` del lienzo, logrando que únicamente la miniatura flotante de Wayland siga al cursor con cero latencia ni estelas de fondo.
* **Integración Completa en Branches (`crates/branches/src/views/icon_view.rs`, `crates/branches/src/views/list_view.rs`):**
  * Soporte de arrastre con `WidgetPaintable` e icono translúcido en Icon View y List View.
  * Receptores de carpetas en subdirectorios y soltado en el lienzo para absorber archivos arrastrados desde el Escritorio.

---

### Diagnóstico de Causa Raíz
* **Rechazo de Reglas de Color por `!important` en el Motor CSS de GTK4:**
  * GTK4 implementa un subconjunto estricto de especificaciones CSS donde `!important` no es un modificador válido y es calificado como *"Junk at end of value"*. Esto provocaba que GTK4 descartase por completo las declaraciones `background-color` y `border`, dejando los popovers 100% transparentes tanto en modo oscuro como en modo claro.
* **Sobreescritura Uncondicional de Tipografía por Bloque `@media`:**
  * GTK4 no evalúa consultas condicionales `@media (prefers-color-scheme)`. Las reglas ubicadas al final de la hoja (`color: #1e293b`) se aplicaban de forma incondicional sobre todos los botones, provocando texto gris oscuro casi negro sobre el fondo transparente del wallpaper oscuro, resultando en ilegibilidad absoluta.
* **Aislamiento de Sesión y Falta de Bus IPC entre Procesos:**
  * Malus DE configura `GSETTINGS_BACKEND="keyfile"`. El backend `keyfile` de GLib no emite señales D-Bus entre procesos cuando otro demonio (como Top Bar o Settings) altera la configuración de apariencia. Al depender únicamente de `gio::Settings::connect_changed`, `malus-de-wallpaper` nunca era notificado del cambio de tema en caliente.

### Solución Implementada (`crates/wallpaper/src/styles/default-gradient.css`, `crates/wallpaper/src/main.rs`)
* **Hoja de Estilos Limpia y Libre de Advertencias GTK4:**
  * Eliminación total de `!important`, propiedades inválidas (`xalign`, `justify`) y bloques `@media`.
  * Reglas limpias para fondo acrílico oscuro (`rgba(30, 30, 36, 0.82)`) y claro (`rgba(246, 246, 248, 0.85)`).
  * Clases temáticas directas para etiquetas: `label.menu-dark-text` (`#f1f5f9`, blanco nítido) y `label.menu-light-text` (`#1e293b`, grafito oscuro), con hover `#ffffff` y acento azul macOS `#007aff`.
* **Conexión Directa al Socket IPC del Compositor:**
  * Se implementó un hilo lector sobre `malus_de_common::ipc::ipc_socket_path()` con canal asíncrono hacia el hilo principal de GTK.
  * Al recibirse `IpcMessage::ThemeChanged { is_dark, .. }`, se actualiza el estado atómico global `CURRENT_IS_DARK`, se sincroniza `gtk_application_prefer_dark_theme` y se refrescan de inmediato todos los popovers registrados.
* **Gestor Centralizado de Popovers de Escritorio (`register_wallpaper_popover`, `apply_theme_to_popover`):**
  * Se unificó la gestión de popovers (fondo primario, fondos secundarios, iconos de escritorio y diálogos de renombrar / nueva carpeta).
  * Al mostrarse o crearse cualquier menú, se aplica dinámicamente la clase `.light-popover` y se recorre el árbol de widgets para asignar `.menu-light-text` o `.menu-dark-text` con contraste garantizado.

---

## [2026-11-08] — Diseño Acrílico en Menús Emergentes del Escritorio (Alineación con Nautilus, Dual Kawase Blur, Sombra GPU y Squircle)

### Hitos Implementados

* **Pipeline de Renderizado de Popovers de Fondo en el Compositor (`crates/compositor/src/backend/drm.rs`, `crates/compositor/src/backend/winit.rs`):**
  * **Captura de Superficies Popover del Fondo de Pantalla (`wallpaper_popups`):** Se interceptan los popovers xdg pertenecientes a la superficie layer shell `malus-wallpaper`, extrayendo su geometría y árboles de renderizado físico para aplicarles el mismo pipeline de postprocesado GPU reservado a ventanas toplevel como Nautilus.
  * **Sombra Volumétrica Flotante (`shadow.render_shadow`):** Sombra Gaussiana profunda con desplazamiento vertical sutil (`is_active = true`), eliminando las sombras CSS planas generadas en el cliente y sustituyéndolas por la penumbra envolvente del compositor.
  * **Desenfoque Dual-Kawase de Fondo (`blur.render_blur`):** Desenfoque por GPU en tiempo real sobre la escena completa detrás del popover con radio de redondeo adaptativo (`menu_radius`).
  * **Recorte Squircle Continuo (`shader.apply_to_frame`):** Superelipse $G^2$ ($n = 4.2$) aplicada mediante shader GLSL a la textura de los popovers con radio de esquina de 32px físicos.

* **Rediseño Visual y Estilizado Acrílico en el Escritorio (`crates/wallpaper/src/styles/default-gradient.css`, `crates/wallpaper/src/main.rs`):**
  * **Misma Transparencia y Bordes que Nautilus:** Fondo translúcido con opacidad acrílica (`rgba(30, 30, 34, 0.82)` en tema oscuro / `rgba(246, 246, 248, 0.82)` en tema claro) y borde sutil de 1px (`rgba(255, 255, 255, 0.14)` / `rgba(0, 0, 0, 0.12)`).
  * **Eliminación de la Sombra CSS:** Se retiró el `box-shadow` CSS de la caja interior, permitiendo que el contenedor coincida de manera exacta con el tamaño físico de la superficie Wayland sin colchones de padding sobrante.
  * **Ergonomía y Alineación de Elementos:** Botones de menú (`desktop-context-btn`) con altura mínima de 28px, bordes redondeados tipo squircle de 8px, alineación de texto a la izquierda (`xalign: 0.0`) y selector azul macOS (`#007aff`) en hover con tipografía blanca nítida.

---

## [2026-11-08] — Eliminación de Oscilación/Jitter y Arrastre Unificado de Iconos en Overlay

### Diagnóstico de Causa Raíz
* **Bucle de Retroalimentación de Coordenadas de Widgets Móviles en GTK4 (*Moving-Widget Feedback Loop*):**
  * Al asociar un `GestureDrag` directamente a cada `item_box` individual dentro de `GtkFixed`, cada llamada a `fixed.move_(&item_box, new_x, new_y)` en el callback `connect_drag_update` alteraba de forma instantánea el origen de coordenadas locales del widget bajo el cursor.
  * GTK4 calcula el delta de movimiento (`offset_x`, `offset_y`) restando la posición actual del puntero respecto a la posición inicial en las coordenadas del propio widget receptor. Al moverse el widget en dirección al cursor, el desplazamiento en el siguiente evento de movimiento se contraía o revertía artificialmente.
  * Esto producía una violenta oscilación a 60/120 Hz donde el icono vibraba o temblaba entre dos posiciones antagónicas, desincronizándose del cursor y cayendo en una posición aleatoria al soltar el botón según la fase de la oscilación en ese fotograma.

### Arquitectura de Solución Implementada (`crates/wallpaper/src/main.rs`)
* **Controlador de Arrastre Centralizado en `GtkOverlay` (`desktop_drag`):**
  * Se retiró completamente el `GestureDrag` de las cajas individuales de los iconos (`item_box`).
  * Se unificó el arrastre del escritorio en un único `GestureDrag` sobre el contenedor raíz `GtkOverlay`, cuyas coordenadas están fijadas estáticamente a `(0, 0)` del monitor y nunca se desplazan ni se alteran.
  * En `connect_drag_begin(start_x, start_y)`:
    * Se utiliza `find_icon_at` recorriendo los hijos en orden inverso (z-order superior a inferior) con hit-testing exacto (108x112px mínimo) y validación de metadatos del archivo (`p.symlink_metadata().is_ok()`).
    * Si el clic ocurre sobre un icono: se adopta `DesktopDragMode::Icon`, asegurando la selección del elemento y almacenando las posiciones de anclaje iniciales de todos los iconos seleccionados.
    * Si el clic ocurre sobre espacio vacío: se adopta `DesktopDragMode::Rubberband` para selección elástica rectangular.
  * En `connect_drag_update(offset_x, offset_y)`:
    * Cuando el desplazamiento supera el umbral de movimiento inicial (> 2px), se reclama la secuencia (`gesture.set_state(Claimed)`) y se actualiza la posición de cada icono seleccionado:
      $$\text{pos}_\text{nueva} = \text{clamp}(\text{pos}_\text{inicial} + \vec{\Delta}_\text{cursor}, (16, 56), (W - 108 - 16, H - 112 - 20))$$
    * Al ser $\vec{\Delta}_\text{cursor}$ un vector estático y continuo emitido por el `overlay`, no existe bucle de retroalimentación: el arrastre es 100% fluido, con seguimiento 1:1 bloqueado con precisión de píxel bajo el cursor y cero temblores.
  * En `connect_drag_end(offset_x, offset_y)`:
    * Si hubo desplazamiento real (> 3px), se imantan las coordenadas finales a la cuadrícula (si `snap_to_grid` está activado) o se conservan las coordenadas libres arbitrarias y se guardan inmediatamente en `~/.config/malus-de/desktop-icons.json`.
    * Si fue un clic simple sin arrastre sobre un elemento que formaba parte de una selección múltiple previa, se deseleccionan los otros elementos dejando seleccionado únicamente el icono pulsado (comportamiento estándar estilo macOS / Windows).

---

## [2026-11-08] — Corrección del Menú Contextual (Clic Derecho), Posicionamiento Libre de Iconos y Despeje de la Top-Bar

### Hitos Implementados

* **Corrección de Eventos y Menús Emergentes de Clic Derecho (`crates/wallpaper/src/main.rs`, `crates/wallpaper/src/styles/default-gradient.css`):**
  * **Picking de Eventos en Lienzo de Fondo (`GtkOverlay` vs `GtkFixed`):** Se identificó que `GtkFixed` (donde residen los iconos) devuelve `NULL` en `gtk_widget_pick` sobre áreas vacías, impidiendo que controladores de eventos (`bg_context`, `bg_click`, `drag_marquee`, `drop_target`) recibieran clics o arrastres en el espacio libre. Se reubicaron dichos controladores en el widget contenedor raíz `GtkOverlay`, garantizando la captura fiable en cualquier coordenada del monitor.
  * **Menú Popover de Fondo de Pantalla:** Clic derecho sobre espacio vacío despliega el menú contextual estilo macOS (`desktop-popover-menu`) con: *Nueva carpeta*, *Usar Pilas*, conmutador de *Alineación automática*, *Alinear a la cuadrícula* y acceso a *Fondo de pantalla y Estilo...*. Se añadió soporte completo en pantallas secundarias.
  * **Menú Popover sobre Iconos Individuales:** Clic derecho sobre un archivo o pila activa el menú contextual vinculado a su `item_box` (*Abrir*, *Vista rápida*, *Mostrar en gestor de archivos*, *Renombrar...*, *Duplicar*, *Mover a la papelera*) con `gesture.set_state(Claimed)`, evitando colisiones con el menú de fondo.
  * **Estilizado CSS:** Clases `.desktop-popover-menu` y `.desktop-popover-menu > contents` con fondos transparentes y sin bordes residuales para integración acrílica perfecta con esquinas redondeadas.

* **Reordenamiento y Colocación Libre de Iconos (*Freeform Icon Dragging*):**
  * **Causa Raíz:** Se detectó que el controlador `GestureClick` en cada icono ejecutaba `gesture.set_state(Claimed)` de forma indiscriminada en la pulsación inicial (`pressed`). En GTK4, reclamar una secuencia en la pulsación inicial cancela inmediatamente cualquier otro controlador de arrastre (`GestureDrag`), imposibilitando mover los iconos. Asimismo, la presencia de `GtkDragSource` entraba en conflicto con los eventos locales al intentar iniciar un arrastre Wayland interproceso.
  * **Controlador `GestureDrag` Puro y Fluido:** Se desacopló `GestureClick` (reservando `Claimed` exclusivamente para el doble clic de apertura) y se confió el movimiento de iconos a un controlador `GestureDrag` dedicado y robusto en cada icono.
  * **Arrastre Multiselección:** Al iniciar el arrastre, se guardan las posiciones relativas de todos los iconos seleccionados, permitiendo mover un grupo completo de archivos juntos de manera reactiva e intuitiva.
  * **Persistencia Inmediata y Modo Libre:** Al soltar (`connect_drag_end`), si el desplazamiento supera 3px, se calcula la posición final (manteniendo coordenadas libres si `snap_to_grid: false` o imantando si está activado) y se persiste en `~/.config/malus-de/desktop-icons.json`.
  * **Picking Confiable:** En `is_point_over_icon` se incluyó `width_request` y `height_request` del widget para asegurar que la caja de 108x112px sea respetada por completo antes y después de los pases de diseño.

* **Separación de la Top-Bar en la Rejilla del Escritorio (`crates/wallpaper/src/main.rs`):**
  * **Margen Superior Ajustado (`DESKTOP_TOP_MARGIN = 56.0`):** Se incrementó la cota mínima vertical de 20px a 56px, garantizando 20px libres de separación visual debajo de los 36px de la top-bar del compositor tanto en la cuadrícula inicial como en el arrastre y la imantación.
  * **Ajuste y Clamping de Coordenadas Guardadas:** Al refrescar iconos o cargar posiciones existentes, cualquier icono con `y < 56.0` es reajustado automáticamente a la cota segura sin quedar oculto o pegado a la barra superior.

* **Soporte de Popovers Layer-Shell en el Compositor (`crates/compositor/src/backend/drm.rs`, `crates/compositor/src/backend/winit.rs`):**
  * **Pila de Renderizado (`upper_elements`):** Los popovers hijos de superficies `Layer::Background` y `Layer::Bottom` se promocionan a la capa de renderizado superior (`upper_elements`), asegurando que se dibujen por encima de ventanas activas, sombras y efectos de desenfoque.
  * **Detección de Puntero (`find_layer_popup_under`):** Inspección prioritaria de popups pertenecientes a capas layer shell antes del árbol de ventanas toplevel, configurando `is_popup = true` para enrutamiento exacto del cursor y botones.

---

## [2026-11-08] — Superficie de Escritorio Activa con Pilas macOS (Stacks), Quick Look y Gestión Fluida de Iconos

### Hitos Implementados

* **Pilas en el Escritorio (*macOS Desktop Stacks*) y Categorización Automática (`crates/wallpaper/src/main.rs`):**
  * **Categorización Reactiva (`StackCategory`):** Clasificación automática de los archivos de `~/Escritorio` en categorías semánticas: *Capturas de pantalla* (detección por prefijo y metadatos), *Imágenes*, *Documentos* (PDF, Office, Markdown, texto), *Vídeos*, *Música y Audio*, *Código y Scripts*, *Archivos Comprimidos*, *Carpetas* y *Otros*.
  * **Despliegue Interactivo:** Cuando las Pilas están activas (`use_stacks: true`), los archivos dispersos se agrupan en un contenedor estilizado con insignia numérica (`.desktop-stack-badge`). Un clic expande los archivos sobre la cuadrícula del escritorio; un segundo clic contrae la pila.
  * **Conmutador Rápido:** Opción *"Usar Pilas"* disponible tanto en el menú contextual del fondo del escritorio como en los Ajustes del Sistema.

* **Integración Nativa de Quick Look (`Espacio`) y Teclas de Acceso Rápido:**
  * **Soporte CLI en Branches (`crates/branches/src/main.rs`, `crates/branches/src/quick_look.rs`):** Nueva bandera `--quick-look <path>` (`-q`) para invocar la ventana modal de previsualización acrílica a gran escala directamente desde la línea de comandos o procesos externos, sin necesidad de abrir la ventana completa del gestor de archivos.
  * **Atajo Universal de Espacio:** Al seleccionar cualquier archivo o icono en el escritorio y presionar la barra espaciadora (`Espacio`), se ejecuta instantáneamente la vista previa de Quick Look.
  * **Atajos Adicionales:** `Return`/`Enter` para abrir el elemento con `xdg-open`, y `Delete`/`Backspace` para enviar a la papelera mediante la API de `gio::File::trash()`.

* **Menús Contextuales Enriquecidos Estilo macOS Tahoe:**
  * **Menú sobre Elementos/Iconos:** Opciones de *Abrir*, *Vista rápida (Espacio)*, *Mostrar en el gestor de archivos*, *Renombrar...* (diálogo integrado), *Duplicar* y *Mover a la papelera*.
  * **Menú sobre Fondo de Pantalla Vacío:** Opciones de *Nueva carpeta* (diálogo modal emergente), *Usar Pilas* (con indicador de estado activo `✓`), *Alinear a la cuadrícula* y acceso directo a *Fondo de pantalla y Estilo...*.

* **Imantación Inteligente (*Snap-to-Grid*) y Selección Elástica (*Rubberband*):**
  * **Cuadrícula Magnética (`snap_position_to_grid`):** Redondeo analítico y fluido a múltiplos de celda manteniendo la alineación predeterminada a la derecha estilo macOS (o configurable a la izquierda).
  * **Selección Elástica:** Arrastre rectangular sobre el lienzo con contorno acrílico translúcido (`.desktop-rubberband`), seleccionando interactivamente todos los iconos intersectados y permitiendo selección múltiple con `Ctrl`.
  * **Resaltado Visual:** Estilo `.desktop-icon.selected` con fondo azul acrílico translúcido y píldora de etiqueta contrastada fiel a macOS Finder.

* **Configuración Unificada y Monitorización Reactiva (`crates/common/src/config.rs`, `crates/settings/src/main.rs`):**
  * **Estructura `DesktopIconsConfig`:** Configuración persistente en `~/.config/malus-de/desktop.toml` con interruptor maestro `show_icons`, tamaño de icono `icon_size` (32px a 96px), alineación `align_left`, `snap_to_grid` y `use_stacks`.
  * **Panel en Ajustes del Sistema:** Nueva tarjeta interactiva en la sección *"Escritorio y Dock"* de `malus-de-settings` con conmutadores, selector de alineación y deslizador de tamaño de iconos con actualización en vivo.
  * **Monitoreo Cero CPU:** El demonio de fondo de pantalla vigila en tiempo real tanto `~/Escritorio` como `desktop.toml` mediante `gio::FileMonitor`.

* **Pruebas Unitarias Automatizadas:**
  * Tests en `crates/wallpaper` verificando resolución de directorios XDG, sujeción de coordenadas (*clamping*), imantación a cuadrícula macOS/izquierda, ordenación de columnas y clasificación exhaustiva de Pilas.

## [2026-11-08] — Exclusión Estricta de la Top-Bar: Prevención de Solapamiento en Redimensionado, Arrastre y Mapeo de Ventanas

### Hitos Implementados

* **Garantía Incondicional de Espacio Libre para la Top-Bar (`crates/compositor/src/state.rs`):**
  * **Zona Utilizable Respetada (`usable_zone_for_output`):** Corrección para asegurar que la coordenada superior `loc.y` de la zona utilizable nunca sea inferior a `output_geo.loc.y + 36`, protegiendo la barra superior incluso antes de que la capa layer shell termine de negociar o en casos de reinicio o recarga de la barra.
  * **Clamping Atómico de Ventanas (`ensure_window_below_topbar`):** Método centralizado que evalúa si una ventana (no en modo pantalla completa) tiene su borde superior invadiendo la zona de la top-bar. En caso afirmativo, traslada atómicamente la ventana hacia abajo.
  * **Compensación Automática de Decoraciones del Servidor (SSD vs CSD):** En ventanas que utilizan decoraciones del servidor (`window_uses_ssd`), la barra de título se dibuja en `win_loc.y - SSD_TITLEBAR_HEIGHT (38px)`. El cálculo exige una cota mínima `min_y = usable_zone.loc.y + 38` para SSD, y `min_y = usable_zone.loc.y` para CSD, asegurando que la barra de título y los botones de control (cerrar, minimizar, maximizar) nunca queden ocultos detrás de la barra superior.
  * **Validación en Commit de Superficies y Mapeo:** Comprobación automática en el commit de buffers XDG para reubicar ventanas si la aplicación intenta redimensionarse o reposicionarse internamente bajo la barra superior.
  * **Posición Inicial de Nuevas Ventanas (`new_toplevel`):** La coordenada `spawn_y` de nuevas ventanas ahora garantiza al menos `usable.loc.y + SSD_TITLEBAR_HEIGHT`.

* **Restricción en Grabs Interactivos de Puntero (`crates/compositor/src/grabs.rs`):**
  * **Redimensionado Superior (`ResizeSurfaceGrab::motion`):** Al redimensionar desde los bordes `Top`, `TopLeft` o `TopRight`, se calcula dinámicamente la cota superior permitida según el monitor de origen y el tipo de decoración (SSD/CSD). Se clampa `new_y = new_y.max(min_top)` y se ajusta la altura disponible manteniendo el borde inferior anclado.
  * **Arrastre y Movimiento de Ventana (`MoveSurfaceGrab::motion`):** Al mover ventanas mediante la barra de título o combinaciones de teclas, se clampa la coordenada vertical `new_location.y = new_location.y.max(min_top as f64)`, impidiendo que el usuario pueda empujar la ventana detrás de la barra superior.

* **Integración con Xwayland (`crates/compositor/src/backend/xwayland.rs`):**
  * Aplicación de `ensure_window_below_topbar` tanto en el mapeo inicial de ventanas X11 (`map_window`) como en las actualizaciones de posición solicitadas por clientes X11 (`update_window_location`).

* **Pruebas Automatizadas:**
  * Nuevo test unitario `usable_zone_respects_top_bar_height` en el compositor para verificar que `usable_zone_for_output` mantenga siempre una cota vertical mínima de 36px.

---

## [2026-11-08] — Rediseño del Centro de Control Estilo macOS Tahoe / iOS 18 Modular, Pantalla Completa Multimonitor y Corrección de Regresiones

### Hitos Implementados

* **Rediseño Completo del Centro de Control Estilo macOS Tahoe (`crates/top-bar/src/ui/control_center.rs`):**
  * **Estructura Modular Basada en Cápsulas e Islas Flotantes:** Adaptación fiel a las capturas de referencia ([`assets/screenshots/macOS-Tahoe-Control-Center-and-Menu-Bar.webp`](../assets/screenshots/macOS-Tahoe-Control-Center-and-Menu-Bar.webp) y [`assets/screenshots/control-center.jpg`](../assets/screenshots/control-center.jpg)).
  * **Módulo Superior de 2 Columnas Homogéneas:**
    * *Columna Izquierda:* Cápsula alargada de Wi-Fi con icono circular activo, nombre SSID dinámico y flecha de subvista, acompañada de dos botones circulares de 52px para **Bluetooth** (alternar encendido y clic derecho para subvista) y **AirDrop** (conmutar modo o abrir subvista).
    * *Columna Derecha (Widget Multimedia Now Playing):* Tarjeta vertical de música con carátula cuadrada de esquinas redondeadas, información de la pista y artista mediante D-Bus MPRIS y fila de controles de reproducción centrados (`⏮`, `⏯`, `⏭`).
  * **Fila de Focus (Concentración) y Modos:**
    * Cápsula de **Concentración / Focus** integrada con DND de GNOME/FreeDesktop (`org.gnome.desktop.notifications show-banners`).
    * Botón circular de **Stage Manager / Espacios**.
    * Botón circular de **Screen Mirroring / Duplicar Pantalla**, que navega directamente a la subvista de Disposición y Escalas de Pantallas (`displays`).
  * **Tarjetas Deslizadoras Continuas (Display y Sound):**
    * Sliders de pantalla y volumen rediseñados como cápsulas continuas gruesas estilo macOS Tahoe con iconos vectoriales en ambos extremos (`display-brightness-low/high` y `audio-volume-low/high`).
    * Botón circular de salida de audio (`sound_output_btn`) para gestionar dispositivos y fuentes de sonido.
  * **Fila de Accesos Rápidos Inferiores:**
    * Píldoras de **Modo Oscuro** y **Distribución de Teclado** (alternando en vivo entre `latam`, `es` y `us`).
    * Cuarteto de botones circulares rápidos de 44px: **Calculadora** (`gnome-calculator`), **Captura de Pantalla** (`gnome-screenshot`), **Spotlight** (`malus-de-spotlight`) y **Modo Noche** (Night Shift con temperatura de color cálida).
    * Botón translúcido centrado **"Editar Controles"** para abrir Ajustes del Sistema.
  * **Estilos Visuales Glassmorphic:** Reglas CSS avanzadas en [`style.css`](../crates/top-bar/src/styles/style.css) con soporte dual para tema claro y tema oscuro, transparencias equilibradas y bordes sutiles.

* **Respeto Estricto de la Pantalla de Origen en Pantalla Completa Multimonitor:**
  * Implementación de [`origin_output_for_window`](../crates/compositor/src/state.rs), calculando la intersección geométrica de mayor área física entre la geometría de la ventana y los monitores activos (con desempates por centroide, coordenadas de origen y puntero del ratón).
  * Solución definitiva al problema donde navegadores (Chromium, Firefox, Brave) o reproductores multimedia (MPV, VLC) pasaban un monitor por defecto o donde la ordenación interna de Smithay enviaba la ventana al monitor primario al activar pantalla completa.
  * Preservación exacta de la geometría y posición previa mediante `window_restore_geometries` para restaurar la ventana al monitor y dimensiones originales al salir de pantalla completa.
  * Soporte equivalente para clientes X11 vía Xwayland en [`backend/xwayland.rs`](../crates/compositor/src/backend/xwayland.rs).

* **Ocultación de Dock y Top Bar en Pantalla Completa según la Especificación `wlr-layer-shell`:**
  * Conforme a la especificación estándar Wayland `wlr-layer-shell`, las superficies con nivel de capa `Layer::Top` (como `malus-top-bar` y `malus-dock`) deben situarse por debajo de ventanas en pantalla completa, reservando solo `Layer::Overlay` por encima.
  * Implementación de [`has_fullscreen_window_on_output`](../crates/compositor/src/state.rs) para comprobar si existe alguna ventana activa en modo pantalla completa en la pantalla dada.
  * Supresión selectiva en [`drm.rs`](../crates/compositor/src/backend/drm.rs) y [`winit.rs`](../crates/compositor/src/backend/winit.rs) de los elementos gráficos de `Layer::Top` en `upper_elements`, recorte de esquinas del dock (`dock_clip_roots`), popups asociados y pasadas de desenfoque GPU (`layer_blur_items`) en monitores con una ventana fullscreen.
  * Filtrado de eventos del puntero e hit-testing en [`find_layer_under`](../crates/compositor/src/backend/drm.rs) y [`update_ssd_hover`](../crates/compositor/src/state.rs) para evitar capturas o clics fantasma sobre la barra o el dock ocultos tras el vídeo.
  * Mantenimiento de independencia multimonitor: un monitor secundario continúa mostrando su barra superior y dock de forma interactiva aunque el monitor principal tenga un vídeo o juego a pantalla completa (o viceversa).

* **Ajuste del Snap Preview de Ventanas:**
  * Renderizado del rectángulo de previsualización por debajo de la ventana activa en movimiento para mantener visibilidad constante del contenido durante el arrastre.
  * Supresión del filtro de desenfoque de fondo en el snap preview a favor de un tinte semitransparente con esquinas redondeadas coincidentes con el radio squircle de la ventana.

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
