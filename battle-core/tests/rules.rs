//! Command rules and battle length.

use battlecore::battle::{GroupStatus, W_LOST, W_OUT};
use battlecore::scenario;
use battlecore::terrain::NEBULA;
use battlecore::types::*;
use battlecore::Battle;

/// Demo battle after deployment, with side 0 left to the test.
fn deployed() -> battlecore::Battle {
    let mut b = scenario::demo(1, 1);
    b.sides[0].ai = false;
    b.step();
    b
}

fn group_of(b: &battlecore::Battle, side: u8, reserve: bool) -> u16 {
    b.groups
        .iter()
        .find(|g| g.side == side && (g.status == GroupStatus::Reserve) == reserve)
        .unwrap()
        .id
}

#[test]
fn command_points_are_spent_and_refilled() {
    let mut b = deployed();
    assert_eq!(b.sides[0].command_points, COMMAND_POINTS);
    let g = group_of(&b, 0, false);
    b.issue(0, Command::Hold { group: g }).unwrap();
    assert_eq!(b.sides[0].command_points, COMMAND_POINTS - 1);
    let r = group_of(&b, 0, true);
    b.issue(0, Command::CommitReserve { group: r }).unwrap();
    assert_eq!(b.sides[0].command_points, COMMAND_POINTS - 3);
    assert_eq!(
        b.issue(0, Command::Hold { group: g }),
        Err(CommandError::NotEnoughPoints)
    );
    b.run_to_pulse();
    assert_eq!(b.sides[0].command_points, COMMAND_POINTS);
}

#[test]
fn invalid_commands_are_rejected_for_free() {
    let mut b = deployed();
    let mine = group_of(&b, 0, false);
    let theirs = group_of(&b, 1, false);
    assert_eq!(
        b.issue(0, Command::Hold { group: theirs }),
        Err(CommandError::NotYourGroup)
    );
    assert_eq!(
        b.issue(0, Command::Hold { group: 999 }),
        Err(CommandError::NoSuchGroup)
    );
    assert_eq!(
        b.issue(
            0,
            Command::Attack {
                group: mine,
                target: mine
            }
        ),
        Err(CommandError::BadTarget)
    );
    assert_eq!(
        b.issue(0, Command::CommitReserve { group: mine }),
        Err(CommandError::NotInReserve)
    );
    assert_eq!(b.sides[0].command_points, COMMAND_POINTS);
}

#[test]
fn retreat_is_free_locks_the_group_then_escapes() {
    let mut b = deployed();
    let g = group_of(&b, 0, false);
    b.issue(0, Command::Retreat { group: g }).unwrap();
    assert_eq!(b.sides[0].command_points, COMMAND_POINTS);
    assert!(matches!(
        b.groups[g as usize].status,
        GroupStatus::Retreating { .. }
    ));
    assert_eq!(
        b.issue(0, Command::Hold { group: g }),
        Err(CommandError::GroupRetreating)
    );
    let ships: Vec<u32> = b.groups[g as usize].ships.clone();
    for _ in 0..RETREAT_TICKS {
        b.step();
        for e in &b.events {
            if e.kind == ev::FIRE || e.kind == ev::MISSILE_LAUNCH {
                assert!(!ships.contains(&e.a), "a retreating ship fired");
            }
        }
        if b.outcome.is_some() {
            break;
        }
    }
    let st = b.groups[g as usize].status;
    assert!(
        matches!(st, GroupStatus::Escaped | GroupStatus::Destroyed),
        "status after lock: {st:?}"
    );
}

#[test]
fn demo_battles_end_in_a_few_pulses() {
    for seed in 1..=4 {
        let mut b = scenario::demo(seed, 1);
        let out = b.run_auto();
        let seconds = out.tick / TICK_HZ;
        assert!((60..=300).contains(&seconds), "seed {seed}: {seconds} s");
    }
}

#[test]
fn signature_waits_for_contact_charges_fires_then_cools_down() {
    let mut b = deployed();
    let g = group_of(&b, 0, false); // the gun line, still far from the enemy
    b.issue(0, Command::Signature { group: g }).unwrap();
    assert_eq!(b.sides[0].command_points, COMMAND_POINTS - 1);
    assert_eq!(
        b.issue(0, Command::Signature { group: g }),
        Err(CommandError::SignatureNotReady)
    );
    let (mut charged, mut fired) = (None, None);
    for _ in 0..PULSE_TICKS {
        b.step();
        for e in b.events.iter().filter(|e| e.a == g as u32) {
            if e.kind == ev::SIGNATURE_CHARGING {
                charged = Some(e.tick);
            }
            if e.kind == ev::SIGNATURE_FIRED {
                fired = Some(e.tick);
            }
        }
        if fired.is_some() || b.outcome.is_some() {
            break;
        }
    }
    let charged = charged.expect("never came in reach");
    assert!(
        charged > 20 * TICK_HZ,
        "should wait for the enemy, charged at {charged}"
    );
    assert_eq!(fired, Some(charged + SIG_CHARGE_TICKS));
    b.step();
    assert_eq!(
        b.issue(0, Command::Signature { group: g }),
        Err(CommandError::SignatureNotReady)
    );
}

