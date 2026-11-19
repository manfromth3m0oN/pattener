//! One-call pattern job: a flat design and options in, SVG documents, a
//! printable PDF and a report out.
//!
//! This is the orchestration shared by the command-line tool and the web
//! app: it works out each piece's ease (uniform, from its latex sheet or
//! from a swatch calibration), builds the pattern pieces, checks that the
//! seams match, renders the pieces, lays them out on the sheets, writes the
//! assembly guide and collects warnings. It returns strings and plain data
//! only; callers decide where they go.

use crate::allowance::polygon_area;
use crate::assembly::assembly_steps;
use crate::calibrate::{calibrate, Calibration, CalibrationResult};
use crate::design::{Design, SeamFinish};
use crate::error::{Error, Result};
use crate::geom::bounds;
use crate::materials::{MaterialUse, Sheet};
use crate::nest::{nest, NestOptions, Placement};
use crate::pattern::{
    build_pattern, trim_strips, PatternOptions, PatternPiece, Reduction, TrimStrip,
};
use crate::pdf::{pattern_pdf, piece_drawing, trim_drawing, PrintOptions};
use crate::svg::{layout_svg, piece_svg, trim_svg};
use glam::DVec2;

/// How the ease is chosen when a piece's sheet does not set it.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum EaseSpec {
    /// The same ease for every piece.
    Uniform(f64),
    /// From swatch tests. Each piece's sheet is looked up in the
    /// calibration by name, then by thickness; `sheet` (a name from the
    /// calibration or a thickness such as `"0.5mm"`) is used otherwise.
    Calibrated {
        calibration: Calibration,
        sheet: String,
    },
}

/// Everything a pattern job needs besides the design.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct JobOptions {
    pub ease: EaseSpec,
    /// Share of the reduction applied along the body (1 = the same as
    /// around it; 0.3–0.5 is typical for latex).
    pub length_relief: f64,
    /// Warn when a seam's sides differ by more than this (mm).
    pub seam_tolerance: f64,
    pub pattern: PatternOptions,
    /// Also lay every piece out on one sheet of this width.
    pub sheet: Option<NestOptions>,
    /// Also produce a tiled print-at-home PDF.
    pub print: Option<PrintOptions>,
    /// The latex sheets: each is laid out on its own roll.
    pub materials: Vec<Sheet>,
    /// Sheet for pieces that do not name one (default: the first).
    pub default_sheet: Option<String>,
}

impl Default for JobOptions {
    fn default() -> Self {
        Self {
            ease: EaseSpec::Uniform(0.85),
            length_relief: 1.0,
            seam_tolerance: 1.0,
            pattern: PatternOptions::default(),
            sheet: None,
            print: None,
            materials: Vec::new(),
            default_sheet: None,
        }
    }
}

/// Per-piece summary.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PieceReport {
    pub index: usize,
    pub name: String,
    pub file: String,
    /// How to cut it.
    pub cut: String,
    /// Number of pieces to cut (2 for a pair).
    pub copies: usize,
    pub folded: bool,
    pub sheet: Option<String>,
    /// Reduction around the body (the ease) and along it.
    pub ease: f64,
    pub ease_along: f64,
    pub seams: Vec<String>,
    pub notches: Vec<u32>,
    pub body_area_mm2: f64,
    pub flat_area_mm2: f64,
    pub flat_width_mm: f64,
    pub flat_height_mm: f64,
    pub cut_width_mm: f64,
    pub cut_height_mm: f64,
}

/// Per-seam summary: the lengths of its two sides.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeamReport {
    pub seam: String,
    pub zip: bool,
    pub a_piece: usize,
    pub b_piece: usize,
    /// Body-size lengths of side a and side b (mm).
    pub a_body_mm: f64,
    pub b_body_mm: f64,
    /// Flat (reduced) lengths (mm).
    pub a_flat_mm: f64,
    pub b_flat_mm: f64,
    /// `a_flat − b_flat` (mm).
    pub mismatch_mm: f64,
}

