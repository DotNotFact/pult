'use strict';
// Вкладка «Сценарии»: свои цепочки команд проекта. Ядро хранит и запускает их (app/src/tray.rs),
// здесь - список, живой ход по шагам и редактор. Тексты - из базы (knowledge/ui.json, раздел scenarios).
// Подключается после app.js и берёт его общие помощники: S, call, listen, esc, tpl, UI, P и другие.
(() => {
  // стрелки и крестик - своими значками: шрифтовые ↑ ↓ ✕ Windows рисует цветными эмодзи
  const svg = (d) => `<svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"><path d="${d}" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
  const UP = svg('M6 10V2M2.5 5.5 6 2l3.5 3.5'), DOWN = svg('M6 2v8M2.5 6.5 6 10l3.5-3.5'), CROSS = svg('M3 3l6 6M9 3l-6 6');
  const T = () => UI().scenarios || {};
  const SC = {
    list: [], loaded: false,
    // id -> { status, step, total, cur, runId, steps: [статус шага], runs: { runId: шаг }, started, ended, msg }
    state: new Map(),
    edit: null, // открытый редактор: { id, name, steps: [{scriptId, name, args}], stopOnError, inTray, q, path }
    // Закрытые уходом на другую вкладку черновики: ключ - путь проекта и id сценария ('' у нового).
    // Панель закрывается, правки не теряются: «Изменить» или «Новый» открывают черновик с того же места.
    drafts: new Map(),
    suggest: null, // открытая панель популярных сценариев: [{ id, name, why, steps, on, exists }]
  };
  const norm = (p) => String(p || '').replace(/\//g, '\\').replace(/\\+$/, '').toLowerCase();
  const mine = () => (P() ? SC.list.filter((s) => norm(s.project) === norm(P().path)) : []);
  const stepWord = (n) => { const f = T().stepForms || []; return `${n} ${plural(n, f[0] || '', f[1] || '', f[2] || '')}`; };
  const stOf = (id) => SC.state.get(id);
  const isRunning = (id) => stOf(id)?.status === 'running';

  // ---------- вкладка в общем списке ----------
  // Подпись читается при каждой отрисовке: база знаний приходит позже, чем грузится этот файл.
  const entry = ['scenarios', '', ''];
  Object.defineProperty(entry, 1, { get: () => T().tab || 'scenarios' });
  Object.defineProperty(entry, 2, { get: () => T().tabShort || T().tab || 'scenarios' });
  if (!TABS.some(([id]) => id === 'scenarios')) TABS.push(entry);
  NAV_ICON.scenarios = 'bolt';

  async function load() {
    try { SC.list = await call('scenarios'); SC.loaded = true; } catch (e) { toast(String(e), 'bad'); }
    try {
      for (const id of await call('scenarios_running')) if (!isRunning(id)) SC.state.set(id, { status: 'running', step: -1, total: 0, steps: [], runs: {}, started: Date.now() });
    } catch { /* не критично */ }
    refresh();
  }

  // перерисовать то, что показывает сценарии: список на вкладке и виджет
  function refresh() {
    if (S.tab === 'scenarios' && P() && $('#sc-list')) renderList();
    window.Widget?.render();
  }

  // ---------- вкладка ----------
  const draftKey = (id) => norm(P()?.path) + '|' + id;
  // уход с вкладки (и смена проекта) закрывает редактор, а правки откладывает в черновик
  function park() {
    if (!SC.edit) return;
    SC.drafts.set(norm(SC.edit.path) + '|' + SC.edit.id, SC.edit);
    SC.edit = null;
  }
  function onTab(t) { if (t !== 'scenarios') { park(); SC.suggest = null; } }
  function renderTab(el) {
    if (SC.edit && norm(SC.edit.path) !== norm(P()?.path)) park();
    if (!SC.loaded) load();
    const t = T();
    el.innerHTML = `<div class="sc-top"><p class="note">${esc(t.intro)}</p><span class="sc-topact" id="sc-topact"></span></div>
      <div id="sc-edit"></div><div id="sc-list"></div>`;
    renderEditor();
    renderList();
  }

  // отложенные черновики этого проекта - кнопкой «Вернуться к черновику»
  function drafts() {
    const pre = norm(P()?.path) + '|';
    return [...SC.drafts].filter(([k]) => k.startsWith(pre)).map(([, d]) => `<button class="btn" data-scact="draft" data-id="${esc(d.id)}">${esc(tpl(T().resumeDraft, { name: d.name || T().newTitle }))}</button>`).join('');
  }
  function chain(s, st) {
    return `<ol class="sc-chain">${s.steps.map((x, i) => {
      const known = scriptOf(x.scriptId) || P()?.scripts.find((y) => y.name === x.name);
      const k = st?.steps?.[i] || (st?.status === 'running' && st.step === i ? 'running' : '');
      return `<li class="sc-link ${k ? 'k-' + esc(k) : ''} ${known ? '' : 'miss'}" title="${esc(known ? known.cmd : T().missing)}${x.args ? ' ' + esc(x.args) : ''}">${k ? sdot(k) : `<span class="n">${i + 1}</span>`}<span class="nm">${esc(x.name)}</span>${x.args ? `<span class="ar">${esc(x.args)}</span>` : ''}</li>`;
    }).join('')}</ol>`;
  }

  function progress(s, st) {
    if (!st) return '';
    const t = T(), RUN = t.status || {};
    let text = '';
    if (st.status === 'running') text = st.step >= 0 ? `${tpl(t.progress, { k: st.step + 1, n: st.total || s.steps.length })} · ${st.cur || ''}` : RUN.running || '';
    else if (st.status === 'ok') text = tpl(t.doneIn, { t: dur((st.ended || Date.now()) - st.started) });
    else text = st.msg || RUN[st.status] || st.status;
    const total = st.total || s.steps.length || 1;
    const done = st.status === 'running' ? Math.max(0, st.step) : st.status === 'ok' ? total : (st.steps || []).filter((x) => x === 'ok' || x === 'fail').length;
    const pct = Math.round((Math.min(done, total) / total) * 100);
    const runId = st.runId && S.runs.has(st.runId) ? st.runId : '';
    // сколько ещё: сумма оценок всех шагов; хотя бы у одного шага нет истории - оценки нет
    const est = st.status === 'running' ? s.steps.reduce((sum, x) => { const id = (scriptOf(x.scriptId) || P()?.scripts.find((y) => y.name === x.name))?.id; const e = scriptEta(id, x.args || ''); return sum == null || e == null ? null : sum + e; }, 0) : null;
    return `<div class="sc-prog p-${esc(st.status)}"><span class="dot s-${esc(st.status)}"></span><span class="tx">${esc(text)}</span>${etaLabel(st.started, est)}
      ${runId ? `<button class="tlink" data-crun="${esc(runId)}" title="${esc(t.logHint)}">${esc(t.log)}</button>` : ''}
      <span class="bar" aria-hidden="true"><i style="width:${pct}%"></i></span></div>`;
  }

  function card(s) {
    const t = T(), st = stOf(s.id), run = st?.status === 'running';
    return `<div class="sc-card ${run ? 'on' : ''} ${SC.edit?.id === s.id ? 'editing' : ''}" data-scid="${esc(s.id)}">
      <div class="sc-h">
        ${run ? `<button class="run stop" data-scstop="${esc(s.id)}" title="${esc(t.stop)}" aria-label="${esc(t.stop)} ${esc(s.name)}">■</button>`
          : `<button class="run" data-scrun="${esc(s.id)}" title="${esc(t.run)}" aria-label="${esc(t.run)} ${esc(s.name)}" ${s.steps.length ? '' : 'disabled'}>▶</button>`}
        <div class="sc-t"><b>${esc(s.name)}</b><small>${esc(stepWord(s.steps.length))}${s.stopOnError ? ` · ${esc(t.stopOnError)}` : ''}${s.inTray ? ` · ${esc(t.inTray)}` : ''}</small></div>
        <div class="sc-acts"><button class="btn ghost sm" data-scact="edit" data-id="${esc(s.id)}">${esc(t.edit)}</button><button class="btn ghost sm danger" data-scact="del" data-id="${esc(s.id)}">${esc(t.delete)}</button></div>
      </div>
      ${chain(s, st)}
      ${progress(s, st)}
    </div>`;
  }

  function renderList() {
    const el = $('#sc-list'); if (!el) return;
    const t = T(), list = mine();
    // кнопки сверху - только рядом со списком: у пустой вкладки те же кнопки стоят в её середине
    const top = $('#sc-topact');
    if (top) top.innerHTML = SC.edit || SC.suggest ? '' : drafts() + (list.length ? `<button class="btn" data-scact="suggest" title="${esc(t.popularHint)}">${esc(t.popular)}</button><button class="btn primary" data-scact="new">+ ${esc(t.create)}</button>` : '');
    if (!list.length) {
      if (SC.edit || SC.suggest) { el.innerHTML = ''; return; }
      const ex = (t.exampleSteps || []).map((x) => `<li class="sc-link"><span class="nm">${esc(x)}</span></li>`).join('');
      el.innerHTML = `<div class="sc-empty"><span class="sc-ico">${Learn.icon('bolt', 26)}</span><b>${esc(t.emptyTitle)}</b><p>${esc(t.emptyText)}</p>
        <div class="sc-example"><span class="sc-exname">${esc(t.exampleName)}:</span><ol class="sc-chain">${ex}</ol></div>
        <div class="sc-topact"><button class="btn" data-scact="suggest" title="${esc(t.popularHint)}">${esc(t.popular)}</button><button class="btn primary" data-scact="new">${esc(t.createFirst)}</button></div></div>`;
      return;
    }
    el.innerHTML = `<div class="sc-cards">${list.map(card).join('')}</div>`;
  }

  // ---------- редактор ----------
  function startEdit(id) {
    const s = id ? SC.list.find((x) => x.id === id) : null;
    const key = draftKey(id || ''), kept = SC.drafts.get(key);
    SC.drafts.delete(key);
    SC.suggest = null;
    SC.edit = kept || (s ? { ...s, steps: s.steps.map((x) => ({ ...x })), q: '' } : { id: '', name: '', steps: [], stopOnError: true, inTray: true, q: '' });
    SC.edit.path = P().path;
    renderTab($('#tabbody'));
    setTimeout(() => { $('#sc-name')?.focus(); $('#sc-edit')?.scrollIntoView({ block: 'nearest' }); }, 0);
  }
  function renderEditor() {
    const el = $('#sc-edit'); if (!el) return;
    const d = SC.edit, t = T();
    if (SC.suggest) { renderSuggest(el); return; }
    if (!d) { el.innerHTML = ''; return; }
    el.innerHTML = `<div class="card sc-editor"><h4>${esc(d.id ? t.editTitle : t.newTitle)}</h4>
      <label class="sc-f"><span>${esc(t.name)}</span><input type="text" id="sc-name" value="${esc(d.name)}" placeholder="${esc(t.namePh)}" autocomplete="off" maxlength="80"></label>
      <div class="sc-f"><span>${esc(t.steps)}</span><ol class="sc-steps" id="sc-steps"></ol></div>
      <div class="sc-f"><span>${esc(t.add)}</span><div class="sc-add"><input type="search" id="sc-q" value="${esc(d.q)}" placeholder="${esc(t.searchPh)}" autocomplete="off"><div class="sc-pick" id="sc-pick"></div></div></div>
      <div class="sc-sw">
        <label class="set"><input type="checkbox" id="sc-soe" ${d.stopOnError ? 'checked' : ''}><span>${esc(t.stopOnError)}<small>${esc(t.stopOnErrorText)}</small></span></label>
        <label class="set"><input type="checkbox" id="sc-tray" ${d.inTray ? 'checked' : ''}><span>${esc(t.inTray)}<small>${esc(t.inTrayText)}</small></span></label>
      </div>
      <div class="sc-foot"><button class="btn" data-scact="cancel">${esc(t.cancel)}</button><button class="btn primary" data-scact="save">${esc(t.save)}</button></div></div>`;
    renderSteps();
    renderPick();
  }
  function renderSteps() {
    const el = $('#sc-steps'); if (!el || !SC.edit) return;
    const t = T(), steps = SC.edit.steps;
    el.innerHTML = steps.length ? steps.map((x, i) => {
      const s = scriptOf(x.scriptId) || P()?.scripts.find((y) => y.name === x.name);
      return `<li class="sc-step ${s ? '' : 'miss'}"><span class="n">${i + 1}</span>
        <span class="sc-sm"><b class="mono">${esc(x.name)}</b><small>${esc(s ? s.info?.summary || s.cmd : t.missing)}</small></span>
        <input type="text" class="sc-args" data-sci="${i}" aria-label="${esc(A('stepArgs', { n: i + 1 }))}" spellcheck="false" value="${esc(x.args)}" placeholder="${esc(t.argsPh)}" title="${esc(t.argsHint)}" autocomplete="off">
        <span class="sc-sb"><button class="btn ghost icon sm" data-scmv="${i}" data-dir="-1" title="${esc(t.up)}" aria-label="${esc(t.up)}" ${i ? '' : 'disabled'}>${UP}</button><button class="btn ghost icon sm" data-scmv="${i}" data-dir="1" title="${esc(t.down)}" aria-label="${esc(t.down)}" ${i < steps.length - 1 ? '' : 'disabled'}>${DOWN}</button><button class="btn ghost icon sm danger" data-scrm="${i}" title="${esc(t.remove)}" aria-label="${esc(t.remove)}">${CROSS}</button></span></li>`;
    }).join('') : `<li class="sc-none">${esc(t.noStepsYet)}</li>`;
  }
  // ---------- популярность скриптов: что предлагать первым ----------
  // Избранное, сколько раз запускали (свежие запуски весят больше), совпадение с шаблонами быстрого
  // запуска (ui.json, quick.names) и вес этапа: разработку и проверку зовут чаще, чем релиз.
  const STAGE_W = { dev: 6, check: 5, build: 4, setup: 3, ops: 2, release: 1 };
  function runStats() {
    const m = new Map(), now = Date.now(), seen = new Set(), here = norm(P()?.path);
    const add = (r) => {
      if (!r?.scriptId || seen.has(r.id) || (r.project && norm(r.project) !== here)) return;
      seen.add(r.id);
      const x = m.get(r.scriptId) || { n: 0, w: 0 };
      x.n++; x.w += 1 + 2 / (1 + Math.max(0, now - (r.startedAt || 0)) / 86400000);
      m.set(r.scriptId, x);
    };
    for (const r of S.runs.values()) if (r.kind === 'script') add(r);
    for (const r of S.hist || []) add(r);
    return m;
  }
  function popularity() {
    const fav = new Set(P().favorites || []), st = runStats(), out = new Map();
    const qr = ((UI().quick || {}).names || []).map((p) => { try { return new RegExp(p); } catch { return null; } }).filter(Boolean);
    for (const s of P().scripts) {
      const r = st.get(s.id) || { n: 0, w: 0 }, f = fav.has(s.id), rec = qr.some((re) => re.test(s.name));
      out.set(s.id, { fav: f, rec, n: r.n, often: r.n >= 2, popular: f || rec || r.n >= 2,
        score: (f ? 100 : 0) + r.w * 10 + (rec ? 30 : 0) + (STAGE_W[s.stage] || 0) });
    }
    return out;
  }
  function badges(x) {
    const t = T(), b = [];
    if (x.fav) b.push(`<span class="sc-bdg fav" title="${esc(t.badgeFavHint)}">${esc(t.badgeFav)}</span>`);
    if (x.often) b.push(`<span class="sc-bdg often" title="${esc(tpl(t.badgeOftenHint, { n: x.n }))}">${esc(t.badgeOften)}</span>`);
    if (x.rec) b.push(`<span class="sc-bdg rec" title="${esc(t.badgeRecHint)}">${esc(t.badgeRec)}</span>`);
    return b.join('');
  }
  function renderPick() {
    const el = $('#sc-pick'); if (!el || !SC.edit || !P()) return;
    const t = T(), raw = SC.edit.q.trim().toLowerCase(), q = raw.split(/\s+/).filter(Boolean);
    const pop = popularity();
    // поиск: выше - совпадение в имени, потом в команде, потом в пояснении; среди равных - популярные
    const score = (s) => {
      const n = s.name.toLowerCase();
      if (!q.length) return 1;
      if (n.includes(raw)) return 4;
      if (q.every((w) => n.includes(w))) return 3;
      if (q.every((w) => `${n} ${s.cmd.toLowerCase()}`.includes(w))) return 2;
      return q.every((w) => `${n} ${s.cmd} ${s.info?.summary || ''} ${s.where}`.toLowerCase().includes(w)) ? 1 : 0;
    };
    const hits = P().scripts.map((s) => [s, score(s)]).filter(([, k]) => k > 0)
      .sort((a, b) => b[1] - a[1] || pop.get(b[0].id).score - pop.get(a[0].id).score).map(([s]) => s);
    const opt = (s) => `<button class="sc-opt" data-scadd="${esc(s.id)}" title="${esc(s.cmd)}"><span class="plus" aria-hidden="true">+</span><span class="sc-sm"><b class="mono">${esc(s.name)}</b><small>${esc(s.info?.summary || s.cmd)}</small></span><span class="sc-bdgs">${badges(pop.get(s.id))}${s.where !== '.' ? `<span class="tag">${esc(s.where)}</span>` : ''}</span></button>`;
    if (!hits.length) { el.innerHTML = `<div class="sc-none">${esc(t.nothing)}</div>`; return; }
    if (q.length) { el.innerHTML = hits.slice(0, 60).map(opt).join(''); return; }
    // без поиска: сначала часто нужные, потом остальные в порядке проекта
    const top = hits.filter((s) => pop.get(s.id).popular);
    const rest = P().scripts.filter((s) => !pop.get(s.id).popular);
    el.innerHTML = (top.length ? `<div class="sc-grp">${esc(t.popularGroup)}</div>${top.map(opt).join('')}${rest.length ? `<div class="sc-grp">${esc(t.othersGroup)}</div>` : ''}` : '') + rest.slice(0, 200).map(opt).join('');
  }

  // ---------- популярные сценарии из knowledge/scenarios.json ----------
  // Шаг находит скрипт по этапу и имени; из подходящих - избранный, потом чаще запускаемый,
  // потом тот, что выше в быстром запуске, потом из корня проекта и с коротким именем.
  function bestFor(step, used, pop, ql) {
    let re = null;
    if (step.name) { try { re = new RegExp(step.name, 'i'); } catch { return null; } }
    const c = P().scripts.filter((s) => !used.has(s.id) && (!step.stage || s.stage === step.stage) && (!re || re.test(s.name)));
    const qi = (s) => { const i = ql.indexOf(s); return i < 0 ? 999 : i; };
    c.sort((a, b) => {
      const pa = pop.get(a.id), pb = pop.get(b.id);
      return (pb.fav - pa.fav) || (pb.n - pa.n) || (qi(a) - qi(b)) || ((a.where === '.' ? 0 : 1) - (b.where === '.' ? 0 : 1)) || a.name.length - b.name.length;
    });
    return c[0] || null;
  }
  function suggest() {
    const pop = popularity(), ql = quickList(), have = new Set(mine().map((s) => s.name.trim().toLowerCase()));
    const out = [];
    for (const tp of (K().scenarios || {}).templates || []) {
      const used = new Set(), steps = [];
      let ok = true;
      for (const st of tp.steps || []) {
        const s = bestFor(st, used, pop, ql);
        if (s) { used.add(s.id); steps.push({ scriptId: s.id, name: s.name, args: '' }); } else if (!st.optional) { ok = false; break; }
      }
      if (!ok || steps.length < 2) continue;
      const exists = have.has(String(tp.name).trim().toLowerCase());
      out.push({ id: tp.id, name: tp.name, why: tp.why, steps, on: !exists, exists });
    }
    return out;
  }
  const ready = (x) => x.on && !x.exists && x.steps.length >= 2;
  function openSuggest() {
    park();
    SC.suggest = suggest();
    renderTab($('#tabbody'));
    setTimeout(() => $('#sc-edit')?.scrollIntoView({ block: 'nearest' }), 0);
  }
  function renderSuggest(el) {
    const t = T(), list = SC.suggest, n = list.filter(ready).length;
    el.innerHTML = `<div class="card sc-editor sc-sug"><h4>${esc(t.suggestTitle)}</h4>
      ${list.length ? `<p class="note sc-sugnote">${esc(t.suggestText)}</p><div class="sc-sglist">${list.map((x, i) => `<div class="sc-sg ${ready(x) ? '' : 'off'}">
        <label class="set"><input type="checkbox" data-sgon="${i}" ${x.on && !x.exists ? 'checked' : ''} ${x.exists || x.steps.length < 2 ? 'disabled' : ''}><span><b>${esc(x.name)}</b>${x.exists ? ` <span class="tag">${esc(t.suggestExists)}</span>` : x.steps.length < 2 ? ` <span class="tag">${esc(t.suggestFew)}</span>` : ''}<small>${esc(x.why)}</small></span></label>
        <ol class="sc-chain">${x.steps.map((s, j) => `<li class="sc-link"><span class="n">${j + 1}</span><span class="nm" title="${esc(scriptOf(s.scriptId)?.cmd || '')}">${esc(s.name)}</span>${scriptOf(s.scriptId)?.where && scriptOf(s.scriptId).where !== '.' ? `<span class="ar">${esc(scriptOf(s.scriptId).where)}</span>` : ''}<button class="sc-x" data-sgrm="${i}:${j}" title="${esc(t.remove)}" aria-label="${esc(t.remove)} ${esc(s.name)}" ${x.exists ? 'disabled' : ''}>${CROSS}</button></li>`).join('')}</ol></div>`).join('')}</div>`
        : `<p class="note">${esc(t.suggestNone)}</p>`}
      <div class="sc-foot"><button class="btn" data-scact="sg-cancel">${esc(t.cancel)}</button>${list.length
        ? `<button class="btn primary" data-scact="sg-create" ${n ? '' : 'disabled'}>${esc(tpl(t.suggestCreate, { n }))}</button>`
        // готовых нет - текст зовёт собрать свой, и кнопка для этого стоит здесь же, а не за «Отменой»
        : `<button class="btn primary" data-scact="new">+ ${esc(t.create)}</button>`}</div></div>`;
  }
  async function createSuggested() {
    const t = T(), pick = (SC.suggest || []).filter(ready), have = new Set(mine().map((s) => s.name.trim().toLowerCase()));
    let n = 0;
    const skipped = [];
    for (const x of pick) {
      if (have.has(x.name.trim().toLowerCase())) { skipped.push(x.name); continue; }
      try {
        const saved = await call('scenario_save', { scenario: { id: '', name: x.name, project: P().path, steps: x.steps, stopOnError: true, inTray: true } });
        SC.list.push(saved); have.add(x.name.trim().toLowerCase()); n++;
      } catch (e) { toast(String(e), 'bad'); }
    }
    SC.suggest = null;
    if (n) toast(tpl(t.suggestCreated, { n }), 'ok', null, 2200);
    if (skipped.length) toast(tpl(t.suggestSkipped, { names: skipped.join(', ') }), '', null, 4000);
    renderTab($('#tabbody'));
    window.Widget?.render();
  }

  async function save() {
    const d = SC.edit, tr = UI().tray || {};
    if (!d || !P()) return;
    d.name = ($('#sc-name')?.value ?? d.name).trim();
    if (!d.name) { toast(tr.noName || '', 'bad'); $('#sc-name')?.focus(); return; }
    if (!d.steps.length) { toast(tr.noSteps || '', 'bad'); $('#sc-q')?.focus(); return; }
    const scenario = { id: d.id, name: d.name, project: d.id ? (SC.list.find((x) => x.id === d.id)?.project || P().path) : P().path,
      steps: d.steps.map((x) => ({ scriptId: x.scriptId, name: x.name, args: String(x.args || '').trim() })), stopOnError: !!d.stopOnError, inTray: !!d.inTray };
    try {
      const saved = await call('scenario_save', { scenario });
      const i = SC.list.findIndex((x) => x.id === saved.id);
      if (i >= 0) SC.list[i] = saved; else SC.list.push(saved);
      // прежний ход относится к старому набору шагов: после правки его не показываем
      if (!isRunning(saved.id)) SC.state.delete(saved.id);
      SC.edit = null;
      toast(T().saved, 'ok', null, 1600);
      renderTab($('#tabbody'));
      window.Widget?.render();
    } catch (e) { toast(String(e), 'bad'); }
  }
  async function del(id) {
    const s = SC.list.find((x) => x.id === id); if (!s) return;
    const t = T();
    const ok = await confirmBox({ title: tpl(t.deleteTitle, { name: s.name }), body: `<p>${esc(t.deleteBody)}</p>`, ok: t.delete, danger: true });
    if (!ok) return;
    try {
      // удалённый сценарий не доигрывает в фоне: идущий шаг и следующие останавливаются
      if (isRunning(id)) await call('scenario_stop', { id }).catch(() => {});
      await call('scenario_delete', { id });
      SC.list = SC.list.filter((x) => x.id !== id);
      SC.state.delete(id);
      SC.drafts.delete(draftKey(id));
      if (SC.edit?.id === id) SC.edit = null;
      toast(t.deleted, '', null, 1600);
      if (S.tab === 'scenarios' && $('#tabbody')) renderTab($('#tabbody'));
      window.Widget?.render();
    } catch (e) { toast(String(e), 'bad'); }
  }
  async function run(id) {
    const s = SC.list.find((x) => x.id === id); if (!s) return;
    SC.state.set(id, { status: 'running', step: -1, total: s.steps.length, steps: [], runs: {}, started: Date.now() });
    refresh();
    try { await call('scenario_run', { id }); } catch (e) { SC.state.delete(id); refresh(); toast(String(e), 'bad'); }
  }
  async function stop(id) { try { await call('scenario_stop', { id }); } catch (e) { toast(String(e), 'bad'); } }

  // ---------- события ядра ----------
  function onScenario(ev) {
    let st = stOf(ev.id);
    if (ev.status === 'running') {
      // первый шаг нового запуска (в том числе из трея) - начинаем ход заново
      if (!st || st.status !== 'running' || (ev.step === 0 && st.step > 0)) st = { status: 'running', step: -1, total: ev.total, steps: [], runs: {}, started: Date.now() };
      st.status = 'running'; st.step = ev.step; st.total = ev.total; st.cur = ev.msg;
      if (ev.runId) { st.runId = ev.runId; st.runs[ev.runId] = ev.step; }
      const r = ev.runId && S.runs.get(ev.runId);
      st.steps[ev.step] = r && r.status !== 'running' ? r.status : 'running';
    } else {
      st ||= { steps: [], runs: {}, started: Date.now() };
      // шаг, на котором оборвалось, получает итог сценария; ход до него уже записан событиями run
      const cur = st.step;
      if (cur >= 0 && st.steps[cur] === 'running') st.steps[cur] = ev.status === 'ok' ? 'ok' : ev.status;
      st.status = ev.status; st.msg = ev.msg; st.total = ev.total; st.ended = Date.now();
      const t = T();
      toast(tpl(t.finished, { name: ev.name, status: ev.status === 'ok' ? tpl(t.doneIn, { t: dur(st.ended - st.started) }) : ev.status === 'stopped' ? (t.status || {}).stopped : ev.msg || (t.status || {})[ev.status] || ev.status }),
        ev.status === 'fail' ? 'bad' : ev.status === 'ok' ? 'ok' : '', st.runId && S.runs.has(st.runId) ? { label: String(t.log || '').replace(/^./, (c) => c.toUpperCase()), fn: () => activate(st.runId) } : null);
    }
    SC.state.set(ev.id, st);
    refresh();
  }
  function onRun(r) {
    let hit = false;
    for (const st of SC.state.values()) {
      const i = st.runs?.[r.id];
      if (i == null) continue;
      st.steps[i] = r.status;
      hit = true;
    }
    // отрисовка после того, как app.js положил запуск в S.runs (его обработчик подписан позже)
    if (hit) setTimeout(refresh, 0);
  }
  listen('scenario', onScenario);
  listen('run', onRun);

  // ---------- клики и ввод ----------
  document.addEventListener('click', (e) => {
    const t = e.target.closest('[data-scact],[data-scrun],[data-scstop],[data-scadd],[data-scmv],[data-scrm],[data-sgrm]');
    if (!t || t.disabled) return;
    const d = t.dataset;
    if (d.scrun) return run(d.scrun);
    if (d.scstop) return stop(d.scstop);
    if (d.scadd && SC.edit) {
      const s = scriptOf(d.scadd); if (!s) return;
      SC.edit.steps.push({ scriptId: s.id, name: s.name, args: '' });
      renderSteps();
      return;
    }
    if (d.scmv && SC.edit) {
      const i = +d.scmv, j = i + +d.dir, a = SC.edit.steps;
      if (j < 0 || j >= a.length) return;
      [a[i], a[j]] = [a[j], a[i]];
      renderSteps();
      $(`[data-scmv="${j}"][data-dir="${d.dir}"]`)?.focus();
      return;
    }
    if (d.sgrm && SC.suggest) {
      const [i, j] = d.sgrm.split(':').map(Number), x = SC.suggest[i];
      if (x) { x.steps.splice(j, 1); renderEditor(); }
      return;
    }
    if (d.scrm != null && SC.edit) { SC.edit.steps.splice(+d.scrm, 1); renderSteps(); return; }
    switch (d.scact) {
      case 'new': startEdit(null); break;
      case 'edit': startEdit(d.id); break;
      case 'del': del(d.id); break;
      case 'save': save(); break;
      case 'cancel': SC.edit = null; renderTab($('#tabbody')); break;
      case 'draft': startEdit(d.id || null); break;
      case 'suggest': openSuggest(); break;
      case 'sg-cancel': SC.suggest = null; renderTab($('#tabbody')); break;
      case 'sg-create': createSuggested(); break;
    }
  });
  document.addEventListener('input', (e) => {
    const el = e.target;
    if (!SC.edit) return;
    if (el.id === 'sc-name') SC.edit.name = el.value;
    else if (el.id === 'sc-q') { SC.edit.q = el.value; renderPick(); }
    else if (el.classList.contains('sc-args')) { const s = SC.edit.steps[+el.dataset.sci]; if (s) s.args = el.value; }
  });
  document.addEventListener('change', (e) => {
    const on = e.target.dataset?.sgon;
    if (on != null && SC.suggest?.[+on]) { SC.suggest[+on].on = e.target.checked; renderEditor(); return; }
    if (!SC.edit) return;
    if (e.target.id === 'sc-soe') SC.edit.stopOnError = e.target.checked;
    if (e.target.id === 'sc-tray') SC.edit.inTray = e.target.checked;
  });
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Enter' || !SC.edit) return;
    if (e.target.id === 'sc-q') { e.preventDefault(); $('#sc-pick .sc-opt')?.click(); }
    else if (e.target.id === 'sc-name' || e.target.classList?.contains('sc-args')) { e.preventDefault(); save(); }
  });

  load();
  window.Scen = { renderTab, onTab, forProject: mine, state: stOf, isRunning, run, stop, stepWord, load };
})();
