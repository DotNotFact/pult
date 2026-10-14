---
paths:
  - ".github/**"
  - "tools/**"
  - "Cargo.toml"
  - "app/tauri.conf.json"
  - "app/installer/**"
  - "CHANGELOG.md"
---

# Релиз и сборка

Рецепт по шагам - скилл `pult-release`. Здесь то, что ломается молча.
Почему установщик устроен так - `docs/adr/0004-installer.md`.

## Версия в двух местах

- `Cargo.toml`, `[workspace.package] version` - её берут все три крейта.
- `app/tauri.conf.json`, `"version"` - её показывает окно, ею называется
  установщик и её видно в «Установленных приложениях».

Обе меняются одним коммитом. Тег `vX.Y.Z` ставится на этот коммит; на теге
`release.yml` требует, чтобы версия совпала с `Cargo.toml`, иначе exe
показал бы не ту версию, что в релизе. `tools/build-installer.ps1` падает,
если две версии разошлись. Версия с дефисом (`2.1.0-rc.1`) уходит как
пре-релиз.

## Что делают workflow

| Файл | Когда | Что |
|---|---|---|
| `.github/workflows/ci.yml` | push в `main`, PR | `validate.mjs`, `cargo fmt --check`, `clippy -D warnings`, `cargo test` на Windows |
| `.github/workflows/release.yml` | тег `v*.*.*` или ручной запуск | проверка базы, zip (`package.ps1`), установщик (`build-installer.ps1`), `SHA256SUMS.txt`, GitHub Release с заметками из `CHANGELOG.md` |

Ручной запуск `release.yml` по умолчанию делает черновик - так проверяют
сборку, ничего не публикуя.

## Что в релизе

| Файл | Кто собирает | Внутри |
|---|---|---|
| `Pult_<версия>_x64-setup.exe` | `tools/build-installer.ps1` | установщик NSIS на одного пользователя: `Pult.exe`, `pult-cli.exe`, `bin\pult.cmd`, удалятор |
| `pult-<версия>-windows-x64.zip` | `tools/package.ps1` | переносная версия: те же exe, `bin\pult.cmd`, папка `knowledge/`, README, LICENSE |
| `SHA256SUMS.txt` | `release.yml` | суммы двух файлов в формате `sha256sum` |

Скрипты одни и те же локально и в CI, чтобы файлы не расходились. Сверять
состав с самими скриптами, а не с этой таблицей. `pult.cmd` один на всех -
`app/installer/pult.cmd`, скрипты его копируют. Два exe с именем `pult` в
одной папке на Windows совпали бы, поэтому терминал - `pult-cli.exe`, а
`pult.cmd` лежит в `bin\`: рядом с `Pult.exe` команда `pult` открыла бы окно
(`PATHEXT` ставит `.EXE` раньше `.CMD`). В PATH идёт `bin`, а не папка программы.

## Установщик: что ломается молча

- **Настройки** - `app/tauri.conf.json`, раздел `bundle`. Ресурсы
  установщика (`pult-cli.exe`, `bin\pult.cmd`) - в `app/installer/tauri.installer.json`,
  его передаёт `--config`. В `tauri.conf.json` их класть нельзя: сборка
  окна (`tauri-build`) копирует ресурсы и упала бы без готового
  `target\release\pult-cli.exe` - вместе с `cargo test` и `clippy`.
- **Tauri CLI закреплён** (`$TauriCli` в `build-installer.ps1`) и берётся
  через `npx`. Хуки `app/installer/hooks.nsh` встают в шаблон NSIS этой
  версии и трогают его в трёх местах «ШАБЛОН». Новая версия CLI - сверить их
  с её `installer.nsi` и `utils.nsh`; скрипт сам проверяет переменные шаблона.
- **Ключи командной строки** установщика и удалятора ищутся по началу
  слова: шаблон занял `/P`, `/NS`, `/R`, `/ARGS`, `/UPDATE`, свои ключи
  (`/ADDTOPATH`, `/WIPEDATA`) с этих букв не начинаются.
- **Данные** (`%LOCALAPPDATA%\dev.dotnotfact.pult`, `%APPDATA%\dev.dotnotfact.pult`)
  ни в zip, ни в установщик не попадают, обновление их не трогает. Удаляет
  их только удаление с ответом «Да» или ключом `/WIPEDATA`. Папка `data/` в
  корне репозитория - след старой версии, она в `.gitignore`.
- **Инструменты Tauri** (NSIS, загрузчик WebView2) лежат в `target\.tauri`
  (`bundle.useLocalToolsDir`), а не в `%LOCALAPPDATA%\tauri`. С
  `-CacheDir` или при наличии `D:\dev-cache` это ссылка на `<кэш>\tauri`.
- **Картинки** установщика - BMP в `app/installer/`, перерисовывает их
  `tools/installer-art.ps1`.
- **Подпись**: сертификата нет. Когда появится -
  `build-installer.ps1 -SignCommand '<команда> "%1"'` или
  `bundle.windows.signCommand`; Tauri подпишет exe, удалятор и установщик.
- **`pult-cli.exe` зависит от `VCRUNTIME140.dll`** (окно - нет, `tauri-build`
  собирает рантайм статически). Скрипт предупреждает об этом при каждой
  сборке, пока в `cli` не включат статическую сборку рантайма.

## Проверка установщика руками

На своей машине, перед релизом. Данные Пульта сначала скопировать.

1. `Pult_<версия>_x64-setup.exe /S /ADDTOPATH /D=<пустая папка>`: в папке
   `Pult.exe`, `pult-cli.exe`, `bin\pult.cmd`, `uninstall.exe`; ярлык
   `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Pult.lnk`; ключ
   `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Pult`; папка `bin` в
   `HKCU\Environment\Path`.
2. Запустить `Pult.exe` из папки: окно без консоли; `where pult` в новом
   терминале находит `bin\pult.cmd`, а не `Pult.exe`; `pult info` печатает сводку.
3. `uninstall.exe /S`: папки программы, ярлыков и ключа нет, данные на месте.
4. Поставить снова, `uninstall.exe /S /WIPEDATA`: нет и обеих папок данных.
5. Вернуть скопированные данные.

## Чего нет

Подписи exe, MSI, сборок для Linux и macOS - см. `TODO.md`.
