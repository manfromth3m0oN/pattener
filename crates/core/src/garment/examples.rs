//! Example garment designs, built from the tape loops of a fitted body.
//!
//! Designs store surface points of the body model, so one made on the
//! default body works on every body. These are used by the tests and to
//! generate the examples shipped with the web app.

use super::edit::locate;
use super::{Curve, CurveKind, GarmentDesign, SurfacePoint};
use crate::human::MeasureKind;
use crate::mesh::Mesh;
use crate::pattern::EdgeFinish;
use crate::seams::SeamFinish;
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
        ..Default::default()
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
        ..Default::default()
    }
}

/// A fitted body's tape loop.
pub type Tape = (MeasureKind, Vec<DVec3>);

/// A starting design in the template library.
#[derive(Debug, Clone, Copy)]
pub struct Template {
    /// File-name-safe id, e.g. `leggings`.
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Builds the design from a fitted body's mesh and tapes.
    pub build: fn(&Mesh, &[Tape]) -> GarmentDesign,
}

/// The template library.
pub const TEMPLATES: [Template; 4] = [
    Template {
        id: "top",
        name: "Short-sleeved top",
        description: "Front and back joined at the shoulders and sides; trimmed neckline, \
                      folded hem at the hips.",
        build: top_finished,
    },
    Template {
        id: "leggings",
        name: "Leggings",
        description: "One piece per leg, joined at centre front and back, with an inside-leg \
                      seam; folded waist hem.",
        build: leggings,
    },
    Template {
        id: "catsuit",
        name: "Catsuit",
        description: "Neck to wrists and ankles in four pieces, with a back zip from neck to \
                      crotch and seams over and under the arms, down the sides and the inside \
                      legs.",
        build: catsuit,
    },
    Template {
        id: "cincher",
        name: "Waist cincher",
        description: "Underbust to hips in four pieces with a back zip; both edges trimmed.",
        build: cincher,
    },
];

/// Shared landmarks of a fitted body (the model is Y-up, +Z to the front;
/// limb tapes are on the +X side, so limb curves are mirrored).
struct Landmarks<'a> {
    mesh: &'a Mesh,
    tapes: &'a [(MeasureKind, Vec<DVec3>)],
}

impl Landmarks<'_> {
    fn at(&self, p: DVec3) -> SurfacePoint {
        locate(self.mesh.positions(), self.mesh.triangles(), p)
    }
    fn tape(&self, k: MeasureKind) -> &[DVec3] {
        tape(self.tapes, k)
    }
    /// Where a torso loop crosses the middle of the body, front or back.
    fn centre(&self, k: MeasureKind, front: bool) -> DVec3 {
        let l = self.tape(k);
        let mid_z = l.iter().map(|p| p.z).sum::<f64>() / l.len() as f64;
        *l.iter()
            .filter(|p| (p.z > mid_z) == front)
            .min_by(|a, b| a.x.abs().total_cmp(&b.x.abs()))
            .expect("loop")
    }
    /// The +X side of a torso loop.
    fn side(&self, k: MeasureKind) -> DVec3 {
        extreme(self.tape(k), |p| p.x)
    }
    /// The lowest point on the middle of the body, moved exactly onto the
    /// centre line so that it and its mirror image coincide.
    fn crotch(&self) -> DVec3 {
        let p = *self
            .mesh
            .positions()
            .iter()
            .filter(|p| p.x.abs() < 10.0 && p.y < self.tape(MeasureKind::Hips)[0].y)
            .min_by(|a, b| a.y.total_cmp(&b.y))
            .expect("body has a crotch");
        DVec3::new(0.0, p.y, p.z)
    }
    /// A guide point a little way down the inside of the +X thigh, so an
    /// inside-leg seam leaves the crotch sideways instead of running
    /// alongside (and recrossing) the centre seams.
    fn off_crotch(&self) -> DVec3 {
        self.crotch()
            .lerp(self.leg(MeasureKind::Thigh, false), 0.25)
    }
    /// Outer (`true`) or inner side of a leg loop.
    fn leg(&self, k: MeasureKind, outer: bool) -> DVec3 {
        extreme(self.tape(k), |p| if outer { p.x } else { -p.x })
    }
    /// Top (`true`) or underside of an arm loop: furthest along "up"
    /// taken square to the arm.
    fn arm(&self, k: MeasureKind, top: bool) -> DVec3 {
        let c = |k| {
            let l = self.tape(k);
            l.iter().copied().sum::<DVec3>() / l.len() as f64
        };
        let d = (c(MeasureKind::Wrist) - c(MeasureKind::UpperArm)).normalize();
        let up = (DVec3::Y - DVec3::Y.dot(d) * d).normalize();
        extreme(self.tape(k), |p| if top { p.dot(up) } else { -p.dot(up) })
    }
    fn ring(&self, k: MeasureKind, n: usize) -> Vec<SurfacePoint> {
        ring(self.mesh, self.tape(k), n)
    }
    fn points(&self, ps: &[DVec3]) -> Vec<SurfacePoint> {
        ps.iter().map(|&p| self.at(p)).collect()
    }
}

fn edge(name: &str, points: Vec<SurfacePoint>, mirror: bool, finish: Option<EdgeFinish>) -> Curve {
    Curve {
        name: name.into(),
        kind: CurveKind::Edge,
        points,
        closed: true,
        mirror,
        edge_finish: finish,
        ..Default::default()
    }
}

