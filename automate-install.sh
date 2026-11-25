#!/usr/bin/env bash

echo "This is the malus-de auto installator, this project stills under develoment so there can be errors."

OS_ID=$(. /etc/os-release && printf '%s' "$ID")
ID_LIKE=$(. /etc/os-release && printf '%s' "$ID_LIKE")

#if [[ "$OS_ID" == "arch" ]]; then
if [[ " $OS_ID $ID_LIKE " == *" arch "* ]]; then
  echo "Arch Linux or arch-based detected"
  sudo pacman -S --needed \
    mesa \
    vulkan-radeon \
    pipewire \
    pipewire-pulse \
    wireplumber \
    networkmanager \
    bluez \
    bluez-utils \
    power-profiles-daemon \
    polkit \
    mate-polkit \
    gnome-keyring \
    libsecret \
    gdm \
    gtk4 \
    libadwaita \
    gtk4-layer-shell \
    xdg-desktop-portal \
    xdg-desktop-portal-gtk \
    xdg-desktop-portal-gnome \
    wl-clipboard \
    libcanberra \
    ffmpeg \
    gvfs \
    udisks2 \
    ptyxis \
    gnome-system-monitor \
    gnome-text-editor \
    loupe \
    inter-font \
    qt5-wayland \
    qt6-wayland \
    xorg-xwayland

elif [[ " $OS_ID $ID_LIKE " == *" debian "* ]]; then
  echo "Debian or debian-based detected, installing via apt"
  sudo apt install -y \
    mesa-va-drivers \
    mesa-vulkan-drivers \
    pipewire \
    pipewire-pulse \
    wireplumber \
    network-manager \
    bluez \
    bluez-tools \
    power-profiles-daemon \
    policykit-1 \
    mate-polkit \
    gnome-keyring \
    libpam-gnome-keyring \
    libsecret-1-0 \
    gdm3 \
    libgtk-4-1 \
    libadwaita-1-0 \
    libgtk4-layer-shell0 \
    xdg-desktop-portal \
    xdg-desktop-portal-gtk \
    xdg-desktop-portal-gnome \
    wl-clipboard \
    libcanberra-gtk3-module \
    ffmpeg \
    gvfs \
    gvfs-backends \
    udisks2 \
    ptyxis \
    gnome-system-monitor \
    gnome-text-editor \
    loupe \
    fonts-inter \
    qtwayland5 \
    qt6-wayland \
    xwayland
fi

cd ~/malus-de/ && echo "Compilando" && cargo build --release --workspace && ./scripts/install-session.sh
