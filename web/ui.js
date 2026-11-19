// Workspace layout: tabbed side panels on wide screens, one screen at a time
// with a bottom tab bar on phones. app.js owns the behaviour; this file
// only arranges the panes and reacts to what app.js shows.
'use strict';
(() => {
  const $id = (id) => document.getElementById(id);
  const narrow = matchMedia('(max-width: 900px)');
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
  // On a phone the inspector sits at the top of the Garment screen and the
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
    if (name === 'design') requestAnimationFrame(() => editor.render());
  }
  for (const b of document.querySelectorAll('.mobilebar button')) b.onclick = () => showScreen(b.dataset.pane);

  const inspector = document.querySelector('.pane[data-pane="inspector"]');
  const inspectorBits = [$id('inspector-empty'), $id('inspector')];
  const sheetsPane = document.querySelector('.pane[data-pane="sheets"]');
  const sheetsBits = [...sheetsPane.children];
  function arrange() {
    if (narrow.matches) {
      document.querySelector('.pane[data-pane="garment"]').prepend(...inspectorBits);
      document.querySelector('.pane[data-pane="settings"]').append(...sheetsBits);
      showScreen(document.body.dataset.screen || 'design');
    } else {
      inspector.append(...inspectorBits);
      sheetsPane.append(...sheetsBits);
      showTab('left', localStorage.getItem('latexpat.tab.left') || 'garment');
      showTab('right', localStorage.getItem('latexpat.tab.right') || 'inspector');
    }
    requestAnimationFrame(() => editor.render());
  }
  narrow.addEventListener('change', arrange);
  arrange();

  // ---- follow what app.js shows ---------------------------------------------
  const watch = (el, fn, opts) => new MutationObserver(fn).observe(el, opts);
  // Selecting something opens the inspector.
  watch($id('inspector'), () => {
    if ($id('inspector').children.length && !narrow.matches) showTab('right', 'inspector');
  }, { childList: true });
  watch($id('pieces'), () => {
    $id('pattern-empty').hidden = $id('pieces').children.length > 0;
  }, { childList: true });
  watch($id('results'), () => {
    $id('report-empty').hidden = !$id('results').hidden;
  }, { attributes: true, attributeFilter: ['hidden'] });
  let runs = 0;
  watch($id('status'), () => {
    if (/^Done/.test($id('status').textContent)) {
      runs += 1;
      if (narrow.matches) {
        if (document.body.dataset.screen !== 'design' || runs === 1) showScreen('pattern');
      } else if (!$id('live').checked) {
        showTab('right', 'report');
      }
    }
  }, { childList: true, characterData: true, subtree: true });

  // Warnings, counted in the status bar.
  const warnButton = $id('status-warnings');
  function countWarnings() {
    const n = document.querySelectorAll('#warnings li, #g-warnings li').length;
    warnButton.hidden = n === 0;
    warnButton.textContent = `${n} warning${n === 1 ? '' : 's'}`;
  }
  for (const id of ['warnings', 'g-warnings']) watch($id(id), countWarnings, { childList: true });
  warnButton.onclick = () => {
    const where = document.querySelector('#warnings li') ? 'report' : 'garment';
    if (narrow.matches) showScreen(where);
    else showTab(sideOf(where), where);
  };
  countWarnings();

  // Piece size in the pattern view.
  const zoom = $id('piece-zoom');
  const setZoom = () => $id('pieces').style.setProperty('--piece-w', `${zoom.value}px`);
  zoom.oninput = setZoom;
  setZoom();
  $id('live').onchange = () => { if ($id('live').checked) check(); };
})();
