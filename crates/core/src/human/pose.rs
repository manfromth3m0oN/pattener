//! Posing the fitted body with its skeleton.
//!
//! The body model carries a reduced skeleton (MakeHuman's default rig, CC0,
//! folded into 21 bones that each turn about one of the landmark joints)
//! and per-vertex bone weights. A pose bends it by skinning: each vertex
//! follows a blend of its bones' rigid transforms `(R_b, t_b)`, weighted
//! by its bone weights (dual quaternion blending, see [`pose_points`]),
//! where each bone's transform comes from its parent's
//! and its own turn about its joint. Bones are turned from the top of the
//! hierarchy down (forward kinematics): with the parent's transform
//! `(R_p, t_p)` and the bone's turn `Q` about its posed joint
//! `j' = R_p j + t_p`,
//!
//! `R_b = Q R_p`, `t_b = j' − R_b j`.
//!
//! Poses are written as *aims*, the direction a bone should point in
//! (from its joint to its child's), so the same pose fits any body: `Q` is
//! the shortest turn from the bone's current direction `R_p a` to the aim.
//! A bone can instead *keep* its standing orientation (`Q = R_p⁻¹`), which
//! keeps feet flat in a squat. Poses for the left side are mirrored to the
//! right (`x → −x`) unless they give the right side themselves.
//!
//! Points that are not body vertices (a garment's cut mesh) take the
//! weights of the nearest point on the body surface: the barycentric blend
//! of its triangle's vertex weights ([`skin_points`]).

use super::fit::HumanBody;
use super::model::BodyModel;
use crate::error::{Error, Result};
use crate::garment::edit::closest_on_triangle;
use glam::{DQuat, DVec3};
use std::collections::HashMap;

/// What a bone does in a pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Aim {
    /// Point the bone this way (body coordinates: +X to the body's left,
    /// +Y up, +Z to the front).
    Dir(DVec3),
    /// Keep the orientation it has standing.
    Keep,
}

/// A named pose.
#[derive(Debug, Clone, Copy)]
pub struct Pose {
    /// File-name-safe id, e.g. `arms-up`.
    pub id: &'static str,
    pub name: &'static str,
    /// Aims by bone name. Left-side bones (`l-…`) are mirrored to the
    /// right unless `mirror` is false.
    pub aims: &'static [(&'static str, Aim)],
    pub mirror: bool,
}

const fn d(x: f64, y: f64, z: f64) -> Aim {
    Aim::Dir(DVec3::new(x, y, z))
}

/// The pose library. The first is the standing pose the body is modelled
/// (and patterns are drafted) in.
pub const POSES: [Pose; 7] = [
    Pose {
        id: "standing",
        name: "Standing",
        aims: &[],
        mirror: true,
    },
    Pose {
        id: "arms-up",
        name: "Arms up",
        aims: &[
            ("l-upper-arm", d(0.6, 1.0, 0.1)),
            ("l-forearm", d(0.35, 1.0, 0.15)),
        ],
        mirror: true,
    },
    Pose {
        id: "arms-forward",
        name: "Reaching forward",
        aims: &[
            ("l-upper-arm", d(0.2, -0.1, 1.0)),
            ("l-forearm", d(0.1, 0.0, 1.0)),
        ],
        mirror: true,
    },
    Pose {
        id: "arms-out",
        name: "Arms out",
        aims: &[
            ("l-upper-arm", d(1.0, 0.0, 0.0)),
            ("l-forearm", d(1.0, 0.0, 0.0)),
        ],
        mirror: true,
    },
    Pose {
        id: "squat",
        name: "Squat",
        aims: &[
            ("spine-lower", d(0.0, 1.0, 0.4)),
            ("l-thigh", d(0.25, -0.35, 1.0)),
            ("l-shin", d(0.05, -1.0, -0.45)),
            ("l-foot", Aim::Keep),
            ("l-upper-arm", d(0.15, -0.2, 1.0)),
            ("l-forearm", d(0.1, -0.1, 1.0)),
        ],
        mirror: true,
    },
    Pose {
        id: "sitting",
        name: "Sitting",
        aims: &[
            ("l-thigh", d(0.12, -0.05, 1.0)),
            ("l-shin", d(0.0, -1.0, 0.05)),
            ("l-foot", Aim::Keep),
        ],
        mirror: true,
    },
    Pose {
        id: "step",
        name: "Walking step",
        aims: &[
            ("l-thigh", d(0.05, -0.85, 0.5)),
            ("l-shin", d(0.0, -0.95, 0.1)),
            ("l-foot", Aim::Keep),
            ("r-thigh", d(-0.05, -0.95, -0.3)),
            ("r-shin", d(0.0, -0.9, -0.45)),
            ("r-foot", Aim::Keep),
            ("l-upper-arm", d(0.35, -0.9, -0.25)),
            ("r-upper-arm", d(-0.35, -0.9, 0.3)),
        ],
        mirror: false,
    },
];

