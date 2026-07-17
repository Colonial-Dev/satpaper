use std::io::Read;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use fimg::{scale::Lanczos3, Image as Img, OverlayAt};
use rayon::prelude::*;
use serde::{de, Deserialize};

use ureq::AgentBuilder;

use super::{Composition, Config, ContentBounds, SourceSpec, OUTPUT_NAME};

/// rgb all the way down
pub type Image<T> = Img<T, 3>;

const SLIDER_BASE_URL: &str = "https://rammb-slider.cira.colostate.edu";

const TIMEOUT: Duration = Duration::from_secs(30);

pub fn composite_latest_image(config: &Config, timestamp: u64) -> Result<bool> {
    download(config, timestamp)
        .and_then(|image| {
            composite(config, image)?;
            Ok(true)
        })
        .or_else(|err| {
            if config.once {
                return Err(err);
            }

            log::error!("Failed to download source image: {err}");
            log::error!("Composition aborted; waiting until next go round.");
            Ok(false)
        })
}

fn download(config: &Config, timestamp: u64) -> Result<Image<Box<[u8]>>> {
    let mut source = config.source_spec()?;
    let product = config.product_for(&source);

    let agent = AgentBuilder::new()
        .timeout(TIMEOUT)
        .user_agent("satpaper")
        .build();

    let date = timestamp_date(timestamp)?;
    source.max_zoom = effective_zoom(&agent, config, &source, product, date, timestamp)?;
    let tile_count = source.tile_count();

    let (canvas_dim, tile_size) = download_geometry(config, source);
    ensure!(
        tile_size > 0,
        "requested output is too small for the selected source"
    );

    let tiles = (0..tile_count)
        .flat_map(|x| (0..tile_count).map(move |y| (x, y)))
        .par_bridge()
        .map(|(x, y)| -> Result<_> {
            let url = tile_url(config, &source, product, date, timestamp, x, y);

            log::info!("Scraping tile at ({x}, {y}).");

            let resp = agent
                .get(&url)
                .call()
                .with_context(|| format!("Failed to download SLIDER tile at {url}"))?;

            let len = resp
                .header("Content-Length")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or_default();

            let buf = decode_tile(resp.into_reader())?;
            ensure!(
                buf.width() == source.native_tile_size && buf.height() == source.native_tile_size,
                "SLIDER tile at {url} is {}x{}, expected {}x{}",
                buf.width(),
                buf.height(),
                source.native_tile_size,
                source.native_tile_size,
            );
            let buf = if buf.width() == tile_size && buf.height() == tile_size {
                buf
            } else {
                buf.scale::<Lanczos3>(tile_size, tile_size)
            };

            log::info!(
                "Finished scraping tile at ({x}, {y}). Size: {:.2}KiB",
                len as f32 / 1024.0
            );

            Ok((x, y, buf))
        });

    log::info!("Stitching tiles...");
    let stitched = Mutex::new(Image::alloc(canvas_dim, canvas_dim).boxed());
    tiles.try_for_each(|a| {
        let (y, x, buf) = a?;
        // yes, this is possible lockless.
        // no, i will not do it.
        // if you do it, construct a sendable pointer, then exclusively use .add and slice::from_raw_parts(_mut)
        // SAFETY: tiles iterates over the number of tiles, each tile == tile_size, `stitched` is a image of tile_size * tile_count.
        unsafe {
            stitched
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .overlay_at(&buf, x * tile_size, y * tile_size)
        };
        anyhow::Ok(())
    })?;

    Ok(stitched.into_inner().unwrap())
}

fn effective_zoom(
    agent: &ureq::Agent,
    config: &Config,
    source: &SourceSpec,
    product: &str,
    date: (u16, u8, u8),
    timestamp: u64,
) -> Result<u32> {
    let zoom = find_effective_zoom(source.max_zoom, |zoom| {
        let mut candidate = *source;
        candidate.max_zoom = zoom;
        let url = tile_url(config, &candidate, product, date, timestamp, 0, 0);

        match agent.get(&url).call() {
            Ok(_) => Ok(true),
            Err(ureq::Error::Status(404, _)) => Ok(false),
            Err(err) => Err(err).with_context(|| format!("Failed to probe SLIDER tile at {url}")),
        }
    })?;

    if let Some(zoom) = zoom {
        return Ok(zoom);
    }

    bail!(
        "SLIDER has no tiles for satellite '{}', sector '{}', product '{}' at timestamp {timestamp}",
        config.satellite.id(),
        source.sector.id(),
        product,
    )
}

