function tauriInvoke(cmd, args) {
  if (typeof window === 'undefined') return Promise.reject(new Error('no window'));
  const core = window.__TAURI__ && window.__TAURI__.core;
  if (!core || typeof core.invoke !== 'function') {
    return Promise.reject(new Error('no tauri core'));
  }
  return core.invoke(cmd, args || {}).catch((err) => {
    if (cmd !== 'bar_report') {
      try {
        core.invoke('bar_report', {
          payload: { kind: 'diag', detail: 'INVOKE_ERR ' + cmd + ': ' + String(err) },
        }).catch(() => {});
      } catch {}
    }
    throw err;
  });
}

const SELF_CLIENT_ID = 'dsh-twin-' + Math.random().toString(36).slice(2, 10);
const IS_BAR =
  typeof window !== 'undefined' &&
  new URLSearchParams(window.location.search).get('dsh_bar_clone') === '1';
const INITIAL_BAR_MODE =
  IS_BAR &&
  typeof window !== 'undefined' &&
  new URLSearchParams(window.location.search).get('dsb_mode') === 'lip'
    ? 'lip'
    : 'summon';

const broadcastChan =
  typeof BroadcastChannel !== 'undefined'
    ? new BroadcastChannel('anything-to-prompt-twin-v1')
    : null;

function readStoredBool(key, fallback) {
  try {
    const raw = localStorage.getItem(key);
    return raw === null ? fallback : raw === '1';
  } catch {
    return fallback;
  }
}

function readStoredNumber(key, fallback, min, max) {
  try {
    const val = Number(localStorage.getItem(key));
    return Number.isFinite(val) && val >= min && val <= max ? val : fallback;
  } catch {
    return fallback;
  }
}

function readStoredString(key, fallback) {
  try {
    const raw = localStorage.getItem(key);
    return raw && raw.trim() ? raw.trim() : fallback;
  } catch {
    return fallback;
  }
}

function writeStoredString(key, value) {
  try {
    if (value) localStorage.setItem(key, String(value));
  } catch {}
}

const STORAGE_KEY_EXT_SEL_PREFIX = 'dsh_card_dock:ext_sel:';
const STORAGE_KEY_HARNESS_PREFS_PREFIX = 'dsh_card_dock:prefs:';
const DETACHED_GAP_PX = 8;

function readStoredHarnessPrefs(harnessId) {
  try {
    const hId = normalizeHarnessId(harnessId);
    const raw = localStorage.getItem(STORAGE_KEY_HARNESS_PREFS_PREFIX + hId);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === 'object' ? parsed : {};
  } catch {
    return {};
  }
}

function writeStoredHarnessPrefs(harnessId, prefs) {
  try {
    const hId = normalizeHarnessId(harnessId);
    localStorage.setItem(STORAGE_KEY_HARNESS_PREFS_PREFIX + hId, JSON.stringify(prefs || {}));
  } catch {}
}

function readStoredExtSelection(hId) {
  const fallback = { ...getHarnessMeta(hId).defaultSelection };
  try {
    const raw = localStorage.getItem(STORAGE_KEY_EXT_SEL_PREFIX + hId);
    if (!raw) return fallback;
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === 'object') {
      return {
        workspaceId: String(parsed.workspaceId || fallback.workspaceId || ''),
        workspacePath: String(parsed.workspacePath || fallback.workspacePath || ''),
        sessionId: String(parsed.sessionId || ''),
        title: String(parsed.title || fallback.title || ''),
      };
    }
  } catch {}
  return fallback;
}

function writeStoredExtSelection(hId, sel) {
  try {
    if (!hId || !sel) return;
    localStorage.setItem(
      STORAGE_KEY_EXT_SEL_PREFIX + hId,
      JSON.stringify({
        workspaceId: sel.workspaceId || '',
        workspacePath: sel.workspacePath || '',
        sessionId: sel.sessionId || '',
        title: sel.title || '',
      }),
    );
  } catch {}
}

