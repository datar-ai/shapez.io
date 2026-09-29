//! Uniform grid rebuilt every step with a counting sort. No trees, no
//! allocation after warm-up, and the cell order is fixed, so neighbor queries
//! return items in the same order on every run.

use crate::fixed::FP;

pub const CELL: i32 = 1000 * FP;
const HALF_CELLS: i32 = 24; // covers +-24,000 units; anything outside is clamped
pub const DIM: i32 = HALF_CELLS * 2;

#[derive(Default)]
pub struct Grid {
    start: Vec<u32>, // DIM*DIM + 1 prefix sums
    items: Vec<u32>,
    cell_of: Vec<u32>,
}

#[inline]
pub fn cell_coord(v: i32) -> i32 {
    (v.div_euclid(CELL) + HALF_CELLS).clamp(0, DIM - 1)
}

impl Grid {
    pub fn rebuild(&mut self, xs: &[i32], ys: &[i32], alive: impl Fn(usize) -> bool) {
        let n_cells = (DIM * DIM) as usize;
        self.start.clear();
        self.start.resize(n_cells + 1, 0);
        self.cell_of.clear();
        self.cell_of.resize(xs.len(), u32::MAX);
        for i in 0..xs.len() {
            if !alive(i) {
                continue;
            }
            let c = (cell_coord(ys[i]) * DIM + cell_coord(xs[i])) as u32;
            self.cell_of[i] = c;
            self.start[c as usize + 1] += 1;
        }
        for c in 0..n_cells {
            self.start[c + 1] += self.start[c];
        }
        self.items.clear();
        self.items.resize(self.start[n_cells] as usize, 0);
        let mut fill = self.start.clone();
        for i in 0..xs.len() {
            let c = self.cell_of[i];
            if c == u32::MAX {
                continue;
            }
            let slot = &mut fill[c as usize];
            self.items[*slot as usize] = i as u32;
            *slot += 1;
        }
    }

    /// Call `f` for every item in the cells overlapping the square of
    /// half-size `r` around (x, y). Stops early when `f` returns false.
    #[inline]
    pub fn query(&self, x: i32, y: i32, r: i32, mut f: impl FnMut(u32) -> bool) {
        if self.items.is_empty() {
            return;
        }
        let (x0, x1) = (cell_coord(x - r), cell_coord(x + r));
        let (y0, y1) = (cell_coord(y - r), cell_coord(y + r));
        for cy in y0..=y1 {
            let row = cy * DIM;
            for cx in x0..=x1 {
                let c = (row + cx) as usize;
                let (s, e) = (self.start[c] as usize, self.start[c + 1] as usize);
                for &it in &self.items[s..e] {
                    if !f(it) {
                        return;
                    }
                }
            }
        }
    }
}
