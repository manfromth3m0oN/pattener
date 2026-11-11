//! Garments drawn on the body.
//!
//! A [`GarmentDesign`] is a set of curves on the body surface plus a point
//! inside the garment. **Edge** curves (necklines, armholes, hems, leg
//! openings) bound the garment; **seam** curves divide it into panels.
//! Curve points are stored as [`SurfacePoint`]s (a triangle and barycentric
//! weights) of the body model, whose triangles are the same for every body
//! shape, so a design re-fits when the measurements change.
//!
//! [`build`] turns a design on a body into a garment mesh with seams,
//! ready for [`job::run`](crate::job::run):
//!
//! 1. **Mirror** curves marked `mirror`: each point `p` maps to the
//!    surface point nearest `(−p.x, p.y, p.z)`.
//! 2. **Smooth** each curve: a centripetal Catmull–Rom spline through its
//!    points (closed or open), sampled about every [`SAMPLE_MM`] and placed
//!    back on the skin (nearest surface point). Curves pass smoothly
//!    through the points you click instead of turning a corner at each;
//!    corners would pinch when the piece is flattened.
//! 3. **Embed** curves into the mesh, edges first then seams. Each sample
//!    becomes a vertex; seam ends within [`SNAP_MM`] of an earlier
//!    curve snap onto it, so seams meet edges and each other. Between two
//!    vertices `A`, `B` the curve follows the body's cross-section in the
//!    plane through `A` and `B` that contains their mean surface normal
//!    `n̄` (plane normal `(B − A) × n̄`): a "straight line drawn on the
//!    skin". It is cut in by walking from `A`: around the current vertex
//!    `V`, the plane crosses the opposite edges of the surrounding
//!    triangles in two places (ahead and behind); take the one that best
//!    continues the walking direction (initially towards `B`), split the
//!    edge there, and repeat until `B` is adjacent. If the crossing is
//!    within 20 % of a corner, the corner is moved onto the plane (along
//!    the plane normal, which is tangent to the skin) and the curve steps
//!    onto it: cutting that close to a corner would leave sliver triangles
//!    that distort when flattened. The move is a few millimetres at most. Following the direction rather
//!    than distance along `B − A` lets the curve pass through hollows,
//!    where it briefly turns back. Where the plane does not join `A` and
//!    `B` at all (it can cut torso and arm as separate loops at the
//!    armpit), the span falls back to the shortest path along mesh edges
//!    (Dijkstra).
//! 4. **Clean up**: collapse edges shorter than 1.5 mm (removing tiny
//!    back-and-forth cusps where samples meet), where the mesh stays
//!    manifold and no face flips, never merging two different curves; then
//!    relax the vertices next to curves (move them towards their
//!    neighbours' centroid, within the tangent plane) to even out sliver
//!    triangles. Curve vertices themselves do not move.
//! 5. **Region**: flood-fill triangles from the inside point without
//!    crossing an edge curve.
//! 6. **Extract** the region as its own mesh; seam curves become vertex
//!    polylines in it (parts outside the garment are dropped).

pub mod edit;

use crate::error::{Error, Result};
use crate::mesh::Mesh;
use crate::seams::{Seam, SeamSet};
use edit::{locate, EditMesh};
use glam::DVec3;
use std::collections::{HashMap, HashSet};

pub use edit::SurfacePoint;

/// Seam ends this close (mm) to an earlier curve snap onto it.
pub const SNAP_MM: f64 = 20.0;
/// Spacing (mm) of the samples along a smoothed curve.
pub const SAMPLE_MM: f64 = 15.0;
/// Edges shorter than this (mm) are collapsed after embedding.
const MIN_EDGE_MM: f64 = 1.5;
const RELAX_ROUNDS: usize = 4;
/// Barycentric distance below which a point snaps to a corner or edge.
const POINT_SNAP: f64 = 0.15;
/// Edge crossings this close (fraction of the edge) to a corner move the
/// corner onto the curve instead of cutting a sliver next to it.
const CROSS_SNAP: f64 = 0.2;
const MAX_WALK: usize = 20_000;

