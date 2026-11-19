//! Plane geometry: the curves of a piece's outline and polyline helpers.
//!
//! ## Curves
//! A piece's outline passes through its nodes. Between two nodes it is a
//! cubic Bézier taken from a **centripetal Catmull–Rom** spline (α = ½),
//! which never forms cusps or loops between the points it passes through.
//! For the segment `P₁ → P₂` with neighbours `P₀`, `P₃` and
//! `dᵢ = |Pᵢ − Pᵢ₋₁|^½`, the inner control points are (Yuksel et al.)
//! ```text
//! B₁ = (d₁² P₂ − d₂² P₀ + (2d₁² + 3d₁d₂ + d₂²) P₁) / (3 d₁ (d₁ + d₂))
//! B₂ = (d₃² P₁ − d₂² P₃ + (2d₃² + 3d₃d₂ + d₂²) P₂) / (3 d₃ (d₃ + d₂))
//! ```
//! At a sharp node the missing neighbour is the reflection of the other end
//! of the segment through the node (`P₀ = 2P₁ − P₂`), which makes the curve
//! leave the node heading straight for the next one; between two sharp
//! nodes the segment is a straight line.
//!
//! Lengths are measured on a fixed sampling of each Bézier (see
//! [`sample_bezier`]); the web editor samples the same way, so the lengths
//! it shows are the ones the pattern uses.

use glam::DVec2;

/// Cubic Bézier control points.
pub type Bezier = [DVec2; 4];

/// Inner control point next to `p1` of the centripetal Catmull–Rom segment
/// `p1 → p2` whose other neighbour is `p0`.
pub fn catmull_rom_handle(p0: DVec2, p1: DVec2, p2: DVec2) -> DVec2 {
    let d1 = p1.distance(p0).sqrt();
    let d2 = p2.distance(p1).sqrt();
    if d1 < 1e-9 || d2 < 1e-9 {
        // Coincident points: fall back to a straight third.
        return p1 + (p2 - p1) / 3.0;
    }
    (d1 * d1 * p2 - d2 * d2 * p0 + (2.0 * d1 * d1 + 3.0 * d1 * d2 + d2 * d2) * p1)
        / (3.0 * d1 * (d1 + d2))
}

/// Bézier of the segment `p1 → p2` with neighbours `p0` (before `p1`) and
/// `p3` (after `p2`).
pub fn segment(p0: DVec2, p1: DVec2, p2: DVec2, p3: DVec2) -> Bezier {
    [
        p1,
        catmull_rom_handle(p0, p1, p2),
        catmull_rom_handle(p3, p2, p1),
        p2,
    ]
}

/// A straight segment as a Bézier.
pub fn line(a: DVec2, b: DVec2) -> Bezier {
    [a, a + (b - a) / 3.0, a + 2.0 * (b - a) / 3.0, b]
}

/// Point on a Bézier at parameter `t`.
pub fn bezier_point(b: &Bezier, t: f64) -> DVec2 {
    let u = 1.0 - t;
    u * u * u * b[0] + 3.0 * u * u * t * b[1] + 3.0 * u * t * t * b[2] + t * t * t * b[3]
}

/// Number of pieces a Bézier is sampled into: about one per 2 mm of its
/// control polygon, between 2 and 128.
pub fn sample_count(b: &Bezier) -> usize {
    let l = b[0].distance(b[1]) + b[1].distance(b[2]) + b[2].distance(b[3]);
    ((l / 2.0).ceil() as usize).clamp(2, 128)
}

/// Points along a Bézier, from `b[0]` to `b[3]` inclusive.
pub fn sample_bezier(b: &Bezier) -> Vec<DVec2> {
    let n = sample_count(b);
    (0..=n)
        .map(|k| bezier_point(b, k as f64 / n as f64))
        .collect()
}

