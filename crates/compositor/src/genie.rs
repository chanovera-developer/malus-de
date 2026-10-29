#[derive(Clone, Copy)]
pub struct GenieRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub fn deform_slice(
    window: GenieRect,
    target: GenieRect,
    progress: f32,
    slice: f32,
) -> (f32, f32, f32) {
    let progress = progress.clamp(0.0, 1.0);
    let slice = slice.clamp(0.0, 1.0);
    let delay = (1.0 - slice) * 0.28;
    let local_progress = ((progress - delay) / (1.0 - delay)).clamp(0.0, 1.0);
    let eased_progress = local_progress * local_progress * (3.0 - 2.0 * local_progress);

    let window_center_x = window.x + window.width * 0.5;
    let target_center_x = target.x + target.width * 0.5;
    let path_dx = target_center_x - window_center_x;
    let path_dy = target.y + target.height * 0.5 - (window.y + window.height * 0.5);
    let path_length = path_dx.hypot(path_dy).max(1.0);
    let bend_side = if path_dx.abs() > 1.0 {
        -path_dx.signum()
    } else {
        1.0
    };
    let bend_distance = (path_length * 0.12).clamp(0.0, 96.0);
    let bend = (std::f32::consts::PI * eased_progress).sin() * bend_distance * bend_side;

    let center_x = window_center_x + path_dx * eased_progress - (path_dy / path_length) * bend;
    let start_y = window.y + slice * window.height;
    let target_y = target.y + slice * target.height;
    let screen_y = start_y + (target_y - start_y) * eased_progress + (path_dx / path_length) * bend;
    let width = (window.width + (target.width - window.width) * eased_progress).max(1.0);

    (screen_y, center_x - width * 0.5, width)
}

#[cfg(test)]
mod tests {
    use super::{deform_slice, GenieRect};

    #[test]
    fn genie_deformation_starts_at_the_window_geometry() {
        let window = GenieRect {
            x: 120.0,
            y: 80.0,
            width: 900.0,
            height: 700.0,
        };
        let target = GenieRect {
            x: 1420.0,
            y: 930.0,
            width: 76.0,
            height: 54.0,
        };

        for slice in [0.0, 0.5, 1.0] {
            let (y, x, width) = deform_slice(window, target, 0.0, slice);
            assert!((y - (window.y + slice * window.height)).abs() < 0.001);
            assert!((x - window.x).abs() < 0.001);
            assert!((width - window.width).abs() < 0.001);
        }
    }

    #[test]
    fn genie_deformation_ends_exactly_at_the_thumbnail_geometry() {
        let window = GenieRect {
            x: 120.0,
            y: 80.0,
            width: 900.0,
            height: 700.0,
        };
        let target = GenieRect {
            x: 1420.0,
            y: 930.0,
            width: 76.0,
            height: 54.0,
        };

        for slice in [0.0, 0.5, 1.0] {
            let (y, x, width) = deform_slice(window, target, 1.0, slice);
            assert!((y - (target.y + slice * target.height)).abs() < 0.001);
            assert!((x - target.x).abs() < 0.001);
            assert!((width - target.width).abs() < 0.001);
        }
    }
}
