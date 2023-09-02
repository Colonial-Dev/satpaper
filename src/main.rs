#![warn(clippy::all)]

mod config;
mod composite;
mod slider;
mod wallpaper;

use anyhow::{Result, Context};
use clap::Parser;
use tokio::time::{sleep, Duration};

use config::*;

const OUTPUT_NAME: &str = "satpaper_latest.png";
const SLEEP_DURATION: Duration = Duration::from_secs(60);

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "debug");
    }

    env_logger::init();
    
    tokio::task::spawn(update_wallpaper())
        .await
        .context("Wallpaper updating task panicked")?
        .context("An error occurred in the wallpaper updating task")?;

    Ok(())
}

async fn update_wallpaper() -> Result<()> {
    let config = Config::parse();
    let mut timestamp = None;
    
    loop  {
        log::info!("Checking timestamp...");

        let new = slider::fetch_latest_timestamp(&config).await?;

        if timestamp
            .map(|old| old != new)
            .unwrap_or(true) 
        {
            log::info!("Timestamp has changed!");
            log::debug!("Old timestamp: {timestamp:?}, new timestamp: {new}");
            log::info!("Fetching updated source and compositing new wallpaper...");

            timestamp = Some(new);
            
            composite::generate(
                &config, 
            ).await?;

            wallpaper::set(
                config.target_path.join(OUTPUT_NAME),
                config.wallpaper_command.as_deref(),
            )
            .await?;

            log::info!("New wallpaper composited and set.");
        }

        log::debug!("Sleeping for {SLEEP_DURATION:?}...");

        sleep(SLEEP_DURATION).await
    }

    #[allow(unreachable_code)]
    Ok(())
}