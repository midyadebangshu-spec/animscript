//! The synth, the mixer and the WAV writer.
use animscript_audio::synth::*;
use animscript_audio::wav::*;
use animscript_audio::*;

const SR: f64 = 44100.0;

fn tone(name: &str, note: u8, vel: f64, len: f64) -> Vec<f32> {
    render_note(&builtin(name).unwrap(), note, vel, len, SR, 1)
}
fn zero_crossings(x: &[f32]) -> usize {
    x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
}
fn rough(x: &[f32]) -> f64 {
    // how much the signal wiggles: the mean absolute difference between neighbours, relative to its level
    let d: f64 = x
        .windows(3)
        .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs() as f64)
        .sum();
    let a: f64 = x.iter().map(|v| v.abs() as f64).sum();
    d / a.max(1e-12)
}
fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, v| m.max(v.abs()))
}

#[test]
fn the_envelope_attacks_decays_sustains_and_releases() {
    let e = Adsr::new(0.1, 0.2, 0.5, 0.4);
    assert_eq!(e.level(0.0, 1.0), 0.0);
    assert!((e.level(0.05, 1.0) - 0.5).abs() < 1e-12);
    assert!((e.level(0.1, 1.0) - 1.0).abs() < 1e-12);
    assert!((e.level(0.2, 1.0) - 0.75).abs() < 1e-12);
    assert!((e.level(0.5, 1.0) - 0.5).abs() < 1e-12);
    // release from the sustain level, linearly to zero
    assert!((e.level(1.2, 1.0) - 0.25).abs() < 1e-12);
    assert_eq!(e.level(1.5, 1.0), 0.0);
    // a note released during its attack releases from where it got to
    assert!((e.level(0.1, 0.05) - 0.5 * (1.0 - 0.05 / 0.4)).abs() < 1e-12);
}

#[test]
fn a_sine_has_the_pitch_it_was_asked_for() {
    let x = tone("sine", 69, 1.0, 1.0); // A4 = 440 Hz
    let c = zero_crossings(&x[..SR as usize]);
    assert!((c as i32 - 440).abs() <= 2, "{c}");
    let c = zero_crossings(&tone("sine", 81, 1.0, 1.0)[..SR as usize]); // A5
    assert!((c as i32 - 880).abs() <= 3, "{c}");
}

#[test]
fn velocity_sets_the_loudness_and_the_release_ends_in_silence() {
    let (loud, soft) = (
        peak(&tone("sine", 60, 1.0, 0.5)),
        peak(&tone("sine", 60, 0.3, 0.5)),
    );
    assert!(loud > soft * 2.0, "{loud} {soft}");
    let x = tone("sine", 60, 1.0, 0.5);
    assert!(x.len() >= (0.5 * SR) as usize);
    assert!(x[x.len() - 20..].iter().all(|v| v.abs() < 1e-3));
}

#[test]
fn a_saw_is_brighter_than_a_sine_and_the_filter_darkens_it() {
    let sine = tone("sine", 57, 1.0, 0.5);
    let lead = tone("saw_lead", 57, 1.0, 0.5);
    assert!(
        rough(&lead) > rough(&sine) * 2.0,
        "{} {}",
        rough(&lead),
        rough(&sine)
    );
    // a pad's filter is lower than the lead's
    let pad = tone("pad", 57, 1.0, 1.0);
    assert!(
        rough(&pad) < rough(&lead),
        "{} {}",
        rough(&pad),
        rough(&lead)
    );
}

#[test]
fn every_builtin_instrument_makes_finite_sound() {
    for name in BUILTIN_NAMES {
        for note in [36u8, 60, 84] {
            let x = tone(name, note, 0.8, 0.3);
            assert!(!x.is_empty(), "{name} {note}");
            assert!(x.iter().all(|v| v.is_finite()), "{name} {note}");
            assert!(peak(&x) > 0.001, "{name} {note} is silent");
            assert!(peak(&x) < 2.0, "{name} {note} peaks at {}", peak(&x));
        }
    }
}

#[test]
fn drums_sound_like_what_they_are() {
    let kick = tone("drumkit", 36, 1.0, 0.2);
    let hat = tone("drumkit", 42, 1.0, 0.2);
    // a kick is low (slow wiggle), a hat is bright (fast wiggle) and short
    assert!(
        rough(&hat) > rough(&kick) * 5.0,
        "{} {}",
        rough(&hat),
        rough(&kick)
    );
    assert!(hat.len() < kick.len());
    let snare = tone("drumkit", 38, 1.0, 0.2);
    assert!(rough(&snare) > rough(&kick));
}

#[test]
fn notes_are_deterministic_down_to_the_bit() {
    for name in ["pluck", "flute", "drumkit", "violin"] {
        assert_eq!(tone(name, 60, 0.7, 0.4), tone(name, 60, 0.7, 0.4), "{name}");
    }
    // a different seed makes a different noise
    let a = render_note(&builtin("pluck").unwrap(), 60, 0.7, 0.3, SR, 1);
    let b = render_note(&builtin("pluck").unwrap(), 60, 0.7, 0.3, SR, 2);
    assert_ne!(a, b);
}

