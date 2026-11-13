//! Fields: user equations stepped over a grid (or a chain of vertices), deterministically.
use animscript_lang::*;
use serde_json::Value;

const HEAD: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 400x300\n    fps: 10\n\n";

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
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}
fn scene(ticks: u32, body: &str) -> String {
    format!("scene \"s\" for {ticks}.frames:\n    main:\n{body}        wait {ticks}.frames\n")
}
fn field(snap: &Value) -> &Value {
    &snap["fields"][0]
}
fn data(snap: &Value, ch: &str) -> Vec<f64> {
    field(snap)["data"][ch]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect()
}

// ---- diffusion ---------------------------------------------------------------------------------------------

const HEAT: &str = "field heat:\n    grid: (16, 16)\n    rect: (0, 0, 160, 160)\n    state: [\"h\"]\n    raw: true\n    edges: \"wrap\"\n    k: 1\n    init: [(\"h\", \"exp(-((x - 80)^2 + (y - 80)^2) / 300)\")]\n    step: [(\"h\", \"h + dt * k * (h[-1, 0] + h[1, 0] + h[0, -1] + h[0, 1] - 4 * h)\")]\n";

#[test]
fn diffusion_conserves_the_total_with_wrapping_edges_and_spreads() {
    let r = go(&format!("{HEAT}{}", scene(30, "")));
    let (a, b) = (&r.snapshots[0], &r.snapshots[29]);
    let sum = |s: &Value| field(s)["stats"]["h"]["sum"].as_f64().unwrap();
    near(sum(a), sum(b), 1e-9);
    assert!(
        field(b)["stats"]["h"]["max"].as_f64().unwrap()
            < field(a)["stats"]["h"]["max"].as_f64().unwrap() * 0.6,
        "the peak has spread"
    );
    assert!(
        field(b)["stats"]["h"]["min"].as_f64().unwrap() >= -1e-12,
        "diffusion with a stable step stays positive"
    );
}

// ---- the wave equation: an exact discrete solution ----------------------------------------------------------

#[test]
fn a_standing_wave_follows_the_discrete_solution_exactly() {
    let n = 24;
    let src = format!(
        "field w:\n    grid: ({n}, 1)\n    rect: (0, 0, 240, 10)\n    state: [\"h\", \"v\"]\n    raw: true\n    edges: \"zero\"\n    c2: 50\n    init: [(\"h\", \"sin(pi * (i + 1) / {np1})\"), (\"v\", \"0\")]\n    step: [[(\"v\", \"v + dt * c2 * (h[-1, 0] + h[1, 0] - 2 * h)\")], [(\"h\", \"h + dt * v\")]]\n{sc}",
        np1 = n + 1,
        sc = scene(40, "")
    );
    let r = go(&src);
    // the first mode: the laplacian is -lambda * h with lambda = 2 (1 - cos(pi / (n + 1)))
    let lambda = 2.0 * (1.0 - (std::f64::consts::PI / (n as f64 + 1.0)).cos());
    let (dt, c2) = (0.1, 50.0);
    let (mut amp, mut vel) = (1.0f64, 0.0f64);
    for tick in 1..40 {
        vel += dt * c2 * (-lambda) * amp;
        amp += dt * vel;
        let h = data(&r.snapshots[tick], "h");
        let mode = |i: usize| (std::f64::consts::PI * (i as f64 + 1.0) / (n as f64 + 1.0)).sin();
        for i in [0usize, 5, 11, 23] {
            near(h[i], amp * mode(i), 1e-9);
        }
    }
}

// ---- edges ---------------------------------------------------------------------------------------------------

fn shifted(edges: &str, offset: i32) -> Vec<f64> {
    let src = format!(
        "field f:\n    grid: (6, 1)\n    rect: (0, 0, 60, 10)\n    state: [\"h\"]\n    raw: true\n    edges: \"{edges}\"\n    init: [(\"h\", \"i + 1\")]\n    step: [(\"h\", \"h[{offset}, 0]\")]\n{}",
        scene(3, "")
    );
    data(&go(&src).snapshots[1], "h")
}

