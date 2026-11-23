#!/usr/bin/env bash
# ==============================================================================
# Malus DE - Instalador Todo-en-Uno para Fedora (Cualquier Edición o Minimal)
# Uso rápido (estilo Brave):
#   curl -fsSL https://raw.githubusercontent.com/chanovera-developer/malus-de/main/scripts/install-fedora-minimal.sh | bash
# ==============================================================================
set -euo pipefail

REPO_URL="https://github.com/chanovera-developer/malus-de.git"
BRANCH="${MALUS_BRANCH:-main}"
INSTALL_DIR="${MALUS_DIR:-}"
TTY_AUTOSTART=false
ENABLE_GREETD=false
SKIP_BUILD=false

usage() {
    cat << 'EOF'
Uso: install-fedora-minimal.sh [opciones]

Instala Malus DE en cualquier edición de Fedora (Workstation/GNOME, KDE Plasma,
Xfce, Cinnamon, Sway, etc.) o sobre Fedora Minimal / Server en modo terminal.

Opciones:
  --dir RUTA         Carpeta donde clonar/compilar (por defecto: ~/malus-de)
  --branch RAMA      Rama de Git a compilar (por defecto: main)
  --tty-autostart    Añadir inicio automático en TTY1 a ~/.bash_profile (solo si no hay Display Manager activo, o forzado)
  --greetd           Instalar y habilitar greetd + tuigreet en VT1 (reemplaza el Display Manager activo si existe)
  --skip-build       Omitir `cargo build` si el workspace ya está compilado
  -h, --help         Mostrar esta ayuda
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dir)
            INSTALL_DIR="${2:-}"
            shift 2
            ;;
        --branch)
            BRANCH="${2:-}"
            shift 2
            ;;
        --tty-autostart)
            TTY_AUTOSTART=true
            shift
            ;;
        --greetd)
            ENABLE_GREETD=true
            shift
            ;;
        --skip-build)
            SKIP_BUILD=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "❌ Opción desconocida: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

echo "============================================================"
echo "🍎 MALUS DE — INSTALADOR UNIVERSAL PARA FEDORA"
echo "============================================================"

if ! command -v dnf >/dev/null 2>&1; then
    echo "❌ Error: No se encontró 'dnf'. Este instalador está diseñado para Fedora." >&2
    exit 1
fi

SUDO=""
TARGET_USER="$USER"
TARGET_HOME="$HOME"
if [ "$(id -u)" -ne 0 ]; then
    if ! command -v sudo >/dev/null 2>&1; then
        echo "❌ Error: Se requiere 'sudo' o ejecutar como root." >&2
        exit 1
    fi
    SUDO="sudo"
    echo "🔑 Solicitando permisos de administrador para instalar dependencias y binarios..."
    sudo -v
elif [ -n "${SUDO_USER:-}" ] && [ "${SUDO_USER}" != "root" ]; then
    TARGET_USER="$SUDO_USER"
    TARGET_HOME="$(getent passwd "$TARGET_USER" 2>/dev/null | cut -d: -f6 || echo "$HOME")"
fi

echo ""
echo "📦 [1/5] Instalando dependencias runtime y de compilación con DNF..."
$SUDO dnf install -y --skip-unavailable \
    mesa-dri-drivers \
    mesa-vulkan-drivers \
    xorg-x11-server-Xwayland \
    libseat \
    gtk4 \
    libadwaita \
    gtk4-layer-shell \
    gsettings-desktop-schemas \
    adwaita-icon-theme \
    rsms-inter-fonts \
    rsms-inter-vf-fonts \
    pipewire \
    wireplumber \
    pipewire-utils \
    NetworkManager \
    NetworkManager-wifi \
    bluez \
    bluez-tools \
    gvfs \
    udisks2 \
    libcanberra-gtk3 \
    polkit \
    mate-polkit \
    gnome-keyring \
    gnome-keyring-pam \
    libsecret \
    xdg-desktop-portal \
    xdg-desktop-portal-gtk \
    wl-clipboard \
    pam \
    rust \
    cargo \
    gcc \
    clang \
    pkgconf-pkg-config \
    git \
    curl \
    gtk4-devel \
    libadwaita-devel \
    gtk4-layer-shell-devel \
    gdk-pixbuf2-devel \
    'pkgconfig(gbm)' \
    libdrm-devel \
    wayland-devel \
    libinput-devel \
    libseat-devel \
    systemd-devel \
    dbus-devel \
    libxkbcommon-devel \
    sqlite-devel

# Instalar paquetes que pueden tener alternativas preinstaladas en otros escritorios de Fedora
# (ej. pulseaudio vs pipewire-pulseaudio, tuned-ppd vs power-profiles-daemon, ffmpeg completo vs ffmpeg-free)
if ! rpm -q pipewire-pulseaudio >/dev/null 2>&1 && ! rpm -q pulseaudio >/dev/null 2>&1; then
    $SUDO dnf install -y pipewire-pulseaudio || true
fi
if ! rpm -q power-profiles-daemon >/dev/null 2>&1 && ! rpm -q tuned-ppd >/dev/null 2>&1; then
    $SUDO dnf install -y power-profiles-daemon || $SUDO dnf install -y tuned-ppd || true
fi
if ! command -v ffmpeg >/dev/null 2>&1; then
    $SUDO dnf install -y ffmpeg-free || true
fi

$SUDO systemctl enable NetworkManager >/dev/null 2>&1 || true
$SUDO systemctl enable bluetooth >/dev/null 2>&1 || true

