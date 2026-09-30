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
}

impl BcBattle {
    fn collect(&mut self, from: usize) {
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
