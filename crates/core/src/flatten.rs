//! End-to-end flattening: cut → LSCM → ARAP → seam matching → strain.

use crate::arap::{arap, ArapOptions, EdgeTarget};
use crate::cut::cut_panels;
use crate::ease::{reference_triangles, Ease};
use crate::error::{Error, Result};
use crate::lscm::lscm;
use crate::mesh::Mesh;
use crate::panel::Panel;
use crate::seam_match::{
    flat_edge_length, pair_seams, seam_edge_pairs, seam_lengths, SeamLengths, SeamPair,
};
use crate::seams::SeamSet;
use crate::strain::{measure, StrainReport};
use glam::{DVec2, DVec3};

/// Options for [`flatten_garment`].
#[derive(Debug, Clone, PartialEq)]
pub struct FlattenOptions {
    pub ease: Ease,
    pub arap: ArapOptions,
    /// Body "up" direction; flat pieces are turned so it points up the
    /// page. OBJ files are conventionally Y-up.
    pub up: DVec3,
    /// Equalise the two sides of every seam; `None` to skip.
    pub seam_matching: Option<SeamMatching>,
    /// Fraction of the around-the-body reduction applied along the body
    /// (see [`reference_triangles`]); 1 = the same in both directions.
    pub length_relief: f64,
    /// "Along the body" direction per input-mesh triangle (e.g. along the
    /// bones). `None` uses `up`.
    pub along: Option<Vec<DVec3>>,
    /// Relative membrane stiffness `k = μ t` of each panel (in
    /// [`cut_panels`] order), used to weight seam matching when panels are
    /// cut from different sheets. `None` means all equal.
    pub panel_stiffness: Option<Vec<f64>>,
}

impl Default for FlattenOptions {
    fn default() -> Self {
        Self {
            ease: Ease::default(),
            arap: ArapOptions::default(),
            up: DVec3::Y,
            seam_matching: Some(SeamMatching::default()),
            panel_stiffness: None,
            length_relief: 1.0,
            along: None,
        }
    }
}

/// Bounds on a corrected seam edge target, relative to the agreed length.
const TARGET_MIN: f64 = 0.75;
const TARGET_MAX: f64 = 1.33;

/// Seam-length equalisation settings.
///
/// For every paired seam edge with flat lengths `ℓ_L`, `ℓ_R` the agreed
/// length is the stiffness-weighted mean `ℓ* = (k_L ℓ_L + k_R ℓ_R) /
/// (k_L + k_R)`, where the two tensions balance: gluing a thin sheet to a
/// thick one moves the thick side less. With equal stiffness (the default)
/// this is the plain mean. Each round re-runs ARAP on every
/// seamed panel with soft [`EdgeTarget`]s of weight `weight`, then
/// corrects each side's target by the gap it still has,
/// `t ← t + (ℓ* − ℓ)` (with `ℓ*` recomputed from the new lengths). A soft
/// constraint alone settles short of its target because the panel pulls
/// back; this integral-style update removes that residual, and its fixed
/// point is `ℓ_L = ℓ_R`. Targets are kept within 0.75–1.33 × `ℓ*` so an
/// edge the panel cannot bend to its target (at a sharp corner, say) does
/// not wind its target up without limit.
///
/// Matching never adds folds: if a round leaves any panel with more
/// flipped triangles than it had unmatched, that round is discarded and
/// matching stops, keeping the remaining mismatch (which is reported). Such
/// a seam needs shaping, such as a dart or a princess line, or easing in. Higher weights converge in fewer rounds but
/// distort the panel more next to the seam.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct SeamMatching {
    pub weight: f64,
    /// Maximum number of rounds.
    pub rounds: usize,
    /// Stop once, for every seam pair, `Σ_k |ℓ_L,k − ℓ_R,k|` is at most
    /// this (mm). This bounds both the total mismatch and every edge's gap.
    pub tolerance: f64,
}

impl Default for SeamMatching {
    fn default() -> Self {
        Self {
            weight: 10.0,
            rounds: 12,
            tolerance: 0.01,
        }
    }
}

/// Seam pair with its lengths before and after matching.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeamReport {
    pub pair: SeamPair,
    /// Lengths after independent flattening.
    pub unmatched: SeamLengths,
    /// Final lengths.
    pub lengths: SeamLengths,
}

