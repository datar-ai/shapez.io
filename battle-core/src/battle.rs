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
    /// Height layer (`LAYER_LOW` or `LAYER_MAIN`), plus `LAYER_MOVING`
    /// while the group climbs or dives.
    pub layer: Vec<u8>,
    /// Steps the ship has held still (artillery deploys by holding still).
    pub still: Vec<u16>,
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
    /// Signature move bought and waiting for the right moment (the enemy in
    /// reach); then it starts charging.
    pub sig_armed: bool,
    /// Signature move charging; fires at this tick. `NONE` when idle.
    pub sig_fire_at: u32,
    /// Which signature moves the group's ships have (bit per `Signature`).
    pub sig_mask: u8,
    /// Afterburn runs until this tick.
    pub boost_until: u32,
    /// Value of living ships and how many of them carry heavy point defense
    /// (destroyers, carriers); refreshed when groups re-plan. Wings use it to
    /// pick soft targets.
    pub alive_value: i32,
    pub screen: u32,
    /// Doctrine card state (see `Phase`) and when it last changed.
    pub phase: Phase,
    pub phase_since: u32,
    /// Shields left, permille of full; refreshed when groups re-plan.
    pub shield_pm: i64,
    /// Height layer the group is on; while it climbs or dives, the move
    /// completes at `layer_until` (`NONE` when idle).
    pub layer: u8,
    pub layer_until: u32,
    /// A structure (station, beacon, pylon): never moves or takes orders.
    pub fixed: bool,
    /// Has long-range gunships, which must stop to fire.
    pub deploys: bool,
    /// Which way the anchor is steering round a planet (0: straight).
    pub detour: i8,
    /// Last tick one of its ships took damage.
    pub last_hit: u32,
    /// Has pulled back to refit once already.
    pub refitted: bool,
}

pub const NONE: u32 = u32::MAX;

impl Group {
    pub fn charging(&self) -> bool {
        self.sig_fire_at != NONE
    }
}