/// Sheet layout summary.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LayoutReport {
    pub file: String,
    pub sheet_width_mm: f64,
    pub length_mm: f64,
    pub utilisation: f64,
    pub placements: Vec<Placement>,
}

/// Everything measured about a job.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Report {
    pub calibration: Option<CalibrationResult>,
    pub pieces: Vec<PieceReport>,
    pub seams: Vec<SeamReport>,
    pub layout: Option<LayoutReport>,
    /// Use of each latex sheet.
    pub materials: Vec<MaterialUse>,
    /// Trim strips to cut (SVGs `trim_NN.svg`, in this order).
    pub trims: Vec<TrimStrip>,
    /// Gluing steps, in order.
    pub assembly: Vec<String>,
}

/// Result of [`run`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct JobOutput {
    pub report: Report,
    /// One SVG document per piece, named by `report.pieces[i].file`.
    pub piece_svgs: Vec<String>,
    /// Layout of every piece on one sheet, if [`JobOptions::sheet`] was set.
    pub layout_svg: Option<String>,
    /// One layout per latex sheet used, parallel to `report.materials`.
    pub material_layouts: Vec<String>,
    /// One SVG per trim strip, parallel to `report.trims`.
    pub trim_svgs: Vec<String>,
    /// The tiled PDF (plain ASCII), if [`JobOptions::print`] was set.
    pub pdf: Option<String>,
    /// Human-readable warnings.
    pub warnings: Vec<String>,
}

/// A piece's cut outline and, for a pair, its mirror image, with labels.
fn cut_copies(p: &PatternPiece) -> Vec<(Vec<DVec2>, String)> {
    let mut out = vec![(p.cut_outline.clone(), p.name.clone())];
    if p.pair {
        let mut m: Vec<DVec2> = p
            .cut_outline
            .iter()
            .map(|q| DVec2::new(-q.x, q.y))
            .collect();
        m.reverse();
        out.push((m, format!("{} (mirrored)", p.name)));
    }
    out
}

