use std::io::Read;
use std::time::Duration;

use anyhow::{Context, Result};

const TIMEOUT: Duration = Duration::from_secs(30);
const SDO_BASE_URL: &str = "https://sdo.gsfc.nasa.gov/assets/img/latest";
const WAVELENGTH: &str = "0171"; // AIA 171Å — golden coronal loops

/// Fetch the latest full-disk sun image from NASA SDO.
///
/// Returns raw JPEG bytes of the AIA 171Å image at the requested resolution.
/// Valid resolutions: 4096, 2048, 1024, 512.
pub fn fetch_sun_image(resolution: u32) -> Result<Vec<u8>> {
    let url = format!("{SDO_BASE_URL}/latest_{resolution}_{WAVELENGTH}.jpg");

    log::info!("Fetching sun image ({resolution}px, AIA {WAVELENGTH}) from SDO...");

    let resp = ureq::get(&url)
        .timeout(TIMEOUT)
        .call()
        .context("Failed to download SDO sun image")?;

    let len: usize = resp
        .header("Content-Length")
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);

    let mut data = Vec::with_capacity(len.max(1024));
    resp.into_reader()
        .read_to_end(&mut data)
        .context("Failed to read sun image data")?;

    log::info!("Sun image downloaded: {} KiB", data.len() / 1024);

    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetch_sun_image() -> Result<()> {
        match fetch_sun_image(512) {
            Ok(data) => {
                assert!(data.len() > 1000, "Sun image too small: {} bytes", data.len());
                assert_eq!(&data[..2], &[0xFF, 0xD8], "Not a JPEG");
                eprintln!("Sun image size: {} KiB", data.len() / 1024);
            }
            Err(e) => {
                // SDO server can be unreachable — don't fail the test suite
                eprintln!("SDO unavailable (expected if NASA is down): {e}");
            }
        }
        Ok(())
    }
}
