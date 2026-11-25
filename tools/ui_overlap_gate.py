#!/usr/bin/env python3
"""
Mechanical UI Pixel-Overlap Gate (>= 90% threshold) for Anything-to-Prompt.
Compares cropped real official app screenshots against our live rendered ATP UI,
computes per-pixel RGB similarity, within-tolerance overlap %, and 8x8 structural SSIM,
and writes 4-panel visual overlay composites: [Official | ATP | 50/50 Blend | Diff Heatmap].
"""
import json
import math
import os
import sys
from PIL import Image, ImageChops, ImageDraw

GATE_THRESHOLD = 90.0


def crop_box(img: Image.Image, box):
    if not box:
        return img.convert("RGB")
    x1, y1, x2, y2 = box
    w, h = img.size
    x1 = max(0, min(w - 1, int(x1)))
    y1 = max(0, min(h - 1, int(y1)))
    x2 = max(x1 + 1, min(w, int(x2)))
    y2 = max(y1 + 1, min(h, int(y2)))
    return img.crop((x1, y1, x2, y2)).convert("RGB")


def compute_metrics(ref_rgb: Image.Image, act_rgb: Image.Image, tol: int = 24):
    w, h = ref_rgb.size
    if act_rgb.size != (w, h):
        act_rgb = act_rgb.resize((w, h), Image.Resampling.LANCZOS)

    ref_px = list(ref_rgb.getdata())
    act_px = list(act_rgb.getdata())
    n = len(ref_px)
    if n == 0:
        return {"rgb_sim": 0.0, "tol_overlap": 0.0, "ssim": 0.0, "pass": False}, act_rgb

    total_abs = 0
    within_tol = 0
    ref_lum = [0.0] * n
    act_lum = [0.0] * n

    for i in range(n):
        r1, g1, b1 = ref_px[i]
        r2, g2, b2 = act_px[i]
        dr = abs(r1 - r2)
        dg = abs(g1 - g2)
        db = abs(b1 - b2)
        total_abs += dr + dg + db
        if dr <= tol and dg <= tol and db <= tol:
            within_tol += 1
        ref_lum[i] = 0.299 * r1 + 0.587 * g1 + 0.114 * b1
        act_lum[i] = 0.299 * r2 + 0.587 * g2 + 0.114 * b2

    rgb_sim = 100.0 * (1.0 - total_abs / (n * 3.0 * 255.0))
    tol_overlap = 100.0 * (within_tol / n)

    # Block 8x8 luminance SSIM
    c1 = (0.01 * 255) ** 2
    c2 = (0.03 * 255) ** 2
    bs = 8
    ssim_sum = 0.0
    blocks = 0
    for by in range(0, h - bs + 1, bs):
        for bx in range(0, w - bs + 1, bs):
            xs = []
            ys = []
            for dy in range(bs):
                row = (by + dy) * w + bx
                for dx in range(bs):
                    idx = row + dx
                    xs.append(ref_lum[idx])
                    ys.append(act_lum[idx])
            m = len(xs)
            mx = sum(xs) / m
            my = sum(ys) / m
            vx = sum((x - mx) ** 2 for x in xs) / m
            vy = sum((y - my) ** 2 for y in ys) / m
            cov = sum((xs[k] - mx) * (ys[k] - my) for k in range(m)) / m
            num = (2 * mx * my + c1) * (2 * cov + c2)
            den = (mx * mx + my * my + c1) * (vx + vy + c2)
            ssim_sum += (num / den) if den != 0 else 1.0
            blocks += 1

    ssim_pct = 100.0 * (ssim_sum / blocks) if blocks > 0 else rgb_sim
    # Gate passes ONLY if RGB similarity >= 90% AND within-tolerance pixel overlap >= 90%
    passed = (rgb_sim >= GATE_THRESHOLD) and (tol_overlap >= GATE_THRESHOLD)
    return {
        "width": w,
        "height": h,
        "rgb_sim": round(rgb_sim, 2),
        "tol_overlap": round(tol_overlap, 2),
        "ssim": round(ssim_pct, 2),
        "pass": passed,
    }, act_rgb


def save_comparison(out_path: str, ref_rgb: Image.Image, act_rgb: Image.Image, name: str, metrics: dict):
    w, h = ref_rgb.size
    blend = Image.blend(ref_rgb, act_rgb, 0.5)
    diff = ImageChops.difference(ref_rgb, act_rgb)
    # Amplify diff into red heatmap for instant visual inspection
    diff_px = []
    for r, g, b in diff.getdata():
        mag = max(r, g, b)
        if mag <= 24:
            diff_px.append((20, 36, 24))  # dark green = matched within tolerance
        else:
            boost = min(255, mag * 3)
            diff_px.append((boost, max(0, 80 - mag), 24))
    heat = Image.new("RGB", (w, h))
    heat.putdata(diff_px)

    pad = 12
    hdr = 30
    canvas = Image.new("RGB", (w * 2 + pad * 3, h * 2 + hdr * 2 + pad * 3), (14, 15, 18))
    draw = ImageDraw.Draw(canvas)
    status = "PASS" if metrics["pass"] else "FAIL"
    title = f"{name} [{status}] | RGB={metrics['rgb_sim']}% | Overlap(<=24)={metrics['tol_overlap']}% | SSIM={metrics['ssim']}%"
    draw.text((pad, 6), title, fill=(74, 222, 128) if metrics["pass"] else (248, 113, 113))
    draw.text((pad, hdr + 2), "1. Official App Reference", fill=(180, 186, 196))
    draw.text((w + pad * 2, hdr + 2), "2. ATP Live Render", fill=(180, 186, 196))
    draw.text((pad, hdr + h + pad * 2 - 14), "3. 50/50 Overlay Blend", fill=(180, 186, 196))
    draw.text((w + pad * 2, hdr + h + pad * 2 - 14), "4. Pixel Diff Heatmap (Green=Match, Red=Diff)", fill=(180, 186, 196))

    y_top = hdr + 18
    y_bot = y_top + h + pad + 18
    canvas.paste(ref_rgb, (pad, y_top))
    canvas.paste(act_rgb, (w + pad * 2, y_top))
    canvas.paste(blend, (pad, y_bot))
    canvas.paste(heat, (w + pad * 2, y_bot))
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    canvas.save(out_path)


def main():
    if len(sys.argv) < 2:
        print("Usage: ui_overlap_gate.py <spec.json>")
        sys.exit(2)
    with open(sys.argv[1], "r", encoding="utf-8") as f:
        spec = json.load(f)
    out_dir = spec.get("outDir", ".")
    results = []
    all_pass = True
    for item in spec.get("cases", []):
        name = item["name"]
        ref_img = crop_box(Image.open(item["ref"]), item.get("refCrop"))
        act_img = crop_box(Image.open(item["actual"]), item.get("actualCrop"))
        tol = int(item.get("tol", 24))
        metrics, resized_act = compute_metrics(ref_img, act_img, tol=tol)
        cmp_path = os.path.join(out_dir, f"{name}_compare.png")
        save_comparison(cmp_path, ref_img, resized_act, name, metrics)
        metrics["name"] = name
        metrics["compareImage"] = cmp_path
        results.append(metrics)
        if not metrics["pass"]:
            all_pass = False
    print(json.dumps({"threshold": GATE_THRESHOLD, "allPass": all_pass, "results": results}, ensure_ascii=False, indent=2))
    sys.exit(0 if all_pass else 1)


if __name__ == "__main__":
    main()
