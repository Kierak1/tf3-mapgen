use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

pub fn write_luma16(path: &Path, data: &[u16], width: u32, height: u32) -> Result<()> {
    let buffer: image::ImageBuffer<image::Luma<u16>, Vec<u16>> =
        image::ImageBuffer::from_raw(width, height, data.to_vec())
            .ok_or_else(|| anyhow::anyhow!("invalid 16-bit image dimensions"))?;
    buffer
        .save_with_format(path, image::ImageFormat::Png)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

pub fn write_luma8(path: &Path, data: &[u8], width: u32, height: u32) -> Result<()> {
    let buffer: image::ImageBuffer<image::Luma<u8>, Vec<u8>> =
        image::ImageBuffer::from_raw(width, height, data.to_vec())
            .ok_or_else(|| anyhow::anyhow!("invalid 8-bit image dimensions"))?;
    buffer
        .save_with_format(path, image::ImageFormat::Png)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

pub fn write_text(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

#[allow(clippy::too_many_arguments)]
pub fn write_preview(
    path: &Path,
    elev: &[f32],
    water: &[bool],
    towns_px: &[(usize, usize)],
    width: usize,
    height: usize,
    min_h: f64,
    max_h: f64,
) -> Result<()> {
    let span = (max_h - min_h).max(1.0) as f32;
    let mut pixels = vec![0u8; width * height * 3];
    for index in 0..width * height {
        let (r, g, b) = if water[index] {
            (35u8, 85u8, 150u8)
        } else {
            let t = ((elev[index] - min_h as f32) / span).clamp(0.0, 1.0);
            land_color(t)
        };
        pixels[index * 3] = r;
        pixels[index * 3 + 1] = g;
        pixels[index * 3 + 2] = b;
    }
    for (col, row) in towns_px {
        for dy in -2i64..=2 {
            for dx in -2i64..=2 {
                if dx * dx + dy * dy > 5 {
                    continue;
                }
                let x = *col as i64 + dx;
                let y = *row as i64 + dy;
                if x >= 0 && y >= 0 && x < width as i64 && y < height as i64 {
                    let index = (y as usize * width + x as usize) * 3;
                    pixels[index] = 230;
                    pixels[index + 1] = 60;
                    pixels[index + 2] = 50;
                }
            }
        }
    }
    let buffer: image::ImageBuffer<image::Rgb<u8>, Vec<u8>> =
        image::ImageBuffer::from_raw(width as u32, height as u32, pixels)
            .ok_or_else(|| anyhow::anyhow!("invalid preview dimensions"))?;
    buffer
        .save_with_format(path, image::ImageFormat::Png)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn land_color(t: f32) -> (u8, u8, u8) {
    let stops: [(f32, (f32, f32, f32)); 5] = [
        (0.0, (74.0, 120.0, 66.0)),
        (0.3, (120.0, 140.0, 82.0)),
        (0.6, (158.0, 140.0, 105.0)),
        (0.85, (175.0, 175.0, 175.0)),
        (1.0, (255.0, 255.0, 255.0)),
    ];
    let mut lower = stops[0];
    let mut upper = stops[stops.len() - 1];
    for pair in stops.windows(2) {
        if t >= pair[0].0 && t <= pair[1].0 {
            lower = pair[0];
            upper = pair[1];
            break;
        }
    }
    let local = if (upper.0 - lower.0).abs() < f32::EPSILON {
        0.0
    } else {
        (t - lower.0) / (upper.0 - lower.0)
    };
    let mix = |a: f32, b: f32| (a + (b - a) * local).round() as u8;
    (
        mix(lower.1 .0, upper.1 .0),
        mix(lower.1 .1, upper.1 .1),
        mix(lower.1 .2, upper.1 .2),
    )
}
