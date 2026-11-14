//! Automation lanes: parsing, interpolation, and their effect on the mix.
use animscript_audio::automation::*;
use animscript_audio::*;

const BAR: f64 = 3840.0;
const M44: Meter = Meter { num: 4, den: 4 };

fn lane(src: &str) -> Lane {
    Lane::parse(src, M44, 0.0, 4.0, "volume").unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn points_are_bars_and_values() {
    let l = lane("0:0.2 4:1 8:0.5");
    assert_eq!(l.points.len(), 3);
    assert_eq!(l.points[1].0, 4.0 * BAR);
    assert_eq!(l.points[2].1, 0.5);
    // a fractional bar
    assert_eq!(lane("1.5:1").points[0].0, 1.5 * BAR);
}

#[test]
fn lanes_interpolate_linearly_smoothly_or_hold() {
    let l = lane("0:0 2:1");
    assert!((l.at_ticks(BAR) - 0.5).abs() < 1e-12);
    assert!((l.at_ticks(0.5 * BAR) - 0.25).abs() < 1e-12);
    // before and after the points the value is held
    assert_eq!(lane("1:0.3 2:0.9").at_ticks(0.0), 0.3);
    assert_eq!(lane("1:0.3 2:0.9").at_ticks(5.0 * BAR), 0.9);
    // an `s` point eases: the quarter-way value is below the straight line, the middle is the same
    let s = lane("0:0s 2:1");
    assert!(s.at_ticks(0.5 * BAR) < 0.25 - 0.05);
    assert!((s.at_ticks(BAR) - 0.5).abs() < 1e-12);
    // an `h` point holds until the next one
    let h = lane("0:0.2h 2:1");
    assert_eq!(h.at_ticks(1.99 * BAR), 0.2);
    assert_eq!(h.at_ticks(2.0 * BAR), 1.0);
}

#[test]
fn seconds_use_the_tempo() {
    // at 120 bpm a bar is 2 s: bar 2 is 4 s
    let l = lane("0:0 2:1");
    assert!((l.at_secs(2.0, 120.0) - 0.5).abs() < 1e-12);
    assert!((l.at_secs(4.0, 120.0) - 1.0).abs() < 1e-12);
}

#[test]
fn bad_lanes_are_errors() {
    let e = |src: &str, lo: f64, hi: f64| Lane::parse(src, M44, lo, hi, "pan").unwrap_err().message;
    assert!(e("0:2", -1.0, 1.0).contains("between -1 and 1"));
    assert!(e("4:1 2:0", -1.0, 1.0).contains("forward"));
    assert!(e("4", -1.0, 1.0).contains("bar:value"));
    assert!(e("x:1", -1.0, 1.0).contains("number"));
    assert!(e("-1:0", -1.0, 1.0).contains("negative"));
    assert!(e("", -1.0, 1.0).contains("no points"));
    assert!(e("0:1 0:1", -1.0, 1.0).contains("forward"));
}

fn long_note(instrument: &str, auto: Automation) -> Score {
    let mut s = Score::new(120.0, Meter::new(4, 4));
    let mut t = Track::new(
        "t",
        instrument,
        parse_notes("A3:2", Meter::new(4, 4)).unwrap(),
    ); // two bars: 4 s
    t.auto = auto;
    s.tracks.push(t);
    s
}
fn render(s: &Score, mix: &Mix) -> Stereo {
    render_score(s, mix, &Bank::new()).unwrap()
}
fn rough(x: &[f32]) -> f64 {
    let d: f64 = x
        .windows(3)
        .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs() as f64)
        .sum();
    let a: f64 = x.iter().map(|v| v.abs() as f64).sum();
    d / a.max(1e-12)
}
fn seg(st: &Stereo, from: f64, to: f64) -> &[f32] {
    &st.left[(from * st.rate as f64) as usize..(to * st.rate as f64) as usize]
}

#[test]
fn a_volume_lane_fades_a_track_in() {
    let auto = Automation {
        volume: Some(lane("0:0 2:1")),
        ..Automation::default()
    };
    let st = render(&long_note("sine", auto), &Mix::default());
    let (early, late) = (st.rms(0.2, 0.6), st.rms(3.2, 3.6));
    assert!(late > early * 4.0, "{early} {late}");
    // without the lane the level is steady
    let flat = render(&long_note("sine", Automation::default()), &Mix::default());
    assert!((flat.rms(0.5, 0.9) / flat.rms(3.0, 3.4) - 1.0).abs() < 0.1);
}

#[test]
fn a_pan_lane_moves_the_sound_across() {
    let auto = Automation {
        pan: Some(Lane::parse("0:-1 2:1", M44, -1.0, 1.0, "pan").unwrap()),
        ..Automation::default()
    };
    let st = render(&long_note("sine", auto), &Mix::default());
    let side = |ch: &[f32], a: f64, b: f64| -> f64 {
        let x = &ch[(a * st.rate as f64) as usize..(b * st.rate as f64) as usize];
        x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64
    };
    // the start is left, the end is right
    assert!(side(&st.left, 0.1, 0.5) > 20.0 * side(&st.right, 0.1, 0.5));
    assert!(side(&st.right, 3.4, 3.8) > 20.0 * side(&st.left, 3.4, 3.8));
}

