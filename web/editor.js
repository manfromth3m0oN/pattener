// The flat design editor: pieces drawn at body size on an SVG canvas.
//
// World coordinates are millimetres with y up (height above the floor);
// everything is drawn in screen pixels, so lines keep their width and text
// stays upright at any zoom. The design object belongs to app.js: the
// editor changes it in place and reports through callbacks:
//   onChange(commit, message)  commit = a finished edit (for undo)
//   onSelect(selection)        { kind: 'piece'|'edge'|'node'|'seam', ... } or null
//   colorOf(piece)             fill colour of a piece (its sheet)
'use strict';

class Editor {
  constructor(svg, callbacks) {
    this.svg = svg;
    this.cb = callbacks;
    this.design = { pieces: [], seams: [] };
    this.guides = [];
    this.showGuides = true;
    this.snap = true;
    this.sel = null;
    this.tool = 'select';
    this.k = 0.5; this.tx = 100; this.ty = 600;  // screen = (x k + tx, −y k + ty)
    this.pending = null;   // tool state: draw points, first split point, first seam edge
    this.pointers = new Map();
    this.drag = null;
    this.hover = null;
    this.cache = new Map();
    this.bind();
    new ResizeObserver(() => this.render()).observe(svg);
  }

  // ---- coordinates ---------------------------------------------------------
  toScreen([x, y]) { return [x * this.k + this.tx, -y * this.k + this.ty]; }
  toWorld([sx, sy]) { return [(sx - this.tx) / this.k, -(sy - this.ty) / this.k]; }
  eventPoint(e) {
    const r = this.svg.getBoundingClientRect();
    return [e.clientX - r.left, e.clientY - r.top];
  }
  piece(id) { return this.design.pieces.find((p) => p.id === id); }
  at(p) { return p.at || [0, 0]; }

  // Sampled geometry of a piece in world coordinates, cached by content.
  geometry(p) {
    const key = JSON.stringify([p.nodes, p.at]);
    const c = this.cache.get(p.id);
    if (c && c.key === key) return c;
    const [ox, oy] = this.at(p);
    const segs = Geo.segmentPoints(p).map((s) => s.map(([x, y]) => [x + ox, y + oy]));
    const es = Geo.edges(p);
    const edges = es.map((e, k) => Geo.edgePoints(p, k, segs));
    const outline = [];
    segs.forEach((s) => outline.push(...s.slice(0, -1)));
    const g = { key, segs, edges, outline, es };
    this.cache.set(p.id, g);
    return g;
  }

  setDesign(d) { this.design = d; this.cache.clear(); if (this.sel && !this.selValid()) this.select(null); this.render(); }
  setGuides(g) { this.guides = g || []; this.render(); }
  selValid() {
    const s = this.sel;
    if (s.kind === 'seam') return this.design.seams.some((x) => x.id === s.seam);
    const p = this.piece(s.piece);
    if (!p) return false;
    if (s.kind === 'node') return p.nodes.some((n) => n.id === s.node);
    if (s.kind === 'edge') return Geo.edges(p).some((e) => p.nodes[e.start].id === s.node);
    return true;
  }
  select(sel) {
    this.sel = sel;
    this.cb.onSelect(sel);
    this.render();
  }
  setTool(t) {
    if (this.cb.onTool) this.cb.onTool(t);
    this.tool = t;
    this.pending = null;
    this.svg.dataset.tool = t;
    const hints = {
      select: 'Click to select · drag points or pieces · double-click an edge to add a point · drag the background to pan, scroll to zoom',
      add: 'Click on an edge to add a point there',
      draw: 'Click the corners of the new piece; click the first one again (or press Enter) to close it',
      split: 'Click a point on a piece\'s outline, then a second one: the piece is cut along the line and the halves seamed',
      seam: 'Click an edge, then the edge it is sewn to (the same edge twice joins a pair piece to its mirror image)',
    };
    this.cb.onHint(hints[t] || '');
    this.render();
  }

