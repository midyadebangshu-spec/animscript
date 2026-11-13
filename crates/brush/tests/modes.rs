//! Tilt, symmetry and per-stroke blend modes. Written first.
use animscript_brush::{Blend, Brush, Canvas, Point, StrokeOpts, Symmetry, SymmetryKind};

fn pt(x: f32, y: f32, xt: f32, yt: f32) -> Point {
    Point {
        x,
        y,
        pressure: 0.9,
        xtilt: xt,
        ytilt: yt,
        dt: 0.01,
    }
}
fn line(x0: f32, y0: f32, x1: f32, y1: f32, n: usize) -> Vec<Point> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            pt(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, 0.0, 0.0)
        })
        .collect()
}
fn alpha(px: &[u8], w: u32, x: u32, y: u32) -> u8 {
    px[((y * w + x) * 4 + 3) as usize]
}
fn rgb(px: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let o = ((y * w + x) * 4) as usize;
    [px[o], px[o + 1], px[o + 2]]
}
fn extent(px: &[u8], w: u32, h: u32) -> (u32, u32) {
    let (mut x0, mut x1, mut y0, mut y1) = (w, 0, h, 0);
    for y in 0..h {
        for x in 0..w {
            if alpha(px, w, x, y) > 20 {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    if x1 < x0 {
        (0, 0)
    } else {
        (x1 - x0 + 1, y1 - y0 + 1)
    }
}
/// A brush whose dab is a long ellipse turned by the pen's tilt.
fn tilt_brush() -> Brush {
    Brush::from_myb(r#"{"version": 3, "settings": {
        "opaque": {"base_value": 1.0, "inputs": {}},
        "hardness": {"base_value": 0.95, "inputs": {}},
        "radius_logarithmic": {"base_value": 1.8, "inputs": {}},
        "elliptical_dab_ratio": {"base_value": 4.0, "inputs": {}},
        "elliptical_dab_angle": {"base_value": 0.0, "inputs": {"tilt_ascension": [[-180.0, 0.0], [180.0, 180.0]]}}
    }}"#).unwrap()
}
fn dab(opts: &StrokeOpts, xt: f32, yt: f32) -> (u32, u32) {
    let mut c = Canvas::new(80, 80);
    let mut b = tilt_brush();
    c.stroke_with(
        &mut b,
        &[
            pt(38.0, 40.0, xt, yt),
            pt(40.0, 40.0, xt, yt),
            pt(42.0, 40.0, xt, yt),
        ],
        opts,
    );
    extent(&c.rgba8(), 80, 80)
}

// ---- tilt ---------------------------------------------------------------------------------

#[test]
fn the_pens_tilt_turns_a_tilt_sensitive_dab() {
    let o = StrokeOpts::default();
    let flat = dab(&o, 0.0, 0.0);
    let right = dab(&o, 0.9, 0.0);
    let down = dab(&o, 0.0, 0.9);
    assert_ne!(
        right, down,
        "tilting a different way gives a different dab: {right:?} {down:?}"
    );
    assert!(right != flat || down != flat);
    // one of the two tilts lays the long axis along x, the other along y
    let wide = |e: (u32, u32)| e.0 > e.1;
    assert_ne!(wide(right), wide(down), "{right:?} vs {down:?}");
}

#[test]
fn a_brush_that_ignores_tilt_ignores_it() {
    let paint = |xt: f32| {
        let mut c = Canvas::new(60, 40);
        let mut b = Brush::new();
        b.set_radius(5.0);
        let pts: Vec<Point> = (0..=20)
            .map(|i| pt(10.0 + i as f32 * 2.0, 20.0, xt, 0.0))
            .collect();
        c.stroke_with(&mut b, &pts, &StrokeOpts::default());
        c.rgba8()
    };
    assert_eq!(paint(0.0), paint(0.9));
}

// ---- symmetry -----------------------------------------------------------------------------

fn sym(kind: SymmetryKind, lines: i32) -> StrokeOpts {
    StrokeOpts {
        symmetry: Some(Symmetry {
            kind,
            center: [100.0, 100.0],
            angle_deg: 0.0,
            lines,
        }),
        ..StrokeOpts::default()
    }
}
fn plain() -> Brush {
    let mut b = Brush::new();
    b.set_radius(4.0);
    b.set_opacity(1.0);
    b.set("hardness", 0.9).unwrap();
    b
}
fn spot(c: &mut Canvas, opts: &StrokeOpts, x: f32, y: f32) {
    let mut b = plain();
    c.stroke_with(&mut b, &line(x - 5.0, y, x + 5.0, y, 10), opts);
}

#[test]
fn vertical_symmetry_mirrors_across_the_centre_line() {
    let mut c = Canvas::new(200, 200);
    c.stroke_with(
        &mut plain(),
        &line(30.0, 50.0, 80.0, 120.0, 30),
        &sym(SymmetryKind::Vertical, 2),
    );
    let px = c.rgba8();
    assert!(alpha(&px, 200, 55, 85) > 100, "the stroke itself");
    for (x, y) in [(40u32, 63u32), (55, 85), (70, 105)] {
        let (a, b) = (alpha(&px, 200, x, y), alpha(&px, 200, 199 - x, y));
        assert!((a as i32 - b as i32).abs() <= 3, "({x}, {y}): {a} vs {b}");
    }
    assert_eq!(
        alpha(&px, 200, 55, 199 - 85),
        0,
        "vertical symmetry does not flip top and bottom"
    );
}

#[test]
fn horizontal_and_both_give_two_and_four_copies() {
    let mut h = Canvas::new(200, 200);
    spot(&mut h, &sym(SymmetryKind::Horizontal, 2), 60.0, 50.0);
    let px = h.rgba8();
    assert!(alpha(&px, 200, 60, 50) > 100 && alpha(&px, 200, 60, 150) > 100);
    assert_eq!(alpha(&px, 200, 140, 50), 0);
    let mut b = Canvas::new(200, 200);
    spot(&mut b, &sym(SymmetryKind::Both, 2), 60.0, 50.0);
    let px = b.rgba8();
    for (x, y) in [(60, 50), (140, 50), (60, 150), (140, 150)] {
        assert!(alpha(&px, 200, x, y) > 100, "({x}, {y})");
    }
}

#[test]
fn radial_symmetry_repeats_around_the_centre() {
    for lines in [3, 6] {
        let mut c = Canvas::new(200, 200);
        spot(&mut c, &sym(SymmetryKind::Radial, lines), 140.0, 100.0); // 40 px right of the centre
        let px = c.rgba8();
        for k in 0..lines {
            let a = std::f32::consts::TAU * k as f32 / lines as f32;
            let (x, y) = (100.0 + 40.0 * a.cos(), 100.0 + 40.0 * a.sin());
            assert!(
                alpha(&px, 200, x.round() as u32, y.round() as u32) > 100,
                "{lines} lines: copy {k} at ({x:.0}, {y:.0})"
            );
        }
        let mid = std::f32::consts::TAU / (2.0 * lines as f32);
        assert_eq!(
            alpha(
                &px,
                200,
                (100.0 + 40.0 * mid.cos()).round() as u32,
                (100.0 + 40.0 * mid.sin()).round() as u32
            ),
            0,
            "nothing between the copies"
        );
    }
}

#[test]
fn the_symmetry_angle_turns_the_pattern_and_no_symmetry_is_the_default() {
    let mut c = Canvas::new(200, 200);
    let mut o = sym(SymmetryKind::Radial, 4);
    o.symmetry.as_mut().unwrap().angle_deg = 0.0;
    spot(&mut c, &o, 140.0, 100.0);
    assert!(
        alpha(&c.rgba8(), 200, 100, 140) > 100,
        "a quarter turn lands below the centre"
    );
    let mut plain_canvas = Canvas::new(200, 200);
    spot(&mut plain_canvas, &StrokeOpts::default(), 140.0, 100.0);
    assert_eq!(alpha(&plain_canvas.rgba8(), 200, 100, 140), 0);
    // the state is cleared afterwards: a later plain stroke is not repeated
    let mut c2 = Canvas::new(200, 200);
    spot(&mut c2, &sym(SymmetryKind::Both, 2), 60.0, 50.0);
    spot(&mut c2, &StrokeOpts::default(), 30.0, 170.0);
    assert_eq!(alpha(&c2.rgba8(), 200, 170, 170), 0, "no leftover mirror");
}

// ---- blend modes --------------------------------------------------------------------------

fn solid(canvas: &mut Canvas, rgb: [f32; 3], opts: &StrokeOpts) {
    let mut b = Brush::new();
    b.set_radius(30.0);
    b.set_opacity(1.0);
    b.set("hardness", 1.0).unwrap();
    b.set_color(rgb);
    let pts: Vec<Point> = (0..=20)
        .map(|i| Point {
            x: 10.0 + i as f32,
            y: 20.0,
            pressure: 1.0,
            xtilt: 0.0,
            ytilt: 0.0,
            dt: 0.01,
        })
        .collect();
    canvas.stroke_with(&mut b, &pts, opts);
}
fn mid(canvas: &Canvas) -> [f32; 3] {
    let px = canvas.rgba8();
    let c = rgb(&px, 40, 20, 20);
    let a = alpha(&px, 40, 20, 20) as f32 / 255.0;
    assert!(a > 0.97, "opaque at the middle: {a}");
    [
        c[0] as f32 / 255.0 / a,
        c[1] as f32 / 255.0 / a,
        c[2] as f32 / 255.0 / a,
    ]
}
fn only(rgb_: [f32; 3]) -> [f32; 3] {
    let mut c = Canvas::new(40, 40);
    solid(&mut c, rgb_, &StrokeOpts::default());
    mid(&c)
}
fn blended(back: [f32; 3], src: [f32; 3], blend: Blend, strength: f32) -> [f32; 3] {
    let mut c = Canvas::new(40, 40);
    solid(&mut c, back, &StrokeOpts::default());
    solid(
        &mut c,
        src,
        &StrokeOpts {
            blend,
            strength,
            ..StrokeOpts::default()
        },
    );
    mid(&c)
}
fn near(a: [f32; 3], b: [f32; 3], tol: f32) {
    for k in 0..3 {
        assert!((a[k] - b[k]).abs() <= tol, "{a:?} vs {b:?}");
    }
}

#[test]
fn blend_modes_follow_their_formulas() {
    let (back, src) = ([0.8, 0.5, 0.2], [0.4, 0.6, 0.9]);
    let (b, s) = (only(back), only(src));
    let f = |g: fn(f32, f32) -> f32| [g(b[0], s[0]), g(b[1], s[1]), g(b[2], s[2])];
    near(
        blended(back, src, Blend::Multiply, 1.0),
        f(|b, s| b * s),
        0.02,
    );
    near(
        blended(back, src, Blend::Screen, 1.0),
        f(|b, s| b + s - b * s),
        0.02,
    );
    near(
        blended(back, src, Blend::Darken, 1.0),
        f(|b, s| b.min(s)),
        0.02,
    );
    near(
        blended(back, src, Blend::Lighten, 1.0),
        f(|b, s| b.max(s)),
        0.02,
    );
    near(
        blended(back, src, Blend::Difference, 1.0),
        f(|b, s| (b - s).abs()),
        0.02,
    );
    near(
        blended(back, src, Blend::Exclusion, 1.0),
        f(|b, s| b + s - 2.0 * b * s),
        0.02,
    );
    near(
        blended(back, src, Blend::Overlay, 1.0),
        f(|b, s| {
            if b <= 0.5 {
                2.0 * b * s
            } else {
                1.0 - 2.0 * (1.0 - b) * (1.0 - s)
            }
        }),
        0.02,
    );
    near(
        blended(back, src, Blend::Add, 1.0),
        f(|b, s| (b + s).min(1.0)),
        0.02,
    );
}

#[test]
fn normal_blend_is_the_plain_stroke_and_strength_mixes() {
    let (back, src) = ([0.8, 0.5, 0.2], [0.4, 0.6, 0.9]);
    near(blended(back, src, Blend::Normal, 1.0), only(src), 0.01);
    let (full, half, none) = (
        blended(back, src, Blend::Multiply, 1.0),
        blended(back, src, Blend::Multiply, 0.5),
        blended(back, src, Blend::Multiply, 0.0),
    );
    near(none, only(back), 0.02);
    for k in 0..3 {
        let (lo, hi) = (full[k].min(only(back)[k]), full[k].max(only(back)[k]));
        assert!(
            half[k] >= lo - 0.02 && half[k] <= hi + 0.02,
            "half way: {half:?} between {full:?} and {:?}",
            only(back)
        );
    }
}

#[test]
fn blending_onto_nothing_paints_the_stroke_and_later_strokes_see_the_result() {
    let mut c = Canvas::new(40, 40);
    solid(
        &mut c,
        [0.9, 0.1, 0.1],
        &StrokeOpts {
            blend: Blend::Multiply,
            ..StrokeOpts::default()
        },
    );
    let s = only([0.9, 0.1, 0.1]);
    near(mid(&c), s, 0.02); // multiplied with a transparent backdrop is just the stroke
    let mut b = Brush::bundled("smudge").unwrap();
    b.set_radius(8.0);
    c.stroke_with(
        &mut b,
        &line(20.0, 20.0, 38.0, 20.0, 10),
        &StrokeOpts::default(),
    );
    assert!(
        alpha(&c.rgba8(), 40, 36, 20) > 0,
        "a smudge afterwards drags the blended paint"
    );
}

#[test]
fn blend_names_round_trip() {
    for n in [
        "normal",
        "multiply",
        "screen",
        "overlay",
        "darken",
        "lighten",
        "color_dodge",
        "color_burn",
        "hard_light",
        "soft_light",
        "difference",
        "exclusion",
        "add",
    ] {
        assert_eq!(Blend::from_name(n).unwrap().name(), n);
    }
    assert_eq!(Blend::from_name("no_such"), None);
}
