# Собирает архив релиза pult-<версия>-windows-x64.zip в dist\ из target\release.
# Тот же скрипт зовёт CI (.github/workflows/release.yml) - локальный архив и архив релиза одинаковы.
#
#   pwsh -File tools/package.ps1                 - cargo build --release, затем zip; версия из Cargo.toml
#   pwsh -File tools/package.ps1 -NoBuild        - только zip из уже собранного target\release
#   pwsh -File tools/package.ps1 -Version 2.1.0  - другая версия в имени архива
#
# В архиве: Pult.exe, pult-cli.exe, bin\pult.cmd, knowledge\ (без validate.mjs), README.md, LICENSE (если есть).
# Рядом кладётся <архив>.sha256 в формате sha256sum: "<хеш>  <имя файла>".
# Это переносная версия; установщик собирает tools/build-installer.ps1. Данные обе версии хранят в одном
# месте - %LOCALAPPDATA%\dev.dotnotfact.pult и %APPDATA%\dev.dotnotfact.pult (docs/adr/0004-installer.md).
[CmdletBinding()]
param(
  # версия в имени архива; по умолчанию - [workspace.package] version из Cargo.toml
  [string]$Version,
  # не запускать cargo build - взять готовые exe из target\release
  [switch]$NoBuild,
  # упасть, если -Version не совпадает с Cargo.toml (так делает CI на теге)
  [switch]$StrictVersion,
  # куда положить архив; по умолчанию dist\ в корне репозитория
  [string]$OutDir
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3

$root = Split-Path $PSScriptRoot -Parent
$rel = Join-Path $root 'target\release'
if (-not $OutDir) { $OutDir = Join-Path $root 'dist' }

# версия из Cargo.toml: строка version внутри секции [workspace.package]
function Get-CargoVersion {
  $section = ''
  foreach ($line in Get-Content (Join-Path $root 'Cargo.toml')) {
    if ($line -match '^\s*\[([^\]]+)\]\s*$') { $section = $Matches[1].Trim(); continue }
    if ($section -eq 'workspace.package' -and $line -match '^\s*version\s*=\s*"([^"]+)"') { return $Matches[1] }
  }
  throw 'В Cargo.toml нет version в секции [workspace.package]'
}

$cargoVersion = Get-CargoVersion
if (-not $Version) { $Version = $cargoVersion }
$Version = $Version -replace '^v', ''
if ($Version -notmatch '^\d+\.\d+\.\d+([-+][0-9A-Za-z.\-+]+)?$') { throw "Версия '$Version' не похожа на X.Y.Z" }
if ($Version -ne $cargoVersion) {
  $msg = "Версия $Version не совпадает с Cargo.toml ($cargoVersion) - exe покажет $cargoVersion"
  if ($StrictVersion) { throw $msg } else { Write-Warning $msg }
}

if (-not $NoBuild) {
  Push-Location $root
  try {
    cargo build --release -p pult-app -p pult-cli
    if ($LASTEXITCODE) { throw "cargo build завершился с кодом $LASTEXITCODE" }
  } finally { Pop-Location }
}

foreach ($exe in 'Pult.exe', 'pult-cli.exe') {
  if (-not (Test-Path (Join-Path $rel $exe))) { throw "Нет $rel\$exe - соберите: cargo build --release -p pult-app -p pult-cli" }
}

$name = "pult-$Version-windows-x64"
$stage = Join-Path $OutDir $name
$zip = Join-Path $OutDir "$name.zip"
$shaFile = "$zip.sha256"

New-Item -ItemType Directory -Force $OutDir | Out-Null
foreach ($p in $stage, $zip, $shaFile) { if (Test-Path $p) { Remove-Item $p -Recurse -Force } }
New-Item -ItemType Directory $stage | Out-Null

Copy-Item (Join-Path $rel 'Pult.exe') $stage
Copy-Item (Join-Path $rel 'pult-cli.exe') $stage
# pult.exe и Pult.exe в одной папке на Windows - одно имя, поэтому короткую команду pult даёт обёртка.
# Она лежит в bin\, как в установке: рядом с Pult.exe команда pult открыла бы окно (PATHEXT ставит .EXE
# раньше .CMD), а папку bin\ можно добавить в PATH. Файл тот же, что ставит установщик.
New-Item -ItemType Directory (Join-Path $stage 'bin') | Out-Null
Copy-Item (Join-Path $root 'app\installer\pult.cmd') (Join-Path $stage 'bin\pult.cmd')

Copy-Item (Join-Path $root 'knowledge') (Join-Path $stage 'knowledge') -Recurse
# проверка базы знаний нужна разработчику, а не пользователю
Remove-Item (Join-Path $stage 'knowledge\validate.mjs') -ErrorAction SilentlyContinue

foreach ($f in 'README.md', 'LICENSE', 'LICENSE.md', 'LICENSE.txt') {
  $src = Join-Path $root $f
  if (Test-Path $src) { Copy-Item $src $stage }
}

# в архиве одна папка pult-<версия>-windows-x64 - распаковка не рассыпает файлы
Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::CreateFromDirectory($stage, $zip, [IO.Compression.CompressionLevel]::Optimal, $true)
Remove-Item $stage -Recurse -Force

$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText($shaFile, "$hash  $name.zip`n", [Text.Encoding]::ASCII)

if ($env:GITHUB_OUTPUT) {
  Add-Content $env:GITHUB_OUTPUT -Encoding utf8 -Value @("version=$Version", "zip=$zip", "sha256file=$shaFile")
}

Write-Output "Архив:  $zip"
Write-Output "SHA256: $hash"
