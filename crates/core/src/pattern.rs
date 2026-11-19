//! Pattern pieces from a flat design: ease, unfolding, allowances, notches.
//!
//! [`build_pattern`] turns each [`Piece`](crate::design::Piece) of a [`Design`] into a
//! [`PatternPiece`] ready to print:
//!
//! 1. **Unfold.** A piece cut on the fold is mirrored about its fold line
//!    and printed whole, since latex is cut single-layer.
//! 2. **Ease.** The design is at body size; the piece is reduced by its
//!    ease `s` around the body and by `1 − r(1 − s)` along it, `r` being
//!    the lengthwise share of the reduction. "Along" is the piece's up
//!    direction ([`Piece::up_deg`](crate::design::Piece::up_deg); the
//!    page's `y` for a piece drawn upright), "around" the direction across
//!    it. So the piece stretches by about `1/s` around the body when worn.
//! 3. **Allowance.** The cut line is the seam line offset outwards (see
//!    [`crate::allowance`]): a glued seam's lap on the side its [`Lap`]
//!    names, a strip on both sides of a zip, the hem allowance of an open
//!    edge (its own if it is hemmed, none if it is raw or trimmed).
//! 4. **Notches.** Numbered matching marks: the same number on both sides
//!    of a seam, at the same fraction of each side's body length. A seam
//!    of body length `L` gets `round(L / spacing) − 1` evenly spaced
//!    notches, plus one at every node marked as a notch on either side
//!    (automatic ones within a quarter spacing of those are dropped).
//!    Latex should not be snipped (cuts start tears), so notches are drawn
//!    as pen marks.

use crate::allowance::{is_simple, offset_polygon, remove_loops};
use crate::design::{Design, EdgeFinish, Lap, SeamFinish};
use crate::error::{Error, Result};
use crate::geom::{self, locate_fraction, point_at, polyline_length};
use glam::DVec2;
use std::collections::HashMap;

/// Options for [`build_pattern`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct PatternOptions {
    /// Glue lap on seams (mm).
    pub seam_allowance: f64,
    /// Allowance on open edges without a finish of their own (mm).
    pub hem_allowance: f64,
    /// Approximate body distance between automatic notches (mm); `None`
    /// for marked notches only.
    pub notch_spacing: Option<f64>,
    /// Strip on each side of a zip seam for gluing the zip tape (mm).
    pub zip_allowance: f64,
}

impl Default for PatternOptions {
    fn default() -> Self {
        Self {
            seam_allowance: 6.0,
            hem_allowance: 0.0,
            notch_spacing: Some(50.0),
            zip_allowance: 10.0,
        }
    }
}

/// The reduction of one piece: scale factors around and along the body,
/// and the direction of "along".
///
/// With `u` the unit up-the-body direction (the page's `y` turned
/// anticlockwise by `up`) and `v` the direction across it, a point `p`
/// becomes `(p·v) around v + (p·u) along u`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reduction {
    pub around: f64,
    pub along: f64,
    /// Up the body, in radians anticlockwise from the page's `y`.
    pub up: f64,
}

impl Reduction {
    /// From an ease and the lengthwise share of the reduction.
    pub fn new(ease: f64, length_relief: f64) -> Self {
        Self {
            around: ease,
            along: 1.0 - length_relief * (1.0 - ease),
            up: 0.0,
        }
    }

    /// The same reduction with up the body `deg` degrees anticlockwise from
    /// the page's up.
    pub fn up_deg(self, deg: f64) -> Self {
        Self {
            up: deg.to_radians(),
            ..self
        }
    }

    /// Unit vector up the body.
    pub fn up_dir(self) -> DVec2 {
        DVec2::new(-self.up.sin(), self.up.cos())
    }

    fn apply(self, p: DVec2) -> DVec2 {
        let u = self.up_dir();
        let v = DVec2::new(u.y, -u.x);
        p.dot(v) * self.around * v + p.dot(u) * self.along * u
    }

