// v0.6.0 Unified DSH Dual-Theme Palettes — eyedropped directly from each official logo
// (DeepSeek #4D6BFE, Google Antigravity 4-color prism, OpenAI Codex Monochrome + Periwinkle Violet, Anthropic Claude #D97757)
const UNIFIED_PALETTES = {
  dsh: {
    dotStyle: '#4D6BFE',
    dark: {
      surface: '#212327', popover: '#1A1C20', drawer: '#181A1E',
      border: 'rgba(77, 107, 254, 0.26)', subtleBorder: 'rgba(255, 255, 255, 0.08)',
      topBar: 'linear-gradient(90deg, transparent 4%, #4D6BFE 50%, transparent 96%)',
      fg: '#ECEEF2', muted: '#8A90A0', accent: '#637EFF', accentBg: 'rgba(77, 107, 254, 0.18)',
      accentBorder: 'rgba(99, 126, 255, 0.42)', sendBg: '#4D6BFE', sendFg: '#FFFFFF', sendRing: 'none',
      permDanger: '#7A93FF', plusBg: 'rgba(255, 255, 255, 0.08)', rowHover: 'rgba(255, 255, 255, 0.075)',
    },
    light: {
      surface: '#FFFFFF', popover: '#FFFFFF', drawer: '#F8FAFF',
      border: 'rgba(77, 107, 254, 0.26)', subtleBorder: 'rgba(15, 23, 42, 0.08)',
      topBar: 'linear-gradient(90deg, transparent 4%, #4D6BFE 50%, transparent 96%)',
      fg: '#111827', muted: '#6B7280', accent: '#4D6BFE', accentBg: 'rgba(77, 107, 254, 0.10)',
      accentBorder: 'rgba(77, 107, 254, 0.36)', sendBg: '#4D6BFE', sendFg: '#FFFFFF', sendRing: 'none',
      permDanger: '#4D6BFE', plusBg: 'rgba(77, 107, 254, 0.08)', rowHover: 'rgba(15, 23, 42, 0.055)',
    },
  },
  antigravity: {
    dotStyle: 'conic-gradient(#4285F4 0 90deg, #34A853 90deg 180deg, #FBBC04 180deg 270deg, #EA4335 270deg 360deg)',
    dark: {
      surface: '#16171A', popover: '#121316', drawer: '#131418',
      border: 'rgba(255, 255, 255, 0.11)', subtleBorder: 'rgba(255, 255, 255, 0.08)',
      topBar: 'linear-gradient(90deg, #4285F4 0%, #34A853 34%, #FBBC04 68%, #EA4335 100%)',
      fg: '#F1F3F4', muted: '#8E929B', accent: '#4285F4', accentBg: 'rgba(66, 133, 244, 0.15)',
      accentBorder: 'rgba(66, 133, 244, 0.42)', sendBg: '#FFFFFF', sendFg: '#111215',
      sendRing: '0 0 0 1.5px rgba(66, 133, 244, 0.55), 0 0 10px rgba(52, 168, 83, 0.25)',
      permDanger: '#FBBC04', plusBg: 'rgba(255, 255, 255, 0.08)', rowHover: 'rgba(255, 255, 255, 0.075)',
    },
    light: {
      surface: '#FFFFFF', popover: '#FFFFFF', drawer: '#FAFBFD',
      border: 'rgba(15, 23, 42, 0.12)', subtleBorder: 'rgba(15, 23, 42, 0.08)',
      topBar: 'linear-gradient(90deg, #4285F4 0%, #34A853 34%, #FBBC04 68%, #EA4335 100%)',
      fg: '#111827', muted: '#5F6368', accent: '#1A73E8', accentBg: 'rgba(26, 115, 232, 0.10)',
      accentBorder: 'rgba(26, 115, 232, 0.36)', sendBg: '#1A73E8', sendFg: '#FFFFFF',
      sendRing: '0 0 0 2px rgba(52, 168, 83, 0.22)',
      permDanger: '#D97706', plusBg: 'rgba(15, 23, 42, 0.06)', rowHover: 'rgba(15, 23, 42, 0.055)',
    },
  },
  codex: {
    dotStyle: 'linear-gradient(135deg, #7A9DFF, #B1A7FF)',
    dark: {
      surface: '#1D1D20', popover: '#161619', drawer: '#151518',
      border: 'rgba(177, 167, 255, 0.22)', subtleBorder: 'rgba(255, 255, 255, 0.08)',
      topBar: 'linear-gradient(90deg, transparent 4%, #7A9DFF 38%, #B1A7FF 65%, transparent 96%)',
      fg: '#F4F4F5', muted: '#8F8F99', accent: '#A39BFF', accentBg: 'rgba(163, 155, 255, 0.16)',
      accentBorder: 'rgba(177, 167, 255, 0.42)', sendBg: '#FFFFFF', sendFg: '#0D0D0D',
      sendRing: '0 0 0 1px rgba(177, 167, 255, 0.38)',
      permDanger: '#B1A7FF', plusBg: 'rgba(255, 255, 255, 0.08)', rowHover: 'rgba(255, 255, 255, 0.075)',
    },
    light: {
      surface: '#FFFFFF', popover: '#FFFFFF', drawer: '#FAFAFE',
      border: 'rgba(17, 24, 39, 0.14)', subtleBorder: 'rgba(15, 23, 42, 0.08)',
      topBar: 'linear-gradient(90deg, transparent 4%, #5B63F6 38%, #9B8CFF 65%, transparent 96%)',
      fg: '#0D0D0D', muted: '#6B7280', accent: '#5850EC', accentBg: 'rgba(88, 80, 236, 0.10)',
      accentBorder: 'rgba(88, 80, 236, 0.36)', sendBg: '#0D0D0D', sendFg: '#FFFFFF', sendRing: 'none',
      permDanger: '#5850EC', plusBg: 'rgba(15, 23, 42, 0.06)', rowHover: 'rgba(15, 23, 42, 0.055)',
    },
  },
  claude_code: {
    dotStyle: '#D97757',
    dark: {
      surface: '#23211E', popover: '#1C1B19', drawer: '#1A1917',
      border: 'rgba(217, 119, 87, 0.26)', subtleBorder: 'rgba(255, 248, 240, 0.08)',
      topBar: 'linear-gradient(90deg, transparent 4%, #D97757 50%, transparent 96%)',
      fg: '#F2EEE8', muted: '#9C948A', accent: '#D97757', accentBg: 'rgba(217, 119, 87, 0.16)',
      accentBorder: 'rgba(217, 119, 87, 0.44)', sendBg: '#D97757', sendFg: '#FFFFFF', sendRing: 'none',
      permDanger: '#E8926E', plusBg: 'rgba(255, 248, 240, 0.08)', rowHover: 'rgba(255, 248, 240, 0.075)',
    },
    light: {
      surface: '#FAF9F5', popover: '#FFFDF9', drawer: '#F5F2EA',
      border: 'rgba(217, 119, 87, 0.28)', subtleBorder: 'rgba(120, 90, 70, 0.10)',
      topBar: 'linear-gradient(90deg, transparent 4%, #D97757 50%, transparent 96%)',
      fg: '#1F1E1D', muted: '#78716C', accent: '#C45D3B', accentBg: 'rgba(217, 119, 87, 0.12)',
      accentBorder: 'rgba(196, 93, 59, 0.38)', sendBg: '#D97757', sendFg: '#FFFFFF', sendRing: 'none',
      permDanger: '#C45D3B', plusBg: 'rgba(120, 90, 70, 0.08)', rowHover: 'rgba(120, 90, 70, 0.065)',
    },
  },
};