#[test]
fn edge_rules_decide_what_lies_beyond_the_grid() {
    // cells hold 1..6; reading h[-2, 0] shifts everything right by two
    assert_eq!(shifted("clamp", -2), vec![1.0, 1.0, 1.0, 2.0, 3.0, 4.0]);
    assert_eq!(shifted("wrap", -2), vec![5.0, 6.0, 1.0, 2.0, 3.0, 4.0]);
    assert_eq!(shifted("zero", -2), vec![0.0, 0.0, 1.0, 2.0, 3.0, 4.0]);
    assert_eq!(shifted("mirror", -2), vec![2.0, 1.0, 1.0, 2.0, 3.0, 4.0]);
    assert_eq!(shifted("mirror", 2), vec![3.0, 4.0, 5.0, 6.0, 6.0, 5.0]);
}

// ---- phases, substeps, time ------------------------------------------------------------------------------------

#[test]
fn a_phase_reads_the_old_state_and_the_next_phase_sees_the_new_one() {
    let mk = |step: &str| {
        let src = format!(
            "field f:\n    grid: (2, 1)\n    rect: (0, 0, 20, 10)\n    state: [\"a\", \"b\"]\n    raw: true\n    init: [(\"a\", \"0\"), (\"b\", \"0\")]\n    step: {step}\n{}",
            scene(3, "")
        );
        let s = go(&src).snapshots[1].clone();
        (data(&s, "a"), data(&s, "b"))
    };
    let (a, b) = mk("[(\"a\", \"a + 1\"), (\"b\", \"a\")]"); // one phase: b sees the old a
    assert_eq!((a[0], b[0]), (1.0, 0.0));
    let (a, b) = mk("[[(\"a\", \"a + 1\")], [(\"b\", \"a\")]]"); // two phases: b sees the new a
    assert_eq!((a[0], b[0]), (1.0, 1.0));
}

#[test]
fn substeps_divide_dt_and_time_runs_from_the_scene_start() {
    let src = format!(
        "field f:\n    grid: (2, 1)\n    rect: (0, 0, 20, 10)\n    state: [\"h\", \"s\"]\n    raw: true\n    substeps: 4\n    init: [(\"h\", \"0\"), (\"s\", \"0\")]\n    step: [(\"h\", \"h + dt\"), (\"s\", \"t\")]\n{}",
        scene(5, "")
    );
    let r = go(&src);
    // each tick is 4 substeps of dt = 1 / (10 * 4): h grows by 0.1 per tick; s holds t at the last substep
    near(data(&r.snapshots[3], "h")[0], 0.3, 1e-12);
    near(data(&r.snapshots[3], "s")[0], 0.3, 1e-12);
}

// ---- reading and writing from the script -----------------------------------------------------------------------

#[test]
fn poke_fill_and_at() {
    let src = format!(
        "field f:\n    grid: (20, 20)\n    rect: (0, 0, 200, 200)\n    state: [\"h\", \"g\"]\n    raw: true\n    init: [(\"h\", \"0\"), (\"g\", \"0\")]\n    step: [(\"g\", \"g\")]\n{}",
        scene(
            4,
            "        f.poke(100, 100, amount: 2, radius: 30)\n        assert f.at(105, 105) > 1.7\n        assert f.at(10, 10) == 0\n        f.fill(0.5, channel: \"g\")\n        assert f.at(10, 10, channel: \"g\") == 0.5\n        assert f.max() <= 2\n"
        )
    );
    let r = go(&src);
    let h = data(&r.snapshots[2], "h");
    assert!(h.iter().cloned().fold(0.0, f64::max) > 1.7 && h.contains(&0.0));
    assert!(data(&r.snapshots[2], "g").iter().all(|v| *v == 0.5));
}

// ---- errors -------------------------------------------------------------------------------------------------------

