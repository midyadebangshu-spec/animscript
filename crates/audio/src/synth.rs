//! Voices: envelopes, oscillators, filters and the built-in instruments. Every voice is a pure function
//! of its note, velocity, length, sample rate and seed.
use std::f64::consts::TAU;

#[derive(Clone, Copy, Debug)]
pub struct Adsr {
    pub a: f64,
    pub d: f64,
    pub s: f64,
    pub r: f64,
}

impl Adsr {
    pub fn new(a: f64, d: f64, s: f64, r: f64) -> Adsr {
        Adsr { a, d, s, r }
    }
    fn held(&self, t: f64) -> f64 {
        if t < self.a {
            if self.a <= 0.0 {
                1.0
            } else {
                t / self.a
            }
        } else if t < self.a + self.d {
            1.0 - (1.0 - self.s) * (t - self.a) / self.d
        } else {
            self.s
        }
    }
    /// The level at time `t` for a note released at `off`: the attack, decay and sustain while held, then a
    /// linear release from wherever the level was.
    pub fn level(&self, t: f64, off: f64) -> f64 {
        if t < off {
            self.held(t)
        } else if self.r <= 0.0 || t >= off + self.r {
            0.0
        } else {
            self.held(off) * (1.0 - (t - off) / self.r)
        }
    }
}

pub fn midi_hz(note: f64) -> f64 {
    440.0 * 2f64.powf((note - 69.0) / 12.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wave {
    Sine,
    Saw,
    Square,
    Tri,
}

/// A small deterministic noise source.
pub struct Noise(u64);
impl Noise {
    pub fn new(seed: u64) -> Noise {
        Noise(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }
    pub fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
    }
}

fn poly_blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        2.0 * x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + 2.0 * x + 1.0
    } else {
        0.0
    }
}

/// One oscillator sample at phase `ph` (0..1) advancing `dt` per sample (band-limited saw and square).
pub fn osc(w: Wave, ph: f64, dt: f64) -> f64 {
    match w {
        Wave::Sine => (ph * TAU).sin(),
        Wave::Saw => 2.0 * ph - 1.0 - poly_blep(ph, dt),
        Wave::Square => {
            let mut v = if ph < 0.5 { 1.0 } else { -1.0 };
            v += poly_blep(ph, dt);
            v -= poly_blep((ph + 0.5) % 1.0, dt);
            v
        }
        Wave::Tri => 4.0 * (ph - 0.5).abs() - 1.0,
    }
}

/// A state-variable low-pass filter (topology-preserving transform).
pub struct Lowpass {
    ic1: f64,
    ic2: f64,
}
impl Lowpass {
    pub fn new() -> Lowpass {
        Lowpass { ic1: 0.0, ic2: 0.0 }
    }
    pub fn process(&mut self, x: f64, cutoff: f64, q: f64, rate: f64) -> f64 {
        let g = (std::f64::consts::PI * (cutoff / rate).clamp(1e-4, 0.49)).tan();
        let k = 1.0 / q.max(0.3);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        v2
    }
}
impl Default for Lowpass {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub wave: Wave,
    pub detune_cents: f64,
    pub gain: f64,
    /// Octave shift (-1 is a sub-oscillator).
    pub octave: i32,
}

#[derive(Clone, Debug)]
pub struct SubDef {
    pub layers: Vec<Layer>,
    pub amp: Adsr,
    /// Filter cutoff (Hz) at middle C, how much it follows the pitch (0 to 1), resonance and the envelope's
    /// reach in octaves.
    pub cutoff: f64,
    pub key_track: f64,
    pub q: f64,
    pub filt_env: f64,
    pub filt_adsr: Adsr,
    pub vib_rate: f64,
    pub vib_cents: f64,
    pub vib_delay: f64,
    pub noise: f64,
}

