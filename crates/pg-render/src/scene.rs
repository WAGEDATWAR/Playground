//! From a map and its residents to a list of textured quads (Blueprint section 14).
//!
//! [`build_scene`] does the culling, the pixel-art zoom snapping, the per-tile variants and the draw order;
//! the GPU layer only uploads the list it returns. Everything is arithmetic over plain data, so it is
//! tested without a window.

use crate::atlas::{Atlas, Uv};
use crate::camera::Camera;
use crate::sprite::{mix, Appearance, Facing, SpriteKey, TileKind, SPRITE_PX};

/// What the renderer needs to know about a map.
pub trait Terrain {
    fn width(&self) -> i32;
    fn height(&self) -> i32;
    fn kind(&self, x: i32, y: i32) -> TileKind;
}

/// One resident to draw. `x` and `y` are tile coordinates and may be fractional (a step in progress).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PawnDraw {
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub facing: Facing,
    pub moving: bool,
    /// 0..1 through the current step.
    pub progress: f32,
    pub selected: bool,
}

/// A textured rectangle in view pixels.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Quad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub uv: Uv,
    pub tint: [u8; 4],
}

#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub quads: Vec<Quad>,
    /// How many of the quads are ground (the rest are residents and their shadows).
    pub ground: usize,
    /// Tiles in view.
    pub visible_tiles: usize,
}

const WHITE: [u8; 4] = [255, 255, 255, 255];

/// Pixel art looks right at whole multiples of its size: above 16 pixels a tile, the zoom snaps down to a
/// multiple of 16. Below that it is left alone (zoomed-out views are overviews).
pub fn snap_zoom(zoom: f32) -> f32 {
    let size = SPRITE_PX as f32;
    if zoom >= size {
        (zoom / size).floor() * size
    } else {
        zoom
    }
}

/// The camera with its zoom snapped for drawing and picking.
pub fn snapped(cam: &Camera) -> Camera {
    Camera {
        zoom: snap_zoom(cam.zoom),
        ..*cam
    }
}

fn variant(kind: TileKind, x: i32, y: i32) -> u8 {
    let n = u64::from(kind.variants().max(1));
    (mix(x as u64, y as u64, kind as u64) % n) as u8
}

/// Builds the draw list for a view of `view_w` by `view_h` pixels.
pub fn build_scene(
    cam: &Camera,
    view_w: f32,
    view_h: f32,
    terrain: &dyn Terrain,
    pawns: &[PawnDraw],
    atlas: &Atlas,
) -> Scene {
    let cam = snapped(cam);
    let px = cam.zoom;
    let whole = (px - px.round()).abs() < 1e-3;
    let snap = |v: f32| if whole { v.round() } else { v };
    let (left, top) = cam.to_world(0.0, 0.0, view_w, view_h);
    let (right, bottom) = cam.to_world(view_w, view_h, view_w, view_h);
    let x0 = (left.floor() as i32).clamp(0, terrain.width());
    let y0 = (top.floor() as i32).clamp(0, terrain.height());
    let x1 = (right.ceil() as i32 + 1).clamp(0, terrain.width());
    let y1 = (bottom.ceil() as i32 + 1).clamp(0, terrain.height());
    let mut quads =
        Vec::with_capacity(((x1 - x0).max(0) * (y1 - y0).max(0)) as usize + pawns.len() * 3);
    for y in y0..y1 {
        for x in x0..x1 {
            let kind = terrain.kind(x, y);
            let Some(uv) = atlas.uv(SpriteKey::Terrain(kind, variant(kind, x, y))) else {
                continue;
            };
            let (sx, sy) = cam.to_screen(x as f32, y as f32, view_w, view_h);
            let (ax, ay) = (snap(sx), snap(sy));
            // Size to the next tile's snapped corner so neighbours always meet.
            let (bx, by) = cam.to_screen((x + 1) as f32, (y + 1) as f32, view_w, view_h);
            quads.push(Quad {
                x: ax,
                y: ay,
                w: snap(bx) - ax,
                h: snap(by) - ay,
                uv,
                tint: WHITE,
            });
        }
    }
    let ground = quads.len();
    let mut order: Vec<&PawnDraw> = pawns
        .iter()
        .filter(|p| p.x > left - 2.0 && p.x < right + 2.0 && p.y > top - 2.0 && p.y < bottom + 2.0)
        .collect();
    order.sort_by(|a, b| {
        a.y.partial_cmp(&b.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.id.cmp(&b.id))
    });
    for p in order {
        let (sx, sy) = cam.to_screen(p.x, p.y, view_w, view_h);
        let (ax, ay) = (snap(sx), snap(sy));
        let mut push = |key: SpriteKey| {
            if let Some(uv) = atlas.uv(key) {
                quads.push(Quad {
                    x: ax,
                    y: ay,
                    w: px,
                    h: px,
                    uv,
                    tint: WHITE,
                });
            }
        };
        push(SpriteKey::Shadow);
        if p.selected {
            push(SpriteKey::Ring);
        }
        let frame = u8::from(p.moving && (0.25..0.75).contains(&p.progress));
        push(SpriteKey::Pawn {
            look: Appearance::from_seed(p.id),
            facing: p.facing,
            frame,
        });
    }
    Scene {
        quads,
        ground,
        visible_tiles: ((x1 - x0).max(0) * (y1 - y0).max(0)) as usize,
    }
}

