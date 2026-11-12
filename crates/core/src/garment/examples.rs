//! Example garment designs, built from the tape loops of a fitted body.
//!
//! Designs store surface points of the body model, so one made on the
//! default body works on every body. These are used by the tests and to
//! generate the examples shipped with the web app.

use super::edit::locate;
use super::{Curve, CurveKind, GarmentDesign, SurfacePoint};
use crate::human::slice::{nearest_section, Plane};
use crate::human::{HumanBody, MeasureKind};
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

/// `n` evenly spaced points of a closed loop starting at the first anchor,
/// with the sample nearest each other anchor moved onto it.
fn ring_through(mesh: &Mesh, loop_: &[DVec3], n: usize, anchors: &[DVec3]) -> Vec<SurfacePoint> {
    let m = loop_.len();
    let index_of = |p: DVec3| {
        (0..m)
            .min_by(|&a, &b| loop_[a].distance(p).total_cmp(&loop_[b].distance(p)))
            .expect("loop")
    };
    let start = anchors.first().map_or(0, |&p| index_of(p));
    let mut idx: Vec<usize> = (0..n).map(|i| (start + i * m / n) % m).collect();
    for &p in anchors.iter().skip(1) {
        let a = index_of(p);
        let k = (1..n)
            .min_by_key(|&i| {
                let d = (idx[i] as isize - a as isize).rem_euclid(m as isize) as usize;
                d.min(m - d)
            })
            .expect("points");
        idx[k] = a;
    }
    idx.iter()
        .map(|&i| locate(mesh.positions(), mesh.triangles(), loop_[i]))
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
    /// Builds the design from a fitted body.
    pub build: fn(&HumanBody) -> GarmentDesign,
}

