import { useState, useEffect, useRef, useMemo } from "react";

// ═══════════════════════════════════════════════════════════
// CONSTANTS & ORBITAL MECHANICS
// ═══════════════════════════════════════════════════════════

const DEG = Math.PI / 180;
const RAD = 180 / Math.PI;

const R_GEO_KM = 42164;
const R_MOON_KM = 384400;
const R_EARTH_KM = 6371;
const AU_KM = 149597870.7;

const SIDEREAL_MONTH = 27.3217;
const SYNODIC_MONTH = 29.5306;
const SIDEREAL_DAY = 0.99727;
const YEAR_DAYS = 365.25;

const ECLIPTIC_OBL = 23.4397 * DEG;
const MOON_INCL = 5.145 * DEG;

const EARTH_ANG_RAD_DEG = Math.asin(R_EARTH_KM / R_GEO_KM) * RAD;

const SATELLITES = [
  { name: "GOES-East", lon: -75.2, color: "#ef4444", abbr: "GE" },
  { name: "GOES-West", lon: -137.0, color: "#f97316", abbr: "GW" },
  { name: "Himawari-9", lon: 140.7, color: "#22d3ee", abbr: "HW" },
  { name: "Meteosat-10", lon: 0.0, color: "#a78bfa", abbr: "M10" },
  { name: "Meteosat-9", lon: 45.5, color: "#34d399", abbr: "M9" },
];

// ── 3D orbital mechanics (same as before, projected to top-down XY) ──

function moonECI(tDays, nodeOmegaDeg) {
  const w = (2 * Math.PI) / SIDEREAL_MONTH;
  const M = w * tDays;
  const Om = nodeOmegaDeg * DEG;
  const x0 = R_MOON_KM * Math.cos(M);
  const y0 = R_MOON_KM * Math.sin(M);
  const im = MOON_INCL;
  const x1 = x0, y1 = y0 * Math.cos(im), z1 = y0 * Math.sin(im);
  const xE = x1 * Math.cos(Om) - y1 * Math.sin(Om);
  const yE = x1 * Math.sin(Om) + y1 * Math.cos(Om);
  const zE = z1;
  const eps = ECLIPTIC_OBL;
  return [xE, yE * Math.cos(eps) - zE * Math.sin(eps), yE * Math.sin(eps) + zE * Math.cos(eps)];
}

function satECI(tDays, lonDeg) {
  const wE = (2 * Math.PI) / SIDEREAL_DAY;
  const angle = lonDeg * DEG + wE * tDays;
  return [R_GEO_KM * Math.cos(angle), R_GEO_KM * Math.sin(angle), 0];
}

function moonFromSat3D(tDays, lonDeg, nodeOmegaDeg) {
  const rm = moonECI(tDays, nodeOmegaDeg);
  const rs = satECI(tDays, lonDeg);
  const L = [rm[0] - rs[0], rm[1] - rs[1], rm[2] - rs[2]];
  const rsMag = Math.sqrt(rs[0] ** 2 + rs[1] ** 2 + rs[2] ** 2);
  const ez = [-rs[0] / rsMag, -rs[1] / rsMag, -rs[2] / rsMag];
  const nDotEz = ez[2];
  let ey = [-nDotEz * ez[0], -nDotEz * ez[1], 1 - nDotEz * ez[2]];
  const eyM = Math.sqrt(ey[0] ** 2 + ey[1] ** 2 + ey[2] ** 2);
  ey = [ey[0] / eyM, ey[1] / eyM, ey[2] / eyM];
  const ex = [ez[1] * ey[2] - ez[2] * ey[1], ez[2] * ey[0] - ez[0] * ey[2], ez[0] * ey[1] - ez[1] * ey[0]];
  const Lx = L[0] * ex[0] + L[1] * ex[1] + L[2] * ex[2];
  const Ly = L[0] * ey[0] + L[1] * ey[1] + L[2] * ey[2];
  const Lz = L[0] * ez[0] + L[1] * ez[1] + L[2] * ez[2];
  const ax = Math.atan2(Lx, Lz) * RAD;
  const ay = Math.atan2(Ly, Lz) * RAD;
  const angOffset = Math.sqrt(ax * ax + ay * ay);
  const behind = Lz < 0;
  const occluded = !behind && angOffset < EARTH_ANG_RAD_DEG;
  return { ax, ay, angOffset, behind, occluded };
}

