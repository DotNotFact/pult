'use strict';
// Уроки и подсказки к терминам. Всё содержимое - из knowledge/topics/*.json и glossary.json,
// здесь только то, как это нарисовать.
(function () {
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const escRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  let ICONS = {};
  let GLOSS = null;
  let NAV = {}; // подписи полосы разделов: knowledge/ui.json, lessonNav
  const quiz = new Map(); // ответы на вопросы: «урок:секция:вопрос» -> номер ответа
  const cheatSel = new Map(); // выбранная группа шпаргалки: «урок:секция» -> имя группы

  // Чернила на цвете урока: тёмные или белые - что контрастнее (WCAG). Цвет урока задаёт база, а жёлтый
  // или светло-зелёный под белыми цифрами не читается; поэтому выбор - здесь, по яркости, а не в данных.
  const INK_DARK = '#0f1117', INK_LIGHT = '#ffffff';
  function lum(hex) {
    let h = String(hex || '').trim().replace(/^#/, '');
    if (h.length === 3) h = h.split('').map((x) => x + x).join('');
    if (!/^[0-9a-f]{6}$/i.test(h)) return null;
    const ch = (i) => { const v = parseInt(h.slice(i, i + 2), 16) / 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
    return 0.2126 * ch(0) + 0.7152 * ch(2) + 0.0722 * ch(4);
  }
  const ratio = (a, b) => (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
  function ink(color) {
    const l = lum(color);
    if (l == null) return INK_LIGHT;
    return ratio(l, lum(INK_DARK)) > ratio(l, 1) ? INK_DARK : INK_LIGHT;
  }
  function icon(name, size = 18, color = 'currentColor', sw = 1.8) {
    const d = ICONS[name] || ICONS.book || '';
    return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="${esc(color)}" stroke-width="${sw}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${esc(d)}"/></svg>`;
  }

  // ---------- подсказки к терминам ----------
  function setKnowledge(kb) {
    ICONS = kb.icons || {};
    NAV = (kb.ui || {}).lessonNav || {};
    const entries = [];
    const add = (form, item) => { const f = String(form || '').trim().toLowerCase(); if (f.length >= 2) entries.push([f, item]); };
    // сначала общий глоссарий (его тексты короче и точнее), потом термины из уроков
    for (const g of [...(Array.isArray(kb.glossary) ? kb.glossary : []), ...(kb.topicTerms || [])]) for (const f of [g.term, ...(g.forms || [])]) add(f, { term: g.term, text: g.text, topic: g.topic });
    const seen = new Set();
    const uniq = entries.filter(([f]) => (seen.has(f) ? false : seen.add(f))).sort((a, b) => b[0].length - a[0].length);
    if (!uniq.length) { GLOSS = null; return; }
    GLOSS = {
      re: new RegExp(`(?<![\\p{L}\\p{N}])(${uniq.map(([f]) => escRe(f)).join('|')})(?![\\p{L}\\p{N}])`, 'giu'),
      map: new Map(uniq),
    };
  }
  /** html - уже экранированный текст. Подчёркивает термины (каждый - один раз на блок). */
  function gloss(html, used = new Set()) {
    if (!GLOSS || !html) return html;
    return html.split(/(<[^>]+>)/).map((part) => {
      if (part.startsWith('<')) return part;
      return part.replace(GLOSS.re, (m) => {
        const g = GLOSS.map.get(m.toLowerCase());
        if (!g || used.has(g.term)) return m;
        used.add(g.term);
        return `<span class="term" data-tip="${esc(g.text)}" data-topic="${esc(g.topic || '')}">${m}</span>`;
      });
    }).join('');
  }

  // ---------- схема: блоки и стрелки ----------
  let diagSeq = 0;
  function diagram(s, color) {
    // повтор id - второй блок встал бы на место первого; берётся первый
    const nodes = (s.nodes || []).filter((n, i, all) => n && all.findIndex((m) => m && m.id === n.id) === i);
    if (!nodes.length) return '';
    const idx = new Map(nodes.map((n, i) => [n.id, i]));
    // подпись из одних пробелов - не подпись
    const edges = (s.edges || []).filter((e) => idx.has(e[0]) && idx.has(e[1]) && e[0] !== e[1]).map((e) => [e[0], e[1], String(e[2] ?? '').trim()]);
    // Обратная стрелка - та, что замыкает цикл (docker pull из реестра обратно в образ): стрелки берутся
    // по порядку, и стрелка, которой цель уже достаёт до начала, в уровни не идёт и рисуется дугой.
    const fwd = [], reach = (from, to) => { const seen = new Set(), st = [from]; while (st.length) { const x = st.pop(); if (x === to) return true; if (seen.has(x)) continue; seen.add(x); for (const [a, b] of fwd) if (a === x) st.push(b); } return false; };
    for (const e of edges) if (!reach(e[1], e[0])) fwd.push(e);
    const isBack = new Set(edges.filter((e) => !fwd.includes(e)));
    const level = new Map(nodes.map((n) => [n.id, 0]));
    // самый длинный путь: повторять, пока уровни растут (граф без циклов - сойдётся за число блоков)
    for (let k = 0; k < nodes.length; k++) { let moved = false; for (const [a, b] of fwd) if (level.get(b) < level.get(a) + 1) { level.set(b, level.get(a) + 1); moved = true; } if (!moved) break; }
    // блок, в который ничего не входит, встаёт прямо над своей ближайшей целью: иначе длинная стрелка
    // от него идёт через чужие блоки (миграции -> база мимо ORM)
    for (const n of nodes) {
      if (fwd.some(([, b]) => b === n.id)) continue;
      const outs = fwd.filter(([a]) => a === n.id).map(([, b]) => level.get(b));
      if (outs.length) level.set(n.id, Math.max(0, Math.min(...outs) - 1));
    }
    const sparse = [];
    for (const n of nodes) (sparse[level.get(n.id)] ||= []).push(n);
    // пустые уровни убираются: иначе в схеме дыра, а в расчёте размеров NaN
    const cols = sparse.filter(Boolean);
    const maxRows = Math.max(...cols.map((c) => c.length), 1);
    const hasLabels = edges.some((e) => e[2]);
    // Две раскладки одной схемы: слева направо и сверху вниз. Какую показать, решает ширина колонки
    // (fitDiagrams): схема не ужимается мельче натурального размера, иначе текст становится нечитаемым.
    const wide = layout(false), tall = layout(true);
    return `<div class="diagpair" data-w="${wide.w}" data-tw="${tall.w}">${wide.svg}${tall.svg}</div>`;

    function layout(vertical) {
      const W = vertical ? (maxRows > 2 ? 196 : maxRows > 1 ? 220 : 280) : 190, H = 78, PAD = 16;
      const GX = vertical ? 26 : hasLabels ? 136 : 60, GY = vertical ? (hasLabels ? 64 : 38) : 26;
      // место справа под дуги обратных стрелок
      const arc = edges.length > fwd.length ? 56 : 0;
      const width = vertical ? PAD * 2 + maxRows * W + (maxRows - 1) * GX + arc : PAD * 2 + cols.length * W + (cols.length - 1) * GX;
      const height = vertical ? PAD * 2 + cols.length * H + (cols.length - 1) * GY : PAD * 2 + maxRows * H + (maxRows - 1) * GY + (arc ? 44 : 0);
      const pos = new Map();
      cols.forEach((c, ci) => c.forEach((n, ri) => {
        const off = (maxRows - c.length) * ((vertical ? W + GX : H + GY) / 2);
        pos.set(n.id, vertical ? { x: PAD + off + ri * (W + GX), y: PAD + ci * (H + GY) } : { x: PAD + ci * (W + GX), y: PAD + off + ri * (H + GY) });
      }));
      // сколько знаков влезает в строку: от ширины блока и размера шрифта (14 px - имя, 12,5 px - пояснение)
      const labMax = Math.floor((W - 46) / 8.2), noteMax = Math.floor((W - 24) / 6.9);
      const mid = `ar${++diagSeq}`;
      let lines = '', labels = '';
      // занятое место: блоки и уже поставленные подписи
      const placed = nodes.map((n) => ({ ...pos.get(n.id), w: W, h: H }));
      const free = (r) => r.x >= 2 && r.y >= 2 && !placed.some((o) => r.x < o.x + o.w + 4 && o.x < r.x + r.w + 4 && r.y < o.y + o.h + 3 && o.y < r.y + r.h + 3);
      for (const e of edges) {
        const [a, b, label] = e;
        const p = pos.get(a), q = pos.get(b), back = isBack.has(e);
        let d, lx, ly;
        if (vertical && !back && q.y > p.y) {
          const x1 = p.x + W / 2, y1 = p.y + H, x2 = q.x + W / 2, y2 = q.y - 2, my = (y1 + y2) / 2;
          d = `M${x1} ${y1} C${x1} ${my} ${x2} ${my} ${x2} ${y2}`; lx = (x1 + x2) / 2; ly = my;
        } else if (vertical) {
          // дуга идёт справа от всех блоков, а не от своего - иначе режет соседние колонки
          const x1 = p.x + W, y1 = p.y + H / 2, x2 = q.x + W + 2, y2 = q.y + H / 2, dx = PAD + maxRows * W + (maxRows - 1) * GX + arc - 10;
          d = `M${x1} ${y1} C${dx} ${y1} ${dx} ${y2} ${x2} ${y2}`; lx = dx - 20; ly = (y1 + y2) / 2;
        } else if (!back && q.x > p.x) {
          const x1 = p.x + W, y1 = p.y + H / 2, x2 = q.x - 2, y2 = q.y + H / 2, mx = (x1 + x2) / 2;
          d = `M${x1} ${y1} C${mx} ${y1} ${mx} ${y2} ${x2} ${y2}`; lx = mx; ly = (y1 + y2) / 2;
        } else {
          // дуга идёт под всеми блоками
          const x1 = p.x + W / 2, y1 = p.y + H, x2 = q.x + W / 2, y2 = q.y + H + 2, dy = PAD + maxRows * H + (maxRows - 1) * GY + 30;
          d = `M${x1} ${y1} C${x1} ${dy} ${x2} ${dy} ${x2} ${y2}`; lx = (x1 + x2) / 2; ly = dy - 10;
        }
        lines += `<path d="${d}" fill="none" stroke="var(--mute)" stroke-width="1.4" marker-end="url(#${mid})" opacity=".8"/>`;
        if (!label) continue;
        // длинная подпись - в две строки; если место занято другой подписью - сдвиг в сторону
        const words = String(label).split(/\s+/), rows = [];
        for (const wd of words) { const last = rows[rows.length - 1]; if (last && (last + ' ' + wd).length <= 16) rows[rows.length - 1] = last + ' ' + wd; else rows.push(wd); }
        const shown = rows.slice(0, 2).map((r, k) => (k === 1 && rows.length > 2 ? r.slice(0, 15) + '…' : r));
        const lw = Math.max(...shown.map((r) => r.length)) * 7.3 + 16, lh = shown.length * 15 + 8;
        let r = { x: lx - lw / 2, y: ly - lh / 2, w: lw, h: lh };
        for (const [dx, dy] of [[0, 0], [lw / 2 + 6, 0], [-lw / 2 - 6, 0], [0, lh + 4], [0, -lh - 4], [lw + 10, 0], [-lw - 10, 0], [0, (lh + 4) * 2], [0, -(lh + 4) * 2], [lw + 10, lh + 4], [-lw - 10, lh + 4],
          ...Array.from({ length: 8 }, (_, k) => [[lw + 10, (k + 2) * (lh + 4)], [-lw - 10, (k + 2) * (lh + 4)], [0, (k + 3) * (lh + 4)]]).flat()]) {
          const t = { x: lx - lw / 2 + dx, y: ly - lh / 2 + dy, w: lw, h: lh };
          if (free(t)) { r = t; break; }
        }
        placed.push(r);
        labels += `<rect x="${r.x}" y="${r.y}" width="${lw}" height="${lh}" rx="5" fill="var(--bg)" stroke="var(--line)"/>`;
        shown.forEach((t, k) => { labels += `<text x="${r.x + lw / 2}" y="${r.y + 16 + k * 15}" text-anchor="middle" class="dl">${esc(t)}</text>`; });
      }
      let boxes = '';
      for (const n of nodes) {
        const p = pos.get(n.id);
        const lab = wrap(n.label, labMax).slice(0, 2), note = wrap(n.note, noteMax).slice(0, 2);
        boxes += `<g><rect x="${p.x}" y="${p.y}" width="${W}" height="${H}" rx="11" fill="var(--panel)" stroke="${esc(color)}" stroke-width="1.6"/>`;
        if (n.icon) boxes += `<g transform="translate(${p.x + 10} ${p.y + 10})" color="${esc(color)}">${icon(n.icon, 18, color)}</g>`;
        const tx = p.x + (n.icon ? 34 : 12);
        lab.forEach((l, i) => { boxes += `<text x="${tx}" y="${p.y + 25 + i * 17}" class="dn">${esc(l)}</text>`; });
        note.forEach((l, i) => { boxes += `<text x="${p.x + 12}" y="${p.y + 25 + lab.length * 17 + 2 + i * 15}" class="dd">${esc(l)}</text>`; });
        boxes += '</g>';
      }
      // порядок слоёв: стрелки, блоки, подписи стрелок поверх всего
      // подписи у дуг могут выйти за расчётный край - холст растёт под них
      const fw = Math.ceil(Math.max(width, ...placed.map((r) => r.x + r.w + PAD))), fh = Math.ceil(Math.max(height, ...placed.map((r) => r.y + r.h + PAD)));
      const svg = `<svg class="diag ${vertical ? 'tall' : 'wide'}" viewBox="0 0 ${fw} ${fh}" width="${fw}" role="img" aria-label="${esc(s.title || 'схема')}"><defs><marker id="${mid}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" fill="var(--mute)"/></marker></defs>${lines}${boxes}${labels}</svg>`;
      return { svg, w: fw };
    }
  }
  function wrap(t, max) {
    const words = String(t || '').split(/\s+/), lines = [];
    let cur = '';
    for (const w of words) { if ((cur + ' ' + w).trim().length > max && cur) { lines.push(cur); cur = w; } else cur = (cur + ' ' + w).trim(); }
    if (cur) lines.push(cur);
    return lines;
  }

  // Широкая раскладка - пока помещается почти в натуральную величину (до 95%), иначе - вертикальная.
  // Вертикальная, если и она шире колонки, прокручивается вбок, но не ужимается.
  function fitDiagrams(root) {
    for (const p of (root || document).querySelectorAll('.diagpair')) {
      const avail = p.clientWidth;
      // вертикальная - только если она и правда лучше: влезает или хотя бы уже широкой
      if (avail) p.classList.toggle('narrow', avail < +p.dataset.w * 0.95 && (+p.dataset.tw <= avail || +p.dataset.tw < +p.dataset.w));
    }
  }
  let fitT;
  window.addEventListener('resize', () => { clearTimeout(fitT); fitT = setTimeout(() => { fitDiagrams(); spy(); }, 60); });
  // панель подробностей и консоль меняют ширину колонки без resize окна
  if (window.ResizeObserver) document.addEventListener('DOMContentLoaded', () => {
    const m = document.getElementById('main');
    if (m) new ResizeObserver(() => fitDiagrams(m)).observe(m);
  });
  if (window.MutationObserver) document.addEventListener('DOMContentLoaded', () => {
    const m = document.getElementById('main');
    if (m) new MutationObserver(() => { fitDiagrams(m); spy(); }).observe(m, { childList: true, subtree: true });
  });

  // ---------- секции ----------
  function section(t, s, i, ctx) {
    const c = t.color || '#3a61e0';
    const head = s.title ? `<h3>${esc(s.title)}</h3>` : '';
    // заголовок по умолчанию - из базы (ui.json, lessonNav.types), тот же, что в полосе разделов
    const own = `<h3>${esc(navTitle({ type: s.type }))}</h3>`;
    const g = (x) => (ctx.learn ? gloss(esc(x)) : esc(x));
    switch (s.type) {
      case 'idea':
        return `<section class="lsec idea">${head}<p class="itext">${g(s.text)}</p>${s.analogy ? `<div class="analogy" style="--tc:${esc(c)}"><b>Как в жизни</b>${g(s.analogy)}</div>` : ''}</section>`;
      case 'diagram':
        return `<section class="lsec">${head}<div class="diagwrap">${diagram(s, c)}</div>${s.caption ? `<p class="cap">${g(s.caption)}</p>` : ''}</section>`;
      case 'terms':
        return `<section class="lsec">${head || own}<div class="tgrid">${(s.items || []).map((x) => `<div class="tterm" style="--tc:${esc(c)}"><b>${esc(x.term)}</b><span>${g(x.text)}</span></div>`).join('')}</div></section>`;
      case 'steps':
        return `<section class="lsec">${head}<ol class="lsteps" style="--tc:${esc(c)};--tc-ink:${ink(c)}">${(s.items || []).map((x) => `<li><span>${g(x)}</span></li>`).join('')}</ol></section>`;
      case 'anatomy':
        return `<section class="lsec">${head}<div class="anat big">${(s.parts || []).map((p, k) => `<div class="ap" style="--pc:hsl(${(k * 67 + 210) % 360} 65% 50%)"><code>${esc(p.text)}</code><span>${g(p.note)}</span></div>`).join('')}</div></section>`;
      case 'example':
        return `<section class="lsec">${head}${s.label ? `<div class="llabel">${esc(s.label)}</div>` : ''}<div class="lcode"><pre>${esc(s.code)}</pre><button class="lcopy" data-copy="${esc(s.code)}" title="Скопировать">Копировать</button></div>${s.output ? `<div class="lout"><small>${esc(s.outputLabel || 'Что выведет')}</small><pre>${esc(s.output)}</pre></div>` : ''}${s.note ? `<p class="cap">${g(s.note)}</p>` : ''}</section>`;
      case 'compare':
        return `<section class="lsec">${head}<div class="ltablewrap"><table class="ltable"><thead><tr>${(s.cols || []).map((x) => `<th>${esc(x)}</th>`).join('')}</tr></thead><tbody>${(s.rows || []).map((r) => `<tr>${r.map((x, k) => (k ? `<td>${g(x)}</td>` : `<th>${esc(x)}</th>`)).join('')}</tr>`).join('')}</tbody></table></div></section>`;
      case 'chart': {
        const max = Math.max(...(s.items || []).map((x) => x.value), 1);
        return `<section class="lsec">${head}<div class="lchart">${(s.items || []).map((x, k) => `<div class="cb"><span class="cl">${esc(x.label)}</span><span class="cbar"><i style="width:${Math.max(2, (x.value / max) * 100)}%;background:${k ? `color-mix(in srgb, ${esc(c)} ${Math.max(35, 100 - k * 18)}%, var(--panel))` : esc(c)}"></i></span><span class="cv">${esc(String(x.value).replace('.', ','))}${s.unit ? ' ' + esc(s.unit) : ''}</span>${x.note ? `<span class="cn">${g(x.note)}</span>` : ''}</div>`).join('')}</div>${s.note ? `<p class="cap">${g(s.note)}</p>` : ''}</section>`;
      }
      case 'mistakes':
        return `<section class="lsec">${head || own}<div class="mgrid">${(s.items || []).map((x) => `<div class="mcard"><div class="mp"><span class="dot d-bad"></span>${g(x.problem)}</div><div class="mf"><span class="dot d-ok"></span>${g(x.fix)}</div></div>`).join('')}</div></section>`;
      case 'cheatsheet': {
        // group стоит у первой команды группы; следующие без group - в той же группе
        let cur = '';
        const rows = (s.items || []).map((x) => ({ ...x, g: (cur = x.group || cur) }));
        const groups = [...new Set(rows.map((x) => x.g))];
        const key = `${t.id}:${i}`, sel = cheatSel.get(key) || '';
        const row = (x) => `<div class="crow"><code>${esc(x.cmd)}</code><span>${g(x.text)}</span><button class="btn ghost sm" data-copy="${esc(x.cmd)}" title="Скопировать" aria-label="Скопировать: ${esc(x.cmd)}">⧉</button></div>`;
        const title = head || own;
        if (groups.length < 2) return `<section class="lsec">${title}<div class="cheat">${rows.map(row).join('')}</div></section>`;
        // несколько групп: фильтр сверху и отдельный блок на группу - команды разных программ не сливаются в один список
        const chip = (id, label, n) => `<button class="fchip ${sel === id ? 'on' : ''}" aria-pressed="${sel === id}" data-cheat="${esc(key)}" data-g="${esc(id)}">${esc(label)} <span class="n">${n}</span></button>`;
        const filter = `<div class="cfilter">${chip('', 'Все', rows.length)}${groups.map((gr) => chip(gr, gr || 'Общее', rows.filter((x) => x.g === gr).length)).join('')}</div>`;
        const blocks = groups.filter((gr) => !sel || gr === sel).map((gr) => `<div class="cblock">${gr ? `<div class="cgroup">${esc(gr)}</div>` : ''}<div class="cheat">${rows.filter((x) => x.g === gr).map(row).join('')}</div></div>`).join('');
        return `<section class="lsec">${title}${filter}${blocks}</section>`;
      }
      case 'quiz': {
        const items = s.items || [];
        const keyOf = (k) => `${t.id}:${i}:${k}`;
        const right = items.filter((q, k) => quiz.get(keyOf(k)) === q.answer).length;
        const done = items.filter((q, k) => quiz.has(keyOf(k))).length;
        // счёт и «Начать заново» видны с первого ответа, а не только в конце
        const bar = done
          ? `<div class="qbar ${done === items.length ? 'fin' : ''}"><span>${done === items.length ? `Готово: верно ${right} из ${items.length}` : `Отвечено ${done} из ${items.length} · верно ${right}`}</span><button class="btn sm" data-quiz-reset="${esc(t.id)}:${i}">Начать заново</button></div>`
          : `<p class="qhint">Нажмите на ответ - Пульт сразу покажет, верно ли, и объяснит почему.</p>`;
        return `<section class="lsec" id="quiz-${esc(t.id)}-${i}">${head || own}${bar}<div class="quiz">${items.map((q, k) => {
          const a = quiz.get(keyOf(k));
          return `<div class="qq"><b>${k + 1}. ${esc(q.q)}</b><div class="qopts">${(q.options || []).map((o, n) => `<button class="qo ${a == null ? '' : n === q.answer ? 'right' : n === a ? 'wrong' : 'dim'}" data-quiz="${esc(keyOf(k))}" data-n="${n}" ${a != null ? 'disabled' : ''}>${esc(o)}</button>`).join('')}</div>${a != null ? `<p class="qwhy ${a === q.answer ? 'ok' : 'bad'}">${a === q.answer ? 'Верно. ' : 'Не совсем. '}${g(q.why)}</p>` : ''}</div>`;
        }).join('')}</div></section>`;
      }
      case 'codepair': {
        // два кода рядом: C# и IL, Dockerfile и слои, до и после
        const side = (x) => `<div class="cpside"><div class="llabel">${esc(x?.label || '')}</div><div class="lcode"><pre>${esc(x?.code || '')}</pre><button class="lcopy" data-copy="${esc(x?.code || '')}" title="Скопировать">Копировать</button></div></div>`;
        return `<section class="lsec">${head}<div class="cpair">${side(s.left)}${side(s.right)}</div>${s.note ? `<p class="cap">${g(s.note)}</p>` : ''}</section>`;
      }
      case 'practice':
        return `<section class="lsec"><div class="practice" style="--tc:${esc(c)}"><div class="ptitle">${icon('play', 16, c)}<b>${esc(navTitle(s))}</b></div>
          <p class="ptask">${g(s.task)}</p>
          ${s.steps?.length ? `<ol class="lsteps" style="--tc:${esc(c)};--tc-ink:${ink(c)}">${s.steps.map((x) => `<li><span>${g(x)}</span></li>`).join('')}</ol>` : ''}
          ${s.check ? `<p class="pcheck"><b>Как понять, что получилось:</b> ${g(s.check)}</p>` : ''}
          ${s.hint ? `<details class="pmore"><summary>Подсказка</summary><p>${g(s.hint)}</p></details>` : ''}
          ${s.solution ? `<details class="pmore"><summary>Решение</summary><div class="lcode"><pre>${esc(s.solution)}</pre><button class="lcopy" data-copy="${esc(s.solution)}" title="Скопировать">Копировать</button></div></details>` : ''}
        </div></section>`;
      case 'rules': {
        // как правильно: делать, не делать, когда применять, совет
        const mark = { do: ['ok', 'Делайте'], dont: ['bad', 'Не делайте'], when: ['info', 'Когда'], tip: ['warn', 'Совет'] };
        return `<section class="lsec">${head || own}<div class="rgrid">${(s.items || []).map((x) => { const [lv, word] = mark[x.kind] || mark.tip; return `<div class="rulecard r-${lv}"><span class="rk">${word}</span>${x.title ? `<b>${esc(x.title)}</b>` : ''}<span>${g(x.text)}</span></div>`; }).join('')}</div></section>`;
      }
      case 'project':
        return `<section class="lsec">${head || own}${ctx.projectRows(t.id)}</section>`;
      default:
        return '';
    }
  }

  // ---------- полоса разделов урока: липкая, с подсветкой текущего раздела ----------
  // подпись раздела без своего заголовка - по типу секции из базы (ui.json, lessonNav.types)
  function navTitle(s) { return String(s.title || (NAV.types || {})[s.type] || '').trim(); }
  function lessonNav(t, related) {
    const items = (t.sections || []).map((s, i) => [String(i), navTitle(s)]).filter(([, x]) => x);
    if (related) items.push(['rel', navTitle({ type: 'related' })]);
    if (items.length < 4) return '';
    const chip = ([i, x]) => `<button class="lnchip" data-lnav="${i}" title="${esc(x)}">${esc(x)}</button>`;
    return `<nav class="lnav" aria-label="${esc(NAV.label || 'Разделы урока')}" style="--tc:${esc(t.color || '#3a61e0')}">
      <details class="lnall"><summary>${esc(NAV.all || 'Все разделы')} <span class="n">${items.length}</span></summary><div class="lnlist">${items.map(chip).join('')}</div></details>
      <div class="lnrow">${items.map(chip).join('')}</div></nav>`;
  }
  function spy() {
    const m = document.getElementById('main'), nav = m && m.querySelector('.lnav');
    if (!nav) return;
    const secs = [...m.querySelectorAll('#lessonbody [data-li]')];
    if (!secs.length) return;
    const line = m.getBoundingClientRect().top + nav.offsetHeight + 40;
    let cur = secs[0];
    for (const s of secs) if (s.getBoundingClientRect().top <= line) cur = s;
    if (m.scrollTop + m.clientHeight >= m.scrollHeight - 4) cur = secs[secs.length - 1];
    const id = cur.dataset.li;
    for (const b of nav.querySelectorAll('.lnchip')) { const on = b.dataset.lnav === id; if (b.classList.contains('on') !== on) { b.classList.toggle('on', on); if (on) b.setAttribute('aria-current', 'location'); else b.removeAttribute('aria-current'); } }
    const row = nav.querySelector('.lnrow'), on = row && row.querySelector('.lnchip.on');
    // текущий раздел - в середину полосы; scrollIntoView не годится: он прокрутил бы и саму страницу
    if (on && row.dataset.on !== id) { row.dataset.on = id; row.scrollLeft = on.offsetLeft - row.clientWidth / 2 + on.offsetWidth / 2; }
  }
  let spyQ = false;
  document.addEventListener('scroll', (e) => { if (e.target.id === 'main' && !spyQ) { spyQ = true; requestAnimationFrame(() => { spyQ = false; spy(); }); } }, true);
  document.addEventListener('click', (e) => {
    const b = e.target.closest('[data-lnav]');
    if (!b) return;
    const sec = document.querySelector(`#lessonbody [data-li="${CSS.escape(b.dataset.lnav)}"]`);
    b.closest('details')?.removeAttribute('open');
    if (sec) sec.scrollIntoView({ block: 'start', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
  });
  // открытый список разделов закрывается кликом мимо него и клавишей Esc
  document.addEventListener('click', (e) => { for (const d of document.querySelectorAll('.lnall[open]')) if (!d.contains(e.target)) d.removeAttribute('open'); });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') for (const d of document.querySelectorAll('.lnall[open]')) d.removeAttribute('open'); });

  function topicHtml(t, ctx) {
    const c = t.color || '#3a61e0';
    const related = (t.related || []).map((r) => ctx.topicTitle(r) ? `<button class="chip linkchip" data-topic="${esc(r)}">${esc(ctx.topicTitle(r))}</button>` : '').join('');
    return `<article class="lesson" style="--tc:${esc(c)};--tc-ink:${ink(c)}">
      <header class="lhead"><span class="tico big" style="background:${esc(c)}">${icon(t.icon, 26, ink(c))}</span>
        <div><h2>${esc(t.title)}</h2><p>${ctx.learn ? gloss(esc(t.summary)) : esc(t.summary)}</p>
        <div class="chips">${t.level ? `<span class="chip">${esc(t.level)}</span>` : ''}${ctx.why ? `<span class="chip">почему показан: ${esc(ctx.why)}</span>` : ''}</div></div></header>
      ${lessonNav(t, related)}
      ${(t.sections || []).map((s, i) => section(t, s, i, ctx).replace('<section ', `<section data-li="${i}" `)).join('')}
      ${related ? `<section class="lsec" data-li="rel"><h3>${esc(navTitle({ type: 'related' }))}</h3><div class="chips">${related}</div></section>` : ''}
    </article>`;
  }

  function cheat(key, g) { if (g) cheatSel.set(key, g); else cheatSel.delete(key); }
  function answer(key, n) { if (!quiz.has(key)) quiz.set(key, n); }
  function resetQuiz(prefix) { for (const k of [...quiz.keys()]) if (k.startsWith(prefix + ':')) quiz.delete(k); }

  window.Learn = { setKnowledge, gloss, icon, ink, topicHtml, diagram, answer, resetQuiz, cheat, esc };
})();
