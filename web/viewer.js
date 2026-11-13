// Minimal WebGL2 viewer: a shaded triangle mesh plus coloured line loops.
// Drag to orbit, wheel or pinch to zoom, double-click or double-tap to
// reset the view.
'use strict';

class Viewer {
  constructor(canvas) {
    this.canvas = canvas;
    const gl = canvas.getContext('webgl2', { antialias: true });
    if (!gl) throw new Error('this browser does not support WebGL2');
    this.gl = gl;
    this.mesh = null;
    this.overlay = null;
    this.lines = [];
    this.marks = null;
    // Dress-form calico.
    this.color = [0.86, 0.83, 0.77];
    this.overlayColor = [0.16, 0.16, 0.18];
    this.resetView();

    const vs = `#version 300 es
      uniform mat4 mvp; uniform mat4 model;
      in vec3 pos; in vec3 nrm; in vec3 col; out vec3 n; out vec3 c;
      void main() { n = mat3(model) * nrm; c = col; gl_Position = mvp * vec4(pos, 1.0); }`;
    const fs = `#version 300 es
      precision mediump float;
      uniform vec3 color; uniform float lit; uniform float useCol; uniform vec3 light; in vec3 n; in vec3 c; out vec4 o;
      void main() {
        vec3 l = light;
        vec3 nn = normalize(n);
        float d = lit > 0.5 ? 0.42 + 0.55 * max(dot(nn, l), 0.0) : 1.0;
        // A soft rim light outlines the form against the background.
        float rim = lit > 0.5 ? 0.18 * pow(1.0 - abs(nn.z), 3.0) : 0.0;
        o = vec4(mix(color, c, useCol) * d + rim, 1.0);
      }`;
    this.prog = this._program(vs, fs);
    this.loc = {
      pos: gl.getAttribLocation(this.prog, 'pos'),
      nrm: gl.getAttribLocation(this.prog, 'nrm'),
      mvp: gl.getUniformLocation(this.prog, 'mvp'),
      model: gl.getUniformLocation(this.prog, 'model'),
      color: gl.getUniformLocation(this.prog, 'color'),
      lit: gl.getUniformLocation(this.prog, 'lit'),
      useCol: gl.getUniformLocation(this.prog, 'useCol'),
      light: gl.getUniformLocation(this.prog, 'light'),
      col: gl.getAttribLocation(this.prog, 'col'),
    };

    this.onClick = null;
    // onPress(e) may return true to claim a drag (then onDrag / onDragEnd
    // get the pointer events instead of orbiting the view).
    this.onPress = null;
    this.onDrag = null;
    this.onDragEnd = null;
    // Pointers currently down (touch can bring several): one turns the view
    // or drags a point; two pinch to zoom.
    const down = new Map();
    let drag = null, travelled = 0, claimed = false, pinch = null, lastTap = 0;
    const spread = () => {
      const [a, b] = [...down.values()];
      return Math.hypot(a[0] - b[0], a[1] - b[1]);
    };
    canvas.addEventListener('pointerdown', (e) => {
      down.set(e.pointerId, [e.clientX, e.clientY]);
      try { canvas.setPointerCapture(e.pointerId); } catch { /* synthetic events */ }
      if (down.size === 2) {
        // A second finger: stop turning or dragging, start pinching.
        if (claimed && this.onDragEnd) this.onDragEnd(e);
        claimed = false;
        drag = null;
        pinch = { spread: spread(), zoom: this.zoom };
        return;
      }
      if (down.size > 2) return;
      drag = [e.clientX, e.clientY];
      travelled = 0;
      claimed = !!(this.onPress && this.onPress(e));
    });
    const up = (e) => {
      if (!down.delete(e.pointerId)) return;
      if (pinch) {
        // The pinch ends when the fingers lift; don't turn on the last one.
        if (down.size === 0) pinch = null;
        drag = null;
        return;
      }
      if (claimed) {
        if (this.onDragEnd) this.onDragEnd(e);
      } else if (drag && travelled < 5) {
        // A tap (not a drag) picks a point; two quick taps reset the view.
        const now = performance.now();
        if (e.pointerType === 'touch' && now - lastTap < 300) {
          lastTap = 0;
          this.resetView();
          this.draw();
        } else {
          lastTap = now;
          if (this.onClick) this.onClick(e);
        }
      }
      drag = null;
      claimed = false;
    };
    canvas.addEventListener('pointerup', up);
    canvas.addEventListener('pointercancel', up);
    canvas.addEventListener('pointermove', (e) => {
      if (!down.has(e.pointerId)) return;
      down.set(e.pointerId, [e.clientX, e.clientY]);
      if (pinch) {
        if (down.size === 2) {
          this.zoom = Math.max(0.3, Math.min(3, pinch.zoom * pinch.spread / Math.max(1, spread())));
          this.draw();
        }
        return;
      }
      if (!drag) return;
      travelled += Math.abs(e.clientX - drag[0]) + Math.abs(e.clientY - drag[1]);
      if (claimed) {
        if (this.onDrag) this.onDrag(e);
        drag = [e.clientX, e.clientY];
        return;
      }
      this.yaw += (e.clientX - drag[0]) * 0.01;
      this.pitch = Math.max(-1.4, Math.min(1.4, this.pitch + (e.clientY - drag[1]) * 0.01));
      drag = [e.clientX, e.clientY];
      this.draw();
    });
    canvas.addEventListener('wheel', (e) => {
      e.preventDefault();
      this.zoom = Math.max(0.3, Math.min(3, this.zoom * Math.exp(e.deltaY * 0.001)));
      this.draw();
    }, { passive: false });
    canvas.addEventListener('dblclick', () => { this.resetView(); this.draw(); });
    new ResizeObserver(() => this.draw()).observe(canvas);
  }

