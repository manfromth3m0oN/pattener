//! Even re-triangulation of the garment area.
//!
//! Cutting curves into the body leaves triangles of very mixed sizes and
//! slivers where a curve passes close to a mesh vertex; flattened, those
//! show up as kinks and meaningless stretch peaks. This is the incremental
//! isotropic remeshing of Botsch and Kobbelt (*A remeshing approach to
//! multiresolution modeling*, 2004), restricted to the garment area and
//! with the curves as fixed features. Each round, for a target edge length
//! `L`:
//!
//! 1. split edges longer than `4/3 L` at their midpoint;
//! 2. collapse edges shorter than `4/5 L`, unless that would make an edge
//!    longer than `4/3 L` (along a curve, only between consecutive curve
//!    vertices, so curves keep their shape);
//! 3. flip edges that are not Delaunay (the angles opposite the edge sum
//!    to more than π), which removes slivers;
//! 4. move each free vertex towards the centroid of its neighbours within
//!    its tangent plane, and project it back onto the body surface.
//!
//! Curve vertices, vertices on the edge of the area and (for a symmetric
//! build) vertices on the centre line never move.

use super::edit::{closest_on_triangle, EditMesh};
use super::CurvePath;
use glam::DVec3;
use std::collections::{HashMap, HashSet};

/// The original body surface, for projecting moved vertices back onto it.
pub struct Surface<'a> {
    positions: &'a [DVec3],
    triangles: &'a [[u32; 3]],
    cell: f64,
    grid: HashMap<(i64, i64, i64), Vec<u32>>,
}

impl<'a> Surface<'a> {
    pub fn new(positions: &'a [DVec3], triangles: &'a [[u32; 3]], cell: f64) -> Self {
        let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
        let key = |x: f64| (x / cell).floor() as i64;
        for (f, t) in triangles.iter().enumerate() {
            let [a, b, c] = t.map(|i| positions[i as usize]);
            let (lo, hi) = (a.min(b).min(c), a.max(b).max(c));
            for i in key(lo.x)..=key(hi.x) {
                for j in key(lo.y)..=key(hi.y) {
                    for k in key(lo.z)..=key(hi.z) {
                        grid.entry((i, j, k)).or_default().push(f as u32);
                    }
                }
            }
        }
        Self {
            positions,
            triangles,
            cell,
            grid,
        }
    }

    /// The nearest point of the surface to `p` (searching the grid cells
    /// around `p`; `p` is expected to be near the surface).
    pub fn closest(&self, p: DVec3) -> DVec3 {
        let key = |x: f64| (x / self.cell).floor() as i64;
        let (ci, cj, ck) = (key(p.x), key(p.y), key(p.z));
        for r in 1..=3i64 {
            let mut best: Option<(f64, DVec3)> = None;
            for i in ci - r..=ci + r {
                for j in cj - r..=cj + r {
                    for k in ck - r..=ck + r {
                        for &f in self.grid.get(&(i, j, k)).into_iter().flatten() {
                            let [a, b, c] =
                                self.triangles[f as usize].map(|v| self.positions[v as usize]);
                            let (u, v) = closest_on_triangle(p, a, b, c);
                            let q = a + u * (b - a) + v * (c - a);
                            let d = q.distance_squared(p);
                            if best.is_none_or(|(bd, _)| d < bd) {
                                best = Some((d, q));
                            }
                        }
                    }
                }
            }
            if let Some((_, q)) = best {
                return q;
            }
        }
        p
    }
}

