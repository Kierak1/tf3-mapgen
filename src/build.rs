use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};

use crate::elevation;
use crate::location;
use crate::mapsize;
use crate::osm::{self, OsmData};
use crate::progress::Progress;
use crate::raster;
use crate::render;
use crate::towns;
use crate::userdata;
use crate::GenerateOptions;

const METRES_PER_PIXEL: f64 = 4.0;
const WATER_EPSILON: f32 = 0.5;
const INLAND_DEPTH: f64 = 5.0;
const RIVER_DEPTH: f64 = 3.0;
const MAX_AUTO_HEIGHT: f64 = 3500.0;
const WATER_TOLERANCE: f64 = 80.0;
const SEAM_THRESHOLD: f32 = 25.0;
const SEAM_RADIUS: usize = 6;
const SEAM_STRAIGHT_MIN_DEVIATION: f32 = 10.0;
const SEAM_STRAIGHT_MIN_FRACTION: f32 = 0.6;
const MOUNTAIN_DEADZONE: f32 = 0.10;
const MOUNTAIN_SLOPE_SCALE: f32 = 0.7;
const BIOME_VALUES: [u16; 5] = [6554, 19661, 32768, 45875, 58982];

#[derive(Debug, Clone)]
pub struct GenerateSummary {
    pub map_name: String,
    pub size: String,
    pub ratio: String,
    pub width: u32,
    pub height: u32,
    pub width_m: f64,
    pub height_m: f64,
    pub min_h: f64,
    pub max_h: f64,
    pub water_level: f64,
    pub location_lat: f64,
    pub location_lon: f64,
    pub location_label: Option<String>,
    pub output_root: PathBuf,
    pub heightmap: PathBuf,
    pub biome_mask: PathBuf,
    pub rivers_mask: PathBuf,
    pub mountains_mask: PathBuf,
    pub towns: PathBuf,
    pub preview: PathBuf,
    pub settings: PathBuf,
    pub metadata: PathBuf,
    pub experimental_size: bool,
    pub clipped_low: usize,
    pub clipped_high: usize,
    pub town_count: usize,
    pub repaired_pixels: usize,
}

