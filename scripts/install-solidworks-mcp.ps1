<#
.SYNOPSIS
  Anything-to-Prompt — Zegel-SWE SolidWorks MCP Ecosystem Installer (MIT).
  Connects Anything-to-Prompt's SolidWorks Feature Pick ([SW-TARGET] blue capsules)
  with Zegel-SWE (q862877400-ux/zegel-solidworks-engineering-delivery — 544 SolidWorks MCP tools).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\install-solidworks-mcp.ps1 -ConfigureDsh
  # Clones or reuses local Zegel-SWE repo and registers mcp-solidworks into DSH.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\install-solidworks-mcp.ps1 -RepoPath "D:\Agent-Studio\DSH-Workspace\Solidworks" -ConfigureDsh
#>
[CmdletBinding()]
param(
  [string]$RepoPath = '',
  [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'AnythingToPrompt\mcp-deps\zegel-swe'),
  [switch]$ConfigureDsh,
  [switch]$ConfigureExternalClients
)

$ErrorActionPreference = 'Stop'

function Resolve-ZegelSweRepo {
  if ($RepoPath -and (Test-Path (Join-Path $RepoPath 'mcp-server\server.py'))) {
    return (Resolve-Path $RepoPath).Path
  }

  $candidates = @(
    $InstallDir,
    'D:\Agent-Studio\DSH-Workspace\Solidworks'
  )
  foreach ($c in $candidates) {
    if ($c -and (Test-Path (Join-Path $c 'mcp-server\server.py'))) {
      return (Resolve-Path $c).Path
    }
  }

  if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw '未找到本地 Zegel-SWE 仓库且系统未安装 git，请传入 -RepoPath 指定本地仓库路径。'
  }
  $parent = Split-Path $InstallDir -Parent
  if (-not (Test-Path $parent)) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
  Write-Host "[Anything-to-Prompt] Cloning q862877400-ux/zegel-solidworks-engineering-delivery -> $InstallDir" -ForegroundColor Cyan
  git clone --depth 1 https://github.com/q862877400-ux/zegel-solidworks-engineering-delivery.git $InstallDir
  if ($LASTEXITCODE -ne 0) { throw 'git clone failed.' }
  return (Resolve-Path $InstallDir).Path
}

$swRepo = Resolve-ZegelSweRepo
$serverPy = Join-Path $swRepo 'mcp-server\server.py'
if (-not (Test-Path $serverPy)) {
  throw "未找到 SolidWorks MCP Server：$serverPy"
}
$pythonCmd = (Get-Command python -ErrorAction SilentlyContinue).Source
if (-not $pythonCmd) { $pythonCmd = 'python' }

Write-Host "[Anything-to-Prompt] Zegel-SWE Repo : $swRepo" -ForegroundColor Green
Write-Host "[Anything-to-Prompt] MCP Server     : $serverPy" -ForegroundColor Green

if ($ConfigureDsh) {
  $dshHome = if ($env:DSH_HOME) { $env:DSH_HOME } else { Join-Path $env:USERPROFILE '.dsh' }
  $patchPath = Join-Path $dshHome 'profiles\web\cordis.patch.yml'
  if (-not (Test-Path $patchPath)) {
    Write-Warning "未找到 DSH cordis.patch.yml ($patchPath)，跳过 DSH 注册。"
  } else {
    $raw = Get-Content $patchPath -Raw -Encoding UTF8
    if ($raw -match 'id:\s*mcp-solidworks') {
      Write-Host "[Anything-to-Prompt] DSH cordis.patch.yml 已包含 mcp-solidworks，无需重复注册。" -ForegroundColor Yellow
    } else {
      $escapedPy = $serverPy.Replace("'", "''")
      $escapedCwd = $swRepo.Replace("'", "''")
      $block = @"

# Anything-to-Prompt <-> Zegel-SWE SolidWorks MCP (544 tools, MIT)
- insert:
    - id: mcp-solidworks
      name: "@deepseek-ai/dsh-mcp-client"
      config:
        transport: stdio
        serverName: solidworks
        command: python
        args:
          - '$escapedPy'
        cwd: '$escapedCwd'
        toolCallTimeoutMs: 180000
        failOnStartupError: false
"@
      Add-Content -Path $patchPath -Value $block -Encoding UTF8
      Write-Host "[Anything-to-Prompt] 已注册 mcp-solidworks 到 $patchPath（下次重启 DSH 后生效）。" -ForegroundColor Green
    }
  }
}

if ($ConfigureExternalClients) {
  $regScript = Join-Path $swRepo 'mcp-server\register_all_ai_mcp.ps1'
  if (Test-Path $regScript) {
    Write-Host "[Anything-to-Prompt] 调用 Zegel-SWE 多客户端注册脚本 ($regScript)..." -ForegroundColor Cyan
    powershell -ExecutionPolicy Bypass -File $regScript
  }
}

Write-Host "[Anything-to-Prompt] SolidWorks 生态接入完成。在 SolidWorks 中按住 Ctrl 点击特征树或 3D 实体即可抓入蓝色胶囊。" -ForegroundColor Cyan
