//! C interface for game engines (Unreal loads the core as a DLL).
//!
//! The core owns all memory. Each step the engine reads the ship arrays and
//! the event list through the pointers returned here; the pointers stay valid
//! until the next call that mutates the battle. Panics never cross the
//! boundary: they turn into an error code. Declarations: `include/battlecore.h`.

use crate::battle::Battle;
use crate::types::*;
use std::panic::{catch_unwind, AssertUnwindSafe};

pub const BC_OK: i32 = 0;
pub const BC_ERR_NULL: i32 = -1;
pub const BC_ERR_PANIC: i32 = -2;
pub const BC_ERR_BAD_COMMAND: i32 = -3;

/// Opaque handle.
pub struct BcBattle(Battle, Pending);

/// Events not yet handed to the engine as JSON, and the last JSON text.
#[derive(Default)]
pub struct Pending {
    events: Vec<Event>,
    json: String,
    /// Set once `bc_header_json` or `bc_frame_json` is used. Engines that
    /// read `bc_events` after every step never touch the JSON views, so the
    /// events are not kept for them (they would pile up forever).
    json_reader: bool,
}

impl BcBattle {
    fn collect(&mut self, from: usize) {
        if !self.1.json_reader {
            return;
        }
        let new = self.0.events[from.min(self.0.events.len())..]
            .iter()
            .filter(|e| crate::replay::keep_event(e));
        self.1.events.extend(new);
    }
    fn step(&mut self) {
        self.0.step();
        self.collect(0);
    }
}

fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

#[no_mangle]
pub extern "C" fn bc_create_demo(seed: u64, scale: u32) -> *mut BcBattle {
    guard(std::ptr::null_mut(), || {
        Box::into_raw(Box::new(BcBattle(
            crate::scenario::demo(seed, scale),
            Pending::default(),
        )))
    })
}

/// # Safety
/// `b` must come from `bc_create_demo` and not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn bc_destroy(b: *mut BcBattle) {
    if !b.is_null() {
        drop(Box::from_raw(b));
    }
}

/// # Safety
/// `b` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn bc_set_threads(b: *mut BcBattle, threads: u32) {
    if let Some(b) = b.as_mut() {
        b.0.threads = threads.max(1) as usize;
    }
}

/// Mark a side as player-controlled (0) or commanded by the built-in AI (1).
/// # Safety
/// `b` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn bc_set_ai(b: *mut BcBattle, side: u32, ai: u32) {
    if let Some(b) = b.as_mut() {
        if side < 2 {
            b.0.sides[side as usize].ai = ai != 0;
        }
    }
}

/// Advance `n` steps. Returns 1 while the battle goes on, 0 when it is over.
/// # Safety
/// `b` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn bc_step(b: *mut BcBattle, n: u32) -> i32 {
    let Some(b) = b.as_mut() else {
        return BC_ERR_NULL;
    };
    guard(BC_ERR_PANIC, || {
        for _ in 0..n {
            b.step();
        }
        b.0.outcome.is_none() as i32
    })
}

/// Step to the next pulse boundary. Returns 1 while the battle goes on.
/// Only the last step's events are kept.
/// # Safety
/// `b` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn bc_run_to_pulse(b: *mut BcBattle) -> i32 {
    let Some(b) = b.as_mut() else {
        return BC_ERR_NULL;
    };
    guard(BC_ERR_PANIC, || loop {
        b.step();
        if b.0.outcome.is_some() {
            return 0;
        }
        if b.0.tick.is_multiple_of(PULSE_TICKS) {
            return 1;
        }
    })
}

