//! A sculpting engine on signed-distance volumes. A shape is a field `d(x, y, z)`: negative inside, positive outside,
//! the distance to the surface near it. Fields are stored sparsely (8x8x8 blocks, uniform or dense, kept in an ordered
//! map so every walk is in a fixed order) and edited by mathematical shapes and booleans (`sdf`), noise (`noise`),
//! brushes (`brush`) and flows (`flow`). `Volume::mesh` turns the surface into triangles with surface nets. Everything
//! is deterministic: the same operations give the same bits.
#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]
pub mod brush;
pub mod flow;
pub mod noise;
pub mod sdf;
pub mod volume;

pub use volume::{Mesh, Volume};

pub type V3 = [f64; 3];

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn mul(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn len(a: V3) -> f64 {
    dot(a, a).sqrt()
}
pub fn norm(a: V3) -> V3 {
    let l = len(a);
    if l < 1e-300 {
        [0.0, 0.0, 0.0]
    } else {
        mul(a, 1.0 / l)
    }
}
