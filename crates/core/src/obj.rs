//! Minimal Wavefront OBJ reader.
//!
//! Only `v` and `f` records are read. Vertex indices are kept in **file
//! order** (0-based), so seam files can refer to the same numbering that
//! modelling tools such as Blender display. Generic OBJ loaders (e.g.
//! `tobj`) re-index vertices by first use, which would silently break seam
//! references, so we parse the two record types we need ourselves.
//!
//! Polygons with more than three corners are fan-triangulated. This
//! function takes `&str` and does no file I/O, so it works under WASM.

use crate::error::{Error, Result};
use crate::mesh::Mesh;
use glam::DVec3;

/// Parse OBJ text into a [`Mesh`].
pub fn parse_obj(text: &str) -> Result<Mesh> {
    let mut positions = Vec::new();
    let mut triangles = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = lineno + 1;
        let content = raw.split('#').next().unwrap_or("").trim();
        let mut it = content.split_whitespace();
        match it.next() {
            Some("v") => {
                let mut c = [0.0; 3];
                for slot in &mut c {
                    let tok = it.next().ok_or_else(|| err(line, "vertex needs 3 coordinates"))?;
                    *slot = tok.parse().map_err(|_| err(line, &format!("bad number '{tok}'")))?;
                }
                positions.push(DVec3::from_array(c));
            }
            Some("f") => {
                let mut idx = Vec::with_capacity(4);
                for tok in it {
                    let first = tok.split('/').next().unwrap_or("");
                    let i: i64 = first.parse().map_err(|_| err(line, &format!("bad index '{tok}'")))?;
                    let resolved = if i > 0 {
                        i - 1
                    } else if i < 0 {
                        positions.len() as i64 + i
                    } else {
                        return Err(err(line, "OBJ indices are 1-based; 0 is invalid"));
                    };
                    if resolved < 0 {
                        return Err(err(line, "relative index before first vertex"));
                    }
                    idx.push(resolved as u32);
                }
                if idx.len() < 3 {
                    return Err(err(line, "face needs at least 3 vertices"));
                }
                for k in 1..idx.len() - 1 {
                    triangles.push([idx[0], idx[k], idx[k + 1]]);
                }
            }
            _ => {}
        }
    }
    Mesh::new(positions, triangles)
}

/// Serialise a mesh as OBJ text (used to write the test meshes).
pub fn write_obj(mesh: &Mesh) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for p in mesh.positions() {
        let _ = writeln!(s, "v {} {} {}", p.x, p.y, p.z);
    }
    for t in mesh.triangles() {
        let _ = writeln!(s, "f {} {} {}", t[0] + 1, t[1] + 1, t[2] + 1);
    }
    s
}

fn err(line: usize, msg: &str) -> Error {
    Error::Obj { line, msg: msg.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quad_with_slashes_and_comments() {
        let m = parse_obj(
            "# a quad\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nf 1/1 2/1 3/1 -1/1 # tail\n",
        )
        .unwrap();
        assert_eq!(m.triangles(), &[[0, 1, 2], [0, 2, 3]]);
    }

    #[test]
    fn round_trip() {
        let m = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
        let m2 = parse_obj(&write_obj(&m)).unwrap();
        assert_eq!(m.positions(), m2.positions());
        assert_eq!(m.triangles(), m2.triangles());
    }

    #[test]
    fn reports_line() {
        assert_eq!(parse_obj("v 0 0\n").unwrap_err(), err(1, "vertex needs 3 coordinates"));
    }
}
