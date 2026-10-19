// Проверка базы знаний: схема, ссылки между файлами, запрещённые символы и слова.
// node knowledge/validate.mjs            - всё
// node knowledge/validate.mjs tools/x.json topics/y.json - только эти файлы (ссылки всё равно сверяются со всеми)
import { promises as fs } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const K = path.dirname(fileURLToPath(import.meta.url));
// список уроков и их порядок - в lessons.json, его же читают окно и терминал
const TOPICS = JSON.parse(await fs.readFile(path.join(K, 'lessons.json'), 'utf8')).order;
const SECTIONS = ['idea', 'diagram', 'terms', 'steps', 'anatomy', 'example', 'codepair', 'compare', 'chart', 'mistakes', 'rules', 'practice', 'cheatsheet', 'quiz', 'project'];
const RULE_KINDS = ['do', 'dont', 'when', 'tip'];
const DASHES = new RegExp('[' + String.fromCharCode(0x2014, 0x2013) + ']');
const BAD_WORDS = /(давайте разбер|важно отметить|стоит отметить|(?<![а-яё])мощн|современн|(?<![а-яё])является|в мире|не секрет|по сути|ключевую роль|играет роль|позволяет вам|уникальн|инновац)/i;
const errors = [];
const err = (file, where, msg) => errors.push(`${file}: ${where}: ${msg}`);

async function readJson(rel) {
  try { return JSON.parse(await fs.readFile(path.join(K, rel), 'utf8')); } catch (e) { err(rel, '-', 'не JSON: ' + e.message); return null; }
}
function walkStrings(v, cb, where = '') {
  if (typeof v === 'string') cb(v, where);
  else if (Array.isArray(v)) v.forEach((x, i) => walkStrings(x, cb, `${where}[${i}]`));
  else if (v && typeof v === 'object') for (const [k, x] of Object.entries(v)) walkStrings(x, cb, where ? `${where}.${k}` : k);
}
function textRules(file, data) {
  walkStrings(data, (s, where) => {
    if (DASHES.test(s)) err(file, where, 'длинное или среднее тире - замените на «-»');
    if (where.endsWith('match') || where.endsWith('re')) return;
    const m = BAD_WORDS.exec(s); if (m) err(file, where, `вода: «${m[0]}»`);
  });
}

const effects = await readJson('effects.json');
const FX = new Set(Object.keys(effects || {}));
const icons = await readJson('icons.json');
const ICONS = new Set(Object.keys(icons || {}));
for (const f of ['effects.json', 'stages.json', 'stacks.json', 'docs.json', 'ui.json', 'glossary.json', 'icons.json', 'heuristics.json', 'manifests.json']) {
  const d = f === 'effects.json' ? effects : f === 'icons.json' ? icons : await readJson(f);
  if (d && f !== 'icons.json') textRules(f, d);
}

const only = process.argv.slice(2);
const want = (rel) => !only.length || only.some((o) => rel.replace(/\\/g, '/').endsWith(o.replace(/\\/g, '/')));

