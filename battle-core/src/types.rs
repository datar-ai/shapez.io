//! Ship classes, doctrines, orders, commands and events.

use crate::fixed::FP;

/// Simulation steps per second.
pub const TICK_HZ: u32 = 20;
/// One pulse = 60 s; the battle pauses at every pulse boundary.
pub const PULSE_TICKS: u32 = 60 * TICK_HZ;
/// A retreating group charges its jump drives this long, unable to fire.
pub const RETREAT_TICKS: u32 = 15 * TICK_HZ;
/// Command points each side receives at the start of every pulse.
pub const COMMAND_POINTS: i32 = 3;
/// Battle groups re-plan twice a second.
pub const GROUP_THINK_TICKS: u32 = 10;
/// A battle that is still undecided after this many pulses is a draw.
pub const MAX_PULSES: u32 = 10;
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
    Carrier = 4,
    /// Long-range gunship: must stand still to deploy before its guns fire.
    Artillery = 5,
    /// Support ship: stays behind its group, repairs and projects shields.
    Support = 6,
    /// Space station: a fixed fort with its own guns (terrain with teeth).
    Station = 7,
    /// Jump beacon (faction terrain, a point): the side's reserve jumps in
    /// here, and groups near it jump out faster.
    Beacon = 8,
    /// Field pylon (faction terrain, an area): powers an energy field that
    /// recharges its own side's shields and stops the enemy's.
    Pylon = 9,
}

