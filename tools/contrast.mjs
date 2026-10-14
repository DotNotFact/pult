// Проверка контраста палитры Пульта (WCAG 2.x).
// node tools/contrast.mjs [путь к app.css] [--all]   (без пути - ui/app.css этого репозитория)
// Читает токены из :root{...} (светлая), :root[data-theme="dark"]{...} (тёмная, наследует светлую)
// и блок auto под prefers-color-scheme (обязан совпадать с тёмным), плюс классы ANSI .f0-.f15 и .b0-.b15.
// Печатает таблицу пар и падает (код 1), если хоть одна пара ниже порога.
// --all печатает все пары, без ключа - только упавшие и сводку.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const file = process.argv.slice(2).find((a) => !a.startsWith('--')) || path.join(here, '..', 'ui', 'app.css');
const showAll = process.argv.includes('--all');
const css = fs.readFileSync(file, 'utf8');

// ---------- разбор ----------
function decls(body) {
  const m = {};
  for (const part of body.split(';')) {
    const i = part.indexOf(':');
    if (i < 0) continue;
    const k = part.slice(0, i).trim(), v = part.slice(i + 1).trim();
    if (k.startsWith('--')) m[k.slice(2)] = v;
  }
  return m;
}
function block(re, what) {
  const m = css.match(re);
  if (!m) throw new Error(`не нашёл блок ${what}`);
  return decls(m[1].replace(/\/\*[\s\S]*?\*\//g, ''));
}
const light = block(/:root\{([^}]*--bg:[^}]*)\}/, ':root');
const darkOwn = block(/:root\[data-theme="dark"\]\{([^}]*)\}/, 'dark');
const autoOwn = block(/@media \(prefers-color-scheme:dark\)\{:root\[data-theme="auto"\]\{([^}]*)\}\}/, 'auto');
// Токены, которых до новой палитры не было: для контрольного прогона берётся то, что правила писали литералом.
const LEGACY = { edge: 'var(--line)', 'on-status': '#ffffff', hl: '#ffd65a', 'code-link': '#8ab4ff', 'code-bad': '#f06a6a', 'code-warn': '#e0b13a' };
const legacyUsed = Object.keys(LEGACY).filter((k) => !(k in light));
for (const k of legacyUsed) light[k] = LEGACY[k];
if (legacyUsed.length) console.log('контроль старой палитры: вместо отсутствующих токенов взяты прежние литералы:', legacyUsed.join(', '));
const dark = { ...light, ...darkOwn };

const fails = [];
// тёмная и авто обязаны совпадать
for (const k of new Set([...Object.keys(darkOwn), ...Object.keys(autoOwn)])) {
  if (darkOwn[k] !== autoOwn[k]) fails.push(`блок auto расходится с dark: --${k} = ${autoOwn[k]} против ${darkOwn[k]}`);
}

const ansi = {};
for (const m of css.matchAll(/\.([fb])(\d{1,2})\{([^}]*)\}/g)) {
  const d = {};
  for (const p of m[3].split(';')) { const [k, v] = p.split(':'); if (v) d[k.trim()] = v.trim(); }
  ansi[m[1] + m[2]] = d;
}

