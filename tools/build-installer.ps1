# Собирает установщик Пульта: dist\Pult_<версия>_x64-setup.exe и рядом .sha256.
# Установщик - NSIS от Tauri, ставит на одного пользователя без прав администратора
# (настройки - app/tauri.conf.json, раздел bundle; хуки - app/installer/hooks.nsh; ADR 0004).
# Тот же скрипт зовёт CI (.github/workflows/release.yml) - локальный установщик и установщик релиза одинаковы.
#
#   pwsh -File tools/build-installer.ps1                  - pult-cli, затем окно и установщик через Tauri CLI
#   pwsh -File tools/build-installer.ps1 -Version 2.1.0   - упасть, если версия не совпала с Cargo.toml
#   pwsh -File tools/build-installer.ps1 -SignCommand 'signtool sign /fd sha256 /a "%1"'  - с подписью
#
# Глобально ничего не ставится: Tauri CLI берётся через npx закреплённой версии ($TauriCli ниже).
# Кэш npm и инструменты Tauri (NSIS, загрузчик WebView2) - в -CacheDir, если он задан или есть D:\dev-cache;
# иначе npm пишет в свой кэш, а инструменты Tauri лежат в target\.tauri (bundle.useLocalToolsDir).
[CmdletBinding()]
param(
  # версия для сверки (как у package.ps1); по умолчанию - из Cargo.toml
  [string]$Version,
  # куда положить установщик; по умолчанию dist\ в корне репозитория
  [string]$OutDir,
  # общая папка кэшей; пусто - D:\dev-cache, если она есть
  [string]$CacheDir,
  # команда подписи для Tauri (bundle.windows.signCommand), %1 - путь к файлу; пусто - без подписи
  [string]$SignCommand
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3

# версия Tauri CLI закреплена: от неё зависит шаблон NSIS, в который встают хуки (hooks.nsh)
$TauriCli = '2.12.1'

$root = Split-Path $PSScriptRoot -Parent
$app = Join-Path $root 'app'
$target = Join-Path $root 'target'
if (-not $OutDir) { $OutDir = Join-Path $root 'dist' }
if (-not $CacheDir -and (Test-Path 'D:\dev-cache')) { $CacheDir = 'D:\dev-cache' }

# версия из Cargo.toml ([workspace.package]) и из app/tauri.conf.json обязаны совпасть:
# первую показывает exe, вторую - установщик и «Установленные приложения»
function Get-CargoVersion {
  $section = ''
  foreach ($line in Get-Content (Join-Path $root 'Cargo.toml')) {
    if ($line -match '^\s*\[([^\]]+)\]\s*$') { $section = $Matches[1].Trim(); continue }
    if ($section -eq 'workspace.package' -and $line -match '^\s*version\s*=\s*"([^"]+)"') { return $Matches[1] }
  }
  throw 'В Cargo.toml нет version в секции [workspace.package]'
}
$cargoVersion = Get-CargoVersion
$tauriVersion = (Get-Content (Join-Path $app 'tauri.conf.json') -Raw | ConvertFrom-Json).version
if ($tauriVersion -ne $cargoVersion) {
  throw "Версия в app/tauri.conf.json ($tauriVersion) не совпадает с Cargo.toml ($cargoVersion) - поменяйте обе"
}
if ($Version) {
  $Version = $Version -replace '^v', ''
  if ($Version -ne $cargoVersion) { throw "Версия $Version не совпадает с Cargo.toml ($cargoVersion)" }
}
$Version = $cargoVersion

foreach ($tool in 'cargo', 'npx') {
  if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "Нет $tool в PATH: нужны Rust stable и Node 22" }
}