  fit() {
    const pts = [];
    for (const p of this.design.pieces) pts.push(...this.geometry(p).outline);
    const r = this.svg.getBoundingClientRect();
    if (!pts.length || !r.width) return;
    const xs = pts.map((q) => q[0]), ys = pts.map((q) => q[1]);
    const [x0, x1, y0, y1] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
    const m = 40;
    this.k = Math.min((r.width - 2 * m) / Math.max(1, x1 - x0), (r.height - 2 * m) / Math.max(1, y1 - y0));
    this.tx = r.width / 2 - ((x0 + x1) / 2) * this.k;
    this.ty = r.height / 2 + ((y0 + y1) / 2) * this.k;
    this.render();
  }
  zoomAt([sx, sy], f) {
    const w = this.toWorld([sx, sy]);
    this.k = Math.min(20, Math.max(0.05, this.k * f));
    this.tx = sx - w[0] * this.k;
    this.ty = sy + w[1] * this.k;
    this.render();
  }

  // The arrow up the body through the middle of a piece, in world
  // coordinates: centre, unit direction, tail, head (as the pattern's).
  arrow(p) {
    const o = this.geometry(p).outline;
    const xs = o.map((q) => q[0]), ys = o.map((q) => q[1]);
    const c = [(Math.min(...xs) + Math.max(...xs)) / 2, (Math.min(...ys) + Math.max(...ys)) / 2];
    const t = ((p.up_deg || 0) * Math.PI) / 180;
    const u = [-Math.sin(t), Math.cos(t)];
    let lo = Infinity, hi = -Infinity;
    for (const q of o) { const d = (q[0] - c[0]) * u[0] + (q[1] - c[1]) * u[1]; lo = Math.min(lo, d); hi = Math.max(hi, d); }
    const half = Math.min(80, Math.max(15, 0.2 * (hi - lo)));
    return { c, u, tail: [c[0] - half * u[0], c[1] - half * u[1]], head: [c[0] + half * u[0], c[1] + half * u[1]] };
  }

  // ---- hit testing ---------------------------------------------------------
  // The nearest node (of the active piece), edge, or piece under a screen point.
  hit(sp, { nodes = true } = {}) {
    const w = this.toWorld(sp);
    const tol = 8 / this.k;
    const active = this.sel && this.sel.piece ? this.piece(this.sel.piece) : null;
    if (nodes && active && active.nodes.length >= 3) {
      const h = this.toScreen(this.arrow(active).head);
      if (Math.hypot(h[0] - sp[0], h[1] - sp[1]) <= 9) return { kind: 'up', piece: active.id };
    }
    if (nodes && active) {
      const [ox, oy] = this.at(active);
      let best = null;
      for (const n of active.nodes) {
        const d = Math.hypot(n.x + ox - w[0], n.y + oy - w[1]);
        if (d <= tol && (!best || d < best.d)) best = { d, node: n };
      }
      if (best) return { kind: 'node', piece: active.id, node: best.node.id };
    }
    let best = null;
    for (const p of this.design.pieces) {
      const g = this.geometry(p);
      g.edges.forEach((pts, k) => {
        const h = Geo.nearestOnPolyline(w, pts);
        if (h.dist <= 6 / this.k && (!best || h.dist < best.dist)) {
          best = { dist: h.dist, piece: p.id, node: p.nodes[g.es[k].start].id, edge: k, point: h.point };
        }
      });
    }
    if (best) return { kind: 'edge', piece: best.piece, node: best.node, edge: best.edge, point: best.point };
    for (const p of [...this.design.pieces].reverse()) {
      if (Geo.pointInPolygon(w, this.geometry(p).outline)) return { kind: 'piece', piece: p.id };
    }
    return null;
  }

  // Outline segment (node i → i + 1) of a piece nearest a world point.
  segmentAt(p, w) {
    const g = this.geometry(p);
    let best = null;
    g.segs.forEach((s, i) => {
      const h = Geo.nearestOnPolyline(w, s);
      if (!best || h.dist < best.dist) best = { ...h, i };
    });
    return best;
  }

