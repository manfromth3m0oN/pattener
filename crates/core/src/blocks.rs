//! Templates: garments drafted flat from the body's measurements.
//!
//! Every template is drafted at body size (the ease is applied when the
//! pattern is made), as the right half of the garment, with `x` measured
//! from the centre line and `y` the height above the floor (see
//! [`Levels`](crate::body::Levels)).
//!
//! ## Torso
//! At each level (underarm, bust, underbust, waist, hip) the side seam sits
//! a quarter of the girth from the centre line, moved towards the back by
//! the body's [`side_offset`](crate::body::Body::side_offset) on the front
//! and back alike, so the two side seams have the same shape and length.
//! The neckline is a quarter ellipse from the side of the neck (a sixth of
//! the neck girth out) to the centre, deeper in front; the shoulder seam
//! slopes down to half the shoulder width; the armhole runs in to the
//! across-front or across-back width and out to the side seam at the
//! underarm. Latex has no darts, so a women's front is lengthened at the
//! centre by [`bust_extra`](crate::body::Body::bust_extra).
//!
//! ## Sleeves
//! The upper arm girth sets the width under the arm. The cap's height and
//! the position of its top are solved (by bisection) so that its front and
//! back curves are exactly as long as the front and back armholes.
//!
//! ## Legs
//! The crotch extensions take up whatever the thigh girth needs beyond half
//! the hips, a third in front and two thirds behind. Down the leg the front
//! and back widths are `(G ± Δ)/2` for the girth `G` at the thigh, knee,
//! calf and ankle, with `Δ` constant: so the front and back inside-leg
//! seams, and the side seams, have the same shape and length. Leggings join
//! the front and back of each leg along the outside of the leg in one
//! piece.
//!
//! ## Following new measurements
//! [`redraft`] moves a design drafted from a template to new measurements
//! while keeping its edits: each point the template drew moves by as much
//! as the template's own point moved (whichever piece it is now in, so a
//! piece split from a template piece follows too); points added since move
//! with their neighbours.

use crate::body::{interpolate, Body, BodySpec, Measure};
use crate::design::{
    Block, Design, EdgeFinish, EdgeRef, Lap, Node, NodeKind, Piece, Seam, SeamFinish,
};
use crate::error::{Error, Result};
use crate::geom;
use glam::DVec2;
use std::collections::HashMap;
use NodeKind::{Corner, Joint, Smooth};

/// A template in the library.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TemplateInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

/// The templates, in the order to offer them.
pub const TEMPLATES: [TemplateInfo; 7] = [
    TemplateInfo {
        id: "top",
        name: "Sleeveless top",
        description: "Front and back, both cut on the fold; shoulder and side seams. Neckline and \
                      armholes bound with trim strips, folded hem.",
    },
    TemplateInfo {
        id: "top-sleeves",
        name: "Top with long sleeves",
        description: "Front and back cut on the fold, and a pair of set-in sleeves whose caps are \
                      drafted to the armholes; underarm seams.",
    },
    TemplateInfo {
        id: "cincher",
        name: "Waist cincher",
        description: "Underbust to high hip: front on the fold, back pair with a back zip; side \
                      seams; top and bottom bound with trim strips.",
    },
    TemplateInfo {
        id: "skirt",
        name: "Pencil skirt",
        description:
            "Waist to just below the knee: front on the fold, back pair with a zip at the \
                      top of the centre back seam; folded waist.",
    },
    TemplateInfo {
        id: "leggings",
        name: "Leggings",
        description: "One piece per leg (front and back in one), joined at centre front and back; \
                      inside leg seams; folded waist.",
    },
    TemplateInfo {
        id: "catsuit",
        name: "Sleeveless catsuit",
        description: "Fronts and backs as pairs: centre front seam, back zip, shoulder, side and \
                      inside leg seams.",
    },
    TemplateInfo {
        id: "catsuit-sleeves",
        name: "Catsuit with sleeves",
        description: "The catsuit with a pair of set-in long sleeves drafted to its armholes.",
    },
];

fn trim() -> Option<EdgeFinish> {
    Some(EdgeFinish::Trim {
        width_mm: 15.0,
        ease: 0.9,
    })
}

fn hem(w: f64) -> Option<EdgeFinish> {
    Some(EdgeFinish::Hem { width_mm: w })
}

/// Builds a piece node by node.
struct Draft {
    prefix: &'static str,
    nodes: Vec<Node>,
}

