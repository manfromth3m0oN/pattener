//! Parametric torso from body measurements.
//!
//! The torso is a lofted tube through measured **levels** (e.g. hip,
//! waist, underbust, bust), each given by its height and circumference.
//! The result is a [`Mesh`] plus ready-made seams, so a garment pattern can
//! be produced from a tape measure alone.
//!
//! ## Shape
//! Coordinates are millimetres, `Y` up, front towards `+Z`, the body's left
//! towards `+X`. Every ring of the mesh is a cross-section
//! ```text
//! x = r cos θ,   z = r · 2d · (f if sin θ ≥ 0 else 1 − f) · sin θ
//! ```
//! i.e. an ellipse of half-width `r`, total depth `2dr` (`d` = depth ratio,
//! default 0.75), with a fraction `f` (front bias, default 0.5) of the depth
//! in front of the side seams; `f > 0.5` adds bust or belly projection.
//! `r` is chosen so that the **perimeter of the mesh ring itself** equals
//! the measured circumference, so the pattern fits the measurements exactly
//! rather than a smooth curve the mesh only approximates.
//!
//! Between levels, circumference, depth ratio and front bias are
//! interpolated in height with monotone cubic Hermite splines (Fritsch–
//! Carlson), which pass through every measurement without overshooting
//! (a waist never comes out smaller than measured). Rings are spaced at
//! most `ring_spacing_mm` apart and every level falls exactly on a ring.
//!
//! ## Seams
//! Seams run straight up mesh columns, so every panel is a disc:
//! - [`SeamStyle::Tube`]: left and right side seams → front and back;
//! - [`SeamStyle::Princess`]: side seams plus two front and two back
//!   princess lines → six panels, which shape bust and waist far better.
//!
//! Each seam gets a notch at every inner level (e.g. the waist), so the
//! landmarks line up when gluing.

use crate::error::{Error, Result};
use crate::mesh::Mesh;
use crate::seams::{Seam, SeamSet};
use glam::DVec3;
use std::f64::consts::TAU;

/// One horizontal measurement.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Level {
    pub name: String,
    /// Height above the lowest level (mm).
    pub height_mm: f64,
    /// Girth at this height (mm).
    pub circumference_mm: f64,
    /// Front-to-back depth over side-to-side width (default 0.75).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub depth_ratio: Option<f64>,
    /// Fraction of the depth in front of the side seams (default 0.5).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub front_bias: Option<f64>,
}

/// Measurements for [`torso`], bottom to top.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BodyMeasurements {
    pub levels: Vec<Level>,
}

/// Seam layout of the generated torso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum SeamStyle {
    /// Two side seams: front and back panels.
    Tube,
    /// Side seams plus front and back princess lines: six panels.
    #[default]
    Princess,
}

/// Mesh resolution and seam layout.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct BodyOptions {
    /// Vertices around each ring (a multiple of 4, at least 16).
    pub around: u32,
    /// Maximum vertical distance between rings (mm).
    pub ring_spacing_mm: f64,
    pub style: SeamStyle,
}

impl Default for BodyOptions {
    fn default() -> Self {
        Self {
            around: 64,
            ring_spacing_mm: 10.0,
            style: SeamStyle::Princess,
        }
    }
}

/// A generated torso.
#[derive(Debug, Clone)]
pub struct Body {
    pub mesh: Mesh,
    pub seams: SeamSet,
    /// Ring index of each level, in input order.
    pub level_rings: Vec<u32>,
    /// Vertices per ring.
    pub around: u32,
}

impl Body {
    /// Vertex index of column `i`, ring `j`.
    pub fn vertex(&self, i: u32, j: u32) -> u32 {
        j * self.around + (i % self.around)
    }
}

const DEFAULT_DEPTH: f64 = 0.75;
const DEFAULT_BIAS: f64 = 0.5;

