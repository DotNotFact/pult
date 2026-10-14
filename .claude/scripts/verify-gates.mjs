#!/usr/bin/env node
// Умеет ли каждый гейт ПОКРАСНЕТЬ. Гейт, который никогда не падал, неотличим
// от отсутствующего: он может быть сломан, выключен или проверять не то.
//
// Что проверяется:
//   - хуки .githooks/ (pre-commit, commit-msg, pre-push) - настоящим git в
//     пустом временном репозитории с core.hooksPath на хуки ЭТОГО дерева;
//     пуш идёт в голый репозиторий рядом, то есть хук получает настоящий ввод;
//   - node tools/status.mjs --check - поломкой на месте с возвратом байт в байт.
//
// У каждой поломки есть контроль: та же операция без поломки обязана пройти.
// Без контроля красное не доказывает ничего - гейт, красный всегда, тоже
// «краснеет».
//
// Запуск: node .claude/scripts/verify-gates.mjs   (нужны git, bash от Git for Windows, node)

import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, unlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const HOOKS = join(ROOT, '.githooks');
const CANON = '140886710+DotNotFact@users.noreply.github.com';
// кодом символа, а не буквой: в этом файле самом тире быть не должно
const LONG = String.fromCharCode(0x2014);
const MID = String.fromCharCode(0x2013);

