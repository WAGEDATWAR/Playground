//! The generated town's layout (Stage 1, milestone 1.2; Blueprint §12): districts, building footprints with
//! their entrances, and the public gathering places. Residents' homes and workplaces are tiles beside
//! these buildings; the generator in [`crate::worldgen`] fills this in and validates it.

use crate::canon::{Canon, ToCanon};
use crate::map::Tile;
use crate::read::{ReadError, Reader};

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DistrictKind {
    Residential,
    Commercial,
    Civic,
    Park,
}

impl DistrictKind {
    pub const ALL: [DistrictKind; 4] = [
        DistrictKind::Residential,
        DistrictKind::Commercial,
        DistrictKind::Civic,
        DistrictKind::Park,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            DistrictKind::Residential => "residential",
            DistrictKind::Commercial => "commercial",
            DistrictKind::Civic => "civic",
            DistrictKind::Park => "park",
        }
    }

    pub fn parse(s: &str) -> Option<DistrictKind> {
        DistrictKind::ALL.into_iter().find(|k| k.name() == s)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BuildingRole {
    Home,
    Shop,
    Office,
    Civic,
}

impl BuildingRole {
    pub const ALL: [BuildingRole; 4] = [
        BuildingRole::Home,
        BuildingRole::Shop,
        BuildingRole::Office,
        BuildingRole::Civic,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            BuildingRole::Home => "home",
            BuildingRole::Shop => "shop",
            BuildingRole::Office => "office",
            BuildingRole::Civic => "civic",
        }
    }

    pub fn parse(s: &str) -> Option<BuildingRole> {
        BuildingRole::ALL.into_iter().find(|r| r.name() == s)
    }

    /// The letter `pg worldgen preview` draws.
    pub const fn letter(self) -> char {
        match self {
            BuildingRole::Home => 'H',
            BuildingRole::Shop => 'S',
            BuildingRole::Office => 'O',
            BuildingRole::Civic => 'C',
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct District {
    pub kind: DistrictKind,
    pub center: Tile,
}

/// A building's footprint (blocked tiles) and the tile in front of its door.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Building {
    pub role: BuildingRole,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub entrance: Tile,
    /// Index into [`Town::districts`].
    pub district: u32,
}

impl Building {
    pub fn contains(&self, t: Tile) -> bool {
        t.x >= self.x && t.x < self.x + self.w && t.y >= self.y && t.y < self.y + self.h
    }

    pub fn overlaps(&self, other: &Building) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Town {
    pub districts: Vec<District>,
    pub buildings: Vec<Building>,
    /// Plazas and the like where residents meet, most central first.
    pub gathering: Vec<Tile>,
    /// The state hash (hex) of the world as generated, so a later edit or divergence can be told from the
    /// starting state (Blueprint §12.4). Never changes after generation.
    pub starting_hash: Option<String>,
}

impl Town {
    pub fn is_empty(&self) -> bool {
        self == &Town::default()
    }

    pub fn buildings_with(&self, role: BuildingRole) -> impl Iterator<Item = (usize, &Building)> {
        self.buildings
            .iter()
            .enumerate()
            .filter(move |(_, b)| b.role == role)
    }

    /// The district a tile belongs to, if any, by nearest centre (the same rule the generator zones with).
    pub fn district_at(&self, t: Tile) -> Option<usize> {
        self.districts
            .iter()
            .enumerate()
            .min_by_key(|(i, d)| (d.center.manhattan(t), *i))
            .map(|(i, _)| i)
    }
}

impl ToCanon for District {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("kind", Canon::str(self.kind.name())),
            ("center", self.center.to_canon()),
        ])
    }
}

impl ToCanon for Building {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("role", Canon::str(self.role.name())),
            ("x", self.x.to_canon()),
            ("y", self.y.to_canon()),
            ("w", self.w.to_canon()),
            ("h", self.h.to_canon()),
            ("entrance", self.entrance.to_canon()),
            ("district", self.district.to_canon()),
        ])
    }
}

impl ToCanon for Town {
    fn to_canon(&self) -> Canon {
        Canon::map([
            (
                "districts",
                Canon::List(self.districts.iter().map(ToCanon::to_canon).collect()),
            ),
            (
                "buildings",
                Canon::List(self.buildings.iter().map(ToCanon::to_canon).collect()),
            ),
            (
                "gathering",
                Canon::List(self.gathering.iter().map(ToCanon::to_canon).collect()),
            ),
            (
                "starting_hash",
                self.starting_hash
                    .as_ref()
                    .map_or(Canon::Null, |h| Canon::str(h.clone())),
            ),
        ])
    }
}

impl District {
    pub fn from_reader(r: Reader<'_>) -> Result<District, ReadError> {
        r.only(&["kind", "center"])?;
        let k = r.child("kind")?;
        Ok(District {
            kind: DistrictKind::parse(k.reader().str()?)
                .ok_or_else(|| k.reader().err("unknown district kind"))?,
            center: Tile::from_reader(r.child("center")?.reader())?,
        })
    }
}

