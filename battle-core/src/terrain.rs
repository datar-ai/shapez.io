//! Terrain: one coarse grid over the field. Each cell holds one kind, and
//! each kind reaches some of the height layers (low, main, fighter):
//!
//! | kind          | layers          |
//! |---------------|-----------------|
//! | asteroids     | low, main       |
//! | nebula        | main, fighter   |
//! | gravity well  | low (main half) |
//! | debris        | low             |
//! | planet        | all             |
//! | energy field  | low, main       |
//!
//! - Asteroid fields are walls for direct fire: a beam or gun shot whose
//!   line passes through one is blocked (ships within 300 units of the
//!   field's edge can still fire out and be hit). Ships inside are slowed to
//!   60% and harder to hit (-200 accuracy), and missiles and torpedoes
//!   flying through can be smashed.
//! - Nebulae hide ships: a ship inside can only be targeted by ships from
//!   within 1,000 units, and fighters cannot find it at all. Beams lose half
//!   their damage there and shields do not recharge.
//!
//! - Gravity wells surround planets: ships inside are slowed (to 50% on the
//!   low layer, 75% on the main layer) and cannot jump out.
//! - Debris fields are left where big ships die: cover (-150 accuracy) for
//!   ships on the low layer.
//! - Planets block every shot, missile and ship on both ship layers; groups
//!   steer round them.
//! - Energy fields belong to a side and are powered by its pylon: that
//!   side's shields recharge three times as fast inside, the enemy's not at
//!   all. They vanish when the pylon dies.
//! - A ship on the low layer is not hidden by a nebula, and is harder to
//!   hit in an asteroid field (-300 instead of -200).
//!
//! Fighters fly over all of it, but cannot see into a nebula.

use crate::fixed::FP;

pub const EMPTY: u8 = 0;
pub const ASTEROIDS: u8 = 1;
pub const NEBULA: u8 = 2;
pub const GRAVITY: u8 = 3;
pub const DEBRIS: u8 = 4;
pub const PLANET: u8 = 5;
/// Energy field of side 0 and of side 1.
pub const FIELD0: u8 = 6;
pub const FIELD1: u8 = 7;

/// Accuracy penalty (permille) against a ship in debris on the low layer,
/// and against one in an asteroid field on the low layer.
pub const DEBRIS_COVER: i32 = 150;
pub const ASTEROID_COVER_LOW: i32 = 300;

/// Energy field kind for a side.
pub const fn field_of(side: u8) -> u8 {
    FIELD0 + side
}

/// Cell size in distance units.
pub const CELL_UNITS: i32 = 500;
const CELL: i32 = CELL_UNITS * FP;
const HALF_CELLS: i32 = 24; // covers +-12,000 units, the whole field
pub const DIM: i32 = HALF_CELLS * 2;

/// Targets inside a nebula are invisible beyond this distance.
pub const NEBULA_SIGHT: i32 = 1000 * FP;
/// Accuracy penalty (permille) against a ship in an asteroid field.
pub const ASTEROID_COVER: i32 = 200;
/// Ships this close to either end of a shot's line are not blocked by rock.
pub const FIRE_EDGE: i32 = 300 * FP;
/// Spacing of the points checked along a line of fire (half a cell).
const LINE_STEP: i32 = CELL_UNITS / 2 * FP;
/// Chance (permille) per tick that a missile inside an asteroid field is destroyed.
pub const ASTEROID_MISSILE_LOSS: i32 = 25;

#[derive(Clone)]
pub struct Terrain {
    pub cells: Vec<u8>,
    /// Any asteroid or planet cell at all; lets open fields skip
    /// line-of-fire checks.
    rocks: bool,
}

impl Default for Terrain {
    fn default() -> Terrain {
        Terrain {
            cells: vec![EMPTY; (DIM * DIM) as usize],
            rocks: false,
        }
    }
}

impl Terrain {
    #[inline]
    pub fn at(&self, x: i32, y: i32) -> u8 {
        let cx = x.div_euclid(CELL) + HALF_CELLS;
        let cy = y.div_euclid(CELL) + HALF_CELLS;
        if cx < 0 || cy < 0 || cx >= DIM || cy >= DIM {
            return EMPTY;
        }
        self.cells[(cy * DIM + cx) as usize]
    }