  // Insert a node into a piece at a world point on its outline; returns it.
  insertNode(p, w, kind) {
    const s = this.segmentAt(p, w);
    const [ox, oy] = this.at(p);
    const near = [p.nodes[s.i], p.nodes[(s.i + 1) % p.nodes.length]]
      .find((n) => Math.hypot(n.x + ox - s.point[0], n.y + oy - s.point[1]) < 4 / this.k);
    if (near) {
      if (kind === 'corner' && near.kind === 'smooth') {
        const k = Geo.segmentEdges(p)[s.i];
        near.edge = { ...Geo.edgeProps(p, k) };
        near.kind = 'corner';
      }
      return near;
    }
    const n = { id: newId('n'), x: round(s.point[0] - ox), y: round(s.point[1] - oy), kind };
    if (kind !== 'smooth') {
      const k = Geo.segmentEdges(p)[s.i];
      n.edge = { ...Geo.edgeProps(p, k) };
    }
    p.nodes.splice(s.i + 1, 0, n);
    this.cache.delete(p.id);
    return n;
  }

  // ---- edits -----------------------------------------------------------------
  // Cut piece p along the straight line between nodes a and b (both on it).
  split(p, a, b) {
    let i = p.nodes.indexOf(a), j = p.nodes.indexOf(b);
    if (i === j || i < 0 || j < 0) return false;
    if (i > j) [i, j] = [j, i];
    const n = p.nodes.length;
    if (j - i === 1 || (i === 0 && j === n - 1)) return false; // already neighbours: nothing to cut off
    const [ni, nj] = [p.nodes[i], p.nodes[j]];
    for (const x of [ni, nj]) if (x.kind === 'smooth') { x.kind = 'corner'; }
    const copy = (x) => ({ ...JSON.parse(JSON.stringify(x)), id: newId('n'), notch: false, edge: { name: 'split' } });
    const cutA = copy(nj);   // A: ni … nj', closing edge nj' → ni
    const cutB = copy(ni);   // B: nj … ni', closing edge ni' → nj
    cutA.kind = 'corner'; cutB.kind = 'corner';
    const aNodes = [...p.nodes.slice(i, j), cutA];
    const bNodes = [...p.nodes.slice(j), ...p.nodes.slice(0, i), cutB];
    const q = { ...JSON.parse(JSON.stringify(p)), id: newId('p'), name: `${p.name} 2`, nodes: bNodes };
    p.nodes = aNodes;
    // A piece cut on the fold stood for both sides: a part without the fold
    // is now cut as a mirrored pair.
    const hasFold = (x) => Geo.edges(x).some((e) => (x.nodes[e.start].edge || {}).fold);
    if (hasFold({ nodes: [...aNodes, ...bNodes] })) {
      for (const x of [p, q]) if (!hasFold(x)) x.pair = true;
    }
    // Seams on edges that moved to the new piece follow them.
    const moved = new Set(bNodes.map((x) => x.id));
    for (const s of this.design.seams) {
      for (const r of [s.a, s.b]) if (r.piece === p.id && moved.has(r.node)) r.piece = q.id;
    }
    this.design.pieces.splice(this.design.pieces.indexOf(p) + 1, 0, q);
    this.design.seams.push({
      id: newId('s'), name: `${p.name} split`, a: { piece: p.id, node: cutA.id }, b: { piece: q.id, node: cutB.id },
      reverse: true, finish: 'glued', lap: 'a',
    });
    this.cache.clear();
    return q;
  }

