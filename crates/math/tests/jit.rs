//! The JIT must give the interpreter's bits (NaN may differ in sign and payload, but is a NaN in both).
#![cfg(feature = "jit")]
use animscript_math::*;

const ALL_UN: [Un; 24] = [
    Un::Neg,
    Un::Abs,
    Un::Sqrt,
    Un::Sin,
    Un::Cos,
    Un::Tan,
    Un::Asin,
    Un::Acos,
    Un::Atan,
    Un::Sinh,
    Un::Cosh,
    Un::Tanh,
    Un::Exp,
    Un::Log,
    Un::Log2,
    Un::Log10,
    Un::Floor,
    Un::Ceil,
    Un::Round,
    Un::Trunc,
    Un::Fract,
    Un::Sign,
    Un::Not,
    Un::Hash,
];
const ALL_BIN: [Bin; 17] = [
    Bin::Add,
    Bin::Sub,
    Bin::Mul,
    Bin::Div,
    Bin::Rem,
    Bin::Pow,
    Bin::Min,
    Bin::Max,
    Bin::Atan2,
    Bin::Lt,
    Bin::Le,
    Bin::Gt,
    Bin::Ge,
    Bin::Eq,
    Bin::Ne,
    Bin::And,
    Bin::Or,
];

fn nasty() -> Vec<f64> {
    vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        -0.5,
        1.5,
        -1.5,
        2.5,
        -2.5,
        3.0,
        0.1,
        123.456,
        1e-5,
        1e-310,
        -1e-310,
        1e300,
        -1e300,
        f64::MAX,
        f64::MIN_POSITIVE,
        9007199254740993.0,
        4503599627370497.5,
        std::f64::consts::PI,
        -7.25,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        20.0,
        -20.0,
        710.0,
        -745.0,
    ]
}

fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

fn check(p: &Program, point: &[&[f64]], frame: &[f64], params: &[f64], what: &str) {
    let want = p.eval(point, frame, params).unwrap();
    let c = p.compile().expect("the JIT works on this machine");
    let got = c.eval(point, frame, params).unwrap();
    assert_eq!(want.len(), got.len());
    for (o, (w, g)) in want.iter().zip(&got).enumerate() {
        for (i, (a, b)) in w.iter().zip(g).enumerate() {
            assert!(
                same(*a, *b),
                "{what}: output {o} at point {i}: interpreter {a:e} ({:#x}) vs jit {b:e} ({:#x})",
                a.to_bits(),
                b.to_bits()
            );
        }
    }
}

#[test]
fn every_unary_op_matches() {
    let vals = nasty();
    for op in ALL_UN {
        let mut b = Builder::new();
        let x = b.input("x", Rate::Point);
        let r = b.un(op, x);
        let p = b.finish(&[r]);
        check(&p, &[&vals], &[], &[], &format!("{op:?}"));
    }
}

#[test]
fn every_binary_op_matches() {
    let vals = nasty();
    let (mut xs, mut ys) = (vec![], vec![]);
    for a in &vals {
        for b in &vals {
            xs.push(*a);
            ys.push(*b);
        }
    }
    for op in ALL_BIN {
        let mut b = Builder::new();
        let x = b.input("x", Rate::Point);
        let y = b.input("y", Rate::Point);
        let r = b.bin(op, x, y);
        let p = b.finish(&[r]);
        check(&p, &[&xs, &ys], &[], &[], &format!("{op:?}"));
    }
}

#[test]
fn select_and_noise_match() {
    let vals = nasty();
    let mut xs = vec![];
    let (mut ys, mut zs) = (vec![], vec![]);
    for (i, a) in vals.iter().enumerate() {
        for (j, b) in vals.iter().enumerate() {
            xs.push(*a);
            ys.push(*b);
            zs.push(vals[(i * 7 + j * 3) % vals.len()]);
        }
    }
    let mut b = Builder::new();
    let (x, y, z) = (
        b.input("x", Rate::Point),
        b.input("y", Rate::Point),
        b.input("z", Rate::Point),
    );
    let s = b.select(x, y, z);
    let p = b.finish(&[s]);
    check(&p, &[&xs, &ys, &zs], &[], &[], "select");
    for dims in 1..=3usize {
        let mut b = Builder::new();
        let (x, y, z) = (
            b.input("x", Rate::Point),
            b.input("y", Rate::Point),
            b.input("z", Rate::Point),
        );
        let args = [x, y, z];
        let n = b.noise(7, &args[..dims]);
        let p = b.finish(&[n]);
        check(&p, &[&xs, &ys, &zs], &[], &[], &format!("noise{dims}"));
    }
}

