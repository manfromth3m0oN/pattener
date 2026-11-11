// latexpat web UI: collects inputs, runs the WebAssembly module in a
// worker, and shows and offers the resulting pattern files.
'use strict';

const $ = (id) => document.getElementById(id);

// ---- worker RPC -----------------------------------------------------------
const worker = new Worker('worker.js');
let nextId = 0;
const pending = new Map();
worker.onmessage = ({ data }) => {
  pending.get(data.id)(data.response);
  pending.delete(data.id);
};
function call(request) {
  return new Promise((resolve) => {
    const id = nextId++;
    pending.set(id, resolve);
    worker.postMessage({ id, request });
  });
}

// ---- full body: measurements, shape, fitting, viewer -------------------------
const MEASURES = [
  ['height', 'Height', 'Standing straight without shoes: floor to the top of the head.'],
  ['inseam', 'Inseam', 'Inside leg: floor to the crotch.'],
  ['waist_height', 'Waist height', 'Floor to the natural waist (the narrowest point).'],
  ['arm_length', 'Arm length', 'Shoulder point, over a slightly bent elbow, to the wrist bone.'],
  ['neck', 'Neck', 'Around the base of the neck.'],
  ['bust', 'Bust / chest', 'Around the fullest part of the bust or chest, tape level.'],
  ['underbust', 'Underbust', 'Around the ribcage just below the bust.'],
  ['waist', 'Waist', 'Around the narrowest part of the torso.'],
  ['hips', 'Hips', 'Around the fullest part of the hips and buttocks.'],
  ['thigh', 'Thigh', 'Around the fullest part of the thigh, just below the crotch.'],
  ['knee', 'Knee', 'Around the middle of the knee.'],
  ['calf', 'Calf', 'Around the fullest part of the calf.'],
  ['ankle', 'Ankle', 'Around the narrowest part just above the ankle bone.'],
  ['upper_arm', 'Upper arm', 'Around the fullest part of the upper arm.'],
  ['wrist', 'Wrist', 'Around the wrist bone.'],
];
const SHAPES = [
  ['body_type', 'Body type', 0, 1, 'feminine', 'masculine'],
  ['muscle', 'Muscle', -1, 1, 'soft', 'muscular'],
  ['cup', 'Cup', -1, 1, 'small', 'large'],
  ['belly', 'Belly', -1, 1, 'flat', 'round'],
  ['buttocks', 'Buttocks', -1, 1, 'flat', 'full'],
];
const UNIT_MM = { mm: 1, cm: 10, in: 25.4 };
const FIT_WARNING_MM = 5;
const LABEL = Object.fromEntries(MEASURES.map(([k, l]) => [k, l]));

MEASURES.forEach(([key, label, hint]) => {
  const l = document.createElement('label');
  const span = document.createElement('span');
  span.textContent = label;
  span.title = hint;
  const input = document.createElement('input');
  input.type = 'number';
  input.step = 'any';
  input.min = '0';
  input.id = `m-${key}`;
  input.dataset.key = key;
  l.append(span, input);
  $('measure-inputs').appendChild(l);
});
SHAPES.forEach(([key, label, lo, hi, left, right]) => {
  const l = document.createElement('label');
  l.title = `${left} ↔ ${right}`;
  const input = document.createElement('input');
  Object.assign(input, { type: 'range', min: lo, max: hi, step: 0.05, value: 0, id: `s-${key}` });
  l.append(document.createTextNode(label), input);
  $('shape-inputs').appendChild(l);
});

let units = 'cm';
function readBody() {
  const measurements = {};
  for (const [key] of MEASURES) {
    const v = $(`m-${key}`).value.trim();
    if (v !== '') measurements[key] = Number(v) * UNIT_MM[units];
  }
  const shape = Object.fromEntries(SHAPES.map(([k]) => [k, Number($(`s-${k}`).value)]));
  return { measurements, shape };
}
function writeBody(body) {
  for (const [key] of MEASURES) {
    const mm = body.measurements?.[key];
    $(`m-${key}`).value = mm === undefined ? '' : +(mm / UNIT_MM[units]).toFixed(units === 'mm' ? 0 : 1);
  }
  for (const [k] of SHAPES) $(`s-${k}`).value = body.shape?.[k] ?? 0;
}
$('units').onchange = () => {
  const body = readBody();
  units = $('units').value;
  writeBody(body);
  saveLocal();
};

const viewer = (() => {
  try {
    return new Viewer($('viewer'));
  } catch (e) {
    $('fit-status').textContent = `3D view unavailable: ${e.message}`;
    return null;
  }
})();