// Shared crisp SVG icons exposed to every drop-in harness module (`harnesses/<id>/ui.js`)
const HARNESS_SDK_ICONS = {
  plus: (sz = 16, col = '#a1a1aa') =>
    h(
      'svg',
      { width: sz, height: sz, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 2, strokeLinecap: 'round' },
      h('path', { d: 'M12 5v14M5 12h14' }),
    ),
  chevronDown: (col = 'rgba(255,255,255,0.45)') =>
    h(
      'svg',
      { width: 11, height: 11, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 2.2, strokeLinecap: 'round', strokeLinejoin: 'round' },
      h('path', { d: 'M6 9l6 6 6-6' }),
    ),
  shield: (col = '#a1a1aa') =>
    h(
      'svg',
      { width: 13, height: 13, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 1.9, strokeLinecap: 'round', strokeLinejoin: 'round' },
      h('path', { d: 'M12 3l7 3v5c0 5-3.2 8.8-7 10-3.8-1.2-7-5-7-10V6l7-3z' }),
      h('path', { d: 'M12 8v4M12 16h.01' }),
    ),
  laptop: (col = '#d4d4d8') =>
    h(
      'svg',
      { width: 14, height: 14, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 1.8, strokeLinecap: 'round', strokeLinejoin: 'round' },
      h('rect', { x: 4, y: 5, width: 16, height: 11, rx: 1.8 }),
      h('path', { d: 'M2 19h20' }),
    ),
  folderOutline: (col = '#d4d4d8') =>
    h(
      'svg',
      { width: 14, height: 14, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 1.8, strokeLinecap: 'round', strokeLinejoin: 'round' },
      h('path', { d: 'M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z' }),
    ),
  arrowUp: (col = '#111214') =>
    h(
      'svg',
      { width: 14, height: 14, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 2.3, strokeLinecap: 'round', strokeLinejoin: 'round' },
      h('path', { d: 'M12 19V5M6 11l6-6 6 6' }),
    ),
  claudeSpark: (sz = 14, col = '#D97757') =>
    h(
      'svg',
      { width: sz, height: sz, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 2, strokeLinecap: 'round' },
      h('path', { d: 'M12 2.5v19M2.5 12h19M5.2 5.2l13.6 13.6M18.8 5.2L5.2 18.8' }),
      h('circle', { cx: 12, cy: 12, r: 2.2, fill: col, stroke: 'none' }),
    ),
  search: (sz = 12, col = 'rgba(229,231,235,0.68)') =>
    h(
      'svg',
      { width: sz, height: sz, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 2.1, strokeLinecap: 'round', strokeLinejoin: 'round' },
      h('circle', { cx: 11, cy: 11, r: 7 }),
      h('path', { d: 'M20 20l-3.5-3.5' }),
    ),
};

