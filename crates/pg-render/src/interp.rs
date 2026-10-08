//! Smooth movement between ticks (Blueprint section 14): a resident that stepped to a new tile glides there
//! over the time the step takes instead of jumping. Plain arithmetic over positions and tick numbers.

use std::collections::BTreeMap;

/// One resident's glide.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Track {
    from: (i32, i32),
    to: (i32, i32),
    /// The simulation tick at which `to` was first seen.
    at: f64,
}

/// Remembers where each resident was and where it is now.
#[derive(Clone, Debug, Default)]
pub struct Interpolator {
    tracks: BTreeMap<u64, Track>,
}

/// A resident's drawn position, in tile coordinates (the middle of the tile is `+ 0.5`).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Placed {
    pub x: f32,
    pub y: f32,
    /// 0..1 through the current step; 1 when standing still.
    pub progress: f32,
    pub moving: bool,
}

impl Interpolator {
    pub fn new() -> Interpolator {
        Interpolator::default()
    }

    /// Records where `id` is at simulation tick `tick`. Call it for every resident whenever a new snapshot
    /// arrives; a resident seen on a new tile starts gliding from the tile it was on.
    pub fn observe(&mut self, id: u64, tile: (i32, i32), tick: f64) {
        match self.tracks.get_mut(&id) {
            None => {
                self.tracks.insert(
                    id,
                    Track {
                        from: tile,
                        to: tile,
                        at: tick,
                    },
                );
            }
            Some(t) if t.to != tile => {
                *t = Track {
                    from: t.to,
                    to: tile,
                    at: tick,
                };
            }
            Some(_) => {}
        }
    }

    /// Forgets residents that are no longer in the world.
    pub fn retain(&mut self, alive: &[u64]) {
        self.tracks.retain(|id, _| alive.contains(id));
    }

    /// Where `id` is drawn at (fractional) simulation tick `now`, taking `step_ticks` to cross a tile.
    pub fn sample(&self, id: u64, now: f64, step_ticks: f64) -> Option<Placed> {
        let t = self.tracks.get(&id)?;
        let span = step_ticks.max(1.0);
        let p = ((now - t.at) / span).clamp(0.0, 1.0) as f32;
        let lerp = |a: i32, b: i32| a as f32 + (b - a) as f32 * p;
        Some(Placed {
            x: lerp(t.from.0, t.to.0),
            y: lerp(t.from.1, t.to.1),
            progress: p,
            moving: t.from != t.to && p < 1.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_resident_stands_where_it_is_and_a_step_glides_over_the_step_time() {
        let mut i = Interpolator::new();
        i.observe(1, (4, 4), 100.0);
        let p = i.sample(1, 100.0, 8.0).unwrap();
        assert_eq!((p.x, p.y, p.moving), (4.0, 4.0, false));
        i.observe(1, (5, 4), 110.0);
        let start = i.sample(1, 110.0, 8.0).unwrap();
        assert_eq!((start.x, start.moving), (4.0, true));
        let mid = i.sample(1, 114.0, 8.0).unwrap();
        assert_eq!((mid.x, mid.y, mid.progress), (4.5, 4.0, 0.5));
        let end = i.sample(1, 130.0, 8.0).unwrap();
        assert_eq!(
            (end.x, end.moving),
            (5.0, false),
            "clamped at the destination"
        );
        // Time before the observation does not go backwards past the start.
        assert_eq!(i.sample(1, 90.0, 8.0).unwrap().x, 4.0);
    }

    #[test]
    fn a_second_step_starts_from_the_tile_just_reached_and_gone_residents_are_forgotten() {
        let mut i = Interpolator::new();
        i.observe(7, (0, 0), 0.0);
        i.observe(7, (0, 1), 10.0);
        i.observe(7, (0, 2), 18.0);
        let p = i.sample(7, 22.0, 8.0).unwrap();
        assert_eq!((p.x, p.y), (0.0, 1.5));
        i.observe(7, (0, 2), 30.0);
        assert_eq!(
            i.sample(7, 30.0, 8.0).unwrap().y,
            2.0,
            "standing: the track is unchanged"
        );
        i.retain(&[]);
        assert!(i.sample(7, 0.0, 8.0).is_none());
    }
}
