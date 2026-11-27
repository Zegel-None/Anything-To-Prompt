"""
Anything-to-Prompt Local Vision Sidecar
Unified multi-tier inference server supporting:
  - Lite Office (0 VRAM): MobileSAM ONNX on CPU
  - Balanced Universal (6-8GB VRAM): SAM 2.1 Tiny + Florence-2-base
  - Pro AI Workstation (>=12GB VRAM): SAM 2.1 Small + Florence-2-large
Includes automatic idle GPU VRAM unloading and organic polygon RDP simplification.
"""
from __future__ import annotations

import argparse
import base64
import gc
import io
import os
import threading
import time
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

import numpy as np
from fastapi import FastAPI
from PIL import Image
from pydantic import BaseModel

try:
    import cv2  # type: ignore
except Exception:
    cv2 = None

try:
    import onnxruntime as ort  # type: ignore
except Exception:
    ort = None

try:
    import torch  # type: ignore
except Exception:
    torch = None


ZH_LABEL_MAP: Dict[str, str] = {
    "person": "人物",
    "man": "人物(男)",
    "woman": "人物(女)",
    "boy": "男孩",
    "girl": "女孩",
    "child": "儿童",
    "human face": "人脸",
    "face": "人脸",
    "grass": "草地",
    "lawn": "草坪",
    "mountain": "山脉",
    "hill": "山丘",
    "river": "河流",
    "water": "水面",
    "lake": "湖泊",
    "sea": "大海",
    "sky": "天空",
    "cloud": "云层",
    "tree": "树木",
    "forest": "森林",
    "flower": "花朵",
    "plant": "植物",
    "building": "建筑",
    "house": "房屋",
    "road": "道路",
    "car": "汽车",
    "dog": "小狗",
    "cat": "小猫",
    "bird": "鸟",
    "rock": "岩石",
    "bridge": "桥梁",
    "boat": "船只",
}


def localize_label(raw: str) -> str:
    clean = raw.strip()
    if not clean:
        return "图像主体"
    low = clean.lower()
    if low in ZH_LABEL_MAP:
        return ZH_LABEL_MAP[low]
    for k, v in ZH_LABEL_MAP.items():
        if low.startswith(k + " ") or low.endswith(" " + k):
            return f"{v} ({clean})"
    return clean


def decode_b64_image(b64_str: str) -> Image.Image:
    raw = base64.b64decode(b64_str)
    return Image.open(io.BytesIO(raw)).convert("RGB")


