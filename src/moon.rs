use std::io::Read;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
struct MoonImageInfo {
    url: String,
}

#[derive(Debug, Deserialize)]
struct DialAMoonImageResponse {
    image: MoonImageInfo,
}

#[derive(Debug)]
pub struct MoonImage {
    pub frame: u32,
    pub data: Vec<u8>,
}

/// Extract frame number from a Dial-a-Moon image URL.
/// e.g. ".../moon.1746.jpg" → 1746
fn parse_frame(url: &str) -> Result<u32> {
    let filename = url
        .rsplit('/')
        .next()
        .context("No filename in URL")?;

    // "moon.1746.jpg" → "1746"
    let num = filename
        .strip_prefix("moon.")
        .and_then(|s| s.strip_suffix(".jpg"))
        .context("Unexpected moon image filename format")?;

    num.parse::<u32>().context("Failed to parse frame number")
}

/// Fetch a clean (undecorated) moon image for a given UTC datetime.
///
/// 1. Calls Dial-a-Moon API to get frame number
/// 2. Downloads the 730x730 JPG from the SVS archive
pub fn fetch_moon_image(datetime: &str) -> Result<MoonImage> {
    let api_url = format!("https://svs.gsfc.nasa.gov/api/dialamoon/{datetime}");

    let reader = ureq::get(&api_url)
        .timeout(TIMEOUT)
        .call()
        .context("Failed to call Dial-a-Moon API")?
        .into_reader();

    let resp: DialAMoonImageResponse =
        serde_json::from_reader(reader).context("Failed to deserialize moon image response")?;

    let frame = parse_frame(&resp.image.url)?;

    // Derive the base path from the API response URL so we don't hardcode the gallery year.
    // API returns: ".../a005587/frames/730x730_1x1_30p/moon.1746.jpg"
    // We want:     ".../a005587/frames/730x730_1x1_30p/moon.1746.jpg" (same path, it's already clean)
    let image_url = &resp.image.url;

    log::info!("Fetching moon image frame {frame} from SVS...");

    let resp = ureq::get(image_url)
        .timeout(TIMEOUT)
        .call()
        .context("Failed to download moon image")?;

    let len: usize = resp
        .header("Content-Length")
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);

    let mut data = Vec::with_capacity(len.max(1024));
    resp.into_reader()
        .read_to_end(&mut data)
        .context("Failed to read moon image data")?;

    log::info!("Moon image downloaded: {} KiB", data.len() / 1024);

    Ok(MoonImage { frame, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frame() {
        assert_eq!(parse_frame("https://svs.gsfc.nasa.gov/vis/a000000/a005500/a005587/frames/730x730_1x1_30p/moon.1746.jpg").unwrap(), 1746);
        assert_eq!(parse_frame("https://example.com/moon.0001.jpg").unwrap(), 1);
    }

    #[test]
    fn test_fetch_moon_image() -> Result<()> {
        let img = fetch_moon_image("2026-03-14T17:00")?;
        assert!(img.data.len() > 1000, "Moon image too small: {} bytes", img.data.len());
        assert!(img.frame > 0);
        // JPEG magic bytes
        assert_eq!(&img.data[..2], &[0xFF, 0xD8]);
        log::info!("Moon frame: {}, size: {} KiB", img.frame, img.data.len() / 1024);
        Ok(())
    }
}
