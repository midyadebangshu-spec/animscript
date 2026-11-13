//! With the `jit` feature, `drive` equations may run as machine code; the snapshots must not change by a bit.
#![cfg(feature = "jit")]
use animscript_lang::value::JitMode;
use animscript_lang::*;

const HEAD: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 400x300\n    fps: 10\n\n";

fn go(src: &str, jit: JitMode) -> RunResult {
    let text = format!("{HEAD}{src}");
    match run(&text, &RunOptions { trace: true, jit }) {
        Ok(r) => r,
        Err(d) => panic!("{}", d.render(Some(&text), "test")),
    }
}

fn same_either_way(src: &str) {
    let off = go(src, JitMode::Off);
    let always = go(src, JitMode::Always);
    let auto = go(src, JitMode::Auto);
    assert!(
        !off.trace.iter().any(|l| l.contains("JIT")),
        "off means off"
    );
    assert!(
        always
            .trace
            .iter()
            .any(|l| l.contains("JIT") && l.contains("compiled")),
        "{:?}",
        always.trace
    );
    assert_eq!(
        off.hashes, always.hashes,
        "snapshots differ between the interpreter and the JIT"
    );
    assert_eq!(off.hashes, auto.hashes);
    assert_eq!(off.snapshots, always.snapshots);
    assert!(
        off.hashes.windows(2).any(|w| w[0] != w[1]),
        "the scene moves"
    );
}

#[test]
fn a_wave_is_identical() {
    same_either_way(
        "shape w = plot(y: \"0\", from: 0, to: 400, n: 300)\n\
         scene \"s\" for 20.frames:\n    main:\n        start w.drive(y: \"sin(x / 7 + t * 5) * 6 + noise(x / 20, t) * 3 + x ^ 0.5\")\n        wait 20.frames\n",
    );
}

#[test]
fn std_waves_and_modes_are_identical() {
    same_either_way(
        "import \"std/waves\"\nshape w = plot(y: \"0\", from: 0, to: 400, n: 200)\nshape b = rectangle(width: 20, height: 10, at: (30, 0))\n\
         scene \"s\" for 20.frames:\n    main:\n        start w.choppy(amp: 8, length: 100, period: 1.second, sharp: 0.5, over: 2.seconds)\n        start b.float_on(amp: 8, length: 100, period: 1.second, over: 2.seconds)\n        wait 20.frames\n",
    );
    same_either_way(
        "shape w = plot(y: \"0\", from: 0, to: 400, n: 200)\n\
         scene \"s\" for 20.frames:\n    main:\n        start w.drive(x: \"x * 1.01\", y: \"i % 7 > 3 ? hash(i) * 9 : -u * t\", mode: \"replace\", speed: 3)\n        wait 20.frames\n",
    );
}

#[test]
fn the_water_example_is_identical() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/water.anim"
    ))
    .unwrap();
    let off = run(
        &src,
        &RunOptions {
            jit: JitMode::Off,
            ..Default::default()
        },
    )
    .unwrap();
    let always = run(
        &src,
        &RunOptions {
            jit: JitMode::Always,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(off.hashes, always.hashes);
}

#[test]
fn small_drives_stay_on_the_interpreter_in_auto() {
    let r = go(
        "shape w = plot(y: \"0\", from: 0, to: 400, n: 20)\nscene \"s\" for 3.frames:\n    main:\n        start w.drive(y: \"t\")\n        wait 3.frames\n",
        JitMode::Auto,
    );
    assert!(!r.trace.iter().any(|l| l.contains("JIT")));
}
