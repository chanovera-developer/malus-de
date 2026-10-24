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
