use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use fimg::Image as Img;

type Image<T> = Img<T, 3>;

/// Convert an ECI satellite direction to J2000 RA/Dec.
/// The camera boresight points from the satellite toward Earth center, i.e. `-sat_dir`.
/// Returns (ra_hours, dec_deg).
pub fn sat_dir_to_ra_dec(sat_dir: &[f64; 3]) -> (f64, f64) {
    // Camera looks toward Earth center = -sat_dir
    let boresight = [-sat_dir[0], -sat_dir[1], -sat_dir[2]];

    let dec_rad = boresight[2].asin();
    let ra_rad = boresight[1].atan2(boresight[0]);

    let mut ra_hours = ra_rad * 12.0 / std::f64::consts::PI;
    if ra_hours < 0.0 {
        ra_hours += 24.0;
    }

    (ra_hours, dec_rad.to_degrees())
}

/// Generate a starfield background image using star-charter.
///
/// Returns `None` if star-charter is not available or fails.
pub fn generate_starfield(
    sat_dir: &[f64; 3],
    fov_deg: f64,
    roll_deg: f64,
    width: u32,
    height: u32,
    star_chart_bin: Option<&str>,
) -> Option<Image<Box<[u8]>>> {
    let bin = star_chart_bin.unwrap_or("starchart.bin");

    // Check if binary exists
    if Command::new(bin).arg("--version").output().is_err() {
        log::warn!("star-charter binary not found at '{bin}', skipping starfield");
        return None;
    }

    match generate_starfield_inner(sat_dir, fov_deg, roll_deg, width, height, bin) {
        Ok(img) => Some(img),
        Err(e) => {
            log::warn!("Failed to generate starfield: {e}");
            None
        }
    }
}

fn generate_starfield_inner(
    sat_dir: &[f64; 3],
    fov_deg: f64,
    roll_deg: f64,
    width: u32,
    height: u32,
    bin: &str,
) -> Result<Image<Box<[u8]>>> {
    let (ra_hours, dec_deg) = sat_dir_to_ra_dec(sat_dir);
    let aspect = height as f64 / width as f64;

    // star-charter width is in cm; at output_dpi=100, 1cm ≈ 39.37px
    // We want the output to be `width` pixels wide.
    let width_cm = width as f64 / 39.3701;

    let tmp_dir = tempfile::tempdir().context("Failed to create temp dir")?;
    let config_path = tmp_dir.path().join("starfield.sch");
    let output_path = tmp_dir.path().join("starfield.png");

    // position_angle rotates the chart; we pass -roll_deg so that when the
    // compositor applies its roll the stars end up correctly oriented.
    let position_angle = -roll_deg;

    // Scale star size with resolution so stars look the same relative to the frame.
    // Tuned for 0.6 at 1920px wide.
    let mag_size_norm = 0.6 * (width as f64 / 1920.0);

    let config = format!(
        "DEFAULTS\n\
         coords=ra_dec\n\
         projection=gnomonic\n\
         ra_central={ra_hours:.6}\n\
         dec_central={dec_deg:.6}\n\
         angular_width={fov_deg:.6}\n\
         aspect={aspect:.6}\n\
         width={width_cm:.4}\n\
         output_dpi=100\n\
         position_angle={position_angle:.6}\n\
         star_col=1,1,1\n\
         star_bv_colour=1\n\
         star_glow=1\n\
         star_label_col=0,0,0,0\n\
         plot_stars=1\n\
         star_names=0\n\
         mag_min=11.5\n\
         mag_max=-2.0\n\
         mag_size_norm={mag_size_norm:.4}\n\
         maximum_star_count=200000\n\
         maximum_star_label_count=0\n\
         plot_dso=0\n\
         constellation_sticks=0\n\
         constellation_names=0\n\
         constellation_boundaries=0\n\
         show_grid_lines=0\n\
         plot_equator=0\n\
         plot_ecliptic=0\n\
         plot_galactic_plane=0\n\
         plot_galaxy_map=1\n\
         galaxy_map_width_pixels=4096\n\
         magnitude_key=0\n\
         great_circle_key=0\n\
         dso_symbol_key=0\n\
         galaxy_col=0.14,0.12,0.10\n\
         galaxy_col0=0,0,0\n\
         chart_edge_line_width=0\n\
         \n\
         CHART\n\
         output_filename={output}\n",
        output = output_path.display(),
    );

    let mut f = std::fs::File::create(&config_path)
        .context("Failed to write star-charter config")?;
    f.write_all(config.as_bytes())?;
    drop(f);

    let status = Command::new(bin)
        .arg(config_path.to_str().unwrap())
        .output()
        .context("Failed to run star-charter")?;

    if !status.status.success() {
        anyhow::bail!(
            "star-charter exited with {}: {}",
            status.status,
            String::from_utf8_lossy(&status.stderr)
        );
    }

    // Read the output PNG and convert to fimg Image
    let img = image::ImageReader::open(&output_path)
        .context("Failed to open starfield PNG")?
        .decode()
        .context("Failed to decode starfield PNG")?
        .into_rgb8();

    let mut result: Image<Box<[u8]>> =
        Img::build(img.width(), img.height()).buf(img.into_vec().into_boxed_slice());

    // Resize to exact target if star-charter produced slightly different dimensions
    if result.width() != width || result.height() != height {
        use fimg::scale::Lanczos3;
        result = result.scale::<Lanczos3>(width, height);
    }

    Ok(result)
}

