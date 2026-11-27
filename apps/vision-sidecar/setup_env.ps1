<#
.SYNOPSIS
  Installs the isolated Anything-to-Prompt Vision Sidecar Python 3.12 environment
  and downloads the selected hardware tier weights into %LOCALAPPDATA%\AnythingToPrompt\.
.PARAMETER Tier
  lite | balanced | pro | all
#>
param(
    [ValidateSet("lite", "balanced", "pro", "all")]
    [string]$Tier = "lite"
)

$ErrorActionPreference = "Stop"
$BaseDir = Join-Path $env:LOCALAPPDATA "AnythingToPrompt"
$VenvDir = Join-Path $BaseDir "vision-env"
$ModelsDir = Join-Path $BaseDir "models"
$SidecarDir = Join-Path $BaseDir "vision-sidecar"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

New-Item -ItemType Directory -Force -Path $ModelsDir, $SidecarDir | Out-Null
Copy-Item -Force (Join-Path $ScriptDir "server.py") (Join-Path $SidecarDir "server.py")
Copy-Item -Force (Join-Path $ScriptDir "download_models.py") (Join-Path $SidecarDir "download_models.py")
Copy-Item -Force (Join-Path $ScriptDir "pyproject.toml") (Join-Path $SidecarDir "pyproject.toml")

$uv = Get-Command uv -ErrorAction SilentlyContinue
if (-not $uv) {
    throw "uv not found in PATH. Install uv first (https://docs.astral.sh/uv/)."
}

if (-not (Test-Path (Join-Path $VenvDir "Scripts\python.exe"))) {
    Write-Host "[1/4] Creating isolated Python 3.12 venv at $VenvDir ..."
    & $uv.Source venv --python 3.12 $VenvDir
}

$VenvPy = Join-Path $VenvDir "Scripts\python.exe"
Write-Host "[2/4] Installing base + ONNX dependencies ..."
& $uv.Source pip install --python $VenvPy fastapi uvicorn pillow numpy opencv-python-headless onnxruntime huggingface-hub

if ($Tier -in @("balanced", "pro", "all")) {
    Write-Host "[3/4] Installing PyTorch CUDA 12.4 + SAM 2.1 + Florence-2 stack ..."
    & $uv.Source pip install --python $VenvPy --index-url https://download.pytorch.org/whl/cu124 torch torchvision
    & $uv.Source pip install --python $VenvPy ultralytics "transformers>=4.45.0,<5.0.0" timm einops accelerate
}

Write-Host "[4/4] Downloading model weights for tier=$Tier into $ModelsDir ..."
& $VenvPy (Join-Path $SidecarDir "download_models.py") --tier $Tier --models-dir $ModelsDir
