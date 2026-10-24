pub mod winit;
pub mod drm;

use smithay::utils::{Logical, Physical, Rectangle};

pub fn dock_clip_rect(
	geometry: Rectangle<i32, Logical>,
	headroom: i32,
	scale: f64,
) -> Rectangle<i32, Physical> {
	let inset = headroom.min(geometry.size.h.max(0));
	let mut clipped = geometry;
	clipped.loc.y += inset;
	clipped.size.h -= inset;
	clipped.to_physical_precise_round(scale)
}
