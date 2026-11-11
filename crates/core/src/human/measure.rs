//! Measuring a morphed body the way a tape measure would.
//!
//! ## Where each measurement is taken
//! Each girth has a **landmark**: a fixed set of body vertices with
//! weights, whose weighted mean position `c = Σ wᵢ pᵢ / Σ wᵢ` follows the
//! body as it morphs. The sets come from the model's own measurement
//! targets (the vertices the "waist" target moves, weighted by how far it
//! moves them), so a girth is measured exactly where the shape parameter
//! that controls it acts. Because the vertex sets are fixed, every
//! measurement is a smooth function of the shape parameters, which the
//! fitting solver relies on.
//!
//! - Torso girths (bust, underbust, waist, hips) cut horizontally at the
//!   landmark's height and keep the loop around the spine.
//! - Limb and neck girths cut perpendicular to the bone (the line between
//!   two joints) through the landmark, on the body's left side.
//! - Lengths are distances between joints or fixed vertices: see
//!   [`MeasureKind`].
//!
//! Girths are convex-hull perimeters (see [`slice`](super::slice)).
//!
//! ## Fullest and narrowest
//! People measure the waist at its narrowest and the hips, bust, thigh and
//! calf at their fullest, wherever that falls. Those girths are therefore
//! the minimum or maximum over a few parallel cuts in a window around the
//! landmark. Each cut is a smooth function of the shape,
//! so the extreme is continuous. Cuts whose girth differs by more than
//! about 12 % from the landmark's own are ignored: that is a cut that ran
//! into the arms or the other leg.
//!
//! ## Underbust
//! A tape under the bust lies on the ribcage, but a horizontal cut at that
//! level can pass through the underside of the breasts. Before taking the
//! hull, the front half of the cut is clipped to the depth, and (with a
//! small margin) the width, of the body 80 mm lower, below the breasts,
//! which is where the tape would sit.

use super::model::{BodyModel, Shape};
use super::slice::{nearest_section, Plane};
use crate::error::{Error, Result};
use glam::DVec3;

/// Everything that can be measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum MeasureKind {
    /// Floor to top of head.
    Height,
    /// Floor to crotch (inside leg).
    Inseam,
    /// Floor to waist level.
    WaistHeight,
    /// Shoulder joint to elbow to wrist.
    ArmLength,
    Neck,
    Bust,
    Underbust,
    Waist,
    Hips,
    /// Around the thigh, just below the crotch.
    Thigh,
    Knee,
    Calf,
    Ankle,
    UpperArm,
    Wrist,
}

impl MeasureKind {
    pub const ALL: [MeasureKind; 15] = [
        Self::Height,
        Self::Inseam,
        Self::WaistHeight,
        Self::ArmLength,
        Self::Neck,
        Self::Bust,
        Self::Underbust,
        Self::Waist,
        Self::Hips,
        Self::Thigh,
        Self::Knee,
        Self::Calf,
        Self::Ankle,
        Self::UpperArm,
        Self::Wrist,
    ];

    /// The `snake_case` name used in files.
    pub fn name(self) -> &'static str {
        match self {
            Self::Height => "height",
            Self::Inseam => "inseam",
            Self::WaistHeight => "waist_height",
            Self::ArmLength => "arm_length",
            Self::Neck => "neck",
            Self::Bust => "bust",
            Self::Underbust => "underbust",
            Self::Waist => "waist",
            Self::Hips => "hips",
            Self::Thigh => "thigh",
            Self::Knee => "knee",
            Self::Calf => "calf",
            Self::Ankle => "ankle",
            Self::UpperArm => "upper_arm",
            Self::Wrist => "wrist",
        }
    }
}

/// A weighted vertex set whose mean follows the body.
#[derive(Debug, Clone)]
struct Landmark {
    indices: Vec<u32>,
    weights: Vec<f64>,
    /// Triangles near the landmark (for speed); slicing falls back to the
    /// whole mesh if these do not give a closed loop.
    tris: Vec<[u32; 3]>,
}