/// Issue a command. `verb`: 0 attack, 1 flank left, 2 flank right, 3 hold,
/// 4 advance (x, y), 5 focus part (a = part bit), 6 set doctrine (a),
/// 7 commit reserve, 8 retreat, 9 signature move. Returns 0 or a negative error.
/// # Safety
/// `b` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn bc_issue(
    b: *mut BcBattle,
    side: u32,
    group: u32,
    verb: u32,
    a: i32,
    bb: i32,
) -> i32 {
    let Some(b) = b.as_mut() else {
        return BC_ERR_NULL;
    };
    let group = group as u16;
    let cmd = match verb {
        0 => Command::Attack {
            group,
            target: a as u16,
        },
        1 => Command::Flank {
            group,
            target: a as u16,
            left: true,
        },
        2 => Command::Flank {
            group,
            target: a as u16,
            left: false,
        },
        3 => Command::Hold { group },
        4 => Command::Advance { group, x: a, y: bb },
        5 => match Part::from_u8(a as u8) {
            Some(part) => Command::FocusPart { group, part },
            None => return BC_ERR_BAD_COMMAND,
        },
        6 => Command::SetDoctrine {
            group,
            doctrine: Doctrine::from_u8(a as u8),
        },
        7 => Command::CommitReserve { group },
        8 => Command::Retreat { group },
        9 => Command::Signature { group },
        10 => Command::ChangeLayer { group },
        _ => return BC_ERR_BAD_COMMAND,
    };
    guard(BC_ERR_PANIC, || {
        let before = b.0.events.len();
        match b.0.issue(side as u8, cmd) {
            Ok(()) => {
                b.collect(before);
                BC_OK
            }
            Err(e) => -10 - e as i32,
        }
    })
}

/// Read-only view of the ship arrays (structure of arrays).
#[repr(C)]
pub struct BcShipView {
    pub count: u32,
    pub x: *const i32,
    pub y: *const i32,
    pub heading: *const u16,
    pub class: *const u8,
    pub side: *const u8,
    pub group: *const u16,
    pub hull: *const i32,
    pub parts: *const u8,
    pub state: *const u8,
    pub target: *const u32,
    /// Height layer: 0 low, 1 main; bit 0x80 while climbing or diving.
    pub layer: *const u8,
    /// Fixed-point steps per distance unit.
    pub fp: i32,
}

/// # Safety
/// `b` must be a live handle; the view is valid until the next mutating call.
#[no_mangle]
pub unsafe extern "C" fn bc_ships(b: *const BcBattle) -> BcShipView {
    let s = &(*b).0.ships;
    BcShipView {
        count: s.len() as u32,
        x: s.x.as_ptr(),
        y: s.y.as_ptr(),
        heading: s.heading.as_ptr(),
        class: s.class.as_ptr(),
        side: s.side.as_ptr(),
        group: s.group.as_ptr(),
        hull: s.hull.as_ptr(),
        parts: s.parts.as_ptr(),
        state: s.state.as_ptr(),
        target: s.target.as_ptr(),
        layer: s.layer.as_ptr(),
        fp: crate::fixed::FP,
    }
}

/// Events from the last step. Writes the count to `count`.
/// # Safety
/// `b` must be a live handle and `count` a valid pointer.
#[no_mangle]
pub unsafe extern "C" fn bc_events(b: *const BcBattle, count: *mut u32) -> *const Event {
    let e = &(*b).0.events;
    if !count.is_null() {
        *count = e.len() as u32;
    }
    e.as_ptr()
}

/// # Safety
/// `b` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn bc_tick(b: *const BcBattle) -> u32 {
    (*b).0.tick
}

/// Winner: 0 or 1, -1 draw, -2 still running.
/// # Safety
/// `b` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn bc_winner(b: *const BcBattle) -> i32 {
    (*b).0.outcome.map_or(-2, |o| o.winner)
}

/// # Safety
/// `b` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn bc_command_points(b: *const BcBattle, side: u32) -> i32 {
    if side < 2 {
        (*b).0.sides[side as usize].command_points
    } else {
        0
    }
}

/// # Safety
/// `b` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn bc_state_hash(b: *const BcBattle) -> u64 {
    (*b).0.state_hash()
}

/// Fixed facts about the battle as UTF-8 JSON (ship classes, groups,
/// terrain). Length from `bc_json_len`. Valid until the next call on `b`.
/// # Safety
/// `b` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn bc_header_json(b: *mut BcBattle) -> *const u8 {
    let Some(b) = b.as_mut() else {
        return std::ptr::null();
    };
    b.1.json_reader = true;
    let mut o = String::from("{");
    crate::replay::write_header(&mut o, &b.0);
    o.push('}');
    b.1.json = o;
    b.1.json.as_ptr()
}

