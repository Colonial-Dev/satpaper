mod config;
mod slider;
mod wallpaper;

use std::time::Duration;
use std::thread::sleep;
use std::sync::Arc;

use anyhow::{Result, Context};
use clap::Parser;
use mimalloc::MiMalloc;

use config::*;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const OUTPUT_NAME: &str = "satpaper_latest.png";
const SLEEP_DURATION: Duration = Duration::from_secs(60);

fn main() -> Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    env_logger::init();
    
    update_wallpaper()
        .context("An error occurred in the wallpaper updating loop")?;

    Ok(())
}

fn update_wallpaper() -> Result<()> {
    let config = Config::parse();
    let config = Arc::new(config);
    
    let mut timestamp = None;
    
    loop  {
        log::info!("Checking timestamp...");

        let new = slider::fetch_latest_timestamp(&config)?;

        if timestamp
            .map(|old| old != new)
            .unwrap_or(true) 
        {
            log::info!("Timestamp has changed!");
            log::debug!("Old timestamp: {timestamp:?}, new timestamp: {new}");
            log::info!("Fetching updated source and compositing new wallpaper...");

            timestamp = Some(new);
            
            slider::composite_latest_image(&config)?;

            if config.once {
                return Ok(());
            }

            wallpaper::set(
                config.target_path.join(OUTPUT_NAME),
                config.wallpaper_command.as_deref(),
            )?;

            log::info!("New wallpaper composited and set.");
        }
        
        // Safety: as far as I can tell, this function doesn't have any safety
        // preconditions?
        //
        // Even the official C documentation doesn't document any invariants etc -
        // it only mentions that it's intended for specific niche cases (which we happen to be one of!)
        unsafe {
            // Aggressively return as much memory to the operating system as possible.
            //
            // (Yes, this is necessary.)
            libmimalloc_sys::mi_collect(true);
        }

        log::debug!("Sleeping for {SLEEP_DURATION:?}...");

        sleep(SLEEP_DURATION)
    }

    #[allow(unreachable_code)]
    Ok(())
}