pub fn generate(options: &GenerateOptions, progress: &dyn Progress) -> Result<GenerateSummary> {
    let options = normalized(options);
    let location = resolve_location(&options)?;
    let size_name = mapsize::normalize_size(&options.size).ok_or_else(|| {
        anyhow!(
            "unknown map size '{}' (available: {})",
            options.size,
            mapsize::list_sizes()
        )
    })?;
    let ratio = mapsize::normalize_ratio(&options.ratio).ok_or_else(|| {
        anyhow!(
            "unknown map ratio '{}' (use one of 1:1, 1:2, 1:3, 1:4, 1:5)",
            options.ratio
        )
    })?;
    let (w, h) = mapsize::pixels(size_name, ratio).expect("validated size and ratio");
    let width = w as usize;
    let height = h as usize;
    let width_m = (w as f64 - 1.0) * METRES_PER_PIXEL;
    let height_m = (h as f64 - 1.0) * METRES_PER_PIXEL;

    let default_name = location
        .label
        .as_ref()
        .and_then(|label| label.split(',').next())
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .unwrap_or_else(|| "TF3 Map".to_string());
    let map_name = sanitize_name(options.name.as_deref().unwrap_or(&default_name));

    progress.info(&format!("tf3-mapgen {}", env!("CARGO_PKG_VERSION")));
    progress.info(&format!(
        "  location : {:.6}, {:.6}{}",
        location.lat,
        location.lon,
        location
            .label
            .as_ref()
            .map(|label| format!("  ({label})"))
            .unwrap_or_default()
    ));
    progress.info(&format!(
        "  map      : {} {}  ->  {}x{} px ({} x {} m at 4 m/px)",
        mapsize::display_size(size_name),
        ratio,
        w,
        h,
        width_m,
        height_m
    ));
    progress.info(&format!("  name     : {map_name}"));

    progress.info("Fetching elevation data ...");
    let mut dem = elevation::fetch_dem(
        location.lat,
        location.lon,
        width_m,
        height_m,
        width,
        height,
        options.zoom,
        progress,
    )?;
    let mut repaired_pixels = 0;
    if options.repair_seams {
        repaired_pixels = repair_seams(&mut dem.elev, width, height, SEAM_THRESHOLD, SEAM_RADIUS);
        if repaired_pixels > 0 {
            progress.warn(&format!(
                "repaired {repaired_pixels} elevation pixels that were narrow artifacts (data seams/outliers)"
            ));
        }
    }
    progress.info(&format!(
        "  elevation: {:.0} m .. {:.0} m (zoom {})",
        min_of(&dem.elev),
        max_of(&dem.elev),
        dem.zoom
    ));

    let osm_data = if options.skip_osm {
        progress.info("Skipping OpenStreetMap (no water bodies, forests or towns)");
        OsmData::default()
    } else {
        progress.info("Fetching OpenStreetMap data (water, rivers, forests, towns) ...");
        let data = osm::fetch(&dem.bbox, options.osm_endpoint.as_deref(), progress)
            .context("OpenStreetMap query failed")?;
        progress.info(&format!(
            "  features : {} water areas, {} rivers, {} forests, {} places",
            data.water.len(),
            data.rivers.len(),
            data.forests.len(),
            data.places.len()
        ));
        data
    };

    let sea = detect_sea(&dem.elev, width, height);
    let mut osm_water = vec![false; width * height];
    raster::rasterize_polygons(width, height, &dem.bbox, &osm_data.water, &mut osm_water);
    let mut river_mask = vec![false; width * height];
    raster::rasterize_lines(width, height, &dem.bbox, &osm_data.rivers, &mut river_mask);
    let mut forest = vec![false; width * height];
    raster::rasterize_polygons(width, height, &dem.bbox, &osm_data.forests, &mut forest);

    let mut water = vec![false; width * height];
    for (index, is_water) in water.iter_mut().enumerate() {
        *is_water = sea[index] || osm_water[index] || river_mask[index];
    }

    let terrain = dem.elev.clone();
    let terrain_smooth = smooth3x3(&smooth3x3(&terrain, width, height), width, height);
    let water_level = match options.water_level {
        Some(level) => level,
        None => estimate_water_level(&sea, &terrain, &osm_water, width, height),
    };
    progress.info(&format!(
        "  water    : level {} m ({})",
        water_level,
        if options.water_level.is_some() {
            "from --water-level"
        } else if water_level.abs() < 1.0 {
            "sea level"
        } else {
            "auto-detected; override with --water-level"
        }
    ));

    let mut carved = vec![false; width * height];
    let mut elev = dem.elev.clone();
    for index in 0..elev.len() {
        if !water[index] || (terrain[index] as f64 - water_level).abs() > WATER_TOLERANCE {
            continue;
        }
        carved[index] = true;
        if sea[index] {
            elev[index] = (water_level - options.sea_depth) as f32;
        } else if osm_water[index] {
            elev[index] = (water_level - INLAND_DEPTH) as f32;
        } else {
            elev[index] = (water_level - RIVER_DEPTH) as f32;
        }
    }

    let (min_h, max_h, clipped_low, clipped_high) = choose_range(&options, &elev);
    let span = max_h - min_h;
    let mut heightmap = vec![0u16; elev.len()];
    for (index, value) in elev.iter().enumerate() {
        let normalized = ((*value as f64 - min_h) / span).clamp(0.0, 1.0);
        heightmap[index] = (normalized * 65535.0).round() as u16;
    }

    let mut biome = vec![0u16; width * height];
    let mut rivers_mask = vec![0u8; width * height];
    let mut mountains = vec![0u8; width * height];
    for row in 0..height {
        for col in 0..width {
            let index = row * width + col;
            let slope = slope_at(&terrain_smooth, width, height, col, row);
            let value = terrain[index];
            let biome_index = if carved[index] {
                0
            } else if value > 1600.0 || slope > 0.55 {
                4
            } else if value > 700.0 || slope > 0.25 {
                3
            } else if forest[index] {
                1
            } else {
                2
            };
            biome[index] = BIOME_VALUES[biome_index];
            if carved[index] && !sea[index] {
                rivers_mask[index] = 255;
            }
            if carved[index] {
                mountains[index] = 0;
            } else {
                let elevation_norm = ((value - 250.0) / 1000.0).clamp(0.0, 1.0);
                let slope_norm = (slope / MOUNTAIN_SLOPE_SCALE).clamp(0.0, 1.0);
                let mountain = elevation_norm.max(slope_norm);
                mountains[index] = if mountain < MOUNTAIN_DEADZONE {
                    0
                } else {
                    (mountain * 255.0).round() as u8
                };
            }
        }
    }

    let max_towns = options.max_towns.unwrap_or_else(|| {
        let area_km2 = width_m * height_m / 1_000_000.0;
        (area_km2 / 10.0).round().clamp(4.0, 40.0) as usize
    });
    let mut town_list = towns::from_places(
        &osm_data.places,
        &dem.bbox,
        width,
        height,
        width_m,
        height_m,
        max_towns,
    );
    progress.info(&format!(
        "  towns    : {} placed (limit {max_towns})",
        town_list.len()
    ));

    if options.flip_x {
        flip_horizontal(&mut heightmap, width, height);
        flip_horizontal(&mut biome, width, height);
        flip_horizontal(&mut rivers_mask, width, height);
        flip_horizontal(&mut mountains, width, height);
        flip_horizontal(&mut carved, width, height);
        for town in &mut town_list {
            town.x = -town.x;
        }
    }
    if options.flip_y {
        flip_vertical(&mut heightmap, width, height);
        flip_vertical(&mut biome, width, height);
        flip_vertical(&mut rivers_mask, width, height);
        flip_vertical(&mut mountains, width, height);
        flip_vertical(&mut carved, width, height);
        for town in &mut town_list {
            town.y = -town.y;
        }
    }

    let towns_px: Vec<(usize, usize)> = town_list
        .iter()
        .map(|town| {
            let col = ((town.x + width_m / 2.0) / width_m * (width as f64 - 1.0))
                .round()
                .clamp(0.0, width as f64 - 1.0) as usize;
            let row = ((height_m / 2.0 - town.y) / height_m * (height as f64 - 1.0))
                .round()
                .clamp(0.0, height as f64 - 1.0) as usize;
            (col, row)
        })
        .collect();

    let root = resolve_output_root(&options)?;
    let heightmap_dir = root.join("heightmaps");
    let biomes_dir = root.join("biomes");
    let towns_dir = root.join("towns_industries");
    let detail_dir = root.join("tf3_mapgen").join(&map_name);
    for dir in [&heightmap_dir, &biomes_dir, &towns_dir, &detail_dir] {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    let heightmap_path = heightmap_dir.join(format!("{map_name}.png"));
    let biome_path = biomes_dir.join(format!("{map_name}_biomes.png"));
    let rivers_path = biomes_dir.join(format!("{map_name}_rivers.png"));
    let mountains_path = biomes_dir.join(format!("{map_name}_mountains.png"));
    let towns_path = towns_dir.join(format!("{map_name}.lua"));
    let preview_path = detail_dir.join("preview.png");
    let settings_path = detail_dir.join("settings.txt");
    let metadata_path = detail_dir.join("metadata.json");

    render::write_luma16(&heightmap_path, &heightmap, w, h)?;
    render::write_luma16(&biome_path, &biome, w, h)?;
    render::write_luma8(&rivers_path, &rivers_mask, w, h)?;
    render::write_luma8(&mountains_path, &mountains, w, h)?;
    render::write_text(&towns_path, &towns::to_lua(&town_list))?;
    render::write_preview(
        &preview_path,
        &elev,
        &carved,
        &towns_px,
        width,
        height,
        min_h,
        max_h,
    )?;

    let settings = settings_text(
        &map_name,
        &location,
        size_name,
        ratio,
        w,
        h,
        min_h,
        max_h,
        water_level,
        options.skip_osm,
        clipped_low,
        clipped_high,
        repaired_pixels,
    );
    render::write_text(&settings_path, &settings)?;
    let metadata = serde_json::json!({
        "generator": format!("tf3-mapgen {}", env!("CARGO_PKG_VERSION")),
        "location": {
            "lat": location.lat,
            "lon": location.lon,
            "label": location.label,
        },
        "bbox": {
            "south": dem.bbox.south,
            "north": dem.bbox.north,
            "west": dem.bbox.west,
            "east": dem.bbox.east,
        },
        "elevation_zoom": dem.zoom,
        "map": {
            "name": map_name,
            "size": size_name,
            "ratio": ratio,
            "width_px": w,
            "height_px": h,
            "width_m": width_m,
            "height_m": height_m,
        },
        "height": {
            "min": min_h,
            "max": max_h,
            "water_level": water_level,
        },
        "flipped_x": options.flip_x,
        "flipped_y": options.flip_y,
        "sources": ["AWS Terrain Tiles (Terrarium)", "OpenStreetMap (Overpass API)"],
    });
    render::write_text(&metadata_path, &serde_json::to_string_pretty(&metadata)?)?;

    Ok(GenerateSummary {
        map_name,
        size: size_name.to_string(),
        ratio: ratio.to_string(),
        width: w,
        height: h,
        width_m,
        height_m,
        min_h,
        max_h,
        water_level,
        location_lat: location.lat,
        location_lon: location.lon,
        location_label: location.label.clone(),
        output_root: root,
        heightmap: heightmap_path,
        biome_mask: biome_path,
        rivers_mask: rivers_path,
        mountains_mask: mountains_path,
        towns: towns_path,
        preview: preview_path,
        settings: settings_path,
        metadata: metadata_path,
        experimental_size: mapsize::is_experimental(size_name),
        clipped_low,
        clipped_high,
        town_count: town_list.len(),
        repaired_pixels,
    })
}

pub fn summary_text(summary: &GenerateSummary) -> String {
    let mut text = String::new();
    text.push_str("Wrote\n");
    text.push_str(&format!(
        "  heightmap      {}\n",
        summary.heightmap.display()
    ));
    text.push_str(&format!(
        "  biome mask     {}\n",
        summary.biome_mask.display()
    ));
    text.push_str(&format!(
        "  river mask     {}\n",
        summary.rivers_mask.display()
    ));
    text.push_str(&format!(
        "  mountain mask  {}\n",
        summary.mountains_mask.display()
    ));
    text.push_str(&format!("  town list      {}\n", summary.towns.display()));
    text.push_str(&format!("  preview        {}\n", summary.preview.display()));
    text.push_str(&format!(
        "  settings       {}\n",
        summary.settings.display()
    ));
    text.push_str("\nNext steps in Transport Fever 3\n");
    text.push_str(&format!(
        "  1. Map Editor -> New Map -> climate 'Temperate', size '{}', ratio '{}'\n",
        mapsize::display_size(&summary.size),
        summary.ratio
    ));
    text.push_str(&format!(
        "  2. Heightmap tool -> Import tab -> select '{}.png' -> Min Height {:.0}, Max Height {:.0}, Water Level {:.0} -> Import\n",
        summary.map_name, summary.min_h, summary.max_h, summary.water_level
    ));
    text.push_str(&format!(
        "  3. Optional: Biomes tab -> pick '{}_rivers.png', '{}_mountains.png', '{}_biomes.png' -> Apply\n",
        summary.map_name, summary.map_name, summary.map_name
    ));
    text.push_str(&format!(
        "  4. Towns & Industries -> Import tab -> select '{}.lua' -> Import\n",
        summary.map_name
    ));
    text.push_str("  5. Street connections tool -> Propose Default Connections -> Generate\n");
    if summary.experimental_size {
        text.push_str(&format!(
            "  note: '{}' is an experimental map size. Set experimentalMapFeatures = true in settings.lua first\n",
            mapsize::display_size(&summary.size)
        ));
    }
    if summary.clipped_high > 0 || summary.clipped_low > 0 {
        text.push_str(&format!(
            "  note: heights were clipped to the range {:.0}..{:.0} m ({} low / {} high pixels)\n",
            summary.min_h, summary.max_h, summary.clipped_low, summary.clipped_high
        ));
    }
    if summary.repaired_pixels > 0 {
        text.push_str(&format!(
            "  note: {} elevation artifact pixels (data seams) were repaired\n",
            summary.repaired_pixels
        ));
    }
    text.push_str("  note: open the preview before importing; if it looks mirrored, regenerate with --flip-x and/or --flip-y\n");
    text.push_str("  note: the *_biomes.png mask is experimental - test it with and without in the Biomes tab\n");
    text
}

fn normalized(options: &GenerateOptions) -> GenerateOptions {
    let mut options = options.clone();
    if options.size.trim().is_empty() {
        options.size = "medium".to_string();
    }
    if options.ratio.trim().is_empty() {
        options.ratio = "1:1".to_string();
    }
    if options.sea_depth <= 0.0 {
        options.sea_depth = 60.0;
    }
    options
}

pub fn resolve_output_root(options: &GenerateOptions) -> Result<PathBuf> {
    if let Some(out) = &options.out {
        return Ok(out.clone());
    }
    if let Some(userdata) = &options.userdata_dir {
        return Ok(userdata.clone());
    }
    if let Some(game_dir) = &options.game_dir {
        if let Some(userdata) = userdata::find_from_game_dir(game_dir) {
            return Ok(userdata);
        }
    }
    userdata::find_userdata().ok_or_else(|| {
        anyhow!(
            "could not detect the Transport Fever 3 userdata folder; \
             set a custom path or run the game once so the folder is created"
        )
    })
}

fn resolve_location(options: &GenerateOptions) -> Result<location::Location> {
    match (&options.location, options.lat, options.lon) {
        (Some(input), _, _) => location::resolve(input),
        (None, Some(lat), Some(lon)) => Ok(location::Location {
            lat,
            lon,
            label: None,
        }),
        (None, Some(_), None) | (None, None, Some(_)) => {
            Err(anyhow!("both latitude and longitude are required"))
        }
        (None, None, None) => Err(anyhow!(
            "no location given; pass a Google Maps URL, a place name or coordinates"
        )),
    }
}

fn detect_sea(elev: &[f32], width: usize, height: usize) -> Vec<bool> {
    let mut sea = vec![false; width * height];
    let mut queue = VecDeque::new();
    let flood = |index: usize, sea: &mut Vec<bool>, queue: &mut VecDeque<usize>| {
        if !sea[index] && elev[index] <= WATER_EPSILON {
            sea[index] = true;
            queue.push_back(index);
        }
    };
    for col in 0..width {
        flood(col, &mut sea, &mut queue);
        if height > 1 {
            flood((height - 1) * width + col, &mut sea, &mut queue);
        }
    }
    for row in 0..height {
        flood(row * width, &mut sea, &mut queue);
        if width > 1 {
            flood(row * width + width - 1, &mut sea, &mut queue);
        }
    }
    while let Some(index) = queue.pop_front() {
        let col = index % width;
        let row = index / width;
        if col > 0 {
            flood(index - 1, &mut sea, &mut queue);
        }
        if col + 1 < width {
            flood(index + 1, &mut sea, &mut queue);
        }
        if row > 0 {
            flood(index - width, &mut sea, &mut queue);
        }
        if row + 1 < height {
            flood(index + width, &mut sea, &mut queue);
        }
    }
    sea
}

fn repair_seams(
    elev: &mut [f32],
    width: usize,
    height: usize,
    threshold: f32,
    radius: usize,
) -> usize {
    let mut repaired = repair_straight_axis(
        elev,
        width,
        height,
        radius,
        SEAM_STRAIGHT_MIN_DEVIATION,
        SEAM_STRAIGHT_MIN_FRACTION,
        true,
    );
    repaired += repair_straight_axis(
        elev,
        width,
        height,
        radius,
        SEAM_STRAIGHT_MIN_DEVIATION,
        SEAM_STRAIGHT_MIN_FRACTION,
        false,
    );
    repaired += repair_axis(elev, width, height, threshold, radius, true);
    repaired += repair_axis(elev, width, height, threshold, radius, false);
    repaired
}

fn repair_straight_axis(
    elev: &mut [f32],
    width: usize,
    height: usize,
    radius: usize,
    min_deviation: f32,
    min_fraction: f32,
    horizontal: bool,
) -> usize {
    if width <= radius * 2 || height <= radius * 2 {
        return 0;
    }
    let original = elev.to_vec();
    let mut repaired = 0;
    let skip_limit = min_deviation * 1.5;
    if horizontal {
        for col in radius..width - radius {
            let mut deviations: Vec<f32> = Vec::with_capacity(height);
            for row in 0..height {
                let offset = row * width + col;
                let left = original[offset - radius];
                let right = original[offset + radius];
                if (left - right).abs() > skip_limit {
                    continue;
                }
                deviations.push((original[offset] - (left + right) * 0.5).abs());
            }
            if deviations.len() < height / 2 {
                continue;
            }
            if !is_straight_seam(&deviations, min_deviation, min_fraction) {
                continue;
            }
            for row in 0..height {
                let offset = row * width + col;
                let left = original[offset - radius];
                let right = original[offset + radius];
                if (left - right).abs() > skip_limit {
                    continue;
                }
                let middle = (left + right) * 0.5;
                if (original[offset] - middle).abs() > min_deviation {
                    elev[offset] = middle;
                    repaired += 1;
                }
            }
        }
    } else {
        for row in radius..height - radius {
            let mut deviations: Vec<f32> = Vec::with_capacity(width);
            for col in 0..width {
                let offset = row * width + col;
                let up = original[offset - radius * width];
                let down = original[offset + radius * width];
                if (up - down).abs() > skip_limit {
                    continue;
                }
                deviations.push((original[offset] - (up + down) * 0.5).abs());
            }
            if deviations.len() < width / 2 {
                continue;
            }
            if !is_straight_seam(&deviations, min_deviation, min_fraction) {
                continue;
            }
            for col in 0..width {
                let offset = row * width + col;
                let up = original[offset - radius * width];
                let down = original[offset + radius * width];
                if (up - down).abs() > skip_limit {
                    continue;
                }
                let middle = (up + down) * 0.5;
                if (original[offset] - middle).abs() > min_deviation {
                    elev[offset] = middle;
                    repaired += 1;
                }
            }
        }
    }
    repaired
}

fn is_straight_seam(deviations: &[f32], min_deviation: f32, min_fraction: f32) -> bool {
    let mut sorted = deviations.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted[sorted.len() / 2];
    if median <= min_deviation {
        return false;
    }
    let anomalous = deviations
        .iter()
        .filter(|deviation| **deviation > min_deviation)
        .count() as f32
        / deviations.len() as f32;
    anomalous > min_fraction
}

fn repair_axis(
    elev: &mut [f32],
    width: usize,
    height: usize,
    threshold: f32,
    radius: usize,
    horizontal: bool,
) -> usize {
    if width <= radius * 2 || height <= radius * 2 {
        return 0;
    }
    let original = elev.to_vec();
    let mut repaired = 0;
    if horizontal {
        for row in 0..height {
            let offset = row * width;
            for col in radius..width - radius {
                let left = original[offset + col - radius];
                let right = original[offset + col + radius];
                if (left - right).abs() > threshold {
                    continue;
                }
                let middle = (left + right) * 0.5;
                if (original[offset + col] - middle).abs() > threshold {
                    elev[offset + col] = middle;
                    repaired += 1;
                }
            }
        }
    } else {
        for col in 0..width {
            for row in radius..height - radius {
                let up = original[(row - radius) * width + col];
                let down = original[(row + radius) * width + col];
                if (up - down).abs() > threshold {
                    continue;
                }
                let middle = (up + down) * 0.5;
                if (original[row * width + col] - middle).abs() > threshold {
                    elev[row * width + col] = middle;
                    repaired += 1;
                }
            }
        }
    }
    repaired
}

fn smooth3x3(data: &[f32], width: usize, height: usize) -> Vec<f32> {
    let mut out = vec![0f32; data.len()];
    for row in 0..height {
        for col in 0..width {
            let mut sum = 0.0f32;
            let mut count = 0.0f32;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let x = col as i32 + dx;
                    let y = row as i32 + dy;
                    if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
                        sum += data[y as usize * width + x as usize];
                        count += 1.0;
                    }
                }
            }
            out[row * width + col] = sum / count;
        }
    }
    out
}

