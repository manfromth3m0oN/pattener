//! Body measurements and the levels and widths drafting works from.
//!
//! ## Estimating what was not measured
//! Any subset of the [`Measure`]s may be given. The rest come from a
//! typical figure of the chosen [`Figure`]:
//! - **Lengths** (heights, back length, shoulder, arm) scale with height.
//! - **Girths** scale by the geometric mean of the ratios between the
//!   girths that were given and the typical figure's.
//!
//! [`Body::estimated`] lists which values were estimated, so they can be
//! shown as such.
//!
//! ## Levels
//! Drafting places each piece at body height: `y` is the height above the
//! floor (mm). The levels of the bust, underarm and so on, which a tape
//! measure does not give, follow common anthropometric proportions of the
//! height, nape and waist (see [`Levels`]).
//!
//! ## Front and back
//! The girth at a level is shared between the front and back pieces: the
//! side seam sits at a quarter of the girth from the centre line, moved
//! towards the back by [`Body::side_offset`] for a women's figure (the bust
//! needs more room in front). The offset is the same at every level, so the
//! front and back side seams have the same shape and length.

use crate::error::{Error, Result};
use std::collections::BTreeMap;

/// A tape measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Measure {
    /// Floor to top of head.
    Height,
    /// Floor to crotch (inside leg).
    Inseam,
    /// Floor to the natural waist.
    WaistHeight,
    /// Nape of the neck (the bone at its base) down the spine to the waist.
    BackLength,
    /// Across the back, from one shoulder point to the other.
    Shoulder,
    /// Shoulder point over the elbow to the wrist.
    ArmLength,
    Neck,
    Bust,
    Underbust,
    Waist,
    Hips,
    /// Around the thigh, just below the crotch.
    Thigh,
    Knee,
    Calf,
    Ankle,
    UpperArm,
    Wrist,
}

impl Measure {
    pub const ALL: [Measure; 17] = [
        Self::Height,
        Self::Inseam,
        Self::WaistHeight,
        Self::BackLength,
        Self::Shoulder,
        Self::ArmLength,
        Self::Neck,
        Self::Bust,
        Self::Underbust,
        Self::Waist,
        Self::Hips,
        Self::Thigh,
        Self::Knee,
        Self::Calf,
        Self::Ankle,
        Self::UpperArm,
        Self::Wrist,
    ];

    /// Lengths scale with height; girths with the other girths.
    pub fn is_girth(self) -> bool {
        !matches!(
            self,
            Self::Height
                | Self::Inseam
                | Self::WaistHeight
                | Self::BackLength
                | Self::Shoulder
                | Self::ArmLength
        )
    }

    /// The `snake_case` name used in files.
    pub fn name(self) -> &'static str {
        match self {
            Self::Height => "height",
            Self::Inseam => "inseam",
            Self::WaistHeight => "waist_height",
            Self::BackLength => "back_length",
            Self::Shoulder => "shoulder",
            Self::ArmLength => "arm_length",
            Self::Neck => "neck",
            Self::Bust => "bust",
            Self::Underbust => "underbust",
            Self::Waist => "waist",
            Self::Hips => "hips",
            Self::Thigh => "thigh",
            Self::Knee => "knee",
            Self::Calf => "calf",
            Self::Ankle => "ankle",
            Self::UpperArm => "upper_arm",
            Self::Wrist => "wrist",
        }
    }
}

/// Measurements in millimetres; any subset.
pub type Measurements = BTreeMap<Measure, f64>;

/// The typical figure missing measurements are taken from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Figure {
    #[default]
    Women,
    Men,
}

