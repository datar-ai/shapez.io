//! The battle: ships, battle groups, missiles, and the fixed-step update.
//!
//! One step (1/20 s) runs in phases:
//! 1. groups re-plan (every 10 steps): where to stand, which way to face;
//! 2. ships think in parallel, reading only last step's state: steer toward
//!    their formation slot, pick targets, fire (hitscan hits are rolled now);
//! 3. intents are written back in ship order;
//! 4. missiles fly and point defense shoots at them and at fighters;
//! 5. fighter wings fly, dogfight and strafe (in wing order);
//! 6. damage is applied in a fixed order; shields, stress, cohesion update;
//! 7. routs, retreats and the end of the battle are checked.
//!
//! Signature moves bought at a pulse fire at the start of the step their
//! charge runs out.
//!
//! Every random roll is a hash of (seed, tick, ship, salt), so the thread
//! count never changes the result.

use crate::fixed::*;
use crate::grid::Grid;
use crate::terrain::{self, Terrain};
use crate::types::*;

pub const ALIVE: u8 = 0;
pub const DEAD: u8 = 1;
pub const ESCAPED: u8 = 2;

/// Ship data, one array per field (structure of arrays).
#[derive(Default, Clone)]
pub struct Ships {
    pub x: Vec<i32>,
    pub y: Vec<i32>,
    pub vx: Vec<i32>,
    pub vy: Vec<i32>,
    pub heading: Vec<u16>,
    pub class: Vec<u8>,
    pub side: Vec<u8>,
    pub group: Vec<u16>,
    pub slot_dx: Vec<i32>,
    pub slot_dy: Vec<i32>,
    pub hull: Vec<i32>,
    /// Shield per quadrant: front, left, rear, right.
    pub shield: Vec<[i32; 4]>,
    pub stress: Vec<i32>,
    pub overload: Vec<u16>,
    pub shield_delay: Vec<u16>,
    pub parts: Vec<u8>,
    pub target: Vec<u32>,
    pub cd_primary: Vec<u16>,
    pub cd_missile: Vec<u16>,
    pub cd_pd: Vec<u16>,
    pub state: Vec<u8>,
    pub kills: Vec<u16>,
    /// Main guns are loaded for a signature salvo until this tick.
    pub salvo_until: Vec<u32>,
}

