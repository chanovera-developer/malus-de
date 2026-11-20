# ADR 0012 — Panel de Accesibilidad y 20 Subvistas: Integración GSettings/Orca vs Etiquetas «por implementar»

**Fecha:** 2026-11-20  
**Referencias visuales:** `assets/screenshots/Accesibilidad1.png`–`Accesibilidad21.png`  
**Módulo afectado:** `crates/settings/src/accessibility_pane.rs`

---

## 1. Integrado directamente con Linux (`GSettings`, `Orca`, `speech-dispatcher`)
- **VoiceOver (`Accesibilidad2.png`)**: Conecta con `org.gnome.desktop.a11y.applications` `screen-reader-enabled` e inicia/detiene `/usr/bin/orca` (además de abrir `orca -s` desde *"Abrir Utilidad VoiceOver..."*).
- **Zoom (`Accesibilidad3.png`)**: Conecta con `org.gnome.desktop.a11y.applications` `screen-magnifier-enabled`.
- **Pantalla (`Accesibilidad5.png`)**:
  - *Invertir colores*: `org.gnome.desktop.a11y.magnifier` `invert-lightness`.
  - *Aumentar contraste*: `org.gnome.desktop.a11y.interface` `high-contrast`.
  - *Diferenciar sin color*: `org.gnome.desktop.a11y.interface` `show-status-shapes`.
  - *Tamaño del texto*: `org.gnome.desktop.interface` `text-scaling-factor`.
  - *Tamaño del puntero*: `org.gnome.desktop.interface` `cursor-size` (`24..64 px`).
  - *Filtros de colores*: vista previa Cairo de 12 lápices de colores y `org.gnome.desktop.a11y.magnifier` `color-saturation`.
- **Movimiento (`Accesibilidad6.png`)**:
  - *Reducir movimiento*: `org.gnome.desktop.a11y.interface` `reduced-motion`.
  - *Preferir que el cursor no parpadee*: `org.gnome.desktop.interface` `cursor-blink`.
- **Lectura y voz / Lectura en vivo (`Accesibilidad7.png`, `Accesibilidad17.png`)**:
  - Prueba y síntesis de locución en español mediante `/usr/bin/spd-say` (`speech-dispatcher` / `espeak-ng`).
- **Teclado y Control del puntero (`Accesibilidad10.png`, `Accesibilidad11.png`)**:
  - *Tecleado fácil (Sticky Keys)*: `org.gnome.desktop.a11y.keyboard` `stickykeys-enable`.
  - *Teclas lentas (Slow Keys)*: `org.gnome.desktop.a11y.keyboard` `slowkeys-enable`.
  - *Teclado de accesibilidad en pantalla*: `org.gnome.desktop.a11y.applications` `screen-keyboard-enabled`.
  - *Teclas para el mouse*: `org.gnome.desktop.a11y.keyboard` `mousekeys-enable`.
- **Audio y Subtítulos (`Accesibilidad13.png`, `Accesibilidad14.png`)**:
  - *Parpadeo de pantalla al emitir sonidos de alerta*: `org.gnome.desktop.wm.preferences` `visual-bell`.
  - *Subtítulos*: lienzo Cairo interactivo con vista previa en vivo de los 4 estilos (*Fondo transparente*, *Clásico*, *Texto con contorno*, *Texto grande*).
- **Atajo (`Accesibilidad21.png`)**:
  - Lista de verificación de las 20 funciones de accesibilidad persistidas en `~/.config/malus-de/accessibility-settings.toml`.

---

## 2. Elementos marcados con etiqueta «por implementar»
1. **Texto flotante / Color flotante / Escritura flotante (`Accesibilidad4.png`)**: Requiere overlay de lupa de texto en `malus-de-compositor`.
2. **Atenuar luces parpadeantes (`Accesibilidad5.png`)**: Requiere análisis de luminancia temporal en el shader del compositor Wayland.
3. **Descripciones de audio (`Accesibilidad8.png`)** y **Subtítulos en vivo (`Accesibilidad15.png`)**: Requieren pipeline ASR local (Whisper/Vosk sobre PipeWire).
4. **Control por voz (`Accesibilidad9.png`)**, **Control por botón (`Accesibilidad12.png`)**, **Puntero con seguimiento de cabeza (`Accesibilidad11.png`)**: Requieren demonios de entrada asistida sobre `uinput` / `libei`.
5. **Sonidos de fondo y Adaptaciones de audífonos (`Accesibilidad13.png`)**: Requieren generador/filtro DSP integrado en PipeWire.
6. **Reconocimiento de nombres (`Accesibilidad16.png`)**, **Voz personal (`Accesibilidad18.png`)**, **Atajos de voz (`Accesibilidad19.png`)** y **Siri (`Accesibilidad20.png`)**: Funciones de modelos neuronales locales etiquetadas `por implementar`.

