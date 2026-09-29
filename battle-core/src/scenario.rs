//! Ready-made battles for tests, benchmarks and the debug viewer.

use crate::battle::Battle;
use crate::fixed::ANG_HALF;
use crate::terrain::{ASTEROIDS, NEBULA};
use crate::types::{Doctrine, ShipClass::*};

/// A mid-sized fleet battle between two identical fleets, one reserve each.
///
/// Red's half of the field is blue's half turned 180 degrees around the centre,
/// terrain included, so each fleet sees exactly the same battlefield from its
/// own side: its anvil on the left, its hammer on the right, a nebula ahead to
/// the left and an asteroid field on the right flank. (A left-right mirror
/// would not be fair: it swaps which hand each fleet's wings are on.)
///
/// `scale` multiplies every group's ship count (scale 1 = 72 ships in total).
pub fn demo(seed: u64, scale: u32) -> Battle {
    let k = scale.max(1);
    let spread = (k as f64).sqrt() as i32; // bigger fleets need more room
    let mut b = Battle::new(seed);
    for dir in [-1, 1] {
        b.terrain.paint_ellipse(
            NEBULA,
            -1300 * dir,
            2700 * spread * dir,
            1000 * spread,
            800 * spread,
        );
        b.terrain.paint_ellipse(
            ASTEROIDS,
            -1300 * dir,
            -2600 * spread * dir,
            700 * spread,
            900 * spread,
        );
    }
    // Blue (side 0) deploys on the left facing right; red on the right facing left.
    for (side, name, dir, facing) in [(0u8, "藍", 1, 0u16), (1, "紅", -1, ANG_HALF)] {
        // (x, y) are blue's coordinates; red gets them turned around the centre.
        let mut group = |label: &str, doctrine, reserve, ships: &[_], x: i32, y: i32| {
            b.add_group(
                side,
                &format!("{name}・{label}"),
                doctrine,
                reserve,
                ships,
                x * dir,
                y * dir,
                facing,
            );
        };
        group(
            "中央砲列",
            Doctrine::Line,
            false,
            &[(Flagship, k), (Battleship, 3 * k)],
            -2400,
            0,
        );
        group(
            "航艦群",
            Doctrine::Line,
            false,
            &[(Carrier, 2 * k), (Destroyer, 4 * k)],
            -4000,
            700 * spread,
        );
        group(
            "左翼鐵砧",
            Doctrine::Anvil,
            false,
            &[(Cruiser, 6 * k)],
            -2200,
            1800 * spread,
        );
        group(
            "右翼鐵鎚",
            Doctrine::Hammer,
            false,
            &[(Destroyer, 12 * k)],
            -2200,
            -1800 * spread,
        );
        group(
            "預備隊",
            Doctrine::Anvil,
            true,
            &[(Cruiser, 4 * k), (Destroyer, 4 * k)],
            -4400,
            -700 * spread,
        );
    }
    b
}