if ($CacheDir) {
  New-Item -ItemType Directory -Force $CacheDir | Out-Null
  $env:npm_config_cache = Join-Path $CacheDir 'npm'
  # Tauri кладёт NSIS и загрузчик WebView2 в target\.tauri; делаем её ссылкой на общий кэш,
  # чтобы инструменты не качались заново после cargo clean и не лежали в каждом проекте
  $tools = Join-Path $CacheDir 'tauri'
  $link = Join-Path $target '.tauri'
  New-Item -ItemType Directory -Force $tools, $target | Out-Null
  if (-not (Test-Path $link)) {
    New-Item -ItemType Junction -Path $link -Target $tools | Out-Null
  }
}

Push-Location $root
try {
  # 1. терминальная версия: установщик берёт готовый target\release\pult-cli.exe (app/installer/tauri.installer.json)
  cargo build --release -p pult-cli
  if ($LASTEXITCODE) { throw "cargo build -p pult-cli завершился с кодом $LASTEXITCODE" }
} finally { Pop-Location }

$cli = Join-Path $target 'release\pult-cli.exe'
# pult-cli собран без статической библиотеки C++ - без VC++ Redistributable на машине он не запустится
$bytes = [IO.File]::ReadAllBytes($cli)
if ([Text.Encoding]::ASCII.GetString($bytes).IndexOf('VCRUNTIME140.dll', [StringComparison]::OrdinalIgnoreCase) -ge 0) {
  Write-Warning 'pult-cli.exe зависит от VCRUNTIME140.dll: на машине без Visual C++ Redistributable команда pult не запустится. Лечится статической сборкой рантайма в cli (см. docs/adr/0004-installer.md)'
}

# 2. окно и установщик. Ресурсы установщика (pult-cli.exe, pult.cmd) - в отдельном файле настроек:
#    в app/tauri.conf.json их нельзя, обычная cargo build окна требовала бы готовый pult-cli.exe
$tauriArgs = @('--yes', "@tauri-apps/cli@$TauriCli", 'build', '--ci', '--bundles', 'nsis', '--config', 'installer/tauri.installer.json')
if ($SignCommand) {
  $sign = @{ bundle = @{ windows = @{ signCommand = $SignCommand } } } | ConvertTo-Json -Depth 5 -Compress
  $tauriArgs += @('--config', $sign)
}
Push-Location $app
try {
  npx @tauriArgs
  if ($LASTEXITCODE) { throw "tauri build завершился с кодом $LASTEXITCODE" }
} finally { Pop-Location }

# 3. хуки опираются на переменные шаблона Tauri: если шаблон их переименовал, makensis не упал бы,
#    а хуки молча перестали бы работать - сверяем по готовому installer.nsi
$nsi = Join-Path $target 'release\nsis\x64\installer.nsi'
if (-not (Test-Path $nsi)) { throw "Нет $nsi - Tauri положил шаблон в другое место, сверьте hooks.nsh с новым шаблоном" }
$nsiText = Get-Content $nsi -Raw
foreach ($need in 'Var PassiveMode', 'Var UpdateMode', 'Var DeleteAppDataCheckboxState', 'hooks.nsh') {
  if (-not $nsiText.Contains($need)) { throw "В шаблоне Tauri $TauriCli нет «$need» - сверьте app/installer/hooks.nsh с $nsi" }
}

$name = "Pult_${Version}_x64-setup.exe"
$built = Join-Path $target "release\bundle\nsis\$name"
if (-not (Test-Path $built)) { throw "Нет $built" }
New-Item -ItemType Directory -Force $OutDir | Out-Null
$setup = Join-Path $OutDir $name
$shaFile = "$setup.sha256"
Copy-Item $built $setup -Force
$hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText($shaFile, "$hash  $name`n", [Text.Encoding]::ASCII)

if ($env:GITHUB_OUTPUT) {
  Add-Content $env:GITHUB_OUTPUT -Encoding utf8 -Value @("setup=$setup", "setupsha256file=$shaFile")
}

$mb = [math]::Round((Get-Item $setup).Length / 1MB, 2)
Write-Output "Установщик: $setup ($mb МБ)"
Write-Output "SHA256:     $hash"
