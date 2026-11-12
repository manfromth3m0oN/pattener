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
pub mod examples;
mod remesh;

use crate::error::{Error, Result};
use crate::mesh::Mesh;
use crate::pattern::{EdgeFinish, EdgeSpec};
use crate::seams::{Seam, SeamFinish, SeamSet};
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
    /// For a seam: glued, or a zip.
    #[cfg_attr(
        feature = "serde",
        serde(skip_serializing_if = "crate::seams::SeamFinish::is_default")
    )]
    pub seam_finish: SeamFinish,
    /// For an edge: how it is finished; `None` uses the job's hem
    /// allowance.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub edge_finish: Option<EdgeFinish>,
}

/// A region of the body with its own ease (see
/// [`EaseZone`](crate::ease::EaseZone)), placed on the body model.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DesignZone {
    #[cfg_attr(feature = "serde", serde(default))]
    pub name: String,
    pub center: SurfacePoint,
    pub radius_mm: f64,
    pub ease: f64,
    /// Also apply it at the mirror image on the other side of the body.
    #[cfg_attr(feature = "serde", serde(default))]
    pub mirror: bool,
}

/// "The piece containing this point is cut from this sheet."
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SheetAssignment {
    pub point: SurfacePoint,
    pub sheet: String,
}

/// A garment: curves, a point inside it, ease zones and sheets.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct GarmentDesign {
    pub curves: Vec<Curve>,
    /// A point on the body inside the garment.
    pub inside: Option<SurfacePoint>,
    pub zones: Vec<DesignZone>,
    /// Sheet for pieces without an assignment (default: the first sheet).
    pub default_sheet: Option<String>,
    /// Pieces cut from other sheets, chosen by a point on them. Points,
    /// not panel numbers, so assignments survive changes to the seams.
    pub sheet_assignments: Vec<SheetAssignment>,
    /// Build each side as drawn. By default a design is symmetric: it is
    /// built on the right half of the body (`x > 0`; curves drawn on the
    /// left are mirrored over, loops are cut at the centre line) and
    /// mirrored, so the two sides match exactly.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "std::ops::Not::not"))]
    pub asymmetric: bool,
}

/// Per panel of a built garment (in cut order), its triangles as indices
/// into the garment mesh.
pub fn panel_faces(g: &BuiltGarment) -> Result<Vec<Vec<u32>>> {
    let Some(mesh) = &g.mesh else {
        return Ok(Vec::new());
    };
    let panels = crate::cut::cut_panels(mesh, &g.seams.resolve(mesh)?)?;
    Ok(panels.into_iter().map(|p| p.face_origin).collect())
}

/// Resolve the design's sheets on a built garment: each panel gets the
/// default sheet, then each assignment sets the sheet of the panel nearest
/// its point (later assignments win). `body` is the fitted body the design
/// was built on; `sheets` the project's catalogue. Returns `None` without
/// sheets, plus warnings for assignments that miss the garment or name an
/// unknown sheet.
pub fn material_plan(
    body: &Mesh,
    g: &BuiltGarment,
    design: &GarmentDesign,
    sheets: &[crate::materials::Sheet],
) -> Result<(Option<crate::materials::MaterialPlan>, Vec<String>)> {
    let mut warnings = Vec::new();
    let (Some(mesh), false) = (&g.mesh, sheets.is_empty()) else {
        return Ok((None, warnings));
    };
    let index = |name: &str| sheets.iter().position(|s| s.name == name);
    let default = match &design.default_sheet {
        Some(n) => index(n).ok_or_else(|| {
            Error::Material(format!("default sheet '{n}' is not in the catalogue"))
        })?,
        None => 0,
    };
    let faces = panel_faces(g)?;
    let mut face_panel = vec![usize::MAX; mesh.triangles().len()];
    for (p, fs) in faces.iter().enumerate() {
        for &f in fs {
            face_panel[f as usize] = p;
        }
    }
    let mut panel_sheet = vec![default; faces.len()];
    for a in &design.sheet_assignments {
        let Some(si) = index(&a.sheet) else {
            warnings.push(format!(
                "sheet '{}' is not in the catalogue; its assignment is ignored",
                a.sheet
            ));
            continue;
        };
        let Some(p) = a.point.position(body.positions(), body.triangles()) else {
            warnings.push(format!(
                "an assignment of sheet '{}' is off the body",
                a.sheet
            ));
            continue;
        };
        let sp = locate(mesh.positions(), mesh.triangles(), p);
        let q = sp
            .position(mesh.positions(), mesh.triangles())
            .expect("located on mesh");
        if q.distance(p) > 10.0 {
            warnings.push(format!(
                "an assignment of sheet '{}' is not on the garment; it is ignored",
                a.sheet
            ));
            continue;
        }
        panel_sheet[face_panel[sp.face as usize]] = si;
    }
    Ok((
        Some(crate::materials::MaterialPlan {
            sheets: sheets.to_vec(),
            panel_sheet,
        }),
        warnings,
    ))
}

/// The design's ease zones placed on `body` (which has the body model's
/// triangles), including mirror images.
pub fn zones_on(body: &Mesh, design: &GarmentDesign) -> Result<Vec<crate::ease::EaseZone>> {
    let (pos, tri) = (body.positions(), body.triangles());
    let mut out = Vec::new();
    for z in &design.zones {
        let c = z
            .center
            .position(pos, tri)
            .ok_or_else(|| Error::Garment(format!("ease zone '{}' is off the body", z.name)))?;
        out.push(crate::ease::EaseZone {
            center: c,
            radius_mm: z.radius_mm,
            ease: z.ease,
        });
        if z.mirror {
            out.push(crate::ease::EaseZone {
                center: DVec3::new(-c.x, c.y, c.z),
                radius_mm: z.radius_mm,
                ease: z.ease,
            });
        }
    }
    Ok(out)
}

