'use strict';
// Уроки и подсказки к терминам. Всё содержимое - из knowledge/topics/*.json и glossary.json,
// здесь только то, как это нарисовать.
(function () {
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const escRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  let ICONS = {};
  let GLOSS = null;
  const quiz = new Map(); // ответы на вопросы: «урок:секция:вопрос» -> номер ответа

  function icon(name, size = 18, color = 'currentColor') {
    const d = ICONS[name] || ICONS.book || '';
    return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="${esc(color)}" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${esc(d)}"/></svg>`;
  }

  // ---------- подсказки к терминам ----------
  function setKnowledge(kb) {
    ICONS = kb.icons || {};
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

  // ---------- схема: блоки и стрелки, слева направо ----------
  function diagram(s, color) {
    const nodes = s.nodes || [], edges = (s.edges || []).filter((e) => nodes.some((n) => n.id === e[0]) && nodes.some((n) => n.id === e[1]));
    const level = new Map(nodes.map((n) => [n.id, 0]));
    // самый длинный путь от начала; циклы обрываются ограничением шагов
    for (let k = 0; k < nodes.length; k++) {
      let moved = false;
      for (const [a, b] of edges) if (a !== b && level.get(b) < level.get(a) + 1 && level.get(a) + 1 < nodes.length) { level.set(b, level.get(a) + 1); moved = true; }
      if (!moved) break;
    }
    const cols = [];
    for (const n of nodes) (cols[level.get(n.id)] ||= []).push(n);
    // если у стрелок есть подписи - между колонками нужно место под текст
    const hasLabels = edges.some((e) => e[2]);
    const W = 168, H = 66, GX = hasLabels ? 118 : 56, GY = 22, PAD = 16;
    const maxRows = Math.max(...cols.map((c) => c?.length || 0), 1);
    const width = PAD * 2 + cols.length * W + (cols.length - 1) * GX;
    const height = PAD * 2 + maxRows * H + (maxRows - 1) * GY;
    const pos = new Map();
    cols.forEach((c, ci) => (c || []).forEach((n, ri) => {
      const off = (maxRows - c.length) * (H + GY) / 2;
      pos.set(n.id, { x: PAD + ci * (W + GX), y: PAD + off + ri * (H + GY) });
    }));
    const wrap = (t, max) => {
      const words = String(t || '').split(/\s+/), lines = [];
      let cur = '';
      for (const w of words) { if ((cur + ' ' + w).trim().length > max && cur) { lines.push(cur); cur = w; } else cur = (cur + ' ' + w).trim(); }
      if (cur) lines.push(cur);
      return lines;
    };
    let svg = `<svg class="diag" viewBox="0 0 ${width} ${height}" width="${width}" role="img" aria-label="${esc(s.title || 'схема')}"><defs><marker id="ar" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" fill="var(--mute)"/></marker></defs>`;
    for (const [a, b, label] of edges) {
      const p = pos.get(a), q = pos.get(b);
      if (!p || !q) continue;
      let d, lx, ly;
      if (q.x > p.x) {
        const x1 = p.x + W, y1 = p.y + H / 2, x2 = q.x - 2, y2 = q.y + H / 2, mx = (x1 + x2) / 2;
        d = `M${x1} ${y1} C${mx} ${y1} ${mx} ${y2} ${x2} ${y2}`; lx = mx; ly = (y1 + y2) / 2;
      } else {
        // назад или в той же колонке - дугой снизу
        const x1 = p.x + W / 2, y1 = p.y + H, x2 = q.x + W / 2, y2 = q.y + H + 2, dy = Math.max(y1, y2) + 34;
        d = `M${x1} ${y1} C${x1} ${dy} ${x2} ${dy} ${x2} ${y2}`; lx = (x1 + x2) / 2; ly = dy - 8;
      }
      svg += `<path d="${d}" fill="none" stroke="var(--mute)" stroke-width="1.4" marker-end="url(#ar)" opacity=".8"/>`;
      if (label) {
        // длинная подпись - в две строки, чтобы не наезжать на блоки
        const words = String(label).split(/\s+/), rows = [];
        for (const wd of words) { const last = rows[rows.length - 1]; if (last && (last + ' ' + wd).length <= 17) rows[rows.length - 1] = last + ' ' + wd; else rows.push(wd); }
        const shown = rows.slice(0, 2).map((r, k, all) => (k === 1 && rows.length > 2 ? r.slice(0, 15) + '…' : r));
        const lw = Math.min(GX + 8, Math.max(...shown.map((r) => r.length)) * 6 + 12), lh = shown.length * 12 + 6;
        svg += `<rect x="${lx - lw / 2}" y="${ly - lh / 2}" width="${lw}" height="${lh}" rx="5" fill="var(--bg)" stroke="var(--line)"/>`;
        shown.forEach((r, k) => { svg += `<text x="${lx}" y="${ly - lh / 2 + 13 + k * 12}" text-anchor="middle" class="dl">${esc(r)}</text>`; });
      }
    }
    for (const n of nodes) {
      const p = pos.get(n.id);
      const lab = wrap(n.label, 17).slice(0, 2), note = wrap(n.note, 26).slice(0, 2);
      svg += `<g><rect x="${p.x}" y="${p.y}" width="${W}" height="${H}" rx="11" fill="var(--panel)" stroke="${esc(color)}" stroke-width="1.6"/>`;
      if (n.icon) svg += `<g transform="translate(${p.x + 10} ${p.y + 10})" color="${esc(color)}">${icon(n.icon, 18, color)}</g>`;
      const tx = p.x + (n.icon ? 34 : 12);
      lab.forEach((l, i) => { svg += `<text x="${tx}" y="${p.y + 23 + i * 15}" class="dn">${esc(l)}</text>`; });
      note.forEach((l, i) => { svg += `<text x="${p.x + 12}" y="${p.y + 23 + lab.length * 15 + 2 + i * 13}" class="dd">${esc(l)}</text>`; });
      svg += '</g>';
    }
    return `${svg}</svg>`;
  }

  // ---------- секции ----------
  function section(t, s, i, ctx) {
    const c = t.color || '#3a61e0';
    const head = s.title ? `<h3>${esc(s.title)}</h3>` : '';
    const g = (x) => (ctx.learn ? gloss(esc(x)) : esc(x));
    switch (s.type) {
      case 'idea':
        return `<section class="lsec idea">${head}<p class="itext">${g(s.text)}</p>${s.analogy ? `<div class="analogy" style="--tc:${esc(c)}"><b>Как в жизни</b>${g(s.analogy)}</div>` : ''}</section>`;
      case 'diagram':
        return `<section class="lsec">${head}<div class="diagwrap">${diagram(s, c)}</div>${s.caption ? `<p class="cap">${g(s.caption)}</p>` : ''}</section>`;
      case 'terms':
        return `<section class="lsec">${head || '<h3>Слова, которые надо знать</h3>'}<div class="tgrid">${(s.items || []).map((x) => `<div class="tterm" style="--tc:${esc(c)}"><b>${esc(x.term)}</b><span>${g(x.text)}</span></div>`).join('')}</div></section>`;
      case 'steps':
        return `<section class="lsec">${head}<ol class="lsteps" style="--tc:${esc(c)}">${(s.items || []).map((x) => `<li><span>${g(x)}</span></li>`).join('')}</ol></section>`;
      case 'anatomy':
        return `<section class="lsec">${head}<div class="anat big">${(s.parts || []).map((p, k) => `<div class="ap" style="--pc:hsl(${(k * 67 + 210) % 360} 65% 50%)"><code>${esc(p.text)}</code><span>${g(p.note)}</span></div>`).join('')}</div></section>`;
      case 'example':
        return `<section class="lsec">${head}<div class="lcode"><pre>${esc(s.code)}</pre><button class="lcopy" data-copy="${esc(s.code)}" title="Скопировать">Копировать</button></div>${s.output ? `<div class="lout"><small>Что выведет</small><pre>${esc(s.output)}</pre></div>` : ''}${s.note ? `<p class="cap">${g(s.note)}</p>` : ''}</section>`;
      case 'compare':
        return `<section class="lsec">${head}<div class="ltablewrap"><table class="ltable"><thead><tr>${(s.cols || []).map((x) => `<th>${esc(x)}</th>`).join('')}</tr></thead><tbody>${(s.rows || []).map((r) => `<tr>${r.map((x, k) => (k ? `<td>${g(x)}</td>` : `<th>${esc(x)}</th>`)).join('')}</tr>`).join('')}</tbody></table></div></section>`;
      case 'chart': {
        const max = Math.max(...(s.items || []).map((x) => x.value), 1);
        return `<section class="lsec">${head}<div class="lchart">${(s.items || []).map((x, k) => `<div class="cb"><span class="cl">${esc(x.label)}</span><span class="cbar"><i style="width:${Math.max(2, (x.value / max) * 100)}%;background:${k ? `color-mix(in srgb, ${esc(c)} ${Math.max(35, 100 - k * 18)}%, var(--panel))` : esc(c)}"></i></span><span class="cv">${esc(String(x.value).replace('.', ','))}${s.unit ? ' ' + esc(s.unit) : ''}</span>${x.note ? `<span class="cn">${g(x.note)}</span>` : ''}</div>`).join('')}</div>${s.note ? `<p class="cap">${g(s.note)}</p>` : ''}</section>`;
      }
      case 'mistakes':
        return `<section class="lsec">${head || '<h3>Что ломается и как чинить</h3>'}<div class="mgrid">${(s.items || []).map((x) => `<div class="mcard"><div class="mp"><span class="dot d-bad"></span>${g(x.problem)}</div><div class="mf"><span class="dot d-ok"></span>${g(x.fix)}</div></div>`).join('')}</div></section>`;
      case 'cheatsheet':
        return `<section class="lsec">${head || '<h3>Шпаргалка</h3>'}<div class="cheat">${(s.items || []).map((x) => `<div class="crow"><code>${esc(x.cmd)}</code><span>${g(x.text)}</span><button class="btn ghost sm" data-copy="${esc(x.cmd)}" title="Скопировать">⧉</button></div>`).join('')}</div></section>`;
      case 'quiz': {
        const items = s.items || [];
        const right = items.filter((q, k) => quiz.get(`${t.id}:${i}:${k}`) === q.answer).length;
        const done = items.filter((q, k) => quiz.has(`${t.id}:${i}:${k}`)).length;
        return `<section class="lsec">${head || '<h3>Проверьте себя</h3>'}<div class="quiz">${items.map((q, k) => {
          const key = `${t.id}:${i}:${k}`, a = quiz.get(key);
          return `<div class="qq"><b>${k + 1}. ${esc(q.q)}</b><div class="qopts">${(q.options || []).map((o, n) => `<button class="qo ${a == null ? '' : n === q.answer ? 'right' : n === a ? 'wrong' : 'dim'}" data-quiz="${esc(key)}" data-n="${n}" ${a != null ? 'disabled' : ''}>${esc(o)}</button>`).join('')}</div>${a != null ? `<p class="qwhy ${a === q.answer ? 'ok' : 'bad'}">${a === q.answer ? 'Верно. ' : 'Не совсем. '}${g(q.why)}</p>` : ''}</div>`;
        }).join('')}${done ? `<p class="qscore">Верно ${right} из ${items.length}${done === items.length ? ` · <a href="#" data-quiz-reset="${esc(t.id)}:${i}">пройти заново</a>` : ''}</p>` : ''}</div></section>`;
      }
      case 'project':
        return `<section class="lsec">${head || '<h3>Где это в вашем проекте</h3>'}${ctx.projectRows(t.id)}</section>`;
      default:
        return '';
    }
  }

  function topicHtml(t, ctx) {
    const c = t.color || '#3a61e0';
    const related = (t.related || []).map((r) => ctx.topicTitle(r) ? `<button class="chip linkchip" data-topic="${esc(r)}">${esc(ctx.topicTitle(r))}</button>` : '').join('');
    return `<article class="lesson" style="--tc:${esc(c)}">
      <header class="lhead"><span class="tico big" style="background:${esc(c)}">${icon(t.icon, 26, '#fff')}</span>
        <div><h1>${esc(t.title)}</h1><p>${ctx.learn ? gloss(esc(t.summary)) : esc(t.summary)}</p>
        <div class="chips">${t.level ? `<span class="chip">${esc(t.level)}</span>` : ''}${ctx.why ? `<span class="chip">почему показан: ${esc(ctx.why)}</span>` : ''}</div></div></header>
      ${(t.sections || []).map((s, i) => section(t, s, i, ctx)).join('')}
      ${related ? `<section class="lsec"><h3>Что почитать дальше</h3><div class="chips">${related}</div></section>` : ''}
    </article>`;
  }

  function answer(key, n) { if (!quiz.has(key)) quiz.set(key, n); }
  function resetQuiz(prefix) { for (const k of [...quiz.keys()]) if (k.startsWith(prefix + ':')) quiz.delete(k); }

  window.Learn = { setKnowledge, gloss, icon, topicHtml, diagram, answer, resetQuiz, esc };
})();
