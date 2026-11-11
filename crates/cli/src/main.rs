//! `latexpat` command-line interface. All file I/O lives here; the maths is
//! in `latexpat-core`.

// Index loops mirror the formulas; `!(a > b)` deliberately rejects NaN.
#![allow(clippy::needless_range_loop, clippy::neg_cmp_op_on_partial_ord)]

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use latexpat_core::arap::ArapOptions;
use latexpat_core::body::{torso, BodyMeasurements, BodyOptions, SeamStyle};
use latexpat_core::calibrate::{calibrate, Calibration, CalibrationResult, SheetFit};
use latexpat_core::garment::build;
use latexpat_core::glam::DVec3;
use latexpat_core::human::{fit, BodyModel, Measurer};
use latexpat_core::job::{self, EaseSpec, JobOptions, JobOutput, Report};
use latexpat_core::nest::NestOptions;
use latexpat_core::obj::{parse_obj, write_obj};
use latexpat_core::pattern::PatternOptions;
use latexpat_core::project::Project;
use latexpat_core::seams::AllowanceSide;
use latexpat_core::svg::HeatmapMetric;
use latexpat_core::{SeamMatching, SeamSet};
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
    /// Generate a torso mesh and seams from body measurements.
    Body(BodyArgs),
    /// Fit the full body model to the measurements in a project file.
    Human(HumanArgs),
}

#[derive(clap::Args)]
struct HumanArgs {
    /// Project file: {"body": {"measurements": {"height": 1680, ...}, "shape": {...}}}.
    project: PathBuf,
    /// Output directory (writes body.obj and measurements.json).
    #[arg(short, long)]
    output: PathBuf,
}

#[derive(clap::Args)]
struct BodyArgs {
    /// Measurements file: {"levels": [{"name", "height_mm", "circumference_mm"}, ...]}, bottom to top.
    measurements: PathBuf,
    /// Output directory (writes body.obj and body.seams.json).
    #[arg(short, long)]
    output: PathBuf,
    /// Seam layout.
    #[arg(long, value_enum, default_value_t = Style::Princess)]
    style: Style,
    /// Vertices around each ring (multiple of 4).
    #[arg(long, default_value_t = 64)]
    around: u32,
    /// Maximum distance between rings (mm).
    #[arg(long, default_value_t = 10.0)]
    ring_spacing: f64,
}

#[derive(Clone, Copy, ValueEnum)]
enum Style {
    /// Side seams only: front and back.
    Tube,
    /// Side seams and princess lines: six panels.
    Princess,
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
    #[arg(required_unless_present = "project")]
    mesh: Option<PathBuf>,
    /// Seam file: {"seams": [{"name": "...", "vertices": [0-based OBJ vertex indices]}]}.
    #[arg(long, conflicts_with = "project")]
    seams: Option<PathBuf>,
    /// Project file with a body and a garment drawn on it (instead of a mesh).
    #[arg(long, conflicts_with = "mesh")]
    project: Option<PathBuf>,
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

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Flatten(args) => flatten(args),
        Command::Calibrate(args) => calibrate_cmd(args),
        Command::Body(args) => body_cmd(args),
        Command::Human(args) => human_cmd(args),
    }
}

/// Fitted measurements more than this far (mm) from the request are flagged.
const FIT_WARNING_MM: f64 = 5.0;

fn human_cmd(args: HumanArgs) -> Result<()> {
    let t = fs::read_to_string(&args.project)
        .with_context(|| format!("reading {}", args.project.display()))?;
    let project: Project =
        serde_json::from_str(&t).with_context(|| format!("parsing {}", args.project.display()))?;
    let model = BodyModel::bundled();
    let measurer = Measurer::new(&model)?;
    let b = fit(
        &model,
        &measurer,
        &project.body.measurements,
        &project.body.shape,
    )?;
    fs::create_dir_all(&args.output)
        .with_context(|| format!("creating {}", args.output.display()))?;
    write(&args.output.join("body.obj"), &write_obj(&b.mesh))?;
    write(
        &args.output.join("measurements.json"),
        &serde_json::to_string_pretty(&b.measured)?,
    )?;

    println!("{:<14} {:>10} {:>10}", "measurement", "requested", "fitted");
    for m in &b.measured {
        let req = m
            .requested
            .map(|r| format!("{r:.1}"))
            .unwrap_or_else(|| "-".into());
        println!("{:<14} {:>10} {:>10.1}", m.kind.name(), req, m.value);
    }
    for m in &b.measured {
        if let Some(r) = m.requested {
            if (m.value - r).abs() > FIT_WARNING_MM {
                eprintln!(
                    "warning: {} fitted at {:.0} mm instead of {r:.0} mm; this combination is outside \
                     what the body model can reach (check the measurement, or adjust the shape options)",
                    m.kind.name(),
                    m.value
                );
            }
        }
    }
    println!(
        "fitted in {} iteration(s); wrote {} and measurements.json",
        b.iterations,
        args.output.join("body.obj").display()
    );
    Ok(())
}

