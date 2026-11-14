//! Scores in the language: `score` blocks, `play` and `stop`, errors.
use animscript_lang::*;

const HEAD: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 640x360\n    fps: 24\n\n";

fn go(s: &str) -> RunResult {
    let text = format!("{HEAD}{s}");
    match run(&text, &RunOptions::default()) {
        Ok(r) => r,
        Err(d) => panic!("{}", d.render(Some(&text), "test")),
    }
}
fn err(s: &str) -> Box<Diagnostic> {
    match run(&format!("{HEAD}{s}"), &RunOptions::default()) {
        Ok(_) => panic!("expected an error"),
        Err(d) => Box::new(d),
    }
}

const THEME: &str = r#"score theme:
    tempo: 120, meter: "4/4", key: "A minor", reverb: 0.25, swing: 0.2
    track pad: instrument: "pad", chords: "i | VI | III | VII", volume: 0.6, pan: -0.3
    track bass: instrument: "synth_bass", notes: "A2:1/2 E2:1/2 | F2:1", transpose: 0
    track drums: instrument: "drumkit", beat: "k.s.k.s.", steps: 8, repeat: 4
    track lead: instrument: "violin", chords: ["Am | F"], style: "arp_up", rate: "1/8", bars: 4
"#;

fn scene(body: &str) -> String {
    format!("{THEME}scene \"s\" for 8.seconds:\n    main:\n{body}        wait 8.seconds\n")
}

#[test]
fn a_score_is_built_from_its_block() {
    let r = go(&scene(""));
    let def = &r.audio.scores["theme"];
    assert_eq!(def.score.tempo, 120.0);
    assert_eq!((def.score.meter.num, def.score.meter.den), (4, 4));
    assert_eq!(def.score.tracks.len(), 4);
    assert_eq!(def.score.tracks[0].instrument, "pad");
    assert_eq!(def.score.tracks[0].pan, -0.3);
    // i VI III VII in A minor: Am, F, C, G (three notes each)
    assert_eq!(def.score.tracks[0].events.len(), 12);
    assert_eq!(def.score.tracks[0].events[0].note, 57);
    assert_eq!(def.mix.reverb, 0.25);
    assert_eq!(def.score.swing, 0.2);
}

#[test]
fn repeat_and_offsets_place_the_events() {
    let r = go(&scene(""));
    let drums = &r.audio.scores["theme"].score.tracks[2];
    // a one-bar pattern of 4 hits repeated 4 times
    assert_eq!(drums.events.len(), 16);
    assert_eq!(drums.events[4].start, 3840);
    // `bars: 4` starts the lead four bars in
    let lead = &r.audio.scores["theme"].score.tracks[3];
    assert_eq!(lead.events[0].start, 4 * 3840);
}

#[test]
fn play_and_stop_become_cues_on_the_timeline() {
    let r = go(&scene(
        "        wait 1.second\n        play(theme, volume: 0.8, fade_in: 0.5.seconds, loop: true)\n        wait 3.seconds\n        stop(theme, fade_out: 1.second)\n",
    ));
    assert_eq!(r.audio.cues.len(), 1);
    let c = &r.audio.cues[0];
    assert_eq!(c.score, "theme");
    assert_eq!(c.start_tick, 24);
    assert_eq!(c.volume, 0.8);
    assert_eq!(c.fade_in, 0.5);
    assert!(c.looping);
    assert_eq!(c.stop_tick, Some(24 * 4));
    assert_eq!(c.fade_out, 1.0);
    assert_eq!(r.audio.fps, 24.0);
    assert_eq!(r.audio.ticks, r.snapshots.len());
}

#[test]
fn the_method_form_and_a_second_play_work() {
    let r = go(&scene("        theme.play()\n        wait 2.seconds\n        theme.play(volume: 0.5, offset: 1.second)\n"));
    assert_eq!(r.audio.cues.len(), 2);
    assert_eq!(r.audio.cues[1].start_tick, 48);
    assert_eq!(r.audio.cues[1].offset, 1.0);
}

#[test]
fn scenes_continue_the_project_clock() {
    let src = format!(
        "{THEME}scene \"a\" for 2.seconds:\n    main:\n        wait 2.seconds\nscene \"b\" for 2.seconds:\n    main:\n        wait 1.second\n        play(theme)\n        wait 1.second\n"
    );
    let r = go(&src);
    assert_eq!(r.audio.cues[0].start_tick, 48 + 24);
}

