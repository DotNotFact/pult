#!/usr/bin/env node
// Куда уходят токены: разбор транскриптов сессий Claude Code (*.jsonl).
// Перенесён из Истока (tools/session-stats.mjs), категории - под Пульт.
//
// Запуск: node .claude/scripts/session-stats.mjs [каталог-с-транскриптами ...]
// Без аргументов - каталоги ~/.claude/projects/<имя этого репозитория>*
// (имя - путь репозитория, где всё, кроме букв и цифр, заменено на «-»).
//
// Считает: токены по ответам модели (output, cache read, cache create);
// объём результатов инструментов по категориям; прогоны проверок без единой
// правки между ними; повторные чтения файлов и повторные команды; снимки
// (каждый живёт в контексте до конца сессии); сжатия контекста.
// Один транскрипт может лежать в двух каталогах - дубли отбрасываются по
// первым строкам.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import readline from 'node:readline';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const slug = ROOT.replace(/[^A-Za-z0-9]/g, '-');
const base = path.join(os.homedir(), '.claude', 'projects');
const dirs = process.argv.length > 2
  ? process.argv.slice(2)
  : (fs.existsSync(base) ? fs.readdirSync(base) : []).filter((d) => d.startsWith(slug)).map((d) => path.join(base, d));
if (!dirs.length) {
  console.error(`session-stats: нет каталогов ${slug}* в ${base}; передайте каталог аргументом`);
  process.exit(2);
}

function category(cmd) {
  if (/cargo (test|clippy|fmt|check)|validate\.mjs|contrast\.mjs|status\.mjs|verify-gates|cdp\.mjs check/.test(cmd)) return 'bash:verify';
  if (/^\s*(git|gh)\b/.test(cmd)) return 'bash:git-gh';
  if (/cargo build|npm (install|ci)|npx/.test(cmd)) return 'bash:build';
  return 'bash:other';
}

async function analyze(fp) {
  const s = { session: path.basename(fp, '.jsonl'), mb: fs.statSync(fp).size / 1048576, turns: 0, out: 0, cacheRead: 0, cacheCreate: 0,
    resultChars: {}, reads: {}, cmds: {}, images: 0, compacts: 0, verify: 0, verifyNoEdit: 0, first: null, last: null, fp: '' };
  const idTool = new Map();
  let edits = 1;
  let n = 0;
  const rl = readline.createInterface({ input: fs.createReadStream(fp), crlfDelay: Infinity });
  for await (const line of rl) {
    if (!line.trim()) continue;
    if (++n <= 3) s.fp += line.slice(0, 80);
    let e;
    try { e = JSON.parse(line); } catch { continue; }
    if (e.timestamp) { s.first ??= e.timestamp; s.last = e.timestamp; }
    if (e.type === 'system' && e.subtype === 'compact_boundary') s.compacts++;
    else if (e.type === 'assistant' && e.message) {
      s.turns++;
      const u = e.message.usage || {};
      s.out += u.output_tokens || 0; s.cacheRead += u.cache_read_input_tokens || 0; s.cacheCreate += u.cache_creation_input_tokens || 0;
      for (const b of Array.isArray(e.message.content) ? e.message.content : []) {
        if (b.type !== 'tool_use') continue;
        let cat = b.name;
        if (b.name === 'Bash' || b.name === 'PowerShell') {
          const cmd = b.input?.command || '';
          cat = category(cmd);
          s.cmds[cmd.slice(0, 160)] = (s.cmds[cmd.slice(0, 160)] || 0) + 1;
          if (cat === 'bash:verify') { s.verify++; if (edits === 0) s.verifyNoEdit++; edits = 0; }
          else if (/node -e|sed -i|cat >|Set-Content|Out-File/.test(cmd)) edits++;
        }
        if (['Edit', 'Write', 'NotebookEdit'].includes(b.name)) edits++;
        if (b.name === 'Read' && b.input?.file_path) { const k = b.input.file_path.replace(/\\/g, '/'); s.reads[k] = (s.reads[k] || 0) + 1; }
        idTool.set(b.id, cat);
      }
    } else if (e.type === 'user' && Array.isArray(e.message?.content)) {
      for (const b of e.message.content) {
        if (b.type !== 'tool_result') continue;
        const cat = idTool.get(b.tool_use_id) || '?';
        let chars = 0;
        const walk = (c) => {
          if (typeof c === 'string') chars += c.length;
          else if (Array.isArray(c)) c.forEach(walk);
          else if (c?.type === 'text') chars += (c.text || '').length;
          else if (c?.type === 'image') s.images++;
        };
        walk(b.content);
        s.resultChars[cat] = (s.resultChars[cat] || 0) + chars;
      }
    }
  }
  return s;
}

