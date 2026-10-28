pub mod build;
pub mod cli;
pub mod elevation;
pub mod geo;
pub mod location;
pub mod mapsize;
pub mod osm;
pub mod progress;
pub mod raster;
pub mod render;
pub mod towns;
pub mod userdata;

use std::path::PathBuf;

pub use build::{generate, summary_text, GenerateSummary};
pub use progress::{Progress, SilentProgress, StderrProgress};

#[derive(Clone, Debug, Default)]
pub struct GenerateOptions {
    pub location: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub name: Option<String>,
    pub size: String,
    pub ratio: String,
    pub water_level: Option<f64>,
    pub min_height: Option<f64>,
    pub max_height: Option<f64>,
    pub sea_depth: f64,
    pub zoom: Option<u8>,
    pub game_dir: Option<PathBuf>,
    pub userdata_dir: Option<PathBuf>,
    pub out: Option<PathBuf>,
    pub skip_osm: bool,
    pub osm_endpoint: Option<String>,
    pub max_towns: Option<usize>,
    pub flip_x: bool,
    pub flip_y: bool,
    pub repair_seams: bool,
}

impl GenerateOptions {
    pub fn with_defaults() -> Self {
        GenerateOptions {
            size: "medium".to_string(),
            ratio: "1:1".to_string(),
            sea_depth: 60.0,
            repair_seams: true,
            ..Default::default()
        }
    }
}
