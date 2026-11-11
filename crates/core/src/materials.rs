//! Latex sheets: the catalogue a garment is made from.
//!
//! A [`Sheet`] is one roll of latex: its colour, thickness and roll width,
//! and optionally its own ease, rubber stiffness and price. A
//! [`MaterialPlan`] says which sheet each panel is cut from. Each panel then
//! gets
//! - its sheet's ease (the sheet's own value, else its swatch calibration,
//!   else the job's ease);
//! - its sheet's membrane stiffness `k = μ t` for seam matching between
//!   different sheets and for the pressure map;
//! - a place on its sheet's layout, packed to the sheet's roll width, from
//!   which the length to buy (and cost) follows.

use crate::error::{Error, Result};

/// Default neo-Hookean shear modulus of latex (MPa).
pub const DEFAULT_SHEAR_MODULUS_MPA: f64 = 0.45;

/// One latex sheet (roll).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sheet {
    pub name: String,
    /// Display colour, `#rrggbb`.
    pub color: String,
    pub thickness_mm: f64,
    /// Usable width of the roll (mm).
    pub roll_width_mm: f64,
    /// Ease for pieces cut from this sheet; `None` uses the calibration or
    /// the job's ease.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub ease: Option<f64>,
    /// Shear modulus μ (MPa); `None` uses the calibration or 0.45.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub shear_modulus_mpa: Option<f64>,
    /// Price per metre of roll length.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub price_per_m: Option<f64>,
}

impl Sheet {
    /// Check the values make sense.
    pub fn validate(&self) -> Result<()> {
        let bad = |what: &str| Err(Error::Material(format!("sheet '{}': {what}", self.name)));
        if self.name.trim().is_empty() {
            return Err(Error::Material("every sheet needs a name".into()));
        }
        if parse_color(&self.color).is_none() {
            return bad(&format!("colour '{}' is not #rrggbb", self.color));
        }
        if !(self.thickness_mm > 0.0 && self.thickness_mm < 5.0) {
            return bad("thickness must be between 0 and 5 mm");
        }
        if !(self.roll_width_mm > 0.0 && self.roll_width_mm.is_finite()) {
            return bad("roll width must be positive");
        }
        if let Some(e) = self.ease {
            if !(e > 0.0 && e <= 1.5) {
                return bad("ease must be between 0 and 1.5");
            }
        }
        if let Some(m) = self.shear_modulus_mpa {
            if !(m > 0.0 && m.is_finite()) {
                return bad("shear modulus must be positive");
            }
        }
        if let Some(p) = self.price_per_m {
            if !(p >= 0.0 && p.is_finite()) {
                return bad("price must be zero or positive");
            }
        }
        Ok(())
    }
}

/// `#rrggbb` to `[r, g, b]` in 0–1.
pub fn parse_color(s: &str) -> Option<[f64; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = |i: usize| {
        u8::from_str_radix(&h[i..i + 2], 16)
            .map(|x| x as f64 / 255.0)
            .ok()
    };
    Some([v(0)?, v(2)?, v(4)?])
}

/// Which sheet each panel is cut from.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MaterialPlan {
    pub sheets: Vec<Sheet>,
    /// Index into `sheets` for each panel (in cut order).
    pub panel_sheet: Vec<usize>,
}

impl MaterialPlan {
    /// Check the sheets and that there is one valid entry per panel.
    pub fn validate(&self, n_panels: usize) -> Result<()> {
        for s in &self.sheets {
            s.validate()?;
        }
        if self.panel_sheet.len() != n_panels {
            return Err(Error::Material(format!(
                "{} sheet assignment(s) for {n_panels} panel(s)",
                self.panel_sheet.len()
            )));
        }
        if let Some(&i) = self.panel_sheet.iter().find(|&&i| i >= self.sheets.len()) {
            return Err(Error::Material(format!("sheet {i} does not exist")));
        }
        Ok(())
    }
}

/// How much of one sheet a garment uses.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MaterialUse {
    pub sheet: String,
    pub color: String,
    pub thickness_mm: f64,
    pub roll_width_mm: f64,
    /// Panels cut from this sheet.
    pub pieces: Vec<usize>,
    /// Length of roll needed (mm).
    pub length_mm: f64,
    /// Area of the cut pieces (m²).
    pub pieces_area_m2: f64,
    /// Pieces' area / (roll width × length).
    pub utilisation: f64,
    /// `price_per_m × length`, if a price is given.
    pub cost: Option<f64>,
    /// Layout SVG file name.
    pub file: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sheet(name: &str, color: &str, t: f64) -> Sheet {
        Sheet {
            name: name.into(),
            color: color.into(),
            thickness_mm: t,
            roll_width_mm: 1000.0,
            ease: None,
            shear_modulus_mpa: None,
            price_per_m: None,
        }
    }

    #[test]
    fn colours_and_validation() {
        assert_eq!(parse_color("#ff8000"), Some([1.0, 128.0 / 255.0, 0.0]));
        assert_eq!(parse_color("ff8000"), None);
        assert_eq!(parse_color("#ff80"), None);
        assert!(sheet("black", "#111111", 0.4).validate().is_ok());
        assert!(sheet("", "#111111", 0.4).validate().is_err());
        assert!(sheet("x", "red", 0.4).validate().is_err());
        assert!(sheet("x", "#111111", 0.0).validate().is_err());
        let plan = MaterialPlan {
            sheets: vec![sheet("a", "#000000", 0.4)],
            panel_sheet: vec![0, 1],
        };
        assert!(plan.validate(2).is_err());
        assert!(plan.validate(3).is_err());
    }
}
