//! Shared geometry for the music player and its expanded lyrics layout.
pub type Rect = (f32, f32, f32, f32);

#[derive(Clone, Copy, Debug)]
pub struct PlayerLayout {
    pub lyrics_button: Rect,
    pub title_width: f32,
    pub seek: Rect,
    pub remaining_x: f32,
    pub previous_x: f32,
    pub play_x: f32,
    pub next_x: f32,
    pub volume_icon: Rect,
    pub volume_bar: Rect,
}

impl PlayerLayout {
    pub fn new(origin: f32, lyrics: f32) -> Self {
        let t = lyrics.clamp(0., 1.);
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Self {
            lyrics_button: (origin + mix(35., 32.), 99., 36., 38.),
            title_width: mix(270., 140.),
            seek: (origin + mix(77., 76.), 80., mix(260., 108.), 22.),
            remaining_x: origin + mix(343., 190.),
            previous_x: origin + mix(145., 70.5),
            play_x: origin + mix(192.5, 108.5),
            next_x: origin + mix(240., 146.5),
            volume_icon: (
                origin + mix(377., 184.),
                mix(104., 99.),
                mix(19., 36.),
                mix(25., 38.),
            ),
            volume_bar: (
                origin + mix(327., 166.),
                mix(104., 142.),
                mix(46., 72.),
                mix(25., 24.),
            ),
        }
    }

    pub fn volume_track(&self) -> (f32, f32, f32) {
        let (x, y, width, _) = self.volume_bar;
        (x + 2., y + 13.5, width - 6.)
    }

    pub fn volume_at(&self, x: f32) -> f32 {
        let (left, _, width) = self.volume_track();
        ((x - left) / width).clamp(0., 1.)
    }

    pub fn seek_thumb(&self, progress: f32) -> (f32, f32) {
        let (x, _, width, _) = self.seek;
        (x + width * progress.clamp(0., 1.), 91.)
    }

    pub fn seek_at(&self, x: f32) -> f64 {
        let (seek_x, _, seek_width, _) = self.seek;
        if seek_width > 0. {
            (((x - seek_x) / seek_width).clamp(0., 1.)) as f64
        } else {
            0.
        }
    }

    pub fn width(lyrics_width: f32, lyrics: f32) -> f32 {
        420. + (lyrics_width - 420.) * lyrics.clamp(0., 1.)
    }