#[test]
fn a_run_away_field_is_reported_with_where_and_when() {
    let d = err(&format!(
        "field f:\n    grid: (4, 4)\n    rect: (0, 0, 40, 40)\n    state: [\"h\"]\n    init: [(\"h\", \"1\")]\n    step: [(\"h\", \"h * 1e200\")]\n{}",
        scene(10, "")
    ));
    assert_eq!(d.code, "E-FLD-003");
    assert!(
        d.message.contains("`h`") && d.message.contains("tick"),
        "{}",
        d.message
    );
}

#[test]
fn field_configuration_errors() {
    let base = |body: &str| format!("field f:\n{body}{}", scene(2, ""));
    let ok = "    rect: (0, 0, 40, 40)\n    state: [\"h\"]\n    init: [(\"h\", \"0\")]\n    step: [(\"h\", \"h\")]\n";
    assert_eq!(
        err(&base(&format!("    grid: (600, 600)\n{ok}"))).code,
        "E-FLD-002"
    );
    assert_eq!(
        err(&base(&format!("    grid: (4, 0)\n{ok}"))).code,
        "E-FLD-002"
    );
    assert_eq!(
        err(&base(
            "    rect: (0, 0, 40, 40)\n    state: [\"h\"]\n    init: [(\"h\", \"0\")]\n"
        ))
        .code,
        "E-FLD-001"
    ); // no grid
    assert_eq!(
        err(&base(&format!(
            "    grid: (4, 4)\n    edges: \"torus\"\n{ok}"
        )))
        .code,
        "E-FLD-001"
    );
    assert_eq!(err(&base("    grid: (4, 4)\n    rect: (0, 0, 40, 40)\n    state: [\"h\"]\n    init: [(\"q\", \"0\")]\n")).code, "E-FLD-001"); // unknown channel
    assert_eq!(err(&base("    grid: (4, 4)\n    rect: (0, 0, 40, 40)\n    state: [\"h\"]\n    init: [(\"h\", \"0\")]\n    step: [(\"h\", \"h[0, 1] + sin(\")]\n")).code, "E-MTH-001");
    assert_eq!(err(&base("    grid: (4, 1)\n    rect: (0, 0, 40, 10)\n    state: [\"h\"]\n    init: [(\"h\", \"0\")]\n    step: [(\"h\", \"h[0, 1]\")]\n")).code, "E-FLD-001"); // no rows in a 1D field
    assert_eq!(err(&base("    grid: (4, 4)\n    rect: (0, 0, 40, 40)\n    state: [\"h\", \"h\"]\n    init: [(\"h\", \"0\")]\n")).code, "E-FLD-001"); // duplicate channel
    assert_eq!(err(&base("    grid: (4, 4)\n    rect: (0, 0, 40, 40)\n    state: [\"h\"]\n    init: [(\"h\", \"0\")]\n    bogus: \"x\"\n")).code, "E-FLD-001");
    assert_eq!(err("field f:\n    grid: (4, 4)\n    rect: (0, 0, 40, 40)\n    state: [\"h\"]\n    init: [(\"h\", \"0\")]\nfield f:\n    grid: (4, 4)\n    rect: (0, 0, 40, 40)\n    state: [\"h\"]\n    init: [(\"h\", \"0\")]\n").code, "E-FLD-001");
    // two fields with one name
}

// ---- determinism and the snapshot ------------------------------------------------------------------------------------

const RD: &str = "field rd:\n    grid: (24, 24)\n    rect: (0, 0, 240, 240)\n    state: [\"u\", \"v\"]\n    edges: \"wrap\"\n    du: 0.16\n    dv: 0.08\n    f: 0.035\n    k: 0.06\n    init: [(\"u\", \"1\"), (\"v\", \"hash(i * 24 + j) > 0.9 ? 0.5 : 0\")]\n    step: [[(\"u\", \"u + dt * 4 * (du * (u[-1,0] + u[1,0] + u[0,-1] + u[0,1] - 4 * u) - u * v * v + f * (1 - u))\"), (\"v\", \"v + dt * 4 * (dv * (v[-1,0] + v[1,0] + v[0,-1] + v[0,1] - 4 * v) + u * v * v - (f + k) * v)\")]]\n    ramp: [#103a6b, #ffffff]\n    channel: \"v\"\n    range: (0, 0.6)\n";