// Shared runtime handles between Dock, Picker Chip, and Tauri Bridge
const dsbRuntime = {
  barMode: IS_BAR ? 'summon' : INITIAL_BAR_MODE,
  barVisibility: 'hidden',
  pendingBubbleTarget: null,
  lastSeenSessionReply: {},
  previewExpanded: false,
  barWidth: readStoredNumber(STORAGE_KEY_BAR_WIDTH, DEFAULT_BAR_WIDTH_PX, MIN_BAR_WIDTH_PX, MAX_BAR_WIDTH_PX),
  sideWidth: readStoredNumber(STORAGE_KEY_SIDE_WIDTH, DEFAULT_SIDE_WIDTH_PX, MIN_SIDE_WIDTH_PX, MAX_SIDE_WIDTH_PX),
  modalOpen: false,
  popupOpen: false,
  quickSearchOpen: false,
  quickSearchSelectedIdx: -1,
  hovered: false,
  pinned: IS_BAR ? false : readStoredBool(STORAGE_KEY_PINNED, false),
  expandedHeight: readStoredNumber(STORAGE_KEY_HEIGHT, DEFAULT_EXPANDED_PX, MIN_EXPANDED_PX, MAX_EXPANDED_PX),
  sessionId: '',
  liveDraft: '',
  localDraftProtectUntil: 0,
  inputActions: null,
  setModeListener: null,
  setPopupCloseListener: null,
  scheduleLayoutReport: null,
  lastReportKeys: new Map(),
  listeners: new Set(),
  // Multi-harness state (auto-initialized dynamically for every registered harness in `harnesses/<id>/`)
  activeHarness: normalizeHarnessId(readStoredString(STORAGE_KEY_ACTIVE_HARNESS, 'dsh')),
  harnessAvailability: [],
  externalCatalogs: Object.fromEntries(
    HARNESS_ORDER.filter((id) => !getHarnessMeta(id).useNativeComposer).map((id) => [id, null]),
  ),
  externalSelection: Object.fromEntries(
    HARNESS_ORDER.filter((id) => !getHarnessMeta(id).useNativeComposer).map((id) => [
      id,
      readStoredExtSelection(id),
    ]),
  ),
  externalMessages: Object.fromEntries(
    HARNESS_ORDER.filter((id) => !getHarnessMeta(id).useNativeComposer).map((id) => [id, []]),
  ),
  externalActiveTurn: null,
  externalLoading: Object.fromEntries(
    HARNESS_ORDER.filter((id) => !getHarnessMeta(id).useNativeComposer).map((id) => [id, false]),
  ),
  liveSyncMeta: Object.fromEntries(
    HARNESS_ORDER.filter((id) => !getHarnessMeta(id).useNativeComposer).map((id) => [id, null]),
  ),
  lastSyncCatalogRefresh: Object.fromEntries(
    HARNESS_ORDER.filter((id) => !getHarnessMeta(id).useNativeComposer).map((id) => [id, 0]),
  ),
  appSettings: null,
  claudePermissionMode: readStoredString(STORAGE_KEY_CLAUDE_PERM, 'auto'),
  themePref: readStoredString(STORAGE_KEY_THEME_PREF, 'auto'),
  resolvedTheme: 'dark',
  extDraftText: '',
  extCapsules: [],
  extAttachments: [],
  harnessOptions: {},
  harnessPrefs: Object.fromEntries(
    HARNESS_ORDER.filter((id) => !getHarnessMeta(id).useNativeComposer).map((id) => [
      id,
      readStoredHarnessPrefs(id),
    ]),
  ),
  sessionSyncState: {},
  sessionDirtyKeys: {},
  universalTarget: null,
  unreadCount: 0,
  unreadPreview: '',
  dockEdge: 'none',
  dockCollapsed: false,
  verticalMode: false,
  lastReportedDockUnread: false,
  dragSnapEdge: 'none',
  toggleHarnessMenu: null,
  openSettingsMenu: null,
};

function refreshHarnessOptions(harnessId) {
  const hId = normalizeHarnessId(harnessId);
  if (!IS_BAR || hId === 'dsh') return Promise.resolve(null);
  return tauriInvoke('harness_list_options', { harness: hId })
    .then((opts) => {
      if (opts && typeof opts === 'object') {
        dsbRuntime.harnessOptions[hId] = opts;
        notifyBarRuntime();
      }
      return opts;
    })
    .catch(() => null);
}

function updateHarnessPrefs(harnessId, patch) {
  const hId = normalizeHarnessId(harnessId);
  const next = Object.assign({}, dsbRuntime.harnessPrefs[hId] || {}, patch || {});
  dsbRuntime.harnessPrefs[hId] = next;
  writeStoredHarnessPrefs(hId, next);
  const curSel = dsbRuntime.externalSelection[hId] || {};
  if (curSel.sessionId && patch && typeof patch === 'object') {
    const dKey = hId + ':' + curSel.sessionId;
    const set = dsbRuntime.sessionDirtyKeys[dKey] || new Set();
    for (const k of ['model', 'effort', 'mode', 'planMode']) {
      if (Object.prototype.hasOwnProperty.call(patch, k)) set.add(k);
    }
    dsbRuntime.sessionDirtyKeys[dKey] = set;
  }
  if (hId === 'claude_code' && next.mode) {
    dsbRuntime.claudePermissionMode = next.mode;
    writeStoredString(STORAGE_KEY_CLAUDE_PERM, next.mode);
  }
  notifyBarRuntime();
  return next;
}

function syncExternalSessionState(harnessId, sessionId, forceOverwrite = true) {
  const hId = normalizeHarnessId(harnessId);
  if (!IS_BAR || hId === 'dsh') return Promise.resolve(null);
  const sid = String(sessionId || '').trim();
  if (!sid) {
    delete dsbRuntime.sessionSyncState[hId];
    dsbRuntime.harnessPrefs[hId] = readStoredHarnessPrefs(hId);
    notifyBarRuntime();
    return Promise.resolve(null);
  }
  return tauriInvoke('harness_session_state', { harness: hId, sessionId: sid })
    .then((st) => {
      const curSel = dsbRuntime.externalSelection[hId] || {};
      if (String(curSel.sessionId || '').trim() !== sid) return null;
      if (!st || typeof st !== 'object') return null;
      dsbRuntime.sessionSyncState[hId] = st;
      const dKey = hId + ':' + sid;
      if (forceOverwrite) {
        delete dsbRuntime.sessionDirtyKeys[dKey];
      }
      const dirty = dsbRuntime.sessionDirtyKeys[dKey];
      const base = forceOverwrite
        ? Object.assign({}, readStoredHarnessPrefs(hId))
        : Object.assign({}, dsbRuntime.harnessPrefs[hId] || {});
      const allowApply = (field) => forceOverwrite || Boolean(st.readOnlyInDesktop) || !dirty || !dirty.has(field);
      if (st.model && allowApply('model')) base.model = st.model;
      if (st.effort && allowApply('effort')) base.effort = st.effort;
      if (st.mode && allowApply('mode')) base.mode = st.mode;
      if (typeof st.planMode === 'boolean' && allowApply('planMode')) base.planMode = st.planMode;
      // Update live display/turn prefs for the bound conversation without mutating global localStorage defaults
      dsbRuntime.harnessPrefs[hId] = base;
      notifyBarRuntime();
      return st;
    })
    .catch(() => null);
}

function addExtAttachments(items) {
  if (!Array.isArray(items) || items.length === 0) return;
  const existing = dsbRuntime.extAttachments || [];
  const seen = new Set(existing.map((a) => a && a.path));
  const merged = existing.slice();
  for (const it of items) {
    if (!it || !it.path || seen.has(it.path)) continue;
    seen.add(it.path);
    merged.push(it);
  }
  dsbRuntime.extAttachments = merged;
  notifyBarRuntime();
  if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
    dsbRuntime.scheduleLayoutReport(240);
  }
}