// Auto-populated at build time by `apps/desktop/src-tauri/build.rs` from `harnesses/<id>/{harness.json,ui.js}`.
// Contributors only add a new folder under `harnesses/` — zero edits needed here.
const __ATP_BUILTIN_HARNESS_MODULES__ = /*__ATP_HARNESS_MODULES__*/ [];

const FALLBACK_DSH_MANIFEST = {
  id: 'dsh',
  order: 10,
  aliases: ['deepseek'],
  label: 'DeepSeek',
  fullLabel: 'DeepSeek Harness',
  badge: 'DS',
  syncHint: '双向实时同步',
  placeholder: '',
  accent: '#60a5fa',
  accentBg: 'rgba(59, 130, 246, 0.16)',
  accentBorder: 'rgba(96, 165, 250, 0.42)',
  surface: 'var(--dsw-alias-bg-layer-2, #27272a)',
  border: 'var(--dsw-alias-border-l2, rgba(128,128,128,0.22))',
  useNativeComposer: true,
  cornerRadius: CARD_RADIUS_PX,
  fallbackSlabHeight: 158,
  chipIconStyle: 'folder',
  newWorkspaceLabel: '新建工作区…',
  detailButtonTitle: '点击在浏览器打开当前 DeepSeek Harness 对话',
  emptyHistoryHint: '暂无历史消息 — 在下方输入框发消息后，对话流将在此实时展开',
  emptyLipHint: '暂无消息 · 双击 Ctrl 唤出输入框',
};

