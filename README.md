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
- Historial local en memoria de hasta 50 elementos de texto, imágenes PNG y listas de archivos, accesible con `Super + V`.
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

El gestor de portapapeles conserva durante la sesión los últimos 50 elementos copiados (texto, enlaces, imágenes PNG y URI de archivos). Pulsa `Super + V` para buscar, fijar o volver a copiar un elemento. El historial solo se mantiene en memoria y se elimina al cerrar la sesión; se omite el contenido marcado como sensible y se requiere `wl-clipboard`.

En **Ajustes → Pantallas**, cada pantalla detectada muestra sus modos DRM compatibles. Puedes seleccionar, por ejemplo, `1920 × 1080` para una pantalla 4K y aplicar el cambio sin cerrar sesión; la escala y la disposición se configuran por separado. Si KMS rechaza el modo guardado durante el arranque, el compositor prueba otros modos anunciados por la pantalla y registra cuál pudo activar.

## Instalación de la sesión

El script [install-session.sh](./scripts/install-session.sh) compila los binarios e instala la sesión de Malus DE para GDM en el sistema. Requiere privilegios de administrador y modifica rutas del sistema; revisa el script y confirma las dependencias de tu distribución antes de ejecutarlo:

```bash
sudo ./scripts/install-session.sh
```

La instalación no modifica los archivos ni las preferencias de VS Code, Antigravity, Firefox o Brave. La posición izquierda de los controles GTK se configura en el entorno aislado de Malus DE; los navegadores conservan sus controles propios.

Nautilus usa en Malus DE una configuración XDG aislada. En el primer inicio, la sesión importa al perfil de Malus los marcadores GTK existentes de GNOME que aún no estén allí; la importación no se repite para respetar cambios posteriores hechos desde Malus. También añade las carpetas personales XDG como marcadores etiquetados en la sección separada de las ubicaciones nativas y evita duplicados al iniciar sesión. Si no existe la configuración de carpetas personales, crea las carpetas estándar. Malus comparte el bus D-Bus de la sesión para que los navegadores y otras aplicaciones puedan comunicarse con xdg-desktop-portal; si Nautilus ya está abierto en otra sesión GNOME, podría reutilizarse esa instancia.

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
