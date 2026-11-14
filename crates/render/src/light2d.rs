//! Lighting for flat 2D shapes marked `lit`: the shape is drawn flat into a layer, its edges are
//! rounded for shading by a bevel profile derived from the distance to the edge, then each covered
//! pixel is lit like a surface at the shape's depth (facing the camera) with shadows from 3D meshes
//! and from 2D occluders. Rows are shaded in parallel; every pixel is independent, so the result
//! does not depend on the thread count.
use crate::bvh::Bvh;
use crate::light::{self, Lt, OccLayer, Surf};
use serde_json::Value;
use tiny_skia::{Pixmap, Transform};

/// Distance (in pixels) from every covered pixel to the nearest uncovered one (3-4 chamfer).
fn distance(mask: &[bool], w: usize, h: usize) -> Vec<f32> {
    let inf = 1e9f32;
    let mut d: Vec<f32> = mask.iter().map(|&m| if m { inf } else { 0.0 }).collect();
    let (a, b) = (1.0f32, std::f32::consts::SQRT_2);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut v = d[i];
            if x > 0 {
                v = v.min(d[i - 1] + a);
            }
            if y > 0 {
                v = v.min(d[i - w] + a);
                if x > 0 {
                    v = v.min(d[i - w - 1] + b);
                }
                if x + 1 < w {
                    v = v.min(d[i - w + 1] + b);
                }
            }
            d[i] = v;
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            let mut v = d[i];
            if x + 1 < w {
                v = v.min(d[i + 1] + a);
            }
            if y + 1 < h {
                v = v.min(d[i + w] + a);
                if x + 1 < w {
                    v = v.min(d[i + w + 1] + b);
                }
                if x > 0 {
                    v = v.min(d[i + w - 1] + b);
                }
            }
            d[i] = v;
        }
    }
    d
}

/// Light every covered pixel of `layer` (premultiplied RGBA, drawn flat) in place.
#[allow(clippy::too_many_arguments)]
pub fn light_layer(
    layer: &mut Pixmap,
    sh: &Value,
    lights: &[Lt],
    bvh: &Bvh,
    occ: &[OccLayer],
    tf: Transform,
    exposure: f64,
    samples: usize,
) {
    let (w, h) = (layer.width() as usize, layer.height() as usize);
    let lit = &sh["lit"];
    let scale = (tf.sx as f64).hypot(tf.ky as f64).max(1e-9);
    let bevel = lit["bevel"].as_f64().unwrap_or(0.0) * scale;
    let rough = lit["surface"][0].as_f64().unwrap_or(0.6);
    let metal = lit["surface"][1].as_f64().unwrap_or(0.0);
    let emissive = lit["surface"][2].as_f64().unwrap_or(0.0);
    let depth = lit["depth"].as_f64().unwrap_or(0.0);
    let data = layer.data().to_vec();
    let mask: Vec<bool> = data.chunks(4).map(|p| p[3] >= 128).collect();
    let dist = if bevel > 0.0 {
        distance(&mask, w, h)
    } else {
        vec![]
    };
    // height of the rounded edge above the sheet, in pixels
    let height = |x: usize, y: usize| -> f64 {
        if bevel <= 0.0 {
            return 0.0;
        }
        let d = dist[y * w + x] as f64;
        bevel * ((d / bevel).min(1.0) * std::f64::consts::FRAC_PI_2).sin()
    };
    // canvas <- pixel
    let det = (tf.sx * tf.sy - tf.kx * tf.ky) as f64;
    let inv = |px: f64, py: f64| -> (f64, f64) {
        let (dx, dy) = (px - tf.tx as f64, py - tf.ty as f64);
        (
            (tf.sy as f64 * dx - tf.kx as f64 * dy) / det,
            (-(tf.ky as f64) * dx + tf.sx as f64 * dy) / det,
        )
    };
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(16);
    let rows = h.div_ceil(threads).max(1);
    std::thread::scope(|sc| {
        for (ci, chunk) in layer.data_mut().chunks_mut(rows * w * 4).enumerate() {
            let (data, height) = (&data, &height);
            sc.spawn(move || {
                for (k, px) in chunk.chunks_mut(4).enumerate() {
                    let i = ci * rows * w + k;
                    let (x, y) = (i % w, i / w);
                    let a = data[i * 4 + 3];
                    if a == 0 {
                        continue;
                    }
                    let af = a as f64 / 255.0;
                    let rgb: [f64; 3] = std::array::from_fn(|c| {
                        light::srgb_to_linear(data[i * 4 + c] as f64 / 255.0 / af)
                    });
                    // the slope over a few pixels (wider for a wider bevel), which smooths the chamfer distance's facets
                    let k = ((bevel / 6.0) as usize).clamp(1, 5);
                    let (hx, hy) = if bevel > 0.0 && x >= k && y >= k && x + k < w && y + k < h {
                        let f = 0.5 / k as f64;
                        (
                            (height(x + k, y) - height(x - k, y)) * f,
                            (height(x, y + k) - height(x, y - k)) * f,
                        )
                    } else {
                        (0.0, 0.0)
                    };
                    let l = (hx * hx + hy * hy + 1.0).sqrt();
                    let n = [-hx / l, -hy / l, -1.0 / l];
                    let (cx, cy) = inv(x as f64 + 0.5, y as f64 + 0.5);
                    let pos = [cx, cy, depth];
                    let surf = Surf {
                        pos,
                        n,
                        view: [0.0, 0.0, -1.0],
                        albedo: rgb,
                        rough,
                        metal,
                        emissive,
                        ao: 1.0,
                    };
                    let rad = light::radiance(lights, &surf, &|li| {
                        light::visibility(&lights[li], bvh, occ, samples, pos, n, x, y)
                    });
                    let t = light::tonemap(rad, exposure);
                    for c in 0..3 {
                        px[c] = ((t[c] * af * 255.0) + 0.5).clamp(0.0, 255.0) as u8;
                    }
                }
            });
        }
    });
}
