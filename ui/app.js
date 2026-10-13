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
const TABS = [['overview', 'Обзор'], ['scripts', 'Скрипты'], ['learn', 'Документация'], ['ci', 'Пайплайны'], ['git', 'Git'], ['history', 'История']];
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
  gitQ: '', pop: null, palette: null, topic: null, topics: new Map(), allTopics: false,
  errRe: null, noErrRe: null, warnRe: null,
};
const K = () => S.kb || {};
const UI = () => K().ui || {};
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

// ---------- уведомления и диалоги ----------
function toast(msg, kind = '', action = null, ms = 4200) {
  const el = document.createElement('div');
  el.className = 'toast ' + kind;
  el.innerHTML = `<span>${esc(msg)}</span>${action ? `<button>${esc(action.label)}</button>` : ''}`;
  if (action) el.querySelector('button').onclick = () => { action.fn(); el.remove(); };
  $('#toasts').append(el);
  setTimeout(() => el.remove(), ms);
}
function overlay(html, onKey) {
  const o = $('#overlay');
  o.innerHTML = html; o.hidden = false;
  o.onclick = (e) => { if (e.target === o) closeOverlay(); };
  S.overlayKey = onKey || null;
}
function closeOverlay() { const o = $('#overlay'); o.hidden = true; o.innerHTML = ''; S.palette = null; S.overlayKey = null; const r = S.confirmResolve; S.confirmResolve = null; r?.(false); }
function confirmBox({ title, body, ok = 'Продолжить', alt = null, cancel = 'Отмена', danger = false }) {
  return new Promise((resolve) => {
    const done = (v) => { S.confirmResolve = null; closeOverlay(); resolve(v === 'cancel' ? false : v); };
    overlay(`<div class="confirm" role="dialog" aria-modal="true"><h3>${esc(title)}</h3><div>${body}</div>
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
  if (isMissing(s)) return `<button class="run" disabled title="Не запустится: нет программы «${esc(s.tool)}»">▶</button>`;
  const danger = s.danger && S.settings.confirmDanger;
  return `<button class="run ${danger ? 'danger' : ''}" data-run="${esc(s.id)}" title="${danger ? 'Запустить - сначала спрошу подтверждение' : 'Запустить: ' + esc(s.cmd)}" aria-label="Запустить ${esc(s.name)}">▶</button>`;
}
function lastHtml(r) {
  if (!r) return '';
  const t = r.status === 'running' ? 'идёт' : dur((r.endedAt || Date.now()) - r.startedAt);
  return `<span class="dot s-${esc(r.status)}" title="${esc(RU[r.status] || r.status)} · ${esc(ago(r.endedAt || r.startedAt))}"></span><span>${esc(t)}</span>`;
}
const info = (text) => `<span class="info" title="${esc(text)}">i</span>`;

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
function sideMini() { const v = LS.get('side.mini', null); return v == null ? window.innerWidth < 1100 : v; }
function renderTop() {
  const p = P(), side = $('#side');
  const mini = !narrow() && sideMini();
  side.classList.toggle('mini', mini);
  document.documentElement.style.setProperty('--sidew', narrow() ? '0px' : mini ? '64px' : '236px');
  const runs = [...S.runs.values()].filter((r) => r.status === 'running' && r.kind === 'script');
  const failed = [...S.runs.values()].filter((r) => r.kind === 'script' && r.status === 'fail').length;
  const ciLast = p ? ciLastStatus() : null;
  const cnt = {
    scripts: runs.length ? `<span class="cnt run">${runs.length} ▶</span>` : p ? `<span class="cnt">${p.scripts.length}</span>` : '',
    learn: p?.topics?.length ? `<span class="cnt">${p.topics.length}</span>` : '',
    ci: ciLast ? `<span class="dot s-${esc(ciLast)}" title="Последний пайплайн: ${esc(RU[ciLast] || ciLast)}"></span>` : '',
    git: p?.git?.dirty ? `<span class="cnt">${p.git.dirty}</span>` : '',
    history: failed ? `<span class="cnt bad">${failed} ✗</span>` : '',
  };
  const mark = { scripts: runs.length > 0, history: failed > 0, git: !!p?.git?.dirty };
  side.innerHTML = `
    <div class="sb-head"><button class="sb-logo" data-act="home" title="Все проекты" aria-label="Все проекты"></button><span class="sb-title">Пульт</span>
      <button class="btn ghost icon sm" data-act="side-toggle" title="${mini ? 'Развернуть панель' : 'Свернуть панель'}" aria-label="${mini ? 'Развернуть панель' : 'Свернуть панель'}">${mini ? '»' : '«'}</button></div>
    ${p ? `<button class="sb-proj" data-act="projects" title="${esc(p.path)}&#10;Сменить проект (Ctrl+O)"><span class="av">${esc((p.name[0] || '?').toUpperCase())}</span><span class="tx"><b>${esc(p.name)}</b><small>${esc(p.path)}</small></span><span class="car">▾</span></button>`
      : '<button class="sb-proj" data-act="pick" title="Открыть папку проекта (Ctrl+O)"><span class="av">+</span><span class="tx"><b>Открыть проект</b><small>или перетащите папку в окно</small></span></button>'}
    <div class="sb-nav" role="tablist">${TABS.map(([id, label], i) => `<button class="sb-item ${p && S.tab === id ? 'on' : ''}" data-tab="${id}" ${p ? '' : 'disabled'} role="tab" aria-selected="${p && S.tab === id}" title="${label} (${i + 1})">${Learn.icon(NAV_ICON[id], 18)}<span class="lbl">${label}</span>${cnt[id] || ''}<span class="sb-mark ${mark[id] ? 'on' : ''}"></span></button>`).join('')}</div>
    ${runs.length ? `<div class="sb-sec">Идут сейчас</div><div class="sb-runs">${runs.map((r) => `<button class="sb-run" data-crun="${esc(r.id)}" title="${esc(r.cmd)}"><span class="dot s-running"></span><span class="t">${esc(r.title)}</span><span class="el" data-el="${esc(r.id)}">${esc(dur(Date.now() - r.startedAt))}</span></button>`).join('')}</div>` : ''}
    <div class="sb-foot">
      <button class="sb-search" data-act="palette" title="Найти и запустить что угодно (Ctrl+K)">${Learn.icon('search', 16)}<span>Поиск</span><kbd>Ctrl K</kbd></button>
      ${p ? `<button class="sb-item" data-act="rescan" title="Перечитать проект с диска (F5)">${S.refreshing ? '<span class="spin"></span>' : Learn.icon('refresh', 18)}<span class="lbl">${S.refreshing ? 'Обновляю…' : 'Перечитать'}</span></button>` : ''}
      <button class="sb-item ${learning() ? 'on' : ''}" data-act="toggle-learn" title="${esc((UI().learnMode || {}).text || 'Режим обучения')}">${Learn.icon('book', 18)}<span class="lbl">Обучение</span><span class="cnt">${learning() ? 'вкл' : 'выкл'}</span></button>
      <button class="sb-item" data-act="settings" title="Настройки">${Learn.icon('gear', 18)}<span class="lbl">Настройки</span></button>
      <button class="sb-item" data-act="help" title="Справка и горячие клавиши (?)">${Learn.icon('help', 18)}<span class="lbl">Справка</span></button>
    </div>`;
  $('#pops').innerHTML = (S.pop === 'projects' ? projectsPop() : '') + (S.pop === 'settings' ? settingsPop() : '');
  if (S.pop === 'projects') setTimeout(() => $('#pq')?.focus(), 0);
}
let resizeT;
window.addEventListener('resize', () => { clearTimeout(resizeT); resizeT = setTimeout(renderTop, 80); });
function sortedRecent() { return [...S.recent].sort((a, b) => (b.pinned ? 1 : 0) - (a.pinned ? 1 : 0) || b.at - a.at); }
function popList() { const q = (S.popQ || '').toLowerCase(); return sortedRecent().filter((r) => !q || `${r.name} ${r.path}`.toLowerCase().includes(q)); }
function projectsPop() {
  const list = popList();
  return `<div class="pop pop-projects" role="dialog">
    <div class="ph"><input type="search" id="pq" placeholder="Найти проект или вставить путь…" value="${esc(S.popQ || '')}" autocomplete="off"><button class="btn primary" data-act="pick">Открыть папку…</button></div>
    <div class="pl">${list.length ? list.map((r, i) => `<div class="pitem ${r.exists === false ? 'missing' : ''} ${i === (S.popSel || 0) ? 'sel' : ''}" data-open="${esc(r.path)}" title="${r.exists === false ? 'Папки больше нет' : esc(r.path)}">
      ${r.pinned ? '<span class="pin" title="Закреплён">★</span>' : ''}<span class="nm">${esc(r.name)}</span><span class="pt">${esc(r.path)}</span>
      <button class="x" data-recent-pin="${esc(r.path)}" title="${r.pinned ? 'Открепить' : 'Закрепить сверху'}">${r.pinned ? '☆' : '★'}</button>
      <button class="x" data-recent-rm="${esc(r.path)}" title="Убрать из списка">✕</button></div>`).join('')
      : `<div class="note" style="padding:14px">${S.popQ ? 'Не нашлось. Enter - открыть как путь.' : 'Недавних проектов пока нет.'}</div>`}</div>
    <div class="pf"><span>↑↓ выбор · Enter открыть · Esc закрыть · можно перетащить папку в окно</span><span>${S.recent.length}</span></div></div>`;
}
function settingsPop() {
  const s = S.settings;
  const tog = (k, title, sub) => `<label class="set"><input type="checkbox" data-set="${k}" ${s[k] ? 'checked' : ''}><span>${esc(title)}<small>${esc(sub)}</small></span></label>`;
  const theme = LS.get('theme', 'auto'), fs = LS.get('termSize', 12.5);
  const lm = UI().learnMode || {};
  return `<div class="pop pop-settings" role="dialog">
    <h4>Обучение</h4>
    ${tog('learn', lm.title || 'Режим обучения', lm.text || '')}
    <h4 style="margin-top:12px">Поведение</h4>
    ${tog('confirmDanger', 'Подтверждать опасные запуски', 'выкладка, публикация, удаление, prod - спрошу перед запуском')}
    ${tog('autoRefresh', 'Следить за файлами проекта', 'правка package.json или смена ветки обновит экран сама')}
    ${tog('ciAutoload', 'Загружать статус CI при открытии вкладки', 'один запрос; дальше - только по кнопке')}
    ${tog('colors', 'Цвета в выводе', 'просить программы печатать в цвете (FORCE_COLOR)')}
    ${tog('notify', 'Уведомлять о завершении', 'всплывающее уведомление Windows, если окно не в фокусе')}
    <div class="set-row"><span>Доверенные серверы CI<small class="note" style="display:block">кроме gitlab.com и github.com, через запятую</small></span><input type="text" id="cihosts" value="${esc((s.ciHosts || []).join(', '))}" style="width:140px"></div>
    <h4 style="margin-top:12px">Вид</h4>
    <div class="set-row"><span>Тема</span><div class="seg">${[['auto', 'Авто'], ['light', 'Светлая'], ['dark', 'Тёмная']].map(([k, l]) => `<button class="${theme === k ? 'on' : ''}" data-theme-set="${k}">${l}</button>`).join('')}</div></div>
    <div class="set-row"><span>Шрифт консоли</span><div class="seg">${[11.5, 12.5, 13.5, 15].map((n) => `<button class="${fs === n ? 'on' : ''}" data-term-size="${n}">${String(n).replace('.', ',')}</button>`).join('')}</div></div>
    <div class="set-row"><span>Подсказки</span><button class="btn sm" data-act="reset-hints">Показать снова</button></div>
    <div class="set-row"><span>База знаний<small class="note" style="display:block">${K().fromDisk ? 'читается из папки knowledge' : 'встроенная копия'}</small></span><button class="btn sm" data-act="kb-reload">Перечитать</button></div>
  </div>`;
}

function renderMain() {
  const m = $('#main');
  if (S.scanning) { m.innerHTML = `<div class="wrap"><div class="note" style="margin:30px 0 10px"><span class="spin"></span> Разбираю проект…</div>${'<div class="skel"></div>'.repeat(6)}</div>`; return; }
  if (S.error && !P()) {
    m.innerHTML = `<div class="empty"><h1>Не открылось</h1><p class="lead">${esc(S.error.msg)}<br><span class="faint">${esc(S.error.path || '')}</span></p>
      <button class="btn primary" data-act="pick">Выбрать другую папку…</button> ${S.error.path ? `<button class="btn" data-open="${esc(S.error.path)}">Попробовать снова</button>` : ''}</div>`;
    return;
  }
  if (!P()) return renderEmpty();
  const keep = m.scrollTop;
  const p = P();
  const label = (TABS.find(([id]) => id === S.tab) || [])[1] || '';
  const full = S.tab === 'overview';
  m.innerHTML = `<div class="wrap">
    <div class="pagehead">
      <div class="ttl"><div class="crumb">${esc(label)}</div>
        <h1><button class="h1btn" data-act="projects" title="Сменить проект (Ctrl+O)">${esc(p.name)}</button>${p.version ? `<span class="ver">v${esc(p.version)}</span>` : ''}${p.git ? `<span class="ver">· ${esc(p.git.branch)}</span>` : ''}</h1>
        ${full && p.description ? `<p class="desc" data-act="desc" title="Нажмите, чтобы развернуть">${esc(p.description)}</p>` : ''}
        ${full ? `<div class="chips">${p.stacks.map((s) => `<span class="chip">${esc(s)}</span>`).join('')}</div>` : ''}
      </div>
      <div class="headact">
        <div class="hero-actions">
          <button class="btn sm" data-open-what="explorer" title="Открыть папку проекта в проводнике">Проводник</button>
          <button class="btn sm" data-open-what="terminal" title="Открыть терминал в папке проекта">Терминал</button>
          <button class="btn sm" data-open-what="code" title="Открыть проект в VS Code">VS Code</button>
        </div>
        <div class="freshness" title="Пульт помнит прошлый разбор и показывает его сразу, а свежий подкладывает фоном">${p.cached ? 'из кэша · ' : ''}разобрано ${esc(ago(p.scannedAt))}${p.scanMs ? ` за ${p.scanMs} мс` : ''}</div>
      </div>
    </div>
    <div id="tabbody"></div>
  </div>`;
  renderTab();
  m.scrollTop = keep;
}
function renderTab() {
  const el = $('#tabbody');
  if (!el || !P()) return;
  if (S.tab === 'overview') el.innerHTML = viewOverview();
  else if (S.tab === 'scripts') { el.innerHTML = viewScripts(); renderChips(); renderList(); }
  else if (S.tab === 'learn') { el.innerHTML = viewLearn(); ensureTopic(); }
  else if (S.tab === 'ci') { el.innerHTML = viewCI(); ensureCI(); }
  else if (S.tab === 'git') el.innerHTML = viewGit();
  else if (S.tab === 'history') el.innerHTML = viewHistory();
}
function setTab(t) {
  S.tab = t;
  S.pop = null;
  if (P()) LS.set('tab:' + P().path, t);
  renderTop();
  renderMain();
  $('#main').scrollTop = 0;
}
function hintBox(key) {
  const h = (UI().hints || {})[key];
  if (!h || (!learning() && LS.get('hint.' + key))) return '';
  return `<div class="hint-box"><div><b>${esc(h.title)}</b><ul>${(h.items || []).map((x) => `<li>${g(x)}</li>`).join('')}</ul></div>${learning() ? '' : `<button class="btn sm" data-hint="${esc(key)}">Понятно</button>`}</div>`;
}

function renderEmpty() {
  const rec = sortedRecent();
  $('#main').innerHTML = `<div class="empty">
    <h1><i></i>Пульт</h1>
    <p class="lead">Откройте папку любого проекта. Пульт найдёт, что в нём можно запустить,<br>объяснит каждую команду и покажет уроки по технологиям проекта.<br><span class="faint">Ничего не меняет, пока вы сами не нажмёте ▶.</span></p>
    <button class="btn primary" data-act="pick" style="padding:10px 22px;font-size:15px">Открыть папку… <span class="k">Ctrl O</span></button>
    <div class="pathbox"><input type="text" id="pathin" placeholder="или вставьте путь: D:\\Projects\\мой-проект" autocomplete="off"><button class="btn" data-act="openpath">Открыть</button></div>
    <p class="note">Ещё можно просто перетащить папку в это окно.</p>
    ${rec.length ? `<div class="recent"><h4>Недавние</h4>${rec.map((r) => `<div class="rcard ${r.exists === false ? 'missing' : ''}" data-open="${esc(r.path)}" title="${r.exists === false ? 'Папки больше нет' : ''}"><b>${r.pinned ? '★ ' : ''}${esc(r.name)}</b><span class="pt">${esc(r.path)}</span></div>`).join('')}</div>` : ''}
  </div>`;
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
  return `<div class="qtile" data-detail="${esc(s.id)}" data-sid="${esc(s.id)}" title="${esc(s.info?.summary)}">${runBtn(s)}<div class="t"><b>${isFav(s.id) ? '★ ' : ''}${esc(s.name)}</b><span>${esc(s.info?.summary)}</span></div>${lastHtml(lastOf(s.id))}</div>`;
}
function viewOverview() {
  const p = P();
  const by = {}, ciBy = {}, runBy = {}, failBy = {};
  for (const s of p.scripts) {
    by[s.stage] = (by[s.stage] || 0) + 1;
    const r = lastOf(s.id);
    if (r?.status === 'running') runBy[s.stage] = 1;
    if (r?.status === 'fail') failBy[s.stage] = 1;
  }
  for (const c of p.ci) { for (const st of c.stages || []) ciBy[st.sid] = (ciBy[st.sid] || 0) + st.jobs.length; for (const j of c.jobs || []) ciBy[j.sid] = (ciBy[j.sid] || 0) + 1; }
  const steps = stages().filter((s) => s.id !== 'other' || by.other).map((s, i) => {
    const n = by[s.id] || 0, c = ciBy[s.id] || 0;
    const mark = runBy[s.id] ? '<span class="dot s-running st" title="Есть запущенный скрипт"></span>' : failBy[s.id] ? '<span class="dot d-bad st" title="Последний запуск одного из скриптов упал"></span>' : '';
    return `<button class="step ${n || c ? '' : 'zero'}" data-stage="${esc(s.id)}" title="${esc(s.hint)}">${mark}<small><span>${i + 1}</span></small><b>${esc(s.label)}</b>
      <div class="big">${n}</div><div class="sub">${plural(n, 'скрипт', 'скрипта', 'скриптов')}${c ? ` · ${c} в CI` : ''}</div>${learning() && s.learn ? `<div class="sub" style="margin-top:4px">${g(s.learn)}</div>` : ''}</button>`;
  }).join('');
  const quick = quickList();
  const hist = recentRuns(6);
  const topics = (p.topics || []).slice(0, 8);
  return `${hintBox('overview')}
    <h2>Быстрый запуск ${info('Избранные (★) и самые частые: разработка, тесты, сборка. Отметьте звёздочкой любой скрипт, чтобы он попал сюда.')}<span class="aside"><a data-tab="scripts" href="#">все скрипты →</a></span></h2>
    ${quick.length ? `<div class="quick">${quick.map(qtile).join('')}</div>` : '<div class="empty-card">Скриптов не нашлось - смотрите «Состояние» ниже.</div>'}
    <h2>Путь разработки ${info('Скрипты разложены по этапам жизни проекта. Этап определяется по имени и по тому, что скрипт делает. «В CI» - сколько джоб пайплайна относятся к этапу.')}</h2>
    <div class="flow">${steps}</div>
    <h2>Состояние ${info('Зелёный - в порядке, жёлтый - стоит посмотреть, красный - мешает работе, синий - для сведения.')}</h2>
    <div class="health">${healthItems().map(healthCard).join('')}</div>
    ${topics.length ? `<h2>Технологии проекта ${info('Уроки по тому, что нашлось в проекте. Откройте любой - там схемы, примеры и вопросы.')}<span class="aside"><a data-tab="learn" href="#">вся документация →</a></span></h2>
      <div class="quick">${topics.map((t) => `<div class="qtile" data-topic="${esc(t.id)}" title="${esc(t.summary)}"><span class="tico" style="background:${esc(t.color)}">${Learn.icon(t.icon, 18, '#fff')}</span><div class="t"><b style="font-family:inherit">${esc(t.title)}</b><span>${esc(t.why)}</span></div></div>`).join('')}</div>` : ''}
    <div class="cols">
      <div><h2>Последние запуски <span class="aside"><a data-tab="history" href="#">вся история →</a></span></h2>${hist.length ? `<div class="list">${hist.map(histLi).join('')}</div>` : '<div class="empty-card">Запусков ещё не было. Нажмите ▶ у любого скрипта.</div>'}</div>
      <div><h2>Документы проекта</h2>${p.docs.length ? `<div class="list">${p.docs.map((d) => `<div class="li click" data-doc="${esc(d.rel)}"><span class="t">${esc(d.name)}<span class="sub">${esc(d.hint || '')}</span></span>${d.kind === 'dir' ? `<span class="d">${d.count} ${plural(d.count, 'файл', 'файла', 'файлов')}</span>` : `<button class="btn ghost sm" data-edit="${esc(d.rel)}" title="Открыть в редакторе">✎</button>`}</div>`).join('')}</div>` : '<div class="empty-card">Документов в корне нет.</div>'}</div>
    </div>`;
}
function H(key, vars) { const h = (UI().health || {})[key] || { t: key, d: '' }; return { t: tpl(h.t, vars), d: tpl(h.d, vars), btn: h.btn ? tpl(h.btn, vars) : null }; }
function healthItems() {
  const p = P(), gi = p.git, h = p.health || {}, items = [];
  const add = (lv, key, vars = {}, extra = {}) => items.push({ lv, ...H(key, vars), ...extra });
  if (!gi) add('info', 'notGit');
  else {
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
function healthCard(x) {
  const act = [];
  if (x.run && scriptOf(x.run)) act.push(`<button class="btn sm" data-run="${esc(x.run)}">▶ ${esc(x.btn || 'Запустить')}</button><button class="btn ghost sm" data-detail="${esc(x.run)}">что это?</button>`);
  else if (x.tab) act.push(`<button class="btn sm" data-tab="${esc(x.tab)}">${esc(x.btn || 'Открыть')}</button>`);
  if (x.act === 'docker-recheck') act.push(`<button class="btn sm" data-act="docker-recheck">${esc(x.btn || 'Проверить ещё раз')}</button>`);
  return `<div class="hcard lv-${esc(x.lv)}"><b><span class="dot d-${x.lv === 'info' ? 'info' : esc(x.lv)}"></span>${esc(x.t)}</b>${x.d ? `<span>${g(x.d)}</span>` : ''}${act.length ? `<div class="act">${act.join('')}</div>` : ''}</div>`;
}
function recentRuns(n) {
  const cur = [...S.runs.values()].filter((r) => r.kind === 'script' && r.project === P().path);
  const ids = new Set(cur.map((r) => r.id));
  return [...cur, ...S.hist.filter((h) => !ids.has(h.id))].sort((a, b) => b.startedAt - a.startedAt).slice(0, n);
}
function histLi(r) {
  return `<div class="li click" data-log="${esc(r.id)}" title="Показать вывод"><span class="dot s-${esc(r.status)}"></span><span class="t"><span class="mono">${esc(r.title)}</span>${r.args ? ` <span class="faint">${esc(r.args)}</span>` : ''}</span><span class="d">${r.status === 'running' ? 'идёт' : esc(dur((r.endedAt || Date.now()) - r.startedAt))} · ${esc(ago(r.startedAt))}</span></div>`;
}

// ---------- Скрипты ----------
function viewScripts() {
  return `${hintBox('scripts')}
    <div class="toolbar">
      <div class="search"><input type="search" id="q" placeholder="Поиск по имени, описанию, команде…" value="${esc(S.f.q)}" autocomplete="off" aria-label="Поиск скриптов"><button class="clear" data-act="clearq" ${S.f.q ? '' : 'hidden'} aria-label="Очистить">×</button></div>
      <div class="seg" title="Как группировать">${[['stage', 'По этапам'], ['where', 'По папкам'], ['source', 'По источнику']].map(([k, l]) => `<button class="${S.f.group === k ? 'on' : ''}" data-group="${k}">${l}</button>`).join('')}</div>
      <span class="cnt" id="cnt"></span>
    </div>
    <div class="fchips" id="fchips"></div>
    <div id="list"></div>`;
}
function renderChips() {
  const p = P(), f = S.f, el = $('#fchips');
  if (!el) return;
  const cnt = (pred, except) => p.scripts.filter((s) => matches(s, except) && pred(s)).length;
  const chip = (attr, val, label, on, n, title = '') => `<button class="fchip ${on ? 'on' : ''} ${n === 0 && !on ? 'zero' : ''}" ${attr}="${esc(val)}" title="${esc(title)}" aria-pressed="${on}">${esc(label)}${n != null ? `<span class="n">${n}</span>` : ''}</button>`;
  const st = stages().map((s) => { const n = cnt((x) => x.stage === s.id, 'stage'); return n || f.stages.includes(s.id) ? chip('data-fstage', s.id, s.label, f.stages.includes(s.id), n, s.hint) : ''; }).join('');
  const only = [['fav', '★ Избранные', 'Отмеченные звёздочкой'], ['safe', 'Только безопасные', 'Скрипты, которые только читают и проверяют'], ['running', '▶ Запущены', 'Работают сейчас'], ['failed', '✗ Упали', 'Последний запуск закончился ошибкой']]
    .map(([k, l, t]) => chip('data-fonly', k, l, f.only === k, cnt(onlyTests[k], 'only'), t)).join('');
  const wheres = [...new Set(p.scripts.map((s) => s.where))];
  const sources = [...new Set(p.scripts.map(srcKind))];
  const active = f.stages.length + (f.only ? 1 : 0) + (f.where ? 1 : 0) + (f.source ? 1 : 0) + (f.hideHooks ? 1 : 0) + (f.q ? 1 : 0);
  el.innerHTML = `${st}<span class="fsep"></span>${only}<span class="fsep"></span>
    ${wheres.length > 1 ? `<select data-fwhere title="Папка, где объявлен скрипт"><option value="">Все папки</option>${wheres.map((w) => `<option value="${esc(w)}" ${f.where === w ? 'selected' : ''}>${esc(w === '.' ? 'корень' : w)} (${cnt((s) => s.where === w, 'where')})</option>`).join('')}</select>` : ''}
    ${sources.length > 1 ? `<select data-fsource title="Откуда скрипт"><option value="">Все источники</option>${sources.map((w) => `<option value="${esc(w)}" ${f.source === w ? 'selected' : ''}>${esc(w)} (${cnt((s) => srcKind(s) === w, 'source')})</option>`).join('')}</select>` : ''}
    ${p.scripts.some((s) => s.info?.hook) ? chip('data-fhooks', '1', 'Скрыть хуки', f.hideHooks, null, 'Хуки pre/post запускаются сами вместе с основным скриптом') : ''}
    ${active ? `<button class="btn ghost sm" data-act="reset-filters">Сбросить (${active})</button>` : ''}`;
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
    html += `<section class="group ${closed ? 'closed' : ''}"><h3 data-fold="${esc(fold)}"><span class="car">▼</span>${esc(label)}<span class="n">${rows.length}</span>${hint ? `<span class="gh">- ${esc(hint)}</span>` : ''}</h3><div class="rows">${part.map(rowHtml).join('')}</div></section>`;
    if (shown >= LIMIT) break;
  }
  if (items.length > shown) html += `<button class="btn more" data-act="showall">Показать все (${items.length})</button>`;
  list.innerHTML = html || (p.scripts.length
    ? '<div class="nothing"><b>Ничего не подходит под фильтры</b>Измените поиск или <a href="#" data-act="reset-filters">сбросьте фильтры</a>.</div>'
    : '<div class="nothing"><b>Скриптов не нашлось</b>Пульт ищет package.json, Makefile, justfile, Taskfile, Cargo, Go, dotnet, Python, Maven/Gradle, Docker Compose и файлы .ps1/.sh/.bat в папках scripts и tools.</div>');
}
function rowHtml(s) {
  const r = lastOf(s.id), fav = isFav(s.id);
  return `<div class="row ${S.detail === s.id ? 'sel' : ''}" data-detail="${esc(s.id)}" data-sid="${esc(s.id)}">${runBtn(s)}
    <div class="rmain"><div class="rname"><span class="nm">${hl(s.name)}</span>${fxBadges(s, true)}${s.where !== '.' && S.f?.group !== 'where' ? `<span class="tag" title="Папка: ${esc(s.where)}">${esc(s.where)}</span>` : ''}${s.info?.hook ? `<span class="tag" title="Запускается сам ${esc(s.info.hook)}">хук</span>` : ''}</div>
    <div class="rdesc">${hl(s.info?.summary || s.cmd)}</div></div>
    <div class="rside">${lastHtml(r)}<button class="star ${fav ? 'on' : ''}" data-fav="${esc(s.id)}" title="${fav ? 'Убрать из быстрого запуска' : 'В быстрый запуск на «Обзоре»'}" aria-label="Избранное">★</button></div></div>`;
}
function refreshScript(id) {
  const s = scriptOf(id); if (!s) return;
  for (const el of $$(`[data-sid="${CSS.escape(id)}"]`)) {
    const t = document.createElement('div');
    t.innerHTML = el.classList.contains('qtile') ? qtile(s) : rowHtml(s);
    el.replaceWith(t.firstElementChild);
  }
  if (S.detail === id) renderDrawer();
}

// ---------- панель подробностей ----------
function renderDrawer() {
  const d = $('#drawer');
  const s = S.detail && P() ? scriptOf(S.detail) : null;
  if (!s) { d.hidden = true; d.innerHTML = ''; return; }
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
  d.innerHTML = `<div class="dh"><div class="top"><h3>${esc(s.name)}</h3><button class="star ${isFav(s.id) ? 'on' : ''}" data-fav="${esc(s.id)}" title="Быстрый запуск">★</button><button class="btn ghost icon" data-act="closedrawer" title="Закрыть (Esc)" aria-label="Закрыть">✕</button></div>
    <div class="meta"><span class="chip" title="${esc(stageOf(s.stage).hint)}">${esc(stageOf(s.stage).label)}</span><span class="chip">${esc(srcKind(s))}</span>${s.where !== '.' ? `<span class="chip">${esc(s.where)}</span>` : ''}${fxBadges(s)}</div></div>
    <div class="db">
      <section><h4>Что делает</h4><p class="lead2">${g(i.summary)}</p>${i.steps?.length ? `<ol class="steps">${i.steps.map((x) => `<li>${g(x)}</li>`).join('')}</ol>` : ''}${i.hook ? `<p class="note" style="margin-top:6px">${esc(tpl(W.hook, { hook: i.hook }))}</p>` : ''}
        ${i.about ? `<p class="note" style="margin-top:6px"><b>Программа:</b> ${g(i.about)}</p>` : ''}<p class="srcnote">${esc((UI().sources || {})[i.source || ''] || '')}</p></section>
      ${i.header && (i.header.split('\n')[0] !== i.summary || i.header.includes('\n')) ? `<section><h4>Описание от автора ${info('Текст из комментария в начале файла скрипта')}</h4><div class="quote">${esc(i.header)}<small>из ${esc(i.file || 'файла скрипта')}</small></div></section>` : ''}
      ${parts ? `<section><h4>Команда по частям ${info('Из чего состоит команда и что значит каждая часть')}</h4><div class="anat">${parts}</div></section>` : ''}
      ${i.how?.length ? `<section><details ${howOpen}><summary><h4 style="display:inline">Что происходит внутри</h4></summary><ol class="how" style="margin-top:8px">${i.how.map((x) => `<li>${g(x)}</li>`).join('')}</ol></details></section>` : ''}
      <section><h4>Что произойдёт при запуске</h4><ul class="will">${will.map(([lv, t, l]) => `<li><span class="dot d-${esc(lv)}"></span><span>${g(t)}${l ? `<div class="learnbox">${g(l)}</div>` : ''}</span></li>`).join('')}</ul></section>
      <section><h4>Команда</h4><div class="cmdbox"><code>${esc(s.cmd)}</code><button data-copy="${esc(s.cmd)}" title="Скопировать в буфер">Копировать</button></div>
        ${s.source === 'package.json' && s.desc ? `<p class="note" style="margin-top:6px">В package.json записано: <code>${esc(s.desc)}</code></p>` : ''}</section>
      <section><h4>Запуск ${info('Аргументы допишутся в конец команды. Для npm-скриптов Пульт сам поставит «--».')}</h4>
        <div class="argrow"><input type="text" id="args" data-args="${esc(s.id)}" placeholder="доп. аргументы, например --fast" value="${esc(S.argDraft[s.id] || '')}" autocomplete="off"><button class="btn ${running ? '' : 'primary'}" data-run-args="${esc(s.id)}" ${miss ? 'disabled' : ''}>${running ? 'Перезапустить' : '▶ Запустить'}</button></div>
        ${running ? `<div class="drow" style="margin-top:8px"><button class="btn sm danger" data-stop="${esc(r.id)}">■ Остановить</button><button class="btn sm" data-log="${esc(r.id)}">Показать вывод</button></div>` : ''}</section>
      <section><h4>Ключи ${info('Нажмите на ключ - он добавится в аргументы. «Спросить у программы» запускает её справку (--help) и разбирает все ключи.')}</h4>
        ${flags.length ? `<div class="flaglist">${flags.map((f) => `<div class="fl ${f.used ? 'used' : ''}"><code>${esc(f.flag)}</code><span>${g(f.text || (f.from === 'file' ? 'ключ из кода скрипта' : ''))}${f.used ? ' · уже в команде' : ''}</span><button class="btn ghost" data-flag="${esc(f.flag.split(/[ ,=|]/)[0])}" title="Добавить в аргументы">+</button></div>`).join('')}</div>` : '<p class="note">Ключей в базе нет.</p>'}
        <div class="drow" style="margin-top:8px"><button class="btn sm" data-ask="${esc(s.id)}" ${help === 'loading' ? 'disabled' : ''}>${help === 'loading' ? '<span class="spin"></span> Спрашиваю…' : 'Спросить у программы (--help)'}</button></div>
        ${help && help !== 'loading' ? (help.error ? `<p class="err" style="margin-top:8px">${esc(help.error)}</p>` : `<p class="note" style="margin-top:8px">${esc(help.program)}: найдено ключей - ${help.flags.length}${help.about ? `. ${esc(help.about)}` : ''}</p>
          ${help.flags.length > 8 ? `<input type="search" id="helpq" placeholder="найти ключ…" value="${esc(S.helpQ)}" style="width:100%;margin:6px 0">` : ''}
          <div class="flaglist">${helpFlags.slice(0, 200).map((f) => `<div class="fl"><code>${esc(f.flag)}</code><span>${esc(f.text)}</span><button class="btn ghost" data-flag="${esc(f.flag.replace(/^-\w,\s*/, '').split(/[ ,=|<[]/)[0])}" title="Добавить в аргументы">+</button></div>`).join('') || '<div class="fl"><span>ничего</span></div>'}</div>
          <details><summary class="note">вся справка программы</summary><pre class="helptext">${esc(help.text)}</pre></details>`) : ''}</section>
      ${tps.length ? `<section><h4>Чему научиться ${info('Уроки на вкладке «Документация» про программы этой команды')}</h4><div class="topiclinks">${tps.map((t) => `<button class="chip linkchip" data-topic="${esc(t)}">${esc(topicMeta(t).title)}</button>`).join('')}</div></section>` : ''}
      <section><h4>Где объявлен</h4><p style="font-size:13px">${src}</p>
        <div class="drow" style="margin-top:8px">${editRel ? `<button class="btn sm" data-edit="${esc(editRel)}" ${s.line ? `data-line="${s.line}"` : ''}>Открыть в редакторе</button><button class="btn sm" data-reveal="${esc(editRel)}">Показать в проводнике</button>` : ''}${i.file && i.file !== s.fileRel ? `<button class="btn sm" data-edit="${esc(i.file)}">Открыть ${esc(i.file.split('/').pop())}</button>` : ''}</div></section>
      <section><h4>Запуски этого скрипта</h4>${scriptRuns(s.id).length ? `<div class="list">${scriptRuns(s.id).map(histLi).join('')}</div>` : '<p class="note">Ещё не запускался.</p>'}</section>
    </div>`;
}
function scriptRuns(id) {
  const cur = [...S.runs.values()].filter((r) => r.scriptId === id && r.kind === 'script');
  const ids = new Set(cur.map((r) => r.id));
  return [...cur, ...S.hist.filter((h) => h.scriptId === id && !ids.has(h.id))].sort((a, b) => b.startedAt - a.startedAt).slice(0, 6);
}
function openDetail(id) {
  const prev = S.detail;
  S.detail = id; S.helpQ = '';
  $$('.row.sel').forEach((el) => el.classList.remove('sel'));
  $$(`.row[data-sid="${CSS.escape(id)}"]`).forEach((el) => el.classList.add('sel'));
  renderDrawer();
  if (prev !== id) $('#drawer').scrollTop = 0;
}
async function askHelp(id) {
  S.help[id] = 'loading'; renderDrawer();
  try { S.help[id] = await call('help', { scriptId: id }); } catch (e) { S.help[id] = { error: String(e) }; }
  if (S.detail === id) renderDrawer();
}

// ---------- запуск ----------
async function runScript(id, { args, force } = {}) {
  const s = scriptOf(id); if (!s) return;
  if (isMissing(s)) { toast(`Нет программы «${s.tool}» - скрипт не запустится`, 'bad'); return; }
  if (s.danger && S.settings.confirmDanger && !force) {
    const ok = await confirmBox({ title: `Запустить «${s.name}»?`, danger: true, ok: 'Запустить',
      body: `<p style="margin:0 0 8px">${esc(s.info?.summary || '')}</p><div class="cmdbox"><code>${esc(s.cmd)}${args ? ' ' + esc(args) : ''}</code></div><p class="note" style="margin-top:8px">${esc(fxInfo('danger').say)}.</p>` });
    if (!ok) return;
  }
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
// кнопка ▶ в строке: идёт - остановить, иначе запустить
function toggleRun(id) { const r = lastOf(id); if (r?.status === 'running') stopRun(r.id); else runScript(id); }

// ---------- Документация ----------
function viewLearn() {
  const found = P().topics || [];
  const ids = new Set(found.map((t) => t.id));
  const all = Object.values(K().topics || {});
  const others = all.filter((t) => !ids.has(t.id));
  if (!S.topic || (!ids.has(S.topic) && !S.allTopics && !others.some((t) => t.id === S.topic))) S.topic = found[0]?.id || all[0]?.id || null;
  const card = (t, why) => `<button class="tcard ${S.topic === t.id ? 'on' : ''} ${why ? '' : 'off'}" data-topic="${esc(t.id)}"><span class="tico" style="background:${esc(t.color)}">${Learn.icon(t.icon, 17, '#fff')}</span><span class="tx"><b>${esc(t.title)}</b><small>${esc(why || t.summary)}</small></span></button>`;
  return `${hintBox('learn')}<div class="learn">
    <nav class="tlist" aria-label="Уроки"><h4>В этом проекте · ${found.length}</h4>${found.map((t) => card(t, t.why)).join('') || '<p class="note">Ничего не определилось.</p>'}
      ${others.length ? `<h4><a href="#" data-act="all-topics">${S.allTopics ? 'скрыть' : 'показать'} остальные уроки · ${others.length}</a></h4>${S.allTopics ? others.map((t) => card(t, '')).join('') : ''}` : ''}</nav>
    <div id="lessonbody"><div class="note"><span class="spin"></span> Открываю урок…</div></div></div>`;
}
async function ensureTopic() {
  const id = S.topic, el = $('#lessonbody');
  if (!id || !el) return;
  let t = S.topics.get(id);
  if (!t) { try { t = await call('topic', { id }); S.topics.set(id, t); } catch (e) { el.innerHTML = `<p class="err">${esc(String(e))}</p>`; return; } }
  if (S.topic !== id || !$('#lessonbody')) return;
  const why = (P().topics || []).find((x) => x.id === id)?.why;
  $('#lessonbody').innerHTML = Learn.topicHtml(t, { learn: learning(), why, topicTitle: (r) => topicMeta(r)?.title, projectRows: (tid) => projectRowsFor(tid) });
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
  let html = hintBox('ci');
  if (!rem.length) html += '<div class="empty-card">У репозитория нет адреса на GitLab или GitHub - живой статус показывать неоткуда. Настройка из файлов - ниже.</div>';
  else {
    const cur = rem.find((r) => r.name === S.ci.remote) || rem[0];
    S.ci.remote = cur.name;
    const st = S.ci.data[remoteKey(cur)] || {};
    const label = (r) => (/gitlab/i.test(r.host) ? 'GitLab' : 'GitHub');
    html += `<div class="ci-bar">
      ${rem.length > 1 ? `<div class="seg">${rem.map((r) => `<button class="${r.name === cur.name ? 'on' : ''}" data-ci-remote="${esc(r.name)}" title="${esc(r.url)}">${label(r)} · ${esc(r.slug)}</button>`).join('')}</div>` : `<b>${label(cur)} · <span class="mono">${esc(cur.slug)}</span></b>`}
      ${p.git?.branch ? `<div class="seg"><button class="${S.ci.scope === 'all' ? 'on' : ''}" data-ci-scope="all">Все ветки</button><button class="${S.ci.scope === 'branch' ? 'on' : ''}" data-ci-scope="branch">Ветка ${esc(p.git.branch)}</button></div>` : ''}
      <div class="seg">${[['', 'Все'], ['running', 'Идут'], ['failed', 'Ошибки'], ['success', 'Успешные']].map(([k, l]) => `<button class="${S.ci.status === k ? 'on' : ''}" data-ci-status="${k}">${l}</button>`).join('')}</div>
      <span class="grow"></span><span class="note">${st.at ? `${st.cached ? 'из кэша · ' : ''}данные на ${clock(st.at)}` : ''}</span>
      <button class="btn sm" data-act="ciload" ${st.loading ? 'disabled' : ''}>↻ Обновить</button></div>`;
    html += cur.trusted ? liveBody(st) : `<div class="err">Сервер «${esc(cur.host)}» не в списке доверенных: Пульт не отправит ему запросы с вашим токеном. Если это ваш сервер CI - добавьте его в настройках (⚙).</div>`;
  }
  html += `<h2>Настройка в репозитории ${info('Читается из файлов проекта. Пунктир - джоба запускается вручную. Подписи - комментарии из файла.')}</h2>`;
  html += p.ci.length ? p.ci.map(ciConfigHtml).join('') : '<div class="empty-card">Файлов CI не найдено (.gitlab-ci.yml, .github/workflows, Jenkinsfile…).</div>';
  return html;
}
function liveBody(st) {
  if (st.loading && !st.runs) return `<div class="runs">${'<div class="pr"><span class="skel" style="grid-column:1/-1;margin:2px 0"></span></div>'.repeat(4)}</div>`;
  let out = '';
  if (st.error) {
    const tool = /glab/.test(st.error) ? 'glab' : /gh/.test(st.error) ? 'gh' : null;
    out += `<div class="err">${esc(st.error)}${tool && /auth login/.test(st.error) ? `<div class="cmdbox" style="margin-top:8px"><code>${tool} auth login</code><button data-copy="${tool} auth login">Копировать</button></div>` : ''}</div>`;
    if (st.runs) out += `<p class="warnline">Показаны прежние данные - от ${clock(st.at)}.</p>`;
  }
  if (!st.runs) return out || '<div class="empty-card">Статус ещё не загружен. <button class="btn sm" data-act="ciload">Загрузить</button></div>';
  if (!st.runs.length) return out + `<div class="empty-card">Пайплайнов ${S.ci.scope === 'branch' ? 'по этой ветке ' : ''}пока нет.</div>`;
  const runs = st.runs.filter((x) => !S.ci.status || x.status === S.ci.status || (S.ci.status === 'running' && x.status === 'pending'));
  out += runs.length ? `<div class="runs">${runs.map((x) => `<div class="pr ${st.sel === x.id ? 'sel' : ''}" data-pipe="${esc(x.id)}" title="${esc(x.title || '')}">
      <span class="dot s-${esc(x.status)}" title="${esc(RU[x.status] || x.status)}"></span><span class="id">#${esc(x.id)}</span>
      <span class="rf">${esc(x.name ? x.name + ' · ' : '')}<b>${esc(refLabel(x.ref))}</b> <span class="mono faint">${esc(x.sha)}</span> ${x.title ? `<span class="faint">${esc(x.title)}</span>` : ''}</span>
      <span class="meta">${esc(RU[x.status] || x.status)} · ${esc(ago(x.createdAt))} <button class="btn ghost sm" data-url="${esc(x.url)}" title="Открыть в браузере">↗</button></span></div>`).join('')}</div>`
    : `<div class="empty-card">Под фильтр «${esc(S.ci.status)}» ничего не попало.</div>`;
  if (st.sel) {
    const sel = st.runs.find((x) => x.id === st.sel);
    if (st.jobsLoading && !st.jobs) out += `<div class="card"><span class="spin"></span> Загружаю этапы пайплайна #${esc(st.sel)}…</div>`;
    else if (st.jobsError) out += `<div class="err">${esc(st.jobsError)}</div>`;
    else if (st.jobs) {
      const all = st.jobs.flatMap((x) => x.jobs);
      const done = all.filter((j) => FINAL.includes(j.status)).length;
      const failed = all.filter((j) => j.status === 'failed' && !j.allowFailure);
      const now = st.jobs.find((x) => x.jobs.some((j) => j.status === 'running' || j.status === 'pending'));
      out += `<div class="card"><h4>Пайплайн #${esc(st.sel)} <small>${esc(refLabel(sel?.ref))} · ${esc(RU[sel?.status] || '')}</small>${st.jobsLoading ? '<span class="spin"></span>' : ''}</h4>
        <div class="summary-line">${now ? `<span>Сейчас: этап <b>${esc(now.name)}</b></span>` : ''}<span>Готово <b>${done}</b> из ${all.length}</span>${failed.length ? `<span style="color:var(--bad)">Упали: <b>${failed.map((j) => esc(j.name)).join(', ')}</b></span>` : ''}<span class="faint">Клик по джобе - её журнал в консоли</span></div>
        <div class="pipe">${st.jobs.map((x) => `<div class="scol ${now === x ? 'now' : ''}"><div class="sh">${esc(x.name)}</div>${x.jobs.map((j) => `<button class="job ${j.status === 'manual' ? 'man' : ''}" data-job="${esc(j.id)}" data-jname="${esc(j.name)}" data-jstatus="${esc(j.status)}" data-jurl="${esc(j.url || '')}" title="${esc(RU[j.status] || j.status)}${j.step ? ' · шаг: ' + esc(j.step) : ''}${j.failedStep ? ' · упал шаг: ' + esc(j.failedStep) : ''}${j.failureReason ? ' · причина: ' + esc(j.failureReason) : ''}${j.allowFailure ? ' · падение допустимо' : ''}${j.runner ? ' · раннер: ' + esc(j.runner) : ''}"><span class="dot s-${esc(j.status)}"></span><span class="jn">${esc(j.name)}</span><span class="dur">${j.duration != null ? esc(dur(j.duration * 1000)) : ''}</span></button>`).join('')}</div>`).join('')}</div></div>`;
    }
  }
  return out;
}
function ciConfigHtml(c) {
  if (c.kind === 'gitlab' || c.kind === 'jenkins') {
    const cols = (c.stages || []).map((st) => `<div class="scol"><div class="sh">${esc(st.name)} <span class="sid">${esc(stageOf(st.sid).label.toLowerCase())}</span></div>${st.jobs.length ? st.jobs.map((j) => {
      const tags = [j.manual && 'вручную', j.allowFailure && 'может упасть', j.onlyMain && 'только main', j.onlyMr && 'в MR', j.needs?.length && `после ${j.needs.length}`].filter(Boolean);
      return `<div class="job cfg ${j.manual ? 'man' : ''}"><span class="jn">${esc(j.name)}</span>${tags.length ? `<span class="jtags">${tags.map((t) => `<span class="jtag">${esc(t)}</span>`).join('')}</span>` : ''}${j.desc ? `<span class="jd">${esc(j.desc)}</span>` : ''}</div>`;
    }).join('') : '<div class="note">- пусто</div>'}</div>`).join('');
    return `<div class="card"><h4>${c.kind === 'gitlab' ? 'GitLab CI' : 'Jenkins'} <small>${esc(c.file)} · ${c.jobCount} ${plural(c.jobCount, 'джоба', 'джобы', 'джоб')}${c.includes?.length ? ` · подключает ${c.includes.length} ${plural(c.includes.length, 'файл', 'файла', 'файлов')}` : ''}</small><button class="btn ghost sm" data-edit="${esc(c.file)}">✎</button></h4><div class="pipe">${cols || '<span class="note">Этапы не распознаны</span>'}</div></div>`;
  }
  if (c.kind === 'github') {
    return `<div class="card"><h4>${esc(c.name)} <small>${esc(c.file)}</small><button class="btn ghost sm" data-edit="${esc(c.file)}">✎</button></h4>
      <p class="note" style="margin:-4px 0 8px">Запускается: ${(c.triggers || []).map((t) => `<span class="jtag">${esc(t)}</span>`).join(' ') || '-'}</p>
      <div class="pipe"><div class="scol" style="max-width:none;flex:1">${(c.jobs || []).map((j) => `<div class="job cfg"><span class="jn">${esc(j.name)}</span>${j.needs.length ? `<span class="jtags"><span class="jtag">после ${esc(j.needs.join(', '))}</span></span>` : ''}</div>`).join('') || '<span class="note">Джоб нет</span>'}</div></div></div>`;
  }
  return `<div class="card"><h4>${esc(c.file)} <small>найден, подробный разбор пока не поддерживается</small><button class="btn ghost sm" data-edit="${esc(c.file)}">✎</button></h4></div>`;
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
  if (res.data) st.jobs = res.data;
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
function viewGit() {
  const gi = P().git;
  if (!gi) return '<div class="empty-card">Эта папка не в git-репозитории.</div>';
  const gq = S.gitQ.toLowerCase();
  const files = gi.files.filter((f) => !gq || f.path.toLowerCase().includes(gq));
  const groups = {};
  for (const f of files) (groups[f.label] ||= []).push(f);
  const actions = P().scripts.filter((s) => s.source === 'git');
  return `${hintBox('git')}
    <div class="stats">
      <div class="stat"><small>Ветка</small><b>${esc(gi.branch)}</b></div>
      <div class="stat" title="Ветка на сервере, с которой сравнивается ваша"><small>На сервере</small><b>${esc(gi.upstream || '-')}</b></div>
      <div class="stat"><small>Не отправлено / не забрано</small><b>↑${gi.ahead} ↓${gi.behind}</b></div>
      <div class="stat"><small>Изменено файлов</small><b>${gi.dirty}</b></div>
      <div class="stat" title="git stash"><small>Отложено</small><b>${gi.stash}</b></div>
      <div class="stat"><small>Последний fetch</small><b style="font-size:14px">${esc(gi.fetchedAt ? ago(gi.fetchedAt) : 'никогда')}</b></div>
      ${gi.tag ? `<div class="stat"><small>Последний тег</small><b>${esc(gi.tag)}</b></div>` : ''}
    </div>
    <h2>Действия</h2><div class="rows">${actions.map(rowHtml).join('')}</div>
    <h2>Изменённые файлы <span class="aside">${gi.files.length > 8 ? `<input type="search" id="gq" placeholder="фильтр…" value="${esc(S.gitQ)}" style="width:200px">` : ''}</span></h2>
    ${gi.files.length ? Object.entries(groups).map(([label, list]) => `<div class="list" style="margin-bottom:8px">${list.slice(0, 300).map((f) => `<div class="li click" data-edit="${esc(f.path)}" title="Открыть в редакторе"><span class="gcode g-${esc(f.code)}">${esc(f.code === '?' ? '+' : f.code)}</span><span class="t mono" style="font-size:12.5px">${esc(f.path)}</span><span class="d">${esc(label)}${f.staged ? ' · в индексе' : ''}</span><button class="btn ghost sm" data-reveal="${esc(f.path)}" title="Показать в проводнике">⌕</button></div>`).join('')}</div>`).join('') : '<div class="empty-card">Всё закоммичено.</div>'}
    <div class="cols">
      <div><h2>Последние коммиты</h2><div class="list">${gi.commits.map((c) => `<div class="li"><span class="h">${esc(c.hash)}</span><span class="t" title="${esc(c.subject)}">${esc(c.subject)} ${c.refs.map((r) => `<span class="ref">${esc(r)}</span>`).join(' ')}</span><span class="d">${esc(c.author)} · ${esc(ago(c.date))}</span></div>`).join('') || '<div class="li note">Коммитов нет</div>'}</div></div>
      <div><h2>Адреса репозитория</h2><div class="list">${gi.remotes.map((r) => `<div class="li"><b class="mono" style="font-size:12.5px">${esc(r.name)}</b><span class="t mono faint" style="font-size:12px">${esc(r.url)}</span>${r.dup ? '<span class="d">дубль</span>' : ''}${r.trusted ? '' : '<span class="d" title="CI-запросы к этому серверу не отправляются">не доверенный</span>'}</div>`).join('') || '<div class="li note">Нет адресов - репозиторий только локальный</div>'}</div></div>
    </div>`;
}

// ---------- История ----------
function viewHistory() {
  const hf = S.histFilter;
  const all = recentRuns(1000);
  const list = all.filter((r) => (!hf.status || r.status === hf.status) && (!hf.q || `${r.title} ${r.args || ''} ${r.cmd}`.toLowerCase().includes(hf.q.toLowerCase())));
  const ok = all.filter((r) => r.status === 'ok').length, fail = all.filter((r) => r.status === 'fail').length;
  return `<div class="toolbar">
      <div class="seg">${[['', `Все ${all.length}`], ['ok', `Успех ${ok}`], ['fail', `Ошибка ${fail}`], ['stopped', 'Остановлены'], ['running', 'Идут']].map(([k, l]) => `<button class="${hf.status === k ? 'on' : ''}" data-hstatus="${k}">${l}</button>`).join('')}</div>
      <div class="search"><input type="search" id="hq" placeholder="Найти запуск…" value="${esc(hf.q)}"></div>
      <span class="grow"></span>${all.length ? '<button class="btn sm" data-act="clear-history">Очистить историю</button>' : ''}</div>
    ${list.length ? `<div class="list htable">${list.slice(0, 400).map((r) => `<div class="li click" data-log="${esc(r.id)}" title="Показать вывод (у старых - последние 80 строк)">
      <span class="dot s-${esc(r.status)}"></span><span class="t mono">${esc(r.title)}</span><span class="t faint mono" style="font-size:12px">${esc(r.args || '')}</span>
      <span class="d">${esc(new Date(r.startedAt).toLocaleString('ru-RU', { day: '2-digit', month: '2-digit', hour: '2-digit', minute: '2-digit' }))}</span><span class="d">${r.status === 'running' ? 'идёт' : esc(dur((r.endedAt || Date.now()) - r.startedAt))}</span><span class="d">${r.code != null ? 'код ' + esc(r.code) : ''}</span>
      ${scriptOf(r.scriptId) ? `<button class="btn ghost sm" data-rerun="${esc(r.scriptId)}" data-rargs="${esc(r.args || '')}" title="Запустить снова с теми же аргументами">▶</button>` : '<span></span>'}</div>`).join('')}</div>`
    : `<div class="nothing"><b>${all.length ? 'Ничего не подходит' : 'История пуста'}</b>${all.length ? 'Измените фильтр.' : 'Здесь появится каждый запуск: когда, сколько шёл и чем закончился.'}</div>`}`;
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
const linkify = (h) => h.replace(/https?:\/\/[^\s"'<>`]+[^\s"'<>`.,;:!?)\]]/g, (u) => `<a data-href="${u}">${u}</a>`);
const lineHtml = (segs) => segs.map(([o, t]) => o + linkify(esc(t)) + (o ? '</span>' : '')).join('');
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
const consoleTabs = () => [...S.runs.values()].sort((a, b) => b.startedAt - a.startedAt).slice(0, 12);
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
  if (!tabs.length) { el.hidden = true; return; }
  el.hidden = false;
  if (!S.runs.has(S.active)) S.active = tabs[0].id;
  const a = activeRun(), c = S.console;
  const stdinEl = $('#stdin'), keepIn = stdinEl?.value || '', keepFocus = document.activeElement === stdinEl;
  el.classList.toggle('collapsed', !c.open);
  el.classList.toggle('max', c.max && c.open);
  el.style.setProperty('--ch', c.h + 'px');
  const urls = a.term ? [...a.term.urls].slice(-3) : [];
  const isScript = a.kind === 'script';
  el.innerHTML = `<div class="cgrip" data-grip title="Потяните, чтобы изменить высоту"></div>
    <div class="chead">
      <div class="ctabs" role="tablist">${tabs.map((r) => `<button class="ctab ${r.id === a.id ? 'on' : ''}" data-crun="${esc(r.id)}" title="${esc(r.cmd || '')}"><span class="dot s-${esc(r.status)}"></span>${esc(r.title)}<span class="el" data-el="${esc(r.id)}">${r.status === 'running' && r.kind === 'script' ? esc(dur(Date.now() - r.startedAt)) : ''}</span>${r.status !== 'running' || r.kind !== 'script' ? `<span class="cx" data-cclose="${esc(r.id)}" title="Закрыть вкладку">×</span>` : ''}</button>`).join('')}</div>
      <div class="cact">
        ${c.open ? `<button class="btn ghost sm" data-c="find" title="Поиск в выводе (Ctrl+Shift+F)">Найти</button>
        ${a.term?.errs > 0 ? `<button class="btn ghost sm" data-c="err" title="Прокрутить к первой строке с ошибкой" style="color:var(--bad)">✗ ${a.term.errs}</button>` : ''}
        <button class="btn ghost sm ${c.wrap ? '' : 'faint'}" data-c="wrap" title="Перенос длинных строк">Перенос</button>
        <button class="btn ghost sm" data-c="copy" title="Скопировать весь вывод">Копировать</button>` : ''}
        ${isScript && a.status === 'running' ? '<button class="btn sm danger" data-c="stop" title="Остановить процесс и всех его потомков">■ Стоп</button>' : ''}
        ${(isScript && a.status !== 'running') || a.kind === 'ci' ? `<button class="btn sm" data-c="restart" title="${a.kind === 'ci' ? 'Загрузить журнал заново' : 'Запустить ещё раз с теми же аргументами'}">↻ Повторить</button>` : ''}
        ${a.kind === 'ci' && a.url ? `<button class="btn ghost sm" data-url="${esc(a.url)}" title="Открыть джобу в браузере">↗</button>` : ''}
        <button class="btn ghost sm" data-c="clear" title="Закрыть все завершённые вкладки">Очистить</button>
        <button class="btn ghost icon sm" data-c="max" title="${c.max ? 'Обычный размер' : 'Развернуть'}">${c.max ? '⤡' : '⤢'}</button>
        <button class="btn ghost icon sm" data-c="toggle" title="${c.open ? 'Свернуть' : 'Развернуть'} (Ctrl+J)">${c.open ? '▾' : '▴'}</button>
      </div>
    </div>
    ${c.open ? `<div class="cmeta"><span class="dot s-${esc(a.status)}"></span><code title="${esc(a.cmd)}">${esc(a.cmd)}</code><span data-ctext>${esc(runStatusText(a))}</span>
      ${urls.length ? `<span class="links">${urls.map((u) => `<a data-href="${esc(u)}" title="Открыть в браузере">${esc(u.replace(/^https?:\/\//, ''))} ↗</a>`).join('')}</span>` : ''}
      ${c.findOpen ? `<span class="csearch"><input type="search" id="cfind" placeholder="Найти в выводе…" value="${esc(c.find)}"><span class="note" id="cfound"></span></span>` : ''}</div>
    <div class="tbody ${c.wrap ? 'tw' : ''}" id="tbody" tabindex="0"></div>
    ${isScript && a.status === 'running' ? '<div class="stdin"><span>›</span><input type="text" id="stdin" placeholder="Ответ процессу - Enter отправит. Не завершается после ответа? Нажмите «Конец ввода»" autocomplete="off"><button class="btn sm" data-c="eof" title="Закрыть ввод (Ctrl+D)">Конец ввода</button></div>' : ''}` : ''}`;
  if (c.open) { a.term ||= makeTerm(); a.term.reset = true; syncTerm(true); if (c.findOpen) { applyFind(false); if (full) $('#cfind')?.focus(); } }
  const si = $('#stdin');
  if (si) { si.value = keepIn; if (keepFocus) si.focus(); }
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
  for (const n of $$('.ln[data-i]', body)) if (a.term.plain[+n.dataset.i]?.toLowerCase().includes(q)) { n.classList.add('hit'); hits.push(n); }
  if (!hits.length) { if (out) out.textContent = 'нет'; return; }
  S.console.hit = step ? (S.console.hit + 1) % hits.length : Math.min(Math.max(S.console.hit, 0), hits.length - 1);
  const h = hits[S.console.hit]; h.classList.add('cur-hit'); h.scrollIntoView({ block: 'center' });
  if (out) out.textContent = `${S.console.hit + 1} из ${hits.length}`;
}
setInterval(() => {
  for (const r of S.runs.values()) {
    if (r.status !== 'running' || r.kind !== 'script') continue;
    for (const el of $$(`[data-el="${CSS.escape(r.id)}"]`)) el.textContent = dur(Date.now() - r.startedAt);
    if (S.active === r.id) { const t = $('[data-ctext]'); if (t) t.textContent = runStatusText(r); }
  }
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
    ...TABS.map(([id, l], i) => [`Вкладка: ${l}`, 'tab:' + id, String(i + 1)]), ['Свернуть или развернуть консоль', 'console', 'Ctrl J'], ['Режим обучения: включить или выключить', 'learn'], ['Настройки', 'settings'], ['Справка и горячие клавиши', 'help', '?']]
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
  overlay(`<div class="palette" role="dialog" aria-label="Палитра команд"><input id="pal" placeholder="Скрипт, урок, действие или проект…" autocomplete="off"><div class="plist" id="plist"></div>
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
    return `<div class="pi ${i === pal.sel ? 'sel' : ''}" data-pi="${i}"><span class="ic">${running ? '■' : ic[it.kind]}</span><span class="tx"><b class="${it.kind === 'script' ? 'mono' : ''}">${esc(it.title)}</b><span>${running ? 'запущен - Enter покажет вывод' : esc(it.sub)}</span></span>${it.kind === 'script' ? fxBadges(it.s, true) : `<span class="kind">${{ project: 'проект', topic: 'урок', action: 'действие' }[it.kind]}</span>`}</div>`;
  }).join('') : '<div class="note" style="padding:14px">Ничего не нашлось.</div>';
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

function openHelp() {
  const fx = FX_ORDER.map((f) => `<span class="fx fx-${f}">${esc(fxInfo(f).label)}</span><span>${esc(fxInfo(f).say)}</span>`).join('');
  overlay(`<div class="dialog" role="dialog" aria-label="Справка"><button class="btn ghost icon x" data-act="close-overlay" aria-label="Закрыть">✕</button>
    <h2>Что такое Пульт</h2>
    <p>Пульт читает папку проекта и показывает всё, что в нём можно запустить, - с объяснением простыми словами. Сам ничего не меняет: каждый запуск - только по вашей кнопке ▶.</p>
    <p>Пояснения берутся из базы знаний (папка knowledge рядом с программой), из комментариев в файлах скриптов, из описаний пакетов и из справки самих программ. Под конкретный проект ничего не подгоняется.</p>
    <h2>Вкладки</h2><ul>
      <li><b>Обзор</b> - быстрый запуск, путь разработки по этапам, состояние и технологии проекта.</li>
      <li><b>Скрипты</b> - все команды с фильтрами. Клик по строке - подробности справа.</li>
      <li><b>Документация</b> - уроки по технологиям проекта: схемы, примеры, частые ошибки, вопросы.</li>
      <li><b>Пайплайны</b> - статус CI на сервере и настройка CI из файлов. Журнал джобы - в консоли.</li>
      <li><b>Git</b> и <b>История</b> - состояние репозитория и все прошлые запуски.</li></ul>
    <h2>Режим обучения</h2><p>Включается в настройках (⚙). Термины подчёркиваются и раскрываются по наведению, у каждого скрипта видно, что происходит внутри, у каждого значка - зачем он.</p>
    <h2>Значки «что затронет»</h2><div class="keys">${fx}<span class="fx fx-long">${esc(fxInfo('long').label)}</span><span>${esc(fxInfo('long').say)}</span></div>
    <h2>Горячие клавиши</h2>
    <div class="keys"><kbd>Ctrl K</kbd><span>найти и запустить что угодно</span><kbd>Ctrl O</kbd><span>сменить проект</span><kbd>/</kbd><span>поиск по скриптам</span><kbd>1</kbd><span>…<kbd>6</kbd> вкладки</span>
      <kbd>F5</kbd><span>перечитать проект</span><kbd>Ctrl J</kbd><span>свернуть или развернуть консоль</span><kbd>Ctrl Shift F</kbd><span>поиск в выводе консоли</span><kbd>Esc</kbd><span>закрыть панель, очистить поиск</span><kbd>Ctrl Enter</kbd><span>запустить из панели подробностей</span><kbd>?</kbd><span>эта справка</span></div>
    <h2>Безопасность</h2><ul>
      <li>Нет сервера и открытого порта: окно говорит с программой напрямую.</li>
      <li>Команды запускаются без оболочки, списком аргументов: имя файла не может стать второй командой.</li>
      <li>Файл .env не читается - только проверяется, что он есть. Сетевые папки не открываются.</li>
      <li>Запросы CI уходят только на доверенные серверы.</li></ul>
    <h2>Пульт в терминале</h2><p>В любой папке проекта наберите <code>pult</code> - тот же Пульт в консоли, без окна. <code>pult help</code> - все команды.</p>
  </div>`);
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
    if (switching) { S.project = null; S.error = { msg: String(e), path }; }
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
    if (!TABS.some(([id]) => id === S.tab)) S.tab = 'overview';
    S.topic = null;
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
function toggleConsole() { S.console.open = !S.console.open; LS.set('c.open', S.console.open); renderConsole(); }
function applyTheme() {
  document.documentElement.dataset.theme = LS.get('theme', 'auto');
  document.documentElement.style.setProperty('--term-size', LS.get('termSize', 12.5) + 'px');
}
async function setSetting(k, v) {
  try { S.settings = await call('settings', { patch: { [k]: v } }); } catch (e) { toast(String(e), 'bad'); }
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
  const prev = S.runs.get(r.id);
  const run = addRun(r, !prev);
  if (r.status !== 'running' && prev?.status === 'running') {
    termSys(run, `${r.status === 'ok' ? '■ Готово' : r.status === 'stopped' ? '■ Остановлено' : '■ Ошибка'} за ${dur(r.endedAt - r.startedAt)}${r.code != null && r.status !== 'stopped' ? ` · код выхода ${r.code}` : ''}`);
    S.histIdx.set(r.scriptId, r);
    S.hist.unshift({ ...r, tail: run.term.plain.slice(-80).join('\n') });
    const took = r.endedAt - r.startedAt;
    if (took > 2500 || r.status === 'fail') toast(`${r.status === 'ok' ? '✓' : r.status === 'stopped' ? '■' : '✗'} ${r.title} - ${runStatusText(run)}`, r.status === 'fail' ? 'bad' : r.status === 'ok' ? 'ok' : '', { label: 'Вывод', fn: () => activate(r.id) });
  }
  renderConsole(!prev);
  renderTop();
  if (P() && r.project === P().path) {
    refreshScript(r.scriptId);
    if (S.tab === 'overview' || S.tab === 'history') { const m = $('#main'), k = m.scrollTop; renderTab(); m.scrollTop = k; }
    if (S.tab === 'scripts') renderChips();
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
  const t = e.target.closest('[data-act],[data-open],[data-tab],[data-stage],[data-run],[data-stop],[data-run-args],[data-detail],[data-fold],[data-fav],[data-crun],[data-cclose],[data-c],[data-pipe],[data-job],[data-url],[data-href],[data-doc],[data-edit],[data-reveal],[data-open-what],[data-copy],[data-flag],[data-fstage],[data-fonly],[data-fhooks],[data-group],[data-ci-remote],[data-ci-scope],[data-ci-status],[data-log],[data-hstatus],[data-rerun],[data-hint],[data-set],[data-theme-set],[data-term-size],[data-recent-rm],[data-recent-pin],[data-pi],[data-topic],[data-quiz],[data-quiz-reset],[data-ask],[data-cf]');
  if (S.pop && !e.target.closest('.pop') && !e.target.closest('[data-act=projects],[data-act=settings]')) { S.pop = null; S.popQ = ''; renderTop(); if (!t) return; }
  if (!t || t.dataset.cf !== undefined) return;
  const d = t.dataset;
  if (t.tagName === 'A' || d.tab || d.act === 'reset-filters' || d.quizReset) e.preventDefault();
  if (d.set !== undefined) { await setSetting(d.set, t.checked); return; }
  e.stopPropagation();
  if (d.recentRm) { await call('recent', { path: d.recentRm, action: 'remove' }); S.recent = S.recent.filter((r) => r.path !== d.recentRm); renderTop(); if (!P()) renderEmpty(); return; }
  if (d.recentPin) { const r = S.recent.find((x) => x.path === d.recentPin); await call('recent', { path: d.recentPin, action: r?.pinned ? 'unpin' : 'pin' }); if (r) r.pinned = !r.pinned; renderTop(); return; }
  if (d.url) return openUrl(d.url);
  if (d.href) return openUrl(d.href);
  if (d.cclose) { const id = d.cclose; S.runs.delete(id); if (!id.includes(':')) call('forget', { runId: id }).catch(() => {}); renderConsole(); return; }
  if (d.fav) {
    const on = !isFav(d.fav);
    P().favorites = await call('fav', { path: P().path, id: d.fav, on });
    refreshScript(d.fav);
    if (S.tab === 'overview') renderTab();
    toast(on ? 'Добавлено в быстрый запуск на «Обзоре»' : 'Убрано из быстрого запуска', '', null, 1600);
    return;
  }
  if (d.quiz) { Learn.answer(d.quiz, +d.n); return ensureTopic(); }
  if (d.quizReset) { Learn.resetQuiz(d.quizReset); return ensureTopic(); }
  if (d.topic) return openTopic(d.topic);
  if (d.ask) return askHelp(d.ask);
  if (d.run) return toggleRun(d.run);
  if (d.stop) return stopRun(d.stop);
  if (d.rerun) return runScript(d.rerun, { args: d.rargs || '' });
  if (d.runArgs) {
    const v = ($('#args')?.value || '').trim(); S.argDraft[d.runArgs] = v;
    const r = lastOf(d.runArgs);
    if (r?.status === 'running') { try { addRun(await call('restart', { runId: r.id, args: v }), true); } catch (x) { toast(String(x), 'bad'); } return; }
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
  if (d.hint) { LS.set('hint.' + d.hint, 1); t.closest('.hint-box')?.remove(); return; }
  if (d.themeSet) { LS.set('theme', d.themeSet); applyTheme(); renderTop(); return; }
  if (d.termSize) { LS.set('termSize', +d.termSize); applyTheme(); renderTop(); return; }
  if (d.detail) return openDetail(d.detail);
  switch (d.act) {
    case 'home': S.project = null; S.detail = null; S.error = null; render(); break;
    case 'projects': S.pop = S.pop === 'projects' ? null : 'projects'; S.popQ = ''; S.popSel = 0; renderTop(); break;
    case 'settings': S.pop = S.pop === 'settings' ? null : 'settings'; renderTop(); break;
    case 'help': openHelp(); break;
    case 'side-toggle': LS.set('side.mini', !sideMini()); renderTop(); break;
    case 'toggle-learn': setSetting('learn', !S.settings.learn); break;
    case 'close-overlay': closeOverlay(); break;
    case 'palette': openPalette(); break;
    case 'pick': pick(); break;
    case 'rescan': rescan(); break;
    case 'openpath': { const v = $('#pathin').value.trim(); if (v) openProject(v); break; }
    case 'clearq': S.f.q = ''; $('#q').value = ''; t.hidden = true; renderChips(); renderList(); $('#q').focus(); break;
    case 'reset-filters': S.f = { ...defaultFilters(), group: S.f.group }; saveFilters(); renderTab(); break;
    case 'showall': S.showAll = true; renderList(); break;
    case 'closedrawer': S.detail = null; $$('.row.sel').forEach((x) => x.classList.remove('sel')); renderDrawer(); break;
    case 'desc': t.classList.toggle('open'); break;
    case 'ciload': { const r = remotesForCI().find((x) => x.name === S.ci.remote); if (r?.trusted) loadCI(r, true); break; }
    case 'docker-recheck': S.docker = null; renderTab(); loadEnv(true); break;
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
    const body = $('#tbody'), i = a.term.kinds.indexOf('err');
    const n = body && $(`.ln[data-i="${i}"]`, body);
    if (n) { n.scrollIntoView({ block: 'center' }); n.classList.add('cur-hit'); setTimeout(() => n.classList.remove('cur-hit'), 1600); }
    else toast('Строка с ошибкой скрыта в начале - используйте «Копировать»');
    return;
  }
  if (c === 'eof') return call('stdin', { runId: a.id, eof: true, text: null }).catch((e) => toast(String(e), 'bad'));
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
  if (!t) { tip.hidden = true; return; }
  const topic = topicMeta(t.dataset.topic);
  tip.innerHTML = `${esc(t.dataset.tip)}${topic ? `<small>урок: ${esc(topic.title)} - клик откроет</small>` : ''}`;
  tip.hidden = false;
  const r = t.getBoundingClientRect();
  tip.style.left = Math.min(window.innerWidth - 350, Math.max(8, r.left)) + 'px';
  tip.style.top = (r.bottom + 6 + 120 > window.innerHeight ? r.top - tip.offsetHeight - 6 : r.bottom + 6) + 'px';
});
document.addEventListener('click', (e) => { const t = e.target.closest('.term'); if (t?.dataset.topic && topicMeta(t.dataset.topic)) { $('#tip').hidden = true; openTopic(t.dataset.topic); } }, true);

document.addEventListener('keydown', async (e) => {
  const tag = e.target.tagName, typing = tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT';
  const k = e.key;
  if (!$('#overlay').hidden) { if (k === 'Escape') { e.preventDefault(); closeOverlay(); return; } S.overlayKey?.(e); return; }
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
    if (e.target.id === 'q' && S.f.q) { S.f.q = ''; e.target.value = ''; renderChips(); renderList(); return; }
    if (e.target.id === 'cfind') { S.console.findOpen = false; S.console.find = ''; renderConsole(); return; }
    if (S.detail) { S.detail = null; $$('.row.sel').forEach((x) => x.classList.remove('sel')); renderDrawer(); return; }
    if (S.console.max) { S.console.max = false; renderConsole(); return; }
    if (typing) e.target.blur();
    return;
  }
  if (typing || e.ctrlKey || e.metaKey || e.altKey) return;
  if (k === '/' && P()) { e.preventDefault(); if (S.tab !== 'scripts') setTab('scripts'); setTimeout(() => $('#q')?.focus(), 0); return; }
  if (k === '?') { openHelp(); return; }
  if (/^[1-6]$/.test(k) && P()) { setTab(TABS[+k - 1][0]); return; }
});
document.addEventListener('mousedown', (e) => {
  if (!e.target.matches('[data-grip]')) return;
  e.preventDefault();
  const el = $('#console'), startY = e.clientY, startH = el.getBoundingClientRect().height;
  S.console.max = false; el.classList.remove('max');
  const move = (ev) => { const h = Math.max(120, Math.min(window.innerHeight - 140, startH + (startY - ev.clientY))); S.console.h = h; el.style.setProperty('--ch', h + 'px'); };
  const up = () => { document.removeEventListener('mousemove', move); document.removeEventListener('mouseup', up); LS.set('c.h', S.console.h); };
  document.addEventListener('mousemove', move);
  document.addEventListener('mouseup', up);
});
// F5 и Ctrl+R в окне перечитывают проект, а не перезагружают страницу
document.addEventListener('contextmenu', (e) => { if (!e.target.closest('input,textarea,.tbody,pre,code')) e.preventDefault(); });

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
})();
