use std::io::Read;
use std::time::Duration;

use anyhow::{Result, Context};
use fimg::{OverlayAt, Image};
use image::*;
use serde::Deserialize;

use ureq::AgentBuilder;

use super::{
    Config,
    OUTPUT_NAME
};

const SLIDER_BASE_URL: &str = "https://rammb-slider.cira.colostate.edu";
const SLIDER_SECTOR: &str = "full_disk";
const SLIDER_PRODUCT: &str = "geocolor";

const TIMEOUT: Duration = Duration::from_secs(10);

pub fn composite_latest_image(config: &Config) -> Result<bool> {
    download(config)
        .and_then(|image| composite(config, image))
        .map(|_| true)
        .or_else(|err| {
            log::error!("Failed to download source image: {err}");
            log::error!("Composition aborted; waiting until next go round.");
            Ok(false)
        })
}

fn download(config: &Config) -> Result<Image<Box<[u8]>, 3>> {
    let image_size = config.satellite.image_size() as u32;
    let tile_count = config.satellite.tile_count();
    let tile_size = config.satellite.tile_size() as u32;

    let agent = AgentBuilder::new()
        .timeout(TIMEOUT)
        .build();

    let time = Time::fetch(config)?;
    let (year, month, day) = Date::fetch(config)?.split();

    let tiles = (0..tile_count)
        .flat_map(|x| {
            (0..tile_count)
                .map(move |y| (x as u32, y as u32))
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

    let mut stitched: Image<_, 3> = fimg::Image::alloc(image_size, image_size).boxed();

    for result in tiles {
        let (y, x, tile) = result?;
        let dec = png::Decoder::new(&*tile);
        let mut reader = dec.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert!(matches!(info.color_type, png::ColorType::Rgb));
        debug_assert!(info.width == tile_size && info.height == tile_size);
        let img = fimg::Image::<_, 3>::build(info.width, info.height).buf(buf).boxed();
        assert!(x * tile_size < image_size);
        assert!(y * tile_size < image_size);
        // SAFETY: bounds checked above
        unsafe { stitched.overlay_at(&img, x * tile_size, y * tile_size) };
    }

    Ok(stitched)
}

fn composite(config: &Config, image: Image<Box<[u8]>, 3>) -> Result<()> {
    use std::cmp::Ordering::*;
    use image::imageops::FilterType;

    log::info!("Compositing...");
    
    let smaller_dim = match config.resolution_x.cmp(&config.resolution_y) {
        Less => config.resolution_x,
        Equal => config.resolution_x,
        Greater => config.resolution_y,
    };

    let disk_dim = smaller_dim as f32 * (config.disk_size as f32 / 100.0);
    let disk_dim = disk_dim.floor() as u32;
    
    log::info!("Resizing source image...");

    let mut source = fr::Image::new(
        disk_dim.try_into().unwrap(),
        disk_dim.try_into().unwrap(),
        fr::PixelType::U8x3
    );

    fr::Resizer::new(fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3))
        .resize(&fr::Image::from_vec_u8(
            // width and height are internally stored as NonZeroU32 anyway
            image.width().try_into().unwrap(),
            image.height().try_into().unwrap(),
            image.take_buffer().into_vec(),
            fr::PixelType::U8x3
        ).unwrap().view(), &mut source.view_mut()).unwrap();
    
    let mut source = fimg::Image::build(source.width().get(), source.height().get()).buf(source.into_vec()).boxed();

    log::info!("Generating destination image...");

    let mut destination;

    if let Some(path) = &config.background_image {        
        use image::io::Reader;

        let mut image = Reader::open(path)
            .context("Failed to open background image at path {path:?}")?
            .decode()
            .context("Failed to load background image - corrupt or unsupported?")?;

        if image.dimensions().0 != config.resolution_x || 
           image.dimensions().1 != config.resolution_y 
        {
            log::info!("Resizing background image to fit...");

            // why is this not cached?
            image = imageops::resize(
                &image,
                config.resolution_x,
                config.resolution_y,
                FilterType::Lanczos3
            ).into();
        }

        log::info!("Applying transparent background to source image...");

        cutout_disk(&mut source);

        let image = image.into_rgb8();
            
        destination = fimg::Image::build(image.width(), image.height()).buf(image.into_vec()).boxed();
    } else {
        destination = Image::alloc(config.resolution_x, config.resolution_y).boxed();
    }

    log::info!("Compositing source into destination...");

    unsafe { destination.overlay_at(
        &source,
        ((config.resolution_x - disk_dim) / 2) as u32,
        ((config.resolution_y - disk_dim) / 2) as u32,
    ) };

    log::info!("Compositing complete.");

    destination.save(
        config.target_path.join(OUTPUT_NAME)
    );

    log::info!("Output saved.");

    Ok(())
}

const CLEAR: [u8; 3] = [0; 3];
const BLACK: [u8; 3] = [4; 3];

#[derive(Clone, Copy, Debug)]
enum Direction {
    Up,
    Down,
    Left,
    Right
}

// Identifies the bounds of the Earth in the image
fn cutout_disk(image: &mut Image<Box<[u8]>, 3>) {
    // Find the midpoint and max of the edges.
    let x_max = image.width() - 1;
    let y_max = image.height() - 1;
    let x_center = x_max / 2;
    let y_center = y_max / 2;

    let step = |x: &mut u32, y: &mut u32, direction: Direction| {
        use Direction::*;

        match direction {
            Up => *y = y.saturating_sub(1),
            Down => *y = y.saturating_add(1),
            Left => *x = x.saturating_sub(1),
            Right => *x = x.saturating_add(1),
        }
    };

    // Step linearly through the image pixels until we encounter a non-black pixel,
    // returning its coordinates.
    let march = |mut x: u32, mut y: u32, direction: Direction| -> (u32, u32) {        
        log::debug!("Performing cutout march for direction {direction:?}...");

        loop {
            assert!(x < image.width());
            assert!(y < image.height());
            // SAFETY: bounds are checked ^
            if unsafe { image.pixel(x, y) } > BLACK {
                log::debug!("Found disk bounds at {x}, {y}.");
                break (x, y)
            };

            step(&mut x, &mut y, direction);

            if y == 0 || x == 0 {
                log::debug!("Found disk bounds (min) at {x}, {y}.");
                break (x, y);
            }

            if x > x_max || y > y_max {
                log::debug!("Found disk bounds (max) at {x}, {y}.");
                break (
                    x.min(x_max),
                    y.min(y_max)
                );
            }
        }
    };

    let disk_bottom = march(x_center, y_max, Direction::Up);
    let disk_top = march(x_center, 0, Direction::Down);
    let disk_left = march(0, y_center, Direction::Right);
    let disk_right = march(x_max, y_center, Direction::Left);

    log::debug!("B {disk_bottom:?} T {disk_top:?} L {disk_left:?} R {disk_right:?}");

    // Approximate the centroid and radius of the circle.
    let radius = (disk_right.0 - disk_left.0) + (disk_bottom.1 - disk_top.1);
    let radius = radius / 4;

    log::debug!("Radius: {radius} Center X: {x_center} Center Y: {y_center}");

    log::debug!("Starting cutout process...");

    // HOLD ONTO YO CPU CORES
    for x in 0..image.width() {
        for y in 0..image.height() {
            let x_component = (x - x_center).pow(2);
            let y_component = (y - y_center).pow(2);

            let root = (x_component + y_component) as f32;
            let root = root.sqrt().floor() as u32;
            
            if root < radius {
                continue;
            }

            // these checks get optimized out
            assert!(x < image.width());
            assert!(y < image.height());
            // SAFETY: literally iterating over bounds (also there are checks)
            unsafe { image.set_pixel(x, y, CLEAR) };
        }
    }
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
            .timeout(TIMEOUT)
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
            .timeout(TIMEOUT)
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
