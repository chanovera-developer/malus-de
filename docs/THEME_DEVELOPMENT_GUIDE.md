# Guía de Desarrollo de Temas para Malus DE
*Arquitectura, directrices y especificación para el diseño y personalización de temas (Light / Dark / Accent)*

---

## 1. Visión General de la Arquitectura de Temas

Malus DE adopta una arquitectura de tematización modular inspirada en macOS (Aqua/Vibrant), dividida en tres capas principales:

```
┌────────────────────────────────────────────────────────────────┐
│                   Entorno Malus DE Shell                       │
│  (Top Bar, Centro de Control, Dock, Spotlight, Notificaciones) │
├────────────────────────────────────────────────────────────────┤
│                    Portal de Configuración                     │
│  (GSettings / D-Bus / keyfile / GTK settings.ini / Libadwaita) │
├────────────────────────────────────────────────────────────────┤
│                 Aplicaciones de Terceros                       │
│  (GTK3, GTK4, Libadwaita, Qt6 / Kvantum, Clientes XWayland)   │
└────────────────────────────────────────────────────────────────┘
```

---

## 2. Ubicación de Temas y Estructura de Directorios

Los temas de usuario se instalan en:
- `~/.config/malus-de/themes/<nombre_del_tema>/` (preferente para usuario)
- `/usr/share/malus-de/themes/<nombre_del_tema>/` (instalaciones a nivel de sistema)

### Estructura canónica de un tema:

```
~/.config/malus-de/themes/mi-tema-macos/
├── theme.toml                  # Metadatos y definición de variables del tema
├── shell.css                   # Estilos visuales del entorno shell (Top Bar, Dock, Popovers)
├── gtk.css                     # Reglas de estilo para aplicaciones GTK3 y GTK4
├── preview.png                 # Miniatura para el selector en Ajustes del Sistema
└── icons/                      # Iconos específicos del tema
    ├── apple-logo.svg          # Logotipo del menú de aplicaciones (esquina superior izquierda)
    └── control-center.svg      # Icono del Centro de Control
```

---

## 3. Especificación del Archivo `theme.toml`

Cada tema debe contener un archivo de configuración declarativo `theme.toml`:

```toml
[theme]
name = "macOS Sequoia Glass"
version = "1.0.0"
author = "Malus DE Design Team"
description = "Tema translúcido acrílico con acentos azules y top-bar transparente"
color_scheme = "adaptive"       # "adaptive", "dark", "light"

[palette.dark]
accent = "#007aff"
background_acrylic = "rgba(26, 26, 32, 0.95)"
card_background = "rgba(255, 255, 255, 0.06)"
card_border = "rgba(255, 255, 255, 0.12)"
text_primary = "#ffffff"
text_secondary = "#a1a1aa"
topbar_background = "transparent"
topbar_text = "#ffffff"

[palette.light]
accent = "#007aff"
background_acrylic = "rgba(245, 245, 247, 0.96)"
card_background = "rgba(0, 0, 0, 0.05)"
card_border = "rgba(0, 0, 0, 0.08)"
text_primary = "#1d1d1f"
text_secondary = "rgba(0, 0, 0, 0.55)"
topbar_background = "transparent"
topbar_text = "#ffffff"

[geometry]
topbar_height = 36              # Altura estándar recomendada
topbar_padding_x = 10
dock_corner_radius = 18
popover_corner_radius = 16
card_corner_radius = 12
```

---

## 4. Estándar de Diseño de la Barra Superior (Top Bar)

### Regla Fundamental:
> **La barra superior de Malus DE tiene diseño base transparente con texto blanco.**
> Esto permite que el fondo de pantalla se aprecie hasta el borde del panel físico de cristal, emulando la estética de macOS.

### Dimensiones y Alineación Óptica:
- **Altura recomendada:** `36px` (mínimo `32px`, máximo `40px`).
- **Alineación Vertical:** Todos los elementos (`root_box`, `left_box`, `right_box`, menús e icono de la manzana) deben mantener `valign: center`.
- **Icono del Logo:** SVG vectorial de 16x16 píxeles con compensación óptica `margin-top: -1px` para alinearse perfectamente con la línea base (*cap-height*) de las fuentes tipográficas.
- **Sombra Tipográfica:** Para garantizar 100% de legibilidad incluso sobre fondos de pantalla claros o blancos, todo el texto de la barra debe incluir sombra de contraste:
  ```css
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.40);
  ```

