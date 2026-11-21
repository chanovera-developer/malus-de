pub mod drm;
pub mod winit;

use smithay::utils::{Physical, Rectangle};
use std::time::Duration;

const DOCK_POPOVER_SHADOW_GUTTER: f64 = 32.0;
/// Radio squircle lógico (en píxeles 1x, n=4.2) para menús emergentes y popups del compositor,
/// calibrado para enmarcar el resaltado squircle interior (~14 px) sin recortes ni huecos excesivos.
pub(super) const POPUP_MENU_CORNER_RADIUS_LOGICAL: f64 = 20.0;

pub fn frame_interval(refresh_millihz: i32) -> Duration {
    if refresh_millihz > 0 {
        Duration::from_nanos(1_000_000_000_000 / refresh_millihz as u64)
    } else {
        Duration::from_nanos(16_666_667)
    }
}

pub fn dock_is_collapsed(
    geometry: Rectangle<i32, Physical>,
    scale: f64,
    position: malus_de_common::config::DockPosition,
) -> bool {
    let min_bar_px = (12.0 * scale.max(1.0)).round() as i32;
    match position {
        malus_de_common::config::DockPosition::Bottom => geometry.size.h <= min_bar_px,
        malus_de_common::config::DockPosition::Left
        | malus_de_common::config::DockPosition::Right => geometry.size.w <= min_bar_px,
    }
}

pub fn dock_clip_rect(
    geometry: Rectangle<i32, Physical>,
    headroom: i32,
    outer_gap: i32,
    scale: f64,
    position: malus_de_common::config::DockPosition,
) -> Rectangle<i32, Physical> {
    let mut clipped = geometry;
    let inner_inset = (headroom.max(0) as f64 * scale).round() as i32;
    let edge_inset = (outer_gap.max(0) as f64 * scale).round() as i32;
    match position {
        malus_de_common::config::DockPosition::Bottom => {
            let applied_top = inner_inset.min(clipped.size.h.max(0));
            clipped.loc.y += applied_top;
            clipped.size.h = (clipped.size.h - applied_top - edge_inset).max(1);
        }
        malus_de_common::config::DockPosition::Left => {
            let applied_left = edge_inset.min(clipped.size.w.max(0));
            clipped.loc.x += applied_left;
            clipped.size.w = (clipped.size.w - applied_left - inner_inset).max(1);
        }
        malus_de_common::config::DockPosition::Right => {
            let applied_left = inner_inset.min(clipped.size.w.max(0));
            clipped.loc.x += applied_left;
            clipped.size.w = (clipped.size.w - applied_left - edge_inset).max(1);
        }
    }
    clipped
}

pub fn dock_popover_blur_rect(
    geometry: Rectangle<i32, Physical>,
    scale: f64,
) -> Rectangle<i32, Physical> {
    let scale = scale.max(f64::EPSILON);
    let shadow_gutter = (DOCK_POPOVER_SHADOW_GUTTER * scale).round() as i32;
    let content_height =
        (geometry.size.h.max(0) as f64 / scale - 2.0 * DOCK_POPOVER_SHADOW_GUTTER).max(0.0);
    let tail_height = 17.0_f64.min(content_height / 4.0);
    let bottom_inset = ((tail_height + 17.0) * scale).round() as i32;
    let mut clipped = geometry;
    clipped.loc.x += shadow_gutter;
    clipped.loc.y += shadow_gutter;
    clipped.size.w = (geometry.size.w - shadow_gutter * 2).max(0);
    clipped.size.h = (geometry.size.h - shadow_gutter * 2 - bottom_inset).max(0);
    clipped
}

