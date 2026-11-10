# Guía de Instalación Standalone en Fedora Minimal (Modo Terminal)

Esta guía documenta el procedimiento completo para desplegar **Malus DE como un entorno de escritorio 100% autónomo** sobre una instalación base de **Fedora en modo terminal puro**, eliminando cualquier dependencia de `gnome-shell` y `mutter`.

---

## 1. Filosofía y Objetivos

* **Autonomía Total:** Demostrar que Malus DE opera como un entorno de primera clase, equivalente a Sway o Hyprland, autogestionando toda la capa gráfica sobre Smithay.
* **Cero Sobrecarga (Zero Bloat):** Sin extensiones en JavaScript, sin daemons innecesarios de GNOME y con consumo en reposo de **~450 MB de RAM** frente a los ~1.5 GB de una instalación estándar de Workstation.
* **Velocidad:** Arranque directo a la sesión gráfica en **menos de 1.5 segundos**.

---

## 2. Paso 1: Instalación de Fedora Base (Solo Terminal)

1. Descarga la imagen **Fedora Everything (Netinstall)** o **Fedora Server**:
   * [Fedora Everything Netinstall ISO](https://alt.fedoraproject.org/)
2. En el instalador interactivo de Fedora (Anaconda):
   * Ve a **Selección de Software (Software Selection)**.
   * Elige **Instalación Mínima (Minimal Install)** o **Sistema Operativo Estándar**.
   * **IMPORTANTE:** No selecciones ningún grupo de entorno de escritorio (*Fedora Workstation*, *GNOME*, *KDE Plasma*, *Xfce*, etc.).
3. Completa la partición y la creación del usuario administrador.
4. Reinicia el equipo: el sistema presentará únicamente un prompt de texto en la consola de Linux (`tty1`).

---

## 3. Paso 2: Instalación de Dependencias Runtime (Sin GNOME)

Inicia sesión con tu usuario y contraseña en la terminal e instala en un solo bloque todos los paquetes necesarios del sistema:

```bash
sudo dnf install -y \
    mesa-dri-drivers \
    mesa-vulkan-drivers \
    xorg-x11-server-Xwayland \
    libseat \
    gtk4 \
    libadwaita \
    gtk4-layer-shell \
    gsettings-desktop-schemas \
    adwaita-icon-theme \
    inter-fonts \
    pipewire \
    wireplumber \
    pipewire-pulseaudio \
    pipewire-utils \
    NetworkManager \
    NetworkManager-wifi \
    bluez \
    bluez-tools \
    power-profiles-daemon \
    gvfs \
    udisks2 \
    ffmpeg-free \
    libcanberra-gtk3 \
    polkit \
    mate-polkit \
    gnome-keyring \
    gnome-keyring-pam \
    libsecret \
    xdg-desktop-portal \
    xdg-desktop-portal-gtk \
    wl-clipboard
```

### Justificación de Paquetes Clave:
* **`xdg-desktop-portal-gtk`:** Sustituye a `xdg-desktop-portal-gnome`. Es un portal liviano para selección de archivos y portapapeles sin dependencias de Mutter.
* **`gnome-keyring` & `gnome-keyring-pam`:** Proporcionan la interfaz estándar `org.freedesktop.secrets` donde navegadores (Chrome, Firefox), Git y editores guardan contraseñas seguras.
* **`gsettings-desktop-schemas`:** Solo contiene esquemas XML (variables de sistema para botones a la izquierda y modo oscuro). No ejecuta procesos.
* **`mate-polkit`:** Agente gráfico que solicita la contraseña de administrador (`sudo`) al instalar software.
* **`gvfs` & `udisks2`:** Permiten que el escritorio y el gestor de archivos monten memorias USB y gestionen la papelera.

---

## 4. Paso 3: Gestión de Sesión y Login (Sin GDM)

En Fedora, el paquete `gdm` depende directamente de `gnome-shell`. Para evitarlo, elige una de las dos siguientes alternativas:

### Opción A: Inicio Directo desde la TTY (Recomendada para pruebas y máxima ligereza)

Esta opción no requiere ningún Display Manager. Al iniciar sesión en la consola negra `tty1`, arranca Malus DE automáticamente:

Edita o crea tu archivo `~/.bash_profile`:
```bash
cat << 'EOF' >> ~/.bash_profile

# Iniciar Malus DE automáticamente si iniciamos sesión en la TTY1 física
if [ -z "$WAYLAND_DISPLAY" ] && [ "$XDG_VTNR" -eq 1 ]; then
    exec /usr/local/bin/malus-session
fi
EOF
```

### Opción B: Pantalla de Login Ligera Wayland (`greetd` + `tuigreet`)

Si deseas una pantalla visual interactiva para seleccionar usuario e introducir contraseña:

1. Instala `greetd` y `tuigreet`:
   ```bash
   sudo dnf install -y greetd tuigreet
   ```
2. Configura `/etc/greetd/config.toml`:
   ```toml
   [terminal]
   vt = 1

   [default_session]
   command = "tuigreet --time --remember --cmd /usr/local/bin/malus-session"
   user = "greeter"
   ```
3. Habilita el servicio:
   ```bash
   sudo systemctl enable greetd --now
   ```

---

## 5. Paso 4: Despliegue de Malus DE

### Opción A: Desplegar Binarios Precompilados (Más Rápido)
Si ya tienes compilado Malus DE en tu equipo de desarrollo:
1. Copia la carpeta del repositorio `malus-de` a la máquina nueva (mediante un pendrive USB o `scp`).
2. Entra al directorio y ejecuta el instalador del sistema saltando el paso de compilación:
   ```bash
   cd malus-de
   sudo ./scripts/install-session.sh --skip-build
   ```

### Opción B: Compilar desde las Fuentes en la Máquina Limpia
Si prefieres construir todo directamente en la máquina limpia:
1. Instala las herramientas de desarrollo y bibliotecas header:
   ```bash
   sudo dnf install -y \
       rust cargo gcc clang pkgconf-pkg-config git \
       gtk4-devel libadwaita-devel gtk4-layer-shell-devel \
       mesa-libgbm-devel libinput-devel libseat-devel \
       systemd-devel dbus-devel libxkbcommon-devel sqlite-devel
   ```
2. Clona y compila en perfil Release:
   ```bash
   git clone https://github.com/chanovera-dev/malus-de.git
   cd malus-de
   cargo build --release
   sudo ./scripts/install-session.sh --skip-build
   ```

---

## 6. Paso 5: Suite de Aplicaciones de Usuario (Experiencia macOS)

Con Malus DE no necesitas aplicaciones pesadas de GNOME, pues ya dispones de herramientas nativas:

| Rol | Aplicación en Malus DE | Comando / Paquete |
| :--- | :--- | :--- |
| **Explorador de Archivos (Finder)** | **Branches** | `malus-de-branches` (Vistas en iconos, lista, columnas Miller y Quick Look con barra espaciadora) |
| **Ajustes del Sistema** | **Malus Settings** | `malus-de-settings` (Pantallas, Dock, Fondo, Teclado, Sonido, Spotlight, etc.) |
| **Terminal macOS** | **Ptyxis** (o Alacritty) | `sudo dnf install -y ptyxis` |
| **Navegador Web** | **Firefox** (con soporte Wayland nativo) | `sudo dnf install -y firefox` |
| **Monitor del Sistema** | **btop** (o gnome-system-monitor) | `sudo dnf install -y btop` |

---

## 7. Diagnóstico y Registro de Sesión

Si necesitas revisar la salida o depurar el inicio de sesión:
* Registro general de sesión: `~/.local/share/malus-de/session.log`
* Estado de los servicios de usuario: `systemctl --user status malus-session.target`
* Monitoreo de socket Wayland: `echo $WAYLAND_DISPLAY` (debe reportar `wayland-0` o `wayland-1`).