function removeExtAttachment(attId) {
  dsbRuntime.extAttachments = (dsbRuntime.extAttachments || []).filter((a) => a && a.id !== attId);
  notifyBarRuntime();
  if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
    dsbRuntime.scheduleLayoutReport(200);
  }
}

function detectResolvedColorMode(pref) {
  const p = String(pref || dsbRuntime.themePref || 'auto').toLowerCase();
  if (p === 'light' || p === 'dark') return p;
  if (typeof document !== 'undefined' && document.documentElement) {
    const root = document.documentElement;
    const dt = String(root.getAttribute('data-theme') || root.getAttribute('data-color-mode') || '').toLowerCase();
    if (dt === 'light' || dt === 'dark') return dt;
    if (root.classList.contains('light')) return 'light';
    if (root.classList.contains('dark')) return 'dark';
    try {
      const card = document.querySelector('[data-composer-card]') || document.body;
      if (card) {
        const bg = getComputedStyle(card).backgroundColor || '';
        const m = bg.match(/rgba?\((\d+),\s*(\d+),\s*(\d+)/);
        if (m) {
          const lum = 0.299 * Number(m[1]) + 0.587 * Number(m[2]) + 0.114 * Number(m[3]);
          if (Number(m[1]) + Number(m[2]) + Number(m[3]) > 0) {
            return lum > 140 ? 'light' : 'dark';
          }
        }
      }
    } catch {}
  }
  try {
    if (typeof window !== 'undefined' && window.matchMedia && window.matchMedia('(prefers-color-scheme: light)').matches) {
      return 'light';
    }
  } catch {}
  return 'dark';
}

function getUnifiedTheme(harnessId) {
  const norm = normalizeHarnessId(harnessId || dsbRuntime.activeHarness);
  const mode = dsbRuntime.resolvedTheme === 'light' ? 'light' : 'dark';
  const entry = UNIFIED_PALETTES[norm] || UNIFIED_PALETTES.dsh;
  const pal = entry[mode] || entry.dark;
  return Object.assign({ mode, isDark: mode === 'dark', dotStyle: entry.dotStyle || pal.accent }, pal);
}

function applyHarnessThemeAttribute(harnessId) {
  const norm = normalizeHarnessId(harnessId || dsbRuntime.activeHarness);
  const resolved = detectResolvedColorMode(dsbRuntime.themePref);
  dsbRuntime.resolvedTheme = resolved;
  if (typeof document !== 'undefined' && document.documentElement) {
    const root = document.documentElement;
    root.setAttribute('data-dsb-harness', norm);
    root.setAttribute('data-dsb-theme', resolved);
    const t = getUnifiedTheme(norm);
    root.style.setProperty('--atp-surface', t.surface);
    root.style.setProperty('--atp-popover', t.popover);
    root.style.setProperty('--atp-drawer', t.drawer);
    root.style.setProperty('--atp-border', t.border);
    root.style.setProperty('--atp-fg', t.fg);
    root.style.setProperty('--atp-muted', t.muted);
    root.style.setProperty('--atp-accent', t.accent);
    root.style.setProperty('--atp-accent-bg', t.accentBg);
  }
}

function setThemePreference(pref) {
  const clean = pref === 'light' || pref === 'dark' ? pref : 'auto';
  dsbRuntime.themePref = clean;
  writeStoredString(STORAGE_KEY_THEME_PREF, clean);
  applyHarnessThemeAttribute(dsbRuntime.activeHarness);
  notifyBarRuntime();
}

if (typeof window !== 'undefined' && typeof document !== 'undefined' && document.documentElement) {
  try {
    const themeObs = new MutationObserver(() => {
      if (dsbRuntime.themePref === 'auto') {
        const next = detectResolvedColorMode('auto');
        if (next !== dsbRuntime.resolvedTheme) {
          applyHarnessThemeAttribute(dsbRuntime.activeHarness);
          notifyBarRuntime();
        }
      }
    });
    themeObs.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'data-theme', 'data-color-mode'] });
    if (window.matchMedia) {
      const mq = window.matchMedia('(prefers-color-scheme: light)');
      if (mq && typeof mq.addEventListener === 'function') {
        mq.addEventListener('change', () => {
          if (dsbRuntime.themePref === 'auto') {
            applyHarnessThemeAttribute(dsbRuntime.activeHarness);
            notifyBarRuntime();
          }
        });
      }
    }
  } catch {}
}

function refreshHarnessAvailability() {
  if (!IS_BAR) return Promise.resolve([]);
  return tauriInvoke('harness_list_available')
    .then((list) => {
      if (Array.isArray(list)) {
        syncDynamicHarnessesFromAvailability(list);
        dsbRuntime.harnessAvailability = list;
        notifyBarRuntime();
      }
      return list;
    })
    .catch(() => []);
}

function refreshUniversalTarget() {
  if (!IS_BAR) return Promise.resolve(null);
  return tauriInvoke('universal_get_target')
    .then((info) => {
      if (info && typeof info === 'object') {
        dsbRuntime.universalTarget = info;
        notifyBarRuntime();
      }
      return info;
    })
    .catch(() => null);
}

function loadExternalConversationMessages(harnessId, sessionId) {
  const hId = normalizeHarnessId(harnessId);
  if (hId === 'dsh') return Promise.resolve([]);
  if (!sessionId) {
    dsbRuntime.externalMessages[hId] = [];
    notifyBarRuntime();
    return Promise.resolve([]);
  }
  return tauriInvoke('harness_load_messages', { harness: hId, sessionId })
    .then((msgs) => {
      if (Array.isArray(msgs)) {
        dsbRuntime.externalMessages[hId] = msgs;
        notifyBarRuntime();
      }
      return msgs;
    })
    .catch(() => []);
}

