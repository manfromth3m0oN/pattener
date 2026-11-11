// Browser smoke test for the web UI (needs Playwright with Chromium).
//   web/build.sh && python3 -m http.server -d web 8765 &
//   node web/smoke-test.cjs http://localhost:8765/
// Run from the repository root (it uploads files from tests/data).
const { chromium } = require('playwright');
const assert = require('assert');

const url = process.argv[2] || 'http://localhost:8765/';
const done = (p) => p.waitForFunction(
  () => !document.getElementById('run').disabled && document.getElementById('status').textContent !== 'Working…',
  null, { timeout: 60000 });

(async () => {
  const browser = await chromium.launch({ args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader'] });
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
  await page.click('#g-example');
  await garmentReady();
  assert.match(await page.textContent('#g-status'), /2 flat pieces/);
  await page.click('#g-new-seam');
  await page.locator('#viewer').scrollIntoViewIfNeeded();
  const box = await page.locator('#viewer').boundingBox();
  await page.mouse.click(box.x + box.width * 0.5, box.y + box.height * 0.42);
  await page.waitForFunction(() => /1 point/.test(document.getElementById('g-curves').textContent), null, { timeout: 30000 });
  await page.click('#g-delete');
  await page.waitForFunction(() => document.querySelectorAll('#g-curves li').length === 5, null, { timeout: 30000 });
  await page.click('#run');
  await page.waitForFunction(
    () => !document.getElementById('run').disabled && /Done|error/i.test(document.getElementById('status').textContent),
    null, { timeout: 60000 });
  assert.match(await page.textContent('#status'), /Done .*2 panels/);
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
