//! Terrain: a coarse grid over the main-battle layer. Each cell is empty, an
//! asteroid field or a nebula.
//!
//! - Asteroids slow ships to 60% speed, give cover (-200 accuracy against
//!   ships inside) and can smash missiles and torpedoes flying through.
//! - Nebulae hide ships: a ship inside can only be targeted from within
//!   1,000 units. Beams lose half their damage there and shields do not
//!   recharge.
//!
//! Fighters fly over all of it.

use crate::fixed::FP;

pub const EMPTY: u8 = 0;
pub const ASTEROIDS: u8 = 1;
pub const NEBULA: u8 = 2;

/// Cell size in distance units.
pub const CELL_UNITS: i32 = 500;
const CELL: i32 = CELL_UNITS * FP;
const HALF_CELLS: i32 = 24; // covers +-12,000 units, the whole field
pub const DIM: i32 = HALF_CELLS * 2;

/// Targets inside a nebula are invisible beyond this distance.
pub const NEBULA_SIGHT: i32 = 1000 * FP;
/// Accuracy penalty (permille) against a ship in an asteroid field.
pub const ASTEROID_COVER: i32 = 200;
/// Chance (permille) per tick that a missile inside an asteroid field is destroyed.
pub const ASTEROID_MISSILE_LOSS: i32 = 25;

#[derive(Clone)]
pub struct Terrain {
    pub cells: Vec<u8>,
}

impl Default for Terrain {
    fn default() -> Terrain {
        Terrain {
            cells: vec![EMPTY; (DIM * DIM) as usize],
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
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cells.iter().all(|&c| c == EMPTY)
    }
}