/// The current state as UTF-8 JSON, with the events since the last call
/// (`e`), both sides' command points (`cp`) and, once the battle is over, the
/// after-action report (`report`). Length from `bc_json_len`.
/// # Safety
/// `b` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn bc_frame_json(b: *mut BcBattle) -> *const u8 {
    use std::fmt::Write;
    let Some(b) = b.as_mut() else {
        return std::ptr::null();
    };
    b.1.json_reader = true;
    let mut o = String::new();
    crate::replay::write_frame(&mut o, &b.0);
    o.pop(); // reopen the frame object
    o.push_str(",\"e\":[");
    for (k, e) in b.1.events.drain(..).enumerate() {
        if k > 0 {
            o.push(',');
        }
        crate::replay::write_event(&mut o, &e);
    }
    let cp = &b.0.sides;
    let _ = write!(
        o,
        "],\"cp\":[{},{}]",
        cp[0].command_points, cp[1].command_points
    );
    if b.0.outcome.is_some() {
        let _ = write!(o, ",\"report\":{}", crate::replay::report_json(&b.0));
    }
    o.push('}');
    b.1.json = o;
    b.1.json.as_ptr()
}

/// Byte length of the last JSON text.
/// # Safety
/// `b` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn bc_json_len(b: *const BcBattle) -> u32 {
    (&(*b).1.json).len() as u32
}

// ----------------------------------------------------------------------
// Typed views for game engines. Each `bc_*` copy function writes up to `cap`
// records to `out` and returns how many exist, so a call with `cap` 0 (and
// `out` null) asks for the size. Positions are in distance units.
// ----------------------------------------------------------------------

fn copy_out<T>(out: *mut T, cap: u32, items: impl Iterator<Item = T>) -> u32 {
    let mut n = 0u32;
    for it in items {
        if n < cap && !out.is_null() {
            // SAFETY: the caller promises room for `cap` records.
            unsafe { out.add(n as usize).write(it) };
        }
        n += 1;
    }
    n
}

/// Per-ship state that only matters for drawing.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BcShipFx {
    /// Hull left, percent of full.
    pub hull_pct: u8,
    /// Shields left over all facings, percent of full.
    pub shield_pct: u8,
    /// Stress, percent of the overload threshold.
    pub stress_pct: u8,
    /// Bit 0: overloaded (weapons offline).
    pub flags: u8,
}

/// One record per ship, in ship order (zeros for ships no longer on the field).
/// # Safety
/// `b` must be a live handle and `out` must have room for `cap` records.
#[no_mangle]
pub unsafe extern "C" fn bc_ship_fx(b: *const BcBattle, out: *mut BcShipFx, cap: u32) -> u32 {
    let Some(b) = b.as_ref() else { return 0 };
    let s = &b.0.ships;
    copy_out(
        out,
        cap,
        (0..s.len()).map(|i| {
            if s.state[i] != crate::battle::ALIVE {
                return BcShipFx::default();
            }
            let st = s.stats(i);
            let sh_max = st.shield[0] + st.shield[1] * 2 + st.shield[2];
            let sh = s.shield[i].iter().sum::<i32>() as i64 * 100 / sh_max.max(1) as i64;
            BcShipFx {
                hull_pct: (s.hull[i] as i64 * 100 / st.hull.max(1) as i64).clamp(0, 100) as u8,
                shield_pct: sh.clamp(0, 100) as u8,
                stress_pct: (s.stress[i] as i64 * 100 / st.max_stress.max(1) as i64).clamp(0, 100)
                    as u8,
                flags: (s.overload[i] > 0) as u8,
            }
        }),
    )
}

/// A missile or torpedo in flight.
#[repr(C)]
pub struct BcMissile {
    /// Slot index; stays the same for the missile's whole flight.
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub target: u32,
    pub side: u8,
    /// 0 missile, 1 torpedo.
    pub kind: u8,
    pub pad: u16,
}

