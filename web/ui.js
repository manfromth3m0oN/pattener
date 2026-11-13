// Workspace layout: tabbed side panels on wide screens, one screen at a time
// with a bottom tab bar on phones. app.js owns the behaviour; this file
// only arranges the panes and reacts to what app.js shows.
'use strict';
(() => {
  const $id = (id) => document.getElementById(id);
  const narrow = matchMedia('(max-width: 900px)');
  const view3d = () => (typeof viewer !== 'undefined' ? viewer : null);
  const panes = (side) => [...document.querySelectorAll(`.sidebar.${side} .pane`)];

  // ---- side panels --------------------------------------------------------
  function showTab(side, name) {
    for (const p of panes(side)) p.hidden = p.dataset.pane !== name;
    for (const b of document.querySelectorAll(`.sidebar.${side} .tabbar button`)) {
      b.setAttribute('aria-selected', String(b.dataset.pane === name));
    }
    localStorage.setItem(`latexpat.tab.${side}`, name);
  }
  for (const side of ['left', 'right']) {
    for (const b of document.querySelectorAll(`.sidebar.${side} .tabbar button`)) {
      b.onclick = () => showTab(side, b.dataset.pane);
    }
  }

  // ---- phones: one screen at a time ----------------------------------------
  // Each screen is a pane of a side panel or one of the two views. On a
  // phone the inspector sits at the top of the Garment screen and the
  // sheets at the end of the Settings screen.
  const sideOf = (name) => (document.querySelector(`.sidebar.left .pane[data-pane="${name}"]`) ? 'left'
    : document.querySelector(`.sidebar.right .pane[data-pane="${name}"]`) ? 'right' : null);
  function showScreen(name) {
    document.body.dataset.screen = name;
    for (const b of document.querySelectorAll('.mobilebar button')) {
      b.setAttribute('aria-selected', String(b.dataset.pane === name));
    }
    const side = sideOf(name);
    if (side) showTab(side, name);
    // The 3D canvas may have been hidden: let it size itself again.
    if (name === '3d' && view3d()) requestAnimationFrame(() => view3d().draw());
  }
  for (const b of document.querySelectorAll('.mobilebar button')) b.onclick = () => showScreen(b.dataset.pane);

  const inspector = document.querySelector('.pane[data-pane="inspector"]');
  const inspectorBits = [$id('inspector-empty'), $id('g-active')];
  const sheetsPane = document.querySelector('.pane[data-pane="sheets"]');
  const sheetsBits = [...sheetsPane.children];
  function arrange() {
    if (narrow.matches) {
      const garment = document.querySelector('.pane[data-pane="garment"]');
      garment.prepend(...inspectorBits);
      document.querySelector('.pane[data-pane="settings"]').append(...sheetsBits);
      showScreen(document.body.dataset.screen || '3d');
    } else {
      inspector.append(...inspectorBits);
      sheetsPane.append(...sheetsBits);
      showTab('left', localStorage.getItem('latexpat.tab.left') || 'body');
      showTab('right', localStorage.getItem('latexpat.tab.right') || 'settings');
    }
    if (view3d()) requestAnimationFrame(() => view3d().draw());
  }
  narrow.addEventListener('change', arrange);
  // The 3D background follows the light / dark theme.
  matchMedia('(prefers-color-scheme: light)').addEventListener('change', () => view3d() && view3d().draw());
  arrange();

  // ---- follow what app.js shows ---------------------------------------------
  const watch = (el, fn, opts) => new MutationObserver(fn).observe(el, opts);
  // Selecting or drawing a curve opens the inspector.
  watch($id('g-active'), () => {
    const shown = !$id('g-active').hidden;
    $id('inspector-empty').hidden = shown;
    if (shown && !narrow.matches) showTab('right', 'inspector');
  }, { attributes: true, attributeFilter: ['hidden'] });
  // A new pattern: show the pieces and the report.
  watch($id('pieces'), () => {
    const has = $id('pieces').children.length > 0;
    $id('pattern-empty').hidden = has;
  }, { childList: true });
  watch($id('results'), () => {
    const shown = !$id('results').hidden;
    $id('report-empty').hidden = shown;
  }, { attributes: true, attributeFilter: ['hidden'] });
  let runs = 0;
  watch($id('status'), () => {
    const t = $id('status').textContent;
    if (/^Done/.test(t)) {
      runs += 1;
      if (narrow.matches) {
        if (document.body.dataset.screen !== '3d' || runs === 1) showScreen('pattern');
      } else if (!$id('live').checked) {
        showTab('right', 'report');
      }
    }
  }, { childList: true, characterData: true, subtree: true });

  // Warnings, counted in the status bar.
  const warnButton = $id('status-warnings');
  function countWarnings() {
    const n = document.querySelectorAll('#warnings li, #g-warnings li, #fit-warnings li').length;
    warnButton.hidden = n === 0;
    warnButton.textContent = `${n} warning${n === 1 ? '' : 's'}`;
  }
  for (const id of ['warnings', 'g-warnings', 'fit-warnings']) watch($id(id), countWarnings, { childList: true });
  warnButton.onclick = () => {
    const where = document.querySelector('#warnings li') ? 'report'
      : document.querySelector('#g-warnings li') ? 'garment' : 'body';
    if (narrow.matches) showScreen(where);
    else showTab(sideOf(where), where);
  };
  countWarnings();

  // Piece size in the pattern view.
  const zoom = $id('piece-zoom');
  const setZoom = () => $id('pieces').style.setProperty('--piece-w', `${zoom.value}px`);
  zoom.oninput = setZoom;
  setZoom();

  // Live: make the pattern again whenever the garment preview is ready.
  let liveTimer = null;
  watch($id('g-status'), () => {
    if (!$id('live').checked || !/^Ready/.test($id('g-status').textContent)) return;
    clearTimeout(liveTimer);
    liveTimer = setTimeout(() => { if (!$id('run').disabled) $id('run').click(); }, 300);
  }, { childList: true, characterData: true, subtree: true });
  $id('live').onchange = () => {
    if ($id('live').checked && /^Ready/.test($id('g-status').textContent)) $id('run').click();
  };
})();
