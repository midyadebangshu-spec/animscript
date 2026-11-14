//! What articulation does to the sound: gates, detune, glissando, velocity brightness, slurs on one voice.
use animscript_audio::synth::*;
use animscript_audio::*;

const SR: f64 = 44100.0;

fn spec(note: u8, vel: f64, hold: f64) -> NoteSpec {
    NoteSpec::plain(note, vel, hold)
}
fn render(name: &str, sp: &NoteSpec) -> Vec<f32> {
    render_note_spec(&builtin(name).unwrap(), sp, SR, 1)
}
fn rate_of(x: &[f32]) -> f64 {
    x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f64 / (x.len() as f64 / SR)
}
fn window(x: &[f32], from: f64, to: f64) -> &[f32] {
    let end = ((to * SR) as usize).min(x.len());
    let start = ((from * SR) as usize).min(end);
    &x[start..end]
}
fn energy(x: &[f32]) -> f64 {
    x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64
}
fn rough(x: &[f32]) -> f64 {
    let d: f64 = x
        .windows(3)
        .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs() as f64)
        .sum();
    let a: f64 = x.iter().map(|v| v.abs() as f64).sum();
    d / a.max(1e-12)
}

#[test]
fn a_shorter_gate_releases_the_note_earlier() {
    // one second written, held half of it: silent well before the end
    let full = render("sine", &spec(69, 1.0, 1.0));
    let half = render("sine", &spec(69, 1.0, 0.5));
    assert!(energy(window(&half, 0.75, 1.0)) < 1e-6);
    assert!(energy(window(&full, 0.75, 0.95)) > 0.01);
}

#[test]
fn detune_moves_the_pitch_by_cents() {
    let sp = NoteSpec {
        cents: 100.0,
        ..spec(69, 1.0, 1.0)
    };
    let up = rate_of(window(&render("sine", &sp), 0.1, 0.9));
    assert!((up - 466.16).abs() < 4.0, "{up}");
    let sp = NoteSpec {
        cents: -50.0,
        ..spec(69, 1.0, 1.0)
    };
    let down = rate_of(window(&render("sine", &sp), 0.1, 0.9));
    assert!((down - 427.5).abs() < 4.0, "{down}");
}

#[test]
fn a_glissando_slides_through_the_pitches() {
    let sp = NoteSpec {
        slide: Some(81),
        ..spec(69, 1.0, 1.0)
    }; // A4 to A5 in a second
    let x = render("sine", &sp);
    let (early, mid, late) = (
        rate_of(window(&x, 0.0, 0.1)),
        rate_of(window(&x, 0.45, 0.55)),
        rate_of(window(&x, 0.85, 0.95)),
    );
    assert!(early > 430.0 && early < 520.0, "{early}");
    assert!(mid > 580.0 && mid < 700.0, "{mid}"); // a half way (an octave over a second): about 622 Hz
    assert!(late > 780.0, "{late}");
    // FM and additive voices slide too
    let y = render("organ", &sp);
    assert!(rate_of(window(&y, 0.8, 0.95)) > rate_of(window(&y, 0.0, 0.15)) * 1.3);
}

#[test]
fn a_harder_note_is_brighter_not_just_louder() {
    let hard = render("saw_lead", &spec(57, 1.0, 0.6));
    let soft = render("saw_lead", &spec(57, 0.35, 0.6));
    assert!(
        rough(&hard) > rough(&soft) * 1.15,
        "{} {}",
        rough(&hard),
        rough(&soft)
    );
    let (fh, fs) = (
        render("bell", &spec(72, 1.0, 0.5)),
        render("bell", &spec(72, 0.35, 0.5)),
    );
    assert!(rough(&fh) > rough(&fs), "FM index follows velocity");
}

