// latexpat web UI: measurements, the flat design editor, and the pattern.
// The WebAssembly module (in a worker) drafts templates, follows new
// measurements, checks designs and makes the pattern files.
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

// ---- measurements ------------------------------------------------------------
const MEASURES = [
  ['height', 'Height', 'Standing straight without shoes: floor to the top of the head.'],
  ['inseam', 'Inseam', 'Inside leg: floor to the crotch.'],
  ['waist_height', 'Waist height', 'Floor to the natural waist (the narrowest point).'],
  ['back_length', 'Back length', 'From the bone at the base of the neck (nape) down the spine to the waist.'],
  ['shoulder', 'Shoulder width', 'Across the back from one shoulder point (the bony tip) to the other.'],
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
const UNIT_MM = { mm: 1, cm: 10, in: 25.4 };
let units = 'cm';
const fmtLen = (mm, d) => `${(mm / UNIT_MM[units]).toFixed(d ?? (units === 'mm' ? 0 : 1))} ${units}`;

MEASURES.forEach(([key, label, hint]) => {
  const l = document.createElement('label');
  const span = document.createElement('span');
  span.textContent = label;
  span.title = hint;
  span.addEventListener('click', (e) => {
    e.preventDefault();
    const h = $('measure-hint');
    const same = !h.hidden && h.dataset.key === key;
    h.hidden = same;
    h.dataset.key = key;
    h.textContent = `${label}: ${hint}`;
  });
  const input = Object.assign(document.createElement('input'), { type: 'number', step: 'any', min: '0', id: `m-${key}` });
  l.append(span, input);
  $('measure-inputs').appendChild(l);
});

const figure = () => document.querySelector('input[name=figure]:checked').value;
function readBody() {
  const measurements = {};
  for (const [key] of MEASURES) {
    const v = $(`m-${key}`).value.trim();
    if (v !== '' && Number(v) > 0) measurements[key] = Number(v) * UNIT_MM[units];
  }
  return { figure: figure(), measurements };
}
function writeBody(body) {
  document.querySelectorAll('input[name=figure]').forEach((r) => { r.checked = r.value === (body.figure || 'women'); });
  for (const [key] of MEASURES) {
    const mm = body.measurements?.[key];
    $(`m-${key}`).value = mm === undefined ? '' : +(mm / UNIT_MM[units]).toFixed(units === 'mm' ? 0 : 1);
  }
}

// The body: estimates for what is missing, levels for the editor, and the
// design redrafted when the measurements change.
let bodyTimer = null;
let lastBody = null;
async function updateBody({ redraft = true } = {}) {
  const body = readBody();
  const r = await call({ command: 'body', body });
  const status = $('body-status');
  if (!r.ok) {
    status.className = 'error';
    status.textContent = r.error.replace(/^body: /, '');
    return;
  }
  status.className = 'hint';
  status.textContent = `${MEASURES.length - r.estimated.length} of ${MEASURES.length} measured; the rest estimated.`;
  for (const [key] of MEASURES) {
    $(`m-${key}`).placeholder = (r.values[key] / UNIT_MM[units]).toFixed(units === 'mm' ? 0 : 1);
  }
  editor.setGuides(r.guides);
  lastBody = body;
  if (redraft && design.block && JSON.stringify(design.block.body) !== JSON.stringify(body)) {
    const d = await call({ command: 'redraft', design, body });
    if (d.ok) {
      setDesign(d.design);
      commit('The design was redrafted for the new measurements; your edits are kept.');
    } else {
      setStatus(d.error, true);
    }
  }
}
const bodyChanged = () => { saveLocal(); clearTimeout(bodyTimer); bodyTimer = setTimeout(updateBody, 400); };
$('measure-inputs').addEventListener('input', bodyChanged);
document.querySelectorAll('input[name=figure]').forEach((r) => { r.onchange = bodyChanged; });
$('units').onchange = () => {
  const body = readBody();
  units = $('units').value;
  writeBody(body);
  saveLocal();
  updateBody({ redraft: false });
  renderLists();
  renderInspector();
};

// ---- the design ---------------------------------------------------------------
let design = { pieces: [], seams: [] };
let problems = [];
const undo = [], redo = [];
let saved = JSON.stringify(design);

const editor = new Editor($('editor'), {
  onChange: (isCommit, message) => {
    if (isCommit) commit(message);
    else { renderInspector(); }
  },
  onSelect: () => { renderInspector(); renderLists(); },
  onHint: (t) => { $('editor-hint').textContent = t; },
  onTool: (t) => {
    document.querySelectorAll('[data-tool]').forEach((b) => b.classList.toggle('active', b.dataset.tool === t));
  },
  colorOf: (p) => sheetOf(p)?.color || '#888888',
});

function setDesign(d) {
  design = d;
  design.pieces = design.pieces || [];
  design.seams = design.seams || [];
  editor.setDesign(design);
}

// Seams whose edges no longer exist (a point deleted or made smooth) go.
function dropBrokenSeams() {
  const ok = (r) => {
    const p = design.pieces.find((x) => x.id === r.piece);
    return p && Geo.edges(p).some((e) => p.nodes[e.start].id === r.node);
  };
  const gone = design.seams.filter((s) => !ok(s.a) || !ok(s.b));
  design.seams = design.seams.filter((s) => ok(s.a) && ok(s.b));
  return gone.map((s) => s.name);
}

// A finished edit: record it for undo, save, check, and maybe remake the pattern.
function commit(message) {
  const gone = dropBrokenSeams();
  const now = JSON.stringify(design);
  if (now !== saved) {
    undo.push(saved);
    if (undo.length > 200) undo.shift();
    redo.length = 0;
    saved = now;
  }
  editor.setDesign(design);
  afterChange();
  if (gone.length) setStatus(`Removed seam ${gone.join(', ')}: its edge no longer exists.`);
  else if (message) setStatus(message);
}
function restore(stack, other) {
  if (!stack.length) return;
  other.push(saved);
  saved = stack.pop();
  setDesign(JSON.parse(saved));
  afterChange();
}
$('undo').onclick = () => restore(undo, redo);
$('redo').onclick = () => restore(redo, undo);

let checkTimer = null, liveTimer = null;
function afterChange() {
  saveLocal();
  renderLists();
  renderInspector();
  $('undo').disabled = !undo.length;
  $('redo').disabled = !redo.length;
  clearTimeout(checkTimer);
  checkTimer = setTimeout(check, 150);
}
async function check() {
  const r = await call({ command: 'check', design });
  problems = r.ok ? r.problems : [r.error];
  $('g-warnings').replaceChildren(...problems.map((w) => Object.assign(document.createElement('li'), { textContent: w })));
  const n = design.pieces.length;
  $('g-status').className = problems.length ? 'error' : '';
  $('g-status').textContent = !n ? 'No pieces yet: start from a template, or draw one.'
    : problems.length ? 'Fix these before making the pattern:'
      : `Ready: ${n} piece${n === 1 ? '' : 's'}, ${design.seams.length} seam${design.seams.length === 1 ? '' : 's'}.`;
  if ($('live').checked && !problems.length && n) {
    clearTimeout(liveTimer);
    liveTimer = setTimeout(() => { if (!$('run').disabled) run(); }, 300);
  }
}

// ---- lists: pieces and seams --------------------------------------------------
function edgeIndex(p, nodeId) { return Geo.edges(p).findIndex((e) => p.nodes[e.start].id === nodeId); }
function edgeName(p, k) {
  const props = Geo.edgeProps(p, k);
  return props.name || `edge ${k + 1}`;
}
function seamLengths(s) {
  const len = (r) => {
    const p = design.pieces.find((x) => x.id === r.piece);
    const k = p ? edgeIndex(p, r.node) : -1;
    return k >= 0 ? Geo.edgeLength(p, k) : NaN;
  };
  return [len(s.a), len(s.b)];
}
function li(text, selected, onclick, color) {
  const el = document.createElement('li');
  el.className = selected ? 'selected' : '';
  if (color) {
    const sw = Object.assign(document.createElement('span'), { className: 'kind' });
    sw.style.background = color;
    el.append(sw);
  }
  el.append(text);
  el.onclick = onclick;
  return el;
}
function renderLists() {
  const sel = editor.sel || {};
  $('g-pieces').replaceChildren(...design.pieces.map((p) => li(
    `${p.name}${p.pair ? ' (pair)' : ''}${p.nodes.some((n) => n.edge?.fold) ? ' (on the fold)' : ''}`,
    sel.piece === p.id && sel.kind === 'piece',
    () => editor.select({ kind: 'piece', piece: p.id }),
    sheetOf(p)?.color,
  )));
  $('g-seams').replaceChildren(...design.seams.map((s, i) => {
    const [a, b] = seamLengths(s);
    const mis = Math.abs(a - b);
    const t = `${s.name || 'seam'}: ${fmtLen(a)}${s.a.piece === s.b.piece && s.a.node === s.b.node ? ' (to its mirror)' : mis > 0.5 ? `, differs by ${fmtLen(mis, 1)}` : ''}${s.finish === 'zip' ? ', zip' : ''}`;
    return li(t, sel.kind === 'seam' && sel.seam === s.id, () => editor.select({ kind: 'seam', seam: s.id }), SEAM_COLORS[i % SEAM_COLORS.length]);
  }));
}

// ---- inspector ----------------------------------------------------------------------
function el(tag, props = {}, ...kids) {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...kids);
  return e;
}
function field(label, input) { return el('label', {}, label, input); }
function checkbox(label, value, onchange) {
  const i = el('input', { type: 'checkbox', checked: !!value });
  i.onchange = () => onchange(i.checked);
  return el('label', { className: 'check' }, i, ` ${label}`);
}
function textInput(value, onchange) {
  const i = el('input', { type: 'text', value: value || '' });
  i.onchange = () => onchange(i.value.trim());
  return i;
}
function numInput(value, onchange, attrs = {}) {
  const i = el('input', { type: 'number', step: 'any', value, ...attrs });
  i.onchange = () => { if (i.value !== '' && Number.isFinite(+i.value)) onchange(+i.value); };
  return i;
}
function selectInput(options, value, onchange) {
  const s = el('select', {}, ...options.map(([v, t]) => el('option', { value: v, textContent: t })));
  s.value = value;
  s.onchange = () => onchange(s.value);
  return s;
}
function button(text, onclick, cls = '') { return el('button', { type: 'button', textContent: text, className: cls, onclick }); }
const lengthText = (mm) => fmtLen(mm, units === 'mm' ? 0 : 1);