// ---- tools
const binOwner = new Map();
const toolIds = new Set();
for (const f of (await fs.readdir(path.join(K, 'tools')).catch(() => [])).filter((n) => n.endsWith('.json'))) {
  const rel = 'tools/' + f;
  const d = await readJson(rel);
  if (!d) continue;
  if (!Array.isArray(d.tools)) { err(rel, '-', 'нет массива tools'); continue; }
  const check = want(rel);
  if (check) textRules(rel, d);
  d.tools.forEach((t, i) => {
    const w = `tools[${i}] ${t.id || '?'}`;
    if (toolIds.has(t.id)) err(rel, w, 'повтор id');
    toolIds.add(t.id);
    for (const b of t.bins || []) {
      if (typeof b !== 'string' || !b.trim()) { err(rel, w, 'пустая строка в bins'); continue; }
      const key = b.toLowerCase();
      if (binOwner.has(key)) err(rel, w, `bin «${b}» уже есть у ${binOwner.get(key)}`);
      binOwner.set(key, `${rel}:${t.id}`);
    }
    if (!check) return;
    for (const k of ['id', 'name', 'topic', 'about']) if (typeof t[k] !== 'string' || !t[k]) err(rel, w, `нет поля ${k}`);
    if (!Array.isArray(t.bins) || !t.bins.length) err(rel, w, 'нет bins');
    if (t.topic && !TOPICS.includes(t.topic)) err(rel, w, `неизвестный topic «${t.topic}»`);
    if (!t.default?.say) err(rel, w, 'нет default.say');
    const desc = (x, ww) => {
      for (const fx of x.fx || []) if (!FX.has(fx)) err(rel, ww, `неизвестный эффект «${fx}»`);
      for (const fx of x.fxRemove || []) if (!FX.has(fx)) err(rel, ww, `неизвестный эффект «${fx}»`);
      if (x.say && /\.$/.test(x.say)) err(rel, ww, 'say без точки в конце');
    };
    desc(t.default || {}, w + '.default');
    (t.sub || []).forEach((s, j) => {
      try { new RegExp(s.match, 'i'); } catch (e) { err(rel, `${w}.sub[${j}]`, 'плохой match: ' + e.message); }
      if (!s.say) err(rel, `${w}.sub[${j}]`, 'нет say');
      desc(s, `${w}.sub[${j}]`);
    });
    (t.flags || []).forEach((fl, j) => {
      if (!/^-/.test(fl.flag || '')) err(rel, `${w}.flags[${j}]`, 'flag должен начинаться с -');
      if (!fl.text) err(rel, `${w}.flags[${j}]`, 'нет text');
      desc(fl, `${w}.flags[${j}]`);
    });
    if (t.how && (!Array.isArray(t.how) || t.how.length > 6)) err(rel, w, 'how - массив до 6 шагов');
    // help: null, ключ («--help») или шаблоны по порядку, каждый с {bin} в начале
    if (t.help !== undefined && t.help !== null) {
      const pats = Array.isArray(t.help) ? t.help : [t.help];
      if (!pats.length) err(rel, w, 'help - пустой список');
      for (const p of pats) {
        if (typeof p !== 'string' || !p.trim()) { err(rel, w, 'help - строка или список строк'); continue; }
        const isPattern = Array.isArray(t.help) || p.includes('{');
        if (isPattern && !p.startsWith('{bin}')) err(rel, w, `шаблон help «${p}» должен начинаться с {bin}`);
        for (const ph of p.match(/\{[^}]*\}/g) || []) if (ph !== '{bin}' && ph !== '{sub}') err(rel, w, `в help неизвестная подстановка ${ph}`);
      }
    }
  });
}

