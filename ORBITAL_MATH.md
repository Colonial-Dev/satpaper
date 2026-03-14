# Orbital Math

## Geostationary Satellite Longitudes

All satellites orbit at the geostationary altitude (~35,786 km) above the equator (0° latitude).

| Satellite | Designation | Longitude | Operator |
|-----------|-------------|-----------|----------|
| GOES-East | GOES-16 | 75.2° W | NOAA |
| GOES-West | GOES-18 | 137.2° W | NOAA |
| Himawari | Himawari-9 | 140.7° E | JMA |
| Meteosat 9 | Meteosat-9 | 45.5° E | EUMETSAT |
| Meteosat 10 | Meteosat-10 | 0.0° E | EUMETSAT |

## Coordinate System

We work in **Earth-Centered Inertial (ECI)** coordinates:
- Origin at Earth's center
- X-axis points toward the vernal equinox (♈)
- Z-axis points toward celestial north pole
- Y-axis completes the right-handed system

All positions are 3D vectors `[x, y, z]` in kilometers.

## Constants

### Distances

| Symbol | Value | Description |
|--------|-------|-------------|
| R_earth | 6,371 km | Mean Earth radius |
| R_geo | 42,164 km | Geostationary orbit radius (from Earth center) |
| R_moon | 384,400 km | Mean lunar orbital radius |
| 1 AU | 149,597,870.7 km | Earth–Sun distance |

### Time Periods

| Symbol | Value | Description |
|--------|-------|-------------|
| T_sidereal_day | 23.9345 hours | Earth rotation period (relative to stars) |
| T_sidereal_month | 655.7208 hours | Moon orbital period (relative to stars) |
| T_synodic_month | 708.7344 hours | Moon phase cycle (new moon to new moon) |
| T_year | 8,766.0 hours | Solar year |

### Angles

| Symbol | Value | Description |
|--------|-------|-------------|
| ε | 23.4397° | Earth's axial tilt (obliquity of the ecliptic) |
| i_moon | 5.145° | Moon's orbital inclination to the ecliptic |

## Position Models

All models take time `t` in **hours** from an arbitrary epoch. We use circular orbit approximations throughout — no eccentricity, no perturbations.

### Sun Direction

The Sun's position relative to Earth is modeled as a unit direction rotating once per year:

```
θ_sun(t) = (2π / T_year) · t

sun_dir = [ cos(θ_sun), sin(θ_sun), 0 ]
```

The Sun is effectively at infinity for shadow/illumination calculations. When a true position is needed, scale by 1 AU:

```
P_sun(t) = AU · sun_dir
```

Note: The ecliptic is the reference plane here. For the Sun's direction in ECI, no obliquity rotation is needed since the Sun stays in the ecliptic plane by definition.

### Earth

Earth is fixed at the origin. Its rotation is tracked via Greenwich Sidereal Angle:

```
θ_GSA(t) = (2π / T_sidereal_day) · t
```

This angle maps geographic longitude to the inertial frame and is essential for placing geostationary satellites.

### Geostationary Satellites

Each satellite has a fixed geographic longitude `λ`. In ECI, its position rotates with Earth:

```
θ(t) = λ + θ_GSA(t)
     = λ + (2π / T_sidereal_day) · t

P_sat(t) = [ R_geo · cos(θ),  R_geo · sin(θ),  0 ]
```

The Z-component is zero because geostationary orbits are equatorial (inclination ≈ 0°).

Satellite longitudes (as signed values for the formula):

| Satellite | λ (degrees) |
|-----------|-------------|
| GOES-East | −75.2° |
| GOES-West | −137.2° |
| Himawari-9 | +140.7° |
| Meteosat-9 | +45.5° |
| Meteosat-10 | +0.0° |

### Moon

The Moon's orbit is inclined 5.145° to the ecliptic, with a **precessing ascending node** (period ≈ 18.6 years). The model applies three rotations to a base circular orbit:

**Step 1 — Circular orbit in the orbital plane:**

```
M(t) = (2π / T_sidereal_month) · t

x₀ = R_moon · cos(M)
y₀ = R_moon · sin(M)
```

**Step 2 — Apply inclination (rotate about X-axis by i_moon):**

```
x₁ = x₀
y₁ = y₀ · cos(i_moon)
z₁ = y₀ · sin(i_moon)
```

**Step 3 — Apply ascending node rotation (rotate about Z-axis by Ω):**

```
x₂ = x₁ · cos(Ω) − y₁ · sin(Ω)
y₂ = x₁ · sin(Ω) + y₁ · cos(Ω)
z₂ = z₁
```

Where `Ω` is the longitude of the ascending node, which precesses through 360° over ~18.6 years.

**Step 4 — Apply ecliptic obliquity (rotate about X-axis by ε):**

This transforms from ecliptic coordinates to ECI (equatorial):

```
x_ECI = x₂
y_ECI = y₂ · cos(ε) − z₂ · sin(ε)
z_ECI = y₂ · sin(ε) + z₂ · cos(ε)

P_moon(t) = [ x_ECI, y_ECI, z_ECI ]
```

## Derived Quantities

### Earth's Angular Radius from GEO

How large does Earth appear from a geostationary satellite?

```
α_earth = arcsin(R_earth / R_geo) ≈ 8.7° (half-angle)
```

### Line-of-Sight from Satellite to Moon

Given satellite position `S` and moon position `M`:

```
L = M − S                          (vector from satellite to Moon)
e_z = −S / |S|                     (unit vector toward Earth center)
```

Construct an orthonormal frame `(e_x, e_y, e_z)` at the satellite. Project `L` onto this frame to get angular offsets. The Moon is:

