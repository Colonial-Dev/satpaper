use std::path::Path;

use anyhow::{Context, Result};
use fimg::Image as Img;

use crate::orbital::{self, ScanEntry, ScanResult};

type Image<T> = Img<T, 3>;

// Physical sizes
const R_EARTH_KM: f64 = 6_371.0;
const R_MOON_KM: f64 = 1_737.4;
const R_SUN_KM: f64 = 695_500.0;
const AU_KM: f64 = 149_597_870.7;

// Maximum size ratio between Earth and Moon/Sun in the final image
const MAX_SIZE_RATIO: f64 = 3.0;

// Padding: bodies should fit within this fraction of the canvas
const FRAME_PADDING: f64 = 0.85;

// Colors (RGB)
const COLOR_EARTH: [u8; 3] = [40, 80, 200];
const COLOR_MOON: [u8; 3] = [220, 220, 220];
const COLOR_SUN: [u8; 3] = [255, 200, 50];

/// The computed dolly zoom camera parameters.
#[derive(Debug)]
struct DollyView {
    camera_distance: f64,   // km from Earth center
    fov_deg: f64,           // horizontal field of view (degrees)
    earth_radius_deg: f64,  // angular radius of Earth (degrees)
    moon_radius_deg: f64,   // angular radius of Moon (degrees)
    sun_radius_deg: f64,    // angular radius of Sun (degrees)
    moon_h: f64,            // Moon horizontal angle from Earth center (degrees)
    moon_v: f64,            // Moon vertical angle (degrees)
    sun_h: f64,             // Sun horizontal angle (degrees)
    sun_v: f64,             // Sun vertical angle (degrees)
    show_moon: bool,        // true = show Moon, false = show Sun
    roll_deg: f64,          // camera roll applied (degrees)
}

const MAX_COMPANION_ANGLE_DEG: f64 = 15.0;

/// Compute dolly zoom parameters from a ScanEntry.
///
/// Shows Earth plus the closest companion body (Moon or Sun).
/// Moves the camera back until Earth is at most MAX_SIZE_RATIO times the
/// companion's angular size, then sets FOV to frame both.
fn compute_dolly(view: &ScanEntry, aspect: f64) -> DollyView {
    let moon_angular_radius = (R_MOON_KM / view.moon_distance).asin().to_degrees();
    let sun_angular_radius = (R_SUN_KM / AU_KM).asin().to_degrees();

    // Pick the closer companion body (smaller theta_em = closer to Earth center).
    // Use the score field which already holds min(moon_theta_em, sun_theta_em).
    let show_moon = view.moon_theta_limb.abs() <= view.sun_theta_limb.abs();

    let companion_radius = if show_moon { moon_angular_radius } else { sun_angular_radius };

    // Find camera distance where Earth angular radius = MAX_SIZE_RATIO * companion
    let target_earth_radius = MAX_SIZE_RATIO * companion_radius;
    let camera_distance = R_EARTH_KM / target_earth_radius.to_radians().sin();

    let earth_radius_deg = (R_EARTH_KM / camera_distance).asin().to_degrees();

    // Place camera at this distance along the satellite direction
    let cam = [
        view.sat_dir[0] * camera_distance,
        view.sat_dir[1] * camera_distance,
        view.sat_dir[2] * camera_distance,
    ];

    // Build camera frame: boresight toward Earth, up = north projected
    let boresight = orbital::normalize(&[-cam[0], -cam[1], -cam[2]]);
    let north: [f64; 3] = [0.0, 0.0, 1.0];
    let right = orbital::normalize(&orbital::cross(&boresight, &north));
    let up = orbital::cross(&right, &boresight);

    let angles_from_camera = |body_pos: &[f64; 3]| -> (f64, f64) {
        let to_body = [
            body_pos[0] - cam[0],
            body_pos[1] - cam[1],
            body_pos[2] - cam[2],
        ];
        let depth = orbital::dot(&to_body, &boresight);
        let h = orbital::dot(&to_body, &right);
        let v = orbital::dot(&to_body, &up);
        (h.atan2(depth).to_degrees(), v.atan2(depth).to_degrees())
    };

    let (moon_h, moon_v) = angles_from_camera(&view.moon_pos);

    let sun_pos = [
        view.sun_dir[0] * AU_KM,
        view.sun_dir[1] * AU_KM,
        view.sun_dir[2] * AU_KM,
    ];
    let (sun_h, sun_v) = angles_from_camera(&sun_pos);

    // Roll the camera so the companion is at most MAX_COMPANION_ANGLE_DEG from horizontal.
    let (comp_h, comp_v) = if show_moon { (moon_h, moon_v) } else { (sun_h, sun_v) };
    let companion_angle = comp_v.atan2(comp_h).to_degrees(); // angle from horizontal
    let roll_deg = if companion_angle.abs() > MAX_COMPANION_ANGLE_DEG {
        companion_angle - companion_angle.signum() * MAX_COMPANION_ANGLE_DEG
    } else {
        0.0
    };

    // Apply 2D rotation to all angles: h' = h*cos(α) + v*sin(α), v' = -h*sin(α) + v*cos(α)
    let roll = roll_deg.to_radians();
    let (sin_r, cos_r) = roll.sin_cos();
    let rotate = |h: f64, v: f64| -> (f64, f64) {
        (h * cos_r + v * sin_r, -h * sin_r + v * cos_r)
    };

    let (moon_h, moon_v) = rotate(moon_h, moon_v);
    let (sun_h, sun_v) = rotate(sun_h, sun_v);

    // FOV: frame Earth + the chosen companion, accounting for aspect ratio.
    // Compute the required half-FOV in horizontal and vertical separately,
    // then pick whichever constrains the horizontal FOV more.
    let mut max_h = earth_radius_deg;
    let mut max_v = earth_radius_deg;

    if show_moon {
        max_h = max_h.max(moon_h.abs() + moon_angular_radius);
        max_v = max_v.max(moon_v.abs() + moon_angular_radius);
    } else {
        max_h = max_h.max(sun_h.abs() + sun_angular_radius);
        max_v = max_v.max(sun_v.abs() + sun_angular_radius);
    }

    // Horizontal FOV needed to fit horizontal extent
    let fov_from_h = 2.0 * max_h / FRAME_PADDING;
    // Horizontal FOV needed so vertical FOV (= h_fov / aspect) fits vertical extent
    let fov_from_v = 2.0 * max_v * aspect / FRAME_PADDING;

    let fov_deg = fov_from_h.max(fov_from_v);

    DollyView {
        camera_distance,
        fov_deg,
        earth_radius_deg,
        moon_radius_deg: moon_angular_radius,
        sun_radius_deg: sun_angular_radius,
        moon_h,
        moon_v,
        sun_h,
        sun_v,
        show_moon,
        roll_deg,
    }
}

