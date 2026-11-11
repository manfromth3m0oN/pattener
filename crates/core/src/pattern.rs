//! Pattern pieces: cut line with glue allowance, and matching notches.
//!
//! [`build_pattern`] turns a flattened [`Garment`] into one
//! [`PatternPiece`] per panel.
//!
//! ## Allowance
//! Each boundary edge gets an outward offset (see [`crate::allowance`]):
//! - a paired seam edge gets `seam_allowance` if its side is chosen by the
//!   seam's [`AllowanceSide`] (per-seam override, else the default);
//! - every other boundary edge (open mesh boundary: hems, necklines, …)
//!   gets `hem_allowance`.
//!
//! ## Notches
//! Notches are matching marks: the same number appears on both pieces of
//! a seam at the same point of the body. Positions are parametrised by
//! body arc length `τ` along the seam pair: mesh edge `k` with body length
//! `b_k` covers `τ ∈ [c_k, c_k + b_k]`, `c_k = Σ_{j<k} b_j`, and a point at
//! fraction `f` of edge `k` is placed at `from + f (to − from)` on the left
//! piece and `from + (1 − f)(to − from)` on the right piece (whose boundary
//! runs the other way). Automatic notches split a seam of body length `L`
//! into `round(L / spacing)` equal parts; explicit notch vertices from the
//! seam file are added at `f = 0` or `1` of their edge and take priority:
//! automatic notches within `spacing / 4` of an explicit one are dropped.
//! Latex should not be snipped (cuts start tears), so notches are drawn as
//! pen marks.

use crate::allowance::{is_simple, offset_polygon};
use crate::flatten::{FlatPanel, Garment};
use crate::seam_match::body_edge_length;
use crate::seams::{AllowanceSide, SeamId, SeamSet};
use glam::DVec2;
use std::collections::HashMap;

/// Options for [`build_pattern`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PatternOptions {
    /// Glue overlap on seams (mm).
    pub seam_allowance: f64,
    /// Default side that carries the seam allowance.
    pub allowance_side: AllowanceSide,
    /// Allowance on open edges (mm).
    pub hem_allowance: f64,
    /// Approximate body distance between automatic notches (mm); `None`
    /// for explicit notches only.
    pub notch_spacing: Option<f64>,
}

impl Default for PatternOptions {
    fn default() -> Self {
        Self {
            seam_allowance: 6.0,
            allowance_side: AllowanceSide::Left,
            hem_allowance: 0.0,
            notch_spacing: Some(50.0),
        }
    }
}

/// A matching mark on a seam edge.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Notch {
    /// Garment-wide number; the matching notch on the other piece has the
    /// same number.
    pub number: u32,
    pub seam: SeamId,
    /// Flat position on the seam line (mm).
    pub position: DVec2,
    /// Unit outward normal of the edge.
    pub normal: DVec2,
    /// Index into the panel's `boundary`.
    pub boundary_edge: usize,
}

/// Everything needed to draw and cut one piece.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternPiece {
    /// Allowance per boundary edge (mm), parallel to `panel.boundary`.
    pub edge_allowance: Vec<f64>,
    /// Cut line (flat coordinates, counter-clockwise).
    pub cut_outline: Vec<DVec2>,
    /// `false` if the allowance made the cut line cross itself.
    pub cut_outline_simple: bool,
    pub notches: Vec<Notch>,
}

impl PatternPiece {
    /// A piece cut on the seam line with no notches.
    pub fn plain(fp: &FlatPanel) -> Self {
        let cut_outline = fp
            .panel
            .boundary
            .iter()
            .map(|e| fp.uv[e.from as usize])
            .collect();
        Self {
            edge_allowance: vec![0.0; fp.panel.boundary.len()],
            cut_outline,
            cut_outline_simple: true,
            notches: Vec::new(),
        }
    }
}

