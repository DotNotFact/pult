'use strict';
// Вкладка «ИИ»: что в проекте настроено для работы с ИИ-агентом, зачем нужна каждая часть и как части связаны.
// Данные собирает ядро (core/src/ai.rs, команда ai_map): только читает файлы проекта, ничего из них не запускает.
// Что за вид детали и что о нём сказать - knowledge/ai.json (S.kb.ai), подписи окна - knowledge/ui.json, раздел ai.
// Вкладки нет, пока в проекте не нашлось ни одного признака ИИ-части (project.aiMarkers).
// Подключается после app.js и берёт его общие помощники: S, K, UI, P, call, listen, esc, tpl, patch, g, info.
(() => {
  const T = () => UI().ai || {};
  const CFG = () => K().ai || {};
  const kindsOf = () => CFG().kinds || [];
  const kindOf = (id) => kindsOf().find((k) => k.id === id);
  const has = () => ((P() || {}).aiMarkers || []).length > 0;
  const AI = { map: null, path: '', loading: false, err: '', q: '', open: new Set(), more: new Set(), seq: 0, shown: false };
  const LAYER_TOKENS = ['accent', 'info', 'violet', 'ok', 'warn', 'bad'];
  // строки списка: сколько видно сразу, остальное - «Показать все»
  const SHOWN = 8;
  const enc = (id) => CSS.escape(String(id));
  const reduce = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

  // ---------- вкладка в общем списке ----------
  // Подпись читается при каждой отрисовке: база знаний приходит позже, чем грузится этот файл.
  const entry = ['ai', '', '', has];
  Object.defineProperty(entry, 1, { get: () => T().tab || 'ИИ' });
  Object.defineProperty(entry, 2, { get: () => T().tabShort || T().tab || 'ИИ' });
  if (!TABS.some(([id]) => id === 'ai')) TABS.push(entry);
  NAV_ICON.ai = 'chip';

  // ---------- загрузка ----------
  async function load(force) {
    const p = P();
    if (!p || !has()) return;
    const path = p.path, seq = ++AI.seq;
    if (AI.path !== path) { AI.map = null; AI.q = ''; AI.open = new Set(); AI.more = new Set(); AI.err = ''; }
    AI.path = path;
    AI.loading = !AI.map;
    if (AI.loading && S.tab === 'ai') draw();
    try {
      const m = await call('ai_map', { path, force: !!force });
      if (seq !== AI.seq || P()?.path !== path) return;
      AI.map = m; AI.err = '';
    } catch (e) { if (seq === AI.seq) AI.err = String(e); }
    AI.loading = false;
    if (S.tab === 'ai') draw();
    renderTop();
  }
  // изменение файлов ИИ-части: ядро присылает свежую карту, только если она поменялась
  listen('ai', ({ path, data }) => {
    if (!P() || path !== P().path) return;
    AI.map = data; AI.err = '';
    if (S.tab === 'ai' && !Blocks.busy()) draw();
    renderTop();
  });

  const badge = () => (AI.map && AI.map.path === P()?.path && AI.map.items.length ? `<span class="cnt">${AI.map.items.length}</span>` : '');
  function onTab(t) {
    if (t !== 'ai') { AI.shown = false; return; }
    if (!AI.shown) { AI.shown = true; load(false); }
  }
  function renderTab(el) {
    // прямой вход в вкладку (восстановление после перезапуска): карту ещё не просили
    // другой проект открыт, пока вкладка стояла выбранной: карта прежнего проекта не годится
    if (!AI.shown || AI.path !== P()?.path) { AI.shown = true; load(false); }
    el._html = null;
    draw(el);
  }
  function draw(el) {
    el = el || $('#tabbody');
    if (!el || S.tab !== 'ai' || !P()) return;
    patch(el, html());
  }

  // ---------- поиск ----------
  const hay = (it) => [it.name, it.summary, it.desc, it.file, it.eco, it.event, it.matcher, it.model, it.command, it.script, it.role, ...(it.tools || [])].filter(Boolean).join(' ').toLowerCase();
  function matcher() {
    const words = AI.q.trim().toLowerCase().split(/\s+/).filter(Boolean);
    return words.length ? (it) => { const h = hay(it); return words.every((w) => h.includes(w)); } : null;
  }

  // ---------- мелочи ----------
  const text = (s) => (learning() ? g(s) : esc(s));
  const modeOf = (k, it) => (k.modes || {})[it.mode] || null;
  const size = (n) => (n < 1024 ? tpl(T().details.bytes, { n }) : tpl(T().details.kb, { n: Math.round(n / 1024) }));
  const chips = (list, cls = '') => `<span class="chips ai-chips ${cls}">${list.map((x) => `<span class="chip" data-ext>${esc(x)}</span>`).join('')}</span>`;
  function pill(label, kind = '', hint = '') {
    return `<span class="ai-b ${kind ? `k-${kind}` : ""}"${hint ? ` title="${esc(hint)}"` : ''}>${esc(label)}</span>`;
  }
  function badgesOf(it, k) {
    const b = T().badges || {}, h = T().badgeHints || {}, out = [];
    const mode = modeOf(k, it);
    if (mode && !(k.format === 'checks')) out.push(pill(mode.label, '', mode.say));
    if (it.eco && it.eco !== 'git') out.push(pill(it.eco, 'plain'));
    if (it.paths && it.paths.length) {
      const f = it.filesMatched;
      out.push(pill(f ? tpl(f === 1 ? b.pathsOne : b.paths, { g: it.paths.length, f }) : b.pathsNone, f ? 'info' : 'warn'));
    }
    if (it.event) out.push(pill(it.event + (it.matcher ? ' · ' + tpl(b.matcher, { v: it.matcher }) : ''), 'info'));
    if (it.model) out.push(pill(tpl(b.model, { v: it.model }), 'violet'));
    if (it.tools && it.tools.length) out.push(pill(tpl(b.tools, { n: it.tools.length }), '', it.tools.join(', ')));
    if (it.transport) out.push(pill(tpl(b.transport, { v: it.transport }), 'info'));
    if (it.unwired) out.push(pill(b.unwired, 'warn', h.unwired));
    if (it.active === true) out.push(pill(b.active, 'ok', h.active));
    if (it.active === false) out.push(pill(b.inactive, 'warn', h.inactive));
    if (it.broken) out.push(pill(b.broken, 'bad', h.broken));
    if (it.private) out.push(pill(b.private, '', h.private));
    if (it.truncated) out.push(pill(b.truncated, '', h.truncated));
    for (const f of it.facts || []) if (f.k === 'disabled' || f.k === 'manual' || f.k === 'standard') out.push(pill(b[f.k] || f.k, 'plain'));
    const dirs = (it.facts || []).find((f) => f.k === 'dirs');
    if (dirs) out.push(pill(tpl(b.dirs, { n: dirs.v }), 'info'));
    if ((it.refsIn || []).length) out.push(pill(tpl(b.refsIn, { n: it.refsIn.length }), 'plain', h.refsIn));
    return out.join('');
  }

  // ---------- верх страницы ----------
  function toolbar(m, shown) {
    const t = T();
    const kinds = m.kinds.filter((s) => s.count > 0).length;
    const meta = [tpl(t.total, { n: m.items.length }), tpl(t.kindsCount, { k: kinds }), tpl(t.built, { ms: m.ms }) + (m.cached ? ` (${t.cached})` : '')];
    return `<div class="ai-top"><p class="note ai-intro">${esc(t.intro)}</p>
      <div class="ai-tools">
        <input type="search" id="ai-q" class="gq ai-q" aria-label="${esc(t.searchLabel)}" placeholder="${esc(t.searchPlaceholder)}" value="${esc(AI.q)}" spellcheck="false" autocomplete="off">
        ${AI.q ? `<button type="button" class="btn ghost sm" data-ai-act="clear">${esc(t.searchClear)}</button>` : ''}
        <span class="note ai-meta">${AI.q ? esc(tpl(t.found, { n: shown })) : esc(meta.join(' · '))}</span>
        <button type="button" class="btn sm ai-re" data-ai-act="rescan" title="${esc(t.rescanHint)}" aria-label="${esc(t.rescanHint)}">${Learn.icon('refresh', 14)}<span>${esc(t.rescan)}</span></button>
      </div>
      ${m.capped ? `<p class="note ai-warn">${esc(tpl(t.capped, { n: m.walked }))}</p>` : ''}</div>`;
  }

  // схема: блоки по слоям (колонка = слой), стрелки - только найденные ссылки между видами
  function archBlock(m) {
    const t = T(), cfg = CFG(), count = new Map(m.kinds.map((s) => [s.id, s]));
    const layers = (cfg.layers || []).filter((l) => l.kinds.some((id) => (count.get(id) || {}).count > 0));
    const col = new Map(layers.map((l, i) => [l.id, i]));
    const tone = (layerId) => { const c = ((cfg.layers || []).find((l) => l.id === layerId) || {}).color; return `var(--${LAYER_TOKENS.includes(c) ? c : 'accent'})`; };
    const nodes = [];
    for (const l of layers) for (const id of l.kinds) {
      const s = count.get(id), k = kindOf(id);
      if (!s || !s.count || !k) continue;
      const note = Object.entries(s.modes).sort((a, b) => b[1] - a[1]).slice(0, 2).map(([mo, n]) => `${n} ${(k.modes[mo] || {}).label || mo}`).join(' · ');
      nodes.push({ id, label: `${k.title} (${s.count})`, note, icon: k.icon, col: col.get(l.id), color: tone(l.id) });
    }
    const ids = new Set(nodes.map((n) => n.id));
    const real = (m.kindEdges || []).filter((e) => ids.has(e.from) && ids.has(e.to));
    // подписей на стрелках нет: на схеме они наезжали друг на друга; число связей - списком под схемой
    const edges = real.map((e) => [e.from, e.to, '']);
    const title = (id) => (kindOf(id) || {}).title || id;
    const rel = real.length ? `<ul class="ai-rels" aria-label="${esc(t.relsLabel)}">${real.map((e) => `<li>${esc(tpl(t.edge, { a: title(e.from), b: title(e.to), n: e.n }))}</li>`).join('')}</ul>` : '';
    const legend = layers.map((l) => `<li class="ai-layer" style="--lc:${tone(l.id)}"><span class="ai-lmark" aria-hidden="true"></span><b>${esc(l.title)}</b><span class="ai-lsay">${esc(l.say)}</span></li>`).join('');
    const body = `<ul class="ai-layers" aria-label="${esc(t.layersTitle)}">${legend}</ul>
      <div class="diagwrap ai-diag">${Learn.diagram({ nodes, edges, title: t.diagramLabel, go: t.go }, 'var(--accent)')}</div>
      ${rel}<p class="cap ai-cap">${esc(edges.length ? t.diagramCaption : t.noEdges)}</p>`;
    return Blocks.card({ title: t.sections.arch, info: info('aiArch'), body });
  }

  // рассказ: шаги только из найденных деталей
  function storyBlock(m) {
    const t = T();
    if (!m.story.length) return '';
    const byId = new Map(m.items.map((it) => [it.id, it]));
    const li = m.story.map((s, i) => {
      const parts = (s.parts || []).slice(0, 6).map((id) => byId.get(id)).filter(Boolean);
      const links = parts.map((it) => `<button type="button" class="chip linkchip ai-go" data-ai-go="${esc(it.id)}" title="${esc(tpl(t.go, { name: it.name }))}" data-ext>${esc(it.name)}</button>`).join('');
      return `<li class="ai-st"><span class="ai-n" aria-hidden="true">${i + 1}</span><div class="ai-stx"><b>${esc(s.title)}</b><span class="ai-sttext">${text(s.text)}</span>${links ? `<span class="ai-parts" aria-label="${esc(t.storyGo)}">${links}</span>` : ''}</div></li>`;
    }).join('');
    return Blocks.card({ title: t.sections.story, info: info('aiStory'), body: `<ol class="ai-story">${li}</ol>` });
  }

  // ---------- блок вида ----------
  function lessonLink(k) {
    const id = k.lesson || CFG().lesson, m = id && topicMeta(id);
    return m ? `<button type="button" class="tlink ai-lesson" data-topic="${esc(id)}">${esc(tpl(T().lesson, { t: m.title }))}</button>` : '';
  }
  function explain(k, stat) {
    const t = T(), w = k.words || {}, L = t.labels || {};
    const modes = Object.entries(stat.modes).map(([mo, n]) => { const d = (k.modes || {})[mo] || {}; return `<span class="ai-mode" title="${esc(d.say || '')}"><b>${n}</b> ${esc(d.label || mo)}</span>`; }).join('');
    const def = (label, v) => (v ? `<div class="ai-def"><dt>${esc(label)}</dt><dd>${text(v)}</dd></div>` : '');
    return `<div class="ai-expl"><p class="ai-what">${text(w.what)}</p>
      <dl class="ai-defs">${def(L.when, w.when)}${def(L.why, w.why)}${learning() ? def(L.analogy, w.analogy) + def(L.look, w.look) : ''}</dl>
      <div class="ai-modes" aria-label="${esc(t.modeChips)}">${modes}</div>${lessonLink(k)}</div>`;
  }
  function details(it, k) {
    const D = T().details, b = T().badges || {}, cfg = CFG();
    const kv = [];
    const row = (label, v) => { if (v) kv.push(`<dt>${esc(label)}</dt><dd>${v}</dd>`); };
    const edit = (rel, line, label) => `<button type="button" class="btn sm" data-edit="${esc(rel)}"${line ? ` data-line="${line}"` : ''} title="${esc(rel)}">${esc(label || D.open)}</button>`;
    const mode = modeOf(k, it);
    if (it.broken) row(D.brokenLabel, `<span class="ai-bad">${esc(it.broken)}</span>`);
    if (it.file) {
      const when = it.mtime ? ` · ${esc(D.mtime)} ${esc(ago(it.mtime))}` : '';
      row(D.file, `<code>${esc(it.file)}${it.line ? ':' + it.line : ''}</code>${it.size ? ` <span class="note">${esc(size(it.size))}${when}</span>` : ''}<span class="ai-acts">${edit(it.file, it.line)}<button type="button" class="btn ghost sm" data-reveal="${esc(it.file)}">${esc(D.reveal)}</button></span>`);
    }
    if (it.event) row(D.event, `<code>${esc(it.event)}</code>`);
    if (it.matcher) row(D.matcher, `<code>${esc(it.matcher)}</code>`);
    if (it.command) row(D.command, `<code class="ai-cmd">${esc(it.command)}</code>`);
    if (it.script) row(D.script, `<code>${esc(it.script)}</code><span class="ai-acts">${edit(it.script, 0, D.open)}</span>`);
    if (it.timeout) row(D.timeout, esc(tpl(D.timeoutSec, { n: it.timeout })));
    if (it.transport && k.format === 'mcp') row(D.transport, `<code>${esc(it.transport)}</code>`);
    if (it.model) row(D.model, `<code>${esc(it.model)}</code>`);
    if (it.tools && it.tools.length) row(D.tools, chips(it.tools));
    if (it.role && k.roles && k.roles[it.role]) row(D.role, esc(k.roles[it.role].label));
    if (it.paths && it.paths.length) {
      const rows = it.paths.map((p) => `<li><code>${esc(p.glob)}</code><span class="${p.files ? 'ai-n0' : 'ai-bad'}">${esc(p.files ? tpl(D.pathsFiles, { n: p.files }) : D.pathsNothing)}</span></li>`).join('');
      row(D.paths, `<ul class="ai-globs">${rows}</ul>`);
    }
    if (it.env && it.env.length) {
      const rows = it.env.map((e) => `<li><code>${esc(e.key)}</code>${e.masked ? `<span class="note">${esc(D.masked)}</span>` : e.value ? `<code>${esc(e.value)}</code>` : `<span class="note">${esc(D.empty)}</span>`}</li>`).join('');
      row(D.env, `<ul class="ai-globs">${rows}</ul>`);
    }
    for (const l of it.lists || []) if (l.items && l.items.length) row((D.lists || {})[l.k] || l.k, chips(l.items.slice(0, 40), l.k === 'deny' ? 'deny' : ''));
    const facts = (it.facts || []).filter((f) => !['disabled', 'manual', 'standard', 'dirs'].includes(f.k) && f.v !== '0' && f.v !== '').map((f) => `${(cfg.facts || {})[f.k] || f.k}: ${f.v}`);
    if (facts.length) row(D.facts, esc(facts.join(' · ')));
    if (it.hits && it.hits.length) {
      const rows = it.hits.map((h) => `<li><button type="button" class="tlink inl" data-edit="${esc(h.file)}" data-line="${h.line}" title="${esc(D.open)}"><code>${esc(h.file)}:${h.line}</code></button><span>${esc(h.text)}</span></li>`).join('');
      row(D.hits, `<ul class="ai-hits">${rows}</ul>`);
    }
    const byId = new Map(AI.map.items.map((x) => [x.id, x]));
    const refs = (ids) => (ids || []).map((id) => byId.get(id)).filter(Boolean).slice(0, 40).map((x) => `<button type="button" class="chip linkchip ai-go" data-ai-go="${esc(x.id)}" title="${esc(tpl(T().go, { name: x.name }))}">${esc(x.name)}</button>`).join('');
    if ((it.refsOut || []).length) row(D.refsOut, `<span class="chips ai-chips">${refs(it.refsOut)}</span>`);
    if ((it.refsIn || []).length) row(D.refsIn, `<span class="chips ai-chips">${refs(it.refsIn)}</span>`);
    const full = it.desc || it.summary;
    return `<div class="ai-det" data-ext>
      ${mode ? `<p class="ai-when"><b>${esc(T().labels.when)}.</b> ${esc(mode.say)}</p>` : ''}
      ${full ? `<p class="ai-full">${esc(full)}</p>` : ''}
      <dl class="ai-kv">${kv.join('')}</dl></div>`;
  }
  function itemRow(it, k) {
    const open = AI.open.has(it.id), D = T().details;
    return `<div class="ai-item${open ? ' open' : ''}" data-ai-id="${esc(it.id)}">
      <div class="ai-row" tabindex="0" role="button" aria-expanded="${open}" data-ai-row="${esc(it.id)}" aria-label="${esc(tpl(open ? T().collapse : T().expand, { name: it.name }))}" data-ext>
        <span class="ai-nm" title="${esc(it.name)}">${esc(it.name)}</span><span class="ai-sm" title="${esc(it.summary)}">${esc(it.summary)}</span>
        <span class="ai-bd">${badgesOf(it, k)}</span><span class="ai-chev" aria-hidden="true">${Learn.icon('chevron', 14, 'currentColor', 2)}</span></div>
      ${open ? details(it, k) : ''}</div>`;
  }
  // список строк; длинный сворачивается до первых SHOWN, пока человек не нажмёт «Показать все» или не ищет
  function rowsOf(list, k, key) {
    const t = T(), all = AI.more.has(key) || !!AI.q || list.length <= SHOWN + 3;
    const view = all ? list : list.slice(0, SHOWN);
    const open = list.filter((it) => AI.open.has(it.id) && !view.includes(it));
    const out = [...view, ...open].map((it) => itemRow(it, k)).join('');
    const more = all ? (AI.more.has(key) && !AI.q && list.length > SHOWN + 3 ? `<button type="button" class="tlink ai-more" data-ai-more="${esc(key)}">${esc(t.less)}</button>` : '') : `<button type="button" class="tlink ai-more" data-ai-more="${esc(key)}">${esc(tpl(t.more, { n: list.length }))}</button>`;
    return `<div class="ai-rows">${out}</div>${more ? `<div class="ai-morerow">${more}</div>` : ''}`;
  }
  // подразделы: роли документов, группы проверок - заголовок и пояснение из ai.json
  function groupsOf(k) {
    const src = k.groups || k.roles;
    if (!src) return null;
    return Object.entries(src).map(([id, v]) => ({ id, title: v.title || v.label, say: v.say }));
  }
  function kindBlock(m, k, stat, items) {
    const t = T(), cap = (s) => String(s || '').charAt(0).toUpperCase() + String(s || '').slice(1);
    let inner = '';
    const groups = groupsOf(k);
    if (groups) {
      for (const gr of groups) {
        const list = items.filter((it) => it.role === gr.id);
        if (!list.length) continue;
        inner += `<div class="ai-group"><h3 class="ai-gh">${esc(cap(gr.title))}<span class="cnt">${list.length}</span></h3><p class="note ai-gsay">${text(gr.say)}</p>${rowsOf(list, k, k.id + ':' + gr.id)}</div>`;
      }
    } else inner = rowsOf(items, k, k.id);
    const purpose = k.purpose && !AI.q
      ? `<div class="ai-purpose"><h3 class="ai-gh">${esc(k.purpose.title)}</h3><div class="ai-pgrid">${k.purpose.items.map((x) => `<div class="ai-pcard"><b>${esc(x.title)}</b><span>${text(x.text)}</span></div>`).join('')}</div></div>` : '';
    const aside = k.id === 'githooks' ? `<span class="note">${esc(m.hooksPath ? tpl(t.hooksPath, { v: m.hooksPath }) : t.hooksPathNone)}</span>` : `<span class="cnt">${items.length}</span>`;
    const title = k.id === 'checks' ? t.sections.checks : k.title;
    const card = Blocks.card({ title, info: k.id === 'checks' ? info('aiChecks') : '', aside, flush: true, body: `${AI.q ? '' : explain(k, stat)}${purpose}${inner}` });
    return `<div class="ai-k" id="ai-k-${esc(k.id)}" tabindex="-1" data-ai-kind="${esc(k.id)}">${card}</div>`;
  }
  function missingBlock(m) {
    const t = T(), none = m.kinds.filter((s) => !s.count).map((s) => kindOf(s.id)).filter(Boolean);
    if (!none.length || AI.q) return '';
    return `<details class="ai-missing"><summary>${esc(t.missingTitle)} <span class="cnt">${none.length}</span></summary><p class="note">${esc(t.missingHint)}</p>
      <ul>${none.map((k) => `<li><b>${esc(k.title)}</b><span class="ai-missing-what">${text(k.words.what)}</span></li>`).join('')}</ul></details>`;
  }
  function problemsBlock(m) {
    const t = T();
    if (!m.problems.length || AI.q) return '';
    const rows = m.problems.map((p) => `<div class="ai-item"><div class="ai-prob" data-ext><code>${esc(p.file)}</code><span class="ai-bad">${esc(p.error)}</span><button type="button" class="btn ghost sm" data-edit="${esc(p.file)}">${esc(t.details.open)}</button></div></div>`).join('');
    return Blocks.card({ title: t.sections.problems, flush: true, body: `<p class="note ai-pnote">${esc(t.problemsText)}</p>${rows}` });
  }

  // ---------- страница ----------
  function html() {
    const t = T(), m = AI.map;
    if (!has()) return `<div class="sc-empty"><b>${esc(t.emptyTitle)}</b><p>${esc(t.emptyText)}</p></div>`;
    if (!m || m.path !== P().path) {
      if (AI.err) return `<div class="sc-empty"><b>${esc(tpl(t.error, { e: AI.err }))}</b><div class="sc-topact"><button type="button" class="btn primary" data-ai-act="retry">${esc(t.retry)}</button></div></div>`;
      return `<p class="note ai-loading" role="status">${esc(t.loading)}</p>`;
    }
    if (!m.items.length) return `<div class="sc-empty"><b>${esc(t.emptyTitle)}</b><p>${esc(t.emptyText)}</p></div>`;
    const match = matcher();
    let shown = 0;
    const blocks = [];
    for (const s of m.kinds) {
      const k = kindOf(s.id);
      if (!k || !s.count) continue;
      const items = m.items.filter((it) => it.kind === s.id && (!match || match(it)));
      if (!items.length) continue;
      shown += items.length;
      blocks.push(kindBlock(m, k, s, items));
    }
    const head = AI.q ? '' : archBlock(m) + storyBlock(m);
    const none = AI.q && !blocks.length ? `<p class="note ai-none" role="status">${esc(t.searchNone)}</p>` : '';
    return `${toolbar(m, shown)}<div class="blk-stack ai">${head}${blocks.join('')}${problemsBlock(m)}</div>${none}${missingBlock(m)}`;
  }

  // ---------- переходы ----------
  function scrollTo(el) {
    if (!el) return;
    el.scrollIntoView({ block: 'start', behavior: reduce() ? 'auto' : 'smooth' });
  }
  function goKind(id) {
    if (AI.q) { AI.q = ''; draw(); }
    const el = document.getElementById('ai-k-' + id);
    if (!el) return;
    scrollTo(el);
    el.focus({ preventScroll: true });
  }
  function goItem(id) {
    const it = AI.map && AI.map.items.find((x) => x.id === id);
    if (!it) return;
    if (AI.q) AI.q = '';
    AI.open.add(id);
    // длинный список раскрывается, иначе строка осталась бы за «Показать все»
    const k = kindOf(it.kind), gr = groupsOf(k);
    AI.more.add(gr ? `${it.kind}:${it.role}` : it.kind);
    draw();
    const el = document.querySelector(`[data-ai-id="${enc(id)}"]`);
    if (!el) return;
    el.scrollIntoView({ block: 'center', behavior: reduce() ? 'auto' : 'smooth' });
    el.querySelector('.ai-row')?.focus({ preventScroll: true });
  }
  function toggle(id) {
    if (AI.open.has(id)) AI.open.delete(id); else AI.open.add(id);
    draw();
  }

  document.addEventListener('click', (e) => {
    if (S.tab !== 'ai' || !e.target.closest('#tabbody')) return;
    const t = e.target;
    const act = t.closest('[data-ai-act]');
    if (act) {
      const a = act.dataset.aiAct;
      if (a === 'clear') { AI.q = ''; draw(); $('#ai-q')?.focus(); }
      else if (a === 'rescan' || a === 'retry') { call('ai_map', { path: P().path, force: true }).then((m) => { AI.map = m; AI.err = ''; draw(); renderTop(); }).catch((x) => { AI.err = String(x); toast(String(x), 'bad'); }); }
      return;
    }
    const more = t.closest('[data-ai-more]');
    if (more) { const key = more.dataset.aiMore; if (AI.more.has(key)) AI.more.delete(key); else AI.more.add(key); draw(); return; }
    const go = t.closest('[data-ai-go]');
    if (go) { goItem(go.dataset.aiGo); return; }
    const node = t.closest('[data-node]');
    if (node) { goKind(node.dataset.node); return; }
    if (t.closest('button, a, input, .ai-det')) return;
    const row = t.closest('[data-ai-row]');
    if (row) toggle(row.dataset.aiRow);
  });
  // блок схемы - ссылка внутри svg: Enter и пробел нажимают его сами (у svg-элемента нет click())
  document.addEventListener('keydown', (e) => {
    if (S.tab !== 'ai') return;
    const n = e.target.closest?.('[data-node]');
    if (n && (e.key === 'Enter' || e.key === ' ') && !e.ctrlKey && !e.altKey && !e.metaKey) {
      e.preventDefault(); e.stopPropagation();
      goKind(n.dataset.node);
      return;
    }
    if (e.key === 'Escape' && e.target.id === 'ai-q' && AI.q) { e.preventDefault(); e.stopPropagation(); AI.q = ''; draw(); }
  }, true);
  document.addEventListener('input', (e) => {
    if (e.target.id !== 'ai-q' || S.tab !== 'ai') return;
    AI.q = e.target.value;
    draw();
  });

  window.AiTab = { renderTab, onTab, badge, has };
})();