echo ""
echo "📥 [2/5] Preparando el código fuente de Malus DE..."
if [ -z "$INSTALL_DIR" ]; then
    if [ -f "$PWD/Cargo.toml" ] && grep -q 'malus-de-compositor' "$PWD/Cargo.toml" 2>/dev/null; then
        INSTALL_DIR="$PWD"
        echo "📂 Usando repositorio local existente en: $INSTALL_DIR"
    else
        INSTALL_DIR="$TARGET_HOME/malus-de"
    fi
fi

if [ -d "$INSTALL_DIR/.git" ]; then
    echo "🔄 Actualizando repositorio en $INSTALL_DIR ($BRANCH)..."
    git -C "$INSTALL_DIR" fetch origin "$BRANCH"
    git -C "$INSTALL_DIR" checkout "$BRANCH"
    git -C "$INSTALL_DIR" pull --ff-only origin "$BRANCH"
else
    mkdir -p "$(dirname "$INSTALL_DIR")"
    echo "⬇️  Clonando $REPO_URL en $INSTALL_DIR..."
    git clone --branch "$BRANCH" "$REPO_URL" "$INSTALL_DIR"
fi

if [ "$SKIP_BUILD" = false ]; then
    echo ""
    echo "🛠️  [3/5] Compilando Malus DE en perfil Release (esto puede tardar varios minutos)..."
    (cd "$INSTALL_DIR" && cargo build --release --workspace)
else
    echo ""
    echo "⏭️  [3/5] Compilación omitida por --skip-build."
fi

echo ""
echo "🚀 [4/5] Instalando binarios, temas, iconos y sesión en el sistema..."
$SUDO bash "$INSTALL_DIR/scripts/install-session.sh" --skip-build

echo ""
echo "⚙️  [5/5] Configurando arranque de sesión..."

# Detectar si ya existe un Display Manager gráfico activo o habilitado (GDM, SDDM, LightDM, LXDM, Greetd)
ACTIVE_DM=""
for dm in gdm sddm lightdm lxdm greetd; do
    if systemctl is-enabled "${dm}.service" >/dev/null 2>&1 || systemctl is-active "${dm}.service" >/dev/null 2>&1; then
        ACTIVE_DM="$dm"
        break
    fi
done

if [ "$TTY_AUTOSTART" = true ]; then
    BASH_PROFILE="$TARGET_HOME/.bash_profile"
    touch "$BASH_PROFILE"
    if ! grep -q '/usr/local/bin/malus-session' "$BASH_PROFILE" 2>/dev/null; then
        cat << 'EOF' >> "$BASH_PROFILE"

# Iniciar Malus DE automáticamente en TTY1
if [ -z "$WAYLAND_DISPLAY" ] && [ -z "$DISPLAY" ] && [ "${XDG_VTNR:-0}" -eq 1 ]; then
    /usr/local/bin/malus-session
fi
EOF
        chown "$TARGET_USER:$TARGET_USER" "$BASH_PROFILE" 2>/dev/null || true
        echo "   ✔ Autoinicio en TTY1 añadido a $BASH_PROFILE"
    else
        echo "   ✔ Autoinicio en TTY1 ya estaba configurado en $BASH_PROFILE"
    fi
fi

if [ "$ENABLE_GREETD" = true ]; then
    $SUDO dnf install -y greetd tuigreet
    $SUDO mkdir -p /etc/greetd
    $SUDO tee /etc/greetd/config.toml >/dev/null << 'EOF'
[terminal]
vt = 1

[default_session]
command = "tuigreet --time --remember --cmd /usr/local/bin/malus-session"
user = "greeter"
EOF
    if [ -n "$ACTIVE_DM" ] && [ "$ACTIVE_DM" != "greetd" ]; then
        echo "   ℹ️  Deshabilitando $ACTIVE_DM para activar greetd..."
        $SUDO systemctl disable "${ACTIVE_DM}.service" >/dev/null 2>&1 || true
    fi
    $SUDO systemctl set-default graphical.target >/dev/null 2>&1 || true
    $SUDO systemctl enable greetd
    ACTIVE_DM="greetd"
    echo "   ✔ greetd + tuigreet habilitados para el próximo arranque"
fi

echo ""
echo "============================================================"
echo "🎉 ¡MALUS DE INSTALADO Y COMPILADO EN FEDORA!"
echo "============================================================"
if [ -n "$ACTIVE_DM" ] && [ "$ACTIVE_DM" != "greetd" ]; then
    echo "✅ Se detectó el gestor de inicio de sesión '${ACTIVE_DM^^}'."
    echo "   La sesión 'Malus DE' ya quedó registrada en /usr/share/wayland-sessions/malus.desktop."
    echo ""
    echo "Para entrar a Malus DE:"
    echo "  1. Cierra tu sesión actual (Log Out)."
    echo "  2. En la pantalla de login (${ACTIVE_DM^^}), elige la sesión 'Malus DE'."
    echo "  3. Inicia sesión con tu contraseña."
    echo ""
    echo "También puedes probarlo en ventana dentro de tu escritorio actual o desde una TTY con:"
    echo "  /usr/local/bin/malus-session"
else
    echo "Para iniciar la sesión gráfica ahora mismo desde la TTY ejecuta:"
    echo "  /usr/local/bin/malus-session"
    if [ "$TTY_AUTOSTART" = false ] && [ "$ENABLE_GREETD" = false ]; then
        echo ""
        echo "Si estás en Fedora Minimal sin gestor de login y deseas inicio automático en TTY1:"
        echo "  $INSTALL_DIR/scripts/install-fedora-minimal.sh --skip-build --tty-autostart"
    fi
fi
echo "============================================================"

