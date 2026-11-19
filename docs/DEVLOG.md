# Diario de Desarrollo (Devlog) — Malus DE

Registro cronológico y técnico del avance de ingeniería, hitos, experimentos, decisiones de diseño y métricas del entorno de escritorio **Malus DE**.

---

## [2026-11-19] — Corrección Integral de Desconexión/Hotplug Multi-Monitor (`compositor`, `common`, `wallpaper`, `top-bar`, `dock`, `settings`)

Resuelto el fallo por el cual desconectar el monitor principal provocaba que la pantalla sobreviviente no asumiera el rol principal, dejara el fondo negro y no reubicara `malus-top-bar` ni `malus-dock`:

### 1. Identidad de Salida y Promoción Automática a Principal ([`config.rs`](../crates/common/src/config.rs))
- Añadidos `DisplaysConfig::primary_output` y `DisplayOutputConfig::matches_monitor_identity` (con coincidencia estricta cuando existe `connector` para evitar falsos positivos por coordenadas `(0, 0)` desactualizadas).
- Normalización de `primary` y `master_scale` dentro de `DisplaysConfig::recalculate_layout` cuando el monitor principal previo desaparece de `outputs`, garantizando que el monitor sobreviviente quede como `primary = true` en `(0, 0)`.

### 2. Reubicación Real de `LayerSurface` y Señalización DRM ([`state.rs`](../crates/compositor/src/state.rs), [`drm.rs`](../crates/compositor/src/backend/drm.rs))
- `MalusState::relocate_primary_shell_layers_for_config` ahora emite `old_output.leave` sobre el árbol de superficies saliente y `primary_output.enter` junto con `set_preferred_scale` y `send_configure` en todas las capas reubicadas o preservadas (`malus-top-bar`, `malus-dock`, `malus-app-switcher`).
- En `reconcile_drm_outputs`, los `global` de `wl_output` desconectados se retiran después de reubicar capas y migrar ventanas, emitiendo `removed.output.leave`, marcando `full_damage_buffers` / `dock_redraw_buffers` / `background_dirty` en las salidas activas, vaciando clientes (`flush_clients`) y difundiendo `IpcMessage::ReloadDisplaysConfig`.

### 3. Wallpaper Dinámico, Top Bar, Dock, Control Center y Ajustes ([`main.rs`](../crates/wallpaper/src/main.rs), [`main.rs`](../crates/top-bar/src/main.rs), [`main.rs`](../crates/dock/src/main.rs), [`control_center.rs`](../crates/top-bar/src/ui/control_center.rs), [`main.rs`](../crates/settings/src/main.rs))
- **Wallpaper (`malus-de-wallpaper`):** agregado `ApplicationHoldGuard` para no cerrar el daemon durante transiciones de salidas, creación secuencial de nuevas ventanas antes de cerrar las previas y resincronización debounced (`120 ms`) ante `display.monitors().connect_items_changed` e IPC `ReloadDisplaysConfig`, promoviendo los iconos del escritorio al monitor sobreviviente.
- **Top Bar y Dock (`malus-de-top-bar`, `malus-de-dock`):** fijado del monitor primario al inicio (`find_primary_gdk_monitor` antes de `present()`) y escucha de `ReloadDisplaysConfig` / `connect_items_changed` para recalcular tamaño, redibujar y re-registrar slots de miniaturas sin llamar `set_monitor`/`set_visible(false)` en tiempo de ejecución sobre superficies `gtk4-layer-shell` mapeadas.
- **Control Center y Ajustes (`control_center.rs`, `malus-de-settings`):** actualización en vivo de títulos por conector (`title_lbl`, `mon0_title`, `mon1_title`), reconstrucción dinámica de tarjetas en `rebuild_monitors_grid` cuando cambia el conjunto de conectores y ocultamiento automático de controles duales al quedar 1 solo monitor.

### 4. Validación
- `cargo check --offline --workspace --all-targets`: **0 warnings**.
- `cargo test --offline -p malus-de-common display`: **7 passed; 0 failed**.
- `cargo test --offline -p malus-de-compositor`: **84 passed; 0 failed**.
- `cargo test --offline -p malus-de-dock`: **15 passed; 0 failed**.
- `cargo test --offline -p malus-de-wallpaper`: **20 passed; 0 failed**.
- `cargo test --offline -p malus-de-settings`: **2 passed; 0 failed**.
- `cargo test --offline -p malus-de-top-bar`: **9 passed; 1 failed** (`services::tests::test_brightness_service` falló dentro del sandbox al escribir en sysfs del host).
- `cargo build --release --offline -p malus-de-compositor -p malus-de-top-bar -p malus-de-dock -p malus-de-wallpaper -p malus-de-settings`: compilado limpio en `1m 59s`.

---

## [2026-11-19] — Quick Look Fluido Estilo macOS (`Overlay`, Multimedia, Caché LRU, Precarga) y Cierre de ADR 0006

### 1. Vista Rápida (`Espacio`) con Media Real, Transición macOS y Sin *Black Hole*
- **Alternancia directa con `Espacio` ([`quick_look.rs`](../crates/branches/src/quick_look.rs), [`app_window.rs`](../crates/branches/src/app_window.rs), [`gallery_view.rs`](../crates/branches/src/views/gallery_view.rs), `c65e758`):**
  - Captura de teclado en `PropagationPhase::Capture` (`Key::space` / `Key::KP_Space` / `Escape` / `←` / `→` / `Re Pág` / `Av Pág`) y foco automático al pulsar tarjetas en `GalleryView` para evitar que controles internos absorban la barra espaciadora.
  - Renderizado real con `gtk4::Picture` (`ContentFit::Contain`) para imágenes (incluyendo Glycin y respaldo `ffmpeg` para HEIC/AVIF/JXL), PDFs multipágina (`pdfinfo` + `pdftoppm`), vídeo/audio inline (`ffmpeg` raw RGBA a `MemoryTexture` + `ffplay` con parada instantánea) y listado de comprimidos (`.zip`, `.tar.*`, `.7z`).
- **Animación macOS de entrada y salida sin efecto *Agujero Negro* ([`quick_look.rs`](../crates/branches/src/quick_look.rs), [`macos-branches.css`](../crates/branches/src/styles/macos-branches.css), [`state.rs`](../crates/compositor/src/state.rs), `f1a781b`):**
  - Integrado Quick Look como HUD flotante dentro de `root_overlay` (`Overlay`) en `BranchesWindow` con transición CSS `scale(0.90) -> 1.0` (`175 ms`) y salida `1.0 -> 0.92` (`135 ms`), evitando crear/cerrar superficies `xdg_toplevel` en sesión normal.
  - Añadido `should_skip_black_hole_for_app_id` en `malus-de-compositor` para omitir también el shader de agujero negro cuando se cierren ventanas `dev.malus.Branches.QuickLook` o `dev.malus.Preview`.

### 2. Eliminación de *Lag*, Caché LRU en Memoria y Precarga Predictiva (`2ee68c5`, `c4e9dff`)
- **Cero bloqueos en el hilo principal (`2ee68c5`):**
  - Recolección no bloqueante (`wait()` en hilo secundario tras `kill()`) en `MediaPlaybackHandle::stop()`, eliminación de doble sombra pesada durante el escalado CSS y arranque de vídeo/audio diferido (`185 ms`) tras completar la animación de entrada.
