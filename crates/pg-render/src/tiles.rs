//! Turning a tile map into rectangles to paint.

use crate::camera::Camera;

pub type Rgb = [u8; 3];

/// A filled rectangle in view pixels.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: Rgb,
}

/// Where tile colours come from.
pub trait TileSource {
    fn width(&self) -> i32;
    fn height(&self) -> i32;
    fn color(&self, x: i32, y: i32) -> Rgb;
}

/// The tile range `x0..x1`, `y0..y1` (end exclusive) that can be seen, clamped to the map.
pub fn visible_range(
    cam: &Camera,
    view_w: f32,
    view_h: f32,
    map_w: i32,
    map_h: i32,
) -> (i32, i32, i32, i32) {
    let (left, top) = cam.to_world(0.0, 0.0, view_w, view_h);
    let (right, bottom) = cam.to_world(view_w, view_h, view_w, view_h);
    (
        (left.floor() as i32).clamp(0, map_w),
        (top.floor() as i32).clamp(0, map_h),
        (right.ceil() as i32 + 1).clamp(0, map_w),
        (bottom.ceil() as i32 + 1).clamp(0, map_h),
    )
}

/// Rectangles for the visible tiles. Runs of equal colour along a row become one rectangle, so open grass
/// costs a handful of shapes instead of thousands.
pub fn tile_rects(cam: &Camera, view_w: f32, view_h: f32, src: &dyn TileSource) -> Vec<Rect> {
    let (x0, y0, x1, y1) = visible_range(cam, view_w, view_h, src.width(), src.height());
    let mut out = Vec::new();
    for y in y0..y1 {
        let mut x = x0;
        while x < x1 {
            let c = src.color(x, y);
            let mut end = x + 1;
            while end < x1 && src.color(end, y) == c {
                end += 1;
            }
            let (sx, sy) = cam.to_screen(x as f32, y as f32, view_w, view_h);
            out.push(Rect {
                x: sx,
                y: sy,
                w: (end - x) as f32 * cam.zoom,
                h: cam.zoom,
                color: c,
            });
            x = end;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stripes;

    impl TileSource for Stripes {
        fn width(&self) -> i32 {
            40
        }
        fn height(&self) -> i32 {
            30
        }
        fn color(&self, x: i32, y: i32) -> Rgb {
            if y == 5 && (10..15).contains(&x) {
                [1, 2, 3]
            } else {
                [9, 9, 9]
            }
        }
    }

    #[test]
    fn runs_of_one_colour_become_one_rectangle() {
        let cam = Camera::fit(40, 30, 800.0, 600.0);
        let rects = tile_rects(&cam, 800.0, 600.0, &Stripes);
        // 30 rows; row 5 has three runs, the others one.
        assert_eq!(rects.len(), 30 + 2);
        let special = rects.iter().find(|r| r.color == [1, 2, 3]).unwrap();
        assert_eq!(special.w, 5.0 * cam.zoom);
        assert_eq!(special.h, cam.zoom);
    }

    #[test]
    fn only_the_visible_part_of_a_big_map_is_drawn() {
        struct Big;
        impl TileSource for Big {
            fn width(&self) -> i32 {
                2000
            }
            fn height(&self) -> i32 {
                2000
            }
            fn color(&self, x: i32, _: i32) -> Rgb {
                [(x % 2) as u8, 0, 0]
            }
        }
        let cam = Camera {
            cx: 1000.0,
            cy: 1000.0,
            zoom: 20.0,
        };
        let rects = tile_rects(&cam, 800.0, 600.0, &Big);
        assert!(rects.len() < 40 * 32 + 64, "{}", rects.len());
        let (x0, y0, x1, y1) = visible_range(&cam, 800.0, 600.0, 2000, 2000);
        assert!(x1 - x0 <= 43 && y1 - y0 <= 33, "{x0}..{x1} {y0}..{y1}");
        // Off the map: nothing.
        let off = Camera {
            cx: -500.0,
            cy: -500.0,
            zoom: 10.0,
        };
        assert!(tile_rects(&off, 800.0, 600.0, &Big).is_empty());
    }
}
