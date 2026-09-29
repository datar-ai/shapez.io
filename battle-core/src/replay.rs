//! Records a battle as compact JSON for the 2D debug viewer, and writes the
//! after-action report (what killed us, what worked, who earned a name).

use crate::battle::{Battle, ALIVE};
use crate::fixed::FP;
use crate::terrain;
use crate::types::*;
use std::fmt::Write;

/// Events worth drawing or listing. Misses and ordinary missile launches are
/// left out to keep replays small.
pub fn keep_event(e: &Event) -> bool {
    match e.kind {
        ev::FIRE => e.c & 1 == 1,
        ev::MISSILE_LAUNCH => e.d == 1,
        ev::MISSILE_HIT | ev::OVERLOAD | ev::PART_DAMAGED => true,
        k => k != 0,
    }
}

pub fn write_event(o: &mut String, e: &Event) {
    let _ = write!(o, "[{},{},{},{},{},{}]", e.tick, e.kind, e.a, e.b, e.c, e.d);
}

/// Fixed facts about the battle: ship classes, groups, terrain.
pub fn write_header(o: &mut String, b: &Battle) {
    let _ = write!(
        o,
        "\"tickHz\":{},\"pulseTicks\":{},\"seed\":{},\"commandPoints\":{},\"ships\":[",
        TICK_HZ, PULSE_TICKS, b.seed, COMMAND_POINTS
    );
    let s = &b.ships;
    for i in 0..s.len() {
        if i > 0 {
            o.push(',');
        }
        let _ = write!(o, "[{},{},{}]", s.class[i], s.side[i], s.group[i]);
    }
    o.push_str("],\"groups\":[");
    for (gi, g) in b.groups.iter().enumerate() {
        if gi > 0 {
            o.push(',');
        }
        let _ = write!(
            o,
            "{{\"name\":\"{}\",\"side\":{},\"doctrine\":{}}}",
            g.name.replace('"', "'"),
            g.side,
            g.doctrine as u8
        );
    }
    let _ = write!(
        o,
        "],\"terrain\":{{\"cell\":{},\"dim\":{},\"cells\":\"",
        terrain::CELL_UNITS,
        terrain::DIM
    );
    for &c in &b.terrain.cells {
        o.push((b'0' + c) as char);
    }
    o.push_str("\"}");
}

/// The current state: ships, missiles, fighter wings, groups.
pub fn write_frame(o: &mut String, b: &Battle) {
    let s = &b.ships;
    let _ = write!(o, "{{\"t\":{},\"s\":[", b.tick);
    for i in 0..s.len() {
        if i > 0 {
            o.push(',');
        }
        if s.state[i] != ALIVE {
            o.push('0');
            continue;
        }
        let st = s.stats(i);
        let hp = (s.hull[i] as i64 * 100 / st.hull as i64) as i32;
        let sh_max: i32 = st.shield[0] + st.shield[1] * 2 + st.shield[2];
        let sh = s.shield[i].iter().sum::<i32>() * 100 / sh_max.max(1);
        let _ = write!(
            o,
            "[{},{},{},{},{},{},{}]",
            s.x[i] / FP,
            s.y[i] / FP,
            s.heading[i] >> 8,
            hp,
            sh,
            (s.stress[i] * 100 / st.max_stress.max(1)).min(100),
            s.parts[i] | if s.overload[i] > 0 { 16 } else { 0 }
        );
    }
    o.push_str("],\"m\":[");
    let m = &b.missiles;
    let mut first = true;
    for k in 0..m.len() {
        if !m.alive[k] {
            continue;
        }
        if !first {
            o.push(',');
        }
        first = false;
        let _ = write!(
            o,
            "[{},{},{},{}]",
            m.x[k] / FP,
            m.y[k] / FP,
            m.side[k],
            m.kind[k]
        );
    }
    o.push_str("],\"w\":[");
    let w = &b.wings;
    let mut first = true;
    for k in 0..w.len() {
        if !w.flying(k) {
            continue;
        }
        if !first {
            o.push(',');
        }
        first = false;
        let _ = write!(
            o,
            "[{},{},{},{},{},{}]",
            k,
            w.x[k] / FP,
            w.y[k] / FP,
            w.side[k],
            w.count[k],
            w.state[k]
        );
    }
    o.push_str("],\"g\":[");
    for (gi, g) in b.groups.iter().enumerate() {
        if gi > 0 {
            o.push(',');
        }
        // Signature: negative while charging (ticks left), else ticks until ready.
        let sig = if g.charging() {
            -((g.sig_fire_at - b.tick) as i64)
        } else {
            g.sig_ready_at.saturating_sub(b.tick) as i64
        };
        let _ = write!(
            o,
            "[{},{},{},{},{},{},{},{},{}]",
            g.cx / FP,
            g.cy / FP,
            g.goal_x / FP,
            g.goal_y / FP,
            g.cohesion,
            g.status.code(),
            g.target_group.map_or(-1, |t| t as i32),
            g.alive,
            sig
        );
    }
    o.push_str("]}");
}

/// Captures one frame every `every` steps plus all notable events.
pub struct Recorder {
    every: u32,
    frames: String,
    events: String,
}

impl Recorder {
    pub fn new(every: u32) -> Recorder {
        Recorder {
            every: every.max(1),
            frames: String::new(),
            events: String::new(),
        }
    }

