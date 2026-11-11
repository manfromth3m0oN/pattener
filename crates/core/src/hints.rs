//! Where a piece strains: hints for adding a dart or seam.
//!
//! A flat sheet can only follow a surface that curves one way at a time.
//! Where the body curves both ways (bust, buttocks, shoulder, armpit), a
//! piece without a dart or seam there cannot take its intended shape when
//! laid flat and is distorted instead.
//!
//! For each flat triangle, the *misfit* is `max |ln sᵢ|` over the principal
//! stretches `sᵢ` of the map from its eased reference shape (what the ease
//! and lengthwise reduction ask for) to its flat shape: 0 when it lies flat
//! exactly as intended, 0.1 when it is about 10 % off in some direction.
//! A piece's *worst patch* is the highest area-weighted mean misfit within
//! [`HINT_RADIUS_MM`] of any of its triangles, which ignores single bad
//! triangles and finds where the body's shape fights the piece. Above
//! [`SPREAD_HINT`] the piece gets a hint at that patch: a dart or seam from
//! the nearest edge towards it (or, at an armpit, a set-in sleeve) lets the
//! piece lie flatter.

use crate::flatten::FlatPanel;
use glam::DVec3;

/// Radius of a patch (mm).
pub const HINT_RADIUS_MM: f64 = 60.0;
/// Worst-patch misfit above which a piece gets a hint.
pub const SPREAD_HINT: f64 = 0.10;

/// The worst patch of a piece: the largest area-weighted mean of
/// `misfit` within [`HINT_RADIUS_MM`] of a triangle (on the body), and the
/// local vertex nearest that triangle's centre.
pub fn worst_patch(fp: &FlatPanel, misfit: &[f64]) -> (f64, u32) {
    let p = &fp.panel;
    let tris: Vec<(DVec3, f64, f64)> = p
        .triangles
        .iter()
        .zip(misfit)
        .map(|(t, &m)| {
            let [a, b, c] = t.map(|i| p.positions[i as usize]);
            ((a + b + c) / 3.0, (b - a).cross(c - a).length() / 2.0, m)
        })
        .collect();
    let r2 = HINT_RADIUS_MM * HINT_RADIUS_MM;
    let mut best = (0.0, 0usize);
    for (i, &(c, _, _)) in tris.iter().enumerate() {
        let (mut wm, mut w) = (0.0, 0.0);
        for &(q, a, m) in &tris {
            if q.distance_squared(c) < r2 {
                wm += a * m;
                w += a;
            }
        }
        let mean = if w > 0.0 { wm / w } else { 0.0 };
        if mean > best.0 {
            best = (mean, i);
        }
    }
    let t = p.triangles[best.1];
    (best.0, t[0])
}
