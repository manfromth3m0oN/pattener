//! Laying pieces out on a sheet (roll) of fixed width.
//!
//! Latex comes in rolls or sheets of fixed width, so the useful number is
//! how much *length* a garment needs. This module packs each piece's cut
//! outline, by its bounding rectangle, with the **bottom-left skyline**
//! heuristic:
//!
//! 1. Pieces are taken in order of decreasing bounding-box area.
//! 2. The skyline is the upper envelope of everything placed so far, a list
//!    of horizontal segments covering the sheet width.
//! 3. For each orientation (0°, and 90° if allowed) and each skyline
//!    segment start `x`, the piece rests at `y = max skyline height over
//!    [x, x + w)`; the position with the lowest top edge wins (ties: lowest
//!    `x`).
//!
//! Every piece is inflated by `gap` so neighbours keep a cutting margin.
//! Pieces are only rotated, never mirrored: latex sheets have a shiny and a
//! matte face, and a mirrored piece would be cut inside out.
//!
//! Bounding-box packing wastes the space inside curved pieces; true
//! polygon nesting is a possible later improvement.

use crate::error::{Error, Result};
use glam::DVec2;

/// Sheet description.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NestOptions {
    /// Usable sheet width (mm).
    pub sheet_width: f64,
    /// Minimum distance between pieces (mm).
    pub gap: f64,
    /// Allow 90° rotation.
    pub allow_rotation: bool,
}

/// Where one piece goes: `p ↦ rot(p) + shift`, where `rot` turns by 90°
/// counter-clockwise if `rotated`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Placement {
    pub piece: usize,
    pub rotated: bool,
    pub shift: DVec2,
}

impl Placement {
    pub fn apply(&self, p: DVec2) -> DVec2 {
        let r = if self.rotated {
            DVec2::new(-p.y, p.x)
        } else {
            p
        };
        r + self.shift
    }
}

/// Result of [`nest`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Layout {
    /// One per piece, in piece order.
    pub placements: Vec<Placement>,
    pub sheet_width: f64,
    /// Sheet length used (mm).
    pub length: f64,
    /// Total piece area / (width × length).
    pub utilisation: f64,
}