impl Draft {
    fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
            nodes: Vec::new(),
        }
    }

    /// A node with no edge of its own.
    fn at(&mut self, id: &str, x: f64, y: f64, kind: NodeKind) -> &mut Self {
        self.nodes
            .push(Node::new(format!("{}-{id}", self.prefix), x, y, kind));
        self
    }

    /// A node starting an edge.
    fn edge(
        &mut self,
        id: &str,
        x: f64,
        y: f64,
        kind: NodeKind,
        name: &str,
        finish: Option<EdgeFinish>,
    ) -> &mut Self {
        self.at(id, x, y, kind);
        let e = &mut self.nodes.last_mut().unwrap().edge;
        e.name = name.into();
        e.finish = finish;
        self
    }

    /// The last node starts a fold.
    fn fold(&mut self) -> &mut Self {
        self.nodes.last_mut().unwrap().edge.fold = true;
        self
    }

    fn id(&self, id: &str) -> String {
        format!("{}-{id}", self.prefix)
    }

    fn piece(&mut self, name: &str, pair: bool) -> Piece {
        Piece {
            id: self.prefix.into(),
            name: name.into(),
            nodes: std::mem::take(&mut self.nodes),
            at: [0.0, 0.0],
            pair,
            sheet: None,
            up_deg: 0.0,
        }
    }
}

/// Seam between edges named by their start nodes' ids.
fn seam(name: &str, a: (&str, &str), b: (&str, &str), reverse: bool, finish: SeamFinish) -> Seam {
    Seam {
        id: name.replace(' ', "-"),
        name: name.into(),
        a: EdgeRef {
            piece: a.0.into(),
            node: a.1.into(),
        },
        b: EdgeRef {
            piece: b.0.into(),
            node: b.1.into(),
        },
        reverse,
        finish,
        lap: Lap::A,
    }
}

/// How the centre of a torso piece is cut.
#[derive(Clone, Copy, PartialEq)]
enum Centre {
    /// On the fold.
    Fold,
    /// As a pair joined by a centre seam.
    Seam,
    /// As a pair with a zip from the top to `y`, then a seam.
    ZipTo(f64),
}

/// Side seam position on the front (`+offset`) or back.
fn side_x(b: &Body, y: f64, front: bool) -> f64 {
    let d = b.side_offset();
    b.torso_quarter(y) + if front { d } else { -d }
}

/// Upper part of a torso piece, from the underarm over the shoulder and
/// round the neckline to the centre: appended to `d`, the underarm node
/// first. Returns the id of the centre neck node.
fn upper(b: &Body, d: &mut Draft, front: bool, centre: Centre) -> String {
    let l = b.levels;
    let nw = b.neck_half_width();
    let sw = b.get(Measure::Shoulder) / 2.0;
    let xu = side_x(b, l.underarm, front);
    let depth = l.shoulder - l.underarm;
    let across = sw * if front { 0.86 } else { 0.93 };
    d.edge("underarm", xu, l.underarm, Corner, "armhole", trim());
    d.at(
        "arm-low",
        across + 0.25 * (xu - across),
        l.underarm + 0.15 * depth,
        Smooth,
    );
    d.at("arm-mid", across, l.underarm + 0.5 * depth, Smooth);
    d.edge("shoulder", sw, l.shoulder, Corner, "shoulder", None);
    d.edge("neck-side", nw, l.neck_side, Corner, "neckline", trim());
    let neck_depth = if front {
        b.get(Measure::Neck) / 5.0 + 5.0
    } else {
        l.neck_side - l.nape
    };
    let r = std::f64::consts::FRAC_1_SQRT_2;
    d.at("neck-mid", r * nw, l.neck_side - r * neck_depth, Smooth);
    let name = if front { "centre front" } else { "centre back" };
    match centre {
        Centre::Fold => {
            d.edge(
                "centre-neck",
                0.0,
                l.neck_side - neck_depth,
                Joint,
                name,
                None,
            )
            .fold();
        }
        Centre::Seam | Centre::ZipTo(_) => {
            d.edge(
                "centre-neck",
                0.0,
                l.neck_side - neck_depth,
                Corner,
                name,
                None,
            );
        }
    }
    d.id("centre-neck")
}

/// Smooth side-seam nodes at the torso levels strictly between `lo` and
/// `hi`, from the bottom up.
fn side_nodes(b: &Body, d: &mut Draft, front: bool, lo: f64, hi: f64) {
    let l = b.levels;
    for (id, y) in [
        ("side-hip", l.hip),
        ("side-waist", l.waist),
        ("side-underbust", l.underbust),
        ("side-bust", l.bust),
    ] {
        if y > lo + 15.0 && y < hi - 15.0 {
            d.at(id, side_x(b, y, front), y, Smooth);
        }
    }
}

/// A top's front or back, hem at `hem_y`.
fn bodice(b: &Body, front: bool, hem_y: f64, centre: Centre) -> Piece {
    let mut d = Draft::new(if front { "front" } else { "back" });
    let drop = if front { b.bust_extra() } else { 0.0 };
    let kind = if centre == Centre::Fold {
        Joint
    } else {
        Corner
    };
    d.edge("centre-hem", 0.0, hem_y - drop, kind, "hem", hem(15.0));
    d.edge(
        "side-hem",
        side_x(b, hem_y, front),
        hem_y,
        Corner,
        "side",
        None,
    );
    side_nodes(b, &mut d, front, hem_y, b.levels.underarm);
    upper(b, &mut d, front, centre);
    d.piece(if front { "front" } else { "back" }, centre != Centre::Fold)
}

