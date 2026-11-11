//! The order to glue a garment together, as plain-language steps.
//!
//! 1. **Join pieces**: glued seams between *different* pieces, shortest
//!    first (shoulders before sides on a top). A union–find over pieces
//!    tells joining seams from *closing* seams, whose two sides are already
//!    connected (they turn the garment into a tube); closing seams come
//!    after, as they are harder to reach.
//! 2. **Zips**, glued under the strips left on both sides.
//! 3. **Edges**: hems folded and glued, then trim strips.
//!
//! Each step names the pieces, which side carries the glue lap, and the
//! notch numbers to match.

use crate::flatten::Garment;
use crate::pattern::{EdgeFinish, EdgeSpec, PatternPiece, TrimStrip};
use crate::seams::{SeamFinish, SeamSet};

fn seam_name(seams: &SeamSet, id: u32) -> String {
    seams.seams[id as usize]
        .name
        .clone()
        .unwrap_or_else(|| format!("seam {id}"))
}

/// Notch numbers on one side of a seam pair, as "notches a–b".
fn notch_text(pieces: &[PatternPiece], panel: usize, edges: &[usize]) -> String {
    let mut n: Vec<u32> = pieces[panel]
        .notches
        .iter()
        .filter(|x| edges.contains(&x.boundary_edge))
        .map(|x| x.number)
        .collect();
    n.sort_unstable();
    n.dedup();
    match n.as_slice() {
        [] => String::new(),
        [a] => format!("; match notch {a}"),
        [a, .., b] => format!("; match notches {a}–{b}"),
    }
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// The assembly steps for a garment.
pub fn assembly_steps(
    garment: &Garment,
    pieces: &[PatternPiece],
    seams: &SeamSet,
    edges: &[EdgeSpec],
    trims: &[TrimStrip],
) -> Vec<String> {
    let mut steps = vec![
        "Cut each piece along its solid line and mark every numbered notch with a pen (do not snip latex: \
         a cut starts a tear)."
            .to_string(),
    ];

    // Seams: joining first (shortest first), then closing.
    let mut order: Vec<usize> = (0..garment.seams.len()).collect();
    order.sort_by(|&a, &b| {
        garment.seams[a]
            .lengths
            .body
            .total_cmp(&garment.seams[b].lengths.body)
    });
    let mut parent: Vec<usize> = (0..garment.panels.len()).collect();
    let (mut joining, mut closing, mut zips) = (Vec::new(), Vec::new(), Vec::new());
    for i in order {
        let p = &garment.seams[i].pair;
        if seams.seams[p.seam as usize].finish == SeamFinish::Zip {
            zips.push(i);
            continue;
        }
        let (a, b) = (
            find(&mut parent, p.left.panel),
            find(&mut parent, p.right.panel),
        );
        if a == b {
            closing.push(i);
        } else {
            parent[a] = b;
            joining.push(i);
        }
    }
    for (closes, list) in [(false, &joining), (true, &closing)] {
        for &i in list {
            let p = &garment.seams[i].pair;
            let name = seam_name(seams, p.seam);
            let lap = |side: &crate::seam_match::SeamSide| {
                side.edges
                    .iter()
                    .map(|&b| pieces[side.panel].edge_allowance[b])
                    .fold(0.0f64, f64::max)
            };
            let (l, r) = (lap(&p.left), lap(&p.right));
            let (carrier, other) = if l >= r {
                (&p.left, &p.right)
            } else {
                (&p.right, &p.left)
            };
            let width = l.max(r);
            let verb = if closes { "Close" } else { "Join" };
            let what = if p.left.panel == p.right.panel {
                format!("piece {} to itself", p.left.panel)
            } else {
                format!("pieces {} and {}", p.left.panel, p.right.panel)
            };
            let how = if width > 0.0 {
                format!(
                    ": glue piece {}'s {width:.0} mm lap (outside its dashed line) under piece {}'s edge",
                    carrier.panel, other.panel
                )
            } else {
                ": butt the edges together and glue a backing strip under them".to_string()
            };
            steps.push(format!(
                "{verb} {what} along the {name} seam{how}{}.",
                notch_text(pieces, carrier.panel, &carrier.edges)
            ));
        }
    }
    for i in zips {
        let p = &garment.seams[i].pair;
        steps.push(format!(
            "Fit the zip in the {} seam: glue the zip tape under the strips of pieces {} and {}, \
             the edges meeting over the teeth{}.",
            seam_name(seams, p.seam),
            p.left.panel,
            p.right.panel,
            notch_text(pieces, p.left.panel, &p.left.edges)
        ));
    }

    // Edges.
    for e in edges {
        if let EdgeFinish::Hem { width_mm } = e.finish {
            steps.push(format!(
                "Hem the {} edge: fold the {width_mm:.0} mm allowance to the inside along the dashed line and glue it.",
                e.name
            ));
        }
    }
    for t in trims {
        let pieces: Vec<String> = t.panels.iter().map(|p| p.to_string()).collect();
        steps.push(format!(
            "Finish the {} edge (pieces {}) with its trim strip, {:.0} × {:.0} mm: it is {:.0} % shorter than \
             the edge, so stretch it evenly as you glue it on.",
            t.edge,
            pieces.join(", "),
            t.width_mm,
            t.length_mm,
            100.0 * (1.0 - t.length_mm / t.edge_length_mm)
        ));
    }
    steps
}