/// Draw a filled circle on the image. Coordinates are in pixels.
fn draw_circle(img: &mut Image<&mut [u8]>, cx: f64, cy: f64, radius: f64, color: [u8; 3]) {
    let w = img.width() as i32;
    let h = img.height() as i32;
    let r2 = radius * radius;

    let x_min = ((cx - radius).floor() as i32).max(0);
    let x_max = ((cx + radius).ceil() as i32).min(w - 1);
    let y_min = ((cy - radius).floor() as i32).max(0);
    let y_max = ((cy + radius).ceil() as i32).min(h - 1);

    for x in x_min..=x_max {
        for y in y_min..=y_max {
            let dx = x as f64 - cx;
            let dy = y as f64 - cy;
            if dx * dx + dy * dy <= r2 {
                unsafe { img.set_pixel(x as u32, y as u32, &color) };
            }
        }
    }
}

/// Render a placeholder image with dolly zoom applied.
pub fn render_placeholder(
    view: &ScanEntry,
    width: u32,
    height: u32,
) -> Image<Box<[u8]>> {
    let aspect = width as f64 / height as f64;
    let dolly = compute_dolly(view, aspect);

    let ppd = width as f64 / dolly.fov_deg;

    let earth_cx = width as f64 / 2.0;
    let earth_cy = height as f64 / 2.0;
    let earth_r = dolly.earth_radius_deg * ppd;

    let moon_cx = earth_cx + dolly.moon_h * ppd;
    let moon_cy = earth_cy - dolly.moon_v * ppd;
    let moon_r = dolly.moon_radius_deg * ppd;

    let sun_cx = earth_cx + dolly.sun_h * ppd;
    let sun_cy = earth_cy - dolly.sun_v * ppd;
    let sun_r = dolly.sun_radius_deg * ppd;

    let companion = if dolly.show_moon { "Moon" } else { "Sun" };
    let companion_ratio = if dolly.show_moon {
        dolly.earth_radius_deg / dolly.moon_radius_deg
    } else {
        dolly.earth_radius_deg / dolly.sun_radius_deg
    };
    eprintln!(
        "Dolly zoom: camera at {:.0} km, FOV {:.2}°, showing Earth+{}, ratio {:.1}:1, roll {:.1}°",
        dolly.camera_distance,
        dolly.fov_deg,
        companion,
        companion_ratio,
        dolly.roll_deg,
    );
    eprintln!(
        "  Earth: ({:.0},{:.0}) r={:.0}px, Moon: ({:.0},{:.0}) r={:.0}px @ h={:.2}° v={:.2}°, Sun: ({:.0},{:.0}) r={:.0}px @ h={:.2}° v={:.2}°",
        earth_cx, earth_cy, earth_r,
        moon_cx, moon_cy, moon_r, dolly.moon_h, dolly.moon_v,
        sun_cx, sun_cy, sun_r, dolly.sun_h, dolly.sun_v,
    );

    let mut canvas = Image::alloc(width, height).boxed();

    // Draw Earth + chosen companion only
    if dolly.show_moon {
        draw_circle(&mut canvas.as_mut(), earth_cx, earth_cy, earth_r, COLOR_EARTH);
        draw_circle(&mut canvas.as_mut(), moon_cx, moon_cy, moon_r, COLOR_MOON);
    } else {
        draw_circle(&mut canvas.as_mut(), sun_cx, sun_cy, sun_r, COLOR_SUN);
        draw_circle(&mut canvas.as_mut(), earth_cx, earth_cy, earth_r, COLOR_EARTH);
    }

    canvas
}