#[test]
fn notation_errors_name_the_track_and_the_problem() {
    let e =
        err("score s:\n    tempo: 100\n    track t: instrument: \"sine\", notes: \"C4:1/4 Z9\"\n");
    assert_eq!(e.code, "E-AUD-002");
    assert!(
        e.message.contains("track `t`") && e.message.contains("Z9"),
        "{}",
        e.message
    );
    let e = err("score s:\n    tempo: 100, meter: \"4/4\"\n    track t: instrument: \"sine\", chords: \"C | Hm\"\n");
    assert_eq!(e.code, "E-AUD-002");
}

#[test]
fn settings_are_checked() {
    assert_eq!(err("score s:\n    tempo: 0\n").code, "E-AUD-001");
    assert_eq!(err("score s:\n    bogus: 1\n").code, "E-AUD-001");
    assert_eq!(err("score s:\n    key: \"A sideways\"\n").code, "E-AUD-001");
    assert_eq!(err("score s:\n    meter: \"4-4\"\n").code, "E-AUD-001");
    assert_eq!(
        err("score s:\n    track t: notes: \"C4\", chords: \"C\"\n").code,
        "E-AUD-001"
    );
    assert_eq!(
        err("score s:\n    track t: instrument: \"sine\", bogus: 1\n").code,
        "E-AUD-001"
    );
    assert_eq!(
        err("score s:\n    track t: instrument: \"sine\"\n").code,
        "E-AUD-001"
    ); // nothing to play
    assert_eq!(
        err("score s:\n    tempo: 90\nscore s:\n    tempo: 91\n").code,
        "E-AUD-001"
    );
}

#[test]
fn unknown_instruments_are_reported_when_the_score_is_built() {
    let e = err("score s:\n    track t: instrument: \"kazoo\", notes: \"C4\"\n");
    assert_eq!(e.code, "E-AUD-003");
    assert!(e.message.contains("kazoo"), "{}", e.message);
}

#[test]
fn play_needs_a_score_and_a_running_scene() {
    let e = err(&format!(
        "{THEME}scene \"s\" for 1.second:\n    main:\n        play(3)\n        wait 1.second\n"
    ));
    assert_eq!(e.code, "E-AUD-004");
    let e = err(&format!(
        "{THEME}play(theme)\nscene \"s\" for 1.second:\n    main:\n        wait 1.second\n"
    ));
    assert_eq!(e.code, "E-AUD-004");
    let e = err(&format!("{THEME}scene \"s\" for 1.second:\n    main:\n        play(theme, volume: -1)\n        wait 1.second\n"));
    assert_eq!(e.code, "E-AUD-004");
    let e = err(&format!(
        "{THEME}scene \"s\" for 1.second:\n    main:\n        stop(theme)\n        wait 1.second\n"
    ));
    assert_eq!(e.code, "E-AUD-004");
}

#[test]
fn a_program_without_scores_has_no_audio() {
    let r = go("scene \"s\" for 1.second:\n    main:\n        wait 1.second\n");
    assert!(r.audio.scores.is_empty() && r.audio.cues.is_empty());
}

#[test]
fn the_same_program_gives_the_same_audio_description() {
    let src = scene("        play(theme)\n");
    let (a, b) = (go(&src), go(&src));
    assert_eq!(a.audio.cues.len(), b.audio.cues.len());
    assert_eq!(
        a.audio.scores["theme"].score.tracks[0].events,
        b.audio.scores["theme"].score.tracks[0].events
    );
}

#[test]
fn the_project_soundtrack_follows_the_cues() {
    let src = scene("        wait 2.seconds\n        play(theme, volume: 0.7)\n        wait 2.seconds\n        stop(theme, fade_out: 0.5.seconds)\n");
    let r = go(&src);
    let st = animscript_lang::audio::render_project_audio(
        &r.audio,
        std::path::Path::new("."),
        Some(22050),
    )
    .unwrap();
    // eight seconds long (the scene), silent before the play at 2 s and again well after the stop
    assert!((st.seconds() - 8.0).abs() < 0.1, "{}", st.seconds());
    assert!(st.rms(0.0, 1.9) < 1e-6);
    assert!(st.rms(2.3, 3.8) > 0.01);
    assert!(st.rms(4.6, 7.9) < 1e-6);
    // the same program renders the same bytes
    let again = animscript_lang::audio::render_project_audio(
        &go(&src).audio,
        std::path::Path::new("."),
        Some(22050),
    )
    .unwrap();
    assert_eq!(st.to_wav(), again.to_wav());
}

#[test]
fn no_play_means_no_soundtrack() {
    let r = go(&scene(""));
    let e = animscript_lang::audio::render_project_audio(&r.audio, std::path::Path::new("."), None)
        .err()
        .unwrap();
    assert!(e.contains("play"), "{e}");
}

