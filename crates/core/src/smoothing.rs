//! Smoothing a surface at the scale latex bridges.
//!
//! Latex under tension spans hollows and small bumps a few centimetres
//! across (navel, collarbone hollows, nipples) instead of following them,
//! so those details should not shape the pattern. Before flattening, the
//! garment's 3D shape is low-pass filtered:
//!
//! - `G`: `N` steps of `p ← p + λ Δp` (`λ = ½`, `Δp` the umbrella
//!   Laplacian: mean of the neighbours minus the vertex). Each step is a
//!   random-walk step of variance about `λh²/2` per direction (`h` the mean
//!   edge length), so `N = 2σ²/(λh²)` steps blur like a Gaussian of
//!   standard deviation `σ`.
//! - *Twicing* (Tukey; also Taubin's λ|μ idea): `2G(p) − G(G(p))`. Plain
//!   blurring pulls a curved surface in by about `σ²κ/2`; twicing cancels
//!   that to leading order, so the overall size and the large-scale
//!   curvature (bust, buttocks, shoulders) are kept while details much
//!   smaller than `σ` are removed.
//!
//! Seam and outline vertices are held fixed (smoothing them would distort
//! the small triangles where curves meet, and change seam lengths), so
//! what is smoothed is the inside of each piece; any other vertex on the
//! open edge of the mesh is smoothed only along that edge.

use crate::mesh::Mesh;
use glam::DVec3;
use std::collections::HashSet;

const LAMBDA: f64 = 0.5;

/// `mesh` with its vertices smoothed over about `sigma_mm` (see the module
/// docs), except those marked in `fixed` (e.g. seams and the outline,
/// which then keep the body's exact lengths). The triangles are unchanged.
pub fn smooth(mesh: &Mesh, sigma_mm: f64, fixed: &[bool]) -> Mesh {
    let p0 = mesh.positions();
    let n = p0.len();
    if sigma_mm <= 0.0 || n == 0 {
        return mesh.clone();
    }
    // Neighbours; boundary vertices only see their boundary neighbours.
    let mut all: Vec<HashSet<u32>> = vec![HashSet::new(); n];
    let mut boundary: Vec<Vec<u32>> = vec![Vec::new(); n];
    for e in mesh.edges() {
        let [a, b] = e.v;
        all[a as usize].insert(b);
        all[b as usize].insert(a);
        if e.is_boundary() {
            boundary[a as usize].push(b);
            boundary[b as usize].push(a);
        }
    }
    let neighbours: Vec<Vec<u32>> = (0..n)
        .map(|v| {
            if boundary[v].is_empty() {
                let mut x: Vec<u32> = all[v].iter().copied().collect();
                x.sort_unstable();
                x
            } else {
                boundary[v].clone()
            }
        })
        .collect();
    let h = {
        let e = mesh.edges();
        e.iter()
            .map(|e| p0[e.v[0] as usize].distance(p0[e.v[1] as usize]))
            .sum::<f64>()
            / e.len().max(1) as f64
    };
    let steps = (2.0 * sigma_mm.powi(2) / (LAMBDA * h.max(1e-9).powi(2))).ceil() as usize;
    let blur = |start: &[DVec3]| -> Vec<DVec3> {
        let mut p = start.to_vec();
        for _ in 0..steps.min(2000) {
            let q = p.clone();
            for v in 0..n {
                let nb = &neighbours[v];
                if nb.is_empty() || fixed.get(v).copied().unwrap_or(false) {
                    continue;
                }
                let c = nb.iter().map(|&w| q[w as usize]).sum::<DVec3>() / nb.len() as f64;
                p[v] = q[v] + LAMBDA * (c - q[v]);
            }
        }
        p
    };
    let g1 = blur(p0);
    let g2 = blur(&g1);
    let p: Vec<DVec3> = g1.iter().zip(&g2).map(|(a, b)| 2.0 * *a - *b).collect();
    Mesh::new(p, mesh.triangles().to_vec()).expect("same triangles")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testmesh;

    #[test]
    fn removes_a_small_bump_and_keeps_the_large_shape() {
        // A 60 mm-radius tube with a 3 mm, ~10 mm-wide bump on its side.
        let tube = testmesh::cylinder(60.0, 300.0, 64, 60, std::f64::consts::TAU);
        let bump_at = DVec3::new(60.0, 0.0, 150.0);
        let positions: Vec<DVec3> = tube
            .positions()
            .iter()
            .map(|&p| {
                let d = p.distance(bump_at);
                let radial = DVec3::new(p.x, p.y, 0.0).normalize_or_zero();
                p + 3.0 * (-(d * d) / (2.0 * 5.0 * 5.0)).exp() * radial
            })
            .collect();
        let bumpy = Mesh::new(positions, tube.triangles().to_vec()).unwrap();
        let smooth = smooth(&bumpy, 15.0, &[]);
        let radius = |m: &Mesh, i: usize| {
            let p = m.positions()[i];
            p.x.hypot(p.y)
        };
        let top = (0..bumpy.positions().len())
            .min_by(|&a, &b| {
                bumpy.positions()[a]
                    .distance(bump_at)
                    .total_cmp(&bumpy.positions()[b].distance(bump_at))
            })
            .unwrap();
        assert!(radius(&bumpy, top) > 62.5);
        assert!(radius(&smooth, top) < 61.0, "{}", radius(&smooth, top));
        // Away from the bump and the ends, the tube keeps its radius.
        let far = (0..bumpy.positions().len())
            .find(|&i| {
                let p = bumpy.positions()[i];
                p.x < -50.0 && (p.z - 150.0).abs() < 5.0
            })
            .unwrap();
        assert!(
            (radius(&smooth, far) - 60.0).abs() < 0.5,
            "{}",
            radius(&smooth, far)
        );
    }
}
