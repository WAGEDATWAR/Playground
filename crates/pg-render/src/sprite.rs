//! Procedural pixel art (Stage 1, milestone 1.6): terrain tiles and residents drawn in code.
//!
//! The art is generated, not loaded, so the game has pictures before there are assets (the decision for
//! Stage 1); real assets can replace it later behind the same [`SpriteKey`]s. Everything is deterministic:
//! the same key always gives the same pixels. Sprites are 16 by 16 RGBA.

/// Side of every sprite, in pixels.
pub const SPRITE_PX: usize = 16;

/// What a map tile looks like (the app maps the simulation's terrain, surface and blocked flags to this).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TileKind {
    Grass,
    Water,
    Sand,
    Road,
    Sidewalk,
    Floor,
    /// A blocked tile that is not water: a building's wall.
    Building,
    Unknown,
}

impl TileKind {
    pub const ALL: [TileKind; 8] = [
        TileKind::Grass,
        TileKind::Water,
        TileKind::Sand,
        TileKind::Road,
        TileKind::Sidewalk,
        TileKind::Floor,
        TileKind::Building,
        TileKind::Unknown,
    ];

    /// How many looks the kind has (picked per tile position, so large areas are not flat).
    pub const fn variants(self) -> u8 {
        match self {
            TileKind::Grass => 4,
            TileKind::Water | TileKind::Sand | TileKind::Road | TileKind::Building => 2,
            TileKind::Sidewalk | TileKind::Floor | TileKind::Unknown => 1,
        }
    }

    /// The colour a whole tile of this kind averages to (used for the smallest zoom levels and thumbnails).
    pub const fn average(self) -> [u8; 3] {
        match self {
            TileKind::Grass => [88, 150, 80],
            TileKind::Water => [60, 110, 190],
            TileKind::Sand => [214, 196, 138],
            TileKind::Road => [90, 90, 96],
            TileKind::Sidewalk => [170, 170, 176],
            TileKind::Floor => [150, 110, 80],
            TileKind::Building => [112, 72, 62],
            TileKind::Unknown => [255, 0, 255],
        }
    }
}

/// How a resident looks: stable for the resident.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Appearance {
    /// 0..8
    pub shirt: u8,
    /// 0..4
    pub skin: u8,
    /// 0..4
    pub hair: u8,
}

pub const SHIRTS: usize = 8;
pub const SKINS: usize = 4;
pub const HAIRS: usize = 4;

impl Appearance {
    /// An appearance from any number (a resident's id, say); the same number always gives the same look.
    pub fn from_seed(seed: u64) -> Appearance {
        // A cheap integer mix so neighbouring ids do not look alike.
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        x ^= x >> 29;
        Appearance {
            shirt: (x % SHIRTS as u64) as u8,
            skin: ((x >> 8) % SKINS as u64) as u8,
            hair: ((x >> 16) % HAIRS as u64) as u8,
        }
    }
}

/// Which way a resident faces.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Facing {
    N,
    E,
    S,
    W,
}

impl Facing {
    pub const ALL: [Facing; 4] = [Facing::N, Facing::E, Facing::S, Facing::W];
}

/// Everything the atlas holds.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SpriteKey {
    Terrain(TileKind, u8),
    Pawn {
        look: Appearance,
        facing: Facing,
        /// 0 standing, 1 mid-step
        frame: u8,
    },
    Shadow,
    Ring,
}

/// A finished sprite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sprite {
    pub w: usize,
    pub h: usize,
    /// RGBA, row by row.
    pub rgba: Vec<u8>,
}

impl Sprite {
    fn new() -> Sprite {
        Sprite {
            w: SPRITE_PX,
            h: SPRITE_PX,
            rgba: vec![0; SPRITE_PX * SPRITE_PX * 4],
        }
    }

    fn put(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if (0..SPRITE_PX as i32).contains(&x) && (0..SPRITE_PX as i32).contains(&y) {
            let i = (y as usize * SPRITE_PX + x as usize) * 4;
            if let Some(px) = self.rgba.get_mut(i..i + 4) {
                px.copy_from_slice(&c);
            }
        }
    }