/// Build a torso mesh and its seams from measurements.
pub fn torso(m: &BodyMeasurements, opts: &BodyOptions) -> Result<Body> {
    let n = opts.around;
    if n < 16 || !n.is_multiple_of(4) {
        return Err(Error::Body(format!(
            "'around' must be a multiple of 4 and at least 16, got {n}"
        )));
    }
    if !(opts.ring_spacing_mm > 0.0) {
        return Err(Error::Body("ring spacing must be positive".into()));
    }
    let lv = &m.levels;
    if lv.len() < 2 {
        return Err(Error::Body("at least two levels are needed".into()));
    }
    for w in lv.windows(2) {
        if !(w[1].height_mm > w[0].height_mm) {
            return Err(Error::Body(format!(
                "levels must be listed bottom to top: '{}' ({} mm) is not above '{}' ({} mm)",
                w[1].name, w[1].height_mm, w[0].name, w[0].height_mm
            )));
        }
    }
    for l in lv {
        let d = l.depth_ratio.unwrap_or(DEFAULT_DEPTH);
        let f = l.front_bias.unwrap_or(DEFAULT_BIAS);
        if !(l.circumference_mm > 0.0 && l.circumference_mm.is_finite()) {
            return Err(Error::Body(format!(
                "level '{}': circumference must be positive",
                l.name
            )));
        }
        if !(0.2..=2.0).contains(&d) {
            return Err(Error::Body(format!(
                "level '{}': depth ratio {d} is outside 0.2–2",
                l.name
            )));
        }
        if !(0.2..=0.8).contains(&f) {
            return Err(Error::Body(format!(
                "level '{}': front bias {f} is outside 0.2–0.8",
                l.name
            )));
        }
    }

    // Ring heights, with every level on a ring.
    let mut heights = vec![lv[0].height_mm];
    let mut level_rings = vec![0u32];
    for w in lv.windows(2) {
        let span = w[1].height_mm - w[0].height_mm;
        let steps = (span / opts.ring_spacing_mm).ceil().max(1.0) as u32;
        for k in 1..=steps {
            heights.push(w[0].height_mm + span * k as f64 / steps as f64);
        }
        level_rings.push(heights.len() as u32 - 1);
    }

    let hs: Vec<f64> = lv.iter().map(|l| l.height_mm).collect();
    let circ = Pchip::new(
        &hs,
        &lv.iter().map(|l| l.circumference_mm).collect::<Vec<_>>(),
    );
    let depth = Pchip::new(
        &hs,
        &lv.iter()
            .map(|l| l.depth_ratio.unwrap_or(DEFAULT_DEPTH))
            .collect::<Vec<_>>(),
    );
    let bias = Pchip::new(
        &hs,
        &lv.iter()
            .map(|l| l.front_bias.unwrap_or(DEFAULT_BIAS))
            .collect::<Vec<_>>(),
    );

    let mut positions = Vec::with_capacity(heights.len() * n as usize);
    for &h in &heights {
        let (d, f) = (depth.eval(h), bias.eval(h));
        let unit: Vec<(f64, f64)> = (0..n)
            .map(|i| {
                let th = TAU * i as f64 / n as f64;
                let s = th.sin();
                let half = 2.0 * d * if s >= 0.0 { f } else { 1.0 - f };
                (th.cos(), half * s)
            })
            .collect();
        let perimeter: f64 = (0..n as usize)
            .map(|i| {
                let (a, b) = (unit[i], unit[(i + 1) % n as usize]);
                (a.0 - b.0).hypot(a.1 - b.1)
            })
            .sum();
        let r = circ.eval(h) / perimeter;
        positions.extend(unit.iter().map(|&(x, z)| DVec3::new(r * x, h, r * z)));
    }

    let rings = heights.len() as u32;
    let id = |i: u32, j: u32| j * n + (i % n);
    let mut triangles = Vec::with_capacity(2 * (n * (rings - 1)) as usize);
    for j in 0..rings - 1 {
        for i in 0..n {
            // Counter-clockwise seen from outside (front is +Z, up is +Y).
            let (a, b, c, d) = (id(i, j), id(i, j + 1), id(i + 1, j + 1), id(i + 1, j));
            triangles.push([a, b, c]);
            triangles.push([a, c, d]);
        }
    }
    let mesh = Mesh::new(positions, triangles)?;

    let inner: Vec<u32> = level_rings[1..level_rings.len() - 1].to_vec();
    let seam = |name: &str, col: u32| Seam {
        name: Some(name.into()),
        vertices: (0..rings).map(|j| id(col, j)).collect(),
        notches: inner.iter().map(|&j| id(col, j)).collect(),
        allowance: None,
    };
    let (q, off) = (n / 4, (n * 3 + 16) / 32); // princess lines ~34° from centre
    let mut seams = vec![seam("side_left", 0), seam("side_right", 2 * q)];
    if opts.style == SeamStyle::Princess {
        seams.extend([
            seam("princess_front_left", q - off),
            seam("princess_front_right", q + off),
            seam("princess_back_right", 3 * q - off),
            seam("princess_back_left", 3 * q + off),
        ]);
    }

    Ok(Body {
        mesh,
        seams: SeamSet { seams },
        level_rings,
        around: n,
    })
}

/// Monotone cubic Hermite interpolation (Fritsch–Carlson): passes through
/// every point and never overshoots between them.
struct Pchip {
    x: Vec<f64>,
    y: Vec<f64>,
    m: Vec<f64>,
}

impl Pchip {
    fn new(x: &[f64], y: &[f64]) -> Self {
        let n = x.len();
        let delta: Vec<f64> = (0..n - 1)
            .map(|k| (y[k + 1] - y[k]) / (x[k + 1] - x[k]))
            .collect();
        let mut m = vec![0.0; n];
        m[0] = delta[0];
        m[n - 1] = delta[n - 2];
        for k in 1..n - 1 {
            m[k] = if delta[k - 1] * delta[k] <= 0.0 {
                0.0
            } else {
                // Weighted harmonic mean of the neighbouring slopes.
                let (h0, h1) = (x[k] - x[k - 1], x[k + 1] - x[k]);
                let (w1, w2) = (2.0 * h1 + h0, h1 + 2.0 * h0);
                (w1 + w2) / (w1 / delta[k - 1] + w2 / delta[k])
            };
        }
        // End slopes: keep monotone.
        for (k, d) in [(0, delta[0]), (n - 1, delta[n - 2])] {
            if m[k] * d <= 0.0 || d == 0.0 {
                m[k] = 0.0;
            } else if (m[k] / d).abs() > 3.0 {
                m[k] = 3.0 * d;
            }
        }
        Self {
            x: x.to_vec(),
            y: y.to_vec(),
            m,
        }
    }