/// What a curve does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum CurveKind {
    /// A garment edge: neckline, armhole, hem, leg opening…
    #[default]
    Edge,
    /// A seam between panels.
    Seam,
}

/// One curve drawn on the body.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Curve {
    pub name: String,
    pub kind: CurveKind,
    pub points: Vec<SurfacePoint>,
    /// Join the last point back to the first.
    pub closed: bool,
    /// Also draw the mirror image on the other side of the body.
    pub mirror: bool,
}

/// A garment: curves and a point inside it.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct GarmentDesign {
    pub curves: Vec<Curve>,
    /// A point on the body inside the garment.
    pub inside: Option<SurfacePoint>,
}

/// One embedded curve.
#[derive(Debug, Clone, PartialEq)]
pub struct CurvePath {
    pub name: String,
    pub kind: CurveKind,
    pub closed: bool,
    /// Vertex indices into [`BuiltGarment::body`], in order.
    pub vertices: Vec<u32>,
}

/// Result of [`build`].
#[derive(Debug, Clone)]
pub struct BuiltGarment {
    /// The body with all curves cut in.
    pub body: Mesh,
    pub curves: Vec<CurvePath>,
    /// Per triangle of `body`: part of the garment?
    pub region: Vec<bool>,
    /// The garment alone, if a region was chosen.
    pub mesh: Option<Mesh>,
    /// Seams in `mesh` vertex indices.
    pub seams: SeamSet,
    pub warnings: Vec<String>,
}

/// Mirror a surface point across the body's midplane `x = 0`.
pub fn mirror_point(
    positions: &[DVec3],
    triangles: &[[u32; 3]],
    sp: SurfacePoint,
) -> Option<SurfacePoint> {
    let p = sp.position(positions, triangles)?;
    Some(locate(positions, triangles, DVec3::new(-p.x, p.y, p.z)))
}

