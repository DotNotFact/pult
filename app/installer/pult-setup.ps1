# Помощник установщика Пульта - его зовёт app/installer/hooks.nsh, внутри установщика и удалятора он едет
# во временную папку NSIS. Делает то, что на NSIS из Tauri ненадёжно: строка там ограничена 1024 знаками,
# а процессов NSIS не видит.
#
#   -Action path-add      добавить папку -Dir в конец PATH пользователя, если её там нет
#   -Action path-remove   убрать из PATH пользователя все записи, равные -Dir
#   -Action webview-stop  остановить WebView2 закрытого Пульта, который ещё держит папку данных -Dir
#                         (EBWebView), и дождаться, пока он её отпустит
#
# PATH: остальные записи остаются как были, %ПЕРЕМЕННЫЕ% не раскрываются, тип значения (обычно
# REG_EXPAND_SZ) сохраняется. Папка приходит аргументом, а не текстом команды.
# WebView2: после того как удалятор закрыл Pult.exe, его процесс msedgewebview2 живёт ещё минуты и пишет
# в EBWebView - папка, удалённая по просьбе «удалить всё», появлялась снова. Останавливаем только процессы
# с этой папкой данных, у которых Pult уже закрыт; WebView2 Пульта, запущенного из другой папки, не трогаем.
#
# Код выхода: 10 - делать нечего, 11 - сделано, 12 - папку держит другой запущенный Пульт, иначе ошибка
# (PowerShell, заблокированный политикой, сам вернёт 1 - поэтому свои коды не 0 и 1).
param(
  [Parameter(Mandatory)][ValidateSet('path-add', 'path-remove', 'webview-stop')][string]$Action,
  [Parameter(Mandatory)][string]$Dir
)
$ErrorActionPreference = 'Stop'

function Edit-UserPath([bool]$add, [string]$dir) {
  $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
  $kind = [Microsoft.Win32.RegistryValueKind]::ExpandString
  if ($key.GetValueNames() -contains 'Path') {
    $kind = $key.GetValueKind('Path')
    if ($kind -ne 'String' -and $kind -ne 'ExpandString') { return 3 }
  }
  $old = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
  $parts = if ($old) { $old.Split(';') } else { @() }
  $isMine = { param($p) $p.Trim().TrimEnd('\') -ieq $dir }
  $found = @($parts | Where-Object { & $isMine $_ }).Count -gt 0
  if ($add) {
    if ($found) { return 10 }
    $new = if (-not $old) { $dir } elseif ($old.EndsWith(';')) { $old + $dir } else { "$old;$dir" }
  } else {
    if (-not $found) { return 10 }
    $new = @($parts | Where-Object { -not (& $isMine $_) }) -join ';'
  }
  if ($new) { $key.SetValue('Path', $new, $kind) } else { $key.DeleteValue('Path', $false) }
  $key.Close()
  return 11
}

function Stop-OrphanWebView([string]$udd) {
  $all = @(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'")
  $result = 10
  foreach ($p in $all) {
    $cmd = [string]$p.CommandLine
    # главный процесс WebView2 - без --type=; папка данных - в --user-data-dir
    if ($cmd -match '--type=') { continue }
    if ($cmd -notmatch '--user-data-dir=(?:"([^"]+)"|(\S+))') { continue }
    $d = if ($Matches[1]) { $Matches[1] } else { $Matches[2] }
    if ($d.TrimEnd('\') -ine $udd) { continue }
    if (Get-Process -Id $p.ParentProcessId -ErrorAction SilentlyContinue) { $result = 12; continue }
    foreach ($k in @($all | Where-Object { $_.ParentProcessId -eq $p.ProcessId })) {
      Stop-Process -Id $k.ProcessId -Force -ErrorAction SilentlyContinue
    }
    Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue
    if ($result -eq 10) { $result = 11 }
  }
  # Chromium держит lockfile в папке данных, пока жив; ждём, пока его отпустят
  $lock = Join-Path $udd 'lockfile'
  for ($i = 0; $i -lt 20 -and (Test-Path -LiteralPath $lock); $i++) {
    try { [IO.File]::Open($lock, 'Open', 'Read', 'None').Dispose(); break } catch { Start-Sleep -Milliseconds 250 }
  }
  return $result
}

try {
  $dir = $Dir.Trim().TrimEnd('\')
  if (-not $dir) { exit 2 }
  switch ($Action) {
    'path-add' { exit (Edit-UserPath $true $dir) }
    'path-remove' { exit (Edit-UserPath $false $dir) }
    'webview-stop' { exit (Stop-OrphanWebView $dir) }
  }
} catch {
  exit 4
}
