// Outline geometry of design pieces: the same curves as the Rust core
// (crates/core/src/geom.rs and design.rs), so the editor draws, and
// measures, exactly what the pattern is made from.
//
// A piece is { id, name, nodes: [{ id, x, y, kind, notch, edge }], at, pair, sheet }.
// The outline passes through the nodes: sharp at a "corner", smooth at a
// "smooth" or "joint" node; an edge runs from one corner or joint to the next.
'use strict';

const Geo = (() => {
  const sub = (a, b) => [a[0] - b[0], a[1] - b[1]];
  const add = (a, b) => [a[0] + b[0], a[1] + b[1]];
  const mul = (a, k) => [a[0] * k, a[1] * k];
  const dist = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1]);
  const pos = (n) => [n.x, n.y];
  const isVertex = (kind) => kind !== 'smooth';
  const kindOf = (n) => n.kind || 'corner';

  // Inner control point next to p1 of the centripetal Catmull–Rom segment
  // p1 → p2 whose other neighbour is p0.
  function handle(p0, p1, p2) {
    const d1 = Math.sqrt(dist(p1, p0));
    const d2 = Math.sqrt(dist(p2, p1));
    if (d1 < 1e-9 || d2 < 1e-9) return add(p1, mul(sub(p2, p1), 1 / 3));
    const k = 3 * d1 * (d1 + d2);
    const c = 2 * d1 * d1 + 3 * d1 * d2 + d2 * d2;
    return [
      (d1 * d1 * p2[0] - d2 * d2 * p0[0] + c * p1[0]) / k,
      (d1 * d1 * p2[1] - d2 * d2 * p0[1] + c * p1[1]) / k,
    ];
  }
  const segment = (p0, p1, p2, p3) => [p1, handle(p0, p1, p2), handle(p3, p2, p1), p2];
  const line = (a, b) => [a, add(a, mul(sub(b, a), 1 / 3)), add(a, mul(sub(b, a), 2 / 3)), b];

  function reflect(p, a, b) {
    let d = sub(b, a);
    const l = Math.hypot(d[0], d[1]);
    if (l === 0) return sub(mul(a, 2), p);
    d = mul(d, 1 / l);
    const v = sub(p, a);
    const t = v[0] * d[0] + v[1] * d[1];
    return sub(add(a, mul(d, 2 * t)), v);
  }

  function bezierPoint(b, t) {
    const u = 1 - t;
    const w = [u * u * u, 3 * u * u * t, 3 * u * t * t, t * t * t];
    return [0, 1].map((i) => w[0] * b[0][i] + w[1] * b[1][i] + w[2] * b[2][i] + w[3] * b[3][i]);
  }
  function sampleCount(b) {
    const l = dist(b[0], b[1]) + dist(b[1], b[2]) + dist(b[2], b[3]);
    return Math.min(128, Math.max(2, Math.ceil(l / 2)));
  }
  function sampleBezier(b) {
    const n = sampleCount(b);
    const out = [];
    for (let k = 0; k <= n; k++) out.push(bezierPoint(b, k / n));
    return out;
  }
  const polylineLength = (pts) => pts.slice(1).reduce((s, p, i) => s + dist(pts[i], p), 0);

  // Edges as { start, end } node indices.
  function edges(piece) {
    const n = piece.nodes.length;
    const starts = [];
    for (let i = 0; i < n; i++) if (isVertex(kindOf(piece.nodes[i]))) starts.push(i);
    if (!starts.length && n) starts.push(0);
    return starts.map((s, k) => ({ start: s, end: starts[(k + 1) % starts.length] }));
  }
  // The edge each outline segment (node i → i + 1) belongs to.
  function segmentEdges(piece) {
    const n = piece.nodes.length;
    const out = new Array(n).fill(0);
    edges(piece).forEach((e, k) => {
      let i = e.start;
      do { out[i] = k; i = (i + 1) % n; } while (i !== e.end);
    });
    return out;
  }
  const edgeProps = (piece, k) => piece.nodes[edges(piece)[k].start].edge || {};

  function beziers(piece) {
    const nodes = piece.nodes;
    const n = nodes.length;
    if (n < 2) return [];
    const segEdge = segmentEdges(piece);
    const es = edges(piece);
    const isFold = (seg) => !!(nodes[es[segEdge[seg]].start].edge || {}).fold;
    const foldLine = (k) => [pos(nodes[es[k].start]), pos(nodes[es[k].end])];
    const p = (i) => pos(nodes[((i % n) + n) % n]);
    const out = [];
    for (let i = 0; i < n; i++) {
      const a = p(i), b = p(i + 1);
      if (isFold(i)) { out.push(line(a, b)); continue; }
      const ka = kindOf(nodes[i]), kb = kindOf(nodes[(i + 1) % n]);
      const prev = (i + n - 1) % n, next = (i + 1) % n;
      let p0, p3;
      if (ka === 'corner') p0 = sub(mul(a, 2), b);
      else if (isVertex(ka) && isFold(prev)) p0 = reflect(b, ...foldLine(segEdge[prev]));
      else p0 = p(i - 1);
      if (kb === 'corner') p3 = sub(mul(b, 2), a);
      else if (isVertex(kb) && isFold(next)) p3 = reflect(a, ...foldLine(segEdge[next]));
      else p3 = p(i + 2);
      out.push(segment(p0, a, b, p3));
    }
    return out;
  }

  // Sampled points of each segment.
  const segmentPoints = (piece) => beziers(piece).map(sampleBezier);

  function edgePoints(piece, k, segs = segmentPoints(piece)) {
    const n = piece.nodes.length;
    const e = edges(piece)[k];
    const pts = [];
    let i = e.start;
    do {
      pts.push(...(pts.length ? segs[i].slice(1) : segs[i]));
      i = (i + 1) % n;
    } while (i !== e.end);
    return pts;
  }
  const edgeLength = (piece, k) => polylineLength(edgePoints(piece, k));

  // Closed outline (no repeated point) with the edge of each segment.
  function outline(piece) {
    const segs = segmentPoints(piece);
    const segEdge = segmentEdges(piece);
    const pts = [], owner = [];
    segs.forEach((s, i) => {
      for (let k = 0; k < s.length - 1; k++) { pts.push(s[k]); owner.push(segEdge[i]); }
    });
    return { pts, owner };
  }

  // SVG path data of the outline (exact Béziers), offset by piece.at.
  function pathData(piece) {
    const bs = beziers(piece);
    if (!bs.length) return '';
    const [ox, oy] = piece.at || [0, 0];
    const f = (q) => `${(q[0] + ox).toFixed(2)} ${(q[1] + oy).toFixed(2)}`;
    let d = `M${f(bs[0][0])}`;
    for (const b of bs) d += `C${f(b[1])} ${f(b[2])} ${f(b[3])}`;
    return `${d}Z`;
  }

  function pointInPolygon(p, pts) {
    let inside = false;
    for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
      const [xi, yi] = pts[i], [xj, yj] = pts[j];
      if ((yi > p[1]) !== (yj > p[1]) && p[0] < ((xj - xi) * (p[1] - yi)) / (yj - yi) + xi) inside = !inside;
    }
    return inside;
  }

  // Nearest point on an open polyline: { point, dist, seg, t }.
  function nearestOnPolyline(p, pts) {
    let best = { dist: Infinity };
    for (let i = 0; i + 1 < pts.length; i++) {
      const a = pts[i], b = pts[i + 1];
      const ab = sub(b, a);
      const l2 = ab[0] * ab[0] + ab[1] * ab[1];
      const t = l2 > 0 ? Math.max(0, Math.min(1, ((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2)) : 0;
      const q = add(a, mul(ab, t));
      const d = dist(p, q);
      if (d < best.dist) best = { point: q, dist: d, seg: i, t };
    }
    return best;
  }

  const signedArea = (pts) => pts.reduce((s, p, i) => {
    const q = pts[(i + 1) % pts.length];
    return s + p[0] * q[1] - q[0] * p[1];
  }, 0) / 2;

  // Point at fraction t of a polyline's length, with the direction there.
  function atFraction(pts, t) {
    const total = polylineLength(pts);
    let target = Math.max(0, Math.min(1, t)) * total;
    for (let i = 0; i + 1 < pts.length; i++) {
      const l = dist(pts[i], pts[i + 1]);
      if (target <= l || i + 2 === pts.length) {
        const f = l > 0 ? Math.min(1, target / l) : 0;
        const dir = l > 0 ? mul(sub(pts[i + 1], pts[i]), 1 / l) : [1, 0];
        return { point: add(pts[i], mul(sub(pts[i + 1], pts[i]), f)), dir };
      }
      target -= l;
    }
    return { point: pts[0], dir: [1, 0] };
  }

  return {
    sub, add, mul, dist, isVertex, kindOf, edges, segmentEdges, edgeProps, beziers, segmentPoints,
    edgePoints, edgeLength, outline, pathData, pointInPolygon, nearestOnPolyline, signedArea,
    atFraction, polylineLength, reflect, bezierPoint,
  };
})();