    /// Paint an ellipse of `kind` centered at (x, y) with radii (rx, ry), all in
    /// distance units. A cell is painted when its center is inside.
    pub fn paint_ellipse(&mut self, kind: u8, x: i32, y: i32, rx: i32, ry: i32) {
        let (x, y, rx, ry) = (x as i64, y as i64, rx.max(1) as i64, ry.max(1) as i64);
        for cy in 0..DIM {
            for cx in 0..DIM {
                let px = ((cx - HALF_CELLS) * CELL_UNITS + CELL_UNITS / 2) as i64 - x;
                let py = ((cy - HALF_CELLS) * CELL_UNITS + CELL_UNITS / 2) as i64 - y;
                if px * px * ry * ry + py * py * rx * rx <= rx * rx * ry * ry {
                    self.cells[(cy * DIM + cx) as usize] = kind;
                    self.rocks |= blocks(kind);
                }
            }
        }
    }

    /// Index of the cell at (x, y), if on the grid.
    pub fn index(&self, x: i32, y: i32) -> Option<usize> {
        let cx = x.div_euclid(CELL) + HALF_CELLS;
        let cy = y.div_euclid(CELL) + HALF_CELLS;
        (cx >= 0 && cy >= 0 && cx < DIM && cy < DIM).then(|| (cy * DIM + cx) as usize)
    }

    /// Replace every cell of kind `from` with `to`.
    pub fn replace(&mut self, from: u8, to: u8) {
        for c in &mut self.cells {
            if *c == from {
                *c = to;
            }
        }
    }

    /// True when a direct-fire shot from `a` to `b` does not pass through an
    /// asteroid field or a planet. The first and last `FIRE_EDGE` units are not checked,
    /// so ships at the edge of a field can fire out and be fired at.
    pub fn clear_line(&self, a: (i32, i32), b: (i32, i32)) -> bool {
        if !self.rocks {
            return true;
        }
        let (dx, dy) = ((b.0 - a.0) as i64, (b.1 - a.1) as i64);
        let d = crate::fixed::len(dx, dy);
        let edge = FIRE_EDGE as i64;
        let mut k = edge;
        while k < d - edge {
            let (x, y) = (a.0 + (dx * k / d) as i32, a.1 + (dy * k / d) as i32);
            if blocks(self.at(x, y)) {
                return false;
            }
            k += LINE_STEP as i64;
        }
        true
    }

    pub fn has_rocks(&self) -> bool {
        self.rocks
    }

    /// True when a ship could go straight from `a` to `b` without hitting
    /// a planet.
    pub fn passable(&self, a: (i32, i32), b: (i32, i32)) -> bool {
        if !self.rocks {
            return true;
        }
        let (dx, dy) = ((b.0 - a.0) as i64, (b.1 - a.1) as i64);
        let d = crate::fixed::len(dx, dy);
        let mut k = 0;
        while k < d {
            let (x, y) = (a.0 + (dx * k / d) as i32, a.1 + (dy * k / d) as i32);
            if self.solid(x, y) {
                return false;
            }
            k += LINE_STEP as i64;
        }
        true
    }

    /// Planet cells are solid for ships.
    #[inline]
    pub fn solid(&self, x: i32, y: i32) -> bool {
        self.at(x, y) == PLANET
    }

    /// True when a ship at (x, y) on `layer` is inside a nebula (hidden).
    #[inline]
    pub fn hides(&self, x: i32, y: i32, layer: u8) -> bool {
        layer & 0x7f != crate::types::LAYER_LOW && self.at(x, y) == NEBULA
    }

    /// Speed (percent) of a ship at (x, y) on `layer`.
    #[inline]
    pub fn speed_pct(&self, x: i32, y: i32, layer: u8) -> i32 {
        let low = layer & 0x7f == crate::types::LAYER_LOW;
        match self.at(x, y) {
            ASTEROIDS => 60,
            GRAVITY if low => 50,
            GRAVITY => 75,
            _ => 100,
        }
    }

    /// Accuracy penalty against a ship at (x, y) on `layer`.
    #[inline]
    pub fn cover(&self, x: i32, y: i32, layer: u8) -> i32 {
        let low = layer & 0x7f == crate::types::LAYER_LOW;
        match self.at(x, y) {
            ASTEROIDS if low => ASTEROID_COVER_LOW,
            ASTEROIDS => ASTEROID_COVER,
            DEBRIS if low => DEBRIS_COVER,
            _ => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cells.iter().all(|&c| c == EMPTY)
    }
}

/// Kinds that stop direct fire.
#[inline]
pub fn blocks(kind: u8) -> bool {
    kind == ASTEROIDS || kind == PLANET
}
