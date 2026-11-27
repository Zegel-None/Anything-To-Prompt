function normalizeDockEdge(raw) {
  const s = String(raw || '').trim().toLowerCase();
  if (s === 'top' || s === 'bottom' || s === 'left' || s === 'right') return s;
  return 'none';
}

function syncPanelWidthVar() {
  if (!IS_BAR || typeof document === 'undefined' || !document.documentElement) return;
  const edge = dsbRuntime.dockEdge || 'none';
  const isVert = edge === 'left' || edge === 'right' || Boolean(dsbRuntime.verticalMode);
  const w = isVert
    ? Math.max(360, Math.min(960, Math.round(dsbRuntime.sideWidth || DEFAULT_SIDE_WIDTH_PX)))
    : Math.max(480, Math.min(960, Math.round(dsbRuntime.barWidth || 760)));
  document.documentElement.style.setProperty('--dsb-panel-w', w + 'px');
}

let morphClearTimer = null;
let collapseNotifyTimer = null;
let collapseRafId = 0;
let activeMorphAnim = null;
function triggerOrientationMorph(targetVertical) {
  // Constant 960dip stage never resizes the WebView2 HWND, so toggle immediately and let
  // CSS transition `--dsb-panel-w` + pre-mounted `[data-dsb-card]` simultaneously in 1 fluid motion!
  tauriInvoke('bar_set_vertical_mode', { vertical: Boolean(targetVertical) }).catch(() => {});
}