let fitting = false;
let bodyUrl = null;
let baseMesh = null;
let tapes = [];
async function fitBody() {
  if (fitting) return;
  fitting = true;
  const status = $('fit-status');
  status.className = '';
  status.textContent = 'Fitting…';
  $('fit-body').disabled = true;
  try {
    const body = readBody();
    const r = await call({ command: 'human', body });
    if (!r.ok) throw new Error(r.error);
    const requested = new Set(Object.keys(body.measurements));
    baseMesh = { positions: new Float32Array(r.positions), triangles: new Uint32Array(r.triangles) };
    tapes = r.tapes.map((t) => ({
      points: new Float32Array(t.points),
      color: requested.has(t.kind) ? [0.9, 0.38, 0.0] : [0.55, 0.55, 0.6],
    }));
    if (viewer) {
      viewer.setMesh(baseMesh.positions, baseMesh.triangles);
      viewer.setLines(tapes);
    }
    const off = r.measured.filter((m) => m.requested != null && Math.abs(m.value - m.requested) > FIT_WARNING_MM);
    table($('fit-table'), ['measurement', `wanted (${units})`, `body (${units})`],
      r.measured.map((m) => [LABEL[m.kind],
        m.requested == null ? '–' : f(m.requested / UNIT_MM[units], units === 'mm' ? 0 : 1),
        f(m.value / UNIT_MM[units], units === 'mm' ? 0 : 1)]));
    [...$('fit-table').rows].slice(1).forEach((row, i) => {
      if (off.includes(r.measured[i])) row.cells[2].className = 'off';
    });
    $('fit-warnings').replaceChildren(...off.map((m) => {
      const li = document.createElement('li');
      li.textContent = `${LABEL[m.kind]} came out ${f(Math.abs(m.value - m.requested) / UNIT_MM[units], 1)} ${units} ` +
        `${m.value > m.requested ? 'over' : 'under'}: the body model can't reach this combination. ` +
        'Check the measurement, or try the shape sliders.';
      return li;
    }));
    // Own URL: pattern results revoke theirs independently.
    if (bodyUrl) URL.revokeObjectURL(bodyUrl);
    bodyUrl = URL.createObjectURL(new Blob([r.obj], { type: 'text/plain' }));
    const a = Object.assign(document.createElement('a'), { href: bodyUrl, download: 'body.obj', textContent: 'body.obj' });
    $('body-downloads').replaceChildren(document.createTextNode('Download: '), a);
    status.textContent = `Fitted in ${r.iterations} step(s).`;
    await refreshGarment();
  } catch (e) {
    status.className = 'error';
    status.textContent = e.message || String(e);
  } finally {
    $('fit-body').disabled = false;
    fitting = false;
  }
}
$('fit-body').onclick = fitBody;

// ---- garment drawn on the body ------------------------------------------------
let design = { curves: [], inside: null, zones: [], sheet_assignments: [] };
const DEFAULT_SHEETS = () => [{ name: 'black 0.40', color: '#151515', thickness_mm: 0.4, roll_width_mm: 1000 }];
let materials = DEFAULT_SHEETS();
const hexToRgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
let active = -1;          // selected curve index
let picking = null;       // 'add' | 'inside' | null
const EDGE_COLOR = [0.95, 0.75, 0.05], SEAM_COLOR = [0.15, 0.6, 1.0], ACTIVE_COLOR = [1, 1, 1];
const ZONE_COLOR = [0.45, 0.85, 0.35];

// Surface point (face + barycentric) to 3D on the fitted body.
function pointOf(sp) {
  const { positions: p, triangles: t } = baseMesh;
  const w = [1 - sp.u - sp.v, sp.u, sp.v];
  return [0, 1, 2].map((k) => w.reduce((acc, wi, i) => acc + wi * p[t[3 * sp.face + i] * 3 + k], 0));
}

function setPicking(mode) {
  picking = mode;
  $('g-inside').classList.toggle('active', mode === 'inside');
  $('g-zone').classList.toggle('active', mode === 'zone');
  $('g-paint').classList.toggle('active', mode === 'paint');
  if (viewer) $('viewer').classList.toggle('picking', !!mode);
}

function newCurve(kind) {
  const n = design.curves.filter((c) => c.kind === kind).length + 1;
  design.curves.push({ name: `${kind} ${n}`, kind, points: [], closed: kind === 'edge', mirror: false });
  active = design.curves.length - 1;
  setPicking('add');
  changed();
}

function renderCurveList() {
  const list = $('g-curves');
  list.replaceChildren(...design.curves.map((c, i) => {
    const li = document.createElement('li');
    li.className = i === active ? 'selected' : '';
    const sw = document.createElement('span');
    sw.className = 'kind';
    sw.style.background = `rgb(${(c.kind === 'edge' ? EDGE_COLOR : SEAM_COLOR).map((x) => x * 255).join(',')})`;
    li.append(sw, `${c.name} (${c.kind}, ${c.points.length} point${c.points.length === 1 ? '' : 's'}${c.closed ? ', loop' : ''}${c.mirror ? ', mirrored' : ''}${finishText(c)})`);
    li.onclick = () => { active = i; setPicking('add'); renderCurveList(); drawMarks(); };
    return li;
  }));
  const c = design.curves[active];
  $('g-active').hidden = !c;
  if (c) {
    $('g-name').value = c.name;
    $('g-closed').checked = c.closed;
    $('g-mirror').checked = c.mirror;
    showFinish(c);
  }
}

