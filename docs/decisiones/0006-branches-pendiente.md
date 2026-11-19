# ADR 0006: Pendiente — Funcionalidades restantes para completar Branches

## Estado
**Completado en su núcleo funcional (`2026-11-19`)** mediante las Fases 1–6 (`7924b69`..`c79a462`) y las optimizaciones de Vista Rápida / Quick Look (`c65e758`..`c4e9dff`). Las capacidades troncales de paridad con macOS Finder en [`crates/branches`](../../crates/branches/Cargo.toml) están operativas y verificadas (`53` pruebas unitarias en `malus-de-branches` y `0` *warnings* en el workspace); al final del documento se listan los detalles menores de seguimiento y tareas separadas por alcance.

---

## 1. Operaciones de Archivo Asíncronas, Progreso, Conflictos y `Deshacer` — ✅ Implementado (`b77a18c`)
- **Dónde ocurre:** [`crates/branches/src/jobs.rs`](../../crates/branches/src/jobs.rs), [`crates/branches/src/job_ui.rs`](../../crates/branches/src/job_ui.rs), [`crates/branches/src/operations.rs`](../../crates/branches/src/operations.rs).
- **Implementado:**
  - Copias, movimientos, duplicados, envíos a papelera y compresión/extracción ejecutados de forma asíncrona en segundo plano con cancelación cooperativa e indicador/popover de progreso en la barra de herramientas.
  - Diálogo interactivo de resolución de conflictos (*Reemplazar / Saltar / Mantener ambos / Aplicar a todo*).
  - Pila `UndoEntry` (`Ctrl+Z` / `Cmd+Z`) para revertir renombrados, movimientos, duplicados y envíos a papelera.

## 2. Gestión Completa de la Papelera — ✅ Implementado (`b49a5ee`)
- **Dónde ocurre:** [`crates/branches/src/operations.rs`](../../crates/branches/src/operations.rs), [`crates/branches/src/views/mod.rs`](../../crates/branches/src/views/mod.rs), [`crates/branches/src/folder_pane.rs`](../../crates/branches/src/folder_pane.rs), [`crates/branches/src/sidebar.rs`](../../crates/branches/src/sidebar.rs) y [`crates/branches/src/app_window.rs`](../../crates/branches/src/app_window.rs).
- **Implementado:**
  - Lectura de metadatos `.trashinfo` y acción **Restaurar a ubicación original** (`restore_from_trash`) con manejo de conflictos.
  - Banner contextual **«Vaciar Papelera»** al abrir `~/.local/share/Trash/files` y menú secundario homónimo sobre «Papelera» en la barra lateral.
  - Acción **«Eliminar inmediatamente»** (`Shift+Delete` / `Cmd+Alt+Backspace`) con diálogo de confirmación y manejo diferenciado dentro de la Papelera.

## 3. Selección Múltiple Homogénea en Todas las Vistas — ✅ Implementado (`7924b69`)
- **Dónde ocurre:** [`crates/branches/src/folder_pane.rs`](../../crates/branches/src/folder_pane.rs), [`crates/branches/src/views/list_view.rs`](../../crates/branches/src/views/list_view.rs), [`crates/branches/src/views/column_view.rs`](../../crates/branches/src/views/column_view.rs), [`crates/branches/src/views/gallery_view.rs`](../../crates/branches/src/views/gallery_view.rs) y [`crates/branches/src/status_bar.rs`](../../crates/branches/src/status_bar.rs).
- **Implementado:**
  - `get_selected_items()`, `select_all()` (`Ctrl/Cmd+A`), selección por rango (`Shift+Clic`) y conmutación (`Ctrl/Cmd+Clic`) unificados en las 4 vistas (`IconView`, `ListView`, `ColumnView`, `GalleryView`).
  - Arrastre multi-archivo coherente y conteo exacto de selección en la barra de estado.

