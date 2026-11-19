# ADR 0006: Pendiente — Funcionalidades restantes para completar Branches

## Estado
Pendiente de implementación (`2026-11-18`). Las fases 1–4 base de [`crates/branches`](../../crates/branches/Cargo.toml) están operativas (4 vistas, Split View dual pane, pestañas, portapapeles de archivos con atajos `Command/Ctrl + C/X/V`, opacidad `0.5` para ocultos/cortados, canvas libre SQLite y marcadores), y los puntos siguientes definen lo que falta para completar la paridad con macOS Finder y la Fase 5 definida en [`PRD.md`](../../PRD.md).

---

## 1. Operaciones de Archivo Asíncronas, Progreso, Conflictos y `Deshacer`
- **Dónde ocurre:** [`crates/branches/src/operations.rs`](../../crates/branches/src/operations.rs) (`copy_into`, `move_into`, `duplicate_item`, `trash_item`, `move_paths_into_folder`, `paste_payload_into`).
- **Problema:** Las copias, movimientos, duplicados y envíos a papelera se ejecutan de forma síncrona sobre el hilo de UI; en archivos grandes o múltiples elementos pueden bloquear la interfaz. Además, no existe cola visible de tareas ni historial para revertir acciones.
- **Acciones sugeridas:**
  - Ejecutar copias/movimientos en un trabajador en segundo plano con canal hacia GTK4 e indicador de progreso en la barra de herramientas o popover de transferencias (con cancelación).
  - Añadir diálogo de resolución de conflictos (*Reemplazar / Saltar / Mantener ambos / Aplicar a todo*) en lugar de renombrar automáticamente con `" copia"`.
  - Implementar pila `Undo` (`Cmd+Z` / `Ctrl+Z`) para renombrados, movimientos, duplicados y envíos a papelera.

## 2. Gestión Completa de la Papelera
- **Dónde ocurre:** [`crates/branches/src/views/mod.rs`](../../crates/branches/src/views/mod.rs) (`show_item_context_menu`), [`crates/branches/src/folder_pane.rs`](../../crates/branches/src/folder_pane.rs) (`trash_selected`) y [`crates/branches/src/sidebar.rs`](../../crates/branches/src/sidebar.rs).
- **Problema:** Hoy es posible enviar elementos a `~/.local/share/Trash/files`, pero no hay acciones en UI para **Restaurar**, **Vaciar Papelera** ni **Eliminar permanentemente** (`Shift+Delete` / `Cmd+Option+Backspace`). Además, `trash_selected()` no maneja de forma diferenciada elementos que ya están dentro de la Papelera.
- **Acciones sugeridas:**
  - Leer metadatos `.trashinfo` (`Trash/info`) para implementar **Restaurar a ubicación original**.
  - Añadir barra superior contextual o botón **«Vaciar Papelera»** cuando la ruta activa sea `Trash/files`, y acción contextual **«Vaciar Papelera»** en la barra lateral.
  - Exponer **«Eliminar inmediatamente»** con diálogo de confirmación ([`malus-de-dialogs`](../../crates/dialogs/Cargo.toml)) tanto en menús contextuales como dentro de la Papelera.

## 3. Selección Múltiple Homogénea en Todas las Vistas
- **Dónde ocurre:** [`crates/branches/src/folder_pane.rs`](../../crates/branches/src/folder_pane.rs) (`get_selected_items`, `select_all`), [`crates/branches/src/views/list_view.rs`](../../crates/branches/src/views/list_view.rs), [`crates/branches/src/views/column_view.rs`](../../crates/branches/src/views/column_view.rs) y [`crates/branches/src/views/gallery_view.rs`](../../crates/branches/src/views/gallery_view.rs).
- **Problema:** `get_selected_items()` y `select_all()` (`Cmd+A` / `Ctrl+A`) solo devuelven o seleccionan múltiples elementos en `IconView`; en Lista, Columnas y Galería devuelven como máximo un ítem.
- **Acciones sugeridas:** Unificar selección múltiple (`MultiSelection` / selección por rango `Shift+Clic` y `Cmd/Ctrl+Clic`) en las 4 vistas para que copiar, cortar, mover, duplicar, etiquetar y borrar en lote funcionen igual en cualquier modo.

## 4. Compresión y Extracción de Archivos
- **Dónde ocurre:** [`crates/branches/src/operations.rs`](../../crates/branches/src/operations.rs), [`crates/branches/src/app_window.rs`](../../crates/branches/src/app_window.rs) y [`crates/branches/src/views/mod.rs`](../../crates/branches/src/views/mod.rs).
- **Problema:** Falta soporte para crear archivos comprimidos (`.zip`, `.tar.gz`) desde selección simple o múltiple y extraer paquetes existentes en la carpeta actual.
- **Acciones sugeridas:** Añadir **«Comprimir…»** y **«Extraer aquí»** en los menús contextuales y en el menú de acciones (`...`), ejecutándolos de forma asíncrona con feedback de progreso.

