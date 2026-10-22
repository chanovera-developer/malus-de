use anyhow::Result;
use clap::Parser;
use gio::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, Fixed, Image, Label,
    Orientation, Overlay, Picture,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use malus_de_common::{config::DisplaysConfig, logging::init_logging};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(name = "malus-de-wallpaper")]
#[command(about = "Demonio de Fondo de Pantalla de Malus DE (Layer Shell Background)")]
struct Args {
    /// Ruta a la imagen de fondo de pantalla
    #[arg(short, long)]
    image: Option<PathBuf>,
}

const DEFAULT_GRADIENT_CSS: &str = r#"
window.malus-wallpaper-window,
window.malus-wallpaper-window.background,
window.malus-wallpaper-window decoration {
    margin: 0;
    padding: 0;
    border: none;
    box-shadow: none;
    background: radial-gradient(circle at 35% 25%, #2e1065 0%, #172554 35%, #0f172a 70%, #0d1526 100%);
}

window.malus-wallpaper-window picture {
    margin: 0;
    padding: 0;
    border: none;
}

.desktop-icon {
    min-width: 88px;
    min-height: 88px;
    padding: 6px;
    color: white;
    background: transparent;
    border: none;
    box-shadow: none;
    border-radius: 8px;
}

.desktop-icon:hover {
    background: alpha(white, 0.14);
}

label.desktop-icon-label {
    color: white;
    text-shadow: 0 1px 4px black;
}
"#;

const DESKTOP_ICON_WIDTH: i32 = 112;
const DESKTOP_ICON_HEIGHT: i32 = 112;
const DESKTOP_ICON_SIZE: i32 = 64;

fn icon_positions_path() -> Option<PathBuf> {
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config_dir.join("malus-de/desktop-icons.json"))
}

fn load_icon_positions() -> HashMap<PathBuf, (f64, f64)> {
    let Some(path) = icon_positions_path() else {
        warn!("No se pudo determinar dónde guardar las posiciones de los iconos.");
        return HashMap::new();
    };

    match fs::read(&path) {
        Ok(contents) => match serde_json::from_slice(&contents) {
            Ok(positions) => positions,
            Err(error) => {
                warn!(
                    "No se pudieron leer las posiciones de iconos {:?}: {}",
                    path, error
                );
                HashMap::new()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(error) => {
            warn!(
                "No se pudieron leer las posiciones de iconos {:?}: {}",
                path, error
            );
            HashMap::new()
        }
    }
}

fn save_icon_positions(positions: &HashMap<PathBuf, (f64, f64)>) {
    let Some(path) = icon_positions_path() else {
        warn!("No se pudo determinar dónde guardar las posiciones de los iconos.");
        return;
    };
    let Some(parent) = path.parent() else {
        warn!(
            "La ruta de posiciones de iconos no tiene carpeta padre: {:?}",
            path
        );
        return;
    };

    let result = (|| -> std::io::Result<()> {
        fs::create_dir_all(parent)?;
        let contents = serde_json::to_vec(positions).map_err(std::io::Error::other)?;
        let temporary_path = path.with_extension("json.tmp");
        fs::write(&temporary_path, contents)?;
        fs::rename(&temporary_path, &path)
    })();
    if let Err(error) = result {
        warn!(
            "No se pudieron guardar las posiciones de iconos {:?}: {}",
            path, error
        );
    }
}

fn move_desktop_icon(fixed: &Fixed, path: &Path, position: (f64, f64)) -> bool {
    let expected_path = path.to_string_lossy();
    let mut child = fixed.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        let item_path = item.property::<Option<String>>("tooltip-text");
        if item_path.as_deref() == Some(expected_path.as_ref()) {
            fixed.move_(&item, position.0, position.1);
            return true;
        }
    }
    false
}

fn clamp_icon_position(
    x: f64,
    y: f64,
    pointer_offset: (f64, f64),
    desktop_size: (i32, i32),
) -> (f64, f64) {
    (
        (x - pointer_offset.0).clamp(0.0, f64::from((desktop_size.0 - DESKTOP_ICON_WIDTH).max(0))),
        (y - pointer_offset.1).clamp(
            0.0,
            f64::from((desktop_size.1 - DESKTOP_ICON_HEIGHT).max(0)),
        ),
    )
}

fn dragged_icon_position(
    original_position: (f64, f64),
    pointer_offset: (f64, f64),
    drag_offset: (f64, f64),
    desktop_size: (i32, i32),
) -> (f64, f64) {
    clamp_icon_position(
        original_position.0 + pointer_offset.0 + drag_offset.0,
        original_position.1 + pointer_offset.1 + drag_offset.1,
        pointer_offset,
        desktop_size,
    )
}

fn resolve_desktop_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let user_dirs_file = config_home.join("user-dirs.dirs");

    let contents = match fs::read_to_string(&user_dirs_file) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            warn!("No se pudo leer {:?}: {}", user_dirs_file, error);
            String::new()
        }
    };

    Some(desktop_dir_from_config(&contents, &home))
}

