use animscript_math::*;

fn compile(src: &str) -> (Program, Vec<&'static str>) {
    let mut b = Builder::new();
    let x = b.input("x", Rate::Point);
    let t = b.input("t", Rate::Frame);
    let env = Env::new().with("x", x).with("t", t);
    let id = parse(src, &mut b, &env).unwrap();
    (b.finish(&[id]), vec!["x", "t"])
}

fn ev(src: &str, x: f64, t: f64) -> f64 {
    let (p, _) = compile(src);
    p.eval1(&[x], &[t], &[]).unwrap()[0]
}

#[test]
fn precedence_and_functions() {
    assert_eq!(ev("1 + 2 * 3", 0.0, 0.0), 7.0);
    assert_eq!(ev("2 ^ 3 ^ 2", 0.0, 0.0), 512.0);
    assert_eq!(ev("-2 ^ 2", 0.0, 0.0), -4.0);
    assert_eq!(ev("2 ^ -1", 0.0, 0.0), 0.5);
    assert_eq!(ev("x > 1 ? 10 : 20", 2.0, 0.0), 10.0);
    assert_eq!(ev("if(x > 1, 10, 20)", 0.0, 0.0), 20.0);
    assert_eq!(ev("clamp(x, 0, 1) + t", 5.0, 0.5), 1.5);
    assert_eq!(ev("1 < 2 && 2 < 1 || 1 == 1", 0.0, 0.0), 1.0);
    assert!((ev("sin(pi / 2)", 0.0, 0.0) - 1.0).abs() < 1e-15);
    assert_eq!(ev("tau", 0.0, 0.0), std::f64::consts::TAU);
}

#[test]
fn parse_errors() {
    let mut b = Builder::new();
    let env = Env::new();
    assert!(parse("1 +", &mut b, &env).is_err());
    assert!(parse("foo", &mut b, &env)
        .unwrap_err()
        .msg
        .contains("unknown name"));
    assert!(parse("sin(1, 2)", &mut b, &env)
        .unwrap_err()
        .msg
        .contains("takes 1"));
    assert!(parse("1 2", &mut b, &env).is_err());
    assert!(parse("1 $ 2", &mut b, &env).is_err());
}

#[test]
fn constant_folding_and_sharing() {
    let mut b = Builder::new();
    let x = b.input("x", Rate::Point);
    let env = Env::new().with("x", x);
    let a = parse("sin(x) + sin(x)", &mut b, &env).unwrap();
    let c = parse("2 * 3 + 1", &mut b, &env).unwrap();
    assert_eq!(b.as_const(c), Some(7.0));
    let p = b.finish(&[a]);
    // input, sin, add: the two sin(x) are one node
    assert_eq!(p.nodes.len(), 3);
}

#[test]
fn dead_code_is_removed() {
    let mut b = Builder::new();
    let x = b.input("x", Rate::Point);
    let _unused = b.un(Un::Cos, x);
    let keep = b.un(Un::Sin, x);
    let p = b.finish(&[keep]);
    assert_eq!(p.nodes.len(), 2);
}

#[test]
fn batch_matches_single_and_is_repeatable() {
    let (p, _) = compile("sin(x / 3 + t) * 4 + noise(x, t)");
    let xs: Vec<f64> = (0..50).map(|i| i as f64 * 0.37).collect();
    let all = p.eval(&[&xs], &[1.25], &[]).unwrap();
    for (i, x) in xs.iter().enumerate() {
        assert_eq!(
            all[0][i].to_bits(),
            p.eval1(&[*x], &[1.25], &[]).unwrap()[0].to_bits()
        );
    }
    let again = p.eval(&[&xs], &[1.25], &[]).unwrap();
    assert_eq!(all, again);
}

#[test]
fn uniform_nodes_broadcast() {
    let mut b = Builder::new();
    let t = b.input("t", Rate::Frame);
    let two = b.constant(2.0);
    let y = b.mul(t, two);
    let p = b.finish(&[y]);
    let r = p.eval_n(4, &[], &[3.0], &[]).unwrap();
    assert_eq!(r[0], vec![6.0; 4]);
}

#[test]
fn input_count_errors() {
    let (p, _) = compile("x + t");
    assert!(matches!(
        p.eval(&[], &[1.0], &[]),
        Err(EvalError::PointInputs { .. })
    ));
    assert!(matches!(
        p.eval(&[&[1.0]], &[], &[]),
        Err(EvalError::FrameInputs { .. })
    ));
}

#[test]
fn params_are_inputs_to_the_program() {
    let mut b = Builder::new();
    let x = b.input("x", Rate::Point);
    let k = b.param("k");
    let y = b.mul(x, k);
    let p = b.finish(&[y]);
    assert_eq!(p.eval1(&[3.0], &[], &[4.0]).unwrap()[0], 12.0);
}

#[test]
fn noise_is_bounded_and_deterministic() {
    let (p, _) = compile("noise(x * 0.7, t)");
    let xs: Vec<f64> = (0..400).map(|i| i as f64 * 0.113).collect();
    let r = p.eval(&[&xs], &[0.3], &[]).unwrap();
    assert!(r[0].iter().all(|v| (-1.0..=1.0).contains(v)));
    assert!(r[0].iter().any(|v| *v > 0.3) && r[0].iter().any(|v| *v < -0.3));
    // value noise is exactly 0 hash-wise only at none of these; but it is continuous
    for w in r[0].windows(2) {
        assert!((w[0] - w[1]).abs() < 0.4);
    }
}

