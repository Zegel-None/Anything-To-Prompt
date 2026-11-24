<#
.SYNOPSIS
  Anything-to-Prompt — Official Office / WPS Live-Edit MCP Ecosystem Installer (All MIT).
  Installs mature open-source MCP servers so AI agents can edit ALREADY-OPEN documents in place
  without closing them or overwriting locked files on disk.

.DESCRIPTION
  Supported Suites:
    - WPS Office (latest / WPS 12):
        * wps-office -> lc2panda/wps-skills (MIT, 243 tools for WPS Writer/Spreadsheets/Presentation)
    - Microsoft Office 2016 ~ 2026 (2016/2019/2021/2024/365/2026):
        * excel-live -> @negokaz/excel-mcp-server (MIT, native Go COM live editing + screenshots)
        * word-live  -> ykarapazar/word-mcp-live (MIT, 114 tools, Track Changes + per-action Ctrl+Z)
        * ppt-live   -> ykuwai/ppt-mcp (MIT, 157 tools, real-time PowerPoint COM control)

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\install-office-mcp.ps1
  # Auto-detects installed WPS and/or MS Office 2016-2026 and installs matching MCP servers.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\install-office-mcp.ps1 -Suite wps -ConfigureDsh
  # Installs lc2panda/wps-skills and registers mcp-wps-office into ~/.dsh/profiles/web/cordis.patch.yml.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\install-office-mcp.ps1 -Suite all -ConfigureDsh
  # Installs both WPS + MS Office 2016-2026 MCP ecosystems.
#>
[CmdletBinding()]
param(
  [ValidateSet('auto', 'wps', 'msoffice', 'all')]
  [string]$Suite = 'auto',

  [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'AnythingToPrompt\mcp-deps'),

  [switch]$ConfigureDsh,
  [switch]$SkipSkills
)

$ErrorActionPreference = 'Stop'