#[test]
fn a_reaction_diffusion_run_is_deterministic_and_the_snapshot_describes_the_field() {
    let src = format!("{RD}{}", scene(20, ""));
    let (a, b) = (go(&src), go(&src));
    assert_eq!(a.hashes, b.hashes);
    assert!(a.hashes.windows(2).any(|w| w[0] != w[1]));
    let f = field(&a.snapshots[19]);
    assert_eq!(f["name"], "rd");
    assert_eq!((f["nx"].as_u64(), f["ny"].as_u64()), (Some(24), Some(24)));
    assert_eq!(f["rect"], serde_json::json!([0.0, 0.0, 240.0, 240.0]));
    assert!(f["checksum"].as_str().unwrap().len() == 16);
    assert!(
        f["rgba"].as_str().unwrap().len() > 100,
        "a colour-mapped image is carried"
    );
    assert!(f.get("data").is_none(), "raw data only when asked for");
    // a field without a ramp has no image
    let plain = go(&format!(
        "{}{}",
        RD.split("    ramp:").next().unwrap(),
        scene(2, "")
    ));
    assert!(field(&plain.snapshots[1]).get("rgba").is_none());
}

#[test]
fn the_field_is_reset_with_each_scene() {
    let src = format!(
        "{HEAT}scene \"a\" for 5.frames:\n    main:\n        heat.poke(80, 80, amount: 5, radius: 40)\n        wait 5.frames\nscene \"b\" for 2.frames:\n    main:\n        wait 2.frames\n"
    );
    let r = go(&src);
    // scene "a" has 5 ticks: tick 5 is the first of scene "b", which starts from the initial bump again, not the poke
    let fresh = go(&format!("{HEAT}{}", scene(2, "")));
    assert_eq!(
        field(&r.snapshots[5])["checksum"],
        field(&fresh.snapshots[0])["checksum"]
    );
    assert_ne!(
        field(&r.snapshots[4])["checksum"],
        field(&fresh.snapshots[0])["checksum"]
    );
}

// ---- the machine-code path gives the same bits ----------------------------------------------------------------------

#[cfg(feature = "jit")]
#[test]
fn a_big_field_is_identical_with_and_without_machine_code() {
    use animscript_lang::value::JitMode;
    let src = format!(
        "{}{}",
        RD.replace("(24, 24)", "(40, 40)").replace("240", "400"),
        scene(12, "")
    );
    let text = format!("{HEAD}{src}");
    let run_with = |jit| run(&text, &RunOptions { trace: true, jit }).unwrap();
    let (off, always) = (run_with(JitMode::Off), run_with(JitMode::Always));
    assert!(
        always.trace.iter().any(|l| l.contains("JIT")),
        "{:?}",
        always.trace
    );
    assert_eq!(off.hashes, always.hashes);
}

// ---- fields feed shapes and particles ------------------------------------------------------------------------------------

