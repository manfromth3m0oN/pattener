//! One-call pattern job: mesh + seams + options → SVG documents and a
//! report.
//!
//! This is the orchestration shared by the command-line tool and the web
//! UI: it resolves the ease (uniform or from a swatch calibration),
//! flattens and matches seams, builds pattern pieces, renders SVGs,
//! optionally lays the pieces out on a sheet, and collects warnings. It
//! returns strings and plain data only; callers decide where they go.

use crate::allowance::polygon_area;
use crate::arap::ArapOptions;
use crate::assembly::assembly_steps;
use crate::calibrate::{calibrate, Calibration, CalibrationResult};
use crate::cut::cut_panels;
use crate::ease::{apply_zones, Ease, EaseZone};
use crate::error::{Error, Result};
use crate::fitmap::{pressure_map, FitMap, Material};
use crate::flatten::{flatten_garment, FlatPanel, FlattenOptions, SeamMatching, SeamReport};
use crate::materials::{MaterialPlan, MaterialUse, DEFAULT_SHEAR_MODULUS_MPA};
use crate::mesh::Mesh;
use crate::nest::{nest, NestOptions, Placement};
use crate::pattern::{build_pattern, trim_strips, EdgeSpec, PatternOptions, TrimStrip};
use crate::pdf::{pattern_pdf, piece_drawing, trim_drawing, PrintOptions};
use crate::seams::SeamSet;
use crate::svg::{layout_svg, panel_svg, trim_svg, EdgeLabel, HeatmapMetric, SvgOptions};
use glam::{DVec2, DVec3};

/// Pressure (mmHg) above which 5 % of a piece triggers a warning: the
/// start of the strongest medical compression class (about 34–46 mmHg).
const HIGH_PRESSURE_MMHG: f64 = 40.0;

/// How the ease is chosen.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum EaseSpec {
    /// The same ease for every panel.
    Uniform(f64),
    /// From swatch tests: `sheet` for every panel, except panels listed in
    /// `panel_sheets` as `(panel index, sheet)`. A sheet is a name from the
    /// calibration or a thickness such as `"0.5mm"` (pooled fit).
    Calibrated {
        calibration: Calibration,
        sheet: String,
        #[cfg_attr(feature = "serde", serde(default))]
        panel_sheets: Vec<(usize, String)>,
    },
}

/// Everything a pattern job needs besides the mesh and seams.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct JobOptions {
    pub ease: EaseSpec,
    pub arap: ArapOptions,
    /// Body axis that points up the page.
    pub up: DVec3,
    /// `None` to keep each panel's independent flattening.
    pub seam_matching: Option<SeamMatching>,
    /// Warn when a seam's sides still differ by more than this (mm).
    pub seam_tolerance: f64,
    pub pattern: PatternOptions,
    pub heatmap: Option<HeatmapMetric>,
    pub heatmap_range: f64,
    /// Lay pieces out on a sheet.
    pub sheet: Option<NestOptions>,
    /// Fraction of the around-the-body reduction applied along the body
    /// (1 = the same in both directions; 0.3–0.5 is typical for latex).
    pub length_relief: f64,
    /// "Along the body" per mesh triangle (set by callers that know the
    /// body, e.g. from its bones); `None` uses `up`.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub along: Option<Vec<DVec3>>,
    /// Regions with their own ease (see [`EaseZone`]).
    pub ease_zones: Vec<EaseZone>,
    /// Latex used for the pressure map when no calibration is given.
    pub material: Material,
    /// Which sheet each panel is cut from (overrides per-panel ease and
    /// stiffness, and lays each sheet out on its own roll).
    pub materials: Option<MaterialPlan>,
    /// Finishes of the garment's open edges (hems, trims).
    pub edges: Vec<EdgeSpec>,
    /// Also produce a tiled print-at-home PDF.
    pub print: Option<PrintOptions>,
}

impl Default for JobOptions {
    fn default() -> Self {
        Self {
            ease: EaseSpec::Uniform(0.85),
            arap: ArapOptions::default(),
            up: DVec3::Y,
            seam_matching: Some(SeamMatching::default()),
            seam_tolerance: 1.0,
            pattern: PatternOptions::default(),
            heatmap: None,
            heatmap_range: 1.5,
            sheet: None,
            length_relief: 1.0,
            along: None,
            ease_zones: Vec::new(),
            material: Material::default(),
            materials: None,
            edges: Vec::new(),
            print: None,
        }
    }
}