    fn eval(&self, t: f64) -> f64 {
        let k = self
            .x
            .partition_point(|&v| v <= t)
            .clamp(1, self.x.len() - 1)
            - 1;
        let h = self.x[k + 1] - self.x[k];
        let s = ((t - self.x[k]) / h).clamp(0.0, 1.0);
        let (s2, s3) = (s * s, s * s * s);
        (2.0 * s3 - 3.0 * s2 + 1.0) * self.y[k]
            + (s3 - 2.0 * s2 + s) * h * self.m[k]
            + (-2.0 * s3 + 3.0 * s2) * self.y[k + 1]
            + (s3 - s2) * h * self.m[k + 1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{flatten_garment, FlattenOptions};
    use approx::assert_relative_eq;

    pub(crate) fn sample() -> BodyMeasurements {
        let l = |name: &str, h: f64, c: f64, f: Option<f64>| Level {
            name: name.into(),
            height_mm: h,
            circumference_mm: c,
            depth_ratio: None,
            front_bias: f,
        };
        BodyMeasurements {
            levels: vec![
                l("hip", 0.0, 980.0, None),
                l("waist", 200.0, 740.0, None),
                l("underbust", 330.0, 790.0, None),
                l("bust", 420.0, 940.0, Some(0.6)),
                l("chest", 500.0, 900.0, None),
            ],
        }
    }

    fn ring_perimeter(b: &Body, j: u32) -> f64 {
        (0..b.around)
            .map(|i| {
                b.mesh.positions()[b.vertex(i, j) as usize]
                    .distance(b.mesh.positions()[b.vertex(i + 1, j) as usize])
            })
            .sum()
    }

    #[test]
    fn rings_match_measurements_exactly() {
        let m = sample();
        let b = torso(&m, &BodyOptions::default()).unwrap();
        for (l, &j) in m.levels.iter().zip(&b.level_rings) {
            assert_relative_eq!(ring_perimeter(&b, j), l.circumference_mm, epsilon = 1e-9);
            assert_relative_eq!(
                b.mesh.positions()[b.vertex(0, j) as usize].y,
                l.height_mm,
                epsilon = 1e-12
            );
        }
        // No ring between hip and waist is narrower than the waist (no overshoot).
        for j in b.level_rings[0]..=b.level_rings[1] {
            assert!(ring_perimeter(&b, j) >= 740.0 - 1e-9);
        }
    }

    #[test]
    fn normals_point_outwards_and_bust_projects_forward() {
        let b = torso(&sample(), &BodyOptions::default()).unwrap();
        let p = b.mesh.positions();
        for t in b.mesh.triangles() {
            let [a, c, d] = t.map(|i| p[i as usize]);
            let normal = (c - a).cross(d - a);
            let centre = (a + c + d) / 3.0;
            let radial = DVec3::new(centre.x, 0.0, centre.z);
            assert!(normal.dot(radial) > 0.0);
        }
        let bust = b.level_rings[3];
        let front = p[b.vertex(b.around / 4, bust) as usize].z;
        let back = -p[b.vertex(3 * b.around / 4, bust) as usize].z;
        assert_relative_eq!(front / back, 0.6 / 0.4, epsilon = 1e-9);
    }

    #[test]
    fn seam_styles_flatten_into_panels() {
        let opts = |style| BodyOptions {
            around: 32,
            ring_spacing_mm: 25.0,
            style,
        };
        for (style, panels) in [(SeamStyle::Tube, 2), (SeamStyle::Princess, 6)] {
            let b = torso(&sample(), &opts(style)).unwrap();
            let g = flatten_garment(&b.mesh, &b.seams, &FlattenOptions::default()).unwrap();
            assert_eq!(g.panels.len(), panels);
            for p in &g.panels {
                assert!(p.strain.flipped.is_empty());
            }
            for s in &g.seams {
                assert!(s.lengths.mismatch().abs() < 0.05, "{:?}", s.lengths);
            }
            // Three inner levels → three notch vertices per seam.
            assert!(b.seams.seams.iter().all(|s| s.notches.len() == 3));
        }
    }

    #[test]
    fn rejects_bad_measurements() {
        let mut m = sample();
        m.levels.swap(1, 2);
        assert!(
            matches!(torso(&m, &BodyOptions::default()), Err(Error::Body(e)) if e.contains("bottom to top"))
        );
        let mut m = sample();
        m.levels[2].front_bias = Some(0.95);
        assert!(
            matches!(torso(&m, &BodyOptions::default()), Err(Error::Body(e)) if e.contains("front bias"))
        );
        let o = BodyOptions {
            around: 30,
            ..Default::default()
        };
        assert!(torso(&sample(), &o).is_err());
    }
}
