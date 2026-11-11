// Browser smoke test for the web UI (needs Playwright with Chromium).
//   web/build.sh && python3 -m http.server -d web 8765 &
//   node web/smoke-test.cjs http://localhost:8765/
// Set PW_CHANNEL=chrome to use an installed Google Chrome.
// Run from the repository root (it uploads files from tests/data).
const { chromium } = require('playwright');
const assert = require('assert');

const url = process.argv[2] || 'http://localhost:8765/';
const done = (p) => p.waitForFunction(
  () => !document.getElementById('run').disabled && document.getElementById('status').textContent !== 'Working…',
  null, { timeout: 60000 });

(async () => {
  // PW_CHANNEL=chrome uses an installed Google Chrome instead of
  // Playwright's own Chromium (as in CI). SwiftShader gives WebGL without a GPU.
  const browser = await chromium.launch({
    channel: process.env.PW_CHANNEL || undefined,
    args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader'],
  });
  const page = await browser.newPage({ viewport: { width: 1250, height: 1000 } });
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.goto(url);

  // 0. Full body (the default): the example project fits without warnings.
  const fitted = () => page.waitForFunction(
    () => /Fitted|error|unavailable/i.test(document.getElementById('fit-status').textContent)
      && !document.getElementById('fit-body').disabled,
    null, { timeout: 60000 });
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await fitted();
  assert.match(await page.textContent('#fit-status'), /Fitted/);
  assert.equal(await page.locator('#fit-warnings li').count(), 0);
  assert.equal(await page.locator('#fit-table tr').count(), 16);
  // An unreachable combination is flagged, and the project autosaves.
  await page.fill('#m-underbust', '120');
  await page.click('#fit-body');
  await fitted();
  assert.ok(await page.locator('#fit-warnings li').count() > 0);
  await page.reload();
  await fitted();
  assert.equal(await page.inputValue('#m-underbust'), '120');
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await fitted();

  // 0b. Garment on the full body: load the example, draw a curve by
  // clicking, remove it again, and make the pattern.
  page.on('dialog', (d) => d.accept());
  const garmentReady = () => page.waitForFunction(
    () => /Ready|cannot|error/i.test(document.getElementById('g-status').textContent),
    null, { timeout: 60000 });
  // Templates: the cincher makes four pieces; then work from the top.
  await page.waitForFunction(() => document.querySelectorAll('#g-template option').length >= 4);
  await page.selectOption('#g-template', 'cincher');
  assert.match(await page.textContent('#g-template-note'), /back zip/);
  await page.click('#g-example');
  await page.waitForFunction(() => design.curves.some((c) => c.name === 'bottom'));
  await garmentReady();
  assert.match(await page.textContent('#g-status'), /4 flat pieces/);
  await page.selectOption('#g-template', 'top');
  await page.click('#g-example');
  await page.waitForFunction(() => /2 flat pieces/.test(document.getElementById('g-status').textContent), null, { timeout: 60000 });
  await page.click('#g-new-seam');
  await page.locator('#viewer').scrollIntoViewIfNeeded();
  const box = await page.locator('#viewer').boundingBox();
  await page.mouse.click(box.x + box.width * 0.5, box.y + box.height * 0.42);
  await page.waitForFunction(() => /1 point/.test(document.getElementById('g-curves').textContent), null, { timeout: 30000 });
  await page.click('#g-delete');
  await page.waitForFunction(() => document.querySelectorAll('#g-curves li').length === 5, null, { timeout: 30000 });
  // Drag the hem's most frontal point down: it moves, no point is added,
  // and the view does not turn.
  await page.locator('#g-curves li').first().click();
  const t = await page.evaluate(() => {
    const pts = design.curves[active].points.map((sp, i) => ({ i, q: pointOf(sp) }));
    pts.sort((a, b) => b.q[2] - a.q[2]);
    const s = viewer.project(pts[0].q);
    return { i: pts[0].i, x: s.x, y: s.y, y3: pts[0].q[1], n: pts.length, yaw: viewer.yaw };
  });
  await page.mouse.move(box.x + t.x, box.y + t.y);
  await page.mouse.down();
  for (let k = 1; k <= 8; k++) await page.mouse.move(box.x + t.x, box.y + t.y + 5 * k);
  await page.mouse.up();
  const moved = await page.evaluate((i) => ({ y3: pointOf(design.curves[active].points[i])[1], n: design.curves[active].points.length, yaw: viewer.yaw }), t.i);
  assert.ok(moved.y3 < t.y3 - 20, `dragged point should move down: ${t.y3} -> ${moved.y3}`);
  assert.equal(moved.n, t.n);
  assert.equal(moved.yaw, t.yaw);
  await garmentReady();
  // Paint the back with the example's second sheet (turn the view to the
  // back first): one piece turns red.
  await page.evaluate(() => { viewer.yaw += Math.PI; viewer.draw(); });
  await page.selectOption('#g-paint-sheet', 'red 0.40');
  await page.click('#g-paint');
  await page.locator('#viewer').scrollIntoViewIfNeeded();
  const vbox = await page.locator('#viewer').boundingBox();
  await page.mouse.click(vbox.x + vbox.width * 0.5, vbox.y + vbox.height * 0.45);
  await page.waitForFunction(() => design.sheet_assignments.length === 1, null, { timeout: 10000 });
  // The preview refreshes asynchronously: wait for exactly one red piece.
  await page.waitForFunction(
    () => viewer.overlay && viewer.overlay.pieces
      && viewer.overlay.pieces.filter((p) => p.color[0] > 0.5).length === 1
      && viewer.overlay.pieces.length === 2,
    null, { timeout: 60000 });
  await page.click('#g-paint');
  await page.evaluate(() => { viewer.yaw -= Math.PI; viewer.draw(); });

  // An ease zone by clicking the body, then the pattern with lengthwise
  // relief: the pressure map appears and the table has pressure columns.
  await page.click('#g-zone');
  await page.locator('#viewer').scrollIntoViewIfNeeded();
  const zbox = await page.locator('#viewer').boundingBox();
  await page.mouse.click(zbox.x + zbox.width * 0.5, zbox.y + zbox.height * 0.42);
  await page.waitForFunction(() => document.querySelectorAll('#g-zones tr').length === 2, null, { timeout: 30000 });
  await garmentReady();
  // Finishes: the example's hem is folded; make the side seam a zip.
  await page.locator('#g-curves li', { hasText: 'hem (edge' }).click();
  assert.equal(await page.inputValue('#g-edge-finish'), 'hem');
  assert.ok(await page.isHidden('#g-seam-finish-wrap'));
  await page.locator('#g-curves li', { hasText: /^side \(seam/ }).click();
  await page.selectOption('#g-seam-finish', 'zip');
  assert.match(await page.locator('#g-curves').innerText(), /side \(seam.*zip\)/);
  await page.fill('#length-relief', '40');
  await page.click('#run');
  await page.waitForFunction(
    () => !document.getElementById('run').disabled && /Done|error/i.test(document.getElementById('status').textContent),
    null, { timeout: 90000 });
  assert.match(await page.textContent('#status'), /Done .*2 panels/);
  assert.ok(await page.isVisible('#pressure-panel'));
  assert.match(await page.locator('#panel-table').innerText(), /mmHg/);
  // One layout per sheet, with a materials list.
  assert.equal(await page.locator('#materials-out img').count(), 2);
  assert.match(await page.locator('#materials-out table').innerText(), /red 0\.40/);
  // Construction: the assembly guide, the neckline's trim strip and the PDF.
  const steps = await page.locator('#assembly li').allInnerTexts();
  assert.ok(steps.some((s) => /^Fit the zip in the side/.test(s)), steps.join('\n'));
  assert.ok(steps.some((s) => /^Hem the hem edge/.test(s)), steps.join('\n'));
  assert.equal(await page.locator('#trims-out img').count(), 1);
  const pdfHref = await page.locator('#downloads a', { hasText: 'pattern.pdf' }).getAttribute('href');
  const pdfHead = await page.evaluate(async (u) => (await (await fetch(u)).text()).slice(0, 8), pdfHref);
  assert.equal(pdfHead, '%PDF-1.4');
  await page.fill('#length-relief', '100');
  await page.evaluate(() => localStorage.clear());

  // 1. Quick torso from the default measurements, with heatmap and sheet layout.
  await page.check('input[name=source][value=measure]');
  await page.selectOption('#heatmap', 'max_stretch');
  await page.fill('#sheet-width', '1000');
  await page.click('#run');
  await done(page);
  assert.match(await page.textContent('#status'), /Done .*6 panels/);
  assert.equal(await page.locator('.piece img').count(), 6);
  assert.equal(await page.locator('#warnings li').count(), 0);
  assert.equal(await page.locator('#layout img').count(), 1);

  // 2. Invalid measurements give a readable error.
  await page.fill('#levels tbody tr:nth-child(2) td:nth-child(2) input', '900');
  await page.click('#run');
  await done(page);
  assert.match(await page.textContent('#status'), /bottom to top/);

  // 3. Uploaded mesh + seams with a swatch calibration.
  await page.check('input[name=source][value=upload]');
  await page.setInputFiles('#obj-file', 'tests/data/cap.obj');
  await page.setInputFiles('#seams-file', 'tests/data/cap.ring.seams.json');
  await page.selectOption('#up', '0,0,1');
  await page.click('#calib-details summary');
  await page.setInputFiles('#calib-file', 'tests/data/swatches.json');
  await page.fill('#calib-sheet', '0.40mm black');
  await page.click('#run');
  await done(page);
  assert.match(await page.textContent('#status'), /Done .*2 panels/);

  assert.deepEqual(errors, []);
  await browser.close();
  console.log('web smoke test passed');
})().catch((e) => { console.error(e); process.exit(1); });
