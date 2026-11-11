//! WebAssembly entry point for latexpat.
//!
//! The interface is deliberately tiny so it needs no binding generator:
//! plain `cargo build --target wasm32-unknown-unknown` produces a module
//! with three exports.
//!
//! - `alloc(len) -> ptr` / `dealloc(ptr, len)`: manage buffers in the
//!   module's memory.
//! - `call(ptr, len) -> out`: read a UTF-8 JSON request of `len` bytes at
//!   `ptr`, run it, and return a pointer to a buffer laid out as a
//!   little-endian `u32` byte length followed by the UTF-8 JSON response.
//!   The caller frees it with `dealloc(out, 4 + length)`.
//!
//! All the logic is in [`handle`], which is plain Rust and tested natively.
//!
//! ## Requests
//! ```json
//! { "command": "body", "measurements": {"levels": [...]}, "options": {"style": "princess"} }
//! { "command": "pattern", "obj": "v ...", "seams": {"seams": [...]}, "scale": 1, "options": { ... } }
//! { "command": "calibrate", "calibration": { "sheets": [...], "target": ... } }
//! { "command": "human", "body": { "measurements": {"height": 1680, ...}, "shape": {...} } }
//! { "command": "garment", "body": {...}, "design": {...}, "pattern": false, "options": {...} }
//! ```
//! `options` for `pattern` is a [`JobOptions`] (every field optional); with
//! `"print": {"paper": "a4"}` the output carries a tiled PDF as `pdf`.
//! Responses are `{"ok": true, ...}` or `{"ok": false, "error": "..."}`.

use latexpat_core::body::{torso, BodyMeasurements, BodyOptions};
use latexpat_core::calibrate::{calibrate, Calibration};
use latexpat_core::cut::cut_panels;
use latexpat_core::garment::{build, material_plan, panel_faces, zones_on, GarmentDesign};
use latexpat_core::glam::DVec3;
use latexpat_core::human::{along_body, fit, BodyModel, HumanBody, Measurer};
use latexpat_core::job::{self, JobOptions};
use latexpat_core::materials::Sheet;
use latexpat_core::obj::{parse_obj, write_obj};
use latexpat_core::project::BodySpec;
use latexpat_core::SeamSet;
use serde::Deserialize;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::sync::OnceLock;

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // one request per call
enum Request {
    Body {
        measurements: BodyMeasurements,
        #[serde(default)]
        options: BodyOptions,
    },
    Pattern {
        obj: String,
        #[serde(default)]
        seams: SeamSet,
        #[serde(default = "one")]
        scale: f64,
        #[serde(default)]
        options: JobOptions,
    },
    Calibrate {
        calibration: Calibration,
    },
    Human {
        #[serde(default)]
        body: BodySpec,
    },
    Garment {
        #[serde(default)]
        body: BodySpec,
        #[serde(default)]
        design: GarmentDesign,
        /// Also make the pattern (otherwise just the preview).
        #[serde(default)]
        pattern: bool,
        /// The project's latex sheets.
        #[serde(default)]
        materials: Vec<Sheet>,
        #[serde(default)]
        options: JobOptions,
    },
}

thread_local! {
    /// The last fitted body, keyed by its spec, so editing a garment does
    /// not refit the body.
    static FITTED: RefCell<Option<(String, HumanBody)>> = const { RefCell::new(None) };
}

fn fitted(spec: &BodySpec) -> Result<HumanBody, String> {
    let key = serde_json::to_string(spec).map_err(|e| e.to_string())?;
    if let Some(b) = FITTED.with(|c| {
        c.borrow()
            .as_ref()
            .filter(|(k, _)| *k == key)
            .map(|(_, b)| b.clone())
    }) {
        return Ok(b);
    }
    let (model, measurer) = human();
    let b = fit(model, measurer, &spec.measurements, &spec.shape).map_err(|e| e.to_string())?;
    FITTED.with(|c| *c.borrow_mut() = Some((key, b.clone())));
    Ok(b)
}

fn flat3(points: impl Iterator<Item = DVec3>) -> Vec<f32> {
    points
        .flat_map(|p| [p.x as f32, p.y as f32, p.z as f32])
        .collect()
}

/// The body model and measurer, loaded once.
fn human() -> &'static (BodyModel, Measurer) {
    static CELL: OnceLock<(BodyModel, Measurer)> = OnceLock::new();
    CELL.get_or_init(|| {
        let model = BodyModel::bundled();
        let measurer = Measurer::new(&model).expect("bundled model has all measurement targets");
        (model, measurer)
    })
}

