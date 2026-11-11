//! Ease factor: how much smaller the flat piece is than the body.
//!
//! A reference triangle is the 3D triangle laid flat and scaled by `s`, so
//! each target flat edge length is `s ×` its 3D length. Pulling the flat
//! piece onto the body then stretches it by roughly `1/s`.
//!
//! `s` is defined per input-mesh triangle so that later milestones (swatch
//! calibration per sheet thickness, local reinforcement, graded ease) only
//! need to produce a vector of values; nothing downstream changes.

use crate::error::{Error, Result};
use crate::panel::Panel;

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
