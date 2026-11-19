//! The order to glue a garment together, as plain-language steps.
//!
//! 1. **Cut** every piece (pairs as mirror images) and mark the notches.
//! 2. **Join pieces**: glued seams that join separate pieces, shortest
//!    first (shoulders before sides on a top). A union–find over the cut
//!    pieces tells joining seams from *closing* seams, whose two sides are
//!    already connected (they turn the garment into a tube); closing seams
//!    come after, as they are harder to reach.
//! 3. **Zips**, glued under the strips left on both sides.
//! 4. **Edges**: hems folded and glued, then trim strips.
//!
//! Each step names the pieces, which side carries the glue lap, and the
//! notch numbers to match.

use crate::design::{Design, EdgeFinish, Lap, SeamFinish};
use crate::pattern::{PatternPiece, TrimStrip};

/// "a", "a and b", "a, b and c".
fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// "; match notches a–b" for a seam's notches.
fn notch_text(pieces: &[PatternPiece], seam: usize) -> String {
    let mut n: Vec<u32> = pieces
        .iter()
        .flat_map(|p| &p.notches)
        .filter(|x| x.seam == seam)
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

/// The assembly steps for a design and its pattern pieces.
pub fn assembly_steps(
    design: &Design,
    pieces: &[PatternPiece],
    seam_allowance: f64,
    trims: &[TrimStrip],
) -> Vec<String> {
    let mut steps = vec![
        "Cut each piece along its solid line (a piece marked \"cut 2\" once as printed and once \
         flipped over) and mark every numbered notch with a pen (do not snip latex: a cut starts a \
         tear)."
            .to_string(),
    ];
    let name = |p: usize| design.pieces[p].name.clone();

    // Cut pieces: one per design piece, two for a pair (right, left).
    let mut first = Vec::new();
    let mut count = 0;
    for p in &design.pieces {
        first.push(count);
        count += if p.pair { 2 } else { 1 };
    }
    let inst = |p: usize, left: bool| first[p] + usize::from(left && design.pieces[p].pair);
    let mut parent: Vec<usize> = (0..count).collect();

    let mut order: Vec<usize> = (0..design.seams.len()).collect();
    let len = |i: usize| {
        design
            .resolve(&design.seams[i].a)
            .map_or(0.0, |(p, k)| pieces[p].body_length(k))
    };
    order.sort_by(|&a, &b| len(a).total_cmp(&len(b)));
    let (mut joining, mut closing, mut zips) = (Vec::new(), Vec::new(), Vec::new());
    for i in order {
        let s = &design.seams[i];
        let (Some(a), Some(b)) = (design.resolve(&s.a), design.resolve(&s.b)) else {
            continue;
        };
        if s.finish == SeamFinish::Zip {
            zips.push(i);
            continue;
        }
        let joins: Vec<(usize, usize)> = if a == b {
            vec![(inst(a.0, false), inst(a.0, true))]
        } else {
            vec![
                (inst(a.0, false), inst(b.0, false)),
                (inst(a.0, true), inst(b.0, true)),
            ]
        };
        let mut joined = false;
        for (x, y) in joins {
            let (rx, ry) = (find(&mut parent, x), find(&mut parent, y));
            if rx != ry {
                parent[rx] = ry;
                joined = true;
            }
        }
        if joined {
            joining.push(i);
        } else {
            closing.push(i);
        }
    }

    for (closes, list) in [(false, &joining), (true, &closing)] {
        for &i in list {
            let s = &design.seams[i];
            let (a, b) = (design.resolve(&s.a).unwrap(), design.resolve(&s.b).unwrap());
            let seam = if s.name.is_empty() {
                "seam".to_string()
            } else {
                format!("{} seam", s.name)
            };
            let verb = if closes { "Close" } else { "Join" };
            let w = seam_allowance;
            let text = if a == b {
                let how = match s.lap {
                    Lap::None => {
                        "butt the edges together and glue a backing strip under them".to_string()
                    }
                    Lap::Both => format!("overlap the two {w:.0} mm laps and glue them"),
                    Lap::A | Lap::B => format!(
                        "the pattern carries a {w:.0} mm lap: cut it off one of the two pieces along \
                         the dashed line, then glue the other's lap under it"
                    ),
                };
                format!(
                    "{verb} the two {} pieces along the {seam}: {how}",
                    name(a.0)
                )
            } else {
                let both = design.pieces[a.0].pair
                    || design.pieces[b.0].pair
                    || pieces[a.0].folded
                    || pieces[b.0].folded;
                let what = if a.0 == b.0 {
                    format!("{} to itself", name(a.0))
                } else {
                    format!("{} and {}", name(a.0), name(b.0))
                };
                let how = match s.lap {
                    Lap::None => {
                        "butt the edges together and glue a backing strip under them".to_string()
                    }
                    Lap::Both => format!("overlap the two {w:.0} mm laps and glue them"),
                    Lap::A => format!(
                        "glue {}'s {w:.0} mm lap (outside its dashed line) under {}'s edge",
                        name(a.0),
                        name(b.0)
                    ),
                    Lap::B => format!(
                        "glue {}'s {w:.0} mm lap (outside its dashed line) under {}'s edge",
                        name(b.0),
                        name(a.0)
                    ),
                };
                format!(
                    "{verb} {what} along the {seam}{}: {how}",
                    if both { " on both sides" } else { "" }
                )
            };
            steps.push(format!("{text}{}.", notch_text(pieces, i)));
        }
    }
    for i in zips {
        let s = &design.seams[i];
        let (a, b) = (design.resolve(&s.a).unwrap(), design.resolve(&s.b).unwrap());
        let seam = if s.name.is_empty() {
            "zip".to_string()
        } else {
            format!("{} seam", s.name)
        };
        let whose = if a == b {
            format!("the two {} pieces", name(a.0))
        } else {
            format!("{} and {}", name(a.0), name(b.0))
        };
        steps.push(format!(
            "Fit the zip in the {seam}: glue the zip tape under the strips of {whose}, the edges \
             meeting over the teeth{}.",
            notch_text(pieces, i)
        ));
    }

    // Hems, one step per edge name.
    let mut hems: Vec<(String, f64, Vec<String>)> = Vec::new();
    for (pi, p) in design.pieces.iter().enumerate() {
        for k in 0..p.edges().len() {
            let props = p.edge_props(k);
            let Some(EdgeFinish::Hem { width_mm }) = props.finish else {
                continue;
            };
            if design.seam_on(pi, k).is_some() || props.fold {
                continue;
            }
            let edge = p.edge_name(k);
            match hems.iter_mut().find(|h| h.0 == edge && h.1 == width_mm) {
                Some(h) => {
                    if !h.2.contains(&p.name) {
                        h.2.push(p.name.clone());
                    }
                }
                None => hems.push((edge, width_mm, vec![p.name.clone()])),
            }
        }
    }
    for (edge, w, on) in hems {
        steps.push(format!(
            "Finish the {edge} of the {} with a {w:.0} mm folded hem: fold the allowance to the \
             inside along the dashed line and glue it.",
            list(&on)
        ));
    }
    for t in trims {
        let on: Vec<String> = t.pieces.iter().map(|&p| name(p)).collect();
        let on = [list(&on)];
        steps.push(format!(
            "Finish the {} of the {} with its trim strip, {:.0} × {:.0} mm: it is {:.0} % shorter than \
             the edge, so stretch it evenly as you glue it on, matching its quarter marks to the \
             edge's quarters.",
            t.edge,
            on.join(", "),
            t.length_mm,
            t.width_mm,
            100.0 * (1.0 - t.length_mm / t.edge_length_mm)
        ));
    }
    steps
}