fn desktop_dir_from_config(contents: &str, home: &Path) -> PathBuf {
    contents
        .lines()
        .find_map(|line| {
            let value = line.trim().strip_prefix("XDG_DESKTOP_DIR=")?;
            Some(path_from_user_dir_value(value, home))
        })
        .unwrap_or_else(|| home.join("Escritorio"))
}

fn path_from_user_dir_value(value: &str, home: &Path) -> PathBuf {
    let value = value.trim();
    let value = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value);

    if value == "$HOME" || value == "${HOME}" {
        return home.to_path_buf();
    }
    if let Some(relative) = value
        .strip_prefix("$HOME/")
        .or_else(|| value.strip_prefix("${HOME}/"))
    {
        return home.join(relative);
    }

    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        home.join(path)
    }
}

fn refresh_desktop_icons(
    fixed: &Fixed,
    desktop_dir: &Path,
    desktop_size: (i32, i32),
    icon_positions: &Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    internal_drag_paths: &Rc<RefCell<HashSet<PathBuf>>>,
    active_icon_drag: &Rc<RefCell<Option<(PathBuf, (f64, f64), (f64, f64))>>>,
) {
    while let Some(child) = fixed.first_child() {
        fixed.remove(&child);
    }

    let entries = match fs::read_dir(desktop_dir) {
        Ok(entries) => entries,
        Err(error) => {
            warn!(
                "No se pudo leer la carpeta de escritorio {:?}: {}",
                desktop_dir, error
            );
            return;
        }
    };

    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry.path()),
            Err(error) => {
                warn!("No se pudo leer una entrada del escritorio: {}", error);
                None
            }
        })
        .collect();
    paths.sort_by_key(|path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    });

    let mut positions = icon_positions.borrow_mut();
    positions.retain(|path, _| paths.contains(path));
    let rows = ((desktop_size.1 - 40).max(1) / (DESKTOP_ICON_HEIGHT + 8)).max(1) as usize;

    for (index, path) in paths.into_iter().enumerate() {
        let file = gio::File::for_path(&path);
        let file_info = file.query_info(
            "standard::display-name,standard::icon,standard::content-type",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        );
        let (display_name, icon, content_type) = match file_info {
            Ok(info) => {
                let display_name = info.display_name().to_string();
                let icon = info.icon();
                let content_type = info.content_type().map(|value| value.to_string());
                (display_name, icon, content_type)
            }
            Err(error) => {
                warn!("No se pudo obtener información de {:?}: {}", path, error);
                continue;
            }
        };

        let thumbnail = content_type
            .as_deref()
            .filter(|content_type| content_type.starts_with("image/"))
            .and_then(|_| {
                gdk_pixbuf::Pixbuf::from_file_at_scale(
                    &path,
                    DESKTOP_ICON_SIZE,
                    DESKTOP_ICON_SIZE,
                    true,
                )
                .map_err(|error| {
                    warn!(
                        "No se pudo generar una miniatura para {:?}: {}",
                        path, error
                    );
                    error
                })
                .ok()
            });
        let icon_image = thumbnail
            .as_ref()
            .map(|pixbuf| {
                let texture = gtk4::gdk::Texture::for_pixbuf(pixbuf);
                Image::from_paintable(Some(&texture))
            })
            .unwrap_or_else(|| match icon {
                Some(icon) => Image::from_gicon(&icon),
                None => Image::from_icon_name("text-x-generic"),
            });
        icon_image.set_pixel_size(DESKTOP_ICON_SIZE);

        let label = Label::builder()
            .label(&display_name)
            .wrap(true)
            .max_width_chars(14)
            .justify(gtk4::Justification::Center)
            .css_classes(["desktop-icon-label"])
            .build();
        let item_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .halign(Align::Center)
            .width_request(DESKTOP_ICON_WIDTH)
            .height_request(DESKTOP_ICON_HEIGHT)
            .focusable(true)
            .css_classes(["desktop-icon"])
            .tooltip_text(path.to_string_lossy())
            .build();
        item_box.append(&icon_image);
        item_box.append(&label);

        let (x, y) = *positions.entry(path.clone()).or_insert_with(|| {
            let column = index / rows;
            let row = index % rows;
            (
                (desktop_size.0
                    - DESKTOP_ICON_WIDTH
                    - 16
                    - column as i32 * (DESKTOP_ICON_WIDTH + 8))
                    .max(8) as f64,
                (20 + row as i32 * (DESKTOP_ICON_HEIGHT + 8)) as f64,
            )
        });
        fixed.put(&item_box, x, y);

        let pointer_offset = Rc::new(RefCell::new((0.0, 0.0)));
        let drag_origin = Rc::new(RefCell::new((x, y)));
        let click = gtk4::GestureClick::new();
        click.set_button(1);
        let pointer_offset_press = pointer_offset.clone();
        let drag_origin_press = drag_origin.clone();
        let drag_item_press = item_box.clone();
        let drag_fixed_press = fixed.clone();
        let open_file = file.clone();
        click.connect_pressed(move |_, presses, pointer_x, pointer_y| {
            *pointer_offset_press.borrow_mut() = (pointer_x, pointer_y);
            *drag_origin_press.borrow_mut() = drag_fixed_press.child_position(&drag_item_press);
            if presses == 2 {
                launch_file(&open_file);
            }
        });
        item_box.add_controller(click);

        let context_click = gtk4::GestureClick::new();
        context_click.set_button(3);
        let context_file = file.clone();
        let context_item = item_box.clone();
        context_click.connect_pressed(move |_, _, _, _| {
            let popover = gtk4::Popover::new();
            popover.set_autohide(true);
            popover.set_parent(&context_item);

            let menu = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(4)
                .margin_top(6)
                .margin_bottom(6)
                .margin_start(6)
                .margin_end(6)
                .build();
            let open_button = Button::with_label("Abrir");
            let open_file = context_file.clone();
            open_button.connect_clicked(move |_| launch_file(&open_file));
            menu.append(&open_button);

            let reveal_button = Button::with_label("Mostrar en el gestor de archivos");
            let reveal_file = context_file.clone();
            reveal_button.connect_clicked(move |_| {
                let parent = reveal_file.parent().unwrap_or_else(|| reveal_file.clone());
                launch_file(&parent);
            });
            menu.append(&reveal_button);

            popover.set_child(Some(&menu));
            popover.connect_closed(|popover| popover.unparent());
            popover.popup();
        });
        item_box.add_controller(context_click);

        let drag_source = gtk4::DragSource::new();
        drag_source.set_actions(gtk4::gdk::DragAction::MOVE);
        let drag_gesture = gtk4::GestureDrag::new();
        drag_gesture.set_button(1);
        let drag_path = path.clone();
        let drag_icons = fixed.clone();
        let drag_origin_update = drag_origin.clone();
        let pointer_offset_update = pointer_offset.clone();
        drag_gesture.connect_drag_update(move |_, offset_x, offset_y| {
            let position = dragged_icon_position(
                *drag_origin_update.borrow(),
                *pointer_offset_update.borrow(),
                (offset_x, offset_y),
                desktop_size,
            );
            move_desktop_icon(&drag_icons, &drag_path, position);
        });
        let drag_path = path.clone();
        let internal_paths_begin = internal_drag_paths.clone();
        let active_drag_begin = active_icon_drag.clone();
        let pointer_offset_begin = pointer_offset.clone();
        let drag_origin_begin = drag_origin.clone();
        drag_source.connect_drag_begin(move |_, _| {
            internal_paths_begin.borrow_mut().remove(&drag_path);
            *active_drag_begin.borrow_mut() = Some((
                drag_path.clone(),
                *drag_origin_begin.borrow(),
                *pointer_offset_begin.borrow(),
            ));
        });
        let drag_file = file.clone();
        drag_source.connect_prepare(move |_, _, _| {
            let files = gtk4::gdk::FileList::from_array(std::slice::from_ref(&drag_file));
            Some(gtk4::gdk::ContentProvider::for_value(&files.to_value()))
        });
        let drag_file = file.clone();
        let drag_path = path.clone();
        let positions_after_drag = icon_positions.clone();
        let internal_paths_end = internal_drag_paths.clone();
        let active_drag_end = active_icon_drag.clone();
        let icons_after_drag = fixed.clone();
        drag_source.connect_drag_end(move |_, _, delete_data| {
            let was_internal_move = internal_paths_end.borrow_mut().remove(&drag_path);
            let original_position = active_drag_end
                .borrow_mut()
                .take()
                .filter(|(active_path, _, _)| active_path == &drag_path)
                .map(|(_, original_position, _)| original_position);
            if delete_data && !was_internal_move {
                match drag_file.delete(gio::Cancellable::NONE) {
                    Ok(()) => {
                        let mut positions = positions_after_drag.borrow_mut();
                        positions.remove(&drag_path);
                        save_icon_positions(&positions);
                    }
                    Err(error) => {
                        warn!(
                            "No se pudo mover {:?} fuera del escritorio: {}",
                            drag_file, error
                        );
                        if let Some(original_position) = original_position {
                            move_desktop_icon(&icons_after_drag, &drag_path, original_position);
                        }
                    }
                }
            } else if !was_internal_move {
                if let Some(original_position) = original_position {
                    move_desktop_icon(&icons_after_drag, &drag_path, original_position);
                }
            }
        });
        drag_source.group_with(&drag_gesture);
        item_box.add_controller(drag_source);
        item_box.add_controller(drag_gesture);
    }
}