impl Figure {
    /// Typical measurements (mm).
    pub fn typical(self) -> Measurements {
        use Measure::*;
        let v: [f64; 17] = match self {
            //            H     ins   wh    back  shou  arm  neck bust  ub   waist hips thigh knee calf ankle ua  wrist
            Figure::Women => [
                1680.0, 770.0, 1040.0, 400.0, 390.0, 590.0, 350.0, 940.0, 790.0, 740.0, 980.0,
                570.0, 370.0, 360.0, 220.0, 280.0, 155.0,
            ],
            Figure::Men => [
                1780.0, 820.0, 1080.0, 450.0, 460.0, 640.0, 400.0, 1000.0, 900.0, 850.0, 980.0,
                560.0, 390.0, 380.0, 240.0, 310.0, 175.0,
            ],
        };
        let kinds = [
            Height,
            Inseam,
            WaistHeight,
            BackLength,
            Shoulder,
            ArmLength,
            Neck,
            Bust,
            Underbust,
            Waist,
            Hips,
            Thigh,
            Knee,
            Calf,
            Ankle,
            UpperArm,
            Wrist,
        ];
        kinds.into_iter().zip(v).collect()
    }
}

/// The body a garment is drafted for: a figure and its measurements.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct BodySpec {
    pub figure: Figure,
    /// Tape measurements (mm); any subset.
    pub measurements: Measurements,
}

/// Heights above the floor (mm) of the levels drafting uses.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Levels {
    /// Base of the neck at the back (nape).
    pub nape: f64,
    /// Base of the neck at the side, where the shoulder seam starts.
    pub neck_side: f64,
    /// Shoulder point (end of the shoulder seam).
    pub shoulder: f64,
    /// Armpit: bottom of the armhole.
    pub underarm: f64,
    pub bust: f64,
    pub underbust: f64,
    pub waist: f64,
    pub hip: f64,
    pub crotch: f64,
    pub knee: f64,
    pub calf: f64,
    pub ankle: f64,
}

/// A body with every measurement known.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    pub figure: Figure,
    values: Measurements,
    /// Measurements that were not given and have been estimated.
    pub estimated: Vec<Measure>,
    pub levels: Levels,
}

impl Body {
    /// Fill in missing measurements and work out the levels.
    pub fn new(spec: &BodySpec) -> Result<Self> {
        for (&k, &v) in &spec.measurements {
            if !(v > 0.0 && v.is_finite()) {
                return Err(Error::Body(format!(
                    "{} must be a positive number of millimetres, got {v}",
                    k.name()
                )));
            }
        }
        let typical = spec.figure.typical();
        let given = &spec.measurements;
        let h_scale = given
            .get(&Measure::Height)
            .map_or(1.0, |h| h / typical[&Measure::Height]);
        // Geometric mean of the given girths' ratios to the typical ones.
        let ratios: Vec<f64> = given
            .iter()
            .filter(|(k, _)| k.is_girth())
            .map(|(k, v)| (v / typical[k]).ln())
            .collect();
        let g_scale = if ratios.is_empty() {
            1.0
        } else {
            (ratios.iter().sum::<f64>() / ratios.len() as f64).exp()
        };
        let mut values = Measurements::new();
        let mut estimated = Vec::new();
        for k in Measure::ALL {
            let v = match given.get(&k) {
                Some(&v) => v,
                None => {
                    estimated.push(k);
                    typical[&k] * if k.is_girth() { g_scale } else { h_scale }
                }
            };
            values.insert(k, v);
        }
        let mut body = Self {
            figure: spec.figure,
            values,
            estimated,
            levels: Levels {
                nape: 0.0,
                neck_side: 0.0,
                shoulder: 0.0,
                underarm: 0.0,
                bust: 0.0,
                underbust: 0.0,
                waist: 0.0,
                hip: 0.0,
                crotch: 0.0,
                knee: 0.0,
                calf: 0.0,
                ankle: 0.0,
            },
        };
        body.check()?;
        body.levels = body.work_out_levels()?;
        Ok(body)
    }

    /// A measurement (mm).
    pub fn get(&self, k: Measure) -> f64 {
        self.values[&k]
    }

    /// Every measurement, given or estimated.
    pub fn values(&self) -> &Measurements {
        &self.values
    }