fn body_cmd(args: BodyArgs) -> Result<()> {
    let t = fs::read_to_string(&args.measurements)
        .with_context(|| format!("reading {}", args.measurements.display()))?;
    let m: BodyMeasurements = serde_json::from_str(&t)
        .with_context(|| format!("parsing {}", args.measurements.display()))?;
    let b = torso(
        &m,
        &BodyOptions {
            around: args.around,
            ring_spacing_mm: args.ring_spacing,
            style: match args.style {
                Style::Tube => SeamStyle::Tube,
                Style::Princess => SeamStyle::Princess,
            },
        },
    )?;
    fs::create_dir_all(&args.output)
        .with_context(|| format!("creating {}", args.output.display()))?;
    let (obj, seams) = (
        args.output.join("body.obj"),
        args.output.join("body.seams.json"),
    );
    write(&obj, &write_obj(&b.mesh))?;
    write(&seams, &serde_json::to_string_pretty(&b.seams)?)?;
    println!(
        "torso: {} vertices, {} triangles, {} seams; levels on rings {:?}",
        b.mesh.positions().len(),
        b.mesh.triangles().len(),
        b.seams.seams.len(),
        b.level_rings
    );
    println!("wrote {} and {}", obj.display(), seams.display());
    println!(
        "next: latexpat flatten {} --seams {} -o pattern/",
        obj.display(),
        seams.display()
    );
    Ok(())
}

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
    let (mesh, seams, source, up) = match (&args.project, &args.mesh) {
        (Some(path), _) => {
            let t =
                fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            let project: Project =
                serde_json::from_str(&t).with_context(|| format!("parsing {}", path.display()))?;
            let model = BodyModel::bundled();
            let measurer = Measurer::new(&model)?;
            let body = fit(
                &model,
                &measurer,
                &project.body.measurements,
                &project.body.shape,
            )?;
            let g = build(&body.mesh, &project.garment)?;
            for w in &g.warnings {
                eprintln!("warning: {w}");
            }
            let mesh = g.mesh.with_context(|| {
                "the project's garment has no area yet: draw its edges and pick a point inside"
            })?;
            // The body model is Y-up.
            (mesh, g.seams, path.display().to_string(), DVec3::Y)
        }
        (None, Some(path)) => {
            let text =
                fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            let mut mesh =
                parse_obj(&text).with_context(|| format!("loading {}", path.display()))?;
            mesh.scale(args.scale);
            let seams: SeamSet = match &args.seams {
                Some(p) => {
                    let t = fs::read_to_string(p)
                        .with_context(|| format!("reading {}", p.display()))?;
                    serde_json::from_str(&t).with_context(|| format!("parsing {}", p.display()))?
                }
                None => SeamSet::default(),
            };
            (mesh, seams, path.display().to_string(), args.up.vec())
        }
        (None, None) => unreachable!("clap requires a mesh or a project"),
    };

    let ease = match &args.calibration {
        None => EaseSpec::Uniform(args.ease.unwrap_or(0.85)),
        Some(path) => {
            let t =
                fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            let calibration: Calibration =
                serde_json::from_str(&t).with_context(|| format!("parsing {}", path.display()))?;
            let mut panel_sheets = Vec::new();
            for ps in &args.panel_sheet {
                let (i, name) = ps
                    .split_once('=')
                    .with_context(|| format!("--panel-sheet '{ps}' must look like 2=0.6mm"))?;
                let i: usize = i
                    .trim()
                    .parse()
                    .with_context(|| format!("bad panel index in '{ps}'"))?;
                panel_sheets.push((i, name.to_string()));
            }
            EaseSpec::Calibrated {
                calibration,
                sheet: args.sheet.clone().expect("clap requires --sheet"),
                panel_sheets,
            }
        }
    };
    let opts = JobOptions {
        ease,
        arap: ArapOptions {
            max_iterations: args.max_iterations,
            tolerance: args.tolerance,
        },
        up,
        seam_matching: (!args.no_seam_matching).then_some(SeamMatching {
            weight: args.seam_weight,
            ..Default::default()
        }),
        seam_tolerance: args.seam_tolerance,
        pattern: PatternOptions {
            seam_allowance: args.glue_allowance,
            allowance_side: args.allowance_side.into(),
            hem_allowance: args.hem_allowance,
            notch_spacing: (args.notch_spacing > 0.0).then_some(args.notch_spacing),
        },
        heatmap: match args.heatmap {
            Heatmap::None => None,
            Heatmap::Max => Some(HeatmapMetric::MaxStretch),
            Heatmap::Area => Some(HeatmapMetric::AreaStretch),
        },
        heatmap_range: args.heatmap_range,
        sheet: args.sheet_width.map(|w| NestOptions {
            sheet_width: w,
            gap: args.sheet_gap,
            allow_rotation: !args.no_rotate,
        }),
    };
    let out = job::run(&mesh, &seams, &opts)?;
    write_job(&args.output, &source, args.scale, &out)
}