#[derive(Clone, Debug)]
pub enum Instrument {
    Sub(SubDef),
    /// A plucked string (Karplus-Strong): `decay` per round trip (0.99 rings long), `bright` 0 to 1.
    Pluck {
        decay: f64,
        bright: f64,
    },
    /// Frequency modulation: carrier at the note, modulator at `ratio` times it, modulation `index` fading with `fade` seconds.
    Fm {
        ratio: f64,
        index: f64,
        fade: f64,
        amp: Adsr,
    },
    /// Partials as (multiple of the note, gain, decay seconds; 0 = held).
    Additive {
        partials: Vec<(f64, f64, f64)>,
        amp: Adsr,
    },
    Drums,
    /// Recorded notes (see `sampler`).
    Sampled(crate::sampler::SampleSet),
}

fn sub(
    layers: Vec<Layer>,
    amp: Adsr,
    cutoff: f64,
    key_track: f64,
    q: f64,
    filt_env: f64,
    filt_adsr: Adsr,
) -> SubDef {
    SubDef {
        layers,
        amp,
        cutoff,
        key_track,
        q,
        filt_env,
        filt_adsr,
        vib_rate: 0.0,
        vib_cents: 0.0,
        vib_delay: 0.0,
        noise: 0.0,
    }
}
fn l(wave: Wave, detune: f64, gain: f64, octave: i32) -> Layer {
    Layer {
        wave,
        detune_cents: detune,
        gain,
        octave,
    }
}

