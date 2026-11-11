//! A triangle mesh that curves can be cut into.
//!
//! Curves drawn on the body are embedded exactly: every point where a curve
//! crosses a mesh edge becomes a new vertex, so afterwards the curve runs
//! along mesh edges and can be used as a seam or a garment edge.
//!
//! Two operations change the mesh, both keeping counter-clockwise winding:
//! - [`EditMesh::split_edge`]: a new vertex at `p_u + t (p_w − p_u)` on edge
//!   `(u, w)`; each adjacent triangle `(a, b, x)` becomes `(a, c, x)` and
//!   `(c, b, x)`.
//! - [`EditMesh::split_face`]: a new vertex inside triangle `(a, b, d)`,
//!   which becomes `(a, b, c)`, `(b, d, c)` and `(d, a, c)`.

use glam::DVec3;

/// A point on a mesh: a triangle and barycentric weights `(1 − u − v, u, v)`
/// of its three corners.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SurfacePoint {
    pub face: u32,
    pub u: f64,
    pub v: f64,
}

impl SurfacePoint {
    /// Barycentric weights of the three corners.
    pub fn weights(&self) -> [f64; 3] {
        [1.0 - self.u - self.v, self.u, self.v]
    }

    /// The 3D position on a mesh with the same triangles.
    pub fn position(&self, positions: &[DVec3], triangles: &[[u32; 3]]) -> Option<DVec3> {
        let t = triangles.get(self.face as usize)?;
        let w = self.weights();
        Some((0..3).map(|k| w[k] * positions[t[k] as usize]).sum())
    }
}