// Construction finish of the selected curve: seams glued or zipped; edges
// left to the hem allowance, raw, hemmed or trimmed.
function showFinish(c) {
  const edge = c.kind === 'edge';
  $('g-seam-finish-wrap').hidden = edge;
  $('g-edge-finish-wrap').hidden = !edge;
  $('g-seam-finish').value = c.seam_finish || 'glued';
  const fin = c.edge_finish;
  $('g-edge-finish').value = fin ? fin.kind : '';
  if (fin && fin.width_mm != null) $('g-finish-width').value = fin.width_mm;
  if (fin && fin.ease != null) $('g-finish-ease').value = Math.round(100 * fin.ease);
  $('g-finish-width-wrap').hidden = !fin || fin.kind === 'raw';
  $('g-finish-ease-wrap').hidden = !fin || fin.kind !== 'trim';
}
function readFinish() {
  const c = design.curves[active];
  if (!c) return;
  if (c.kind === 'seam') {
    if ($('g-seam-finish').value === 'zip') c.seam_finish = 'zip'; else delete c.seam_finish;
  } else {
    const kind = $('g-edge-finish').value;
    const width_mm = Math.max(1, +$('g-finish-width').value || 15);
    const ease = Math.min(1, Math.max(0.5, (+$('g-finish-ease').value || 90) / 100));
    if (!kind) delete c.edge_finish;
    else if (kind === 'raw') c.edge_finish = { kind };
    else if (kind === 'hem') c.edge_finish = { kind, width_mm };
    else c.edge_finish = { kind, width_mm, ease };
  }
  showFinish(c);
  saveLocal();
  renderCurveList();
}
['g-seam-finish', 'g-edge-finish', 'g-finish-width', 'g-finish-ease'].forEach((id) => { $(id).onchange = readFinish; });
function finishText(c) {
  if (c.kind === 'seam') return c.seam_finish === 'zip' ? ', zip' : '';
  const fin = c.edge_finish;
  if (!fin) return '';
  if (fin.kind === 'raw') return ', raw';
  if (fin.kind === 'hem') return `, ${fin.width_mm} mm hem`;
  return `, ${fin.width_mm} mm trim`;
}

function drawMarks() {
  if (!viewer || !baseMesh) return;
  const c = design.curves[active];
  const pts = [];
  if (c) c.points.forEach((p) => pts.push(...pointOf(p)));
  if (design.inside) pts.push(...pointOf(design.inside));
  viewer.setMarks(new Float32Array(pts), ACTIVE_COLOR, 7);
}

let garmentBusy = false, garmentAgain = false;
async function refreshGarment() {
  $('g-asym').checked = !!design.asymmetric;
  renderCurveList();
  renderZones();
  lastFit = null;
  $('pressure-panel').hidden = true;
  if (!baseMesh) return;
  if (garmentBusy) { garmentAgain = true; return; }
  garmentBusy = true;
  const status = $('g-status');
  try {
    do {
      garmentAgain = false;
      const empty = design.curves.every((c) => c.points.length === 0) && !design.inside && !design.zones.length;
      if (empty) {
        viewer?.setMesh(baseMesh.positions, baseMesh.triangles);
        viewer?.setLines(tapes);
        status.className = '';
        status.textContent = 'No garment yet: start with New edge, or load the example.';
        $('g-warnings').replaceChildren();
        continue;
      }
      const r = await call({ command: 'garment', body: readBody(), design, materials });
      if (!r.ok) throw new Error(r.error);
      const pieces = (r.pieces || []).filter((p) => p.color).map((p) => ({
        triangles: new Uint32Array(p.triangles), color: hexToRgb(p.color),
      }));
      viewer?.setMesh(new Float32Array(r.positions), new Uint32Array(r.triangles), new Uint32Array(r.garment_triangles), null, pieces);
      viewer?.setLines([
        ...r.curves.map((c) => ({
          points: new Float32Array(c.points),
          closed: c.closed,
          color: c.kind === 'edge' ? EDGE_COLOR : SEAM_COLOR,
        })),
        ...zoneLoops(r.zones),
      ]);
      $('g-warnings').replaceChildren(...r.warnings.map((w) => Object.assign(document.createElement('li'), { textContent: w })));
      if (r.panel_error) {
        status.className = 'error';
        status.textContent = r.panel_error.replace(/^panel \d+ /, 'A piece ');
      } else if (r.panels != null) {
        status.className = '';
        status.textContent = `Ready: the seams cut this garment into ${r.panels} flat piece${r.panels === 1 ? '' : 's'}.`;
      } else {
        status.className = '';
        status.textContent = 'Pick a point inside the garment to choose its area.';
      }
    } while (garmentAgain);
  } catch (e) {
    status.className = 'error';
    status.textContent = e.message || String(e);
  } finally {
    garmentBusy = false;
    drawMarks();
  }
}

function changed() {
  saveLocal();
  refreshGarment();
}
$('g-asym').onchange = () => {
  if ($('g-asym').checked) design.asymmetric = true; else delete design.asymmetric;
  changed();
};