fn launch_file(file: &gio::File) {
    if let Err(error) =
        gio::AppInfo::launch_default_for_uri(&file.uri(), None::<&gio::AppLaunchContext>)
    {
        warn!("No se pudo abrir {:?}: {}", file, error);
    }
}

fn create_wallpaper_window(
    application: &Application,
    monitor: Option<&gtk4::gdk::Monitor>,
    resolved_path: &Option<PathBuf>,
    desktop_dir: Option<&Path>,
    desktop_monitors: &Rc<RefCell<Vec<gio::FileMonitor>>>,
    icon_positions: &Rc<RefCell<HashMap<PathBuf, (f64, f64)>>>,
    internal_drag_paths: &Rc<RefCell<HashSet<PathBuf>>>,
    active_icon_drag: &Rc<RefCell<Option<(PathBuf, (f64, f64), (f64, f64))>>>,
) {
    let window = ApplicationWindow::builder()
        .application(application)
        .title("Fondo de Pantalla Malus DE")
        .decorated(false)
        .css_classes(["malus-wallpaper-window"])
        .build();

    window.set_decorated(false);

    if gtk4_layer_shell::is_supported() {
        info!("Anclando Fondo de Pantalla en capa Background a pantalla completa...");
        window.init_layer_shell();
        if let Some(mon) = monitor {
            window.set_monitor(mon);
        }
        window.set_layer(Layer::Background);
        window.set_namespace("malus-wallpaper");
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Right, true);
        window.set_exclusive_zone(-1);
    } else {
        window.maximize();
    }

    let overlay = Overlay::new();
    if let Some(ref img_path) = resolved_path {
        if img_path.exists() {
            info!("Cargando imagen de fondo: {:?}", img_path);
            let picture = Picture::for_filename(img_path);
            picture.set_content_fit(gtk4::ContentFit::Cover);
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            picture.set_halign(gtk4::Align::Fill);
            picture.set_valign(gtk4::Align::Fill);
            overlay.set_child(Some(&picture));
        }
    }

    if let Some(desktop_dir) = desktop_dir {
        let desktop_size = monitor
            .map(|monitor| {
                let geometry = monitor.geometry();
                (geometry.width(), geometry.height())
            })
            .unwrap_or((1920, 1080));
        let icons = Fixed::builder()
            .hexpand(true)
            .vexpand(true)
            .halign(Align::Fill)
            .valign(Align::Fill)
            .build();
        refresh_desktop_icons(
            &icons,
            desktop_dir,
            desktop_size,
            icon_positions,
            internal_drag_paths,
            active_icon_drag,
        );

        let drop_target = gtk4::DropTarget::new(
            gtk4::gdk::FileList::static_type(),
            gtk4::gdk::DragAction::MOVE,
        );
        let drop_dir = desktop_dir.to_path_buf();
        let drop_positions = icon_positions.clone();
        let internal_paths_drop = internal_drag_paths.clone();
        let active_drag_motion = active_icon_drag.clone();
        let drop_icons = icons.clone();
        drop_target.connect_motion(move |_, x, y| {
            if let Some((path, _, pointer_offset)) = active_drag_motion.borrow().as_ref() {
                let position = clamp_icon_position(x, y, *pointer_offset, desktop_size);
                move_desktop_icon(&drop_icons, path, position);
            }
            gtk4::gdk::DragAction::MOVE
        });
        let active_drag_drop = active_icon_drag.clone();
        let drop_icons = icons.clone();
        drop_target.connect_drop(move |_, value, x, y| {
            let files = match value.get::<gtk4::gdk::FileList>() {
                Ok(files) => files,
                Err(error) => {
                    warn!(
                        "El contenido arrastrado al escritorio no es una lista de archivos: {}",
                        error
                    );
                    return false;
                }
            };

            let mut moved_any = false;
            for (index, source) in files.files().into_iter().enumerate() {
                let Some(name) = source.basename() else {
                    warn!(
                        "No se pudo mover al escritorio un elemento sin nombre: {:?}",
                        source
                    );
                    continue;
                };
                let destination = gio::File::for_path(&drop_dir).child(&name);
                if source.equal(&destination) {
                    if let Some(path) = source.path() {
                        let pointer_offset = active_drag_drop
                            .borrow()
                            .as_ref()
                            .filter(|(active_path, _, _)| active_path == &path)
                            .map(|(_, _, pointer_offset)| *pointer_offset)
                            .unwrap_or((
                                f64::from(DESKTOP_ICON_WIDTH) / 2.0,
                                f64::from(DESKTOP_ICON_HEIGHT) / 2.0,
                            ));
                        let position = clamp_icon_position(x, y, pointer_offset, desktop_size);
                        if move_desktop_icon(&drop_icons, &path, position) {
                            drop_positions.borrow_mut().insert(path.clone(), position);
                            internal_paths_drop.borrow_mut().insert(path);
                            moved_any = true;
                        } else {
                            warn!(
                                "No se encontró el icono del escritorio que se intentó mover: {:?}",
                                path
                            );
                        }
                    }
                    continue;
                }

                let position = (
                    (x + (index % 4) as f64 * 24.0)
                        .clamp(0.0, f64::from((desktop_size.0 - DESKTOP_ICON_WIDTH).max(0))),
                    (y + (index % 4) as f64 * 24.0).clamp(
                        0.0,
                        f64::from((desktop_size.1 - DESKTOP_ICON_HEIGHT).max(0)),
                    ),
                );
                match source.move_(
                    &destination,
                    gio::FileCopyFlags::NONE,
                    gio::Cancellable::NONE,
                    None,
                ) {
                    Ok(()) => {
                        if let Some(path) = destination.path() {
                            drop_positions.borrow_mut().insert(path, position);
                        }
                        moved_any = true;
                    }
                    Err(error) => {
                        warn!(
                            "No se pudo mover {:?} al escritorio {:?}: {}",
                            source, drop_dir, error
                        );
                    }
                }
            }
            if moved_any {
                save_icon_positions(&drop_positions.borrow());
            }
            moved_any
        });
        icons.add_controller(drop_target);
        overlay.add_overlay(&icons);

        let desktop_file = gio::File::for_path(desktop_dir);
        match desktop_file.monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
            Ok(monitor) => {
                let icons_monitor = icons.clone();
                let dir_monitor = desktop_dir.to_path_buf();
                let icon_positions_monitor = icon_positions.clone();
                let internal_drag_paths_monitor = internal_drag_paths.clone();
                let active_icon_drag_monitor = active_icon_drag.clone();
                monitor.connect_changed(move |_, _, _, _| {
                    refresh_desktop_icons(
                        &icons_monitor,
                        &dir_monitor,
                        desktop_size,
                        &icon_positions_monitor,
                        &internal_drag_paths_monitor,
                        &active_icon_drag_monitor,
                    );
                });
                desktop_monitors.borrow_mut().push(monitor);
            }
            Err(error) => {
                warn!(
                    "No se pudo monitorear la carpeta de escritorio {:?}: {}",
                    desktop_dir, error
                );
            }
        }
    }

    window.set_child(Some(&overlay));
    window.present();
}

