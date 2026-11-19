//! A saved design: everything needed to reproduce a pattern.
//!
//! The project file is JSON (with the `serde` feature): the body's
//! measurements, the flat design, the latex sheets and the pattern
//! settings. Missing fields take defaults, so older files keep loading.
//! Projects from the 3D editor (version 1) kept their garment as curves on
//! a body model; only their measurements carry over (see
//! [`Project::is_3d`]).
//!
//! ```json
//! { "version": 2, "name": "Top",
//!   "body": { "figure": "women", "measurements": { "height": 1680, "bust": 940 } },
//!   "design": { "pieces": [ ... ], "seams": [ ... ] },
//!   "materials": [ { "name": "black", "color": "#111111", "thickness_mm": 0.4, "roll_width_mm": 1000 } ],
//!   "settings": { "ease": 0.85, "length_relief": 0.5 } }
//! ```

use crate::body::BodySpec;
use crate::design::Design;
use crate::job::{EaseSpec, JobOptions};
use crate::materials::Sheet;
use crate::nest::NestOptions;
use crate::pattern::PatternOptions;
use crate::pdf::{Paper, PrintOptions};

/// Current project file version.
pub const PROJECT_VERSION: u32 = 2;

/// Pattern settings kept with a project.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Settings {
    /// Flat size / body size around the body.
    pub ease: f64,
    /// Share of the reduction applied along the body (0–1).
    pub length_relief: f64,
    pub seam_allowance_mm: f64,
    pub hem_allowance_mm: f64,
    pub zip_allowance_mm: f64,
    /// 0 for marked notches only.
    pub notch_spacing_mm: f64,
    pub seam_tolerance_mm: f64,
    /// Also lay every piece out on one sheet this wide.
    pub sheet_width_mm: Option<f64>,
    /// Paper for the printable PDF; `None` for no PDF.
    pub paper: Option<Paper>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ease: 0.85,
            length_relief: 1.0,
            seam_allowance_mm: 6.0,
            hem_allowance_mm: 0.0,
            zip_allowance_mm: 10.0,
            notch_spacing_mm: 50.0,
            seam_tolerance_mm: 1.0,
            sheet_width_mm: None,
            paper: Some(Paper::A4),
        }
    }
}

/// A design project.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Project {
    pub version: u32,
    pub name: String,
    /// Units the measurements are shown in ("cm", "mm" or "in").
    pub units: String,
    pub body: BodySpec,
    pub design: Design,
    /// The latex sheets available for this garment.
    pub materials: Vec<Sheet>,
    /// Sheet for pieces that do not name one.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub default_sheet: Option<String>,
    pub settings: Settings,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            version: PROJECT_VERSION,
            name: "Untitled".into(),
            units: "cm".into(),
            body: BodySpec::default(),
            design: Design::default(),
            materials: Vec::new(),
            default_sheet: None,
            settings: Settings::default(),
        }
    }
}

impl Project {
    /// Whether this project came from the 3D editor.
    pub fn is_3d(&self) -> bool {
        self.version < 2
    }

    /// The pattern job these settings describe.
    pub fn job_options(&self) -> JobOptions {
        let s = &self.settings;
        JobOptions {
            ease: EaseSpec::Uniform(s.ease),
            length_relief: s.length_relief,
            seam_tolerance: s.seam_tolerance_mm,
            pattern: PatternOptions {
                seam_allowance: s.seam_allowance_mm,
                hem_allowance: s.hem_allowance_mm,
                notch_spacing: (s.notch_spacing_mm > 0.0).then_some(s.notch_spacing_mm),
                zip_allowance: s.zip_allowance_mm,
            },
            sheet: s.sheet_width_mm.filter(|w| *w > 0.0).map(|w| NestOptions {
                sheet_width: w,
                gap: 5.0,
                allow_rotation: true,
            }),
            print: s.paper.map(|paper| PrintOptions {
                paper,
                ..Default::default()
            }),
            materials: self.materials.clone(),
            default_sheet: self.default_sheet.clone(),
        }
    }
}