impl Landmark {
    fn centre(&self, s: &Shape) -> DVec3 {
        let (mut c, mut w) = (DVec3::ZERO, 0.0);
        for (&i, &wi) in self.indices.iter().zip(&self.weights) {
            c += wi * s.positions[i as usize];
            w += wi;
        }
        c / w
    }
}

/// Which cut in a window around the landmark counts.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Search {
    /// The cut at the landmark only.
    At,
    /// Narrowest cut for offsets in `[lo, hi]` mm along the cut normal.
    Min(f64, f64),
    /// Fullest cut for offsets in `[lo, hi]` mm along the cut normal.
    Max(f64, f64),
}

/// Distance below the underbust cut (mm) at which the body sets the depth
/// and width of the tape's front half.
const UNDERBUST_REFERENCE: f64 = 80.0;
/// Extra half-width (mm) allowed over the reference cut.
const UNDERBUST_MARGIN: f64 = 8.0;

/// `(kind, measurement target, bone joints, search)` for one girth.
type GirthSpec<'a> = (MeasureKind, &'a str, Option<(&'a str, &'a str)>, Search);

/// Number of cuts tried in a search window.
const SAMPLES: usize = 7;

/// A girth: where to cut and along which bone (or horizontally).
#[derive(Debug, Clone)]
struct GirthDef {
    kind: MeasureKind,
    landmark: Landmark,
    /// `(from joint, to joint)` giving the plane normal; `None` = horizontal.
    bone: Option<(usize, usize)>,
    search: Search,
}

/// Precomputed measurement definitions for a [`BodyModel`].
#[derive(Debug, Clone)]
pub struct Measurer {
    girths: Vec<GirthDef>,
    ground: usize,
    crotch_vertex: u32,
    /// Surface point on top of the left shoulder.
    shoulder_vertex: u32,
    /// Left elbow and wrist joints.
    arm: [usize; 2],
}

/// Half-thickness (mm, along the cut normal, in the base shape) of the band
/// of triangles kept around each landmark.
const BAND: f64 = 150.0;