fn main() -> Result<()> {
    init_logging("malus_de_wallpaper");
    let args = Args::parse();
    info!("Iniciando Malus DE Wallpaper Daemon...");

    // Esperar de forma activa a que el compositor esté listo y aceptando conexiones
    malus_de_common::wayland::wait_for_wayland(10);

    let app = Application::builder()
        .application_id("org.malus_de.Wallpaper")
        .build();

    let image_path = args.image;
    let desktop_dir = resolve_desktop_dir();
    let desktop_monitors = Rc::new(RefCell::new(Vec::new()));
    let icon_positions = Rc::new(RefCell::new(load_icon_positions()));
    let internal_drag_paths = Rc::new(RefCell::new(HashSet::new()));
    let active_icon_drag = Rc::new(RefCell::new(None));

    app.connect_startup(|_| {
        let provider = CssProvider::new();
        provider.load_from_string(DEFAULT_GRADIENT_CSS);
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });

    app.connect_activate(move |application| {
        let resolved_path = image_path.clone().or_else(|| {
            let config_dir = std::env::var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    std::env::var("HOME")
                        .map(|h| PathBuf::from(h).join(".config/malus-de"))
                        .unwrap_or_else(|_| PathBuf::from("/etc/malus-de"))
                });
            let candidates = [
                config_dir.join("wallpaper.png"),
                config_dir.join("wallpaper.jpg"),
                config_dir.join("wallpaper.jpeg"),
                config_dir.join("wallpaper.webp"),
                config_dir.join("wallpaper.svg"),
            ];
            candidates.into_iter().find(|p| p.exists())
        });

        if let Some(desktop_dir) = &desktop_dir {
            info!(
                "Mostrando contenido de la carpeta de escritorio: {:?}",
                desktop_dir
            );
        } else {
            warn!("No se pudo determinar la carpeta de escritorio del usuario.");
        }

        let mut created = 0;
        if let Some(display) = gtk4::gdk::Display::default() {
            let monitors = display.monitors();
            let primary_connector = DisplaysConfig::load()
                .outputs
                .into_iter()
                .find(|output| output.primary)
                .map(|output| output.connector);
            let monitor_list: Vec<gtk4::gdk::Monitor> = (0..monitors.n_items())
                .filter_map(|i| monitors.item(i))
                .filter_map(|monitor| monitor.downcast().ok())
                .collect();
            let primary_index = primary_connector
                .as_deref()
                .and_then(|connector| {
                    monitor_list.iter().position(|monitor| {
                        monitor
                            .connector()
                            .is_some_and(|name| name.as_str() == connector)
                    })
                })
                .unwrap_or(0);

            match (primary_connector.as_deref(), monitor_list.get(primary_index)) {
                (Some(connector), Some(_)) => info!(
                    "Mostrando iconos del escritorio en la pantalla principal: {}",
                    connector
                ),
                (_, Some(monitor)) => info!(
                    "No se encontró la pantalla principal configurada; usando {} para los iconos del escritorio.",
                    monitor
                        .connector()
                        .map(|name| name.to_string())
                        .unwrap_or_else(|| "la primera pantalla".to_string())
                ),
                (_, None) => {}
            }

            for (index, monitor) in monitor_list.iter().enumerate() {
                create_wallpaper_window(
                    application,
                    Some(monitor),
                    &resolved_path,
                    if index == primary_index {
                        desktop_dir.as_deref()
                    } else {
                        None
                    },
                    &desktop_monitors,
                    &icon_positions,
                    &internal_drag_paths,
                    &active_icon_drag,
                );
                created += 1;
            }
        }

        if created == 0 {
            create_wallpaper_window(
                application,
                None,
                &resolved_path,
                desktop_dir.as_deref(),
                &desktop_monitors,
                &icon_positions,
                &internal_drag_paths,
                &active_icon_drag,
            );
        }
    });

    app.run_with_args(&["malus-de-wallpaper"]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{clamp_icon_position, desktop_dir_from_config, dragged_icon_position};
    use std::path::{Path, PathBuf};

    #[test]
    fn keeps_pointer_offset_and_clamps_icon_to_desktop() {
        assert_eq!(
            clamp_icon_position(150.0, 100.0, (20.0, 30.0), (800, 600)),
            (130.0, 70.0)
        );
        assert_eq!(
            clamp_icon_position(-20.0, -30.0, (20.0, 30.0), (800, 600)),
            (0.0, 0.0)
        );
        assert_eq!(
            clamp_icon_position(900.0, 700.0, (0.0, 0.0), (800, 600)),
            (688.0, 488.0)
        );
    }

    #[test]
    fn follows_drag_offset_without_shifting_the_pointer_anchor() {
        assert_eq!(
            dragged_icon_position((130.0, 70.0), (20.0, 30.0), (15.0, -10.0), (800, 600)),
            (145.0, 60.0)
        );
        assert_eq!(
            dragged_icon_position((680.0, 480.0), (20.0, 30.0), (100.0, 100.0), (800, 600)),
            (688.0, 488.0)
        );
    }

    #[test]
    fn resolves_spanish_desktop_directory_from_user_dirs() {
        let path = desktop_dir_from_config(
            "XDG_DESKTOP_DIR=\"$HOME/Escritorio\"\n",
            Path::new("/home/user"),
        );
        assert_eq!(path, PathBuf::from("/home/user/Escritorio"));
    }

    #[test]
    fn defaults_to_escritorio_when_user_dir_is_not_configured() {
        let path = desktop_dir_from_config("", Path::new("/home/user"));
        assert_eq!(path, PathBuf::from("/home/user/Escritorio"));
    }
}
