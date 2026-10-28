use crate::geo::Bbox;

const METRES_PER_PIXEL: f64 = 4.0;

pub fn rasterize_polygons(
    width: usize,
    height: usize,
    bbox: &Bbox,
    rings: &[Vec<[f64; 2]>],
    mask: &mut [bool],
) {
    for ring in rings {
        fill_ring(width, height, bbox, ring, mask);
    }
}

pub fn rasterize_lines(
    width: usize,
    height: usize,
    bbox: &Bbox,
    lines: &[(Vec<[f64; 2]>, f64)],
    mask: &mut [bool],
) {
    for (points, width_m) in lines {
        let radius = (width_m / METRES_PER_PIXEL / 2.0).max(0.5);
        for segment in points.windows(2) {
            let start = to_pixel(bbox, segment[0][0], segment[0][1], width, height);
            let end = to_pixel(bbox, segment[1][0], segment[1][1], width, height);
            stamp_segment(start, end, radius, width, height, mask);
        }
    }
}

fn to_pixel(bbox: &Bbox, lat: f64, lon: f64, width: usize, height: usize) -> (f64, f64) {
    let x = (lon - bbox.west) / (bbox.east - bbox.west) * width as f64;
    let y = (bbox.north - lat) / (bbox.north - bbox.south) * height as f64;
    (x, y)
}

fn fill_ring(width: usize, height: usize, bbox: &Bbox, ring: &[[f64; 2]], mask: &mut [bool]) {
    if ring.len() < 3 {
        return;
    }
    let points: Vec<(f64, f64)> = ring
        .iter()
        .map(|point| to_pixel(bbox, point[0], point[1], width, height))
        .collect();
    let mut edges: Vec<((f64, f64), (f64, f64))> = Vec::new();
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        if (a.1 - b.1).abs() > f64::EPSILON {
            edges.push((a, b));
        }
    }
    if edges.is_empty() {
        return;
    }
    let mut intersections: Vec<f64> = Vec::new();
    for row in 0..height {
        let scan_y = row as f64 + 0.5;
        intersections.clear();
        for (a, b) in &edges {
            let (low, high) = if a.1 < b.1 { (a, b) } else { (b, a) };
            if scan_y >= low.1 && scan_y < high.1 {
                let t = (scan_y - low.1) / (high.1 - low.1);
                intersections.push(low.0 + (high.0 - low.0) * t);
            }
        }
        if intersections.len() < 2 {
            continue;
        }
        intersections.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut index = 0;
        while index + 1 < intersections.len() {
            let start = ((intersections[index] - 0.5).ceil() as i64).max(0);
            let end = ((intersections[index + 1] - 0.5).floor() as i64).min(width as i64 - 1);
            for col in start..=end {
                mask[row * width + col as usize] = true;
            }
            index += 2;
        }
    }
}

fn stamp_segment(
    start: (f64, f64),
    end: (f64, f64),
    radius: f64,
    width: usize,
    height: usize,
    mask: &mut [bool],
) {
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let length = (dx * dx + dy * dy).sqrt();
    let steps = (length / 0.5).ceil().max(1.0) as usize;
    for step in 0..=steps {
        let t = step as f64 / steps as f64;
        stamp_circle(
            start.0 + dx * t,
            start.1 + dy * t,
            radius,
            width,
            height,
            mask,
        );
    }
}

fn stamp_circle(cx: f64, cy: f64, radius: f64, width: usize, height: usize, mask: &mut [bool]) {
    let min_x = (cx - radius - 0.5).floor().max(0.0) as usize;
    let max_x = ((cx + radius - 0.5).ceil() as i64).clamp(0, width as i64 - 1) as usize;
    let min_y = (cy - radius - 0.5).floor().max(0.0) as usize;
    let max_y = ((cy + radius - 0.5).ceil() as i64).clamp(0, height as i64 - 1) as usize;
    let radius_sq = radius * radius;
    for row in min_y..=max_y {
        let dy = row as f64 + 0.5 - cy;
        let row_offset = row * width;
        for col in min_x..=max_x {
            let dx = col as f64 + 0.5 - cx;
            if dx * dx + dy * dy <= radius_sq {
                mask[row_offset + col] = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bbox() -> Bbox {
        Bbox {
            south: 0.0,
            north: 1.0,
            west: 0.0,
            east: 1.0,
        }
    }

    #[test]
    fn polygon_fill_sets_inside_cells() {
        let bbox = bbox();
        let ring = vec![[0.25, 0.25], [0.25, 0.75], [0.75, 0.75], [0.75, 0.25]];
        let mut mask = vec![false; 100 * 100];
        rasterize_polygons(100, 100, &bbox, &[ring], &mut mask);
        assert!(mask[50 * 100 + 50]);
        assert!(!mask[5 * 100 + 5]);
        assert!(!mask[95 * 100 + 95]);
    }

    #[test]
    fn line_stamping_marks_pixels() {
        let bbox = bbox();
        let line = (vec![[0.1, 0.1], [0.1, 0.9]], 40.0);
        let mut mask = vec![false; 100 * 100];
        rasterize_lines(100, 100, &bbox, &[line], &mut mask);
        assert!(mask.iter().any(|value| *value));
        assert!(mask[90 * 100 + 10]);
        assert!(!mask[10 * 100 + 10]);
    }
}