def mask_to_polygon(mask_bool: np.ndarray, max_points: int = 56) -> List[List[int]]:
    """Extracts the largest connected foreground contour and simplifies it to <= max_points vertices."""
    if mask_bool.ndim != 2 or mask_bool.size == 0:
        return []
    u8 = (mask_bool.astype(np.uint8) * 255)
    if cv2 is not None:
        contours, _ = cv2.findContours(u8, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
        if not contours:
            return []
        largest = max(contours, key=cv2.contourArea)
        if cv2.contourArea(largest) < 36:
            return []
        peri = cv2.arcLength(largest, True)
        eps = max(1.5, 0.008 * peri)
        approx = cv2.approxPolyDP(largest, eps, True)
        pts = [[int(p[0][0]), int(p[0][1])] for p in approx]
        while len(pts) > max_points and eps < 40.0:
            eps *= 1.35
            approx = cv2.approxPolyDP(largest, eps, True)
            pts = [[int(p[0][0]), int(p[0][1])] for p in approx]
        return pts if len(pts) >= 3 else []

    # Pure-NumPy bounding box fallback if OpenCV is absent
    ys, xs = np.where(mask_bool)
    if len(xs) < 12:
        return []
    x0, x1 = int(xs.min()), int(xs.max())
    y0, y1 = int(ys.min()), int(ys.max())
    return [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]


def grabcut_box_polygon(img_rgb: np.ndarray, bbox: List[int]) -> List[List[int]]:
    """Fast CPU-only fallback polygon extractor using OpenCV GrabCut inside a box."""
    x, y, w, h = [int(v) for v in bbox]
    ih, iw = img_rgb.shape[:2]
    x = max(0, min(iw - 2, x))
    y = max(0, min(ih - 2, y))
    w = max(2, min(iw - x, w))
    h = max(2, min(ih - y, h))
    if cv2 is None or w < 16 or h < 16:
        return [[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
    try:
        mask = np.zeros((ih, iw), np.uint8)
        bgd = np.zeros((1, 65), np.float64)
        fgd = np.zeros((1, 65), np.float64)
        rect = (x, y, max(2, w - 1), max(2, h - 1))
        cv2.grabCut(img_rgb, mask, rect, bgd, fgd, 2, cv2.GC_INIT_WITH_RECT)
        fg = (mask == cv2.GC_FGD) | (mask == cv2.GC_PR_FGD)
        poly = mask_to_polygon(fg)
        if len(poly) >= 3:
            return poly
    except Exception:
        pass
    return [[x, y], [x + w, y], [x + w, y + h], [x, y + h]]


def classify_landscape_region(img_rgb: np.ndarray, mask_bool: np.ndarray) -> str:
    """Fast HSV + spatial-zone semantic namer for landscape SAM masks (sky/cloud/mountain/grass/river/tree)."""
    ys, xs = np.where(mask_bool)
    if len(ys) < 16:
        return "画面主体"
    ih, iw = img_rgb.shape[:2]
    cy_ratio = float(ys.mean()) / max(1, ih)
    top_ratio = float(ys.min()) / max(1, ih)
    pixels = img_rgb[mask_bool].astype(np.float32)
    r, g, b = float(pixels[:, 0].mean()), float(pixels[:, 1].mean()), float(pixels[:, 2].mean())
    maxc = max(r, g, b)
    minc = min(r, g, b)
    sat = (maxc - minc) / max(1.0, maxc)
    lum = 0.299 * r + 0.587 * g + 0.114 * b

    # 1. Bright white / pale gray in upper half -> Cloud or Snow Peak
    if lum >= 205 and sat < 0.18 and cy_ratio < 0.52:
        return "云层" if top_ratio < 0.28 else "雪山/云雾"
    # 2. Blue dominant: Sky vs Distant hazy mountain vs River/Water
    if b > r + 18 and b >= g - 8:
        if cy_ratio < 0.44 and top_ratio < 0.18:
            return "天空"
        if cy_ratio < 0.54 and sat < 0.34:
            return "山脉"
        return "河流/水面"
    # 3. Green dominant -> Grass vs Forest/Trees
    if g > r + 10 and g > b + 8:
        if lum < 105 or cy_ratio < 0.58:
            return "树木/山林"
        return "草地"
    # 4. Mid/upper muted slate, brown, gray-blue -> Mountain / Distant Hills
    if cy_ratio < 0.65 and sat < 0.35 and 45 <= lum <= 185:
        return "山脉"
    # 5. Warm golden/brown/ochre -> Ground / Road / Building
    if r > b + 20 and g > b + 8:
        return "大地/道路" if cy_ratio >= 0.55 else "建筑/景物"
    return "前景主体" if cy_ratio >= 0.45 else "景物区域"


class ModelManager:
    def __init__(self, models_dir: Path, idle_unload_sec: int) -> None:
        self.models_dir = models_dir
        self.models_dir.mkdir(parents=True, exist_ok=True)
        self.idle_unload_sec = max(30, idle_unload_sec)
        self.lock = threading.RLock()
        self.last_used = time.time()
        self.loaded: Dict[str, Any] = {}
        self._stop = threading.Event()
        self._watchdog = threading.Thread(target=self._idle_loop, daemon=True)
        self._watchdog.start()

    def _idle_loop(self) -> None:
        while not self._stop.wait(15.0):
            with self.lock:
                if self.loaded and (time.time() - self.last_used) >= self.idle_unload_sec:
                    self.unload_all()

    def touch(self) -> None:
        self.last_used = time.time()

    def cuda_ready(self, min_free_mb: int = 1800) -> bool:
        if torch is None or not torch.cuda.is_available():
            return False
        try:
            free_b, _total_b = torch.cuda.mem_get_info()
            return (free_b // (1024 * 1024)) >= min_free_mb
        except Exception:
            return False

    def vram_allocated_mb(self) -> int:
        if torch is None or not torch.cuda.is_available():
            return 0
        try:
            return int(torch.cuda.memory_allocated() // (1024 * 1024))
        except Exception:
            return 0

    def available_backends(self) -> List[str]:
        out: List[str] = []
        if ort is not None and (self.models_dir / "mobilesam" / "mobile_sam_encoder.onnx").is_file():
            out.append("onnx_mobilesam_cpu")
        elif cv2 is not None:
            out.append("cpu_grabcut_fallback")
        if torch is not None:
            if (self.models_dir / "omniparser" / "icon_detect.pt").is_file():
                out.append("omniparser_v2_ui")
            if (self.models_dir / "sam2" / "sam2.1_t.pt").is_file():
                out.append("gpu_sam2_tiny")
            if (self.models_dir / "sam2" / "sam2.1_s.pt").is_file():
                out.append("gpu_sam2_small")
            if (self.models_dir / "florence2-base").is_dir():
                out.append("florence2_base")
            if (self.models_dir / "florence2-large").is_dir():
                out.append("florence2_large")
        return out

    def _get_omniparser_yolo(self) -> Optional[Any]:
        key = "omniparser_v2_yolo"
        if key in self.loaded:
            return self.loaded[key]
        weight = self.models_dir / "omniparser" / "icon_detect.pt"
        if not weight.is_file():
            return None
        try:
            from ultralytics import YOLO  # type: ignore

            model = YOLO(str(weight))
            self.loaded[key] = model
            return model
        except Exception:
            return None

    def detect_ui_icons(
        self, img_pil: Image.Image, conf: float = 0.12, iou: float = 0.45
    ) -> List[Dict[str, Any]]:
        """Runs Microsoft OmniParser v2 (YOLO icon_detect) to locate UI buttons/icons in ~20-60ms."""
        with self.lock:
            self.touch()
            model = self._get_omniparser_yolo()
            if model is None:
                return []
            device = "cuda:0" if self.cuda_ready(900) else "cpu"
            img_rgb = np.array(img_pil)
            ih, iw = img_rgb.shape[:2]
            imgsz = min(1280, max(640, ((max(iw, ih) + 31) // 32) * 32))
            preds = model.predict(
                img_rgb,
                conf=max(0.05, min(0.85, conf)),
                iou=max(0.1, min(0.85, iou)),
                imgsz=imgsz,
                device=device,
                verbose=False,
            )
            if not preds or preds[0].boxes is None:
                return []
            xyxy = preds[0].boxes.xyxy.cpu().numpy()
            confs = preds[0].boxes.conf.cpu().numpy()
            out: List[Dict[str, Any]] = []
            for box, sc in zip(xyxy, confs):
                x0 = max(0, int(round(float(box[0]))))
                y0 = max(0, int(round(float(box[1]))))
                x1 = min(iw, int(round(float(box[2]))))
                y1 = min(ih, int(round(float(box[3]))))
                w = x1 - x0
                h = y1 - y0
                if w >= 12 and h >= 12:
                    out.append({"bbox": [x0, y0, w, h], "score": round(float(sc), 4)})
            # Sort left-to-right, top-to-bottom for deterministic order
            out.sort(key=lambda b: (b["bbox"][1] // 16, b["bbox"][0]))
            return out[:180]

    def unload_all(self) -> None:
        with self.lock:
            self.loaded.clear()
            gc.collect()
            if torch is not None and torch.cuda.is_available():
                try:
                    torch.cuda.empty_cache()
                except Exception:
                    pass

    def _get_mobilesam_onnx(self) -> Optional[Tuple[Any, Any]]:
        key = "mobilesam_onnx"
        if key in self.loaded:
            return self.loaded[key]
        if ort is None:
            return None
        enc_path = self.models_dir / "mobilesam" / "mobile_sam_encoder.onnx"
        dec_path = self.models_dir / "mobilesam" / "mobile_sam_decoder.onnx"
        if not enc_path.is_file() or not dec_path.is_file():
            return None
        opts = ort.SessionOptions()
        opts.intra_op_num_threads = max(2, min(8, os.cpu_count() or 4))
        enc = ort.InferenceSession(str(enc_path), sess_options=opts, providers=["CPUExecutionProvider"])
        dec = ort.InferenceSession(str(dec_path), sess_options=opts, providers=["CPUExecutionProvider"])
        self.loaded[key] = (enc, dec)
        return (enc, dec)

    def _run_mobilesam_onnx_boxes(
        self, img_rgb: np.ndarray, boxes: List[Dict[str, Any]]
    ) -> Optional[List[Dict[str, Any]]]:
        sess = self._get_mobilesam_onnx()
        if sess is None or cv2 is None:
            return None
        enc, dec = sess
        ih, iw = img_rgb.shape[:2]
        scale = 1024.0 / max(ih, iw)
        nh, nw = int(round(ih * scale)), int(round(iw * scale))
        enc_in = enc.get_inputs()[0]
        enc_in_name = enc_in.name
        if len(enc_in.shape) == 3:
            # Acly/MobileSAM graph takes raw HxWx3 float32 directly and handles resize/norm internally
            inp = img_rgb.astype(np.float32)
        else:
            resized = cv2.resize(img_rgb, (nw, nh), interpolation=cv2.INTER_LINEAR).astype(np.float32)
            mean = np.array([123.675, 116.28, 103.53], dtype=np.float32)
            std = np.array([58.395, 57.12, 57.375], dtype=np.float32)
            norm = (resized - mean) / std
            padded = np.zeros((1024, 1024, 3), dtype=np.float32)
            padded[:nh, :nw, :] = norm
            inp = np.transpose(padded, (2, 0, 1))[None, :, :, :]

        emb = enc.run(None, {enc_in_name: inp})[0]

        results: List[Dict[str, Any]] = []
        dec_inputs = {i.name: i for i in dec.get_inputs()}
        for item in boxes:
            x, y, w, h = [int(v) for v in item["bbox"]]
            # Box prompt encoded as 2 corner points with labels [2, 3]
            pts = np.array([[[x * scale, y * scale], [(x + w) * scale, (y + h) * scale]]], dtype=np.float32)
            lbls = np.array([[2.0, 3.0]], dtype=np.float32)
            feed: Dict[str, Any] = {}
            for name in dec_inputs:
                low = name.lower()
                if "embed" in low:
                    feed[name] = emb
                elif "coord" in low:
                    feed[name] = pts
                elif "label" in low:
                    feed[name] = lbls
                elif "mask_input" in low and "has" not in low:
                    feed[name] = np.zeros((1, 1, 256, 256), dtype=np.float32)
                elif "has_mask" in low:
                    feed[name] = np.zeros((1,), dtype=np.float32)
                elif "orig_im_size" in low:
                    feed[name] = np.array([ih, iw], dtype=np.float32)
            outs = dec.run(None, feed)
            mask_logits = outs[0]
            best_idx = 0
            if len(outs) >= 2 and outs[1].ndim == 2 and outs[1].shape[1] > 1:
                best_idx = int(np.argmax(outs[1][0]))
            if mask_logits.ndim == 4:
                m = mask_logits[0, min(best_idx, mask_logits.shape[1] - 1)] > 0.0
            else:
                m = mask_logits[0] > 0.0
            if m.shape != (ih, iw):
                m = cv2.resize(m.astype(np.uint8), (iw, ih), interpolation=cv2.INTER_NEAREST) > 0
            poly = mask_to_polygon(m)
            if len(poly) < 3:
                poly = grabcut_box_polygon(img_rgb, [x, y, w, h])
            results.append({
                "label": localize_label(str(item.get("label", "主体"))),
                "score": 0.9,
                "bbox": [x, y, w, h],
                "polygon": poly,
            })
        return results

    def _get_sam2(self, variant: str) -> Optional[Any]:
        key = f"sam2_{variant}"
        if key in self.loaded:
            return self.loaded[key]
        try:
            from ultralytics import SAM  # type: ignore
        except Exception:
            return None
        fname = "sam2.1_s.pt" if variant == "small" else "sam2.1_t.pt"
        weight = self.models_dir / "sam2" / fname
        if not weight.is_file():
            return None
        model = SAM(str(weight))
        self.loaded[key] = model
        return model

    def _run_sam2_boxes(
        self, variant: str, img_rgb: np.ndarray, boxes: List[Dict[str, Any]]
    ) -> Optional[List[Dict[str, Any]]]:
        model = self._get_sam2(variant)
        if model is None:
            return None
        # Measured live: SAM 2.1 Tiny uses only ~273 MB VRAM on CUDA!
        device = "cuda:0" if self.cuda_ready(450) else "cpu"
        xyxy = [[int(b["bbox"][0]), int(b["bbox"][1]), int(b["bbox"][0] + b["bbox"][2]), int(b["bbox"][1] + b["bbox"][3])] for b in boxes]
        preds = model.predict(img_rgb, bboxes=xyxy, device=device, verbose=False)
        if not preds or preds[0].masks is None:
            return None
        masks_data = preds[0].masks.data.cpu().numpy()
        ih, iw = img_rgb.shape[:2]
        out: List[Dict[str, Any]] = []
        for idx, item in enumerate(boxes):
            bbox = [int(v) for v in item["bbox"]]
            poly: List[List[int]] = []
            if idx < len(masks_data):
                m = masks_data[idx] > 0.5
                if m.shape != (ih, iw) and cv2 is not None:
                    m = cv2.resize(m.astype(np.uint8), (iw, ih), interpolation=cv2.INTER_NEAREST) > 0
                poly = mask_to_polygon(m)
            if len(poly) < 3:
                poly = grabcut_box_polygon(img_rgb, bbox)
            out.append({
                "label": localize_label(str(item.get("label", "主体"))),
                "score": 0.94,
                "bbox": bbox,
                "polygon": poly,
            })
        return out

    def _run_sam2_everything_photo(
        self, variant: str, img_rgb: np.ndarray, extra_points: List[List[int]], od_boxes: List[Dict[str, Any]]
    ) -> Optional[List[Dict[str, Any]]]:
        """Segments BOTH foreground subjects AND natural landscape regions (sky, mountains, grass, river, trees)
        in one fast batched SAM 2.1 CUDA pass (~273 MB VRAM, ~150-500ms)!"""
        model = self._get_sam2(variant)
        if model is None:
            return None
        device = "cuda:0" if self.cuda_ready(450) else "cpu"
        ih, iw = img_rgb.shape[:2]
        # 5x4 uniform grid covers sky, mountains, river, grass, left/right subjects
        grid_pts = [
            [[int(iw * (gx + 0.5) / 5.0), int(ih * (gy + 0.5) / 4.0)]]
            for gy in range(4)
            for gx in range(5)
        ]
        for pt in extra_points[:6]:
            if len(pt) == 2 and 0 <= pt[0] < iw and 0 <= pt[1] < ih:
                grid_pts.append([[int(pt[0]), int(pt[1])]])
        grid_lbls = [[1] for _ in grid_pts]

        preds = model.predict(img_rgb, points=grid_pts, labels=grid_lbls, device=device, verbose=False)
        raw_masks: List[Tuple[np.ndarray, Optional[str]]] = []
        if preds and preds[0].masks is not None:
            for m in preds[0].masks.data.cpu().numpy():
                mb = m > 0.5
                if mb.shape != (ih, iw) and cv2 is not None:
                    mb = cv2.resize(mb.astype(np.uint8), (iw, ih), interpolation=cv2.INTER_NEAREST) > 0
                raw_masks.append((mb, None))

        # Also add masks for any Florence-2 detected foreground objects (people, animals, buildings)
        if od_boxes:
            xyxy = [[b["bbox"][0], b["bbox"][1], b["bbox"][0] + b["bbox"][2], b["bbox"][1] + b["bbox"][3]] for b in od_boxes]
            bpreds = model.predict(img_rgb, bboxes=xyxy, device=device, verbose=False)
            if bpreds and bpreds[0].masks is not None:
                for m, binfo in zip(bpreds[0].masks.data.cpu().numpy(), od_boxes):
                    mb = m > 0.5
                    if mb.shape != (ih, iw) and cv2 is not None:
                        mb = cv2.resize(mb.astype(np.uint8), (iw, ih), interpolation=cv2.INTER_NEAREST) > 0
                    raw_masks.insert(0, (mb, binfo["label"]))

        img_area = float(max(1, ih * iw))
        kept: List[Tuple[np.ndarray, str, List[int], List[List[int]]]] = []
        for mb, forced_label in raw_masks:
            area = float(mb.sum())
            ratio = area / img_area
            if ratio < 0.008 or ratio > 0.68:
                continue
            dup = False
            for km, _, _, _ in kept:
                inter = float(np.logical_and(mb, km).sum())
                union = float(np.logical_or(mb, km).sum())
                if union > 0 and (inter / union) >= 0.58:
                    dup = True
                    break
            if dup:
                continue
            poly = mask_to_polygon(mb, max_points=52)
            if len(poly) < 3:
                continue
            ys, xs = np.where(mb)
            x0, x1 = int(xs.min()), int(xs.max())
            y0, y1 = int(ys.min()), int(ys.max())
            lbl = forced_label or classify_landscape_region(img_rgb, mb)
            kept.append((mb, lbl, [x0, y0, max(1, x1 - x0), max(1, y1 - y0)], poly))
            if len(kept) >= 14:
                break

        return [
            {"label": lbl, "score": 0.92, "bbox": bbox, "polygon": poly}
            for (_, lbl, bbox, poly) in kept
        ]

    def _get_florence2(self, variant: str) -> Optional[Tuple[Any, Any, str]]:
        key = f"florence2_{variant}"
        if key in self.loaded:
            return self.loaded[key]
        if torch is None:
            return None
        # Only run Florence-2 when CUDA has enough headroom so it NEVER stalls 120s on CPU!
        if not self.cuda_ready(1050):
            return None
        local_dir = self.models_dir / ("florence2-large" if variant == "large" else "florence2-base")
        if not local_dir.is_dir():
            return None
        try:
            from transformers import AutoModelForCausalLM, AutoProcessor  # type: ignore

            device = "cuda:0"
            dtype = torch.float16
            proc = AutoProcessor.from_pretrained(str(local_dir), trust_remote_code=True)
            model = AutoModelForCausalLM.from_pretrained(
                str(local_dir),
                dtype=dtype,
                trust_remote_code=True,
                attn_implementation="eager",
            ).to(device)
            model.eval()
            self.loaded[key] = (proc, model, device)
            return (proc, model, device)
        except Exception:
            return None

    def _run_florence2_detect(self, variant: str, img_pil: Image.Image) -> List[Dict[str, Any]]:
        pair = self._get_florence2(variant)
        if pair is None:
            return []
        proc, model, device = pair
        task = "<OD>"
        inputs = proc(text=task, images=img_pil, return_tensors="pt")
        dtype = torch.float16 if device.startswith("cuda") else torch.float32
        inputs = {
            k: (v.to(device=device, dtype=dtype) if v.is_floating_point() else v.to(device))
            for k, v in inputs.items()
        }
        with torch.no_grad():
            gen_ids = model.generate(
                input_ids=inputs["input_ids"],
                pixel_values=inputs["pixel_values"],
                max_new_tokens=96,
                num_beams=1,
                do_sample=False,
                use_cache=False,
            )
        gen_text = proc.batch_decode(gen_ids, skip_special_tokens=False)[0]
        parsed = proc.post_process_generation(gen_text, task=task, image_size=(img_pil.width, img_pil.height))
        od = parsed.get(task, {})
        bboxes = od.get("bboxes", [])
        labels = od.get("labels", [])
        out: List[Dict[str, Any]] = []
        for box, lbl in zip(bboxes, labels):
            x0, y0, x1, y1 = [int(round(v)) for v in box]
            w = max(0, x1 - x0)
            h = max(0, y1 - y0)
            if w >= 14 and h >= 14:
                out.append({"label": localize_label(str(lbl)), "bbox": [x0, y0, w, h]})
        return out[:18]

    def segment_boxes(self, backend: str, img_pil: Image.Image, boxes: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
        with self.lock:
            self.touch()
            img_rgb = np.array(img_pil)
            if not boxes:
                return []
            res: Optional[List[Dict[str, Any]]] = None
            if backend == "gpu_sam2_small":
                res = self._run_sam2_boxes("small", img_rgb, boxes) or self._run_sam2_boxes("tiny", img_rgb, boxes)
            elif backend == "gpu_sam2_tiny":
                res = self._run_sam2_boxes("tiny", img_rgb, boxes)
            elif backend == "onnx_mobilesam_cpu":
                res = self._run_mobilesam_onnx_boxes(img_rgb, boxes)
            if res is None:
                res = self._run_mobilesam_onnx_boxes(img_rgb, boxes)
            if res is not None:
                return res
            # Always gracefully produce organic GrabCut polygons if weights are still downloading
            return [
                {
                    "label": localize_label(str(b.get("label", "主体"))),
                    "score": 0.75,
                    "bbox": [int(v) for v in b["bbox"]],
                    "polygon": grabcut_box_polygon(img_rgb, b["bbox"]),
                }
                for b in boxes
            ]

    def auto_semantic_segment(
        self, backend: str, img_pil: Image.Image, points: List[List[int]], mode: str
    ) -> List[Dict[str, Any]]:
        with self.lock:
            self.touch()
            img_rgb = np.array(img_pil)
            fl_variant = "large" if backend == "gpu_sam2_small" else "base"
            od_boxes = self._run_florence2_detect(fl_variant, img_pil)
            if mode == "ui" and od_boxes:
                return [
                    {"label": b["label"], "score": 0.88, "bbox": b["bbox"], "polygon": []}
                    for b in od_boxes
                ]
            # For natural/landscape photos, run SAM 2.1 full-scene grid+object everything-matting
            # so sky, mountains, grass, rivers, trees AND people all emerge as organic polygons!
            if mode == "photo":
                sam_variant = "small" if backend == "gpu_sam2_small" else "tiny"
                res = self._run_sam2_everything_photo(sam_variant, img_rgb, points, od_boxes)
                if res:
                    return res
            if od_boxes:
                return self.segment_boxes(backend, img_pil, od_boxes)

            ih, iw = img_pil.height, img_pil.width
            if points:
                cx, cy = points[len(points) // 2]
            else:
                cx, cy = iw // 2, ih // 2
            bw, bh = max(48, iw // 2), max(48, ih // 2)
            x0 = max(0, min(iw - bw, cx - bw // 2))
            y0 = max(0, min(ih - bh, cy - bh // 2))
            return self.segment_boxes(backend, img_pil, [{"label": "图像主体", "bbox": [x0, y0, bw, bh]}])


class BoxItem(BaseModel):
    label: str = "主体"
    bbox: List[int]


class SegmentBoxesReq(BaseModel):
    backend: str = "onnx_mobilesam_cpu"
    imageB64: str
    boxes: List[BoxItem]
    points: List[List[int]] = []


class AutoSemanticReq(BaseModel):
    backend: str = "gpu_sam2_tiny"
    imageB64: str
    points: List[List[int]] = []
    mode: str = "photo"


class DetectUiIconsReq(BaseModel):
    imageB64: str
    conf: float = 0.12
    iou: float = 0.45


def build_app(mgr: ModelManager) -> FastAPI:
    app = FastAPI(title="Anything-to-Prompt Vision Sidecar")

    @app.get("/health")
    def health() -> Dict[str, Any]:
        cuda_avail = bool(torch is not None and torch.cuda.is_available())
        return {
            "ok": True,
            "device": "cuda" if mgr.cuda_ready(1200) else "cpu",
            "cudaAvailable": cuda_avail,
            "loadedModels": list(mgr.loaded.keys()),
            "availableBackends": mgr.available_backends(),
            "vramAllocatedMb": mgr.vram_allocated_mb(),
        }

    @app.post("/segment/boxes")
    def segment_boxes(req: SegmentBoxesReq) -> Dict[str, Any]:
        img = decode_b64_image(req.imageB64)
        segs = mgr.segment_boxes(
            req.backend,
            img,
            [{"label": b.label, "bbox": b.bbox} for b in req.boxes],
        )
        return {"ok": True, "segments": segs}

    @app.post("/segment/auto_semantic")
    def auto_semantic(req: AutoSemanticReq) -> Dict[str, Any]:
        img = decode_b64_image(req.imageB64)
        segs = mgr.auto_semantic_segment(req.backend, img, req.points, req.mode)
        return {"ok": True, "segments": segs}

    @app.post("/detect/ui_icons")
    def detect_ui_icons(req: DetectUiIconsReq) -> Dict[str, Any]:
        t0 = time.time()
        img = decode_b64_image(req.imageB64)
        boxes = mgr.detect_ui_icons(img, conf=req.conf, iou=req.iou)
        return {
            "ok": True,
            "boxes": boxes,
            "latencyMs": int((time.time() - t0) * 1000),
        }

    @app.post("/unload")
    def unload() -> Dict[str, Any]:
        mgr.unload_all()
        return {"ok": True}

    return app


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=18765)
    parser.add_argument(
        "--models-dir",
        default=str(Path(os.environ.get("LOCALAPPDATA", ".")) / "AnythingToPrompt" / "models"),
    )
    parser.add_argument("--idle-unload", type=int, default=180)
    args = parser.parse_args()

    import uvicorn

    mgr = ModelManager(Path(args.models_dir), args.idle_unload)
    app = build_app(mgr)
    uvicorn.run(app, host=args.host, port=args.port, log_level="warning")


if __name__ == "__main__":
    main()
