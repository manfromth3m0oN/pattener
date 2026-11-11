//! Full body from measurements, using a morphable body model.
//!
//! - [`model`]: the base mesh, joints and morph targets (MakeHuman, CC0).
//! - [`mod@slice`]: tape-measure girths from plane cuts.
//! - [`measure`]: where and how each body measurement is taken.
//! - [`mod@fit`]: solving the shape parameters so the body matches a set of
//!   measurements.

pub mod along;
pub mod fit;
pub mod measure;
pub mod model;
pub mod slice;

pub use along::along_body;
pub use fit::{fit, HumanBody, Measured, Measurements, ShapeOptions};
pub use measure::{MeasureKind, Measurer};
pub use model::{BodyModel, Shape, Target};

/// The bundled model, read from the repository (tests do not depend on
/// the `bundled-body` feature).
#[cfg(test)]
pub(crate) fn test_model() -> BodyModel {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/body/makehuman.lpbody"
    );
    BodyModel::from_bytes(&std::fs::read(path).expect("body model file")).expect("valid body model")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mesh;
    use approx::assert_relative_eq;
    use MeasureKind::*;

    fn measurements(v: &[(MeasureKind, f64)]) -> Measurements {
        v.iter().copied().collect()
    }

    #[test]
    fn base_mesh_is_closed_outward_and_symmetric() {
        let m = test_model();
        assert_eq!(m.n_body, 13380);
        let shape = m.morph(&[]);
        let mesh = Mesh::new(shape.body(&m).to_vec(), m.triangles.clone()).unwrap();
        assert_eq!(mesh.edges().iter().filter(|e| e.is_boundary()).count(), 0);
        let volume: f64 = mesh
            .triangles()
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.positions()[i as usize]);
                a.dot(b.cross(c)) / 6.0
            })
            .sum();
        // Outward-facing triangles give a positive volume, here about 55 litres.
        assert!((40e6..80e6).contains(&volume), "{volume}");
        let (l, r) = (
            shape.joint(&m, m.joint("l-shoulder").unwrap()),
            shape.joint(&m, m.joint("r-shoulder").unwrap()),
        );
        assert!(l.x > 0.0);
        assert_relative_eq!(l.x, -r.x, epsilon = 1e-3);
        assert_relative_eq!(l.y, r.y, epsilon = 1e-3);
    }

    #[test]
    fn rejects_corrupt_data() {
        assert!(BodyModel::from_bytes(b"nonsense").is_err());
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/body/makehuman.lpbody"
        );
        let data = std::fs::read(path).unwrap();
        assert!(BodyModel::from_bytes(&data[..data.len() / 2]).is_err());
    }

    #[test]
    fn default_body_has_plausible_measurements() {
        let m = test_model();
        let me = Measurer::new(&m).unwrap();
        let b = fit(&m, &me, &Measurements::new(), &ShapeOptions::default()).unwrap();
        let get = |k| b.measured.iter().find(|x| x.kind == k).unwrap().value;
        for (k, lo, hi) in [
            (Height, 1500.0, 1700.0),
            (Inseam, 680.0, 800.0),
            (Bust, 780.0, 900.0),
            (Waist, 600.0, 720.0),
            (Hips, 880.0, 1000.0),
            (Thigh, 480.0, 600.0),
            (Wrist, 110.0, 170.0),
        ] {
            assert!((lo..hi).contains(&get(k)), "{} = {}", k.name(), get(k));
        }
        // Body proportions: the waist is the narrowest torso girth.
        assert!(get(Waist) < get(Underbust) && get(Underbust) < get(Bust));
        assert_eq!(b.tapes.len(), 11);
        assert_eq!(b.iterations, 0);
    }

    #[test]
    fn fits_a_full_set_of_measurements() {
        let m = test_model();
        let me = Measurer::new(&m).unwrap();
        let want = measurements(&[
            (Height, 1680.0),
            (Inseam, 780.0),
            (WaistHeight, 1050.0),
            (ArmLength, 560.0),
            (Neck, 330.0),
            (Bust, 940.0),
            (Underbust, 790.0),
            (Waist, 740.0),
            (Hips, 980.0),
            (Thigh, 570.0),
            (Knee, 370.0),
            (Calf, 360.0),
            (Ankle, 220.0),
            (UpperArm, 280.0),
            (Wrist, 160.0),
        ]);
        let b = fit(&m, &me, &want, &ShapeOptions::default()).unwrap();
        for x in &b.measured {
            let r = x.requested.unwrap();
            assert!(
                (x.value - r).abs() < 1.0,
                "{}: {} vs {}",
                x.kind.name(),
                x.value,
                r
            );
        }
        assert!(b.mesh.edges().iter().all(|e| !e.is_boundary()));
    }

    #[test]
    fn fits_a_few_measurements_and_predicts_the_rest() {
        let m = test_model();
        let me = Measurer::new(&m).unwrap();
        let masculine = ShapeOptions {
            body_type: 1.0,
            ..Default::default()
        };
        let want = measurements(&[
            (Height, 1830.0),
            (Bust, 1050.0),
            (Waist, 900.0),
            (Hips, 1020.0),
        ]);
        let b = fit(&m, &me, &want, &masculine).unwrap();
        let get = |k| b.measured.iter().find(|x| x.kind == k).unwrap();
        for &k in want.keys() {
            assert!((get(k).value - want[&k]).abs() < 1.0, "{}", k.name());
        }
        // Unmeasured parts scale along sensibly.
        assert!(get(Inseam).requested.is_none());
        assert!(
            (780.0..950.0).contains(&get(Inseam).value),
            "{}",
            get(Inseam).value
        );
        assert!(
            (330.0..450.0).contains(&get(Neck).value),
            "{}",
            get(Neck).value
        );
    }

    #[test]
    fn rejects_bad_input() {
        let m = test_model();
        let me = Measurer::new(&m).unwrap();
        let bad = measurements(&[(Waist, -5.0)]);
        assert!(fit(&m, &me, &bad, &ShapeOptions::default()).is_err());
        let opts = ShapeOptions {
            body_type: 1.5,
            ..Default::default()
        };
        assert!(fit(&m, &me, &Measurements::new(), &opts).is_err());
    }
}