    /// How much an edge running in direction `d` is reduced.
    pub fn factor(self, d: DVec2) -> f64 {
        self.apply(d.normalize_or_zero()).length()
    }
}

/// A numbered matching mark on a seam.
#[derive(Debug, Clone, PartialEq)]
pub struct Notch {
    /// Garment-wide number; the matching notch has the same number.
    pub number: u32,
    /// Index of the design seam.
    pub seam: usize,
    /// On the seam line (mm).
    pub position: DVec2,
    /// Unit outward normal of the edge.
    pub normal: DVec2,
    /// Allowance of the edge here (mm), so the mark crosses it.
    pub allowance: f64,
}

/// Text along an edge, placed inside the piece.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeLabel {
    /// Midpoint of the edge on the seam line.
    pub at: DVec2,
    /// Unit direction of the outline there (counter-clockwise).
    pub dir: DVec2,
    pub text: String,
}

/// One copy of a design edge on a pattern piece (a piece cut on the fold
/// has each edge twice).
#[derive(Debug, Clone, PartialEq)]
struct EdgeCopy {
    /// Flat points in the design edge's direction.
    points: Vec<DVec2>,
    /// The copy's direction is the outline's (counter-clockwise).
    forward: bool,
}

/// Everything needed to draw and cut one piece.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternPiece {
    /// Index of the design piece.
    pub piece: usize,
    pub name: String,
    /// Cut twice, the second as a mirror image.
    pub pair: bool,
    /// Unfolded from a piece cut on the fold.
    pub folded: bool,
    pub reduction: Reduction,
    /// Seam line, counter-clockwise (mm, flat).
    pub seam_line: Vec<DVec2>,
    /// Allowance of each seam-line segment (from point `i` to `i + 1`).
    pub allowance: Vec<f64>,
    /// Cut line (counter-clockwise).
    pub cut_outline: Vec<DVec2>,
    /// `false` if the allowance made the cut line cross itself.
    pub cut_outline_simple: bool,
    pub notches: Vec<Notch>,
    /// The centre line a folded piece was unfolded about.
    pub fold_line: Option<[DVec2; 2]>,
    pub labels: Vec<EdgeLabel>,
    /// Per design edge, its copies on this piece.
    edges: Vec<Vec<EdgeCopy>>,
    /// Body length of each design edge (mm).
    body_lengths: Vec<f64>,
    /// Body-size points of each design edge, for locating notches.
    body_edges: Vec<Vec<DVec2>>,
}

impl PatternPiece {
    /// Flat length of design edge `k` (one copy).
    pub fn flat_length(&self, k: usize) -> f64 {
        polyline_length(&self.edges[k][0].points)
    }

    /// Body length of design edge `k`.
    pub fn body_length(&self, k: usize) -> f64 {
        self.body_lengths[k]
    }

    /// Flat area of the seam-line outline (mm²).
    pub fn area(&self) -> f64 {
        geom::signed_area(&self.seam_line).abs()
    }

    /// Number of pieces to cut.
    pub fn copies(&self) -> usize {
        if self.pair {
            2
        } else {
            1
        }
    }

    /// How to cut it, in words.
    pub fn cut_text(&self) -> String {
        match (self.pair, self.folded) {
            (true, true) => "cut 2 (a mirrored pair), each unfolded as shown".into(),
            (true, false) => "cut 2: one as printed, one flipped over (a mirrored pair)".into(),
            (false, true) => "cut 1 (drafted on the fold, shown unfolded)".into(),
            (false, false) => "cut 1".into(),
        }
    }

    /// Add a notch at fraction `t` of design edge `k`'s body length, on
    /// every copy of the edge.
    fn add_notch(&mut self, number: u32, seam: usize, k: usize, t: f64, allowance: f64) {
        let at = locate_fraction(&self.body_edges[k], t);
        for c in &self.edges[k] {
            let p = point_at(&c.points, at);
            let d = geom::direction_at(&c.points, at);
            let d = if c.forward { d } else { -d };
            self.notches.push(Notch {
                number,
                seam,
                position: p,
                normal: DVec2::new(d.y, -d.x),
                allowance,
            });
        }
    }
}