impl Ships {
    pub fn len(&self) -> usize {
        self.x.len()
    }
    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }
    #[inline]
    pub fn stats(&self, i: usize) -> &'static ClassStats {
        ShipClass::from_u8(self.class[i]).stats()
    }
    #[inline]
    pub fn alive(&self, i: usize) -> bool {
        self.state[i] == ALIVE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupStatus {
    Active,
    Reserve,
    /// Charging jump drives; leaves the field at `until`.
    Retreating {
        until: u32,
    },
    Routed,
    Escaped,
    Destroyed,
}

impl GroupStatus {
    /// Still counts as a fighting presence on the field.
    pub fn in_battle(self) -> bool {
        matches!(
            self,
            GroupStatus::Active | GroupStatus::Reserve | GroupStatus::Retreating { .. }
        )
    }
    pub fn code(self) -> u8 {
        match self {
            GroupStatus::Active => 0,
            GroupStatus::Reserve => 1,
            GroupStatus::Retreating { .. } => 2,
            GroupStatus::Routed => 3,
            GroupStatus::Escaped => 4,
            GroupStatus::Destroyed => 5,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            GroupStatus::Active => "active",
            GroupStatus::Reserve => "reserve",
            GroupStatus::Retreating { .. } => "retreating",
            GroupStatus::Routed => "routed",
            GroupStatus::Escaped => "escaped",
            GroupStatus::Destroyed => "destroyed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Group {
    pub id: u16,
    pub side: u8,
    pub name: String,
    pub ships: Vec<u32>,
    pub doctrine: Doctrine,
    pub order: Order,
    pub status: GroupStatus,
    pub cohesion: i32,
    /// Moving formation center that the ships follow.
    pub anchor_x: i32,
    pub anchor_y: i32,
    /// Where the anchor is heading.
    pub goal_x: i32,
    pub goal_y: i32,
    pub facing: u16,
    pub want_facing: u16,
    pub target_group: Option<u16>,
    pub focus_part: Option<Part>,
    /// Centroid of living ships, refreshed when the group re-plans.
    pub cx: i32,
    pub cy: i32,
    pub alive: u32,
    pub init_cost: i32,
    pub init_hull: i64,
    pub speed: i32,
    pub range: i32,
    pub turn: u16,
    pub kills: u32,
    pub damage_dealt: i64,
    pub damage_taken: i64,
    /// Signature move: usable from this tick on.
    pub sig_ready_at: u32,
    /// Signature move bought and charging; fires at this tick. `NONE` when idle.
    pub sig_fire_at: u32,
    /// Afterburn runs until this tick.
    pub boost_until: u32,
    /// Value of living ships and how many of them carry heavy point defense
    /// (destroyers, carriers); refreshed when groups re-plan. Wings use it to
    /// pick soft targets.
    pub alive_value: i32,
    pub screen: u32,
}

pub const NONE: u32 = u32::MAX;

impl Group {
    pub fn charging(&self) -> bool {
        self.sig_fire_at != NONE
    }
}

#[derive(Default, Clone)]
pub struct Missiles {
    pub x: Vec<i32>,
    pub y: Vec<i32>,
    pub target: Vec<u32>,
    pub src: Vec<u32>,
    pub side: Vec<u8>,
    pub damage: Vec<i32>,
    pub speed: Vec<i32>,
    pub life: Vec<u16>,
    pub alive: Vec<bool>,
    /// 0 missile, 1 torpedo.
    pub kind: Vec<u8>,
    /// Point-defense hits still needed to stop it.
    pub hp: Vec<u8>,
}

pub const W_DOCKED: u8 = 0;
pub const W_OUT: u8 = 1;
pub const W_RETURNING: u8 = 2;
pub const W_LOST: u8 = 3;

/// Fighter wings: squadrons of up to `WING_SIZE` fighters that live on their
/// carrier, fly out to fight and come back to rearm.
#[derive(Default, Clone)]
pub struct Wings {
    pub x: Vec<i32>,
    pub y: Vec<i32>,
    pub side: Vec<u8>,
    pub carrier: Vec<u32>,
    pub count: Vec<u8>,
    pub state: Vec<u8>,
    /// Ship being attacked, or `NONE`.
    pub target: Vec<u32>,
    /// Enemy wing being fought, or `NONE`.
    pub target_wing: Vec<u32>,
    pub cd: Vec<u16>,
    pub rearm: Vec<u16>,
}

impl Wings {
    pub fn len(&self) -> usize {
        self.x.len()
    }
    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }
    #[inline]
    pub fn flying(&self, w: usize) -> bool {
        matches!(self.state[w], W_OUT | W_RETURNING) && self.count[w] > 0
    }
}

impl Missiles {
    pub fn len(&self) -> usize {
        self.x.len()
    }
    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SideState {
    pub command_points: i32,
    /// When true, the built-in commander spends this side's points each pulse.
    pub ai: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// 0 or 1, or -1 for a draw.
    pub winner: i32,
    pub tick: u32,
}

#[derive(Clone, Copy, Debug)]
struct Damage {
    target: u32,
    src: u32,
    amount: i32,
    dtype: DamageType,
    from_x: i32,
    from_y: i32,
}

#[derive(Clone, Copy, Default)]
struct Intent {
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
    heading: u16,
    target: u32,
    cd_primary: u16,
    cd_missile: u16,
    cd_pd: u16,
    stress: i32,
    salvo_until: u32,
}

/// A point-defense shot at a missile or a fighter wing.
#[derive(Clone, Copy)]
struct Intercept {
    ship: u32,
    id: u32,
    hit: bool,
    wing: bool,
}

#[derive(Default)]
struct ChunkOut {
    intents: Vec<Intent>,
    damages: Vec<Damage>,
    /// (shooter, target, kind): kind 0 missile, 1 torpedo.
    launches: Vec<(u32, u32, u8)>,
    intercepts: Vec<Intercept>,
    events: Vec<Event>,
}

pub struct Battle {
    pub tick: u32,
    pub seed: u64,
    /// Worker threads for the ship phase. Any value gives the same result.
    pub threads: usize,
    pub ships: Ships,
    pub groups: Vec<Group>,
    pub missiles: Missiles,
    pub wings: Wings,
    pub terrain: Terrain,
    pub sides: [SideState; 2],
    /// Events produced by the most recent step (and commands issued since).
    pub events: Vec<Event>,
    pub outcome: Option<Outcome>,
    /// Damage taken per side and damage type, for the after-action report.
    pub damage_by_type: [[i64; 3]; 2],
    /// Fleet value lost per side; past half, the whole fleet wavers.
    pub value_lost: [i32; 2],
    started: bool,
    grid: Grid,
    missile_grid: Grid,
    wing_grid: Grid,
}

impl Battle {
    pub fn new(seed: u64) -> Battle {
        Battle {
            tick: 0,
            seed,
            threads: 1,
            ships: Ships::default(),
            groups: Vec::new(),
            missiles: Missiles::default(),
            wings: Wings::default(),
            terrain: Terrain::default(),
            sides: [SideState {
                command_points: 0,
                ai: true,
            }; 2],
            events: Vec::new(),
            outcome: None,
            damage_by_type: [[0; 3]; 2],
            value_lost: [0; 2],
            started: false,
            grid: Grid::default(),
            missile_grid: Grid::default(),
            wing_grid: Grid::default(),
        }
    }

    pub fn pulse(&self) -> u32 {
        self.tick / PULSE_TICKS
    }

    /// Add a battle group in a block formation centered at (x, y) facing `facing`.
    /// Positions are in distance units.
    #[allow(clippy::too_many_arguments)]
    pub fn add_group(
        &mut self,
        side: u8,
        name: &str,
        doctrine: Doctrine,
        reserve: bool,
        ships: &[(ShipClass, u32)],
        x: i32,
        y: i32,
        facing: u16,
    ) -> u16 {
        let gid = self.groups.len() as u16;
        let total: u32 = ships.iter().map(|s| s.1).sum();
        let cols = ((total as f64 * 2.0).sqrt().ceil() as i64).max(1);
        let max_radius = ships.iter().map(|s| s.0.stats().radius).max().unwrap_or(FP) as i64;
        let spacing = max_radius * 3 + 20 * FP as i64;
        let (x, y) = (x * FP, y * FP);
        let mut ids = Vec::new();
        let (mut cost, mut hull) = (0, 0i64);
        let (mut speed, mut range, mut turn) = (i32::MAX, i32::MAX, u16::MAX);
        let mut has_carrier = false;
        let mut k: i64 = 0;
        for &(class, count) in ships {
            let st = class.stats();
            for _ in 0..count {
                let (row, col) = (k / cols, k % cols);
                let row_n = cols.min(total as i64 - row * cols);
                let dx = -row * spacing;
                let dy = (col * 2 - (row_n - 1)) * spacing / 2;
                let (wx, wy) = rotate(dx, dy, facing);
                let id = self.ships.len() as u32;
                let s = &mut self.ships;
                s.x.push(x + wx as i32);
                s.y.push(y + wy as i32);
                s.vx.push(0);
                s.vy.push(0);
                s.heading.push(facing);
                s.class.push(class as u8);
                s.side.push(side);
                s.group.push(gid);
                s.slot_dx.push(dx as i32);
                s.slot_dy.push(dy as i32);
                s.hull.push(st.hull);
                s.shield
                    .push([st.shield[0], st.shield[1], st.shield[2], st.shield[1]]);
                s.stress.push(0);
                s.overload.push(0);
                s.shield_delay.push(0);
                s.parts.push(0);
                s.target.push(u32::MAX);
                s.cd_primary.push((id % 20) as u16);
                s.cd_missile.push((id % 40) as u16);
                s.cd_pd.push(0);
                s.state.push(ALIVE);
                s.kills.push(0);
                s.salvo_until.push(0);
                for _ in 0..st.hangar {
                    let w = &mut self.wings;
                    w.x.push(x + wx as i32);
                    w.y.push(y + wy as i32);
                    w.side.push(side);
                    w.carrier.push(id);
                    w.count.push(WING_SIZE);
                    w.state.push(W_DOCKED);
                    w.target.push(NONE);
                    w.target_wing.push(NONE);
                    w.cd.push(0);
                    w.rearm.push(REARM_TICKS);
                }
                ids.push(id);
                cost += st.cost;
                hull += st.hull as i64;
                speed = speed.min(st.max_speed);
                range = range.min(st.primary.range);
                has_carrier |= st.hangar > 0;
                turn = turn.min(st.turn_rate);
                k += 1;
            }
        }
        if has_carrier {
            // Carriers stand off and let their wings do the fighting.
            range = CARRIER_STANDOFF;
        }
        self.groups.push(Group {
            id: gid,
            side,
            name: name.to_string(),
            alive: ids.len() as u32,
            ships: ids,
            doctrine,
            order: Order::Engage,
            status: if reserve {
                GroupStatus::Reserve
            } else {
                GroupStatus::Active
            },
            cohesion: COHESION_MAX,
            anchor_x: x,
            anchor_y: y,
            goal_x: x,
            goal_y: y,
            facing,
            want_facing: facing,
            target_group: None,
            focus_part: None,
            cx: x,
            cy: y,
            init_cost: cost.max(1),
            init_hull: hull.max(1),
            speed,
            range,
            turn,
            kills: 0,
            damage_dealt: 0,
            damage_taken: 0,
            sig_ready_at: 0,
            sig_fire_at: NONE,
            boost_until: 0,
            alive_value: cost,
            screen: 0,
        });
        gid
    }

    // ------------------------------------------------------------------
    // Commands
    // ------------------------------------------------------------------

    pub fn issue(&mut self, side: u8, cmd: Command) -> Result<(), CommandError> {
        if self.outcome.is_some() {
            return Err(CommandError::BattleOver);
        }
        let gi = cmd.group() as usize;
        let g = self.groups.get(gi).ok_or(CommandError::NoSuchGroup)?;
        if g.side != side {
            return Err(CommandError::NotYourGroup);
        }
        match g.status {
            GroupStatus::Retreating { .. } => return Err(CommandError::GroupRetreating),
            GroupStatus::Routed | GroupStatus::Escaped | GroupStatus::Destroyed => {
                return Err(CommandError::GroupOutOfAction)
            }
            _ => {}
        }
        let is_reserve = g.status == GroupStatus::Reserve;
        match cmd {
            Command::CommitReserve { .. } if !is_reserve => return Err(CommandError::NotInReserve),
            Command::Signature { .. }
                if is_reserve || g.charging() || self.tick < g.sig_ready_at =>
            {
                return Err(CommandError::SignatureNotReady)
            }
            Command::Attack { target, .. } | Command::Flank { target, .. } => {
                let t = self
                    .groups
                    .get(target as usize)
                    .ok_or(CommandError::BadTarget)?;
                if t.side == side || !t.status.in_battle() {
                    return Err(CommandError::BadTarget);
                }
            }
            _ => {}
        }
        if self.sides[side as usize].command_points < cmd.cost() {
            return Err(CommandError::NotEnoughPoints);
        }
        self.sides[side as usize].command_points -= cmd.cost();
        let tick = self.tick;
        let g = &mut self.groups[gi];
        let kind: i32 = match cmd {
            Command::Attack { target, .. } => {
                g.order = Order::Attack { target };
                1
            }
            Command::Flank { target, left, .. } => {
                g.order = Order::Flank { target, left };
                2
            }
            Command::Hold { .. } => {
                g.order = Order::Hold { x: g.cx, y: g.cy };
                3
            }
            Command::Advance { x, y, .. } => {
                g.order = Order::Advance {
                    x: x * FP,
                    y: y * FP,
                };
                4
            }
            Command::FocusPart { part, .. } => {
                g.focus_part = Some(part);
                5
            }
            Command::SetDoctrine { doctrine, .. } => {
                g.doctrine = doctrine;
                6
            }
            Command::CommitReserve { .. } => {
                g.status = GroupStatus::Active;
                g.order = Order::Engage;
                7
            }
            Command::Retreat { .. } => {
                g.status = GroupStatus::Retreating {
                    until: tick + PULSE_TICKS,
                };
                g.goal_x = g.cx;
                g.goal_y = g.cy;
                self.events
                    .push(Event::new(tick, ev::RETREAT_STARTED, gi as u32, 0, 0));
                8
            }
            Command::Signature { .. } => {
                g.sig_fire_at = tick + SIG_CHARGE_TICKS;
                self.events.push(Event::new(
                    tick,
                    ev::SIGNATURE_CHARGING,
                    gi as u32,
                    0,
                    SIG_CHARGE_TICKS as i32,
                ));
                9
            }
        };
        self.events
            .push(Event::new(tick, ev::COMMAND, gi as u32, side as u32, kind));
        Ok(())
    }

    fn begin_pulse(&mut self) {
        for s in 0..2 {
            self.sides[s].command_points = COMMAND_POINTS;
        }
        self.events
            .push(Event::new(self.tick, ev::PULSE, 0, 0, self.pulse() as i32));
        for s in 0..2u8 {
            if self.sides[s as usize].ai {
                for cmd in crate::ai::plan(self, s) {
                    let _ = self.issue(s, cmd);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Stepping
    // ------------------------------------------------------------------

    /// Advance one step (1/20 s). Events from this step land in `self.events`.
    pub fn step(&mut self) {
        if !self.started {
            // Deployment pulse: commanders get their first points before anything moves.
            self.started = true;
            self.begin_pulse();
            return;
        }
        self.events.clear();
        if self.outcome.is_some() {
            return;
        }
        let s = &self.ships;
        self.grid.rebuild(&s.x, &s.y, |i| s.state[i] == ALIVE);
        let ms = &self.missiles;
        self.missile_grid.rebuild(&ms.x, &ms.y, |k| ms.alive[k]);
        let ws = &self.wings;
        self.wing_grid.rebuild(&ws.x, &ws.y, |w| ws.flying(w));
        let mut launches = Vec::new();
        self.fire_signatures(&mut launches);
        if self.tick.is_multiple_of(GROUP_THINK_TICKS) {
            self.plan_groups();
        }
        self.move_anchors();

        let out = self.ship_phase();
        let mut damages = Vec::new();
        let mut intercepts = Vec::new();
        {
            let s = &mut self.ships;
            let mut i = 0usize;
            for chunk in out {
                for it in chunk.intents {
                    s.x[i] = it.x;
                    s.y[i] = it.y;
                    s.vx[i] = it.vx;
                    s.vy[i] = it.vy;
                    s.heading[i] = it.heading;
                    s.target[i] = it.target;
                    s.cd_primary[i] = it.cd_primary;
                    s.cd_missile[i] = it.cd_missile;
                    s.cd_pd[i] = it.cd_pd;
                    s.stress[i] = it.stress;
                    s.salvo_until[i] = it.salvo_until;
                    i += 1;
                }
                damages.extend(chunk.damages);
                launches.extend(chunk.launches);
                intercepts.extend(chunk.intercepts);
                self.events.extend(chunk.events);
            }
        }
        self.missile_phase(&intercepts, &mut damages);
        self.wing_phase(&mut damages);
        self.launch_missiles(&launches);
        for d in &damages {
            self.apply_damage(d);
        }
        self.upkeep();
        self.check_groups();
        self.check_outcome();
        self.tick += 1;
        if self.outcome.is_none() && self.tick.is_multiple_of(PULSE_TICKS) {
            self.begin_pulse();
        }
    }

    /// Step until the next pulse boundary (or the end). Returns true while the
    /// battle is still going.
    pub fn run_to_pulse(&mut self) -> bool {
        if !self.started {
            self.step();
        }
        loop {
            self.step();
            if self.outcome.is_some() {
                return false;
            }
            if self.tick.is_multiple_of(PULSE_TICKS) {
                return true;
            }
        }
    }

    /// Run to the end with the built-in commander on both sides (auto-resolve).
    pub fn run_auto(&mut self) -> Outcome {
        self.sides[0].ai = true;
        self.sides[1].ai = true;
        while self.outcome.is_none() {
            self.step();
        }
        self.outcome.unwrap()
    }

    fn nearest_enemy_group(&self, g: &Group) -> Option<u16> {
        let mut best: Option<(i64, u16)> = None;
        for o in &self.groups {
            if o.side == g.side || !o.status.in_battle() || o.alive == 0 {
                continue;
            }
            let d = len((o.cx - g.cx) as i64, (o.cy - g.cy) as i64);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, o.id));
            }
        }
        best.map(|b| b.1)
    }

    fn plan_groups(&mut self) {
        // Refresh centroids and derived stats first so every group plans on the same picture.
        for gi in 0..self.groups.len() {
            let (mut sx, mut sy, mut n) = (0i64, 0i64, 0i64);
            let (mut speed, mut turn) = (i32::MAX, u16::MAX);
            let (mut value, mut screen) = (0, 0);
            for &id in &self.groups[gi].ships {
                let i = id as usize;
                if self.ships.state[i] != ALIVE {
                    continue;
                }
                sx += self.ships.x[i] as i64;
                sy += self.ships.y[i] as i64;
                n += 1;
                let st = self.ships.stats(i);
                value += st.cost;
                if st.pd.chance >= 400 {
                    screen += 1;
                }
                let engine_hit = self.ships.parts[i] & Part::Engine as u8 != 0;
                speed = speed.min(if engine_hit {
                    st.max_speed / 2
                } else {
                    st.max_speed
                });
                turn = turn.min(st.turn_rate);
            }
            let g = &mut self.groups[gi];
            g.alive_value = value;
            g.screen = screen;
            if n > 0 {
                g.cx = (sx / n) as i32;
                g.cy = (sy / n) as i32;
                g.speed = speed;
                g.turn = turn;
            }
        }
        for gi in 0..self.groups.len() {
            let g = self.groups[gi].clone();
            let (mut goal_x, mut goal_y, mut want) = (g.goal_x, g.goal_y, g.want_facing);
            let mut target_group = g.target_group;
            let mut order = g.order;
            match g.status {
                GroupStatus::Destroyed | GroupStatus::Escaped => continue,
                GroupStatus::Retreating { .. } => {}
                GroupStatus::Routed => {
                    let edge = if g.side == 0 { -FIELD_HALF } else { FIELD_HALF };
                    goal_x = edge;
                    goal_y = g.cy;
                    want = if g.side == 0 { ANG_HALF } else { 0 };
                }
                GroupStatus::Reserve => {
                    if let Some(t) = self.nearest_enemy_group(&g) {
                        let t = &self.groups[t as usize];
                        want = atan2((t.cy - g.cy) as i64, (t.cx - g.cx) as i64);
                    }
                }
                GroupStatus::Active => {
                    let explicit = match order {
                        Order::Attack { target } | Order::Flank { target, .. } => {
                            let t = &self.groups[target as usize];
                            if t.status.in_battle() && t.alive > 0 {
                                Some(target)
                            } else {
                                order = Order::Engage;
                                None
                            }
                        }
                        _ => None,
                    };
                    target_group = explicit.or_else(|| self.nearest_enemy_group(&g));
                    let dr = g.range as i64 * g.doctrine.range_permille() / 1000;
                    if let Some(tid) = target_group {
                        let t = &self.groups[tid as usize];
                        let to_us = atan2((g.cy - t.cy) as i64, (g.cx - t.cx) as i64);
                        match order {
                            Order::Engage | Order::Attack { .. } => {
                                let (ox, oy) = polar(dr, to_us);
                                goal_x = t.cx + ox as i32;
                                goal_y = t.cy + oy as i32;
                                want = to_us.wrapping_add(ANG_HALF);
                            }
                            Order::Flank { left, .. } => {
                                let side_dir = if left {
                                    t.facing.wrapping_add(ANG_QUARTER)
                                } else {
                                    t.facing.wrapping_sub(ANG_QUARTER)
                                };
                                // Swing around the target at a safe distance instead of
                                // cutting straight through its guns: aim 60 degrees
                                // further around the circle until the flank is reached.
                                let turn_left = angle_diff(side_dir, to_us);
                                let (r, dir) = if turn_left.abs() > 10923 {
                                    let cur = len((g.cx - t.cx) as i64, (g.cy - t.cy) as i64);
                                    let step = if turn_left > 0 {
                                        10923u16
                                    } else {
                                        0u16.wrapping_sub(10923)
                                    };
                                    (cur.max(dr * 13 / 10), to_us.wrapping_add(step))
                                } else {
                                    (dr, side_dir)
                                };
                                let (ox, oy) = polar(r, dir);
                                goal_x = t.cx + ox as i32;
                                goal_y = t.cy + oy as i32;
                                want = to_us.wrapping_add(ANG_HALF);
                            }
                            Order::Hold { x, y } => {
                                goal_x = x;
                                goal_y = y;
                                want = to_us.wrapping_add(ANG_HALF);
                            }
                            Order::Advance { x, y } => {
                                goal_x = x;
                                goal_y = y;
                                want = atan2((y - g.cy) as i64, (x - g.cx) as i64);
                            }
                        }
                    } else {
                        goal_x = g.anchor_x;
                        goal_y = g.anchor_y;
                    }
                }
            }
            let g = &mut self.groups[gi];
            g.goal_x = goal_x;
            g.goal_y = goal_y;
            g.want_facing = want;
            g.target_group = target_group;
            g.order = order;
        }
    }

    fn move_anchors(&mut self) {
        let tick = self.tick;
        for g in &mut self.groups {
            if !matches!(g.status, GroupStatus::Active | GroupStatus::Routed) {
                continue;
            }
            let (dx, dy) = (
                (g.goal_x - g.anchor_x) as i64,
                (g.goal_y - g.anchor_y) as i64,
            );
            let d = len(dx, dy);
            // Keep the anchor on a leash so it never runs away from slow ships.
            let lag = len((g.anchor_x - g.cx) as i64, (g.anchor_y - g.cy) as i64);
            let mut v = g.speed as i64;
            if tick < g.boost_until {
                v *= 2;
            }
            if lag > 600 * FP as i64 {
                v /= 3;
            }
            if d > 0 {
                let step = v.min(d);
                g.anchor_x += (dx * step / d) as i32;
                g.anchor_y += (dy * step / d) as i32;
            }
            g.facing = turn_toward(g.facing, g.want_facing, g.turn.max(1));
        }
    }

    fn ship_phase(&self) -> Vec<ChunkOut> {
        let n = self.ships.len();
        let threads = self.threads.max(1);
        if threads == 1 || n < 1024 {
            let mut out = ChunkOut::default();
            for i in 0..n {
                self.think(i, &mut out);
            }
            return vec![out];
        }
        let chunk = n.div_ceil(threads);
        std::thread::scope(|sc| {
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    let (lo, hi) = ((t * chunk).min(n), ((t + 1) * chunk).min(n));
                    sc.spawn(move || {
                        let mut out = ChunkOut::default();
                        for i in lo..hi {
                            self.think(i, &mut out);
                        }
                        out
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("ship phase worker panicked"))
                .collect()
        })
    }

    /// One ship's decision for this step. Reads only the previous state.
    fn think(&self, i: usize, out: &mut ChunkOut) {
        let s = &self.ships;
        let mut it = Intent {
            x: s.x[i],
            y: s.y[i],
            vx: s.vx[i],
            vy: s.vy[i],
            heading: s.heading[i],
            target: s.target[i],
            cd_primary: s.cd_primary[i],
            cd_missile: s.cd_missile[i],
            cd_pd: s.cd_pd[i],
            stress: s.stress[i],
            salvo_until: s.salvo_until[i],
        };
        if s.state[i] != ALIVE {
            out.intents.push(it);
            return;
        }
        let st = s.stats(i);
        let g = &self.groups[s.group[i] as usize];
        let engine_hit = s.parts[i] & Part::Engine as u8 != 0;
        let (mut max_speed, turn) = if engine_hit {
            (st.max_speed / 2, (st.turn_rate / 2).max(1))
        } else {
            (st.max_speed, st.turn_rate)
        };
        let mut accel = st.accel as i64;
        if self.terrain.at(s.x[i], s.y[i]) == terrain::ASTEROIDS {
            max_speed = max_speed * 6 / 10;
        }
        if self.tick < g.boost_until && st.signature == Signature::Afterburn {
            max_speed *= 2;
            accel *= 2;
        }
        let (x, y) = (s.x[i] as i64, s.y[i] as i64);
        // Ships inside a nebula can only be seen from close by.
        let visible = |j: usize, d: i64| {
            d <= terrain::NEBULA_SIGHT as i64 || self.terrain.at(s.x[j], s.y[j]) != terrain::NEBULA
        };

        // Steering: chase the formation slot around the group anchor.
        let (ox, oy) = rotate(s.slot_dx[i] as i64, s.slot_dy[i] as i64, g.facing);
        let (dx, dy) = (g.anchor_x as i64 + ox - x, g.anchor_y as i64 + oy - y);
        let dist = len(dx, dy);
        let move_dir = atan2(dy, dx);
        let mut want_vx = 0i64;
        let mut want_vy = 0i64;
        if dist > 0 {
            let facing_move = angle_diff(move_dir, s.heading[i]).abs() < 10923; // 60 degrees
            let cap = if facing_move {
                max_speed
            } else {
                max_speed * 55 / 100
            } as i64;
            let want = cap.min(dist / 6);
            want_vx = dx * want / dist;
            want_vy = dy * want / dist;
        }
        // Soft separation from nearby friendly ships.
        let sep = st.radius as i64 * 3;
        // Look at no more than 24 nearby ships, so dense blobs stay cheap.
        let mut checked = 0;
        self.grid.query(s.x[i], s.y[i], sep as i32, |j| {
            let j = j as usize;
            checked += 1;
            if j == i || s.side[j] != s.side[i] {
                return checked < 24;
            }
            let (ex, ey) = (x - s.x[j] as i64, y - s.y[j] as i64);
            let d = len(ex, ey);
            if d < sep {
                let push = (sep - d) / 8;
                if d > 0 {
                    want_vx += ex * push / d;
                    want_vy += ey * push / d;
                } else {
                    want_vx += if i < j { push } else { -push };
                }
            }
            checked < 24
        });
        it.vx = (s.vx[i] as i64 + (want_vx - s.vx[i] as i64).clamp(-accel, accel)) as i32;
        it.vy = (s.vy[i] as i64 + (want_vy - s.vy[i] as i64).clamp(-accel, accel)) as i32;
        it.x = s.x[i] + it.vx;
        it.y = s.y[i] + it.vy;

        // Targeting: refresh every 5 steps (staggered), or when the target is gone.
        let reach = st.primary.range.max(st.missiles.map_or(0, |m| m.range));
        let mut target = s.target[i];
        let target_ok = |t: u32| t != u32::MAX && s.state[t as usize] == ALIVE;
        if !target_ok(target) || (self.tick + i as u32).is_multiple_of(5) {
            let mut best = (i64::MAX, u32::MAX);
            // First choice: a ship of the group we were told to fight. Sample up to
            // 24 of its ships from a rotating start so fire spreads across the group.
            if let Some(tg) = g.target_group {
                let list = &self.groups[tg as usize].ships;
                let n = list.len();
                if n > 0 {
                    let start = (hash(self.seed, self.tick, i as u32, 5) % n as u64) as usize;
                    for k in 0..n.min(24) {
                        let j = list[(start + k) % n];
                        let ju = j as usize;
                        if s.state[ju] != ALIVE {
                            continue;
                        }
                        let d = len(s.x[ju] as i64 - x, s.y[ju] as i64 - y);
                        if d <= reach as i64 && d < best.0 && visible(ju, d) {
                            best = (d, j);
                        }
                    }
                }
            }
            // Otherwise the nearest enemy, scanning at most 96 nearby ships.
            if best.1 == u32::MAX {
                let mut seen = 0;
                self.grid.query(s.x[i], s.y[i], reach, |j| {
                    seen += 1;
                    let ju = j as usize;
                    if s.side[ju] != s.side[i] {
                        let d = len(s.x[ju] as i64 - x, s.y[ju] as i64 - y);
                        if d <= reach as i64 && d < best.0 && visible(ju, d) {
                            best = (d, j);
                        }
                    }
                    seen < 96
                });
            }
            target = best.1;
        }
        it.target = target;

        // Facing: toward the target when one is near, else along the move, else the group facing.
        let mut tdist = i64::MAX;
        let mut tbear = 0u16;
        if target_ok(target) {
            let t = target as usize;
            let (tx, ty) = (s.x[t] as i64 - x, s.y[t] as i64 - y);
            tdist = len(tx, ty);
            tbear = atan2(ty, tx);
            if !visible(t, tdist) {
                target = u32::MAX;
                it.target = target;
                tdist = i64::MAX;
            }
        }
        let desired = if tdist <= reach as i64 * 11 / 10 {
            tbear
        } else if dist > 150 * FP as i64 {
            move_dir
        } else {
            g.facing
        };
        it.heading = turn_toward(s.heading[i], desired, turn);

        // Weapons.
        it.cd_primary = s.cd_primary[i].saturating_sub(1);
        it.cd_missile = s.cd_missile[i].saturating_sub(1);
        it.stress = (s.stress[i] - st.stress_decay).max(0);
        let can_fire = s.overload[i] == 0
            && !matches!(
                g.status,
                GroupStatus::Retreating { .. } | GroupStatus::Routed
            );
        // Salvo guns hold their fire while the signature move charges.
        let holding = g.charging() && st.signature == Signature::Salvo;
        let salvo = s.salvo_until[i] > self.tick;
        if can_fire && !holding && target_ok(target) {
            let t = target as usize;
            let w = &st.primary;
            let range = if salvo {
                w.range as i64 * 13 / 10
            } else {
                w.range as i64
            };
            if it.cd_primary == 0
                && tdist <= range
                && angle_diff(tbear, it.heading).abs() <= w.arc_half as i32
            {
                let mut acc = w.accuracy;
                if salvo {
                    acc += 150;
                }
                if self.terrain.at(s.x[t], s.y[t]) == terrain::ASTEROIDS {
                    acc -= terrain::ASTEROID_COVER;
                }
                acc += (s.stats(t).radius - 35 * FP) * 5 / FP;
                if tdist > w.range as i64 * 6 / 10 {
                    acc -= 150;
                }
                if g.cohesion < 300 {
                    acc -= 200;
                }
                if s.parts[i] & Part::Bridge as u8 != 0 {
                    acc -= 150;
                }
                let hit = roll(self.seed, self.tick, i as u32, 1, acc);
                let mut e = Event::new(self.tick, ev::FIRE, i as u32, target, hit as i32);
                e.d = w.dtype as u8;
                if salvo {
                    e.c |= 2;
                    it.salvo_until = 0;
                    it.stress += st.max_stress / 2;
                }
                if hit {
                    out.damages.push(Damage {
                        target,
                        src: i as u32,
                        amount: if salvo { w.damage * 3 } else { w.damage },
                        dtype: w.dtype,
                        from_x: s.x[i],
                        from_y: s.y[i],
                    });
                }
                out.events.push(e);
                let weapons_hit = s.parts[i] & Part::Weapons as u8 != 0;
                it.cd_primary = if weapons_hit {
                    w.cooldown * 2
                } else {
                    w.cooldown
                };
                it.stress += w.stress;
            }
            if let Some(m) = st.missiles {
                if it.cd_missile == 0 && tdist <= m.range as i64 {
                    out.launches.push((i as u32, target, 0));
                    it.cd_missile = m.cooldown;
                }
            }
        }
        // Point defense: shoot at the first enemy missile in range, else at a fighter wing.
        if it.cd_pd > 0 {
            it.cd_pd -= 1;
        } else {
            let pd = st.pd;
            let mut shot: Option<(u32, bool)> = None;
            if !self.missiles.is_empty() {
                let ms = &self.missiles;
                self.missile_grid.query(s.x[i], s.y[i], pd.range, |k| {
                    let ku = k as usize;
                    if ms.alive[ku]
                        && ms.side[ku] != s.side[i]
                        && len(ms.x[ku] as i64 - x, ms.y[ku] as i64 - y) <= pd.range as i64
                    {
                        shot = Some((k, false));
                        return false;
                    }
                    true
                });
            }
            if shot.is_none() && !self.wings.is_empty() {
                let ws = &self.wings;
                self.wing_grid.query(s.x[i], s.y[i], pd.range, |w| {
                    let wu = w as usize;
                    if ws.side[wu] != s.side[i]
                        && len(ws.x[wu] as i64 - x, ws.y[wu] as i64 - y) <= pd.range as i64
                    {
                        shot = Some((w, true));
                        return false;
                    }
                    true
                });
            }
            if let Some((id, wing)) = shot {
                it.cd_pd = pd.cooldown;
                out.intercepts.push(Intercept {
                    ship: i as u32,
                    id,
                    hit: roll(
                        self.seed,
                        self.tick,
                        i as u32,
                        2,
                        if wing {
                            pd.chance * PD_VS_FIGHTER / 100
                        } else {
                            pd.chance
                        },
                    ),
                    wing,
                });
            }
        }
        out.intents.push(it);
    }

    /// Signature moves whose charge runs out this step take effect now.
    fn fire_signatures(&mut self, launches: &mut Vec<(u32, u32, u8)>) {
        let tick = self.tick;
        for gi in 0..self.groups.len() {
            if self.groups[gi].sig_fire_at != tick {
                continue;
            }
            let g = &mut self.groups[gi];
            g.sig_fire_at = NONE;
            g.sig_ready_at = tick + SIG_COOLDOWN_TICKS;
            if !matches!(g.status, GroupStatus::Active) {
                continue;
            }
            let target_group = g.target_group;
            let ships = g.ships.clone();
            let mut boost = false;
            for &id in &ships {
                let i = id as usize;
                if self.ships.state[i] != ALIVE {
                    continue;
                }
                match self.ships.stats(i).signature {
                    Signature::Salvo => self.ships.salvo_until[i] = tick + 5 * TICK_HZ,
                    Signature::Afterburn => boost = true,
                    Signature::Torpedoes => {
                        let t = self.torpedo_target(i, target_group);
                        if t != NONE {
                            launches.push((id, t, 1));
                            launches.push((id, t, 1));
                        }
                    }
                    Signature::Scramble => {
                        for w in 0..self.wings.len() {
                            if self.wings.carrier[w] == id && self.wings.state[w] != W_LOST {
                                self.wings.count[w] = WING_SIZE;
                                if self.wings.state[w] != W_OUT {
                                    self.launch_wing(w);
                                }
                            }
                        }
                    }
                }
            }
            if boost {
                self.groups[gi].boost_until = tick + AFTERBURN_TICKS;
            }
            self.events
                .push(Event::new(tick, ev::SIGNATURE_FIRED, gi as u32, 0, 0));
        }
    }

    /// Torpedo target: the ship's own target, else the nearest living ship of the group's target.
    fn torpedo_target(&self, i: usize, target_group: Option<u16>) -> u32 {
        let s = &self.ships;
        let t = s.target[i];
        if t != NONE && s.state[t as usize] == ALIVE {
            return t;
        }
        let Some(tg) = target_group else { return NONE };
        let mut best = (i64::MAX, NONE);
        for &j in &self.groups[tg as usize].ships {
            let ju = j as usize;
            if s.state[ju] == ALIVE {
                let d = len((s.x[ju] - s.x[i]) as i64, (s.y[ju] - s.y[i]) as i64);
                if d < best.0 {
                    best = (d, j);
                }
            }
        }
        best.1
    }

    fn launch_wing(&mut self, w: usize) {
        let ws = &mut self.wings;
        ws.state[w] = W_OUT;
        ws.target[w] = NONE;
        ws.target_wing[w] = NONE;
        ws.cd[w] = 0;
        self.events.push(Event::new(
            self.tick,
            ev::WING_LAUNCHED,
            w as u32,
            ws.carrier[w],
            ws.count[w] as i32,
        ));
    }

    /// Fighter wings, updated one after another in wing order.
    fn wing_phase(&mut self, damages: &mut Vec<Damage>) {
        for w in 0..self.wings.len() {
            let state = self.wings.state[w];
            if state == W_LOST {
                continue;
            }
            let c = self.wings.carrier[w] as usize;
            let home = self.ships.state[c] == ALIVE;
            if self.wings.count[w] == 0 || (state == W_DOCKED && !home) {
                self.wings.count[w] = 0;
                self.wings.state[w] = W_LOST;
                self.events
                    .push(Event::new(self.tick, ev::WING_LOST, w as u32, 0, 0));
                continue;
            }
            let g = &self.groups[self.ships.group[c] as usize];
            let group_fighting = g.status == GroupStatus::Active;
            match state {
                W_DOCKED => {
                    let ws = &mut self.wings;
                    ws.x[w] = self.ships.x[c];
                    ws.y[w] = self.ships.y[c];
                    if ws.count[w] < WING_SIZE {
                        ws.rearm[w] = ws.rearm[w].saturating_sub(1);
                        if ws.rearm[w] == 0 {
                            ws.count[w] += 1;
                            ws.rearm[w] = REARM_TICKS;
                        }
                    }
                    let enemy_near = g.target_group.is_some_and(|t| {
                        let t = &self.groups[t as usize];
                        len((t.cx - g.cx) as i64, (t.cy - g.cy) as i64) < 6000 * FP as i64
                    });
                    if group_fighting && enemy_near && self.wings.count[w] >= WING_SIZE - 2 {
                        self.launch_wing(w);
                    }
                }
                W_RETURNING => {
                    if !home {
                        self.wings.state[w] = W_OUT;
                        continue;
                    }
                    let (cx, cy) = (self.ships.x[c], self.ships.y[c]);
                    if self.fly(w, cx, cy, self.ships.stats(c).radius) {
                        let ws = &mut self.wings;
                        ws.state[w] = W_DOCKED;
                        ws.rearm[w] = REARM_TICKS;
                    }
                }
                _ => {
                    let going_home = !group_fighting || self.wings.count[w] <= WING_RETURN_AT;
                    if home && going_home {
                        self.wings.state[w] = W_RETURNING;
                        continue;
                    }
                    self.wing_fight(w, c, damages);
                }
            }
        }
    }

    /// Move a wing toward (x, y). Returns true once within `reach` of it.
    fn fly(&mut self, w: usize, x: i32, y: i32, reach: i32) -> bool {
        let ws = &mut self.wings;
        let (dx, dy) = ((x - ws.x[w]) as i64, (y - ws.y[w]) as i64);
        let d = len(dx, dy);
        if d <= reach as i64 {
            return true;
        }
        let step = (WING_SPEED as i64).min(d - reach as i64 / 2);
        ws.x[w] += (dx * step / d) as i32;
        ws.y[w] += (dy * step / d) as i32;
        len((x - ws.x[w]) as i64, (y - ws.y[w]) as i64) <= reach as i64
    }

    fn wing_fight(&mut self, w: usize, carrier: usize, damages: &mut Vec<Damage>) {
        let side = self.wings.side[w];
        let (wx, wy) = (self.wings.x[w] as i64, self.wings.y[w] as i64);
        let (hx, hy) = (self.ships.x[carrier] as i64, self.ships.y[carrier] as i64);
        self.wings.cd[w] = self.wings.cd[w].saturating_sub(1);
        // Retarget twice a second, or when the target is gone.
        let tw = self.wings.target_wing[w];
        let tw_ok = tw != NONE && self.wings.flying(tw as usize);
        let t = self.wings.target[w];
        let t_ok = t != NONE && self.ships.state[t as usize] == ALIVE;
        if (self.tick + w as u32).is_multiple_of(10) || !(tw_ok || t_ok) {
            // Enemy fighters near our carrier or near us come first.
            let mut best = (i64::MAX, NONE);
            let ws = &self.wings;
            for o in 0..ws.len() {
                if ws.side[o] == side || !ws.flying(o) {
                    continue;
                }
                let (ox, oy) = (ws.x[o] as i64, ws.y[o] as i64);
                let d = len(ox - wx, oy - wy);
                let near = d < 1500 * FP as i64 || len(ox - hx, oy - hy) < 1500 * FP as i64;
                if near && d < best.0 {
                    best = (d, o as u32);
                }
            }
            self.wings.target_wing[w] = best.1;
            if best.1 == NONE {
                self.wings.target[w] = self.wing_ship_target(w, carrier);
            }
        }
        let tw = self.wings.target_wing[w];
        if tw != NONE && self.wings.flying(tw as usize) {
            let o = tw as usize;
            let (ox, oy) = (self.wings.x[o], self.wings.y[o]);
            if self.fly(w, ox, oy, WING_DOGFIGHT_RANGE) && self.wings.cd[w] == 0 {
                self.wings.cd[w] = DOGFIGHT_TICKS;
                let mut kills = 0;
                for f in 0..self.wings.count[w] {
                    if roll(
                        self.seed,
                        self.tick,
                        w as u32,
                        7 + f as u32 * 16,
                        DOGFIGHT_CHANCE,
                    ) {
                        kills += 1;
                    }
                }
                let kills = kills.min(self.wings.count[o]);
                self.wings.count[o] -= kills;
                for _ in 0..kills {
                    self.events.push(Event::new(
                        self.tick,
                        ev::FIGHTER_DOWN,
                        o as u32,
                        w as u32 | 1 << 31,
                        0,
                    ));
                }
            }
            return;
        }
        self.wings.target_wing[w] = NONE;
        let t = self.wings.target[w];
        if t == NONE || self.ships.state[t as usize] != ALIVE {
            // Nothing to hit: circle home.
            let (hx, hy) = (hx as i32, hy as i32);
            self.fly(w, hx, hy, 400 * FP);
            return;
        }
        let tu = t as usize;
        let (tx, ty) = (self.ships.x[tu], self.ships.y[tu]);
        if self.fly(w, tx, ty, WING_STRIKE_RANGE) && self.wings.cd[w] == 0 {
            self.wings.cd[w] = WING_PASS_TICKS;
            let amount = self.wings.count[w] as i32 * FIGHTER_DAMAGE;
            damages.push(Damage {
                target: t,
                src: carrier as u32,
                amount,
                dtype: DamageType::Missile,
                from_x: self.wings.x[w],
                from_y: self.wings.y[w],
            });
            self.events
                .push(Event::new(self.tick, ev::WING_STRIKE, w as u32, t, amount));
        }
    }

    /// A ship for a wing to strike: from the carrier group's target first, else the nearest enemy.
    fn wing_ship_target(&self, w: usize, carrier: usize) -> u32 {
        let s = &self.ships;
        let (wx, wy) = (self.wings.x[w] as i64, self.wings.y[w] as i64);
        let side = self.wings.side[w];
        let visible = |j: usize, d: i64| {
            d <= terrain::NEBULA_SIGHT as i64 || self.terrain.at(s.x[j], s.y[j]) != terrain::NEBULA
        };
        let mut best = (i64::MAX, NONE);
        // Strike the most valuable group within reach that has the thinnest
        // point-defense screen.
        let reach = 7000 * FP as i64;
        let (hx, hy) = (s.x[carrier] as i64, s.y[carrier] as i64);
        let prey = self
            .groups
            .iter()
            .filter(|g| {
                g.side != side
                    && g.status.in_battle()
                    && g.alive > 0
                    && len(g.cx as i64 - hx, g.cy as i64 - hy) <= reach
            })
            .max_by_key(|g| {
                (
                    g.alive_value as i64 * 1000 / (1 + g.screen as i64 * 2),
                    std::cmp::Reverse(g.id),
                )
            })
            .map(|g| g.id)
            .or(self.groups[s.group[carrier] as usize].target_group);
        if let Some(tg) = prey {
            let list = &self.groups[tg as usize].ships;
            let n = list.len();
            if n > 0 {
                let start = (hash(self.seed, self.tick, w as u32, 8) % n as u64) as usize;
                for k in 0..n.min(24) {
                    let j = list[(start + k) % n] as usize;
                    if s.state[j] != ALIVE {
                        continue;
                    }
                    let d = len(s.x[j] as i64 - wx, s.y[j] as i64 - wy);
                    if d < best.0 && visible(j, d) {
                        best = (d, j as u32);
                    }
                }
            }
        }
        if best.1 == NONE {
            let mut seen = 0;
            self.grid
                .query(self.wings.x[w], self.wings.y[w], 5000 * FP, |j| {
                    seen += 1;
                    let ju = j as usize;
                    if s.side[ju] != side {
                        let d = len(s.x[ju] as i64 - wx, s.y[ju] as i64 - wy);
                        if d < best.0 && visible(ju, d) {
                            best = (d, j);
                        }
                    }
                    seen < 96
                });
        }
        best.1
    }

    fn launch_missiles(&mut self, launches: &[(u32, u32, u8)]) {
        for &(src, target, kind) in launches {
            let i = src as usize;
            let m = if kind == 1 {
                TORPEDO
            } else {
                self.ships.stats(i).missiles.expect("launcher without rack")
            };
            let id = self.missiles.len() as u32;
            let ms = &mut self.missiles;
            ms.x.push(self.ships.x[i]);
            ms.y.push(self.ships.y[i]);
            ms.target.push(target);
            ms.src.push(src);
            ms.side.push(self.ships.side[i]);
            ms.damage.push(m.damage);
            ms.speed.push(m.speed);
            ms.life
                .push(((m.range as i64 * 3 / 2) / m.speed as i64) as u16);
            ms.alive.push(true);
            ms.kind.push(kind);
            ms.hp.push(if kind == 1 { TORPEDO_HP } else { 1 });
            let mut e = Event::new(self.tick, ev::MISSILE_LAUNCH, src, id, target as i32);
            e.d = kind;
            self.events.push(e);
        }
    }

    fn missile_phase(&mut self, intercepts: &[Intercept], damages: &mut Vec<Damage>) {
        // Point-defense hits land in ship order; hits on an already stopped
        // missile or an empty wing are wasted.
        for p in intercepts {
            if !p.hit {
                continue;
            }
            let k = p.id as usize;
            if p.wing {
                let ws = &mut self.wings;
                if ws.count[k] > 0 {
                    ws.count[k] -= 1;
                    self.events
                        .push(Event::new(self.tick, ev::FIGHTER_DOWN, p.id, p.ship, 0));
                }
            } else {
                let ms = &mut self.missiles;
                if ms.alive[k] {
                    ms.hp[k] -= 1;
                    if ms.hp[k] == 0 {
                        ms.alive[k] = false;
                        self.events.push(Event::new(
                            self.tick,
                            ev::MISSILE_INTERCEPTED,
                            p.ship,
                            p.id,
                            0,
                        ));
                    }
                }
            }
        }
        if self.missiles.is_empty() {
            return;
        }
        let s = &self.ships;
        let ms = &mut self.missiles;
        // Flight.
        for k in 0..ms.len() {
            if !ms.alive[k] {
                continue;
            }
            ms.life[k] = ms.life[k].saturating_sub(1);
            if ms.life[k] == 0 {
                ms.alive[k] = false;
                continue;
            }
            if self.terrain.at(ms.x[k], ms.y[k]) == terrain::ASTEROIDS
                && roll(
                    self.seed,
                    self.tick,
                    k as u32,
                    6,
                    terrain::ASTEROID_MISSILE_LOSS,
                )
            {
                ms.alive[k] = false;
                self.events.push(Event::new(
                    self.tick,
                    ev::MISSILE_INTERCEPTED,
                    u32::MAX,
                    k as u32,
                    0,
                ));
                continue;
            }
            let t = ms.target[k] as usize;
            if s.state[t] != ALIVE {
                ms.alive[k] = false;
                continue;
            }
            let (dx, dy) = (
                s.x[t] as i64 - ms.x[k] as i64,
                s.y[t] as i64 - ms.y[k] as i64,
            );
            let d = len(dx, dy);
            let sp = ms.speed[k] as i64;
            if d <= sp + ShipClass::from_u8(s.class[t]).stats().radius as i64 {
                ms.alive[k] = false;
                damages.push(Damage {
                    target: t as u32,
                    src: ms.src[k],
                    amount: ms.damage[k],
                    dtype: DamageType::Missile,
                    from_x: ms.x[k],
                    from_y: ms.y[k],
                });
                self.events.push(Event::new(
                    self.tick,
                    ev::MISSILE_HIT,
                    k as u32,
                    t as u32,
                    0,
                ));
            } else {
                ms.x[k] += (dx * sp / d) as i32;
                ms.y[k] += (dy * sp / d) as i32;
            }
        }
        // Drop spent missiles once none are in flight, so the arrays don't grow forever.
        if ms.alive.iter().all(|a| !a) {
            *ms = Missiles::default();
        }
    }

    fn apply_damage(&mut self, d: &Damage) {
        let t = d.target as usize;
        if self.ships.state[t] != ALIVE {
            return;
        }
        let st = self.ships.stats(t);
        let s = &mut self.ships;
        let bearing = atan2((d.from_y - s.y[t]) as i64, (d.from_x - s.x[t]) as i64);
        let rel = angle_diff(bearing, s.heading[t]);
        let q = if rel.abs() <= 8192 {
            0
        } else if rel.abs() >= 24576 {
            2
        } else if rel > 0 {
            1
        } else {
            3
        };
        let mut raw = d.amount as i64;
        if d.dtype == DamageType::Beam && self.terrain.at(s.x[t], s.y[t]) == terrain::NEBULA {
            raw /= 2;
        }
        let shields_up = s.overload[t] == 0 && s.parts[t] & Part::ShieldGen as u8 == 0;
        if shields_up && s.shield[t][q] > 0 {
            let mult = d.dtype.vs_shield();
            let eff = raw * mult / 100;
            let absorbed = eff.min(s.shield[t][q] as i64);
            s.shield[t][q] -= absorbed as i32;
            s.stress[t] += (absorbed / 3) as i32;
            raw = (eff - absorbed) * 100 / mult;
        }
        s.shield_delay[t] = 60;
        let armor = st.armor as i64 * d.dtype.armor_share() / 100;
        let hull_dmg = (raw * (100 - armor) / 100) as i32;
        s.hull[t] -= hull_dmg;
        let side = s.side[t] as usize;
        self.damage_by_type[side][d.dtype as usize] += d.amount as i64;

        let tg = s.group[t] as usize;
        let src = d.src as usize;
        let sg = s.group[src] as usize;
        self.groups[sg].damage_dealt += d.amount as i64;
        self.groups[tg].damage_taken += d.amount as i64;

        // Hits on the flanks and rear shake a group far more than hits on its front.
        let k_q: i64 = [20, 60, 120, 60][q];
        let init_hull = self.groups[tg].init_hull;
        self.groups[tg].cohesion -=
            (hull_dmg as i64 * k_q * COHESION_MAX as i64 / 100 / init_hull) as i32;

        if hull_dmg > 0 {
            let focus = self.groups[sg].focus_part;
            let chance = 60 + if focus.is_some() { 250 } else { 0 };
            if roll(
                self.seed,
                self.tick,
                d.target,
                3 + d.src.wrapping_mul(7),
                chance,
            ) {
                let part = focus.unwrap_or_else(|| {
                    let h = hash(self.seed, self.tick, d.target, 4 + d.src) % 4;
                    [Part::Engine, Part::Weapons, Part::ShieldGen, Part::Bridge][h as usize]
                }) as u8;
                if self.ships.parts[t] & part == 0 {
                    self.ships.parts[t] |= part;
                    self.events.push(Event::new(
                        self.tick,
                        ev::PART_DAMAGED,
                        t as u32,
                        0,
                        part as i32,
                    ));
                }
            }
        }
        if self.ships.hull[t] <= 0 {
            self.kill(t, d.src);
        }
    }

    fn kill(&mut self, t: usize, killer: u32) {
        let s = &mut self.ships;
        s.state[t] = DEAD;
        s.hull[t] = 0;
        let class = ShipClass::from_u8(s.class[t]);
        let (tg, side) = (s.group[t] as usize, s.side[t]);
        if killer != u32::MAX {
            let k = killer as usize;
            s.kills[k] = s.kills[k].saturating_add(1);
            let kg = s.group[k] as usize;
            self.groups[kg].kills += 1;
        }
        self.value_lost[side as usize] += class.stats().cost;
        let g = &mut self.groups[tg];
        g.alive = g.alive.saturating_sub(1);
        g.cohesion -= class.stats().cost * COHESION_MAX * 12 / 10 / g.init_cost;
        self.events
            .push(Event::new(self.tick, ev::SHIP_KILLED, t as u32, killer, 0));
        if class == ShipClass::Flagship {
            self.events.push(Event::new(
                self.tick,
                ev::FLAGSHIP_LOST,
                t as u32,
                side as u32,
                0,
            ));
            for g in &mut self.groups {
                if g.side == side && g.status.in_battle() {
                    g.cohesion -= 200;
                }
            }
        }
    }

    fn upkeep(&mut self) {
        let s = &mut self.ships;
        for i in 0..s.len() {
            if s.state[i] != ALIVE {
                continue;
            }
            let st = ShipClass::from_u8(s.class[i]).stats();
            if s.overload[i] > 0 {
                s.overload[i] -= 1;
            } else if s.stress[i] >= st.max_stress {
                s.overload[i] = 60;
                s.stress[i] = st.max_stress / 2;
                s.shield[i] = [0; 4];
                self.events
                    .push(Event::new(self.tick, ev::OVERLOAD, i as u32, 0, 0));
            }
            if s.shield_delay[i] > 0 {
                s.shield_delay[i] -= 1;
            } else if s.overload[i] == 0
                && s.parts[i] & Part::ShieldGen as u8 == 0
                && self.terrain.at(s.x[i], s.y[i]) != terrain::NEBULA
            {
                let max = [st.shield[0], st.shield[1], st.shield[2], st.shield[1]];
                for q in 0..4 {
                    s.shield[i][q] = (s.shield[i][q] + st.shield_regen).min(max[q]);
                }
            }
            // Routed ships that reach their own edge get away.
            let g = &self.groups[s.group[i] as usize];
            if g.status == GroupStatus::Routed && (s.x[i].abs() >= FIELD_HALF - 500 * FP) {
                s.state[i] = ESCAPED;
            }
        }
    }

    /// Total starting value of a side's fleet.
    pub fn side_value(&self, side: u8) -> i32 {
        self.groups
            .iter()
            .filter(|g| g.side == side)
            .map(|g| g.init_cost)
            .sum()
    }

    fn check_groups(&mut self) {
        let tick = self.tick;
        // A fleet that has lost half its value wavers: every group bleeds cohesion.
        let wavering = [0u8, 1].map(|sd| self.value_lost[sd as usize] * 2 > self.side_value(sd));
        for g in &mut self.groups {
            if wavering[g.side as usize] && g.status.in_battle() {
                g.cohesion -= 3;
            }
        }
        for gi in 0..self.groups.len() {
            let alive = self.groups[gi]
                .ships
                .iter()
                .filter(|&&id| self.ships.state[id as usize] == ALIVE)
                .count() as u32;
            let g = &mut self.groups[gi];
            let escaped_now = g.alive.saturating_sub(alive);
            g.alive = alive;
            g.cohesion = g.cohesion.max(0);
            match g.status {
                GroupStatus::Retreating { until } if tick + 1 >= until => {
                    let mut saved = 0;
                    for &id in &g.ships {
                        if self.ships.state[id as usize] == ALIVE {
                            self.ships.state[id as usize] = ESCAPED;
                            saved += 1;
                        }
                    }
                    g.alive = 0;
                    g.status = GroupStatus::Escaped;
                    self.events
                        .push(Event::new(tick, ev::GROUP_ESCAPED, gi as u32, 0, saved));
                }
                GroupStatus::Routed if alive == 0 => {
                    g.status = GroupStatus::Escaped;
                    self.events.push(Event::new(
                        tick,
                        ev::GROUP_ESCAPED,
                        gi as u32,
                        0,
                        escaped_now as i32,
                    ));
                }
                GroupStatus::Active | GroupStatus::Reserve | GroupStatus::Retreating { .. }
                    if alive == 0 =>
                {
                    g.status = GroupStatus::Destroyed;
                }
                GroupStatus::Active | GroupStatus::Reserve if g.cohesion == 0 => {
                    g.status = GroupStatus::Routed;
                    self.events
                        .push(Event::new(tick, ev::GROUP_ROUTED, gi as u32, 0, 0));
                }
                _ => {}
            }
        }
    }

    fn check_outcome(&mut self) {
        let fighting = |side: u8| {
            self.groups
                .iter()
                .any(|g| g.side == side && g.status.in_battle() && g.alive > 0)
        };
        let (a, b) = (fighting(0), fighting(1));
        let winner = match (a, b) {
            (true, true) if self.tick + 1 >= MAX_PULSES * PULSE_TICKS => Some(-1),
            (true, true) => None,
            (true, false) => Some(0),
            (false, true) => Some(1),
            (false, false) => Some(-1),
        };
        if let Some(w) = winner {
            self.outcome = Some(Outcome {
                winner: w,
                tick: self.tick + 1,
            });
            self.events
                .push(Event::new(self.tick, ev::BATTLE_OVER, 0, 0, w));
        }
    }

    /// FNV-1a over the whole simulation state; equal hashes mean equal battles.
    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |v: i64| {
            for b in v.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        let s = &self.ships;
        for i in 0..s.len() {
            eat(s.x[i] as i64);
            eat(s.y[i] as i64);
            eat(s.vx[i] as i64);
            eat(s.vy[i] as i64);
            eat(s.heading[i] as i64);
            eat(s.hull[i] as i64);
            eat(s.shield[i].iter().map(|&v| v as i64).sum());
            eat(s.stress[i] as i64);
            eat(s.parts[i] as i64);
            eat(s.target[i] as i64);
            eat(s.state[i] as i64);
        }
        for g in &self.groups {
            eat(g.cohesion as i64);
            eat(g.anchor_x as i64);
            eat(g.anchor_y as i64);
            eat(g.status.code() as i64);
        }
        let m = &self.missiles;
        for k in 0..m.len() {
            eat(m.x[k] as i64);
            eat(m.y[k] as i64);
            eat(m.alive[k] as i64);
            eat(m.hp[k] as i64);
        }
        let w = &self.wings;
        for k in 0..w.len() {
            eat(w.x[k] as i64);
            eat(w.y[k] as i64);
            eat(w.count[k] as i64);
            eat(w.state[k] as i64);
        }
        for g in &self.groups {
            eat(g.sig_fire_at as i64);
            eat(g.sig_ready_at as i64);
        }
        eat(self.tick as i64);
        h
    }
}
