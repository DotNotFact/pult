---
name: pult-flow
description: Рабочий ход Пульта на GitHub - ветка, гейты, пуш с ожиданием паузы хука, PR с номером прогона CI, одна проверка CI вместо опроса, перезапуск упавших джоб, слияние и уборка, параллельные агенты в своих рабочих копиях. Использовать, когда пора пушить, когда задач больше одной или когда CI красный.
---

# Ветка, пуш, PR: рецепты

Правила - `.claude/rules/git-flow.md`; здесь только команды. Репозиторий -
`DotNotFact/pult`, публичный, один хостинг (GitHub).

## Один раз на рабочую копию

```bash
git config core.hooksPath .githooks
git config user.name DotNotFact
git config user.email 140886710+DotNotFact@users.noreply.github.com
```

## Ветка и гейты

```bash
git checkout main && git pull --ff-only && git checkout -b task/<коротко>
# ... правки ...
node tools/status.mjs                 # если менялись модули, тесты, IPC, правила, агенты, скиллы, шаги CI
node knowledge/validate.mjs && cargo fmt --all --check \
  && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace \
  && node tools/status.mjs --check
```

Коммит слоями (ядро, окно, база, документы), сообщение по-русски, первой
строкой - что стало правдой. Отступление от правила - словами в сообщении.

## Пуш - только после слова «пушим»

Перед пушем - авторство и строки про ИИ (хук проверит то же, но отказ
посреди пуша дороже):

```bash
git log --format='%an <%ae> | %cn <%ce>' main..HEAD | sort -u           # один DotNotFact
git log --format=%B main..HEAD | grep -ciE 'co-authored|generated with'  # 0
```

Пуш в фоне (`run_in_background`), он сам ждёт окна хука. Штамп общий для
всех рабочих копий - считать «3 минуты от своего пуша» бесполезно:

```bash
f="$(git rev-parse --git-common-dir)/last-push-at"
need=185   # ветка задачи; 605 - если в отправке есть main
[ -f "$f" ] && until [ $(( $(date +%s) - $(cat "$f") )) -ge $need ]; do sleep 5; done
git push -u origin task/<коротко>
git ls-remote --heads origin task/<коротко>; git rev-parse HEAD           # хеши совпали
```

## PR

```bash
gh pr create --base main --fill
```

Описание: что стало правдой, чем проверено. Без строки «Generated with
Claude Code». Номер прогона CI дописывается, когда он есть:
`gh pr edit <N> --body-file <файл>`.

PR пачкой: 2-3 готовые ветки - одним заходом, с перерывом в несколько минут
между заходами, а не каждая по готовности.

## CI: одна проверка, без опроса

```bash
gh run list --branch task/<коротко> -L 1          # когда прогон мог закончиться, не циклом
gh run view <id> --log-failed | tail -60          # красное: лог упавшего шага
gh run rerun <id> --failed                         # не код (сеть, раннер) - только упавшие джобы
```

Повтор той же джобы зелёный - нестабильность заводится задачей, а не
прячется. Зелёное локально и красное в CI - находка: окружение разное.

## Слияние и уборка

```bash
gh pr merge <N> --merge --delete-branch
git checkout main && git pull --ff-only && git branch -d task/<коротко>
```

Сливать по одному и сверять состояние после каждого (`gh pr view <N> --json state`):
пока PR ждал, `main` мог уйти вперёд.

## Ветка отстала от main

```bash
git fetch origin && git rebase origin/main
node tools/status.mjs                       # чужие слияния приносят тесты и модули
git push --force-with-lease origin task/<коротко>
```

Конфликт в `CHANGELOG.md` - оставить обе записи. Конфликт в `TODO.md` -
решать глазами: склейка даёт две правды о задаче.

## Параллельные агенты

1. Независимые задачи - агентам одним сообщением, каждому своя рабочая копия
   (`isolation: worktree`) и граница словами: какие каталоги правит, каких не
   трогает, что сдаёт (ветка, коммиты, отчёт). Без границы два агента правят
   один `CHANGELOG.md`.
2. Пушит основная сессия, агенты только коммитят.
3. У каждой копии своя папка сборки: общий `target/` держит блокировку cargo
   и подменяет exe под запущенным окном. Тестовое окно - свой порт
   (`tools/test-window.ps1 -Port 93xx`, скилл `pult-ui`).
4. Агент отвалился по сети - продолжить его `SendMessage`, а не заводить нового.
5. Две ветки правят одно место - договориться, кто правит, а кто переносит
   («кто сливается вторым, тот и поправит»), и записать в описании PR.

## Windows-ловушки

- Git Bash портит пути вида `rev:path`: `MSYS_NO_PATHCONV=1`.
- `git worktree remove` на длинном пути молчит: `rd /s /q \\?\<полный путь>`
  из cmd, затем `git worktree prune`.
- Процесс окна гасится по пути exe (`tools/test-window.ps1 -Stop`), не по имени:
  иначе погаснет окно, в котором работает человек.
