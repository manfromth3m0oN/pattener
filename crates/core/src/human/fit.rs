//! Fitting the body model to tape measurements.
//!
//! ## Parameters
//! The body is `base + Σ w_k Δ_k` (see [`model`](super::model)). The
//! weights come from two groups.
//!
//! **Chosen by the user** ([`ShapeOptions`]): body type `g` (0 feminine to
//! 1 masculine, blending the two base shapes and every gendered target),
//! muscle, cup, belly and buttocks. A tape cannot tell these apart, so they
//! are set by eye.
//!
//! **Solved for** (`p_k ∈ [−LIMIT, LIMIT]`): overall height and weight, and
//! one parameter per local shape target (waist girth, thigh girth, upper
//! leg length, …). Each drives a pair of targets, `max(p, 0)` on the
//! "increase" one and `max(−p, 0)` on the "decrease" one.
//!
//! ## Solver
//! With measured values `tᵢ` and the model's own measurements `mᵢ(p)` (see
//! [`measure`](super::measure)), minimise
//! ```text
//! E(p) = Σᵢ ((mᵢ(p) − tᵢ) / tᵢ)² + λ Σ_k p_k²
//! ```
//! by Levenberg–Marquardt: with residuals `r`, Jacobian `J` (forward
//! differences) and damping `μ`, solve `(JᵀJ + (λ + μ) I) δ = −(Jᵀr + λp)`,
//! accept the step if `E` drops (then relax `μ`), otherwise raise `μ`. The
//! small `λ` picks the least extreme shape among those that fit, which
//! matters because there are more parameters than measurements; its bias
//! on the fitted measurements is far below a millimetre.
//!
//! Only the measurements supplied are fitted; the rest are predicted by the
//! model from overall height and weight.

use super::measure::{MeasureKind, Measurer};
use super::model::{BodyModel, Shape};
use crate::error::{Error, Result};
use crate::mesh::Mesh;
use glam::DVec3;
use std::collections::BTreeMap;

/// Shape choices a tape measure cannot determine.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ShapeOptions {
    /// 0 = feminine base shape, 1 = masculine.
    pub body_type: f64,
    /// −1 (soft) to 1 (muscular).
    pub muscle: f64,
    /// −1 (small) to 1 (large); scaled by `1 − body_type`.
    pub cup: f64,
    /// −1 (flat) to 1 (round).
    pub belly: f64,
    /// −1 (flat) to 1 (full).
    pub buttocks: f64,
    /// Keep the body's anatomical detail (face, nipples, genitals) instead
    /// of smoothing it away like a dressmaker's mannequin (see
    /// [`mannequin`](crate::human::mannequin)).
    pub realistic: bool,
}

impl Default for ShapeOptions {
    fn default() -> Self {
        Self {
            body_type: 0.0,
            muscle: 0.0,
            cup: 0.0,
            belly: 0.0,
            buttocks: 0.0,
            realistic: false,
        }
    }
}

/// Measurements in millimetres, by name. Any subset may be given.
pub type Measurements = BTreeMap<MeasureKind, f64>;

/// One line of the fit report.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Measured {
    pub kind: MeasureKind,
    /// What was asked for, if this measurement was supplied.
    pub requested: Option<f64>,
    /// What the fitted body measures (mm).
    pub value: f64,
}

/// A body fitted to measurements.
#[derive(Debug, Clone)]
pub struct HumanBody {
    pub mesh: Mesh,
    /// Joint positions by name (mm).
    pub joints: Vec<(String, DVec3)>,
    /// Every measurement of the fitted body.
    pub measured: Vec<Measured>,
    /// Where each girth is taken: the cross-section loop at its landmark.
    pub tapes: Vec<(MeasureKind, Vec<DVec3>)>,
    /// Solved parameters by name, for reference.
    pub parameters: Vec<(String, f64)>,
    pub iterations: usize,
}

/// Bound on every solved parameter (1 = the target's designed range).
const LIMIT: f64 = 2.5;
const LAMBDA: f64 = 1e-5;
const MAX_ITERATIONS: usize = 30;
const STEP: f64 = 0.05;

