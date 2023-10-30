use std::sync::{PoisonError, OnceLock};
use std::{io::Read, sync::Mutex};
use std::time::Duration;

use anyhow::{Result, Context};
use fimg::{OverlayAt, Image as Img, scale::Lanczos3};
use rayon::prelude::*;
use serde::{Deserialize, de};

use ureq::AgentBuilder;

use super::{
    Config,
    OUTPUT_NAME
};

/// rgb all the way down
pub type Image<T> = Img<T, 3>;

const SLIDER_BASE_URL: &str = "https://rammb-slider.cira.colostate.edu";
const SLIDER_SECTOR: &str = "full_disk";
const SLIDER_PRODUCT: &str = "geocolor";

const TIMEOUT: Duration = Duration::from_secs(30);

pub fn composite_latest_image(config: &Config) -> Result<bool> {
    download(config)
        .and_then(|image| { composite(config, image)?; Ok(true) })
        .or_else(|err| {
            log::error!("Failed to download source image: {err}");
            log::error!("Composition aborted; waiting until next go round.");
            Ok(false)
        })
}

fn download(config: &Config) -> Result<Image<Box<[u8]>>> {
    let tile_count = config.satellite.tile_count();
    let tile_size = config.satellite.tile_size();

    let agent = AgentBuilder::new()
        .timeout(TIMEOUT)
        .user_agent("satpaper")
        .build();

    let time = Time::fetch(config)?;
    let (year, month, day) = Date::fetch(config)?.split();

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
                config.satellite.id(),
                time.as_int(),
                config.satellite.max_zoom()
            );

            log::debug!("Scraping tile at ({x}, {y}).");
            
            let resp = agent
                .get(&url)
                .call()?;

            let len: usize = resp.header("Content-Length")
                .expect("Response header should have Content-Length")
                .parse()?;
            
            let mut buf = vec![0; len];
            let mut read = 0;
            let mut reader = resp.into_reader();
            
            while read < len {
                read += reader.read(&mut buf[read..])?;
            }

            log::debug!(
                "Finished scraping tile at ({x}, {y}). Size: {:.2}KiB",
                len as f32 / 1024.0
            );

            Ok((x, y, buf))
        });
    
    log::info!("Stitching tiles...");
    let stitched = Mutex::new(config.satellite.image());

    tiles.try_for_each(|a|{
        let (y, x, tile) = a?;
        log::debug!("Decoding png ({x} {y})");
        let dec = png::Decoder::new(&*tile);
        let mut reader = dec.read_info()?;
        let mut buf = config.satellite.tile_image();
        let info = reader.next_frame(unsafe { buf.buffer_mut() })?;
        debug_assert!(matches!(info.color_type, png::ColorType::Rgb));
        // yes, this is possible lockless.
        // no, i will not do it.
        // if you do it, construct a sendable pointer, then exclusively use .add and slice::from_raw_parts(_mut)
        // SAFETY: tiles iterates over the number of tiles, each tile == tile_size, `stitched` is a image of tile_size * tile_count.
        unsafe { stitched.lock().unwrap_or_else(PoisonError::into_inner).overlay_at(&buf, x * tile_size, y * tile_size) };        
        anyhow::Ok(())
    })?;

    Ok(stitched.into_inner().unwrap())
}

fn composite(config: &Config, image: Image<Box<[u8]>>) -> Result<()> {
    use std::cmp::Ordering;

    log::info!("Compositing...");
    
    let smaller_dim = match config.resolution_x.cmp(&config.resolution_y) {
        Ordering::Less => config.resolution_x,
        Ordering::Equal => config.resolution_x,
        Ordering::Greater => config.resolution_y,
    };

    let disk_dim = smaller_dim as f32 * (config.disk_size as f32 / 100.0);
    let disk_dim = disk_dim.floor() as u32;
    
    log::info!("Resizing source image...");

    let source = image.scale::<Lanczos3>(disk_dim, disk_dim);

    let composite = if let Some(path) = &config.background_image {
        static BG: OnceLock<Image<Box<[u8]>>> = OnceLock::new();

        let mut bg = BG.get_or_try_init(|| {
            use image::io::Reader;

            let image = Reader::open(path)
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

        cutout_disk(
            bg.as_mut(),
            source.as_ref(),
            (config.resolution_x - disk_dim) / 2,
            (config.resolution_y - disk_dim) / 2
        );

        bg
    }
    else {
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
    offset_x: u32,
    offset_y: u32
) {
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
            if inside(x)(y) {
                // overlay the earth
                unsafe { bg.set_pixel(offset_x + x, offset_y + y, earth.pixel(x, y)) };
            }
        }
    }
}

pub fn fetch_latest_timestamp(config: &Config) -> Result<u64> {
    Ok(Time::fetch(config)?.as_int())   
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
    pub fn fetch(config: &Config) -> Result<Self> {
        let url = format!(
            "{SLIDER_BASE_URL}/data/json/{}/{SLIDER_SECTOR}/{SLIDER_PRODUCT}/latest_times.json",
            config.satellite.id()
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
    pub fn fetch(config: &Config) -> Result<Self> {
        let url = format!(
            "{SLIDER_BASE_URL}/data/json/{}/{SLIDER_SECTOR}/{SLIDER_PRODUCT}/available_dates.json",
            config.satellite.id()
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