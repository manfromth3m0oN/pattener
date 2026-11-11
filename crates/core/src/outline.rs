//! Clean flat outlines: smoothing and exact mirror symmetry.
//!
//! **Smoothing.** A flat piece's outline joins the vertices where curves
//! were cut into the body mesh, so it carries small kinks from the mesh and
//! from flattening. Each stretch of outline between two *corners* (where
//! the neighbouring piece or seam changes, or the outline turns sharply) is
//! smoothed with a Gaussian of standard deviation `σ` in arc length:
//!
//! `p̃ᵢ = Σⱼ g(sᵢ − sⱼ) ℓⱼ pⱼ / Σⱼ g(sᵢ − sⱼ) ℓⱼ`, `g(d) = exp(−d²/2σ²)`,
//!
//! where `ℓⱼ` is the outline length belonging to vertex `j`. The ends are
//! kept fixed by point-reflecting the stretch through each end
//! (`p' = 2 p₀ − p`), so a straight stretch stays straight and corners stay
//! sharp. Plain Gaussian smoothing pulls a curve of radius `R` in by about
//! `σ²/2R`; *twicing*, `2G(p) − G(G(p))`, cancels that to leading order
//! while still removing wiggles much shorter than `σ`. Points move only
//! across the outline (the displacement is projected onto the normal), so
//! they do not slide along it. The inside of the piece is then flattened again (ARAP) with its
//! outline held at the smoothed positions.
//!
//! **Mirroring.** On an exactly symmetric garment (see
//! [`garment::build`](crate::garment::build)) each piece has a partner
//! whose triangles are the mirror images of its own. The partner's layout
//! is replaced by the reflection `(x, y) → (−x, y)` of the piece's, so the
//! two are exact mirror images. A piece that is its own mirror image (it
//! straddles the centre line) is made exactly symmetric: with `F` the best
//! reflection across a line through its centroid, mapping each vertex's
//! mirror partner onto it,
//!
//! `uᵢ ← ½ (uᵢ + F(u_{m(i)}))`,
//!
//! which satisfies `F(u_{m(i)}) = uᵢ` exactly. The best line's angle `φ`
//! maximises `Σ aᵢ · Ref_φ bᵢ` with `aᵢ = uᵢ − c`, `bᵢ = u_{m(i)} − c`,
//! giving `tan 2φ = Σ(aₓb_y + a_ybₓ) / Σ(aₓbₓ − a_yb_y)`.

use crate::flatten::FlatPanel;
use crate::mesh::Mesh;
use glam::DVec2;
use std::collections::HashMap;

/// Outline turns sharper than this (radians, measured `2σ` either side)
/// are kept as corners.
const CORNER_ANGLE: f64 = 1.0;

/// Smoothed outline positions for a piece with scale `sigma_mm` (see the
/// module docs): `fp.uv` with its outline vertices moved. `nb[k]`
/// identifies what lies across boundary edge `k` (seam and piece); the
/// outline keeps a corner wherever it changes. The caller re-solves the
/// inside of the piece with the outline held there.
pub fn smoothed_outline(
    fp: &FlatPanel,
    nb: &[(Option<usize>, usize)],
    sigma_mm: f64,
) -> Option<Vec<DVec2>> {
    if sigma_mm <= 0.0 {
        return None;
    }
    smooth_piece(fp, nb, sigma_mm)
}

