'use strict';
// Блоки страницы: реестр, раскладка, перестановка, свёртка, скрытие и ширина.
//
// Новый блок - одна запись Blocks.register({ id, page, render, ... }), а не новая разметка страницы:
// карточку с шапкой, ручкой, свёрткой и меню рисует этот файл. Порядок, свёрнутые, скрытые и ширина
// живут в localStorage (pult.blocks.<страница>) - в настройках ядра понадобилась бы правка Rust.
// Тексты - knowledge/ui.json, раздел blocks. Правила вида - .claude/rules/ui.md, раздел «Блоки».
//
// Движение: на новое место блок едет (FLIP, только transform, --t-med и --ease), свёртка - высотой
// строки сетки 1fr -> 0fr (blocks.css). «Уменьшить движение» - без анимации.
window.Blocks = (() => {
  const reg = new Map(); // id -> описание блока
  const st = {}; // страница -> { order, collapsed, hidden, span }
  const view = { menu: null, list: null, cols: {} }; // открытое меню блока ('стр:id'), меню «Блоки» (стр), колонок у сетки
  const seen = new WeakSet();
  let host = { texts: () => ({}), rerender() {}, announce() {}, dirty() {}, info: () => '', icon: () => '' };
  let drag = null, later = false, swallow = false;

  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const tpl = (s, v = {}) => String(s ?? '').replace(/\{(\w+)\}/g, (_, k) => (v[k] ?? ''));
  const T = () => host.texts() || {};
  const arr = (v) => (Array.isArray(v) ? v.filter((x) => typeof x === 'string') : []);
  const motion = () => document.documentElement.classList.contains('anim-ready') && !matchMedia('(prefers-reduced-motion: reduce)').matches;
  const css = (k, d) => getComputedStyle(document.documentElement).getPropertyValue(k).trim() || d;
  const SPANS = ['full', 'half', 'third'];

  // ---------- состояние ----------
  const KEY = (page) => 'pult.blocks.' + page;
  function load(page) {
    if (st[page]) return st[page];
    let v = null;
    try { v = JSON.parse(localStorage.getItem(KEY(page))); } catch { /* хранилище недоступно - раскладка по умолчанию */ }
    const span = {};
    for (const [k, x] of Object.entries((v && v.span) || {})) if (SPANS.includes(x)) span[k] = x;
    return (st[page] = { order: arr(v?.order), collapsed: arr(v?.collapsed), hidden: arr(v?.hidden), span });
  }
  function save(page) { try { localStorage.setItem(KEY(page), JSON.stringify(st[page])); } catch { /* приватный режим */ } }

  function register(def) {
    if (!def || !def.id || typeof def.render !== 'function') throw new Error('Blocks.register: нужны id и render');
    reg.set(def.id, { page: 'overview', span: 'full', ...def });
  }
  // все блоки страницы в порядке человека; блок, которого нет в сохранённом порядке (новый), - в конец
  function defs(page) {
    const s = load(page), pos = new Map(s.order.map((id, i) => [id, i]));
    return [...reg.values()].filter((d) => d.page === page && (!d.when || d.when()))
      .map((d, i) => [d, pos.has(d.id) ? pos.get(d.id) : 1e6 + i]).sort((a, b) => a[1] - b[1]).map((x) => x[0]);
  }
  const titleOf = (d) => (d.title ? d.title() : (T().titles || {})[d.id] || d.id);
  const spanOf = (page, d) => load(page).span[d.id] || d.span || 'full';
  const visible = (page) => { const s = load(page); return defs(page).filter((d) => !s.hidden.includes(d.id)); };

  // Сетка: 1, 2 или 6 дорожек по ширине самой сетки (1, 2 или 3 видимые колонки). Ширина блока - доля строки;
  // блок, который не влезает в остаток строки, начинает новую, а последний в неполной строке добирает
  // её до края: дыр в сетке не бывает при любом порядке.
  const colsFor = (w) => (w < 640 ? 1 : w < 1100 ? 2 : 6);
  const want = (span, cols) => (cols === 1 ? 1 : cols === 2 ? (span === 'full' ? 2 : 1) : { full: 6, half: 3, third: 2 }[span] || 6);
  function pack(items, cols) {
    const out = new Map();
    let used = 0, last = null;
    for (const it of items) {
      const w = want(it.span, cols);
      if (used && used + w > cols) { out.set(last, out.get(last) + cols - used); used = 0; }
      out.set(it.id, w); last = it.id; used = (used + w) % cols;
    }
    if (used && last) out.set(last, out.get(last) + cols - used);
    return out;
  }
  function guessCols() {
    const m = document.getElementById('main');
    return colsFor(m ? m.clientWidth - 40 : 1000);
  }
  const colsOf = (page) => view.cols[page] || (view.cols[page] = guessCols());

  // ---------- разметка ----------
  const ic = (name, size = 16, sw) => host.icon(name, size, sw);
  function grid(page) {
    const t = T(), vis = visible(page), cols = colsOf(page);
    // подсказка для чтения с экрана - внутри сетки (position:relative): вне её .sr вставал по высоте страницы и
    // растил прокрутку окна целиком
    const help = `<p class="sr" id="blk-help-${esc(page)}">${esc(t.help)}</p>`;
    if (!vis.length) return `<div class="blk-grid" data-blk-page="${esc(page)}" data-cols="${cols}">${help}<div class="empty-card blk-none">${esc(t.allHidden)}</div></div>`;
    const n = pack(vis.map((d) => ({ id: d.id, span: spanOf(page, d) })), cols);
    return `<div class="blk-grid" data-blk-page="${esc(page)}" data-cols="${cols}">${help}${vis.map((d, i) => card(page, d, n.get(d.id), i, vis.length)).join('')}</div>`;
  }
  function card(page, d, n, i, m) {
    const t = T(), s = load(page), title = titleOf(d), id = `blk-${esc(d.id)}`;
    const closed = s.collapsed.includes(d.id), menu = view.menu === page + ':' + d.id;
    const flush = typeof d.flush === 'function' ? d.flush() : !!d.flush;
    const aside = d.aside ? d.aside() : '';
    return `<section class="blk${closed ? ' closed' : ''}${menu ? ' menu-open' : ''}" data-blk="${esc(d.id)}" data-span="${n}" aria-labelledby="${id}-t">`
      + `<header class="blk-h" data-blk-drag="${esc(d.id)}"><h2 class="blk-t" id="${id}-t">${esc(title)}</h2>${d.info ? host.info(d.info) : ''}`
      + `<span class="blk-aside">${aside}</span><span class="blk-ctl">`
      + `<button type="button" class="blk-ic blk-grip" data-blk-grip="${esc(d.id)}" aria-label="${esc(tpl(t.grip, { title }))}" aria-describedby="blk-help-${esc(page)}" title="${esc(tpl(t.grip, { title }))}">${ic('grip', 16, 3)}</button>`
      + `<button type="button" class="blk-ic blk-fold" data-blk-fold="${esc(d.id)}" aria-expanded="${!closed}" aria-controls="${id}-b" aria-label="${esc(tpl(closed ? t.unfold : t.fold, { title }))}" title="${esc(tpl(closed ? t.unfold : t.fold, { title }))}">${ic('chevron', 16, 2)}</button>`
      + `<button type="button" class="blk-ic" data-blk-menu="${esc(d.id)}" aria-haspopup="menu" aria-expanded="${menu}" aria-label="${esc(tpl(t.menu, { title }))}" title="${esc(tpl(t.menu, { title }))}">${ic('more', 16, 3)}</button>`
      + `</span>${menu ? menuHtml(page, d, i, m, title) : ''}</header>`
      + `<div class="blk-b" id="${id}-b"${closed ? ' inert' : ''}><div class="blk-in"><div class="blk-pad${flush ? ' flush' : ''}">${d.render()}</div></div></div></section>`;
  }
  function menuHtml(page, d, i, m, title) {
    const t = T(), sp = spanOf(page, d);
    const item = (act, label, off) => `<button type="button" role="menuitem" class="blk-mi" data-blk-do="${act}" data-blk-id="${esc(d.id)}"${off ? ' aria-disabled="true"' : ''}>${esc(label)}</button>`;
    const radio = (v) => `<button type="button" role="menuitemradio" class="blk-mi" aria-checked="${sp === v}" data-blk-do="span:${v}" data-blk-id="${esc(d.id)}"><span class="blk-chk" aria-hidden="true"></span>${esc(t[v])}</button>`;
    return `<div class="blk-menu" role="menu" aria-label="${esc(tpl(t.menu, { title }))}">${item('up', t.up, i === 0)}${item('down', t.down, i === m - 1)}${item('first', t.first, i === 0)}${item('last', t.last, i === m - 1)}`
      + `<div class="blk-sep" role="separator"></div><div role="group" aria-label="${esc(t.width)}"><div class="blk-mh" aria-hidden="true">${esc(t.width)}</div>${SPANS.map(radio).join('')}</div>`
      + `<div class="blk-sep" role="separator"></div>${item('hide', t.hide)}</div>`;
  }
  // кнопка «Блоки»: какие показывать и «вернуть как было»; ставит её шапка страницы
  function listButton(page) {
    const t = T(), s = load(page), all = defs(page), hid = all.filter((d) => s.hidden.includes(d.id)).length, open = view.list === page;
    return `<span class="blk-list"><button type="button" class="btn ghost sm" data-blk-list="${esc(page)}" aria-haspopup="menu" aria-expanded="${open}">${ic('blocks', 14)}<span>${esc(t.list)}</span>${hid ? `<span class="blk-n">${esc(tpl(t.listHidden, { n: hid }))}</span>` : ''}</button>`
      + (open ? `<div class="blk-menu" role="menu" aria-label="${esc(t.listTitle)}"><div class="blk-mh" aria-hidden="true">${esc(t.listTitle)}</div>${all.map((d) => `<button type="button" role="menuitemcheckbox" class="blk-mi" aria-checked="${!s.hidden.includes(d.id)}" data-blk-do="toggle" data-blk-id="${esc(d.id)}" data-blk-of="${esc(page)}"><span class="blk-chk" aria-hidden="true"></span>${esc(titleOf(d))}</button>`).join('')}`
        + `<div class="blk-sep" role="separator"></div><button type="button" role="menuitem" class="blk-mi" data-blk-do="reset" data-blk-of="${esc(page)}">${esc(t.reset)}</button></div>` : '')
      + '</span>';
  }
  // такая же карточка без перестановки - для разделов других вкладок: всё окно читается одной системой
  function card0({ title, info = '', aside = '', body, flush = false, cls = '' }) {
    return `<section class="blk static ${cls}"><header class="blk-h"><h2 class="blk-t">${esc(title)}</h2>${info}<span class="blk-aside">${aside}</span></header>`
      + `<div class="blk-b"><div class="blk-in"><div class="blk-pad${flush ? ' flush' : ''}">${body}</div></div></div></section>`;
  }

  // ---------- движение ----------
  const gridEl = (page) => document.querySelector(`.blk-grid[data-blk-page="${CSS.escape(page)}"]`);
  // FLIP: где блоки были (с текущим transform - летящий блок продолжает лететь), правка, где встали; разница
  // уходит в transform и тает за --t-med. Новый блок проявляется. skip - блок, который тянут: его ведёт указатель.
  function flip(page, mutate, skip) {
    const g0 = gridEl(page);
    if (!g0 || !motion()) { mutate(); return; }
    const before = new Map();
    for (const b of g0.querySelectorAll(':scope>.blk')) if (b !== skip) before.set(b.dataset.blk, b.getBoundingClientRect());
    mutate();
    const g = gridEl(page);
    if (!g) return;
    const now = [...g.querySelectorAll(':scope>.blk')].filter((b) => b !== skip);
    for (const b of now) for (const a of b.getAnimations()) if (a.id === 'blk-flip') a.cancel();
    const rects = now.map((b) => b.getBoundingClientRect());
    const dur = parseFloat(css('--t-med', '200')) || 200, easing = css('--ease', 'ease-out');
    now.forEach((b, k) => {
      const r0 = before.get(b.dataset.blk), r1 = rects[k];
      let a = null;
      if (!r0) a = b.animate([{ opacity: 0, transform: 'translateY(6px)' }, { opacity: 1, transform: 'none' }], { duration: dur, easing });
      else {
        const dx = r0.left - r1.left, dy = r0.top - r1.top;
        if (Math.abs(dx) >= 0.5 || Math.abs(dy) >= 0.5) a = b.animate([{ transform: `translate(${dx}px,${dy}px)` }, { transform: 'none' }], { duration: dur, easing });
      }
      if (a) a.id = 'blk-flip';
    });
  }
  // ширины блоков по порядку в DOM - пока блок тянут, сетка пересчитывается без перерисовки страницы
  function repack(grid) {
    const page = grid.dataset.blkPage, blocks = [...grid.querySelectorAll(':scope>.blk')];
    const n = pack(blocks.map((b) => ({ id: b.dataset.blk, span: spanOf(page, reg.get(b.dataset.blk) || {}) })), colsOf(page));
    for (const b of blocks) { const v = String(n.get(b.dataset.blk)); if (b.dataset.span !== v) b.dataset.span = v; }
  }

  // ---------- действия ----------
  function rerender(page, fn, focusSel, say) {
    flip(page, () => { if (fn) fn(); host.rerender(); });
    if (focusSel) { const el = document.querySelector(focusSel); if (el) { el.focus({ preventScroll: true }); reveal(el.closest('.blk') || el); } }
    if (say) host.announce(say);
  }
  // блок, уехавший за край области прокрутки, показывается прокруткой самой страницы (#main), а не окна
  function reveal(el) {
    const m = el.closest('#main');
    if (!m) return;
    const r = el.getBoundingClientRect(), v = m.getBoundingClientRect();
    if (r.top < v.top) m.scrollTop -= v.top - r.top + 8;
    else if (r.bottom > v.bottom) m.scrollTop += Math.min(r.bottom - v.bottom + 8, r.top - v.top - 8);
  }
  // полный порядок (со скрытыми); id переезжает на место соседа среди видимых
  function moveTo(page, id, where) {
    const s = load(page), all = defs(page).map((d) => d.id), vis = visible(page).map((d) => d.id);
    const i = vis.indexOf(id);
    if (i < 0) return false;
    const j = where === 'up' ? i - 1 : where === 'down' ? i + 1 : where === 'first' ? 0 : vis.length - 1;
    if (j < 0 || j >= vis.length || j === i) return false;
    vis.splice(i, 1); vis.splice(j, 0, id);
    s.order = mergeOrder(all, vis, s.hidden);
    save(page);
    return true;
  }
  // скрытые блоки держат свои места в полном порядке, видимые встают по новому
  function mergeOrder(all, vis, hidden) {
    let k = 0;
    return all.map((id) => (hidden.includes(id) ? id : vis[k++] ?? id));
  }
  const say = (key, d, extra = {}) => tpl(T()[key], { title: titleOf(d), ...extra });
  function placeOf(page, id) { const v = visible(page).map((d) => d.id); return { n: v.indexOf(id) + 1, m: v.length }; }

  function doAct(page, act, id, trigger) {
    const d = reg.get(id), s = load(page);
    if (act === 'reset') {
      st[page] = { order: [], collapsed: [], hidden: [], span: {} }; save(page);
      view.list = null;
      return rerender(page, null, `[data-blk-list="${CSS.escape(page)}"]`, T().resetDone);
    }
    if (!d) return;
    if (act === 'toggle') {
      const on = s.hidden.includes(id);
      s.hidden = on ? s.hidden.filter((x) => x !== id) : [...s.hidden, id]; save(page);
      return rerender(page, null, `[data-blk-do="toggle"][data-blk-id="${CSS.escape(id)}"]`, say(on ? 'shown' : 'hidden', d));
    }
    if (act === 'hide') {
      // фокус - на соседний блок, а не на body: следующий Tab продолжает с того же места страницы
      const v = visible(page).map((x) => x.id), i = v.indexOf(id), next = v[i + 1] || v[i - 1];
      s.hidden = [...s.hidden, id]; save(page); view.menu = null;
      return rerender(page, null, next ? `[data-blk-menu="${CSS.escape(next)}"]` : `[data-blk-list="${CSS.escape(page)}"]`, say('hidden', d));
    }
    if (act.startsWith('span:')) {
      const v = act.slice(5);
      if (!SPANS.includes(v)) return;
      s.span = { ...s.span, [id]: v }; save(page); view.menu = null;
      return rerender(page, null, `[data-blk-menu="${CSS.escape(id)}"]`);
    }
    if (['up', 'down', 'first', 'last'].includes(act)) {
      if (!moveTo(page, id, act)) return;
      view.menu = null;
      const p = placeOf(page, id);
      return rerender(page, null, trigger === 'grip' ? `[data-blk-grip="${CSS.escape(id)}"]` : `[data-blk-menu="${CSS.escape(id)}"]`, say('moved', d, p));
    }
  }
  // свёртка - прямо в DOM: перерисовка заменила бы тело блока, и высота не доехала бы плавно
  function fold(page, id, btn) {
    const s = load(page), d = reg.get(id), sec = btn.closest('.blk');
    if (!d || !sec) return;
    const close = !s.collapsed.includes(id), t = T(), title = titleOf(d);
    s.collapsed = close ? [...s.collapsed, id] : s.collapsed.filter((x) => x !== id); save(page);
    sec.classList.toggle('closed', close);
    const body = sec.querySelector(':scope>.blk-b');
    if (body) body.inert = close;
    btn.setAttribute('aria-expanded', String(!close));
    const label = tpl(close ? t.unfold : t.fold, { title });
    btn.setAttribute('aria-label', label); btn.setAttribute('title', label);
    host.dirty(sec);
    host.announce(say(close ? 'folded' : 'unfolded', d));
  }
  function setMenu(menu, list, focusFirst) {
    const was = view.menu || view.list;
    view.menu = menu; view.list = list;
    host.rerender();
    if (focusFirst) document.querySelector('.blk-menu .blk-mi:not([aria-disabled])')?.focus({ preventScroll: true });
    return was;
  }
  function close(back) {
    if (!view.menu && !view.list) return false;
    const sel = view.menu ? `[data-blk-menu="${CSS.escape(view.menu.split(':')[1])}"]` : `[data-blk-list="${CSS.escape(view.list)}"]`;
    view.menu = null; view.list = null;
    host.rerender();
    if (back) document.querySelector(sel)?.focus({ preventScroll: true });
    return true;
  }

  // ---------- перетаскивание ----------
  // Тянуть можно за шапку блока или за ручку. Цель - блок, в чьём месте раскладки сейчас указатель:
  // место берётся из offset* (без transform), поэтому блоки, что ещё едут на новые места, не путают выбор.
  function onDown(e) {
    if (e.button !== 0 || !e.isPrimary || drag) return;
    const h = e.target.closest('[data-blk-drag]');
    if (!h || e.target.closest('button:not([data-blk-grip]), a, input, select, .blk-menu, .info')) return;
    const sec = h.closest('.blk'), grid = sec?.parentElement;
    if (!grid?.dataset.blkPage) return;
    drag = { sec, grid, page: grid.dataset.blkPage, id: sec.dataset.blk, x0: e.clientX, y0: e.clientY, x: e.clientX, y: e.clientY, pid: e.pointerId, on: false, last: null, grip: !!e.target.closest('[data-blk-grip]') };
    try { h.setPointerCapture(e.pointerId); } catch { /* указатель уже отпущен */ }
  }
  function begin() {
    const r = drag.sec.getBoundingClientRect();
    drag.offX = drag.x0 - r.left; drag.offY = drag.y0 - r.top;
    drag.orig = [...drag.grid.querySelectorAll(':scope>.blk')].map((b) => b.dataset.blk);
    drag.on = true;
    if (view.menu || view.list) { view.menu = null; view.list = null; }
    drag.sec.classList.add('drag');
    document.documentElement.classList.add('blk-dragging');
    const body = drag.sec.querySelector(':scope>.blk-b'); if (body) body.inert = true;
    window.getSelection?.().removeAllRanges();
    drag.raf = requestAnimationFrame(tick);
  }
  function inside(b, px, py) { return px >= b.offsetLeft && px < b.offsetLeft + b.offsetWidth && py >= b.offsetTop && py < b.offsetTop + b.offsetHeight; }
  function step() {
    const { grid, sec } = drag;
    let g = grid.getBoundingClientRect();
    const px = drag.x - g.left, py = drag.y - g.top;
    if (drag.last && !inside(drag.last, px, py)) drag.last = null;
    let target = null;
    for (const b of grid.querySelectorAll(':scope>.blk')) if (b !== sec && b !== drag.last && inside(b, px, py)) { target = b; break; }
    if (target) {
      const after = !!(sec.compareDocumentPosition(target) & Node.DOCUMENT_POSITION_FOLLOWING);
      flip(drag.page, () => { if (after) target.after(sec); else target.before(sec); repack(grid); }, sec);
      drag.last = target;
      g = grid.getBoundingClientRect();
    }
    // блок не уезжает за края сетки: иначе его transform растил прокрутку страницы вбок и вниз
    const cl = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
    const tx = cl(drag.x - drag.offX - g.left - sec.offsetLeft, -sec.offsetLeft, grid.clientWidth - sec.offsetLeft - sec.offsetWidth);
    const ty = cl(drag.y - drag.offY - g.top - sec.offsetTop, -sec.offsetTop, grid.clientHeight - sec.offsetTop - sec.offsetHeight);
    sec.style.transform = `translate(${Math.round(tx)}px,${Math.round(ty)}px)`;
  }
  // у края области прокрутки страница едет сама: длинный «Обзор» не нужно листать колесом во время переноса
  function tick() {
    if (!drag?.on) return;
    const m = drag.grid.closest('#main');
    if (m) {
      const r = m.getBoundingClientRect(), edge = 48;
      const v = drag.y < r.top + edge ? -Math.min(16, Math.ceil((r.top + edge - drag.y) / 3)) : drag.y > r.bottom - edge ? Math.min(16, Math.ceil((drag.y - r.bottom + edge) / 3)) : 0;
      if (v && ((v < 0 && m.scrollTop > 0) || (v > 0 && m.scrollTop + m.clientHeight < m.scrollHeight))) { m.scrollTop += v; step(); }
    }
    drag.raf = requestAnimationFrame(tick);
  }
  function onMove(e) {
    if (!drag || e.pointerId !== drag.pid) return;
    drag.x = e.clientX; drag.y = e.clientY;
    if (!drag.on) { if (Math.hypot(drag.x - drag.x0, drag.y - drag.y0) < 5) return; begin(); }
    e.preventDefault();
    step();
  }
  function finish(cancel) {
    const d = drag; drag = null;
    if (!d) return;
    cancelAnimationFrame(d.raf);
    if (!d.on) return;
    const { sec, grid, page, id } = d;
    if (cancel) flip(page, () => { for (const k of d.orig) { const b = grid.querySelector(`:scope>.blk[data-blk="${CSS.escape(k)}"]`); if (b) grid.append(b); } repack(grid); }, sec);
    const from = sec.getBoundingClientRect();
    sec.style.transform = '';
    sec.classList.remove('drag');
    document.documentElement.classList.remove('blk-dragging');
    swallow = true; setTimeout(() => { swallow = false; }, 0);
    const body = sec.querySelector(':scope>.blk-b'); if (body) body.inert = load(page).collapsed.includes(id);
    const to = sec.getBoundingClientRect();
    if (motion() && (Math.abs(from.left - to.left) > 0.5 || Math.abs(from.top - to.top) > 0.5)) {
      sec.animate([{ transform: `translate(${from.left - to.left}px,${from.top - to.top}px)` }, { transform: 'none' }], { duration: parseFloat(css('--t-med', '200')) || 200, easing: css('--ease', 'ease-out') }).id = 'blk-flip';
    }
    const s = load(page), dom = [...grid.querySelectorAll(':scope>.blk')].map((b) => b.dataset.blk);
    const moved = dom.join() !== d.orig.join();
    if (moved) { s.order = mergeOrder(defs(page).map((x) => x.id), dom, s.hidden); save(page); }
    host.dirty(grid);
    if (d.grip) sec.querySelector('[data-blk-grip]')?.focus({ preventScroll: true });
    if (moved) host.announce(say('moved', reg.get(id), placeOf(page, id)));
    if (later) { later = false; host.rerender(); }
  }

  // ---------- события ----------
  document.addEventListener('pointerdown', onDown);
  window.addEventListener('pointermove', onMove, { passive: false });
  window.addEventListener('pointerup', (e) => { if (drag && e.pointerId === drag.pid) finish(false); });
  window.addEventListener('pointercancel', (e) => { if (drag && e.pointerId === drag.pid) finish(true); });
  // щелчок, которым кончился перенос, не нажимает кнопку под указателем
  document.addEventListener('click', (e) => { if (swallow) { e.preventDefault(); e.stopImmediatePropagation(); } }, true);
  document.addEventListener('click', (e) => {
    const b = e.target.closest('[data-blk-fold],[data-blk-menu],[data-blk-list],[data-blk-do]');
    if (!b) { if ((view.menu || view.list) && !e.target.closest('.blk-menu')) close(false); return; }
    const page = b.dataset.blkOf || b.closest('[data-blk-page]')?.dataset.blkPage || 'overview';
    if (b.dataset.blkFold) return fold(page, b.dataset.blkFold, b);
    if (b.dataset.blkMenu) { const k = page + ':' + b.dataset.blkMenu; return view.menu === k ? close(true) : setMenu(k, null, e.detail === 0); }
    if (b.dataset.blkList) return view.list === b.dataset.blkList ? close(true) : setMenu(null, b.dataset.blkList, e.detail === 0);
    if (b.getAttribute('aria-disabled') === 'true') return;
    doAct(page, b.dataset.blkDo, b.dataset.blkId, 'menu');
  });
  const MOVE_KEYS = { ArrowUp: 'up', ArrowLeft: 'up', ArrowDown: 'down', ArrowRight: 'down', Home: 'first', End: 'last' };
  document.addEventListener('keydown', (e) => {
    if (drag?.on && e.key === 'Escape') { e.preventDefault(); e.stopImmediatePropagation(); finish(true); return; }
    const t = e.target;
    const grip = t.closest?.('[data-blk-grip]');
    if (grip && MOVE_KEYS[e.key] && !e.ctrlKey && !e.altKey && !e.metaKey) {
      e.preventDefault(); e.stopImmediatePropagation();
      const page = grip.closest('[data-blk-page]')?.dataset.blkPage;
      if (page) doAct(page, MOVE_KEYS[e.key], grip.dataset.blkGrip, 'grip');
      return;
    }
    if (!view.menu && !view.list) return;
    if (e.key === 'ArrowDown' && t.closest?.('[data-blk-menu],[data-blk-list]')?.getAttribute('aria-expanded') === 'true') { e.preventDefault(); e.stopImmediatePropagation(); document.querySelector('.blk-menu .blk-mi:not([aria-disabled])')?.focus(); return; }
    const menu = t.closest?.('.blk-menu');
    if (e.key === 'Escape') { e.preventDefault(); e.stopImmediatePropagation(); close(true); return; }
    if (!menu) return;
    const items = [...menu.querySelectorAll('.blk-mi')], i = items.indexOf(t);
    const go = (k) => { e.preventDefault(); e.stopImmediatePropagation(); items[(k + items.length) % items.length]?.focus(); };
    if (e.key === 'ArrowDown') go(i + 1);
    else if (e.key === 'ArrowUp') go(i - 1);
    else if (e.key === 'Home') go(0);
    else if (e.key === 'End') go(items.length - 1);
    else if (e.key === 'Tab') close(false);
  });
  // ширина сетки меняется (окно, консоль, панель подробностей) - число колонок пересчитывается после раскладки
  const ro = typeof ResizeObserver === 'function' ? new ResizeObserver((ents) => {
    let changed = false;
    for (const en of ents) {
      const page = en.target.dataset.blkPage, c = colsFor(en.contentRect.width);
      if (page && en.contentRect.width > 0 && view.cols[page] !== c) { view.cols[page] = c; changed = true; }
    }
    if (!changed) return;
    if (drag?.on) later = true; else host.rerender();
  }) : null;
  function mount(root) {
    for (const g of (root || document).querySelectorAll('.blk-grid[data-blk-page]')) if (!seen.has(g)) { seen.add(g); ro?.observe(g); }
  }

  return {
    register, grid, card: card0, listButton, mount, close,
    init(h) { host = { ...host, ...h }; },
    busy: () => !!drag?.on,
    defer() { later = true; },
    defs, // для проверки и справки: список блоков страницы в порядке человека
  };
})();