  // Join two edges with a seam, pairing the ends that sit at nearer heights.
  join(a, b) {
    const used = (r) => this.design.seams.some((s) => [s.a, s.b].some((x) => x.piece === r.piece && x.node === r.node));
    if (used(a) || used(b)) return 'That edge is already in a seam; delete the seam first.';
    const ends = (r) => {
      const p = this.piece(r.piece);
      const g = this.geometry(p);
      const k = g.es.findIndex((e) => p.nodes[e.start].id === r.node);
      const pts = g.edges[k];
      return [pts[0], pts[pts.length - 1]];
    };
    let reverse = false;
    if (a.piece !== b.piece || a.node !== b.node) {
      const [a0, a1] = ends(a), [b0, b1] = ends(b);
      const same = Math.abs(a0[1] - b0[1]) + Math.abs(a1[1] - b1[1]);
      const crossed = Math.abs(a0[1] - b1[1]) + Math.abs(a1[1] - b0[1]);
      reverse = crossed <= same;
    }
    const s = { id: newId('s'), name: `seam ${this.design.seams.length + 1}`, a, b, reverse, finish: 'glued', lap: 'a' };
    this.design.seams.push(s);
    return s;
  }

  // ---- events ----------------------------------------------------------------
  bind() {
    const svg = this.svg;
    svg.addEventListener('wheel', (e) => {
      e.preventDefault();
      this.zoomAt(this.eventPoint(e), Math.exp(-e.deltaY * (e.deltaMode ? 0.05 : 0.0015)));
    }, { passive: false });
    svg.addEventListener('pointerdown', (e) => this.down(e));
    svg.addEventListener('pointermove', (e) => this.move(e));
    svg.addEventListener('pointerup', (e) => this.up(e));
    svg.addEventListener('pointercancel', (e) => this.up(e));
    svg.addEventListener('dblclick', (e) => {
      if (this.tool !== 'select') return;
      const h = this.hit(this.eventPoint(e), { nodes: false });
      if (h && h.kind === 'edge') {
        const p = this.piece(h.piece);
        const n = this.insertNode(p, h.point, 'smooth');
        this.select({ kind: 'node', piece: p.id, node: n.id });
        this.cb.onChange(true, 'Point added');
      }
    });
  }

  down(e) {
    this.svg.focus({ preventScroll: true });
    this.svg.setPointerCapture(e.pointerId);
    const sp = this.eventPoint(e);
    this.pointers.set(e.pointerId, sp);
    if (this.pointers.size === 2) {
      // Pinch: zoom and pan with two fingers; drop any drag.
      const [a, b] = [...this.pointers.values()];
      this.drag = { kind: 'pinch', d: Math.hypot(a[0] - b[0], a[1] - b[1]), c: [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2] };
      return;
    }
    if (e.button === 1 || e.button === 2) { this.drag = { kind: 'pan', last: sp }; return; }
    const w = this.toWorld(sp);
    if (this.tool === 'draw') return this.drawClick(w, sp);
    if (this.tool === 'add' || this.tool === 'split' || this.tool === 'seam') {
      const h = this.hit(sp, { nodes: false });
      if (!h || h.kind !== 'edge') { this.drag = { kind: 'pan', last: sp, moved: false }; return; }
      return this[`${this.tool}Click`](h);
    }
    const h = this.hit(sp);
    if (!h) {
      this.drag = { kind: 'pan', last: sp, moved: false, deselect: true };
      return;
    }
    if (h.kind === 'up') {
      this.drag = { kind: 'up', piece: this.piece(h.piece), moved: false };
      return;
    }
    if (h.kind === 'node') {
      const p = this.piece(h.piece);
      this.select({ kind: 'node', piece: h.piece, node: h.node });
      this.drag = { kind: 'node', node: p.nodes.find((n) => n.id === h.node), piece: p, moved: false };
    } else {
      this.select(h.kind === 'edge' ? { kind: 'edge', piece: h.piece, node: h.node } : { kind: 'piece', piece: h.piece });
      const p = this.piece(h.piece);
      this.drag = { kind: 'piece', piece: p, start: w, at0: [...this.at(p)], moved: false };
    }
  }

