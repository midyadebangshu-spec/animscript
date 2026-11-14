//! Effects: a Freeverb-style reverb and a feedback delay. Fixed buffers, fixed order: deterministic.

struct Comb {
    buf: Vec<f64>,
    i: usize,
    store: f64,
}
impl Comb {
    fn new(n: usize) -> Comb {
        Comb { buf: vec![0.0; n.max(1)], i: 0, store: 0.0 }
    }
    fn process(&mut self, x: f64, feedback: f64, damp: f64) -> f64 {
        let y = self.buf[self.i];
        self.store = y * (1.0 - damp) + self.store * damp;
        self.buf[self.i] = x + self.store * feedback;
        self.i = (self.i + 1) % self.buf.len();
        y
    }
}
struct Allpass {
    buf: Vec<f64>,
    i: usize,
}
impl Allpass {
    fn new(n: usize) -> Allpass {
        Allpass { buf: vec![0.0; n.max(1)], i: 0 }
    }
    fn process(&mut self, x: f64) -> f64 {
        let b = self.buf[self.i];
        let y = -x + b;
        self.buf[self.i] = x + b * 0.5;
        self.i = (self.i + 1) % self.buf.len();
        y
    }
}

pub struct Reverb {
    combs: [Vec<Comb>; 2],
    allpass: [Vec<Allpass>; 2],
    feedback: f64,
    damp: f64,
}

impl Reverb {
    /// `room` 0 to 1 (size of the space), `damp` 0 to 1 (how much the tail darkens).
    pub fn new(rate: f64, room: f64, damp: f64) -> Reverb {
        let scale = rate / 44100.0;
        let combs_len = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
        let ap_len = [556, 441, 341, 225];
        let mk = |spread: usize| {
            (
                combs_len.iter().map(|n| Comb::new(((n + spread) as f64 * scale) as usize)).collect::<Vec<_>>(),
                ap_len.iter().map(|n| Allpass::new(((n + spread) as f64 * scale) as usize)).collect::<Vec<_>>(),
            )
        };
        let (cl, al) = mk(0);
        let (cr, ar) = mk(23);
        Reverb { combs: [cl, cr], allpass: [al, ar], feedback: 0.7 + 0.28 * room.clamp(0.0, 1.0), damp: damp.clamp(0.0, 0.95) }
    }

    /// Reverberate a stereo signal; returns the wet signal only.
    pub fn process(&mut self, left: &[f32], right: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let n = left.len().min(right.len());
        let mut wl = Vec::with_capacity(n);
        let mut wr = Vec::with_capacity(n);
        for i in 0..n {
            let x = (left[i] as f64 + right[i] as f64) * 0.015;
            let mut out = [0.0; 2];
            for ch in 0..2 {
                let mut acc = 0.0;
                for c in self.combs[ch].iter_mut() {
                    acc += c.process(x, self.feedback, self.damp);
                }
                for a in self.allpass[ch].iter_mut() {
                    acc = a.process(acc);
                }
                out[ch] = acc;
            }
            wl.push(out[0] as f32);
            wr.push(out[1] as f32);
        }
        (wl, wr)
    }
}

/// A feedback delay (`time` seconds, `feedback` 0 to 0.95); returns the wet signal.
pub fn delay(input: &[f32], rate: f64, time: f64, feedback: f64) -> Vec<f32> {
    let n = ((time * rate) as usize).max(1);
    let mut buf = vec![0.0f32; n];
    let mut out = Vec::with_capacity(input.len());
    for (i, x) in input.iter().enumerate() {
        let y = buf[i % n];
        buf[i % n] = x + y * feedback.clamp(0.0, 0.95) as f32;
        out.push(y);
    }
    out
}
