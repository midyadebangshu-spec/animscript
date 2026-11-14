//! Mixing a score down to stereo: every note of every track is synthesised (or sampled), panned and
//! summed, the reverb and delay are added and the result is soft-limited. A pure function of its inputs.
use crate::fx::{delay, Reverb};
use crate::synth::{builtin, render_note, Instrument};
use crate::{ticks_to_seconds, Score};
use std::collections::BTreeMap;

/// Named instruments: the built-in synths plus any sampled sets added to the bank.
#[derive(Clone, Debug, Default)]
pub struct Bank {
    extra: BTreeMap<String, Instrument>,
}

impl Bank {
    pub fn new() -> Bank {
        Bank::default()
    }
    pub fn insert(&mut self, name: &str, inst: Instrument) {
        self.extra.insert(name.to_string(), inst);
    }
    pub fn get(&self, name: &str) -> Option<Instrument> {
        self.extra.get(name).cloned().or_else(|| builtin(name))
    }
    /// Load an SFZ library as the instrument `name`.
    pub fn load_sfz(&mut self, name: &str, path: &std::path::Path) -> Result<(), String> {
        let set = crate::sfz::load_sfz(path)?;
        self.insert(name, Instrument::Sampled(set));
        Ok(())
    }
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = crate::synth::BUILTIN_NAMES
            .iter()
            .map(|s| s.to_string())
            .collect();
        v.extend(self.extra.keys().cloned());
        v.sort();
        v.dedup();
        v
    }
}

#[derive(Clone, Debug)]
pub struct Mix {
    pub rate: u32,
    /// Wet level of the reverb (0 to 1) and the room size.
    pub reverb: f64,
    pub room: f64,
    /// Feedback delay: wet level, time in seconds and feedback.
    pub delay: f64,
    pub delay_time: f64,
    pub delay_feedback: f64,
    pub master: f64,
}
impl Default for Mix {
    fn default() -> Mix {
        Mix {
            rate: 44100,
            reverb: 0.0,
            room: 0.6,
            delay: 0.0,
            delay_time: 0.375,
            delay_feedback: 0.4,
            master: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stereo {
    pub rate: u32,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

/// An unknown instrument name is the only way rendering fails.
pub fn render_score(score: &Score, mix: &Mix, bank: &Bank) -> Result<Stereo, String> {
    let rate = mix.rate as f64;
    let tail = if mix.reverb > 0.0 {
        2.5
    } else if mix.delay > 0.0 {
        1.5
    } else {
        0.6
    };
    let n = ((score.length_seconds() + tail) * rate).ceil() as usize;
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    for (ti, tr) in score.tracks.iter().enumerate() {
        let inst = bank.get(&tr.instrument).ok_or_else(|| {
            format!(
                "unknown instrument `{}` (built in: {})",
                tr.instrument,
                bank.names().join(", ")
            )
        })?;
        let ang = (tr.pan.clamp(-1.0, 1.0) + 1.0) * std::f64::consts::FRAC_PI_4;
        let (gl, gr) = (
            (ang.cos() * tr.volume * 0.9) as f32,
            (ang.sin() * tr.volume * 0.9) as f32,
        );
        for (ei, e) in tr.events.iter().enumerate() {
            let start = (ticks_to_seconds(e.start, score.tempo) * rate).round() as usize;
            let len = ticks_to_seconds(e.len, score.tempo);
            let seed = (ti as u64) << 32 | ei as u64;
            let note = render_note(&inst, e.note, e.vel, len, rate, seed);
            for (k, s) in note.iter().enumerate() {
                let i = start + k;
                if i >= n {
                    break;
                }
                l[i] += s * gl;
                r[i] += s * gr;
            }
        }
    }
    if mix.delay > 0.0 {
        let (dl, dr) = (
            delay(&l, rate, mix.delay_time, mix.delay_feedback),
            delay(&r, rate, mix.delay_time * 1.5, mix.delay_feedback),
        );
        for i in 0..n {
            l[i] += dl[i] * mix.delay as f32;
            r[i] += dr[i] * mix.delay as f32;
        }
    }
    if mix.reverb > 0.0 {
        let mut rv = Reverb::new(rate, mix.room, 0.3);
        let (wl, wr) = rv.process(&l, &r);
        for i in 0..n {
            l[i] += wl[i] * mix.reverb as f32 * 3.0;
            r[i] += wr[i] * mix.reverb as f32 * 3.0;
        }
    }
    let m = mix.master as f32;
    for i in 0..n {
        l[i] = (l[i] * m).tanh();
        r[i] = (r[i] * m).tanh();
    }
    Ok(Stereo {
        rate: mix.rate,
        left: l,
        right: r,
    })
}

impl Stereo {
    pub fn to_wav(&self) -> Vec<u8> {
        crate::wav::write_wav16(self.rate, &self.left, &self.right)
    }
    pub fn seconds(&self) -> f64 {
        self.left.len() as f64 / self.rate as f64
    }
    /// The root mean square of both channels over a time window (seconds).
    pub fn rms(&self, from: f64, to: f64) -> f64 {
        let (a, b) = (
            (from * self.rate as f64) as usize,
            ((to * self.rate as f64) as usize).min(self.left.len()),
        );
        if b <= a {
            return 0.0;
        }
        let s: f64 = (a..b)
            .map(|i| (self.left[i] as f64).powi(2) + (self.right[i] as f64).powi(2))
            .sum();
        (s / (2 * (b - a)) as f64).sqrt()
    }
}