if (viewer) {
  viewer.onClick = (e) => {
    if (!picking || !baseMesh) return;
    const hit = Viewer.intersect(viewer.ray(e), baseMesh.positions, baseMesh.triangles);
    if (!hit) return;
    const sp = { face: hit.face, u: hit.u, v: hit.v };
    if (picking === 'inside') {
      design.inside = sp;
      setPicking(design.curves[active] ? 'add' : null);
    } else if (picking === 'paint') {
      design.sheet_assignments.push({ point: sp, sheet: $('g-paint-sheet').value });
    } else if (picking === 'zone') {
      const base = num('ease') || 0.85;
      design.zones.push({
        name: `zone ${design.zones.length + 1}`, center: sp, radius_mm: 100,
        ease: Math.min(1, +(base + 0.05).toFixed(2)), mirror: false,
      });
      setPicking(design.curves[active] ? 'add' : null);
    } else if (design.curves[active]) {
      design.curves[active].points.push(sp);
    } else {
      return;
    }
    changed();
  };
}
// Dragging: press on a point of the selected curve (or the inside point)
// and move it along the body; the garment updates when it is released.
let dragging = null;   // { set(sp) }
function hitAt(e) {
  const hit = Viewer.intersect(viewer.ray(e), baseMesh.positions, baseMesh.triangles);
  return hit && { face: hit.face, u: hit.u, v: hit.v, t: hit.t };
}
if (viewer) {
  viewer.onPress = (e) => {
    if (!baseMesh) return false;
    const r = $('viewer').getBoundingClientRect();
    const px = e.clientX - r.left, py = e.clientY - r.top;
    const c = design.curves[active];
    const targets = [];
    if (c) c.points.forEach((p, i) => targets.push({ sp: p, set: (sp) => { c.points[i] = sp; } }));
    if (design.inside) targets.push({ sp: design.inside, set: (sp) => { design.inside = sp; } });
    // Only points facing the camera: the body hit under the pointer must
    // be near the point itself.
    const hit = hitAt(e);
    if (!hit) return false;
    const under = pointOf(hit);
    let best = null;
    for (const t of targets) {
      const q = pointOf(t.sp), s = viewer.project(q);
      if (!s) continue;
      const d = Math.hypot(s.x - px, s.y - py);
      const near = Math.hypot(q[0] - under[0], q[1] - under[1], q[2] - under[2]) < 40;
      if (d < 10 && near && (!best || d < best.d)) best = { d, t };
    }
    if (!best) return false;
    dragging = best.t;
    return true;
  };
  viewer.onDrag = (e) => {
    const hit = dragging && hitAt(e);
    if (!hit) return;
    dragging.set({ face: hit.face, u: hit.u, v: hit.v });
    drawMarks();
  };
  viewer.onDragEnd = () => {
    if (!dragging) return;
    dragging = null;
    changed();
  };
}

$('g-new-edge').onclick = () => newCurve('edge');
$('g-new-seam').onclick = () => newCurve('seam');
$('g-inside').onclick = () => setPicking(picking === 'inside' ? 'add' : 'inside');
$('g-paint').onclick = () => setPicking(picking === 'paint' ? (design.curves[active] ? 'add' : null) : 'paint');
$('g-paint-clear').onclick = () => { design.sheet_assignments = []; changed(); };

// ---- latex sheets ---------------------------------------------------------------
function renderSheets() {
  const t = $('sheets');
  table(t, ['name', 'colour', 'thickness (mm)', 'roll width (mm)', 'ease', 'price / m', ''], []);
  materials.forEach((m, i) => {
    const row = t.insertRow();
    const input = (key, type, attrs = {}, optional = false) => {
      const el = Object.assign(document.createElement('input'), { type, ...attrs });
      el.value = m[key] ?? '';
      el.onchange = () => {
        const v = el.value.trim();
        if (type === 'number') {
          if (optional && v === '') delete m[key]; else m[key] = Number(v);
        } else {
          const old = m.name;
          m[key] = v;
          // Renaming a sheet keeps its assignments.
          if (key === 'name') {
            design.sheet_assignments.forEach((a) => { if (a.sheet === old) a.sheet = v; });
            if (design.default_sheet === old) design.default_sheet = v;
          }
        }
        renderSheets();
        changed();
      };
      row.insertCell().appendChild(el);
    };
    input('name', 'text');
    input('color', 'color');
    input('thickness_mm', 'number', { min: 0.05, step: 0.05 });
    input('roll_width_mm', 'number', { min: 100, step: 50 });
    input('ease', 'number', { min: 0.5, max: 1.2, step: 0.01, placeholder: 'auto' }, true);
    input('price_per_m', 'number', { min: 0, step: 0.5, placeholder: '–' }, true);
    const del = Object.assign(document.createElement('button'), { type: 'button', textContent: '✕', title: 'Remove sheet' });
    del.disabled = materials.length === 1;
    del.onclick = () => { materials.splice(i, 1); renderSheets(); changed(); };
    row.insertCell().appendChild(del);
  });
  for (const id of ['sheet-default', 'g-paint-sheet']) {
    const sel = $(id), keep = sel.value;
    sel.replaceChildren(...materials.map((m) => Object.assign(document.createElement('option'), { value: m.name, textContent: m.name })));
    if (materials.some((m) => m.name === keep)) sel.value = keep;
  }
  if (design.default_sheet && materials.some((m) => m.name === design.default_sheet)) $('sheet-default').value = design.default_sheet;
}
$('sheet-add').onclick = () => {
  const colours = ['#b01020', '#1d3fa0', '#1a7a3a', '#6a2a8a', '#c0c0c8'];
  materials.push({ name: `sheet ${materials.length + 1}`, color: colours[(materials.length - 1) % colours.length], thickness_mm: 0.4, roll_width_mm: 1000 });
  renderSheets();
  changed();
};
$('sheet-default').onchange = () => { design.default_sheet = $('sheet-default').value; changed(); };
renderSheets();