/// The resident under a screen position (the nearest within about half a tile), for click to focus.
pub fn pick_pawn(
    cam: &Camera,
    view_w: f32,
    view_h: f32,
    pawns: &[PawnDraw],
    sx: f32,
    sy: f32,
) -> Option<u64> {
    let cam = snapped(cam);
    let reach = (cam.zoom * 0.55).max(6.0);
    pawns
        .iter()
        .filter_map(|p| {
            let (cx, cy) = cam.to_screen(p.x + 0.5, p.y + 0.5, view_w, view_h);
            let d = ((cx - sx).powi(2) + (cy - sy).powi(2)).sqrt();
            (d <= reach).then_some((d, p.id))
        })
        .min_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        })
        .map(|(_, id)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Flat(i32, i32, TileKind);

    impl Terrain for Flat {
        fn width(&self) -> i32 {
            self.0
        }
        fn height(&self) -> i32 {
            self.1
        }
        fn kind(&self, x: i32, _y: i32) -> TileKind {
            if x == 3 {
                TileKind::Road
            } else {
                self.2
            }
        }
    }

    fn pawn(id: u64, x: f32, y: f32) -> PawnDraw {
        PawnDraw {
            id,
            x,
            y,
            facing: Facing::S,
            moving: false,
            progress: 1.0,
            selected: false,
        }
    }

    #[test]
    fn zoom_snaps_to_whole_multiples_of_the_art_size_above_sixteen() {
        assert_eq!(snap_zoom(7.5), 7.5);
        assert_eq!(snap_zoom(16.0), 16.0);
        assert_eq!(snap_zoom(31.9), 16.0);
        assert_eq!(snap_zoom(33.0), 32.0);
        assert_eq!(snap_zoom(96.0), 96.0);
    }

    #[test]
    fn only_the_visible_tiles_are_drawn_and_neighbours_meet_without_gaps() {
        let atlas = Atlas::standard();
        let cam = Camera {
            cx: 50.0,
            cy: 50.0,
            zoom: 32.0,
        };
        let scene = build_scene(
            &cam,
            320.0,
            160.0,
            &Flat(100, 100, TileKind::Grass),
            &[],
            &atlas,
        );
        // 10 x 5 tiles in view, plus the margin the range adds.
        assert!(
            scene.ground >= 50 && scene.ground <= 12 * 8,
            "{}",
            scene.ground
        );
        assert_eq!(scene.visible_tiles, scene.ground);
        let q = &scene.quads;
        for w in q.windows(2) {
            if (w[0].y - w[1].y).abs() < 0.01 {
                assert!((w[0].x + w[0].w - w[1].x).abs() < 0.01, "a gap in a row");
            }
        }
        assert!(q.iter().all(|q| q.w == 32.0 && q.h == 32.0));
        // A map smaller than the view draws only itself.
        let small = build_scene(
            &cam,
            4000.0,
            4000.0,
            &Flat(4, 3, TileKind::Sand),
            &[],
            &atlas,
        );
        assert_eq!(small.ground, 12);
    }

    #[test]
    fn residents_are_sorted_back_to_front_with_a_shadow_and_a_ring_when_selected() {
        let atlas = Atlas::standard();
        let cam = Camera {
            cx: 5.0,
            cy: 5.0,
            zoom: 16.0,
        };
        let mut chosen = pawn(1, 5.0, 6.0);
        chosen.selected = true;
        let pawns = [chosen, pawn(2, 5.0, 4.0), pawn(3, 500.0, 500.0)];
        let scene = build_scene(
            &cam,
            160.0,
            160.0,
            &Flat(10, 10, TileKind::Grass),
            &pawns,
            &atlas,
        );
        let after = &scene.quads[scene.ground..];
        // pawn 2 (shadow + body), then pawn 1 (shadow + ring + body); pawn 3 is off screen.
        assert_eq!(after.len(), 2 + 3);
        assert!(after[0].y < after[2].y, "the one further up is drawn first");
        assert_eq!(after[0].uv, atlas.uv(SpriteKey::Shadow).unwrap());
        assert_eq!(after[3].uv, atlas.uv(SpriteKey::Ring).unwrap());
    }

    #[test]
    fn a_step_in_progress_uses_the_mid_step_frame() {
        let atlas = Atlas::standard();
        let cam = Camera {
            cx: 5.0,
            cy: 5.0,
            zoom: 16.0,
        };
        let mut a = pawn(9, 5.0, 5.0);
        a.moving = true;
        a.progress = 0.5;
        let mut b = a;
        b.progress = 0.9;
        let ground = Flat(10, 10, TileKind::Grass);
        let (sa, sb) = (
            build_scene(&cam, 160.0, 160.0, &ground, &[a], &atlas),
            build_scene(&cam, 160.0, 160.0, &ground, &[b], &atlas),
        );
        assert_ne!(sa.quads.last().unwrap().uv, sb.quads.last().unwrap().uv);
    }

    #[test]
    fn clicking_picks_the_nearest_resident_within_reach_and_nothing_far_away() {
        let cam = Camera {
            cx: 5.0,
            cy: 5.0,
            zoom: 32.0,
        };
        let pawns = [pawn(1, 5.0, 5.0), pawn(2, 6.0, 5.0)];
        // The middle of pawn 1's tile on a 320x320 view is (160 + 0.5*32, 160 + 0.5*32) = (176, 176).
        assert_eq!(pick_pawn(&cam, 320.0, 320.0, &pawns, 176.0, 176.0), Some(1));
        assert_eq!(pick_pawn(&cam, 320.0, 320.0, &pawns, 208.0, 176.0), Some(2));
        assert_eq!(pick_pawn(&cam, 320.0, 320.0, &pawns, 10.0, 10.0), None);
    }

    /// FNV-1a over the draw list, so a change to culling, ordering, snapping or the art shows up as one number.
    fn draw_list_hash(scene: &Scene) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |bytes: &[u8]| {
            for b in bytes {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        for q in &scene.quads {
            for v in [q.x, q.y, q.w, q.h, q.uv.u0, q.uv.v0, q.uv.u1, q.uv.v1] {
                eat(&v.to_bits().to_le_bytes());
            }
            eat(&q.tint);
        }
        h
    }

    #[test]
    fn the_golden_draw_list_for_a_small_town_view_does_not_change() {
        struct Town;
        impl Terrain for Town {
            fn width(&self) -> i32 {
                12
            }
            fn height(&self) -> i32 {
                9
            }
            fn kind(&self, x: i32, y: i32) -> TileKind {
                match (x, y) {
                    (_, 4) => TileKind::Road,
                    (3..=5, 1..=2) => TileKind::Building,
                    (8..=11, 6..=8) => TileKind::Water,
                    (7, _) => TileKind::Sidewalk,
                    _ => TileKind::Grass,
                }
            }
        }
        let atlas = Atlas::standard();
        let cam = Camera {
            cx: 6.0,
            cy: 4.5,
            zoom: 32.0,
        };
        let pawns = [
            PawnDraw {
                moving: true,
                progress: 0.5,
                selected: true,
                ..pawn(3, 5.5, 4.0)
            },
            pawn(11, 2.0, 6.0),
        ];
        let scene = build_scene(&cam, 384.0, 288.0, &Town, &pawns, &atlas);
        assert_eq!((scene.ground, scene.quads.len()), (108, 108 + 3 + 2));
        assert_eq!(draw_list_hash(&scene), GOLDEN_DRAW_LIST);
    }

    const GOLDEN_DRAW_LIST: u64 = 15_247_877_864_760_140_043;
}
