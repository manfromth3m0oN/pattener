//! # latexpat-core
//!
//! Pattern drafting for latex (natural rubber sheet) garments.
//!
//! A garment is designed flat, at body size: pieces drawn in 2D (usually
//! starting from a template drafted from the wearer's measurements) and the
//! seams that join them. Making the pattern reduces each piece by its
//! *ease* `s` (e.g. 0.85), so it stretches by about `1/s` when worn, then
//! adds glue laps and hems, numbered notches, trim strips, sheet layouts,
//! an assembly guide and a printable PDF.
//!
//! Pipeline:
//! 1. [`body::Body`]: measurements (missing ones estimated) and the levels
//!    drafting works from;
//! 2. [`blocks::template`]: a [`design::Design`] drafted for that body,
//!    edited by the user; [`blocks::redraft`] moves it to new measurements;
//! 3. [`pattern::build_pattern`]: unfolded, reduced pieces with allowances
//!    and notches;
//! 4. [`job::run`]: SVGs, layouts, the assembly guide, the PDF and a report.
//!
//! This crate does no file I/O and spawns no threads, so it can be
//! compiled to WebAssembly.

pub mod allowance;
pub mod assembly;
pub mod blocks;
pub mod body;
pub mod calibrate;
pub mod design;
pub mod error;
pub mod geom;
pub mod job;
pub mod materials;
pub mod nest;
pub mod pattern;
pub mod pdf;
pub mod project;
pub mod svg;

pub use design::{Design, Piece, Seam};
pub use error::{Error, Result};
pub use glam;
