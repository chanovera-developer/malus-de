#!/usr/bin/env bash
# ==============================================================================
# Malus DE - Instalador Todo-en-Uno (Fedora / Arch / Debian y derivadas)
#
# Instalación con una sola línea:
#   curl -fsSL https://raw.githubusercontent.com/chanovera-developer/malus-de/main/automate-install.sh | bash
#
# Qué hace:
#   1. Detecta la distribución e instala sus dependencias (incluye Ptyxis).
#   2. Clona (o reutiliza) el repositorio y compila en perfil Release.
#   3. Instala binarios y registra la sesión "Malus DE" (/usr/share/wayland-sessions).
#   4. Detecta el gestor de inicio de sesión (GDM, SDDM, LightDM, LXDM, greetd, Ly...).
#      Si no hay ninguno (distro solo-terminal) ofrece: arrancar desde la TTY,
#      autoinicio en TTY1 o instalar un gestor de inicio gráfico.
# ==============================================================================
set -euo pipefail

REPO_URL="https://github.com/chanovera-developer/malus-de.git"
BRANCH="${MALUS_BRANCH:-main}"
INSTALL_DIR="${MALUS_DIR:-}"
SKIP_BUILD=false
ASSUME_YES=false
FORCE_TTY_AUTOSTART=false
FORCE_DM=""

usage() {
    cat <<'EOF'
Uso: automate-install.sh [opciones]

  --dir RUTA        Carpeta donde clonar/compilar (por defecto: ~/malus-de)
  --branch RAMA     Rama de Git (por defecto: main)
  --skip-build      Omitir `cargo build` si ya está compilado
  --tty-autostart   Sin gestor de login: iniciar Malus DE automáticamente en TTY1
  --dm NOMBRE       Sin gestor de login: instalar gdm | sddm | lightdm | greetd
  -y, --yes         No preguntar (sin gestor de login: solo mostrar cómo arrancar)
  -h, --help        Mostrar esta ayuda

Uso por tubería con opciones:
  curl -fsSL <url>/automate-install.sh | bash -s -- --dm sddm
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dir) INSTALL_DIR="${2:-}"; shift 2 ;;
        --branch) BRANCH="${2:-}"; shift 2 ;;
        --skip-build) SKIP_BUILD=true; shift ;;
        --tty-autostart) FORCE_TTY_AUTOSTART=true; shift ;;
        --dm) FORCE_DM="${2:-}"; shift 2 ;;
        -y|--yes) ASSUME_YES=true; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "❌ Opción desconocida: $1" >&2; usage >&2; exit 1 ;;
    esac
done

echo "============================================================"
echo "🍎 MALUS DE — INSTALADOR TODO-EN-UNO"
echo "   (proyecto en desarrollo, pueden existir errores)"
echo "============================================================"

# ------------------------------------------------------------------------------
# Privilegios
# ------------------------------------------------------------------------------
SUDO=""
TARGET_USER="${USER:-$(id -un)}"
TARGET_HOME="$HOME"
if [ "$(id -u)" -ne 0 ]; then
    command -v sudo >/dev/null 2>&1 || { echo "❌ Se requiere 'sudo' o ejecutar como root." >&2; exit 1; }
    SUDO="sudo"
    echo "🔑 Solicitando permisos de administrador..."
    sudo -v
elif [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != "root" ]; then
    TARGET_USER="$SUDO_USER"
    TARGET_HOME="$(getent passwd "$TARGET_USER" | cut -d: -f6 || echo "$HOME")"
fi

# Entrada interactiva incluso con `curl | bash` (stdin es la tubería)
HAVE_TTY=false
if [ "$ASSUME_YES" = false ] && [ -r /dev/tty ] && [ -w /dev/tty ] && (: </dev/tty) 2>/dev/null; then
    HAVE_TTY=true