const results = [];
function check(name, want, got, detail = '') {
  const ok = want === got;
  results.push(ok);
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${ok ? '' : `  - ждали «${want}», вышло «${got}»${detail ? `\n${detail}` : ''}`}`);
}

// Окружение без наших обходов и чужих подмен автора.
const baseEnv = { ...process.env };
for (const k of Object.keys(baseEnv)) {
  if (/^(GIT_(AUTHOR|COMMITTER)_|IDENTITY_FORCE$|PULT_PUSH_FORCE$|GIT_DIR$|GIT_WORK_TREE$|GIT_INDEX_FILE$)/.test(k)) delete baseEnv[k];
}

const work = mkdtempSync(join(tmpdir(), 'pult-gates-'));
const repo = join(work, 'repo');
const remote = join(work, 'remote.git');
mkdirSync(repo);

function git(args, env = {}, cwd = repo) {
  const r = spawnSync('git', args, { cwd, env: { ...baseEnv, ...env }, encoding: 'utf8' });
  return { code: r.status, out: `${r.stdout ?? ''}${r.stderr ?? ''}` };
}
const verdict = (r) => (r.code === 0 ? 'пропущен' : 'отклонён');
const put = (rel, text) => {
  mkdirSync(dirname(join(repo, rel)), { recursive: true });
  writeFileSync(join(repo, rel), text);
};
const stampFile = () => join(repo, '.git', 'last-push-at');
const ageStamp = (sec) => writeFileSync(stampFile(), `${Math.floor(Date.now() / 1000) - sec}\n`);
const dropStamp = () => {
  for (const f of ['last-push-at', 'last-push-refs']) rmSync(join(repo, '.git', f), { force: true });
};
// Откат к голове до попытки: если сломанный хук всё же пропустил коммит, он не
// должен остаться в истории и ронять следующие сценарии (pre-push видит его).
const mark = () => git(['rev-parse', 'HEAD']).out.trim();
const undo = (head) => {
  git(['reset', '-q', '--hard', head]);
  git(['clean', '-qfd']);
};

let commitN = 0;
function commit(msg = 'Правка', env = {}, extra = []) {
  put(`f${++commitN}.txt`, `строка ${commitN}\n`);
  git(['add', '-A']);
  return git([...extra, 'commit', '-q', '-m', msg], env);
}

try {
  git(['init', '-q', '-b', 'main', '.']);
  git(['config', 'core.hooksPath', HOOKS.replace(/\\/g, '/')]);
  git(['config', 'user.name', 'DotNotFact']);
  git(['config', 'user.email', CANON]);
  git(['config', 'core.autocrlf', 'false']);
  git(['init', '-q', '--bare', remote], {}, work);
  git(['remote', 'add', 'origin', remote.replace(/\\/g, '/')]);
  // Подставной валидатор базы: красный, если в JSON есть "bad". Проверяется
  // проводка хука (зовёт ли он валидатор и слышит ли его код), а не сама база.
  put('knowledge/validate.mjs', "import fs from 'node:fs';\nconst t = fs.readdirSync('knowledge').filter((n) => n.endsWith('.json')).map((n) => fs.readFileSync('knowledge/' + n, 'utf8')).join('');\nif (t.includes('\"bad\"')) { console.error('плохо'); process.exit(1); }\n");
  git(['add', '-A']);
  const first = git(['commit', '-q', '-m', 'Начало']);
  if (first.code !== 0) throw new Error(`первый коммит не создался:\n${first.out}`);

  // ---- pre-commit
  put('a.md', 'обычный текст - с дефисом\n');
  git(['add', '-A']);
  check('Hook_PreCommit_PlainText_IsAccepted (контроль)', 'пропущен', verdict(git(['commit', '-q', '-m', 'Текст'])));

  for (const [name, ch] of [['LongDash', LONG], ['MidDash', MID]]) {
    const h = mark();
    put('b.md', `текст ${ch} с тире\n`);
    git(['add', '-A']);
    const r = git(['commit', '-q', '-m', 'Тире']);
    check(`Hook_PreCommit_${name}_IsRejected`, 'отклонён', verdict(r), r.out);
    undo(h);
  }

  for (const f of ['.env', 'cfg/.env.local']) {
    const h = mark();
    put(f, 'KEY=1\n');
    git(['add', '-A']);
    const r = git(['commit', '-q', '-m', 'Окружение']);
    check(`Hook_PreCommit_EnvFile_IsRejected (${f})`, 'отклонён', verdict(r), r.out);
    undo(h);
  }

  put('knowledge/x.json', '{ "ok": 1 }\n');
  git(['add', '-A']);
  check('Hook_PreCommit_KnowledgeGreen_IsAccepted (контроль)', 'пропущен', verdict(git(['commit', '-q', '-m', 'База'])));
  {
    const h = mark();
    put('knowledge/x.json', '{ "bad": 1 }\n');
    git(['add', '-A']);
    check('Hook_PreCommit_KnowledgeRed_IsRejected', 'отклонён', verdict(git(['commit', '-q', '-m', 'База'])));
    undo(h);
  }

  // ---- commit-msg
  check(
    'Hook_CommitMsg_PlainMessage_IsAccepted (контроль)',
    'пропущен',
    verdict(commit('Файл состояния сгенерирован командой node tools/status.mjs')),
  );
  for (const [name, msg] of [
    ['CoAuthoredByAI', 'Правка\n\nCo-Authored-By: Claude <noreply@anthropic.com>'],
    ['GeneratedWith', 'Правка\n\nGenerated with [Claude Code](https://claude.com/claude-code)'],
    ['RobotBanner', 'Правка\n\n\u{1F916} сделано'],
  ]) {
    const h = mark();
    const r = commit(msg);
    check(`Hook_CommitMsg_${name}_IsRejected`, 'отклонён', verdict(r), r.out);
    undo(h);
  }
  {
    const h = mark();
    const r = commit('Правка', {}, ['-c', 'user.email=other@example.com']);
    check('Hook_CommitMsg_ForeignAuthor_IsRejected', 'отклонён', verdict(r), r.out);
    undo(h);
  }

  // ---- pre-push: настоящие пуши в голый репозиторий
  dropStamp();
  commit('Ветка');
  check('Hook_PrePush_FirstBranchPush_IsAllowed (контроль)', 'пропущен', verdict(git(['push', '-q', 'origin', 'HEAD:refs/heads/task/a'])));
  commit('Ещё');
  {
    const r = git(['push', '-q', 'origin', 'HEAD:refs/heads/task/a']);
    check('Hook_PrePush_SecondBranchPushWithin3Min_IsRejected', 'отклонён', verdict(r), r.out);
  }
  ageStamp(240);
  check('Hook_PrePush_BranchPushAfter4Min_IsAllowed', 'пропущен', verdict(git(['push', '-q', 'origin', 'HEAD:refs/heads/task/a'])));
  ageStamp(240);
  {
    const r = git(['push', '-q', 'origin', 'HEAD:refs/heads/main']);
    check('Hook_PrePush_MainPushAfter4Min_IsRejected', 'отклонён', verdict(r), r.out);
  }
  ageStamp(660);
  check('Hook_PrePush_MainPushAfter11Min_IsAllowed', 'пропущен', verdict(git(['push', '-q', 'origin', 'HEAD:refs/heads/main'])));
  check('Hook_PrePush_BranchDeletion_IsFree', 'пропущен', verdict(git(['push', '-q', 'origin', '--delete', 'task/a'])));
  check('Hook_PrePush_NothingToSend_IsFree', 'пропущен', verdict(git(['push', '-q', 'origin', 'HEAD:refs/heads/main'])));
  commit('Срочно');
  check(
    'Hook_PrePush_ForceVariable_SkipsThePause',
    'пропущен',
    verdict(git(['push', '-q', 'origin', 'HEAD:refs/heads/main'], { PULT_PUSH_FORCE: '1' })),
  );
  {
    commit('Чужой', { IDENTITY_FORCE: '1' }, ['-c', 'user.email=other@example.com']);
    const r = git(['push', '-q', 'origin', 'HEAD:refs/heads/task/b'], { PULT_PUSH_FORCE: '1' });
    check('Hook_PrePush_ForeignAuthor_IsRejectedEvenWithForce', 'отклонён', verdict(r), r.out);
    git(['reset', '-q', '--hard', 'HEAD~1']);
  }
  {
    // коммит мимо commit-msg: так испорченный коммит приезжает с другой машины
    put(`f${++commitN}.txt`, 'чужое\n');
    git(['add', '-A']);
    git(['commit', '-q', '--no-verify', '-m', 'Правка\n\nCo-Authored-By: Claude <noreply@anthropic.com>']);
    const r = git(['push', '-q', 'origin', 'HEAD:refs/heads/task/c'], { PULT_PUSH_FORCE: '1' });
    check('Hook_PrePush_AiTrailerInMessage_IsRejected', 'отклонён', verdict(r), r.out);
    git(['reset', '-q', '--hard', 'HEAD~1']);
  }
  {
    // Если у origin появится второй push-url, git позовёт хук дважды на одну
    // отправку; второе плечо не должно отклоняться как «слишком часто».
    const remote2 = join(work, 'remote2.git');
    git(['init', '-q', '--bare', remote2], {}, work);
    git(['remote', 'set-url', '--add', '--push', 'origin', remote.replace(/\\/g, '/')]);
    git(['remote', 'set-url', '--add', '--push', 'origin', remote2.replace(/\\/g, '/')]);
    ageStamp(240);
    commit('Два адреса');
    const r = git(['push', '-q', 'origin', 'HEAD:refs/heads/task/d']);
    check('Hook_PrePush_OnePushToTwoPushUrls_IsOneSend', 'пропущен', verdict(r), r.out);
  }
} finally {
  rmSync(work, { recursive: true, force: true });
}

// ---- status --check: поломка на месте, возврат байт в байт
const node = (args) => spawnSync(process.execPath, args, { cwd: ROOT, encoding: 'utf8' });
const status = () => verdict({ code: node(['tools/status.mjs', '--check']).status });
const statusFile = join(ROOT, 'docs', 'status.md');

check('Gate_Status_CleanTree_IsGreen (контроль)', 'пропущен', status());

const original = readFileSync(statusFile);
try {
  writeFileSync(statusFile, Buffer.concat([original, Buffer.from('| Выдумано руками | 1 | - |\n')]));
  check('Gate_Status_FailsWhenTheFileIsEditedByHand', 'отклонён', status());
} finally {
  writeFileSync(statusFile, original);
}

const fake = join(ROOT, '.claude', 'agents', 'zz-sabotage.md');
if (existsSync(fake)) throw new Error(`${fake} уже есть - не моя поломка, не трогаю`);
try {
  writeFileSync(fake, '---\nname: zz-sabotage\n---\n');
  check('Gate_Status_FailsWhenAnAgentIsAddedWithoutRegenerating', 'отклонён', status());
} finally {
  unlinkSync(fake);
}
check('Gate_Status_AfterRestore_IsGreenAgain (контроль)', 'пропущен', status());

const failed = results.filter((r) => !r).length;
console.log(`\n${results.length - failed} из ${results.length} зелёные`);
process.exit(failed ? 1 : 0);
