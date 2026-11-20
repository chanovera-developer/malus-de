# ADR 0008 — Panel de Red y Firewall: Estado y Adaptaciones Pendientes para Linux

**Fecha:** 2026-11-19  
**Referencias visuales:** `assets/screenshots/Red00.png`, `assets/screenshots/Red01.png`, `assets/screenshots/Red02.png`, `assets/screenshots/Red03.png`  
**Módulo afectado:** `crates/settings/src/network_pane.rs`

---

## 1. Implementado de forma funcional sobre Linux
- **Vista principal `Red` (`Red00.png`)**:
  - Tarjetas independientes con squircle para **Wi-Fi** (estado vivo vía `nmcli`, navegación al panel Wi-Fi con soporte de historial `< | >`) y **Firewall** (estado vivo vía `firewall-cmd --state`, navegación a la subvista `Firewall`).
  - Sección **Otros servicios** con interfaces y perfiles no-Wi-Fi de NetworkManager (`ethernet`, `bridge`, `vlan`, `puente Thunderbolt`, `gsm`) más diálogo de detalles e interruptor de activación (`nmcli connection up/down`).
  - Menú contextual `••• ⌄` con creación de servicios NetworkManager (`bridge`, `ethernet`, `vlan`) y reordenamiento de prioridades IPv4 (`ipv4.route-metric`).
- **Subvista `Firewall` (`Red01.png`)**:
  - Interruptor principal conectado a `systemctl enable/disable --now firewalld` mediante `pkexec`.
  - Banner informativo/advertencia dinámico cuando el bloqueo total (`block`/`drop`) está activo.
- **Modal `Opciones de Firewall` (`Red02.png` / `Red03.png`)**:
  - Interruptor **"Bloquear todas las conexiones entrantes"** mapeado a zona `block` vs zona de trabajo previa (`public` / `FedoraWorkstation`).
  - Tabla de servicios/puertos permitidos de `firewalld` (`--list-services`, `--list-ports`) con botones `[+ | -]` y traducción automática de demonios comunes (`cupsd` -> `ipp`, `smbd` -> `samba`, `sshd` -> `ssh`).
  - Interruptor **"Activar modo encubierto"** mapeado a `firewall-cmd --add-icmp-block=echo-request` y `sysctl net.ipv4.icmp_echo_ignore_all`.

---

## 2. Aspectos pendientes de adaptación al 100% en Linux

1. **Filtrado de conexiones entrantes por binario firmado (`Red02.png`)**:
   - **Brecha:** Las opciones *"Permitir automáticamente que el software integrado reciba conexiones entrantes"* y *"Permitir automáticamente que el software firmado descargado reciba conexiones entrantes"* dependen en macOS del Application Firewall (ALF) a nivel socket + firma criptográfica Mach-O/CA. Linux (`firewalld`/`nftables`) filtra por zona, servicio y puerto de red, no por firma de binario.
   - **Estado actual:** Ambas preferencias se muestran fielmente en la UI y se persisten en `~/.config/malus-de/firewall-settings.toml`.
   - **Siguiente paso:** Integrar un servicio auxiliar eBPF/LSM o puente D-Bus compatible con OpenSnitch para asociar sockets entrantes a rutas de ejecutables ELF y firmas de paquetes RPM/Flatpak.

2. **Puente Thunderbolt automático y servicios virtuales USB4 (`Red00.png`)**:
   - **Brecha:** En macOS `"Puente Thunderbolt"` aparece como interfaz virtual dedicada gestionada por Apple Thunderbolt Networking. En Linux aparece solamente cuando el módulo `thunderbolt_net` expone una interfaz `thunderbolt0` activa o existe un perfil `bridge` en NetworkManager.
   - **Siguiente paso:** Detectar módulos `sys/bus/thunderbolt/devices` / interfaces `thunderbolt*` en `udev` para crear automáticamente el perfil de enlace P2P Thunderbolt IP cuando no exista.

3. **Ubicaciones de red (`Ubicaciones...`)**:
   - **Brecha:** El selector de ubicaciones guarda el perfil activo en `~/.config/malus-de/network-locations.toml`, pero aún no aplica snapshots completos de DNS estático, proxy SOCKS/HTTP ni rutas personalizadas al cambiar de ubicación.
   - **Siguiente paso:** Asociar cada ubicación con un conjunto de overlays NetworkManager (`ipv4.dns`, `proxy.*`) o trigger dispatcher al cambiar de ubicación o SSID.

4. **Orden absoluto de servicios multi-gateway (`Establecer orden de servicios...`)**:
   - **Estado actual:** Modifica `ipv4.route-metric` por perfil en NetworkManager. Si una conexión no tiene perfil persistente o usa métrica asignada por DHCPv6, requiere ajustar además `ipv6.route-metric` y reactivar la interfaz para aplicar el cambio en vivo.