/// The pose with this id.
pub fn pose_by_id(id: &str) -> Option<&'static Pose> {
    POSES.iter().find(|p| p.id == id)
}

/// A rigid transform `v → R v + t` per bone.
pub type Transforms = Vec<(DQuat, DVec3)>;

/// Each bone's transform for `pose` on a fitted body.
pub fn bone_transforms(model: &BodyModel, body: &HumanBody, pose: &Pose) -> Result<Transforms> {
    let skin = model
        .skin
        .as_ref()
        .ok_or_else(|| Error::Body("this body model has no skeleton for posing".into()))?;
    let joints: HashMap<&str, DVec3> = body.joints.iter().map(|(n, p)| (n.as_str(), *p)).collect();
    let pivot = |b: usize| -> Result<DVec3> {
        let name = &model.joint_names[skin.bones[b].joint];
        joints
            .get(name.as_str())
            .copied()
            .ok_or_else(|| Error::Body(format!("the fitted body has no joint '{name}'")))
    };
    // Rest direction of each bone: towards its first child, or along its
    // parent for the end of a chain.
    let n = skin.bones.len();
    let mut axis = vec![DVec3::ZERO; n];
    for b in 0..n {
        if let Some(c) = (0..n).find(|&c| skin.bones[c].parent == Some(b)) {
            axis[b] = (pivot(c)? - pivot(b)?).normalize_or_zero();
        }
    }
    for b in 0..n {
        if axis[b] == DVec3::ZERO {
            axis[b] = skin.bones[b].parent.map_or(DVec3::Y, |p| axis[p]);
        }
    }
    // Aims, with left-side ones mirrored to the right.
    let mut aims: HashMap<String, Aim> = HashMap::new();
    for &(bone, aim) in pose.aims {
        aims.insert(bone.to_string(), aim);
        if pose.mirror {
            if let Some(rest) = bone.strip_prefix("l-") {
                let m = match aim {
                    Aim::Dir(v) => Aim::Dir(DVec3::new(-v.x, v.y, v.z)),
                    Aim::Keep => Aim::Keep,
                };
                aims.entry(format!("r-{rest}")).or_insert(m);
            }
        }
    }
    for name in aims.keys() {
        if !skin.bones.iter().any(|b| &b.name == name) {
            return Err(Error::Body(format!(
                "pose '{}' names unknown bone '{name}'",
                pose.id
            )));
        }
    }
    let mut out: Transforms = Vec::with_capacity(n);
    for b in 0..n {
        let (rp, tp) = skin.bones[b]
            .parent
            .map_or((DQuat::IDENTITY, DVec3::ZERO), |p| out[p]);
        let j = pivot(b)?;
        let q = match aims.get(&skin.bones[b].name) {
            Some(Aim::Dir(t)) => {
                DQuat::from_rotation_arc((rp * axis[b]).normalize(), t.normalize())
            }
            Some(Aim::Keep) => rp.inverse(),
            None => DQuat::IDENTITY,
        };
        let r = (q * rp).normalize();
        let j_posed = rp * j + tp;
        out.push((r, j_posed - r * j));
    }
    Ok(out)
}

