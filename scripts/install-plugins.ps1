<#
.SYNOPSIS
  Install Anything-to-Prompt as a shared companion dependency binary and register plugins for
  DeepSeek Harness (DSH), Antigravity, Claude Code, and Codex, then launch quietly.
#>
param(
  [string]$SourceExe = ""
)

$ErrorActionPreference = 'Stop'

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir

if (-not $SourceExe) {
  $candidates = @(
    (Join-Path $repoRoot 'release\anything-to-prompt.exe'),
    (Join-Path $repoRoot 'apps\desktop\src-tauri\target\release\anything-to-prompt.exe')
  )
  foreach ($c in $candidates) {
    if (Test-Path $c) { $SourceExe = $c; break }
  }
}

if (-not $SourceExe -or -not (Test-Path $SourceExe)) {
  throw "anything-to-prompt.exe not found. Pass -SourceExe <path> or build first."
}

$binDir = Join-Path $env:LOCALAPPDATA 'AnythingToPrompt\bin'
New-Item -ItemType Directory -Force -Path $binDir | Out-Null
$destExe = Join-Path $binDir 'anything-to-prompt.exe'

Get-Process anything-to-prompt -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300
Copy-Item -Force $SourceExe $destExe

Write-Host "[Anything-to-Prompt] Installed runtime dependency -> $destExe"
Write-Host "[Anything-to-Prompt] Registering plugins for DSH / Antigravity / Claude Code / Codex..."
Start-Process -FilePath $destExe -ArgumentList @('--install-plugins') -Wait -NoNewWindow

Write-Host "[Anything-to-Prompt] Starting background companion (watch cursor breath pulse)..."
Start-Process -FilePath $destExe -ArgumentList @('--ensure-running', '--launched-by', 'installer')
