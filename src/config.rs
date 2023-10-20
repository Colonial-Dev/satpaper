use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// The satellite to source imagery from.
    /// 
    /// Options include:
    /// 
    /// - GOES East (covers most of North and South America)
    /// 
    /// - GOES West (Pacific Ocean and parts of the western US)
    /// 
    /// - Himawari (Oceania and East Asia)
    /// 
    /// - Meteosat 9 (Africa, Middle East, India, Central Asia)
    /// 
    /// - Meteosat 10 (Atlantic Ocean, Africa, Europe)
    #[arg(short, long, env = "SATPAPER_SATELLITE")]
    pub satellite: Satellite,
    /// The X resolution/width of the generated wallpaper.
    #[arg(short = 'x', long, env = "SATPAPER_RESOLUTION_X")]
    pub resolution_x: u32,
    /// The Y resolution/height of the generated wallpaper.
    #[arg(short = 'y', long, env = "SATPAPER_RESOLUTION_Y")]
    pub resolution_y: u32,
    /// The size of the "disk" (Earth) relative to the generated wallpaper's
    /// smaller dimension.
    /// 
    /// Values in the 90-95 range are the best if you want maximum detail.
    #[arg(short, long, value_parser = clap::value_parser!(u32).range(1..=100), env = "SATPAPER_DISK_SIZE")]
    pub disk_size: u32,
    /// Where generated wallpapers should be saved.
    /// 
    /// Satpaper will output to a file called "satpaper_latest.png" at this path.
    #[arg(short, long, env = "SATPAPER_TARGET_PATH")]
    pub target_path: PathBuf,
    /// Command to run to change the wallpaper. This overrides automatic update handling.
    /// 
    /// The command will be ran as `sh -c "{wallpaper_command} file://{path}"`. 
    #[arg(short, long, env = "SATPAPER_WALLPAPER_COMMAND")]
    pub wallpaper_command: Option<String>,
    /// Whether or not to only run once.
    /// 
    /// By default, Satpaper is designed to run in the background - it stays resident once launched
    /// and periodically attempts to update your wallpaper.
    /// 
    /// With --once set, Satpaper will instead generate one wallpaper and terminate, without
    /// affecting your existing wallpaper or staying resident.
    /// 
    /// This is ideal if you want to use Satpaper as a simple wallpaper generator or as part of a larger script/program.
    #[arg(short, long, env = "SATPAPER_ONCE", default_value_t = false)]
    pub once: bool,
    /// A background image to use instead of the default pure black.
    /// 
    /// For best results, the image should match the specified resolution, 
    /// but Satpaper will resize the image to fit if need be.
    #[arg(short, long, env = "SATPAPER_BACKGROUND_IMAGE")]
    pub background_image: Option<PathBuf>,
}

#[derive(Debug, Copy, Clone, ValueEnum)]
pub enum Satellite {
    GOESEast,
    GOESWest,
    Himawari,
    Meteosat9,
    Meteosat10,
}

impl Satellite {
    pub fn id(&self) -> &'static str {
        use Satellite::*;

        match self {
            GOESEast => "goes-16",
            GOESWest => "goes-18",
            Himawari => "himawari",
            Meteosat9 => "meteosat-9",
            Meteosat10 => "meteosat-0deg"
        }
    }

    pub fn max_zoom(&self) -> usize {
        use Satellite::*;

        match self {
            GOESEast | GOESWest | Himawari => 4,
            Meteosat9 | Meteosat10 => 3,
        }
    }

    pub fn image_size(&self) -> usize {
        use Satellite::*;

        match self {
            GOESEast | GOESWest | Himawari => 10_848,
            Meteosat9 | Meteosat10 => 3_712,
        }
    }

    pub fn tile_count(&self) -> usize {
        use Satellite::*;

        match self {
            GOESEast | GOESWest | Himawari => 16,
            Meteosat9 | Meteosat10 => 8,
        }
    }

    pub fn tile_size(&self) -> usize {
        use Satellite::*;

        match self {
            GOESEast | GOESWest | Himawari => 678,
            Meteosat9 | Meteosat10 => 464,
        }
    }
}