//! `Program::eval_scalar` is `eval1` without allocating: the same numbers, bit for bit.
use animscript_math::*;

#[test]
fn eval_scalar_matches_eval1_bit_for_bit() {
    for src in [
        "sin(u * 9) * cos(v * 3) + noise(u, v) * 0.1",
        "u > v ? u : v * 2",
        "fract(u * 7) < 0.5 && v > 0.2",
        "max(abs(u - 0.5), abs(v - 0.5))",
    ] {
        let mut b = Builder::new();
        let (u, v) = (b.input("u", Rate::Point), b.input("v", Rate::Point));
        let env = Env::new().with("u", u).with("v", v);
        let id = parse(src, &mut b, &env).unwrap();
        let p = b.finish(&[id]);
        let mut regs = vec![];
        for (a, c) in [(0.1, 0.9), (0.37, 0.02), (0.99, 0.5), (0.0, 0.0)] {
            let want = p.eval1(&[a, c], &[], &[]).unwrap()[0];
            let got = p.eval_scalar(&[a, c], &mut regs);
            assert_eq!(want.to_bits(), got.to_bits(), "{src} at ({a}, {c})");
        }
    }
}
