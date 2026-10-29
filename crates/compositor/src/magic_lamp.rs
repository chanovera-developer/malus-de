use crate::genie::GenieRect;

pub struct MagicLampAnimation {
    window: GenieRect,
    full_height: f32,
    current_height: f32,
    offset_y: f32,
    window_center_x: f32,
    target_w: f32,
    path_dx: f32,
    path_dy: f32,
    path_length: f32,
    bend_side: f32,
    bend_distance: f32,
    k: f32,
    j: f32,
}

impl MagicLampAnimation {
    pub fn new(
        window: GenieRect,
        target: GenieRect,
        raw_progress: f32,
        is_minimizing: bool,
    ) -> Self {
        let raw_progress = raw_progress.clamp(0.0, 1.0);
        let t = if is_minimizing {
            if raw_progress < 0.65 {
                (raw_progress / 0.65).clamp(0.0, 1.0).powf(1.35) * 0.65
            } else {
                let p = ((raw_progress - 0.65) / 0.35).clamp(0.0, 1.0);
                0.65 + (1.0 - (1.0 - p).powf(2.6)) * 0.35
            }
        } else if raw_progress < 0.75 {
            (raw_progress / 0.75).clamp(0.0, 1.0).powf(0.85) * 0.75
        } else {
            let p = ((raw_progress - 0.75) / 0.25).clamp(0.0, 1.0);
            0.75 + (1.0 - (1.0 - p).powf(2.2)) * 0.25
        };

        let split = 0.30;
        let (k, j) = if is_minimizing {
            let k = if t <= split { t / split } else { 1.0 };
            let j = if t > split {
                (t - split) / (1.0 - split)
            } else {
                0.0
            };
            (k, j)
        } else {
            let j = 1.0
                - if t <= (1.0 - split) {
                    t / (1.0 - split)
                } else {
                    1.0
                };
            let k = 1.0
                - if t > (1.0 - split) {
                    (t - (1.0 - split)) / split
                } else {
                    0.0
                };
            (k, j)
        };

        let slot_w = target.width.max(24.0);
        let slot_h = target.height.max(24.0);
        let target_w = slot_w.max(32.0);
        let target_y = target.y + slot_h * 0.5;
        let target_top = target_y - slot_h * 0.5;
        let expand_h = (target_y - window.y - window.height).max(10.0);
        let full_height = (target_y - window.y) - expand_h * (1.0 - k);
        let current_height = (full_height * (1.0 - j) + slot_h * j).max(1.0);
        let offset_y = (target_top - window.y) * j;
        let window_center_x = window.x + window.width * 0.5;
        let target_center_x = target.x + target_w * 0.5;
        let path_dx = target_center_x - window_center_x;
        let path_dy = target_y - (window.y + window.height * 0.5);
        let path_length = path_dx.hypot(path_dy).max(1.0);
        let bend_side = if path_dx.abs() > 1.0 {
            -path_dx.signum()
        } else {
            1.0
        };
        let bend_distance = (path_length * 0.12).clamp(0.0, 96.0);

        Self {
            window,
            full_height,
            current_height,
            offset_y,
            window_center_x,
            target_w,
            path_dx,
            path_dy,
            path_length,
            bend_side,
            bend_distance,
            k,
            j,
        }
    }

    pub fn deform_slice(&self, slice: f32) -> (f32, f32, f32) {
        let local_y = slice * self.current_height;
        let norm_y = if self.full_height > 1.0 {
            (local_y / self.full_height).clamp(0.0, 1.0)
        } else {
            slice
        };
        let path_progress = norm_y * norm_y * (3.0 - 2.0 * norm_y);
        let norm_x = (path_progress * self.k + self.j).min(1.0);
        let bend = (std::f32::consts::PI * path_progress).sin()
            * self.bend_distance
            * self.k
            * (1.0 - self.j)
            * self.bend_side;
        let current_center_x =
            self.window_center_x + self.path_dx * norm_x - (self.path_dy / self.path_length) * bend;
        let screen_y =
            self.window.y + local_y + self.offset_y + (self.path_dx / self.path_length) * bend;

        let curved_taper = (1.0 - self.k) + self.k * (1.0 - slice) * (1.0 - slice);
        let tapered_width = self.target_w + (self.window.width - self.target_w) * curved_taper;
        let slice_w = (tapered_width * (1.0 - self.j) + self.target_w * self.j).max(2.0);

        (screen_y, current_center_x - slice_w * 0.5, slice_w)
    }

    pub fn alpha(&self) -> f32 {
        if self.j > 0.98 {
            ((1.0 - self.j) / 0.02).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MagicLampAnimation;
    use crate::genie::GenieRect;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> GenieRect {
        GenieRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn minimizing_starts_at_window_geometry() {
        let window = rect(120.0, 80.0, 900.0, 700.0);
        let target = rect(1420.0, 930.0, 76.0, 54.0);
        let animation = MagicLampAnimation::new(window, target, 0.0, true);

        for slice in [0.0, 0.5, 1.0] {
            let (y, x, width) = animation.deform_slice(slice);
            assert!((y - (window.y + slice * window.height)).abs() < 0.001);
            assert!((x - window.x).abs() < 0.001);
            assert!((width - window.width).abs() < 0.001);
        }
        assert_eq!(animation.alpha(), 1.0);
    }

    #[test]
    fn minimizing_ends_at_dock_geometry_and_fades_out() {
        let window = rect(120.0, 80.0, 900.0, 700.0);
        let target = rect(1420.0, 930.0, 76.0, 54.0);
        let animation = MagicLampAnimation::new(window, target, 1.0, true);

        for slice in [0.0, 0.5, 1.0] {
            let (y, x, width) = animation.deform_slice(slice);
            assert!((y - (target.y + slice * target.height)).abs() < 0.001);
            assert!((x - target.x).abs() < 0.001);
            assert!((width - target.width).abs() < 0.001);
        }
        assert_eq!(animation.alpha(), 0.0);
    }

    #[test]
    fn minimizing_tapers_the_window_toward_the_dock() {
        let window = rect(120.0, 80.0, 900.0, 700.0);
        let target = rect(1420.0, 930.0, 76.0, 54.0);
        let animation = MagicLampAnimation::new(window, target, 0.2, true);

        let (top_y, _, top_width) = animation.deform_slice(0.0);
        let (bottom_y, _, bottom_width) = animation.deform_slice(1.0);

        assert!(bottom_width < top_width);
        assert!(bottom_y > window.y + window.height);
        assert!(top_y >= window.y);
    }

    #[test]
    fn restoring_starts_at_dock_and_ends_at_window_geometry() {
        let window = rect(120.0, 80.0, 900.0, 700.0);
        let target = rect(1420.0, 930.0, 76.0, 54.0);
        let at_dock = MagicLampAnimation::new(window, target, 0.0, false);
        let at_window = MagicLampAnimation::new(window, target, 1.0, false);

        for slice in [0.0, 0.5, 1.0] {
            let (dock_y, dock_x, dock_width) = at_dock.deform_slice(slice);
            assert!((dock_y - (target.y + slice * target.height)).abs() < 0.001);
            assert!((dock_x - target.x).abs() < 0.001);
            assert!((dock_width - target.width).abs() < 0.001);

            let (window_y, window_x, window_width) = at_window.deform_slice(slice);
            assert!((window_y - (window.y + slice * window.height)).abs() < 0.001);
            assert!((window_x - window.x).abs() < 0.001);
            assert!((window_width - window.width).abs() < 0.001);
        }
        assert_eq!(at_dock.alpha(), 0.0);
        assert_eq!(at_window.alpha(), 1.0);
    }
}
