//! Keep the native input region in sync with the DirectComposition surface.
//! HTTRANSPARENT alone only forwards input between windows on the same thread.
use windows::Win32::{Foundation::*, Graphics::Gdi::*};

#[derive(Default)]
pub struct WindowRegion {
    applied: Vec<(i32, i32)>,
}

impl WindowRegion {
    /// # Safety
    /// Call on the owning UI thread with the current physical-pixel scale.
    pub unsafe fn update(
        &mut self,
        hwnd: HWND,
        scale: f32,
        outline: &[(f32, f32)],
        overlays: &[(f32, f32, f32, f32, f32)],
    ) {
        let mut pixels: Vec<_> = outline
            .iter()
            .map(|&(x, y)| ((x * scale).round() as i32, (y * scale).round() as i32))
            .collect();
        let outline_len = pixels.len();
        for &(x, y, w, h, r) in overlays {
            pixels.extend([
                ((x * scale).floor() as i32, (y * scale).floor() as i32),
                (
                    ((x + w) * scale).ceil() as i32,
                    ((y + h) * scale).ceil() as i32,
                ),
                ((r * 2. * scale).round() as i32, 0),
            ]);
        }
        if pixels == self.applied {
            return;
        }
        let points: Vec<_> = pixels[..outline_len]
            .iter()
            .map(|&(x, y)| POINT { x, y })
            .collect();
        let region = CreatePolygonRgn(&points, WINDING);
        if region.is_invalid() {
            return;
        }
        for bounds in pixels[outline_len..].as_chunks::<3>().0 {
            let overlay = CreateRoundRectRgn(
                bounds[0].0,
                bounds[0].1,
                bounds[1].0,
                bounds[1].1,
                bounds[2].0,
                bounds[2].0,
            );
            let combined = if overlay.is_invalid() {
                false
            } else {
                let ok = CombineRgn(Some(region), Some(region), Some(overlay), RGN_OR) != RGN_ERROR;
                let _ = DeleteObject(overlay.into());
                ok
            };
            if !combined {
                let _ = DeleteObject(region.into());
                return;
            }
        }
        if SetWindowRgn(hwnd, Some(region), true) != 0 {
            // Ownership passes to Windows only on success.
            self.applied = pixels;
        } else {
            let _ = DeleteObject(region.into());
        }
    }
}

/// Same cubic contour as Renderer::notch, flattened for the native region.
pub fn notch_outline(x: f32, w: f32, h: f32, r: f32, ear: f32) -> Vec<(f32, f32)> {
    let mut points = vec![(x - ear, 0.)];
    curve(
        &mut points,
        [
            (x - ear, 0.),
            (x - ear * 0.45, 0.),
            (x, ear * 0.45),
            (x, ear),
        ],
    );
    points.push((x, h - r));
    curve(
        &mut points,
        [(x, h - r), (x, h - r * 0.45), (x + r * 0.45, h), (x + r, h)],
    );
    points.push((x + w - r, h));
    curve(
        &mut points,
        [
            (x + w - r, h),
            (x + w - r * 0.45, h),
            (x + w, h - r * 0.45),
            (x + w, h - r),
        ],
    );
    points.push((x + w, ear));
    curve(
        &mut points,
        [
            (x + w, ear),
            (x + w, ear * 0.45),
            (x + w + ear * 0.45, 0.),
            (x + w + ear, 0.),
        ],
    );
    points
}