function renderInspector() {
  const box = $('inspector');
  const sel = editor.sel;
  $('inspector-empty').hidden = !!sel;
  box.replaceChildren();
  if (!sel) return;
  const g = el('div', { className: 'group' });
  box.append(g);
  const p = sel.piece && design.pieces.find((x) => x.id === sel.piece);

  if (sel.kind === 'piece' && p) {
    const { pts } = Geo.outline(p);
    const xs = pts.map((q) => q[0]), ys = pts.map((q) => q[1]);
    const fold = p.nodes.some((n) => n.edge?.fold);
    g.append(
      el('h3', { textContent: 'Piece' }),
      field('Name', textInput(p.name, (v) => { p.name = v || p.name; commit(); })),
      checkbox('Cut as a mirrored pair (one for each side)', p.pair, (v) => { p.pair = v; commit(); }),
      field('Up the body (° from the page\'s up)', numInput(p.up_deg || 0, (v) => {
        const d = ((((v + 180) % 360) + 360) % 360) - 180;
        if (d) p.up_deg = d; else delete p.up_deg;
        editor.cache.delete(p.id);
        commit();
      }, { step: 15, min: -180, max: 180 })),
      el('p', {
        className: 'hint',
        textContent: 'The arrow on the piece points up the body: the lengthwise reduction is applied along it, '
          + 'the ease around the body across it. Drag its round handle to turn it.',
      }),
      field('Latex sheet', selectInput([['', `default (${defaultSheet()?.name || '–'})`], ...materials.map((m) => [m.name, m.name])],
        p.sheet || '', (v) => { if (v) p.sheet = v; else delete p.sheet; commit(); })),
      el('p', {
        className: 'hint',
        textContent: `${lengthText(Math.max(...xs) - Math.min(...xs))} × ${lengthText(Math.max(...ys) - Math.min(...ys))} at body size` +
          `${fold ? ', drafted on the fold (cut unfolded, twice as wide)' : ''}. Drag it to move it; `
          + 'select an edge or a point to change it.',
      }),
      el('div', { className: 'row' },
        button('Duplicate', () => {
          const q = JSON.parse(JSON.stringify(p));
          q.id = newId('p');
          q.name = `${p.name} copy`;
          q.at = [(p.at || [0, 0])[0] + 60, (p.at || [0, 0])[1]];
          design.pieces.push(q);
          editor.select({ kind: 'piece', piece: q.id });
          commit('Piece duplicated');
        }),
        button('Flip', () => {
          for (const n of p.nodes) n.x = -n.x;
          if (p.up_deg) p.up_deg = -p.up_deg;
          p.nodes.reverse();
          // Each edge now starts at the node that ended it.
          const props = p.nodes.map((n) => n.edge);
          const kinds = p.nodes.map((n) => n.kind || 'corner');
          const vs = p.nodes.map((n, i) => (kinds[i] !== 'smooth' ? i : -1)).filter((i) => i >= 0);
          vs.forEach((v, j) => { p.nodes[v].edge = props[vs[(j + 1) % vs.length]]; });
          // The seams' edge references move with the properties.
          const map = new Map(vs.map((v, j) => [p.nodes[vs[(j + 1) % vs.length]].id, p.nodes[v].id]));
          for (const s of design.seams) for (const r of [s.a, s.b]) if (r.piece === p.id && map.has(r.node)) r.node = map.get(r.node);
          for (const s of design.seams) if ((s.a.piece === p.id) !== (s.b.piece === p.id)) s.reverse = !s.reverse;
          editor.cache.delete(p.id);
          commit('Piece flipped');
        }),
        button('Delete piece', () => deleteSelection(), 'quiet'),
      ),
    );
  } else if (sel.kind === 'edge' && p) {
    const k = edgeIndex(p, sel.node);
    if (k < 0) return;
    const n = p.nodes[Geo.edges(p)[k].start];
    n.edge = n.edge || {};
    const props = n.edge;
    const seam = design.seams.find((s) => [s.a, s.b].some((r) => r.piece === p.id && r.node === sel.node));
    g.append(
      el('h3', { textContent: `Edge of ${p.name}` }),
      field('Name', textInput(props.name, (v) => { props.name = v; commit(); })),
      el('p', { className: 'hint', textContent: `${lengthText(Geo.edgeLength(p, k))} long at body size.` }),
    );
    if (seam) {
      g.append(el('p', {}, `Part of the ${seam.name || 'seam'} seam. `,
        button('Edit the seam', () => editor.select({ kind: 'seam', seam: seam.id }))));
    } else {
      const straight = Geo.edges(p)[k].end === (Geo.edges(p)[k].start + 1) % p.nodes.length;
      g.append(checkbox('Fold line: cut the piece on the fold here (a straight edge, usually the centre front or back)',
        props.fold, (v) => { if (v) props.fold = true; else delete props.fold; editor.cache.delete(p.id); commit(); }));
      if (props.fold && !straight) g.append(el('p', { className: 'hint', textContent: 'A fold is drawn straight from end to end; its inner points are ignored.' }));
      if (!props.fold) g.append(...finishFields(props));
      g.append(el('p', { className: 'hint', textContent: 'To sew this edge to another, use the Join tool.' }));
    }
  } else if (sel.kind === 'node' && p) {
    const n = p.nodes.find((x) => x.id === sel.node);
    if (!n) return;
    const [ox, oy] = p.at || [0, 0];
    const u = UNIT_MM[units];
    g.append(
      el('h3', { textContent: `Point of ${p.name}` }),
      field('Kind', selectInput([
        ['corner', 'corner (sharp; an edge starts here)'],
        ['smooth', 'smooth (the curve passes through)'],
        ['joint', 'joint (smooth, but an edge starts here)'],
      ], n.kind || 'corner', (v) => {
        if (v !== 'smooth' && (n.kind || 'corner') === 'smooth') {
          const k = Geo.segmentEdges(p)[p.nodes.indexOf(n)];
          n.edge = { ...Geo.edgeProps(p, k) };
        }
        n.kind = v;
        editor.cache.delete(p.id);
        commit();
      })),
      el('div', { className: 'grid' },
        field(`Across (${units})`, numInput(+(n.x / u).toFixed(2), (v) => { n.x = v * u; editor.cache.delete(p.id); commit(); })),
        field(`Height (${units})`, numInput(+((n.y + oy) / u).toFixed(2), (v) => { n.y = v * u - oy; editor.cache.delete(p.id); commit(); })),
      ),
      el('p', { className: 'hint', textContent: `Across is measured from the piece's centre line${ox ? '' : ''}; height above the floor.` }),
      checkbox('Notch here (matched on the edge it is sewn to)', n.notch, (v) => { if (v) n.notch = true; else delete n.notch; commit(); }),
      el('div', { className: 'row' }, button('Delete point', () => deleteSelection(), 'quiet')),
    );
  } else if (sel.kind === 'seam') {
    const s = design.seams.find((x) => x.id === sel.seam);
    if (!s) return;
    const pa = design.pieces.find((x) => x.id === s.a.piece), pb = design.pieces.find((x) => x.id === s.b.piece);
    const [la, lb] = seamLengths(s);
    const self = s.a.piece === s.b.piece && s.a.node === s.b.node;
    const side = (pp, r) => `${pp?.name}: ${pp ? edgeName(pp, edgeIndex(pp, r.node)) : '?'}`;
    g.append(
      el('h3', { textContent: 'Seam' }),
      field('Name', textInput(s.name, (v) => { s.name = v; commit(); })),
      el('p', {
        className: 'hint',
        textContent: self ? `Joins ${side(pa, s.a)} (${lengthText(la)}) to its mirror image on the other side.`
          : `Joins ${side(pa, s.a)} (${lengthText(la)}) to ${side(pb, s.b)} (${lengthText(lb)})`
            + `${Math.abs(la - lb) > 0.5 ? `: they differ by ${fmtLen(Math.abs(la - lb), 1)}` : ''}.`,
      }),
      field('Closed with', selectInput([['glued', 'glued lap seam'], ['zip', 'zip']], s.finish || 'glued', (v) => { s.finish = v; commit(); })),
    );
    if ((s.finish || 'glued') === 'glued') {
      g.append(field('Glue lap on', selectInput(self
        ? [['a', 'the pattern (trim it off one copy)'], ['none', 'neither (butted over a backing strip)']]
        : [['a', pa?.name], ['b', pb?.name], ['both', 'both'], ['none', 'neither (butted over a backing strip)']],
      s.lap || 'a', (v) => { s.lap = v; commit(); })));
    }
    if (!self) {
      g.append(el('p', { className: 'hint', textContent: 'The coloured dots mark the ends that meet. If they are on opposite ends, swap them.' }),
        el('div', { className: 'row' }, button('Swap ends', () => { s.reverse = !s.reverse; commit('Seam ends swapped'); })));
    }
    g.append(el('div', { className: 'row' }, button('Delete seam', () => deleteSelection(), 'quiet')));
  }
}

