//! SVG export at 1:1 scale.
//!
//! The document is sized in millimetres (`width="…mm"`) with a `viewBox` in
//! the same units, so printing at 100 % reproduces the pattern at true
//! size; a 50 mm scale bar is included to check the print. Pieces are drawn
//! as designed (`y` up the body, flipped for SVG's downward axis).

use crate::nest::Layout;
use crate::pattern::PatternPiece;
use glam::DVec2;
use std::fmt::Write;

/// Render one pattern piece as a standalone SVG document, with `title` and
/// `notes` printed above it.
///
/// The solid line is the cut line; where it carries an allowance the seam
/// line is drawn dashed inside it. Notches are ticks across the seam line
/// with their number; edges are labelled with what they join. A folded
/// piece shows its centre line (dash-dot), and an arrow points up the
/// body, the direction of the lengthwise reduction.
pub fn piece_svg(piece: &PatternPiece, title: &str, notes: &[String]) -> String {
    let flip = |p: DVec2| DVec2::new(p.x, -p.y);
    let cut: Vec<DVec2> = piece.cut_outline.iter().map(|&p| flip(p)).collect();
    let line: Vec<DVec2> = piece.seam_line.iter().map(|&p| flip(p)).collect();
    let (mut lo, mut hi) = crate::geom::bounds(&cut);
    let (l2, h2) = crate::geom::bounds(&line);
    lo = lo.min(l2);
    hi = hi.max(h2);
    let m = 10.0;
    let header = 6.0 + 5.0 * notes.len() as f64 + 4.0;
    let footer = 12.0;
    let x0 = lo.x - m;
    let y0 = lo.y - m - header;
    let w = ((hi.x - lo.x) + 2.0 * m).max(90.0);
    let h = (hi.y - lo.y) + 2.0 * m + header + footer;

    let mut s = String::new();
    let _ = writeln!(s, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.3}mm" height="{h:.3}mm" viewBox="{x0:.3} {y0:.3} {w:.3} {h:.3}">"#
    );
    let _ = writeln!(
        s,
        r#"<g font-family="sans-serif" font-size="3.5" fill="black">"#
    );
    let tx = lo.x - m + 2.0;
    let mut ty = y0 + 6.0;
    let _ = writeln!(
        s,
        r#"<text x="{tx:.3}" y="{ty:.3}" font-size="5">{}</text>"#,
        escape(title)
    );
    for n in notes {
        ty += 5.0;
        let _ = writeln!(s, r#"<text x="{tx:.3}" y="{ty:.3}">{}</text>"#, escape(n));
    }
    let _ = writeln!(s, "</g>");

    // Cut line.
    let _ = writeln!(
        s,
        r#"<path id="cut-line" d="{}" fill="none" stroke="black" stroke-width="0.3" stroke-linejoin="round"/>"#,
        closed_path(&cut)
    );
    // Seam line wherever it differs from the cut line.
    let runs = allowance_runs(piece);
    if !runs.is_empty() {
        let mut d = String::new();
        for run in &runs {
            for (k, p) in run.iter().enumerate() {
                let p = flip(*p);
                let _ = write!(d, "{}{:.3} {:.3}", if k == 0 { "M" } else { "L" }, p.x, p.y);
            }
        }
        let _ = writeln!(
            s,
            r#"<path id="seam-line" d="{d}" fill="none" stroke="black" stroke-width="0.2" stroke-dasharray="2 1"/>"#
        );
    }
    if let Some([a, b]) = piece.fold_line {
        let (a, b) = (flip(a), flip(b));
        let _ = writeln!(
            s,
            r#"<path id="fold-line" d="M{:.3} {:.3}L{:.3} {:.3}" stroke="gray" stroke-width="0.25" stroke-dasharray="6 1.5 1 1.5"/>"#,
            a.x, a.y, b.x, b.y
        );
    }
    // Arrow up the body through the middle of the piece.
    let (g0, g1) = grain_arrow(piece);
    let [b0, b1] = arrow_barbs(g0, g1);
    let (g0, g1, b0, b1) = (flip(g0), flip(g1), flip(b0), flip(b1));
    let _ = writeln!(
        s,
        r#"<g id="grain" stroke="gray" stroke-width="0.3" fill="none"><path d="M{:.3} {:.3}L{:.3} {:.3}M{:.3} {:.3}L{:.3} {:.3}L{:.3} {:.3}"/></g>"#,
        g0.x, g0.y, g1.x, g1.y, b0.x, b0.y, g1.x, g1.y, b1.x, b1.y
    );
    let c = 0.5 * (g0 + g1);
    let _ = writeln!(
        s,
        r#"<text x="{:.3}" y="{:.3}" font-family="sans-serif" font-size="6" text-anchor="middle" fill="gray">{}</text>"#,
        c.x + 4.0,
        c.y,
        escape(&piece.name)
    );

    if !piece.notches.is_empty() {
        let _ = writeln!(
            s,
            r#"<g id="notches" stroke="black" stroke-width="0.3" font-family="sans-serif" font-size="2.5" text-anchor="middle">"#
        );
        for n in &piece.notches {
            let out = n.allowance.max(2.0);
            let (a, b) = (
                flip(n.position - 4.0 * n.normal),
                flip(n.position + out * n.normal),
            );
            let t = flip(n.position - 6.5 * n.normal);
            let _ = writeln!(
                s,
                r#"<path d="M{:.3} {:.3}L{:.3} {:.3}"/><text x="{:.3}" y="{:.3}" stroke="none" dominant-baseline="middle">{}</text>"#,
                a.x, a.y, b.x, b.y, t.x, t.y, n.number
            );
        }
        let _ = writeln!(s, "</g>");
    }
    if !piece.labels.is_empty() {
        let _ = writeln!(
            s,
            r#"<g id="edge-labels" font-family="sans-serif" font-size="2.5" text-anchor="middle">"#
        );
        for l in &piece.labels {
            // Inside is to the left of the outline's direction; beyond the
            // notch numbers (6.5 mm).
            let q = l.at + 10.0 * l.dir.perp();
            let mut a = (-l.dir.y).atan2(l.dir.x).to_degrees();
            if a > 90.0 {
                a -= 180.0;
            } else if a < -90.0 {
                a += 180.0;
            }
            let _ = writeln!(
                s,
                r#"<text transform="translate({:.3} {:.3}) rotate({a:.2})" dominant-baseline="middle">{}</text>"#,
                q.x,
                -q.y,
                escape(&l.text)
            );
        }
        let _ = writeln!(s, "</g>");
    }

    // Footer: 50 mm scale bar.
    let fy = hi.y + m + 4.0;
    let fx = lo.x - m + 2.0;
    let _ = writeln!(
        s,
        r#"<g id="scale" stroke="black" stroke-width="0.3"><path d="M{fx:.3} {:.3}V{fy:.3}H{:.3}V{:.3}" fill="none"/></g>"#,
        fy - 1.5,
        fx + 50.0,
        fy - 1.5
    );
    let _ = writeln!(
        s,
        r#"<text x="{:.3}" y="{:.3}" font-family="sans-serif" font-size="3">50 mm</text>"#,
        fx,
        fy + 4.0
    );
    let _ = writeln!(s, "</svg>");
    s
}