    fn fill(&mut self, c: [u8; 3]) {
        for y in 0..SPRITE_PX as i32 {
            for x in 0..SPRITE_PX as i32 {
                self.put(x, y, [c[0], c[1], c[2], 255]);
            }
        }
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: [u8; 3]) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, [c[0], c[1], c[2], 255]);
            }
        }
    }

    /// The pixel at `(x, y)`.
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.w + x) * 4;
        self.rgba
            .get(i..i + 4)
            .map_or([0; 4], |p| [p[0], p[1], p[2], p[3]])
    }
}

/// A small deterministic hash of a few numbers.
pub fn mix(a: u64, b: u64, c: u64) -> u64 {
    let mut x = a
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
        .wrapping_add(c.wrapping_mul(0x1656_67B1_9E37_79F9));
    x ^= x >> 31;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 29)
}

fn shade(c: [u8; 3], d: i32) -> [u8; 3] {
    let f = |v: u8| (i32::from(v) + d).clamp(0, 255) as u8;
    [f(c[0]), f(c[1]), f(c[2])]
}

/// Scatters `n` flecks of `color` over the sprite, placed by the hash of `salt`.
fn flecks(s: &mut Sprite, salt: u64, n: u64, color: [u8; 3]) {
    for i in 0..n {
        let h = mix(salt, i, 7);
        s.rect((h % 16) as i32, ((h >> 8) % 16) as i32, 1, 1, color);
    }
}

fn terrain(kind: TileKind, variant: u8) -> Sprite {
    let mut s = Sprite::new();
    let base = kind.average();
    let v = u64::from(variant);
    s.fill(base);
    match kind {
        TileKind::Grass => {
            flecks(&mut s, v + 1, 9, shade(base, -16));
            flecks(&mut s, v + 50, 5, shade(base, 14));
        }
        TileKind::Water => {
            let hi = shade(base, 38);
            let y = 3 + i32::from(variant) * 5;
            s.rect(2, y, 4, 1, hi);
            s.rect(9, y + 6, 5, 1, hi);
            s.rect(11, y + 1, 3, 1, shade(base, 20));
            flecks(&mut s, v + 90, 4, shade(base, -14));
        }
        TileKind::Sand => {
            flecks(&mut s, v + 7, 8, shade(base, -18));
            flecks(&mut s, v + 70, 4, shade(base, 12));
        }
        TileKind::Road => {
            flecks(&mut s, v + 13, 10, shade(base, -10));
            flecks(&mut s, v + 130, 3, shade(base, 10));
        }
        TileKind::Sidewalk => {
            s.rect(15, 0, 1, 16, shade(base, -22));
            s.rect(0, 15, 16, 1, shade(base, -22));
            flecks(&mut s, 3, 4, shade(base, -8));
        }
        TileKind::Floor => {
            for row in [3, 7, 11, 15] {
                s.rect(0, row, 16, 1, shade(base, -22));
            }
            s.rect(5, 0, 1, 3, shade(base, -22));
            s.rect(11, 4, 1, 3, shade(base, -22));
            s.rect(3, 8, 1, 3, shade(base, -22));
            s.rect(9, 12, 1, 3, shade(base, -22));
        }
        TileKind::Building => {
            let mortar = shade(base, -26);
            for row in 0..4 {
                let y = row * 4 + 3;
                s.rect(0, y, 16, 1, mortar);
                let off = if row % 2 == 0 { 3 } else { 7 };
                s.rect(off, row * 4, 1, 3, mortar);
                s.rect(off + 8, row * 4, 1, 3, mortar);
            }
            s.rect(0, 0, 16, 1, shade(base, 26));
            if variant == 1 {
                s.rect(4, 4, 8, 8, shade(base, -40));
                s.rect(5, 5, 6, 6, [150, 200, 230]);
                s.rect(8, 5, 1, 6, shade(base, -40));
                s.rect(5, 8, 6, 1, shade(base, -40));
                s.rect(5, 5, 2, 1, [220, 240, 250]);
            }
        }
        TileKind::Unknown => {
            s.rect(0, 0, 8, 8, [0, 0, 0]);
            s.rect(8, 8, 8, 8, [0, 0, 0]);
        }
    }
    s
}

