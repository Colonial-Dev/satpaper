use std::path::PathBuf;

use anyhow::{bail, Result};
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
    #[arg(short = 'x', long, value_parser = clap::value_parser!(u32).range(1..), env = "SATPAPER_RESOLUTION_X")]
    pub resolution_x: u32,
    /// The Y resolution/height of the generated wallpaper.
    #[arg(short = 'y', long, value_parser = clap::value_parser!(u32).range(1..), env = "SATPAPER_RESOLUTION_Y")]
    pub resolution_y: u32,
    /// The SLIDER sector to source imagery from.
    #[arg(
        short = 'c',
        long,
        env = "SATPAPER_SECTOR",
        default_value = "full_disk"
    )]
    pub sector: Sector,
    /// The SLIDER product to source. Defaults to the selected sector's preferred product.
    #[arg(short = 'p', long, env = "SATPAPER_PRODUCT")]
    pub product: Option<String>,
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

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub enum Sector {
    #[value(name = "full_disk")]
    FullDisk,
    #[value(name = "conus")]
    Conus,
    #[value(name = "mesoscale_01")]
    Mesoscale01,
    #[value(name = "mesoscale_02")]
    Mesoscale02,
    #[value(name = "japan")]
    Japan,
}

impl Sector {
    pub fn id(self) -> &'static str {
        match self {
            Self::FullDisk => "full_disk",
            Self::Conus => "conus",
            Self::Mesoscale01 => "mesoscale_01",
            Self::Mesoscale02 => "mesoscale_02",
            Self::Japan => "japan",
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct ContentBounds {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub denominator: u32,
}

impl ContentBounds {
    const FULL: Self = Self {
        left: 0,
        top: 0,
        right: 1,
        bottom: 1,
        denominator: 1,
    };

    const GOES_CONUS: Self = Self {
        left: 0,
        top: 124,
        right: 625,
        bottom: 501,
        denominator: 625,
    };

    const HIMAWARI_JAPAN: Self = Self {
        left: 76,
        top: 120,
        right: 724,
        bottom: 636,
        denominator: 750,
    };

    pub fn aspect_ratio(self) -> f64 {
        f64::from(self.right - self.left) / f64::from(self.bottom - self.top)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Composition {
    FullDisk,
    Regional(ContentBounds),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct SourceSpec {
    pub sector: Sector,
    pub max_zoom: u32,
    pub native_tile_size: u32,
    pub default_product: &'static str,
    pub composition: Composition,
}

impl SourceSpec {
    pub fn tile_count(self) -> u32 {
        1 << self.max_zoom
    }
}

impl Config {
    pub fn disk(&self) -> u32 {
        let smaller_dim = self.resolution_x.min(self.resolution_y);

        let disk_dim = smaller_dim as f32 * (self.disk_size as f32 / 100.0);
        disk_dim.floor() as u32
    }

    pub fn source_spec(&self) -> Result<SourceSpec> {
        self.satellite.source_spec(self.sector)
    }

    pub fn product_for<'a>(&'a self, source: &SourceSpec) -> &'a str {
        self.product.as_deref().unwrap_or(source.default_product)
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
            Meteosat10 => "meteosat-0deg",
        }
    }

    pub fn source_spec(self, sector: Sector) -> Result<SourceSpec> {
        use Satellite::*;
        use Sector::*;

        let source = match (self, sector) {
            (GOESEast | GOESWest, FullDisk) => SourceSpec {
                sector,
                max_zoom: 4,
                native_tile_size: 678,
                default_product: "geocolor",
                composition: Composition::FullDisk,
            },
            (GOESEast | GOESWest, Conus) => SourceSpec {
                sector,
                max_zoom: 4,
                native_tile_size: 625,
                default_product: "geocolor",
                composition: Composition::Regional(ContentBounds::GOES_CONUS),
            },
            (GOESEast | GOESWest, Mesoscale01 | Mesoscale02) => SourceSpec {
                sector,
                max_zoom: 2,
                native_tile_size: 500,
                default_product: "geocolor",
                composition: Composition::Regional(ContentBounds::FULL),
            },
            (Himawari, FullDisk) => SourceSpec {
                sector,
                max_zoom: 4,
                native_tile_size: 688,
                default_product: "geocolor",
                composition: Composition::FullDisk,
            },
            (Himawari, Japan) => SourceSpec {
                sector,
                max_zoom: 3,
                native_tile_size: 750,
                default_product: "band_13",
                composition: Composition::Regional(ContentBounds::HIMAWARI_JAPAN),
            },
            (Himawari, Mesoscale01) => SourceSpec {
                sector,
                max_zoom: 2,
                native_tile_size: 500,
                default_product: "band_13",
                composition: Composition::Regional(ContentBounds::FULL),
            },
            (Meteosat9 | Meteosat10, FullDisk) => SourceSpec {
                sector,
                max_zoom: 3,
                native_tile_size: 464,
                default_product: "geocolor",
                composition: Composition::FullDisk,
            },
            _ => bail!(
                "sector '{}' is not available for satellite '{}'",
                sector.id(),
                self.id()
            ),
        };

        Ok(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn required_args() -> Vec<&'static str> {
        vec![
            "satpaper",
            "--satellite",
            "goes-east",
            "--resolution-x",
            "1920",
            "--resolution-y",
            "1080",
            "--disk-size",
            "95",
            "--target-path",
            ".",
        ]
    }

    #[test]
    fn defaults_to_full_disk_and_sector_product() {
        let config = Config::try_parse_from(required_args()).unwrap();
        let source = config.source_spec().unwrap();

        assert_eq!(config.sector, Sector::FullDisk);
        assert!(config.product.is_none());
        assert_eq!(config.product_for(&source), "geocolor");
    }

    #[test]
    fn parses_sector_and_product_overrides() {
        let mut args = required_args();
        args.extend(["--sector", "conus", "--product", "cira_geoproxy"]);
        let config = Config::try_parse_from(args).unwrap();
        let source = config.source_spec().unwrap();

        assert_eq!(config.sector, Sector::Conus);
        assert_eq!(config.product_for(&source), "cira_geoproxy");
    }

    #[test]
    fn source_geometry_matches_slider() {
        let conus = Satellite::GOESEast.source_spec(Sector::Conus).unwrap();
        assert_eq!(conus.max_zoom, 4);
        assert_eq!(conus.tile_count(), 16);
        assert_eq!(conus.native_tile_size, 625);

        let mesoscale = Satellite::GOESWest
            .source_spec(Sector::Mesoscale02)
            .unwrap();
        assert_eq!(mesoscale.max_zoom, 2);
        assert_eq!(mesoscale.tile_count(), 4);

        let japan = Satellite::Himawari.source_spec(Sector::Japan).unwrap();
        assert_eq!(japan.max_zoom, 3);
        assert_eq!(japan.tile_count(), 8);
        assert_eq!(japan.default_product, "band_13");
    }

    #[test]
    fn rejects_unavailable_sector_for_satellite() {
        let error = Satellite::Meteosat9.source_spec(Sector::Conus).unwrap_err();
        assert!(error.to_string().contains("not available"));
    }

    #[test]
    fn rejects_zero_dimensions() {
        for flag in ["--resolution-x", "--resolution-y"] {
            let mut args = required_args();
            let value = args.iter().position(|arg| *arg == flag).unwrap() + 1;
            args[value] = "0";
            assert!(Config::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn conus_bounds_preserve_regional_aspect_ratio() {
        let Composition::Regional(bounds) = Satellite::GOESEast
            .source_spec(Sector::Conus)
            .unwrap()
            .composition
        else {
            panic!("CONUS must use regional composition");
        };

        assert!((bounds.aspect_ratio() - (625.0 / 377.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn japan_bounds_cover_the_slider_footprint() {
        let Composition::Regional(bounds) = Satellite::Himawari
            .source_spec(Sector::Japan)
            .unwrap()
            .composition
        else {
            panic!("Japan must use regional composition");
        };

        assert_eq!(bounds, ContentBounds::HIMAWARI_JAPAN);
        assert!((bounds.aspect_ratio() - (648.0 / 516.0)).abs() < f64::EPSILON);
    }
}