fn estimate_water_level(
    sea: &[bool],
    terrain: &[f32],
    water: &[bool],
    width: usize,
    height: usize,
) -> f64 {
    if sea.iter().any(|value| *value) {
        return 0.0;
    }
    let mut visited = vec![false; width * height];
    let mut best: Vec<usize> = Vec::new();
    let mut queue = VecDeque::new();
    for start in 0..water.len() {
        if !water[start] || visited[start] {
            continue;
        }
        let mut component: Vec<usize> = Vec::new();
        visited[start] = true;
        queue.push_back(start);
        while let Some(index) = queue.pop_front() {
            component.push(index);
            let col = index % width;
            let row = index / width;
            let visit = |neighbour: usize, visited: &mut Vec<bool>, queue: &mut VecDeque<usize>| {
                if !visited[neighbour] && water[neighbour] {
                    visited[neighbour] = true;
                    queue.push_back(neighbour);
                }
            };
            if col > 0 {
                visit(index - 1, &mut visited, &mut queue);
            }
            if col + 1 < width {
                visit(index + 1, &mut visited, &mut queue);
            }
            if row > 0 {
                visit(index - width, &mut visited, &mut queue);
            }
            if row + 1 < height {
                visit(index + width, &mut visited, &mut queue);
            }
        }
        if component.len() > best.len() {
            best = component;
        }
    }
    if best.is_empty() {
        return 0.0;
    }

    let mut shoreline: Vec<f32> = Vec::new();
    for &index in &best {
        let col = index % width;
        let row = index / width;
        let at_border = col == 0 || row == 0 || col + 1 == width || row + 1 == height;
        let touches_land = (col > 0 && !water[index - 1])
            || (col + 1 < width && !water[index + 1])
            || (row > 0 && !water[index - width])
            || (row + 1 < height && !water[index + width]);
        if at_border || touches_land {
            shoreline.push(terrain[index]);
        }
    }

    let mut values: Vec<f32> = if shoreline.len() >= 8 {
        shoreline
    } else {
        best.iter().map(|&index| terrain[index]).collect()
    };
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values[values.len() / 2].round() as f64
}