    /// Measurements that cannot belong to one body.
    fn check(&self) -> Result<()> {
        use Measure::*;
        let v = |k| self.get(k);
        let order = |lo: Measure, hi: Measure| -> Result<()> {
            if v(lo) >= v(hi) {
                return Err(Error::Body(format!(
                    "{} ({:.0} mm) must be less than {} ({:.0} mm)",
                    lo.name(),
                    v(lo),
                    hi.name(),
                    v(hi)
                )));
            }
            Ok(())
        };
        order(Inseam, WaistHeight)?;
        order(WaistHeight, Height)?;
        order(Wrist, UpperArm)?;
        order(Ankle, Thigh)?;
        if v(WaistHeight) + v(BackLength) >= v(Height) - 0.08 * v(Height) {
            return Err(Error::Body(format!(
                "waist height ({:.0} mm) and back length ({:.0} mm) put the nape above the head \
                 (height {:.0} mm)",
                v(WaistHeight),
                v(BackLength),
                v(Height)
            )));
        }
        if v(WaistHeight) - v(Inseam) < 0.06 * v(Height) {
            return Err(Error::Body(format!(
                "waist height ({:.0} mm) is too close to the inseam ({:.0} mm): \
                 there is no room for the hips",
                v(WaistHeight),
                v(Inseam)
            )));
        }
        if v(Shoulder) <= 2.0 * self.neck_half_width() + 40.0 {
            return Err(Error::Body(format!(
                "shoulder width ({:.0} mm) is too narrow for the neck ({:.0} mm)",
                v(Shoulder),
                v(Neck)
            )));
        }
        Ok(())
    }

    fn work_out_levels(&self) -> Result<Levels> {
        use Measure::*;
        let h = self.get(Height);
        let waist = self.get(WaistHeight);
        let nape = waist + self.get(BackLength);
        let neck_side = nape + 20.0;
        let shoulder = neck_side - 0.36 * (self.get(Shoulder) / 2.0 - self.neck_half_width());
        let underarm = nape - (0.105 * h + 0.03 * self.get(Bust));
        let bust = underarm - 0.015 * h;
        let underbust = bust - 0.06 * h;
        let crotch = self.get(Inseam);
        let hip = (waist - 0.12 * h).max(crotch + 0.3 * (waist - crotch));
        let l = Levels {
            nape,
            neck_side,
            shoulder,
            underarm,
            bust,
            underbust: underbust.max(waist + 0.4 * (bust - waist)),
            waist,
            hip,
            crotch,
            knee: 0.285 * h,
            calf: 0.2 * h,
            ankle: 0.055 * h,
        };
        let chain = [
            ("shoulder", l.shoulder),
            ("underarm", l.underarm),
            ("bust", l.bust),
            ("underbust", l.underbust),
            ("waist", l.waist),
            ("hip", l.hip),
            ("crotch", l.crotch),
            ("knee", l.knee),
            ("calf", l.calf),
            ("ankle", l.ankle),
        ];
        for w in chain.windows(2) {
            if w[1].1 >= w[0].1 - 10.0 {
                return Err(Error::Body(format!(
                    "these measurements put the {} level ({:.0} mm) at or above the {} level \
                     ({:.0} mm); check the lengths",
                    w[1].0, w[1].1, w[0].0, w[0].1
                )));
            }
        }
        Ok(l)
    }

    /// Half the width of the neckline at the base of the neck.
    pub fn neck_half_width(&self) -> f64 {
        self.get(Measure::Neck) / 6.0 + 5.0
    }

    /// How far the side seam sits towards the back of a quarter girth (mm).
    pub fn side_offset(&self) -> f64 {
        match self.figure {
            Figure::Women => 0.01 * self.get(Measure::Bust),
            Figure::Men => 0.0,
        }
    }

    /// Extra length at the centre front of a top for the bust (mm): latex
    /// has no darts, so the front is lengthened at the centre instead.
    pub fn bust_extra(&self) -> f64 {
        (0.4 * (self.get(Measure::Bust) - self.get(Measure::Underbust) - 80.0)).clamp(0.0, 60.0)
    }

