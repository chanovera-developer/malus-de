pub mod drm;
pub mod winit;

use smithay::utils::{Physical, Rectangle};
use std::time::Duration;

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
    let logical_height = geometry.size.h.max(0) as f64 / scale;
    let tail_height = 17.0_f64.min(logical_height / 4.0);
    // Match the tail height and trailing inset used by the dock's SVG popover.
    let bottom_inset = ((tail_height + 17.0) * scale).round() as i32;
    let mut clipped = geometry;
    clipped.size.h -= bottom_inset.min(geometry.size.h.max(0));
    clipped
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

        assert_eq!(clipped.loc, geometry.loc);
        assert_eq!(clipped.size, (320, 230).into());
    }

    #[test]
    fn dock_popover_blur_scales_the_tail_inset() {
        let geometry = Rectangle::new((12, 34).into(), (640, 528).into());
        let clipped = dock_popover_blur_rect(geometry, 2.0);

        assert_eq!(clipped.size, (640, 460).into());
    }

    #[test]
    fn dock_popover_blur_clamps_short_popovers() {
        let geometry = Rectangle::new((0, 0).into(), (100, 40).into());
        let clipped = dock_popover_blur_rect(geometry, 1.0);

        assert_eq!(clipped.size, (100, 13).into());
    }
}