fn seam(name: &str, points: Vec<SurfacePoint>, mirror: bool, finish: SeamFinish) -> Curve {
    Curve {
        name: name.into(),
        kind: CurveKind::Seam,
        points,
        closed: false,
        mirror,
        seam_finish: finish,
        ..Default::default()
    }
}

const HEM: EdgeFinish = EdgeFinish::Hem { width_mm: 20.0 };
const TRIM: EdgeFinish = EdgeFinish::Trim {
    width_mm: 15.0,
    ease: 0.9,
};

/// [`top`] with its finishes: a trimmed neckline, a folded hem and raw
/// sleeve openings.
pub fn top_finished(mesh: &Mesh, tapes: &[(MeasureKind, Vec<DVec3>)]) -> GarmentDesign {
    let mut d = top(mesh, tapes);
    for c in &mut d.curves {
        c.edge_finish = match c.name.as_str() {
            "neckline" => Some(TRIM),
            "hem" => Some(HEM),
            "sleeve" => Some(EdgeFinish::Raw),
            _ => None,
        };
    }
    d
}

/// Leggings: waist to ankles.
pub fn leggings(mesh: &Mesh, tapes: &[(MeasureKind, Vec<DVec3>)]) -> GarmentDesign {
    use MeasureKind::*;
    let b = Landmarks { mesh, tapes };
    let crotch = b.crotch();
    GarmentDesign {
        curves: vec![
            edge("waist", b.ring(Waist, 12), false, Some(HEM)),
            edge("ankle", b.ring(Ankle, 8), true, Some(EdgeFinish::Raw)),
            seam(
                "centre front",
                b.points(&[b.centre(Waist, true), b.centre(Hips, true), crotch]),
                false,
                SeamFinish::Glued,
            ),
            seam(
                "centre back",
                b.points(&[b.centre(Waist, false), b.centre(Hips, false), crotch]),
                false,
                SeamFinish::Glued,
            ),
            seam(
                "inside leg",
                b.points(&[
                    crotch,
                    b.off_crotch(),
                    b.leg(Thigh, false),
                    b.leg(Knee, false),
                    b.leg(Calf, false),
                    b.leg(Ankle, false),
                ]),
                true,
                SeamFinish::Glued,
            ),
        ],
        inside: Some(b.at(extreme(b.tape(Thigh), |p| p.z))),
        ..Default::default()
    }
}

/// Catsuit: neck to wrists and ankles.
pub fn catsuit(mesh: &Mesh, tapes: &[(MeasureKind, Vec<DVec3>)]) -> GarmentDesign {
    use MeasureKind::*;
    let b = Landmarks { mesh, tapes };
    let crotch = b.crotch();
    let torso = |front: bool| {
        let mut v = vec![b.centre(Neck, front)];
        v.extend([Bust, Underbust, Waist, Hips].map(|k| b.centre(k, front)));
        v.push(crotch);
        b.points(&v)
    };
    GarmentDesign {
        curves: vec![
            edge("neckline", b.ring(Neck, 10), false, Some(TRIM)),
            edge("wrist", b.ring(Wrist, 8), true, Some(EdgeFinish::Raw)),
            edge("ankle", b.ring(Ankle, 8), true, Some(EdgeFinish::Raw)),
            seam("centre front", torso(true), false, SeamFinish::Glued),
            seam("centre back", torso(false), false, SeamFinish::Zip),
            seam(
                "over arm",
                b.points(&[b.side(Neck), b.arm(UpperArm, true), b.arm(Wrist, true)]),
                true,
                SeamFinish::Glued,
            ),
            seam(
                "side",
                b.points(&[
                    b.arm(Wrist, false),
                    b.arm(UpperArm, false),
                    b.side(Bust),
                    b.side(Underbust),
                    b.side(Waist),
                    b.side(Hips),
                    b.leg(Thigh, true),
                    b.leg(Knee, true),
                    b.leg(Calf, true),
                    b.leg(Ankle, true),
                ]),
                true,
                SeamFinish::Glued,
            ),
            seam(
                "inside leg",
                b.points(&[
                    crotch,
                    b.off_crotch(),
                    b.leg(Thigh, false),
                    b.leg(Knee, false),
                    b.leg(Calf, false),
                    b.leg(Ankle, false),
                ]),
                true,
                SeamFinish::Glued,
            ),
        ],
        inside: Some(b.at(extreme(b.tape(Thigh), |p| p.z))),
        ..Default::default()
    }
}

/// Waist cincher: underbust to hips.
pub fn cincher(mesh: &Mesh, tapes: &[(MeasureKind, Vec<DVec3>)]) -> GarmentDesign {
    use MeasureKind::*;
    let b = Landmarks { mesh, tapes };
    let line = |front: bool| b.points(&[Underbust, Waist, Hips].map(|k| b.centre(k, front)));
    GarmentDesign {
        curves: vec![
            edge("top", b.ring(Underbust, 12), false, Some(TRIM)),
            edge("bottom", b.ring(Hips, 12), false, Some(TRIM)),
            seam("centre front", line(true), false, SeamFinish::Glued),
            seam("centre back", line(false), false, SeamFinish::Zip),
            seam(
                "side",
                b.points(&[Underbust, Waist, Hips].map(|k| b.side(k))),
                true,
                SeamFinish::Glued,
            ),
        ],
        // Front, between the centre and the side.
        inside: Some(b.at(extreme(b.tape(Waist), |p| p.z + 0.5 * p.x))),
        ..Default::default()
    }
}
