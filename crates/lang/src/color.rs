//! Colours as the runtime animates them: linear-light RGBA (spec 3.1, 7.3).
//!
//! Colours are written and stored as 8-bit sRGB; blending happens in linear light and the result is
//! converted back, rounding half to even, so the snapshot (and its hash) is deterministic.
use std::sync::OnceLock;

fn lut() -> &'static [f64; 256] {
    static L: OnceLock<[f64; 256]> = OnceLock::new();
    L.get_or_init(|| {
        let mut a = [0.0; 256];
        for (i, slot) in a.iter_mut().enumerate() {
            let c = i as f64 / 255.0;
            *slot = if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            };
        }
        a
    })
}

/// 8-bit sRGB (alpha is already linear) to linear-light RGBA in 0..=1.
pub fn to_linear(c: [u8; 4]) -> [f64; 4] {
    let l = lut();
    [
        l[c[0] as usize],
        l[c[1] as usize],
        l[c[2] as usize],
        c[3] as f64 / 255.0,
    ]
}

fn encode(l: f64) -> u8 {
    let l = l.clamp(0.0, 1.0);
    let s = if l <= 0.0031308 {
        12.92 * l
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round_ties_even().clamp(0.0, 255.0) as u8
}

/// Linear-light RGBA back to 8-bit sRGB.
pub fn to_srgb8(l: [f64; 4]) -> [u8; 4] {
    [
        encode(l[0]),
        encode(l[1]),
        encode(l[2]),
        (l[3].clamp(0.0, 1.0) * 255.0).round_ties_even() as u8,
    ]
}

/// `#rrggbb`, or `#rrggbbaa` when not opaque.
pub fn hex(c: [u8; 4]) -> String {
    if c[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
    }
}
