//! A garment designed flat: pieces drawn in 2D and the seams that join them.
//!
//! ## Pieces
//! A [`Piece`] is a closed outline through its [`Node`]s, in millimetres at
//! body size (before the ease reduces it), `x` across the body and `y` up
//! it: `y` is the height above the floor for pieces drafted from the body,
//! so the editor can show the body's levels across them. The outline runs
//! through the nodes as described in [`crate::geom`]: sharp at a
//! [`NodeKind::Corner`], smooth elsewhere.
//!
//! The garment is designed as its right half, as in flat pattern drafting.
//! A piece is either
//! - **cut on the fold**: one edge is marked as a fold, and the piece is
//!   unfolded about it into one symmetric piece (a centre front, say), or
//! - **cut as a pair** ([`Piece::pair`]): the piece and its mirror image,
//!   one for each side, or
//! - neither: cut once, as drawn.
//!
//! ## Edges
//! The outline is split into **edges** at its corner and joint nodes. An
//! edge is named by the node it starts at (following the outline), which
//! also holds its properties ([`EdgeProps`]): a name, whether it is a
//! fold, and how it is finished if it is left open. At a smooth or joint
//! node next to a fold, the outline meets the fold square, so the unfolded
//! piece has no point there.
//!
//! ## Seams
//! A [`Seam`] glues edge `a` to edge `b`. Unless `reverse` is set, the
//! start of `a` meets the start of `b`. A seam whose two sides are the same
//! edge of a pair piece joins that edge to its mirror image (a centre back
//! seam, say).

use crate::body::BodySpec;
use crate::geom::{self, Bezier};
use glam::DVec2;

/// How the outline passes through a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum NodeKind {
    /// A sharp point; an edge ends here.
    #[default]
    Corner,
    /// A smooth point along an edge.
    Smooth,
    /// A smooth point where one edge ends and the next begins.
    Joint,
}

impl NodeKind {
    /// Whether an edge ends at a node of this kind.
    pub fn is_vertex(self) -> bool {
        self != NodeKind::Smooth
    }
}

/// How an open edge of the garment (neckline, hem, cuff…) is finished.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase", tag = "kind"))]
pub enum EdgeFinish {
    /// Cut on the line and left as it is (latex does not fray).
    #[default]
    Raw,
    /// An allowance `width_mm` wide, folded to the inside and glued.
    Hem { width_mm: f64 },
    /// A separate strip `width_mm` wide glued along the edge, cut to
    /// `ease` × the edge's length so it hugs the body.
    Trim { width_mm: f64, ease: f64 },
}

/// Properties of the edge that starts at a node.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct EdgeProps {
    /// Shown on the pattern; open edges with the same name and a trim
    /// finish share one trim strip.
    pub name: String,
    /// The piece is cut on the fold along this (straight) edge.
    pub fold: bool,
    /// Finish of an open edge; `None` takes the pattern's hem allowance.
    pub finish: Option<EdgeFinish>,
}

impl EdgeProps {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A point the outline passes through.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Node {
    /// Unique within the piece.
    pub id: String,
    pub x: f64,
    pub y: f64,
    #[cfg_attr(feature = "serde", serde(default))]
    pub kind: NodeKind,
    /// A matching notch here, on this edge and the edge it is sewn to.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_false"))]
    pub notch: bool,
    /// The edge that starts here (for corner and joint nodes).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "EdgeProps::is_default")
    )]
    pub edge: EdgeProps,
}

#[cfg(feature = "serde")]
fn is_false(b: &bool) -> bool {
    !*b
}

impl Node {
    pub fn new(id: impl Into<String>, x: f64, y: f64, kind: NodeKind) -> Self {
        Self {
            id: id.into(),
            x,
            y,
            kind,
            notch: false,
            edge: EdgeProps::default(),
        }
    }

    pub fn pos(&self) -> DVec2 {
        DVec2::new(self.x, self.y)
    }
}

