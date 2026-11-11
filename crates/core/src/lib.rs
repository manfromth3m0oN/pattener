//! # latexpat-core
//!
//! Pattern generation for latex (natural rubber sheet) garments.
//!
//! A garment is a set of panels bounded by seams on a 3D body mesh. Each
//! panel is flattened to 2D at a target *ease factor* `s` (e.g. 0.85): the
//! flat piece is `s` times the size of the body, so it stretches by about
//! `1/s` when pulled on. Latex absorbs curvature by stretching, so we do
//! not aim for a distortion-free layout; we aim for low, even strain and
//! report it.
//!
//! Pipeline:
//! 1. [`obj::parse_obj`] → [`Mesh`]
//! 2. [`SeamSet::resolve`] marks cut edges
//!
//! This crate does no file I/O and spawns no threads, so it can be
//! compiled to WebAssembly.

pub mod error;
pub mod mesh;
pub mod obj;
pub mod seams;

pub use error::{Error, Result};
pub use mesh::Mesh;
pub use seams::{Seam, SeamId, SeamSet};
