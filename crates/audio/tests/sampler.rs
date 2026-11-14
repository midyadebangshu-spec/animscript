//! Sample libraries: SFZ mapping, zone selection, pitch shifting, loops.
use animscript_audio::sampler::*;
use animscript_audio::sfz::*;
use animscript_audio::wav::*;
use animscript_audio::*;
use std::sync::Arc;

const SR: u32 = 22050;

/// A mono sine of `hz` lasting `secs`, as 16-bit WAV bytes (the stereo writer with both channels equal).
fn sine_wav(hz: f64, secs: f64) -> Vec<u8> {
    let n = (SR as f64 * secs) as usize;
    let x: Vec<f32> = (0..n).map(|i| (i as f64 / SR as f64 * hz * std::f64::consts::TAU).sin() as f32 * 0.8).collect();
    write_wav16(SR, &x, &x)
}
fn crossings(x: &[f32], rate: f64) -> f64 {
    x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f64 / (x.len() as f64 / rate)
}
fn zone(lo: u8, hi: u8, root: u8, hz: f64, lovel: u8, hivel: u8) -> Zone {
    let w = read_wav(&sine_wav(hz, 1.0)).unwrap();
    Zone { lo_key: lo, hi_key: hi, lo_vel: lovel, hi_vel: hivel, root, rate: w.rate, data: Arc::new(w.mono()), loop_range: None, gain: 1.0 }
}

#[test]
fn a_sample_is_played_at_the_pitch_of_the_note() {
    // a recording of A3 (220 Hz) at key 57: asking for A4 plays it an octave up
    let set = SampleSet::new(vec![zone(0, 127, 57, 220.0, 0, 127)]);
    let same = set.render(57, 1.0, 0.5, 44100.0);
    let up = set.render(69, 1.0, 0.5, 44100.0);
    let (f1, f2) = (crossings(&same[..22000], 44100.0), crossings(&up[..22000], 44100.0));
    assert!((f1 - 220.0).abs() < 6.0, "{f1}");
    assert!((f2 - 440.0).abs() < 10.0, "{f2}");
}

#[test]
fn zones_are_chosen_by_key_and_velocity() {
    let set = SampleSet::new(vec![
        zone(0, 59, 48, 130.0, 0, 127),
        zone(60, 127, 72, 523.0, 0, 64),
        zone(60, 127, 72, 525.0, 65, 127),
    ]);
    assert_eq!(set.pick(40, 0.5).unwrap().root, 48);
    assert_eq!(set.pick(70, 0.3).unwrap().root, 72);
    // the loud layer of the upper zone
    let soft = set.pick(70, 0.3).unwrap().data.as_ptr();
    let loud = set.pick(70, 0.9).unwrap().data.as_ptr();
    assert_ne!(soft, loud);
    // a key outside every zone takes the nearest root
    let only_low = SampleSet::new(vec![zone(0, 50, 48, 130.0, 0, 127), zone(51, 60, 55, 196.0, 0, 127)]);
    assert_eq!(only_low.pick(100, 0.5).unwrap().root, 55);
}

#[test]
fn a_loop_sustains_a_held_note_and_a_release_fades_it() {
    let mut z = zone(0, 127, 60, 262.0, 0, 127);
    z.loop_range = Some((2000, 8000));
    let set = SampleSet::new(vec![z]);
    // the recording is one second; held for three, the loop keeps it going
    let long = set.render(60, 1.0, 3.0, 22050.0);
    assert!(long.len() as f64 / 22050.0 > 3.0);
    let late: f32 = long[(2.8 * 22050.0) as usize..(2.9 * 22050.0) as usize].iter().fold(0.0, |m, v| m.max(v.abs()));
    assert!(late > 0.3, "{late}");
    // without the loop it ends with the recording
    let set2 = SampleSet::new(vec![zone(0, 127, 60, 262.0, 0, 127)]);
    assert!(set2.render(60, 1.0, 3.0, 22050.0).len() < 22050 + 10);
    // the end of the release is silent
    assert!(long[long.len() - 5..].iter().all(|v| v.abs() < 0.01));
}