/// Missiles in flight.
/// # Safety
/// `b` must be a live handle and `out` must have room for `cap` records.
#[no_mangle]
pub unsafe extern "C" fn bc_missiles(b: *const BcBattle, out: *mut BcMissile, cap: u32) -> u32 {
    let Some(b) = b.as_ref() else { return 0 };
    let m = &b.0.missiles;
    copy_out(
        out,
        cap,
        (0..m.len()).filter(|&k| m.alive[k]).map(|k| BcMissile {
            id: k as u32,
            x: m.x[k] / crate::fixed::FP,
            y: m.y[k] / crate::fixed::FP,
            target: m.target[k],
            side: m.side[k],
            kind: m.kind[k],
            pad: 0,
        }),
    )
}

/// A fighter wing out of its hangar.
#[repr(C)]
pub struct BcWing {
    /// Wing index; stays the same for the whole battle.
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub carrier: u32,
    /// Ship being attacked, or 0xFFFFFFFF.
    pub target: u32,
    pub side: u8,
    /// Fighters left in the wing.
    pub count: u8,
    /// 1 out, 2 returning.
    pub state: u8,
    pub pad: u8,
}

/// Fighter wings in the air.
/// # Safety
/// `b` must be a live handle and `out` must have room for `cap` records.
#[no_mangle]
pub unsafe extern "C" fn bc_wings(b: *const BcBattle, out: *mut BcWing, cap: u32) -> u32 {
    let Some(b) = b.as_ref() else { return 0 };
    let w = &b.0.wings;
    copy_out(
        out,
        cap,
        (0..w.len()).filter(|&k| w.flying(k)).map(|k| BcWing {
            id: k as u32,
            x: w.x[k] / crate::fixed::FP,
            y: w.y[k] / crate::fixed::FP,
            carrier: w.carrier[k],
            target: w.target[k],
            side: w.side[k],
            count: w.count[k],
            state: w.state[k],
            pad: 0,
        }),
    )
}

/// A battle group as the player sees it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct BcGroup {
    /// Centroid of living ships.
    pub cx: i32,
    pub cy: i32,
    /// Where the group is heading.
    pub goal_x: i32,
    pub goal_y: i32,
    /// Morale, 0..1000; the group routs at 0.
    pub cohesion: i32,
    /// Signature move: negative while charging (ticks until it fires),
    /// otherwise ticks until it can be bought again (0: ready).
    pub signature: i32,
    /// Enemy group it is fighting, -1 when none.
    pub target_group: i32,
    pub alive: u32,
    pub ships: u32,
    pub fighters_out: u16,
    pub fighters_docked: u16,
    /// BC_SIDE: 0 blue, 1 red.
    pub side: u8,
    /// BC_GROUP_* status.
    pub status: u8,
    /// 0 line, 1 anvil, 2 hammer.
    pub doctrine: u8,
    /// 0 approach, 1 engage, 2 disengage, 3 refit.
    pub phase: u8,
    /// BC_LAYER_*, | BC_LAYER_MOVING while climbing or diving.
    pub layer: u8,
    /// 1 while the signature move is bought and waits for the enemy in reach.
    pub signature_armed: u8,
    /// 1 for structures, which take no orders.
    pub fixed: u8,
    /// BC_PART_* being focused, 0 when none.
    pub focus_part: u8,
}

