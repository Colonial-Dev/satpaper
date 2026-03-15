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

// Satellite colors for graphs
fn satellite_color(sat: crate::config::Satellite) -> [u8; 3] {
    use crate::config::Satellite::*;
    match sat {
        GOESEast   => [100, 180, 255], // light blue
        GOESWest   => [255, 100, 100], // red
        Himawari   => [100, 255, 100], // green
        Meteosat9  => [255, 180, 50],  // orange
        Meteosat10 => [200, 100, 255], // purple
    }
}

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
    show_moon: bool,        // show Moon
    show_sun: bool,         // show Sun
    roll_deg: f64,          // camera roll applied (degrees)
}

const MAX_COMPANION_ANGLE_DEG: f64 = 15.0;
const BOTH_VISIBLE_THRESHOLD: f64 = 40.0;

/// Compute dolly zoom parameters from a ScanEntry.
///
/// If both Moon and Sun are within BOTH_VISIBLE_THRESHOLD, shows all three.
/// Otherwise shows Earth plus the closest companion body.
/// Moves the camera back until Earth is at most MAX_SIZE_RATIO times the
/// smallest companion's angular size, then sets FOV to frame visible bodies.
fn compute_dolly(view: &ScanEntry, aspect: f64) -> DollyView {
    let moon_angular_radius = (R_MOON_KM / view.moon_distance).asin().to_degrees();
    let sun_angular_radius = (R_SUN_KM / AU_KM).asin().to_degrees();

    let moon_angle = view.moon_horizontal.hypot(view.moon_vertical);
    let sun_angle = view.sun_horizontal.hypot(view.sun_vertical);

    let show_both = moon_angle < BOTH_VISIBLE_THRESHOLD && sun_angle < BOTH_VISIBLE_THRESHOLD;
    let (show_moon, show_sun) = if show_both {
        (true, true)
    } else if moon_angle <= sun_angle {
        (true, false)
    } else {
        (false, true)
    };

    // Size the dolly zoom based on the smallest visible companion
    let companion_radius = match (show_moon, show_sun) {
        (true, true) => moon_angular_radius.max(sun_angular_radius),
        (true, false) => moon_angular_radius,
        (false, true) => sun_angular_radius,
        _ => unreachable!(),
    };

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

    // Roll the camera to align visible bodies near horizontal.
    // When showing both, align the midpoint of the two companions.
    // When showing one, align that companion.
    let roll_deg = {
        let (target_h, target_v) = if show_both {
            ((moon_h + sun_h) / 2.0, (moon_v + sun_v) / 2.0)
        } else if show_moon {
            (moon_h, moon_v)
        } else {
            (sun_h, sun_v)
        };
        let angle = target_v.atan2(target_h).to_degrees();
        if angle.abs() > MAX_COMPANION_ANGLE_DEG {
            angle - angle.signum() * MAX_COMPANION_ANGLE_DEG
        } else {
            0.0
        }
    };

    // Apply 2D rotation
    let roll = roll_deg.to_radians();
    let (sin_r, cos_r) = roll.sin_cos();
    let rotate = |h: f64, v: f64| -> (f64, f64) {
        (h * cos_r + v * sin_r, -h * sin_r + v * cos_r)
    };

    let (moon_h, moon_v) = rotate(moon_h, moon_v);
    let (sun_h, sun_v) = rotate(sun_h, sun_v);

    // FOV: frame Earth + visible companions, accounting for aspect ratio.
    let mut max_h = earth_radius_deg;
    let mut max_v = earth_radius_deg;

    if show_moon {
        max_h = max_h.max(moon_h.abs() + moon_angular_radius);
        max_v = max_v.max(moon_v.abs() + moon_angular_radius);
    }
    if show_sun {
        max_h = max_h.max(sun_h.abs() + sun_angular_radius);
        max_v = max_v.max(sun_v.abs() + sun_angular_radius);
    }

    let fov_from_h = 2.0 * max_h / FRAME_PADDING;
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
        show_sun,
        roll_deg,
    }
}

