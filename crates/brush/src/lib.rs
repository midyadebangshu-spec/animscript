//! The libmypaint brush engine (vendored under `third_party/libmypaint`, ISC licence) behind a small safe API.
//!
//! A [`Brush`] holds libmypaint's settings (loaded from a `.myb` file or set by name); a [`Canvas`] is a fixed size
//! transparent picture; [`Canvas::stroke`] feeds a list of [`Point`]s (position, pressure, tilt, time step) to the
//! brush, which lays down dabs. libmypaint keeps 15-bit fixed point pixels and 32-bit float arithmetic for the dab
//! shapes; the build disables fused multiply-add, so the same stroke gives the same picture on one machine.
//! Create a fresh [`Brush`] to repeat a stroke exactly: its random generator carries on from stroke to stroke.
use std::ffi::{c_char, c_double, c_float, c_int, c_void, CString};

mod ffi {
    use super::*;
    #[repr(C)]
    pub struct MyPaintBrush {
        _p: [u8; 0],
    }
    #[repr(C)]
    pub struct MyPaintSurface {
        _p: [u8; 0],
    }
    #[repr(C)]
    pub struct MyPaintFixedTiledSurface {
        _p: [u8; 0],
    }
    extern "C" {
        pub fn mypaint_brush_new() -> *mut MyPaintBrush;
        pub fn mypaint_brush_unref(b: *mut MyPaintBrush);
        pub fn mypaint_brush_reset(b: *mut MyPaintBrush);
        pub fn mypaint_brush_new_stroke(b: *mut MyPaintBrush);
        pub fn mypaint_brush_from_defaults(b: *mut MyPaintBrush);
        pub fn mypaint_brush_set_base_value(b: *mut MyPaintBrush, id: c_int, v: c_float);
        pub fn mypaint_brush_get_base_value(b: *mut MyPaintBrush, id: c_int) -> c_float;
        pub fn mypaint_brush_set_mapping_n(b: *mut MyPaintBrush, id: c_int, input: c_int, n: c_int);
        pub fn mypaint_brush_set_mapping_point(
            b: *mut MyPaintBrush,
            id: c_int,
            input: c_int,
            i: c_int,
            x: c_float,
            y: c_float,
        );
        pub fn mypaint_brush_setting_from_cname(name: *const c_char) -> c_int;
        pub fn mypaint_brush_input_from_cname(name: *const c_char) -> c_int;
        pub fn mypaint_brush_stroke_to(
            b: *mut MyPaintBrush,
            s: *mut MyPaintSurface,
            x: c_float,
            y: c_float,
            pressure: c_float,
            xtilt: c_float,
            ytilt: c_float,
            dtime: c_double,
            viewzoom: c_float,
            viewrotation: c_float,
            barrel_rotation: c_float,
            linear: c_int,
        ) -> c_int;
        pub fn mypaint_fixed_tiled_surface_new(w: c_int, h: c_int)
            -> *mut MyPaintFixedTiledSurface;
        pub fn mypaint_fixed_tiled_surface_interface(
            s: *mut MyPaintFixedTiledSurface,
        ) -> *mut MyPaintSurface;
        pub fn mypaint_fixed_tiled_surface_clear(s: *mut MyPaintFixedTiledSurface);
        pub fn mypaint_fixed_tiled_surface_read(s: *mut MyPaintFixedTiledSurface, out: *mut u16);
        pub fn mypaint_fixed_tiled_surface_write(
            s: *mut MyPaintFixedTiledSurface,
            input: *const u16,
        );
        pub fn mypaint_tiled_surface_set_symmetry_state(
            s: *mut MyPaintFixedTiledSurface,
            active: c_int,
            cx: c_float,
            cy: c_float,
            angle: c_float,
            kind: c_int,
            lines: c_int,
        );
        pub fn mypaint_surface_begin_atomic(s: *mut MyPaintSurface);
        pub fn mypaint_surface_end_atomic(s: *mut MyPaintSurface, roi: *mut c_void);
        pub fn mypaint_surface_unref(s: *mut MyPaintSurface);
    }
}

