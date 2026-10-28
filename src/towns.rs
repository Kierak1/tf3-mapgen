use crate::geo::Bbox;
use crate::osm::Place;

pub struct Town {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub size: f64,
}

pub fn from_places(
    places: &[Place],
    bbox: &Bbox,
    width: usize,
    height: usize,
    map_width_m: f64,
    map_height_m: f64,
    max_towns: usize,
) -> Vec<Town> {
    let margin_px = 120.0 / 4.0;
    let min_distance_px = 300.0 / 4.0;
    let mut candidates: Vec<(f64, f64, f64, String)> = Vec::new();
    for place in places {
        let col = (place.lon - bbox.west) / (bbox.east - bbox.west) * width as f64;
        let row = (bbox.north - place.lat) / (bbox.north - bbox.south) * height as f64;
        if col < margin_px
            || col > width as f64 - margin_px
            || row < margin_px
            || row > height as f64 - margin_px
        {
            continue;
        }
        let population = place
            .population
            .unwrap_or_else(|| default_population(&place.kind));
        let score = population as f64;
        let x = col / (width as f64 - 1.0) * map_width_m - map_width_m / 2.0;
        let y = map_height_m / 2.0 - row / (height as f64 - 1.0) * map_height_m;
        let name = place.name.clone().unwrap_or_default();
        candidates.push((score, x, y, name));
    }
    candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut chosen: Vec<(f64, f64, f64, String)> = Vec::new();
    for (score, x, y, name) in candidates {
        let too_close = chosen.iter().any(|(_, other_x, other_y, _)| {
            let dx = (x - other_x) / 4.0;
            let dy = (y - other_y) / 4.0;
            (dx * dx + dy * dy).sqrt() < min_distance_px
        });
        if too_close {
            continue;
        }
        chosen.push((score, x, y, name));
        if chosen.len() >= max_towns {
            break;
        }
    }

    chosen
        .into_iter()
        .enumerate()
        .map(|(index, (score, x, y, name))| Town {
            name: if name.is_empty() {
                format!("Town {}", index + 1)
            } else {
                name
            },
            x,
            y,
            size: size_factor(score),
        })
        .collect()
}

fn default_population(kind: &str) -> u64 {
    match kind {
        "city" => 90_000,
        "town" => 12_000,
        "village" => 1_200,
        _ => 250,
    }
}

fn size_factor(population: f64) -> f64 {
    let population = population.max(1.0);
    (0.6 + 0.22 * population.log10()).clamp(0.8, 2.4)
}

pub fn to_lua(towns: &[Town]) -> String {
    let mut out = String::new();
    out.push_str("function data()\nreturn { \n\tindustries = { \n\t},\n\ttowns = { \n");
    for town in towns {
        out.push_str(&format!(
            "\t\t{{ landUse2CargoNeedsCategories = {{ }}, name = \"{}\", position = {{ x = {:.2}, y = {:.2} }}, sizeFactors = {{ {:.2}, {:.2}, {:.2} }}, }},\n",
            escape_lua(&town.name),
            town.x,
            town.y,
            town.size,
            town.size,
            town.size
        ));
    }
    out.push_str("\t}\n}\nend\n");
    out
}

fn escape_lua(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bbox(width_m: f64) -> Bbox {
        Bbox::from_center(48.0, 2.0, width_m, width_m)
    }

    #[test]
    fn picks_largest_and_respects_cap() {
        let bbox = bbox(4096.0);
        let places = vec![
            Place {
                lat: 48.0,
                lon: 2.0,
                name: Some("Big".into()),
                kind: "town".into(),
                population: Some(50_000),
            },
            Place {
                lat: 48.01,
                lon: 2.01,
                name: Some("Small".into()),
                kind: "village".into(),
                population: Some(500),
            },
        ];
        let towns = from_places(&places, &bbox, 1025, 1025, 4096.0, 4096.0, 1);
        assert_eq!(towns.len(), 1);
        assert_eq!(towns[0].name, "Big");
        assert!(towns[0].size > 1.0);
    }

    #[test]
    fn lua_is_valid_and_escaped() {
        let towns = vec![Town {
            name: "A \"quoted\" town".into(),
            x: -12.345,
            y: 678.9,
            size: 1.25,
        }];
        let lua = to_lua(&towns);
        assert!(lua.contains("name = \"A \\\"quoted\\\" town\""));
        assert!(lua.contains("x = -12.35"));
        assert!(lua.starts_with("function data()"));
        assert!(lua.ends_with("end\n"));
    }
}