function Test-CommandExists([string]$Name) {
  return $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

function Detect-InstalledSuites {
  $hasWps = $false
  $hasMso = $false

  # Detect WPS Office
  if (Get-Process wps,et,wpp -ErrorAction SilentlyContinue) { $hasWps = $true }
  if (-not $hasWps) {
    foreach ($prog in @('KWPS.Application', 'KET.Application', 'KWPP.Application')) {
      if (Test-Path "Registry::HKEY_CLASSES_ROOT\$prog") { $hasWps = $true; break }
    }
  }

  # Detect Microsoft Office 2016 ~ 2026 (App Paths for WINWORD/EXCEL/POWERPNT)
  foreach ($exe in @('WINWORD.EXE', 'excel.exe', 'powerpnt.exe')) {
    foreach ($root in @('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths', 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths')) {
      if (Test-Path "$root\$exe") { $hasMso = $true; break }
    }
    if ($hasMso) { break }
  }

  return [pscustomobject]@{ Wps = $hasWps; MsOffice = $hasMso }
}

$wantWps = $false
$wantMso = $false
if ($Suite -eq 'auto') {
  $det = Detect-InstalledSuites
  $wantWps = $det.Wps
  $wantMso = $det.MsOffice
  Write-Host "[Auto-Detect] WPS Office=$wantWps | MS Office 2016-2026=$wantMso" -ForegroundColor Cyan
  if (-not $wantWps -and -not $wantMso) {
    Write-Host "No Office/WPS installation auto-detected. Use -Suite wps, -Suite msoffice, or -Suite all." -ForegroundColor Yellow
    exit 0
  }
} elseif ($Suite -eq 'wps') {
  $wantWps = $true
} elseif ($Suite -eq 'msoffice') {
  $wantMso = $true
} elseif ($Suite -eq 'all') {
  $wantWps = $true
  $wantMso = $true
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$installedServers = New-Object System.Collections.Generic.List[object]

# ==============================================================================
# 1. WPS Office (Latest / WPS 12) -> lc2panda/wps-skills (MIT)
# ==============================================================================
if ($wantWps) {
  Write-Host "`n=== Installing WPS Live MCP: lc2panda/wps-skills (MIT) ===" -ForegroundColor Green
  if (-not (Test-CommandExists 'git')) { throw "git is required to install lc2panda/wps-skills." }
  if (-not (Test-CommandExists 'node') -or -not (Test-CommandExists 'npm')) { throw "Node.js >= 18 & npm are required to build wps-skills." }

  $wpsRepoDir = Join-Path $InstallDir 'wps-skills'
  if (Test-Path (Join-Path $wpsRepoDir '.git')) {
    Write-Host "Updating existing wps-skills repo at $wpsRepoDir..."
    git -C $wpsRepoDir pull --ff-only
  } else {
    Write-Host "Cloning https://github.com/lc2panda/wps-skills.git -> $wpsRepoDir..."
    git clone --depth 1 https://github.com/lc2panda/wps-skills.git $wpsRepoDir
  }

  $mcpDir = Join-Path $wpsRepoDir 'wps-office-mcp'
  Push-Location $mcpDir
  try {
    npm install
    if ($LASTEXITCODE -ne 0) { throw "npm install failed in $mcpDir" }
    npm run build
    if ($LASTEXITCODE -ne 0) { throw "npm run build failed in $mcpDir" }
  } finally {
    Pop-Location
  }

  $entryJs = Join-Path $mcpDir 'dist\index.js'
  if (-not (Test-Path $entryJs)) { throw "Expected built MCP entry not found: $entryJs" }
  $nodeExe = (Get-Command node).Source

  $installedServers.Add([pscustomobject]@{
    Id         = 'mcp-wps-office'
    ServerName = 'wps-office'
    Command    = $nodeExe
    Args       = @($entryJs)
    Cwd        = $mcpDir
    Repo       = 'lc2panda/wps-skills (MIT)'
  })

  # Sync bundled WPS Skills into ~/.dsh/skills when present
  if (-not $SkipSkills) {
    $dshSkillsDir = Join-Path $env:USERPROFILE '.dsh\skills'
    $srcSkillsDir = Join-Path $wpsRepoDir 'skills'
    if ((Test-Path $dshSkillsDir) -and (Test-Path $srcSkillsDir)) {
      foreach ($sk in @('wps-office', 'wps-word', 'wps-excel', 'wps-ppt')) {
        $src = Join-Path $srcSkillsDir $sk
        $dst = Join-Path $dshSkillsDir $sk
        if (Test-Path $src) {
          if (Test-Path $dst) { Remove-Item -Recurse -Force $dst }
          Copy-Item -Recurse -Force $src $dst
          Write-Host "  [Skill] Synced $sk -> $dst"
        }
      }
    }
  }
  Write-Host "[OK] wps-office MCP ready: $entryJs" -ForegroundColor Green
}

# ==============================================================================
# 2. MS Office 2016 ~ 2026 -> MIT Live COM Golden Trio
# ==============================================================================
if ($wantMso) {
  Write-Host "`n=== Installing MS Office 2016-2026 Live MCP Trio (All MIT) ===" -ForegroundColor Green

  # 2a. Excel: @negokaz/excel-mcp-server (1051 stars, MIT, native Go COM)
  if (-not (Test-CommandExists 'npm')) { throw "npm is required to install @negokaz/excel-mcp-server." }
  $excelDir = Join-Path $InstallDir 'excel-mcp-server'
  New-Item -ItemType Directory -Force -Path $excelDir | Out-Null
  if (-not (Test-Path (Join-Path $excelDir 'package.json'))) {
    '{"name":"atp-excel-mcp","private":true}' | Set-Content -Encoding UTF8 (Join-Path $excelDir 'package.json')
  }
  npm --prefix $excelDir install --no-audit --no-fund @negokaz/excel-mcp-server
  if ($LASTEXITCODE -ne 0) { throw "Failed to install @negokaz/excel-mcp-server" }
  $excelBin = Join-Path $excelDir 'node_modules\.bin\excel-mcp-server.cmd'
  $installedServers.Add([pscustomobject]@{
    Id         = 'mcp-excel-live'
    ServerName = 'excel-live'
    Command    = 'cmd.exe'
    Args       = @('/c', $excelBin)
    Cwd        = $excelDir
    Repo       = 'negokaz/excel-mcp-server (MIT)'
  })
  Write-Host "[OK] excel-live MCP ready: $excelBin" -ForegroundColor Green

  # 2b. Word: ykarapazar/word-mcp-live (280 stars, MIT, Python pywin32)
  $wordDir = Join-Path $InstallDir 'word-mcp-live'
  if (Test-CommandExists 'uv') {
    uv venv (Join-Path $wordDir '.venv') --allow-existing
    uv pip install --python (Join-Path $wordDir '.venv\Scripts\python.exe') word-mcp-live
    $wordExe = Join-Path $wordDir '.venv\Scripts\word-mcp-live.exe'
    if (-not (Test-Path $wordExe)) {
      $wordExe = Join-Path $wordDir '.venv\Scripts\python.exe'
      $wordArgs = @('-m', 'office_word_mcp_server')
    } else {
      $wordArgs = @()
    }
    $installedServers.Add([pscustomobject]@{
      Id         = 'mcp-word-live'
      ServerName = 'word-live'
      Command    = $wordExe
      Args       = $wordArgs
      Cwd        = $wordDir
      Repo       = 'ykarapazar/word-mcp-live (MIT)'
    })
    Write-Host "[OK] word-live MCP ready: $wordExe" -ForegroundColor Green
  } else {
    Write-Host "[WARN] uv not found; skipping word-mcp-live (install via: pip install word-mcp-live)" -ForegroundColor Yellow
  }

  # 2c. PowerPoint: ykuwai/ppt-mcp (134 stars, MIT, Python pywin32)
  if ((Test-CommandExists 'git') -and (Test-CommandExists 'uv')) {
    $pptRepoDir = Join-Path $InstallDir 'ppt-mcp'
    if (Test-Path (Join-Path $pptRepoDir '.git')) {
      git -C $pptRepoDir pull --ff-only
    } else {
      git clone --depth 1 https://github.com/ykuwai/ppt-mcp.git $pptRepoDir
    }
    uv venv (Join-Path $pptRepoDir '.venv') --allow-existing
    uv pip install --python (Join-Path $pptRepoDir '.venv\Scripts\python.exe') -e $pptRepoDir
    $pptPy = Join-Path $pptRepoDir '.venv\Scripts\python.exe'
    $installedServers.Add([pscustomobject]@{
      Id         = 'mcp-ppt-live'
      ServerName = 'ppt-live'
      Command    = $pptPy
      Args       = @('-m', 'src.server')
      Cwd        = $pptRepoDir
      Repo       = 'ykuwai/ppt-mcp (MIT)'
    })
    Write-Host "[OK] ppt-live MCP ready: $pptRepoDir" -ForegroundColor Green
  } else {
    Write-Host "[WARN] git or uv not found; skipping ykuwai/ppt-mcp" -ForegroundColor Yellow
  }
}

# ==============================================================================
# 3. Optional: Register into DSH (~/.dsh/profiles/web/cordis.patch.yml)
# ==============================================================================
if ($ConfigureDsh -and $installedServers.Count -gt 0) {
  $patchPath = Join-Path $env:USERPROFILE '.dsh\profiles\web\cordis.patch.yml'
  if (-not (Test-Path $patchPath)) {
    Write-Host "[WARN] DSH config not found at $patchPath; skipping auto-registration." -ForegroundColor Yellow
  } else {
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $backupPath = "$patchPath.bak-$stamp-before-office-mcp"
    Copy-Item -Force $patchPath $backupPath
    Write-Host "`nBacked up DSH config -> $backupPath"

    $yaml = Get-Content -Raw -Encoding UTF8 $patchPath
    $addedAny = $false
    foreach ($srv in $installedServers) {
      if ($yaml -match "(?m)^\s*-\s*id:\s*$([regex]::Escape($srv.Id))\s*$") {
        Write-Host "  [DSH] $($srv.Id) already present in cordis.patch.yml (kept existing)."
        continue
      }
      $argsYaml = ''
      if ($srv.Args.Count -gt 0) {
        $argsLines = $srv.Args | ForEach-Object { "          - '$($_.Replace("'", "''"))'" }
        $argsYaml = "        args:`n" + ($argsLines -join "`n") + "`n"
      } else {
        $argsYaml = "        args: []`n"
      }
      $block = @"

# Anything-to-Prompt Office/WPS Live MCP ($($srv.Repo))
- insert:
    - id: $($srv.Id)
      name: "@deepseek-ai/dsh-mcp-client"
      config:
        transport: stdio
        serverName: $($srv.ServerName)
        command: '$($srv.Command.Replace("'", "''"))'
$argsYaml        cwd: '$($srv.Cwd.Replace("'", "''"))'
        toolCallTimeoutMs: 120000
        failOnStartupError: false
"@
      $yaml = $yaml.TrimEnd() + "`n" + $block
      $addedAny = $true
      Write-Host "  [DSH] Registered $($srv.Id) (serverName=$($srv.ServerName))" -ForegroundColor Green
    }

    if ($addedAny) {
      [System.IO.File]::WriteAllText($patchPath, $yaml, [System.Text.UTF8Encoding]::new($false))
      Write-Host "[OK] Updated $patchPath (restart DSH when convenient to load the new MCP tools)." -ForegroundColor Cyan
    }
  }
}

# ==============================================================================
# 4. Print Standard MCP Config Snippet (Claude Code / Cursor / Codex / Generic)
# ==============================================================================
if ($installedServers.Count -gt 0) {
  Write-Host "`n=== Standard mcpServers JSON (Claude Code / Cursor / Generic MCP Clients) ===" -ForegroundColor Cyan
  $mcpMap = @{}
  foreach ($srv in $installedServers) {
    $mcpMap[$srv.ServerName] = @{
      command = $srv.Command
      args    = $srv.Args
      cwd     = $srv.Cwd
    }
  }
  @{ mcpServers = $mcpMap } | ConvertTo-Json -Depth 5
}