// ---------- цвет ----------
function parse(v) {
  v = v.trim();
  let m = v.match(/^#([0-9a-f]{3,8})$/i);
  if (m) {
    let h = m[1];
    if (h.length <= 4) h = [...h].map((c) => c + c).join('');
    const n = (i) => parseInt(h.slice(i, i + 2), 16);
    return [n(0), n(2), n(4), h.length === 8 ? n(6) / 255 : 1];
  }
  m = v.match(/^rgba?\(([^)]*)\)$/i);
  if (m) { const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p[3] ?? 1]; }
  if (v === 'transparent') return [0, 0, 0, 0];
  throw new Error(`не разобрал цвет "${v}"`);
}
function resolve(theme, expr) {
  // expr: имя токена, литерал, mix(a,pct,b) - доля pct процентов a поверх b (как color-mix in srgb)
  const mx = expr.match(/^mix\((.+?),(\d+),(.+)\)$/);
  if (mx) {
    const a = resolve(theme, mx[1]), b = resolve(theme, mx[3]), p = +mx[2] / 100;
    return [0, 1, 2].map((i) => a[i] * p + b[i] * (1 - p)).concat(1);
  }
  if (expr.startsWith('#') || expr.startsWith('rgb')) return parse(expr);
  let v = theme[expr];
  if (v === undefined) throw new Error(`нет токена --${expr}`);
  const r = v.match(/^var\(--([a-z0-9-]+)\)$/);
  if (r) return resolve(theme, r[1]);
  return parse(v);
}
const lin = (c) => { c /= 255; return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
const lum = ([r, g, b]) => 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
const over = (fg, bg) => fg[3] >= 1 ? fg : [0, 1, 2].map((i) => fg[i] * fg[3] + bg[i] * (1 - fg[3])).concat(1);
const ratio = (fg, bg) => { const f = over(fg, bg), a = lum(f), b = lum(bg); return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05); };
const hex = (c) => '#' + c.slice(0, 3).map((x) => Math.round(x).toString(16).padStart(2, '0')).join('');

// ---------- пары ----------
// min 4.5 - текст; 3 - границы элементов, кольцо фокуса, значки, второстепенный текст; 1.3 - видимость декоративной линии.
const S = ['bg', 'panel', 'panel2', 'soft'];
const SEM = ['ok', 'warn', 'bad', 'info', 'violet'];
const SOFTS = ['accent-soft', ...SEM.map((s) => s + '-soft')];
const pairs = [];
const add = (fg, bgs, min, why) => { for (const bg of bgs) pairs.push({ fg, bg, min, why }); };

add('ink', [...S, ...SOFTS], 4.5, 'основной текст');
add('mute', [...S, 'accent-soft', 'info-soft'], 4.5, 'вторичный текст (выбранные строки, подсказки)');
add('faint', ['bg', 'panel', 'panel2'], 4.5, 'третичный текст: подписи групп, легенда, время');
add('faint', ['soft', 'accent-soft'], 3, 'значки и второстепенное на выделенном');
add('accent-ink', [...S, 'accent-soft'], 4.5, 'ссылки и выбранное');
for (const s of SEM) add(s, [...S, s + '-soft'], 4.5, `смысловой текст ${s}`);
add('on-accent', ['accent'], 4.5, 'текст основной кнопки');
for (const s of ['ok', 'warn', 'bad', 'info']) add('on-status', [s], 4.5, 'текст на заливке статуса (уведомление, значок)');
add('bg', ['ink'], 4.5, 'уведомление: текст фона на чернилах');
add('ink', ['mix(hl,35,panel)', 'mix(hl,35,bg)'], 4.5, 'текст под подсветкой совпадения (mark)');
add('accent', ['bg', 'panel', 'panel2'], 3, 'кольцо фокуса, рамки выбора');
add('edge', ['bg', 'panel', 'panel2'], 3, 'граница поля ввода и переключателя');
add('on-accent', ['edge'], 3, 'ручка переключателя на выключенной дорожке');
for (const s of SEM) add(s, ['panel', 'soft'], 3, 'точка статуса, полоса карточки');
add('line', ['bg', 'panel'], 1.3, 'декоративная линия видна');
add('panel', ['bg'], 1.04, 'слой панели отделён от фона');
add('soft', ['panel'], 1.1, 'мягкий слой отделён от панели');
// консоль и код
add('code-ink', ['code'], 4.5, 'код');
add('code-mute', ['code'], 4.5, 'служебные строки консоли');
add('code-link', ['code'], 4.5, 'ссылка в консоли');
add('code-bad', ['code', 'mix(code-bad,12,code)'], 4.5, 'ошибка в консоли');
add('code-warn', ['code'], 4.5, 'предупреждение в консоли');
add('code-ink', ['mix(hl,22,code)', 'mix(hl,45,code)'], 4.5, 'текст в найденной строке консоли');
add('code-line', ['code'], 3, 'граница поля ввода консоли');
add('console-edge', ['bg', 'code'], 3, 'рамка островка консоли видна контуром на фоне страницы и у тёмного вывода');

