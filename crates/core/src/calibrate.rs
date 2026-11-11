//! Swatch calibration: deriving the ease factor for each latex sheet from
//! simple stretch tests.
//!
//! Thicker latex pulls harder for the same stretch, so it needs less
//! reduction (an ease closer to 1) to feel the same when worn. This module
//! fits a material model to stretch tests on each sheet and picks, for each
//! sheet, the ease that gives the same wearing tension.
//!
//! ## Swatch test
//! Cut a strip of width `w`, draw two gauge marks `L₀` apart, hang a known
//! load `F` (or mass) from it and measure the new mark distance `L`. Several
//! loads per sheet improve the fit. The stretch is `λ = L / L₀`.
//!
//! ## Material model
//! Natural rubber is well described at garment-level stretch (up to ~2×) by
//! the incompressible **neo-Hookean** model with shear modulus `μ`. For
//! uniaxial tension the nominal stress (force per original cross-section
//! `w·t`) is
//! ```text
//! F / (w t) = μ (λ − λ⁻²)
//! ```
//! which is linear in `μ`, so the least-squares fit over samples `i` is
//! `μ = Σ xᵢ Fᵢ / Σ xᵢ²` with `xᵢ = wᵢ t (λᵢ − λᵢ⁻²)`.
//!
//! ## Wearing tension
//! A panel flattened at ease `s` is stretched by about `λ = 1/s` in every
//! direction when worn (that is what the uniform reference scaling in ARAP
//! does). For an equi-biaxially stretched neo-Hookean membrane of thickness
//! `t`, the tension per unit of original width is
//! ```text
//! T = μ t (λ − λ⁻⁵).
//! ```
//! The target `T` is either given directly or taken from a reference fit
//! (a sheet and the ease that fits well with it). For each sheet the ease is
//! `s = 1/λ` where `μ t (λ − λ⁻⁵) = T`; the left side increases
//! monotonically for `λ ≥ 1`, so `λ` is found by bisection.

use crate::error::{Error, Result};

/// Standard gravity, for converting hanging masses to loads.
pub const G: f64 = 9.806_65;

/// One stretch measurement on a strip.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sample {
    /// Strip width (mm).
    pub width_mm: f64,
    /// Unstretched distance between gauge marks (mm).
    pub gauge_mm: f64,
    /// Distance between gauge marks under load (mm).
    pub stretched_mm: f64,
    /// Load in newtons (give this or `mass_g`).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub load_n: Option<f64>,
    /// Hanging mass in grams (give this or `load_n`).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub mass_g: Option<f64>,
}

impl Sample {
    /// Load in newtons.
    pub fn load(&self) -> Result<f64> {
        match (self.load_n, self.mass_g) {
            (Some(f), None) => Ok(f),
            (None, Some(m)) => Ok(m * 1e-3 * G),
            _ => Err(Error::Calibration(
                "each sample needs exactly one of load_n or mass_g".into(),
            )),
        }
    }

    /// Stretch `λ = L / L₀`.
    pub fn stretch(&self) -> f64 {
        self.stretched_mm / self.gauge_mm
    }
}

/// Stretch tests for one latex sheet.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Swatch {
    pub name: String,
    pub thickness_mm: f64,
    pub samples: Vec<Sample>,
}

/// What "a good fit" means.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Target {
    /// Wearing tension in newtons per centimetre of width.
    TensionNPerCm(f64),
    /// "This sheet at this ease fits well."
    Reference { sheet: String, ease: f64 },
}

/// Calibration input.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Calibration {
    pub sheets: Vec<Swatch>,
    pub target: Target,
}

/// Fitted material and derived ease for one sheet.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SheetFit {
    pub name: String,
    pub thickness_mm: f64,
    /// Neo-Hookean shear modulus μ (MPa = N/mm²).
    pub shear_modulus_mpa: f64,
    /// Root-mean-square load residual of the fit (N).
    pub rms_error_n: f64,
    /// RMS residual relative to the mean load.
    pub relative_error: f64,
    /// Ease giving the target wearing tension.
    pub ease: f64,
}