impl Measurer {
    pub fn new(model: &BodyModel) -> Result<Self> {
        let base = Shape {
            positions: model.base.clone(),
        };
        let j = |n: &str| model.joint(n);
        use Search::*;
        // Normals: horizontal cuts point up; bone cuts point from the first
        // joint to the second (down the leg, down the arm, up the neck).
        let defs: [GirthSpec; 11] = [
            (
                MeasureKind::Neck,
                "neck-circ",
                Some(("neck", "head")),
                Min(-20.0, 20.0),
            ),
            (MeasureKind::Bust, "bust-circ", None, Max(-40.0, 10.0)),
            (MeasureKind::Underbust, "underbust-circ", None, At),
            (MeasureKind::Waist, "waist-circ", None, Min(-50.0, 50.0)),
            (MeasureKind::Hips, "hips-circ", None, Max(-60.0, 60.0)),
            (
                MeasureKind::Thigh,
                "thigh-circ",
                Some(("l-upper-leg", "l-knee")),
                Max(-90.0, 0.0),
            ),
            (
                MeasureKind::Knee,
                "knee-circ",
                Some(("l-upper-leg", "l-ankle")),
                At,
            ),
            (
                MeasureKind::Calf,
                "calf-circ",
                Some(("l-knee", "l-ankle")),
                Max(-40.0, 40.0),
            ),
            (
                MeasureKind::Ankle,
                "ankle-circ",
                Some(("l-knee", "l-ankle")),
                Min(-30.0, 30.0),
            ),
            (
                MeasureKind::UpperArm,
                "upperarm-circ",
                Some(("l-shoulder", "l-elbow")),
                Max(-30.0, 30.0),
            ),
            (
                MeasureKind::Wrist,
                "wrist-circ",
                Some(("l-elbow", "l-hand")),
                Min(-20.0, 20.0),
            ),
        ];
        let mut girths = Vec::new();
        for (kind, target, bone, search) in defs {
            let t = &model.targets[model.target(&format!("measure-{target}-incr"))?];
            let limb = bone.is_some() && kind != MeasureKind::Neck;
            // The underbust tape lies on the ribcage; the target also moves
            // the breasts, so only its back half marks the level.
            let spine_z = base.joint(model, j("spine-2")?).z;
            let (mut indices, mut weights) = (Vec::new(), Vec::new());
            for (&i, d) in t.indices.iter().zip(&t.deltas) {
                let p = model.base[i as usize];
                // Body vertices only; limbs use the body's left side.
                if (i as usize) < model.n_body
                    && (!limb || p.x > 0.0)
                    && (kind != MeasureKind::Underbust || p.z < spine_z)
                {
                    indices.push(i);
                    weights.push(d.length());
                }
            }
            if indices.is_empty() {
                return Err(Error::Body(format!(
                    "target '{target}' moves no body vertices"
                )));
            }
            let bone = match bone {
                Some((a, b)) => Some((j(a)?, j(b)?)),
                None => None,
            };
            let mut landmark = Landmark {
                indices,
                weights,
                tris: Vec::new(),
            };
            let c = landmark.centre(&base);
            let n = normal(&base, model, bone);
            landmark.tris = model
                .triangles
                .iter()
                .filter(|t| {
                    let m = t.iter().map(|&i| model.base[i as usize]).sum::<DVec3>() / 3.0;
                    ((m - c).dot(n)).abs() < BAND && (!limb || m.x > 0.0)
                })
                .copied()
                .collect();
            girths.push(GirthDef {
                kind,
                landmark,
                bone,
                search,
            });
        }

        // Crotch: the lowest midline vertex between the hip joints and knees.
        let (hip, knee) = (
            base.joint(model, j("l-upper-leg")?),
            base.joint(model, j("l-knee")?),
        );
        let crotch_vertex = (0..model.n_body)
            .filter(|&i| {
                let p = model.base[i];
                p.x.abs() < 0.5 && p.y < hip.y && p.y > knee.y
            })
            .min_by(|&a, &b| model.base[a].y.total_cmp(&model.base[b].y))
            .ok_or_else(|| Error::Body("no crotch vertex found on the midline".into()))?
            as u32;

        // Shoulder point: the surface vertex most nearly straight above the
        // shoulder joint (where a tape for the arm length starts).
        let sj = base.joint(model, j("l-shoulder")?);
        let shoulder_vertex = (0..model.n_body)
            .filter(|&i| {
                let p = model.base[i];
                p.y > sj.y && p.y < sj.y + 150.0
            })
            .min_by(|&a, &b| {
                let h = |i: usize| (model.base[i] - sj).with_y(0.0).length_squared();
                h(a).total_cmp(&h(b))
            })
            .ok_or_else(|| Error::Body("no shoulder point found".into()))?
            as u32;

        Ok(Self {
            girths,
            ground: j("ground")?,
            crotch_vertex,
            shoulder_vertex,
            arm: [j("l-elbow")?, j("l-hand")?],
        })
    }

    /// The cutting plane of a girth on `shape` (for display).
    pub fn plane(&self, model: &BodyModel, shape: &Shape, kind: MeasureKind) -> Option<Plane> {
        let g = self.girths.iter().find(|g| g.kind == kind)?;
        let c = g.landmark.centre(shape);
        let point = if g.bone.is_none() {
            DVec3::new(0.0, c.y, c.z)
        } else {
            c
        };
        Some(Plane {
            point,
            normal: normal(shape, model, g.bone),
        })
    }

