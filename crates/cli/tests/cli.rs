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
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("cannot be laid flat: it still wraps around")
    );
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

#[test]
fn allowance_notches_and_layout() {
    let out = std::env::temp_dir().join(format!("latexpat-cli-test-m3-{}", std::process::id()));
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_latexpat"))
            .arg("flatten")
            .arg(data("grid.obj"))
            .arg("--seams")
            .arg(data("grid.seams.json"))
            .arg("-o")
            .arg(&out)
            .args(extra)
            .output()
            .unwrap()
    };
    let o = run(&["--sheet-width", "300", "--glue-allowance", "8"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    // 85 x 136 mm halves (ease 0.85); exactly one gets the 8 mm lap.
    let mut widths: Vec<f64> = (0..2)
        .map(|i| report["panels"][i]["cut_width_mm"].as_f64().unwrap())
        .collect();
    widths.sort_by(f64::total_cmp);
    assert!(
        (widths[0] - 85.0).abs() < 1e-6 && (widths[1] - 93.0).abs() < 1e-6,
        "{widths:?}"
    );
    // 160 mm seam: two automatic notches plus the explicit one, on both pieces.
    for i in 0..2 {
        assert_eq!(report["panels"][i]["notches"].as_array().unwrap().len(), 3);
    }
    assert!(report["layout"]["length_mm"].as_f64().unwrap() > 0.0);
    let layout = std::fs::read_to_string(out.join("layout.svg")).unwrap();
    assert!(layout.contains("panel 0") && layout.contains("panel 1"));
    let p0 = std::fs::read_to_string(out.join("panel_00.svg")).unwrap();
    assert!(p0.contains("id=\"notches\"") && p0.contains("id=\"cut-line\""));

    // A sheet narrower than both pieces' widths fails with a clear message.
    let o = run(&["--sheet-width", "80", "--no-rotate"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("does not fit a 80 mm sheet"));
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn calibrate_command_prints_ease_per_sheet() {
    let o = Command::new(env!("CARGO_BIN_EXE_latexpat"))
        .arg("calibrate")
        .arg(data("swatches.json"))
        .args(["--thickness", "0.5"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = String::from_utf8_lossy(&o.stdout);
    // The reference sheet keeps its ease; thicker sheets get less reduction.
    let ease = |name: &str| -> f64 {
        out.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().last())
            .unwrap()
            .parse()
            .unwrap()
    };
    assert_eq!(ease("0.40mm black"), 0.85);
    assert!(
        ease("0.25mm black") < 0.85 && ease("0.60mm black") > 0.85,
        "{out}"
    );
    assert!(out.contains("0.500 mm → ease"));
}

#[test]
fn flatten_with_calibrated_sheets() {
    let out = std::env::temp_dir().join(format!("latexpat-cli-test-m4-{}", std::process::id()));
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_latexpat"))
            .arg("flatten")
            .arg(data("grid.obj"))
            .arg("--seams")
            .arg(data("grid.seams.json"))
            .arg("-o")
            .arg(&out)
            .args(extra)
            .output()
            .unwrap()
    };
    let cal = data("swatches.json");
    let cal = cal.to_str().unwrap();
    let o = run(&[
        "--calibration",
        cal,
        "--sheet",
        "0.40mm black",
        "--panel-sheet",
        "1=0.60mm black",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let r: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(r["panels"][0]["sheet"], "0.40mm black");
    assert_eq!(r["panels"][1]["sheet"], "0.60mm black");
    let (e0, e1) = (
        r["panels"][0]["ease"].as_f64().unwrap(),
        r["panels"][1]["ease"].as_f64().unwrap(),
    );
    assert!((e0 - 0.85).abs() < 1e-9 && e1 > 0.88, "{e0} {e1}");
    // The seam settles nearer the stiffer (thicker) side's natural length.
    let seam = &r["seams"][0];
    let len = seam["left_length_mm"].as_f64().unwrap();
    let mid = 0.5 * 160.0 * (e0 + e1);
    assert!(len > mid, "{len} vs plain mean {mid}");

    // A thickness instead of a name uses the pooled fit.
    let o = run(&["--calibration", cal, "--sheet", "0.5mm"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));

    // Bad combinations.
    let o = run(&["--calibration", cal, "--sheet", "nope"]);
    assert!(String::from_utf8_lossy(&o.stderr).contains("unknown sheet 'nope'"));
    let o = run(&["--calibration", cal, "--sheet", "0.5mm", "--ease", "0.8"]);
    assert!(!o.status.success());
    let o = run(&[
        "--calibration",
        cal,
        "--sheet",
        "0.5mm",
        "--panel-sheet",
        "7=0.5mm",
    ]);
    assert!(String::from_utf8_lossy(&o.stderr).contains("panel 7 does not exist"));
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn body_from_measurements_then_flatten() {
    let out = std::env::temp_dir().join(format!("latexpat-cli-test-body-{}", std::process::id()));
    let o = Command::new(env!("CARGO_BIN_EXE_latexpat"))
        .arg("body")
        .arg(data("measurements.json"))
        .args([
            "--style",
            "princess",
            "--around",
            "32",
            "--ring-spacing",
            "25",
            "-o",
        ])
        .arg(&out)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let seams = std::fs::read_to_string(out.join("body.seams.json")).unwrap();
    assert!(seams.contains("princess_front_left") && seams.contains("\"notches\""));

    let pat = out.join("pattern");
    let o = Command::new(env!("CARGO_BIN_EXE_latexpat"))
        .arg("flatten")
        .arg(out.join("body.obj"))
        .arg("--seams")
        .arg(out.join("body.seams.json"))
        .arg("-o")
        .arg(&pat)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(
        o.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(pat.join("panel_05.svg").exists() && !pat.join("panel_06.svg").exists());
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn human_body_from_project() {
    let out = std::env::temp_dir().join(format!("latexpat-cli-test-human-{}", std::process::id()));
    let o = Command::new(env!("CARGO_BIN_EXE_latexpat"))
        .arg("human")
        .arg(data("project.json"))
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(
        o.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("measurements.json")).unwrap())
            .unwrap();
    let waist = m
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["kind"] == "waist")
        .unwrap();
    assert!((waist["value"].as_f64().unwrap() - 740.0).abs() < 1.0);
    assert!(std::fs::read_to_string(out.join("body.obj"))
        .unwrap()
        .starts_with("v "));
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn flatten_garment_from_project() {
    let out =
        std::env::temp_dir().join(format!("latexpat-cli-test-garment-{}", std::process::id()));
    let o = Command::new(env!("CARGO_BIN_EXE_latexpat"))
        .arg("flatten")
        .arg("--project")
        .arg(data("top.project.json"))
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(out.join("panel_00.svg").exists() && out.join("panel_01.svg").exists());
    assert!(!out.join("panel_02.svg").exists());
    // The design has no bust shaping, so the front's seams come out longer.
    assert!(String::from_utf8_lossy(&o.stderr).contains("easing in"));
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn project_materials_give_a_layout_per_sheet() {
    let out = std::env::temp_dir().join(format!(
        "latexpat-cli-test-materials-{}",
        std::process::id()
    ));
    let o = Command::new(env!("CARGO_BIN_EXE_latexpat"))
        .arg("flatten")
        .arg("--project")
        .arg(data("top.project.json"))
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let stdout = String::from_utf8_lossy(&o.stdout);
    // Every piece defaults to the first sheet of the catalogue.
    assert!(
        stdout.contains("black 0.40") && stdout.contains("layout_black_0.40.svg"),
        "{stdout}"
    );
    assert!(out.join("layout_black_0.40.svg").exists());
    let r: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    let m = &r["materials"][0];
    assert_eq!(m["pieces"].as_array().unwrap().len(), 2);
    assert!(m["cost"].as_f64().unwrap() > 0.0);
    assert_eq!(r["panels"][0]["sheet"], "black 0.40");
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn project_construction_and_print() {
    let out = std::env::temp_dir().join(format!("latexpat-cli-test-print-{}", std::process::id()));
    let o = Command::new(env!("CARGO_BIN_EXE_latexpat"))
        .arg("flatten")
        .arg("--project")
        .arg(data("top.project.json"))
        .args(["--pdf", "letter"])
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let stdout = String::from_utf8_lossy(&o.stdout);
    // The example top has a bound neckline and a folded hem.
    assert!(stdout.contains("trim for neckline"), "{stdout}");
    assert!(stdout.contains("Hem the hem edge"), "{stdout}");
    assert!(stdout.contains("Finish the neckline edge"), "{stdout}");
    assert!(out.join("trim_00.svg").exists());
    let pdf = std::fs::read_to_string(out.join("pattern.pdf")).unwrap();
    assert!(pdf.starts_with("%PDF-1.4") && pdf.contains("/MediaBox [0 0 612.00 792.00]"));
    let r: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(r["trims"][0]["edge"], "neckline");
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn project_pattern_checked_in_a_pose() {
    let out = std::env::temp_dir().join(format!("latexpat-cli-test-pose-{}", std::process::id()));
    let o = Command::new(env!("CARGO_BIN_EXE_latexpat"))
        .arg("flatten")
        .arg("--project")
        .arg(data("top.project.json"))
        .args(["--pose", "arms-out"])
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("in pose"));
    let r: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    assert!(r["panels"][0]["posed_p95_stretch"].as_f64().unwrap() > 1.0);
    let bad = Command::new(env!("CARGO_BIN_EXE_latexpat"))
        .arg("flatten")
        .arg("--project")
        .arg(data("top.project.json"))
        .args(["--pose", "handstand"])
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unknown pose"));
    std::fs::remove_dir_all(&out).unwrap();
}
