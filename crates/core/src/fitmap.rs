//! How a garment presses on the body: the pressure and bridging map.
//!
//! A stretched membrane pressing on a curved surface exerts the pressure
//! (Laplace's law for membranes)
//! ```text
//! p = T₁ κ₁ + T₂ κ₂
//! ```
//! where `Tᵢ` is the membrane tension (force per unit width) in principal
//! stretch direction `i` and `κᵢ` the body's normal curvature in that
//! direction. Where the body curves away beneath the latex (hollows such
//! as the small of the back or between the breasts), `p < 0`: the latex
//! cannot pull into the hollow and **bridges** over it.
//!
//! ## Tension
//! For an incompressible neo-Hookean sheet (shear modulus `μ`, thickness
//! `t₀`) stretched by `λ₁, λ₂` in plane, the Cauchy stress is
//! `σᵢ = μ (λᵢ² − (λ₁λ₂)⁻²)` and the thickness `t₀ / (λ₁λ₂)`, so the
//! tension per unit current width is
//! ```text
//! Tᵢ = k (λᵢ² − (λ₁λ₂)⁻²) / (λ₁λ₂),   k = μ t₀,
//! ```
//! clamped at 0 (slack latex does not push). The principal stretches and
//! their directions on the body come from the deformation gradient
//! `F = P Q⁻¹` of each triangle (flat → body; see [`crate::strain`]): with
//! `FᵀF = V Λ Vᵀ`, the body directions are `uᵢ = F vᵢ / λᵢ`.
//!
//! ## Curvature
//! Latex bridges bumps much smaller than a few centimetres instead of
//! following them, so curvature is measured at that scale: area-weighted
//! vertex normals are smoothed by [`NORMAL_SMOOTHING`] rounds of averaging
//! with their neighbours (on the 10–15 mm body mesh, a scale of about
//! 4–6 cm). Without this, small sharp details of the body mesh (nipples,
//! the navel) show hundreds of mmHg over a few square millimetres. The
//! smoothed normals `n_j` are interpolated linearly over each triangle; the normal curvature along a unit tangent `u` is
//! `κ(u) = u · (∂n/∂u) = u · Σ_j n_j (∇φ_j · u)` with the barycentric
//! gradients `∇φ_j = N × (x_k − x_i) / (2A)` (`(i, j, k)` cyclic, `N` the
//! unit face normal). Outward normals make convex body parts positive.
//!
//! Pressures are reported in mmHg (1 N/mm² = 7500.6 mmHg), the unit used
//! for compression garments: 15–20 mmHg is light medical compression.

use crate::flatten::Garment;
use crate::mesh::Mesh;
use glam::{DVec2, DVec3};

/// N/mm² to mmHg.
pub const MMHG_PER_N_MM2: f64 = 7500.62;

/// Pressure below which (mmHg) the latex counts as bridging.
const BRIDGE_MMHG: f64 = -0.5;

/// Latex properties for the pressure map when no calibration is given.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Material {
    /// Neo-Hookean shear modulus μ (MPa); natural rubber latex is about 0.45.
    pub shear_modulus_mpa: f64,
    /// Sheet thickness (mm).
    pub thickness_mm: f64,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            shear_modulus_mpa: 0.45,
            thickness_mm: 0.4,
        }
    }
}

impl Material {
    /// Membrane stiffness `k = μ t₀` (N/mm).
    pub fn stiffness(&self) -> f64 {
        self.shear_modulus_mpa * self.thickness_mm
    }
}

/// Fit summary of one panel.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanelFit {
    /// Body-area-weighted mean pressure (mmHg).
    pub mean_pressure_mmhg: f64,
    /// 95 % of the panel's area presses less than this (mmHg).
    pub p95_pressure_mmhg: f64,
    /// Fraction of the panel's area where the latex bridges (p < −0.5 mmHg).
    pub bridging_fraction: f64,
}

/// Pressure map of a garment.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FitMap {
    /// Per vertex of the garment mesh (mmHg); area-weighted average of the
    /// surrounding triangles.
    pub pressure_mmhg: Vec<f64>,
    /// Per triangle of the garment mesh (mmHg).
    pub triangle_pressure_mmhg: Vec<f64>,
    pub panels: Vec<PanelFit>,
}

/// Neo-Hookean membrane tensions `(T₁, T₂)` per unit current width (N/mm)
/// for in-plane stretches `λ₁, λ₂` and stiffness `k = μ t₀`.
pub fn tensions(k: f64, l1: f64, l2: f64) -> (f64, f64) {
    let j = l1 * l2;
    let t = |l: f64| (k * (l * l - 1.0 / (j * j)) / j).max(0.0);
    (t(l1), t(l2))
}