  move(e) {
    const sp = this.eventPoint(e);
    if (this.pointers.has(e.pointerId)) this.pointers.set(e.pointerId, sp);
    const d = this.drag;
    if (d && d.kind === 'pinch' && this.pointers.size === 2) {
      const [a, b] = [...this.pointers.values()];
      const dist = Math.hypot(a[0] - b[0], a[1] - b[1]);
      const c = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
      this.tx += c[0] - d.c[0];
      this.ty += c[1] - d.c[1];
      this.zoomAt(c, dist / d.d);
      d.d = dist; d.c = c;
      return;
    }
    if (!d) {
      this.hover = this.tool === 'draw' ? this.toWorld(sp) : (this.tool === 'select' ? null : this.hit(sp, { nodes: false }));
      if (this.tool !== 'select' || this.pending) this.render();
      return;
    }
    const w = this.toWorld(sp);
    if (d.kind === 'pan') {
      if (Math.hypot(sp[0] - d.last[0], sp[1] - d.last[1]) > 0) d.moved = true;
      this.tx += sp[0] - d.last[0];
      this.ty += sp[1] - d.last[1];
      d.last = sp;
      this.render();
    } else if (d.kind === 'node') {
      const [ox, oy] = this.at(d.piece);
      let [x, y] = [w[0] - ox, w[1] - oy];
      if (this.snap) [x, y] = this.snapPoint(x, y, oy);
      d.node.x = x; d.node.y = y; d.moved = true;
      this.cache.delete(d.piece.id);
      this.cb.onChange(false);
      this.render();
    } else if (d.kind === 'up') {
      // Up the body points from the middle of the piece to the pointer.
      const { c } = this.arrow(d.piece);
      let deg = (Math.atan2(-(w[0] - c[0]), w[1] - c[1]) * 180) / Math.PI;
      if (this.snap) deg = Math.round(deg / 15) * 15;
      deg = Math.round(deg * 10) / 10;
      if (deg <= -180) deg += 360;
      if (deg) d.piece.up_deg = deg; else delete d.piece.up_deg;
      d.moved = true;
      this.cb.onChange(false);
      this.render();
    } else if (d.kind === 'piece') {
      let dx = w[0] - d.start[0], dy = w[1] - d.start[1];
      if (!d.moved && Math.hypot(dx, dy) * this.k < 3) return;
      if (this.snap) { dx = Math.round(dx); dy = Math.round(dy); }
      d.piece.at = [d.at0[0] + dx, d.at0[1] + dy];
      d.moved = true;
      this.cache.delete(d.piece.id);
      this.render();
    }
  }

  up(e) {
    this.pointers.delete(e.pointerId);
    const d = this.drag;
    if (d && d.kind === 'pinch') { if (this.pointers.size === 0) this.drag = null; return; }
    this.drag = null;
    if (!d) return;
    if (d.kind === 'pan' && d.deselect && !d.moved) this.select(null);
    if ((d.kind === 'node' || d.kind === 'piece' || d.kind === 'up') && d.moved) {
      const what = { node: 'Point moved', piece: 'Piece moved', up: 'Up the body turned' }[d.kind];
      this.cb.onChange(true, what);
    }
    this.render();
  }

  // Whole millimetres; the body's levels and the centre line within 6 px.
  snapPoint(x, y, oy) {
    const t = 6 / this.k;
    let sy = Math.round(y);
    if (this.showGuides) {
      for (const [, gy] of this.guides) if (Math.abs(y + oy - gy) < t) sy = gy - oy;
    }
    const sx = Math.abs(x) < t ? 0 : Math.round(x);
    return [sx, sy];
  }

  addClick(h) {
    const p = this.piece(h.piece);
    const n = this.insertNode(p, h.point, 'smooth');
    this.select({ kind: 'node', piece: p.id, node: n.id });
    this.cb.onChange(true, 'Point added');
  }

  splitClick(h) {
    if (!this.pending) {
      this.pending = { piece: h.piece, point: h.point };
      this.cb.onHint('Now click the second point on the same piece\'s outline (Esc to cancel)');
      this.render();
      return;
    }
    if (h.piece !== this.pending.piece) { this.cb.onHint('Pick the second point on the same piece'); return; }
    const p = this.piece(h.piece);
    const a = this.insertNode(p, this.pending.point, 'corner');
    const b = this.insertNode(p, h.point, 'corner');
    const q = this.split(p, a, b);
    this.pending = null;
    if (!q) {
      this.cb.onChange(true, 'Those points are next to each other: nothing to cut off');
    } else {
      this.select({ kind: 'piece', piece: q.id });
      this.cb.onChange(true, `Split ${p.name}: the new seam joins the two parts`);
    }
    this.setTool('select');
  }