/// One embedded curve.
#[derive(Debug, Clone, PartialEq)]
pub struct CurvePath {
    pub name: String,
    pub kind: CurveKind,
    pub closed: bool,
    /// Vertex indices into [`BuiltGarment::body`], in order.
    pub vertices: Vec<u32>,
    pub seam_finish: SeamFinish,
    pub edge_finish: Option<EdgeFinish>,
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
    /// Finished edges (those with an `edge_finish`) in `mesh` vertex
    /// indices.
    pub edges: Vec<EdgeSpec>,
    /// For each vertex of `mesh`, its vertex in `body`.
    pub vertex_origin: Vec<u32>,
    /// For a symmetric build, each vertex of `mesh`'s mirror image across
    /// `x = 0` (itself on the centre line).
    pub mirror: Option<Vec<u32>>,
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

    // 1. Check the curves.
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
    }

    // 2. Embed: on one half and mirrored (exactly symmetric), or each
    // curve and its mirror separately.
    let symmetric = if design.asymmetric {
        None
    } else {
        body_mirror(body)
    };
    let inside = design.inside.and_then(|sp| sp.position(pos0, tri0));
    let (cut, paths, mirror) = match symmetric {
        Some(body_mirror) => embed_symmetric(body, &body_mirror, &curves, inside)?,
        None => {
            let (m, paths) = embed_both_sides(body, &curves, inside)?;
            let cut = Mesh::new(m.positions.clone(), m.live_triangles()).map_err(|e| {
                Error::Garment(format!("cutting the curves into the body failed: {e}"))
            })?;
            (cut, paths, None)
        }
    };
    let body_mesh = cut;

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
    let mut edges: Vec<EdgeSpec> = Vec::new();
    let mut vertex_origin = Vec::new();
    let mut garment_mirror = None;
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
                        vertex_origin.push(v);
                        (positions.len() - 1) as u32
                    })
                }));
            }
        }
        let gm = Mesh::new(positions, triangles)?;
        // Mirror images within the garment, if its area is symmetric too.
        if let Some(body_mirror) = &mirror {
            let m: Option<Vec<u32>> = vertex_origin
                .iter()
                .map(|&v| local.get(&body_mirror[v as usize]).copied())
                .collect();
            if m.is_none() {
                warnings.push(
                    "the garment's area is not the same on both sides, so its pieces are not \
                     mirrored"
                        .into(),
                );
            }
            garment_mirror = m;
        }
        // Runs of a curve along edges of the garment mesh, in its indices.
        let runs_of = |p: &CurvePath| {
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
            runs
        };
        for p in paths.iter().filter(|p| p.kind == CurveKind::Edge) {
            let Some(finish) = p.edge_finish else {
                continue;
            };
            let runs = runs_of(p);
            let several = runs.len() > 1;
            for (i, r) in runs.into_iter().enumerate() {
                edges.push(EdgeSpec {
                    name: if several {
                        format!("{} {}", p.name, i + 1)
                    } else {
                        p.name.clone()
                    },
                    vertices: r,
                    closed: false,
                    finish,
                });
            }
        }
        for p in paths.iter().filter(|p| p.kind == CurveKind::Seam) {
            let runs = runs_of(p);
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
                    finish: p.seam_finish,
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
        edges,
        vertex_origin,
        mirror: garment_mirror,
        warnings,
    })
}

/// The mirror image of each vertex across `x = 0`, if the body is exactly
/// symmetric (as the body model is) with no triangle crossing the
/// midplane.
pub fn body_mirror(body: &Mesh) -> Option<Vec<u32>> {
    let p = body.positions();
    let key = |q: DVec3| {
        (
            (q.x * 1000.0).round() as i64,
            (q.y * 1000.0).round() as i64,
            (q.z * 1000.0).round() as i64,
        )
    };
    let index: HashMap<_, u32> = p
        .iter()
        .enumerate()
        .map(|(i, &q)| (key(q), i as u32))
        .collect();
    let mirror: Option<Vec<u32>> = p
        .iter()
        .map(|&q| index.get(&key(DVec3::new(-q.x, q.y, q.z))).copied())
        .collect();
    let crossing = body.triangles().iter().any(|t| {
        let x = t.map(|i| p[i as usize].x);
        x.iter().any(|&v| v > MID_EPS) && x.iter().any(|&v| v < -MID_EPS)
    });
    if crossing {
        None
    } else {
        mirror
    }
}

/// Vertices closer than this (mm) to `x = 0` are on the body's midline.
const MID_EPS: f64 = 1e-6;
/// A curve whose points all lie this close (mm) to `x = 0` runs along the
/// centre line.
const CENTRE_MM: f64 = 15.0;

/// Nearest point to `p` on the midline edges `mid` of `m`: (edge, t).
fn nearest_on_midline(m: &EditMesh, mid: &[(u32, u32)], p: DVec3) -> Option<((u32, u32), f64)> {
    mid.iter()
        .map(|&(a, b)| {
            let (pa, pb) = (m.positions[a as usize], m.positions[b as usize]);
            let d = pb - pa;
            let t = ((p - pa).dot(d) / d.length_squared().max(1e-12)).clamp(0.0, 1.0);
            ((a, b), t, p.distance(pa + t * d))
        })
        .min_by(|x, y| x.2.total_cmp(&y.2))
        .map(|(e, t, _)| (e, t))
}

