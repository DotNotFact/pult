# Вырезает из CHANGELOG.md секцию версии для заметок релиза.
# Узнаёт заголовки "## [2.1.0]", "## 2.1.0", "## v2.1.0 - 2026-10-14"; секция идёт до следующего "## ".
# Печатает true и пишет секцию в -Out, если она есть и не пустая; иначе печатает false.
#
#   pwsh -File tools/release-notes.ps1 -Version 2.1.0 -Out notes.md
[CmdletBinding()]
param(
  [Parameter(Mandatory)][string]$Version,
  [Parameter(Mandatory)][string]$Out,
  [string]$Changelog
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3

if (-not $Changelog) { $Changelog = Join-Path (Split-Path $PSScriptRoot -Parent) 'CHANGELOG.md' }
$Version = $Version -replace '^v', ''
if (-not (Test-Path $Changelog)) { Write-Output 'false'; return }

$head = '^##\s+\[?v?' + [regex]::Escape($Version) + '\]?(\s|$)'
$body = [Collections.Generic.List[string]]::new()
$inside = $false
foreach ($line in [IO.File]::ReadAllLines($Changelog, [Text.Encoding]::UTF8)) {
  if ($line -match '^##\s') {
    if ($inside) { break }
    if ($line -match $head) { $inside = $true }
    continue
  }
  if ($inside) { $body.Add($line) }
}

$text = ($body -join "`n").Trim()
if (-not $text) { Write-Output 'false'; return }
[IO.File]::WriteAllText($Out, "$text`n", [Text.UTF8Encoding]::new($false))
Write-Output 'true'