/// One pattern piece.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Piece {
    /// Unique within the design.
    pub id: String,
    pub name: String,
    pub nodes: Vec<Node>,
    /// Where the editor shows the piece (an offset added to every node);
    /// it does not change the pattern.
    pub at: [f64; 2],
    /// Cut twice, the second time as a mirror image.
    pub pair: bool,
    /// Latex sheet to cut it from; `None` for the project's default sheet.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub sheet: Option<String>,
    /// Which way is up the body on this piece, in degrees anticlockwise from
    /// the page's up (0 for a piece drawn upright). The lengthwise
    /// reduction is applied in this direction, the reduction around the
    /// body across it (see [`Reduction`](crate::pattern::Reduction)).
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "is_zero"))]
    pub up_deg: f64,
}

#[cfg(feature = "serde")]
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// An edge of a piece: from a vertex node to the next one along the
/// outline (`end == start` when the piece has a single vertex).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub start: usize,
    pub end: usize,
}

impl Piece {
    /// Index of the node with this id.
    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// The outline's edges, in order. A piece with no vertex at all is one
    /// edge starting at its first node.
    pub fn edges(&self) -> Vec<Edge> {
        let n = self.nodes.len();
        let mut starts: Vec<usize> = (0..n).filter(|&i| self.nodes[i].kind.is_vertex()).collect();
        if starts.is_empty() && n > 0 {
            starts.push(0);
        }
        (0..starts.len())
            .map(|k| Edge {
                start: starts[k],
                end: starts[(k + 1) % starts.len()],
            })
            .collect()
    }

    /// The edge each outline segment (node `i` to node `i + 1`) belongs to.
    pub fn segment_edges(&self) -> Vec<usize> {
        let n = self.nodes.len();
        let edges = self.edges();
        let mut out = vec![0; n];
        for (k, e) in edges.iter().enumerate() {
            let mut i = e.start;
            loop {
                out[i] = k;
                i = (i + 1) % n;
                if i == e.end {
                    break;
                }
            }
        }
        out
    }

    /// Index of the edge starting at the node with this id.
    pub fn edge_at(&self, node_id: &str) -> Option<usize> {
        let i = self.node_index(node_id)?;
        self.edges().iter().position(|e| e.start == i)
    }

    /// Properties of edge `k`.
    pub fn edge_props(&self, k: usize) -> &EdgeProps {
        &self.nodes[self.edges()[k].start].edge
    }

    /// Display name of edge `k`: its own name, else its position.
    pub fn edge_name(&self, k: usize) -> String {
        let e = self.edge_props(k);
        if e.name.is_empty() {
            format!("edge {}", k + 1)
        } else {
            e.name.clone()
        }
    }

    /// The fold edges.
    pub fn folds(&self) -> Vec<usize> {
        let edges = self.edges();
        (0..edges.len())
            .filter(|&k| self.nodes[edges[k].start].edge.fold)
            .collect()
    }

    /// The line of the fold edge `k`: its two end nodes.
    fn fold_line(&self, k: usize) -> (DVec2, DVec2) {
        let e = self.edges()[k];
        (self.nodes[e.start].pos(), self.nodes[e.end].pos())
    }

    /// One Bézier per outline segment (node `i` to node `i + 1`).
    pub fn beziers(&self) -> Vec<Bezier> {
        let n = self.nodes.len();
        if n < 2 {
            return Vec::new();
        }
        let seg_edge = self.segment_edges();
        let edges = self.edges();
        let is_fold = |seg: usize| self.nodes[edges[seg_edge[seg]].start].edge.fold;
        let p = |i: usize| self.nodes[i % n].pos();
        (0..n)
            .map(|i| {
                let (a, b) = (p(i), p(i + 1));
                if is_fold(i) {
                    return geom::line(a, b);
                }
                let (ka, kb) = (self.nodes[i].kind, self.nodes[(i + 1) % n].kind);
                let prev = (i + n - 1) % n;
                let next = (i + 1) % n;
                let p0 = if ka == NodeKind::Corner {
                    2.0 * a - b
                } else if ka.is_vertex() && is_fold(prev) {
                    let (f0, f1) = self.fold_line(seg_edge[prev]);
                    geom::reflect(b, f0, f1)
                } else {
                    p(i + n - 1)
                };
                let p3 = if kb == NodeKind::Corner {
                    2.0 * b - a
                } else if kb.is_vertex() && is_fold(next) {
                    let (f0, f1) = self.fold_line(seg_edge[next]);
                    geom::reflect(a, f0, f1)
                } else {
                    p(i + 2)
                };
                geom::segment(p0, a, b, p3)
            })
            .collect()
    }

