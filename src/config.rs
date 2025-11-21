use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use fimg::Image;

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

    /// The product requested. (geocolor, etc)
    #[arg(short = 'p', long, env = "SATPAPER_PRODUCT", default_value = "geocolor")]
    pub product: String,

    /// The sector requested. (full_disk, conus, etc)
    #[arg(short = 'c', long, env = "SATPAPER_SECTOR", default_value = "full_disk")]
    pub sector: String,

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

impl Config {
    pub fn disk(&self) -> u32 {
        let smaller_dim = self.resolution_x.min(self.resolution_y);

        let disk_dim = smaller_dim as f32 * (self.disk_size as f32 / 100.0);
        disk_dim.floor() as u32
    }
}

impl Satellite {
    pub fn id(self) -> &'static str {
        use Satellite::*;

        match self {
            GOESEast => "goes-19",
            GOESWest => "goes-18",
            Himawari => "himawari",
            Meteosat9 => "meteosat-9",
            Meteosat10 => "meteosat-0deg"
        }
    }

    pub fn max_zoom(self) -> u32 {
        use Satellite::*;

        match self {
            GOESEast | GOESWest | Himawari => 4,
            Meteosat9 | Meteosat10 => 3,
        }
    }

    pub fn tile_image(self, sector: &String) -> Image<Box<[u8]>, 3> {
        Image::alloc(self.tile_size(sector), self.tile_size(sector)).boxed()
    }

    pub fn tile_count(self) -> u32 {
        use Satellite::*;

        match self {
            GOESEast | GOESWest | Himawari => 16,
            Meteosat9 | Meteosat10 => 8,
        }
    }

    pub fn tile_size(self, sector: &String) -> u32 {
        use Satellite::*;

        match self {
            GOESEast | GOESWest => match sector.as_str() {
                "full_disk" => 678,
                "conus" => 625,
                "mesoscale_01" => 500,
                "mesoscale_02" => 500,
                _ => 678
            },
            Himawari => match sector.as_str() {
                "full_disk" => 688,
                "japan" => 750,
                "mesoscale_01" => 500,
                _ => 688
            },
            Meteosat9 | Meteosat10 =>  match sector.as_str() {
                "full_disk" => 464,
                _ => 464
            },
        }
    }
}