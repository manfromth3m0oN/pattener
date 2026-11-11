//! As-rigid-as-possible (ARAP) refinement with ease-scaled reference shapes
//! (local/global method of Liu et al. 2008, "A Local/Global Approach to
//! Mesh Parameterization").
//!
//! ## Energy
//! Each triangle `T` has a reference shape `x_T`: the 3D triangle laid flat
//! isometrically and scaled by its ease (uniformly, or differently around
//! and along the body: see [`reference_triangles`](crate::ease::reference_triangles)).
//! With cotangent weights `w_ij = cot θ_ij` (angle opposite edge `ij` in
//! the reference triangle) the energy is
//! ```text
//! E(u, R) = ½ Σ_T Σ_(i,j)∈T  w_ij ‖(u_i − u_j) − R_T (x_i − x_j)‖²
//! ```
//! over flat positions `u` and per-triangle rotations `R_T`.
//!
//! ## Local step
//! With `u` fixed, each `R_T` maximises `tr(R_Tᵀ S_T)` for the covariance
//! `S_T = Σ w_ij (u_i − u_j)(x_i − x_j)ᵀ`; this is the 2×2 SVD/polar
//! rotation, see [`closest_rotation`].
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
use crate::linalg::{closest_rotation, cot_at, SymBuilder};
use crate::panel::Panel;
use glam::DVec2;

/// ARAP iteration controls.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ArapOptions {
    pub max_iterations: usize,
    /// Relative energy decrease below which iteration stops.
    pub tolerance: f64,
}

impl Default for ArapOptions {
    fn default() -> Self {
        Self {
            max_iterations: 500,
            tolerance: 1e-8,
        }
    }
}

/// A soft constraint pulling the flat distance between two local vertices
/// towards `length` (used to equalise the two sides of a seam).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeTarget {
    pub a: u32,
    pub b: u32,
    /// Target flat length (mm).
    pub length: f64,
    /// Weight relative to the (dimensionless) cotangent weights.
    pub weight: f64,
}

/// Result of [`arap`].
#[derive(Debug, Clone)]
pub struct ArapResult {
    pub uv: Vec<DVec2>,
    pub iterations: usize,
    pub energy: f64,
}

/// One term `½ w ‖(u_i − u_j) − d‖²` of the energy; `d` is refreshed by
/// every local step.
struct Spring {
    i: usize,
    j: usize,
    w: f64,
}

