//! Command rules and battle length.

use battlecore::battle::GroupStatus;
use battlecore::scenario;
use battlecore::types::*;

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
    for _ in 0..PULSE_TICKS {
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
        let pulses = out.tick.div_ceil(PULSE_TICKS);
        assert!((4..=10).contains(&pulses), "seed {seed}: {pulses} pulses");
    }
}
