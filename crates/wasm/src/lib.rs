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
//! ```
//! `options` for `pattern` is a [`JobOptions`] (every field optional).
//! Responses are `{"ok": true, ...}` or `{"ok": false, "error": "..."}`.

use latexpat_core::body::{torso, BodyMeasurements, BodyOptions};
use latexpat_core::calibrate::{calibrate, Calibration};
use latexpat_core::job::{self, JobOptions};
use latexpat_core::obj::{parse_obj, write_obj};
use latexpat_core::SeamSet;
use serde::Deserialize;
use serde_json::{json, Value};

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