/// Result of [`calibrate`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CalibrationResult {
    pub sheets: Vec<SheetFit>,
    /// μ fitted to every sample of every sheet together (for untested
    /// thicknesses of the same compound).
    pub pooled_shear_modulus_mpa: f64,
    /// Target wearing tension (N/cm).
    pub tension_n_per_cm: f64,
}

/// Fit μ to a sheet's samples. Returns `(μ, rms error, relative error)`.
pub fn fit_shear_modulus(thickness: f64, samples: &[Sample]) -> Result<(f64, f64, f64)> {
    fit_pooled(&samples.iter().map(|s| (thickness, *s)).collect::<Vec<_>>())
}

fn fit_pooled(samples: &[(f64, Sample)]) -> Result<(f64, f64, f64)> {
    if samples.is_empty() {
        return Err(Error::Calibration(
            "a sheet needs at least one sample".into(),
        ));
    }
    let mut pts = Vec::with_capacity(samples.len());
    for (t, s) in samples {
        let ok = |v: f64| v.is_finite() && v > 0.0;
        if !(ok(*t) && ok(s.width_mm) && ok(s.gauge_mm)) {
            return Err(Error::Calibration(
                "thickness, width and gauge must be positive".into(),
            ));
        }
        let f = s.load()?;
        let l = s.stretch();
        if !(l > 1.0 && f > 0.0) {
            return Err(Error::Calibration(format!(
                "sample with {:.1} mm → {:.1} mm under {f:.3} N: the strip must lengthen under a positive load",
                s.gauge_mm, s.stretched_mm
            )));
        }
        pts.push((s.width_mm * t * (l - l.powi(-2)), f));
    }
    let mu =
        pts.iter().map(|(x, y)| x * y).sum::<f64>() / pts.iter().map(|(x, _)| x * x).sum::<f64>();
    let n = pts.len() as f64;
    let rms = (pts.iter().map(|(x, y)| (y - mu * x).powi(2)).sum::<f64>() / n).sqrt();
    let mean = pts.iter().map(|(_, y)| y).sum::<f64>() / n;
    Ok((mu, rms, rms / mean))
}

/// Equi-biaxial wearing tension per unit original width (N/mm):
/// `T = μ t (λ − λ⁻⁵)`.
pub fn biaxial_tension(mu: f64, thickness: f64, stretch: f64) -> f64 {
    mu * thickness * (stretch - stretch.powi(-5))
}