function finishFields(props) {
  const f = props.finish;
  const kind = f ? f.kind : '';
  const out = [field('Finish', selectInput([
    ['', 'hem allowance (pattern setting)'], ['raw', 'raw (cut on the line)'], ['hem', 'folded hem'], ['trim', 'trim strip'],
  ], kind, (v) => {
    if (!v) delete props.finish;
    else if (v === 'raw') props.finish = { kind: 'raw' };
    else if (v === 'hem') props.finish = { kind: 'hem', width_mm: f?.width_mm || 15 };
    else props.finish = { kind: 'trim', width_mm: f?.width_mm || 15, ease: f?.ease || 0.9 };
    commit();
  }))];
  if (f && f.kind !== 'raw') {
    out.push(field('Width (mm)', numInput(f.width_mm, (v) => { f.width_mm = Math.max(1, v); commit(); }, { min: 1, step: 1 })));
  }
  if (f && f.kind === 'trim') {
    out.push(field('Trim length (% of edge)', numInput(Math.round(100 * f.ease), (v) => { f.ease = Math.min(1, Math.max(0.5, v / 100)); commit(); }, { min: 50, max: 100, step: 1 })));
    out.push(el('p', { className: 'hint', textContent: 'Edges with the same name share one trim strip.' }));
  }
  return out;
}