/// The built-in instruments by name.
pub fn builtin(name: &str) -> Option<Instrument> {
    let flat = Adsr::new(0.0, 0.0, 1.0, 0.0);
    Some(match name {
        "sine" => Instrument::Sub(sub(
            vec![l(Wave::Sine, 0.0, 1.0, 0)],
            Adsr::new(0.01, 0.0, 1.0, 0.15),
            20000.0,
            0.0,
            0.7,
            0.0,
            flat,
        )),
        "saw_lead" => Instrument::Sub(sub(
            vec![l(Wave::Saw, -7.0, 0.5, 0), l(Wave::Saw, 7.0, 0.5, 0)],
            Adsr::new(0.01, 0.1, 0.8, 0.2),
            3500.0,
            0.8,
            1.2,
            1.5,
            Adsr::new(0.01, 0.25, 0.4, 0.2),
        )),
        "square_lead" => Instrument::Sub(sub(
            vec![l(Wave::Square, 0.0, 1.0, 0)],
            Adsr::new(0.01, 0.1, 0.8, 0.15),
            3000.0,
            0.7,
            1.0,
            1.0,
            Adsr::new(0.0, 0.2, 0.5, 0.15),
        )),
        "pad" | "strings" | "synth_strings" => Instrument::Sub(SubDef {
            vib_rate: 5.0,
            vib_cents: 6.0,
            vib_delay: 0.3,
            ..sub(
                vec![
                    l(Wave::Saw, -9.0, 0.35, 0),
                    l(Wave::Saw, 0.0, 0.35, 0),
                    l(Wave::Saw, 9.0, 0.35, 0),
                ],
                Adsr::new(0.5, 0.4, 0.85, 0.7),
                2200.0,
                0.6,
                0.9,
                0.6,
                Adsr::new(0.6, 0.5, 0.7, 0.7),
            )
        }),
        "synth_bass" | "bass" => Instrument::Sub(sub(
            vec![
                l(Wave::Saw, 0.0, 0.6, 0),
                l(Wave::Square, 0.0, 0.3, -1),
                l(Wave::Sine, 0.0, 0.5, -1),
            ],
            Adsr::new(0.005, 0.15, 0.7, 0.1),
            900.0,
            0.5,
            1.4,
            2.0,
            Adsr::new(0.0, 0.2, 0.2, 0.1),
        )),
        "brass" => Instrument::Sub(SubDef {
            vib_rate: 5.5,
            vib_cents: 5.0,
            vib_delay: 0.4,
            ..sub(
                vec![l(Wave::Saw, -4.0, 0.5, 0), l(Wave::Saw, 4.0, 0.5, 0)],
                Adsr::new(0.08, 0.15, 0.8, 0.15),
                1800.0,
                0.8,
                0.9,
                2.0,
                Adsr::new(0.08, 0.2, 0.6, 0.15),
            )
        }),
        "flute" => Instrument::Sub(SubDef {
            vib_rate: 5.0,
            vib_cents: 8.0,
            vib_delay: 0.25,
            noise: 0.04,
            ..sub(
                vec![l(Wave::Tri, 0.0, 1.0, 0)],
                Adsr::new(0.06, 0.05, 0.9, 0.12),
                4000.0,
                1.0,
                0.8,
                0.0,
                flat,
            )
        }),
        "violin" => Instrument::Sub(SubDef {
            vib_rate: 5.8,
            vib_cents: 14.0,
            vib_delay: 0.2,
            noise: 0.01,
            ..sub(
                vec![l(Wave::Saw, -3.0, 0.5, 0), l(Wave::Saw, 3.0, 0.5, 0)],
                Adsr::new(0.09, 0.1, 0.85, 0.2),
                3200.0,
                0.9,
                0.8,
                1.0,
                Adsr::new(0.09, 0.2, 0.7, 0.2),
            )
        }),
        "cello" => Instrument::Sub(SubDef {
            vib_rate: 5.2,
            vib_cents: 12.0,
            vib_delay: 0.25,
            ..sub(
                vec![l(Wave::Saw, -3.0, 0.5, 0), l(Wave::Saw, 3.0, 0.5, 0)],
                Adsr::new(0.1, 0.1, 0.85, 0.25),
                1500.0,
                0.8,
                0.8,
                1.0,
                Adsr::new(0.1, 0.2, 0.7, 0.25),
            )
        }),
        "pluck" | "guitar" | "harp" => Instrument::Pluck {
            decay: 0.996,
            bright: 0.6,
        },
        "bell" => Instrument::Fm {
            ratio: 3.5,
            index: 5.0,
            fade: 1.2,
            amp: Adsr::new(0.002, 1.5, 0.0, 0.5),
        },
        "epiano" => Instrument::Fm {
            ratio: 1.0,
            index: 2.0,
            fade: 0.6,
            amp: Adsr::new(0.003, 1.0, 0.2, 0.3),
        },
        "organ" => Instrument::Additive {
            partials: vec![
                (0.5, 0.6, 0.0),
                (1.0, 1.0, 0.0),
                (2.0, 0.8, 0.0),
                (3.0, 0.5, 0.0),
                (4.0, 0.4, 0.0),
                (6.0, 0.25, 0.0),
                (8.0, 0.2, 0.0),
            ],
            amp: Adsr::new(0.01, 0.0, 1.0, 0.08),
        },
        "piano" => Instrument::Additive {
            partials: vec![
                (1.0, 1.0, 1.8),
                (2.0, 0.55, 1.2),
                (3.0, 0.35, 0.9),
                (4.0, 0.25, 0.7),
                (5.0, 0.15, 0.5),
                (6.0, 0.1, 0.4),
            ],
            amp: Adsr::new(0.003, 0.0, 1.0, 0.25),
        },
        "drumkit" | "drums" => Instrument::Drums,
        _ => return None,
    })
}

pub const BUILTIN_NAMES: &[&str] = &[
    "sine",
    "saw_lead",
    "square_lead",
    "pad",
    "strings",
    "synth_strings",
    "synth_bass",
    "bass",
    "brass",
    "flute",
    "violin",
    "cello",
    "pluck",
    "guitar",
    "harp",
    "bell",
    "epiano",
    "organ",
    "piano",
    "drumkit",
    "drums",
];

