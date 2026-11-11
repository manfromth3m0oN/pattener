//! A panel: one connected, disc-shaped piece of the garment after cutting.

use crate::mesh::EdgeId;
use crate::seams::SeamId;
use glam::DVec3;

/// One directed edge of a panel's boundary loop.
///
/// Edges are listed in loop order and oriented so that the panel interior
/// lies to the left (counter-clockwise when viewed from outside the body).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BoundaryEdge {
    /// Local start vertex.
    pub from: u32,
    /// Local end vertex.
    pub to: u32,
    /// The original mesh edge.
    pub mesh_edge: EdgeId,
    /// The seam that created this boundary edge, or `None` if it lies on
    /// the open boundary of the input mesh (neckline, hem, …).
    pub seam: Option<SeamId>,
}

/// A topological disc cut from the body mesh, with its own local indexing.
///
/// Vertices on a seam are duplicated: each panel that touches a seam gets
/// its own copy. `vertex_origin` and `face_origin` map back to the input
/// mesh so later stages (seam-length matching, notches) can pair edges
/// across panels.
#[derive(Debug, Clone)]
pub struct Panel {
    /// 3D positions of local vertices.
    pub positions: Vec<DVec3>,
    /// Counter-clockwise triangles in local indices.
    pub triangles: Vec<[u32; 3]>,
    /// Input-mesh vertex for each local vertex.
    pub vertex_origin: Vec<u32>,
    /// Input-mesh triangle for each local triangle.
    pub face_origin: Vec<u32>,
    /// The single boundary loop, in order.
    pub boundary: Vec<BoundaryEdge>,
}

impl Panel {
    /// Number of local vertices.
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Total 3D surface area.
    pub fn area(&self) -> f64 {
        self.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.positions[i as usize]);
                0.5 * (b - a).cross(c - a).length()
            })
            .sum()
    }

    /// Local vertex indices of the boundary loop, in order.
    pub fn boundary_vertices(&self) -> Vec<u32> {
        self.boundary.iter().map(|e| e.from).collect()
    }
}
