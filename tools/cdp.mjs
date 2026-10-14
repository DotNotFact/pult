#!/usr/bin/env node
// Тестовое окно Пульта через протокол отладки WebView2 (CDP), без Playwright.
//
// Окно поднимает tools/test-window.ps1 (отладочная сборка с PULT_DEBUG_PORT).
// Порт: ключ --port N, иначе PULT_DEBUG_PORT, иначе 9222.
//
//   node tools/cdp.mjs eval "<js>"            выполнить в окне, напечатать результат (async, return)
//   node tools/cdp.mjs shot <файл.png> [w h]   снимок; с w h - на эмулированном размере
//   node tools/cdp.mjs theme light|dark        тема через localStorage и перезагрузка
//   node tools/cdp.mjs check [w h]             замер: переполнение, наложения, края, сдвиг, тире, ряды (середины, верх и высота соседей), сетка
//   node tools/cdp.mjs console [секунд]        ошибки консоли и исключения за время ожидания
//
// Размер меньше 360 x 300 включает виджет, и он запоминается в localStorage
// (widget.on): выйти можно только кнопкой «Вернуть прежний размер». Поэтому
// меньше 360 x 300 скрипт не эмулирует.
//
// Node 22+: WebSocket и fetch встроены, зависимостей нет.

import { writeFileSync } from 'node:fs';

const argv = process.argv.slice(2);
let port = Number(process.env.PULT_DEBUG_PORT) || 9222;
const pi = argv.indexOf('--port');
if (pi >= 0) {
  port = Number(argv[pi + 1]);
  argv.splice(pi, 2);
}
const [cmd, a, b, c] = argv;
const MIN = { w: 360, h: 300 };

let list;
try {
  list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
} catch {
  console.error(`cdp: порт ${port} не отвечает. Окно поднимает: pwsh -File tools/test-window.ps1 -Port ${port}`);
  process.exit(2);
}
const page = list.find((x) => x.type === 'page');
if (!page) {
  console.error(`cdp: на порту ${port} нет страницы`);
  process.exit(2);
}

const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0;
const pending = new Map();
const events = [];
ws.onmessage = (m) => {
  const d = JSON.parse(m.data);
  if (d.id && pending.has(d.id)) {
    pending.get(d.id)(d);
    pending.delete(d.id);
  } else if (d.method) events.push(d);
};
await new Promise((r) => (ws.onopen = r));
const send = (method, params = {}) =>
  new Promise((r) => {
    const i = ++id;
    pending.set(i, r);
    ws.send(JSON.stringify({ id: i, method, params }));
  });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function evaluate(body) {
  const r = await send('Runtime.evaluate', {
    expression: `(async () => { ${body} })()`,
    awaitPromise: true,
    returnByValue: true,
  });
  if (r.result?.exceptionDetails) throw new Error(r.result.exceptionDetails.exception?.description ?? 'исключение в окне');
  return r.result?.result?.value;
}

async function withSize(w, h, fn) {
  if (!w) return fn();
  if (+w < MIN.w || +h < MIN.h) {
    console.error(`cdp: ${w} x ${h} меньше ${MIN.w} x ${MIN.h} - окно уйдёт в виджет и запомнит это. Не эмулирую.`);
    process.exit(2);
  }
  await send('Emulation.setDeviceMetricsOverride', { width: +w, height: +h, deviceScaleFactor: 1, mobile: false });
  try {
    await sleep(400); // перерисовка и @container-правила
    return await fn();
  } finally {
    await send('Emulation.clearDeviceMetricsOverride');
  }
}