const HARNESS_SDK = { h, icons: HARNESS_SDK_ICONS };
const HARNESS_ORDER = [];
const HARNESS_META = {};
const HARNESS_ADAPTERS = {};
const HARNESS_ALIAS_MAP = {};

function registerHarnessModule(entry) {
  if (!entry || !entry.manifest || !entry.manifest.id) return;
  const raw = entry.manifest;
  const id = String(raw.id).trim().toLowerCase();
  if (!id || HARNESS_META[id]) return;
  const meta = {
    id,
    order: Number.isFinite(raw.order) ? raw.order : 500,
    aliases: Array.isArray(raw.aliases) ? raw.aliases : [],
    label: raw.label || id,
    fullLabel: raw.fullLabel || raw.label || id,
    badge: raw.badge || id.slice(0, 2).toUpperCase(),
    syncHint: raw.syncHint || '本地对话实时同步',
    placeholder: raw.placeholder || '输入消息，支持按 Ctrl 圈选屏幕元素…',
    accent: raw.accent || '#60a5fa',
    accentBg: raw.accentBg || 'rgba(59, 130, 246, 0.16)',
    accentBorder: raw.accentBorder || 'rgba(96, 165, 250, 0.42)',
    surface: raw.surface || '#1e1e22',
    border: raw.border || 'rgba(255, 255, 255, 0.12)',
    useNativeComposer: Boolean(raw.useNativeComposer),
    cornerRadius: Number.isFinite(raw.cornerRadius) ? raw.cornerRadius : 18,
    fallbackSlabHeight: Number.isFinite(raw.fallbackSlabHeight) ? raw.fallbackSlabHeight : 168,
    chipIconStyle: raw.chipIconStyle || 'dot',
    newWorkspaceLabel: raw.newWorkspaceLabel || '新建工作区…',
    detailButtonTitle: raw.detailButtonTitle || ('点击将 ' + (raw.label || id) + ' 拉到最前并展示这个对话'),
    emptyHistoryHint: raw.emptyHistoryHint || ((raw.fullLabel || raw.label || id) + ' 已就绪 — 发消息后对话流将在此实时展开'),
    emptyLipHint: raw.emptyLipHint || ((raw.label || id) + ' 已就绪 · 双击 Ctrl 唤出输入框'),
    defaultSelection: raw.defaultSelection || {
      workspaceId: '',
      workspacePath: '',
      sessionId: '',
      title: (raw.label || id) + ' · 新对话',
    },
  };
  HARNESS_ORDER.push(id);
  HARNESS_META[id] = meta;
  HARNESS_ALIAS_MAP[id] = id;
  for (const alias of meta.aliases) {
    const key = String(alias || '').trim().toLowerCase();
    if (key) HARNESS_ALIAS_MAP[key] = id;
  }
  let adapter = {};
  if (typeof entry.initUi === 'function') {
    try {
      adapter = entry.initUi(HARNESS_SDK) || {};
    } catch {}
  }
  HARNESS_ADAPTERS[id] = adapter;
}

if (Array.isArray(__ATP_BUILTIN_HARNESS_MODULES__) && __ATP_BUILTIN_HARNESS_MODULES__.length > 0) {
  for (const mod of __ATP_BUILTIN_HARNESS_MODULES__) {
    registerHarnessModule(mod);
  }
}
if (!HARNESS_META.dsh) {
  registerHarnessModule({ manifest: FALLBACK_DSH_MANIFEST, initUi: null });
}

function getHarnessMeta(id) {
  return HARNESS_META[id] || HARNESS_META.dsh || FALLBACK_DSH_MANIFEST;
}