// ---- topics
const termForms = new Map();
for (const f of (await fs.readdir(path.join(K, 'topics')).catch(() => [])).filter((n) => n.endsWith('.json'))) {
  const rel = 'topics/' + f;
  const t = await readJson(rel);
  if (!t || !want(rel)) continue;
  textRules(rel, t);
  if (t.id !== f.replace(/\.json$/, '')) err(rel, '-', 'id должен совпадать с именем файла');
  if (!TOPICS.includes(t.id)) err(rel, '-', `id не из списка уроков`);
  for (const k of ['title', 'summary', 'color', 'icon']) if (!t[k]) err(rel, '-', `нет поля ${k}`);
  if (t.icon && !ICONS.has(t.icon)) err(rel, '-', `нет значка «${t.icon}» в icons.json`);
  if (!/^#[0-9a-f]{6}$/i.test(t.color || '')) err(rel, '-', 'color - #rrggbb');
  if (!t.detect || typeof t.detect !== 'object') err(rel, '-', 'нет detect');
  for (const r of t.related || []) if (!TOPICS.includes(r)) err(rel, 'related', `неизвестный урок «${r}»`);
  if (!Array.isArray(t.sections) || t.sections.length < 4) err(rel, '-', 'нужно хотя бы 4 секции');
  (t.sections || []).forEach((s, i) => {
    const w = `sections[${i}] ${s.type}`;
    if (!SECTIONS.includes(s.type)) return err(rel, w, 'неизвестный type');
    const arr = (k) => { if (!Array.isArray(s[k]) || !s[k].length) err(rel, w, `нужен непустой ${k}`); return s[k] || []; };
    if (s.type === 'idea' && !s.text) err(rel, w, 'нет text');
    if (s.type === 'diagram') {
      const ids = new Set(arr('nodes').map((n) => n.id));
      if (ids.size !== (s.nodes || []).length) err(rel, w, 'повтор id узла');
      for (const e of s.edges || []) if (typeof e[2] === 'string' && e[2] !== e[2].trim()) err(rel, w, `пробелы по краям подписи «${e[2]}»`);
      for (const n of s.nodes || []) { if (!n.id || !n.label) err(rel, w, 'узлу нужны id и label'); if (n.icon && !ICONS.has(n.icon)) err(rel, w, `нет значка «${n.icon}»`); }
      for (const e of arr('edges')) if (!ids.has(e[0]) || !ids.has(e[1])) err(rel, w, `стрелка к неизвестному узлу: ${e.join(' -> ')}`);
    }
    if (s.type === 'terms') for (const x of arr('items')) { if (!x.term || !x.text) err(rel, w, 'термину нужны term и text'); for (const fm of [x.term, ...(x.forms || [])]) termForms.set(fm.toLowerCase(), rel); }
    if (s.type === 'steps') arr('items');
    if (s.type === 'anatomy') for (const p of arr('parts')) if (!p.text || !p.note) err(rel, w, 'части нужны text и note');
    if (s.type === 'example' && !s.code) err(rel, w, 'нет code');
    if (s.type === 'compare') { const n = arr('cols').length; for (const r of arr('rows')) if (r.length !== n) err(rel, w, 'в строке другое число ячеек'); }
    if (s.type === 'chart') for (const x of arr('items')) if (typeof x.value !== 'number' || !x.label) err(rel, w, 'столбцу нужны label и число value');
    if (s.type === 'mistakes') for (const x of arr('items')) if (!x.problem || !x.fix) err(rel, w, 'нужны problem и fix');
    if (s.type === 'cheatsheet') for (const x of arr('items')) if (!x.cmd || !x.text) err(rel, w, 'нужны cmd и text');
    if (s.type === 'codepair') for (const k of ['left', 'right']) if (!s[k]?.label || !s[k]?.code) err(rel, w, `${k}: нужны label и code`);
    if (s.type === 'practice' && !s.task) err(rel, w, 'нет task');
    if (s.type === 'rules') for (const x of arr('items')) { if (!RULE_KINDS.includes(x.kind)) err(rel, w, `kind - одно из ${RULE_KINDS.join(', ')}`); if (!x.text) err(rel, w, 'нет text'); }
    if (s.type === 'quiz') for (const x of arr('items')) if (!x.q || !Array.isArray(x.options) || typeof x.answer !== 'number' || x.answer >= x.options.length || !x.why) err(rel, w, 'вопросу нужны q, options, answer (номер), why');
  });
}

// ---- scenarios.json: готовые сценарии вкладки «Сценарии»
{
  const rel = 'scenarios.json';
  const d = await readJson(rel), st = await readJson('stages.json');
  const STAGES = new Set((Array.isArray(st) ? st : st?.stages || []).map((x) => x.id));
  if (d) {
    textRules(rel, d);
    if (!Array.isArray(d.templates) || !d.templates.length) err(rel, '-', 'нет массива templates');
    const ids = new Set();
    (d.templates || []).forEach((t, i) => {
      const w = `templates[${i}] ${t.id || '?'}`;
      for (const k of ['id', 'name', 'why']) if (typeof t[k] !== 'string' || !t[k].trim()) err(rel, w, `нет поля ${k}`);
      if (ids.has(t.id)) err(rel, w, 'повтор id');
      ids.add(t.id);
      if (!Array.isArray(t.steps) || t.steps.length < 2) err(rel, w, 'нужно хотя бы два шага');
      (t.steps || []).forEach((s, j) => {
        const ws = `${w}.steps[${j}]`;
        if (!s.stage && !s.name) err(rel, ws, 'шагу нужен stage или name');
        if (s.stage && !STAGES.has(s.stage)) err(rel, ws, `неизвестный этап «${s.stage}»`);
        if (s.name) { try { new RegExp(s.name, 'i'); } catch (e) { err(rel, ws, 'плохой name: ' + e.message); } }
      });
      if ((t.steps || []).every((s) => s.optional)) err(rel, w, 'все шаги необязательные');
    });
  }
}

// ---- ai.json: виды деталей ИИ-части, слои и рассказ (core/src/ai.rs читает этот файл)
let aiStats = '';
{
  const rel = 'ai.json';
  const d = await readJson(rel), st = await readJson('stages.json');
  const STAGES = new Set((Array.isArray(st) ? st : st?.stages || []).map((x) => x.id));
  const FORMATS = ['markdown', 'script', 'permissions', 'hooks', 'hookscript', 'mcp', 'config', 'checks', 'signals'];
  const REFS = ['path', 'file', 'name', 'tag', 'slash', 'script'];
  const NAME_FROM = ['file', 'filename', 'dir', 'path'];
  const WORDS = ['what', 'when', 'why', 'analogy', 'look'];
  // регулярные выражения написаны для Rust (regex); для проверки снимаем встроенные флаги (?i)
  const re = (w, s) => { try { new RegExp(String(s).replace(/^\(\?i\)/, ''), 'i'); } catch (e) { err(rel, w, 'плохое регулярное выражение: ' + e.message); } };
  const str = (w, v) => { if (typeof v !== 'string' || !v.trim()) err(rel, w, 'нужна непустая строка'); };
  if (d) {
    textRules(rel, d);
    str('lesson', d.lesson);
    if (d.lesson && !TOPICS.includes(d.lesson)) err(rel, 'lesson', `урока «${d.lesson}» нет в lessons.json`);
    for (const k of ['fileBytes', 'walkFiles', 'itemsPerKind', 'summary', 'desc', 'bodyScan', 'hits', 'mentionMin']) if (!Number.isInteger(d.limits?.[k]) || d.limits[k] <= 0) err(rel, `limits.${k}`, 'нужно положительное целое');
    for (const k of ['walkSkip', 'markers', 'ignoreFiles']) if (!Array.isArray(d[k]) || d[k].some((x) => typeof x !== 'string' || !x.trim())) err(rel, k, 'нужен список непустых строк');
    if (!d.markers?.length) err(rel, 'markers', 'без признаков вкладка «ИИ» никогда не покажется');
    for (const k of ['key', 'value', 'inline', 'userinfo']) { str(`secrets.${k}`, d.secrets?.[k]); if (d.secrets?.[k]) re(`secrets.${k}`, d.secrets[k]); }
    if (!String(d.storyMore || '').includes('{n}')) err(rel, 'storyMore', 'нужна подстановка {n}');
    for (const [k, v] of Object.entries(d.facts || {})) if (k !== '_') str(`facts.${k}`, v);
    const layerIds = new Set((d.layers || []).map((l) => l.id));
    const kindIds = new Set((d.kinds || []).map((k) => k.id));
    if (!Array.isArray(d.kinds) || !d.kinds.length) err(rel, '-', 'нет массива kinds');
    const seen = new Set();
    // роли, которые деталь вида может получить: из roles вида и из groups (проверки)
    const rolesOf = (k) => new Set([...Object.keys(k.roles || {}), ...Object.keys(k.groups || {}), ...(k.signals || []).map((s) => s.id)]);
    (d.kinds || []).forEach((k, i) => {
      const w = `kinds[${i}] ${k.id || '?'}`;
      for (const f of ['id', 'title', 'icon', 'layer', 'format']) str(`${w}.${f}`, k[f]);
      if (seen.has(k.id)) err(rel, w, 'повтор id');
      seen.add(k.id);
      if (k.icon && !ICONS.has(k.icon)) err(rel, w, `нет значка «${k.icon}» в icons.json`);
      if (k.layer && !layerIds.has(k.layer)) err(rel, w, `слоя «${k.layer}» нет в layers`);
      if (!FORMATS.includes(k.format)) err(rel, w, `format - одно из ${FORMATS.join(', ')}`);
      if (k.lesson && !TOPICS.includes(k.lesson)) err(rel, w, `урока «${k.lesson}» нет в lessons.json`);
      if (!k.lesson) err(rel, w, 'нет lesson: ссылки на урок');
      for (const f of WORDS) str(`${w}.words.${f}`, k.words?.[f]);
      const modes = k.modes || {};
      if (!Object.keys(modes).length) err(rel, w, 'нет modes');
      for (const [m, v] of Object.entries(modes)) { str(`${w}.modes.${m}.label`, v.label); str(`${w}.modes.${m}.say`, v.say); }
      for (const r of k.refs || []) if (!REFS.includes(r)) err(rel, w, `refs: неизвестный способ «${r}»`);
      if (!Array.isArray(k.refs)) err(rel, w, 'нужен массив refs (может быть пустым)');
      if (k.parse) {
        if (!NAME_FROM.includes(k.parse.nameFrom)) err(rel, w, `parse.nameFrom - одно из ${NAME_FROM.join(', ')}`);
        for (const f of ['name', 'desc', 'paths', 'tools', 'model']) if (!Array.isArray(k.parse[f])) err(rel, w, `parse.${f}: нужен массив`);
      } else if (!['checks', 'signals'].includes(k.format)) err(rel, w, 'нет parse');
      for (const [r, v] of Object.entries(k.roles || {})) { str(`${w}.roles.${r}.label`, v.label); str(`${w}.roles.${r}.say`, v.say); }
      const hasDetect = (k.detect || []).length > 0;
      if (!hasDetect && !['checks', 'signals'].includes(k.format)) err(rel, w, 'нет detect');
      (k.detect || []).forEach((x, j) => {
        const wd = `${w}.detect[${j}]`;
        str(`${wd}.glob`, x.glob);
        if (typeof x.eco !== 'string') err(rel, wd, 'нужна строка eco (может быть пустой)');
        if (!modes[x.mode]) err(rel, wd, `mode «${x.mode}» не описан в modes вида`);
        for (const m of x.modes || []) { if (!modes[m.mode]) err(rel, wd, `modes: «${m.mode}» не описан в modes вида`); str(`${wd}.modes.key`, m.key); }
        if (x.format && !FORMATS.includes(x.format)) err(rel, wd, `format - одно из ${FORMATS.join(', ')}`);
        if (x.role && !rolesOf(k).has(x.role) && x.role !== 'tool') err(rel, wd, `роли «${x.role}» нет в roles вида`);
        if (/^[a-z]:|^\\\\|\.\.\//i.test(x.glob || '')) err(rel, wd, 'glob - путь от корня проекта, без диска и «..»');
      });
      if (k.format === 'script' && !(k.events || []).length) err(rel, w, 'нет events: какие имена файлов - события git');
      if (k.format === 'checks') {
        for (const [g, v] of Object.entries(k.groups || {})) { str(`${w}.groups.${g}.title`, v.title); str(`${w}.groups.${g}.say`, v.say); }
        for (const g of ['tests', 'ci', 'meta', 'measure']) if (!k.groups?.[g]) err(rel, w, `нет groups.${g}`);
        for (const s of k.scripts?.stages || []) if (!STAGES.has(s)) err(rel, w, `scripts.stages: неизвестный этап «${s}»`);
        for (const f of ['metaRe', 'measureRe']) { str(`${w}.${f}`, k[f]); if (k[f]) re(`${w}.${f}`, k[f]); }
        if (k.scripts?.alsoRe) re(`${w}.scripts.alsoRe`, k.scripts.alsoRe);
        if (k.purpose) {
          str(`${w}.purpose.title`, k.purpose.title);
          if (!Array.isArray(k.purpose.items) || !k.purpose.items.length) err(rel, w, 'purpose.items - непустой список');
          for (const x of k.purpose.items || []) { str(`${w}.purpose.items.title`, x.title); str(`${w}.purpose.items.text`, x.text); }
        }
      }
      if (k.format === 'signals') {
        const ids = new Set();
        (k.signals || []).forEach((s, j) => {
          const ws = `${w}.signals[${j}]`;
          for (const f of ['id', 'title', 'say']) str(`${ws}.${f}`, s[f]);
          if (ids.has(s.id)) err(rel, ws, 'повтор id');
          ids.add(s.id);
          const kinds = ['dir', 'file', 'scan'].filter((f) => s[f]);
          if (kinds.length !== 1) err(rel, ws, 'нужен ровно один из dir, file, scan');
          if (s.scan) re(`${ws}.scan`, s.scan);
        });
        for (const x of k.scanKinds || []) if (!kindIds.has(x)) err(rel, w, `scanKinds: неизвестный вид «${x}»`);
        if (!(k.scanKinds || []).length) err(rel, w, 'нет scanKinds');
      }
    });
    const inLayer = new Map();
    (d.layers || []).forEach((l, i) => {
      const w = `layers[${i}] ${l.id || '?'}`;
      for (const f of ['id', 'title', 'say']) str(`${w}.${f}`, l[f]);
      if (!['accent', 'info', 'violet', 'ok', 'warn', 'bad'].includes(l.color)) err(rel, w, 'color - имя токена цвета: accent, info, violet, ok, warn или bad');
      if (!(l.kinds || []).length) err(rel, w, 'нет kinds');
      for (const x of l.kinds || []) { if (!kindIds.has(x)) err(rel, w, `неизвестный вид «${x}»`); if (inLayer.has(x)) err(rel, w, `вид «${x}» уже в слое ${inLayer.get(x)}`); inLayer.set(x, l.id); }
    });
    for (const k of kindIds) if (!inLayer.has(k)) err(rel, 'layers', `вид «${k}» не лежит ни в одном слое`);
    const stepIds = new Set();
    (d.story || []).forEach((s, i) => {
      const w = `story[${i}] ${s.id || '?'}`;
      for (const f of ['id', 'title', 'say']) str(`${w}.${f}`, s[f]);
      if (stepIds.has(s.id)) err(rel, w, 'повтор id');
      stepIds.add(s.id);
      for (const ph of String(s.say || '').match(/\{[^}]*\}/g) || []) if (ph !== '{parts}' && ph !== '{n}') err(rel, w, `неизвестная подстановка ${ph}`);
      if (!s.always && !(s.any || []).length) err(rel, w, 'нужен any или always');
      (s.any || []).forEach((c, j) => {
        const wc = `${w}.any[${j}]`;
        const k = (d.kinds || []).find((x) => x.id === c.kind);
        if (!k) return err(rel, wc, `неизвестный вид «${c.kind}»`);
        if (c.mode && !k.modes?.[c.mode]) err(rel, wc, `у вида «${c.kind}» нет режима «${c.mode}»`);
        if (c.role && !rolesOf(k).has(c.role)) err(rel, wc, `у вида «${c.kind}» нет роли «${c.role}»`);
        if (c.match) re(`${wc}.match`, c.match);
      });
    });
    aiStats = `, видов ИИ-части: ${(d.kinds || []).length}`;
  }
}

// ---- ссылки tools -> topics на диске
const existingTopics = new Set((await fs.readdir(path.join(K, 'topics')).catch(() => [])).map((n) => n.replace(/\.json$/, '')));
if (!only.length) {
  for (const f of (await fs.readdir(path.join(K, 'tools')).catch(() => [])).filter((n) => n.endsWith('.json'))) {
    const d = await readJson('tools/' + f);
    for (const t of d?.tools || []) if (t.topic && !existingTopics.has(t.topic)) err('tools/' + f, t.id, `урока «${t.topic}» пока нет в topics/ (предупреждение)`);
  }
}

if (errors.length) { console.log(errors.join('\n')); console.log(`\nошибок: ${errors.length}`); process.exit(1); }
console.log(`ok · программ: ${toolIds.size}, команд-префиксов: ${binOwner.size}, уроков: ${existingTopics.size}, терминов: ${termForms.size}${aiStats}`);
