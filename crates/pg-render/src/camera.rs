//! A 2D camera over a tile map: where the view is centred (in tiles) and how many pixels a tile covers.

/// Pixels per tile the camera allows.
pub const MIN_ZOOM: f32 = 2.0;
pub const MAX_ZOOM: f32 = 96.0;

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Camera {
    /// The tile coordinate at the middle of the view (fractional).
    pub cx: f32,
    pub cy: f32,
    /// Pixels per tile.
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Camera {
        Camera {
            cx: 0.0,
            cy: 0.0,
            zoom: 16.0,
        }
    }
}

impl Camera {
    /// A camera that shows the whole map in a view of `view_w` by `view_h` pixels.
    pub fn fit(map_w: i32, map_h: i32, view_w: f32, view_h: f32) -> Camera {
        let (mw, mh) = (map_w.max(1) as f32, map_h.max(1) as f32);
        let zoom = (view_w / mw).min(view_h / mh).clamp(MIN_ZOOM, MAX_ZOOM);
        Camera {
            cx: mw / 2.0,
            cy: mh / 2.0,
            zoom,
        }
    }

    /// Screen position (pixels from the view's top-left) of a tile coordinate.
    pub fn to_screen(&self, tx: f32, ty: f32, view_w: f32, view_h: f32) -> (f32, f32) {
        (
            view_w / 2.0 + (tx - self.cx) * self.zoom,
            view_h / 2.0 + (ty - self.cy) * self.zoom,
        )
    }

    /// The tile coordinate under a screen position.
    pub fn to_world(&self, sx: f32, sy: f32, view_w: f32, view_h: f32) -> (f32, f32) {
        (
            self.cx + (sx - view_w / 2.0) / self.zoom,
            self.cy + (sy - view_h / 2.0) / self.zoom,
        )
    }

    /// Drags the map by a pixel amount.
    pub fn pan_pixels(&mut self, dx: f32, dy: f32) {
        self.cx -= dx / self.zoom;
        self.cy -= dy / self.zoom;
    }

    /// Zooms by `factor`, keeping the tile under the cursor where it is.
    pub fn zoom_at(&mut self, sx: f32, sy: f32, factor: f32, view_w: f32, view_h: f32) {
        let before = self.to_world(sx, sy, view_w, view_h);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.to_world(sx, sy, view_w, view_h);
        self.cx += before.0 - after.0;
        self.cy += before.1 - after.1;
    }

    /// Keeps the centre inside the map, so the map cannot be dragged out of sight.
    pub fn clamp_to(&mut self, map_w: i32, map_h: i32) {
        self.cx = self.cx.clamp(0.0, map_w.max(1) as f32);
        self.cy = self.cy.clamp(0.0, map_h.max(1) as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_shows_the_whole_map_centred() {
        let c = Camera::fit(64, 48, 1280.0, 720.0);
        assert_eq!((c.cx, c.cy), (32.0, 24.0));
        assert_eq!(c.zoom, 15.0, "720 / 48 limits the zoom");
        let (x, y) = c.to_screen(0.0, 0.0, 1280.0, 720.0);
        assert!(
            x >= 0.0 && y >= -0.01,
            "the map's corner is inside the view: {x},{y}"
        );
        let (x2, y2) = c.to_screen(64.0, 48.0, 1280.0, 720.0);
        assert!(x2 <= 1280.0 && y2 <= 720.01);
    }

    #[test]
    fn screen_and_world_are_inverses() {
        let c = Camera {
            cx: 10.5,
            cy: 7.25,
            zoom: 20.0,
        };
        for (sx, sy) in [(0.0, 0.0), (640.0, 360.0), (1000.0, 123.0)] {
            let (wx, wy) = c.to_world(sx, sy, 1280.0, 720.0);
            let (bx, by) = c.to_screen(wx, wy, 1280.0, 720.0);
            assert!((bx - sx).abs() < 1e-3 && (by - sy).abs() < 1e-3);
        }
    }

    #[test]
    fn zooming_keeps_the_tile_under_the_cursor() {
        let mut c = Camera {
            cx: 20.0,
            cy: 20.0,
            zoom: 16.0,
        };
        let (sx, sy) = (900.0, 200.0);
        let before = c.to_world(sx, sy, 1280.0, 720.0);
        c.zoom_at(sx, sy, 1.5, 1280.0, 720.0);
        let after = c.to_world(sx, sy, 1280.0, 720.0);
        assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3);
        assert_eq!(c.zoom, 24.0);
        c.zoom_at(sx, sy, 1000.0, 1280.0, 720.0);
        assert_eq!(c.zoom, MAX_ZOOM);
        c.zoom_at(sx, sy, 0.0001, 1280.0, 720.0);
        assert_eq!(c.zoom, MIN_ZOOM);
    }

    #[test]
    fn panning_and_clamping() {
        let mut c = Camera {
            cx: 10.0,
            cy: 10.0,
            zoom: 10.0,
        };
        c.pan_pixels(50.0, -20.0);
        assert_eq!(
            (c.cx, c.cy),
            (5.0, 12.0),
            "dragging right moves the map right, so the centre moves left"
        );
        c.pan_pixels(-100000.0, 100000.0);
        c.clamp_to(40, 30);
        assert_eq!((c.cx, c.cy), (40.0, 0.0));
    }
}