#[test]
fn a_missing_sample_library_is_reported_when_rendering() {
    let src = "score s:\n    library: \"nope\"\n    track t: instrument: \"harp_sampled\", notes: \"C4\"\nscene \"x\" for 1.second:\n    main:\n        play(s)\n        wait 1.second\n";
    let r = go(src);
    let e = animscript_lang::audio::render_project_audio(&r.audio, std::path::Path::new("."), None)
        .err()
        .unwrap();
    assert!(e.contains("harp_sampled"), "{e}");
}

// ---- music-driven animation

fn shape_x(r: &RunResult, tick: usize, id: &str) -> f64 {
    r.snapshots[tick]["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap()["transform"]["position"][0]
        .as_f64()
        .unwrap()
}

const SYNC: &str = r#"score groove:
    tempo: 120
    track kick: instrument: "drumkit", beat: "k...k...", steps: 8, repeat: 8
    track pad: instrument: "pad", chords: "C | F", repeat: 4
shape probe = circle(n: 12, radius: 5, at: (0, 50))
shape counter = circle(n: 12, radius: 5, at: (0, 100))
shape p2 = circle(n: 12, radius: 5, at: (0, 150))
shape p3 = circle(n: 12, radius: 5, at: (0, 200))
"#;

fn sync_scene(body: &str) -> String {
    format!("{SYNC}scene \"s\" for 6.seconds:\n    main:\n{body}        wait 6.seconds\n")
}

#[test]
fn a_score_reports_whether_it_plays_and_how_far_in_it_is() {
    let r = go(&sync_scene(
        "        probe.move_to((groove.time * 100, 50))\n        wait 1.second\n        play(groove)\n        wait 12.frames\n        if groove.playing:\n            probe.move_to((groove.time * 100, 90))\n        wait 1.second\n        stop(groove)\n        wait 2.frames\n        if groove.playing:\n            probe.move_to((0, 200))\n",
    ));
    // before the play: time 0
    assert_eq!(shape_x(&r, 5, "probe"), 0.0);
    // half a second in (12 ticks at 24 fps) and playing
    let x = shape_x(&r, 24 + 12 + 1, "probe");
    assert!((x - 50.0).abs() < 1.0, "{x}");
    // after the stop it no longer plays, so the last move never happens
    let end = shape_x(&r, 24 + 12 + 24 + 4, "probe");
    assert!((end - 50.0).abs() < 1.0, "{end}");
}

#[test]
fn beats_and_bars_are_events_that_fire_for_one_tick() {
    let r = go(
        &sync_scene("        play(groove)\n        wait 6.seconds\n").replace(
            "    main:",
            "    whenever groove.beat_now:\n        counter.move_by(1, 0)\n    main:",
        ),
    );
    // 120 bpm: a beat every 0.5 s, so 12 beats in 6 s (the first at the very start)
    let n = shape_x(&r, r.snapshots.len() - 1, "counter");
    assert!((11.0..=13.0).contains(&n), "{n}");
}

#[test]
fn pulse_and_phase_follow_the_beat() {
    let r = go(&sync_scene(
        "        play(groove)\n        wait 1.frames\n        probe.move_to((groove.pulse * 1000, 50))\n        wait 5.frames\n        p2.move_to((groove.pulse * 1000, 150))\n        wait 1.frames\n        p3.move_to((groove.beat * 1000, 200))\n",
    ));
    let last = r.snapshots.len() - 1;
    let (early, later, phase) = (
        shape_x(&r, last, "probe"),
        shape_x(&r, last, "p2"),
        shape_x(&r, last, "p3"),
    );
    // just after a beat the pulse is high; half a beat later it has faded; the phase counts up through the beat
    assert!(early > 600.0, "{early}");
    assert!(later < early / 2.0, "{later} {early}");
    assert!((phase - 500.0).abs() < 150.0, "{phase}");
}

#[test]
fn a_track_has_a_level_that_jumps_on_its_notes() {
    let r = go(&sync_scene(
        "        play(groove)\n        wait 1.frames\n        probe.move_to((groove.kick * 1000, 50))\n        wait 10.frames\n        p2.move_to((groove.kick * 1000, 150))\n",
    ));
    let last = r.snapshots.len() - 1;
    let (hit, later) = (shape_x(&r, last, "probe"), shape_x(&r, last, "p2"));
    assert!(hit > 300.0, "{hit}");
    assert!(later < hit * 0.6, "{later}");
}

#[test]
fn unknown_score_members_are_errors() {
    let e = err(&sync_scene("        probe.move_to((groove.bogus, 0))\n"));
    assert_eq!(e.code, "E-NAM-003");
    assert!(e.message.contains("bogus"));
}