#[test]
fn a_plucked_string_rings_then_dies() {
    let x = tone("pluck", 57, 1.0, 1.0);
    let early: f64 = x[1000..3000].iter().map(|v| (*v as f64).powi(2)).sum();
    let late: f64 = x[40000..42000].iter().map(|v| (*v as f64).powi(2)).sum();
    assert!(early > late * 3.0);
}

fn score_with(track: Track) -> Score {
    let mut s = Score::new(120.0, Meter::new(4, 4));
    s.tracks.push(track);
    s
}
fn one_note(instrument: &str) -> Track {
    Track::new(
        "t",
        instrument,
        parse_notes("A4:1/2", Meter::new(4, 4)).unwrap(),
    )
}

#[test]
fn a_score_renders_to_its_length_plus_a_tail() {
    let st = render_score(&score_with(one_note("sine")), &Mix::default(), &Bank::new()).unwrap();
    // half a bar at 120 bpm is 1 s, plus 0.6 s of tail
    assert!((st.seconds() - 1.6).abs() < 0.01, "{}", st.seconds());
    assert!(st.rms(0.1, 0.9) > 0.05);
    assert!(st.rms(1.4, 1.6) < 1e-3);
}

#[test]
fn panning_moves_sound_between_the_ears() {
    let at = |pan: f64| {
        let mut t = one_note("sine");
        t.pan = pan;
        render_score(&score_with(t), &Mix::default(), &Bank::new()).unwrap()
    };
    let (l, c, r) = (at(-1.0), at(0.0), at(1.0));
    assert!(l.right.iter().all(|v| v.abs() < 1e-6) && l.left.iter().any(|v| v.abs() > 0.05));
    assert!(r.left.iter().all(|v| v.abs() < 1e-6) && r.right.iter().any(|v| v.abs() > 0.05));
    let (pl, pr) = (peak(&c.left), peak(&c.right));
    assert!((pl - pr).abs() < 1e-6);
}

#[test]
fn track_volume_scales_the_level() {
    let mut a = one_note("sine");
    a.volume = 0.25;
    let loud = render_score(&score_with(one_note("sine")), &Mix::default(), &Bank::new()).unwrap();
    let quiet = render_score(&score_with(a), &Mix::default(), &Bank::new()).unwrap();
    assert!(loud.rms(0.1, 0.9) > quiet.rms(0.1, 0.9) * 3.0);
}

#[test]
fn reverb_and_delay_add_a_tail() {
    let dry = render_score(&score_with(one_note("sine")), &Mix::default(), &Bank::new()).unwrap();
    let wet = render_score(
        &score_with(one_note("sine")),
        &Mix {
            reverb: 0.4,
            ..Mix::default()
        },
        &Bank::new(),
    )
    .unwrap();
    assert!(wet.rms(1.2, 2.2) > dry.rms(1.2, 2.2) + 1e-3);
    let echo = render_score(
        &score_with(one_note("sine")),
        &Mix {
            delay: 0.6,
            delay_time: 0.5,
            ..Mix::default()
        },
        &Bank::new(),
    )
    .unwrap();
    assert!(
        echo.rms(1.1, 1.5) > 1e-3,
        "the delayed copy comes after the note ends"
    );
}

#[test]
fn the_output_never_leaves_the_range() {
    let mut s = Score::new(120.0, Meter::new(4, 4));
    for k in 0..8 {
        s.tracks.push(Track::new(
            "t",
            "saw_lead",
            parse_notes(&format!("[C{} E4 G4 C5]:1", 2 + k % 3), Meter::new(4, 4)).unwrap(),
        ));
    }
    let st = render_score(
        &s,
        &Mix {
            reverb: 0.3,
            ..Mix::default()
        },
        &Bank::new(),
    )
    .unwrap();
    assert!(peak(&st.left) <= 1.0 && peak(&st.right) <= 1.0);
    assert!(st.left.iter().all(|v| v.is_finite()));
}

#[test]
fn an_unknown_instrument_is_an_error_that_lists_the_known_ones() {
    let e = render_score(
        &score_with(one_note("kazoo")),
        &Mix::default(),
        &Bank::new(),
    )
    .unwrap_err();
    assert!(e.contains("kazoo") && e.contains("piano"), "{e}");
}

#[test]
fn rendering_is_deterministic_byte_for_byte() {
    let mut s = Score::new(110.0, Meter::new(4, 4));
    s.tracks.push(Track::new(
        "a",
        "pad",
        parse_progression(
            "Am | F",
            Meter::new(4, 4),
            &Key::default(),
            &ChordStyle::block(),
            3,
        )
        .unwrap(),
    ));
    s.tracks.push(Track::new(
        "b",
        "drumkit",
        parse_beat("k.h.s.h.", 8, Meter::new(4, 4)).unwrap(),
    ));
    let mix = Mix {
        reverb: 0.2,
        delay: 0.1,
        ..Mix::default()
    };
    let a = render_score(&s, &mix, &Bank::new()).unwrap().to_wav();
    let b = render_score(&s, &mix, &Bank::new()).unwrap().to_wav();
    assert_eq!(a, b);
}

