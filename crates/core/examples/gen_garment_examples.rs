//! Write the example garment projects:
//! `cargo run -p latexpat-core --features serde,bundled-body --example gen_garment_examples`
//! writes `tests/data/top.project.json` and `web/examples/top.project.json`.

use latexpat_core::garment::examples::top;
use latexpat_core::human::{fit, BodyModel, Measurements, Measurer, ShapeOptions};
use latexpat_core::materials::Sheet;
use latexpat_core::project::{BodySpec, Project};

fn main() {
    let model = BodyModel::bundled();
    let measurer = Measurer::new(&model).unwrap();
    let body = fit(
        &model,
        &measurer,
        &Measurements::new(),
        &ShapeOptions::default(),
    )
    .unwrap();
    let design = top(&body.mesh, &body.tapes);
    let project = Project {
        name: "Short-sleeved top".into(),
        body: BodySpec::default(),
        garment: design,
        materials: vec![
            Sheet {
                name: "black 0.40".into(),
                color: "#151515".into(),
                thickness_mm: 0.4,
                roll_width_mm: 1000.0,
                ease: None,
                shear_modulus_mpa: None,
                price_per_m: Some(28.0),
            },
            Sheet {
                name: "red 0.40".into(),
                color: "#b01020".into(),
                thickness_mm: 0.4,
                roll_width_mm: 1000.0,
                ease: None,
                shear_modulus_mpa: None,
                price_per_m: Some(30.0),
            },
        ],
        ..Default::default()
    };
    let json = serde_json::to_string_pretty(&project).unwrap() + "\n";
    for path in [
        "tests/data/top.project.json",
        "web/examples/top.project.json",
    ] {
        std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap()).unwrap();
        std::fs::write(path, &json).unwrap();
        println!("wrote {path}");
    }
}