#[test]
fn a_cutoff_lane_sweeps_the_filter_of_a_subtractive_voice() {
    let auto = Automation {
        cutoff: Some(Lane::parse("0:0.1 2:4", M44, 0.05, 8.0, "cutoff").unwrap()),
        ..Automation::default()
    };
    let st = render(&long_note("saw_lead", auto), &Mix::default());
    // dark at the start, bright at the end (compare the shape of the wave, not the level)
    assert!(
        rough(seg(&st, 3.3, 3.7)) > rough(seg(&st, 0.3, 0.7)) * 1.5,
        "{} {}",
        rough(seg(&st, 3.3, 3.7)),
        rough(seg(&st, 0.3, 0.7))
    );
}

#[test]
fn brightness_opens_and_closes_the_tone() {
    let dark = Automation {
        brightness: Some(Lane::parse("0:0.1", M44, 0.0, 1.0, "brightness").unwrap()),
        ..Automation::default()
    };
    let bright = Automation {
        brightness: Some(Lane::parse("0:0.9", M44, 0.0, 1.0, "brightness").unwrap()),
        ..Automation::default()
    };
    let (d, b) = (
        render(&long_note("saw_lead", dark.clone()), &Mix::default()),
        render(&long_note("saw_lead", bright.clone()), &Mix::default()),
    );
    assert!(rough(seg(&b, 0.5, 1.0)) > rough(seg(&d, 0.5, 1.0)) * 1.5);
    // an FM voice takes it once per note: brighter is a higher index, which makes a busier wave
    let fm = |a: Automation| {
        let mut s = Score::new(120.0, Meter::new(4, 4));
        let mut t = Track::new(
            "t",
            "bell",
            parse_notes("A4:1/2", Meter::new(4, 4)).unwrap(),
        );
        t.auto = a;
        s.tracks.push(t);
        render(&s, &Mix::default())
    };
    assert!(rough(seg(&fm(bright.clone()), 0.05, 0.4)) > rough(seg(&fm(dark.clone()), 0.05, 0.4)));
}

#[test]
fn a_reverb_lane_controls_how_much_of_a_track_reaches_the_reverb() {
    let wet = Mix {
        reverb: 0.5,
        ..Mix::default()
    };
    let none = Automation {
        reverb: Some(Lane::parse("0:0", M44, 0.0, 2.0, "reverb").unwrap()),
        ..Automation::default()
    };
    let full = Automation {
        reverb: Some(Lane::parse("0:1", M44, 0.0, 2.0, "reverb").unwrap()),
        ..Automation::default()
    };
    let mut s = Score::new(120.0, Meter::new(4, 4));
    s.tracks.push(Track::new(
        "t",
        "sine",
        parse_notes("A4:1/4", Meter::new(4, 4)).unwrap(),
    ));
    let tail = |a: Automation| {
        let mut sc = s.clone();
        sc.tracks[0].auto = a;
        render(&sc, &wet).rms(1.0, 2.5)
    };
    let (n, f) = (tail(none), tail(full));
    assert!(n < 1e-5, "no send, no tail: {n}");
    assert!(f > 1e-3, "{f}");
    // the dry sound is the same either way
    let dry = |a: Automation| {
        let mut sc = s.clone();
        sc.tracks[0].auto = a;
        render(&sc, &wet).rms(0.0, 0.4)
    };
    assert!(
        (dry(Automation {
            reverb: Some(Lane::parse("0:0", M44, 0.0, 2.0, "reverb").unwrap()),
            ..Automation::default()
        }) - dry(Automation::default()))
        .abs()
            < 0.02
    );
}

#[test]
fn automated_scores_are_deterministic() {
    let auto = Automation {
        volume: Some(lane("0:0.2 2:1")),
        pan: Some(Lane::parse("0:-0.5 2:0.5", M44, -1.0, 1.0, "pan").unwrap()),
        cutoff: Some(Lane::parse("0:0.3 2:2", M44, 0.05, 8.0, "cutoff").unwrap()),
        reverb: Some(Lane::parse("0:1 1:0", M44, 0.0, 2.0, "reverb").unwrap()),
        ..Automation::default()
    };
    let mix = Mix {
        reverb: 0.3,
        ..Mix::default()
    };
    let a = render(&long_note("saw_lead", auto.clone()), &mix).to_wav();
    assert_eq!(a, render(&long_note("saw_lead", auto), &mix).to_wav());
}

#[test]
fn no_lanes_means_the_old_sound() {
    // an empty `Automation` is exactly what tracks had before lanes existed
    assert!(Automation::default().is_empty());
    let s = long_note("saw_lead", Automation::default());
    let mix = Mix {
        reverb: 0.2,
        delay: 0.1,
        ..Mix::default()
    };
    assert_eq!(render(&s, &mix).to_wav(), render(&s, &mix).to_wav());
}