fn find_effective_zoom(
    max_zoom: u32,
    mut available: impl FnMut(u32) -> Result<bool>,
) -> Result<Option<u32>> {
    for zoom in (0..=max_zoom).rev() {
        if available(zoom)? {
            return Ok(Some(zoom));
        }
    }

    Ok(None)
}

fn decode_tile(mut reader: impl Read) -> Result<Image<Box<[u8]>>> {
    let mut encoded = Vec::new();
    reader.read_to_end(&mut encoded)?;

    let rgba = image::load_from_memory_with_format(&encoded, image::ImageFormat::Png)?.into_rgba8();
    let (width, height) = rgba.dimensions();
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);

    for pixel in rgba.pixels() {
        let alpha = u16::from(pixel[3]);
        rgb.extend(
            pixel.0[..3]
                .iter()
                .map(|value| ((u16::from(*value) * alpha + 127) / 255) as u8),
        );
    }

    Ok(Image::build(width, height).buf(rgb.into_boxed_slice()))
}

fn download_geometry(config: &Config, source: SourceSpec) -> (u32, u32) {
    let tile_count = source.tile_count();

    match source.composition {
        Composition::FullDisk => {
            let canvas_dim = config.disk();
            (canvas_dim, canvas_dim / tile_count)
        }
        Composition::Regional(bounds) => {
            let desired = config
                .resolution_x
                .max((f64::from(config.resolution_y) * bounds.aspect_ratio()).ceil() as u32);
            let tile_size = desired.div_ceil(tile_count);
            (tile_size * tile_count, tile_size)
        }
    }
}

fn tile_url(
    config: &Config,
    source: &SourceSpec,
    product: &str,
    (year, month, day): (u16, u8, u8),
    timestamp: u64,
    x: u32,
    y: u32,
) -> String {
    format!(
        "{SLIDER_BASE_URL}/data/imagery/{year:04}/{month:02}/{day:02}/{}---{}/{product}/{timestamp}/{:02}/{x:03}_{y:03}.png",
        config.satellite.id(),
        source.sector.id(),
        source.max_zoom,
    )
}

fn composite(config: &Config, source: Image<Box<[u8]>>) -> Result<()> {
    log::info!("Compositing...");

    let source_spec = config.source_spec()?;
    let composite = match source_spec.composition {
        Composition::FullDisk => composite_full_disk(config, source)?,
        Composition::Regional(bounds) => {
            if config.background_image.is_some() {
                log::warn!(
                    "Background images are ignored for regional sectors, which fill the output"
                );
            }

            composite_regional(config, source, bounds)
        }
    };

    log::info!("Compositing complete.");

    composite.save(config.target_path.join(OUTPUT_NAME));

    log::info!("Output saved.");

    Ok(())
}

fn composite_full_disk(config: &Config, source: Image<Box<[u8]>>) -> Result<Image<Box<[u8]>>> {
    let disk_dim = config.disk();

    let composite = if let Some(path) = &config.background_image {
        static BG: OnceLock<Image<Box<[u8]>>> = OnceLock::new();

        let mut bg = BG
            .get_or_try_init(|| {
                use image::io::Reader;

                let image = Reader::open(path)
                    .context("Failed to open background image at path {path:?}")?
                    .decode()
                    .context("Failed to load background image - corrupt or unsupported?")?
                    .into_rgb8();

                let mut image = Image::build(image.width(), image.height())
                    .buf(image.into_vec().into_boxed_slice());

                if image.width() != config.resolution_x || image.height() != config.resolution_y {
                    log::info!("Resizing background image to fit...");

                    image = image.scale::<Lanczos3>(config.resolution_x, config.resolution_y);
                }

                anyhow::Ok(image)
            })?
            .clone();

        log::info!("Compositing source into destination...");

        cutout_disk(
            bg.as_mut(),
            source.as_ref(),
            (config.resolution_x - disk_dim) / 2,
            (config.resolution_y - disk_dim) / 2,
        );

        bg
    } else {
        let mut behind = Image::alloc(config.resolution_x, config.resolution_y).boxed();

        unsafe {
            behind.overlay_at(
                &source,
                (config.resolution_x - disk_dim) / 2,
                (config.resolution_y - disk_dim) / 2,
            )
        };

        behind
    };

    Ok(composite)
}

