<#
.SYNOPSIS
  Anything-to-Prompt — Zegel-SW v2 SolidWorks MCP Ecosystem Installer (MIT).
  Connects Anything-to-Prompt's universal `[SW-TARGET v2]` blue capsules with Zegel-SW v2
  (or legacy Zegel-SWE fallback), verifying a `pywin32`-capable Python interpreter and
  coexisting safely with other MCP servers (such as `weknora`) in DSH `cordis.patch.yml`.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\install-solidworks-mcp.ps1 -RepoPath "D:\Agent-Studio\zegel-sw-v2" -ConfigureDsh

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\install-solidworks-mcp.ps1 -RepoPath "D:\Agent-Studio\zegel-sw-v2" -PythonPath "D:\Agent-Studio\zegel-sw-v2\.venv\Scripts\python.exe" -ConfigureDsh -ForceUpdate
#>
[CmdletBinding()]
param(
  [string]$RepoPath = '',
  [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'AnythingToPrompt\mcp-deps\zegel-sw-v2'),
  [string]$PythonPath = '',
  [string]$DshPatchPath = '',
  [switch]$ConfigureDsh,
  [switch]$ForceUpdate,
  [switch]$ConfigureExternalClients
)

$ErrorActionPreference = 'Stop'

function Resolve-SolidWorksMcpRepo {
  param([string]$ExplicitRepo, [string]$DefaultInstall)

  if ($ExplicitRepo -and (Test-Path (Join-Path $ExplicitRepo 'mcp-server\server.py'))) {
    return (Resolve-Path $ExplicitRepo).Path
  }

  $candidates = @(
    $DefaultInstall,
    (Join-Path $env:LOCALAPPDATA 'AnythingToPrompt\mcp-deps\zegel-swe'),
    'D:\Agent-Studio\zegel-sw-v2',
    'D:\Agent-Studio\Antigravity-Workspace\zegel-sw-v2',
    'D:\Agent-Studio\DSH-Workspace\zegel-sw-v2',
    'D:\Agent-Studio\DSH-Workspace\Solidworks'
  )
  foreach ($c in $candidates) {
    if ($c -and (Test-Path (Join-Path $c 'mcp-server\server.py'))) {
      return (Resolve-Path $c).Path
    }
  }

  if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw '未找到本地 Zegel-SW v2 / Zegel-SWE 仓库且系统未安装 git，请传入 -RepoPath 指定本地仓库路径。'
  }
  $parent = Split-Path $DefaultInstall -Parent
  if (-not (Test-Path $parent)) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
  Write-Host "[Anything-to-Prompt] Cloning SolidWorks MCP repo -> $DefaultInstall" -ForegroundColor Cyan
  git clone --depth 1 https://github.com/q862877400-ux/zegel-solidworks-engineering-delivery.git $DefaultInstall
  if ($LASTEXITCODE -ne 0) { throw 'git clone failed.' }
  return (Resolve-Path $DefaultInstall).Path
}

function Test-PythonHasPyWin32 {
  param([string]$ExePath)
  if (-not $ExePath -or -not (Test-Path $ExePath)) { return $false }
  try {
    $null = & $ExePath -c "import win32com.client" 2>$null
    return ($LASTEXITCODE -eq 0)
  } catch {
    return $false
  }
}

function Resolve-VerifiedPythonPath {
  param([string]$ExplicitPy, [string]$RepoRoot)

  $candidates = New-Object System.Collections.Generic.List[string]
  if ($ExplicitPy) { $candidates.Add($ExplicitPy) }

  # 1. Prefer repo-local virtualenvs first
  foreach ($rel in @('.venv\Scripts\python.exe', 'venv\Scripts\python.exe', 'mcp-server\.venv\Scripts\python.exe')) {
    $p = Join-Path $RepoRoot $rel
    if (Test-Path $p) { $candidates.Add((Resolve-Path $p).Path) }
  }

  # 2. Active VIRTUAL_ENV / CONDA_PREFIX
  if ($env:VIRTUAL_ENV) {
    $p = Join-Path $env:VIRTUAL_ENV 'Scripts\python.exe'
    if (Test-Path $p) { $candidates.Add((Resolve-Path $p).Path) }
  }
  if ($env:CONDA_PREFIX) {
    $p = Join-Path $env:CONDA_PREFIX 'python.exe'
    if (Test-Path $p) { $candidates.Add((Resolve-Path $p).Path) }
  }

  # 3. Windows Python Launcher (`py -3`) and PATH `python` entries
  if (Get-Command py -ErrorAction SilentlyContinue) {
    try {
      $pyExe = (& py -3 -c "import sys; print(sys.executable)" 2>$null | Select-Object -First 1)
      if ($pyExe -and (Test-Path $pyExe.Trim())) { $candidates.Add($pyExe.Trim()) }
    } catch {}
  }
  foreach ($cmd in (Get-Command python -All -ErrorAction SilentlyContinue)) {
    if ($cmd.Source -and (Test-Path $cmd.Source)) { $candidates.Add($cmd.Source) }
  }

  $seen = New-Object System.Collections.Generic.HashSet[string]([System.StringComparer]::OrdinalIgnoreCase)
  foreach ($cand in $candidates) {
    if (-not $seen.Add($cand)) { continue }
    if (Test-PythonHasPyWin32 -ExePath $cand) {
      return (Resolve-Path $cand).Path
    }
  }

  throw "未找到安装了 pywin32 的 Python 解释器。请先在目标环境中运行 `pip install pywin32` 或通过 `-PythonPath` 显式传入完整 python.exe 路径。"
}