/// A marked barrage: lands on (x, y) at `land`, hitting every enemy of
/// `side` inside `BARRAGE_RADIUS`.
#[derive(Clone, Copy, Debug)]
pub struct Strike {
    pub x: i32,
    pub y: i32,
    pub side: u8,
    pub land: u32,
    pub damage: i32,
    pub src: u32,
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
    /// Place in the carrier's hangar; the carrier's wings sit next to each
    /// other, and slot / `WING_WAVE` is the launch wave.
    pub slot: Vec<u8>,
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
    /// When true, this side reads the terrain: its gun lines and anvils fight
    /// the enemies they can see before ones hidden in a nebula, its fast
    /// groups close in to flush hidden ones out, and its commander plays
    /// candidate moves forward to pick where to fight (see `ai::plan`). Off
    /// only for measuring what reading the terrain is worth.
    pub terrain_sense: bool,
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
    /// From above or below (another layer, or a barrage): hits a side shield.
    vertical: bool,
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
    still: u16,
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

#[derive(Clone)]
pub struct Battle {
    pub tick: u32,
    pub seed: u64,
    /// Worker threads for the ship phase. Any value gives the same result.
    pub threads: usize,
    pub ships: Ships,
    pub groups: Vec<Group>,
    pub missiles: Missiles,
    pub wings: Wings,
    /// Marked barrages, landed ones included (the id is the index).
    pub strikes: Vec<Strike>,
    pub terrain: Terrain,
    pub sides: [SideState; 2],
    /// Events produced by the most recent step (and commands issued since).
    pub events: Vec<Event>,
    pub outcome: Option<Outcome>,
    /// Damage taken per side and damage type, for the after-action report.
    pub damage_by_type: [[i64; 3]; 2],
    /// Fleet value lost per side; past half, the whole fleet wavers.
    pub value_lost: [i32; 2],
    /// Last tick a ship was sunk; a long quiet spell makes both commanders
    /// press the attack (see `ai::plan`).
    pub last_kill: u32,
    started: bool,
    /// True for the copies the commander runs ahead to compare plans; they
    /// do not plan ahead themselves.
    pub(crate) lookahead: bool,
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
            strikes: Vec::new(),
            terrain: Terrain::default(),
            sides: [SideState {
                command_points: 0,
                ai: true,
                terrain_sense: true,
            }; 2],
            events: Vec::new(),
            outcome: None,
            damage_by_type: [[0; 3]; 2],
            value_lost: [0; 2],
            last_kill: 0,
            started: false,
            lookahead: false,
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
        let mut sig_mask = 0u8;
        let (mut fixed, mut deploys) = (true, false);
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
                s.layer.push(LAYER_MAIN);
                s.still.push(0);
                for slot in 0..st.hangar {
                    let w = &mut self.wings;
                    w.x.push(x + wx as i32);
                    w.y.push(y + wy as i32);
                    w.side.push(side);
                    w.carrier.push(id);
                    w.slot.push(slot);
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
                // Support ships hang back; they do not set the fighting distance.
                if st.repair == 0 {
                    range = range.min(st.primary.range);
                }
                has_carrier |= st.hangar > 0;
                fixed &= st.fixed();
                deploys |= st.deploy > 0;
                sig_mask |= 1 << st.signature as u8;
                turn = turn.min(st.turn_rate);
                k += 1;
            }
        }
        if has_carrier {
            // Carriers stand off and let their wings do the fighting.
            range = CARRIER_STANDOFF;
        }
        if range == i32::MAX {
            range = 1000 * FP;
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
            sig_armed: false,
            sig_fire_at: NONE,
            sig_mask,
            boost_until: 0,
            alive_value: cost,
            screen: 0,
            phase: Phase::Approach,
            phase_since: 0,
            shield_pm: 1000,
            layer: LAYER_MAIN,
            layer_until: NONE,
            fixed,
            deploys,
            detour: 0,
            last_hit: 0,
            refitted: false,
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
        if g.fixed {
            return Err(CommandError::Fixed);
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
                if is_reserve || g.sig_armed || g.charging() || self.tick < g.sig_ready_at =>
            {
                return Err(CommandError::SignatureNotReady)
            }
            Command::ChangeLayer { .. } if g.layer_until != NONE => {
                return Err(CommandError::ChangingLayer)
            }
            Command::Retreat { .. } if self.in_gravity_well(g) => {
                return Err(CommandError::InGravityWell)
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
        let retreat_ticks =
            if matches!(cmd, Command::Retreat { .. }) && self.near_own_beacon(&self.groups[gi]) {
                BEACON_RETREAT_TICKS
            } else {
                RETREAT_TICKS
            };
        if let Command::CommitReserve { .. } = cmd {
            self.jump_in(gi);
        }
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
                    until: tick + retreat_ticks,
                };
                g.goal_x = g.cx;
                g.goal_y = g.cy;
                self.events
                    .push(Event::new(tick, ev::RETREAT_STARTED, gi as u32, 0, 0));
                8
            }
            Command::Signature { .. } => {
                g.sig_armed = true;
                9
            }
            Command::ChangeLayer { .. } => {
                let to = if g.layer == LAYER_MAIN {
                    LAYER_LOW
                } else {
                    LAYER_MAIN
                };
                g.layer_until = tick + LAYER_CHANGE_TICKS;
                for &id in &g.ships {
                    self.ships.layer[id as usize] |= LAYER_MOVING;
                }
                let mut e = Event::new(tick, ev::LAYER_CHANGED, gi as u32, 0, to as i32);
                e.d = 1;
                self.events.push(e);
                10
            }
        };
        self.events
            .push(Event::new(tick, ev::COMMAND, gi as u32, side as u32, kind));
        Ok(())
    }

    /// True when the group's centre sits in a gravity well.
    pub fn in_gravity_well(&self, g: &Group) -> bool {
        self.terrain.at(g.cx, g.cy) == terrain::GRAVITY
    }

    /// The side's living jump beacon, if any.
    fn beacon_of(&self, side: u8) -> Option<usize> {
        let s = &self.ships;
        (0..s.len()).find(|&i| {
            s.side[i] == side && s.state[i] == ALIVE && s.class[i] == ShipClass::Beacon as u8
        })
    }

    fn near_own_beacon(&self, g: &Group) -> bool {
        self.beacon_of(g.side).is_some_and(|b| {
            let s = &self.ships;
            len((s.x[b] - g.cx) as i64, (s.y[b] - g.cy) as i64) <= BEACON_REACH as i64
        })
    }

    /// A committed reserve jumps in at its side's beacon, facing the nearest
    /// enemy. Without a beacon it starts from where it waited.
    fn jump_in(&mut self, gi: usize) {
        let g = &self.groups[gi];
        let Some(b) = self.beacon_of(g.side) else {
            return;
        };
        let (bx, by) = (self.ships.x[b], self.ships.y[b]);
        let mut facing = g.facing;
        let mut best = i64::MAX;
        for o in &self.groups {
            if o.side != g.side && o.status.in_battle() && o.alive > 0 && !o.fixed {
                let d = len((o.cx - bx) as i64, (o.cy - by) as i64);
                if d < best {
                    best = d;
                    facing = atan2((o.cy - by) as i64, (o.cx - bx) as i64);
                }
            }
        }
        // Come out a little ahead of the beacon.
        let (ox, oy) = polar(500 * FP as i64, facing);
        let (cx, cy) = (bx + ox as i32, by + oy as i32);
        let ships = g.ships.clone();
        let s = &mut self.ships;
        for &id in &ships {
            let i = id as usize;
            if s.state[i] != ALIVE {
                continue;
            }
            let (wx, wy) = rotate(s.slot_dx[i] as i64, s.slot_dy[i] as i64, facing);
            s.x[i] = cx + wx as i32;
            s.y[i] = cy + wy as i32;
            s.vx[i] = 0;
            s.vy[i] = 0;
            s.heading[i] = facing;
        }
        let g = &mut self.groups[gi];
        g.anchor_x = cx;
        g.anchor_y = cy;
        g.goal_x = cx;
        g.goal_y = cy;
        g.cx = cx;
        g.cy = cy;
        g.facing = facing;
        g.want_facing = facing;
        self.events
            .push(Event::new(self.tick, ev::JUMP_IN, gi as u32, b as u32, 0));
    }

    fn begin_pulse(&mut self) {
        for s in 0..2 {
            self.sides[s].command_points = COMMAND_POINTS;
        }
        self.events
            .push(Event::new(self.tick, ev::PULSE, 0, 0, self.pulse() as i32));
        // Both commanders plan from the same picture, then their orders land.
        let plans = [0u8, 1].map(|s| {
            if self.sides[s as usize].ai {
                crate::ai::plan(self, s)
            } else {
                Vec::new()
            }
        });
        for (s, plan) in plans.into_iter().enumerate() {
            for cmd in plan {
                let _ = self.issue(s as u8, cmd);
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
                    s.still[i] = it.still;
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
        self.land_strikes(&mut damages);
        self.launch_missiles(&launches);
        for d in &damages {
            self.apply_damage(d);
        }
        self.upkeep();
        self.repair();
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

    /// Default target: stay on the current enemy group while it is still
    /// fighting, unless another one is clearly closer. Without this, slow gun
    /// lines keep swinging after whichever fast group passes by.
    fn engage_target(&self, g: &Group) -> Option<u16> {
        // Reading the terrain: an enemy hidden in a nebula counts as three
        // times as far, so gun lines fight what they can see first.
        let sense = self.sides[g.side as usize].terrain_sense;
        let mut nearest: Option<(i64, u16)> = None;
        for o in &self.groups {
            if o.side == g.side || !o.status.in_battle() || o.alive == 0 {
                continue;
            }
            let mut d = len((o.cx - g.cx) as i64, (o.cy - g.cy) as i64);
            if sense && g.doctrine != Doctrine::Hammer && self.hidden(o) {
                d *= 3;
            }
            // Structures come after the enemy fleet.
            if o.fixed && !g.fixed {
                d *= 2;
            }
            if nearest.is_none_or(|(bd, _)| d < bd) {
                nearest = Some((d, o.id));
            }
        }
        let nearest = nearest?.1;
        let Some(cur) = g.target_group else {
            return Some(nearest);
        };
        let c = &self.groups[cur as usize];
        if cur == nearest || c.side == g.side || !c.status.in_battle() || c.alive == 0 {
            return Some(nearest);
        }
        let n = &self.groups[nearest as usize];
        let d_cur = len((c.cx - g.cx) as i64, (c.cy - g.cy) as i64);
        let d_new = len((n.cx - g.cx) as i64, (n.cy - g.cy) as i64);
        Some(if d_new * 3 < d_cur * 2 { nearest } else { cur })
    }

    /// True when most of a group sits inside a nebula on the main layer.
    pub fn hidden(&self, g: &Group) -> bool {
        g.layer != LAYER_LOW && self.cover_at(g.cx, g.cy).1 >= 3
    }

    /// Terrain under a group standing at (x, y): how many of five points
    /// spread over its formation lie in an asteroid field and in a nebula.
    pub fn cover_at(&self, x: i32, y: i32) -> (i64, i64) {
        const SPREAD: i32 = 300 * FP;
        let (mut rocks, mut fog) = (0, 0);
        for (dx, dy) in [(0, 0), (SPREAD, 0), (-SPREAD, 0), (0, SPREAD), (0, -SPREAD)] {
            match self.terrain.at(x + dx, y + dy) {
                terrain::ASTEROIDS => rocks += 1,
                terrain::NEBULA => fog += 1,
                _ => {}
            }
        }
        (rocks, fog)
    }

    fn plan_groups(&mut self) {
        // Refresh centroids and derived stats first so every group plans on the same picture.
        for gi in 0..self.groups.len() {
            let (mut sx, mut sy, mut n) = (0i64, 0i64, 0i64);
            let (mut speed, mut turn) = (i32::MAX, u16::MAX);
            let (mut value, mut screen) = (0, 0);
            let (mut sh, mut sh_max) = (0i64, 0i64);
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
                if st.pd.mounts >= 5 {
                    screen += 1;
                }
                sh += self.ships.shield[i].iter().map(|&v| v as i64).sum::<i64>();
                sh_max += (st.shield[0] + st.shield[1] * 2 + st.shield[2]) as i64;
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
            g.shield_pm = if sh_max > 0 { sh * 1000 / sh_max } else { 0 };
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
            let mut phase = g.phase;
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
                GroupStatus::Active if g.fixed => {
                    // Structures only turn their guns.
                    target_group = self.engage_target(&g);
                    if let Some(tid) = target_group {
                        let t = &self.groups[tid as usize];
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
                    target_group = explicit.or_else(|| self.engage_target(&g));
                    let sense = self.sides[g.side as usize].terrain_sense;
                    let mut dr = g.range as i64 * g.doctrine.range_permille() / 1000;
                    if let Some(tid) = target_group {
                        let t = &self.groups[tid as usize];
                        let to_us = atan2((g.cy - t.cy) as i64, (g.cx - t.cx) as i64);
                        let d = len((g.cx - t.cx) as i64, (g.cy - t.cy) as i64);
                        // An enemy hiding in a nebula can only be seen from close by.
                        if sense && g.doctrine != Doctrine::Line && self.hidden(t) {
                            dr = dr.min(terrain::NEBULA_SIGHT as i64 * 9 / 10);
                        }
                        // Guns reach less far up or down to the other layer.
                        if t.layer != g.layer && !t.fixed {
                            dr = dr * CROSS_LAYER_RANGE_PCT / 100;
                        }
                        // Doctrine card: a group fighting on its own pulls back
                        // when worn down, refits out of reach, then returns.
                        // Given orders are followed to the end.
                        let own = order == Order::Engage;
                        let since = self.tick.saturating_sub(g.phase_since);
                        let (sh_min, coh_min) = g.doctrine.disengage_at();
                        let back = t.range.max(g.range) as i64 * 13 / 10 + 400 * FP as i64;
                        let close = dr + 800 * FP as i64;
                        phase = match g.phase {
                            _ if !own => {
                                if d <= close {
                                    Phase::Engage
                                } else {
                                    Phase::Approach
                                }
                            }
                            Phase::Approach if d <= close => Phase::Engage,
                            // Once per battle: a group that has refitted fights on.
                            Phase::Engage
                                if (g.shield_pm < sh_min || g.cohesion < coh_min)
                                    && since >= 20 * TICK_HZ
                                    && !g.refitted =>
                            {
                                Phase::Disengage
                            }
                            Phase::Engage if d > dr + 2500 * FP as i64 => Phase::Approach,
                            Phase::Disengage
                                if d >= back * 9 / 10 || since >= DISENGAGE_MAX_TICKS =>
                            {
                                Phase::Refit
                            }
                            Phase::Refit
                                if g.shield_pm >= REFIT_SHIELDS || since >= REFIT_MAX_TICKS =>
                            {
                                Phase::Approach
                            }
                            // Pressed while refitting: turn and fight.
                            Phase::Refit if d < t.range as i64 => Phase::Engage,
                            p => p,
                        };
                        match order {
                            Order::Engage | Order::Attack { .. }
                                if matches!(phase, Phase::Disengage | Phase::Refit) =>
                            {
                                let (ox, oy) = polar(back, to_us);
                                goal_x = t.cx + ox as i32;
                                goal_y = t.cy + oy as i32;
                                want = to_us.wrapping_add(ANG_HALF);
                            }
                            Order::Engage | Order::Attack { .. } => {
                                // Stand where the line of fire is clear.
                                let dir = self.clear_bearing(t.cx, t.cy, dr, to_us, g.cx, g.cy);
                                let (ox, oy) = polar(dr, dir);
                                goal_x = t.cx + ox as i32;
                                goal_y = t.cy + oy as i32;
                                want = dir.wrapping_add(ANG_HALF);
                                // Gunships stop and deploy once the target is in reach.
                                if g.deploys
                                    && d <= g.range as i64
                                    && self.terrain.clear_line((g.cx, g.cy), (t.cx, t.cy))
                                {
                                    goal_x = g.anchor_x;
                                    goal_y = g.anchor_y;
                                    want = to_us.wrapping_add(ANG_HALF);
                                }
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
                                    let step = if turn_left > 0 {
                                        10923u16
                                    } else {
                                        0u16.wrapping_sub(10923)
                                    };
                                    (d.max(dr * 13 / 10), to_us.wrapping_add(step))
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
                    // Get out from under a marked barrage.
                    for st in &self.strikes {
                        if st.side == g.side || st.land <= self.tick {
                            continue;
                        }
                        let (ex, ey) = ((g.cx - st.x) as i64, (g.cy - st.y) as i64);
                        if len(ex, ey) < (BARRAGE_RADIUS + 400 * FP) as i64 {
                            let dir = if ex == 0 && ey == 0 {
                                g.facing.wrapping_add(ANG_QUARTER)
                            } else {
                                atan2(ey, ex)
                            };
                            let (ox, oy) = polar(BARRAGE_RADIUS as i64 * 2, dir);
                            goal_x = g.cx + ox as i32;
                            goal_y = g.cy + oy as i32;
                        }
                    }
                }
            }
            let g = &mut self.groups[gi];
            g.goal_x = goal_x;
            g.goal_y = goal_y;
            g.want_facing = want;
            g.target_group = target_group;
            g.order = order;
            if phase != g.phase {
                g.refitted |= phase == Phase::Disengage;
                g.phase = phase;
                g.phase_since = self.tick;
                self.events
                    .push(Event::new(self.tick, ev::PHASE, gi as u32, 0, phase as i32));
            }
        }
    }

    /// Bearing from (tx, ty) to stand on at distance `r` with a clear line
    /// of fire to the target: `dir` if that works, else the clear spot up to
    /// 80 degrees round that is nearest the group at (gx, gy). Nearest, so
    /// two groups on either side of a planet meet instead of circling it.
    fn clear_bearing(&self, tx: i32, ty: i32, r: i64, dir: u16, gx: i32, gy: i32) -> u16 {
        if !self.terrain.has_rocks() {
            return dir;
        }
        const STEP: u16 = 3641; // 20 degrees
                                // Distance to walk there; a spot behind a planet counts as far.
        let clear = |a: u16| {
            let (ox, oy) = polar(r, a);
            let spot = (tx + ox as i32, ty + oy as i32);
            let walk = len((spot.0 - gx) as i64, (spot.1 - gy) as i64)
                + if self.terrain.passable((gx, gy), spot) {
                    0
                } else {
                    3000 * FP as i64
                };
            (!self.terrain.solid(spot.0, spot.1) && self.terrain.clear_line(spot, (tx, ty)))
                .then_some(walk)
        };
        if clear(dir).is_some() {
            return dir;
        }
        let mut best: Option<(i64, u16)> = None;
        for k in 1..=4u16 {
            for a in [dir.wrapping_add(STEP * k), dir.wrapping_sub(STEP * k)] {
                if let Some(d) = clear(a) {
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, a));
                    }
                }
            }
        }
        best.map_or(dir, |b| b.1)
    }

    fn move_anchors(&mut self) {
        let tick = self.tick;
        let terrain = &self.terrain;
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
            if d > 0 && v > 0 {
                let step = v.min(d);
                // Steer round planets: look ahead along the way, and if it
                // runs into rock turn by 30-degree steps to one side (kept
                // until the way is clear, so the group does not dither).
                let dir = atan2(dy, dx);
                let ahead = |a: u16| {
                    [250, 500].iter().all(|&k| {
                        let (ox, oy) = polar(k * FP as i64, a);
                        !terrain.solid(g.anchor_x + ox as i32, g.anchor_y + oy as i32)
                    })
                };
                let goal_blocked = d < 1200 * FP as i64 && terrain.solid(g.goal_x, g.goal_y);
                let mut go = Some(dir);
                if terrain.has_rocks() && !goal_blocked && !ahead(dir) {
                    const STEP: u16 = 5461; // 30 degrees
                    if g.detour == 0 {
                        g.detour =
                            if ahead(dir.wrapping_add(STEP)) || !ahead(dir.wrapping_sub(STEP)) {
                                1
                            } else {
                                -1
                            };
                    }
                    go = (1..=5u16)
                        .map(|k| {
                            if g.detour > 0 {
                                dir.wrapping_add(STEP * k)
                            } else {
                                dir.wrapping_sub(STEP * k)
                            }
                        })
                        .find(|&a| ahead(a));
                } else {
                    g.detour = 0;
                }
                if let Some(a) = go {
                    let (sx, sy) = polar(step, a);
                    let (nx, ny) = (g.anchor_x + sx as i32, g.anchor_y + sy as i32);
                    if !terrain.solid(nx, ny) {
                        g.anchor_x = nx;
                        g.anchor_y = ny;
                    }
                }
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
            still: s.still[i],
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
        let layer = s.layer[i];
        max_speed = max_speed * self.terrain.speed_pct(s.x[i], s.y[i], layer) / 100;
        if self.tick < g.boost_until && st.signature == Signature::Afterburn {
            max_speed *= 2;
            accel *= 2;
        }
        let (x, y) = (s.x[i] as i64, s.y[i] as i64);
        // Ships inside a nebula can only be seen from close by.
        // A target must be seen (ships inside a nebula only from close by) and
        // not behind an asteroid field.
        let visible = |j: usize, d: i64| {
            (d <= terrain::NEBULA_SIGHT as i64 || !self.terrain.hides(s.x[j], s.y[j], s.layer[j]))
                && self.terrain.clear_line((s.x[i], s.y[i]), (s.x[j], s.y[j]))
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
        if st.fixed() {
            (it.vx, it.vy) = (0, 0);
        }
        it.x = s.x[i] + it.vx;
        it.y = s.y[i] + it.vy;
        // Planets are solid: slide along them, or stop.
        let solid = |x, y| self.terrain.solid(x, y);
        if solid(it.x, it.y) && !solid(s.x[i], s.y[i]) {
            if !solid(it.x, s.y[i]) {
                (it.y, it.vy) = (s.y[i], 0);
            } else if !solid(s.x[i], it.y) {
                (it.x, it.vx) = (s.x[i], 0);
            } else {
                (it.x, it.y, it.vx, it.vy) = (s.x[i], s.y[i], 0, 0);
            }
        }
        // Gunships deploy by holding still.
        let slow =
            len(it.vx as i64, it.vy as i64) * 100 <= (st.max_speed * DEPLOY_STILL_PCT) as i64;
        it.still = if slow {
            s.still[i].saturating_add(1)
        } else {
            0
        };

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
            let mut range = if salvo {
                w.range as i64 * 13 / 10
            } else {
                w.range as i64
            };
            // Shooting up or down at the other layer: shorter reach, worse
            // aim, but the shot hits a side shield. Structures sit on both.
            let lt = s.layer[t];
            let vertical = layer & LAYER_MOVING == 0
                && lt & LAYER_MOVING == 0
                && layer != lt
                && !st.fixed()
                && !s.stats(t).fixed();
            if vertical {
                range = range * CROSS_LAYER_RANGE_PCT / 100;
            }
            let deployed = it.still >= st.deploy;
            if it.cd_primary == 0
                && deployed
                && tdist <= range
                && angle_diff(tbear, it.heading).abs() <= w.arc_half as i32
            {
                let mut acc = w.accuracy;
                if salvo {
                    acc += 150;
                }
                if vertical {
                    acc -= CROSS_LAYER_ACCURACY;
                }
                acc -= self.terrain.cover(s.x[t], s.y[t], lt);
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
                        vertical,
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
        // Autocannons: each mount shoots at its own enemy missile in range,
        // then at fighter wings; spare mounts double up on the same targets.
        if it.cd_pd > 0 {
            it.cd_pd -= 1;
        } else {
            let pd = st.pd;
            let mounts = pd.mounts as usize;
            let mut shots: [(u32, bool); 8] = [(0, false); 8];
            let mut n = 0;
            if !self.missiles.is_empty() {
                let ms = &self.missiles;
                self.missile_grid.query(s.x[i], s.y[i], pd.range, |k| {
                    let ku = k as usize;
                    if ms.alive[ku]
                        && ms.side[ku] != s.side[i]
                        && len(ms.x[ku] as i64 - x, ms.y[ku] as i64 - y) <= pd.range as i64
                    {
                        shots[n] = (k, false);
                        n += 1;
                    }
                    n < mounts
                });
            }
            if n < mounts && !self.wings.is_empty() {
                let ws = &self.wings;
                self.wing_grid.query(s.x[i], s.y[i], pd.range, |w| {
                    let wu = w as usize;
                    if ws.side[wu] != s.side[i]
                        && len(ws.x[wu] as i64 - x, ws.y[wu] as i64 - y) <= pd.range as i64
                    {
                        shots[n] = (w, true);
                        n += 1;
                    }
                    n < mounts
                });
            }
            if n > 0 {
                it.cd_pd = pd.cooldown;
                for m in 0..mounts {
                    let (id, wing) = shots[m % n];
                    let chance = if wing {
                        pd.chance * PD_VS_FIGHTER / 100
                    } else {
                        pd.chance
                    };
                    out.intercepts.push(Intercept {
                        ship: i as u32,
                        id,
                        hit: roll(self.seed, self.tick, i as u32, 1_000_000 + m as u32, chance),
                        wing,
                    });
                }
            }
        }
        out.intents.push(it);
    }

    /// Signature moves whose charge runs out this step take effect now.
    fn fire_signatures(&mut self, launches: &mut Vec<(u32, u32, u8)>) {
        let tick = self.tick;
        // Armed moves start charging once the enemy is in reach.
        for gi in 0..self.groups.len() {
            let g = &self.groups[gi];
            if !g.sig_armed || g.status != GroupStatus::Active {
                continue;
            }
            let d = g.target_group.map_or(i64::MAX, |t| {
                let t = &self.groups[t as usize];
                len((t.cx - g.cx) as i64, (t.cy - g.cy) as i64)
            });
            let has = |s: Signature| g.sig_mask & (1 << s as u8) != 0;
            let reach = |u: i64| d <= u * FP as i64;
            let go = (has(Signature::Salvo) && reach(2600))
                || (has(Signature::Torpedoes) && reach(3400))
                || (has(Signature::Afterburn) && reach(4500))
                || (has(Signature::Scramble) && reach(7000))
                || (has(Signature::Barrage) && reach(5200))
                || (has(Signature::ShieldBoost) && reach(3000));
            if go {
                let g = &mut self.groups[gi];
                g.sig_armed = false;
                g.sig_fire_at = tick + SIG_CHARGE_TICKS;
                self.events.push(Event::new(
                    tick,
                    ev::SIGNATURE_CHARGING,
                    gi as u32,
                    0,
                    SIG_CHARGE_TICKS as i32,
                ));
            }
        }
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
            let side = g.side;
            let ships = g.ships.clone();
            let mut boost = false;
            let mut gunships = 0;
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
                    Signature::Barrage => gunships += 1,
                    Signature::ShieldBoost => self.shield_boost(i),
                    Signature::Nothing => {}
                }
            }
            // One marked barrage per group, on the target group's centre.
            if gunships > 0 {
                if let Some(tg) = target_group {
                    let t = &self.groups[tg as usize];
                    let id = self.strikes.len() as u32;
                    self.strikes.push(Strike {
                        x: t.cx,
                        y: t.cy,
                        side,
                        land: tick + BARRAGE_DELAY,
                        damage: BARRAGE_DAMAGE * gunships,
                        src: ships[0],
                    });
                    let mut e =
                        Event::new(tick, ev::BARRAGE_MARKED, id, (t.cx / FP) as u32, t.cy / FP);
                    e.d = side;
                    self.events.push(e);
                }
            }
            if boost {
                self.groups[gi].boost_until = tick + AFTERBURN_TICKS;
            }
            self.events
                .push(Event::new(tick, ev::SIGNATURE_FIRED, gi as u32, 0, 0));
        }
    }

    /// Support signature: every friendly ship nearby gets half its shields
    /// back and a working shield generator.
    fn shield_boost(&mut self, i: usize) {
        let s = &mut self.ships;
        let (x, y, side) = (s.x[i] as i64, s.y[i] as i64, s.side[i]);
        let mut n = 0;
        for j in 0..s.len() {
            if s.state[j] != ALIVE || s.side[j] != side {
                continue;
            }
            if len(s.x[j] as i64 - x, s.y[j] as i64 - y) > SHIELD_BOOST_RANGE as i64 {
                continue;
            }
            let st = ShipClass::from_u8(s.class[j]).stats();
            let max = [st.shield[0], st.shield[1], st.shield[2], st.shield[1]];
            for q in 0..4 {
                s.shield[j][q] = (s.shield[j][q] + max[q] / 2).min(max[q]);
            }
            s.parts[j] &= !(Part::ShieldGen as u8);
            s.overload[j] = 0;
            n += 1;
        }
        self.events
            .push(Event::new(self.tick, ev::SHIELD_BOOST, i as u32, 0, n));
    }

    /// Barrages whose time has come land now.
    fn land_strikes(&mut self, damages: &mut Vec<Damage>) {
        for k in 0..self.strikes.len() {
            let st = self.strikes[k];
            if st.land != self.tick {
                continue;
            }
            let s = &self.ships;
            let mut hit = 0;
            self.grid.query(st.x, st.y, BARRAGE_RADIUS, |j| {
                let ju = j as usize;
                if s.side[ju] != st.side
                    && s.state[ju] == ALIVE
                    && len((s.x[ju] - st.x) as i64, (s.y[ju] - st.y) as i64)
                        <= BARRAGE_RADIUS as i64
                {
                    damages.push(Damage {
                        target: j,
                        src: st.src,
                        amount: st.damage,
                        dtype: DamageType::Kinetic,
                        from_x: st.x,
                        from_y: st.y,
                        vertical: true,
                    });
                    hit += 1;
                }
                true
            });
            self.events
                .push(Event::new(self.tick, ev::BARRAGE_HIT, k as u32, 0, hit));
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
                    // One wave at a time: the first wave in the hangar that is
                    // armed goes out together, once no other wave is flying.
                    // The wave's first surviving wing decides for the wave.
                    let slot = self.wings.slot[w] as usize;
                    let first = w - slot;
                    let hangar = self.ships.stats(c).hangar as usize;
                    let lead = first + slot / WING_WAVE as usize * WING_WAVE as usize;
                    let wave = lead..(lead + WING_WAVE as usize).min(first + hangar);
                    let ws = &self.wings;
                    let leads = wave.clone().find(|&o| ws.state[o] != W_LOST) == Some(w);
                    if group_fighting && enemy_near && leads {
                        // Another wave still out fighting holds this one back;
                        // one flying home does not.
                        let busy = (first..first + hangar).any(|o| ws.state[o] == W_OUT);
                        let ready = wave.clone().all(|o| {
                            ws.state[o] == W_LOST
                                || (ws.state[o] == W_DOCKED && ws.count[o] >= WING_SIZE - 2)
                        }) && wave.clone().any(|o| ws.state[o] == W_DOCKED);
                        if !busy && ready {
                            for o in wave {
                                if self.wings.state[o] == W_DOCKED {
                                    self.launch_wing(o);
                                }
                            }
                        }
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
        if d <= reach as i64 || d == 0 {
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
        let t_ok = t != NONE && self.ships.state[t as usize] == ALIVE && {
            let tu = t as usize;
            let s = &self.ships;
            !self.terrain.hides(s.x[tu], s.y[tu], s.layer[tu])
        };
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
        // Circle the target at strafing distance instead of sitting on top of it.
        let (dx, dy) = ((self.wings.x[w] - tx) as i64, (self.wings.y[w] - ty) as i64);
        let near = len(dx, dy) <= WING_STRIKE_RANGE as i64 * 3 / 2;
        let (ax, ay) = if near {
            let around = atan2(dy, dx).wrapping_add(WING_ORBIT_STEP);
            let (ox, oy) = polar(WING_ORBIT_RADIUS as i64, around);
            (tx + ox as i32, ty + oy as i32)
        } else {
            (tx, ty)
        };
        self.fly(w, ax, ay, 0);
        let in_range = len((self.wings.x[w] - tx) as i64, (self.wings.y[w] - ty) as i64)
            <= WING_STRIKE_RANGE as i64;
        if in_range && self.wings.cd[w] == 0 {
            self.wings.cd[w] = WING_PASS_TICKS;
            let amount = self.wings.count[w] as i32 * FIGHTER_DAMAGE;
            damages.push(Damage {
                target: t,
                src: carrier as u32,
                amount,
                dtype: DamageType::Missile,
                from_x: self.wings.x[w],
                from_y: self.wings.y[w],
                vertical: false,
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
        // Fighters cannot find ships inside a nebula at all.
        let visible = |j: usize, _d: i64| !self.terrain.hides(s.x[j], s.y[j], s.layer[j]);
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
                    && !g.fixed
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
            let here = self.terrain.at(ms.x[k], ms.y[k]);
            if here == terrain::PLANET
                || here == terrain::ASTEROIDS
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
                    vertical: false,
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
        let q = if d.vertical {
            // From above or below: a side shield takes it.
            if rel >= 0 {
                1
            } else {
                3
            }
        } else if rel.abs() <= 8192 {
            0
        } else if rel.abs() >= 24576 {
            2
        } else if rel > 0 {
            1
        } else {
            3
        };
        let mut raw = d.amount as i64;
        if d.dtype == DamageType::Beam && self.terrain.hides(s.x[t], s.y[t], s.layer[t]) {
            raw /= 2;
        }
        let shields_up = s.overload[t] == 0 && s.parts[t] & Part::ShieldGen as u8 == 0;
        if shields_up && s.shield[t][q] > 0 {
            let mut mult = d.dtype.vs_shield();
            // Shields are weak while climbing or diving.
            if s.layer[t] & LAYER_MOVING != 0 {
                mult = mult * 3 / 2;
            }
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
        self.groups[tg].last_hit = self.tick;
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
        self.last_kill = self.tick;
        let g = &mut self.groups[tg];
        g.alive = g.alive.saturating_sub(1);
        g.cohesion -= class.stats().cost * COHESION_MAX * 12 / 10 / g.init_cost;
        self.events
            .push(Event::new(self.tick, ev::SHIP_KILLED, t as u32, killer, 0));
        // Big wrecks leave debris on the low layer.
        let (x, y) = (self.ships.x[t], self.ships.y[t]);
        if class.stats().cost >= 5 {
            if let Some(c) = self.terrain.index(x, y) {
                if self.terrain.cells[c] == terrain::EMPTY {
                    self.terrain.cells[c] = terrain::DEBRIS;
                    self.events
                        .push(Event::new(self.tick, ev::DEBRIS, c as u32, 0, 0));
                }
            }
        }
        // A side's energy field goes down with its last pylon.
        if class == ShipClass::Pylon {
            let s = &self.ships;
            let more = (0..s.len()).any(|j| {
                s.side[j] == side && s.state[j] == ALIVE && s.class[j] == ShipClass::Pylon as u8
            });
            if !more {
                self.terrain
                    .replace(terrain::field_of(side), terrain::EMPTY);
                self.events.push(Event::new(
                    self.tick,
                    ev::FIELD_LOST,
                    t as u32,
                    side as u32,
                    0,
                ));
            }
        }
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
            // Energy fields: three times the recharge for their own side,
            // none for the enemy.
            let here = self.terrain.at(s.x[i], s.y[i]);
            let regen = if here == terrain::field_of(s.side[i]) {
                st.shield_regen * 3
            } else if here == terrain::FIELD0 || here == terrain::FIELD1 {
                0
            } else {
                st.shield_regen
            };
            if s.shield_delay[i] > 0 {
                s.shield_delay[i] -= 1;
            } else if s.overload[i] == 0
                && s.parts[i] & Part::ShieldGen as u8 == 0
                && !self.terrain.hides(s.x[i], s.y[i], s.layer[i])
            {
                let max = [st.shield[0], st.shield[1], st.shield[2], st.shield[1]];
                for q in 0..4 {
                    s.shield[i][q] = (s.shield[i][q] + regen).min(max[q]);
                }
            }
            // Routed ships that reach their own edge get away.
            let g = &self.groups[s.group[i] as usize];
            if g.status == GroupStatus::Routed && (s.x[i].abs() >= FIELD_HALF - 500 * FP) {
                s.state[i] = ESCAPED;
            }
        }
    }

    /// Once a second, every support ship patches up the most damaged
    /// friendly ship near it (itself included).
    fn repair(&mut self) {
        if !self.tick.is_multiple_of(TICK_HZ) {
            return;
        }
        let s = &self.ships;
        let mut fixes = Vec::new();
        for i in 0..s.len() {
            let st = s.stats(i);
            if st.repair == 0 || s.state[i] != ALIVE {
                continue;
            }
            let mut best = (1000i64, NONE);
            self.grid.query(s.x[i], s.y[i], REPAIR_RANGE, |j| {
                let ju = j as usize;
                let sj = s.stats(ju);
                if s.side[ju] == s.side[i]
                    && s.state[ju] == ALIVE
                    && !sj.fixed()
                    && len((s.x[ju] - s.x[i]) as i64, (s.y[ju] - s.y[i]) as i64)
                        <= REPAIR_RANGE as i64
                {
                    let pm = s.hull[ju] as i64 * 1000 / sj.hull as i64;
                    if pm < best.0 {
                        best = (pm, j);
                    }
                }
                true
            });
            if best.1 != NONE {
                fixes.push((best.1 as usize, st.repair));
            }
        }
        for (j, hp) in fixes {
            let max = self.ships.stats(j).hull;
            self.ships.hull[j] = (self.ships.hull[j] + hp).min(max);
        }
    }

    /// Total starting value of a side's fleet.
    /// What a side still has on the field: each ship's cost scaled by its
    /// remaining hull. Ships that jumped out count in full.
    pub fn side_strength(&self, side: u8) -> i64 {
        let s = &self.ships;
        let mut total = 0i64;
        for i in 0..s.len() {
            if s.side[i] != side || s.state[i] == DEAD {
                continue;
            }
            let st = s.stats(i);
            total += if s.state[i] == ALIVE {
                st.cost as i64 * s.hull[i] as i64 / st.hull.max(1) as i64
            } else {
                st.cost as i64
            };
        }
        total
    }

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
            // Out of contact while refitting, the crews steady.
            if g.phase == Phase::Refit
                && tick.is_multiple_of(TICK_HZ)
                && tick >= g.last_hit + 3 * TICK_HZ
                && g.cohesion > 0
                && g.cohesion < REFIT_COHESION_CAP
            {
                g.cohesion = (g.cohesion + REFIT_COHESION_PER_S).min(REFIT_COHESION_CAP);
            }
            g.cohesion = g.cohesion.max(0);
            if g.layer_until != NONE && tick + 1 >= g.layer_until {
                g.layer_until = NONE;
                g.layer = if g.layer == LAYER_MAIN {
                    LAYER_LOW
                } else {
                    LAYER_MAIN
                };
                for &id in &g.ships {
                    self.ships.layer[id as usize] = g.layer;
                }
                self.events.push(Event::new(
                    tick,
                    ev::LAYER_CHANGED,
                    gi as u32,
                    0,
                    g.layer as i32,
                ));
            }
            let g = &mut self.groups[gi];
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
                GroupStatus::Active | GroupStatus::Reserve if g.cohesion == 0 && !g.fixed => {
                    g.status = GroupStatus::Routed;
                    self.events
                        .push(Event::new(tick, ev::GROUP_ROUTED, gi as u32, 0, 0));
                }
                _ => {}
            }
        }
    }

    fn check_outcome(&mut self) {
        // Structures alone do not hold the field.
        let fighting = |side: u8| {
            self.groups
                .iter()
                .any(|g| g.side == side && g.status.in_battle() && g.alive > 0 && !g.fixed)
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
            eat(s.layer[i] as i64);
            eat(s.still[i] as i64);
        }
        for g in &self.groups {
            eat(g.cohesion as i64);
            eat(g.anchor_x as i64);
            eat(g.anchor_y as i64);
            eat(g.status.code() as i64);
            eat(g.phase as i64);
            eat(g.layer as i64);
        }
        for st in &self.strikes {
            eat(st.x as i64);
            eat(st.y as i64);
            eat(st.land as i64);
        }
        for &c in &self.terrain.cells {
            eat(c as i64);
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
            eat(g.sig_armed as i64);
            eat(g.sig_ready_at as i64);
        }
        eat(self.tick as i64);
        h
    }
}
