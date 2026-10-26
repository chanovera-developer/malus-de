# Malus DE

**Malus DE** es una reimplementación libre del entorno de escritorio de macOS para Linux, construida sobre Wayland. Busca trasladar su estética y ergonomía a un sistema abierto y modular, con un compositor escrito en Rust y componentes de shell independientes.

> **Estado:** en desarrollo activo. El PRD marca como implementados el compositor base, el dock, la barra superior y el centro de control, entre otros hitos. La integración completa de AirDrop/AWDL sigue en progreso; consulta [PRD.md](./PRD.md) para ver el estado detallado.

## Características

- Compositor Wayland basado en [Smithay](https://github.com/Smithay/smithay), con backend anidado Winit y backend DRM/KMS.
- Shell modular con barra superior, menú global, centro de control, dock flotante, Spotlight (incluye acciones para cerrar sesión, reiniciar y apagar), notificaciones y fondo de pantalla.
- Escritorio interactivo con iconos movibles, intercambio de archivos por arrastrar y soltar con el gestor de archivos y miniaturas de imágenes.
- Selector visual de ventanas Alt-Tab/Super-Tab funcional para recorrer y cambiar entre ventanas abiertas; su presentación y comportamiento siguen en desarrollo.
- Integración con servicios de Linux para audio, red, Bluetooth y controles multimedia.
- Captura real de pantalla completa solicitada al compositor mediante IPC, guardada en la carpeta de Imágenes y copiada como PNG al portapapeles.
- Historial local persistente de hasta 50 elementos de texto, imágenes, documentos y URI de archivos, accesible con `Super + V`. Admite búsqueda, fijar elementos, volver a copiarlos y borrarlos; ocupa como máximo 50 MiB (10 MiB por elemento). La sincronización con la selección primaria de Wayland es opcional y está desactivada por defecto. El filtro automático de credenciales está activado por defecto y se puede configurar desde la interfaz; es heurístico y no detecta todos los secretos. Los datos se guardan localmente con permisos privados, se conservan entre sesiones y se eliminan al borrar el historial. Requiere `wl-clipboard`.
- Efectos gráficos con shaders, esquinas squircle y animaciones de ventanas.
- Detección de pantallas y modos compatibles por DRM/KMS, con resolución, disposición y escala fraccionaria configurables en caliente.
- Experimentos de integración con dispositivos Apple mediante AirDrop; el backend AWDL/BLE está en desarrollo.

## Arquitectura

El proyecto es un workspace de Rust. Sus componentes principales viven en `crates/`:

| Componente | Paquete |
| --- | --- |
| Compositor Wayland | `malus-de-compositor` |
| Barra superior y centro de control | `malus-de-top-bar` |
| Dock | `malus-de-dock` |
| Lanzador Spotlight | `malus-de-spotlight` |
| Notificaciones | `malus-de-notifications` |
| Fondo de pantalla | `malus-de-wallpaper` |
| Ajustes | `malus-de-settings` |
| Servicios comunes | `malus-de-common` |
| Bloqueo de sesión, captura, portapapeles, enlace y CLI | `malus-de-session-lock`, `malus-de-screenshot`, `malus-de-clipboard`, `malus-de-link`, `malus-de-ctl` |

## Requisitos

- Linux con soporte para Wayland.
- Rust y Cargo (edición 2021).
- Bibliotecas de desarrollo de GTK4, libadwaita, gtk4-layer-shell, Mesa/GBM, libinput, libseat, systemd y D-Bus.
- Para las funciones de escritorio, los servicios del sistema correspondientes, como PipeWire/WirePlumber, NetworkManager, BlueZ y Polkit.

La lista de paquetes por distribución está en el [manifiesto de dependencias](./docs/PACKAGE_MANIFEST.md). Los componentes y requisitos de producto se describen en el [PRD](./PRD.md).

## Compilar

En Fedora, instala las herramientas y bibliotecas de desarrollo necesarias:

```bash
sudo dnf install -y \
    rust \
    rustfmt \
    cargo \
    gcc \
    clang \
    pkgconf-pkg-config \
    gtk4-devel \
    libadwaita-devel \
    gtk4-layer-shell-devel \
    mesa-libgbm-devel \
    libinput-devel \
    libseat-devel \
    systemd-devel \
    dbus-devel \
    libxkbcommon-devel
```

`systemd-devel` proporciona los archivos de desarrollo de `libudev`; `libxkbcommon-devel` es necesario para enlazar el compositor. Asegúrate también de tener Rust y Cargo instalados (edición 2021).

Clona el repositorio y compila desde su raíz:

```bash
git clone https://github.com/chanovera-dev/malus-de.git
cd malus-de
cargo build --workspace
```

Para otras distribuciones, consulta los paquetes equivalentes en el [manifiesto de dependencias](./docs/PACKAGE_MANIFEST.md).

Para compilar en modo optimizado:

```bash
cargo build --workspace --release
```

## Capturas de pantalla

La captura completa solicita el fotograma al compositor de Malus mediante IPC, sin depender del portal de GNOME Shell. Al completarse, muestra un breve destello en las pantallas, reproduce el sonido de obturador y envía una notificación; el PNG se guarda en la carpeta de Imágenes configurada por XDG (por ejemplo, `~/Imágenes/Capturas de pantalla` en español), se copia automáticamente al portapapeles como imagen PNG y después muestra una miniatura. El modo de área muestra una capa de selección en la primera pantalla, permite arrastrar para delimitar la región y cancelar con `Esc`, y guarda solo esa selección.
Pulsa `Impr Pant` para iniciar una captura completa. Después de actualizar el compositor, recompila Malus y reinicia la sesión para que estén disponibles el atajo y el servicio IPC de captura.

```bash
cargo run -p malus-de-screenshot -- --mode full
cargo run -p malus-de-screenshot -- --mode full --clipboard-only
cargo run -p malus-de-screenshot -- --mode area
```

La copia al portapapeles requiere `wl-copy` (`wl-clipboard`). También puedes usar `--clipboard-only` para copiar sin guardar el archivo ni abrir la miniatura. La captura de ventana, grabación y OCR todavía no están implementados.

El gestor de portapapeles conserva hasta 50 elementos (texto, enlaces, URI de archivos, imágenes comunes y formatos de documento como PDF y RTF), con un límite de 10 MiB por elemento y 50 MiB en total. Pulsa `Super + V` para buscar, fijar, volver a copiar o borrar elementos. El historial se guarda localmente entre sesiones con permisos privados y requiere `wl-clipboard`; borrar el historial elimina también su copia persistente. La sincronización con la selección primaria de Wayland es opcional y está desactivada por defecto; se configura desde la interfaz del historial. El filtro sensible, activado por defecto, omite las selecciones marcadas como contraseñas por la aplicación de origen y detecta algunas señales comunes como claves privadas y credenciales con nombre. Es una protección heurística, no una garantía: puede dejar pasar secretos o descartar texto que parezca una credencial. Se puede desactivar desde la interfaz; el cambio afecta al contenido nuevo y no modifica el historial existente.

En **Ajustes → Pantallas**, cada pantalla detectada muestra sus modos DRM compatibles. Puedes seleccionar, por ejemplo, `1920 × 1080` para una pantalla 4K y aplicar el cambio sin cerrar sesión; la escala y la disposición se configuran por separado. Si KMS rechaza el modo guardado durante el arranque, el compositor prueba otros modos anunciados por la pantalla y registra cuál pudo activar.

## Instalación de la sesión

El script [install-session.sh](./scripts/install-session.sh) compila los binarios e instala la sesión de Malus DE para GDM en el sistema. Requiere privilegios de administrador y modifica rutas del sistema; revisa el script y confirma las dependencias de tu distribución antes de ejecutarlo:

```bash
sudo ./scripts/install-session.sh
```

La instalación no modifica los archivos ni las preferencias de VS Code, Antigravity, Firefox o Brave. La sesión de Malus comparte `XDG_CONFIG_HOME` con el usuario y no aísla la configuración GTK. Nautilus puede así usar la configuración personal y mostrar las carpetas XDG del usuario en Lugares; esas carpetas se determinan mediante `~/.config/user-dirs.dirs`, no mediante los marcadores. Comparte la configuración GTK y los marcadores de GNOME, por lo que las aplicaciones pueden modificarlos. Haz una copia de seguridad de `~/.config/gtk-3.0`, `~/.config/gtk-4.0` y `~/.config/user-dirs.dirs` antes de iniciar la sesión, y cierra cualquier otra sesión GNOME para evitar que Nautilus reutilice una instancia abierta.

El lanzador crea las carpetas personales XDG estándar si faltan y añade sus ubicaciones como marcadores GTK. Estos marcadores son accesos adicionales y se administran por separado de las carpetas que Nautilus muestra en Lugares. Malus también comparte el bus D-Bus de la sesión para que los navegadores y otras aplicaciones puedan comunicarse con xdg-desktop-portal.

La [guía de manifiesto](./docs/PACKAGE_MANIFEST.md) documenta dependencias de ejecución, compilación y servicios que deben habilitarse.
Los selectores de archivos de GTK y aplicaciones compatibles (por ejemplo, VS Code) usan xdg-desktop-portal. Malus prefiere el backend GNOME para estos diálogos, con GTK como alternativa; instala ambos backends. Para aplicar cambios al lanzador de sesión, vuelve a ejecutar `sudo ./scripts/install-session.sh` y cierra e inicia sesión por completo.

El menú global solo puede mostrar y ejecutar menús que la aplicación publique mediante `com.canonical.dbusmenu` y `com.canonical.AppMenu.Registrar`. Malus no inventa menús aproximados para aplicaciones que no los publican; algunas aplicaciones Electron, incluida VS Code según su versión y configuración, pueden no exponerlos en Linux.

## Incidencias pendientes

Se reporta que las funciones solicitadas no están funcionando. No se han especificado aún los fallos concretos ni pasos para reproducirlos, por lo que queda pendiente su diagnóstico; las características descritas arriba no deben considerarse verificadas por este reporte.

## Documentación

- [PRD y arquitectura técnica](./PRD.md)
- [Manifiesto de dependencias y paquetes](./docs/PACKAGE_MANIFEST.md)
- [Guía de desarrollo de temas](./docs/THEME_DEVELOPMENT_GUIDE.md)

## Licencia

El workspace declara las licencias **MIT OR Apache-2.0** en `Cargo.toml`.