impl Building {
    pub fn from_reader(r: Reader<'_>) -> Result<Building, ReadError> {
        r.only(&["role", "x", "y", "w", "h", "entrance", "district"])?;
        let role = r.child("role")?;
        let (w, h) = (r.child("w")?.reader().i32()?, r.child("h")?.reader().i32()?);
        if w < 1 || h < 1 {
            return Err(r.err("a building is at least one tile"));
        }
        Ok(Building {
            role: BuildingRole::parse(role.reader().str()?)
                .ok_or_else(|| role.reader().err("unknown building role"))?,
            x: r.child("x")?.reader().i32()?,
            y: r.child("y")?.reader().i32()?,
            w,
            h,
            entrance: Tile::from_reader(r.child("entrance")?.reader())?,
            district: r.child("district")?.reader().u32()?,
        })
    }
}

impl Town {
    pub fn from_reader(r: Reader<'_>) -> Result<Town, ReadError> {
        r.only(&["districts", "buildings", "gathering", "starting_hash"])?;
        let list = |key: &str| -> Result<Vec<crate::read::Child>, ReadError> {
            r.child(key)?.reader().list()
        };
        let town = Town {
            districts: list("districts")?
                .iter()
                .map(|c| District::from_reader(c.reader()))
                .collect::<Result<_, _>>()?,
            buildings: list("buildings")?
                .iter()
                .map(|c| Building::from_reader(c.reader()))
                .collect::<Result<_, _>>()?,
            gathering: list("gathering")?
                .iter()
                .map(|c| Tile::from_reader(c.reader()))
                .collect::<Result<_, _>>()?,
            starting_hash: match r.maybe("starting_hash")? {
                Some(c) => Some(c.reader().str()?.to_owned()),
                None => None,
            },
        };
        let districts = town.districts.len();
        if town
            .buildings
            .iter()
            .any(|b| usize::try_from(b.district).map_or(true, |d| d >= districts))
        {
            return Err(r.err("a building names a district that does not exist"));
        }
        Ok(town)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::json;

    fn town() -> Town {
        Town {
            districts: vec![
                District {
                    kind: DistrictKind::Commercial,
                    center: Tile::new(20, 20),
                },
                District {
                    kind: DistrictKind::Residential,
                    center: Tile::new(5, 5),
                },
            ],
            buildings: vec![
                Building {
                    role: BuildingRole::Home,
                    x: 4,
                    y: 4,
                    w: 3,
                    h: 3,
                    entrance: Tile::new(5, 7),
                    district: 1,
                },
                Building {
                    role: BuildingRole::Shop,
                    x: 19,
                    y: 19,
                    w: 4,
                    h: 3,
                    entrance: Tile::new(20, 22),
                    district: 0,
                },
            ],
            gathering: vec![Tile::new(20, 24)],
            starting_hash: Some("ab12".into()),
        }
    }

    #[test]
    fn a_town_round_trips_and_reads_back_identically() {
        let t = town();
        let text = t.to_canon().to_canonical_string();
        let back = Town::from_reader(Reader::new(&json::parse(&text).unwrap(), "town")).unwrap();
        assert_eq!(back, t);
        assert!(Town::default().is_empty() && !t.is_empty());
    }

    #[test]
    fn names_and_geometry_helpers_agree() {
        for k in DistrictKind::ALL {
            assert_eq!(DistrictKind::parse(k.name()), Some(k));
        }
        for r in BuildingRole::ALL {
            assert_eq!(BuildingRole::parse(r.name()), Some(r));
        }
        let t = town();
        assert!(
            t.buildings[0].contains(Tile::new(6, 6)) && !t.buildings[0].contains(Tile::new(7, 6))
        );
        assert!(!t.buildings[0].overlaps(&t.buildings[1]));
        let mut b = t.buildings[1];
        b.x = 5;
        b.y = 5;
        assert!(t.buildings[0].overlaps(&b));
        assert_eq!(t.district_at(Tile::new(6, 6)), Some(1));
        assert_eq!(t.district_at(Tile::new(18, 22)), Some(0));
        assert_eq!(t.buildings_with(BuildingRole::Shop).count(), 1);
    }

    #[test]
    fn a_damaged_town_is_refused() {
        let mut c = town().to_canon().to_canonical_string();
        c = c.replace("\"district\":1", "\"district\":9");
        let v = json::parse(&c).unwrap();
        assert!(Town::from_reader(Reader::new(&v, "town")).is_err());
        let c = town()
            .to_canon()
            .to_canonical_string()
            .replace("residential", "swamp");
        assert!(Town::from_reader(Reader::new(&json::parse(&c).unwrap(), "town")).is_err());
    }
}
