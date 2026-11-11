//! A saved design: everything needed to reproduce a garment.
//!
//! The project file is JSON (with the `serde` feature). It grows with each
//! design stage; for now it holds the body and the garment drawn on it. Unknown fields are ignored and
//! missing ones take defaults, so older files keep loading.
//!
//! ```json
//! { "version": 1, "name": "Catsuit",
//!   "body": { "measurements": { "height": 1680, "bust": 940, "waist": 740 },
//!             "shape": { "body_type": 0, "cup": 0.2 } } }
//! ```

use crate::garment::GarmentDesign;
use crate::human::{Measurements, ShapeOptions};

/// Current project file version.
pub const PROJECT_VERSION: u32 = 1;

/// The body a garment is designed on.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct BodySpec {
    /// Tape measurements (mm); any subset.
    pub measurements: Measurements,
    pub shape: ShapeOptions,
}

/// A design project.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub body: BodySpec,
    /// The garment drawn on the body.
    pub garment: GarmentDesign,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            version: PROJECT_VERSION,
            name: "Untitled".into(),
            body: BodySpec::default(),
            garment: GarmentDesign::default(),
        }
    }
}