    /// The cross-section loop of a girth at its landmark (for display).
    pub fn tape(&self, model: &BodyModel, shape: &Shape, kind: MeasureKind) -> Option<Vec<DVec3>> {
        let g = self.girths.iter().find(|g| g.kind == kind)?;
        let plane = self.plane(model, shape, kind)?;
        let body = shape.body(model);
        nearest_section(body, &g.landmark.tris, &plane)
            .or_else(|| nearest_section(body, &model.triangles, &plane))
            .map(|s| s.points)
    }

    /// Measure one quantity (mm).
    pub fn measure(&self, model: &BodyModel, shape: &Shape, kind: MeasureKind) -> Result<f64> {
        let ground = shape.joint(model, self.ground).y;
        Ok(match kind {
            MeasureKind::Height => {
                shape
                    .body(model)
                    .iter()
                    .map(|p| p.y)
                    .fold(f64::MIN, f64::max)
                    - ground
            }
            MeasureKind::Inseam => shape.positions[self.crotch_vertex as usize].y - ground,
            MeasureKind::WaistHeight => {
                self.plane(model, shape, MeasureKind::Waist)
                    .expect("waist is defined")
                    .point
                    .y
                    - ground
            }
            MeasureKind::ArmLength => {
                let a = shape.positions[self.shoulder_vertex as usize];
                let [b, c] = self.arm.map(|j| shape.joint(model, j));
                a.distance(b) + b.distance(c)
            }
            _ => {
                let g = self
                    .girths
                    .iter()
                    .find(|g| g.kind == kind)
                    .expect("girth is defined");
                let plane = self.plane(model, shape, kind).unwrap();
                let body = shape.body(model);
                let section = |p: &Plane| {
                    nearest_section(body, &g.landmark.tris, p)
                        .or_else(|| nearest_section(body, &model.triangles, p))
                };
                let cut = |offset: f64| {
                    let p = Plane {
                        point: plane.point + offset * plane.normal,
                        normal: plane.normal,
                    };
                    let mut s = section(&p)?;
                    if kind == MeasureKind::Underbust {
                        let below = Plane {
                            point: p.point - UNDERBUST_REFERENCE * DVec3::Y,
                            normal: p.normal,
                        };
                        let reference = section(&below)?;
                        let front = reference
                            .points
                            .iter()
                            .map(|q| q.z)
                            .fold(f64::MIN, f64::max);
                        let half_width = reference
                            .points
                            .iter()
                            .map(|q| q.x.abs())
                            .fold(0.0, f64::max)
                            + UNDERBUST_MARGIN;
                        let mid = s.centroid.z;
                        for q in &mut s.points {
                            q.z = q.z.min(front);
                            if q.z > mid {
                                q.x = q.x.clamp(-half_width, half_width);
                            }
                        }
                    }
                    Some(s.hull_length(&p))
                };
                let at = cut(0.0)
                    .ok_or_else(|| Error::Body(format!("could not measure {}", kind.name())))?;
                let window = |lo: f64, hi: f64| {
                    (0..SAMPLES)
                        .filter_map(move |k| cut(lo + (hi - lo) * k as f64 / (SAMPLES - 1) as f64))
                        .filter(move |v| (0.85 * at..=1.12 * at).contains(v))
                };
                match g.search {
                    Search::At => at,
                    Search::Min(lo, hi) => window(lo, hi).fold(at, f64::min),
                    Search::Max(lo, hi) => window(lo, hi).fold(at, f64::max),
                }
            }
        })
    }

    /// Measure everything.
    pub fn measure_all(&self, model: &BodyModel, shape: &Shape) -> Result<Vec<(MeasureKind, f64)>> {
        MeasureKind::ALL
            .iter()
            .map(|&k| Ok((k, self.measure(model, shape, k)?)))
            .collect()
    }
}

fn normal(shape: &Shape, model: &BodyModel, bone: Option<(usize, usize)>) -> DVec3 {
    match bone {
        Some((a, b)) => (shape.joint(model, b) - shape.joint(model, a)).normalize(),
        None => DVec3::Y,
    }
}
