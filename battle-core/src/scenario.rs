//! Ready-made battles for tests, benchmarks and the debug viewer.

use crate::battle::Battle;
use crate::fixed::ANG_HALF;
use crate::types::{Doctrine, ShipClass::*};

/// A mid-sized fleet battle: four groups a side, one of them in reserve.
/// `scale` multiplies every group's ship count (scale 1 = 68 ships in total).
pub fn demo(seed: u64, scale: u32) -> Battle {
    let k = scale.max(1);
    let spread = (k as f64).sqrt() as i32; // bigger fleets need more room
    let mut b = Battle::new(seed);
    let (w, e) = (0u16, ANG_HALF);
    // Blue (side 0) deploys on the left facing right.
    b.add_group(
        0,
        "藍・中央砲列",
        Doctrine::Line,
        false,
        &[(Flagship, k), (Battleship, 4 * k)],
        -2400,
        0,
        w,
    );
    b.add_group(
        0,
        "藍・左舷鐵砧",
        Doctrine::Anvil,
        false,
        &[(Cruiser, 6 * k)],
        -2200,
        1800 * spread,
        w,
    );
    b.add_group(
        0,
        "藍・右舷鐵鎚",
        Doctrine::Hammer,
        false,
        &[(Destroyer, 12 * k)],
        -2200,
        -1800 * spread,
        w,
    );
    b.add_group(
        0,
        "藍・預備隊",
        Doctrine::Anvil,
        true,
        &[(Cruiser, 4 * k), (Destroyer, 6 * k)],
        -4200,
        0,
        w,
    );
    // Red (side 1) deploys on the right facing left, with a different mix.
    b.add_group(
        1,
        "紅・中央主力",
        Doctrine::Line,
        false,
        &[(Flagship, k), (Battleship, 3 * k), (Cruiser, 4 * k)],
        2400,
        0,
        e,
    );
    b.add_group(
        1,
        "紅・鐵鎚群",
        Doctrine::Hammer,
        false,
        &[(Destroyer, 16 * k)],
        2200,
        1800 * spread,
        e,
    );
    b.add_group(
        1,
        "紅・鐵砧群",
        Doctrine::Anvil,
        false,
        &[(Cruiser, 8 * k)],
        2200,
        -1800 * spread,
        e,
    );
    b.add_group(
        1,
        "紅・預備戰艦",
        Doctrine::Line,
        true,
        &[(Battleship, 3 * k)],
        4200,
        0,
        e,
    );
    b
}
