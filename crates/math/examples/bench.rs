//! Interpreter versus hand-fused native code for the same equations: an upper bound on what a JIT could gain.
//! Run: `cargo run --release -p animscript-math --example bench`
use animscript_math::*;
use std::time::Instant;

fn build(src: &str) -> Program {
    let mut b = Builder::new();
    let x = b.input("x", Rate::Point);
    let t = b.input("t", Rate::Frame);
    let id = parse(src, &mut b, &Env::new().with("x", x).with("t", t)).unwrap();
    b.finish(&[id])
}

type Fused = fn(f64, f64) -> f64;

fn time(label: &str, reps: usize, mut f: impl FnMut() -> f64) -> f64 {
    let mut sink = 0.0;
    f();
    let s = Instant::now();
    for _ in 0..reps {
        sink += f();
    }
    let per = s.elapsed().as_secs_f64() / reps as f64;
    std::hint::black_box(sink);
    println!("    {label:<12} {:>9.1} us", per * 1e6);
    per
}

fn main() {
    let cases: [(&str, &str, Fused); 4] = [
        ("swell", "10 * sin(tau * (x / 240 - t / 3))", |x, t| {
            10.0 * libm::sin(std::f64::consts::TAU * (x / 240.0 - t / 3.0))
        }),
        (
            "ripple+noise",
            "6 * (sin(tau * (x / 40 - t)) + 0.6 * sin(tau * (x / 68 + t / 1.3 + 0.2))) / 1.6 + noise(x / 40, t) * 1.8",
            |x, t| {
                6.0 * (libm::sin(std::f64::consts::TAU * (x / 40.0 - t))
                    + 0.6 * libm::sin(std::f64::consts::TAU * (x / 68.0 + t / 1.3 + 0.2)))
                    / 1.6
                    + noise_apply(2, 0, x / 40.0, t, 0.0) * 1.8
            },
        ),
        ("polynomial", "((x*x*0.001 + x*0.5 - 3)*x + 7) / (1 + x*x*0.0001) + t", |x, t| {
            ((x * x * 0.001 + x * 0.5 - 3.0) * x + 7.0) / (1.0 + x * x * 0.0001) + t
        }),
        ("trig-heavy", "sin(x) * cos(x * 0.3) + tan(x * 0.1) * atan(x) + exp(-x * x * 0.001)", |x, _| {
            libm::sin(x) * libm::cos(x * 0.3) + libm::tan(x * 0.1) * libm::atan(x) + libm::exp(-x * x * 0.001)
        }),
    ];
    for n in [1_000usize, 10_000, 100_000] {
        println!("{n} points");
        let xs: Vec<f64> = (0..n).map(|i| i as f64 * 0.37 - 200.0).collect();
        let reps = (2_000_000 / n).max(5);
        for (name, src, fused) in &cases {
            let p = build(src);
            println!("  {name}");
            let a = time("interpreter", reps, || {
                p.eval(&[&xs], &[0.7], &[]).unwrap()[0][n / 2]
            });
            #[cfg(feature = "jit")]
            {
                let t0 = Instant::now();
                let c = p.compile().expect("jit");
                println!(
                    "    jit compile  {:>9.1} us",
                    t0.elapsed().as_secs_f64() * 1e6
                );
                let j = time("jit", reps, || {
                    c.eval(&[&xs], &[0.7], &[]).unwrap()[0][n / 2]
                });
                println!("    jit speedup  {:.2}x", a / j);
            }
            #[cfg(feature = "llvm")]
            {
                let t0 = Instant::now();
                let c = p.compile_llvm().expect("llvm");
                println!(
                    "    llvm compile {:>9.1} us",
                    t0.elapsed().as_secs_f64() * 1e6
                );
                let l = time("llvm", reps, || {
                    c.eval(&[&xs], &[0.7], &[]).unwrap()[0][n / 2]
                });
                println!("    llvm speedup {:.2}x", a / l);
            }
            let mut out = vec![0.0; n];
            let b = time("fused native", reps, || {
                for (o, x) in out.iter_mut().zip(&xs) {
                    *o = fused(*x, 0.7);
                }
                out[n / 2]
            });
            println!("    speedup bound {:.2}x", a / b);
        }
    }
}