function findExternalCatalogEntry(hId, sessionId) {
  if (!sessionId) return null;
  const catalog = dsbRuntime.externalCatalogs[hId];
  const groups = catalog && Array.isArray(catalog.groups) ? catalog.groups : [];
  for (const ws of groups) {
    const convs = Array.isArray(ws.convs) ? ws.convs : [];
    for (const conv of convs) {
      if (conv && String(conv.id) === String(sessionId)) {
        return { ws, conv };
      }
    }
  }
  return null;
}

let defaultWorkspacePromise = null;
function ensureDefaultWorkspacePath() {
  if (dsbRuntime.defaultWorkspacePath) {
    return Promise.resolve(dsbRuntime.defaultWorkspacePath);
  }
  if (!defaultWorkspacePromise) {
    defaultWorkspacePromise = tauriInvoke('app_ensure_default_workspace')
      .then((p) => {
        const clean = typeof p === 'string' ? p.trim() : '';
        if (clean) dsbRuntime.defaultWorkspacePath = clean;
        return clean;
      })
      .catch(() => '')
      .finally(() => {
        defaultWorkspacePromise = null;
      });
  }
  return defaultWorkspacePromise;
}

function applyDefaultWorkspaceSelectionIfEmpty(hId) {
  if (hId === 'dsh') return Promise.resolve(false);
  return ensureDefaultWorkspacePath().then((defaultPath) => {
    if (!defaultPath) return false;
    const cur = dsbRuntime.externalSelection[hId] || {};
    if (cur.sessionId || cur.workspacePath) return false;
    const label = workspaceBasename(defaultPath) || '默认工作区';
    dsbRuntime.externalSelection[hId] = {
      workspaceId: hId === 'antigravity' ? 'default-cli-project' : defaultPath,
      workspacePath: defaultPath,
      sessionId: '',
      title: label + ' · 新对话',
    };
    writeStoredExtSelection(hId, dsbRuntime.externalSelection[hId]);
    notifyBarRuntime();
    return true;
  });
}

function refreshExternalCatalog(harnessId, autoSelectIfEmpty = true) {
  const hId = normalizeHarnessId(harnessId);
  if (hId === 'dsh' || !IS_BAR) return Promise.resolve(null);
  dsbRuntime.externalLoading[hId] = true;
  notifyBarRuntime();
  return tauriInvoke('harness_load_catalog', { harness: hId })
    .then((catalog) => {
      dsbRuntime.externalLoading[hId] = false;
      if (catalog && typeof catalog === 'object') {
        dsbRuntime.externalCatalogs[hId] = catalog;
        const groups = Array.isArray(catalog.groups) ? catalog.groups : [];
        const cur = dsbRuntime.externalSelection[hId] || {};
        if (cur.sessionId) {
          const matched = findExternalCatalogEntry(hId, cur.sessionId);
          if (matched) {
            dsbRuntime.externalSelection[hId] = {
              workspaceId: matched.ws.workspaceId || cur.workspaceId || '',
              workspacePath: matched.ws.path || matched.conv.cwd || cur.workspacePath || '',
              sessionId: cur.sessionId,
              title: matched.conv.title || cur.title || matched.ws.label || '新对话',
            };
            writeStoredExtSelection(hId, dsbRuntime.externalSelection[hId]);
            if (!Array.isArray(dsbRuntime.externalMessages[hId]) || dsbRuntime.externalMessages[hId].length === 0) {
              loadExternalConversationMessages(hId, cur.sessionId);
            }
            syncExternalSessionState(hId, cur.sessionId, false);
          } else if (groups.length > 0) {
            const firstWs = groups[0];
            const firstConv = Array.isArray(firstWs.convs) && firstWs.convs.length > 0 ? firstWs.convs[0] : null;
            dsbRuntime.externalSelection[hId] = firstConv
              ? {
                  workspaceId: firstWs.workspaceId || '',
                  workspacePath: firstWs.path || firstConv.cwd || '',
                  sessionId: firstConv.id,
                  title: firstConv.title || firstWs.label || '新对话',
                }
              : {
                  workspaceId: firstWs.workspaceId || '',
                  workspacePath: firstWs.path || '',
                  sessionId: '',
                  title: (firstWs.label || HARNESS_META[hId].label) + ' · 新对话',
                };
            writeStoredExtSelection(hId, dsbRuntime.externalSelection[hId]);
            if (firstConv) {
              loadExternalConversationMessages(hId, firstConv.id);
              syncExternalSessionState(hId, firstConv.id, true);
            } else {
              syncExternalSessionState(hId, '', true);
            }
          }
        } else if (
          (autoSelectIfEmpty ||
            (cur.workspaceId && !groups.some((g) => g && g.workspaceId === cur.workspaceId))) &&
          groups.length > 0
        ) {
          const firstWs = groups[0];
          const firstConv = Array.isArray(firstWs.convs) && firstWs.convs.length > 0 ? firstWs.convs[0] : null;
          if (firstConv) {
            dsbRuntime.externalSelection[hId] = {
              workspaceId: firstWs.workspaceId || '',
              workspacePath: firstWs.path || firstConv.cwd || '',
              sessionId: firstConv.id,
              title: firstConv.title || firstWs.label || '新对话',
            };
            loadExternalConversationMessages(hId, firstConv.id);
            syncExternalSessionState(hId, firstConv.id, true);
          } else {
            dsbRuntime.externalSelection[hId] = {
              workspaceId: firstWs.workspaceId || '',
              workspacePath: firstWs.path || '',
              sessionId: '',
              title: (firstWs.label || HARNESS_META[hId].label) + ' · 新对话',
            };
            syncExternalSessionState(hId, '', true);
          }
        } else if (catalog.available && groups.length === 0) {
          // Cold-start with zero existing workspaces/conversations: auto-bind unified default workspace
          // so the user can send message #1 immediately without manual workspace setup.
          applyDefaultWorkspaceSelectionIfEmpty(hId);
        }
        notifyBarRuntime();
      }
      return catalog;
    })
    .catch(() => {
      dsbRuntime.externalLoading[hId] = false;
      notifyBarRuntime();
      return null;
    });
}

