'use strict';
// Пульт - интерфейс окна. Ядро на Rust отвечает через IPC Tauri; здесь только отрисовка.
// Все пояснения - из базы знаний (S.kb), в этом файле их нет.
const TAURI = window.__TAURI__;
const call = (cmd, args) => TAURI.core.invoke(cmd, args || {});
const listen = (ev, fn) => TAURI.event.listen(ev, (e) => fn(e.payload));
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const tpl = (s, v = {}) => String(s ?? '').replace(/\{(\w+)\}/g, (_, k) => (v[k] ?? ''));
const LS = {
  get(k, d) { try { const v = localStorage.getItem('pult.' + k); return v == null ? d : JSON.parse(v); } catch { return d; } },
  set(k, v) { try { localStorage.setItem('pult.' + k, JSON.stringify(v)); } catch { /* приватный режим */ } },
};

const RU = { success: 'успех', ok: 'готово', failed: 'ошибка', fail: 'ошибка', running: 'идёт', pending: 'в очереди', canceled: 'отменён', stopped: 'остановлен', skipped: 'пропущен', manual: 'вручную' };
// третье - короткая подпись для нижней панели узкого окна: там на подпись 50 px
const TABS = [['overview', 'Обзор', 'Обзор'], ['scripts', 'Скрипты', 'Скрипты'], ['learn', 'Документация', 'Уроки'], ['ci', 'Пайплайны', 'CI'], ['git', 'Git', 'Git'], ['history', 'История', 'История']];
// 4-й элемент (необязательный) - когда вкладка есть: у «ИИ» её нет, пока в проекте не нашлось ИИ-части (ui/ai.js)
const tabsOn = () => TABS.filter((t) => !t[3] || t[3]());
const FX_ORDER = ['danger', 'delete', 'admin', 'db', 'install', 'write', 'network', 'docker', 'read'];
const URL_RE = /\bhttps?:\/\/[^\s"'<>`]+[^\s"'<>`.,;:!?)\]]/g;
const LOCAL_URL = /^https?:\/\/(localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\]|[\w.-]+\.(localhost|test))(:\d+)?/i;

// ---------- состояние ----------
const S = {
  kb: null, project: null, byId: new Map(), scanning: false, error: null, refreshing: false,
  tab: 'overview', recent: [], settings: {}, env: null, docker: null,
  runs: new Map(), active: null,
  console: { open: LS.get('c.open', true), h: LS.get('c.h', 300), max: false, wrap: LS.get('c.wrap', true), find: '', findOpen: false, hit: -1 },
  f: null, collapsed: new Set(), detail: null, argDraft: {}, showAll: false, help: {}, helpQ: '',
  ci: { remote: null, scope: 'all', status: '', data: {} },
  hist: [], histIdx: new Map(), histFilter: { status: '', q: '' },
  gitQ: '', hintOpen: new Set(), flagsAll: new Set(), pop: null, palette: null, topic: null, topics: new Map(), allTopics: false,
  errRe: null, noErrRe: null, warnRe: null,
};
const K = () => S.kb || {};
const UI = () => K().ui || {};
// подписи для чтения с экрана и клавиатуры: knowledge/ui.json, раздел a11y
const A = (k, v) => tpl((UI().a11y || {})[k] || '', v);
// точка статуса: цвет и форма (app.css) плюс имя для чтения с экрана - статус не только цветом
const sdot = (st, cls = '') => `<span class="dot s-${esc(st)} ${cls}" role="img" aria-label="${esc(A('status', { s: RU[st] || st }))}"></span>`;
// короткое объявление для чтения с экрана: запуск начался или закончился (index.html, #sr)
function announce(text) { const el = $('#sr'); if (!el || !text) return; el.textContent = ''; requestAnimationFrame(() => { el.textContent = text; }); }
const learning = () => !!S.settings.learn;
const fxInfo = (f) => (K().effects || {})[f] || { label: f, say: '', level: 'info' };
const stages = () => K().stages || [];
const stageOf = (id) => stages().find((s) => s.id === id) || { id, label: id, hint: '', learn: '' };
const topicMeta = (id) => (K().topics || {})[id];
const g = (text) => (learning() ? Learn.gloss(esc(text)) : esc(text));

// ---------- время ----------
function ago(ts) {
  if (!ts) return '-';
  const s = Math.max(0, (Date.now() - new Date(typeof ts === 'number' ? ts : Date.parse(ts))) / 1000);
  if (s < 45) return 'только что';
  if (s < 3600) return Math.round(s / 60) + ' мин назад';
  if (s < 86400) return Math.round(s / 3600) + ' ч назад';
  if (s < 86400 * 30) return Math.round(s / 86400) + ' дн назад';
  return new Date(ts).toLocaleDateString('ru-RU');
}
function dur(ms) {
  if (ms == null || ms < 0) return '';
  const s = ms / 1000;
  if (s < 10) return s.toFixed(1).replace('.', ',') + ' с';
  if (s < 60) return Math.round(s) + ' с';
  if (s < 3600) return Math.floor(s / 60) + ' м ' + String(Math.round(s % 60)).padStart(2, '0') + ' с';
  return Math.floor(s / 3600) + ' ч ' + Math.floor((s % 3600) / 60) + ' м';
}
const clock = (ts) => new Date(ts).toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
const plural = (n, a, b, c) => { const m = n % 10, h = n % 100; return m === 1 && h !== 11 ? a : m >= 2 && m <= 4 && (h < 12 || h > 14) ? b : c; };

// ---------- сколько ещё: оценка по прошлым успешным запускам ----------
// Ничего не запрашивает и не придумывает: берёт историю запусков, уже загруженный список пайплайнов и
// память длительностей джоб. Меньше двух прошлых запусков - оценки нет. Тексты - ui.json, раздел eta.
const ET = () => UI().eta || {};
const median = (a) => { const b = [...a].sort((x, y) => x - y), m = b.length >> 1; return b.length % 2 ? b[m] : (b[m - 1] + b[m]) / 2; };
const isoMs = (s) => { const t = Date.parse(s || ''); return Number.isFinite(t) ? t : null; };
// скрипт: медиана последних 7 успешных запусков с теми же аргументами (если таких хотя бы два), иначе любых
function scriptEta(scriptId, args = '', skip = null) {
  if (!scriptId) return null;
  const ok = S.hist.filter((h) => h.scriptId === scriptId && h.status === 'ok' && h.id !== skip && h.endedAt > h.startedAt).sort((a, b) => b.startedAt - a.startedAt);
  const same = ok.filter((h) => (h.args || '') === (args || ''));
  const pick = (same.length >= 2 ? same : ok).slice(0, 7);
  return pick.length >= 2 ? median(pick.map((h) => h.endedAt - h.startedAt)) : null;
}
const runEta = (r) => (r && r.kind === 'script' && P() && r.project === P().path ? scriptEta(r.scriptId, r.args, r.id) : null);
// пайплайн: медиана 10 последних успешных с тем же именем, лучше - на той же ветке
function pipeSpan(y) { const a = isoMs(y.startedAt) ?? isoMs(y.createdAt), b = isoMs(y.updatedAt); return a != null && b != null && b > a ? b - a : null; }
function pipeEta(runs, x) {
  const ok = (runs || []).filter((y) => y.id !== x.id && y.status === 'success' && (y.name || '') === (x.name || '') && pipeSpan(y) != null);
  const ref = ok.filter((y) => y.ref === x.ref);
  const pick = (ref.length >= 2 ? ref : ok).slice(0, 10);
  return pick.length >= 2 ? median(pick.map(pipeSpan)) : null;
}
// джоба: память «имя -> последние длительности» на проект, пополняется при каждой загрузке списка джоб
const jobMemoKey = () => 'jobs:' + (P()?.path || '');
function rememberJobs(groups) {
  const memo = LS.get(jobMemoKey(), {});
  let changed = false;
  for (const j of (groups || []).flatMap((x) => x.jobs || [])) {
    if (j.status !== 'success' || j.duration == null) continue;
    const list = memo[j.name] || [];
    if (list.some(([id]) => String(id) === String(j.id))) continue;
    memo[j.name] = [[j.id, j.duration], ...list].slice(0, 10);
    changed = true;
  }
  if (changed) LS.set(jobMemoKey(), memo);
}
function jobEta(j) { const list = (LS.get(jobMemoKey(), {})[j.name] || []).map(([, d]) => d * 1000); return list.length >= 2 ? median(list) : null; }
function etaSpan(ms) { const s = Math.max(1, Math.round(ms / 1000)); return s < 60 ? tpl(ET().sec, { n: s }) : tpl(ET().min, { n: Math.round(s / 60) }); }
function etaText(start, est) {
  if (!est || start == null || !Number.isFinite(start)) return '';
  const left = est - (Date.now() - start);
  if (left >= 0) return tpl(ET().left, { t: etaSpan(left) });
  return etaLate(start, est) ? tpl(ET().over, { t: etaSpan(-left) }) : ET().soon || tpl(ET().left, { t: etaSpan(0) });
}
// «дольше обычного» - только когда перебор заметен: не меньше 2 с и не меньше пятой части оценки.
// Секунда сверху - разброс запуска, а не повод для тревоги
const etaLate = (start, est) => Date.now() - start - est >= Math.max(2000, est * 0.2);
const etaPct = (start, est) => Math.min(95, Math.max(2, ((Date.now() - start) / est) * 100));
// подпись и полоска; секундный таймер ниже обновляет их по data-s (начало) и data-e (оценка), без перерисовки
const etaOk = (start, est) => !!est && start != null && Number.isFinite(start);
function etaLabel(start, est, cls = '') { return etaOk(start, est) ? `<span class="eta ${cls}" data-eta data-s="${start}" data-e="${Math.round(est)}">${esc(etaText(start, est))}</span>` : ''; }
function etaBar(start, est, cls = '') {
  if (!etaOk(start, est)) return '';
  return `<span class="etabar ${cls} ${etaLate(start, est) ? 'over' : ''}" data-etab data-s="${start}" data-e="${Math.round(est)}" aria-hidden="true"><i style="width:${etaPct(start, est).toFixed(1)}%"></i></span>`;
}
// подсказка с оценкой: базовый текст в data-etat, строку «сколько ещё» дописывает таймер
function etaTip(base, start, est) { return etaOk(start, est) ? `data-etat="${esc(base)}" data-s="${start}" data-e="${Math.round(est)}" title="${esc(base + '\n' + etaText(start, est))}"` : `title="${esc(base)}"`; }
// подсказка счётчика «идут: N» в строке состояния: по строке на запуск, с оценкой, если она есть
function etaRunsTip(base) {
  const lines = [...S.runs.values()].filter((r) => r.status === 'running' && r.kind === 'script').map((r) => {
    const e = etaText(r.startedAt, runEta(r));
    return e ? tpl(ET().tip, { name: r.title, eta: e }) : r.title;
  });
  return [base, ...lines].join('\n');
}
function etaTick() {
  for (const el of $$('[data-eta]')) { const t = etaText(+el.dataset.s, +el.dataset.e); if (el.textContent !== t) el.textContent = t; }
  for (const el of $$('[data-etab]')) {
    const s = +el.dataset.s, e = +el.dataset.e;
    el.firstElementChild.style.width = etaPct(s, e).toFixed(1) + '%';
    el.classList.toggle('over', etaLate(s, e));
  }
  for (const el of $$('[data-etat]')) el.title = el.dataset.etat + '\n' + etaText(+el.dataset.s, +el.dataset.e);
  for (const el of $$('[data-eta-runs]')) el.title = etaRunsTip(el.dataset.etaRuns);
}

// ---------- движение ----------
// Появление - анимацией CSS (app.css, раздел «движение»), уход - классом leaving: элемент живёт до
// animationend, потом cb. Без движения (первая отрисовка, «уменьшить движение», нет анимации) - cb сразу.
const motionOn = () => document.documentElement.classList.contains('anim-ready') && !matchMedia('(prefers-reduced-motion: reduce)').matches;
function closeWithAnim(el, cb) {
  if (!el || !motionOn()) { cb(); return; }
  el.classList.add('leaving');
  const name = getComputedStyle(el).animationName;
  if (!name || name === 'none') { el.classList.remove('leaving'); cb(); return; }
  // ждём конца именно ухода: прерванное появление тоже шлёт animationcancel
  const fin = (e) => {
    if (e.target !== el || e.animationName !== name) return;
    el.removeEventListener('animationend', fin); el.removeEventListener('animationcancel', fin);
    el.classList.remove('leaving');
    cb();
  };
  el.addEventListener('animationend', fin); el.addEventListener('animationcancel', fin);
}
// короткое появление куска, который остаётся на месте (вкладка, урок, раскрытая подсказка): Web Animations,
// без перестройки раскладки; кривая и длительность - из токенов --ease и --t-med
function enterAnim(el, dy = 4) {
  if (!el || !motionOn() || !el.animate) return;
  const cs = getComputedStyle(document.documentElement);
  el.animate([{ opacity: 0, transform: `translateY(${dy}px)` }, { opacity: 1, transform: 'none' }],
    { duration: parseFloat(cs.getPropertyValue('--t-med')) || 200, easing: cs.getPropertyValue('--ease').trim() || 'ease-out' });
}

// ---------- уведомления и диалоги ----------
function toast(msg, kind = '', action = null, ms = 4200) {
  const el = document.createElement('div');
  el.className = 'toast ' + kind;
  el.innerHTML = `<span>${esc(msg)}</span>${action ? `<button type="button">${esc(action.label)}</button>` : ''}`;
  const gone = () => { if (!el._gone) { el._gone = true; closeWithAnim(el, () => el.remove()); } };
  if (action) el.querySelector('button').onclick = () => { action.fn(); gone(); };
  $('#toasts').append(el);
  // сколько сообщение видно, а не длительность анимации; под мышью и в фокусе счёт стоит (WCAG 2.2.1):
  // кнопку «Вывод» успевают нажать, а не ловят до исчезновения
  let left = ms, t0 = Date.now(), timer = setTimeout(gone, ms), held = false;
  const hold = () => { if (held) return; held = true; clearTimeout(timer); left -= Date.now() - t0; };
  const go = () => { if (!held || el.matches(':hover') || el.contains(document.activeElement)) return; held = false; t0 = Date.now(); timer = setTimeout(gone, Math.max(1200, left)); };
  el.addEventListener('mouseenter', hold); el.addEventListener('focusin', hold);
  el.addEventListener('mouseleave', go); el.addEventListener('focusout', () => setTimeout(go, 0));
}
function overlay(html, onKey) {
  const o = $('#overlay');
  // куда вернуть фокус после закрытия: то, что было в фокусе до окна (палитра, сменившаяся подтверждением, его не теряет)
  const ae = document.activeElement;
  if (ae && ae !== document.body && !o.contains(ae)) { o._ret = ae; o._retSel = retSel(ae); }
  o._gen = (o._gen || 0) + 1; o._closing = false; o.classList.remove('leaving');
  o.innerHTML = html; o.hidden = false;
  o.onclick = (e) => { if (e.target === o) closeOverlay(); };
  S.overlayKey = onKey || null;
}
// как найти кнопку снова после перерисовки: по её id или первому data-* из списка, внутри ближайшего узла с id
function retSel(el) {
  if (el.id) return '#' + CSS.escape(el.id);
  const k = ['data-act', 'data-run', 'data-run-args', 'data-c', 'data-crun', 'data-tab', 'data-detail', 'data-scact', 'data-wact'].find((x) => el.hasAttribute(x));
  if (!k) return null;
  const box = el.parentElement?.closest('[id]');
  return `${box ? '#' + CSS.escape(box.id) + ' ' : ''}[${k}="${CSS.escape(el.getAttribute(k))}"]`;
}
// состояние сбрасывается сразу, окно уходит анимацией; новое окно, открытое за это время, не прячется
function closeOverlay() {
  const o = $('#overlay');
  S.palette = null; S.overlayKey = null; const r = S.confirmResolve; S.confirmResolve = null; r?.(false);
  if (o.hidden || o._closing) return;
  const gen = o._gen;
  o._closing = true;
  // вернуть фокус туда, откуда открыли; кнопку могла заменить перерисовка (▶ стал ■) - тогда ищем её заново
  const ret = o._ret?.isConnected ? o._ret : o._retSel && $(o._retSel); o._ret = null; o._retSel = null;
  if (ret && (o.contains(document.activeElement) || document.activeElement === document.body)) ret.focus({ preventScroll: true });
  closeWithAnim(o, () => { if (o._gen !== gen) return; o._closing = false; o.hidden = true; o.innerHTML = ''; });
}
const overlayOpen = () => { const o = $('#overlay'); return !o.hidden && !o._closing; };
function confirmBox({ title, body, ok = 'Продолжить', alt = null, cancel = 'Отмена', danger = false }) {
  return new Promise((resolve) => {
    const done = (v) => { S.confirmResolve = null; closeOverlay(); resolve(v === 'cancel' ? false : v); };
    overlay(`<div class="confirm" role="dialog" aria-modal="true" aria-labelledby="cf-title" aria-describedby="cf-body"><h3 id="cf-title">${esc(title)}</h3><div id="cf-body">${body}</div>
      <div class="acts"><button class="btn" data-cf="cancel">${esc(cancel)}</button>${alt ? `<button class="btn" data-cf="alt">${esc(alt)}</button>` : ''}<button class="btn ${danger ? 'danger' : 'primary'}" data-cf="ok">${esc(ok)}</button></div></div>`,
    (e) => {
      // Enter жмёт ту кнопку, на которой фокус; без фокуса - главную
      if (e.key === 'Enter') { e.preventDefault(); const f = document.activeElement?.dataset?.cf; done(f || 'ok'); }
    });
    S.confirmResolve = (v) => resolve(v);
    $$('[data-cf]', $('#overlay')).forEach((b) => (b.onclick = () => done(b.dataset.cf)));
    setTimeout(() => $(`[data-cf=${danger ? 'cancel' : 'ok'}]`, $('#overlay'))?.focus(), 0);
  });
}
async function copy(text) {
  try { await navigator.clipboard.writeText(text); toast('Скопировано', 'ok', null, 1500); } catch { toast('Не удалось скопировать', 'bad'); }
}

// ---------- выборки ----------
const P = () => S.project;
const scriptOf = (id) => S.byId.get(id);
const isFav = (id) => (P()?.favorites || []).includes(id);
const where = (s) => (s.where === '.' ? 'корень проекта' : s.where);
const srcKind = (s) => (s.fileRel ? 'файлы-скрипты' : s.source === 'package.json' ? 'package.json' : /compose/.test(s.source) ? 'Docker Compose' : /\.(sln|slnx|csproj|fsproj)$/i.test(s.source) ? '.NET' : /^(Makefile|makefile|GNUmakefile)$/.test(s.source) ? 'Makefile' : s.source);
function isMissing(s) { return !!(s.tool && S.env && S.env[s.tool] === null); }
function missingTools() { return !S.env || !P() ? [] : [...new Set(P().scripts.filter(isMissing).map((s) => s.tool))]; }
function lastOf(id) {
  let r = null;
  for (const x of S.runs.values()) if (x.scriptId === id && x.kind === 'script' && (!r || x.startedAt > r.startedAt)) r = x;
  return r || S.histIdx.get(id) || null;
}
const gitScript = (name) => P()?.scripts.find((s) => s.source === 'git' && s.name === name)?.id;
function remotesForCI() {
  return (P()?.git?.remotes || []).filter((r) => !r.dup && r.host && /gitlab|github/i.test(r.host)).sort((a, b) => (/gitlab/i.test(b.host) ? 1 : 0) - (/gitlab/i.test(a.host) ? 1 : 0));
}