/// Rounds of neighbour averaging applied to the vertex normals.
pub const NORMAL_SMOOTHING: usize = 3;

/// Area-weighted vertex normals, smoothed over their neighbours.
fn vertex_normals(mesh: &Mesh) -> Vec<DVec3> {
    let mut n = vec![DVec3::ZERO; mesh.positions().len()];
    for t in mesh.triangles() {
        let [a, b, c] = t.map(|i| mesh.positions()[i as usize]);
        let f = (b - a).cross(c - a);
        for &i in t {
            n[i as usize] += f;
        }
    }
    let mut n: Vec<DVec3> = n.into_iter().map(|v| v.normalize_or_zero()).collect();
    for _ in 0..NORMAL_SMOOTHING {
        let mut sum = n.clone();
        for e in mesh.edges() {
            let (a, b) = (e.v[0] as usize, e.v[1] as usize);
            sum[a] += n[b];
            sum[b] += n[a];
        }
        n = sum.into_iter().map(|v| v.normalize_or_zero()).collect();
    }
    n
}

/// Normal curvature of triangle `t` along unit tangent `u`.
fn curvature(mesh: &Mesh, normals: &[DVec3], t: [u32; 3], u: DVec3) -> f64 {
    let x = t.map(|i| mesh.positions()[i as usize]);
    let cross = (x[1] - x[0]).cross(x[2] - x[0]);
    let area2 = cross.length();
    if area2 < 1e-12 {
        return 0.0;
    }
    let nf = cross / area2;
    let mut dn = DVec3::ZERO;
    for j in 0..3 {
        let grad = nf.cross(x[(j + 2) % 3] - x[(j + 1) % 3]) / area2;
        dn += normals[t[j] as usize] * grad.dot(u);
    }
    u.dot(dn)
}

/// Compute the pressure map. `stiffness[p]` is `k = μ t₀` (N/mm) of panel
/// `p`; `mesh` is the mesh the garment was flattened from.
pub fn pressure_map(mesh: &Mesh, garment: &Garment, stiffness: &[f64]) -> FitMap {
    let normals = vertex_normals(mesh);
    let nt = mesh.triangles().len();
    let mut tri_p = vec![0.0; nt];
    let mut tri_area = vec![0.0; nt];
    let mut panels = Vec::with_capacity(garment.panels.len());
    for (pi, fp) in garment.panels.iter().enumerate() {
        let k = stiffness[pi];
        let mut samples: Vec<(f64, f64)> = Vec::with_capacity(fp.panel.triangles.len());
        for (lt, tri) in fp.panel.triangles.iter().enumerate() {
            let f = fp.panel.face_origin[lt] as usize;
            let p = tri.map(|i| fp.panel.positions[i as usize]);
            let u = tri.map(|i| fp.uv[i as usize]);
            let area = 0.5 * (p[1] - p[0]).cross(p[2] - p[0]).length();
            let pressure =
                triangle_pressure(mesh, &normals, mesh.triangles()[f], p, u, k) * MMHG_PER_N_MM2;
            tri_p[f] = pressure;
            tri_area[f] = area;
            samples.push((pressure, area));
        }
        let total: f64 = samples.iter().map(|s| s.1).sum();
        let mean = samples.iter().map(|(p, a)| p * a).sum::<f64>() / total;
        let bridging = samples
            .iter()
            .filter(|s| s.0 < BRIDGE_MMHG)
            .map(|s| s.1)
            .sum::<f64>()
            / total;
        samples.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut acc = 0.0;
        let mut p95 = samples.last().map_or(0.0, |s| s.0);
        for &(p, a) in &samples {
            acc += a;
            if acc >= 0.95 * total {
                p95 = p;
                break;
            }
        }
        panels.push(PanelFit {
            mean_pressure_mmhg: mean,
            p95_pressure_mmhg: p95,
            bridging_fraction: bridging,
        });
    }
    let mut vp = vec![0.0; mesh.positions().len()];
    let mut vw = vec![0.0; mesh.positions().len()];
    for (f, t) in mesh.triangles().iter().enumerate() {
        for &i in t {
            vp[i as usize] += tri_p[f] * tri_area[f];
            vw[i as usize] += tri_area[f];
        }
    }
    let pressure_mmhg = vp
        .iter()
        .zip(&vw)
        .map(|(p, w)| if *w > 0.0 { p / w } else { 0.0 })
        .collect();
    FitMap {
        pressure_mmhg,
        triangle_pressure_mmhg: tri_p,
        panels,
    }
}