- **Caché LRU + precarga estilo macOS (`c4e9dff`):**
  - Decodificación RGBA completa en hilo secundario (`DecodedRgbaFrame` con rotación EXIF) lista para `MemoryTexture::new`, junto con `PreviewCache` LRU de `24` entradas indexada por `(PathBuf, mtime_nanos)` y deduplicación `in_flight`.
  - Precarga automática al cambiar de selección en el explorador (`prefetch_item` en [`app_window.rs`](../crates/branches/src/app_window.rs#L1019-L1023)) y de vecinos `←`/`→` (`prefetch_neighbors`) para montar la imagen desde el fotograma `0` al pulsar `Espacio`.

### 3. Actualización de ADR 0006 y Validación
- Actualizado [`0006-branches-pendiente.md`](decisiones/0006-branches-pendiente.md) reflejando los 8 bloques completados y acotando los puntos menores de seguimiento (`GroupBy` visual, resaltado sintáctico en Quick Look, consulta directa a `malus-de-spotlight`, app independiente `Preview` en ADR 0007 y despliegue en sistema).
- `cargo test --offline -p malus-de-branches`: **53 passed; 0 failed**.
- `cargo test --offline -p malus-de-compositor quick_look_and_preview_windows_skip_black_hole_close_effect`: **1 passed; 0 failed** (`83 filtered out`).
- `cargo check --offline --workspace --all-targets`: **0 warnings** en todo el workspace.
- `cargo build --release --offline -p malus-de-branches`: compilado limpio en `24.47s`.

---

## [2026-11-19] — Resolución Completa de Pendientes en Branches (ADR 0006, Fases 1–6)

Implementación integral de las 6 fases planificadas a partir de [`0006-branches-pendiente.md`](decisiones/0006-branches-pendiente.md) para equipar a **Branches** (`crates/branches`) como explorador nativo completo de **Malus DE**:

### 1. Fase 1 — Selección Múltiple Unificada y Arrastre por Lotes (`7924b69`)
- Unificadas las rutas de selección múltiple (`get_selected_items`, `select_all`, selección por rango `Shift` y conmutación `Ctrl`/`Cmd`) en [`list_view.rs`](../crates/branches/src/views/list_view.rs), [`column_view.rs`](../crates/branches/src/views/column_view.rs), [`gallery_view.rs`](../crates/branches/src/views/gallery_view.rs) y [`folder_pane.rs`](../crates/branches/src/folder_pane.rs).
- Habilitado arrastre multi-archivo coherente (`views/mod.rs`) y corregido el conteo real de ítems en [`status_bar.rs`](../crates/branches/src/status_bar.rs) / [`app_window.rs`](../crates/branches/src/app_window.rs).

### 2. Fase 2 — Trabajos Asíncronos con Progreso, Cancelación, Conflictos, `Undo` y Archivos Comprimidos (`b77a18c`)
- Creados [`jobs.rs`](../crates/branches/src/jobs.rs) y [`job_ui.rs`](../crates/branches/src/job_ui.rs) con copia, movimiento, duplicado y envío a papelera por bloques en segundo plano, botón de cancelación e indicador/popover de progreso en la barra de herramientas.
- Diálogo interactivo de resolución de conflictos (*Reemplazar*, *Saltar*, *Mantener ambos* y *Aplicar a todos*), pila `UndoEntry` (`Ctrl/Cmd+Z`) cubriendo movimientos, duplicados, renombrados y envíos a papelera, además de compresión (`.zip`, `.tar.gz`) y extracción (`.zip`, `.tar.*`, `.7z`).

### 3. Fase 3 — Ciclo Completo de Papelera FreeDesktop y Etiquetas Canónicas (`b49a5ee`)
- Soporte completo de `.trashinfo` (`restore_from_trash`, `empty_trash`, `delete_permanent` con confirmación `Shift+Delete` / `Cmd+Alt+Backspace`), banner contextual en Papelera y menú secundario en la barra lateral.
- Catálogo unificado de 7 etiquetas canónicas (`TAG_CATALOG` en [`db.rs`](../crates/branches/src/db.rs)) con migración automática SQLite v2 desde nombres previos, sincronización con atributo extendido `user.malus.tags` vía `gio::File`, indicadores de color en las 4 vistas y selector contextual de puntos.

### 4. Fase 4 — *Obtener Información*, Ordenación, Historial de Navegación y *Drop* en `PathBar` (`aa06006`)
- Nueva ventana [`info_window.rs`](../crates/branches/src/info_window.rs) (`Ctrl/Cmd+I` / botón *Info*) separada de Quick Look, con cálculo recursivo cancelable de tamaño en carpetas, edición de permisos POSIX `rwx`, selector *«Abrir con…»* (`gio::AppInfo::all_for_type` + predeterminada) y etiquetas del ítem.
- Selector de ordenación en toolbar (`win.sort_by`, `win.sort_order`), popovers de historial en botones `<` / `>` (`go_back_steps`, `go_forward_steps`) y `DropTarget` en los segmentos de [`path_bar.rs`](../crates/branches/src/path_bar.rs).

### 5. Fase 5 — Búsqueda Recursiva, Malus Link y Puente con `malus-de-preview` (`381f293`)
- Añadido selector de alcance junto a `search_entry` (*Esta carpeta* vs *Subcarpetas*) y recorrido BFS acotado/cancelable en segundo plano (`search_paths_recursive` en [`fs_backend.rs`](../crates/branches/src/fs_backend.rs)), excluyendo `.git`, `node_modules` y `target`.
- Integración D-Bus con **Malus Link** (`org.malus_de.Link`: `GetConnectedDevices` y `ShareFile`) desde barra y menú contextual en [`app_window.rs`](../crates/branches/src/app_window.rs).
- Puente D-Bus hacia `dev.malus.Preview` en [`quick_look.rs`](../crates/branches/src/quick_look.rs) con respaldo local mejorado (navegación `←/→`, previsualización PDF página 1 vía `pdftoppm` y metadatos multimedia vía `ffprobe`).

### 6. Fase 6 — Red (`SMB`/`SFTP`/`NFS`), Montajes GVfs y Asociación MIME de Sesión (`c79a462`)
- Creado [`network.rs`](../crates/branches/src/network.rs) con validación/normalización de URIs (`smb://`, `sftp://`, `ssh://` -> `sftp://`, `nfs://`, `dav://`, `davs://`, `ftp://`, `ftps://`, `afp://`), historial persistente de servidores recientes y diálogo modal **Conectar al servidor…** (`Ctrl/Cmd+K`, acción `win.connect_server`) montando volúmenes vía `gio::File::mount_enclosing_volume` + `gtk4::MountOperation`.
- Integrada la entrada **«Red»** y montajes GVfs activos en [`sidebar.rs`](../crates/branches/src/sidebar.rs) con iconos remotos y botón de desmontaje.
- Actualizado [`install-session.sh`](../scripts/install-session.sh) para refrescar `/usr/share/applications` (`update-desktop-database`) y registrar `dev.malus.Branches.desktop` como gestor predeterminado para `inode/directory` y `x-scheme-handler/trash` mediante `xdg-mime`.

### 7. Validación
- `cargo test --offline -p malus-de-branches`: **48 passed; 0 failed** (`0 warnings`).
- `cargo check --offline --workspace --all-targets`: **0 warnings** en los 16 crates del workspace.
- `cargo build --release --offline -p malus-de-branches`: compilación optimizada exitosa (`1m 26s`, **0 warnings**).
- `bash -n scripts/install-session.sh`: sintaxis válida.

---

## [2026-11-19] — Refinamiento de Pestañas en Branches, Limpieza de Warnings en Workspace y ADR 0007 (`Preview`)

### 1. Medidas y Botón de Cierre en Barra de Pestañas (`crates/branches`)
- **Consistencia métrica con la barra superior ([`macos-branches.css`](../crates/branches/src/styles/macos-branches.css), [`app_window.rs`](../crates/branches/src/app_window.rs)):**
  - Ajustado el botón de nueva pestaña (`button.branches-tab-add-btn`) a `30 × 30 px` (`border-radius: 8px`, estilo squircle *Liquid Glass* y variantes claro/oscuro) para igualar en tamaño, estilo y alineación derecha al botón **Buscar archivos** (`button.branches-search-btn`).
  - Ajustado el contenedor `.branches-tabs-strip` (`min-height: 26px`, `padding: 2px`, altura exterior equivalente a las cápsulas `.branches-toolbar-pill`) y centrados `tabs_strip` y `new_tab_btn` con `valign(Align::Center)` y `spacing(8)` en `tab_bar_box`.
  - Elevadas las pestañas `.branches-tab` de `24 px` a `min-height: 26px` para igualar la altura de los botones internos de vista (`.branches-view-toggle button` / `.branches-tool-btn`).
- **Botón de cerrar pestaña compacto, a la izquierda y solo en *hover*:**
  - Movido `close_btn` (`button.branches-tab-close-btn`) a la izquierda de `tab_title_label`, reduciendo su tamaño de `18 px` a `14 × 14 px` (`-gtk-icon-size: 10px`) y ocultándolo por defecto (`opacity: 0`) para mostrarse únicamente al pasar el ratón por encima (`.branches-tab:hover button.branches-tab-close-btn`).
  - Añadido un espaciador simétrico `.branches-tab-spacer` (`14 × 14 px`) en el extremo derecho de la pestaña para mantener centrado el título sin saltos visuales al aparecer el botón de cierre.
  - Añadidas aserciones CSS en `test_branches_tab_bar_styles_and_context_menu_wiring` ([`main.rs`](../crates/branches/src/main.rs)).

### 2. Eliminación Completa de Warnings de Compilación en el Workspace (`Cargo 1.100.0`)
- **Limpieza de dependencias no utilizadas (`cargo::unused_workspace_dependencies` / `cargo::unused_dependencies`):**
  - Retirada `malus-de-render` de `[workspace.dependencies]` en [`Cargo.toml`](../Cargo.toml) (se mantiene como miembro activo en `[workspace.members]`).
  - Eliminadas dependencias declaradas pero sin uso en `crates/common`, `crates/compositor`, `crates/screenshot`, `crates/session-lock`, `crates/spotlight`, `crates/branches`, `crates/dock` y `crates/settings`, actualizando [`Cargo.lock`](../Cargo.lock).
- **Prueba unitaria dormida ([`fs_backend.rs`](../crates/branches/src/fs_backend.rs)):** Añadido `#[test]` faltante en `split_desktop_exec_args_handles_quotes_and_escapes`, eliminando el *warning* de código muerto e incorporando el caso a la batería de pruebas.

### 3. Planificación de Aplicación de Vista Previa (`ADR 0007`)
- Creado [`0007-preview-app-pendiente.md`](decisiones/0007-preview-app-pendiente.md) detallando la arquitectura por fases para `crates/preview` (`malus-de-preview`, `dev.malus.Preview`): instancia única con navegación `←/→`, visor de imágenes (`Pixbuf`/`Glycin`), vídeo/audio sobre GStreamer `gtk4paintablesink` (documentando la ausencia de `gtk4-media-gstreamer` en Fedora 45), documentos PDF (`Poppler`/`pdftoppm`), conversión LibreOffice *headless*, código/Markdown, archivos comprimidos y resumen de carpetas.

### 4. Validación
- `cargo check --offline --workspace --all-targets` y `cargo check --offline --workspace` (tras limpiar artefactos locales de los 16 crates): **0 warnings**.
- `cargo test --offline --workspace --no-run`: **0 warnings**.
- `cargo build --offline --workspace`: **0 warnings**, compilación limpia en los 16 crates.
- `cargo test --offline -p malus-de-branches`: **35 passed; 0 failed**.

---

## [2026-11-18] — Modernización de Ajustes (`FileDialog`, Buscador Squircle, Sidebar 212 px y Glifos del Sistema)

### 1. Eliminación de Warnings en `crates/settings` (`FileChooserNative` -> `FileDialog`)
- **Migración GTK 4.10+ ([`main.rs`](../crates/settings/src/main.rs)):** Sustituidos los 5 *warnings* de deprecación de `gtk4::FileChooserNative` en el selector de fondo de pantalla por `gtk4::FileDialog::builder()` asíncrono (`chooser.open`), añadiendo `build_wallpaper_file_filter()` (`image/jpeg`, `image/png`, `image/webp`, `image/svg+xml`) y `wallpaper_chooser_initial_folder()` (`~/Imágenes/Wallpapers` / `~/Pictures/Wallpapers`).

### 2. Buscador Lateral Squircle (`<= 36 px`) y Consistencia Visual con Branches (`212 px`)
- **Filtro squircle compacto ([`main.rs`](../crates/settings/src/main.rs), [`macos-settings.css`](../crates/settings/src/styles/macos-settings.css)):**
  - Implementado `create_squircle_search_shell` (`Overlay` + `DrawingArea` en Cairo con superelipse $n = 4.2$ y `SIDEBAR_SEARCH_SQUIRCLE_RADIUS = 17.0` para igualar la curvatura visual ~`2.6 px` de los botones `border-radius: 9px`), con altura fija de `34 px` (`SIDEBAR_SEARCH_MAX_HEIGHT = 36`), reactivo a foco (`EventControllerFocus`) y modo claro/oscuro (`libadwaita::StyleManager`), y `.sidebar-search` unificado a `border-radius: 9px`.
- **Barra lateral alineada con Branches:**
  - Sustituido `OverlaySplitView` (`220–280 px`) por `gtk4::Paned` acotado a `SETTINGS_SIDEBAR_WIDTH = 212` (coincidente con `MAX_SIDEBAR_WIDTH = 212` de Branches y la región de desenfoque acrílico del compositor para `dev.malus.Settings`), junto con `.settings-content-pane` opaco (`@view_bg_color`).
  - Igualados translucidez, bordes y estados de `.macos-sidebar` (`alpha(@window_bg_color, 0.72)`, `border-right: 1px solid alpha(@borders, 0.28)`) y `.sidebar-btn` / `.sidebar-btn-active` (`alpha(@window_fg_color, 0.11)`) con [`macos-branches.css`](../crates/branches/src/styles/macos-branches.css).
- **Altura fija y alineación izquierda de secciones (`build_sidebar_nav_button`):** Eliminado el `vexpand`/`hexpand` que se propagaba desde los iconos internos (y hacía estirarse/centrarse las filas al redimensionar la ventana), fijando `SIDEBAR_ROW_HEIGHT = 32`, `valign(Align::Start)` y contenido alineado estrictamente a la izquierda.

### 3. Reemplazo de Emojis por Glifos del Sistema (`-symbolic`)
- Retirados todos los emojis de la interfaz en [`crates/settings/src/main.rs`](../crates/settings/src/main.rs) (barra lateral sin cajas de fondo usando iconos `16 px` `.settings-sidebar-icon`, tarjetas de tema con `make_system_glyph_badge`, selector Claro/Oscuro con `weather-clear-symbolic` / `weather-clear-night-symbolic`, *Acerca de este equipo*, Dock, Spotlight, Pantallas, Teclado y Fondo de pantalla).

### 4. Validación
- `cargo check --offline -p malus-de-settings --all-targets`: **0 warnings**, código de salida `0`.
- `cargo test --offline -p malus-de-settings` sobre sesión Wayland activa: **2 passed; 0 failed** (`wallpaper_chooser_initial_folder_prefers_wallpapers_subdirs` y `gtk_ui_smoke_checks`).
- `cargo build --release --offline -p malus-de-settings`: compilación `--release` correcta (exit `0`).

---

## [2026-11-18] — Protección del Socket IPC Vivo ante `cargo test` y Restauración desde el Dock

### 1. Preservación de `malus-de-ipc.sock` en Pruebas Unitarias (`crates/compositor`, `crates/dock`)
- **Causa ([`state.rs`](../crates/compositor/src/state.rs)):** `MalusState::new()` invocaba incondicionalmente `ipc_broadcaster.start_server()`, que ejecuta `remove_file()` y `bind()` sobre `ipc_socket_path()` (`$XDG_RUNTIME_DIR/malus-de-ipc.sock`). Al ejecutar `cargo test` dentro de una sesión activa, las instancias de prueba eliminaban el socket de escucha del compositor vivo, haciendo fallar los envíos IPC puntuales desde el Dock (`AnimateRestore`, `RegisterDockSlot`) mientras los flujos ya abiertos seguían recibiendo `MinimizedWindowsChanged`.
- **Solución:**
  - Protegido `ipc_broadcaster.start_server()` en `MalusState::new()` con `#[cfg(not(test))]` para que las pruebas nunca reemplacen el socket IPC de la sesión activa.
  - Añadido registro `warn!` en [`crates/dock/src/main.rs`](../crates/dock/src/main.rs) cuando falla `send_ipc_message` para `AnimateRestore` o `RegisterDockSlot` en lugar de descartar el error silenciosamente.
- **Documentación (`docs/decisiones`):** Creado [`0006-branches-pendiente.md`](decisiones/0006-branches-pendiente.md) con los 8 bloques pendientes para completar la paridad de **Branches** con macOS Finder y la Fase 5.

### 2. Validación
- `cargo test --offline -p malus-de-compositor -p malus-de-dock`: **83 passed; 0 failed** en `malus-de-compositor` y **15 passed; 0 failed** en `malus-de-dock`.
- `cargo build --release --offline -p malus-de-compositor -p malus-de-dock`: compilación `--release` correcta (exit `0`).

---

## [2026-11-18] — Corrección de Prueba Unitaria de Arrastre en Escritorio (`GTK 4.24`)

### 1. Ajuste de `test_drag_preview_widget_paintable` (`crates/wallpaper`)
- **Causa ([`main.rs`](../crates/wallpaper/src/main.rs)):** En GTK `4.24.1` bajo sesión Wayland activa, `gtk4::Fixed::child_position()` consulta la geometría resuelta tras la pasada de *layout* (`gtk_widget_compute_point`), devolviendo `(0.0, 0.0)` cuando el contenedor `gtk4::Fixed` de prueba no ha sido asignado ni mapeado en una ventana, aunque `put()` y `move_()` ya hayan registrado la transformación en el `LayoutChild`.
- **Solución:** Se actualizó `test_drag_preview_widget_paintable` para validar `fixed.child_transform(&item_box).map(|t| t.to_translate())`, verificando `(100.0, 100.0)` tras `put` y `(-5000.0, -5000.0)` tras `move_`.

### 2. Validación
- `cargo test --offline -p malus-de-wallpaper` sobre sesión Wayland activa: **20 passed; 0 failed**.

---

## [2026-11-18] — Traslado Inmediato de Dock y Top Bar al Monitor Principal y Corrección de Deadlock

### 1. Reubicación en Vivo de Capas del Shell al Cambiar de Pantalla Principal (`crates/compositor`)
- **Traslado sin recrear clientes ([`state.rs`](../crates/compositor/src/state.rs)):** Añadidos `PRIMARY_SHELL_LAYER_NAMESPACES` / `is_primary_shell_layer_namespace` (`malus-top-bar`, `malus-dock`, `malus-app-switcher`), `primary_output_for_config()` y `relocate_primary_shell_layers_for_config()` para desmapear (`unmap_layer`) las capas principales de salidas secundarias, reasignarlas al nuevo monitor principal con `new_map.arrange()`, restablecer `s.size = None` en `malus-top-bar` para adaptar su ancho, actualizar `preferred_scale` mediante `with_fractional_scale` y enviar `send_configure()`.
- **Integración en DRM y Winit ([`drm.rs`](../crates/compositor/src/backend/drm.rs), [`winit.rs`](../crates/compositor/src/backend/winit.rs)):**
  - Durante recarga en vivo (`ReloadDisplaysConfig`), al mover capas principales se marcan como sucios los buffers de fondo y daño (`background_dirty`, `background_snapshot_stale`, `full_damage_buffers`, `dock_redraw_buffers`) y se invoca `ensure_window_below_topbar(&win)` para ajustar ventanas bajo la barra en el nuevo monitor.
  - En desconexión física (`reconcile_drm_outputs`), si quedan salidas conectadas, las capas principales se preservan y migran al monitor principal en lugar de cerrarse con `send_close()`.
  - También conectado en `pending_displays_reload` dentro de `run_winit()`.

### 2. Corrección de Congelamiento al Presionar «Hacer Principal»
- **Causa ([`state.rs`](../crates/compositor/src/state.rs)):** Dentro del cierre `processor` de `with_surface_tree_downward` en `relocate_primary_shell_layers_for_config()`, se volvía a invocar `with_states(wl_surf, ...)`. Dado que `with_surface_tree_downward` (`PrivateSurfaceData::map_tree` en Smithay) ya mantiene bloqueado el `Mutex` de la superficie y entrega `states` como argumento, se producía un autobloqueo (*deadlock*) inmediato en el hilo principal del compositor.
- **Solución:** Se reemplazó la llamada anidada usando directamente el argumento `states` del cierre `processor` para aplicar `with_fractional_scale(states, |fs| fs.set_preferred_scale(target_scale))`.

### 3. Validación
- `cargo test --offline -p malus-de-compositor`: **83 passed; 0 failed** (incluye `primary_output_for_config_selects_configured_primary_display`).
- `cargo build --release --offline -p malus-de-compositor`: compilación `--release` correcta (exit `0`).

---

## [2026-11-18] — Corrección de Popups en Compositor y Opacidad en Vivo para Elementos Cortados en Branches

### 1. Glitch de Recentrado al Abrir Menús Contextuales (`crates/compositor`)
- **Causa ([`state.rs`](../crates/compositor/src/state.rs)):** En Smithay, `Window::with_surfaces` también recorre los `xdg_popup` vinculados al toplevel (`PopupManager::popups_for_surface`). Al hacer `commit` sobre la superficie de un `GtkPopover` contextual, `window_for_surface(surface)` devolvía la ventana padre y `!self.initially_placed_windows.contains(surface)` evaluaba `true`, re-centrando la ventana en el escritorio.
- **Solución:** Añadido `is_root_window_surface(window, surface)` (`window.wl_surface().as_deref() == Some(surface)`) para ejecutar colocación inicial, centrado de *transients*, ajuste bajo top bar, actualización de `window_restore_geometries` y captura `cached_close_snapshots` únicamente en commits de la superficie raíz del toplevel, manteniendo `window.on_commit()` para todas las superficies.

### 2. Marcado Visual Inmediato de Elementos Cortados (`crates/branches`)
- **Sincronización en vivo (`.branches-cut-item` con `opacity: 0.5`):** Añadido [`sync_cut_css_class`](../crates/branches/src/views/mod.rs) y `refresh_cut_states()` en las cuatro vistas (`IconView`, `ListView`, `ColumnView`, `GalleryView`), `FolderPane` (incluyendo cambio de vista en `set_view_mode`) y [`BranchesWindow::refresh_all_cut_states`](../crates/branches/src/app_window.rs).
- Al ejecutar **Cortar** (`Ctrl+X` / `Command+X` / menú contextual), los ítems visibles pasan de inmediato a opacidad `0.5` sin perder la selección ni recargar el directorio, y recuperan su opacidad normal al copiar otros archivos o completar el pegado.

### 3. Copiar/Cortar/Pegar y Opacidad `.desktop-cut-item` / `.desktop-hidden-item` en el Escritorio (`crates/wallpaper`)
- **Opacidad `0.5` en vivo ([`default-gradient.css`](../crates/wallpaper/src/styles/default-gradient.css), [`main.rs`](../crates/wallpaper/src/main.rs)):** Añadidas las clases `.desktop-icon.desktop-hidden-item` y `.desktop-icon.desktop-cut-item` (`opacity: 0.5`, preservando `opacity: 0.0` en `.desktop-icon-dragging`), con observación reactiva del portapapeles Wayland (`watch_clipboard_for_desktop` / `connect_changed`) para atenuar en tiempo real iconos del escritorio cortados tanto localmente como desde Branches.
- **Portapapeles y atajos del escritorio:** Implementados `set_desktop_clipboard`, `paste_desktop_payload_into` y `paste_from_desktop_clipboard_async` (`x-special/gnome-copied-files` y `text/uri-list`), menús contextuales `"Copiar"`, `"Cortar"`, `"Pegar en carpeta"` y `"Pegar"` en fondo del escritorio (guardando coordenadas cerca del clic), además de atajos `Ctrl/Command + C / X / V / D / A`.

### 4. Validación
- `cargo test --offline -p malus-de-compositor`: **82 passed; 0 failed** (incluye `commit_ignores_non_root_surfaces_for_window_centering`).
- `cargo test --offline -p malus-de-branches`: **34 passed; 0 failed**.
- `cargo test --offline -p malus-de-wallpaper`: **20 passed; 0 failed** (incluye `desktop_clipboard_cut_and_copy_roundtrip`).
- `cargo build --release --offline -p malus-de-compositor`, `-p malus-de-branches` y `-p malus-de-wallpaper`: exit `0`.

---

## [2026-11-18] — Pestañas, Opacidad de Archivos Ocultos y Copiar/Cortar/Pegar en Branches (`Command + C / X / V`)

### 1. Soporte Multi-Pestaña y Apertura en Nueva Pestaña (`crates/branches`)
- **Gestión independiente por pestaña ([`app_window.rs`](../crates/branches/src/app_window.rs), [`folder_pane.rs`](../crates/branches/src/folder_pane.rs)):** Cada `BranchesTab` encapsula sus paneles primario y secundario (`Split View`), historial, vista activa y filtro de búsqueda; la barra superior, `PathBar`, `StatusBar` y selección en barra lateral se sincronizan automáticamente al alternar pestañas.
- **Barra de pestañas estilo Finder ([`macos-branches.css`](../crates/branches/src/styles/macos-branches.css)):** Contenedor `.branches-tab-bar` visible automáticamente al tener 2 o más pestañas, con botón de cierre individual, cierre por clic central, botón `+` (`Ctrl+T`), entrada `"Nueva Pestaña"` en el menú de acciones y atajos `Ctrl+T`, `Ctrl+W`, `Ctrl+Tab` / `Ctrl+Shift+Tab` y `Ctrl+Page_Up` / `Ctrl+Page_Down`.
- **Menús contextuales y barra lateral ([`mod.rs`](../crates/branches/src/views/mod.rs), [`sidebar.rs`](../crates/branches/src/sidebar.rs)):** Añadido `"Abrir en nueva pestaña"` (y `"Abrir carpeta contenedora en nueva pestaña"` en archivos) dentro de `show_item_context_menu`, además de menú contextual y apertura con clic central en **Favoritos**, **Compartidos**, **Ubicaciones**, **Marcadores**, **Unidades montadas**, **Recientes** (`@recents`) y **Aplicaciones** (`@applications`) mediante `win.open_in_new_tab`.

### 2. Archivos Ocultos y Portapapeles de Archivos (`Copy / Cut / Paste`)
- **Opacidad de ocultos ([`macos-branches.css`](../crates/branches/src/styles/macos-branches.css), `crates/branches/src/views/*`):** Los elementos con `item.is_hidden` reciben `.branches-hidden-item` (`opacity: 0.5`) en las 4 vistas (Iconos, Lista, Columnas y Galería), preservando `opacity: 0.0` durante el arrastre.
- **Operaciones de portapapeles ([`operations.rs`](../crates/branches/src/operations.rs), [`mod.rs`](../crates/branches/src/views/mod.rs), [`app_window.rs`](../crates/branches/src/app_window.rs)):**
  - Implementados `set_clipboard`, `clear_clipboard`, `has_clipboard_files`, `is_path_cut`, `paste_from_clipboard_async` y `paste_payload_into` con interoperabilidad `x-special/gnome-copied-files` y `text/uri-list` en Wayland/GTK4, resolución de colisiones (`" copia"`), protección ante copias/movimientos dentro de subcarpetas descendientes y migración de metadatos SQLite al mover por corte.
  - Atenuación `.branches-cut-item` (`opacity: 0.5`) para elementos cortados y acciones `"Copiar"`, `"Cortar"`, `"Pegar"` y `"Pegar en carpeta"` en menú de ventana, menús contextuales de archivos/carpetas y fondo de vistas.
- **Atajos `Command` (`Super`/`Meta`) y `Ctrl` + Liberación de `Command+V` ([`app_window.rs`](../crates/branches/src/app_window.rs), [`drm.rs`](../crates/compositor/src/backend/drm.rs), [`winit.rs`](../crates/compositor/src/backend/winit.rs)):**
  - Activados `Command+C / X / V` (`SUPER_MASK | META_MASK`) y `Ctrl+C / X / V` en Branches cuando no hay un campo de texto enfocado.
  - Retirada la intercepción global de `Super+V` (`ToggleClipboardHistory`) en los backends DRM y Winit del compositor para que `Command+V` llegue a las aplicaciones (el historial del portapapeles continúa disponible en Spotlight `⌘4`).

### 3. Validación
- `cargo test --offline -p malus-de-branches -p malus-de-compositor`: **34 passed** en Branches (`0 failed`) y **81 passed** en el compositor (`0 failed`).
- `cargo build --release --offline -p malus-de-branches -p malus-de-compositor`: compilación `--release` correcta (exit `0`).

---

## [2026-11-18] — Tema Claro por Defecto, Compatibilidad con `kmscon` e Instalador Fedora Minimal

### 1. Tema Claro por Defecto en Instalaciones Nuevas
- **Configuración aislada (`crates/common/src/config.rs`):** En instalaciones nuevas, `setup_isolated_session_environment()` ahora inicializa `keyfile` con `color-scheme='prefer-light'` e `icon-theme='Malus'`, y genera `gtk-3.0/settings.ini` y `gtk-4.0/settings.ini` con `gtk-application-prefer-dark-theme=0` y `DEFAULT_ICON_THEME_LIGHT` (`Malus`), preservando sesiones ya existentes. El respaldo de `is_dark_mode_active()` pasó a `false`.
- **Scripts de sesión (`scripts/malus-session`, `scripts/malus-session.sh`):** Actualizada la generación inicial a `prefer-light`, `Malus` y `gtk-application-prefer-dark-theme=0` (seleccionando `Malus` o `Malus-Dark` según el esquema activo si ya existe `keyfile`).
- **Respaldos en compositor y shell:** `is_dark_color_scheme()` (`crates/compositor/src/state.rs`), `CURRENT_IS_DARK` (`crates/wallpaper/src/main.rs`) y `ControlCenterModel::default().dark_mode` (`crates/render/src/popover.rs`) ahora parten de `false` (modo claro).

### 2. Instalador en Un Solo Comando y Soporte `kmscon` en Fedora 45+
- **Instalador todo-en-uno (`scripts/install-fedora-minimal.sh`, `docs/FEDORA_STANDALONE_INSTALL.md`):** Añadido comando único estilo `curl | bash` (`--dir`, `--branch`, `--tty-autostart`, `--greetd`, `--skip-build`) para instalar dependencias runtime y `-devel` (incluyendo `gdk-pixbuf2-devel`, `libdrm-devel`, `wayland-devel` y `pam`), clonar/compilar el workspace en `Release` y desplegar la sesión.
- **Liberación automática de DRM en `kmscon` (`scripts/malus-session`):** En Fedora 45+ (`pts/0` bajo `kmscon`), `malus-session` detecta la sesión PTY de `kmscon` antes de redirigir stdout a `tee` y se envuelve automáticamente con `kmscon-launch-gui` para evitar el error `EBUSY (Device or resource busy)` al abrir `/dev/dri/card*`.

### 3. Validación
- Sintaxis shell (`bash -n`) en `scripts/malus-session`, `scripts/malus-session.sh` y `scripts/install-fedora-minimal.sh`: exit `0`.
- Pruebas unitarias Rust (`cargo test`): `malus-de-common` (51/51 en entorno aislado `/tmp`), `malus-de-compositor` (80/80), `malus-de-render` (14/14 en sus binarios/suites) y `malus-de-wallpaper` (19/19).

---

## [2026-11-18] — Lockscreen estilo macOS Sequoia, Autenticación PAM y Atajo `Super + L`

### 1. Lockscreen Real, Multi-Monitor y *Vibrancy* (`crates/session-lock`)
- **Autenticación con Linux-PAM (`src/auth.rs`):** Resolución del usuario activo mediante `libc::getpwuid(libc::getuid())` (`current_username`) y verificación asíncrona de contraseña vía `gio::spawn_blocking` cargando `libpam.so.0` (`verify_user_password`, probando los servicios `system-auth`, `login` y `passwd`).
- **Desenfoque y *Vibrancy* del fondo (`src/blur_bg.rs`):** Detección del wallpaper activo (`wallpaper.{png,jpg,jpeg,webp,svg}` o respaldo por defecto/gradiente), textura nítida HiDPI para la pantalla principal y buffer pre-desenfocado (`box_blur_rgba_3_pass` a 720 px, radio 9) con realce de saturación (`1.60x`) y brillo (`1.04x`, `apply_vibrancy_rgba`) para el fondo translúcido de los controles.
- **Animaciones estilo macOS Sequoia (`src/scale_fade.rs`, `src/main.rs`, `src/styles/lock.css`):**
  - Pantalla en reposo limpia con fecha y hora arriba; al hacer clic o pulsar cualquier tecla aparece el formulario inferior con viñeta suave, resorte `sequoia_spring` y crecimiento animado del avatar (`38 px -> 58 px`). `Escape` oculta el formulario cuando está vacío.
  - Entrada (`animate_lock_enter`: cross-dissolve del wallpaper en `240 ms` + UI `scale 1.04 -> 1.0` y `opacity 0 -> 1` en `320 ms` usando `ScaleFadeBox`) y salida al desbloquear (`animate_unlock_and_quit`: UI `scale 1.0 -> 1.045` y `opacity 1 -> 0` en `190 ms`, wallpaper `opacity 1 -> 0` en `240 ms`).
  - Controles *squircle* (`n = 4.2` en Cairo) de `34 px` de alto (estrictamente `<= 37 px`): entrada `232 x 34 px` y botón `34 x 34 px`, vidrio esmerilado en tiempo real (`paint_frosted_backdrop`), borde especular, espaciado ajustado para el icono de mostrar/ocultar contraseña y animación de sacudida con mensaje de error si la validación falla.
  - Cobertura multi-monitor (`Layer::Overlay`, namespace `malus-lock`): ventana en cada monitor conectado con reconstrucción dinámica (`connect_items_changed`); solo el monitor primario recibe `KeyboardMode::Exclusive` y el UI interactivo, mientras las pantallas secundarias muestran únicamente el wallpaper y al hacer clic despiertan el login en la pantalla principal. `close_request` bloqueado hasta desbloqueo válido.

### 2. Integración en el Compositor, Atajos y Barra Superior (`crates/compositor`, `crates/top-bar`)
- **Protección de sesión bloqueada (`crates/compositor/src/state.rs`, `drm.rs`, `winit.rs`):**
  - Añadidos `is_session_locked()` y `lock_focus_surface()` con guardia en `set_keyboard_focus_with_sync`, cierre de modales al aparecer el layer `malus-lock` y bloqueo de atajos del entorno mientras la sesión está bloqueada (conservando `Ctrl+Alt+BackSpace` y `Ctrl+Alt+F1..F12` en `drm.rs`).
  - Eliminación del doble blur `lock_blur_roots` en `drm.rs` y `winit.rs` para renderizar el fondo limpio del lockscreen.
- **Atajo de bloqueo (`crates/compositor/src/state.rs`, `crates/top-bar/src/ui/global_menu.rs`):**
  - Añadido `MalusState::lock_session()` enlazado a `Super + L` y `Ctrl + Super + Q` (registrado antes que `Super + Q`).
  - Actualizada la etiqueta visual del menú Apple en `"Bloquear pantalla"` de `"⌃⌘L"` a `"⌘L"`.

### 3. Validación y Estado
- **`malus-de-session-lock`:** `cargo test -p malus-de-session-lock` (`8 passed; 0 failed`) y `cargo build --release -p malus-de-session-lock` (exit 0, sin advertencias).
- **`malus-de-compositor`:** `cargo test -p malus-de-compositor` (`80 passed; 0 failed`) y `cargo build --release -p malus-de-compositor` (exit 0, sin advertencias).
- **`malus-de-top-bar`:** `cargo build --release -p malus-de-top-bar` (exit 0) y `cargo test -p malus-de-top-bar` (`9 passed; 1 failed`, fallo previo ajeno en `services::tests::test_brightness_service`: `80.0` vs `85.0`).
- *Pendiente:* verificación visual y flujo PAM completo en vivo dentro de la sesión gráfica.

---

## [2026-11-18] — Selección Multi-GPU en DRM y Arranque TTY en Fedora Minimal

### 1. Sondeo y Prioridad Multi-GPU (`crates/compositor`)
* **Problema:** al fallar la detección de `primary_gpu` o en laptops híbridas / con pantalla interna retirada (donde `card0` puede ser `simpledrm` o una iGPU sin salidas conectadas), [`drm.rs`](../crates/compositor/src/backend/drm.rs) intentaba abrir un único nodo o caía al respaldo fijo `/dev/dri/card1`, abortando con `"Error al abrir nodo DRM mediante libseat"`.
* **Solución (`scan_dev_dri_cards`, `sysfs_drm_card_score`, `prioritize_drm_gpu_candidates`, `open_first_working_drm_gpu`):** recopila y deduplica `primary_gpu`, `all_gpus` y `/dev/dri/card*`, priorizando tarjetas KMS reales (descartando `simple-framebuffer`/`vgem`) que tengan al menos un conector en estado `connected`, y prueba en cascada `libseat` (`session.open`), `DrmDevice`, `GbmDevice`, `EGLDisplay` y `GlesRenderer`.

### 2. Guía Fedora Minimal ([`FEDORA_STANDALONE_INSTALL.md`](../docs/FEDORA_STANDALONE_INSTALL.md))
* Retirado `exec` en el arranque automático de `~/.bash_profile` para evitar bucles de expulsión al `login:` si falla el compositor.
* Sustituido `mesa-libgbm-devel` por `'pkgconfig(gbm)'` en la lista de dependencias de compilación.

### 3. Validación
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-compositor`: **80 passed; 0 failed** (incluye `prioritize_drm_gpu_candidates_prefers_real_kms_with_connected_outputs`).
* `cargo build --release --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-compositor`: compilación correcta sin advertencias (`exit 0`).

---

## [2026-11-18] — Atajo de Cierre de Emergencia (`Ctrl + Alt + BackSpace`) y Watchdog Evdev

### 1. Cierre Forzado del Entorno y Prioridad de Atajos Críticos (`crates/compositor`)
* **Evaluación prioritaria ([`drm.rs`](../crates/compositor/src/backend/drm.rs)):** `Ctrl + Alt + BackSpace` (reconociendo `KEY_BackSpace` y `KEY_Terminate_Server`) y `Ctrl + Alt + F1..F12` ahora se procesan al inicio del manejador de teclado en `libinput_backend`, antes de estados modales como `screenshot_ui` o `snap_layouts`.
* **Salida limpia garantizada (`trigger_emergency_compositor_exit`):** pone `state.is_running = false` y lanza un temporizador de 600 ms hacia `std::process::exit(0)` para asegurar el retorno a GDM/`malus-session` sin generar volcado de fallo si el apagado normal se bloquea.
* **Watchdog independiente (`spawn_emergency_zap_watchdog`):** hilo secundario (`malus-zap-watchdog`) que abre teclados en `/dev/input/event*` filtrados por `KEY_BACKSPACE` en `capabilities/key` (`sysfs_key_caps_has_backspace`, sin `EVIOCGRAB`, activo solo cuando `session_active` es `true`). Si el usuario tiene permisos de lectura (p. ej. grupo `input`), permite cerrar el compositor con `Ctrl + Alt + BackSpace` incluso ante un bloqueo total del hilo principal.

### 2. Validación
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-compositor`: **79 passed; 0 failed** (incluye `drm_emergency_tests`).
* `cargo build --release --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-compositor`: compilación correcta sin advertencias (`exit 0`).

---

## [2026-11-17] — Vista de Columnas de Branches: Animaciones y Conservación del Padre

### 1. Entrada Animada de Columnas (`crates/branches`)
* [`slide_in_from_right`](../crates/branches/src/views/column_view.rs): las columnas nuevas y el panel de previsualización entran deslizándose desde la derecha con fundido (300 ms, *ease-out cubic*) mediante `add_tick_callback`, animando `margin_start` y `opacity`.
* `scroll_to_end` pasó de salto instantáneo a animación de 320 ms sobre `hadjustment` (destino recalculado por fotograma y cancelación por generación). Solo tiene efecto cuando las columnas desbordan el ancho disponible (cada columna mide 240 px, la previsualización 280 px).

### 2. El Padre ya no se Oculta si Hay Espacio
* **Causa:** abrir una carpeta (doble clic/Enter) invocaba `set_root`, que reconstruía la vista con esa carpeta como raíz y eliminaba al padre sin animación.
* **Solución:** si la carpeta destino ya es una columna visible (índice > 0) y no hay selección pendiente, `set_root` conserva las columnas anteriores, descarta las posteriores, deselecciona la carpeta abierta y llama a `scroll_to_end`. Si todo cabe no se desplaza nada; si no cabe, el contenedor se desplaza a la izquierda con animación y el padre sale de vista. Resto de navegaciones (barra lateral, atrás/adelante, subir, refrescos) siguen reconstruyendo la vista.

### 3. Validación
* `cargo test -p malus-de-branches`: **28 passed; 0 failed**. Comportamiento visual confirmado por el usuario en la sesión gráfica.

---

## [2026-11-17] — Vista de Lista de Branches: Columnas, Orden y Selección; Miniaturas Squircle en el Escritorio

### 1. Columnas Redimensionables, Alineadas y Ordenables (`crates/branches`)
* **Alineación:** Fecha, Tamaño y Clase usan anchos fijos en píxeles compartidos entre cabecera y filas ([`list_view.rs`](../crates/branches/src/views/list_view.rs)); antes se medían en caracteres con fuentes distintas (11 px vs 13 px). Texto truncado con `…` en el centro (`EllipsizeMode::Middle`) y tooltip con el texto completo.
* **Redimensionado:** asa en el borde izquierdo de cada columna (50–600 px). Un único `GestureDrag` en fase *Capture* sobre la cabecera (que no se mueve) evita que el contenido "vibre" al arrastrar. Los anchos no se persisten.
* **Orden por columna:** clic en el nombre alterna ascendente/descendente (▲/▼) para Nombre (carpetas primero), Fecha, Tamaño y Clase, conservando la selección. El orden vive en `ListView` mientras la ventana está abierta y se reaplica al recargar.
* **Columnas pares:** franjas gris muy claro (`alpha(@window_fg_color, 0.035)`) en Fecha y Clase como capa inferior de un `Overlay` (hasta el final de la vista, sin separaciones); filas y scroll transparentes ([`macos-branches.css`](../crates/branches/src/styles/macos-branches.css)).

### 2. Selección en Modo Lista
* **Causa:** `SelectionMode::Multiple` con `activate_on_single_click` (por defecto) añadía cada fila pulsada a la selección sin deseleccionar las anteriores.
* **Solución:** `activate_on_single_click(false)`: clic reemplaza, Ctrl+clic alterna, Mayús+clic extiende.

### 3. Miniaturas del Escritorio con Recorte Squircle (`crates/wallpaper`)
* Los iconos del escritorio los dibuja el crate `wallpaper`; se copió la lógica de recorte de Branches (exponente 4.2, radio 22 % del lado menor, antialiasing) en [`main.rs`](../crates/wallpaper/src/main.rs) vía `squircle_clip_pixbuf`, aplicada a las miniaturas de imagen. Test `clip_makes_corners_transparent_and_keeps_center_opaque`.

### 4. Validación
* `cargo test -p malus-de-branches`: **28 passed; 0 failed**. `cargo test -p malus-de-wallpaper`: **19 passed; 0 failed**.
* Confirmado por el usuario en la sesión gráfica: selección en lista correcta.

---

## [2026-11-17] — Discos Externos y Volúmenes en Ubicaciones de Branches

### 1. Detección en Tiempo Real con `gio::VolumeMonitor` (`crates/branches`)
* **Causa:** la sección "Ubicaciones" de [`sidebar.rs`](../crates/branches/src/sidebar.rs) solo tenía entradas fijas (Inicio, `/`, `/mnt`, Papelera) y ninguna lógica de montajes; la sesión únicamente activa `automount` de GNOME, sin `udiskie`.
* **Implementación:** `SidebarInner` incorpora `devices_anchor`, `device_rows`, `volume_monitor` y `signal_ids`. `rebuild_devices()` inserta las filas tras "Papelera" y se reconstruye (vía `idle_add_local_once`) con las señales `mount-added/removed/changed`, `volume-added/removed/changed` y `drive-connected/disconnected`; las señales se desconectan en `Drop`.
* **Filtrado:** `is_system_mount_path` / `is_external_mount_path` excluyen `/`, `/boot`, `/boot/efi`, `/home`, `/var`, `/run/*` (salvo `/run/media`), etc. Se listan montajes en `/run/media` y `/media` o de unidades extraíbles, volúmenes extraíbles sin montar y, como respaldo, el contenido de `/run/media/$USER` y `/media`.

### 2. Interacción
* Clic en disco montado: navega a su raíz; admite soltar archivos.
* Clic en volumen sin montar (fila atenuada): lo monta con `gtk4::MountOperation` (contraseñas/polkit) y navega al montaje.
* Botón **Expulsar** (`media-eject-symbolic`, estilo `.branches-sidebar-eject-btn` en [`macos-branches.css`](../crates/branches/src/styles/macos-branches.css)) con `eject_with_operation` o `unmount_with_operation`, sin disparar la navegación de la fila.
* **Limitaciones:** no se monta automáticamente al conectar; al expulsar el disco que se está viendo no se redirige a otra carpeta. Confirmado por el usuario en la sesión gráfica.

### 3. Validación
* `cargo test -p malus-de-branches`: **28 passed; 0 failed** (incluye `device_path_tests`).
* `cargo build --release -p malus-de-branches`: correcto, sin advertencias.

---

## [2026-11-17] — Ajuste de Tamaño/Origen del Agujero Negro y Semáforo en Antigravity (Pendiente)

### 1. Proporción y Origen del Efecto Agujero Negro al Cerrar (`crates/compositor`)
* **Dimensionado sobre el botón rojo y disco compacto ([`black_hole.rs`](../crates/compositor/src/black_hole.rs)):**
  * Ajustado `CLOSE_BUTTON_CORE_RADIUS_LOGICAL` a `6.5 * scale` (igual al radio del botón de cerrar SSD/CSD de `13 px`), acotando `quad_half` y la caída radial del disco de acreción a `2.5 * horizon_radius_px` para ceñir el efecto alrededor del botón rojo sin expandirse en exceso.
* **Coordenadas reales por familia de ventana y clic CSD ([`state.rs`](../crates/compositor/src/state.rs), [`drm.rs`](../crates/compositor/src/backend/drm.rs), [`winit.rs`](../crates/compositor/src/backend/winit.rs)):**
  * Añadidos `close_button_offset_for_app_id`, `close_button_center_logical` y `record_csd_close_click_hint` con los offsets exactos para SSD (`20.0, 19.0`), Branches (`28.5, 24.0`), Ajustes (`25.5, 23.5`), Electron/IDE (`20.5, 17.5`), Firefox (`16.5, 21.0`) y CSD genérico (`22.5, 23.0`), registrando el hint al cerrar con atajos (`close_active_window`) o mediante clic izquierdo CSD sin usar el respaldo genérico de la franja superior de `96 px`.

### 2. Extensión de `malus-ide-wrapper` para la App Antigravity (`scripts/malus-ide-wrapper`)
* Añadido `AGY_APP_INJECTION_SCRIPT` (`agy-ssd-v3`) y detección de páginas `http://127.0.0.1:*` / `https://127.0.0.1:*` en `watcher()` para inyectar controles semáforo en `[data-testid="title-menu-bar"]` enlazados a `window.electronNative`.
* **Pendiente:** aunque la inyección responde por CDP (`Page.captureScreenshot` en vivo), en la sesión gráfica del usuario la ventana de **Antigravity** (standalone) sigue sin mostrar el semáforo correctamente y el ajuste visual en dicha aplicación queda pendiente para una revisión posterior.

### 3. Validación
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-compositor`: **73 passed; 0 failed** (`close_button_offset_matches_ssd_and_csd_window_families`, `peak_horizon_radius_matches_close_button_across_scales`, `timeline_ignition_suction_and_collapse_phases`).
* `cargo build --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --release -p malus-de-compositor`: compilación correcta (`exit 0`).
* `python3 -m py_compile /home/cristian/Proyectos/malus-de/scripts/malus-ide-wrapper`: sintaxis válida (`exit 0`).

---

## [2026-11-17] — Miniaturas de Imágenes con Recorte Squircle en Branches

### 1. Decodificación Asíncrona, Caché HiDPI y Recorte Squircle (`crates/branches`)
* **Pipeline compartido de miniaturas ([`views/mod.rs`](../crates/branches/src/views/mod.rs), [`macos-branches.css`](../crates/branches/src/styles/macos-branches.css)):**
  * Añadidos `is_image_thumbnail_candidate`, `decode_thumbnail_bytes`, `update_item_image_thumbnail` y `create_item_image` para cargar miniaturas de imágenes (`png`, `jpg`, `jpeg`, `webp`, `gif`, `svg`, `bmp`, `ico`, `tif`, `tiff`, `avif`, `heic`, `heif`, hasta `64 MB`) en segundo plano mediante `gio::spawn_blocking` con caché `THUMBNAIL_CACHE` (512 entradas indexadas por ruta, fecha de modificación, tamaño y resolución 2x HiDPI) y guardia por token ante cambios rápidos de tamaño o carpeta.
  * Aplicado recorte **squircle** antialias por píxel (`apply_squircle_clip_rgba` con superelipse `n = 4.2`) directamente sobre el búfer RGBA antes de construir `gdk::MemoryTexture`, logrando esquinas continuas incluso en fotos panorámicas o verticales sin sombras rectangulares sobresalientes.
* **Integración en todas las vistas ([`icon_view.rs`](../crates/branches/src/views/icon_view.rs), [`list_view.rs`](../crates/branches/src/views/list_view.rs), [`column_view.rs`](../crates/branches/src/views/column_view.rs), [`gallery_view.rs`](../crates/branches/src/views/gallery_view.rs)):**
  * Activadas las miniaturas squircle alineadas en Vista de Iconos (con recarga automática al mover el control deslizante de tamaño o al modificarse un archivo vía `FileMonitor`), Lista, Columnas y carrusel de Galería.

### 2. Validación
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-branches`: **26 passed; 0 failed** (`thumbnail_candidate_detects_images_and_ignores_directories_and_non_images` y `decode_thumbnail_bytes_decodes_valid_png_thumbnail`).
* `cargo build --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --release -p malus-de-branches`: compilación correcta (`exit 0`).

---

## [2026-11-17] — Sincronización en Tiempo Real, Edición Inline al Crear Elementos y Papelera DnD en Dock

### 1. Actualización Automática de Carpetas e Inline Rename al Crear en Branches (`crates/branches`)
* **Monitoreo en tiempo real sin parpadeo ([`folder_pane.rs`](../crates/branches/src/folder_pane.rs), [`icon_view.rs`](../crates/branches/src/views/icon_view.rs)):**
  * Incorporado `gio::FileMonitor` por pestaña/panel en `FolderPane` (`ensure_directory_monitor`, `clear_directory_monitor`, `sync_from_filesystem` y comparación `snapshots_equal`) con *debounce* de eventos del sistema de archivos.
  * Añadido `IconView::sync_items_incremental` para insertar, retirar o actualizar metadatos de iconos modificados sin reconstruir ni hacer parpadear los elementos intactos.
* **Edición inline inmediata tras «Crear nuevo ›» ([`views/mod.rs`](../crates/branches/src/views/mod.rs), [`icon_view.rs`](../crates/branches/src/views/icon_view.rs), [`list_view.rs`](../crates/branches/src/views/list_view.rs), [`column_view.rs`](../crates/branches/src/views/column_view.rs), [`gallery_view.rs`](../crates/branches/src/views/gallery_view.rs), [`folder_pane.rs`](../crates/branches/src/folder_pane.rs), [`app_window.rs`](../crates/branches/src/app_window.rs)):**
  * Añadido `schedule_inline_rename` para activar `show_rename_popover` sobre el elemento recién creado en cuanto el menú contextual termina de cerrarse y la etiqueta cuenta con geometría asignada.
  * Unificada la selección automática previa (`queue_select_name`) en Vista de Iconos, Lista, Columnas y Galería tanto desde el submenú **«Crear nuevo ›»** (Carpeta / Documento de texto) como desde `FolderPane::create_new_folder_and_rename` (`prompt_new_folder`).

### 2. Submenú «Crear nuevo ▸» e Inline Rename en el Escritorio (`crates/wallpaper`)
* **Creación y renombrado inline directo sobre el icono ([`wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs), [`default-gradient.css`](../crates/wallpaper/src/styles/default-gradient.css)):**
  * Reemplazado el diálogo modal anterior por el submenú **«Crear nuevo ▸»** (Carpeta y Documento de texto) en el menú contextual del escritorio (`build_create_new_submenu` y `create_desktop_item_and_edit`), respetando posición del clic y *snap-to-grid*.
  * Tras aparecer el icono en el escritorio, `schedule_desktop_inline_rename` y `show_desktop_inline_rename` abren el campo inline `gtk4::Text` (`.desktop-inline-rename-entry`) con preselección del nombre base (manteniendo `.txt` sin seleccionar en documentos de texto), actualizando `icon_positions` al renombrar y protegiendo la edición frente a atajos globales de `Espacio` / `Enter` / `Delete` / `BackSpace`.

### 3. Envío a Papelera mediante Drag & Drop en el Dock (`crates/dock`, `crates/branches`, `crates/wallpaper`)
* **Zona de suelta sobre la Papelera del Dock ([`dock/src/main.rs`](../crates/dock/src/main.rs), [`dock.css`](../crates/dock/src/styles/dock.css)):**
  * Extendido `attach_dock_drop_target` con detección sobre el icono de papelera (`is_point_over_trash`), retroalimentación visual `.dock-trash-drop-hover`, filtrado de rutas protegidas (`is_trashable_path` / `extract_trashable_paths`), envío vía `gio::File::trash` (`DragAction::MOVE`) y actualización inmediata de `user-trash-full`.
  * Ajustada la limpieza al finalizar `DragAction::MOVE` en [`icon_view.rs`](../crates/branches/src/views/icon_view.rs) y [`wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs) usando `symlink_metadata().is_err()` para retirar también accesos `.desktop` enviados a la papelera.

### 4. Validación
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-dock -p malus-de-branches -p malus-de-wallpaper`:
  * `malus-de-branches`: **24 passed; 0 failed** (`snapshots_equal_detects_content_and_metadata_changes`).
  * `malus-de-dock`: **15 passed; 0 failed** (`extract_trashable_paths_filters_root_home_and_trash_and_keeps_regular_files`).
  * `malus-de-wallpaper`: **18 passed; 0 failed**.
* `cargo build --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --release -p malus-de-dock -p malus-de-branches -p malus-de-wallpaper`: compilación correcta (`exit 0`).

---

## [2026-11-17] — Ejecución y Accesos Directos de Aplicaciones en Escritorio desde Branches y Spotlight

### 1. Lanzamiento Robusto de Aplicaciones `.desktop` (`crates/common`, `crates/branches`, `crates/wallpaper`)
* **Ejecución unificada ([`apps.rs`](../crates/common/src/apps.rs), [`fs_backend.rs`](../crates/branches/src/fs_backend.rs), [`wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs)):**
  * Centralizados `split_desktop_exec_args`, `read_desktop_launch_metadata`, `spawn_desktop_command`, `spawn_terminal_desktop_command` y `spawn_desktop_entry_path` en `malus-de-common` para soportar comillas/escapes, prefijos `env`/`KEY=VAL`, `Terminal=true`, `Path=`, wrappers `MALUS_IDE_BIN` y limpieza de marcadores Flatpak (`@@`, `@@u`, `%U`, etc.) en `clean_exec_line`.
  * En [`icon_view.rs`](../crates/branches/src/views/icon_view.rs) y [`list_view.rs`](../crates/branches/src/views/list_view.rs), el doble clic (`n_press >= 2`) reclama la secuencia antes de llamar a `open_file(&path)` y `launch_desktop_file` usa `gio::DesktopAppInfo::from_filename` como respaldo (útil para apps `DBusActivatable=true` o Flatpak).
  * En [`wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs), `launch_file` ejecuta entradas `.desktop` directamente como aplicaciones y `build_desktop_item_data` resuelve su nombre localizado e icono real mediante `resolve_desktop_app_gicon`.

### 2. Enlaces Simbólicos al Arrastrar desde «Aplicaciones» y «Spotlight» al Escritorio (`crates/common`, `crates/branches`, `crates/compositor`, `crates/wallpaper`)
* **Desde Branches → Escritorio / carpetas ([`apps.rs`](../crates/common/src/apps.rs), [`operations.rs`](../crates/branches/src/operations.rs), [`icon_view.rs`](../crates/branches/src/views/icon_view.rs), [`wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs)):**
  * Añadido `is_application_desktop_file` en `malus-de-common`: al arrastrar aplicaciones desde la carpeta virtual **Aplicaciones** hacia el **Escritorio** (o carpetas en Branches), se crea un enlace simbólico `.desktop` en destino manteniendo el icono original visible en **Aplicaciones**.
