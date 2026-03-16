use std::io::Read;
use std::sync::{PoisonError, OnceLock, Mutex};
use std::time::Duration;

use anyhow::{Result, Context};
use fimg::{OverlayAt, Image as Img, scale::Lanczos3};
use rayon::prelude::*;
use serde::{Deserialize, de};
use std::collections::HashMap;

use ureq::AgentBuilder;

use super::{
    Config,
    OUTPUT_NAME
};
use crate::config::Satellite;

/// rgb all the way down
pub type Image<T> = Img<T, 3>;

const SLIDER_BASE_URL: &str = "https://rammb-slider.cira.colostate.edu";
const SLIDER_SECTOR: &str = "full_disk";
const SLIDER_PRODUCT: &str = "geocolor";

const TIMEOUT: Duration = Duration::from_secs(30);

pub fn composite_latest_image(config: &Config) -> Result<()> {
    let image = download(config)?;
    composite(config, image)
}

/// Download the latest full-disk image for a satellite at a fixed 2048×2048 resolution.
///
/// This is the standalone entry point — no `Config` needed.
pub fn download_disk(satellite: Satellite) -> Result<Image<Box<[u8]>>> {
    download_disk_inner(satellite, 2048)
}

pub fn download(config: &Config) -> Result<Image<Box<[u8]>>> {
    let satellite = config.satellite.expect("download requires satellite to be set");
    let disk = config.disk().expect("download requires disk_size to be set");
    download_disk_inner(satellite, disk)
}

fn download_disk_inner(satellite: Satellite, disk_dim: u32) -> Result<Image<Box<[u8]>>> {
    let tile_count = satellite.tile_count();

    let agent = AgentBuilder::new()
        .timeout(TIMEOUT)
        .user_agent("spacepaper")
        .build();

    let time = Time::fetch(satellite)?;
    let (year, month, day) = Date::fetch(satellite)?.split();

    let tile_size = disk_dim / tile_count;

    let tiles = (0..tile_count)
        .flat_map(|x| {
            (0..tile_count)
                .map(move |y| (x, y))
        })
        .par_bridge()
        .map(|(x, y)| -> Result<_> {
            // year:04 i am hilarious
            let url = format!(
                "{SLIDER_BASE_URL}/data/imagery/{year:04}/{month:02}/{day:02}/{}---{SLIDER_SECTOR}/{SLIDER_PRODUCT}/{}/{:02}/{x:03}_{y:03}.png",
                satellite.id(),
                time.as_int(),
                satellite.max_zoom()
            );

            log::info!("Scraping tile at ({x}, {y}).");

            let resp = agent
                .get(&url)
                .call()?;

            let len: usize = resp.header("Content-Length")
                .expect("Response header should have Content-Length")
                .parse()?;

            let mut data = Vec::with_capacity(len);
            resp.into_reader().read_to_end(&mut data)?;
            let dec = png::Decoder::new(std::io::Cursor::new(data));
            let mut reader = dec.read_info()?;
            let mut buf = satellite.tile_image();
            let info = reader.next_frame(unsafe { buf.buffer_mut() })?;
            debug_assert!(matches!(info.color_type, png::ColorType::Rgb));
            let buf = buf.scale::<Lanczos3>(tile_size, tile_size);

            log::info!(
                "Finished scraping tile at ({x}, {y}). Size: {:.2}KiB",
                len as f32 / 1024.0
            );

            Ok((x, y, buf))
        });

    log::info!("Stitching tiles...");
    let stitched = Mutex::new(Image::alloc(disk_dim, disk_dim).boxed());
    tiles.try_for_each(|a|{
        let (y, x, buf) = a?;
        // yes, this is possible lockless.
        // no, i will not do it.
        // if you do it, construct a sendable pointer, then exclusively use .add and slice::from_raw_parts(_mut)
        // SAFETY: tiles iterates over the number of tiles, each tile == tile_size, `stitched` is a image of tile_size * tile_count.
        unsafe { stitched.lock().unwrap_or_else(PoisonError::into_inner).overlay_at(&buf, x * tile_size, y * tile_size) };
        anyhow::Ok(())
    })?;

    Ok(stitched.into_inner().unwrap())
}

