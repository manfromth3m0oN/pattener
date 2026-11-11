//! The "along the body" direction: along the limbs and up the torso.
//!
//! Latex is reduced differently around and along the body (see
//! [`reference_triangles`](crate::ease::reference_triangles)), so every
//! point of a garment needs a lengthwise direction. It comes from the
//! skeleton: each bone (a segment between two joints) contributes its
//! direction `u` with weight `w = 1 / (d⁴ + ε)`, `d` the distance from the
//! point to the bone. The directions are blended as orientation tensors,
//! `A = Σ w u uᵀ`, and the result is the principal eigenvector of `A`
//! (power iteration). Tensors ignore which way each bone points and blend
//! smoothly where bones meet (hips, shoulders).

use crate::error::{Error, Result};
use glam::{DMat3, DVec3};

/// Bones as joint-name pairs.
const BONES: [(&str, &str); 16] = [
    ("pelvis", "spine-4"),
    ("spine-4", "spine-3"),
    ("spine-3", "spine-2"),
    ("spine-2", "spine-1"),
    ("spine-1", "neck"),
    ("neck", "head"),
    ("l-upper-leg", "l-knee"),
    ("l-knee", "l-ankle"),
    ("r-upper-leg", "r-knee"),
    ("r-knee", "r-ankle"),
    ("l-clavicle", "l-shoulder"),
    ("l-shoulder", "l-elbow"),
    ("l-elbow", "l-hand"),
    ("r-clavicle", "r-shoulder"),
    ("r-shoulder", "r-elbow"),
    ("r-elbow", "r-hand"),
];

/// Lengthwise direction (unit) at each of `points`, from named joint
/// positions (as in [`HumanBody::joints`](super::HumanBody)).
pub fn along_body(joints: &[(String, DVec3)], points: &[DVec3]) -> Result<Vec<DVec3>> {
    let joint = |n: &str| {
        joints
            .iter()
            .find(|(name, _)| name == n)
            .map(|(_, p)| *p)
            .ok_or_else(|| Error::Body(format!("missing joint '{n}'")))
    };
    let bones: Vec<(DVec3, DVec3)> = BONES
        .iter()
        .map(|(a, b)| Ok((joint(a)?, joint(b)?)))
        .collect::<Result<_>>()?;
    Ok(points
        .iter()
        .map(|&p| {
            let mut a = DMat3::ZERO;
            let mut nearest = (f64::INFINITY, DVec3::Y);
            for &(s, e) in &bones {
                let v = e - s;
                let t = ((p - s).dot(v) / v.length_squared()).clamp(0.0, 1.0);
                let d = p.distance(s + t * v);
                let u = v.normalize();
                // ε keeps weights finite on the bone; 10 mm is far below
                // any point's distance to its bones.
                let w = 1.0 / (d.powi(4) + 1e4);
                a += w * DMat3::from_cols(u * u.x, u * u.y, u * u.z);
                if d < nearest.0 {
                    nearest = (d, u);
                }
            }
            let mut x = nearest.1;
            for _ in 0..30 {
                x = (a * x).normalize_or(x);
            }
            x
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::human::{fit, test_model, Measurements, Measurer, ShapeOptions};

    #[test]
    fn follows_limbs_and_torso() {
        let m = test_model();
        let me = Measurer::new(&m).unwrap();
        let b = fit(&m, &me, &Measurements::new(), &ShapeOptions::default()).unwrap();
        let j = |n: &str| b.joints.iter().find(|(name, _)| name == n).unwrap().1;
        let bone = |a: &str, c: &str| (j(c) - j(a)).normalize();
        // Mid-thigh, a little in front of the bone: along the thigh.
        let thigh = j("l-upper-leg").lerp(j("l-knee"), 0.5) + DVec3::new(0.0, 0.0, 60.0);
        // Mid forearm, below the bone: along the forearm.
        let forearm = j("l-elbow").lerp(j("l-hand"), 0.5) - DVec3::new(0.0, 30.0, 0.0);
        // Front of the waist: up the body.
        let waist = j("spine-3") + DVec3::new(0.0, 0.0, 100.0);
        let d = along_body(&b.joints, &[thigh, forearm, waist]).unwrap();
        assert!(
            d[0].dot(bone("l-upper-leg", "l-knee")).abs() > 0.95,
            "{}",
            d[0]
        );
        assert!(d[1].dot(bone("l-elbow", "l-hand")).abs() > 0.95, "{}", d[1]);
        assert!(d[2].y.abs() > 0.95, "{}", d[2]);
        assert!(d.iter().all(|v| (v.length() - 1.0).abs() < 1e-9));
    }
}