function Upsert-DshSolidWorksPatch {
  param(
    [string]$PatchFile,
    [string]$PythonExe,
    [string]$ServerScript,
    [string]$RepoRoot,
    [bool]$ReplaceExisting
  )

  $escapedPyExe = $PythonExe.Replace("'", "''")
  $escapedServer = $ServerScript.Replace("'", "''")
  $escapedCwd = $RepoRoot.Replace("'", "''")
  $block = @"
# Anything-to-Prompt <-> Zegel-SW v2 SolidWorks MCP (sw-target/v2, coexists with weknora & other MCPs)
- insert:
    - id: mcp-solidworks
      name: "@deepseek-ai/dsh-mcp-client"
      config:
        transport: stdio
        serverName: solidworks
        command: '$escapedPyExe'
        args:
          - '$escapedServer'
        cwd: '$escapedCwd'
        toolCallTimeoutMs: 180000
        failOnStartupError: false
"@

  $raw = Get-Content $PatchFile -Raw -Encoding UTF8
  if ($null -eq $raw) { $raw = '' }

  if ($raw -match '(?m)^\s*-\s*id:\s*mcp-solidworks\b') {
    if (-not $ReplaceExisting) {
      Write-Host "[Anything-to-Prompt] DSH cordis.patch.yml 已包含 mcp-solidworks（保留现有及 weknora 等其他 MCP 配置）。如需更新解释器/路径请加 -ForceUpdate。" -ForegroundColor Yellow
      return
    }
    # Surgically replace ONLY the `- insert:` section containing `id: mcp-solidworks`, preserving sibling blocks (e.g., weknora).
    $pattern = '(?ms)(?:^#\s*Anything-to-Prompt[^\r\n]*\r?\n)?^-\s*insert:\s*\r?\n\s*-\s*id:\s*mcp-solidworks\b.*?(?=^(?:-\s*insert:|#\s*[^\r\n]+\r?\n-\s*insert:)|\z)'
    $updated = [System.Text.RegularExpressions.Regex]::Replace($raw, $pattern, ($block.TrimEnd() + "`r`n`r`n"))
    Set-Content -Path $PatchFile -Value $updated -Encoding UTF8 -NoNewline
    Write-Host "[Anything-to-Prompt] 已定点更新 $PatchFile 中的 mcp-solidworks 块（其他 MCP 块完整保留）。" -ForegroundColor Green
  } else {
    $prefix = if ($raw.Length -gt 0 -and -not $raw.EndsWith("`n")) { "`r`n`r`n" } elseif ($raw.Length -gt 0) { "`r`n" } else { '' }
    Set-Content -Path $PatchFile -Value ($raw + $prefix + $block.TrimEnd() + "`r`n") -Encoding UTF8 -NoNewline
    Write-Host "[Anything-to-Prompt] 已追加注册 mcp-solidworks 到 $PatchFile（保留 weknora 等已有 MCP，下次重启 DSH 生效）。" -ForegroundColor Green
  }
}

$swRepo = Resolve-SolidWorksMcpRepo -ExplicitRepo $RepoPath -DefaultInstall $InstallDir
$serverPy = Join-Path $swRepo 'mcp-server\server.py'
if (-not (Test-Path $serverPy)) {
  throw "未找到 SolidWorks MCP Server：$serverPy"
}
$verifiedPython = Resolve-VerifiedPythonPath -ExplicitPy $PythonPath -RepoRoot $swRepo

Write-Host "[Anything-to-Prompt] SolidWorks Repo : $swRepo" -ForegroundColor Green
Write-Host "[Anything-to-Prompt] MCP Server      : $serverPy" -ForegroundColor Green
Write-Host "[Anything-to-Prompt] Verified Python : $verifiedPython (pywin32 OK)" -ForegroundColor Green

if ($ConfigureDsh) {
  $patchPath = if ($DshPatchPath) {
    $DshPatchPath
  } else {
    $dshHome = if ($env:DSH_HOME) { $env:DSH_HOME } else { Join-Path $env:USERPROFILE '.dsh' }
    Join-Path $dshHome 'profiles\web\cordis.patch.yml'
  }
  if (-not (Test-Path $patchPath)) {
    Write-Warning "未找到 DSH cordis.patch.yml ($patchPath)，跳过 DSH 注册。"
  } else {
    Upsert-DshSolidWorksPatch `
      -PatchFile $patchPath `
      -PythonExe $verifiedPython `
      -ServerScript $serverPy `
      -RepoRoot $swRepo `
      -ReplaceExisting ([bool]$ForceUpdate)
  }
}

if ($ConfigureExternalClients) {
  $regScript = Join-Path $swRepo 'mcp-server\register_all_ai_mcp.ps1'
  if (Test-Path $regScript) {
    Write-Host "[Anything-to-Prompt] 调用多客户端注册脚本 ($regScript)..." -ForegroundColor Cyan
    powershell -ExecutionPolicy Bypass -File $regScript
  }
}

Write-Host "[Anything-to-Prompt] SolidWorks sw-target/v2 生态接入完成。在 SolidWorks 中按住 Ctrl 点击即可毫秒级抓入 [SW-TARGET v2] 蓝色胶囊。" -ForegroundColor Cyan