/// Battle groups, in group order (the `group` index used by `bc_issue`).
/// # Safety
/// `b` must be a live handle and `out` must have room for `cap` records.
#[no_mangle]
pub unsafe extern "C" fn bc_groups(b: *const BcBattle, out: *mut BcGroup, cap: u32) -> u32 {
    let Some(b) = b.as_ref() else { return 0 };
    let bt = &b.0;
    let w = &bt.wings;
    let mut fighters = vec![[0u32; 2]; bt.groups.len()];
    for k in 0..w.len() {
        let gi = bt.ships.group[w.carrier[k] as usize] as usize;
        match w.state[k] {
            crate::battle::W_OUT | crate::battle::W_RETURNING => {
                fighters[gi][0] += w.count[k] as u32
            }
            crate::battle::W_DOCKED => fighters[gi][1] += w.count[k] as u32,
            _ => {}
        }
    }
    let fp = crate::fixed::FP;
    copy_out(
        out,
        cap,
        bt.groups.iter().enumerate().map(|(gi, g)| BcGroup {
            cx: g.cx / fp,
            cy: g.cy / fp,
            goal_x: g.goal_x / fp,
            goal_y: g.goal_y / fp,
            cohesion: g.cohesion,
            signature: if g.charging() {
                -((g.sig_fire_at - bt.tick) as i32)
            } else {
                g.sig_ready_at.saturating_sub(bt.tick) as i32
            },
            target_group: g.target_group.map_or(-1, |t| t as i32),
            alive: g.alive,
            ships: g.ships.len() as u32,
            fighters_out: fighters[gi][0].min(u16::MAX as u32) as u16,
            fighters_docked: fighters[gi][1].min(u16::MAX as u32) as u16,
            side: g.side,
            status: g.status.code(),
            doctrine: g.doctrine as u8,
            phase: g.phase as u8,
            layer: g.layer
                | if g.layer_until != crate::battle::NONE {
                    LAYER_MOVING
                } else {
                    0
                },
            signature_armed: g.sig_armed as u8,
            fixed: g.fixed as u8,
            focus_part: g.focus_part.map_or(0, |p| p as u8),
        }),
    )
}

/// The group's display name as UTF-8 (not NUL-terminated); length in `len`.
/// Valid while the battle lives. Null for a bad index.
/// # Safety
/// `b` must be a live handle and `len` a valid pointer or null.
#[no_mangle]
pub unsafe extern "C" fn bc_group_name(b: *const BcBattle, group: u32, len: *mut u32) -> *const u8 {
    let Some(g) = b.as_ref().and_then(|b| b.0.groups.get(group as usize)) else {
        if !len.is_null() {
            *len = 0;
        }
        return std::ptr::null();
    };
    if !len.is_null() {
        *len = g.name.len() as u32;
    }
    g.name.as_ptr()
}

/// A marked barrage that has not landed yet.
#[repr(C)]
pub struct BcStrike {
    pub x: i32,
    pub y: i32,
    /// Ticks until it lands.
    pub ticks_left: u32,
    /// Side that fired it.
    pub side: u8,
    pub pad: [u8; 3],
}

/// Marked barrages still to land.
/// # Safety
/// `b` must be a live handle and `out` must have room for `cap` records.
#[no_mangle]
pub unsafe extern "C" fn bc_strikes(b: *const BcBattle, out: *mut BcStrike, cap: u32) -> u32 {
    let Some(b) = b.as_ref() else { return 0 };
    let bt = &b.0;
    copy_out(
        out,
        cap,
        bt.strikes
            .iter()
            .filter(|st| st.land > bt.tick)
            .map(|st| BcStrike {
                x: st.x / crate::fixed::FP,
                y: st.y / crate::fixed::FP,
                ticks_left: st.land - bt.tick,
                side: st.side,
                pad: [0; 3],
            }),
    )
}

/// Terrain grid: `dim` x `dim` cells of `cell_units` distance units, row
/// major (`cells[cy * dim + cx]`). Cell (cx, cy) covers
/// x in [(cx - dim/2) * cell_units, (cx - dim/2 + 1) * cell_units), same for y.
/// Values are BC_TERRAIN_*. Cells change during the battle (debris, a pylon
/// falling); the pointer stays valid while the battle lives.
/// # Safety
/// `b` must be a live handle; `dim` and `cell_units` valid pointers or null.
#[no_mangle]
pub unsafe extern "C" fn bc_terrain(
    b: *const BcBattle,
    dim: *mut u32,
    cell_units: *mut u32,
) -> *const u8 {
    if !dim.is_null() {
        *dim = crate::terrain::DIM as u32;
    }
    if !cell_units.is_null() {
        *cell_units = crate::terrain::CELL_UNITS as u32;
    }
    match b.as_ref() {
        Some(b) => b.0.terrain.cells.as_ptr(),
        None => std::ptr::null(),
    }
}

