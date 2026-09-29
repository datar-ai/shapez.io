//! Ship classes, doctrines, orders, commands and events.

use crate::fixed::FP;

/// Simulation steps per second.
pub const TICK_HZ: u32 = 20;
/// One pulse = 15 s; the battle pauses at every pulse boundary.
pub const PULSE_TICKS: u32 = 15 * TICK_HZ;
/// Command points each side receives at the start of every pulse.
pub const COMMAND_POINTS: i32 = 3;
/// Battle groups re-plan twice a second.
pub const GROUP_THINK_TICKS: u32 = 10;
/// A battle that is still undecided after this many pulses is a draw.
pub const MAX_PULSES: u32 = 40;
/// Cohesion is kept on a 0..=1000 scale.
pub const COHESION_MAX: i32 = 1000;
/// Half-width of the battlefield; routed groups leave past this line.
pub const FIELD_HALF: i32 = 12_000 * FP;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ShipClass {
    Battleship = 0,
    Cruiser = 1,
    Destroyer = 2,
    Flagship = 3,
}

impl ShipClass {
    pub fn from_u8(v: u8) -> ShipClass {
        match v {
            0 => ShipClass::Battleship,
            1 => ShipClass::Cruiser,
            2 => ShipClass::Destroyer,
            _ => ShipClass::Flagship,
        }
    }
    pub fn stats(self) -> &'static ClassStats {
        &CLASS_STATS[self as usize]
    }
    pub fn name(self) -> &'static str {
        match self {
            ShipClass::Battleship => "battleship",
            ShipClass::Cruiser => "cruiser",
            ShipClass::Destroyer => "destroyer",
            ShipClass::Flagship => "flagship",
        }
    }
}

/// Damage types. Beams are strong against shields, kinetics against armor,
/// missiles hit hard but can be shot down in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DamageType {
    Beam = 0,
    Kinetic = 1,
    Missile = 2,
}