  seamClick(h) {
    const r = { piece: h.piece, node: h.node };
    if (!this.pending) {
      this.pending = r;
      this.cb.onHint('Now click the edge it is sewn to (the same edge again to join a pair piece to its mirror image)');
      this.render();
      return;
    }
    const res = this.join(this.pending, r);
    this.pending = null;
    if (typeof res === 'string') { this.cb.onHint(res); this.render(); return; }
    this.select({ kind: 'seam', seam: res.id });
    this.cb.onChange(true, 'Seam added');
    this.setTool('select');
  }

  drawClick(w, sp) {
    const pts = (this.pending && this.pending.points) || [];
    if (pts.length >= 3) {
      const f = this.toScreen(pts[0]);
      if (Math.hypot(f[0] - sp[0], f[1] - sp[1]) < 10) return this.finishDraw();
    }
    const q = this.snap ? [Math.round(w[0]), Math.round(w[1])] : w;
    this.pending = { points: [...pts, q] };
    this.render();
  }
  finishDraw() {
    const pts = (this.pending && this.pending.points) || [];
    if (pts.length < 3) return;
    // Counter-clockwise, as drafted pieces are.
    const ordered = Geo.signedArea(pts) < 0 ? [...pts].reverse() : pts;
    const n = this.design.pieces.length + 1;
    const p = {
      id: newId('p'), name: `piece ${n}`, at: [0, 0], pair: false,
      nodes: ordered.map(([x, y]) => ({ id: newId('n'), x, y, kind: 'corner' })),
    };
    this.design.pieces.push(p);
    this.pending = null;
    this.setTool('select');
    this.select({ kind: 'piece', piece: p.id });
    this.cb.onChange(true, 'Piece added: drag its arrow so it points up the body, if it was drawn turned');
  }
  cancel() {
    if (this.pending) { this.pending = null; this.setTool(this.tool); return true; }
    if (this.tool !== 'select') { this.setTool('select'); return true; }
    return false;
  }

