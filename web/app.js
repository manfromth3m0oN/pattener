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
document.querySelectorAll('input[name=source]').forEach((r) => {
  r.onchange = () => {
    const measure = document.querySelector('input[name=source]:checked').value === 'measure';
    $('measure-panel').hidden = !measure;
    $('upload-panel').hidden = measure;
  };
});

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
  if (output.layout_svg) dl.appendChild(fileLink('layout.svg', output.layout_svg, 'image/svg+xml'));
  dl.appendChild(fileLink('report.json', JSON.stringify(r, null, 2), 'application/json'));

  table($('panel-table'), ['panel', 'ease', 'min σ', 'max σ', 'cut size (mm)', 'seams'],
    r.panels.map((p) => [p.index, f(p.ease, 3), f(p.min_stretch, 3), f(p.max_stretch, 3),
      `${f(p.cut_width_mm)} × ${f(p.cut_height_mm)}`, p.seams.join(', ')]));
  table($('seam-table'), ['seam', 'panels', 'body (mm)', 'flat (mm)', 'mismatch (mm)', 'before matching'],
    r.seams.map((s) => [s.seam, `${s.left_panel}↔${s.right_panel}`, f(s.body_length_mm),
      f(s.left_length_mm), f(s.mismatch_mm, 2), f(s.mismatch_before_matching_mm, 2)]));

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
    if (document.querySelector('input[name=source]:checked').value === 'measure') {
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
      },
      heatmap: $('heatmap').value || null,
      sheet: sheetWidth > 0 ? { sheet_width: sheetWidth, gap: 5, allow_rotation: true } : null,
    };
    const t0 = performance.now();
    const res = await call({ command: 'pattern', obj, seams, scale, options });
    if (!res.ok) throw new Error(res.error);
    show(res.output, extra);
    status.textContent = `Done in ${f((performance.now() - t0) / 1000, 1)} s: ${res.output.report.panels.length} panels.`;
  } catch (e) {
    status.className = 'error';
    status.textContent = e.message || String(e);
  } finally {
    $('run').disabled = false;
  }
}
$('run').onclick = run;