    /// Call after every `Battle::step`.
    pub fn capture(&mut self, b: &Battle) {
        for e in b.events.iter().filter(|e| keep_event(e)) {
            if !self.events.is_empty() {
                self.events.push(',');
            }
            write_event(&mut self.events, e);
        }
        if !b.tick.is_multiple_of(self.every) && b.outcome.is_none() {
            return;
        }
        if !self.frames.is_empty() {
            self.frames.push(',');
        }
        write_frame(&mut self.frames, b);
    }

    pub fn to_json(&self, b: &Battle) -> String {
        let mut o = String::from("{");
        write_header(&mut o, b);
        let _ = write!(
            o,
            ",\"frames\":[{}],\"events\":[{}],\"report\":{}}}",
            self.frames,
            self.events,
            report_json(b)
        );
        o
    }
}

/// After-action report as JSON.
pub fn report_json(b: &Battle) -> String {
    let r = Report::new(b);
    let mut o = String::new();
    let _ = write!(
        o,
        "{{\"winner\":{},\"pulses\":{},\"seconds\":{},\"sides\":[",
        r.winner, r.pulses, r.seconds
    );
    for (k, sd) in r.sides.iter().enumerate() {
        if k > 0 {
            o.push(',');
        }
        let _ = write!(
            o,
            "{{\"ships\":{},\"lost\":{},\"escaped\":{},\"valueLost\":{},\"damageTaken\":[{},{},{}]}}",
            sd.ships, sd.lost, sd.escaped, sd.value_lost, sd.damage_taken[0], sd.damage_taken[1], sd.damage_taken[2]
        );
    }
    o.push_str("],\"groups\":[");
    for (gi, g) in b.groups.iter().enumerate() {
        if gi > 0 {
            o.push(',');
        }
        let _ = write!(
            o,
            "{{\"kills\":{},\"dealt\":{},\"taken\":{},\"cohesion\":{},\"status\":\"{}\"}}",
            g.kills,
            g.damage_dealt,
            g.damage_taken,
            g.cohesion,
            g.status.name()
        );
    }
    o.push_str("]}");
    o
}

pub struct SideReport {
    pub ships: u32,
    pub lost: u32,
    pub escaped: u32,
    pub value_lost: i32,
    pub damage_taken: [i64; 3],
}

pub struct Report {
    pub winner: i32,
    pub pulses: u32,
    pub seconds: u32,
    pub sides: [SideReport; 2],
    /// Group with the most kills on each side.
    pub mvp: [Option<u16>; 2],
}

impl Report {
    pub fn new(b: &Battle) -> Report {
        let mk = || SideReport {
            ships: 0,
            lost: 0,
            escaped: 0,
            value_lost: 0,
            damage_taken: [0; 3],
        };
        let mut sides = [mk(), mk()];
        let s = &b.ships;
        for i in 0..s.len() {
            let sd = &mut sides[s.side[i] as usize];
            sd.ships += 1;
            match s.state[i] {
                crate::battle::DEAD => {
                    sd.lost += 1;
                    sd.value_lost += s.stats(i).cost;
                }
                crate::battle::ESCAPED => sd.escaped += 1,
                _ => {}
            }
        }
        for k in 0..2 {
            sides[k].damage_taken = b.damage_by_type[k];
        }
        let mut mvp = [None, None];
        for side in 0..2u8 {
            mvp[side as usize] = b
                .groups
                .iter()
                .filter(|g| g.side == side)
                .max_by_key(|g| (g.kills, g.damage_dealt, std::cmp::Reverse(g.id)))
                .map(|g| g.id);
        }
        let end = b.outcome.map_or(b.tick, |o| o.tick);
        Report {
            winner: b.outcome.map_or(-2, |o| o.winner),
            pulses: end.div_ceil(PULSE_TICKS),
            seconds: end / TICK_HZ,
            sides,
            mvp,
        }
    }

    /// Plain-text report answering: what killed us, what worked, who stood out.
    pub fn text(&self, b: &Battle) -> String {
        let mut o = String::new();
        let who = match self.winner {
            0 => "藍方勝",
            1 => "紅方勝",
            -1 => "平手",
            _ => "未結束",
        };
        let _ = writeln!(
            o,
            "結果：{}，{} 秒（{} 個脈衝）",
            who, self.seconds, self.pulses
        );
        let dnames = ["光束", "動能", "飛彈、魚雷與戰機"];
        for (k, sd) in self.sides.iter().enumerate() {
            let name = if k == 0 { "藍方" } else { "紅方" };
            let worst = (0..3).max_by_key(|&t| sd.damage_taken[t]).unwrap();
            let _ = writeln!(
                o,
                "{}：{} 艘，被擊沉 {}，撤出 {}，損失價值 {}；受到最多的傷害來自{}",
                name, sd.ships, sd.lost, sd.escaped, sd.value_lost, dnames[worst]
            );
            if let Some(g) = self.mvp[k] {
                let g = &b.groups[g as usize];
                let _ = writeln!(o, "  立功：{}（擊沉 {} 艘）", g.name, g.kills);
            }
        }
        for g in &b.groups {
            let _ = writeln!(
                o,
                "  {:<10} {:<10} 凝聚力 {:>4}  擊沉 {:>3}  造成 {:>6}  承受 {:>6}",
                g.name,
                g.status.name(),
                g.cohesion,
                g.kills,
                g.damage_dealt,
                g.damage_taken
            );
        }
        o
    }
}
