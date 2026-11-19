# ADR 0007: Pendiente — Planificación de la Aplicación de Vista Previa / Quick Look (`malus-de-preview`)

## Estado
Planificado y pendiente de implementación (`2026-11-18`). Diseñado para implementarse por fases a partir de la base actual de [`crates/branches/src/quick_look.rs`](../../crates/branches/src/quick_look.rs).

---

## 1. Objetivo y Contexto Actual
Crear la aplicación nativa de vista previa (**`crates/preview`**, binario **`malus-de-preview`**, App ID `dev.malus.Preview`) inspirada en **Quick Look + Vista Previa (Preview.app)** de macOS, que permita inspeccionar al instante imágenes, vídeos, audio, PDFs, documentos de oficina, texto/código, archivos comprimidos y carpetas al pulsar la **barra espaciadora** (o «Vista Rápida») tanto en **Branches** como en el **Escritorio**.

### Situación actual verificada en el workspace
- Existe una base mínima embebida en [`crates/branches/src/quick_look.rs`](../../crates/branches/src/quick_look.rs) (`--quick-look <ARCHIVO>` en [`crates/branches/src/main.rs`](../../crates/branches/src/main.rs)) que únicamente previsualiza imágenes estáticas (`gdk_pixbuf`) y texto plano (`TextView`), cayendo en metadatos genéricos para el resto de formatos.
- [`crates/wallpaper/src/main.rs`](../../crates/wallpaper/src/main.rs) ya captura `Key::space` y el menú contextual `"Vista Rápida"`, delegando por CLI a `malus-de-branches --quick-look`.
- **Dependencias del sistema ya disponibles en Fedora 45:**
  - `gtk4` (`4.24.1`), `libadwaita`, `glycin-gtk4-libs` (`glycin-2` para formatos modernos de imagen como HEIC/AVIF).
  - `poppler-glib` y `poppler-utils` (`pdftoppm`, `pdfinfo`, `pdftotext`).
  - `gstreamer1` + `gstreamer1-plugin-gtk4` (`libgstgtk4.so` / `gtk4paintablesink`), `gstreamer1-plugins-{base,good,bad-free,ugly-free}`, `libav`, `openh264`, `dav1d`, `ffmpeg` y `ffprobe`.
  - **Nota técnica clave:** en este sistema **no** está instalado el módulo `gtk4-media-gstreamer` en `/usr/lib64/gtk-4.0/4.0.0/media/`, por lo que `gtk4::Video` / `MediaFile` por sí solo no reproduciría vídeo sin usar `gtk4paintablesink` (`gstreamer1-plugin-gtk4`) directamente.
  - `libreoffice-core` (`soffice --headless --convert-to pdf`) disponible para convertir documentos `.docx/.xlsx/.pptx/.odt/.ods/.odp` a PDF en caché temporal.

---

## 2. Arquitectura Propuesta

```text
  Branches (Espacio / botón "Vista Rápida") ──┐
                                              ├──▶ IPC / D-Bus (dev.malus.Preview)
  Escritorio / Wallpaper (Espacio / menú)  ───┘         │
                                                        ▼
                                            malus-de-preview (crates/preview)
                                            ├── QuickLookController (instancia única, toggle y navegación ←/→)
                                            ├── PreviewRouter (detección por MIME + extensión)
                                            ├── Visores especializados:
                                            │   ├── ImageViewer (Pixbuf/Glycin, GIF/WebP animado, zoom/rotación)
                                            │   ├── MediaViewer (GStreamer gtk4paintablesink + ffprobe)
                                            │   ├── PdfViewer (Poppler / pdftoppm + caché LRU por página)
                                            │   ├── OfficeViewer (LibreOffice headless -> PdfViewer cacheado)
                                            │   ├── CodeTextViewer (carga acotada, líneas, sintaxis/Markdown)
                                            │   ├── ArchiveViewer (listado ZIP/TAR/7Z sin extraer)
                                            │   └── Folder/FallbackViewer (resumen asíncrono + metadatos)
                                            └── UI GTK4 (ventana squircle adaptativa + barra compacta)
```