  // The canvas's --view-bg colour as linear 0–1 RGB (light grey if unset).
  static themeColor(canvas) {
    const v = getComputedStyle(canvas).getPropertyValue('--view-bg').trim();
    const m = /^#([0-9a-f]{6})$/i.exec(v);
    if (!m) return [0.97, 0.97, 0.97];
    const n = parseInt(m[1], 16);
    return [(n >> 16) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
  }

  resetView() { this.yaw = 0.5; this.pitch = 0.1; this.zoom = 1; }

  _program(vsrc, fsrc) {
    const gl = this.gl;
    const sh = (type, src) => {
      const s = gl.createShader(type);
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
      return s;
    };
    const p = gl.createProgram();
    gl.attachShader(p, sh(gl.VERTEX_SHADER, vsrc));
    gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fsrc));
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p));
    return p;
  }

  _buffer(data, target = this.gl.ARRAY_BUFFER) {
    const gl = this.gl;
    const b = gl.createBuffer();
    gl.bindBuffer(target, b);
    gl.bufferData(target, data, gl.STATIC_DRAW);
    return b;
  }

  // positions: Float32Array (x, y, z per vertex, mm); triangles: Uint32Array.
  // overlay: optional Uint32Array of triangles drawn in overlayColor (the
  // garment) instead of the base colour.
  // overlayColors: optional Float32Array (r, g, b per vertex) colouring the
  // overlay instead of overlayColor.
  // pieces: optional array of { triangles: Uint32Array, color: [r, g, b] },
  // drawn instead of `overlay`, each in its own colour.
  setMesh(positions, triangles, overlay = null, overlayColors = null, pieces = null) {
    if (pieces && pieces.length) {
      let n = 0;
      pieces.forEach((p) => { n += p.triangles.length; });
      overlay = new Uint32Array(n);
      let at = 0;
      pieces.forEach((p) => { overlay.set(p.triangles, at); at += p.triangles.length; });
    }
    const normals = new Float32Array(positions.length);
    for (let t = 0; t < triangles.length; t += 3) {
      const [a, b, c] = [triangles[t] * 3, triangles[t + 1] * 3, triangles[t + 2] * 3];
      const u = [0, 1, 2].map((k) => positions[b + k] - positions[a + k]);
      const v = [0, 1, 2].map((k) => positions[c + k] - positions[a + k]);
      const n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
      for (const i of [a, b, c]) for (let k = 0; k < 3; k++) normals[i + k] += n[k];
    }
    const lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
    for (let i = 0; i < positions.length; i += 3) {
      for (let k = 0; k < 3; k++) {
        lo[k] = Math.min(lo[k], positions[i + k]);
        hi[k] = Math.max(hi[k], positions[i + k]);
      }
    }
    this.center = [0, 1, 2].map((k) => (lo[k] + hi[k]) / 2);
    this.radius = Math.max(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) / 2;
    const gl = this.gl;
    let base = triangles;
    if (overlay && overlay.length) {
      const key = (a, b, c) => `${a},${b},${c}`;
      const inOverlay = new Set();
      for (let t = 0; t < overlay.length; t += 3) inOverlay.add(key(overlay[t], overlay[t + 1], overlay[t + 2]));
      const keep = [];
      for (let t = 0; t < triangles.length; t += 3) {
        if (!inOverlay.has(key(triangles[t], triangles[t + 1], triangles[t + 2]))) keep.push(triangles[t], triangles[t + 1], triangles[t + 2]);
      }
      base = new Uint32Array(keep);
    }
    this.mesh = {
      pos: this._buffer(positions),
      nrm: this._buffer(normals),
      idx: this._buffer(base, gl.ELEMENT_ARRAY_BUFFER),
      count: base.length,
    };
    this.overlay = overlay && overlay.length
      ? {
        idx: this._buffer(overlay, gl.ELEMENT_ARRAY_BUFFER),
        count: overlay.length,
        col: overlayColors ? this._buffer(overlayColors) : null,
        pieces: pieces && pieces.length
          ? pieces.map((p) => ({ idx: this._buffer(p.triangles, gl.ELEMENT_ARRAY_BUFFER), count: p.triangles.length, color: p.color }))
          : null,
      }
      : null;
    this.draw();
  }

  // Screen position (CSS px, relative to the canvas) of a 3D point.
  project(p) {
    const m = this.mvp, r = this.canvas.getBoundingClientRect();
    if (!m) return null;
    const x = m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12];
    const y = m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13];
    const w = m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15];
    if (w <= 0) return null;
    return { x: (x / w + 1) / 2 * r.width, y: (1 - y / w) / 2 * r.height };
  }

  // Ray from the camera through a pointer event: { origin, dir }.
  ray(e) {
    const r = this.canvas.getBoundingClientRect();
    const nx = ((e.clientX - r.left) / r.width) * 2 - 1;
    const ny = 1 - ((e.clientY - r.top) / r.height) * 2;
    const { eye, x, y, z } = this.camera;
    const h = Math.tan(FOVY / 2), aspect = r.width / r.height;
    const d = [0, 1, 2].map((k) => -z[k] + x[k] * nx * h * aspect + y[k] * ny * h);
    const l = Math.hypot(...d);
    return { origin: eye, dir: d.map((v) => v / l) };
  }

  // Nearest hit of a ray on a mesh: { face, u, v, t } with barycentric u, v
  // of the triangle's second and third corners (Möller–Trumbore), or null.
  static intersect(ray, positions, triangles) {
    const { origin: o, dir: d } = ray;
    let best = null;
    for (let f = 0; f < triangles.length / 3; f++) {
      const a = triangles[3 * f] * 3, b = triangles[3 * f + 1] * 3, c = triangles[3 * f + 2] * 3;
      const e1 = [positions[b] - positions[a], positions[b + 1] - positions[a + 1], positions[b + 2] - positions[a + 2]];
      const e2 = [positions[c] - positions[a], positions[c + 1] - positions[a + 1], positions[c + 2] - positions[a + 2]];
      const p = [d[1] * e2[2] - d[2] * e2[1], d[2] * e2[0] - d[0] * e2[2], d[0] * e2[1] - d[1] * e2[0]];
      const det = e1[0] * p[0] + e1[1] * p[1] + e1[2] * p[2];
      if (Math.abs(det) < 1e-12) continue;
      const inv = 1 / det;
      const s = [o[0] - positions[a], o[1] - positions[a + 1], o[2] - positions[a + 2]];
      const u = (s[0] * p[0] + s[1] * p[1] + s[2] * p[2]) * inv;
      if (u < 0 || u > 1) continue;
      const q = [s[1] * e1[2] - s[2] * e1[1], s[2] * e1[0] - s[0] * e1[2], s[0] * e1[1] - s[1] * e1[0]];
      const v = (d[0] * q[0] + d[1] * q[1] + d[2] * q[2]) * inv;
      if (v < 0 || u + v > 1) continue;
      const t = (e2[0] * q[0] + e2[1] * q[1] + e2[2] * q[2]) * inv;
      if (t > 0 && (!best || t < best.t)) best = { face: f, u, v, t };
    }
    return best;
  }

  // Small crosses at points (Float32Array x, y, z ...), size in mm.
  setMarks(points, color, size = 8) {
    if (!points || !points.length) { this.marks = null; this.draw(); return; }
    const seg = [];
    for (let i = 0; i < points.length; i += 3) {
      for (let k = 0; k < 3; k++) {
        const a = [points[i], points[i + 1], points[i + 2]], b = [...a];
        a[k] -= size; b[k] += size;
        seg.push(...a, ...b);
      }
    }
    this.marks = { buf: this._buffer(new Float32Array(seg)), count: seg.length / 3, color };
    this.draw();
  }

  // loops: array of { points: Float32Array, color: [r, g, b], closed = true }.
  setLines(loops) {
    this.lines = loops.map((l) => ({
      buf: this._buffer(l.points), count: l.points.length / 3, color: l.color, closed: l.closed !== false,
    }));
    this.draw();
  }

  setColor(rgb) { this.color = rgb; this.draw(); }

  draw() {
    const gl = this.gl, c = this.canvas;
    const w = c.clientWidth * devicePixelRatio, h = c.clientHeight * devicePixelRatio;
    if (c.width !== w || c.height !== h) { c.width = w; c.height = h; }
    gl.viewport(0, 0, w, h);
    // Background from the page theme (--view-bg), so it follows dark / light.
    const bg = Viewer.themeColor(this.canvas);
    gl.clearColor(bg[0], bg[1], bg[2], 1);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    if (!this.mesh) return;
    gl.enable(gl.DEPTH_TEST);
    gl.useProgram(this.prog);

    const dist = this.radius * 3.4 * this.zoom;
    const eye = [
      this.center[0] + dist * Math.sin(this.yaw) * Math.cos(this.pitch),
      this.center[1] + dist * Math.sin(this.pitch),
      this.center[2] + dist * Math.cos(this.yaw) * Math.cos(this.pitch),
    ];
    const proj = perspective(FOVY, w / h, dist * 0.05, dist * 4);
    const view = lookAt(eye, this.center, [0, 1, 0]);
    this.camera = { eye, ...basis(eye, this.center, [0, 1, 0]) };
    // Headlight: from the camera, a little above and to the side.
    const { x: cx, y: cy, z: cz } = this.camera;
    const l = [0, 1, 2].map((k) => cz[k] + 0.5 * cy[k] + 0.3 * cx[k]);
    const ll = Math.hypot(...l);
    gl.uniform3fv(this.loc.light, l.map((v) => v / ll));
    this.mvp = mul(proj, view);
    gl.uniformMatrix4fv(this.loc.mvp, false, this.mvp);
    gl.uniformMatrix4fv(this.loc.model, false, [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);

    // Body, pushed back slightly so lines on its surface stay visible.
    gl.enable(gl.POLYGON_OFFSET_FILL);
    gl.polygonOffset(1, 1);
    gl.uniform3fv(this.loc.color, this.color);
    gl.uniform1f(this.loc.lit, 1);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.mesh.pos);
    gl.enableVertexAttribArray(this.loc.pos);
    gl.vertexAttribPointer(this.loc.pos, 3, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.mesh.nrm);
    gl.enableVertexAttribArray(this.loc.nrm);
    gl.vertexAttribPointer(this.loc.nrm, 3, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.mesh.idx);
    gl.drawElements(gl.TRIANGLES, this.mesh.count, gl.UNSIGNED_INT, 0);
    gl.uniform1f(this.loc.useCol, 0);
    if (this.overlay) {
      gl.uniform3fv(this.loc.color, this.overlayColor);
      if (this.overlay.col && this.loc.col >= 0) {
        gl.uniform1f(this.loc.useCol, 1);
        gl.bindBuffer(gl.ARRAY_BUFFER, this.overlay.col);
        gl.enableVertexAttribArray(this.loc.col);
        gl.vertexAttribPointer(this.loc.col, 3, gl.FLOAT, false, 0, 0);
      }
      if (this.overlay.pieces && !this.overlay.col) {
        for (const p of this.overlay.pieces) {
          gl.uniform3fv(this.loc.color, p.color);
          gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, p.idx);
          gl.drawElements(gl.TRIANGLES, p.count, gl.UNSIGNED_INT, 0);
        }
      } else {
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.overlay.idx);
        gl.drawElements(gl.TRIANGLES, this.overlay.count, gl.UNSIGNED_INT, 0);
      }
      if (this.loc.col >= 0) gl.disableVertexAttribArray(this.loc.col);
      gl.uniform1f(this.loc.useCol, 0);
    }
    gl.disable(gl.POLYGON_OFFSET_FILL);

    gl.uniform1f(this.loc.lit, 0);
    gl.disableVertexAttribArray(this.loc.nrm);
    gl.vertexAttrib3f(this.loc.nrm, 0, 0, 1);
    for (const l of this.lines) {
      gl.uniform3fv(this.loc.color, l.color);
      gl.bindBuffer(gl.ARRAY_BUFFER, l.buf);
      gl.vertexAttribPointer(this.loc.pos, 3, gl.FLOAT, false, 0, 0);
      gl.drawArrays(l.closed ? gl.LINE_LOOP : gl.LINE_STRIP, 0, l.count);
    }
    if (this.marks) {
      gl.uniform3fv(this.loc.color, this.marks.color);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.marks.buf);
      gl.vertexAttribPointer(this.loc.pos, 3, gl.FLOAT, false, 0, 0);
      gl.drawArrays(gl.LINES, 0, this.marks.count);
    }
  }
}