/// Embed a design into `body` (which must have the body model's triangles).
pub fn build(body: &Mesh, design: &GarmentDesign) -> Result<BuiltGarment> {
    let (pos0, tri0) = (body.positions(), body.triangles());
    let mut warnings = Vec::new();

    // 1. Curves with mirrors, edges before seams.
    let mut curves: Vec<Curve> = Vec::new();
    for c in &design.curves {
        if c.points.len() < 2 {
            warnings.push(format!("curve '{}' needs at least two points", c.name));
            continue;
        }
        let bad = c.points.iter().any(|p| p.face as usize >= tri0.len());
        if bad {
            return Err(Error::Garment(format!(
                "curve '{}' has a point off the body",
                c.name
            )));
        }
        curves.push(c.clone());
        if c.mirror {
            let points = c
                .points
                .iter()
                .map(|&p| mirror_point(pos0, tri0, p).expect("checked"))
                .collect();
            curves.push(Curve {
                name: format!("{} (mirror)", c.name),
                points,
                mirror: false,
                ..c.clone()
            });
        }
    }
    curves.sort_by_key(|c| c.kind == CurveKind::Seam);

    // 2. Embed.
    let mut m = EditMesh::new(pos0.to_vec(), tri0.to_vec());
    let mut paths: Vec<CurvePath> = Vec::new();
    for c in &curves {
        let control: Vec<DVec3> = c
            .points
            .iter()
            .map(|sp| sp.position(pos0, tri0).expect("checked"))
            .collect();
        let samples = smooth(&control, c.closed, SAMPLE_MM);
        let n = samples.len();
        let mut verts = Vec::with_capacity(n);
        for (k, &p) in samples.iter().enumerate() {
            let end = !c.closed && (k == 0 || k == n - 1);
            let snapped = if end && c.kind == CurveKind::Seam {
                nearest_curve_vertex(&m, &paths, p, SNAP_MM)
            } else {
                None
            };
            let v = match snapped {
                Some(v) => v,
                None => {
                    let here = locate(&m.positions, &m.triangles, p);
                    m.insert_point(here, POINT_SNAP)
                }
            };
            if verts.last() != Some(&v) {
                verts.push(v);
            }
        }
        if c.closed && verts.len() > 1 && verts.first() == verts.last() {
            verts.pop();
        }
        let mut path = vec![verts[0]];
        let segments = if c.closed {
            verts.len()
        } else {
            verts.len() - 1
        };
        for s in 0..segments {
            let (a, b) = (verts[s], verts[(s + 1) % verts.len()]);
            if a == b {
                continue;
            }
            let start = path.len();
            if walk(&mut m, &mut paths, &mut path, a, b).is_err() {
                // The plane through the two points does not join them on
                // the surface (e.g. through the armpit): fall back to the
                // shortest route along mesh edges.
                path.truncate(start);
                shortest_edge_path(&m, a, b, &mut path).ok_or_else(|| {
                    Error::Garment(format!("curve '{}' crosses a gap in the body", c.name))
                })?;
            }
        }
        if c.closed && path.len() > 1 && path.first() == path.last() {
            path.pop();
        }
        paths.push(CurvePath {
            name: c.name.clone(),
            kind: c.kind,
            closed: c.closed,
            vertices: path,
        });
    }

    cleanup(&mut m, &mut paths);
    let body_mesh = Mesh::new(m.positions.clone(), m.live_triangles())
        .map_err(|e| Error::Garment(format!("cutting the curves into the body failed: {e}")))?;

    // 3. Region.
    let mut blocked: HashSet<(u32, u32)> = HashSet::new();
    for p in paths.iter().filter(|p| p.kind == CurveKind::Edge) {
        for (a, b) in path_edges(p) {
            blocked.insert((a.min(b), a.max(b)));
        }
    }
    let mut region = vec![false; body_mesh.triangles().len()];
    let mut mesh = None;
    let mut seams = SeamSet::default();
    if let Some(inside) = design.inside {
        let p = inside
            .position(pos0, tri0)
            .ok_or_else(|| Error::Garment("the inside point is off the body".into()))?;
        let seed = locate(body_mesh.positions(), body_mesh.triangles(), p).face;
        let mut stack = vec![seed];
        region[seed as usize] = true;
        while let Some(f) = stack.pop() {
            for e in body_mesh.face_edges(f) {
                let edge = body_mesh.edges()[e as usize];
                if blocked.contains(&(edge.v[0], edge.v[1])) {
                    continue;
                }
                if let Some(g) = edge.other_face(f) {
                    if !region[g as usize] {
                        region[g as usize] = true;
                        stack.push(g);
                    }
                }
            }
        }
        let count = region.iter().filter(|&&r| r).count();
        if count == region.len() {
            warnings.push(
                "the garment covers the whole body: its edge curves do not enclose an area yet \
                 (close the loops, or end edge curves on other edge curves)"
                    .into(),
            );
        }

        // 4. Extract.
        let mut local = HashMap::new();
        let mut positions = Vec::new();
        let mut triangles = Vec::new();
        for (f, t) in body_mesh.triangles().iter().enumerate() {
            if region[f] {
                triangles.push(t.map(|v| {
                    *local.entry(v).or_insert_with(|| {
                        positions.push(body_mesh.positions()[v as usize]);
                        (positions.len() - 1) as u32
                    })
                }));
            }
        }
        let gm = Mesh::new(positions, triangles)?;
        for p in paths.iter().filter(|p| p.kind == CurveKind::Seam) {
            let mut runs: Vec<Vec<u32>> = Vec::new();
            let mut cur: Vec<u32> = Vec::new();
            for (a, b) in path_edges(p) {
                let (la, lb) = (local.get(&a), local.get(&b));
                match (la, lb) {
                    (Some(&la), Some(&lb)) if gm.find_edge(la, lb).is_some() => {
                        if cur.is_empty() {
                            cur.push(la);
                        }
                        cur.push(lb);
                    }
                    _ => {
                        if cur.len() > 1 {
                            runs.push(std::mem::take(&mut cur));
                        }
                        cur.clear();
                    }
                }
            }
            if cur.len() > 1 {
                runs.push(cur);
            }
            if runs.is_empty() {
                warnings.push(format!("seam '{}' is outside the garment", p.name));
            }
            let several = runs.len() > 1;
            for (i, r) in runs.into_iter().enumerate() {
                seams.seams.push(Seam {
                    name: Some(if several {
                        format!("{} {}", p.name, i + 1)
                    } else {
                        p.name.clone()
                    }),
                    vertices: r,
                    ..Default::default()
                });
            }
        }
        mesh = Some(gm);
    } else if !paths.is_empty() {
        warnings.push("click inside the garment to choose its area".into());
    }

    Ok(BuiltGarment {
        body: body_mesh,
        curves: paths,
        region,
        mesh,
        seams,
        warnings,
    })
}