/// Skinning weights `(bone, weight)` for each point, from the nearest
/// point of the body's surface.
pub fn skin_points(
    model: &BodyModel,
    body: &HumanBody,
    points: &[DVec3],
) -> Result<Vec<Vec<(u8, f32)>>> {
    let (pos, tri) = (body.mesh.positions(), body.mesh.triangles());
    let body_w = body_weights(model)?;
    let grid = Grid::new(pos, tri, 25.0);
    Ok(points
        .iter()
        .map(|&p| {
            let (f, u, v) = grid.closest(p);
            let t = tri[f as usize];
            let bary = [1.0 - u - v, u, v];
            let mut acc: Vec<(u8, f32)> = Vec::with_capacity(6);
            for (k, &i) in t.iter().enumerate() {
                for &(b, w) in &body_w[i as usize] {
                    let w = w * bary[k] as f32;
                    match acc.iter_mut().find(|e| e.0 == b) {
                        Some(e) => e.1 += w,
                        None => acc.push((b, w)),
                    }
                }
            }
            acc
        })
        .collect())
}

/// Body vertex weights: the body model's own, spread over the surface by
/// [`WEIGHT_SMOOTHING`] rounds of neighbour averaging so that a bend is
/// shared over a band of skin a few centimetres wide instead of a few
/// triangles (which overstretches the skin at the armpit and groin).
pub fn body_weights(model: &BodyModel) -> Result<Vec<Vec<(u8, f32)>>> {
    let skin = model
        .skin
        .as_ref()
        .ok_or_else(|| Error::Body("this body model has no skeleton for posing".into()))?;
    let n = skin.weights.len();
    let nb_count = skin.bones.len();
    let mut nb: Vec<Vec<u32>> = vec![Vec::new(); n];
    for t in &model.triangles {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if !nb[a as usize].contains(&b) {
                nb[a as usize].push(b);
                nb[b as usize].push(a);
            }
        }
    }
    let mut dense: Vec<Vec<f32>> = skin
        .weights
        .iter()
        .map(|w| {
            let mut d = vec![0.0; nb_count];
            for &(b, x) in w {
                d[b as usize] += x;
            }
            d
        })
        .collect();
    for _ in 0..WEIGHT_SMOOTHING {
        let old = dense.clone();
        for v in 0..n {
            if nb[v].is_empty() {
                continue;
            }
            for b in 0..nb_count {
                let avg =
                    nb[v].iter().map(|&u| old[u as usize][b]).sum::<f32>() / nb[v].len() as f32;
                dense[v][b] = 0.5 * old[v][b] + 0.5 * avg;
            }
        }
    }
    Ok(dense
        .into_iter()
        .map(|d| {
            let mut w: Vec<(u8, f32)> = d
                .iter()
                .enumerate()
                .filter(|(_, &x)| x > 1e-4)
                .map(|(b, &x)| (b as u8, x))
                .collect();
            w.sort_by(|a, b| b.1.total_cmp(&a.1));
            w.truncate(4);
            let total: f32 = w.iter().map(|e| e.1).sum();
            for e in &mut w {
                e.1 /= total;
            }
            w
        })
        .collect())
}

/// Rounds of neighbour averaging applied to the skinning weights.
pub const WEIGHT_SMOOTHING: usize = 20;

/// Move points by skinning: dual quaternion blending (Kavan et al., 2007)
/// of the bones' rigid transforms. Unlike blending the transforms
/// linearly, it does not collapse or bulge the skin around joints that
/// turn far (a raised arm): each bone's `(R, t)` becomes a unit dual
/// quaternion `q₀ + ε q_ε` (`q_ε = ½ (0, t) q₀`), the weighted sum (with
/// each `q₀` turned to the same hemisphere as the dominant bone's) is
/// normalised, and the point is moved by the resulting rigid transform.
pub fn pose_points(
    points: &[DVec3],
    weights: &[Vec<(u8, f32)>],
    transforms: &Transforms,
) -> Vec<DVec3> {
    let duals: Vec<(DQuat, DQuat)> = transforms
        .iter()
        .map(|&(r, t)| (r, DQuat::from_xyzw(t.x, t.y, t.z, 0.0) * r * 0.5))
        .collect();
    points
        .iter()
        .zip(weights)
        .map(|(&v, w)| {
            let Some(&(lead, _)) = w.iter().max_by(|a, b| a.1.total_cmp(&b.1)) else {
                return v;
            };
            let pivot = duals[lead as usize].0;
            let (mut real, mut dual) = (
                DQuat::from_xyzw(0.0, 0.0, 0.0, 0.0),
                DQuat::from_xyzw(0.0, 0.0, 0.0, 0.0),
            );
            for &(b, x) in w {
                let (q0, qe) = duals[b as usize];
                let s = if q0.dot(pivot) < 0.0 {
                    -(x as f64)
                } else {
                    x as f64
                };
                real += q0 * s;
                dual += qe * s;
            }
            let n = real.length();
            if n < 1e-12 {
                return v;
            }
            let (real, dual) = (real / n, dual / n);
            // Translation t = 2 q_ε q₀*, rotation q₀.
            let t = dual * 2.0 * real.conjugate();
            real * v + DVec3::new(t.x, t.y, t.z)
        })
        .collect()
}

