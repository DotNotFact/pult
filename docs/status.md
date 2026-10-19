# Состояние дерева

Собрано командой `node tools/status.mjs`. **Руками не правится:** следующий
запуск перезапишет файл целиком. Число здесь неверное - неверно дерево, а не
документ. Сверяет шаг CI `node tools/status.mjs --check`.

Здесь только то, что видно без запуска. Сколько проверок ПРОШЛО, знает CI.
Сколько программ в базе знаний - печатает `node knowledge/validate.mjs`.

| Что | Сколько | Что именно |
|---|---|---|
| Крейты | 3 | app, cli, core |
| Модули ядра | 16 | ai, ci, ci_config, env, explain, git, helpcmd, kb, lib, model, paths, runner, scan, store, topics, util |
| Команды IPC | 33 | `generate_handler!` в `app/src/main.rs` |
| Проверки в Rust | 154 | `#[test]` в app, cli, core |
| Файлы тестов ядра | 11 | ai, ci_config, explain, git_stash, helpcmd, kb, runner_util, scan, security, store, sub_engine |
| Уроки | 26 | `knowledge/topics/` |
| Группы программ в базе | 16 | data, docker, dotnet, files, frontend, git, go, infra, jvm, node, php, python, quality, rust, shell, testing |
| Скрипты | 9 | `build-installer.ps1`, `cdp.mjs`, `contrast.mjs`, `install.ps1`, `installer-art.ps1`, `package.ps1`, `release-notes.ps1`, `status.mjs`, `test-window.ps1` |
| Файлы правил | 9 | README, git-flow, knowledge-base, release, rust-core, security, tokens, ui, verification |
| Агенты | 8 | architect, code-reviewer, debugger, kb-author, perf-auditor, security-reviewer, test-runner, ui-qa |
| Скиллы | 6 | feature, fix-bug, pult-flow, pult-kb, pult-release, pult-ui |
| Хуки git | 3 | commit-msg, pre-commit, pre-push |
| Решения (ADR) | 4 | `docs/adr/` |

## Шаги CI

| Workflow | Шагов |
|---|---|
| `ci.yml` | 10 |
| `release.yml` | 10 |
