use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::geo::Bbox;
use crate::location::USER_AGENT;
use crate::progress::Progress;

const ENDPOINTS: [&str; 4] = [
    "https://overpass-api.de/api/interpreter",
    "https://overpass.private.coffee/api/interpreter",
    "https://maps.mail.ru/osm/tools/overpass/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
];

#[derive(Debug, Default)]
pub struct OsmData {
    pub water: Vec<Vec<[f64; 2]>>,
    pub rivers: Vec<(Vec<[f64; 2]>, f64)>,
    pub forests: Vec<Vec<[f64; 2]>>,
    pub places: Vec<Place>,
}

#[derive(Debug, Clone)]
pub struct Place {
    pub lat: f64,
    pub lon: f64,
    pub name: Option<String>,
    pub kind: String,
    pub population: Option<u64>,
}

pub fn fetch(
    bbox: &Bbox,
    endpoint_override: Option<&str>,
    progress: &dyn Progress,
) -> Result<OsmData> {
    let area = format!(
        "{:.5},{:.5},{:.5},{:.5}",
        bbox.south, bbox.west, bbox.north, bbox.east
    );
    let query_text = format!(
        "[out:json][timeout:180];(\
         way[\"natural\"=\"water\"]({area});\
         way[\"waterway\"=\"riverbank\"]({area});\
         way[\"landuse\"=\"reservoir\"]({area});\
         way[\"landuse\"=\"forest\"]({area});\
         way[\"natural\"=\"wood\"]({area});\
         way[\"waterway\"~\"^(river|canal|stream)$\"]({area});\
         node[\"place\"~\"^(city|town|village|hamlet)$\"]({area});\
         relation[\"natural\"=\"water\"]({area});\
         relation[\"waterway\"=\"riverbank\"]({area});\
         relation[\"landuse\"=\"reservoir\"]({area});\
         );out geom;"
    );

    let mut data = OsmData::default();
    for element in query(&query_text, endpoint_override, progress)? {
        match element.kind.as_str() {
            "way" => parse_way(element, &mut data),
            "node" => parse_place(element, &mut data),
            "relation" => parse_water_relation(element, &mut data),
            _ => {}
        }
    }
    Ok(data)
}

fn query(
    body: &str,
    endpoint_override: Option<&str>,
    progress: &dyn Progress,
) -> Result<Vec<OverpassElement>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(300))
        .build()?;
    let endpoints: Vec<&str> = match endpoint_override {
        Some(url) => vec![url],
        None => ENDPOINTS.to_vec(),
    };
    let mut last_error = None;
    for endpoint in endpoints {
        for attempt in 0..2 {
            progress.info(&format!(
                "  querying {endpoint}{} (this can take up to a minute) ...",
                if attempt > 0 { " (retry)" } else { "" }
            ));
            let started = std::time::Instant::now();
            match client.post(endpoint).form(&[("data", body)]).send() {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        match response.json::<OverpassResponse>() {
                            Ok(parsed) => {
                                progress.info(&format!(
                                    "  Overpass replied in {:.1}s",
                                    started.elapsed().as_secs_f64()
                                ));
                                return Ok(parsed.elements);
                            }
                            Err(error) => {
                                last_error = Some(anyhow!("invalid JSON from {endpoint}: {error}"));
                                break;
                            }
                        }
                    }
                    last_error = Some(anyhow!("{endpoint} returned HTTP {status}"));
                    if matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504) {
                        std::thread::sleep(Duration::from_secs(5));
                        continue;
                    }
                    break;
                }
                Err(error) => {
                    last_error = Some(anyhow!("request to {endpoint} failed: {error}"));
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        }
        if let Some(error) = &last_error {
            progress.warn(&format!("{error}"));
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("no OpenStreetMap endpoint available")))
}