/// Length of an open polyline.
pub fn polyline_length(pts: &[DVec2]) -> f64 {
    pts.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// Where fraction `t` (0–1) of an open polyline's length falls: the segment
/// index and the parameter within it.
pub fn locate_fraction(pts: &[DVec2], t: f64) -> (usize, f64) {
    let total = polyline_length(pts);
    let n = pts.len().saturating_sub(1);
    if n == 0 || total <= 0.0 {
        return (0, 0.0);
    }
    let target = t.clamp(0.0, 1.0) * total;
    let mut acc = 0.0;
    for k in 0..n {
        let l = pts[k].distance(pts[k + 1]);
        if acc + l >= target || k + 1 == n {
            let f = if l > 0.0 { (target - acc) / l } else { 0.0 };
            return (k, f.clamp(0.0, 1.0));
        }
        acc += l;
    }
    (n - 1, 1.0)
}

/// Point at a located position (see [`locate_fraction`]).
pub fn point_at(pts: &[DVec2], (k, f): (usize, f64)) -> DVec2 {
    if pts.len() < 2 {
        return pts.first().copied().unwrap_or(DVec2::ZERO);
    }
    pts[k] + f * (pts[k + 1] - pts[k])
}

/// Unit direction of the segment at a located position.
pub fn direction_at(pts: &[DVec2], (k, _): (usize, f64)) -> DVec2 {
    if pts.len() < 2 {
        return DVec2::X;
    }
    (pts[k + 1] - pts[k]).normalize_or_zero()
}

/// Signed area of a closed polygon (positive if counter-clockwise).
pub fn signed_area(pts: &[DVec2]) -> f64 {
    let n = pts.len();
    (0..n)
        .map(|i| pts[i].perp_dot(pts[(i + 1) % n]))
        .sum::<f64>()
        / 2.0
}

/// Reflection of `p` in the line through `a` and `b`.
pub fn reflect(p: DVec2, a: DVec2, b: DVec2) -> DVec2 {
    let d = (b - a).normalize_or_zero();
    if d == DVec2::ZERO {
        return 2.0 * a - p;
    }
    let v = p - a;
    a + 2.0 * v.dot(d) * d - v
}

/// Whether the open segments `a1–a2` and `b1–b2` cross (touching at an end
/// does not count).
pub fn segments_cross(a1: DVec2, a2: DVec2, b1: DVec2, b2: DVec2) -> bool {
    let d1 = (a2 - a1).perp_dot(b1 - a1);
    let d2 = (a2 - a1).perp_dot(b2 - a1);
    let d3 = (b2 - b1).perp_dot(a1 - b1);
    let d4 = (b2 - b1).perp_dot(a2 - b1);
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// Pairs of non-adjacent segments of a closed polygon that cross. Returns
/// the first crossing found, as segment indices.
pub fn first_self_crossing(pts: &[DVec2]) -> Option<(usize, usize)> {
    let n = pts.len();
    if n < 4 {
        return None;
    }
    // Bounding boxes first: outlines have a few hundred segments.
    let bb: Vec<(DVec2, DVec2)> = (0..n)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            (a.min(b), a.max(b))
        })
        .collect();
    for i in 0..n {
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let ((lo1, hi1), (lo2, hi2)) = (bb[i], bb[j]);
            if lo1.x > hi2.x || lo2.x > hi1.x || lo1.y > hi2.y || lo2.y > hi1.y {
                continue;
            }
            if segments_cross(pts[i], pts[(i + 1) % n], pts[j], pts[(j + 1) % n]) {
                return Some((i, j));
            }
        }
    }
    None
}

/// Axis-aligned bounds of a set of points.
pub fn bounds(pts: &[DVec2]) -> (DVec2, DVec2) {
    pts.iter().fold(
        (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn straight_segments_between_sharp_points() {
        let (a, b) = (DVec2::new(0.0, 0.0), DVec2::new(30.0, 40.0));
        // Reflected neighbours, as at two sharp nodes.
        let s = segment(2.0 * a - b, a, b, 2.0 * b - a);
        for p in sample_bezier(&s) {
            assert_relative_eq!((p - a).perp_dot(b - a), 0.0, epsilon = 1e-9);
        }
        assert_relative_eq!(polyline_length(&sample_bezier(&s)), 50.0, epsilon = 1e-9);
        for (p, q) in s.iter().zip(line(a, b)) {
            assert_relative_eq!(p.distance(q), 0.0, epsilon = 1e-9);
        }
    }

    #[test]
    fn closed_spline_through_a_square_is_nearly_a_circle() {
        // Four points on a circle of radius 100: the spline through them
        // never bulges outside the circle; midway between points it cuts in
        // to (62.5, 62.5), 88 % of the radius (B(½) = (P₁ + 3B₁ + 3B₂ + P₂)/8
        // with handles P₁ ± (P₂ − P₀)/6).
        let p: Vec<DVec2> = (0..4)
            .map(|k| {
                let a = std::f64::consts::FRAC_PI_2 * k as f64;
                100.0 * DVec2::new(a.cos(), a.sin())
            })
            .collect();
        for i in 0..4 {
            let s = segment(p[(i + 3) % 4], p[i], p[(i + 1) % 4], p[(i + 2) % 4]);
            let pts = sample_bezier(&s);
            for q in &pts {
                assert!(q.length() > 88.0 && q.length() < 100.0 + 1e-9, "{q}");
            }
            let mid = bezier_point(&s, 0.5);
            assert_relative_eq!(mid.length(), 62.5 * 2f64.sqrt(), epsilon = 1e-9);
        }
    }

    #[test]
    fn fractions_reflections_and_crossings() {
        let pts = [DVec2::ZERO, DVec2::new(10.0, 0.0), DVec2::new(10.0, 30.0)];
        let at = locate_fraction(&pts, 0.5);
        assert_eq!(at.0, 1);
        assert_relative_eq!(point_at(&pts, at).y, 10.0, epsilon = 1e-12);
        assert_relative_eq!(
            reflect(DVec2::new(3.0, 1.0), DVec2::ZERO, DVec2::Y).x,
            -3.0,
            epsilon = 1e-12
        );
        let square = [
            DVec2::ZERO,
            DVec2::new(1.0, 0.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(0.0, 1.0),
        ];
        assert_relative_eq!(signed_area(&square), 1.0);
        assert_eq!(first_self_crossing(&square), None);
        let bow = [square[0], square[2], square[1], square[3]];
        assert!(first_self_crossing(&bow).is_some());
    }
}