/// Re-triangulate the faces marked in `region` (extended as faces are
/// added) towards edge length `target`, keeping `paths` as features.
pub fn remesh(
    m: &mut EditMesh,
    region: &mut Vec<bool>,
    paths: &mut [CurvePath],
    surface: &Surface,
    target: f64,
    rounds: usize,
) {
    let (long, short) = (4.0 / 3.0 * target, 4.0 / 5.0 * target);
    for _ in 0..rounds {
        // 1. Split.
        for (u, w) in region_edges(m, region) {
            if !editable(m, region, u, w) || len(m, u, w) <= long {
                continue;
            }
            let c = m.split_edge(u, w, 0.5);
            region.resize(m.triangles.len(), true);
            insert_into_paths(paths, u, w, c);
            project(m, surface, c);
        }
        // 2. Collapse.
        for (x, y) in region_edges(m, region) {
            if m.faces_of(x).is_empty() || m.faces_of(y).is_empty() {
                continue;
            }
            if !editable(m, region, x, y) || len(m, x, y) >= short {
                continue;
            }
            let curves = curves_of(paths);
            let on = |v: u32| curves.get(&v).cloned().unwrap_or_default();
            let (cx, cy) = (on(x), on(y));
            let (a, b) = if cx.len() <= cy.len() { (x, y) } else { (y, x) };
            let (ca, cb) = if a == x { (&cx, &cy) } else { (&cy, &cx) };
            let along = ca
                .iter()
                .all(|&i| cb.contains(&i) && consecutive(&paths[i], a, b));
            let inside = m.faces_of(a).iter().all(|&f| region[f as usize]);
            let pb = m.positions[b as usize];
            let stays_short = m
                .neighbors(a)
                .iter()
                .all(|&n| m.positions[n as usize].distance(pb) < long);
            if along && inside && stays_short && m.collapse(a, b) {
                for c in paths.iter_mut() {
                    for v in c.vertices.iter_mut() {
                        if *v == a {
                            *v = b;
                        }
                    }
                    c.vertices.dedup();
                    if c.closed && c.vertices.len() > 1 && c.vertices.first() == c.vertices.last() {
                        c.vertices.pop();
                    }
                }
            }
        }
        // 3. Flip towards Delaunay.
        let features = feature_edges(paths);
        for (u, w) in region_edges(m, region) {
            if !editable(m, region, u, w)
                || features.contains(&(u.min(w), u.max(w)))
                || (m.pinned(u) && m.pinned(w))
            {
                continue;
            }
            let opposite: Vec<u32> = m
                .edge_faces(u, w)
                .iter()
                .map(|&f| {
                    *m.triangles[f as usize]
                        .iter()
                        .find(|&&v| v != u && v != w)
                        .expect("third vertex")
                })
                .collect();
            let angle = |c: u32| {
                let (pc, pu, pw) = (
                    m.positions[c as usize],
                    m.positions[u as usize],
                    m.positions[w as usize],
                );
                (pu - pc).angle_between(pw - pc)
            };
            if opposite.len() == 2
                && angle(opposite[0]) + angle(opposite[1]) > std::f64::consts::PI + 1e-9
            {
                m.flip(u, w);
            }
        }
        // 4. Relax free vertices and project them onto the body.
        let curves = curves_of(paths);
        let mut free: Vec<u32> = region_vertices(m, region)
            .into_iter()
            .filter(|&v| {
                !curves.contains_key(&v)
                    && !m.pinned(v)
                    && m.faces_of(v).iter().all(|&f| region[f as usize])
            })
            .collect();
        free.sort_unstable();
        for v in free {
            m.relax(v, 0.5);
            project(m, surface, v);
        }
    }
}

fn len(m: &EditMesh, u: u32, w: u32) -> f64 {
    m.positions[u as usize].distance(m.positions[w as usize])
}

/// An interior edge of the region: two faces, both in it.
fn editable(m: &EditMesh, region: &[bool], u: u32, w: u32) -> bool {
    let f = m.edge_faces(u, w);
    f.len() == 2 && f.iter().all(|&g| region[g as usize])
}

fn region_edges(m: &EditMesh, region: &[bool]) -> Vec<(u32, u32)> {
    let mut e = HashSet::new();
    for (f, t) in m.triangles.iter().enumerate() {
        if t[0] == u32::MAX || !region[f] {
            continue;
        }
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            e.insert((a.min(b), a.max(b)));
        }
    }
    let mut e: Vec<_> = e.into_iter().collect();
    e.sort_unstable();
    e
}

fn region_vertices(m: &EditMesh, region: &[bool]) -> Vec<u32> {
    let mut v: HashSet<u32> = HashSet::new();
    for (f, t) in m.triangles.iter().enumerate() {
        if t[0] != u32::MAX && region[f] {
            v.extend(t);
        }
    }
    v.into_iter().collect()
}

fn curves_of(paths: &[CurvePath]) -> HashMap<u32, Vec<usize>> {
    let mut c: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, p) in paths.iter().enumerate() {
        for &v in &p.vertices {
            let e = c.entry(v).or_default();
            if !e.contains(&i) {
                e.push(i);
            }
        }
    }
    c
}

fn feature_edges(paths: &[CurvePath]) -> HashSet<(u32, u32)> {
    let mut e = HashSet::new();
    for p in paths {
        let v = &p.vertices;
        for k in 0..v.len() {
            if k + 1 < v.len() || (p.closed && v.len() > 2) {
                let (a, b) = (v[k], v[(k + 1) % v.len()]);
                e.insert((a.min(b), a.max(b)));
            }
        }
    }
    e
}

