//! Utilidad compartida para tintar dinámicamente los íconos de carpeta SVG del tema activo.
//!
//! Los íconos de carpeta de Colloid/Malus (`places/scalable/folder*.svg`) usan el color base
//! `#60c0f0` junto con capas de sombra/brillo en `#000000` y `#ffffff` con opacidad. Al sustituir
//! `#60c0f0` por el color elegido en Ajustes → Aspecto, todas las carpetas especiales conservan
//! sus relieves, transparencias y detalles vectoriales a resolución 2x.

use crate::apps::resolve_icon_file_path;
use crate::config::load_folder_tint_hex;
use gtk4::gdk;
use gtk4::gdk_pixbuf;
use gtk4::gio;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

const MAX_FOLDER_TEX_CACHE: usize = 256;

fn is_folder_themed_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.contains("trash") {
        return false;
    }
    lower == "folder"
        || lower.starts_with("folder-")
        || lower.starts_with("folder_")
        || lower == "user-desktop"
        || lower == "user-home"
        || lower == "inode-directory"
}

fn load_raw_svg_cached(path: &PathBuf) -> Option<String> {
    thread_local! {
        static RAW_CACHE: RefCell<HashMap<PathBuf, Option<String>>> = RefCell::new(HashMap::new());
    }
    RAW_CACHE.with(|cell| {
        let mut map = cell.borrow_mut();
        if let Some(cached) = map.get(path) {
            return cached.clone();
        }
        let read = std::fs::read_to_string(path).ok();
        map.insert(path.clone(), read.clone());
        read
    })
}

/// Resuelve y renderiza un ícono de carpeta tintado según `folder_color_idx` y `accent_color_idx`.
pub fn tinted_folder_texture(gicon: &gio::Icon, pixel_size: i32) -> Option<gdk::Texture> {
    let themed = gicon.downcast_ref::<gio::ThemedIcon>()?;
    let names = themed.names();
    let primary_folder_name = names
        .iter()
        .find(|n| is_folder_themed_name(n.as_str()))?
        .as_str()
        .to_string();

    let target_hex = load_folder_tint_hex();
    let render_px = (pixel_size.max(16) * 2).clamp(32, 256);

    thread_local! {
        static NAME_TEX_CACHE: RefCell<HashMap<(String, i32, String), Option<gdk::Texture>>> =
            RefCell::new(HashMap::new());
        static TEX_CACHE: RefCell<HashMap<(PathBuf, i32, String), gdk::Texture>> =
            RefCell::new(HashMap::new());
    }

    let fast_key = (primary_folder_name, render_px, target_hex.clone());
    if let Some(hit) = NAME_TEX_CACHE.with(|c| c.borrow().get(&fast_key).cloned()) {
        return hit;
    }

    let mut resolved_path: Option<PathBuf> = None;
    for name in &names {
        if !is_folder_themed_name(name.as_str()) {
            continue;
        }
        if let Some(p) = resolve_icon_file_path(name.as_str()) {
            if p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
            {
                resolved_path = Some(p);
                break;
            }
        }
    }
    if resolved_path.is_none() {
        resolved_path = resolve_icon_file_path("folder").filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
        });
    }
    let Some(svg_path) = resolved_path else {
        NAME_TEX_CACHE.with(|c| {
            let mut map = c.borrow_mut();
            if map.len() >= MAX_FOLDER_TEX_CACHE {
                map.clear();
            }
            map.insert(fast_key, None);
        });
        return None;
    };

    let key = (svg_path.clone(), render_px, target_hex.clone());
    if let Some(hit) = TEX_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        NAME_TEX_CACHE.with(|c| {
            let mut map = c.borrow_mut();
            if map.len() >= MAX_FOLDER_TEX_CACHE {
                map.clear();
            }
            map.insert(fast_key, Some(hit.clone()));
        });
        return Some(hit);
    }

    let Some(raw_svg) = load_raw_svg_cached(&svg_path) else {
        return None;
    };
    if !raw_svg.contains("#60c0f0") && !raw_svg.contains("#60C0F0") {
        NAME_TEX_CACHE.with(|c| {
            let mut map = c.borrow_mut();
            if map.len() >= MAX_FOLDER_TEX_CACHE {
                map.clear();
            }
            map.insert(fast_key, None);
        });
        return None;
    }
    let tinted_svg = raw_svg
        .replace("#60c0f0", &target_hex)
        .replace("#60C0F0", &target_hex);

    let loader = gdk_pixbuf::PixbufLoader::with_type("svg").ok()?;
    loader.set_size(render_px, render_px);
    loader.write(tinted_svg.as_bytes()).ok()?;
    loader.close().ok()?;
    let pixbuf = loader.pixbuf()?;
    let texture = gdk::Texture::for_pixbuf(&pixbuf);

    TEX_CACHE.with(|c| {
        let mut map = c.borrow_mut();
        if map.len() >= MAX_FOLDER_TEX_CACHE {
            map.clear();
        }
        map.insert(key, texture.clone());
    });
    NAME_TEX_CACHE.with(|c| {
        let mut map = c.borrow_mut();
        if map.len() >= MAX_FOLDER_TEX_CACHE {
            map.clear();
        }
        map.insert(fast_key, Some(texture.clone()));
    });
    Some(texture)
}

/// Aplica el ícono de carpeta tintado dinámicamente a un `gtk4::Image` si corresponde.
pub fn apply_tinted_folder_icon(img: &gtk4::Image, gicon: &gio::Icon, pixel_size: i32) -> bool {
    if let Some(tex) = tinted_folder_texture(gicon, pixel_size) {
        img.set_paintable(Some(&tex));
        img.set_pixel_size(pixel_size);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::is_folder_themed_name;

    #[test]
    fn folder_themed_name_filter_includes_folders_and_excludes_trash() {
        assert!(is_folder_themed_name("folder"));
        assert!(is_folder_themed_name("folder-documents"));
        assert!(is_folder_themed_name("user-desktop"));
        assert!(!is_folder_themed_name("user-trash"));
        assert!(!is_folder_themed_name("application-x-executable"));
    }
}
