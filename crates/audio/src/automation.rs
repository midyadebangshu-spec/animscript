//! Automation lanes: a value that changes over a track's time. A lane is written as points
//! `bar:value` separated by spaces (`"0:0.2 4:1 8:0.5"`; a bar may be fractional, bar 0 is the start), each point
//! optionally ending in `s` (ease smoothly to the next point) or `h` (hold until the next point); the default is a
//! straight line. Before the first point and after the last the value is held.
use crate::{Meter, ParseError, PPQ};

type PRes<T> = Result<T, ParseError>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    Linear,
    Smooth,
    Hold,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    /// (tick, value, how to get to the next point)
    pub points: Vec<(f64, f64, Ease)>,
}

impl Lane {
    /// Parse `src`; every value must lie in `lo..=hi`. `what` names the lane in errors.
    pub fn parse(src: &str, meter: Meter, lo: f64, hi: f64, what: &str) -> PRes<Lane> {
        let bar = meter.bar_ticks() as f64;
        let mut points: Vec<(f64, f64, Ease)> = vec![];
        for tok in src.split_whitespace() {
            let err = |m: &str| ParseError::new(format!("{what} lane, `{tok}`: {m}"));
            let (mut body, mut ease) = (tok, Ease::Linear);
            if let Some(b) = body.strip_suffix('s') {
                body = b;
                ease = Ease::Smooth;
            } else if let Some(b) = body.strip_suffix('h') {
                body = b;
                ease = Ease::Hold;
            }
            let (pos, val) = body
                .split_once(':')
                .ok_or_else(|| err("a point is `bar:value`, like 4:0.8"))?;
            let pos: f64 = pos.parse().map_err(|_| err("the bar must be a number"))?;
            let val: f64 = val.parse().map_err(|_| err("the value must be a number"))?;
            if pos < 0.0 {
                return Err(err("the bar cannot be negative"));
            }
            if !(lo..=hi).contains(&val) {
                return Err(err(&format!("the value must be between {lo} and {hi}")));
            }
            let tick = pos * bar;
            if let Some(last) = points.last() {
                if tick <= last.0 {
                    return Err(err("points must go forward in time"));
                }
            }
            points.push((tick, val, ease));
        }
        if points.is_empty() {
            return Err(ParseError::new(format!("{what} lane has no points")));
        }
        Ok(Lane { points })
    }

    /// The value at `tick` (ticks may be fractional).
    pub fn at_ticks(&self, tick: f64) -> f64 {
        let p = &self.points;
        if tick <= p[0].0 {
            return p[0].1;
        }
        if tick >= p[p.len() - 1].0 {
            return p[p.len() - 1].1;
        }
        let k = p.iter().rposition(|q| q.0 <= tick).expect("inside");
        let (t0, v0, ease) = p[k];
        let (t1, v1, _) = p[k + 1];
        let u = (tick - t0) / (t1 - t0);
        match ease {
            Ease::Hold => v0,
            Ease::Linear => v0 + (v1 - v0) * u,
            Ease::Smooth => v0 + (v1 - v0) * u * u * (3.0 - 2.0 * u),
        }
    }

    /// The value `secs` seconds into the track at `tempo` bpm.
    pub fn at_secs(&self, secs: f64, tempo: f64) -> f64 {
        self.at_ticks(secs * tempo / 60.0 * PPQ as f64)
    }
}

/// The lanes of a track; a missing lane leaves its quantity alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Automation {
    /// Gain (0 to 4).
    pub volume: Option<Lane>,
    /// -1 (left) to 1 (right).
    pub pan: Option<Lane>,
    /// A multiplier on the filter cutoff of subtractive voices (0.05 to 8; 1 leaves it).
    pub cutoff: Option<Lane>,
    /// Brightness 0 to 1 (0.5 leaves it): opens or closes filters and, per note, FM and additive spectra.
    pub brightness: Option<Lane>,
    /// How much of the track goes to the reverb (0 to 2; the default is 1).
    pub reverb: Option<Lane>,
}

impl Automation {
    pub fn is_empty(&self) -> bool {
        self.volume.is_none()
            && self.pan.is_none()
            && self.cutoff.is_none()
            && self.brightness.is_none()
            && self.reverb.is_none()
    }

    /// The tone multiplier at `secs`: the cutoff lane times two to the power of four times the brightness's
    /// distance from 0.5 (a full swing is four octaves each way).
    pub fn tone_at(&self, secs: f64, tempo: f64) -> f64 {
        let c = self.cutoff.as_ref().map_or(1.0, |l| l.at_secs(secs, tempo));
        let b = self
            .brightness
            .as_ref()
            .map_or(0.5, |l| l.at_secs(secs, tempo));
        c * 2f64.powf((b - 0.5) * 4.0)
    }
}