/// 5x7 bitmap font for graph labels. Each glyph is 5 columns of 7 bits (LSB = top row).
fn glyph(ch: char) -> [u8; 5] {
    match ch {
        '0' => [0x3E, 0x51, 0x49, 0x45, 0x3E],
        '1' => [0x00, 0x42, 0x7F, 0x40, 0x00],
        '2' => [0x42, 0x61, 0x51, 0x49, 0x46],
        '3' => [0x21, 0x41, 0x45, 0x4B, 0x31],
        '4' => [0x18, 0x14, 0x12, 0x7F, 0x10],
        '5' => [0x27, 0x45, 0x45, 0x45, 0x39],
        '6' => [0x3C, 0x4A, 0x49, 0x49, 0x30],
        '7' => [0x01, 0x71, 0x09, 0x05, 0x03],
        '8' => [0x36, 0x49, 0x49, 0x49, 0x36],
        '9' => [0x06, 0x49, 0x49, 0x29, 0x1E],
        '-' => [0x08, 0x08, 0x08, 0x08, 0x08],
        '/' => [0x20, 0x10, 0x08, 0x04, 0x02],
        ' ' => [0x00, 0x00, 0x00, 0x00, 0x00],
        'M' => [0x7F, 0x02, 0x0C, 0x02, 0x7F],
        'F' => [0x7F, 0x09, 0x09, 0x09, 0x01],
        'S' => [0x46, 0x49, 0x49, 0x49, 0x31],
        'T' => [0x01, 0x01, 0x7F, 0x01, 0x01],
        'W' => [0x3F, 0x40, 0x30, 0x40, 0x3F],
        _ => [0x7F, 0x41, 0x41, 0x41, 0x7F], // box for unknown
    }
}