function applyDockState(edge, collapsed, vertical) {
  const norm = normalizeDockEdge(edge);
  const isCollapsed = norm !== 'none' && Boolean(collapsed);
  // Strict edge-orientation invariant:
  // - Left / Right dock -> ALWAYS Vertical
  // - Top / Bottom dock -> ALWAYS Horizontal
  // - Free floating ('none') -> Either Horizontal or Vertical!
  const isVert =
    norm === 'left' || norm === 'right'
      ? true
      : norm === 'top' || norm === 'bottom'
        ? false
        : typeof vertical === 'boolean'
          ? vertical
          : Boolean(dsbRuntime.verticalMode);
  const prevEdge = dsbRuntime.dockEdge;
  const prevCollapsed = dsbRuntime.dockCollapsed;
  const prevVert = Boolean(dsbRuntime.verticalMode);
  dsbRuntime.dockEdge = norm;
  dsbRuntime.dockCollapsed = isCollapsed;
  dsbRuntime.verticalMode = isVert;

  const onlyCollapseChanged = prevEdge === norm && prevVert === isVert && prevCollapsed !== isCollapsed;
  const rootEl = typeof document !== 'undefined' ? document.documentElement : null;

  if (collapseRafId) {
    cancelAnimationFrame(collapseRafId);
    collapseRafId = 0;
  }

  if (rootEl) {
    rootEl.setAttribute('data-dsb-dock', norm);
    rootEl.setAttribute('data-dsb-vertical', isVert ? '1' : '0');
    syncPanelWidthVar();
    if (onlyCollapseChanged && norm !== 'none') {
      // Commit collapse/expand attribute on the next VSync frame with ZERO synchronous React
      // re-render so Chromium's GPU compositor glides smoothly from 0% -> 100% with zero hitch!
      dsbRuntime.dockAnimUntil = performance.now() + 290;
      collapseRafId = requestAnimationFrame(() => {
        collapseRafId = 0;
        if (document.documentElement) {
          document.documentElement.setAttribute(
            'data-dsb-dock-collapsed',
            dsbRuntime.dockCollapsed ? '1' : '0',
          );
        }
      });
    } else {
      rootEl.setAttribute('data-dsb-dock-collapsed', isCollapsed ? '1' : '0');
    }
    if (prevVert !== isVert) {
      // 100% GPU Compositor-Thread FLIP Morph via Web Animations API (ZERO layout/width/clip-path
      // animations and ZERO legacy SetWindowRgn clipping -> locked 60/120/144Hz single piece!):
      dsbRuntime.dockAnimUntil = performance.now() + 265;
      if (activeMorphAnim) {
        try {
          activeMorphAnim.cancel();
        } catch {}
        activeMorphAnim = null;
      }
      requestAnimationFrame(() => {
        try {
          const seat = document.querySelector('[data-composer-seat]');
          const comp =
            document.querySelector('[data-dsb-external-composer]') ||
            document.querySelector('[data-composer-card]');
          if (isVert && seat && typeof seat.animate === 'function') {
            seat.style.transformOrigin =
              norm === 'right' ? 'right bottom' : norm === 'left' ? 'left bottom' : 'center bottom';
            const anim = seat.animate(
              [
                { transform: 'translate3d(0, 0, 0) scale3d(1.32, 0.24, 1)', opacity: 0.72 },
                { transform: 'translate3d(0, 0, 0) scale3d(1, 1, 1)', opacity: 1 },
              ],
              { duration: 235, easing: 'cubic-bezier(0.22, 1, 0.36, 1)' },
            );
            activeMorphAnim = anim;
            anim.onfinish = () => {
              seat.style.transformOrigin = '';
              if (activeMorphAnim === anim) activeMorphAnim = null;
            };
          } else if (!isVert && comp && typeof comp.animate === 'function') {
            comp.style.transformOrigin = norm === 'top' ? 'center top' : 'center bottom';
            const anim = comp.animate(
              [
                { transform: 'translate3d(0, -12px, 0) scale3d(0.56, 1.16, 1)', opacity: 0.72 },
                { transform: 'translate3d(0, 0, 0) scale3d(1, 1, 1)', opacity: 1 },
              ],
              { duration: 235, easing: 'cubic-bezier(0.22, 1, 0.36, 1)' },
            );
            activeMorphAnim = anim;
            anim.onfinish = () => {
              comp.style.transformOrigin = '';
              if (activeMorphAnim === anim) activeMorphAnim = null;
            };
          }
        } catch {}
      });
      if (morphClearTimer) clearTimeout(morphClearTimer);
      morphClearTimer = setTimeout(() => {
        morphClearTimer = null;
        if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
          dsbRuntime.lastReportKeys.delete('layout');
          dsbRuntime.scheduleLayoutReport(120);
        }
      }, 270);
    }
  }

  // Opening a docked or vertical panel shows the full history stream immediately -> clear unread pulse
  if (isVert && !isCollapsed && dsbRuntime.unreadCount > 0) {
    dsbRuntime.unreadCount = 0;
    dsbRuntime.unreadPreview = '';
  }

  if (prevEdge !== norm || prevCollapsed !== isCollapsed || prevVert !== isVert) {
    dsbRuntime.lastReportKeys.delete('layout');
    if (collapseNotifyTimer) {
      clearTimeout(collapseNotifyTimer);
      collapseNotifyTimer = null;
    }
    if (onlyCollapseChanged && norm !== 'none') {
      // Defer React state notification until AFTER the 240ms GPU glide completes!
      collapseNotifyTimer = setTimeout(() => {
        collapseNotifyTimer = null;
        notifyBarRuntime();
        if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
          dsbRuntime.lastReportKeys.delete('layout');
          dsbRuntime.scheduleLayoutReport(120);
        }
      }, 295);
    } else {
      notifyBarRuntime();
      if (prevVert === isVert && typeof dsbRuntime.scheduleLayoutReport === 'function') {
        dsbRuntime.scheduleLayoutReport(340);
      }
    }
  }
}

function notifyBarRuntime() {
  const hasUnread = (dsbRuntime.unreadCount || 0) > 0;
  if (typeof document !== 'undefined' && document.documentElement) {
    document.documentElement.setAttribute('data-dsb-dock-unread', hasUnread ? '1' : '0');
  }
  if (IS_BAR && hasUnread !== Boolean(dsbRuntime.lastReportedDockUnread)) {
    dsbRuntime.lastReportedDockUnread = hasUnread;
    tauriInvoke('bar_set_dock_unread', { unread: hasUnread }).catch(() => {});
  }
  for (const fn of dsbRuntime.listeners) {
    try {
      fn();
    } catch {}
  }
}

const EVENT_REPORT_KINDS = new Set([
  'step_down',
  'composer_pointerdown',
  'summon_from_lip',
  'submitted',
  'bubble_notify',
]);

function getActiveComposerCapsuleCount() {
  return dsbRuntime.activeHarness !== 'dsh'
    ? (dsbRuntime.extCapsules || []).length
    : countComposerCapsules(dsbRuntime.liveDraft);
}

