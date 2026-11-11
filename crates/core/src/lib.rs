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
//! 2. [`SeamSet::resolve`] + [`cut::cut_panels`] → [`Panel`]s (topological discs)
//!
//! This crate does no file I/O and spawns no threads, so it can be
//! compiled to WebAssembly.

pub mod cut;
pub mod error;
pub mod linalg;
pub mod lscm;
pub mod mesh;
pub mod obj;
pub mod panel;
pub mod seams;
pub mod testmesh;

pub use error::{Error, Result};
pub use mesh::Mesh;
pub use panel::{BoundaryEdge, Panel};
pub use seams::{Seam, SeamId, SeamSet};
