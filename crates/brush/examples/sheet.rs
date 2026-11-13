//! Paints one wavy stroke with every bundled brush and writes raw RGBA (premultiplied) to the file given.
use animscript_brush::{Brush, Canvas, Point};
fn main() {
    let out = std::env::args().nth(1).expect("output file");
    let names = Brush::bundled_names();
    let (w, h) = (400u32, 40 * names.len() as u32 + 20);
    let mut c = Canvas::new(w, h);
    for (i, n) in names.iter().enumerate() {
        let mut b = Brush::bundled(n).unwrap();
        b.set_color([0.1, 0.1, 0.35]);
        let y0 = 30.0 + 40.0 * i as f32;
        let pts: Vec<Point> = (0..=60)
            .map(|k| {
                let t = k as f32 / 60.0;
                Point {
                    x: 20.0 + 360.0 * t,
                    y: y0 + 10.0 * (t * 12.0).sin(),
                    pressure: 0.3 + 0.6 * (t * std::f32::consts::PI).sin(),
                    xtilt: 0.0,
                    ytilt: 0.0,
                    dt: 0.012,
                }
            })
            .collect();
        c.stroke(&mut b, &pts);
    }
    std::fs::write(out, c.rgba8()).unwrap();
    println!("{w} {h}");
}