function deleteSelection() {
  const sel = editor.sel;
  if (!sel) return;
  if (sel.kind === 'seam') {
    design.seams = design.seams.filter((s) => s.id !== sel.seam);
    editor.select(null);
    commit('Seam deleted');
  } else if (sel.kind === 'node') {
    const p = design.pieces.find((x) => x.id === sel.piece);
    if (p.nodes.length <= 3) { setStatus('A piece needs at least three points; delete the piece instead.', true); return; }
    p.nodes = p.nodes.filter((n) => n.id !== sel.node);
    editor.cache.delete(p.id);
    editor.select({ kind: 'piece', piece: p.id });
    commit('Point deleted');
  } else if (sel.piece) {
    const p = design.pieces.find((x) => x.id === sel.piece);
    if (!confirm(`Delete the piece "${p.name}" and its seams?`)) return;
    design.pieces = design.pieces.filter((x) => x.id !== sel.piece);
    design.seams = design.seams.filter((s) => s.a.piece !== sel.piece && s.b.piece !== sel.piece);
    editor.select(null);
    commit('Piece deleted');
  }
}
$('delete').onclick = deleteSelection;

// ---- tools and keys -------------------------------------------------------------
document.querySelectorAll('[data-tool]').forEach((b) => {
  b.onclick = () => setTool(b.dataset.tool);
});
const setTool = (t) => editor.setTool(t);
$('fit').onclick = () => editor.fit();
$('show-guides').onchange = () => { editor.showGuides = $('show-guides').checked; editor.render(); };
$('snap').onchange = () => { editor.snap = $('snap').checked; };
$('g-add-piece').onclick = () => {
  const y = lastGuide('waist') ?? 1000;
  const p = {
    id: newId('p'), name: `piece ${design.pieces.length + 1}`, pair: false, at: [nextX(), 0],
    nodes: [[0, y - 150], [200, y - 150], [200, y + 150], [0, y + 150]].map(([x, yy]) => ({ id: newId('n'), x, y: yy, kind: 'corner' })),
  };
  design.pieces.push(p);
  editor.select({ kind: 'piece', piece: p.id });
  commit('Piece added: drag its points to shape it');
  editor.fit();
};
const lastGuide = (name) => (editor.guides.find((g) => g[0] === name) || [])[1];
function nextX() {
  let x = 0;
  for (const p of design.pieces) {
    for (const q of editor.geometry(p).outline) x = Math.max(x, q[0]);
  }
  return design.pieces.length ? x + 60 : 0;
}
document.addEventListener('keydown', (e) => {
  const typing = /^(INPUT|SELECT|TEXTAREA)$/.test(document.activeElement?.tagName);
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z' && !typing) {
    e.preventDefault();
    if (e.shiftKey) restore(redo, undo); else restore(undo, redo);
    return;
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'y' && !typing) { e.preventDefault(); restore(redo, undo); return; }
  if (typing || e.ctrlKey || e.metaKey || e.altKey) return;
  if (e.key === 'Escape') { if (!editor.cancel()) editor.select(null); setTool(editor.tool); }
  else if (e.key === 'Enter' && editor.tool === 'draw') editor.finishDraw();
  else if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); deleteSelection(); }
  else if (e.key === 'f') editor.fit();
  else {
    const t = { v: 'select', a: 'add', p: 'draw', s: 'split', j: 'seam' }[e.key];
    if (t) setTool(t);
  }
});

