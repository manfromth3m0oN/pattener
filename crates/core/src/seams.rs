//! Seam definitions.
//!
//! A seam is a polyline of **vertex indices** (0-based, OBJ file order).
//! Each consecutive pair must be joined by a mesh edge. To close a loop
//! (e.g. around an armhole) repeat the first vertex at the end.
//!
//! JSON form (with the `serde` feature):
//! ```json
//! { "seams": [ { "name": "side_left", "vertices": [12, 40, 41, 77] } ] }
//! ```

use crate::error::{Error, Result};
use crate::mesh::{EdgeId, Mesh};

/// Index of a seam in a [`SeamSet`].
pub type SeamId = u32;

/// One seam polyline.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Seam {
    /// Optional human-readable name, carried through to reports.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub name: Option<String>,
    /// Vertex path; consecutive vertices must share a mesh edge.
    pub vertices: Vec<u32>,
}

/// All seams of a garment.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeamSet {
    pub seams: Vec<Seam>,
}

impl SeamSet {
    /// Resolve every seam to mesh edges.
    ///
    /// Returns, for each mesh edge, the seam that cuts it (if any). When two
    /// seams share an edge, the first one listed wins.
    pub fn resolve(&self, mesh: &Mesh) -> Result<Vec<Option<SeamId>>> {
        let mut cut = vec![None; mesh.edges().len()];
        for (s, seam) in self.seams.iter().enumerate() {
            if seam.vertices.len() < 2 {
                return Err(Error::SeamTooShort { seam: s });
            }
            for w in seam.vertices.windows(2) {
                let e: EdgeId = mesh.find_edge(w[0], w[1]).ok_or(Error::SeamNotAnEdge {
                    seam: s,
                    a: w[0],
                    b: w[1],
                })?;
                cut[e as usize].get_or_insert(s as SeamId);
            }
        }
        Ok(cut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn resolves_and_rejects() {
        let m = Mesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::new(1., 1., 0.), DVec3::Y],
            vec![[0, 1, 2], [0, 2, 3]],
        )
        .unwrap();
        let set = SeamSet {
            seams: vec![Seam {
                name: None,
                vertices: vec![0, 2],
            }],
        };
        let cut = set.resolve(&m).unwrap();
        assert_eq!(cut[m.find_edge(0, 2).unwrap() as usize], Some(0));
        assert_eq!(cut.iter().flatten().count(), 1);

        let bad = SeamSet {
            seams: vec![Seam {
                name: None,
                vertices: vec![1, 3],
            }],
        };
        assert_eq!(
            bad.resolve(&m).unwrap_err(),
            Error::SeamNotAnEdge {
                seam: 0,
                a: 1,
                b: 3
            }
        );
    }
}
