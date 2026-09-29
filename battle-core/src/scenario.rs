//! Ready-made battles for tests, benchmarks and the debug viewer.

use crate::battle::Battle;
use crate::fixed::hash;
use crate::fixed::ANG_HALF;
use crate::terrain::{ASTEROIDS, NEBULA};
use crate::types::{Doctrine, ShipClass::*};

/// Group names say the doctrine and what the group is made of.
const GUNS: &str = "砲列：旗艦、戰艦";
const CARRIERS: &str = "航艦：航艦、驅逐艦";
const ANVIL: &str = "鐵砧：巡洋艦";
const HAMMER: &str = "鐵鎚：驅逐艦";
const RESERVE: &str = "預備：巡洋艦、驅逐艦";

/// A mid-sized fleet battle between two identical fleets (36 ships each, one
/// reserve each) deployed in different formations.
///
/// Terrain is fair: red's half of the field is blue's half turned 180 degrees
/// around the centre. Each side has, from its own point of view, a nebula
/// far out on the left and a small one close in on the right, a rock outcrop
/// ahead on the left of the centre line, and an asteroid field out on the
/// right flank. Rock blocks direct fire, so the outcrops split the field.
///
/// - Blue spreads out: anvil on the left wing, hammer on the right wing,
///   carriers behind the left, reserve behind the right.
/// - Red holds a deep centre: gun line in the middle with its anvil just
///   ahead on the right, carriers straight behind; hammer out on its left
///   wing, reserve behind its right.
///
/// Every battle shifts each group a little by the seed (see `jitter`), so
/// the balance reflects the formations rather than one exact placement.
///
/// `scale` multiplies every group's ship count (scale 1 = 72 ships in total).
pub fn demo(seed: u64, scale: u32) -> Battle {
    demo_with(seed, scale, &RED_LAYOUT)
}

/// Red's formation: (x, y) for guns, carriers, anvil, hammer, reserve, in
/// scale-1 world units (y is multiplied by the spread).
pub const RED_LAYOUT: [(i32, i32); 5] = [
    (3000, 0),
    (4400, 0),
    (2200, 900),
    (2800, -2000),
    (3600, 1800),
];

pub fn demo_with(seed: u64, scale: u32, red_at: &[(i32, i32); 5]) -> Battle {
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
        // Between the fleets: a rock outcrop on each side of the centre line,
        // and a nebula a short walk from each fleet's line.
        b.terrain.paint_ellipse(
            ASTEROIDS,
            -500 * dir,
            1300 * spread * dir,
            450 * spread,
            650 * spread,
        );
        b.terrain.paint_ellipse(
            NEBULA,
            -1700 * dir,
            -1200 * spread * dir,
            650 * spread,
            550 * spread,
        );
    }
    let guns = [(Flagship, k), (Battleship, 3 * k)];
    let carriers = [(Carrier, 2 * k), (Destroyer, 4 * k)];
    let anvil = [(Cruiser, 6 * k)];
    let hammer = [(Destroyer, 12 * k)];
    let reserve = [(Cruiser, 4 * k), (Destroyer, 4 * k)];
    let s = spread;
    // Each battle shifts every group a little (up to 300 units across, 400
    // along the line), picked by the seed. Without it, one layout decides
    // most battles the same way: moving one group 40 units could swing the
    // win rate from 40% to 80%.
    let jitter = |n: u32| {
        let h = hash(seed, 0, n, 0x6a17);
        let dx = (h % 601) as i32 - 300;
        let dy = ((h >> 20) % 801) as i32 - 400;
        (dx * s, dy * s)
    };
    let mut n = 0;
    // Blue (side 0) on the left facing right. Coordinates are world units.
    let mut blue = |label: &str, d, r, ships: &[_], x: i32, y: i32| {
        let (dx, dy) = jitter(n);
        n += 1;
        b.add_group(0, &format!("藍・{label}"), d, r, ships, x + dx, y + dy, 0);
    };
    blue(GUNS, Doctrine::Line, false, &guns, -2400, 0);
    blue(CARRIERS, Doctrine::Line, false, &carriers, -4000, 700 * s);
    blue(ANVIL, Doctrine::Anvil, false, &anvil, -2200, 1800 * s);
    blue(HAMMER, Doctrine::Hammer, false, &hammer, -2200, -1800 * s);
    blue(RESERVE, Doctrine::Anvil, true, &reserve, -4400, -700 * s);
    // Red (side 1) on the right facing left: its left is south, its right north.
    let mut red = |label: &str, d, r, ships: &[_], x: i32, y: i32| {
        let (dx, dy) = jitter(n);
        n += 1;
        b.add_group(
            1,
            &format!("紅・{label}"),
            d,
            r,
            ships,
            x + dx,
            y + dy,
            ANG_HALF,
        );
    };
    let l = red_at;
    red(GUNS, Doctrine::Line, false, &guns, l[0].0, l[0].1 * s);
    red(
        CARRIERS,
        Doctrine::Line,
        false,
        &carriers,
        l[1].0,
        l[1].1 * s,
    );
    red(ANVIL, Doctrine::Anvil, false, &anvil, l[2].0, l[2].1 * s);
    red(HAMMER, Doctrine::Hammer, false, &hammer, l[3].0, l[3].1 * s);
    red(RESERVE, Doctrine::Anvil, true, &reserve, l[4].0, l[4].1 * s);
    b
}
