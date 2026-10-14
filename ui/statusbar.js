'use strict';
// Строка состояния внизу окна: проект и ветка, что идёт, последний CI, время разбора, откуда база,
// размер окна и кнопка «Рекомендованный размер». Тексты - knowledge/ui.json (раздел statusbar).
// Подключается после app.js и берёт его общие помощники: S, call, esc, tpl, UI, P и другие.
(() => {
  const T = () => UI().statusbar || {};
  const REC = { w: 1320, h: 900 };
  // стрелки своими значками: шрифтовые ↑ ↓ Windows рисует цветными эмодзи
  const svg = (d) => `<svg width="10" height="10" viewBox="0 0 12 12" aria-hidden="true"><path d="${d}" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
  const UP = svg('M6 10V2M2.5 5.5 6 2l3.5 3.5'), DOWN = svg('M6 2v8M2.5 6.5 6 10l3.5-3.5');
  const host = () => document.getElementById('statusbar');
  // на маленьком экране 1320 x 900 не влезет: берём сколько есть, за вычетом рамки и заголовка окна
  const target = () => ({ w: Math.min(REC.w, Math.max(400, screen.availWidth - 16)), h: Math.min(REC.h, Math.max(320, screen.availHeight - 48)) });
  const close = () => { const g = target(); return Math.abs(window.innerWidth - g.w) <= 24 && Math.abs(window.innerHeight - g.h) <= 24; };

  // data-p - очередь на выход: чем больше число, тем раньше пункт уходит, когда строка не помещается.
  // Пункты не сжимаются: лучше убрать целиком, чем показать обрезанным. 0 - не уходит никогда.
  const item = (p, inner, attrs = '', tag = 'span') => `<${tag} class="stb-i" data-p="${p}" ${attrs}>${inner}</${tag}>`;
  function render() {
    const el = host(); if (!el) return;
    if (window.Widget?.isOn()) { if (el._html) { el.innerHTML = ''; el._html = ''; } return; }
    const t = T(), p = P(), g = target();
    const left = [];
    if (p) {
      left.push(item(0, `<b>${esc(p.name)}</b>`, `data-act="projects" title="${esc(t.project)}&#10;${esc(p.path)}"`, 'button'));
      const gi = p.git;
      if (gi && !gi.untrusted) {
        const ab = gi.ahead || gi.behind ? `<span class="stb-ab" data-p="5">${UP}${esc(gi.ahead)} ${DOWN}${esc(gi.behind)}</span>` : '';
        left.push(item(1.2, `${Learn.icon('branch', 13)}<span class="tx">${esc(gi.branch)}</span>${gi.dirty ? `<span class="stb-n warn">${esc(gi.dirty)}</span>` : ''}${ab}`,
          `data-tab="git" title="${esc(t.branch)}${gi.dirty ? '&#10;' + esc(tpl(t.dirty, { n: gi.dirty })) : ''}"`, 'button'));
      }
      const n = [...S.runs.values()].filter((r) => r.status === 'running' && r.kind === 'script').length;
      if (n) left.push(item(1, `<span class="dot s-running"></span>${esc(tpl(t.running, { n }))}`, `data-tab="scripts" data-eta-runs="${esc(t.runningHint)}" title="${esc(etaRunsTip(t.runningHint))}"`, 'button'));
      const ci = ciLastStatus();
      if (ci) left.push(item(3, `<span class="dot s-${esc(ci)}"></span>${esc(tpl(t.ci, { status: RU[ci] || ci }))}`, `data-tab="ci" title="${esc(t.ciHint)}"`, 'button'));
      if (p.scanMs) left.push(item(6, esc(tpl(t.scan, { ms: p.scanMs })), `title="${esc(t.scanHint)}"`));
    }
    left.push(item(7, esc(K().fromDisk ? t.kbDisk : t.kbEmbedded), `title="${esc(t.kbHint)}"`));
    const near = close();
    const html = `<div class="stb"><div class="stb-g">${left.join('')}</div><span class="grow"></span><div class="stb-g">${item(4, `${window.innerWidth} × ${window.innerHeight}`, `title="${esc(t.size)}"`)}<button class="stb-i stb-rec" data-sbact="rec" ${near ? 'disabled' : ''} title="${esc(near ? t.recommendedDone : tpl(t.recommendedHint, { w: g.w, h: g.h }))}" aria-label="${esc(t.recommended)}">${Learn.icon('window', 13)}<span data-p="2">${esc(t.recommended)}</span></button></div></div>`;
    if (el._html !== html) { el.innerHTML = html; el._html = html; }
    fit(el);
  }
  // убрать по очереди то, что не помещается: сначала самое неважное, при равенстве - то, что правее
  function fit(el) {
    const bar = el.firstElementChild; if (!bar) return;
    const all = [...bar.querySelectorAll('[data-p]')];
    for (const x of all) x.hidden = false;
    const order = all.map((x, i) => [x, +x.dataset.p, i]).filter(([, k]) => k > 0).sort((a, b) => b[1] - a[1] || b[2] - a[2]);
    for (const [x] of order) {
      if (bar.scrollWidth <= bar.clientWidth + 1) break;
      x.hidden = true;
    }
  }

  document.addEventListener('click', async (e) => {
    const b = e.target.closest('[data-sbact]');
    if (!b || b.disabled) return;
    if (b.dataset.sbact === 'rec') {
      const g = target();
      try { await call('window_set', { width: g.w, height: g.h, center: true, onTop: false }); } catch (x) { toast(String(x), 'bad'); }
    }
  });
  let rT = 0;
  window.addEventListener('resize', () => { cancelAnimationFrame(rT); rT = requestAnimationFrame(render); });
  for (const ev of ['run', 'git', 'project']) listen(ev, () => setTimeout(render, 0));

  window.StatusBar = { render };
})();