fi
ask() { # ask "pregunta" default -> REPLY
    local prompt="$1" def="${2:-}"
    REPLY="$def"
    if [ "$HAVE_TTY" = true ]; then
        read -r -p "$prompt" REPLY </dev/tty || REPLY="$def"
        REPLY="${REPLY:-$def}"
    fi
}

# ------------------------------------------------------------------------------
# 1. Detección de distribución
# ------------------------------------------------------------------------------
[ -r /etc/os-release ] || { echo "❌ No se encontró /etc/os-release." >&2; exit 1; }
OS_ID="$(. /etc/os-release && printf '%s' "${ID:-}")"
ID_LIKE="$(. /etc/os-release && printf '%s' "${ID_LIKE:-}")"
OS_NAME="$(. /etc/os-release && printf '%s' "${PRETTY_NAME:-$OS_ID}")"
IDS=" $OS_ID $ID_LIKE "

if [[ "$IDS" == *" arch "* ]]; then
    FAMILY="arch"
elif [[ "$IDS" == *" fedora "* || "$IDS" == *" rhel "* || "$IDS" == *" centos "* ]]; then
    FAMILY="fedora"
elif [[ "$IDS" == *" debian "* || "$IDS" == *" ubuntu "* ]]; then
    FAMILY="debian"
else
    echo "❌ Distribución no soportada: $OS_NAME (ID=$OS_ID, ID_LIKE=$ID_LIKE)." >&2
    echo "   Soportadas: Fedora/RHEL, Arch y derivadas, Debian/Ubuntu y derivadas." >&2
    exit 1
fi
echo "🐧 Distribución detectada: $OS_NAME  (familia: $FAMILY)"

# ------------------------------------------------------------------------------
# 2. Listas de dependencias por distribución (runtime + compilación + Ptyxis)
# ------------------------------------------------------------------------------
PKGS_FEDORA=(
    mesa-dri-drivers mesa-vulkan-drivers xorg-x11-server-Xwayland libseat
    gtk4 libadwaita gtk4-layer-shell gsettings-desktop-schemas adwaita-icon-theme
    rsms-inter-fonts rsms-inter-vf-fonts
    pipewire wireplumber pipewire-utils pipewire-pulseaudio
    NetworkManager NetworkManager-wifi bluez bluez-tools
    power-profiles-daemon gvfs udisks2 libcanberra-gtk3
    polkit mate-polkit gnome-keyring gnome-keyring-pam libsecret
    xdg-desktop-portal xdg-desktop-portal-gtk wl-clipboard libnotify pam
    ptyxis
    rust cargo gcc clang pkgconf-pkg-config git curl
    gtk4-devel libadwaita-devel gtk4-layer-shell-devel gdk-pixbuf2-devel
    'pkgconfig(gbm)' libdrm-devel wayland-devel libinput-devel libseat-devel
    systemd-devel dbus-devel libxkbcommon-devel sqlite-devel
)
PKGS_ARCH=(
    mesa vulkan-radeon vulkan-intel libva-mesa-driver xorg-xwayland seatd
    pipewire pipewire-pulse wireplumber networkmanager bluez bluez-utils
    power-profiles-daemon polkit mate-polkit gnome-keyring libsecret
    gtk4 libadwaita gtk4-layer-shell
    xdg-desktop-portal xdg-desktop-portal-gtk wl-clipboard libcanberra libnotify
    ffmpeg gvfs udisks2 inter-font adwaita-icon-theme gsettings-desktop-schemas
    qt5-wayland qt6-wayland
    ptyxis
    base-devel rust clang git curl pkgconf
    libdrm wayland libinput libxkbcommon sqlite dbus systemd-libs pam
)
PKGS_DEBIAN=(
    mesa-va-drivers mesa-vulkan-drivers xwayland libseat1
    pipewire pipewire-pulse wireplumber network-manager bluez bluez-tools
    power-profiles-daemon policykit-1 mate-polkit gnome-keyring libpam-gnome-keyring
    libsecret-1-0 libgtk-4-1 libadwaita-1-0 libgtk4-layer-shell0
    xdg-desktop-portal xdg-desktop-portal-gtk wl-clipboard libcanberra-gtk3-module
    libnotify-bin ffmpeg gvfs gvfs-backends udisks2 fonts-inter
    adwaita-icon-theme gsettings-desktop-schemas qtwayland5 qt6-wayland
    ptyxis
    build-essential cargo rustc clang pkg-config git curl ca-certificates
    libgtk-4-dev libadwaita-1-dev libgtk4-layer-shell-dev libgdk-pixbuf-2.0-dev
    libgbm-dev libdrm-dev libwayland-dev libinput-dev libseat-dev libudev-dev
    libdbus-1-dev libxkbcommon-dev libsqlite3-dev libpam0g-dev
)

