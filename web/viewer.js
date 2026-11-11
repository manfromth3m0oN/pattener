// Minimal WebGL2 viewer: a shaded triangle mesh plus coloured line loops.
// Drag to orbit, wheel to zoom, double-click to reset the view.
'use strict';

class Viewer {
  constructor(canvas) {
    this.canvas = canvas;
    const gl = canvas.getContext('webgl2', { antialias: true });
    if (!gl) throw new Error('this browser does not support WebGL2');
    this.gl = gl;
    this.mesh = null;
    this.lines = [];
    this.color = [0.82, 0.78, 0.74];
    this.resetView();

    const vs = `#version 300 es
      uniform mat4 mvp; uniform mat4 model;
      in vec3 pos; in vec3 nrm; out vec3 n;
      void main() { n = mat3(model) * nrm; gl_Position = mvp * vec4(pos, 1.0); }`;
    const fs = `#version 300 es
      precision mediump float;
      uniform vec3 color; uniform float lit; in vec3 n; out vec4 o;
      void main() {
        vec3 l = normalize(vec3(0.4, 0.6, 1.0));
        float d = lit > 0.5 ? 0.35 + 0.65 * max(dot(normalize(n), l), 0.0) : 1.0;
        o = vec4(color * d, 1.0);
      }`;
    this.prog = this._program(vs, fs);
    this.loc = {
      pos: gl.getAttribLocation(this.prog, 'pos'),
      nrm: gl.getAttribLocation(this.prog, 'nrm'),
      mvp: gl.getUniformLocation(this.prog, 'mvp'),
      model: gl.getUniformLocation(this.prog, 'model'),
      color: gl.getUniformLocation(this.prog, 'color'),
      lit: gl.getUniformLocation(this.prog, 'lit'),
    };

    let drag = null;
    canvas.addEventListener('pointerdown', (e) => { drag = [e.clientX, e.clientY]; canvas.setPointerCapture(e.pointerId); });
    canvas.addEventListener('pointerup', () => { drag = null; });
    canvas.addEventListener('pointermove', (e) => {
      if (!drag) return;
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
  setMesh(positions, triangles) {
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
    this.mesh = {
      pos: this._buffer(positions),
      nrm: this._buffer(normals),
      idx: this._buffer(triangles, gl.ELEMENT_ARRAY_BUFFER),
      count: triangles.length,
    };
    this.draw();
  }

  // loops: array of { points: Float32Array, color: [r, g, b] }.
  setLines(loops) {
    this.lines = loops.map((l) => ({ buf: this._buffer(l.points), count: l.points.length / 3, color: l.color }));
    this.draw();
  }

  setColor(rgb) { this.color = rgb; this.draw(); }

  draw() {
    const gl = this.gl, c = this.canvas;
    const w = c.clientWidth * devicePixelRatio, h = c.clientHeight * devicePixelRatio;
    if (c.width !== w || c.height !== h) { c.width = w; c.height = h; }
    gl.viewport(0, 0, w, h);
    gl.clearColor(0.97, 0.97, 0.97, 1);
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
    const proj = perspective(0.6, w / h, dist * 0.05, dist * 4);
    const view = lookAt(eye, this.center, [0, 1, 0]);
    gl.uniformMatrix4fv(this.loc.mvp, false, mul(proj, view));
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
    gl.disable(gl.POLYGON_OFFSET_FILL);

    gl.uniform1f(this.loc.lit, 0);
    gl.disableVertexAttribArray(this.loc.nrm);
    gl.vertexAttrib3f(this.loc.nrm, 0, 0, 1);
    for (const l of this.lines) {
      gl.uniform3fv(this.loc.color, l.color);
      gl.bindBuffer(gl.ARRAY_BUFFER, l.buf);
      gl.vertexAttribPointer(this.loc.pos, 3, gl.FLOAT, false, 0, 0);
      gl.drawArrays(gl.LINE_LOOP, 0, l.count);
    }
  }
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