function extractSwEntityKeys(text) {
  const s = String(text || '');
  const keys = [];
  let pos = 0;
  while (pos < s.length) {
    const swIdx = s.indexOf('[SW-TARGET v2] ', pos);
    if (swIdx < 0) break;
    const lineEnd = s.indexOf('\n', swIdx);
    const rawJson = (lineEnd >= 0 ? s.slice(swIdx + 15, lineEnd) : s.slice(swIdx + 15)).trim();
    pos = lineEnd >= 0 ? lineEnd + 1 : s.length;
    try {
      const parsed = JSON.parse(rawJson);
      const doc = (parsed && parsed.doc_context && parsed.doc_context.doc) || {};
      const docKey = doc.path || doc.title || '';
      const targets = Array.isArray(parsed && parsed.targets) ? parsed.targets : [];
      for (const t of targets) {
        if (!t) continue;
        const tid = Number(t.select_type_id) || 0;
        const pref = typeof t.persist_ref_b64 === 'string' ? t.persist_ref_b64.trim() : '';
        if (pref) {
          keys.push(docKey + '|' + tid + '|pid:' + pref);
        } else {
          keys.push(docKey + '|' + tid + '|name:' + String(t.display_name || '').trim());
        }
      }
    } catch {}
  }
  return keys;
}

function collectActiveSwEntityKeys() {
  const set = new Set();
  if (dsbRuntime.activeHarness !== 'dsh') {
    for (const c of dsbRuntime.extCapsules || []) {
      const arr = Array.isArray(c && c.swEntityKeys) ? c.swEntityKeys : extractSwEntityKeys(c && c.text);
      for (const k of arr) {
        if (k) set.add(k);
      }
    }
  } else {
    for (const k of extractSwEntityKeys(dsbRuntime.liveDraft)) {
      if (k) set.add(k);
    }
  }
  return Array.from(set);
}

function computeCapsuleDedupKey(text) {
  const keys = extractSwEntityKeys(text);
  if (keys.length > 0) {
    return 'sw:' + keys.slice().sort().join('|');
  }
  return '';
}

function circledBadgeForIndex(n) {
  const badges = '①②③④⑤⑥⑦⑧⑨⑩⑪⑫⑬⑭⑮⑯⑰⑱⑲⑳';
  return n >= 1 && n <= 20 ? badges[n - 1] : '#' + n;
}