impl DamageType {
    /// Multiplier (percent) applied against shields.
    pub fn vs_shield(self) -> i64 {
        match self {
            DamageType::Beam => 150,
            DamageType::Kinetic => 75,
            DamageType::Missile => 100,
        }
    }
    /// Share (percent) of the target's armor that applies.
    pub fn armor_share(self) -> i64 {
        match self {
            DamageType::Beam => 100,
            DamageType::Kinetic => 50,
            DamageType::Missile => 100,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Weapon {
    pub dtype: DamageType,
    pub range: i32,    // fixed point
    pub arc_half: u16, // half-angle of the firing arc
    pub cooldown: u16, // ticks
    pub damage: i32,
    pub accuracy: i32, // permille
    pub stress: i32,   // stress added per shot
}

#[derive(Clone, Copy, Debug)]
pub struct MissileRack {
    pub range: i32,
    pub cooldown: u16,
    pub damage: i32,
    pub speed: i32, // fixed point per tick
}

#[derive(Clone, Copy, Debug)]
pub struct PointDefense {
    pub range: i32,
    pub cooldown: u16,
    pub chance: i32, // permille per shot
}

#[derive(Clone, Copy, Debug)]
pub struct ClassStats {
    pub max_speed: i32, // fixed point per tick
    pub accel: i32,
    pub turn_rate: u16, // angle per tick
    pub radius: i32,
    pub hull: i32,
    /// Shields per quadrant: front, side, rear.
    pub shield: [i32; 3],
    pub shield_regen: i32, // per tick, after a short delay
    pub armor: i32,        // percent damage reduction
    pub max_stress: i32,
    pub stress_decay: i32, // per tick
    pub cost: i32,         // used for cohesion loss and value reports
    pub primary: Weapon,
    pub missiles: Option<MissileRack>,
    pub pd: PointDefense,
}

const fn deg(d: u32) -> u16 {
    (d * 65536 / 360) as u16
}

/// Distance units per second -> fixed point per tick.
const fn ups(v: i32) -> i32 {
    v * FP / TICK_HZ as i32
}

pub static CLASS_STATS: [ClassStats; 4] = [
    // Battleship: slow, heavy kinetic broadsides, thick front shield.
    ClassStats {
        max_speed: ups(30),
        accel: 6,
        turn_rate: deg(12) / TICK_HZ as u16,
        radius: 60 * FP,
        hull: 6000,
        shield: [3000, 2000, 900],
        shield_regen: 10,
        armor: 50,
        max_stress: 3000,
        stress_decay: 12,
        cost: 10,
        primary: Weapon {
            dtype: DamageType::Kinetic,
            range: 2400 * FP,
            arc_half: deg(150),
            cooldown: 40,
            damage: 600,
            accuracy: 800,
            stress: 260,
        },
        missiles: None,
        pd: PointDefense {
            range: 400 * FP,
            cooldown: 10,
            chance: 200,
        },
    },
    // Cruiser: beam lance forward, missile rack, middling everything.
    ClassStats {
        max_speed: ups(45),
        accel: 10,
        turn_rate: deg(24) / TICK_HZ as u16,
        radius: 35 * FP,
        hull: 2500,
        shield: [1800, 1200, 500],
        shield_regen: 6,
        armor: 30,
        max_stress: 1600,
        stress_decay: 8,
        cost: 5,
        primary: Weapon {
            dtype: DamageType::Beam,
            range: 1800 * FP,
            arc_half: deg(45),
            cooldown: 30,
            damage: 330,
            accuracy: 780,
            stress: 150,
        },
        missiles: Some(MissileRack {
            range: 3000 * FP,
            cooldown: 100,
            damage: 520,
            speed: ups(140),
        }),
        pd: PointDefense {
            range: 400 * FP,
            cooldown: 10,
            chance: 250,
        },
    },
    // Destroyer: fast, turreted autocannons, strong point defense.
    ClassStats {
        max_speed: ups(70),
        accel: 18,
        turn_rate: deg(45) / TICK_HZ as u16,
        radius: 20 * FP,
        hull: 1000,
        shield: [700, 500, 250],
        shield_regen: 3,
        armor: 15,
        max_stress: 700,
        stress_decay: 5,
        cost: 2,
        primary: Weapon {
            dtype: DamageType::Kinetic,
            range: 1200 * FP,
            arc_half: deg(180),
            cooldown: 10,
            damage: 70,
            accuracy: 700,
            stress: 20,
        },
        missiles: None,
        pd: PointDefense {
            range: 700 * FP,
            cooldown: 4,
            chance: 550,
        },
    },
    // Flagship: a tougher battleship whose loss shakes the whole fleet.
    ClassStats {
        max_speed: ups(30),
        accel: 6,
        turn_rate: deg(12) / TICK_HZ as u16,
        radius: 80 * FP,
        hull: 9000,
        shield: [4000, 2600, 1200],
        shield_regen: 14,
        armor: 55,
        max_stress: 4000,
        stress_decay: 16,
        cost: 15,
        primary: Weapon {
            dtype: DamageType::Kinetic,
            range: 2400 * FP,
            arc_half: deg(150),
            cooldown: 36,
            damage: 650,
            accuracy: 820,
            stress: 260,
        },
        missiles: None,
        pd: PointDefense {
            range: 500 * FP,
            cooldown: 8,
            chance: 250,
        },
    },
];

/// Doctrine card: how a group behaves when nobody is spending points on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Doctrine {
    /// Gun line: hold at the edge of its own range.
    Line = 0,
    /// Anvil: pin the enemy front at medium range.
    Anvil = 1,
    /// Hammer: close in fast, prefers flanks.
    Hammer = 2,
}

impl Doctrine {
    /// Preferred engagement distance as permille of the group's weapon range.
    pub fn range_permille(self) -> i64 {
        match self {
            Doctrine::Line => 900,
            Doctrine::Anvil => 700,
            Doctrine::Hammer => 450,
        }
    }
    pub fn from_u8(v: u8) -> Doctrine {
        match v {
            0 => Doctrine::Line,
            1 => Doctrine::Anvil,
            _ => Doctrine::Hammer,
        }
    }
}

/// Parts that can be knocked out. Stored as bit flags on each ship.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Part {
    Engine = 1,
    Weapons = 2,
    ShieldGen = 4,
    Bridge = 8,
}

impl Part {
    pub fn from_u8(v: u8) -> Option<Part> {
        match v {
            1 => Some(Part::Engine),
            2 => Some(Part::Weapons),
            4 => Some(Part::ShieldGen),
            8 => Some(Part::Bridge),
            _ => None,
        }
    }
}

/// What a group is currently trying to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// Doctrine default: engage the nearest enemy group.
    Engage,
    /// Engage a specific enemy group head on.
    Attack { target: u16 },
    /// Swing around to the side of an enemy group.
    Flank { target: u16, left: bool },
    /// Stay where you are and face the nearest enemy.
    Hold { x: i32, y: i32 },
    /// Move to a point.
    Advance { x: i32, y: i32 },
}