/// Pressure (N/mm²) of one triangle with body corners `p`, flat corners
/// `u`, mesh triangle `t` and stiffness `k`.
fn triangle_pressure(
    mesh: &Mesh,
    normals: &[DVec3],
    t: [u32; 3],
    p: [DVec3; 3],
    u: [DVec2; 3],
    k: f64,
) -> f64 {
    let q = glam::DMat2::from_cols(u[1] - u[0], u[2] - u[0]);
    if q.determinant().abs() < 1e-12 {
        return 0.0;
    }
    let qi = q.inverse();
    let (p1, p2) = (p[1] - p[0], p[2] - p[0]);
    // Columns of F = P Q⁻¹ (flat x, flat y → body).
    let f0 = p1 * qi.x_axis.x + p2 * qi.x_axis.y;
    let f1 = p1 * qi.y_axis.x + p2 * qi.y_axis.y;
    let (a, b, d) = (f0.dot(f0), f0.dot(f1), f1.dot(f1));
    let (l_lo, l_hi) = crate::linalg::sym2_eigenvalues(a, b, d);
    // Eigenvector of [[a, b], [b, d]] for the larger eigenvalue.
    let v = if b.abs() > 1e-12 {
        DVec2::new(l_hi - d, b).normalize()
    } else if a >= d {
        DVec2::X
    } else {
        DVec2::Y
    };
    let w = v.perp();
    let (s_hi, s_lo) = (l_hi.max(0.0).sqrt(), l_lo.max(0.0).sqrt());
    if s_lo < 1e-9 {
        return 0.0;
    }
    let u_hi = (f0 * v.x + f1 * v.y) / s_hi;
    let u_lo = (f0 * w.x + f1 * w.y) / s_lo;
    let (t_hi, t_lo) = tensions(k, s_hi, s_lo);
    t_hi * curvature(mesh, normals, t, u_hi) + t_lo * curvature(mesh, normals, t, u_lo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{flatten_garment, testmesh, Ease, FlattenOptions, Seam, SeamSet};
    use approx::assert_relative_eq;
    use std::f64::consts::TAU;

    fn tube(inside_out: bool) -> (Mesh, SeamSet) {
        let (n, up) = (48u32, 8u32);
        let m = testmesh::cylinder(60.0, 150.0, n, up, TAU);
        let m = if inside_out {
            Mesh::new(
                m.positions().to_vec(),
                m.triangles().iter().map(|t| [t[0], t[2], t[1]]).collect(),
            )
            .unwrap()
        } else {
            m
        };
        let seams = SeamSet {
            seams: vec![Seam {
                vertices: (0..=up).map(|j| j * n).collect(),
                ..Default::default()
            }],
        };
        (m, seams)
    }

    #[test]
    fn tube_pressure_is_tension_over_radius() {
        let (m, seams) = tube(false);
        let s = 0.85;
        let opts = FlattenOptions {
            ease: Ease::Uniform(s),
            up: DVec3::Z,
            ..Default::default()
        };
        let g = flatten_garment(&m, &seams, &opts).unwrap();
        let k = Material::default().stiffness();
        let map = pressure_map(&m, &g, &[k]);
        // Stretch 1/s both ways: T = k (λ² − λ⁻⁴)/λ² around, κ = 1/R around
        // and 0 along; the polygon of 48 sides has κ = 1/R (up to the
        // normal smoothing at the open ends).
        let l = 1.0 / s;
        let (t, _) = tensions(k, l, l);
        let expected = t / 60.0 * MMHG_PER_N_MM2;
        // About 14 mmHg on a 60 mm radius: light compression.
        assert!((10.0..20.0).contains(&expected), "{expected}");
        for &p in &map.triangle_pressure_mmhg {
            assert_relative_eq!(p, expected, max_relative = 1e-2);
        }
        assert_relative_eq!(
            map.panels[0].mean_pressure_mmhg,
            expected,
            max_relative = 1e-3
        );
        assert_eq!(map.panels[0].bridging_fraction, 0.0);
    }

    #[test]
    fn concave_surface_bridges() {
        // The inside of a tube curves away from the latex: negative pressure.
        let (m, seams) = tube(true);
        let opts = FlattenOptions {
            up: DVec3::Z,
            ..Default::default()
        };
        let g = flatten_garment(&m, &seams, &opts).unwrap();
        let map = pressure_map(&m, &g, &[Material::default().stiffness()]);
        assert!(map.triangle_pressure_mmhg.iter().all(|&p| p < -1.0));
        assert_relative_eq!(map.panels[0].bridging_fraction, 1.0);
    }

    #[test]
    fn slack_latex_does_not_push() {
        assert_eq!(tensions(1.0, 0.9, 0.95), (0.0, 0.0));
        let (a, b) = tensions(1.0, 1.2, 1.0);
        assert!(a > b && b > 0.0);
    }
}
