use anyhow::Result;
use serde::Deserialize;

use super::Config;

type Image = image::ImageBuffer<image::Rgba<u8>, Vec<u8>>;

const SLIDER_BASE_URL: &str = "https://rammb-slider.cira.colostate.edu";
const SLIDER_SECTOR: &str = "full_disk";
const SLIDER_PRODUCT: &str = "geocolor";

pub async fn fetch_latest_image(config: &Config) -> Result<Image> {
    use image::imageops;
    use image::RgbaImage;
    use image::ImageFormat;
    use image::EncodableLayout;
    use reqwest::Client;

    let image_size = config.satellite.image_size() as u32;
    let tile_count = config.satellite.tile_count();
    let tile_size = config.satellite.tile_size();

    let client = Client::new();
    let time = Time::fetch(config).await?;
    let (year, month, day) = Date::fetch(config).await?.split();

    let mut stitched = RgbaImage::new(
        image_size,
        image_size
    );

    let tiles = (0..tile_count)
        .flat_map(|x| {
            (0..tile_count)
                .map(move |y| (x, y))
        })
        .map(|(x, y)| {
            let url = format!(
                "{SLIDER_BASE_URL}/data/imagery/{year}/{month}/{day}/{}---{SLIDER_SECTOR}/{SLIDER_PRODUCT}/{}/{:02}/{x:03}_{y:03}.png",
                config.satellite.id(),
                time.as_int(),
                config.satellite.max_zoom()
            );

            let client = client.clone();

            let bytes_fut = tokio::task::spawn(async move {
                let request = client.get(url).build()?;

                client
                    .execute(request)
                    .await?
                    .bytes()
                    .await
            });

            (x, y, bytes_fut)
        });

    for (x, y, fut) in tiles {
        let tile = image::load_from_memory_with_format(
            fut.await??.as_bytes(),
            ImageFormat::Png
        )?;

        imageops::overlay(
            &mut stitched,
            &tile,
            (y * tile_size) as i64,
            (x * tile_size) as i64
        );
    }
    
    Ok(stitched)
}

pub async fn fetch_latest_timestamp(config: &Config) -> Result<u64> {
    Ok(Time::fetch(config)
        .await?
        .as_int()
    )   
}

#[derive(Debug, Deserialize)]
struct Time {
    #[serde(rename = "timestamps_int")]
    timestamps: Vec<u64>
}

impl Time {
    pub async fn fetch(config: &Config) -> Result<Self> {
        let url = format!(
            "{SLIDER_BASE_URL}/data/json/{}/{SLIDER_SECTOR}/{SLIDER_PRODUCT}/latest_times.json",
            config.satellite.id()
        );
        
        let json = reqwest::get(url)
            .await?
            .text()
            .await?;

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
    pub async fn fetch(config: &Config) -> Result<Self> {
        let url = format!(
            "{SLIDER_BASE_URL}/data/json/{}/{SLIDER_SECTOR}/{SLIDER_PRODUCT}/available_dates.json",
            config.satellite.id()
        );

        let json = reqwest::get(url)
            .await?
            .text()
            .await?;

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

        let chars: Vec<_> = str
            .chars()
            .collect();

        (
            chars[0..=3].iter().collect(),
            chars[4..=5].iter().collect(),
            chars[6..=7].iter().collect()
        )
    }
}