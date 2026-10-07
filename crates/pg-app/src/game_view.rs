//! The in-game map view: tiles and pawns, with pan and zoom.
//!
//! The geometry comes from `pg-render` (camera, run-merged tile rectangles) and the colours from the shared
//! palette; this module only paints them through egui and handles the mouse.

use egui::{Color32, Pos2, Sense};
use pg_core::id::EntityId;
use pg_core::map::{MapData, Tile};
use pg_render::{tile_rects, Camera, Rgb, TileSource};
use pg_runtime::snapshot::RenderSnapshot;
use pg_ui_model::palette::{pawn_color, tile_color};

struct MapColors<'a>(&'a MapData);

impl TileSource for MapColors<'_> {
    fn width(&self) -> i32 {
        self.0.width()
    }

    fn height(&self) -> i32 {
        self.0.height()
    }

    fn color(&self, x: i32, y: i32) -> Rgb {
        tile_color(self.0, Tile::new(x, y))
    }
}

fn c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

#[derive(Default)]
pub struct GameView {
    camera: Camera,
    fitted: Option<EntityId>,
    /// The player moved the camera: stop re-framing the map as the window changes.
    moved: bool,
}

impl GameView {
    /// Forgets the camera, so the next world is framed from scratch.
    pub fn reset(&mut self) {
        self.fitted = None;
        self.moved = false;
    }

    /// Paints the first map of `snap` into the space `ui` has left.
    pub fn show(&mut self, ui: &mut egui::Ui, snap: &RenderSnapshot) {
        let size = ui.available_size();
        let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let rect = response.rect;
        painter.rect_filled(rect, 0.0, Color32::from_rgb(20, 24, 30));
        let Some(map) = snap.maps.first() else {
            return;
        };
        let (vw, vh) = (rect.width(), rect.height());
        if (self.fitted != Some(map.id) || !self.moved) && vw > 1.0 && vh > 1.0 {
            self.camera = Camera::fit(map.width(), map.height(), vw, vh);
            self.fitted = Some(map.id);
        }
        // Drag to pan, wheel to zoom around the cursor.
        if response.dragged() {
            let d = response.drag_delta();
            self.camera.pan_pixels(d.x, d.y);
            self.moved = true;
        }
        if let Some(hover) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let factor = (scroll * 0.0025).exp();
                self.camera
                    .zoom_at(hover.x - rect.min.x, hover.y - rect.min.y, factor, vw, vh);
                self.moved = true;
            }
        }
        self.camera.clamp_to(map.width(), map.height());

        let painter = painter.with_clip_rect(rect);
        for r in tile_rects(&self.camera, vw, vh, &MapColors(map)) {
            let min = rect.min + egui::vec2(r.x, r.y);
            // A hair of overlap hides seams between rectangles at fractional zoom.
            painter.rect_filled(
                egui::Rect::from_min_size(min, egui::vec2(r.w + 0.5, r.h + 0.5)),
                0.0,
                c32(r.color),
            );
        }
        let zoom = self.camera.zoom;
        let mut hovered: Option<(String, Pos2)> = None;
        for p in snap.pawns.iter().filter(|p| p.map == map.id) {
            let (sx, sy) =
                self.camera
                    .to_screen(p.tile.x as f32 + 0.5, p.tile.y as f32 + 0.5, vw, vh);
            let center = rect.min + egui::vec2(sx, sy);
            if !rect.expand(zoom).contains(center) {
                continue;
            }
            let radius = (zoom * 0.38).max(2.0);
            painter.circle_filled(center, radius, c32(pawn_color(p.id)));
            painter.circle_stroke(
                center,
                radius,
                egui::Stroke::new(1.0, Color32::from_black_alpha(160)),
            );
            if zoom >= 22.0 {
                painter.text(
                    center + egui::vec2(0.0, -radius - 3.0),
                    egui::Align2::CENTER_BOTTOM,
                    &p.name,
                    egui::FontId::proportional(12.0),
                    Color32::WHITE,
                );
            }
            if let Some(h) = response.hover_pos() {
                if h.distance(center) <= radius + 3.0 {
                    hovered = Some((format!("{} ({})", p.name, p.activity), center));
                }
            }
        }
        if let Some((text, at)) = hovered {
            painter.text(
                at + egui::vec2(10.0, 10.0),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::proportional(14.0),
                Color32::from_rgb(255, 240, 180),
            );
        }
    }
}
