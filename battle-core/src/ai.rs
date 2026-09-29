//! The commander: spends a side's command points at each pulse.
//!
//! Utility scoring: every possible command gets a score, the best ones are
//! bought until the points run out. The enemy uses exactly the same points and
//! rules as the player, and the same code resolves battles automatically.

use crate::battle::{Battle, Group, GroupStatus, ALIVE};
use crate::fixed::*;
use crate::types::*;

fn hull_permille(b: &Battle, g: &Group) -> i64 {
    let (mut now, mut max) = (0i64, 0i64);
    for &id in &g.ships {
        let i = id as usize;
        max += b.ships.stats(i).hull as i64;
        if b.ships.state[i] == ALIVE {
            now += b.ships.hull[i] as i64;
        }
    }
    if max == 0 {
        0
    } else {
        now * 1000 / max
    }
}

fn dist(a: &Group, b: &Group) -> i64 {
    len((a.cx - b.cx) as i64, (a.cy - b.cy) as i64)
}

pub fn plan(b: &Battle, side: u8) -> Vec<Command> {
    let mut options: Vec<(i32, Command)> = Vec::new();
    let mine: Vec<&Group> = b.groups.iter().filter(|g| g.side == side).collect();
    let theirs: Vec<&Group> = b
        .groups
        .iter()
        .filter(|g| g.side != side && g.status.in_battle() && g.alive > 0)
        .collect();
    if theirs.is_empty() {
        return Vec::new();
    }
    let front_shaken = mine
        .iter()
        .any(|g| g.status == GroupStatus::Active && g.cohesion < 550);

    for g in &mine {
        match g.status {
            GroupStatus::Active => {
                let hp = hull_permille(b, g);
                if g.cohesion < 250 && hp < 450 {
                    options.push((90, Command::Retreat { group: g.id }));
                    continue;
                }
                let nearest = theirs.iter().min_by_key(|t| (dist(g, t), t.id)).unwrap();
                // Hammers go for the flank of the most valuable enemy group.
                if g.doctrine == Doctrine::Hammer && !matches!(g.order, Order::Flank { .. }) {
                    let prize = theirs
                        .iter()
                        .filter(|t| t.doctrine != Doctrine::Hammer)
                        .max_by_key(|t| (t.init_cost, std::cmp::Reverse(t.id)))
                        .unwrap_or(nearest);
                    // Pick the flank that is closer to us.
                    let l = polar(1000 * FP as i64, prize.facing.wrapping_add(ANG_QUARTER));
                    let r = polar(1000 * FP as i64, prize.facing.wrapping_sub(ANG_QUARTER));
                    let dl = len(
                        prize.cx as i64 + l.0 - g.cx as i64,
                        prize.cy as i64 + l.1 - g.cy as i64,
                    );
                    let dr = len(
                        prize.cx as i64 + r.0 - g.cx as i64,
                        prize.cy as i64 + r.1 - g.cy as i64,
                    );
                    options.push((
                        60,
                        Command::Flank {
                            group: g.id,
                            target: prize.id,
                            left: dl <= dr,
                        },
                    ));
                }
                // Gun lines knock out the weapons of whatever they are shooting.
                if g.doctrine == Doctrine::Line && g.focus_part.is_none() {
                    let part = if nearest.doctrine == Doctrine::Hammer {
                        Part::Engine
                    } else {
                        Part::Weapons
                    };
                    options.push((30, Command::FocusPart { group: g.id, part }));
                }
                // Anvils pin the enemy group that is closest to the flagship line.
                if g.doctrine == Doctrine::Anvil && g.order == Order::Engage && b.pulse() >= 1 {
                    options.push((
                        25,
                        Command::Attack {
                            group: g.id,
                            target: nearest.id,
                        },
                    ));
                }
            }
            GroupStatus::Reserve => {
                if front_shaken || b.pulse() >= 3 {
                    options.push((70, Command::CommitReserve { group: g.id }));
                }
            }
            _ => {}
        }
    }
    options.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.group().cmp(&b.1.group())));
    let mut points = b.sides[side as usize].command_points;
    let mut picked = Vec::new();
    let mut used = Vec::new();
    for (_, cmd) in options {
        if used.contains(&cmd.group()) || cmd.cost() > points {
            continue;
        }
        points -= cmd.cost();
        used.push(cmd.group());
        picked.push(cmd);
    }
    picked
}
