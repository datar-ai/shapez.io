use simcore::*;
use std::time::Instant;

fn rss_mb() -> f64 {
    let s = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: f64 = s.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
    pages * 4096.0 / 1e6
}

fn cfg_for(target_nodes: u32, belt_cap: u16, belt_period: u16) -> Scenario {
    let mut c = Scenario {
        lines: 1,
        belts_in: 8,
        belts_out: 8,
        belt_cap,
        belt_period,
        mach_dur: belt_period,
        mach_need: 1,
        src_period: belt_period,
    };
    c.lines = (target_nodes / c.nodes_per_line()).max(1);
    c
}

struct Res {
    ticks: u32,
    secs: f64,
    events: u64,
    delivered: u64,
}

fn measure(w: &mut World, ticks: u32, mut per_tick: impl FnMut(&mut World, u32)) -> Res {
    let e0 = w.events;
    let d0 = w.delivered();
    let t0 = Instant::now();
    for i in 0..ticks {
        per_tick(w, i);
        w.step();
    }
    let secs = t0.elapsed().as_secs_f64();
    Res { ticks, secs, events: w.events - e0, delivered: w.delivered() - d0 }
}

fn report(name: &str, nodes: usize, r: &Res, mem: usize) {
    let tps = r.ticks as f64 / r.secs;
    let eps = r.events as f64 / r.secs;
    let ns_per_event = if r.events > 0 { r.secs * 1e9 / r.events as f64 } else { 0.0 };
    println!(
        "{name:<14} 建築={nodes:>10}  ticks/s={tps:>9.0}  (相對 60 步/秒 = {ratio:.2}x)",
        name = name, nodes = nodes, tps = tps, ratio = tps / 60.0
    );
    println!(
        "               事件={ev:>11}  事件/秒={eps:>11.0}  每事件 {ns:.1} ns  每 tick {ept:.1} 事件",
        ev = r.events, eps = eps, ns = ns_per_event, ept = r.events as f64 / r.ticks as f64
    );
    println!(
        "               送達={dl:>11}  核心記憶體={mem:.2} GB ({bpn:.1} B/建築)  RSS={rss:.2} GB",
        dl = r.delivered,
        mem = mem as f64 / 1e9,
        bpn = mem as f64 / nodes as f64,
        rss = rss_mb() / 1000.0
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.get(1).cloned().unwrap_or_else(|| "steady".into());
    let target: u32 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(1_000_000);
    let ticks: u32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(600);
    // 輸送帶 2 物品/秒、60 步/秒 -> 每 30 tick 前進一格（一格 = 一個物品間距）
    let cfg = cfg_for(target, 16, 30);
    let build_t = Instant::now();
    let mut w = build(cfg);
    w.sort_events = std::env::var("SORT").is_ok();
    let nodes = w.node_count();
    println!(
        "場景 {scenario}: {lines} 條生產線 x {npl} 個建築 = {nodes} 個建築；建構 {bs:.1}s",
        lines = cfg.lines,
        npl = cfg.nodes_per_line(),
        bs = build_t.elapsed().as_secs_f64()
    );

    let warm = (cfg.belt_cap as u32 * cfg.belt_period as u32) * (cfg.belts_in + cfg.belts_out + 2);
    let wt = Instant::now();
    w.run(warm);
    println!(
        "暖機 {warm} ticks / {ws:.1}s，帶上物品={items}",
        ws = wt.elapsed().as_secs_f64(),
        items = w.items_on_belts()
    );

    let mem = w.mem_bytes();
    match scenario.as_str() {
        "steady" => {
            let r = measure(&mut w, ticks, |_, _| {});
            report("穩態滿載", nodes, &r, mem);
        }
        "jam" => {
            for i in 0..w.sinks.len() {
                w.set_sink_open(mkid(K_SINK, i as u32), false);
            }
            w.run(cfg.belt_cap as u32 * cfg.belt_period as u32 * 4);
            let r = measure(&mut w, ticks, |_, _| {});
            report("全廠堵死", nodes, &r, mem);
        }
        "churn" => {
            let half = cfg.belt_period as u32 * 2;
            let r = measure(&mut w, ticks, move |w, i| {
                if i % half == 0 {
                    let open = (i / half) % 2 == 0;
                    for k in 0..w.sinks.len() {
                        w.set_sink_open(mkid(K_SINK, k as u32), open);
                    }
                }
            });
            report("全廠劇烈變動", nodes, &r, mem);
        }
        "hash" => {
            let h = w.state_hash();
            println!("tick={} hash={:016x} 送達={}", w.now, h, w.delivered());
        }
        _ => println!("未知場景"),
    }
}
