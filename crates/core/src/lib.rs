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
//! 3. [`lscm::lscm`] → initial conformal layout
//! 4. [`arap::arap`] → refinement towards reference triangles scaled by [`Ease`]
//! 5. [`seam_match`] → pair seam sides, equalise their flat lengths
//! 6. [`strain::measure`] → principal stretches and flipped triangles
//! 7. [`svg::panel_svg`] → 1:1 SVG, optionally with a strain heatmap
//!
//! [`flatten_garment`] runs steps 2–6.
//!
//! This crate does no file I/O and spawns no threads, so it can be
//! compiled to WebAssembly.

// Index loops mirror the formulas; `!(a > b)` deliberately rejects NaN.
#![allow(clippy::needless_range_loop, clippy::neg_cmp_op_on_partial_ord)]

pub mod allowance;
pub mod arap;
pub mod body;
pub mod calibrate;
pub mod cut;
pub mod ease;
pub mod error;
pub mod flatten;
pub mod garment;
pub mod human;
pub mod job;
pub mod linalg;
pub mod lscm;
pub mod mesh;
pub mod nest;
pub mod obj;
pub mod panel;
pub mod pattern;
pub mod project;
pub mod seam_match;
pub mod seams;
pub mod strain;
pub mod svg;
pub mod testmesh;

pub use ease::Ease;
pub use error::{Error, Result};
pub use flatten::{
    flatten_garment, flatten_panel, FlatPanel, FlattenOptions, Garment, SeamMatching, SeamReport,
};
pub use glam;
pub use mesh::Mesh;
pub use panel::{BoundaryEdge, Panel};
pub use seams::{Seam, SeamId, SeamSet};
pub use strain::{PrincipalStretch, StrainReport};
