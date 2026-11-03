// Geometry in physical pixels; content and canvas dimensions are DIPs.
pub fn origin(
    area: (i32, i32, i32, i32),
    canvas: f32,
    body: (f32, f32),
    scale: f32,
    column: u8,
) -> (i32, i32) {
    let (left, top, right, _) = area;
    let width = body.0 * scale;
    let margin = (24. * scale).min(((right - left) as f32 - width).max(0.) / 2.);
    let body_x = match column {
        0 => left as f32 + margin,
        2 => right as f32 - margin - width,
        _ => (left + right) as f32 / 2. - width / 2.,
    };
    (
        (body_x - (canvas - body.0) * scale / 2.).round() as i32,
        top,
    )
}

pub fn nearest(pointer: (i32, i32), area: (i32, i32, i32, i32)) -> u8 {
    let (left, _, right, _) = area;
    let third = ((right - left) / 3).max(1);
    ((pointer.0 - left) / third).clamp(0, 2) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_on_offset_monitor_remain_at_top() {
        let area = (-1920, 40, 0, 1040);
        for column in 0..3 {
            for pointer_y in [40, 200, 900, 1200] {
                let pointer = (-1600 + column as i32 * 640, pointer_y);
                assert_eq!(nearest(pointer, area), column);
            }
            let (x, y) = origin(area, 740., (420., 170.), 1.5, column);
            let body_left = x as f32 + (740. - 420.) * 1.5 / 2.;
            assert!(body_left >= -1920. && body_left + 420. * 1.5 <= 0.);
            assert_eq!(y, 40);
        }
    }

    #[test]
    fn top_remains_anchored_when_expanding() {
        let area = (0, 0, 1920, 1040);
        let (_, small) = origin(area, 740., (150., 30.), 1., 1);
        let (_, large) = origin(area, 740., (680., 240.), 1., 1);
        assert_eq!(small, 0);
        assert_eq!(large, 0);
        assert_eq!(origin(area, 740., (420., 170.), 1., 1).1, 0);
    }
}
