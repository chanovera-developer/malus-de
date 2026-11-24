# ADR 0013 — Indicador circular de descarga en el Dock para navegadores Chromium/Brave (Pendiente)

**Fecha:** 2026-11-24  
**Módulos afectados:** `crates/dock/src/progress.rs`, `crates/dock/libunity-shim/malus_libunity.c`, `crates/dock/src/main.rs`

---

## 1. Estado actual implementado y verificado
- **Capa visual Liquid Glass en iconos del Dock** (`crates/dock/src/progress.rs`):
  - Anillo circular con relleno sectorial horario proporcional (`0% → 100%`) y flecha `↓` en el color de acento; check verde `✓` con *fade-out* de `660 ms` al completar; modo indeterminado rotatorio cuando el tamaño total es desconocido.
- **Firefox / LibreWolf / Zen**:
  - Funciona el relleno por porcentaje real (`0% → 100%`) leyendo `totalBytes` desde `downloads.json` del perfil junto al tamaño en vivo del archivo `.part` en `~/Descargas` / `~/Downloads`.
- **Notificación nativa al completar**:
  - Al finalizar una descarga en disco, el Dock emite una notificación `org.freedesktop.Notifications.Notify` con icono y nombre del navegador, título *"Descarga completada"* y nombre/tamaño del archivo, sin requerir extensiones.
- **Suscripción D-Bus `com.canonical.Unity.LauncherEntry` (`Update`)**:
  - Activa en `malus-de-dock` con limpieza automática por `NameOwnerChanged`.
- **Puente experimental `libunity.so.9`** (`crates/dock/libunity-shim/malus_libunity.c`):
  - Implementa los 7 símbolos `unity_inspector_get_default`, `unity_inspector_get_unity_running` y `unity_launcher_entry_*` sobre `GObject` + `gio-2.0`.

---

## 2. Pendiente: porcentaje en vivo para Brave / Chromium
Aunque Firefox muestra el nivel de descarga correctamente, **Brave Origin** (y potencialmente otros navegadores basados en Chromium) sigue sin mostrar el porcentaje de avance en el círculo del Dock.

### Causas probables a investigar al retomar
1. **Validación de `XDG_CURRENT_DESKTOP` dentro de Chromium (`unity_service.cc`)**:
   - Confirmar si la compilación de Brave Origin restringe `dlopen("libunity.so.9")` a entornos concretos (p. ej. `UNITY`/`UBUNTU`) y omite cargarla bajo `XDG_CURRENT_DESKTOP="Malus:GNOME"`.
   - Comprobar en ejecución con `grep libunity /proc/$(pgrep -f brave-origin | head -n1)/maps` y `dbus-monitor "interface='com.canonical.Unity.LauncherEntry'"`.
2. **Retraso/bloqueo en `History` SQLite de Chromium**:
   - Brave abre `History` en modo exclusivo (`History-journal`) y confirma transacciones periódicamente (~10 s), por lo que el lector SQLite en `query_chromium_download_info` puede no ver la fila `state = 0` durante descargas cortas o mientras la transacción sigue en memoria.
3. **Alternativas a evaluar**:
   - Verificar qué condición exacta exige `chrome/browser/ui/views/unity_service.cc` en Brave para activar `libunity.so.9` (o si requiere que exista el propietario D-Bus `com.canonical.Unity`).
   - Evaluar monitoreo del archivo `History-journal` / `current_path` o un puente ligero opcional si Chromium bloquea el acceso a `total_bytes` antes del commit.
