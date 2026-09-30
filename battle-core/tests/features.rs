//! Ship types, doctrine phases, height layers and terrain added after the
//! first prototype: long-range gunships, support ships, structures, the low
//! layer, gravity wells, planets, debris and energy fields.

use battlecore::battle::{GroupStatus, SideState, ALIVE};
use battlecore::scenario;
use battlecore::terrain::{self, GRAVITY, PLANET};
use battlecore::types::*;
use battlecore::Battle;

/// Two groups facing each other, nobody's commander running, orders free.
fn duel(blue: &[(ShipClass, u32)], bx: i32, red: &[(ShipClass, u32)], rx: i32) -> Battle {
    let mut b = Battle::new(3);
    b.add_group(0, "blue", Doctrine::Line, false, blue, bx, 0, 0);
    b.add_group(1, "red", Doctrine::Line, false, red, rx, 0, fixed_half());
    b.sides = [SideState {
        command_points: 0,
        ai: false,
        terrain_sense: true,
    }; 2];
    b.step();
    b
}

fn fixed_half() -> u16 {
    battlecore::fixed::ANG_HALF
}

fn run(b: &mut Battle, ticks: u32, mut f: impl FnMut(&Battle)) {
    for _ in 0..ticks {
        b.step();
        f(b);
        b.sides[0].command_points = COMMAND_POINTS;
        b.sides[1].command_points = COMMAND_POINTS;
    }
}

#[test]
fn gunships_fire_only_after_holding_still() {
    let mut b = duel(
        &[(ShipClass::Artillery, 1)],
        -4000,
        &[(ShipClass::Battleship, 1)],
        0,
    );
    b.issue(1, Command::Hold { group: 1 }).unwrap();
    // Moving across the field: in range the whole way, never fires.
    b.issue(
        0,
        Command::Advance {
            group: 0,
            x: -4000,
            y: 3000,
        },
    )
    .unwrap();
    let mut shots = 0;
    run(&mut b, 100, |b| {
        shots += b
            .events
            .iter()
            .filter(|e| e.kind == ev::FIRE && e.a == 0)
            .count();
    });
    assert_eq!(shots, 0, "fired while moving");
    // Stop: after deploying (3 s) it opens fire from 4,000+ units.
    b.issue(0, Command::Hold { group: 0 }).unwrap();
    let mut first = None;
    let start = b.tick;
    run(&mut b, 300, |b| {
        if first.is_none() && b.events.iter().any(|e| e.kind == ev::FIRE && e.a == 0) {
            first = Some(b.tick);
        }
    });
    let first = first.expect("never fired after stopping");
    assert!(
        first - start >= 3 * TICK_HZ,
        "fired {} ticks after stopping",
        first - start
    );
}

#[test]
fn barrage_is_marked_then_lands_on_the_spot() {
    let mut b = duel(
        &[(ShipClass::Artillery, 3)],
        -4000,
        &[(ShipClass::Battleship, 3)],
        0,
    );
    b.issue(1, Command::Hold { group: 1 }).unwrap();
    b.issue(0, Command::Hold { group: 0 }).unwrap();
    b.issue(0, Command::Signature { group: 0 }).unwrap();
    let (mut marked, mut landed) = (None, None);
    run(&mut b, 400, |b| {
        for e in &b.events {
            if e.kind == ev::BARRAGE_MARKED {
                marked = Some(e.tick);
            }
            if e.kind == ev::BARRAGE_HIT {
                landed = Some((e.tick, e.c));
            }
        }
    });
    let marked = marked.expect("no barrage marked");
    let (at, hit) = landed.expect("barrage never landed");
    assert_eq!(at - marked, BARRAGE_DELAY);
    assert!(hit >= 1, "barrage hit {hit} ships");
}

#[test]
fn support_ships_repair_the_worst_hurt_ship_nearby() {
    let mut b = duel(
        &[(ShipClass::Battleship, 1), (ShipClass::Support, 1)],
        -9000,
        &[(ShipClass::Destroyer, 1)],
        9000,
    );
    b.ships.hull[0] = 1000;
    run(&mut b, 5 * TICK_HZ, |_| {});
    assert!(b.ships.hull[0] >= 1000 + 4 * ShipClass::Support.stats().repair);
}

