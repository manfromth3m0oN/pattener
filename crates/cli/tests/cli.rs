//! Smoke tests: run the binary on the checked-in meshes.

use std::path::PathBuf;
use std::process::Command;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/data")
        .join(name)
}

fn run(mesh: &str, up: &str, out: &PathBuf) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_latexpat"))
        .arg("flatten")
        .arg(data(&format!("{mesh}.obj")))
        .arg("--seams")
        .arg(data(&format!("{mesh}.seams.json")))
        .args(["--ease", "0.85", "--heatmap", "area", "--up", up, "-o"])
        .arg(out)
        .output()
        .expect("run latexpat")
}

#[test]
fn flattens_all_test_meshes() {
    for (mesh, up, panels) in [("grid", "y", 2), ("tube", "z", 1), ("cap", "z", 2)] {
        let out =
            std::env::temp_dir().join(format!("latexpat-cli-test-{mesh}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let o = run(mesh, up, &out);
        assert!(
            o.status.success(),
            "{mesh}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(
            o.stderr.is_empty(),
            "{mesh} warnings: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        for i in 0..panels {
            let svg = std::fs::read_to_string(out.join(format!("panel_{i:02}.svg"))).unwrap();
            assert!(svg.contains("mm\" height=") && svg.contains("id=\"heatmap\""));
        }
        assert!(!out.join(format!("panel_{panels:02}.svg")).exists());
        let report = std::fs::read_to_string(out.join("report.json")).unwrap();
        assert!(report.contains("\"flipped_triangles\": []"));
        std::fs::remove_dir_all(&out).unwrap();
    }
}

#[test]
fn reports_non_disc_panel() {
    let out = std::env::temp_dir().join(format!("latexpat-cli-test-nodisc-{}", std::process::id()));
    let o = Command::new(env!("CARGO_BIN_EXE_latexpat"))
        .arg("flatten")
        .arg(data("tube.obj"))
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("not a topological disc"));
}

#[test]
fn seam_mismatch_is_matched_or_warned() {
    let out = std::env::temp_dir().join(format!("latexpat-cli-test-ring-{}", std::process::id()));
    let base = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_latexpat"))
            .arg("flatten")
            .arg(data("cap.obj"))
            .arg("--seams")
            .arg(data("cap.ring.seams.json"))
            .args(["--up", "z", "-o"])
            .arg(&out)
            .args(extra)
            .output()
            .unwrap()
    };

    // Matching on (default): no warnings, report records before/after.
    let o = base(&[]);
    assert!(o.status.success());
    assert!(
        o.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let report = std::fs::read_to_string(out.join("report.json")).unwrap();
    assert!(
        report.contains("\"seam\": \"ring\"") && report.contains("mismatch_before_matching_mm")
    );
    let svg = std::fs::read_to_string(out.join("panel_01.svg")).unwrap();
    assert!(svg.contains("ring ↔ panel 0"));

    // Matching off: the ring seam's ~3 mm mismatch triggers a warning.
    let o = base(&["--no-seam-matching"]);
    assert!(o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("warning: seam 'ring'"), "{err}");
    std::fs::remove_dir_all(&out).unwrap();
}
