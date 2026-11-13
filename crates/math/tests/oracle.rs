//! Differential test against ExprTk (feature `exprtk`): the interpreter and an independent implementation must agree.
#![cfg(feature = "exprtk")]
use animscript_math::*;
use std::ffi::CString;
use std::os::raw::{c_char, c_double, c_int};

extern "C" {
    fn ox_eval(
        src: *const c_char,
        names: *const *const c_char,
        vals: *const c_double,
        n: c_int,
        ok: *mut c_int,
    ) -> c_double;
}

fn oracle(src: &str, x: f64, t: f64) -> Option<f64> {
    let s = CString::new(src).unwrap();
    let names = [CString::new("x").unwrap(), CString::new("t").unwrap()];
    let ptrs = [names[0].as_ptr(), names[1].as_ptr()];
    let vals = [x, t];
    let mut ok = 0;
    let v = unsafe { ox_eval(s.as_ptr(), ptrs.as_ptr(), vals.as_ptr(), 2, &mut ok) };
    (ok != 0).then_some(v)
}

#[test]
fn agrees_with_exprtk() {
    let srcs = [
        "sin(x) * cos(t) + x / 3",
        "x ^ 2 - t * 4 + 1",
        "exp(x / 4) * log(x + 10)",
        "sqrt(abs(x)) + tanh(t)",
        "atan2(x, t + 5) * 2",
        "x > 1 ? sin(x) : cos(x)",
        "min(x, t) + max(x, t) * 0.5",
        "sinh(x / 3) + cosh(t / 3) - tan(x / 7)",
        "floor(x) + ceil(t) + round(x * 2)",
        "(x + 1) * (t - 2) / (1 + x * x)",
        "asin(x / 10) + acos(t / 10) + atan(x)",
        "log10(x + 20) + log2(t + 20)",
    ];
    let mut checked = 0;
    for src in srcs {
        let mut b = Builder::new();
        let x = b.input("x", Rate::Point);
        let t = b.input("t", Rate::Frame);
        let f = parse(src, &mut b, &Env::new().with("x", x).with("t", t)).unwrap();
        let p = b.finish(&[f]);
        for xv in [-3.7, -1.0, 0.0, 0.4, 2.2, 6.5] {
            for tv in [-2.0, 0.3, 1.7, 4.0] {
                let mine = p.eval1(&[xv], &[tv], &[]).unwrap()[0];
                let Some(theirs) = oracle(src, xv, tv) else {
                    panic!("exprtk rejected `{src}`")
                };
                if mine.is_nan() && theirs.is_nan() {
                    continue;
                }
                assert!(
                    (mine - theirs).abs() <= 1e-9 * (1.0 + theirs.abs()),
                    "{src} at x={xv} t={tv}: {mine} vs {theirs}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 200);
}
