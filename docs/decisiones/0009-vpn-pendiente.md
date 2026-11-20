# ADR 0009 — Panel de VPN: Compatibilidad de Protocolos y Adaptaciones Linux

**Fecha:** 2026-11-19  
**Referencia visual:** `assets/screenshots/VPN.png`  
**Módulo afectado:** `crates/settings/src/vpn_pane.rs`

---

## 1. Implementado directamente sobre Linux (`NetworkManager`)
- **Vista `VPN` (`VPN.png`)**:
  - Tarjeta superior squircle con estado vacío `"Sin configuraciones de VPN"` o lista de perfiles `vpn` / `wireguard` de NetworkManager (`nmcli connection show`) con interruptor `.macos-switch` para conectar/desconectar y botón `(i)` de detalles/eliminación.
  - Botón inferior izquierdo **"Acerca de las redes VPN y la privacidad..."** con diálogo modal informativo.
  - Menú inferior derecho **`+ ⌄`** transparente (estilo unificado del compositor) con las 3 entradas de referencia (`L2TP sobre IPSec...`, `Cisco IPSec...`, `IKEv2...`) además de importación directa de archivos `.ovpn` (OpenVPN) y `.conf` (WireGuard) vía `nmcli connection import`.
  - Botón **`?`** circular centrado con popover de ayuda.

## 2. Adaptaciones pendientes / dependencias de plugins en Fedora
1. **L2TP sobre IPSec (`l2tp`) y Cisco IPSec (`vpnc`)**:
   - **Estado del sistema:** El sistema base trae instalados `NetworkManager-libreswan` (**IKEv2**), `NetworkManager-openvpn` (**OpenVPN**), `NetworkManager-openconnect` y soporte kernel para **WireGuard**, pero no `NetworkManager-l2tp` ni `NetworkManager-vpnc`.
   - **Comportamiento actual:** La UI detecta dinámicamente los descriptores `/usr/lib*/NetworkManager/VPN/nm-*-service.name`. Para **IKEv2**, **OpenVPN** y **WireGuard** la creación/importación funciona de extremo a extremo; para **L2TP sobre IPSec** y **Cisco IPSec**, si falta el plugin, el diálogo informa qué paquete (`NetworkManager-l2tp` / `NetworkManager-vpnc`) requiere antes de activar el botón **Crear**.
2. **VPN bajo demanda / App VPN de terceros (Apple NEVPNManager)**:
   - macOS permite extensiones de proveedor VPN de App Store con reglas `OnDemand`. En Linux el equivalente actual queda mapeado a `connection.autoconnect` en NetworkManager y perfiles `NetworkManager-*`.

