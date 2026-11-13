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

    /// Paint one stroke. libmypaint wants a first sample with no pressure to start the stroke and one to end it.
    pub fn stroke(&mut self, brush: &mut Brush, points: &[Point]) {
        let Some(first) = points.first() else { return };
        unsafe {
            let surf = ffi::mypaint_fixed_tiled_surface_interface(self.raw);
            ffi::mypaint_surface_begin_atomic(surf);
            brush.reset();
            ffi::mypaint_brush_new_stroke(brush.raw);
            let to = |p: &Point, pressure: f32, dt: f64| {
                ffi::mypaint_brush_stroke_to(
                    brush.raw, surf, p.x, p.y, pressure, p.xtilt, p.ytilt, dt, 1.0, 0.0, 0.0, 0,
                );
            };
            to(first, 0.0, 0.1);
            for p in points {
                to(p, p.pressure.clamp(0.0, 1.0), p.dt.max(0.0));
            }
            let last = points.last().expect("not empty");
            to(last, 0.0, 0.05);
            ffi::mypaint_surface_end_atomic(surf, std::ptr::null_mut());
        }
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