/// Refine `init` (e.g. from LSCM) towards the reference triangles `refs`
/// (one per local triangle, built by
/// [`reference_triangles`](crate::ease::reference_triangles)), with
/// optional edge-length targets.
///
/// ## Edge-length targets
/// Each [`EdgeTarget`] adds `½ λ ‖(u_a − u_b) − d‖²` to the energy. The
/// local step projects the current edge onto the target length,
/// `d = ℓ (u_a − u_b)/‖u_a − u_b‖`; the global step then gains `λ` in the
/// Laplacian and `±λ d` on the right-hand side. Because `λ` is constant
/// the system matrix is unchanged between iterations and is still factored
/// only once.
pub fn arap(
    panel: &Panel,
    refs: &[[DVec2; 3]],
    init: &[DVec2],
    opts: &ArapOptions,
    targets: &[EdgeTarget],
) -> Result<ArapResult> {
    assert_eq!(refs.len(), panel.triangles.len());
    assert_eq!(init.len(), panel.vertex_count());
    let n = panel.vertex_count();

    // Cotangent weights of the reference shapes (see
    // [`reference_triangles`](crate::ease::reference_triangles)). `w[t][k]`
    // weighs edge (k+1, k+2), opposite corner k.
    let w: Vec<[f64; 3]> = refs
        .iter()
        .map(|q| std::array::from_fn(|k| cot_at(*q, k)))
        .collect();

    // All springs: three per triangle, then one per edge target.
    let mut springs = Vec::with_capacity(3 * panel.triangles.len() + targets.len());
    for (t, tri) in panel.triangles.iter().enumerate() {
        for k in 0..3 {
            let (i, j) = (tri[(k + 1) % 3] as usize, tri[(k + 2) % 3] as usize);
            springs.push(Spring { i, j, w: w[t][k] });
        }
    }
    for e in targets {
        springs.push(Spring {
            i: e.a as usize,
            j: e.b as usize,
            w: e.weight,
        });
    }

    // Laplacian with vertex `pin` removed.
    let pin = 0usize;
    let free = |i: usize| {
        if i < pin {
            Some(i)
        } else if i > pin {
            Some(i - 1)
        } else {
            None
        }
    };
    let mut lap = SymBuilder::new(n - 1);
    for sp in &springs {
        let (i, j, wk) = (sp.i, sp.j, sp.w);
        for (a, b, v) in [(i, i, wk), (j, j, wk), (i, j, -wk), (j, i, -wk)] {
            if let (Some(fa), Some(fb)) = (free(a), free(b)) {
                lap.add(fa, fb, v);
            }
        }
    }
    let solver = lap.factor()?;

    let mut uv = init.to_vec();
    let mut d = vec![DVec2::ZERO; springs.len()];
    let mut prev = f64::INFINITY;
    let mut energy = f64::INFINITY;
    let mut iterations = 0;

    for it in 1..=opts.max_iterations {
        iterations = it;
        // Local step: triangle rotations.
        for (t, tri) in panel.triangles.iter().enumerate() {
            let (mut a, mut b, mut c, mut dd) = (0.0, 0.0, 0.0, 0.0);
            for k in 0..3 {
                let (i, j) = ((k + 1) % 3, (k + 2) % 3);
                let du = uv[tri[i] as usize] - uv[tri[j] as usize];
                let dx = refs[t][i] - refs[t][j];
                let wk = w[t][k];
                a += wk * du.x * dx.x;
                b += wk * du.x * dx.y;
                c += wk * du.y * dx.x;
                dd += wk * du.y * dx.y;
            }
            let (cs, sn) = closest_rotation(a, b, c, dd);
            for k in 0..3 {
                let dx = refs[t][(k + 1) % 3] - refs[t][(k + 2) % 3];
                d[3 * t + k] = DVec2::new(cs * dx.x - sn * dx.y, sn * dx.x + cs * dx.y);
            }
        }
        // Local step: edge-length projections.
        let base = 3 * panel.triangles.len();
        for (k, e) in targets.iter().enumerate() {
            let du = uv[e.a as usize] - uv[e.b as usize];
            let dir = du
                .try_normalize()
                .unwrap_or_else(|| d[base + k].normalize_or(DVec2::X));
            d[base + k] = e.length * dir;
        }

        // Global step: L u = Σ ±w d, with the pinned vertex moved to the right.
        let mut bx = vec![0.0; n - 1];
        let mut by = vec![0.0; n - 1];
        let p0 = uv[pin];
        for (sp, dk) in springs.iter().zip(&d) {
            let r = sp.w * *dk;
            for (a, other, sign) in [(sp.i, sp.j, 1.0), (sp.j, sp.i, -1.0)] {
                if let Some(fa) = free(a) {
                    bx[fa] += sign * r.x;
                    by[fa] += sign * r.y;
                    if other == pin {
                        bx[fa] += sp.w * p0.x;
                        by[fa] += sp.w * p0.y;
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

        energy = springs
            .iter()
            .zip(&d)
            .map(|(sp, dk)| 0.5 * sp.w * (uv[sp.i] - uv[sp.j] - *dk).length_squared())
            .sum();
        if prev.is_finite() && prev - energy <= opts.tolerance * prev {
            break;
        }
        prev = energy;
    }

    Ok(ArapResult {
        uv,
        iterations,
        energy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cut::cut_panels, lscm::lscm, testmesh, SeamSet};
    use approx::assert_relative_eq;

    #[test]
    fn flat_grid_shrinks_by_ease() {
        let m = testmesh::grid(5, 4, 10.0);
        let p = cut_panels(&m, &SeamSet::default().resolve(&m).unwrap())
            .unwrap()
            .remove(0);
        let init = lscm(&p).unwrap();
        let s = 0.85;
        let refs = crate::ease::reference_triangles(&p, &vec![s; p.triangles.len()], 1.0, None);
        let r = arap(&p, &refs, &init, &ArapOptions::default(), &[]).unwrap();
        assert!(r.energy < 1e-16, "energy {}", r.energy);
        for t in &p.triangles {
            for k in 0..3 {
                let (a, b) = (t[k] as usize, t[(k + 1) % 3] as usize);
                assert_relative_eq!(
                    r.uv[a].distance(r.uv[b]),
                    s * p.positions[a].distance(p.positions[b]),
                    epsilon = 1e-8
                );
            }
        }
    }
}
