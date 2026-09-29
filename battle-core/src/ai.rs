//! The commander: spends a side's command points at each pulse.
//!
//! Utility scoring: every possible command gets a score, the best ones are
//! bought until the points run out. A commander that reads the terrain then
//! tries changes to that plan (moving a group into a nebula, onto the edge of
//! an asteroid field, round the target's side) by playing each one forward
//! for `LOOKAHEAD_TICKS` on a copy of the battle, and keeps the one that
//! comes out best. The enemy uses exactly the same points and
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

/// The rule-of-thumb plan: score every sensible command, buy the best.
pub fn heuristic_plan(b: &Battle, side: u8) -> Vec<Command> {
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

/// How far ahead the commander plays each candidate plan (20 seconds).
pub const LOOKAHEAD_TICKS: u32 = 20 * TICK_HZ;
/// Battles bigger than this plan by rule of thumb only.
pub const LOOKAHEAD_MAX_SHIPS: usize = 1500;

pub fn plan(b: &Battle, side: u8) -> Vec<Command> {
    let base = heuristic_plan(b, side);
    // Big battles skip it: each look ahead copies the whole battle.
    if b.lookahead || !b.sides[side as usize].terrain_sense || b.ships.len() > LOOKAHEAD_MAX_SHIPS {
        return base;
    }
    let opp = 1 - side;
    // The enemy is assumed to follow its rule-of-thumb plan this pulse.
    let their = if b.sides[opp as usize].ai {
        heuristic_plan(b, opp)
    } else {
        Vec::new()
    };
    let mut best = (play_forward(b, side, &base, &their), base.clone());
    for cand in candidates(b, side, &base) {
        let sc = play_forward(b, side, &cand, &their);
        if sc > best.0 {
            best = (sc, cand);
        }
    }
    best.1
}

/// Our strength kept minus theirs, after playing `ours` against `theirs`.
fn play_forward(b: &Battle, side: u8, ours: &[Command], theirs: &[Command]) -> i64 {
    let mut c = b.clone();
    c.lookahead = true;
    c.sides[0].ai = false;
    c.sides[1].ai = false;
    let opp = 1 - side;
    for &cmd in ours {
        let _ = c.issue(side, cmd);
    }
    for &cmd in theirs {
        let _ = c.issue(opp, cmd);
    }
    let (me0, them0) = (c.side_strength(side), c.side_strength(opp));
    let end = c.tick + LOOKAHEAD_TICKS;
    while c.tick < end && c.outcome.is_none() {
        c.step();
    }
    (them0 - c.side_strength(opp)) - (me0 - c.side_strength(side))
}

/// Variations on the base plan: one group gets a different order.
fn candidates(b: &Battle, side: u8, base: &[Command]) -> Vec<Vec<Command>> {
    let mut out = Vec::new();
    for g in b.groups.iter().filter(|g| g.side == side) {
        if g.status != GroupStatus::Active || g.alive == 0 {
            continue;
        }
        let Some(t) = g.target_group.map(|t| &b.groups[t as usize]) else {
            continue;
        };
        let mut orders = Vec::new();
        // Around the target at the group's fighting distance.
        let dr = g.range as i64 * g.doctrine.range_permille() / 1000;
        let to_us = atan2((g.cy - t.cy) as i64, (g.cx - t.cx) as i64);
        for turn in [ANG_QUARTER / 2, 0u16.wrapping_sub(ANG_QUARTER / 2)] {
            let (ox, oy) = polar(dr, to_us.wrapping_add(turn));
            orders.push(advance(g.id, t.cx + ox as i32, t.cy + oy as i32));
        }
        // Into the nearest nebula, and onto the nearest asteroid field.
        for kind in [crate::terrain::NEBULA, crate::terrain::ASTEROIDS] {
            if let Some((x, y)) = nearest_cell(b, g, kind, 3000 * FP) {
                orders.push(advance(g.id, x, y));
            }
        }
        orders.push(Command::Hold { group: g.id });
        for order in orders {
            let mut plan: Vec<Command> = base
                .iter()
                .copied()
                .filter(|c| c.group() != g.id || matches!(c, Command::Signature { .. }))
                .collect();
            plan.insert(0, order);
            // Keep within the command points, dropping the base plan's last buys.
            while plan.iter().map(|c| c.cost()).sum::<i32>() > b.sides[side as usize].command_points
            {
                plan.pop();
            }
            out.push(plan);
        }
    }
    out
}

fn advance(group: u16, x: i32, y: i32) -> Command {
    Command::Advance {
        group,
        x: x / FP,
        y: y / FP,
    }
}

/// Centre of the nearest terrain cell of `kind` within `reach` of the group.
fn nearest_cell(b: &Battle, g: &Group, kind: u8, reach: i32) -> Option<(i32, i32)> {
    use crate::terrain::{CELL_UNITS, DIM};
    let cell = CELL_UNITS * FP;
    let mut best: Option<(i64, (i32, i32))> = None;
    for cy in 0..DIM {
        for cx in 0..DIM {
            if b.terrain.cells[(cy * DIM + cx) as usize] != kind {
                continue;
            }
            let x = (cx - DIM / 2) * cell + cell / 2;
            let y = (cy - DIM / 2) * cell + cell / 2;
            let d = len((x - g.cx) as i64, (y - g.cy) as i64);
            if d <= reach as i64 && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, (x, y)));
            }
        }
    }
    best.map(|b| b.1)
}