    pub fn lyrics_opacity(lyrics: f32) -> f32 {
        // Reveal the reader after the shared controls clear its left edge.
        (lyrics * 5. - 4.).clamp(0., 1.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_and_width_transition_preserve_music_and_lyrics_endpoints() {
        let music = PlayerLayout::new(20., 0.);
        let lyrics = PlayerLayout::new(20., 1.);
        assert_eq!(music.seek.1, lyrics.seek.1);
        assert_eq!(music.seek.3, lyrics.seek.3);
        assert_eq!(music.seek.2, 260.);
        assert_eq!(lyrics.seek.2, 108.);
        assert_eq!(PlayerLayout::width(680., 0.), 420.);
        assert_eq!(PlayerLayout::width(680., 1.), 680.);
    }

    #[test]
    fn volume_hit_targets_and_pointer_mapping_follow_the_visible_track() {
        for frame in 0..=100 {
            let layout = PlayerLayout::new(200., frame as f32 / 100.);
            let (x, y, width) = layout.volume_track();
            assert!(x >= layout.volume_bar.0);
            assert!(x + width <= layout.volume_bar.0 + layout.volume_bar.2);
            assert!(y > layout.volume_bar.1);
            assert!(y < layout.volume_bar.1 + layout.volume_bar.3);
            assert_eq!(layout.volume_at(x - 50.), 0.);
            assert!((layout.volume_at(x + width / 2.) - 0.5).abs() < 0.00001);
            assert_eq!(layout.volume_at(x + width + 50.), 1.);
        }
    }

    #[test]
    fn volume_icon_aligns_with_transport_and_slider_stays_clear_of_reader() {
        let layout = PlayerLayout::new(0., 1.);
        assert_eq!(layout.volume_icon.1, 99.);
        assert!(layout.volume_icon.0 >= layout.next_x + 34.5);
        assert!(layout.volume_bar.1 + layout.volume_bar.3 < 180.);
        assert!(layout.volume_bar.0 + layout.volume_bar.2 < 252.);
        assert!(layout.volume_bar.1 >= 137.);
    }

    #[test]
    fn reader_is_hidden_until_shared_volume_clears_its_edge() {
        for frame in 0..=100 {
            let t = frame as f32 / 100.;
            if PlayerLayout::lyrics_opacity(t) > 0. {
                let layout = PlayerLayout::new(0., t);
                assert!(layout.volume_bar.0 + layout.volume_bar.2 < 264.);
            }
        }
        assert_eq!(PlayerLayout::lyrics_opacity(0.), 0.);
        assert_eq!(PlayerLayout::lyrics_opacity(1.), 1.);
    }

    #[test]
    fn seek_at_maps_ratio_and_clamps() {
        let layout = PlayerLayout::new(20., 0.);
        let (x, _, w, _) = layout.seek;
        assert_eq!(layout.seek_at(x - 50.), 0.);
        assert_eq!(layout.seek_at(x), 0.);
        assert!((layout.seek_at(x + w * 0.5) - 0.5).abs() < 0.001);
        assert_eq!(layout.seek_at(x + w), 1.);
        assert_eq!(layout.seek_at(x + w + 50.), 1.);
    }

    #[test]
    fn seek_thumb_tracks_progress_and_clamps() {
        let layout = PlayerLayout::new(20., 0.);
        let (x0, y0) = layout.seek_thumb(0.);
        assert_eq!(x0, layout.seek.0);
        assert_eq!(y0, 91.);
        let (x1, y1) = layout.seek_thumb(1.);
        assert_eq!(x1, layout.seek.0 + layout.seek.2);
        assert_eq!(y1, 91.);
        let (x_half, _) = layout.seek_thumb(0.5);
        assert!((x_half - (layout.seek.0 + layout.seek.2 * 0.5)).abs() < 0.001);
        let (x_neg, _) = layout.seek_thumb(-0.5);
        assert_eq!(x_neg, x0);
        let (x_over, _) = layout.seek_thumb(1.5);
        assert_eq!(x_over, x1);
    }
}

/// Shared centers and adjacent 32 px targets for window, captions, options and close.
pub fn lyrics_header_action(width: f32, index: usize) -> Rect {
    (width - 132. + index.min(3) as f32 * 32., 6.5, 32., 32.)
}
#[cfg(test)]
mod header_alignment_tests {
    use super::*;
    #[test]
    fn header_controls_share_centers_and_do_not_overlap() {
        for width in [520., 680., 1200., 1800.] {
            let rects = (0..4)
                .map(|i| lyrics_header_action(width, i))
                .collect::<Vec<_>>();
            for r in &rects {
                assert_eq!(r.1 + r.3 / 2., 22.5);
                assert!(r.0 + r.2 <= width);
            }
            for pair in rects.windows(2) {
                assert_eq!(pair[0].0 + pair[0].2, pair[1].0);
            }
            assert_eq!(rects[3].0 + 16., width - 20.);
        }
    }
}

#[cfg(test)]
mod compact_controls_tests {
    use super::*;
    #[test]
    fn five_controls_have_equal_spacing_and_balanced_end_buttons() {
        let l = PlayerLayout::new(20., 1.);
        let centers = [
            l.lyrics_button.0 + 18.,
            l.previous_x + 17.5,
            l.play_x + 17.5,
            l.next_x + 17.5,
            l.volume_icon.0 + l.volume_icon.2 / 2.,
        ];
        for pair in centers.windows(2) {
            assert_eq!(pair[1] - pair[0], 38.);
        }
        assert_eq!(l.lyrics_button.2, l.volume_icon.2);
        assert_eq!(l.lyrics_button.3, l.volume_icon.3);
        assert_eq!(l.lyrics_button.1, l.volume_icon.1);
        assert!(l.volume_bar.1 >= l.volume_icon.1 + l.volume_icon.3);
    }
}
