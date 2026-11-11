//! Tape-measure girths by slicing the body with a plane.
//!
//! A girth is measured the way a tape measure does it: cut the surface
//! with a plane, take the closed cross-section curve around the limb or
//! torso in question, and measure the perimeter of its **convex hull** (a
//! tape bridges hollows such as the cleavage or the small of the back).
//!
//! 1. Every triangle crossing the plane contributes one segment between the
//!    two points where its edges cross (`p = a + t (b − a)` with
//!    `t = d_a / (d_a − d_b)` for signed distances `d`).
//! 2. Segments are linked into loops through shared mesh edges.
//! 3. The loop whose centroid is nearest the requested centre is kept; a
//!    horizontal cut at chest height also crosses the arms, for example.
//! 4. Its points are projected into the plane and the convex hull
//!    perimeter is computed (Andrew's monotone chain).

use glam::{DVec2, DVec3};
use std::collections::HashMap;

/// A cutting plane with a preferred centre for choosing the loop.
#[derive(Debug, Clone, Copy)]
pub struct Plane {
    pub point: DVec3,
    /// Unit normal.
    pub normal: DVec3,
}

/// One closed cross-section.
#[derive(Debug, Clone)]
pub struct Section {
    /// Loop points in order.
    pub points: Vec<DVec3>,
    pub centroid: DVec3,
}

impl Section {
    /// Perimeter of the loop itself.
    pub fn contour_length(&self) -> f64 {
        let n = self.points.len();
        (0..n)
            .map(|i| self.points[i].distance(self.points[(i + 1) % n]))
            .sum()
    }

    /// Perimeter of the convex hull of the loop within `plane`.
    pub fn hull_length(&self, plane: &Plane) -> f64 {
        let u = plane.normal.any_orthonormal_vector();
        let v = plane.normal.cross(u);
        let mut pts: Vec<DVec2> = self
            .points
            .iter()
            .map(|p| DVec2::new((*p - plane.point).dot(u), (*p - plane.point).dot(v)))
            .collect();
        pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        let mut hull: Vec<DVec2> = Vec::with_capacity(pts.len() + 1);
        for pass in 0..2 {
            let start = hull.len();
            let iter: Box<dyn Iterator<Item = &DVec2>> = if pass == 0 {
                Box::new(pts.iter())
            } else {
                Box::new(pts.iter().rev())
            };
            for &p in iter {
                while hull.len() >= start + 2 {
                    let (a, b) = (hull[hull.len() - 2], hull[hull.len() - 1]);
                    if (b - a).perp_dot(p - b) <= 0.0 {
                        hull.pop();
                    } else {
                        break;
                    }
                }
                hull.push(p);
            }
            hull.pop();
        }
        let n = hull.len();
        (0..n).map(|i| hull[i].distance(hull[(i + 1) % n])).sum()
    }
}