#[test]
fn derivatives_match_finite_differences() {
    let srcs = [
        "sin(x) * x",
        "x ^ 3 - 2 * x",
        "exp(x / 3) / (1 + x * x)",
        "atan2(x, 2) + tanh(x)",
        "sqrt(x + 5) * cos(x)",
        "log(x + 4) + abs(x - 0.3)",
        "x > 0 ? x * x : sin(x)",
        "min(x, 0.5) + max(x, -1.5)",
        "2 ^ x",
        "asin(x / 4) + acos(x / 5) + atan(x)",
        "sinh(x) * cosh(x) + tan(x / 5)",
        "x % 1.5",
    ];
    for src in srcs {
        let mut b = Builder::new();
        let x = b.input("x", Rate::Point);
        let f = parse(src, &mut b, &Env::new().with("x", x)).unwrap();
        let d = b.diff(f, x).unwrap();
        let p = b.finish(&[f, d]);
        for x0 in [-1.1, -0.35, 0.2, 0.77, 1.9] {
            let h = 1e-6;
            let fp = p.eval1(&[x0 + h], &[], &[]).unwrap()[0];
            let fm = p.eval1(&[x0 - h], &[], &[]).unwrap()[0];
            let fd = (fp - fm) / (2.0 * h);
            let an = p.eval1(&[x0], &[], &[]).unwrap()[1];
            assert!(
                (fd - an).abs() < 1e-5 * (1.0 + fd.abs()),
                "{src} at {x0}: fd {fd} vs {an}"
            );
        }
    }
}

#[test]
fn derivative_of_noise_is_an_error() {
    let mut b = Builder::new();
    let x = b.input("x", Rate::Point);
    let f = parse("noise(x)", &mut b, &Env::new().with("x", x)).unwrap();
    assert!(b.diff(f, x).is_err());
}

#[test]
fn derivative_wrt_other_input_is_zero() {
    let mut b = Builder::new();
    let x = b.input("x", Rate::Point);
    let t = b.input("t", Rate::Frame);
    let f = parse("sin(x) * 2", &mut b, &Env::new().with("x", x).with("t", t)).unwrap();
    let d = b.diff(f, t).unwrap();
    assert_eq!(b.as_const(d), Some(0.0));
}

#[test]
fn noise_at_huge_coordinates_does_not_overflow() {
    let mut b = Builder::new();
    let x = b.input("x", Rate::Point);
    let n = b.noise(1, &[x]);
    let p = b.finish(&[n]);
    let r = p
        .eval(&[&[1e300, -1e300, 9.3e18, f64::MAX]], &[], &[])
        .unwrap();
    assert!(r[0].iter().all(|v| (-1.0..=1.0).contains(v)));
}

// ---- stencil reads: `h[-1, 0]` is the input `h@-1,0` ------------------------------------------------------------------

fn stencil_env(b: &mut Builder) -> Env {
    let h = b.input("h", Rate::Point);
    let mut env = Env::new().with("h", h);
    env.stencils.insert("h".to_string());
    env
}

#[test]
fn a_stencil_read_becomes_a_named_point_input() {
    let mut b = Builder::new();
    let env = stencil_env(&mut b);
    let id = parse("h[-1, 0] + h[1,0] + h[0, 2] - 3 * h", &mut b, &env).unwrap();
    let p = b.finish(&[id]);
    let names = p.input_names();
    for want in ["h", "h@-1,0", "h@1,0", "h@0,2"] {
        assert!(names.contains(&want), "{want} in {names:?}");
    }
    // evaluate with the neighbours as plain columns
    let cols: Vec<Vec<f64>> = names
        .iter()
        .map(|n| match *n {
            "h" => vec![10.0],
            "h@-1,0" => vec![1.0],
            "h@1,0" => vec![2.0],
            _ => vec![4.0],
        })
        .collect();
    let refs: Vec<&[f64]> = cols.iter().map(|c| c.as_slice()).collect();
    assert_eq!(
        p.eval(&refs, &[], &[]).unwrap()[0][0],
        1.0 + 2.0 + 4.0 - 30.0
    );
}

#[test]
fn the_same_stencil_twice_is_one_input() {
    let mut b = Builder::new();
    let env = stencil_env(&mut b);
    let id = parse("h[1, 0] * h[1, 0]", &mut b, &env).unwrap();
    let p = b.finish(&[id]);
    assert_eq!(p.input_names().iter().filter(|n| **n == "h@1,0").count(), 1);
}

#[test]
fn stencil_errors() {
    let mut b = Builder::new();
    let env = stencil_env(&mut b);
    assert!(parse("h[x, 0]", &mut b, &env)
        .unwrap_err()
        .msg
        .contains("whole number"));
    assert!(parse("h[1]", &mut b, &env).unwrap_err().msg.contains("two"));
    assert!(parse("h[1.5, 0]", &mut b, &env)
        .unwrap_err()
        .msg
        .contains("whole number"));
    assert!(parse("h[40, 0]", &mut b, &env)
        .unwrap_err()
        .msg
        .contains("at most"));
    assert!(parse("h[1, 0", &mut b, &env).is_err());
    // only names declared as stencil fields can be indexed
    let mut b2 = Builder::new();
    let x = b2.input("x", Rate::Point);
    let env2 = Env::new().with("x", x);
    assert!(parse("x[1, 0]", &mut b2, &env2)
        .unwrap_err()
        .msg
        .contains("not a field"));
}