- **Behind the satellite** if `L · e_z < 0`
- **Occluded by Earth** if angular offset from Earth center < α_earth

## Moon Position from NASA API

Instead of computing the Moon's position from our simplified circular model, we fetch it from the NASA SVS Dial-a-Moon API, which provides precise J2000 ephemeris data:

```
GET https://svs.gsfc.nasa.gov/api/dialamoon/{YYYY}-{MM}-{DD}T{HH}:{MM}
```

Response includes:
- `j2000_ra` — Right Ascension in degrees
- `j2000_dec` — Declination in degrees
- `distance` — Earth–Moon distance in km

### Converting RA/Dec to ECI

J2000 RA/Dec maps directly to our ECI frame (both are Earth-centered equatorial J2000):

```
α = j2000_ra  (converted to radians)
δ = j2000_dec (converted to radians)
d = distance  (km)

P_moon = [ d · cos(δ) · cos(α),
           d · cos(δ) · sin(α),
           d · sin(δ) ]
```

## Closest Satellite Function

### Goal

Given a UTC datetime, determine which geostationary satellite sees the Moon nearest to Earth's limb — i.e., the smallest angular distance between the Moon and the edge of Earth's disk, as observed from the satellite.

### Algorithm: `closest_satellite(utc_datetime)`

**Step 1 — Get Moon position in ECI:**

Fetch from NASA API and convert RA/Dec/distance to ECI vector `P_moon`.

**Step 2 — Compute elapsed hours since J2000 epoch:**

The J2000 epoch is 2000-01-01T12:00:00 UTC.

```
t = (utc_datetime − J2000_epoch) in hours
```

**Step 3 — Compute Greenwich Sidereal Angle:**

We need `θ_GSA(t)` to place the satellites in ECI. Using the standard approximation:

```
θ_GSA(t) = (2π / T_sidereal_day) · t
```

More precisely, GMST at J2000 epoch is 280.46061837°, so:

```
θ_GSA(t) = 280.46061837° + (360° / T_sidereal_day) · t
```

(reduced modulo 360°)

**Step 4 — Compute each satellite's ECI position:**

For each satellite with longitude `λ`:

```
θ = λ + θ_GSA(t)
P_sat = [ R_geo · cos(θ),  R_geo · sin(θ),  0 ]
```

**Step 5 — Compute angular distance from Earth's limb to Moon (as seen from satellite):**

From the satellite at position `S`, compute:

```
L = P_moon − S              (vector from satellite to Moon)
E = −S                      (vector from satellite to Earth center)

                 L · E
cos(θ_EM) = ─────────────   (angle between Earth center and Moon)
              |L| · |E|

θ_EM = arccos(cos(θ_EM))    (angular separation: Earth center ↔ Moon)
```

The Moon's angular distance from Earth's **limb** (not center) is:

```
α_earth = arcsin(R_earth / |E|)     (Earth's angular radius from this satellite)

θ_limb = θ_EM − α_earth             (angular distance from Earth's edge to Moon)
```

- If `θ_limb < 0`, the Moon is behind Earth (occluded)
- If `θ_limb > 90°`, the Moon is far from Earth in the satellite's FOV

**Step 6 — Select the winner:**

The function accepts an optional `θ_limb_max` parameter (default: no limit). Selection logic:

1. **Visible candidates**: satellites where `0 ≤ θ_limb ≤ θ_limb_max`
2. **Occluded candidates**: satellites where `θ_limb < 0` (Moon behind Earth)

```
if any visible candidates exist:
    pick the one with the smallest θ_limb        (Moon closest to Earth's limb)
else if θ_limb_max is set and any occluded candidates exist:
    pick the one with the largest θ_limb          (least occluded, closest to limb from behind)
else:
    pick the satellite with the smallest |θ_limb| (closest to limb either way)
```

The idea: when every satellite sees the Moon too far from Earth to frame them together, prefer an occluded view where the Moon is *about to emerge* from behind Earth — still a compelling image.

### Output

The function returns data for the winning satellite, plus all positions needed to draw a top-down orrery:

```
{
  // Winner info
  satellite:       name of the winning satellite,
  θ_limb:          angular distance from Earth's limb to Moon (degrees),
  θ_EM:            angular distance from Earth's center to Moon (degrees),
  occluded:        true if θ_limb < 0,

  // ECI positions (km) — for orrery rendering
  moon_pos:        [x, y, z],          // Moon ECI position
  sun_dir:         [x, y],             // Unit vector toward Sun (ecliptic plane)
  satellites: {                        // All satellites, not just the winner
    "GOES-East":   { pos: [x, y, z], θ_limb, θ_EM, occluded },
    "GOES-West":   { pos: [x, y, z], θ_limb, θ_EM, occluded },
    "Himawari-9":  { pos: [x, y, z], θ_limb, θ_EM, occluded },
    "Meteosat-9":  { pos: [x, y, z], θ_limb, θ_EM, occluded },
    "Meteosat-10": { pos: [x, y, z], θ_limb, θ_EM, occluded },
  },

  // Moon metadata from API
  moon_phase:      phase angle (degrees),
  moon_distance:   Earth–Moon distance (km),
}
```

A top-down orrery can project the XY plane directly: Earth at origin, satellite dots on the GEO ring, Moon far out, Sun direction as an arrow. The Z component gives out-of-plane info if needed for a side view.

## Approximations & Limitations

1. **Circular orbits** — no eccentricity for any body
2. **No solar perturbations** on the Moon
3. **No satellite station-keeping drift** — fixed longitudes assumed
4. **No lunar apsidal precession** — only nodal precession modeled
5. **Sun at infinity** — parallax ignored for illumination
6. **Classical mechanics only** — no relativistic corrections
