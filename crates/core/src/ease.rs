//! Ease factor: how much smaller the flat piece is than the body.
//!
//! A reference triangle is the 3D triangle laid flat and scaled by `s`, so
//! each target flat edge length is `s ×` its 3D length. Pulling the flat
//! piece onto the body then stretches it by roughly `1/s`.
//!
//! ## Around and along the body
//! Latex is usually reduced much more around the body than along it:
//! lengthwise stretch drags at the shoulders and crotch. With a *length
//! relief* `r ∈ [0, 1]` the reduction along the body is `r` times the
//! reduction around it, `s_len = 1 − r (1 − s)`; `r = 1` is the same in
//! both directions. See [`reference_triangles`].
//!
//! `s` is defined per input-mesh triangle so that later milestones (swatch
//! calibration per sheet thickness, local reinforcement, graded ease) only
//! need to produce a vector of values; nothing downstream changes.

use crate::cut::cut_panels;
use crate::error::{Error, Result};
use crate::mesh::Mesh;
use crate::panel::Panel;
use crate::seams::SeamSet;
use glam::{DVec2, DVec3};

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

/// A region of the body with its own ease: within `radius_mm / 2` of
/// `center` the ease is `ease`; between `radius_mm / 2` and `radius_mm` it
/// blends back to the surrounding value with a cosine ramp
/// `w = ½ (1 + cos(π (d − R/2) / (R/2)))`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EaseZone {
    pub center: DVec3,
    pub radius_mm: f64,
    pub ease: f64,
}

impl EaseZone {
    /// Blend weight at distance `d` from the centre.
    pub fn weight(&self, d: f64) -> f64 {
        let half = 0.5 * self.radius_mm;
        if d <= half {
            1.0
        } else if d >= self.radius_mm {
            0.0
        } else {
            0.5 * (1.0 + (std::f64::consts::PI * (d - half) / half).cos())
        }
    }
}

/// Apply ease zones (in order) to `base`, giving one value per mesh
/// triangle (blended at each triangle's centre).
pub fn apply_zones(mesh: &Mesh, base: &Ease, zones: &[EaseZone]) -> Result<Ease> {
    let n = mesh.triangles().len();
    let mut values = match base {
        Ease::Uniform(s) => vec![*s; n],
        Ease::PerTriangle(v) if v.len() == n => v.clone(),
        Ease::PerTriangle(v) => {
            return Err(Error::InvalidEase(format!(
                "{} ease values for {n} triangles",
                v.len()
            )));
        }
    };
    for z in zones {
        if !(z.radius_mm > 0.0 && z.ease > 0.0 && z.ease.is_finite()) {
            return Err(Error::InvalidEase(format!(
                "an ease zone needs a positive radius and ease, got {} mm and {}",
                z.radius_mm, z.ease
            )));
        }
        for (t, s) in mesh.triangles().iter().zip(values.iter_mut()) {
            let c = t
                .iter()
                .map(|&i| mesh.positions()[i as usize])
                .sum::<DVec3>()
                / 3.0;
            let w = z.weight(c.distance(z.center));
            *s += w * (z.ease - *s);
        }
    }
    Ok(Ease::PerTriangle(values))
}

/// Reference (target) flat shapes of a panel's triangles for ARAP.
///
/// Each 3D triangle is laid flat isometrically in its own frame (`e₁` along
/// its first edge, `e₂ = n × e₁`) and scaled by
/// ```text
/// M = s_len d dᵀ + s d⊥ d⊥ᵀ,    s_len = 1 − r (1 − s),
/// ```
/// where `s` is the triangle's ease (around the body), `r` the length
/// relief, and `d` the "along the body" direction projected into the
/// triangle (`d⊥` its in-plane perpendicular). `along[t]` is that 3D
/// direction per local triangle; with `None`, or where it is nearly normal
/// to the triangle (less than 30 % of it in plane), the scaling is uniform
/// in `s`.
pub fn reference_triangles(
    panel: &Panel,
    ease: &[f64],
    relief: f64,
    along: Option<&[DVec3]>,
) -> Vec<[DVec2; 3]> {
    panel
        .triangles
        .iter()
        .enumerate()
        .map(|(t, tri)| {
            let p = tri.map(|i| panel.positions[i as usize]);
            let q = crate::linalg::triangle_frame(p);
            let s = ease[t];
            let s_len = 1.0 - relief * (1.0 - s);
            let dir = along.and_then(|a| {
                let e1 = (p[1] - p[0]).normalize();
                let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
                let e2 = n.cross(e1);
                let d = DVec2::new(a[t].dot(e1), a[t].dot(e2));
                (d.length() > 0.3 * a[t].length()).then(|| d.normalize())
            });
            match dir {
                Some(d) if (s_len - s).abs() > 1e-12 => {
                    let dp = d.perp();
                    q.map(|x| s_len * x.dot(d) * d + s * x.dot(dp) * dp)
                }
                _ => q.map(|x| x * s),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{flatten_garment, testmesh, FlattenOptions, Seam};
    use approx::assert_relative_eq;

    #[test]
    fn zones_blend_smoothly() {
        let m = testmesh::grid(20, 2, 10.0);
        let z = EaseZone {
            center: DVec3::new(0.0, 10.0, 0.0),
            radius_mm: 100.0,
            ease: 0.95,
        };
        assert_eq!(z.weight(10.0), 1.0);
        assert_relative_eq!(z.weight(75.0), 0.5, epsilon = 1e-12);
        assert_eq!(z.weight(150.0), 0.0);
        let Ease::PerTriangle(v) = apply_zones(&m, &Ease::Uniform(0.85), &[z]).unwrap() else {
            panic!()
        };
        // Near the centre: the zone's ease; far away: the base.
        assert_relative_eq!(v[0], 0.95, epsilon = 1e-12);
        assert_relative_eq!(*v.last().unwrap(), 0.85, epsilon = 1e-12);
        assert!(v.iter().all(|&s| (0.85..=0.95).contains(&s)));
        let bad = EaseZone {
            radius_mm: 0.0,
            ..z
        };
        assert!(apply_zones(&m, &Ease::Uniform(0.85), &[bad]).is_err());
    }

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