/// Cache key for starfield images — regenerate only when satellite or FOV changes.
#[derive(Debug)]
struct StarfieldCacheKey {
    satellite_id: String,
    fov_deg_10x: i32, // fov_deg * 10, rounded
    roll_deg_1x: i32, // roll_deg * 1, rounded
}

impl StarfieldCacheKey {
    fn new(satellite_id: &str, fov_deg: f64, roll_deg: f64) -> Self {
        Self {
            satellite_id: satellite_id.to_string(),
            fov_deg_10x: (fov_deg * 10.0).round() as i32,
            roll_deg_1x: roll_deg.round() as i32,
        }
    }

    fn cache_path(&self) -> PathBuf {
        PathBuf::from(format!(
            "/tmp/spacepaper_stars_{}_fov{}_roll{}.png",
            self.satellite_id, self.fov_deg_10x, self.roll_deg_1x
        ))
    }
}

/// Generate a starfield, using a file cache to avoid regenerating when params haven't changed.
pub fn generate_starfield_cached(
    satellite_id: &str,
    sat_dir: &[f64; 3],
    fov_deg: f64,
    roll_deg: f64,
    width: u32,
    height: u32,
    star_chart_bin: Option<&str>,
) -> Option<Image<Box<[u8]>>> {
    let key = StarfieldCacheKey::new(satellite_id, fov_deg, roll_deg);
    let cache_path = key.cache_path();

    // Try loading from cache
    if cache_path.exists() {
        if let Ok(reader) = image::ImageReader::open(&cache_path) {
            if let Ok(img) = reader.decode() {
                let rgb = img.into_rgb8();
                if rgb.width() == width && rgb.height() == height {
                    log::info!("Using cached starfield from {}", cache_path.display());
                    return Some(
                        Img::build(rgb.width(), rgb.height())
                            .buf(rgb.into_vec().into_boxed_slice()),
                    );
                }
            }
        }
    }

    // Generate fresh
    let img = generate_starfield(sat_dir, fov_deg, roll_deg, width, height, star_chart_bin)?;

    // Save to cache
    img.save(&cache_path);
    log::info!("Cached starfield to {}", cache_path.display());

    Some(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ra_dec_conversion() {
        // Satellite at +X axis → boresight = -X → RA=12h, Dec=0°
        let (ra, dec) = sat_dir_to_ra_dec(&[1.0, 0.0, 0.0]);
        assert!((ra - 12.0).abs() < 0.001, "RA should be 12h, got {ra}");
        assert!(dec.abs() < 0.001, "Dec should be 0°, got {dec}");

        // Satellite at -X axis → boresight = +X → RA=0h, Dec=0°
        let (ra, dec) = sat_dir_to_ra_dec(&[-1.0, 0.0, 0.0]);
        assert!(ra.abs() < 0.001 || (ra - 24.0).abs() < 0.001, "RA should be 0h, got {ra}");
        assert!(dec.abs() < 0.001, "Dec should be 0°, got {dec}");

        // Satellite at +Z axis → boresight = -Z → Dec = -90°
        let (_, dec) = sat_dir_to_ra_dec(&[0.0, 0.0, 1.0]);
        assert!((dec - (-90.0)).abs() < 0.001, "Dec should be -90°, got {dec}");

        // Satellite at -Z axis → boresight = +Z → Dec = +90°
        let (_, dec) = sat_dir_to_ra_dec(&[0.0, 0.0, -1.0]);
        assert!((dec - 90.0).abs() < 0.001, "Dec should be +90°, got {dec}");
    }

    #[test]
    fn test_generate_starfield() {
        // Point toward Orion (RA ~5.5h, Dec ~0°) for a star-rich field
        let ra_h = 5.5;
        let dec_d = 0.0_f64;
        let ra_rad = ra_h * std::f64::consts::PI / 12.0;
        let dec_rad = dec_d.to_radians();
        // Boresight = -sat_dir, so sat_dir = -boresight
        let boresight = [dec_rad.cos() * ra_rad.cos(), dec_rad.cos() * ra_rad.sin(), dec_rad.sin()];
        let sat_dir = [-boresight[0], -boresight[1], -boresight[2]];
        let bin = "/home/chad/Code/star-charter/bin/starchart.bin";
        let img = generate_starfield(&sat_dir, 25.0, 0.0, 1920, 1080, Some(bin));
        if let Some(img) = &img {
            let out = std::path::Path::new("/tmp/spacepaper_test_starfield.png");
            img.save(out);
            eprintln!("Saved starfield test to {}", out.display());
            assert_eq!(img.width(), 1920);
            assert_eq!(img.height(), 1080);
            // Should not be all white — most pixels should be dark
            let buf = img.buffer();
            let dark_pixels = buf.chunks(3).filter(|px| px[0] < 30 && px[1] < 30 && px[2] < 30).count();
            let total = buf.len() / 3;
            eprintln!("Dark pixels: {dark_pixels}/{total} ({:.1}%)", 100.0 * dark_pixels as f64 / total as f64);
            assert!(dark_pixels > total / 2, "Starfield should be mostly dark");
        } else {
            eprintln!("star-charter not found, skipping");
        }
    }
}