fn parse_way(element: OverpassElement, data: &mut OsmData) {
    let Some(tags) = element.tags else { return };
    let Some(geometry) = element.geometry else {
        return;
    };
    if geometry.len() < 2 {
        return;
    }
    let ring: Vec<[f64; 2]> = geometry
        .iter()
        .map(|point| [point.lat, point.lon])
        .collect();
    let natural = tags.get("natural").map(String::as_str);
    let landuse = tags.get("landuse").map(String::as_str);
    let waterway = tags.get("waterway").map(String::as_str);
    if natural == Some("water") || waterway == Some("riverbank") || landuse == Some("reservoir") {
        data.water.push(ring);
    } else if landuse == Some("forest") || natural == Some("wood") {
        data.forests.push(ring);
    } else if let Some(waterway) = waterway {
        if is_route_or_underground(&tags) || max_segment_m(&ring) > 1500.0 {
            return;
        }
        let width = tags
            .get("width")
            .and_then(|width| width.trim_end_matches('m').trim().parse::<f64>().ok())
            .unwrap_or(match waterway {
                "river" => 50.0,
                "canal" => 30.0,
                _ => 8.0,
            });
        data.rivers.push((ring, width.clamp(6.0, 400.0)));
    }
}

fn parse_place(element: OverpassElement, data: &mut OsmData) {
    let (Some(lat), Some(lon)) = (element.lat, element.lon) else {
        return;
    };
    let tags = element.tags.unwrap_or_default();
    let Some(kind) = tags.get("place") else {
        return;
    };
    data.places.push(Place {
        lat,
        lon,
        name: tags.get("name").cloned(),
        kind: kind.clone(),
        population: tags
            .get("population")
            .and_then(|value| value.replace(',', "").trim().parse().ok()),
    });
}

fn parse_water_relation(element: OverpassElement, data: &mut OsmData) {
    let Some(members) = element.members else {
        return;
    };
    let mut parts: Vec<Vec<[f64; 2]>> = Vec::new();
    for member in members {
        if member.role.as_deref().unwrap_or("outer") != "outer" {
            continue;
        }
        let Some(geometry) = member.geometry else {
            continue;
        };
        if geometry.len() >= 2 {
            parts.push(
                geometry
                    .iter()
                    .map(|point| [point.lat, point.lon])
                    .collect(),
            );
        }
    }
    for ring in stitch_rings(parts) {
        if ring.len() >= 4 {
            data.water.push(ring);
        }
    }
}

fn is_route_or_underground(tags: &HashMap<String, String>) -> bool {
    for key in ["boat", "motorboat", "passenger", "ferry", "route"] {
        if tags.contains_key(key) {
            return true;
        }
    }
    if let Some(tunnel) = tags.get("tunnel") {
        if tunnel != "no" {
            return true;
        }
    }
    matches!(tags.get("intermittent").map(String::as_str), Some("yes"))
}

fn max_segment_m(points: &[[f64; 2]]) -> f64 {
    let lat = points[points.len() / 2][0];
    let metres_per_deg_lat = crate::geo::m_per_deg_lat(lat);
    let metres_per_deg_lon = crate::geo::m_per_deg_lon(lat);
    points
        .windows(2)
        .map(|pair| {
            let dy = (pair[1][0] - pair[0][0]) * metres_per_deg_lat;
            let dx = (pair[1][1] - pair[0][1]) * metres_per_deg_lon;
            (dx * dx + dy * dy).sqrt()
        })
        .fold(0.0, f64::max)
}

fn stitch_rings(parts: Vec<Vec<[f64; 2]>>) -> Vec<Vec<[f64; 2]>> {
    const EPSILON: f64 = 1e-7;
    let close =
        |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < EPSILON && (a[1] - b[1]).abs() < EPSILON;
    let mut remaining: Vec<Vec<[f64; 2]>> =
        parts.into_iter().filter(|part| part.len() >= 2).collect();
    let mut rings = Vec::new();
    while !remaining.is_empty() {
        let mut ring = remaining.remove(0);
        loop {
            let start = ring[0];
            let end = *ring.last().unwrap();
            if close(start, end) {
                break;
            }
            let mut found = None;
            for (index, part) in remaining.iter().enumerate() {
                if close(part[0], end) {
                    found = Some((index, false));
                    break;
                }
                if close(*part.last().unwrap(), end) {
                    found = Some((index, true));
                    break;
                }
            }
            match found {
                Some((index, reverse)) => {
                    let mut part = remaining.remove(index);
                    if reverse {
                        part.reverse();
                    }
                    ring.extend(part.into_iter().skip(1));
                }
                None => break,
            }
        }
        rings.push(ring);
    }
    rings
}