/// Shirt colours.
pub const SHIRT_RGB: [[u8; 3]; SHIRTS] = [
    [240, 200, 60],
    [235, 120, 70],
    [230, 90, 140],
    [170, 110, 230],
    [90, 190, 230],
    [110, 220, 150],
    [245, 245, 245],
    [70, 70, 90],
];
const SKIN_RGB: [[u8; 3]; SKINS] = [
    [255, 219, 172],
    [224, 172, 105],
    [198, 134, 66],
    [141, 85, 36],
];
const HAIR_RGB: [[u8; 3]; HAIRS] = [[60, 40, 30], [30, 30, 36], [190, 140, 60], [150, 70, 40]];
const PANTS: [u8; 3] = [48, 52, 78];

fn pawn(look: Appearance, facing: Facing, frame: u8) -> Sprite {
    let mut s = Sprite::new();
    let shirt = SHIRT_RGB[usize::from(look.shirt) % SHIRTS];
    let skin = SKIN_RGB[usize::from(look.skin) % SKINS];
    let hair = HAIR_RGB[usize::from(look.hair) % HAIRS];
    let dark = [24, 24, 30];
    // Legs: standing side by side; mid-step lifts one leg a pixel.
    let (left_up, right_up) = match (frame, facing) {
        (1, Facing::N | Facing::S) => (1, 0),
        (1, _) => (0, 1),
        _ => (0, 0),
    };
    match facing {
        Facing::E | Facing::W => {
            // Side view: legs overlap, one slightly forward.
            s.rect(6, 12 + left_up, 2, 4 - left_up, PANTS);
            s.rect(8, 12 + right_up, 2, 4 - right_up, shade(PANTS, 14));
        }
        _ => {
            s.rect(5, 12 + left_up, 3, 4 - left_up, PANTS);
            s.rect(8, 12 + right_up, 3, 4 - right_up, PANTS);
        }
    }
    s.rect(5, 7, 6, 5, shirt);
    s.rect(5, 7, 6, 1, shade(shirt, 20));
    s.rect(4, 8, 1, 3, shade(shirt, -18));
    s.rect(11, 8, 1, 3, shade(shirt, -18));
    s.rect(6, 2, 4, 5, skin);
    match facing {
        Facing::S => {
            s.rect(6, 2, 4, 2, hair);
            s.rect(7, 5, 1, 1, dark);
            s.rect(9, 5, 1, 1, dark);
        }
        Facing::N => {
            s.rect(6, 2, 4, 4, hair);
        }
        Facing::E => {
            s.rect(6, 2, 4, 2, hair);
            s.rect(6, 4, 1, 2, hair);
            s.rect(9, 5, 1, 1, dark);
        }
        Facing::W => {
            s.rect(6, 2, 4, 2, hair);
            s.rect(9, 4, 1, 2, hair);
            s.rect(6, 5, 1, 1, dark);
        }
    }
    s
}

fn shadow() -> Sprite {
    let mut s = Sprite::new();
    for y in 0..SPRITE_PX as i32 {
        for x in 0..SPRITE_PX as i32 {
            let (dx, dy) = (x as f32 - 7.5, (y as f32 - 13.5) * 2.2);
            if dx * dx + dy * dy <= 30.0 {
                s.put(x, y, [0, 0, 0, 70]);
            }
        }
    }
    s
}

fn ring() -> Sprite {
    let mut s = Sprite::new();
    for y in 0..SPRITE_PX as i32 {
        for x in 0..SPRITE_PX as i32 {
            let (dx, dy) = (x as f32 - 7.5, (y as f32 - 13.0) * 1.8);
            let d = dx * dx + dy * dy;
            if (30.0..=50.0).contains(&d) {
                s.put(x, y, [255, 235, 120, 230]);
            }
        }
    }
    s
}

