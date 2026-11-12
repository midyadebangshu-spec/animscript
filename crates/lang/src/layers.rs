//! The property layer stack and its fixed fold (spec 7.2, 7.3).
//!
//! Every scalar property has a rest value (layer 0). Everything that wants to change it adds a
//! *contribution*; the result is one value per tick, computed by folding the contributions in a fixed order:
//! `(layer, priority, start tick, id)` ascending, so later items win.

/// How a contribution combines with the running value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendMode {
    /// `v = lerp(v, value, weight)`
    Override,
    /// `v = v + weight * value`
    Add,
    /// `v = v * lerp(1, value, weight)`
    Multiply,
}

/// Layers: 1 semantic actions, 2 procedural modifiers (reserved), 3 keyframes, 4 locks (reserved).
#[derive(Clone, Debug)]
pub struct Contribution {
    pub layer: u8,
    pub mode: BlendMode,
    pub weight: f64,
    pub priority: i64,
    pub start: i64,
    pub id: u32,
    pub value: f64,
}

/// Resolve one property component from its rest value and its contributions.
pub fn fold(rest: f64, contributions: &mut [Contribution]) -> f64 {
    contributions.sort_by_key(|c| (c.layer, c.priority, c.start, c.id));
    let mut v = rest;
    for c in contributions.iter() {
        match c.mode {
            BlendMode::Override => v += (c.value - v) * c.weight,
            BlendMode::Add => v += c.weight * c.value,
            BlendMode::Multiply => v *= 1.0 + (c.value - 1.0) * c.weight,
        }
    }
    v
}