/// Run a complete pattern job.
pub fn run(design: &Design, opts: &JobOptions) -> Result<JobOutput> {
    if !(0.0..=1.0).contains(&opts.length_relief) {
        return Err(Error::InvalidEase(format!(
            "the lengthwise share of the reduction must be between 0 and 1, got {}",
            opts.length_relief
        )));
    }
    let mut warnings = Vec::new();
    for s in &opts.materials {
        s.validate()?;
    }

    // The ease when a sheet does not give its own.
    let mut calibration = None;
    let base_ease = match &opts.ease {
        EaseSpec::Uniform(e) => *e,
        EaseSpec::Calibrated {
            calibration: c,
            sheet,
        } => {
            let r = calibrate(c)?;
            for s in &r.sheets {
                if s.relative_error > 0.1 {
                    warnings.push(format!(
                        "sheet '{}' fits the rubber model poorly ({:.0}% error); re-check its measurements",
                        s.name,
                        100.0 * s.relative_error
                    ));
                }
            }
            let e = r.resolve(sheet)?.0;
            calibration = Some(r);
            e
        }
    };
    if !(base_ease > 0.3 && base_ease <= 1.2) {
        return Err(Error::InvalidEase(format!(
            "ease must be between 0.3 and 1.2, got {base_ease}"
        )));
    }

    // Each piece's sheet and ease.
    let default_sheet = opts
        .default_sheet
        .as_ref()
        .and_then(|n| opts.materials.iter().position(|s| &s.name == n))
        .unwrap_or(0);
    let mut piece_sheet: Vec<Option<usize>> = Vec::new();
    let mut eases = Vec::new();
    for p in &design.pieces {
        let si = if opts.materials.is_empty() {
            None
        } else {
            match &p.sheet {
                Some(name) => match opts.materials.iter().position(|s| &s.name == name) {
                    Some(i) => Some(i),
                    None => {
                        warnings.push(format!(
                            "piece '{}' uses sheet '{name}', which is not in the list; it is cut from '{}'",
                            p.name, opts.materials[default_sheet].name
                        ));
                        Some(default_sheet)
                    }
                },
                None => Some(default_sheet),
            }
        };
        let ease = si
            .map(|i| &opts.materials[i])
            .and_then(|s| {
                s.ease.or_else(|| {
                    let r = calibration.as_ref()?;
                    r.resolve(&s.name)
                        .or_else(|_| r.resolve(&format!("{}mm", s.thickness_mm)))
                        .ok()
                        .map(|x| x.0)
                })
            })
            .unwrap_or(base_ease);
        piece_sheet.push(si);
        eases.push(ease);
    }
    let reductions: Vec<Reduction> = eases
        .iter()
        .zip(&design.pieces)
        .map(|(&e, p)| Reduction::new(e, opts.length_relief).up_deg(p.up_deg))
        .collect();

    let pieces = build_pattern(design, &reductions, &opts.pattern)?;
    let trims = trim_strips(design, &pieces);
    let assembly = assembly_steps(design, &pieces, opts.pattern.seam_allowance, &trims);

    // Seams.
    let mut seams = Vec::new();
    for s in &design.seams {
        let (a, b) = (design.resolve(&s.a).unwrap(), design.resolve(&s.b).unwrap());
        let r = SeamReport {
            seam: if s.name.is_empty() {
                s.id.clone()
            } else {
                s.name.clone()
            },
            zip: s.finish == SeamFinish::Zip,
            a_piece: a.0,
            b_piece: b.0,
            a_body_mm: pieces[a.0].body_length(a.1),
            b_body_mm: pieces[b.0].body_length(b.1),
            a_flat_mm: pieces[a.0].flat_length(a.1),
            b_flat_mm: pieces[b.0].flat_length(b.1),
            mismatch_mm: pieces[a.0].flat_length(a.1) - pieces[b.0].flat_length(b.1),
        };
        if r.mismatch_mm.abs() > opts.seam_tolerance {
            let (long, short) = if r.mismatch_mm > 0.0 {
                (a.0, b.0)
            } else {
                (b.0, a.0)
            };
            // Why: the sides already differ at body size, or the reduction
            // shortens them differently (different eases, or edges running
            // at different angles to the body).
            let why = if (r.a_body_mm - r.b_body_mm).abs() > opts.seam_tolerance {
                format!(
                    "they already differ by {:.1} mm at body size",
                    (r.a_body_mm - r.b_body_mm).abs()
                )
            } else if (eases[a.0] - eases[b.0]).abs() > 1e-9 {
                format!(
                    "they match at body size, but {} and {} have different eases",
                    design.pieces[a.0].name, design.pieces[b.0].name
                )
            } else {
                "they match at body size, but run at different angles to the body, so the \
                 lengthwise reduction shortens them differently (check each piece's arrow points \
                 up the body)"
                    .to_string()
            };
            warnings.push(format!(
                "seam '{}': the side on {} is {:.1} mm longer than the side on {} (tolerance {} mm): \
                 {why}; reshape one side, or ease the longer side in",
                r.seam,
                design.pieces[long].name,
                r.mismatch_mm.abs(),
                design.pieces[short].name,
                opts.seam_tolerance
            ));
        }
        seams.push(r);
    }

    // Pieces.
    let roles = crate::pattern::edge_roles(design);
    let mut reports = Vec::new();
    let mut piece_svgs = Vec::new();
    let mut titles = Vec::new();
    for (i, p) in pieces.iter().enumerate() {
        let mut names: Vec<String> = Vec::new();
        for role in &roles[i] {
            if let crate::pattern::EdgeRole::Seam { seam, .. } = role {
                let n = &seams[*seam].seam;
                if !names.contains(n) {
                    names.push(n.clone());
                }
            }
        }
        let sheet = piece_sheet[i].map(|s| opts.materials[s].name.clone());
        let mut notes = vec![p.cut_text()];
        notes.push(format!(
            "ease {:.2} around, {:.2} along the body (arrow){}",
            p.reduction.around,
            p.reduction.along,
            sheet
                .as_ref()
                .map(|s| format!("; sheet: {s}"))
                .unwrap_or_default()
        ));
        piece_svgs.push(piece_svg(p, &p.name, &notes));
        titles.push(format!("{}: {}", p.name, p.cut_text()));
        if !p.cut_outline_simple {
            warnings.push(format!(
                "piece '{}': the allowance makes the cut line cross itself (an edge curves inwards \
                 more tightly than the allowance); check the pattern",
                p.name
            ));
        }
        let (lo, hi) = bounds(&p.seam_line);
        let (clo, chi) = bounds(&p.cut_outline);
        let body_area = p.area() / (p.reduction.around * p.reduction.along);
        let mut notches: Vec<u32> = p.notches.iter().map(|n| n.number).collect();
        notches.sort_unstable();
        notches.dedup();
        reports.push(PieceReport {
            index: i,
            name: p.name.clone(),
            file: format!("piece_{i:02}.svg"),
            cut: p.cut_text(),
            copies: p.copies(),
            folded: p.folded,
            sheet,
            ease: p.reduction.around,
            ease_along: p.reduction.along,
            seams: names,
            notches,
            body_area_mm2: body_area,
            flat_area_mm2: p.area(),
            flat_width_mm: hi.x - lo.x,
            flat_height_mm: hi.y - lo.y,
            cut_width_mm: chi.x - clo.x,
            cut_height_mm: chi.y - clo.y,
        });
    }

    // Every copy of every piece, and the trims, for the layouts.
    let mut all: Vec<(Vec<DVec2>, String, Option<usize>)> = Vec::new();
    for (i, p) in pieces.iter().enumerate() {
        for (o, l) in cut_copies(p) {
            all.push((o, l, piece_sheet[i]));
        }
    }
    for t in &trims {
        let sheet = t.pieces.first().and_then(|&p| piece_sheet[p]);
        all.push((t.outline(), format!("{} trim", t.edge), sheet));
    }

    let mut layout = None;
    let mut layout_doc = None;
    if let Some(sheet) = &opts.sheet {
        let outlines: Vec<Vec<DVec2>> = all.iter().map(|x| x.0.clone()).collect();
        let labels: Vec<String> = all.iter().map(|x| x.1.clone()).collect();
        let l = nest(&outlines, sheet)?;
        layout_doc = Some(layout_svg(&outlines, &labels, &l));
        layout = Some(LayoutReport {
            file: "layout.svg".into(),
            sheet_width_mm: sheet.sheet_width,
            length_mm: l.length,
            utilisation: l.utilisation,
            placements: l.placements,
        });
    }

    // One layout per latex sheet, on its roll width.
    let mut materials = Vec::new();
    let mut material_layouts = Vec::new();
    let (gap, allow_rotation) = opts
        .sheet
        .map_or((5.0, true), |s| (s.gap, s.allow_rotation));
    for (si, sheet) in opts.materials.iter().enumerate() {
        let mine: Vec<&(Vec<DVec2>, String, Option<usize>)> =
            all.iter().filter(|x| x.2 == Some(si)).collect();
        if mine.is_empty() {
            continue;
        }
        let outlines: Vec<Vec<DVec2>> = mine.iter().map(|x| x.0.clone()).collect();
        let labels: Vec<String> = mine.iter().map(|x| x.1.clone()).collect();
        let nest_opts = NestOptions {
            sheet_width: sheet.roll_width_mm,
            gap,
            allow_rotation,
        };
        let l = nest(&outlines, &nest_opts).map_err(|e| match e {
            Error::PieceTooWide {
                piece,
                width,
                height,
                ..
            } => Error::Material(format!(
                "{} ({width:.0} × {height:.0} mm) does not fit sheet '{}' ({:.0} mm wide); \
                 use a wider roll, allow rotation, or add a seam to split it",
                labels[piece], sheet.name, sheet.roll_width_mm
            )),
            other => other,
        })?;
        let area: f64 = outlines.iter().map(|o| polygon_area(o).abs()).sum::<f64>() / 1e6;
        materials.push(MaterialUse {
            sheet: sheet.name.clone(),
            color: sheet.color.clone(),
            thickness_mm: sheet.thickness_mm,
            roll_width_mm: sheet.roll_width_mm,
            pieces: (0..pieces.len())
                .filter(|&i| piece_sheet[i] == Some(si))
                .collect(),
            length_mm: l.length,
            pieces_area_m2: area,
            utilisation: l.utilisation,
            cost: sheet.price_per_m.map(|p| p * l.length / 1000.0),
            file: format!("layout_{}.svg", file_stem(&sheet.name)),
        });
        material_layouts.push(layout_svg(&outlines, &labels, &l));
    }

    let report = Report {
        calibration,
        pieces: reports,
        seams,
        layout,
        materials,
        trims: trims.clone(),
        assembly,
    };

    let pdf = opts.print.map(|print| {
        let mut info = vec![
            "Print at 100 % (actual size, no scaling), then check the 50 mm square.".to_string(),
            "Trim each page on its corner marks where it overlaps the next one, line the pieces up \
             and tape them together. Solid lines are cut lines; dashed lines show where the overlap \
             of a seam or hem starts; numbered ticks are notches; the arrow points up the body."
                .to_string(),
        ];
        info.push("Pieces:".into());
        for r in &report.pieces {
            info.push(format!("{}: {}", r.name, r.cut));
        }
        if !report.materials.is_empty() {
            info.push("Materials:".into());
            for u in &report.materials {
                let cost = u.cost.map(|c| format!(", cost {c:.2}")).unwrap_or_default();
                let names: Vec<&str> = u.pieces.iter().map(|&i| pieces[i].name.as_str()).collect();
                info.push(format!(
                    "{} ({:.2} mm): {:.2} m of {:.0} mm roll for {}{cost}",
                    u.sheet,
                    u.thickness_mm,
                    u.length_mm / 1000.0,
                    u.roll_width_mm,
                    names.join(", ")
                ));
            }
        }
        info.push("Assembly:".into());
        info.extend(
            report
                .assembly
                .iter()
                .enumerate()
                .map(|(k, step)| format!("{}. {step}", k + 1)),
        );
        let drawings: Vec<_> = pieces
            .iter()
            .zip(&titles)
            .map(|(p, t)| piece_drawing(t, p))
            .chain(trims.iter().map(|t| trim_drawing(&format!("{} trim", t.edge), t)))
            .collect();
        pattern_pdf("Latex pattern", &info, &drawings, &print)
    });

    Ok(JobOutput {
        report,
        piece_svgs,
        layout_svg: layout_doc,
        material_layouts,
        trim_svgs: trims.iter().map(trim_svg).collect(),
        pdf,
        warnings,
    })
}