function setActiveHarness(harnessId, persist = true, skipCatalogRefresh = false) {
  const norm = normalizeHarnessId(harnessId);
  ensureExternalRuntimeSlots(norm);
  if (dsbRuntime.activeHarness !== norm) {
    dsbRuntime.unreadCount = 0;
    dsbRuntime.unreadPreview = '';
    dsbRuntime.pendingBubbleTarget = null;
  }
  dsbRuntime.activeHarness = norm;
  applyHarnessThemeAttribute(norm);
  if (persist) {
    writeStoredString(STORAGE_KEY_ACTIVE_HARNESS, norm);
    tauriInvoke('harness_set_active', { harness: norm }).catch(() => {});
  }
  if (norm === 'universal') {
    refreshUniversalTarget();
    refreshHarnessOptions('universal');
  } else if (!getHarnessMeta(norm).useNativeComposer && !skipCatalogRefresh) {
    refreshExternalCatalog(norm, true);
    if (norm.startsWith('acp:') || !dsbRuntime.harnessOptions[norm]) {
      refreshHarnessOptions(norm);
    }
  } else if (!getHarnessMeta(norm).useNativeComposer) {
    const curSel = dsbRuntime.externalSelection[norm] || {};
    if (curSel.sessionId) syncExternalSessionState(norm, curSel.sessionId, false);
  }
  notifyBarRuntime();
  if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
    dsbRuntime.scheduleLayoutReport(280);
  }
}

function selectExternalConversation(harnessId, group, conv) {
  const hId = normalizeHarnessId(harnessId);
  if (hId === 'dsh' || !conv) return;
  dsbRuntime.unreadCount = 0;
  dsbRuntime.unreadPreview = '';
  dsbRuntime.pendingBubbleTarget = null;
  dsbRuntime.externalSelection[hId] = {
    workspaceId: group ? group.workspaceId || '' : '',
    workspacePath: (group && group.path) || conv.cwd || '',
    sessionId: conv.id,
    title: conv.title || '新对话',
  };
  writeStoredExtSelection(hId, dsbRuntime.externalSelection[hId]);
  notifyBarRuntime();
  loadExternalConversationMessages(hId, conv.id);
  syncExternalSessionState(hId, conv.id, true);
}

function startNewExternalConversation(harnessId, group) {
  const hId = normalizeHarnessId(harnessId);
  if (hId === 'dsh') return;
  dsbRuntime.unreadCount = 0;
  dsbRuntime.unreadPreview = '';
  dsbRuntime.pendingBubbleTarget = null;
  const wsId = group ? group.workspaceId || '' : (hId === 'antigravity' ? 'default-cli-project' : '');
  const wsPath = group ? group.path || '' : '';
  const wsLabel = group ? group.label || HARNESS_META[hId].label : HARNESS_META[hId].label;
  dsbRuntime.externalSelection[hId] = {
    workspaceId: wsId,
    workspacePath: wsPath,
    sessionId: '',
    title: wsLabel + ' · 新对话',
  };
  writeStoredExtSelection(hId, dsbRuntime.externalSelection[hId]);
  dsbRuntime.externalMessages[hId] = [];
  syncExternalSessionState(hId, '', true);
  notifyBarRuntime();
}

function cancelActiveExternalTurn() {
  const turn = dsbRuntime.externalActiveTurn;
  if (!turn || !turn.turnId) return;
  tauriInvoke('harness_cancel_turn', { turnId: turn.turnId }).catch(() => {});
  const list = dsbRuntime.externalMessages[turn.harness];
  if (Array.isArray(list)) {
    const idx = list.findIndex((m) => m.id === turn.assistantMsgId);
    if (idx >= 0) {
      list[idx] = {
        ...list[idx],
        text: (list[idx].text || '') + (list[idx].text ? '\n[已停止]' : '[已停止]'),
        streaming: false,
      };
    }
  }
  dsbRuntime.externalActiveTurn = null;
  notifyBarRuntime();
}

function showExternalSessionStats(harnessId) {
  const hId = normalizeHarnessId(harnessId || dsbRuntime.activeHarness);
  if (hId === 'dsh') return Promise.resolve(null);
  const sel = dsbRuntime.externalSelection[hId] || {};
  return tauriInvoke('harness_session_stats', {
    harness: hId,
    sessionId: sel.sessionId || '',
  })
    .then((stats) => {
      const md =
        (stats && stats.summaryMarkdown) ||
        '暂无该会话的本地用量统计记录。';
      const prev = Array.isArray(dsbRuntime.externalMessages[hId])
        ? dsbRuntime.externalMessages[hId]
        : [];
      dsbRuntime.externalMessages[hId] = prev.concat([
        {
          id: 'ext-stats-' + Date.now(),
          role: 'assistant',
          text: md,
          streaming: false,
        },
      ]);
      notifyBarRuntime();
      return stats;
    })
    .catch(() => null);
}