/// Collapse very short edges and relax the triangles around curves.
fn cleanup(m: &mut EditMesh, paths: &mut [CurvePath]) {
    // Which curves use each vertex.
    let on_curves = |paths: &[CurvePath], v: u32| -> Vec<usize> {
        paths
            .iter()
            .enumerate()
            .filter(|(_, c)| c.vertices.contains(&v))
            .map(|(i, _)| i)
            .collect()
    };
    let consecutive = |c: &CurvePath, a: u32, b: u32| {
        let v = &c.vertices;
        let n = v.len();
        (0..n).any(|i| {
            let j = (i + 1) % n;
            (j != 0 || c.closed) && ((v[i] == a && v[j] == b) || (v[i] == b && v[j] == a))
        })
    };
    for _ in 0..3 {
        let mut changed = false;
        for f in 0..m.triangles.len() {
            let t = m.triangles[f];
            if t[0] == u32::MAX {
                continue;
            }
            for k in 0..3 {
                let (x, y) = (t[k], t[(k + 1) % 3]);
                if m.positions[x as usize].distance(m.positions[y as usize]) >= MIN_EDGE_MM {
                    continue;
                }
                let (cx, cy) = (on_curves(paths, x), on_curves(paths, y));
                // Remove the vertex that is on fewer curves; only merge two
                // curve vertices along a curve they share.
                let (a, b) = if cx.len() <= cy.len() { (x, y) } else { (y, x) };
                let ca = if a == x { &cx } else { &cy };
                let cb = if a == x { &cy } else { &cx };
                let allowed = ca
                    .iter()
                    .all(|&i| cb.contains(&i) && consecutive(&paths[i], a, b));
                if allowed && m.collapse(a, b) {
                    for c in paths.iter_mut() {
                        for v in c.vertices.iter_mut() {
                            if *v == a {
                                *v = b;
                            }
                        }
                        c.vertices.dedup();
                        if c.closed
                            && c.vertices.len() > 1
                            && c.vertices.first() == c.vertices.last()
                        {
                            c.vertices.pop();
                        }
                    }
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }
    // Relax the two rings of free vertices around the curves.
    let fixed: HashSet<u32> = paths
        .iter()
        .flat_map(|c| c.vertices.iter().copied())
        .collect();
    let mut ring: HashSet<u32> = HashSet::new();
    for &v in &fixed {
        for n in m.neighbors(v) {
            if !fixed.contains(&n) {
                ring.insert(n);
                for n2 in m.neighbors(n) {
                    if !fixed.contains(&n2) {
                        ring.insert(n2);
                    }
                }
            }
        }
    }
    let mut ring: Vec<u32> = ring.into_iter().collect();
    ring.sort_unstable();
    for _ in 0..RELAX_ROUNDS {
        for &v in &ring {
            m.relax(v, 0.5);
        }
    }
}

/// Centripetal Catmull–Rom spline through `points` (wrapping if
/// `closed`), sampled about every `spacing`, keeping the given points.
/// Samples are not yet on the skin; callers place them.
pub fn smooth(points: &[DVec3], closed: bool, spacing: f64) -> Vec<DVec3> {
    let n = points.len();
    if n < 2 {
        return points.to_vec();
    }
    let get = |i: isize| -> DVec3 {
        if closed {
            points[i.rem_euclid(n as isize) as usize]
        } else if i < 0 {
            2.0 * points[0] - points[1]
        } else if i as usize >= n {
            2.0 * points[n - 1] - points[n - 2]
        } else {
            points[i as usize]
        }
    };
    let segments = if closed { n } else { n - 1 };
    let mut out = Vec::new();
    for s in 0..segments as isize {
        let (p0, p1, p2, p3) = (get(s - 1), get(s), get(s + 1), get(s + 2));
        // Centripetal parametrisation: t_{i+1} = t_i + |P_{i+1} − P_i|^½.
        let knot = |a: DVec3, b: DVec3| a.distance(b).sqrt().max(1e-6);
        let t0 = 0.0;
        let t1 = t0 + knot(p0, p1);
        let t2 = t1 + knot(p1, p2);
        let t3 = t2 + knot(p2, p3);
        let steps = (p1.distance(p2) / spacing).ceil().max(1.0) as usize;
        out.push(p1);
        for k in 1..steps {
            let t = t1 + (t2 - t1) * k as f64 / steps as f64;
            // Barry–Goldman pyramidal evaluation.
            let lerp = |a: DVec3, b: DVec3, ta: f64, tb: f64| {
                a * ((tb - t) / (tb - ta)) + b * ((t - ta) / (tb - ta))
            };
            let a1 = lerp(p0, p1, t0, t1);
            let a2 = lerp(p1, p2, t1, t2);
            let a3 = lerp(p2, p3, t2, t3);
            let b1 = lerp(a1, a2, t0, t2);
            let b2 = lerp(a2, a3, t1, t3);
            out.push(lerp(b1, b2, t1, t2));
        }
    }
    if !closed {
        out.push(points[n - 1]);
    }
    out
}

/// Consecutive vertex pairs of a path (including the closing pair).
fn path_edges(p: &CurvePath) -> Vec<(u32, u32)> {
    let v = &p.vertices;
    let mut e: Vec<(u32, u32)> = v.windows(2).map(|w| (w[0], w[1])).collect();
    if p.closed && v.len() > 2 {
        e.push((v[v.len() - 1], v[0]));
    }
    e
}

fn nearest_curve_vertex(m: &EditMesh, paths: &[CurvePath], p: DVec3, within: f64) -> Option<u32> {
    paths
        .iter()
        .flat_map(|c| c.vertices.iter().copied())
        .map(|v| (v, m.positions[v as usize].distance(p)))
        .filter(|&(_, d)| d <= within)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(v, _)| v)
}

/// Insert vertex `c` between `u` and `w` wherever an earlier path uses edge
/// `(u, w)`.
fn update_paths(paths: &mut [CurvePath], current: &mut Vec<u32>, u: u32, w: u32, c: u32) {
    let fix = |v: &mut Vec<u32>, closed: bool| {
        let n = v.len();
        let mut i = 0;
        while i < v.len() {
            let j = (i + 1) % v.len();
            if (j != 0 || (closed && n > 2))
                && ((v[i] == u && v[j] == w) || (v[i] == w && v[j] == u))
            {
                v.insert(i + 1, c);
                i += 1;
            }
            i += 1;
        }
    };
    for p in paths.iter_mut() {
        fix(&mut p.vertices, p.closed);
    }
    fix(current, false);
}

/// Cut the plane section from vertex `a` to vertex `b` into the mesh,
/// appending the vertices after `a` to `path`.
fn walk(
    m: &mut EditMesh,
    paths: &mut [CurvePath],
    path: &mut Vec<u32>,
    a: u32,
    b: u32,
) -> std::result::Result<(), String> {
    let (pa, pb) = (m.positions[a as usize], m.positions[b as usize]);
    let chord = pb - pa;
    let len2 = chord.length_squared();
    if len2 < 1e-12 {
        return Ok(());
    }
    let nbar = m.normal(a) + m.normal(b);
    let normal = chord.cross(nbar);
    if nbar.length() < 0.3 || normal.length() < 1e-9 * chord.length() {
        return Err(
            "the points are on opposite sides of the body; add a point between them".into(),
        );
    }
    let normal = normal.normalize();
    let d = |p: DVec3| (p - pa).dot(normal);

    let mut v = a;
    let mut dir = chord.normalize();
    let mut visited: HashSet<u32> = HashSet::from([a]);
    let mut travelled = 0.0;
    let limit = 4.0 * chord.length() + 200.0;
    for _ in 0..MAX_WALK {
        if v == b {
            return Ok(());
        }
        if m.adjacent(v, b) {
            path.push(b);
            return Ok(());
        }
        let pv = m.positions[v as usize];
        // Best continuation: (alignment with `dir`, step).
        let mut best: Option<(f64, Step, DVec3)> = None;
        for &f in m.faces_of(v) {
            let t = m.triangles[f as usize];
            let k = t.iter().position(|&x| x == v).expect("vertex in face");
            let (u, w) = (t[(k + 1) % 3], t[(k + 2) % 3]);
            let (pu, pw) = (m.positions[u as usize], m.positions[w as usize]);
            let (du, dw) = (d(pu), d(pw));
            if (du >= 0.0) == (dw >= 0.0) && du != 0.0 && dw != 0.0 {
                continue;
            }
            let t = if du == dw { 0.0 } else { du / (du - dw) };
            let step = if t <= CROSS_SNAP {
                Step::Vertex(u)
            } else if t >= 1.0 - CROSS_SNAP {
                Step::Vertex(w)
            } else {
                Step::Split(u, w, t)
            };
            let p = match step {
                Step::Vertex(x) if visited.contains(&x) => continue,
                Step::Vertex(x) => {
                    let q = m.positions[x as usize];
                    q - d(q) * normal
                }
                Step::Split(..) => pu.lerp(pw, t),
            };
            let q = (p - pv).normalize_or_zero();
            let score = q.dot(dir);
            if score > -0.3 && best.as_ref().is_none_or(|(bs, ..)| score > *bs) {
                best = Some((score, step, p));
            }
        }
        let Some((_, step, p)) = best else {
            return Err(
                "the curve could not follow the body here; add a point between them".into(),
            );
        };
        travelled += p.distance(pv);
        if travelled > limit {
            return Err("the curve went the long way round; add a point between them".into());
        }
        // Smooth the heading so one off-plane corner snap does not turn it.
        dir = (dir + (p - pv).normalize_or_zero()).normalize_or(dir);
        v = match step {
            Step::Vertex(x) => {
                // Move the corner onto the curve's plane (unless it already
                // carries a curve, which must stay where it is).
                if !paths.iter().any(|c| c.vertices.contains(&x)) && !path.contains(&x) {
                    m.positions[x as usize] = p;
                }
                x
            }
            Step::Split(u, w, t) => {
                let c = m.split_edge(u, w, t);
                update_paths(paths, path, u, w, c);
                c
            }
        };
        visited.insert(v);
        if path.last() != Some(&v) {
            path.push(v);
        }
    }
    Err("the curve is too long to follow; add points along it".into())
}

/// Dijkstra along mesh edges from `a` to `b`; appends the vertices after
/// `a` to `path`.
fn shortest_edge_path(m: &EditMesh, a: u32, b: u32, path: &mut Vec<u32>) -> Option<()> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    #[derive(PartialEq)]
    struct Key(f64);
    impl Eq for Key {}
    impl PartialOrd for Key {
        fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for Key {
        fn cmp(&self, o: &Self) -> std::cmp::Ordering {
            self.0.total_cmp(&o.0)
        }
    }
    let n = m.positions.len();
    let mut dist = vec![f64::INFINITY; n];
    let mut prev = vec![u32::MAX; n];
    let mut heap = BinaryHeap::new();
    dist[a as usize] = 0.0;
    heap.push(Reverse((Key(0.0), a)));
    while let Some(Reverse((Key(d), v))) = heap.pop() {
        if v == b {
            break;
        }
        if d > dist[v as usize] {
            continue;
        }
        for &f in m.faces_of(v) {
            for w in m.triangles[f as usize] {
                let nd = d + m.positions[v as usize].distance(m.positions[w as usize]);
                if nd < dist[w as usize] {
                    dist[w as usize] = nd;
                    prev[w as usize] = v;
                    heap.push(Reverse((Key(nd), w)));
                }
            }
        }
    }
    if !dist[b as usize].is_finite() {
        return None;
    }
    let mut back = vec![b];
    let mut v = b;
    while prev[v as usize] != a {
        v = prev[v as usize];
        back.push(v);
    }
    path.extend(back.into_iter().rev());
    Some(())
}

enum Step {
    Vertex(u32),
    Split(u32, u32, f64),
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::human::{fit, test_model, MeasureKind, Measurements, Measurer, ShapeOptions};
    use crate::job::{run, JobOptions};

    /// The default body and its tape loops.
    pub(crate) fn body() -> (Mesh, Vec<(MeasureKind, Vec<DVec3>)>) {
        let m = test_model();
        let me = Measurer::new(&m).unwrap();
        let b = fit(&m, &me, &Measurements::new(), &ShapeOptions::default()).unwrap();
        (b.mesh, b.tapes)
    }

    fn tape(tapes: &[(MeasureKind, Vec<DVec3>)], k: MeasureKind) -> &[DVec3] {
        &tapes.iter().find(|t| t.0 == k).unwrap().1
    }

    /// `n` evenly spaced points of a closed loop, as surface points.
    fn ring(mesh: &Mesh, loop_: &[DVec3], n: usize) -> Vec<SurfacePoint> {
        (0..n)
            .map(|i| {
                locate(
                    mesh.positions(),
                    mesh.triangles(),
                    loop_[i * loop_.len() / n],
                )
            })
            .collect()
    }

    fn extreme(loop_: &[DVec3], key: impl Fn(&DVec3) -> f64) -> DVec3 {
        *loop_
            .iter()
            .max_by(|a, b| key(a).total_cmp(&key(b)))
            .unwrap()
    }

    /// A short-sleeved top: hem at the hips, neckline, sleeve openings
    /// around the upper arms, side seams from hem to sleeve and shoulder
    /// seams from neckline to sleeve.
    pub(crate) fn top(mesh: &Mesh, tapes: &[(MeasureKind, Vec<DVec3>)]) -> GarmentDesign {
        use MeasureKind::*;
        let at = |p: DVec3| locate(mesh.positions(), mesh.triangles(), p);
        let hips = tape(tapes, Hips);
        let side = |k| extreme(tape(tapes, k), |p| p.x);
        let arm_bottom = extreme(tape(tapes, UpperArm), |p| -p.y);
        let arm_top = extreme(tape(tapes, UpperArm), |p| p.y);
        let neck_side = extreme(tape(tapes, Neck), |p| p.x);
        let curve = |name: &str, kind, points, closed, mirror| Curve {
            name: name.into(),
            kind,
            points,
            closed,
            mirror,
        };
        GarmentDesign {
            curves: vec![
                curve("hem", CurveKind::Edge, ring(mesh, hips, 12), true, false),
                curve(
                    "neckline",
                    CurveKind::Edge,
                    ring(mesh, tape(tapes, Neck), 10),
                    true,
                    false,
                ),
                curve(
                    "sleeve",
                    CurveKind::Edge,
                    ring(mesh, tape(tapes, UpperArm), 8),
                    true,
                    true,
                ),
                curve(
                    "side",
                    CurveKind::Seam,
                    vec![
                        at(side(Hips)),
                        at(side(Waist)),
                        at(side(Underbust)),
                        at(side(Bust)),
                        at(arm_bottom),
                    ],
                    false,
                    true,
                ),
                curve(
                    "shoulder",
                    CurveKind::Seam,
                    vec![at(neck_side), at(arm_top)],
                    false,
                    true,
                ),
            ],
            inside: Some(at(extreme(tape(tapes, Waist), |p| p.z))),
        }
    }

    #[test]
    fn spline_passes_through_points() {
        let pts = [
            DVec3::ZERO,
            DVec3::new(100.0, 0.0, 0.0),
            DVec3::new(100.0, 100.0, 0.0),
        ];
        let open = smooth(&pts, false, 10.0);
        for p in pts {
            assert!(open.iter().any(|q| q.distance(p) < 1e-9));
        }
        assert_eq!(open.first(), Some(&pts[0]));
        assert_eq!(open.last(), Some(&pts[2]));
        assert!(open.windows(2).all(|w| w[0].distance(w[1]) < 12.0));
        // The corner is rounded: samples near (100, 0) bulge past the straight path.
        assert!(open.iter().any(|q| q.x > 100.5 || q.y < -0.5));
        let closed = smooth(&pts, true, 10.0);
        assert!(closed.len() > open.len());
    }

    #[test]
    fn mirror_is_symmetric() {
        let (mesh, tapes) = body();
        let p = locate(
            mesh.positions(),
            mesh.triangles(),
            extreme(tape(&tapes, MeasureKind::Bust), |p| p.x),
        );
        let q = mirror_point(mesh.positions(), mesh.triangles(), p).unwrap();
        let (a, b) = (
            p.position(mesh.positions(), mesh.triangles()).unwrap(),
            q.position(mesh.positions(), mesh.triangles()).unwrap(),
        );
        assert!(
            (a.x + b.x).abs() < 1.0 && (a.y - b.y).abs() < 1.0 && (a.z - b.z).abs() < 1.0,
            "{a} {b}"
        );
    }

    #[test]
    fn top_builds_into_front_and_back() {
        let (mesh, tapes) = body();
        let design = top(&mesh, &tapes);
        let g = build(&mesh, &design).unwrap();
        assert!(g.warnings.is_empty(), "{:?}", g.warnings);
        assert_eq!(g.curves.len(), 8);
        // Curves run along mesh edges.
        for c in &g.curves {
            for w in c.vertices.windows(2) {
                assert!(
                    g.body.find_edge(w[0], w[1]).is_some(),
                    "{} breaks at {:?}",
                    c.name,
                    w
                );
            }
        }
        let gm = g.mesh.as_ref().unwrap();
        let share = gm.triangles().len() as f64 / g.body.triangles().len() as f64;
        assert!((0.1..0.5).contains(&share), "garment share {share}");
        assert_eq!(g.seams.seams.len(), 4);

        let out = run(gm, &g.seams, &JobOptions::default()).unwrap();
        assert_eq!(out.report.panels.len(), 2, "{:?}", out.warnings);
        for p in &out.report.panels {
            assert!(p.flipped_triangles.is_empty(), "panel {} folds", p.index);
            // Ease 0.85: an area stretch of about 1/0.85² = 1.38 overall.
            assert!(
                (1.3..1.5).contains(&p.mean_area_stretch),
                "{}",
                p.mean_area_stretch
            );
        }
        // A one-piece front over the bust has longer side and shoulder
        // seams than the back. Matching halves the difference without
        // folding the panels, and the rest is reported as needing shaping.
        for s in &out.report.seams {
            assert!(
                s.mismatch_mm.abs() < 0.75 * s.mismatch_before_matching_mm.abs(),
                "{s:?}"
            );
        }
        assert!(
            out.warnings.iter().any(|w| w.contains("easing in")),
            "{:?}",
            out.warnings
        );
    }

    #[test]
    fn unenclosed_garment_is_reported() {
        let (mesh, tapes) = body();
        let mut design = top(&mesh, &tapes);
        design.curves.retain(|c| c.name == "side");
        let g = build(&mesh, &design).unwrap();
        assert!(
            g.warnings.iter().any(|w| w.contains("whole body")),
            "{:?}",
            g.warnings
        );
    }
}
