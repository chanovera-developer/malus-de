# Malus DE

**Malus DE** es una reimplementación libre del entorno de escritorio de macOS para Linux, construida sobre Wayland. Busca trasladar su estética y ergonomía a un sistema abierto y modular, con un compositor escrito en Rust y componentes de shell independientes.

> **Estado:** en desarrollo activo. El PRD marca como implementados el compositor base, el dock, la barra superior y el centro de control, entre otros hitos. La integración completa de AirDrop/AWDL sigue en progreso; consulta [PRD.md](./PRD.md) para ver el estado detallado.

## Características

- Compositor Wayland basado en [Smithay](https://github.com/Smithay/smithay), con backend anidado Winit y backend DRM/KMS.
- Shell modular con barra superior, menú global, centro de control, dock flotante, Spotlight (incluye acciones para cerrar sesión, reiniciar y apagar), notificaciones y fondo de pantalla.
- Escritorio interactivo con iconos movibles, intercambio de archivos por arrastrar y soltar con el gestor de archivos y miniaturas de imágenes.
- Selector visual de ventanas Alt-Tab/Super-Tab funcional para recorrer y cambiar entre ventanas abiertas; su presentación y comportamiento siguen en desarrollo.
- Integración con servicios de Linux para audio, red, Bluetooth y controles multimedia.
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
| Bloqueo de sesión, captura, enlace y CLI | `malus-de-session-lock`, `malus-de-screenshot`, `malus-de-link`, `malus-de-ctl` |

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

En **Ajustes → Pantallas**, cada pantalla detectada muestra sus modos DRM compatibles. Puedes seleccionar, por ejemplo, `1920 × 1080` para una pantalla 4K y aplicar el cambio sin cerrar sesión; la escala y la disposición se configuran por separado.

## Instalación de la sesión

El script [install-session.sh](./scripts/install-session.sh) compila los binarios e instala la sesión de Malus DE para GDM en el sistema. Requiere privilegios de administrador y modifica rutas del sistema; revisa el script y confirma las dependencias de tu distribución antes de ejecutarlo:

```bash
sudo ./scripts/install-session.sh
```

La instalación no modifica los archivos ni las preferencias de VS Code, Antigravity, Firefox o Brave. La posición izquierda de los controles GTK se configura en el entorno aislado de Malus DE; los navegadores conservan sus controles propios.

Nautilus usa en Malus DE una configuración XDG aislada. En el primer inicio, la sesión importa al perfil de Malus los marcadores GTK existentes de GNOME que aún no estén allí; la importación no se repite para respetar cambios posteriores hechos desde Malus. También añade las carpetas personales XDG como marcadores etiquetados en la sección separada de las ubicaciones nativas y evita duplicados al iniciar sesión. Si no existe la configuración de carpetas personales, crea las carpetas estándar. La sesión usa un bus D-Bus separado para evitar que Nautilus reutilice la instancia de GNOME y omita estos marcadores.

La [guía de manifiesto](./docs/PACKAGE_MANIFEST.md) documenta dependencias de ejecución, compilación y servicios que deben habilitarse.
Los selectores de archivos de GTK y aplicaciones compatibles (por ejemplo, VS Code) usan xdg-desktop-portal. Malus prefiere el backend GNOME para estos diálogos, con GTK como alternativa; instala ambos backends y cierra e inicia sesión después de actualizar Malus.

El menú global solo puede mostrar y ejecutar menús que la aplicación publique mediante `com.canonical.dbusmenu` y `com.canonical.AppMenu.Registrar`. Malus no inventa menús aproximados para aplicaciones que no los publican; algunas aplicaciones Electron, incluida VS Code según su versión y configuración, pueden no exponerlos en Linux.

## Documentación

- [PRD y arquitectura técnica](./PRD.md)
- [Manifiesto de dependencias y paquetes](./docs/PACKAGE_MANIFEST.md)
- [Guía de desarrollo de temas](./docs/THEME_DEVELOPMENT_GUIDE.md)

## Licencia

El workspace declara las licencias **MIT OR Apache-2.0** en `Cargo.toml`.