fn choose_range(options: &GenerateOptions, elev: &[f32]) -> (f64, f64, usize, usize) {
    let data_min = min_of(elev) as f64;
    let data_max = max_of(elev) as f64;
    let min_h = options
        .min_height
        .unwrap_or_else(|| data_min.floor())
        .max(-1000.0);
    let max_h = options
        .max_height
        .unwrap_or_else(|| data_max.ceil().min(MAX_AUTO_HEIGHT))
        .max(min_h + 1.0);
    let mut clipped_low = 0;
    let mut clipped_high = 0;
    for value in elev {
        let value = *value as f64;
        if value < min_h {
            clipped_low += 1;
        }
        if value > max_h {
            clipped_high += 1;
        }
    }
    (min_h, max_h, clipped_low, clipped_high)
}

fn slope_at(elev: &[f32], width: usize, height: usize, col: usize, row: usize) -> f32 {
    let x0 = col.saturating_sub(1);
    let x1 = (col + 1).min(width - 1);
    let y0 = row.saturating_sub(1);
    let y1 = (row + 1).min(height - 1);
    let dx = (elev[row * width + x1] - elev[row * width + x0]) / ((x1 - x0) as f32 * 4.0);
    let dy = (elev[y1 * width + col] - elev[y0 * width + col]) / ((y1 - y0) as f32 * 4.0);
    (dx * dx + dy * dy).sqrt()
}

