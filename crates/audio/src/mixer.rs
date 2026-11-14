//! Putting rendered scores on a timeline: each cue starts a rendered score at a time, with a volume, fades,
//! optional looping and an optional stop. The sum is the soundtrack.
use crate::Stereo;

#[derive(Clone, Debug)]
pub struct Cue {
    /// Index into the list of rendered sources.
    pub source: usize,
    pub start: f64,
    pub volume: f64,
    pub fade_in: f64,
    pub fade_out: f64,
    /// Repeat every `loop_len` seconds until stopped or the end.
    pub looping: bool,
    pub loop_len: f64,
    pub stop: Option<f64>,
    /// Start this far into the source.
    pub offset: f64,
}

impl Cue {
    pub fn new(source: usize, start: f64) -> Cue {
        Cue {
            source,
            start,
            volume: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            looping: false,
            loop_len: 0.0,
            stop: None,
            offset: 0.0,
        }
    }
}

/// The gain of a cue at `t` seconds after its start (stop time `stop` relative to the start).
fn gain(c: &Cue, t: f64, stop: Option<f64>) -> f64 {
    let mut g = c.volume;
    if c.fade_in > 0.0 && t < c.fade_in {
        g *= t / c.fade_in;
    }
    if let Some(s) = stop {
        if t >= s {
            if c.fade_out <= 0.0 {
                return 0.0;
            }
            g *= (1.0 - (t - s) / c.fade_out).max(0.0);
        }
    }
    g
}

pub fn mix_cues(sources: &[Stereo], cues: &[Cue], total: f64, rate: u32) -> Stereo {
    let n = (total * rate as f64).ceil() as usize;
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    for c in cues {
        let Some(src) = sources.get(c.source) else {
            continue;
        };
        let stop = c.stop.map(|s| (s - c.start).max(0.0));
        let end_t = match stop {
            Some(s) => s + c.fade_out,
            None => f64::INFINITY,
        };
        let first = (c.start * rate as f64).round() as i64;
        let off = (c.offset * rate as f64).round() as usize;
        let period = if c.looping && c.loop_len > 0.0 {
            Some((c.loop_len * rate as f64).round() as i64)
        } else {
            None
        };
        let mut iter = 0i64;
        loop {
            let begin = first + iter * period.unwrap_or(0);
            if begin >= n as i64 || (begin - first) as f64 / rate as f64 >= end_t {
                break;
            }
            let skip = if iter == 0 { off } else { 0 };
            for (k, i) in (skip..src.left.len()).enumerate() {
                let pos = begin + k as i64;
                if pos < 0 {
                    continue;
                }
                let pos = pos as usize;
                if pos >= n {
                    break;
                }
                let t = (pos as i64 - first) as f64 / rate as f64;
                if t >= end_t {
                    break;
                }
                let g = gain(c, t, stop) as f32;
                l[pos] += src.left[i] * g;
                r[pos] += src.right[i] * g;
            }
            match period {
                Some(_) => iter += 1,
                None => break,
            }
        }
    }
    for i in 0..n {
        l[i] = l[i].clamp(-1.0, 1.0);
        r[i] = r[i].clamp(-1.0, 1.0);
    }
    Stereo {
        rate,
        left: l,
        right: r,
    }
}
