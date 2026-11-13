//! The brush engine: libmypaint drawing strokes onto a fixed canvas. Written first.
use animscript_brush::{Brush, Canvas, Point};

fn line(x0: f32, y0: f32, x1: f32, y1: f32, n: usize, pressure: f32) -> Vec<Point> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            Point {
                x: x0 + (x1 - x0) * t,
                y: y0 + (y1 - y0) * t,
                pressure,
                xtilt: 0.0,
                ytilt: 0.0,
                dt: 0.01,
            }
        })
        .collect()
}
fn alpha(px: &[u8], w: u32, x: u32, y: u32) -> u8 {
    px[((y * w + x) * 4 + 3) as usize]
}
fn rgb(px: &[u8], w: u32, x: u32, y: u32) -> (u8, u8, u8) {
    let o = ((y * w + x) * 4) as usize;
    (px[o], px[o + 1], px[o + 2])
}
fn covered(px: &[u8], w: u32, h: u32) -> usize {
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| alpha(px, w, x, y) > 0)
        .count()
}
fn plain() -> Brush {
    let mut b = Brush::new();
    b.set_radius(6.0);
    b.set("opaque", 1.0).unwrap();
    b.set("hardness", 0.9).unwrap();
    b
}

#[test]
fn a_stroke_paints_along_its_path_and_nowhere_else() {
    let mut c = Canvas::new(120, 60);
    let mut b = plain();
    c.stroke(&mut b, &line(20.0, 30.0, 100.0, 30.0, 40, 0.8));
    let px = c.rgba8();
    assert!(
        alpha(&px, 120, 60, 30) > 200,
        "the middle of the line is painted"
    );
    assert_eq!(alpha(&px, 120, 60, 5), 0, "far from it nothing is");
    assert_eq!(
        alpha(&px, 120, 5, 30),
        0,
        "and the line does not start before its start"
    );
}

#[test]
fn a_new_canvas_is_transparent_and_clear_empties_it() {
    let mut c = Canvas::new(40, 40);
    assert_eq!(covered(&c.rgba8(), 40, 40), 0);
    let mut b = plain();
    c.stroke(&mut b, &line(5.0, 20.0, 35.0, 20.0, 10, 0.8));
    assert!(covered(&c.rgba8(), 40, 40) > 0);
    c.clear();
    assert_eq!(covered(&c.rgba8(), 40, 40), 0);
}

#[test]
fn the_same_stroke_gives_the_same_picture() {
    let paint = || {
        let mut c = Canvas::new(100, 60);
        let mut b = Brush::bundled("charcoal").unwrap(); // a brush with random jitter
        b.set_radius(5.0);
        c.stroke(&mut b, &line(10.0, 30.0, 90.0, 30.0, 30, 0.7));
        c.rgba8()
    };
    assert_eq!(paint(), paint());
}

#[test]
fn a_bigger_radius_covers_more() {
    let area = |r: f32| {
        let mut c = Canvas::new(100, 60);
        let mut b = plain();
        b.set_radius(r);
        c.stroke(&mut b, &line(15.0, 30.0, 85.0, 30.0, 30, 0.8));
        covered(&c.rgba8(), 100, 60)
    };
    assert!(
        area(10.0) > area(3.0) * 2,
        "{} vs {}",
        area(10.0),
        area(3.0)
    );
}

#[test]
fn the_colour_setting_is_the_colour_painted() {
    let mut c = Canvas::new(60, 40);
    let mut b = plain();
    b.set_color([1.0, 0.0, 0.0]);
    c.stroke(&mut b, &line(10.0, 20.0, 50.0, 20.0, 20, 0.9));
    let px = c.rgba8();
    let (r, g, bl) = rgb(&px, 60, 30, 20);
    assert!(r > 200 && g < 40 && bl < 40, "red: {r} {g} {bl}");
    let mut c = Canvas::new(60, 40);
    b.set_color([0.0, 0.0, 1.0]);
    c.stroke(&mut b, &line(10.0, 20.0, 50.0, 20.0, 20, 0.9));
    let (r, g, bl) = rgb(&c.rgba8(), 60, 30, 20);
    assert!(bl > 200 && r < 40 && g < 40, "blue: {r} {g} {bl}");
}

#[test]
fn pressure_changes_how_much_paint_a_pressure_brush_lays_down() {
    let a = |p: f32| {
        let mut c = Canvas::new(80, 40);
        let mut b = Brush::bundled("pencil").unwrap();
        b.set_radius(4.0);
        c.stroke(&mut b, &line(10.0, 20.0, 70.0, 20.0, 40, p));
        c.rgba8().chunks(4).map(|p| p[3] as u32).sum::<u32>()
    };
    assert!(a(1.0) > a(0.2), "{} vs {}", a(1.0), a(0.2));
}

