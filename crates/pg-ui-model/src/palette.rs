//! Map colours, shared by the in-game view and the save thumbnails so the picture in the Saved Worlds list
//! looks like the world it was taken from.

use pg_core::id::EntityId;
use pg_core::map::{MapData, Tile, FLOOR, GRASS, ROAD, SAND, SIDEWALK, SURFACE_NONE, WATER};

pub type Rgb = [u8; 3];

pub const GRASS_RGB: Rgb = [88, 150, 80];
pub const WATER_RGB: Rgb = [60, 110, 190];
pub const SAND_RGB: Rgb = [214, 196, 138];
pub const ROAD_RGB: Rgb = [90, 90, 96];
pub const SIDEWALK_RGB: Rgb = [170, 170, 176];
pub const FLOOR_RGB: Rgb = [150, 110, 80];
pub const BUILDING_RGB: Rgb = [112, 72, 62];
pub const UNKNOWN_RGB: Rgb = [255, 0, 255];

/// The colour of one tile: a surface (road, sidewalk, floor) over the terrain, and blocked tiles that are
/// not water drawn as buildings.
pub fn tile_color(map: &MapData, t: Tile) -> Rgb {
    let terrain = match map.terrain_at(t) {
        Some(GRASS) => GRASS_RGB,
        Some(WATER) => WATER_RGB,
        Some(SAND) => SAND_RGB,
        _ => UNKNOWN_RGB,
    };
    match map.surface_at(t) {
        Some(ROAD) => ROAD_RGB,
        Some(SIDEWALK) => SIDEWALK_RGB,
        Some(FLOOR) => FLOOR_RGB,
        Some(s) if s != SURFACE_NONE => UNKNOWN_RGB,
        _ => {
            if map.is_blocked(t) && map.terrain_at(t) != Some(WATER) {
                BUILDING_RGB
            } else {
                terrain
            }
        }
    }
}

const PAWN_COLORS: [Rgb; 8] = [
    [240, 200, 60],
    [235, 120, 70],
    [230, 90, 140],
    [170, 110, 230],
    [90, 190, 230],
    [110, 220, 150],
    [250, 250, 250],
    [60, 60, 70],
];

/// A colour per pawn, stable for the pawn's id.
pub fn pawn_color(id: EntityId) -> Rgb {
    PAWN_COLORS
        .get(id.counter() as usize % PAWN_COLORS.len())
        .copied()
        .unwrap_or(UNKNOWN_RGB)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_core::id::Kind;
    use pg_core::map::MapKind;

    #[test]
    fn surfaces_cover_terrain_and_pawn_colours_are_stable() {
        let mut m = MapData::new(EntityId::new(Kind::Map, 1), MapKind::Overworld, 8, 8).unwrap();
        let t = Tile::new(2, 2);
        assert_eq!(tile_color(&m, t), GRASS_RGB);
        m.set_terrain(t, WATER).unwrap();
        assert_eq!(tile_color(&m, t), WATER_RGB);
        m.set_surface(t, ROAD).unwrap();
        assert_eq!(tile_color(&m, t), ROAD_RGB, "a road over water is a bridge");
        assert_eq!(
            tile_color(&m, Tile::new(99, 99)),
            BUILDING_RGB,
            "outside the map reads as blocked grass"
        );
        let a = EntityId::new(Kind::Pawn, 1);
        assert_eq!(pawn_color(a), pawn_color(a));
        assert_ne!(pawn_color(a), pawn_color(EntityId::new(Kind::Pawn, 2)));
    }
}