#[test]
fn cruiser_signature_launches_torpedoes() {
    let mut b = deployed();
    let cruisers = b
        .groups
        .iter()
        .find(|g| g.side == 0 && g.name.contains("鐵砧"))
        .unwrap()
        .id;
    for _ in 0..30 * TICK_HZ {
        b.step(); // close the distance first
    }
    b.issue(0, Command::Signature { group: cruisers }).unwrap();
    let mut torpedoes = 0;
    for _ in 0..SIG_CHARGE_TICKS + 2 {
        b.step();
        torpedoes += b
            .events
            .iter()
            .filter(|e| e.kind == ev::MISSILE_LAUNCH && e.d == 1)
            .count();
    }
    assert!(torpedoes >= 2, "torpedoes launched: {torpedoes}");
}

#[test]
fn wings_launch_strike_and_die_with_their_carrier() {
    let mut b = scenario::demo(2, 1);
    let (mut launched, mut strikes) = (0, 0);
    while b.outcome.is_none() {
        b.step();
        for e in &b.events {
            launched += (e.kind == ev::WING_LAUNCHED) as u32;
            strikes += (e.kind == ev::WING_STRIKE) as u32;
        }
    }
    assert!(
        launched >= 4 && strikes > 10,
        "launched {launched}, strikes {strikes}"
    );
    for w in 0..b.wings.len() {
        let c = b.wings.carrier[w] as usize;
        if b.ships.state[c] == battlecore::battle::DEAD {
            assert_eq!(b.wings.state[w], W_LOST);
        }
    }
}

#[test]
fn nebula_hides_ships_beyond_close_range() {
    let fire_count = |nebula: bool| {
        let mut b = Battle::new(1);
        if nebula {
            b.terrain.paint_ellipse(NEBULA, 0, 0, 600, 600);
        }
        b.add_group(
            0,
            "gun",
            Doctrine::Line,
            false,
            &[(ShipClass::Battleship, 1)],
            -2000,
            0,
            0,
        );
        b.add_group(
            1,
            "hidden",
            Doctrine::Line,
            false,
            &[(ShipClass::Destroyer, 1)],
            0,
            0,
            0,
        );
        b.sides = [battlecore::battle::SideState {
            command_points: 0,
            ai: false,
        }; 2];
        b.step();
        b.issue(0, Command::Hold { group: 0 }).unwrap();
        b.issue(1, Command::Hold { group: 1 }).unwrap();
        let mut shots = 0;
        for _ in 0..200 {
            b.step();
            shots += b
                .events
                .iter()
                .filter(|e| e.kind == ev::FIRE && e.a == 0)
                .count();
        }
        shots
    };
    assert!(fire_count(false) > 0);
    assert_eq!(fire_count(true), 0);
}

#[test]
fn demo_is_roughly_balanced() {
    let mut wins = [0; 2];
    // Both fleets are identical, so over 100 seeds each side should win
    // well over a third (fair play gives 50 +- 5).
    for seed in 1..=100 {
        let o = scenario::demo(seed, 1).run_auto();
        if o.winner >= 0 {
            wins[o.winner as usize] += 1;
        }
    }
    assert!(
        wins[0] >= 35 && wins[1] >= 35,
        "blue {} red {}",
        wins[0],
        wins[1]
    );
}

#[test]
fn carriers_send_one_wave_at_a_time_until_they_scramble() {
    let mut b = scenario::demo(3, 1);
    b.sides[0].ai = true;
    b.sides[1].ai = true;
    let mut scrambled = vec![false; b.groups.len()];
    let mut launches = 0;
    while b.outcome.is_none() && b.tick < 90 * TICK_HZ {
        b.step();
        for e in &b.events {
            if e.kind == ev::SIGNATURE_FIRED {
                scrambled[e.a as usize] = true;
            }
            launches += (e.kind == ev::WING_LAUNCHED) as u32;
        }
        let w = &b.wings;
        for c in 0..b.ships.len() {
            let g = b.ships.group[c] as usize;
            let out = (0..w.len())
                .filter(|&k| w.carrier[k] == c as u32 && w.state[k] == W_OUT)
                .count();
            assert!(
                scrambled[g] || out <= WING_WAVE as usize,
                "tick {}: carrier {c} has {out} wings out",
                b.tick
            );
        }
    }
    assert!(launches >= 4, "launches {launches}");
}
