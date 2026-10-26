pub mod winit;
pub mod drm;

use smithay::utils::{Physical, Rectangle};

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