/// A trim strip to cut for edges with [`EdgeFinish::Trim`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TrimStrip {
    /// The edge it finishes (edges with the same name share a strip).
    pub edge: String,
    /// Flat length of the edge, all around the garment (mm).
    pub edge_length_mm: f64,
    /// Length to cut: `ease ×` the edge length (mm).
    pub length_mm: f64,
    pub width_mm: f64,
    /// Pattern pieces along the edge.
    pub pieces: Vec<usize>,
}

impl TrimStrip {
    /// The strip as a closed outline (a rectangle at the origin).
    pub fn outline(&self) -> Vec<DVec2> {
        let (l, w) = (self.length_mm, self.width_mm);
        vec![
            DVec2::ZERO,
            DVec2::new(l, 0.0),
            DVec2::new(l, w),
            DVec2::new(0.0, w),
        ]
    }
}

/// What an edge is: one side of a seam, a fold, or open.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EdgeRole {
    /// Side `a` (`true`) or `b` of seam `seam`; both for a seam joining an
    /// edge to its mirror image.
    Seam {
        seam: usize,
        a: bool,
        b: bool,
    },
    Fold,
    Open,
}

/// The role of every edge of every piece.
pub fn edge_roles(design: &Design) -> Vec<Vec<EdgeRole>> {
    let mut roles: Vec<Vec<EdgeRole>> = design
        .pieces
        .iter()
        .map(|p| {
            (0..p.edges().len())
                .map(|k| {
                    if p.edge_props(k).fold {
                        EdgeRole::Fold
                    } else {
                        EdgeRole::Open
                    }
                })
                .collect()
        })
        .collect();
    for (si, s) in design.seams.iter().enumerate() {
        let (Some(a), Some(b)) = (design.resolve(&s.a), design.resolve(&s.b)) else {
            continue;
        };
        roles[a.0][a.1] = EdgeRole::Seam {
            seam: si,
            a: true,
            b: a == b,
        };
        if a != b {
            roles[b.0][b.1] = EdgeRole::Seam {
                seam: si,
                a: false,
                b: true,
            };
        }
    }
    roles
}

/// Allowance of an edge (mm).
fn allowance_of(
    design: &Design,
    role: EdgeRole,
    finish: Option<EdgeFinish>,
    o: &PatternOptions,
) -> f64 {
    match role {
        EdgeRole::Fold => 0.0,
        EdgeRole::Open => match finish {
            Some(EdgeFinish::Hem { width_mm }) => width_mm,
            Some(EdgeFinish::Raw | EdgeFinish::Trim { .. }) => 0.0,
            None => o.hem_allowance,
        },
        EdgeRole::Seam { seam, a, b } => {
            let s = &design.seams[seam];
            if s.finish == SeamFinish::Zip {
                return o.zip_allowance;
            }
            let gets = match s.lap {
                Lap::A => a,
                Lap::B => b,
                Lap::Both => true,
                Lap::None => false,
            };
            if gets {
                o.seam_allowance
            } else {
                0.0
            }
        }
    }
}

