//! Small linear-algebra helpers: a sparse SPD solver wrapper around
//! `faer`, isometric triangle frames, and closed-form 2×2 decompositions.

use crate::error::{Error, Result};
use faer::linalg::solvers::Solve;
use faer::sparse::{SparseColMat, Triplet};
use faer::{Mat, Side};
use glam::{DVec2, DVec3};
use std::collections::BTreeMap;

/// Accumulates entries of a symmetric matrix. Duplicate `(i, j)` entries
/// are summed. Callers add both `(i, j)` and `(j, i)`.
#[derive(Debug, Default, Clone)]
pub struct SymBuilder {
    n: usize,
    entries: BTreeMap<(usize, usize), f64>,
}

impl SymBuilder {
    pub fn new(n: usize) -> Self {
        Self {
            n,
            entries: BTreeMap::new(),
        }
    }

    pub fn add(&mut self, i: usize, j: usize, v: f64) {
        *self.entries.entry((i, j)).or_insert(0.0) += v;
    }

    /// Iterate over accumulated `(i, j, value)` entries.
    pub fn iter(&self) -> impl Iterator<Item = (usize, usize, f64)> + '_ {
        self.entries.iter().map(|(&(i, j), &v)| (i, j, v))
    }

    /// Cholesky-factor the matrix (`LLᵀ`); it must be symmetric positive definite.
    pub fn factor(&self) -> Result<SpdSolver> {
        let trips: Vec<Triplet<usize, usize, f64>> = self
            .iter()
            .filter(|&(i, j, _)| i >= j)
            .map(|(i, j, v)| Triplet::new(i, j, v))
            .collect();
        let a = SparseColMat::<usize, f64>::try_new_from_triplets(self.n, self.n, &trips)
            .map_err(|e| Error::Solve(format!("{e:?}")))?;
        let llt = a
            .sp_cholesky(Side::Lower)
            .map_err(|e| Error::Solve(format!("Cholesky failed: {e:?}")))?;
        Ok(SpdSolver { n: self.n, llt })
    }
}

/// A factored sparse symmetric positive definite matrix.
pub struct SpdSolver {
    n: usize,
    llt: faer::sparse::linalg::solvers::Llt<usize, f64>,
}

impl SpdSolver {
    /// Solve `A X = B` for several right-hand sides (each of length `n`).
    pub fn solve(&self, rhs: &[Vec<f64>]) -> Result<Vec<Vec<f64>>> {
        let b = Mat::<f64>::from_fn(self.n, rhs.len(), |i, j| rhs[j][i]);
        let x = self.llt.solve(&b);
        let out: Vec<Vec<f64>> = (0..rhs.len())
            .map(|j| (0..self.n).map(|i| x[(i, j)]).collect())
            .collect();
        if out.iter().flatten().all(|v| v.is_finite()) {
            Ok(out)
        } else {
            Err(Error::Solve("non-finite solution".into()))
        }
    }
}

/// Lay a 3D triangle isometrically into the plane:
/// `q₀ = (0, 0)`, `q₁ = (|e₁|, 0)`, `q₂ = (e₁·e₂ / |e₁|, |e₁×e₂| / |e₁|)`
/// with `e₁ = p₁ − p₀`, `e₂ = p₂ − p₀`. The result is counter-clockwise.
pub fn triangle_frame(p: [DVec3; 3]) -> [DVec2; 3] {
    let e1 = p[1] - p[0];
    let e2 = p[2] - p[0];
    let l1 = e1.length();
    [
        DVec2::ZERO,
        DVec2::new(l1, 0.0),
        DVec2::new(e1.dot(e2) / l1, e1.cross(e2).length() / l1),
    ]
}

/// Signed area of a 2D triangle (positive if counter-clockwise).
pub fn signed_area(q: [DVec2; 3]) -> f64 {
    0.5 * (q[1] - q[0]).perp_dot(q[2] - q[0])
}

