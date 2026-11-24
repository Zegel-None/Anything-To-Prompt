param(
    [string]$Profile = "web"
)

$ErrorActionPreference = "Stop"

$pluginDir = (Resolve-Path (Join-Path $PSScriptRoot "..\packages\anything-to-prompt")).Path
$dshHome = if ($env:DSH_HOME -and $env:DSH_HOME.Trim()) {
    $env:DSH_HOME.Trim()
} else {
    Join-Path $HOME ".dsh"
}

$profileDir = Join-Path $dshHome "profiles\$Profile"
$pkgJsonPath = Join-Path $profileDir "package.json"

Write-Host "== Anything-to-Prompt — @local/anything-to-prompt Plugin Installer ==" -ForegroundColor Cyan
Write-Host "Plugin Source : $pluginDir"
Write-Host "DSH Profile   : $profileDir"

& node --check (Join-Path $pluginDir "lib\index.js")
if ($LASTEXITCODE -ne 0) { throw "Syntax check failed on lib/index.js" }
& node --check (Join-Path $pluginDir "lib\client.js")
if ($LASTEXITCODE -ne 0) { throw "Syntax check failed on lib/client.js" }

if (-not (Test-Path $pkgJsonPath)) {
    throw "DSH profile package.json not found at $pkgJsonPath. Please initialize or run DeepSeek Harness once first."
}

$localDest = Join-Path $profileDir "local-plugins\anything-to-prompt"
if (Test-Path $localDest) {
    Remove-Item -Path $localDest -Recurse -Force
}
New-Item -ItemType Directory -Force -Path (Split-Path $localDest -Parent) | Out-Null
Copy-Item -Path $pluginDir -Destination $localDest -Recurse -Force

$scopeDir = Join-Path $profileDir "node_modules\@local"
New-Item -ItemType Directory -Force -Path $scopeDir | Out-Null

# Remove legacy @local/dsh-card-dock and @local/deepseek-bar links if present
foreach ($legacy in @("dsh-card-dock", "deepseek-bar")) {
    $legacyPath = Join-Path $scopeDir $legacy
    if (Test-Path $legacyPath) {
        cmd /c rmdir "$legacyPath" 2>$null
        if (Test-Path $legacyPath) { Remove-Item -Path $legacyPath -Recurse -Force }
    }
}

$junctionPath = Join-Path $scopeDir "anything-to-prompt"
if (Test-Path $junctionPath) {
    cmd /c rmdir "$junctionPath" 2>$null
    if (Test-Path $junctionPath) { Remove-Item -Path $junctionPath -Recurse -Force }
}
New-Item -ItemType Junction -Path $junctionPath -Target $localDest | Out-Null
Write-Host "  Linked Junction: $junctionPath -> $localDest" -ForegroundColor Green

$nodeScript = @'
const fs = require('fs');
const pkgPath = process.argv[1];
const raw = fs.readFileSync(pkgPath, 'utf8').replace(/^\uFEFF/, '');
const pkg = JSON.parse(raw);
pkg.dependencies = pkg.dependencies || {};
delete pkg.dependencies['@local/dsh-card-dock'];
delete pkg.dependencies['@local/deepseek-bar'];
pkg.dependencies['@local/anything-to-prompt'] = 'link:local-plugins\\anything-to-prompt';
pkg.dsh = pkg.dsh || {};
pkg.dsh.profile = pkg.dsh.profile || {};
const bundles = Array.isArray(pkg.dsh.profile.bundles) ? pkg.dsh.profile.bundles : [];
const cleaned = bundles.filter((b) => b !== '@local/dsh-card-dock' && b !== '@local/deepseek-bar');
if (!cleaned.includes('@local/anything-to-prompt')) {
  cleaned.push('@local/anything-to-prompt');
}
pkg.dsh.profile.bundles = cleaned;
fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + '\n', 'utf8');
'@
& node -e $nodeScript -- $pkgJsonPath
if ($LASTEXITCODE -ne 0) { throw "Failed to update $pkgJsonPath" }
Write-Host "  Registered '@local/anything-to-prompt' in $pkgJsonPath" -ForegroundColor Green
Write-Host "`nDone! Restart DSH if it was already running so @local/anything-to-prompt loads cleanly." -ForegroundColor Cyan