fn smooth_piece(fp: &FlatPanel, nb: &[(Option<usize>, usize)], sigma: f64) -> Option<Vec<DVec2>> {
    let b = &fp.panel.boundary;
    let n = b.len();
    if n < 4 {
        return None;
    }
    let loop_v: Vec<u32> = b.iter().map(|e| e.from).collect();
    let uv0 = fp.uv.clone();
    let pos = |k: usize| uv0[loop_v[k % n] as usize];
    // Arc length at each outline vertex.
    let mut s = vec![0.0; n + 1];
    for k in 0..n {
        s[k + 1] = s[k] + pos(k).distance(pos(k + 1));
    }
    let total = s[n];
    // Corners: a change of neighbour, or a sharp turn.
    let point_at = |k: usize, d: f64| -> DVec2 {
        // Point at arc distance d (±) from vertex k along the loop.
        let target = (s[k] + d).rem_euclid(total);
        let j = s.partition_point(|&x| x <= target).clamp(1, n) - 1;
        let t = ((target - s[j]) / (s[j + 1] - s[j]).max(1e-12)).clamp(0.0, 1.0);
        pos(j).lerp(pos(j + 1), t)
    };
    let mut corner: Vec<bool> = (0..n).map(|k| nb[(k + n - 1) % n] != nb[k]).collect();
    for (k, c) in corner.iter_mut().enumerate() {
        let (a, o, z) = (point_at(k, -2.0 * sigma), pos(k), point_at(k, 2.0 * sigma));
        let (u, w) = (o - a, z - o);
        if u.length() > 1e-9 && w.length() > 1e-9 && u.angle_to(w).abs() > CORNER_ANGLE {
            *c = true;
        }
    }
    // Stretches between corners (or the whole loop when there are none).
    let starts: Vec<usize> = (0..n).filter(|&k| corner[k]).collect();
    let mut target = uv0.clone();
    let g = |d: f64| (-(d * d) / (2.0 * sigma * sigma)).exp();
    let smooth_run = |idx: &[usize], closed: bool, target: &mut Vec<DVec2>| {
        let m = idx.len();
        let p: Vec<DVec2> = idx.iter().map(|&k| pos(k)).collect();
        let mut a = vec![0.0; m];
        for i in 1..m {
            a[i] = a[i - 1] + p[i].distance(p[i - 1]);
        }
        let len = if closed {
            a[m - 1] + p[m - 1].distance(p[0])
        } else {
            a[m - 1]
        };
        let weight = |i: usize| -> f64 {
            let prev = if i > 0 {
                a[i] - a[i - 1]
            } else if closed {
                len - a[m - 1]
            } else {
                0.0
            };
            let next = if i + 1 < m {
                a[i + 1] - a[i]
            } else if closed {
                len - a[m - 1]
            } else {
                0.0
            };
            0.5 * (prev + next) + 1e-9
        };
        // One Gaussian pass over positions `q` (ends kept by reflection).
        let blur = |q: &[DVec2]| -> Vec<DVec2> {
            let mut out = q.to_vec();
            let interior = if closed { 0..m } else { 1..m.saturating_sub(1) };
            for i in interior {
                let (mut sum, mut wsum) = (DVec2::ZERO, 0.0);
                for j in 0..m {
                    let mut add = |d: f64, x: DVec2| {
                        if d.abs() < 4.0 * sigma {
                            let w = g(d) * weight(j);
                            sum += w * x;
                            wsum += w;
                        }
                    };
                    if closed {
                        let d = (a[j] - a[i] + 0.5 * len).rem_euclid(len) - 0.5 * len;
                        add(d, q[j]);
                    } else {
                        add(a[j] - a[i], q[j]);
                        // Reflections through both ends keep them fixed.
                        if j > 0 {
                            add(-a[j] - a[i], 2.0 * q[0] - q[j]);
                        }
                        if j + 1 < m {
                            add(2.0 * len - a[j] - a[i], 2.0 * q[m - 1] - q[j]);
                        }
                    }
                }
                out[i] = sum / wsum;
            }
            out
        };
        // Twicing, 2G(p) − G(G(p)), cancels the shrinking of curves.
        let g1 = blur(&p);
        let g2 = blur(&g1);
        for i in 0..m {
            let at_end = !closed && (i == 0 || i + 1 == m);
            if at_end {
                continue;
            }
            // Move only across the outline, never along it.
            let (prev, next) = (p[(i + m - 1) % m], p[(i + 1) % m]);
            let normal = (next - prev).perp().normalize_or_zero();
            let d = 2.0 * g1[i] - g2[i] - p[i];
            target[loop_v[idx[i]] as usize] = p[i] + d.dot(normal) * normal;
        }
    };
    if starts.is_empty() {
        let idx: Vec<usize> = (0..n).collect();
        smooth_run(&idx, true, &mut target);
    } else {
        for (r, &a) in starts.iter().enumerate() {
            let z = starts[(r + 1) % starts.len()];
            let mut idx = vec![a];
            let mut k = a;
            loop {
                k = (k + 1) % n;
                idx.push(k);
                if k == z {
                    break;
                }
            }
            smooth_run(&idx, false, &mut target);
        }
    }
    Some(target)
}

