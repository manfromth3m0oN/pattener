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
//! - the nipples are replaced by the bust's own curve: a quadratic height
//!   field is fitted to the ring of bust around each nipple and the disc
//!   inside it (radius [`NIPPLE_RADIUS`]) is moved onto it along the
//!   surface normal (plain smoothing there shrinks the bust into a
//!   flattened cone);
//! - the genitals are flattened by Laplacian smoothing,
//!   `p ← p + w λ Δp` (`λ = ½`, `Δp` the umbrella Laplacian: mean of the
//!   neighbours minus the vertex), repeated.
//!
//! The rest of the body is untouched, so measurements and garment geometry
//! elsewhere do not change; the latex bridges the smoothed details anyway.

use glam::DVec3;

const LAMBDA: f64 = 0.5;
const ROUNDS: usize = 150;
const HEAD_ROUNDS: usize = 160;
/// Radius of the patch filled in around each nipple (mm).
pub const NIPPLE_RADIUS: f64 = 32.0;
/// The nipple vertices of the MakeHuman base mesh (left, right). The mesh
/// is the same for every body, and a men's nipple is not where the chest is
/// most forward, so they are fixed rather than searched for.
const NIPPLES: [usize; 2] = [8454, 1782];

/// `positions` (body vertices) with the mannequin smoothing applied.
pub fn mannequin(
    positions: &[DVec3],
    triangles: &[[u32; 3]],
    joints: &[(String, DVec3)],
) -> Vec<DVec3> {
    let joint = |name: &str| joints.iter().find(|j| j.0 == name).map(|j| j.1);
    let (Some(neck), Some(head), Some(pelvis)) = (joint("neck"), joint("head"), joint("pelvis"))
    else {
        return positions.to_vec();
    };
    let p = positions;
    let mut w = vec![0.0f64; p.len()];
    // The head blends in about the underside of the chin: the lowest
    // point of the centre line well in front of the neck. (Measured from
    // the neck joint instead, a men's chin, which sits lower, would be
    // left out.)
    let chin = p
        .iter()
        .filter(|q| q.x.abs() < 1.0 && q.z > neck.z + 100.0 && q.y > neck.y - 40.0 && q.y < head.y)
        .min_by(|a, b| a.y.total_cmp(&b.y))
        .map_or(neck.y + 40.0, |q| q.y);
    let head_w: Vec<f64> = p
        .iter()
        .map(|q| smoothstep(chin - 25.0, chin + 15.0, q.y))
        .collect();
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
    for &v in &NIPPLES {
        if v < p.len() {
            fill_disc(&mut out, triangles, v, NIPPLE_RADIUS);
        }
    }
    // The head: smoothed much harder, melting the face and ears away.
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
    // radial projection does not fold. Melting much longer bunches the
    // vertices towards the neck and leaves the top of the head flat.
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

/// Replace the surface within `radius` of vertex `centre` by a smooth
/// height field. In a frame whose `n` axis is the mean surface normal
/// around the nipple, fit `h(u, v)` (a quadratic, by least squares) to the
/// ring of vertices between `radius` and 1.6 `radius`, then move each disc
/// vertex along `n` onto it: fully within 0.6 `radius`, fading to nothing
/// at `radius`, so the patch meets the bust smoothly.
fn fill_disc(p: &mut [DVec3], triangles: &[[u32; 3]], centre: usize, radius: f64) {
    let c = p[centre];
    let (r0, r1) = (0.6 * radius, 1.6 * radius);
    let mut n = DVec3::ZERO;
    for t in triangles {
        let [a, b, d] = t.map(|i| p[i as usize]);
        if [a, b, d].iter().all(|q| q.distance(c) < r1) {
            n += (b - a).cross(d - a);
        }
    }
    let Some(n) = n.try_normalize() else { return };
    let u = n.any_orthonormal_vector();
    let v = n.cross(u);
    let near: Vec<usize> = (0..p.len())
        .filter(|&i| p[i].distance(c) < r1 && (p[i] - c).dot(n) > -radius)
        .collect();
    let basis = |q: DVec3| {
        let d = q - c;
        let (x, y) = (d.dot(u) / radius, d.dot(v) / radius);
        [1.0, x, y, x * x, x * y, y * y]
    };
    // Normal equations for the least-squares fit on the ring.
    let mut a = vec![vec![0.0; 6]; 6];
    let mut b = vec![DVec3::ZERO; 6];
    let mut count = 0;
    for &i in &near {
        if p[i].distance(c) >= radius {
            let f = basis(p[i]);
            let h = (p[i] - c).dot(n);
            for k in 0..6 {
                for l in 0..6 {
                    a[k][l] += f[k] * f[l];
                }
                b[k] += DVec3::splat(f[k] * h);
            }
            count += 1;
        }
    }
    if count < 12 {
        return;
    }
    let Some(coef) = solve(a, b) else { return };
    for &i in &near {
        let d = p[i].distance(c);
        if d < radius {
            let w = 1.0 - smoothstep(r0, radius, d);
            let f = basis(p[i]);
            let h: f64 = (0..6).map(|k| coef[k].x * f[k]).sum();
            let now = (p[i] - c).dot(n);
            p[i] += n * (w * (h - now));
        }
    }
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Dense Gaussian elimination with partial pivoting, three right-hand
/// sides at once.
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<DVec3>) -> Option<Vec<DVec3>> {
    let n = b.len();
    for c in 0..n {
        let piv = (c..n).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[piv][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, piv);
        b.swap(c, piv);
        for i in c + 1..n {
            let f = a[i][c] / a[c][c];
            if f != 0.0 {
                for k in c..n {
                    a[i][k] -= f * a[c][k];
                }
                let bc = b[c];
                b[i] -= f * bc;
            }
        }
    }
    for c in (0..n).rev() {
        for k in c + 1..n {
            let bk = b[k];
            b[c] -= a[c][k] * bk;
        }
        b[c] /= a[c][c];
    }
    Some(b)
}

#[cfg(test)]
mod tests {
    use crate::human::{fit, test_model, Measurements, Measurer, ShapeOptions};

    #[test]
    fn smooths_only_the_details_and_stays_symmetric() {
        let m = test_model();
        let me = Measurer::new(&m).unwrap();
        for body_type in [0.0, 1.0] {
            let shape = |realistic| ShapeOptions {
                body_type,
                realistic,
                ..Default::default()
            };
            let dummy = fit(&m, &me, &Measurements::new(), &shape(false)).unwrap();
            let real = fit(&m, &me, &Measurements::new(), &shape(true)).unwrap();
            let joint = |n: &str| real.joints.iter().find(|j| j.0 == n).unwrap().1;
            let (neck, pelvis, knee) = (joint("neck"), joint("pelvis"), joint("l-knee"));
            let (a, b) = (dummy.mesh.positions(), real.mesh.positions());
            let moved = |i: usize| a[i].distance(b[i]);
            // The face, chin included, moves (it is smoothed into a form)...
            let nose = (0..b.len())
                .filter(|&i| b[i].y > neck.y + 60.0)
                .max_by(|&i, &j| b[i].z.total_cmp(&b[j].z))
                .unwrap();
            assert!(moved(nose) > 2.0, "{body_type}: nose {}", moved(nose));
            let chin = (0..b.len())
                .filter(|&i| b[i].x == 0.0 && b[i].y > neck.y - 40.0 && b[i].y < neck.y + 60.0)
                .max_by(|&i, &j| b[i].z.total_cmp(&b[j].z))
                .unwrap();
            assert!(moved(chin) > 2.0, "{body_type}: chin {}", moved(chin));
            // ...and the head stays round: its crown is still at the top.
            let top = |q: &[glam::DVec3]| q.iter().map(|v| v.y).fold(f64::MIN, f64::max);
            assert!(top(a) > top(b) - 3.0, "{body_type}: {} {}", top(a), top(b));
            // The nipples are smoothed into the bust, which keeps its shape.
            for v in super::NIPPLES {
                let most = (0..b.len())
                    .filter(|&i| b[i].distance(b[v]) < super::NIPPLE_RADIUS)
                    .map(moved)
                    .fold(0.0, f64::max);
                assert!(
                    (0.5..8.0).contains(&most),
                    "{body_type}: nipple {v} moved {most}"
                );
            }
            // The waist, legs and arms do not move.
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
}
