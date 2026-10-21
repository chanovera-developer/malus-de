#!/usr/bin/env bash
# ==============================================================================
# Malus DE - Instalador de Sesión GDM y Binarios del Sistema
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "============================================================"
echo "🍎 INSTALADOR DE SESIÓN MALUS DE PARA GDM Y LINUX"
echo "============================================================"

# Verificar privilegios de superusuario (requerido para /usr/share y /usr/local)
if [ "$(id -u)" -ne 0 ]; then
    echo "🔑 Este script requiere permisos de administrador para instalar archivos en el sistema."
    echo "   Reejecutando con sudo..."
    exec sudo bash "$0" "$@"
fi

# Detectar usuario original no-root
TARGET_USER="${SUDO_USER:-$USER}"
TARGET_HOME=$(getent passwd "$TARGET_USER" | cut -d: -f6)

echo "👤 Usuario detectado: $TARGET_USER (Home: $TARGET_HOME)"
echo "📂 Directorio del proyecto: $PROJECT_DIR"
echo ""

# 1. Compilar binaries en modo Release como el usuario original (no como root)
echo "📦 [1/4] Compilando binarios de Malus DE en perfil Release..."
su - "$TARGET_USER" -c "cd '$PROJECT_DIR' && cargo build --release --workspace"

# 2. Instalar binarios en /usr/local/bin
echo "🚀 [2/4] Instalando binarios ejecutables en /usr/local/bin/..."
mkdir -p /usr/local/bin

BINARIES=(
    "malus-de-compositor"
    "malus-de-top-bar"
    "malus-de-dock"
    "malus-de-spotlight"
    "malus-de-wallpaper"
    "malus-de-notifications"
    "malus-de-screenshot"
    "malus-de-session-lock"
    "malus-de-link"
    "malus-de-ctl"
    "malus-de-settings"
)

for bin in "${BINARIES[@]}"; do
    SRC="$PROJECT_DIR/target/release/$bin"
    if [ -f "$SRC" ]; then
        cp -f "$SRC" "/usr/local/bin/$bin"
        chmod 755 "/usr/local/bin/$bin"
        echo "   ✔ /usr/local/bin/$bin"
    else
        echo "   ⚠ No encontrado: $SRC"
    fi
done

# Instalar script de inicio de sesión
cp -f "$SCRIPT_DIR/malus-session" "/usr/local/bin/malus-session"
chmod 755 "/usr/local/bin/malus-session"
echo "   ✔ /usr/local/bin/malus-session"

# 3. Instalar target de systemd para gestión de sesión gráfica y portales
echo "⚙️ [3/4] Instalando malus-session.target para systemd..."
mkdir -p /usr/lib/systemd/user
cat << 'EOF' > /usr/lib/systemd/user/malus-session.target
[Unit]
Description=Malus DE Session
BindsTo=graphical-session.target
Wants=graphical-session.target
After=graphical-session-pre.target
EOF
chmod 644 /usr/lib/systemd/user/malus-session.target
echo "   ✔ /usr/lib/systemd/user/malus-session.target"

# Instalar preferencia de backend para xdg-desktop-portal
echo "🗂️ Instalando preferencia de portal para Malus DE..."
mkdir -p /usr/share/xdg-desktop-portal
cp -f "$PROJECT_DIR/data/malus-portals.conf" /usr/share/xdg-desktop-portal/malus-portals.conf
chmod 644 /usr/share/xdg-desktop-portal/malus-portals.conf
echo "   ✔ /usr/share/xdg-desktop-portal/malus-portals.conf"

# 4. Instalar entrada de sesión Wayland para GDM
echo "🖥️ [4/4] Instalando archivo de sesión para GDM en /usr/share/wayland-sessions/..."
mkdir -p /usr/share/wayland-sessions

cp -f "$SCRIPT_DIR/malus.desktop" "/usr/share/wayland-sessions/malus.desktop"
chmod 644 "/usr/share/wayland-sessions/malus.desktop"
echo "   ✔ /usr/share/wayland-sessions/malus.desktop"

echo ""
echo "============================================================"
echo "🎉 ¡INSTALACIÓN COMPLETADA CON ÉXITO!"
echo "============================================================"
echo "Para acceder a Malus DE:"
echo "1. Cierra tu sesión actual de GNOME (Cerrar sesión / Log Out)."
echo "2. En la pantalla de bienvenida de GDM, selecciona tu usuario."
echo "3. Pulsa el icono de engranaje (⚙️) en la esquina inferior derecha."
echo "4. Elige 'Malus DE' en la lista de entornos."
echo "5. Introduce tu contraseña para iniciar la experiencia completa de Malus DE."
echo "============================================================"