// ---- templates -------------------------------------------------------------------
let templates = [];
(async () => {
  const r = await call({ command: 'templates' });
  templates = r.ok ? r.templates : [];
  $('g-template').replaceChildren(...templates.map((t) => el('option', { value: t.id, textContent: t.name })));
  showTemplateNote();
})();
function showTemplateNote() {
  const t = templates.find((x) => x.id === $('g-template').value);
  $('g-template-note').textContent = t ? t.description : '';
}
$('g-template').onchange = showTemplateNote;
async function startTemplate(id, ask = true) {
  const name = (templates.find((t) => t.id === id) || { name: id }).name;
  if (ask && design.pieces.length && !confirm(`Replace the current design with the ${name.toLowerCase()} template?`)) return;
  const r = await call({ command: 'template', id, body: readBody() });
  if (!r.ok) { setStatus(r.error, true); return; }
  setDesign(r.design);
  editor.select(null);
  commit(`${name}: drafted for these measurements.`);
  if ($('project-name').value === 'Untitled') $('project-name').value = name;
  requestAnimationFrame(() => editor.fit());
}
$('g-start').onclick = () => startTemplate($('g-template').value || 'top');

// ---- latex sheets ----------------------------------------------------------------
const DEFAULT_SHEETS = () => [{ name: 'black 0.40', color: '#151515', thickness_mm: 0.4, roll_width_mm: 1000 }];
let materials = DEFAULT_SHEETS();
let defaultSheetName = null;
const defaultSheet = () => materials.find((m) => m.name === defaultSheetName) || materials[0];
const sheetOf = (p) => materials.find((m) => m.name === p.sheet) || defaultSheet();
function table(elt, head, rows) {
  elt.replaceChildren();
  const tr = elt.insertRow();
  head.forEach((h) => tr.appendChild(el('th', { textContent: h })));
  rows.forEach((r) => { const row = elt.insertRow(); r.forEach((c) => { row.insertCell().textContent = c; }); });
}
function renderSheets() {
  const t = $('sheets');
  table(t, ['name', 'colour', 'thickness (mm)', 'roll width (mm)', 'ease', 'price / m', ''], []);
  materials.forEach((m, i) => {
    const row = t.insertRow();
    const input = (key, type, attrs = {}, optional = false) => {
      const e = el('input', { type, ...attrs });
      e.value = m[key] ?? '';
      e.onchange = () => {
        const v = e.value.trim();
        if (type === 'number') {
          if (optional && v === '') delete m[key]; else m[key] = Number(v);
        } else {
          const old = m.name;
          m[key] = v;
          if (key === 'name') {
            for (const p of design.pieces) if (p.sheet === old) p.sheet = v;
            if (defaultSheetName === old) defaultSheetName = v;
          }
        }
        renderSheets();
        sheetsChanged();
      };
      row.insertCell().appendChild(e);
    };
    input('name', 'text');
    input('color', 'color');
    input('thickness_mm', 'number', { min: 0.05, step: 0.05 });
    input('roll_width_mm', 'number', { min: 100, step: 50 });
    input('ease', 'number', { min: 0.5, max: 1.2, step: 0.01, placeholder: 'auto' }, true);
    input('price_per_m', 'number', { min: 0, step: 0.5, placeholder: '–' }, true);
    const del = el('button', { type: 'button', textContent: '✕', title: 'Remove sheet' });
    del.disabled = materials.length === 1;
    del.onclick = () => { materials.splice(i, 1); renderSheets(); sheetsChanged(); };
    row.insertCell().appendChild(del);
  });
  const sel = $('sheet-default');
  sel.replaceChildren(...materials.map((m) => el('option', { value: m.name, textContent: m.name })));
  sel.value = defaultSheet().name;
}
function sheetsChanged() { editor.render(); renderLists(); renderInspector(); saveLocal(); }
$('sheet-add').onclick = () => {
  const colours = ['#b01020', '#1d3fa0', '#1a7a3a', '#6a2a8a', '#c0c0c8'];
  materials.push({ name: `sheet ${materials.length + 1}`, color: colours[(materials.length - 1) % colours.length], thickness_mm: 0.4, roll_width_mm: 1000 });
  renderSheets();
  sheetsChanged();
};
$('sheet-default').onchange = () => { defaultSheetName = $('sheet-default').value; sheetsChanged(); };