fn composite_regional(
    config: &Config,
    source: Image<Box<[u8]>>,
    bounds: ContentBounds,
) -> Image<Box<[u8]>> {
    let source_width = source.width();
    let source_height = source.height();
    let scale = |value: u32, dimension: u32| {
        ((u64::from(value) * u64::from(dimension)) / u64::from(bounds.denominator)) as u32
    };

    let cropped = crop_image(
        source,
        scale(bounds.left, source_width),
        scale(bounds.top, source_height),
        scale(bounds.right, source_width),
        scale(bounds.bottom, source_height),
    );
    scale_to_cover(cropped, config.resolution_x, config.resolution_y)
}

fn scale_to_cover(
    source: Image<Box<[u8]>>,
    target_width: u32,
    target_height: u32,
) -> Image<Box<[u8]>> {
    let scale = (f64::from(target_width) / f64::from(source.width()))
        .max(f64::from(target_height) / f64::from(source.height()));
    let scaled_width = (f64::from(source.width()) * scale).ceil() as u32;
    let scaled_height = (f64::from(source.height()) * scale).ceil() as u32;
    let scaled = if scaled_width == source.width() && scaled_height == source.height() {
        source
    } else {
        source.scale::<Lanczos3>(scaled_width, scaled_height)
    };
    let offset_x = (scaled_width - target_width) / 2;
    let offset_y = (scaled_height - target_height) / 2;

    crop_image(
        scaled,
        offset_x,
        offset_y,
        offset_x + target_width,
        offset_y + target_height,
    )
}

fn crop_image(
    source: Image<Box<[u8]>>,
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
) -> Image<Box<[u8]>> {
    assert!(left < right && top < bottom);
    assert!(right <= source.width() && bottom <= source.height());

    let mut cropped = Image::alloc(right - left, bottom - top).boxed();
    for x in left..right {
        for y in top..bottom {
            unsafe {
                cropped.set_pixel(x - left, y - top, source.pixel(x, y));
            }
        }
    }

    cropped
}

const BLACK: [u8; 3] = [4; 3];

#[derive(Clone, Copy, Debug)]
enum Direction {
    Left,
    Right,
}

// Identifies the bounds of the Earth in the image
fn cutout_disk(mut bg: Image<&mut [u8]>, earth: Image<&[u8]>, offset_x: u32, offset_y: u32) {
    // Find the midpoint and max of the edges.
    let x_max = earth.width() - 1;
    let y_max = earth.height() - 1;
    let x_center = x_max / 2;
    let y_center = y_max / 2;

    let step = |x: &mut u32, direction: Direction| {
        use Direction::*;

        match direction {
            Left => *x = x.saturating_sub(1),
            Right => *x = x.saturating_add(1),
        }
    };

    // Step linearly through the image pixels until we encounter a non-black pixel,
    // returning its coordinates.
    let march = |mut x: u32, y: u32, direction: Direction| -> u32 {
        log::debug!("Performing cutout march for direction {direction:?}...");

        loop {
            // SAFETY: march
            if unsafe { earth.pixel(x, y) } > BLACK {
                log::debug!("Found disk bounds at {x}, {y}.");
                break x;
            };

            step(&mut x, direction);

            if x == 0 {
                log::debug!("Found disk bounds (min) at {x}, {y}.");
                break x;
            }

            if x > x_max {
                log::debug!("Found disk bounds (max) at {x}, {y}.");
                break x.min(x_max);
            }
        }
    };

    let disk_left = march(0, y_center, Direction::Right);
    let disk_right = march(x_max, y_center, Direction::Left);

    log::debug!("L {disk_left:?} R {disk_right:?}");

    // Approximate the centroid and radius of the circle.
    let radius = (disk_right - disk_left) / 2;

    log::debug!("Radius: {radius} Center X: {x_center} Center Y: {y_center}");

    log::debug!("Starting cutout process...");

    let inside = |x: u32| {
        move |y: u32| {
            ((x_center as i32 - x as i32) * (x_center as i32 - x as i32)
                + (y_center as i32 - y as i32) * (y_center as i32 - y as i32))
                .isqrt()
                < radius as i32
        }
    };

    for x in 0..earth.width() {
        for y in 0..earth.height() {
            if inside(x)(y) {
                // overlay the earth
                unsafe { bg.set_pixel(offset_x + x, offset_y + y, earth.pixel(x, y)) };
            }
        }
    }
}