# Instala paquetes ignorando los que no existan en los repos de esta versión.
pkg_install() {
    case "$FAMILY" in
        fedora)
            $SUDO dnf install -y --skip-unavailable --setopt=install_weak_deps=True "$@" ;;
        arch)
            local p avail=()
            for p in "$@"; do
                if pacman -Si "$p" >/dev/null 2>&1 || pacman -Qi "$p" >/dev/null 2>&1; then
                    avail+=("$p")
                else
                    echo "   ⚠ Paquete no disponible, se omite: $p"
                fi
            done
            $SUDO pacman -S --needed --noconfirm "${avail[@]}" ;;
        debian)
            local p avail=()
            for p in "$@"; do
                if apt-cache show "$p" >/dev/null 2>&1; then
                    avail+=("$p")
                else
                    echo "   ⚠ Paquete no disponible, se omite: $p"
                fi
            done
            $SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y "${avail[@]}" ;;
    esac
}

echo ""
echo "📦 [1/5] Instalando dependencias ($FAMILY)..."
case "$FAMILY" in
    fedora) PKGS=("${PKGS_FEDORA[@]}") ;;
    arch)
        PKGS=("${PKGS_ARCH[@]}")
        $SUDO pacman -Sy --noconfirm >/dev/null 2>&1 || true
        ;;
    debian)
        PKGS=("${PKGS_DEBIAN[@]}")
        $SUDO apt-get update
        ;;
esac
pkg_install "${PKGS[@]}"

$SUDO systemctl enable NetworkManager >/dev/null 2>&1 || true
$SUDO systemctl enable bluetooth >/dev/null 2>&1 || true

if ! command -v cargo >/dev/null 2>&1; then
    echo "🦀 cargo no disponible en los repos; instalando Rust con rustup..."
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
    # shellcheck disable=SC1091
    . "$TARGET_HOME/.cargo/env"
fi

# ------------------------------------------------------------------------------
# 3. Código fuente y compilación
# ------------------------------------------------------------------------------
echo ""
echo "📥 [2/5] Preparando el código fuente..."
if [ -z "$INSTALL_DIR" ]; then
    SELF_DIR=""
    if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
        SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    fi
    if [ -n "$SELF_DIR" ] && grep -q 'malus-de-compositor' "$SELF_DIR/Cargo.toml" 2>/dev/null; then
        INSTALL_DIR="$SELF_DIR"
        echo "📂 Usando repositorio local: $INSTALL_DIR"
    else
        INSTALL_DIR="$TARGET_HOME/malus-de"
    fi
fi

if [ -d "$INSTALL_DIR/.git" ]; then
    if [ -n "$(git -C "$INSTALL_DIR" status --porcelain 2>/dev/null)" ]; then
        echo "🔄 Hay cambios locales en $INSTALL_DIR; no se actualiza el repositorio."
    else
        echo "🔄 Actualizando $INSTALL_DIR ($BRANCH)..."
        git -C "$INSTALL_DIR" fetch origin "$BRANCH"
        git -C "$INSTALL_DIR" checkout "$BRANCH"
        git -C "$INSTALL_DIR" pull --ff-only origin "$BRANCH"
    fi