impl ShipClass {
    pub fn from_u8(v: u8) -> ShipClass {
        match v {
            0 => ShipClass::Battleship,
            1 => ShipClass::Cruiser,
            2 => ShipClass::Destroyer,
            3 => ShipClass::Flagship,
            4 => ShipClass::Carrier,
            5 => ShipClass::Artillery,
            6 => ShipClass::Support,
            7 => ShipClass::Station,
            8 => ShipClass::Beacon,
            _ => ShipClass::Pylon,
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
            ShipClass::Carrier => "carrier",
            ShipClass::Artillery => "artillery",
            ShipClass::Support => "support",
            ShipClass::Station => "station",
            ShipClass::Beacon => "beacon",
            ShipClass::Pylon => "pylon",
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
    /// Autocannon mounts. Every class carries the same mount; they differ
    /// only in how many. Each mount fires on its own at a missile or fighter.
    pub mounts: u8,
    pub range: i32,
    pub cooldown: u16,
    pub chance: i32, // permille per shot
}

/// Builds a class's autocannon battery from the shared mount.
pub const fn autocannons(mounts: u8) -> PointDefense {
    PointDefense {
        mounts,
        range: 600 * FP,
        cooldown: 10,
        chance: 250,
    }
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
    /// Fighter wings carried (carriers only), in waves of `WING_WAVE`.
    pub hangar: u8,
    /// What this class does when its group uses its signature move.
    pub signature: Signature,
    /// Ticks a ship must hold still before its main gun can fire (0: none).
    pub deploy: u16,
    /// Hull repaired each second on the most damaged friendly ship nearby.
    pub repair: i32,
}

impl ClassStats {
    /// Structures never move and are not commanded.
    pub fn fixed(&self) -> bool {
        self.max_speed == 0
    }
}

/// Signature moves: one per class, bought with a command point. A bought move
/// waits until the enemy is in reach, is announced to both sides while it
/// charges, then cools down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Signature {
    /// Battleships and flagships: the next main-gun shot hits three times as
    /// hard and reaches further, but the guns stay silent while charging.
    Salvo = 0,
    /// Cruisers: two heavy torpedoes each. Slow, need several point-defense hits.
    Torpedoes = 1,
    /// Destroyers: double speed for ten seconds.
    Afterburn = 2,
    /// Carriers: every wing rearmed and launched at once.
    Scramble = 3,
    /// Long-range gunships: a barrage on a marked spot that lands a few
    /// seconds later. The mark is shown to both sides, so it can be dodged.
    Barrage = 4,
    /// Support ships: give every friendly ship nearby half its shields back
    /// and fix their shield generators.
    ShieldBoost = 5,
    /// Structures have none.
    Nothing = 6,
}

/// Ticks between buying a signature move and its effect.
pub const SIG_CHARGE_TICKS: u32 = 3 * TICK_HZ;
/// Ticks before a group can use its signature move again.
pub const SIG_COOLDOWN_TICKS: u32 = 30 * TICK_HZ;
/// Afterburn duration.
pub const AFTERBURN_TICKS: u32 = 10 * TICK_HZ;

/// Fighter wings (the fighter height layer). Only point defense and other
/// fighters can hit them.
pub const WING_SIZE: u8 = 10;
/// Wings per launch wave. A carrier holds three waves; it sends one wave at a
/// time on its own, or all of them with its signature move.
pub const WING_WAVE: u8 = 2;
pub const WING_SPEED: i32 = ups(400);
/// Point defense is less accurate against nimble fighters (percent of its missile chance).
pub const PD_VS_FIGHTER: i32 = 25;
/// Fighters strafe a ship from this close.
pub const WING_STRIKE_RANGE: i32 = 450 * FP;
/// Wings circle their target at this distance...
pub const WING_ORBIT_RADIUS: i32 = 350 * FP;
/// ...moving this far around it each tick (about 8 degrees).
pub const WING_ORBIT_STEP: u16 = 1500;
/// Wings dogfight other wings from this close.
pub const WING_DOGFIGHT_RANGE: i32 = 300 * FP;
/// Damage per fighter per strafing pass, and ticks between passes.
pub const FIGHTER_DAMAGE: i32 = 45;
pub const WING_PASS_TICKS: u16 = 20;
/// Chance (permille) per fighter per dogfight round to down an enemy fighter.
pub const DOGFIGHT_CHANCE: i32 = 90;
pub const DOGFIGHT_TICKS: u16 = 10;
/// A docked wing gains one fighter every this many ticks.
pub const REARM_TICKS: u16 = 40;
/// A wing heads home when it is down to this many fighters.
pub const WING_RETURN_AT: u8 = 3;

/// Barrage: radius of the marked circle, delay before it lands, and damage
/// per gunship to every enemy ship inside.
pub const BARRAGE_RADIUS: i32 = 600 * FP;
pub const BARRAGE_DELAY: u32 = 5 * TICK_HZ;
pub const BARRAGE_DAMAGE: i32 = 700;
/// Support ships repair ships within this distance, and their shield boost
/// reaches this far.
pub const REPAIR_RANGE: i32 = 900 * FP;
pub const SHIELD_BOOST_RANGE: i32 = 1500 * FP;
/// Artillery counts as holding still below this share (percent) of its top speed.
pub const DEPLOY_STILL_PCT: i32 = 20;

/// Height layers. Ships fight on the main layer or drop to the low (orbital
/// and terrain) layer; fighter wings fly on their own layer above both.
pub const LAYER_LOW: u8 = 0;
pub const LAYER_MAIN: u8 = 1;
pub const LAYER_FIGHTER: u8 = 2;
/// Set on a ship's layer while its group climbs or dives: it counts as on
/// both layers and its shields take half again as much.
pub const LAYER_MOVING: u8 = 0x80;
pub const LAYER_CHANGE_TICKS: u32 = 4 * TICK_HZ;
/// Shooting at a ship on the other layer: less accurate and shorter reach,
/// but the shot comes from above or below and hits a side shield.
pub const CROSS_LAYER_ACCURACY: i32 = 150;
pub const CROSS_LAYER_RANGE_PCT: i64 = 85;
/// A group in the jump beacon's reach charges its jump drives this long.
pub const BEACON_RETREAT_TICKS: u32 = 7 * TICK_HZ;
pub const BEACON_REACH: i32 = 2500 * FP;

/// Heavy torpedo from the cruiser signature move.
pub const TORPEDO: MissileRack = MissileRack {
    range: 3600 * FP,
    cooldown: 0,
    damage: 2200,
    speed: ups(75),
};
/// Distance a group with carriers keeps from its target.
pub const CARRIER_STANDOFF: i32 = 3800 * FP;

/// Point-defense hits needed to stop a torpedo.
pub const TORPEDO_HP: u8 = 3;

const fn deg(d: u32) -> u16 {
    (d * 65536 / 360) as u16
}

/// Distance units per second -> fixed point per tick.
const fn ups(v: i32) -> i32 {
    v * FP / TICK_HZ as i32
}

pub static CLASS_STATS: [ClassStats; 10] = [
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
        pd: autocannons(3),
        hangar: 0,
        signature: Signature::Salvo,
        deploy: 0,
        repair: 0,
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
        pd: autocannons(2),
        hangar: 0,
        signature: Signature::Torpedoes,
        deploy: 0,
        repair: 0,
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
            damage: 90,
            accuracy: 700,
            stress: 20,
        },
        missiles: None,
        pd: autocannons(5),
        hangar: 0,
        signature: Signature::Afterburn,
        deploy: 0,
        repair: 0,
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
        pd: autocannons(4),
        hangar: 0,
        signature: Signature::Salvo,
        deploy: 0,
        repair: 0,
    },
    // Carrier: slow, light guns, strong point defense, two fighter wings.
    ClassStats {
        max_speed: ups(35),
        accel: 7,
        turn_rate: deg(15) / TICK_HZ as u16,
        radius: 65 * FP,
        hull: 5000,
        shield: [2200, 1600, 800],
        shield_regen: 9,
        armor: 35,
        max_stress: 2400,
        stress_decay: 10,
        cost: 9,
        primary: Weapon {
            dtype: DamageType::Kinetic,
            range: 1200 * FP,
            arc_half: deg(180),
            cooldown: 20,
            damage: 90,
            accuracy: 700,
            stress: 30,
        },
        missiles: None,
        pd: autocannons(3),
        hangar: 3 * WING_WAVE,
        signature: Signature::Scramble,
        deploy: 0,
        repair: 0,
    },
    // Long-range gunship: very slow, must deploy (hold still 3 s) to fire;
    // outranges everything, but cannot see into a nebula from afar.
    ClassStats {
        max_speed: ups(22),
        accel: 5,
        turn_rate: deg(10) / TICK_HZ as u16,
        radius: 50 * FP,
        hull: 3200,
        shield: [1600, 1100, 500],
        shield_regen: 6,
        armor: 30,
        max_stress: 2400,
        stress_decay: 10,
        cost: 7,
        primary: Weapon {
            dtype: DamageType::Kinetic,
            range: 4800 * FP,
            arc_half: deg(25),
            cooldown: 80,
            damage: 700,
            accuracy: 700,
            stress: 300,
        },
        missiles: None,
        pd: autocannons(2),
        hangar: 0,
        signature: Signature::Barrage,
        deploy: 3 * TICK_HZ as u16,
        repair: 0,
    },
    // Support ship: weak beam, thick shields, keeps to the back of its group
    // and patches up whoever is hurt worst nearby.
    ClassStats {
        max_speed: ups(40),
        accel: 9,
        turn_rate: deg(22) / TICK_HZ as u16,
        radius: 40 * FP,
        hull: 2200,
        shield: [2000, 1500, 900],
        shield_regen: 10,
        armor: 25,
        max_stress: 2000,
        stress_decay: 10,
        cost: 5,
        primary: Weapon {
            dtype: DamageType::Beam,
            range: 1000 * FP,
            arc_half: deg(90),
            cooldown: 30,
            damage: 120,
            accuracy: 750,
            stress: 40,
        },
        missiles: None,
        pd: autocannons(4),
        hangar: 0,
        signature: Signature::ShieldBoost,
        deploy: 0,
        repair: 40,
    },
    // Space station: fixed fort. Heavy guns all round, a missile battery and
    // the most autocannons of anything.
    ClassStats {
        max_speed: 0,
        accel: 0,
        turn_rate: 0,
        radius: 90 * FP,
        hull: 12000,
        shield: [2500, 2500, 2500],
        shield_regen: 10,
        armor: 50,
        max_stress: 6000,
        stress_decay: 20,
        cost: 12,
        primary: Weapon {
            dtype: DamageType::Kinetic,
            range: 2600 * FP,
            arc_half: deg(180),
            cooldown: 30,
            damage: 380,
            accuracy: 760,
            stress: 100,
        },
        missiles: Some(MissileRack {
            range: 3200 * FP,
            cooldown: 120,
            damage: 520,
            speed: ups(140),
        }),
        pd: autocannons(8),
        hangar: 0,
        signature: Signature::Nothing,
        deploy: 0,
        repair: 0,
    },
    // Jump beacon: unarmed.
    ClassStats {
        max_speed: 0,
        accel: 0,
        turn_rate: 0,
        radius: 40 * FP,
        hull: 3500,
        shield: [1500, 1500, 1500],
        shield_regen: 6,
        armor: 30,
        max_stress: 3000,
        stress_decay: 10,
        cost: 4,
        primary: UNARMED,
        missiles: None,
        pd: autocannons(2),
        hangar: 0,
        signature: Signature::Nothing,
        deploy: 0,
        repair: 0,
    },
    // Field pylon: unarmed.
    ClassStats {
        max_speed: 0,
        accel: 0,
        turn_rate: 0,
        radius: 40 * FP,
        hull: 4500,
        shield: [1800, 1800, 1800],
        shield_regen: 6,
        armor: 30,
        max_stress: 3000,
        stress_decay: 10,
        cost: 5,
        primary: UNARMED,
        missiles: None,
        pd: autocannons(2),
        hangar: 0,
        signature: Signature::Nothing,
        deploy: 0,
        repair: 0,
    },
];