fn composite(config: &Config, source: Image<Box<[u8]>>) -> Result<()> {
    log::info!("Compositing...");

    let composite = if let Some(path) = &config.background_image {
        static BG: OnceLock<Image<Box<[u8]>>> = OnceLock::new();

        let mut bg = BG.get_or_try_init(|| {
            use image::ImageReader;

            let image = ImageReader::open(path)
                .context("Failed to open background image at path {path:?}")?
                .decode()
                .context("Failed to load background image - corrupt or unsupported?")?
                .into_rgb8();

            let mut image = Image::build(image.width(), image.height()).buf(image.into_vec().into_boxed_slice());

            if image.width() != config.resolution_x || 
               image.height() != config.resolution_y 
            {
                log::info!("Resizing background image to fit...");

                image = image.scale::<Lanczos3>(config.resolution_x, config.resolution_y);
            }

            anyhow::Ok(image)
        })?.clone();

        log::info!("Compositing source into destination...");

        let (offset_x, offset_y) = config.disk_offset().expect("composite requires disk_size to be set");

        cutout_disk(
            bg.as_mut(),
            source.as_ref(),
            offset_x,
            offset_y
        );

        bg
    }
    else {
        let mut behind = Image::alloc(config.resolution_x, config.resolution_y).boxed();

        let (offset_x, offset_y) = config.disk_offset().expect("composite requires disk_size to be set");
        let bg_w = config.resolution_x as i32;
        let bg_h = config.resolution_y as i32;

        for x in 0..source.width() {
            for y in 0..source.height() {
                let bx = offset_x + x as i32;
                let by = offset_y + y as i32;
                if bx >= 0 && bx < bg_w && by >= 0 && by < bg_h {
                    unsafe { behind.set_pixel(bx as u32, by as u32, source.pixel(x, y)) };
                }
            }
        }

        behind
    };
    
    log::info!("Compositing complete.");

    composite.save(
        config.target_path.join(OUTPUT_NAME)
    );

    log::info!("Output saved.");

    Ok(())
}

const BLACK: [u8; 3] = [4; 3];

#[derive(Clone, Copy, Debug)]
enum Direction {
    Left,
    Right
}