/// One sample of a stroke: where the pen is, how hard it presses (0..1), its tilt (-1..1) and the seconds since
/// the previous sample (libmypaint's speed-dependent settings use it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
    pub xtilt: f32,
    pub ytilt: f32,
    pub dt: f64,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Point {
        Point {
            x,
            y,
            pressure: 0.6,
            xtilt: 0.0,
            ytilt: 0.0,
            dt: 0.01,
        }
    }
}

/// How a stroke is repeated around a centre (libmypaint's symmetry): every dab is also painted at the mirrored or
/// rotated places.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SymmetryKind {
    /// Reflection across the vertical line through the centre.
    Vertical,
    /// Reflection across the horizontal line through the centre.
    Horizontal,
    /// Both reflections: four copies.
    Both,
    /// `lines` copies turned around the centre.
    Radial,
    /// `lines` rotated copies, each also reflected.
    Snowflake,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Symmetry {
    pub kind: SymmetryKind,
    pub center: [f32; 2],
    /// The turn of the symmetry lines, in degrees.
    pub angle_deg: f32,
    /// The number of copies for `Radial` and `Snowflake` (at least 2).
    pub lines: i32,
}

/// How a stroke is composited onto the picture (the W3C compositing modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Add,
}

const BLENDS: &[(&str, Blend)] = &[
    ("normal", Blend::Normal),
    ("multiply", Blend::Multiply),
    ("screen", Blend::Screen),
    ("overlay", Blend::Overlay),
    ("darken", Blend::Darken),
    ("lighten", Blend::Lighten),
    ("color_dodge", Blend::ColorDodge),
    ("color_burn", Blend::ColorBurn),
    ("hard_light", Blend::HardLight),
    ("soft_light", Blend::SoftLight),
    ("difference", Blend::Difference),
    ("exclusion", Blend::Exclusion),
    ("add", Blend::Add),
];

impl Blend {
    pub fn from_name(name: &str) -> Option<Blend> {
        BLENDS.iter().find(|b| b.0 == name).map(|b| b.1)
    }
    pub fn name(self) -> &'static str {
        BLENDS.iter().find(|b| b.1 == self).expect("listed").0
    }
    pub fn names() -> Vec<&'static str> {
        BLENDS.iter().map(|b| b.0).collect()
    }

    /// The blend function of one colour channel (backdrop `b`, source `s`, both 0 to 1, not premultiplied).
    fn channel(self, b: f32, s: f32) -> f32 {
        let hard = |b: f32, s: f32| {
            if s <= 0.5 {
                b * 2.0 * s
            } else {
                let t = 2.0 * s - 1.0;
                b + t - b * t
            }
        };
        match self {
            Blend::Normal => s,
            Blend::Multiply => b * s,
            Blend::Screen => b + s - b * s,
            Blend::Overlay => hard(s, b),
            Blend::Darken => b.min(s),
            Blend::Lighten => b.max(s),
            Blend::ColorDodge => {
                if b == 0.0 {
                    0.0
                } else if s >= 1.0 {
                    1.0
                } else {
                    (b / (1.0 - s)).min(1.0)
                }
            }
            Blend::ColorBurn => {
                if b >= 1.0 {
                    1.0
                } else if s <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - b) / s).min(1.0)
                }
            }
            Blend::HardLight => hard(b, s),
            Blend::SoftLight => {
                if s <= 0.5 {
                    b - (1.0 - 2.0 * s) * b * (1.0 - b)
                } else {
                    let d = if b <= 0.25 {
                        ((16.0 * b - 12.0) * b + 4.0) * b
                    } else {
                        b.sqrt()
                    };
                    b + (2.0 * s - 1.0) * (d - b)
                }
            }
            Blend::Difference => (b - s).abs(),
            Blend::Exclusion => b + s - 2.0 * b * s,
            Blend::Add => (b + s).min(1.0),
        }
    }
}

/// Options of one stroke beyond the brush and the points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeOpts {
    /// Pen twist, in turns (libmypaint's `barrel_rotation` input).
    pub barrel: f32,
    pub symmetry: Option<Symmetry>,
    /// How the stroke is composited onto what is already painted.
    pub blend: Blend,
    /// 0 to 1: how much of the stroke is composited (0 leaves the picture as it was).
    pub strength: f32,
}

impl Default for StrokeOpts {
    fn default() -> StrokeOpts {
        StrokeOpts {
            barrel: 0.0,
            symmetry: None,
            blend: Blend::Normal,
            strength: 1.0,
        }
    }
}

