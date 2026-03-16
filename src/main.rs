#![feature(once_cell_try)]

mod compositor;
mod config;
mod moon;
mod orbital;
mod slider;
mod stars;
mod sun;
mod wallpaper;

use anyhow::{Result, Context};
use chrono::Utc;
use clap::Parser;
use fimg::Image;
use fimg::scale::Lanczos3;

use crate::config::*;

const OUTPUT_NAME: &str = "spacepaper_latest.png";

fn main() -> Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    env_logger::init();

    let config = Config::parse();
    std::fs::create_dir_all(&config.target_path)
        .context("Failed to create target directory")?;
    let historical = config.datetime.is_some();
    let datetime = config.datetime.clone()
        .unwrap_or_else(|| Utc::now().format("%Y-%m-%dT%H:%M").to_string());

    log::info!("Selecting optimal satellite for {datetime}{}...", if historical { " (historical)" } else { "" });

    let data = orbital::closest_satellite(&datetime, None)
        .context("Failed to select satellite")?;
    let winner = data.winner;

    log::info!("Selected satellite: {:?}", winner);

    // Skip timestamp cache check for historical mode — always generate.
    if !historical {
        log::debug!("Checking timestamp...");

        let new = slider::fetch_latest_timestamp(winner)
            .context("Failed to fetch latest timestamp")?;

        let cache_path = format!("/tmp/spacepaper_{}_timestamp", winner.id());

        if let Ok(cached) = std::fs::read_to_string(&cache_path) {
            if cached.trim() == new.to_string() {
                log::info!("Up to date (timestamp {new}).");
                return Ok(());
            }
        }

        log::info!("Timestamp has changed, fetching updated source and compositing new wallpaper...");

        let background = load_background(&config)?;

        let image = compositor::compose(
            &datetime,
            config.resolution_x,
            config.resolution_y,
            background.as_ref(),
            config.star_chart,
            config.star_chart_bin.as_deref(),
        ).context("Failed to compose wallpaper")?;

        image.save(config.target_path.join(OUTPUT_NAME));

        std::fs::write(&cache_path, new.to_string())
            .context("Failed to write timestamp cache")?;
    } else {
        log::info!("Historical mode — fetching imagery for {datetime}...");

        let background = load_background(&config)?;

        let image = compositor::compose_historical(
            &datetime,
            config.resolution_x,
            config.resolution_y,
            background.as_ref(),
            config.star_chart,
            config.star_chart_bin.as_deref(),
        ).context("Failed to compose wallpaper")?;

        image.save(config.target_path.join(OUTPUT_NAME));
    }

    log::info!("New wallpaper composited and saved.");

    Ok(())
}

fn load_background(config: &Config) -> Result<Option<Image<Box<[u8]>, 3>>> {
    config.background_image.as_ref().map(|path| {
        use image::ImageReader;

        let img = ImageReader::open(path)
            .context("Failed to open background image")?
            .decode()
            .context("Failed to decode background image")?
            .into_rgb8();

        let mut img: Image<Box<[u8]>, 3> = Image::build(img.width(), img.height()).buf(img.into_vec().into_boxed_slice());

        if img.width() != config.resolution_x || img.height() != config.resolution_y {
            log::info!("Resizing background image to fit...");
            img = img.scale::<Lanczos3>(config.resolution_x, config.resolution_y);
        }

        anyhow::Ok(img)
    }).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_wallpaper() -> Result<()> {
        let datetime = Utc::now().format("%Y-%m-%dT%H:%M").to_string();

        let image = compositor::compose(
            &datetime,
            2556,
            1440,
            None,
            false,
            None,
        )?;

        image.save("./spacepaper_latest.png");
        std::fs::remove_file("./spacepaper_latest.png")?;

        Ok(())
    }
}