#[test]
fn an_eraser_brush_takes_paint_away() {
    let mut c = Canvas::new(80, 40);
    let mut paint = plain();
    c.stroke(&mut paint, &line(10.0, 20.0, 70.0, 20.0, 30, 0.9));
    let before = covered(&c.rgba8(), 80, 40);
    let mut eraser = plain();
    eraser.set("eraser", 1.0).unwrap();
    eraser.set_radius(10.0);
    c.stroke(&mut eraser, &line(30.0, 20.0, 50.0, 20.0, 10, 1.0));
    let px = c.rgba8();
    assert_eq!(alpha(&px, 80, 40, 20), 0, "erased in the middle");
    assert!(alpha(&px, 80, 15, 20) > 100, "kept at the ends");
    assert!(covered(&px, 80, 40) < before);
}

#[test]
fn a_smudge_brush_drags_colour_along() {
    let mut c = Canvas::new(100, 60);
    let mut red = plain();
    red.set_radius(10.0);
    red.set_color([1.0, 0.0, 0.0]);
    c.stroke(&mut red, &line(10.0, 30.0, 40.0, 30.0, 20, 0.9));
    let mut smudge = Brush::bundled("smudge").unwrap();
    smudge.set_radius(8.0);
    c.stroke(&mut smudge, &line(30.0, 30.0, 90.0, 30.0, 40, 1.0));
    let px = c.rgba8();
    assert!(
        alpha(&px, 100, 70, 30) > 0,
        "paint was carried beyond where it was laid"
    );
    let (r, g, _) = rgb(&px, 100, 70, 30);
    let a = alpha(&px, 100, 70, 30) as u32;
    assert!(
        r as u32 * 255 / a > 150 && (g as u32) * 255 / a < 100,
        "and it is red: r {r} g {g} a {a}"
    );
}

#[test]
fn bundled_brushes_load_and_a_bad_file_is_an_error() {
    for n in Brush::bundled_names() {
        assert!(Brush::bundled(n).is_some(), "{n}");
    }
    assert!(Brush::bundled("nope").is_none());
    assert!(Brush::from_myb("not json").is_err());
    assert!(
        Brush::from_myb("{\"version\": 3}").is_err(),
        "needs settings"
    );
}

#[test]
fn myb_settings_are_applied() {
    let b = Brush::from_myb(r#"{"version": 3, "settings": {"opaque": {"base_value": 0.25, "inputs": {}}, "radius_logarithmic": {"base_value": 1.5, "inputs": {"pressure": [[0.0, 0.0], [1.0, 1.0]]}}}}"#).unwrap();
    assert_eq!(b.get("opaque"), Some(0.25));
    assert_eq!(b.get("radius_logarithmic"), Some(1.5));
    assert_eq!(b.get("no_such_setting"), None);
    let mut b = b;
    assert!(b.set("no_such_setting", 1.0).is_err());
}

#[test]
fn strokes_may_leave_the_canvas_and_empty_strokes_are_fine() {
    let mut c = Canvas::new(40, 40);
    let mut b = plain();
    c.stroke(&mut b, &line(-30.0, 20.0, 80.0, 20.0, 30, 0.8));
    assert!(alpha(&c.rgba8(), 40, 20, 20) > 100);
    c.stroke(&mut b, &[]);
    c.stroke(&mut b, &line(5.0, 5.0, 5.0, 5.0, 1, 0.5));
}

#[test]
fn a_stroke_starts_at_its_first_point_even_with_slow_tracking() {
    // the ink brush smooths the pen position; without a clean start the line would slide in from the last position
    let mut c = Canvas::new(200, 100);
    let mut ink = Brush::bundled("ink").unwrap();
    ink.set_radius(3.0);
    c.stroke(&mut ink, &line(10.0, 10.0, 30.0, 10.0, 10, 0.8));
    c.stroke(&mut ink, &line(150.0, 60.0, 190.0, 60.0, 20, 0.8));
    let px = c.rgba8();
    for x in [60u32, 90, 120] {
        // on the straight path from the end of the first stroke to the start of the second: nothing
        let y = 10 + (x - 30) * 50 / 120;
        assert_eq!(alpha(&px, 200, x, y), 0, "a stray tail at ({x}, {y})");
    }
    assert!(alpha(&px, 200, 170, 60) > 100);
}
