//! Example garment designs, built from the tape loops of a fitted body.
//!
//! Designs store surface points of the body model, so one made on the
//! default body works on every body. These are used by the tests and to
//! generate the examples shipped with the web app.

use super::edit::locate;
use super::{Curve, CurveKind, GarmentDesign, SurfacePoint};
use crate::human::MeasureKind;
use crate::mesh::Mesh;
use glam::DVec3;

fn tape(tapes: &[(MeasureKind, Vec<DVec3>)], k: MeasureKind) -> &[DVec3] {
    &tapes.iter().find(|t| t.0 == k).expect("tape").1
}

/// `n` evenly spaced points of a closed loop, as surface points.
fn ring(mesh: &Mesh, loop_: &[DVec3], n: usize) -> Vec<SurfacePoint> {
    (0..n)
        .map(|i| {
            locate(
                mesh.positions(),
                mesh.triangles(),
                loop_[i * loop_.len() / n],
            )
        })
        .collect()
}

fn extreme(loop_: &[DVec3], key: impl Fn(&DVec3) -> f64) -> DVec3 {
    *loop_
        .iter()
        .max_by(|a, b| key(a).total_cmp(&key(b)))
        .expect("loop")
}

/// A short-sleeved top: hem at the hips, neckline, sleeve openings around
/// the upper arms, side seams from hem to sleeve and shoulder seams from
/// neckline to sleeve. `mesh` and `tapes` come from a fitted body.
pub fn top(mesh: &Mesh, tapes: &[(MeasureKind, Vec<DVec3>)]) -> GarmentDesign {
    use MeasureKind::*;
    let at = |p: DVec3| locate(mesh.positions(), mesh.triangles(), p);
    let hips = tape(tapes, Hips);
    let side = |k| extreme(tape(tapes, k), |p| p.x);
    let arm_bottom = extreme(tape(tapes, UpperArm), |p| -p.y);
    let arm_top = extreme(tape(tapes, UpperArm), |p| p.y);
    let neck_side = extreme(tape(tapes, Neck), |p| p.x);
    let curve = |name: &str, kind, points, closed, mirror| Curve {
        name: name.into(),
        kind,
        points,
        closed,
        mirror,
    };
    GarmentDesign {
        curves: vec![
            curve("hem", CurveKind::Edge, ring(mesh, hips, 12), true, false),
            curve(
                "neckline",
                CurveKind::Edge,
                ring(mesh, tape(tapes, Neck), 10),
                true,
                false,
            ),
            curve(
                "sleeve",
                CurveKind::Edge,
                ring(mesh, tape(tapes, UpperArm), 8),
                true,
                true,
            ),
            curve(
                "side",
                CurveKind::Seam,
                vec![
                    at(side(Hips)),
                    at(side(Waist)),
                    at(side(Underbust)),
                    at(side(Bust)),
                    at(arm_bottom),
                ],
                false,
                true,
            ),
            curve(
                "shoulder",
                CurveKind::Seam,
                vec![at(neck_side), at(arm_top)],
                false,
                true,
            ),
        ],
        inside: Some(at(extreme(tape(tapes, Waist), |p| p.z))),
    }
}