/// After-action report as UTF-8 text (Traditional Chinese), length in `len`.
/// Valid until the next call on `b`.
/// # Safety
/// `b` must be a live handle and `len` a valid pointer or null.
#[no_mangle]
pub unsafe extern "C" fn bc_report_text(b: *mut BcBattle, len: *mut u32) -> *const u8 {
    let Some(b) = b.as_mut() else {
        return std::ptr::null();
    };
    b.1.json = crate::replay::Report::new(&b.0).text(&b.0);
    if !len.is_null() {
        *len = b.1.json.len() as u32;
    }
    b.1.json.as_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    /// The same numbers are pinned by `_Static_assert`s in examples/ffi_views.c,
    /// so the Rust records and include/battlecore.h cannot drift apart.
    #[test]
    fn record_layouts_match_the_header() {
        assert_eq!(size_of::<Event>(), 20);
        assert_eq!(size_of::<BcShipFx>(), 4);
        assert_eq!(size_of::<BcMissile>(), 20);
        assert_eq!(offset_of!(BcMissile, side), 16);
        assert_eq!(size_of::<BcWing>(), 24);
        assert_eq!(offset_of!(BcWing, side), 20);
        assert_eq!(size_of::<BcGroup>(), 48);
        assert_eq!(offset_of!(BcGroup, alive), 28);
        assert_eq!(offset_of!(BcGroup, fighters_out), 36);
        assert_eq!(offset_of!(BcGroup, side), 40);
        assert_eq!(offset_of!(BcGroup, focus_part), 47);
        assert_eq!(size_of::<BcStrike>(), 16);
    }

    #[test]
    fn typed_views_follow_the_battle() {
        unsafe {
            let b = bc_create_demo(3, 1);
            bc_set_ai(b, 0, 1);
            let ships = bc_ships(b).count;
            let groups = bc_groups(b, std::ptr::null_mut(), 0) as usize;
            assert!(groups > 0);
            let mut g = vec![std::mem::zeroed::<BcGroup>(); groups];
            let mut fx = vec![BcShipFx::default(); ships as usize];
            let mut saw_wings = false;
            let mut saw_missiles = false;
            let mut kills = 0;
            while bc_step(b, 1) == 1 && bc_tick(b) < 4 * BC_PULSE {
                assert_eq!(bc_ship_fx(b, fx.as_mut_ptr(), ships), ships);
                saw_wings |= bc_wings(b, std::ptr::null_mut(), 0) > 0;
                saw_missiles |= bc_missiles(b, std::ptr::null_mut(), 0) > 0;
                let mut n = 0;
                let ev = bc_events(b, &mut n);
                kills += std::slice::from_raw_parts(ev, n as usize)
                    .iter()
                    .filter(|e| e.kind == ev::SHIP_KILLED)
                    .count();
            }
            assert!(saw_wings && saw_missiles && kills > 0);
            // Living ships per group agree with the ship arrays.
            bc_groups(b, g.as_mut_ptr(), groups as u32);
            let v = bc_ships(b);
            for (gi, gr) in g.iter().enumerate() {
                let alive = (0..v.count as usize)
                    .filter(|&i| *v.group.add(i) as usize == gi && *v.state.add(i) == 0)
                    .count() as u32;
                assert_eq!(gr.alive, alive, "group {gi}");
                let mut len = 0;
                let name = bc_group_name(b, gi as u32, &mut len);
                assert!(std::str::from_utf8(std::slice::from_raw_parts(name, len as usize)).is_ok());
            }
            // Dead ships read as zeros.
            let dead = (0..v.count as usize).find(|&i| *v.state.add(i) != 0).unwrap();
            assert_eq!(fx[dead].hull_pct, 0);
            let (mut dim, mut cell) = (0, 0);
            let t = bc_terrain(b, &mut dim, &mut cell);
            assert!(!t.is_null() && dim > 0 && cell > 0);
            let mut len = 0;
            assert!(!bc_report_text(b, &mut len).is_null() && len > 0);
            bc_destroy(b);
        }
    }

    const BC_PULSE: u32 = PULSE_TICKS;
}
