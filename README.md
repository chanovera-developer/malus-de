# Malus DE

**Malus DE** es un entorno de escritorio modular para Linux basado en Wayland. Su objetivo es combinar una interfaz inspirada en macOS con la flexibilidad de Linux, usando un compositor escrito en Rust y componentes de shell independientes.

> **Estado:** en desarrollo activo. El PRD marca como implementados el compositor base, el dock, la barra superior y el centro de control, entre otros hitos. La integración completa de AirDrop/AWDL sigue en progreso; consulta [PRD.md](./PRD.md) para ver el estado detallado.

## Características

- Compositor Wayland basado en [Smithay](https://github.com/Smithay/smithay), con backend anidado Winit y backend DRM/KMS.
- Shell modular con barra superior, menú global, centro de control, dock flotante, Spotlight, notificaciones y fondo de pantalla.
- Integración con servicios de Linux para audio, red, Bluetooth y controles multimedia.
- Efectos gráficos con shaders, esquinas squircle y animaciones de ventanas.
- Soporte para configuración de teclado y disposición de monitores.
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

Clona el repositorio y, con las dependencias del sistema instaladas, compila desde su raíz:

```bash
git clone https://github.com/chanovera-dev/malus-de.git
cd malus-de
cargo build --workspace
```

Para compilar en modo optimizado:

```bash
cargo build --workspace --release
```

## Instalación de la sesión

El script [install-session.sh](./scripts/install-session.sh) compila los binarios e instala la sesión de Malus DE para GDM en el sistema. Requiere privilegios de administrador y modifica rutas del sistema; revisa el script y confirma las dependencias de tu distribución antes de ejecutarlo:

```bash
sudo ./scripts/install-session.sh
```

La [guía de manifiesto](./docs/PACKAGE_MANIFEST.md) documenta dependencias de ejecución, compilación y servicios que deben habilitarse.

## Documentación

- [PRD y arquitectura técnica](./PRD.md)
- [Manifiesto de dependencias y paquetes](./docs/PACKAGE_MANIFEST.md)
- [Guía de desarrollo de temas](./docs/THEME_DEVELOPMENT_GUIDE.md)

## Licencia

El workspace declara las licencias **MIT OR Apache-2.0** en `Cargo.toml`.
