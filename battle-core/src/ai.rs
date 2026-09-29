//! The commander: spends a side's command points at each pulse.
//!
//! Utility scoring: every possible command gets a score, the best ones are
//! bought until the points run out. The enemy uses exactly the same points and
//! rules as the player, and the same code resolves battles automatically.

use crate::battle::{Battle, Group, GroupStatus, ALIVE, W_DOCKED, W_LOST};
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

/// How much a group wants to use its signature move now, if it can.
fn signature_score(b: &Battle, g: &Group, nearest: &Group) -> Option<i32> {
    if g.sig_armed || g.charging() || b.tick < g.sig_ready_at {
        return None;
    }
    let d = dist(g, nearest);
    let (mut salvo, mut torpedo, mut burner, mut carriers) = (0, 0, 0, 0);
    for &id in &g.ships {
        let i = id as usize;
        if b.ships.state[i] != ALIVE {
            continue;
        }
        match b.ships.stats(i).signature {
            Signature::Salvo => salvo += 1,
            Signature::Torpedoes => torpedo += 1,
            Signature::Afterburn => burner += 1,
            Signature::Scramble => carriers += 1,
        }
    }
    let mut best = None;
    let mut consider = |score: i32| best = Some(best.map_or(score, |b: i32| b.max(score)));
    // Bought moves wait until the enemy is in reach, so buy them a pulse ahead.
    if salvo > 0 && d <= 6000 * FP as i64 {
        consider(55);
    }
    if torpedo >= 3 && d <= 6000 * FP as i64 {
        consider(50);
    }
    if burner * 2 > salvo + torpedo + carriers + burner
        && matches!(g.order, Order::Flank { .. })
        && d > 1800 * FP as i64
    {
        consider(45);
    }
    if carriers > 0 {
        // Worth it when two or more waves sit ready in the hangars (sending
        // them all beats one at a time), or when the wings are worn down.
        let (mut docked, mut have, mut max) = (0, 0, 0);
        let w = &b.wings;
        for k in 0..w.len() {
            if g.ships.contains(&w.carrier[k]) && w.state[k] != W_LOST {
                have += w.count[k] as i32;
                max += WING_SIZE as i32;
                docked += (w.state[k] == W_DOCKED) as i32;
            }
        }
        if max > 0 && have * 2 < max {
            consider(45);
        } else if docked >= 2 * WING_WAVE as i32 * carriers || b.pulse() >= 1 {
            consider(35);
        }
    }
    best
}

/// Extra distance an approach from `a` to `b` is worth: each stretch through
/// an asteroid field counts double, each stretch through a nebula counts half.
fn approach_cost(b: &Battle, a: (i32, i32), to: (i32, i32)) -> i64 {
    const STEP: i64 = 250 * FP as i64;
    let (dx, dy) = ((to.0 - a.0) as i64, (to.1 - a.1) as i64);
    let n = (len(dx, dy) / STEP).max(1);
    let mut extra = 0;
    for k in 1..=n {
        let p = (a.0 + (dx * k / n) as i32, a.1 + (dy * k / n) as i32);
        extra += match b.terrain.at(p.0, p.1) {
            crate::terrain::ASTEROIDS => STEP,
            crate::terrain::NEBULA => -STEP / 2,
            _ => 0,
        };
    }
    extra
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
                    let lp = (prize.cx + l.0 as i32, prize.cy + l.1 as i32);
                    let rp = (prize.cx + r.0 as i32, prize.cy + r.1 as i32);
                    let mut dl = len((lp.0 - g.cx) as i64, (lp.1 - g.cy) as i64);
                    let mut dr = len((rp.0 - g.cx) as i64, (rp.1 - g.cy) as i64);
                    // Reading the terrain: an approach through a nebula stays hidden,
                    // one through an asteroid field is slow.
                    if b.sides[side as usize].terrain_sense {
                        dl += approach_cost(b, (g.cx, g.cy), lp);
                        dr += approach_cost(b, (g.cx, g.cy), rp);
                    }
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
                if let Some(score) = signature_score(b, g, nearest) {
                    options.push((score, Command::Signature { group: g.id }));
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
                if front_shaken || b.pulse() >= 1 {
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
        // One order per group, plus its signature move, which does not
        // conflict with where the group goes.
        let is_sig = matches!(cmd, Command::Signature { .. });
        if (!is_sig && used.contains(&cmd.group())) || cmd.cost() > points {
            continue;
        }
        points -= cmd.cost();
        if !is_sig {
            used.push(cmd.group());
        }
        picked.push(cmd);
    }
    picked
}
