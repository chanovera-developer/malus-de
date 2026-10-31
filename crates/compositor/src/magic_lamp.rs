use crate::genie::GenieRect;

pub struct MagicLampAnimation {
    window: GenieRect,
    target: GenieRect,
    k: f32,
    j: f32,
}

#[inline]
fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

#[inline]
fn smootherstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

impl MagicLampAnimation {
    pub fn new(
        window: GenieRect,
        target: GenieRect,
        raw_progress: f32,
        is_minimizing: bool,
    ) -> Self {
        let raw_progress = raw_progress.clamp(0.0, 1.0);

        // En macOS Genie:
        // - Fase 1 (k: 0 -> 1, j = 0): el borde inferior se estrecha y desciende en curva S hasta tocar el icono del Dock.
        // - Fase 2 (k = 1, j: 0 -> 1): la ventana se desliza y comprime a través del embudo espacial estacionario hacia el Dock.
        // Al restaurar (is_minimizing = false), se recorre exactamente la evolución inversa (1.0 - raw_progress).
        let directed = if is_minimizing {
            raw_progress
        } else {
            1.0 - raw_progress
        };

        // Curva temporal suave que da peso al inicio de la succión y acelera el paso por el cuello
        let split = 0.34;
        let (k, j) = if directed <= split {
            let p = (directed / split).clamp(0.0, 1.0);
            let eased_k = 1.0 - (1.0 - p).powf(2.1);
            (eased_k, 0.0)
        } else {
            let p = ((directed - split) / (1.0 - split)).clamp(0.0, 1.0);
            // Combinación de arranque progresivo y succión fluida hacia el final
            let eased_j = smoothstep(p) * 0.45 + p.powf(1.35) * 0.55;
            (1.0, eased_j.clamp(0.0, 1.0))
        };

        Self {
            window,
            target: GenieRect {
                x: target.x,
                y: target.y,
                width: target.width.max(24.0),
                height: target.height.max(24.0),
            },
            k,
            j,
        }
    }

    pub fn deform_slice(&self, slice: f32) -> (f32, f32, f32) {
        let s = slice.clamp(0.0, 1.0);

        let win_top = self.window.y;
        let win_bottom = self.window.y + self.window.height;
        let dock_top = self.target.y;
        let dock_bottom = self.target.y + self.target.height;

        // 1. Posición vertical de la rebanada `s`:
        // Durante Fase 1 (k: 0 -> 1, j = 0), el borde superior sigue en win_top y el inferior baja de win_bottom a dock_bottom.
        // Durante Fase 2 (k = 1, j: 0 -> 1), el borde superior baja de win_top a dock_top mientras las rebanadas
        // se comprimen progresivamente al entrar en el cuello del embudo.
        let top_y = win_top + (dock_top - win_top) * self.j.powf(1.12);
        let reach = self.k.max(self.j);
        let bottom_y = win_bottom + (dock_bottom - win_bottom) * reach;

        // Ligera aceleración vertical de las rebanadas inferiores hacia el cuello durante la succión
        let suction_warp = s + self.k * (1.0 - self.j) * 0.22 * s * (1.0 - s);
        let screen_y = top_y + suction_warp * (bottom_y - top_y);

        // 2. Coordenada espacial normalizada dentro del embudo vertical [win_top .. dock_bottom]
        let total_span = (dock_bottom - win_top).abs().max(1.0);
        let spatial_u = ((screen_y - win_top) / total_span).clamp(0.0, 1.0);

        // El hombro superior del embudo acompaña el descenso de la ventana durante la primera parte de `j`,
        // manteniendo la parte alta ancha antes de estrecharse rápidamente al acercarse al Dock.
        let neck_start = (self.j * 0.52).clamp(0.0, 0.72);
        let funnel_t = if self.j > 0.0 {
            ((spatial_u - neck_start) / (1.0 - neck_start).max(0.05)).clamp(0.0, 1.0)
        } else {
            s
        };

        // Combinar la coordenada espacial del embudo con la coordenada propia de la rebanada
        // para garantizar convergencia exacta en los extremos (s = 0 y j = 1).
        let tau = (funnel_t * (1.0 - self.j) + self.j).clamp(0.0, 1.0);

        // 3. Curva de estrechamiento (Pinch) y curva en S de desplazamiento horizontal (Sweep):
        // - `pinch_curve` estrecha el ancho más rápido en la mitad inferior para formar el cuello esbelto de macOS.
        // - `sweep_curve` usa una quíntica C2 (derivadas 1ª y 2ª nulas en 0 y 1) para que los bordes laterales
        //   salgan verticales de la ventana, tracen una S suave y entren verticales al icono del Dock.
        let base_s = smoothstep(tau);
        let pinch_curve = (1.0 - (1.0 - base_s).powf(1.85)) * self.k;
        let sweep_curve = smootherstep(tau) * self.k;

        // Colapso final hacia el ancho y centro exactos del slot a medida que j -> 1
        let width_factor = (pinch_curve + (1.0 - pinch_curve) * self.j).clamp(0.0, 1.0);
        let center_factor = (sweep_curve + (1.0 - sweep_curve) * self.j).clamp(0.0, 1.0);

        let slice_w = (self.window.width + (self.target.width - self.window.width) * width_factor)
            .max(2.0);

        let win_center_x = self.window.x + self.window.width * 0.5;
        let target_center_x = self.target.x + self.target.width * 0.5;
        let current_center_x = win_center_x + (target_center_x - win_center_x) * center_factor;

        (screen_y, current_center_x - slice_w * 0.5, slice_w)
    }

    pub fn alpha(&self) -> f32 {
        if self.j > 0.92 {
            ((1.0 - self.j) / 0.08).clamp(0.0, 1.0)
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