/// Runs of the seam line that carry an allowance (open polylines).
pub fn allowance_runs(piece: &PatternPiece) -> Vec<Vec<DVec2>> {
    let n = piece.seam_line.len();
    let has = |i: usize| piece.allowance[i % n] > 0.0;
    let Some(start) = (0..n).find(|&i| !has(i)) else {
        // Allowance all round: one closed run.
        let mut run = piece.seam_line.clone();
        run.push(piece.seam_line[0]);
        return vec![run];
    };
    let mut runs = Vec::new();
    let mut cur: Vec<DVec2> = Vec::new();
    for j in 1..=n {
        let i = (start + j) % n;
        if has(i) {
            if cur.is_empty() {
                cur.push(piece.seam_line[i]);
            }
            cur.push(piece.seam_line[(i + 1) % n]);
        } else if !cur.is_empty() {
            runs.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    runs
}

/// An arrow through the middle of a piece, pointing up the body (see
/// [`Piece::up_deg`](crate::design::Piece::up_deg)): its tail and head.
pub fn grain_arrow(piece: &PatternPiece) -> (DVec2, DVec2) {
    let (lo, hi) = crate::geom::bounds(&piece.seam_line);
    let c = 0.5 * (lo + hi);
    let u = piece.reduction.up_dir();
    // Extent of the piece along the arrow.
    let ext = piece
        .seam_line
        .iter()
        .map(|p| (*p - c).dot(u))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), t| {
            (a.min(t), b.max(t))
        });
    let half = (0.2 * (ext.1 - ext.0)).clamp(15.0, 80.0);
    (c - half * u, c + half * u)
}

