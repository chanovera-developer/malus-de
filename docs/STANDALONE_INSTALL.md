# Guía de Instalación

Esta guía documenta el procedimiento completo para desplegar **Malus DE** tanto sobre **cualquier edición de Fedora|Arch|Debian|Ubuntu con escritorio preinstalado** como sobre una instalación base de **Fedora|Arch|Debian|Ubuntu en modo terminal puro (Minimal / Everything)**, sin depender de `gnome-shell` ni `mutter`.

---

## Instalación Rápida en Un Solo Comando (Cualquier Fedora|Arch|Debian|Ubuntu)

Tanto si estás en **Fedora con cualquier entorno de escritorio** (GNOME, KDE Plasma, Xfce, etc.) como en la terminal de **Fedora Minimal**, puedes instalar todas las dependencias, clonar el repositorio, compilar Malus DE en perfil `Release` y registrar la sesión Wayland en un único comando:

```bash
curl -fsSL https://raw.githubusercontent.com/chanovera-developer/malus-de/main/automate-install.sh | bash
```

* **En una instalación con escritorio:** El instalador respeta los paquetes de tu sistema, compila e instala Malus DE y registra automáticamente la sesión **Malus DE** en `/usr/share/wayland-sessions/malus.desktop`. Solo tienes que cerrar sesión (*Log Out*) y elegir **Malus DE** en tu pantalla de login.
* **En una instalación en modo terminal puro (Minimal / Everything):** Si además quieres dejar configurado el arranque automático en `tty1` o instalar `greetd` + `tuigreet` directamente desde el mismo comando:

```bash
# Con arranque automático en TTY1 (~/.bash_profile)
curl -fsSL https://raw.githubusercontent.com/chanovera-developer/malus-de/main/automate-install.sh | bash -s -- --tty-autostart

# O con gestor visual ligero greetd + tuigreet
curl -fsSL https://raw.githubusercontent.com/chanovera-developer/malus-de/main/automate-install.sh | bash -s -- --greetd
```

> [!NOTE]
> El script [automate-install.sh](../automate-install.sh) descarga el código en `~/malus-de` (configurable con `--dir`), ejecuta `cargo build --release --workspace`, despliega los binarios en `/usr/local/bin/` y registra la sesión Wayland para cualquier Display Manager. Si prefieres ejecutar o revisar cada paso manualmente en una instalación limpia, sigue las secciones detalladas a continuación.

---

## 1. Filosofía y Objetivos

* **Autonomía Total:** Demostrar que Malus DE opera como un entorno de primera clase, equivalente a Sway o Hyprland, autogestionando toda la capa gráfica sobre Smithay.
* **Cero Sobrecarga (Zero Bloat):** Sin extensiones en JavaScript, sin daemons innecesarios de GNOME y con consumo en reposo de **~450 MB de RAM** frente a los ~1.5 GB de una instalación estándar de Workstation.
* **Velocidad:** Arranque directo a la sesión gráfica en **menos de 1.5 segundos**.

---

### Justificación de Paquetes Clave:
* **`xdg-desktop-portal-gtk`:** Sustituye a `xdg-desktop-portal-gnome`. Es un portal liviano para selección de archivos y portapapeles sin dependencias de Mutter.
* **`gnome-keyring` & `gnome-keyring-pam`:** Proporcionan la interfaz estándar `org.freedesktop.secrets` donde navegadores (Chrome, Firefox), Git y editores guardan contraseñas seguras.
* **`gsettings-desktop-schemas`:** Solo contiene esquemas XML (variables de sistema para botones a la izquierda y modo oscuro). No ejecuta procesos.
* **`mate-polkit`:** Agente gráfico que solicita la contraseña de administrador (`sudo`) al instalar software.
* **`gvfs` & `udisks2`:** Permiten que el escritorio y el gestor de archivos monten memorias USB y gestionen la papelera.

---

## 3. Suite de Aplicaciones de Usuario (Experiencia macOS)

Con Malus DE no necesitas aplicaciones pesadas de GNOME, pues ya dispones de herramientas nativas:

| Rol | Aplicación en Malus DE | Comando / Paquete |
| :--- | :--- | :--- |
| **Explorador de Archivos (Finder)** | **Branches** | `malus-de-branches` (Vistas en iconos, lista, columnas Miller y Quick Look con barra espaciadora) |
| **Ajustes del Sistema** | **Malus Settings** | `malus-de-settings` (Pantallas, Dock, Fondo, Teclado, Sonido, Spotlight, etc.) |
| **Terminal macOS** | **Ptyxis** (o Alacritty) | `sudo dnf install -y ptyxis` |

---

## 7. Diagnóstico y Registro de Sesión

Si necesitas revisar la salida o depurar el inicio de sesión:
* Registro general de sesión: `~/.local/share/malus-de/session.log`
* Procesos activos del entorno: `pgrep -af "malus-de"`
* Monitoreo de socket Wayland: `echo $WAYLAND_DISPLAY` (debe reportar `wayland-0` o `wayland-1`).

