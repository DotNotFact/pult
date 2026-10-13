# Кладёт собранные Pult.exe и pult.exe в bin\ и ярлыки на рабочий стол.
# Запускать после сборки: cargo build --release ; pwsh -File make-shortcut.ps1
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$bin = Join-Path $here 'bin'
$rel = Join-Path $here 'target\release'
New-Item -ItemType Directory -Force $bin | Out-Null
foreach ($exe in 'Pult.exe', 'pult-cli.exe') {
  $src = Join-Path $rel $exe
  if (-not (Test-Path $src)) { throw "Нет $src - сначала соберите: cargo build --release" }
}
# окно может быть открыто - тогда exe занят; закрываем только наш Pult.exe из bin
Get-Process Pult -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($bin, [StringComparison]::OrdinalIgnoreCase) } | Stop-Process -Force
Start-Sleep -Milliseconds 300
Copy-Item (Join-Path $rel 'Pult.exe') (Join-Path $bin 'Pult.exe') -Force
Copy-Item (Join-Path $rel 'pult-cli.exe') (Join-Path $bin 'pult-cli.exe') -Force
# pult.exe и Pult.exe в одной папке на Windows - одно имя, поэтому консольная версия называется pult-cli.exe,
# а короткую команду pult даёт pult.cmd рядом
Set-Content (Join-Path $bin 'pult.cmd') "@echo off`r`n`"%~dp0pult-cli.exe`" %*" -Encoding ascii

$icon = Join-Path $here 'app\icons\icon.ico'
$desktop = [Environment]::GetFolderPath('Desktop')
$sh = New-Object -ComObject WScript.Shell

# старые ярлыки прошлой версии (через PowerShell и Node) - заменяем
foreach ($old in 'Пульт — консоль.lnk') { $p = Join-Path $desktop $old; if (Test-Path $p) { Remove-Item $p } }

$s = $sh.CreateShortcut((Join-Path $desktop 'Пульт.lnk'))
$s.TargetPath = Join-Path $bin 'Pult.exe'
$s.Arguments = ''
$s.WorkingDirectory = $bin
$s.WindowStyle = 1
$s.IconLocation = $icon
$s.Description = 'Пульт - управление любым проектом: скрипты с объяснениями, уроки, CI'
$s.Save()

$c = $sh.CreateShortcut((Join-Path $desktop 'Пульт - консоль.lnk'))
$wt = (Get-Command wt.exe -ErrorAction SilentlyContinue).Source
if ($wt) {
  $c.TargetPath = $wt
  $c.Arguments = '--title "Пульт" "' + (Join-Path $bin 'pult-cli.exe') + '"'
} else {
  $c.TargetPath = Join-Path $bin 'pult-cli.exe'
  $c.Arguments = ''
}
$c.WorkingDirectory = $bin
$c.IconLocation = $icon
$c.Description = 'Пульт в терминале: скрипты, объяснения, уроки и CI без окна'
$c.Save()
Write-Output "ok: $bin"