/// Cotangent of the angle at corner `k` of a 2D triangle:
/// `cot θ = (a·b) / |a×b|` with `a = q_{k+1} − q_k`, `b = q_{k+2} − q_k`.
pub fn cot_at(q: [DVec2; 3], k: usize) -> f64 {
    let a = q[(k + 1) % 3] - q[k];
    let b = q[(k + 2) % 3] - q[k];
    a.dot(b) / a.perp_dot(b).abs()
}

/// The rotation closest to a 2×2 matrix `S = [[a, b], [c, d]]` (row-major),
/// i.e. the `R ∈ SO(2)` maximising `tr(Rᵀ S)`.
///
/// This equals `U Vᵀ` from the SVD `S = U Σ Vᵀ`, with the sign of the
/// last column of `U` flipped when `det(U Vᵀ) < 0`. In 2D it has the closed
/// form `R = rot(θ)` with `θ = atan2(c − b, a + d)`, because
/// `tr(rot(θ)ᵀ S) = (a + d) cos θ + (c − b) sin θ`.
///
/// Returns `(cos θ, sin θ)`.
pub fn closest_rotation(a: f64, b: f64, c: f64, d: f64) -> (f64, f64) {
    let th = (c - b).atan2(a + d);
    (th.cos(), th.sin())
}

/// Eigenvalues `(λ_min, λ_max)` of the symmetric matrix `[[a, b], [b, d]]`:
/// `λ = (a + d)/2 ∓ √(((a − d)/2)² + b²)`.
pub fn sym2_eigenvalues(a: f64, b: f64, d: f64) -> (f64, f64) {
    let m = 0.5 * (a + d);
    let r = (0.25 * (a - d) * (a - d) + b * b).sqrt();
    (m - r, m + r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn spd_solve() {
        // [[4, 1], [1, 3]] x = [1, 2] → x = [1/11, 7/11]
        let mut b = SymBuilder::new(2);
        b.add(0, 0, 4.0);
        b.add(0, 1, 1.0);
        b.add(1, 0, 1.0);
        b.add(1, 1, 2.0);
        b.add(1, 1, 1.0); // duplicates sum
        let x = b.factor().unwrap().solve(&[vec![1.0, 2.0]]).unwrap();
        assert_relative_eq!(x[0][0], 1.0 / 11.0, epsilon = 1e-12);
        assert_relative_eq!(x[0][1], 7.0 / 11.0, epsilon = 1e-12);
    }

    #[test]
    fn frame_is_isometric() {
        let p = [
            DVec3::new(1., 2., 3.),
            DVec3::new(4., 0., 1.),
            DVec3::new(0., 5., -2.),
        ];
        let q = triangle_frame(p);
        for (i, j) in [(0, 1), (1, 2), (2, 0)] {
            assert_relative_eq!(
                (q[i] - q[j]).length(),
                (p[i] - p[j]).length(),
                epsilon = 1e-12
            );
        }
        assert!(signed_area(q) > 0.0);
    }

    #[test]
    fn rotation_fit_recovers_rotation_and_rejects_reflection() {
        let (c, s) = (0.3f64.cos(), 0.3f64.sin());
        // S = 2 * R(0.3)
        let (rc, rs) = closest_rotation(2.0 * c, -2.0 * s, 2.0 * s, 2.0 * c);
        assert_relative_eq!(rc, c, epsilon = 1e-12);
        assert_relative_eq!(rs, s, epsilon = 1e-12);
        // A reflection diag(1, -1) has closest rotation with det +1.
        let (rc, rs) = closest_rotation(1.0, 0.0, 0.0, -1.0);
        assert_relative_eq!(rc * rc + rs * rs, 1.0, epsilon = 1e-12);
    }

    #[test]
    fn eigenvalues() {
        let (l0, l1) = sym2_eigenvalues(2.0, 1.0, 2.0);
        assert_relative_eq!(l0, 1.0, epsilon = 1e-12);
        assert_relative_eq!(l1, 3.0, epsilon = 1e-12);
    }
}