## 4. Compresión y Extracción de Archivos — ✅ Implementado (`b77a18c`)
- **Dónde ocurre:** [`crates/branches/src/jobs.rs`](../../crates/branches/src/jobs.rs), [`crates/branches/src/app_window.rs`](../../crates/branches/src/app_window.rs) y [`crates/branches/src/views/mod.rs`](../../crates/branches/src/views/mod.rs).
- **Implementado:**
  - Acciones **«Comprimir (.zip)»**, **«Comprimir (.tar.gz)»** y **«Extraer aquí»** (`.zip`, `.tar.*`, `.7z`) desde menú contextual y menú de acciones (`...`), ejecutadas en segundo plano con barra de progreso.

## 5. Ventana Separada «Obtener Información» (`Cmd+I`) y Ordenación en Toolbar — ✅ Implementado (`aa06006`) *(salvo agrupación visual)*
- **Dónde ocurre:** [`crates/branches/src/info_window.rs`](../../crates/branches/src/info_window.rs), [`crates/branches/src/app_window.rs`](../../crates/branches/src/app_window.rs), [`crates/branches/src/state.rs`](../../crates/branches/src/state.rs) y [`crates/branches/src/path_bar.rs`](../../crates/branches/src/path_bar.rs).
- **Implementado:**
  - Inspector independiente **Obtener Información** (`Ctrl/Cmd+I` y botón `"i"`) con cálculo recursivo cancelable de tamaño, permisos POSIX editables (`chmod`), selector **«Abrir con…»** / app predeterminada (`gio::AppInfo`) y editor de etiquetas.
  - Cápsula/menú desplegable **Ordenar por** (`Nombre`, `Fecha`, `Tamaño`, `Clase`, `Ascendente / Descendente`) conectado a `win.sort_by` y `win.sort_order`.
  - Popovers de historial al mantener clic o usar clic derecho en `<` / `>` (`go_back_steps`, `go_forward_steps`) y `DropTarget` en segmentos de [`path_bar.rs`](../../crates/branches/src/path_bar.rs).
- **Faltante menor:** agrupación visual por secciones/cabeceras (`GroupBy`) dentro de `IconView` / `ListView` (actualmente solo ordena).

## 6. Sistema Completo de Etiquetas (*Tags*) — ✅ Implementado (`b49a5ee`)
- **Dónde ocurre:** [`crates/branches/src/db.rs`](../../crates/branches/src/db.rs), [`crates/branches/src/operations.rs`](../../crates/branches/src/operations.rs), [`crates/branches/src/sidebar.rs`](../../crates/branches/src/sidebar.rs), [`crates/branches/src/app_window.rs`](../../crates/branches/src/app_window.rs) y [`crates/branches/src/views/`](../../crates/branches/src/views/mod.rs).
- **Implementado:**
  - Catálogo unificado de los 7 colores (`TAG_CATALOG`) con migración SQLite v2 desde nombres previos en español.
  - Asignación/eliminación múltiple desde menú contextual (selector de puntos de color) y menú de barra superior (`win.toggle_tag`).
  - Renderizado de indicadores visuales en las 4 vistas y sincronización con el atributo extendido `user.malus.tags` vía `gio::File`.

