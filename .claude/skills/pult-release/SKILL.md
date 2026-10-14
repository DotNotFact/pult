---
name: pult-release
description: Рецепт релиза Пульта - поднять версию в Cargo.toml и app/tauri.conf.json, записать CHANGELOG, прогнать гейты, проверить установщик, поставить тег vX.Y.Z и выпустить GitHub Release с установщиком, zip и SHA256SUMS.txt (workflow по тегу или gh release create). Использовать, когда владелец сказал выпускать версию.
---

# Релиз: рецепт

Правила - `.claude/rules/release.md`. Пуш и тег - только по слову владельца.

## 1. Версия

Semver: исправления - `X.Y.Z+1`, новые возможности - `X.Y+1.0`, ломающие
данные или поведение - `X+1.0.0`. Поменять в двух местах одним коммитом:

- `Cargo.toml` -> `[workspace.package] version = "X.Y.Z"`
- `app/tauri.conf.json` -> `"version": "X.Y.Z"`

`cargo check --workspace` обновит `Cargo.lock`, он едет тем же коммитом.

## 2. CHANGELOG

Секцию `[Unreleased]` переименовать в `[X.Y.Z] - ГГГГ-ММ-ДД`, над ней
завести пустую `[Unreleased]`. Заметки релиза workflow берёт именно из этой
секции (`tools/release-notes.ps1`).

## 3. Гейты

```bash
node knowledge/validate.mjs
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
LC_ALL=C.UTF-8 grep -rnP "[\x{2014}\x{2013}]" CHANGELOG.md
```

Коммит «Пульт X.Y.Z», ветка `task/release-X.Y.Z`, PR, слияние на зелёном CI.

## 4. Тег и релиз

На `main` после слияния:

```bash
git checkout main && git pull --ff-only
git tag vX.Y.Z
git push origin vX.Y.Z
```

Тег запускает `.github/workflows/release.yml`: сборка, zip, установщик
`Pult_X.Y.Z_x64-setup.exe`, `SHA256SUMS.txt`, GitHub Release с заметками.
Версия тега обязана совпасть с `Cargo.toml` и `app/tauri.conf.json`.

Проверить сборку без публикации - ручной запуск workflow (черновик):

```bash
gh workflow run release.yml -f version=X.Y.Z -f draft=true
```

Если workflow недоступен - собрать локально и выпустить руками:

```powershell
pwsh -File tools/package.ps1 -Version X.Y.Z
pwsh -File tools/build-installer.ps1 -Version X.Y.Z
cd dist
# суммы обоих файлов одним файлом, формат sha256sum - как делает release.yml
$files = 'Pult_X.Y.Z_x64-setup.exe', 'pult-X.Y.Z-windows-x64.zip'
$lines = foreach ($f in $files) { "$((Get-FileHash $f).Hash.ToLowerInvariant())  $f" }
[IO.File]::WriteAllText("$PWD\SHA256SUMS.txt", ($lines -join "`n") + "`n")
gh release create vX.Y.Z @files SHA256SUMS.txt --title "Пульт X.Y.Z" --notes-file <заметки> --verify-tag
```

## 5. Установщик перед релизом

Проверить руками на своей машине - порядок в `.claude/rules/release.md`,
раздел «Проверка установщика руками». Данные Пульта
(`%LOCALAPPDATA%\dev.dotnotfact.pult`, `%APPDATA%\dev.dotnotfact.pult`)
перед проверкой удаления скопировать и потом вернуть. Если менялась версия
`@tauri-apps/cli` в `build-installer.ps1` - сначала сверить места
«ШАБЛОН» в `app/installer/hooks.nsh`.

## 6. После

```bash
gh release view vX.Y.Z
```

В релизе установщик, zip и `SHA256SUMS.txt`, заметки совпадают с секцией
CHANGELOG. Скачать установщик, поставить, запустить Пульт из меню «Пуск» и
`pult info` в новом терминале; скачать zip, распаковать, запустить
`Pult.exe` и `bin\pult.cmd info` в любой папке.