/// A file-name-safe version of a sheet name.
fn file_stem(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    s.trim_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{template, TEMPLATES};
    use crate::body::BodySpec;

    fn sheet(name: &str, color: &str, t: f64, ease: Option<f64>, price: Option<f64>) -> Sheet {
        Sheet {
            name: name.into(),
            color: color.into(),
            thickness_mm: t,
            roll_width_mm: 1000.0,
            ease,
            shear_modulus_mpa: None,
            price_per_m: price,
        }
    }

    #[test]
    fn every_template_makes_a_pattern_without_warnings() {
        for t in TEMPLATES {
            let d = template(t.id, &BodySpec::default()).unwrap();
            let opts = JobOptions {
                print: Some(PrintOptions::default()),
                sheet: Some(NestOptions {
                    sheet_width: 1000.0,
                    gap: 5.0,
                    allow_rotation: true,
                }),
                materials: vec![sheet("black", "#111111", 0.4, None, None)],
                ..Default::default()
            };
            let out = run(&d, &opts).unwrap();
            assert!(out.warnings.is_empty(), "{}: {:?}", t.id, out.warnings);
            assert_eq!(out.piece_svgs.len(), d.pieces.len());
            assert!(out.pdf.as_ref().unwrap().starts_with("%PDF"));
            assert!(out.layout_svg.is_some());
            assert_eq!(out.material_layouts.len(), 1);
            assert!(out.report.assembly.len() >= 3, "{:?}", out.report.assembly);
            // Every copy of every piece is laid out.
            let copies: usize = out.report.pieces.iter().map(|p| p.copies).sum();
            let placed = out.report.layout.as_ref().unwrap().placements.len();
            assert_eq!(placed, copies + out.report.trims.len(), "{}", t.id);
        }
    }

    #[test]
    fn sheets_set_the_ease_and_have_their_own_layouts() {
        let mut d = template("top-sleeves", &BodySpec::default()).unwrap();
        d.pieces[2].sheet = Some("red 0.60".into());
        let opts = JobOptions {
            materials: vec![
                sheet("black 0.40", "#111111", 0.4, None, Some(20.0)),
                sheet("red 0.60", "#b01020", 0.6, Some(0.9), None),
            ],
            ..Default::default()
        };
        let out = run(&d, &opts).unwrap();
        let r = &out.report;
        assert_eq!(r.pieces[0].ease, 0.85);
        assert_eq!(r.pieces[2].ease, 0.9);
        assert_eq!(r.pieces[2].sheet.as_deref(), Some("red 0.60"));
        assert_eq!(r.materials.len(), 2);
        assert_eq!(r.materials[0].pieces, vec![0, 1]);
        assert_eq!(r.materials[1].pieces, vec![2]);
        assert_eq!(r.materials[0].file, "layout_black_0.40.svg");
        let cost = r.materials[0].cost.unwrap();
        assert!((cost - 20.0 * r.materials[0].length_mm / 1000.0).abs() < 1e-9);
        // The sleeve is reduced less than the armholes it is sewn to.
        assert!(
            out.warnings
                .iter()
                .any(|w| w.contains("armhole") && w.contains("different eases")),
            "{:?}",
            out.warnings
        );

        // A roll narrower than a piece is reported by sheet and piece.
        let mut narrow = opts.clone();
        narrow.materials[1].roll_width_mm = 50.0;
        let err = run(&d, &narrow).unwrap_err();
        assert!(
            err.to_string().contains("does not fit sheet 'red 0.60'"),
            "{err}"
        );
    }

    #[test]
    fn lengthwise_relief_and_bad_options() {
        let d = template("leggings", &BodySpec::default()).unwrap();
        let opts = JobOptions {
            length_relief: 0.4,
            ..Default::default()
        };
        let out = run(&d, &opts).unwrap();
        let p = &out.report.pieces[0];
        assert!((p.ease_along - 0.94).abs() < 1e-12);
        assert!(p.cut.contains("pair"));

        // A sleeve cap and its armhole run at different angles: with less
        // reduction along the body they come out different lengths, and
        // the warning says why.
        let d = template("top-sleeves", &BodySpec::default()).unwrap();
        let out = run(&d, &opts).unwrap();
        assert!(
            out.warnings
                .iter()
                .any(|w| w.contains("armhole") && w.contains("different angles")),
            "{:?}",
            out.warnings
        );
        // A piece drawn on its side, with its arrow turned to match, comes
        // out the same as drawn upright.
        let mut turned = template("skirt", &BodySpec::default()).unwrap();
        for n in &mut turned.pieces[0].nodes {
            (n.x, n.y) = (-n.y, n.x);
        }
        turned.pieces[0].up_deg = 90.0;
        let upright = run(&template("skirt", &BodySpec::default()).unwrap(), &opts).unwrap();
        let turned = run(&turned, &opts).unwrap();
        let (a, b) = (&upright.report.pieces[0], &turned.report.pieces[0]);
        assert!((a.flat_area_mm2 - b.flat_area_mm2).abs() < 1e-3);
        assert!((a.flat_width_mm - b.flat_height_mm).abs() < 1e-6);
        assert!(turned.warnings.is_empty(), "{:?}", turned.warnings);
        assert!(run(
            &d,
            &JobOptions {
                length_relief: 2.0,
                ..Default::default()
            }
        )
        .is_err());
        assert!(run(
            &d,
            &JobOptions {
                ease: EaseSpec::Uniform(0.1),
                ..Default::default()
            }
        )
        .is_err());
        assert!(run(&Design::default(), &JobOptions::default()).is_err());
    }
}
