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
  const browser = await chromium.launch();
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.goto(url);

  // 1. Torso from the default measurements, with heatmap and sheet layout.
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
  await page.click('details summary');
  await page.setInputFiles('#calib-file', 'tests/data/swatches.json');
  await page.fill('#calib-sheet', '0.40mm black');
  await page.click('#run');
  await done(page);
  assert.match(await page.textContent('#status'), /Done .*2 panels/);

  assert.deepEqual(errors, []);
  await browser.close();
  console.log('web smoke test passed');
})().catch((e) => { console.error(e); process.exit(1); });