#[derive(Deserialize)]
struct OverpassResponse {
    elements: Vec<OverpassElement>,
}
#[derive(Deserialize)]
struct OverpassElement {
    #[serde(rename = "type")]
    kind: String,
    lat: Option<f64>,
    lon: Option<f64>,
    tags: Option<HashMap<String, String>>,
    geometry: Option<Vec<GeoPoint>>,
    members: Option<Vec<Member>>,
}

#[derive(Deserialize)]
struct GeoPoint {
    lat: f64,
    lon: f64,
}

#[derive(Deserialize)]
struct Member {
    role: Option<String>,
    geometry: Option<Vec<GeoPoint>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn way(tags: &[(&str, &str)], points: &[(f64, f64)]) -> OverpassElement {
        OverpassElement {
            kind: "way".into(),
            lat: None,
            lon: None,
            tags: Some(
                tags.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ),
            geometry: Some(
                points
                    .iter()
                    .map(|(lat, lon)| GeoPoint {
                        lat: *lat,
                        lon: *lon,
                    })
                    .collect(),
            ),
            members: None,
        }
    }

    #[test]
    fn classifies_water_forest_and_rivers() {
        let mut data = OsmData::default();
        for element in [
            way(
                &[("natural", "water")],
                &[(1.0, 1.0), (1.0, 2.0), (2.0, 2.0)],
            ),
            way(
                &[("landuse", "forest")],
                &[(1.0, 1.0), (1.0, 2.0), (2.0, 2.0)],
            ),
            way(
                &[("waterway", "river")],
                &[(1.0, 1.0), (1.0005, 1.0005), (1.001, 1.001)],
            ),
        ] {
            parse_way(element, &mut data);
        }
        assert_eq!(data.water.len(), 1);
        assert_eq!(data.forests.len(), 1);
        assert_eq!(data.rivers.len(), 1);
        assert!((data.rivers[0].1 - 50.0).abs() < 1e-9);
    }

    #[test]
    fn honours_explicit_width() {
        let mut data = OsmData::default();
        parse_way(
            way(
                &[("waterway", "river"), ("width", "150 m")],
                &[(1.0, 1.0), (1.0005, 1.0005)],
            ),
            &mut data,
        );
        assert!((data.rivers[0].1 - 150.0).abs() < 1e-9);
    }

    #[test]
    fn skips_boat_routes_and_culverts() {
        let mut data = OsmData::default();
        for element in [
            way(
                &[("waterway", "river"), ("boat", "yes")],
                &[(1.0, 1.0), (1.0005, 1.0005)],
            ),
            way(
                &[("waterway", "stream"), ("tunnel", "culvert")],
                &[(1.0, 1.0), (1.0005, 1.0005)],
            ),
            way(&[("waterway", "river")], &[(1.0, 1.0), (1.02, 1.02)]),
        ] {
            parse_way(element, &mut data);
        }
        assert!(data.rivers.is_empty());
    }

    #[test]
    fn stitches_open_ways_into_rings() {
        let parts = vec![
            vec![[0.0, 0.0], [0.0, 1.0]],
            vec![[0.0, 1.0], [1.0, 1.0]],
            vec![[1.0, 0.0], [0.0, 0.0]],
            vec![[1.0, 1.0], [1.0, 0.0]],
        ];
        let rings = stitch_rings(parts);
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].len(), 5);
        assert_eq!(rings[0][0], rings[0][rings[0].len() - 1]);
    }

    #[test]
    fn reverses_way_direction_while_stitching() {
        let parts = vec![
            vec![[0.0, 0.0], [0.0, 1.0]],
            vec![[1.0, 1.0], [0.0, 1.0]],
            vec![[1.0, 1.0], [1.0, 0.0]],
            vec![[0.0, 0.0], [1.0, 0.0]],
        ];
        let rings = stitch_rings(parts);
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].len(), 5);
    }
}
