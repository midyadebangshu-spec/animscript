//! Per-note control in the notation: articulation, slurs, glissando, grace notes, detune, dynamics, hairpins,
//! and the seeded humaniser.
use animscript_audio::*;

fn notes(src: &str) -> Vec<Ev> {
    parse_notes(src, Meter::new(4, 4)).unwrap_or_else(|e| panic!("{e}"))
}
fn err(src: &str) -> String {
    parse_notes(src, Meter::new(4, 4)).unwrap_err().message
}

#[test]
fn staccato_tenuto_and_gates_set_how_long_a_note_is_held() {
    let e = notes("C4:1/4' D4:1/4_ E4:1/4@0.7 F4:1/4");
    assert_eq!(
        e.iter().map(|x| x.gate).collect::<Vec<_>>(),
        vec![0.5, 1.0, 0.7, 1.0]
    );
    // the written length (what the next note waits for) is unchanged
    assert_eq!(
        e.iter().map(|x| x.start).collect::<Vec<_>>(),
        vec![0, 960, 1920, 2880]
    );
    assert!(e.iter().all(|x| x.len == 960));
}

#[test]
fn marks_combine_in_any_order() {
    let e = notes("C4:1/4'^!100 D4:1/8@0.3+20c");
    assert_eq!((e[0].gate, e[0].vel), (0.5, 100.0 / 127.0));
    assert_eq!((e[1].gate, e[1].cents, e[1].len), (0.3, 20.0, 480));
    let e = notes("C4'-35c:1/2");
    assert_eq!((e[0].gate, e[0].cents, e[0].len), (0.5, -35.0, 1920));
}

#[test]
fn bad_gates_and_detunes_are_errors() {
    assert!(err("C4@0").contains("gate"));
    assert!(err("C4@2").contains("gate"));
    assert!(err("C4@x").contains("gate"));
    assert!(err("C4+150c").contains("100"));
    assert!(err("C4+20").contains("cents"));
    assert!(err("C4?").contains("unexpected"));
}

#[test]
fn a_slur_connects_all_but_its_last_note() {
    let e = notes("(C4:1/4 D4 E4) F4:1/4");
    assert_eq!(
        e.iter().map(|x| x.legato).collect::<Vec<_>>(),
        vec![true, true, false, false]
    );
    // a chord group inside a slur keeps its notes together
    let e = notes("([C4 E4]:1/4 G4:1/4) A4");
    assert_eq!(
        e.iter().map(|x| x.legato).collect::<Vec<_>>(),
        vec![true, true, false, false]
    );
    // two slurs
    let e = notes("(C4 D4) (E4 F4)");
    assert_eq!(
        e.iter().map(|x| x.legato).collect::<Vec<_>>(),
        vec![true, false, true, false]
    );
}

#[test]
fn slur_errors() {
    assert!(err("(C4 D4").contains("never closed"));
    assert!(err("C4 D4)").contains("never opened"));
    assert!(err("(C4 (D4 E4))").contains("nested"));
}

#[test]
fn a_glissando_is_one_note_that_slides() {
    let e = notes("C4~E4:1/2 G4:1/2");
    assert_eq!(e.len(), 2);
    assert_eq!((e[0].note, e[0].slide, e[0].len), (60, Some(64), 1920));
    assert_eq!(e[1].start, 1920);
    assert!(err("C4~").contains("second pitch"));
    assert!(err("C4~Q9").contains("second pitch"));
}

#[test]
fn a_grace_note_takes_its_time_from_the_note_it_leads_into() {
    let e = notes("{D4}C4:1/4 E4:1/4");
    assert_eq!(e.len(), 3);
    assert_eq!((e[0].note, e[0].start, e[0].len), (62, 0, 120));
    assert!(e[0].vel < e[1].vel);
    assert_eq!((e[1].note, e[1].start, e[1].len), (60, 120, 840));
    assert_eq!(e[2].start, 960); // the bar time is unchanged
    assert!(err("{D4}.:1/4").contains("rest"));
    assert!(err("{D4 C4").contains("closed"));
}

#[test]
fn dynamic_marks_set_the_velocity_that_follows() {
    let e = notes("C4 p D4 D4 f E4 ff F4 pp G4");
    let v: Vec<f64> = e.iter().map(|x| x.vel).collect();
    assert_eq!(v, vec![0.7, 0.4, 0.4, 0.85, 0.98, 0.25]);
    // an accent is louder than the level, an explicit velocity wins
    let e = notes("p C4^ C4!100");
    assert!((e[0].vel - 0.6).abs() < 1e-12);
    assert_eq!(e[1].vel, 100.0 / 127.0);
    assert_eq!(dynamic("mf"), Some(0.7));
    assert_eq!(dynamic("x"), None);
}

#[test]
fn a_crescendo_ramps_the_notes_between_its_marks() {
    let e = notes("mp C4 < C4 C4 C4 f C4");
    let v: Vec<f64> = e.iter().map(|x| x.vel).collect();
    assert_eq!(v[0], 0.55);
    assert!((v[1] - 0.55).abs() < 1e-12);
    assert!((v[2] - 0.65).abs() < 1e-12);
    assert!((v[3] - 0.75).abs() < 1e-12);
    assert_eq!(v[4], 0.85);
    // a decrescendo goes down
    let e = notes("f > C4 C4 p C4");
    assert!(e[0].vel > e[1].vel && e[1].vel > 0.4);
    assert_eq!(e[2].vel, 0.4);
}

#[test]
fn hairpin_errors() {
    assert!(err("mp < C4 C4").contains("dynamic mark"));
    assert!(err("f < C4 p").contains("softer"));
    assert!(err("p > C4 f").contains("louder"));
    assert!(err("p < C4 < C4 f").contains("inside"));
}

#[test]
fn an_explicit_velocity_inside_a_hairpin_is_kept() {
    let e = notes("p < C4 C4!120 C4 f");
    assert_eq!(e[1].vel, 120.0 / 127.0);
    assert!(e[0].vel < e[2].vel);
}

#[test]
fn old_notation_still_means_the_same() {
    let e = notes("C4:1/4 D4:1/8 [C4 E4]:1/2 .:1/8 | G4:1");
    assert!(e
        .iter()
        .all(|x| x.gate == 1.0 && !x.legato && x.slide.is_none() && x.cents == 0.0));
    assert_eq!(e[0].vel, 0.7);
}

#[test]
fn humanize_is_seeded_bounded_and_off_at_zero() {
    let base = notes("C4:1/4 D4 E4 F4 G4 A4 B4 C5");
    let (mut a, mut b, mut c, mut z) = (base.clone(), base.clone(), base.clone(), base.clone());
    humanize(&mut a, 0.5, 7);
    humanize(&mut b, 0.5, 7);
    humanize(&mut c, 0.5, 8);
    humanize(&mut z, 0.0, 7);
    assert_eq!(a, b, "the same seed gives the same result");
    assert_ne!(a, c, "another seed differs");
    assert_eq!(z, base);
    for (h, o) in a.iter().zip(&base) {
        assert!((h.start as i64 - o.start as i64).abs() <= 20);
        assert!((h.vel - o.vel).abs() <= 0.075 + 1e-12);
        assert_eq!(h.len, o.len);
    }
    // never before the start of time
    let mut first = base.clone();
    humanize(&mut first, 1.0, 3);
    assert!(first.iter().all(|e| e.start < 1_000_000));
}
