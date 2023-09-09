use std::io::Read;

use anyhow::{Result, Context};
use image::*;
use serde::Deserialize;
use ureq::Agent;

use super::{
    Config,
    OUTPUT_NAME
};

const SLIDER_BASE_URL: &str = "https://rammb-slider.cira.colostate.edu";
const SLIDER_SECTOR: &str = "full_disk";
const SLIDER_PRODUCT: &str = "geocolor";

pub fn composite_latest_image(config: &Config) -> Result<bool> {
    download(config)
        .map(|image| {
            composite(config, image)
        })
        .and_then(std::convert::identity)
        .map(|_| true)
        .or_else(|err| {
            log::error!("Failed to download source image: {err}");
            log::error!("Composition aborted; waiting until next go round.");
            Ok(false)
        })
}

fn download(config: &Config) -> Result<DynamicImage> {
    let image_size = config.satellite.image_size() as u32;
    let tile_count = config.satellite.tile_count();
    let tile_size = config.satellite.tile_size();

    let agent = Agent::new();

    let time = Time::fetch(config)?;
    let (year, month, day) = Date::fetch(config)?.split();

    let mut stitched = RgbaImage::new(
        image_size,
        image_size
    );

    let tiles = (0..tile_count)
        .flat_map(|x| {
            (0..tile_count)
                .map(move |y| (x, y))
        })
        .map(|(x, y)| -> Result<_> {
            let url = format!(
                "{SLIDER_BASE_URL}/data/imagery/{year}/{month}/{day}/{}---{SLIDER_SECTOR}/{SLIDER_PRODUCT}/{}/{:02}/{x:03}_{y:03}.png",
                config.satellite.id(),
                time.as_int(),
                config.satellite.max_zoom()
            );

            log::debug!("Scraping tile at ({x}, {y}.)");

            let resp = agent
                .get(&url)
                .call()?;

            let len: usize = resp.header("Content-Length")
                .expect("Response header should have Content-Length")
                .parse()?;

            let mut buf = Vec::with_capacity(len);

            resp
                .into_reader()
                .take(10_000_000)
                .read_to_end(&mut buf)?;

            log::debug!(
                "Finished scraping tile at ({x}, {y}). Size: {}mB",
                len as f32 / 1024.0 / 1024.0
            );

            Ok((x, y, buf))
        });    
    
    log::info!("Stitching tiles...");

    for result in tiles {
        let (x, y, tile) = result?;

        let tile = image::load_from_memory_with_format(
            tile.as_bytes(),
            ImageFormat::Png
        )?;

        imageops::overlay(
            &mut stitched,
            &tile,
            (y * tile_size) as i64,
            (x * tile_size) as i64
        );
    }

    Ok(stitched.into())
}

fn composite(config: &Config, mut image: DynamicImage) -> Result<()> {
    use std::cmp::Ordering::*;
    use image::imageops::FilterType;

    log::info!("Compositing...");

    if config.background_image.is_some() {
        log::info!("Applying transparent background to source image...");
        cutout_disk(&mut image);
    }
    
    let smaller_dim = match config.resolution_x.cmp(&config.resolution_y) {
        Less => config.resolution_x,
        Equal => config.resolution_x,
        Greater => config.resolution_y,
    };

    let disk_dim = smaller_dim as f32 * (config.disk_size as f32 / 100.0);
    let disk_dim = disk_dim.floor() as u32;
    
    log::info!("Resizing source image...");

    let source = imageops::resize(
        &image,
        disk_dim,
        disk_dim,
        FilterType::Lanczos3
    );

    log::info!("Generating destination image...");

    let mut destination;

    if let Some(path) = &config.background_image {        
        let image = std::fs::read(path)
            .context("Failed to read background image from path {path:?}")?;

        let mut image = image::load_from_memory(&image)
            .context("Failed to load background image - corrupt or unsupported?")?;

        if image.dimensions().0 != config.resolution_x || 
           image.dimensions().1 != config.resolution_y 
        {
            log::info!("Resizing background image to fit...");

            image = imageops::resize(
                &image,
                config.resolution_x,
                config.resolution_y,
                FilterType::Lanczos3
            ).into()
        }

        destination = image;
    } 
    else {
        let mut default = ImageBuffer::new(
            config.resolution_x,
            config.resolution_y
        );
    
        for pixel in default.pixels_mut() {
            *pixel = Rgba([u8::MIN, u8::MIN, u8::MIN, u8::MAX])
        }

        destination = default.into();
    }

    log::info!("Compositing source into destination...");

    imageops::overlay(
        &mut destination,
        &source,
        ((config.resolution_x - disk_dim) / 2) as i64,
        ((config.resolution_y - disk_dim) / 2) as i64
    );

    log::info!("Compositing complete.");

    destination.save(
        config.target_path.join(OUTPUT_NAME)
    )?;

    log::info!("Output saved.");

    Ok(())
}