/// All closed cross-sections of the triangles `tris` (a subset of a mesh
/// with vertex `positions`) with `plane`.
pub fn sections(positions: &[DVec3], tris: &[[u32; 3]], plane: &Plane) -> Vec<Section> {
    let dist = |i: u32| (positions[i as usize] - plane.point).dot(plane.normal);
    let key = |a: u32, b: u32| if a < b { (a, b) } else { (b, a) };
    // Crossed mesh edge -> the (up to two) segments that end on it.
    let mut ends: HashMap<(u32, u32), [usize; 2]> = HashMap::new();
    let mut segs: Vec<[(u32, u32); 2]> = Vec::new();
    for t in tris {
        let d = [dist(t[0]), dist(t[1]), dist(t[2])];
        // Treat exact zeros as positive so every crossing is counted once.
        let s = d.map(|x| x >= 0.0);
        if s[0] == s[1] && s[1] == s[2] {
            continue;
        }
        let mut e = [(0, 0); 2];
        let mut n = 0;
        for k in 0..3 {
            if s[k] != s[(k + 1) % 3] {
                e[n] = key(t[k], t[(k + 1) % 3]);
                n += 1;
            }
        }
        let id = segs.len();
        for k in e {
            let slot = ends.entry(k).or_insert([usize::MAX; 2]);
            if slot[0] == usize::MAX {
                slot[0] = id;
            } else {
                slot[1] = id;
            }
        }
        segs.push(e);
    }
    let point = |(a, b): (u32, u32)| {
        let (da, db) = (dist(a), dist(b));
        let t = da / (da - db);
        positions[a as usize].lerp(positions[b as usize], t)
    };

    let mut used = vec![false; segs.len()];
    let mut out = Vec::new();
    for start in 0..segs.len() {
        if used[start] {
            continue;
        }
        let mut pts = Vec::new();
        let mut cur = start;
        let mut edge = segs[start][0];
        let mut closed = false;
        loop {
            used[cur] = true;
            pts.push(point(edge));
            // Leave through the segment's other edge.
            let exit = if segs[cur][0] == edge {
                segs[cur][1]
            } else {
                segs[cur][0]
            };
            let pair = ends[&exit];
            let next = if pair[0] == cur { pair[1] } else { pair[0] };
            if next == usize::MAX {
                break; // open chain (mesh boundary or subset edge)
            }
            if next == start {
                closed = true;
                break;
            }
            if used[next] {
                break;
            }
            edge = exit;
            cur = next;
        }
        if closed && pts.len() >= 3 {
            let centroid = pts.iter().sum::<DVec3>() / pts.len() as f64;
            out.push(Section {
                points: pts,
                centroid,
            });
        }
    }
    out
}

/// The closed loop nearest `plane.point`, or `None` if the plane does not
/// cut a closed loop.
pub fn nearest_section(positions: &[DVec3], tris: &[[u32; 3]], plane: &Plane) -> Option<Section> {
    sections(positions, tris, plane).into_iter().min_by(|a, b| {
        a.centroid
            .distance_squared(plane.point)
            .total_cmp(&b.centroid.distance_squared(plane.point))
    })
}

/// Tape girth around the loop nearest `plane.point`.
pub fn girth(positions: &[DVec3], tris: &[[u32; 3]], plane: &Plane) -> Option<f64> {
    nearest_section(positions, tris, plane).map(|s| s.hull_length(plane))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testmesh;
    use approx::assert_relative_eq;
    use std::f64::consts::{PI, TAU};

    #[test]
    fn cylinder_girth_is_polygon_perimeter() {
        // Axis is Z in the test mesh.
        let m = testmesh::cylinder(50.0, 200.0, 32, 8, TAU);
        let plane = Plane {
            point: DVec3::new(0.0, 0.0, 90.0),
            normal: DVec3::Z,
        };
        let g = girth(m.positions(), m.triangles(), &plane).unwrap();
        assert_relative_eq!(g, 32.0 * 2.0 * 50.0 * (PI / 32.0).sin(), epsilon = 1e-9);
        // A tilted cut of a cylinder is an ellipse with a longer perimeter.
        let tilted = Plane {
            point: DVec3::new(0.0, 0.0, 100.0),
            normal: DVec3::new(0.0, 0.5, 1.0).normalize(),
        };
        assert!(girth(m.positions(), m.triangles(), &tilted).unwrap() > g * 1.02);
        // No loop outside the tube.
        let outside = Plane {
            point: DVec3::new(0.0, 0.0, 500.0),
            normal: DVec3::Z,
        };
        assert!(girth(m.positions(), m.triangles(), &outside).is_none());
    }

    #[test]
    fn hull_bridges_hollows() {
        let s = Section {
            points: [(0., 0.), (4., 0.), (4., 4.), (2., 1.), (0., 4.)]
                .iter()
                .map(|&(x, y)| DVec3::new(x, y, 0.0))
                .collect(),
            centroid: DVec3::ZERO,
        };
        let plane = Plane {
            point: DVec3::ZERO,
            normal: DVec3::Z,
        };
        assert_relative_eq!(s.hull_length(&plane), 16.0, epsilon = 1e-12);
        assert!(s.contour_length() > 16.0);
    }
}