/// A flattened garment.
#[derive(Debug, Clone)]
pub struct Garment {
    pub panels: Vec<FlatPanel>,
    pub seams: Vec<SeamReport>,
}

/// A panel together with its flat layout and strain report.
#[derive(Debug, Clone)]
pub struct FlatPanel {
    pub panel: Panel,
    /// Flat position (mm) per local vertex. Seen from outside the body
    /// (normal towards the viewer), counter-clockwise, `y` up.
    pub uv: Vec<DVec2>,
    pub strain: StrainReport,
    pub arap_iterations: usize,
}

/// Flatten one panel towards reference triangles `refs` (see
/// [`reference_triangles`]), oriented so that `up` points up the page.
pub fn flatten_panel(
    panel: Panel,
    refs: &[[DVec2; 3]],
    opts: &ArapOptions,
    up: DVec3,
) -> Result<FlatPanel> {
    let init = lscm(&panel)?;
    let mut r = arap(&panel, refs, &init, opts, &[])?;
    orient(&panel, &mut r.uv, up);
    let strain = measure(&panel, &r.uv);
    Ok(FlatPanel {
        panel,
        uv: r.uv,
        strain,
        arap_iterations: r.iterations,
    })
}

/// Rigidly place a flat layout (rotation + translation only, so strain is
/// unchanged):
/// 1. rotate so the minimum-area bounding rectangle is axis-aligned (one of
///    its sides is collinear with a convex-hull edge, so every boundary
///    edge direction is tried);
/// 2. of the four 90° turns of that, keep the one maximising the
///    covariance `Σ (y_i − ȳ)(h_i − h̄)` between flat `y` and body height
///    `h_i = p_i · up`;
/// 3. translate the bounding box's lower-left corner to the origin.
pub fn orient(panel: &Panel, uv: &mut [DVec2], up: DVec3) {
    let rotate = |p: DVec2, a: DVec2| DVec2::new(a.x * p.x - a.y * p.y, a.y * p.x + a.x * p.y);
    let area_at = |a: DVec2| {
        let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
        for e in &panel.boundary {
            let q = rotate(uv[e.from as usize], a);
            lo = lo.min(q);
            hi = hi.max(q);
        }
        (hi - lo).x * (hi - lo).y
    };
    let mut best = (DVec2::X, f64::INFINITY);
    for e in &panel.boundary {
        let d = uv[e.to as usize] - uv[e.from as usize];
        if d.length_squared() == 0.0 {
            continue;
        }
        // Rotation taking direction d to +x: angle −atan2(d).
        let n = d.normalize();
        let a = DVec2::new(n.x, -n.y);
        let area = area_at(a);
        if area < best.1 * (1.0 - 1e-9) {
            best = (a, area);
        }
    }
    let base = best.0;

    let heights: Vec<f64> = panel.positions.iter().map(|p| p.dot(up)).collect();
    let hm = heights.iter().sum::<f64>() / heights.len() as f64;
    let quarter = [DVec2::X, DVec2::Y, DVec2::NEG_X, DVec2::NEG_Y];
    let score = |a: DVec2| {
        let ys: Vec<f64> = uv.iter().map(|&p| rotate(p, a).y).collect();
        let ym = ys.iter().sum::<f64>() / ys.len() as f64;
        ys.iter()
            .zip(&heights)
            .map(|(y, h)| (y - ym) * (h - hm))
            .sum::<f64>()
    };
    let a = quarter
        .iter()
        .map(|&q| rotate(base, q))
        .max_by(|x, y| score(*x).total_cmp(&score(*y)))
        .unwrap();

    for p in uv.iter_mut() {
        *p = rotate(*p, a);
    }
    let lo = uv
        .iter()
        .fold(DVec2::splat(f64::INFINITY), |l, p| l.min(*p));
    for p in uv.iter_mut() {
        *p -= lo;
    }
}

/// Re-run ARAP on an already flattened panel with edge-length targets.
pub fn refine_panel(
    fp: FlatPanel,
    refs: &[[DVec2; 3]],
    opts: &ArapOptions,
    up: DVec3,
    targets: &[EdgeTarget],
) -> Result<FlatPanel> {
    let mut r = arap(&fp.panel, refs, &fp.uv, opts, targets)?;
    orient(&fp.panel, &mut r.uv, up);
    let strain = measure(&fp.panel, &r.uv);
    Ok(FlatPanel {
        panel: fp.panel,
        uv: r.uv,
        strain,
        arap_iterations: fp.arap_iterations + r.iterations,
    })
}

