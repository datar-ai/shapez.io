//! Run the demo battle with the built-in commander on both sides.
//!
//!   battle [--seed N] [--scale K] [--threads T] [--replay out.json]

use battlecore::battle::GroupStatus;
use battlecore::replay::{Recorder, Report};
use battlecore::scenario;
use battlecore::types::*;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seed: u64 = arg(&args, "--seed")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let scale: u32 = arg(&args, "--scale")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let threads: usize = arg(&args, "--threads")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let replay = arg(&args, "--replay");

    let mut b = scenario::demo(seed, scale);
    b.threads = threads;
    let mut rec = replay.as_ref().map(|_| Recorder::new(4));
    println!(
        "種子 {seed}，艦船 {} 艘，戰鬥群 {} 個",
        b.ships.len(),
        b.groups.len()
    );
    let t0 = std::time::Instant::now();
    let mut steps = 0u64;
    while b.outcome.is_none() {
        b.step();
        steps += 1;
        if let Some(r) = rec.as_mut() {
            r.capture(&b);
        }
        for e in &b.events {
            if e.kind == ev::COMMAND {
                let g = &b.groups[e.a as usize];
                let verb = [
                    "",
                    "攻擊",
                    "側擊",
                    "固守",
                    "推進",
                    "集火部位",
                    "換準則",
                    "投入預備隊",
                    "跳躍撤離",
                    "招牌技",
                    "換層",
                ][e.c as usize];
                println!("  [脈衝 {}] {} → {}", b.pulse(), g.name, verb);
            }
            if e.kind == ev::GROUP_ROUTED {
                println!(
                    "  {:>5.1}s  {} 潰散",
                    e.tick as f64 / TICK_HZ as f64,
                    b.groups[e.a as usize].name
                );
            }
            if e.kind == ev::PHASE && e.c == Phase::Disengage as i32 {
                println!(
                    "  {:>5.1}s  {} 脫離整補",
                    e.tick as f64 / TICK_HZ as f64,
                    b.groups[e.a as usize].name
                );
            }
            if e.kind == ev::JUMP_IN {
                println!(
                    "  {:>5.1}s  {} 從跳躍信標躍入",
                    e.tick as f64 / TICK_HZ as f64,
                    b.groups[e.a as usize].name
                );
            }
            if e.kind == ev::FLAGSHIP_LOST {
                println!(
                    "  {:>5.1}s  {}方旗艦被擊沉",
                    e.tick as f64 / TICK_HZ as f64,
                    if e.b == 0 { "藍" } else { "紅" }
                );
            }
        }
        if b.tick.is_multiple_of(PULSE_TICKS) && b.outcome.is_none() {
            let line: Vec<String> = b
                .groups
                .iter()
                .filter(|g| g.status != GroupStatus::Destroyed)
                .map(|g| {
                    format!(
                        "{}{}/{}",
                        g.name.split('・').nth(1).unwrap_or(&g.name),
                        g.alive,
                        g.cohesion
                    )
                })
                .collect();
            println!("脈衝 {:>2}：{}", b.pulse(), line.join("  "));
        }
    }
    let dt = t0.elapsed();
    println!();
    print!("{}", Report::new(&b).text(&b));
    println!(
        "模擬 {} 步，耗時 {:.1} ms（每步 {:.3} ms），狀態雜湊 {:016x}",
        steps,
        dt.as_secs_f64() * 1000.0,
        dt.as_secs_f64() * 1000.0 / steps as f64,
        b.state_hash()
    );
    if let (Some(path), Some(r)) = (replay, rec) {
        std::fs::write(&path, r.to_json(&b)).expect("write replay");
        println!("重播已寫入 {path}");
    }
}