fn gm_drum(note: u8, rate: f64, vel: f64, seed: u64) -> Vec<f32> {
    let mut n = Noise::new(seed);
    let len = |secs: f64| (secs * rate) as usize;
    let mut out = vec![];
    match note {
        36 | 35 => {
            // a sine falling from 140 Hz to 48 Hz with a short click
            let mut ph = 0.0;
            for i in 0..len(0.45) {
                let t = i as f64 / rate;
                let f = 48.0 + 92.0 * (-t * 38.0).exp();
                ph += f / rate;
                let env = (-t * 7.0).exp();
                let click = (-t * 400.0).exp() * n.next() * 0.35;
                out.push(((ph * TAU).sin() * env + click) as f32 * vel as f32);
            }
        }
        38 | 40 => {
            let mut ph = 0.0;
            let mut prev = 0.0;
            for i in 0..len(0.3) {
                let t = i as f64 / rate;
                ph += (185.0 + 60.0 * (-t * 30.0).exp()) / rate;
                let x = n.next();
                let hp = x - prev * 0.6; // a rough high-pass
                prev = x;
                let v = (ph * TAU).sin() * (-t * 22.0).exp() * 0.5 + hp * (-t * 17.0).exp() * 0.7;
                out.push(v as f32 * vel as f32);
            }
        }
        42 | 44 | 46 | 49 | 51 | 57 => {
            let decay = match note {
                42 | 44 => 60.0,
                46 => 9.0,
                _ => 2.2,
            };
            let secs = match note {
                42 | 44 => 0.12,
                46 => 0.6,
                _ => 2.2,
            };
            let (mut p1, mut p2) = (0.0, 0.0);
            for i in 0..len(secs) {
                let t = i as f64 / rate;
                let x = n.next();
                let hp = x - p1 + 0.35 * p2; // brighter than white noise
                p2 = p1;
                p1 = x;
                out.push((hp * (-t * decay).exp() * 0.5) as f32 * vel as f32);
            }
        }
        45 | 47 | 48 | 50 => {
            let base = match note {
                45 => 90.0,
                47 => 110.0,
                48 => 135.0,
                _ => 160.0,
            };
            let mut ph = 0.0;
            for i in 0..len(0.5) {
                let t = i as f64 / rate;
                ph += (base + 70.0 * (-t * 25.0).exp()) / rate;
                out.push(((ph * TAU).sin() * (-t * 9.0).exp()) as f32 * vel as f32 * 0.8);
            }
        }
        39 => {
            for i in 0..len(0.25) {
                let t = i as f64 / rate;
                let burst = ((t * 100.0) % 1.0 < 0.3 && t < 0.03) as u8 as f64;
                let env = burst * (-(t % 0.01) * 300.0).exp()
                    + (-t * 18.0).exp() * 0.5 * (t >= 0.03) as u8 as f64;
                out.push((n.next() * env * 0.7) as f32 * vel as f32);
            }
        }
        _ => {
            // a rim shot / anything else: a short tone and click
            let mut ph = 0.0;
            for i in 0..len(0.08) {
                let t = i as f64 / rate;
                ph += 820.0 / rate;
                out.push(
                    (((ph * TAU).sin() * 0.6 + n.next() * 0.4) * (-t * 80.0).exp()) as f32
                        * vel as f32,
                );
            }
        }
    }
    out
}

