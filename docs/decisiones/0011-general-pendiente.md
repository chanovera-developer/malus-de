# ADR 0011 — Panel General y Subvistas: Funcionalidades Nativas vs Etiquetas «por implementar»

**Fecha:** 2026-11-20  
**Referencias visuales:** `assets/screenshots/General01.png`–`General12.png`  
**Módulo afectado:** `crates/settings/src/general_pane.rs`

---

## 1. Implementado directamente sobre Linux
- **Barra lateral y Hub `General` (`General01.png`)**:
  - Espaciado de separación de grupo (`14 px`) después de `Batería` seguido por `General`.
  - Tarjeta hero superior con insignia squircle y las 6 agrupaciones de subvistas con navegación completa en el historial `< | >`.
- **Información (`General02.png`)**:
  - Lectura real de modelo DMI (`/sys/devices/virtual/dmi/id/*`), hostname (`/etc/hostname`), CPU (`/proc/cpuinfo`), memoria RAM (`/proc/meminfo`), ID de máquina (`/etc/machine-id`), distribución (`/etc/os-release`), kernel, monitores (`DisplaysConfig`), almacenamiento raíz (`df -B1 /`), diálogo `"Reporte del sistema..."` y saltos directos a `Pantallas` y `Almacenamiento`.
- **Actualización de software (`General03.png`)**:
  - Consulta asíncrona de actualizaciones con `dnf check-update` y preferencia de actualizaciones automáticas.
- **Almacenamiento (`General04.png`)**:
  - Barra segmentada en Cairo con cálculo asíncrono real de `~/Documentos`, `~/Imágenes`, `~/Descargas`, `~/.local/share/Trash`, `~/.local/share/flatpak`, `~/.cache` y espacio disponible del volumen raíz.
- **Compartir (`General07.png`)**:
  - Control de `sshd` (Sesión remota) y `cups` (Compartir impresora) vía `systemctl`, además de edición de hostname local (`pkexec hostnamectl set-hostname`).
- **Disco de arranque (`General08.png`)**:
  - Muestra el volumen raíz activo y permite reiniciar el sistema (`systemctl reboot`).
- **Elementos de inicio (`General09.png` / `General10.png`)**:
  - Gestión real de entradas XDG Autostart (`~/.config/autostart/*.desktop`) con creación `+` y eliminación `-`, más vista de extensiones por app y por categoría.
- **Fecha y hora (`General11.png`)**:
  - Sincronización NTP (`timedatectl set-ntp`), reloj de 24 horas (`org.gnome.desktop.interface` `clock-format`) y zona horaria activa (`/etc/localtime`).
- **Idioma y región (`General12.png`)**:
  - Lista de idiomas preferidos, vista previa interactiva en vivo de fecha/moneda/números/temperatura y selectores regionales persistidos en `~/.config/malus-de/general-settings.toml`.

---

## 2. Elementos marcados con etiqueta «por implementar»
1. **AppleCare y garantía (`general-warranty`)**: No existe equivalente en Linux; previsto integrar con metadatos OEM / `fwupd`.
2. **AirDrop, Handoff, Widgets del teléfono y Receptor de AirPlay (`General05.png`)**: Protocolos propietarios del ecosistema Apple; etiquetados como `por implementar` para futura integración con Malus Link / LocalSend / UxPlay.
3. **App integrada de Contraseñas y Autorrelleno del sistema (`General06.png`)**: El botón `"Abrir Contraseñas"` lanza `seahorse` o `keepassxc` si están instalados; el gestor nativo de Malus DE queda marcado `por implementar`.
4. **Servicios de Compartir adicionales (`General07.png`)**: Compartir pantalla RDP/PipeWire desde Ajustes, Caché de contenido, Administración remota ARD y Scripts de apps remotas.
5. **Extensiones del sistema (`General09.png` / `General10.png`)**: Catálogo visual implementado con pestañas `Por app` y `Por categoría`; el sistema de plugins de compartición/acciones entre apps de Malus DE está etiquetado `por implementar`.
6. **Tratamiento gramatical y Texto en vivo OCR (`General12.png`)**: Preferencias persistidas y etiquetadas `por implementar`.
7. **Time Machine (`general-timemachine`), Admón. de dispositivos MDM (`general-mdm`) y Asistente de Migración (`general-reset`)**: Subvistas con descripción técnica y etiqueta `por implementar`.

