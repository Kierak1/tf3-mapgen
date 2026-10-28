use std::path::PathBuf;

use clap::Parser;

use crate::GenerateOptions;

#[derive(Parser, Debug)]
#[command(
    name = "tf3-mapgen",
    version,
    about = "Generate Transport Fever 3 maps from real-world locations",
    long_about = "Fetches elevation (AWS Terrarium) and OpenStreetMap data for a location and writes a \
                 16-bit heightmap, biome/river/mountain masks and town positions into the Transport Fever 3 \
                 userdata folders, ready to be imported in the built-in Map Editor."
)]
pub struct Cli {
    /// Google Maps URL, a place name, or "lat,lon"
    pub location: Option<String>,

    /// Latitude (alternative to LOCATION)
    #[arg(long, allow_hyphen_values = true)]
    pub lat: Option<f64>,

    /// Longitude (alternative to LOCATION)
    #[arg(long, allow_hyphen_values = true)]
    pub lon: Option<f64>,

    /// Output map name (default: derived from the location)
    #[arg(long)]
    pub name: Option<String>,

    /// Map size: tiny, small, medium, large, verylarge, huge, megalomaniac, gigantomaniac
    #[arg(long, default_value = "medium")]
    pub size: String,

    /// Aspect ratio: 1:1, 1:2, 1:3, 1:4, 1:5
    #[arg(long, default_value = "1:1")]
    pub ratio: String,

    /// Water level in meters (default: auto-detected from the largest water body; 0 if the map has a sea)
    #[arg(long)]
    pub water_level: Option<f64>,

    /// Minimum height in meters (default: derived from the terrain)
    #[arg(long)]
    pub min_height: Option<f64>,

    /// Maximum height in meters (default: derived from the terrain)
    #[arg(long)]
    pub max_height: Option<f64>,

    /// How deep seas are carved below the water level, in meters
    #[arg(long, default_value_t = 60.0)]
    pub sea_depth: f64,

    /// Elevation data zoom level 8-15 (default: auto)
    #[arg(long)]
    pub zoom: Option<u8>,

    /// Transport Fever 3 installation folder (used to locate the userdata folder)
    #[arg(long)]
    pub game_dir: Option<PathBuf>,

    /// Transport Fever 3 userdata folder (default: auto-detected)
    #[arg(long)]
    pub userdata: Option<PathBuf>,

    /// Write output to this directory instead of the game userdata folder
    #[arg(long, short)]
    pub out: Option<PathBuf>,

    /// Do not query OpenStreetMap (no water bodies, forests or towns)
    #[arg(long)]
    pub skip_osm: bool,

    /// Use a specific Overpass API endpoint instead of the built-in fallbacks
    #[arg(long)]
    pub osm_endpoint: Option<String>,

    /// Maximum number of towns to place (default: scaled with map area)
    #[arg(long)]
    pub max_towns: Option<usize>,

    /// Flip the map vertically (use if the in-game preview is mirrored north/south)
    #[arg(long)]
    pub flip_y: bool,

    /// Flip the map horizontally (use if the in-game preview is mirrored east/west)
    #[arg(long)]
    pub flip_x: bool,

    /// Disable the automatic repair of narrow elevation data seams/outliers
    #[arg(long)]
    pub no_seam_repair: bool,

    /// Print the detected Transport Fever 3 userdata folder and exit
    #[arg(long)]
    pub print_userdata: bool,
}

impl Cli {
    pub fn into_options(self) -> GenerateOptions {
        GenerateOptions {
            location: self.location,
            lat: self.lat,
            lon: self.lon,
            name: self.name,
            size: self.size,
            ratio: self.ratio,
            water_level: self.water_level,
            min_height: self.min_height,
            max_height: self.max_height,
            sea_depth: self.sea_depth,
            zoom: self.zoom,
            game_dir: self.game_dir,
            userdata_dir: self.userdata,
            out: self.out,
            skip_osm: self.skip_osm,
            osm_endpoint: self.osm_endpoint,
            max_towns: self.max_towns,
            flip_x: self.flip_x,
            flip_y: self.flip_y,
            repair_seams: !self.no_seam_repair,
        }
    }
}
