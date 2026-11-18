#!/usr/bin/env bash
# ==============================================================================
# Malus DE - Instalador Todo-en-Uno para Fedora Minimal (Descarga, Compila e Instala)
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

Opciones:
  --dir RUTA         Carpeta donde clonar/compilar (por defecto: ~/malus-de)
  --branch RAMA      Rama de Git a compilar (por defecto: main)
  --tty-autostart    Añadir inicio automático en TTY1 a ~/.bash_profile
  --greetd           Instalar y habilitar greetd + tuigreet en VT1
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
echo "🍎 MALUS DE — INSTALADOR STANDALONE PARA FEDORA MINIMAL"
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
$SUDO dnf install -y \
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
if [ "$TTY_AUTOSTART" = true ]; then
    BASH_PROFILE="$TARGET_HOME/.bash_profile"
    touch "$BASH_PROFILE"
    if ! grep -q '/usr/local/bin/malus-session' "$BASH_PROFILE" 2>/dev/null; then
        cat << 'EOF' >> "$BASH_PROFILE"

# Iniciar Malus DE automáticamente en TTY1
if [ -z "$WAYLAND_DISPLAY" ] && [ "${XDG_VTNR:-0}" -eq 1 ]; then
    /usr/local/bin/malus-session
fi
EOF
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
    $SUDO systemctl enable greetd
    echo "   ✔ greetd + tuigreet habilitados para el próximo arranque"
fi

echo ""
echo "============================================================"
echo "🎉 ¡MALUS DE INSTALADO Y COMPILADO EN FEDORA MINIMAL!"
echo "============================================================"
echo "Para iniciar la sesión gráfica ahora mismo desde la TTY ejecuta:"
echo "  /usr/local/bin/malus-session"
if [ "$TTY_AUTOSTART" = false ] && [ "$ENABLE_GREETD" = false ]; then
    echo ""
    echo "Si deseas inicio automático en TTY1, vuelve a ejecutar con:"
    echo "  $INSTALL_DIR/scripts/install-fedora-minimal.sh --skip-build --tty-autostart"
fi
echo "============================================================"