/// Local shape targets solved for.
const LOCAL: [&str; 17] = [
    "neck-circ",
    "upperarm-circ",
    "upperarm-length",
    "lowerarm-length",
    "wrist-circ",
    "bust-circ",
    "underbust-circ",
    "waist-circ",
    "napetowaist-dist",
    "waisttohip-dist",
    "hips-circ",
    "upperleg-height",
    "thigh-circ",
    "lowerleg-height",
    "calf-circ",
    "ankle-circ",
    "knee-circ",
];

/// One solved parameter: `(target, factor)` lists for its positive and
/// negative side.
struct Param {
    name: String,
    incr: Vec<(usize, f64)>,
    decr: Vec<(usize, f64)>,
}

struct Problem<'a> {
    model: &'a BodyModel,
    measurer: &'a Measurer,
    fixed: Vec<(usize, f64)>,
    params: Vec<Param>,
}

impl Problem<'_> {
    fn shape(&self, p: &[f64]) -> Shape {
        let mut w = self.fixed.clone();
        for (param, &v) in self.params.iter().zip(p) {
            let side = if v >= 0.0 { &param.incr } else { &param.decr };
            w.extend(side.iter().map(|&(t, f)| (t, f * v.abs())));
        }
        self.model.morph(&w)
    }

    fn residuals(&self, p: &[f64], want: &[(MeasureKind, f64)]) -> Result<Vec<f64>> {
        let s = self.shape(p);
        want.iter()
            .map(|&(k, t)| Ok((self.measurer.measure(self.model, &s, k)? - t) / t))
            .collect()
    }
}

