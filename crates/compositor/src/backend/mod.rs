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

#[cfg(test)]
mod tests {
    use super::*;

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