  // ---- drawing ------------------------------------------------------------------
  render() {
    const r = this.svg.getBoundingClientRect();
    const W = r.width, H = r.height;
    if (!W) return;
    const out = [];
    const S = (q) => this.toScreen(q);
    const f = (v) => v.toFixed(1);
    const poly = (pts) => pts.map((q, i) => { const s = S(q); return `${i ? 'L' : 'M'}${f(s[0])} ${f(s[1])}`; }).join('');

    // Grid: 10 mm (if dense enough), 100 mm stronger.
    const w0 = this.toWorld([0, H]), w1 = this.toWorld([W, 0]);
    for (const [step, cls] of [[10, 'grid-minor'], [100, 'grid-major']]) {
      if (step * this.k < 6) continue;
      let d = '';
      for (let x = Math.ceil(w0[0] / step) * step; x <= w1[0]; x += step) { const sx = f(S([x, 0])[0]); d += `M${sx} 0V${H}`; }
      for (let y = Math.ceil(w0[1] / step) * step; y <= w1[1]; y += step) { const sy = f(S([0, y])[1]); d += `M0 ${sy}H${W}`; }
      out.push(`<path class="${cls}" d="${d}"/>`);
    }
    if (this.showGuides) {
      for (const [name, y] of this.guides) {
        const sy = f(S([0, y])[1]);
        out.push(`<path class="guide" d="M0 ${sy}H${W}"/><text class="guide-label" x="6" y="${sy - 3}">${esc(name)}</text>`);
      }
    }

    const sel = this.sel || {};
    const seamColor = (i) => SEAM_COLORS[i % SEAM_COLORS.length];
    const seamOf = new Map();
    this.design.seams.forEach((s, i) => {
      for (const ref of [s.a, s.b]) seamOf.set(`${ref.piece}/${ref.node}`, i);
    });

    for (const p of this.design.pieces) {
      const g = this.geometry(p);
      const [ox, oy] = this.at(p);
      const active = sel.piece === p.id;
      const bs = Geo.beziers(p);
      if (!bs.length) continue;
      let d = '';
      bs.forEach((b, i) => {
        const pts = b.map(([x, y]) => S([x + ox, y + oy]));
        if (!i) d += `M${f(pts[0][0])} ${f(pts[0][1])}`;
        d += `C${pts.slice(1).map((q) => `${f(q[0])} ${f(q[1])}`).join(' ')}`;
      });
      out.push(`<path class="piece${active ? ' active' : ''}" d="${d}Z" style="--fill:${this.cb.colorOf(p)}"/>`);
      // Edges: folds, seams, the selected edge.
      g.es.forEach((e, k) => {
        const id = p.nodes[e.start].id;
        const props = p.nodes[e.start].edge || {};
        const pts = g.edges[k];
        const si = seamOf.get(`${p.id}/${id}`);
        if (props.fold) out.push(`<path class="fold" d="${poly(pts)}"/>`);
        if (si !== undefined) {
          const s = this.design.seams[si];
          const hl = sel.kind === 'seam' && sel.seam === s.id;
          out.push(`<path class="seam-edge${hl ? ' hl' : ''}" d="${poly(pts)}" style="--c:${seamColor(si)}"/>`);
          // A dot where the ends that meet are: side a's start, and side b's start or end.
          const isB = s.b.piece === p.id && s.b.node === id && !(s.a.piece === p.id && s.a.node === id);
          const end = isB && s.reverse ? pts[pts.length - 1] : pts[0];
          const q = S(end);
          out.push(`<circle class="seam-dot" cx="${f(q[0])}" cy="${f(q[1])}" r="3.5" style="--c:${seamColor(si)}"/>`);
        }
        if (sel.kind === 'edge' && sel.piece === p.id && sel.node === id) out.push(`<path class="edge-sel" d="${poly(pts)}"/>`);
        if (this.hover && this.hover.kind === 'edge' && this.hover.piece === p.id && this.hover.node === id) {
          out.push(`<path class="edge-hover" d="${poly(pts)}"/>`);
        }
      });
    }
    // Nodes of the active piece.
    const ap = sel.piece && this.piece(sel.piece);
    if (ap) {
      const [ox, oy] = this.at(ap);
      for (const n of ap.nodes) {
        const [x, y] = S([n.x + ox, n.y + oy]);
        const cls = `node ${n.kind || 'corner'}${sel.kind === 'node' && sel.node === n.id ? ' sel' : ''}`;
        if ((n.kind || 'corner') === 'corner') out.push(`<rect class="${cls}" x="${f(x - 4)}" y="${f(y - 4)}" width="8" height="8"/>`);
        else if (n.kind === 'joint') out.push(`<path class="${cls}" d="M${f(x)} ${f(y - 5.5)}L${f(x + 5.5)} ${f(y)}L${f(x)} ${f(y + 5.5)}L${f(x - 5.5)} ${f(y)}Z"/>`);
        else out.push(`<circle class="${cls}" cx="${f(x)}" cy="${f(y)}" r="4.2"/>`);
        if (n.notch) out.push(`<circle class="notch" cx="${f(x)}" cy="${f(y)}" r="8"/>`);
      }
    }
    // Labels: piece names, seam names, folds.
    for (const p of this.design.pieces) {
      const g = this.geometry(p);
      if (!g.outline.length) continue;
      const xs = g.outline.map((q) => q[0]), ys = g.outline.map((q) => q[1]);
      const c = S([(Math.min(...xs) + Math.max(...xs)) / 2, (Math.min(...ys) + Math.max(...ys)) / 2]);
      const cut = p.pair ? ' ×2' : '';
      // The arrow up the body, with a handle to turn it on the active piece.
      const a = this.arrow(p);
      const [t, hd] = [S(a.tail), S(a.head)];
      const dx = hd[0] - t[0], dy = hd[1] - t[1], len = Math.hypot(dx, dy) || 1;
      const [ux, uy] = [dx / len, dy / len];
      const barb = (s) => `${f(hd[0] - 9 * ux + s * 5 * uy)} ${f(hd[1] - 9 * uy - s * 5 * ux)}`;
      out.push(`<path class="up-arrow" d="M${f(t[0])} ${f(t[1])}L${f(hd[0])} ${f(hd[1])}M${barb(1)}L${f(hd[0])} ${f(hd[1])}L${barb(-1)}"/>`);
      if (sel.piece === p.id) out.push(`<circle class="up-handle" cx="${f(hd[0])}" cy="${f(hd[1])}" r="6"><title>Drag to turn: which way is up the body</title></circle>`);
      // The name beside the arrow.
      const lx = c[0] + uy * 14, ly = c[1] - ux * 14;
      out.push(`<text class="piece-label" x="${f(lx)}" y="${f(ly)}">${esc(p.name)}${cut}</text>`);
      g.es.forEach((e, k) => {
        const props = p.nodes[e.start].edge || {};
        const si = seamOf.get(`${p.id}/${p.nodes[e.start].id}`);
        let text = null;
        if (props.fold) text = 'fold';
        else if (si !== undefined) text = this.design.seams[si].name || 'seam';
        if (!text || this.k < 0.25) return;
        const m = Geo.atFraction(g.edges[k], 0.5);
        const q = S(m.point);
        // Inside the piece: left of a counter-clockwise outline.
        const ccw = Geo.signedArea(g.outline) > 0 ? 1 : -1;
        const nx = -m.dir[1] * ccw, ny = m.dir[0] * ccw;
        out.push(`<text class="edge-label" x="${f(q[0] + nx * 14)}" y="${f(q[1] - ny * 14)}">${esc(text)}</text>`);
      });
    }
    // Tool previews.
    if (this.tool === 'draw' && this.pending) {
      const pts = [...this.pending.points];
      if (this.hover && Array.isArray(this.hover)) pts.push(this.hover);
      out.push(`<path class="preview" d="${poly(pts)}"/>`);
      for (const q of this.pending.points) { const s = S(q); out.push(`<circle class="node corner" cx="${f(s[0])}" cy="${f(s[1])}" r="3.5"/>`); }
    }
    if (this.tool === 'split' && this.pending) {
      const a = S(this.pending.point);
      out.push(`<circle class="node sel" cx="${f(a[0])}" cy="${f(a[1])}" r="4.5"/>`);
      if (this.hover && this.hover.kind === 'edge') {
        const b = S(this.hover.point);
        out.push(`<path class="preview" d="M${f(a[0])} ${f(a[1])}L${f(b[0])} ${f(b[1])}"/>`);
      }
    }
    if (this.tool === 'seam' && this.pending) {
      const p = this.piece(this.pending.piece);
      const g = p && this.geometry(p);
      const k = g ? g.es.findIndex((e) => p.nodes[e.start].id === this.pending.node) : -1;
      if (k >= 0) out.push(`<path class="edge-sel" d="${poly(g.edges[k])}"/>`);
    }
    // Scale bar: 100 mm.
    const bar = 100 * this.k;
    if (bar > 20 && bar < W / 2) {
      out.push(`<path class="scale" d="M${W - 16 - bar} ${H - 14}h${bar}"/><text class="scale-label" x="${W - 16 - bar / 2}" y="${H - 20}">10 cm</text>`);
    }
    this.svg.innerHTML = out.join('');
  }
}

const SEAM_COLORS = ['#3d9df0', '#f0a24b', '#55c278', '#c47de8', '#e8d34d', '#4dd6d0', '#ff7aa8', '#9fa8ff'];
const esc = (t) => String(t).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const round = (v) => Math.round(v * 10) / 10;
function newId(prefix) {
  return `${prefix}${Date.now().toString(36).slice(-4)}${Math.random().toString(36).slice(2, 7)}`;
}
