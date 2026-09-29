//! Measure step time on a large battle.
//!
//!   bench [--scale 75] [--steps 1200] [--threads 1,2,4]
//!
//! Scale 75 is about 5,100 ships. Reports the average and worst step and
//! checks that every thread count ends in the same state.

use battlecore::scenario;
use std::time::Instant;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scale: u32 = arg(&args, "--scale")
        .and_then(|v| v.parse().ok())
        .unwrap_or(75);
    let steps: u32 = arg(&args, "--steps")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1200);
    let threads: Vec<usize> = arg(&args, "--threads")
        .unwrap_or_else(|| "1,2,4".into())
        .split(',')
        .filter_map(|t| t.parse().ok())
        .collect();
    let mut hashes = Vec::new();
    for &t in &threads {
        let mut b = scenario::demo(7, scale);
        b.threads = t;
        let n = b.ships.len();
        let mut worst = 0f64;
        let mut peak_missiles = 0;
        let t0 = Instant::now();
        let mut done = 0;
        for _ in 0..steps {
            let s0 = Instant::now();
            b.step();
            worst = worst.max(s0.elapsed().as_secs_f64());
            peak_missiles = peak_missiles.max(b.missiles.alive.iter().filter(|a| **a).count());
            done += 1;
            if b.outcome.is_some() {
                break;
            }
        }
        let total = t0.elapsed().as_secs_f64();
        let alive = b.ships.state.iter().filter(|s| **s == 0).count();
        println!(
            "threads {t}: {n} ships, {done} steps, avg {:.3} ms/step, worst {:.3} ms, {:.0} steps/s, alive at end {alive}, peak missiles {peak_missiles}",
            total * 1000.0 / done as f64,
            worst * 1000.0,
            done as f64 / total
        );
        hashes.push(b.state_hash());
    }
    let same = hashes.windows(2).all(|w| w[0] == w[1]);
    println!("same final state for every thread count: {same}");
}
