//! `latexpat` command-line interface. All file I/O lives here; the
//! drafting is in `latexpat-core`.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use latexpat_core::blocks::{redraft, template, TEMPLATES};
use latexpat_core::body::{Body, BodySpec, Figure, Measure, Measurements};
use latexpat_core::calibrate::{calibrate, Calibration, CalibrationResult};
use latexpat_core::job::{self, EaseSpec, JobOutput};
use latexpat_core::materials::Sheet;
use latexpat_core::pdf::Paper;
use latexpat_core::project::Project;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "latexpat",
    version,
    about = "Flat pattern drafting for latex (rubber sheet) garments"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the templates.
    Templates,
    /// Show the body a set of measurements describes, with estimates for
    /// the ones not given.
    Body(BodyArgs),
    /// Start a project from a template drafted for a body.
    Template(TemplateArgs),
    /// Move a project's design to new measurements, keeping its edits.
    Redraft(RedraftArgs),
    /// Make the pattern of a project: SVGs, layouts, PDF and a report.
    Pattern(PatternArgs),
    /// Fit swatch stretch tests and print the ease for each latex sheet.
    Calibrate(CalibrateArgs),
}

#[derive(Clone, Copy, ValueEnum)]
enum FigureArg {
    Women,
    Men,
}

impl From<FigureArg> for Figure {
    fn from(f: FigureArg) -> Self {
        match f {
            FigureArg::Women => Figure::Women,
            FigureArg::Men => Figure::Men,
        }
    }
}

/// Where measurements come from.
#[derive(clap::Args)]
struct BodySource {
    /// Measurements file: {"height": 1680, "bust": 940, ...} in mm.
    #[arg(long, conflicts_with = "project")]
    measurements: Option<PathBuf>,
    /// Take the body from this project file instead.
    #[arg(long)]
    project: Option<PathBuf>,
    /// Figure the missing measurements are estimated from.
    #[arg(long, value_enum)]
    figure: Option<FigureArg>,
}

impl BodySource {
    fn load(&self) -> Result<BodySpec> {
        let mut spec = if let Some(p) = &self.project {
            // Only the body: a 3D editor project's measurements still count.
            let p: Project = serde_json::from_str(&read(p)?)
                .with_context(|| format!("parsing {}", p.display()))?;
            p.body
        } else if let Some(m) = &self.measurements {
            let t = read(m)?;
            let measurements: Measurements =
                serde_json::from_str(&t).with_context(|| format!("parsing {}", m.display()))?;
            BodySpec {
                measurements,
                ..Default::default()
            }
        } else {
            BodySpec::default()
        };
        if let Some(f) = self.figure {
            spec.figure = f.into();
        }
        Ok(spec)
    }
}

#[derive(clap::Args)]
struct BodyArgs {
    #[command(flatten)]
    body: BodySource,
}

#[derive(clap::Args)]
struct TemplateArgs {
    /// Template id (see `latexpat templates`).
    id: String,
    #[command(flatten)]
    body: BodySource,
    /// Project file to write.
    #[arg(short, long)]
    output: PathBuf,
}

