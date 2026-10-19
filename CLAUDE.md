# Пульт - правила для ИИ-агента

Читать первым в каждой сессии. Здесь то, что нельзя нарушить, и куда идти
за подробностями. Устройство целиком - `docs/ARCHITECTURE.md`.

## Что это

Настольное приложение: открываешь папку любого проекта - Пульт находит все
скрипты (npm, Makefile, Cargo, dotnet, Docker Compose, `.ps1`/`.sh` и другие)
и объясняет каждую команду простыми словами: что делает, команда по частям,
что внутри, ключи, что затронет. Рядом - CI (GitLab и GitHub через `glab`/`gh`),
git, история запусков и уроки по технологиям проекта.

Две оболочки над одним ядром: **окно** (Tauri 2 + WebView2, `Pult.exe`) и
**терминал** (ratatui, `pult-cli.exe` + `pult.cmd`).

## Карта

```
core/        pult-core - вся логика
  src/scan.rs        разбор проекта: скрипты, стек, документы, health
  src/explain.rs     движок объяснений: команда по частям, шапка файла, эффекты
  src/kb.rs          загрузка базы знаний (диск, иначе копия в exe)
  src/git.rs         состояние репозитория: сначала свой разбор .git/config (доверие), потом git с SAFE
  src/ci.rs          пайплайны и джобы через glab/gh, только доверенные хосты
  src/ci_config.rs   разбор .gitlab-ci.yml и GitHub workflows
  src/runner.rs      запуск без оболочки, Job Object, открытие файлов
  src/store.rs       настройки, недавние, избранное, история, кэш
  src/helpcmd.rs     разбор вывода --help
  src/topics.rs      какие уроки нужны проекту (по detect из урока)
  src/ai.rs          ИИ-часть проекта: детали (инструкции, правила, агенты, хуки, MCP, проверки), связи, рассказ
  src/env.rs         какие программы есть, жив ли Docker
  src/paths.rs       где знания и где данные
  src/model.rs util.rs  типы и мелочи (макрос re!, cached_re, is_unc)
  examples/          scan.rs, prof.rs, perf.rs - прогон и замер ядра на настоящей папке
  tests/             тесты ядра (cargo test): ai, ci_config, explain, git_stash, kb, runner_util,
                     scan, security, store, sub_engine; common/ - общие помощники
app/         pult-app - окно Tauri: src/main.rs (команды IPC, события), src/tray.rs (трей,
             сценарии, размер окна), tauri.conf.json, capabilities/, icons/,
             installer/ (хуки NSIS, pult.cmd, картинки установщика)
cli/         pult-cli - терминал на ratatui: src/main.rs, build.rs
ui/          интерфейс окна: index.html, app.js, blocks.js, learn.js, scenarios.js, ai.js, widget.js,
             statusbar.js, titlebar.js, theme.js, *.css, img/
knowledge/   база знаний JSON: tools/*.json, topics/*.json, effects, stages,
             heuristics, manifests, stacks, docs, ui, glossary, icons, lessons,
             scenarios, ai (виды деталей ИИ-части); SCHEMA.md - формат, validate.mjs - проверка
tools/       install.ps1 и скрипты сборки; status.mjs (числа дерева), contrast.mjs (палитра),
             test-window.ps1 и cdp.mjs (тестовое окно и его замер)
.github/     CI и релиз (GitHub Actions), шаблоны issue и PR
.githooks/   хуки git: pre-commit, commit-msg, pre-push (включаются core.hooksPath)
.claude/     правила, агенты, скиллы, хук базы знаний, scripts/ (verify-gates, session-stats)
docs/        ARCHITECTURE.md, decisions.md, contract.md (кто что решает), status.md (числа,
             генерируется), optimizations.md (замеры), adr/, assets/ (картинки README)
```

Карта обязана совпадать с `ls`. Числа (сколько программ, уроков) здесь не
пишутся: их печатает `node knowledge/validate.mjs`.