fn one() -> f64 {
    1.0
}

/// Run one JSON request and return the JSON response.
pub fn handle(request: &str) -> String {
    let result = run(request).unwrap_or_else(|e| json!({ "ok": false, "error": e }));
    result.to_string()
}

fn run(request: &str) -> Result<Value, String> {
    let req: Request = serde_json::from_str(request).map_err(|e| format!("bad request: {e}"))?;
    match req {
        Request::Body {
            measurements,
            options,
        } => {
            let b = torso(&measurements, &options).map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true, "obj": write_obj(&b.mesh), "seams": b.seams }))
        }
        Request::Pattern {
            obj,
            seams,
            scale,
            options,
        } => {
            if !(scale > 0.0 && scale.is_finite()) {
                return Err("scale must be a positive number".into());
            }
            let mut mesh = parse_obj(&obj).map_err(|e| e.to_string())?;
            mesh.scale(scale);
            let out = job::run(&mesh, &seams, &options).map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true, "output": out }))
        }
        Request::Human { body } => {
            let (model, measurer) = human();
            let b =
                fit(model, measurer, &body.measurements, &body.shape).map_err(|e| e.to_string())?;
            // Flat arrays keep the response compact for the 3D viewer.
            let positions: Vec<f32> = b
                .mesh
                .positions()
                .iter()
                .flat_map(|p| [p.x as f32, p.y as f32, p.z as f32])
                .collect();
            let triangles: Vec<u32> = b.mesh.triangles().iter().flatten().copied().collect();
            let tapes: Vec<Value> = b
                .tapes
                .iter()
                .map(|(k, pts)| {
                    let flat: Vec<f32> = pts
                        .iter()
                        .flat_map(|p| [p.x as f32, p.y as f32, p.z as f32])
                        .collect();
                    json!({ "kind": k, "points": flat })
                })
                .collect();
            Ok(json!({
                "ok": true,
                "positions": positions,
                "triangles": triangles,
                "measured": b.measured,
                "tapes": tapes,
                "parameters": b.parameters,
                "iterations": b.iterations,
                "obj": write_obj(&b.mesh),
            }))
        }
        Request::Garment {
            body,
            design,
            pattern,
            materials,
            options,
        } => {
            let b = fitted(&body)?;
            let g = build(&b.mesh, &design).map_err(|e| e.to_string())?;
            let garment_triangles: Vec<u32> = g
                .body
                .triangles()
                .iter()
                .zip(&g.region)
                .filter(|(_, &r)| r)
                .flat_map(|(t, _)| *t)
                .collect();
            let curves: Vec<Value> = g
                .curves
                .iter()
                .map(|c| {
                    let pts = flat3(c.vertices.iter().map(|&v| g.body.positions()[v as usize]));
                    json!({ "name": c.name, "kind": c.kind, "closed": c.closed, "points": pts })
                })
                .collect();
            // Can the garment be cut into flat panels as drawn?
            let panels = g.mesh.as_ref().map(|m| {
                g.seams
                    .resolve(m)
                    .and_then(|e| cut_panels(m, &e))
                    .map(|p| p.len())
                    .map_err(|e| e.to_string())
            });
            let zones = zones_on(&b.mesh, &design).map_err(|e| e.to_string())?;
            // Each piece with its sheet, for the colour preview.
            let (plan, mut plan_warnings) =
                material_plan(&b.mesh, &g, &design, &materials).map_err(|e| e.to_string())?;
            let mut pieces = Vec::new();
            if let (Some(gm), true) = (&g.mesh, panels.as_ref().is_some_and(|p| p.is_ok())) {
                for (pi, faces) in panel_faces(&g)
                    .map_err(|e| e.to_string())?
                    .iter()
                    .enumerate()
                {
                    let tris: Vec<u32> = faces
                        .iter()
                        .flat_map(|&f| {
                            gm.triangles()[f as usize].map(|v| g.vertex_origin[v as usize])
                        })
                        .collect();
                    let sheet = plan.as_ref().map(|p| &p.sheets[p.panel_sheet[pi]]);
                    pieces.push(json!({
                        "triangles": tris,
                        "sheet": sheet.map(|s| s.name.clone()),
                        "color": sheet.map(|s| s.color.clone()),
                    }));
                }
            }
            let output = match (&g.mesh, pattern) {
                (Some(m), true) => {
                    let mut options = options;
                    // The body is Y-up; "along" follows its bones.
                    options.up = DVec3::Y;
                    let centres: Vec<DVec3> = m
                        .triangles()
                        .iter()
                        .map(|t| t.iter().map(|&i| m.positions()[i as usize]).sum::<DVec3>() / 3.0)
                        .collect();
                    options.along =
                        Some(along_body(&b.joints, &centres).map_err(|e| e.to_string())?);
                    options.ease_zones = zones.clone();
                    options.materials = plan.clone();
                    options.edges = g.edges.clone();
                    options.mirror = g.mirror.clone();
                    Some(job::run(m, &g.seams, &options).map_err(|e| e.to_string())?)
                }
                (None, true) => {
                    return Err("draw the garment's edges and click inside it first".into())
                }
                _ => None,
            };
            // Pressure on the cut body's vertices (0 off the garment).
            let pressure = output.as_ref().map(|o| {
                let mut p = vec![0.0f32; g.body.positions().len()];
                for (&v, &x) in g.vertex_origin.iter().zip(&o.fit.pressure_mmhg) {
                    p[v as usize] = x as f32;
                }
                p
            });
            let zone_json: Vec<Value> = zones
                .iter()
                .map(|z| json!({ "center": [z.center.x, z.center.y, z.center.z], "radius_mm": z.radius_mm, "ease": z.ease }))
                .collect();
            let mut warnings = g.warnings.clone();
            warnings.append(&mut plan_warnings);
            Ok(json!({
                "ok": true,
                "positions": flat3(g.body.positions().iter().copied()),
                "triangles": g.body.triangles().iter().flatten().copied().collect::<Vec<u32>>(),
                "garment_triangles": garment_triangles,
                "curves": curves,
                "warnings": warnings,
                "panels": panels.as_ref().and_then(|p| p.as_ref().ok()),
                "panel_error": panels.as_ref().and_then(|p| p.as_ref().err()),
                "zones": zone_json,
                "pieces": pieces,
                "pressure": pressure,
                "output": output,
            }))
        }
        Request::Calibrate { calibration } => {
            let r = calibrate(&calibration).map_err(|e| e.to_string())?;
            Ok(json!({ "ok": true, "result": r }))
        }
    }
}

