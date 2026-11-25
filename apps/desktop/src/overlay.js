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
  };

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
  }

  function needsAnimationLoop() {
    if (!state.active) return false;
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
    if (!tauriCore || !payload || !payload.id || !payload.pngBase64) return;
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

  function bindEvents() {
    const evApi = window.__TAURI__ && window.__TAURI__.event;
    if (!evApi || typeof evApi.listen !== 'function') {
      setTimeout(bindEvents, 80);
      return;
    }

    evApi.listen('pen:show', (ev) => {
      const p = ev.payload || {};
      const mr = p.monitorRect || {};
      state.active = true;
      state.monX = mr.x || 0;
      state.monY = mr.y || 0;
      state.scale = p.scale || window.devicePixelRatio || 1;
      state.cursor = p.cursor && typeof p.cursor.x === 'number' ? p.cursor : null;
      state.hover = null;
      state.dwell = null;
      state.liveStroke = [];
      state.liveDrag = null;
      state.pickedItems = [];
      state.trail = [];
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
          scheduleAnimTick();
        }
      } else {
        state.dwell = null;
      }
      redraw();
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
          scheduleAnimTick();
        }
      }
      state.trail = [];
      state.dwell = null;
      redraw();
    });

    evApi.listen('pen:picked', (ev) => {
      const p = ev.payload || {};
      state.liveStroke = [];
      state.blueMorph = { lockedLasso: false, morphStartTs: 0, lastBox: null };
      state.liveDrag = null;
      state.pickedItems.push(p);
      redraw();
    });

    evApi.listen('pen:toast', (ev) => {
      const p = ev.payload || {};
      if (p.text) showToast(p.text, p.durationMs || 1500);
    });

    evApi.listen('pen:compose', (ev) => {
      composeScreenshot(ev.payload);
    });

    evApi.listen('pen:hide', () => {
      state.active = false;
      state.cursor = null;
      state.hover = null;
      state.dwell = null;
      state.trail = [];
      state.liveStroke = [];
      state.liveDrag = null;
      state.pickedItems = [];
      stopAnimLoop();
      if (toastEl) toastEl.style.display = 'none';
      redraw();
    });
  }

  bindEvents();
})();
