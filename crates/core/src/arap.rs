//! As-rigid-as-possible (ARAP) refinement with ease-scaled reference shapes
//! (local/global method of Liu et al. 2008, "A Local/Global Approach to
//! Mesh Parameterization").
//!
//! ## Energy
//! Each triangle `T` has a reference shape `x_T = s_T · frame(T)`: the 3D
//! triangle laid flat isometrically and scaled by its ease factor `s_T`.
//! With cotangent weights `w_ij = cot θ_ij` (angle opposite edge `ij` in
//! the reference triangle, which `s_T` leaves unchanged) the energy is
//! ```text
//! E(u, R) = ½ Σ_T Σ_(i,j)∈T  w_ij ‖(u_i − u_j) − R_T (x_i − x_j)‖²
//! ```
//! over flat positions `u` and per-triangle rotations `R_T`.
//!
//! ## Local step
//! With `u` fixed, each `R_T` maximises `tr(R_Tᵀ S_T)` for the covariance
//! `S_T = Σ w_ij (u_i − u_j)(x_i − x_j)ᵀ`; this is the 2×2 SVD/polar
//! rotation, see [`closest_rotation`](crate::linalg::closest_rotation).
//!
//! ## Global step
//! With `R` fixed, `∂E/∂u = 0` gives the sparse cotangent-Laplacian system
//! ```text
//! L u = b,   L_ii = Σ_j w_ij,  L_ij = −w_ij,
//!            b_i  = Σ_T Σ_j w_ij R_T (x_i − x_j)
//! ```
//! for both coordinates. `L` is pinned at one vertex (removing the
//! translation null space) and factored once.
//!
//! Iteration stops when `(E_prev − E) ≤ tol · E_prev` or after
//! `max_iterations`.

use crate::error::Result;
use crate::linalg::{closest_rotation, cot_at, triangle_frame, SymBuilder};
use crate::panel::Panel;
use glam::DVec2;

/// ARAP iteration controls.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArapOptions {
    pub max_iterations: usize,
    /// Relative energy decrease below which iteration stops.
    pub tolerance: f64,
}

impl Default for ArapOptions {
    fn default() -> Self {
        Self { max_iterations: 500, tolerance: 1e-8 }
    }
}

/// Result of [`arap`].
#[derive(Debug, Clone)]
pub struct ArapResult {
    pub uv: Vec<DVec2>,
    pub iterations: usize,
    pub energy: f64,
}