## 5. Ventana Separada «Obtener Información» (`Cmd+I`) y Ordenación/Agrupación en Toolbar
- **Dónde ocurre:** [`crates/branches/src/app_window.rs`](../../crates/branches/src/app_window.rs) (`info_btn`, barra de herramientas) y [`crates/branches/src/state.rs`](../../crates/branches/src/state.rs) (`SortBy`, `SortOrder`).
- **Problema:**
  - El botón `"i"` (`info_btn`) actualmente dispara `toggle_quick_look()` en lugar de un inspector detallado de propiedades.
  - `SortBy` (`Name`, `DateModified`, `Size`, `Kind`) y `SortOrder` existen en el estado, pero no tienen menú desplegable en la barra de herramientas ni agrupación visual por secciones.
  - Los botones `<` / `>` aún no muestran menú desplegable de historial al mantener clic, y [`path_bar.rs`](../../crates/branches/src/path_bar.rs) no actúa como destino de *drag & drop* hacia carpetas padre.
- **Acciones sugeridas:**
  - Crear diálogo/inspector **Obtener Información** (`Cmd+I` / `Ctrl+I`) con cálculo asíncrono de tamaño en carpetas, permisos POSIX editables, selector **«Abrir con…»** / aplicación predeterminada y notas/etiquetas.
  - Agregar cápsula/menú de **Ordenar / Agrupar por** en la barra superior de `app_window.rs`, menú contextual en `<` / `>` y `DropTarget` en cada botón de `PathBar`.

## 6. Sistema Completo de Etiquetas (*Tags*)
- **Dónde ocurre:** [`crates/branches/src/app_window.rs`](../../crates/branches/src/app_window.rs) (`tag_red`, `tag_green`, `tag_blue`), [`crates/branches/src/sidebar.rs`](../../crates/branches/src/sidebar.rs) (`tags`), [`crates/branches/src/db.rs`](../../crates/branches/src/db.rs) y [`crates/branches/src/views/`](../../crates/branches/src/views/mod.rs).
- **Problema:** La barra lateral lista los 7 colores (`Rojo/Red`, `Naranja/Orange`, `Amarillo/Yellow`, `Verde/Green`, `Azul/Blue`, `Púrpura/Purple`, `Gris/Gray`), mientras `app_window.rs` solo expone 3 acciones fijas en español (`"Rojo"`, `"Verde"`, `"Azul"`), sin acción para quitar etiquetas, sin menú en clic derecho, sin indicador visual de color sobre cada archivo/fila y sin persistencia en `xattr` (`user.malus.tags`).
- **Acciones sugeridas:** Unificar los 7 identificadores de etiqueta independientemente del idioma de la UI, permitir asignar/quitar múltiples etiquetas desde menú contextual y barra superior, renderizar los puntos de color en los ítems y sincronizar `user.malus.tags` junto con SQLite.

## 7. Quick Look Enriquecido, Búsqueda Recursiva y Compartir con Malus Link
- **Dónde ocurre:** [`crates/branches/src/quick_look.rs`](../../crates/branches/src/quick_look.rs) (`create_preview_content`) y [`crates/branches/src/app_window.rs`](../../crates/branches/src/app_window.rs) (`search_entry`, `share_model`).
- **Problema:**
  - `QuickLook` solo previsualiza imágenes y texto plano (sin resaltado sintáctico); PDF, audio y vídeo muestran únicamente metadatos genéricos, y no permite navegar con flechas entre ítems mientras está abierto.
  - `search_entry` únicamente filtra por nombre en el directorio actual, sin búsqueda recursiva ni consulta a [`malus-de-spotlight`](../../crates/spotlight/Cargo.toml).
  - El menú Compartir solo conecta `copy_path`, `kdeconnect-handler` y `mailto:`, sin envío directo mediante [`malus-de-link`](../../crates/link/Cargo.toml).
- **Acciones sugeridas:** Incorporar vista previa de audio/vídeo/PDF y resaltado de código en `quick_look.rs` (más navegación `Izquierda`/`Derecha`), soporte de búsqueda recursiva / Spotlight en `FolderPane`, e integración D-Bus con `malus-de-link`.

## 8. Red (`SMB`/`SFTP`/`NFS`) e Integración como Explorador Predeterminado (Fase 5)
- **Dónde ocurre:** [`crates/branches/src/sidebar.rs`](../../crates/branches/src/sidebar.rs) (entrada `Red` apuntando a `/mnt`) y [`scripts/install-session.sh`](../../scripts/install-session.sh).
- **Problema:**
  - La entrada «Red» en la barra lateral solo abre `/mnt` local; falta descubrimiento de equipos/recursos compartidos y diálogo **«Conectar al servidor…»** (`Cmd+K`).
  - Aunque se instala el binario `malus-de-branches`, falta instalar su `.desktop` y registrarlo como manejador por defecto para carpetas (`inode/directory`).
- **Acciones sugeridas:**
  - Añadir soporte de montajes de red (`gio`/`gvfs` o `cifs`/`sshfs` bajo demanda) y diálogo *Conectar al servidor* (`Cmd+K`).
  - Crear archivo `malus-de-branches.desktop` con `MimeType=inode/directory;` y ejecutar `xdg-mime default malus-de-branches.desktop inode/directory` dentro del flujo de instalación de sesión.

