//! Integer math that gives the same answer on every machine.
//!
//! Positions are `i32` fixed point (`FP` per distance unit), angles are `u16`
//! (65536 = one full turn, 0 = +x, counter-clockwise). The sine and arctangent
//! tables are built once from plain IEEE `+ - * / sqrt`, which are exactly
//! rounded everywhere, so no platform `libm` result can leak into the tables.

use std::sync::OnceLock;

/// Fixed-point steps per distance unit.
pub const FP: i32 = 100;

pub const ANG_QUARTER: u16 = 16384;
pub const ANG_HALF: u16 = 32768;

/// Sine/cosine are returned scaled by `TRIG_ONE`.
pub const TRIG_SHIFT: u32 = 14;
pub const TRIG_ONE: i64 = 1 << TRIG_SHIFT;

const SIN_SIZE: usize = 4096; // one entry per 16 angle steps
const ATAN_SIZE: usize = 1024; // ratio resolution for atan2

struct Tables {
    sin: Vec<i32>,
    atan: Vec<u16>,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(build_tables)
}

use std::f64::consts::PI;

fn taylor_sin(x: f64) -> f64 {
    // |x| <= pi/2, 12 terms is far below the table's rounding step.
    let x2 = x * x;
    let mut term = x;
    let mut sum = x;
    let mut n = 1.0;
    for _ in 0..12 {
        term = -term * x2 / ((n + 1.0) * (n + 2.0));
        sum += term;
        n += 2.0;
    }
    sum
}

fn taylor_atan(r: f64) -> f64 {
    // Halve the argument once so the series converges fast: atan(r) = 2 atan(a).
    let a = r / (1.0 + (1.0 + r * r).sqrt());
    let a2 = a * a;
    let mut term = a;
    let mut sum = a;
    let mut k = 1.0;
    for _ in 0..40 {
        term = -term * a2;
        k += 2.0;
        sum += term / k;
    }
    2.0 * sum
}

fn build_tables() -> Tables {
    let quarter = SIN_SIZE / 4;
    let mut q = vec![0i32; quarter + 1];
    for (i, v) in q.iter_mut().enumerate() {
        let x = PI / 2.0 * (i as f64) / (quarter as f64);
        *v = (taylor_sin(x) * TRIG_ONE as f64).round() as i32;
    }
    let mut sin = vec![0i32; SIN_SIZE];
    for i in 0..SIN_SIZE {
        let (k, j) = (i / quarter, i % quarter);
        sin[i] = match k {
            0 => q[j],
            1 => q[quarter - j],
            2 => -q[j],
            _ => -q[quarter - j],
        };
    }
    let mut atan = vec![0u16; ATAN_SIZE + 1];
    for (i, v) in atan.iter_mut().enumerate() {
        let r = i as f64 / ATAN_SIZE as f64;
        *v = (taylor_atan(r) * 65536.0 / (2.0 * PI)).round() as u16;
    }
    Tables { sin, atan }
}

#[inline]
pub fn sin(a: u16) -> i64 {
    tables().sin[(a >> 4) as usize] as i64
}

#[inline]
pub fn cos(a: u16) -> i64 {
    sin(a.wrapping_add(ANG_QUARTER))
}

/// Angle of the vector (dx, dy).
pub fn atan2(dy: i64, dx: i64) -> u16 {
    if dx == 0 && dy == 0 {
        return 0;
    }
    let t = &tables().atan;
    let (ax, ay) = (dx.abs(), dy.abs());
    let base: u16 = if ax >= ay {
        t[(ay * ATAN_SIZE as i64 / ax) as usize]
    } else {
        ANG_QUARTER - t[(ax * ATAN_SIZE as i64 / ay) as usize]
    };
    match (dx >= 0, dy >= 0) {
        (true, true) => base,
        (false, true) => ANG_HALF.wrapping_sub(base),
        (false, false) => ANG_HALF.wrapping_add(base),
        (true, false) => 0u16.wrapping_sub(base),
    }
}

/// Signed difference `a - b` in (-32768, 32767].
#[inline]
pub fn angle_diff(a: u16, b: u16) -> i32 {
    a.wrapping_sub(b) as i16 as i32
}

/// Turn `from` toward `to` by at most `rate`.
#[inline]
pub fn turn_toward(from: u16, to: u16, rate: u16) -> u16 {
    let d = angle_diff(to, from);
    let r = rate as i32;
    if d.abs() <= r {
        to
    } else if d > 0 {
        from.wrapping_add(rate)
    } else {
        from.wrapping_sub(rate)
    }
}

/// Rotate (dx, dy) by angle `a`.
#[inline]
pub fn rotate(dx: i64, dy: i64, a: u16) -> (i64, i64) {
    let (c, s) = (cos(a), sin(a));
    (
        (dx * c - dy * s) >> TRIG_SHIFT,
        (dx * s + dy * c) >> TRIG_SHIFT,
    )
}

/// Unit vector of angle `a` scaled to `len`.
#[inline]
pub fn polar(len: i64, a: u16) -> (i64, i64) {
    ((len * cos(a)) >> TRIG_SHIFT, (len * sin(a)) >> TRIG_SHIFT)
}

#[inline]
pub fn isqrt(v: i64) -> i64 {
    (v.max(0) as u64).isqrt() as i64
}

#[inline]
pub fn len(dx: i64, dy: i64) -> i64 {
    isqrt(dx * dx + dy * dy)
}

/// Stateless random numbers: the same (seed, tick, id, salt) always gives the
/// same value, no matter which thread asks or in what order.
#[inline]
pub fn hash(seed: u64, tick: u32, id: u32, salt: u32) -> u64 {
    let mut z = seed
        ^ (tick as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ ((id as u64) << 20).wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ (salt as u64).wrapping_mul(0x94D0_49BB_1331_11EB);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// True with probability `permille` / 1000.
#[inline]
pub fn roll(seed: u64, tick: u32, id: u32, salt: u32, permille: i32) -> bool {
    (hash(seed, tick, id, salt) % 1000) < permille.clamp(0, 1000) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trig_is_sane() {
        assert_eq!(sin(0), 0);
        assert_eq!(sin(ANG_QUARTER), TRIG_ONE);
        assert_eq!(cos(ANG_HALF), -TRIG_ONE);
        assert_eq!(atan2(0, 10), 0);
        assert_eq!(atan2(10, 0), ANG_QUARTER);
        assert_eq!(atan2(0, -10), ANG_HALF);
        assert_eq!(atan2(-10, 0), 3 * ANG_QUARTER);
        assert_eq!(atan2(10, 10), 8192);
        for a in (0..65536u32).step_by(997) {
            let (x, y) = polar(100_000, a as u16);
            let back = atan2(y, x);
            assert!(
                angle_diff(back, a as u16).abs() <= 24,
                "angle {a} came back as {back}"
            );
        }
    }
}