else
    mkdir -p "$(dirname "$INSTALL_DIR")"
    echo "⬇️  Clonando $REPO_URL en $INSTALL_DIR..."
    git clone --branch "$BRANCH" "$REPO_URL" "$INSTALL_DIR"
fi

if [ "$SKIP_BUILD" = false ]; then
    echo ""
    echo "🛠️  [3/5] Compilando en perfil Release (puede tardar varios minutos)..."
    (cd "$INSTALL_DIR" && cargo build --release --workspace)
else
    echo ""
    echo "⏭️  [3/5] Compilación omitida (--skip-build)."
fi

echo ""
echo "🚀 [4/5] Instalando binarios, temas, iconos y sesión..."
$SUDO bash "$INSTALL_DIR/scripts/install-session.sh" --skip-build

# ------------------------------------------------------------------------------
# 4. Gestor de inicio de sesión
# ------------------------------------------------------------------------------
echo ""
echo "⚙️  [5/5] Configurando el inicio de sesión..."

detect_dm() {
    local dm unit
    unit="$(readlink -f /etc/systemd/system/display-manager.service 2>/dev/null || true)"
    if [ -n "$unit" ] && [ -e "$unit" ]; then
        dm="$(basename "$unit" .service)"
        case "$dm" in gdm3) dm="gdm" ;; esac
        echo "$dm"; return
    fi
    for dm in gdm gdm3 sddm lightdm lxdm greetd ly cosmic-greeter; do
        if systemctl is-enabled "$dm.service" >/dev/null 2>&1 || systemctl is-active "$dm.service" >/dev/null 2>&1; then
            case "$dm" in gdm3) dm="gdm" ;; esac
            echo "$dm"; return
        fi
    done
}

ACTIVE_DM="$(detect_dm || true)"
SESSION_FILE="/usr/share/wayland-sessions/malus.desktop"

write_tty_autostart() {
    local prof="$TARGET_HOME/.bash_profile"
    touch "$prof"
    if ! grep -q '/usr/local/bin/malus-session' "$prof" 2>/dev/null; then
        cat >>"$prof" <<'EOF'

# Iniciar Malus DE automáticamente en TTY1
if [ -z "$WAYLAND_DISPLAY" ] && [ -z "$DISPLAY" ] && [ "${XDG_VTNR:-0}" -eq 1 ]; then
    /usr/local/bin/malus-session
fi
EOF
        chown "$TARGET_USER:$(id -gn "$TARGET_USER")" "$prof" 2>/dev/null || true
        echo "   ✔ Autoinicio en TTY1 añadido a $prof"
    else
        echo "   ✔ Autoinicio en TTY1 ya estaba configurado en $prof"
    fi
}

install_dm() {
    local dm="$1"
    case "$dm" in
        gdm)     case "$FAMILY" in debian) pkg_install gdm3 ;; *) pkg_install gdm ;; esac ;;
        sddm)    pkg_install sddm ;;
        lightdm) case "$FAMILY" in
                     fedora) pkg_install lightdm lightdm-gtk ;;
                     arch)   pkg_install lightdm lightdm-gtk-greeter ;;
                     debian) pkg_install lightdm lightdm-gtk-greeter ;;
                 esac ;;
        greetd)
            case "$FAMILY" in
                arch) pkg_install greetd greetd-tuigreet ;;
                *)    pkg_install greetd tuigreet ;;
            esac
            local greeter="agreety --cmd /usr/local/bin/malus-session"
            if command -v tuigreet >/dev/null 2>&1; then
                greeter="tuigreet --time --remember --cmd /usr/local/bin/malus-session"
            fi
            $SUDO mkdir -p /etc/greetd
            $SUDO tee /etc/greetd/config.toml >/dev/null <<EOF
[terminal]
vt = 1

