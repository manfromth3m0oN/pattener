//! Pairing the two sides of each seam and comparing their flat lengths.
//!
//! When two latex panels are glued, the two edges of a seam are laid
//! together flat. If one is longer the excess has to be eased in, which in
//! latex shows up as puckers or ripples along the seam. Because every panel
//! is flattened independently, the two sides of a seam generally come out
//! with slightly different lengths; this module finds the pairs and
//! measures the mismatch, and [`seam_edge_pairs`] lists the per-edge units
//! that [`flatten_garment`](crate::flatten_garment) equalises.
//!
//! ## Pairing
//! Each interior seam edge `(a, b)` of the input mesh becomes two directed
//! boundary edges after cutting: `a → b` in the panel on its left and
//! `b → a` in the panel on its right (panel interiors lie to the left of
//! their boundary). Walking a seam's polyline in order, consecutive edges
//! with the same `(left panel, right panel)` form one [`SeamPair`]; a new
//! pair starts wherever either panel changes, e.g. where the seam crosses
//! another seam. Seam edges on the open boundary of the mesh have only one
//! side and are skipped.

use crate::flatten::FlatPanel;
use crate::mesh::Mesh;
use crate::seams::{SeamId, SeamSet};
use std::collections::HashMap;

/// One side of a seam segment.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeamSide {
    /// Panel index.
    pub panel: usize,
    /// Indices into that panel's `boundary`, in seam order. `edges[k]` on
    /// the left side and `edges[k]` on the right side are the same mesh edge.
    pub edges: Vec<usize>,
}

/// The two sides of one seam segment.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeamPair {
    pub seam: SeamId,
    /// Side whose boundary runs in the seam's vertex order.
    pub left: SeamSide,
    /// Side whose boundary runs against the seam's vertex order.
    pub right: SeamSide,
}

/// A panel boundary edge: `(panel index, index into its boundary)`.
pub type EdgeRef = (usize, usize);

/// Lengths of a seam pair in a flattened garment.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeamLengths {
    /// Length on the body (mm).
    pub body: f64,
    /// Flat length of the left side (mm).
    pub left: f64,
    /// Flat length of the right side (mm).
    pub right: f64,
}

impl SeamLengths {
    /// `left − right` (mm).
    pub fn mismatch(&self) -> f64 {
        self.left - self.right
    }

    /// `|left − right| / ((left + right) / 2)`.
    pub fn relative_mismatch(&self) -> f64 {
        self.mismatch().abs() / (0.5 * (self.left + self.right))
    }
}

/// Find all seam pairs. `panels` must come from cutting `mesh` along `seams`.
pub fn pair_seams(mesh: &Mesh, seams: &SeamSet, panels: &[FlatPanel]) -> Vec<SeamPair> {
    // mesh edge -> [(panel, boundary index, origin from, origin to)]
    let mut sides: HashMap<u32, Vec<(usize, usize, u32, u32)>> = HashMap::new();
    for (pi, fp) in panels.iter().enumerate() {
        let origin = &fp.panel.vertex_origin;
        for (bi, e) in fp.panel.boundary.iter().enumerate() {
            if e.seam.is_some() {
                sides.entry(e.mesh_edge).or_default().push((
                    pi,
                    bi,
                    origin[e.from as usize],
                    origin[e.to as usize],
                ));
            }
        }
    }

    let mut pairs: Vec<SeamPair> = Vec::new();
    for (s, seam) in seams.seams.iter().enumerate() {
        let s = s as SeamId;
        let mut used = HashMap::new();
        let mut open = false; // whether the last pair can be extended
        for w in seam.vertices.windows(2) {
            let (a, b) = (w[0], w[1]);
            let found = mesh.find_edge(a, b).and_then(|e| {
                let list = sides.get(&e)?;
                let left = list.iter().find(|x| (x.2, x.3) == (a, b))?;
                let right = list.iter().find(|x| (x.2, x.3) == (b, a))?;
                let owned = panels[left.0].panel.boundary[left.1].seam == Some(s);
                (owned && list.len() == 2 && used.insert(e, ()).is_none())
                    .then_some((*left, *right))
            });
            let Some((l, r)) = found else {
                open = false;
                continue;
            };
            match pairs.last_mut() {
                Some(p) if open && p.left.panel == l.0 && p.right.panel == r.0 => {
                    p.left.edges.push(l.1);
                    p.right.edges.push(r.1);
                }
                _ => pairs.push(SeamPair {
                    seam: s,
                    left: SeamSide {
                        panel: l.0,
                        edges: vec![l.1],
                    },
                    right: SeamSide {
                        panel: r.0,
                        edges: vec![r.1],
                    },
                }),
            }
            open = true;
        }
    }
    pairs
}

