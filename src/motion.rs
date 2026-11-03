//! Analytic critically damped motion, independent of frame cadence.

pub const FREQUENCY_SNAPPY: f32 = 28.0;
pub const FREQUENCY_DEFAULT: f32 = 22.0;
pub const FREQUENCY_GENTLE: f32 = 12.0;

pub fn equalizer_heights(phase: Option<f32>, playing: bool) -> [f32; 5] {
    if !playing {
        return [3.; 5];
    }
    let Some(phase) = phase else {
        return [7., 13., 9., 14., 6.];
    };
    std::array::from_fn(|index| {
        let wave = (phase * (2.8 + index as f32 * 0.31) + index as f32 * 1.7).sin();
        4. + 10. * (wave * 0.5 + 0.5)
    })
}

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub value: f32,
    velocity: f32,
}

impl Spring {
    pub fn new(value: f32) -> Self {
        Self {
            value,
            velocity: 0.,
        }
    }

    pub fn snap(&mut self, target: f32) {
        self.value = target;
        self.velocity = 0.;
    }

    /// Returns true while another frame is needed. Retargeting preserves velocity.
    pub fn advance(&mut self, target: f32, dt: f32, frequency: f32) -> bool {
        let offset = self.value - target;
        let momentum = self.velocity + frequency * offset;
        let decay = (-frequency * dt.max(0.)).exp();
        self.value = target + (offset + momentum * dt.max(0.)) * decay;
        self.velocity = (self.velocity - frequency * momentum * dt.max(0.)) * decay;
        if (self.value - target).abs() < 0.0005 && self.velocity.abs() < 0.005 {
            self.snap(target);
            false
        } else {
            true
        }
    }

    pub fn advance_snappy(&mut self, target: f32, dt: f32) -> bool {
        self.advance(target, dt, FREQUENCY_SNAPPY)
    }

    pub fn advance_default(&mut self, target: f32, dt: f32) -> bool {
        self.advance(target, dt, FREQUENCY_DEFAULT)
    }

    pub fn advance_gentle(&mut self, target: f32, dt: f32) -> bool {
        self.advance(target, dt, FREQUENCY_GENTLE)
    }

    /// Advance with automatic snap when reduced_motion is active.
    pub fn step(&mut self, target: f32, dt: f32, frequency: f32, reduced_motion: bool) -> bool {
        if reduced_motion {
            self.snap(target);
            false
        } else {
            self.advance(target, dt, frequency)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playing_bars_vary_but_stay_inside_the_icon_and_pause_is_static() {
        assert_ne!(
            equalizer_heights(Some(0.), true),
            equalizer_heights(Some(0.5), true)
        );
        for frame in 0..500 {
            assert!(equalizer_heights(Some(frame as f32 / 20.), true)
                .iter()
                .all(|height| (4. ..=14.).contains(height)));
            assert_eq!(equalizer_heights(Some(frame as f32), false), [3.; 5]);
        }
        assert_eq!(equalizer_heights(None, true), [7., 13., 9., 14., 6.]);
    }

    #[test]
    fn cadence_does_not_change_trajectory() {
        let mut a = Spring::new(0.);
        let mut b = a;
        for _ in 0..24 {
            a.advance(1., 1. / 120., 22.);
        }
        for _ in 0..6 {
            b.advance(1., 1. / 30., 22.);
        }
        assert!((a.value - b.value).abs() < 0.00001);
    }

    #[test]
    fn reversal_and_long_pause_settle() {
        let mut spring = Spring::new(0.);
        spring.advance(1., 0.08, 22.);
        let before = spring.value;
        spring.advance(0., 0.001, 26.);
        assert!((spring.value - before).abs() < 0.01);
        assert!(!spring.advance(0., 5., 26.));
        assert_eq!(spring.value, 0.);
    }

    #[test]
    fn spring_presets_and_reduced_motion() {
        let mut snappy = Spring::new(0.);
        snappy.advance_snappy(1., 0.05);
        assert!(snappy.value > 0.);

        let mut gentle = Spring::new(0.);
        gentle.advance_gentle(1., 0.05);
        assert!(gentle.value > 0.);

        let mut reduced = Spring::new(0.);
        let active = reduced.step(1., 0.05, FREQUENCY_DEFAULT, true);
        assert!(!active);
        assert_eq!(reduced.value, 1.);
    }
}

/// Reveal the edge handle at the end of the surface contraction.
pub fn edge_hint_opacity(height: f32) -> f32 {
    let t = ((30. - height) / 20.).clamp(0., 1.);
    t * t * (3. - 2. * t)
}

#[cfg(test)]
mod edge_hint_tests {
    #[test]
    fn handle_appears_monotonically_only_near_final_height() {
        assert_eq!(super::edge_hint_opacity(140.), 0.);
        assert_eq!(super::edge_hint_opacity(30.), 0.);
        assert_eq!(super::edge_hint_opacity(20.), 0.5);
        assert_eq!(super::edge_hint_opacity(10.), 1.);
        let mut last = 0.;
        for height in (10..=140).rev() {
            let opacity = super::edge_hint_opacity(height as f32);
            assert!(opacity >= last);
            last = opacity;
        }
    }
}