/// For each piece, its mirror-image piece and, per local vertex, the
/// partner's matching local vertex; `None` where a piece has no clean
/// partner.
pub fn mirror_partners(
    mesh: &Mesh,
    mirror: &[u32],
    flat: &[FlatPanel],
) -> Vec<Option<(usize, Vec<u32>)>> {
    let key = |t: [u32; 3]| {
        let mut k = t;
        k.sort_unstable();
        k
    };
    // Mesh triangle → (piece, local triangle).
    let mut owner: HashMap<[u32; 3], (usize, usize)> = HashMap::new();
    for (pi, fp) in flat.iter().enumerate() {
        for (lt, &f) in fp.panel.face_origin.iter().enumerate() {
            owner.insert(key(mesh.triangles()[f as usize]), (pi, lt));
        }
    }
    flat.iter()
        .map(|fp| {
            let mut partner: Option<usize> = None;
            let mut map = vec![u32::MAX; fp.uv.len()];
            for (lt, &f) in fp.panel.face_origin.iter().enumerate() {
                let t = mesh.triangles()[f as usize];
                let (pj, plt) = *owner.get(&key(t.map(|v| mirror[v as usize])))?;
                if *partner.get_or_insert(pj) != pj {
                    return None;
                }
                let local = fp.panel.triangles[lt];
                let other = flat[pj].panel.triangles[plt];
                for (k, &v) in t.iter().enumerate() {
                    let want = mirror[v as usize];
                    let j = (0..3)
                        .find(|&j| flat[pj].panel.vertex_origin[other[j] as usize] == want)?;
                    map[local[k] as usize] = other[j];
                }
            }
            if map.contains(&u32::MAX) {
                return None;
            }
            Some((partner?, map))
        })
        .collect()
}

/// Make mirror-image pieces exact mirror images, and pieces on the centre
/// line exactly symmetric (see the module docs). Returns, per piece, the
/// piece it mirrors (`Some(self)` for a symmetric piece).
pub fn mirror_pieces(mesh: &Mesh, mirror: &[u32], flat: &mut [FlatPanel]) -> Vec<Option<usize>> {
    let partners = mirror_partners(mesh, mirror, flat);
    let mut out = vec![None; flat.len()];
    for q in 0..flat.len() {
        let Some((p, map)) = &partners[q] else {
            continue;
        };
        let p = *p;
        // The partner must agree that q is its mirror.
        if partners[p].as_ref().map(|x| x.0) != Some(q) {
            continue;
        }
        if p == q {
            symmetrise(&mut flat[q].uv, map);
            out[q] = Some(q);
        } else if p < q {
            let src = flat[p].uv.clone();
            let uv: Vec<DVec2> = map
                .iter()
                .map(|&j| {
                    let u = src[j as usize];
                    DVec2::new(-u.x, u.y)
                })
                .collect();
            let lo = uv
                .iter()
                .fold(DVec2::splat(f64::INFINITY), |m, u| m.min(*u));
            let base = src
                .iter()
                .fold(DVec2::splat(f64::INFINITY), |m, u| m.min(*u));
            flat[q].uv = uv.into_iter().map(|u| u - lo + base).collect();
            out[q] = Some(p);
            out[p] = Some(q);
        }
    }
    out
}