## Четыре правила, которые не обсуждаются

1. **Знания - в данных, не в коде.** Любое пояснение, слово-признак этапа,
   стандартная команда манифеста, текст подсказки - в `knowledge/*.json`.
   Ни одного правила под конкретный проект. Владелец сильно презирает
   хардкод: увидел строку-пояснение в `.rs` или `.js` - это дефект.
2. **Запуск без оболочки.** Команды - списком аргументов (`runner.rs`),
   никаких `shell: true`, `cmd /c`, `sh -c`. git - с `-c core.fsmonitor=false`.
   Документы открываются программой по умолчанию только с текстовыми
   расширениями (`heuristics.docExtensions`). CI-запросы - только на
   доверенные хосты. Сетевые (UNC) пути не трогаем. `.env` не читаем.
   Подробно - `.claude/rules/security.md`.
3. **Окно и терминал - тонкие оболочки.** Логика в `core`, одинаковая для
   обоих. Если правка нужна и окну, и терминалу - она в ядре.
4. **Тексты.** Русский, простые слова для новичка, только дефис «-»
   (длинные и средние тире запрещены), без воды и ИИ-оборотов («давайте
   разберёмся», «важно отметить», «мощный», «является»). Проверяет
   `validate.mjs` для базы знаний; для кода и документов - тот же `grep`.

## Правило ноль: документ меняется вместе с кодом

Документ, разошедшийся с деревом, хуже отсутствующего: агент читает его
первым и планирует по нему. Правка из левой колонки обязана тем же коммитом
править правую.

| Что изменилось | Что обновить |
|---|---|
| Новый или удалённый модуль в `core/src`, каталог в корне | Карта в `CLAUDE.md`, `docs/ARCHITECTURE.md` |
| Новая или удалённая команда IPC, событие | `docs/ARCHITECTURE.md`, раздел IPC |
| Новый урок (id в `topics/`) | строка в `knowledge/lessons.json` (поле `order`; его читают окно, терминал и `validate.mjs`) |
| Новое поле в базе знаний | `knowledge/SCHEMA.md` и проверка в `validate.mjs` |
| Изменилась защита (запуск, пути, CI, CSP) | `.claude/rules/security.md`, раздел «Безопасность» в `README.md` |
| Изменилось поведение для пользователя | `CHANGELOG.md`, `[Unreleased]` |
| Закрыт пункт `TODO.md` | Снять пункт и записать исход в `CHANGELOG.md` |
| Владелец принял решение | Строка в `docs/decisions.md`; архитектурное - ADR в `docs/adr/` |
| Новое правило, агент или скилл | Таблица ниже и `.claude/rules/README.md` |
| Модуль, файл тестов или `#[test]`, команда IPC, урок, правило, агент, скилл, шаг CI | `node tools/status.mjs` (пишет `docs/status.md`; шаг CI сверяет) |

Числа дерева - только в `docs/status.md`, руками нигде не набираются.
Сверять командой, а не памятью: `ls core/src`, `grep -n generate_handler app/src/main.rs`.
Документ правится ровно там, где разошёлся, а не переписывается целиком.

## Рабочий цикл

1. **Ветка** `task/<коротко>` от свежего `main`. В `main` напрямую не коммитим.
2. **Решения** - сначала `grep` по `docs/decisions.md`, потом вопрос
   владельцу. Вопросы - одной пачкой до первой строки кода. Что решаю сам,
   что спрашиваю всегда - `docs/contract.md`.
3. **Код** в границах задачи. Знания - в JSON (правило 1).
4. **Гейты** - все четыре зелёные локально:

   ```bash
   node knowledge/validate.mjs
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```

   Плюс `node tools/status.mjs --check` и проверка тире в изменённых файлах:
   `LC_ALL=C.UTF-8 grep -rP "[\x{2014}\x{2013}]" <файлы>`. Во время работы -
   только затронутое (таблица в `.claude/rules/tokens.md`).
   Правка окна - снимок и замер через порт отладки (скилл `pult-ui`).