pub fn fetch_latest_timestamp(config: &Config) -> Result<u64> {
    let source = config.source_spec()?;
    let product = config.product_for(&source);
    Ok(Time::fetch(config, &source, product)?.as_int())
}

#[derive(Debug, Deserialize)]
struct Time {
    #[serde(rename = "timestamps_int")]
    #[serde(deserialize_with = "one")]
    timestamp: u64,
}

fn one<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Visit;
    impl<'de> de::Visitor<'de> for Visit {
        type Value = u64;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "array of u64")
        }

        fn visit_seq<S: de::SeqAccess<'de>>(self, mut seq: S) -> Result<Self::Value, S::Error> {
            let value = seq.next_element()?.ok_or(de::Error::custom("empty seq"))?;

            #[allow(clippy::redundant_pattern_matching)]
            while let Some(_) = seq.next_element::<u64>()? {}

            Ok(value)
        }
    }
    deserializer.deserialize_seq(Visit {})
}

impl Time {
    pub fn fetch(config: &Config, source: &SourceSpec, product: &str) -> Result<Self> {
        let url = metadata_url(config, source, product, "latest_times.json");

        let json = ureq::get(&url)
            .timeout(TIMEOUT)
            .call()
            .with_context(|| invalid_source_context(config, source, product, &url))?
            .into_reader();

        Ok(serde_json::from_reader(json)?)
    }

    pub fn as_int(&self) -> u64 {
        self.timestamp
    }
}

fn timestamp_date(timestamp: u64) -> Result<(u16, u8, u8)> {
    ensure!(
        (10_000_000_000_000..=99_999_999_999_999).contains(&timestamp),
        "invalid SLIDER timestamp {timestamp}: expected YYYYMMDDHHMMSS"
    );

    let date = timestamp / 1_000_000;
    let year = (date / 10_000) as u16;
    let month = ((date / 100) % 100) as u8;
    let day = (date % 100) as u8;

    ensure!(
        (1..=12).contains(&month) && (1..=31).contains(&day),
        "invalid date in SLIDER timestamp {timestamp}"
    );

    Ok((year, month, day))
}

fn metadata_url(config: &Config, source: &SourceSpec, product: &str, document: &str) -> String {
    format!(
        "{SLIDER_BASE_URL}/data/json/{}/{}/{product}/{document}",
        config.satellite.id(),
        source.sector.id(),
    )
}

fn invalid_source_context(
    config: &Config,
    source: &SourceSpec,
    product: &str,
    url: &str,
) -> String {
    format!(
        "SLIDER has no metadata for satellite '{}', sector '{}', product '{}'; verify that combination is available ({url})",
        config.satellite.id(),
        source.sector.id(),
        product,
    )
}

#[test]
#[allow(clippy::inconsistent_digit_grouping)]
fn extracts_date_from_timestamp() {
    assert_eq!(timestamp_date(2023_10_26_12_34_56).unwrap(), (2023, 10, 26));
    assert_eq!(timestamp_date(2027_04_25_00_00_00).unwrap(), (2027, 4, 25));
    assert!(timestamp_date(2027_13_25_00_00_00).is_err());
    assert!(timestamp_date(2027_04_25).is_err());
}