#[test]
fn the_wav_header_and_samples_are_right() {
    let bytes = write_wav16(44100, &[0.0, 1.0, -1.0, 0.5], &[0.0, 0.0, 2.0, -0.5]);
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..16], b"WAVEfmt ");
    assert_eq!(bytes.len(), 44 + 16);
    let back = read_wav(&bytes).unwrap();
    assert_eq!((back.rate, back.channels, back.frames()), (44100, 2, 4));
    assert!((back.data[2] - 1.0).abs() < 1e-3); // 1.0 -> 32767
    assert!((back.data[5] - 1.0).abs() < 1e-3, "clamped"); // 2.0 clamps to 1
    assert!((back.data[7] + 0.5).abs() < 1e-3);
}

#[test]
fn the_reader_handles_other_formats_and_rejects_junk() {
    // mono 24-bit
    let mut b = write_wav16(8000, &[0.0], &[0.0]);
    assert!(read_wav(&b).is_ok());
    assert!(read_wav(b"not a wav").is_err());
    b.truncate(20);
    assert!(read_wav(&b).is_err());
}

// ---- the mixer

use animscript_audio::mixer::*;

fn blip(rate: u32, secs: f64, level: f32) -> Stereo {
    let n = (rate as f64 * secs) as usize;
    Stereo {
        rate,
        left: vec![level; n],
        right: vec![level; n],
    }
}

#[test]
fn cues_start_at_their_time_with_their_volume() {
    let src = vec![blip(1000, 1.0, 0.5)];
    let mut c = Cue::new(0, 2.0);
    c.volume = 0.5;
    let out = mix_cues(&src, &[c], 4.0, 1000);
    assert_eq!(out.left.len(), 4000);
    assert_eq!(out.left[1999], 0.0);
    assert!((out.left[2000] - 0.25).abs() < 1e-6);
    assert!((out.left[2999] - 0.25).abs() < 1e-6);
    assert_eq!(out.left[3000], 0.0);
}

#[test]
fn fades_ramp_the_gain() {
    let src = vec![blip(1000, 2.0, 1.0)];
    let mut c = Cue::new(0, 0.0);
    c.fade_in = 1.0;
    let out = mix_cues(&src, &[c], 2.0, 1000);
    assert!((out.left[500] - 0.5).abs() < 1e-3);
    assert!((out.left[1500] - 1.0).abs() < 1e-6);
    // a stop with a fade out
    let mut c = Cue::new(0, 0.0);
    c.stop = Some(0.5);
    c.fade_out = 0.5;
    let out = mix_cues(&src, &[c], 2.0, 1000);
    assert!((out.left[400] - 1.0).abs() < 1e-6);
    assert!((out.left[750] - 0.5).abs() < 1e-3);
    assert_eq!(out.left[1100], 0.0);
}

#[test]
fn looping_repeats_until_stopped_or_the_end() {
    let src = vec![blip(1000, 0.5, 1.0)];
    let mut c = Cue::new(0, 0.0);
    c.looping = true;
    c.loop_len = 1.0;
    c.stop = Some(2.5);
    let out = mix_cues(&src, &[c], 5.0, 1000);
    assert_eq!(out.left[250], 1.0);
    assert_eq!(out.left[750], 0.0); // the gap after the half-second source
    assert_eq!(out.left[1250], 1.0); // second pass
    assert_eq!(out.left[2250], 1.0); // third pass starts at 2.0 < stop
    assert_eq!(out.left[3250], 0.0); // stopped at 2.5
}

#[test]
fn cues_add_and_the_sum_is_clamped_and_an_offset_skips_ahead() {
    let mut a = blip(1000, 1.0, 0.8);
    a.left[500] = 0.1;
    let src = vec![a, blip(1000, 1.0, 0.8)];
    let out = mix_cues(&src, &[Cue::new(0, 0.0), Cue::new(1, 0.0)], 1.0, 1000);
    assert_eq!(out.left[10], 1.0); // 1.6 clamped
    let mut c = Cue::new(0, 0.0);
    c.offset = 0.5;
    let out = mix_cues(&src, &[c], 1.0, 1000);
    assert!((out.left[0] - 0.1).abs() < 1e-6); // starts at the 0.5 s sample
}

#[test]
fn a_looped_score_has_a_whole_number_of_bars() {
    let mut s = Score::new(120.0, Meter::new(4, 4));
    s.tracks.push(Track::new(
        "t",
        "sine",
        parse_notes("A4:1/2", Meter::new(4, 4)).unwrap(),
    ));
    assert!((s.loop_seconds() - 2.0).abs() < 1e-9); // one 4/4 bar at 120 bpm
}