/// Make the point of the midline nearest `p` a vertex.
fn midline_vertex(m: &mut EditMesh, p: DVec3) -> Option<u32> {
    let mid = midline_edges(m);
    let ((a, b), t) = nearest_on_midline(m, &mid, p)?;
    let len = m.positions[a as usize].distance(m.positions[b as usize]);
    Some(if t * len < 2.0 {
        a
    } else if (1.0 - t) * len < 2.0 {
        b
    } else {
        m.split_edge(a, b, t)
    })
}

/// Live edges with both ends on the midplane.
fn midline_edges(m: &EditMesh) -> Vec<(u32, u32)> {
    let mut e = HashSet::new();
    for t in m.triangles.iter().filter(|t| t[0] != u32::MAX) {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if m.positions[a as usize].x == 0.0 && m.positions[b as usize].x == 0.0 {
                e.insert((a.min(b), a.max(b)));
            }
        }
    }
    let mut e: Vec<_> = e.into_iter().collect();
    e.sort_unstable();
    e
}

/// How a design curve is cut on the right half.
enum HalfCurve {
    /// Along the centre line between two midline vertices; `guide` picks
    /// which way round.
    Centre { guide: DVec3 },
    /// Pieces of the curve on the right half.
    Runs(Vec<(Vec<DVec3>, bool)>),
}

/// Pieces of a sampled curve with `x ≥ 0`; ends where it crosses the
/// midplane are moved onto the midline. A closed curve that stays on the
/// right is one closed run.
fn clip_right(m: &EditMesh, samples: &[DVec3], closed: bool) -> Vec<(Vec<DVec3>, bool)> {
    let n = samples.len();
    if samples.iter().all(|p| p.x >= 0.0) {
        return vec![(samples.to_vec(), closed)];
    }
    let mid = midline_edges(m);
    let on_mid = |p: DVec3| {
        nearest_on_midline(m, &mid, p)
            .map(|((a, b), t)| m.positions[a as usize].lerp(m.positions[b as usize], t))
            .unwrap_or(p)
    };
    let segs = if closed { n } else { n - 1 };
    // Start a closed curve at a left sample so that runs do not wrap.
    let start = if closed {
        samples.iter().position(|p| p.x < 0.0).unwrap_or(0)
    } else {
        0
    };
    let at = |i: usize| samples[(start + i) % n];
    let mut runs = Vec::new();
    let mut cur: Vec<DVec3> = Vec::new();
    if at(0).x >= 0.0 {
        cur.push(at(0));
    }
    for i in 0..segs {
        let (a, b) = (at(i), at(i + 1));
        match (a.x >= 0.0, b.x >= 0.0) {
            (true, true) => cur.push(b),
            (true, false) => {
                let t = a.x / (a.x - b.x);
                cur.push(on_mid(a.lerp(b, t)));
                runs.push((std::mem::take(&mut cur), false));
            }
            (false, true) => {
                let t = a.x / (a.x - b.x);
                cur = vec![on_mid(a.lerp(b, t)), b];
            }
            (false, false) => {}
        }
    }
    if cur.len() > 1 {
        runs.push((cur, false));
    }
    runs.retain(|(r, _)| r.len() > 1);
    runs
}