/// Build the pattern pieces of a design, one per design piece.
/// `reductions` gives each piece's ease (see [`Reduction`]).
pub fn build_pattern(
    design: &Design,
    reductions: &[Reduction],
    opts: &PatternOptions,
) -> Result<Vec<PatternPiece>> {
    let problems = design.problems();
    if !problems.is_empty() {
        return Err(Error::Design(problems.join("; ")));
    }
    assert_eq!(reductions.len(), design.pieces.len());
    let roles = edge_roles(design);
    let mut pieces: Vec<PatternPiece> = design
        .pieces
        .iter()
        .enumerate()
        .map(|(pi, piece)| {
            let red = reductions[pi];
            let segs = piece.segment_points();
            let n_edges = piece.edges().len();
            let body_edges: Vec<Vec<DVec2>> = (0..n_edges)
                .map(|k| piece.edge_points_from(&segs, k))
                .collect();
            let fold = piece.folds().first().copied();

            // The outline as (point, (edge, mirrored)) for the segment
            // starting at the point, in the design's direction.
            let mut loop_pts: Vec<(DVec2, (usize, bool))> = Vec::new();
            let mut fold_line = None;
            match fold {
                None => {
                    let (pts, owner) = piece.outline();
                    loop_pts = pts
                        .into_iter()
                        .zip(owner.into_iter().map(|k| (k, false)))
                        .collect();
                }
                Some(kf) => {
                    let e = piece.edges()[kf];
                    let (fa, fb) = (piece.nodes[e.start].pos(), piece.nodes[e.end].pos());
                    fold_line = Some([fa, fb]);
                    // Chain from the fold's end round to its start.
                    let mut chain: Vec<(DVec2, usize)> = Vec::new();
                    for j in 1..n_edges {
                        let k = (kf + j) % n_edges;
                        let pts = &body_edges[k];
                        for q in &pts[..pts.len() - 1] {
                            chain.push((*q, k));
                        }
                    }
                    for &(q, k) in &chain {
                        loop_pts.push((q, (k, false)));
                    }
                    // The fold's start, then the chain mirrored backwards.
                    loop_pts.push((fa, (chain[chain.len() - 1].1, true)));
                    for i in (1..chain.len()).rev() {
                        let q = geom::reflect(chain[i].0, fa, fb);
                        loop_pts.push((q, (chain[i - 1].1, true)));
                    }
                }
            }
            // Counter-clockwise.
            let pts: Vec<DVec2> = loop_pts.iter().map(|x| x.0).collect();
            let ccw = geom::signed_area(&pts) > 0.0;
            if !ccw {
                let n = loop_pts.len();
                let owners: Vec<(usize, bool)> = loop_pts.iter().map(|x| x.1).collect();
                loop_pts = (0..n)
                    .map(|j| (pts[n - 1 - j], owners[(2 * n - 2 - j) % n]))
                    .collect();
            }

            let seam_line: Vec<DVec2> = loop_pts.iter().map(|x| red.apply(x.0)).collect();
            let edge_allowance: Vec<f64> = (0..n_edges)
                .map(|k| allowance_of(design, roles[pi][k], piece.edge_props(k).finish, opts))
                .collect();
            let allowance: Vec<f64> = loop_pts.iter().map(|x| edge_allowance[x.1 .0]).collect();
            let cut_outline = remove_loops(offset_polygon(&seam_line, &allowance));

            let mirror = |q: DVec2| match fold_line {
                Some([a, b]) => geom::reflect(q, a, b),
                None => q,
            };
            let edges: Vec<Vec<EdgeCopy>> = (0..n_edges)
                .map(|k| {
                    let mut copies = vec![EdgeCopy {
                        points: body_edges[k].iter().map(|&q| red.apply(q)).collect(),
                        forward: ccw,
                    }];
                    if fold.is_some() && Some(k) != fold {
                        copies.push(EdgeCopy {
                            points: body_edges[k]
                                .iter()
                                .map(|&q| red.apply(mirror(q)))
                                .collect(),
                            forward: !ccw,
                        });
                    }
                    copies
                })
                .collect();

            PatternPiece {
                piece: pi,
                name: piece.name.clone(),
                pair: piece.pair,
                folded: fold.is_some(),
                reduction: red,
                cut_outline_simple: is_simple(&cut_outline),
                cut_outline,
                seam_line,
                allowance,
                notches: Vec::new(),
                fold_line: fold_line.map(|[a, b]| [red.apply(a), red.apply(b)]),
                labels: Vec::new(),
                body_lengths: body_edges.iter().map(|e| polyline_length(e)).collect(),
                body_edges,
                edges,
            }
        })
        .collect();

    // Notches, seam by seam.
    let mut number = 0;
    for (si, s) in design.seams.iter().enumerate() {
        let (a, b) = (design.resolve(&s.a).unwrap(), design.resolve(&s.b).unwrap());
        let (la, lb) = (pieces[a.0].body_length(a.1), pieces[b.0].body_length(b.1));
        let len = 0.5 * (la + lb);
        let to_b = |t: f64| if s.reverse { 1.0 - t } else { t };
        // Marked notches on either side, as fractions of side a.
        let mut marked: Vec<f64> = design.pieces[a.0].notch_fractions(a.1);
        if a != b {
            marked.extend(
                design.pieces[b.0]
                    .notch_fractions(b.1)
                    .into_iter()
                    .map(to_b),
            );
        }
        let mut at = marked.clone();
        if let Some(spacing) = opts.notch_spacing.filter(|x| *x > 0.0) {
            let parts = (len / spacing).round() as usize;
            for j in 1..parts {
                let t = j as f64 / parts as f64;
                if marked.iter().all(|m| (m - t).abs() * len >= 0.25 * spacing) {
                    at.push(t);
                }
            }
        }
        at.sort_by(f64::total_cmp);
        at.dedup_by(|x, y| (*x - *y).abs() * len < 1.0);
        let allowance = |p: (usize, usize)| {
            allowance_of(
                design,
                roles[p.0][p.1],
                design.pieces[p.0].edge_props(p.1).finish,
                opts,
            )
        };
        let (wa, wb) = (allowance(a), allowance(b));
        for t in at {
            number += 1;
            pieces[a.0].add_notch(number, si, a.1, t, wa);
            if a != b {
                pieces[b.0].add_notch(number, si, b.1, to_b(t), wb);
            }
        }
    }

    // Edge labels.
    let piece_name = |p: usize| design.pieces[p].name.clone();
    for (pi, piece) in design.pieces.iter().enumerate() {
        for (k, role) in roles[pi].iter().enumerate() {
            let props = piece.edge_props(k);
            let text = match *role {
                EdgeRole::Fold => continue,
                EdgeRole::Seam { seam, a, b } => {
                    let s = &design.seams[seam];
                    let name = if s.name.is_empty() {
                        "seam".to_string()
                    } else {
                        s.name.clone()
                    };
                    let kind = if s.finish == SeamFinish::Zip {
                        "zip"
                    } else {
                        "seam"
                    };
                    if a && b {
                        format!("{name} {kind}: joins its mirror image")
                    } else {
                        let other = if a { &s.b } else { &s.a };
                        let other = design
                            .piece_index(&other.piece)
                            .map(piece_name)
                            .unwrap_or_default();
                        if s.name.is_empty() {
                            format!("{kind} to {other}")
                        } else {
                            format!("{name} {kind} to {other}")
                        }
                    }
                }
                EdgeRole::Open => {
                    let finish = match props.finish {
                        Some(EdgeFinish::Hem { width_mm }) => format!("{width_mm:.0} mm hem"),
                        Some(EdgeFinish::Trim { width_mm, .. }) => format!("{width_mm:.0} mm trim"),
                        Some(EdgeFinish::Raw) => "raw edge".to_string(),
                        None if opts.hem_allowance > 0.0 => {
                            format!("{:.0} mm hem", opts.hem_allowance)
                        }
                        None => String::new(),
                    };
                    match (props.name.is_empty(), finish.is_empty()) {
                        (true, true) => continue,
                        (false, true) => props.name.clone(),
                        (true, false) => finish,
                        (false, false) => format!("{}: {finish}", props.name),
                    }
                }
            };
            let labels: Vec<EdgeLabel> = pieces[pi].edges[k]
                .iter()
                .map(|c| {
                    let at = locate_fraction(&c.points, 0.5);
                    let d = geom::direction_at(&c.points, at);
                    EdgeLabel {
                        at: point_at(&c.points, at),
                        dir: if c.forward { d } else { -d },
                        text: text.clone(),
                    }
                })
                .collect();
            pieces[pi].labels.extend(labels);
        }
    }
    Ok(pieces)
}