fn curve(points: &mut Vec<(f32, f32)>, controls: [(f32, f32); 4]) {
    for step in 1..=16 {
        let t = step as f32 / 16.;
        let u = 1. - t;
        let weights = [u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t];
        points.push(
            controls
                .iter()
                .zip(weights)
                .fold((0., 0.), |(x, y), (p, a)| (x + p.0 * a, y + p.1 * a)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{core::w, Win32::UI::WindowsAndMessaging::*};

    #[test]
    fn window_region_tracks_collapse_and_removes_expired_overlays() {
        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Region test"),
                WS_POPUP,
                0,
                0,
                580,
                380,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let mut clip = WindowRegion::default();
            let probe = CreateRectRgn(0, 0, 0, 0);
            for scale in [1., 1.5, 2.] {
                let outline = notch_outline(80., 420., 140., 32., 22.);
                clip.update(hwnd, scale, &outline, &[]);
                assert_ne!(GetWindowRgn(hwnd, probe), RGN_ERROR);
                assert!(PtInRegion(probe, (290. * scale) as i32, (100. * scale) as i32).as_bool());

                let outline = notch_outline(215., 150., 30., 14., 10.);
                clip.update(hwnd, scale, &outline, &[(135., 40., 310., 46., 18.)]);
                assert_ne!(GetWindowRgn(hwnd, probe), RGN_ERROR);
                assert!(PtInRegion(probe, (290. * scale) as i32, (60. * scale) as i32).as_bool());
                assert!(!PtInRegion(probe, (290. * scale) as i32, (35. * scale) as i32).as_bool());

                // Repeat to exercise the unchanged-frame cache, then expire the toast.
                clip.update(hwnd, scale, &outline, &[(135., 40., 310., 46., 18.)]);
                clip.update(hwnd, scale, &outline, &[]);
                assert_ne!(GetWindowRgn(hwnd, probe), RGN_ERROR);
                assert!(PtInRegion(probe, (290. * scale) as i32, (15. * scale) as i32).as_bool());
                assert!(!PtInRegion(probe, (290. * scale) as i32, (60. * scale) as i32).as_bool());
                assert!(!PtInRegion(probe, (290. * scale) as i32, (100. * scale) as i32).as_bool());
            }
            assert!(DeleteObject(probe.into()).as_bool());
            DestroyWindow(hwnd).unwrap();
        }
    }

    #[test]
    fn minimized_contour_stays_attached_to_top_with_flared_corners() {
        let (radius, ear) = crate::fullscreen::edge_hint_corners(crate::fullscreen::EDGE_HEIGHT);
        assert!(ear > 0.);
        let outline = notch_outline(
            100.,
            crate::fullscreen::EDGE_WIDTH,
            crate::fullscreen::EDGE_HEIGHT,
            radius,
            ear,
        );
        assert_eq!(outline.first().copied(), Some((100. - ear, 0.)));
        assert_eq!(outline.last().copied(), Some((196. + ear, 0.)));
        assert!(outline.iter().all(|&(_, y)| (0. ..=10.).contains(&y)));
        assert!(outline.iter().any(|&(x, y)| x < 100. && y > 0.));
        assert!(outline.iter().any(|&(x, y)| x > 196. && y > 0.));
    }

    #[test]
    fn native_region_releases_the_expanded_area_at_every_scale() {
        unsafe {
            for scale in [1., 1.25, 1.5, 2.] {
                for (w, h, r, ear) in [
                    (
                        crate::fullscreen::EDGE_WIDTH,
                        crate::fullscreen::EDGE_HEIGHT,
                        5.,
                        5.,
                    ),
                    (150., 30., 14., 10.),
                    (112., 18., 14., 10.),
                    (312., 36., 14., 10.),
                    (420., 140., 32., 22.),
                    (520., 140., 32., 22.),
                    (420., 286., 32., 22.),
                ] {
                    let points: Vec<_> = notch_outline((580. - w) / 2., w, h, r, ear)
                        .iter()
                        .map(|&(x, y)| POINT {
                            x: (x * scale).round() as i32,
                            y: (y * scale).round() as i32,
                        })
                        .collect();
                    let region = CreatePolygonRgn(&points, WINDING);
                    assert!(!region.is_invalid());
                    assert!(
                        PtInRegion(region, (290. * scale) as i32, (h * scale / 2.) as i32)
                            .as_bool()
                    );
                    assert!(
                        !PtInRegion(region, (290. * scale) as i32, ((h + 5.) * scale) as i32)
                            .as_bool()
                    );
                    assert!(
                        !PtInRegion(region, (20. * scale) as i32, (10. * scale) as i32).as_bool()
                    );
                    assert!(DeleteObject(region.into()).as_bool());
                }
            }
        }
    }
}