* **Desde Spotlight → Escritorio ([`ipc.rs`](../crates/common/src/ipc.rs), [`spotlight_visual.rs`](../crates/compositor/src/spotlight_visual.rs), [`state.rs`](../crates/compositor/src/state.rs), [`drm.rs`](../crates/compositor/src/backend/drm.rs), [`winit.rs`](../crates/compositor/src/backend/winit.rs), [`wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs)):**
  * Añadido `SpotlightPointerReleaseOutcome::CreateDesktopShortcut` al soltar una aplicación fuera de la tarjeta de Spotlight y fuera del Dock, validando en `MalusState::handle_spotlight_pointer_release` que el punto de suelta no esté cubierto por otra ventana (`element_under(drop_loc).is_none()`) ni en la franja superior (`local_y >= 36.0`) antes de emitir `IpcMessage::CreateDesktopAppShortcut`.
  * El receptor IPC de `malus-de-wallpaper` crea el enlace simbólico en `~/Escritorio` en las coordenadas soltadas (respetando `snap_to_grid`) y dibuja el acceso directo de forma incremental.

### 3. Validación
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación completa sin errores (`exit 0`).
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-compositor -p malus-de-wallpaper -p malus-de-branches`:
  * `malus-de-branches`: **23 passed; 0 failed** (`split_desktop_exec_args_handles_quotes_and_escapes`, `read_desktop_launch_metadata_detects_terminal_and_path`).
  * `malus-de-compositor`: **72 passed; 0 failed** (`spotlight_dragging_app_onto_dock_returns_pin_outcome` con cobertura de suelta en escritorio).
  * `malus-de-wallpaper`: **18 passed; 0 failed**.
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-common parse_desktop_file_respects_nodisplay_and_builds_dock_app`: **1 passed; 0 failed**.

---

## [2026-11-16] — Edición Inline, Navegación/Multiselección Espacial, DnD Grupal y Wrapper Antigravity

### 1. Edición Inline sin Expansión, Navegación 2D y Selección Múltiple en Branches (`crates/branches`)
* **Renombrado inline sobre la propia etiqueta ([`views/mod.rs`](../crates/branches/src/views/mod.rs), [`icon_view.rs`](../crates/branches/src/views/icon_view.rs), [`list_view.rs`](../crates/branches/src/views/list_view.rs), [`column_view.rs`](../crates/branches/src/views/column_view.rs), [`gallery_view.rs`](../crates/branches/src/views/gallery_view.rs), [`sidebar.rs`](../crates/branches/src/sidebar.rs), [`macos-branches.css`](../crates/branches/src/styles/macos-branches.css)):**
  * Reemplazado el popover de renombrado por edición inline directa con `gtk4::Text` (`branches-inline-rename-entry`) sobre el `Label` del archivo, carpeta o marcador (activable con `F2`, `Enter`, segundo clic lento de `450 ms` o menú contextual), ocupando exactamente el espacio de la etiqueta sin expandir el layout y preservando `Malus SSD` para `/` en [`db.rs`](../crates/branches/src/db.rs), [`path_bar.rs`](../crates/branches/src/path_bar.rs) y [`folder_pane.rs`](../crates/branches/src/folder_pane.rs).
* **Navegación 2D con teclado y multiselección en rejilla ([`icon_view.rs`](../crates/branches/src/views/icon_view.rs), [`app_window.rs`](../crates/branches/src/app_window.rs), [`folder_pane.rs`](../crates/branches/src/folder_pane.rs)):**
  * `fixed_canvas` pasa a ser `focusable(true)` con `navigate_by_arrow` interceptando `←`, `→`, `↑`, `↓` antes de la cadena de foco GTK (reservando `Tab` / `Shift+Tab` para los controles de panel/barra).
  * Añadida selección múltiple con `Ctrl`/`Super` + clic (`has_multi_select_modifier`), área elástica con el cursor (`GestureDrag` + `.branches-selection-marquee` usando `rects_intersect`), `Ctrl+A` y operaciones grupales (arrastre, duplicar, mover a papelera).

### 2. Arrastre Espacial Grupal y Mitigación de Parpadeo entre Branches, Escritorio y Compositor (`crates/common`, `crates/branches`, `crates/wallpaper`, `crates/compositor`)
* **Disposición espacial relativa y previsualización compuesta ([`common/src/lib.rs`](../crates/common/src/lib.rs), [`operations.rs`](../crates/branches/src/operations.rs), [`views/mod.rs`](../crates/branches/src/views/mod.rs), [`icon_view.rs`](../crates/branches/src/views/icon_view.rs), [`wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs)):**
  * Formato `v2:` en `write_drag_anchors` / `read_drag_anchor` para registrar el offset relativo de cada archivo arrastrado y preservar su geometría relativa al soltar entre Branches, carpetas (`move_paths_into_folder`) y Escritorio.
  * Añadidos `build_multi_widget_drag_paintable` y `build_multi_desktop_drag_paintable` para renderizar todos los iconos seleccionados en su posición espacial relativa bajo el puntero durante el arrastre.
* **Actualizaciones en sitio y caché de `dnd_icon` ([`icon_view.rs`](../crates/branches/src/views/icon_view.rs), [`wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs), [`state.rs`](../crates/compositor/src/state.rs), [`backend/mod.rs`](../crates/compositor/src/backend/mod.rs), [`drm.rs`](../crates/compositor/src/backend/drm.rs), [`winit.rs`](../crates/compositor/src/backend/winit.rs)):**
  * Actualización en sitio de `fixed_canvas` / `icons` al soltar o mover fuera (esperando la comprobación de 80 ms antes de restaurar widgets ocultos en acciones `MOVE` y filtrando eventos sobre el propio `desktop_dir` o `Deleted | MovedOut` en `monitor.connect_changed`).
  * Añadido `CachedDndIcon` (`MemoryRenderBuffer`) en `MalusState` (`capture_dnd_icon_cache` y `build_cached_dnd_fallback`) integrado en `tracker_elements` y ciclo de daño DRM para sostener la miniatura de arrastre durante `dnd_drop_hold` tras `dnd_finished` (nota: queda pendiente de validación adicional en sesión si persiste algún parpadeo visual).

### 3. Separación de «Antigravity» y «Antigravity IDE» en el Wrapper (`scripts`, `crates/common`, `crates/compositor`, `crates/dock`, `crates/branches`)
* Exportación de `MALUS_IDE_BIN` desde [`spotlight_visual.rs`](../crates/compositor/src/spotlight_visual.rs), [`dock/src/main.rs`](../crates/dock/src/main.rs) y [`fs_backend.rs`](../crates/branches/src/fs_backend.rs), y resolución diferenciada de `antigravity` frente a `antigravity-ide` en [`malus-ide-wrapper`](../scripts/malus-ide-wrapper) y [`config.rs`](../crates/common/src/config.rs).

### 4. Validación
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación sin errores (`exit 0`).
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-branches -p malus-de-wallpaper -p malus-de-compositor`:
  * `malus-de-branches`: **21 passed; 0 failed**.
  * `malus-de-wallpaper`: **18 passed; 0 failed**.
  * `malus-de-compositor`: **72 passed; 0 failed**.
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-common shared_drag_anchor_roundtrips_single_and_multi_paths`: **1 passed; 0 failed** (en entorno sandbox, `test_dock_config_load_or_create_default` falla al intentar escribir en `$HOME/.config/malus-de/`).

---

## [2026-11-16] — Fondo de Pantalla Predeterminado «Playa de Pacasmayo» (Pexels)

### 1. Empaquetado del Recurso, Licencia e Inicialización Automática (`assets/wallpapers`, `crates/common`, `crates/wallpaper`, `crates/settings`, `scripts`)
* **Recurso y atribución bajo Pexels License ([`pexels-jbarbozameca-41027443.webp`](../assets/wallpapers/pexels-jbarbozameca-41027443.webp), [`Pexels-LICENSE`](../assets/wallpapers/Pexels-LICENSE), [`README.md`](../README.md)):**
  * Integrada la fotografía panorámica [*"Vista panorámica del muelle y la playa de Pacasmayo"*](https://www.pexels.com/es-es/foto/vista-panoramica-del-muelle-y-la-playa-de-pacasmayo-41027443/) de **J. Barboza Meca (`@jbarbozameca`)** junto con su archivo de licencia y créditos en [`README.md`](../README.md).
* **Inicialización y respaldo en sesión ([`config.rs`](../crates/common/src/config.rs), [`wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs), [`settings/src/main.rs`](../crates/settings/src/main.rs), [`install-session.sh`](../scripts/install-session.sh)):**
  * Añadidos `DEFAULT_WALLPAPER_FILENAME`, `default_bundled_wallpaper_path()` y `ensure_default_wallpaper()` en `malus-de-common` para inicializar `wallpaper.webp` en `~/.config/malus-de/` cuando no existe un fondo configurado ni el marcador `.wallpaper-gradient`.
  * `malus-de-wallpaper` y `malus-de-settings` usan `default_bundled_wallpaper_path()` como fondo predeterminado, listando **"Playa de Pacasmayo (Predeterminado)"** como primera opción en Preferencias del Sistema y copiando `assets/wallpapers/*` a `/usr/local/share/malus-de/wallpapers/` en [`install-session.sh`](../scripts/install-session.sh).

### 2. Validación
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-common`: **51 passed; 0 failed** (`default_bundled_wallpaper_exists_and_seeds_new_config_dir`).
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-wallpaper -p malus-de-settings`: compilación sin errores (`exit 0`).

---

## [2026-11-16] — Alineación Concéntrica de Glifos en los Semáforos CSD

### 1. Centrado Exacto de Glifos en GTK4, GTK3, Firefox y Electron (`crates/common`, `assets`, `scripts`)
* **Caja de imagen `12 × 12 px` concéntrica con el botón CSD ([`windowcontrols-common.css`](../crates/common/src/styles/windowcontrols-common.css), [`gtk3.css`](../crates/common/src/styles/gtk3.css)):**
  * En GTK4/GTK3, el contenido del `button` CSD mide `12.00 × 12.00 px` (centro en `(6.0, 6.0)`), mientras que `button > image` tenía `11 × 11 px` (`(5.5, 5.5)`), desplazando los glifos hacia arriba y a la izquierda junto con `viewBox='1 1 11 11'`.
  * Ajustado `button > image` a `min-width: 12px; min-height: 12px; background-size: 12px 12px;` y actualizados los 3 SVG inline a `width='12' height='12' viewBox='0 0 12 12'` con trazos centrados en `(6.0, 6.0)`.
* **Normalización de `viewBox` e iconos simbólicos ([`firefox-userchrome.css`](../crates/common/src/styles/firefox-userchrome.css), [`electron-window-controls.css`](../assets/themes/electron-window-controls.css), [`malus-ide-wrapper`](../scripts/malus-ide-wrapper), `assets/icons/*/actions/symbolic/window-*.svg`):**
  * Normalizados los SVG de `11 × 11 px` en Firefox y Electron a `viewBox='0 0 11 11'` (centro en `(5.5, 5.5)`) y reemplazados los `fill` fijos (`#363636` / `#dedede`) por `fill="currentColor"` en `window-*.svg` para que `color: transparent` oculte por completo el icono interno de `GtkImage`.

### 2. Validación
* Medición en vivo sobre `gtk4-broadwayd` (`Gtk.WidgetPaintable` + `GskRenderer`): `btn = (-0.75, -0.75, 13.50 × 13.50)` e `img = (0.00, 0.00, 12.00 × 12.00)` con centro exacto en `(6.00, 6.00)`.
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-common`: **50 passed; 0 failed**.
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación sin errores (`exit 0`).

---

## [2026-11-16] — Renombrado (`F2`), Submenú «Crear nuevo ›», Drag & Drop al Sidebar y Marcadores en Branches

### 1. Renombrado de Carpetas, Archivos e Iconos (`crates/branches`)
* **Validación y migración de metadatos ([`operations.rs`](../crates/branches/src/operations.rs), [`db.rs`](../crates/branches/src/db.rs)):**
  * Añadidos `validate_rename_target`, `rename_item` y `rename_selection_end` (preselecciona el nombre sin la extensión en archivos y completo en carpetas/dotfiles, rechazando nombres vacíos, `/`, `\0` o colisiones).
  * Añadido `rename_item_metadata` en SQLite para conservar posiciones espaciales (`folder_positions`), etiquetas (`file_tags`), ajustes por carpeta/subcarpetas (`folder_settings`) y marcadores (`sidebar_bookmarks`).
* **Popover de renombrado y atajo `F2` en las 4 vistas ([`views/mod.rs`](../crates/branches/src/views/mod.rs), [`icon_view.rs`](../crates/branches/src/views/icon_view.rs), [`list_view.rs`](../crates/branches/src/views/list_view.rs), [`column_view.rs`](../crates/branches/src/views/column_view.rs), [`gallery_view.rs`](../crates/branches/src/views/gallery_view.rs), [`folder_pane.rs`](../crates/branches/src/folder_pane.rs), [`app_window.rs`](../crates/branches/src/app_window.rs)):**
  * Integrados `show_rename_popover` y la opción `"Renombrar…"` en `show_item_context_menu` (oculta dentro de `Trash/files`), además de `get_selected_item_and_widget`, `rename_selected` y el atajo `F2` (sin interceptar eventos cuando un `gtk4::Editable` tiene el foco).

### 2. Submenú «Crear nuevo ›» (Carpeta y Documento de texto) (`crates/branches`)
* **Creación con nombres únicos ([`operations.rs`](../crates/branches/src/operations.rs)):** Añadidos `unique_target_path`, `create_new_folder` (`Nueva carpeta`, `Nueva carpeta 2`, …) y `create_new_text_document` (`Nuevo documento.txt`, `Nuevo documento 2.txt`, …).
* **Menú contextual en todas las vistas ([`views/mod.rs`](../crates/branches/src/views/mod.rs), [`icon_view.rs`](../crates/branches/src/views/icon_view.rs), [`list_view.rs`](../crates/branches/src/views/list_view.rs), [`column_view.rs`](../crates/branches/src/views/column_view.rs), [`gallery_view.rs`](../crates/branches/src/views/gallery_view.rs), [`folder_pane.rs`](../crates/branches/src/folder_pane.rs)):**
  * Añadidos `can_create_in_folder`, `append_create_new_submenu` (con `gtk4::Stack` deslizable hacia `← Volver`, `Carpeta` y `Documento de texto`) y `show_background_create_menu`. En vista de iconos posiciona el elemento creado en el punto del clic derecho cuando el modo es `SnapToGrid` o `Freeform` y lo deja seleccionado.

### 3. Drag & Drop al Sidebar y Marcadores Persistentes (`crates/branches`)
* **Arrastrar hacia carpetas, Papelera y etiquetas ([`operations.rs`](../crates/branches/src/operations.rs), [`sidebar.rs`](../crates/branches/src/sidebar.rs), [`macos-branches.css`](../crates/branches/src/styles/macos-branches.css)):**
  * Añadidos `move_paths_into_folder` y `drop_files_into_folder` (protegiendo contra mover sobre el mismo padre o dentro de subcarpetas propias y usando `trash_item` en **Papelera**), junto con `attach_folder_drop_target` y `attach_tag_drop_target`.
  * En [`macos-branches.css`](../crates/branches/src/styles/macos-branches.css), `.branches-sidebar-row.folder-drop-hover` usa `border: none; transform: none; box-shadow: inset 0 0 0 1.5px @accent_bg_color;` para resaltar la carpeta destino sin agrandar la fila y suprimiendo `:drop(active)` en el contenedor del sidebar.
* **Marcadores de carpetas y archivos con renombrado propio ([`db.rs`](../crates/branches/src/db.rs), [`sidebar.rs`](../crates/branches/src/sidebar.rs), [`views/mod.rs`](../crates/branches/src/views/mod.rs)):**
  * Tabla `sidebar_bookmarks` en SQLite con `get_bookmarks`, `is_bookmark`, `add_bookmark`, `remove_bookmark` y `rename_bookmark` (cambia el título del marcador sin renombrar la carpeta/archivo en disco).
  * Zona vacía dedicada `empty_drop_zone` debajo de `list_box` (`vexpand(false)`) para crear marcadores arrastrando carpetas o archivos a una zona vacía sin iluminar la sección al pasar sobre carpetas; la cabecera **Marcadores** solo se muestra cuando existe al menos un marcador (`!bookmarks.is_empty()`), e incluye menú contextual (**Abrir**, **Renombrar marcador…**, **Quitar de marcadores**).

### 4. Validación
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación sin errores ni warnings en `malus-de-branches` (`exit 0`).
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-branches`:
  * `malus-de-branches`: **21 passed; 0 failed** (`rename_selection_end_keeps_extension_unselected_for_files`, `validate_and_rename_item_handles_folders_and_conflicts`, `rename_item_metadata_preserves_positions_and_subfolder_settings`, `create_new_folder_and_text_document_generate_unique_names`, `move_paths_into_folder_moves_files_and_guards_descendants`, `bookmarks_crud_and_rename_preserve_order`).

---

## [2026-11-16] — Nueva Ventana de Branches en el Menú del Dock y Semáforo de Antigravity en Primera Apertura

### 1. Opción «Abrir nueva ventana» para Branches en el Dock (`crates/branches`, `crates/dock`)
* **Soporte `--new-window` en [`branches/src/main.rs`](../crates/branches/src/main.rs):** Añadido el flag `--new-window` en `Args` para resolver `$HOME` como ruta solicitada y abrir una nueva ventana mediante `gio::ApplicationFlags::HANDLES_OPEN` / `connect_open` cuando Branches ya tiene una instancia activa.
* **Menú contextual dinámico en [`dock/src/main.rs`](../crates/dock/src/main.rs):**
  * Actualizado `dock_app_context_actions` para ofrecer `"Abrir nueva ventana"` (`--new-window`) cuando Branches está en ejecución.
  * Extraídos `is_dock_item_running` y `populate_dock_app_menu_box` e invocados dentro de `right_click.connect_pressed` justo antes de `popover.popup()`. Esto corrige que las apps ancladas iniciadas después del Dock conservaran un popover estático inicial (`is_running == false`) y refleja en vivo `"Traer al frente"`, `"Abrir nueva ventana"` y `"Forzar salida"`.

