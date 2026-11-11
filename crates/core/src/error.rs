//! Error type shared by every stage of the pipeline.

use thiserror::Error;

/// Everything that can go wrong between loading a mesh and exporting a pattern.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum Error {
    /// The OBJ text could not be parsed.
    #[error("OBJ parse error on line {line}: {msg}")]
    Obj { line: usize, msg: String },

    /// A triangle refers to a vertex that does not exist.
    #[error(
        "triangle {triangle} references vertex {vertex}, but the mesh has only {count} vertices"
    )]
    VertexOutOfRange {
        triangle: usize,
        vertex: u32,
        count: usize,
    },

    /// A triangle repeats a vertex or has (numerically) zero area.
    #[error("triangle {triangle} is degenerate (repeated vertex or zero area)")]
    DegenerateTriangle { triangle: usize },

    /// More than two triangles share an edge.
    #[error("edge ({a}, {b}) is shared by more than two triangles (non-manifold)")]
    NonManifoldEdge { a: u32, b: u32 },

    /// Two triangles traverse a shared edge in the same direction.
    #[error("triangles adjacent to edge ({a}, {b}) have inconsistent winding")]
    InconsistentOrientation { a: u32, b: u32 },

    /// A seam polyline has fewer than two vertices.
    #[error("seam {seam} has fewer than two vertices")]
    SeamTooShort { seam: usize },

    /// Two consecutive seam vertices are not joined by a mesh edge.
    #[error("seam {seam}: vertices {a} and {b} are not joined by a mesh edge")]
    SeamNotAnEdge { seam: usize, a: u32, b: u32 },

    /// A notch vertex is not on its seam.
    #[error("seam {seam}: notch vertex {vertex} is not on the seam")]
    NotchNotOnSeam { seam: usize, vertex: u32 },

    /// A panel produced by cutting is not a topological disc.
    #[error(
        "panel {panel} cannot be laid flat: {}",
        if *boundary_loops > 1 {
            format!("it still wraps around the body ({boundary_loops} separate edges); add a seam joining two of them")
        } else if *boundary_loops == 0 {
            "it is a closed surface with no edge; add an edge curve or seams".to_string()
        } else {
            format!("it is not a simple piece (Euler characteristic {euler_characteristic}); add seams to split it")
        }
    )]
    NotADisc {
        panel: usize,
        euler_characteristic: i64,
        boundary_loops: usize,
    },

    /// A vertex has more than one outgoing boundary edge after cutting.
    #[error("panel {panel}: vertex {vertex} is non-manifold (pinched boundary)")]
    NonManifoldVertex { panel: usize, vertex: u32 },

    /// The ease specification does not match the panel.
    #[error("ease: {0}")]
    InvalidEase(String),

    /// A piece is wider than the sheet in every allowed orientation.
    #[error(
        "piece {piece} ({width:.0} × {height:.0} mm) does not fit a {sheet_width:.0} mm sheet; \
         allow rotation, use a wider sheet, or add a seam to split it"
    )]
    PieceTooWide {
        piece: usize,
        width: f64,
        height: f64,
        sheet_width: f64,
    },

    /// A garment design cannot be built.
    #[error("garment: {0}")]
    Garment(String),

    /// Body measurements are invalid.
    #[error("body: {0}")]
    Body(String),

    /// Swatch calibration input is invalid.
    #[error("calibration: {0}")]
    Calibration(String),

    /// A sparse linear solve failed (usually a degenerate panel).
    #[error("linear solve failed: {0}")]
    Solve(String),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;
