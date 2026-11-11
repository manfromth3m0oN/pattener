//! End-to-end flattening: cut → LSCM → ARAP → strain.

use crate::arap::{arap, ArapOptions};
use crate::cut::cut_panels;
use crate::ease::Ease;
use crate::error::Result;
use crate::lscm::lscm;
use crate::mesh::Mesh;
use crate::panel::Panel;
use crate::seams::SeamSet;
use crate::strain::{measure, StrainReport};
use glam::DVec2;

/// Options for [`flatten_garment`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FlattenOptions {
    pub ease: Ease,
    pub arap: ArapOptions,
}

/// A panel together with its flat layout and strain report.
#[derive(Debug, Clone)]
pub struct FlatPanel {
    pub panel: Panel,
    /// Flat position (mm) per local vertex. Seen from outside the body
    /// (normal towards the viewer), counter-clockwise, `y` up.
    pub uv: Vec<DVec2>,
    pub strain: StrainReport,
    pub arap_iterations: usize,
}

/// Flatten one panel with per-local-triangle ease values.
pub fn flatten_panel(panel: Panel, ease: &[f64], opts: &ArapOptions) -> Result<FlatPanel> {
    let init = lscm(&panel)?;
    let r = arap(&panel, ease, &init, opts)?;
    let strain = measure(&panel, &r.uv);
    Ok(FlatPanel { panel, uv: r.uv, strain, arap_iterations: r.iterations })
}

/// Cut `mesh` along `seams` and flatten every panel.
pub fn flatten_garment(mesh: &Mesh, seams: &SeamSet, opts: &FlattenOptions) -> Result<Vec<FlatPanel>> {
    let panels = cut_panels(mesh, &seams.resolve(mesh)?)?;
    panels
        .into_iter()
        .map(|p| {
            let ease = opts.ease.for_panel(&p)?;
            flatten_panel(p, &ease, &opts.arap)
        })
        .collect()
}