$('g-zone').onclick = () => setPicking(picking === 'zone' ? (design.curves[active] ? 'add' : null) : 'zone');

function renderZones() {
  $('g-zones-wrap').hidden = design.zones.length === 0;
  table($('g-zones'), ['zone', 'radius (mm)', 'ease', 'mirror', ''], []);
  design.zones.forEach((z, i) => {
    const row = $('g-zones').insertRow();
    const input = (type, value, set, attrs = {}) => {
      const el = Object.assign(document.createElement('input'), { type, ...attrs });
      if (type === 'checkbox') el.checked = value; else el.value = value;
      el.onchange = () => { set(type === 'checkbox' ? el.checked : type === 'number' ? Number(el.value) : el.value); changed(); };
      row.insertCell().appendChild(el);
    };
    input('text', z.name, (v) => { z.name = v; });
    input('number', z.radius_mm, (v) => { z.radius_mm = v; }, { min: 10, step: 10 });
    input('number', z.ease, (v) => { z.ease = v; }, { min: 0.5, max: 1.2, step: 0.01 });
    input('checkbox', z.mirror, (v) => { z.mirror = v; });
    const del = Object.assign(document.createElement('button'), { type: 'button', textContent: '✕', title: 'Remove zone' });
    del.onclick = () => { design.zones.splice(i, 1); changed(); };
    row.insertCell().appendChild(del);
  });
}

// Three great circles per zone, so its extent reads from any angle.
function zoneLoops(zones) {
  const loops = [];
  for (const z of zones) {
    for (let axis = 0; axis < 3; axis++) {
      const pts = [];
      for (let k = 0; k < 48; k++) {
        const a = (2 * Math.PI * k) / 48, p = [...z.center];
        const [i, j] = [[1, 2], [0, 2], [0, 1]][axis];
        p[i] += z.radius_mm * Math.cos(a);
        p[j] += z.radius_mm * Math.sin(a);
        pts.push(...p);
      }
      loops.push({ points: new Float32Array(pts), color: ZONE_COLOR, closed: true });
    }
  }
  return loops;
}

// Pressure colours: blue where the latex bridges, white at 0, orange to
// red up to 40 mmHg (matches the legend in index.html).
function pressureColor(p) {
  const lerp = (a, b, t) => a.map((x, k) => x + (b[k] - x) * t);
  const blue = [0.16, 0.35, 0.78], white = [0.95, 0.95, 0.95], orange = [0.95, 0.55, 0.1], red = [0.7, 0, 0];
  if (p < 0) return lerp(white, blue, Math.min(1, -p / 10));
  if (p < 20) return lerp(white, orange, p / 20);
  return lerp(orange, red, Math.min(1, (p - 20) / 20));
}

let lastFit = null;
function showFit() {
  if (!viewer || !lastFit) return;
  const { positions, triangles, garment, pressure } = lastFit;
  let colors = null;
  if ($('show-pressure').checked) {
    colors = new Float32Array(positions.length);
    pressure.forEach((p, i) => colors.set(pressureColor(p), 3 * i));
  }
  viewer.setMesh(positions, triangles, garment, colors);
}
$('show-pressure').onchange = showFit;
$('g-undo').onclick = () => {
  const c = design.curves[active];
  if (c && c.points.length) { c.points.pop(); changed(); }
};
$('g-delete').onclick = () => {
  if (active < 0) return;
  design.curves.splice(active, 1);
  active = Math.min(active, design.curves.length - 1);
  changed();
};
$('g-clear').onclick = () => {
  if (design.curves.length && !confirm('Remove the whole garment drawing?')) return;
  design = { curves: [], inside: null, zones: [], sheet_assignments: [] };
  active = -1;
  setPicking(null);
  changed();
};
// Template library: examples/index.json lists examples/<id>.project.json.
let templates = [];
(async () => {
  try {
    templates = await (await fetch('examples/index.json')).json();
  } catch {
    templates = [{ id: 'top', name: 'Short-sleeved top', description: '' }];
  }
  $('g-template').replaceChildren(...templates.map((t) => Object.assign(document.createElement('option'), { value: t.id, textContent: t.name })));
  showTemplateNote();
})();
function showTemplateNote() {
  const t = templates.find((x) => x.id === $('g-template').value);
  $('g-template-note').textContent = t ? t.description : '';
}
$('g-template').onchange = showTemplateNote;
$('g-example').onclick = async () => {
  const id = $('g-template').value || 'top';
  const name = (templates.find((t) => t.id === id) || { name: id }).name;
  if (design.curves.length && !confirm(`Replace the current garment with the ${name.toLowerCase()} template?`)) return;
  const p = await (await fetch(`examples/${id}.project.json`)).json();
  design = p.garment;
  design.zones = design.zones || [];
  design.sheet_assignments = design.sheet_assignments || [];
  if (p.materials && p.materials.length) { materials = p.materials; renderSheets(); }
  active = -1;
  setPicking(null);
  changed();
};
$('g-name').oninput = () => { const c = design.curves[active]; if (c) { c.name = $('g-name').value; saveLocal(); renderCurveList(); } };
$('g-closed').onchange = () => { const c = design.curves[active]; if (c) { c.closed = $('g-closed').checked; changed(); } };
$('g-mirror').onchange = () => { const c = design.curves[active]; if (c) { c.mirror = $('g-mirror').checked; changed(); } };

