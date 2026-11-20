# ADR 0010 — Panel de Batería: Telemetría Linux y Adaptaciones Pendientes

**Fecha:** 2026-11-20  
**Referencias visuales:** `assets/screenshots/Bateria00.png`–`Bateria05.png`  
**Módulo afectado:** `crates/settings/src/battery_pane.rs`

---

## 1. Implementado de forma funcional sobre Linux
- **Cabecera con subtítulo dinámico (`Bateria00.png`)**: Muestra `"Nivel de batería: N%"` con icono dinámico cuando se detecta batería en `/sys/class/power_supply/BAT*` o `upower -i DisplayDevice`, o `"Corriente alterna (sin batería física)"` en equipos de escritorio.
- **Ahorrar batería (`Bateria01.png`)**: Menú transparente con `Nunca`, `Siempre`, `Sólo al usar la batería`, `Sólo con el adaptador de corriente`. Persiste en `~/.config/malus-de/battery-settings.toml` y solicita cambio de perfil mediante `tuned-adm profile` (`powersave` / `balanced-battery` / `balanced`).
- **Condición de la batería (`Bateria02.png`)**: Calcula salud (`Normal` / `Se recomienda revisión`) y capacidad máxima relativa (`energy_full / energy_full_design` o `charge_full / charge_full_design`) más conteo de ciclos y capacidad en Wh en `"Obtén detalles..."`.
- **Recarga (`Bateria03.png`)**: Deslizador por pasos (`80%`, `85%`, `90%`, `95%`, `100%`) conectado a `charge_control_end_threshold` vía `pkexec` cuando el controlador del portátil expone el nodo en `sysfs`.
- **Gráficas `Últimas 24 horas` y `Últimos 10 días` (`Bateria00.png`, `Bateria04.png`)**: Renderizado en Cairo con barras verdes de nivel/energía y barras azules de uso con pantalla encendida alimentadas desde `~/.local/share/malus-de/battery-history.csv`.

## 2. Aspectos pendientes de adaptación al 100% en Linux
1. **Daemon continuo de muestreo de pantalla encendida (`Uso con pantalla encendida`)**:
   - Actualmente `battery_pane.rs` registra muestras al abrir o consultar el panel. Para contar con minutos continuos de pantalla activa incluso con Ajustes cerrado, falta un muestreador ligero en `malus-de-compositor` o `top-bar` que escriba en `battery-history.csv` cuando no haya bloqueo/reposo activo.
2. **Recarga optimizada basada en rutina**:
   - Guardada como preferencia (`optimized_charging`) en `battery-settings.toml`; requiere un servicio en segundo plano con permiso sysfs que eleve temporalmente `charge_control_end_threshold` de 80% a 100% antes de la hora habitual de desconexión.
3. **Opciones de energía avanzadas (`Bateria05.png`)**:
   - *Atenuar ligeramente la pantalla al usar la batería*: pendiente conectar con `brightnessctl` / control de retroiluminación de compositor al cambiar evento AC/batería de UPower.
   - *Impedir entrar en reposo automáticamente...*: pendiente lanzar/liberar `systemd-inhibit --what=sleep` cuando esté en corriente alterna con pantalla apagada.
   - *Activar la computadora para permitir el acceso a la red*: pendiente asociar a `ethtool -s <if> wol g` / WoWLAN (`iw phy0 wowlan enable`).
   - *Optimizar transmisión de vídeo al usar la batería (SDR vs HDR)*: preferencia persistida para futura negociación HDR del compositor / pipeline Wayland (`wp-color-management-v1`).
4. **Controladores sin `charge_control_end_threshold` o escritorios sin batería**:
   - Equipos sin dicho nodo en sysfs persisten la configuración sin error; cuando no existe TuneD corriendo en D-Bus, la política queda reflejada en `MalusConfig.power`.
