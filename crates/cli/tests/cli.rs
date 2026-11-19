//! Smoke tests: run the binary from measurements to a printed pattern.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/data")
        .join(name)
}

fn latexpat(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_latexpat"))
        .args(args)
        .output()
        .expect("run latexpat")
}

fn ok(o: &Output) -> String {
    assert!(
        o.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("latexpat-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn lists_templates_and_estimates_the_body() {
    let out = ok(&latexpat(&["templates"]));
    assert!(
        out.contains("top-sleeves") && out.contains("leggings"),
        "{out}"
    );
    let out = ok(&latexpat(&[
        "body",
        "--measurements",
        s(&data("measurements.json")),
    ]));
    // Given values are printed as given; the rest are estimated.
    assert!(out
        .lines()
        .any(|l| l.starts_with("bust") && l.contains("940") && !l.contains("estimated")));
    assert!(out
        .lines()
        .any(|l| l.starts_with("knee") && l.contains("estimated")));
    assert!(out.contains("underarm"));
}

#[test]
fn measurements_to_printed_pattern() {
    let dir = tmp("top");
    let project = dir.join("top.latexpat.json");
    ok(&latexpat(&[
        "template",
        "top-sleeves",
        "--measurements",
        s(&data("measurements.json")),
        "-o",
        s(&project),
    ]));
    let out_dir = dir.join("out");
    let out = ok(&latexpat(&[
        "pattern",
        s(&project),
        "-o",
        s(&out_dir),
        "--pdf",
        "letter",
        "--sheet-width",
        "1000",
    ]));
    assert!(out.contains("assembly:"), "{out}");
    for f in [
        "piece_00.svg",
        "piece_01.svg",
        "piece_02.svg",
        "layout.svg",
        "layout_black_0.40.svg",
        "pattern.pdf",
        "report.json",
        "trim_00.svg",
    ] {
        assert!(out_dir.join(f).exists(), "missing {f}");
    }
    let pdf = std::fs::read_to_string(out_dir.join("pattern.pdf")).unwrap();
    assert!(pdf.contains("/MediaBox [0 0 612.00 792.00]"));
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out_dir.join("report.json")).unwrap())
            .unwrap();
    assert_eq!(report["pieces"].as_array().unwrap().len(), 3);
    assert_eq!(report["pieces"][2]["copies"], 2);

    // New measurements: the design follows them.
    let bigger = dir.join("bigger.json");
    std::fs::write(
        &bigger,
        r#"{"height": 1750, "bust": 1000, "waist": 800, "hips": 1040}"#,
    )
    .unwrap();
    ok(&latexpat(&[
        "redraft",
        s(&project),
        "--measurements",
        s(&bigger),
    ]));
    let p: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&project).unwrap()).unwrap();
    assert_eq!(p["body"]["measurements"]["bust"], 1000.0);
    assert_eq!(p["design"]["block"]["body"]["measurements"]["bust"], 1000.0);
}

#[test]
fn old_3d_projects_are_explained() {
    let dir = tmp("old");
    let old = dir.join("old.json");
    std::fs::write(&old, r#"{"version": 1, "garment": {"curves": []}}"#).unwrap();
    let o = latexpat(&["pattern", s(&old), "-o", s(&dir.join("out"))]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("3D editor"));
    // Its measurements still start a new design.
    ok(&latexpat(&[
        "template",
        "skirt",
        "--project",
        s(&old),
        "-o",
        s(&dir.join("new.json")),
    ]));
}

#[test]
fn calibrate_command_prints_ease_per_sheet() {
    let out = ok(&latexpat(&[
        "calibrate",
        s(&data("swatches.json")),
        "--thickness",
        "0.5",
    ]));
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
fn pattern_with_calibrated_sheets() {
    let dir = tmp("calib");
    let project = dir.join("p.json");
    ok(&latexpat(&["template", "cincher", "-o", s(&project)]));
    let out_dir = dir.join("out");
    ok(&latexpat(&[
        "pattern",
        s(&project),
        "-o",
        s(&out_dir),
        "--calibration",
        s(&data("swatches.json")),
        "--sheet",
        "0.60mm black",
    ]));
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out_dir.join("report.json")).unwrap())
            .unwrap();
    // The project's "black 0.40" sheet is not in the calibration by name,
    // so it takes the pooled fit for 0.4 mm: close to the 0.40 mm reference.
    let ease = report["pieces"][0]["ease"].as_f64().unwrap();
    assert!((ease - 0.85).abs() < 0.02, "{ease}");
    assert!(report["calibration"].is_object());
}
