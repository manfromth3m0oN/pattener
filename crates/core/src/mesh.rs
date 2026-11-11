//! Indexed triangle mesh with an edge table.
//!
//! The mesh is stored as a flat list of positions plus a list of
//! counter-clockwise triangles (`[u32; 3]`). On construction we build an
//! edge table that maps every undirected edge to the (one or two)
//! triangles that use it. This is all the adjacency the pipeline needs;
//! a full half-edge structure would add complexity without benefit here.

use crate::error::{Error, Result};
use glam::DVec3;
use std::collections::HashMap;

/// Index of an undirected edge in [`Mesh::edges`].
pub type EdgeId = u32;

/// An undirected mesh edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    /// Endpoints with `v[0] < v[1]`.
    pub v: [u32; 2],
    /// Adjacent triangles; only the first `face_count` entries are valid.
    pub faces: [u32; 2],
    /// Number of valid entries in `faces` (1 or 2).
    pub face_count: u8,
}

impl Edge {
    /// `true` if only one triangle uses this edge.
    pub fn is_boundary(&self) -> bool {
        self.face_count == 1
    }

    /// The triangle across this edge from `face`, if any.
    pub fn other_face(&self, face: u32) -> Option<u32> {
        if self.face_count < 2 {
            None
        } else if self.faces[0] == face {
            Some(self.faces[1])
        } else {
            Some(self.faces[0])
        }
    }
}

/// A validated, consistently oriented, edge-manifold triangle mesh.
#[derive(Debug, Clone)]
pub struct Mesh {
    positions: Vec<DVec3>,
    triangles: Vec<[u32; 3]>,
    edges: Vec<Edge>,
    edge_lookup: HashMap<[u32; 2], EdgeId>,
    /// `face_edges[f][k]` is the edge from corner `k` to corner `k+1`.
    face_edges: Vec<[EdgeId; 3]>,
}

fn key(a: u32, b: u32) -> [u32; 2] {
    if a < b {
        [a, b]
    } else {
        [b, a]
    }
}

impl Mesh {
    /// Build a mesh, checking that
    /// - every index is in range,
    /// - no triangle is degenerate,
    /// - every edge has at most two triangles (edge-manifold),
    /// - adjacent triangles have consistent winding (each directed edge is
    ///   used at most once).
    pub fn new(positions: Vec<DVec3>, triangles: Vec<[u32; 3]>) -> Result<Self> {
        let n = positions.len();
        let mut edges: Vec<Edge> = Vec::with_capacity(triangles.len() * 3 / 2 + 1);
        let mut edge_lookup = HashMap::with_capacity(edges.capacity());
        let mut directed: HashMap<[u32; 2], ()> = HashMap::with_capacity(triangles.len() * 3);
        let mut face_edges = Vec::with_capacity(triangles.len());

        for (t, tri) in triangles.iter().enumerate() {
            for &v in tri {
                if v as usize >= n {
                    return Err(Error::VertexOutOfRange {
                        triangle: t,
                        vertex: v,
                        count: n,
                    });
                }
            }
            let [a, b, c] = *tri;
            if a == b || b == c || a == c {
                return Err(Error::DegenerateTriangle { triangle: t });
            }
            let (pa, pb, pc) = (
                positions[a as usize],
                positions[b as usize],
                positions[c as usize],
            );
            let area2 = (pb - pa).cross(pc - pa).length();
            let scale = (pb - pa).length_squared().max((pc - pa).length_squared());
            if !(area2 > 1e-12 * scale) {
                return Err(Error::DegenerateTriangle { triangle: t });
            }

            let mut fe = [0; 3];
            for k in 0..3 {
                let (u, v) = (tri[k], tri[(k + 1) % 3]);
                if directed.insert([u, v], ()).is_some() {
                    return Err(Error::InconsistentOrientation {
                        a: u.min(v),
                        b: u.max(v),
                    });
                }
                let id = *edge_lookup.entry(key(u, v)).or_insert_with(|| {
                    edges.push(Edge {
                        v: key(u, v),
                        faces: [t as u32, u32::MAX],
                        face_count: 0,
                    });
                    (edges.len() - 1) as EdgeId
                });
                let e = &mut edges[id as usize];
                if e.face_count == 2 {
                    return Err(Error::NonManifoldEdge {
                        a: e.v[0],
                        b: e.v[1],
                    });
                }
                e.faces[e.face_count as usize] = t as u32;
                e.face_count += 1;
                fe[k] = id;
            }
            face_edges.push(fe);
        }

        Ok(Self {
            positions,
            triangles,
            edges,
            edge_lookup,
            face_edges,
        })
    }

    /// Vertex positions (millimetres, by convention).
    pub fn positions(&self) -> &[DVec3] {
        &self.positions
    }

    /// Counter-clockwise triangles (outward normal by the right-hand rule).
    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }

    /// All undirected edges.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Edge ids of triangle `f`, `face_edges(f)[k]` joining corners `k`, `k+1`.
    pub fn face_edges(&self, f: u32) -> [EdgeId; 3] {
        self.face_edges[f as usize]
    }

    /// Look up the edge between two vertices.
    pub fn find_edge(&self, a: u32, b: u32) -> Option<EdgeId> {
        self.edge_lookup.get(&key(a, b)).copied()
    }

    /// Apply a uniform scale to all positions (e.g. cm → mm).
    pub fn scale(&mut self, factor: f64) {
        for p in &mut self.positions {
            *p *= factor;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> (Vec<DVec3>, Vec<[u32; 3]>) {
        let p = vec![
            DVec3::new(0., 0., 0.),
            DVec3::new(1., 0., 0.),
            DVec3::new(1., 1., 0.),
            DVec3::new(0., 1., 0.),
        ];
        (p, vec![[0, 1, 2], [0, 2, 3]])
    }

    #[test]
    fn builds_edges() {
        let (p, t) = quad();
        let m = Mesh::new(p, t).unwrap();
        assert_eq!(m.edges().len(), 5);
        let diag = m.find_edge(2, 0).unwrap();
        assert!(!m.edges()[diag as usize].is_boundary());
        assert_eq!(m.edges().iter().filter(|e| e.is_boundary()).count(), 4);
    }

    #[test]
    fn rejects_flipped_neighbour() {
        let (p, _) = quad();
        let err = Mesh::new(p, vec![[0, 1, 2], [0, 3, 2], [2, 3, 0]]).unwrap_err();
        assert!(matches!(
            err,
            Error::InconsistentOrientation { .. } | Error::NonManifoldEdge { .. }
        ));
        let (p, _) = quad();
        let err = Mesh::new(p, vec![[0, 1, 2], [2, 0, 3]]).unwrap_err();
        assert!(
            matches!(err, Error::InconsistentOrientation { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_degenerate() {
        let (mut p, t) = quad();
        p[2] = p[1];
        assert!(matches!(
            Mesh::new(p, t),
            Err(Error::DegenerateTriangle { .. })
        ));
    }
}