/// `uᵢ ← ½ (uᵢ + F(u_{m(i)}))` with `F` the best-fitting reflection.
fn symmetrise(uv: &mut [DVec2], map: &[u32]) {
    let n = uv.len() as f64;
    let c = uv.iter().sum::<DVec2>() / n;
    let (mut sc, mut ss) = (0.0, 0.0);
    for (i, &j) in map.iter().enumerate() {
        let (a, b) = (uv[i] - c, uv[j as usize] - c);
        sc += a.x * b.x - a.y * b.y;
        ss += a.x * b.y + a.y * b.x;
    }
    let two_phi = ss.atan2(sc);
    let (s2, c2) = two_phi.sin_cos();
    let reflect = |u: DVec2| {
        let d = u - c;
        c + DVec2::new(c2 * d.x + s2 * d.y, s2 * d.x - c2 * d.y)
    };
    let old = uv.to_vec();
    for (i, &j) in map.iter().enumerate() {
        uv[i] = 0.5 * (old[i] + reflect(old[j as usize]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flatten::{flatten_garment, FlattenOptions};
    use crate::seams::SeamSet;
    use crate::testmesh;
    use glam::DVec3;

    /// Distance of the bottom edge's vertices from the line y = 0.
    fn bottom_wobble(fp: &FlatPanel) -> f64 {
        // The bottom edge: outline vertices whose body y was 0.
        let ys: Vec<f64> = fp
            .panel
            .boundary
            .iter()
            .map(|e| e.from as usize)
            .filter(|&v| fp.panel.positions[v].y.abs() < 1.5)
            .map(|v| fp.uv[v].y)
            .collect();
        let mean = ys.iter().sum::<f64>() / ys.len() as f64;
        ys.iter().map(|y| (y - mean).abs()).fold(0.0, f64::max)
    }

    #[test]
    fn smoothing_irons_out_a_zigzag_and_keeps_corners() {
        // A flat 200 × 100 mm sheet whose bottom edge zigzags by ±1 mm.
        let g = testmesh::grid(20, 10, 10.0);
        let positions: Vec<DVec3> = g
            .positions()
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                if p.y == 0.0 && p.x > 0.0 && p.x < 200.0 {
                    p + DVec3::new(0.0, if i % 2 == 0 { 1.0 } else { -1.0 }, 0.0)
                } else {
                    p
                }
            })
            .collect();
        let m = Mesh::new(positions, g.triangles().to_vec()).unwrap();
        let run = |sigma: f64| {
            let opts = FlattenOptions {
                outline_smoothing_mm: sigma,
                ..Default::default()
            };
            flatten_garment(&m, &SeamSet::default(), &opts)
                .unwrap()
                .panels
                .remove(0)
        };
        let (raw, smooth) = (run(0.0), run(10.0));
        let (w0, w1) = (bottom_wobble(&raw), bottom_wobble(&smooth));
        // ±1 mm on the body is about ±0.85 mm flat at the default ease.
        assert!(w0 > 0.7 && w1 < 0.2 * w0, "{w0} -> {w1}");
        // The four corners keep their shape: the sheet stays as wide.
        let width = |fp: &FlatPanel| {
            let xs = fp.uv.iter().map(|u| u.x);
            xs.clone().fold(f64::MIN, f64::max) - xs.fold(f64::MAX, f64::min)
        };
        assert!((width(&raw) - width(&smooth)).abs() < 0.05);
        assert!(smooth.strain.flipped.is_empty());
    }

    #[test]
    fn symmetrise_gives_an_exact_reflection() {
        // A noisy symmetric shape: pairs (i, i+3) mirrored across x = 1,
        // tilted by 0.3 rad.
        let rot = |p: DVec2| DVec2::from_angle(0.3).rotate(p);
        let base = [
            DVec2::new(0.0, 0.0),
            DVec2::new(0.2, 1.0),
            DVec2::new(0.5, 2.1),
        ];
        let mut uv: Vec<DVec2> = base
            .iter()
            .map(|&p| rot(p))
            .chain(
                base.iter()
                    .map(|&p| rot(DVec2::new(2.0 - p.x + 0.05, p.y - 0.03))),
            )
            .collect();
        let map = [3, 4, 5, 0, 1, 2];
        symmetrise(&mut uv, &map);
        // Distances between partners' images are now mirror-consistent:
        // |u0 − u1| = |u3 − u4|, and the midpoints of pairs are collinear.
        assert!((uv[0].distance(uv[1]) - uv[3].distance(uv[4])).abs() < 1e-12);
        assert!((uv[1].distance(uv[2]) - uv[4].distance(uv[5])).abs() < 1e-12);
        let m: Vec<DVec2> = (0..3).map(|i| 0.5 * (uv[i] + uv[i + 3])).collect();
        assert!((m[1] - m[0]).perp_dot(m[2] - m[0]).abs() < 1e-12);
    }
}