pub fn spotlight_component_blur_rect(
    geometry: Rectangle<i32, Physical>,
    scale: f64,
) -> Rectangle<i32, Physical> {
    let padding = (28.0 * scale.max(f64::EPSILON)).round() as i32;
    let inset = padding
        .min((geometry.size.w.max(1) - 1) / 2)
        .min((geometry.size.h.max(1) - 1) / 2);
    Rectangle::new(
        geometry.loc + smithay::utils::Size::from((inset, inset)),
        (geometry.size.w - inset * 2, geometry.size.h - inset * 2).into(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundBlurCapturePlan {
    None,
    FullScreen,
    Region(Rectangle<i32, Physical>),
}

/// Calcula el rectángulo de desenfoque de una ventana translúcida (p. ej. barra lateral de 212 px).
pub fn translucent_window_blur_rect(
    win_rect: Rectangle<i32, Physical>,
    is_branches: bool,
    is_translucent: bool,
    win_phys_r: Option<i32>,
    scale: f64,
) -> Rectangle<i32, Physical> {
    if is_branches || is_translucent {
        let sidebar_blur_w = ((212.0 * scale).round() as i32 + win_phys_r.unwrap_or(0) as i32 + 4)
            .min(win_rect.size.w);
        Rectangle::new(win_rect.loc, (sidebar_blur_w, win_rect.size.h).into())
    } else {
        win_rect
    }
}

/// Planifica si es necesario recapturar el snapshot del fondo para el desenfoque GPU en este cuadro.
pub fn plan_background_blur_capture(
    force_full_screen: bool,
    bg_committed: bool,
    snapshot_stale: bool,
    bottom_translucent_rect: Option<Rectangle<i32, Physical>>,
    bottom_needs_shadow_blur: bool,
    damage: &[Rectangle<i32, Physical>],
) -> BackgroundBlurCapturePlan {
    if force_full_screen {
        return BackgroundBlurCapturePlan::FullScreen;
    }
    if !bg_committed && !snapshot_stale {
        return BackgroundBlurCapturePlan::None;
    }
    let Some(blur_rect) = bottom_translucent_rect else {
        return BackgroundBlurCapturePlan::None;
    };
    if bottom_needs_shadow_blur || damage.iter().any(|d| d.overlaps(blur_rect)) {
        BackgroundBlurCapturePlan::Region(blur_rect)
    } else {
        BackgroundBlurCapturePlan::None
    }
}

/// Comprueba si un rectángulo exterior contiene completamente a un rectángulo interior.
#[inline]
pub fn rect_contains_rect(
    outer: &Rectangle<i32, Physical>,
    inner: &Rectangle<i32, Physical>,
) -> bool {
    if inner.size.w <= 0 || inner.size.h <= 0 {
        return true;
    }
    if outer.size.w <= 0 || outer.size.h <= 0 {
        return false;
    }
    outer.loc.x <= inner.loc.x
        && outer.loc.y <= inner.loc.y
        && outer.loc.x + outer.size.w >= inner.loc.x + inner.size.w
        && outer.loc.y + outer.size.h >= inner.loc.y + inner.size.h
}

/// Recorta un rectángulo para que no exceda los límites del área visible indicada.
#[inline]
pub fn clamp_rect_to_bounds(
    rect: Rectangle<i32, Physical>,
    bounds: Rectangle<i32, Physical>,
) -> Rectangle<i32, Physical> {
    let x0 = rect.loc.x.max(bounds.loc.x);
    let y0 = rect.loc.y.max(bounds.loc.y);
    let x1 = (rect.loc.x + rect.size.w).min(bounds.loc.x + bounds.size.w);
    let y1 = (rect.loc.y + rect.size.h).min(bounds.loc.y + bounds.size.h);
    Rectangle::new((x0, y0).into(), ((x1 - x0).max(0), (y1 - y0).max(0)).into())
}

/// Representa un oclusor opaco (ventana que cubre áreas inferiores) para descarte de sobre-renderizado.
#[derive(Debug, Clone, Copy)]
pub struct OpaqueOccluder {
    pub rect: Rectangle<i32, Physical>,
    pub corner_radius: i32,
}

impl OpaqueOccluder {
    /// Determina si el objetivo (con su propio radio de esquina `target_radius`) queda 100% cubierto
    /// por el área opaca de este oclusor.
    pub fn contains_rounded_rect(
        &self,
        target: Rectangle<i32, Physical>,
        target_radius: i32,
    ) -> bool {
        if target.size.w <= 0 || target.size.h <= 0 {
            return true;
        }
        if self.rect.size.w <= 0 || self.rect.size.h <= 0 {
            return false;
        }
        if !rect_contains_rect(&self.rect, &target) {
            return false;
        }
        let r = self
            .corner_radius
            .max(0)
            .min(self.rect.size.w / 2)
            .min(self.rect.size.h / 2);
        if r == 0 || target_radius >= r {
            return true;
        }
        // Dos bandas ortogonales opacas que excluyen las 4 esquinas redondeadas
        let band_h = Rectangle::new(
            (self.rect.loc.x, self.rect.loc.y + r).into(),
            (self.rect.size.w, (self.rect.size.h - 2 * r).max(0)).into(),
        );
        let band_v = Rectangle::new(
            (self.rect.loc.x + r, self.rect.loc.y).into(),
            ((self.rect.size.w - 2 * r).max(0), self.rect.size.h).into(),
        );
        rect_contains_rect(&band_h, &target) || rect_contains_rect(&band_v, &target)
    }

    /// Determina si el objetivo queda 100% cubierto por el área opaca de este oclusor.
    pub fn contains_rect(&self, target: Rectangle<i32, Physical>) -> bool {
        self.contains_rounded_rect(target, 0)
    }
}

pub fn capture_dnd_icon_cache(
    renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
    state: &mut crate::state::MalusState,
    dnd_surf: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    scale: f64,
) {
    use smithay::backend::allocator::Fourcc;
    use smithay::backend::renderer::{
        element::{
            memory::MemoryRenderBuffer,
            surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
            Element, Kind,
        },
        gles::{GlesRenderer, GlesTexture},
        utils::draw_render_elements,
        Bind, Color32F, ExportMem, Frame, Offscreen, Renderer,
    };
    use smithay::utils::{Buffer as BufferCoord, Point, Scale, Size, Transform};

    if !state.dnd_icon_cache_dirty {
        return;
    }
    let scale = scale.max(1.0);
    let elems = render_elements_from_surface_tree::<
        GlesRenderer,
        WaylandSurfaceRenderElement<GlesRenderer>,
    >(
        renderer,
        dnd_surf,
        Point::from((0, 0)),
        Scale::from(scale),
        1.0,
        Kind::Unspecified,
    );
    if elems.is_empty() {
        return;
    }

    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    let scale_s = Scale::from(scale);
    for elem in &elems {
        let g = elem.geometry(scale_s);
        min_x = min_x.min(g.loc.x);
        min_y = min_y.min(g.loc.y);
        max_x = max_x.max(g.loc.x + g.size.w);
        max_y = max_y.max(g.loc.y + g.size.h);
    }
    let w = (max_x - min_x).max(0);
    let h = (max_y - min_y).max(0);
    if w <= 0 || h <= 0 {
        return;
    }

    let elems = render_elements_from_surface_tree::<
        GlesRenderer,
        WaylandSurfaceRenderElement<GlesRenderer>,
    >(
        renderer,
        dnd_surf,
        Point::from((-min_x, -min_y)),
        scale_s,
        1.0,
        Kind::Unspecified,
    );

    let buf_size = Size::<i32, BufferCoord>::from((w, h));
    let Ok(mut texture) = <GlesRenderer as Offscreen<GlesTexture>>::create_buffer(
        renderer,
        Fourcc::Abgr8888,
        buf_size,
    ) else {
        return;
    };

    let phys_size = Size::<i32, Physical>::from((w, h));
    let damage_rect = Rectangle::from_size(phys_size);
    let Ok(mut target) = renderer.bind(&mut texture) else {
        return;
    };
    {
        let Ok(mut frame) = renderer.render(&mut target, phys_size, Transform::Normal) else {
            return;
        };
        let _ = frame.clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[damage_rect]);
        let _ = draw_render_elements(&mut frame, scale, &elems, &[damage_rect]);
        if frame.finish().is_err() {
            return;
        }
    }

    let copy_rect = Rectangle::from_size(buf_size);
    let Ok(mapping) = renderer.copy_framebuffer(&target, copy_rect, Fourcc::Abgr8888) else {
        return;
    };
    let Ok(mapped) = renderer.map_texture(&mapping) else {
        return;
    };
    let expected_len = (w as usize).saturating_mul(h as usize).saturating_mul(4);
    if mapped.len() != expected_len {
        return;
    }

    let buffer = MemoryRenderBuffer::from_slice(
        &mapped,
        Fourcc::Abgr8888,
        buf_size,
        1,
        Transform::Normal,
        None,
    );
    state.dnd_icon_cache = Some(crate::state::CachedDndIcon {
        buffer,
        rel_offset_logical: (min_x as f64 / scale, min_y as f64 / scale),
    });
    state.dnd_icon_cache_dirty = false;
}

pub fn build_cached_dnd_fallback(
    renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
    state: &crate::state::MalusState,
    draw_x: f64,
    draw_y: f64,
    scale: f64,
) -> Option<
    smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement<
        smithay::backend::renderer::gles::GlesRenderer,
    >,
> {
    use smithay::backend::renderer::element::{memory::MemoryRenderBufferRenderElement, Kind};
    use smithay::utils::Point;

    let cached = state.dnd_icon_cache.as_ref()?;
    let offset = state.dnd_icon_offset;
    let scale = scale.max(1.0);
    let phys_x = (draw_x + offset.x as f64 + cached.rel_offset_logical.0) * scale;
    let phys_y = (draw_y + offset.y as f64 + cached.rel_offset_logical.1) * scale;
    MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        Point::<f64, Physical>::from((phys_x.round(), phys_y.round())),
        &cached.buffer,
        Some(1.0),
        None,
        None,
        Kind::Unspecified,
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_rect_detects_complete_enclosure() {
        let outer = Rectangle::new((0, 0).into(), (1920, 1080).into());
        let inside = Rectangle::new((100, 100).into(), (800, 600).into());
        let overlapping = Rectangle::new((1800, 1000).into(), (200, 200).into());

        assert!(rect_contains_rect(&outer, &inside));
        assert!(!rect_contains_rect(&outer, &overlapping));
    }

    #[test]
    fn opaque_occluder_handles_rounded_corners_safely() {
        let occluder = OpaqueOccluder {
            rect: Rectangle::new((0, 0).into(), (1000, 800).into()),
            corner_radius: 32,
        };

        // En el centro de la ventana: completamente cubierto
        let central = Rectangle::new((100, 100).into(), (400, 300).into());
        assert!(occluder.contains_rect(central));

        // En la esquina superior izquierda (dentro del radio de corte de esquina): no ocluido si target no es redondeado
        let in_corner = Rectangle::new((5, 5).into(), (20, 20).into());
        assert!(!occluder.contains_rect(in_corner));

        // Otra ventana maximizada con el mismo rectángulo y mismo radio de esquina (32) queda 100% ocluida
        let background_maximized = Rectangle::new((0, 0).into(), (1000, 800).into());
        assert!(occluder.contains_rounded_rect(background_maximized, 32));
    }

    #[test]
    fn frame_interval_matches_output_refresh_rate() {
        assert_eq!(frame_interval(60_000), Duration::from_nanos(16_666_666));
        assert_eq!(frame_interval(120_000), Duration::from_nanos(8_333_333));
    }

    #[test]
    fn frame_interval_uses_sixty_hertz_for_invalid_refresh_rates() {
        assert_eq!(frame_interval(0), Duration::from_nanos(16_666_667));
        assert_eq!(frame_interval(-1), Duration::from_nanos(16_666_667));
    }

    #[test]
    fn dock_popover_blur_stops_at_the_body_before_the_tail() {
        let geometry = Rectangle::new((12, 34).into(), (320, 264).into());
        let clipped = dock_popover_blur_rect(geometry, 1.0);

        assert_eq!(clipped.loc, (44, 66).into());
        assert_eq!(clipped.size, (256, 166).into());
    }

    #[test]
    fn dock_popover_blur_scales_shadow_gutter_and_tail_inset() {
        let geometry = Rectangle::new((12, 34).into(), (640, 528).into());
        let clipped = dock_popover_blur_rect(geometry, 2.0);

        assert_eq!(clipped.loc, (76, 98).into());
        assert_eq!(clipped.size, (512, 332).into());
    }

    #[test]
    fn dock_popover_blur_clamps_short_popovers() {
        let geometry = Rectangle::new((0, 0).into(), (100, 40).into());
        let clipped = dock_popover_blur_rect(geometry, 1.0);

        assert_eq!(clipped.size, (36, 0).into());
    }

    #[test]
    fn spotlight_blur_excludes_the_shadow_gutter() {
        let geometry = Rectangle::new((12, 34).into(), (486, 110).into());
        let clipped = spotlight_component_blur_rect(geometry, 1.0);

        assert_eq!(clipped.loc, (40, 62).into());
        assert_eq!(clipped.size, (430, 54).into());
    }

    #[test]
    fn spotlight_blur_scales_the_shadow_gutter() {
        let geometry = Rectangle::new((24, 68).into(), (972, 220).into());
        let clipped = spotlight_component_blur_rect(geometry, 2.0);

        assert_eq!(clipped.loc, (80, 124).into());
        assert_eq!(clipped.size, (860, 108).into());
    }

    #[test]
    fn background_blur_capture_skips_desktop_animation_without_translucent_overlap() {
        let damage = [Rectangle::new((1400, 100).into(), (240, 320).into())];
        assert_eq!(
            plan_background_blur_capture(false, true, false, None, false, &damage),
            BackgroundBlurCapturePlan::None
        );

        let sidebar_blur = translucent_window_blur_rect(
            Rectangle::new((40, 40).into(), (900, 600).into()),
            true,
            true,
            Some(28),
            1.0,
        );
        assert_eq!(
            plan_background_blur_capture(false, true, false, Some(sidebar_blur), false, &damage),
            BackgroundBlurCapturePlan::None
        );

        let overlapping_damage = [Rectangle::new((120, 120).into(), (180, 180).into())];
        assert_eq!(
            plan_background_blur_capture(
                false,
                true,
                false,
                Some(sidebar_blur),
                false,
                &overlapping_damage
            ),
            BackgroundBlurCapturePlan::Region(sidebar_blur)
        );
        assert_eq!(
            plan_background_blur_capture(true, false, false, None, false, &damage),
            BackgroundBlurCapturePlan::FullScreen
        );
    }

    #[test]
    fn dock_auto_hide_collapse_detection_and_outer_gap_clip_work_across_edges() {
        use malus_de_common::config::DockPosition;
        assert!(dock_is_collapsed(
            Rectangle::new((0, 0).into(), (560, 6).into()),
            1.0,
            DockPosition::Bottom
        ));
        assert!(!dock_is_collapsed(
            Rectangle::new((0, 0).into(), (560, 82).into()),
            1.0,
            DockPosition::Bottom
        ));
        assert!(dock_is_collapsed(
            Rectangle::new((0, 0).into(), (12, 1120).into()),
            2.0,
            DockPosition::Left
        ));
        assert!(!dock_is_collapsed(
            Rectangle::new((0, 0).into(), (160, 1120).into()),
            2.0,
            DockPosition::Right
        ));

        let bottom = dock_clip_rect(
            Rectangle::new((100, 900).into(), (500, 130).into()),
            46,
            4,
            1.0,
            DockPosition::Bottom,
        );
        assert_eq!(bottom.loc, (100, 946).into());
        assert_eq!(bottom.size, (500, 80).into());

        let left = dock_clip_rect(
            Rectangle::new((0, 200).into(), (130, 500).into()),
            46,
            4,
            1.0,
            DockPosition::Left,
        );
        assert_eq!(left.loc, (4, 200).into());
        assert_eq!(left.size, (80, 500).into());

        let right = dock_clip_rect(
            Rectangle::new((1790, 200).into(), (130, 500).into()),
            46,
            4,
            1.0,
            DockPosition::Right,
        );
        assert_eq!(right.loc, (1836, 200).into());
        assert_eq!(right.size, (80, 500).into());
    }
}