function getHarnessAdapter(id) {
  return HARNESS_ADAPTERS[id] || {};
}

function hexToRgba(hex, alpha) {
  const clean = String(hex || '#38bdf8').replace('#', '').trim();
  const full = clean.length === 3 ? clean.split('').map((c) => c + c).join('') : clean;
  if (full.length !== 6) return 'rgba(56, 189, 248, ' + alpha + ')';
  const n = parseInt(full, 16);
  if (!Number.isFinite(n)) return 'rgba(56, 189, 248, ' + alpha + ')';
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  return 'rgba(' + r + ', ' + g + ', ' + b + ', ' + alpha + ')';
}

function buildDynamicPalette(accentHex) {
  const accent = accentHex && accentHex.startsWith('#') ? accentHex : '#38bdf8';
  return {
    dotStyle: accent,
    dark: {
      surface: '#1b1d22',
      popover: '#1f2228',
      drawer: '#181a1f',
      border: 'rgba(255, 255, 255, 0.11)',
      subtleBorder: 'rgba(255, 255, 255, 0.08)',
      fg: '#eceff4',
      muted: '#9ca3af',
      hoverBg: 'rgba(255, 255, 255, 0.06)',
      plusBg: 'rgba(255, 255, 255, 0.08)',
      accent,
      accentBg: hexToRgba(accent, 0.16),
      accentBorder: hexToRgba(accent, 0.42),
      topBar: accent,
      sendBg: accent,
      sendFg: '#ffffff',
      sendRing: '0 2px 10px ' + hexToRgba(accent, 0.35),
      permDanger: '#fb923c',
    },
    light: {
      surface: '#ffffff',
      popover: '#ffffff',
      drawer: '#f8fafc',
      border: 'rgba(15, 23, 42, 0.11)',
      subtleBorder: 'rgba(15, 23, 42, 0.08)',
      fg: '#0f172a',
      muted: '#64748b',
      hoverBg: 'rgba(15, 23, 42, 0.05)',
      plusBg: 'rgba(15, 23, 42, 0.06)',
      accent,
      accentBg: hexToRgba(accent, 0.12),
      accentBorder: hexToRgba(accent, 0.34),
      topBar: accent,
      sendBg: accent,
      sendFg: '#ffffff',
      sendRing: '0 2px 10px ' + hexToRgba(accent, 0.28),
      permDanger: '#ea580c',
    },
  };
}

const BUILTIN_STATIC_HARNESS_IDS = new Set(HARNESS_ORDER.slice());

function ensureExternalRuntimeSlots(id) {
  if (!dsbRuntime || id === 'dsh') return;
  if (!Object.prototype.hasOwnProperty.call(dsbRuntime.externalCatalogs, id)) {
    dsbRuntime.externalCatalogs[id] = null;
  }
  if (!Object.prototype.hasOwnProperty.call(dsbRuntime.externalSelection, id)) {
    const meta = getHarnessMeta(id);
    dsbRuntime.externalSelection[id] = readStoredExtSelection(id, meta.defaultSelection);
  }
  if (!Object.prototype.hasOwnProperty.call(dsbRuntime.externalMessages, id)) {
    dsbRuntime.externalMessages[id] = [];
  }
  if (!Object.prototype.hasOwnProperty.call(dsbRuntime.externalLoading, id)) {
    dsbRuntime.externalLoading[id] = false;
  }
  if (!Object.prototype.hasOwnProperty.call(dsbRuntime.liveSyncMeta, id)) {
    dsbRuntime.liveSyncMeta[id] = null;
  }
  if (!Object.prototype.hasOwnProperty.call(dsbRuntime.lastSyncCatalogRefresh, id)) {
    dsbRuntime.lastSyncCatalogRefresh[id] = 0;
  }
  if (!Object.prototype.hasOwnProperty.call(dsbRuntime.harnessPrefs, id)) {
    dsbRuntime.harnessPrefs[id] = readStoredHarnessPrefs(id);
  }
}

