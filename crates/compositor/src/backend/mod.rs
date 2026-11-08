pub mod drm;
pub mod winit;

use smithay::utils::{Physical, Rectangle};
use std::time::Duration;

const DOCK_POPOVER_SHADOW_GUTTER: f64 = 32.0;

pub fn frame_interval(refresh_millihz: i32) -> Duration {
    if refresh_millihz > 0 {
        Duration::from_nanos(1_000_000_000_000 / refresh_millihz as u64)
    } else {
        Duration::from_nanos(16_666_667)
    }
}

pub fn dock_clip_rect(
    geometry: Rectangle<i32, Physical>,
    headroom: i32,
    scale: f64,
) -> Rectangle<i32, Physical> {
    let inset = ((headroom.max(0) as f64 * scale).round() as i32).min(geometry.size.h.max(0));
    let mut clipped = geometry;
    clipped.loc.y += inset;
    clipped.size.h -= inset;
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
}