/// The brushes that come with the crate (MyPaint's classic brushes, CC0).
const BUNDLED: &[(&str, &str)] = &[
    ("pencil", include_str!("../brushes/pencil.myb")),
    ("pen", include_str!("../brushes/pen.myb")),
    ("marker", include_str!("../brushes/marker_small.myb")),
    ("charcoal", include_str!("../brushes/charcoal.myb")),
    ("calligraphy", include_str!("../brushes/calligraphy.myb")),
    ("ink", include_str!("../brushes/slow_ink.myb")),
    ("smudge", include_str!("../brushes/smudge.myb")),
    ("eraser", include_str!("../brushes/ink_eraser.myb")),
    ("rounded", include_str!("../brushes/rounded.myb")),
];

fn setting_id(name: &str) -> Option<c_int> {
    let c = CString::new(name).ok()?;
    let id = unsafe { ffi::mypaint_brush_setting_from_cname(c.as_ptr()) };
    (id >= 0).then_some(id)
}
fn input_id(name: &str) -> Option<c_int> {
    let c = CString::new(name).ok()?;
    let id = unsafe { ffi::mypaint_brush_input_from_cname(c.as_ptr()) };
    (id >= 0).then_some(id)
}

/// A brush: libmypaint's settings and the state of the stroke in progress.
pub struct Brush {
    raw: *mut ffi::MyPaintBrush,
}

impl Default for Brush {
    fn default() -> Brush {
        Brush::new()
    }
}

impl Brush {
    /// libmypaint's default brush (a round dab that fades with pressure).
    pub fn new() -> Brush {
        let raw = unsafe { ffi::mypaint_brush_new() };
        assert!(!raw.is_null(), "libmypaint could not make a brush");
        unsafe { ffi::mypaint_brush_from_defaults(raw) };
        Brush { raw }
    }

    /// Read a `.myb` brush file (JSON, version 3): each listed setting gets its base value and its input curves.
    pub fn from_myb(json: &str) -> Result<Brush, String> {
        let v: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("not a brush file: {e}"))?;
        let settings = v["settings"]
            .as_object()
            .ok_or("a brush file needs a `settings` object")?;
        let mut b = Brush::new();
        for (name, s) in settings {
            // settings this libmypaint does not know (older or newer brushes) are skipped
            let Some(id) = setting_id(name) else { continue };
            let base = s["base_value"]
                .as_f64()
                .ok_or_else(|| format!("setting `{name}` has no base_value"))?;
            unsafe { ffi::mypaint_brush_set_base_value(b.raw, id, base as f32) };
            // clear every input first: a file lists only the ones it uses
            for input in 0..INPUT_COUNT {
                unsafe { ffi::mypaint_brush_set_mapping_n(b.raw, id, input, 0) };
            }
            for (iname, pts) in s["inputs"].as_object().into_iter().flatten() {
                let Some(input) = input_id(iname) else {
                    continue;
                };
                let pts = pts
                    .as_array()
                    .ok_or_else(|| format!("input `{iname}` of `{name}` needs a list of points"))?;
                if pts.len() < 2 {
                    return Err(format!(
                        "input `{iname}` of `{name}` needs at least two points"
                    ));
                }
                unsafe { ffi::mypaint_brush_set_mapping_n(b.raw, id, input, pts.len() as c_int) };
                for (i, p) in pts.iter().enumerate() {
                    let (x, y) = (p[0].as_f64().unwrap_or(0.0), p[1].as_f64().unwrap_or(0.0));
                    unsafe {
                        ffi::mypaint_brush_set_mapping_point(
                            b.raw, id, input, i as c_int, x as f32, y as f32,
                        )
                    };
                }
            }
        }
        // the file may not say it, but the brush starts from its own base values
        b.reset();
        Ok(b)
    }

    /// One of the bundled brushes: `pencil`, `pen`, `marker`, `charcoal`, `calligraphy`, `ink`, `smudge`, `eraser`, `rounded`.
    pub fn bundled(name: &str) -> Option<Brush> {
        let (_, json) = BUNDLED.iter().find(|b| b.0 == name)?;
        Brush::from_myb(json).ok()
    }
    pub fn bundled_names() -> Vec<&'static str> {
        BUNDLED.iter().map(|b| b.0).collect()
    }

    fn reset(&mut self) {
        unsafe { ffi::mypaint_brush_reset(self.raw) };
    }

    /// Set a setting's base value by its libmypaint name (`opaque`, `hardness`, `radius_logarithmic`, `eraser`, ...).
    pub fn set(&mut self, setting: &str, value: f32) -> Result<(), String> {
        let id = setting_id(setting)
            .ok_or_else(|| format!("libmypaint has no brush setting `{setting}`"))?;
        unsafe { ffi::mypaint_brush_set_base_value(self.raw, id, value) };
        Ok(())
    }
    pub fn get(&self, setting: &str) -> Option<f32> {
        Some(unsafe { ffi::mypaint_brush_get_base_value(self.raw, setting_id(setting)?) })
    }
    /// The dab radius in pixels (stored by libmypaint as its logarithm).
    pub fn set_radius(&mut self, px: f32) {
        let _ = self.set("radius_logarithmic", px.max(0.2).ln());
    }
    pub fn set_opacity(&mut self, v: f32) {
        let _ = self.set("opaque", v.clamp(0.0, 1.0));
    }
    /// The colour of the paint, as sRGB 0..1 (libmypaint keeps it as hue, saturation and value).
    pub fn set_color(&mut self, rgb: [f32; 3]) {
        let (h, s, v) = rgb_to_hsv(rgb);
        let _ = self.set("color_h", h);
        let _ = self.set("color_s", s);
        let _ = self.set("color_v", v);
    }
}

