//! Procedural texture generation for the `add_texture` MCP tool.

use std::io::Cursor;

use image::{ImageFormat, Rgba, RgbaImage};
use serde::Deserialize;

#[derive(Deserialize, Debug)]
#[serde(default)]
pub struct Procedural {
    /// noise | fbm | voronoi | checker | gradient | radial | stripes
    pub kind: String,
    pub size: u32,
    /// Feature scale (cells / octave base frequency / stripe count).
    pub scale: f32,
    pub seed: u32,
    /// Linear 0..1 RGBA colors mapped to t=0 and t=1.
    pub color_a: [f32; 4],
    pub color_b: [f32; 4],
}

impl Default for Procedural {
    fn default() -> Self {
        Self {
            kind: "fbm".into(),
            size: 256,
            scale: 8.0,
            seed: 0,
            color_a: [0.0, 0.0, 0.0, 1.0],
            color_b: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

pub fn generate_png(p: &Procedural) -> Result<Vec<u8>, String> {
    if !(4..=2048).contains(&p.size) {
        return Err("size must be between 4 and 2048".into());
    }
    let size = p.size;
    let seed = p.seed as f32 * 17.13;
    let scale = p.scale.max(0.01);
    let field: Box<dyn Fn(f32, f32) -> f32> = match p.kind.as_str() {
        // Tiling variants: noise coordinates wrap at `scale` cells.
        "noise" => Box::new(move |u, v| value_noise(u * scale, v * scale, scale as i32, seed)),
        "fbm" => Box::new(move |u, v| {
            let mut sum = 0.0;
            let mut amp = 0.5;
            let mut freq = scale;
            for octave in 0..6 {
                sum += amp * value_noise(u * freq, v * freq, freq as i32, seed + octave as f32);
                freq *= 2.0;
                amp *= 0.5;
            }
            sum / 0.984
        }),
        "voronoi" => Box::new(move |u, v| voronoi(u * scale, v * scale, scale as i32, seed)),
        "checker" => Box::new(move |u, v| {
            (((u * scale).floor() + (v * scale).floor()) as i32).rem_euclid(2) as f32
        }),
        "gradient" => Box::new(|u, _| u),
        "radial" => Box::new(|u, v| ((u - 0.5).hypot(v - 0.5) * 2.0).min(1.0)),
        "stripes" => Box::new(move |u, _| 0.5 + 0.5 * (u * scale * std::f32::consts::TAU).sin()),
        other => {
            return Err(format!(
                "unknown procedural kind `{other}` (noise|fbm|voronoi|checker|gradient|radial|stripes)"
            ));
        }
    };
    let mut img = RgbaImage::new(size, size);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let t = field(
            (x as f32 + 0.5) / size as f32,
            (y as f32 + 0.5) / size as f32,
        )
        .clamp(0.0, 1.0);
        let c: [u8; 4] = std::array::from_fn(|i| {
            let lin = p.color_a[i] + (p.color_b[i] - p.color_a[i]) * t;
            // Alpha is linear; colors are stored as sRGB.
            let v = if i == 3 { lin } else { linear_to_srgb(lin) };
            (v.clamp(0.0, 1.0) * 255.0).round() as u8
        });
        *px = Rgba(c);
    }
    let mut bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn hash(x: i32, y: i32, seed: f32) -> f32 {
    let h = (x as f32 * 127.1 + y as f32 * 311.7 + seed * 74.7).sin() * 43758.547;
    h - h.floor()
}

fn wrap(i: i32, period: i32) -> i32 {
    if period > 0 { i.rem_euclid(period) } else { i }
}

fn value_noise(x: f32, y: f32, period: i32, seed: f32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let h = |dx, dy| hash(wrap(ix + dx, period), wrap(iy + dy, period), seed);
    let a = h(0, 0) + (h(1, 0) - h(0, 0)) * ux;
    let b = h(0, 1) + (h(1, 1) - h(0, 1)) * ux;
    a + (b - a) * uy
}

fn voronoi(x: f32, y: f32, period: i32, seed: f32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let mut best = f32::MAX;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (cx, cy) = (wrap(ix + dx, period), wrap(iy + dy, period));
            let px = dx as f32 + hash(cx, cy, seed) - fx;
            let py = dy as f32 + hash(cy, cx, seed + 1.0) - fy;
            best = best.min(px * px + py * py);
        }
    }
    best.sqrt().min(1.0)
}
