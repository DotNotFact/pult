#!/usr/bin/env node
// Состояние дерева числами - командой, а не памятью.
//
// Число, набранное в документе руками, живёт до первой правки: агент читает
// документ первым и планирует по нему. Поэтому числа собираются здесь, из
// дерева, и пишутся в docs/status.md целиком. Руками файл не правится.
//
// Считается только то, что видно без запуска: файлы, каталоги, строки
// разметки. Сколько тестов ПРОШЛО, знает CI, а не документ.
//
// Запуск:
//   node tools/status.mjs          перезаписать docs/status.md
//   node tools/status.mjs --check  сверить файл с деревом (шаг CI), код 1 при расхождении
//
// Пол измерения: ноль там, где ноль невозможен (команды IPC, шаги CI, уроки),
// - отказ словами «сломан разбор», а не зелёное. Зелёное на пустом
// множестве - ровно тот дефект, против которого инструмент написан.

import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const OUT = join(ROOT, 'docs', 'status.md');
const SKIP = new Set(['target', 'node_modules', 'dist', '.git']);

const read = (rel) => readFileSync(join(ROOT, rel), 'utf8');

function entries(rel) {
  try {
    return readdirSync(join(ROOT, rel), { withFileTypes: true });
  } catch {
    return [];
  }
}
const files = (rel, test) => entries(rel).filter((e) => e.isFile() && test(e.name)).map((e) => e.name).sort();
const dirs = (rel, test = () => true) => entries(rel).filter((e) => e.isDirectory() && test(e.name)).map((e) => e.name).sort();

/** Файлы по маске, рекурсивно; каталоги сборки не считаются. */
function walk(rel, test, out = []) {
  for (const e of entries(rel)) {
    if (SKIP.has(e.name)) continue;
    const next = `${rel}/${e.name}`;
    if (e.isDirectory()) walk(next, test, out);
    else if (test(e.name)) out.push(next);
  }
  return out;
}

const problems = [];
const floor = (what, n) => {
  if (n === 0) problems.push(`${what}: ноль - сломан разбор, а не пустое дерево`);
  return n;
};

// Крейты: members из корневого Cargo.toml.
const members = (read('Cargo.toml').match(/^\s*members\s*=\s*\[([^\]]*)\]/m)?.[1] ?? '')
  .split(',')
  .map((s) => s.trim().replace(/^"|"$/g, ''))
  .filter(Boolean)
  .sort();

// Команды IPC: имена внутри generate_handler![...] в app/src/main.rs.
const handler = read('app/src/main.rs').match(/generate_handler!\s*\[([\s\S]*?)\]/)?.[1] ?? '';
const ipc = handler
  .split(',')
  .map((s) => s.replace(/\/\/.*$/gm, '').trim())
  .filter(Boolean);

// Шаги CI: элементы списка steps в каждом workflow.
function workflowSteps(text) {
  const lines = text.split(/\r?\n/);
  let count = 0;
  for (let i = 0; i < lines.length; i++) {
    const m = lines[i].match(/^(\s*)steps:\s*$/);
    if (!m) continue;
    const base = m[1].length;
    let item = -1;
    for (let j = i + 1; j < lines.length; j++) {
      const l = lines[j];
      if (!l.trim() || l.trim().startsWith('#')) continue;
      const indent = l.match(/^\s*/)[0].length;
      if (indent <= base) break;
      if (item < 0 && l.trim().startsWith('- ')) item = indent;
      if (indent === item && l.trim().startsWith('- ')) count++;
    }
  }
  return count;
}
const workflows = files('.github/workflows', (n) => /\.ya?ml$/.test(n)).map((n) => ({
  name: n,
  steps: workflowSteps(read(`.github/workflows/${n}`)),
}));

