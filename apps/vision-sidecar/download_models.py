"""Downloads model weights for Anything-to-Prompt Vision Sidecar hardware tiers."""
from __future__ import annotations

import argparse
import os
import shutil
import urllib.request
from pathlib import Path

from huggingface_hub import hf_hub_download, list_repo_files, snapshot_download


def download_omniparser_v2(models_dir: Path) -> None:
    op = models_dir / "omniparser"
    op.mkdir(parents=True, exist_ok=True)
    pt_dst = op / "icon_detect.pt"
    if pt_dst.is_file() and pt_dst.stat().st_size > 1_000_000:
        print(f"[OmniParser-v2] already present: {pt_dst}")
        return
    try:
        print("[OmniParser-v2] downloading microsoft/OmniParser-v2.0 icon_detect/model.pt ...")
        p = hf_hub_download(
            repo_id="microsoft/OmniParser-v2.0",
            filename="icon_detect/model.pt",
            local_dir=str(op),
        )
        if Path(p).resolve() != pt_dst.resolve():
            shutil.copy2(p, pt_dst)
        print(f"[OmniParser-v2] ready: {pt_dst} ({pt_dst.stat().st_size / (1024*1024):.1f} MB)")
    except Exception as e:
        print(f"[OmniParser-v2] WARN: download skipped: {e}")


def download_mobilesam_onnx(models_dir: Path) -> None:
    ms = models_dir / "mobilesam"
    ms.mkdir(parents=True, exist_ok=True)
    enc_dst = ms / "mobile_sam_encoder.onnx"
    dec_dst = ms / "mobile_sam_decoder.onnx"
    if enc_dst.is_file() and dec_dst.is_file():
        print("[MobileSAM] already present")
        return

    repos = [
        "Acly/MobileSAM",
        "vietanhdev/segment-anything-onnx-models",
    ]
    for repo in repos:
        try:
            files = list_repo_files(repo)
            enc_file = next(
                (f for f in files if f.lower().endswith(".onnx") and "enc" in f.lower() and ("mobile" in f.lower() or "vit_t" in f.lower() or repo == "Acly/MobileSAM")),
                None,
            )
            dec_file = next(
                (f for f in files if f.lower().endswith(".onnx") and "dec" in f.lower() and ("mobile" in f.lower() or "vit_t" in f.lower() or repo == "Acly/MobileSAM")),
                None,
            )
            if enc_file and dec_file:
                print(f"[MobileSAM] downloading from {repo}: {enc_file}, {dec_file}")
                p_enc = hf_hub_download(repo_id=repo, filename=enc_file, local_dir=str(ms))
                p_dec = hf_hub_download(repo_id=repo, filename=dec_file, local_dir=str(ms))
                if Path(p_enc).resolve() != enc_dst.resolve():
                    shutil.copy2(p_enc, enc_dst)
                if Path(p_dec).resolve() != dec_dst.resolve():
                    shutil.copy2(p_dec, dec_dst)
                print("[MobileSAM] ready:", enc_dst, dec_dst)
                return
        except Exception as e:
            print(f"[MobileSAM] repo {repo} skipped: {e}")
    print("[MobileSAM] WARN: could not locate pre-exported ONNX; GrabCut CPU fallback remains active")


def download_sam2(models_dir: Path, tier: str) -> None:
    s2 = models_dir / "sam2"
    s2.mkdir(parents=True, exist_ok=True)
    targets = []
    if tier in ("balanced", "all"):
        targets.append(("sam2.1_t.pt", "https://github.com/ultralytics/assets/releases/download/v8.3.0/sam2.1_t.pt"))
    if tier in ("pro", "all"):
        targets.append(("sam2.1_s.pt", "https://github.com/ultralytics/assets/releases/download/v8.3.0/sam2.1_s.pt"))
    for fname, url in targets:
        dst = s2 / fname
        if dst.is_file() and dst.stat().st_size > 1_000_000:
            print(f"[SAM2] already present: {dst}")
            continue
        print(f"[SAM2] downloading {url} -> {dst}")
        urllib.request.urlretrieve(url, str(dst))
        print(f"[SAM2] saved {dst} ({dst.stat().st_size / (1024*1024):.1f} MB)")


def download_florence2(models_dir: Path, tier: str) -> None:
    if tier in ("balanced", "all"):
        fb = models_dir / "florence2-base"
        if (fb / "config.json").is_file() and any(fb.glob("*.safetensors")):
            print(f"[Florence-2-base] already present: {fb}")
        else:
            print("[Florence-2-base] downloading microsoft/Florence-2-base ...")
            snapshot_download("microsoft/Florence-2-base", local_dir=str(fb))
            print("[Florence-2-base] ready")
    if tier in ("pro", "all"):
        fl = models_dir / "florence2-large"
        if (fl / "config.json").is_file() and any(fl.glob("*.safetensors")):
            print(f"[Florence-2-large] already present: {fl}")
        else:
            print("[Florence-2-large] downloading microsoft/Florence-2-large ...")
            snapshot_download("microsoft/Florence-2-large", local_dir=str(fl))
            print("[Florence-2-large] ready")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tier", choices=["lite", "balanced", "pro", "all"], default="lite")
    parser.add_argument(
        "--models-dir",
        default=str(Path(os.environ.get("LOCALAPPDATA", ".")) / "AnythingToPrompt" / "models"),
    )
    args = parser.parse_args()
    models_dir = Path(args.models_dir)
    models_dir.mkdir(parents=True, exist_ok=True)

    if args.tier in ("lite", "all"):
        download_omniparser_v2(models_dir)
        download_mobilesam_onnx(models_dir)
    if args.tier in ("balanced", "pro", "all"):
        download_omniparser_v2(models_dir)
        download_sam2(models_dir, args.tier)
        download_florence2(models_dir, args.tier)
    print("ALL_MODELS_READY")


if __name__ == "__main__":
    main()