// Identifies the bounds of the Earth in the image
fn cutout_disk(
    mut bg: Image<&mut [u8]>,
    earth: Image<&[u8]>,
    offset_x: i32,
    offset_y: i32
) {
    let bg_w = bg.width() as i32;
    let bg_h = bg.height() as i32;
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
            if *unsafe { earth.pixel(x, y) } > BLACK {
                log::debug!("Found disk bounds at {x}, {y}.");
                break x
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

    let inside = |x: u32| move |y: u32| {
        ((x_center as i32 - x as i32) * (x_center as i32 - x as i32) + (y_center as i32 - y as i32) * (y_center as i32 - y as i32)).isqrt() < radius as i32
    };

    for x in 0..earth.width() {
        for y in 0..earth.height() {
            let bx = offset_x + x as i32;
            let by = offset_y + y as i32;
            if bx >= 0 && bx < bg_w && by >= 0 && by < bg_h && inside(x)(y) {
                unsafe { bg.set_pixel(bx as u32, by as u32, earth.pixel(x, y)) };
            }
        }
    }
}

/// Download the latest full-disk image for a satellite, or historical imagery
/// if `utc_datetime` is provided (format: `YYYY-MM-DDTHH:MM`).
pub fn download_disk_for(satellite: Satellite, utc_datetime: Option<&str>) -> Result<Image<Box<[u8]>>> {
    match utc_datetime {
        Some(dt) => {
            let timestamp = fetch_closest_timestamp(satellite, dt)?;
            download_disk_at(satellite, timestamp)
        }
        None => download_disk(satellite),
    }
}

/// Find the SLIDER timestamp closest to the requested UTC datetime.
///
/// Fetches `{date}_by_hour.json` and picks the timestamp nearest to the target.
pub fn fetch_closest_timestamp(satellite: Satellite, utc_datetime: &str) -> Result<u64> {
    // Parse "YYYY-MM-DDTHH:MM" -> date string "YYYYMMDD" and target minutes-since-midnight
    anyhow::ensure!(utc_datetime.len() >= 16, "datetime must be YYYY-MM-DDTHH:MM");
    let date_str = format!(
        "{}{}{}",
        &utc_datetime[0..4],
        &utc_datetime[5..7],
        &utc_datetime[8..10]
    );
    let target_hour: u32 = utc_datetime[11..13].parse()?;
    let target_min: u32 = utc_datetime[14..16].parse()?;
    let target_minutes = target_hour * 60 + target_min;

    let url = format!(
        "{SLIDER_BASE_URL}/data/json/{}/{SLIDER_SECTOR}/{SLIDER_PRODUCT}/{date_str}_by_hour.json",
        satellite.id()
    );

    let json: String = ureq::get(&url)
        .timeout(TIMEOUT)
        .call()?
        .into_string()?;

    let parsed: ByHour = serde_json::from_str(&json)?;

    // Flatten all timestamps from all hours
    let mut best: Option<u64> = None;
    let mut best_diff: u32 = u32::MAX;

    for (_hour_key, timestamps) in &parsed.timestamps_int {
        for &ts in timestamps {
            // Timestamp format: YYYYMMDDHHmmSS — extract HH and mm
            let ts_hour = ((ts / 10000) % 100) as u32;
            let ts_min = ((ts / 100) % 100) as u32;
            let ts_minutes = ts_hour * 60 + ts_min;
            let diff = ts_minutes.abs_diff(target_minutes);
            if diff < best_diff {
                best_diff = diff;
                best = Some(ts);
            }
        }
    }

    best.context("No timestamps found for the requested date")
}

/// Download a full-disk image using a specific SLIDER timestamp.
fn download_disk_at(satellite: Satellite, timestamp: u64) -> Result<Image<Box<[u8]>>> {
    // Extract date components from timestamp (YYYYMMDDHHmmSS)
    let year = (timestamp / 10_000_000_000) as u16;
    let month = ((timestamp / 100_000_000) % 100) as u8;
    let day = ((timestamp / 1_000_000) % 100) as u8;

    let disk_dim: u32 = 2048;
    let tile_count = satellite.tile_count();
    let tile_size = disk_dim / tile_count;

    let agent = AgentBuilder::new()
        .timeout(TIMEOUT)
        .user_agent("spacepaper")
        .build();

    let tiles = (0..tile_count)
        .flat_map(|x| (0..tile_count).map(move |y| (x, y)))
        .par_bridge()
        .map(|(x, y)| -> Result<_> {
            let url = format!(
                "{SLIDER_BASE_URL}/data/imagery/{year:04}/{month:02}/{day:02}/{}---{SLIDER_SECTOR}/{SLIDER_PRODUCT}/{timestamp}/{:02}/{x:03}_{y:03}.png",
                satellite.id(),
                satellite.max_zoom()
            );

            log::info!("Scraping tile at ({x}, {y}).");

            let resp = agent.get(&url).call()?;
            let len: usize = resp
                .header("Content-Length")
                .expect("Response header should have Content-Length")
                .parse()?;

            let mut data = Vec::with_capacity(len);
            resp.into_reader().read_to_end(&mut data)?;
            let dec = png::Decoder::new(std::io::Cursor::new(data));
            let mut reader = dec.read_info()?;
            let mut buf = satellite.tile_image();
            let info = reader.next_frame(unsafe { buf.buffer_mut() })?;
            debug_assert!(matches!(info.color_type, png::ColorType::Rgb));
            let buf = buf.scale::<Lanczos3>(tile_size, tile_size);

            log::info!(
                "Finished scraping tile at ({x}, {y}). Size: {:.2}KiB",
                len as f32 / 1024.0
            );

            Ok((x, y, buf))
        });

    log::info!("Stitching tiles...");
    let stitched = Mutex::new(Image::alloc(disk_dim, disk_dim).boxed());
    tiles.try_for_each(|a| {
        let (y, x, buf) = a?;
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

#[derive(Debug, Deserialize)]
struct ByHour {
    timestamps_int: HashMap<String, Vec<u64>>,
}

pub fn fetch_latest_timestamp(satellite: Satellite) -> Result<u64> {
    Ok(Time::fetch(satellite)?.as_int())
}

#[derive(Debug, Deserialize)]
struct Time {
    #[serde(rename = "timestamps_int")]
    #[serde(deserialize_with = "one")]
    timestamp: u64
}

fn one<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de> {
    struct Visit;
    impl<'de> de::Visitor<'de> for Visit {
        type Value = u64;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "array of u64")
        }

        fn visit_seq<S: de::SeqAccess<'de>>(self, mut seq: S) -> Result<Self::Value, S::Error> {    
            let value = seq.next_element()?
                .ok_or(de::Error::custom("empty seq"))?;
            
            #[allow(clippy::redundant_pattern_matching)]
            while let Some(_) = seq.next_element::<u64>()? {}

            Ok(value)
        }
    }
    deserializer.deserialize_seq(Visit {})
}


impl Time {
    pub fn fetch(satellite: Satellite) -> Result<Self> {
        let url = format!(
            "{SLIDER_BASE_URL}/data/json/{}/{SLIDER_SECTOR}/{SLIDER_PRODUCT}/latest_times.json",
            satellite.id()
        );
        
        let json = ureq::get(&url)
            .timeout(TIMEOUT)
            .call()?
            .into_reader();

        Ok(serde_json::from_reader(json)?)
    }

    pub fn as_int(&self) -> u64 {
        self.timestamp
    }
}

#[derive(Debug, Deserialize)]
struct Date {
    #[serde(rename = "dates_int")]
    #[serde(deserialize_with = "one")]
    date: u64
}

impl Date {
    pub fn fetch(satellite: Satellite) -> Result<Self> {
        let url = format!(
            "{SLIDER_BASE_URL}/data/json/{}/{SLIDER_SECTOR}/{SLIDER_PRODUCT}/available_dates.json",
            satellite.id()
        );

        let json = ureq::get(&url)
            .timeout(TIMEOUT)
            .call()?
            .into_reader();

        Ok(serde_json::from_reader(json)?)
    }

    /// Splits date into year, month, and day
    pub fn split(&self) -> (u16, u8, u8) {
        let dig = |n: u8| ((self.date / 10u64.pow(u32::from(n))) % 10) as u8;
        (
            (u16::from(dig(7)) * 1000) + (u16::from(dig(6)) * 100) + (u16::from(dig(5)) * 10) + u16::from(dig(4)), // yyyy
            (dig(3) * 10) + dig(2), // mm
            (dig(1) * 10) + dig(0), // dd
        )
    }
}

#[test]
#[allow(clippy::inconsistent_digit_grouping)]
fn test_date_split() {
    assert_eq!(Date { date: 2023_10_26 }.split(), (2023, 10, 26));
    assert_eq!(Date { date: 2027_04_25 }.split(), (2027, 4, 25));
}