const FOVY = 0.6;

// Camera axes for lookAt: x right, y up, z towards the eye.
function basis(eye, at, up) {
  const norm = (a) => { const l = Math.hypot(...a); return a.map((v) => v / l); };
  const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  const z = norm(eye.map((v, i) => v - at[i]));
  const x = norm(cross(up, z));
  return { x, y: cross(z, x), z };
}

function perspective(fovy, aspect, near, far) {
  const f = 1 / Math.tan(fovy / 2), nf = 1 / (near - far);
  return [f / aspect, 0, 0, 0, 0, f, 0, 0, 0, 0, (far + near) * nf, -1, 0, 0, 2 * far * near * nf, 0];
}

function lookAt(eye, at, up) {
  const sub = (a, b) => a.map((x, i) => x - b[i]);
  const norm = (a) => { const l = Math.hypot(...a); return a.map((x) => x / l); };
  const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  const z = norm(sub(eye, at)), x = norm(cross(up, z)), y = cross(z, x);
  return [x[0], y[0], z[0], 0, x[1], y[1], z[1], 0, x[2], y[2], z[2], 0, -dot(x, eye), -dot(y, eye), -dot(z, eye), 1];
}

// Column-major 4x4 product a * b.
function mul(a, b) {
  const o = new Array(16).fill(0);
  for (let c = 0; c < 4; c++) for (let r = 0; r < 4; r++) for (let k = 0; k < 4; k++) o[c * 4 + r] += a[k * 4 + r] * b[c * 4 + k];
  return o;
}