/// Build the pattern pieces of a garment.
pub fn build_pattern(
    garment: &Garment,
    seams: &SeamSet,
    opts: &PatternOptions,
) -> Vec<PatternPiece> {
    let panels = &garment.panels;

    // (panel, boundary index) -> is left side, seam id.
    let mut side_of: HashMap<(usize, usize), (bool, SeamId)> = HashMap::new();
    for sr in &garment.seams {
        for &b in &sr.pair.left.edges {
            side_of.insert((sr.pair.left.panel, b), (true, sr.pair.seam));
        }
        for &b in &sr.pair.right.edges {
            side_of.insert((sr.pair.right.panel, b), (false, sr.pair.seam));
        }
    }

    let mut pieces: Vec<PatternPiece> = panels
        .iter()
        .enumerate()
        .map(|(pi, fp)| {
            let edge_allowance: Vec<f64> = (0..fp.panel.boundary.len())
                .map(|bi| match side_of.get(&(pi, bi)) {
                    Some(&(left, s)) => {
                        let side = seams.seams[s as usize]
                            .allowance
                            .unwrap_or(opts.allowance_side);
                        let gets = match side {
                            AllowanceSide::Left => left,
                            AllowanceSide::Right => !left,
                            AllowanceSide::Both => true,
                            AllowanceSide::None => false,
                        };
                        if gets {
                            opts.seam_allowance
                        } else {
                            0.0
                        }
                    }
                    None => opts.hem_allowance,
                })
                .collect();
            let pts: Vec<DVec2> = fp
                .panel
                .boundary
                .iter()
                .map(|e| fp.uv[e.from as usize])
                .collect();
            let cut_outline = offset_polygon(&pts, &edge_allowance);
            PatternPiece {
                cut_outline_simple: is_simple(&cut_outline),
                edge_allowance,
                cut_outline,
                notches: Vec::new(),
            }
        })
        .collect();

    let mut number = 0;
    for sr in &garment.seams {
        let p = &sr.pair;
        let lp = &panels[p.left.panel];
        let body: Vec<f64> = p
            .left
            .edges
            .iter()
            .map(|&b| body_edge_length(lp, b))
            .collect();
        let mut cum = vec![0.0];
        for b in &body {
            cum.push(cum.last().unwrap() + b);
        }
        let total = *cum.last().unwrap();

        // Positions as (edge k, fraction f).
        let tau_of = |(k, f): (usize, f64)| cum[k] + f * body[k];
        let to_edge = |tau: f64| {
            let k = cum
                .partition_point(|&c| c <= tau)
                .saturating_sub(1)
                .min(body.len() - 1);
            (k, ((tau - cum[k]) / body[k]).clamp(0.0, 1.0))
        };

        // Explicit notches first: they mark landmarks (e.g. the waist).
        let mut at: Vec<(usize, f64)> = Vec::new();
        let origin = &lp.panel.vertex_origin;
        for &v in &seams.seams[p.seam as usize].notches {
            for (k, &b) in p.left.edges.iter().enumerate() {
                let e = lp.panel.boundary[b];
                if origin[e.from as usize] == v {
                    at.push((k, 0.0));
                } else if k + 1 == p.left.edges.len() && origin[e.to as usize] == v {
                    at.push((k, 1.0));
                }
            }
        }
        // Automatic notches, skipping any within a quarter spacing of an
        // explicit one so two marks never sit side by side.
        if let Some(spacing) = opts.notch_spacing.filter(|s| *s > 0.0) {
            let explicit: Vec<f64> = at.iter().map(|&a| tau_of(a)).collect();
            let parts = (total / spacing).round() as usize;
            for j in 1..parts {
                let tau = total * j as f64 / parts as f64;
                if explicit.iter().all(|e| (e - tau).abs() >= 0.25 * spacing) {
                    at.push(to_edge(tau));
                }
            }
        }
        at.sort_by(|&a, &b| tau_of(a).total_cmp(&tau_of(b)));
        at.dedup_by(|a, b| (tau_of(*a) - tau_of(*b)).abs() < 1.0);

        for (k, f) in at {
            number += 1;
            for (panel, bi, frac) in [
                (p.left.panel, p.left.edges[k], f),
                (p.right.panel, p.right.edges[k], 1.0 - f),
            ] {
                let fp = &panels[panel];
                let e = fp.panel.boundary[bi];
                let (a, b) = (fp.uv[e.from as usize], fp.uv[e.to as usize]);
                let t = (b - a).normalize_or_zero();
                pieces[panel].notches.push(Notch {
                    number,
                    seam: p.seam,
                    position: a + frac * (b - a),
                    normal: DVec2::new(t.y, -t.x),
                    boundary_edge: bi,
                });
            }
        }
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allowance::polygon_area;
    use crate::{flatten_garment, testmesh, Ease, FlattenOptions, Seam};
    use approx::assert_relative_eq;

    /// 200 × 160 mm sheet split down the middle at ease 1, so flat = body.
    fn split_grid(seam: Seam) -> (Garment, SeamSet) {
        let m = testmesh::grid(10, 8, 20.0);
        let set = SeamSet { seams: vec![seam] };
        let opts = FlattenOptions {
            ease: Ease::Uniform(1.0),
            ..Default::default()
        };
        (flatten_garment(&m, &set, &opts).unwrap(), set)
    }

    fn centre(notches: Vec<u32>, allowance: Option<AllowanceSide>) -> Seam {
        Seam {
            name: None,
            vertices: (0..=8).map(|j| j * 11 + 5).collect(),
            notches,
            allowance,
        }
    }

    #[test]
    fn lap_allowance_goes_on_one_side_only() {
        let (g, set) = split_grid(centre(vec![], None));
        let pieces = build_pattern(&g, &set, &PatternOptions::default());
        let added: Vec<f64> = pieces
            .iter()
            .zip(&g.panels)
            .map(|(p, fp)| polygon_area(&p.cut_outline) - fp.strain.flat_area)
            .collect();
        // One 160 mm seam side gets 6 mm; the other piece is cut on the line.
        let left = g.seams[0].pair.left.panel;
        assert_relative_eq!(added[left], 160.0 * 6.0, epsilon = 1e-6);
        assert_relative_eq!(added[1 - left], 0.0, epsilon = 1e-6);
        assert!(pieces.iter().all(|p| p.cut_outline_simple));
    }

    #[test]
    fn per_seam_override_and_hem_allowance() {
        let (g, set) = split_grid(centre(vec![], Some(AllowanceSide::Both)));
        let opts = PatternOptions {
            hem_allowance: 10.0,
            ..Default::default()
        };
        for (p, fp) in build_pattern(&g, &set, &opts).iter().zip(&g.panels) {
            // Each half is 100 × 160: seam side +6, three open sides +10 (mitred corners).
            let expected = (100.0 + 6.0 + 10.0) * (160.0 + 20.0);
            assert_relative_eq!(polygon_area(&p.cut_outline), expected, epsilon = 1e-6);
            assert_relative_eq!(fp.strain.flat_area, 100.0 * 160.0, epsilon = 1e-6);
        }
    }

    #[test]
    fn notches_match_across_the_seam() {
        // Explicit notch at vertex (5, 2) = 27 plus automatic ones every ~50 mm.
        let (g, set) = split_grid(centre(vec![27], None));
        let pieces = build_pattern(&g, &set, &PatternOptions::default());
        let (a, b) = (&pieces[0].notches, &pieces[1].notches);
        // 160 mm / 50 → 3 parts → 2 automatic notches, plus the explicit one at 40 mm.
        assert_eq!(a.len(), 3);
        assert_eq!(b.len(), 3);
        for na in a {
            let nb = b.iter().find(|n| n.number == na.number).unwrap();
            // Both pieces were cut from one flat sheet: their seam lines
            // coincide up to a rigid motion, so compare body heights (y).
            let hy = |fp: &FlatPanel, n: &Notch| {
                n.position.y - fp.uv.iter().map(|p| p.y).fold(f64::INFINITY, f64::min)
            };
            assert_relative_eq!(hy(&g.panels[0], na), hy(&g.panels[1], nb), epsilon = 1e-6);
        }
        let mut heights: Vec<f64> = a
            .iter()
            .map(|n| {
                n.position.y
                    - g.panels[0]
                        .uv
                        .iter()
                        .map(|p| p.y)
                        .fold(f64::INFINITY, f64::min)
            })
            .collect();
        heights.sort_by(f64::total_cmp);
        for (h, e) in heights.iter().zip([40.0, 160.0 / 3.0, 320.0 / 3.0]) {
            assert_relative_eq!(*h, e, epsilon = 1e-6);
        }
    }

    #[test]
    fn explicit_notch_replaces_nearby_automatic_one() {
        // Explicit notch at vertex (5, 3) = 38, i.e. 60 mm up: within 12.5 mm
        // of the automatic notch at 53.3 mm, which is dropped.
        let (g, set) = split_grid(centre(vec![38], None));
        let pieces = build_pattern(&g, &set, &PatternOptions::default());
        let ymin = g.panels[0]
            .uv
            .iter()
            .map(|p| p.y)
            .fold(f64::INFINITY, f64::min);
        let mut h: Vec<f64> = pieces[0]
            .notches
            .iter()
            .map(|n| n.position.y - ymin)
            .collect();
        h.sort_by(f64::total_cmp);
        assert_eq!(h.len(), 2, "{h:?}");
        assert_relative_eq!(h[0], 60.0, epsilon = 1e-6);
        assert_relative_eq!(h[1], 320.0 / 3.0, epsilon = 1e-6);
    }
}