## 7. Quick Look Enriquecido, Búsqueda Recursiva y Compartir con Malus Link — ✅ Implementado (`381f293`, `c65e758`..`c4e9dff`) *(con 2 mejoras opcionales)*
- **Dónde ocurre:** [`crates/branches/src/quick_look.rs`](../../crates/branches/src/quick_look.rs), [`crates/branches/src/styles/macos-branches.css`](../../crates/branches/src/styles/macos-branches.css), [`crates/branches/src/fs_backend.rs`](../../crates/branches/src/fs_backend.rs), [`crates/branches/src/folder_pane.rs`](../../crates/branches/src/folder_pane.rs) y [`crates/branches/src/app_window.rs`](../../crates/branches/src/app_window.rs).
- **Implementado:**
  - **Quick Look estilo macOS (`Espacio` abrir/cerrar):** HUD flotante integrado en `Overlay` con transición *zoom + fade* sin disparar *Black Hole* del compositor (más exención en [`crates/compositor/src/state.rs`](../../crates/compositor/src/state.rs) para `--quick-look` / `dev.malus.Preview`), caché LRU de 24 entradas (`PreviewCache`), decodificación RGBA fuera del hilo UI (`DecodedRgbaFrame`) y precarga predictiva al seleccionar o navegar con `←`/`→`.
  - Previsualización real de imágenes (`Picture` + Glycin/`Pixbuf`/`ffmpeg`), PDF multipágina (`pdfinfo` + `pdftoppm`), vídeo/audio inline (`ffmpeg` + `ffplay`), listado de comprimidos (`.zip`, `.tar.*`, `.7z`), texto plano/código y conteo en carpetas.
  - Puente D-Bus hacia `dev.malus.Preview` (para delegar cuando esté activa la app independiente de ADR 0007).
  - Búsqueda recursiva acotada y cancelable en segundo plano (`search_paths_recursive`) con conmutador *Esta carpeta / Subcarpetas*.
  - Integración D-Bus en menú Compartir hacia [`malus-de-link`](../../crates/link/Cargo.toml) (`org.malus_de.Link`: `GetConnectedDevices` / `ShareFile`).
- **Faltante menor:**
  - Resaltado sintáctico de código (actualmente `TextView` monoespaciado sin colores de sintaxis ni `gtksourceview5`).
  - Consulta al índice de [`malus-de-spotlight`](../../crates/spotlight/Cargo.toml) como fuente adicional/instantánea para búsqueda global.

## 8. Red (`SMB`/`SFTP`/`NFS`) e Integración como Explorador Predeterminado — ✅ Implementado (`c79a462`)
- **Dónde ocurre:** [`crates/branches/src/network.rs`](../../crates/branches/src/network.rs), [`crates/branches/src/sidebar.rs`](../../crates/branches/src/sidebar.rs), [`crates/branches/src/app_window.rs`](../../crates/branches/src/app_window.rs) y [`scripts/install-session.sh`](../../scripts/install-session.sh).
- **Implementado:**
  - Diálogo **Conectar al servidor…** (`Ctrl+K` / `Cmd+K`, acción `win.connect_server`) con validación `normalize_network_uri` (`smb://`, `sftp://`, `ssh://`, `nfs://`, `dav://`, `davs://`, `ftp://`, `ftps://`, `afp://`), historial persistente de servidores y montaje asíncrono vía `gio::File::mount_enclosing_volume` + `gtk4::MountOperation`.
  - Entrada **«Red»** integrada con montajes GVfs activos y visualización directa de recursos de red montados (con desmontaje/expulsión) en la barra lateral.
  - Refresco de base `.desktop` (`update-desktop-database`) y asociación MIME predeterminada (`xdg-mime default dev.malus.Branches.desktop` para `inode/directory` y `x-scheme-handler/trash`) añadidos a [`scripts/install-session.sh`](../../scripts/install-session.sh).

---

## Pendiente menor / Seguimiento posterior
1. **Agrupación visual por secciones (`GroupBy`)** en `IconView` y `ListView` (separadores de grupo por clase/fecha/tamaño/etiqueta complementando `sort_by`).
2. **Resaltado sintáctico en Quick Look** para código fuente y Markdown (`gtksourceview5` o tokenizador ligero).
3. **Búsqueda indexada vía `malus-de-spotlight`** junto al recorrido BFS en disco (`search_paths_recursive`).
4. **Aplicación dedicada `Preview` (`crates/preview`):** planificada por separado en [`0007-preview-app-pendiente.md`](0007-preview-app-pendiente.md) para servir `dev.malus.Preview` como app independiente.
5. **Despliegue en sesión viva e iconos de escritorio (`PRD §3.7 Fase 5`):** ejecutar [`scripts/install-session.sh`](../../scripts/install-session.sh) (y reiniciar el compositor para activar el filtro *Black Hole* en `--quick-look`) y mantener/extender el soporte de iconos en escritorio desde [`crates/wallpaper`](../../crates/wallpaper/Cargo.toml).
