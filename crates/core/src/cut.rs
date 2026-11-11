//! Cutting the body mesh along seams into panels.
//!
//! ## Algorithm
//! 1. An edge is *cut* if a seam runs along it or it is on the mesh boundary.
//! 2. **Panels.** Union-find over triangles, joining the two triangles of
//!    every uncut edge. Each class is one panel.
//! 3. **Vertex splitting.** Union-find over triangle *corners* (`3f + k`).
//!    Across every uncut edge, the two triangles' corners at each shared
//!    endpoint are joined. Each class is then one fan of triangles around
//!    a vertex that are connected without crossing a seam, and becomes
//!    one local panel vertex. This correctly separates the two sides of a
//!    seam, including slits that end inside a panel.
//! 4. **Disc check.** With `F` triangles and `B` boundary half-edges, the
//!    panel has `E = (3F + B) / 2` edges and Euler characteristic
//!    `χ = V − E + F`. A connected panel is a topological disc iff `χ = 1`
//!    and it has exactly one boundary loop.

use crate::error::{Error, Result};
use crate::mesh::Mesh;
use crate::panel::{BoundaryEdge, Panel};
use crate::seams::SeamId;
use std::collections::HashMap;

struct Dsu(Vec<u32>);

impl Dsu {
    fn new(n: usize) -> Self {
        Self((0..n as u32).collect())
    }
    fn find(&mut self, mut x: u32) -> u32 {
        while self.0[x as usize] != x {
            let p = self.0[self.0[x as usize] as usize];
            self.0[x as usize] = p;
            x = p;
        }
        x
    }
    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            let (lo, hi) = (ra.min(rb), ra.max(rb));
            self.0[hi as usize] = lo;
        }
    }
}

fn corner_of(mesh: &Mesh, f: u32, v: u32) -> u32 {
    let k = mesh.triangles()[f as usize]
        .iter()
        .position(|&x| x == v)
        .expect("vertex in face");
    3 * f + k as u32
}

/// Cut `mesh` along the seams given by `edge_seams` (from
/// [`SeamSet::resolve`](crate::SeamSet::resolve)) and return the panels,
/// ordered by their lowest input triangle index.
///
/// Fails with [`Error::NotADisc`] if any panel cannot be laid flat.
pub fn cut_panels(mesh: &Mesh, edge_seams: &[Option<SeamId>]) -> Result<Vec<Panel>> {
    assert_eq!(
        edge_seams.len(),
        mesh.edges().len(),
        "edge_seams must have one entry per edge"
    );
    let nf = mesh.triangles().len();
    let is_cut = |e: usize| edge_seams[e].is_some() || mesh.edges()[e].is_boundary();

    let mut faces = Dsu::new(nf);
    let mut corners = Dsu::new(3 * nf);
    for (e, edge) in mesh.edges().iter().enumerate() {
        if is_cut(e) {
            continue;
        }
        let [f, g] = edge.faces;
        faces.union(f, g);
        for v in edge.v {
            corners.union(corner_of(mesh, f, v), corner_of(mesh, g, v));
        }
    }

    // Group faces into panels, in order of first appearance.
    let mut panel_of_root: HashMap<u32, usize> = HashMap::new();
    let mut panel_faces: Vec<Vec<u32>> = Vec::new();
    for f in 0..nf as u32 {
        let r = faces.find(f);
        let p = *panel_of_root.entry(r).or_insert_with(|| {
            panel_faces.push(Vec::new());
            panel_faces.len() - 1
        });
        panel_faces[p].push(f);
    }

    panel_faces
        .iter()
        .enumerate()
        .map(|(pi, fs)| build_panel(mesh, edge_seams, &mut corners, pi, fs, &is_cut))
        .collect()
}