#[derive(clap::Args)]
struct RedraftArgs {
    /// Project drafted from a template.
    project: PathBuf,
    /// New measurements file: {"height": 1680, ...} in mm.
    #[arg(long)]
    measurements: PathBuf,
    /// Project file to write (default: overwrite the input).
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum PaperArg {
    A4,
    Letter,
    None,
}

#[derive(clap::Args)]
struct PatternArgs {
    /// Project file.
    project: PathBuf,
    /// Output directory.
    #[arg(short, long)]
    output: PathBuf,
    /// Ease (flat size / body size around the body); overrides the project.
    #[arg(long)]
    ease: Option<f64>,
    /// Share of the reduction along the body, 0–1; overrides the project.
    #[arg(long)]
    length_relief: Option<f64>,
    /// Paper for the printable PDF; overrides the project.
    #[arg(long, value_enum)]
    pdf: Option<PaperArg>,
    /// Also lay every piece out on one sheet this wide (mm).
    #[arg(long)]
    sheet_width: Option<f64>,
    /// Swatch calibration file; the ease then comes from each piece's sheet.
    #[arg(long, requires = "sheet")]
    calibration: Option<PathBuf>,
    /// Sheet for pieces whose own sheet is not in the calibration: a name
    /// from the calibration file or a thickness such as 0.5mm.
    #[arg(long)]
    sheet: Option<String>,
}

#[derive(clap::Args)]
struct CalibrateArgs {
    /// Swatch file: {"sheets": [{"name", "thickness_mm", "samples": [...]}], "target": ...}.
    file: PathBuf,
    /// Also print the ease for these untested thicknesses (mm), from the pooled fit.
    #[arg(long, value_delimiter = ',')]
    thickness: Vec<f64>,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Templates => {
            for t in TEMPLATES {
                println!("{:<16} {}: {}", t.id, t.name, t.description);
            }
            Ok(())
        }
        Command::Body(args) => body_cmd(args),
        Command::Template(args) => template_cmd(args),
        Command::Redraft(args) => redraft_cmd(args),
        Command::Pattern(args) => pattern_cmd(args),
        Command::Calibrate(args) => calibrate_cmd(args),
    }
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

fn write(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}

fn read_project(path: &Path) -> Result<Project> {
    let p: Project = serde_json::from_str(&read(path)?)
        .with_context(|| format!("parsing {}", path.display()))?;
    if p.is_3d() {
        bail!(
            "{} was made with the 3D editor, which has been replaced by the flat editor; \
             start a new design from a template with its measurements (latexpat template ID \
             --project {})",
            path.display(),
            path.display()
        );
    }
    Ok(p)
}

fn body_cmd(args: BodyArgs) -> Result<()> {
    let b = Body::new(&args.body.load()?)?;
    println!("{:<14} {:>8}", "measurement", "mm");
    for k in Measure::ALL {
        let note = if b.estimated.contains(&k) {
            "  (estimated)"
        } else {
            ""
        };
        println!("{:<14} {:>8.0}{note}", k.name(), b.get(k));
    }
    println!();
    println!("{:<14} {:>8}", "level", "height");
    for (name, y) in b.guides() {
        println!("{name:<14} {y:>8.0}");
    }
    Ok(())
}

fn template_cmd(args: TemplateArgs) -> Result<()> {
    let body = args.body.load()?;
    let design = template(&args.id, &body)?;
    let info = TEMPLATES.iter().find(|t| t.id == args.id).unwrap();
    let p = Project {
        name: info.name.into(),
        body,
        design,
        materials: vec![Sheet {
            name: "black 0.40".into(),
            color: "#151515".into(),
            thickness_mm: 0.4,
            roll_width_mm: 1000.0,
            ease: None,
            shear_modulus_mpa: None,
            price_per_m: None,
        }],
        ..Default::default()
    };
    write(&args.output, &serde_json::to_string_pretty(&p)?)?;
    println!(
        "{}: {} pieces, {} seams → {}",
        info.name,
        p.design.pieces.len(),
        p.design.seams.len(),
        args.output.display()
    );
    Ok(())
}

fn redraft_cmd(args: RedraftArgs) -> Result<()> {
    let mut p = read_project(&args.project)?;
    if p.design.block.is_none() {
        bail!("{} was not drafted from a template", args.project.display());
    }
    let measurements: Measurements = serde_json::from_str(&read(&args.measurements)?)
        .with_context(|| format!("parsing {}", args.measurements.display()))?;
    p.body.measurements = measurements;
    p.design = redraft(&p.design, &p.body)?;
    let out = args.output.as_ref().unwrap_or(&args.project);
    write(out, &serde_json::to_string_pretty(&p)?)?;
    println!("redrafted → {}", out.display());
    Ok(())
}

fn load_calibration(path: &Path) -> Result<(Calibration, CalibrationResult)> {
    let c: Calibration = serde_json::from_str(&read(path)?)
        .with_context(|| format!("parsing {}", path.display()))?;
    let r = calibrate(&c)?;
    Ok((c, r))
}

fn pattern_cmd(args: PatternArgs) -> Result<()> {
    let mut p = read_project(&args.project)?;
    if let Some(e) = args.ease {
        p.settings.ease = e;
    }
    if let Some(r) = args.length_relief {
        p.settings.length_relief = r;
    }
    if let Some(paper) = args.pdf {
        p.settings.paper = match paper {
            PaperArg::A4 => Some(Paper::A4),
            PaperArg::Letter => Some(Paper::Letter),
            PaperArg::None => None,
        };
    }
    if args.sheet_width.is_some() {
        p.settings.sheet_width_mm = args.sheet_width;
    }
    let mut opts = p.job_options();
    if let Some(c) = &args.calibration {
        let (calibration, _) = load_calibration(c)?;
        opts.ease = EaseSpec::Calibrated {
            calibration,
            sheet: args.sheet.clone().unwrap_or_default(),
        };
    }
    let out = job::run(&p.design, &opts)?;
    write_job(&args.output, &out)
}

fn write_job(dir: &Path, out: &JobOutput) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let r = &out.report;
    println!(
        "{:<16} {:<10} {:>6} {:>6} {:>17}  seams",
        "piece", "cut", "ease", "along", "cut size (mm)"
    );
    for (p, svg) in r.pieces.iter().zip(&out.piece_svgs) {
        write(&dir.join(&p.file), svg)?;
        let cut = match (p.copies, p.folded) {
            (2, _) => "pair",
            (_, true) => "on fold",
            _ => "1",
        };
        println!(
            "{:<16} {:<10} {:>6.3} {:>6.3} {:>8.0} × {:<6.0}  {}",
            p.name,
            cut,
            p.ease,
            p.ease_along,
            p.cut_width_mm,
            p.cut_height_mm,
            p.seams.join(", ")
        );
    }
    if !r.seams.is_empty() {
        println!();
        println!(
            "{:<18} {:>10} {:>10} {:>10}",
            "seam", "body (mm)", "flat (mm)", "mismatch"
        );
        for s in &r.seams {
            println!(
                "{:<18} {:>10.1} {:>10.1} {:>10.2}",
                s.seam, s.a_body_mm, s.a_flat_mm, s.mismatch_mm
            );
        }
    }
    if let Some(svg) = &out.layout_svg {
        write(&dir.join("layout.svg"), svg)?;
    }
    for (u, svg) in r.materials.iter().zip(&out.material_layouts) {
        write(&dir.join(&u.file), svg)?;
        println!(
            "{}: {:.2} m of a {:.0} mm roll ({:.0} % used){}",
            u.sheet,
            u.length_mm / 1000.0,
            u.roll_width_mm,
            100.0 * u.utilisation,
            u.cost.map(|c| format!(", cost {c:.2}")).unwrap_or_default()
        );
    }
    for (i, svg) in out.trim_svgs.iter().enumerate() {
        write(&dir.join(format!("trim_{i:02}.svg")), svg)?;
    }
    if let Some(pdf) = &out.pdf {
        write(&dir.join("pattern.pdf"), pdf)?;
    }
    write(&dir.join("report.json"), &serde_json::to_string_pretty(r)?)?;
    if !r.assembly.is_empty() {
        println!();
        println!("assembly:");
        for (k, s) in r.assembly.iter().enumerate() {
            println!("{:>3}. {s}", k + 1);
        }
    }
    for w in &out.warnings {
        eprintln!("warning: {w}");
    }
    println!();
    println!("wrote {}", dir.display());
    Ok(())
}

fn calibrate_cmd(args: CalibrateArgs) -> Result<()> {
    let (_, r) = load_calibration(&args.file)?;
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
        if s.relative_error > 0.1 {
            eprintln!(
                "warning: sheet '{}' fits the rubber model poorly ({:.0}% error); re-check its measurements",
                s.name,
                100.0 * s.relative_error
            );
        }
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