### Principios de diseño
1. **Crate desacoplado (`crates/preview`):** Evita cargar lógica/dependencias multimedia y de documentos dentro de `malus-de-branches` y `malus-de-wallpaper`, manteniendo rápidos ambos binarios y permitiendo abrir archivos directamente con `malus-de-preview`.
2. **Instancia única e IPC ágil:**
   - Al lanzar `malus-de-preview --quick-look <archivo> [--list <archivo_lista>]` por primera vez, responde por instancia única / socket propio (`$XDG_RUNTIME_DIR/malus-de-preview.sock`), permitiendo:
     - **Toggle inmediato con `Espacio`:** si ya está abierto para la misma ruta, cierra; si cambió la selección en Branches/Escritorio, actualiza el contenido en la misma ventana sin recrearla ni parpadear.
     - **Navegación interna (`←` / `→`):** recorrer todos los archivos visibles de la carpeta sin volver a abrir la app.
3. **Renderizado 100% asíncrono:** Ninguna decodificación de imagen, extracción de páginas PDF, conversión LibreOffice ni cálculo de tamaño de carpetas puede bloquear el hilo principal de GTK4; siempre se muestra primero la cabecera y un estado de carga fluido (`Spinner`) si tarda `> 60 ms`.
4. **Sin emojis en UI:** Toda la barra de herramientas, controles de reproducción, botones de zoom y fichas de respaldo usarán **glifos `-symbolic` del sistema**.

---

## 3. Plan de Visores por Tipo de Archivo

| Categoría | Extensiones / MIME | Estrategia Técnica | Controles y Capacidades |
|---|---|---|---|
| **Imágenes** | `png`, `jpg`, `jpeg`, `webp`, `gif`, `svg`, `bmp`, `ico`, `avif`, `heic`, `tiff` | Decodificación asíncrona en hilo worker (`gdk_pixbuf::Pixbuf` / `glycin` para HEIC/AVIF y `PixbufAnimation` para GIF/WebP animados). | Ajuste automático a ventana, zoom (`Cmd/Ctrl + +/-/0`, scroll), paneo arrastrando y rotación `90°`. |
| **Vídeo** | `mp4`, `mkv`, `webm`, `mov`, `avi`, `m4v` | Pipeline GStreamer (`playbin3` + `gtk4paintablesink` de `libgstgtk4.so`) + lectura inicial de códec/resolución/duración con `ffprobe` (o fallback de miniatura `ffmpeg` si falla un códec). | Barra flotante auto-ocultable (*Play/Pause* con clic, *seek* por línea de tiempo, volumen/mute, duración). |
| **Audio** | `mp3`, `flac`, `ogg`, `opus`, `wav`, `m4a`, `aac` | Mismo motor GStreamer sin sink de vídeo + extracción de carátula embebida y tags (`ffprobe`). | Tarjeta compacta con carátula *squircle*, título/artista/álbum, barra de progreso y controles de audio. |
| **PDF** | `pdf` (`application/pdf`) | `pdftoppm` / `poppler-glib` en hilo worker a escala HiDPI del monitor activo + `pdfinfo` para conteo y tamaño de páginas; caché LRU en memoria (±3 páginas). | Scroll fluido o navegación por página (`↑`/`↓`, `RePág`/`AvPág`), indicador `Página X / N`, zoom y barra lateral opcional de miniaturas. |
| **Oficina** | `docx`, `doc`, `odt`, `xlsx`, `xls`, `ods`, `pptx`, `ppt`, `odp`, `rtf` | Conversión bajo demanda en `/tmp/malus-preview-cache/<hash>.pdf` con `libreoffice --headless --convert-to pdf` (reutilizando caché si `mtime` y tamaño no cambian) + visor PDF. | Reutiliza las mismas herramientas de paginación y zoom que PDF. |
| **Código / Texto / Markdown** | `rs`, `ts`, `js`, `py`, `c`, `cpp`, `sh`, `json`, `toml`, `yaml`, `md`, `txt`, `csv`, `log`, `xml`, `html`, `css` | Lectura segura en streaming acotado inicialmente (`512 KiB` con opción «Cargar archivo completo»); vista monoespaciada con numeración de líneas y resaltado ligero (o vista enriquecida básica para Markdown/CSV). | Búsqueda interna (`Cmd/Ctrl+F`), ajuste de línea (*word wrap*), copia rápida. |
| **Comprimidos** | `zip`, `tar`, `tar.gz`, `tgz`, `tar.xz`, `tar.zst`, `7z`, `rar` | Inspección de índice sin extraer (`unzip -Z`, `tar -tvf`, `7z l`) en hilo secundario. | Tabla ordenada de archivos internos, tamaños comprimido/original y acceso a extracción en carpeta padre. |
| **Carpetas y Otros** | Directorios / binarios desconocidos | Conteo asíncrono de elementos y tamaño total + resolución de icono MIME del sistema (`gio::File::query_info`). | Ficha limpia estilo macOS con tipo, tamaño, ubicación, permisos, fechas y acciones rápidas (**Abrir**, **Mostrar en Branches**, **Compartir**). |