const rows = [];
for (const [tname, theme] of [['light', light], ['dark', dark]]) {
  for (const p of pairs) {
    let r, f, b;
    try { b = resolve(theme, p.bg); f = resolve(theme, p.fg); r = ratio(f, b); }
    catch (e) { fails.push(`${tname}: ${p.fg} на ${p.bg}: ${e.message}`); continue; }
    rows.push({ theme: tname, fg: p.fg, bg: p.bg, fgv: hex(over(f, b)), bgv: hex(b), r, min: p.min, ok: r >= p.min, why: p.why });
  }
  // ANSI: цвет текста на фоне консоли, текст по умолчанию на цветных фонах
  const code = resolve(theme, 'code');
  for (let i = 0; i < 16; i++) {
    const f = ansi['f' + i]?.color;
    if (f) { const r = ratio(parse(f), code); rows.push({ theme: tname, fg: `.f${i}`, bg: 'code', fgv: f, bgv: hex(code), r, min: 4.5, ok: r >= 4.5, why: 'ANSI текст' }); }
    const bd = ansi['b' + i];
    if (bd?.background) {
      const bgc = parse(bd.background);
      const fgExpr = bd.color ? bd.color.replace(/^var\(--(.+)\)$/, '$1') : 'code-ink';
      const fc = resolve(theme, fgExpr);
      const r = ratio(fc, bgc);
      rows.push({ theme: tname, fg: bd.color ? fgExpr : 'code-ink', bg: `.b${i}`, fgv: hex(fc), bgv: bd.background, r, min: 4.5, ok: r >= 4.5, why: 'ANSI фон' });
    }
  }
}

// ---------- вывод ----------
const bad = rows.filter((r) => !r.ok);
const pad = (s, n) => String(s).padEnd(n);
const print = (list) => {
  console.log(pad('тема', 6), pad('цвет', 14), pad('на', 22), pad('значения', 18), pad('контраст', 9), pad('порог', 6), 'зачем');
  for (const r of list) console.log(pad(r.theme, 6), pad(r.fg, 14), pad(r.bg, 22), pad(`${r.fgv}/${r.bgv}`, 18), pad(r.r.toFixed(2), 9), pad(r.min, 6), (r.ok ? '' : 'НИЖЕ ПОРОГА  ') + r.why);
};
print(showAll ? rows : bad);

// справочно: цвета тем уроков из базы знаний (данные, не токены; на результат не влияют)
try {
  const dir = path.resolve(path.dirname(file), '../knowledge/topics');
  const white = [255, 255, 255, 1];
  const low = [];
  for (const f of fs.readdirSync(dir).filter((x) => x.endsWith('.json'))) {
    const c = JSON.parse(fs.readFileSync(path.join(dir, f), 'utf8')).color;
    if (!c) continue;
    const r = ratio(white, parse(c));
    if (r < 4.5) low.push(`${f.replace('.json', '')} ${c} ${r.toFixed(2)}${r < 3 ? ' (ниже 3 - белый значок плохо виден)' : ''}`);
  }
  if (low.length) console.log(`\nсправочно: белый текст на цвете темы урока ниже 4.5 (номера шагов урока):\n  ${low.join('\n  ')}`);
} catch { /* нет базы знаний рядом */ }

for (const f of fails) console.log('ОШИБКА:', f);
console.log(`\nпар проверено: ${rows.length}, ниже порога: ${bad.length} (светлая ${bad.filter((r) => r.theme === 'light').length}, тёмная ${bad.filter((r) => r.theme === 'dark').length}), ошибок разбора: ${fails.length}`);
process.exit(bad.length || fails.length ? 1 : 0);