/// A small deterministic generator for the fuzz test.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn random_expr(r: &mut Lcg, b: &mut Builder, leaves: &[Id], depth: usize) -> Id {
    if depth == 0 || r.below(8) == 0 {
        return match r.below(4) {
            0 => b.constant(
                [0.0, 1.0, -2.0, 0.5, 3.0, 1e-3, 100.0, std::f64::consts::PI][r.below(8)],
            ),
            _ => leaves[r.below(leaves.len())],
        };
    }
    match r.below(10) {
        0..=3 => {
            let (x, y) = (
                random_expr(r, b, leaves, depth - 1),
                random_expr(r, b, leaves, depth - 1),
            );
            b.bin(ALL_BIN[r.below(ALL_BIN.len())], x, y)
        }
        4..=7 => {
            let x = random_expr(r, b, leaves, depth - 1);
            b.un(ALL_UN[r.below(ALL_UN.len())], x)
        }
        8 => {
            let (c, x, y) = (
                random_expr(r, b, leaves, depth - 1),
                random_expr(r, b, leaves, depth - 1),
                random_expr(r, b, leaves, depth - 1),
            );
            b.select(c, x, y)
        }
        _ => {
            let dims = 1 + r.below(3);
            let args: Vec<Id> = (0..dims)
                .map(|_| random_expr(r, b, leaves, depth - 1))
                .collect();
            b.noise(r.below(5) as u32, &args)
        }
    }
}

#[test]
fn random_programs_match() {
    let mut r = Lcg(0xA11CE);
    let mut xs: Vec<f64> = (0..60).map(|i| i as f64 * 0.731 - 20.0).collect();
    xs.extend(nasty());
    let ys: Vec<f64> = xs.iter().rev().cloned().collect();
    for case in 0..600 {
        let mut b = Builder::new();
        let x = b.input("x", Rate::Point);
        let y = b.input("y", Rate::Point);
        let t = b.input("t", Rate::Frame);
        let k = b.param("k");
        let leaves = [x, y, t, k];
        let a = random_expr(&mut r, &mut b, &leaves, 5);
        let c = random_expr(&mut r, &mut b, &leaves, 4);
        let mut outs = vec![a, c];
        // sometimes also check a derivative
        if case % 5 == 0 {
            if let Ok(d) = b.diff(a, x) {
                outs.push(d);
            }
        }
        let p = b.finish(&outs);
        check(
            &p,
            &[&xs, &ys],
            &[0.37],
            &[2.5],
            &format!("random program {case}"),
        );
    }
}

#[test]
fn sizes_and_uniform_programs() {
    let mut b = Builder::new();
    let x = b.input("x", Rate::Point);
    let t = b.input("t", Rate::Frame);
    let s = b.un(Un::Sin, t);
    let u = b.bin(Bin::Mul, s, s); // uniform only
    let v = b.bin(Bin::Add, x, u);
    let p = b.finish(&[u, v]);
    for n in [0usize, 1, 2, 3, 7, 64, 1001] {
        let xs: Vec<f64> = (0..n).map(|i| i as f64 * 0.3).collect();
        check(&p, &[&xs], &[1.9], &[], &format!("n = {n}"));
    }
    // a program with no point inputs at all
    let mut b = Builder::new();
    let t = b.input("t", Rate::Frame);
    let r = b.un(Un::Exp, t);
    let p = b.finish(&[r]);
    let c = p.compile().unwrap();
    assert_eq!(
        c.eval_n(5, &[], &[0.5], &[]).unwrap(),
        p.eval_n(5, &[], &[0.5], &[]).unwrap()
    );
}

#[test]
fn argument_errors_match_the_interpreter() {
    let mut b = Builder::new();
    let x = b.input("x", Rate::Point);
    let p = b.finish(&[x]);
    let c = p.compile().unwrap();
    assert_eq!(c.eval(&[], &[], &[]), p.eval(&[], &[], &[]));
    assert_eq!(
        c.eval_n(3, &[&[1.0]], &[], &[]),
        p.eval_n(3, &[&[1.0]], &[], &[])
    );
    assert!(matches!(
        c.eval(&[&[1.0]], &[1.0], &[]),
        Err(EvalError::FrameInputs { .. })
    ));
}

#[test]
fn parsed_equations_match() {
    for src in [
        "10 * sin(tau * (x / 240 - t / 3))",
        "6 * (sin(tau * (x / 40 - t)) + 0.6 * sin(tau * (x / 68 + t / 1.3 + 0.2))) / 1.6 + noise(x / 40, t) * 1.8",
        "x > 0 ? sqrt(x) : -log(-x)",
        "min(max(x, -3), 3) % 1.7 + atan2(x, t) + hash(x) + frac(x * 3.3) + round(x) + sign(x)",
    ] {
        let mut b = Builder::new();
        let x = b.input("x", Rate::Point);
        let t = b.input("t", Rate::Frame);
        let id = parse(src, &mut b, &Env::new().with("x", x).with("t", t)).unwrap();
        let p = b.finish(&[id]);
        let xs: Vec<f64> = (0..500).map(|i| i as f64 * 0.77 - 150.0).collect();
        check(&p, &[&xs], &[0.9], &[], src);
    }
}
