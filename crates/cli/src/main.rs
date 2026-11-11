//! `latexpat` command-line interface. All file I/O lives here; the maths is
//! in `latexpat-core`.

// Index loops mirror the formulas; `!(a > b)` deliberately rejects NaN.
#![allow(clippy::needless_range_loop, clippy::neg_cmp_op_on_partial_ord)]

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use latexpat_core::arap::ArapOptions;
use latexpat_core::calibrate::{calibrate, Calibration, CalibrationResult, SheetFit};
use latexpat_core::cut::cut_panels;
use latexpat_core::glam::{DVec2, DVec3};
use latexpat_core::nest::{nest, Layout, NestOptions};
use latexpat_core::obj::parse_obj;
use latexpat_core::pattern::{build_pattern, PatternOptions};
use latexpat_core::seams::AllowanceSide;
use latexpat_core::svg::{layout_svg, panel_svg, EdgeLabel, HeatmapMetric, SvgOptions};
use latexpat_core::{
    flatten_garment, Ease, FlatPanel, FlattenOptions, SeamMatching, SeamReport, SeamSet,
};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "latexpat",
    version,
    about = "Pattern generator for latex (rubber sheet) garments"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // parsed once; boxing buys nothing
enum Command {
    /// Cut a body mesh along seams and flatten each panel to an SVG pattern.
    Flatten(FlattenArgs),
    /// Fit swatch stretch tests and print the ease for each latex sheet.
    Calibrate(CalibrateArgs),
}

#[derive(clap::Args)]
struct CalibrateArgs {
    /// Swatch file: {"sheets": [{"name", "thickness_mm", "samples": [...]}], "target": ...}.
    file: PathBuf,
    /// Also print the ease for these untested thicknesses (mm), from the pooled fit.
    #[arg(long)]
    thickness: Vec<f64>,
}

