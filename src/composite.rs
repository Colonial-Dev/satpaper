use anyhow::Result;

use crate::slider;

use super::{
    Config,
    OUTPUT_NAME
};

pub async fn generate(config: &Config) -> Result<()> {
    use image::Rgba;
    use image::ImageBuffer;
    use image::imageops::{FilterType, self};

    log::info!("Downloading SLIDER source image...");

    let source = slider::fetch_latest_image(config).await?;

    log::info!("Resizing source image...");

    let new_dims = config.resolution_y as f32 * (config.disk_size as f32 / 100.0);
    let new_dims = new_dims.floor() as u32;

    let source = imageops::resize(
        &source,
        new_dims,
        new_dims,
        FilterType::Lanczos3
    );

    log::info!("Generating destination image...");

    let mut destination = ImageBuffer::new(
        config.resolution_x, 
        config.resolution_y
    );

    for (_, _, pixel) in destination.enumerate_pixels_mut() {
        *pixel = Rgba([u8::MIN, u8::MIN, u8::MIN, u8::MAX])
    }

    log::info!("Compositing source into destination...");

    imageops::overlay(
        &mut destination,
        &source,
        ((config.resolution_x - new_dims) / 2) as i64,
        ((config.resolution_y - new_dims) / 2) as i64
    );

    log::info!("Compositing complete.");

    destination.save(
        config.target_path.join(OUTPUT_NAME)
    )?;

    log::info!("Output saved.");

    Ok(())
}