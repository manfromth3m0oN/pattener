//! Glue-overlap allowance: offsetting a piece's outline edge by edge.
//!
//! Latex seams are glued lap seams: one piece carries an extra strip
//! (the allowance) that is glued under the other piece, which is cut on the
//! seam line. Hems can carry an allowance that is folded and glued back.
//! The *cut line* is therefore the boundary polygon with some edges offset
//! outwards.
//!
//! ## Construction
//! The boundary is counter-clockwise, so the outward normal of an edge with
//! unit direction `t` is `n = (t.y, −t.x)`. Edge `k` is moved out by
//! `d_k ≥ 0`. At each vertex `v` between incoming edge `1` and outgoing
//! edge `2`:
//! - `d₁ = d₂ = 0`: keep `v`;
//! - both `> 0`: mitre, the intersection of the two offset lines
//!   `v + d₁n₁ + λt₁ = v + d₂n₂ + μt₂`, solved as a 2×2 system; if the
//!   lines are parallel or the mitre is longer than [`MITRE_LIMIT`]`·max(d)`,
//!   bevel with the two points `v + d₁n₁`, `v + d₂n₂`;
//! - only one `> 0`: square the allowance off, `v + d₁n₁` then `v` (or
//!   `v` then `v + d₂n₂`).
//!
//! Where the boundary curves inwards more tightly than the allowance width
//! the offset can cross itself; [`is_simple`] detects this so it can be
//! reported.

use glam::DVec2;

/// Mitres longer than this multiple of the offset are bevelled.
pub const MITRE_LIMIT: f64 = 4.0;

/// Offset a counter-clockwise polygon outwards, edge `k` (from `pts[k]` to
/// `pts[k+1]`) by `offsets[k]`. Returns the new polygon.
pub fn offset_polygon(pts: &[DVec2], offsets: &[f64]) -> Vec<DVec2> {
    assert_eq!(pts.len(), offsets.len());
    let n = pts.len();
    let dir = |k: usize| (pts[(k + 1) % n] - pts[k]).normalize_or_zero();
    let normal = |t: DVec2| DVec2::new(t.y, -t.x);
    let mut out = Vec::with_capacity(n + 8);
    for k in 0..n {
        let v = pts[k];
        let k1 = (k + n - 1) % n;
        let (d1, d2) = (offsets[k1], offsets[k]);
        let (t1, t2) = (dir(k1), dir(k));
        let (n1, n2) = (normal(t1), normal(t2));
        match (d1 > 0.0, d2 > 0.0) {
            (false, false) => out.push(v),
            (true, false) => {
                out.push(v + d1 * n1);
                out.push(v);
            }
            (false, true) => {
                out.push(v);
                out.push(v + d2 * n2);
            }
            (true, true) => {
                // λ t₁ − μ t₂ = d₂n₂ − d₁n₁
                let rhs = d2 * n2 - d1 * n1;
                let det = -t1.perp_dot(t2);
                let limit = MITRE_LIMIT * d1.max(d2);
                if det.abs() > 1e-9 {
                    let lambda = rhs.perp_dot(-t2) / det;
                    let p = v + d1 * n1 + lambda * t1;
                    if (p - v).length() <= limit {
                        out.push(p);
                        continue;
                    }
                }
                if (d1 * n1 - d2 * n2).length() < 1e-9 {
                    out.push(v + d1 * n1);
                } else {
                    out.push(v + d1 * n1);
                    out.push(v + d2 * n2);
                }
            }
        }
    }
    out
}

/// Remove self-intersection loops from a closed polygon: while two
/// non-adjacent edges `i` and `j` cross at `X`, split the polygon there into
/// `X, p_{i+1} … p_j` and `p_0 … p_i, X, p_{j+1} …`, and keep the part with
/// the larger area. At an inward corner, an allowance offset folds over
/// itself in a small loop; this trims it to the outline you would cut.
pub fn remove_loops(mut pts: Vec<DVec2>) -> Vec<DVec2> {
    for _ in 0..pts.len() {
        let n = pts.len();
        let mut hit = None;
        'search: for i in 0..n {
            for j in i + 2..n {
                if i == 0 && j == n - 1 {
                    continue;
                }
                let (a, b, c, d) = (pts[i], pts[(i + 1) % n], pts[j], pts[(j + 1) % n]);
                if segments_cross(a, b, c, d) {
                    // Intersection: a + t (b − a) with t = ((c − a) × (d − c)) / ((b − a) × (d − c)).
                    let t = (c - a).perp_dot(d - c) / (b - a).perp_dot(d - c);
                    hit = Some((i, j, a + t * (b - a)));
                    break 'search;
                }
            }
        }
        let Some((i, j, x)) = hit else {
            return pts;
        };
        let inner: Vec<DVec2> = std::iter::once(x)
            .chain(pts[i + 1..=j].iter().copied())
            .collect();
        let outer: Vec<DVec2> = pts[..=i]
            .iter()
            .copied()
            .chain(std::iter::once(x))
            .chain(pts[j + 1..].iter().copied())
            .collect();
        pts = if polygon_area(&outer).abs() >= polygon_area(&inner).abs() {
            outer
        } else {
            inner
        };
    }
    pts
}