function submitExternalHarnessPrompt(rawText, attachmentsInput, rollbackInput) {
  const hId = dsbRuntime.activeHarness;
  if (hId === 'dsh') return false;
  const prompt = String(rawText || '').trim();
  const atts = Array.isArray(attachmentsInput) ? attachmentsInput : [];
  if (!prompt && atts.length === 0) return false;
  if (dsbRuntime.externalActiveTurn && dsbRuntime.externalActiveTurn.harness === hId) {
    cancelActiveExternalTurn();
  }
  const turnId = 'turn-' + Date.now() + '-' + Math.random().toString(36).slice(2, 7);
  const userMsgId = 'ext-u-' + turnId;
  const asstMsgId = 'ext-a-' + turnId;
  const sel = dsbRuntime.externalSelection[hId] || {};
  const opts = dsbRuntime.harnessOptions[hId] || {};
  const prefs = dsbRuntime.harnessPrefs[hId] || {};
  const adapter = getHarnessAdapter(hId);
  const def = adapter && typeof adapter.getDefaults === 'function' ? adapter.getDefaults() : {};
  const resolvedModel = prefs.model || opts.defaultModel || def.defaultModel || null;
  const resolvedEffort = prefs.effort || opts.defaultEffort || def.defaultEffort || null;
  const resolvedMode = prefs.mode || opts.defaultMode || def.defaultMode || null;

  const attNames = atts.map((a) => (a.kind === 'image' ? '🖼 ' : a.kind === 'folder' ? '📁 ' : '📎 ') + a.name);
  const displayUserText = attNames.length > 0
    ? attNames.join('  ') + (prompt ? '\n' + prompt : '')
    : prompt;

  const prevList = Array.isArray(dsbRuntime.externalMessages[hId]) ? dsbRuntime.externalMessages[hId] : [];
  dsbRuntime.externalMessages[hId] = prevList.concat([
    { id: userMsgId, role: 'user', text: displayUserText, streaming: false },
    { id: asstMsgId, role: 'assistant', text: '', streaming: true },
  ]);
  dsbRuntime.externalSendError = '';
  dsbRuntime.externalActiveTurn = {
    turnId,
    harness: hId,
    assistantMsgId: asstMsgId,
    toolNote: '',
    receivedAssistantText: false,
    rollback: rollbackInput || null,
  };
  if (!sel.sessionId) {
    const seed = prompt || (atts[0] && atts[0].name) || '新对话';
    const snippet = seed.replace(/\s+/g, ' ').slice(0, 22);
    dsbRuntime.externalSelection[hId] = {
      ...sel,
      title: snippet + (seed.length > 22 ? '…' : ''),
    };
  }
  notifyBarRuntime();
  sendBarReport({ kind: 'submitted', sessionId: sel.sessionId || turnId });

  const restoreOnSendFailure = (errMsg) => {
    dsbRuntime.externalSendError = String(errMsg || '消息发送失败');
    const rb = rollbackInput;
    if (
      rb &&
      !(dsbRuntime.extDraftText || '').trim() &&
      (!Array.isArray(dsbRuntime.extCapsules) || dsbRuntime.extCapsules.length === 0) &&
      (!Array.isArray(dsbRuntime.extAttachments) || dsbRuntime.extAttachments.length === 0)
    ) {
      dsbRuntime.extDraftText = rb.draft || '';
      dsbRuntime.extCapsules = Array.isArray(rb.capsules) ? rb.capsules.slice() : [];
      dsbRuntime.extAttachments = Array.isArray(rb.attachments) ? rb.attachments.slice() : [];
      sendBarReport({
        kind: 'capsules',
        capsuleCount: dsbRuntime.extCapsules.length,
        swEntityKeys: collectActiveSwEntityKeys(),
        afterInsert: false,
      });
    }
  };

  const harnessSpecific =
    adapter && typeof adapter.buildTurnPayload === 'function'
      ? adapter.buildTurnPayload({
          selection: sel,
          options: opts,
          prefs,
          resolvedModel,
          resolvedEffort,
          resolvedMode,
        })
      : {
          projectId: hId === 'antigravity' ? (sel.workspaceId || 'default-cli-project') : null,
          permissionMode: hId === 'claude_code' ? (resolvedMode || dsbRuntime.claudePermissionMode || 'auto') : null,
          model: resolvedModel,
          effort: resolvedEffort,
          sandboxMode: hId === 'codex' ? (resolvedMode || 'danger-full-access') : null,
          executionMode:
            hId === 'universal'
              ? (resolvedMode || 'paste_only')
              : hId.startsWith('acp:') || hId === 'antigravity'
                ? (resolvedMode || 'auto')
                : null,
        };

  tauriInvoke('harness_send_message', {
    payload: Object.assign(
      {
        harness: hId,
        turnId,
        cwd: sel.workspacePath || null,
        projectId: null,
        sessionId: sel.sessionId || null,
        permissionMode: null,
        model: resolvedModel,
        effort: resolvedEffort,
        sandboxMode: null,
        executionMode: null,
        fastSpeed: null,
        planMode: null,
        sandboxEnabled: null,
        chromeEnabled: null,
        enabledPlugins: [],
        attachments: atts.map((a) => ({ kind: a.kind || 'file', path: a.path, name: a.name })),
        prompt,
      },
      harnessSpecific || {},
    ),
  }).catch((err) => {
    const msg = err instanceof Error ? err.message : String(err || '发送失败');
    const list = dsbRuntime.externalMessages[hId] || [];
    const idx = list.findIndex((m) => m.id === asstMsgId);
    if (idx >= 0) {
      list[idx] = { ...list[idx], text: '⚠️ 发送失败：' + msg, streaming: false, isError: true };
      dsbRuntime.externalMessages[hId] = list.slice();
    }
    if (dsbRuntime.externalActiveTurn && dsbRuntime.externalActiveTurn.turnId === turnId) {
      dsbRuntime.externalActiveTurn = null;
    }
    restoreOnSendFailure(msg);
    notifyBarRuntime();
  });
  return true;
}

