//! Packing sprites into one texture (Blueprint section 14): a shelf packer with a pixel of padding, so
//! nearest-neighbour sampling never reads a neighbour's colours. Pure and deterministic.

use crate::sprite::{all_keys, render, Sprite, SpriteKey};
use std::collections::BTreeMap;

/// Where a sprite sits in the atlas, as fractions of its size (`u0, v0` top-left to `u1, v1` bottom-right).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Uv {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

type PixelRect = (usize, usize, usize, usize);

#[derive(Clone, Debug)]
pub struct Atlas {
    pub width: usize,
    pub height: usize,
    /// RGBA, row by row.
    pub rgba: Vec<u8>,
    uvs: BTreeMap<SpriteKey, Uv>,
    /// Pixel rectangles `(x, y, w, h)` (for tests and tools).
    rects: BTreeMap<SpriteKey, PixelRect>,
}

const PAD: usize = 1;

impl Atlas {
    /// Packs `sprites` into a texture `width` pixels wide (the height follows, rounded up to a power of two).
    pub fn pack(sprites: &[(SpriteKey, Sprite)], width: usize) -> Atlas {
        let mut order: Vec<usize> = (0..sprites.len()).collect();
        // Tallest first, then by key, so the result does not depend on the input order beyond the keys.
        order.sort_by(|a, b| {
            let (sa, sb) = (&sprites[*a], &sprites[*b]);
            sb.1.h.cmp(&sa.1.h).then(sa.0.cmp(&sb.0))
        });
        let (mut x, mut y, mut shelf) = (PAD, PAD, 0usize);
        let mut placed: Vec<(usize, usize, usize)> = Vec::new();
        for i in order {
            let s = &sprites[i].1;
            if x + s.w + PAD > width {
                x = PAD;
                y += shelf + PAD;
                shelf = 0;
            }
            placed.push((i, x, y));
            shelf = shelf.max(s.h);
            x += s.w + PAD;
        }
        let used = y + shelf + PAD;
        let height = used.next_power_of_two().max(2);
        let mut rgba = vec![0u8; width * height * 4];
        let mut uvs = BTreeMap::new();
        let mut rects = BTreeMap::new();
        for (i, px, py) in placed {
            let (key, s) = &sprites[i];
            for row in 0..s.h {
                let from = row * s.w * 4;
                let to = ((py + row) * width + px) * 4;
                if let (Some(src), Some(dst)) = (
                    s.rgba.get(from..from + s.w * 4),
                    rgba.get_mut(to..to + s.w * 4),
                ) {
                    dst.copy_from_slice(src);
                }
            }
            uvs.insert(
                *key,
                Uv {
                    u0: px as f32 / width as f32,
                    v0: py as f32 / height as f32,
                    u1: (px + s.w) as f32 / width as f32,
                    v1: (py + s.h) as f32 / height as f32,
                },
            );
            rects.insert(*key, (px, py, s.w, s.h));
        }
        Atlas {
            width,
            height,
            rgba,
            uvs,
            rects,
        }
    }

    /// The atlas of every sprite the game draws.
    pub fn standard() -> Atlas {
        let sprites: Vec<(SpriteKey, Sprite)> =
            all_keys().into_iter().map(|k| (k, render(k))).collect();
        Atlas::pack(&sprites, 512)
    }

    pub fn uv(&self, key: SpriteKey) -> Option<Uv> {
        self.uvs.get(&key).copied()
    }

    pub fn len(&self) -> usize {
        self.uvs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.uvs.is_empty()
    }

    pub fn rects(&self) -> impl Iterator<Item = (&SpriteKey, &PixelRect)> {
        self.rects.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprite::{Appearance, Facing, TileKind};

    #[test]
    fn every_sprite_is_placed_inside_the_texture_without_overlap_and_with_padding() {
        let a = Atlas::standard();
        assert_eq!(a.len(), all_keys().len());
        assert!(a.width.is_power_of_two() && a.height.is_power_of_two());
        assert!(a.height <= 2048, "{}x{}", a.width, a.height);
        let rects: Vec<_> = a.rects().map(|(_, r)| *r).collect();
        for (i, r) in rects.iter().enumerate() {
            assert!(r.0 + r.2 <= a.width && r.1 + r.3 <= a.height);
            for o in rects.iter().skip(i + 1) {
                let apart = r.0 + r.2 + PAD <= o.0
                    || o.0 + o.2 + PAD <= r.0
                    || r.1 + r.3 + PAD <= o.1
                    || o.1 + o.3 + PAD <= r.1;
                assert!(apart, "{r:?} touches {o:?}");
            }
        }
    }

    #[test]
    fn uvs_are_fractions_and_the_pixels_at_them_are_the_sprite() {
        let a = Atlas::standard();
        let key = SpriteKey::Pawn {
            look: Appearance {
                shirt: 3,
                skin: 1,
                hair: 2,
            },
            facing: Facing::E,
            frame: 1,
        };
        let uv = a.uv(key).unwrap();
        assert!((0.0..=1.0).contains(&uv.u0) && uv.u1 > uv.u0 && uv.v1 <= 1.0);
        let s = render(key);
        let (x0, y0) = (
            (uv.u0 * a.width as f32).round() as usize,
            (uv.v0 * a.height as f32).round() as usize,
        );
        for (x, y) in [(6, 8), (7, 3), (0, 0), (15, 15)] {
            let i = ((y0 + y) * a.width + x0 + x) * 4;
            assert_eq!(&a.rgba[i..i + 4], &s.pixel(x, y), "({x},{y})");
        }
        assert!(a.uv(SpriteKey::Terrain(TileKind::Grass, 0)).is_some());
    }

    #[test]
    fn packing_is_deterministic_and_independent_of_input_order() {
        let mut sprites: Vec<(SpriteKey, Sprite)> = all_keys()
            .into_iter()
            .take(60)
            .map(|k| (k, render(k)))
            .collect();
        let a = Atlas::pack(&sprites, 256);
        sprites.reverse();
        let b = Atlas::pack(&sprites, 256);
        assert_eq!(a.rgba, b.rgba);
        assert_eq!(a.uv(SpriteKey::Shadow), b.uv(SpriteKey::Shadow));
    }
}