// ---- settings ----------------------------------------------------------------------
const num = (id) => Number($(id).value);
function readSettings() {
  const sw = num('sheet-width');
  return {
    ease: num('ease') || 0.85,
    length_relief: Math.max(0, Math.min(100, num('length-relief'))) / 100,
    seam_allowance_mm: num('allowance'),
    hem_allowance_mm: num('hem'),
    zip_allowance_mm: num('zip'),
    notch_spacing_mm: num('notch'),
    seam_tolerance_mm: num('seam-tolerance'),
    sheet_width_mm: sw > 0 ? sw : null,
    paper: $('paper').value || null,
  };
}
const DEFAULT_SETTINGS = readSettings();
function writeSettings(s) {
  s = { ...DEFAULT_SETTINGS, ...(s || {}) };
  $('ease').value = s.ease;
  $('length-relief').value = Math.round(100 * s.length_relief);
  $('allowance').value = s.seam_allowance_mm;
  $('hem').value = s.hem_allowance_mm;
  $('zip').value = s.zip_allowance_mm;
  $('notch').value = s.notch_spacing_mm;
  $('seam-tolerance').value = s.seam_tolerance_mm;
  $('sheet-width').value = s.sheet_width_mm ?? '';
  $('paper').value = s.paper ?? '';
}
document.querySelectorAll('[data-pane="settings"] input, [data-pane="settings"] select').forEach((e) => {
  e.addEventListener('change', () => { saveLocal(); if ($('live').checked) check(); });
});