function renumberExtCapsules(list) {
  const arr = Array.isArray(list) ? list : [];
  return arr.map((cap, idx) => {
    const num = idx + 1;
    const badge = circledBadgeForIndex(num);
    const prevSummary = (cap && cap.summary) || { label: badge, title: '引用', detail: '', body: '' };
    const rawTitle = String(prevSummary.title || '引用');
    const strippedTitle = rawTitle.replace(/^(?:[①-⑳]|#\d+)\s*/, '');
    const nextTitle = strippedTitle ? badge + ' ' + strippedTitle : badge;
    const nextText = String((cap && cap.text) || '')
      .replace(/^(\[DSB-PICK\s+)#\d+/m, '$1#' + num)
      .replace(/(\[DSB-PICK\s+#\d+\s+badge=)(?:[①-⑳]|#\d+)/m, '$1' + badge);
    return {
      ...cap,
      summary: {
        ...prevSummary,
        label: badge,
        title: nextTitle,
      },
      text: nextText,
    };
  });
}

function countComposerCapsules(draftText) {
  let draftCount = 0;
  if (typeof draftText === 'string' && draftText.length > 0) {
    const matches = draftText.match(/\[DSB-PICK\s+#\d+/g);
    if (matches) draftCount = matches.length;
  }
  let domCount = 0;
  if (typeof document !== 'undefined') {
    const editor =
      document.querySelector('[data-composer-card] [data-lexical-editor="true"]') ||
      document.querySelector('[data-composer-card] [contenteditable="true"]');
    if (editor) {
      const decorators = editor.querySelectorAll('[data-lexical-decorator="true"]');
      const candidates =
        decorators.length > 0
          ? Array.from(decorators)
          : Array.from(editor.querySelectorAll('[contenteditable="false"]')).filter(
              (el) => !el.parentElement || !el.parentElement.closest('[contenteditable="false"]'),
            );
      for (const el of candidates) {
        const txt = (el.textContent || '').trim();
        if (/[①-⑳]/.test(txt) || /#\d+/.test(txt) || txt.includes('“')) {
          domCount += 1;
        }
      }
    }
  }
  return draftCount > 0 ? draftCount : domCount;
}

function sendBarReport(payload) {
  if (!IS_BAR || typeof window === 'undefined' || !payload || !payload.kind) return;
  const tauriCore = window.__TAURI__ && window.__TAURI__.core;
  if (!tauriCore || typeof tauriCore.invoke !== 'function') return;
  const bypassDedupe = EVENT_REPORT_KINDS.has(payload.kind) || Boolean(payload.afterInsert);
  if (!bypassDedupe) {
    const serialized = JSON.stringify(payload);
    if (dsbRuntime.lastReportKeys.get(payload.kind) === serialized) return;
    dsbRuntime.lastReportKeys.set(payload.kind, serialized);
  } else if (payload.kind === 'capsules') {
    dsbRuntime.lastReportKeys.set(
      'capsules',
      JSON.stringify({ kind: 'capsules', capsuleCount: payload.capsuleCount, afterInsert: false }),
    );
  }
  tauriCore.invoke('bar_report', { payload }).catch(() => {});
}

function focusComposerElement() {
  if (typeof document === 'undefined') return false;
  const el =
    (dsbRuntime.activeHarness !== 'dsh'
      ? document.querySelector('[data-dsb-ext-input="1"]')
      : null) ||
    document.querySelector('[data-composer-card] [data-lexical-editor="true"]') ||
    document.querySelector('[data-composer-card] [contenteditable="true"]') ||
    document.querySelector('[data-composer-card] textarea');
  if (el && typeof el.focus === 'function') {
    try {
      window.focus();
    } catch {}
    el.focus({ preventScroll: true });
    if (el.getAttribute && el.getAttribute('contenteditable') === 'true') {
      try {
        const sel = window.getSelection && window.getSelection();
        if (sel) {
          const range = document.createRange();
          range.selectNodeContents(el);
          range.collapse(false);
          sel.removeAllRanges();
          sel.addRange(range);
        }
      } catch {}
    }
    return true;
  }
  return false;
}

function setBarMode(mode) {
  // Scheme B: in Bar mode, the composer is ALWAYS kept mounted & sized ('summon');
  // the obsolete 56px Lip-only pill is never shown.
  const normalized = IS_BAR ? 'summon' : mode === 'lip' ? 'lip' : 'summon';
  const wantExpanded = mode === 'preview' || mode === 'expanded';
  dsbRuntime.barMode = normalized;
  dsbRuntime.previewExpanded = wantExpanded;
  dsbRuntime.lastReportKeys.delete('layout');
  if (typeof document !== 'undefined') {
    document.documentElement.setAttribute('data-dsb-mode', normalized);
  }
  if (typeof dsbRuntime.setModeListener === 'function') {
    dsbRuntime.setModeListener(normalized, wantExpanded);
  }
  notifyBarRuntime();
  if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
    dsbRuntime.scheduleLayoutReport(380);
  }
}

function installGlobalBridge() {
  if (!IS_BAR || typeof window === 'undefined') return;
  dsbRuntime.barMode = 'summon';
  document.documentElement.setAttribute('data-dsb-mode', 'summon');
  applyHarnessThemeAttribute(dsbRuntime.activeHarness);
  window.__DSB_HARNESS_EVENT__ = handleIncomingHarnessEvent;
  window.__DSB_HARNESS_SYNC__ = handleIncomingHarnessSync;
  window.__DSB_DOCK_STATE__ = (st) => {
    if (st && typeof st === 'object') {
      applyDockState(st.edge, st.collapsed, st.vertical);
    }
  };
  tauriInvoke('bar_get_dock_state')
    .then((st) => {
      if (st && typeof st === 'object') {
        applyDockState(st.edge, st.collapsed, st.vertical);
      }
    })
    .catch(() => {});
  if (!window.__DSB_DOCK_CLICK_INSTALLED__) {
    window.__DSB_DOCK_CLICK_INSTALLED__ = true;
    document.addEventListener(
      'pointerdown',
      (ev) => {
        if (dsbRuntime.dockEdge !== 'none' && dsbRuntime.dockCollapsed && ev.button === 0) {
          tauriInvoke('bar_dock_expand').catch(() => {});
        }
      },
      true,
    );
  }
  window.__DSB_TARGET_WINDOW__ = (info) => {
    if (info && typeof info === 'object') {
      dsbRuntime.universalTarget = info;
      notifyBarRuntime();
    }
  };
  window.__DSB_BRIDGE__ = {
    setMode(mode) {
      setBarMode(mode);
      return dsbRuntime.barMode;
    },
    setVisibility(vis) {
      const next = vis === 'hidden' || vis === 'lip' || vis === 'summon' ? vis : 'hidden';
      dsbRuntime.barVisibility = next;
      if (next === 'hidden' && typeof dsbRuntime.onHiddenReset === 'function') {
        dsbRuntime.onHiddenReset();
      }
      notifyBarRuntime();
      return dsbRuntime.barVisibility;
    },
    setHarness(harnessId) {
      setActiveHarness(harnessId, false);
      return dsbRuntime.activeHarness;
    },
    openSettings() {
      if (typeof dsbRuntime.openSettingsMenu === 'function') {
        dsbRuntime.openSettingsMenu();
        return true;
      }
      return false;
    },
    refreshSettings() {
      refreshAppSettings();
      return true;
    },
    focusComposer() {
      return focusComposerElement();
    },
    setDraft(text) {
      const val = String(text || '');
      if (dsbRuntime.activeHarness !== 'dsh') {
        dsbRuntime.extDraftText = val;
        if (val === '') {
          dsbRuntime.extCapsules = [];
          sendBarReport({ kind: 'capsules', capsuleCount: 0, swEntityKeys: [], afterInsert: false });
        }
        notifyBarRuntime();
        focusComposerElement();
        if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
          dsbRuntime.scheduleLayoutReport(220);
        }
        return true;
      }
      const actions = dsbRuntime.inputActions;
      let ok = false;
      if (actions && typeof actions.setDraft === 'function') {
        try {
          actions.setDraft(val);
          focusComposerElement();
          ok = true;
        } catch {}
      }
      if (val === '' && typeof document !== 'undefined') {
        const ed =
          document.querySelector('[data-composer-card] [data-lexical-editor="true"]') ||
          document.querySelector('[data-composer-card] [contenteditable="true"]');
        if (ed && (ed.textContent || '').trim().length > 0) {
          try {
            ed.focus({ preventScroll: true });
            const sel = window.getSelection && window.getSelection();
            if (sel) {
              const range = document.createRange();
              range.selectNodeContents(ed);
              sel.removeAllRanges();
              sel.addRange(range);
              document.execCommand('delete', false);
            }
            ok = true;
          } catch {}
        }
      }
      return ok;
    },
    insertCapsule(capsule) {
      if (!capsule || typeof capsule.text !== 'string') return false;
      if (dsbRuntime.activeHarness !== 'dsh') {
        const dedupKey = computeCapsuleDedupKey(capsule.text);
        const swEntityKeys = extractSwEntityKeys(capsule.text);
        const existingSet = new Set(collectActiveSwEntityKeys());
        const isSwDup =
          swEntityKeys.length > 0 && swEntityKeys.every((k) => existingSet.has(k));
        if (
          isSwDup ||
          (dedupKey &&
            (dsbRuntime.extCapsules || []).some((c) => c && c.dedupKey === dedupKey))
        ) {
          focusComposerElement();
          sendBarReport({
            kind: 'capsules',
            capsuleCount: dsbRuntime.extCapsules.length,
            swEntityKeys: collectActiveSwEntityKeys(),
            afterInsert: false,
          });
          return false;
        }
        dsbRuntime.extCapsules = renumberExtCapsules([
          ...dsbRuntime.extCapsules,
          {
            id:
              (typeof capsule.id === 'string' && capsule.id.trim()) ||
              'cap-' + Date.now() + '-' + Math.random().toString(36).slice(2, 6),
            insertedAt: Date.now(),
            meta: capsule.meta && typeof capsule.meta === 'object' ? capsule.meta : null,
            dedupKey,
            swEntityKeys,
            summary: capsule.summary || { label: '•', title: '引用', detail: '', body: '' },
            text: capsule.text,
          },
        ]);
        notifyBarRuntime();
        focusComposerElement();
        sendBarReport({
          kind: 'capsules',
          capsuleCount: dsbRuntime.extCapsules.length,
          swEntityKeys: collectActiveSwEntityKeys(),
          afterInsert: true,
        });
        if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
          dsbRuntime.scheduleLayoutReport(260);
        }
        return true;
      }
      dsbRuntime.localDraftProtectUntil = Date.now() + 1800;
      const beforeCount = countComposerCapsules(dsbRuntime.liveDraft);
      const reportNow = (afterInsert) => {
        sendBarReport({
          kind: 'capsules',
          capsuleCount: countComposerCapsules(dsbRuntime.liveDraft),
          swEntityKeys: collectActiveSwEntityKeys(),
          afterInsert: Boolean(afterInsert),
        });
      };
      const attemptInsert = (attempt) => {
        const actions = dsbRuntime.inputActions;
        focusComposerElement();
        let called = false;
        if (actions && typeof actions.insertQuote === 'function') {
          try {
            const ok = actions.insertQuote(capsule.text, capsule.summary);
            if (ok !== false) called = true;
          } catch {}
        }
        if (!called && actions && typeof actions.setDraft === 'function' && attempt >= 2) {
          try {
            const prev = typeof dsbRuntime.liveDraft === 'string' ? dsbRuntime.liveDraft : '';
            const next = prev ? prev + '\n' + capsule.text : capsule.text;
            actions.setDraft(next);
            called = true;
          } catch {}
        }
        if (typeof window !== 'undefined') {
          window.requestAnimationFrame(() => {
            focusComposerElement();
            setTimeout(() => {
              focusComposerElement();
              const afterCount = countComposerCapsules(dsbRuntime.liveDraft);
              if (afterCount > beforeCount || attempt >= 2) {
                reportNow(true);
              } else {
                // Composer may still have been waking from Lip/Hidden; retry cleanly
                attemptInsert(attempt + 1);
              }
            }, attempt === 0 ? 55 : 95);
          });
        } else {
          reportNow(true);
        }
        return called;
      };
      return attemptInsert(0);
    },
    patchCapsule(patch) {
      if (!patch || typeof patch.id !== 'string' || !patch.id) return false;
      const list = dsbRuntime.extCapsules || [];
      let changed = false;
      const next = list.map((c) => {
        if (!c || c.id !== patch.id) return c;
        changed = true;
        const nextText = typeof patch.text === 'string' ? patch.text : c.text;
        return {
          ...c,
          summary: patch.summary ? { ...(c.summary || {}), ...patch.summary } : c.summary,
          text: nextText,
          dedupKey: computeCapsuleDedupKey(nextText),
        };
      });
      if (changed) {
        dsbRuntime.extCapsules = renumberExtCapsules(next);
        notifyBarRuntime();
        return true;
      }
      return false;
    },
    getState() {
      const activeEl = typeof document !== 'undefined' ? document.activeElement : null;
      const composerEl =
        typeof document !== 'undefined'
          ? dsbRuntime.activeHarness !== 'dsh'
            ? document.querySelector('[data-dsb-external-composer]')
            : document.querySelector('[data-composer-card]')
          : null;
      return {
        mode: dsbRuntime.barMode,
        sessionId: dsbRuntime.sessionId,
        hovered: dsbRuntime.hovered,
        pinned: dsbRuntime.pinned,
        popupOpen: dsbRuntime.popupOpen,
        modal: dsbRuntime.modalOpen,
        composerFocused: Boolean(composerEl && activeEl && composerEl.contains(activeEl)),
        capsuleCount:
          dsbRuntime.activeHarness !== 'dsh'
            ? dsbRuntime.extCapsules.length
            : countComposerCapsules(dsbRuntime.liveDraft),
        expandedHeight: dsbRuntime.expandedHeight,
        barWidth: dsbRuntime.barWidth,
      };
    },
  };
}


function dismissNativeTriggerMenu() {
  if (typeof document === 'undefined' || !document.body) return false;
  if (!document.querySelector('[data-trigger-menu]')) return false;
  try {
    const Ev = typeof PointerEvent === 'function' ? PointerEvent : MouseEvent;
    document.body.dispatchEvent(new Ev('pointerdown', { bubbles: true, cancelable: true }));
    if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
      dsbRuntime.scheduleLayoutReport(220);
    }
    return true;
  } catch {
    return false;
  }
}