// ---------- фильтры ----------
function defaultFilters() { return { q: '', stages: [], only: '', where: '', source: '', group: 'stage', hideHooks: false }; }
function loadFilters(path) { return { ...defaultFilters(), ...LS.get('f:' + path, {}), q: '' }; }
function saveFilters() { const { q, ...rest } = S.f; LS.set('f:' + P().path, rest); }
const terms = () => S.f.q.trim().toLowerCase().split(/\s+/).filter(Boolean);
function hayOf(s) {
  return s._hay || (s._hay = `${s.name} ${s.info?.summary || ''} ${s.cmd} ${s.group} ${s.where} ${s.desc || ''} ${s.info?.header || ''} ${s.info?.about || ''} ${stageOf(s.stage).label} ${(s.info?.fx || []).map((f) => fxInfo(f).label).join(' ')} ${(s.info?.tools || []).join(' ')}`.toLowerCase());
}
const onlyTests = {
  fav: (s) => isFav(s.id),
  safe: (s) => (s.info?.fx || []).length > 0 && (s.info?.fx || []).every((x) => x === 'read'),
  running: (s) => lastOf(s.id)?.status === 'running',
  failed: (s) => lastOf(s.id)?.status === 'fail',
};
function matches(s, except) {
  const f = S.f;
  if (except !== 'q') { const t = terms(); if (t.length) { const h = hayOf(s); if (!t.every((x) => h.includes(x))) return false; } }
  if (except !== 'stage' && f.stages.length && !f.stages.includes(s.stage)) return false;
  if (except !== 'where' && f.where && s.where !== f.where) return false;
  if (except !== 'source' && f.source && srcKind(s) !== f.source) return false;
  if (except !== 'only' && f.only && !onlyTests[f.only]?.(s)) return false;
  if (f.hideHooks && s.info?.hook) return false;
  return true;
}
// подсветка по сырому тексту, потом экранирование кусков - так «amp» не находит «&amp;»
function hl(text) {
  const t = terms();
  const raw = String(text ?? '');
  if (!t.length) return esc(raw);
  const re = new RegExp(t.map((x) => x.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|'), 'gi');
  let out = '', last = 0, m;
  while ((m = re.exec(raw))) { if (!m[0]) { re.lastIndex++; continue; } out += esc(raw.slice(last, m.index)) + `<mark>${esc(m[0])}</mark>`; last = m.index + m[0].length; }
  return out + esc(raw.slice(last));
}

// ---------- значки ----------
function fxBadges(s, compact = false) {
  const fx = [...(s.info?.fx || [])].sort((a, b) => FX_ORDER.indexOf(a) - FX_ORDER.indexOf(b));
  let list = compact ? fx.filter((f) => f !== 'read').slice(0, 2) : fx;
  if (compact && !list.length && fx.includes('read')) list = ['read'];
  const out = list.map((f) => `<span class="fx fx-${esc(f)}" title="${esc(fxInfo(f).say)}">${esc(fxInfo(f).label)}</span>`);
  if (s.info?.long) out.push(`<span class="fx fx-long" title="${esc(fxInfo('long').say)}">${compact ? 'долгий' : esc(fxInfo('long').label)}</span>`);
  if (isMissing(s)) out.unshift(`<span class="fx fx-missing" title="Программа не найдена на компьютере">нет ${esc(s.tool)}</span>`);
  return `<span class="fxs">${out.join('')}</span>`;
}
function runBtn(s) {
  const r = lastOf(s.id);
  if (r?.status === 'running') return `<button class="run stop" data-run="${esc(s.id)}" title="Остановить" aria-label="Остановить ${esc(s.name)}">■</button>`;
  if (isMissing(s)) return `<button class="run" disabled title="Не запустится: нет программы «${esc(s.tool)}»" aria-label="Не запустится: нет программы «${esc(s.tool)}»">▶</button>`;
  const danger = s.danger && S.settings.confirmDanger;
  return `<button class="run ${danger ? 'danger' : ''}" data-run="${esc(s.id)}" title="${danger ? 'Запустить - сначала спрошу подтверждение' : 'Запустить: ' + esc(s.cmd)}" aria-label="Запустить ${esc(s.name)}">▶</button>`;
}
function lastHtml(r) {
  if (!r) return '';
  const t = r.status === 'running' ? 'идёт' : dur((r.endedAt || Date.now()) - r.startedAt);
  return `<span class="dot s-${esc(r.status)}" role="img" aria-label="${esc(A('status', { s: RU[r.status] || r.status }))}" title="${esc(RU[r.status] || r.status)} · ${esc(ago(r.endedAt || r.startedAt))}"></span><span>${esc(t)}</span>`;
}
// значок «i» с пояснением из базы: knowledge/ui.json, раздел infos; для чтения с экрана - картинка с этим текстом
const info = (key) => { const t = (UI().infos || {})[key]; return t ? `<span class="info" role="img" aria-label="${esc(t)}" title="${esc(t)}">i</span>` : ''; };

// ---------- перерисовка без лишней замены DOM ----------
// Та же разметка - DOM не трогаем; другая - меняются только отличающиеся узлы. Иначе событие git или
// project, пришедшее между нажатием и отпусканием кнопки мыши, заменяет кнопку, и клик теряется;
// поле ввода теряет фокус. Узел с data-own наполняет свой отрисовщик (список скриптов, урок): его
// содержимое здесь не сравнивается.
function patch(el, html) {
  if (!el || el._html === html) return false;
  if (!el.firstChild) el.innerHTML = html;
  else { const t = document.createElement('template'); t.innerHTML = html; morph(el, t.content); }
  el._html = html;
  return true;
}
function sameAttrs(a, b) {
  const skip = (n) => (a.tagName === 'INPUT' && n === 'value') || (a.tagName === 'DETAILS' && n === 'open');
  const an = [...a.attributes].filter((x) => !skip(x.name)), bn = [...b.attributes].filter((x) => !skip(x.name));
  return an.length === bn.length && an.every((x) => b.getAttribute(x.name) === x.value);
}
function morph(cur, next) {
  cur._html = null;
  const a = [...cur.childNodes], b = [...next.childNodes];
  if (a.length !== b.length) { cur.replaceChildren(...b); return; }
  for (let i = 0; i < a.length; i++) {
    const x = a[i], y = b[i];
    if (x.isEqualNode(y)) continue;
    if (x.nodeType === 3 && y.nodeType === 3) { x.nodeValue = y.nodeValue; continue; }
    // узел на пути к фокусу (сама кнопка или её предок) с другими атрибутами (on, aria-pressed, closed) правится
    // на месте, а не заменяется: замена роняла фокус клавиатуры на body, и следующий Tab начинал с начала окна.
    // Остальные узлы по-прежнему заменяются: классы, которые ставит скрипт (narrow у схемы), так не теряются
    if (x.nodeType === 1 && x.nodeName === y.nodeName && x.contains(document.activeElement) && !/^(INPUT|SELECT|TEXTAREA|OPTION|DETAILS)$/.test(x.tagName) && !sameAttrs(x, y)) {
      syncAttrs(x, y);
      if (!x.hasAttribute('data-own')) morph(x, y);
      continue;
    }
    if (x.nodeType !== 1 || x.nodeName !== y.nodeName || !sameAttrs(x, y)) { x.replaceWith(y); continue; }
    if (x.tagName === 'INPUT') {
      // поле, в котором сейчас печатают, не трогаем: его значение и так уже в состоянии
      const v = y.getAttribute('value') ?? '';
      if (x.getAttribute('value') !== v) x.setAttribute('value', v);
      if (document.activeElement !== x && x.value !== v) x.value = v;
      continue;
    }
    if (x.hasAttribute('data-own')) continue;
    morph(x, y);
  }
}
function syncAttrs(x, y) {
  for (const a of [...x.attributes]) if (!y.hasAttribute(a.name)) x.removeAttribute(a.name);
  for (const a of [...y.attributes]) if (x.getAttribute(a.name) !== a.value) x.setAttribute(a.name, a.value);
}
// DOM поменяли руками (строка выделена, плитка обновлена) - у предков нет больше права на «та же разметка»
function dirty(node) { for (let p = node; p; p = p.parentElement) p._html = null; }

// =====================================================================
// ОТРИСОВКА
// =====================================================================
function render() { renderTop(); renderMain(); renderDrawer(); renderConsole(); updateTitle(); }
function updateTitle() {
  const n = [...S.runs.values()].filter((r) => r.status === 'running' && r.kind === 'script').length;
  document.title = `${n ? `▶${n} · ` : ''}${P() ? P().name + ' - ' : ''}Пульт`;
}

// островок слева: проект, разделы, что идёт сейчас, поиск и настройки
const NAV_ICON = { overview: 'grid', scripts: 'play', learn: 'learn', ci: 'pipeline', git: 'git', history: 'history' };
const narrow = () => window.innerWidth <= 760;
// Свёрнут ли островок, если человек сам не выбирал. Порог с запасом (1080 и 1120), а не одна черта 1100:
// окно, которое тянут возле черты, не дёргает панель туда-обратно на каждом пикселе.
let autoMini = window.innerWidth < 1100;
function sideMini() { const v = LS.get('side.mini', null); return v == null ? autoMini : v; }
function renderTop() {
  const p = P(), side = $('#side');
  window.TitleBar?.setTitle(p?.name);
  document.documentElement.classList.toggle('ui-min', !!S.settings?.minimal);
  const mini = !narrow() && sideMini();
  side.classList.toggle('mini', mini);
  const runs = [...S.runs.values()].filter((r) => r.status === 'running' && r.kind === 'script');
  const failed = [...S.runs.values()].filter((r) => r.kind === 'script' && r.status === 'fail').length;
  const ciLast = p ? ciLastStatus() : null;
  const cnt = {
    scripts: runs.length ? `<span class="cnt run">${runs.length} ▶</span>` : p ? `<span class="cnt">${p.scripts.length}</span>` : '',
    learn: p?.topics?.length ? `<span class="cnt">${p.topics.length}</span>` : '',
    ci: ciLast ? `<span class="dot s-${esc(ciLast)}" role="img" aria-label="Последний пайплайн: ${esc(RU[ciLast] || ciLast)}" title="Последний пайплайн: ${esc(RU[ciLast] || ciLast)}"></span>` : '',
    git: p?.git?.dirty ? `<span class="cnt">${p.git.dirty}</span>` : '',
    history: failed ? `<span class="cnt bad">${failed} ✗</span>` : '',
  };
  if (window.AiTab) cnt.ai = window.AiTab.badge();
  const mark = { scripts: runs.length > 0, history: failed > 0, git: !!p?.git?.dirty };
  const html = `
    <div class="sb-head"><button class="sb-logo" data-act="home" title="Все проекты" aria-label="Все проекты"></button><span class="sb-title">Пульт</span>
      <button class="btn ghost icon sm" data-act="side-toggle" title="${mini ? 'Развернуть панель' : 'Свернуть панель'}" aria-label="${mini ? 'Развернуть панель' : 'Свернуть панель'}">${mini ? '»' : '«'}</button></div>
    ${p ? `<button class="sb-proj" data-act="projects" aria-haspopup="dialog" aria-expanded="${S.pop === 'projects'}" title="${esc(p.path)}&#10;Сменить проект (Ctrl+O)"><span class="av" aria-hidden="true">${esc((p.name[0] || '?').toUpperCase())}</span><span class="tx"><b>${esc(p.name)}</b><small>${esc(p.path)}</small></span><span class="car" aria-hidden="true">▾</span></button>`
      : '<button class="sb-proj" data-act="pick" title="Открыть папку проекта (Ctrl+O)"><span class="av">+</span><span class="tx"><b>Открыть проект</b><small>или перетащите папку в окно</small></span></button>'}
    <div class="sb-nav">${tabsOn().map(([id, label, short], i) => `<button class="sb-item ${p && S.tab === id ? 'on' : ''}" data-tab="${id}" ${p ? '' : 'disabled'} ${p && S.tab === id ? 'aria-current="page"' : ''} aria-label="${label}" title="${label}${i < 9 ? ` (${i + 1})` : ''}"><span class="ic">${Learn.icon(NAV_ICON[id], 18)}<span class="sb-mark ${mark[id] ? 'on' : ''}"></span></span><span class="lbl">${label}</span><span class="lbs">${short || label}</span>${cnt[id] || ''}</button>`).join('')}
      <button class="sb-item sb-more ${S.pop === 'more' ? 'on' : ''}" data-act="more" aria-haspopup="dialog" aria-expanded="${S.pop === 'more'}" aria-label="Ещё: проект, поиск, настройки, справка" title="Проект, поиск, настройки, справка"><span class="ic">${Learn.icon('gear', 18)}<span class="sb-mark ${learning() ? 'on' : ''}"></span></span><span class="lbs">Ещё</span></button></div>
    ${runs.length ? `<div class="sb-sec">Идут сейчас</div><div class="sb-runs">${runs.map((r) => `<button class="sb-run" data-crun="${esc(r.id)}" ${etaTip(r.cmd, r.startedAt, runEta(r))}>${sdot('running')}<span class="t">${esc(r.title)}</span><span class="el" data-el="${esc(r.id)}">${esc(dur(Date.now() - r.startedAt))}</span>${etaBar(r.startedAt, runEta(r), 'under')}</button>`).join('')}</div>` : ''}
    <div class="sb-foot">
      <button class="sb-search" data-act="palette" title="Найти и запустить что угодно (Ctrl+K)">${Learn.icon('search', 16)}<span>Поиск</span><kbd>Ctrl K</kbd></button>
      ${p ? `<button class="sb-item" data-act="rescan" title="Перечитать проект с диска (F5)">${S.refreshing ? '<span class="spin"></span>' : Learn.icon('refresh', 18)}<span class="lbl">${S.refreshing ? 'Обновляю…' : 'Перечитать'}</span></button>` : ''}
      <button class="sb-item ${learning() ? 'on' : ''}" data-act="toggle-learn" aria-pressed="${learning()}" title="${esc((UI().learnMode || {}).text || 'Режим обучения')}">${Learn.icon('book', 18)}<span class="lbl">Обучение</span><span class="cnt">${learning() ? 'вкл' : 'выкл'}</span></button>
      ${window.Widget?.sideItem() || ''}
      <button class="sb-item" data-act="settings" aria-haspopup="dialog" aria-expanded="${S.pop === 'settings'}" title="Настройки">${Learn.icon('gear', 18)}<span class="lbl">Настройки</span></button>
      <button class="sb-item" data-act="help" title="Справка и горячие клавиши (?)">${Learn.icon('help', 18)}<span class="lbl">Справка</span></button>
    </div>`;
  if (S.pop === 'more' && !narrow()) S.pop = null;
  // та же разметка - DOM не трогаем: иначе клик, попавший между нажатием и отпусканием кнопки мыши, теряется
  patch(side, html);
  const skip = $('#skip'), skipT = A('skip') || 'К содержимому';
  if (skip && skip.textContent !== skipT) skip.textContent = skipT;
  setPops();
  window.Widget?.sync(); window.StatusBar?.render(); // ui/widget.js, ui/statusbar.js
  if (S.pop === 'projects') setTimeout(() => $('#pq')?.focus(), 0);
}
// Ширину островка и нижнюю панель выбирает CSS (медиазапросы), сразу и без задержки. Здесь только класс
// mini: меняется синхронно с размером окна, а не через отложенную перерисовку. Раньше перерисовка ждала
// 80 мс тишины, а ширину задавала переменная из прошлого размера - пока окно тянут, островок схлопывался
// до 0 и потом раскрывался анимацией: «панель сама открывается и закрывается».
window.addEventListener('resize', () => {
  const w = window.innerWidth;
  if (w < 1080) autoMini = true; else if (w > 1120) autoMini = false;
  const mini = !narrow() && sideMini();
  if ($('#side').classList.contains('mini') !== mini || (S.pop === 'more' && !narrow())) renderTop();
});
// всплывающее окно у островка: новое появляется анимацией CSS, закрытое уходит своей (closeWithAnim)
const popsHtml = () => (S.pop === 'projects' ? projectsPop() : '') + (S.pop === 'settings' ? settingsPop() : '') + (S.pop === 'more' ? morePop() : '');
// Фокус: открытое окно забирает его на первое поле или кнопку (у проектов - поле поиска, renderTop),
// закрытое (Esc, выбор пункта) возвращает туда, откуда открыли. Щелчок мышью мимо окна фокус не трогает.
function setPops() {
  const host = $('#pops'), cur = host.firstElementChild, html = popsHtml();
  if (!html && cur && !cur.classList.contains('leaving')) {
    // кнопку, открывшую окно, перерисовка могла заменить (aria-expanded) - тогда она ищется заново (retSel)
    const back = host._ret?.isConnected ? host._ret : host._retSel && $(host._retSel);
    host._ret = null; host._retSel = null; host._pop = null;
    if (back && (cur.contains(document.activeElement) || document.activeElement === document.body)) back.focus({ preventScroll: true });
    closeWithAnim(cur, () => patch(host, popsHtml()));
    return;
  }
  if (!html && cur) return; // уже уходит
  patch(host, html);
  if (S.pop && S.pop !== host._pop) {
    const ae = document.activeElement;
    if (ae && ae !== document.body && !host.contains(ae)) { host._ret = ae; host._retSel = retSel(ae); }
    if (S.pop !== 'projects') setTimeout(() => $('#pops .pop')?.querySelector('button:not([disabled]), input, select')?.focus(), 0);
  }
  host._pop = S.pop;
}
function sortedRecent() { return [...S.recent].sort((a, b) => (b.pinned ? 1 : 0) - (a.pinned ? 1 : 0) || b.at - a.at); }
function popList() { const q = (S.popQ || '').toLowerCase(); return sortedRecent().filter((r) => !q || `${r.name} ${r.path}`.toLowerCase().includes(q)); }
function projectsPop() {
  const list = popList();
  const sel = Math.min(S.popSel || 0, list.length - 1);
  // поле - комбобокс: стрелки двигают выбор по списку, чтение с экрана слышит выбранный проект
  return `<div class="pop pop-projects" role="dialog" aria-label="${esc(A('projectQ'))}">
    <div class="ph"><input type="search" id="pq" role="combobox" aria-expanded="${list.length > 0}" aria-controls="pl" aria-autocomplete="list" ${list.length ? `aria-activedescendant="pi-p${sel}"` : ''} aria-label="${esc(A('projectQ'))}" placeholder="Найти проект или вставить путь…" value="${esc(S.popQ || '')}" autocomplete="off"><button class="btn primary" data-act="pick">Открыть папку…</button></div>
    <div class="pl" id="pl" role="listbox" aria-label="${esc(A('results'))}">${list.length ? list.map((r, i) => `<div class="pitem ${r.exists === false ? 'missing' : ''} ${i === sel ? 'sel' : ''}" data-open="${esc(r.path)}" title="${r.exists === false ? 'Папки больше нет' : esc(r.path)}">
      <span class="po" id="pi-p${i}" role="option" aria-selected="${i === sel}">${r.pinned ? '<span class="pin" title="Закреплён">★</span>' : ''}<span class="nm">${esc(r.name)}</span><span class="pt">${esc(r.path)}</span></span>
      <button class="x" data-recent-pin="${esc(r.path)}" title="${r.pinned ? 'Открепить' : 'Закрепить сверху'}" aria-label="${r.pinned ? 'Открепить' : 'Закрепить сверху'}: ${esc(r.name)}">${r.pinned ? '☆' : '★'}</button>
      <button class="x" data-recent-rm="${esc(r.path)}" title="Убрать из списка" aria-label="Убрать из списка: ${esc(r.name)}">✕</button></div>`).join('')
      : `<div class="note" style="padding:14px">${S.popQ ? 'Не нашлось. Enter - открыть как путь.' : 'Недавних проектов пока нет.'}</div>`}</div>
    <div class="pf"><span>↑↓ выбор · Enter открыть · Esc закрыть · можно перетащить папку в окно</span><span>${S.recent.length}</span></div></div>`;
}
// узкое окно: всё, что у широкого островка внизу и вверху, - одним меню над нижней панелью
function morePop() {
  const p = P();
  const it = (act, icon, label, sub = '') => `<button class="sb-item" data-act="${act}">${Learn.icon(icon, 18)}<span class="lbl">${esc(label)}</span>${sub ? `<span class="cnt">${esc(sub)}</span>` : ''}</button>`;
  return `<div class="pop pop-more" role="dialog" aria-label="Ещё">
    ${p ? `<button class="sb-proj" data-act="projects" title="${esc(p.path)}"><span class="av">${esc((p.name[0] || '?').toUpperCase())}</span><span class="tx"><b>${esc(p.name)}</b><small>${esc(p.path)}</small></span><span class="car">▾</span></button>`
      : '<button class="sb-proj" data-act="pick"><span class="av">+</span><span class="tx"><b>Открыть проект</b><small>или перетащите папку в окно</small></span></button>'}
    ${it('palette', 'search', 'Поиск', 'Ctrl K')}
    ${p ? it('rescan', 'refresh', 'Перечитать проект', 'F5') : ''}
    ${it('toggle-learn', 'book', 'Обучение', learning() ? 'вкл' : 'выкл')}
    ${window.Widget?.sideItem() || ''}
    ${it('settings', 'gear', 'Настройки')}
    ${it('help', 'help', 'Справка', '?')}
    ${p ? it('home', 'grid', 'Все проекты') : ''}
  </div>`;
}
function settingsPop() {
  const s = S.settings;
  const tog = (k, title, sub) => `<label class="set"><input type="checkbox" data-set="${k}" ${s[k] ? 'checked' : ''}><span>${esc(title)}<small>${esc(sub)}</small></span></label>`;
  const theme = LS.get('theme', 'auto'), fs = LS.get('termSize', 12.5);
  const lm = UI().learnMode || {};
  return `<div class="pop pop-settings" role="dialog" aria-label="Настройки">
    <h4>Обучение</h4>
    ${tog('learn', lm.title || 'Режим обучения', lm.text || '')}
    ${tog('minimal', (UI().minimalMode || {}).title || 'Минимальный интерфейс', (UI().minimalMode || {}).text || '')}
    <h4 style="margin-top:12px">Поведение</h4>
    ${tog('confirmDanger', 'Подтверждать опасные запуски', 'выкладка, публикация, удаление, prod - спрошу перед запуском')}
    ${tog('autoRefresh', 'Следить за файлами проекта', 'правка package.json или смена ветки обновит экран сама')}
    ${tog('ciAutoload', 'Загружать статус CI при открытии вкладки', 'один запрос; дальше - только по кнопке')}
    ${tog('colors', 'Цвета в выводе', 'просить программы печатать в цвете (FORCE_COLOR)')}
    ${tog('notify', 'Уведомлять о завершении', 'всплывающее уведомление Windows, если окно не в фокусе')}
    <div class="set-row"><span>Доверенные серверы CI<small class="note" style="display:block">кроме gitlab.com и github.com, через запятую</small></span><input type="text" id="cihosts" aria-label="${esc(A('ciHosts'))}" spellcheck="false" value="${esc((s.ciHosts || []).join(', '))}" style="width:140px"></div>
    <h4 style="margin-top:12px">Вид</h4>
    <div class="set-row"><span>Тема</span><div class="seg">${[['auto', 'Авто'], ['light', 'Светлая'], ['dark', 'Тёмная']].map(([k, l]) => `<button class="${theme === k ? 'on' : ''}" aria-pressed="${theme === k}" data-theme-set="${k}">${l}</button>`).join('')}</div></div>
    <div class="set-row"><span>Шрифт консоли</span><div class="seg">${[11.5, 12.5, 13.5, 15].map((n) => `<button class="${fs === n ? 'on' : ''}" aria-pressed="${fs === n}" data-term-size="${n}">${String(n).replace('.', ',')}</button>`).join('')}</div></div>
    <div class="set-row"><span>Подсказки</span><button class="btn sm" data-act="reset-hints">Показать снова</button></div>
    <div class="set-row"><span>База знаний<small class="note" style="display:block">${K().fromDisk ? 'читается из папки knowledge' : 'встроенная копия'}</small></span><button class="btn sm" data-act="kb-reload">Перечитать</button></div>
    ${window.Widget?.settingsHtml() || ''}
  </div>`;
}

function renderMain() {
  const m = $('#main');
  // идёт разбор - область занята: чтение с экрана не зачитывает заглушки-скелеты
  if (S.scanning) m.setAttribute('aria-busy', 'true'); else m.removeAttribute('aria-busy');
  if (S.scanning) { patch(m, `<div class="wrap"><div class="note" style="margin:30px 0 10px"><span class="spin"></span> Разбираю проект…</div>${'<div class="skel"></div>'.repeat(6)}</div>`); return; }
  if (S.error && !P()) {
    patch(m, `<div class="empty"><h1>Не открылось</h1><p class="lead">${esc(S.error.msg)}<br><span class="faint">${esc(S.error.path || '')}</span></p>
      <button class="btn primary" data-act="pick">Выбрать другую папку…</button> ${S.error.path ? `<button class="btn" data-open="${esc(S.error.path)}">Попробовать снова</button>` : ''}</div>`);
    return;
  }
  if (!P()) return renderEmpty();
  const keep = m.scrollTop;
  // каркас страницы ставится один раз; дальше шапка и вкладка сверяются каждая со своей прошлой разметкой
  if (!$('#pagehead') || !m.contains($('#pagehead'))) { m.innerHTML = '<div class="wrap"><header class="pagehead" id="pagehead"></header><div id="tabbody"></div></div>'; m._html = null; }
  renderHead();
  renderTab();
  m.scrollTop = keep;
}
// Шапка страницы - сетка (layout.css): строка 1 - проект и кнопки на одной средней линии; строка 2 - раздел,
// версия, ветка, свежесть разбора (справа - «Блоки» на «Обзоре»); строка 3 - описание в две строки с «ещё»;
// строка 4 - стек. Ничего не стоит на своей базовой линии: каждая строка выравнивает детей по центру.
function renderHead() {
  const head = $('#pagehead'), p = P();
  if (!head || !p) return;
  const label = (TABS.find(([id]) => id === S.tab) || [])[1] || (S.tab === 'scenarios' ? (UI().scenarios || {}).tab || '' : '');
  const full = S.tab === 'overview', PH = UI().pagehead || {}, open = !!S.descOpen;
  patch(head, `
      <h1 class="ph-title"><button class="h1btn" data-act="projects" title="Сменить проект (Ctrl+O)">${esc(p.name)}</button></h1>
      <div class="ph-acts hero-actions">
        <button class="btn sm" data-open-what="explorer" title="Открыть папку проекта в проводнике">Проводник</button>
        <button class="btn sm" data-open-what="terminal" title="Открыть терминал в папке проекта">Терминал</button>
        <button class="btn sm" data-open-what="code" title="Открыть проект в VS Code">VS Code</button>
      </div>
      <div class="ph-meta"><span class="crumb">${esc(label)}</span>${p.version ? `<span class="ph-m">v${esc(p.version)}</span>` : ''}${p.git ? `<span class="ph-m ph-branch" title="Ветка git">${Learn.icon('branch', 13)}${esc(p.git.branch)}</span>` : ''}<span class="ph-m freshness" title="${esc((UI().infos || {}).fresh || '')}">${p.cached ? 'из кэша · ' : ''}разобрано ${esc(ago(p.scannedAt))}${p.scanMs ? ` за ${p.scanMs} мс` : ''}</span></div>
      ${full ? `<div class="ph-tools">${Blocks.listButton('overview')}</div>` : ''}
      ${full && p.description ? `<div class="ph-desc"><p class="desc${open ? ' open' : ''}" id="ph-desc">${esc(p.description)}</p><button type="button" class="tlink ph-more" data-act="desc" aria-controls="ph-desc" aria-expanded="${open}" aria-label="${esc(open ? PH.lessLabel : PH.moreLabel)}"${open || S.descClamp ? '' : ' hidden'}>${esc(open ? PH.less : PH.more)}</button></div>` : ''}
      ${full && p.stacks.length ? `<div class="chips ph-chips">${p.stacks.map((x) => `<span class="chip">${esc(x)}</span>`).join('')}</div>` : ''}`);
  watchDesc();
}
// «ещё» у описания - только когда две строки правда обрезали текст: замер после раскладки (ResizeObserver),
// а не чтением размеров посреди отрисовки
let descEl = null;
const descRO = typeof ResizeObserver === 'function' ? new ResizeObserver(() => {
  const el = $('#ph-desc');
  if (!el || S.descOpen) return;
  const over = el.scrollHeight > el.clientHeight + 1;
  if (over !== !!S.descClamp) { S.descClamp = over; renderHead(); }
}) : null;
function watchDesc() {
  const el = $('#ph-desc');
  if (el === descEl || !descRO) return;
  if (descEl) descRO.unobserve(descEl);
  descEl = el;
  if (el) descRO.observe(el);
}
function renderTab() {
  const el = $('#tabbody');
  if (!el || !P()) return;
  // блок тянут - страница не перерисовывается под указателем; перерисовка догонит после отпускания
  if (S.tab === 'overview') { if (Blocks.busy()) { Blocks.defer(); return; } patch(el, viewOverview()); Blocks.mount(el); trackArrows(el); }
  else if (S.tab === 'scripts') { patch(el, viewScripts()); renderChips(); renderList(); }
  else if (S.tab === 'learn') { patch(el, viewLearn()); ensureTopic(); }
  else if (S.tab === 'ci') { patch(el, viewCI()); ensureCI(); }
  else if (S.tab === 'git') patch(el, viewGit());
  else if (S.tab === 'history') patch(el, viewHistory());
  else if (S.tab === 'scenarios') { el._html = null; window.Scen?.renderTab(el); } // ui/scenarios.js рисует сам
  else if (S.tab === 'ai') window.AiTab?.renderTab(el); // ui/ai.js: своя разметка через patch
}
function setTab(t) {
  const moved = t !== S.tab;
  S.tab = t;
  S.pop = null;
  if (P()) LS.set('tab:' + P().path, t);
  if (moved) closeTransient();
  window.Scen?.onTab(t); // ui/scenarios.js: открытый редактор закрывается, черновик остаётся в памяти
  window.AiTab?.onTab(t); // ui/ai.js: карта ИИ-части запрашивается при входе во вкладку
  renderTop();
  renderMain();
  $('#main').scrollTop = 0;
  if (moved) enterAnim($('#tabbody')); // новая вкладка проявляется, старая уходит сразу: быстрый обход не ждёт
}
// переход на другую вкладку закрывает всё временное: панель подробностей, палитру и справку, поиск в выводе,
// подсказку к термину. Консоль остаётся открытой - в ней идут запуски.
function closeTransient() {
  if (!$('#overlay').hidden) closeOverlay();
  Blocks.close(false);
  if (S.detail) { S.detail = null; renderDrawer(); }
  if (S.console.findOpen) { S.console.findOpen = false; S.console.find = ''; S.console.hit = -1; renderConsole(); }
  $('#tip').hidden = true;
}
function hintBox(key) {
  const h = (UI().hints || {})[key];
  if (!h || (!learning() && LS.get('hint.' + key))) return '';
  const open = S.hintOpen.has(key);
  return `<div class="hint-box ${open ? 'open' : ''}"><button class="hint-chip" data-hint-x="${esc(key)}" aria-expanded="${open}"><span class="q" aria-hidden="true">?</span><span class="ht">${esc(h.title)}</span></button><div><b>${esc(h.title)}</b><ul>${(h.items || []).map((x) => `<li>${g(x)}</li>`).join('')}</ul></div>${learning() ? '<span class="btn sm slot" aria-hidden="true">Понятно</span>' : `<button class="btn sm" data-hint="${esc(key)}">Понятно</button>`}</div>`;
}

function renderEmpty() {
  const rec = sortedRecent();
  patch($('#main'), `<div class="empty">
    <img class="empty-art" src="img/empty.png" alt="" width="240" height="225">
    <h1><i></i>Пульт</h1>
    <p class="lead">Откройте папку любого проекта. Пульт найдёт, что в нём можно запустить,<br>объяснит каждую команду и покажет уроки по технологиям проекта.<br><span class="faint">Ничего не меняет, пока вы сами не нажмёте ▶.</span></p>
    <button class="btn primary" data-act="pick" style="padding:10px 22px;font-size:15px">Открыть папку… <span class="k">Ctrl O</span></button>
    <div class="pathbox"><input type="text" id="pathin" aria-label="${esc(A('path'))}" spellcheck="false" placeholder="или вставьте путь: D:\\Projects\\мой-проект" autocomplete="off"><button class="btn" data-act="openpath">Открыть</button></div>
    <p class="note">Ещё можно просто перетащить папку в это окно.</p>
    ${rec.length ? `<div class="recent"><h4>Недавние</h4>${rec.map((r) => `<div class="rcard ${r.exists === false ? 'missing' : ''}" role="button" tabindex="0" data-open="${esc(r.path)}" title="${r.exists === false ? 'Папки больше нет' : ''}"><b>${r.pinned ? '★ ' : ''}${esc(r.name)}</b><span class="pt">${esc(r.path)}</span></div>`).join('')}</div>` : ''}
  </div>`);
}

// ---------- Обзор ----------
function quickList() {
  const p = P();
  const out = (p.favorites || []).map(scriptOf).filter(Boolean);
  for (const pat of (UI().quick || {}).names || []) {
    if (out.length >= 8) break;
    let re; try { re = new RegExp(pat); } catch { continue; }
    const s = p.scripts.find((x) => re.test(x.name) && !out.includes(x) && x.where === '.') || p.scripts.find((x) => re.test(x.name) && !out.includes(x));
    if (s) out.push(s);
  }
  return out.slice(0, 8);
}
function qtile(s) {
  return `<div class="qtile" tabindex="0" data-detail="${esc(s.id)}" data-sid="${esc(s.id)}" title="${esc(s.info?.summary)}">${runBtn(s)}<div class="t"><b>${isFav(s.id) ? '★ ' : ''}${esc(s.name)}</b><span>${esc(s.info?.summary)}</span></div>${lastHtml(lastOf(s.id))}</div>`;
}
// Обзор - сетка блоков (ui/blocks.js). Каждый раздел - запись в реестре: заголовок, ссылка в шапке и
// пустое состояние - knowledge/ui.json, раздел blocks; порядок, ширину, свёртку и скрытие выбирает человек.
// Стрелка между этапами нужна, только если следующий этап на той же строке: при переносе она висела
// в конце строки. Пересчёт - после отрисовки и когда меняется ширина цепочки.
const trackRO = window.ResizeObserver ? new ResizeObserver((es) => es.forEach((e) => hideStrayArrows(e.target))) : null;
function hideStrayArrows(track) {
  // этап и его стрелка - один сегмент .tseg; стрелка видна, если следующий сегмент на той же строке
  const segs = [...track.children].filter((x) => x.classList.contains('tseg'));
  segs.forEach((s, i) => {
    const a = s.querySelector('.tarrow'), next = segs[i + 1];
    if (a) a.style.visibility = next && Math.abs(s.offsetTop - next.offsetTop) < 4 ? '' : 'hidden';
  });
}
function trackArrows(root) {
  for (const tr of root.querySelectorAll('.track')) { hideStrayArrows(tr); if (trackRO && !tr._ro) { trackRO.observe(tr); tr._ro = 1; } }
}
function viewOverview() { return hintBox('overview') + Blocks.grid('overview'); }
const BT = () => UI().blocks || {};
const blkEmpty = (k) => `<p class="blk-empty">${esc((BT().empty || {})[k] || '')}</p>`;
const blkAside = (k, tab) => `<button class="tlink" data-tab="${tab}">${esc((BT().asides || {})[k] || '')}</button>`;
// путь разработки - цепочка этапов: точка состояния, этап, число скриптов, сколько джоб CI; без номеров.
// Стрелка - хвост этапа, а не начало следующего: при переносе строки она не повисает в начале новой
function flowHtml() {
  const p = P(), by = {}, ciBy = {}, runBy = {}, failBy = {};
  for (const s of p.scripts) {
    by[s.stage] = (by[s.stage] || 0) + 1;
    const r = lastOf(s.id);
    if (r?.status === 'running') runBy[s.stage] = 1;
    if (r?.status === 'fail') failBy[s.stage] = 1;
  }
  for (const c of p.ci) { for (const st of c.stages || []) ciBy[st.sid] = (ciBy[st.sid] || 0) + st.jobs.length; for (const j of c.jobs || []) ciBy[j.sid] = (ciBy[j.sid] || 0) + 1; }
  const FL = UI().flow || {};
  const list = stages().filter((s) => s.id !== 'other' || by.other);
  return `<div class="track">${list.map((s, i) => {
    const n = by[s.id] || 0, c = ciBy[s.id] || 0;
    const st = runBy[s.id] ? 'run' : failBy[s.id] ? 'fail' : '';
    const why = st === 'run' ? FL.running : st === 'fail' ? FL.failed : '';
    const tip = [s.hint, why, learning() ? s.learn : ''].filter(Boolean).join(' · ');
    return `<span class="tseg"><button class="tstep ${n || c ? '' : 'zero'}" data-stage="${esc(s.id)}" title="${esc(tip)}"><span class="tdot ${st}"></span><b>${esc(s.label)}</b><span class="tn">${n}</span>${c ? `<span class="tci">${esc(tpl(FL.ci || 'CI {n}', { n: c }))}</span>` : ''}</button>${i < list.length - 1 ? '<span class="tarrow" aria-hidden="true"></span>' : ''}</span>`;
  }).join('')}</div>`;
}
function topicTile(t) {
  return `<div class="qtile" role="button" tabindex="0" data-topic="${esc(t.id)}" title="${esc(t.summary)}"><span class="tico" style="background:${esc(t.color)}">${Learn.icon(t.icon, 18, Learn.ink(t.color))}</span><div class="t"><b class="plain">${esc(t.title)}</b><span>${esc(t.why)}</span></div></div>`;
}
function docLi(d) {
  return `<div class="li click" tabindex="0" ${d.kind === 'dir' ? 'role="button"' : ''} data-doc="${esc(d.rel)}"><span class="t">${esc(d.name)}<span class="sub">${esc(d.hint || '')}</span></span>${d.kind === 'dir' ? `<span class="d">${d.count} ${plural(d.count, 'файл', 'файла', 'файлов')}</span>` : `<button class="btn ghost sm" data-edit="${esc(d.rel)}" title="Открыть в редакторе" aria-label="Открыть в редакторе: ${esc(d.name)}">✎</button>`}</div>`;
}
Blocks.register({ id: 'quick', info: 'quick', aside: () => blkAside('quick', 'scripts'),
  render: () => { const q = quickList(); return q.length ? `<div class="quick">${q.map(qtile).join('')}</div>` : blkEmpty('quick'); } });
Blocks.register({ id: 'flow', info: 'flow', render: flowHtml });
Blocks.register({ id: 'health', info: 'health', flush: true, render: () => `<div class="hlist">${healthItems().map(healthRow).join('')}</div>` });
Blocks.register({ id: 'topics', info: 'topics', when: () => !!(P()?.topics || []).length, aside: () => blkAside('topics', 'learn'),
  render: () => `<div class="quick">${(P().topics || []).slice(0, 8).map(topicTile).join('')}</div>` });
Blocks.register({ id: 'runs', span: 'half', flush: () => recentRuns(6).length > 0, aside: () => blkAside('runs', 'history'),
  render: () => { const h = recentRuns(6); return h.length ? `<div class="list">${h.map(histLi).join('')}</div>` : blkEmpty('runs'); } });
Blocks.init({
  texts: () => UI().blocks,
  rerender: () => { if (P() && S.tab === 'overview') renderMain(); },
  announce, dirty, info,
  icon: (n, size, sw) => Learn.icon(n, size, 'currentColor', sw),
});
Blocks.register({ id: 'docs', span: 'half', flush: () => P().docs.length > 0,
  render: () => (P().docs.length ? `<div class="list">${P().docs.map(docLi).join('')}</div>` : blkEmpty('docs')) });
function H(key, vars) { const h = (UI().health || {})[key] || { t: key, d: '' }; return { t: tpl(h.t, vars), d: tpl(h.d, vars), btn: h.btn ? tpl(h.btn, vars) : null }; }
function healthItems() {
  const p = P(), gi = p.git, h = p.health || {}, items = [];
  const add = (lv, key, vars = {}, extra = {}) => items.push({ lv, ...H(key, vars), ...extra });
  if (!gi) add('info', 'notGit');
  else if (gi.untrusted) {
    // git не запускался: настройки чужого репозитория опасны (ядро, git::locate)
    const why = tpl((UI().health?.gitWhy || {})[gi.why] || '', { key: gi.whyKey || '' });
    if (gi.refused) add('bad', 'gitRefused', { why });
    else add('warn', 'gitUntrusted', { why }, { act: 'git-trust' });
  } else {
    if (gi.op) add('bad', gi.op, {}, { tab: 'git' });
    if (gi.dirty) add('warn', 'dirty', { n: `${gi.dirty} ${plural(gi.dirty, 'изменение', 'изменения', 'изменений')}`, branch: gi.branch }, { tab: 'git' });
    else add('ok', 'clean', { branch: gi.branch });
    if (/отсоед/.test(gi.branch)) add('warn', 'detached');
    else if (!gi.upstream && gi.remotes.length) add('warn', 'noUpstream');
    if (gi.gone) add('warn', 'gone');
    if (gi.behind) add('warn', 'behind', { n: `${gi.behind} ${plural(gi.behind, 'коммит', 'коммита', 'коммитов')}` }, { run: gitScript('git pull --ff-only') });
    if (gi.ahead) add('info', 'ahead', { n: gi.ahead });
    if (gi.remotes.length) {
      const age = gi.fetchedAt ? Date.now() - gi.fetchedAt : null;
      if (age == null) add('warn', 'neverFetched', {}, { run: gitScript('git fetch') });
      else if (age > 86400_000) add('warn', 'staleFetch', { ago: ago(gi.fetchedAt) }, { run: gitScript('git fetch') });
    }
    if (gi.stash) add('info', 'stash', { n: gi.stash });
  }
  if (h.deps === 'missing') add('bad', 'depsMissing', {}, { run: h.installScript });
  else if (h.deps === 'stale') add('warn', 'depsStale', {}, { run: h.installScript });
  else if (h.deps === 'ok') add('ok', 'depsOk');
  if (h.node?.want) add(h.node.ok === false ? 'warn' : 'ok', h.node.ok === false ? 'nodeWrong' : 'nodeOk', h.node);
  if (h.env) add('ok', 'envOk'); else if (h.envExample) add('warn', 'envMissing');
  if (h.docker) {
    const d = S.docker;
    if (!d) add('info', 'dockerChecking');
    else if (d.state === 'up') add('ok', 'dockerUp', { running: d.running ?? 0, version: d.version || '' });
    else if (d.state === 'down') add('bad', 'dockerDown', { error: d.error || 'не отвечает' }, { act: 'docker-recheck' });
    else add('bad', 'dockerAbsent');
  }
  const miss = missingTools();
  if (miss.length) add('bad', 'toolsMissing', { list: miss.join(', ') });
  else if (S.env) add('ok', 'toolsOk');
  if (p.ci.length) {
    const last = ciLastStatus();
    const kinds = [...new Set(p.ci.map((c) => ({ gitlab: 'GitLab', github: 'GitHub Actions', jenkins: 'Jenkins' }[c.kind] || c.file)))].join(', ');
    add(last === 'failed' ? 'bad' : last === 'success' ? 'ok' : 'info', last ? 'ciLast' : 'ci', { kinds, last: RU[last] || last }, { tab: 'ci' });
  } else add('info', 'ciNone');
  for (const w of p.warnings || []) add('warn', 'warning', { text: w });
  if (h.hooks) add('info', 'hooks');
  if (h.agent) add('info', 'agent');
  const rank = { bad: 0, warn: 1, ok: 2, info: 3 };
  return items.sort((a, b) => rank[a.lv] - rank[b.lv]);
}
function healthRow(x) {
  const act = [];
  if (x.run && scriptOf(x.run)) act.push(`<button class="btn sm" data-run="${esc(x.run)}">▶ ${esc(x.btn || 'Запустить')}</button><button class="btn ghost sm" data-detail="${esc(x.run)}">что это?</button>`);
  else if (x.tab) act.push(`<button class="btn sm" data-tab="${esc(x.tab)}">${esc(x.btn || 'Открыть')}</button>`);
  if (x.act === 'git-trust') act.push(`<button class="btn sm" data-act="git-trust">${esc(x.btn || '')}</button>`);
  if (x.act === 'docker-recheck') act.push(`<button class="btn sm" data-act="docker-recheck">${esc(x.btn || 'Проверить ещё раз')}</button>`);
  // строка списка: точка (цвет и форма) на линии заголовка, под ним пояснение, справа - чем исправить
  return `<div class="hrow lv-${esc(x.lv)}"><div class="tx"><b><span aria-hidden="true" class="dot d-${x.lv === 'info' ? 'info' : esc(x.lv)}"></span>${esc(x.t)}</b>${x.d ? `<span class="hd">${g(x.d)}</span>` : ''}</div>${act.length ? `<div class="act">${act.join('')}</div>` : ''}</div>`;
}
function recentRuns(n) {
  const cur = [...S.runs.values()].filter((r) => r.kind === 'script' && r.project === P().path);
  const ids = new Set(cur.map((r) => r.id));
  return [...cur, ...S.hist.filter((h) => !ids.has(h.id))].sort((a, b) => b.startedAt - a.startedAt).slice(0, n);
}
function histLi(r) {
  return `<div class="li click" role="button" tabindex="0" data-log="${esc(r.id)}" title="Показать вывод">${sdot(r.status)}<span class="t"><span class="mono">${esc(r.title)}</span>${r.args ? ` <span class="faint">${esc(r.args)}</span>` : ''}</span><span class="d">${r.status === 'running' ? 'идёт' : esc(dur((r.endedAt || Date.now()) - r.startedAt))} · ${esc(ago(r.startedAt))}</span></div>`;
}

// ---------- Скрипты ----------
function viewScripts() {
  return `${hintBox('scripts')}
    <div class="toolbar">
      <div class="search"><input type="search" id="q" placeholder="Поиск по имени, описанию, команде…" value="${esc(S.f.q)}" autocomplete="off" aria-label="Поиск скриптов"><button class="clear" data-act="clearq" ${S.f.q ? '' : 'hidden'} aria-label="Очистить">×</button></div>
      <div class="seg" title="Как группировать">${[['stage', 'По этапам'], ['where', 'По папкам'], ['source', 'По источнику']].map(([k, l]) => `<button class="${S.f.group === k ? 'on' : ''}" aria-pressed="${S.f.group === k}" data-group="${k}">${l}</button>`).join('')}</div>
      <span class="cnt" id="cnt"></span>
    </div>
    <div class="fchips" id="fchips" data-own></div>
    <div id="list" data-own></div>`;
}
function renderChips() {
  const p = P(), f = S.f, el = $('#fchips');
  if (!el) return;
  const cnt = (pred, except) => p.scripts.filter((s) => matches(s, except) && pred(s)).length;
  const chip = (attr, val, label, on, n, title = '') => `<button class="fchip ${on ? 'on' : ''} ${n === 0 && !on ? 'zero' : ''}" ${attr}="${esc(val)}" title="${esc(title)}" aria-pressed="${on}">${esc(label)}${n != null ? `<span class="n">${n}</span>` : ''}</button>`;
  const st = stages().map((s) => { const n = cnt((x) => x.stage === s.id, 'stage'); return n || f.stages.includes(s.id) ? chip('data-fstage', s.id, s.label, f.stages.includes(s.id), n, s.hint) : ''; }).join('');
  // подписи - knowledge/ui.json (filters): этап, показать, где
  const L = UI().filters || {};
  const only = ['fav', 'safe', 'running', 'failed']
    .map((k) => chip('data-fonly', k, (L.only?.[k] || {}).label || k, f.only === k, cnt(onlyTests[k], 'only'), (L.only?.[k] || {}).title || '')).join('');
  const wheres = [...new Set(p.scripts.map((s) => s.where))];
  const sources = [...new Set(p.scripts.map(srcKind))];
  const active = f.stages.length + (f.only ? 1 : 0) + (f.where ? 1 : 0) + (f.source ? 1 : 0) + (f.hideHooks ? 1 : 0) + (f.q ? 1 : 0);
  const row = (label, body) => (body.trim() ? `<div class="frow"><span class="flab">${esc(label)}</span><div class="fset">${body}</div></div>` : '');
  const hooks = p.scripts.some((s) => s.info?.hook) ? chip('data-fhooks', '1', L.hideHooks || 'Скрыть хуки', f.hideHooks, null, L.hideHooksHint || '') : '';
  patch(el, `${row(L.stage || 'Этап', st)}${row(L.show || 'Показать', only + hooks)}${row(L.where || 'Где', `
    ${wheres.length > 1 ? `<select data-fwhere aria-label="${esc(A('where'))}" title="${esc(L.whereHint || '')}"><option value="">${esc(L.allFolders || 'Все папки')}</option>${wheres.map((w) => `<option value="${esc(w)}" ${f.where === w ? 'selected' : ''}>${esc(w === '.' ? 'корень' : w)} (${cnt((s) => s.where === w, 'where')})</option>`).join('')}</select>` : ''}
    ${sources.length > 1 ? `<select data-fsource aria-label="${esc(A('source'))}" title="${esc(L.sourceHint || '')}"><option value="">${esc(L.allSources || 'Все источники')}</option>${sources.map((w) => `<option value="${esc(w)}" ${f.source === w ? 'selected' : ''}>${esc(w)} (${cnt((s) => srcKind(s) === w, 'source')})</option>`).join('')}</select>` : ''}
    ${active ? `<button class="btn ghost sm freset" data-act="reset-filters">${esc(L.reset || 'Сбросить')} (${active})</button>` : ''}`)}`);
}
const groupKey = (s) => (S.f.group === 'where' ? s.where : S.f.group === 'source' ? srcKind(s) : s.stage);
function renderList() {
  const p = P(), list = $('#list');
  if (!list) return;
  const items = p.scripts.filter((s) => matches(s));
  const cnt = $('#cnt'); if (cnt) cnt.textContent = items.length === p.scripts.length ? `${items.length} ${plural(items.length, 'скрипт', 'скрипта', 'скриптов')}` : `${items.length} из ${p.scripts.length}`;
  const LIMIT = S.showAll ? Infinity : 400;
  let shown = 0;
  const groups = new Map();
  if (S.f.group === 'stage') for (const st of stages()) groups.set(st.id, []);
  for (const s of items) { const k = groupKey(s); if (!groups.has(k)) groups.set(k, []); groups.get(k).push(s); }
  let html = '';
  for (const [k, rows] of groups) {
    if (!rows.length) continue;
    const st = S.f.group === 'stage' ? stageOf(k) : null;
    const label = st ? st.label : S.f.group === 'where' ? (k === '.' ? 'Корень проекта' : k) : k;
    const hint = st ? (learning() && st.learn ? st.learn : st.hint) : '';
    const fold = S.f.group + ':' + k;
    const closed = S.collapsed.has(fold) && !S.f.q;
    const part = rows.slice(0, Math.max(0, LIMIT - shown));
    shown += part.length;
    // заголовок группы - кнопка внутри h3: сворачивается и мышью, и с клавиатуры, а заголовок остаётся заголовком
    html += `<section class="group ${closed ? 'closed' : ''}"><h3><button type="button" class="fold" data-fold="${esc(fold)}" aria-expanded="${!closed}"><span class="car" aria-hidden="true">▼</span>${esc(label)}<span class="n">${rows.length}</span>${hint ? `<span class="gh">- ${esc(hint)}</span>` : ''}</button></h3><div class="rows">${part.map(rowHtml).join('')}</div></section>`;
    if (shown >= LIMIT) break;
  }
  if (items.length > shown) html += `<button class="btn more" data-act="showall">Показать все (${items.length})</button>`;
  patch(list, html || (p.scripts.length
    ? '<div class="nothing"><b>Ничего не подходит под фильтры</b>Измените поиск или <button class="tlink inl" data-act="reset-filters">сбросьте фильтры</button>.</div>'
    : '<div class="nothing"><b>Скриптов не нашлось</b>Пульт ищет package.json, Makefile, justfile, Taskfile, Cargo, Go, dotnet, Python, Maven/Gradle, Docker Compose и файлы .ps1/.sh/.bat в папках scripts и tools.</div>'));
}
function rowHtml(s) {
  const r = lastOf(s.id), fav = isFav(s.id);
  return `<div class="row ${S.detail === s.id ? 'sel' : ''}" tabindex="0" data-detail="${esc(s.id)}" data-sid="${esc(s.id)}">${runBtn(s)}
    <div class="rmain"><div class="rname"><span class="nm">${hl(s.name)}</span>${fxBadges(s, true)}${s.where !== '.' && S.f?.group !== 'where' ? `<span class="tag" title="Папка: ${esc(s.where)}">${esc(s.where)}</span>` : ''}${s.info?.hook ? `<span class="tag" title="Запускается сам ${esc(s.info.hook)}">хук</span>` : ''}</div>
    <div class="rdesc">${hl(s.info?.summary || s.cmd)}</div></div>
    <div class="rside">${lastHtml(r)}<button class="star ${fav ? 'on' : ''}" data-fav="${esc(s.id)}" title="${fav ? 'Убрать из быстрого запуска' : 'В быстрый запуск на «Обзоре»'}" aria-label="Избранное: ${esc(s.name)}" aria-pressed="${fav}">★</button></div></div>`;
}
function refreshScript(id) {
  const s = scriptOf(id); if (!s) return;
  for (const el of $$(`[data-sid="${CSS.escape(id)}"]`)) {
    const t = document.createElement('div');
    t.innerHTML = el.classList.contains('qtile') ? qtile(s) : rowHtml(s);
    dirty(el.parentElement);
    el.replaceWith(t.firstElementChild);
  }
  if (S.detail === id) renderDrawer();
}

// ---------- панель подробностей ----------
const FLAGS_SHOWN = 10;
// шапка файла написана под ширину редактора: перенос посреди фразы (следующая строка с маленькой буквы
// или прошлая кончилась запятой) - пробел; пустая строка - новый абзац; пункт списка («- », «1. ») и
// строка с большой буквы остаются с новой строки - это новая мысль, а не продолжение
function reflow(text) {
  return String(text || '').replace(/\r/g, '').split(/\n[ \t]*\n+/).map((p) => p.split('\n').reduce((acc, l) => {
    const t = l.trim();
    if (!acc) return t;
    if (!t) return acc;
    const cont = !/^([-*•]|\d+[.)])\s/.test(t) && (/^[a-zа-яё0-9(«"'`]/.test(t) || /[,(]$/.test(acc));
    return acc + (cont ? ' ' : '\n') + t;
  }, '')).filter(Boolean).join('\n\n');
}
function renderDrawer() {
  const d = $('#drawer');
  const s = S.detail && P() ? scriptOf(S.detail) : null;
  if (!s) {
    if (d._top) d._top.hidden = true;
    // панель уезжает анимацией; открыли другую за это время - она остаётся
    const hide = () => { if (S.detail && P() && scriptOf(S.detail)) return; d.hidden = true; patch(d, ''); };
    if (!d.hidden && !d.classList.contains('leaving')) closeWithAnim(d, hide); else if (d.hidden) hide();
    return;
  }
  d.classList.remove('leaving');
  d.hidden = false;
  const i = s.info || {}, r = lastOf(s.id), running = r?.status === 'running', miss = isMissing(s);
  const W = UI().will || {}, PARTS = UI().parts || {};
  const will = [['info', tpl(W.where, { where: where(s) })]];
  if (miss) will.push(['bad', tpl(W.missing, { tool: s.tool })]);
  for (const f of [...(i.fx || [])].sort((a, b) => FX_ORDER.indexOf(a) - FX_ORDER.indexOf(b))) will.push([fxInfo(f).level || 'info', fxInfo(f).say, learning() ? fxInfo(f).learn : '']);
  if (i.fxFromFile) will.push(['off', W.fromFile]);
  will.push(i.long ? ['info', W.long, learning() ? fxInfo('long').learn : ''] : ['ok', W.short]);
  if (s.danger && S.settings.confirmDanger) will.push(['warn', W.confirm]);
  if (!i.known) will.push(['warn', W.unknown]);
  will.push(['off', W.console]);
  const editRel = s.fileRel || (!s.virtual ? (s.where === '.' ? '' : s.where + '/') + (s.source === 'package.json' ? 'package.json' : s.source) : null);
  const src = s.fileRel ? `<code>${esc(s.fileRel)}</code>` : s.virtual ? `Стандартная команда: Пульт добавил её, потому что нашёл <code>${esc(s.source)}</code>${s.where !== '.' ? ` в ${esc(s.where)}` : ''}` : `<code>${esc(editRel)}</code>${s.line ? `, строка ${s.line}` : ''}`;
  const parts = (i.parts || []).map((p) => `<div class="ap k-${esc(p.kind)}"><code>${esc(p.text)}</code><span>${g(p.note || PARTS[p.kind] || '')}</span></div>`).join('');
  const flags = i.flags || [];
  const help = S.help[s.id];
  const helpFlags = help?.flags?.filter((f) => !S.helpQ || `${f.flag} ${f.text}`.toLowerCase().includes(S.helpQ.toLowerCase())) || [];
  const tps = (i.topics || []).filter(topicMeta);
  const howOpen = learning() ? 'open' : '';
  // длинный список ключей не прокручивается внутри панели: видны первые, остальные - по кнопке
  const all = (k) => S.flagsAll.has(s.id + ':' + k) || (k === 'help' && S.helpQ);
  const cut = (list, k) => (all(k) ? list : list.slice(0, FLAGS_SHOWN));
  const more = (list, k) => (!all(k) && list.length > FLAGS_SHOWN ? `<button class="btn ghost sm flmore" data-flags-all="${k}">${esc(tpl((UI().drawer || {}).flagsAll, { n: list.length }))}</button>` : '');
  patch(d, `<div class="dh"><div class="top"><h3 id="dh-title" tabindex="-1"><span class="sr">${esc(A('drawer'))}: </span>${esc(s.name)}</h3><button class="star ${isFav(s.id) ? 'on' : ''}" data-fav="${esc(s.id)}" title="Быстрый запуск" aria-label="Быстрый запуск: ${esc(s.name)}" aria-pressed="${isFav(s.id)}">★</button><button class="btn ghost icon" data-act="closedrawer" title="Закрыть (Esc)" aria-label="Закрыть">✕</button></div>
    <div class="meta"><span class="chip" title="${esc(stageOf(s.stage).hint)}">${esc(stageOf(s.stage).label)}</span><span class="chip">${esc(srcKind(s))}</span>${s.where !== '.' ? `<span class="chip">${esc(s.where)}</span>` : ''}${fxBadges(s)}</div></div>
    <div class="db">
      <section><h4>Что делает</h4><p class="lead2">${g(i.summary)}</p>${i.steps?.length ? `<ol class="steps">${i.steps.map((x) => `<li>${g(x)}</li>`).join('')}</ol>` : ''}${i.hook ? `<p class="note" style="margin-top:6px">${esc(tpl(W.hook, { hook: i.hook }))}</p>` : ''}
        ${i.about ? `<p class="note" style="margin-top:6px"><b>Программа:</b> ${g(i.about)}</p>` : ''}<p class="srcnote">${esc((UI().sources || {})[i.source || ''] || '')}</p></section>
      ${i.header && (i.header.split('\n')[0] !== i.summary || i.header.includes('\n')) ? `<section><h4>Описание от автора ${info('author')}</h4><div class="quote">${esc(reflow(i.header))}<small>из ${esc(i.file || 'файла скрипта')}</small></div></section>` : ''}
      ${parts ? `<section><h4>Команда по частям ${info('parts')}</h4><div class="anat">${parts}</div></section>` : ''}
      ${i.how?.length ? `<section><details ${howOpen}><summary><h4 style="display:inline">Что происходит внутри</h4></summary><ol class="how" style="margin-top:8px">${i.how.map((x) => `<li>${g(x)}</li>`).join('')}</ol></details></section>` : ''}
      <section><h4>Что произойдёт при запуске</h4><ul class="will">${will.map(([lv, t, l]) => `<li><span class="dot d-${esc(lv)}"></span><span>${g(t)}${l ? `<div class="learnbox">${g(l)}</div>` : ''}</span></li>`).join('')}</ul></section>
      <section><h4>Команда</h4><div class="cmdbox"><code>${esc(s.cmd)}</code><button data-copy="${esc(s.cmd)}" title="Скопировать в буфер">Копировать</button></div>
        ${s.source === 'package.json' && s.desc ? `<p class="note" style="margin-top:6px">В package.json записано: <code>${esc(s.desc)}</code></p>` : ''}</section>
      <section><h4>Запуск ${info('args')}</h4>
        <div class="argrow"><input type="text" id="args" data-args="${esc(s.id)}" aria-label="${esc(A('args'))}" spellcheck="false" placeholder="доп. аргументы, например --fast" value="${esc(S.argDraft[s.id] || '')}" autocomplete="off"><button class="btn ${running ? '' : 'primary'}" data-run-args="${esc(s.id)}" ${miss ? 'disabled' : ''}>${running ? 'Перезапустить' : '▶ Запустить'}</button></div>
        ${running ? `<div class="drow" style="margin-top:8px"><button class="btn sm danger" data-stop="${esc(r.id)}">■ Остановить</button><button class="btn sm" data-log="${esc(r.id)}">Показать вывод</button></div>` : ''}</section>
      <section><h4>Ключи ${info('flags')}</h4>
        ${flags.length ? `<div class="flaglist">${cut(flags, 'kb').map((f) => `<div class="fl ${f.used ? 'used' : ''}"><code>${esc(f.flag)}</code><span>${g(f.text || (f.from === 'file' ? 'ключ из кода скрипта' : ''))}${f.used ? ' · уже в команде' : ''}</span><button class="btn ghost" data-flag="${esc(f.flag.split(/[ ,=|]/)[0])}" title="Добавить в аргументы" aria-label="${esc(A('addFlag', { flag: f.flag.split(/[ ,=|]/)[0] }))}">+</button></div>`).join('')}</div>${more(flags, 'kb')}` : '<p class="note">Ключей в базе нет.</p>'}
        <div class="drow" style="margin-top:8px"><button class="btn sm" data-ask="${esc(s.id)}" ${help === 'loading' ? 'disabled' : ''}>${help === 'loading' ? '<span class="spin"></span> Спрашиваю…' : 'Спросить у программы (--help)'}</button></div>
        ${help && help !== 'loading' ? (help.error ? `<p class="err" style="margin-top:8px">${esc(help.error)}</p>` : `<p class="note" style="margin-top:8px">${esc(help.program)}: найдено ключей - ${help.flags.length}${help.about ? `. ${esc(help.about)}` : ''}</p>
          ${help.flags.length > 8 ? `<input type="search" id="helpq" aria-label="${esc(A('helpQ'))}" spellcheck="false" placeholder="найти ключ…" value="${esc(S.helpQ)}" style="width:100%;margin:6px 0">` : ''}
          <div class="flaglist">${cut(helpFlags.slice(0, 200), 'help').map((f) => `<div class="fl"><code>${esc(f.flag)}</code><span>${esc(f.text)}</span><button class="btn ghost" data-flag="${esc(f.flag.replace(/^-\w,\s*/, '').split(/[ ,=|<[]/)[0])}" title="Добавить в аргументы" aria-label="${esc(A('addFlag', { flag: f.flag.replace(/^-\w,\s*/, '').split(/[ ,=|<[]/)[0] }))}">+</button></div>`).join('') || '<div class="fl"><span>ничего</span></div>'}</div>${more(helpFlags.slice(0, 200), 'help')}
          <details><summary class="note">вся справка программы</summary><pre class="helptext">${esc(help.text)}</pre></details>`) : ''}</section>
      ${tps.length ? `<section><h4>Чему научиться ${info('learnLinks')}</h4><div class="topiclinks">${tps.map((t) => `<button class="chip linkchip" data-topic="${esc(t)}">${esc(topicMeta(t).title)}</button>`).join('')}</div></section>` : ''}
      <section><h4>Где объявлен</h4><p style="font-size:13px">${src}</p>
        <div class="drow" style="margin-top:8px">${editRel ? `<button class="btn sm" data-edit="${esc(editRel)}" ${s.line ? `data-line="${s.line}"` : ''}>Открыть в редакторе</button><button class="btn sm" data-reveal="${esc(editRel)}">Показать в проводнике</button>` : ''}${i.file && i.file !== s.fileRel ? `<button class="btn sm" data-edit="${esc(i.file)}">Открыть ${esc(i.file.split('/').pop())}</button>` : ''}</div></section>
      <section><h4>Запуски этого скрипта</h4>${scriptRuns(s.id).length ? `<div class="list">${scriptRuns(s.id).map(histLi).join('')}</div>` : '<p class="note">Ещё не запускался.</p>'}</section>
    </div>`);
}
function scriptRuns(id) {
  const cur = [...S.runs.values()].filter((r) => r.scriptId === id && r.kind === 'script');
  const ids = new Set(cur.map((r) => r.id));
  return [...cur, ...S.hist.filter((h) => h.scriptId === id && !ids.has(h.id))].sort((a, b) => b.startedAt - a.startedAt).slice(0, 6);
}
// kbd - открыли с клавиатуры (Enter на строке): фокус уходит в панель на её заголовок, закрытие вернёт его на строку
function openDetail(id, kbd = false) {
  const prev = S.detail;
  S.detail = id; S.helpQ = '';
  if (prev !== id) S.flagsAll.clear();
  $$('.row.sel').forEach((el) => { el.classList.remove('sel'); dirty(el); });
  $$(`.row[data-sid="${CSS.escape(id)}"]`).forEach((el) => { el.classList.add('sel'); dirty(el); });
  renderDrawer();
  if (prev !== id) $('#drawer').scrollTop = 0;
  if (kbd) $('#dh-title')?.focus({ preventScroll: true });
}
function closeDetail() {
  const id = S.detail, d = $('#drawer'), inside = d.contains(document.activeElement) || document.activeElement === document.body;
  S.detail = null;
  $$('.row.sel').forEach((x) => { x.classList.remove('sel'); dirty(x); });
  renderDrawer();
  if (id && inside) $(`#main [data-sid="${CSS.escape(id)}"]`)?.focus({ preventScroll: false });
}
// программу из папки проекта ядро без согласия не запускает: возвращает её путь в confirm
async function askHelp(id, confirmed = null) {
  S.help[id] = 'loading'; renderDrawer();
  try { S.help[id] = await call('help', { scriptId: id, confirmed }); } catch (e) { S.help[id] = { error: String(e) }; }
  const h = S.help[id];
  if (h?.confirm) {
    const a = UI().askHelp || {};
    const ok = await confirmBox({ title: a.title, danger: true, ok: a.ok,
      body: `<p style="margin:0 0 8px">${esc(tpl(a.body, { path: h.confirm }))}</p><p class="note">${esc(a.note || '')}</p>` });
    if (ok) return askHelp(id, h.confirm);
    delete S.help[id];
  }
  if (S.detail === id) renderDrawer();
}
async function trustRepo() {
  try { const git = await call('trust_repo', { path: P().path }); P().git = git; renderMain(); } catch (e) { toast(String(e), 'bad'); }
}

// ---------- запуск ----------
// опасный скрипт (выкладка, удаление) спрашивает согласия при каждом пути запуска: ▶, «Повторить»,
// история и «Перезапустить» в панели подробностей
async function askDanger(s, args) {
  if (!s.danger || !S.settings.confirmDanger) return true;
  return !!(await confirmBox({ title: `Запустить «${s.name}»?`, danger: true, ok: 'Запустить',
    body: `<p style="margin:0 0 8px">${esc(s.info?.summary || '')}</p><div class="cmdbox"><code>${esc(s.cmd)}${args ? ' ' + esc(args) : ''}</code></div><p class="note" style="margin-top:8px">${esc(fxInfo('danger').say)}.</p>` }));
}
async function runScript(id, { args, force } = {}) {
  const s = scriptOf(id); if (!s) return;
  if (isMissing(s)) { toast(`Нет программы «${s.tool}» - скрипт не запустится`, 'bad'); return; }
  if (!force && !(await askDanger(s, args))) return;
  try {
    const run = await call('run', { scriptId: id, args: args || '', force: !!force });
    addRun(run, true);
  } catch (e) {
    const m = /^RUNNING:(\w+)/.exec(String(e));
    if (m) {
      const v = await confirmBox({ title: `«${s.name}» уже запущен`, body: '<p>Можно перезапустить его с новыми аргументами или запустить ещё один экземпляр рядом.</p>', ok: 'Перезапустить', alt: 'Ещё один', cancel: 'Показать текущий' });
      if (v === 'ok') { try { addRun(await call('restart', { runId: m[1], args: args ?? null }), true); } catch (x) { toast(String(x), 'bad'); } }
      else if (v === 'alt') runScript(id, { args, force: true });
      else activate(m[1]);
    } else toast(String(e), 'bad');
  }
}
async function stopRun(runId) { try { await call('stop', { runId }); } catch (e) { toast(String(e), 'bad'); } }
// крестик вкладки: завершённую закрывает сразу, идущую - после согласия останавливает всё дерево процессов.
// Вкладка уходит сразу; итог запуска (событие run) всё равно попадает в историю - см. onRun, closedRuns
const closedRuns = new Map();
async function closeRunTab(id) {
  const r = S.runs.get(id); if (!r) return;
  if (r.status === 'running' && r.kind === 'script') {
    const T = UI().run || {};
    if (!(await confirmBox({ title: tpl(T.closeTitle, { name: r.title }), body: `<p>${esc(T.closeBody)}</p><div class="cmdbox"><code>${esc(r.cmd)}</code></div>`, ok: T.closeOk, danger: true }))) return;
    if (S.runs.get(id)?.status === 'running') { closedRuns.set(id, r); S.runs.delete(id); renderConsole(); await stopRun(id); return; }
  }
  S.runs.delete(id);
  if (!id.includes(':')) call('forget', { runId: id }).catch(() => {});
  renderConsole();
}
// кнопка ▶ в строке: идёт - остановить, иначе запустить
function toggleRun(id) { const r = lastOf(id); if (r?.status === 'running') stopRun(r.id); else runScript(id); }

// ---------- Документация ----------
function viewLearn() {
  const found = P().topics || [];
  const ids = new Set(found.map((t) => t.id));
  const all = Object.values(K().topics || {});
  const others = all.filter((t) => !ids.has(t.id));
  if (!S.topic || (!ids.has(S.topic) && !S.allTopics && !others.some((t) => t.id === S.topic))) S.topic = found[0]?.id || all[0]?.id || null;
  const card = (t, why) => `<button class="tcard ${S.topic === t.id ? 'on' : ''} ${why ? '' : 'off'}" ${S.topic === t.id ? 'aria-current="true"' : ''} data-topic="${esc(t.id)}"><span class="tico" aria-hidden="true" style="background:${esc(t.color)}">${Learn.icon(t.icon, 17, Learn.ink(t.color))}</span><span class="tx"><b>${esc(t.title)}</b><small>${esc(why || t.summary)}</small></span></button>`;
  return `${hintBox('learn')}<div class="learn">
    <nav class="tlist" aria-label="Уроки"><h4>В этом проекте · ${found.length}</h4>${found.map((t) => card(t, t.why)).join('') || '<p class="note">Ничего не определилось.</p>'}
      ${others.length ? `<button class="tlink tmore" data-act="all-topics" aria-expanded="${S.allTopics}">${S.allTopics ? 'Скрыть' : 'Показать'} остальные уроки · ${others.length}</button>${S.allTopics ? others.map((t) => card(t, '')).join('') : ''}` : ''}</nav>
    <div id="lessonbody" data-own><div class="note"><span class="spin"></span> Открываю урок…</div></div></div>`;
}
async function ensureTopic() {
  const id = S.topic, el = $('#lessonbody');
  if (!id || !el) return;
  let t = S.topics.get(id);
  if (!t) { try { t = await call('topic', { id }); S.topics.set(id, t); } catch (e) { patch(el, `<p class="err">${esc(String(e))}</p>`); return; } }
  if (S.topic !== id || !$('#lessonbody')) return;
  const why = (P().topics || []).find((x) => x.id === id)?.why;
  const lb = $('#lessonbody'), switched = lb._topic && lb._topic !== id;
  lb._topic = id;
  patch(lb, Learn.topicHtml(t, { learn: learning(), why, topicTitle: (r) => topicMeta(r)?.title, projectRows: (tid) => projectRowsFor(tid) }));
  if (switched) enterAnim(lb.firstElementChild, 6);
}
function projectRowsFor(tid) {
  const rows = P().scripts.filter((s) => (s.info?.topics || []).includes(tid)).slice(0, 30);
  return rows.length ? `<div class="rows">${rows.map(rowHtml).join('')}</div>` : '<p class="note">В скриптах проекта эта технология напрямую не вызывается.</p>';
}
function openTopic(id) {
  S.topic = id;
  if (!(P()?.topics || []).some((t) => t.id === id)) S.allTopics = true;
  if (S.tab !== 'learn') setTab('learn'); else { renderTab(); }
  $('#main').scrollTop = 0;
}

// ---------- Пайплайны ----------
const remoteKey = (r) => `${r.name}|${S.ci.scope === 'branch' ? P().git?.branch : ''}`;
function ciLastStatus() {
  const rem = remotesForCI()[0]; if (!rem) return null;
  return (S.ci.data[`${rem.name}|`] || S.ci.data[remoteKey(rem)])?.runs?.[0]?.status || null;
}
const refLabel = (r) => String(r || '').replace(/^refs\/merge-requests\/(\d+)\/head$/, 'MR !$1').replace(/^refs\/heads\//, '');
const FINAL = ['success', 'failed', 'canceled', 'skipped'];
function viewCI() {
  const p = P(), rem = remotesForCI();
  let html = hintBox('ci') + '<div class="blk-stack">';
  if (!rem.length) html += Blocks.card({ title: SEC('ciLive'), body: '<p class="blk-empty">У репозитория нет адреса на GitLab или GitHub - живой статус показывать неоткуда. Настройка из файлов - ниже.</p>' });
  else {
    const cur = rem.find((r) => r.name === S.ci.remote) || rem[0];
    S.ci.remote = cur.name;
    const st = S.ci.data[remoteKey(cur)] || {};
    const label = (r) => (/gitlab/i.test(r.host) ? 'GitLab' : 'GitHub');
    const bar = `<div class="ci-bar blk-tools">
      ${rem.length > 1 ? `<div class="seg">${rem.map((r) => `<button class="${r.name === cur.name ? 'on' : ''}" aria-pressed="${r.name === cur.name}" data-ci-remote="${esc(r.name)}" title="${esc(r.url)}">${label(r)} · ${esc(r.slug)}</button>`).join('')}</div>` : `<b>${label(cur)} · <span class="mono">${esc(cur.slug)}</span></b>`}
      ${p.git?.branch ? `<div class="seg"><button class="${S.ci.scope === 'all' ? 'on' : ''}" aria-pressed="${S.ci.scope === 'all'}" data-ci-scope="all">Все ветки</button><button class="${S.ci.scope === 'branch' ? 'on' : ''}" aria-pressed="${S.ci.scope === 'branch'}" data-ci-scope="branch">Ветка ${esc(p.git.branch)}</button></div>` : ''}
      <div class="seg">${[['', 'Все'], ['running', 'Идут'], ['failed', 'Ошибки'], ['success', 'Успешные']].map(([k, l]) => `<button class="${S.ci.status === k ? 'on' : ''}" aria-pressed="${S.ci.status === k}" data-ci-status="${k}">${l}</button>`).join('')}</div>
      <span class="grow"></span><span class="note">${st.at ? `${st.cached ? 'из кэша · ' : ''}данные на ${clock(st.at)}` : ''}</span>
      <button class="btn sm" data-act="ciload" ${st.loading ? 'disabled' : ''}>↻ Обновить</button></div>`;
    const live = cur.trusted ? liveBody(st) : { list: `<div class="blk-in-pad"><div class="err">Сервер «${esc(cur.host)}» не в списке доверенных: Пульт не отправит ему запросы с вашим токеном. Если это ваш сервер CI - добавьте его в настройках (⚙).</div></div>`, pipe: '' };
    html += Blocks.card({ title: SEC('ciLive'), flush: true, body: bar + live.list });
    html += live.pipe;
  }
  html += Blocks.card({ title: SEC('ciConfig'), info: info('ciConfig'), flush: p.ci.length > 0, body: p.ci.length ? p.ci.map(ciConfigHtml).join('') : '<p class="blk-empty">Файлов CI не найдено (.gitlab-ci.yml, .github/workflows, Jenkinsfile…).</p>' });
  return html + '</div>';
}
const live = (x) => x.status === 'running' || x.status === 'pending';
const runStart = (x) => isoMs(x.startedAt) ?? isoMs(x.createdAt);
// живой статус: list - содержимое карточки «Пайплайны на сервере» (во всю её ширину), pipe - карточка выбранного
// пайплайна. Ошибки и пустые состояния - с полем карточки (.blk-in-pad), список - без своей рамки
function liveBody(st) {
  const pad = (x) => `<div class="blk-in-pad">${x}</div>`;
  if (st.loading && !st.runs) return { list: `<div class="runs">${'<div class="pr"><span class="skel" style="grid-column:1/-1;margin:2px 0"></span></div>'.repeat(4)}</div>`, pipe: '' };
  let out = '', pipe = '';
  if (st.error) {
    const tool = /glab/.test(st.error) ? 'glab' : /gh/.test(st.error) ? 'gh' : null;
    out += pad(`<div class="err">${esc(st.error)}${tool && /auth login/.test(st.error) ? `<div class="cmdbox" style="margin-top:8px"><code>${tool} auth login</code><button data-copy="${tool} auth login">Копировать</button></div>` : ''}</div>${st.runs ? `<p class="warnline">Показаны прежние данные - от ${clock(st.at)}.</p>` : ''}`);
  }
  if (!st.runs) return { list: out || '<p class="blk-empty">Статус ещё не загружен. <button class="btn sm" data-act="ciload">Загрузить</button></p>', pipe };
  if (!st.runs.length) return { list: out + `<p class="blk-empty">Пайплайнов ${S.ci.scope === 'branch' ? 'по этой ветке ' : ''}пока нет.</p>`, pipe };
  const runs = st.runs.filter((x) => !S.ci.status || x.status === S.ci.status || (S.ci.status === 'running' && x.status === 'pending'));
  out += runs.length ? `<div class="runs">${runs.map((x) => `<div class="pr ${st.sel === x.id ? 'sel' : ''}" tabindex="0" aria-current="${st.sel === x.id}" data-pipe="${esc(x.id)}" title="${esc(x.title || '')}">
      <span class="dot s-${esc(x.status)}" role="img" aria-label="${esc(A('status', { s: RU[x.status] || x.status }))}" title="${esc(RU[x.status] || x.status)}"></span><span class="id">#${esc(x.id)}</span>
      ${live(x) ? etaBar(runStart(x), pipeEta(st.runs, x), 'under narrow-only') : ''}<span class="rf">${esc(x.name ? x.name + ' · ' : '')}<b>${esc(refLabel(x.ref))}</b> <span class="mono faint">${esc(x.sha)}</span> ${x.title ? `<span class="faint">${esc(x.title)}</span>` : ''}</span>
      <span class="meta">${live(x) ? etaLabel(runStart(x), pipeEta(st.runs, x)) + etaBar(runStart(x), pipeEta(st.runs, x), 'inl') + ' · ' : ''}${esc(RU[x.status] || x.status)} · ${esc(ago(x.createdAt))} <button class="btn ghost sm" data-url="${esc(x.url)}" title="Открыть в браузере" aria-label="Открыть пайплайн #${esc(x.id)} в браузере">↗</button></span></div>`).join('')}</div>`
    : `<p class="blk-empty">Под фильтр «${esc(S.ci.status)}» ничего не попало.</p>`;
  if (st.sel) {
    const sel = st.runs.find((x) => x.id === st.sel);
    const title = SEC('ciPipe', { id: st.sel });
    if (st.jobsLoading && !st.jobs) pipe = Blocks.card({ title, body: `<p class="note"><span class="spin"></span> Загружаю этапы пайплайна #${esc(st.sel)}…</p>` });
    else if (st.jobsError) pipe = Blocks.card({ title, body: `<div class="err">${esc(st.jobsError)}</div>` });
    else if (st.jobs) {
      const all = st.jobs.flatMap((x) => x.jobs);
      const done = all.filter((j) => FINAL.includes(j.status)).length;
      const failed = all.filter((j) => j.status === 'failed' && !j.allowFailure);
      const now = st.jobs.find((x) => x.jobs.some((j) => j.status === 'running' || j.status === 'pending'));
      pipe = Blocks.card({ title, aside: `<span class="note">${esc(refLabel(sel?.ref))} · ${esc(RU[sel?.status] || '')}</span>${st.jobsLoading ? '<span class="spin"></span>' : ''}`, body: `<div class="summary-line">${now ? `<span>Сейчас: этап <b>${esc(now.name)}</b></span>` : ''}<span>Готово <b>${done}</b> из ${all.length}</span>${failed.length ? `<span style="color:var(--bad)">Упали: <b>${failed.map((j) => esc(j.name)).join(', ')}</b></span>` : ''}<span class="faint">Клик по джобе - её журнал в консоли</span></div>
        <div class="pipe">${st.jobs.map((x) => `<div class="scol ${now === x ? 'now' : ''}"><div class="sh">${esc(x.name)}</div>${x.jobs.map((j) => `<button class="job ${j.status === 'manual' ? 'man' : ''}" data-job="${esc(j.id)}" data-jname="${esc(j.name)}" data-jstatus="${esc(j.status)}" data-jurl="${esc(j.url || '')}" title="${esc(RU[j.status] || j.status)}${j.step ? ' · шаг: ' + esc(j.step) : ''}${j.failedStep ? ' · упал шаг: ' + esc(j.failedStep) : ''}${j.failureReason ? ' · причина: ' + esc(j.failureReason) : ''}${j.allowFailure ? ' · падение допустимо' : ''}${j.runner ? ' · раннер: ' + esc(j.runner) : ''}">${sdot(j.status)}<span class="jn">${esc(j.name)}</span><span class="dur">${j.duration != null ? esc(dur(j.duration * 1000)) : j.status === 'running' ? etaLabel(isoMs(j.startedAt), jobEta(j)) : ''}</span>${j.status === 'running' ? etaBar(isoMs(j.startedAt), jobEta(j), 'under') : ''}</button>`).join('')}</div>`).join('')}</div>` });
    }
  }
  return { list: out, pipe };
}
function ciConfigHtml(c) {
  if (c.kind === 'gitlab' || c.kind === 'jenkins') {
    const cols = (c.stages || []).map((st) => `<div class="scol"><div class="sh">${esc(st.name)} <span class="sid">${esc(stageOf(st.sid).label.toLowerCase())}</span></div>${st.jobs.length ? st.jobs.map((j) => {
      const tags = [j.manual && 'вручную', j.allowFailure && 'может упасть', j.onlyMain && 'только main', j.onlyMr && 'в MR', j.needs?.length && `после ${j.needs.length}`].filter(Boolean);
      return `<div class="job cfg ${j.manual ? 'man' : ''}"><span class="jn">${esc(j.name)}</span>${tags.length ? `<span class="jtags">${tags.map((t) => `<span class="jtag">${esc(t)}</span>`).join('')}</span>` : ''}${j.desc ? `<span class="jd" data-ext>${esc(j.desc)}</span>` : ''}</div>`;
    }).join('') : '<div class="note">- пусто</div>'}</div>`).join('');
    return `<section class="cfgsec"><h4>${c.kind === 'gitlab' ? 'GitLab CI' : 'Jenkins'} <small>${esc(c.file)} · ${c.jobCount} ${plural(c.jobCount, 'джоба', 'джобы', 'джоб')}${c.includes?.length ? ` · подключает ${c.includes.length} ${plural(c.includes.length, 'файл', 'файла', 'файлов')}` : ''}</small><button class="btn ghost sm" data-edit="${esc(c.file)}" title="Открыть в редакторе" aria-label="Открыть в редакторе: ${esc(c.file)}">✎</button></h4><div class="pipe">${cols || '<span class="note">Этапы не распознаны</span>'}</div></section>`;
  }
  if (c.kind === 'github') {
    return `<section class="cfgsec"><h4>${esc(c.name)} <small>${esc(c.file)}</small><button class="btn ghost sm" data-edit="${esc(c.file)}" title="Открыть в редакторе" aria-label="Открыть в редакторе: ${esc(c.file)}">✎</button></h4>
      <p class="note" style="margin:-4px 0 8px">Запускается: ${(c.triggers || []).map((t) => `<span class="jtag">${esc(t)}</span>`).join(' ') || '-'}</p>
      <div class="pipe"><div class="scol" style="max-width:none;flex:1">${(c.jobs || []).map((j) => `<div class="job cfg"><span class="jn">${esc(j.name)}</span>${j.needs.length ? `<span class="jtags"><span class="jtag">после ${esc(j.needs.join(', '))}</span></span>` : ''}</div>`).join('') || '<span class="note">Джоб нет</span>'}</div></div></section>`;
  }
  return `<section class="cfgsec"><h4>${esc(c.file)} <small>найден, подробный разбор пока не поддерживается</small><button class="btn ghost sm" data-edit="${esc(c.file)}" title="Открыть в редакторе" aria-label="Открыть в редакторе: ${esc(c.file)}">✎</button></h4></section>`;
}
function ensureCI() {
  if (!S.settings.ciAutoload) return;
  const r = remotesForCI().find((x) => x.name === S.ci.remote) || remotesForCI()[0];
  if (r?.trusted && !S.ci.data[remoteKey(r)]) loadCI(r);
}
// каждый ответ CI проверяется: тот ли проект и та ли выборка, что при запросе (переключились - выбрасываем)
async function loadCI(r, force = false) {
  const path = P().path, k = remoteKey(r), gitRef = S.ci.scope === 'branch' ? P().git?.branch : '';
  const st = S.ci.data[k] = { ...(S.ci.data[k] || {}), loading: true, error: null };
  if (S.tab === 'ci') renderTab();
  let res;
  try { res = await call('ci_runs', { path, remote: r.name, gitRef, force }); } catch (e) { res = { error: String(e) }; }
  if (P()?.path !== path || S.ci.data[k] !== st) return;
  st.loading = false;
  if (res.data) { st.runs = res.data; st.at = res.at; st.cached = res.cached; }
  st.error = res.error || null;
  const keep = st.runs?.find((x) => x.id === st.sel) || st.runs?.[0];
  if (keep) await loadJobs(r, keep.id, force && keep.id === st.sel);
  if (S.tab === 'ci') renderTab(); else if (S.tab === 'overview') renderMain();
  renderTop();
}
async function loadJobs(r, id, force = false) {
  const path = P().path, st = S.ci.data[remoteKey(r)];
  if (!st) return;
  if (st.sel !== id) st.jobs = null;
  st.sel = id; st.jobsError = null; st.jobsLoading = true;
  if (S.tab === 'ci') renderTab();
  const run = st.runs?.find((x) => x.id === id);
  let res;
  try { res = await call('ci_jobs', { path, remote: r.name, id, done: FINAL.includes(run?.status), force }); } catch (e) { res = { error: String(e) }; }
  if (P()?.path !== path || st.sel !== id) return;
  st.jobsLoading = false;
  if (res.data) { st.jobs = res.data; rememberJobs(res.data); }
  st.jobsError = res.error && !res.data ? res.error : null;
  if (S.tab === 'ci') renderTab();
}
async function openCiLog(r, job) {
  const id = 'ci:' + job.id, path = P().path;
  const run = { id, kind: 'ci', title: `CI · ${job.name}`, cmd: `${/gitlab/i.test(r.host) ? 'GitLab' : 'GitHub'} · ${r.slug} · джоба #${job.id}`, where: r.slug, status: job.status === 'running' || job.status === 'pending' ? 'running' : job.status === 'success' ? 'ok' : job.status === 'failed' ? 'fail' : 'stopped', startedAt: Date.now(), url: job.url, remote: r.name, job, project: path };
  run.term = makeTerm(); termSys(run, `Загружаю журнал джобы ${job.name}…`);
  S.runs.set(id, run);
  activate(id);
  let res;
  try { res = await call('ci_log', { path, remote: r.name, job: Number(job.id) }); } catch (e) { res = { error: String(e) }; }
  if (!S.runs.has(id)) return;
  run.term = makeTerm();
  if (res.error) termSys(run, 'Не удалось загрузить журнал: ' + res.error);
  else { termWrite(run.term, (res.data || '(журнал пуст)') + '\n'); termSys(run, FINAL.includes(job.status) ? 'Журнал полный' : `Джоба ещё идёт - журнал на ${clock(res.at)}. «Повторить» загрузит свежий`); }
  run.endedAt = Date.now();
  if (S.active === id) renderConsole(true);
}

// ---------- Git ----------
// заголовок раздела вкладки - knowledge/ui.json, раздел sections
const SEC = (k, v) => tpl((UI().sections || {})[k] || k, v);
function viewGit() {
  const gi = P().git;
  if (!gi) return '<div class="empty-card">Эта папка не в git-репозитории.</div>';
  const gq = S.gitQ.toLowerCase();
  const files = gi.files.filter((f) => !gq || f.path.toLowerCase().includes(gq));
  const groups = {};
  for (const f of files) (groups[f.label] ||= []).push(f);
  const actions = P().scripts.filter((s) => s.source === 'git');
  // недоверенный репозиторий: git не запускался - счётчики, файлы и коммиты неизвестны, а не нулевые
  const GT = UI().gitTab || {}, off = !!gi.untrusted;
  const why = off ? tpl((UI().health?.gitWhy || {})[gi.why] || '', { key: gi.whyKey || '' }) : '';
  const unk = `<b class="faint stat-unk">${esc(GT.unknown || '-')}</b>`;
  const stat = (label, val, title = '') => `<div class="stat"${title ? ` title="${esc(title)}"` : ''}><small>${esc(label)}</small>${val}</div>`;
  const notRun = `<div class="blk-state"><b>${esc(gi.refused ? tpl(GT.refused, { why }) : GT.notRun)}</b>${gi.refused ? '' : `<p>${esc(tpl(GT.notRunText, { why }))}</p><button class="btn sm" data-act="git-trust">${esc(GT.trust)}</button>`}</div>`;
  const fileList = gi.files.length
    ? Object.entries(groups).map(([label, list]) => `<div class="list">${list.slice(0, 300).map((f) => `<div class="li click" tabindex="0" data-edit="${esc(f.path)}" title="Открыть в редакторе"><span class="gcode g-${esc(f.code)}">${esc(f.code === '?' ? '+' : f.code)}</span><span class="t mono" style="font-size:12.5px">${esc(f.path)}</span><span class="d">${esc(label)}${f.staged ? ' · в индексе' : ''}</span><button class="btn ghost sm" data-reveal="${esc(f.path)}" title="Показать в проводнике" aria-label="Показать в проводнике: ${esc(f.path)}">⌕</button></div>`).join('')}</div>`).join('') || `<p class="blk-empty">${esc(A('gitNoMatch'))}</p>`
    : '<p class="blk-empty">Всё закоммичено.</p>';
  const commits = `<div class="list">${gi.commits.map((c) => `<div class="li"><span class="h">${esc(c.hash)}</span><span class="t" title="${esc(c.subject)}">${esc(c.subject)} ${c.refs.map((r) => `<span class="ref">${esc(r)}</span>`).join(' ')}</span><span class="d">${esc(c.author)} · ${esc(ago(c.date))}</span></div>`).join('') || '<div class="li note">Коммитов нет</div>'}</div>`;
  const remotes = `<div class="list">${gi.remotes.map((r) => `<div class="li"><b class="mono" style="font-size:12.5px">${esc(r.name)}</b><span class="t mono faint" style="font-size:12px">${esc(r.url)}</span>${r.dup ? '<span class="d">дубль</span>' : ''}${r.trusted ? '' : '<span class="d" title="CI-запросы к этому серверу не отправляются">не доверенный</span>'}</div>`).join('') || '<div class="li note">Нет адресов - репозиторий только локальный</div>'}</div>`;
  return `${hintBox('git')}
    <div class="stats">
      ${stat('Ветка', `<b>${esc(gi.branch)}</b>`)}
      ${stat('На сервере', off ? unk : `<b>${esc(gi.upstream || '-')}</b>`, 'Ветка на сервере, с которой сравнивается ваша')}
      ${stat('Не отправлено / не забрано', off ? unk : `<b>↑${gi.ahead} ↓${gi.behind}</b>`)}
      ${stat('Изменено файлов', off ? unk : `<b>${gi.dirty}</b>`)}
      ${stat('Отложено', off ? unk : `<b>${gi.stash}</b>`, 'git stash')}
      ${stat('Последний fetch', `<b style="font-size:14px">${esc(gi.fetchedAt ? ago(gi.fetchedAt) : 'никогда')}</b>`)}
      ${gi.tag ? stat('Последний тег', `<b>${esc(gi.tag)}</b>`) : ''}
    </div>
    <div class="blk-stack">
    ${actions.length ? Blocks.card({ title: SEC('gitActions'), flush: true, body: `<div class="rows">${actions.map(rowHtml).join('')}</div>` }) : ''}
    ${off ? Blocks.card({ title: SEC('gitFiles'), body: notRun }) : Blocks.card({ title: SEC('gitFiles'), flush: files.length > 0,
      aside: gi.files.length > 8 ? `<input type="search" id="gq" placeholder="фильтр…" value="${esc(S.gitQ)}" class="gq" aria-label="${esc(A('gitQ') || 'Фильтр файлов')}" autocomplete="off" spellcheck="false">` : '',
      body: fileList })}
    <div class="blk-pair">
      ${Blocks.card({ title: SEC('gitCommits'), flush: !off, body: off ? `<p class="blk-empty">${esc(GT.notRun)}</p>` : commits })}
      ${Blocks.card({ title: SEC('gitRemotes'), flush: true, body: remotes })}
    </div>
    </div>`;
}

// ---------- История ----------
function viewHistory() {
  const hf = S.histFilter;
  const all = recentRuns(1000);
  const list = all.filter((r) => (!hf.status || r.status === hf.status) && (!hf.q || `${r.title} ${r.args || ''} ${r.cmd}`.toLowerCase().includes(hf.q.toLowerCase())));
  const n = (st) => all.filter((r) => r.status === st).length;
  return Blocks.card({ title: SEC('history'), flush: true, body: `<div class="blk-tools toolbar">
      <div class="seg">${[['', `Все ${all.length}`], ['ok', `Успех ${n('ok')}`], ['fail', `Ошибка ${n('fail')}`], ['stopped', `Остановлены ${n('stopped')}`], ['running', `Идут ${n('running')}`]].map(([k, l]) => `<button class="${hf.status === k ? 'on' : ''}" aria-pressed="${hf.status === k}" data-hstatus="${k}">${l}</button>`).join('')}</div>
      <div class="search"><input type="search" id="hq" aria-label="${esc(A('histQ'))}" placeholder="Найти запуск…" value="${esc(hf.q)}" autocomplete="off"></div>
      <span class="grow"></span>${all.length ? '<button class="btn sm" data-act="clear-history">Очистить историю</button>' : ''}</div>
    ${list.length ? `<div class="list htable">${list.slice(0, 400).map((r) => `<div class="li click" tabindex="0" data-log="${esc(r.id)}" title="Показать вывод (у старых - последние 80 строк)">
      ${sdot(r.status)}<span class="t mono">${esc(r.title)}</span><span class="t faint mono" style="font-size:12px">${esc(r.args || '')}</span>
      <span class="d">${esc(new Date(r.startedAt).toLocaleString('ru-RU', { day: '2-digit', month: '2-digit', hour: '2-digit', minute: '2-digit' }))}</span><span class="d">${r.status === 'running' ? 'идёт' : esc(dur((r.endedAt || Date.now()) - r.startedAt))}</span><span class="d">${r.code != null && r.status !== 'stopped' ? 'код ' + esc(r.code) : ''}</span>
      ${scriptOf(r.scriptId) ? `<button class="btn ghost sm" data-rerun="${esc(r.scriptId)}" data-rargs="${esc(r.args || '')}" title="Запустить снова с теми же аргументами" aria-label="Запустить снова: ${esc(r.title)}">▶</button>` : '<span></span>'}</div>`).join('')}</div>`
    : `<div class="nothing blk-nothing"><b>${all.length ? 'Ничего не подходит' : 'История пуста'}</b>${all.length ? 'Измените фильтр.' : 'Здесь появится каждый запуск: когда, сколько шёл и чем закончился.'}</div>`}` });
}

// =====================================================================
// КОНСОЛЬ: ANSI-цвета, \r, перерисовка строк, поиск, ввод
// =====================================================================
const MAX_LINES = 20000, MAX_DOM = 4000;
function makeTerm() { return { lines: [], plain: [], kinds: [], cur: [], st: {}, cr: false, dirty: 0, urls: new Set(), errs: 0, carry: '' }; }
function styleKey(st) {
  const cls = [], sty = [];
  if (st.b) cls.push('a-b'); if (st.d) cls.push('a-d'); if (st.i) cls.push('a-i'); if (st.u) cls.push('a-u'); if (st.s) cls.push('a-s');
  let fg = st.fg, bg = st.bg;
  if (st.inv) [fg, bg] = [bg ?? 0, fg ?? 7];
  for (const [v, p] of [[fg, 'f'], [bg, 'b']]) {
    if (v == null) continue;
    if (typeof v === 'number') cls.push(p + v); else sty.push(`${p === 'f' ? 'color' : 'background'}:${v}`);
  }
  return cls.length || sty.length ? `<span${cls.length ? ` class="${cls.join(' ')}"` : ''}${sty.length ? ` style="${sty.join(';')}"` : ''}>` : '';
}
function c256(n) {
  if (n < 16) return n;
  if (n < 232) { n -= 16; const v = (x) => (x ? x * 40 + 55 : 0); return `rgb(${v(Math.floor(n / 36))},${v(Math.floor(n / 6) % 6)},${v(n % 6)})`; }
  const x = 8 + (n - 232) * 10; return `rgb(${x},${x},${x})`;
}
function sgr(st, args) {
  const a = args === '' ? [0] : args.split(';').map((x) => +x || 0);
  for (let i = 0; i < a.length; i++) {
    const n = a[i];
    if (n === 0) { for (const k of Object.keys(st)) delete st[k]; }
    else if (n === 1) st.b = 1; else if (n === 2) st.d = 1; else if (n === 3) st.i = 1; else if (n === 4) st.u = 1; else if (n === 7) st.inv = 1; else if (n === 9) st.s = 1;
    else if (n === 22) { delete st.b; delete st.d; } else if (n === 23) delete st.i; else if (n === 24) delete st.u; else if (n === 27) delete st.inv; else if (n === 29) delete st.s;
    else if (n >= 30 && n <= 37) st.fg = n - 30; else if (n === 39) delete st.fg;
    else if (n >= 40 && n <= 47) st.bg = n - 40; else if (n === 49) delete st.bg;
    else if (n >= 90 && n <= 97) st.fg = n - 90 + 8; else if (n >= 100 && n <= 107) st.bg = n - 100 + 8;
    else if (n === 38 || n === 48) {
      const key = n === 38 ? 'fg' : 'bg';
      if (a[i + 1] === 5) { const v = c256(a[i + 2] || 0); st[key] = v; i += 2; }
      else if (a[i + 1] === 2) { st[key] = `rgb(${a[i + 2] || 0},${a[i + 3] || 0},${a[i + 4] || 0})`; i += 4; }
    }
  }
}
const TOKEN_RE = /\x1b\[([0-9;?]*)([A-Za-z@`])|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()][A-Za-z0-9]|\x1b[=>78DEHMc]|\r\n|\r|\n|\x08|\x07|[^\x1b\r\n\x08\x07]+/g;
// ссылки ищутся по сырому тексту всей строки, а не по кускам одного цвета после esc(): Vite красит порт
// отдельно («http://localhost:» + «5173»), а кавычка после esc() - это «&quot;», и она уезжала в адрес
function lineHtml(segs) {
  const plain = segs.map((x) => x[1]).join('');
  const urls = plain.includes('://') ? [...plain.matchAll(URL_RE)].map((m) => [m.index, m.index + m[0].length, m[0]]) : [];
  let pos = 0, out = '';
  for (const [o, t] of segs) {
    let body = '', i = 0;
    while (i < t.length) {
      const at = pos + i, u = urls.find((x) => x[1] > at);
      if (!u || u[0] >= pos + t.length) { body += esc(t.slice(i)); break; }
      if (u[0] > at) { body += esc(t.slice(i, u[0] - pos)); i = u[0] - pos; continue; }
      const stop = Math.min(t.length, u[1] - pos);
      body += `<a data-href="${esc(u[2])}">${esc(t.slice(i, stop))}</a>`;
      i = stop;
    }
    out += o + body + (o ? '</span>' : '');
    pos += t.length;
  }
  return out;
}
function lineKind(plain) {
  if (S.errRe?.test(plain) && !S.noErrRe?.test(plain)) return 'err';
  return S.warnRe?.test(plain) ? 'warn' : '';
}
function pushLine(t) {
  const plain = t.cur.map((x) => x[1]).join('');
  t.lines.push(lineHtml(t.cur)); t.plain.push(plain);
  const kind = lineKind(plain);
  t.kinds.push(kind);
  if (kind === 'err') t.errs++;
  for (const u of plain.match(URL_RE) || []) if (LOCAL_URL.test(u)) t.urls.add(u.replace(/\/$/, ''));
  t.cur = []; t.cr = false;
  if (t.lines.length > MAX_LINES) {
    const cut = 2000;
    t.errs -= t.kinds.slice(0, cut).filter((k) => k === 'err').length;
    t.lines.splice(0, cut); t.plain.splice(0, cut); t.kinds.splice(0, cut);
    t.dirty = Math.max(0, t.dirty - cut); t.trimmed = (t.trimmed || 0) + cut; t.reset = true;
  }
}
function popLines(t, n) {
  n = Math.min(n, t.lines.length);
  if (!n) return;
  t.errs -= t.kinds.slice(-n).filter((k) => k === 'err').length;
  t.lines.splice(-n); t.plain.splice(-n); t.kinds.splice(-n);
  t.dirty = Math.min(t.dirty, t.lines.length);
}
function termWrite(t, text) {
  // хвост кусочка мог оборваться посреди ESC-последовательности - склеиваем со следующим
  text = t.carry + text;
  t.carry = '';
  const tail = /\x1b(?:\[[0-9;?]*|\][^\x07\x1b]*|[()])?$/.exec(text);
  if (tail) { t.carry = tail[0]; text = text.slice(0, tail.index); }
  TOKEN_RE.lastIndex = 0;
  let m;
  while ((m = TOKEN_RE.exec(text))) {
    const s = m[0];
    if (s === '\n' || s === '\r\n') pushLine(t);
    else if (s === '\r') t.cr = true;
    else if (s === '\x08') { const last = t.cur[t.cur.length - 1]; if (last) { last[1] = last[1].slice(0, -1); if (!last[1]) t.cur.pop(); } }
    else if (s === '\x07') { /* звонок */ }
    else if (s[0] === '\x1b') {
      if (!m[2]) continue;
      const cmd = m[2], args = m[1];
      if (cmd === 'm') sgr(t.st, args.replace(/\?/g, ''));
      else if (cmd === 'K') { if (args === '1' || args === '2' || t.cr || !t.cur.length) t.cur = []; }
      else if (cmd === 'A' || cmd === 'F') { popLines(t, +args || 1); t.cur = []; t.cr = false; }
      else if (cmd === 'G' || (cmd === 'H' && !args)) t.cr = true;
      else if (cmd === 'J' && (args === '2' || args === '3')) { t.lines = []; t.plain = []; t.kinds = []; t.cur = []; t.dirty = 0; t.errs = 0; t.reset = true; }
    } else {
      if (t.cr) { t.cur = []; t.cr = false; }
      t.cur.push([styleKey(t.st), s]);
    }
  }
}
function termSys(run, text) {
  run.term ||= makeTerm();
  const t = run.term;
  if (t.cur.length) pushLine(t);
  t.lines.push(`<span class="sys">${esc(text)}</span>`); t.plain.push(text); t.kinds.push('sys');
}
const activeRun = () => S.runs.get(S.active);
function activate(id) { if (!S.runs.has(id)) return; S.active = id; S.console.open = true; LS.set('c.open', true); renderConsole(true); }
function openLog(id) {
  if (S.runs.has(id)) return activate(id);
  const h = S.hist.find((x) => x.id === id);
  if (!h) return;
  const run = { ...h, id: 'h:' + id, kind: 'hist' };
  run.term = makeTerm();
  termSys(run, `Сохранённый вывод запуска от ${new Date(h.startedAt).toLocaleString('ru-RU')} - последние строки`);
  termWrite(run.term, (h.tail || '') + '\n');
  termSys(run, `${RU[h.status] || h.status}${h.code != null ? ', код ' + h.code : ''} · ${dur((h.endedAt || h.startedAt) - h.startedAt)}`);
  S.runs.set(run.id, run);
  activate(run.id);
}
function addRun(r, focus) {
  const prev = S.runs.get(r.id);
  const run = Object.assign(prev || { term: null }, r);
  if (!run.term) { run.term = makeTerm(); termSys(run, `▶ ${run.cmd}   ·   ${run.where === '.' ? 'корень проекта' : run.where}   ·   ${clock(run.startedAt)}`); }
  S.runs.set(r.id, run);
  if (focus || !prev) { S.active = r.id; S.console.open = true; }
  return run;
}
// не больше 12 вкладок, но открытая видна всегда: старый запуск из истории иначе показывал вывод без вкладки
function consoleTabs() {
  const all = [...S.runs.values()].sort((a, b) => b.startedAt - a.startedAt), top = all.slice(0, 12);
  const a = S.runs.get(S.active);
  if (a && !top.includes(a)) top[top.length - 1] = a;
  return top;
}
function runStatusText(r) {
  if (r.kind === 'ci') return r.status === 'running' ? 'джоба идёт' : RU[r.status] || '';
  if (r.status === 'running') return `идёт ${dur(Date.now() - r.startedAt)}`;
  if (r.status === 'ok') return `готово за ${dur(r.endedAt - r.startedAt)}`;
  if (r.status === 'stopped') return `остановлен через ${dur(r.endedAt - r.startedAt)}`;
  return `ошибка, код ${r.code} · ${dur((r.endedAt || Date.now()) - r.startedAt)}`;
}
function renderConsole(full = false) {
  const el = $('#console');
  const tabs = consoleTabs();
  // вкладок нет - консоль прячется и отдаёт свой DOM: до 4000 строк вывода иначе висели скрытыми (замер: 13 тыс. узлов)
  if (!tabs.length) { el.hidden = true; if (el.firstChild) el.textContent = ''; return; }
  el.hidden = false;
  if (!S.runs.has(S.active)) S.active = tabs[0].id;
  const a = activeRun(), c = S.console;
  const stdinEl = $('#stdin'), keepIn = stdinEl?.value || '', keepFocus = document.activeElement === stdinEl;
  // консоль рисуется целиком: кнопка в фокусе (вкладка, «Перенос») находится заново по своему data-*,
  // иначе каждое событие запуска роняло фокус клавиатуры на body
  const fe = el.contains(document.activeElement) && document.activeElement !== stdinEl ? document.activeElement : null;
  const fkey = fe ? ['data-crun', 'data-c', 'data-url', 'data-href', 'id'].find((k) => fe.hasAttribute(k)) : null;
  const fsel = fkey ? `[${fkey}="${CSS.escape(fe.getAttribute(fkey))}"]` : null;
  el.setAttribute('aria-label', A('console'));
  el.classList.toggle('collapsed', !c.open);
  el.classList.toggle('max', c.max && c.open);
  el.style.setProperty('--ch', c.h + 'px');
  const urls = a.term ? [...a.term.urls].slice(-3) : [];
  const isScript = a.kind === 'script';
  el.innerHTML = `<div class="cgrip" data-grip title="Потяните, чтобы изменить высоту"></div>
    <div class="chead">
      <div class="ctabs">${tabs.map((r) => `<button class="ctab ${r.id === a.id ? 'on' : ''}" ${r.id === a.id ? 'aria-current="true"' : ''} aria-keyshortcuts="Delete" data-crun="${esc(r.id)}" ${r.status === 'running' ? etaTip(r.cmd || '', r.startedAt, runEta(r)) : `title="${esc(r.cmd || '')}"`}>${sdot(r.status)}${esc(r.title)}<span class="el" data-el="${esc(r.id)}">${r.status === 'running' && r.kind === 'script' ? esc(dur(Date.now() - r.startedAt)) : ''}</span><span class="cx" data-cclose="${esc(r.id)}" aria-hidden="true" title="${esc(r.status === 'running' && r.kind === 'script' ? (UI().run || {}).closeHint || '' : 'Закрыть вкладку')}">×</span>${r.status === 'running' ? etaBar(r.startedAt, runEta(r), 'under') : ''}</button>`).join('')}</div>
      <div class="cact">
        ${c.open ? `<button class="btn ghost sm" data-c="find" title="Поиск в выводе (Ctrl+Shift+F)">Найти</button>
        ${a.term?.errs > 0 ? `<button class="btn ghost sm" data-c="err" title="Прокрутить к следующей строке с ошибкой" aria-label="Строк с ошибкой: ${a.term.errs}. Прокрутить к следующей" style="color:var(--bad)">✗ ${a.term.errs}</button>` : ''}
        <button class="btn ghost sm tog" data-c="wrap" aria-pressed="${c.wrap}" title="Перенос длинных строк">Перенос</button>
        <button class="btn ghost sm" data-c="copy" title="Скопировать весь вывод">Копировать</button>` : ''}
        ${isScript && a.status === 'running' ? '<button class="btn sm danger" data-c="stop" title="Остановить процесс и всех его потомков">■ Стоп</button>' : ''}
        ${(isScript && a.status !== 'running') || a.kind === 'ci' ? `<button class="btn sm" data-c="restart" title="${a.kind === 'ci' ? 'Загрузить журнал заново' : 'Запустить ещё раз с теми же аргументами'}">↻ Повторить</button>` : ''}
        ${a.kind === 'ci' && a.url ? `<button class="btn ghost sm" data-url="${esc(a.url)}" title="Открыть джобу в браузере" aria-label="Открыть джобу в браузере">↗</button>` : ''}
        <button class="btn ghost sm" data-c="clear" title="Закрыть все завершённые вкладки">Очистить</button>
      </div>
      <div class="cwin">
        <button class="btn ghost icon sm" data-c="max" title="${c.max ? 'Обычный размер' : 'Развернуть'}" aria-label="${c.max ? 'Обычный размер консоли' : 'Развернуть консоль'}" aria-pressed="${!!c.max}">${c.max ? '⤡' : '⤢'}</button>
        <button class="btn ghost icon sm" data-c="toggle" title="${c.open ? 'Свернуть' : 'Развернуть'} (Ctrl+J)" aria-label="${c.open ? 'Свернуть' : 'Развернуть'} вывод" aria-expanded="${c.open}" aria-keyshortcuts="Control+J"><span class="cchev${c.open ? '' : ' up'}" aria-hidden="true">${Learn.icon('chevron', 16, 'currentColor', 2)}</span></button>
      </div>
    </div>
    ${c.open ? `<div class="cmeta">${sdot(a.status)}<code title="${esc(a.cmd)}">${esc(a.cmd)}</code><span data-ctext>${esc(runStatusText(a))}</span>${a.status === 'running' ? etaLabel(a.startedAt, runEta(a)) + etaBar(a.startedAt, runEta(a), 'inl') : ''}
      ${urls.length ? `<span class="links">${urls.map((u) => `<a data-href="${esc(u)}" role="link" tabindex="0" title="Открыть в браузере">${esc(u.replace(/^https?:\/\//, ''))} <span aria-hidden="true">↗</span></a>`).join('')}</span>` : ''}
      ${c.findOpen ? `<span class="csearch"><input type="search" id="cfind" aria-label="${esc(A('find'))}" autocomplete="off" placeholder="Найти в выводе…" value="${esc(c.find)}"><span class="note" id="cfound"></span></span>` : ''}</div>
    <div class="tbody ${c.wrap ? 'tw' : ''}" id="tbody" tabindex="0" role="log" aria-live="off" aria-label="${esc(A('log'))}"></div>
    ${isScript && a.status === 'running' && !a.stdinClosed ? '<div class="stdin"><span aria-hidden="true">›</span><input type="text" id="stdin" aria-label="' + esc(A('stdin')) + '" placeholder="Ответ процессу - Enter отправит. Не завершается после ответа? Нажмите «Конец ввода»" autocomplete="off"><button class="btn sm" data-c="eof" title="Закрыть ввод (Ctrl+D)">Конец ввода</button></div>' : ''}` : ''}`;
  if (c.open) { a.term ||= makeTerm(); a.term.reset = true; syncTerm(true); if (c.findOpen) { applyFind(false); if (full) $('#cfind')?.focus(); } }
  const si = $('#stdin');
  if (si) { si.value = keepIn; if (keepFocus) si.focus(); }
  if (fsel && !el.contains(document.activeElement)) $(fsel, el)?.focus({ preventScroll: true });
  updateTitle();
}
let syncQueued = false;
function queueSync() { if (!syncQueued) { syncQueued = true; requestAnimationFrame(() => { syncQueued = false; syncTerm(false); }); } }
function syncTerm(scrollEnd) {
  const a = activeRun(), body = $('#tbody');
  if (!a || !body || !a.term) return;
  const t = a.term;
  const stick = scrollEnd || body.scrollTop + body.clientHeight >= body.scrollHeight - 30;
  if (t.reset) {
    const start = Math.max(0, t.lines.length - MAX_DOM);
    let html = start || t.trimmed ? `<div class="ln sys">… ранние ${start + (t.trimmed || 0)} строк скрыты - «Копировать» отдаст ${t.trimmed ? 'последние ' + t.lines.length : 'всё'}</div>` : '';
    for (let i = start; i < t.lines.length; i++) html += `<div class="ln ${t.kinds[i]}" data-i="${i}">${t.lines[i]}</div>`;
    html += `<div class="ln cur">${lineHtml(t.cur)}</div>`;
    body.innerHTML = t.lines.length || t.cur.length ? html : '<div class="tempty">Вывода пока нет…</div>';
    t.reset = false; t.dirty = t.lines.length;
  } else {
    const cur = body.querySelector('.ln.cur');
    if (!cur) { t.reset = true; return syncTerm(scrollEnd); }
    let node = cur.previousElementSibling;
    while (node && node.dataset.i != null && +node.dataset.i >= t.dirty) { const pr = node.previousElementSibling; node.remove(); node = pr; }
    if (t.dirty < t.lines.length) {
      const frag = document.createDocumentFragment();
      for (let i = t.dirty; i < t.lines.length; i++) { const d = document.createElement('div'); d.className = 'ln ' + t.kinds[i]; d.dataset.i = i; d.innerHTML = t.lines[i]; frag.append(d); }
      cur.before(frag);
      t.dirty = t.lines.length;
      let extra = body.children.length - MAX_DOM - 1;
      while (extra-- > 0 && body.firstElementChild !== cur) body.firstElementChild.remove();
    }
    cur.innerHTML = lineHtml(t.cur);
    $('.tempty', body)?.remove();
  }
  if (stick) body.scrollTop = body.scrollHeight;
}
function applyFind(step) {
  const a = activeRun(), body = $('#tbody'); if (!a || !body) return;
  const q = S.console.find.trim().toLowerCase();
  $$('.ln.hit', body).forEach((n) => n.classList.remove('hit', 'cur-hit'));
  const out = $('#cfound');
  if (!q) { if (out) out.textContent = ''; return; }
  const hits = [];
  const lines = $$('.ln[data-i]', body);
  for (const n of lines) if (a.term.plain[+n.dataset.i]?.toLowerCase().includes(q)) { n.classList.add('hit'); hits.push(n); }
  // совпадения в начале, которое не показано (последние 4000 строк в окне), - тоже считаем и называем
  const first = lines.length ? +lines[0].dataset.i : a.term.plain.length;
  let hidden = 0;
  for (let i = 0; i < first; i++) if (a.term.plain[i].toLowerCase().includes(q)) hidden++;
  const more = hidden ? ' · ' + tpl((UI().run || {}).findHidden, { n: hidden }) : '';
  if (!hits.length) { if (out) out.textContent = (hidden ? '' : 'нет') + more.replace(/^ · /, ''); return; }
  S.console.hit = step ? (S.console.hit + 1) % hits.length : Math.min(Math.max(S.console.hit, 0), hits.length - 1);
  const h = hits[S.console.hit]; h.classList.add('cur-hit'); h.scrollIntoView({ block: 'center' });
  if (out) out.textContent = `${S.console.hit + 1} из ${hits.length}${more}`;
}
setInterval(() => {
  for (const r of S.runs.values()) {
    if (r.status !== 'running' || r.kind !== 'script') continue;
    for (const el of $$(`[data-el="${CSS.escape(r.id)}"]`)) el.textContent = dur(Date.now() - r.startedAt);
    if (S.active === r.id) { const t = $('[data-ctext]'); if (t) t.textContent = runStatusText(r); }
  }
  etaTick();
}, 1000);

// =====================================================================
// ПАЛИТРА КОМАНД
// =====================================================================
function fuzzy(q, s) {
  q = q.toLowerCase(); s = s.toLowerCase();
  if (!q) return 1;
  const i = s.indexOf(q);
  if (i >= 0) return 1000 - i * 2 - s.length / 50;
  let pos = 0, score = 0, last = -2;
  for (const ch of q) { const k = s.indexOf(ch, pos); if (k < 0) return -1; score += k === last + 1 ? 6 : 1; last = k; pos = k + 1; }
  return score;
}
function paletteItems(q) {
  const out = [], p = P();
  const actions = [['Открыть папку проекта…', 'pick', 'Ctrl O'], ['Перечитать проект', 'rescan', 'F5'], ['Открыть терминал в проекте', 'term'], ['Открыть проект в VS Code', 'code'], ['Открыть проект в проводнике', 'explorer'],
    ...tabsOn().map(([id, l], i) => [`Вкладка: ${l}`, 'tab:' + id, i < 9 ? String(i + 1) : '']), ['Свернуть или развернуть консоль', 'console', 'Ctrl J'], ['Режим обучения: включить или выключить', 'learn'], ['Настройки', 'settings'], ['Справка и горячие клавиши', 'help', '?']]
    .filter(([, a]) => p || ['pick', 'settings', 'help', 'learn'].includes(a));
  if (p) for (const s of p.scripts) {
    const sc = Math.max(fuzzy(q, s.name) * 2, q.length > 2 && hayOf(s).includes(q.toLowerCase()) ? 300 : -1);
    if (sc >= 0) out.push({ kind: 'script', id: s.id, score: sc + (isFav(s.id) ? 50 : 0), title: s.name, sub: s.info?.summary || s.cmd, s });
  }
  for (const t of Object.values(K().topics || {})) { const sc = fuzzy(q, `урок ${t.title}`); if (sc >= 0) out.push({ kind: 'topic', id: t.id, title: `Урок: ${t.title}`, sub: t.summary, score: sc - 30 }); }
  for (const [title, act, key] of actions) { const sc = fuzzy(q, title); if (sc >= 0) out.push({ kind: 'action', act, title, sub: key ? `клавиша ${key}` : '', score: sc - 20 }); }
  for (const r of sortedRecent()) { if (p && r.path === p.path) continue; const sc = fuzzy(q, `${r.name} ${r.path}`); if (sc >= 0) out.push({ kind: 'project', path: r.path, title: `Проект: ${r.name}`, sub: r.path, score: sc - 40 }); }
  return out.sort((a, b) => b.score - a.score).slice(0, 60);
}
function openPalette() {
  S.palette = { q: '', sel: 0 };
  // поле - комбобокс над списком: стрелки двигают выбор (aria-activedescendant), фокус остаётся в поле
  overlay(`<div class="palette" role="dialog" aria-modal="true" aria-label="Палитра команд"><input id="pal" role="combobox" aria-expanded="true" aria-controls="plist" aria-autocomplete="list" aria-label="${esc(A('palette'))}" spellcheck="false" placeholder="Скрипт, урок, действие или проект…" autocomplete="off"><div class="plist" id="plist" role="listbox" aria-label="${esc(A('results'))}"></div>
    <div class="pfoot"><span>↑↓ выбор</span><span>Enter - запустить или открыть</span><span>Shift+Enter - подробности</span><span>Esc - закрыть</span></div></div>`, paletteKey);
  renderPalette();
  setTimeout(() => $('#pal')?.focus(), 0);
}
function renderPalette() {
  const pal = S.palette; if (!pal) return;
  pal.items = paletteItems(pal.q);
  pal.sel = Math.min(pal.sel, Math.max(0, pal.items.length - 1));
  const ic = { script: '▶', action: '›', project: '◫', topic: '✦' };
  $('#plist').innerHTML = pal.items.length ? pal.items.map((it, i) => {
    const running = it.kind === 'script' && lastOf(it.id)?.status === 'running';
    return `<div class="pi ${i === pal.sel ? 'sel' : ''}" id="pi-${i}" role="option" aria-selected="${i === pal.sel}" data-pi="${i}"><span class="ic" aria-hidden="true">${running ? '■' : ic[it.kind]}</span><span class="tx"><b class="${it.kind === 'script' ? 'mono' : ''}">${esc(it.title)}</b><span>${running ? 'запущен - Enter покажет вывод' : esc(it.sub)}</span></span>${it.kind === 'script' ? fxBadges(it.s, true) : `<span class="kind">${{ project: 'проект', topic: 'урок', action: 'действие' }[it.kind]}</span>`}</div>`;
  }).join('') : '<div class="note" style="padding:14px">Ничего не нашлось.</div>';
  const inp = $('#pal');
  if (inp) { if (pal.items.length) inp.setAttribute('aria-activedescendant', 'pi-' + pal.sel); else inp.removeAttribute('aria-activedescendant'); }
  $('.pi.sel')?.scrollIntoView({ block: 'nearest' });
}
function paletteKey(e) {
  const pal = S.palette; if (!pal) return;
  if (e.key === 'ArrowDown') { e.preventDefault(); pal.sel = Math.min(pal.sel + 1, pal.items.length - 1); renderPalette(); }
  else if (e.key === 'ArrowUp') { e.preventDefault(); pal.sel = Math.max(pal.sel - 1, 0); renderPalette(); }
  else if (e.key === 'Enter') { e.preventDefault(); execPalette(pal.items[pal.sel], e.shiftKey); }
}
function execPalette(it, detail) {
  if (!it) return;
  closeOverlay();
  if (it.kind === 'script') {
    const r = lastOf(it.id);
    if (detail) { if (S.tab !== 'scripts') setTab('scripts'); openDetail(it.id); }
    else if (r?.status === 'running') activate(r.id); // запущенный не останавливаем случайно
    else runScript(it.id);
  } else if (it.kind === 'project') openProject(it.path);
  else if (it.kind === 'topic') openTopic(it.id);
  else doAction(it.act);
}
function doAction(act) {
  if (act === 'pick') pick();
  else if (act === 'rescan') rescan();
  else if (act === 'term') openWhat('terminal');
  else if (act === 'code') openWhat('code');
  else if (act === 'explorer') openWhat('explorer');
  else if (act.startsWith('tab:')) setTab(act.slice(4));
  else if (act === 'console') toggleConsole();
  else if (act === 'learn') setSetting('learn', !S.settings.learn);
  else if (act === 'settings') { S.pop = 'settings'; renderTop(); }
  else if (act === 'help') openHelp();
}

// справка: тексты - knowledge/ui.json, раздел help; вкладки - по списку TABS, значки - из effects.json
function openHelp() {
  const h = UI().help || {}, sec = (x) => x || {};
  const fx = [...FX_ORDER, 'long'].map((f) => `<span class="fx fx-${f}">${esc(fxInfo(f).label)}</span><span>${esc(fxInfo(f).say)}</span>`).join('');
  const tabs = tabsOn().map(([id, label]) => (sec(h.tabs).items || {})[id] ? `<li><b>${esc(label)}</b> - ${esc(h.tabs.items[id])}</li>` : '').join('');
  const n = Math.min(9, tabsOn().length);
  const keys = (sec(h.keys).items || []).map(([k, t]) => `<kbd>${esc(tpl(k, { n }))}</kbd><span>${esc(t)}</span>`).join('');
  const code = { pult: '<code>pult</code>', help: '<code>pult help</code>' };
  overlay(`<div class="dialog" role="dialog" aria-modal="true" tabindex="-1" aria-label="${esc(h.label || '')}"><button class="btn ghost icon x" data-act="close-overlay" aria-label="Закрыть">✕</button>
    <h2>${esc(sec(h.about).title)}</h2>${(sec(h.about).text || []).map((p) => `<p>${esc(p)}</p>`).join('')}
    <h2>${esc(sec(h.tabs).title)}</h2><ul>${tabs}</ul>
    <h2>${esc(sec(h.learn).title)}</h2><p>${esc(sec(h.learn).text)}</p>
    <h2>${esc(sec(h.fx).title)}</h2><div class="keys">${fx}</div>
    <h2>${esc(sec(h.keys).title)}</h2><div class="keys">${keys}</div>
    <h2>${esc(sec(h.safe).title)}</h2><ul>${(sec(h.safe).items || []).map((x) => `<li>${esc(x)}</li>`).join('')}</ul>
    <h2>${esc(sec(h.cli).title)}</h2><p>${tpl(esc(sec(h.cli).text), code)}</p>
  </div>`);
  // фокус - в окно справки: Tab и Esc работают в нём, а не на странице под подложкой
  setTimeout(() => $('#overlay .dialog')?.focus(), 0);
}

// =====================================================================
// ДЕЙСТВИЯ
// =====================================================================
async function openProject(path, { force = false, background = false } = {}) {
  const switching = !P() || P().path.toLowerCase() !== String(path).toLowerCase();
  if (switching) { S.scanning = true; S.error = null; S.pop = null; renderTop(); renderMain(); }
  else { S.refreshing = true; renderTop(); }
  try {
    const d = await call('scan_project', { path, force, background });
    setProject(d, switching);
    loadHistory();
    loadEnv();
  } catch (e) {
    // открытый проект остаётся на месте: страница ошибки - только когда показать больше нечего
    if (switching && !P()) S.error = { msg: String(e), path };
    toast(String(e), 'bad');
  }
  S.scanning = false; S.refreshing = false;
  try { S.recent = (await call('state')).recent; } catch { /* не критично */ }
  render();
}
function setProject(d, switching) {
  S.project = d;
  S.byId = new Map(d.scripts.map((s) => [s.id, s]));
  if (switching) {
    S.f = loadFilters(d.path);
    S.ci = { remote: null, scope: 'all', status: '', data: {} };
    S.detail = null; S.showAll = false; S.gitQ = ''; S.help = {};
    S.collapsed = new Set(LS.get('fold:' + d.path, []));
    S.tab = LS.get('tab:' + d.path, 'overview');
    if (!tabsOn().some(([id]) => id === S.tab)) S.tab = 'overview';
    S.topic = null; S.descOpen = false; S.descClamp = false;
  } else if (S.detail && !S.byId.has(S.detail)) S.detail = null;
}
async function loadHistory() {
  if (!P()) return;
  const path = P().path;
  try {
    const h = await call('history', { path });
    if (P()?.path !== path) return;
    S.hist = h;
    S.histIdx = new Map();
    for (const x of h) if (!S.histIdx.has(x.scriptId)) S.histIdx.set(x.scriptId, x);
    if (S.tab === 'overview' || S.tab === 'history') renderTab(); else if (S.tab === 'scripts') renderList();
  } catch { /* истории нет */ }
}
async function loadEnv(force = false) {
  if (!P()) return;
  const path = P().path;
  const names = [...new Set(P().scripts.map((s) => s.tool).filter(Boolean))];
  try {
    const e = await call('env_info', { names, docker: !!P().health?.docker, force });
    if (P()?.path !== path) return;
    S.env = e.tools; S.docker = e.docker;
    if (S.tab === 'overview') renderTab(); else if (S.tab === 'scripts') { renderChips(); renderList(); }
    renderDrawer();
  } catch { /* не критично */ }
}
async function rescan() { if (!P()) return; await openProject(P().path, { force: true }); loadEnv(true); toast('Проект перечитан', 'ok', null, 1600); }
async function pick() {
  S.pop = null; renderTop();
  try { const path = await call('pick', { initial: P()?.path || null }); if (path) await openProject(path); } catch (e) { toast(String(e), 'bad'); }
}
async function openWhat(what, rel, line) { try { await call('open', { what, path: P()?.path || null, rel: rel ?? null, line: line ?? null, url: null }); } catch (e) { toast(String(e), 'bad'); } }
function openUrl(url) { call('open', { what: 'url', url, path: null, rel: null, line: null }).catch((e) => toast(String(e), 'bad')); }
// консоль сворачивается и разворачивается без анимации высоты: уходит и проявляется только её вывод
function toggleConsole() {
  const set = (v) => { S.console.open = v; LS.set('c.open', v); renderConsole(); };
  if (S.console.open) { const b = $('#tbody'); if (b && !b.classList.contains('leaving')) closeWithAnim(b, () => set(false)); else if (!b) set(false); return; }
  set(true);
  enterAnim($('#tbody'), 6); enterAnim($('#console .cmeta'), 3);
}
function applyTheme() {
  document.documentElement.dataset.theme = LS.get('theme', 'auto');
  document.documentElement.style.setProperty('--term-size', LS.get('termSize', 12.5) + 'px');
  syncWindowBg();
}
// фон самого окна - цвет страницы: иначе при изменении размера и открытии меню видны белые полосы WebView2
function syncWindowBg() {
  const c = getComputedStyle(document.body || document.documentElement).backgroundColor.match(/\d+/g);
  if (!c) return;
  const hex = '#' + c.slice(0, 3).map((x) => (+x).toString(16).padStart(2, '0')).join('');
  call('window_bg', { color: hex }).catch(() => {});
}
window.matchMedia?.('(prefers-color-scheme: dark)').addEventListener?.('change', () => syncWindowBg());
async function setSetting(k, v) {
  // обучение и минимальный интерфейс - противоположности: включение одного выключает другое
  const patch = { [k]: v };
  if (v === true && k === 'minimal') patch.learn = false;
  if (v === true && k === 'learn') patch.minimal = false;
  // сразу в окне - чтобы второй переключатель, нажатый следом, не потерял первый; ответ ядра принимается,
  // только если он на последний запрос: ответы двух быстрых нажатий могут прийти в другом порядке
  Object.assign(S.settings, patch);
  const seq = (setSetting.seq = (setSetting.seq || 0) + 1);
  try { const s = await call('settings', { patch }); if (seq === setSetting.seq) S.settings = s; } catch (e) { toast(String(e), 'bad'); }
  render();
}
function applyKb(kb) {
  S.kb = kb;
  Learn.setKnowledge(kb);
  const rx = (s) => { try { return s ? new RegExp(s, 'i') : null; } catch { return null; } };
  S.errRe = rx(kb.console?.error); S.noErrRe = rx(kb.console?.notError); S.warnRe = rx(kb.console?.warning);
  S.topics = new Map();
  if (kb.errors?.length) toast(`В базе знаний ошибки: ${kb.errors.length}. Подробности - в консоли разработчика`, 'bad', null, 6000);
}

// ---------- события ядра ----------
function onRun(r) {
  // вкладку закрыли, пока шёл: в консоль не возвращаем, итог - только в историю
  const gone = closedRuns.get(r.id);
  if (gone) {
    if (r.status === 'running') return;
    closedRuns.delete(r.id);
    S.histIdx.set(r.scriptId, r);
    S.hist.unshift({ ...r, tail: (gone.term?.plain || []).slice(-80).join('\n') });
    call('forget', { runId: r.id }).catch(() => {});
    renderTop();
    if (P() && r.project === P().path) { refreshScript(r.scriptId); if (S.tab === 'overview' || S.tab === 'history') renderTab(); else if (S.tab === 'scripts') renderChips(); }
    return;
  }
  // прежний статус - до addRun: он дописывает новые поля в тот же объект, и prev.status стал бы уже новым
  const prev = S.runs.get(r.id), was = prev?.status;
  const run = addRun(r, !prev);
  // чтение с экрана слышит начало и конец запуска; секундный счёт в консоли не объявляется
  if (!prev && r.status === 'running' && r.kind === 'script') announce(A('started', { name: r.title }));
  if (r.status !== 'running' && was === 'running') {
    termSys(run, `${r.status === 'ok' ? '■ Готово' : r.status === 'stopped' ? '■ Остановлено' : '■ Ошибка'} за ${dur(r.endedAt - r.startedAt)}${r.code != null && r.status !== 'stopped' ? ` · код выхода ${r.code}` : ''}`);
    S.histIdx.set(r.scriptId, r);
    S.hist.unshift({ ...r, tail: run.term.plain.slice(-80).join('\n') });
    const took = r.endedAt - r.startedAt;
    // короткий успешный запуск уведомления не даёт - о его конце говорит только объявление
    if (!(took > 2500 || r.status === 'fail')) announce(A('finished', { name: r.title, status: runStatusText(run) }));
    if (took > 2500 || r.status === 'fail') toast(`${r.status === 'ok' ? '✓' : r.status === 'stopped' ? '■' : '✗'} ${r.title} - ${runStatusText(run)}`, r.status === 'fail' ? 'bad' : r.status === 'ok' ? 'ok' : '', { label: 'Вывод', fn: () => activate(r.id) });
  }
  renderConsole(!prev);
  renderTop();
  if (P() && r.project === P().path) {
    refreshScript(r.scriptId);
    if (S.tab === 'overview' || S.tab === 'history') { const m = $('#main'), k = m.scrollTop; renderTab(); m.scrollTop = k; }
    // под фильтром «Запущены» или «Упали» строка входит в список и уходит из него вместе с запуском
    if (S.tab === 'scripts') { renderChips(); if (S.f.only === 'running' || S.f.only === 'failed') renderList(); }
  }
}
function onLog({ runId, text }) {
  const r = S.runs.get(runId); if (!r) return;
  r.term ||= makeTerm();
  const urls = r.term.urls.size, errs = r.term.errs;
  termWrite(r.term, text);
  if (S.active === runId && S.console.open) {
    queueSync();
    if (r.term.urls.size !== urls || (errs === 0) !== (r.term.errs === 0)) renderConsole();
  }
}
async function connect() {
  await listen('run', onRun);
  await listen('log', onLog);
  await listen('project', ({ reason, data }) => {
    if (!P() || data.path !== P().path) return;
    const before = P().scripts.length;
    setProject({ ...data, cached: false }, false);
    if (reason === 'files') toast(`Проект обновлён: файлы изменились${data.scripts.length !== before ? ` (${data.scripts.length - before > 0 ? '+' : ''}${data.scripts.length - before} скр.)` : ''}`, '', null, 2600);
    if (S.tab === 'scripts') { renderTop(); const m = $('#main'), k = m.scrollTop; renderChips(); renderList(); m.scrollTop = k; renderDrawer(); }
    else if (S.tab !== 'learn') render();
  });
  await listen('git', ({ path, git }) => { if (P() && path === P().path) { P().git = git; if (S.tab !== 'scripts' && S.tab !== 'learn') renderMain(); else renderTop(); } });
  await listen('project-error', ({ path, error }) => { if (P() && path === P().path) toast('Не удалось перечитать проект: ' + error, 'bad'); });
  await listen('open-path', (p) => openProject(p));
  // перетащили папку в окно - открываем её
  await listen('tauri://drag-drop', (p) => { const f = p?.paths?.[0]; if (f) openProject(f); });
}

// ---------- клики ----------
document.addEventListener('click', async (e) => {
  const t = e.target.closest('[data-act],[data-open],[data-tab],[data-stage],[data-run],[data-stop],[data-run-args],[data-detail],[data-fold],[data-fav],[data-crun],[data-cclose],[data-c],[data-pipe],[data-job],[data-url],[data-href],[data-doc],[data-edit],[data-reveal],[data-open-what],[data-copy],[data-flag],[data-fstage],[data-fonly],[data-fhooks],[data-group],[data-ci-remote],[data-ci-scope],[data-ci-status],[data-log],[data-hstatus],[data-rerun],[data-hint],[data-set],[data-theme-set],[data-term-size],[data-recent-rm],[data-recent-pin],[data-pi],[data-topic],[data-quiz],[data-quiz-reset],[data-cheat],[data-ask],[data-cf],[data-hint-x],[data-flags-all]');
  if (S.pop && !e.target.closest('.pop') && !e.target.closest('[data-act=projects],[data-act=settings],[data-act=more]')) { S.pop = null; S.popQ = ''; renderTop(); if (!t) return; }
  if (!t || t.dataset.cf !== undefined) return;
  const d = t.dataset;
  // пункт меню «Ещё» закрывает меню; проект и настройки открывают своё окно на его месте
  if (t.closest('.pop-more') && d.act !== 'projects' && d.act !== 'settings') { S.pop = null; renderTop(); }
  if (t.tagName === 'A' || d.tab || d.act === 'reset-filters' || d.quizReset) e.preventDefault();
  if (d.set !== undefined) { await setSetting(d.set, t.checked); return; }
  e.stopPropagation();
  if (d.recentRm) { await call('recent', { path: d.recentRm, action: 'remove' }); S.recent = S.recent.filter((r) => r.path !== d.recentRm); renderTop(); if (!P()) renderEmpty(); return; }
  if (d.recentPin) { const r = S.recent.find((x) => x.path === d.recentPin); await call('recent', { path: d.recentPin, action: r?.pinned ? 'unpin' : 'pin' }); if (r) r.pinned = !r.pinned; renderTop(); return; }
  if (d.url) return openUrl(d.url);
  if (d.href) return openUrl(d.href);
  if (d.cclose) { closeRunTab(d.cclose); return; }
  if (d.fav) {
    const on = !isFav(d.fav);
    P().favorites = await call('fav', { path: P().path, id: d.fav, on });
    refreshScript(d.fav);
    if (S.tab === 'overview') renderTab();
    // счётчик «Избранные» и сам список под этим фильтром зависят от звезды
    else if (S.tab === 'scripts') { renderChips(); if (S.f.only === 'fav') renderList(); }
    toast(on ? 'Добавлено в быстрый запуск на «Обзоре»' : 'Убрано из быстрого запуска', '', null, 1600);
    return;
  }
  if (d.quiz) { Learn.answer(d.quiz, +d.n); return ensureTopic(); }
  if (d.quizReset) { Learn.resetQuiz(d.quizReset); ensureTopic(); return requestAnimationFrame(() => document.getElementById('quiz-' + d.quizReset.replace(':', '-'))?.scrollIntoView({ block: 'start', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' })); }
  if (d.cheat != null) { Learn.cheat(d.cheat, d.g); return ensureTopic(); }
  if (d.topic) return openTopic(d.topic);
  if (d.ask) return askHelp(d.ask);
  if (d.run) return toggleRun(d.run);
  if (d.stop) return stopRun(d.stop);
  if (d.rerun) return runScript(d.rerun, { args: d.rargs || '' });
  if (d.runArgs) {
    const v = ($('#args')?.value || '').trim(); S.argDraft[d.runArgs] = v;
    const r = lastOf(d.runArgs);
    if (r?.status === 'running') { const s = scriptOf(d.runArgs); if (s && !(await askDanger(s, v))) return; try { addRun(await call('restart', { runId: r.id, args: v }), true); } catch (x) { toast(String(x), 'bad'); } return; }
    return runScript(d.runArgs, { args: v });
  }
  if (d.copy !== undefined) return copy(d.copy);
  if (d.flag) { const inp = $('#args'); if (inp) { inp.value = (inp.value.trim() + ' ' + d.flag).trim() + ' '; S.argDraft[S.detail] = inp.value; inp.focus(); } return; }
  if (d.edit !== undefined) return openWhat('edit', d.edit, d.line ? +d.line : undefined);
  if (d.reveal) return openWhat('reveal', d.reveal);
  if (d.doc) return openWhat('file', d.doc);
  if (d.openWhat) return openWhat(d.openWhat);
  if (d.job) { const r = remotesForCI().find((x) => x.name === S.ci.remote); if (r) openCiLog(r, { id: d.job, name: d.jname, status: d.jstatus, url: d.jurl }); return; }
  if (d.pipe) { const r = remotesForCI().find((x) => x.name === S.ci.remote); if (r) loadJobs(r, +d.pipe); return; }
  if (d.log) return openLog(d.log);
  if (d.pi !== undefined) return execPalette(S.palette?.items[+d.pi], e.shiftKey);
  if (d.crun) { S.active = d.crun; S.console.find = ''; S.console.hit = -1; renderConsole(true); return; }
  if (d.c) return consoleAction(d.c);
  if (d.open) { S.pop = null; return openProject(d.open); }
  if (d.tab) { S.pop = null; return setTab(d.tab); }
  if (d.stage) { S.f.stages = [d.stage]; S.f.only = ''; S.f.q = ''; saveFilters(); setTab('scripts'); return; }
  if (d.fold) { S.collapsed.has(d.fold) ? S.collapsed.delete(d.fold) : S.collapsed.add(d.fold); LS.set('fold:' + P().path, [...S.collapsed]); renderList(); return; }
  if (d.fstage) { const i = S.f.stages.indexOf(d.fstage); i >= 0 ? S.f.stages.splice(i, 1) : S.f.stages.push(d.fstage); saveFilters(); renderChips(); renderList(); return; }
  if (d.fonly) { S.f.only = S.f.only === d.fonly ? '' : d.fonly; saveFilters(); renderChips(); renderList(); return; }
  if (d.fhooks) { S.f.hideHooks = !S.f.hideHooks; saveFilters(); renderChips(); renderList(); return; }
  if (d.group) { S.f.group = d.group; saveFilters(); renderTab(); return; }
  if (d.ciRemote) { S.ci.remote = d.ciRemote; renderTab(); ensureCI(); return; }
  if (d.ciScope) { S.ci.scope = d.ciScope; renderTab(); ensureCI(); return; }
  if (d.ciStatus !== undefined) { S.ci.status = d.ciStatus; renderTab(); return; }
  if (d.hstatus !== undefined) { S.histFilter.status = d.hstatus; renderTab(); return; }
  if (d.hint) { LS.set('hint.' + d.hint, 1); const box = t.closest('.hint-box'); if (box) { dirty(box); box.remove(); } return; }
  if (d.hintX) {
    const on = !S.hintOpen.has(d.hintX);
    if (on) S.hintOpen.add(d.hintX); else S.hintOpen.delete(d.hintX);
    const box = t.closest('.hint-box'), body = box?.querySelector(':scope>div');
    t.setAttribute('aria-expanded', on); dirty(box);
    if (on) { box?.classList.add('open'); enterAnim(body); enterAnim(box?.querySelector(':scope>.btn'), 4); }
    else closeWithAnim(body, () => box?.classList.remove('open'));
    return;
  }
  if (d.flagsAll) { S.flagsAll.add(S.detail + ':' + d.flagsAll); renderDrawer(); return; }
  if (d.themeSet) { LS.set('theme', d.themeSet); applyTheme(); renderTop(); return; }
  if (d.termSize) { LS.set('termSize', +d.termSize); applyTheme(); renderTop(); return; }
  if (d.detail) return openDetail(d.detail, e.detail === 0);
  switch (d.act) {
    case 'home': S.project = null; S.detail = null; S.error = null; render(); break;
    case 'projects': S.pop = S.pop === 'projects' ? null : 'projects'; S.popQ = ''; S.popSel = 0; renderTop(); break;
    case 'settings': S.pop = S.pop === 'settings' ? null : 'settings'; renderTop(); break;
    case 'more': S.pop = S.pop === 'more' ? null : 'more'; renderTop(); break;
    case 'help': openHelp(); break;
    case 'side-toggle': { const sd = $('#side'); sd.classList.add('anim'); setTimeout(() => sd.classList.remove('anim'), 260); LS.set('side.mini', !sideMini()); renderTop(); break; }
    case 'toggle-learn': setSetting('learn', !S.settings.learn); break;
    case 'close-overlay': closeOverlay(); break;
    case 'palette': openPalette(); break;
    case 'pick': pick(); break;
    case 'rescan': rescan(); break;
    case 'openpath': { const v = $('#pathin').value.trim(); if (v) openProject(v); break; }
    case 'clearq': S.f.q = ''; $('#q').value = ''; t.hidden = true; renderChips(); renderList(); $('#q').focus(); break;
    // поле поиска - руками: patch() сравнивает атрибуты, а набранный текст живёт в свойстве value, и
    // разметка «до» и «после» сброса одинакова - без этого в поле оставался прежний запрос
    case 'reset-filters': { S.f = { ...defaultFilters(), group: S.f.group }; saveFilters(); const qi = $('#q'), c = $('[data-act=clearq]'); if (qi) qi.value = ''; if (c) c.hidden = true; renderTab(); break; }
    case 'showall': S.showAll = true; renderList(); break;
    case 'closedrawer': closeDetail(); break;
    case 'desc': S.descOpen = !S.descOpen; renderHead(); break;
    case 'skip': $('#main').focus(); break;
    case 'ciload': { const r = remotesForCI().find((x) => x.name === S.ci.remote); if (r?.trusted) loadCI(r, true); break; }
    case 'docker-recheck': S.docker = null; renderTab(); loadEnv(true); break;
    case 'git-trust': trustRepo(); break;
    case 'reset-hints': for (const k of Object.keys(UI().hints || {})) LS.set('hint.' + k, 0); S.pop = null; render(); break;
    case 'kb-reload': try { applyKb(await call('kb_reload')); toast('База знаний перечитана. Нажмите F5, чтобы обновить объяснения проекта', 'ok'); render(); } catch (x) { toast(String(x), 'bad'); } break;
    case 'all-topics': S.allTopics = !S.allTopics; renderTab(); break;
    case 'clear-history': if (await confirmBox({ title: 'Очистить историю запусков?', body: '<p>Сотрётся история этого проекта: когда и что запускалось. Сам проект не затронется.</p>', ok: 'Очистить', danger: true })) { await call('history_clear', { path: P().path }); S.hist = []; S.histIdx = new Map(); renderTab(); } break;
  }
});

async function consoleAction(c) {
  const a = activeRun(); if (!a) return;
  if (c === 'stop') return stopRun(a.id);
  if (c === 'restart') {
    if (a.kind === 'ci') { const r = remotesForCI().find((x) => x.name === a.remote); if (r) { S.runs.delete(a.id); openCiLog(r, a.job); } return; }
    const s = scriptOf(a.scriptId);
    if (!s) return toast('Скрипт больше не найден в проекте', 'bad');
    return runScript(s.id, { args: a.args || '' });
  }
  if (c === 'clear') { await call('forget', {}).catch(() => {}); for (const [id, r] of S.runs) if (r.status !== 'running' || r.kind !== 'script') S.runs.delete(id); return renderConsole(); }
  if (c === 'toggle') return toggleConsole();
  if (c === 'max') { S.console.max = !S.console.max; S.console.open = true; return renderConsole(); }
  if (c === 'wrap') { S.console.wrap = !S.console.wrap; LS.set('c.wrap', S.console.wrap); return renderConsole(); }
  if (c === 'copy') return copy([...(a.term?.plain || []), (a.term?.cur || []).map((x) => x[1]).join('')].join('\n'));
  if (c === 'find') { S.console.findOpen = !S.console.findOpen; return renderConsole(true); }
  if (c === 'err') {
    // каждое нажатие - следующая строка с ошибкой по кругу, а не всегда первая
    const body = $('#tbody'); if (!body) return;
    const first = +($('.ln[data-i]', body)?.dataset.i ?? 0);
    const errs = [];
    a.term.kinds.forEach((k, i) => { if (k === 'err' && i >= first) errs.push(i); });
    if (!errs.length) { toast('Строка с ошибкой скрыта в начале - используйте «Копировать»'); return; }
    const i = errs.find((x) => x > (a.errAt ?? -1)) ?? errs[0];
    a.errAt = i;
    const n = $(`.ln[data-i="${i}"]`, body);
    if (n) { $$('.ln.cur-hit', body).forEach((x) => x.classList.remove('cur-hit')); n.scrollIntoView({ block: 'center' }); n.classList.add('cur-hit'); setTimeout(() => n.classList.remove('cur-hit'), 1600); }
    return;
  }
  if (c === 'eof') {
    // после конца ввода поле прячется: писать в закрытый ввод - только получить ошибку
    try { await call('stdin', { runId: a.id, eof: true, text: null }); a.stdinClosed = true; termSys(a, (UI().run || {}).eofDone || ''); renderConsole(); } catch (e) { toast(String(e), 'bad'); }
  }
}

// ---------- ввод ----------
let qT;
document.addEventListener('input', (e) => {
  const id = e.target.id;
  const refocus = (sel) => { const pos = e.target.selectionStart; const n = $(sel); if (n) { n.focus(); n.setSelectionRange(pos, pos); } };
  if (id === 'q') { S.f.q = e.target.value; const c = $('[data-act=clearq]'); if (c) c.hidden = !S.f.q; clearTimeout(qT); qT = setTimeout(() => { renderChips(); renderList(); }, 60); }
  else if (id === 'pq') { S.popQ = e.target.value; S.popSel = 0; renderTop(); refocus('#pq'); }
  else if (id === 'pal') { S.palette.q = e.target.value; S.palette.sel = 0; renderPalette(); }
  else if (id === 'args') S.argDraft[e.target.dataset.args] = e.target.value;
  else if (id === 'cfind') { S.console.find = e.target.value; S.console.hit = 0; applyFind(false); }
  else if (id === 'gq') { S.gitQ = e.target.value; renderTab(); refocus('#gq'); }
  else if (id === 'hq') { S.histFilter.q = e.target.value; renderTab(); refocus('#hq'); }
  else if (id === 'helpq') { S.helpQ = e.target.value; renderDrawer(); refocus('#helpq'); }
});
document.addEventListener('change', (e) => {
  if (e.target.matches('[data-fwhere]')) { S.f.where = e.target.value; saveFilters(); renderChips(); renderList(); }
  if (e.target.matches('[data-fsource]')) { S.f.source = e.target.value; saveFilters(); renderChips(); renderList(); }
  if (e.target.id === 'cihosts') setSetting('ciHosts', e.target.value.split(',').map((x) => x.trim()).filter(Boolean));
});
// подсказка к термину
document.addEventListener('mouseover', (e) => {
  const t = e.target.closest('.term'), tip = $('#tip');
  // mouseover идёт на каждое движение мыши: атрибут ставится, только если подсказка правда видна
  if (!t) { if (!tip.hidden) tip.hidden = true; return; }
  const topic = topicMeta(t.dataset.topic);
  tip.innerHTML = `${esc(t.dataset.tip)}${topic ? `<small>урок: ${esc(topic.title)} - клик откроет</small>` : ''}`;
  tip.hidden = false;
  const r = t.getBoundingClientRect();
  // в узком окне подсказка не уходит за левый край: ширина берётся настоящая, а не 340
  tip.style.left = Math.max(8, Math.min(window.innerWidth - tip.offsetWidth - 8, r.left)) + 'px';
  tip.style.top = (r.bottom + 6 + 120 > window.innerHeight ? r.top - tip.offsetHeight - 6 : r.bottom + 6) + 'px';
});
document.addEventListener('click', (e) => { const t = e.target.closest('.term'); if (t?.dataset.topic && topicMeta(t.dataset.topic)) { $('#tip').hidden = true; openTopic(t.dataset.topic); } }, true);

document.addEventListener('keydown', async (e) => {
  const tag = e.target.tagName, typing = tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT';
  const k = e.key;
  if (overlayOpen()) {
    if (k === 'Escape') { e.preventDefault(); closeOverlay(); return; }
    // фокус не уходит из окна поверх страницы: Tab и Shift+Tab ходят по кругу внутри него
    if (k === 'Tab') {
      const o = $('#overlay'), f = $$('button:not([disabled]), input, select, textarea, a[href], [tabindex]:not([tabindex="-1"])', o).filter((x) => x.offsetParent);
      const first = f[0], last = f[f.length - 1], cur = document.activeElement;
      if (!f.length) { e.preventDefault(); return; }
      if (!o.contains(cur) || (e.shiftKey ? cur === first || cur === o.firstElementChild : cur === last)) { e.preventDefault(); (e.shiftKey ? last : first).focus(); }
      return;
    }
    S.overlayKey?.(e); return;
  }
  // Delete на вкладке консоли закрывает её, как крестик: крестик - часть вкладки, Tab на него не попадает
  if (k === 'Delete' && e.target.matches?.('.ctab[data-crun]')) {
    e.preventDefault();
    // фокус переходит на соседнюю вкладку, а не падает на body; вкладок не осталось - на страницу
    const i = $$('.ctab').indexOf(e.target);
    closeRunTab(e.target.dataset.crun).then(() => { if (document.activeElement !== document.body) return; const tabs = $$('.ctab'); (tabs[Math.min(i, tabs.length - 1)] || $('#main')).focus(); });
    return;
  }
  if ((e.ctrlKey || e.metaKey) && ['k', 'K', 'p', 'л', 'з'].includes(k)) { e.preventDefault(); openPalette(); return; }
  if ((e.ctrlKey || e.metaKey) && ['o', 'O', 'щ'].includes(k)) { e.preventDefault(); if (P()) { S.pop = 'projects'; S.popQ = ''; S.popSel = 0; renderTop(); } else pick(); return; }
  if ((e.ctrlKey || e.metaKey) && ['j', 'J', 'о', '`'].includes(k)) { e.preventDefault(); toggleConsole(); return; }
  if ((e.ctrlKey || e.metaKey) && e.shiftKey && ['F', 'А'].includes(k)) { e.preventDefault(); S.console.findOpen = true; S.console.open = true; renderConsole(true); return; }
  if (k === 'F5') { e.preventDefault(); if (P()) rescan(); return; }
  if (e.target.id === 'stdin') {
    if (k === 'Enter') { e.preventDefault(); const a = activeRun(), v = e.target.value; e.target.value = ''; call('stdin', { runId: a.id, text: v, eof: false }).catch((x) => toast(String(x), 'bad')); }
    else if (e.ctrlKey && ['d', 'в'].includes(k)) { e.preventDefault(); consoleAction('eof'); }
    else if (e.ctrlKey && ['c', 'с'].includes(k) && !window.getSelection().toString()) { e.preventDefault(); consoleAction('stop'); }
    return;
  }
  if (e.target.id === 'args' && k === 'Enter') { e.preventDefault(); $(`[data-run-args="${CSS.escape(e.target.dataset.args)}"]`)?.click(); return; }
  if (e.ctrlKey && k === 'Enter' && S.detail) { e.preventDefault(); $(`[data-run-args="${CSS.escape(S.detail)}"]`)?.click(); return; }
  if (e.target.id === 'cfind' && k === 'Enter') { e.preventDefault(); applyFind(true); return; }
  if (e.target.id === 'pathin' && k === 'Enter') { const v = e.target.value.trim(); if (v) openProject(v); return; }
  if (e.target.id === 'pq') {
    const list = popList();
    if (k === 'ArrowDown' || k === 'ArrowUp') { e.preventDefault(); S.popSel = Math.max(0, Math.min(list.length - 1, (S.popSel || 0) + (k === 'ArrowDown' ? 1 : -1))); renderTop(); $('#pq')?.focus(); return; }
    if (k === 'Enter') { e.preventDefault(); const r = list[S.popSel || 0]; const v = r ? r.path : S.popQ.trim(); S.pop = null; if (v) openProject(v); else renderTop(); return; }
  }
  if (k === 'Escape') {
    if (S.pop) { S.pop = null; renderTop(); return; }
    if (e.target.id === 'q' && S.f.q) { S.f.q = ''; e.target.value = ''; const c = $('[data-act=clearq]'); if (c) c.hidden = true; renderChips(); renderList(); return; }
    if (e.target.id === 'cfind') { S.console.findOpen = false; S.console.find = ''; renderConsole(); return; }
    if (S.detail) { closeDetail(); return; }
    if (S.console.max) { S.console.max = false; renderConsole(); return; }
    if (typing) e.target.blur();
    return;
  }
  if (typing || e.ctrlKey || e.metaKey || e.altKey) return;
  // строка, плитка, карточка с tabindex - не кнопка браузера: Enter и пробел нажимают её сами
  const own = e.target.matches?.('[tabindex="0"]:not(button, a[href], input, select, textarea, summary, #tbody)');
  if (own && (k === 'Enter' || k === ' ')) { e.preventDefault(); e.target.click(); return; }
  // стрелки вверх и вниз - по соседним строкам того же списка, без десятков нажатий Tab
  if (own && (k === 'ArrowDown' || k === 'ArrowUp')) {
    const cls = ['row', 'pr', 'li', 'qtile', 'rcard', 'ai-row'].find((c) => e.target.classList.contains(c));
    if (cls) {
      const all = $$(`.${cls}[tabindex="0"]`, e.target.closest('#main, #drawer') || document).filter((x) => x.offsetParent);
      const i = all.indexOf(e.target), next = all[i + (k === 'ArrowDown' ? 1 : -1)];
      if (next) { e.preventDefault(); next.focus(); }
      return;
    }
  }
  if (k === '/' && P()) { e.preventDefault(); if (S.tab !== 'scripts') setTab('scripts'); setTimeout(() => $('#q')?.focus(), 0); return; }
  if (k === '?') { openHelp(); return; }
  if (/^[1-9]$/.test(k) && P() && tabsOn()[+k - 1]) { setTab(tabsOn()[+k - 1][0]); return; }
});
document.addEventListener('mousedown', (e) => {
  if (!e.target.matches('[data-grip]')) return;
  e.preventDefault();
  const el = $('#console'), startY = e.clientY, startH = el.getBoundingClientRect().height;
  S.console.max = false; el.classList.remove('max');
  // потолок тот же, что у #console в app.css (высота окна без заголовка минус 200): иначе запомненная
  // высота больше видимой, и при уменьшении окна консоль закрывала всю страницу
  const move = (ev) => { const h = Math.max(120, Math.min(window.innerHeight - 234, startH + (startY - ev.clientY))); S.console.h = h; el.style.setProperty('--ch', h + 'px'); };
  // планка ручки видна, пока тянут, даже если указатель ушёл с полосы
  const grip = e.target; grip.classList.add('on');
  const up = () => { grip.classList.remove('on'); document.removeEventListener('mousemove', move); document.removeEventListener('mouseup', up); LS.set('c.h', S.console.h); };
  document.addEventListener('mousemove', move);
  document.addEventListener('mouseup', up);
});
// кнопка «наверх» у длинной страницы и у длинной панели подробностей: появляется, когда пролистано
// почти экран (не меньше 400 px), стоит в правом нижнем углу своей области (не поверх соседней)
function toTop(scroller) {
  let b = scroller._top;
  const far = scroller.scrollTop > Math.max(400, scroller.clientHeight * 0.8);
  if (!far) { if (b) b.hidden = true; return; }
  if (!b) {
    b = scroller._top = document.createElement('button');
    b.className = 'totop of-' + scroller.id; b.type = 'button';
    b.innerHTML = '<span aria-hidden="true">↑</span>';
    b.addEventListener('click', () => scroller.scrollTo({ top: 0, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' }));
    $('#body').append(b);
  }
  const t = (UI().lessonNav || {}).top || 'Наверх';
  b.title = t; b.setAttribute('aria-label', t);
  const r = scroller.getBoundingClientRect(), o = $('#body').getBoundingClientRect();
  b.style.right = Math.round(o.right - r.right + 18) + 'px';
  b.style.bottom = Math.round(o.bottom - r.bottom + 18) + 'px';
  // кнопка не ложится на то, что можно нажать (край плитки, звезда, кнопка строки): там она прячется
  // и появится снова, как только под ней окажется пустое место или текст
  const x1 = r.right - 18 - 38, y1 = r.bottom - 18 - 38, pts = [[x1 + 3, y1 + 3], [x1 + 35, y1 + 3], [x1 + 3, y1 + 35], [x1 + 35, y1 + 35], [x1 + 19, y1 + 19]];
  const HIT = 'button,a,input,select,textarea,summary,.row,.qtile,.li.click,.fchip,.sc-card';
  b.hidden = pts.some(([x, y]) => document.elementsFromPoint(x, y).some((el) => el !== b && !b.contains(el) && scroller.contains(el) && el.closest(HIT)));
}
document.addEventListener('scroll', (e) => { if (e.target.id === 'main' || e.target.id === 'drawer') toTop(e.target); }, true);
// контекстное меню WebView запрещает ui/titlebar.js: там же исключения - поля ввода и выделенный текст

// ---------- старт ----------
(async () => {
  applyTheme();
  try {
    const st = await call('state');
    applyKb(st.kb);
    S.recent = st.recent; S.settings = st.settings;
    await connect();
    for (const r of st.runs) {
      const run = addRun(r, false);
      call('run_log', { runId: r.id }).then((j) => { run.term = makeTerm(); termWrite(run.term, j.log || ''); if (S.active === r.id) renderConsole(); }).catch(() => {});
    }
    const target = st.openPath || (st.last && st.recent.find((r) => r.path === st.last && r.exists !== false) ? st.last : null);
    if (target) { await openProject(target); return; }
  } catch (e) { toast(String(e), 'bad'); }
  render();
})().finally(() => {
  // движение включается после первой отрисовки: окно открывается сразу готовым, без въезжающих частей
  requestAnimationFrame(() => requestAnimationFrame(() => document.documentElement.classList.add('anim-ready')));
});