function viewScore(p) {
  if (p.behind || p.occluded) return Infinity;
  return p.angOffset;
}

// ═══════════════════════════════════════════════════════════
// DISPLAY SCALES
// ═══════════════════════════════════════════════════════════
// We need to show: sun (far away), earth, GEO ring, moon orbit
// Strategy: two nested coordinate systems
// Inner view: Earth + GEO ring + moon orbit (true scale ratios, just scaled)
// Sun: shown as direction indicator off-screen with rays

const CANVAS = 700;
const CX = CANVAS / 2;
const CY = CANVAS / 2;

// Scale: 1px = how many km?
// Moon orbit radius in px: ~260px → scale = 384400/260 ≈ 1478 km/px
const MOON_ORBIT_PX = 240;
const KM_PER_PX = R_MOON_KM / MOON_ORBIT_PX;
const EARTH_PX = Math.max(R_EARTH_KM / KM_PER_PX, 5);
const GEO_PX = R_GEO_KM / KM_PER_PX; // ~28px

// ═══════════════════════════════════════════════════════════
// COMPONENT
// ═══════════════════════════════════════════════════════════

export default function OrreryView() {
  const [nodeOmega, setNodeOmega] = useState(0);
  const [currentDay, setCurrentDay] = useState(0);
  const [animating, setAnimating] = useState(true);
  const [speed, setSpeed] = useState(1);
  const [showTrail, setShowTrail] = useState(true);
  const [showLoS, setShowLoS] = useState(true);
  const animRef = useRef(null);
  const lastTRef = useRef(null);

  const totalDays = SYNODIC_MONTH;

  useEffect(() => {
    if (!animating) { if (animRef.current) cancelAnimationFrame(animRef.current); return; }
    lastTRef.current = null;
    const tick = (ts) => {
      if (!lastTRef.current) lastTRef.current = ts;
      const dt = (ts - lastTRef.current) / 1000;
      lastTRef.current = ts;
      setCurrentDay(d => { let n = d + dt * speed * 0.4; return n > totalDays ? n % totalDays : n; });
      animRef.current = requestAnimationFrame(tick);
    };
    animRef.current = requestAnimationFrame(tick);
    return () => { if (animRef.current) cancelAnimationFrame(animRef.current); };
  }, [animating, speed, totalDays]);

  // ── Earth position (slight drift in its solar orbit over the month) ──
  // Earth's orbital angle changes ~1°/day. We keep Earth at center and show sun direction.
  const earthSolarAngle = (2 * Math.PI / YEAR_DAYS) * currentDay; // Earth's orbital progress

  // Sun direction (opposite of Earth's orbital position = sun is "that way")
  // In our ECI frame, at t=0 let's say sun is along +X
  // Actually for top-down view, sun direction doesn't change much over a month
  // Let's put sun at a fixed direction for simplicity, slight drift
  const sunAngle = Math.PI + earthSolarAngle; // direction FROM earth TO sun in ECI
  const sunDirX = Math.cos(sunAngle);
  const sunDirY = Math.sin(sunAngle);

  // ── Moon position (top-down = XY of ECI) ──
  const moonXYZ = moonECI(currentDay, nodeOmega);
  const moonPx = CX + moonXYZ[0] / KM_PER_PX;
  const moonPy = CY - moonXYZ[1] / KM_PER_PX; // flip Y for screen coords

  // ── Satellite positions ──
  const satPositions = SATELLITES.map(s => {
    const pos = satECI(currentDay, s.lon);
    return {
      px: CX + pos[0] / KM_PER_PX,
      py: CY - pos[1] / KM_PER_PX,
      eciX: pos[0],
      eciY: pos[1],
    };
  });

  // ── View quality from each satellite ──
  const satViews = SATELLITES.map(s => moonFromSat3D(currentDay, s.lon, nodeOmega));
  const satScores = satViews.map(v => viewScore(v));
  const bestIdx = satScores.reduce((bi, s, i) => s < satScores[bi] ? i : bi, 0);
  const bestSat = satScores[bestIdx] === Infinity ? null : bestIdx;

  // ── Moon trail ──
  const trailPoints = useMemo(() => {
    const pts = [];
    const steps = 300;
    for (let i = 0; i <= steps; i++) {
      const t = (i / steps) * totalDays;
      const m = moonECI(t, nodeOmega);
      pts.push({ x: CX + m[0] / KM_PER_PX, y: CY - m[1] / KM_PER_PX, t });
    }
    return pts;
  }, [nodeOmega, totalDays]);

  // ── Sun rays (decorative, from edge of canvas) ──
  const sunEdgeX = CX + sunDirX * (CANVAS / 2 + 20);
  const sunEdgeY = CY - sunDirY * (CANVAS / 2 + 20);
  const sunLabelX = CX + sunDirX * (CANVAS / 2 - 30);
  const sunLabelY = CY - sunDirY * (CANVAS / 2 - 30);

  // Build line-of-sight lines from each satellite toward moon
  // We'll draw them as lines from the satellite, past Earth's limb, toward the moon
  // Color-coded and dashed if occluded/behind

  const bestName = bestSat !== null ? SATELLITES[bestSat].name : "None";
  const bestColor = bestSat !== null ? SATELLITES[bestSat].color : "#555";
  const bestAngle = bestSat !== null ? satViews[bestSat].angOffset.toFixed(1) : "—";

  return (
    <div style={{
      background: "#06060c",
      minHeight: "100vh",
      color: "#9aa0ad",
      fontFamily: "'IBM Plex Mono', 'JetBrains Mono', monospace",
      padding: "12px",
      display: "flex",
      flexDirection: "column",
      alignItems: "center",
      gap: "10px",
    }}>
      <div style={{ textAlign: "center" }}>
        <h1 style={{
          fontSize: "13px", fontWeight: 500, letterSpacing: "2.5px",
          textTransform: "uppercase", color: "#5a6578", margin: "0 0 2px"
        }}>
          Orrery — Geostationary Moon Proximity
        </h1>
        <p style={{ fontSize: "9px", color: "#2e3848", margin: 0, letterSpacing: "1px" }}>
          Top-down view · North celestial pole toward viewer · Not to scale (GEO ring enlarged 3×)
        </p>
      </div>

      <svg
        width={CANVAS} height={CANVAS}
        viewBox={`0 0 ${CANVAS} ${CANVAS}`}
        style={{ background: "#030308", borderRadius: "6px", border: "1px solid #111320", maxWidth: "100%" }}
      >
        <defs>
          <radialGradient id="earthGlow">
            <stop offset="0%" stopColor="#2563eb" stopOpacity="0.12" />
            <stop offset="100%" stopColor="#2563eb" stopOpacity="0" />
          </radialGradient>
          <radialGradient id="moonGlow">
            <stop offset="0%" stopColor="#fef3c7" stopOpacity="0.5" />
            <stop offset="100%" stopColor="#fef3c7" stopOpacity="0" />
          </radialGradient>
          <radialGradient id="sunGlow">
            <stop offset="0%" stopColor="#fbbf24" stopOpacity="0.3" />
            <stop offset="100%" stopColor="#fbbf24" stopOpacity="0" />
          </radialGradient>
          {/* Earth shadow cone (simplified — anti-sun direction) */}
          <linearGradient id="shadowCone"
            x1={CX} y1={CY}
            x2={CX - sunDirX * 300} y2={CY + sunDirY * 300}
            gradientUnits="userSpaceOnUse"
          >
            <stop offset="0%" stopColor="#000" stopOpacity="0.25" />
            <stop offset="100%" stopColor="#000" stopOpacity="0" />
          </linearGradient>
        </defs>

        {/* ── Star field (subtle) ── */}
        {useMemo(() => {
          const stars = [];
          const rng = (seed) => {
            let s = seed;
            return () => { s = (s * 16807) % 2147483647; return (s - 1) / 2147483646; };
          };
          const r = rng(42);
          for (let i = 0; i < 120; i++) {
            stars.push(
              <circle key={i} cx={r() * CANVAS} cy={r() * CANVAS}
                r={r() < 0.3 ? 0.8 : 0.4} fill="#fff" opacity={0.05 + r() * 0.1} />
            );
          }
          return stars;
        }, [])}

        {/* ── Sun direction indicator ── */}
        <circle cx={sunEdgeX} cy={sunEdgeY} r={40} fill="url(#sunGlow)" />
        {/* Sun rays toward Earth */}
        {[-0.08, -0.04, 0, 0.04, 0.08].map((off, i) => {
          const a = sunAngle + off;
          const x1 = CX + Math.cos(a) * (CANVAS * 0.48);
          const y1 = CY - Math.sin(a) * (CANVAS * 0.48);
          return <line key={i} x1={x1} y1={y1} x2={CX} y2={CY}
            stroke="#fbbf24" strokeWidth={0.3} opacity={0.08} />;
        })}
        <text x={sunLabelX} y={sunLabelY} textAnchor="middle" fill="#fbbf24"
          fontSize="10" opacity={0.4} dominantBaseline="middle"
          transform={`rotate(${-sunAngle * RAD + 90}, ${sunLabelX}, ${sunLabelY})`}
        >
          ☀ Sun
        </text>

        {/* ── Earth shadow cone (decorative) ── */}
        <polygon
          points={`${CX - sunDirY * EARTH_PX},${CY - sunDirX * EARTH_PX} ${CX + sunDirY * EARTH_PX},${CY + sunDirX * EARTH_PX} ${CX - sunDirX * MOON_ORBIT_PX * 1.2 + sunDirY * EARTH_PX * 2},${CY + sunDirY * MOON_ORBIT_PX * 1.2 + sunDirX * EARTH_PX * 2} ${CX - sunDirX * MOON_ORBIT_PX * 1.2 - sunDirY * EARTH_PX * 2},${CY + sunDirY * MOON_ORBIT_PX * 1.2 - sunDirX * EARTH_PX * 2}`}
          fill="#000" opacity={0.15}
        />

        {/* ── Moon orbit path ── */}
        <circle cx={CX} cy={CY} r={MOON_ORBIT_PX}
          fill="none" stroke="#1a1f2e" strokeWidth={0.5} strokeDasharray="4,8" />

        {/* ── Moon trail ── */}
        {showTrail && (
          <polyline
            points={trailPoints.filter(p => p.t <= currentDay).map(p => `${p.x.toFixed(1)},${p.y.toFixed(1)}`).join(" ")}
            fill="none" stroke="#d4a017" strokeWidth={1} opacity={0.25}
          />
        )}

        {/* ── GEO orbit ring (enlarged 3× for visibility) ── */}
        <circle cx={CX} cy={CY} r={GEO_PX * 3}
          fill="none" stroke="#1e293b" strokeWidth={0.5} strokeDasharray="2,6" opacity={0.5} />

        {/* ── Lines of sight from satellites to moon ── */}
        {showLoS && satPositions.map((sp, si) => {
          const v = satViews[si];
          const isBest = bestSat === si;
          // Draw line from satellite toward moon
          // Satellite position is very close to Earth at this scale,
          // so we'll draw from slightly outside Earth toward moon
          const dx = moonPx - sp.px;
          const dy = moonPy - sp.py;
          const dist = Math.sqrt(dx * dx + dy * dy);
          if (dist < 1) return null;

          // Extend line from satellite position (enlarged) to moon
          const satDisplayX = CX + (sp.px - CX) * 3; // match GEO ring 3× scale
          const satDisplayY = CY + (sp.py - CY) * 3;

          const opacity = isBest ? 0.5 : 0.12;
          const width = isBest ? 1.5 : 0.5;
          const dash = v.behind ? "2,6" : v.occluded ? "3,4" : "none";

          return <line key={si}
            x1={satDisplayX} y1={satDisplayY}
            x2={moonPx} y2={moonPy}
            stroke={SATELLITES[si].color} strokeWidth={width}
            opacity={opacity} strokeDasharray={dash}
          />;
        })}

        {/* ── Earth ── */}
        <circle cx={CX} cy={CY} r={EARTH_PX * 3} fill="url(#earthGlow)" />
        <circle cx={CX} cy={CY} r={EARTH_PX}
          fill="#0f2744" stroke="#3b82f6" strokeWidth={0.8} />
        <text x={CX} y={CY + EARTH_PX + 12} textAnchor="middle"
          fill="#3b82f6" fontSize="8" opacity={0.5}>Earth</text>

        {/* ── GEO Satellites (drawn on enlarged ring) ── */}
        {satPositions.map((sp, si) => {
          const isBest = bestSat === si;
          const displayX = CX + (sp.px - CX) * 3;
          const displayY = CY + (sp.py - CY) * 3;
          const v = satViews[si];
          const visible = !v.behind && !v.occluded;

          return (
            <g key={si}>
              {isBest && <circle cx={displayX} cy={displayY} r={8}
                fill={SATELLITES[si].color} opacity={0.15} />}
              <circle cx={displayX} cy={displayY}
                r={isBest ? 4 : 2.5}
                fill={visible ? SATELLITES[si].color : SATELLITES[si].color + "40"}
                stroke={isBest ? "#fff" : "none"} strokeWidth={0.5}
              />
              <text x={displayX} y={displayY - (isBest ? 10 : 7)}
                textAnchor="middle" fill={SATELLITES[si].color}
                fontSize={isBest ? "9" : "7"} fontWeight={isBest ? 700 : 400}
                opacity={isBest ? 1 : 0.6}
              >
                {SATELLITES[si].abbr}
              </text>
            </g>
          );
        })}

        {/* ── Moon ── */}
        <circle cx={moonPx} cy={moonPy} r={16} fill="url(#moonGlow)" />
        <circle cx={moonPx} cy={moonPy} r={5}
          fill="#fef3c7" stroke="#fbbf24" strokeWidth={0.5} />
        <text x={moonPx} y={moonPy - 10} textAnchor="middle"
          fill="#fbbf24" fontSize="9" opacity={0.7}>Moon</text>

        {/* ── Scale / info ── */}
        <text x={12} y={CANVAS - 8} fill="#1e293b" fontSize="8">
          GEO ring shown 3× actual scale relative to lunar orbit
        </text>

        {/* ── Direction labels ── */}
        <text x={CX} y={14} textAnchor="middle" fill="#1e293b" fontSize="8">0° (vernal equinox direction)</text>
      </svg>

      {/* ── Status ── */}
      <div style={{
        display: "flex", justifyContent: "space-between", alignItems: "center",
        width: CANVAS, maxWidth: "100%", padding: "6px 12px",
        background: "#0a0a16", borderRadius: "4px", border: "1px solid #141422",
        fontSize: "11px",
      }}>
        <span>Day {currentDay.toFixed(1)} / {totalDays.toFixed(1)}</span>
        <span style={{ color: bestColor, fontWeight: 600 }}>
          Best: {bestName}{bestSat !== null ? ` (${bestAngle}° from limb)` : ""}
        </span>
      </div>

      {/* ── Satellite status row ── */}
      <div style={{
        display: "flex", gap: "6px", flexWrap: "wrap", justifyContent: "center",
        width: CANVAS, maxWidth: "100%",
      }}>
        {SATELLITES.map((s, si) => {
          const v = satViews[si];
          const isBest = bestSat === si;
          const status = v.behind ? "behind" : v.occluded ? "occluded" : `${v.angOffset.toFixed(1)}°`;
          return (
            <div key={si} style={{
              padding: "4px 8px", borderRadius: "3px",
              background: isBest ? s.color + "18" : "#0a0a16",
              border: `1px solid ${isBest ? s.color + "60" : "#141422"}`,
              fontSize: "10px", minWidth: "110px", textAlign: "center",
            }}>
              <span style={{ color: s.color, fontWeight: isBest ? 700 : 400 }}>
                {s.name}{isBest ? " ★" : ""}
              </span>
              <br />
              <span style={{ color: v.behind || v.occluded ? "#444" : "#888", fontSize: "9px" }}>
                {status}
              </span>
            </div>
          );
        })}
      </div>

      {/* ── Controls ── */}
      <div style={{
        width: CANVAS, maxWidth: "100%",
        display: "flex", flexDirection: "column", gap: "6px", fontSize: "11px",
      }}>
        <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
          <label style={{ color: "#2e3848", minWidth: "70px" }}>Time</label>
          <input type="range" min={0} max={totalDays} step={0.01}
            value={currentDay}
            onChange={e => { setCurrentDay(parseFloat(e.target.value)); setAnimating(false); }}
            style={{ flex: 1, accentColor: "#fbbf24" }} />
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
          <label style={{ color: "#2e3848", minWidth: "70px" }}>Node Ω: {nodeOmega}°</label>
          <input type="range" min={0} max={360} step={1}
            value={nodeOmega}
            onChange={e => setNodeOmega(parseInt(e.target.value))}
            style={{ flex: 1, accentColor: "#60a5fa" }} />
        </div>
        <div style={{ display: "flex", gap: "6px", alignItems: "center" }}>
          <button onClick={() => setAnimating(!animating)} style={{
            background: animating ? "#141422" : "#1e3a5f",
            border: "1px solid #1e1e32", color: "#b8bcc5",
            padding: "4px 12px", borderRadius: "3px",
            cursor: "pointer", fontSize: "11px", fontFamily: "inherit",
          }}>
            {animating ? "⏸ Pause" : "▶ Play"}
          </button>
          {[0.25, 0.5, 1, 2, 4, 8].map(s => (
            <button key={s} onClick={() => setSpeed(s)} style={{
              background: speed === s ? "#1e1e36" : "transparent",
              border: "1px solid #141422",
              color: speed === s ? "#fbbf24" : "#2e3848",
              padding: "3px 6px", borderRadius: "3px",
              cursor: "pointer", fontSize: "10px", fontFamily: "inherit",
            }}>
              {s}×
            </button>
          ))}
          <div style={{ flex: 1 }} />
          <label style={{ display: "flex", alignItems: "center", gap: "4px", cursor: "pointer" }}>
            <input type="checkbox" checked={showTrail} onChange={e => setShowTrail(e.target.checked)}
              style={{ accentColor: "#d4a017" }} />
            <span style={{ color: "#2e3848", fontSize: "10px" }}>Trail</span>
          </label>
          <label style={{ display: "flex", alignItems: "center", gap: "4px", cursor: "pointer" }}>
            <input type="checkbox" checked={showLoS} onChange={e => setShowLoS(e.target.checked)}
              style={{ accentColor: "#ef4444" }} />
            <span style={{ color: "#2e3848", fontSize: "10px" }}>Sightlines</span>
          </label>
        </div>

        <div style={{
          fontSize: "9px", color: "#1e2838", lineHeight: "1.5", padding: "4px 8px",
        }}>
          Sightlines connect each satellite to the moon. Solid = clear view, dashed = occluded or behind observer.
          The best satellite (★) has the clearest near-limb view of the moon next to Earth.
          Ω controls the 18.6-year lunar nodal precession.
        </div>
      </div>
    </div>
  );
}
