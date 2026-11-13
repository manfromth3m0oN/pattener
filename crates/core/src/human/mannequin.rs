//! A dressmaker's-mannequin version of the fitted body.
//!
//! The body model is anatomically detailed. For designing garments a
//! mannequin is easier to work with (and to show), so by default the
//! details a dress form does not have are smoothed away. Each region has a
//! weight `w ∈ [0, 1]` that fades out at its edge:
//!
//! - the head becomes a smooth form: it is smoothed hard with the same
//!   Laplacian (which melts the face, ears and their hollows away), then
//!   each vertex moves a fraction `w` onto the ellipsoid bounding the
//!   original head, along the ray from its centre;
//! - the nipples and genitals are flattened by Laplacian smoothing,
//!   `p ← p + w λ Δp` (`λ = ½`, `Δp` the umbrella Laplacian: mean of the
//!   neighbours minus the vertex), repeated.
//!
//! The rest of the body is untouched, so measurements and garment geometry
//! elsewhere do not change; the latex bridges the smoothed details anyway.

use glam::DVec3;

const LAMBDA: f64 = 0.5;
const ROUNDS: usize = 150;
const HEAD_ROUNDS: usize = 300;

/// `positions` (body vertices) with the mannequin smoothing applied.
pub fn mannequin(
    positions: &[DVec3],
    triangles: &[[u32; 3]],
    joints: &[(String, DVec3)],
) -> Vec<DVec3> {
    let joint = |name: &str| joints.iter().find(|j| j.0 == name).map(|j| j.1);
    let (Some(neck), Some(chest_lo), Some(chest_hi), Some(pelvis)) = (
        joint("neck"),
        joint("spine-2"),
        joint("spine-1"),
        joint("pelvis"),
    ) else {
        return positions.to_vec();
    };
    let smoothstep = |a: f64, b: f64, x: f64| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let p = positions;
    let mut w = vec![0.0f64; p.len()];
    let head_w: Vec<f64> = p
        .iter()
        .map(|q| smoothstep(neck.y + 15.0, neck.y + 55.0, q.y))
        .collect();
    // The nipples: the most forward point of the chest on each side.
    for side in [1.0, -1.0] {
        let apex = p
            .iter()
            .filter(|q| {
                let x = side * q.x;
                (25.0..180.0).contains(&x) && q.y > chest_lo.y && q.y < chest_hi.y + 30.0
            })
            .max_by(|a, b| a.z.total_cmp(&b.z))
            .copied();
        if let Some(a) = apex {
            for (i, q) in p.iter().enumerate() {
                w[i] = w[i].max(1.0 - smoothstep(28.0, 50.0, q.distance(a)));
            }
        }
    }
    // The genitals: in front of and around the lowest point between the legs.
    let crotch = p
        .iter()
        .filter(|q| q.x.abs() < 10.0 && q.y < pelvis.y)
        .min_by(|a, b| a.y.total_cmp(&b.y))
        .copied();
    if let Some(c) = crotch {
        for (i, q) in p.iter().enumerate() {
            if q.z > c.z - 25.0 {
                let d = DVec3::new(q.x * 1.4, q.y - c.y, q.z - c.z).length();
                w[i] = w[i].max(1.0 - smoothstep(55.0, 90.0, d));
            }
        }
    }
    // Neighbours of each vertex.
    let mut nb: Vec<Vec<u32>> = vec![Vec::new(); p.len()];
    for t in triangles {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if !nb[a as usize].contains(&b) {
                nb[a as usize].push(b);
            }
            if !nb[b as usize].contains(&a) {
                nb[b as usize].push(a);
            }
        }
    }
    for list in &mut nb {
        list.sort_unstable();
    }
    let active: Vec<usize> = (0..p.len())
        .filter(|&i| w[i] > 0.0 && !nb[i].is_empty())
        .collect();
    let mut out = p.to_vec();
    for _ in 0..ROUNDS {
        let q = out.clone();
        for &i in &active {
            let c = nb[i].iter().map(|&j| q[j as usize]).sum::<DVec3>() / nb[i].len() as f64;
            out[i] = q[i] + w[i] * LAMBDA * (c - q[i]);
        }
    }
    // The head: smoothed much harder (melting the face and ears away),
    // then scaled back about its centre to its original size, since
    // smoothing a closed shape also shrinks it.
    let head: Vec<usize> = (0..p.len())
        .filter(|&i| head_w[i] > 0.0 && !nb[i].is_empty())
        .collect();
    for _ in 0..HEAD_ROUNDS {
        let q = out.clone();
        for &i in &head {
            let c = nb[i].iter().map(|&j| q[j as usize]).sum::<DVec3>() / nb[i].len() as f64;
            out[i] = q[i] + head_w[i] * LAMBDA * (c - q[i]);
        }
    }
    // Then onto the ellipsoid bounding the original head: after the
    // melting every direction from its centre crosses the head once, so the
    // radial projection does not fold.
    let core: Vec<usize> = head.iter().copied().filter(|&i| head_w[i] >= 1.0).collect();
    if !core.is_empty() {
        let lo = core.iter().fold(p[core[0]], |m, &i| m.min(p[i]));
        let hi = core.iter().fold(p[core[0]], |m, &i| m.max(p[i]));
        let (c, r) = (0.5 * (lo + hi), (0.5 * (hi - lo)).max(DVec3::splat(1.0)));
        for &i in &head {
            let d = out[i] - c;
            let k = (d / r).length();
            if k > 1e-9 {
                out[i] = out[i].lerp(c + d / k, head_w[i]);
            }
        }
    }
    // Keep the centre line exactly on it (the smoothing is symmetric up to
    // rounding).
    for (o, q) in out.iter_mut().zip(p) {
        if q.x == 0.0 {
            o.x = 0.0;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::human::{fit, test_model, Measurements, Measurer, ShapeOptions};

    #[test]
    fn smooths_only_the_details_and_stays_symmetric() {
        let m = test_model();
        let me = Measurer::new(&m).unwrap();
        let shape = |realistic| ShapeOptions {
            realistic,
            ..Default::default()
        };
        let dummy = fit(&m, &me, &Measurements::new(), &shape(false)).unwrap();
        let real = fit(&m, &me, &Measurements::new(), &shape(true)).unwrap();
        let joint = |n: &str| real.joints.iter().find(|j| j.0 == n).unwrap().1;
        let (neck, pelvis, knee) = (joint("neck"), joint("pelvis"), joint("l-knee"));
        let (a, b) = (dummy.mesh.positions(), real.mesh.positions());
        let moved = |i: usize| a[i].distance(b[i]);
        // The face moves (it is smoothed into a form)...
        let nose = (0..b.len())
            .filter(|&i| b[i].y > neck.y + 60.0)
            .max_by(|&i, &j| b[i].z.total_cmp(&b[j].z))
            .unwrap();
        assert!(moved(nose) > 2.0, "{}", moved(nose));
        // ...but the waist, legs and arms do not.
        for (i, p) in b.iter().enumerate() {
            let far = (p.y < knee.y + 100.0)
                || (p.y > pelvis.y + 60.0 && p.y < neck.y - 250.0 && p.z < 0.0);
            if far {
                assert!(moved(i) < 1e-9, "vertex {i} at {p} moved {}", moved(i));
            }
        }
        // Still exactly mirror-symmetric, so garments build symmetric.
        assert!(crate::garment::body_mirror(&dummy.mesh).is_some());
    }
}
