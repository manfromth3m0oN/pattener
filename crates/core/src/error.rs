//! Error type shared by every stage of the pipeline.

use thiserror::Error;

/// Everything that can go wrong between measurements and a printed pattern.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum Error {
    /// The ease specification is invalid.
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

    /// A latex sheet or material plan is invalid.
    #[error("materials: {0}")]
    Material(String),

    /// A design cannot be made into a pattern.
    #[error("design: {0}")]
    Design(String),

    /// Body measurements are invalid.
    #[error("body: {0}")]
    Body(String),

    /// Swatch calibration input is invalid.
    #[error("calibration: {0}")]
    Calibration(String),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;
