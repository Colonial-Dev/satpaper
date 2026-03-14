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
    #[arg(short, long, env = "SPACEPAPER_SATELLITE")]
    pub satellite: Satellite,
    /// The X resolution/width of the generated wallpaper.
    #[arg(short = 'x', long, env = "SPACEPAPER_RESOLUTION_X")]
    pub resolution_x: u32,
    /// The Y resolution/height of the generated wallpaper.
    #[arg(short = 'y', long, env = "SPACEPAPER_RESOLUTION_Y")]
    pub resolution_y: u32,
    /// The size of the "disk" (Earth) relative to the generated wallpaper's
    /// smaller dimension.
    ///
    /// Values in the 90-95 range are the best if you want maximum detail.
    #[arg(short, long, value_parser = clap::value_parser!(u32).range(1..=200), env = "SPACEPAPER_DISK_SIZE")]
    pub disk_size: u32,
    /// The horizontal position of the disk as a percentage of the wallpaper width.
    ///
    /// 0 = left edge, 50 = centered, 100 = right edge.
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=100), env = "SPACEPAPER_DISK_X", default_value_t = 50)]
    pub disk_x: u32,
    /// The vertical position of the disk as a percentage of the wallpaper height.
    ///
    /// 0 = top edge, 50 = centered, 100 = bottom edge.
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=100), env = "SPACEPAPER_DISK_Y", default_value_t = 50)]
    pub disk_y: u32,
    /// Where generated wallpapers should be saved.
    /// 
    /// Spacepaper will output to a file called "spacepaper_latest.png" at this path.
    #[arg(short, long, env = "SPACEPAPER_TARGET_PATH")]
    pub target_path: PathBuf,
    /// Command to run to change the wallpaper. This overrides automatic update handling.
    /// 
    /// The command will be ran as `sh -c "{wallpaper_command} file://{path}"`. 
    #[arg(short, long, env = "SPACEPAPER_WALLPAPER_COMMAND")]
    pub wallpaper_command: Option<String>,
    /// Whether or not to only run once.
    /// 
    /// By default, Spacepaper is designed to run in the background - it stays resident once launched
    /// and periodically attempts to update your wallpaper.
    /// 
    /// With --once set, Spacepaper will instead generate one wallpaper and terminate, without
    /// affecting your existing wallpaper or staying resident.
    /// 
    /// This is ideal if you want to use Spacepaper as a simple wallpaper generator or as part of a larger script/program.
    #[arg(short, long, env = "SPACEPAPER_ONCE", default_value_t = false)]
    pub once: bool,
    /// A background image to use instead of the default pure black.
    /// 
    /// For best results, the image should match the specified resolution, 
    /// but Spacepaper will resize the image to fit if need be.
    #[arg(short, long, env = "SPACEPAPER_BACKGROUND_IMAGE")]
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

    /// Returns the (x, y) pixel offset for placing the disk on the wallpaper.
    ///
    /// The percentage positions the *center* of the disk, so at 50/50 the disk
    /// is centered, and at 100/100 the disk center is at the bottom-right corner
    /// (only the top-left quadrant visible).
    pub fn disk_offset(&self) -> (i32, i32) {
        let disk_dim = self.disk() as i32;
        let half = disk_dim / 2;

        let center_x = (self.resolution_x as f32 * (self.disk_x as f32 / 100.0)).floor() as i32;
        let center_y = (self.resolution_y as f32 * (self.disk_y as f32 / 100.0)).floor() as i32;

        (center_x - half, center_y - half)
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

    pub fn tile_image(self) -> Image<Box<[u8]>, 3> {
        Image::alloc(self.tile_size(), self.tile_size()).boxed()
    }

    pub fn tile_count(self) -> u32 {
        use Satellite::*;

        match self {
            GOESEast | GOESWest | Himawari => 16,
            Meteosat9 | Meteosat10 => 8,
        }
    }

    pub fn tile_size(self) -> u32 {
        use Satellite::*;

        match self {
            GOESEast | GOESWest => 678,
            Himawari => 688,
            Meteosat9 | Meteosat10 => 464,
        }
    }
}