    /// Sampled points of each outline segment, from its first node to its
    /// last (inclusive).
    pub fn segment_points(&self) -> Vec<Vec<DVec2>> {
        self.beziers().iter().map(geom::sample_bezier).collect()
    }

    /// Sampled points of edge `k`, from its start node to its end node.
    pub fn edge_points(&self, k: usize) -> Vec<DVec2> {
        let segs = self.segment_points();
        self.edge_points_from(&segs, k)
    }

    /// As [`Self::edge_points`], from already sampled segments.
    pub fn edge_points_from(&self, segs: &[Vec<DVec2>], k: usize) -> Vec<DVec2> {
        let n = self.nodes.len();
        let e = self.edges()[k];
        let mut pts: Vec<DVec2> = Vec::new();
        let mut i = e.start;
        loop {
            let s = &segs[i];
            if pts.is_empty() {
                pts.extend_from_slice(s);
            } else {
                pts.extend_from_slice(&s[1..]);
            }
            i = (i + 1) % n;
            if i == e.end {
                break;
            }
        }
        pts
    }

    /// Length of edge `k` (mm, body size).
    pub fn edge_length(&self, k: usize) -> f64 {
        geom::polyline_length(&self.edge_points(k))
    }

    /// The closed outline (without repeating the first point), with the
    /// edge each of its segments belongs to.
    pub fn outline(&self) -> (Vec<DVec2>, Vec<usize>) {
        let segs = self.segment_points();
        let seg_edge = self.segment_edges();
        let mut pts = Vec::new();
        let mut owner = Vec::new();
        for (i, s) in segs.iter().enumerate() {
            for q in &s[..s.len() - 1] {
                pts.push(*q);
                owner.push(seg_edge[i]);
            }
        }
        (pts, owner)
    }

    /// Fraction of edge `k`'s length at which each of its inner nodes with
    /// a notch lies.
    pub fn notch_fractions(&self, k: usize) -> Vec<f64> {
        let n = self.nodes.len();
        let e = self.edges()[k];
        let segs = self.segment_points();
        let total = geom::polyline_length(&self.edge_points_from(&segs, k));
        let mut out = Vec::new();
        let mut acc = 0.0;
        let mut i = e.start;
        loop {
            acc += geom::polyline_length(&segs[i]);
            i = (i + 1) % n;
            if i == e.end {
                break;
            }
            if self.nodes[i].notch && total > 0.0 {
                out.push(acc / total);
            }
        }
        out
    }
}

/// One side of a seam: the edge starting at node `node` of piece `piece`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EdgeRef {
    pub piece: String,
    pub node: String,
}

/// How a seam is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum SeamFinish {
    /// Lap seam: one side's allowance glued under the other's edge.
    #[default]
    Glued,
    /// A zip: each side gets a strip to glue the zip tape under.
    Zip,
}

/// Which side of a glued seam carries the lap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Lap {
    #[default]
    A,
    B,
    Both,
    /// Butted edges (glued over a backing strip).
    None,
}

/// Two edges glued together.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Seam {
    pub id: String,
    pub name: String,
    pub a: EdgeRef,
    pub b: EdgeRef,
    /// The start of `a` meets the end of `b` (instead of its start).
    pub reverse: bool,
    pub finish: SeamFinish,
    pub lap: Lap,
}

