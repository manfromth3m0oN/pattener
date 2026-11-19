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
//! { "command": "body", "body": { "figure": "women", "measurements": { "height": 1680 } } }
//! { "command": "templates" }
//! { "command": "template", "id": "top", "body": { ... } }
//! { "command": "redraft", "design": { ... }, "body": { ... } }
//! { "command": "check", "design": { ... } }
//! { "command": "pattern", "design": { ... }, "options": { ... } }
//! { "command": "calibrate", "calibration": { "sheets": [...], "target": ... } }
//! ```
//! `options` for `pattern` is a [`JobOptions`] (every field optional); with
//! `"print": {"paper": "a4"}` the output carries a tiled PDF as `pdf`.
//! Responses are `{"ok": true, ...}` or `{"ok": false, "error": "..."}`.

use latexpat_core::blocks::{redraft, template, TEMPLATES};
use latexpat_core::body::{Body, BodySpec};
use latexpat_core::calibrate::{calibrate, Calibration};
use latexpat_core::design::Design;
use latexpat_core::job::{self, JobOptions};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // one request per call
enum Request {
    /// Every measurement (given or estimated) and the body's levels.
    Body {
        #[serde(default)]
        body: BodySpec,
    },
    Templates,
    /// A template drafted for a body.
    Template {
        id: String,
        #[serde(default)]
        body: BodySpec,
    },
    /// A design moved to new measurements, keeping its edits.
    Redraft {
        design: Design,
        body: BodySpec,
    },
    /// Edge and seam lengths of a design, and what stops it being made
    /// into a pattern.
    Check {
        design: Design,
    },
    Pattern {
        design: Design,
        #[serde(default)]
        options: JobOptions,
    },
    Calibrate {
        calibration: Calibration,
    },
}

/// Run one JSON request and return the JSON response.
pub fn handle(request: &str) -> String {
    let result = run(request).unwrap_or_else(|e| json!({ "ok": false, "error": e }));
    result.to_string()
}

fn run(request: &str) -> Result<Value, String> {
    let req: Request = serde_json::from_str(request).map_err(|e| format!("bad request: {e}"))?;
    let err = |e: latexpat_core::Error| e.to_string();
    match req {
        Request::Body { body } => {
            let b = Body::new(&body).map_err(err)?;
            Ok(json!({
                "ok": true,
                "values": b.values(),
                "estimated": b.estimated,
                "guides": b.guides(),
            }))
        }
        Request::Templates => Ok(json!({ "ok": true, "templates": TEMPLATES })),
        Request::Template { id, body } => {
            let design = template(&id, &body).map_err(err)?;
            Ok(json!({ "ok": true, "design": design }))
        }
        Request::Redraft { design, body } => {
            let design = redraft(&design, &body).map_err(err)?;
            Ok(json!({ "ok": true, "design": design }))
        }
        Request::Check { design } => {
            let pieces: Vec<Value> = design
                .pieces
                .iter()
                .map(|p| {
                    if p.nodes.len() < 2 {
                        return json!({ "id": p.id, "edges": [] });
                    }
                    let edges: Vec<Value> = p
                        .edges()
                        .iter()
                        .enumerate()
                        .map(|(k, e)| {
                            json!({
                                "node": p.nodes[e.start].id,
                                "length": p.edge_length(k),
                                "seam": design.seam_on(design.piece_index(&p.id).unwrap(), k),
                            })
                        })
                        .collect();
                    json!({ "id": p.id, "edges": edges })
                })
                .collect();
            Ok(json!({ "ok": true, "pieces": pieces, "problems": design.problems() }))
        }
        Request::Pattern { design, options } => {
            let out = job::run(&design, &options).map_err(err)?;
            Ok(json!({ "ok": true, "output": out }))
        }
        Request::Calibrate { calibration } => {
            let r = calibrate(&calibration).map_err(err)?;
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
    fn measurements_to_pattern() {
        let body = r#"{"figure": "women", "measurements": {"height": 1700, "bust": 960}}"#;
        let b = parse(&handle(&format!(
            r#"{{"command": "body", "body": {body}}}"#
        )));
        assert_eq!(b["ok"], true, "{b}");
        assert_eq!(b["values"]["bust"], 960.0);
        assert!(b["estimated"].as_array().unwrap().contains(&json!("waist")));
        assert!(b["guides"].as_array().unwrap().len() > 8);

        let t = parse(&handle(r#"{"command": "templates"}"#));
        assert_eq!(t["templates"][0]["id"], "top");

        let d = parse(&handle(&format!(
            r#"{{"command": "template", "id": "top", "body": {body}}}"#
        )));
        assert_eq!(d["ok"], true, "{d}");
        let design = d["design"].to_string();

        let c = parse(&handle(&format!(
            r#"{{"command": "check", "design": {design}}}"#
        )));
        assert_eq!(c["problems"].as_array().unwrap().len(), 0);
        assert!(c["pieces"][0]["edges"][0]["length"].as_f64().unwrap() > 100.0);

        let p = parse(&handle(&format!(
            r##"{{"command": "pattern", "design": {design},
                "options": {{"print": {{"paper": "a4"}}, "length_relief": 0.5,
                             "materials": [{{"name": "black", "color": "#111111", "thickness_mm": 0.4, "roll_width_mm": 1000}}]}}}}"##
        )));
        assert_eq!(p["ok"], true, "{p}");
        let out = &p["output"];
        assert_eq!(out["piece_svgs"].as_array().unwrap().len(), 2);
        assert!(out["pdf"].as_str().unwrap().starts_with("%PDF"));
        assert_eq!(out["report"]["materials"][0]["sheet"], "black");

        let bigger = r#"{"figure": "women", "measurements": {"height": 1700, "bust": 1060}}"#;
        let r = parse(&handle(&format!(
            r#"{{"command": "redraft", "design": {design}, "body": {bigger}}}"#
        )));
        assert_eq!(r["design"]["block"]["body"]["measurements"]["bust"], 1060.0);
    }

    #[test]
    fn errors_are_reported_not_panicked() {
        let r = parse(&handle(
            r#"{"command": "pattern", "design": {"pieces": []}}"#,
        ));
        assert_eq!(r["ok"], false);
        assert!(r["error"].as_str().unwrap().contains("no pieces"));
        let r = parse(&handle(
            r#"{"command": "body", "body": {"measurements": {"inseam": 2000}}}"#,
        ));
        assert!(r["error"].as_str().unwrap().contains("inseam"));
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
