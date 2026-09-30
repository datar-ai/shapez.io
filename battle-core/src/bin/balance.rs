//! Auto-resolve many seeds of the demo battle and report the balance.
//!
//!   balance [--seeds 100] [--first 1] [--scale 1] [--no-terrain blue|red|both]
//!
//! `--no-terrain` makes one side ignore the terrain, to measure what reading it is worth.

use battlecore::scenario;
use battlecore::types::*;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seeds: u64 = arg(&args, "--seeds")
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let scale: u32 = arg(&args, "--scale")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let mut wins = [0u32; 3];
    let mut pulses = Vec::new();
    // Kills and damage dealt by the attacker's class.
    let mut kills = [0u64; 10];
    let mut dealt = [0i64; 10];
    let mut sig_used = [0u32; 7];
    // Doctrine phase changes (to disengage), layer changes, barrages, boosts,
    // reserves jumping in at a beacon, debris cells, fields lost.
    let mut misc = [0u32; 7];
    let started = std::time::Instant::now();
    let mut group_dealt = Vec::new();
    let first: u64 = arg(&args, "--first")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    for seed in first..first + seeds {
        // `--red-layout x,y;x,y;...` tries another red formation (6 groups).
        let mut b = match arg(&args, "--red-layout") {
            Some(l) => {
                let mut at = scenario::RED_LAYOUT;
                for (k, p) in l.split(';').enumerate().take(6) {
                    let v: Vec<i32> = p.split(',').filter_map(|n| n.trim().parse().ok()).collect();
                    if v.len() == 2 {
                        at[k] = (v[0], v[1]);
                    }
                }
                scenario::demo_with(seed, scale, &at)
            }
            None => scenario::demo(seed, scale),
        };
        b.sides[0].ai = true;
        b.sides[1].ai = true;
        match arg(&args, "--no-terrain").as_deref() {
            Some("blue") => b.sides[0].terrain_sense = false,
            Some("red") => b.sides[1].terrain_sense = false,
            Some("both") => {
                b.sides[0].terrain_sense = false;
                b.sides[1].terrain_sense = false;
            }
            _ => {}
        }
        while b.outcome.is_none() {
            b.step();
            for e in &b.events {
                match e.kind {
                    ev::SHIP_KILLED if e.b != u32::MAX => {
                        kills[b.ships.class[e.b as usize] as usize] += 1
                    }
                    ev::FIRE if e.c & 1 == 1 => {
                        let st = ShipClass::from_u8(b.ships.class[e.a as usize]).stats();
                        let mult = if e.c & 2 != 0 { 3 } else { 1 };
                        dealt[b.ships.class[e.a as usize] as usize] +=
                            (st.primary.damage * mult) as i64;
                    }
                    ev::WING_STRIKE => dealt[ShipClass::Carrier as usize] += e.c as i64,
                    ev::SIGNATURE_FIRED => {
                        let g = &b.groups[e.a as usize];
                        let mut seen = [false; 7];
                        for &id in &g.ships {
                            seen[ShipClass::from_u8(b.ships.class[id as usize])
                                .stats()
                                .signature as usize] = true;
                        }
                        for (k, s) in seen.iter().enumerate() {
                            sig_used[k] += *s as u32;
                        }
                    }
                    ev::PHASE if e.c == Phase::Disengage as i32 => misc[0] += 1,
                    ev::LAYER_CHANGED if e.d == 1 => misc[1] += 1,
                    ev::BARRAGE_MARKED => misc[2] += 1,
                    ev::SHIELD_BOOST => misc[3] += 1,
                    ev::JUMP_IN => misc[4] += 1,
                    ev::DEBRIS => misc[5] += 1,
                    ev::FIELD_LOST => misc[6] += 1,
                    _ => {}
                }
            }
        }
        let o = b.outcome.unwrap();
        if o.winner < 0 && arg(&args, "--show-draws").is_some() {
            println!("draw seed {seed}:");
            for g in b
                .groups
                .iter()
                .filter(|g| g.status.in_battle() && g.alive > 0)
            {
                println!(
                    "  {} {:?} {:?} layer {} alive {} cohesion {} at ({}, {})",
                    g.name,
                    g.status,
                    g.phase,
                    g.layer,
                    g.alive,
                    g.cohesion,
                    g.cx / 100,
                    g.cy / 100
                );
            }
        }
        wins[match o.winner {
            0 => 0,
            1 => 1,
            _ => 2,
        }] += 1;
        pulses.push(o.tick.div_ceil(PULSE_TICKS));
        if group_dealt.is_empty() {
            group_dealt = vec![0i64; b.groups.len()];
        }
        for (i, g) in b.groups.iter().enumerate() {
            group_dealt[i] += g.damage_dealt;
        }
        if seed == 1 {
            println!(
                "groups: {}",
                b.groups
                    .iter()
                    .map(|g| g.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    pulses.sort();
    let avg = pulses.iter().sum::<u32>() as f64 / pulses.len() as f64;
    println!(
        "{seeds} battles at scale {scale}: blue {} / red {} / draw {}",
        wins[0], wins[1], wins[2]
    );
    println!(
        "pulses: min {} median {} max {} avg {avg:.1}",
        pulses[0],
        pulses[pulses.len() / 2],
        pulses[pulses.len() - 1]
    );
    let names = [
        "battleship",
        "cruiser",
        "destroyer",
        "flagship",
        "carrier",
        "artillery",
        "support",
        "station",
        "beacon",
        "pylon",
    ];
    for c in 0..8 {
        println!(
            "  {:<10} kills {:>6.1}/battle  gun+wing damage {:>8.0}/battle",
            names[c],
            kills[c] as f64 / seeds as f64,
            dealt[c] as f64 / seeds as f64
        );
    }
    println!(
        "signatures per battle: salvo {:.1}, torpedoes {:.1}, afterburn {:.1}, scramble {:.1}",
        sig_used[0] as f64 / seeds as f64,
        sig_used[1] as f64 / seeds as f64,
        sig_used[2] as f64 / seeds as f64,
        sig_used[3] as f64 / seeds as f64
    );
    let per = |v: u32| v as f64 / seeds as f64;
    println!(
        "per battle: barrage {:.1}, shield boost {:.1}; disengages {:.1}, layer changes {:.1}, \
         jump-ins {:.1}, debris cells {:.1}, fields lost {:.1}",
        per(sig_used[4]),
        per(sig_used[5]),
        per(misc[0]),
        per(misc[1]),
        per(misc[4]),
        per(misc[5]),
        per(misc[6])
    );
    println!(
        "time per battle: {:.2} s",
        started.elapsed().as_secs_f64() / seeds as f64
    );
    println!(
        "damage dealt per group: {}",
        group_dealt
            .iter()
            .map(|d| format!("{:.0}", *d as f64 / seeds as f64))
            .collect::<Vec<_>>()
            .join(" ")
    );
}