/// The fitted body's vertices in `pose`.
pub fn pose_body(model: &BodyModel, body: &HumanBody, pose: &Pose) -> Result<Vec<DVec3>> {
    let t = bone_transforms(model, body, pose)?;
    Ok(pose_points(
        body.mesh.positions(),
        &body_weights(model)?,
        &t,
    ))
}

/// Uniform grid of triangles for nearest-point queries near the surface.
struct Grid<'a> {
    pos: &'a [DVec3],
    tri: &'a [[u32; 3]],
    cell: f64,
    cells: HashMap<(i64, i64, i64), Vec<u32>>,
}

impl<'a> Grid<'a> {
    fn new(pos: &'a [DVec3], tri: &'a [[u32; 3]], cell: f64) -> Self {
        let key = |x: f64| (x / cell).floor() as i64;
        let mut cells: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
        for (f, t) in tri.iter().enumerate() {
            let [a, b, c] = t.map(|i| pos[i as usize]);
            let (lo, hi) = (a.min(b).min(c), a.max(b).max(c));
            for i in key(lo.x)..=key(hi.x) {
                for j in key(lo.y)..=key(hi.y) {
                    for k in key(lo.z)..=key(hi.z) {
                        cells.entry((i, j, k)).or_default().push(f as u32);
                    }
                }
            }
        }
        Self {
            pos,
            tri,
            cell,
            cells,
        }
    }

