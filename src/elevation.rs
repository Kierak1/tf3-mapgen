use std::time::Duration;

use anyhow::{bail, Context, Result};
use rayon::prelude::*;

use crate::geo::{lat_to_merc_px, lon_to_merc_px, tile_range, Bbox, TILE_SIZE};
use crate::location::USER_AGENT;
use crate::progress::Progress;

const TERRARIUM_URL: &str = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium";
const MAX_TILES: usize = 700;

pub struct DemGrid {
    pub elev: Vec<f32>,
    pub bbox: Bbox,
    pub zoom: u8,
}

#[allow(clippy::too_many_arguments)]
pub fn fetch_dem(
    center_lat: f64,
    center_lon: f64,
    width_m: f64,
    height_m: f64,
    width: usize,
    height: usize,
    requested_zoom: Option<u8>,
    progress: &dyn Progress,
) -> Result<DemGrid> {
    let bbox = Bbox::from_center(center_lat, center_lon, width_m, height_m);
    let (zoom, range) = pick_zoom(&bbox, requested_zoom);
    let (x0, y0, x1, y1) = range;
    let nx = (x1 - x0 + 1) as usize;
    let ny = (y1 - y0 + 1) as usize;
    progress.info(&format!(
        "  elevation: {}x{} tiles at zoom {} ({}x{} px source)",
        nx,
        ny,
        zoom,
        nx * 256,
        ny * 256
    ));

    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(60))
        .build()
        .context("building tile HTTP client")?;

    let indices: Vec<(i64, i64)> = (0..nx * ny)
        .map(|i| (x0 + (i % nx) as i64, y0 + (i / nx) as i64))
        .collect();
    let total = indices.len();
    let finished = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<Result<Vec<f32>>> = indices
        .par_iter()
        .map(|(x, y)| {
            let result = fetch_tile(&client, zoom, *x, *y);
            let done = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if done.is_multiple_of(16) || done == total {
                progress.info(&format!("  elevation tiles: {done}/{total}"));
            }
            result
        })
        .collect();

    let mosaic_width = nx * 256;
    let mosaic_height = ny * 256;
    let mut mosaic = vec![0f32; mosaic_width * mosaic_height];
    let mut missing = 0usize;
    for ((x, y), result) in indices.iter().zip(results) {
        match result {
            Ok(tile) => {
                let ox = ((x - x0) as usize) * 256;
                let oy = ((y - y0) as usize) * 256;
                for row in 0..256 {
                    let src = row * 256;
                    let dst = (oy + row) * mosaic_width + ox;
                    mosaic[dst..dst + 256].copy_from_slice(&tile[src..src + 256]);
                }
            }
            Err(error) => {
                missing += 1;
                progress.warn(&format!("elevation tile {zoom}/{x}/{y} failed: {error}"));
            }
        }
    }
    if missing * 5 > indices.len() {
        bail!(
            "too many elevation tiles failed ({missing}/{}) - check your internet connection",
            indices.len()
        );
    }

    let origin_x = x0 as f64 * TILE_SIZE;
    let origin_y = y0 as f64 * TILE_SIZE;
    let mut elev = vec![0f32; width * height];
    for row in 0..height {
        let lat = bbox.lat_at_row(row as f64 + 0.5, height as f64);
        let my = lat_to_merc_px(lat, zoom) - origin_y;
        for col in 0..width {
            let lon = bbox.lon_at_col(col as f64 + 0.5, width as f64);
            let mx = lon_to_merc_px(lon, zoom) - origin_x;
            elev[row * width + col] = sample_bilinear(&mosaic, mosaic_width, mosaic_height, mx, my);
        }
    }

    Ok(DemGrid { elev, bbox, zoom })
}

fn pick_zoom(bbox: &Bbox, requested: Option<u8>) -> (u8, (i64, i64, i64, i64)) {
    if let Some(zoom) = requested {
        let zoom = zoom.clamp(8, 15);
        return (zoom, tile_range(bbox, zoom));
    }
    for zoom in (12..=15u8).rev() {
        let range = tile_range(bbox, zoom);
        let count = ((range.2 - range.0 + 1) * (range.3 - range.1 + 1)) as usize;
        if count <= MAX_TILES {
            return (zoom, range);
        }
    }
    (12, tile_range(bbox, 12))
}

fn fetch_tile(client: &reqwest::blocking::Client, zoom: u8, x: i64, y: i64) -> Result<Vec<f32>> {
    let url = format!("{TERRARIUM_URL}/{zoom}/{x}/{y}.png");
    let mut last_error = None;
    for attempt in 0..3 {
        match client
            .get(&url)
            .send()
            .and_then(|response| response.error_for_status())
        {
            Ok(response) => {
                let bytes = response.bytes().context("reading tile body")?;
                let image = image::load_from_memory(&bytes)
                    .context("decoding terrarium tile")?
                    .to_rgb8();
                let mut elevations = Vec::with_capacity((image.width() * image.height()) as usize);
                for pixel in image.pixels() {
                    elevations.push(
                        pixel[0] as f32 * 256.0 + pixel[1] as f32 + pixel[2] as f32 / 256.0
                            - 32768.0,
                    );
                }
                return Ok(elevations);
            }
            Err(error) => {
                let not_found = error
                    .status()
                    .map(|status| status == reqwest::StatusCode::NOT_FOUND)
                    .unwrap_or(false);
                last_error = Some(error);
                if not_found {
                    break;
                }
                std::thread::sleep(Duration::from_millis(400 * (attempt + 1)));
            }
        }
    }
    Err(anyhow::anyhow!(
        "{}",
        last_error.map(|e| e.to_string()).unwrap_or_default()
    ))
}

fn sample_bilinear(data: &[f32], width: usize, height: usize, x: f64, y: f64) -> f32 {
    let x = x.clamp(0.0, (width - 1) as f64);
    let y = y.clamp(0.0, (height - 1) as f64);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let fx = (x - x0 as f64) as f32;
    let fy = (y - y0 as f64) as f32;
    let top = data[y0 * width + x0] + (data[y0 * width + x1] - data[y0 * width + x0]) * fx;
    let bottom = data[y1 * width + x0] + (data[y1 * width + x1] - data[y1 * width + x0]) * fx;
    top + (bottom - top) * fy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bilinear_interpolates_midpoint() {
        let data = [0.0f32, 10.0, 20.0, 30.0];
        let value = sample_bilinear(&data, 2, 2, 0.5, 0.5);
        assert!((value - 15.0).abs() < 1e-6);
    }

    #[test]
    fn bilinear_clamps_out_of_bounds() {
        let data = [1.0f32, 2.0, 3.0, 4.0];
        assert!((sample_bilinear(&data, 2, 2, -5.0, -5.0) - 1.0).abs() < 1e-6);
        assert!((sample_bilinear(&data, 2, 2, 99.0, 99.0) - 4.0).abs() < 1e-6);
    }

    #[test]
    fn zoom_selection_respects_tile_budget() {
        let bbox = Bbox::from_center(48.85, 2.29, 11264.0, 11264.0);
        let (zoom, range) = pick_zoom(&bbox, None);
        let count = ((range.2 - range.0 + 1) * (range.3 - range.1 + 1)) as usize;
        assert!(count <= MAX_TILES, "zoom {zoom} had {count} tiles");
    }
}
