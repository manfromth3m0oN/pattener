//! One-call pattern job: mesh + seams + options → SVG documents and a
//! report.
//!
//! This is the orchestration shared by the command-line tool and the web
//! UI: it resolves the ease (uniform or from a swatch calibration),
//! flattens and matches seams, builds pattern pieces, renders SVGs,
//! optionally lays the pieces out on a sheet, and collects warnings. It
//! returns strings and plain data only; callers decide where they go.

use crate::arap::ArapOptions;
use crate::calibrate::{calibrate, Calibration, CalibrationResult};
use crate::cut::cut_panels;
use crate::ease::Ease;
use crate::error::{Error, Result};
use crate::flatten::{flatten_garment, FlatPanel, FlattenOptions, SeamMatching, SeamReport};
use crate::mesh::Mesh;
use crate::nest::{nest, NestOptions, Placement};
use crate::pattern::{build_pattern, PatternOptions};
use crate::seams::SeamSet;
use crate::svg::{layout_svg, panel_svg, EdgeLabel, HeatmapMetric, SvgOptions};
use glam::{DVec2, DVec3};

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
    /// Human-readable warnings.
    pub warnings: Vec<String>,
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
    let (ease, stiffness, panel_sheets) = match &opts.ease {
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

    let fopts = FlattenOptions {
        ease,
        arap: opts.arap,
        up: opts.up,
        seam_matching: opts.seam_matching,
        panel_stiffness: stiffness,
    };
    let garment = flatten_garment(mesh, seams, &fopts)?;
    let pieces = build_pattern(&garment, seams, &opts.pattern);

    let mut report = Report {
        calibration,
        panels: vec![],
        seams: vec![],
        layout: None,
    };
    let mut panel_svgs = Vec::new();
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
                label: Some(label),
                edge_labels: edge_labels(i, &garment.seams, seams),
                ..Default::default()
            },
        ));
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
        let outlines: Vec<_> = pieces.iter().map(|p| p.cut_outline.clone()).collect();
        let l = nest(&outlines, sheet)?;
        let labels: Vec<String> = (0..pieces.len()).map(|i| format!("panel {i}")).collect();
        layout = Some(layout_svg(&outlines, &labels, &l));
        report.layout = Some(LayoutReport {
            file: "layout.svg".into(),
            sheet_width_mm: sheet.sheet_width,
            length_mm: l.length,
            utilisation: l.utilisation,
            placements: l.placements,
        });
    }

    Ok(JobOutput {
        report,
        panel_svgs,
        layout_svg: layout,
        warnings,
    })
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
}