function handleIncomingHarnessEvent(evt) {
  if (!evt || typeof evt !== 'object') return;
  const hId = normalizeHarnessId(evt.harness);
  if (hId === 'dsh') return;
  const turn = dsbRuntime.externalActiveTurn;
  const list = Array.isArray(dsbRuntime.externalMessages[hId])
    ? dsbRuntime.externalMessages[hId].slice()
    : [];
  const expectedMsgId = evt.turnId ? 'ext-a-' + evt.turnId : '';
  const targetId = turn && turn.turnId === evt.turnId ? turn.assistantMsgId : expectedMsgId;
  const idx = targetId ? list.findIndex((m) => m.id === targetId) : -1;

  if (evt.sessionId) {
    const curSel = dsbRuntime.externalSelection[hId] || {};
    if (curSel.sessionId !== evt.sessionId) {
      dsbRuntime.externalSelection[hId] = { ...curSel, sessionId: evt.sessionId };
    }
  }

  if (evt.kind === 'delta' && typeof evt.text === 'string' && idx >= 0) {
    list[idx] = { ...list[idx], text: (list[idx].text || '') + evt.text, streaming: true };
    dsbRuntime.externalMessages[hId] = list;
    if (turn && turn.turnId === evt.turnId) {
      turn.toolNote = '';
      if (evt.text.trim()) turn.receivedAssistantText = true;
    }
  } else if (evt.kind === 'replace' && typeof evt.text === 'string' && idx >= 0) {
    list[idx] = { ...list[idx], text: evt.text, streaming: true };
    dsbRuntime.externalMessages[hId] = list;
    if (turn && turn.turnId === evt.turnId && evt.text.trim()) {
      turn.receivedAssistantText = true;
    }
  } else if (evt.kind === 'tool' && typeof evt.toolName === 'string') {
    if (turn && turn.turnId === evt.turnId) {
      turn.toolNote = evt.toolName;
    }
  } else if (evt.kind === 'completed') {
    if (idx >= 0) {
      const fallbackText =
        !list[idx].text && typeof evt.text === 'string' && evt.text ? evt.text : list[idx].text;
      list[idx] = { ...list[idx], text: fallbackText || '（已完成）', streaming: false };
      dsbRuntime.externalMessages[hId] = list;
    }
    if (turn && turn.turnId === evt.turnId) {
      dsbRuntime.externalActiveTurn = null;
    }
    dsbRuntime.externalSendError = '';
    const doneSid = evt.sessionId || (dsbRuntime.externalSelection[hId] && dsbRuntime.externalSelection[hId].sessionId) || '';
    if (doneSid) {
      delete dsbRuntime.sessionDirtyKeys[hId + ':' + doneSid];
    }
    setTimeout(() => {
      refreshExternalCatalog(hId, false);
      if (doneSid) syncExternalSessionState(hId, doneSid, true);
    }, 450);
  } else if (evt.kind === 'error') {
    const errText = evt.error || '执行出错';
    if (idx >= 0) {
      const prefix = list[idx].text ? list[idx].text + '\n\n' : '';
      list[idx] = {
        ...list[idx],
        text: prefix + '⚠️ ' + errText,
        streaming: false,
        isError: true,
      };
      dsbRuntime.externalMessages[hId] = list;
    } else {
      list.push({
        id: expectedMsgId || 'ext-err-' + Date.now(),
        role: 'assistant',
        text: '⚠️ ' + errText,
        streaming: false,
        isError: true,
      });
      dsbRuntime.externalMessages[hId] = list;
    }
    dsbRuntime.externalSendError = errText;
    if (
      turn &&
      turn.turnId === evt.turnId &&
      !turn.receivedAssistantText &&
      turn.rollback &&
      !(dsbRuntime.extDraftText || '').trim() &&
      (!Array.isArray(dsbRuntime.extCapsules) || dsbRuntime.extCapsules.length === 0) &&
      (!Array.isArray(dsbRuntime.extAttachments) || dsbRuntime.extAttachments.length === 0)
    ) {
      dsbRuntime.extDraftText = turn.rollback.draft || '';
      dsbRuntime.extCapsules = Array.isArray(turn.rollback.capsules)
        ? turn.rollback.capsules.slice()
        : [];
      dsbRuntime.extAttachments = Array.isArray(turn.rollback.attachments)
        ? turn.rollback.attachments.slice()
        : [];
      sendBarReport({
        kind: 'capsules',
        capsuleCount: dsbRuntime.extCapsules.length,
        swEntityKeys: collectActiveSwEntityKeys(),
        afterInsert: false,
      });
    }
    if (turn && turn.turnId === evt.turnId) {
      dsbRuntime.externalActiveTurn = null;
    }
    if (hId === dsbRuntime.activeHarness) {
      dsbRuntime.unreadCount = Math.max(1, dsbRuntime.unreadCount || 0);
      dsbRuntime.unreadPreview = '⚠️ 发送失败：' + String(errText).slice(0, 60);
    }
  }

  notifyBarRuntime();
}

function refreshAppSettings() {
  if (!IS_BAR) return Promise.resolve(null);
  return tauriInvoke('app_get_settings')
    .then((snap) => {
      if (snap && typeof snap === 'object') {
        dsbRuntime.appSettings = snap;
        notifyBarRuntime();
      }
      return snap;
    })
    .catch(() => null);
}

function updateAppSettings(patch) {
  if (!IS_BAR || !patch) return Promise.resolve(null);
  // Optimistic local preview when running inside browser-only test mode
  if (dsbRuntime.appSettings && dsbRuntime.appSettings.config) {
    dsbRuntime.appSettings = {
      ...dsbRuntime.appSettings,
      config: { ...dsbRuntime.appSettings.config, ...patch },
    };
    notifyBarRuntime();
  }
  return tauriInvoke('app_update_settings', { patch })
    .then((snap) => {
      if (snap && typeof snap === 'object') {
        dsbRuntime.appSettings = snap;
        notifyBarRuntime();
      }
      return snap;
    })
    .catch(() => dsbRuntime.appSettings);
}