#[cfg(test)]
mod source_tests {
    use std::io::Cursor;
    use std::path::PathBuf;

    use super::*;
    use crate::{Satellite, Sector};

    fn config(sector: Sector) -> Config {
        Config {
            satellite: Satellite::GOESEast,
            resolution_x: 160,
            resolution_y: 90,
            sector,
            product: None,
            disk_size: 95,
            target_path: PathBuf::from("."),
            wallpaper_command: None,
            once: true,
            background_image: None,
        }
    }

    #[test]
    fn constructs_sector_specific_urls() {
        let config = config(Sector::Conus);
        let source = config.source_spec().unwrap();
        let product = config.product_for(&source);

        assert_eq!(
            metadata_url(&config, &source, product, "latest_times.json"),
            "https://rammb-slider.cira.colostate.edu/data/json/goes-19/conus/geocolor/latest_times.json"
        );
        assert_eq!(
            tile_url(
                &config,
                &source,
                product,
                (2026, 7, 16),
                20260716212615,
                8,
                9,
            ),
            "https://rammb-slider.cira.colostate.edu/data/imagery/2026/07/16/goes-19---conus/geocolor/20260716212615/04/008_009.png"
        );
    }

    #[test]
    fn regional_composition_fills_exact_output_dimensions() {
        let config = config(Sector::Conus);
        let mut source = Image::alloc(160, 160).boxed();
        for x in 0..source.width() {
            for y in 0..source.height() {
                unsafe { source.set_pixel(x, y, [255; 3]) };
            }
        }
        let Composition::Regional(bounds) = config.source_spec().unwrap().composition else {
            panic!("CONUS must use regional composition");
        };

        let output = composite_regional(&config, source, bounds);
        assert_eq!(output.width(), 160);
        assert_eq!(output.height(), 90);
        assert_eq!(unsafe { output.pixel(80, 45) }, [255; 3]);
    }

    #[test]
    fn regional_download_geometry_uses_sector_tile_count() {
        let config = config(Sector::Mesoscale01);
        let source = config.source_spec().unwrap();
        let (canvas, tile) = download_geometry(&config, source);

        assert_eq!(source.tile_count(), 4);
        assert_eq!(canvas, tile * 4);
        assert!(canvas >= config.resolution_x);
        assert!(canvas >= config.resolution_y);
    }

    #[test]
    fn product_zoom_adjustment_uses_highest_available_level() {
        let mut probed = Vec::new();
        let zoom = find_effective_zoom(3, |candidate| {
            probed.push(candidate);
            Ok(candidate <= 1)
        })
        .unwrap();

        assert_eq!(zoom, Some(1));
        assert_eq!(probed, [3, 2, 1]);
    }

    #[test]
    fn decodes_indexed_png_tiles_to_rgb() {
        let mut data = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut data, 2, 1);
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_palette(vec![10, 20, 30, 40, 50, 60]);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0, 1]).unwrap();
        }

        let image = decode_tile(Cursor::new(data)).unwrap();
        assert_eq!(image.width(), 2);
        assert_eq!(image.height(), 1);
        assert_eq!(unsafe { image.pixel(0, 0) }, [10, 20, 30]);
        assert_eq!(unsafe { image.pixel(1, 0) }, [40, 50, 60]);
    }

    #[test]
    fn decodes_rgba_png_tiles_over_black() {
        let data = encode_single_pixel_png(png::ColorType::Rgba, &[100, 50, 200, 128]);
        let image = decode_tile(Cursor::new(data)).unwrap();

        assert_eq!(unsafe { image.pixel(0, 0) }, [50, 25, 100]);
    }

    #[test]
    fn decodes_grayscale_png_tiles_to_rgb() {
        let data = encode_single_pixel_png(png::ColorType::Grayscale, &[77]);
        let image = decode_tile(Cursor::new(data)).unwrap();

        assert_eq!(unsafe { image.pixel(0, 0) }, [77, 77, 77]);
    }

    fn encode_single_pixel_png(color: png::ColorType, pixel: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut data, 1, 1);
            encoder.set_color(color);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(pixel).unwrap();
        }
        data
    }
}
