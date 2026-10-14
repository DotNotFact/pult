# Пульт - AGENTS.md

Для инструментов, которые читают `AGENTS.md`. Полное описание - `CLAUDE.md`,
устройство - `docs/ARCHITECTURE.md`. Здесь минимум, который нужен любому агенту.

## Продукт

Настольное приложение: открывает папку проекта, находит все скрипты и
объясняет каждую команду простыми словами. Плюс CI, git, история запусков,
уроки. Ядро `core/` на Rust, две оболочки: окно `app/` (Tauri 2) и терминал
`cli/` (ratatui). Интерфейс окна - `ui/`, база знаний - `knowledge/`.

## Четыре правила

1. Знания - в `knowledge/*.json`, не в коде. Ни одного правила под конкретный проект.
2. Запуск без оболочки, списком аргументов; git с `core.fsmonitor=false`;
   CI только на доверенные хосты; UNC не трогаем; `.env` не читаем.
3. Логика - в `core`, окно и терминал только показывают.
4. Тексты: русский, простые слова, только дефис «-», без воды.

## Гейты перед коммитом

```bash
node knowledge/validate.mjs
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
node tools/status.mjs --check   # после правки модулей, тестов, IPC, правил: сначала node tools/status.mjs
```

Ветка `task/<коротко>`, коммит только на зелёном. Автор - только
`DotNotFact <140886710+DotNotFact@users.noreply.github.com>`, без
`Co-Authored-By` и без строки «Generated with Claude Code».

Хуки: `git config core.hooksPath .githooks` (тире, `.env*`, база, автор,
пауза между пушами). Пуш - только по слову владельца. Кто что решает -
`docs/contract.md`.
