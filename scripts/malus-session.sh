#!/usr/bin/env bash
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ -d "$DIR/target/release" ] && [ -x "$DIR/target/release/malus-de-compositor" ]; then
    BIN_DIR="$DIR/target/release"
else
    BIN_DIR="$DIR/target/debug"
fi

# Aislar configuración de usuario de GNOME/GTK para Malus DE (macOS: botones a la izquierda)
export XDG_CONFIG_HOME="$HOME/.config/malus-de"
export XDG_CURRENT_DESKTOP="Malus:GNOME"
export XDG_SESSION_DESKTOP="malus"
export XDG_SESSION_TYPE="wayland"
export GSETTINGS_BACKEND="keyfile"
export GTK_USE_PORTAL=1
export UBUNTU_MENUPROXY=1
export GTK_MODULES="${GTK_MODULES:+$GTK_MODULES:}appmenu-gtk-module"
mkdir -p "$XDG_CONFIG_HOME/glib-2.0/settings" "$XDG_CONFIG_HOME/gtk-3.0" "$XDG_CONFIG_HOME/gtk-4.0"
if [ ! -f "$XDG_CONFIG_HOME/glib-2.0/settings/keyfile" ]; then
cat << 'EOF' > "$XDG_CONFIG_HOME/glib-2.0/settings/keyfile"
[org/gnome/desktop/wm/preferences]
button-layout='close,minimize,maximize:'

[org/gnome/desktop/interface]
color-scheme='prefer-light'
cursor-theme='macOS'
cursor-size=24

[org/gnome/settings-daemon/plugins/xsettings]
overrides={'Gtk/ShellShowsAppMenu': <1>, 'Gtk/ShellShowsMenubar': <1>}
EOF
fi

CURSOR_THEME="$(gsettings get org.gnome.desktop.interface cursor-theme 2>/dev/null | sed "s/^'//; s/'$//")" || CURSOR_THEME=""
CURSOR_SIZE="$(gsettings get org.gnome.desktop.interface cursor-size 2>/dev/null)" || CURSOR_SIZE=""
[ -n "$CURSOR_THEME" ] || CURSOR_THEME="macOS"
case "$CURSOR_SIZE" in
    ''|*[!0-9]*) CURSOR_SIZE=24 ;;
esac
export XCURSOR_THEME="$CURSOR_THEME"
export XCURSOR_SIZE="$CURSOR_SIZE"

if [ ! -f "$XDG_CONFIG_HOME/gtk-3.0/settings.ini" ]; then
cat << EOF > "$XDG_CONFIG_HOME/gtk-3.0/settings.ini"
[Settings]
gtk-decoration-layout=close,minimize,maximize:
gtk-application-prefer-dark-theme=0
gtk-cursor-theme-name=$CURSOR_THEME
gtk-cursor-theme-size=$CURSOR_SIZE
EOF
fi

if [ ! -f "$XDG_CONFIG_HOME/gtk-4.0/settings.ini" ]; then
cat << EOF > "$XDG_CONFIG_HOME/gtk-4.0/settings.ini"
[Settings]
gtk-decoration-layout=close,minimize,maximize:
gtk-application-prefer-dark-theme=0
gtk-cursor-theme-name=$CURSOR_THEME
gtk-cursor-theme-size=$CURSOR_SIZE
EOF
fi

# Aislar el bus de sesión D-Bus para evitar que apps de instancia única (ej. Nautilus) se redirijan a la sesión GNOME anfitriona
if [ -z "$MALUS_NESTED_DBUS" ] && which dbus-run-session >/dev/null 2>&1; then
    export MALUS_NESTED_DBUS=1
    exec dbus-run-session -- "$0" "$@"
fi

# Sincronizar variables con el daemon D-Bus para que los servicios activados (como portales) hereden la configuración aislada
dbus-update-activation-environment --all 2>/dev/null || true

echo "============================================================"
echo "🍎 INICIANDO SESIÓN COMPLETA DE MALUS DE (MODO ANIDADO WINIT)"
echo "============================================================"

# Limpiar procesos previos si existieran
killall malus-de-compositor malus-de-top-bar malus-de-dock malus-de-spotlight malus-de-wallpaper malus-de-notifications malus-de-link malus-de-session-lock gnome-calculator 2>/dev/null || true

# 1. Iniciar Compositor Wayland Smithay
echo "🚀 [1/5] Iniciando Compositor Wayland..."
"$BIN_DIR/malus-de-compositor" --backend winit &
COMPOSITOR_PID=$!

# Esperar a que el socket Wayland esté listo
sleep 1.2
export WAYLAND_DISPLAY="wayland-1"
export GDK_BACKEND="wayland"
export GSK_RENDERER="cairo"

# Actualizar el entorno D-Bus con el display Wayland de Malus DE
dbus-update-activation-environment WAYLAND_DISPLAY DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE GDK_BACKEND GSK_RENDERER XDG_CONFIG_HOME GSETTINGS_BACKEND GTK_USE_PORTAL 2>/dev/null || true


# 2. Iniciar Demonio de Fondo de Pantalla (Layer Background)
echo "🎨 [2/5] Iniciando Fondo de Pantalla..."
"$BIN_DIR/malus-de-wallpaper" &
WALLPAPER_PID=$!

# 3. Iniciar Top Bar (Layer Top)
echo "🍏 [3/5] Iniciando Top Bar (macOS Menubar & Control Center)..."
"$BIN_DIR/malus-de-top-bar" &
TOPBAR_PID=$!

# 4. Iniciar Dock (Layer Top / Bottom)
echo "🛳  [4/5] Iniciando Floating Dock..."
"$BIN_DIR/malus-de-dock" &
DOCK_PID=$!

# 5. Iniciar Demonio de Notificaciones (Layer Overlay)
echo "🔔 [5/5] Iniciando Servidor de Notificaciones D-Bus..."
"$BIN_DIR/malus-de-notifications" &
NOTIF_PID=$!

# 6. Iniciar Demonio Malus Link & AirDrop (D-Bus org.malus_de.Link)
echo "📡 [6/6] Iniciando Demonio de Enlace y AirDrop (malus-de-link)..."
"$BIN_DIR/malus-de-link" &
LINK_PID=$!

# 7. Iniciar Aplicación de prueba (Calculadora GNOME)
echo "🧮 Lanzando ventana de prueba (gnome-calculator)..."
gnome-calculator &
CALC_PID=$!

echo "============================================================"
echo "✨ ENTORNO DE ESCRITORIO MALUS DE EN EJECUCIÓN"
echo "   Socket activo: WAYLAND_DISPLAY=$WAYLAND_DISPLAY"
echo "   Presiona Ctrl+C en esta terminal para finalizar la sesión."
echo "============================================================"

cleanup() {
    echo ""
    echo "🛑 Cerrando sesión de Malus DE..."
    kill "$CALC_PID" 2>/dev/null || true
    kill "$LINK_PID" 2>/dev/null || true
    kill "$NOTIF_PID" 2>/dev/null || true
    kill "$DOCK_PID" 2>/dev/null || true
    kill "$TOPBAR_PID" 2>/dev/null || true
    kill "$WALLPAPER_PID" 2>/dev/null || true
    kill "$COMPOSITOR_PID" 2>/dev/null || true
    echo "Sesión finalizada limpiamente."
}

trap cleanup EXIT INT TERM

wait "$COMPOSITOR_PID"