#[test]
fn velocity_scales_a_sample() {
    let set = SampleSet::new(vec![zone(0, 127, 60, 262.0, 0, 127)]);
    let pk = |v: f64| set.render(60, v, 0.2, 22050.0).iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(pk(1.0) > pk(0.4) * 2.0);
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("animscript_audio_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn an_sfz_file_maps_wavs_over_keys_and_velocities() {
    let dir = temp_dir("sfz");
    std::fs::create_dir_all(dir.join("samples")).unwrap();
    std::fs::write(dir.join("samples/low.wav"), sine_wav(130.8, 1.0)).unwrap();
    std::fs::write(dir.join("samples/high soft.wav"), sine_wav(523.0, 1.0)).unwrap();
    std::fs::write(dir.join("samples/high loud.wav"), sine_wav(523.0, 1.0)).unwrap();
    let sfz = "// a tiny library
<control> default_path=samples/
<group> loop_mode=no_loop volume=-6
<region> sample=low.wav lokey=c0 hikey=b3 pitch_keycenter=c3
<group> lokey=c4 hikey=c8 pitch_keycenter=c5
<region> sample=high soft.wav hivel=64
<region> sample=high loud.wav lovel=65 volume=0 loop_mode=loop_continuous loop_start=1000 loop_end=9000
";
    std::fs::write(dir.join("lib.sfz"), sfz).unwrap();
    let set = load_sfz(&dir.join("lib.sfz")).unwrap();
    assert_eq!(set.zones.len(), 3);
    let low = &set.zones[0];
    assert_eq!((low.lo_key, low.hi_key, low.root), (12, 59, 48));
    assert!((low.gain - 10f64.powf(-6.0 / 20.0)).abs() < 1e-9);
    let soft = &set.zones[1];
    assert_eq!((soft.lo_key, soft.hi_key, soft.root, soft.hi_vel), (60, 108, 72, 64));
    let loud = &set.zones[2];
    assert_eq!(loud.lo_vel, 65);
    assert_eq!(loud.loop_range, Some((1000, 9000)));
    assert!((loud.gain - 1.0).abs() < 1e-9, "a region overrides its group's volume");
    // a played note sounds
    let note = set.render(72, 0.5, 0.3, 44100.0);
    assert!(note.iter().any(|v| v.abs() > 0.05));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn sfz_errors_say_what_is_wrong() {
    let dir = temp_dir("sfzerr");
    std::fs::write(dir.join("missing.sfz"), "<region> sample=nope.wav key=c4\n").unwrap();
    let e = load_sfz(&dir.join("missing.sfz")).err().unwrap();
    assert!(e.contains("nope.wav"), "{e}");
    std::fs::write(dir.join("empty.sfz"), "<region> key=c4\n").unwrap();
    assert!(load_sfz(&dir.join("empty.sfz")).err().unwrap().contains("no regions"));
    std::fs::write(dir.join("a.wav"), sine_wav(100.0, 0.1)).unwrap();
    std::fs::write(dir.join("badkey.sfz"), "<region> sample=a.wav key=zz9\n").unwrap();
    assert!(load_sfz(&dir.join("badkey.sfz")).err().unwrap().contains("key"));
    assert!(load_sfz(&dir.join("absent.sfz")).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn key_names_and_numbers() {
    assert_eq!(key_of("c4"), Some(60));
    assert_eq!(key_of("C#3"), Some(49));
    assert_eq!(key_of("db3"), Some(49));
    assert_eq!(key_of("60"), Some(60));
    assert_eq!(key_of("200"), None);
    assert_eq!(key_of("zz"), None);
}

#[test]
fn a_loop_in_the_wav_file_is_read() {
    // build a WAV with a `smpl` chunk holding one loop (frames 100..900)
    let mut bytes = sine_wav(200.0, 0.2);
    let mut smpl = vec![0u8; 36 + 24];
    smpl[28..32].copy_from_slice(&1u32.to_le_bytes());
    smpl[36 + 8..36 + 12].copy_from_slice(&100u32.to_le_bytes());
    smpl[36 + 12..36 + 16].copy_from_slice(&900u32.to_le_bytes());
    bytes.extend_from_slice(b"smpl");
    bytes.extend_from_slice(&(smpl.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&smpl);
    let riff_len = (bytes.len() - 8) as u32;
    bytes[4..8].copy_from_slice(&riff_len.to_le_bytes());
    assert_eq!(read_wav(&bytes).unwrap().loop_points, Some((100, 900)));
}

#[test]
fn a_sampled_instrument_plays_in_a_score() {
    let dir = temp_dir("bank");
    std::fs::write(dir.join("a.wav"), sine_wav(440.0, 0.5)).unwrap();
    std::fs::write(dir.join("a.sfz"), "<region> sample=a.wav key=a4\n").unwrap();
    let mut bank = Bank::new();
    bank.load_sfz("my_sine", &dir.join("a.sfz")).unwrap();
    assert!(bank.names().contains(&"my_sine".to_string()));
    let mut s = Score::new(120.0, Meter::new(4, 4));
    s.tracks.push(Track::new("t", "my_sine", parse_notes("A4:1/4", Meter::new(4, 4)).unwrap()));
    let st = render_score(&s, &Mix::default(), &bank).unwrap();
    assert!(st.rms(0.05, 0.4) > 0.05);
    assert_eq!(st.to_wav(), render_score(&s, &Mix::default(), &bank).unwrap().to_wav());
    std::fs::remove_dir_all(&dir).ok();
}