/// Allocate `len` bytes in module memory for the caller to fill.
#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Free a buffer from [`alloc`] or [`call`].
///
/// # Safety
/// `ptr` must come from `alloc(len)` or be a `call` result of total size
/// `len`, and must not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len));
}

/// Handle the JSON request in `ptr[..len]` (which is freed) and return a
/// length-prefixed JSON response (see the crate docs).
///
/// # Safety
/// `ptr` must come from `alloc(len)` and hold `len` initialised bytes.
#[no_mangle]
pub unsafe extern "C" fn call(ptr: *mut u8, len: usize) -> *mut u8 {
    let input = Vec::from_raw_parts(ptr, len, len);
    let response = match std::str::from_utf8(&input) {
        Ok(s) => handle(s),
        Err(_) => json!({ "ok": false, "error": "request is not UTF-8" }).to_string(),
    };
    let mut out = Vec::with_capacity(4 + response.len());
    out.extend_from_slice(&(response.len() as u32).to_le_bytes());
    out.extend_from_slice(response.as_bytes());
    let mut out = std::mem::ManuallyDrop::new(out.into_boxed_slice());
    out.as_mut_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn body_then_pattern() {
        let body = parse(&handle(
            r#"{"command": "body",
                "measurements": {"levels": [
                    {"name": "hip", "height_mm": 0, "circumference_mm": 960},
                    {"name": "waist", "height_mm": 220, "circumference_mm": 720},
                    {"name": "bust", "height_mm": 420, "circumference_mm": 900}]},
                "options": {"around": 32, "ring_spacing_mm": 25, "style": "tube"}}"#,
        ));
        assert_eq!(body["ok"], true, "{body}");
        let req = json!({
            "command": "pattern",
            "obj": body["obj"],
            "seams": body["seams"],
            "options": { "ease": {"uniform": 0.8}, "heatmap": "max_stretch",
                         "sheet": {"sheet_width": 600, "gap": 5, "allow_rotation": true} }
        });
        let out = parse(&handle(&req.to_string()));
        assert_eq!(out["ok"], true, "{out}");
        let o = &out["output"];
        assert_eq!(o["panel_svgs"].as_array().unwrap().len(), 2);
        assert!(o["layout_svg"].as_str().unwrap().starts_with("<?xml"));
        assert_eq!(o["report"]["panels"][0]["ease"], 0.8);
    }

    #[test]
    fn human_body_fits_measurements() {
        let r = parse(&handle(
            r#"{"command": "human", "body": {"measurements": {"height": 1700, "waist": 720},
                "shape": {"body_type": 0.2}}}"#,
        ));
        assert_eq!(
            r["ok"],
            true,
            "{}",
            &r.to_string()[..200.min(r.to_string().len())]
        );
        assert_eq!(r["positions"].as_array().unwrap().len(), 3 * 13380);
        let measured = r["measured"].as_array().unwrap();
        let waist = measured.iter().find(|m| m["kind"] == "waist").unwrap();
        assert!((waist["value"].as_f64().unwrap() - 720.0).abs() < 1.0);
        assert_eq!(waist["requested"], 720.0);
        assert!(r["tapes"].as_array().unwrap().len() >= 10);
        let bad = parse(&handle(
            r#"{"command": "human", "body": {"measurements": {"elbow": 3}}}"#,
        ));
        assert_eq!(bad["ok"], false);
    }

    #[test]
    fn garment_preview_and_pattern() {
        let project: Value = serde_json::from_str(
            &std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/data/top.project.json"
            ))
            .unwrap(),
        )
        .unwrap();
        let req = |pattern: bool| {
            json!({ "command": "garment", "body": project["body"], "design": project["garment"],
                    "materials": project["materials"], "pattern": pattern,
                    "options": { "print": { "paper": "letter" } } })
            .to_string()
        };
        let preview = parse(&handle(&req(false)));
        assert_eq!(preview["ok"], true, "{}", preview["error"]);
        assert_eq!(preview["panels"], 2);
        assert!(preview["panel_error"].is_null());
        assert_eq!(preview["curves"].as_array().unwrap().len(), 8);
        assert!(preview["output"].is_null());
        // Two pieces, both on the first sheet of the catalogue.
        let pieces = preview["pieces"].as_array().unwrap();
        assert_eq!(pieces.len(), 2);
        assert!(pieces
            .iter()
            .all(|p| p["sheet"] == "black 0.40" && p["color"] == "#151515"));
        let n = preview["garment_triangles"].as_array().unwrap().len();
        assert!(n > 0 && n.is_multiple_of(3));
        let full = parse(&handle(&req(true)));
        assert_eq!(full["output"]["panel_svgs"].as_array().unwrap().len(), 2);
        // The example's finishes: a neckline trim and a step for each.
        assert_eq!(full["output"]["report"]["trims"][0]["edge"], "neckline");
        assert_eq!(full["output"]["trim_svgs"].as_array().unwrap().len(), 1);
        let steps = full["output"]["report"]["assembly"].as_array().unwrap();
        assert!(steps
            .iter()
            .any(|s| s.as_str().unwrap().starts_with("Hem the hem")));
        assert!(full["output"]["pdf"]
            .as_str()
            .unwrap()
            .contains("/MediaBox [0 0 612.00 792.00]"));
        // Pressure on every body vertex; positive on the garment somewhere.
        let p = full["pressure"].as_array().unwrap();
        assert_eq!(p.len(), full["positions"].as_array().unwrap().len() / 3);
        assert!(p.iter().any(|x| x.as_f64().unwrap() > 5.0));
        assert!(
            full["output"]["report"]["panels"][0]["p95_pressure_mmhg"]
                .as_f64()
                .unwrap()
                > 0.0
        );
    }

    #[test]
    fn errors_are_reported_not_panicked() {
        let r = parse(&handle(r#"{"command": "pattern", "obj": "v 0 0\n"}"#));
        assert_eq!(r["ok"], false);
        assert!(r["error"].as_str().unwrap().contains("line 1"));
        let r = parse(&handle("not json"));
        assert!(r["error"].as_str().unwrap().starts_with("bad request"));
    }

    #[test]
    fn exports_round_trip() {
        let req = br#"{"command": "calibrate", "calibration": {"sheets": [{"name": "a", "thickness_mm": 0.4,
            "samples": [{"width_mm": 25, "gauge_mm": 100, "stretched_mm": 110, "load_n": 1}]}],
            "target": {"tension_n_per_cm": 1.0}}}"#;
        unsafe {
            let p = alloc(req.len());
            std::ptr::copy_nonoverlapping(req.as_ptr(), p, req.len());
            let out = call(p, req.len());
            let n =
                u32::from_le_bytes(std::slice::from_raw_parts(out, 4).try_into().unwrap()) as usize;
            let text = std::str::from_utf8(std::slice::from_raw_parts(out.add(4), n))
                .unwrap()
                .to_string();
            dealloc(out, 4 + n);
            assert!(parse(&text)["ok"] == true, "{text}");
        }
    }
}