    /// Quarter girth of the torso at height `y`: from the measured girths at
    /// their levels, linear in between (bust above the bust level).
    pub fn torso_quarter(&self, y: f64) -> f64 {
        use Measure::*;
        let l = &self.levels;
        let pts = [
            (l.hip, self.get(Hips)),
            (l.waist, self.get(Waist)),
            (l.underbust, self.get(Underbust)),
            (l.bust, self.get(Bust)),
        ];
        interpolate(&pts, y) / 4.0
    }

    /// Guide lines for the editor: the name and height of each level.
    pub fn guides(&self) -> Vec<(&'static str, f64)> {
        let l = &self.levels;
        vec![
            ("neck", l.neck_side),
            ("nape", l.nape),
            ("shoulder", l.shoulder),
            ("underarm", l.underarm),
            ("bust", l.bust),
            ("underbust", l.underbust),
            ("waist", l.waist),
            ("hip", l.hip),
            ("crotch", l.crotch),
            ("knee", l.knee),
            ("calf", l.calf),
            ("ankle", l.ankle),
        ]
    }
}

/// Piecewise-linear interpolation through `(x, value)` points in any order,
/// constant beyond the ends.
pub fn interpolate(pts: &[(f64, f64)], x: f64) -> f64 {
    let mut p = pts.to_vec();
    p.sort_by(|a, b| a.0.total_cmp(&b.0));
    if x <= p[0].0 {
        return p[0].1;
    }
    for w in p.windows(2) {
        if x <= w[1].0 {
            let t = (x - w[0].0) / (w[1].0 - w[0].0);
            return w[0].1 + t * (w[1].1 - w[0].1);
        }
    }
    p[p.len() - 1].1
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn spec(figure: Figure, m: &[(Measure, f64)]) -> BodySpec {
        BodySpec {
            figure,
            measurements: m.iter().copied().collect(),
        }
    }

    #[test]
    fn typical_figures_are_valid_bodies() {
        for f in [Figure::Women, Figure::Men] {
            let b = Body::new(&spec(f, &[])).unwrap();
            assert_eq!(b.estimated.len(), Measure::ALL.len());
            assert_eq!(b.values(), &f.typical());
        }
    }

    #[test]
    fn missing_measurements_scale_with_what_is_given() {
        use Measure::*;
        let b = Body::new(&spec(
            Figure::Women,
            &[(Height, 1848.0), (Bust, 1034.0), (Waist, 814.0)],
        ))
        .unwrap();
        // Lengths ×1.1 with height; girths ×1.1 with the two girths given.
        assert_relative_eq!(b.get(Inseam), 847.0, epsilon = 1e-9);
        assert_relative_eq!(b.get(Hips), 1078.0, epsilon = 1e-9);
        assert_relative_eq!(b.get(Bust), 1034.0);
        assert!(!b.estimated.contains(&Bust));
        assert!(b.estimated.contains(&Hips));
    }

    #[test]
    fn impossible_measurements_are_reported() {
        use Measure::*;
        let err = Body::new(&spec(Figure::Women, &[(Inseam, 1100.0)])).unwrap_err();
        assert!(err.to_string().contains("inseam"), "{err}");
        assert!(Body::new(&spec(Figure::Men, &[(Waist, -3.0)])).is_err());
        assert!(Body::new(&spec(Figure::Women, &[(BackLength, 900.0)])).is_err());
    }

    #[test]
    fn quarter_girths_follow_the_levels() {
        let b = Body::new(&spec(Figure::Women, &[])).unwrap();
        let l = b.levels;
        assert_relative_eq!(b.torso_quarter(l.waist), 185.0);
        assert_relative_eq!(b.torso_quarter(l.hip), 245.0);
        assert_relative_eq!(b.torso_quarter(l.underarm), 235.0);
        assert!(l.nape > l.underarm && l.underarm > l.bust && l.bust > l.waist);
        assert!(l.hip > l.crotch && l.crotch > l.knee);
    }
}
