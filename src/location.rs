use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

pub const USER_AGENT: &str = concat!("tf3-mapgen/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
    pub label: Option<String>,
}

pub fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(120))
        .build()
        .context("building HTTP client")
}

pub fn resolve(input: &str) -> Result<Location> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("empty location");
    }
    if let Some(location) = parse_lat_lon(trimmed) {
        return Ok(location);
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let client = client()?;
        let url = if is_short_link(trimmed) {
            follow(&client, trimmed)?
        } else {
            trimmed.to_string()
        };
        if let Some(location) = parse_maps_url(&url) {
            return Ok(location);
        }
        if let Some(label) = place_label(&url) {
            return geocode(&client, &label);
        }
        bail!("could not find coordinates in URL: {url}");
    }
    geocode(&client()?, trimmed)
}

fn is_short_link(url: &str) -> bool {
    url.contains("maps.app.goo.gl") || url.contains("goo.gl/maps")
}

fn follow(client: &reqwest::blocking::Client, url: &str) -> Result<String> {
    let response = client.get(url).send().context("following shortened URL")?;
    Ok(response.url().to_string())
}

fn parse_lat_lon(text: &str) -> Option<Location> {
    let cleaned = text.replace(',', " ");
    let parts: Vec<&str> = cleaned.split_whitespace().collect();
    if parts.len() != 2 {
        return None;
    }
    let lat: f64 = parts[0].parse().ok()?;
    let lon: f64 = parts[1].parse().ok()?;
    if lat.abs() > 90.0 || lon.abs() > 180.0 {
        return None;
    }
    Some(Location {
        lat,
        lon,
        label: None,
    })
}

fn parse_maps_url(url: &str) -> Option<Location> {
    if let Some(index) = url.find("!3d") {
        if let Some(lat) = number_after(url, index + 3) {
            if let Some(index4) = url.find("!4d") {
                if let Some(lon) = number_after(url, index4 + 3) {
                    if lat.abs() <= 90.0 && lon.abs() <= 180.0 {
                        return Some(Location {
                            lat,
                            lon,
                            label: place_label(url),
                        });
                    }
                }
            }
        }
    }
    if let Some(index) = url.find("/@") {
        let rest = &url[index + 2..];
        let mut parts = rest.split(['/', '?', ',']);
        if let (Some(lat), Some(lon)) = (parts.next(), parts.next()) {
            if let (Ok(lat), Ok(lon)) = (lat.parse::<f64>(), lon.parse::<f64>()) {
                if lat.abs() <= 90.0 && lon.abs() <= 180.0 {
                    return Some(Location {
                        lat,
                        lon,
                        label: place_label(url),
                    });
                }
            }
        }
    }
    if let Ok(parsed) = url::Url::parse(url) {
        for key in ["q", "query", "ll", "center", "daddr"] {
            if let Some(value) = parsed
                .query_pairs()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.to_string())
            {
                if let Some(location) = parse_lat_lon(&value) {
                    return Some(location);
                }
            }
        }
        for segment in parsed.path_segments()? {
            if let Some(location) = parse_lat_lon(segment) {
                return Some(location);
            }
        }
    }
    None
}

fn number_after(text: &str, start: usize) -> Option<f64> {
    text[start..]
        .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .next()
        .filter(|part| !part.is_empty())
        .and_then(|part| part.parse().ok())
}

fn place_label(url: &str) -> Option<String> {
    let index = url.find("/maps/place/")? + "/maps/place/".len();
    let rest = &url[index..];
    let end = rest.find('/').unwrap_or(rest.len());
    let label = percent_decode(&rest[..end]);
    if label.is_empty() {
        None
    } else {
        Some(label)
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                    out.push(high * 16 + low);
                    index += 3;
                } else {
                    out.push(b'%');
                    index += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Deserialize)]
struct NominatimItem {
    lat: String,
    lon: String,
    display_name: String,
}

fn geocode(client: &reqwest::blocking::Client, query: &str) -> Result<Location> {
    let response = client
        .get("https://nominatim.openstreetmap.org/search")
        .query(&[("q", query), ("format", "json"), ("limit", "1")])
        .send()
        .context("geocoding request failed")?;
    if !response.status().is_success() {
        bail!("geocoding failed with HTTP {}", response.status());
    }
    let items: Vec<NominatimItem> = response.json().context("parsing geocoding response")?;
    let item = items
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no geocoding result for '{query}'"))?;
    Ok(Location {
        lat: item.lat.parse()?,
        lon: item.lon.parse()?,
        label: Some(item.display_name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_coordinates() {
        let location = parse_lat_lon("48.8584, 2.2945").unwrap();
        assert!((location.lat - 48.8584).abs() < 1e-9);
        assert!((location.lon - 2.2945).abs() < 1e-9);
    }

    #[test]
    fn rejects_out_of_range_coordinates() {
        assert!(parse_lat_lon("91.0, 2.0").is_none());
        assert!(parse_lat_lon("not a location").is_none());
    }

    #[test]
    fn parses_google_maps_at_url() {
        let location = parse_maps_url("https://www.google.com/maps/@48.8584,2.2945,17z").unwrap();
        assert!((location.lat - 48.8584).abs() < 1e-9);
        assert!((location.lon - 2.2945).abs() < 1e-9);
    }

    #[test]
    fn parses_google_maps_place_url() {
        let location = parse_maps_url(
            "https://www.google.com/maps/place/Eiffel+Tower/@48.858,2.29,17z/data=!3d48.85837!4d2.294481",
        )
        .unwrap();
        assert!((location.lat - 48.85837).abs() < 1e-9);
        assert!((location.lon - 2.294481).abs() < 1e-9);
        assert_eq!(location.label.as_deref(), Some("Eiffel Tower"));
    }

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(percent_decode("S%C3%A3o+Paulo"), "São Paulo");
    }
}