// ---- project: save, open, autosave ----------------------------------------------
const STORAGE_KEY = 'latexpat.project';
const EXAMPLE_BODY = {
  measurements: { height: 1680, bust: 940, underbust: 790, waist: 740, hips: 980, thigh: 570 },
  shape: {},
};
function readProject() {
  return { version: 1, name: $('project-name').value.trim() || 'Untitled', body: readBody(), garment: design, materials, units };
}
function writeProject(p) {
  $('project-name').value = p.name || 'Untitled';
  units = UNIT_MM[p.units] ? p.units : 'cm';
  $('units').value = units;
  writeBody(p.body || {});
  design = p.garment && Array.isArray(p.garment.curves) ? p.garment : { curves: [], inside: null };
  design.zones = design.zones || [];
  design.sheet_assignments = design.sheet_assignments || [];
  materials = Array.isArray(p.materials) && p.materials.length ? p.materials : DEFAULT_SHEETS();
  renderSheets();
  active = -1;
  setPicking(null);
}
function saveLocal() {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(readProject()));
    $('project-status').textContent = 'Saved in this browser.';
  } catch {
    $('project-status').textContent = '';
  }
}
$('project-name').oninput = saveLocal;
$('measure-inputs').oninput = saveLocal;
$('shape-inputs').oninput = saveLocal;
$('project-save').onclick = () => {
  const p = readProject();
  const a = fileLink(`${p.name.replace(/[^\w.-]+/g, '_')}.latexpat.json`, JSON.stringify(p, null, 2), 'application/json');
  a.click();
};
$('project-open').onchange = async () => {
  const text = await readFile($('project-open'));
  $('project-open').value = '';
  if (!text) return;
  try {
    writeProject(JSON.parse(text));
    saveLocal();
    fitBody();
  } catch (e) {
    $('project-status').textContent = `Could not open: ${e.message}`;
  }
};
$('project-new').onclick = () => {
  if (!confirm('Start a new project? Unsaved changes to this one will be lost.')) return;
  writeProject({ name: 'Untitled', body: { measurements: {}, shape: {} }, garment: { curves: [], inside: null, zones: [], sheet_assignments: [] }, materials: DEFAULT_SHEETS(), units });
  saveLocal();
  fitBody();
};

// ---- measurement table ----------------------------------------------------
const DEFAULT_LEVELS = [
  ['hip', 0, 980, '', ''],
  ['waist', 200, 740, '', ''],
  ['underbust', 330, 790, '', ''],
  ['bust', 420, 940, '', 0.6],
  ['chest', 500, 900, '', ''],
];

function addLevel(values = ['', '', '', '', '']) {
  const tr = document.createElement('tr');
  const kinds = ['text', 'number', 'number', 'number', 'number'];
  values.forEach((v, i) => {
    const td = document.createElement('td');
    const input = document.createElement('input');
    input.type = kinds[i];
    input.value = v;
    if (i > 2) input.placeholder = i === 3 ? '0.75' : '0.5';
    if (i > 0) input.step = 'any';
    td.appendChild(input);
    tr.appendChild(td);
  });
  const td = document.createElement('td');
  const del = document.createElement('button');
  del.type = 'button';
  del.textContent = '✕';
  del.title = 'Remove level';
  del.onclick = () => tr.remove();
  td.appendChild(del);
  tr.appendChild(td);
  $('levels').tBodies[0].appendChild(tr);
}
DEFAULT_LEVELS.forEach(addLevel);
$('add-level').onclick = () => addLevel();

function readLevels() {
  return [...$('levels').tBodies[0].rows].map((tr) => {
    const v = [...tr.querySelectorAll('input')].map((i) => i.value.trim());
    const level = { name: v[0] || 'level', height_mm: Number(v[1]), circumference_mm: Number(v[2]) };
    if (v[3] !== '') level.depth_ratio = Number(v[3]);
    if (v[4] !== '') level.front_bias = Number(v[4]);
    return level;
  });
}