// Проверки в Rust: #[test] и #[tokio::test] в исходниках трёх крейтов.
const rsFiles = members.flatMap((m) => walk(m, (n) => n.endsWith('.rs')));
const rustTests = rsFiles.reduce((sum, f) => sum + (read(f).match(/#\[(?:tokio::)?test\]/g)?.length ?? 0), 0);
const testFiles = files('core/tests', (n) => n.endsWith('.rs'));

const coreModules = files('core/src', (n) => n.endsWith('.rs')).map((n) => n.replace(/\.rs$/, ''));
const topics = files('knowledge/topics', (n) => n.endsWith('.json')).map((n) => n.replace(/\.json$/, ''));
const toolGroups = files('knowledge/tools', (n) => n.endsWith('.json')).map((n) => n.replace(/\.json$/, ''));
const scripts = files('tools', (n) => /\.(mjs|js|ps1|sh)$/.test(n));
const rules = files('.claude/rules', (n) => n.endsWith('.md'));
const agents = files('.claude/agents', (n) => n.endsWith('.md')).map((n) => n.replace(/\.md$/, ''));
const skills = dirs('.claude/skills', (n) => existsSync(join(ROOT, '.claude/skills', n, 'SKILL.md')));
const gitHooks = files('.githooks', (n) => !n.includes('.'));
const adr = files('docs/adr', (n) => /^\d{4}-.*\.md$/.test(n));

floor('Крейты', members.length);
floor('Команды IPC', ipc.length);
floor('Уроки', topics.length);
floor('Проверки в Rust', rustTests);
for (const w of workflows) floor(`Шаги CI в ${w.name}`, w.steps);
floor('Workflow', workflows.length);

const list = (a) => a.join(', ');
const code = (a) => a.map((x) => `\`${x}\``).join(', ');

const text = `# Состояние дерева

Собрано командой \`node tools/status.mjs\`. **Руками не правится:** следующий
запуск перезапишет файл целиком. Число здесь неверное - неверно дерево, а не
документ. Сверяет шаг CI \`node tools/status.mjs --check\`.

Здесь только то, что видно без запуска. Сколько проверок ПРОШЛО, знает CI.
Сколько программ в базе знаний - печатает \`node knowledge/validate.mjs\`.

| Что | Сколько | Что именно |
|---|---|---|
| Крейты | ${members.length} | ${list(members)} |
| Модули ядра | ${coreModules.length} | ${list(coreModules)} |
| Команды IPC | ${ipc.length} | \`generate_handler!\` в \`app/src/main.rs\` |
| Проверки в Rust | ${rustTests} | \`#[test]\` в ${list(members)} |
| Файлы тестов ядра | ${testFiles.length} | ${list(testFiles.map((n) => n.replace(/\.rs$/, '')))} |
| Уроки | ${topics.length} | \`knowledge/topics/\` |
| Группы программ в базе | ${toolGroups.length} | ${list(toolGroups)} |
| Скрипты | ${scripts.length} | ${code(scripts)} |
| Файлы правил | ${rules.length} | ${list(rules.map((n) => n.replace(/\.md$/, '')))} |
| Агенты | ${agents.length} | ${list(agents)} |
| Скиллы | ${skills.length} | ${list(skills)} |
| Хуки git | ${gitHooks.length} | ${list(gitHooks)} |
| Решения (ADR) | ${adr.length} | \`docs/adr/\` |

## Шаги CI

| Workflow | Шагов |
|---|---|
${workflows.map((w) => `| \`${w.name}\` | ${w.steps} |`).join('\n')}
`;

if (problems.length) {
  console.error(`status: ${problems.join('; ')}`);
  process.exit(1);
}

if (process.argv.includes('--check')) {
  const have = existsSync(OUT) ? readFileSync(OUT, 'utf8').replace(/\r\n/g, '\n') : '';
  if (have === text) {
    console.log('status: docs/status.md совпадает с деревом');
    process.exit(0);
  }
  const a = have.split('\n');
  const b = text.split('\n');
  const diff = [];
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if (a[i] !== b[i]) diff.push(`  было:  ${a[i] ?? '<нет строки>'}\n  стало: ${b[i] ?? '<нет строки>'}`);
  }
  console.error(`status: docs/status.md разошёлся с деревом (${have ? 'строк с разницей: ' + diff.length : 'файла нет'}).`);
  if (diff.length) console.error(diff.slice(0, 8).join('\n'));
  console.error('Пересобрать: node tools/status.mjs, и закоммитить вместе с правкой.');
  process.exit(1);
}

writeFileSync(OUT, text);
console.log('status: docs/status.md записан');