/// Main "gun" of an unarmed structure: never in range.
const UNARMED: Weapon = Weapon {
    dtype: DamageType::Kinetic,
    range: 0,
    arc_half: 0,
    cooldown: 20,
    damage: 0,
    accuracy: 0,
    stress: 0,
};

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
    /// When a group fighting on its own pulls back to refit: its shields
    /// (permille of full) or its cohesion falls below these.
    pub fn disengage_at(self) -> (i64, i32) {
        match self {
            // Gun lines save their ships once the shields are gone.
            Doctrine::Line => (200, 350),
            // Anvils hold on: only a shaken anvil lets go.
            Doctrine::Anvil => (0, 300),
            // Hammers hit and run.
            Doctrine::Hammer => (150, 450),
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

/// Where a group fighting on its own (the doctrine default) is in its
/// doctrine card: approach, engage, pull back when worn down, refit, return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
    Approach = 0,
    Engage = 1,
    Disengage = 2,
    Refit = 3,
}

/// A group pulls back to refit at most once per battle. It refits until
/// its shields are back to this
/// (permille of full), or for at most `REFIT_MAX_TICKS`.
pub const REFIT_SHIELDS: i64 = 700;
pub const REFIT_MAX_TICKS: u32 = 25 * TICK_HZ;
/// Pulling back ends after this long even if the enemy follows.
pub const DISENGAGE_MAX_TICKS: u32 = 15 * TICK_HZ;
/// While refitting out of contact, cohesion recovers this much per second,
/// up to `REFIT_COHESION_CAP`.
pub const REFIT_COHESION_PER_S: i32 = 8;
pub const REFIT_COHESION_CAP: i32 = 600;

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
    /// Use the group's signature move (each ship by its own class).
    Signature {
        group: u16,
    },
    /// Climb to the main layer or drop to the low layer.
    ChangeLayer {
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
            | Command::Retreat { group }
            | Command::Signature { group }
            | Command::ChangeLayer { group } => group,
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
    SignatureNotReady,
    /// Jump drives do not work inside a gravity well.
    InGravityWell,
    /// Structures cannot be commanded.
    Fixed,
    /// Already climbing or diving.
    ChangingLayer,
}

/// Event kinds, shared with the C interface.
pub mod ev {
    pub const FIRE: u8 = 1; // a = shooter, b = target, c = 1 if hit, d = damage type
    pub const MISSILE_LAUNCH: u8 = 2; // a = shooter, b = missile id, c = target, d = 1 for a torpedo
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
    pub const SIGNATURE_CHARGING: u8 = 15; // a = group, c = ticks until it fires
    pub const SIGNATURE_FIRED: u8 = 16; // a = group
    pub const WING_LAUNCHED: u8 = 17; // a = wing, b = carrier
    pub const FIGHTER_DOWN: u8 = 18; // a = wing, b = shooter ship, or wing | 1<<31
    pub const WING_STRIKE: u8 = 19; // a = wing, b = target ship, c = damage
    pub const WING_LOST: u8 = 20; // a = wing
    pub const LAYER_CHANGED: u8 = 21; // a = group, c = new layer (d = 1 when it starts moving)
    pub const BARRAGE_MARKED: u8 = 22; // a = strike id, b = x (units, as i32), c = y (units), d = side
    pub const BARRAGE_HIT: u8 = 23; // a = strike id, c = ships hit
    pub const PHASE: u8 = 24; // a = group, c = new phase
    pub const JUMP_IN: u8 = 25; // a = group, b = beacon ship
    pub const DEBRIS: u8 = 26; // a = terrain cell index
    pub const FIELD_LOST: u8 = 27; // a = pylon ship, b = side
    pub const SHIELD_BOOST: u8 = 28; // a = support ship, c = ships boosted
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