/// `true` if no two non-adjacent edges of the closed polygon intersect.
pub fn is_simple(pts: &[DVec2]) -> bool {
    let n = pts.len();
    let seg = |k: usize| (pts[k], pts[(k + 1) % n]);
    for i in 0..n {
        for j in i + 1..n {
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            let ((a, b), (c, d)) = (seg(i), seg(j));
            if segments_cross(a, b, c, d) {
                return false;
            }
        }
    }
    true
}

fn segments_cross(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> bool {
    let o = |p: DVec2, q: DVec2, r: DVec2| (q - p).perp_dot(r - p);
    let (d1, d2) = (o(c, d, a), o(c, d, b));
    let (d3, d4) = (o(a, b, c), o(a, b, d));
    (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0) && d1 != 0.0 && d2 != 0.0
}

/// Signed area of a polygon (positive if counter-clockwise).
pub fn polygon_area(pts: &[DVec2]) -> f64 {
    let n = pts.len();
    0.5 * (0..n)
        .map(|k| pts[k].perp_dot(pts[(k + 1) % n]))
        .sum::<f64>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn square(side: f64, per_edge: usize) -> Vec<DVec2> {
        let corners = [DVec2::ZERO, DVec2::X, DVec2::ONE, DVec2::Y].map(|c| c * side);
        let mut pts = Vec::new();
        for k in 0..4 {
            for j in 0..per_edge {
                let f = j as f64 / per_edge as f64;
                pts.push(corners[k].lerp(corners[(k + 1) % 4], f));
            }
        }
        pts
    }

    #[test]
    fn uniform_offset_of_square_is_mitred() {
        let pts = square(20.0, 2);
        let out = offset_polygon(&pts, &vec![5.0; pts.len()]);
        assert_relative_eq!(polygon_area(&out), 30.0 * 30.0, epsilon = 1e-9);
        assert!(is_simple(&out));
    }

    #[test]
    fn single_side_allowance_is_squared_off() {
        // Bottom side = edges 0 and 1.
        let pts = square(20.0, 2);
        let mut d = vec![0.0; pts.len()];
        d[0] = 6.0;
        d[1] = 6.0;
        let out = offset_polygon(&pts, &d);
        assert_relative_eq!(polygon_area(&out), 400.0 + 20.0 * 6.0, epsilon = 1e-9);
        assert!(out.contains(&DVec2::new(0.0, -6.0)) && out.contains(&DVec2::new(20.0, -6.0)));
        assert!(is_simple(&out));
    }

    #[test]
    fn zero_offsets_are_identity() {
        let pts = square(10.0, 3);
        assert_eq!(offset_polygon(&pts, &vec![0.0; pts.len()]), pts);
    }

    #[test]
    fn loops_are_trimmed_to_the_outer_outline() {
        // A square whose bottom edge folds back on itself in a small loop:
        // (0,0) → (6,0) → (4,-1) → (4,1) → (10,0) …
        let pts = vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(6.0, 0.0),
            DVec2::new(4.0, -1.0),
            DVec2::new(4.0, 1.0),
            DVec2::new(10.0, 0.0),
            DVec2::new(10.0, 10.0),
            DVec2::new(0.0, 10.0),
        ];
        assert!(!is_simple(&pts));
        let fixed = remove_loops(pts);
        assert!(is_simple(&fixed));
        assert!(fixed.len() < 7);
        assert!(
            (polygon_area(&fixed) - 100.0).abs() < 6.0,
            "{}",
            polygon_area(&fixed)
        );
        // A simple polygon is left alone.
        let sq = square(10.0, 2);
        assert_eq!(remove_loops(sq.clone()), sq);
    }

    #[test]
    fn tight_inward_curve_is_detected() {
        // A thin U shape: a 2 mm wide slot offset by 5 mm from both sides crosses itself.
        let pts = vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(30.0, 0.0),
            DVec2::new(30.0, 30.0),
            DVec2::new(16.0, 30.0),
            DVec2::new(16.0, 10.0),
            DVec2::new(14.0, 10.0),
            DVec2::new(14.0, 30.0),
            DVec2::new(0.0, 30.0),
        ];
        assert!(is_simple(&pts));
        let d = [0.0, 0.0, 0.0, 5.0, 5.0, 5.0, 0.0, 0.0];
        assert!(!is_simple(&offset_polygon(&pts, &d)));
    }
}