const BLACK: Rgba<u8> = Rgba([0, 0, 0, 255]);

#[derive(Clone, Copy, Debug)]
enum Direction {
    Up,
    Down,
    Left,
    Right
}

// Identifies the bounds of the Earth in the image
fn cutout_disk(image: &mut DynamicImage) {
    // Find the midpoint and max of the edges.
    let x_max = image.dimensions().0;
    let y_max = image.dimensions().1;
    let x_center = x_max / 2;
    let y_center = y_max / 2;

    let step = |x: &mut u32, y: &mut u32, direction: Direction| {
        use Direction::*;

        match direction {
            Up => y.saturating_add(1),
            Down => y.saturating_sub(1),
            Left => x.saturating_sub(1),
            Right => x.saturating_add(1),
        }
    };

    // Step linearly through the image pixels until we encounter a non-black pixel,
    // returning its coordinates.
    let march = |mut x: u32, mut y: u32, direction: Direction| -> (u32, u32) {        
        log::debug!("Performing cutout march for direction {direction:?}...");

        loop {
            if x >= x_max || y >= y_max {
                log::debug!("Found disk bounds (max) at {x}, {y}.");
                return (x, y);
            }

            if image.get_pixel(x, y) != BLACK {
                log::debug!("Found disk bounds at {x}, {y}.");
                return (x, y)
            };

            step(&mut x, &mut y, direction);
        }
    };

    let disk_bottom = march(x_center, y_max, Direction::Up);
    let disk_top = march(x_center, 0, Direction::Down);
    let disk_left = march(0, y_center, Direction::Right);
    let disk_right = march(x_max, y_center, Direction::Left);

    // Approximate the centroid and radius of the circle.
    let radius = (disk_right.0 - disk_left.0) + (disk_bottom.1 - disk_top.1);
    let radius = radius / 4;

    let x_center = (disk_bottom.0 + disk_top.0 + disk_left.0 + disk_right.0) / 4;
    let y_center = (disk_bottom.1 + disk_top.1 + disk_left.1 + disk_right.1) / 4;

    log::debug!("Starting cutout process...");

    // HOLD ONTO YO CPU CORES
    image
        .as_mut_rgba8()
        .unwrap()
        .enumerate_pixels_mut()
        .filter(|(x, y, _)| {
            // Compute the distance between the pixel and the circle's center.
            // If it's greater than the radius, it's outside the circle.
            let x_component = (x - x_center).pow(2);
            let y_component = (y - y_center).pow(2);

            let root = (x_component + y_component) as f32;
            let root = root.sqrt().floor() as u32;

            root > radius
        })
        .for_each(|(_, _, pixel)| {
            // Make the pixel transparent.
            pixel.0 = [0; 4];
        });
}

pub fn fetch_latest_timestamp(config: &Config) -> Result<u64> {
    Ok(Time::fetch(config)?.as_int())   
}

#[derive(Debug, Deserialize)]
struct Time {
    #[serde(rename = "timestamps_int")]
    timestamps: Vec<u64>
}

impl Time {
    pub fn fetch(config: &Config) -> Result<Self> {
        let url = format!(
            "{SLIDER_BASE_URL}/data/json/{}/{SLIDER_SECTOR}/{SLIDER_PRODUCT}/latest_times.json",
            config.satellite.id()
        );
        
        let json = ureq::get(&url)
            .call()?
            .into_string()?;

        let mut new: Self = serde_json::from_str(&json)?;

        new.timestamps.drain(1..);
        new.timestamps.shrink_to_fit();

        Ok(new)
    }

    pub fn as_int(&self) -> u64 {
        *self
            .timestamps
            .first()
            .expect("At least one timestamp should exist")
    }
}

#[derive(Debug, Deserialize)]
struct Date {
    #[serde(rename = "dates_int")]
    dates: Vec<u64>
}

impl Date {
    pub fn fetch(config: &Config) -> Result<Self> {
        let url = format!(
            "{SLIDER_BASE_URL}/data/json/{}/{SLIDER_SECTOR}/{SLIDER_PRODUCT}/available_dates.json",
            config.satellite.id()
        );

        let json = ureq::get(&url)
            .call()?
            .into_string()?;

        let mut new: Self= serde_json::from_str(&json)?;

        new.dates.drain(1..);
        new.dates.shrink_to_fit();

        Ok(new)
    }

    pub fn as_int(&self) -> u64 {
        *self
            .dates
            .first()
            .expect("At least one timestamp should exist")
    }

    pub fn split(&self) -> (String, String, String) {
        let str = format!(
            "{}", 
            self.as_int()
        );

        (
            str[0..=3].to_owned(),
            str[4..=5].to_owned(),
            str[6..=7].to_owned()
        )
    }
}