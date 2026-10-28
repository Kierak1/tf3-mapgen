use std::f64::consts::PI;

pub const TILE_SIZE: f64 = 256.0;

#[derive(Clone, Copy, Debug)]
pub struct Bbox {
    pub south: f64,
    pub north: f64,
    pub west: f64,
    pub east: f64,
}

impl Bbox {
    pub fn from_center(lat: f64, lon: f64, width_m: f64, height_m: f64) -> Self {
        let lat_span = height_m / m_per_deg_lat(lat);
        let lon_span = width_m / m_per_deg_lon(lat);
        Bbox {
            south: lat - lat_span / 2.0,
            north: lat + lat_span / 2.0,
            west: lon - lon_span / 2.0,
            east: lon + lon_span / 2.0,
        }
    }

    pub fn lat_at_row(&self, row: f64, rows: f64) -> f64 {
        self.north - row / rows * (self.north - self.south)
    }

    pub fn lon_at_col(&self, col: f64, cols: f64) -> f64 {
        self.west + col / cols * (self.east - self.west)
    }
}

pub fn m_per_deg_lat(lat: f64) -> f64 {
    let p = lat.to_radians();
    111132.92 - 559.82 * (2.0 * p).cos() + 1.175 * (4.0 * p).cos() - 0.0023 * (6.0 * p).cos()
}

pub fn m_per_deg_lon(lat: f64) -> f64 {
    let p = lat.to_radians();
    111412.84 * p.cos() - 93.5 * (3.0 * p).cos() + 0.118 * (5.0 * p).cos()
}

pub fn lon_to_merc_px(lon: f64, zoom: u8) -> f64 {
    (lon + 180.0) / 360.0 * TILE_SIZE * 2f64.powi(zoom as i32)
}

pub fn lat_to_merc_px(lat: f64, zoom: u8) -> f64 {
    let lat = lat.clamp(-85.05112878, 85.05112878);
    let s = lat.to_radians().sin();
    (0.5 - ((1.0 + s) / (1.0 - s)).ln() / (4.0 * PI)) * TILE_SIZE * 2f64.powi(zoom as i32)
}

pub fn tile_range(bbox: &Bbox, zoom: u8) -> (i64, i64, i64, i64) {
    let max_index = (1i64 << zoom) - 1;
    let x0 = (lon_to_merc_px(bbox.west, zoom) / TILE_SIZE).floor() as i64;
    let x1 = (lon_to_merc_px(bbox.east, zoom) / TILE_SIZE).floor() as i64;
    let y0 = (lat_to_merc_px(bbox.north, zoom) / TILE_SIZE).floor() as i64;
    let y1 = (lat_to_merc_px(bbox.south, zoom) / TILE_SIZE).floor() as i64;
    (
        x0.clamp(0, max_index),
        y0.clamp(0, max_index),
        x1.clamp(0, max_index),
        y1.clamp(0, max_index),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_dimensions_are_close_to_request() {
        let bbox = Bbox::from_center(48.85, 2.29, 11264.0, 11264.0);
        let width = (bbox.east - bbox.west) * m_per_deg_lon(48.85);
        let height = (bbox.north - bbox.south) * m_per_deg_lat(48.85);
        assert!((width - 11264.0).abs() < 1.0, "width was {width}");
        assert!((height - 11264.0).abs() < 1.0, "height was {height}");
    }

    #[test]
    fn mercator_round_trip_origin() {
        let x = lon_to_merc_px(0.0, 10);
        assert!((x - TILE_SIZE * 512.0).abs() < 1e-6);
        let y = lat_to_merc_px(0.0, 10);
        assert!((y - TILE_SIZE * 512.0).abs() < 1e-6);
    }

    #[test]
    fn lat_lon_grid_maps_rows() {
        let bbox = Bbox {
            south: 48.0,
            north: 49.0,
            west: 2.0,
            east: 3.0,
        };
        assert!((bbox.lat_at_row(0.0, 10.0) - 49.0).abs() < 1e-9);
        assert!((bbox.lon_at_col(0.0, 10.0) - 2.0).abs() < 1e-9);
    }
}
