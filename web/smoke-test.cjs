// Browser smoke test for the web UI (needs Playwright with Chromium).
//   web/build.sh && python3 -m http.server -d web 8765 &
//   node web/smoke-test.cjs http://localhost:8765/
// Set PW_CHANNEL=chrome to use an installed Google Chrome, or PW_EXECUTABLE
// to the path of another Chromium.
const { chromium, devices } = require('playwright');
const assert = require('assert');

const url = process.argv[2] || 'http://localhost:8765/';

(async () => {
  const browser = await chromium.launch({
    channel: process.env.PW_CHANNEL || undefined,
    executablePath: process.env.PW_EXECUTABLE || undefined,
  });
  const page = await browser.newPage({ viewport: { width: 1350, height: 950 } });
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  page.on('dialog', (d) => d.accept());
  const tab = (name) => page.click(`.tabbar button[data-pane="${name}"]`);
  const ready = () => page.waitForFunction(() => /^Ready|Fix these|No pieces/.test(document.getElementById('g-status').textContent), null, { timeout: 30000 });
  const made = () => page.waitForFunction(
    () => !document.getElementById('run').disabled && /^Done|error|design:/i.test(document.getElementById('status').textContent),
    null, { timeout: 60000 });
  // Screen position (page coordinates) of a world point.
  const screen = (w) => page.evaluate((w) => {
    const r = document.getElementById('editor').getBoundingClientRect();
    const [x, y] = editor.toScreen(w);
    return [x + r.left, y + r.top];
  }, w);
  // A point on edge k of piece i, at fraction t, in world coordinates.
  const onEdge = (i, k, t) => page.evaluate(([i, k, t]) => Geo.atFraction(editor.geometry(design.pieces[i]).edges[k], t).point, [i, k, t]);
  const clickWorld = async (w) => { const [x, y] = await screen(w); await page.mouse.click(x, y); };

  // 1. A fresh start drafts the sleeveless top for the typical figure.
  await page.goto(url);
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await page.waitForFunction(() => design.pieces.length === 2);
  await ready();
  assert.match(await page.textContent('#g-status'), /Ready: 2 pieces, 2 seams/);
  assert.equal(await page.locator('#g-warnings li').count(), 0);
  assert.ok((await page.getAttribute('#m-bust', 'placeholder')).length > 0, 'estimates shown');
  assert.ok(await page.evaluate(() => editor.guides.length > 8));
  // The editor's curves are the pattern's: every edge length agrees with Rust.
  const worst = await page.evaluate(async () => {
    const r = await call({ command: 'check', design });
    let w = 0;
    design.pieces.forEach((p, i) => r.pieces[i].edges.forEach((e, k) => { w = Math.max(w, Math.abs(e.length - Geo.edgeLength(p, k))); }));
    return w;
  });
  assert.ok(worst < 1e-6, `JS and Rust edge lengths differ by ${worst} mm`);

  // 2. New measurements redraft the design; undo goes back.
  await tab('body');
  await page.fill('#m-bust', '100');
  await page.waitForFunction(() => design.block.body.measurements.bust === 1000, null, { timeout: 10000 });
  const wider = await page.evaluate(() => Geo.edgeLength(design.pieces[0], 1));
  await page.click('#editor');
  await page.keyboard.press('Control+z');
  assert.equal(await page.evaluate(() => design.block.body.measurements.bust), undefined);
  assert.ok(await page.evaluate(() => Geo.edgeLength(design.pieces[0], 1)) !== wider);
  await page.keyboard.press('Control+Shift+z');
  assert.equal(await page.evaluate(() => design.block.body.measurements.bust), 1000);
  // An impossible body is explained, not drafted.
  await page.fill('#m-inseam', '150');
  await page.waitForFunction(() => /inseam/.test(document.getElementById('body-status').textContent));
  await page.fill('#m-inseam', '');

  // 3. Select a piece and an edge; the inspector edits them.
  await tab('garment');
  const mid = await page.evaluate(() => {
    const o = editor.geometry(design.pieces[1]).outline;
    const xs = o.map((q) => q[0]), ys = o.map((q) => q[1]);
    return [(Math.min(...xs) + Math.max(...xs)) / 2, (Math.min(...ys) + Math.max(...ys)) / 2];
  });
  await clickWorld(mid);
  await page.waitForFunction(() => editor.sel && editor.sel.kind === 'piece' && editor.sel.piece === 'back');
  await page.check('#inspector label.check input');
  assert.equal(await page.evaluate(() => design.pieces[1].pair), true);
  await clickWorld(await onEdge(0, 0, 0.5));   // the front hem
  await page.waitForFunction(() => editor.sel && editor.sel.kind === 'edge');
  assert.match(await page.textContent('#inspector'), /Edge of front/);
  // Turn the front's arrow a quarter turn anticlockwise (up the body to the
  // page's left), then undo.
  const arrow = await page.evaluate(() => editor.arrow(design.pieces[0]));
  const [hx, hy] = await screen(arrow.head);
  const [cx, cy] = await screen(arrow.c);
  await page.mouse.move(hx, hy);
  await page.mouse.down();
  await page.mouse.move(cx - Math.abs(hy - cy), cy, { steps: 6 });
  await page.mouse.up();
  assert.equal(await page.evaluate(() => design.pieces[0].up_deg), 90);
  await page.keyboard.press('Control+z');
  assert.equal(await page.evaluate(() => design.pieces[0].up_deg), undefined);

  // 4. Drag a point: select the front, then move its centre neck point down.
  const neck = await page.evaluate(() => {
    const p = design.pieces[0];
    const n = p.nodes.find((x) => x.id === 'front-centre-neck');
    return [n.x + p.at[0], n.y + p.at[1]];
  });
  await clickWorld(mid.map((v, i) => (i ? v : v - 400)).map((v) => v));  // background: deselect
  await clickWorld(await page.evaluate(() => { const o = editor.geometry(design.pieces[0]).outline; return [o[0][0] + 40, o[0][1] + 60]; }));
  const [nx, ny] = await screen(neck);
  await page.mouse.move(nx, ny);
  await page.mouse.down();
  await page.mouse.move(nx, ny + 30, { steps: 5 });
  await page.mouse.up();
  const moved = await page.evaluate(() => design.pieces[0].nodes.find((x) => x.id === 'front-centre-neck').y);
  assert.ok(moved < neck[1] - 10, `neck moved: ${moved} vs ${neck[1]}`);

  // 5. Split the back with a seam, then make the pattern.
  await page.click('[data-tool="split"]');
  await clickWorld(await onEdge(1, 0, 0.5));   // back hem
  await clickWorld(await onEdge(1, 4, 0.5));   // back neckline
  await page.waitForFunction(() => design.pieces.length === 3 && design.seams.length === 3);
  await ready();
  assert.equal(await page.locator('#g-warnings li').count(), 0, await page.textContent('#g-warnings'));
  await page.click('#run');
  await made();
  assert.match(await page.textContent('#status'), /Done/, await page.textContent('#status'));
  assert.equal(await page.locator('#pieces .piece').count(), 3);
  assert.ok(await page.locator('#downloads a', { hasText: 'pattern.pdf' }).count() === 1);
  assert.ok((await page.locator('#assembly li').count()) >= 4);

  // The part split off still follows the measurements.
  const before = await page.evaluate(() => JSON.stringify(design.pieces[2].nodes));
  await tab('body');
  await page.fill('#m-hips', '110');
  await page.waitForFunction(() => design.block.body.measurements.hips === 1100, null, { timeout: 10000 });
  assert.notEqual(await page.evaluate(() => JSON.stringify(design.pieces[2].nodes)), before);
  await page.fill('#m-hips', '');
  await page.waitForFunction(() => design.block.body.measurements.hips === undefined, null, { timeout: 10000 });
  await tab('garment');

  // 6. Draw a new piece: a triangle.
  await page.evaluate(() => editor.zoomAt([0, 0], 0.5));
  const box = await page.locator('#editor').boundingBox();
  await page.click('[data-tool="draw"]');
  for (const [x, y] of [[30, 40], [150, 40], [90, 130], [30, 40]]) await page.mouse.click(box.x + x, box.y + y);
  await page.waitForFunction(() => design.pieces.length === 4);
  await page.keyboard.press('Delete');
  await page.waitForFunction(() => design.pieces.length === 3);

  // 7. Join two edges with a seam, then delete it.
  await page.click('#fit');
  await page.click('[data-tool="seam"]');
  await clickWorld(await onEdge(0, 0, 0.3));
  await clickWorld(await onEdge(1, 0, 0.5));
  await page.waitForFunction(() => design.seams.length === 4);
  await page.keyboard.press('Delete');
  await page.waitForFunction(() => design.seams.length === 3);

  // 8. The project is kept in the browser.
  await page.reload();
  await page.waitForFunction(() => design.pieces.length === 3);
  assert.equal(await page.inputValue('#m-bust'), '100');

  // 9. Another template, and its pattern with a sheet layout.
  await tab('garment');
  await page.waitForFunction(() => document.querySelectorAll('#g-template option').length >= 7);
  await page.selectOption('#g-template', 'catsuit-sleeves');
  await page.click('#g-start');
  await page.waitForFunction(() => design.pieces.length === 3 && design.pieces[2].id === 'sleeve');
  await tab('settings');
  await page.fill('#sheet-width', '1200');
  await page.press('#sheet-width', 'Tab');
  await page.click('#run');
  await made();
  assert.match(await page.textContent('#status'), /Done/);
  assert.equal(await page.locator('#warnings li').count(), 0, await page.textContent('#warnings'));
  assert.match(await page.textContent('#layout'), /Layout/);

  // 10. A project from the 3D editor keeps its measurements.
  await page.setInputFiles('#project-open', {
    name: 'old.latexpat.json', mimeType: 'application/json',
    buffer: Buffer.from(JSON.stringify({ version: 1, name: 'Old', body: { measurements: { height: 1750, bust: 960 }, shape: { body_type: 1 } }, garment: { curves: [] } })),
  });
  await page.waitForFunction(() => /3D editor/.test(document.getElementById('status').textContent));
  assert.equal(await page.inputValue('#m-height'), '175');
  assert.equal(await page.evaluate(() => readBody().figure), 'men');
  assert.equal(await page.evaluate(() => design.pieces.length), 0);

  assert.deepEqual(errors, []);

  // 11. Phone: one screen at a time.
  const phone = await browser.newPage({ ...devices['iPhone 13'] });
  phone.on('pageerror', (e) => errors.push(String(e)));
  await phone.goto(url);
  await phone.evaluate(() => localStorage.clear());
  await phone.reload();
  await phone.waitForFunction(() => design.pieces.length === 2);
  assert.ok(await phone.isVisible('#editor'));
  await phone.click('.mobilebar button[data-pane="body"]');
  assert.ok(await phone.isVisible('#m-bust'));
  assert.ok(!(await phone.isVisible('#editor')));
  await phone.click('#run');
  await phone.waitForFunction(() => document.body.dataset.screen === 'pattern', null, { timeout: 60000 });
  assert.equal(await phone.locator('#pieces .piece').count(), 2);

  assert.deepEqual(errors, []);
  await browser.close();
  console.log('smoke test passed');
})().catch((e) => { console.error(e); process.exit(1); });
