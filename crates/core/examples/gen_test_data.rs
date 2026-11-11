//! Regenerate the meshes in `tests/data/`:
//! `cargo run -p latexpat-core --example gen_test_data -- tests/data`

use latexpat_core::obj::write_obj;
use latexpat_core::testmesh;
use std::f64::consts::{PI, TAU};
use std::path::Path;

fn seams_json(seams: &[(&str, Vec<u32>)]) -> String {
    let items: Vec<String> = seams
        .iter()
        .map(|(name, v)| {
            let vs: Vec<String> = v.iter().map(|i| i.to_string()).collect();
            format!(
                "    {{ \"name\": \"{name}\", \"vertices\": [{}] }}",
                vs.join(", ")
            )
        })
        .collect();
    format!("{{\n  \"seams\": [\n{}\n  ]\n}}\n", items.join(",\n"))
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tests/data".into());
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();
    let write = |name: &str, s: String| std::fs::write(dir.join(name), s).unwrap();

    // 200 × 160 mm flat sheet, split down the middle.
    let (nx, ny) = (10, 8);
    write("grid.obj", write_obj(&testmesh::grid(nx, ny, 20.0)));
    write(
        "grid.seams.json",
        seams_json(&[("centre", (0..=ny).map(|j| j * (nx + 1) + 5).collect())]),
    );

    // Tube r = 60 mm, h = 150 mm (Z up), opened by one seam.
    let (n, up) = (32, 8);
    write(
        "tube.obj",
        write_obj(&testmesh::cylinder(60.0, 150.0, n, up, TAU)),
    );
    write(
        "tube.seams.json",
        seams_json(&[("back", (0..=up).map(|j| j * n).collect())]),
    );

    // Spherical cap R = 100 mm to 60°, split into two halves through the pole.
    let (rings, segs) = (10, 32);
    write(
        "cap.obj",
        write_obj(&testmesh::spherical_cap(100.0, PI / 3.0, rings, segs)),
    );
    let ring = |k: u32, i: u32| 1 + (k - 1) * segs + i;
    let mut path: Vec<u32> = (1..=rings).rev().map(|k| ring(k, segs / 2)).collect();
    path.push(0);
    path.extend((1..=rings).map(|k| ring(k, 0)));
    write("cap.seams.json", seams_json(&[("meridian", path)]));

    // Same cap cut around ring 5 (a dome inside a band) plus one radial seam
    // to open the band. The ring's two sides flatten to different lengths.
    let ring_loop: Vec<u32> = (0..=segs).map(|i| ring(5, i % segs)).collect();
    let radial: Vec<u32> = (5..=rings).map(|k| ring(k, 0)).collect();
    write(
        "cap.ring.seams.json",
        seams_json(&[("ring", ring_loop), ("radial", radial)]),
    );
}