/// Write a job's files and print its summary tables and warnings.
fn write_job(dir: &Path, mesh: &str, scale: f64, out: &JobOutput) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let r = &out.report;
    println!(
        "{:<10} {:>6} {:>6} {:>9} {:>9} {:>9} {:>9} {:>17}  seams",
        "panel", "tris", "ease", "min σ", "p95 σ", "max σ", "area ×", "size (mm)"
    );
    for (p, svg) in r.panels.iter().zip(&out.panel_svgs) {
        write(&dir.join(&p.file), svg)?;
        println!(
            "{:<10} {:>6} {:>6.3} {:>9.4} {:>9.4} {:>9.4} {:>9.4} {:>8.1} × {:<6.1}  {}",
            p.index,
            p.triangles,
            p.ease,
            p.min_stretch,
            p.p95_stretch,
            p.max_stretch,
            p.mean_area_stretch,
            p.flat_width_mm,
            p.flat_height_mm,
            p.seams.join(", ")
        );
    }
    if !r.seams.is_empty() {
        println!();
        println!(
            "{:<22} {:>9} {:>9} {:>9} {:>9} {:>17}",
            "seam", "panels", "body", "left", "right", "mismatch (mm)"
        );
    }
    for s in &r.seams {
        println!(
            "{:<22} {:>9} {:>9.1} {:>9.1} {:>9.1} {:>+8.2} (was {:+.2})",
            s.seam,
            format!("{}↔{}", s.left_panel, s.right_panel),
            s.body_length_mm,
            s.left_length_mm,
            s.right_length_mm,
            s.mismatch_mm,
            s.mismatch_before_matching_mm
        );
    }
    if let (Some(l), Some(svg)) = (&r.layout, &out.layout_svg) {
        write(&dir.join(&l.file), svg)?;
        println!();
        println!(
            "layout: {:.0} mm of {:.0} mm wide sheet ({:.0}% used) → {}",
            l.length_mm,
            l.sheet_width_mm,
            100.0 * l.utilisation,
            l.file
        );
    }
    for w in &out.warnings {
        eprintln!("warning: {w}");
    }

    #[derive(Serialize)]
    struct FileReport<'a> {
        mesh: &'a str,
        scale: f64,
        #[serde(flatten)]
        report: &'a Report,
    }
    write(
        &dir.join("report.json"),
        &serde_json::to_string_pretty(&FileReport {
            mesh,
            scale,
            report: r,
        })?,
    )?;
    println!("wrote {} panel(s) to {}", r.panels.len(), dir.display());
    Ok(())
}

fn write(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}