/// Render one note (`len` seconds held) to a mono buffer, including its release.
pub fn render_note(
    inst: &Instrument,
    note: u8,
    vel: f64,
    len: f64,
    rate: f64,
    seed: u64,
) -> Vec<f32> {
    let hz = midi_hz(note as f64);
    let amp_vel = vel.clamp(0.0, 1.0).powf(1.2);
    match inst {
        Instrument::Drums => gm_drum(note, rate, amp_vel, seed),
        Instrument::Sampled(set) => set.render(note, vel, len, rate),
        Instrument::Sub(d) => {
            let total = len + d.amp.r;
            let n = (total * rate).ceil() as usize;
            let mut out = Vec::with_capacity(n);
            let mut phases = vec![0.0f64; d.layers.len()];
            let mut lp = Lowpass::new();
            let mut noise = Noise::new(seed);
            let mut vib_ph = 0.0;
            for i in 0..n {
                let t = i as f64 / rate;
                let vib = if d.vib_cents > 0.0 && t >= d.vib_delay {
                    vib_ph += d.vib_rate / rate;
                    (vib_ph * TAU).sin() * d.vib_cents * ((t - d.vib_delay) / 0.3).min(1.0)
                } else {
                    0.0
                };
                let mut s = 0.0;
                for (k, ly) in d.layers.iter().enumerate() {
                    let f = hz * 2f64.powf(ly.octave as f64 + (ly.detune_cents + vib) / 1200.0);
                    let dt = f / rate;
                    phases[k] = (phases[k] + dt) % 1.0;
                    s += osc(ly.wave, phases[k], dt) * ly.gain;
                }
                s += noise.next() * d.noise;
                let env = d.amp.level(t, len);
                let fe = d.filt_adsr.level(t, len);
                let cutoff = d.cutoff
                    * (hz / 261.63).powf(d.key_track)
                    * 2f64.powf(d.filt_env * (fe - 0.5) * 2.0);
                let y = lp.process(s, cutoff.clamp(40.0, rate * 0.45), d.q, rate);
                out.push((y * env * amp_vel * 0.5) as f32);
            }
            out
        }
        Instrument::Pluck { decay, bright } => {
            let period = ((rate / hz).round() as usize).max(2);
            let mut noise = Noise::new(seed);
            let mut line: Vec<f64> = (0..period).map(|_| noise.next()).collect();
            let total = ((len + 0.3) * rate).ceil() as usize;
            let mut out = Vec::with_capacity(total);
            let mut idx = 0;
            let blend = 0.5 + 0.5 * (1.0 - bright.clamp(0.0, 1.0)) * 0.9;
            for i in 0..total {
                let t = i as f64 / rate;
                let next = (idx + 1) % period;
                let avg = blend * line[idx] + (1.0 - blend) * line[next];
                let damp = if t > len { 0.9 } else { *decay };
                let y = line[idx];
                line[idx] = avg * damp;
                idx = next;
                out.push((y * amp_vel * 0.5) as f32);
            }
            out
        }
        Instrument::Fm {
            ratio,
            index,
            fade,
            amp,
        } => {
            let n = ((len + amp.r) * rate).ceil() as usize;
            let (mut pc, mut pm) = (0.0, 0.0);
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let t = i as f64 / rate;
                pm += hz * ratio / rate;
                pc += hz / rate;
                let idx = index * (-t / fade.max(1e-3)).exp();
                let y = (pc * TAU + idx * (pm * TAU).sin()).sin();
                out.push((y * amp.level(t, len) * amp_vel * 0.5) as f32);
            }
            out
        }
        Instrument::Additive { partials, amp } => {
            let n = ((len + amp.r) * rate).ceil() as usize;
            let mut phases = vec![0.0f64; partials.len()];
            let mut out = Vec::with_capacity(n);
            let norm: f64 = partials.iter().map(|p| p.1).sum::<f64>().max(1.0);
            for i in 0..n {
                let t = i as f64 / rate;
                let mut s = 0.0;
                for (k, (mult, gain, dec)) in partials.iter().enumerate() {
                    let f = hz * mult;
                    if f >= rate * 0.45 {
                        continue;
                    }
                    phases[k] = (phases[k] + f / rate) % 1.0;
                    let env = if *dec > 0.0 { (-t / dec).exp() } else { 1.0 };
                    s += (phases[k] * TAU).sin() * gain * env;
                }
                out.push((s / norm * amp.level(t, len) * amp_vel * 0.6) as f32);
            }
            out
        }
    }
}