#[test]
fn shield_boost_refills_nearby_shields() {
    let mut b = duel(
        &[(ShipClass::Cruiser, 2), (ShipClass::Support, 1)],
        -3000,
        &[(ShipClass::Battleship, 1)],
        0,
    );
    for i in 0..3 {
        b.ships.shield[i] = [0; 4];
    }
    b.issue(0, Command::Signature { group: 0 }).unwrap();
    let mut boosted = 0;
    run(&mut b, 200, |b| {
        for e in &b.events {
            if e.kind == ev::SHIELD_BOOST {
                boosted = e.c;
            }
        }
    });
    assert!(boosted >= 3, "boosted {boosted}");
}

#[test]
fn worn_down_groups_pull_back_refit_and_return() {
    let mut b = duel(
        &[(ShipClass::Battleship, 4)],
        -2600,
        &[(ShipClass::Battleship, 4)],
        0,
    );
    b.issue(1, Command::Hold { group: 1 }).unwrap();
    // Fight on its own long enough to be allowed to pull back.
    run(&mut b, 25 * TICK_HZ, |_| {});
    assert_eq!(b.groups[0].phase, Phase::Engage);
    for &id in &b.groups[0].ships.clone() {
        b.ships.shield[id as usize] = [0; 4];
    }
    let mut seen = Vec::new();
    run(&mut b, 60 * TICK_HZ, |b| {
        for e in &b.events {
            if e.kind == ev::PHASE && e.a == 0 {
                seen.push(e.c);
            }
        }
    });
    assert!(seen.len() >= 2, "phases {seen:?}");
    assert_eq!(seen[0], Phase::Disengage as i32, "phases {seen:?}");
    assert_eq!(seen[1], Phase::Refit as i32, "phases {seen:?}");
}

#[test]
fn changing_layer_takes_time_and_shots_across_layers_hit_a_side_shield() {
    let mut b = duel(
        &[(ShipClass::Battleship, 1)],
        -1800,
        &[(ShipClass::Battleship, 1)],
        0,
    );
    b.issue(0, Command::Hold { group: 0 }).unwrap();
    b.issue(1, Command::Hold { group: 1 }).unwrap();
    b.issue(1, Command::ChangeLayer { group: 1 }).unwrap();
    assert_eq!(
        b.issue(1, Command::ChangeLayer { group: 1 }),
        Err(CommandError::ChangingLayer)
    );
    assert_ne!(b.ships.layer[1] & LAYER_MOVING, 0);
    run(&mut b, LAYER_CHANGE_TICKS, |_| {});
    assert_eq!(b.ships.layer[1], LAYER_LOW);
    assert_eq!(b.groups[1].layer, LAYER_LOW);
    // Now blue shoots down at red: red's front shield is spared.
    let st = ShipClass::Battleship.stats();
    b.ships.shield[1] = [st.shield[0], st.shield[1], st.shield[2], st.shield[1]];
    b.ships.overload[1] = 0;
    let mut hits = 0;
    run(&mut b, 200, |b| {
        hits += b
            .events
            .iter()
            .filter(|e| e.kind == ev::FIRE && e.a == 0 && e.c & 1 == 1)
            .count();
    });
    assert!(hits > 0);
    let sh = b.ships.shield[1];
    assert!(
        sh[1] < st.shield[1] || sh[3] < st.shield[1],
        "side shields {sh:?}"
    );
}

#[test]
fn no_jumping_out_of_a_gravity_well() {
    let mut b = duel(
        &[(ShipClass::Cruiser, 3)],
        -2000,
        &[(ShipClass::Cruiser, 3)],
        6000,
    );
    b.terrain.paint_ellipse(GRAVITY, -2000, 0, 1500, 1500);
    b.step();
    assert_eq!(
        b.issue(0, Command::Retreat { group: 0 }),
        Err(CommandError::InGravityWell)
    );
    assert!(b.issue(1, Command::Retreat { group: 1 }).is_ok());
}

#[test]
fn planets_block_shots_and_ships() {
    let mut b = duel(
        &[(ShipClass::Battleship, 1)],
        -2000,
        &[(ShipClass::Destroyer, 1)],
        0,
    );
    b.terrain.paint_ellipse(PLANET, -1000, 0, 400, 400);
    b.issue(0, Command::Hold { group: 0 }).unwrap();
    b.issue(1, Command::Hold { group: 1 }).unwrap();
    let mut shots = 0;
    run(&mut b, 200, |b| {
        shots += b
            .events
            .iter()
            .filter(|e| e.kind == ev::FIRE && e.a == 0)
            .count();
    });
    assert_eq!(shots, 0);
    // Sent straight through it, a group goes round and no ship enters it.
    b.issue(
        1,
        Command::Advance {
            group: 1,
            x: -2000,
            y: 0,
        },
    )
    .unwrap();
    let mut inside = 0;
    run(&mut b, 60 * TICK_HZ, |b| {
        let s = &b.ships;
        inside += (0..s.len())
            .filter(|&i| s.state[i] == ALIVE && b.terrain.solid(s.x[i], s.y[i]))
            .count();
    });
    assert_eq!(inside, 0);
}