/// The trim strips for edges finished with a trim: one per edge name,
/// covering that edge on every copy of every piece.
pub fn trim_strips(design: &Design, pieces: &[PatternPiece]) -> Vec<TrimStrip> {
    let mut order: Vec<String> = Vec::new();
    let mut by_name: HashMap<String, TrimStrip> = HashMap::new();
    let mut ease_of: HashMap<String, f64> = HashMap::new();
    for (pi, piece) in design.pieces.iter().enumerate() {
        for k in 0..piece.edges().len() {
            let props = piece.edge_props(k);
            let Some(EdgeFinish::Trim { width_mm, ease }) = props.finish else {
                continue;
            };
            if design.seam_on(pi, k).is_some() || props.fold {
                continue;
            }
            let name = if props.name.is_empty() {
                format!("{} {}", piece.name, piece.edge_name(k))
            } else {
                props.name.clone()
            };
            let p = &pieces[pi];
            let length: f64 = p.edges[k]
                .iter()
                .map(|c| polyline_length(&c.points))
                .sum::<f64>()
                * p.copies() as f64;
            let t = by_name.entry(name.clone()).or_insert_with(|| {
                order.push(name.clone());
                ease_of.insert(name.clone(), ease);
                TrimStrip {
                    edge: name.clone(),
                    edge_length_mm: 0.0,
                    length_mm: 0.0,
                    width_mm,
                    pieces: Vec::new(),
                }
            });
            t.edge_length_mm += length;
            if !t.pieces.contains(&pi) {
                t.pieces.push(pi);
            }
        }
    }
    order
        .into_iter()
        .map(|name| {
            let mut t = by_name.remove(&name).unwrap();
            t.length_mm = ease_of[&name] * t.edge_length_mm;
            t
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allowance::polygon_area;
    use crate::design::tests::rect;
    use crate::design::{EdgeRef, Node, NodeKind, Seam};
    use approx::assert_relative_eq;

    fn edge(piece: &str, node: &str) -> EdgeRef {
        EdgeRef {
            piece: piece.into(),
            node: node.into(),
        }
    }

    /// Two 100 × 160 mm rectangles side by side, seamed along "a"'s right
    /// edge and "b"'s left edge (opposite directions).
    fn two_halves(seam: Seam) -> Design {
        Design {
            pieces: vec![rect("a", 100.0, 160.0), rect("b", 100.0, 160.0)],
            seams: vec![seam],
            block: None,
        }
    }

    fn centre_seam() -> Seam {
        Seam {
            id: "s".into(),
            name: "centre".into(),
            a: edge("a", "br"),
            b: edge("b", "tl"),
            reverse: true,
            ..Default::default()
        }
    }

    fn unit(n: usize) -> Vec<Reduction> {
        vec![Reduction::new(1.0, 1.0); n]
    }

    #[test]
    fn lap_allowance_goes_on_one_side_only() {
        let d = two_halves(centre_seam());
        let p = build_pattern(&d, &unit(2), &PatternOptions::default()).unwrap();
        let added: Vec<f64> = p
            .iter()
            .map(|x| polygon_area(&x.cut_outline) - x.area())
            .collect();
        assert_relative_eq!(added[0], 160.0 * 6.0, epsilon = 1e-6);
        assert_relative_eq!(added[1], 0.0, epsilon = 1e-6);
        assert!(p.iter().all(|x| x.cut_outline_simple));
    }

    #[test]
    fn zip_hem_and_both_laps() {
        let mut s = centre_seam();
        s.finish = SeamFinish::Zip;
        let mut d = two_halves(s);
        for piece in &mut d.pieces {
            piece.nodes[0].edge.finish = Some(EdgeFinish::Hem { width_mm: 15.0 });
        }
        let opts = PatternOptions::default();
        for x in build_pattern(&d, &unit(2), &opts).unwrap() {
            // +10 mm zip strip, +15 mm hem, meeting at a mitred corner.
            let added = polygon_area(&x.cut_outline) - x.area();
            assert_relative_eq!(added, 160.0 * 10.0 + 100.0 * 15.0 + 150.0, epsilon = 1e-6);
        }
        let mut s = centre_seam();
        s.lap = Lap::Both;
        let d = two_halves(s);
        let opts = PatternOptions {
            hem_allowance: 10.0,
            ..Default::default()
        };
        for x in build_pattern(&d, &unit(2), &opts).unwrap() {
            assert_relative_eq!(
                polygon_area(&x.cut_outline),
                (100.0 + 6.0 + 10.0) * (160.0 + 20.0),
                epsilon = 1e-6
            );
        }
    }

    #[test]
    fn ease_reduces_around_and_along() {
        let d = two_halves(centre_seam());
        let red = vec![Reduction::new(0.8, 0.5); 2];
        let p = build_pattern(&d, &red, &PatternOptions::default()).unwrap();
        assert_relative_eq!(p[0].area(), 80.0 * 160.0 * 0.9, epsilon = 1e-6);
        assert_relative_eq!(p[0].flat_length(1), 144.0, epsilon = 1e-9);
        assert_relative_eq!(p[0].body_length(1), 160.0, epsilon = 1e-9);
    }

    #[test]
    fn reduction_follows_the_pieces_up_direction() {
        // A 100 × 160 rectangle whose up the body is the page's −x: its
        // 100 mm width runs along the body, its 160 mm height around it.
        let mut d = two_halves(centre_seam());
        d.seams.clear();
        d.pieces.truncate(1);
        let red = Reduction::new(0.8, 0.5).up_deg(90.0);
        let p = build_pattern(&d, &[red], &PatternOptions::default()).unwrap();
        let (lo, hi) = geom::bounds(&p[0].seam_line);
        assert_relative_eq!(hi.x - lo.x, 90.0, epsilon = 1e-9);
        assert_relative_eq!(hi.y - lo.y, 128.0, epsilon = 1e-9);

        // Drawn turned by 35° with up turned by 35°, a piece reduces to
        // the same shape as drawn upright: every edge the same flat length.
        let up = Reduction::new(0.8, 0.4);
        let upright = build_pattern(&d, &[up], &PatternOptions::default()).unwrap();
        let (c, s) = (35f64.to_radians().cos(), 35f64.to_radians().sin());
        for n in &mut d.pieces[0].nodes {
            (n.x, n.y) = (c * n.x - s * n.y, s * n.x + c * n.y);
        }
        let turned = build_pattern(&d, &[up.up_deg(35.0)], &PatternOptions::default()).unwrap();
        for k in 0..4 {
            assert_relative_eq!(
                turned[0].flat_length(k),
                upright[0].flat_length(k),
                epsilon = 1e-9
            );
        }
        assert_relative_eq!(turned[0].area(), upright[0].area(), epsilon = 1e-6);
        assert_relative_eq!(up.factor(DVec2::X), 0.8, epsilon = 1e-12);
        assert_relative_eq!(up.factor(DVec2::Y), 0.92, epsilon = 1e-12);
    }

    #[test]
    fn notches_match_across_the_seam() {
        // A marked node 40 mm up "a"'s right edge, plus automatic notches
        // every ~50 mm of the 160 mm seam (at 53.3 and 106.7 mm).
        let mut d = two_halves(centre_seam());
        d.pieces[0]
            .nodes
            .insert(2, Node::new("m", 100.0, 40.0, NodeKind::Smooth));
        d.pieces[0].nodes[2].notch = true;
        let p = build_pattern(&d, &unit(2), &PatternOptions::default()).unwrap();
        assert_eq!(p[0].notches.len(), 3);
        assert_eq!(p[1].notches.len(), 3);
        let mut ys: Vec<f64> = p[0].notches.iter().map(|n| n.position.y).collect();
        ys.sort_by(f64::total_cmp);
        for (y, e) in ys.iter().zip([40.0, 160.0 / 3.0, 320.0 / 3.0]) {
            assert_relative_eq!(*y, e, epsilon = 1e-6);
        }
        for na in &p[0].notches {
            let nb = p[1].notches.iter().find(|n| n.number == na.number).unwrap();
            assert_relative_eq!(na.position.y, nb.position.y, epsilon = 1e-6);
            // Outward normals: +x on "a"'s right edge, −x on "b"'s left.
            assert_relative_eq!(na.normal.x, 1.0, epsilon = 1e-9);
            assert_relative_eq!(nb.normal.x, -1.0, epsilon = 1e-9);
        }
        // Without `reverse` the seam would be crossed: the marked notch
        // lands at the other end of "b".
        d.seams[0].reverse = false;
        let p = build_pattern(&d, &unit(2), &PatternOptions::default()).unwrap();
        let first = p[0]
            .notches
            .iter()
            .find(|n| (n.position.y - 40.0).abs() < 1e-6)
            .unwrap();
        let other = p[1]
            .notches
            .iter()
            .find(|n| n.number == first.number)
            .unwrap();
        assert_relative_eq!(other.position.y, 120.0, epsilon = 1e-6);
    }

    #[test]
    fn folded_pieces_unfold_with_mirrored_notches() {
        // The left edge of "a" is a fold: printed 200 mm wide; the seam on
        // its right edge appears on both sides, notches and all.
        let mut d = two_halves(centre_seam());
        d.pieces[0].nodes[3].edge.fold = true;
        let p = build_pattern(&d, &unit(2), &PatternOptions::default()).unwrap();
        assert!(p[0].folded);
        assert_relative_eq!(p[0].area(), 200.0 * 160.0, epsilon = 1e-6);
        assert_eq!(p[0].notches.len(), 2 * p[1].notches.len());
        let xs: Vec<f64> = p[0].notches.iter().map(|n| n.position.x).collect();
        assert!(xs.iter().any(|x| (x - 100.0).abs() < 1e-9));
        assert!(xs.iter().any(|x| (x + 100.0).abs() < 1e-9));
        assert!(p[0]
            .notches
            .iter()
            .all(|n| (n.normal.x.abs() - 1.0).abs() < 1e-9
                && n.normal.x.signum() == n.position.x.signum()));
        let [f0, f1] = p[0].fold_line.unwrap();
        assert_eq!((f0.x, f1.x), (0.0, 0.0));
    }

    #[test]
    fn trims_cover_every_copy() {
        // Top edges trimmed: "a" folded (2 × 100 mm) and "b" a pair
        // (2 × 100 mm) give one 400 mm neckline strip.
        let mut d = two_halves(centre_seam());
        d.pieces[0].nodes[3].edge.fold = true;
        d.pieces[1].pair = true;
        for piece in &mut d.pieces {
            piece.nodes[2].edge.name = "neckline".into();
            piece.nodes[2].edge.finish = Some(EdgeFinish::Trim {
                width_mm: 12.0,
                ease: 0.9,
            });
        }
        let p = build_pattern(&d, &unit(2), &PatternOptions::default()).unwrap();
        let t = trim_strips(&d, &p);
        assert_eq!(t.len(), 1);
        assert_relative_eq!(t[0].edge_length_mm, 400.0, epsilon = 1e-9);
        assert_relative_eq!(t[0].length_mm, 360.0, epsilon = 1e-9);
        assert_eq!(t[0].pieces, vec![0, 1]);
    }

    #[test]
    fn clockwise_pieces_are_turned_round() {
        let mut d = two_halves(centre_seam());
        d.pieces[1].nodes.reverse();
        // "b"'s left edge now starts at "bl" and runs upwards.
        d.seams[0].b = edge("b", "bl");
        d.seams[0].reverse = false;
        let p = build_pattern(&d, &unit(2), &PatternOptions::default()).unwrap();
        assert!(geom::signed_area(&p[1].seam_line) > 0.0);
        assert!(p[1].notches.iter().all(|n| (n.normal.x + 1.0).abs() < 1e-9));
    }
}