fn min_of(values: &[f32]) -> f32 {
    values.iter().copied().fold(f32::INFINITY, f32::min)
}

fn max_of(values: &[f32]) -> f32 {
    values.iter().copied().fold(f32::NEG_INFINITY, f32::max)
}

fn flip_horizontal<T: Copy>(data: &mut [T], width: usize, height: usize) {
    for row in 0..height {
        let start = row * width;
        for col in 0..width / 2 {
            data.swap(start + col, start + width - 1 - col);
        }
    }
}

fn flip_vertical<T: Copy>(data: &mut [T], width: usize, height: usize) {
    for row in 0..height / 2 {
        let top = row * width;
        let bottom = (height - 1 - row) * width;
        for col in 0..width {
            data.swap(top + col, bottom + col);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn settings_text(
    map_name: &str,
    location: &location::Location,
    size: &str,
    ratio: &str,
    width: u32,
    height: u32,
    min_h: f64,
    max_h: f64,
    water_level: f64,
    skip_osm: bool,
    clipped_low: usize,
    clipped_high: usize,
    repaired_pixels: usize,
) -> String {
    let mut text = String::new();
    text.push_str("Transport Fever 3 import settings\n");
    text.push_str("=================================\n");
    text.push_str(&format!("Map name       : {map_name}\n"));
    text.push_str(&format!(
        "Location       : {:.6}, {:.6}\n",
        location.lat, location.lon
    ));
    if let Some(label) = &location.label {
        text.push_str(&format!("Place          : {label}\n"));
    }
    text.push_str(&format!(
        "Map size/ratio : {} ({ratio}) - select this in the Map Editor\n",
        mapsize::display_size(size)
    ));
    text.push_str(&format!(
        "Heightmap      : {map_name}.png ({width} x {height} px)\n"
    ));
    text.push_str(&format!("Minimum Height : {min_h:.0}\n"));
    text.push_str(&format!("Maximum Height : {max_h:.0}\n"));
    text.push_str(&format!("Water Level    : {water_level:.0}\n"));
    if clipped_low > 0 || clipped_high > 0 {
        text.push_str(&format!(
            "Clipped pixels : {clipped_low} low, {clipped_high} high\n"
        ));
    }
    if repaired_pixels > 0 {
        text.push_str(&format!(
            "Repaired pixels: {repaired_pixels} (elevation data seams)\n"
        ));
    }
    text.push_str("\nSteps\n");
    text.push_str("1. Map Editor -> New Map -> climate Temperate (or Tropical/Dry/Subarctic), matching size and ratio\n");
    text.push_str(&format!(
        "2. Heightmap tool -> Import tab -> select '{map_name}.png'\n"
    ));
    text.push_str(&format!("   set Minimum Height = {min_h:.0}, Maximum Height = {max_h:.0}, Water Level = {water_level:.0}\n"));
    text.push_str(
        "3. Optional: Biomes tab -> select the *_rivers / *_mountains / *_biomes masks -> Apply\n",
    );
    text.push_str("   (the *_biomes mask is experimental, test it with and without)\n");
    text.push_str(&format!(
        "4. Towns & Industries -> Import tab -> select '{map_name}.lua' -> Import\n"
    ));
    text.push_str("5. Street connections tool -> Propose Default Connections -> Generate\n");
    text.push('\n');
    if skip_osm {
        text.push_str(
            "OpenStreetMap was skipped: no real water bodies, forests or towns in this output.\n",
        );
    }
    if mapsize::is_experimental(size) {
        text.push_str(
            "Experimental map size: set experimentalMapFeatures = true in settings.lua first.\n",
        );
    }
    text.push_str(
        "If the imported terrain is mirrored, regenerate with --flip-x and/or --flip-y.\n",
    );
    text
}

pub fn sanitize_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '(' | ')') {
                c
            } else if c.is_whitespace() || c == '.' {
                ' '
            } else {
                '_'
            }
        })
        .collect();
    while out.contains("  ") {
        out = out.replace("  ", " ");
    }
    out = out.trim().trim_matches('.').trim().to_string();
    if out.chars().count() > 60 {
        out = out.chars().take(60).collect();
    }
    if out.is_empty() {
        "TF3 Map".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sea_detection_floods_from_border_only() {
        let width = 5;
        let height = 5;
        let mut elev = vec![100.0f32; width * height];
        for row in 0..height {
            elev[row * width] = 0.0;
        }
        elev[2 * width + 2] = 0.0;
        let sea = detect_sea(&elev, width, height);
        assert!(sea[4 * width]);
        assert!(!sea[2 * width + 2]);
        assert!(!sea[width + 1]);
    }

    #[test]
    fn estimate_water_level_is_zero_when_sea_exists() {
        let width = 4;
        let height = 4;
        let elev = vec![100.0f32; width * height];
        let mut sea = vec![false; width * height];
        sea[0] = true;
        let level = estimate_water_level(&sea, &elev, &sea, width, height);
        assert_eq!(level, 0.0);
    }

    #[test]
    fn estimate_water_level_uses_largest_lake() {
        let width = 10;
        let height = 10;
        let mut terrain = vec![900.0f32; width * height];
        let mut water = vec![false; width * height];
        for (row, col) in [(1, 1), (1, 2), (2, 1), (2, 2)] {
            water[row * width + col] = true;
            terrain[row * width + col] = 100.0;
        }
        water[8 * width + 8] = true;
        terrain[8 * width + 8] = 500.0;
        let sea = vec![false; width * height];
        let level = estimate_water_level(&sea, &terrain, &water, width, height);
        assert!((level - 100.0).abs() < 1e-9);
    }

    #[test]
    fn estimate_water_level_ignores_bathymetry() {
        let width = 11;
        let height = 11;
        let mut terrain = vec![180.0f32; width * height];
        let mut water = vec![false; width * height];
        for row in 3..8 {
            for col in 3..8 {
                water[row * width + col] = true;
                let shoreline = row == 3 || row == 7 || col == 3 || col == 7;
                terrain[row * width + col] = if shoreline { 150.0 } else { 100.0 };
            }
        }
        let sea = vec![false; width * height];
        let level = estimate_water_level(&sea, &terrain, &water, width, height);
        assert!((level - 150.0).abs() < 1e-9, "level was {level}");
    }

    #[test]
    fn repair_seams_fixes_narrow_trench_and_keeps_cliffs() {
        let width = 40;
        let height = 40;
        let mut elev = vec![200.0f32; width * height];
        for row in 0..height {
            for col in 20..22 {
                elev[row * width + col] = 100.0;
            }
            for col in 34..width {
                elev[row * width + col] = 150.0;
            }
        }
        let repaired = repair_seams(&mut elev, width, height, 25.0, 5);
        assert!(repaired > 0);
        for row in 0..height {
            assert!(
                (elev[row * width + 20] - 200.0).abs() < 1.0,
                "trench not repaired at row {row}"
            );
        }
        assert!((elev[20 * width + 30] - 200.0).abs() < 1e-6);
        assert!((elev[20 * width + 36] - 150.0).abs() < 1e-6);
    }

    #[test]
    fn repair_seams_keeps_gentle_slopes() {
        let width = 30;
        let height = 30;
        let mut elev = vec![0f32; width * height];
        for row in 0..height {
            for col in 0..width {
                elev[row * width + col] = col as f32 * 2.0;
            }
        }
        let before = elev.clone();
        let repaired = repair_seams(&mut elev, width, height, 25.0, 5);
        assert_eq!(repaired, 0);
        assert_eq!(elev, before);
    }

    #[test]
    fn straight_seam_pass_fixes_shallow_linear_trench() {
        let width = 40;
        let height = 60;
        let mut elev = vec![200.0f32; width * height];
        for row in 0..height {
            elev[row * width + 20] = 180.0;
        }
        let repaired = repair_straight_axis(&mut elev, width, height, 6, 10.0, 0.6, true);
        assert!(repaired >= height - 12, "repaired {repaired}");
        for row in 0..height {
            assert!((elev[row * width + 20] - 200.0).abs() < 1.0);
        }
    }

    #[test]
    fn straight_seam_pass_keeps_linear_slopes() {
        let width = 30;
        let height = 40;
        let mut elev = vec![0f32; width * height];
        for row in 0..height {
            for col in 0..width {
                elev[row * width + col] = col as f32 * 3.0;
            }
        }
        let before = elev.clone();
        let repaired = repair_straight_axis(&mut elev, width, height, 6, 10.0, 0.6, false);
        assert_eq!(repaired, 0);
        assert_eq!(elev, before);
    }

    #[test]
    fn smoothing_reduces_single_pixel_spikes() {
        let width = 5;
        let height = 5;
        let mut data = vec![100.0f32; width * height];
        data[2 * width + 2] = 1000.0;
        let smooth = smooth3x3(&data, width, height);
        assert!(smooth[2 * width + 2] < 250.0);
        assert!((smooth[0] - 100.0).abs() < 1e-6);
    }

    #[test]
    fn sanitize_keeps_unicode_and_removes_path_chars() {
        assert_eq!(sanitize_name("São Paulo"), "São Paulo");
        assert_eq!(sanitize_name("A/B:C*D"), "A_B_C_D");
        assert_eq!(sanitize_name("  ...  "), "TF3 Map");
    }

    #[test]
    fn flips_reverse_data() {
        let mut data = vec![1u8, 2, 3, 4, 5, 6];
        flip_horizontal(&mut data, 3, 2);
        assert_eq!(data, vec![3, 2, 1, 6, 5, 4]);
        flip_vertical(&mut data, 3, 2);
        assert_eq!(data, vec![6, 5, 4, 3, 2, 1]);
    }
}
