# Кладёт собранные Pult.exe и pult-cli.exe в bin\ в корне репозитория и ярлыки на рабочий стол.
# Запускать после сборки: cargo build --release -p pult-app -p pult-cli ; pwsh -File tools/install.ps1
# Это способ для разработчика. Обычная установка - установщик: pwsh -File tools/build-installer.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$bin = Join-Path $root 'bin'
$rel = Join-Path $root 'target\release'
foreach ($exe in 'Pult.exe', 'pult-cli.exe') {
  $src = Join-Path $rel $exe
  if (-not (Test-Path $src)) { throw "Нет $src - сначала соберите: cargo build --release -p pult-app -p pult-cli" }
}
New-Item -ItemType Directory -Force $bin | Out-Null
# окно может быть открыто - тогда exe занят; закрываем только наш Pult.exe из bin
Get-Process Pult -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($bin, [StringComparison]::OrdinalIgnoreCase) } | Stop-Process -Force
Start-Sleep -Milliseconds 300
Copy-Item (Join-Path $rel 'Pult.exe') (Join-Path $bin 'Pult.exe') -Force
Copy-Item (Join-Path $rel 'pult-cli.exe') (Join-Path $bin 'pult-cli.exe') -Force
# pult.exe и Pult.exe в одной папке на Windows - одно имя, поэтому консольная версия называется pult-cli.exe,
# а короткую команду pult даёт pult.cmd рядом (один файл на установщик, архив и bin\)
Copy-Item (Join-Path $root 'app\installer\pult.cmd') (Join-Path $bin 'pult.cmd') -Force

$icon = Join-Path $root 'app\icons\icon.ico'
$desktop = [Environment]::GetFolderPath('Desktop')
$sh = New-Object -ComObject WScript.Shell

# ярлык прошлой версии назывался с длинным тире - убираем, чтобы на столе не было двух консолей
$old = Join-Path $desktop ('Пульт ' + [char]0x2014 + ' консоль.lnk')
if (Test-Path -LiteralPath $old) { Remove-Item -LiteralPath $old -Force }

$s = $sh.CreateShortcut((Join-Path $desktop 'Пульт.lnk'))
$s.TargetPath = Join-Path $bin 'Pult.exe'
$s.Arguments = ''
$s.WorkingDirectory = $bin
$s.WindowStyle = 1
$s.IconLocation = $icon
$s.Description = 'Пульт - управление любым проектом: скрипты с объяснениями, уроки, CI'
$s.Save()

$c = $sh.CreateShortcut((Join-Path $desktop 'Пульт - консоль.lnk'))
$cli = Join-Path $bin 'pult-cli.exe'
$wt = Get-Command wt.exe -ErrorAction SilentlyContinue
if ($wt) {
  $c.TargetPath = $wt.Source
  $c.Arguments = '--title "Пульт" "' + $cli + '"'
} else {
  $c.TargetPath = $cli
  $c.Arguments = ''
}
$c.WorkingDirectory = $bin
$c.IconLocation = $icon
$c.Description = 'Пульт в терминале: скрипты, объяснения, уроки и CI без окна'
$c.Save()
Write-Output "ok: $bin"