/// A set-in sleeve whose cap halves are `front_len` and `back_len` long.
fn sleeve(b: &Body, front_len: f64, back_len: f64) -> Result<Piece> {
    let w = b.get(Measure::UpperArm);
    let wrist = b.get(Measure::Wrist);
    let elbow = 0.5 * (w + wrist) + 0.05 * w;
    let arm = b.get(Measure::ArmLength);
    if front_len + back_len <= w * 1.02 {
        return Err(Error::Body(format!(
            "the upper arm ({w:.0} mm) is as wide as the whole armhole ({:.0} mm); \
             check the upper arm and bust measurements",
            front_len + back_len
        )));
    }
    let build = |cap: f64, top: f64| -> Piece {
        let mut d = Draft::new("sleeve");
        let f = |u: f64| if u < 0.5 { 0.2 } else { 0.85 };
        let (yw, ye) = (cap - arm, cap - 0.56 * arm);
        d.edge("wrist-front", -wrist / 2.0, yw, Corner, "cuff", None);
        d.edge("wrist-back", wrist / 2.0, yw, Corner, "underarm back", None);
        d.at("elbow-back", elbow / 2.0, ye, Smooth);
        d.edge("underarm-back", w / 2.0, 0.0, Corner, "cap back", None);
        for u in [0.3, 0.7] {
            let id = if u < 0.5 {
                "cap-back-low"
            } else {
                "cap-back-high"
            };
            d.at(id, w / 2.0 - u * (w / 2.0 - top), cap * f(u), Smooth);
        }
        d.edge("cap-top", top, cap, Joint, "cap front", None);
        for u in [0.7, 0.3] {
            let id = if u < 0.5 {
                "cap-front-low"
            } else {
                "cap-front-high"
            };
            d.at(id, -w / 2.0 + u * (top + w / 2.0), cap * f(u), Smooth);
        }
        d.edge(
            "underarm-front",
            -w / 2.0,
            0.0,
            Corner,
            "underarm front",
            None,
        );
        d.at("elbow-front", -elbow / 2.0, ye, Smooth);
        d.piece("sleeve", true)
    };
    // Cap front and back lengths: the edges starting at the cap top and at
    // the back underarm.
    let lengths = |p: &Piece| {
        (
            p.edge_length(p.edge_at("sleeve-cap-top").unwrap()),
            p.edge_length(p.edge_at("sleeve-underarm-back").unwrap()),
        )
    };
    let ratio = front_len / (front_len + back_len);
    let top_for = |cap: f64| {
        let (mut lo, mut hi) = (-0.35 * w, 0.35 * w);
        for _ in 0..50 {
            let mid = 0.5 * (lo + hi);
            let (lf, lb) = lengths(&build(cap, mid));
            // Moving the top backwards lengthens the front.
            if lf / (lf + lb) < ratio {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    };
    let (mut lo, mut hi) = (0.02 * w, 2.0 * w);
    for _ in 0..50 {
        let mid = 0.5 * (lo + hi);
        let (lf, lb) = lengths(&build(mid, top_for(mid)));
        if lf + lb < front_len + back_len {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let cap = 0.5 * (lo + hi);
    let mut p = build(cap, top_for(cap));
    // A notch at the top of the cap matches the shoulder seam.
    let i = p.node_index("sleeve-cap-top").unwrap();
    p.nodes[i].notch = true;
    Ok(p)
}

/// Sleeves for a design with "front" and "back" pieces: the sleeve and its
/// seams.
fn add_sleeves(b: &Body, d: &mut Design) -> Result<()> {
    let arm = |id: &str| {
        let p = &d.pieces[d.piece_index(id).unwrap()];
        p.edge_length(p.edge_at(&format!("{id}-underarm")).unwrap())
    };
    let (f, k) = (arm("front"), arm("back"));
    let mut s = sleeve(b, f, k)?;
    s.at = [0.0, b.levels.underarm];
    d.pieces.push(s);
    for p in &mut d.pieces {
        for n in &mut p.nodes {
            if n.id.ends_with("-underarm") && n.edge.name == "armhole" {
                n.edge.finish = None;
            }
        }
    }
    d.seams.extend([
        seam(
            "underarm",
            ("sleeve", "sleeve-wrist-back"),
            ("sleeve", "sleeve-underarm-front"),
            true,
            SeamFinish::Glued,
        ),
        seam(
            "front armhole",
            ("front", "front-underarm"),
            ("sleeve", "sleeve-cap-top"),
            true,
            SeamFinish::Glued,
        ),
        seam(
            "back armhole",
            ("back", "back-underarm"),
            ("sleeve", "sleeve-underarm-back"),
            false,
            SeamFinish::Glued,
        ),
    ]);
    Ok(())
}

/// Widths of the legs: see the module documentation.
struct Legs {
    /// Front and back crotch extensions beyond the centre line.
    front_ext: f64,
    back_ext: f64,
    /// Front width minus back width, at every leg level.
    delta: f64,
    /// Side-seam position (before the front/back offset) at the crotch.
    side_crotch: f64,
    /// Front leg centre line.
    front_centre: f64,
    offset: f64,
}

impl Legs {
    fn new(b: &Body) -> Self {
        let hips = b.get(Measure::Hips);
        let thigh = b.get(Measure::Thigh);
        let mut ext = thigh - 2.0 * (hips / 4.0 - 0.01 * hips);
        ext = ext.max(0.06 * hips);
        let side_crotch = (thigh - ext) / 2.0;
        let (fe, be) = (ext / 3.0, 2.0 * ext / 3.0);
        let offset = b.side_offset();
        let delta = 2.0 * offset + fe - be;
        let front_centre = side_crotch + offset - (thigh + delta) / 4.0;
        Self {
            front_ext: fe,
            back_ext: be,
            delta,
            side_crotch,
            front_centre,
            offset,
        }
    }

    /// Front or back width for girth `g`.
    fn width(&self, g: f64, front: bool) -> f64 {
        if front {
            (g + self.delta) / 2.0
        } else {
            (g - self.delta) / 2.0
        }
    }

    /// (side, inside) `x` of the front or back at a leg level of girth `g`
    /// (the crotch level uses the thigh and gives the crotch point).
    fn at(&self, g: f64, front: bool) -> (f64, f64) {
        let x = self.front_centre + self.width(g, true) / 2.0 - self.offset;
        let side = x + if front { self.offset } else { -self.offset };
        (side, side - self.width(g, front))
    }

    /// The leg levels below the crotch: id, height, girth.
    fn levels(b: &Body) -> [(&'static str, f64, f64); 3] {
        let l = b.levels;
        [
            ("knee", l.knee, b.get(Measure::Knee)),
            ("calf", l.calf, b.get(Measure::Calf)),
            ("ankle", l.ankle, b.get(Measure::Ankle)),
        ]
    }
}

/// A catsuit front or back: torso and leg in one, cut as a pair.
fn catsuit_half(b: &Body, legs: &Legs, front: bool) -> Piece {
    let l = b.levels;
    let thigh = b.get(Measure::Thigh);
    let mut d = Draft::new(if front { "front" } else { "back" });
    let lv = Legs::levels(b);
    let (ankle_side, ankle_in) = legs.at(lv[2].2, front);
    d.edge("ankle-in", ankle_in, l.ankle, Corner, "ankle", None);
    d.edge("ankle-side", ankle_side, l.ankle, Corner, "side", None);
    for &(id, y, g) in lv[..2].iter().rev() {
        d.at(&format!("side-{id}"), legs.at(g, front).0, y, Smooth);
    }
    d.at("side-crotch", legs.at(thigh, front).0, l.crotch, Smooth);
    side_nodes(b, &mut d, front, l.crotch, l.underarm);
    let centre = if front {
        Centre::Seam
    } else {
        Centre::ZipTo(l.waist - 0.5 * (l.waist - l.hip))
    };
    upper(b, &mut d, front, centre);
    if let Centre::ZipTo(y) = centre {
        let n = d.nodes.last_mut().unwrap();
        n.edge.name = "centre back zip".into();
        d.edge("zip-end", 0.0, y, Joint, "centre back", None);
    } else {
        d.at("centre-waist", 0.0, l.waist, Smooth);
    }
    let ext = if front { legs.front_ext } else { legs.back_ext };
    let start = l.hip + if front { 0.0 } else { 20.0 };
    let a = start - l.crotch;
    d.at("crotch-start", 0.0, start, Smooth);
    d.at("crotch-mid", -0.3 * ext, l.crotch + 0.3 * a, Smooth);
    d.edge("crotch", -ext, l.crotch, Corner, "inside leg", None);
    for &(id, y, g) in &lv[..2] {
        d.at(&format!("in-{id}"), legs.at(g, front).1, y, Smooth);
    }
    d.piece(if front { "front" } else { "back" }, true)
}

/// Leggings: one piece per leg, front on the left of the side line (`x =
/// 0`) and back on the right.
fn leggings_piece(b: &Body, legs: &Legs) -> Piece {
    let l = b.levels;
    let thigh = b.get(Measure::Thigh);
    let lv = Legs::levels(b);
    // Width of the front or back from the side line to its inside edge.
    let w = |g: f64, front: bool| legs.width(g, front);
    // Torso widths above the crotch, from the side to the centre line.
    let tw = |y: f64, front: bool| side_x(b, y, front);
    let side_at = |y: f64, front: bool| {
        let side_c = legs.side_crotch + if front { legs.offset } else { -legs.offset };
        interpolate(&[(l.crotch, side_c), (l.hip, tw(l.hip, front))], y)
    };
    let mut d = Draft::new("leg");
    d.edge(
        "ankle-front",
        -w(lv[2].2, true),
        l.ankle,
        Corner,
        "ankle",
        None,
    );
    d.edge(
        "ankle-back",
        w(lv[2].2, false),
        l.ankle,
        Corner,
        "inside leg back",
        None,
    );
    for &(id, y, g) in lv[..2].iter().rev() {
        d.at(&format!("back-{id}"), w(g, false), y, Smooth);
    }
    let (fe, be) = (legs.front_ext, legs.back_ext);
    d.edge(
        "back-crotch",
        w(thigh, false),
        l.crotch,
        Corner,
        "centre back",
        None,
    );
    let back_start = l.hip + 20.0;
    let a = back_start - l.crotch;
    let y = l.crotch + 0.3 * a;
    d.at("back-crotch-mid", side_at(y, false) + 0.3 * be, y, Smooth);
    d.at(
        "back-crotch-start",
        side_at(back_start, false),
        back_start,
        Smooth,
    );
    let rise = 0.08 * (l.waist - l.crotch);
    d.edge(
        "back-waist",
        tw(l.waist, false),
        l.waist + rise,
        Corner,
        "waist",
        hem(30.0),
    );
    d.edge(
        "front-waist",
        -tw(l.waist, true),
        l.waist,
        Corner,
        "centre front",
        None,
    );
    d.at("front-crotch-start", -tw(l.hip, true), l.hip, Smooth);
    let a = l.hip - l.crotch;
    let y = l.crotch + 0.3 * a;
    d.at(
        "front-crotch-mid",
        -(side_at(y, true) + 0.3 * fe),
        y,
        Smooth,
    );
    d.edge(
        "front-crotch",
        -w(thigh, true),
        l.crotch,
        Corner,
        "inside leg front",
        None,
    );
    for &(id, y, g) in &lv[..2] {
        d.at(&format!("front-{id}"), -w(g, true), y, Smooth);
    }
    d.piece("leg", true)
}

/// A skirt front or back, waist to just below the knee.
fn skirt_half(b: &Body, front: bool) -> Piece {
    let l = b.levels;
    let hem_y = l.knee - 30.0;
    let hips = b.get(Measure::Hips);
    let mut d = Draft::new(if front { "front" } else { "back" });
    let centre_kind = if front { Joint } else { Corner };
    d.edge("centre-hem", 0.0, hem_y, centre_kind, "hem", None);
    d.edge(
        "side-hem",
        side_x(b, l.hip, front) - 0.01 * hips,
        hem_y,
        Corner,
        "side",
        None,
    );
    d.at("side-hip", side_x(b, l.hip, front), l.hip, Smooth);
    d.edge(
        "side-waist",
        side_x(b, l.waist, front),
        l.waist,
        Corner,
        "waist",
        hem(25.0),
    );
    if front {
        d.edge("centre-waist", 0.0, l.waist, Joint, "centre front", None)
            .fold();
    } else {
        d.edge(
            "centre-waist",
            0.0,
            l.waist + 10.0,
            Corner,
            "centre back zip",
            None,
        );
        d.edge("zip-end", 0.0, l.hip - 30.0, Joint, "centre back", None);
    }
    d.piece(if front { "front" } else { "back" }, !front)
}

/// A cincher front or back, underbust to high hip.
fn cincher_half(b: &Body, front: bool) -> Piece {
    let l = b.levels;
    let (bot, top) = (l.waist - 0.45 * (l.waist - l.hip), l.underbust - 10.0);
    let mut d = Draft::new(if front { "front" } else { "back" });
    let centre_kind = if front { Joint } else { Corner };
    d.edge("centre-bottom", 0.0, bot, centre_kind, "bottom", trim());
    d.edge(
        "side-bottom",
        side_x(b, bot, front),
        bot,
        Corner,
        "side",
        None,
    );
    d.at("side-waist", side_x(b, l.waist, front), l.waist, Smooth);
    d.edge(
        "side-top",
        side_x(b, top, front),
        top,
        Corner,
        "top",
        trim(),
    );
    if front {
        d.edge("centre-top", 0.0, top, Joint, "centre front", None)
            .fold();
    } else {
        d.edge("centre-top", 0.0, top, Corner, "centre back zip", None);
    }
    d.piece(if front { "front" } else { "back" }, !front)
}

/// Lay pieces out left to right, 60 mm apart, keeping their heights.
fn arrange(d: &mut Design) {
    let mut x = 0.0;
    for p in &mut d.pieces {
        let pts: Vec<DVec2> = p.nodes.iter().map(|n| n.pos()).collect();
        let (lo, hi) = geom::bounds(&pts);
        p.at[0] = x - lo.x;
        x += hi.x - lo.x + 60.0;
    }
}

/// Draft a template for a body.
pub fn template(id: &str, spec: &BodySpec) -> Result<Design> {
    let b = Body::new(spec)?;
    let l = b.levels;
    let glued = SeamFinish::Glued;
    let mut d = Design::default();
    match id {
        "top" | "top-sleeves" => {
            d.pieces = vec![
                bodice(&b, true, l.hip, Centre::Fold),
                bodice(&b, false, l.hip, Centre::Fold),
            ];
            d.seams = vec![
                seam(
                    "shoulder",
                    ("front", "front-shoulder"),
                    ("back", "back-shoulder"),
                    false,
                    glued,
                ),
                seam(
                    "side",
                    ("front", "front-side-hem"),
                    ("back", "back-side-hem"),
                    false,
                    glued,
                ),
            ];
            if id == "top-sleeves" {
                add_sleeves(&b, &mut d)?;
            }
        }
        "cincher" => {
            d.pieces = vec![cincher_half(&b, true), cincher_half(&b, false)];
            d.seams = vec![
                seam(
                    "side",
                    ("front", "front-side-bottom"),
                    ("back", "back-side-bottom"),
                    false,
                    glued,
                ),
                seam(
                    "centre back",
                    ("back", "back-centre-top"),
                    ("back", "back-centre-top"),
                    false,
                    SeamFinish::Zip,
                ),
            ];
        }
        "skirt" => {
            d.pieces = vec![skirt_half(&b, true), skirt_half(&b, false)];
            d.seams = vec![
                seam(
                    "side",
                    ("front", "front-side-hem"),
                    ("back", "back-side-hem"),
                    false,
                    glued,
                ),
                seam(
                    "centre back zip",
                    ("back", "back-centre-waist"),
                    ("back", "back-centre-waist"),
                    false,
                    SeamFinish::Zip,
                ),
                seam(
                    "centre back",
                    ("back", "back-zip-end"),
                    ("back", "back-zip-end"),
                    false,
                    glued,
                ),
            ];
        }
        "leggings" => {
            let legs = Legs::new(&b);
            d.pieces = vec![leggings_piece(&b, &legs)];
            d.seams = vec![
                seam(
                    "inside leg",
                    ("leg", "leg-ankle-back"),
                    ("leg", "leg-front-crotch"),
                    true,
                    glued,
                ),
                seam(
                    "centre front",
                    ("leg", "leg-front-waist"),
                    ("leg", "leg-front-waist"),
                    false,
                    glued,
                ),
                seam(
                    "centre back",
                    ("leg", "leg-back-crotch"),
                    ("leg", "leg-back-crotch"),
                    false,
                    glued,
                ),
            ];
        }
        "catsuit" | "catsuit-sleeves" => {
            let legs = Legs::new(&b);
            d.pieces = vec![
                catsuit_half(&b, &legs, true),
                catsuit_half(&b, &legs, false),
            ];
            d.seams = vec![
                seam(
                    "shoulder",
                    ("front", "front-shoulder"),
                    ("back", "back-shoulder"),
                    false,
                    glued,
                ),
                seam(
                    "side",
                    ("front", "front-ankle-side"),
                    ("back", "back-ankle-side"),
                    false,
                    glued,
                ),
                seam(
                    "inside leg",
                    ("front", "front-crotch"),
                    ("back", "back-crotch"),
                    false,
                    glued,
                ),
                seam(
                    "centre front",
                    ("front", "front-centre-neck"),
                    ("front", "front-centre-neck"),
                    false,
                    glued,
                ),
                seam(
                    "centre back zip",
                    ("back", "back-centre-neck"),
                    ("back", "back-centre-neck"),
                    false,
                    SeamFinish::Zip,
                ),
                seam(
                    "centre back",
                    ("back", "back-zip-end"),
                    ("back", "back-zip-end"),
                    false,
                    glued,
                ),
            ];
            if id == "catsuit-sleeves" {
                add_sleeves(&b, &mut d)?;
            }
        }
        other => return Err(Error::Design(format!("unknown template '{other}'"))),
    }
    arrange(&mut d);
    d.block = Some(Block {
        template: id.into(),
        body: spec.clone(),
    });
    Ok(d)
}

/// Move a design drafted from a template to new measurements, keeping its
/// edits (see the module documentation). Designs not drafted from a
/// template are returned unchanged.
pub fn redraft(design: &Design, spec: &BodySpec) -> Result<Design> {
    let Some(block) = &design.block else {
        return Ok(design.clone());
    };
    let old = template(&block.template, &block.body)?;
    let new = template(&block.template, spec)?;
    // Every template point's move, by id: ids are unique across the
    // template's pieces, so a point still moves after its piece was split
    // or its points moved into another piece.
    let mut moved: HashMap<&str, DVec2> = HashMap::new();
    for o in &old.pieces {
        let Some(n) = new.piece_index(&o.id).map(|i| &new.pieces[i]) else {
            continue;
        };
        for on in &o.nodes {
            if let Some(i) = n.node_index(&on.id) {
                moved.insert(on.id.as_str(), n.nodes[i].pos() - on.pos());
            }
        }
    }
    let mut out = design.clone();
    for piece in &mut out.pieces {
        let count = piece.nodes.len();
        let known: Vec<usize> = (0..count)
            .filter(|&i| moved.contains_key(piece.nodes[i].id.as_str()))
            .collect();
        if known.is_empty() {
            continue;
        }
        // Distance along the outline (through the nodes) to each node.
        let pos: Vec<DVec2> = piece.nodes.iter().map(|x| x.pos()).collect();
        let mut along = vec![0.0; count + 1];
        for i in 0..count {
            along[i + 1] = along[i] + pos[i].distance(pos[(i + 1) % count]);
        }
        let perimeter = along[count];
        let shift: Vec<DVec2> = (0..count)
            .map(|i| {
                if let Some(m) = moved.get(piece.nodes[i].id.as_str()) {
                    return *m;
                }
                // The nearest known nodes before and after, by arc length.
                let before = *known
                    .iter()
                    .rev()
                    .find(|&&k| k < i)
                    .unwrap_or(known.last().unwrap());
                let after = *known
                    .iter()
                    .find(|&&k| k > i)
                    .unwrap_or(known.first().unwrap());
                let gap = |a: usize, b: usize| (along[b] - along[a]).rem_euclid(perimeter);
                let (d0, d1) = (gap(before, i), gap(i, after));
                let (m0, m1) = (
                    moved[piece.nodes[before].id.as_str()],
                    moved[piece.nodes[after].id.as_str()],
                );
                if d0 + d1 <= 0.0 {
                    m0
                } else {
                    m0 + (m1 - m0) * (d0 / (d0 + d1))
                }
            })
            .collect();
        for (node, s) in piece.nodes.iter_mut().zip(shift) {
            node.x += s.x;
            node.y += s.y;
        }
    }
    out.block = Some(Block {
        template: block.template.clone(),
        body: spec.clone(),
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::Figure;
    use crate::pattern::{build_pattern, PatternOptions, Reduction};
    use approx::assert_relative_eq;

    fn bodies() -> Vec<BodySpec> {
        let mut out = Vec::new();
        for figure in [Figure::Women, Figure::Men] {
            for scale in [0.85, 1.0, 1.2] {
                let m = figure
                    .typical()
                    .into_iter()
                    .map(|(k, v)| (k, if k.is_girth() { v * scale } else { v }))
                    .collect();
                out.push(BodySpec {
                    figure,
                    measurements: m,
                });
            }
        }
        // Some measurements given, the rest estimated.
        out.push(BodySpec {
            figure: Figure::Women,
            measurements: [(Measure::Height, 1600.0), (Measure::Hips, 1100.0)]
                .into_iter()
                .collect(),
        });
        out
    }

    /// Body length of both sides of every seam.
    fn seam_lengths(d: &Design) -> Vec<(String, f64, f64)> {
        d.seams
            .iter()
            .map(|s| {
                let (a, b) = (d.resolve(&s.a).unwrap(), d.resolve(&s.b).unwrap());
                (
                    s.name.clone(),
                    d.pieces[a.0].edge_length(a.1),
                    d.pieces[b.0].edge_length(b.1),
                )
            })
            .collect()
    }

    #[test]
    fn every_template_drafts_cleanly_for_every_body() {
        for spec in bodies() {
            for t in TEMPLATES {
                let d = template(t.id, &spec).unwrap();
                assert!(d.problems().is_empty(), "{}: {:?}", t.id, d.problems());
                for (name, a, b) in seam_lengths(&d) {
                    assert!((a - b).abs() < 0.5, "{} {name}: {a:.2} vs {b:.2}", t.id);
                }
                let red = vec![Reduction::new(0.85, 1.0); d.pieces.len()];
                let p = build_pattern(&d, &red, &PatternOptions::default()).unwrap();
                assert!(p.iter().all(|x| x.cut_outline_simple), "{}", t.id);
            }
        }
    }

    /// Width of a piece's outline at height `y`.
    fn width_at(p: &Piece, y: f64) -> f64 {
        let (pts, _) = p.outline();
        let n = pts.len();
        let mut xs = Vec::new();
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            if (a.y - y) * (b.y - y) <= 0.0 && a.y != b.y {
                xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
            }
        }
        let (lo, hi) = xs
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), x| {
                (l.min(*x), h.max(*x))
            });
        hi - lo
    }

    #[test]
    fn widths_add_up_to_the_girths() {
        let spec = BodySpec::default();
        let b = Body::new(&spec).unwrap();
        let l = b.levels;
        // Top: front and back halves make half of each girth.
        let d = template("top", &spec).unwrap();
        for (y, k) in [
            (l.bust, Measure::Bust),
            (l.waist, Measure::Waist),
            (l.underbust, Measure::Underbust),
        ] {
            let half = width_at(&d.pieces[0], y) + width_at(&d.pieces[1], y);
            assert_relative_eq!(2.0 * half, b.get(k), epsilon = 0.5);
        }
        // Leggings: the leg's width is its girth at the knee and ankle.
        let d = template("leggings", &spec).unwrap();
        assert_relative_eq!(
            width_at(&d.pieces[0], l.knee),
            b.get(Measure::Knee),
            epsilon = 0.5
        );
        assert_relative_eq!(
            width_at(&d.pieces[0], l.ankle + 0.01),
            b.get(Measure::Ankle),
            epsilon = 0.5
        );
        // Catsuit: front and back legs together make the girth.
        let d = template("catsuit", &spec).unwrap();
        let knee = width_at(&d.pieces[0], l.knee) + width_at(&d.pieces[1], l.knee);
        assert_relative_eq!(knee, b.get(Measure::Knee), epsilon = 0.5);
        let waist = width_at(&d.pieces[0], l.waist) + width_at(&d.pieces[1], l.waist);
        assert_relative_eq!(2.0 * waist, b.get(Measure::Waist), epsilon = 0.5);
    }

    #[test]
    fn sleeve_caps_fit_their_armholes() {
        let d = template("top-sleeves", &BodySpec::default()).unwrap();
        let s = &d.pieces[2];
        let cap_top = s.nodes[s.node_index("sleeve-cap-top").unwrap()].pos();
        // The cap rises well above the underarm line and its top sits near
        // the middle.
        assert!(cap_top.y > 60.0 && cap_top.x.abs() < 40.0, "{cap_top}");
    }

    #[test]
    fn redraft_keeps_edits_and_follows_the_body() {
        let spec = BodySpec::default();
        let mut d = template("top", &spec).unwrap();
        // Unchanged measurements: unchanged design.
        assert_eq!(redraft(&d, &spec).unwrap(), d);
        // Lower the front neckline by 30 mm and add a point to the hem.
        let front = &mut d.pieces[0];
        let i = front.node_index("front-centre-neck").unwrap();
        front.nodes[i].y -= 30.0;
        let hem_y = front.nodes[0].y;
        front
            .nodes
            .insert(1, Node::new("added", 50.0, hem_y, Smooth));
        // 50 mm taller, 10 % fuller.
        let mut bigger = spec.clone();
        for (k, v) in Figure::Women.typical() {
            let v = if k.is_girth() {
                v * 1.1
            } else {
                v * 1680.0 / 1680.0
            };
            bigger.measurements.insert(k, v);
        }
        bigger.measurements.insert(Measure::Height, 1730.0);
        let r = redraft(&d, &bigger).unwrap();
        let fresh = template("top", &bigger).unwrap();
        let get = |d: &Design, id: &str| {
            let p = &d.pieces[0];
            p.nodes[p.node_index(id).unwrap()].pos()
        };
        // The edit is kept relative to the new draft.
        let neck = get(&r, "front-centre-neck") - get(&fresh, "front-centre-neck");
        assert_relative_eq!(neck.y, -30.0, epsilon = 1e-9);
        assert_relative_eq!(get(&r, "front-side-hem").x, get(&fresh, "front-side-hem").x);
        // The added point moves with its neighbours: still on the hem.
        let added = get(&r, "added");
        assert!(added.y < get(&r, "front-side-hem").y + 1e-9);
        assert!(added.x > get(&r, "front-centre-hem").x && added.x < get(&r, "front-side-hem").x);
        assert_eq!(r.block.unwrap().body, bigger);

        // A piece made from template points (as a split makes) follows too.
        let mut d = template("top", &spec).unwrap();
        d.pieces[1].id = "renamed".into();
        let r = redraft(&d, &bigger).unwrap();
        let p = &r.pieces[1];
        let fresh_back = &fresh.pieces[1];
        let id = "back-side-hem";
        assert_eq!(
            p.nodes[p.node_index(id).unwrap()].pos(),
            fresh_back.nodes[fresh_back.node_index(id).unwrap()].pos()
        );
    }

    #[test]
    fn unknown_templates_are_reported() {
        assert!(template("cape", &BodySpec::default()).is_err());
    }
}