// ---- source toggle ----------------------------------------------------------
const source = () => document.querySelector('input[name=source]:checked').value;
function showSource() {
  const s = source();
  $('human-panel').hidden = s !== 'human';
  $('measure-panel').hidden = s !== 'measure';
  $('upload-panel').hidden = s !== 'upload';
  $('run').disabled = false;
  $('run-note').hidden = true;
}
document.querySelectorAll('input[name=source]').forEach((r) => { r.onchange = showSource; });

const readFile = (input) => (input.files[0] ? input.files[0].text() : Promise.resolve(null));
const num = (id) => Number($(id).value);

// ---- output helpers -----------------------------------------------------------
const urls = [];
function fileLink(name, text, type) {
  const url = URL.createObjectURL(new Blob([text], { type }));
  urls.push(url);
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.textContent = name;
  return a;
}
function svgImage(svg, alt) {
  const img = document.createElement('img');
  const url = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml' }));
  urls.push(url);
  img.src = url;
  img.alt = alt;
  return img;
}
function table(el, head, rows) {
  el.replaceChildren();
  const tr = el.insertRow();
  head.forEach((h) => { const th = document.createElement('th'); th.textContent = h; tr.appendChild(th); });
  rows.forEach((r) => { const row = el.insertRow(); r.forEach((c) => { row.insertCell().textContent = c; }); });
}
const f = (x, d = 1) => Number(x).toFixed(d);

function show(output, extraFiles) {
  urls.splice(0).forEach(URL.revokeObjectURL);
  const r = output.report;
  $('results').hidden = false;

  $('warnings').replaceChildren(...output.warnings.map((w) => {
    const li = document.createElement('li');
    li.textContent = w;
    return li;
  }));

  const dl = $('downloads');
  dl.replaceChildren(document.createTextNode('Download: '));
  extraFiles.forEach(([n, t, ty]) => dl.appendChild(fileLink(n, t, ty)));
  r.panels.forEach((p, i) => dl.appendChild(fileLink(p.file, output.panel_svgs[i], 'image/svg+xml')));
  if (output.pdf) dl.appendChild(fileLink('pattern.pdf', output.pdf, 'application/pdf'));
  if (output.layout_svg) dl.appendChild(fileLink('layout.svg', output.layout_svg, 'image/svg+xml'));
  dl.appendChild(fileLink('report.json', JSON.stringify(r, null, 2), 'application/json'));

  const pairing = (p) => (p.mirror_of == null ? '' : p.mirror_of === p.index ? ' (symmetric)' : ` (mirror of ${p.mirror_of})`);
  table($('panel-table'), ['panel', 'ease', 'min σ', '95 % under', 'max σ', 'slack', 'mmHg', '95 % under', 'bridging', 'cut size (mm)', 'seams'],
    r.panels.map((p) => [`${p.index}${pairing(p)}`, f(p.ease, 3), f(p.min_stretch, 3), f(p.p95_stretch, 3), f(p.max_stretch, 3),
      `${f(100 * p.slack_fraction, 0)} %`, f(p.mean_pressure_mmhg, 1), f(p.p95_pressure_mmhg, 1), `${f(100 * p.bridging_fraction, 0)} %`,
      `${f(p.cut_width_mm)} × ${f(p.cut_height_mm)}`, p.seams.join(', ')]));
  table($('seam-table'), ['seam', 'panels', 'body (mm)', 'flat (mm)', 'mismatch (mm)', 'before matching'],
    r.seams.map((s) => [s.seam, `${s.left_panel}↔${s.right_panel}`, f(s.body_length_mm),
      f(s.left_length_mm), f(s.mismatch_mm, 2), f(s.mismatch_before_matching_mm, 2)]));

  const mout = $('materials-out');
  mout.replaceChildren();
  if (r.materials && r.materials.length) {
    const h = document.createElement('h3');
    h.textContent = 'Materials';
    const t = document.createElement('table');
    mout.append(h, t);
    table(t, ['sheet', 'pieces', 'roll (mm)', 'length to buy', 'piece area', 'used', 'cost'],
      r.materials.map((u) => [u.sheet, u.pieces.join(', '), f(u.roll_width_mm, 0), `${f(u.length_mm / 1000, 2)} m`,
        `${f(u.pieces_area_m2, 3)} m²`, `${f(100 * u.utilisation, 0)} %`, u.cost == null ? '–' : f(u.cost, 2)]));
    [...t.rows].slice(1).forEach((row, i) => {
      const sw = Object.assign(document.createElement('span'), { className: 'swatch' });
      sw.style.background = r.materials[i].color;
      row.cells[0].prepend(sw);
    });
    r.materials.forEach((u, i) => {
      const h4 = document.createElement('h4');
      h4.textContent = `${u.sheet}: ${f(u.length_mm / 1000, 2)} m of a ${f(u.roll_width_mm, 0)} mm roll`;
      mout.append(h4, svgImage(output.material_layouts[i], `layout on ${u.sheet}`));
      dl.appendChild(fileLink(u.file, output.material_layouts[i], 'image/svg+xml'));
    });
  }

  const aout = $('assembly-out');
  aout.replaceChildren();
  if (r.assembly && r.assembly.length) {
    const h = document.createElement('h3');
    h.textContent = 'Assembly';
    const ol = document.createElement('ol');
    ol.id = 'assembly';
    ol.append(...r.assembly.map((step) => Object.assign(document.createElement('li'), { textContent: step })));
    aout.append(h, ol);
  }
  const tout = $('trims-out');
  tout.replaceChildren();
  if (r.trims && r.trims.length) {
    const h = document.createElement('h3');
    h.textContent = 'Trim strips';
    tout.append(h);
    r.trims.forEach((t, i) => {
      const name = `trim_${String(i).padStart(2, '0')}.svg`;
      const h4 = document.createElement('h4');
      h4.textContent = `${t.edge}: ${f(t.length_mm, 0)} × ${f(t.width_mm, 0)} mm (edge ${f(t.edge_length_mm, 0)} mm)`;
      tout.append(h4, svgImage(output.trim_svgs[i], `${t.edge} trim`));
      dl.appendChild(fileLink(name, output.trim_svgs[i], 'image/svg+xml'));
    });
  }

  const layout = $('layout');
  layout.replaceChildren();
  if (output.layout_svg) {
    const h = document.createElement('h3');
    h.textContent = `Layout: ${f(r.layout.length_mm, 0)} mm of a ${f(r.layout.sheet_width_mm, 0)} mm sheet`;
    layout.append(h, svgImage(output.layout_svg, 'sheet layout'));
  }

  $('pieces').replaceChildren(...output.panel_svgs.map((svg, i) => {
    const div = document.createElement('div');
    div.className = 'piece';
    const cap = document.createElement('div');
    cap.appendChild(fileLink(r.panels[i].file, svg, 'image/svg+xml'));
    div.append(svgImage(svg, `panel ${i}`), cap);
    return div;
  }));
}