/// Per-panel summary.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanelReport {
    pub index: usize,
    pub file: String,
    pub sheet: Option<String>,
    pub ease: f64,
    pub seams: Vec<String>,
    pub vertices: usize,
    pub triangles: usize,
    pub min_stretch: f64,
    pub max_stretch: f64,
    /// 95 % of the piece's area stretches less than this.
    pub p95_stretch: f64,
    /// Fraction of the piece's area that is slack in some direction.
    pub slack_fraction: f64,
    /// Mean pressure on the body (mmHg).
    pub mean_pressure_mmhg: f64,
    /// 95 % of the piece's area presses less than this (mmHg).
    pub p95_pressure_mmhg: f64,
    /// Fraction of the piece's area that bridges over a hollow.
    pub bridging_fraction: f64,
    pub mean_area_stretch: f64,
    pub body_area_mm2: f64,
    pub flat_area_mm2: f64,
    pub flat_width_mm: f64,
    pub flat_height_mm: f64,
    pub cut_width_mm: f64,
    pub cut_height_mm: f64,
    pub notches: Vec<u32>,
    pub flipped_triangles: Vec<u32>,
    pub arap_iterations: usize,
}

/// Per-seam-pair summary.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeamPairReport {
    pub seam: String,
    pub left_panel: usize,
    pub right_panel: usize,
    pub edges: usize,
    pub body_length_mm: f64,
    pub left_length_mm: f64,
    pub right_length_mm: f64,
    pub mismatch_mm: f64,
    pub mismatch_before_matching_mm: f64,
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
    pub panels: Vec<PanelReport>,
    pub seams: Vec<SeamPairReport>,
    pub layout: Option<LayoutReport>,
    /// Use of each sheet of the material plan.
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
    /// One SVG document per panel, named by `report.panels[i].file`.
    pub panel_svgs: Vec<String>,
    /// Sheet layout SVG, if a sheet was given.
    pub layout_svg: Option<String>,
    /// One layout per sheet of the material plan, parallel to
    /// `report.materials`.
    pub material_layouts: Vec<String>,
    /// One SVG per trim strip, parallel to `report.trims`.
    pub trim_svgs: Vec<String>,
    /// The tiled PDF (plain ASCII), if [`JobOptions::print`] was set.
    pub pdf: Option<String>,
    /// Human-readable warnings.
    pub warnings: Vec<String>,
    /// Pressure and bridging map on the input mesh.
    pub fit: FitMap,
}