#[derive(clap::Args)]
struct FlattenArgs {
    /// Triangle/polygon mesh in OBJ format (millimetres unless --scale is given).
    mesh: PathBuf,
    /// Seam file: {"seams": [{"name": "...", "vertices": [0-based OBJ vertex indices]}]}.
    #[arg(long)]
    seams: Option<PathBuf>,
    /// Ease factor s: flat edge length = s × body edge length (default 0.85).
    #[arg(long, conflicts_with = "calibration")]
    ease: Option<f64>,
    /// Swatch calibration file; the ease then comes from the chosen sheet.
    #[arg(long, requires = "sheet")]
    calibration: Option<PathBuf>,
    /// Sheet for all panels: a sheet name from the calibration file, or a
    /// thickness such as 0.5mm (uses the pooled fit).
    #[arg(long, requires = "calibration")]
    sheet: Option<String>,
    /// Use a different sheet for one panel, e.g. --panel-sheet 2=0.6mm (repeatable).
    #[arg(long, value_name = "PANEL=SHEET", requires = "calibration")]
    panel_sheet: Vec<String>,
    /// Output directory.
    #[arg(short, long)]
    output: PathBuf,
    /// Multiply mesh coordinates by this to get millimetres (10 for cm, 1000 for m).
    #[arg(long, default_value_t = 1.0)]
    scale: f64,
    /// Body axis that should point up the page.
    #[arg(long, value_enum, default_value_t = Axis::Y)]
    up: Axis,
    /// Fill triangles by stretch.
    #[arg(long, value_enum, default_value_t = Heatmap::None)]
    heatmap: Heatmap,
    /// Per-direction stretch at which heatmap colours saturate.
    #[arg(long, default_value_t = 1.5)]
    heatmap_range: f64,
    /// Maximum ARAP iterations.
    #[arg(long, default_value_t = 500)]
    max_iterations: usize,
    /// ARAP relative energy tolerance.
    #[arg(long, default_value_t = 1e-8)]
    tolerance: f64,
    /// Do not equalise the flat lengths of the two sides of each seam.
    #[arg(long)]
    no_seam_matching: bool,
    /// Weight of the seam-length constraints during matching.
    #[arg(long, default_value_t = 10.0)]
    seam_weight: f64,
    /// Warn when the two sides of a seam differ by more than this (mm).
    #[arg(long, default_value_t = 1.0)]
    seam_tolerance: f64,
    /// Glue overlap added to one side of each seam (mm).
    #[arg(long, default_value_t = 6.0)]
    glue_allowance: f64,
    /// Which side of a seam carries the glue overlap (seams can override this).
    #[arg(long, value_enum, default_value_t = Side::Left)]
    allowance_side: Side,
    /// Allowance on open edges such as hems and necklines (mm).
    #[arg(long, default_value_t = 0.0)]
    hem_allowance: f64,
    /// Body distance between automatic notches (mm); 0 for explicit notches only.
    #[arg(long, default_value_t = 50.0)]
    notch_spacing: f64,
    /// Lay all pieces out on a sheet of this width (mm) and write layout.svg.
    #[arg(long)]
    sheet_width: Option<f64>,
    /// Gap between pieces on the sheet (mm).
    #[arg(long, default_value_t = 5.0)]
    sheet_gap: f64,
    /// Do not rotate pieces by 90° when laying them out.
    #[arg(long)]
    no_rotate: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Side {
    Left,
    Right,
    Both,
    None,
}

impl From<Side> for AllowanceSide {
    fn from(s: Side) -> Self {
        match s {
            Side::Left => AllowanceSide::Left,
            Side::Right => AllowanceSide::Right,
            Side::Both => AllowanceSide::Both,
            Side::None => AllowanceSide::None,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Axis {
    X,
    Y,
    Z,
    #[value(name = "-x")]
    NegX,
    #[value(name = "-y")]
    NegY,
    #[value(name = "-z")]
    NegZ,
}

impl Axis {
    fn vec(self) -> DVec3 {
        match self {
            Axis::X => DVec3::X,
            Axis::Y => DVec3::Y,
            Axis::Z => DVec3::Z,
            Axis::NegX => DVec3::NEG_X,
            Axis::NegY => DVec3::NEG_Y,
            Axis::NegZ => DVec3::NEG_Z,
        }
    }
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Heatmap {
    None,
    /// Largest principal stretch.
    Max,
    /// Area stretch (product of principal stretches).
    Area,
}

#[derive(Serialize)]
struct Report {
    mesh: String,
    scale: f64,
    calibration: Option<CalibrationResult>,
    panels: Vec<PanelReport>,
    seams: Vec<SeamPairReport>,
    layout: Option<LayoutReport>,
}

#[derive(Serialize)]
struct LayoutReport {
    file: String,
    sheet_width_mm: f64,
    length_mm: f64,
    utilisation: f64,
    placements: Vec<latexpat_core::nest::Placement>,
}

#[derive(Serialize)]
struct SeamPairReport {
    seam: String,
    left_panel: usize,
    right_panel: usize,
    edges: usize,
    body_length_mm: f64,
    left_length_mm: f64,
    right_length_mm: f64,
    mismatch_mm: f64,
    mismatch_before_matching_mm: f64,
}

#[derive(Serialize)]
struct PanelReport {
    index: usize,
    file: String,
    sheet: Option<String>,
    ease: f64,
    seams: Vec<String>,
    vertices: usize,
    triangles: usize,
    min_stretch: f64,
    max_stretch: f64,
    mean_area_stretch: f64,
    body_area_mm2: f64,
    flat_area_mm2: f64,
    flat_width_mm: f64,
    flat_height_mm: f64,
    cut_width_mm: f64,
    cut_height_mm: f64,
    notches: Vec<u32>,
    flipped_triangles: Vec<u32>,
    arap_iterations: usize,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Flatten(args) => flatten(args),
        Command::Calibrate(args) => calibrate_cmd(args),
    }
}

/// Ease spec, per-panel stiffness, and per-panel (ease, sheet label).
type PanelSetup = (Ease, Option<Vec<f64>>, Vec<(f64, Option<String>)>);

fn load_calibration(path: &Path) -> Result<CalibrationResult> {
    let t = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let c: Calibration =
        serde_json::from_str(&t).with_context(|| format!("parsing {}", path.display()))?;
    Ok(calibrate(&c)?)
}

fn calibrate_cmd(args: CalibrateArgs) -> Result<()> {
    let r = load_calibration(&args.file)?;
    println!("target wearing tension: {:.3} N/cm", r.tension_n_per_cm);
    println!();
    println!(
        "{:<20} {:>9} {:>10} {:>9} {:>7}",
        "sheet", "t (mm)", "μ (MPa)", "fit err", "ease"
    );
    for s in &r.sheets {
        println!(
            "{:<20} {:>9.3} {:>10.3} {:>8.1}% {:>7.3}",
            s.name,
            s.thickness_mm,
            s.shear_modulus_mpa,
            100.0 * s.relative_error,
            s.ease
        );
        warn_fit(s);
    }
    println!();
    println!(
        "pooled μ = {:.3} MPa (all sheets together)",
        r.pooled_shear_modulus_mpa
    );
    for t in args.thickness {
        println!("  {t:.3} mm → ease {:.3}", r.ease_for_thickness(t)?);
    }
    Ok(())
}

fn warn_fit(s: &SheetFit) {
    if s.relative_error > 0.1 {
        eprintln!(
            "warning: sheet '{}' fits the rubber model poorly ({:.0}% error); re-check its measurements",
            s.name,
            100.0 * s.relative_error
        );
    }
}

/// Resolve a sheet name, or a thickness like "0.5mm", to (ease, stiffness μt, label).
fn resolve_sheet(r: &CalibrationResult, spec: &str) -> Result<(f64, f64, String)> {
    if let Some(s) = r.sheet(spec) {
        return Ok((s.ease, s.shear_modulus_mpa * s.thickness_mm, s.name.clone()));
    }
    let num = spec.trim().trim_end_matches("mm").trim();
    if let Ok(t) = num.parse::<f64>() {
        return Ok((
            r.ease_for_thickness(t)?,
            r.pooled_shear_modulus_mpa * t,
            format!("{t} mm (pooled fit)"),
        ));
    }
    let names: Vec<&str> = r.sheets.iter().map(|s| s.name.as_str()).collect();
    bail!("unknown sheet '{spec}'; use one of {names:?} or a thickness such as 0.5mm")
}

fn flatten(args: FlattenArgs) -> Result<()> {
    if !(args.scale > 0.0 && args.scale.is_finite()) {
        bail!("--scale must be a positive number");
    }
    for (name, v) in [
        ("--glue-allowance", args.glue_allowance),
        ("--hem-allowance", args.hem_allowance),
        ("--notch-spacing", args.notch_spacing),
        ("--sheet-gap", args.sheet_gap),
    ] {
        if !(v >= 0.0 && v.is_finite()) {
            bail!("{name} must be zero or a positive number");
        }
    }
    if !(args.heatmap_range > 1.0) {
        bail!("--heatmap-range must be greater than 1");
    }
    let text = fs::read_to_string(&args.mesh)
        .with_context(|| format!("reading {}", args.mesh.display()))?;
    let mut mesh = parse_obj(&text).with_context(|| format!("loading {}", args.mesh.display()))?;
    mesh.scale(args.scale);

    let seams: SeamSet = match &args.seams {
        Some(p) => {
            let t = fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
            serde_json::from_str(&t).with_context(|| format!("parsing {}", p.display()))?
        }
        None => SeamSet::default(),
    };

    // Ease (and stiffness) per panel: uniform, or from the calibrated sheets.
    let n_panels = cut_panels(&mesh, &seams.resolve(&mesh)?)?.len();
    let mut calibration = None;
    let (ease, stiffness, panel_sheets): PanelSetup = match &args.calibration {
        None => {
            let e = args.ease.unwrap_or(0.85);
            if !(e > 0.0 && e.is_finite()) {
                bail!("--ease must be a positive number");
            }
            (Ease::Uniform(e), None, vec![(e, None); n_panels])
        }
        Some(path) => {
            let r = load_calibration(path)?;
            r.sheets.iter().for_each(warn_fit);
            let default = args.sheet.as_deref().expect("clap requires --sheet");
            let mut specs = vec![default.to_string(); n_panels];
            for ps in &args.panel_sheet {
                let (i, name) = ps
                    .split_once('=')
                    .with_context(|| format!("--panel-sheet '{ps}' must look like 2=0.6mm"))?;
                let i: usize = i
                    .trim()
                    .parse()
                    .with_context(|| format!("bad panel index in '{ps}'"))?;
                if i >= n_panels {
                    bail!("--panel-sheet: panel {i} does not exist (the seams make {n_panels} panels)");
                }
                specs[i] = name.to_string();
            }
            let mut eases = Vec::new();
            let mut k = Vec::new();
            let mut info = Vec::new();
            for spec in &specs {
                let (e, stiff, label) = resolve_sheet(&r, spec)?;
                eases.push(e);
                k.push(stiff);
                info.push((e, Some(label)));
            }
            calibration = Some(r);
            (Ease::per_panel(&mesh, &seams, &eases)?, Some(k), info)
        }
    };

    let opts = FlattenOptions {
        ease,
        arap: ArapOptions {
            max_iterations: args.max_iterations,
            tolerance: args.tolerance,
        },
        up: args.up.vec(),
        seam_matching: (!args.no_seam_matching).then_some(SeamMatching {
            weight: args.seam_weight,
            ..Default::default()
        }),
        panel_stiffness: stiffness,
    };
    let garment = flatten_garment(&mesh, &seams, &opts)?;
    let panels = &garment.panels;
    let pieces = build_pattern(
        &garment,
        &seams,
        &PatternOptions {
            seam_allowance: args.glue_allowance,
            allowance_side: args.allowance_side.into(),
            hem_allowance: args.hem_allowance,
            notch_spacing: (args.notch_spacing > 0.0).then_some(args.notch_spacing),
        },
    );

    fs::create_dir_all(&args.output)
        .with_context(|| format!("creating {}", args.output.display()))?;
    let heatmap = match args.heatmap {
        Heatmap::None => None,
        Heatmap::Max => Some(HeatmapMetric::MaxStretch),
        Heatmap::Area => Some(HeatmapMetric::AreaStretch),
    };

    let mut report = Report {
        mesh: args.mesh.display().to_string(),
        scale: args.scale,
        calibration,
        panels: vec![],
        seams: vec![],
        layout: None,
    };
    println!(
        "{:<10} {:>6} {:>6} {:>9} {:>9} {:>9} {:>17}  seams",
        "panel", "tris", "ease", "min σ", "max σ", "area ×", "size (mm)"
    );
    for (i, fp) in panels.iter().enumerate() {
        let names = seam_names(fp, &seams);
        let file = format!("panel_{i:02}.svg");
        let label = if names.is_empty() {
            format!("panel {i}")
        } else {
            format!("panel {i} — {}", names.join(", "))
        };
        let svg = panel_svg(
            fp,
            &pieces[i],
            &SvgOptions {
                heatmap,
                range: args.heatmap_range,
                label: Some(label),
                edge_labels: edge_labels(i, &garment.seams, &seams),
                ..Default::default()
            },
        );
        write(&args.output.join(&file), &svg)?;

        let size = fp.uv.iter().fold(DVec2::ZERO, |m, p| m.max(*p));
        let cut_size = {
            let o = &pieces[i].cut_outline;
            let lo = o.iter().fold(DVec2::splat(f64::INFINITY), |m, p| m.min(*p));
            o.iter().fold(DVec2::ZERO, |m, p| m.max(*p - lo))
        };
        let s = &fp.strain;
        println!(
            "{:<10} {:>6} {:>6.3} {:>9.4} {:>9.4} {:>9.4} {:>8.1} × {:<6.1}  {}",
            i,
            fp.panel.triangles.len(),
            panel_sheets[i].0,
            s.min_stretch,
            s.max_stretch,
            s.mean_area_stretch,
            size.x,
            size.y,
            names.join(", ")
        );
        if !pieces[i].cut_outline_simple {
            eprintln!(
                "warning: panel {i}: the allowance makes the cut line cross itself \
                 (an edge curves inwards more tightly than the allowance); check the SVG"
            );
        }
        if !s.flipped.is_empty() {
            eprintln!(
                "warning: panel {i} has {} flipped triangle(s) in the flat layout",
                s.flipped.len()
            );
        }
        report.panels.push(PanelReport {
            index: i,
            file,
            sheet: panel_sheets[i].1.clone(),
            ease: panel_sheets[i].0,
            seams: names,
            vertices: fp.panel.vertex_count(),
            triangles: fp.panel.triangles.len(),
            min_stretch: s.min_stretch,
            max_stretch: s.max_stretch,
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
    if !garment.seams.is_empty() {
        println!();
        println!(
            "{:<16} {:>9} {:>9} {:>9} {:>9} {:>17}",
            "seam", "panels", "body", "left", "right", "mismatch (mm)"
        );
    }
    for sr in &garment.seams {
        let (l, r) = (sr.pair.left.panel, sr.pair.right.panel);
        let name = seam_name(&seams, sr.pair.seam);
        println!(
            "{:<16} {:>9} {:>9.1} {:>9.1} {:>9.1} {:>+8.2} (was {:+.2})",
            name,
            format!("{l}↔{r}"),
            sr.lengths.body,
            sr.lengths.left,
            sr.lengths.right,
            sr.lengths.mismatch(),
            sr.unmatched.mismatch()
        );
        if sr.lengths.mismatch().abs() > args.seam_tolerance {
            eprintln!(
                "warning: seam '{name}' between panels {l} and {r}: sides differ by {:.2} mm \
                 (tolerance {} mm); the longer side will need easing in",
                sr.lengths.mismatch().abs(),
                args.seam_tolerance
            );
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

    if let Some(width) = args.sheet_width {
        let outlines: Vec<_> = pieces.iter().map(|p| p.cut_outline.clone()).collect();
        let layout: Layout = nest(
            &outlines,
            &NestOptions {
                sheet_width: width,
                gap: args.sheet_gap,
                allow_rotation: !args.no_rotate,
            },
        )?;
        let labels: Vec<String> = (0..pieces.len()).map(|i| format!("panel {i}")).collect();
        write(
            &args.output.join("layout.svg"),
            &layout_svg(&outlines, &labels, &layout),
        )?;
        println!();
        println!(
            "layout: {:.0} mm of {:.0} mm wide sheet ({:.0}% used) → layout.svg",
            layout.length,
            width,
            100.0 * layout.utilisation
        );
        report.layout = Some(LayoutReport {
            file: "layout.svg".into(),
            sheet_width_mm: width,
            length_mm: layout.length,
            utilisation: layout.utilisation,
            placements: layout.placements,
        });
    }

    write(
        &args.output.join("report.json"),
        &serde_json::to_string_pretty(&report)?,
    )?;
    println!(
        "wrote {} panel(s) to {}",
        panels.len(),
        args.output.display()
    );
    Ok(())
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

fn write(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}
