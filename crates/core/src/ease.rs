//! Ease factor: how much smaller the flat piece is than the body.
//!
//! A reference triangle is the 3D triangle laid flat and scaled by `s`, so
//! each target flat edge length is `s ×` its 3D length. Pulling the flat
//! piece onto the body then stretches it by roughly `1/s`.
//!
//! `s` is defined per input-mesh triangle so that later milestones (swatch
//! calibration per sheet thickness, local reinforcement, graded ease) only
//! need to produce a vector of values; nothing downstream changes.

use crate::cut::cut_panels;
use crate::error::{Error, Result};
use crate::mesh::Mesh;
use crate::panel::Panel;
use crate::seams::SeamSet;

/// Ease specification for a whole garment.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Ease {
    /// One value everywhere.
    Uniform(f64),
    /// One value per input-mesh triangle.
    PerTriangle(Vec<f64>),
}

impl Default for Ease {
    fn default() -> Self {
        Ease::Uniform(0.85)
    }
}

impl Ease {
    /// One ease value per panel (in [`cut_panels`] order), e.g. when panels
    /// are made from different sheets. Cuts the mesh to find which input
    /// triangles belong to which panel and expands to
    /// [`Ease::PerTriangle`].
    pub fn per_panel(mesh: &Mesh, seams: &SeamSet, values: &[f64]) -> Result<Ease> {
        let panels = cut_panels(mesh, &seams.resolve(mesh)?)?;
        if panels.len() != values.len() {
            return Err(Error::InvalidEase(format!(
                "{} panel value(s) given, but the seams make {} panel(s)",
                values.len(),
                panels.len()
            )));
        }
        let mut per = vec![f64::NAN; mesh.triangles().len()];
        for (p, &v) in panels.iter().zip(values) {
            for &f in &p.face_origin {
                per[f as usize] = v;
            }
        }
        Ok(Ease::PerTriangle(per))
    }

    /// Per-local-triangle ease values for `panel`.
    pub fn for_panel(&self, panel: &Panel) -> Result<Vec<f64>> {
        let values: Vec<f64> = match self {
            Ease::Uniform(s) => vec![*s; panel.triangles.len()],
            Ease::PerTriangle(v) => panel
                .face_origin
                .iter()
                .map(|&f| {
                    v.get(f as usize).copied().ok_or_else(|| {
                        Error::InvalidEase(format!(
                            "no value for mesh triangle {f} ({} given)",
                            v.len()
                        ))
                    })
                })
                .collect::<Result<_>>()?,
        };
        if let Some(bad) = values.iter().find(|s| !(s.is_finite() && **s > 0.0)) {
            return Err(Error::InvalidEase(format!(
                "ease must be positive and finite, got {bad}"
            )));
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{flatten_garment, testmesh, FlattenOptions, Seam};
    use approx::assert_relative_eq;

    #[test]
    fn per_panel_ease_scales_each_panel() {
        let m = testmesh::grid(10, 8, 20.0);
        let seams = SeamSet {
            seams: vec![Seam {
                vertices: (0..=8).map(|j| j * 11 + 5).collect(),
                ..Default::default()
            }],
        };
        let ease = Ease::per_panel(&m, &seams, &[0.8, 0.9]).unwrap();
        let opts = FlattenOptions {
            ease,
            seam_matching: None,
            ..Default::default()
        };
        let g = flatten_garment(&m, &seams, &opts).unwrap();
        for (fp, s) in g.panels.iter().zip([0.8, 0.9]) {
            assert_relative_eq!(fp.strain.max_stretch, 1.0 / s, epsilon = 1e-9);
        }
        assert!(matches!(
            Ease::per_panel(&m, &seams, &[0.8]),
            Err(Error::InvalidEase(_))
        ));
    }
}