#[test]
fn demo_structures_debris_and_energy_fields_work() {
    let mut b = scenario::demo(5, 1);
    let fixed: Vec<_> = b.groups.iter().filter(|g| g.fixed).map(|g| g.id).collect();
    assert_eq!(fixed.len(), 6);
    // Structures take no orders.
    b.step();
    assert_eq!(
        b.issue(0, Command::Hold { group: fixed[0] }),
        Err(CommandError::Fixed)
    );
    // Big wrecks leave debris.
    let mut debris = 0;
    while b.outcome.is_none() && debris == 0 {
        b.step();
        debris += b.events.iter().filter(|e| e.kind == ev::DEBRIS).count();
    }
    assert!(debris > 0);
}

#[test]
fn energy_field_recharges_its_side_and_dies_with_its_pylon() {
    let field = terrain::field_of(0);
    // Inside its own field, a ship's shields come back three times as fast;
    // inside the enemy's, not at all.
    let regen_in = |kind: u8| {
        let mut b = duel(
            &[(ShipClass::Cruiser, 1)],
            -1500,
            &[(ShipClass::Cruiser, 1)],
            9000,
        );
        b.terrain.paint_ellipse(kind, -1500, 0, 600, 600);
        b.ships.shield[0] = [0; 4];
        run(&mut b, 20, |_| {});
        b.ships.shield[0][0]
    };
    let regen = ShipClass::Cruiser.stats().shield_regen * 20;
    assert_eq!(regen_in(terrain::EMPTY), regen);
    assert_eq!(regen_in(field), regen * 3);
    assert_eq!(regen_in(terrain::field_of(1)), 0);

    // Kill the pylon and the field goes.
    let mut b = Battle::new(3);
    b.add_group(
        0,
        "pylon",
        Doctrine::Line,
        false,
        &[(ShipClass::Pylon, 1)],
        -1500,
        0,
        0,
    );
    b.add_group(
        0,
        "far",
        Doctrine::Line,
        false,
        &[(ShipClass::Cruiser, 1)],
        -9000,
        5000,
        0,
    );
    let half = fixed_half();
    b.add_group(
        1,
        "guns",
        Doctrine::Line,
        false,
        &[(ShipClass::Battleship, 2)],
        0,
        0,
        half,
    );
    b.sides = [SideState {
        command_points: 0,
        ai: false,
        terrain_sense: true,
    }; 2];
    b.terrain.paint_ellipse(field, -1500, 0, 600, 600);
    b.step();
    b.issue(1, Command::Hold { group: 2 }).unwrap();
    b.issue(0, Command::Hold { group: 1 }).unwrap();
    let mut lost = false;
    run(&mut b, 120 * TICK_HZ, |b| {
        lost |= b.events.iter().any(|e| e.kind == ev::FIELD_LOST);
    });
    assert!(lost, "pylon survived");
    assert!(!b.terrain.cells.contains(&field));
}

#[test]
fn structures_alone_do_not_hold_the_field() {
    let b = {
        let mut b = duel(
            &[(ShipClass::Station, 1)],
            -9000,
            &[(ShipClass::Destroyer, 1)],
            9000,
        );
        b.step();
        b
    };
    assert_eq!(b.outcome.map(|o| o.winner), Some(1));
}

#[test]
fn committed_reserves_jump_in_at_the_beacon() {
    let mut b = scenario::demo(1, 1);
    b.sides[0].ai = false;
    b.step();
    let reserve = b
        .groups
        .iter()
        .find(|g| g.side == 0 && g.status == GroupStatus::Reserve)
        .unwrap()
        .id;
    let beacon = (0..b.ships.len())
        .find(|&i| b.ships.class[i] == ShipClass::Beacon as u8 && b.ships.side[i] == 0)
        .unwrap();
    b.issue(0, Command::CommitReserve { group: reserve })
        .unwrap();
    assert!(b.events.iter().any(|e| e.kind == ev::JUMP_IN));
    let g = &b.groups[reserve as usize];
    let d = ((g.cx - b.ships.x[beacon]) as f64).hypot((g.cy - b.ships.y[beacon]) as f64) / 100.0;
    assert!(d < 1000.0, "reserve {d} units from the beacon");
}
