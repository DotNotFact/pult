# Тестовое окно Пульта: отладочная сборка с портом CDP, своя папка сборки и свои данные.
#
# Зачем всё своё. Окно, в котором работает человек, и окна соседних агентов
# не должны пересекаться с проверкой: у каждого порта своя папка сборки
# (иначе cargo ждёт блокировку чужой сборки или подменяет exe под запущенным
# окном), свои данные Пульта (PULT_DATA) и своя папка WebView2
# (pult-test-webview-<порт> во временной папке, её выбирает app/src/main.rs).
# Идентификатор у тестового окна свой (<обычный>.test-<порт>, app/src/main.rs): защита от второго
# экземпляра и файл места и размера окна - свои на порт, обычное окно они не трогают.
#
#   pwsh -File tools/test-window.ps1                       порт 9222, папка проекта - этот репозиторий
#   pwsh -File tools/test-window.ps1 -Port 9335 -Project D:\путь\к\проекту
#   pwsh -File tools/test-window.ps1 -Port 9335 -NoBuild   перезапуск без сборки
#   pwsh -File tools/test-window.ps1 -Port 9335 -Stop      только погасить
#
# Потом: node tools/cdp.mjs --port 9335 check 900 700 (tools/cdp.mjs).

param(
  [int]$Port = 9222,
  [string]$Project = '',
  [string]$Target = '',
  [string]$Data = '',
  [switch]$NoBuild,
  [switch]$Stop
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8   # иначе русский вывод в Git Bash приходит кракозябрами
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $Project) { $Project = $root }
if (-not $Target) { $Target = Join-Path $root "target\test-$Port" }
if (-not $Data) { $Data = Join-Path ([IO.Path]::GetTempPath()) "pult-test-data-$Port" }
$exe = Join-Path $Target 'debug\Pult.exe'

# Гасится только своё: exe из этой папки сборки и WebView2 с папкой этого порта.
Get-Process Pult -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | Stop-Process -Force
Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" |
  Where-Object { $_.CommandLine -like "*pult-test-webview-$Port*" } |
  ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
if ($Stop) { "погашено: порт $Port"; exit 0 }
Start-Sleep -Milliseconds 500

if (-not $NoBuild) {
  Push-Location $root
  # cargo пишет ход сборки в stderr; при Stop Windows PowerShell 5.1 счёл бы первую строку ошибкой
  $ErrorActionPreference = 'Continue'
  try {
    cargo build -p pult-app --target-dir $Target 2>&1 |
      ForEach-Object { "$_" } | Select-String -Pattern '^error|^warning|Finished' | ForEach-Object { $_.Line }
    if ($LASTEXITCODE -ne 0) { throw "сборка не прошла (код $LASTEXITCODE)" }
  } finally { Pop-Location; $ErrorActionPreference = 'Stop' }
}
if (-not (Test-Path $exe)) { throw "нет $exe - соберите без -NoBuild" }

$env:PULT_DEBUG_PORT = "$Port"
$env:PULT_DATA = $Data
Start-Process $exe -ArgumentList "`"$Project`""

for ($i = 0; $i -lt 40; $i++) {
  Start-Sleep -Milliseconds 500
  try {
    if ((Invoke-WebRequest "http://127.0.0.1:$Port/json" -UseBasicParsing -TimeoutSec 1).StatusCode -eq 200) {
      Start-Sleep 2   # первая отрисовка и команда state
      "окно готово: порт $Port, сборка $Target, данные $Data"
      exit 0
    }
  } catch { $null = $_ }   # порт ещё закрыт - ждём дальше
}
throw "порт $Port не открылся за 20 с: смотрите, запустился ли $exe"