/// The stretch `λ ≥ 1` at which [`biaxial_tension`] equals `tension`
/// (bisection; the tension increases monotonically with `λ`).
pub fn stretch_for_tension(mu: f64, thickness: f64, tension: f64) -> Result<f64> {
    let f = |l: f64| biaxial_tension(mu, thickness, l) - tension;
    let (mut lo, mut hi) = (1.0, 2.0);
    while f(hi) < 0.0 {
        hi *= 2.0;
        if hi > 64.0 {
            return Err(Error::Calibration(format!(
                "target tension {:.3} N/cm is out of reach for a {thickness} mm sheet",
                tension * 10.0
            )));
        }
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if f(mid) < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(0.5 * (lo + hi))
}

/// Fit every sheet and derive its ease.
pub fn calibrate(c: &Calibration) -> Result<CalibrationResult> {
    if c.sheets.is_empty() {
        return Err(Error::Calibration("no sheets given".into()));
    }
    let mut fits = Vec::with_capacity(c.sheets.len());
    for sw in &c.sheets {
        let (mu, rms, rel) = fit_shear_modulus(sw.thickness_mm, &sw.samples)
            .map_err(|e| Error::Calibration(format!("sheet '{}': {e}", sw.name)))?;
        fits.push((sw, mu, rms, rel));
    }
    let tension = match &c.target {
        Target::TensionNPerCm(t) if *t > 0.0 => t / 10.0,
        Target::TensionNPerCm(t) => {
            return Err(Error::Calibration(format!(
                "target tension must be positive, got {t}"
            )))
        }
        Target::Reference { sheet, ease } => {
            if !(*ease > 0.0 && *ease <= 1.0) {
                return Err(Error::Calibration(format!(
                    "reference ease must be in (0, 1], got {ease}"
                )));
            }
            let (sw, mu, _, _) =
                fits.iter()
                    .find(|(sw, ..)| &sw.name == sheet)
                    .ok_or_else(|| {
                        Error::Calibration(format!("reference sheet '{sheet}' is not in the list"))
                    })?;
            biaxial_tension(*mu, sw.thickness_mm, 1.0 / ease)
        }
    };
    let pooled: Vec<(f64, Sample)> = c
        .sheets
        .iter()
        .flat_map(|sw| sw.samples.iter().map(move |s| (sw.thickness_mm, *s)))
        .collect();
    let (pooled_mu, _, _) = fit_pooled(&pooled)?;

    let sheets = fits
        .into_iter()
        .map(|(sw, mu, rms, rel)| {
            Ok(SheetFit {
                name: sw.name.clone(),
                thickness_mm: sw.thickness_mm,
                shear_modulus_mpa: mu,
                rms_error_n: rms,
                relative_error: rel,
                ease: 1.0 / stretch_for_tension(mu, sw.thickness_mm, tension)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(CalibrationResult {
        sheets,
        pooled_shear_modulus_mpa: pooled_mu,
        tension_n_per_cm: tension * 10.0,
    })
}

impl CalibrationResult {
    /// Fitted sheet by name.
    pub fn sheet(&self, name: &str) -> Option<&SheetFit> {
        self.sheets.iter().find(|s| s.name == name)
    }

    /// Resolve a sheet name, or a thickness such as `"0.5mm"` (pooled
    /// fit), to `(ease, stiffness μt, label)`.
    pub fn resolve(&self, spec: &str) -> Result<(f64, f64, String)> {
        if let Some(s) = self.sheet(spec) {
            return Ok((s.ease, s.shear_modulus_mpa * s.thickness_mm, s.name.clone()));
        }
        let num = spec.trim().trim_end_matches("mm").trim();
        if let Ok(t) = num.parse::<f64>() {
            return Ok((
                self.ease_for_thickness(t)?,
                self.pooled_shear_modulus_mpa * t,
                format!("{t} mm (pooled fit)"),
            ));
        }
        let names: Vec<&str> = self.sheets.iter().map(|s| s.name.as_str()).collect();
        Err(Error::Calibration(format!(
            "unknown sheet '{spec}'; use one of {names:?} or a thickness such as 0.5mm"
        )))
    }

    /// Ease for an untested thickness of the same compound, using the
    /// pooled shear modulus.
    pub fn ease_for_thickness(&self, thickness_mm: f64) -> Result<f64> {
        if !(thickness_mm > 0.0 && thickness_mm.is_finite()) {
            return Err(Error::Calibration("thickness must be positive".into()));
        }
        Ok(1.0
            / stretch_for_tension(
                self.pooled_shear_modulus_mpa,
                thickness_mm,
                self.tension_n_per_cm / 10.0,
            )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    /// Exact neo-Hookean samples for modulus `mu`: hang `masses` (g) from a
    /// 25 mm strip with 100 mm gauge marks.
    fn synthetic(mu: f64, t: f64, masses: &[f64]) -> Vec<Sample> {
        masses
            .iter()
            .map(|&m| {
                let f = m * 1e-3 * G;
                // Solve μ w t (λ − λ⁻²) = F by bisection.
                let (mut lo, mut hi) = (1.0, 10.0);
                for _ in 0..200 {
                    let mid: f64 = 0.5 * (lo + hi);
                    if mu * 25.0 * t * (mid - mid.powi(-2)) < f {
                        lo = mid
                    } else {
                        hi = mid
                    }
                }
                Sample {
                    width_mm: 25.0,
                    gauge_mm: 100.0,
                    stretched_mm: 100.0 * lo,
                    mass_g: Some(m),
                    ..Default::default()
                }
            })
            .collect()
    }

    fn sheets() -> Vec<Swatch> {
        [("thin", 0.25), ("medium", 0.4), ("thick", 0.6)]
            .iter()
            .map(|&(n, t)| Swatch {
                name: n.into(),
                thickness_mm: t,
                samples: synthetic(0.45, t, &[100.0, 200.0, 300.0]),
            })
            .collect()
    }

    #[test]
    fn recovers_modulus_from_exact_data() {
        let (mu, rms, _) =
            fit_shear_modulus(0.4, &synthetic(0.45, 0.4, &[50.0, 150.0, 400.0])).unwrap();
        assert_relative_eq!(mu, 0.45, epsilon = 1e-9);
        assert!(rms < 1e-9);
    }

    #[test]
    fn reference_sheet_keeps_its_ease_and_others_match_tension() {
        let c = Calibration {
            sheets: sheets(),
            target: Target::Reference {
                sheet: "medium".into(),
                ease: 0.85,
            },
        };
        let r = calibrate(&c).unwrap();
        assert_relative_eq!(r.sheet("medium").unwrap().ease, 0.85, epsilon = 1e-9);
        // Thicker sheets need less reduction.
        let e: Vec<f64> = r.sheets.iter().map(|s| s.ease).collect();
        assert!(e[0] < e[1] && e[1] < e[2], "{e:?}");
        // Every sheet produces the same wearing tension at its ease.
        for s in &r.sheets {
            let t = biaxial_tension(s.shear_modulus_mpa, s.thickness_mm, 1.0 / s.ease) * 10.0;
            assert_relative_eq!(t, r.tension_n_per_cm, epsilon = 1e-9);
        }
        // Same compound: pooled modulus and untested thickness interpolate.
        assert_relative_eq!(r.pooled_shear_modulus_mpa, 0.45, epsilon = 1e-9);
        assert_relative_eq!(r.ease_for_thickness(0.4).unwrap(), 0.85, epsilon = 1e-9);
        let mid = r.ease_for_thickness(0.5).unwrap();
        assert!(e[1] < mid && mid < e[2]);
    }

    #[test]
    fn explicit_tension_target() {
        let c = Calibration {
            sheets: sheets(),
            target: Target::TensionNPerCm(1.0),
        };
        let r = calibrate(&c).unwrap();
        for s in &r.sheets {
            assert_relative_eq!(
                biaxial_tension(0.45, s.thickness_mm, 1.0 / s.ease),
                0.1,
                epsilon = 1e-9
            );
        }
    }

    #[test]
    fn rejects_bad_input() {
        let mut s = sheets();
        s[0].samples[0].load_n = Some(1.0); // both load_n and mass_g
        let c = Calibration {
            sheets: s,
            target: Target::TensionNPerCm(1.0),
        };
        assert!(
            matches!(calibrate(&c), Err(Error::Calibration(m)) if m.contains("thin") && m.contains("exactly one"))
        );

        let mut s = sheets();
        s[1].samples[0].stretched_mm = 90.0;
        let c = Calibration {
            sheets: s,
            target: Target::TensionNPerCm(1.0),
        };
        assert!(matches!(calibrate(&c), Err(Error::Calibration(m)) if m.contains("lengthen")));

        let c = Calibration {
            sheets: sheets(),
            target: Target::Reference {
                sheet: "nope".into(),
                ease: 0.85,
            },
        };
        assert!(matches!(calibrate(&c), Err(Error::Calibration(m)) if m.contains("nope")));
    }
}
