//! battlecore: headless, deterministic fleet-battle simulation core.
//!
//! The player commands battle groups; ships are light simulated objects;
//! fighters, gunfire and debris are left to the renderer. The simulation runs
//! at a fixed 20 steps per second in integer math and pauses every pulse
//! (15 s) so the player can spend command points.
//!
//! See `README.md` for the design and how to run it.

pub mod ai;
pub mod battle;
pub mod ffi;
pub mod fixed;
pub mod grid;
pub mod replay;
pub mod scenario;
pub mod types;

pub use battle::{Battle, Group, GroupStatus, Outcome};
pub use types::*;