// ---- run ----------------------------------------------------------------------
async function run() {
  const status = $('status');
  status.className = '';
  status.textContent = 'Working…';
  $('run').disabled = true;
  try {
    let obj, seams, scale = 1, up = [0, 1, 0];
    const extra = [];
    if (source() === 'human') {
      // Built below from the garment drawn on the full body.
    } else if (source() === 'measure') {
      const body = await call({
        command: 'body',
        measurements: { levels: readLevels() },
        options: { style: $('style').value },
      });
      if (!body.ok) throw new Error(body.error);
      ({ obj, seams } = body);
      extra.push(['body.obj', obj, 'text/plain'], ['body.seams.json', JSON.stringify(seams, null, 2), 'application/json']);
    } else {
      obj = await readFile($('obj-file'));
      if (!obj) throw new Error('choose an .obj file');
      const s = await readFile($('seams-file'));
      seams = s ? JSON.parse(s) : { seams: [] };
      scale = num('scale');
      up = $('up').value.split(',').map(Number);
    }

    const calib = await readFile($('calib-file'));
    const ease = calib
      ? { calibrated: { calibration: JSON.parse(calib), sheet: $('calib-sheet').value.trim(), panel_sheets: [] } }
      : { uniform: num('ease') };
    const sheetWidth = num('sheet-width');
    const options = {
      ease,
      up,
      seam_matching: $('match').checked ? {} : null,
      pattern: {
        seam_allowance: num('allowance'),
        allowance_side: $('allowance-side').value,
        hem_allowance: num('hem'),
        notch_spacing: num('notch') > 0 ? num('notch') : null,
        zip_allowance: num('zip'),
      },
      outline_smoothing_mm: Math.max(0, num('smooth')),
      print: $('paper').value ? { paper: $('paper').value } : null,
      heatmap: $('heatmap').value || null,
      length_relief: Math.max(0, Math.min(100, num('length-relief'))) / 100,
      material: { thickness_mm: num('thickness') || 0.4 },
      sheet: sheetWidth > 0 ? { sheet_width: sheetWidth, gap: 5, allow_rotation: true } : null,
    };
    const t0 = performance.now();
    const res = source() === 'human'
      ? await call({ command: 'garment', body: readBody(), design, materials, pattern: true, options })
      : await call({ command: 'pattern', obj, seams, scale, options });
    if (!res.ok) throw new Error(res.error);
    show(res.output, extra);
    if (res.pressure) {
      lastFit = {
        positions: new Float32Array(res.positions),
        triangles: new Uint32Array(res.triangles),
        garment: new Uint32Array(res.garment_triangles),
        pressure: res.pressure,
      };
      $('pressure-panel').hidden = false;
      showFit();
    }
    status.textContent = `Done in ${f((performance.now() - t0) / 1000, 1)} s: ${res.output.report.panels.length} panels.`;
  } catch (e) {
    status.className = 'error';
    status.textContent = e.message || String(e);
  } finally {
    $('run').disabled = false;
  }
}
$('run').onclick = run;

// ---- start ----------------------------------------------------------------------
(() => {
  let saved = null;
  try {
    saved = JSON.parse(localStorage.getItem(STORAGE_KEY));
  } catch { /* ignore */ }
  writeProject(saved || { name: 'Example', body: EXAMPLE_BODY, units: 'cm' });
  showSource();
  fitBody();
})();
