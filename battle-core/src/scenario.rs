//! Ready-made battles for tests, benchmarks and the debug viewer.

use crate::battle::Battle;
use crate::fixed::hash;
use crate::fixed::ANG_HALF;
use crate::terrain::{field_of, ASTEROIDS, GRAVITY, NEBULA, PLANET};
use crate::types::{Doctrine, ShipClass::*};

/// Group names say the doctrine and what the group is made of.
const GUNS: &str = "砲列：旗艦、戰艦、支援艦";
const ARTILLERY: &str = "遠砲：長程砲艦、支援艦";
const CARRIERS: &str = "航艦：航艦、驅逐艦";
const ANVIL: &str = "鐵砧：巡洋艦";
const HAMMER: &str = "鐵鎚：驅逐艦";
const RESERVE: &str = "預備：巡洋艦、驅逐艦";

/// A mid-sized fleet battle between two identical fleets (41 ships each, one
/// reserve each) deployed in different formations, plus each side's
/// structures (a space station by the central planet, a jump beacon out on
/// one flank and a pylon powering an energy field around the gun line).
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
/// - A planet sits in the middle of the field inside a gravity well.
///
/// Every battle shifts each group a little by the seed (see `jitter`), so
/// the balance reflects the formations rather than one exact placement.
///
/// `scale` multiplies every group's ship count (scale 1 = 82 ships and 6
/// structures in total).
pub fn demo(seed: u64, scale: u32) -> Battle {
    demo_with(seed, scale, &RED_LAYOUT)
}

/// Red's formation: (x, y) for guns, carriers, anvil, hammer, reserve,
/// gunships, in scale-1 world units (y is multiplied by the spread).
pub const RED_LAYOUT: [(i32, i32); 6] = [
    (3000, 0),
    (4400, 0),
    (2200, 900),
    (2800, -2000),
    (3600, 1800),
    (5000, 0),
];

pub fn demo_with(seed: u64, scale: u32, red_at: &[(i32, i32); 6]) -> Battle {
    let k = scale.max(1);
    let spread = (k as f64).sqrt() as i32; // bigger fleets need more room
    let mut b = Battle::new(seed);
    // The planet's gravity well goes down first; the rest paints over it.
    b.terrain
        .paint_ellipse(GRAVITY, 0, 0, 1400 * spread, 1400 * spread);
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
        // Each side's energy field around its gun line.
        let side = if dir < 0 { 0 } else { 1 };
        b.terrain
            .paint_ellipse(field_of(side), 3000 * dir, 0, 900, 900 * spread);
    }
    b.terrain
        .paint_ellipse(PLANET, 0, 0, 400 * spread, 400 * spread);
    let guns = [(Flagship, k), (Battleship, 3 * k), (Support, k)];
    let artillery = [(Artillery, 3 * k), (Support, k)];
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
    blue(
        ARTILLERY,
        Doctrine::Line,
        false,
        &artillery,
        -4400,
        1700 * s,
    );
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
    red(
        ARTILLERY,
        Doctrine::Line,
        false,
        &artillery,
        l[5].0,
        l[5].1 * s,
    );
    // Structures, placed the same for both sides (turned 180 degrees).
    for (side, dir) in [(0u8, -1), (1u8, 1)] {
        let (tag, facing) = if side == 0 {
            ("藍", 0)
        } else {
            ("紅", ANG_HALF)
        };
        let mut fixed = |name: &str, class, x: i32, y: i32| {
            b.add_group(
                side,
                &format!("{tag}・{name}"),
                Doctrine::Line,
                false,
                &[(class, 1)],
                x * dir,
                y * s * dir,
                facing,
            );
        };
        fixed("空間站", Station, 1100, -700);
        fixed("跳躍信標", Beacon, 3500, 2600);
        fixed("能量場塔", Pylon, 3300, 0);
    }
    b
}