/// Run a complete pattern job.
pub fn run(mesh: &Mesh, seams: &SeamSet, opts: &JobOptions) -> Result<JobOutput> {
    if !(opts.heatmap_range > 1.0) {
        return Err(Error::InvalidEase(
            "heatmap range must be greater than 1".into(),
        ));
    }
    let mut warnings = Vec::new();

    // Ease and stiffness per panel.
    let n_panels = cut_panels(mesh, &seams.resolve(mesh)?)?.len();
    let mut calibration = None;
    let (mut ease, mut stiffness, mut panel_sheets) = match &opts.ease {
        EaseSpec::Uniform(e) => {
            if !(*e > 0.0 && e.is_finite()) {
                return Err(Error::InvalidEase(format!(
                    "ease must be positive, got {e}"
                )));
            }
            (Ease::Uniform(*e), None, vec![(*e, None); n_panels])
        }
        EaseSpec::Calibrated {
            calibration: c,
            sheet,
            panel_sheets,
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
            let mut specs = vec![sheet.clone(); n_panels];
            for (i, name) in panel_sheets {
                if *i >= n_panels {
                    return Err(Error::Calibration(format!(
                        "panel {i} does not exist (the seams make {n_panels} panels)"
                    )));
                }
                specs[*i] = name.clone();
            }
            let mut eases = Vec::new();
            let mut k = Vec::new();
            let mut info = Vec::new();
            for spec in &specs {
                let (e, stiff, label) = r.resolve(spec)?;
                eases.push(e);
                k.push(stiff);
                info.push((e, Some(label)));
            }
            calibration = Some(r);
            (Ease::per_panel(mesh, seams, &eases)?, Some(k), info)
        }
    };

    // Sheets from the material plan override the per-panel ease and
    // stiffness: the sheet's own ease, else its calibration (by name, then
    // by thickness), else the job's ease.
    if let Some(plan) = &opts.materials {
        plan.validate(n_panels)?;
        let mut eases = Vec::with_capacity(n_panels);
        let mut k = Vec::with_capacity(n_panels);
        let mut info = Vec::with_capacity(n_panels);
        for &si in &plan.panel_sheet {
            let sheet = &plan.sheets[si];
            let calibrated = calibration.as_ref().and_then(|r| {
                r.resolve(&sheet.name)
                    .or_else(|_| r.resolve(&format!("{}mm", sheet.thickness_mm)))
                    .ok()
            });
            let e = match (sheet.ease, &calibrated, &opts.ease) {
                (Some(e), _, _) => e,
                (None, Some((e, _, _)), _) => *e,
                (None, None, EaseSpec::Uniform(e)) => *e,
                (None, None, EaseSpec::Calibrated { .. }) => {
                    unreachable!("calibration resolves thicknesses")
                }
            };
            let stiff = match (sheet.shear_modulus_mpa, &calibrated) {
                (Some(mu), _) => mu * sheet.thickness_mm,
                (None, Some((_, k, _))) => *k,
                (None, None) => DEFAULT_SHEAR_MODULUS_MPA * sheet.thickness_mm,
            };
            eases.push(e);
            k.push(stiff);
            info.push((e, Some(sheet.name.clone())));
        }
        ease = Ease::per_panel(mesh, seams, &eases)?;
        stiffness = Some(k);
        panel_sheets = info;
    }

    let ease = if opts.ease_zones.is_empty() {
        ease
    } else {
        apply_zones(mesh, &ease, &opts.ease_zones)?
    };
    // Membrane stiffness per panel for the pressure map.
    let pressure_k: Vec<f64> = stiffness
        .clone()
        .unwrap_or_else(|| vec![opts.material.stiffness(); n_panels]);

    let fopts = FlattenOptions {
        ease,
        arap: opts.arap,
        up: opts.up,
        seam_matching: opts.seam_matching,
        panel_stiffness: stiffness,
        length_relief: opts.length_relief,
        along: opts.along.clone(),
    };
    let garment = flatten_garment(mesh, seams, &fopts)?;
    let fit = pressure_map(mesh, &garment, &pressure_k);
    let pieces = build_pattern(&garment, seams, &opts.pattern, &opts.edges);
    let trims = trim_strips(&garment, &opts.edges);

    let mut report = Report {
        calibration,
        panels: vec![],
        seams: vec![],
        layout: None,
        materials: vec![],
        trims: trims.clone(),
        assembly: assembly_steps(&garment, &pieces, seams, &opts.edges, &trims),
    };
    let mut panel_svgs = Vec::new();
    let mut titles = Vec::new();
    for (i, fp) in garment.panels.iter().enumerate() {
        let names = seam_names(fp, seams);
        let label = if names.is_empty() {
            format!("panel {i}")
        } else {
            format!("panel {i} — {}", names.join(", "))
        };
        panel_svgs.push(panel_svg(
            fp,
            &pieces[i],
            &SvgOptions {
                heatmap: opts.heatmap,
                range: opts.heatmap_range,
                label: Some(label.clone()),
                edge_labels: edge_labels(i, &garment.seams, seams),
                ..Default::default()
            },
        ));
        titles.push(label);
        let size = fp.uv.iter().fold(DVec2::ZERO, |m, p| m.max(*p));
        let cut_size = {
            let o = &pieces[i].cut_outline;
            let lo = o.iter().fold(DVec2::splat(f64::INFINITY), |m, p| m.min(*p));
            o.iter().fold(DVec2::ZERO, |m, p| m.max(*p - lo))
        };
        let s = &fp.strain;
        if !pieces[i].cut_outline_simple {
            warnings.push(format!(
                "panel {i}: the allowance makes the cut line cross itself \
                 (an edge curves inwards more tightly than the allowance); check the SVG"
            ));
        }
        let pf = fit.panels[i];
        if pf.p95_pressure_mmhg > HIGH_PRESSURE_MMHG {
            warnings.push(format!(
                "panel {i} presses hard: 5 % of it exceeds {:.0} mmHg; loosen the ease there (an ease zone) \
                 or use thinner latex",
                pf.p95_pressure_mmhg
            ));
        }
        if !s.flipped.is_empty() {
            warnings.push(format!(
                "panel {i} has {} flipped triangle(s) in the flat layout",
                s.flipped.len()
            ));
        }
        report.panels.push(PanelReport {
            index: i,
            file: format!("panel_{i:02}.svg"),
            sheet: panel_sheets[i].1.clone(),
            ease: panel_sheets[i].0,
            seams: names,
            vertices: fp.panel.vertex_count(),
            triangles: fp.panel.triangles.len(),
            min_stretch: s.min_stretch,
            max_stretch: s.max_stretch,
            p95_stretch: s.p95_stretch,
            slack_fraction: s.slack_fraction,
            mean_pressure_mmhg: pf.mean_pressure_mmhg,
            p95_pressure_mmhg: pf.p95_pressure_mmhg,
            bridging_fraction: pf.bridging_fraction,
            mean_area_stretch: s.mean_area_stretch,
            body_area_mm2: s.body_area,
            flat_area_mm2: s.flat_area,
            flat_width_mm: size.x,
            flat_height_mm: size.y,
            cut_width_mm: cut_size.x,
            cut_height_mm: cut_size.y,
            notches: pieces[i].notches.iter().map(|n| n.number).collect(),
            flipped_triangles: s.flipped.clone(),
            arap_iterations: fp.arap_iterations,
        });
    }

    for sr in &garment.seams {
        let (l, r) = (sr.pair.left.panel, sr.pair.right.panel);
        let name = seam_name(seams, sr.pair.seam);
        if sr.lengths.mismatch().abs() > opts.seam_tolerance {
            warnings.push(format!(
                "seam '{name}' between panels {l} and {r}: sides differ by {:.2} mm \
                 (tolerance {} mm); the longer side will need easing in",
                sr.lengths.mismatch().abs(),
                opts.seam_tolerance
            ));
        }
        report.seams.push(SeamPairReport {
            seam: name,
            left_panel: l,
            right_panel: r,
            edges: sr.pair.left.edges.len(),
            body_length_mm: sr.lengths.body,
            left_length_mm: sr.lengths.left,
            right_length_mm: sr.lengths.right,
            mismatch_mm: sr.lengths.mismatch(),
            mismatch_before_matching_mm: sr.unmatched.mismatch(),
        });
    }

    let mut layout = None;
    if let Some(sheet) = &opts.sheet {
        let outlines: Vec<_> = pieces
            .iter()
            .map(|p| p.cut_outline.clone())
            .chain(trims.iter().map(|t| t.outline()))
            .collect();
        let l = nest(&outlines, sheet)?;
        let labels: Vec<String> = (0..pieces.len())
            .map(|i| format!("panel {i}"))
            .chain(trims.iter().map(|t| format!("{} trim", t.edge)))
            .collect();
        layout = Some(layout_svg(&outlines, &labels, &l));
        report.layout = Some(LayoutReport {
            file: "layout.svg".into(),
            sheet_width_mm: sheet.sheet_width,
            length_mm: l.length,
            utilisation: l.utilisation,
            placements: l.placements,
        });
    }

    // One layout per sheet of the material plan, on its roll width.
    let mut material_layouts = Vec::new();
    if let Some(plan) = &opts.materials {
        let (gap, allow_rotation) = opts
            .sheet
            .map_or((5.0, true), |s| (s.gap, s.allow_rotation));
        for (si, sheet) in plan.sheets.iter().enumerate() {
            let ids: Vec<usize> = (0..pieces.len())
                .filter(|&i| plan.panel_sheet[i] == si)
                .collect();
            let trim_ids: Vec<usize> = (0..trims.len())
                .filter(|&t| {
                    trims[t]
                        .panels
                        .first()
                        .is_some_and(|&p| plan.panel_sheet[p] == si)
                })
                .collect();
            if ids.is_empty() && trim_ids.is_empty() {
                continue;
            }
            let outlines: Vec<_> = ids
                .iter()
                .map(|&i| pieces[i].cut_outline.clone())
                .chain(trim_ids.iter().map(|&t| trims[t].outline()))
                .collect();
            let nest_opts = NestOptions {
                sheet_width: sheet.roll_width_mm,
                gap,
                allow_rotation,
            };
            let names: Vec<String> = ids
                .iter()
                .map(|i| format!("panel {i}"))
                .chain(trim_ids.iter().map(|&t| format!("{} trim", trims[t].edge)))
                .collect();
            let l = nest(&outlines, &nest_opts).map_err(|e| match e {
                Error::PieceTooWide {
                    piece,
                    width,
                    height,
                    ..
                } => Error::Material(format!(
                    "{} ({width:.0} × {height:.0} mm) does not fit sheet '{}' ({:.0} mm wide); \
                     use a wider roll, allow rotation, or add a seam to split it",
                    names[piece], sheet.name, sheet.roll_width_mm
                )),
                other => other,
            })?;
            let labels = names;
            let area: f64 = outlines.iter().map(|o| polygon_area(o).abs()).sum::<f64>() / 1e6;
            let file = format!("layout_{}.svg", file_stem(&sheet.name));
            report.materials.push(MaterialUse {
                sheet: sheet.name.clone(),
                color: sheet.color.clone(),
                thickness_mm: sheet.thickness_mm,
                roll_width_mm: sheet.roll_width_mm,
                pieces: ids,
                length_mm: l.length,
                pieces_area_m2: area,
                utilisation: l.utilisation,
                cost: sheet.price_per_m.map(|p| p * l.length / 1000.0),
                file,
            });
            material_layouts.push(layout_svg(&outlines, &labels, &l));
        }
    }

    let pdf = opts.print.map(|print| {
        let mut info = vec![
            "Print at 100 % (actual size, no scaling), then check the 50 mm square.".to_string(),
            "Trim each page on its corner marks where it overlaps the next one, line the \
             pieces up and tape them together. Solid lines are cut lines; dashed lines show \
             where the overlap of a seam or hem starts; numbered ticks are notches."
                .to_string(),
        ];
        if !report.materials.is_empty() {
            info.push("Materials:".into());
            for u in &report.materials {
                let cost = u.cost.map(|c| format!(", cost {c:.2}")).unwrap_or_default();
                info.push(format!(
                    "{} ({:.2} mm): {:.2} m of {:.0} mm roll for pieces {:?}{cost}",
                    u.sheet,
                    u.thickness_mm,
                    u.length_mm / 1000.0,
                    u.roll_width_mm,
                    u.pieces
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
        let drawings: Vec<_> = garment
            .panels
            .iter()
            .zip(&pieces)
            .zip(&titles)
            .map(|((fp, piece), t)| piece_drawing(t, fp, piece))
            .chain(
                trims
                    .iter()
                    .map(|t| trim_drawing(&format!("{} trim", t.edge), t)),
            )
            .collect();
        pattern_pdf("Latex pattern", &info, &drawings, &print)
    });

    Ok(JobOutput {
        report,
        panel_svgs,
        layout_svg: layout,
        material_layouts,
        trim_svgs: trims.iter().map(trim_svg).collect(),
        pdf,
        warnings,
        fit,
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

fn seam_name(seams: &SeamSet, id: u32) -> String {
    seams.seams[id as usize]
        .name
        .clone()
        .unwrap_or_else(|| format!("seam {id}"))
}

/// Labels for panel `panel`'s seam edges: "name ↔ panel N".
fn edge_labels(panel: usize, pairs: &[SeamReport], seams: &SeamSet) -> Vec<EdgeLabel> {
    let mut out = Vec::new();
    for sr in pairs {
        let name = seam_name(seams, sr.pair.seam);
        let (l, r) = (&sr.pair.left, &sr.pair.right);
        for (side, other) in [(l, r.panel), (r, l.panel)] {
            if side.panel == panel {
                let text = if other == panel {
                    format!("{name} (joins itself)")
                } else {
                    format!("{name} ↔ panel {other}")
                };
                out.push(EdgeLabel {
                    edges: side.edges.clone(),
                    text,
                });
            }
        }
    }
    out
}

/// Names of the seams on a panel's boundary, in loop order, deduplicated.
fn seam_names(fp: &FlatPanel, seams: &SeamSet) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in &fp.panel.boundary {
        if let Some(id) = e.seam {
            let name = seam_name(seams, id);
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{torso, BodyMeasurements, BodyOptions, Level, SeamStyle};

    #[test]
    fn job_on_generated_torso() {
        let l = |name: &str, h: f64, c: f64| Level {
            name: name.into(),
            height_mm: h,
            circumference_mm: c,
            ..Default::default()
        };
        let m = BodyMeasurements {
            levels: vec![
                l("hip", 0.0, 960.0),
                l("waist", 220.0, 720.0),
                l("bust", 420.0, 900.0),
            ],
        };
        let b = torso(
            &m,
            &BodyOptions {
                around: 32,
                ring_spacing_mm: 25.0,
                style: SeamStyle::Princess,
            },
        )
        .unwrap();
        let opts = JobOptions {
            heatmap: Some(HeatmapMetric::MaxStretch),
            sheet: Some(NestOptions {
                sheet_width: 600.0,
                gap: 5.0,
                allow_rotation: true,
            }),
            ..Default::default()
        };
        let out = run(&b.mesh, &b.seams, &opts).unwrap();
        assert_eq!(out.panel_svgs.len(), 6);
        assert_eq!(out.report.panels.len(), 6);
        assert_eq!(out.report.seams.len(), 6);
        assert!(out.layout_svg.is_some());
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    #[test]
    fn material_plan_sets_ease_and_lays_out_each_sheet() {
        use crate::materials::{MaterialPlan, Sheet};
        let l = |name: &str, h: f64, c: f64| Level {
            name: name.into(),
            height_mm: h,
            circumference_mm: c,
            ..Default::default()
        };
        let m = BodyMeasurements {
            levels: vec![
                l("hip", 0.0, 960.0),
                l("waist", 220.0, 720.0),
                l("bust", 420.0, 900.0),
            ],
        };
        let b = torso(
            &m,
            &BodyOptions {
                around: 32,
                ring_spacing_mm: 25.0,
                style: SeamStyle::Princess,
            },
        )
        .unwrap();
        let sheet =
            |name: &str, color: &str, t: f64, ease: Option<f64>, price: Option<f64>| Sheet {
                name: name.into(),
                color: color.into(),
                thickness_mm: t,
                roll_width_mm: 1000.0,
                ease,
                shear_modulus_mpa: None,
                price_per_m: price,
            };
        let plan = MaterialPlan {
            sheets: vec![
                sheet("black 0.40", "#111111", 0.4, None, Some(20.0)),
                sheet("red 0.60", "#b01020", 0.6, Some(0.9), None),
            ],
            panel_sheet: vec![0, 0, 0, 1, 1, 0],
        };
        let opts = JobOptions {
            materials: Some(plan.clone()),
            ..Default::default()
        };
        let out = run(&b.mesh, &b.seams, &opts).unwrap();
        let r = &out.report;
        for (i, p) in r.panels.iter().enumerate() {
            let (name, ease) = if plan.panel_sheet[i] == 0 {
                ("black 0.40", 0.85)
            } else {
                ("red 0.60", 0.9)
            };
            assert_eq!(p.sheet.as_deref(), Some(name));
            assert!((p.ease - ease).abs() < 1e-12);
        }
        assert_eq!(r.materials.len(), 2);
        assert_eq!(out.material_layouts.len(), 2);
        assert_eq!(r.materials[0].pieces, vec![0, 1, 2, 5]);
        assert_eq!(r.materials[1].pieces, vec![3, 4]);
        for u in &r.materials {
            assert!(
                u.length_mm > 100.0 && u.pieces_area_m2 > 0.01 && u.utilisation <= 1.0,
                "{u:?}"
            );
        }
        let cost = r.materials[0].cost.unwrap();
        assert!((cost - 20.0 * r.materials[0].length_mm / 1000.0).abs() < 1e-9);
        assert!(r.materials[1].cost.is_none());
        assert_eq!(r.materials[0].file, "layout_black_0.40.svg");

        // A roll narrower than a piece is reported by sheet and panel.
        let mut narrow = plan.clone();
        narrow.sheets[1].roll_width_mm = 50.0;
        let err = run(
            &b.mesh,
            &b.seams,
            &JobOptions {
                materials: Some(narrow),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("does not fit sheet 'red 0.60'"),
            "{err}"
        );
        // One assignment per panel is required.
        let mut short = plan;
        short.panel_sheet.pop();
        assert!(run(
            &b.mesh,
            &b.seams,
            &JobOptions {
                materials: Some(short),
                ..Default::default()
            }
        )
        .is_err());
    }
}
