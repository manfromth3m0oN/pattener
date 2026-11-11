//! `latexpat` command-line interface. All file I/O lives here; the maths is
//! in `latexpat-core`.

// Index loops mirror the formulas; `!(a > b)` deliberately rejects NaN.
#![allow(clippy::needless_range_loop, clippy::neg_cmp_op_on_partial_ord)]

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use latexpat_core::arap::ArapOptions;
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
enum Command {
    /// Cut a body mesh along seams and flatten each panel to an SVG pattern.
    Flatten(FlattenArgs),
}

#[derive(clap::Args)]
struct FlattenArgs {
    /// Triangle/polygon mesh in OBJ format (millimetres unless --scale is given).
    mesh: PathBuf,
    /// Seam file: {"seams": [{"name": "...", "vertices": [0-based OBJ vertex indices]}]}.
    #[arg(long)]
    seams: Option<PathBuf>,
    /// Ease factor s: flat edge length = s × body edge length.
    #[arg(long, default_value_t = 0.85)]
    ease: f64,
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
    ease: f64,
    scale: f64,
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
    }
}

fn flatten(args: FlattenArgs) -> Result<()> {
    if !(args.ease > 0.0 && args.ease.is_finite()) {
        bail!("--ease must be a positive number");
    }
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

    let opts = FlattenOptions {
        ease: Ease::Uniform(args.ease),
        arap: ArapOptions {
            max_iterations: args.max_iterations,
            tolerance: args.tolerance,
        },
        up: args.up.vec(),
        seam_matching: (!args.no_seam_matching).then_some(SeamMatching {
            weight: args.seam_weight,
            ..Default::default()
        }),
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
        ease: args.ease,
        scale: args.scale,
        panels: vec![],
        seams: vec![],
        layout: None,
    };
    println!(
        "{:<10} {:>6} {:>9} {:>9} {:>9} {:>17}  seams",
        "panel", "tris", "min σ", "max σ", "area ×", "size (mm)"
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
            "{:<10} {:>6} {:>9.4} {:>9.4} {:>9.4} {:>8.1} × {:<6.1}  {}",
            i,
            fp.panel.triangles.len(),
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