/// The number of brush inputs of the vendored libmypaint (`MYPAINT_BRUSH_INPUTS_COUNT`).
const INPUT_COUNT: c_int = 18;

fn rgb_to_hsv([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, if max <= 0.0 { 0.0 } else { d / max }, max)
}

impl Drop for Brush {
    fn drop(&mut self) {
        unsafe { ffi::mypaint_brush_unref(self.raw) };
    }
}

/// A fixed size picture of premultiplied RGBA pixels that brushes paint on; it starts transparent.
pub struct Canvas {
    raw: *mut ffi::MyPaintFixedTiledSurface,
    w: u32,
    h: u32,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Canvas {
        let raw =
            unsafe { ffi::mypaint_fixed_tiled_surface_new(w.max(1) as c_int, h.max(1) as c_int) };
        assert!(!raw.is_null(), "libmypaint could not make a canvas");
        unsafe { ffi::mypaint_fixed_tiled_surface_clear(raw) };
        Canvas {
            raw,
            w: w.max(1),
            h: h.max(1),
        }
    }
    pub fn width(&self) -> u32 {
        self.w
    }
    pub fn height(&self) -> u32 {
        self.h
    }
    pub fn clear(&mut self) {
        unsafe { ffi::mypaint_fixed_tiled_surface_clear(self.raw) };
    }

    /// Paint one stroke with the default options (see [`Canvas::stroke_with`]).
    pub fn stroke(&mut self, brush: &mut Brush, points: &[Point]) {
        self.stroke_with(brush, points, &StrokeOpts::default());
    }

    /// Paint one stroke. libmypaint wants a first sample with no pressure to start the stroke and one to end it.
    /// With a blend mode other than `Normal` (or a strength below 1) the stroke is painted on a transparent copy of the
    /// canvas first and then composited onto the picture, so a smudge brush sees none of the paint underneath.
    pub fn stroke_with(&mut self, brush: &mut Brush, points: &[Point], opts: &StrokeOpts) {
        if points.is_empty() {
            return;
        }
        if opts.blend == Blend::Normal && opts.strength >= 1.0 {
            self.paint(brush, points, opts);
            return;
        }
        let mut layer = Canvas::new(self.w, self.h);
        layer.paint(brush, points, opts);
        self.composite(&layer, opts.blend, opts.strength.clamp(0.0, 1.0));
    }