/// Embed the design on the right half of an exactly symmetric body and
/// mirror it: the cut body, the curves, and each vertex's mirror image.
fn embed_symmetric(
    body: &Mesh,
    body_mirror: &[u32],
    curves: &[Curve],
    inside: Option<DVec3>,
) -> Result<(Mesh, Vec<CurvePath>, Option<Vec<u32>>)> {
    let (pos0, tri0) = (body.positions(), body.triangles());
    let mut positions = pos0.to_vec();
    for (i, p) in positions.iter_mut().enumerate() {
        if body_mirror[i] as usize == i || p.x.abs() < MID_EPS {
            p.x = 0.0;
        }
    }
    let mut m = EditMesh::new(positions, tri0.to_vec());
    m.keep_midline = true;

    // Each curve as centre-line ends or right-half runs; centre curves
    // first (so other seams can end on them), then edges, then seams.
    let mut order: Vec<&Curve> = curves.iter().collect();
    let centre = |c: &Curve| {
        !c.mirror
            && c.points.iter().all(|sp| {
                sp.position(pos0, tri0)
                    .is_some_and(|p| p.x.abs() < CENTRE_MM)
            })
    };
    order.sort_by_key(|c| (!centre(c), c.kind == CurveKind::Seam));
    let mut plan: Vec<(&Curve, HalfCurve)> = Vec::new();
    let mut paths: Vec<CurvePath> = Vec::new();
    for c in order {
        let control: Vec<DVec3> = c
            .points
            .iter()
            .map(|sp| sp.position(pos0, tri0).expect("checked"))
            .collect();
        if centre(c) {
            let flat = |p: DVec3| DVec3::new(0.0, p.y, p.z);
            let a = midline_vertex(&mut m, flat(control[0]));
            let b = midline_vertex(&mut m, flat(control[control.len() - 1]));
            let (Some(a), Some(b)) = (a, b) else {
                return Err(Error::Garment("the body has no centre line".into()));
            };
            let guide = flat(control[control.len() / 2]);
            // Placeholder, so later seams can end on these vertices.
            paths.push(CurvePath {
                name: c.name.clone(),
                kind: c.kind,
                closed: false,
                vertices: vec![a, b],
                seam_finish: c.seam_finish,
                edge_finish: c.edge_finish,
            });
            plan.push((c, HalfCurve::Centre { guide }));
            continue;
        }
        // Draw on the right: mirror curves drawn mostly on the left.
        let mean_x = control.iter().map(|p| p.x).sum::<f64>() / control.len() as f64;
        let control: Vec<DVec3> = if mean_x < 0.0 {
            control.iter().map(|p| DVec3::new(-p.x, p.y, p.z)).collect()
        } else {
            control
        };
        let samples = smooth(&control, c.closed, SAMPLE_MM);
        let runs = clip_right(&m, &samples, c.closed);
        for (run, closed) in &runs {
            let path = embed_samples(&mut m, &mut paths, c, *closed, run)?;
            paths.push(CurvePath {
                name: c.name.clone(),
                kind: c.kind,
                closed: *closed,
                vertices: path,
                seam_finish: c.seam_finish,
                edge_finish: c.edge_finish,
            });
        }
        plan.push((c, HalfCurve::Runs(runs)));
    }
    cleanup(&mut m, &mut paths);
    if let Some(p) = inside {
        remesh::remesh_garment_area(&mut m, &mut paths, p, pos0, tri0, true);
    }

    // Mirror the right half.
    let live = m.live_triangles();
    let p = &m.positions;
    if live.iter().any(|t| {
        let x = t.map(|i| p[i as usize].x);
        x.iter().any(|&v| v > 0.0) && x.iter().any(|&v| v < 0.0)
    }) {
        return Err(Error::Garment(
            "a curve crosses the centre line of the body; end it on the centre line or tick \
             'Asymmetric design'"
                .into(),
        ));
    }
    let right: Vec<[u32; 3]> = live
        .into_iter()
        .filter(|t| {
            t.iter().all(|&i| p[i as usize].x >= 0.0) && t.iter().any(|&i| p[i as usize].x > 0.0)
        })
        .collect();
    let mut new_index = vec![u32::MAX; p.len()];
    let mut positions: Vec<DVec3> = Vec::new();
    for t in &right {
        for &v in t {
            if new_index[v as usize] == u32::MAX {
                new_index[v as usize] = positions.len() as u32;
                positions.push(p[v as usize]);
            }
        }
    }
    let half = positions.len();
    let mut mirror: Vec<u32> = (0..half as u32).collect();
    for i in 0..half {
        if positions[i].x > 0.0 {
            mirror[i] = positions.len() as u32;
            mirror.push(i as u32);
            let q = positions[i];
            positions.push(DVec3::new(-q.x, q.y, q.z));
        }
    }
    let mut triangles: Vec<[u32; 3]> = right
        .iter()
        .map(|t| t.map(|v| new_index[v as usize]))
        .collect();
    let mirrored: Vec<[u32; 3]> = triangles
        .iter()
        .map(|t| {
            [
                mirror[t[0] as usize],
                mirror[t[2] as usize],
                mirror[t[1] as usize],
            ]
        })
        .collect();
    triangles.extend(mirrored);
    let cut = Mesh::new(positions, triangles)
        .map_err(|e| Error::Garment(format!("mirroring the cut body failed: {e}")))?;

    // Curves on the full body: the right-half pieces joined to their
    // mirror images; centre curves along the midline.
    let mid_adj = {
        let mut adj: HashMap<u32, Vec<u32>> = HashMap::new();
        for e in cut.edges() {
            let [a, b] = e.v;
            if cut.positions()[a as usize].x == 0.0 && cut.positions()[b as usize].x == 0.0 {
                adj.entry(a).or_default().push(b);
                adj.entry(b).or_default().push(a);
            }
        }
        adj
    };
    let remap = |v: u32| new_index[v as usize];
    let on_mid = |v: u32| cut.positions()[v as usize].x == 0.0;
    let mut out: Vec<CurvePath> = Vec::new();
    let mut half_paths = paths.into_iter();
    for (c, h) in plan {
        let meta = |name: String, closed: bool, vertices: Vec<u32>| CurvePath {
            name,
            kind: c.kind,
            closed,
            vertices,
            seam_finish: c.seam_finish,
            edge_finish: c.edge_finish,
        };
        match h {
            HalfCurve::Centre { guide } => {
                let placeholder = half_paths.next().expect("one path per centre curve");
                let (a, b) = (
                    remap(placeholder.vertices[0]),
                    remap(*placeholder.vertices.last().expect("two ends")),
                );
                let route = midline_route(&cut, &mid_adj, a, b, guide).ok_or_else(|| {
                    Error::Garment(format!(
                        "curve '{}' does not follow the centre line",
                        c.name
                    ))
                })?;
                out.push(meta(c.name.clone(), false, route));
            }
            HalfCurve::Runs(runs) => {
                for _ in 0..runs.len() {
                    let hp = half_paths.next().expect("one path per run");
                    let r: Vec<u32> = hp.vertices.iter().map(|&v| remap(v)).collect();
                    if r.contains(&u32::MAX) {
                        return Err(Error::Garment(format!(
                            "curve '{}' crosses the centre line of the body; end it on the centre \
                             line or tick 'Asymmetric design'",
                            c.name
                        )));
                    }
                    // The mirror image, reversed so that "left of the
                    // curve" maps to the mirror of the same side.
                    let back: Vec<u32> = r.iter().rev().map(|&v| mirror[v as usize]).collect();
                    let n = r.len();
                    let (first, last) = (on_mid(r[0]), on_mid(r[n - 1]));
                    if hp.closed {
                        out.push(meta(c.name.clone(), true, r));
                        out.push(meta(format!("{} (mirror)", c.name), true, back));
                    } else if first && last && n > 2 {
                        let mut v = r.clone();
                        v.extend_from_slice(&back[1..back.len() - 1]);
                        out.push(meta(c.name.clone(), true, v));
                    } else if last {
                        let mut v = r.clone();
                        v.extend_from_slice(&back[1..]);
                        out.push(meta(c.name.clone(), false, v));
                    } else if first {
                        let mut v = back.clone();
                        v.extend_from_slice(&r[1..]);
                        out.push(meta(c.name.clone(), false, v));
                    } else {
                        out.push(meta(c.name.clone(), false, r));
                        out.push(meta(format!("{} (mirror)", c.name), false, back));
                    }
                }
            }
        }
    }
    Ok((cut, out, Some(mirror)))
}

