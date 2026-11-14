//! A minimal WAV reader and writer (PCM 8/16/24/32-bit integer and 32-bit float, any channel count),
//! enough for sample libraries and for the rendered output. No dependencies.

pub struct Wav {
    pub rate: u32,
    pub channels: usize,
    /// Interleaved samples in -1..1.
    pub data: Vec<f32>,
    /// The first sustain loop (start, end frames) stored in the file's `smpl` chunk, if any.
    pub loop_points: Option<(usize, usize)>,
}

impl Wav {
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels.max(1)
    }
    /// One channel as a mono signal (the mean of all channels when `channel` is `None`).
    pub fn mono(&self) -> Vec<f32> {
        let c = self.channels.max(1);
        self.data.chunks(c).map(|f| f.iter().sum::<f32>() / c as f32).collect()
    }
}

/// 16-bit stereo PCM WAV bytes. Samples are clamped to -1..1.
pub fn write_wav16(rate: u32, left: &[f32], right: &[f32]) -> Vec<u8> {
    let n = left.len().min(right.len());
    let data_len = (n * 4) as u32;
    let mut out = Vec::with_capacity(44 + n * 4);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..n {
        for s in [left[i], right[i]] {
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

pub fn read_wav(bytes: &[u8]) -> Result<Wav, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let (mut fmt, mut data): (Option<(u16, usize, u32, u16)>, Option<&[u8]>) = (None, None);
    let mut loop_points = None;
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let len = u32_at(i + 4) as usize;
        let body = i + 8;
        let end = (body + len).min(bytes.len());
        match id {
            b"fmt " if end - body >= 16 => {
                let mut tag = u16_at(body);
                if tag == 0xFFFE && end - body >= 26 {
                    tag = u16_at(body + 24); // WAVE_FORMAT_EXTENSIBLE: the sub-format's first two bytes
                }
                fmt = Some((tag, u16_at(body + 2) as usize, u32_at(body + 4), u16_at(body + 14)));
            }
            b"data" => data = Some(&bytes[body..end]),
            b"smpl" if end - body >= 36 + 24 && u32_at(body + 28) >= 1 => {
                loop_points = Some((u32_at(body + 36 + 8) as usize, u32_at(body + 36 + 12) as usize));
            }
            _ => {}
        }
        i = body + len + (len & 1);
    }
    let (tag, channels, rate, bits) = fmt.ok_or("no fmt chunk")?;
    let data = data.ok_or("no data chunk")?;
    if channels == 0 {
        return Err("zero channels".into());
    }
    let samples: Vec<f32> = match (tag, bits) {
        (1, 8) => data.iter().map(|b| (*b as f32 - 128.0) / 128.0).collect(),
        (1, 16) => data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect(),
        (1, 24) => data.chunks_exact(3).map(|c| (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8_388_608.0).collect(),
        (1, 32) => data.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2_147_483_648.0).collect(),
        (3, 32) => data.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
        _ => return Err(format!("unsupported WAV format (tag {tag}, {bits} bits); use PCM 8/16/24/32 or float32")),
    };
    Ok(Wav { rate, channels, data: samples, loop_points })
}