/// The template library.
pub const TEMPLATES: [Template; 8] = [
    Template {
        id: "top",
        name: "Short-sleeved top (cut-on sleeves)",
        description: "Front and back with the sleeves cut in one with the body, joined at the \
                      shoulders and sides; trimmed neckline, folded hem at the hips. The \
                      simplest construction; the front has no bust shaping.",
        build: |b| top_finished(&b.mesh, &b.tapes),
    },
    Template {
        id: "top-set-in",
        name: "Top with set-in sleeves",
        description: "Front, back and two tube sleeves set into round armholes, with an \
                      underarm seam on each sleeve; the usual construction for a fitted top.",
        build: top_set_in,
    },
    Template {
        id: "top-raglan",
        name: "Raglan top",
        description: "Diagonal raglan seams from the neckline to the armpits, so each sleeve \
                      includes the top of the shoulder; easy to fit across the shoulders.",
        build: top_raglan,
    },
    Template {
        id: "top-princess",
        name: "Princess-seam top",
        description: "Cut-on sleeves with princess seams from the shoulders over the bust \
                      to the hem: a centre front and two side fronts that shape the bust.",
        build: top_princess,
    },
    Template {
        id: "leggings",
        name: "Leggings",
        description: "One piece per leg, joined at centre front and back, with an inside-leg \
                      seam; folded waist hem.",
        build: |b| leggings(&b.mesh, &b.tapes),
    },
    Template {
        id: "catsuit",
        name: "Catsuit",
        description: "Neck to wrists and ankles in four pieces, with a back zip from neck to \
                      crotch and seams over and under the arms, down the sides and the inside \
                      legs.",
        build: |b| catsuit(&b.mesh, &b.tapes),
    },
    Template {
        id: "catsuit-set-in",
        name: "Catsuit with set-in sleeves and gusset",
        description: "Left and right fronts and backs, two set-in sleeves and a diamond \
                      crotch gusset where the centre and inside-leg seams meet; back zip.",
        build: catsuit_set_in,
    },
    Template {
        id: "cincher",
        name: "Waist cincher",
        description: "Underbust to hips in four pieces with a back zip; both edges trimmed.",
        build: |b| cincher(&b.mesh, &b.tapes),
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
    /// `n` points round tape `k` that include the given points of it
    /// exactly, so seams ending there meet the edge cleanly.
    fn ring_through(&self, k: MeasureKind, n: usize, anchors: &[DVec3]) -> Vec<SurfacePoint> {
        ring_through(self.mesh, self.tape(k), n, anchors)
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
            edge(
                "ankle",
                b.ring_through(Ankle, 8, &[b.leg(Ankle, false)]),
                true,
                Some(EdgeFinish::Raw),
            ),
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
            edge(
                "wrist",
                b.ring_through(Wrist, 8, &[b.arm(Wrist, false), b.arm(Wrist, true)]),
                true,
                Some(EdgeFinish::Raw),
            ),
            edge(
                "ankle",
                b.ring_through(Ankle, 8, &[b.leg(Ankle, false), b.leg(Ankle, true)]),
                true,
                Some(EdgeFinish::Raw),
            ),
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

/// Landmarks that need the skeleton too.
struct Skeleton<'a> {
    b: Landmarks<'a>,
    body: &'a HumanBody,
}

impl<'a> Skeleton<'a> {
    fn new(body: &'a HumanBody) -> Self {
        Self {
            b: Landmarks {
                mesh: &body.mesh,
                tapes: &body.tapes,
            },
            body,
        }
    }
    fn joint(&self, name: &str) -> DVec3 {
        self.body
            .joints
            .iter()
            .find(|j| j.0 == name)
            .map(|j| j.1)
            .expect("joint")
    }
    /// The +X armhole: the body's section by the vertical plane just inside
    /// the shoulder joint, around the top of the arm (front crease,
    /// armpit, back crease, top of the shoulder).
    fn armhole(&self) -> Vec<DVec3> {
        let s = self.joint("l-shoulder");
        let plane = Plane {
            point: s - 10.0 * DVec3::X,
            normal: DVec3::X,
        };
        nearest_section(self.b.mesh.positions(), self.b.mesh.triangles(), &plane)
            .expect("armhole section")
            .points
    }
    /// Ten points round the armhole, starting at the armpit and including
    /// the top of the shoulder, so the seams that end there meet it exactly.
    fn armhole_ring(&self) -> Vec<SurfacePoint> {
        let loop_ = self.armhole();
        let n = loop_.len();
        let lowest = |key: &dyn Fn(&DVec3) -> f64| {
            (0..n)
                .min_by(|&a, &b| key(&loop_[a]).total_cmp(&key(&loop_[b])))
                .expect("loop")
        };
        let pit = lowest(&|p| p.y);
        let tip = lowest(&|p| -p.y);
        let mut idx: Vec<usize> = (0..10).map(|i| (pit + i * n / 10) % n).collect();
        let near_tip = (0..10)
            .min_by_key(|&i| {
                let d = (idx[i] as isize - tip as isize).rem_euclid(n as isize) as usize;
                d.min(n - d)
            })
            .expect("ten points");
        idx[near_tip] = tip;
        idx.iter().map(|&i| self.b.at(loop_[i])).collect()
    }
    fn armpit(&self) -> DVec3 {
        extreme(&self.armhole(), |p| -p.y)
    }
    fn shoulder_tip(&self) -> DVec3 {
        extreme(&self.armhole(), |p| p.y)
    }
}

/// A short-sleeved top with set-in sleeves.
pub fn top_set_in(body: &HumanBody) -> GarmentDesign {
    use MeasureKind::*;
    let k = Skeleton::new(body);
    let b = &k.b;
    let (pit, tip) = (k.armpit(), k.shoulder_tip());
    GarmentDesign {
        curves: vec![
            edge("hem", b.ring(Hips, 12), false, Some(HEM)),
            edge("neckline", b.ring(Neck, 10), false, Some(TRIM)),
            edge(
                "sleeve",
                b.ring_through(UpperArm, 8, &[b.arm(UpperArm, false)]),
                true,
                Some(EdgeFinish::Raw),
            ),
            seam("armhole", k.armhole_ring(), true, SeamFinish::Glued).closed_loop(),
            seam(
                "side",
                b.points(&[
                    b.side(Hips),
                    b.side(Waist),
                    b.side(Underbust),
                    b.side(Bust),
                    pit,
                ]),
                true,
                SeamFinish::Glued,
            ),
            seam(
                "shoulder",
                b.points(&[b.side(Neck), tip]),
                true,
                SeamFinish::Glued,
            ),
            seam(
                "underarm",
                b.points(&[pit, b.arm(UpperArm, false)]),
                true,
                SeamFinish::Glued,
            ),
        ],
        inside: Some(b.at(extreme(b.tape(Waist), |p| p.z))),
        ..Default::default()
    }
}

/// A short-sleeved raglan top.
pub fn top_raglan(body: &HumanBody) -> GarmentDesign {
    use MeasureKind::*;
    let k = Skeleton::new(body);
    let b = &k.b;
    let pit = k.armpit();
    let neck = b.tape(Neck);
    let c = neck.iter().copied().sum::<DVec3>() / neck.len() as f64;
    // Where the raglan lines leave the neckline: diagonally front and back.
    let front = extreme(neck, |p| (p.x - c.x) + (p.z - c.z));
    let back = extreme(neck, |p| (p.x - c.x) - (p.z - c.z));
    // Raglan lines reach the armpit along the front and back of the
    // armhole, through its creases (halfway down from the shoulder), so
    // they meet the side seam at the armpit instead of crossing it.
    let hole = k.armhole();
    let mid_y = 0.5 * (pit.y + k.shoulder_tip().y);
    let crease = |front: bool| {
        extreme(&hole, |p| {
            let side = if front { p.z } else { -p.z };
            side - 2.0 * (p.y - mid_y).abs()
        })
    };
    GarmentDesign {
        curves: vec![
            edge("hem", b.ring(Hips, 12), false, Some(HEM)),
            edge("neckline", b.ring(Neck, 10), false, Some(TRIM)),
            edge(
                "sleeve",
                b.ring_through(UpperArm, 8, &[b.arm(UpperArm, false)]),
                true,
                Some(EdgeFinish::Raw),
            ),
            seam(
                "side",
                b.points(&[
                    b.side(Hips),
                    b.side(Waist),
                    b.side(Underbust),
                    b.side(Bust),
                    pit,
                    b.arm(UpperArm, false),
                ]),
                true,
                SeamFinish::Glued,
            ),
            seam(
                "front raglan",
                b.points(&[front, crease(true), pit]),
                true,
                SeamFinish::Glued,
            ),
            seam(
                "back raglan",
                b.points(&[back, crease(false), pit]),
                true,
                SeamFinish::Glued,
            ),
        ],
        inside: Some(b.at(extreme(b.tape(Waist), |p| p.z))),
        ..Default::default()
    }
}

/// [`top_finished`] with princess seams: from the middle of each shoulder
/// over the bust point, then down to the hem.
pub fn top_princess(body: &HumanBody) -> GarmentDesign {
    use MeasureKind::*;
    let b = Landmarks {
        mesh: &body.mesh,
        tapes: &body.tapes,
    };
    let mut d = top_finished(&body.mesh, &body.tapes);
    let apex = extreme(b.tape(Bust), |p| if p.x > 20.0 { p.z } else { f64::MIN });
    let front_at = |k, x: f64| {
        *b.tape(k)
            .iter()
            .filter(|p| p.z > 0.0)
            .min_by(|a, c| (a.x - x).abs().total_cmp(&(c.x - x).abs()))
            .expect("loop")
    };
    let shoulder = b.side(Neck).lerp(extreme(b.tape(UpperArm), |p| p.y), 0.4);
    d.curves.push(seam(
        "princess",
        b.points(&[
            shoulder,
            apex,
            front_at(Underbust, apex.x),
            front_at(Waist, 0.8 * apex.x),
            front_at(Hips, 0.85 * apex.x),
        ]),
        true,
        SeamFinish::Glued,
    ));
    d
}

/// A catsuit with set-in sleeves and a diamond crotch gusset.
pub fn catsuit_set_in(body: &HumanBody) -> GarmentDesign {
    use MeasureKind::*;
    let k = Skeleton::new(body);
    let b = &k.b;
    let (pit, tip) = (k.armpit(), k.shoulder_tip());
    let crotch = b.crotch();
    // The gusset: an oval round the crotch, 60 mm to the front and back
    // along the centre line and 45 mm onto each inner thigh, as eight points
    // placed on the skin. Its front and back points lie on the centre line,
    // where the centre seams end; the inside-leg seams start at its sides.
    let oval = |theta: f64| {
        let target = crotch + DVec3::new(45.0 * theta.sin(), 10.0, 60.0 * theta.cos());
        let p = b
            .at(target)
            .position(b.mesh.positions(), b.mesh.triangles())
            .expect("on body");
        if theta.sin().abs() < 1e-9 {
            DVec3::new(0.0, p.y, p.z)
        } else {
            p
        }
    };
    let ring: Vec<DVec3> = (0..8)
        .map(|k| oval(k as f64 * std::f64::consts::FRAC_PI_4))
        .collect();
    let (gf, gb, gs) = (ring[0], ring[4], ring[2]);
    let torso = |front: bool, end: DVec3| {
        let mut v = vec![b.centre(Neck, front)];
        v.extend([Bust, Underbust, Waist, Hips].map(|kk| b.centre(kk, front)));
        v.push(end);
        b.points(&v)
    };
    GarmentDesign {
        curves: vec![
            edge("neckline", b.ring(Neck, 10), false, Some(TRIM)),
            edge(
                "wrist",
                b.ring_through(Wrist, 8, &[b.arm(Wrist, false), b.arm(Wrist, true)]),
                true,
                Some(EdgeFinish::Raw),
            ),
            edge(
                "ankle",
                b.ring_through(Ankle, 8, &[b.leg(Ankle, false), b.leg(Ankle, true)]),
                true,
                Some(EdgeFinish::Raw),
            ),
            seam("centre front", torso(true, gf), false, SeamFinish::Glued),
            seam("centre back", torso(false, gb), false, SeamFinish::Zip),
            seam("gusset", b.points(&ring), false, SeamFinish::Glued).closed_loop(),
            seam("armhole", k.armhole_ring(), true, SeamFinish::Glued).closed_loop(),
            seam(
                "shoulder",
                b.points(&[b.side(Neck), tip]),
                true,
                SeamFinish::Glued,
            ),
            seam(
                "underarm",
                b.points(&[pit, b.arm(UpperArm, false), b.arm(Wrist, false)]),
                true,
                SeamFinish::Glued,
            ),
            seam(
                "side",
                b.points(&[
                    pit,
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
                    gs,
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

impl Curve {
    fn closed_loop(mut self) -> Self {
        self.closed = true;
        self
    }
}
