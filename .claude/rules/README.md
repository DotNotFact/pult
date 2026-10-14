# Какие правила читать под какую задачу

Сначала `CLAUDE.md`, потом одна строка из таблицы. Остальное не открывать,
пока не понадобится. Правила с шапкой `paths:` подгружаются сами, когда
агент читает или правит подходящие файлы; без шапки всегда загружены этот
файл, `security.md`, `git-flow.md`, `verification.md` и `tokens.md` - все
короткие.

| Трогаю | Читать обязательно | Заглянуть, если сомневаюсь |
|---|---|---|
| `knowledge/*.json`, новая программа или урок | `knowledge-base.md`, скилл `pult-kb` | `knowledge/SCHEMA.md` |
| `core/src/**` | `rust-core.md`, `security.md` | `docs/ARCHITECTURE.md` |
| `app/src/**` (main.rs, tray.rs), IPC, события | `rust-core.md`, `security.md` | `docs/ARCHITECTURE.md` |
| `cli/src/**` | `rust-core.md` | - |
| `ui/**` | `ui.md`, `security.md`, скилл `pult-ui` | агент `ui-qa` до коммита |
| `app/tauri.conf.json`, `capabilities/` | `security.md` | `release.md` |
| `.github/**`, `tools/**`, `Cargo.toml`, версия | `release.md`, скилл `pult-release` | - |
| Запуск процессов, пути, git, CI | `security.md` | агент `security-reviewer` |
| Новая фича | скилл `feature` | агент `architect` |
| Баг, красный тест | скилл `fix-bug` | агент `debugger` |
| Обещание «быстрее», «легче» | `verification.md`, раздел «Замер» | агент `perf-auditor`, `docs/optimizations.md` |
| `.githooks/**`, `tools/status.mjs`, новый гейт | `git-flow.md`, `verification.md` | `node .claude/scripts/verify-gates.mjs` |
| Ветка, пуш, PR, красный CI, параллельные агенты | `git-flow.md`, скилл `pult-flow` | `docs/contract.md` |
| Любой код перед коммитом | гейты из `CLAUDE.md`, таблица в `tokens.md` | агенты `test-runner`, `code-reviewer` |

Решения владельца - `docs/decisions.md` (грепать до вопроса). Кто что
решает - `docs/contract.md`. Почему так устроено - `docs/adr/`. Числа
дерева - `docs/status.md` (генерируется, руками не правится).