impl Seam {
    /// Whether the seam joins an edge to its own mirror image.
    pub fn joins_mirror(&self) -> bool {
        self.a == self.b
    }
}

/// The template a design was drafted from, and for which body, so the
/// design can follow new measurements (see
/// [`redraft`](crate::blocks::redraft)).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Block {
    pub template: String,
    pub body: BodySpec,
}

/// A garment design.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Design {
    pub pieces: Vec<Piece>,
    pub seams: Vec<Seam>,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub block: Option<Block>,
}

impl Design {
    pub fn piece_index(&self, id: &str) -> Option<usize> {
        self.pieces.iter().position(|p| p.id == id)
    }

    /// The piece and edge index an edge reference names.
    pub fn resolve(&self, r: &EdgeRef) -> Option<(usize, usize)> {
        let p = self.piece_index(&r.piece)?;
        let e = self.pieces[p].edge_at(&r.node)?;
        Some((p, e))
    }

    /// The seam on edge `k` of piece `p`, if any.
    pub fn seam_on(&self, p: usize, k: usize) -> Option<usize> {
        self.seams.iter().position(|s| {
            [&s.a, &s.b]
                .into_iter()
                .any(|r| self.resolve(r) == Some((p, k)))
        })
    }

    /// Problems that stop a pattern being made, as messages.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.pieces.is_empty() {
            out.push("the design has no pieces".to_string());
        }
        let mut ids = std::collections::HashSet::new();
        for p in &self.pieces {
            if !ids.insert(&p.id) {
                out.push(format!("two pieces have the id '{}'", p.id));
            }
            if p.nodes.len() < 3 {
                out.push(format!("piece '{}' needs at least three points", p.name));
                continue;
            }
            let mut nids = std::collections::HashSet::new();
            for n in &p.nodes {
                if !nids.insert(&n.id) {
                    out.push(format!(
                        "piece '{}' has two points with the id '{}'",
                        p.name, n.id
                    ));
                }
                if !(n.x.is_finite() && n.y.is_finite()) {
                    out.push(format!(
                        "piece '{}' has a point at an invalid position",
                        p.name
                    ));
                }
            }
            let folds = p.folds();
            if folds.len() > 1 {
                out.push(format!(
                    "piece '{}' has {} fold lines; a piece can be cut on one fold only",
                    p.name,
                    folds.len()
                ));
            }
            if let Some(&k) = folds.first() {
                let e = p.edges()[k];
                if e.start == e.end {
                    out.push(format!(
                        "piece '{}': the fold must be a straight edge between two corners",
                        p.name
                    ));
                }
            }
            let (pts, _) = p.outline();
            if geom::signed_area(&pts).abs() < 1.0 {
                out.push(format!("piece '{}' has no area", p.name));
            } else if geom::first_self_crossing(&pts).is_some() {
                out.push(format!("piece '{}': its outline crosses itself", p.name));
            }
        }
        let mut used: Vec<(usize, usize)> = Vec::new();
        for s in &self.seams {
            let name = if s.name.is_empty() { &s.id } else { &s.name };
            let (Some(a), Some(b)) = (self.resolve(&s.a), self.resolve(&s.b)) else {
                out.push(format!(
                    "seam '{name}' refers to an edge that no longer exists; delete it or join the edges again"
                ));
                continue;
            };
            for (p, k) in if a == b { vec![a] } else { vec![a, b] } {
                if used.contains(&(p, k)) {
                    out.push(format!(
                        "edge '{}' of piece '{}' is in two seams",
                        self.pieces[p].edge_name(k),
                        self.pieces[p].name
                    ));
                }
                used.push((p, k));
                if self.pieces[p].edge_props(k).fold {
                    out.push(format!(
                        "seam '{name}': edge '{}' of piece '{}' is a fold, so it cannot be sewn",
                        self.pieces[p].edge_name(k),
                        self.pieces[p].name
                    ));
                }
            }
            if a == b && !self.pieces[a.0].pair {
                out.push(format!(
                    "seam '{name}' joins an edge of piece '{}' to its mirror image, but the piece is \
                     not cut as a pair",
                    self.pieces[a.0].name
                ));
            }
        }
        out
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use approx::assert_relative_eq;

    /// A `w × h` rectangle with its lower-left corner at the origin; its
    /// edges start at nodes "bl" (bottom), "br" (right), "tr" (top), "tl"
    /// (left).
    pub fn rect(id: &str, w: f64, h: f64) -> Piece {
        Piece {
            id: id.into(),
            name: id.into(),
            nodes: vec![
                Node::new("bl", 0.0, 0.0, NodeKind::Corner),
                Node::new("br", w, 0.0, NodeKind::Corner),
                Node::new("tr", w, h, NodeKind::Corner),
                Node::new("tl", 0.0, h, NodeKind::Corner),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn rectangle_edges_and_lengths() {
        let p = rect("r", 100.0, 50.0);
        assert_eq!(p.edges().len(), 4);
        assert_relative_eq!(p.edge_length(0), 100.0, epsilon = 1e-9);
        assert_relative_eq!(p.edge_length(3), 50.0, epsilon = 1e-9);
        let (pts, owner) = p.outline();
        assert_eq!(pts.len(), owner.len());
        assert_relative_eq!(geom::signed_area(&pts), 5000.0, epsilon = 1e-6);
        assert_eq!(p.edge_at("tr"), Some(2));
    }

    #[test]
    fn smooth_and_joint_nodes() {
        // A smooth node in the middle of the top edge: still 4 edges, and
        // the top bulges up through it.
        let mut p = rect("r", 100.0, 50.0);
        p.nodes
            .insert(3, Node::new("top", 50.0, 70.0, NodeKind::Smooth));
        assert_eq!(p.edges().len(), 4);
        assert_eq!(p.segment_edges(), vec![0, 1, 2, 2, 3]);
        assert!(p.edge_length(2) > 100.0);
        // As a joint it splits the top into two edges.
        p.nodes[3].kind = NodeKind::Joint;
        assert_eq!(p.edges().len(), 5);
        p.nodes[3].notch = true;
        p.nodes[3].kind = NodeKind::Smooth;
        let f = p.notch_fractions(2);
        assert_eq!(f.len(), 1);
        assert_relative_eq!(f[0], 0.5, epsilon = 1e-9);
    }

    #[test]
    fn outlines_meet_a_fold_square() {
        // Left edge is a fold; the top-left node is a joint, so the top
        // curve leaves it horizontally (perpendicular to the fold).
        let mut p = rect("r", 100.0, 50.0);
        p.nodes[3].edge.fold = true;
        p.nodes[3].kind = NodeKind::Joint;
        p.nodes[2].y = 80.0;
        let b = p.beziers();
        // Segment 2 runs from (100, 80) to the joint at (0, 50).
        let t = b[2][3] - b[2][2];
        assert_relative_eq!(t.y, 0.0, epsilon = 1e-9);
        assert!(p.folds() == vec![3]);
    }

    #[test]
    fn problems_are_found() {
        let mut d = Design {
            pieces: vec![rect("a", 10.0, 10.0), rect("a", 10.0, 10.0)],
            ..Default::default()
        };
        assert!(d.problems()[0].contains("two pieces"));
        d.pieces[1].id = "b".into();
        d.seams.push(Seam {
            id: "s".into(),
            a: EdgeRef {
                piece: "a".into(),
                node: "br".into(),
            },
            b: EdgeRef {
                piece: "a".into(),
                node: "br".into(),
            },
            ..Default::default()
        });
        assert!(d.problems()[0].contains("not cut as a pair"));
        d.pieces[0].pair = true;
        assert!(d.problems().is_empty(), "{:?}", d.problems());
        d.seams[0].b.node = "gone".into();
        assert!(d.problems()[0].contains("no longer exists"));
    }
}