const seen = new Set();
const all = [];
for (const d of dirs) {
  for (const f of fs.readdirSync(d).filter((x) => x.endsWith('.jsonl'))) {
    const s = await analyze(path.join(d, f));
    if (seen.has(s.fp)) continue;
    seen.add(s.fp);
    all.push(s);
  }
}

const fmt = (x) => x.toLocaleString('ru-RU');
const sum = { out: 0, cacheRead: 0, turns: 0, images: 0, verify: 0, verifyNoEdit: 0, compacts: 0, resultChars: {}, reads: {}, cmds: {} };
const add = (into, from) => { for (const [k, v] of Object.entries(from)) into[k] = (into[k] || 0) + v; };
for (const s of all.sort((a, b) => (a.first || '').localeCompare(b.first || ''))) {
  console.log(`\n${s.session} [${s.mb.toFixed(1)} МБ] ${s.first?.slice(0, 16)} -> ${s.last?.slice(0, 16)}`);
  console.log(`  ходов ${s.turns}, output ${fmt(s.out)}, cache read ${fmt(s.cacheRead)}, сжатий ${s.compacts}, снимков ${s.images}`);
  console.log(`  проверок ${s.verify}, из них без правки перед ними ${s.verifyNoEdit}`);
  for (const k of ['out', 'cacheRead', 'turns', 'images', 'verify', 'verifyNoEdit', 'compacts']) sum[k] += s[k];
  add(sum.resultChars, s.resultChars); add(sum.reads, s.reads); add(sum.cmds, s.cmds);
}

console.log(`\n=== ИТОГО: сессий ${all.length}, ходов ${fmt(sum.turns)}, output ${fmt(sum.out)}, cache read ${fmt(sum.cacheRead)}`);
console.log(`средний контекст на ход: ${fmt(Math.round(sum.cacheRead / Math.max(1, sum.turns)))} токенов; сжатий ${sum.compacts}; снимков ${sum.images}`);
console.log(`проверок без правки перед ними: ${sum.verifyNoEdit} из ${sum.verify} (${Math.round((100 * sum.verifyNoEdit) / Math.max(1, sum.verify))}%, цель - меньше 15%)`);
const total = Object.values(sum.resultChars).reduce((a, b) => a + b, 0) || 1;
console.log('\nРезультаты инструментов (символы):');
for (const [k, v] of Object.entries(sum.resultChars).sort((a, b) => b[1] - a[1]).slice(0, 12)) console.log(`  ${k}: ${fmt(v)} (${((100 * v) / total).toFixed(1)}%)`);
console.log('\nФайлы, прочитанные 3+ раз:');
for (const [k, v] of Object.entries(sum.reads).filter(([, v]) => v >= 3).sort((a, b) => b[1] - a[1]).slice(0, 15)) console.log(`  ${v}x ${k}`);
console.log('\nКоманды, запущенные 5+ раз:');
for (const [k, v] of Object.entries(sum.cmds).filter(([, v]) => v >= 5).sort((a, b) => b[1] - a[1]).slice(0, 15)) console.log(`  ${v}x ${k.replace(/\n/g, ' ')}`);