/// Compose a placeholder by fetching live orbital data for the given datetime.
pub fn compose_placeholder(
    utc_datetime: &str,
    width: u32,
    height: u32,
) -> Result<Image<Box<[u8]>>> {
    let data = orbital::closest_satellite(utc_datetime, None)?;

    let winner = data
        .satellites
        .iter()
        .find(|s| s.satellite == data.winner)
        .expect("Winner must be in satellites list");

    let sat_mag = orbital::mag(&winner.sat_pos);
    let sat_dir = [
        winner.sat_pos[0] / sat_mag,
        winner.sat_pos[1] / sat_mag,
        winner.sat_pos[2] / sat_mag,
    ];

    let view = ScanEntry {
        datetime: utc_datetime.to_string(),
        winner: winner.satellite,
        tier: winner.tier,
        moon_theta_limb: winner.moon_theta_limb,
        moon_horizontal: winner.moon_horizontal,
        moon_vertical: winner.moon_vertical,
        moon_visible: winner.moon_visible,
        sun_theta_limb: winner.sun_theta_limb,
        sun_horizontal: winner.sun_horizontal,
        sun_vertical: winner.sun_vertical,
        sun_visible: winner.sun_visible,
        moon_phase: data.moon_phase,
        moon_distance: data.moon_distance,
        score: winner.moon_theta_limb.abs() + 0.5 * winner.sun_theta_limb.abs(),
        sat_dir,
        moon_pos: data.moon_pos,
        sun_dir: data.sun_dir,
    };

    Ok(render_placeholder(&view, width, height))
}

/// Load a ScanResult from a JSON file.
pub fn load_scan(path: &Path) -> Result<ScanResult> {
    let json = std::fs::read_to_string(path)
        .context("Failed to read scan JSON file")?;
    serde_json::from_str(&json)
        .context("Failed to parse scan JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_from_scan_file() -> Result<()> {
        let scan_path = Path::new("scan_results.json");
        if !scan_path.exists() {
            eprintln!("No scan_results.json found — run scan_best_view test first");
            return Ok(());
        }

        let scan = load_scan(scan_path)?;
        eprintln!(
            "Best: {} {:?} ({:?}) score={:.1}",
            scan.best.datetime, scan.best.winner, scan.best.tier, scan.best.score,
        );

        let img = render_placeholder(&scan.best, 1920, 1080);

        let out = Path::new("/tmp/spacepaper_test_composite.png");
        img.save(out);
        eprintln!("Saved to {}", out.display());

        assert_eq!(img.width(), 1920);
        assert_eq!(img.height(), 1080);

        Ok(())
    }

    #[test]
    fn test_compose_live() -> Result<()> {
        let img = compose_placeholder("2026-03-14T12:00", 1920, 1080)?;

        assert_eq!(img.width(), 1920);
        assert_eq!(img.height(), 1080);

        let buf = img.buffer();
        let non_black = buf.chunks(3).any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
        assert!(non_black, "Image is entirely black");

        Ok(())
    }
}