fn phrase(inst: &str, legato: bool) -> Vec<f32> {
    // A3, C4, E4 a quarter each at 120 bpm (0.5 s)
    let mut ev = parse_notes(
        if legato {
            "(A3:1/4 C4 E4)"
        } else {
            "A3:1/4' C4' E4'"
        },
        Meter::new(4, 4),
    )
    .unwrap();
    if !legato {
        for e in ev.iter_mut() {
            e.gate = 0.5;
        }
    }
    let mut s = Score::new(120.0, Meter::new(4, 4));
    s.tracks.push(Track::new("t", inst, ev));
    let st = render_score(&s, &Mix::default(), &Bank::new()).unwrap();
    st.left
}

#[test]
fn a_slur_is_one_voice_without_a_gap_between_its_notes() {
    let slurred = phrase("saw_lead", true);
    let detached = phrase("saw_lead", false);
    // around the first boundary (0.5 s) the slurred sound carries on, the detached one has dipped
    let (a, b) = (
        energy(window(&slurred, 0.46, 0.5)),
        energy(window(&detached, 0.46, 0.5)),
    );
    assert!(a > b * 20.0 && a > 1e-4, "{a} {b}");
}

#[test]
fn slurred_notes_glide_between_pitches() {
    let x = phrase("sine", true);
    // the notes are A3 220 Hz, then C4 261.6 Hz: halfway through the glide the pitch is between them
    let before = rate_of(window(&x, 0.3, 0.48));
    let after = rate_of(window(&x, 0.55, 0.7));
    assert!((before - 220.0).abs() < 15.0, "{before}");
    assert!((after - 261.6).abs() < 15.0, "{after}");
    let during = rate_of(window(&x, 0.5, 0.54));
    assert!(during > before - 5.0 && during < after + 30.0, "{during}");
}

#[test]
fn slurs_on_instruments_that_cannot_glide_still_play_every_note() {
    // plucks and samples retrigger: three notes, three attacks
    let x = phrase("pluck", true);
    assert!(
        energy(window(&x, 0.0, 0.1)) > 1e-5
            && energy(window(&x, 0.5, 0.6)) > 1e-5
            && energy(window(&x, 1.0, 1.1)) > 1e-5
    );
}

#[test]
fn articulated_scores_are_deterministic_and_different_from_plain_ones() {
    let render_with = |src: &str| {
        let mut s = Score::new(100.0, Meter::new(4, 4));
        s.tracks.push(Track::new(
            "t",
            "violin",
            parse_notes(src, Meter::new(4, 4)).unwrap(),
        ));
        render_score(&s, &Mix::default(), &Bank::new())
            .unwrap()
            .to_wav()
    };
    let a = render_with("(A3:1/4 C4 E4) {D5}A4~E5:1/4+10c");
    assert_eq!(a, render_with("(A3:1/4 C4 E4) {D5}A4~E5:1/4+10c"));
    assert_ne!(a, render_with("A3:1/4 C4 E4 {D5}A4~E5:1/4+10c"));
    assert_ne!(render_with("C4:1/4'"), render_with("C4:1/4"));
}

#[test]
fn a_sample_can_be_detuned() {
    use animscript_audio::sampler::*;
    let n = 22050;
    let data: Vec<f32> = (0..n)
        .map(|i| (i as f64 / 22050.0 * 220.0 * std::f64::consts::TAU).sin() as f32)
        .collect();
    let z = Zone {
        lo_key: 0,
        hi_key: 127,
        lo_vel: 0,
        hi_vel: 127,
        root: 57,
        rate: 22050,
        data: std::sync::Arc::new(data),
        loop_range: None,
        gain: 1.0,
    };
    let set = SampleSet::new(vec![z]);
    let plain = set.render_pitched(57, 1.0, 0.5, 44100.0, 1.0, None);
    let up = set.render_pitched(57, 1.0, 0.5, 44100.0, 2f64.powf(1.0 / 12.0), None);
    assert!(rate_of(&up[..20000]) > rate_of(&plain[..20000]) * 1.04);
    let sl = set.render_pitched(57, 1.0, 0.5, 44100.0, 1.0, Some(12.0));
    assert!(rate_of(&sl[10000..14000]) > rate_of(&sl[..4000]) * 1.2);
}