/// Closest point to `p` on triangle `(a, b, c)`, as barycentric `(u, v)`
/// for `b` and `c` (Ericson, *Real-Time Collision Detection*, 5.1.5).
pub fn closest_on_triangle(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> (f64, f64) {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return (0.0, 0.0);
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return (1.0, 0.0);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return (d1 / (d1 - d3), 0.0);
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return (0.0, 1.0);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return (0.0, d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (1.0 - w, w);
    }
    let denom = 1.0 / (va + vb + vc);
    (vb * denom, vc * denom)
}

/// The surface point of `(positions, triangles)` nearest to `p`.
pub fn locate(positions: &[DVec3], triangles: &[[u32; 3]], p: DVec3) -> SurfacePoint {
    let mut best = (
        f64::INFINITY,
        SurfacePoint {
            face: 0,
            u: 0.0,
            v: 0.0,
        },
    );
    for (f, t) in triangles.iter().enumerate() {
        let [a, b, c] = t.map(|i| positions[i as usize]);
        // Cheap reject by bounding sphere.
        let centre = (a + b + c) / 3.0;
        let r2 = a
            .distance_squared(centre)
            .max(b.distance_squared(centre))
            .max(c.distance_squared(centre));
        let d = p.distance(centre) - r2.sqrt();
        if d > 0.0 && d * d > best.0 {
            continue;
        }
        let (u, v) = closest_on_triangle(p, a, b, c);
        let q = a + u * (b - a) + v * (c - a);
        let d2 = q.distance_squared(p);
        if d2 < best.0 {
            best = (
                d2,
                SurfacePoint {
                    face: f as u32,
                    u,
                    v,
                },
            );
        }
    }
    best.1
}

/// A mutable triangle mesh with vertex → face adjacency.
#[derive(Debug, Clone)]
pub struct EditMesh {
    pub positions: Vec<DVec3>,
    pub triangles: Vec<[u32; 3]>,
    vertex_faces: Vec<Vec<u32>>,
    /// Keep vertices on the midplane `x = 0` there (see
    /// [`EditMesh::pinned`]).
    pub keep_midline: bool,
}

impl EditMesh {
    pub fn new(positions: Vec<DVec3>, triangles: Vec<[u32; 3]>) -> Self {
        let mut vertex_faces = vec![Vec::new(); positions.len()];
        for (f, t) in triangles.iter().enumerate() {
            for &v in t {
                vertex_faces[v as usize].push(f as u32);
            }
        }
        Self {
            positions,
            triangles,
            vertex_faces,
            keep_midline: false,
        }
    }

    /// With [`EditMesh::keep_midline`], vertices on the plane `x = 0` stay
    /// exactly on it: they are never moved, relaxed or collapsed away.
    pub fn pinned(&self, v: u32) -> bool {
        self.keep_midline && self.positions[v as usize].x == 0.0
    }

    /// Faces using vertex `v`.
    pub fn faces_of(&self, v: u32) -> &[u32] {
        &self.vertex_faces[v as usize]
    }

    /// Faces using edge `(a, b)`.
    pub fn edge_faces(&self, a: u32, b: u32) -> Vec<u32> {
        self.vertex_faces[a as usize]
            .iter()
            .copied()
            .filter(|&f| self.triangles[f as usize].contains(&b))
            .collect()
    }

    pub fn adjacent(&self, a: u32, b: u32) -> bool {
        self.vertex_faces[a as usize]
            .iter()
            .any(|&f| self.triangles[f as usize].contains(&b))
    }

    /// Area-weighted vertex normal.
    pub fn normal(&self, v: u32) -> DVec3 {
        self.vertex_faces[v as usize]
            .iter()
            .map(|&f| {
                let [a, b, c] = self.triangles[f as usize].map(|i| self.positions[i as usize]);
                (b - a).cross(c - a)
            })
            .sum::<DVec3>()
            .normalize_or_zero()
    }

    fn push_vertex(&mut self, p: DVec3) -> u32 {
        self.positions.push(p);
        self.vertex_faces.push(Vec::new());
        (self.positions.len() - 1) as u32
    }

    fn set_face(&mut self, f: u32, t: [u32; 3]) {
        let old = self.triangles[f as usize];
        for v in old {
            if !t.contains(&v) {
                self.vertex_faces[v as usize].retain(|&g| g != f);
            }
        }
        for v in t {
            if !old.contains(&v) {
                self.vertex_faces[v as usize].push(f);
            }
        }
        self.triangles[f as usize] = t;
    }

    fn add_face(&mut self, t: [u32; 3]) -> u32 {
        let f = self.triangles.len() as u32;
        self.triangles.push(t);
        for v in t {
            self.vertex_faces[v as usize].push(f);
        }
        f
    }

    /// Insert a vertex on edge `(u, w)` at `p_u + t (p_w − p_u)`.
    pub fn split_edge(&mut self, u: u32, w: u32, t: f64) -> u32 {
        let c = self.push_vertex(self.positions[u as usize].lerp(self.positions[w as usize], t));
        for f in self.edge_faces(u, w) {
            let tri = self.triangles[f as usize];
            // Rotate so the shared edge is (tri[0], tri[1]) in winding order.
            let k = (0..3)
                .find(|&k| {
                    let (a, b) = (tri[k], tri[(k + 1) % 3]);
                    (a == u && b == w) || (a == w && b == u)
                })
                .expect("edge in face");
            let (a, b, x) = (tri[k], tri[(k + 1) % 3], tri[(k + 2) % 3]);
            self.set_face(f, [a, c, x]);
            self.add_face([c, b, x]);
        }
        c
    }

    /// Insert a vertex inside face `f` at barycentric `(u, v)`.
    pub fn split_face(&mut self, f: u32, u: f64, v: f64) -> u32 {
        let [a, b, d] = self.triangles[f as usize];
        let p = (1.0 - u - v) * self.positions[a as usize]
            + u * self.positions[b as usize]
            + v * self.positions[d as usize];
        let c = self.push_vertex(p);
        self.set_face(f, [a, b, c]);
        self.add_face([b, d, c]);
        self.add_face([d, a, c]);
        c
    }

    /// Vertices sharing a face with `v`.
    pub fn neighbors(&self, v: u32) -> Vec<u32> {
        let mut n: Vec<u32> = self.vertex_faces[v as usize]
            .iter()
            .flat_map(|&f| self.triangles[f as usize])
            .filter(|&x| x != v)
            .collect();
        n.sort_unstable();
        n.dedup();
        n
    }

    fn face_normal(&self, t: [u32; 3]) -> DVec3 {
        let [a, b, c] = t.map(|i| self.positions[i as usize]);
        (b - a).cross(c - a)
    }

    /// Merge vertex `a` into its neighbour `b`, removing the (two) faces on
    /// edge `(a, b)`. Refused (returns `false`) if the result would not be
    /// a manifold (the link condition: `a` and `b` share exactly the two
    /// opposite vertices) or a face around `a` would flip. Removed faces
    /// become `[u32::MAX; 3]`; see [`EditMesh::live_triangles`].
    pub fn collapse(&mut self, a: u32, b: u32) -> bool {
        if self.pinned(a) && !self.pinned(b) {
            return false;
        }
        let shared = self.edge_faces(a, b);
        if shared.len() != 2 {
            return false;
        }
        let (na, nb) = (self.neighbors(a), self.neighbors(b));
        let common = na.iter().filter(|x| nb.contains(x)).count();
        if common != 2 {
            return false;
        }
        let pb = self.positions[b as usize];
        let moved: Vec<u32> = self.vertex_faces[a as usize]
            .iter()
            .copied()
            .filter(|f| !shared.contains(f))
            .collect();
        for &f in &moved {
            let t = self.triangles[f as usize];
            let before = self.face_normal(t);
            let after = {
                let q = t.map(|i| {
                    if i == a {
                        pb
                    } else {
                        self.positions[i as usize]
                    }
                });
                (q[1] - q[0]).cross(q[2] - q[0])
            };
            if before.dot(after) <= 0.2 * before.length() * after.length() {
                return false;
            }
        }
        for &f in &shared {
            let t = self.triangles[f as usize];
            for v in t {
                self.vertex_faces[v as usize].retain(|&g| g != f);
            }
            self.triangles[f as usize] = [u32::MAX; 3];
        }
        for f in moved {
            let t = self.triangles[f as usize].map(|i| if i == a { b } else { i });
            self.triangles[f as usize] = t;
            self.vertex_faces[b as usize].push(f);
        }
        self.vertex_faces[a as usize].clear();
        true
    }

    /// Triangles not removed by [`EditMesh::collapse`].
    pub fn live_triangles(&self) -> Vec<[u32; 3]> {
        self.triangles
            .iter()
            .copied()
            .filter(|t| t[0] != u32::MAX)
            .collect()
    }

    /// Move `v` a fraction `k` towards the centroid of its neighbours,
    /// within its tangent plane, unless that would flip a face.
    pub fn relax(&mut self, v: u32, k: f64) {
        if self.pinned(v) {
            return;
        }
        let n = self.neighbors(v);
        if n.is_empty() {
            return;
        }
        let p = self.positions[v as usize];
        let c = n.iter().map(|&x| self.positions[x as usize]).sum::<DVec3>() / n.len() as f64;
        let normal = self.normal(v);
        let mut d = c - p;
        d -= d.dot(normal) * normal;
        let q = p + k * d;
        if self.keep_midline && (q.x < 0.0) != (p.x < 0.0) {
            return;
        }
        let faces = self.vertex_faces[v as usize].clone();
        let before: Vec<DVec3> = faces
            .iter()
            .map(|&f| self.face_normal(self.triangles[f as usize]))
            .collect();
        self.positions[v as usize] = q;
        let ok = faces.iter().zip(&before).all(|(&f, b)| {
            let a = self.face_normal(self.triangles[f as usize]);
            a.dot(*b) > 0.2 * a.length() * b.length()
        });
        if !ok {
            self.positions[v as usize] = p;
        }
    }

    /// Make the surface point (on this mesh) a vertex, reusing a corner when
    /// it is within `snap` (barycentric weight) of one and splitting an edge
    /// when it is that close to an edge, to avoid sliver triangles.
    pub fn insert_point(&mut self, sp: SurfacePoint, snap: f64) -> u32 {
        let t = self.triangles[sp.face as usize];
        let w = sp.weights();
        if let Some(k) = (0..3).find(|&k| w[k] > 1.0 - snap) {
            return t[k];
        }
        if let Some(k) = (0..3).find(|&k| w[k] < snap) {
            // Near the edge opposite corner k: between corners k+1 and k+2.
            let (i, j) = ((k + 1) % 3, (k + 2) % 3);
            let s = w[j] / (w[i] + w[j]);
            return self.split_edge(t[i], t[j], s);
        }
        self.split_face(sp.face, sp.u, sp.v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testmesh;
    use crate::Mesh;
    use approx::assert_relative_eq;

    #[test]
    fn closest_point_regions() {
        let (a, b, c) = (DVec3::ZERO, DVec3::X, DVec3::Y);
        let (u, v) = closest_on_triangle(DVec3::new(0.2, 0.3, 5.0), a, b, c);
        assert_relative_eq!(u, 0.2, epsilon = 1e-12);
        assert_relative_eq!(v, 0.3, epsilon = 1e-12);
        assert_eq!(
            closest_on_triangle(DVec3::new(-1.0, -1.0, 0.0), a, b, c),
            (0.0, 0.0)
        );
        let (u, v) = closest_on_triangle(DVec3::new(1.0, 1.0, 0.0), a, b, c);
        assert_relative_eq!(u, 0.5);
        assert_relative_eq!(v, 0.5);
    }

    #[test]
    fn splits_keep_a_valid_oriented_mesh() {
        let g = testmesh::grid(3, 3, 10.0);
        let mut m = EditMesh::new(g.positions().to_vec(), g.triangles().to_vec());
        let c = m.split_edge(5, 10, 0.3);
        assert!(m.adjacent(c, 5) && m.adjacent(c, 10));
        let sp = locate(&m.positions, &m.triangles, DVec3::new(21.0, 23.0, 0.0));
        let d = m.insert_point(sp, 0.05);
        assert_relative_eq!(m.positions[d as usize].x, 21.0, epsilon = 1e-9);
        // Mesh::new checks manifoldness and consistent winding.
        let mesh = Mesh::new(m.positions.clone(), m.triangles.clone()).unwrap();
        let area: f64 = mesh
            .triangles()
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.positions()[i as usize]);
                0.5 * (b - a).cross(c - a).z
            })
            .sum();
        assert_relative_eq!(area, 900.0, epsilon = 1e-9);
        // Snapping to a corner reuses it.
        let corner = locate(&m.positions, &m.triangles, DVec3::new(10.1, 10.1, 0.0));
        assert_eq!(m.insert_point(corner, 0.05), 5);
    }
}
