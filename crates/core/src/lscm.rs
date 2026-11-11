//! Least-squares conformal map (LSCM), used as the initial flattening.
//!
//! For a triangle `T` with isometric 2D frame `q₀, q₁, q₂` and area `A_T`, a
//! piecewise-linear map `q ↦ (u, v)` has constant Jacobian
//! `J = [[∂u/∂x, ∂u/∂y], [∂v/∂x, ∂v/∂y]]`, with
//! `∇u = Σ_j u_j g_j` and the barycentric gradients
//! `g_j = perp(q_{j+2} − q_{j+1}) / (2 A_T)`, `perp(x, y) = (−y, x)`.
//!
//! The map is conformal on `T` iff `J` is a similarity matrix
//! `[[a, −b], [b, a]]`, i.e. the Cauchy–Riemann residuals vanish:
//! ```text
//! r₁ = ∂u/∂x − ∂v/∂y = Σ_j (u_j g_j.x − v_j g_j.y)
//! r₂ = ∂u/∂y + ∂v/∂x = Σ_j (u_j g_j.y + v_j g_j.x)
//! ```
//! LSCM minimises `E = Σ_T A_T (r₁² + r₂²)`, a quadratic in the unknowns
//! `(u, v)`. Pinning two vertices removes the similarity null space; the
//! normal equations `M_ff x_f = −M_fp x_p` are SPD and solved by sparse
//! Cholesky. Similarity Jacobians have `det J = a² + b² ≥ 0`, so the result
//! is orientation-preserving.
//!
//! Pins: the two boundary vertices farthest apart in 3D, placed at
//! `(0, 0)` and `(d, 0)` with `d` their 3D distance.

use crate::error::{Error, Result};
use crate::linalg::{signed_area, triangle_frame, SymBuilder};
use crate::panel::Panel;
use glam::DVec2;

/// Choose two pin vertices: the farthest-apart pair on the boundary.
pub fn choose_pins(panel: &Panel) -> (u32, u32) {
    let b = panel.boundary_vertices();
    let mut best = (b[0], b[b.len() / 2], -1.0);
    for (i, &a) in b.iter().enumerate() {
        for &c in &b[i + 1..] {
            let d = panel.positions[a as usize].distance_squared(panel.positions[c as usize]);
            if d > best.2 {
                best = (a, c, d);
            }
        }
    }
    (best.0, best.1)
}

/// Flatten `panel` conformally. Returns one 2D position per local vertex.
pub fn lscm(panel: &Panel) -> Result<Vec<DVec2>> {
    let n = panel.vertex_count();
    let (pa, pb) = choose_pins(panel);
    let d = panel.positions[pa as usize].distance(panel.positions[pb as usize]);

    // Unknown layout: u_i at index i, v_i at index n + i.
    let mut pinned: Vec<Option<f64>> = vec![None; 2 * n];
    pinned[pa as usize] = Some(0.0);
    pinned[n + pa as usize] = Some(0.0);
    pinned[pb as usize] = Some(d);
    pinned[n + pb as usize] = Some(0.0);

    let mut full = SymBuilder::new(2 * n);
    for tri in &panel.triangles {
        let q = triangle_frame(tri.map(|i| panel.positions[i as usize]));
        let area = signed_area(q);
        let g: [DVec2; 3] =
            std::array::from_fn(|j| (q[(j + 2) % 3] - q[(j + 1) % 3]).perp() / (2.0 * area));
        let idx = |j: usize| tri[j] as usize;
        let rows: [[(usize, f64); 6]; 2] = [
            std::array::from_fn(|m| {
                if m < 3 {
                    (idx(m), g[m].x)
                } else {
                    (n + idx(m - 3), -g[m - 3].y)
                }
            }),
            std::array::from_fn(|m| {
                if m < 3 {
                    (idx(m), g[m].y)
                } else {
                    (n + idx(m - 3), g[m - 3].x)
                }
            }),
        ];
        for row in &rows {
            for &(i, a) in row {
                for &(j, b) in row {
                    full.add(i, j, area * a * b);
                }
            }
        }
    }

    // Eliminate pinned unknowns.
    let mut free_index = vec![usize::MAX; 2 * n];
    let mut nf = 0;
    for (k, p) in pinned.iter().enumerate() {
        if p.is_none() {
            free_index[k] = nf;
            nf += 1;
        }
    }
    let mut reduced = SymBuilder::new(nf);
    let mut rhs = vec![0.0; nf];
    for (i, j, v) in full.iter() {
        match (free_index[i], pinned[j]) {
            (fi, None) if fi != usize::MAX => reduced.add(fi, free_index[j], v),
            (fi, Some(xj)) if fi != usize::MAX => rhs[fi] -= v * xj,
            _ => {}
        }
    }
    let x = reduced.factor()?.solve(&[rhs])?.pop().unwrap();

    let value = |k: usize| pinned[k].unwrap_or_else(|| x[free_index[k]]);
    let uv: Vec<DVec2> = (0..n).map(|i| DVec2::new(value(i), value(n + i))).collect();
    if uv.iter().any(|p| !p.is_finite()) {
        return Err(Error::Solve("LSCM produced non-finite coordinates".into()));
    }
    Ok(uv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cut::cut_panels, testmesh, SeamSet};
    use approx::assert_relative_eq;

    fn single_panel(m: &crate::Mesh) -> Panel {
        cut_panels(m, &SeamSet::default().resolve(m).unwrap())
            .unwrap()
            .remove(0)
    }

    #[test]
    fn flat_grid_is_reproduced_up_to_similarity() {
        let m = testmesh::grid(5, 4, 10.0);
        let p = single_panel(&m);
        let uv = lscm(&p).unwrap();
        // Distances must be preserved (pins are at true 3D distance).
        for t in &p.triangles {
            for k in 0..3 {
                let (a, b) = (t[k] as usize, t[(k + 1) % 3] as usize);
                assert_relative_eq!(
                    uv[a].distance(uv[b]),
                    p.positions[a].distance(p.positions[b]),
                    epsilon = 1e-8
                );
            }
            let q = t.map(|i| uv[i as usize]);
            assert!(signed_area(q) > 0.0);
        }
    }
}
