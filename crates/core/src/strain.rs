//! Strain of the flat pattern when it is pulled onto the body.
//!
//! For a triangle with flat corners `u₀, u₁, u₂` and body corners
//! `p₀, p₁, p₂`, the deformation gradient of the map *flat → body* is
//! ```text
//! F = P Q⁻¹,   P = [p₁ − p₀, p₂ − p₀] (3×2),   Q = [u₁ − u₀, u₂ − u₀] (2×2).
//! ```
//! Its singular values `σ_min ≤ σ_max` are the principal stretches: a unit
//! length of flat latex becomes `σ` units on the body. They are the square
//! roots of the eigenvalues of the 2×2 right Cauchy–Green tensor `C = FᵀF`.
//! `σ = 1` means unstretched, `σ > 1` stretched, `σ < 1` slack (wrinkles).
//! The area stretch is `σ_min σ_max = A_body / A_flat`.
//!
//! A triangle is *flipped* if its signed flat area is `≤ 0`.

use crate::linalg::sym2_eigenvalues;
use crate::panel::Panel;
use glam::{DMat2, DVec2, DVec3};

/// Principal stretches of one triangle.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PrincipalStretch {
    pub min: f64,
    pub max: f64,
}

impl PrincipalStretch {
    /// Area stretch `σ_min σ_max`.
    pub fn area(&self) -> f64 {
        self.min * self.max
    }
}

/// Strain summary for one flattened panel.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StrainReport {
    /// Per local triangle.
    pub triangles: Vec<PrincipalStretch>,
    /// Local triangle indices with non-positive flat area.
    pub flipped: Vec<u32>,
    /// Smallest `σ_min` over the panel.
    pub min_stretch: f64,
    /// Largest `σ_max` over the panel.
    pub max_stretch: f64,
    /// Area-weighted 95th percentile of `σ_max`: 95 % of the piece (by
    /// body area) stretches less than this. Unlike [`max_stretch`], it is
    /// not dominated by a few tiny triangles at sharp seam corners.
    ///
    /// [`max_stretch`]: StrainReport::max_stretch
    pub p95_stretch: f64,
    /// Fraction of the piece's body area where some direction is slack
    /// (`σ_min < 0.99`): there the latex is not stretched in every
    /// direction and may wrinkle.
    pub slack_fraction: f64,
    /// Body-area-weighted mean of the area stretch `σ_min σ_max`.
    pub mean_area_stretch: f64,
    /// 3D surface area (mm²).
    pub body_area: f64,
    /// Signed flat area (mm²).
    pub flat_area: f64,
}

/// Principal stretches of the flat → body map on a single triangle.
///
/// Returns infinite stretch for a collapsed flat triangle.
pub fn principal_stretch(p: [DVec3; 3], u: [DVec2; 3]) -> PrincipalStretch {
    let (p1, p2) = (p[1] - p[0], p[2] - p[0]);
    let q = DMat2::from_cols(u[1] - u[0], u[2] - u[0]);
    let det = q.determinant();
    if det.abs() < f64::MIN_POSITIVE {
        return PrincipalStretch {
            min: f64::INFINITY,
            max: f64::INFINITY,
        };
    }
    let qi = q.inverse();
    // Columns of F = P Q⁻¹.
    let f0 = p1 * qi.x_axis.x + p2 * qi.x_axis.y;
    let f1 = p1 * qi.y_axis.x + p2 * qi.y_axis.y;
    let (l0, l1) = sym2_eigenvalues(f0.dot(f0), f0.dot(f1), f1.dot(f1));
    PrincipalStretch {
        min: l0.max(0.0).sqrt(),
        max: l1.max(0.0).sqrt(),
    }
}

/// Measure the strain of `uv` as a layout of `panel`.
pub fn measure(panel: &Panel, uv: &[DVec2]) -> StrainReport {
    let mut triangles = Vec::with_capacity(panel.triangles.len());
    let mut flipped = Vec::new();
    let (mut min_s, mut max_s) = (f64::INFINITY, 0.0f64);
    let (mut body_area, mut flat_area, mut weighted) = (0.0, 0.0, 0.0);
    let mut areas = Vec::with_capacity(panel.triangles.len());
    for (t, tri) in panel.triangles.iter().enumerate() {
        let p = tri.map(|i| panel.positions[i as usize]);
        let u = tri.map(|i| uv[i as usize]);
        let fa = 0.5 * (u[1] - u[0]).perp_dot(u[2] - u[0]);
        let ba = 0.5 * (p[1] - p[0]).cross(p[2] - p[0]).length();
        if fa <= 0.0 {
            flipped.push(t as u32);
        }
        let s = principal_stretch(p, u);
        min_s = min_s.min(s.min);
        max_s = max_s.max(s.max);
        body_area += ba;
        flat_area += fa;
        weighted += ba * s.area();
        areas.push(ba);
        triangles.push(s);
    }
    let p95_stretch = area_percentile(&triangles, &areas, 0.95);
    let slack: f64 = triangles
        .iter()
        .zip(&areas)
        .filter(|(s, _)| s.min < SLACK)
        .map(|(_, a)| a)
        .sum();
    StrainReport {
        triangles,
        flipped,
        min_stretch: min_s,
        max_stretch: max_s,
        p95_stretch,
        slack_fraction: slack / body_area,
        mean_area_stretch: weighted / body_area,
        body_area,
        flat_area,
    }
}

/// Minimum stretch below which a triangle counts as slack.
const SLACK: f64 = 0.99;

/// The value of `σ_max` below which a fraction `q` of the total area lies:
/// sort triangles by `σ_max` and accumulate their areas until `q` of the
/// total is reached.
fn area_percentile(stretch: &[PrincipalStretch], areas: &[f64], q: f64) -> f64 {
    let mut idx: Vec<usize> = (0..stretch.len()).collect();
    idx.sort_by(|&a, &b| stretch[a].max.total_cmp(&stretch[b].max));
    let total: f64 = areas.iter().sum();
    let mut acc = 0.0;
    for &i in &idx {
        acc += areas[i];
        if acc >= q * total {
            return stretch[i].max;
        }
    }
    idx.last().map_or(f64::NAN, |&i| stretch[i].max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn percentile_ignores_tiny_outliers() {
        let s = |m: f64| PrincipalStretch { min: m, max: m };
        // 99 % of the area at 1.2, a sliver at 5.
        let stretch = [s(1.2), s(1.2), s(5.0)];
        let areas = [50.0, 49.0, 1.0];
        assert_relative_eq!(area_percentile(&stretch, &areas, 0.95), 1.2);
        assert_relative_eq!(area_percentile(&stretch, &areas, 1.0), 5.0);
    }

    #[test]
    fn known_stretch() {
        // Flat triangle of legs 2 and 1 maps to body legs 3 and 1 (stretch 1.5 in x).
        let p = [DVec3::ZERO, DVec3::new(3., 0., 0.), DVec3::new(0., 0., 1.)];
        let u = [DVec2::ZERO, DVec2::new(2., 0.), DVec2::new(0., 1.)];
        let s = principal_stretch(p, u);
        assert_relative_eq!(s.min, 1.0, epsilon = 1e-12);
        assert_relative_eq!(s.max, 1.5, epsilon = 1e-12);
    }
}
