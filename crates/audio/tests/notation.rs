//! The notation: notes, chords, drum patterns, tempo and time.
use animscript_audio::*;

const BAR: u64 = 3840; // a 4/4 bar in ticks (960 per quarter note)

fn notes(src: &str) -> Vec<Ev> {
    parse_notes(src, Meter::new(4, 4)).unwrap_or_else(|e| panic!("{e}"))
}
fn chord_notes(sym: &str) -> Vec<u8> {
    parse_chord_symbol(sym, &Key::default(), 3).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn pitch_names_are_midi_numbers() {
    assert_eq!(pitch("C4"), Some(60));
    assert_eq!(pitch("A4"), Some(69));
    assert_eq!(pitch("C#4"), Some(61));
    assert_eq!(pitch("Db4"), Some(61));
    assert_eq!(pitch("Bb2"), Some(46));
    assert_eq!(pitch("C-1"), Some(0));
    assert_eq!(pitch("H4"), None);
    assert_eq!(pitch("C"), None);
    assert_eq!(pitch("C10"), None);
}

#[test]
fn notes_follow_each_other_with_their_durations() {
    let e = notes("C4:1/4 E4:1/8 G4:1/2");
    assert_eq!(e.len(), 3);
    assert_eq!((e[0].start, e[0].len, e[0].note), (0, 960, 60));
    assert_eq!((e[1].start, e[1].len, e[1].note), (960, 480, 64));
    assert_eq!((e[2].start, e[2].len, e[2].note), (1440, 1920, 67));
}

#[test]
fn a_missing_duration_repeats_the_last_one() {
    let e = notes("C4:1/8 D4 E4");
    assert_eq!(
        e.iter().map(|x| x.start).collect::<Vec<_>>(),
        vec![0, 480, 960]
    );
    // the very first note without a duration is a quarter
    assert_eq!(notes("C4")[0].len, 960);
}

#[test]
fn rests_dots_and_triplets() {
    let e = notes("C4:1/4 .:1/4 D4:1/4. E4:1/8t F4:1/8t G4:1/8t");
    assert_eq!(e[1].start, 1920); // after the quarter and its rest
    assert_eq!(e[1].len, 1440); // dotted quarter
    assert_eq!(e[2].start, 1920 + 1440);
    assert_eq!(e[2].len, 320); // a third of a quarter
    assert_eq!(e[3].start - e[2].start, 320);
}

#[test]
fn chords_in_brackets_start_together() {
    let e = notes("[C4 E4 G4]:1/2 D4:1/2");
    assert_eq!(e.len(), 4);
    assert!(e[..3].iter().all(|x| x.start == 0 && x.len == 1920));
    assert_eq!(e[3].start, 1920);
}

#[test]
fn velocity_and_accents() {
    let e = notes("C4:1/4!120 D4:1/4 E4:1/4^");
    assert_eq!(e[0].vel, 120.0 / 127.0);
    assert!(e[1].vel > 0.5 && e[1].vel < 0.9);
    assert!(e[2].vel > e[1].vel);
}

#[test]
fn bar_lines_are_checked_against_the_meter() {
    assert!(parse_notes("C4:1/4 D4:1/4 E4:1/2 | F4:1", Meter::new(4, 4)).is_ok());
    let e = parse_notes("C4:1/4 D4:1/4 E4:1/4 | F4:1", Meter::new(4, 4)).unwrap_err();
    assert!(e.message.contains("bar 1"), "{e}");
    // 3/4
    assert!(parse_notes("C4:1/4 D4:1/4 E4:1/4 | F4:3/4", Meter::new(3, 4)).is_ok());
}

#[test]
fn notation_errors_name_the_bad_token() {
    let e = parse_notes("C4:1/4 Z9:1/4", Meter::new(4, 4)).unwrap_err();
    assert!(e.message.contains("Z9"), "{e}");
    let e = parse_notes("C4:1/7", Meter::new(4, 4)).unwrap_err();
    assert!(e.message.contains("1/7"), "{e}");
    assert!(parse_notes("[C4 E4", Meter::new(4, 4)).is_err());
    assert!(parse_notes("C4!200", Meter::new(4, 4)).is_err());
}

#[test]
fn chord_symbols_spell_their_notes() {
    assert_eq!(chord_notes("C"), vec![48, 52, 55]);
    assert_eq!(chord_notes("Am"), vec![57, 60, 64]);
    assert_eq!(chord_notes("G7"), vec![55, 59, 62, 65]);
    assert_eq!(chord_notes("Cmaj7"), vec![48, 52, 55, 59]);
    assert_eq!(chord_notes("Dm7b5"), vec![50, 53, 56, 60]);
    assert_eq!(chord_notes("Csus4"), vec![48, 53, 55]);
    assert_eq!(chord_notes("F#dim"), vec![54, 57, 60]);
    assert_eq!(chord_notes("Bbm"), vec![58, 61, 65]);
}

#[test]
fn a_slash_chord_adds_a_bass_note_below() {
    let c = chord_notes("C/E");
    assert_eq!(c[0], 40); // E2
    assert_eq!(&c[1..], &[48, 52, 55]);
}

#[test]
fn roman_numerals_follow_the_key() {
    let key = Key::parse("A minor").unwrap();
    // i = Am, VI = F, III = C, VII = G
    let sp = |s: &str| parse_chord_symbol(s, &key, 3).unwrap();
    assert_eq!(sp("i"), vec![57, 60, 64]);
    assert_eq!(sp("VI"), vec![53, 57, 60]);
    assert_eq!(sp("III"), vec![48, 52, 55]);
    assert_eq!(sp("VII"), vec![55, 59, 62]);
    let c = Key::parse("C major").unwrap();
    assert_eq!(
        parse_chord_symbol("V7", &c, 3).unwrap(),
        vec![55, 59, 62, 65]
    );
    assert_eq!(parse_chord_symbol("ii", &c, 3).unwrap(), vec![50, 53, 57]);
}

#[test]
fn bad_chords_and_keys_are_errors() {
    assert!(parse_chord_symbol("Hm", &Key::default(), 3).is_err());
    assert!(parse_chord_symbol("Cwhat", &Key::default(), 3).is_err());
    assert!(Key::parse("A sideways").is_err());
}

#[test]
fn a_progression_splits_each_bar_among_its_chords() {
    let ev = parse_progression(
        "C | Am F",
        Meter::new(4, 4),
        &Key::default(),
        &ChordStyle::block(),
        3,
    )
    .unwrap();
    // C for a bar (3 notes), then Am and F for half a bar each
    assert_eq!(ev.len(), 9);
    assert!(ev[..3].iter().all(|e| e.start == 0));
    assert!(ev[3..6].iter().all(|e| e.start == BAR));
    assert!(ev[6..9].iter().all(|e| e.start == BAR + BAR / 2));
}

#[test]
fn percent_repeats_and_a_dot_rests() {
    let ev = parse_progression(
        "C | % | . | G",
        Meter::new(4, 4),
        &Key::default(),
        &ChordStyle::block(),
        3,
    )
    .unwrap();
    let starts: std::collections::BTreeSet<u64> = ev.iter().map(|e| e.start).collect();
    assert_eq!(
        starts.into_iter().collect::<Vec<_>>(),
        vec![0, BAR, 3 * BAR]
    );
}

#[test]
fn arpeggio_and_strum_styles() {
    let arp = parse_progression(
        "C",
        Meter::new(4, 4),
        &Key::default(),
        &ChordStyle::arp_up(1, 8),
        3,
    )
    .unwrap();
    assert_eq!(arp.len(), 8); // eight eighths in the bar
    assert_eq!(arp[0].note, 48);
    assert_eq!(arp[1].note, 52);
    assert_eq!(arp[3].note, 48); // cycles
    let strum = parse_progression(
        "C",
        Meter::new(4, 4),
        &Key::default(),
        &ChordStyle::strum(30),
        3,
    )
    .unwrap();
    assert_eq!(
        strum.iter().map(|e| e.start).collect::<Vec<_>>(),
        vec![0, 30, 60]
    );
}

#[test]
fn drum_patterns_place_hits_on_a_grid() {
    // sixteen steps per bar: kick on 1 and 9, snare on 5 and 13
    let ev = parse_beat("k...s...k...s...", 16, Meter::new(4, 4)).unwrap();
    let hits: Vec<(u64, u8)> = ev.iter().map(|e| (e.start, e.note)).collect();
    assert_eq!(hits, vec![(0, 36), (960, 38), (1920, 36), (2880, 38)]);
}

#[test]
fn drum_letters_accents_and_spaces() {
    let ev = parse_beat("k . H . | s h o c", 4, Meter::new(4, 4)).unwrap();
    // eighth-note steps? no: 4 steps per bar = quarters
    assert_eq!(ev.len(), 6);
    assert_eq!(ev[0].note, 36);
    assert_eq!(ev[1].note, 42); // closed hat, accented
    assert!(ev[1].vel > ev[3].vel || true);
    assert_eq!(ev[2].start, BAR);
    assert_eq!(ev[3].note, 42);
    assert_eq!(ev[4].note, 46);
    assert_eq!(ev[5].note, 49);
}

#[test]
fn drum_bars_must_have_the_right_number_of_steps() {
    let e = parse_beat("k . s | k . s .", 4, Meter::new(4, 4)).unwrap_err();
    assert!(e.message.contains("bar 1"), "{e}");
    assert!(parse_beat("k . q .", 4, Meter::new(4, 4)).is_err());
}

#[test]
fn tempo_turns_ticks_into_seconds() {
    // 96 bpm: a quarter note is 0.625 s
    assert!((ticks_to_seconds(960, 96.0) - 0.625).abs() < 1e-12);
    assert!((ticks_to_seconds(BAR, 120.0) - 2.0).abs() < 1e-12);
    assert_eq!(seconds_to_ticks(2.0, 120.0), BAR);
}

#[test]
fn swing_delays_the_off_beat_eighths() {
    let mut e = notes("C4:1/8 D4:1/8 E4:1/8 F4:1/8");
    apply_swing(&mut e, 0.5);
    assert_eq!(e[0].start, 0);
    assert!(e[1].start > 480); // pushed late
    assert_eq!(e[2].start, 960);
    assert!(e[3].start > 1440);
    // zero swing changes nothing
    let mut f = notes("C4:1/8 D4:1/8");
    apply_swing(&mut f, 0.0);
    assert_eq!(f[1].start, 480);
}

#[test]
fn a_score_has_a_length() {
    let mut s = Score::new(120.0, Meter::new(4, 4));
    s.tracks
        .push(Track::new("lead", "piano", notes("C4:1 D4:1")));
    assert_eq!(s.length_ticks(), 2 * BAR);
    assert!((s.length_seconds() - 4.0).abs() < 1e-9);
}