// Замер в окне. Анимации сперва доигрывают: щелчок или снимок посреди
// проявления меряет кадр анимации, а не раскладку.
const MEASURE = `
  await Promise.all(document.getAnimations().map((x) => x.finished.catch(() => {})));
  const W = innerWidth, H = innerHeight;
  const vis = (el) => { const s = getComputedStyle(el); const r = el.getBoundingClientRect();
    return s.visibility !== 'hidden' && s.display !== 'none' && +s.opacity > 0 && r.width > 0 && r.height > 0; };
  const name = (el) => el.tagName.toLowerCase() + (el.id ? '#' + el.id : '') + (el.classList.length ? '.' + [...el.classList].slice(0, 2).join('.') : '');
  const out = { size: W + 'x' + H, theme: document.documentElement.dataset.theme || 'auto' };
  out.pageScrollX = document.documentElement.scrollWidth > W ? document.documentElement.scrollWidth - W : 0;
  // текст, обрезанный без многоточия: содержимое шире блока при overflow visible/hidden без ellipsis
  out.clipped = [...document.querySelectorAll('body *')].filter((el) => {
    if (!vis(el) || !el.childNodes.length || el.closest('#titlebar, .con, pre, code, .sr')) return false;
    const s = getComputedStyle(el);
    if (s.textOverflow === 'ellipsis' || /auto|scroll/.test(s.overflowX)) return false;
    if (!(el.scrollWidth > el.clientWidth + 1 && [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim()))) return false;
    // шире делает не текст, а вынесенный за край значок (::after «!» у кнопки запуска) - это не обрезка
    const rg = document.createRange(); rg.selectNodeContents(el);
    return rg.getBoundingClientRect().width > el.clientWidth + 1;
  }).slice(0, 15).map(name);
  // Видимая часть элемента: прямоугольник, обрезанный прокручиваемыми предками.
  // Без обрезки содержимое, уехавшее под строку состояния при прокрутке, читалось бы наложением.
  const clip = (el) => { const r = el.getBoundingClientRect(); let l = r.left, t = r.top, rr = r.right, b = r.bottom;
    for (let p = el.parentElement; p && p !== document.documentElement; p = p.parentElement) {
      const s = getComputedStyle(p); if (s.overflowX === 'visible' && s.overflowY === 'visible') continue;
      const q = p.getBoundingClientRect(); l = Math.max(l, q.left); t = Math.max(t, q.top); rr = Math.min(rr, q.right); b = Math.min(b, q.bottom); }
    return { left: l, top: t, right: rr, bottom: b, w: rr - l, h: b - t, full: r }; };
  const all = [...document.querySelectorAll('button, a[href], input, select, textarea, [data-act], [role=button]')]
    .filter(vis).map((el) => [el, clip(el)]);
  const hits = all.filter(([, c]) => c.w > 2 && c.h > 2);
  // нажимаемое за краем окна; в горизонтально прокручиваемой ленте это законно
  const inHScroll = (el) => { for (let p = el.parentElement; p; p = p.parentElement) if (/auto|scroll/.test(getComputedStyle(p).overflowX)) return true; return false; };
  out.offEdge = all.filter(([el, c]) => (c.full.left < -1 || c.full.right > W + 1) && !inHScroll(el))
    .slice(0, 15).map(([el]) => name(el));
  // нажимаемое поверх нажимаемого - по видимым частям
  const rects = hits;
  const ov = [];
  for (let i = 0; i < rects.length; i++) for (let j = i + 1; j < rects.length; j++) {
    const [e1, r1] = rects[i], [e2, r2] = rects[j];
    if (e1.contains(e2) || e2.contains(e1)) continue;
    // всплывающее окно и окно поверх страницы лежат над ней по замыслу: их кнопки над кнопками страницы - не наложение
    // панель подробностей уже 1100 px - тоже поверх страницы, с подложкой #scrim (layout.css)
    const layer = (e) => e.id === 'scrim' ? 3 : e.closest('#pops, #overlay, #tip') ? 2 : e.closest('#drawer') && getComputedStyle(document.getElementById('drawer')).position === 'absolute' ? 1 : 0;
    if (layer(e1) !== layer(e2)) continue;
    const x = Math.min(r1.right, r2.right) - Math.max(r1.left, r2.left);
    const y = Math.min(r1.bottom, r2.bottom) - Math.max(r1.top, r2.top);
    if (x > 2 && y > 2) ov.push(name(e1) + ' / ' + name(e2));
  }
  out.overlaps = ov.slice(0, 15);
  // сдвиг раскладки с загрузки страницы (CLS-подобная сумма без ввода пользователя)
  out.layoutShift = await new Promise((res) => { let s = 0;
    try { new PerformanceObserver((l) => { for (const e of l.getEntries()) if (!e.hadRecentInput) s += e.value; })
      .observe({ type: 'layout-shift', buffered: true }); } catch { return res(null); }
    setTimeout(() => res(Math.round(s * 1000) / 1000), 300); });
  // тире в своих текстах окна; текст проекта (комментарий джобы из .gitlab-ci.yml и т.п.) помечен data-ext - это данные, а не текст Пульта
  { const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT); let n; out.dashes = false;
    while ((n = w.nextNode())) if (/[\u2014\u2013]/.test(n.textContent) && !n.parentElement.closest('[data-ext], script, style') && vis(n.parentElement)) { out.dashes = true; break; } }
  // Ряды (.claude/rules/ui.md, «Блоки» и «Проверка окна»). Ряд - дети одной сетки или flex-строки, чьи отрезки по
  // вертикали перекрываются больше чем на половину меньшего. Что сравнивается - зависит от выравнивания ряда:
  //   center                          - средние линии (кнопка рядом с заголовком, точка рядом с текстом);
  //   normal, stretch, start          - верхние края: соседние карточки начинаются на одной линии;
  //   normal, stretch (карточки)      - и высоты: растянутые карточки одной строки одной высоты.
  // Расхождение больше 1 px - находка: селектор ряда, что сравнивалось, на сколько, кто в ряду.
  const cs = (el) => getComputedStyle(el);
  const CARD = /(^|\s)(blk|card|qtile|hcard|stat|tcard|sc-card|rcard|feature|hint-box|list|rows)(\s|$)/;
  const rows = [];
  let rowsChecked = 0;
  for (const c of document.querySelectorAll('#app *, #pops *, #overlay *')) {
    if (c.closest('#titlebar, .tbody, svg') || !vis(c)) continue;
    const s = cs(c), ai = s.alignItems;
    const flexRow = /flex/.test(s.display) && s.flexDirection.startsWith('row'), grid = /grid/.test(s.display);
    if (!flexRow && !grid) continue;
    const mode = ai === 'center' ? 'mid' : /normal|stretch|start/.test(ai) ? 'top' : null;
    if (!mode) continue;
    const kids = [...c.children].filter((k) => vis(k) && !/absolute|fixed/.test(cs(k).position) && (mode === 'mid' ? /auto|normal|center/.test(cs(k).alignSelf) : /auto|normal|stretch|start/.test(cs(k).alignSelf)))
      .map((k) => [k, k.getBoundingClientRect()]).sort((a, b) => a[1].top - b[1].top);
    const lines = [];
    for (const [k, r] of kids) {
      const ln = lines.find((l) => Math.min(l.bottom, r.bottom) - Math.max(l.top, r.top) > Math.min(l.h, r.height) / 2);
      if (ln) { ln.m.push([k, r]); ln.top = Math.min(ln.top, r.top); ln.bottom = Math.max(ln.bottom, r.bottom); ln.h = Math.min(ln.h, r.height); }
      else lines.push({ top: r.top, bottom: r.bottom, h: r.height, m: [[k, r]] });
    }
    for (const l of lines) {
      if (l.m.length < 2) continue;
      // верх и высота сравниваются только у карточек: точка у строки текста или колонка рядом с островком
      // выровнены к верху намеренно и со своим полем
      if (mode === 'top' && !l.m.every(([k]) => CARD.test(k.className) || k.tagName === 'SECTION')) continue;
      rowsChecked++;
      const who = () => l.m.map(([k]) => name(k)).join(', ');
      const spread = (f) => { const v = l.m.map(([, r]) => f(r)); return Math.max(...v) - Math.min(...v); };
      const d = spread(mode === 'mid' ? (r) => r.top + r.height / 2 : (r) => r.top);
      if (d > 1) rows.push(name(c) + ' ' + (mode === 'mid' ? 'середины' : 'верх') + ' ' + d.toFixed(1) + 'px: ' + who());
      // высоты сравниваются у карточек в растягивающем ряду (у растянутых правило по умолчанию)
      if (mode === 'top' && ai !== 'start' && ai !== 'flex-start' && l.m.every(([k]) => /auto|normal|stretch/.test(cs(k).alignSelf))) {
        const dh = spread((r) => r.height);
        if (dh > 1) rows.push(name(c) + ' высота ' + dh.toFixed(1) + 'px: ' + who());
      }
    }
  }
  out.rowsChecked = rowsChecked;
  out.rows = rows.slice(0, 20);
  // Сетка: края шапки, содержимого вкладки и консоли - на линиях поля --gx; края блока - на линиях дорожек сетки.
  const off = [];
  const main = document.getElementById('main');
  if (main && vis(main)) {
    const mr = main.getBoundingClientRect(), ms = cs(main);
    const L = mr.left + main.clientLeft + parseFloat(ms.paddingLeft), R = mr.left + main.clientLeft + main.clientWidth - parseFloat(ms.paddingRight);
    out.gridLines = [Math.round(L * 10) / 10, Math.round(R * 10) / 10];
    const edge = (el, l, r, what) => { const b = el.getBoundingClientRect(); if (Math.abs(b.left - l) > 0.5 || Math.abs(b.right - r) > 0.5) off.push(name(el) + ' ' + what + ': ' + b.left.toFixed(1) + '..' + b.right.toFixed(1) + ' против ' + l.toFixed(1) + '..' + r.toFixed(1)); };
    for (const el of document.querySelectorAll('#pagehead, #tabbody>.hint-box, #tabbody>.blk-grid, #tabbody>.blk-stack, #tabbody>.blk')) if (vis(el)) edge(el, L, R, 'поле --gx');
    const con = document.getElementById('console');
    // островок консоли стоит внутрь от линий --gx на --con-in (layout.css)
    const ci = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--con-in')) || 0;
    if (con && vis(con)) edge(con, L + ci, R - ci, 'поле --gx + --con-in');
    let blocksChecked = 0;
    for (const g of document.querySelectorAll('.blk-grid')) {
      if (!vis(g)) continue;
      const gr = g.getBoundingClientRect(), tracks = cs(g).gridTemplateColumns.split(' ').map(parseFloat), gap = parseFloat(cs(g).columnGap) || 0;
      const starts = [], ends = [];
      let x = gr.left;
      for (const w of tracks) { starts.push(x); ends.push(x + w); x += w + gap; }
      for (const b of g.querySelectorAll(':scope>.blk')) {
        if (!vis(b)) continue;
        blocksChecked++;
        const r = b.getBoundingClientRect();
        if (!starts.some((v) => Math.abs(v - r.left) <= 0.5) || !ends.some((v) => Math.abs(v - r.right) <= 0.5)) off.push(name(b) + '[' + b.dataset.blk + '] дорожки: ' + r.left.toFixed(1) + '..' + r.right.toFixed(1));
      }
    }
    out.blocksChecked = blocksChecked;
  }
  out.offGrid = off.slice(0, 15);
  return out;
`;