    fn paint(&mut self, brush: &mut Brush, points: &[Point], opts: &StrokeOpts) {
        let first = &points[0];
        unsafe {
            match opts.symmetry {
                Some(s) => {
                    let kind = match s.kind {
                        SymmetryKind::Vertical => 0,
                        SymmetryKind::Horizontal => 1,
                        SymmetryKind::Both => 2,
                        SymmetryKind::Radial => 3,
                        SymmetryKind::Snowflake => 4,
                    };
                    ffi::mypaint_tiled_surface_set_symmetry_state(
                        self.raw,
                        1,
                        s.center[0],
                        s.center[1],
                        s.angle_deg,
                        kind,
                        s.lines.max(2),
                    );
                }
                None => {
                    ffi::mypaint_tiled_surface_set_symmetry_state(self.raw, 0, 0.0, 0.0, 0.0, 0, 2)
                }
            }
            let surf = ffi::mypaint_fixed_tiled_surface_interface(self.raw);
            ffi::mypaint_surface_begin_atomic(surf);
            brush.reset();
            ffi::mypaint_brush_new_stroke(brush.raw);
            let to = |p: &Point, pressure: f32, dt: f64| {
                ffi::mypaint_brush_stroke_to(
                    brush.raw,
                    surf,
                    p.x,
                    p.y,
                    pressure,
                    p.xtilt,
                    p.ytilt,
                    dt,
                    1.0,
                    0.0,
                    opts.barrel,
                    0,
                );
            };
            // libmypaint smooths the cursor towards where it was before: a long first time step (above its 5 s
            // limit) makes the stroke start exactly at its first point instead of sliding in from the old position
            to(first, 0.0, 10.0);
            for p in points {
                to(p, p.pressure.clamp(0.0, 1.0), p.dt.max(0.0));
            }
            let last = points.last().expect("not empty");
            to(last, 0.0, 0.05);
            ffi::mypaint_surface_end_atomic(surf, std::ptr::null_mut());
            ffi::mypaint_tiled_surface_set_symmetry_state(self.raw, 0, 0.0, 0.0, 0.0, 0, 2);
        }
    }

    fn read_raw(&self) -> Vec<u16> {
        let mut raw = vec![0u16; (self.w * self.h) as usize * 4];
        unsafe { ffi::mypaint_fixed_tiled_surface_read(self.raw, raw.as_mut_ptr()) };
        raw
    }

    /// Composite `top` (a picture of the same size) onto this one with the blend mode, premultiplied 15-bit pixels:
    /// `co = cs (1 - ab) + cb (1 - as) + as ab B(cb / ab, cs / as)` and `ao = as + ab - as ab`.
    fn composite(&mut self, top: &Canvas, blend: Blend, strength: f32) {
        let (mut dst, src) = (self.read_raw(), top.read_raw());
        const ONE: f32 = 32768.0;
        for (d, s) in dst.chunks_mut(4).zip(src.chunks(4)) {
            let a_s = s[3] as f32 / ONE * strength;
            if a_s <= 0.0 {
                continue;
            }
            let a_b = d[3] as f32 / ONE;
            let a_o = a_s + a_b - a_s * a_b;
            for k in 0..3 {
                let cs_p = s[k] as f32 / ONE * strength;
                let cb_p = d[k] as f32 / ONE;
                let cs = if a_s > 0.0 {
                    (cs_p / a_s).min(1.0)
                } else {
                    0.0
                };
                let cb = if a_b > 0.0 {
                    (cb_p / a_b).min(1.0)
                } else {
                    0.0
                };
                let mixed = a_s * a_b * blend.channel(cb, cs);
                let co = cs_p * (1.0 - a_b) + cb_p * (1.0 - a_s) + mixed;
                d[k] = (co.clamp(0.0, a_o) * ONE + 0.5) as u16;
            }
            d[3] = (a_o.clamp(0.0, 1.0) * ONE + 0.5) as u16;
        }
        unsafe { ffi::mypaint_fixed_tiled_surface_write(self.raw, dst.as_ptr()) };
    }

    /// The picture as `width * height * 4` bytes, premultiplied RGBA, row by row.
    pub fn rgba8(&self) -> Vec<u8> {
        let n = (self.w * self.h) as usize;
        let mut raw = vec![0u16; n * 4];
        unsafe { ffi::mypaint_fixed_tiled_surface_read(self.raw, raw.as_mut_ptr()) };
        raw.iter()
            .map(|&v| ((v as u32 * 255 + 16384) / 32768).min(255) as u8)
            .collect()
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe { ffi::mypaint_surface_unref(ffi::mypaint_fixed_tiled_surface_interface(self.raw)) };
    }
}
