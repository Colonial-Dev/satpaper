use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::Satellite;

// --- Constants ---

const R_EARTH_KM: f64 = 6_371.0;
const R_GEO_KM: f64 = 42_164.0;
const T_SIDEREAL_DAY_HOURS: f64 = 23.9345;
const T_YEAR_HOURS: f64 = 8_766.0;
const GMST_AT_J2000_DEG: f64 = 280.46061837;
const OBLIQUITY_DEG: f64 = 23.4397;
const AU_KM: f64 = 149_597_870.7;


// --- Data Structures ---

#[derive(Debug)]
struct MoonData {
    pos: [f64; 3],
    phase: f64,
    distance: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ViewTier {
    AllThree = 1,  // Earth + Moon + Sun
    EarthMoon = 2,
    EarthSun = 3,
    EarthOnly = 4,
}

#[derive(Debug)]
pub struct SatelliteResult {
    pub satellite: Satellite,
    pub sat_pos: [f64; 3],
    pub moon_theta_limb: f64,
    pub moon_theta_em: f64,
    pub moon_visible: bool,
    pub moon_horizontal: f64, // degrees, positive = east (right)
    pub moon_vertical: f64,   // degrees, positive = north (up)
    pub sun_theta_limb: f64,
    pub sun_theta_em: f64,
    pub sun_visible: bool,
    pub sun_horizontal: f64,
    pub sun_vertical: f64,
    pub tier: ViewTier,
}

#[derive(Debug)]
pub struct OrreryData {
    pub winner: Satellite,
    pub moon_pos: [f64; 3],
    pub moon_phase: f64,
    pub moon_distance: f64,
    pub sun_dir: [f64; 3],
    pub satellites: Vec<SatelliteResult>,
}

// --- API Response ---

#[derive(Deserialize)]
struct DialAMoonResponse {
    j2000_ra: f64,
    j2000_dec: f64,
    distance: f64,
    phase: f64,
}

// --- Functions ---

fn fetch_moon(datetime: &str) -> Result<MoonData> {
    let url = format!("https://svs.gsfc.nasa.gov/api/dialamoon/{datetime}");
    let reader = ureq::get(&url)
        .call()
        .context("Failed to call Dial-a-Moon API")?
        .into_reader();

    let data: DialAMoonResponse =
        serde_json::from_reader(reader).context("Failed to deserialize moon data")?;

    let alpha = data.j2000_ra.to_radians();
    let delta = data.j2000_dec.to_radians();
    let d = data.distance;

    let pos = [
        d * delta.cos() * alpha.cos(),
        d * delta.cos() * alpha.sin(),
        d * delta.sin(),
    ];

    Ok(MoonData {
        pos,
        phase: data.phase,
        distance: d,
    })
}

fn satellite_longitude(sat: Satellite) -> f64 {
    use Satellite::*;
    match sat {
        GOESEast => -75.2,
        GOESWest => -137.2,
        Himawari => 140.7,
        Meteosat9 => 45.5,
        Meteosat10 => 0.0,
    }
}

fn satellite_eci(sat: Satellite, hours_since_j2000: f64) -> [f64; 3] {
    let gmst_deg =
        GMST_AT_J2000_DEG + (360.0 / T_SIDEREAL_DAY_HOURS) * hours_since_j2000;
    let theta_deg = satellite_longitude(sat) + gmst_deg;
    let theta = theta_deg.to_radians();

    [R_GEO_KM * theta.cos(), R_GEO_KM * theta.sin(), 0.0]
}

fn sun_eci(hours_since_j2000: f64) -> [f64; 3] {
    let theta = 2.0 * std::f64::consts::PI * hours_since_j2000 / T_YEAR_HOURS;
    let eps = OBLIQUITY_DEG.to_radians();
    [theta.cos(), theta.sin() * eps.cos(), theta.sin() * eps.sin()]
}

fn dot(a: &[f64; 3], b: &[f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn mag(v: &[f64; 3]) -> f64 {
    dot(v, v).sqrt()
}

fn cross(a: &[f64; 3], b: &[f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: &[f64; 3]) -> [f64; 3] {
    let m = mag(v);
    [v[0] / m, v[1] / m, v[2] / m]
}

/// Decompose a body's position into horizontal and vertical angles in the satellite's frame.
/// Boresight points toward Earth center. "Up" is north celestial pole projected
/// perpendicular to boresight. Returns (horizontal_deg, vertical_deg).
/// Positive vertical = body is north of Earth's equatorial plane.
/// Positive horizontal = body is to the east (right).
fn camera_frame_angles(sat_pos: &[f64; 3], body_pos: &[f64; 3]) -> (f64, f64) {
    let boresight = normalize(&[-sat_pos[0], -sat_pos[1], -sat_pos[2]]);
    let north: [f64; 3] = [0.0, 0.0, 1.0];
    let right = normalize(&cross(&boresight, &north));
    let up = cross(&right, &boresight);

    let to_body = [
        body_pos[0] - sat_pos[0],
        body_pos[1] - sat_pos[1],
        body_pos[2] - sat_pos[2],
    ];

    let depth = dot(&to_body, &boresight);
    let h = dot(&to_body, &right);
    let v = dot(&to_body, &up);

    (h.atan2(depth).to_degrees(), v.atan2(depth).to_degrees())
}

/// Compute angular distance from Earth's limb for a body as seen from the satellite.
/// Returns (theta_em, theta_limb, visible).
/// theta_em = angle between sat-to-body and sat-to-Earth-center vectors (degrees).
/// theta_limb = theta_em - alpha_earth (degrees). Positive = body outside Earth's disk.
fn body_visibility(sat_pos: &[f64; 3], body_pos: &[f64; 3]) -> (f64, f64, bool) {
    let l = [
        body_pos[0] - sat_pos[0],
        body_pos[1] - sat_pos[1],
        body_pos[2] - sat_pos[2],
    ];
    let e = [-sat_pos[0], -sat_pos[1], -sat_pos[2]];

    let cos_theta_em = dot(&l, &e) / (mag(&l) * mag(&e));
    let theta_em = cos_theta_em.clamp(-1.0, 1.0).acos().to_degrees();

    let alpha_earth = (R_EARTH_KM / mag(&e)).asin().to_degrees();
    let theta_limb = theta_em - alpha_earth;

    (theta_em, theta_limb, theta_limb > 0.0)
}

fn parse_hours_since_j2000(datetime: &str) -> Result<f64> {
    use chrono::{NaiveDateTime, NaiveDate};

    let dt = if datetime.len() <= 16 {
        NaiveDateTime::parse_from_str(datetime, "%Y-%m-%dT%H:%M")
            .context("Failed to parse datetime (expected YYYY-MM-DDTHH:MM)")?
    } else {
        NaiveDateTime::parse_from_str(datetime, "%Y-%m-%dT%H:%M:%S")
            .context("Failed to parse datetime")?
    };

    let j2000 = NaiveDate::from_ymd_opt(2000, 1, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap();

    let duration = dt.signed_duration_since(j2000);
    Ok(duration.num_seconds() as f64 / 3600.0)
}

const ALL_SATELLITES: [Satellite; 5] = [
    Satellite::GOESEast,
    Satellite::GOESWest,
    Satellite::Himawari,
    Satellite::Meteosat9,
    Satellite::Meteosat10,
];

pub fn closest_satellite(
    utc_datetime: &str,
    theta_limb_max: Option<f64>,
) -> Result<OrreryData> {
    let moon = fetch_moon(utc_datetime)?;
    let hours = parse_hours_since_j2000(utc_datetime)?;
    let sun_dir = sun_eci(hours);

    let mut results: Vec<SatelliteResult> = Vec::with_capacity(5);

    for &sat in &ALL_SATELLITES {
        let sat_pos = satellite_eci(sat, hours);

        // Moon visibility from satellite
        let (moon_theta_em, moon_theta_limb, moon_visible) =
            body_visibility(&sat_pos, &moon.pos);

        // Sun visibility from satellite
        let sun_pos = [sun_dir[0] * AU_KM, sun_dir[1] * AU_KM, sun_dir[2] * AU_KM];
        let (sun_theta_em, sun_theta_limb, sun_visible) =
            body_visibility(&sat_pos, &sun_pos);

        // Camera-frame angles (horizontal/vertical decomposition)
        let (moon_horizontal, moon_vertical) = camera_frame_angles(&sat_pos, &moon.pos);
        let (sun_horizontal, sun_vertical) = camera_frame_angles(&sat_pos, &sun_pos);

        let tier = match (moon_visible, sun_visible) {
            (true, true) => ViewTier::AllThree,
            (true, false) => ViewTier::EarthMoon,
            (false, true) => ViewTier::EarthSun,
            (false, false) => ViewTier::EarthOnly,
        };

        results.push(SatelliteResult {
            satellite: sat,
            sat_pos,
            moon_theta_limb,
            moon_theta_em,
            moon_visible,
            moon_horizontal,
            moon_vertical,
            sun_theta_limb,
            sun_theta_em,
            sun_visible,
            sun_horizontal,
            sun_vertical,
            tier,
        });
    }

    let winner = select_winner(&results, theta_limb_max);

    Ok(OrreryData {
        winner,
        moon_pos: moon.pos,
        moon_phase: moon.phase,
        moon_distance: moon.distance,
        sun_dir,
        satellites: results,
    })
}

fn select_winner(results: &[SatelliteResult], theta_limb_max: Option<f64>) -> Satellite {
    let score = |r: &SatelliteResult| -> (ViewTier, f64) {
        let moon_score = r.moon_theta_limb.abs();
        let sun_score = r.sun_theta_limb.abs();
        (r.tier, moon_score + 0.5 * sun_score)
    };

    let filtered: Vec<&SatelliteResult> = if let Some(max) = theta_limb_max {
        results
            .iter()
            .filter(|r| r.moon_theta_limb <= max)
            .collect()
    } else {
        results.iter().collect()
    };

    let candidates = if filtered.is_empty() { results.iter().collect::<Vec<_>>() } else { filtered };

    candidates
        .iter()
        .min_by(|a, b| {
            let sa = score(a);
            let sb = score(b);
            sa.0.cmp(&sb.0)
                .then(sa.1.partial_cmp(&sb.1).unwrap())
        })
        .unwrap()
        .satellite
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_closest_satellite() -> Result<()> {
        let data = closest_satellite("2026-03-14T17:00", None)?;

        log::info!("Winner: {:?}", data.winner);
        log::info!("Moon phase: {}", data.moon_phase);
        log::info!("Moon distance: {} km", data.moon_distance);
        log::info!("Sun direction (ECI): [{:.4}, {:.4}, {:.4}]",
            data.sun_dir[0], data.sun_dir[1], data.sun_dir[2]);

        for sat in &data.satellites {
            log::info!(
                "{:?}: tier={:?}, moon(limb={:.2}°, em={:.2}°, vis={}, h={:.2}°, v={:.2}°), sun(limb={:.2}°, em={:.2}°, vis={}, h={:.2}°, v={:.2}°)",
                sat.satellite,
                sat.tier,
                sat.moon_theta_limb,
                sat.moon_theta_em,
                sat.moon_visible,
                sat.moon_horizontal,
                sat.moon_vertical,
                sat.sun_theta_limb,
                sat.sun_theta_em,
                sat.sun_visible,
                sat.sun_horizontal,
                sat.sun_vertical,
            );
        }

        // Sanity checks
        assert!(data.moon_distance > 350_000.0 && data.moon_distance < 410_000.0);
        assert!(data.satellites.len() == 5);

        for sat in &data.satellites {
            assert!(sat.moon_theta_em >= 0.0 && sat.moon_theta_em <= 180.0);
            assert!(sat.sun_theta_em >= 0.0 && sat.sun_theta_em <= 180.0);
            assert!(sat.moon_vertical.abs() <= 180.0);
            assert!(sat.moon_horizontal.abs() <= 180.0);
        }

        Ok(())
    }
}