// ---- project: save, open, autosave ---------------------------------------------------
const STORAGE_KEY = 'latexpat.project.v2';
function readProject() {
  const p = {
    version: 2, name: $('project-name').value.trim() || 'Untitled', units, body: readBody(), design, materials,
    settings: readSettings(),
  };
  if (defaultSheetName) p.default_sheet = defaultSheetName;
  return p;
}
function writeProject(p) {
  $('project-name').value = p.name || 'Untitled';
  units = UNIT_MM[p.units] ? p.units : 'cm';
  $('units').value = units;
  writeBody(p.body || {});
  materials = Array.isArray(p.materials) && p.materials.length ? p.materials : DEFAULT_SHEETS();
  defaultSheetName = p.default_sheet || null;
  renderSheets();
  writeSettings(p.settings);
  setDesign(p.design || { pieces: [], seams: [] });
  undo.length = 0; redo.length = 0;
  saved = JSON.stringify(design);
  editor.select(null);
}
// A project from the 3D editor: keep its measurements and sheets only.
function from3d(p) {
  const shape = p.body?.shape || {};
  return {
    name: p.name, units: p.units, materials: p.materials,
    body: { figure: shape.body_type >= 0.5 ? 'men' : 'women', measurements: p.body?.measurements || {} },
    design: { pieces: [], seams: [] },
  };
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
const readFile = (input) => (input.files[0] ? input.files[0].text() : Promise.resolve(null));
$('project-save').onclick = () => {
  const p = readProject();
  fileLink(`${p.name.replace(/[^\w.-]+/g, '_')}.latexpat.json`, JSON.stringify(p, null, 2), 'application/json').click();
};
$('project-open').onchange = async () => {
  const text = await readFile($('project-open'));
  $('project-open').value = '';
  if (!text) return;
  try {
    let p = JSON.parse(text);
    const old = (p.version ?? 1) < 2 || p.garment;
    if (old) p = from3d(p);
    writeProject(p);
    afterChange();
    await updateBody({ redraft: false });
    requestAnimationFrame(() => editor.fit());
    setStatus(old ? 'This project was made with the 3D editor, which has been replaced: its measurements and sheets are loaded. Start a design from a template.'
      : `Opened ${p.name}.`);
  } catch (e) {
    $('project-status').textContent = `Could not open: ${e.message}`;
  }
};
$('project-new').onclick = async () => {
  if (!confirm('Start a new project? Unsaved changes to this one will be lost.')) return;
  writeProject({ name: 'Untitled', body: { figure: figure(), measurements: {} }, units });
  afterChange();
  await updateBody({ redraft: false });
};

// ---- output helpers --------------------------------------------------------------------
const urls = [];
function fileLink(name, text, type) {
  const url = URL.createObjectURL(new Blob([text], { type }));
  urls.push(url);
  return el('a', { href: url, download: name, textContent: name });
}
function svgImage(svg, alt) {
  const url = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml' }));
  urls.push(url);
  return el('img', { src: url, alt });
}
const f = (x, d = 1) => Number(x).toFixed(d);
function setStatus(text, error = false) {
  $('status').className = error ? 'error' : '';
  $('status').textContent = text;
}

function show(output) {
  urls.splice(0).forEach(URL.revokeObjectURL);
  const r = output.report;
  $('results').hidden = false;
  $('warnings').replaceChildren(...output.warnings.map((w) => el('li', { textContent: w })));

  const dl = $('downloads');
  dl.replaceChildren();
  if (output.pdf) dl.appendChild(fileLink('pattern.pdf', output.pdf, 'application/pdf'));
  r.pieces.forEach((p, i) => dl.appendChild(fileLink(p.file, output.piece_svgs[i], 'image/svg+xml')));
  if (output.layout_svg) dl.appendChild(fileLink('layout.svg', output.layout_svg, 'image/svg+xml'));
  dl.appendChild(fileLink('report.json', JSON.stringify(r, null, 2), 'application/json'));

  table($('piece-table'), ['piece', 'cut', 'sheet', 'ease', 'along', 'cut size (mm)', 'seams'],
    r.pieces.map((p) => [p.name, p.copies === 2 ? 'pair' : p.folded ? 'on fold' : '1', p.sheet || '–', f(p.ease, 2), f(p.ease_along, 2),
      `${f(p.cut_width_mm, 0)} × ${f(p.cut_height_mm, 0)}`, p.seams.join(', ')]));
  table($('seam-table'), ['seam', 'pieces', 'body (mm)', 'flat (mm)', 'mismatch (mm)'],
    r.seams.map((s) => [s.seam, `${r.pieces[s.a_piece].name} ↔ ${r.pieces[s.b_piece].name}`, f(s.a_body_mm),
      f(s.a_flat_mm), f(s.mismatch_mm, 2)]));

  const mout = $('materials-out');
  mout.replaceChildren();
  if (r.materials.length) {
    const t = el('table');
    mout.append(el('h3', { textContent: 'Materials' }), t);
    table(t, ['sheet', 'pieces', 'roll (mm)', 'length to buy', 'piece area', 'used', 'cost'],
      r.materials.map((u) => [u.sheet, u.pieces.map((i) => r.pieces[i].name).join(', '), f(u.roll_width_mm, 0), `${f(u.length_mm / 1000, 2)} m`,
        `${f(u.pieces_area_m2, 3)} m²`, `${f(100 * u.utilisation, 0)} %`, u.cost == null ? '–' : f(u.cost, 2)]));
    [...t.rows].slice(1).forEach((row, i) => {
      const sw = el('span', { className: 'swatch' });
      sw.style.background = r.materials[i].color;
      row.cells[0].prepend(sw);
    });
    r.materials.forEach((u, i) => {
      mout.append(el('h4', { textContent: `${u.sheet}: ${f(u.length_mm / 1000, 2)} m of a ${f(u.roll_width_mm, 0)} mm roll` }),
        svgImage(output.material_layouts[i], `layout on ${u.sheet}`));
      dl.appendChild(fileLink(u.file, output.material_layouts[i], 'image/svg+xml'));
    });
  }
  const aout = $('assembly-out');
  aout.replaceChildren();
  if (r.assembly.length) {
    aout.append(el('h3', { textContent: 'Assembly' }),
      el('ol', { id: 'assembly' }, ...r.assembly.map((step) => el('li', { textContent: step }))));
  }
  const tout = $('trims-out');
  tout.replaceChildren();
  if (r.trims.length) {
    tout.append(el('h3', { textContent: 'Trim strips' }));
    r.trims.forEach((t, i) => {
      const name = `trim_${String(i).padStart(2, '0')}.svg`;
      tout.append(el('h4', { textContent: `${t.edge}: ${f(t.length_mm, 0)} × ${f(t.width_mm, 0)} mm (edge ${f(t.edge_length_mm, 0)} mm)` }),
        svgImage(output.trim_svgs[i], `${t.edge} trim`));
      dl.appendChild(fileLink(name, output.trim_svgs[i], 'image/svg+xml'));
    });
  }
  const layout = $('layout');
  layout.replaceChildren();
  if (output.layout_svg) {
    layout.append(el('h3', { textContent: `Layout: ${f(r.layout.length_mm, 0)} mm of a ${f(r.layout.sheet_width_mm, 0)} mm sheet` }),
      svgImage(output.layout_svg, 'sheet layout'));
  }
  $('pieces').replaceChildren(...output.piece_svgs.map((svg, i) => el('div', { className: 'piece' },
    svgImage(svg, r.pieces[i].name), el('div', {}, fileLink(r.pieces[i].file, svg, 'image/svg+xml')))));
}

// ---- make the pattern -------------------------------------------------------------------
async function run() {
  setStatus('Working…');
  $('run').disabled = true;
  try {
    const p = readProject();
    const s = p.settings;
    const calib = await readFile($('calib-file'));
    const options = {
      ease: calib ? { calibrated: { calibration: JSON.parse(calib), sheet: $('calib-sheet').value.trim() || `${defaultSheet().thickness_mm}mm` } }
        : { uniform: s.ease },
      length_relief: s.length_relief,
      seam_tolerance: s.seam_tolerance_mm,
      pattern: {
        seam_allowance: s.seam_allowance_mm,
        hem_allowance: s.hem_allowance_mm,
        notch_spacing: s.notch_spacing_mm > 0 ? s.notch_spacing_mm : null,
        zip_allowance: s.zip_allowance_mm,
      },
      sheet: s.sheet_width_mm ? { sheet_width: s.sheet_width_mm, gap: 5, allow_rotation: true } : null,
      print: s.paper ? { paper: s.paper } : null,
      materials,
      default_sheet: defaultSheet().name,
    };
    const t0 = performance.now();
    const res = await call({ command: 'pattern', design, options });
    if (!res.ok) throw new Error(res.error);
    show(res.output);
    setStatus(`Done in ${f((performance.now() - t0) / 1000, 1)} s: ${res.output.report.pieces.length} pieces.`);
  } catch (e) {
    setStatus(e.message || String(e), true);
  } finally {
    $('run').disabled = false;
  }
}
$('run').onclick = run;

// ---- start ----------------------------------------------------------------------------------
(async () => {
  let stored = null;
  try { stored = JSON.parse(localStorage.getItem(STORAGE_KEY)); } catch { /* ignore */ }
  setTool('select');
  if (stored) {
    writeProject(stored);
    afterChange();
    await updateBody({ redraft: false });
    requestAnimationFrame(() => editor.fit());
  } else {
    writeProject({ name: 'Untitled', body: { figure: 'women', measurements: {} }, units: 'cm' });
    await updateBody({ redraft: false });
    await startTemplate('top', false);
  }
})();