function handleIncomingHarnessSync(evt) {
  if (!evt || typeof evt !== 'object') return;
  const hId = normalizeHarnessId(evt.harness);
  if (hId === 'dsh') return;
  // Never overwrite an active turn currently streaming from the floating bar itself
  if (dsbRuntime.externalActiveTurn && dsbRuntime.externalActiveTurn.harness === hId) {
    return;
  }

  const incomingSessionId = String(evt.sessionId || '');
  const curSel = dsbRuntime.externalSelection[hId] || {};
  const incomingMsgs = Array.isArray(evt.messages) ? evt.messages : [];
  let latestReply = null;
  for (let i = incomingMsgs.length - 1; i >= 0; i -= 1) {
    const m = incomingMsgs[i];
    if (m && m.role === 'assistant' && String(m.text || '').trim().length > 0) {
      latestReply = m;
      break;
    }
  }

  const sessionKey = incomingSessionId ? hId + ':' + incomingSessionId : '';
  const replySig = latestReply
    ? String(latestReply.id || '') + ':' + String(latestReply.text || '').trim().length
    : '';
  const prevSig = sessionKey ? dsbRuntime.lastSeenSessionReply[sessionKey] : undefined;
  if (sessionKey && replySig) {
    dsbRuntime.lastSeenSessionReply[sessionKey] = replySig;
  }
  const isNewReply = Boolean(
    replySig && prevSig !== undefined && prevSig !== replySig,
  );

  const catHit = incomingSessionId ? findExternalCatalogEntry(hId, incomingSessionId) : null;
  const sameSession = Boolean(curSel.sessionId && incomingSessionId && curSel.sessionId === incomingSessionId);
  const isAuthTitle = Boolean(evt.titleAuthoritative && evt.title);
  const resolvedTitle = isAuthTitle
    ? evt.title
    : (catHit && catHit.conv && catHit.conv.title) ||
      (sameSession && curSel.title ? curSel.title : evt.title) ||
      HARNESS_META[hId].label;
  const resolvedWsId =
    (isAuthTitle && evt.workspaceId) ||
    (catHit && catHit.ws && catHit.ws.workspaceId) ||
    (evt.workspaceId && evt.workspaceId !== 'default-cli-project' ? evt.workspaceId : '') ||
    (sameSession ? curSel.workspaceId : '') ||
    evt.workspaceId ||
    '';
  const resolvedWsPath =
    (isAuthTitle && evt.workspacePath) ||
    (catHit && catHit.ws && (catHit.ws.path || (catHit.conv && catHit.conv.cwd))) ||
    evt.workspacePath ||
    (sameSession ? curSel.workspacePath : '') ||
    '';

  const barIsHiddenOrLip =
    dsbRuntime.barVisibility === 'hidden' || dsbRuntime.barVisibility === 'lip';
  const hasDraftOrCapsules = Boolean(
    (dsbRuntime.extDraftText || '').trim() ||
      (dsbRuntime.extCapsules || []).length > 0 ||
      (dsbRuntime.liveDraft || '').trim(),
  );

  let activeSessionUpdated = false;
  if (sameSession) {
    // Bound conversation updated -> refresh its messages in-place (never jumps away).
    // Unread toast notifications are handled exclusively by CardDock for the currently
    // selected Harness + Conversation (never cross-harness or cross-conversation).
    dsbRuntime.externalSelection[hId] = {
      workspaceId: resolvedWsId,
      workspacePath: resolvedWsPath,
      sessionId: incomingSessionId,
      title: resolvedTitle,
    };
    if (incomingMsgs.length > 0) {
      // Preserve trailing local send-error notice if sync hasn't caught up yet
      const existing = Array.isArray(dsbRuntime.externalMessages[hId])
        ? dsbRuntime.externalMessages[hId]
        : [];
      const tailErr =
        existing.length > 0 && existing[existing.length - 1] && existing[existing.length - 1].isError
          ? existing[existing.length - 1]
          : null;
      dsbRuntime.externalMessages[hId] =
        tailErr && incomingMsgs.length <= existing.length - 1
          ? incomingMsgs.concat([tailErr])
          : incomingMsgs;
    }
    activeSessionUpdated = hId === dsbRuntime.activeHarness;
    syncExternalSessionState(hId, incomingSessionId, false);
  } else if (!curSel.sessionId && incomingSessionId && barIsHiddenOrLip && !hasDraftOrCapsules) {
    // Brand-new unbound harness bootstrap only when Bar is not actively being used
    dsbRuntime.externalSelection[hId] = {
      workspaceId: resolvedWsId,
      workspacePath: resolvedWsPath,
      sessionId: incomingSessionId,
      title: resolvedTitle,
    };
    if (incomingMsgs.length > 0) {
      // Preserve trailing local send-error notice if sync hasn't caught up yet
      const existing = Array.isArray(dsbRuntime.externalMessages[hId])
        ? dsbRuntime.externalMessages[hId]
        : [];
      const tailErr =
        existing.length > 0 && existing[existing.length - 1] && existing[existing.length - 1].isError
          ? existing[existing.length - 1]
          : null;
      dsbRuntime.externalMessages[hId] =
        tailErr && incomingMsgs.length <= existing.length - 1
          ? incomingMsgs.concat([tailErr])
          : incomingMsgs;
    }
    activeSessionUpdated = hId === dsbRuntime.activeHarness;
    syncExternalSessionState(hId, incomingSessionId, true);
  }

  dsbRuntime.liveSyncMeta[hId] = {
    source: evt.source || 'watch',
    running: Boolean(evt.running),
    updatedAt: evt.updatedAtMs || Date.now(),
  };

  const now = Date.now();
  if (now - (dsbRuntime.lastSyncCatalogRefresh[hId] || 0) > 2500) {
    dsbRuntime.lastSyncCatalogRefresh[hId] = now;
    refreshExternalCatalog(hId, false);
  }

  notifyBarRuntime();
  if (activeSessionUpdated && typeof dsbRuntime.scheduleLayoutReport === 'function') {
    dsbRuntime.scheduleLayoutReport(220);
  }
}