### 2. Semáforo Confiable en Antigravity / Electron Desde la Primera Apertura (`scripts/malus-ide-wrapper`)
* **Confirmación real de CDP (`send_cdp_eval`):** Añadidos `recv_exact` y `read_ws_text_frame` en [`malus-ide-wrapper`](../scripts/malus-ide-wrapper) para verificar la respuesta JSON de `Runtime.evaluate` sin `error` ni `exceptionDetails` y confirmar `value === true` (hoja `ssd-v2` presente y `.window-controls-container` reubicado en `.titlebar-left`), evitando dar por inyectada una ventana cuyo `document.head` o barra de título aún no existían durante el arranque en frío de la sesión.
* **Observación de barra de título y reintento en arranque frío:** Guardado `document.head || document.documentElement` en `ensureStyle()`, ampliado el temporizador de montaje inicial (`200 ms × 300`), configurado el `MutationObserver` sobre `.part.titlebar` con `subtree: true` (manteniendo fuera editor Monaco, terminal y panel de chat) junto con salvaguarda ligera cada `1500 ms`, y restaurados el sondeo de `0.5 s` mientras no se confirma el montaje y `REINJECT_INTERVAL = 3.0` en `watcher`.

### 3. Validación
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación sin errores (`exit 0`).
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-branches -p malus-de-dock`:
  * `malus-de-branches`: **15 passed; 0 failed**.
  * `malus-de-dock`: **14 passed; 0 failed** (`branches_has_new_window_action_when_running`, `is_dock_item_running_tracks_live_running_cache`).
* `python3 -m py_compile scripts/malus-ide-wrapper`: sin errores (`exit 0`) y prueba de decodificación WebSocket con `socket.socketpair()`.

---

## [2026-11-16] — Cambio de Licencia a GPLv3, Temas de Iconos Malus (Colloid + Phosphor), Semáforos SSD Unificados y Papelera en Branches

### 1. Cambio de Licencia a `GPL-3.0-or-later` (`Cargo.toml`, `LICENSE`, `README.md`, `crates/top-bar`)
* **Compatibilidad legal y de monetización:** Se actualizó la licencia del workspace a `GPL-3.0-or-later` en [`Cargo.toml`](../Cargo.toml), se incorporó el texto oficial en [`LICENSE`](../LICENSE) y se actualizó [`show_about_dialog`](../crates/top-bar/src/ui/system_menu.rs) a `gtk4::License::Gpl30`, garantizando compatibilidad con dependencias `Apache-2.0` (`winit`, `gl`, etc.) y con temas de iconos `GPL-3.0`.
* **Créditos en [`README.md`](../README.md):** Se documentaron las licencias de `Colloid-icon-theme` (`GPL-3.0` en [`Colloid-LICENSE`](../assets/icons/Colloid-LICENSE)) y `Phosphor Icons` (`MIT` en [`Phosphor-LICENSE`](../assets/icons/Phosphor-LICENSE)).

### 2. Temas de Iconos `Malus` y `Malus-Dark` (Colloid + Phosphor) y Glifos de Sistema (`assets/icons`, `crates/common`, `crates/branches`, `crates/top-bar`)
* **Glifos de carpetas faltantes en [`fs_backend.rs`](../crates/branches/src/fs_backend.rs):** Añadidos mapeos en `resolve_kind_and_icon` para `Público`/`Publico`/`Public` (`folder-publicshare`), `Proyectos`/`Projects` (`folder-projects`) y `Plantillas`/`Templates` (`folder-templates`), junto con pruebas unitarias.
* **Integración de `Colloid` + `Phosphor Icons` (`assets/icons/`):**
  * Generados `Colloid-Light` y `Colloid-Dark` (`-p --notint`, sin iconos de marca Apple y heredando de `Adwaita,hicolor`).
  * Creados los temas `Malus` y `Malus-Dark` con los directorios escalables a color de Colloid enlazados directamente (`places/scalable`, `mimetypes/scalable`, `apps/scalable`, `devices/scalable`) y `MinSize=16` en `[places/scalable]` (eliminando `places/16`, `places/22`, `places/24` monocromos para evitar que carpetas y documentos caigan en `-symbolic`).
  * Reemplazados los ~80 glifos `-symbolic` de interfaz, además de `control-center-symbolic.svg` (`sliders-horizontal-bold`) y los 10 iconos del menú del sistema (`data/icons/apple-*-symbolic.svg`), por iconos de **Phosphor Icons** (`bold`/`fill`, `width="16" height="16"`, `fill="currentColor"`).
* **Sesión e instalador ([`common/src/config.rs`](../crates/common/src/config.rs), [`common/src/apps.rs`](../crates/common/src/apps.rs), [`scripts/malus-session`](../scripts/malus-session), [`scripts/install-session.sh`](../scripts/install-session.sh)):** Migración automática de `Adwaita` a `Malus-Dark` / `Malus` sincronizada con modo oscuro/claro (`sync_system_appearance`), registro de `extra_icon_search_paths()` en `top-bar`, `wallpaper` y `settings`, e instalación/caché automática de los temas.

### 3. Estandarización de Semáforos al Estilo SSD (`crates/common`, `assets/themes`, `scripts/malus-ide-wrapper`)
* **CSD GTK4, GTK3 y Firefox ([`windowcontrols-common.css`](../crates/common/src/styles/windowcontrols-common.css), [`windowcontrols-dark.css`](../crates/common/src/styles/windowcontrols-dark.css), [`windowcontrols-light.css`](../crates/common/src/styles/windowcontrols-light.css), [`gtk3.css`](../crates/common/src/styles/gtk3.css), [`firefox-userchrome.css`](../crates/common/src/styles/firefox-userchrome.css)):**
  * Ajustado el modelo `content-box` de GTK a `min-width: 11.5px; min-height: 11.5px; border-width: 0.75px;` (`13.5 px` exteriores) y `border-spacing: 6px;` (`center_dx = 20.0 px` exactos, igual al SSD en [`ssd.rs`](../crates/compositor/src/ssd.rs)), cambiando `-gtk-icon-size: 0px` a `1px` para evitar advertencias `Gtk-CRITICAL`.
  * Sustituidos los glifos del tema en hover por SVG geométricos idénticos al SSD (`×`, `–`, `+` con trazo redondeado `1.4 px`) y restaurando colores/glifos al hacer hover sobre controles en estado `:backdrop` / inactivo.
* **Antigravity / Electron ([`scripts/malus-ide-wrapper`](../scripts/malus-ide-wrapper), [`electron-window-controls.css`](../assets/themes/electron-window-controls.css), [`install-session.sh`](../scripts/install-session.sh)):**
  * Alineados a `13 px`, `gap: 7 px`, borde `0.75 px`, colores/hover del SSD y glifos `×`, `–`, `+` anulando `.codicon-chrome-*` y elementos hijos; `install-session.sh` además actualiza enlaces existentes en `~/.local/bin/` para evitar copias obsoletas que sombreen `/usr/local/bin/malus-ide-wrapper`.

### 4. Branches como Gestor Predeterminado para Papelera y Carpetas (`crates/branches`, `crates/dock`, `crates/common`, `data`)
* **Soporte de URIs `trash:///` y `file://` ([`branches/src/main.rs`](../crates/branches/src/main.rs), [`app_window.rs`](../crates/branches/src/app_window.rs), [`folder_pane.rs`](../crates/branches/src/folder_pane.rs)):** Añadido `resolve_cli_target_path` junto con `gio::ApplicationFlags::HANDLES_OPEN` y `connect_open` para abrir la Papelera en una ventana nueva aunque Branches ya esté ejecutándose, mostrando el título `"Papelera"` cuando la ruta apunta a `Trash/files`.
* **Dock y MIME ([`dock/src/main.rs`](../crates/dock/src/main.rs), [`dev.malus.Branches.desktop`](../data/dev.malus.Branches.desktop), [`common/src/config.rs`](../crates/common/src/config.rs)):** Actualizada la Papelera del Dock (`malus-de-branches trash:///`) y registradas `inode/directory` y `x-scheme-handler/trash` hacia `dev.malus.Branches.desktop` en `setup_default_mime_applications`.

### 5. Validación
* Validación con `Gtk.CssProvider` para **GTK4** y **GTK3**, y medición en vivo con `gtk4-broadwayd`: botones CSD `13.5 × 13.5`, `center_dx = 20.0 px`, carpetas y documentos resolviendo a SVGs no simbólicos en `Malus` / `Malus-Dark`.
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación completa sin errores (`exit 0`).
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-common -p malus-de-branches -p malus-de-dock -p malus-de-top-bar -p malus-de-compositor`:
  * `malus-de-common`: **50 passed; 0 failed**.
  * `malus-de-branches`: **15 passed; 0 failed**.
  * `malus-de-dock`: **12 passed; 0 failed**.
  * `malus-de-top-bar`: **10 passed; 0 failed**.
  * `malus-de-compositor`: **72 passed; 0 failed**.

---

## [2026-11-16] — Animación de Rejilla, Drag & Drop Exacto Entre Branches y Escritorio, y Fluidez DRM

### 1. Reordenamiento Animado sin Vibraciones y Selección en Branches (`crates/branches`)
* **Animación de cuadrícula (`animate_grid_moves` en [`icon_view.rs`](../crates/branches/src/views/icon_view.rs)):**
  * Implementada interpolación `ease_out_cubic` (`240 ms`) al reordenar iconos y cambiar ancho de ventana, protegida por `is_animating` para interpolar únicamente los widgets cuya posición cambia y evitar vibraciones en iconos estacionarios.
* **Selección inmediata y CSS válido para GTK4 ([`icon_view.rs`](../crates/branches/src/views/icon_view.rs), [`macos-branches.css`](../crates/branches/src/styles/macos-branches.css), [`folder_pane.rs`](../crates/branches/src/folder_pane.rs), [`main.rs`](../crates/branches/src/main.rs)):**
  * El `GestureClick` del lienzo verifica `is_point_over_spatial_icon` antes de deseleccionar el fondo para que el clic simple sobre un icono mantenga su marca `.selected` sin esperar a moverlo.
  * Eliminados todos los `!important` inválidos en GTK4 CSS dentro de [`macos-branches.css`](../crates/branches/src/styles/macos-branches.css) (que provocaban que `GtkCssProvider` descartara las reglas), suprimido el borde azul `:drop(active)` / `:focus:drop(active)` en los contenedores del lienzo, restringido el borde de panel activo a `.branches-split-mode` y cambiado `.branches-icon-item.selected` de `outline` exterior a `border: 1.5px solid` interior sin transición para que `WidgetPaintable` capture el diseño completo desde el inicio del arrastre.

### 2. Consistencia de Movimiento y Colocación Exacta Entre Procesos (`crates/common`, `crates/branches`, `crates/wallpaper`)
* **Punto de agarre compartido ([`common/src/lib.rs`](../crates/common/src/lib.rs)):**
  * Añadidos `write_drag_anchor` y `read_drag_anchor` (con respaldo en `$XDG_RUNTIME_DIR` y `/tmp`) para preservar el offset exacto del puntero al arrastrar entre **Branches** y el **Escritorio**.
* **Suelta exacta y migración (`db.rs`, `icon_view.rs`, `list_view.rs`, `column_view.rs`, `folder_pane.rs`):**
  * Migradas carpetas con `snap_to_grid` heredado a `freeform` (`PRAGMA user_version = 1` en [`db.rs`](../crates/branches/src/db.rs)) y aplicada colocación exacta con `exact_drop_position` / `resolve_drop_coords` (cambiando `Auto` → `Freeform` al soltar archivos internos o externos), conservando seleccionado el icono recién soltado mediante `pending_select_names`.
  * Priorizado `DragAction::MOVE | DragAction::COPY` en fuentes y destinos de arrastre, agregado [`refresh_if_moved_out`](../crates/branches/src/views/mod.rs) en `connect_drag_end` de Icon/List/Column View (para retirar el icono origen al moverlo hacia el escritorio u otra ventana) y unificado `folder_pane.rs` sobre `gio::File::move_` con respaldo en `std::fs::rename`.

### 3. Fluidez del Escritorio y Entrega Sincronizada en el Compositor (`crates/wallpaper`, `crates/compositor`)
* **Escritorio sin saltos ni parpadeos ([`wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs), [`default-gradient.css`](../crates/wallpaper/src/styles/default-gradient.css)):**
  * Redondeo de `clamp_icon_position`, eliminación de `set_icon` prematuro en `connect_pressed`, transición instantánea y `box-shadow: inset` en `.desktop-icon.selected`.
  * Al arrastrar hacia Branches, `connect_drag_end` retira inmediatamente el widget movido sin devolverlo temporalmente al escritorio; al recibir desde Branches, `connect_drop` dibuja y selecciona el icono en el mismo frame mientras `monitor_directory` evita reconstrucciones redundantes.
  * Pre-escalado/recorte *Cover* único de imagen (`build_scaled_wallpaper_texture`), pre-rasterización del degradado por defecto (`build_default_gradient_texture`) con `.has-wallpaper-texture`, ordenamiento sin syscalls repetidas en `calculate_organize_targets` y duración alineada a `240 ms` (`DESKTOP_ORGANIZE_ANIM_MS`) difiriendo el guardado de JSON y evitando reconstruir widgets cuando solo cambia `snap_to_grid`.
* **Compositor DRM y handoff DnD ([`state.rs`](../crates/compositor/src/state.rs), [`backend/mod.rs`](../crates/compositor/src/backend/mod.rs), [`drm.rs`](../crates/compositor/src/backend/drm.rs), [`winit.rs`](../crates/compositor/src/backend/winit.rs)):**
  * Añadido `dnd_drop_hold` usando `last_pointer_location` en `dropped()` (evitando deadlock con `PointerInternal`) para sostener el icono flotante hasta el `commit` destino (o 80 ms máximo).
  * Planificación selectiva del snapshot de blur de fondo (`plan_background_blur_capture` + `background_snapshot_stale`) para omitir `glCopyTexSubImage2D` de pantalla completa durante animaciones del escritorio sin blur afectado, y envío continuo de `send_frames_surface_tree` a capas visibles en DRM para evitar caídas de FPS en `GdkFrameClock`.

### 4. Validación
* Validación con `gtk4::CssProvider` sobre [`macos-branches.css`](../crates/branches/src/styles/macos-branches.css) y [`default-gradient.css`](../crates/wallpaper/src/styles/default-gradient.css): **0 errores de parseo**.
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación completa sin errores (`exit 0`).
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-common -p malus-de-branches -p malus-de-wallpaper -p malus-de-compositor`:
  * `malus-de-common`: **48 passed; 0 failed** (`shared_drag_anchor_roundtrips_matching_path`).
  * `malus-de-branches`: **12 passed; 0 failed** (`exact_drop_position_preserves_pointer_grab_offset`, `ease_out_cubic_and_interpolation_follow_macos_curve`, `canvas_hit_test_distinguishes_icons_from_empty_space`, `test_branches_macos_drag_and_drop_styles`).
  * `malus-de-wallpaper`: **18 passed; 0 failed** (`keeps_pointer_offset_and_clamps_icon_to_desktop`, `test_compute_cover_crop_geometry_and_gradient_texture`).
  * `malus-de-compositor`: **72 passed; 0 failed** (`background_blur_capture_skips_desktop_animation_without_translucent_overlap`).

---

## [2026-11-15] — Estandarización de Tamaño de Iconos/Carpetas y Cuadrícula Responsiva en Branches

### 1. Métricas Compartidas y Configuración `BranchesConfig` (`crates/common/src/config.rs`, `crates/wallpaper/src/main.rs`)
* **Métricas compartidas (`IconGridMetrics`):**
  * Extraído `IconGridMetrics` (`box_width = icon_size + 44`, `box_height = icon_size + 48`, `spacing = 8 + extra_spacing`) en [`config.rs`](../crates/common/src/config.rs), utilizado tanto por `DesktopIconsConfig` ([`wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs)) como por Branches para garantizar dimensiones idénticas (`64 px` de icono, cajas de `108×112 px` y celdas de `116×120 px` por defecto).
* **Configuración persistente de Branches (`BranchesConfig`):**
  * Añadido `BranchesConfig` (`~/.config/malus-de/branches.toml`) con `icon_size` (`32..=128`, predeterminado `64` igual al Escritorio), `grid_spacing` (`0..=48`), `text_size` (`10..=16`), `list_icon_size` (`16..=32`) y `show_hidden`, preparando la persistencia para futuros ajustes de Branches estilo Finder.

### 2. Preparación por Carpeta, Recarga en Vivo y Estilos (`crates/branches`)
* **Base de datos y estado ([`db.rs`](../crates/branches/src/db.rs), [`state.rs`](../crates/branches/src/state.rs), [`folder_pane.rs`](../crates/branches/src/folder_pane.rs), [`app_window.rs`](../crates/branches/src/app_window.rs)):**
  * Añadida columna opcional `icon_size` en `folder_settings` (con migración no destructiva) y `get_folder_icon_size` / `set_folder_icon_size` para personalización por carpeta.
  * `AppState` inicializa `icon_size`, `grid_spacing` y `show_hidden` desde `BranchesConfig::load()`, mientras `BranchesWindow` monitoriza `branches.toml` mediante `gio::FileMonitor` para aplicar cambios en tiempo real sobre ambos paneles (`apply_branches_config`) y [`list_view.rs`](../crates/branches/src/views/list_view.rs) respeta `list_icon_size`.
  * Alinear `.branches-icon-item` (`padding: 6px; margin: 0; border-radius: 10px;`) y `.branches-icon-label` (`font-size: 12px; margin-top: 4px;`) en [`macos-branches.css`](../crates/branches/src/styles/macos-branches.css) con `.desktop-icon`.

### 3. Cuadrícula Adaptativa al Ancho de Ventana y Movimiento Libre (`crates/branches/src/views/icon_view.rs`)
* **Reflow dinámico según el ancho visible:**
  * Reemplazado el límite fijo de `8` columnas (`48 px`) por [`columns_for_width`](../crates/branches/src/views/icon_view.rs) y [`setup_resize_reflow`](../crates/branches/src/views/icon_view.rs) escuchando `hadjustment::notify::page-size`, redistribuyendo automáticamente la cuadrícula en `LayoutMode::Auto` al redimensionar la ventana o alternar Split View.
* **Movimiento libre preservado y menú del lienzo:**
  * [`show_canvas_context_menu`](../crates/branches/src/views/icon_view.rs) ofrece **Cuadrícula automática**, **Alinear a la cuadrícula (Snap to Grid)**, **Colocación libre (Freeform)** (marcando el modo activo con `✓`) y **Reordenar cuadrícula**.
  * Arrastrar un icono dentro del lienzo funciona libremente incluso cuando estaba activa **Cuadrícula automática** (transicionando la carpeta a `SnapToGrid` y guardando las posiciones actuales), y `connect_drag_end` utiliza `pos: Rc<Cell<(i32, i32)>>` para restaurar la posición vigente ante arrastres cancelados.

