//! Validation of the flattening pipeline against analytic cases.

use approx::assert_relative_eq;
use glam::DVec3;
use latexpat_core::{
    flatten_garment, testmesh, Ease, FlatPanel, FlattenOptions, Mesh, Seam, SeamSet,
};
use std::f64::consts::{PI, TAU};

const S: f64 = 0.85;

fn flatten(mesh: &Mesh, seams: Vec<Vec<u32>>) -> Vec<FlatPanel> {
    let set = SeamSet {
        seams: seams
            .into_iter()
            .map(|v| Seam {
                vertices: v,
                ..Default::default()
            })
            .collect(),
    };
    let opts = FlattenOptions {
        ease: Ease::Uniform(S),
        ..Default::default()
    };
    flatten_garment(mesh, &set, &opts).expect("flatten").panels
}

/// Property check shared by every test mesh: positive flat areas, no flips.
fn assert_no_flips(fp: &FlatPanel) {
    assert!(
        fp.strain.flipped.is_empty(),
        "flipped triangles: {:?}",
        fp.strain.flipped
    );
    for t in &fp.panel.triangles {
        let u = t.map(|i| fp.uv[i as usize]);
        assert!((u[1] - u[0]).perp_dot(u[2] - u[0]) > 0.0);
    }
}

#[test]
fn flat_patch_stretches_uniformly_by_inverse_ease() {
    let m = testmesh::grid(8, 6, 12.5);
    let fp = flatten(&m, vec![]);
    assert_eq!(fp.len(), 1);
    let fp = &fp[0];
    assert_no_flips(fp);
    for s in &fp.strain.triangles {
        assert_relative_eq!(s.min, 1.0 / S, epsilon = 1e-9);
        assert_relative_eq!(s.max, 1.0 / S, epsilon = 1e-9);
    }
    assert_relative_eq!(
        fp.strain.flat_area,
        S * S * fp.strain.body_area,
        max_relative = 1e-9
    );
}

#[test]
fn closed_cylinder_unrolls_to_rectangle() {
    // Tube r = 60 mm, h = 150 mm, opened by one vertical seam.
    let (r, h, n, up) = (60.0, 150.0, 48u32, 10u32);
    let m = testmesh::cylinder(r, h, n, up, TAU);
    let seam: Vec<u32> = (0..=up).map(|j| j * n).collect();
    let fp = flatten(&m, vec![seam]);
    assert_eq!(fp.len(), 1);
    let fp = &fp[0];
    assert_no_flips(fp);
    // A polyhedral cylinder of planar rectangles is exactly developable.
    for s in &fp.strain.triangles {
        assert_relative_eq!(s.min, 1.0 / S, epsilon = 1e-6);
        assert_relative_eq!(s.max, 1.0 / S, epsilon = 1e-6);
    }
    let width = n as f64 * 2.0 * r * (PI / n as f64).sin();
    assert_relative_eq!(fp.strain.flat_area, S * S * width * h, max_relative = 1e-6);
}

#[test]
fn open_cylinder_panel_unrolls() {
    let m = testmesh::cylinder(40.0, 80.0, 16, 6, PI / 2.0);
    let fp = flatten(&m, vec![]);
    assert_no_flips(&fp[0]);
    assert_relative_eq!(fp[0].strain.min_stretch, 1.0 / S, epsilon = 1e-6);
    assert_relative_eq!(fp[0].strain.max_stretch, 1.0 / S, epsilon = 1e-6);
}