/// Flat length of boundary edge `bi` of a panel.
pub fn flat_edge_length(fp: &FlatPanel, bi: usize) -> f64 {
    let e = fp.panel.boundary[bi];
    fp.uv[e.from as usize].distance(fp.uv[e.to as usize])
}

/// Body length of boundary edge `bi` of a panel.
pub fn body_edge_length(fp: &FlatPanel, bi: usize) -> f64 {
    let e = fp.panel.boundary[bi];
    fp.panel.positions[e.from as usize].distance(fp.panel.positions[e.to as usize])
}

/// Measure a seam pair.
pub fn seam_lengths(pair: &SeamPair, panels: &[FlatPanel]) -> SeamLengths {
    let (lp, rp) = (&panels[pair.left.panel], &panels[pair.right.panel]);
    SeamLengths {
        body: pair
            .left
            .edges
            .iter()
            .map(|&b| body_edge_length(lp, b))
            .sum(),
        left: pair
            .left
            .edges
            .iter()
            .map(|&b| flat_edge_length(lp, b))
            .sum(),
        right: pair
            .right
            .edges
            .iter()
            .map(|&b| flat_edge_length(rp, b))
            .sum(),
    }
}

/// Both sides of every paired seam edge, as `(pair index, (panel,
/// boundary index), (panel, boundary index))`. These are the units that seam matching
/// equalises: matching edge by edge (not just total length) keeps
/// positions along the seam aligned, so notches placed later line up.
pub fn seam_edge_pairs(pairs: &[SeamPair]) -> Vec<(usize, EdgeRef, EdgeRef)> {
    pairs
        .iter()
        .enumerate()
        .flat_map(|(pi, p)| {
            p.left
                .edges
                .iter()
                .zip(&p.right.edges)
                .map(move |(&lb, &rb)| (pi, (p.left.panel, lb), (p.right.panel, rb)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{flatten_garment, testmesh, FlattenOptions, Garment, Seam};
    use approx::assert_relative_eq;
    use std::f64::consts::{PI, TAU};

    fn run(mesh: &Mesh, seams: Vec<Vec<u32>>, matching: bool) -> Garment {
        let set = SeamSet {
            seams: seams
                .into_iter()
                .map(|v| Seam {
                    vertices: v,
                    ..Default::default()
                })
                .collect(),
        };
        let mut opts = FlattenOptions::default();
        if !matching {
            opts.seam_matching = None;
        }
        flatten_garment(mesh, &set, &opts).unwrap()
    }

    #[test]
    fn grid_centre_seam_pairs_two_panels() {
        // 10 x 8 cells of 20 mm; vertex (i, j) = j * 11 + i.
        let m = testmesh::grid(10, 8, 20.0);
        let g = run(&m, vec![(0..=8).map(|j| j * 11 + 5).collect()], false);
        assert_eq!(g.seams.len(), 1);
        let s = &g.seams[0];
        assert_ne!(s.pair.left.panel, s.pair.right.panel);
        assert_eq!(s.pair.left.edges.len(), 8);
        assert_relative_eq!(s.lengths.body, 160.0, epsilon = 1e-9);
        assert_relative_eq!(s.lengths.left, 0.85 * 160.0, epsilon = 1e-6);
        assert_relative_eq!(s.lengths.mismatch(), 0.0, epsilon = 1e-6);
    }

    #[test]
    fn crossing_seams_split_into_segments() {
        let m = testmesh::grid(10, 8, 20.0);
        let vertical: Vec<u32> = (0..=8).map(|j| j * 11 + 5).collect();
        let horizontal: Vec<u32> = (0..=10).map(|i| 4 * 11 + i).collect();
        let g = run(&m, vec![vertical, horizontal], false);
        assert_eq!(g.panels.len(), 4);
        // Each seam crosses the other once: two segments each.
        assert_eq!(g.seams.iter().filter(|s| s.pair.seam == 0).count(), 2);
        assert_eq!(g.seams.iter().filter(|s| s.pair.seam == 1).count(), 2);
        for s in &g.seams {
            assert_eq!(s.pair.left.edges.len(), s.pair.right.edges.len());
            assert_ne!(s.pair.left.panel, s.pair.right.panel);
        }
    }

    #[test]
    fn seam_on_open_boundary_has_no_pair() {
        let m = testmesh::grid(4, 4, 10.0);
        let g = run(&m, vec![vec![0, 1, 2, 3, 4]], true);
        assert!(g.seams.is_empty());
    }

    #[test]
    fn tube_seam_pairs_panel_with_itself() {
        let (n, up) = (24u32, 6u32);
        let m = testmesh::cylinder(50.0, 100.0, n, up, TAU);
        let g = run(&m, vec![(0..=up).map(|j| j * n).collect()], true);
        assert_eq!(g.seams.len(), 1);
        let s = &g.seams[0];
        assert_eq!((s.pair.left.panel, s.pair.right.panel), (0, 0));
        assert_relative_eq!(s.lengths.left, 85.0, epsilon = 1e-6);
        assert_relative_eq!(s.lengths.mismatch(), 0.0, epsilon = 1e-6);
    }

    /// Spherical cap cut around a ring: a small dome inside, a band outside
    /// (opened by a radial seam). The two sides of the ring seam flatten to
    /// different lengths, which matching must equalise edge by edge.
    #[test]
    fn ring_seam_on_cap_is_equalised() {
        let (rings, segs) = (10u32, 32u32);
        let m = testmesh::spherical_cap(100.0, PI / 3.0, rings, segs);
        let ring = |k: u32, i: u32| 1 + (k - 1) * segs + (i % segs);
        let seams = vec![
            (0..=segs).map(|i| ring(5, i)).collect(),
            (5..=rings).map(|k| ring(k, 0)).collect(),
        ];
        let before = run(&m, seams.clone(), false);
        let after = run(&m, seams, true);

        let ring_before = before.seams.iter().find(|s| s.pair.seam == 0).unwrap();
        assert!(
            ring_before.lengths.mismatch().abs() > 2.0,
            "{:?}",
            ring_before.lengths
        );

        let ring_after = after.seams.iter().find(|s| s.pair.seam == 0).unwrap();
        assert_relative_eq!(
            ring_after.unmatched.mismatch(),
            ring_before.lengths.mismatch(),
            epsilon = 1e-9
        );
        assert!(
            ring_after.lengths.mismatch().abs() < 0.01,
            "{:?}",
            ring_after.lengths
        );
        for (_, (lp, lb), (rp, rb)) in seam_edge_pairs(
            &after
                .seams
                .iter()
                .map(|s| s.pair.clone())
                .collect::<Vec<_>>(),
        ) {
            let d =
                flat_edge_length(&after.panels[lp], lb) - flat_edge_length(&after.panels[rp], rb);
            assert!(d.abs() <= 0.002, "edge mismatch {d}");
        }
        for (a, b) in before.panels.iter().zip(&after.panels) {
            assert!(b.strain.flipped.is_empty());
            assert!((b.strain.max_stretch - a.strain.max_stretch).abs() < 0.01);
            assert!((b.strain.min_stretch - a.strain.min_stretch).abs() < 0.01);
        }
    }
}