---

## 4. Integración con Malus DE (Compositor, Branches y Escritorio)
1. **Compositor ([`crates/compositor`](../../crates/compositor/Cargo.toml)):**
   - Registrar `dev.malus.Preview` en reglas de centrado inicial (`state.rs`) sin forzar restauraciones de tamaño fijas de sesiones previas (para que cada medio ajuste la ventana a su relación de aspecto).
   - Aplicar recorte *squircle* y foco inmediato al invocarse, devolviendo el foco a la ventana anterior (`Branches` o escritorio) al cerrarse con `Espacio`/`Esc`.
2. **Branches ([`crates/branches`](../../crates/branches/Cargo.toml)):**
   - Sustituir la lógica de previsualización duplicada en [`quick_look.rs`](../../crates/branches/src/quick_look.rs) por un cliente ligero que invoque a `malus-de-preview` pasando el ítem seleccionado y la lista de rutas del panel activo, sincronizando `update_if_open` al cambiar de selección.
3. **Escritorio ([`crates/wallpaper`](../../crates/wallpaper/Cargo.toml)):**
   - Actualizar `launch_quick_look` en [`crates/wallpaper/src/main.rs`](../../crates/wallpaper/src/main.rs) para llamar a `malus-de-preview --quick-look` con la lista de elementos del escritorio.
4. **Instalación ([`scripts/install-session.sh`](../../scripts/install-session.sh)):**
   - Añadir `"malus-de-preview"` a la compilación/instalación de binarios y crear `malus-de-preview.desktop`.

---

## 5. UX, Geometría y Atajos
- **Dimensionamiento inteligente:** Calcular tamaño inicial de la ventana según el medio (imágenes/vídeos respetando proporción dentro de un máximo de `78% × 78%` del área útil del monitor activo; PDFs/documentos verticales a ~`680 × 860`; audio/fichas a ~`520 × 320`).
- **Cabecera limpia estilo macOS:**
  - Botones de navegación `←` / `→` (cuando hay lista de carpeta), título + subtítulo compacto (`Nombre`, `Dimensiones / Páginas / Duración • Tamaño`), y botones `-symbolic` a la derecha: **Zoom** (si aplica), **Compartir** ([`malus-de-link`](../../crates/link/Cargo.toml) / copiar ruta), **Abrir con…** y **Pantalla completa**.
- **Atajos de teclado:**
  - `Espacio` / `Esc`: cerrar Quick Look (decidir antes de la Fase 3 si en vídeo/audio `Espacio` cierra mientras clic/`K` pausa, o si `Espacio` pausa y `Esc` cierra).
  - `←` / `→`: archivo anterior / siguiente.
  - `↑` / `↓` / `PageUp` / `PageDown`: página anterior / siguiente en PDF/oficina.
  - `Cmd/Ctrl + +`, `-`, `0`: acercar, alejar, tamaño real / ajustar.
  - `Enter`: abrir con la aplicación predeterminada y cerrar Quick Look.

---

## 6. Fases de Implementación Recomendadas
1. **Fase 1 — Crate `crates/preview`, IPC, Geometría y Visor de Imágenes + Texto:**
   - Crear `crates/preview` en [`Cargo.toml`](../../Cargo.toml), instancia única IPC, CLI `--quick-look`, ventanas adaptativas, visor de imágenes (incluyendo SVG/GIF animado) y texto/código acotado; conectar `branches`, `wallpaper` y `compositor`.
2. **Fase 2 — Visor de PDF y Documentos de Oficina:**
   - Implementar renderizado por páginas con `pdftoppm`/`pdfinfo` + caché LRU, paginación fluida, zoom y conversión cacheada vía `libreoffice --headless`.
3. **Fase 3 — Reproductor Multimedia (Vídeo y Audio):**
   - Integrar GStreamer (`libgstgtk4.so` / `gtk4paintablesink` + evaluación de crates necesarios) y `ffprobe`, barra flotante de reproducción y vista de carátula para audio.
4. **Fase 4 — Comprimidos, Carpetas, Markdown/Código y Pulido Final:**
   - Visor de archivos comprimidos (`.zip`/`.tar.*`/`.7z`), cálculo asíncrono de carpetas, resaltado ligero de código/Markdown, integración con `malus-de-link`, `.desktop` en `install-session.sh` y pruebas unitarias de detección MIME/caché/geometría sin tocar sockets de sesión viva.

