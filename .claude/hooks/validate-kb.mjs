#!/usr/bin/env node
// Хук PostToolUse: правка файла в knowledge/ -> node knowledge/validate.mjs по этому файлу.
//
// Срабатывает только на knowledge/: хук стоит на весь репозиторий, а проверка
// базы на каждую правку .rs или .md была бы лишней. Правка самого validate.mjs
// или SCHEMA.md проверяет всю базу. Ссылки между файлами validate.mjs сверяет
// со всеми файлами в любом случае.
//
// Код 2 - ошибки уходят агенту как обратная связь (протокол хуков Claude Code),
// код 0 - молчание. Сбой окружения (нет node) называется словами и не выдаётся
// за «база красная». Запуск без оболочки: node и аргументы списком.
import { spawnSync } from 'node:child_process';
import path from 'node:path';

const root = process.env.CLAUDE_PROJECT_DIR || process.cwd();

let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', (c) => (input += c));
process.stdin.on('end', () => {
  let file = '';
  try {
    file = String(JSON.parse(input)?.tool_input?.file_path ?? '');
  } catch {
    process.exit(0); // не наш формат - не наш хук
  }
  if (!file) process.exit(0);
  const kdir = path.resolve(root, 'knowledge');
  const rel = path.relative(kdir, path.resolve(root, file)).replace(/\\/g, '/');
  if (!rel || rel.startsWith('..') || path.isAbsolute(rel)) process.exit(0);

  let args;
  if (rel.endsWith('.json')) args = [rel];
  else if (rel === 'validate.mjs' || rel === 'SCHEMA.md') args = [];
  else process.exit(0);

  const r = spawnSync(process.execPath, [path.join(kdir, 'validate.mjs'), ...args], { cwd: root, encoding: 'utf8' });
  const out = `${r.stdout ?? ''}${r.stderr ?? ''}`.trim();
  if (r.status === 0) process.exit(0);
  if (r.status === null || r.error) {
    process.stderr.write(`Хук базы знаний: validate.mjs не запустился, база НЕ проверена:\n${r.error?.message ?? out.slice(-1500)}\n`);
    process.exit(0);
  }
  process.stderr.write(`База знаний красная после правки knowledge/${rel}:\n${out}\n`);
  process.exit(2);
});