/// The way along the midline from `a` to `b` that passes nearest `guide`.
fn midline_route(
    mesh: &Mesh,
    adj: &HashMap<u32, Vec<u32>>,
    a: u32,
    b: u32,
    guide: DVec3,
) -> Option<Vec<u32>> {
    let p = mesh.positions();
    let mut best: Option<(f64, Vec<u32>)> = None;
    for &first in adj.get(&a)? {
        let mut route = vec![a, first];
        let mut ok = true;
        while *route.last().expect("non-empty") != b {
            let (prev, cur) = (route[route.len() - 2], route[route.len() - 1]);
            let next = adj
                .get(&cur)
                .and_then(|n| n.iter().copied().find(|&x| x != prev));
            match next {
                Some(x) if !route.contains(&x) => route.push(x),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            let d = route
                .iter()
                .map(|&v| p[v as usize].distance(guide))
                .fold(f64::INFINITY, f64::min);
            if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                best = Some((d, route));
            }
        }
    }
    best.map(|(_, r)| r)
}

/// Embed each curve, and the mirror image of mirrored curves, into the
/// full body.
fn embed_both_sides(
    body: &Mesh,
    design_curves: &[Curve],
    inside: Option<DVec3>,
) -> Result<(EditMesh, Vec<CurvePath>)> {
    let (pos0, tri0) = (body.positions(), body.triangles());
    let mut curves: Vec<Curve> = Vec::new();
    for c in design_curves {
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
    let mut m = EditMesh::new(pos0.to_vec(), tri0.to_vec());
    let mut paths: Vec<CurvePath> = Vec::new();
    for c in &curves {
        let control: Vec<DVec3> = c
            .points
            .iter()
            .map(|sp| sp.position(pos0, tri0).expect("checked"))
            .collect();
        let samples = smooth(&control, c.closed, SAMPLE_MM);
        let path = embed_samples(&mut m, &mut paths, c, c.closed, &samples)?;
        paths.push(CurvePath {
            name: c.name.clone(),
            kind: c.kind,
            closed: c.closed,
            vertices: path,
            seam_finish: c.seam_finish,
            edge_finish: c.edge_finish,
        });
    }
    cleanup(&mut m, &mut paths);
    if let Some(p) = inside {
        remesh::remesh_garment_area(&mut m, &mut paths, p, pos0, tri0, false);
    }
    Ok((m, paths))
}

/// Cut a sampled curve into `m`: each sample becomes a vertex (seam ends
/// snap onto earlier curves), joined by plane walks.
fn embed_samples(
    m: &mut EditMesh,
    paths: &mut [CurvePath],
    c: &Curve,
    closed: bool,
    samples: &[DVec3],
) -> Result<Vec<u32>> {
    let n = samples.len();
    let mut verts = Vec::with_capacity(n);
    for (k, &p) in samples.iter().enumerate() {
        let end = !closed && (k == 0 || k == n - 1);
        let snapped = if end && m.keep_midline && p.x == 0.0 {
            // An end on the centre line (a loop cut there): join a curve
            // already on the centre line nearby, so the two meet exactly.
            nearest_midline_curve_vertex(m, paths, p, SNAP_MM).or_else(|| {
                (c.kind == CurveKind::Seam)
                    .then(|| nearest_curve_vertex(m, paths, p, SNAP_MM))
                    .flatten()
            })
        } else if end && c.kind == CurveKind::Seam {
            nearest_curve_vertex(m, paths, p, SNAP_MM)
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
        // Only a curve's ends may lie on the centre line: a point next to
        // it that landed there would make the curve run along it.
        if !end && m.pinned(v) {
            continue;
        }
        if verts.last() != Some(&v) {
            verts.push(v);
        }
    }
    if closed && verts.len() > 1 && verts.first() == verts.last() {
        verts.pop();
    }
    let mut path = vec![verts[0]];
    let segments = if closed { verts.len() } else { verts.len() - 1 };
    for s in 0..segments {
        let (a, b) = (verts[s], verts[(s + 1) % verts.len()]);
        if a == b {
            continue;
        }
        let start = path.len();
        if walk(m, paths, &mut path, a, b).is_err() {
            // The plane through the two points does not join them on
            // the surface (e.g. through the armpit): fall back to the
            // shortest route along mesh edges.
            path.truncate(start);
            shortest_edge_path(m, a, b, &mut path).ok_or_else(|| {
                Error::Garment(format!("curve '{}' crosses a gap in the body", c.name))
            })?;
        }
    }
    if closed && path.len() > 1 && path.first() == path.last() {
        path.pop();
    }
    Ok(path)
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

/// [`nearest_curve_vertex`] among curve vertices on the centre line.
fn nearest_midline_curve_vertex(
    m: &EditMesh,
    paths: &[CurvePath],
    p: DVec3,
    within: f64,
) -> Option<u32> {
    paths
        .iter()
        .flat_map(|c| c.vertices.iter().copied())
        .filter(|&v| m.pinned(v))
        .map(|v| (v, m.positions[v as usize].distance(p)))
        .filter(|&(_, d)| d <= within)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(v, _)| v)
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
                // Never run along the centre line (it would leave a spur
                // there instead of crossing it).
                Step::Vertex(x) if x != b && m.pinned(v) && m.pinned(x) => continue,
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
                let crosses_midline =
                    m.keep_midline && (p.x < 0.0) != (m.positions[x as usize].x < 0.0);
                if !paths.iter().any(|c| c.vertices.contains(&x))
                    && !path.contains(&x)
                    && !m.pinned(x)
                    && !crosses_midline
                {
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
                // Never along the centre line (see `walk`).
                if w != b && m.pinned(w) {
                    continue;
                }
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
    use super::edit::locate;
    use super::examples::top;
    use super::*;
    use crate::human::{fit, test_model, MeasureKind, Measurements, Measurer, ShapeOptions};
    use crate::job::{run, JobOptions};

    /// The default body and its tape loops.
    pub(crate) fn fitted() -> crate::human::HumanBody {
        fitted_as(0.0)
    }

    /// The default body with the given body type (0 feminine, 1 masculine).
    pub(crate) fn fitted_as(body_type: f64) -> crate::human::HumanBody {
        let m = test_model();
        let me = Measurer::new(&m).unwrap();
        let shape = ShapeOptions {
            body_type,
            ..Default::default()
        };
        fit(&m, &me, &Measurements::new(), &shape).unwrap()
    }

    pub(crate) fn body() -> (Mesh, Vec<(MeasureKind, Vec<DVec3>)>) {
        let m = test_model();
        let me = Measurer::new(&m).unwrap();
        let b = fit(&m, &me, &Measurements::new(), &ShapeOptions::default()).unwrap();
        (b.mesh, b.tapes)
    }

    fn tape(tapes: &[(MeasureKind, Vec<DVec3>)], k: MeasureKind) -> &[DVec3] {
        &tapes.iter().find(|t| t.0 == k).unwrap().1
    }

    fn extreme(loop_: &[DVec3], key: impl Fn(&DVec3) -> f64) -> DVec3 {
        *loop_
            .iter()
            .max_by(|a, b| key(a).total_cmp(&key(b)))
            .unwrap()
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
    fn pressure_map_and_ease_zone_on_the_top() {
        let (mesh, tapes) = body();
        let mut design = top(&mesh, &tapes);
        let g = build(&mesh, &design).unwrap();
        let gm = g.mesh.as_ref().unwrap();
        let plain = run(gm, &g.seams, &JobOptions::default()).unwrap();
        assert_eq!(plain.fit.pressure_mmhg.len(), gm.positions().len());
        for p in &plain.report.panels {
            // Ease 0.85 on a torso: light compression on average, with a
            // little bridging (e.g. between the breasts, small of the back).
            assert!(
                (2.0..20.0).contains(&p.mean_pressure_mmhg),
                "{}",
                p.mean_pressure_mmhg
            );
            assert!(p.p95_pressure_mmhg < 40.0, "{}", p.p95_pressure_mmhg);
            assert!(p.bridging_fraction < 0.25, "{}", p.bridging_fraction);
        }
        assert!(!plain.warnings.iter().any(|w| w.contains("presses hard")));

        // A looser zone over the front of the waist lowers the front's pressure.
        let waist_front = extreme(tape(&tapes, MeasureKind::Waist), |p| p.z);
        design.zones.push(DesignZone {
            name: "waist".into(),
            center: locate(mesh.positions(), mesh.triangles(), waist_front),
            radius_mm: 200.0,
            ease: 0.97,
            mirror: false,
        });
        let zones = zones_on(&mesh, &design).unwrap();
        let loose = run(
            gm,
            &g.seams,
            &JobOptions {
                ease_zones: zones,
                ..Default::default()
            },
        )
        .unwrap();
        // Near the zone's centre the latex is barely stretched (1/0.97
        // instead of 1/0.85), so it presses far less.
        let near = |out: &crate::job::JobOutput| {
            let v: Vec<f64> = gm
                .positions()
                .iter()
                .zip(&out.fit.pressure_mmhg)
                .filter(|(p, _)| p.distance(waist_front) < 60.0)
                .map(|(_, &x)| x)
                .collect();
            v.iter().sum::<f64>() / v.len() as f64
        };
        let (before, after) = (near(&plain), near(&loose));
        assert!(after < 0.5 * before, "{before} -> {after}");
    }

    #[test]
    fn sheet_assignments_follow_points() {
        use crate::materials::Sheet;
        let (mesh, tapes) = body();
        let mut design = top(&mesh, &tapes);
        let sheet = |name: &str, color: &str| Sheet {
            name: name.into(),
            color: color.into(),
            thickness_mm: 0.4,
            roll_width_mm: 1000.0,
            ease: None,
            shear_modulus_mpa: None,
            price_per_m: None,
        };
        let sheets = vec![sheet("black", "#111111"), sheet("red", "#b01020")];
        let back = extreme(tape(&tapes, MeasureKind::Waist), |p| -p.z);
        design.sheet_assignments.push(SheetAssignment {
            point: locate(mesh.positions(), mesh.triangles(), back),
            sheet: "red".into(),
        });
        // Unknown sheet, and a point off the garment (on the thigh).
        design.sheet_assignments.push(SheetAssignment {
            point: locate(mesh.positions(), mesh.triangles(), back),
            sheet: "gold".into(),
        });
        let thigh = extreme(tape(&tapes, MeasureKind::Thigh), |p| p.z);
        design.sheet_assignments.push(SheetAssignment {
            point: locate(mesh.positions(), mesh.triangles(), thigh),
            sheet: "black".into(),
        });
        let g = build(&mesh, &design).unwrap();
        let (plan, warnings) = material_plan(&mesh, &g, &design, &sheets).unwrap();
        let plan = plan.unwrap();
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        // Exactly the back panel (the one containing the back of the waist) is red.
        let faces = panel_faces(&g).unwrap();
        let gm = g.mesh.as_ref().unwrap();
        let centre_z = |p: usize| {
            faces[p]
                .iter()
                .map(|&f| {
                    gm.triangles()[f as usize]
                        .iter()
                        .map(|&v| gm.positions()[v as usize].z)
                        .sum::<f64>()
                        / 3.0
                })
                .sum::<f64>()
                / faces[p].len() as f64
        };
        for p in 0..faces.len() {
            let is_back = centre_z(p) < centre_z(1 - p);
            assert_eq!(plan.panel_sheet[p], usize::from(is_back), "panel {p}");
        }
        // Without a catalogue there is no plan.
        assert!(material_plan(&mesh, &g, &design, &[]).unwrap().0.is_none());
    }

    #[test]
    fn assembly_order_on_the_top() {
        let (mesh, tapes) = body();
        let design = top(&mesh, &tapes);
        let g = build(&mesh, &design).unwrap();
        let out = run(g.mesh.as_ref().unwrap(), &g.seams, &JobOptions::default()).unwrap();
        let steps = &out.report.assembly;
        // Cutting first; then one shoulder joins front and back (shortest
        // seam), and the other shoulder and both sides close the garment.
        assert!(steps[0].starts_with("Cut each piece"));
        assert!(
            steps[1].starts_with("Join pieces") && steps[1].contains("shoulder"),
            "{}",
            steps[1]
        );
        assert_eq!(
            steps.iter().filter(|s| s.starts_with("Close")).count(),
            3,
            "{steps:#?}"
        );
        assert!(
            steps[1].contains("6 mm lap") && steps[1].contains("match notches"),
            "{}",
            steps[1]
        );
    }

    #[test]
    fn curve_finishes_reach_the_pattern() {
        use crate::pattern::EdgeFinish;
        use crate::seams::SeamFinish;
        let (mesh, tapes) = body();
        let mut design = top(&mesh, &tapes);
        for c in &mut design.curves {
            match c.name.as_str() {
                "neckline" => {
                    c.edge_finish = Some(EdgeFinish::Trim {
                        width_mm: 15.0,
                        ease: 0.9,
                    })
                }
                "hem" => c.edge_finish = Some(EdgeFinish::Hem { width_mm: 20.0 }),
                "side" => c.seam_finish = SeamFinish::Zip,
                _ => {}
            }
        }
        let g = build(&mesh, &design).unwrap();
        let names: Vec<&str> = g.edges.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["hem", "neckline"]);
        let zips = g
            .seams
            .seams
            .iter()
            .filter(|s| s.finish == SeamFinish::Zip)
            .count();
        assert_eq!(zips, 2, "side and its mirror");
        let opts = JobOptions {
            edges: g.edges.clone(),
            print: Some(Default::default()),
            ..Default::default()
        };
        let out = run(g.mesh.as_ref().unwrap(), &g.seams, &opts).unwrap();
        let r = &out.report;
        assert_eq!(r.trims.len(), 1);
        let t = &r.trims[0];
        assert_eq!(t.edge, "neckline");
        assert!((t.length_mm - 0.9 * t.edge_length_mm).abs() < 1e-9);
        // The whole neckline, near the default ease (0.85) of its body length
        // (the curved shoulders shorten the flat edge a little more).
        let gm = g.mesh.as_ref().unwrap();
        let neck = &g.edges[1].vertices;
        let body_len: f64 = neck
            .windows(2)
            .map(|w| gm.positions()[w[0] as usize].distance(gm.positions()[w[1] as usize]))
            .sum();
        let ratio = t.edge_length_mm / body_len;
        assert!(
            (0.65..0.95).contains(&ratio),
            "{} / {body_len}",
            t.edge_length_mm
        );
        let all = r.assembly.join("\n");
        assert!(all.contains("Fit the zip in the side"), "{all}");
        assert!(all.contains("Hem the hem edge"), "{all}");
        assert!(all.contains("Finish the neckline edge"), "{all}");
        assert_eq!(out.trim_svgs.len(), 1);
        let pdf = out.pdf.as_ref().unwrap();
        assert!(pdf.starts_with("%PDF") && pdf.contains("Fit the zip in the side"));
        assert!(pdf.contains("neckline trim  -  page A1"));
        if let Ok(path) = std::env::var("LATEXPAT_WRITE_PDF") {
            std::fs::write(path, pdf).unwrap();
        }
    }

    #[test]
    fn every_template_makes_its_pieces() {
        use super::examples::TEMPLATES;
        // On the women's and the men's figure alike.
        for body_type in [0.0, 1.0] {
            let fitted = fitted_as(body_type);
            let mesh = &fitted.mesh;
            for (t, pieces) in TEMPLATES.iter().zip([2, 4, 4, 4, 2, 4, 7, 4]) {
                let g = build(mesh, &(t.build)(&fitted)).unwrap();
                assert!(g.warnings.is_empty(), "{}: {:?}", t.id, g.warnings);
                // Evenly triangulated: no slivers.
                let gm = g.mesh.as_ref().unwrap();
                let angles: Vec<f64> = gm
                    .triangles()
                    .iter()
                    .map(|t| {
                        let [a, b, c] = t.map(|i| gm.positions()[i as usize]);
                        (b - a)
                            .angle_between(c - a)
                            .min((a - b).angle_between(c - b))
                            .min((a - c).angle_between(b - c))
                    })
                    .collect();
                let thin = angles.iter().filter(|&&x| x < 15f64.to_radians()).count();
                // (Without re-triangulation 4–7 % are; what remains sits
                // between curve vertices, which stay where they are.)
                assert!(
                    thin * 60 < angles.len(),
                    "{}: {thin} of {} triangles have an angle under 15°",
                    t.id,
                    angles.len()
                );
                // Built symmetric: every garment vertex has a mirror image.
                let mirror = g.mirror.as_ref().expect("symmetric build");
                for (v, &w) in mirror.iter().enumerate() {
                    let (p, q) = (gm.positions()[v], gm.positions()[w as usize]);
                    assert!(
                        (p.x + q.x).abs() < 1e-9 && p.y == q.y && p.z == q.z,
                        "{}",
                        t.id
                    );
                    assert_eq!(mirror[w as usize] as usize, v);
                }
                let faces = panel_faces(&g).unwrap();
                let sizes: Vec<usize> = faces.iter().map(|f| f.len()).collect();
                assert_eq!(
                    sizes.len(),
                    pieces,
                    "{} ({body_type}): piece sizes {sizes:?}",
                    t.id
                );
                // No slivers where seams meet (a gusset is small but real).
                assert!(sizes.iter().all(|&n| n > 30), "{}: {sizes:?}", t.id);
            }
        }
    }

    #[test]
    fn symmetric_garments_give_mirror_image_pieces() {
        use super::examples::{cincher, top};
        use crate::flatten::{flatten_garment, FlattenOptions};
        use crate::outline::mirror_partners;
        let (mesh, tapes) = body();
        // The cincher: left and right fronts and backs; the top: front and
        // back, each symmetric about the centre line.
        for (design, expect_pairs) in [(cincher(&mesh, &tapes), true), (top(&mesh, &tapes), false)]
        {
            let g = build(&mesh, &design).unwrap();
            let gm = g.mesh.as_ref().unwrap();
            let opts = FlattenOptions {
                mirror: g.mirror.clone(),
                outline_smoothing_mm: crate::flatten::DEFAULT_OUTLINE_SMOOTHING_MM,
                ..Default::default()
            };
            let flat = flatten_garment(gm, &g.seams, &opts).unwrap();
            let m = g.mirror.as_ref().unwrap();
            let partners = mirror_partners(gm, m, &flat.panels);
            for (q, part) in partners.iter().enumerate() {
                let (p, map) = part.as_ref().expect("every piece has a mirror image");
                assert_eq!(flat.mirrored[q], Some(*p));
                assert_eq!(*p != q, expect_pairs);
                // Distances between corresponding points agree exactly:
                // the layouts are congruent by a reflection.
                let (a, b) = (&flat.panels[q].uv, &flat.panels[*p].uv);
                for i in (0..a.len()).step_by(7) {
                    for j in (0..a.len()).step_by(11) {
                        let d1 = a[i].distance(a[j]);
                        let d2 = b[map[i] as usize].distance(b[map[j] as usize]);
                        assert!((d1 - d2).abs() < 1e-6, "{d1} vs {d2}");
                    }
                }
                let (s1, s2) = (&flat.panels[q].strain, &flat.panels[*p].strain);
                assert!((s1.p95_stretch - s2.p95_stretch).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn darts_open_into_wedges_and_strain_hints_point_at_the_bust() {
        use super::examples::{cincher, top_set_in};
        use crate::job::{run, JobOptions};
        use crate::seams::SeamFinish;
        let fitted = fitted();
        let mesh = &fitted.mesh;
        let tapes = &fitted.tapes;
        let job = |design: &GarmentDesign| {
            let g = build(mesh, design).unwrap();
            let opts = JobOptions {
                mirror: g.mirror.clone(),
                length_relief: 0.4,
                ..Default::default()
            };
            (
                g.clone(),
                run(g.mesh.as_ref().unwrap(), &g.seams, &opts).unwrap(),
            )
        };
        // The cincher lies flat: no hints.
        let (_, out) = job(&cincher(mesh, tapes));
        assert!(out.report.panels.iter().all(|p| p.strain_hint.is_none()));
        // The set-in top's front strains at the bust.
        let plain = top_set_in(&fitted);
        let (_, out) = job(&plain);
        let front = &out.report.panels[0];
        let hint = front.strain_hint.expect("a hint on the front");
        let bust = tapes.iter().find(|t| t.0 == MeasureKind::Bust).unwrap();
        let apex = bust
            .1
            .iter()
            .copied()
            .max_by(|a, b| a.z.total_cmp(&b.z))
            .unwrap();
        assert!(
            (hint[1] - apex.y).abs() < 80.0 && hint[2] > 100.0,
            "hint {hint:?}, bust apex {apex}"
        );
        // A side bust dart: from the side seam to just short of the bust
        // point. It stays one front piece, opens into a wedge, and eases
        // the strain there.
        let side = bust
            .1
            .iter()
            .copied()
            .max_by(|a, b| a.x.total_cmp(&b.x))
            .unwrap();
        let tip = bust
            .1
            .iter()
            .copied()
            .filter(|p| p.x > 20.0)
            .max_by(|a, b| a.z.total_cmp(&b.z))
            .unwrap();
        let at = |p: DVec3| locate(mesh.positions(), mesh.triangles(), p);
        let mut darted = plain.clone();
        darted.curves.push(Curve {
            name: "bust dart".into(),
            kind: CurveKind::Seam,
            points: vec![at(side - 15.0 * DVec3::Y), at(side.lerp(tip, 0.8))],
            mirror: true,
            seam_finish: SeamFinish::Glued,
            ..Default::default()
        });
        let (g, out2) = job(&darted);
        assert_eq!(panel_faces(&g).unwrap().len(), 4);
        let dart = out2
            .report
            .seams
            .iter()
            .find(|s| s.seam.starts_with("bust dart"))
            .expect("the dart is a seam");
        assert_eq!(
            dart.left_panel, dart.right_panel,
            "a dart joins a piece to itself"
        );
        assert!(out2.report.panels[0].strain_spread < front.strain_spread);
    }

    #[test]
    fn asymmetric_designs_build_each_side() {
        let (mesh, tapes) = body();
        let mut design = top(&mesh, &tapes);
        design.asymmetric = true;
        let g = build(&mesh, &design).unwrap();
        assert!(g.mirror.is_none());
        assert_eq!(panel_faces(&g).unwrap().len(), 2);
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