/// Draws the sprite for `key`.
pub fn render(key: SpriteKey) -> Sprite {
    match key {
        SpriteKey::Terrain(kind, v) => terrain(kind, v % kind.variants().max(1)),
        SpriteKey::Pawn {
            look,
            facing,
            frame,
        } => pawn(look, facing, frame),
        SpriteKey::Shadow => shadow(),
        SpriteKey::Ring => ring(),
    }
}

/// Every key the game uses, in a fixed order.
pub fn all_keys() -> Vec<SpriteKey> {
    let mut keys = Vec::new();
    for kind in TileKind::ALL {
        for v in 0..kind.variants() {
            keys.push(SpriteKey::Terrain(kind, v));
        }
    }
    keys.push(SpriteKey::Shadow);
    keys.push(SpriteKey::Ring);
    for shirt in 0..SHIRTS as u8 {
        for skin in 0..SKINS as u8 {
            for hair in 0..HAIRS as u8 {
                for facing in Facing::ALL {
                    for frame in 0..2 {
                        keys.push(SpriteKey::Pawn {
                            look: Appearance { shirt, skin, hair },
                            facing,
                            frame,
                        });
                    }
                }
            }
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprites_are_deterministic_full_size_and_every_terrain_looks_like_its_colour() {
        for key in all_keys().into_iter().take(40) {
            let a = render(key);
            assert_eq!(a, render(key), "{key:?}");
            assert_eq!((a.w, a.h, a.rgba.len()), (16, 16, 16 * 16 * 4));
        }
        for kind in TileKind::ALL {
            if kind == TileKind::Unknown {
                continue;
            }
            let s = render(SpriteKey::Terrain(kind, 0));
            let mut sum = [0u64; 3];
            for y in 0..16 {
                for x in 0..16 {
                    let p = s.pixel(x, y);
                    assert_eq!(p[3], 255, "terrain is opaque");
                    for (i, t) in sum.iter_mut().enumerate() {
                        *t += u64::from(p[i]);
                    }
                }
            }
            let avg = kind.average();
            for i in 0..3 {
                let got = (sum[i] / 256) as i32;
                assert!(
                    (got - i32::from(avg[i])).abs() < 40,
                    "{kind:?} channel {i}: {got} vs {avg:?}"
                );
            }
        }
    }

    #[test]
    fn variants_differ_and_wrap() {
        assert_ne!(
            render(SpriteKey::Terrain(TileKind::Grass, 0)),
            render(SpriteKey::Terrain(TileKind::Grass, 1))
        );
        assert_eq!(
            render(SpriteKey::Terrain(TileKind::Grass, 4)),
            render(SpriteKey::Terrain(TileKind::Grass, 0))
        );
        assert_ne!(
            render(SpriteKey::Terrain(TileKind::Building, 0)),
            render(SpriteKey::Terrain(TileKind::Building, 1)),
            "one building tile has a window"
        );
    }

    #[test]
    fn residents_have_four_facings_two_frames_and_stable_looks() {
        let look = Appearance::from_seed(7);
        assert_eq!(look, Appearance::from_seed(7));
        let mut seen = std::collections::BTreeSet::new();
        for id in 0..200u64 {
            seen.insert(Appearance::from_seed(id));
        }
        assert!(seen.len() > 60, "{} looks among 200 residents", seen.len());
        let mut sprites = Vec::new();
        for f in Facing::ALL {
            for frame in 0..2 {
                sprites.push(render(SpriteKey::Pawn {
                    look,
                    facing: f,
                    frame,
                }));
            }
        }
        for i in 0..sprites.len() {
            for j in i + 1..sprites.len() {
                assert_ne!(sprites[i], sprites[j], "{i} vs {j}");
            }
        }
        // Residents are see-through around the body so the ground shows.
        assert_eq!(sprites[0].pixel(0, 0)[3], 0);
        assert_eq!(all_keys().len(), 15 + 2 + SHIRTS * SKINS * HAIRS * 8);
    }
}