5. **Ревью** агентом `code-reviewer`; правка запуска, путей, CI или `ui` -
   ещё `security-reviewer`.
6. **Коммит только на зелёном.** Сообщение по-русски, первой строкой - что
   стало правдой. Отступление от правил называется прямо.
7. **Автор** только `DotNotFact <140886710+DotNotFact@users.noreply.github.com>`.
   Без `Co-Authored-By` и без строки «Generated with Claude Code» - ни в
   коммитах, ни в PR. Проверка: `git log --format='%an <%ae>' main..HEAD | sort -u`.
8. **Пуш** только по слову владельца «пушим», с паузой между пушами. PR на
   GitHub (`DotNotFact/pult`, публичный, open source под MIT), слияние на
   зелёном CI, ветка удаляется. CI не опрашивается. Порядок -
   `.claude/rules/git-flow.md`, команды - скилл `pult-flow`.

Хуки git включаются один раз на рабочую копию: `git config core.hooksPath .githooks`
(тире, `.env*`, база знаний, автор, строки про ИИ, пауза пушей).

## Правила, агенты, скиллы

| Файл | О чём | Когда грузится |
|---|---|---|
| `.claude/rules/README.md` | что трогаю - что читать | всегда |
| `.claude/rules/security.md` | список защит и почему | всегда |
| `.claude/rules/git-flow.md` | порядок задачи, хуки, пуш редко и пачкой, CI без опроса | всегда |
| `.claude/rules/verification.md` | «готово» - только машиной; диверсия с контролем; пол замера | всегда |
| `.claude/rules/tokens.md` | экономия токенов; что гонять под какую правку | всегда |
| `.claude/rules/knowledge-base.md` | формат базы, как добавить программу и урок | `knowledge/**` |
| `.claude/rules/rust-core.md` | границы модулей, без shell, скорость | `core/**`, `app/src/**`, `cli/**` |
| `.claude/rules/ui.md` | тексты из `S.kb`, `esc()`, проверка окна | `ui/**` |
| `.claude/rules/release.md` | версии, теги, что в архиве | `.github/**`, `tools/**`, `Cargo.toml` |

| Агент | Зачем |
|---|---|
| `code-reviewer` | враждебное ревью диффа до коммита |
| `security-reviewer` | shell, пути, XSS в `ui`, CI-хосты, fsmonitor |
| `kb-author` | пишет и правит JSON базы по `SCHEMA.md`, гоняет `validate.mjs` |
| `ui-qa` | окно через порт отладки: три ширины, обе темы |
| `architect` | план в Plan Mode, одна пачка вопросов, имена тестов и границы |
| `debugger` | первопричина до фикса |
| `test-runner` | гейты затронутой области, честно красное и зелёное |
| `perf-auditor` | замер попеременно, медиана, пол прибора |

| Скилл | Рецепт |
|---|---|
| `pult-kb` | добавить программу или урок, проверить на настоящем проекте |
| `pult-release` | поднять версию, CHANGELOG, тег, релиз |
| `pult-flow` | ветка, пуш с паузой, PR, одна проверка CI, слияние, параллельные агенты |
| `pult-ui` | тестовое окно на своём порту, `tools/cdp.mjs`: снимки и замеры |
| `feature` | фича в два шага: план без правок, потом выполнение |
| `fix-bug` | баг: первопричина, минимальный фикс, регресс-тест |

Хук: правка файла в `knowledge/` сразу прогоняет `validate.mjs` по этому
файлу (`.claude/settings.json`, `.claude/hooks/validate-kb.mjs`).
Что гейты умеют краснеть - `node .claude/scripts/verify-gates.mjs`; куда
ушли токены - `node .claude/scripts/session-stats.mjs`.

Что не сделано - `TODO.md`. Решения владельца - `docs/decisions.md`.
Числа дерева - `docs/status.md`.
