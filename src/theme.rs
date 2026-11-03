//! Nivumi desktop tokens. Colors are RGB; convert only at Win32 boundaries.
pub const SURFACE: u32 = 0x1c1c1e;
pub const CARD: u32 = 0x262628;
pub const INPUT: u32 = 0x343436;
pub const TEXT: u32 = 0xffffff;
pub const MUTED: u32 = 0xb7b7bf;
pub const ACCENT: u32 = 0x30d158;
pub const SELECTED: u32 = 0x244a32;
pub const PRESSED: u32 = 0x305c40;
pub const CAPTION: i32 = 12;
pub const BODY: i32 = 16;
pub const TITLE: i32 = 24;
pub const SPACE: i32 = 8;
pub const ROW_HEIGHT: i32 = 56;
pub const ROW_PITCH: i32 = ROW_HEIGHT + SPACE;
pub const fn colorref(rgb: u32) -> u32 {
    ((rgb >> 16) & 255) | (rgb & 0xff00) | ((rgb & 255) << 16)
}

// Lyrics surfaces and emphasis are shared by the notch and detached reader.
pub const LYRICS_SURFACE: u32 = 0x000000;
pub const LYRICS_TEXT: u32 = 0xffffff;
pub const LYRICS_MUTED: u32 = 0x9a9aa4;
pub const LYRICS_TRANSLATION: u32 = 0x92aab8;
pub const LYRICS_TRANSLATION_ACTIVE: u32 = 0x38bdf8;
pub const LYRICS_PILL: u32 = 0x141414;

// Semantic colors, shared by all desktop surfaces.
pub const BORDER: u32 = 0x55555d;
pub const POSITIVE: u32 = 0x30d158;
pub const WARNING: u32 = 0xffc94d;
pub const ERROR: u32 = 0xff656b;
pub const ERROR_SURFACE: u32 = 0x47191d;
pub const POSITIVE_SURFACE: u32 = 0x073d29;

pub const ACCENT_HOVER: u32 = 0x64e885;
pub const ACCENT_PRESSED: u32 = 0x30b950;

/// Retain the artwork hue while reserving contrast for secondary text and the light wave.
pub fn artwork_accent(accent: [f32; 3]) -> [f32; 3] {
    let clean = accent.map(|v| if v.is_finite() { v.clamp(0., 1.) } else { 0. });
    let peak = clean.into_iter().fold(0f32, f32::max);
    let gain = if peak > 0.20 { 0.20 / peak } else { 1. };
    clean.map(|v| v * gain)
}

// Original per-feature highlights, retained independently of the Nivumi monogram.
pub const NOTCH_SURFACE: u32 = 0x000000;
pub const FOCUS: u32 = 0xff4b55;
pub const BLUE: u32 = 0x38bdf8;
pub const CALENDAR_SELECTED: u32 = 0x276ff0;
pub const BLUE_SURFACE: u32 = 0x0e4a86;

#[cfg(test)]
mod contrast_tests {
    use super::*;
    fn luminance(rgb: u32) -> f64 {
        [16, 8, 0]
            .into_iter()
            .zip([0.2126, 0.7152, 0.0722])
            .map(|(shift, weight)| {
                let c = ((rgb >> shift) & 255) as f64 / 255.;
                weight
                    * if c <= 0.04045 {
                        c / 12.92
                    } else {
                        ((c + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum()
    }
    #[test]
    fn artwork_remains_readable_without_changing_its_hue() {
        let c = artwork_accent([1., 0.5, 0.25]);
        assert_eq!(c, [0.20, 0.10, 0.05]);
        assert_eq!(artwork_accent([f32::NAN, -1., 0.]), [0.; 3]);
        // Maximum neutral artwork plus a 32%-alpha, 1.6x radial light wave.
        for fg in [MUTED, TEXT, LYRICS_TRANSLATION_ACTIVE] {
            assert!((luminance(fg) + 0.05) / (luminance(0x3d3d3d) + 0.05) >= 4.5);
        }
    }
    #[test]
    fn theme_text_and_primary_buttons_meet_normal_text_contrast() {
        for bg in [SURFACE, CARD, INPUT] {
            for fg in [TEXT, MUTED, ACCENT, POSITIVE] {
                assert!(
                    (luminance(fg) + 0.05) / (luminance(bg) + 0.05) >= 4.5,
                    "{fg:06x} on {bg:06x}"
                );
            }
        }
        for bg in [ACCENT, ACCENT_HOVER, ACCENT_PRESSED, POSITIVE] {
            assert!((luminance(bg) + 0.05) / (luminance(SURFACE) + 0.05) >= 4.5);
        }
    }
}
