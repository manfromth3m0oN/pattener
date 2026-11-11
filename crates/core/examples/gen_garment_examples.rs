//! Write the example garment projects:
//! `cargo run -p latexpat-core --features serde,bundled-body --example gen_garment_examples`
//! writes every template to `web/examples/<id>.project.json` with an
//! `index.json`, and the top to `tests/data/top.project.json`.

use latexpat_core::garment::examples::TEMPLATES;
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
    let materials = vec![
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
    ];
    let write = |path: &str, json: &str| {
        std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap()).unwrap();
        std::fs::write(path, json).unwrap();
        println!("wrote {path}");
    };
    let mut index = Vec::new();
    for t in TEMPLATES {
        let project = Project {
            name: t.name.into(),
            body: BodySpec::default(),
            garment: (t.build)(&body),
            materials: materials.clone(),
            ..Default::default()
        };
        let json = serde_json::to_string_pretty(&project).unwrap() + "\n";
        write(&format!("web/examples/{}.project.json", t.id), &json);
        if t.id == "top" {
            write("tests/data/top.project.json", &json);
        }
        index.push(serde_json::json!({ "id": t.id, "name": t.name, "description": t.description }));
    }
    write(
        "web/examples/index.json",
        &(serde_json::to_string_pretty(&index).unwrap() + "\n"),
    );
}