try {
  if (cmd === 'eval') {
    console.log(JSON.stringify(await evaluate(a), null, 1));
  } else if (cmd === 'shot') {
    if (!a) throw new Error('куда сохранить: node tools/cdp.mjs shot <файл.png> [w h]');
    await withSize(b, c, async () => {
      await evaluate(`await Promise.all(document.getAnimations().map((x) => x.finished.catch(() => {})));`);
      const r = await send('Page.captureScreenshot', { format: 'png' });
      writeFileSync(a, Buffer.from(r.result.data, 'base64'));
    });
    console.log('сохранено', a);
  } else if (cmd === 'theme') {
    if (!['light', 'dark'].includes(a)) throw new Error('тема: light или dark');
    // через хранилище и перезагрузку, а не переключателем: перерисовка уже показанной
    // страницы даёт стабильно неверный кадр (урок Истока)
    await evaluate(`localStorage.setItem('pult.theme', JSON.stringify('${a}')); location.reload();`).catch(() => {});
    await sleep(1500);
    console.log('тема', a);
  } else if (cmd === 'check') {
    const r = await withSize(a, b, () => evaluate(MEASURE));
    console.log(JSON.stringify(r, null, 1));
    const bad = r.pageScrollX || r.clipped.length || r.offEdge.length || r.overlaps.length || r.dashes || r.layoutShift > 0.1 || r.rows.length || r.offGrid.length;
    process.exitCode = bad ? 1 : 0;
  } else if (cmd === 'console') {
    await send('Runtime.enable');
    await send('Log.enable');
    await sleep((Number(a) || 3) * 1000);
    const errs = events
      .filter((e) => (e.method === 'Runtime.consoleAPICalled' && e.params.type === 'error') || e.method === 'Runtime.exceptionThrown' || (e.method === 'Log.entryAdded' && e.params.entry.level === 'error'))
      .map((e) => e.params.exceptionDetails?.exception?.description ?? e.params.entry?.text ?? e.params.args?.map((x) => x.value ?? x.description).join(' '));
    console.log(errs.length ? errs.join('\n') : 'ошибок нет');
    process.exitCode = errs.length ? 1 : 0;
  } else {
    console.error('команды: eval, shot, theme, check, console (подробно - шапка файла)');
    process.exitCode = 2;
  }
} catch (e) {
  console.error(`cdp: ${e.message}`);
  process.exitCode = 1;
} finally {
  ws.close();
}