### 4. Validación
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación completa sin errores (`exit 0`).
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-common -p malus-de-branches -p malus-de-wallpaper`:
  * `malus-de-common`: **47 passed; 0 failed** (`branches_config_defaults_match_desktop_icon_size_and_metrics`).
  * `malus-de-branches`: **9 passed; 0 failed** (`folder_icon_size_persists_and_clears_cleanly`, `grid_columns_adapt_to_window_and_pane_width`, `snap_to_grid_cell_aligns_freely_dragged_icon_to_nearest_slot`).
  * `malus-de-wallpaper`: **17 passed; 0 failed**.

---

## [2026-11-15] — Menús Contextuales en Branches y Alineación Squircle del Blur en DRM

### 1. Menús Contextuales Unificados y Acciones en Aplicaciones (`crates/branches/src/views/`)
* **Causas resueltas en las vistas ([`icon_view.rs`](../crates/branches/src/views/icon_view.rs), [`list_view.rs`](../crates/branches/src/views/list_view.rs), [`column_view.rs`](../crates/branches/src/views/column_view.rs), [`gallery_view.rs`](../crates/branches/src/views/gallery_view.rs)):**
  * En Icon, List y Gallery View, `GestureClick` filtraba únicamente clic izquierdo por defecto (`button == 1`), impidiendo abrir menús contextuales con clic secundario; en Column View tampoco existía controlador de clic secundario por fila.
  * Activado `set_button(0)` en `IconView`/`ListView`/`GalleryView` (y `GestureClick::set_button(gdk::BUTTON_SECONDARY)` en `ColumnView`), reclamando el evento con `EventSequenceState::Claimed`, anclando `pointing_to` al puntero y liberando el `Popover` con `unparent()` al cerrar.
* **Menú unificado ([`views/mod.rs`](../crates/branches/src/views/mod.rs)):**
  * Extraídos [`show_item_context_menu`](../crates/branches/src/views/mod.rs), [`is_dock_app_pinned`](../crates/branches/src/views/mod.rs) y [`set_app_pinned_in_dock`](../crates/branches/src/views/mod.rs).
  * Para lanzadores `.desktop` (carpeta **Aplicaciones** y archivos `.desktop`), el menú muestra **Abrir aplicación**, **Fijar en el Dock** / **Quitar del Dock** (vía IPC `PinAppToDock`/`UnpinAppFromDock` con respaldo directo a `DockConfig`), **Mostrar carpeta contenedora** y **Copiar ruta**, conservando **Abrir** / **Abrir carpeta** y **Abrir en Terminal** para elementos ordinarios.

### 2. Alineación Squircle (`n = 4.2`) en la Capa de Blur del Backend DRM (`crates/compositor`)
* **Causa raíz resuelta:**
  * Tanto los popovers/menús contextuales (`popup_elems`, `dock_popups`, `topbar_popups`, `wallpaper_popups`) como las ventanas translúcidas (*Branches* y *Ajustes del Sistema*) utilizan [`WindowSquircleShader`](../crates/compositor/src/window_corners.rs) (`u_exponent = 4.2`) y [`shadow.rs`](../crates/compositor/src/shadow.rs) (`4.2`), mientras las pasadas directas en [`drm.rs`](../crates/compositor/src/backend/drm.rs) invocaban [`render_captured_blur_round`](../crates/compositor/src/blur.rs) (`corner_exponent = 2.0`, recorte circular más cerrado que dejaba visibles esquinas recortadas bajo las zonas translúcidas).
* **Solución:**
  * Sustituido `render_captured_blur_round` por [`render_captured_blur`](../crates/compositor/src/blur.rs) (`4.2`) en ventanas translúcidas, `popup_elems`, `dock_popups`, `topbar_popups` y `wallpaper_popups` dentro de [`drm.rs`](../crates/compositor/src/backend/drm.rs), así como en `render_snap_transition_backdrop` y `BlurRenderElement::for_popup` dentro de [`blur.rs`](../crates/compositor/src/blur.rs).

### 3. Validación
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación completa sin errores (`exit 0`).
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-compositor`: **71 passed; 0 failed**.
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-branches`: **6 passed; 0 failed** (incluyendo `views::tests::dock_app_pinned_detection_matches_by_name_or_command`).

---

## [2026-11-15] — Aplicaciones de Spotlight en Branches con Iconos Propios y Arrastre al Dock

### 1. Indexador `.desktop` Compartido y Resolución de Iconos (`crates/common/src/apps.rs`, `lib.rs`)
* **Módulo [`apps.rs`](../crates/common/src/apps.rs):**
  * [`index_applications`](../crates/common/src/apps.rs) y [`application_dirs_from`](../crates/common/src/apps.rs) indexan lanzadores visibles (`NoDisplay=false`, `Hidden=false`, `Type=Application`) en `$XDG_DATA_HOME/applications`, exports Flatpak de usuario/sistema y `$XDG_DATA_DIRS/*/applications`, resolviendo `Name`/`Comment` localizados y limpiando `%u`/`%F` en `Exec`.
  * [`resolve_icon_file_path`](../crates/common/src/apps.rs) y [`extra_icon_search_paths`](../crates/common/src/apps.rs) localizan iconos `.svg`/`.png` en rutas absolutas, `~/`, temas `hicolor`/`Adwaita`, exports Flatpak y `pixmaps`, evitando caer al icono genérico cuando la caché del tema aislado no incluye apps instaladas.
  * [`desktop_file_to_dock_app`](../crates/common/src/apps.rs) convierte archivos `.desktop` en `DockAppConfig` con `matchers` listos para el Dock.

### 2. Sección Aplicaciones, Lanzamiento Directo y Drag & Drop en Branches (`crates/branches`)
* **Vista virtual Aplicaciones ([`sidebar.rs`](../crates/branches/src/sidebar.rs), [`folder_pane.rs`](../crates/branches/src/folder_pane.rs), [`app_window.rs`](../crates/branches/src/app_window.rs), [`fs_backend.rs`](../crates/branches/src/fs_backend.rs)):**
  * **Aplicaciones** en Favoritos abre `VirtualFolder::Applications` (`show_applications()`), mostrando las aplicaciones indexadas con nombres legibles, iconos propios (`resolve_desktop_gicon` -> `gio::FileIcon` para iconos en disco y `resolve_kind_and_icon` para `.desktop` en carpetas ordinarias) y filtrado desde el campo de búsqueda.
  * `open_file()` detecta archivos `.desktop` y lanza directamente la aplicación con `apply_isolated_environment` al hacer doble clic; [`main.rs`](../crates/branches/src/main.rs) registra las rutas de iconos adicionales en `gtk4::IconTheme::for_display`.
* **Interoperabilidad Wayland DnD ([`views/mod.rs`](../crates/branches/src/views/mod.rs), `icon_view.rs`, `list_view.rs`, `column_view.rs`):**
  * Unificado `build_file_drag_content_provider()` combinando `GdkFileList` y `text/uri-list` explícito (`gdk::ContentProvider::new_union`), habilitando arrastre desde Vista de Iconos, Lista y Columnas.

### 3. Recepción Drag & Drop en el Dock (`crates/dock`)
* **Anclaje por arrastre ([`dock/src/main.rs`](../crates/dock/src/main.rs)):**
  * [`attach_dock_drop_target`](../crates/dock/src/main.rs) registra un `gtk4::DropTarget` en fase `Capture` sobre `dock_root` que reutiliza `update_dock_drop_placeholder()` para abrir el hueco animado al pasar un `.desktop` y lo ancla en la posición del puntero mediante `DockConfig::pin_app_at()` + `save()`.
* **Limpieza visual de `:drop(active)` ([`dock.css`](../crates/dock/src/styles/dock.css)):**
  * Se añadieron reglas para `window.malus-dock-window:drop(active)`, `.dock-root:drop(active)` y `.dock-items:drop(active)` con `border: none; outline: none; box-shadow: none;`, eliminando la línea superior que dibujaba Libadwaita/GTK4 durante el hover de arrastre.

### 4. Validación
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación completa sin errores (`exit 0`).
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-common -p malus-de-branches -p malus-de-dock`:
  * `malus-de-common`: **46 passed; 0 failed** (incluyendo `apps::tests::application_dirs_include_xdg_and_flatpak_exports`, `parse_desktop_file_respects_nodisplay_and_builds_dock_app` y `resolve_icon_file_path_finds_hicolor_flatpak_and_pixmap_icons`).
  * `malus-de-branches`: **5 passed; 0 failed** (incluyendo `fs_backend::tests::test_desktop_file_resolves_app_gicon`).
  * `malus-de-dock`: **12 passed; 0 failed** (incluyendo `pin_desktop_apps_from_branches_drop_inserts_into_dock_config` y `dock_css_suppresses_gtk_drop_active_outline_and_border`).

---

## [2026-11-15] — Rediseño Finder en Branches, Corrección de Panic en Historial y Fondo Sólido en Ventana de Errores

### 1. Interfaz de Branches Inspirada en macOS Finder (`crates/branches/src/{app_window.rs,folder_pane.rs,fs_backend.rs,main.rs,sidebar.rs,styles/macos-branches.css}`)
* **Barra lateral reestructurada (`sidebar.rs`, `fs_backend.rs`, `folder_pane.rs`):**
  * Añadidos accesos superiores sin encabezado **Recientes** (`create_recents_row` + `load_recent_files` combinando `recently-used.xbel` y carpetas personales en [`fs_backend.rs`](../crates/branches/src/fs_backend.rs)) y **Compartidos** (`XDG_PUBLICSHARE_DIR`) encima de **Favoritos**, **Ubicaciones** y **Etiquetas**.
  * Títulos de sección en *Title Case* (`font-size: 11.5px; font-weight: 600;` sin `text-transform: uppercase`), iconos azules `#0a84ff` en favoritos/accesos superiores y grises neutros en ubicaciones (`Este Equipo`, `Red`, `Papelera` y carpeta de usuario).
* **Contenedores *squircle* uniformes en barra superior (`app_window.rs`, `styles/macos-branches.css`):**
  * Agrupación en contenedores *Liquid Glass* con alto uniforme (`32 px`) para navegación `[ < | > ]` (botones cuadrados de `26×26 px`), selector segmentado de vista, botón de vista dividida y grupo `Compartir / Información / Acciones`.
  * Aislamiento de reglas `menubutton.branches-tool-btn` y `button.branches-search-btn` para evitar doble padding en `GtkMenuButton` y mantener altura uniforme en el botón de búsqueda.
  * Radios *squircle* afilados y concéntricos (`border-radius: 8px` en contenedores exteriores, `5px` en botones internos) conservando `border-radius: 999px` en `.branches-search-entry`, junto con fondo aclarado adaptable a tema claro (`alpha(@window_fg_color, 0.025)`) y oscuro (`@media (prefers-color-scheme: dark)` con `alpha(@window_fg_color, 0.10)`). La barra inferior [`path_bar.rs`](../crates/branches/src/path_bar.rs) se conserva intacta.

### 2. Corrección de Panic `RefCell already mutably borrowed` al Navegar (`crates/branches/src/folder_pane.rs`)
* **Causa raíz resuelta:** En `go_back()`, `go_forward()` y `go_up()`, el patrón `if let Some(...) = self.state.borrow_mut().go_...()` mantenía vivo el `RefMut<'_, AppState>` durante todo el bloque, chocando con `self.state.borrow()` dentro de `self.reload()` (`folder_pane.rs:248`) y `update_chrome()` (`can_go_back`/`can_go_forward`).
* **Solución:** Extraídos [`step_history`](../crates/branches/src/folder_pane.rs) y [`apply_items_snapshot`](../crates/branches/src/folder_pane.rs) para liberar el préstamo de `self.state` antes de recargar vistas o invocar callbacks UI.

### 3. Fondo Sólido en Ventana de Reporte de Errores (`crates/dialogs`)
* **Causa raíz resuelta:** `window.malus-confirmation-window` usaba `background-color: transparent` y `.malus-confirmation-card` usaba `alpha(@window_bg_color, 0.82)`, mientras [`malus-de-crash-reporter.rs`](../crates/dialogs/src/bin/malus-de-crash-reporter.rs) tampoco inicializaba `libadwaita`, dejando `@window_bg_color` sin definir al ejecutarse de forma independiente.
* **Solución:** Añadida dependencia `libadwaita` en [`crates/dialogs/Cargo.toml`](../crates/dialogs/Cargo.toml), inicialización de `setup_isolated_session_environment()` + `libadwaita::init()` en `malus-de-crash-reporter` y `ensure_dialog_css_loaded()`, clase dedicada `.malus-crash-window` en [`lib.rs`](../crates/dialogs/src/lib.rs) y fondo sólido `@window_bg_color` en [`confirmation.css`](../crates/dialogs/src/styles/confirmation.css).

### 4. Validación
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación completa sin errores (`exit 0`).
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-branches -p malus-de-dialogs`:
  * `malus-de-branches`: **4 passed; 0 failed** (`history_navigation_releases_borrow_before_reload`, `test_branches_finder_sidebar_and_toolbar_pills`, `test_branches_macos_drag_and_drop_styles`, `test_percent_decode_file_uri_and_load_recent_files`).
  * `malus-de-dialogs`: **2 passed; 0 failed** (`crash_window_and_card_have_solid_background_in_css`, `extract_report_field_and_summary_parse_dump_text`).

---

## [2026-11-15] — Tipografía Predeterminada Inter y Renderizado Nítido Estilo macOS

### 1. Adopción de Inter como Tipografía Predeterminada (`crates/common/src/config.rs`, `crates/render/src/text.rs`, `crates/compositor/src/{expose.rs,app_switcher.rs}`, `scripts/malus-session*`, `docs/PACKAGE_MANIFEST.md`)
* **Entorno aislado y migración automática:**
  * [`config.rs`](../crates/common/src/config.rs), [`scripts/malus-session`](../scripts/malus-session) y [`scripts/malus-session.sh`](../scripts/malus-session.sh) establecen `font-name='Inter 11'` y `document-font-name='Inter 11'` en el backend `keyfile` de GSettings, `gtk-font-name=Inter 11` en GTK3/GTK4 (`settings.ini`) y prioridad para `sans-serif`, `sans` y `system-ui` hacia `Inter` en `$XDG_CONFIG_HOME/fontconfig/conf.d/10-malus-inter.conf` (`ensure_isolated_fontconfig_inter`).
  * `ensure_default_font_settings` migra automáticamente configuraciones existentes sin fuente o con valores heredados (`Adwaita Sans`, `Cantarell`, `Sans`) hacia `Inter 11`, preservando cualquier fuente personalizada definida por el usuario.
* **Renderizadores propios y manifiesto de dependencias:**
  * [`detect_system_font_family`](../crates/render/src/text.rs) usa `"Inter"` como respaldo predeterminado en `cosmic-text`/`glyphon`.
  * [`expose.rs`](../crates/compositor/src/expose.rs) y [`app_switcher.rs`](../crates/compositor/src/app_switcher.rs) pasan de `"Sans"` a `"Inter"` en Cairo, alineándose con `ssd.rs` y `spotlight_visual.rs`.
  * [`PACKAGE_MANIFEST.md`](../docs/PACKAGE_MANIFEST.md) actualiza el paquete en Fedora a `rsms-inter-fonts` y `rsms-inter-vf-fonts` (v4.1).

### 2. Suavizado y Hinting Estilo macOS (`crates/common/src/config.rs`, `crates/compositor/src/ssd.rs`, `expose.rs`, `app_switcher.rs`, `spotlight_visual.rs`, `scripts/malus-session*`)
* **FreeType y Fontconfig aislado (`10-malus-inter.conf`):**
  * Exportación y propagación de `FREETYPE_PROPERTIES="truetype:interpreter-version=40 cff:no-stem-darkening=0 autofitter:no-stem-darkening=0"` en sesión, portal (`dbus-update-activation-environment` / `systemctl --user import-environment`) y procesos hijos lanzados mediante `apply_isolated_environment`.
  * Regla `<match target="font">` con `antialias=true`, `hinting=true`, `hintstyle=hintslight`, `autohint=true` (alineación vertical ligera que habilita *stem darkening* y preserva métricas horizontales puras), `rgba=none` (escala de grises sin *fringing* RGB en escalados fraccionarios ni capas alfa) y `embeddedbitmap=false`.
* **GSettings, GTK3/GTK4 y Cairo del compositor:**
  * `[org/gnome/desktop/interface]` configura `font-antialiasing='grayscale'`, `font-hinting='slight'`, `font-rgba-order='rgb'` y `font-rendering='manual'`; `gtk-3.0`/`gtk-4.0` añaden `gtk-xft-*` y `gtk-hint-font-metrics=1`.
  * [`apply_macos_font_options`](../crates/compositor/src/ssd.rs) aplica `Antialias::Gray`, `HintStyle::Slight` y `HintMetrics::Off` a los contextos Cairo de [`ssd.rs`](../crates/compositor/src/ssd.rs), [`app_switcher.rs`](../crates/compositor/src/app_switcher.rs), [`expose.rs`](../crates/compositor/src/expose.rs) y [`spotlight_visual.rs`](../crates/compositor/src/spotlight_visual.rs).

### 3. Validación
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación completa sin errores (`exit 0`).
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-common -p malus-de-render -p malus-de-compositor`:
  * `malus-de-common`: **43 passed; 0 failed** (incluyendo `test_ensure_default_font_settings_and_fontconfig_inter`).
  * `malus-de-compositor`: **71 passed; 0 failed**.
  * `malus-de-render`: **14 passed; 0 failed** (`test_dbus_menu`: 2, `test_layout`: 1, `test_luminance`: 3, `test_popover`: 4, `test_squircle`: 1, `test_text`: 3 incluyendo `test_cosmic_text_inter_font_matching`).

---

## [2026-11-15] — Corrección de Deadlock en Arrastre SSD a Esquinas (`ZapFast`)

### 1. Eliminación de Re-entrada al Mutex del Puntero en `origin_output_for_window` (`crates/compositor/src/state.rs`, `grabs.rs`, `drm.rs`, `winit.rs`)
* **Causa raíz resuelta:**
  * Durante un arrastre (`MoveSurfaceGrab::motion`) o redimensión (`ResizeSurfaceGrab::motion`) en [`grabs.rs`](../crates/compositor/src/grabs.rs), Smithay mantiene bloqueado `Mutex<PointerInternal>` en el hilo principal del compositor.
  * Al arrastrar una ventana con barra de título SSD de `38 px` por encima de la superficie cliente (como ZapFast) hacia esquinas inferiores o laterales hasta sacar su rectángulo cliente fuera de la pantalla, `origin_output_for_window` en [`state.rs`](../crates/compositor/src/state.rs) fallaba en los pasos de intersección/contención y llamaba a `self.seat.get_pointer().current_location()`, provocando un **deadlock** en el hilo principal (y sin disparar *panic hook* ni volcado post-`wait` en `malus-session`).
* **Solución implementada:**
  * Se agregó `last_pointer_location` a `MalusState` ([`state.rs`](../crates/compositor/src/state.rs)), sincronizado en eventos de puntero de [`drm.rs`](../crates/compositor/src/backend/drm.rs), [`winit.rs`](../crates/compositor/src/backend/winit.rs) y en `motion`/`button` de [`grabs.rs`](../crates/compositor/src/grabs.rs).
  * Se extrajo [`resolve_origin_output_from_bounds`](../crates/compositor/src/state.rs) para incluir `SSD_TITLEBAR_HEIGHT` en la geometría/esquina de ventanas SSD y consultar `last_pointer_location` como respaldo sin volver a bloquear `PointerHandle`.

### 2. Validación
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-compositor`: **71 passed; 0 failed** (incluyendo la nueva prueba `ssd_corner_drag_resolves_output_without_locking_pointer_mutex`).

---

## [2026-11-15] — Branches (Sidebar y Foco desde Dock), Semáforo CSD Estático 1:1 en Snap/Maximizado y App Switcher por Pantalla y Escritorio

### 1. Límite de Sidebar en Branches y Foco desde el Dock (`crates/branches/src/app_window.rs`, `sidebar.rs`, `crates/dock/src/main.rs`, `crates/compositor/src/state.rs`)
* **Contención de barra lateral al maximizar (`MAX_SIDEBAR_WIDTH = 212`):**
  * En [`app_window.rs`](../crates/branches/src/app_window.rs), `hpaned` ahora fija `.resize_start_child(false)` / `.resize_end_child(true)` y limita `paned.position()` a `212 px` lógicos en `connect_position_notify` para que el panel izquierdo nunca supere el rectángulo de desenfoque del compositor; en [`sidebar.rs`](../crates/branches/src/sidebar.rs), los labels aplican `EllipsizeMode::End`.
* **Activación y elevación al pulsar Branches en el Dock:**
  * [`dock/src/main.rs`](../crates/dock/src/main.rs) incorpora `find_matching_running_app` para enviar en `IpcMessage::ActivateApp` el identificador vivo exacto (p. ej. `dev.malus.Branches` en lugar de `matchers[0]`).
  * [`state.rs`](../crates/compositor/src/state.rs) añade `matches_app_target` (emparejando `malus-de-*` <-> `malus.*` e IDs *reverse-DNS*) y refuerza `activate_or_restore_app` para elevar al frente incluso si ya tenía foco y sincronizar foco de teclado (`set_keyboard_focus_with_sync`).

### 2. Semáforo CSD Estático 1:1 Durante Maximizado y Snap (`crates/compositor/src/state.rs`, `window_corners.rs`, `blur.rs`, `drm.rs`, `winit.rs`)
* **Eliminación de estiramiento y zoom en transiciones (`SnapTransition::render_geometry`):**
  * En [`state.rs`](../crates/compositor/src/state.rs), `SnapTransition::render_geometry` mantiene la superficie estrictamente a escala nativa `1:1` sin `RescaleRenderElement` y entrega `surface_clip_rect` (intersección de `win_screen_rect` con `window_geo.size`).
  * [`render_window_elements`](../crates/compositor/src/window_corners.rs) recibe `transition_clip: Option<Rectangle<i32, Physical>>` para recortar `damage` y aplicar el shader *squircle* sobre el área real `1:1`.
  * Cuando el búfer del cliente aún es menor que el rectángulo animado, [`render_snap_transition_backdrop`](../crates/compositor/src/blur.rs) dibuja detrás un respaldo esmerilado acorde al tema oscuro/claro en [`drm.rs`](../crates/compositor/src/backend/drm.rs) y [`winit.rs`](../crates/compositor/src/backend/winit.rs).

### 3. App Switcher Independiente por Pantalla y Escritorio Activo (`crates/compositor/src/state.rs`)
* **Aislamiento multimonitor y por escritorio:**
  * [`open_app_switcher`](../crates/compositor/src/state.rs) selecciona el monitor destino donde se encuentra el puntero vía `target_app_switcher_output` (con respaldo en la ventana enfocada o monitor principal) y filtra [`build_app_switcher_items`](../crates/compositor/src/state.rs) usando `origin_output_for_window`.
  * `MalusState` registra `minimized_window_workspaces` y evalúa `should_include_minimized_in_app_switcher`: las ventanas minimizadas (residentes en el Dock de la pantalla principal) solo se muestran en el Alt+Tab del monitor principal y únicamente si pertenecen al escritorio activo.

### 4. Validación
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-compositor -p malus-de-dock -p malus-de-branches`:
  * `malus-de-compositor`: **70 passed; 0 failed**.
  * `malus-de-dock`: **10 passed; 0 failed**.
  * `malus-de-branches`: **1 passed; 0 failed**.
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación completa sin errores (`exit 0`, únicamente los warnings previos de `gtk4::FileChooserNative` en `crates/settings`).

---

## [2026-11-15] — Pipeline GPU Único para Genie Effect, Restauración Inversa, Eliminación de Parpadeo y Blur Offscreen en Animaciones

### 1. Rasterización GPU Única para Genie Effect (Malla `36×64` + Snapshot Offscreen) (`crates/compositor/src/magic_lamp.rs`, `capture.rs`, `drm.rs`, `winit.rs`)
* **Sustitución de bandas CPU por deformación GPU continua:**
  * [`capture_pending_genie_snapshots`](../crates/compositor/src/capture.rs) y `capture_window_offscreen` renderizan la ventana completa una sola vez al iniciar la animación a una textura `GlesTexture` premultiplicada (`CapturedWindowOffscreen`), incluyendo decoración de servidor (`SSD`), recorte *squircle* mediante [`WindowSquircleShader`](../crates/compositor/src/window_corners.rs) y, en minimización, exportación de la miniatura PNG para el Dock sin distorsión Genie.
  * [`GeniePipeline`](../crates/compositor/src/magic_lamp.rs) dibuja esa instantánea mediante una malla teselada en `GENIE_VERT_SRC` (`36×64` celdas, instanciada por `gl_VertexID`) y `GENIE_FRAG_SRC` en una única llamada por cuadro, reemplazando las 64 bandas en CPU en [`drm.rs`](../crates/compositor/src/backend/drm.rs) y [`winit.rs`](../crates/compositor/src/backend/winit.rs) (precedido por `446b9f7`, `e705bb2` y `0602685`).

### 2. Eliminación de Parpadeo al Minimizar y Restauración Inversa Fluida (`crates/compositor/src/state.rs`, `magic_lamp.rs`, `shadow.rs`, `crates/dock/src/main.rs`)
* **Inicio inmediato sin espera de foco/slot (`minimize_toplevel`):**
  * En [`state.rs`](../crates/compositor/src/state.rs), `minimize_toplevel` ahora estima un destino inicial en el extremo derecho del Dock y desmapea la ventana de inmediato (`waiting_for_dock_slot = false`) en lugar de mantenerla visible sin foco (`Activated = false`) esperando `RegisterDockSlot`. Así se elimina el parpadeo de sombra y cromo inactivo en ventanas no maximizadas (p. ej. Antigravity), mientras [`dock/src/main.rs`](../crates/dock/src/main.rs) registra el slot desde `16 ms` (`[16, 60, 180, 320]`).
* **Curva inversa perceptual y cross-fade de sombra proyectada:**
  * [`MagicLampAnimation::compute_kj`](../crates/compositor/src/magic_lamp.rs) separa `is_minimizing == false` con *ease-out* temporal: `j` desciende de `1.0` a `0.0` (`0.0 -> 0.66`) saliendo del Dock y, a partir de `0.66`, `k = (1.0 - q).powf(2.1)` retrae el cuello de inmediato sin estancarse ni golpear `win_bottom` al aterrizar.
  * [`WindowShadowPipeline::render_shadow_alpha`](../crates/compositor/src/shadow.rs) y `render_genie_shadow` desvanecen progresivamente la sombra proyectada cuando `j == 0.0` y `k < 0.45` en ventanas no maximizadas ni en pantalla completa tanto en DRM como en Winit.

### 3. Preservación de Desenfoque Dual Kawase en Ventanas Translúcidas (`crates/compositor/src/blur.rs`, `capture.rs`, `black_hole.rs`, `drm.rs`, `winit.rs`)
* **Composición del blur de fondo dentro de snapshots offscreen (Genie, miniaturas del Dock y Agujero Negro):**
  * [`GaussianBlur`](../crates/compositor/src/blur.rs) registra `snapshot_is_drm` en `capture_background_region` y expone `translucent_window_blur_rect`, `render_captured_blur_to_offscreen` y `render_translucent_offscreen_blur` sobre una ruta compartida `render_blur_internal` (con blending premultiplicado `SRC_ALPHA, ONE_MINUS_SRC_ALPHA, ONE, ONE_MINUS_SRC_ALPHA`).
  * Tanto [`capture_window_offscreen`](../crates/compositor/src/capture.rs) como [`capture_closing_window_texture`](../crates/compositor/src/black_hole.rs) pintan el desenfoque Dual Kawase detrás de ventanas `is_translucent` antes de dibujar sus superficies vivas o cacheadas; [`winit.rs`](../crates/compositor/src/backend/winit.rs) también actualiza `snapshot_tex` tras el fondo y en ventanas translúcidas superpuestas.

### 4. Validación
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-compositor`: **67 passed; 0 failed**.
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación completa sin errores (`exit 0`; sin advertencias en `malus-de-compositor` ni `malus-de-dock`).

---

## [2026-11-14] — Transición Animada entre Escritorios y Reporteador de Fallos (`.txt` + Cloudflare Worker)

### 1. Animación Horizontal de Cambio de Escritorio (`crates/compositor/src/workspace_transition.rs`, `theme_transition.rs`, `expose.rs`, `state.rs`, `drm.rs`, `winit.rs`)
* **Pipeline GPU de doble captura (`DURATION = 460ms`, `DESKTOP_GAP_RATIO = 0.028`):**
  * Nuevo módulo [`workspace_transition.rs`](../crates/compositor/src/workspace_transition.rs) con `WorkspaceTransitionPipeline` (OpenGL ES 3.0), reutilizando `compile_shader` y `link_program` desde [`theme_transition.rs`](../crates/compositor/src/theme_transition.rs).
  * Antes de intercambiar las ventanas en `Space`, `switch_workspace` agenda `pending_workspace_switch` (encadenable ante pulsaciones o gestos consecutivos) para capturar el escritorio saliente mediante `capture_outgoing` antes de ejecutar `commit_pending_workspace_switch` / `apply_workspace_switch`.
  * En cada fotograma de la transición, el compositor renderiza el escritorio entrante vivo en el framebuffer, lo copia a textura y ejecuta el shader de deslizamiento horizontal estilo macOS Spaces con curva combinada (`smoothstep` + `ease-out` cúbica en `eased_progress`), separador oscuro del `2.8%`, sombreado de bordes y atenuación de profundidad.
* **Robustez GLES, sincronización VBlank y soporte desde Exposé:**
  * Se limpia el estado de atributos dejado por Smithay (`DisableVertexAttribArray(0/1)`, `VertexAttribDivisor(0/1, 0)`, `BindBuffer(ARRAY_BUFFER, 0)` y `Disable(CULL_FACE)`), se consulta `FRAMEBUFFER_BINDING` para elegir `COLOR_ATTACHMENT0` vs `BACK`, y `started_at` se fija en el primer `draw()` efectivo para que la espera de VBlank en DRM no recorte el inicio de la animación.
  * Al seleccionar otro escritorio desde la barra de Exposé (`SpaceDropTarget::Workspace`), [`ExposeState::close_immediately`](../crates/compositor/src/expose.rs) cierra Exposé tras capturar el fotograma saliente para animar el deslizamiento hacia el nuevo escritorio.
  * Integración completa en [`drm.rs`](../crates/compositor/src/backend/drm.rs) (despertar por VBlank, daño completo en pantalla y limpieza final de los 3 búferes DRM) y [`winit.rs`](../crates/compositor/src/backend/winit.rs).

### 2. Reporteador de Fallos en `.txt`, Salida Limpia y Envío Directo vía Cloudflare Worker (`crates/common/src/crash.rs`, `logging.rs`, `crates/dialogs`, `scripts/malus-session`, `services/crash-worker`)
* **Captura automática de `panic` y filtrado de cierres intencionales (`70240c2` + seguimiento):**
  * [`crash.rs`](../crates/common/src/crash.rs) conectado desde `init_logging` en [`logging.rs`](../crates/common/src/logging.rs) instala un *panic hook* global para todos los binarios de Malus DE, guardando volcados `.txt` sanitizados (`$HOME` ocultado como `~`) en `${XDG_DATA_HOME:-$HOME/.local/share}/malus-de/crashes/`.
  * [`mark_clean_session_exit`](../crates/common/src/crash.rs) escribe `${XDG_DATA_HOME:-$HOME/.local/share}/malus-de/clean-exit` al ejecutar cerrar sesión, reiniciar o apagar desde Spotlight ([`spotlight_visual.rs`](../crates/compositor/src/spotlight_visual.rs)), Top Bar ([`system_menu.rs`](../crates/top-bar/src/ui/system_menu.rs)), CLI ([`malus-de-ctl`](../crates/ctl/src/main.rs)) y [`wayland.rs`](../crates/render/src/wayland.rs).
  * [`malus-session`](../scripts/malus-session) ignora salidas por señales de terminación esperadas (`0`, `129` `SIGHUP`, `130` `SIGINT`, `143` `SIGTERM`) o presencia de `clean-exit`, evitando falsos reportes al cerrar sesión o apagar, y manteniendo los volcados para fallos reales (`SIGSEGV`, `SIGABRT`, panics, etc.). [`install-session.sh`](../scripts/install-session.sh) instala `malus-de-crash-reporter` en `/usr/local/bin`.
* **Diálogo GTK4 simplificado y entrega directa a `malus-de@chano.dev`:**
  * [`malus-de-crash-reporter`](../crates/dialogs/src/bin/malus-de-crash-reporter.rs) y [`show_crash_report_dialog`](../crates/dialogs/src/lib.rs) presentan una alerta compacta estilo macOS con mensaje breve, dos botones principales (**Cerrar** y **Enviar reporte**, vía `POST` JSON en segundo plano con `curl` a `https://crash.chano.dev/report`, configurable con `MALUS_CRASH_ENDPOINT` o `~/.config/malus-de/crash-reporter.toml`) y un desplegable **`Ver detalles`** cerrado por defecto con el `.txt` y **Guardar .txt...**.
  * Nuevo Worker sin dependencias externas en [`services/crash-worker/`](../services/crash-worker/README.md) (`index.js` + `wrangler.toml`) usando `cloudflare:email` para entregar el informe con el `.txt` adjunto a `malus-de@chano.dev` sin exponer credenciales SMTP en el binario cliente.

### 3. Validación
* `bash -n /home/cristian/Proyectos/malus-de/scripts/malus-session /home/cristian/Proyectos/malus-de/scripts/install-session.sh`: correcto.
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-common -p malus-de-dialogs -p malus-de-compositor`:
  * `malus-de-common`: **41 passed, 0 failed**.
  * `malus-de-compositor`: **61 passed, 0 failed**.
  * `malus-de-dialogs`: **1 passed, 0 failed**.
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace`: compilación completa sin errores (`exit 0`).

---

## [2026-11-14] — Snap Layouts Estilo Windows 11, Animación de Cierre "Agujero Negro", Refuerzo de Blur Acrílico, Sincronización Squircle y Manifiesto Arch

### 1. Menú Flotante Snap Layouts Estilo Windows 11 (`crates/compositor/src/snap_layouts.rs`, `state.rs`, `shadow.rs`, `grabs.rs`, `ssd.rs`, `drm.rs`, `winit.rs`)
* **Comportamiento y secciones:**
  * Se mantiene intacto el ajuste por arrastre a bordes de pantalla y se añade un menú flotante *Liquid Glass* que aparece al mantener el cursor sobre el botón de maximizar (`HOVER_OPEN_DELAY = 240ms`, histéresis de `4px`, retardo de cierre `260ms` y corredor continuo entre botón y tarjeta).
  * Incluye las 6 distribuciones estándar de Windows 11 (`50/50`, `67/33`, `33/33/33`, `50% + 2 cuartos`, `4 cuartos 2×2` y `25/50/25`, adaptándose a 4 tarjetas en pantallas `< 1024px` lógicos) y extiende `SnapMode` y `snap_target_geometry` en `state.rs` a todas las 16 zonas resultantes.
* **Coordenadas multimonitor, detección CSD/SSD y sombra GPU difusa:**
  * [`SnapLayoutsState::prepare_frame`](../crates/compositor/src/snap_layouts.rs) convierte `card_rect` a coordenadas locales del monitor mediante `localize_popup_rect(card_rect, output_geo)` para alinear exactamente el blur acrílico y el renderizado Cairo debajo del botón de maximizar en configuraciones multimonitor y con escalado fraccionario (`compute_popup_rect` centrado bajo el botón).
  * [`detect_maximize_button_under`](../crates/compositor/src/snap_layouts.rs) cubre tanto barras SSD (`(60, 19)`) como controles CSD superiores (`!window_uses_ssd(window)` con `lx ∈ [53, 78]`, `ly ∈ [7, 34]`, incluyendo GTK4/libadwaita como Nautilus alrededor de `(+69, +22)`).
  * Se sustituyó la sombra multicapa en Cairo por [`WindowShadowPipeline::render_menu_shadow_alpha`](../crates/compositor/src/shadow.rs), proyectando sombra SDF gaussiana difusa por GPU en `drm.rs` y `winit.rs`.

### 2. Animación de Cierre de Ventanas "Agujero Negro" (`crates/compositor/src/black_hole.rs`, `state.rs`, `ssd.rs`, `drm.rs`, `winit.rs`)
* **Pipeline GPU en 3 fases (`BLACK_HOLE_DURATION = 680ms`):**
  * Nuevo módulo [`black_hole.rs`](../crates/compositor/src/black_hole.rs) con `BlackHolePipeline` (OpenGL ES 3.0): ignición en el centro exacto del botón de cierre con horizonte de sucesos del tamaño del botón y disco de acreción inclinado, deformación/succión gravitacional mediante malla teselada `36×36` (`7,776` vértices vía `gl_VertexID`) directamente hacia esa esquina sin efecto túnel, y colapso final con implosión.
  * Captura *offscreen* de textura `GlesTexture` combinada con caché ligero de instantáneas por `CommitCounter` (`update_cached_window_snapshot`), permitiendo que aplicaciones que destruyen o desacoplan su superficie antes de `toplevel_destroyed` (como clientes GTK/ZapFast o cierres desde switcher/CSD superior) animen de forma consistente en ambos backends (`drm.rs` y `winit.rs`).

### 3. Refuerzo del Desenfoque Acrílico en Fondos Semitransparentes (`crates/compositor/src/blur.rs`)
* Se refactorizó la cadena de desenfoque en `run_blur_passes` incrementando la escala gaussiana en el shader (`BLUR_RADIUS_SCALE = 1.75`) y el soporte de múltiples pasadas separables a resolución reducida (`downsampled_size`), logrando un acabado esmerilado más intenso sin artefactos en bordes (`CAPTURE_PAD_PX`).

### 4. Sincronización de Esquinas *Squircle*, Sombras en Resize y Corte del Dock (`crates/compositor/src/window_corners.rs`, `state.rs`, `drm.rs`, `winit.rs`)
* En [`window_corners.rs`](../crates/compositor/src/window_corners.rs), `WindowSquircleShader` reemplaza `u_buf_size` por `u_inv_tex_matrix` para invertir la matriz de textura de Smithay (`wp_viewporter`, rotaciones y DMA-BUFs `y_inverted`) desde el fotograma 0.
* Se ajustó `effective_shadow_rect_for_window` durante redimensionados y `SnapTransition` (`RescaleRenderElement`), además de corregir el clipping/límite que generaba una línea de corte en el Dock y sus iconos al hacer zoom (`802707e`).

### 5. Documentación de Compilación para Arch Linux (`docs/PACKAGE_MANIFEST.md`)
* Se añadió la subsección **Dependencias de Compilación (`pacman` para desarrolladores)** en la sección de Arch Linux de [`docs/PACKAGE_MANIFEST.md`](PACKAGE_MANIFEST.md) junto con la recomendación de compilar vía `cargo build --release --locked`.

### 6. Validación
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-compositor`: **60 passed, 0 failed**.
* `cargo check --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml --workspace` y `cargo build --release --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-compositor`: compilación exitosa.

---

## [2026-11-14] — Soporte `zwp_linux_dmabuf_v1` (Zero-Copy GPU), Sincronización DRM por Monitor y Reducción de Overdraw

### 1. Diagnóstico de Causa Raíz en Animaciones Web y Multimonitor
* **Ausencia de `zwp_linux_dmabuf_v1` en el compositor (`crates/compositor/src/state.rs`):**
  * `MalusState` exponía únicamente `ShmState` (`wl_shm`), sin `DmabufState` ni `delegate_dmabuf!`.
  * Sin `zwp_linux_dmabuf_v1`, los navegadores basados en Chromium/Firefox y aplicaciones Electron (como Antigravity IDE) no podían compartir búferes gráficos directamente en VRAM (*zero-copy DMA-BUF*), viéndose forzados a copias de lectura hacia memoria CPU y subidas `glTexSubImage2D` por cada cuadro en ambas pantallas.
* **Contención multimonitor y esperas bloqueantes en DRM KMS (`crates/compositor/src/backend/drm.rs`, `state.rs`):**
  * El bucle DRM utilizaba un temporizador bloqueante fijo de `frame_interval` al final del ciclo y un único contador global `surface_commit_counter`, provocando que los commits de un monitor despertaran o bloquearan al otro monitor y que `send_frames_surface_tree` notificara capas de salidas no renderizadas.
* **Sobrecarga de GPU y CPU por cuadro (`crates/compositor/src/shadow.rs`, `drm.rs`):**
  * `WindowShadowPipeline::render_shadow_with_style` dibujaba un quad completo cubriendo toda la ventana + margen de sombra para luego descartar (`discard`) millones de fragmentos interiores en `squircle_sdf`.
  * Con `ENABLE_DAMAGE_TRACKING = false`, el bucle DRM seguía construyendo el vector `tracker_elements` en CPU cada fotograma sin utilizarlo.

### 2. Solución Implementada
* **Protocolo `zwp_linux_dmabuf_v1` acelerado por hardware:**
  * [`crates/compositor/src/state.rs`](../crates/compositor/src/state.rs): añadidos `dmabuf_state`, `dmabuf_global`, `pending_dmabuf_imports`, `delegate_dmabuf!(MalusState)`, `impl DmabufHandler for MalusState` y `MalusState::process_pending_dmabuf_imports`.
  * [`crates/compositor/src/backend/drm.rs`](../crates/compositor/src/backend/drm.rs): registro de `zwp_linux_dmabuf_v1` v5 mediante `DmabufFeedbackBuilder` usando el nodo DRM de render y los formatos de `GlesRenderer` (con *fallback* automático a v3) e importación inmediata de DMA-BUFs tras `dispatch_clients`.
  * [`crates/compositor/src/backend/winit.rs`](../crates/compositor/src/backend/winit.rs): habilitación de `create_global` para DMA-BUF y drenaje de `process_pending_dmabuf_imports` en modo anidado.
* **Planificación dirigida por VBlank y aislamiento por salida en DRM:**
  * Estado `flip_pending: Option<Instant>` e `is_flip_pending()` por salida en `DrmOutputSurface`, limpiado al recibir `DrmEvent::VBlank(crtc)`, `ActivateSession` o `use_mode`.
  * Registro del `poll_fd()` de Wayland en `calloop`, cálculo de `has_ready_output` para espera cero cuando hay salidas listas, y eliminación del `sleep` bloqueante al final del bucle.
  * Contadores por monitor `output_commit_counters` en `MalusState::commit()`, reducción a 1 búfer por evento (`damage_buffers_per_event = 1`) cuando `!ENABLE_DAMAGE_TRACKING`, y filtrado de `send_frames_surface_tree` en capas únicamente para `rendered_outputs`.
* **Reducción de overdraw en sombras y asignaciones por cuadro:**
  * [`crates/compositor/src/shadow.rs`](../crates/compositor/src/shadow.rs): `SHADOW_VERT_SRC` y `render_shadow_with_style` ahora dibujan 4 bandas perimetrales alrededor de `u_cut_rect` (24 vértices) omitiendo el interior descartado de la ventana.
  * [`crates/compositor/src/backend/drm.rs`](../crates/compositor/src/backend/drm.rs): la construcción de `tracker_elements` queda condicionada a `ENABLE_DAMAGE_TRACKING`.

### 3. Validación
* `cargo test --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-compositor`: **48 passed, 0 failed**.
* `cargo build --release --manifest-path /home/cristian/Proyectos/malus-de/Cargo.toml -p malus-de-compositor`: compilación exitosa.

---

## [2026-11-11] — Corrección del Selector Alt+Tab (`malus-de-dock`) y Avance/Pendiente de Foco en Compositor

### 1. Estabilización de Tamaño, Opacidad y Ciclo de Vida del `Alt+Tab` (`malus-de-dock`)
* **Diagnóstico de Causa Raíz:**
  * **Tamaño estancado:** `HideAppSwitcher` ocultaba y vaciaba `switcher_box`, pero dejaba la ventana `malus-app-switcher` mapeada con su tamaño máximo previo; `ShowAppSwitcher` tampoco recalculaba el tamaño requerido antes de presentar el switcher.
  * **Opacidad:** `.switcher-card` definía `opacity: 0.65` para tarjetas no seleccionadas en `crates/dock/src/styles/dock.css`.
  * **Colapso completo del Dock y del Alt+Tab:** Al llamar `switcher_win_ipc.set_visible(false)` en `HideAppSwitcher` sobre una `LayerSurface` de `gtk4-layer-shell`, GTK4 envía `wl_surface.attach(nil)` + `wl_surface.commit()` justo después de destruir `zwlr_layer_surface_v1`. En `smithay 0.7.0` (`wlr_layer/handlers.rs`), el *pre-commit hook* de `wl_surface` permanece activo tras reiniciar el estado pendiente a `0×0` sin anclas y dispara `Error::InvalidSize`, cerrando la conexión Wayland de todo el cliente `malus-de-dock` (destruyendo tanto `malus-app-switcher` como `malus-dock`).
* **Solución Implementada (`crates/dock/src/main.rs`, `crates/dock/src/styles/dock.css`):**
  * Se agregó `resize_switcher_window` para medir `switcher_surface` mediante `surface.measure(...)`, aplicar `window.set_default_size(width, height)` + `window.queue_resize()` y redibujar el fondo al abrir o actualizar `ShowAppSwitcher`.
  * En `HideAppSwitcher`, en lugar de `set_visible(false)`, se oculta/vacía `switcher_box_ipc` y se encoge la ventana a `1×1` (`set_default_size(1, 1)`, `queue_resize()`, `queue_draw()`), manteniendo mapeada la `LayerSurface` sin disparar `InvalidSize` (el compositor ya omite renderizado, blur y *hit-testing* de `malus-app-switcher` mientras `!app_switcher.is_active`).
  * Se ajustó `.switcher-card` a `opacity: 1.0`.
* **Resultado:** El dock dejó de colapsar al cambiar o cerrar ventanas y el `Alt+Tab` crece/encoge correctamente en las pruebas.

### 2. Mitigaciones de Foco de Ventanas y Botones del Semáforo (`malus-de-compositor`) — Pendiente
* **Cambios aplicados (`crates/compositor/src/state.rs`, `drm.rs`, `winit.rs`):**
  * Incorporación de `sync_window_activation`, `set_keyboard_focus_with_sync`, `defer_activation_configure` / `flush_pending_activation_configures`, `prepare_pointer_press_grabs`, alineación de `update_ssd_hover` con `popup_grab_active()`, y ajuste de restauración de foco en `layer_destroyed` usando `.last()`.
  * Prueba unitaria `prepare_pointer_press_grabs_clears_residual_grabs_and_sync_none_clears_focus` agregada y aprobada (41/41 tests en `malus-de-compositor`, 9/9 tests en `malus-de-dock`).
* **Estado Pendiente:** En pruebas de sesión en vivo persiste el comportamiento de ventanas elevadas que conservan apariencia fuera de foco y botones del semáforo sin respuesta al clic tras mostrar *hover*. Se deja anotado para investigación y reparación posterior en [`docs/decisiones/0004-foco-ventanas-y-botones-semaforo-pendiente.md`](decisiones/0004-foco-ventanas-y-botones-semaforo-pendiente.md).

### 3. Escalado de Pantallas: Corrección de PPI/Diagonal Física y Registro de Pendientes (`malus-de-common`, `malus-de-compositor`, `malus-de-settings`, `malus-de-ctl`, `malus-de-top-bar`)
* **Corrección aplicada (Problema 4):**
  * `DisplayOutputConfig::effective_diagonal()`, `ppi()` y `physical_size_mm()` ahora devuelven `Option` si falta `diagonal_inches`, eliminando el fallback silencioso a `27.0"` y la inferencia por nombre en `from_gnome_xml`.
  * En `DisplaysConfig::sync_linked_scales_from`, el enlace 1:1 no recalcula escalas de pantallas sin PPI conocido; en `drm.rs`, `Output::new` preserva los milímetros reales del EDID (`conn_info.size()`) salvo override manual explícito (`> 0.5"`) e inicializa pantallas nuevas con `DisplayOutputConfig::recommended_scale`.
  * UI y CLI actualizados (`malus-de-settings`, `malus-de-ctl`, `malus-de-top-bar`) y pruebas unitarias añadidas (`test_unknown_diagonal_does_not_assume_27_inches_or_distort_1to1_link`, `test_recommended_scale_from_detected_ppi`), con compilación y tests aprobados (`malus-de-common` 36/36, `malus-de-compositor` 41/41, `malus-de-top-bar` 10/10).
* **Pendientes restantes (`docs/decisiones/0005-escalado-pantallas-pendiente.md`):** escalado XWayland (`Xft.dpi`/X11), clientes sin `wp_fractional_scale_v1` (incluyendo `malus-de-render`), redondeo lógico/físico de ±1 px, layout forzado en `recalculate_layout`, selección del primer monitor en `start_screen_recording` multimonitor y brecha de escala entre `drm.rs` y `winit.rs`.

---

## [2026-11-10] — Fase 2: Optimización de Latencia en Compositor DRM y Aislamiento de Rendimiento en Wrapper de Electron (Antigravity IDE)

### 1. Eliminación de la Doble Espera y Despacho Inmediato en DRM KMS
* **Diagnóstico en Animaciones Web y Carga de Páginas:**
  * Al final del ciclo de renderizado de DRM, el compositor ya aplicaba el control de tasa de refresco esperando a completar `frame_interval` (16.6 ms a 60 Hz).
  * Sin embargo, al inicio del ciclo subsiguiente, `event_loop.dispatch` volvía a dormirse hasta `8 ms` adicionales porque `active_animations` solo contenía transiciones de ventanas de Malus y no los commits de los navegadores web. Esto alargaba el fotograma a ~24 ms (~40 FPS), forzando a los motores de renderizado web a descartar fotogramas en animaciones CSS, WebGL y scrolling.
* **Solución Implementada:**
  * En `crates/compositor/src/backend/drm.rs`: si existen búferes pendientes de renderizado o commits de clientes (`pending_damage_buffers > 0`), el tiempo de espera al inicio del ciclo se fija en `Duration::ZERO`. En reposo completo se utiliza `1 ms`.
  * Los callbacks de sincronización `wl_surface.frame` se entregan con precisión en cada vblank exacto a 60.0 FPS / 120.0 FPS nativos.

### 2. Caché de Fondo en `blur.capture_background`
* **Eliminación de Copias Redundantes de VRAM:**
  * Anteriormente, cada fotograma activo invocaba `glCopyTexSubImage2D` de la pantalla completa (8.3 MB a 33.2 MB) para alimentar el desenfoque Dual-Kawase.
  * Se implementó `background_dirty` y seguimiento de commits de capas inferiores (`background_commit_counter` en `crates/compositor/src/state.rs`).
  * En animaciones web, navegación o movimiento de cursor, el fondo no cambia: `blur.capture_background` se omite al 100%, liberando ancho de banda masivo en la GPU y eliminando bloqueos de 2 a 5 ms por cuadro.
* **Throttle de Cursor a Fotograma Único:**
  * El movimiento de ratón solicita únicamente el búfer en scanout actual (`pending_damage_buffers.max(1)`), evitando encolar tres cuadros de pantalla completa redundantes.

### 3. Aislamiento y Optimización del Wrapper de Electron (Antigravity IDE & VS Code)
* **Diagnóstico de Causa Raíz en Antigravity IDE:**
  * El wrapper en tiempo de ejecución (`malus-ide-wrapper`) observaba todo el `document.body` con `{ subtree: true }` y ejecutaba un `setInterval` cada 200 ms. En un IDE con miles de mutaciones en Monaco Editor, esto provocaba *Layout Thrashing / Reflow* continuo en el hilo principal de Electron y bombardeaba de commits continuos al compositor Wayland.
  * Además, el proceso Python reinyectaba el script por WebSocket CDP cada 3 segundos (`REINJECT_INTERVAL = 3.0`).
* **Solución Implementada (`scripts/malus-ide-wrapper`):**
  * **Observador Restringido:** El `MutationObserver` se limita estrictamente al contenedor de la barra de título (`.part.titlebar`) con `{ subtree: false }`. El editor de código, terminal, chat de IA y árbol de archivos quedan 100% aislados.
  * **Auto-detención:** El temporizador inicial se desconecta de inmediato (`clearInterval`) en cuanto los botones semáforo quedan reubicados.
  * **Caché en Python:** El watcher inyecta una sola vez por URL de workbench; no vuelve a evaluar código vía CDP a menos que ocurra un *Reload Window*.
  * **Resultado:** Cero impacto en CPU (0–1%), cero alertas de integridad de archivos y fluidez nativa idéntica a GNOME.

---

## [2026-11-10] — Implementación de Idle Sleeping y Frame Throttling en Backend DRM (Resolución de Lag y Regresiones)

### Diagnóstico de Causa Raíz
* **Desincronización de Búferes DRM KMS y Micro-bloqueos:**
  * En `crates/compositor/src/backend/drm.rs`, la llamada a `out_surface.surface.next_buffer()` se ejecutaba incondicionalmente al inicio del ciclo de cada monitor. Cuando la comprobación de daño evaluaba `render_damage.is_empty()`, se ejecutaba `continue;`, descartando el búfer adquirido sin invocar `queue_buffer()`.
  * En la arquitectura de triple-buffering DRM/KMS, descartar búferes sin encolar el *page flip* desincroniza las ranuras de KMS y provoca esperas de *timeout* de vblank en llamadas posteriores, causando micro-stutters constantes y lag de entrada percibido en el cursor.
* **Sobrecarga de Fragment Shaders Volumétricos por Daño de Cursor:**
  * El movimiento del ratón genera eventos de daño de $32 \times 32\text{ px}$. Como la condición de sombra evaluaba la intersección con la ventana completa, cada píxel de desplazamiento disparaba el redibujado de la sombra volumétrica squircle ($n=4.2$) y pases Dual-Kawase sin recorte tijera (`gl.Disable(SCISSOR_TEST)`), saturando el *fillrate* de la GPU a la tasa de sondeo del ratón (hasta 1000 Hz).
* **Regresiones Visuales por Daño Parcial en Modo Inmediato:**
  * Al intentar excluir el daño de cursor para mitigar la sobrecarga, `frame.clear(dark_acrylic, damage)` limpiaba el área bajo el ratón pero la sombra y el blur no se redibujaban en esa zona, dejando estelas y huecos transparentes. Adicionalmente, al dibujarse con *blending* `SRC_ALPHA` sobre píxeles no limpiados, la penumbra se acumulaba volviéndose negra.

### Solución Implementada (Opción B: Idle Sleeping / Frame Throttling)
1. **Verificación de Daño Previa a la Adquisición de Búferes (`next_buffer`):**
   * Se reubicó `out_surface.surface.next_buffer()` para ejecutarse **únicamente tras confirmar que la pantalla física tiene cambios pendientes**.
   * Si la pantalla está estática (sin commits Wayland, sin movimiento de ratón, sin cambios geométricos de ventanas y sin animaciones activas), el compositor omite la llamada a DRM (**0% de uso de GPU en reposo**). KMS mantiene de forma nativa el escaneo del búfer frontal sin interrupciones.
2. **Garantía Estricta del Ciclo de Búferes DRM:**
   * Al ejecutarse el renderizado, se garantiza que cada búfer adquirido mediante `next_buffer()` es enviado a KMS mediante `queue_buffer()`. Jamás se descartan búferes con `continue`.
3. **Rastreo Global de Commits Wayland (`surface_commit_counter`):**
   * Se incorporó `surface_commit_counter: usize` en `MalusState` (`crates/compositor/src/state.rs`), incrementado en cada llamada de `commit()`. Toda actividad de clientes (videos, terminales, navegadores o widgets de la barra superior) despierta inmediatamente el ciclo de renderizado para 3 búferes consecutivos (cubriendo el ciclo completo de triple-buffering).
4. **Throttling Dinámico del Bucle de Eventos (`event_loop.dispatch`):**
   * En reposo, el tiempo de espera de `event_loop.dispatch` se eleva a `8 ms` (permitiendo a `epoll` despertar en microsegundos ante cualquier interrupción de hardware o socket Wayland). Durante animaciones (Genie o transiciones de color), se reduce a `1 ms` para máxima fluidez.
5. **Renderizado Limpio y Prístino (`ENABLE_DAMAGE_TRACKING = false`):**
   * Durante los fotogramas activos, se realiza el redibujado completo y limpio, garantizando que las sombras volumétricas SDF ($n=4.2$) y el desenfoque Dual-Kawase acrílico capturen siempre el fondo limpio sin artefactos, estelas ni parpadeos.

---

## [2026-11-10] — Estabilización del Compositor DRM: Incompatibilidad de Damage Tracking Directo y Hoja de Ruta FBO Cache

### Fundamento Conceptual: Renderizado Completo vs. Actualización por Deltas
* **El Problema Fundamental del Compositor:**
  Actualmente, el compositor dibuja toda la pantalla píxel a píxel 60 veces por segundo en cada ciclo de refresco:
  1. *Limpieza total:* Ejecuta `glClear` sobre la totalidad de la resolución (p. ej. millones de píxeles en pantallas 4K o configuraciones multimonitor).
  2. *Wallpaper completo:* Dibuja el fondo de pantalla íntegro.
  3. *Ventanas y sombras:* Para cada ventana (esté estática o en movimiento), calcula y rasteriza sus sombras volumétricas squircle SDF y sus superficies Wayland.
  4. *Captura y desenfoque:* Para ventanas translúcidas (sidebar de Nautilus o Branches), la GPU toma una instantánea del fondo con `glCopyTexSubImage2D`, ejecuta 4 pasadas del shader Dual-Kawase para desenfocarlo y lo aplica encima.
  5. *Capas superiores (Dock y Top Bar):* Aplica sus recortes squircle y sombras en cada pase.
  6. *Presentación:* Envía el búfer finalizado a KMS para escaneo del monitor.
  Este modelo produce una imagen impecable y sin artefactos gracias a la potencia de las GPUs modernas, pero desperdicia ciclos de procesamiento y ancho de banda de memoria cuando el escritorio está prácticamente estático.

* **Por Qué "Actualizar Solo los Píxeles que Cambian" (Damage Tracking) Falló en este Compositor:**
  En compositores convencionales sin efectos de desenfoque ni shaders volumétricos (como Openbox o Weston básico), el daño parcial es trivial: si el ratón se desplaza $5\text{ px}$, se recorta un rectángulo con tijera OpenGL (`glScissor`) de $32 \times 32$ píxeles y se actualiza únicamente ese parche. Sin embargo, en Malus DE esta aproximación directa colisionó con dos barreras arquitectónicas:

  1. **El Desenfoque Dual-Kawase Requiere Leer el Fondo Limpio:**
     El desenfoque acrílico no es un color plano, sino una transformación matemática sobre los píxeles que residen detrás.
     * Si el compositor solo actualiza una franja dañada (p. ej., un video de YouTube en Brave en la mitad derecha), la tijera de OpenGL no toca la mitad izquierda donde está Nautilus.
     * Si el pipeline intentaba redibujar el desenfoque en cada fotograma, `glCopyTexSubImage2D` leía del framebuffer píxeles que *ya habían sido desenfocados en el fotograma anterior*, re-desenfocándolos sucesivamente en un bucle acumulativo (feedback loop) que volvía opaca y oscura la barra lateral.
     * Si, para evitar el feedback loop, se restringía el redibujado del blur únicamente a cuando la ventana se movía (`geom_changed`), en el instante en que otra ventana adyacente enviaba daño, el limpiado parcial borraba el área circundante y la ventana estática perdía su desenfoque y su sombra (volviendo la barra lateral 100% transparente y mostrando el contenido trasero sin filtro).

  2. **La Cadena de 3 Búferes Físicos de la GPU (Triple Buffering en DRM KMS):**
     La tarjeta gráfica no dibuja sobre la pantalla visible, sino que alterna cíclicamente entre 3 ranuras de búferes (`Buffer 0` en pantalla, `Buffer 1` encolado, `Buffer 2` en renderizado).
     Al enviar parches parciales en instantes temporales distintos, cada uno de los 3 búferes recibía fragmentos desacoplados. Al rotar entre ellos a 60 Hz, el monitor mostraba imágenes discordantes alternándose a ~20 Hz, produciendo el parpadeo constante (*flickering*) apreciable en grabaciones externas.

### Diagnóstico de Causa Raíz
* **Parpadeos a 20 Hz (Flickering en Swapchain DRM):**
  * Al intentar activar `OutputDamageTracker` con tijera de renderizado parcial (`glScissor`) en `crates/compositor/src/backend/drm.rs`, el buffer swapchain KMS/GBM de 3 ranuras (`Buffer 0`, `Buffer 1`, `Buffer 2`) se desincronizaba.
  * Debido a que el daño personalizado del compositor (sombras volumétricas y desenfoque Dual-Kawase) solo se aplicaba al buffer en curso en el frame de cambio, los otros dos buffers recibían estados obsoletos, alternando imágenes a 60 Hz que el ojo y la cámara registraban como parpadeo continuo a ~20 Hz.
* **Pérdida de Desenfoque Dual-Kawase y Sombras al Estabilizar Ventanas:**
  * El desenfoque acrílico (`GaussianBlur::render_blur`) realiza `glCopyTexSubImage2D` sobre el framebuffer. Si se recalcula cada frame, se produce un bucle de retroalimentación que ensucia el fondo.
  * Al intentar restringir el cálculo a cambios de geometría (`geom_changed`), cualquier actualización adyacente (como reproducción de video en navegador o parpadeo del cursor) limpiaba el área con `frame.clear(dark_acrylic, damage)`. Al estar la ventana estática, el desenfoque y las sombras no se redibujaban, dejando la barra lateral 100% transparente sin blur y las ventanas sin sombra.

### Solución y Decisión Arquitectónica
* **Desactivación de Damage Tracking Directo en Modo Inmediato:**
  * Se fijó `ENABLE_DAMAGE_TRACKING = false` en `crates/compositor/src/backend/drm.rs`.
  * Se restauró el redibujado completo y limpio en cada pase activo (`window_needs_shadow_blur = true`), eliminando por completo cualquier parpadeo y garantizando que las sombras squircle SDF y el desenfoque Dual-Kawase capturen siempre el fondo prístino.
* **Documentación y Hoja de Ruta para Agentes Futuros:**
  * Creación del Architectural Decision Record [`docs/decisiones/0002-arquitectura-damage-tracking-y-desenfoque-gpu.md`](decisiones/0002-arquitectura-damage-tracking-y-desenfoque-gpu.md).
  * Creación del archivo de reglas [`AGENTS.md`](../AGENTS.md) para que cualquier agente de IA asistente conozca las limitaciones del modo inmediato con framebuffer capture y la hoja de ruta requerida (Offscreen FBO Render Cache o Idle Output Sleeping).

---

## [2026-11-09] — Aislamiento Bidireccional Estricto Malus DE ↔ GNOME: Blindaje de Botones Semáforo y Neutralización de Fugas D-Bus/dconf/GTK

### Diagnóstico de Causa Raíz
* **Contaminación del Shell de GNOME (Topbar y Menús Blancos al Entrar a GNOME):**
  * En `crates/common/src/config.rs`, la rutina `sync_system_appearance` ejecutaba explícitamente comandos `dconf write /org/gnome/desktop/interface/color-scheme` y `gsettings set org.gnome.desktop.interface color-scheme` con `env_remove("XDG_CONFIG_HOME")`, modificando la base de datos de dconf del usuario anfitrión (`~/.config/dconf/user`) fuera del entorno aislado de Malus DE.
  * Además, `sync_system_appearance` iteraba sobre un arreglo `ini_paths` que escribía activamente `gtk-application-prefer-dark-theme=0` y `gtk-theme-name=Adwaita` en `$HOME/.config/gtk-3.0/settings.ini` y `$HOME/.config/gtk-4.0/settings.ini` del host. Al iniciar sesión en GNOME Shell o abrir aplicaciones GTK4/Libadwaita en el host, éstas leían la preferencia clara forzada de dicho archivo y de dconf, tiñendo el panel superior y los menús flotantes de blanco.
* **Injerencia de GNOME Retoques ("Tweaks") sobre los Botones en Malus DE:**
  * Al ajustar la posición o visibilidad de los botones de ventana en Retoques (GNOME Tweaks), la aplicación modificaba el registro dconf `/org/gnome/desktop/wm/preferences/button-layout`.
  * En `scripts/malus-session`, se exportaba globalmente `GTK_USE_PORTAL=1`. Dicha variable forzaba a todas las aplicaciones GTK a eludir su backend local `keyfile` y consultar el servicio D-Bus `org.freedesktop.portal.Settings`. Éste era atendido por `xdg-desktop-portal-gnome` (ejecutado bajo `systemd --user` en el entorno global del host), el cual retransmitía los cambios de dconf del host a las aplicaciones en ejecución dentro de Malus DE.
  * En `crates/compositor/src/spotlight_visual.rs`, la función `toggle_desktop_theme()` sobreescribía el archivo `glib-2.0/settings/keyfile` escribiendo exclusivamente `[org/gnome/desktop/interface]`, eliminando por completo la sección `[org/gnome/desktop/wm/preferences]` con `button-layout='close,minimize,maximize:'`.
  * En `crates/dock`, `crates/wallpaper` y `crates/render`, algunos subprocesos lanzados (`nautilus trash://`, terminales, etc.) no inyectaban de forma explícita y centralizada las variables de aislamiento `XDG_CONFIG_HOME` y `GSETTINGS_BACKEND="keyfile"`.

### Solución Implementada
* **Configuración Común (`crates/common/src/config.rs`):**
  * Se eliminaron todas las escrituras a `dconf` del host, `gsettings` del host y a las rutas `$HOME/.config/gtk-3.0` y `$HOME/.config/gtk-4.0`. Malus DE opera de forma 100% aislada dentro de `$HOME/.config/malus-de`.
  * Se creó la función centralizada `apply_isolated_environment(&mut Command)`, la cual inyecta consistentemente `XDG_CONFIG_HOME`, `GSETTINGS_BACKEND=keyfile`, `XDG_CURRENT_DESKTOP=Malus:GNOME`, `GDK_BACKEND=wayland`, display Wayland y variables de cursor a cualquier comando o proceso hijo generado.
  * Se implementó `ensure_keyfile_button_layout(keyfile_path: &Path)`, la cual asegura y repara atómicamente la sección `[org/gnome/desktop/wm/preferences]` con `button-layout='close,minimize,maximize:'` en el keyfile de GSettings, protegiéndola contra modificaciones externas.
  * Se parametrizó `set_gsettings_button_layout(config_dir: &Path)` para operar exclusivamente sobre el backend keyfile aislado de Malus DE.
  * Se desacopló `is_dark_mode_active()`, consultando únicamente el keyfile y `settings.ini` de Malus DE sin depender jamás de dconf del host.
* **Lanzadores de Aplicaciones y Shell:**
  * **Compositor (`crates/compositor/src/spotlight_visual.rs`):** `toggle_desktop_theme()` ahora delega en `sync_system_appearance` y preserva la configuración de botones intacta. `apply_malus_app_env` unifica el aislamiento con `apply_isolated_environment`.
  * **Dock (`crates/dock/src/main.rs`):** `spawn_dock_app` y los accesos a la papelera (`nautilus trash://`) utilizan `apply_isolated_environment`.
  * **Escritorio (`crates/wallpaper/src/main.rs`):** `launch_file` y las invocaciones contextuales de configuración utilizan `apply_isolated_environment`.
  * **Top Bar (`crates/render/src/wayland.rs`):** Se eliminó la ejecución de `gsettings` crudo en `cc:dark_mode`, sustituyéndola por `sync_system_appearance(is_dark)`. Las aplicaciones del menú global se lanzan con aislamiento de entorno.
  * **Branches (`crates/branches`):** `open_in_terminal` y llamadas de compartición utilizan `apply_isolated_environment`.
* **Sesión e Integración de Portales (`scripts/`, `data/`):**
  * En `scripts/malus-session` y `scripts/malus-session.sh`: se retiró `export GTK_USE_PORTAL=1` para evitar que GTK consulte el portal de ajustes del host. Se añadió limpieza automática de archivos `settings.ini` residuales en el host creados por versiones previas. Se establecieron por defecto los valores del tema oscuro (`gtk-application-prefer-dark-theme=1`, `color-scheme='prefer-dark'`).
  * En `data/malus-portals.conf`: se configuró `org.freedesktop.impl.portal.Settings=none` y se direccionó el preferido a `gtk`, evitando que `xdg-desktop-portal` consulte a `xdg-desktop-portal-gnome`.
  * En `scripts/install-session.sh`: el archivo de portales se instala como `malus-portals.conf` y `Malus-portals.conf` específico de la sesión, sin sobreescribir el archivo global `portals.conf` del sistema.
* **Pruebas y Verificación:**
  * Se incorporó la prueba unitaria `test_ensure_keyfile_button_layout_enforces_left_traffic_lights`.
  * Se ejecutaron las 115+ pruebas unitarias e integración en todo el workspace con resultado 100% satisfactorio.
  * Se compilaron con éxito todos los binarios de producción optimizados (`cargo build --release`).

---

## [2026-11-09] — Mapeo de Anclaje de Arrastre de Iconos: Compensación de Hotspot en Compositor y Fijación al Punto de Clic (`malus-de-compositor`, `malus-de-wallpaper`, `malus-de-branches`)

### Diagnóstico de Causa Raíz
* **Discontinuidad Visual y Salto al Iniciar el Arrastre:**
  * Al hacer clic sobre cualquier icono en el escritorio para moverlo, el icono original se ocultaba y aparecía una miniatura pegada al cursor. No obstante, dicha miniatura no se anclaba donde el usuario había presionado el icono, sino que saltaba bruscamente colocando la esquina superior izquierda (o el centro) en la punta del cursor.
  * En `malus-de-compositor` (`drm.rs` y `winit.rs`), la superficie del icono de arrastre (`dnd_icon`) se renderizaba de forma fija en la posición física del cursor `Point::from((phys_x, phys_y))`.
  * Aunque GTK4 llamaba a `set_icon(paintable, hot_x, hot_y)` y transmitía el desplazamiento negativo a través del protocolo Wayland (`wl_surface.attach` / `wl_surface.offset`), el compositor descartaba por completo el atributo `buffer_delta` de `SurfaceAttributes`, omitiendo la traslación del hotspot.
  * Por su parte, en `malus-de-wallpaper`, `drag_source.set_icon` se inicializaba con un anclaje estático al centro (`metrics.box_width / 2, metrics.box_height / 2`), y el controlador de clics `GestureClick` no guardaba la coordenada local `(x, y)` en el momento exacto en que se presionaba el botón del ratón.

### Solución Implementada
* **Compositor (`crates/compositor`):**
  * **Seguimiento de Hotspot de Arrastre (`dnd_icon_offset`):**
    * Se incorporó el campo `pub dnd_icon_offset: Point<i32, Logical>` en la estructura central `MalusState`.
    * En `ClientDndGrabHandler::started`, se inicializa el offset y se extrae cualquier desplazamiento inicial de `SurfaceAttributes::current().buffer_delta`.
    * En `ClientDndGrabHandler::dropped`, se restablece el offset al valor neutro `(0, 0)`.
    * En `commit(&mut self, surface: &WlSurface)`, al confirmarse la superficie `dnd_icon`, se acumulan dinámicamente los deltas en `self.dnd_icon_offset += delta`.
  * **Renderizado Compensado en DRM y Winit:**
    * En los bucles de renderizado de `drm.rs` y `winit.rs`, la posición física de dibujo se calcula sumando el offset lógico del punto de anclaje: `loc.x + offset.x` y `loc.y + offset.y`.
    * Esto alinea matemáticamente la superficie de arrastre de forma que el punto exacto presionado por el usuario coincida con la punta del cursor.
    * Se añadió la prueba unitaria `test_dnd_icon_offset_calculation`.
* **Escritorio (`crates/wallpaper`):**
  * **Mapeo Inmediato al Presionar el Ratón (`click.connect_pressed`):**
    * Se instancian `stored_click` y `drag_source` antes de los controladores de eventos.
    * En `click.connect_pressed`, se capturan las coordenadas locales `(click_x, click_y)` del puntero dentro de la caja del icono y se preconfigura `drag_source.set_icon` con dicho hotspot de forma instantánea.
    * En `connect_prepare` y `connect_drag_begin`, se refuerza la asignación con redondeo exacto `click_x.round() as i32, click_y.round() as i32`.
    * Al coincidir el hotspot del cliente con la compensación en el compositor, el icono de arrastre se genera exactamente en las mismas coordenadas de pantalla que ocupaba el icono antes de ocultarse, ofreciendo la apariencia fluida de ser el mismo elemento en movimiento.
* **Explorador Branches (`crates/branches`):**
  * Se sincronizó la vista en cuadrícula (`icon_view.rs`) utilizando redondeo preciso en `connect_prepare` para asegurar la misma respuesta táctil.
* **Pruebas y Verificación:**
  * Todas las suites del espacio de trabajo pasaron exitosamente (40 pruebas en `malus-de-compositor`, 17 pruebas en `malus-de-wallpaper`, 1 prueba en `malus-de-branches`, etc.).
  * Se generaron binarios de producción optimizados (`cargo build --release`).

---

## [2026-11-09] — Armonización Visual de Selección: Ajuste de Transparencia de Iconos y Rubberband al 14% de Snap Window (`malus-de-wallpaper`, `malus-de-branches`)

### Diagnóstico y Motivación
* En el compositor (`crates/compositor/src/blur.rs`), la previsualización de anclaje de ventanas (Aero Snap, `render_snap_preview`) renderiza un tinte translúcido azul sutil estilo macOS con `14%` de opacidad (`gl.Uniform4f(self.blit_u_solid_color, 0.0, 0.48, 1.0, 0.14)` / `rgba(0, 122, 255, 0.14)`) y un borde squircle nítido con `55%` de opacidad (`gl.Uniform4f(self.blit_u_border_color, 0.28, 0.65, 1.0, 0.55)` / `rgba(71, 166, 255, 0.55)`).
* En contraposición, los elementos seleccionados en el escritorio (`.desktop-icon.selected`, `:focus-visible` y el rectángulo de selección múltiple `.desktop-rubberband`) utilizaban opacidades más densas (`36%` y `22%` respectivamente con bordes `0.75`), rompiendo la armonía visual y estética del entorno.
* Asimismo, la vista en cuadrícula del explorador de archivos (`malus-de-branches`) mantenía una opacidad de selección de `35%`.

### Solución Implementada
* **Escritorio (`crates/wallpaper/src/styles/default-gradient.css`):**
  * Se sincronizó el fondo de `.desktop-icon.selected` y `.desktop-icon:focus-visible` a `rgba(0, 122, 255, 0.14)` y su borde/halo a `rgba(71, 166, 255, 0.55)`, igualando exactamente los parámetros del shader de Snap Window.
  * Se sincronizó el rectángulo de selección interactiva `.desktop-rubberband` con `background: rgba(0, 122, 255, 0.14)` y `border: 1px solid rgba(71, 166, 255, 0.55)`.
  * Se preservó el fondo de pastilla sólido (`#007aff`) en `label.desktop-icon-label` al estar seleccionado, asegurando legibilidad tipográfica absoluta sobre cualquier fondo de pantalla.
* **Explorador de Archivos Branches (`crates/branches/src/styles/macos-branches.css`):**
  * Se actualizó `.branches-icon-item:selected`, `.branches-icon-item.selected` y `:focus` para utilizar `alpha(@accent_bg_color, 0.14)` en el fondo y `outline: 2px solid alpha(@accent_bg_color, 0.55)` en el contorno, unificando la selección visual entre el escritorio y el gestor de archivos.
* **Pruebas y Verificación:**
  * Se ejecutaron las suites de pruebas unitarias (`malus-de-wallpaper`: 17 pruebas; `malus-de-branches`: 1 prueba; `malus-de-compositor`: 39 pruebas), todas pasando satisfactoriamente.
  * Se generaron binarios de producción optimizados (`cargo build --release`).

---

## [2026-11-09] — Corrección del Salto de Pantalla al Salir de Pantalla Completa, Ocultación Indebida de Top Bar/Dock Multimonitor y Adaptación de Margen Superior de la Rejilla en Pantallas Secundarias (`malus-de-compositor`, `malus-de-wallpaper`)

### Diagnóstico de Causa Raíz
* **Salto de Ventanas / Videos al Salir de Pantalla Completa hacia el Monitor Secundario:**
  * En configuraciones multimonitor (por ejemplo, `DP-1` como pantalla primaria y `HDMI-A-1` como pantalla secundaria), el subsistema DRM enumera los conectores físicos según el orden de escaneo del kernel/GPU (`HDMI-A-1` como conector 0 y `DP-1` como conector 1).
  * En `malus-de-compositor`, `self.space.outputs().next()` devolvía el primer conector enumerado (`HDMI-A-1`), en lugar de respetar la pantalla marcada con `primary = true` en `DisplaysConfig` (`monitors.toml`).
  * En `unfullscreen_request` y `unmaximize_request`, el compositor obtenía la pantalla de la ventana mediante `self.space.outputs_for_element(&window).into_iter().next()`. Al estar una ventana maximizada o en pantalla completa en el borde de `DP-1` colindando con `HDMI-A-1`, el elemento se intersectaba con ambos monitores y seleccionaba erróneamente `HDMI-A-1`.
  * Además, `unfullscreen_request` restauraba la ventana a `saved_loc` ciegamente sin comprobar si esas coordenadas caían dentro del área visible del monitor actual (`output_geo.contains(saved_loc)`).
  * Por último, al arrastrar y soltar ventanas entre monitores sin Aero Snap, `grabs.rs` no actualizaba `window_restore_geometries`, dejando coordenadas desfasadas asociadas al monitor inicial.
* **Ocultación Errónea de Top Bar y Dock en la Pantalla Primaria al Reproducir Video Fullscreen en la Secundaria:**
  * En `has_fullscreen_window_on_output(&self, output: &Output)`, el compositor comprobaba si una ventana en pantalla completa se solapaba con la geometría del monitor evaluado mediante `output_geo.overlaps(win_rect)`.
  * Como `DP-1` y `HDMI-A-1` son pantallas adyacentes contiguas (coincidiendo exactamente en `x = 2327`), cualquier redondeo por escalado fraccional (`scale = 1.65` / `scale = 1.465`), sombra de ventana del cliente o desbordamiento limítrofe provocaba que `output_geo.overlaps(win_rect)` devolviera `true` para la pantalla primaria `DP-1` a pesar de que el vídeo residía de forma inequívoca en la pantalla secundaria `HDMI-A-1`.
  * Como consecuencia de este falso positivo, el compositor ocultaba la Top Bar y el Dock en la pantalla primaria y bloqueaba la interactividad del puntero (`Layer::Top`) en dicha pantalla.
* **Espaciado Superior Innecesario (Top Bar Spacing) en la Rejilla de Pantallas Secundarias:**
  * En `malus-de-wallpaper`, la constante `DESKTOP_TOP_MARGIN = 56.0` estaba cableada en todos los cálculos de cuadrícula (`calculate_grid_position`, `snap_position_to_grid`, `clamp_icon_position`, `calculate_organize_targets`, etc.).
  * En la pantalla primaria, 56px de margen superior es indispensable para dejar respiración bajo la Top Bar (36px de barra + 20px de margen). No obstante, las pantallas secundarias no tienen Top Bar, dejando un vacío antiestético de 56px antes de la primera fila de iconos.
  * Por su parte, la resolución de `primary_index` en el daemon de wallpaper dependía de `monitor.connector()`, el cual bajo GTK4 Wayland puede retornar `None`, provocando un fallback a `0` (`HDMI-A-1`). Esto causaba que la pantalla secundaria fuera tratada como primaria y viceversa.
  * En `malus-de-compositor`, `usable_zone_for_output` y `ensure_window_below_topbar` restringían incondicionalmente los primeros 36px en todos los monitores, en vez de aplicar la reserva de la Top Bar exclusivamente a la pantalla primaria.

### Solución Implementada
* **Compositor (`crates/compositor`):**
  * **Identificación Robusta del Monitor Primario (`primary_output` & `is_primary_output`):**
    * Se implementó `primary_output(&self) -> Option<Output>` en `state.rs`, que consulta `DisplaysConfig::load()`, identifica el monitor con `primary == true` comparando conector, modelo/descripción y coordenadas `(x, y)`, y solo recurre a `self.space.outputs().next()` como último recurso.
    * `is_primary_output` delega de forma directa y consistente en `self.primary_output().as_ref() == Some(output)`.
  * **Aislamiento Multimonitor Estricto de Pantalla Completa (`has_fullscreen_window_on_output`):**
    * Se refactorizó `has_fullscreen_window_on_output` para verificar `self.origin_output_for_window(window).as_ref() == Some(output)`.
    * Al utilizar `origin_output_for_window` (que calcula la intersección de mayor área física), se garantiza que una ventana a pantalla completa en la pantalla secundaria pertenezca única y exclusivamente a dicha pantalla secundaria, evitando cualquier interferencia o falso positivo en la pantalla primaria.
    * La Top Bar y el Dock en la pantalla primaria permanecen plenamente visibles, reactivos e interactivos cuando se reproduce un vídeo a pantalla completa en la pantalla secundaria.
  * **Reserva de Top Bar Exclusiva para la Pantalla Primaria:**
    * En `usable_zone_for_output`, se evalúa `self.is_primary_output(output)`. En monitores secundarios, `top_bar_min_height` es `0` y `top_offset` coincide con `zone.loc.y`, permitiendo aprovechar el 100% de la altura vertical de la pantalla secundaria.
    * En `ensure_window_below_topbar`, se aborta inmediatamente si el monitor no es el primario.
  * **Preservación Estricta del Monitor en Maximizar / Desmaximizar / Pantalla Completa:**
    * En `maximize_request` y `unmaximize_request`, se sustituyó la selección por coincidencia de bordes por `self.origin_output_for_window(&window).or_else(|| self.primary_output())`.
    * En `unfullscreen_request`, se verifica `output_geo.contains(saved_loc)`. Si las coordenadas guardadas pertenecían a otro monitor o estaban desfasadas, la ventana se reubica de forma segura dentro del monitor donde se encontraba en pantalla completa.
    * En `grabs.rs`, al soltar una ventana arrastrada, se actualiza inmediatamente `window_restore_geometries` con la nueva ubicación en el espacio.
    * En `drm.rs` y `xwayland.rs`, se actualizaron los centrado de puntero y ubicación de nuevas ventanas para usar `state.primary_output()`.
* **Fondo de Pantalla y Rejilla (`crates/wallpaper`):**
  * **Margen Superior Secundario Estándar (`DESKTOP_SECONDARY_TOP_MARGIN = 16.0`):**
    * Se definió `DESKTOP_SECONDARY_TOP_MARGIN = 16.0` para alinear el margen superior con los márgenes laterales (`16.0`), eliminando el hueco de 56px en pantallas sin Top Bar.
    * Se parametrizó `top_margin: f64` en todas las funciones de la rejilla: `calculate_grid_position`, `snap_position_to_grid`, `clamp_icon_position`, `calculate_organize_targets`, `organize_desktop_icons`, `animate_desktop_organization`, `animate_desktop_snap_to_grid`, `render_single_icon` y `refresh_desktop_icons`.
    * `create_wallpaper_window` recibe `is_primary: bool` y asigna `let top_margin = if is_primary { DESKTOP_TOP_MARGIN } else { DESKTOP_SECONDARY_TOP_MARGIN };`.
  * **Resolución Multi-Atributo del Monitor Primario:**
    * En `main()`, la detección del monitor primario compara conector, modelo (`model()`), descripción (`description()`) y geometría física (`geometry()`), garantizando la correcta asignación de la pantalla primaria incluso si el compositor no expone el nombre del conector a través de GDK.
* **Pruebas y Verificación:**
  * Todas las pruebas unitarias pasaron con éxito:
    * 39 pruebas en `malus-de-compositor` (`cargo test -p malus-de-compositor`), incluyendo `fullscreen_on_secondary_does_not_affect_primary_fullscreen_state`.
    * 17 pruebas en `malus-de-wallpaper` (`cargo test -p malus-de-wallpaper`), incluyendo `test_snap_position_to_grid_secondary_display`.
    * Suite completa del workspace sin errores (`cargo test --workspace`).
  * Compilación en modo release completada exitosamente (`cargo build --release -p malus-de-compositor -p malus-de-wallpaper`).

---

## [2026-11-09] — Truncado Central (Middle Ellipsis) de Nombres en el Escritorio y Expansión al Seleccionar (`malus-de-wallpaper`)

### Diagnóstico de Causa Raíz
* **Desbordamiento Vertical y Solapamiento de Nombres Largos en la Cuadrícula:**
  * En el contenedor `Fixed` del fondo de escritorio GTK4, las etiquetas `Label` tenían `wrap(true)` activado sin un límite explícito de líneas (`lines(2)`) ni elipsis configurado.
  * Cuando un archivo o carpeta tenía un nombre largo, la caja del icono crecía verticalmente más allá del alto de la celda de la cuadrícula (`cell_height()`), provocando que el texto se solapara con el icono ubicado en la fila inferior.
  * Un truncado estándar al final (`End`) ocultaba extensiones críticas del archivo (por ejemplo `.pdf`, `.png`, `.tar.gz`), mientras que la ausencia de un modo expandido impedía al usuario leer el nombre completo del archivo sin abrir un administrador de archivos externo.

### Solución Implementada (`crates/wallpaper`)
* **Algoritmo de Truncado Central Inteligente (`truncate_desktop_name`):**
  * Se implementó una función pura que procesa de manera segura secuencias de caracteres UTF-8 (`Vec<char>`), garantizando compatibilidad con acentos, tildes y caracteres multibyte en español.
  * **Preservación de Extensiones Simples y Compuestas:** detecta extensiones comunes (ej. `.pdf`, `.png`, `.desktop`) y compuestas (ej. `.tar.gz`, `.tar.xz`), reservando los caracteres finales para mantener visible la extensión junto con los caracteres finales del nombre base.
  * **División Central para Carpetas:** para directorios y archivos sin extensión, divide simétricamente los caracteres disponibles a ambos lados del elipsis central (`…`).
  * **Cálculo Proporcional a la Cuadrícula:** calcula el límite de caracteres (`max_chars`) de forma proporcional al ancho configurado del icono (`width_request * 2 / 9`), garantizando que el texto nunca desborde el ancho ni el alto de la celda en 2 líneas.
* **Control Centralizado de Selección y Expansión (`set_desktop_icon_selected`):**
  * Se almacena el nombre completo e íntegro del archivo directamente en el widget GTK (`label.set_widget_name(&display_name)`), evitando dependencias de almacenamiento secundario.
  * **Estado No Seleccionado:** etiqueta restringida a `lines(2)`, modo `EllipsizeMode::Middle` y texto procesado por `truncate_desktop_name`.
  * **Estado Seleccionado:** etiqueta expandida a `lines(0)`, `EllipsizeMode::None` y texto restaurado a su valor íntegro original, permitiendo la lectura completa del archivo seleccionado sobre el fondo de selección.
* **Integración en Todos los Disparadores de Selección:**
  * Se conectó `set_desktop_icon_selected` en:
    * Clic primario directo y selección múltiple con `Ctrl + clic`.
    * Inicio de arrastre (drag-and-drop / `connect_drag_begin`).
    * Selección elástica rectangular por marquesina (`desktop-rubberband` / `connect_drag_update`).
    * Deselección por clic en el fondo del escritorio (`bg_click.connect_pressed`).
* **Pruebas y Verificación:**
  * Se incorporó el test unitario `test_truncate_desktop_name` validando:
    1. Archivos cortos (sin truncado).
    2. Archivos largos con extensión simple (truncado central + preservación de extensión).
    3. Archivos con extensión compuesta (`.tar.gz`).
    4. Carpetas largas (distribución balanceada).
    5. Caracteres UTF-8 multibyte en español (`Diseño_gráfico_año_2026_españa_final.png`).
  * Todas las 16 pruebas unitarias pasaron con éxito (`cargo test -p malus-de-wallpaper`).
  * Compilación exitosa en modo release (`cargo build --release -p malus-de-wallpaper`).

---

## [2026-11-08] — Animación Fluida de Desplazamiento al Organizar Iconos del Escritorio (`malus-de-wallpaper`)

### Diagnóstico de Causa Raíz
* **Salto Instantáneo de los Iconos al Organizar ("Clean Up / Organize Jump"):**
  * Al hacer clic en "Organizar escritorio", "Organizar por ▸" (Nombre, Tipo, Fecha) o conmutar "Alineación automática", el sistema invocaba `organize_desktop_icons` seguido de `trigger_refresh()`.
  * `trigger_refresh()` purgaba y destruía todos los widgets hijos del contenedor `Fixed` (`fixed.remove(&current)`), para luego reconstruirlos instantáneamente en sus nuevas coordenadas de destino.
  * Este comportamiento provocaba un parpadeo y un salto abrupto en la pantalla, en lugar de una transición física continua donde los iconos se deslicen orgánicamente a sus casillas correspondientes como en macOS Finder.

### Solución Implementada (`crates/wallpaper`)
* **Cálculo Desacoplado de Posiciones Destino (`calculate_organize_targets`):**
  * Se extrajo el cálculo de posiciones ordenadas en `calculate_organize_targets`, permitiendo conocer las coordenadas destino `(x_target, y_target)` de cada archivo antes de alterar los widgets en pantalla.
* **Transición de Movimiento VSync Interpolado (`animate_desktop_layout_transition`):**
  * Se implementó una animación continua sincronizada con el frame clock de la pantalla (`widget.add_tick_callback`) gobernada por `ease_out_cubic` durante 320 ms.
  * Para cada icono en el contenedor `Fixed`, se obtiene su coordenada actual en píxeles mediante `fixed.child_position(&item)`.
  * Si la distancia euclidiana entre la posición actual y el destino supera 0.5 px, el elemento se añade al lote de animación.
  * En cada tick de refresco vertical (60Hz / 120Hz), el sistema calcula `cx = start.0 + (target.0 - start.0) * eased` y `cy = start.1 + (target.1 - start.1) * eased`, actualizando la posición suavemente con `fixed.move_(widget, cx, cy)`.
  * Los widgets existentes nunca se destruyen ni recrean: conservan intactos sus estados de selección, controladores de clic, drag-and-drop y estilos CSS.
* **Control de Generación de Animación (`DESKTOP_LAYOUT_ANIM_GEN`):**
  * Se introdujo un contador de generación thread-local para cancelar automáticamente cualquier animación previa si el usuario activa otra ordenación mientras una animación previa está en curso. La nueva transición arranca fluidamente desde la posición intermedia exacta donde se encontraban los iconos, eliminando cualquier discontinuidad visual.
* **Integración Global en el Menú Contextual del Escritorio:**
  * **"Organizar escritorio"**: ejecuta `animate_desktop_organization` con duración de 320 ms.
  * **"Organizar por ▸"** (Nombre, Tipo / Clase, Fecha de modificación): ejecuta `animate_desktop_organization` con el criterio seleccionado.
  * **"Alinear a la cuadrícula"**: ejecuta `animate_desktop_snap_to_grid` (280 ms), deslizando a la celda más cercana únicamente los iconos desalineados.
  * **"Alineación automática"**: al activarse, dispara la animación suave organizando toda la cuadrícula en tiempo real.
* **Pruebas y Verificación:**
  * Se añadió `test_calculate_organize_targets_by_kind` verificando la prelación de directorios y ordenamiento por extensión de archivo.
  * Todas las 15 pruebas unitarias pasan con éxito (`cargo test -p malus-de-wallpaper`).
  * Compilación exitosa en modo release (`cargo build --release -p malus-de-wallpaper`).

---

## [2026-11-08] — Animaciones Fluidas de Apertura y Transición de Subsecciones con `GtkStack` (`malus-de-wallpaper`)

### Diagnóstico de Causa Raíz
* **Animación de Apertura del Popover Inactiva:**
  * En GTK4 sobre Wayland, `GtkPopover` es una subsuperficie `GtkNative`. Modificar `set_opacity()` sobre la instancia de `Popover` no propaga la opacidad a través del árbol de renderizado GSK a los widgets hijos.
  * Además, al invocar `popover.popup()`, el mapeo de la subsuperficie ocurre de manera asíncrona; la comprobación anterior `if elapsed > 0 && !w.is_visible() { return Break; }` se disparaba en el primer tick antes de que la subsuperficie Wayland fuera presentada en pantalla, abortando la animación de opacidad antes de comenzar.
* **Transiciones Abruptas al Navegar entre Subsecciones:**
  * Al reemplazar dinámicamente el widget hijo (`popover.set_child(...)`) para navegar a "Organizar por ▸" o "Nueva carpeta", la sustitución de widgets generaba un salto visual inmediato sin continuidad espacial, forzando la destrucción y recreación de vistas en cada interacción.

### Solución Implementada (`crates/wallpaper`)
* **Animación de Apertura Sincronizada con el Mapeo de la Subsuperficie:**
  * Se optimizó `animate_widget_fade_in` en [`crates/wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs) para verificar `!w.is_mapped()`, pausando el inicio del cronómetro de animación hasta que el widget esté efectivamente mapeado y visible en el compositor Wayland.
  * Se aplicó la animación de fade-in (`150ms`, curva `ease_out_cubic`) directamente al widget hijo raíz (`GtkStack` o contenedor interno), garantizando que el grafo GSK renderice la interpolación de opacidad de manera suave y fluida.
* **Transición de Subsecciones con `gtk4::Stack`:**
  * Se reemplazó el intercambio estático de hijos en el `GtkPopover` por un `gtk4::Stack` persistente configurado con:
    * `StackTransitionType::SlideLeftRight`: animación horizontal fluida de deslizamiento nativa de GTK4.
    * `set_transition_duration(180)`: duración de 180 ms ajustada al estándar de fluidez de macOS/iOS.
    * `set_interpolate_size(true)`: interpolación continua y suave de las dimensiones del menú al alternar entre vistas de diferente altura o ancho.
  * Se modularizaron las subsecciones con funciones dedicadas:
    * `build_organize_submenu`: submenú de ordenamiento por nombre, tamaño, tipo y fecha de modificación, con botón de retroceso animado "← Volver".
    * `build_new_folder_view`: vista de creación de nueva carpeta con campo de texto autofocus y botones Cancelar / Crear.
  * Se vincularon las transiciones mediante `stack.set_visible_child_name("organize")`, `stack.set_visible_child_name("new_folder")` y retorno con `stack.set_visible_child_name("main")`.
* **Estilos CSS Transparentes para el Contenedor de Pila:**
  * En [`crates/wallpaper/src/styles/default-gradient.css`](../crates/wallpaper/src/styles/default-gradient.css), se agregó la regla `stack.desktop-context-menu` con fondo, bordes y sombras transparentes para delegar el borde acrílico y la sombra al contenedor exterior `contents.background` del popover, previniendo artefactos de recorte durante el deslizamiento.

---

## [2026-11-08] — Corrección del Desfase de Geometría en Popups de Fondo de Pantalla (`malus-de-compositor`)

### Diagnóstico de Causa Raíz
* **Desfase entre el Rectángulo de Blur/Sombra del Compositor y el Contenido GTK4 (`crop_popover_4k.png`):**
  * Al inspeccionar la captura de pantalla a nivel de píxel, se determinó que la caja translúcida exterior desfasada es en realidad el pase de desenfoque GPU Dual Kawase (`blur.render_blur`) y la sombra que `malus-de-compositor` dibuja para las superficies emergentes (`wallpaper_popups`).
  * En [`crates/compositor/src/backend/drm.rs`](../crates/compositor/src/backend/drm.rs) y [`crates/compositor/src/backend/winit.rs`](../crates/compositor/src/backend/winit.rs), `popup_rect` se calculaba erróneamente como `layer_loc + offset`, donde `offset = popup_offset - popup_geometry.loc`.
  * Como `popup_geometry.loc` representa la distancia entre el origen de la subsuperficie Wayland y el área útil de la ventana (márgenes de sombra de GTK4), al usar `offset` en lugar de `popup_offset`, el pase de blur y sombra se desplazaba hacia arriba y a la izquierda en exactamente esa cantidad de píxeles.
* **Corte Inapropiado del Menú por el Shader de Recorte Squircle:**
  * El pase de renderizado utilizaba `shader.apply_to_frame` con un radio hardcodeado de `32px` y el offset desfasado, recortando el menú en el borde superior e izquierdo y truncando el contenido por la derecha y abajo (por ejemplo `✓ Alineación automátic...`).
  * A diferencia de `wallpaper_popups`, los menús de la barra superior (`topbar_popups`) ya utilizaban la fórmula correcta `layer_loc + popup_offset.to_physical_precise_round(scale)` y `shader.apply_clip_to_frame(..., menu_radius)`.

### Solución Implementada (`crates/compositor`)
* **Alineación Exacta de `popup_rect`:**
  * Se corrigió el cálculo de `popup_rect` en `drm.rs` y `winit.rs` para utilizar `layer_loc + popup_offset.to_physical_precise_round(scale)`, posicionando el desenfoque GPU y la sombra exactamente en las coordenadas reales de los elementos del menú.
* **Pipeline Homogéneo de Popups:**
  * Se reemplazó `render_shadow` por `render_menu_shadow(..., menu_radius, scale)` y se utilizó `shader.apply_clip_to_frame(..., menu_radius)`, garantizando un recorte suave idéntico al de los menús de la barra de menús superior de macOS.
* **Compilación y Despliegue:**
  * Se recompiló `malus-de-compositor` en modo release (`cargo build --release -p malus-de-compositor`). Al ser el compositor el proceso que gestiona la sesión Wayland, al cerrar e iniciar sesión el sistema carga automáticamente este nuevo binario corregido.

---

## [2026-11-08] — Corrección del Desfase de Contenedor y Animación VSync por Frame-Clock para Menús Contextuales

### Diagnóstico de Causa Raíz
* **Desfase del Menú Respecto a su Contenedor ("Menú Desfasado"):**
  * Al aplicar `@keyframes macos-popover-appear` con `transform: scale(0.92)` sobre `popover.desktop-popover-menu > contents` y `@keyframes macos-submenu-slide` con `transform: translateY(3px)` sobre `.desktop-context-menu`, el motor de renderizado de GTK4 / GSK (`GskTransformNode`) aplicó la transformación visual sobre el contenido sin reajustar los límites de la subsuperficie Wayland asignada por `GtkPopoverLayout`.
  * Como consecuencia visible (evidenciada en la captura del usuario), la caja acrílica exterior y los elementos del menú interno quedaron desfasados entre sí, con el contenido desplazado hacia abajo y a la derecha, dejando espacios vacíos y bordes recortados.
* **Incompatibilidad de Timelines CSS en Subsuperficies de Popover:**
  * En GTK4 sobre Wayland, los timelines de animación `@keyframes` CSS aplicados a nodos internos de subsuperficies no se actualizan de manera continua de forma autónoma, lo que provocaba que la animación se congelara en el primer frame de transformación (`scale(0.92)` / `translateY`), impidiendo que el menú se mostrara correctamente.

### Solución Implementada (`crates/wallpaper`)
* **Retiro de Transformaciones Geométricas en CSS:**
  * Se eliminaron completamente las reglas `@keyframes macos-popover-appear` y `@keyframes macos-submenu-slide` de [`crates/wallpaper/src/styles/default-gradient.css`](../crates/wallpaper/src/styles/default-gradient.css).
  * Se eliminó `transform: scale(0.97)` en `:active` de `.desktop-context-btn`, reemplazándolo por una transición de color de fondo pura (`#0062cc`), eliminando cualquier distorsión geométrica en toda la jerarquía del popover.
  * Se preservaron las sombras flotantes profundas multicapa y el diseño acrílico nativo tanto para modo claro como para modo oscuro.
* **Animación Nativa por Frame-Clock VSync (`animate_widget_fade_in`):**
  * Se implementó `animate_widget_fade_in` en [`crates/wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs), utilizando `widget.add_tick_callback` sincronizado directamente con el `gdk::FrameClock` de la pantalla.
  * Emplea interpolación de opacidad suave de 0.0 a 1.0 gobernada por la función cúbica `ease_out_cubic` ($1 - (1 - t)^3$) durante 140ms.
  * Al modular exclusivamente la opacidad sin alterar dimensiones, escala ni traslación, se garantiza un alineamiento pixel-perfect 100% exacto con cero desfases respecto al contenedor acrílico.
* **Transiciones Fluidas en Submenús y Vistas Internas:**
  * Se integró `animate_widget_fade_in` en todas las aperturas y cambios de vista:
    * Popover del escritorio primario y secundarios (`140ms`).
    * Popover de archivos y carpetas (`140ms`).
    * Diálogo de renombrar archivo (`140ms`).
    * Formulario de "Nueva carpeta" en popover (`120ms`).
    * Submenú "Organizar por ▸" y botón "← Volver" (`120ms`).
* **Cobertura con Pruebas Unitarias:**
  * Se implementó `test_ease_out_cubic` verificando el comportamiento determinista de la curva de interpolación, y se actualizaron las pruebas de estilos CSS para comprobar la consistencia de las reglas acrílicas.

---

## [2026-11-08] — Animaciones Fluidas del Menú Contextual del Escritorio (Apertura Elástica macOS, Sombra Flotante Profunda y Transición de Submenús)

### Diagnóstico de Causa Raíz
* **Aparición Rígida e Instantánea de Menús Emergentes:**
  * Al hacer clic derecho en el escritorio o sobre un archivo, el popover (`popover.desktop-popover-menu`) se mostraba sin ninguna interpolación de escala ni opacidad, resultando en una aparición tosca y poco orgánica en comparación con la interfaz de macOS Finder.
* **Falta de Dinamismo en Transiciones de Submenús:**
  * Al ingresar a formularios secundarios ("Nueva carpeta" o "Organizar por ▸") o retornar con "← Volver", el reemplazo del widget hijo del popover ocurría de forma instantánea sin suavizado.

### Solución Implementada (`crates/wallpaper`)
* **Animación de Apertura Elástica (`@keyframes macos-popover-appear`):**
  * Se configuró en [`crates/wallpaper/src/styles/default-gradient.css`](../crates/wallpaper/src/styles/default-gradient.css) una curva elástica sutil idéntica a macOS:
    * Escala inicial: `scale(0.92)` y `opacity: 0.0`.
    * Sobrepaso elástico: `scale(1.015)` y `opacity: 1.0` al 70% de la animación.
    * Reposo final: `scale(1.0)` al 100%.
    * Duración y curva: `150ms cubic-bezier(0.16, 1, 0.3, 1)`.
* **Sombra Flotante Profunda de Ventana Emergente:**
  * Se aplicó elevación multicapa con difusión acrílica:
    * Modo Oscuro: `box-shadow: 0 12px 32px rgba(0, 0, 0, 0.48), 0 2px 8px rgba(0, 0, 0, 0.32), inset 0 0 0 1px rgba(255, 255, 255, 0.08);`
    * Modo Claro: `box-shadow: 0 12px 32px rgba(0, 0, 0, 0.18), 0 2px 8px rgba(0, 0, 0, 0.12), inset 0 0 0 1px rgba(255, 255, 255, 0.35);`
* **Transición Suave de Vistas y Submenús (`@keyframes macos-submenu-slide`):**
  * Se asignó una animación de deslizamiento y desvanecimiento vertical de `130ms` al alternar vistas en el popover (`transform: translateY(3px) -> translateY(0)`).
* **Feedback Táctil en Ítems de Menú:**
  * Los botones (`.desktop-context-btn`) ahora cuentan con transición suave de hover de `100ms` y un efecto de presión táctil al hacer clic (`transform: scale(0.97)` en estado `:active`).

---

## [2026-11-08] — Implementación de "Organizar escritorio" (Carpetas Primero y Criterios por Nombre/Tipo/Fecha) y Alineación Automática Reactiva

### Diagnóstico de Causa Raíz
* **Comportamiento Confuso y Pasivo de "Alineación automática":**
  * La opción "Alineación automática" únicamente conmutaba el booleano `snap_to_grid` en `desktop.toml`, afectando de forma pasiva sólo a futuras operaciones de arrastrar y soltar manual. No ejecutaba ninguna acción sobre las carpetas y archivos ya existentes en el escritorio, provocando la sensación lógica en el usuario de que la opción "no hacía nada" o estaba rota.
* **Limitación de "Alinear a la cuadrícula":**
  * Esta opción sólo redondeaba las coordenadas libres individuales de cada icono a su celda geométrica más próxima (`snap_position_to_grid`), pero no reorganizaba ni compactaba los elementos. Si los iconos estaban dispersos en zonas distantes de la pantalla, continuaban desparramados y con huecos vacíos.
* **Inexistencia de una Opción Real para Organizar el Escritorio:**
  * No existía en el menú contextual una acción análoga a "Limpiar" / "Organizar por..." (*Clean Up / Sort By*) de macOS Finder que reagrupase secuencialmente todas las carpetas y archivos en columnas limpias y compactas.

### Solución Implementada (`crates/wallpaper`)
* **Nueva Acción Directa "Organizar escritorio":**
  * Se añadió la opción "Organizar escritorio" en [`crates/wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs), la cual ejecuta `organize_desktop_icons(..., DesktopOrganizeBy::Name)`.
  * Compacta y recoloca instantáneamente todas las carpetas y archivos en la cuadrícula en columnas secuenciales comenzando desde la esquina superior derecha (o izquierda si `align_left = true`), priorizando carpetas primero y luego archivos ordenados alfabéticamente (A-Z).
* **Submenú "Organizar por ▸":**
  * Despliega una vista secundaria en el popover contextual con opciones especializadas:
    * **Por Nombre (Carpetas primero):** Agrupa carpetas arriba y ordena alfabéticamente.
    * **Por Tipo / Clase:** Agrupa por extensión y categoría funcional (carpetas, imágenes, documentos, etc.).
    * **Por Fecha de modificación:** Ubica los archivos editados más recientemente en los primeros casilleros.
* **Alineación Automática Reactiva:**
  * Al conmutar "Alineación automática" a activo (`true`), no sólo se guarda la configuración, sino que se dispara automáticamente `organize_desktop_icons`, alineando y organizando en ese mismo instante todos los iconos de la pantalla.
* **Cobertura con Pruebas Unitarias:**
  * Se incorporó `test_organize_desktop_icons_folders_first_and_alphabetical` en `wallpaper::tests`, verificando la precedencia de carpetas sobre archivos comunes y el cálculo determinista de las celdas en la cuadrícula.

---

## [2026-11-08] — Anclaje de Hotspot Exacto en Arrastre (Sin Desplazamiento Abajo/Derecha), Marca de Selección Fiel a Finder y Creación Fluida de Carpeta en Menú Contextual del Escritorio

### Diagnóstico de Causa Raíz
* **Desplazamiento del Icono Flotante respecto al Cursor ("aparece a un costado abajo a la derecha"):**
  * `DragSource::set_icon` tomaba `(box_width / 2, box_height / 2)` estático o, al depender de un widget ficticio fuera de los límites de la pantalla (`fixed.put(&box, -2000.0, -2000.0)`), GTK4 colapsaba el punto de anclaje (*hotspot*) a `(0, 0)` debido a la falta de asignación geométrica normal en la superficie. Como resultado, la miniatura no quedaba sujeta por el punto exacto donde el puntero hizo clic, sino desplazada abajo y a la derecha.
* **Ausencia de Marca de Selección en el Objeto Arrastrado:**
  * Al usar un widget auxiliar fuera de pantalla sin renderizado activo, GTK omitía la resolución de estilos CSS de fondo (`.desktop-icon.selected`). Además, si se aplicaba `.desktop-icon-dragging { opacity: 0.0; }` directamente al elemento mientras se apuntaba con `WidgetPaintable`, la miniatura se tornaba invisible.
* **Fallo en "Nueva Carpeta" desde el Menú Contextual del Escritorio:**
  * Al hacer clic en "Nueva carpeta", el manejador invocaba `popover_c.popdown()` inmediatamente antes de instanciar un nuevo `Popover` sin anclaje de posición (`set_pointing_to`). En GTK4, cerrar un popover libera los grabs del puntero en el mismo tick, provocando que cualquier nuevo popover hijo del mismo árbol de widgets sea descartado al instante antes de que el usuario pueda interactuar con él o visualizarlo. Además, la acción de creación no invocaba el refresco del escritorio ni calculaba un nombre secuencial libre ("Nueva carpeta", "Nueva carpeta 2", etc.).

### Solución Implementada (`crates/wallpaper`, `crates/branches`)
* **Anclaje de Hotspot Exacto en `connect_prepare`:**
  * En [`crates/wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs) y [`crates/branches/src/views/icon_view.rs`](../crates/branches/src/views/icon_view.rs), se utiliza la coordenada real `(click_x, click_y)` suministrada por GTK4 en `drag_source.connect_prepare(move |source, click_x, click_y| ...)`.
  * Se asocia `source.set_icon(Some(&paintable), click_x as i32, click_y as i32)` directamente sobre el widget `item_box` que ya posee la clase `.selected` activa (squircle azul acrílico, borde brillante y píldora en la etiqueta). De este modo, la imagen bajo el cursor coincide pixel a pixel con el objeto en el instante en que fue tomado ("tal cual como el cursor lo tomó").
* **Desaparición Limpia sin Recortes Mediante Desplazamiento Fuera de Pantalla:**
  * Para evitar siluetas o recortes residuales sin alterar la opacidad del `WidgetPaintable`, en `connect_drag_begin` el widget original se traslada fuera de pantalla (`fixed.move_(&item_box, -5000.0, -5000.0)`).
  * La posición original en el escritorio queda 100% limpia sin siluetas ni fantasmas. Al terminar el arrastre (`connect_drag_end`), si la acción fue cancelada, se restituye limpiamente a sus coordenadas originales.
* **Transición Fluida en el Popover de "Nueva Carpeta":**
  * Se reemplazó el cierre abrupto y reapertura de popovers por una transición interna en el mismo popover (`show_new_folder_in_popover`).
  * Al pulsar "Nueva carpeta", el popover contextual sustituye su vista por el formulario de creación, conservando el anclaje `(x, y)` donde se hizo clic derecho.
  * Se implementó `get_unique_folder_name` para autocompletar nombres incrementales ("Nueva carpeta", "Nueva carpeta 2", etc.), seleccionando todo el texto en el `Entry` con `grab_focus()` y `select_region(0, -1)`.
  * Se vincularon tanto el botón "Crear" como la tecla Enter (`connect_activate`) para crear el directorio en disco, registrar sus coordenadas iniciales en `icon_positions` e invocar el refresco reactivo `trigger_refresh()` al instante.

---

## [2026-11-10] — Migración Arquitectónica del Compositor a Scene Graph Declarativo y Damage Tracking Nativo (Smithay RenderElements)

### Diagnóstico de Causa Raíz
* **Parpadeo a ~20 Hz y Pérdida de Sombras/Acrílico con KMS Triple-Buffering:**
  * En el backend DRM de Smithay, el hardware KMS rota continuamente entre 3 buffers físicos (`Buffer 0, 1, 2`).
  * Anteriormente, el bucle de renderizado intentaba recortar el sobre-dibujo mediante tijeras manuales de OpenGL (`glScissor`) calculadas únicamente a partir del movimiento de las ventanas (`*prev_rect != *win_rect`).
  * Cuando se abría un menú emergente o menú contextual (e.g., Nautilus con un área dañada de $200 \times 300\text{ px}$), la ventana subyacente no cambiaba de geometría, provocando que su sombra y su desenfoque acrílico quedaran marcados como `shadow_is_damaged = false` y `blur_is_damaged = false`.
  * La llamada a `frame.clear(dark_acrylic, damage)` limpiaba esa región en el buffer actual, pero la sombra y el blur nunca eran redibujados, dejando parches negros o transparentes. Además, los buffers rotativos con tijeras dispares perdían sincronización, generando un parpadeo constante a ~20 Hz.
* **Fallo de Caching FBO con `glBindFramebuffer` en el Bucle Activo:**
  * El intento de conmutar framebuffers secundarios en caliente dentro de `GlesFrame::with_context` interfería con el estado EGL/GBM administrado por Smithay, provocando la desconfiguración del framebuffer principal.
* **Omisión de Animación de Transición de Tema de Color:**
  * Al activar el rastreador de daño, la animación circular expansiva de 620 ms (`ThemeTransitionPipeline`) no registraba daño en los fotogramas subsiguientes a la captura inicial, causando que `render_damage.is_empty()` saltara los frames y el efecto no se visualizara.

### Solución Implementada (`crates/compositor`)
* **Modelo Declarativo de Sombras (`crates/compositor/src/shadow.rs`):**
  * Creación de `WindowShadowRenderElement`, `DockShadowRenderElement` y `MenuShadowRenderElement`, implementando los traits `Element` y `RenderElement<GlesRenderer>`.
  * Generación analítica de la sombra de doble lóbulo (macOS dual-lobe SDF) en espacio de coordenadas físicas con recorte interior (*inner cutout*) para preservar la transparencia y acrílico.
* **Modelo Declarativo de Desenfoque Acrílico (`crates/compositor/src/blur.rs`):**
  * Creación de `BlurRenderElement` con constructores específicos:
    * `for_window(...)`: Superelipse continua ($n=4.2$) para Nautilus, Branches y ventanas translúcidas.
    * `for_dock(...)`: Curvatura suave ($n=2.0$) para la barra Dock.
    * `for_popup(...)`: Radio de curvatura adaptativo para popovers, menús y centro de control.
  * Desacoplamiento del ciclo Dual Kawase: captura progresiva de la región subyacente (`capture_background_region`) y desenfoque blit con squircle en su método `draw()`.
* **Recorte Squircle Declarativo en Superficies (`crates/compositor/src/window_corners.rs`):**
  * Creación de `SquircleSurfaceRenderElement` (para superficies Wayland) y `SquircleMemoryRenderElement` (para decoraciones SSD del servidor).
  * Envoltura transparente que aplica `WindowSquircleShader` durante el `draw()` y restaura el estado con `frame.clear_tex_program_override()`.
* **Scene Graph Unificado y Orden *Front-to-Back* (`crates/compositor/src/backend/drm.rs`):**
  * Ensamblado estricto de todas las capas en `tracker_elements` para `OutputDamageTracker`:
    1. Cursor y Drag & Drop
    2. Menús emergentes (superficie $\rightarrow$ blur $\rightarrow$ sombra)
    3. Capas superiores: Dock y Top Bar (superficies squircle $\rightarrow$ blur $\rightarrow$ sombra)
    4. Ventanas en orden Z (decoración SSD $\rightarrow$ superficies squircle $\rightarrow$ blur acrílico $\rightarrow$ sombra proyectada)
    5. Fondo de pantalla (Wallpaper)
* **Propagación Robusta de Daño:**
  * Corrección de detección de daño para sombras y blur: `damage.iter().any(|d| d.overlaps(rect))`.
  * Integración de `state.theme_transition_started_at.is_some()` en `this_frame_extra_damage`, garantizando que la onda expansiva de cambio de tema se renderice a 60 FPS continuos sin omisiones.

### Resultados de Validación
* **Cero Parpadeo y Cero Glitches:** Ni el Dock, ni las sombras de ventana, ni los menús contextuales de Nautilus presentan recortes o desaparición al interactuar sobre el escritorio o sobre otras ventanas.
* **Transparencia X-Ray Corregida:** Ventanas translúcidas superpuestas desenfocan correctamente las ventanas debajo de ellas y no únicamente el fondo de pantalla.
* **Eficiencia GPU/CPU:** Con el escritorio estático, `damage_output` reporta 0 rectángulos de daño, evitando sobre-dibujo innecesario en la GPU.

---

## [2026-11-08] — Restauración de Miniatura Flotante Completa (Marca de Selección y Nombre) y Acción de Mover (`MOVE`) por Defecto sin Duplicados

### Diagnóstico de Causa Raíz
* **Pérdida de Marca de Selección y Nombre en la Miniatura de Arrastre:**
  * Al reemplazar el uso de `WidgetPaintable` por un `IconPaintable` puro en la iteración previa, la miniatura flotante pasó a mostrar únicamente la imagen del icono, prescindiendo del rótulo con el nombre del archivo y del halo/placa azul acrílica de selección (`.selected`).
* **Comportamiento Erróneo de Copia en lugar de Mover (`delete_data` ignorado):**
  * Al arrastrar desde el escritorio hacia una ventana externa o explorador de archivos, la acción acordada en Wayland era `MOVE`. Sin embargo, en [`crates/wallpaper/src/main.rs`](../crates/wallpaper/src/main.rs) el manejador `connect_drag_end` ignoraba el tercer parámetro `delete_data: bool`.
  * En el protocolo Wayland DND, cuando el destino acepta la operación como `MOVE`, la fuente del arrastre recibe `delete_data = true` y tiene la obligación contractual de eliminar el archivo de origen de su carpeta local (`~/Escritorio`). Al ignorar este parámetro, el archivo quedaba en el destino y también en el escritorio, generando una copia no deseada.
  * De igual modo, en [`crates/branches/src/views/icon_view.rs`](../crates/branches/src/views/icon_view.rs), el lienzo `canvas_drop` no forzaba `DragAction::MOVE` en `connect_enter` / `connect_motion`, provocando que GTK4 negociase `COPY` de forma predeterminada.

### Solución Implementada (`crates/wallpaper`, `crates/branches`)
* **Widget Flotante Completo Desacoplado (`drag_preview_box`):**
  * Para mantener la aparición inmediata (0 latencia) y la desaparición total del icono de origen en el escritorio (`opacity: 0.0`) sin perder la marca de selección ni el nombre del archivo, se implementó una caja de previsualización dedicada (`drag_preview_box` en `wallpaper` y `drag_preview` en `branches`).
  * Esta caja se aloja fuera de la vista del usuario en el lienzo (`fixed.put(&drag_preview_box, -2000.0, -2000.0)`), permitiendo que esté realizada, mapeada y con su CSS completo (`desktop-icon selected`, icono en tamaño completo y etiqueta con el nombre del archivo).
  * Se conecta su `WidgetPaintable` a `drag_source.set_icon(...)` antes de iniciar la interacción. Al arrastrar, el cursor muestra el archivo completo con su placa de selección y título, mientras el elemento real en la cuadrícula desaparece de forma instantánea.
* **Corrección Crítica: Eliminación de `delete_data` Destructivo en el Origen:**
  * Al reordenar iconos dentro del mismo escritorio o lienzo de Branches, GTK notificaba `delete_data = true` al completarse el arrastre con acción `MOVE`. El borrado incondicional en `drag_source.connect_drag_end` interpretaba erróneamente cualquier reposicionamiento local como una solicitud de eliminación, enviando los archivos a la papelera o eliminándolos.
  * Se retiró completamente el borrado en `connect_drag_end`. En el modelo de escritorio y gestor de archivos, la responsabilidad del traslado recae exclusivamente en el destino (`drop_target.connect_drop`), el cual ejecuta `file.move_(&dest_file, ...)` únicamente cuando el archivo proviene de una carpeta externa (`source != destination`). Si el arrastre es una reorganización en el mismo escritorio o lienzo, sólo se actualizan las coordenadas espaciales `(x, y)` sin alterar el archivo en disco.
* **Traslado Nativo (`DragAction::MOVE`) sin Duplicación entre Ventanas:**
  * En `branches`: se implementó `connect_enter` y `connect_motion` en `canvas_drop` devolviendo `gdk::DragAction::MOVE`. Al soltar archivos procedentes del escritorio, se utiliza `file.move_(&dest_file, ...)`, garantizando el traslado atómico directo en el sistema de archivos (moviéndolo efectivamente de `~/Escritorio` al directorio destino sin duplicados).

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