fn consecutive(c: &CurvePath, a: u32, b: u32) -> bool {
    let v = &c.vertices;
    let n = v.len();
    (0..n).any(|i| {
        let j = (i + 1) % n;
        (j != 0 || c.closed) && ((v[i] == a && v[j] == b) || (v[i] == b && v[j] == a))
    })
}

fn insert_into_paths(paths: &mut [CurvePath], u: u32, w: u32, c: u32) {
    for p in paths.iter_mut() {
        let v = &mut p.vertices;
        let n = v.len();
        let mut i = 0;
        while i < v.len() {
            let j = (i + 1) % v.len();
            if (j != 0 || (p.closed && n > 2))
                && ((v[i] == u && v[j] == w) || (v[i] == w && v[j] == u))
            {
                v.insert(i + 1, c);
                i += 1;
            }
            i += 1;
        }
    }
}

/// Move `v` onto the body surface, unless (for a symmetric build) that
/// would take it across the centre line.
fn project(m: &mut EditMesh, surface: &Surface, v: u32) {
    if m.pinned(v) {
        return;
    }
    let p = m.positions[v as usize];
    let q = surface.closest(p);
    if m.keep_midline && (q.x <= 0.0) != (p.x <= 0.0) {
        return;
    }
    m.positions[v as usize] = q;
}

/// Re-triangulate the garment area of `m` evenly (see the module docs):
/// the faces reachable from `inside` without crossing an edge curve (and,
/// with `right_only`, without leaving the half `x ≥ 0`).
pub fn remesh_garment_area(
    m: &mut EditMesh,
    paths: &mut [CurvePath],
    inside: DVec3,
    body_positions: &[DVec3],
    body_triangles: &[[u32; 3]],
    right_only: bool,
) {
    let inside = if right_only && inside.x < 0.0 {
        DVec3::new(-inside.x, inside.y, inside.z)
    } else {
        inside
    };
    let live = |f: usize| m.triangles[f][0] != u32::MAX;
    let on_right = |f: usize| {
        !right_only
            || m.triangles[f]
                .iter()
                .all(|&v| m.positions[v as usize].x >= 0.0)
    };
    // Seed: the face whose centre is nearest the inside point.
    let centre = |f: usize| {
        m.triangles[f]
            .iter()
            .map(|&v| m.positions[v as usize])
            .sum::<DVec3>()
            / 3.0
    };
    let Some(seed) = (0..m.triangles.len())
        .filter(|&f| live(f) && on_right(f))
        .min_by(|&a, &b| {
            centre(a)
                .distance_squared(inside)
                .total_cmp(&centre(b).distance_squared(inside))
        })
    else {
        return;
    };
    let mut blocked: HashSet<(u32, u32)> = HashSet::new();
    for p in paths.iter().filter(|p| p.kind == super::CurveKind::Edge) {
        let v = &p.vertices;
        for k in 0..v.len() {
            if k + 1 < v.len() || (p.closed && v.len() > 2) {
                let (a, b) = (v[k], v[(k + 1) % v.len()]);
                blocked.insert((a.min(b), a.max(b)));
            }
        }
    }
    let mut region = vec![false; m.triangles.len()];
    region[seed] = true;
    let mut stack = vec![seed];
    while let Some(f) = stack.pop() {
        let t = m.triangles[f];
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if blocked.contains(&(a.min(b), a.max(b))) {
                continue;
            }
            for g in m.edge_faces(a, b) {
                let g = g as usize;
                if !region[g] && live(g) && on_right(g) {
                    region[g] = true;
                    stack.push(g);
                }
            }
        }
    }
    let n_live = (0..m.triangles.len()).filter(|&f| live(f)).count();
    let n_region = region.iter().filter(|&&r| r).count();
    if n_region * 2 > n_live {
        // Not enclosed (or nearly the whole body): leave it as it is.
        return;
    }
    // Target: the median edge length of the area, within 8–20 mm.
    let mut lengths: Vec<f64> = region_edges(m, &region)
        .iter()
        .map(|&(a, b)| len(m, a, b))
        .collect();
    if lengths.is_empty() {
        return;
    }
    lengths.sort_by(f64::total_cmp);
    let target = lengths[lengths.len() / 2].clamp(8.0, 20.0);
    let surface = Surface::new(body_positions, body_triangles, 2.0 * target);
    remesh(m, &mut region, paths, &surface, target, 5);
}
