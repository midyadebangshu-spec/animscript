//! Seeded randomness for procedural modifiers and the `noise` / `random` builtins (spec 6.11).
//!
//! Every stream is named (`"<scene>/<shape>/<target>/<kind>#<n>/<axis>"`), hashed together with the
//! project seed, and read through *pure* functions of (stream, index or time). There is no sequential
//! generator state, so any frame can be computed on its own, in any order, and adding a modifier
//! elsewhere never changes the numbers of an existing one.

pub fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 64-bit FNV-1a of a stream name.
pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The seed of a named stream under a project seed.
pub fn stream_seed(project_seed: i64, name: &str) -> u64 {
    splitmix64(fnv1a(name) ^ splitmix64(project_seed as u64))
}

/// The `i`-th number of a stream, in `[0, 1)`.
pub fn hash01(seed: u64, i: i64) -> f64 {
    let h = splitmix64(seed ^ (i as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93));
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Smooth value noise in `[-1, 1]`: lattice values at whole numbers, blended with a smoothstep.
pub fn value_noise(seed: u64, t: f64) -> f64 {
    let i = t.floor();
    let f = t - i;
    let a = hash01(seed, i as i64) * 2.0 - 1.0;
    let b = hash01(seed, i as i64 + 1) * 2.0 - 1.0;
    let u = f * f * (3.0 - 2.0 * f);
    a + (b - a) * u
}