fn build_panel(
    mesh: &Mesh,
    edge_seams: &[Option<SeamId>],
    corners: &mut Dsu,
    panel_index: usize,
    faces: &[u32],
    is_cut: &dyn Fn(usize) -> bool,
) -> Result<Panel> {
    let mut local_of_root: HashMap<u32, u32> = HashMap::new();
    let mut vertex_origin = Vec::new();
    let mut triangles = Vec::with_capacity(faces.len());
    for &f in faces {
        let tri = mesh.triangles()[f as usize];
        let mut lt = [0u32; 3];
        for k in 0..3 {
            let root = corners.find(3 * f + k as u32);
            lt[k] = *local_of_root.entry(root).or_insert_with(|| {
                vertex_origin.push(tri[k]);
                (vertex_origin.len() - 1) as u32
            });
        }
        triangles.push(lt);
    }

    // Boundary half-edges: from -> (to, mesh edge).
    let mut next: HashMap<u32, (u32, u32)> = HashMap::new();
    for (lf, &f) in faces.iter().enumerate() {
        let fe = mesh.face_edges(f);
        for k in 0..3 {
            if is_cut(fe[k] as usize) {
                let (a, b) = (triangles[lf][k], triangles[lf][(k + 1) % 3]);
                if next.insert(a, (b, fe[k])).is_some() {
                    return Err(Error::NonManifoldVertex {
                        panel: panel_index,
                        vertex: vertex_origin[a as usize],
                    });
                }
            }
        }
    }

    let v = vertex_origin.len() as i64;
    let f = faces.len() as i64;
    let b = next.len() as i64;
    let e = (3 * f + b) / 2;
    let chi = v - e + f;

    // Walk the boundary loops; start from the smallest vertex for determinism.
    let mut starts: Vec<u32> = next.keys().copied().collect();
    starts.sort_unstable();
    let mut visited: HashMap<u32, ()> = HashMap::new();
    let mut loops: Vec<Vec<BoundaryEdge>> = Vec::new();
    for s in starts {
        if visited.contains_key(&s) {
            continue;
        }
        let mut lp = Vec::new();
        let mut cur = s;
        loop {
            visited.insert(cur, ());
            let Some(&(to, me)) = next.get(&cur) else {
                // A boundary chain that does not close: treat as malformed.
                return Err(Error::NonManifoldVertex {
                    panel: panel_index,
                    vertex: vertex_origin[cur as usize],
                });
            };
            lp.push(BoundaryEdge {
                from: cur,
                to,
                mesh_edge: me,
                seam: edge_seams[me as usize],
            });
            cur = to;
            if cur == s {
                break;
            }
            if visited.contains_key(&cur) {
                return Err(Error::NonManifoldVertex {
                    panel: panel_index,
                    vertex: vertex_origin[cur as usize],
                });
            }
        }
        loops.push(lp);
    }

    if chi != 1 || loops.len() != 1 {
        return Err(Error::NotADisc {
            panel: panel_index,
            euler_characteristic: chi,
            boundary_loops: loops.len(),
        });
    }

    let positions = vertex_origin
        .iter()
        .map(|&o| mesh.positions()[o as usize])
        .collect();
    Ok(Panel {
        positions,
        triangles,
        vertex_origin,
        face_origin: faces.to_vec(),
        boundary: loops.pop().unwrap(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testmesh;
    use crate::{Seam, SeamSet};

    fn cut(mesh: &Mesh, seams: Vec<Vec<u32>>) -> Result<Vec<Panel>> {
        let set = SeamSet {
            seams: seams
                .into_iter()
                .map(|v| Seam {
                    name: None,
                    vertices: v,
                })
                .collect(),
        };
        cut_panels(mesh, &set.resolve(mesh)?)
    }

    #[test]
    fn grid_without_seams_is_one_disc() {
        let m = testmesh::grid(4, 3, 10.0);
        let p = cut(&m, vec![]).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].vertex_count(), 20);
        assert_eq!(p[0].boundary.len(), 14);
    }

    #[test]
    fn seam_across_grid_splits_in_two() {
        // 4x2 cells: vertices (i, j) -> j*5 + i. Cut along column i = 2.
        let m = testmesh::grid(4, 2, 1.0);
        let p = cut(&m, vec![vec![2, 7, 12]]).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p.iter().map(|p| p.vertex_count()).sum::<usize>(), 15 + 3);
        let seam_edges: usize = p
            .iter()
            .map(|p| p.boundary.iter().filter(|e| e.seam == Some(0)).count())
            .sum();
        assert_eq!(seam_edges, 4);
    }

    #[test]
    fn slit_keeps_one_panel_with_duplicated_vertices() {
        // Slit from the boundary into the middle of a 4x4 grid.
        let m = testmesh::grid(4, 4, 1.0);
        let p = cut(&m, vec![vec![2, 7, 12]]).unwrap();
        assert_eq!(p.len(), 1);
        // Vertices 2 and 7 are duplicated, the slit tip 12 is not.
        assert_eq!(p[0].vertex_count(), 25 + 2);
    }

    #[test]
    fn uncut_cylinder_is_not_a_disc() {
        let m = testmesh::cylinder(50.0, 100.0, 12, 4, std::f64::consts::TAU);
        let err = cut(&m, vec![]).unwrap_err();
        assert_eq!(
            err,
            Error::NotADisc {
                panel: 0,
                euler_characteristic: 0,
                boundary_loops: 2
            }
        );
    }

    #[test]
    fn cylinder_with_one_seam_is_a_disc() {
        let m = testmesh::cylinder(50.0, 100.0, 12, 4, std::f64::consts::TAU);
        let seam: Vec<u32> = (0..=4).map(|j| j * 12).collect();
        let p = cut(&m, vec![seam]).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].vertex_count(), 12 * 5 + 5);
    }
}
