//! Recorded instruments: zones of samples mapped over keys and velocities, played back at the pitch of the
//! note asked for (linear interpolation), looped while the note is held and faded out on release.
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Zone {
    pub lo_key: u8,
    pub hi_key: u8,
    pub lo_vel: u8,
    pub hi_vel: u8,
    /// The key the recording was made at.
    pub root: u8,
    pub rate: u32,
    pub data: Arc<Vec<f32>>,
    /// Sustain loop (frame range) used while the note is held.
    pub loop_range: Option<(usize, usize)>,
    pub gain: f64,
}

#[derive(Clone, Debug, Default)]
pub struct SampleSet {
    pub zones: Vec<Zone>,
    /// Release fade in seconds.
    pub release: f64,
}

impl SampleSet {
    pub fn new(zones: Vec<Zone>) -> SampleSet {
        SampleSet {
            zones,
            release: 0.25,
        }
    }

    /// The zone for a note and velocity (0 to 1): the first whose key and velocity ranges contain it, else the
    /// one with the nearest root.
    pub fn pick(&self, note: u8, vel: f64) -> Option<&Zone> {
        let v = (vel.clamp(0.0, 1.0) * 127.0).round().max(1.0) as u8;
        self.zones
            .iter()
            .find(|z| (z.lo_key..=z.hi_key).contains(&note) && (z.lo_vel..=z.hi_vel).contains(&v))
            .or_else(|| {
                self.zones
                    .iter()
                    .filter(|z| (z.lo_vel..=z.hi_vel).contains(&v))
                    .min_by_key(|z| (z.root as i32 - note as i32).abs())
            })
            .or_else(|| self.zones.first())
    }

    pub fn render(&self, note: u8, vel: f64, len: f64, rate: f64) -> Vec<f32> {
        let Some(z) = self.pick(note, vel) else {
            return vec![];
        };
        let step = 2f64.powf((note as f64 - z.root as f64) / 12.0) * z.rate as f64 / rate;
        let total = ((len + self.release) * rate).ceil() as usize;
        let gain = vel.clamp(0.0, 1.0).powf(1.2) * z.gain;
        let n = z.data.len();
        let mut out = Vec::with_capacity(total.min(n * 4 + 1));
        let mut pos = 0.0f64;
        for i in 0..total {
            let t = i as f64 / rate;
            if let Some((ls, le)) = z.loop_range {
                if le > ls && pos >= le as f64 && t < len + self.release {
                    pos = ls as f64 + (pos - le as f64);
                }
            }
            let k = pos as usize;
            if k + 1 >= n {
                break;
            }
            let f = (pos - k as f64) as f32;
            let s = z.data[k] * (1.0 - f) + z.data[k + 1] * f;
            let env = if t < len {
                1.0
            } else {
                (1.0 - (t - len) / self.release.max(1e-3)).max(0.0)
            };
            out.push(s * (env * gain) as f32);
            pos += step;
        }
        out
    }
}