fn bounds(pts: &[DVec2]) -> (DVec2, DVec2) {
    pts.iter().fold(
        (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    )
}

/// Pack `outlines` (cut lines, any position) onto a sheet.
pub fn nest(outlines: &[Vec<DVec2>], opts: &NestOptions) -> Result<Layout> {
    let eps = 1e-9;
    let width = opts.sheet_width + opts.gap;
    let rotate = |p: DVec2| DVec2::new(-p.y, p.x);

    let mut order: Vec<usize> = (0..outlines.len()).collect();
    let size = |i: usize| {
        let (lo, hi) = bounds(&outlines[i]);
        hi - lo
    };
    order.sort_by(|&a, &b| {
        let (sa, sb) = (size(a), size(b));
        (sb.x * sb.y).total_cmp(&(sa.x * sa.y)).then(a.cmp(&b))
    });

    // Skyline segments (x0, x1, y), sorted, covering [0, width].
    let mut sky: Vec<(f64, f64, f64)> = vec![(0.0, width, 0.0)];
    let mut placements = vec![None; outlines.len()];
    let mut length: f64 = 0.0;

    for &i in &order {
        let mut best: Option<(f64, f64, bool, DVec2)> = None; // (top, x, rotated, lo)
        for rotated in [false, true] {
            if rotated && !opts.allow_rotation {
                continue;
            }
            let pts: Vec<DVec2> = outlines[i]
                .iter()
                .map(|&p| if rotated { rotate(p) } else { p })
                .collect();
            let (lo, hi) = bounds(&pts);
            let (w, h) = (hi.x - lo.x + opts.gap, hi.y - lo.y + opts.gap);
            for &(x, _, _) in &sky {
                if x + w > width + eps {
                    continue;
                }
                let y = sky
                    .iter()
                    .filter(|s| s.1 > x + eps && s.0 < x + w - eps)
                    .map(|s| s.2)
                    .fold(0.0, f64::max);
                let top = y + h;
                let better = match best {
                    None => true,
                    Some((bt, bx, _, _)) => {
                        top < bt - eps || ((top - bt).abs() <= eps && x < bx - eps)
                    }
                };
                if better {
                    best = Some((top, x, rotated, lo));
                }
            }
        }
        let Some((top, x, rotated, lo)) = best else {
            let s = size(i);
            return Err(Error::PieceTooWide {
                piece: i,
                width: s.x,
                height: s.y,
                sheet_width: opts.sheet_width,
            });
        };
        let (_, hi) = bounds(
            &outlines[i]
                .iter()
                .map(|&p| if rotated { rotate(p) } else { p })
                .collect::<Vec<_>>(),
        );
        let w = hi.x - lo.x + opts.gap;
        let y = top - (hi.y - lo.y + opts.gap);
        placements[i] = Some(Placement {
            piece: i,
            rotated,
            shift: DVec2::new(x, y) - lo,
        });
        length = length.max(top - opts.gap);

        // Raise the skyline over [x, x + w).
        let mut next = Vec::with_capacity(sky.len() + 2);
        for &(a, b, h) in &sky {
            if b <= x + eps || a >= x + w - eps {
                next.push((a, b, h));
                continue;
            }
            if a < x {
                next.push((a, x, h));
            }
            if b > x + w {
                next.push((x + w, b, h));
            }
        }
        next.push((x, x + w, top));
        next.sort_by(|p, q| p.0.total_cmp(&q.0));
        sky.clear();
        for seg in next {
            match sky.last_mut() {
                Some(last) if (last.2 - seg.2).abs() < eps => last.1 = seg.1,
                _ => sky.push(seg),
            }
        }
    }

    let area: f64 = outlines
        .iter()
        .map(|o| crate::allowance::polygon_area(o).abs())
        .sum();
    Ok(Layout {
        placements: placements.into_iter().map(Option::unwrap).collect(),
        sheet_width: opts.sheet_width,
        length,
        utilisation: if length > 0.0 {
            area / (opts.sheet_width * length)
        } else {
            0.0
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Vec<DVec2> {
        vec![
            DVec2::new(x, y),
            DVec2::new(x + w, y),
            DVec2::new(x + w, y + h),
            DVec2::new(x, y + h),
        ]
    }

    fn placed(o: &[DVec2], p: &Placement) -> (DVec2, DVec2) {
        bounds(&o.iter().map(|&q| p.apply(q)).collect::<Vec<_>>())
    }

    fn opts(w: f64, rot: bool) -> NestOptions {
        NestOptions {
            sheet_width: w,
            gap: 5.0,
            allow_rotation: rot,
        }
    }

    #[test]
    fn row_of_pieces_fits_across() {
        let pieces = vec![
            rect(10.0, 10.0, 300.0, 200.0),
            rect(-50.0, 0.0, 300.0, 150.0),
            rect(0.0, 0.0, 300.0, 100.0),
        ];
        let l = nest(&pieces, &opts(910.0, false)).unwrap();
        assert_relative_eq!(l.length, 200.0, epsilon = 1e-9);
        check_valid(&pieces, &l);
    }

    #[test]
    fn rotation_lets_long_piece_fit() {
        let pieces = vec![rect(0.0, 0.0, 800.0, 100.0)];
        assert!(matches!(
            nest(&pieces, &opts(500.0, false)),
            Err(Error::PieceTooWide { piece: 0, .. })
        ));
        let l = nest(&pieces, &opts(500.0, true)).unwrap();
        assert!(l.placements[0].rotated);
        assert_relative_eq!(l.length, 800.0, epsilon = 1e-9);
    }

    #[test]
    fn many_pieces_stay_inside_and_apart() {
        let pieces: Vec<Vec<DVec2>> = (0..12)
            .map(|i| {
                rect(
                    i as f64 * 7.0,
                    -(i as f64),
                    60.0 + 23.0 * (i % 5) as f64,
                    40.0 + 31.0 * (i % 3) as f64,
                )
            })
            .collect();
        let l = nest(&pieces, &opts(400.0, true)).unwrap();
        check_valid(&pieces, &l);
        assert!(
            l.utilisation > 0.5 && l.utilisation <= 1.0,
            "{}",
            l.utilisation
        );
    }

    fn check_valid(pieces: &[Vec<DVec2>], l: &Layout) {
        let boxes: Vec<(DVec2, DVec2)> = pieces
            .iter()
            .zip(&l.placements)
            .map(|(o, p)| placed(o, p))
            .collect();
        for (lo, hi) in &boxes {
            assert!(
                lo.x >= -1e-9
                    && hi.x <= l.sheet_width + 1e-9
                    && lo.y >= -1e-9
                    && hi.y <= l.length + 1e-9
            );
        }
        for i in 0..boxes.len() {
            for j in i + 1..boxes.len() {
                let (a, b) = (boxes[i], boxes[j]);
                let apart = a.1.x + 5.0 <= b.0.x + 1e-9
                    || b.1.x + 5.0 <= a.0.x + 1e-9
                    || a.1.y + 5.0 <= b.0.y + 1e-9
                    || b.1.y + 5.0 <= a.0.y + 1e-9;
                assert!(apart, "pieces {i} and {j} overlap: {a:?} {b:?}");
            }
        }
    }
}
