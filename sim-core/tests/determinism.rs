use simcore::*;

fn scen() -> Scenario {
    Scenario {
        lines: 500,
        belts_in: 5,
        belts_out: 3,
        belt_cap: 13,
        belt_period: 7,
        mach_dur: 11,
        mach_need: 2,
        src_period: 5,
    }
}

/// 同樣的輸入 -> 同樣的雜湊
#[test]
fn same_input_same_hash() {
    let mut a = build(scen());
    let mut b = build(scen());
    a.run(5000);
    b.run(5000);
    assert_eq!(a.state_hash(), b.state_hash());
    assert_eq!(a.delivered(), b.delivered());
    assert!(a.delivered() > 0);
}

/// 懶惰結算不影響結果：中途一直去讀狀態（強迫 sync）不能改變任何東西
#[test]
fn lazy_sync_is_transparent() {
    let mut a = build(scen());
    let mut b = build(scen());
    a.run(5000);
    for _ in 0..5000 {
        b.step();
        b.state_hash(); // 強迫每個 tick 結算全部
    }
    assert_eq!(a.state_hash(), b.state_hash());
}

/// 分段執行 == 一次執行
#[test]
fn split_run_equals_single_run() {
    let mut a = build(scen());
    let mut b = build(scen());
    a.run(3000);
    b.run(1000);
    b.run(1000);
    b.run(1000);
    assert_eq!(a.state_hash(), b.state_hash());
}

/// 堵住再放開：物品不會憑空消失或增加
#[test]
fn jam_conserves_items() {
    let mut w = build(scen());
    w.run(2000);
    let before = w.delivered() + w.items_on_belts();
    for i in 0..w.sinks.len() {
        w.set_sink_open(mkid(K_SINK, i as u32), false);
    }
    w.run(3000);
    for i in 0..w.sinks.len() {
        w.set_sink_open(mkid(K_SINK, i as u32), true);
    }
    w.run(3000);
    let after = w.delivered() + w.items_on_belts();
    assert!(after >= before, "物品減少了: {before} -> {after}");
}

/// 全廠堵死後事件數應該趨近於零（休眠）
#[test]
fn jammed_factory_is_free() {
    let mut w = build(scen());
    for i in 0..w.sinks.len() {
        w.set_sink_open(mkid(K_SINK, i as u32), false);
    }
    w.run(4000);
    let e0 = w.events;
    w.run(1000);
    let per_tick = (w.events - e0) as f64 / 1000.0;
    assert!(per_tick < 0.01, "堵死的工廠每 tick 還有 {per_tick} 個事件");
}