/// The two barbs of an arrowhead at `head`, for an arrow from `tail`.
pub fn arrow_barbs(tail: DVec2, head: DVec2) -> [DVec2; 2] {
    let d = (head - tail).normalize_or_zero();
    let n = d.perp();
    [head - 4.0 * d + 2.0 * n, head - 4.0 * d - 2.0 * n]
}

/// Render a trim strip at 1:1: its outline, with marks at each quarter of
/// its length to spread it evenly along the edge it finishes.
pub fn trim_svg(t: &crate::pattern::TrimStrip) -> String {
    let m = 10.0;
    let header = 12.0;
    let (l, w) = (t.length_mm, t.width_mm);
    let (vw, vh) = (l + 2.0 * m, w + 2.0 * m + header + 10.0);
    let mut s = String::new();
    let _ = writeln!(s, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{vw:.3}mm" height="{vh:.3}mm" viewBox="{:.3} {:.3} {vw:.3} {vh:.3}">"#,
        -m,
        -m - header
    );
    let _ = writeln!(
        s,
        r#"<text x="0" y="{:.3}" font-family="sans-serif" font-size="5">trim: {} edge, {:.0} × {:.0} mm (edge {:.0} mm)</text>"#,
        -header + 1.0,
        escape(&t.edge),
        l,
        w,
        t.edge_length_mm
    );
    let _ = writeln!(
        s,
        r#"<path id="cut-line" d="M0 0H{l:.3}V{w:.3}H0Z" fill="none" stroke="black" stroke-width="0.3"/>"#
    );
    let _ = writeln!(
        s,
        r#"<g id="quarter-marks" stroke="black" stroke-width="0.3">"#
    );
    for k in 1..4 {
        let x = l * k as f64 / 4.0;
        let _ = writeln!(s, r#"<path d="M{x:.3} 0V3M{x:.3} {:.3}V{w:.3}"/>"#, w - 3.0);
    }
    let _ = writeln!(s, "</g>");
    let fy = w + m + 4.0;
    let _ = writeln!(
        s,
        r#"<g stroke="black" stroke-width="0.3"><path d="M0 {:.3}V{fy:.3}H50V{:.3}" fill="none"/></g><text x="0" y="{:.3}" font-family="sans-serif" font-size="3">50 mm</text>"#,
        fy - 1.5,
        fy - 1.5,
        fy + 4.0
    );
    let _ = writeln!(s, "</svg>");
    s
}

/// Render a sheet layout: every piece's cut line at its placement, the
/// sheet edges, and the length used. 1:1 scale in mm; `y` runs along the
/// roll (down the page), pieces keep the outside-facing orientation of the
/// individual panel SVGs.
pub fn layout_svg(outlines: &[Vec<DVec2>], labels: &[String], layout: &Layout) -> String {
    let m = 10.0;
    let header = 8.0;
    let (w, l) = (layout.sheet_width, layout.length);
    let (vw, vh) = (w + 2.0 * m, l + 2.0 * m + header);
    let flip = |p: DVec2| DVec2::new(p.x, l - p.y);
    let mut s = String::new();
    let _ = writeln!(s, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{vw:.3}mm" height="{vh:.3}mm" viewBox="{:.3} {:.3} {vw:.3} {vh:.3}">"#,
        -m,
        -m - header
    );
    let _ = writeln!(
        s,
        r#"<text x="0" y="{:.3}" font-family="sans-serif" font-size="5">sheet {w:.0} mm wide, {l:.0} mm used ({:.0}% of the area)</text>"#,
        -m - header + 5.5,
        100.0 * layout.utilisation
    );
    let _ = writeln!(
        s,
        r#"<path id="sheet" d="M0 {:.3}V{:.3}M{w:.3} {:.3}V{:.3}M0 {l:.3}H{w:.3}" stroke="gray" stroke-width="0.4" stroke-dasharray="6 3" fill="none"/>"#,
        -m,
        l + m,
        -m,
        l + m
    );
    for (pl, label) in layout.placements.iter().zip(labels) {
        let pts: Vec<DVec2> = outlines[pl.piece]
            .iter()
            .map(|&p| flip(pl.apply(p)))
            .collect();
        let c = pts.iter().fold(DVec2::ZERO, |a, p| a + *p) / pts.len() as f64;
        let _ = writeln!(
            s,
            r#"<path d="{}" fill="none" stroke="black" stroke-width="0.3"/><text x="{:.3}" y="{:.3}" font-family="sans-serif" font-size="6" text-anchor="middle">{}</text>"#,
            closed_path(&pts),
            c.x,
            c.y,
            escape(label)
        );
    }
    let _ = writeln!(s, "</svg>");
    s
}

fn closed_path(pts: &[DVec2]) -> String {
    let mut d = String::new();
    for (k, p) in pts.iter().enumerate() {
        let _ = write!(d, "{}{:.3} {:.3}", if k == 0 { "M" } else { "L" }, p.x, p.y);
    }
    d.push('Z');
    d
}

fn escape(t: &str) -> String {
    t.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::tests::rect;
    use crate::design::Design;
    use crate::pattern::{build_pattern, PatternOptions, Reduction};

    #[test]
    fn svg_is_mm_sized_and_contains_outline() {
        let mut d = Design {
            pieces: vec![rect("a", 100.0, 160.0)],
            ..Default::default()
        };
        d.pieces[0].nodes[3].edge.fold = true;
        d.pieces[0].name = "A&B".into();
        let p =
            build_pattern(&d, &[Reduction::new(0.85, 1.0)], &PatternOptions::default()).unwrap();
        let svg = piece_svg(&p[0], "front", &["cut 1".into()]);
        // Unfolded 200 mm × 0.85 wide piece + 2 × 10 mm margin.
        assert!(svg.contains(r#"width="190.000mm""#), "{}", &svg[..300]);
        assert!(svg.contains(r#"id="cut-line""#));
        assert!(svg.contains(r#"id="fold-line""#));
        assert!(!svg.contains(r#"id="seam-line""#));
        assert!(svg.contains("A&amp;B"));
    }

    #[test]
    fn allowance_runs_follow_the_outline() {
        let d = Design {
            pieces: vec![rect("a", 100.0, 160.0)],
            ..Default::default()
        };
        let opts = PatternOptions {
            hem_allowance: 5.0,
            ..Default::default()
        };
        let p = build_pattern(&d, &[Reduction::new(1.0, 1.0)], &opts).unwrap();
        let runs = allowance_runs(&p[0]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].first(), runs[0].last());
    }
}