/// Continuous axisymmetric ARAP solution for a spherical cap.
///
/// A radially symmetric flat map `r(φ)` of the cap has, relative to the
/// reference (`s ×` body), singular values `a = r′/(sR)` (radial) and
/// `b = r/(sR sin φ)` (circumferential), and no rotation, so the ARAP
/// energy `∫ [(a − 1)² + (b − 1)²] sin φ dφ` is quadratic in `r`. We
/// discretise `ρ = r/(sR)` on a fine 1D grid with `ρ(0) = 0` and solve the
/// tridiagonal normal equations. Returns `(φ_mid, λ_radial, λ_circ)` per
/// interval, where `λ = 1/(s·a)` etc. is the body/flat stretch.
fn cap_reference(alpha: f64, n: usize) -> Vec<(f64, f64, f64)> {
    let h = alpha / n as f64;
    // Unknowns ρ_1..ρ_n (ρ_0 = 0). Tridiagonal: lower, diag, upper, rhs.
    let mut lo = vec![0.0; n];
    let mut di = vec![0.0; n];
    let mut up = vec![0.0; n];
    let mut rhs = vec![0.0; n];
    for i in 0..n {
        let m = (i as f64 + 0.5) * h;
        let w = m.sin() * h;
        // Residuals: c0·ρ_i + c1·ρ_{i+1} − 1.
        for (c0, c1) in [(-1.0 / h, 1.0 / h), (0.5 / m.sin(), 0.5 / m.sin())] {
            // ρ_i is unknown index i-1 (absent for i = 0); ρ_{i+1} is index i.
            di[i] += w * c1 * c1;
            rhs[i] += w * c1;
            if i > 0 {
                di[i - 1] += w * c0 * c0;
                rhs[i - 1] += w * c0;
                up[i - 1] += w * c0 * c1;
                lo[i] += w * c0 * c1;
            }
        }
    }
    // Thomas algorithm.
    for i in 1..n {
        let f = lo[i] / di[i - 1];
        di[i] -= f * up[i - 1];
        rhs[i] -= f * rhs[i - 1];
    }
    let mut rho = vec![0.0; n + 1];
    rho[n] = rhs[n - 1] / di[n - 1];
    for i in (1..n).rev() {
        rho[i] = (rhs[i - 1] - up[i - 1] * rho[i + 1]) / di[i - 1];
    }
    (0..n)
        .map(|i| {
            let m = (i as f64 + 0.5) * h;
            let a = (rho[i + 1] - rho[i]) / h;
            let b = 0.5 * (rho[i] + rho[i + 1]) / m.sin();
            (m, 1.0 / (S * a), 1.0 / (S * b))
        })
        .collect()
}

#[test]
fn spherical_cap_matches_axisymmetric_reference() {
    let (radius, alpha, rings, segs) = (100.0, PI / 3.0, 20u32, 64u32);
    let m = testmesh::spherical_cap(radius, alpha, rings, segs);
    let fp = flatten(&m, vec![]);
    assert_eq!(fp.len(), 1);
    let fp = &fp[0];
    assert_no_flips(fp);

    let reference = cap_reference(alpha, 4000);
    let h = alpha / reference.len() as f64;
    let mut worst = 0.0f64;
    for (t, tri) in fp.panel.triangles.iter().enumerate() {
        let c: DVec3 = tri
            .iter()
            .map(|&i| fp.panel.positions[i as usize])
            .sum::<DVec3>()
            / 3.0;
        let phi = (c.z / c.length()).acos();
        if phi < 3.0 * alpha / rings as f64 {
            continue; // slender triangles near the pole
        }
        let k = ((phi / h) as usize).min(reference.len() - 1);
        let (_, lr, lc) = reference[k];
        let (emin, emax) = (lr.min(lc), lr.max(lc));
        let s = fp.strain.triangles[t];
        worst = worst
            .max(((s.min - emin) / emin).abs())
            .max(((s.max - emax) / emax).abs());
    }
    assert!(
        worst < 0.015,
        "max relative deviation from reference stretch: {worst}"
    );

    // On a sphere the circumference grows slower than on a plane, so the
    // rim is circumferentially slack relative to the centre and the overall
    // stretch straddles 1/s.
    assert!(fp.strain.min_stretch < 1.0 / S && fp.strain.max_stretch > 1.0 / S);
    assert_relative_eq!(
        fp.strain.mean_area_stretch,
        1.0 / (S * S),
        max_relative = 0.05
    );
}

#[test]
fn spherical_cap_is_rotationally_symmetric() {
    let (rings, segs) = (12u32, 48u32);
    let m = testmesh::spherical_cap(80.0, PI / 4.0, rings, segs);
    let fp = &flatten(&m, vec![])[0];
    // Triangles are generated ring by ring: `segs` pole triangles, then
    // 2·segs per ring band. Within each band, stretches should agree.
    for band in 1..rings as usize {
        let start = segs as usize + (band - 1) * 2 * segs as usize;
        let tris = &fp.strain.triangles[start..start + 2 * segs as usize];
        for parity in 0..2 {
            let vals: Vec<f64> = tris.iter().skip(parity).step_by(2).map(|s| s.max).collect();
            let (lo, hi) = vals
                .iter()
                .fold((f64::INFINITY, 0.0f64), |(l, h), &v| (l.min(v), h.max(v)));
            assert!((hi - lo) / lo < 1e-3, "band {band}: spread {lo}..{hi}");
        }
    }
}