    /// Nearest triangle and barycentric `(u, v)` (searching outwards).
    fn closest(&self, p: DVec3) -> (u32, f64, f64) {
        let key = |x: f64| (x / self.cell).floor() as i64;
        let (ci, cj, ck) = (key(p.x), key(p.y), key(p.z));
        let mut best: Option<(f64, u32, f64, f64)> = None;
        for r in 1..=8i64 {
            for i in ci - r..=ci + r {
                for j in cj - r..=cj + r {
                    for k in ck - r..=ck + r {
                        for &f in self.cells.get(&(i, j, k)).into_iter().flatten() {
                            let [a, b, c] = self.tri[f as usize].map(|v| self.pos[v as usize]);
                            let (u, v) = closest_on_triangle(p, a, b, c);
                            let q = a + u * (b - a) + v * (c - a);
                            let d = q.distance_squared(p);
                            if best.is_none_or(|x| d < x.0) {
                                best = Some((d, f, u, v));
                            }
                        }
                    }
                }
            }
            if let Some((d, f, u, v)) = best {
                // Anything nearer lies within distance √d.
                if d.sqrt() <= (r as f64 - 1.0) * self.cell {
                    return (f, u, v);
                }
            }
        }
        best.map_or((0, 0.0, 0.0), |(_, f, u, v)| (f, u, v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::human::{fit, test_model, Measurements, Measurer, ShapeOptions};

    fn body() -> (BodyModel, HumanBody) {
        let m = test_model();
        let me = Measurer::new(&m).unwrap();
        let b = fit(&m, &me, &Measurements::new(), &ShapeOptions::default()).unwrap();
        (m, b)
    }

    fn joint(b: &HumanBody, name: &str) -> DVec3 {
        b.joints.iter().find(|j| j.0 == name).unwrap().1
    }

    /// Where a joint goes: transformed by its own bone.
    fn posed_joint(m: &BodyModel, b: &HumanBody, t: &Transforms, bone: &str) -> DVec3 {
        let skin = m.skin.as_ref().unwrap();
        let k = skin.bones.iter().position(|x| x.name == bone).unwrap();
        let (r, tr) = t[k];
        r * joint(b, &m.joint_names[skin.bones[k].joint]) + tr
    }

    #[test]
    fn standing_leaves_the_body_as_it_is() {
        let (m, b) = body();
        let p = pose_body(&m, &b, pose_by_id("standing").unwrap()).unwrap();
        for (a, c) in p.iter().zip(b.mesh.positions()) {
            assert!(a.distance(*c) < 1e-3, "{a} vs {c}");
        }
    }

    #[test]
    fn poses_move_limbs_rigidly_and_symmetrically() {
        let (m, b) = body();
        let head = joint(&b, "head");
        for pose in &POSES {
            let t = bone_transforms(&m, &b, pose).unwrap();
            // Bones keep their length: the elbow and the hand stay as far
            // from the shoulder and the elbow as standing.
            for (a, c) in [
                ("l-upper-arm", "l-forearm"),
                ("l-forearm", "l-hand"),
                ("l-thigh", "l-shin"),
            ] {
                let rest = joint(
                    &b,
                    &m.joint_names[m
                        .skin
                        .as_ref()
                        .unwrap()
                        .bones
                        .iter()
                        .find(|x| x.name == a)
                        .unwrap()
                        .joint],
                )
                .distance(joint(
                    &b,
                    &m.joint_names[m
                        .skin
                        .as_ref()
                        .unwrap()
                        .bones
                        .iter()
                        .find(|x| x.name == c)
                        .unwrap()
                        .joint],
                ));
                let now = posed_joint(&m, &b, &t, a).distance(posed_joint(&m, &b, &t, c));
                assert!(
                    (rest - now).abs() < 1e-6,
                    "{}: {a}–{c} {rest} vs {now}",
                    pose.id
                );
            }
            if pose.mirror {
                let (l, r) = (
                    posed_joint(&m, &b, &t, "l-hand"),
                    posed_joint(&m, &b, &t, "r-hand"),
                );
                assert!(
                    (l.x + r.x).abs() < 1e-6
                        && (l.y - r.y).abs() < 1e-6
                        && (l.z - r.z).abs() < 1e-6,
                    "{}",
                    pose.id
                );
            }
            let p = pose_body(&m, &b, pose).unwrap();
            assert!(p.iter().all(|v| v.is_finite()));
        }
        // Arms up: the hands are above the head.
        let t = bone_transforms(&m, &b, pose_by_id("arms-up").unwrap()).unwrap();
        assert!(posed_joint(&m, &b, &t, "l-hand").y > head.y + 100.0);
        // Squat: the knees come forward and the feet stay flat.
        let t = bone_transforms(&m, &b, pose_by_id("squat").unwrap()).unwrap();
        assert!(posed_joint(&m, &b, &t, "l-shin").z > joint(&b, "l-knee").z + 200.0);
        let skin = m.skin.as_ref().unwrap();
        let foot = skin.bones.iter().position(|x| x.name == "l-foot").unwrap();
        assert!(t[foot].0.angle_between(DQuat::IDENTITY) < 1e-9);
    }

    #[test]
    fn surface_points_take_the_body_weights() {
        let (m, b) = body();
        // A body vertex gets (almost) its own weights back.
        let v = 5000;
        let w = skin_points(&m, &b, &[b.mesh.positions()[v]]).unwrap();
        let mut got = w[0].clone();
        got.retain(|e| e.1 > 1e-4);
        got.sort_by_key(|e| e.0);
        let mut want = m.skin.as_ref().unwrap().weights[v].clone();
        want.sort_by_key(|e| e.0);
        assert_eq!(got.len(), want.len());
        for (a, c) in got.iter().zip(&want) {
            assert_eq!(a.0, c.0);
            assert!((a.1 - c.1).abs() < 1e-4);
        }
    }
}
