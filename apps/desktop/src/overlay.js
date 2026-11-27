// Anything-to-Prompt — Click-through Selection Overlay Canvas
// (Restrained Blue Light Trail + Apple-Blue Glow Hover + 0.5s Background Dwell Ring + Stroke)
(function () {
  const canvas = document.getElementById('cv');
  const ctx = canvas.getContext('2d');
  const toastEl = document.getElementById('toast');

  const TRAIL_TTL_MS = 95; // Short, restrained silky trail
  const MAX_TRAIL_POINTS = 12;

  const BG_DWELL_MS = 500; // 0.5s stationary dwell required to unlock background/desktop
  const BG_RING_SHOW_AFTER_MS = 75; // Filter quick cursor sweeps between foreground elements
  const BG_DWELL_JITTER_SQ = 100; // 10px physical tolerance for natural hand micro-tremor

  const state = {
    active: false,
    monX: 0,
    monY: 0,
    scale: window.devicePixelRatio || 1,
    cursor: null,
    trail: [],
    hover: null,
    dwell: null, // { key, originX, originY, startTs, unlocked }
    liveStroke: [],
    liveStrokeColor: 'blue', // 'blue' (left-drag box/lasso) | 'red' (right-drag doodle screenshot)
    blueMorph: {
      lockedLasso: false,
      morphStartTs: 0,
      lastBox: null, // { x, y, w, h } in CSS coords for smooth 160ms box -> lasso morph
    },
    liveDrag: null, // { mode: 'word_text' | 'excel_box', start: {x,y}, end: {x,y} }
    pickedItems: [],
    toastTimer: null,
    rafId: 0,
    scoutPulse: null, // { tier: 1|2, x, y, startTs } — subtle cursor wake ripple (no yellow water!)
    scan: {
      active: false, // True when Dose 2 revealed photo cutouts or expanded submenu items
      revealed: [], // [{ key, rect, polygon, semanticKind, revealedAt, label, isExpandable }]
      revealedSet: new Set(),
    },
  };

  let lastScanSnapshotForCompose = null; // Preserved briefly across pen:hide so red-doodle captures active cutouts

  function snapshotScanForCompose() {
    if (!state.scan.active || state.scan.revealed.length === 0) return;
    lastScanSnapshotForCompose = {
      ts: performance.now(),
      monX: state.monX,
      monY: state.monY,
      scale: state.scale || window.devicePixelRatio || 1,
      revealed: state.scan.revealed.map((it) => ({
        rect: it.rect,
        polygon: it.polygon || null,
        label: it.label || '',
        semanticKind: it.semanticKind || 'ui',
        isExpandable: Boolean(it.isExpandable),
      })),
    };
  }

  function clearScanLayer() {
    if (state.scan.active) {
      snapshotScanForCompose();
    }
    state.scan.active = false;
    state.scan.revealed = [];
    state.scan.revealedSet = new Set();
  }

  function drawScanInspectionLayer(now) {
    if (!state.scan.active || state.scan.revealed.length === 0) return;

    // Draw Dose 2 revealed organic photo cutouts & expanded submenu items
    for (const item of state.scan.revealed) {
      const elapsed = now - item.revealedAt;
      if (elapsed < 0) continue;
      const rawT = Math.min(1, elapsed / 340);
      const c1 = 1.70158;
      const c3 = c1 + 1;
      const ease = 1 + c3 * Math.pow(rawT - 1, 3) + c1 * Math.pow(rawT - 1, 2);
      const scale = 0.92 + 0.08 * ease;

      const isPhoto = item.semanticKind === 'photo_object';
      const isVlmFixed = item.semanticKind === 'ui_vlm_fixed';
      const isMenu = item.semanticKind === 'menu_item';

      if (Array.isArray(item.polygon) && item.polygon.length >= 3) {
        const cssPts = item.polygon.map((pt) => toCssPt(pt[0], pt[1]));
        let minY = cssPts[0].y;
        let topX = cssPts[0].x;
        ctx.save();
        ctx.beginPath();
        ctx.moveTo(cssPts[0].x, cssPts[0].y);
        for (let i = 1; i < cssPts.length; i += 1) {
          ctx.lineTo(cssPts[i].x, cssPts[i].y);
          if (cssPts[i].y < minY) {
            minY = cssPts[i].y;
            topX = cssPts[i].x;
          }
        }
        ctx.closePath();
        const whiteAlpha = 0.22 * (1 - rawT * 0.65);
        ctx.fillStyle = `rgba(224, 242, 254, ${whiteAlpha.toFixed(3)})`;
        ctx.strokeStyle = '#22d3ee';
        ctx.shadowColor = 'rgba(14, 165, 233, 0.92)';
        ctx.shadowBlur = 12 + 6 * (1 - rawT);
        ctx.lineWidth = 2.1;
        ctx.lineJoin = 'round';
        ctx.fill();
        ctx.stroke();
        ctx.restore();

        if (item.label) {
          drawCompactSemanticTag(topX, Math.max(10, minY - 22), item.label, '#0284c7');
        }
        continue;
      }

      const r = toCssRect(item.rect);
      if (!r || r.w <= 2 || r.h <= 2) continue;
      const cx = r.x + r.w / 2;
      const cy = r.y + r.h / 2;
      const bw = Math.max(4, r.w * scale);
      const bh = Math.max(4, r.h * scale);
      const bx = cx - bw / 2;
      const by = cy - bh / 2;

      const rippleT = Math.min(1, elapsed / 440);
      if (rippleT < 1) {
        const expand = 12 * rippleT;
        ctx.save();
        ctx.strokeStyle = isVlmFixed
          ? `rgba(192, 132, 252, ${(0.72 * (1 - rippleT)).toFixed(3)})`
          : `rgba(125, 211, 252, ${(0.68 * (1 - rippleT)).toFixed(3)})`;
        ctx.lineWidth = Math.max(0.8, 2.0 * (1 - rippleT));
        ctx.beginPath();
        ctx.roundRect(bx - expand, by - expand, bw + expand * 2, bh + expand * 2, 7 + expand * 0.5);
        ctx.stroke();
        ctx.restore();
      }

      ctx.save();
      const whiteAlpha = 0.22 * (1 - rawT * 0.65);
      ctx.fillStyle = `rgba(255, 255, 255, ${whiteAlpha.toFixed(3)})`;
      ctx.strokeStyle = isVlmFixed ? '#818cf8' : isMenu ? '#38bdf8' : '#60a5fa';
      ctx.shadowColor = isVlmFixed ? 'rgba(129, 140, 248, 0.90)' : 'rgba(0, 140, 255, 0.85)';
      ctx.shadowBlur = 10 + 6 * (1 - rawT);
      ctx.lineWidth = isPhoto || isVlmFixed ? 2.0 : 1.7;
      ctx.beginPath();
      ctx.roundRect(bx, by, bw, bh, 6);
      ctx.fill();
      ctx.stroke();
      ctx.restore();

      if ((isPhoto || isMenu) && item.label) {
        drawCompactSemanticTag(bx + 6, Math.max(10, by - 20), item.label, '#0284c7');
      }
    }
  }

  function drawCompactSemanticTag(x, y, text, accent) {
    ctx.save();
    ctx.font = '600 11px system-ui, -apple-system, "Segoe UI", "PingFang SC", sans-serif';
    const padX = 7;
    const h = 19;
    const w = Math.ceil(ctx.measureText(text).width) + padX * 2;
    const cx = Math.max(6, Math.min(window.innerWidth - w - 6, x));
    const cy = Math.max(6, Math.min(window.innerHeight - h - 6, y));
    ctx.fillStyle = 'rgba(15, 23, 42, 0.86)';
    ctx.strokeStyle = accent || '#38bdf8';
    ctx.lineWidth = 1.1;
    ctx.beginPath();
    ctx.roundRect(cx, cy, w, h, 999);
    ctx.fill();
    ctx.stroke();
    ctx.fillStyle = '#e0f2fe';
    ctx.textBaseline = 'middle';
    ctx.fillText(text, cx + padX, cy + h / 2 + 0.5);
    ctx.restore();
  }

  // Mirrors Rust `classify_blue_gesture`: monotonic diagonal drag -> 'box';
  // curving, reversing, or looping trajectory -> 'lasso'.
  function classifyBlueGesture(points) {
    if (!Array.isArray(points) || points.length < 3) return 'box';
    const start = points[0];
    const end = points[points.length - 1];
    const dxTotal = end.x - start.x;
    const dyTotal = end.y - start.y;
    const chord = Math.hypot(dxTotal, dyTotal);

    let arcLen = 0;
    let minX = start.x;
    let maxX = start.x;
    let minY = start.y;
    let maxY = start.y;
    for (let i = 1; i < points.length; i += 1) {
      const a = points[i - 1];
      const b = points[i];
      arcLen += Math.hypot(b.x - a.x, b.y - a.y);
      if (b.x < minX) minX = b.x;
      if (b.x > maxX) maxX = b.x;
      if (b.y < minY) minY = b.y;
      if (b.y > maxY) maxY = b.y;
    }

    if (arcLen < 36) return 'box';
    if (chord < 24) return 'lasso';

    const maxOvershoot = Math.max(
      maxX - Math.max(start.x, end.x),
      Math.min(start.x, end.x) - minX,
      maxY - Math.max(start.y, end.y),
      Math.min(start.y, end.y) - minY,
    );
    if (maxOvershoot > 26) return 'lasso';

    const straightness = chord / Math.max(arcLen, 1);
    if (straightness < 0.80) return 'lasso';

    let maxPerp = 0;
    for (let i = 1; i < points.length - 1; i += 1) {
      const p = points[i];
      const cross = Math.abs((p.x - start.x) * dyTotal - (p.y - start.y) * dxTotal);
      const perp = cross / chord;
      if (perp > maxPerp) maxPerp = perp;
    }
    if (maxPerp > 34 && maxPerp / chord > 0.24) return 'lasso';

    return 'box';
  }

  function syncCanvasSize() {
    const dpr = window.devicePixelRatio || 1;
    const w = window.innerWidth;
    const h = window.innerHeight;
    if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }

  function toCssPt(px, py) {
    const s = state.scale || window.devicePixelRatio || 1;
    return {
      x: (px - state.monX) / s,
      y: (py - state.monY) / s,
    };
  }

  function toCssRect(r) {
    if (!r) return null;
    const s = state.scale || window.devicePixelRatio || 1;
    return {
      x: (r.x - state.monX) / s,
      y: (r.y - state.monY) / s,
      w: r.w / s,
      h: r.h / s,
    };
  }

  function drawRoundedTag(x, y, text, isOk) {
    ctx.save();
    ctx.font = '600 12px system-ui, -apple-system, "Segoe UI", "PingFang SC", sans-serif';
    const padX = 10;
    const h = 24;
    const w = Math.ceil(ctx.measureText(text).width) + padX * 2;
    const clampedX = Math.max(8, Math.min(window.innerWidth - w - 8, x));
    const clampedY = Math.max(8, Math.min(window.innerHeight - h - 8, y));

    ctx.shadowColor = isOk ? 'rgba(0, 136, 255, 0.42)' : 'rgba(245, 158, 11, 0.36)';
    ctx.shadowBlur = 10;
    ctx.fillStyle = isOk ? 'rgba(15, 23, 42, 0.86)' : 'rgba(69, 26, 3, 0.88)';
    ctx.beginPath();
    ctx.roundRect(clampedX, clampedY, w, h, 8);
    ctx.fill();

    ctx.shadowBlur = 0;
    ctx.strokeStyle = isOk ? 'rgba(125, 211, 252, 0.68)' : 'rgba(251, 191, 36, 0.68)';
    ctx.lineWidth = 1;
    ctx.stroke();

    ctx.fillStyle = isOk ? '#f0f9ff' : '#fef3c7';
    ctx.textBaseline = 'middle';
    ctx.fillText(text, clampedX + padX, clampedY + h / 2 + 0.5);
    ctx.restore();
  }

  function drawNumberBadge(cx, cy, n, color) {
    ctx.save();
    ctx.shadowColor = color;
    ctx.shadowBlur = 8;
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.arc(cx, cy, 11, 0, Math.PI * 2);
    ctx.fill();
    ctx.shadowBlur = 0;
    ctx.fillStyle = '#ffffff';
    ctx.font = '700 12px system-ui, sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(String(n), cx, cy + 0.5);
    ctx.restore();
  }

  function drawDwellProgressRing(cssX, cssY, progress) {
    const r = 7;
    const cx = Math.max(r + 6, Math.min(window.innerWidth - r - 6, cssX + 18));
    const cy = Math.max(r + 6, Math.min(window.innerHeight - r - 6, cssY + 18));
    const clamped = Math.max(0.02, Math.min(1, progress));

    ctx.save();
    // Frosted dark disc backing for clear contrast over both light and dark wallpapers
    ctx.fillStyle = 'rgba(15, 23, 42, 0.62)';
    ctx.beginPath();
    ctx.arc(cx, cy, r + 2.5, 0, Math.PI * 2);
    ctx.fill();

    // Subtle track ring
    ctx.strokeStyle = 'rgba(226, 238, 255, 0.24)';
    ctx.lineWidth = 1.8;
    ctx.beginPath();
    ctx.arc(cx, cy, r, 0, Math.PI * 2);
    ctx.stroke();

    // Electric-blue glowing progress arc sweeping from 12 o'clock
    const startAngle = -Math.PI / 2;
    const endAngle = startAngle + clamped * Math.PI * 2;
    ctx.strokeStyle = 'rgba(175, 232, 255, 0.96)';
    ctx.shadowColor = 'rgba(0, 140, 255, 0.95)';
    ctx.shadowBlur = 7;
    ctx.lineWidth = 2.1;
    ctx.lineCap = 'round';
    ctx.beginPath();
    ctx.arc(cx, cy, r, startAngle, endAngle);
    ctx.stroke();
    ctx.restore();
  }

  function drawRestrainedLightTrail(now) {
    const cutoff = now - TRAIL_TTL_MS;
    while (state.trail.length > 0 && state.trail[0].ts < cutoff) {
      state.trail.shift();
    }
    if (state.trail.length < 2 || state.liveStroke.length > 0 || state.liveDrag) return;

    ctx.save();
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    ctx.shadowColor = 'rgba(0, 136, 255, 0.72)';
    ctx.shadowBlur = 6;

    for (let i = 1; i < state.trail.length; i += 1) {
      const a = toCssPt(state.trail[i - 1].x, state.trail[i - 1].y);
      const b = toCssPt(state.trail[i].x, state.trail[i].y);
      const life = Math.max(0, Math.min(1, (state.trail[i].ts - cutoff) / TRAIL_TTL_MS));
      const eased = life * life;
      ctx.strokeStyle = `rgba(110, 210, 255, ${(eased * 0.42).toFixed(3)})`;
      ctx.lineWidth = 0.6 + eased * 1.9;
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      ctx.stroke();
    }
    ctx.restore();
  }

  function drawHighlightFrame(r, label, isBackground, cursorPt) {
    // Inset slightly if the frame hugs screen edges (e.g. desktop / maximized window)
    const inset = isBackground ? 3 : 0;
    const fx = Math.max(inset, r.x);
    const fy = Math.max(inset, r.y);
    const fw = Math.min(window.innerWidth - inset, r.x + r.w) - fx;
    const fh = Math.min(window.innerHeight - inset, r.y + r.h) - fy;
    if (fw <= 2 || fh <= 2) return;

    ctx.save();
    ctx.fillStyle = isBackground ? 'rgba(56, 189, 248, 0.032)' : 'rgba(56, 189, 248, 0.05)';
    ctx.strokeStyle = 'rgba(235, 248, 255, 0.94)';
    ctx.lineWidth = isBackground ? 2.0 : 1.6;
    ctx.shadowColor = 'rgba(0, 136, 255, 0.88)';
    ctx.shadowBlur = isBackground ? 14 : 12;
    ctx.beginPath();
    ctx.roundRect(fx, fy, fw, fh, isBackground ? 10 : 6);
    ctx.fill();
    ctx.stroke();
    ctx.restore();

    if (label) {
      if (isBackground && fw > 460 && fh > 320 && cursorPt) {
        const c = toCssPt(cursorPt.x, cursorPt.y);
        drawRoundedTag(c.x + 18, c.y + 18, label, true);
      } else {
        const tagY = fy + fh + 8 + 24 > window.innerHeight ? fy - 30 : fy + fh + 8;
        drawRoundedTag(fx, tagY, label, true);
      }
    }
  }

  function redraw(now = performance.now()) {
    syncCanvasSize();
    ctx.clearRect(0, 0, window.innerWidth, window.innerHeight);
    if (!state.active) return;

    // 0. Green-Brush Deep Scan Inspection View (yellow water + emerged crystal-clear blue buttons)
    drawScanInspectionLayer(now);

    // 1. Picked items during current selection session
    for (const item of state.pickedItems) {
      const isBlueStroke =
        item.color === 'blue' ||
        item.mode === 'blue_lasso' ||
        item.mode === 'blue_box' ||
        item.mode === 'blue_text';

      if (isBlueStroke) {
        // Highlight every selected foreground element in the group with crisp blue frames
        if (Array.isArray(item.itemRects) && item.itemRects.length > 0) {
          ctx.save();
          ctx.fillStyle = 'rgba(56, 189, 248, 0.11)';
          ctx.strokeStyle = 'rgba(125, 211, 252, 0.92)';
          ctx.shadowColor = 'rgba(0, 136, 255, 0.72)';
          ctx.shadowBlur = 8;
          ctx.lineWidth = 1.6;
          for (const rawIr of item.itemRects) {
            const ir = toCssRect(rawIr);
            if (!ir || ir.w <= 2 || ir.h <= 2) continue;
            ctx.beginPath();
            ctx.roundRect(ir.x, ir.y, ir.w, ir.h, 5);
            ctx.fill();
            ctx.stroke();
          }
          ctx.restore();
        }

        if (item.mode === 'blue_lasso' && item.strokes && item.strokes.length > 1) {
          ctx.save();
          ctx.strokeStyle = '#38bdf8';
          ctx.shadowColor = 'rgba(0, 136, 255, 0.78)';
          ctx.shadowBlur = 9;
          ctx.lineWidth = 2.4;
          ctx.lineCap = 'round';
          ctx.lineJoin = 'round';
          ctx.beginPath();
          const p0 = toCssPt(item.strokes[0].x, item.strokes[0].y);
          ctx.moveTo(p0.x, p0.y);
          for (let i = 1; i < item.strokes.length; i += 1) {
            const p = toCssPt(item.strokes[i].x, item.strokes[i].y);
            ctx.lineTo(p.x, p.y);
          }
          ctx.stroke();
          ctx.restore();
          drawNumberBadge(p0.x + 12, Math.max(14, p0.y - 12), item.n, '#0084ff');
        } else if (item.rect) {
          const r = toCssRect(item.rect);
          ctx.save();
          ctx.fillStyle = 'rgba(56, 189, 248, 0.08)';
          ctx.strokeStyle = '#38bdf8';
          ctx.shadowColor = 'rgba(0, 136, 255, 0.78)';
          ctx.shadowBlur = 10;
          ctx.lineWidth = 1.8;
          ctx.beginPath();
          ctx.roundRect(r.x, r.y, Math.max(4, r.w), Math.max(4, r.h), 6);
          ctx.fill();
          ctx.stroke();
          ctx.restore();
          drawNumberBadge(Math.min(window.innerWidth - 14, r.x + r.w + 8), Math.max(14, r.y - 6), item.n, '#0084ff');
        }
      } else if (item.strokes && item.strokes.length > 1) {
        ctx.save();
        ctx.strokeStyle = '#ef4444';
        ctx.shadowColor = 'rgba(239, 68, 68, 0.55)';
        ctx.shadowBlur = 8;
        ctx.lineWidth = 2.6;
        ctx.lineCap = 'round';
        ctx.lineJoin = 'round';
        ctx.beginPath();
        const p0 = toCssPt(item.strokes[0].x, item.strokes[0].y);
        ctx.moveTo(p0.x, p0.y);
        for (let i = 1; i < item.strokes.length; i += 1) {
          const p = toCssPt(item.strokes[i].x, item.strokes[i].y);
          ctx.lineTo(p.x, p.y);
        }
        ctx.stroke();
        ctx.restore();
        drawNumberBadge(p0.x + 12, Math.max(14, p0.y - 12), item.n, '#ef4444');
      } else if (item.rect) {
        const r = toCssRect(item.rect);
        const fx = Math.max(3, r.x);
        const fy = Math.max(3, r.y);
        const fw = Math.min(window.innerWidth - 3, r.x + r.w) - fx;
        const fh = Math.min(window.innerHeight - 3, r.y + r.h) - fy;
        if (fw <= 2 || fh <= 2) continue;
        const isExcel = item.mode === 'excel_box' || item.mode === 'excel_cell';
        const isWord = item.mode === 'word_text';
        const strokeColor = isExcel ? '#10b981' : isWord ? '#38bdf8' : '#ffffff';
        const glowColor = isExcel
          ? 'rgba(16, 185, 129, 0.72)'
          : isWord
          ? 'rgba(56, 189, 248, 0.72)'
          : 'rgba(0, 140, 255, 0.85)';
        const badgeColor = isExcel ? '#10b981' : isWord ? '#0284c7' : '#0084ff';
        ctx.save();
        if (isExcel) {
          ctx.fillStyle = 'rgba(16, 185, 129, 0.10)';
        } else if (isWord) {
          ctx.fillStyle = 'rgba(56, 189, 248, 0.12)';
        }
        ctx.strokeStyle = strokeColor;
        ctx.shadowColor = glowColor;
        ctx.shadowBlur = 12;
        ctx.lineWidth = isExcel ? 2.0 : 1.8;
        ctx.beginPath();
        ctx.roundRect(fx, fy, fw, fh, isExcel ? 3 : 6);
        if (isExcel || isWord) ctx.fill();
        ctx.stroke();
        ctx.restore();
        drawNumberBadge(Math.min(window.innerWidth - 14, fx + fw + 8), Math.max(14, fy - 6), item.n, badgeColor);
      }
    }

    // 2. Hover feedback: immediate for foreground elements; 0.5s dwell ring -> highlight for backgrounds/desktop
    if (state.liveStroke.length === 0 && !state.liveDrag && state.hover) {
      const kind = state.hover.kind || (state.hover.ok ? 'element' : 'none');
      if (kind === 'element' && state.hover.rect) {
        drawHighlightFrame(
          toCssRect(state.hover.rect),
          state.hover.label || '可抓取元素',
          false,
          state.cursor || state.hover.cursor
        );
      } else if (kind === 'background' && state.hover.rect && state.dwell) {
        const elapsed = now - state.dwell.startTs;
        if (!state.dwell.unlocked && elapsed >= BG_DWELL_MS) {
          state.dwell.unlocked = true;
        }
        const curPt = state.cursor || state.hover.cursor;
        if (state.dwell.unlocked) {
          drawHighlightFrame(
            toCssRect(state.hover.rect),
            state.hover.label || '背景',
            true,
            curPt
          );
        } else if (elapsed >= BG_RING_SHOW_AFTER_MS && curPt) {
          const c = toCssPt(curPt.x, curPt.y);
          const progress = (elapsed - BG_RING_SHOW_AFTER_MS) / (BG_DWELL_MS - BG_RING_SHOW_AFTER_MS);
          drawDwellProgressRing(c.x, c.y, progress);
        }
      }
    }

    // 3. Live drag feedback:
    //    - word_text: subtle sky-blue text-selection band (NO red line)
    //    - excel_box: crisp emerald rectangular cell marquee (NO red line)
    //    - stroke (default + PPT): classic red freehand line
    if (state.liveDrag && state.liveDrag.start && state.liveDrag.end) {
      const a = toCssPt(state.liveDrag.start.x, state.liveDrag.start.y);
      const b = toCssPt(state.liveDrag.end.x, state.liveDrag.end.y);
      const rx = Math.min(a.x, b.x);
      const ry = Math.min(a.y, b.y);
      const rw = Math.max(4, Math.abs(b.x - a.x));
      const rh = Math.max(16, Math.abs(b.y - a.y));

      ctx.save();
      if (state.liveDrag.mode === 'excel_box') {
        ctx.fillStyle = 'rgba(16, 185, 129, 0.12)';
        ctx.strokeStyle = '#10b981';
        ctx.shadowColor = 'rgba(16, 185, 129, 0.55)';
        ctx.shadowBlur = 10;
        ctx.lineWidth = 2.0;
        ctx.beginPath();
        ctx.roundRect(rx, ry, rw, rh, 3);
        ctx.fill();
        ctx.stroke();
        ctx.restore();
        drawRoundedTag(b.x + 14, b.y + 14, '框选单元格', true);
      } else if (state.liveDrag.mode === 'word_text') {
        ctx.fillStyle = 'rgba(56, 189, 248, 0.13)';
        ctx.strokeStyle = 'rgba(56, 189, 248, 0.65)';
        ctx.setLineDash([4, 3]);
        ctx.lineWidth = 1.2;
        ctx.beginPath();
        ctx.roundRect(rx, ry, rw, rh, 3);
        ctx.fill();
        ctx.stroke();
        ctx.restore();
        drawRoundedTag(b.x + 14, b.y + 14, '光标选字', true);
      } else {
        ctx.restore();
      }
    } else if (state.liveStroke.length > 1) {
      if (state.liveStrokeColor === 'red') {
        // Right-button RED freehand doodle (produces annotated + clean dual screenshot capsule)
        ctx.save();
        ctx.strokeStyle = '#ef4444';
        ctx.shadowColor = 'rgba(239, 68, 68, 0.65)';
        ctx.shadowBlur = 8;
        ctx.lineWidth = 2.8;
        ctx.lineCap = 'round';
        ctx.lineJoin = 'round';
        ctx.beginPath();
        const first = toCssPt(state.liveStroke[0].x, state.liveStroke[0].y);
        ctx.moveTo(first.x, first.y);
        for (let i = 1; i < state.liveStroke.length; i += 1) {
          const pt = toCssPt(state.liveStroke[i].x, state.liveStroke[i].y);
          ctx.lineTo(pt.x, pt.y);
        }
        ctx.stroke();
        ctx.restore();
        const last = toCssPt(
          state.liveStroke[state.liveStroke.length - 1].x,
          state.liveStroke[state.liveStroke.length - 1].y,
        );
        drawRoundedTag(last.x + 14, last.y + 14, '红线涂鸦截图', false);
      } else {
        // Left-button BLUE CAD Auto-Switch: monotonic diagonal -> Box; curving/reversing -> seamless 160ms morph to Freehand Lasso
        const first = toCssPt(state.liveStroke[0].x, state.liveStroke[0].y);
        const last = toCssPt(
          state.liveStroke[state.liveStroke.length - 1].x,
          state.liveStroke[state.liveStroke.length - 1].y,
        );
        if (!state.blueMorph.lockedLasso) {
          const rx = Math.min(first.x, last.x);
          const ry = Math.min(first.y, last.y);
          const rw = Math.max(4, Math.abs(last.x - first.x));
          const rh = Math.max(4, Math.abs(last.y - first.y));
          state.blueMorph.lastBox = { x: rx, y: ry, w: rw, h: rh };

          ctx.save();
          ctx.fillStyle = 'rgba(56, 189, 248, 0.10)';
          ctx.strokeStyle = '#38bdf8';
          ctx.shadowColor = 'rgba(0, 136, 255, 0.72)';
          ctx.shadowBlur = 10;
          ctx.lineWidth = 1.8;
          ctx.beginPath();
          ctx.roundRect(rx, ry, rw, rh, 5);
          ctx.fill();
          ctx.stroke();
          ctx.restore();
          drawRoundedTag(last.x + 14, last.y + 14, '蓝线框选 / 划字', true);
        } else {
          const MORPH_MS = 160;
          const t = Math.max(0, Math.min(1, (now - state.blueMorph.morphStartTs) / MORPH_MS));
          const eased = 1 - Math.pow(1 - t, 3);

          // 1) Fade out the previous blue selection box seamlessly during the first 160ms
          if (t < 1 && state.blueMorph.lastBox) {
            const lb = state.blueMorph.lastBox;
            const expand = eased * 4;
            ctx.save();
            ctx.globalAlpha = 1 - eased;
            ctx.fillStyle = 'rgba(56, 189, 248, 0.09)';
            ctx.strokeStyle = '#38bdf8';
            ctx.setLineDash([5, 4]);
            ctx.lineWidth = 1.5;
            ctx.beginPath();
            ctx.roundRect(lb.x - expand, lb.y - expand, lb.w + expand * 2, lb.h + expand * 2, 8);
            ctx.fill();
            ctx.stroke();
            ctx.restore();
          }

          // 2) Bloom in the glowing blue freehand lasso with soft translucent enclosed fill
          ctx.save();
          ctx.globalAlpha = Math.max(0.3, eased);
          ctx.beginPath();
          ctx.moveTo(first.x, first.y);
          for (let i = 1; i < state.liveStroke.length; i += 1) {
            const pt = toCssPt(state.liveStroke[i].x, state.liveStroke[i].y);
            ctx.lineTo(pt.x, pt.y);
          }
          if (state.liveStroke.length >= 6) {
            ctx.save();
            ctx.closePath();
            ctx.fillStyle = 'rgba(56, 189, 248, 0.085)';
            ctx.fill();
            ctx.restore();
          }
          ctx.strokeStyle = '#38bdf8';
          ctx.shadowColor = 'rgba(0, 136, 255, 0.85)';
          ctx.shadowBlur = 9;
          ctx.lineWidth = 2.6 + (1 - eased) * 1.1;
          ctx.lineCap = 'round';
          ctx.lineJoin = 'round';
          ctx.stroke();

          // Subtle dashed closing chord so the user sees the enclosed lasso region
          if (state.liveStroke.length >= 6) {
            ctx.shadowBlur = 0;
            ctx.strokeStyle = 'rgba(125, 211, 252, 0.42)';
            ctx.lineWidth = 1.2;
            ctx.setLineDash([4, 4]);
            ctx.beginPath();
            ctx.moveTo(last.x, last.y);
            ctx.lineTo(first.x, first.y);
            ctx.stroke();
          }
          ctx.restore();
          drawRoundedTag(last.x + 14, last.y + 14, '蓝线圈选 · 元素组', true);
        }
      }
    }

    // 4. Short, restrained electric-blue light trail
    drawRestrainedLightTrail(now);

    // 5. Subtle Shake-to-Wake cursor pulse (Dose 1 amber / Dose 2 violet-cyan)
    if (state.scoutPulse) {
      const elapsed = now - state.scoutPulse.startTs;
      const dur = 420;
      if (elapsed >= dur) {
        state.scoutPulse = null;
      } else {
        const t = Math.max(0, Math.min(1, elapsed / dur));
        const ease = 1 - Math.pow(1 - t, 3);
        const c = toCssPt(state.scoutPulse.x, state.scoutPulse.y);
        const isDose2 = state.scoutPulse.tier >= 2;
        const color = isDose2 ? '168, 85, 247' : '245, 158, 11';
        const r = 12 + ease * (isDose2 ? 44 : 32);
        ctx.save();
        ctx.strokeStyle = `rgba(${color}, ${((1 - t) * 0.85).toFixed(3)})`;
        ctx.shadowColor = `rgba(${color}, 0.72)`;
        ctx.shadowBlur = 10;
        ctx.lineWidth = Math.max(1.0, 2.4 * (1 - t));
        ctx.beginPath();
        ctx.arc(c.x, c.y, r, 0, Math.PI * 2);
        ctx.stroke();
        ctx.restore();
      }
    }
  }

  function needsAnimationLoop() {
    if (!state.active) return false;
    if (state.scoutPulse && performance.now() - state.scoutPulse.startTs < 440) return true;
    if (state.scan.active) {
      const now = performance.now();
      for (const item of state.scan.revealed) {
        if (now - item.revealedAt < 480) return true;
      }
    }
    if (state.trail.length > 0) return true;
    if (
      state.liveStroke.length > 1 &&
      state.liveStrokeColor === 'blue' &&
      state.blueMorph.lockedLasso &&
      performance.now() - state.blueMorph.morphStartTs < 170
    ) {
      return true;
    }
    if (state.dwell && !state.dwell.unlocked && state.liveStroke.length === 0 && !state.liveDrag) return true;
    return false;
  }

  function scheduleAnimTick() {
    if (state.rafId || !state.active) return;
    const tick = (now) => {
      state.rafId = 0;
      redraw(now);
      if (needsAnimationLoop()) {
        state.rafId = window.requestAnimationFrame(tick);
      }
    };
    state.rafId = window.requestAnimationFrame(tick);
  }

  function stopAnimLoop() {
    if (state.rafId) {
      window.cancelAnimationFrame(state.rafId);
      state.rafId = 0;
    }
  }

  function updateCursor(x, y) {
    if (typeof x !== 'number' || typeof y !== 'number') return;
    const now = performance.now();
    state.cursor = { x, y };
    positionStatusPill(state.cursor);
    state.trail.push({ x, y, ts: now });
    if (state.trail.length > MAX_TRAIL_POINTS) {
      state.trail.shift();
    }
    if (state.dwell && !state.dwell.unlocked) {
      const dx = x - state.dwell.originX;
      const dy = y - state.dwell.originY;
      if (dx * dx + dy * dy > BG_DWELL_JITTER_SQ) {
        state.dwell.originX = x;
        state.dwell.originY = y;
        state.dwell.startTs = now;
      }
    }
    scheduleAnimTick();
  }

  let statusPillEl = null;
  let statusPillTimer = null;

  function ensureStatusPillEl() {
    if (statusPillEl) return statusPillEl;
    if (typeof document === 'undefined' || !document.body) return null;
    const el = document.createElement('div');
    el.id = 'atp-cursor-status-pill';
    el.style.position = 'fixed';
    el.style.zIndex = '999999';
    el.style.pointerEvents = 'none';
    el.style.display = 'none';
    el.style.padding = '5px 10px';
    el.style.borderRadius = '999px';
    el.style.fontSize = '12px';
    el.style.fontWeight = '500';
    el.style.fontFamily = '"Segoe UI", "Microsoft YaHei UI", system-ui, sans-serif';
    el.style.color = '#f8fafc';
    el.style.background = 'rgba(15, 23, 42, 0.86)';
    el.style.border = '1px solid rgba(56, 189, 248, 0.45)';
    el.style.boxShadow = '0 4px 16px rgba(0, 0, 0, 0.35)';
    el.style.backdropFilter = 'blur(8px)';
    el.style.whiteSpace = 'nowrap';
    el.style.transition = 'opacity 140ms ease';
    document.body.appendChild(el);
    statusPillEl = el;
    return el;
  }

  function positionStatusPill(cursorPt) {
    if (!statusPillEl || statusPillEl.style.display === 'none') return;
    const pt = cursorPt || state.cursor;
    if (!pt || typeof pt.x !== 'number' || typeof pt.y !== 'number') return;
    const css = toCssPt(pt.x, pt.y);
    const vw = window.innerWidth || 1920;
    const vh = window.innerHeight || 1080;
    const w = statusPillEl.offsetWidth || 170;
    const h = statusPillEl.offsetHeight || 28;
    const left = Math.max(10, Math.min(vw - w - 12, css.x + 18));
    const top = Math.max(10, Math.min(vh - h - 12, css.y + 20));
    statusPillEl.style.left = Math.round(left) + 'px';
    statusPillEl.style.top = Math.round(top) + 'px';
  }

  function showCursorStatusPill(text, cursorPt, phase, durationMs = 2000) {
    const el = ensureStatusPillEl();
    if (!el) return;
    const prefix = phase === 'done' ? '✓ ' : '✦ ';
    el.textContent = prefix + String(text || '');
    el.style.borderColor =
      phase === 'done' ? 'rgba(52, 211, 153, 0.55)' : 'rgba(56, 189, 248, 0.5)';
    el.style.display = 'block';
    el.style.opacity = '1';
    positionStatusPill(cursorPt);
    if (statusPillTimer) clearTimeout(statusPillTimer);
    statusPillTimer = setTimeout(() => {
      if (statusPillEl) {
        statusPillEl.style.opacity = '0';
        setTimeout(() => {
          if (statusPillEl && statusPillEl.style.opacity === '0') {
            statusPillEl.style.display = 'none';
          }
        }, 160);
      }
    }, Math.max(400, Number(durationMs) || 2000));
  }

  function hideCursorStatusPill() {
    if (statusPillTimer) {
      clearTimeout(statusPillTimer);
      statusPillTimer = null;
    }
    if (statusPillEl) {
      statusPillEl.style.display = 'none';
      statusPillEl.style.opacity = '0';
    }
  }

  function showToast(text, durationMs = 1500) {
    if (!toastEl) return;
    toastEl.textContent = text;
    toastEl.style.display = 'block';
    if (state.toastTimer) clearTimeout(state.toastTimer);
    state.toastTimer = setTimeout(() => {
      toastEl.style.display = 'none';
    }, durationMs);
  }

  function composeScreenshot(payload) {
    const tauriCore = window.__TAURI__ && window.__TAURI__.core;
    if (!tauriCore || !payload || !payload.id) return;
    if (!payload.pngBase64) {
      tauriCore.invoke('overlay_compose_done', { id: payload.id, dataUrl: '' }).catch(() => {});
      return;
    }
    const img = new Image();
    img.onload = () => {
      const off = document.createElement('canvas');
      off.width = img.width;
      off.height = img.height;
      const octx = off.getContext('2d');
      octx.drawImage(img, 0, 0);

      const ox = payload.originX || 0;
      const oy = payload.originY || 0;
      let badgeX = 24;
      let badgeY = 24;

      // If Green-Brush Scan Inspection View is active (or just finished within 3s before pen:hide),
      // composite the yellow-water mask + emerged blue boxes onto the annotated screenshot so
      // right-drag red doodle screenshots faithfully show the blue scan frames (while clean screenshot stays untouched!).
      const scanSnap = state.scan.active
        ? {
            monX: state.monX,
            monY: state.monY,
            scale: state.scale || window.devicePixelRatio || 1,
            revealed: state.scan.revealed,
          }
        : lastScanSnapshotForCompose && performance.now() - lastScanSnapshotForCompose.ts < 3000
          ? lastScanSnapshotForCompose
          : null;
      if (scanSnap) {
        if (Array.isArray(scanSnap.revealed) && scanSnap.revealed.length > 0) {
          octx.save();
          octx.lineWidth = 2;
          for (const item of scanSnap.revealed) {
            if (!item || !item.rect) continue;
            if (Array.isArray(item.polygon) && item.polygon.length >= 3) {
              octx.fillStyle = 'rgba(224, 242, 254, 0.12)';
              octx.strokeStyle = '#22d3ee';
              octx.lineWidth = 2.4;
              octx.beginPath();
              octx.moveTo(item.polygon[0][0] - ox, item.polygon[0][1] - oy);
              for (let i = 1; i < item.polygon.length; i += 1) {
                octx.lineTo(item.polygon[i][0] - ox, item.polygon[i][1] - oy);
              }
              octx.closePath();
              octx.fill();
              octx.stroke();
              continue;
            }
            const bx = item.rect.x - ox;
            const by = item.rect.y - oy;
            const bw = item.rect.w;
            const bh = item.rect.h;
            if (bw <= 2 || bh <= 2) continue;
            octx.fillStyle = 'rgba(255, 255, 255, 0.08)';
            octx.strokeStyle =
              item.semanticKind === 'ui_vlm_fixed'
                ? '#818cf8'
                : item.isExpandable
                  ? '#60a5fa'
                  : '#38bdf8';
            octx.lineWidth = 2;
            octx.beginPath();
            octx.roundRect(bx, by, bw, bh, 6);
            octx.fill();
            octx.stroke();
          }
          octx.restore();
        }
      }

      octx.strokeStyle = '#ef4444';
      octx.lineWidth = 3;
      octx.lineCap = 'round';
      octx.lineJoin = 'round';

      if (Array.isArray(payload.strokes) && payload.strokes.length > 1) {
        octx.beginPath();
        octx.moveTo(payload.strokes[0].x - ox, payload.strokes[0].y - oy);
        for (let i = 1; i < payload.strokes.length; i += 1) {
          octx.lineTo(payload.strokes[i].x - ox, payload.strokes[i].y - oy);
        }
        octx.stroke();
        badgeX = Math.max(16, Math.min(off.width - 16, payload.strokes[0].x - ox + 14));
        badgeY = Math.max(16, Math.min(off.height - 16, payload.strokes[0].y - oy - 14));
      } else if (payload.rect) {
        const rx = payload.rect.x - ox;
        const ry = payload.rect.y - oy;
        octx.strokeRect(rx, ry, payload.rect.w, payload.rect.h);
        badgeX = Math.max(16, Math.min(off.width - 16, rx + payload.rect.w + 10));
        badgeY = Math.max(16, Math.min(off.height - 16, ry - 8));
      }

      if (payload.n) {
        octx.fillStyle = '#ef4444';
        octx.beginPath();
        octx.arc(badgeX, badgeY, 13, 0, Math.PI * 2);
        octx.fill();
        octx.fillStyle = '#ffffff';
        octx.font = '700 14px system-ui, sans-serif';
        octx.textAlign = 'center';
        octx.textBaseline = 'middle';
        octx.fillText(String(payload.n), badgeX, badgeY + 0.5);
      }

      const dataUrl = off.toDataURL('image/png');
      tauriCore.invoke('overlay_compose_done', { id: payload.id, dataUrl }).catch(() => {});
    };
    img.onerror = () => {
      tauriCore.invoke('overlay_compose_done', { id: payload.id, dataUrl: '' }).catch(() => {});
    };
    img.src = 'data:image/png;base64,' + payload.pngBase64;
  }

  window.addEventListener('resize', () => redraw());

  function resetSessionState() {
    stopAnimLoop();
    clearScanLayer();
    if (state.toastTimer) {
      clearTimeout(state.toastTimer);
      state.toastTimer = null;
    }
    state.cursor = null;
    state.hover = null;
    state.dwell = null;
    state.trail = [];
    state.liveStroke = [];
    state.liveStrokeColor = 'blue';
    state.blueMorph = { lockedLasso: false, morphStartTs: 0, lastBox: null };
    state.liveDrag = null;
    state.pickedItems = [];
    state.scoutPulse = null;
  }

  function requestDraw() {
    if (!state.rafId) {
      scheduleAnimTick();
    }
  }

  function bindEvents() {
    const evApi = window.__TAURI__ && window.__TAURI__.event;
    if (!evApi || typeof evApi.listen !== 'function') {
      setTimeout(bindEvents, 80);
      return;
    }

    evApi.listen('pen:show', (ev) => {
      const p = ev.payload || {};
      const mr = p.monitorRect || {};
      resetSessionState();
      state.active = true;
      state.monX = mr.x || 0;
      state.monY = mr.y || 0;
      state.scale = p.scale || window.devicePixelRatio || 1;
      state.cursor = p.cursor && typeof p.cursor.x === 'number' ? p.cursor : null;
      redraw();
    });

    evApi.listen('pen:cursor', (ev) => {
      const p = ev.payload || {};
      updateCursor(p.x, p.y);
    });

    evApi.listen('pen:hover', (ev) => {
      const h = ev.payload || null;
      state.hover = h;
      if (h && h.cursor && typeof h.cursor.x === 'number') {
        state.cursor = h.cursor;
      }

      if (h && h.kind === 'background' && h.targetKey) {
        const cur = state.cursor || h.cursor || { x: 0, y: 0 };
        if (!state.dwell || state.dwell.key !== h.targetKey) {
          state.dwell = {
            key: h.targetKey,
            originX: cur.x,
            originY: cur.y,
            startTs: performance.now(),
            unlocked: false,
          };
        }
      } else {
        state.dwell = null;
      }
      requestDraw();
    });

    evApi.listen('pen:stroke', (ev) => {
      const p = ev.payload || {};
      if (p.mode === 'word_text' || p.mode === 'excel_box') {
        state.liveStroke = [];
        state.liveDrag = { mode: p.mode, start: p.start, end: p.end };
      } else {
        state.liveDrag = null;
        const pts = Array.isArray(p.points) ? p.points : [];
        const nextColor = p.color === 'red' || p.mode === 'red_doodle' ? 'red' : 'blue';
        const isFresh =
          pts.length <= 2 ||
          state.liveStroke.length === 0 ||
          state.liveStrokeColor !== nextColor ||
          pts.length < state.liveStroke.length;
        if (isFresh) {
          state.blueMorph = { lockedLasso: false, morphStartTs: 0, lastBox: null };
        }
        state.liveStrokeColor = nextColor;
        state.liveStroke = pts;
        if (nextColor === 'blue' && !state.blueMorph.lockedLasso && classifyBlueGesture(pts) === 'lasso') {
          state.blueMorph.lockedLasso = true;
          state.blueMorph.morphStartTs = performance.now();
        }
      }
      state.trail = [];
      state.dwell = null;
      requestDraw();
    });

    evApi.listen('pen:picked', (ev) => {
      const p = ev.payload || {};
      state.liveStroke = [];
      state.blueMorph = { lockedLasso: false, morphStartTs: 0, lastBox: null };
      state.liveDrag = null;
      state.pickedItems.push(p);
      redraw();
    });

    evApi.listen('pen:clear_stroke', () => {
      state.liveStroke = [];
      state.blueMorph = { lockedLasso: false, morphStartTs: 0, lastBox: null };
      state.liveDrag = null;
      redraw();
    });

    evApi.listen('pen:toast', (ev) => {
      const p = ev.payload || {};
      if (p.text) showToast(p.text, p.durationMs || 1500);
    });

    evApi.listen('pen:status_hint', (ev) => {
      const p = ev.payload || {};
      if (p.text) {
        showCursorStatusPill(p.text, p.cursor, p.phase || 'working', p.durationMs || 2000);
      }
    });

    evApi.listen('pen:scout_pulse', (ev) => {
      const p = ev.payload || {};
      if (typeof p.x === 'number' && typeof p.y === 'number') {
        state.scoutPulse = {
          tier: Number(p.tier) || 1,
          x: p.x,
          y: p.y,
          startTs: performance.now(),
        };
        scheduleAnimTick();
      }
    });

    evApi.listen('pen:scan_clear', () => {
      clearScanLayer();
      redraw();
    });

    evApi.listen('pen:scan_reveal', (ev) => {
      const p = ev.payload || {};
      const items = Array.isArray(p.items) ? p.items : [];
      const replaceKeys = Array.isArray(p.replaceKeys) ? p.replaceKeys : [];
      if (items.length === 0 && replaceKeys.length === 0) return;
      state.scan.active = true;
      let replacedAny = false;
      if (replaceKeys.length > 0) {
        const dropSet = new Set(replaceKeys.map(String));
        const beforeLen = state.scan.revealed.length;
        state.scan.revealed = state.scan.revealed.filter((ex) => {
          if (ex && dropSet.has(ex.key)) {
            state.scan.revealedSet.delete(ex.key);
            return false;
          }
          return true;
        });
        replacedAny = state.scan.revealed.length !== beforeLen;
      }
      const baseNow = performance.now();
      let added = 0;
      const overlapsExisting = (nr) => {
        for (const ex of state.scan.revealed) {
          const r = ex && ex.rect;
          if (!r) continue;
          const ix0 = Math.max(nr.x, r.x);
          const iy0 = Math.max(nr.y, r.y);
          const ix1 = Math.min(nr.x + nr.w, r.x + r.w);
          const iy1 = Math.min(nr.y + nr.h, r.y + r.h);
          const iw = Math.max(0, ix1 - ix0);
          const ih = Math.max(0, iy1 - iy0);
          if (iw > 0 && ih > 0) {
            const inter = iw * ih;
            const minArea = Math.max(1, Math.min(nr.w * nr.h, r.w * r.h));
            if (inter / minArea >= 0.38) return true;
          }
        }
        return false;
      };
      for (const it of items) {
        if (!it || !it.rect) continue;
        const k = String(it.key || `${it.rect.x}:${it.rect.y}:${it.rect.w}:${it.rect.h}`);
        const isPhoto = it.semanticKind === 'photo_object';
        // CRITICAL: Never drop replacement items via overlapsExisting after deleting replaceKeys!
        if (state.scan.revealedSet.has(k) || (!isPhoto && !replacedAny && overlapsExisting(it.rect))) continue;
        state.scan.revealedSet.add(k);
        // Stagger emergence slightly (20ms each, capped at 320ms) so buttons visibly rise one by one!
        const delay = replacedAny ? 0 : Math.min(320, added * 20);
        state.scan.revealed.push({
          key: k,
          rect: it.rect,
          polygon: Array.isArray(it.polygon) && it.polygon.length >= 3 ? it.polygon : null,
          semanticKind: it.semanticKind || 'ui',
          label: it.label || '',
          isExpandable: Boolean(it.isExpandable),
          revealedAt: baseNow + delay,
        });
        added += 1;
      }
      if (added > 0 || replacedAny) scheduleAnimTick();
    });

    evApi.listen('pen:compose', (ev) => {
      composeScreenshot(ev.payload);
    });

    evApi.listen('pen:hide', () => {
      state.active = false;
      resetSessionState();
      if (toastEl) toastEl.style.display = 'none';
      hideCursorStatusPill();
      redraw();
    });
  }

  bindEvents();
})();
