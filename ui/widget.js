'use strict';
// Виджет: маленькое окно целиком становится карточкой с самым нужным - что идёт, быстрый запуск,
// сценарии, CI, git - и строкой состояния внизу. Только имена, статусы, числа и кнопки: пояснений нет.
// Окно само подбирает высоту под содержимое (от MIN.h до maxH(), дальше прокрутка).
// Блоки и их порядок - settings.widget, тексты - knowledge/ui.json (раздел widget).
// Подключается после app.js и берёт его общие помощники: S, call, esc, tpl, UI, P, patch и другие.
(() => {
  // значки своими SVG: шрифтовые ↑ ↓ ✕ Windows рисует цветными эмодзи
  const svg = (d, n = 12) => `<svg width="${n}" height="${n}" viewBox="0 0 12 12" aria-hidden="true"><path d="${d}" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
  const UP = svg('M6 10V2M2.5 5.5 6 2l3.5 3.5'), DOWN = svg('M6 2v8M2.5 6.5 6 10l3.5-3.5');
  const CROSS = svg('M3 3l6 6M9 3l-6 6', 13);
  const PIN = svg('M4.5 1.5h3M5 1.5v3L3 6.5h6L7 4.5v-3M6 6.5V10.5', 13);
  const T = () => UI().widget || {};
  const BLOCKS = ['runs', 'quick', 'scenarios', 'ci', 'git'];
  // Полное окно становится виджетом, когда его сжали ниже 360 x 300. Обратно - только кнопкой
  // «Вернуть прежний размер»: у виджета края не тянутся (setResizable(false)), размер задаёт он сам.
  const IN = { w: 360, h: 300 };
  // ширина виджета, наименьший размер, шаг (меньшую разницу высоты не правим), полный размер
  const W0 = 340, MIN = { w: 300, h: 220 }, STEP = 8, FULL = { w: 1320, h: 900 };
  const maxH = () => Math.max(MIN.h, Math.min(640, Math.round((screen.availHeight || 800) * 0.8)));

  let on = false, started = false;
  // ours - размер, который окну задали мы сами: его событие resize не считается действием человека
  let ours = null, oursT = 0;
  // curW - ширина виджета: 340, или меньше, если в виджет вошли, сузив окно
  let curW = W0;
  let fitT = 0, settleT = 0, saveT = 0, ro = null, seen = [];
  // ожидание, пока отпустят край окна: идёт ли опрос и когда размер менялся последний раз
  let waiting = false, lastSize = 0;
  // topSent - «поверх окон», которое уже отдали окну: после перезапуска в виджете размер уже верный,
  // и без этой отметки fit() не звал window_set - закреплённый виджет открывался незакреплённым
  let topSent = null;
  const host = () => document.getElementById('widget');
  const win = () => window.__TAURI__?.window?.getCurrentWindow?.();
  const near = (a, b, d = 2) => Math.abs(a - b) <= d;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const blocks = () => (Array.isArray(S.settings.widget) ? S.settings.widget : BLOCKS.slice(0, 4)).filter((b) => BLOCKS.includes(b));
  // тянуть края и разворачивать двойным щелчком по строке: у виджета нельзя, у полного окна можно
  function resizable(v) {
    const w = win();
    if (!w) return Promise.resolve();
    return Promise.all([w.setResizable?.(v), w.setMaximizable?.(v)]).catch((e) => console.warn('resizable', e));
  }

  // ---------- размер окна ----------
  // Задать размер окна. mode - каким станет вид, когда окно дойдёт до этого размера.
  function setSize(w, h, mode, { center = false, onTop = null } = {}) {
    w = Math.round(w); h = Math.round(h);
    const same = near(w, window.innerWidth) && near(h, window.innerHeight);
    clearTimeout(oursT);
    ours = same ? null : { w, h, mode };
    // окно не дошло до размера (экран мал, система поправила) - вид всё равно переключается
    if (ours) oursT = setTimeout(() => { const o = ours; ours = null; if (o && o.mode !== on) setMode(o.mode); }, 1500);
    else if (mode !== on) setMode(mode);
    if (onTop !== null) topSent = onTop;
    return call('window_set', { width: w, height: h, center, onTop }).catch((e) => toast(String(e), 'bad'));
  }

  // Высота, которая нужна виджету при ширине w: замер на месте, с временной шириной и большой
  // высотой (без полосы прокрутки); стиль возвращается до отрисовки кадра.
  function natural(w) {
    const el = host(), wg = el?.firstElementChild;
    if (!wg) return MIN.h;
    const keep = el.style.cssText;
    el.style.cssText = `width:${w}px;height:4000px;right:auto;bottom:auto`;
    fitBar();
    const n = (x, k) => parseFloat(getComputedStyle(x)[k]) || 0;
    const vpad = (x) => n(x, 'paddingTop') + n(x, 'paddingBottom') + n(x, 'borderTopWidth') + n(x, 'borderBottomWidth');
    const body = $('.wg-body', el), list = $('.wg-list', el), foot = $('.wg-foot', el), top = $('.wg-top', el);
    const h = vpad(el) + vpad(wg) + (top?.offsetHeight || 0) + (foot?.offsetHeight || 0) + (body ? vpad(body) : 0) + (list?.offsetHeight || 0);
    el.style.cssText = keep;
    fitBar();
    return Math.ceil(h);
  }

  // Подогнать окно под содержимое: ширина curW, высота от MIN.h до maxH(), дальше прокрутка.
  function fit() {
    clearTimeout(fitT);
    if (!on) return;
    if (ours) { fitT = setTimeout(fit, 300); return; }
    const w = Math.max(MIN.w, curW);
    const h = clamp(natural(w), MIN.h, maxH());
    const top = !!S.settings.widgetOnTop;
    if (near(w, window.innerWidth) && Math.abs(h - window.innerHeight) <= STEP && topSent === top) return;
    setSize(w, h, true, { onTop: top });
  }
  const refit = (ms = 150) => { if (!on) return; clearTimeout(fitT); fitT = setTimeout(fit, ms); };
  // вошли, сузив окно: подогнать, когда край отпустят
  function settle() { clearTimeout(settleT); settleT = setTimeout(fit, 450); }

  // ---------- вход и выход ----------
  function setMode(v) {
    if (v === on) return;
    on = v;
    LS.set('widget.on', on);
    document.documentElement.classList.toggle('widget-mode', on);
    // фон самого окна - цвет карточки: виджет растёт под содержимое, и новая полоса не мелькает другим цветом
    window.syncWindowBg?.();
    const el = host();
    el.hidden = !on;
    if (on) { resizable(false); S.pop = null; $('#tip').hidden = true; window.TitleBar?.hide(); draw(); }
    else { ro?.disconnect(); seen = []; el.innerHTML = ''; el._html = ''; window.TitleBar?.show(); render(); }
  }
  function enter(button) {
    curW = button || window.innerWidth >= IN.w ? W0 : Math.max(MIN.w, window.innerWidth);
    setMode(true);
    if (button) fit(); else settle();
  }

  function onResize() {
    const w = window.innerWidth, h = window.innerHeight;
    if (ours) {
      // наш размер дошёл - переключить вид, если надо; промежуточные размеры пропускаем
      if (near(w, ours.w) && near(h, ours.h)) { const m = ours.mode; ours = null; clearTimeout(oursT); if (m !== on) setMode(m); }
      if (on) fitBar();
      return;
    }
    // Сжали ниже порога. Виджет включается, только когда кнопку мыши ОТПУСТИЛИ (ядро спрашивает у
    // Windows) и размер постоял 250 мс: иначе он отключает растягивание и задаёт размер прямо под
    // рукой, и окно дёргается. Пока тянут - вместо сжатого окна спокойная заглушка «Отпустите».
    if (!on) {
      if (w < IN.w || h < IN.h) { lastSize = Date.now(); pending(true); waitRelease(); }
      else { pending(false); remember(); }
      return;
    }
    // край виджета не тянется; сюда попадает смена масштаба экрана - подогнать заново
    fitBar();
    settle();
  }

  // Ждать, пока отпустят кнопку мыши и размер постоит; тогда - виджет (или ничего, если окно уже
  // растянули обратно). Один опрос за раз: повторный вызов только сдвигает «последнее изменение».
  function waitRelease() {
    if (waiting) return;
    waiting = true;
    const tick = async () => {
      const small = window.innerWidth < IN.w || window.innerHeight < IN.h;
      if (on || !small) { waiting = false; pending(false); return; }
      const down = await call('mouse_down').catch(() => false);
      if (down || Date.now() - lastSize < 250) { setTimeout(tick, 120); return; }
      waiting = false;
      pending(false);
      enter(false);
    };
    setTimeout(tick, 120);
  }
  // Заглушка на время перетаскивания: сжатое полное окно не показывается вовсе
  function pending(v) {
    document.documentElement.classList.toggle('widget-pending', v);
    let el = document.getElementById('wg-pending');
    if (!v) { if (el) el.hidden = true; return; }
    if (!el) {
      el = document.createElement('div');
      el.id = 'wg-pending';
      document.body.appendChild(el);
    }
    const t = T();
    el.innerHTML = `<img src="img/logo.png" alt="" width="40" height="40"><b>${esc(t.pendingTitle || 'Отпустите')}</b><span>${esc(t.pendingText || '')}</span>`;
    el.hidden = false;
  }

  // Зовётся из renderTop (app.js). При первом вызове: Пульт закрыли виджетом или окно открылось
  // маленьким - снова виджет; иначе окну возвращается право тянуть края (вдруг его закрыли виджетом
  // со старой версией).
  function sync() {
    if (!started) {
      started = true;
      if (LS.get('widget.on', false) || window.innerWidth < IN.w || window.innerHeight < IN.h) enter(false);
      else resizable(true);
    }
    // дальше вход в виджет - только из onResize, после того как край отпустили
    if (on) draw(); else remember();
  }

  // Прежний размер запоминается, когда окно в полном виде и его перестали тянуть: при сжатии одним
  // движением промежуточные 400 x 330 не успевают записаться, и «Вернуть прежний размер» вернёт настоящий.
  // Развёрнутое окно свой размер (весь экран) не записывает: остаётся обычный размер и отметка max,
  // и «Вернуть прежний размер» ставит обычный размер и разворачивает окно снова.
  const isMax = () => win()?.isMaximized?.().catch(() => false) ?? Promise.resolve(false);
  async function savePrev(force) {
    const max = await isMax();
    if (!force && (on || ours)) return;
    if (max) LS.set('widget.prev', { ...(LS.get('widget.prev', null) || {}), max: true });
    else LS.set('widget.prev', { w: window.innerWidth, h: window.innerHeight });
  }
  function remember() {
    clearTimeout(saveT);
    saveT = setTimeout(() => { if (!on && !ours) savePrev(); }, 700);
  }

  async function collapse() {
    if (on) return;
    clearTimeout(saveT);
    await savePrev(true);
    S.pop = null;
    enter(true);
  }
  // «Вернуть прежний размер»: прежний размер полного окна, иначе рекомендованный 1320 x 900 по центру.
  // Прежний размер ставится на месте, если окно оттуда помещается на экран, иначе тоже по центру.
  // Право тянуть края возвращается ДО смены размера.
  async function expand(then) {
    const p = LS.get('widget.prev', null);
    const saved = p && p.w >= IN.w && p.h >= IN.h;
    const sw = screen.availWidth || FULL.w, sh = screen.availHeight || FULL.h;
    const w = Math.min(saved ? p.w : FULL.w, sw), h = Math.min(saved ? p.h : FULL.h, sh);
    const left = screen.availLeft ?? 0, top = screen.availTop ?? 0;
    const fits = saved && window.screenX + w <= left + sw && window.screenY + h <= top + sh;
    clearTimeout(fitT); clearTimeout(settleT);
    await resizable(true);
    await setSize(w, h, false, { center: !fits, onTop: false });
    // в виджет ушли из развёрнутого окна - оно и вернётся развёрнутым (размер выше - его обычный размер)
    if (p?.max) await win()?.toggleMaximize?.().catch(() => {});
    // прежний размер мог быть низким (окно сжимали до виджета): боковая панель обязана поместиться
    // целиком - если её кнопки не влезли, окно растёт ровно на нехватку, но не выше экрана
    setTimeout(() => {
      const side = document.getElementById('side');
      if (on || p?.max || !side || window.innerWidth <= 760) return;
      const lack = side.scrollHeight - side.clientHeight;
      if (lack > 1) {
        const nh = Math.min(window.innerHeight + lack + 8, sh);
        setSize(window.innerWidth, nh, false, { center: window.screenY + nh > top + sh });
      }
    }, 450);
    if (then) setTimeout(then, 150);
  }

  // ---------- блоки ----------
  const sec = (id, body) => `<section class="wg-sec wg-b-${id}"><h5>${esc((T().blocks || {})[id] || id)}</h5>${body}</section>`;
  const none = (text) => `<div class="wg-none">${esc(text)}</div>`;
  const scriptRuns = () => [...S.runs.values()].filter((r) => r.status === 'running' && r.kind === 'script').sort((a, b) => a.startedAt - b.startedAt);
  function bRuns() {
    const p = P(), t = T(), runs = scriptRuns();
    let body = runs.length ? runs.map((r) => {
      const eta = runEta(r);
      return `<div class="wg-row wg-run"><span class="dot s-running"></span><span class="t mono">${esc(r.title)}</span>
        <button class="wg-act stop" data-stop="${esc(r.id)}" title="${esc(t.stop)}" aria-label="${esc(t.stop)} ${esc(r.title)}">■</button>
        <span class="m"><span class="el" data-el="${esc(r.id)}">${esc(dur(Date.now() - r.startedAt))}</span>${etaLabel(r.startedAt, eta, 'wg-eta')}</span>${etaBar(r.startedAt, eta, 'under')}</div>`;
    }).join('') : '';
    if (!runs.length) return '';
    const last = p ? recentRuns(12).find((r) => r.status !== 'running') : null;
    if (last) body += `<button class="wg-last" data-wact="log" data-id="${esc(last.id)}" title="${esc(t.lastHint)}">${sdot(last.status)}<span class="t"><span class="lb">${esc(t.last)}</span> <span class="mono">${esc(last.title)}</span></span><span class="d">${esc(dur((last.endedAt || last.startedAt) - last.startedAt))}</span></button>`;
    return sec('runs', body);
  }
  function bQuick() {
    const t = T(), list = quickList().slice(0, 4);
    if (!list.length) return '';
    return sec('quick', `<div class="wg-quick">${list.map((s) => {
      const r = lastOf(s.id), run = r?.status === 'running', miss = isMissing(s);
      return `<button class="wg-q ${run ? 'on' : ''}" data-run="${esc(s.id)}" ${miss ? 'disabled' : ''} title="${esc(s.cmd)}"><span class="ic">${run ? '■' : '▶'}</span><span class="t">${isFav(s.id) ? '★ ' : ''}${esc(s.name)}</span>${r && !run ? sdot(r.status) : ''}</button>`;
    }).join('')}</div>`);
  }
  function bScenarios() {
    const t = T(), list = window.Scen?.forProject() || [];
    if (!list.length) return '';
    const tr = UI().scenarios || {};
    return sec('scenarios', list.map((s) => {
      const st = window.Scen.state(s.id), run = st?.status === 'running';
      const n = st?.total || s.steps.length;
      const prog = run ? `${st.step >= 0 ? st.step + 1 : 0}/${n}` : st ? (tr.status || {})[st.status] || '' : window.Scen.stepWord(s.steps.length);
      return `<div class="wg-row wg-sc ${run ? 'on' : ''}">${st ? sdot(st.status) : '<span class="dot d-off" aria-hidden="true"></span>'}<span class="t">${esc(s.name)}</span>
        ${run ? `<button class="wg-act stop" data-scstop="${esc(s.id)}" title="${esc(tr.stop)}" aria-label="${esc(tr.stop)} ${esc(s.name)}">■</button>`
          : `<button class="wg-act go" data-scrun="${esc(s.id)}" title="${esc(tr.run)}" aria-label="${esc(tr.run)} ${esc(s.name)}" ${s.steps.length ? '' : 'disabled'}>▶</button>`}
        <span class="m">${esc(prog)}</span></div>`;
    }).join(''));
  }
  function bCI() {
    const t = T(), rem = remotesForCI()[0];
    if (!rem) return '';
    const st = ciLastStatus(), loading = Object.values(S.ci.data).some((x) => x.loading);
    const run = (S.ci.data[`${rem.name}|`] || {}).runs?.[0];
    return sec('ci', `<div class="wg-row"><span class="dot ${st ? 's-' + esc(st) : 'd-off'}"></span><span class="t">${st ? esc(RU[st] || st) : esc(t.ciNone)}</span>
      ${rem.trusted ? `<button class="wg-act go" data-wact="ci" title="${esc(t.ciLoad)}" aria-label="${esc(t.ciLoad)}" ${loading ? 'disabled' : ''}>${loading ? '<span class="spin"></span>' : '↻'}</button>` : ''}
      ${run ? `<span class="m">#${esc(run.id)} · ${esc(ago(run.createdAt))}</span>` : ''}</div>`);
  }
  function bGit() {
    const t = T(), gi = P()?.git;
    if (!gi || gi.untrusted) return '';
    return sec('git', `<div class="wg-row wg-git"><span class="ic">${Learn.icon('branch', 14)}</span><span class="t mono">${esc(gi.branch)}</span>
      <span class="m"><span class="${gi.dirty ? 'warn' : 'ok'}">${gi.dirty ? esc(tpl(t.dirty, { n: gi.dirty })) : esc(t.clean)}</span> · <span class="ab" title="${esc(t.aheadBehind)}">${UP}${esc(gi.ahead)} ${DOWN}${esc(gi.behind)}</span></span></div>`);
  }
  const BUILD = { runs: bRuns, quick: bQuick, scenarios: bScenarios, ci: bCI, git: bGit };

  // ---------- полосы ----------
  // Сверху - своя полоса окна без рамки: значок и имя (за них окно перетаскивается), справа три кнопки:
  // поверх окон, вернуть прежний размер, закрыть. Атрибут перетаскивания стоит только на фоне и подписи,
  // не на кнопках и не на блоках: текст в блоках выделяется и копируется.
  function topHtml() {
    const t = T(), pinned = !!S.settings.widgetOnTop;
    const closeT = S.settings.closeToTray ? t.toTray : t.close;
    const pinT = pinned ? t.unpin : t.pin;
    return `<div class="wg-top" data-tauri-drag-region><img class="wg-logo" src="img/logo.png" alt="" data-tauri-drag-region><span class="wg-title" data-tauri-drag-region>Пульт</span>
      <button class="wg-tb ${pinned ? 'on' : ''}" data-wact="pin" title="${esc(pinT)}" aria-label="${esc(pinT)}" aria-pressed="${pinned}">${PIN}</button>
      <button class="wg-tb" data-wact="expand" title="${esc(t.expandHint || t.expand)}" aria-label="${esc(t.expand)}">${Learn.icon('window', 13)}</button>
      <button class="wg-tb x" data-wact="close" title="${esc(closeT)}" aria-label="${esc(closeT)}">${CROSS}</button></div>`;
  }
  // Снизу - строка состояния: репозиторий и ветка, в одну строку. Не влезает ветка - прячется она;
  // имя проекта остаётся всегда (полное - в подсказке).
  function barHtml() {
    const p = P(), t = T(), gi = p?.git;
    const name = p ? p.name : 'Пульт';
    const git = gi && !gi.untrusted ? `<span class="stb-i wf-git" data-p="1">${Learn.icon('branch', 13)}<span class="mono">${esc(gi.branch)}</span>${gi.dirty ? `<span class="stb-n warn" title="${esc(tpl(t.dirty, { n: gi.dirty }))}">${esc(gi.dirty)}</span>` : ''}</span>` : '';
    return `<div class="stb wg-stb"><span class="stb-i wf-proj" title="${esc(p?.path || name)}"><b>${esc(name)}</b></span>${git}</div>`;
  }
  function fitBar() {
    const bar = $('.wg-stb', host());
    if (!bar) return;
    const g = $('.wf-git', bar);
    if (!g) return;
    g.hidden = false;
    if (bar.scrollWidth > bar.clientWidth + 1) g.hidden = true;
  }

  // ---------- отрисовка ----------
  function draw() {
    const el = host(); if (!el) return;
    const t = T(), p = P(), list = blocks();
    const parts = p ? list.map((b) => BUILD[b]()).filter(Boolean) : [];
    const body = !p ? none(t.noProject) : parts.length ? parts.join('') : none(t.empty);
    const html = `<div class="wg" role="region" aria-label="${esc(t.settingsTitle)}">
      ${topHtml()}
      <div class="wg-body"><div class="wg-list">${body}</div></div>
      <footer class="wg-foot" aria-label="${esc((t.bar || {}).label)}">${barHtml()}</footer></div>`;
    if (!patch(el, html)) return;
    fitBar();
    watch();
    refit();
  }
  // Высота списка или строки поменялась (перенос при другой ширине, новый запуск) - подогнать окно
  function watch() {
    const el = host(), nodes = [$('.wg-list', el), $('.wg-foot', el)].filter(Boolean);
    if (nodes.length === seen.length && nodes.every((n, i) => n === seen[i])) return;
    ro = ro || new ResizeObserver(() => { if (on) { fitBar(); refit(); } });
    ro.disconnect();
    for (const n of nodes) ro.observe(n);
    seen = nodes;
  }
  function redraw() { if (on) draw(); }

  // ---------- островок и настройки ----------
  function sideItem() {
    const t = T();
    return `<button class="sb-item" data-wact="collapse" title="${esc(t.collapseHint)}">${Learn.icon('window', 18)}<span class="lbl">${esc(t.collapse)}</span></button>`;
  }
  function settingsHtml() {
    const t = T(), sc = UI().scenarios || {}, s = S.settings;
    const onList = blocks(), off = BLOCKS.filter((b) => !onList.includes(b));
    const row = (b, i, enabled) => `<div class="wset ${enabled ? '' : 'off'}"><label class="set"><input type="checkbox" data-wblock="${b}" ${enabled ? 'checked' : ''}><span>${esc((t.blocks || {})[b] || b)}</span></label>
      <span class="wset-mv"><button class="btn ghost icon sm" data-wmove="${b}" data-dir="-1" title="${esc(t.up)}" aria-label="${esc(t.up)}" ${enabled && i > 0 ? '' : 'disabled'}>${UP}</button><button class="btn ghost icon sm" data-wmove="${b}" data-dir="1" title="${esc(t.down)}" aria-label="${esc(t.down)}" ${enabled && i < onList.length - 1 ? '' : 'disabled'}>${DOWN}</button></span></div>`;
    return `<h4 style="margin-top:12px">${esc(sc.trayTitle)}</h4>
      <label class="set"><input type="checkbox" data-set="closeToTray" ${s.closeToTray ? 'checked' : ''}><span>${esc(sc.closeToTray)}<small>${esc(sc.closeToTrayText)}</small></span></label>
      <h4 style="margin-top:12px">${esc(t.settingsTitle)}</h4>
      <p class="note" style="margin:0 0 4px">${esc(t.settingsText)}</p>
      <div class="wset-list">${onList.map((b, i) => row(b, i, true)).join('')}${off.map((b) => row(b, -1, false)).join('')}</div>
      <label class="set"><input type="checkbox" data-set="widgetOnTop" ${s.widgetOnTop ? 'checked' : ''}><span>${esc(t.onTop)}<small>${esc(t.onTopText)}</small></span></label>`;
  }

  // ---------- клики ----------
  document.addEventListener('click', (e) => {
    const t = e.target.closest('[data-wact],[data-wblock],[data-wmove]');
    if (!t || t.disabled) return;
    const d = t.dataset;
    if (d.wblock) {
      const list = blocks().filter((b) => b !== d.wblock);
      if (t.checked) list.push(d.wblock);
      setSetting('widget', list);
      return;
    }
    if (d.wmove) {
      const list = blocks(), i = list.indexOf(d.wmove), j = i + +d.dir;
      if (i < 0 || j < 0 || j >= list.length) return;
      [list[i], list[j]] = [list[j], list[i]];
      setSetting('widget', list).then(() => $(`[data-wmove="${d.wmove}"][data-dir="${d.dir}"]:not(:disabled)`)?.focus());
      return;
    }
    if (d.wact === 'collapse') { if (S.pop) { S.pop = null; renderTop(); } collapse(); }
    else if (d.wact === 'expand') expand();
    else if (d.wact === 'pin') { const v = !S.settings.widgetOnTop; topSent = v; setSetting('widgetOnTop', v); call('window_set', { width: window.innerWidth, height: window.innerHeight, center: false, onTop: v }).catch(() => {}); }
    // закрыть: Пульт уйдёт в трей или закроется - как в настройках (обработчик закрытия в main.rs)
    else if (d.wact === 'close') win()?.close().catch((err) => toast(String(err), 'bad'));
    else if (d.wact === 'log') { const id = d.id; expand(() => openLog(id)); }
    else if (d.wact === 'ci') { const r = remotesForCI()[0]; if (r?.trusted) { S.ci.remote = S.ci.remote || r.name; loadCI(r, true); draw(); } }
  });

  window.addEventListener('resize', onResize);
  // ход запусков, ветка и CI меняются без перерисовки островка - подхватываем следом за app.js
  for (const ev of ['run', 'git', 'project']) listen(ev, () => { if (on) setTimeout(draw, 0); });

  window.Widget = { sync, render: redraw, sideItem, settingsHtml, collapse, expand, isOn: () => on };
  // первый замер - когда отрисуется островок (renderTop зовёт sync), а не до загрузки базы знаний
})();