/// Fit the model to `measurements` and return the body.
pub fn fit(
    model: &BodyModel,
    measurer: &Measurer,
    measurements: &Measurements,
    options: &ShapeOptions,
) -> Result<HumanBody> {
    let o = options;
    for (name, v, lo) in [
        ("body_type", o.body_type, 0.0),
        ("muscle", o.muscle, -1.0),
        ("cup", o.cup, -1.0),
        ("belly", o.belly, -1.0),
        ("buttocks", o.buttocks, -1.0),
    ] {
        if !(lo..=1.0).contains(&v) {
            return Err(Error::Body(format!(
                "{name} must be between {lo} and 1, got {v}"
            )));
        }
    }
    let want: Vec<(MeasureKind, f64)> = measurements.iter().map(|(&k, &v)| (k, v)).collect();
    for &(k, v) in &want {
        if !(v > 0.0 && v.is_finite()) {
            return Err(Error::Body(format!(
                "{} must be a positive length in mm, got {v}",
                k.name()
            )));
        }
    }

    let t = |n: &str| model.target(n);
    let (g, f) = (o.body_type, 1.0 - o.body_type);
    // A user slider in [−1, 1] driving an incr/decr pair, scaled by `k`.
    let pair = |name: &str, v: f64, k: f64| -> Result<(usize, f64)> {
        Ok((
            t(&format!(
                "{name}-{}",
                if v >= 0.0 { "incr" } else { "decr" }
            ))?,
            k * v.abs(),
        ))
    };
    let fixed = vec![
        (t("female")?, f),
        (t("male")?, g),
        pair("f-muscle", o.muscle, f)?,
        pair("m-muscle", o.muscle, g)?,
        pair("cup", o.cup, f)?,
        pair("belly", o.belly, 1.0)?,
        pair("buttocks", o.buttocks, 1.0)?,
    ];
    let mut params = Vec::new();
    for name in ["height", "weight"] {
        params.push(Param {
            name: name.to_string(),
            incr: vec![
                (t(&format!("f-{name}-incr"))?, f),
                (t(&format!("m-{name}-incr"))?, g),
            ],
            decr: vec![
                (t(&format!("f-{name}-decr"))?, f),
                (t(&format!("m-{name}-decr"))?, g),
            ],
        });
    }
    for name in LOCAL {
        params.push(Param {
            name: name.to_string(),
            incr: vec![(t(&format!("measure-{name}-incr"))?, 1.0)],
            decr: vec![(t(&format!("measure-{name}-decr"))?, 1.0)],
        });
    }
    let problem = Problem {
        model,
        measurer,
        fixed,
        params,
    };

    let n = problem.params.len();
    let mut p = vec![0.0; n];
    let mut iterations = 0;
    if !want.is_empty() {
        let cost = |r: &[f64], p: &[f64]| {
            r.iter().map(|x| x * x).sum::<f64>() + LAMBDA * p.iter().map(|x| x * x).sum::<f64>()
        };
        let mut r = problem.residuals(&p, &want)?;
        let mut e = cost(&r, &p);
        let mut mu = 1e-3;
        while iterations < MAX_ITERATIONS {
            iterations += 1;
            // Jacobian by forward differences (stepping away from a bound).
            let mut jac = vec![vec![0.0; n]; want.len()];
            for k in 0..n {
                let h = if p[k] + STEP > LIMIT { -STEP } else { STEP };
                let mut q = p.clone();
                q[k] += h;
                let rq = problem.residuals(&q, &want)?;
                for i in 0..want.len() {
                    jac[i][k] = (rq[i] - r[i]) / h;
                }
            }
            let mut improved = false;
            for _ in 0..8 {
                // (JᵀJ + (λ + μ) I) δ = −(Jᵀr + λp)
                let mut a = vec![vec![0.0; n]; n];
                let mut b = vec![0.0; n];
                for i in 0..want.len() {
                    for k in 0..n {
                        b[k] -= jac[i][k] * r[i];
                        for l in 0..n {
                            a[k][l] += jac[i][k] * jac[i][l];
                        }
                    }
                }
                for k in 0..n {
                    a[k][k] += LAMBDA + mu;
                    b[k] -= LAMBDA * p[k];
                }
                let delta = solve_dense(a, b)?;
                let q: Vec<f64> = p
                    .iter()
                    .zip(&delta)
                    .map(|(x, d)| (x + d).clamp(-LIMIT, LIMIT))
                    .collect();
                let rq = problem.residuals(&q, &want)?;
                let eq = cost(&rq, &q);
                if eq < e {
                    let done = e - eq <= 1e-12 * e.max(1e-12);
                    (p, r, e) = (q, rq, eq);
                    mu = (mu / 3.0).max(1e-9);
                    improved = !done;
                    break;
                }
                mu *= 4.0;
            }
            if !improved {
                break;
            }
        }
    }

    let shape = problem.shape(&p);
    let joints: Vec<(String, DVec3)> = model
        .joint_names
        .iter()
        .enumerate()
        .map(|(j, name)| (name.clone(), shape.joint(model, j)))
        .collect();
    let body = if options.realistic {
        shape.body(model).to_vec()
    } else {
        super::mannequin::mannequin(shape.body(model), &model.triangles, &joints)
    };
    let mesh = Mesh::new(body, model.triangles.clone())?;
    let measured = measurer
        .measure_all(model, &shape)?
        .into_iter()
        .map(|(kind, value)| Measured {
            kind,
            requested: measurements.get(&kind).copied(),
            value,
        })
        .collect();
    let tapes = MeasureKind::ALL
        .iter()
        .filter_map(|&k| Some((k, measurer.tape(model, &shape, k)?)))
        .collect();
    Ok(HumanBody {
        mesh,
        joints,
        measured,
        tapes,
        parameters: problem
            .params
            .iter()
            .zip(&p)
            .map(|(q, &v)| (q.name.clone(), v))
            .collect(),
        iterations,
    })
}

/// Solve a small dense system by Gaussian elimination with partial pivoting.
fn solve_dense(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Result<Vec<f64>> {
    let n = b.len();
    for c in 0..n {
        let piv = (c..n)
            .max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))
            .unwrap();
        if a[piv][c].abs() < 1e-300 {
            return Err(Error::Solve("singular system in body fit".into()));
        }
        a.swap(c, piv);
        b.swap(c, piv);
        for i in c + 1..n {
            let f = a[i][c] / a[c][c];
            if f != 0.0 {
                for k in c..n {
                    a[i][k] -= f * a[c][k];
                }
                b[i] -= f * b[c];
            }
        }
    }
    for c in (0..n).rev() {
        for k in c + 1..n {
            b[c] -= a[c][k] * b[k];
        }
        b[c] /= a[c][c];
    }
    Ok(b)
}