/// Commands the player (or the enemy commander) spends points on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Attack {
        group: u16,
        target: u16,
    },
    Flank {
        group: u16,
        target: u16,
        left: bool,
    },
    Hold {
        group: u16,
    },
    Advance {
        group: u16,
        x: i32,
        y: i32,
    },
    FocusPart {
        group: u16,
        part: Part,
    },
    SetDoctrine {
        group: u16,
        doctrine: Doctrine,
    },
    CommitReserve {
        group: u16,
    },
    /// Jump out. Free, but the group charges for a full pulse and cannot fire.
    Retreat {
        group: u16,
    },
}

impl Command {
    pub fn cost(&self) -> i32 {
        match self {
            Command::CommitReserve { .. } => 2,
            Command::Retreat { .. } => 0,
            _ => 1,
        }
    }
    pub fn group(&self) -> u16 {
        match *self {
            Command::Attack { group, .. }
            | Command::Flank { group, .. }
            | Command::Hold { group }
            | Command::Advance { group, .. }
            | Command::FocusPart { group, .. }
            | Command::SetDoctrine { group, .. }
            | Command::CommitReserve { group }
            | Command::Retreat { group } => group,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandError {
    NoSuchGroup,
    NotYourGroup,
    GroupOutOfAction,
    NotEnoughPoints,
    GroupRetreating,
    NotInReserve,
    BadTarget,
    BattleOver,
}

/// Event kinds, shared with the C interface.
pub mod ev {
    pub const FIRE: u8 = 1; // a = shooter, b = target, c = 1 if hit, d = damage type
    pub const MISSILE_LAUNCH: u8 = 2; // a = shooter, b = missile id, c = target
    pub const MISSILE_INTERCEPTED: u8 = 3; // a = point-defense ship, b = missile id
    pub const MISSILE_HIT: u8 = 4; // a = missile id, b = target
    pub const SHIP_KILLED: u8 = 5; // a = ship, b = killer ship (or u32::MAX)
    pub const PART_DAMAGED: u8 = 6; // a = ship, c = part bit
    pub const OVERLOAD: u8 = 7; // a = ship
    pub const GROUP_ROUTED: u8 = 8; // a = group
    pub const GROUP_ESCAPED: u8 = 9; // a = group, c = ships saved
    pub const RETREAT_STARTED: u8 = 10; // a = group
    pub const COMMAND: u8 = 11; // a = group, b = side, c = command kind
    pub const PULSE: u8 = 12; // c = pulse number
    pub const FLAGSHIP_LOST: u8 = 13; // a = ship, b = side
    pub const BATTLE_OVER: u8 = 14; // c = winner (0, 1, or -1 draw)
}

/// One simulation event. `#[repr(C)]` so the engine can read the list directly.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub tick: u32,
    pub kind: u8,
    pub d: u8,
    pub _pad: u16,
    pub a: u32,
    pub b: u32,
    pub c: i32,
}

impl Event {
    pub fn new(tick: u32, kind: u8, a: u32, b: u32, c: i32) -> Event {
        Event {
            tick,
            kind,
            d: 0,
            _pad: 0,
            a,
            b,
            c,
        }
    }
}