fn verts(snap: &Value, shape: usize) -> Vec<(f64, f64)> {
    snap["shapes"][shape]["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v[1].as_f64().unwrap(), v[2].as_f64().unwrap()))
        .collect()
}

#[test]
fn a_drive_can_read_a_field_at_its_vertices() {
    // a chain field has one cell per vertex: h = the vertex index, and the line is displaced by it
    let src = "shape w = plot(y: \"0\", from: 0, to: 100, n: 11)\nfield pond:\n    chain: w\n    state: [\"h\"]\n    init: [(\"h\", \"i\")]\n    step: [(\"h\", \"h + dt * 10\")]\nscene \"s\" for 11.frames:\n    main:\n        start w.drive(y: \"-pond_h * 2\", fields: [pond])\n        wait 11.frames\n";
    let r = go(src);
    let v0 = verts(&r.snapshots[0], 0);
    for (k, (_, y)) in v0.iter().enumerate() {
        near(*y, -2.0 * k as f64, 1e-9);
    }
    // the field evolves (h grows by 1 per tick) and the line follows it
    let v5 = verts(&r.snapshots[5], 0);
    near(v5[3].1, -2.0 * (3.0 + 5.0), 1e-9);
}

#[test]
fn a_grid_field_is_sampled_bilinearly_between_cell_centres() {
    let src = "shape w = plot(y: \"0\", from: 0, to: 100, n: 5)\nfield f:\n    grid: (2, 1)\n    rect: (0, 0, 100, 10)\n    state: [\"h\"]\n    init: [(\"h\", \"i * 10\")]\n    step: [(\"h\", \"h\")]\nscene \"s\" for 3.frames:\n    main:\n        start w.drive(y: \"f\", fields: [f])\n        assert f.at(50, 5) == 5\n        wait 3.frames\n";
    let r = go(src);
    let ys: Vec<f64> = verts(&r.snapshots[1], 0).iter().map(|p| p.1).collect();
    // vertices at x = 0, 25, 50, 75, 100: clamped, the first centre, the middle, the second centre, clamped
    for (y, want) in ys.iter().zip([0.0, 0.0, 5.0, 10.0, 10.0]) {
        near(*y, want, 1e-9);
    }
}

#[test]
fn drive_field_errors() {
    let base = |drive: &str| {
        format!("shape w = plot(y: \"0\", from: 0, to: 100, n: 5)\nfield f:\n    grid: (2, 1)\n    rect: (0, 0, 100, 10)\n    state: [\"h\"]\n    init: [(\"h\", \"0\")]\nscene \"s\" for 2.frames:\n    main:\n        start w.drive({drive})\n        wait 2.frames\n")
    };
    assert_eq!(err(&base("y: \"f_q\", fields: [f]")).code, "E-MTH-001"); // no such channel input
    assert_eq!(err(&base("y: \"f\", fields: [3]")).code, "E-MTH-003");
    assert_eq!(err(&base("y: \"f\"")).code, "E-MTH-001"); // `f` is only known when listed in `fields`
}

#[test]
fn particles_can_follow_force_equations_and_fields() {
    let em = |force: &str, extra: &str| {
        format!(
        "{extra}emitter dust:\n    at: (100, 100)\n    rate: 0\n    lifetime: (5, 5)\n    speed: (0, 0)\n    max: 4\n{force}scene \"s\" for 10.frames:\n    main:\n        dust.burst(1)\n        wait 10.frames\n"
    )
    };
    let first = |r: &RunResult, k: usize| -> (f64, f64) {
        let p = &r.snapshots[k]["particles"][0]["items"][0];
        (p[0].as_f64().unwrap(), p[1].as_f64().unwrap())
    };
    // a constant force: y = 100 + F t^2 / 2 (up to the sub-stepping of the integrator)
    let r = go(&em("    force_y: \"100\"\n", ""));
    let (x, y) = first(&r, 9);
    near(x, 100.0, 1e-9);
    assert!((y - (100.0 + 0.5 * 100.0 * 0.81)).abs() < 3.0, "y = {y}");
    // a force that depends on the particle: a spring towards x = 200 (x = 200 - 100 cos(2 t): it has passed the centre after 0.9 s)
    let r = go(&em("    force_x: \"(200 - x) * 4\"\n", ""));
    let (x, y) = first(&r, 9);
    assert!(
        x > 205.0 && x < 245.0 && (y - 100.0).abs() < 1e-9,
        "x = {x}, y = {y}"
    );
    // a force read from a field: a steady wind of 60 to the right
    let wind = "field wind:\n    grid: (2, 2)\n    rect: (0, 0, 400, 400)\n    state: [\"w\"]\n    init: [(\"w\", \"60\")]\n    step: [(\"w\", \"w\")]\n";
    let r = go(&em("    force_x: \"wind_w\"\n    fields: [wind]\n", wind));
    let (x, _) = first(&r, 9);
    assert!((x - (100.0 + 0.5 * 60.0 * 0.81)).abs() < 3.0, "x = {x}");
    // errors
    let bad = |force: &str| err(&em(force, "")).code.to_string();
    assert_eq!(bad("    force_x: \"nope\"\n"), "E-MTH-001");
    assert_eq!(bad("    force_x: 3\n    fields: [3]\n"), "E-MTH-003");
}