/// Refine `init` (e.g. from LSCM) towards reference triangles scaled by
/// `ease` (one value per local triangle).
pub fn arap(panel: &Panel, ease: &[f64], init: &[DVec2], opts: &ArapOptions) -> Result<ArapResult> {
    assert_eq!(ease.len(), panel.triangles.len());
    assert_eq!(init.len(), panel.vertex_count());
    let n = panel.vertex_count();

    // Reference shapes and cotangent weights. `w[t][k]` weighs edge
    // (k+1, k+2), opposite corner k.
    let refs: Vec<[DVec2; 3]> = panel
        .triangles
        .iter()
        .zip(ease)
        .map(|(t, &s)| triangle_frame(t.map(|i| panel.positions[i as usize])).map(|q| q * s))
        .collect();
    let w: Vec<[f64; 3]> = refs.iter().map(|q| std::array::from_fn(|k| cot_at(*q, k))).collect();

    // Laplacian with vertex `pin` removed.
    let pin = 0usize;
    let free = |i: usize| if i < pin { Some(i) } else if i > pin { Some(i - 1) } else { None };
    let mut lap = SymBuilder::new(n - 1);
    for (t, tri) in panel.triangles.iter().enumerate() {
        for k in 0..3 {
            let (i, j) = (tri[(k + 1) % 3] as usize, tri[(k + 2) % 3] as usize);
            let wk = w[t][k];
            for (a, b, v) in [(i, i, wk), (j, j, wk), (i, j, -wk), (j, i, -wk)] {
                if let (Some(fa), Some(fb)) = (free(a), free(b)) {
                    lap.add(fa, fb, v);
                }
            }
        }
    }
    let solver = lap.factor()?;

    let mut uv = init.to_vec();
    let mut rot = vec![(1.0, 0.0); panel.triangles.len()];
    let mut prev = f64::INFINITY;
    let mut energy = f64::INFINITY;
    let mut iterations = 0;

    for it in 1..=opts.max_iterations {
        iterations = it;
        // Local step.
        for (t, tri) in panel.triangles.iter().enumerate() {
            let (mut a, mut b, mut c, mut d) = (0.0, 0.0, 0.0, 0.0);
            for k in 0..3 {
                let (i, j) = ((k + 1) % 3, (k + 2) % 3);
                let du = uv[tri[i] as usize] - uv[tri[j] as usize];
                let dx = refs[t][i] - refs[t][j];
                let wk = w[t][k];
                a += wk * du.x * dx.x;
                b += wk * du.x * dx.y;
                c += wk * du.y * dx.x;
                d += wk * du.y * dx.y;
            }
            rot[t] = closest_rotation(a, b, c, d);
        }

        // Global step.
        let mut bx = vec![0.0; n - 1];
        let mut by = vec![0.0; n - 1];
        let p0 = uv[pin];
        for (t, tri) in panel.triangles.iter().enumerate() {
            let (cs, sn) = rot[t];
            for k in 0..3 {
                let (li, lj) = ((k + 1) % 3, (k + 2) % 3);
                let (i, j) = (tri[li] as usize, tri[lj] as usize);
                let dx = refs[t][li] - refs[t][lj];
                let r = w[t][k] * DVec2::new(cs * dx.x - sn * dx.y, sn * dx.x + cs * dx.y);
                // Move the pinned vertex's Laplacian column to the right-hand side.
                let wk = w[t][k];
                for (a, other, sign) in [(i, j, 1.0), (j, i, -1.0)] {
                    if let Some(fa) = free(a) {
                        bx[fa] += sign * r.x;
                        by[fa] += sign * r.y;
                        if other == pin {
                            bx[fa] += wk * p0.x;
                            by[fa] += wk * p0.y;
                        }
                    }
                }
            }
        }
        let sol = solver.solve(&[bx, by])?;
        for i in 0..n {
            if let Some(fi) = free(i) {
                uv[i] = DVec2::new(sol[0][fi], sol[1][fi]);
            }
        }

        energy = arap_energy(panel, &refs, &w, &rot, &uv);
        if prev.is_finite() && prev - energy <= opts.tolerance * prev {
            break;
        }
        prev = energy;
    }

    Ok(ArapResult { uv, iterations, energy })
}

fn arap_energy(panel: &Panel, refs: &[[DVec2; 3]], w: &[[f64; 3]], rot: &[(f64, f64)], uv: &[DVec2]) -> f64 {
    let mut e = 0.0;
    for (t, tri) in panel.triangles.iter().enumerate() {
        let (cs, sn) = rot[t];
        for k in 0..3 {
            let (i, j) = ((k + 1) % 3, (k + 2) % 3);
            let du = uv[tri[i] as usize] - uv[tri[j] as usize];
            let dx = refs[t][i] - refs[t][j];
            let rdx = DVec2::new(cs * dx.x - sn * dx.y, sn * dx.x + cs * dx.y);
            e += 0.5 * w[t][k] * (du - rdx).length_squared();
        }
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cut::cut_panels, lscm::lscm, testmesh, SeamSet};
    use approx::assert_relative_eq;

    #[test]
    fn flat_grid_shrinks_by_ease() {
        let m = testmesh::grid(5, 4, 10.0);
        let p = cut_panels(&m, &SeamSet::default().resolve(&m).unwrap()).unwrap().remove(0);
        let init = lscm(&p).unwrap();
        let s = 0.85;
        let r = arap(&p, &vec![s; p.triangles.len()], &init, &ArapOptions::default()).unwrap();
        assert!(r.energy < 1e-16, "energy {}", r.energy);
        for t in &p.triangles {
            for k in 0..3 {
                let (a, b) = (t[k] as usize, t[(k + 1) % 3] as usize);
                assert_relative_eq!(r.uv[a].distance(r.uv[b]), s * p.positions[a].distance(p.positions[b]), epsilon = 1e-8);
            }
        }
    }
}