/// Draw a text string at (x, y) with the given color and scale.
fn draw_text(img: &mut Image<&mut [u8]>, x: f64, y: f64, text: &str, color: [u8; 3], scale: u32) {
    let w = img.width() as i32;
    let h = img.height() as i32;

    for (ci, ch) in text.chars().enumerate() {
        let cols = glyph(ch);
        for (col_i, &col_bits) in cols.iter().enumerate() {
            for row in 0..7u32 {
                if col_bits & (1 << row) != 0 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let px = x as i32 + (ci as i32 * 6 * scale as i32) + col_i as i32 * scale as i32 + sx as i32;
                            let py = y as i32 + row as i32 * scale as i32 + sy as i32;
                            if px >= 0 && px < w && py >= 0 && py < h {
                                unsafe { img.set_pixel(px as u32, py as u32, &color) };
                            }
                        }
                    }
                }
            }
        }
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

    let bodies = match (dolly.show_moon, dolly.show_sun) {
        (true, true) => "Moon+Sun",
        (true, false) => "Moon",
        (false, true) => "Sun",
        _ => "none",
    };
    eprintln!(
        "Dolly zoom: camera at {:.0} km, FOV {:.2}°, showing Earth+{}, roll {:.1}°",
        dolly.camera_distance,
        dolly.fov_deg,
        bodies,
        dolly.roll_deg,
    );
    eprintln!(
        "  Earth: ({:.0},{:.0}) r={:.0}px, Moon: ({:.0},{:.0}) r={:.0}px @ h={:.2}° v={:.2}°, Sun: ({:.0},{:.0}) r={:.0}px @ h={:.2}° v={:.2}°",
        earth_cx, earth_cy, earth_r,
        moon_cx, moon_cy, moon_r, dolly.moon_h, dolly.moon_v,
        sun_cx, sun_cy, sun_r, dolly.sun_h, dolly.sun_v,
    );

    let mut canvas = Image::alloc(width, height).boxed();

    // Draw back to front: sun behind earth, moon in front
    if dolly.show_sun {
        draw_circle(&mut canvas.as_mut(), sun_cx, sun_cy, sun_r, COLOR_SUN);
    }
    draw_circle(&mut canvas.as_mut(), earth_cx, earth_cy, earth_r, COLOR_EARTH);
    if dolly.show_moon {
        draw_circle(&mut canvas.as_mut(), moon_cx, moon_cy, moon_r, COLOR_MOON);
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

    #[test]
    fn test_render_angle_graph() -> Result<()> {
        let scan_path = Path::new("scan_results.json");
        if !scan_path.exists() {
            eprintln!("No scan_results.json found — run scan_best_view test first");
            return Ok(());
        }

        let scan = load_scan(scan_path)?;

        // Collect all entries sorted chronologically
        let mut all: Vec<&ScanEntry> = std::iter::once(&scan.best)
            .chain(scan.entries.iter())
            .collect();
        all.sort_by(|a, b| a.datetime.cmp(&b.datetime));

        let w: u32 = 1920;
        let h: u32 = 600;
        let margin_l: f64 = 60.0;
        let margin_r: f64 = 20.0;
        let margin_t: f64 = 30.0;
        let margin_b: f64 = 50.0;
        let plot_w = w as f64 - margin_l - margin_r;
        let plot_h = h as f64 - margin_t - margin_b;

        // Y axis: companion angle (theta_em). Find range.
        let angles: Vec<f64> = all.iter().map(|e| {
            if e.moon_theta_limb.abs() <= e.sun_theta_limb.abs() {
                e.moon_horizontal.hypot(e.moon_vertical)
            } else {
                e.sun_horizontal.hypot(e.sun_vertical)
            }
        }).collect();
        // Round y_max up to next 10°
        let y_max_raw = angles.iter().cloned().fold(0.0f64, f64::max).max(1.0);
        let y_max = (y_max_raw / 10.0).ceil() * 10.0;
        let y_min = 0.0;

        let mut canvas = Image::alloc(w, h).boxed();
        // Dark gray background
        let bg = [30u8, 30, 30];
        for y in 0..h {
            for x in 0..w {
                unsafe { canvas.as_mut().set_pixel(x, y, &bg) };
            }
        }

        let label_color = [160u8, 160, 160];
        let grid_color = [80u8, 80, 80];
        let axis_color = [180u8, 180, 180];

        // Draw horizontal grid lines every 10° with degree labels
        let mut deg = 0.0;
        while deg <= y_max {
            let py = margin_t + plot_h * (1.0 - (deg - y_min) / (y_max - y_min));
            let py_i = py as u32;
            if py_i > 0 && py_i < h {
                for x in (margin_l as u32)..(w - margin_r as u32) {
                    unsafe { canvas.as_mut().set_pixel(x, py_i, &grid_color) };
                }
                // Y-axis label
                let label = format!("{}", deg as u32);
                let text_x = margin_l - (label.len() as f64 * 12.0) - 4.0;
                draw_text(&mut canvas.as_mut(), text_x, py - 7.0, &label, label_color, 2);
            }
            deg += 10.0;
        }

        // Draw vertical grid lines at date boundaries, label every 3rd day
        let mut last_date = String::new();
        let mut date_count = 0u32;
        let n = all.len() as f64;
        for (i, entry) in all.iter().enumerate() {
            let date = &entry.datetime[..10]; // "YYYY-MM-DD"
            if date != last_date {
                last_date = date.to_string();
                let px = margin_l + plot_w * (i as f64 / (n - 1.0));
                let px_i = px as u32;
                if px_i > margin_l as u32 && px_i < w - margin_r as u32 {
                    // Vertical grid line
                    for y in (margin_t as u32)..((margin_t + plot_h) as u32) {
                        unsafe { canvas.as_mut().set_pixel(px_i, y, &grid_color) };
                    }
                    // Date label every 3 days
                    if date_count % 3 == 0 {
                        let date_label = format!("{}/{}", &date[5..7], &date[8..10]);
                        draw_text(
                            &mut canvas.as_mut(),
                            px - 12.0,
                            margin_t + plot_h + 8.0,
                            &date_label,
                            label_color,
                            2,
                        );
                    }
                }
                date_count += 1;
            }
        }

        // Plot data points
        for (i, entry) in all.iter().enumerate() {
            let show_moon = entry.moon_theta_limb.abs() <= entry.sun_theta_limb.abs();
            let angle = angles[i];

            let px = margin_l + plot_w * (i as f64 / (n - 1.0));
            let py = margin_t + plot_h * (1.0 - (angle - y_min) / (y_max - y_min));

            let color = if show_moon { COLOR_MOON } else { COLOR_SUN };
            draw_circle(&mut canvas.as_mut(), px, py, 3.0, color);
        }

        // Draw axes
        // Left axis
        for y in (margin_t as u32)..((margin_t + plot_h) as u32 + 1) {
            unsafe { canvas.as_mut().set_pixel(margin_l as u32, y, &axis_color) };
        }
        // Bottom axis
        for x in (margin_l as u32)..(w - margin_r as u32) {
            unsafe { canvas.as_mut().set_pixel(x, (margin_t + plot_h) as u32, &axis_color) };
        }

        let out = Path::new("/tmp/spacepaper_angle_graph.png");
        canvas.save(out);
        eprintln!("Saved angle graph to {} ({} entries, y_max={:.0}°)", out.display(), all.len(), y_max);

        Ok(())
    }

    fn render_graph(
        all: &[&ScanEntry],
        angles: &[f64],
        color: [u8; 3],
        colors: Option<&[[u8; 3]]>,
        title: &str,
        out_path: &Path,
    ) {
        let w: u32 = 1920;
        let h: u32 = 600;
        let margin_l: f64 = 60.0;
        let margin_r: f64 = 20.0;
        let margin_t: f64 = 30.0;
        let margin_b: f64 = 50.0;
        let plot_w = w as f64 - margin_l - margin_r;
        let plot_h = h as f64 - margin_t - margin_b;

        let y_max_raw = angles.iter().cloned().fold(0.0f64, f64::max).max(1.0);
        let y_max = (y_max_raw / 10.0).ceil() * 10.0;
        let y_min = 0.0;

        let mut canvas = Image::alloc(w, h).boxed();
        let bg = [30u8, 30, 30];
        for y in 0..h {
            for x in 0..w {
                unsafe { canvas.as_mut().set_pixel(x, y, &bg) };
            }
        }

        let label_color = [160u8, 160, 160];
        let grid_color = [80u8, 80, 80];
        let axis_color = [180u8, 180, 180];

        // Horizontal grid lines every 10°
        let mut deg = 0.0;
        while deg <= y_max {
            let py = margin_t + plot_h * (1.0 - (deg - y_min) / (y_max - y_min));
            let py_i = py as u32;
            if py_i > 0 && py_i < h {
                for x in (margin_l as u32)..(w - margin_r as u32) {
                    unsafe { canvas.as_mut().set_pixel(x, py_i, &grid_color) };
                }
                let label = format!("{}", deg as u32);
                let text_x = margin_l - (label.len() as f64 * 12.0) - 4.0;
                draw_text(&mut canvas.as_mut(), text_x, py - 7.0, &label, label_color, 2);
            }
            deg += 10.0;
        }

        // Vertical grid lines at date boundaries, label every 3 days
        let mut last_date = String::new();
        let mut date_count = 0u32;
        let n = all.len() as f64;
        for (i, entry) in all.iter().enumerate() {
            let date = &entry.datetime[..10];
            if date != last_date {
                last_date = date.to_string();
                let px = margin_l + plot_w * (i as f64 / (n - 1.0));
                let px_i = px as u32;
                if px_i > margin_l as u32 && px_i < w - margin_r as u32 {
                    for y in (margin_t as u32)..((margin_t + plot_h) as u32) {
                        unsafe { canvas.as_mut().set_pixel(px_i, y, &grid_color) };
                    }
                    if date_count % 3 == 0 {
                        let date_label = format!("{}/{}", &date[5..7], &date[8..10]);
                        draw_text(
                            &mut canvas.as_mut(),
                            px - 12.0,
                            margin_t + plot_h + 8.0,
                            &date_label,
                            label_color,
                            2,
                        );
                    }
                }
                date_count += 1;
            }
        }

        // Plot data points
        for (i, _entry) in all.iter().enumerate() {
            let angle = angles[i];
            let px = margin_l + plot_w * (i as f64 / (n - 1.0));
            let py = margin_t + plot_h * (1.0 - (angle - y_min) / (y_max - y_min));
            let c = colors.map_or(color, |cs| cs[i]);
            draw_circle(&mut canvas.as_mut(), px, py, 3.0, c);
        }

        // Axes
        for y in (margin_t as u32)..((margin_t + plot_h) as u32 + 1) {
            unsafe { canvas.as_mut().set_pixel(margin_l as u32, y, &axis_color) };
        }
        for x in (margin_l as u32)..(w - margin_r as u32) {
            unsafe { canvas.as_mut().set_pixel(x, (margin_t + plot_h) as u32, &axis_color) };
        }

        canvas.save(out_path);
        eprintln!("Saved {} to {} ({} entries, y_max={:.0}°)", title, out_path.display(), all.len(), y_max);
    }

    #[test]
    fn test_render_body_graphs() -> Result<()> {
        use crate::orbital::HourlyDataset;

        let data_path = Path::new("test_data/hourly_30d.json");
        if !data_path.exists() {
            eprintln!("No test_data/hourly_30d.json — run collect_test_api_data first");
            return Ok(());
        }

        let dataset = HourlyDataset::load(data_path)?;

        // For each hour, find the satellite with closest Moon and closest Sun
        let datetimes: Vec<String> = dataset.entries.iter().map(|e| e.datetime.clone()).collect();

        let moon_best: Vec<_> = dataset.entries.iter().map(|e| {
            e.data.satellites.iter()
                .min_by(|a, b| {
                    let da = a.moon_horizontal.hypot(a.moon_vertical);
                    let db = b.moon_horizontal.hypot(b.moon_vertical);
                    da.partial_cmp(&db).unwrap()
                })
                .unwrap()
        }).collect();

        let sun_best: Vec<_> = dataset.entries.iter().map(|e| {
            e.data.satellites.iter()
                .min_by(|a, b| {
                    let da = a.sun_horizontal.hypot(a.sun_vertical);
                    let db = b.sun_horizontal.hypot(b.sun_vertical);
                    da.partial_cmp(&db).unwrap()
                })
                .unwrap()
        }).collect();

        let moon_angles: Vec<f64> = moon_best.iter()
            .map(|s| s.moon_horizontal.hypot(s.moon_vertical))
            .collect();
        let sun_angles: Vec<f64> = sun_best.iter()
            .map(|s| s.sun_horizontal.hypot(s.sun_vertical))
            .collect();

        let moon_colors: Vec<[u8; 3]> = moon_best.iter()
            .map(|s| satellite_color(s.satellite))
            .collect();
        let sun_colors: Vec<[u8; 3]> = sun_best.iter()
            .map(|s| satellite_color(s.satellite))
            .collect();

        // Build minimal ScanEntries just for the graph's date extraction
        let stubs: Vec<ScanEntry> = datetimes.iter().map(|dt| ScanEntry {
            datetime: dt.clone(),
            winner: crate::config::Satellite::GOESEast,
            tier: orbital::ViewTier::EarthOnly,
            moon_theta_limb: 0.0, moon_horizontal: 0.0, moon_vertical: 0.0, moon_visible: false,
            sun_theta_limb: 0.0, sun_horizontal: 0.0, sun_vertical: 0.0, sun_visible: false,
            moon_phase: 0.0, moon_distance: 0.0, score: 0.0,
            sat_dir: [0.0; 3], moon_pos: [0.0; 3], sun_dir: [0.0; 3],
        }).collect();
        let stub_refs: Vec<&ScanEntry> = stubs.iter().collect();

        render_graph(&stub_refs, &moon_angles, COLOR_MOON, Some(&moon_colors), "Moon angle", Path::new("/tmp/spacepaper_moon_graph.png"));
        render_graph(&stub_refs, &sun_angles, COLOR_SUN, Some(&sun_colors), "Sun angle", Path::new("/tmp/spacepaper_sun_graph.png"));

        Ok(())
    }

    #[test]
    fn test_render_preview_video() -> Result<()> {
        let scan_path = Path::new("scan_results.json");
        if !scan_path.exists() {
            eprintln!("No scan_results.json found — run scan_best_view test first");
            return Ok(());
        }

        let scan = load_scan(scan_path)?;

        // Collect all entries sorted by datetime (chronological)
        let mut all: Vec<&ScanEntry> = std::iter::once(&scan.best)
            .chain(scan.entries.iter())
            .collect();
        all.sort_by(|a, b| a.datetime.cmp(&b.datetime));

        let frames_dir = Path::new("/tmp/spacepaper_frames");
        std::fs::create_dir_all(frames_dir)?;

        for (i, entry) in all.iter().enumerate() {
            let img = render_placeholder(entry, 1920, 1080);
            let frame_path = frames_dir.join(format!("frame_{:04}.png", i));
            img.save(&frame_path);
        }

        eprintln!("Rendered {} frames to {}", all.len(), frames_dir.display());

        // Encode to mp4 with ffmpeg (4 fps = 24 second video for 96 frames)
        let output = Path::new("/tmp/spacepaper_preview.mp4");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-framerate", "4",
                "-i", &frames_dir.join("frame_%04d.png").to_string_lossy(),
                "-c:v", "libx264",
                "-pix_fmt", "yuv420p",
                "-crf", "18",
                output.to_str().unwrap(),
            ])
            .status()
            .context("Failed to run ffmpeg")?;

        assert!(status.success(), "ffmpeg failed");
        eprintln!("Saved preview video to {}", output.display());

        Ok(())
    }
}