/// Cut `mesh` along `seams`, flatten every panel and, if enabled, equalise
/// the lengths of paired seam edges.
pub fn flatten_garment(mesh: &Mesh, seams: &SeamSet, opts: &FlattenOptions) -> Result<Garment> {
    let panels = cut_panels(mesh, &seams.resolve(mesh)?)?;
    if let Some(k) = &opts.panel_stiffness {
        if k.len() != panels.len() || k.iter().any(|v| !(v.is_finite() && *v > 0.0)) {
            return Err(Error::InvalidEase(format!(
                "panel_stiffness needs {} positive value(s), got {:?}",
                panels.len(),
                k
            )));
        }
    }
    if !(0.0..=1.0).contains(&opts.length_relief) {
        return Err(Error::InvalidEase(format!(
            "length relief must be between 0 and 1, got {}",
            opts.length_relief
        )));
    }
    if let Some(a) = &opts.along {
        if a.len() != mesh.triangles().len() {
            return Err(Error::InvalidEase(
                "one 'along' direction is needed per mesh triangle".into(),
            ));
        }
    }
    let eases = panels
        .iter()
        .map(|p| {
            let ease = opts.ease.for_panel(p)?;
            let along: Vec<DVec3> = p
                .face_origin
                .iter()
                .map(|&f| opts.along.as_ref().map_or(opts.up, |a| a[f as usize]))
                .collect();
            Ok(reference_triangles(
                p,
                &ease,
                opts.length_relief,
                Some(&along),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut flat = panels
        .into_iter()
        .zip(&eases)
        .map(|(p, e)| flatten_panel(p, e, &opts.arap, opts.up))
        .collect::<Result<Vec<_>>>()?;

    let pairs = pair_seams(mesh, seams, &flat);
    let unmatched: Vec<SeamLengths> = pairs.iter().map(|p| seam_lengths(p, &flat)).collect();

    if let Some(m) = opts.seam_matching {
        let edges = seam_edge_pairs(&pairs);
        let baseline_flips: Vec<usize> = flat.iter().map(|fp| fp.strain.flipped.len()).collect();
        let k = |pi: usize| opts.panel_stiffness.as_ref().map_or(1.0, |v| v[pi]);
        let agreed = |(_, l, r): (usize, (usize, usize), (usize, usize)), ll: f64, lr: f64| {
            (k(l.0) * ll + k(r.0) * lr) / (k(l.0) + k(r.0))
        };
        // Current target per side, initialised to the agreed length.
        let mut targets: Vec<[f64; 2]> = edges
            .iter()
            .map(|&(_, l, r)| {
                let avg = agreed(
                    (0, l, r),
                    flat_edge_length(&flat[l.0], l.1),
                    flat_edge_length(&flat[r.0], r.1),
                );
                [avg, avg]
            })
            .collect();
        for round in 0..m.rounds {
            let lengths: Vec<(f64, f64)> = edges
                .iter()
                .map(|&(_, l, r)| {
                    (
                        flat_edge_length(&flat[l.0], l.1),
                        flat_edge_length(&flat[r.0], r.1),
                    )
                })
                .collect();
            let mut gap = vec![0.0; pairs.len()];
            for (&(pi, _, _), (a, b)) in edges.iter().zip(&lengths) {
                gap[pi] += (a - b).abs();
            }
            if gap.iter().all(|&g| g <= m.tolerance) {
                break;
            }
            if round > 0 {
                for ((t, &(ll, lr)), &e) in targets.iter_mut().zip(&lengths).zip(&edges) {
                    let avg = agreed(e, ll, lr);
                    // Anti-windup: an edge the panel cannot bend to its
                    // target (e.g. at a sharp corner) must not drive the
                    // target ever further away.
                    t[0] = (t[0] + avg - ll).clamp(TARGET_MIN * avg, TARGET_MAX * avg);
                    t[1] = (t[1] + avg - lr).clamp(TARGET_MIN * avg, TARGET_MAX * avg);
                }
            }
            let mut per_panel: Vec<Vec<EdgeTarget>> = vec![Vec::new(); flat.len()];
            for (t, &(_, l, r)) in targets.iter().zip(&edges) {
                for ((pi, bi), length) in [(l, t[0]), (r, t[1])] {
                    let e = flat[pi].panel.boundary[bi];
                    per_panel[pi].push(EdgeTarget {
                        a: e.from,
                        b: e.to,
                        length,
                        weight: m.weight,
                    });
                }
            }
            let next = flat
                .iter()
                .zip(&eases)
                .zip(&per_panel)
                .map(|((fp, e), t)| {
                    if t.is_empty() {
                        Ok(fp.clone())
                    } else {
                        refine_panel(fp.clone(), e, &opts.arap, opts.up, t)
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            // Never trade folds for equal seams: a seam the panels cannot
            // match without folding keeps its remaining mismatch, which is
            // then reported.
            let folds = next
                .iter()
                .zip(&baseline_flips)
                .any(|(fp, &b)| fp.strain.flipped.len() > b);
            if folds {
                break;
            }
            flat = next;
        }
    }

    let seams = pairs
        .into_iter()
        .zip(unmatched)
        .map(|(pair, unmatched)| {
            let lengths = seam_lengths(&pair, &flat);
            SeamReport {
                pair,
                unmatched,
                lengths,
            }
        })
        .collect();
    Ok(Garment {
        panels: flat,
        seams,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testmesh;

    #[test]
    fn pieces_are_axis_aligned_and_upright() {
        // Tall grid lying in the XY plane, rotated 30° about Z so it starts tilted.
        let g = testmesh::grid(2, 6, 10.0);
        let rot = glam::DQuat::from_rotation_z(0.5);
        let m = Mesh::new(
            g.positions().iter().map(|&p| rot * p).collect(),
            g.triangles().to_vec(),
        )
        .unwrap();
        let fp = &flatten_garment(&m, &SeamSet::default(), &FlattenOptions::default())
            .unwrap()
            .panels[0];
        let hi = fp.uv.iter().fold(DVec2::ZERO, |h, p| h.max(*p));
        assert!(
            (hi.x - 0.85 * 20.0).abs() < 1e-6 && (hi.y - 0.85 * 60.0).abs() < 1e-6,
            "{hi}"
        );
        // The vertex highest on the body is at the top of the page.
        let top = (0..m.positions().len())
            .max_by(|&a, &b| m.positions()[a].y.total_cmp(&m.positions()[b].y))
            .unwrap();
        let local = fp
            .panel
            .vertex_origin
            .iter()
            .position(|&o| o as usize == top)
            .unwrap();
        assert!(fp.uv[local].y > 0.5 * hi.y);
    }

    #[test]
    fn stiffer_panel_moves_less_in_seam_matching() {
        // Two halves at ease 0.8 and 0.9: the centre seam's sides want 128
        // and 144 mm.
        let m = testmesh::grid(10, 8, 20.0);
        let seams = SeamSet {
            seams: vec![crate::Seam {
                vertices: (0..=8).map(|j| j * 11 + 5).collect(),
                ..Default::default()
            }],
        };
        let ease = Ease::per_panel(&m, &seams, &[0.8, 0.9]).unwrap();
        let run = |k: Option<Vec<f64>>| {
            let opts = FlattenOptions {
                ease: ease.clone(),
                panel_stiffness: k,
                ..Default::default()
            };
            let g = flatten_garment(&m, &seams, &opts).unwrap();
            let s = &g.seams[0];
            assert!(s.lengths.mismatch().abs() < 0.01, "{:?}", s.lengths);
            // Length on panel 0's side.
            if s.pair.left.panel == 0 {
                s.lengths.left
            } else {
                s.lengths.right
            }
        };
        let equal = run(None);
        let p0_stiff = run(Some(vec![3.0, 1.0]));
        let p1_stiff = run(Some(vec![1.0, 3.0]));
        assert!((equal - 136.0).abs() < 0.1, "{equal}");
        assert!(
            p0_stiff < equal - 3.0 && (p0_stiff - 132.0).abs() < 0.1,
            "{p0_stiff}"
        );
        assert!(
            p1_stiff > equal + 3.0 && (p1_stiff - 140.0).abs() < 0.1,
            "{p1_stiff}"
        );
        assert!(flatten_garment(
            &m,
            &seams,
            &FlattenOptions {
                panel_stiffness: Some(vec![1.0]),
                ..Default::default()
            }
        )
        .is_err());
    }
}
