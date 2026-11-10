# Reglas y Pautas para Agentes de IA en Malus DE

## 1. Renderizado en Compositor y Damage Tracking (Backend DRM / KMS)

> [!WARNING]
> **NO intentes habilitar Damage Tracking parcial (`glScissor`) de forma directa en el pase de renderizado inmediato (`crates/compositor/src/backend/drm.rs`).**

* **Causa del fallo:**
  El compositor utiliza efectos visuales avanzados dependientes del framebuffer:
  1. **Dual-Kawase Blur** (`GaussianBlur::render_blur`): captura el fondo con `glCopyTexSubImage2D`. Si el daño es parcial o se restringe a cambios de geometría, los fotogramas limpian el fondo (`frame.clear`) pero no recalculan el blur de ventanas estáticas adyacentes (p. ej., cuando se reproduce un video en otra ventana), haciendo que el desenfoque desaparezca o genere un bucle de retroalimentación oscuro.
  2. **Triple-buffering DRM (KMS/GBM):** La GPU rota 3 búferes físicos (`Buffer 0`, `Buffer 1`, `Buffer 2`). El daño parcial desincroniza las 3 ranuras, causando parpadeo severo a 20 Hz (*flickering* detectable en pantalla y cámara externa).
  3. **Sombras volumétricas SDF ($n=4.2$) y Capas Layer Shell (Dock y Top Bar):** Quedan recortadas o desaparecen si no se encuentran en la lista de daño del fotograma en rotación.

* **Hoja de ruta obligatoria para Damage Tracking:**
  Consulta la decisión arquitectónica en [`docs/decisiones/0002-arquitectura-damage-tracking-y-desenfoque-gpu.md`](docs/decisiones/0002-arquitectura-damage-tracking-y-desenfoque-gpu.md).
  Para implementar Damage Tracking sin errores, se debe diseñar una **Caché de Render Offscreen (FBO)** por superficie (como hace Mutter o KWin) o implementar **Idle Sleeping** (omitir el pase únicamente si el daño total es 0, pero dibujando completo cuando haya actividad).

## 2. Gestión de Commits y Control de Versiones

* **Regla estricta:** NUNCA ejecutes `git commit` sin autorización explícita previa del usuario en el turno actual.
* **Instalación de binarios del sistema:** El binario del compositor en producción se encuentra en `/usr/local/bin/malus-de-compositor`. La copia requiere elevación de permisos fuera del sandbox (`sudo cp malus-de/target/release/malus-de-compositor /usr/local/bin/malus-de-compositor`).