function syncDynamicHarnessesFromAvailability(list) {
  if (!Array.isArray(list)) return;
  const incomingDynamicIds = new Set();
  for (const item of list) {
    if (!item || !item.id) continue;
    const id = String(item.id).trim().toLowerCase();
    if (!id || BUILTIN_STATIC_HARNESS_IDS.has(id)) continue;
    incomingDynamicIds.add(id);
    const isUniversal = id === 'universal' || item.providerKind === 'universal';
    const accent = item.accent && String(item.accent).startsWith('#') ? item.accent : (isUniversal ? '#06b6d4' : '#38bdf8');
    const label = item.label || id;
    const meta = {
      id,
      order: isUniversal ? 999 : (item.custom ? 700 : 600),
      aliases: [id],
      label,
      fullLabel: label,
      badge: item.badge || (isUniversal ? '投' : 'AC'),
      providerKind: item.providerKind || (isUniversal ? 'universal' : 'acp'),
      custom: Boolean(item.custom),
      syncHint: item.syncNote || item.subtitle || (isUniversal ? '一键投递到任意焦点窗口' : 'ACP 标准协议实时握手'),
      placeholder: isUniversal
        ? '输入或圈选内容，回车一键投递到目标窗口输入框…'
        : '向 ' + label + ' 发送消息（ACP 标准协议，支持 Ctrl 圈选屏幕）…',
      accent,
      accentBg: hexToRgba(accent, 0.16),
      accentBorder: hexToRgba(accent, 0.42),
      surface: '#1b1d22',
      border: 'rgba(255, 255, 255, 0.11)',
      useNativeComposer: false,
      cornerRadius: 18,
      fallbackSlabHeight: 168,
      chipIconStyle: 'dot',
      newWorkspaceLabel: isUniversal ? '刷新目标窗口' : '选择工作区目录…',
      detailButtonTitle: isUniversal ? '查看当前目标窗口' : ('ACP 标准会话 · ' + label),
      emptyHistoryHint: isUniversal
        ? '万能投递模式 — 圈选屏幕元素或写好 Prompt，回车直接粘贴到上一个聚焦的聊天窗口'
        : (label + ' (ACP) 已就绪 — 模型与权限由 Agent 实时下发'),
      emptyLipHint: label + ' 已就绪 · 双击 Ctrl 唤出输入框',
      defaultSelection: {
        workspaceId: isUniversal ? 'universal-focus' : '',
        workspacePath: '',
        sessionId: isUniversal ? 'universal-active' : '',
        title: isUniversal ? '当前焦点窗口' : (label + ' · 新对话'),
      },
    };
    if (!HARNESS_META[id]) {
      HARNESS_ORDER.push(id);
    }
    HARNESS_META[id] = meta;
    HARNESS_ALIAS_MAP[id] = id;
    UNIFIED_PALETTES[id] = buildDynamicPalette(accent);
    ensureExternalRuntimeSlots(id);
  }
  // Prune removed custom ACP harnesses
  for (let i = HARNESS_ORDER.length - 1; i >= 0; i -= 1) {
    const hid = HARNESS_ORDER[i];
    if (!BUILTIN_STATIC_HARNESS_IDS.has(hid) && !incomingDynamicIds.has(hid)) {
      HARNESS_ORDER.splice(i, 1);
      delete HARNESS_META[hid];
      delete HARNESS_ALIAS_MAP[hid];
      if (dsbRuntime && dsbRuntime.activeHarness === hid) {
        dsbRuntime.activeHarness = 'dsh';
      }
    }
  }
}

function normalizeHarnessId(raw) {
  const s = String(raw || '').trim().toLowerCase();
  if (s && HARNESS_ALIAS_MAP[s]) return HARNESS_ALIAS_MAP[s];
  if (s === 'universal' || s.startsWith('acp:')) return s;
  return HARNESS_ORDER[0] || 'dsh';
}

