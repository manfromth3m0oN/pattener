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
use glam::{DVec2, DVec3};

/// Options for [`flatten_garment`].
#[derive(Debug, Clone, PartialEq)]
pub struct FlattenOptions {
    pub ease: Ease,
    pub arap: ArapOptions,
    /// Body "up" direction; flat pieces are turned so it points up the
    /// page. OBJ files are conventionally Y-up.
    pub up: DVec3,
}

impl Default for FlattenOptions {
    fn default() -> Self {
        Self { ease: Ease::default(), arap: ArapOptions::default(), up: DVec3::Y }
    }
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

/// Flatten one panel with per-local-triangle ease values, oriented so that
/// `up` points up the page.
pub fn flatten_panel(panel: Panel, ease: &[f64], opts: &ArapOptions, up: DVec3) -> Result<FlatPanel> {
    let init = lscm(&panel)?;
    let mut r = arap(&panel, ease, &init, opts)?;
    orient(&panel, &mut r.uv, up);
    let strain = measure(&panel, &r.uv);
    Ok(FlatPanel { panel, uv: r.uv, strain, arap_iterations: r.iterations })
}

/// Rigidly place a flat layout (rotation + translation only, so strain is
/// unchanged):
/// 1. rotate so the minimum-area bounding rectangle is axis-aligned (one of
///    its sides is collinear with a convex-hull edge, so every boundary
///    edge direction is tried);
/// 2. of the four 90° turns of that, keep the one maximising the
///    covariance `Σ (y_i − ȳ)(h_i − h̄)` between flat `y` and body height
///    `h_i = p_i · up`;
/// 3. translate the bounding box's lower-left corner to the origin.
pub fn orient(panel: &Panel, uv: &mut [DVec2], up: DVec3) {
    let rotate = |p: DVec2, a: DVec2| DVec2::new(a.x * p.x - a.y * p.y, a.y * p.x + a.x * p.y);
    let area_at = |a: DVec2| {
        let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
        for e in &panel.boundary {
            let q = rotate(uv[e.from as usize], a);
            lo = lo.min(q);
            hi = hi.max(q);
        }
        (hi - lo).x * (hi - lo).y
    };
    let mut best = (DVec2::X, f64::INFINITY);
    for e in &panel.boundary {
        let d = uv[e.to as usize] - uv[e.from as usize];
        if d.length_squared() == 0.0 {
            continue;
        }
        // Rotation taking direction d to +x: angle −atan2(d).
        let n = d.normalize();
        let a = DVec2::new(n.x, -n.y);
        let area = area_at(a);
        if area < best.1 * (1.0 - 1e-9) {
            best = (a, area);
        }
    }
    let base = best.0;

    let heights: Vec<f64> = panel.positions.iter().map(|p| p.dot(up)).collect();
    let hm = heights.iter().sum::<f64>() / heights.len() as f64;
    let quarter = [DVec2::X, DVec2::Y, DVec2::NEG_X, DVec2::NEG_Y];
    let score = |a: DVec2| {
        let ys: Vec<f64> = uv.iter().map(|&p| rotate(p, a).y).collect();
        let ym = ys.iter().sum::<f64>() / ys.len() as f64;
        ys.iter().zip(&heights).map(|(y, h)| (y - ym) * (h - hm)).sum::<f64>()
    };
    let a = quarter
        .iter()
        .map(|&q| rotate(base, q))
        .max_by(|x, y| score(*x).total_cmp(&score(*y)))
        .unwrap();

    for p in uv.iter_mut() {
        *p = rotate(*p, a);
    }
    let lo = uv.iter().fold(DVec2::splat(f64::INFINITY), |l, p| l.min(*p));
    for p in uv.iter_mut() {
        *p -= lo;
    }
}

/// Cut `mesh` along `seams` and flatten every panel.
pub fn flatten_garment(mesh: &Mesh, seams: &SeamSet, opts: &FlattenOptions) -> Result<Vec<FlatPanel>> {
    let panels = cut_panels(mesh, &seams.resolve(mesh)?)?;
    panels
        .into_iter()
        .map(|p| {
            let ease = opts.ease.for_panel(&p)?;
            flatten_panel(p, &ease, &opts.arap, opts.up)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testmesh;

    #[test]
    fn pieces_are_axis_aligned_and_upright() {
        // Tall grid lying in the XY plane, rotated 30° about Z so it starts tilted.
        let g = testmesh::grid(2, 6, 10.0);
        let rot = glam::DQuat::from_rotation_z(0.5);
        let m = Mesh::new(g.positions().iter().map(|&p| rot * p).collect(), g.triangles().to_vec()).unwrap();
        let fp = &flatten_garment(&m, &SeamSet::default(), &FlattenOptions::default()).unwrap()[0];
        let hi = fp.uv.iter().fold(DVec2::ZERO, |h, p| h.max(*p));
        assert!((hi.x - 0.85 * 20.0).abs() < 1e-6 && (hi.y - 0.85 * 60.0).abs() < 1e-6, "{hi}");
        // The vertex highest on the body is at the top of the page.
        let top = (0..m.positions().len()).max_by(|&a, &b| m.positions()[a].y.total_cmp(&m.positions()[b].y)).unwrap();
        let local = fp.panel.vertex_origin.iter().position(|&o| o as usize == top).unwrap();
        assert!(fp.uv[local].y > 0.5 * hi.y);
    }
}