[default_session]
command = "$greeter"
user = "greeter"
EOF
            ;;
        *) echo "❌ Gestor no soportado: $dm" >&2; return 1 ;;
    esac
    local unit="$dm"
    [ "$dm" = "gdm" ] && [ "$FAMILY" = "debian" ] && unit="gdm3"
    $SUDO systemctl set-default graphical.target >/dev/null 2>&1 || true
    $SUDO systemctl enable -f "$unit.service"
    ACTIVE_DM="$dm"
    echo "   ✔ $dm instalado y habilitado para el próximo arranque"
}

LAUNCH_NOW=false
if [ -n "$ACTIVE_DM" ]; then
    echo "🔎 Gestor de inicio de sesión detectado: ${ACTIVE_DM^^}"
    if [ -f "$SESSION_FILE" ]; then
        echo "   ✔ Entrada 'Malus DE' registrada en $SESSION_FILE"
    else
        echo "   ⚠ No se encontró $SESSION_FILE (revisa la salida del paso 4)"
    fi
    case "$ACTIVE_DM" in
        greetd)
            if grep -q 'malus-session' /etc/greetd/config.toml 2>/dev/null; then
                echo "   ✔ /etc/greetd/config.toml ya apunta a malus-session"
            else
                echo "   ℹ greetd lanza el comando fijado en /etc/greetd/config.toml."
                echo "     Usa p.ej.: tuigreet --sessions /usr/share/wayland-sessions"
                echo "     o --cmd /usr/local/bin/malus-session"
            fi ;;
        ly)
            echo "   ℹ Ly lee /usr/share/wayland-sessions; elige 'Malus DE' con ← →." ;;
    esac
    [ "$FORCE_TTY_AUTOSTART" = true ] && write_tty_autostart
else
    echo "🖥️  No se detectó ningún gestor de inicio de sesión gráfico (sistema solo-terminal)."
    CHOICE=""
    if [ "$FORCE_TTY_AUTOSTART" = true ]; then
        CHOICE="2"
    elif [ -n "$FORCE_DM" ]; then
        CHOICE="3"
    else
        echo ""
        echo "¿Cómo quieres arrancar Malus DE?"
        echo "  1) Desde la terminal (ejecutar /usr/local/bin/malus-session)"
        echo "  2) Desde la terminal con autoinicio al entrar en TTY1"
        echo "  3) Instalar un gestor de inicio de sesión gráfico"
        ask "Opción [1]: " "1"
        CHOICE="$REPLY"
    fi
    case "$CHOICE" in
        2) write_tty_autostart ;;
        3)
            DM="$FORCE_DM"
            if [ -z "$DM" ]; then
                echo ""
                echo "Gestores disponibles:"
                echo "  1) SDDM     (ligero, gráfico)"
                echo "  2) LightDM  (ligero, gráfico)"
                echo "  3) GDM      (GNOME; arrastra muchas dependencias)"
                echo "  4) greetd + tuigreet (pantalla de login en modo texto, mínimo)"
                ask "Opción [1]: " "1"
                case "$REPLY" in
                    2) DM="lightdm" ;; 3) DM="gdm" ;; 4) DM="greetd" ;; *) DM="sddm" ;;
                esac
            fi
            install_dm "$DM"
            ;;
        *) LAUNCH_NOW=false ;;
    esac
fi

# ------------------------------------------------------------------------------
# Resumen
# ------------------------------------------------------------------------------
echo ""
echo "============================================================"
echo "🎉 ¡MALUS DE INSTALADO!"
echo "============================================================"
if [ -n "$ACTIVE_DM" ]; then
    echo "Cierra sesión (o reinicia) y elige 'Malus DE' en la pantalla de login (${ACTIVE_DM^^})."
    echo "También puedes iniciarlo desde una TTY con: /usr/local/bin/malus-session"
else
    echo "Para iniciar Malus DE desde la terminal (TTY real, no SSH):"
    echo "  /usr/local/bin/malus-session"
fi
echo "Terminal incluida: Ptyxis (ptyxis)"
echo "Registro de sesión: ~/.local/share/malus-de/session.log"
echo "============================================================"