---

## 5. Clases CSS Principales del Entorno Shell

### Top Bar y Menús Globales
| Selector CSS | Función |
| :--- | :--- |
| `window.malus-topbar` | Ventana raíz anclada a Layer Shell Top |
| `.topbar-content` | Contenedor horizontal con altura mínima (`36px`) |
| `button.topbar-btn` | Botones de menús y reloj interactivos |
| `.apple-logo-btn` | Botón del logotipo de Apple en la esquina superior izquierda |
| `.apple-icon` | Imagen SVG del icono de la manzana |
| `.active-app-name` | Título en negrita de la aplicación en foco |
| `.global-menu-item` | Opciones de menú ("Archivo", "Edición", "Ver", etc.) |

### Popovers y Menús Desplegables
| Selector CSS | Función |
| :--- | :--- |
| `popover.malus-popover-menu > contents` | Recuadro acrílico del menú de la manzana y menús globales |
| `popover.control-center-popover > contents` | Recuadro del Centro de Control |
| `popover.light-popover > contents` | Modo Claro para popovers (fondo blanco lechoso) |
| `.popover-menu-item` | Fila interactiva de opción dentro del menú |
| `.popover-menu-item:hover` | Estado hover (azul macOS `#007aff` con texto blanco) |
| `.menu-shortcut-hint` | Indicador atajo de teclado (ej. `⌘N`, `⌥⌘H`) |

### Módulos del Centro de Control
| Selector CSS | Función |
| :--- | :--- |
| `.cc-tile` | Tarjetas cuadradas interactivas (Wi-Fi, Bluetooth, Modo Oscuro) |
| `.cc-tile-active` | Estado activado (destacado en azul macOS) |
| `.cc-icon-circle` | Círculo contenedor del icono del módulo |
| `.cc-slider-card` | Tarjeta contenedora de sliders de Brillo y Sonido |

---

## 6. Modo Claro / Oscuro Aislado de GNOME

Al alternar entre Modo Claro y Modo Oscuro desde Malus DE, los cambios se limitan a su sesión:

1. **GSettings con backend `keyfile`:** el valor `org.gnome.desktop.interface color-scheme` se guarda en `~/.config/malus-de/glib-2.0/settings/keyfile`, no en el almacén DConf de GNOME.
2. **Archivos GTK3 y GTK4 de Malus DE:**
   - `~/.config/malus-de/gtk-3.0/settings.ini`
   - `~/.config/malus-de/gtk-4.0/settings.ini`
   - Sus hojas CSS se guardan en esos mismos directorios.
3. **Libadwaita en caliente (`AdwStyleManager`):**
   ```rust
   libadwaita::StyleManager::default().set_color_scheme(
       if is_dark { libadwaita::ColorScheme::ForceDark } else { libadwaita::ColorScheme::ForceLight }
   );
   ```
4. **Bus IPC del Compositor:**
   Transmisión de `IpcMessage::ThemeChanged { theme_name, is_dark }` a todos los demonios en ejecución (Dock, Settings, Spotlight).

Malus DE no escribe en `~/.config/gtk-3.0`, `~/.config/gtk-4.0` ni cambia el valor DConf de GNOME. Las preferencias y transparencias GTK se aplican mediante `XDG_CONFIG_HOME=~/.config/malus-de` en la sesión de Malus.

La transparencia de ventanas se limita al estilo `window.view` de Nautilus y a la ventana `.macos-settings-window`, cuyo CSS se carga desde la propia aplicación de Ajustes. El resto de ventanas mantiene el fondo opaco. En Nautilus, los menús y popovers conservan un fondo opaco y no añaden sombra CSS; el desenfoque del compositor se limita al fondo de la ventana.

---

## 7. Flujo de Trabajo para Crear un Tema Nuevo

1. **Crear la carpeta:**
   ```bash
   mkdir -p ~/.config/malus-de/themes/mi-tema
   ```
2. **Copiar la plantilla base:**
   Copia `theme.toml` y adapta las variables de color deseadas.
3. **Personalizar `shell.css`:**
   Sobrescribe las clases que desees modificar (por ejemplo, bordes más redondeados o un color de acento morado/naranja).
4. **Probar en vivo:**
   Desde la aplicación de Ajustes del Sistema (`malus-de-settings` -> *Apariencia y Dock*) o desde el Centro de Control, selecciona tu tema para verificar la respuesta inmediata.
