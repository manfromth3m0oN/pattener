//! Analytic test meshes with known flattening behaviour.
//!
//! All meshes are counter-clockwise with outward normals and use
//! millimetres. They back the unit tests and the files in `tests/data/`.

use crate::mesh::Mesh;
use glam::DVec3;

/// Flat `nx × ny` grid of square cells in the XY plane (normal +Z).
///
/// Vertex `(i, j)` has index `j * (nx + 1) + i`.
pub fn grid(nx: u32, ny: u32, cell: f64) -> Mesh {
    let mut p = Vec::new();
    for j in 0..=ny {
        for i in 0..=nx {
            p.push(DVec3::new(i as f64 * cell, j as f64 * cell, 0.0));
        }
    }
    let id = |i: u32, j: u32| j * (nx + 1) + i;
    let mut t = Vec::new();
    for j in 0..ny {
        for i in 0..nx {
            quad(
                &mut t,
                id(i, j),
                id(i + 1, j),
                id(i + 1, j + 1),
                id(i, j + 1),
            );
        }
    }
    Mesh::new(p, t).expect("grid is valid")
}

/// Cylinder of `radius` around the Z axis, `height` tall, covering `sweep`
/// radians (`TAU` for a closed tube).
///
/// `n_around` cells around, `n_up` cells up. Vertex `(i, j)` (column `i`,
/// ring `j`) has index `j * cols + i`, where `cols = n_around` for a closed
/// tube and `n_around + 1` otherwise. For a closed tube the seam
/// `[0, cols, 2*cols, …]` opens it into a rectangle.
pub fn cylinder(radius: f64, height: f64, n_around: u32, n_up: u32, sweep: f64) -> Mesh {
    let closed = (sweep - std::f64::consts::TAU).abs() < 1e-12;
    let cols = if closed { n_around } else { n_around + 1 };
    let mut p = Vec::new();
    for j in 0..=n_up {
        let z = height * j as f64 / n_up as f64;
        for i in 0..cols {
            let th = sweep * i as f64 / n_around as f64;
            p.push(DVec3::new(radius * th.cos(), radius * th.sin(), z));
        }
    }
    let id = |i: u32, j: u32| j * cols + (i % cols);
    let mut t = Vec::new();
    for j in 0..n_up {
        for i in 0..n_around {
            quad(
                &mut t,
                id(i, j),
                id(i + 1, j),
                id(i + 1, j + 1),
                id(i, j + 1),
            );
        }
    }
    Mesh::new(p, t).expect("cylinder is valid")
}

/// Spherical cap of sphere `radius`, centred on the +Z pole, extending to
/// polar angle `alpha` (radians).
///
/// Vertex 0 is the pole; ring `k` (1-based, polar angle `alpha * k / rings`)
/// has `segments` vertices starting at index `1 + (k - 1) * segments`.
pub fn spherical_cap(radius: f64, alpha: f64, rings: u32, segments: u32) -> Mesh {
    let mut p = vec![DVec3::new(0.0, 0.0, radius)];
    for k in 1..=rings {
        let phi = alpha * k as f64 / rings as f64;
        for i in 0..segments {
            let th = std::f64::consts::TAU * i as f64 / segments as f64;
            p.push(radius * DVec3::new(phi.sin() * th.cos(), phi.sin() * th.sin(), phi.cos()));
        }
    }
    let ring = |k: u32, i: u32| 1 + (k - 1) * segments + (i % segments);
    let mut t = Vec::new();
    for i in 0..segments {
        t.push([0, ring(1, i), ring(1, i + 1)]);
    }
    for k in 1..rings {
        for i in 0..segments {
            quad(
                &mut t,
                ring(k, i),
                ring(k + 1, i),
                ring(k + 1, i + 1),
                ring(k, i + 1),
            );
        }
    }
    Mesh::new(p, t).expect("cap is valid")
}

/// Split quad `a b c d` (counter-clockwise) into two triangles.
fn quad(t: &mut Vec<[u32; 3]>, a: u32, b: u32, c: u32, d: u32) {
    t.push([a, b, c]);
    t.push([a, c, d]);
}
