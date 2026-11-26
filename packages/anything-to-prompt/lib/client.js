// @local/anything-to-prompt/lib/client.js — Anything-to-Prompt Ambient Floating Bar + Session Picker + Tauri Bridge (v1.0.0)
// Only activates inside Anything-to-Prompt's own floating window (?dsh_bar_clone=1); never injects into DSH's normal WebUI.

window.__ModuleLoader__.load({
  id: '@local/anything-to-prompt',
  factory(require) {
    const React = require('react');
    const {
      useCallback,
      useEffect,
      useLayoutEffect,
      useMemo,
      useRef,
      useState,
    } = React;
    const h = React.createElement;
    const ReactDOM = (() => {
      try {
        return require('react-dom');
      } catch {
        return null;
      }
    })();

    const DSHPrimitives = (() => {
      try {
        return require('@deepseek-ai/dsh-client-ui-primitives');
      } catch {
        return null;
      }
    })();

    const CARD_RADIUS_PX = 24;
    const COLLAPSED_LIP_PX = 56;
    const TUCK_OVERLAP_PX = 0;
    const HOVER_EXPAND_DELAY_MS = 300;
    const DEFAULT_EXPANDED_PX = 280;
    const MIN_EXPANDED_PX = 160;
    const MAX_EXPANDED_PX = 560;
    const DEFAULT_BAR_WIDTH_PX = 760;
    const MIN_BAR_WIDTH_PX = 480;
    const MAX_BAR_WIDTH_PX = 1200;

    const STORAGE_KEY_PINNED = 'dsh_card_dock:pinned';
    const STORAGE_KEY_HEIGHT = 'dsh_card_dock:expanded_height';
    const STORAGE_KEY_AGENT_NAME = 'dsh_card_dock:agent_name';
    const STORAGE_KEY_BAR_WIDTH = 'dsh_card_dock:bar_width';
    const STORAGE_KEY_LAST_SESSION = 'dsh_card_dock:bar_last_session';
    const STORAGE_KEY_ACTIVE_HARNESS = 'dsh_card_dock:active_harness';
    const STORAGE_KEY_CLAUDE_PERM = 'dsh_card_dock:claude_perm_mode';
    const STORAGE_KEY_THEME_PREF = 'dsh_card_dock:theme_pref';

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

    function normalizeHarnessId(raw) {
      const s = String(raw || '').trim().toLowerCase();
      if (s && HARNESS_ALIAS_MAP[s]) return HARNESS_ALIAS_MAP[s];
      return HARNESS_ORDER[0] || 'dsh';
    }

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
      unreadCount: 0,
      unreadPreview: '',
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
      if (hId === 'claude_code' && next.mode) {
        dsbRuntime.claudePermissionMode = next.mode;
        writeStoredString(STORAGE_KEY_CLAUDE_PERM, next.mode);
      }
      notifyBarRuntime();
      return next;
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
            dsbRuntime.harnessAvailability = list;
            notifyBarRuntime();
          }
          return list;
        })
        .catch(() => []);
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
                if (firstConv) loadExternalConversationMessages(hId, firstConv.id);
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
              } else {
                dsbRuntime.externalSelection[hId] = {
                  workspaceId: firstWs.workspaceId || '',
                  workspacePath: firstWs.path || '',
                  sessionId: '',
                  title: (firstWs.label || HARNESS_META[hId].label) + ' · 新对话',
                };
              }
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
      dsbRuntime.activeHarness = norm;
      applyHarnessThemeAttribute(norm);
      if (persist) {
        writeStoredString(STORAGE_KEY_ACTIVE_HARNESS, norm);
        tauriInvoke('harness_set_active', { harness: norm }).catch(() => {});
      }
      if (!getHarnessMeta(norm).useNativeComposer && !skipCatalogRefresh) {
        refreshExternalCatalog(norm, true);
      }
      notifyBarRuntime();
      if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
        dsbRuntime.scheduleLayoutReport(280);
      }
    }

    function selectExternalConversation(harnessId, group, conv) {
      const hId = normalizeHarnessId(harnessId);
      if (hId === 'dsh' || !conv) return;
      dsbRuntime.externalSelection[hId] = {
        workspaceId: group ? group.workspaceId || '' : '',
        workspacePath: (group && group.path) || conv.cwd || '',
        sessionId: conv.id,
        title: conv.title || '新对话',
      };
      writeStoredExtSelection(hId, dsbRuntime.externalSelection[hId]);
      notifyBarRuntime();
      loadExternalConversationMessages(hId, conv.id);
    }

    function startNewExternalConversation(harnessId, group) {
      const hId = normalizeHarnessId(harnessId);
      if (hId === 'dsh') return;
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

    function submitExternalHarnessPrompt(rawText, attachmentsInput) {
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
      dsbRuntime.externalActiveTurn = {
        turnId,
        harness: hId,
        assistantMsgId: asstMsgId,
        toolNote: '',
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
              executionMode: hId === 'antigravity' ? (resolvedMode || 'auto') : null,
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
          list[idx] = { ...list[idx], text: '错误：' + msg, streaming: false };
          dsbRuntime.externalMessages[hId] = list.slice();
        }
        if (dsbRuntime.externalActiveTurn && dsbRuntime.externalActiveTurn.turnId === turnId) {
          dsbRuntime.externalActiveTurn = null;
        }
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
      const targetId = turn && turn.turnId === evt.turnId ? turn.assistantMsgId : '';
      const idx = targetId ? list.findIndex((m) => m.id === targetId) : list.length - 1;

      if (evt.sessionId) {
        const curSel = dsbRuntime.externalSelection[hId] || {};
        if (curSel.sessionId !== evt.sessionId) {
          dsbRuntime.externalSelection[hId] = { ...curSel, sessionId: evt.sessionId };
        }
      }

      if (evt.kind === 'delta' && typeof evt.text === 'string' && idx >= 0) {
        list[idx] = { ...list[idx], text: (list[idx].text || '') + evt.text, streaming: true };
        dsbRuntime.externalMessages[hId] = list;
        if (turn && turn.turnId === evt.turnId) turn.toolNote = '';
      } else if (evt.kind === 'replace' && typeof evt.text === 'string' && idx >= 0) {
        list[idx] = { ...list[idx], text: evt.text, streaming: true };
        dsbRuntime.externalMessages[hId] = list;
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
        setTimeout(() => refreshExternalCatalog(hId, false), 450);
      } else if (evt.kind === 'error') {
        if (idx >= 0) {
          const prefix = list[idx].text ? list[idx].text + '\n\n' : '';
          list[idx] = {
            ...list[idx],
            text: prefix + '⚠️ ' + (evt.error || '执行出错'),
            streaming: false,
          };
          dsbRuntime.externalMessages[hId] = list;
        }
        if (turn && turn.turnId === evt.turnId) {
          dsbRuntime.externalActiveTurn = null;
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

      if (sameSession) {
        // Bound conversation updated -> refresh its messages in-place (never jumps away)
        dsbRuntime.externalSelection[hId] = {
          workspaceId: resolvedWsId,
          workspacePath: resolvedWsPath,
          sessionId: incomingSessionId,
          title: resolvedTitle,
        };
        if (incomingMsgs.length > 0) {
          dsbRuntime.externalMessages[hId] = incomingMsgs;
        }
        // Scheme B: if a genuinely new reply arrives on another harness, record it for the
        // detached floating unread toast above the composer (never pop up the obsolete 56px Lip pill).
        if (hId !== dsbRuntime.activeHarness && isNewReply && latestReply) {
          dsbRuntime.pendingBubbleTarget = {
            harness: hId,
            workspaceId: resolvedWsId,
            workspacePath: resolvedWsPath,
            sessionId: incomingSessionId,
            title: resolvedTitle,
            previewText: stripMarkdownForLip(latestReply.text),
            assistantId: String(latestReply.id || ''),
          };
          dsbRuntime.unreadCount = (dsbRuntime.unreadCount || 0) + 1;
          dsbRuntime.unreadPreview =
            (resolvedTitle ? '[' + resolvedTitle + '] ' : '') +
            stripMarkdownForLip(latestReply.text);
        }
      } else if (!curSel.sessionId && incomingSessionId && barIsHiddenOrLip && !hasDraftOrCapsules) {
        // Brand-new unbound harness bootstrap only when Bar is not actively being used
        dsbRuntime.externalSelection[hId] = {
          workspaceId: resolvedWsId,
          workspacePath: resolvedWsPath,
          sessionId: incomingSessionId,
          title: resolvedTitle,
        };
        if (incomingMsgs.length > 0) {
          dsbRuntime.externalMessages[hId] = incomingMsgs;
        }
      } else if (incomingSessionId && isNewReply && latestReply) {
        // Another conversation replied: never auto-switch active selection in the background,
        // and never pop up the obsolete 56px Lip pill. Surface it in Scheme B's unread toast instead.
        dsbRuntime.pendingBubbleTarget = {
          harness: hId,
          workspaceId: resolvedWsId,
          workspacePath: resolvedWsPath,
          sessionId: incomingSessionId,
          title: resolvedTitle,
          previewText: stripMarkdownForLip(latestReply.text),
          assistantId: String(latestReply.id || ''),
        };
        dsbRuntime.unreadCount = (dsbRuntime.unreadCount || 0) + 1;
        dsbRuntime.unreadPreview =
          (resolvedTitle ? '[' + resolvedTitle + '] ' : '') +
          stripMarkdownForLip(latestReply.text);
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
      if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
        dsbRuntime.scheduleLayoutReport(220);
      }
    }

    function notifyBarRuntime() {
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
              sendBarReport({ kind: 'capsules', capsuleCount: 0, afterInsert: false });
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
            dsbRuntime.extCapsules = [
              ...dsbRuntime.extCapsules,
              {
                id: 'cap-' + Date.now() + '-' + Math.random().toString(36).slice(2, 6),
                summary: capsule.summary || { label: '•', title: '引用', detail: '', body: '' },
                text: capsule.text,
              },
            ];
            notifyBarRuntime();
            focusComposerElement();
            sendBarReport({
              kind: 'capsules',
              capsuleCount: dsbRuntime.extCapsules.length,
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

    const STYLE_ID = 'anything-to-prompt-standalone-styles-v5';
    function ensureCardDockStyles() {
      if (typeof document === 'undefined' || document.getElementById(STYLE_ID)) return;
      const styleEl = document.createElement('style');
      styleEl.id = STYLE_ID;
      styleEl.textContent = [
        '@keyframes dgPopIn {',
        '  0%   { opacity: 0.25; transform: translateY(5px); }',
        '  100% { opacity: 1;    transform: translateY(0); }',
        '}',
        '[data-dg-msg-pop] {',
        '  animation: dgPopIn 0.2s cubic-bezier(0.22, 1, 0.36, 1);',
        '}',
        '[data-dg-scroll]::-webkit-scrollbar { width: 5px; }',
        '[data-dg-scroll]::-webkit-scrollbar-track { background: transparent; }',
        '[data-dg-scroll]::-webkit-scrollbar-thumb {',
        '  background: rgba(128, 128, 128, 0.28);',
        '  border-radius: 999px;',
        '}',
        'html:not([data-dsh-bar-clone="1"]) [data-composer-card] {',
        '  position: relative !important;',
        '  z-index: 2 !important;',
        '  border-radius: 0 0 24px 24px !important;',
        '  box-sizing: border-box !important;',
        '  background: var(--dsw-alias-bg-layer-2, #2c2c2e) !important;',
        '  box-shadow: none !important;',
        '  border: none !important;',
        '}',
        '[data-dsb-card] {',
        '  box-shadow: 0 18px 46px rgba(0, 0, 0, 0.48), 0 0 0 0.5px var(--dsw-alias-border-l2, rgba(255, 255, 255, 0.10)) inset !important;',
        '  border: none !important;',
        '}',
        '[data-dsb-card], [data-dsb-card] *:not([data-dsb-md]):not([data-dsb-md] *) {',
        '  font-family: inherit !important;',
        '  font-weight: inherit !important;',
        '}',
        '[data-dsb-card] *:not([data-dsb-md]):not([data-dsb-md] *) {',
        '  color: inherit !important;',
        '}',
        '[data-width-handle] {',
        '  z-index: 12 !important;',
        '}',
        '@keyframes dsbUnreadPulse {',
        '  0%, 100% { box-shadow: 0 0 0 0 rgba(56, 189, 248, 0.55); opacity: 1; }',
        '  50% { box-shadow: 0 0 0 5px rgba(56, 189, 248, 0); opacity: 0.85; }',
        '}',
        '[data-dsb-drag-handle="1"]:hover > span:first-child {',
        '  background: rgba(226, 232, 240, 0.78) !important;',
        '  width: 46px !important;',
        '}',
        '[data-dsb-drag-handle="1"]:active {',
        '  cursor: grabbing !important;',
        '}',
        '[data-composer-card] span[class*="chip"],',
        '[data-composer-card] [data-quote-chip],',
        '[data-composer-card] [data-ref-chip] {',
        '  max-width: 176px !important;',
        '  overflow: hidden !important;',
        '  text-overflow: ellipsis !important;',
        '  white-space: nowrap !important;',
        '}',
        // =====================================================================
        // ?dsh_bar_clone=1 — Bar Mode (100% Untouched Native DSH Composer + Native Queue Dock!)
        // =====================================================================
        'html[data-dsh-bar-clone="1"],',
        'html[data-dsh-bar-clone="1"] body {',
        '  margin: 0 !important;',
        '  padding: 0 !important;',
        '  width: 100vw !important;',
        '  height: 100vh !important;',
        '  overflow: hidden !important;',
        '  background: transparent !important;',
        '}',
        'html[data-dsh-bar-clone="1"] body *:not(:has([data-composer-seat])):not([data-composer-seat]):not([data-composer-seat] *):not([data-dsb-popup]):not([data-dsb-popup] *):not([role="menu"]):not([role="menu"] *):not([role="listbox"]):not([role="listbox"] *):not([role="dialog"]):not([role="dialog"] *):not([data-radix-popper-content-wrapper]):not([data-radix-popper-content-wrapper] *):not([data-floating-ui-portal]):not([data-floating-ui-portal] *):not([data-popover]):not([data-popover] *):not([data-dsw-popover]):not([data-dsw-popover] *) {',
        '  display: none !important;',
        '}',
        'html[data-dsh-bar-clone="1"] body *:has([data-composer-seat]) {',
        '  margin: 0 !important;',
        '  padding: 0 !important;',
        '  border: none !important;',
        '  width: 100vw !important;',
        '  max-width: 100vw !important;',
        '  height: 100vh !important;',
        '  min-height: 0 !important;',
        '  background: transparent !important;',
        '  overflow: visible !important;',
        '}',
        'html[data-dsh-bar-clone="1"] [data-conversation-scroll] {',
        '  --dsh-composer-side-clearance: 0px !important;',
        '  --dsh-composer-dock-inset: 0px !important;',
        '  --dsh-composer-stack-gap: 0px !important;',
        '  display: flex !important;',
        '  flex-direction: column !important;',
        '  justify-content: flex-end !important;',
        '  height: 100vh !important;',
        '  overflow: visible !important;',
        '  padding: 0 !important;',
        '  background: transparent !important;',
        '}',
        'html[data-dsh-bar-clone="1"] [data-composer-seat] {',
        '  --dsh-composer-side-clearance: 0px !important;',
        '  --dsh-composer-dock-inset: 0px !important;',
        '  --dsh-composer-stack-gap: 0px !important;',
        '  position: fixed !important;',
        '  left: 0 !important;',
        '  right: 0 !important;',
        '  bottom: 0 !important;',
        '  width: 100vw !important;',
        '  max-width: 100vw !important;',
        '  margin: 0 !important;',
        '  padding: 0 !important;',
        '  background: transparent !important;',
        '  z-index: 20 !important;',
        '}',
        'html[data-dsh-bar-clone="1"] [data-composer-seat] > div {',
        '  width: 100vw !important;',
        '  max-width: 100vw !important;',
        '  margin: 0 !important;',
        '  padding: 0 !important;',
        '  gap: 0 !important;',
        '  background: transparent !important;',
        '}',
        'html[data-dsh-bar-clone="1"] [data-composer-seat] *:has([data-composer-card]) {',
        '  width: 100vw !important;',
        '  max-width: 100vw !important;',
        '  margin: 0 !important;',
        '  padding: 0 !important;',
        '  gap: 0 !important;',
        '  align-self: stretch !important;',
        '  transform: none !important;',
        '  left: 0 !important;',
        '}',
        'html[data-dsh-bar-clone="1"] [data-composer-seat] *:has([data-composer-card]) > *:not([data-composer-card]):not(:has([data-composer-card])):not([data-donegate-dock-wrap]):not(:has([data-donegate-dock-wrap])):not([data-dsb-external-composer]):not(:has([data-dsb-external-composer])):not([data-dsb-stack]):not(:has([data-dsb-stack])):not([data-queue-dock]):not(:has([data-queue-dock])):not([data-slot="conversation.input.dock"]):not(:has([data-slot="conversation.input.dock"])):not([data-composer-overlay]):not(:has([data-composer-overlay])):not([data-conversation-composer-overlay]):not(:has([data-conversation-composer-overlay])):not([data-trigger-menu]):not(:has([data-trigger-menu])) {',
        '  display: none !important;',
        '}',
        'html[data-dsh-bar-clone="1"] [data-trigger-menu] {',
        '  max-height: 440px !important;',
        '}',
        'html[data-dsh-bar-clone="1"] [data-composer-seat] [data-slot="conversation.input.dock"],',
        'html[data-dsh-bar-clone="1"] [data-composer-seat] [data-composer-dock],',
        'html[data-dsh-bar-clone="1"] [data-composer-seat] :has(> [data-composer-card]) {',
        '  width: 100vw !important;',
        '  max-width: 100vw !important;',
        '  margin: 0 !important;',
        '  padding: 0 !important;',
        '  gap: 0 !important;',
        '}',
        'html[data-dsh-bar-clone="1"] [data-composer-card] {',
        '  width: 100vw !important;',
        '  max-width: 100vw !important;',
        '  margin: 0 !important;',
        '  box-shadow: none !important;',
        '}',
        'html[data-dsh-bar-clone="1"] [data-dsb-card] {',
        '  box-shadow: 0 0 0 0.5px var(--dsw-alias-border-l2, rgba(128, 128, 128, 0.22)) inset !important;',
        '}',
        'html[data-dsh-bar-clone="1"] [data-width-handle] {',
        '  display: none !important;',
        '}',
        'html[data-dsh-bar-clone="1"] [data-composer-card] ~ *:not([data-composer-overlay]):not(:has([data-composer-overlay])):not([data-conversation-composer-overlay]):not(:has([data-conversation-composer-overlay])):not([data-trigger-menu]):not(:has([data-trigger-menu])) {',
        '  display: none !important;',
        '}',
        'html[data-dsh-bar-clone="1"][data-dsb-mode="lip"] [data-composer-card],',
        'html[data-dsh-bar-clone="1"][data-dsb-mode="lip"] [data-queue-dock] {',
        '  display: none !important;',
        '}',
        'html[data-dsh-bar-clone="1"] [data-dsb-menu-scroll]::-webkit-scrollbar { width: 4px; }',
        'html[data-dsh-bar-clone="1"] [data-dsb-menu-scroll]::-webkit-scrollbar-thumb { background: rgba(128,128,128,0.28); border-radius: 999px; }',
        '[data-dsb-composer-popover="1"]::-webkit-scrollbar { width: 5px; }',
        '[data-dsb-composer-popover="1"]::-webkit-scrollbar-track { background: transparent; }',
        '[data-dsb-composer-popover="1"]::-webkit-scrollbar-thumb { background: rgba(128,128,140,0.32); border-radius: 999px; }',
        '[data-dsb-pop-row="1"]:hover { background: rgba(255, 255, 255, 0.08) !important; }',
        'html[data-dsb-theme="light"] [data-dsb-pop-row="1"]:hover { background: rgba(15, 23, 42, 0.055) !important; }',
        'html[data-dsh-bar-clone="1"][data-dsb-harness]:not([data-dsb-harness="dsh"]) [data-dsb-card] { background: var(--atp-drawer, #181a1e) !important; color: var(--atp-fg, #eceef2) !important; box-shadow: 0 0 0 1px var(--atp-border, rgba(255,255,255,0.1)) inset !important; border-radius: 18px !important; }',
        'html[data-dsb-theme="light"] [data-dsb-ext-input="1"]::placeholder { color: rgba(15, 23, 42, 0.44) !important; }',
        'html[data-dsb-theme="dark"] [data-dsb-ext-input="1"]::placeholder { color: rgba(236, 238, 242, 0.44) !important; }',
        // =====================================================================
        // Multi-Harness Auto-Discovered Styles (`harnesses/<id>/{harness.json,ui.js}`)
        // Hide DSH's native [data-composer-card] whenever an external harness renders
        // its composer ([data-dsb-external-composer]), and append each module's scoped CSS.
        // =====================================================================
        ...HARNESS_ORDER.filter((id) => !getHarnessMeta(id).useNativeComposer).map(
          (id) =>
            'html[data-dsh-bar-clone="1"][data-dsb-harness="' +
            id +
            '"]:has([data-dsb-external-composer]) [data-composer-card] { display: none !important; }',
        ),
        'html[data-dsh-bar-clone="1"][data-dsb-mode="lip"] [data-dsb-external-composer] {',
        '  display: none !important;',
        '}',
        ...HARNESS_ORDER.map((id) => {
          const adapter = getHarnessAdapter(id);
          if (adapter && typeof adapter.css === 'string' && adapter.css.trim()) {
            return adapter.css;
          }
          const m = getHarnessMeta(id);
          if (m.useNativeComposer) return '';
          // Sensible adaptive default card theme for declarative harness.json-only modules
          return [
            'html[data-dsh-bar-clone="1"][data-dsb-harness="' + id + '"] [data-dsb-card] {',
            '  background: ' + m.surface + ' !important;',
            '  box-shadow: 0 0 0 1px ' + m.border + ' inset !important;',
            '  border-radius: ' + m.cornerRadius + 'px ' + m.cornerRadius + 'px 0 0 !important;',
            '  color: #f3f4f6 !important;',
            '}',
            'html[data-dsh-bar-clone="1"][data-dsb-harness="' + id + '"][data-dsb-mode="lip"] [data-dsb-card] {',
            '  border-radius: ' + m.cornerRadius + 'px !important;',
            '}',
            'html[data-dsh-bar-clone="1"][data-dsb-harness="' + id + '"] [data-dsb-divider] {',
            '  display: none !important;',
            '}',
          ].join('\n');
        }).filter(Boolean),
        '[data-dsb-ext-input="1"]::placeholder {',
        '  color: rgba(255, 255, 255, 0.36);',
        '}',
        // =====================================================================
        // Rich Markdown typography inside the expanded history drawer
        // =====================================================================
        '[data-dsb-md] {',
        '  width: 100%;',
        '  font-family: inherit;',
        '  font-size: 13px;',
        '  line-height: 1.62;',
        '  color: inherit;',
        '  word-break: normal;',
        '  overflow-wrap: break-word;',
        '}',
        '[data-dsb-md] > *:first-child { margin-top: 0; }',
        '[data-dsb-md] > *:last-child { margin-bottom: 0; }',
        '[data-dsb-md] p {',
        '  margin: 0 0 8px 0;',
        '  line-height: 1.62;',
        '}',
        '[data-dsb-md] h1, [data-dsb-md] h2, [data-dsb-md] h3, [data-dsb-md] h4 {',
        '  font-weight: 650;',
        '  line-height: 1.36;',
        '  margin: 13px 0 6px 0;',
        '  letter-spacing: -0.01em;',
        '}',
        '[data-dsb-md] h1 {',
        '  font-size: 15.5px;',
        '  padding-bottom: 4px;',
        '  border-bottom: 1px solid color-mix(in srgb, currentColor 14%, transparent);',
        '}',
        '[data-dsb-md] h2 {',
        '  font-size: 14.5px;',
        '  padding-bottom: 3px;',
        '  border-bottom: 1px solid color-mix(in srgb, currentColor 11%, transparent);',
        '}',
        '[data-dsb-md] h3 { font-size: 13.5px; }',
        '[data-dsb-md] h4 { font-size: 13px; opacity: 0.92; }',
        '[data-dsb-md] strong {',
        '  font-weight: 650;',
        '}',
        '[data-dsb-md] em { font-style: italic; }',
        '[data-dsb-md] del { text-decoration: line-through; opacity: 0.65; }',
        '[data-dsb-md] ul, [data-dsb-md] ol {',
        '  margin: 4px 0 8px 0;',
        '  padding-left: 19px;',
        '}',
        '[data-dsb-md] ul { list-style-type: disc; }',
        '[data-dsb-md] ul ul { list-style-type: circle; margin: 3px 0 3px 0; }',
        '[data-dsb-md] ul ul ul { list-style-type: square; }',
        '[data-dsb-md] ol { list-style-type: decimal; }',
        '[data-dsb-md] li {',
        '  margin: 3.5px 0;',
        '  line-height: 1.58;',
        '}',
        '[data-dsb-md] li::marker {',
        '  color: color-mix(in srgb, currentColor 48%, transparent);',
        '}',
        '[data-dsb-md] [data-dsb-md-task-list] {',
        '  list-style-type: none;',
        '  padding-left: 4px;',
        '}',
        '[data-dsb-md] [data-dsb-md-check] {',
        '  display: inline-flex;',
        '  align-items: center;',
        '  justify-content: center;',
        '  width: 13px;',
        '  height: 13px;',
        '  margin-right: 6px;',
        '  border-radius: 3.5px;',
        '  border: 1px solid color-mix(in srgb, currentColor 38%, transparent);',
        '  font-size: 9.5px;',
        '  line-height: 1;',
        '  vertical-align: -1px;',
        '}',
        '[data-dsb-md] [data-dsb-md-check="1"] {',
        '  background: var(--dsb-md-accent, #60a5fa);',
        '  border-color: var(--dsb-md-accent, #60a5fa);',
        '  color: #0b0d12;',
        '  font-weight: 700;',
        '}',
        '[data-dsb-md] hr {',
        '  border: none;',
        '  height: 1px;',
        '  margin: 11px 0;',
        '  background: linear-gradient(90deg, color-mix(in srgb, currentColor 18%, transparent), color-mix(in srgb, currentColor 5%, transparent));',
        '}',
        '[data-dsb-md] blockquote {',
        '  margin: 7px 0 9px 0;',
        '  padding: 5px 11px;',
        '  border-left: 3px solid var(--dsb-md-accent, #60a5fa);',
        '  background: color-mix(in srgb, currentColor 6%, transparent);',
        '  border-radius: 0 8px 8px 0;',
        '  opacity: 0.94;',
        '}',
        '[data-dsb-md] blockquote p { margin: 2px 0; }',
        '[data-dsb-md] [data-dsb-md-alert-label] {',
        '  display: inline-block;',
        '  font-size: 11px;',
        '  font-weight: 650;',
        '  letter-spacing: 0.03em;',
        '  color: var(--dsb-md-accent, #60a5fa);',
        '  margin-bottom: 3px;',
        '}',
        '[data-dsb-md] code:not([data-dsb-md-codeblock]) {',
        '  font-family: ui-monospace, "Cascadia Code", "JetBrains Mono", Consolas, monospace;',
        '  font-size: 11.8px;',
        '  font-weight: 500;',
        '  padding: 1px 5.5px;',
        '  margin: 0 1px;',
        '  border-radius: 5px;',
        '  background: color-mix(in srgb, currentColor 11%, transparent);',
        '  border: 1px solid color-mix(in srgb, currentColor 13%, transparent);',
        '  word-break: break-word;',
        '}',
        '[data-dsb-md] strong code:not([data-dsb-md-codeblock]) {',
        '  font-weight: 650;',
        '}',
        '[data-dsb-md] [data-dsb-md-pre-wrap] {',
        '  margin: 8px 0 10px 0;',
        '  border-radius: 10px;',
        '  border: 1px solid color-mix(in srgb, currentColor 14%, transparent);',
        '  background: rgba(0, 0, 0, 0.24);',
        '  overflow: hidden;',
        '}',
        '[data-dsb-md] [data-dsb-md-pre-head] {',
        '  display: flex;',
        '  align-items: center;',
        '  justify-content: space-between;',
        '  padding: 4px 10px;',
        '  font-size: 11px;',
        '  border-bottom: 1px solid color-mix(in srgb, currentColor 10%, transparent);',
        '  background: color-mix(in srgb, currentColor 4%, transparent);',
        '  opacity: 0.82;',
        '}',
        '[data-dsb-md] [data-dsb-md-copy] {',
        '  border: none;',
        '  background: transparent;',
        '  color: inherit;',
        '  font-family: inherit;',
        '  font-size: 11px;',
        '  font-weight: 500;',
        '  padding: 1.5px 7px;',
        '  border-radius: 5px;',
        '  cursor: pointer;',
        '  transition: background 120ms ease;',
        '}',
        '[data-dsb-md] [data-dsb-md-copy]:hover {',
        '  background: color-mix(in srgb, currentColor 14%, transparent);',
        '}',
        '[data-dsb-md] pre {',
        '  margin: 0;',
        '  padding: 9px 11px;',
        '  overflow-x: auto;',
        '  font-family: ui-monospace, "Cascadia Code", "JetBrains Mono", Consolas, monospace;',
        '  font-size: 12px;',
        '  line-height: 1.52;',
        '  tab-size: 2;',
        '}',
        '[data-dsb-md] pre::-webkit-scrollbar { height: 5px; }',
        '[data-dsb-md] pre::-webkit-scrollbar-thumb {',
        '  background: rgba(148, 163, 184, 0.3);',
        '  border-radius: 999px;',
        '}',
        '[data-dsb-md] code[data-dsb-md-codeblock] {',
        '  font-family: inherit;',
        '  font-size: inherit;',
        '  font-weight: 400;',
        '  background: transparent;',
        '  padding: 0;',
        '  border: none;',
        '  white-space: pre;',
        '  word-break: normal;',
        '}',
        '[data-dsb-md] [data-dsb-md-table-wrap] {',
        '  margin: 8px 0 10px 0;',
        '  overflow-x: auto;',
        '  border-radius: 8px;',
        '  border: 1px solid color-mix(in srgb, currentColor 16%, transparent);',
        '}',
        '[data-dsb-md] table {',
        '  width: 100%;',
        '  border-collapse: collapse;',
        '  font-size: 12px;',
        '  line-height: 1.48;',
        '}',
        '[data-dsb-md] th, [data-dsb-md] td {',
        '  padding: 5.5px 9px;',
        '  border-bottom: 1px solid color-mix(in srgb, currentColor 11%, transparent);',
        '  border-right: 1px solid color-mix(in srgb, currentColor 8%, transparent);',
        '  text-align: left;',
        '  vertical-align: top;',
        '}',
        '[data-dsb-md] th:last-child, [data-dsb-md] td:last-child { border-right: none; }',
        '[data-dsb-md] tr:last-child td { border-bottom: none; }',
        '[data-dsb-md] th {',
        '  font-weight: 650;',
        '  white-space: nowrap;',
        '  background: color-mix(in srgb, currentColor 7%, transparent);',
        '}',
        '[data-dsb-md] a {',
        '  color: var(--dsb-md-accent, #60a5fa);',
        '  text-decoration: none;',
        '  border-bottom: 1px solid color-mix(in srgb, var(--dsb-md-accent, #60a5fa) 42%, transparent);',
        '  cursor: pointer;',
        '  transition: opacity 120ms ease, border-color 120ms ease;',
        '}',
        '[data-dsb-md] a:hover {',
        '  opacity: 0.92;',
        '  border-bottom-color: var(--dsb-md-accent, #60a5fa);',
        '}',
      ].join('\n');
      document.head.appendChild(styleEl);
    }

    function stripMarkdownForLip(raw) {
      if (!raw) return '';
      let s = String(raw).replace(/\r\n?/g, '\n');
      // Strip fenced code block markers while keeping short inline content readable
      s = s.replace(/```[^\n]*\n?([\s\S]*?)```/g, (_, inner) => {
        const clean = String(inner || '').replace(/\s+/g, ' ').trim();
        return clean.length > 64 ? ' ' + clean.slice(0, 64) + '… ' : ' ' + clean + ' ';
      });
      s = s.replace(/```[^\n]*/g, ' ');
      // Replace markdown images and links with their visible labels
      s = s.replace(/!\[([^\]]*)\]\((?:[^()\s]+|\([^)]*\))+\)/g, '$1');
      s = s.replace(/\[([^\]]+)\]\((?:[^()\s]+|\([^)]*\))+\)/g, '$1');
      // Strip ATX headings, horizontal rules, table separator lines, blockquote markers, alerts, list markers
      s = s.replace(/^\s*([-*_])(?:\s*\1){2,}\s*$/gm, ' ');
      s = s.replace(/^\s*\|?\s*:?-{2,}:?\s*(?:\|\s*:?-{2,}:?\s*)+\|?\s*$/gm, ' ');
      s = s.replace(/^\s{0,3}#{1,6}\s+/gm, '');
      s = s.replace(/^\s*>\s?(?:\[!(?:NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*)?/gim, '');
      s = s.replace(/^\s*(?:[-*+]|\d+\.)\s+(?:\[[ xX]\]\s+)?/gm, '· ');
      s = s.replace(/\|/g, ' ');
      // Strip inline formatting markers
      s = s.replace(/`{1,2}([^`]+)`{1,2}/g, '$1');
      s = s.replace(/(\*\*|__|~~)([\s\S]*?)\1/g, '$2');
      s = s.replace(/(^|[\s(（【\[])\*([^*\n]+)\*(?=$|[\s.,;:!?)\]）】。，；：])/g, '$1$2');
      return s.replace(/\s+/g, ' ').trim();
    }

    function openMarkdownLink(href, e) {
      if (e) {
        e.preventDefault();
        e.stopPropagation();
      }
      const target = String(href || '').trim();
      if (!target) return;
      if (/^file:\/\//i.test(target)) {
        let localPath = target.replace(/^file:\/\//i, '').replace(/#.*$/, '');
        if (/^\/[a-zA-Z]:/.test(localPath)) localPath = localPath.slice(1);
        try {
          localPath = decodeURIComponent(localPath);
        } catch {}
        if (/^[a-zA-Z]:\//.test(localPath)) {
          localPath = localPath.replace(/\//g, '\\');
        }
        tauriInvoke('quick_search_open_path', { path: localPath }).catch(() => {});
        return;
      }
      if (/^https?:\/\//i.test(target) || /^mailto:/i.test(target)) {
        try {
          window.open(target, '_blank', 'noopener,noreferrer');
        } catch {}
      }
    }

    function MdCodeBlock({ lang, code }) {
      const [copied, setCopied] = useState(false);
      const handleCopy = useCallback(
        (e) => {
          e.stopPropagation();
          if (typeof navigator !== 'undefined' && navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard
              .writeText(code)
              .then(() => {
                setCopied(true);
                setTimeout(() => setCopied(false), 1400);
              })
              .catch(() => {});
          }
        },
        [code],
      );
      return h(
        'div',
        { 'data-dsb-md-pre-wrap': '' },
        h(
          'div',
          { 'data-dsb-md-pre-head': '' },
          h(
            'span',
            {
              style: {
                fontFamily: 'ui-monospace, "Cascadia Code", Consolas, monospace',
                textTransform: 'lowercase',
              },
            },
            lang || 'code',
          ),
          h(
            'button',
            {
              type: 'button',
              'data-dsb-md-copy': '',
              onClick: handleCopy,
            },
            copied ? '已复制 ✓' : '复制',
          ),
        ),
        h('pre', null, h('code', { 'data-dsb-md-codeblock': '' }, code)),
      );
    }

    function findClosingMarkerSkippingCode(text, startIdx, marker) {
      const mLen = marker.length;
      let j = startIdx;
      while (j <= text.length - mLen) {
        if (text[j] === '\\') {
          j += 2;
          continue;
        }
        if (text[j] === '`') {
          let ticks = 1;
          while (text[j + ticks] === '`') ticks += 1;
          const endTick = text.indexOf('`'.repeat(ticks), j + ticks);
          if (endTick !== -1) {
            j = endTick + ticks;
            continue;
          }
        }
        if (text.slice(j, j + mLen) === marker) {
          if (mLen === 1 && text[j + 1] === marker) {
            j += 2;
            continue;
          }
          if (j > startIdx && !/\s/.test(text[j - 1])) {
            return j;
          }
        }
        j += 1;
      }
      return -1;
    }

    function renderMarkdownInline(text, keyPrefix, depth) {
      const d = depth || 0;
      const src = String(text || '');
      if (!src) return null;
      const out = [];
      let buf = '';
      let tok = 0;
      let i = 0;
      const len = src.length;
      const flush = () => {
        if (buf) {
          out.push(buf);
          buf = '';
        }
      };

      while (i < len) {
        const ch = src[i];

        // Escaped punctuation
        if (ch === '\\' && i + 1 < len && /[\\`*_[\]()~#>-]/.test(src[i + 1])) {
          buf += src[i + 1];
          i += 2;
          continue;
        }

        // Line break inside paragraph
        if (ch === '\n') {
          flush();
          out.push(h('br', { key: keyPrefix + '-br-' + tok++ }));
          i += 1;
          continue;
        }

        // Inline code: `code` or ``code``
        if (ch === '`') {
          let ticks = 1;
          while (src[i + ticks] === '`' && ticks < 3) ticks += 1;
          const delim = '`'.repeat(ticks);
          const closeIdx = src.indexOf(delim, i + ticks);
          if (closeIdx !== -1 && closeIdx > i + ticks) {
            let inner = src.slice(i + ticks, closeIdx);
            if (ticks > 1 && inner.startsWith(' ') && inner.endsWith(' ') && inner.trim()) {
              inner = inner.slice(1, -1);
            }
            flush();
            out.push(h('code', { key: keyPrefix + '-c-' + tok++ }, inner));
            i = closeIdx + ticks;
            continue;
          }
        }

        // Markdown link: [label](url)
        if (ch === '[') {
          let j = i + 1;
          let bracketDepth = 1;
          while (j < len && bracketDepth > 0) {
            if (src[j] === '\\') {
              j += 2;
              continue;
            }
            if (src[j] === '`') {
              const ct = src.indexOf('`', j + 1);
              if (ct !== -1) {
                j = ct + 1;
                continue;
              }
            }
            if (src[j] === '[') bracketDepth += 1;
            else if (src[j] === ']') bracketDepth -= 1;
            j += 1;
          }
          if (bracketDepth === 0 && src[j] === '(') {
            let k = j + 1;
            let parenDepth = 1;
            while (k < len && parenDepth > 0) {
              if (src[k] === '\\') {
                k += 2;
                continue;
              }
              if (src[k] === '(') parenDepth += 1;
              else if (src[k] === ')') parenDepth -= 1;
              else if (/\s/.test(src[k])) break;
              k += 1;
            }
            if (parenDepth === 0) {
              const label = src.slice(i + 1, j - 1);
              const href = src.slice(j + 1, k - 1).trim();
              if (/^(https?:\/\/|file:\/\/|mailto:)/i.test(href)) {
                flush();
                const linkKey = keyPrefix + '-a-' + tok++;
                out.push(
                  h(
                    'a',
                    {
                      key: linkKey,
                      href,
                      title: href,
                      target: '_blank',
                      rel: 'noopener noreferrer',
                      onClick: (e) => openMarkdownLink(href, e),
                    },
                    d < 3 ? renderMarkdownInline(label, linkKey, d + 1) : label,
                  ),
                );
                i = k;
                continue;
              }
            }
          }
        }

        // Bold: **text** or __text__
        if (d < 3 && ((ch === '*' && src[i + 1] === '*') || (ch === '_' && src[i + 1] === '_'))) {
          const marker = ch + ch;
          const prevWord = i > 0 && /[A-Za-z0-9]/.test(src[i - 1]);
          if (!(ch === '_' && prevWord) && i + 2 < len && !/\s/.test(src[i + 2])) {
            const closeIdx = findClosingMarkerSkippingCode(src, i + 2, marker);
            if (closeIdx !== -1) {
              const inner = src.slice(i + 2, closeIdx);
              flush();
              const bKey = keyPrefix + '-b-' + tok++;
              out.push(h('strong', { key: bKey }, renderMarkdownInline(inner, bKey, d + 1)));
              i = closeIdx + 2;
              continue;
            }
          }
        }

        // Strikethrough: ~~text~~
        if (d < 3 && ch === '~' && src[i + 1] === '~' && i + 2 < len && !/\s/.test(src[i + 2])) {
          const closeIdx = findClosingMarkerSkippingCode(src, i + 2, '~~');
          if (closeIdx !== -1) {
            const inner = src.slice(i + 2, closeIdx);
            flush();
            const sKey = keyPrefix + '-s-' + tok++;
            out.push(h('del', { key: sKey }, renderMarkdownInline(inner, sKey, d + 1)));
            i = closeIdx + 2;
            continue;
          }
        }

        // Italic: *text* (single asterisk, not **)
        if (d < 3 && ch === '*' && src[i + 1] !== '*' && i + 1 < len && !/\s/.test(src[i + 1])) {
          const closeIdx = findClosingMarkerSkippingCode(src, i + 1, '*');
          if (closeIdx !== -1 && src.slice(i + 1, closeIdx).indexOf('\n') === -1) {
            const inner = src.slice(i + 1, closeIdx);
            flush();
            const eKey = keyPrefix + '-em-' + tok++;
            out.push(h('em', { key: eKey }, renderMarkdownInline(inner, eKey, d + 1)));
            i = closeIdx + 1;
            continue;
          }
        }

        // Bare URL: https://... or http://...
        if (
          (ch === 'h' || ch === 'H') &&
          (src.slice(i, i + 8).toLowerCase() === 'https://' ||
            src.slice(i, i + 7).toLowerCase() === 'http://') &&
          (i === 0 || !/[A-Za-z0-9=_"']/.test(src[i - 1]))
        ) {
          let end = i;
          while (end < len && !/[\s<>"'`\u3000-\u303f\uff00-\uffef]/.test(src[end])) {
            end += 1;
          }
          let url = src.slice(i, end);
          while (url.length > 8 && /[.,;:!?\]}>]$/.test(url)) {
            url = url.slice(0, -1);
          }
          while (url.endsWith(')') && (url.match(/\(/g) || []).length < (url.match(/\)/g) || []).length) {
            url = url.slice(0, -1);
          }
          if (url.length > 8) {
            flush();
            out.push(
              h(
                'a',
                {
                  key: keyPrefix + '-u-' + tok++,
                  href: url,
                  title: url,
                  target: '_blank',
                  rel: 'noopener noreferrer',
                  onClick: (e) => openMarkdownLink(url, e),
                },
                url,
              ),
            );
            i += url.length;
            continue;
          }
        }

        buf += ch;
        i += 1;
      }
      flush();
      return out;
    }

    function parseMdTableRow(line) {
      let s = String(line || '').trim();
      if (s.startsWith('|')) s = s.slice(1);
      if (s.endsWith('|')) s = s.slice(0, -1);
      const cells = [];
      let cur = '';
      let inCode = false;
      for (let i = 0; i < s.length; i += 1) {
        if (s[i] === '\\' && s[i + 1] === '|') {
          cur += '|';
          i += 1;
          continue;
        }
        if (s[i] === '`') inCode = !inCode;
        if (s[i] === '|' && !inCode) {
          cells.push(cur.trim());
          cur = '';
          continue;
        }
        cur += s[i];
      }
      cells.push(cur.trim());
      return cells;
    }

    function isMdTableSep(line) {
      return /^\s*\|?\s*:?-{2,}:?\s*(?:\|\s*:?-{2,}:?\s*)+\|?\s*$/.test(String(line || ''));
    }

    function isMdBlockBoundary(line, nextLine) {
      if (!line || !line.trim()) return true;
      if (/^\s*(`{3,}|~{3,})/.test(line)) return true;
      if (/^\s{0,3}#{1,4}\s+/.test(line)) return true;
      if (/^\s*([-*_])(?:\s*\1){2,}\s*$/.test(line)) return true;
      if (/^\s*>\s?/.test(line)) return true;
      if (/^\s*(?:[-*+]|\d+\.)\s+/.test(line)) return true;
      if (line.indexOf('|') !== -1 && nextLine && isMdTableSep(nextLine)) return true;
      return false;
    }

    function renderMdListNodes(items, baseKey) {
      if (!items || items.length === 0) return null;
      const nodes = [];
      let idx = 0;
      let groupIdx = 0;
      while (idx < items.length) {
        const listType = items[idx].type;
        const baseIndent = items[idx].indent;
        const group = [];
        let hasTasks = false;
        while (idx < items.length && items[idx].indent === baseIndent && items[idx].type === listType) {
          const item = items[idx];
          idx += 1;
          const nested = [];
          while (idx < items.length && items[idx].indent > baseIndent) {
            nested.push(items[idx]);
            idx += 1;
          }
          if (item.checked !== null) hasTasks = true;
          group.push({ item, nested });
        }
        const gKey = baseKey + '-l' + groupIdx++;
        nodes.push(
          h(
            listType,
            {
              key: gKey,
              'data-dsb-md-task-list': hasTasks ? '1' : undefined,
            },
            group.map(({ item, nested }, liIdx) => {
              const liKey = gKey + '-i' + liIdx;
              return h(
                'li',
                { key: liKey },
                item.checked !== null
                  ? h(
                      'span',
                      {
                        'data-dsb-md-check': item.checked ? '1' : '0',
                        'aria-hidden': 'true',
                      },
                      item.checked ? '✓' : '',
                    )
                  : null,
                renderMarkdownInline(item.text, liKey, 0),
                nested.length > 0 ? renderMdListNodes(nested, liKey) : null,
              );
            }),
          ),
        );
      }
      return nodes;
    }

    function renderMarkdownBlocks(rawText) {
      const normalized = String(rawText || '').replace(/\r\n?/g, '\n');
      if (!normalized.trim()) return null;
      const lines = normalized.split('\n');
      const blocks = [];
      let i = 0;
      let bIdx = 0;

      while (i < lines.length) {
        const line = lines[i];
        if (!line.trim()) {
          i += 1;
          continue;
        }

        // 1. Fenced code block (``` or ~~~)
        const fenceMatch = line.match(/^\s*(`{3,}|~{3,})(.*)$/);
        if (fenceMatch) {
          const fenceChar = fenceMatch[1][0];
          const fenceLen = fenceMatch[1].length;
          const lang = String(fenceMatch[2] || '').trim().split(/\s+/)[0] || '';
          i += 1;
          const codeLines = [];
          const closeRe = new RegExp('^\\s*' + fenceChar + '{' + fenceLen + ',}\\s*$');
          while (i < lines.length && !closeRe.test(lines[i])) {
            codeLines.push(lines[i]);
            i += 1;
          }
          if (i < lines.length) i += 1;
          blocks.push(
            h(MdCodeBlock, {
              key: 'md-code-' + bIdx++,
              lang,
              code: codeLines.join('\n'),
            }),
          );
          continue;
        }

        // 2. ATX Heading (# to ####)
        const headMatch = line.match(/^\s{0,3}(#{1,4})\s+(.+?)\s*#*\s*$/);
        if (headMatch) {
          const level = headMatch[1].length;
          const hKey = 'md-h-' + bIdx++;
          blocks.push(h('h' + level, { key: hKey }, renderMarkdownInline(headMatch[2], hKey, 0)));
          i += 1;
          continue;
        }

        // 3. Horizontal rule (---, ***, ___)
        if (/^\s*([-*_])(?:\s*\1){2,}\s*$/.test(line)) {
          blocks.push(h('hr', { key: 'md-hr-' + bIdx++ }));
          i += 1;
          continue;
        }

        // 4. Blockquote (> ... and GitHub [!NOTE]/[!TIP]/[!IMPORTANT]/[!WARNING]/[!CAUTION] callouts)
        if (/^\s*>\s?/.test(line)) {
          const qLines = [];
          while (i < lines.length && /^\s*>\s?/.test(lines[i])) {
            qLines.push(lines[i].replace(/^\s*>\s?/, ''));
            i += 1;
          }
          const qKey = 'md-bq-' + bIdx++;
          let alertLabel = null;
          if (qLines.length > 0) {
            const alertMatch = qLines[0].match(/^\s*\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*(.*)$/i);
            if (alertMatch) {
              alertLabel = alertMatch[1].toUpperCase();
              if (alertMatch[2]) qLines[0] = alertMatch[2];
              else qLines.shift();
            }
          }
          blocks.push(
            h(
              'blockquote',
              { key: qKey },
              alertLabel ? h('div', { 'data-dsb-md-alert-label': '' }, alertLabel) : null,
              renderMarkdownBlocks(qLines.join('\n')),
            ),
          );
          continue;
        }

        // 5. GFM Table
        if (line.indexOf('|') !== -1 && i + 1 < lines.length && isMdTableSep(lines[i + 1])) {
          const headers = parseMdTableRow(line);
          const aligns = parseMdTableRow(lines[i + 1]).map((cell) => {
            if (cell.startsWith(':') && cell.endsWith(':')) return 'center';
            if (cell.endsWith(':')) return 'right';
            return 'left';
          });
          i += 2;
          const rows = [];
          while (
            i < lines.length &&
            lines[i].trim() &&
            lines[i].indexOf('|') !== -1 &&
            !/^\s*(`{3,}|~{3,})/.test(lines[i])
          ) {
            rows.push(parseMdTableRow(lines[i]));
            i += 1;
          }
          const tKey = 'md-tbl-' + bIdx++;
          blocks.push(
            h(
              'div',
              { key: tKey, 'data-dsb-md-table-wrap': '' },
              h(
                'table',
                null,
                h(
                  'thead',
                  null,
                  h(
                    'tr',
                    null,
                    headers.map((cell, cIdx) =>
                      h(
                        'th',
                        {
                          key: tKey + '-th-' + cIdx,
                          style: aligns[cIdx] && aligns[cIdx] !== 'left' ? { textAlign: aligns[cIdx] } : undefined,
                        },
                        renderMarkdownInline(cell, tKey + '-th-' + cIdx, 0),
                      ),
                    ),
                  ),
                ),
                rows.length > 0
                  ? h(
                      'tbody',
                      null,
                      rows.map((row, rIdx) =>
                        h(
                          'tr',
                          { key: tKey + '-r-' + rIdx },
                          headers.map((_, cIdx) =>
                            h(
                              'td',
                              {
                                key: tKey + '-r-' + rIdx + '-c-' + cIdx,
                                style:
                                  aligns[cIdx] && aligns[cIdx] !== 'left'
                                    ? { textAlign: aligns[cIdx] }
                                    : undefined,
                              },
                              renderMarkdownInline(row[cIdx] || '', tKey + '-r-' + rIdx + '-c-' + cIdx, 0),
                            ),
                          ),
                        ),
                      ),
                    )
                  : null,
              ),
            ),
          );
          continue;
        }

        // 6. Unordered / Ordered / Task Lists (supports nesting)
        if (/^\s*(?:[-*+]|\d+\.)\s+/.test(line)) {
          const listItems = [];
          while (i < lines.length) {
            const cur = lines[i];
            const lm = cur.match(/^(\s*)(?:([-*+])|(\d+)\.)\s+(.*)$/);
            if (lm) {
              const rawIndent = lm[1].replace(/\t/g, '  ').length;
              const indent = Math.floor(rawIndent / 2);
              const type = lm[2] ? 'ul' : 'ol';
              let content = lm[4];
              let checked = null;
              const tm = content.match(/^\[([ xX])\]\s+(.*)$/);
              if (tm) {
                checked = tm[1].toLowerCase() === 'x';
                content = tm[2];
              }
              listItems.push({ indent, type, text: content, checked });
              i += 1;
              continue;
            }
            // Indented continuation line inside previous list item
            if (
              listItems.length > 0 &&
              cur.trim() &&
              /^\s{2,}/.test(cur) &&
              !isMdBlockBoundary(cur, lines[i + 1])
            ) {
              listItems[listItems.length - 1].text += '\n' + cur.trim();
              i += 1;
              continue;
            }
            break;
          }
          const lKey = 'md-list-' + bIdx++;
          blocks.push(h(React.Fragment, { key: lKey }, renderMdListNodes(listItems, lKey)));
          continue;
        }

        // 7. Paragraph
        const pLines = [line];
        i += 1;
        while (i < lines.length && !isMdBlockBoundary(lines[i], lines[i + 1])) {
          pLines.push(lines[i]);
          i += 1;
        }
        const pKey = 'md-p-' + bIdx++;
        blocks.push(h('p', { key: pKey }, renderMarkdownInline(pLines.join('\n'), pKey, 0)));
      }

      return blocks;
    }

    const MarkdownMessage = React.memo(function MarkdownMessage({ text, accent, streamingNode }) {
      const rendered = useMemo(() => renderMarkdownBlocks(text), [text]);
      return h(
        'div',
        {
          'data-dsb-md': '1',
          style: accent ? { '--dsb-md-accent': accent } : undefined,
        },
        rendered,
        streamingNode ? h('div', { style: { marginTop: '4px' } }, streamingNode) : null,
      );
    });

    function isExcludedBlockKind(rawKind) {
      const k = String(rawKind || '').toLowerCase();
      if (!k) return false;
      return (
        k.includes('think') ||
        k.includes('reason') ||
        k.includes('thought') ||
        k.includes('tool') ||
        k.includes('function') ||
        k.includes('status') ||
        k.includes('meta')
      );
    }

    function stripInlineThinkingTags(text) {
      if (!text) return '';
      return String(text)
        .replace(/<think>[\s\S]*?<\/think>/gi, '')
        .replace(/<thinking>[\s\S]*?<\/thinking>/gi, '')
        .trim();
    }

    function extractTextFromParts(parts) {
      if (!parts) return '';
      if (typeof parts === 'string') return stripInlineThinkingTags(parts);
      if (!Array.isArray(parts)) return '';
      const buf = [];
      for (const p of parts) {
        if (!p) continue;
        if (typeof p === 'string') {
          buf.push(p);
          continue;
        }
        if (isExcludedBlockKind(p.type || p.kind)) continue;
        if (typeof p.text === 'string' && (p.type === 'text' || p.kind === 'text' || (!p.type && !p.kind))) {
          buf.push(p.text);
        } else if (typeof p.content === 'string' && (!p.type || p.type === 'text') && (!p.kind || p.kind === 'text')) {
          buf.push(p.content);
        }
      }
      return stripInlineThinkingTags(buf.join('\n'));
    }

    function parseAssistantBlocksList(blocks) {
      const texts = [];
      if (Array.isArray(blocks)) {
        for (const b of blocks) {
          if (!b) continue;
          if (isExcludedBlockKind(b.kind || b.type)) continue;
          if ((b.kind === 'text' || b.type === 'text') && typeof b.text === 'string') {
            const cleaned = stripInlineThinkingTags(b.text);
            if (cleaned) texts.push(cleaned);
          }
        }
      }
      return { text: texts.join('\n\n').trim() };
    }

    function extractMessagesFromDshProps(props) {
      const result = [];
      if (!props || typeof props !== 'object') return result;

      let chatSnap = null;
      if (typeof props.useConversation === 'function') {
        try {
          chatSnap = props.useConversation((c) => {
            if (!c) return null;
            if (c.views && typeof c.views.get === 'function') return c.views.get('chat');
            return c.chat || null;
          });
        } catch {}
      }
      const session = props.session || null;
      if (!chatSnap && session && session.chat) {
        chatSnap = session.chat;
      }

      if (chatSnap && chatSnap.legacy && Array.isArray(chatSnap.legacy.nodes) && chatSnap.legacy.nodes.length > 0) {
        for (let i = 0; i < chatSnap.legacy.nodes.length; i += 1) {
          const nd = chatSnap.legacy.nodes[i];
          if (!nd) continue;
          if (nd.kind === 'user' || nd.kind === 'steering') {
            const txt = extractTextFromParts(nd.content);
            if (txt) {
              result.push({
                id: String(nd.messageId || 'u-' + i),
                role: 'user',
                text: txt,
                streaming: false,
              });
            }
          } else if (nd.kind === 'assistant') {
            const parsed = parseAssistantBlocksList(nd.blocks);
            if (parsed.text) {
              result.push({
                id: String(nd.messageId || 'a-' + i),
                role: 'assistant',
                text: parsed.text,
                streaming: false,
              });
            }
          }
        }
        if (chatSnap.legacy.partial && Array.isArray(chatSnap.legacy.partial.blocks)) {
          const pp = parseAssistantBlocksList(chatSnap.legacy.partial.blocks);
          if (pp.text) {
            result.push({
              id: 'streaming-partial',
              role: 'assistant',
              text: pp.text,
              streaming: true,
            });
          }
        }
        if (result.length > 0) return result.slice(-40);
      }

      if (
        chatSnap &&
        Array.isArray(chatSnap.order) &&
        chatSnap.nodes &&
        typeof chatSnap.nodes.get === 'function'
      ) {
        for (const key of chatSnap.order) {
          const node = chatSnap.nodes.get(key);
          const d = node && node.data ? node.data : node;
          if (!d) continue;
          const kind = node.kind || d.kind;
          if (isExcludedBlockKind(kind)) continue;
          if (kind === 'user' || kind === 'steering') {
            const txt = extractTextFromParts(d.content ?? d.parts ?? d.text);
            if (txt) {
              result.push({
                id: String(key),
                role: 'user',
                text: txt,
                streaming: false,
              });
            }
          } else if (kind === 'assistant-step' || kind === 'assistant' || Array.isArray(d.blocks)) {
            const parsed = parseAssistantBlocksList(d.blocks);
            if (parsed.text) {
              result.push({
                id: String(key),
                role: 'assistant',
                text: parsed.text,
                streaming: d.status === 'running',
              });
            }
          }
        }
        if (result.length > 0) return result.slice(-40);
      }

      const list =
        session && Array.isArray(session.messages)
          ? session.messages
          : session && Array.isArray(session.items)
            ? session.items
            : session && Array.isArray(session.turns)
              ? session.turns
              : [];
      for (let i = 0; i < list.length; i += 1) {
        const m = list[i];
        if (!m || typeof m !== 'object') continue;
        const rawRole = String(m.role || m.sender || m.author || '').toLowerCase();
        if (rawRole.includes('tool') || rawRole.includes('system') || rawRole.includes('function')) continue;
        const role = rawRole.includes('user') || rawRole.includes('human') ? 'user' : 'assistant';
        const txt = extractTextFromParts(m.content ?? m.parts ?? m.blocks ?? m.text ?? m.message);
        if (txt) {
          result.push({
            id: String(m.id || 'msg-' + i),
            role,
            text: txt,
            streaming: Boolean(m.streaming),
          });
        }
      }
      return result.slice(-40);
    }

    function postTwinSync(payload) {
      fetch('/anything-to-prompt/sync', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'X-DSB-Client': '1',
        },
        body: JSON.stringify(Object.assign({ clientId: SELF_CLIENT_ID, dshOrigin: window.location.origin }, payload)),
      }).catch(() => {});
    }

    function renderFolderIcon(size = 14) {
      if (DSHPrimitives && DSHPrimitives.IconFolderCloseRegular) {
        return h(DSHPrimitives.IconFolderCloseRegular, { size, style: { flexShrink: 0, opacity: 0.85 } });
      }
      return h(
        'svg',
        {
          xmlns: 'http://www.w3.org/2000/svg',
          viewBox: '0 0 16 16',
          width: size,
          height: size,
          fill: 'none',
          strokeWidth: 1,
          style: { flexShrink: 0, opacity: 0.85 },
        },
        h('path', {
          d: 'M1.50439 3.11059C1.50439 2.55831 1.95211 2.1106 2.50439 2.1106H5.43389C5.67773 2.1106 5.91318 2.19969 6.09593 2.36113L7.71649 3.79265C7.89924 3.95409 8.1347 4.04319 8.3785 4.04319H13.4958C14.0481 4.04319 14.4958 4.4909 14.4958 5.04319V12.8894C14.4958 13.4417 14.0481 13.8894 13.4958 13.8894H2.50439C1.95211 13.8894 1.50439 13.4417 1.50439 12.8894V4.04319V3.11059Z',
          stroke: 'currentColor',
        }),
        h('path', {
          d: 'M3.63501 7.66614H12.3647',
          stroke: 'currentColor',
        }),
      );
    }

    function formatRelativeTime(ts) {
      if (!ts) return '';
      const ms = typeof ts === 'number' ? ts : Date.parse(String(ts));
      if (!Number.isFinite(ms)) return '';
      const diffSec = Math.max(0, Math.floor((Date.now() - ms) / 1000));
      if (diffSec < 60) return '刚刚';
      const diffMin = Math.floor(diffSec / 60);
      if (diffMin < 60) return diffMin + '分钟前';
      const diffHr = Math.floor(diffMin / 60);
      if (diffHr < 24) return diffHr + '小时前';
      const diffDay = Math.floor(diffHr / 24);
      if (diffDay < 7) return diffDay + '天前';
      if (diffDay < 30) return Math.floor(diffDay / 7) + '周前';
      return Math.floor(diffDay / 30) + '个月前';
    }

    function workspaceBasename(p) {
      if (!p || typeof p !== 'string') return '未命名工作区';
      const clean = p.replace(/[\\/]+$/, '');
      const idx = Math.max(clean.lastIndexOf('/'), clean.lastIndexOf('\\'));
      return idx >= 0 ? clean.slice(idx + 1) || clean : clean;
    }

    function mainSessionIdOfList(listSnap) {
      if (!listSnap || !listSnap.byId) return '';
      const values = Object.values(listSnap.byId);
      for (const row of values) {
        if (row && row.retainedBy && (row.retainedBy.mainView || 0) > 0) {
          return String(row.id || '');
        }
      }
      return '';
    }

    function sessionDisplayTitle(s) {
      if (!s) return '新对话';
      if (s.blank) return '新对话';
      const t = typeof s.title === 'string' ? s.title.trim() : '';
      return t || '新对话';
    }

    function makeCardDockComponent(services) {
      const { sessions, workspaces, uiWorkspace } = services;

      function switchToSession(targetSessionId, targetPath) {
        if (targetSessionId) {
          writeStoredString(STORAGE_KEY_LAST_SESSION, targetSessionId);
          if (uiWorkspace && typeof uiWorkspace.openSession === 'function') {
            try {
              uiWorkspace.openSession(targetSessionId);
              return;
            } catch {}
          }
        }
        if (targetPath && typeof targetPath === 'string' && targetPath.startsWith('/')) {
          const sep = targetPath.includes('?') ? '&' : '?';
          const nextUrl = targetPath + (IS_BAR ? sep + 'dsh_bar_clone=1' : '');
          if (window.location.pathname !== targetPath) {
            window.location.replace(nextUrl);
          }
        }
      }

      return function CardholderSlipDock(props) {
        const liveDraft =
          props && props.input && typeof props.input.draft === 'string' ? props.input.draft : '';
        const inputActions = props && props.inputActions ? props.inputActions : null;
        const sessionId = props && props.sessionId ? String(props.sessionId) : '';

        const [barMode, setBarModeState] = useState(() => dsbRuntime.barMode);
        const [previewExpanded, setPreviewExpanded] = useState(() => dsbRuntime.previewExpanded);
        const [barWidth, setBarWidth] = useState(() => dsbRuntime.barWidth);
        const [hovered, setHovered] = useState(false);
        const [pinned, setPinned] = useState(() =>
          IS_BAR ? false : readStoredBool(STORAGE_KEY_PINNED, false),
        );
        const [historyOpen, setHistoryOpen] = useState(false);
        const [hoverPeek, setHoverPeek] = useState(false);
        const [composerPopover, setComposerPopover] = useState(null);
        const [composerSubView, setComposerSubView] = useState(null);
        const [mentionQuery, setMentionQuery] = useState('');
        const [mentionHits, setMentionHits] = useState([]);
        const mentionSeqRef = useRef(0);
        const sketchCanvasRef = useRef(null);
        const sketchDrawingRef = useRef(false);
        const sketchColorRef = useRef('#38bdf8');
        const [sketchColor, setSketchColor] = useState('#38bdf8');
        const hoverPeekTimerRef = useRef(null);
        const lastKnownAsstRef = useRef({ sessionKey: '', asstId: '' });
        const [expandedHeight, setExpandedHeight] = useState(() =>
          readStoredNumber(STORAGE_KEY_HEIGHT, DEFAULT_EXPANDED_PX, MIN_EXPANDED_PX, MAX_EXPANDED_PX),
        );
        const [agentName, setAgentName] = useState(() =>
          readStoredString(STORAGE_KEY_AGENT_NAME, '小张'),
        );
        const [editingName, setEditingName] = useState(false);
        const [nameDraft, setNameDraft] = useState(agentName);
        const [matchedCardWidth, setMatchedCardWidth] = useState(null);
        const [lipOverflow, setLipOverflow] = useState(false);
        const [, setHarnessTick] = useState(0);
        const [quickHits, setQuickHits] = useState([]);
        const [quickSelectedIdx, setQuickSelectedIdx] = useState(-1);
        const [quickStatus, setQuickStatus] = useState(null);
        const quickHitsRef = useRef([]);
        const quickSelectedRef = useRef(-1);
        const quickReqSeqRef = useRef(0);
        const quickDismissedQueryRef = useRef('');
        const quickQueryRef = useRef('');
        const quickTypedQueryRef = useRef('');
        const quickSyncedPreviewRef = useRef(null);

        useEffect(() => {
          const bump = () => setHarnessTick((n) => n + 1);
          dsbRuntime.listeners.add(bump);
          if (IS_BAR) {
            refreshHarnessAvailability();
            tauriInvoke('harness_get_active')
              .then((saved) => {
                if (typeof saved === 'string' && saved.trim()) {
                  setActiveHarness(saved, false);
                }
              })
              .catch(() => {});
          }
          return () => {
            dsbRuntime.listeners.delete(bump);
          };
        }, []);

        // Sync authentic placeholder text when switching between DSH / Antigravity / Codex
        useEffect(() => {
          if (!IS_BAR || typeof document === 'undefined') return undefined;
          const applyPlaceholder = () => {
            const card = document.querySelector('[data-composer-card]');
            if (!card) return;
            const meta = HARNESS_META[dsbRuntime.activeHarness] || HARNESS_META.dsh;
            const customPh = meta.placeholder || '';
            const ta = card.querySelector('textarea');
            if (ta) {
              if (!ta.dataset.dsbOrigPlaceholder) {
                ta.dataset.dsbOrigPlaceholder = ta.getAttribute('placeholder') || '';
              }
              ta.setAttribute('placeholder', customPh || ta.dataset.dsbOrigPlaceholder);
            }
            const phNodes = card.querySelectorAll('[data-placeholder]');
            for (const node of phNodes) {
              if (!(node instanceof HTMLElement)) continue;
              if (!node.dataset.dsbOrigPlaceholder) {
                node.dataset.dsbOrigPlaceholder = node.getAttribute('data-placeholder') || '';
              }
              node.setAttribute('data-placeholder', customPh || node.dataset.dsbOrigPlaceholder);
            }
          };
          applyPlaceholder();
          const t = setTimeout(applyPlaceholder, 260);
          return () => clearTimeout(t);
        }, [dsbRuntime.activeHarness, liveDraft]);

        const wrapRef = useRef(null);
        const scrollRef = useRef(null);
        const lipViewportRef = useRef(null);
        const msgNodeMapRef = useRef(new Map());
        const [activeAssistantId, setActiveAssistantId] = useState('');
        const collapseTimerRef = useRef(null);
        const hoverTimerRef = useRef(null);
        const submitIntentUntilRef = useRef(0);
        const sessionSubBaselineRef = useRef({ sessionId: '', userCount: 0 });
        const isResizingHeightRef = useRef(false);
        const isResizingWidthRef = useRef(false);
        const lastRemoteDraftRef = useRef(null);
        const layoutRafRef = useRef(0);
        const layoutAnimUntilRef = useRef(0);

        useEffect(() => {
          ensureCardDockStyles();
          if (IS_BAR && typeof document !== 'undefined') {
            document.documentElement.setAttribute('data-dsh-bar-clone', '1');
            if (!document.documentElement.getAttribute('data-dsb-mode')) {
              document.documentElement.setAttribute('data-dsb-mode', dsbRuntime.barMode);
            }
          }
          installGlobalBridge();
          if (IS_BAR) {
            sendBarReport({ kind: 'diag', detail: 'CardDock MOUNT sessionId=' + sessionId });
          }
          return () => {
            if (IS_BAR) {
              sendBarReport({ kind: 'diag', detail: 'CardDock UNMOUNT sessionId=' + sessionId });
            }
          };
        }, []);

        useEffect(() => {
          dsbRuntime.inputActions = inputActions;
          dsbRuntime.sessionId = sessionId;
          dsbRuntime.liveDraft = liveDraft;
          dsbRuntime.hovered = hovered || previewExpanded;
          dsbRuntime.pinned = pinned;
          dsbRuntime.expandedHeight = expandedHeight;
          dsbRuntime.barWidth = barWidth;
          if (IS_BAR && sessionId) {
            writeStoredString(STORAGE_KEY_LAST_SESSION, sessionId);
          }
          notifyBarRuntime();
        }, [inputActions, sessionId, liveDraft, hovered, previewExpanded, pinned, expandedHeight, barWidth]);

        // Keep Rust pick counter synced 1:1 with ALL capsules currently in the active composer
        // (both DSH native composer and external Harness composers — never overwrite extCapsules with 0!)
        useEffect(() => {
          if (!IS_BAR || typeof document === 'undefined') return undefined;
          let rafId = 0;
          const syncCapsuleCount = () => {
            if (rafId) cancelAnimationFrame(rafId);
            rafId = requestAnimationFrame(() => {
              rafId = 0;
              sendBarReport({
                kind: 'capsules',
                capsuleCount: getActiveComposerCapsuleCount(),
                afterInsert: false,
              });
            });
          };
          syncCapsuleCount();
          const comp = document.querySelector('[data-composer-card]');
          const mo = comp ? new MutationObserver(syncCapsuleCount) : null;
          if (mo && comp) {
            mo.observe(comp, { childList: true, subtree: true, characterData: true });
          }
          return () => {
            if (rafId) cancelAnimationFrame(rafId);
            if (mo) mo.disconnect();
          };
        }, [liveDraft, sessionId, dsbRuntime.activeHarness, (dsbRuntime.extCapsules || []).length]);

        useEffect(() => {
          const resetTransientUi = () => {
            setComposerPopover(null);
            setHoverPeek(false);
            setHovered(false);
            setHistoryOpen(false);
            setPreviewExpanded(false);
            setPinned((prev) => {
              if (prev) {
                try { localStorage.setItem(STORAGE_KEY_PINNED, '0'); } catch {}
              }
              return false;
            });
            if (typeof dsbRuntime.setPopupCloseListener === 'function') {
              dsbRuntime.setPopupCloseListener();
            }
          };
          dsbRuntime.onHiddenReset = resetTransientUi;
          dsbRuntime.setModeListener = (nextMode, wantExpanded = false) => {
            setBarModeState(IS_BAR ? 'summon' : nextMode);
            setPreviewExpanded(Boolean(wantExpanded));
            setComposerPopover(null);
            setHoverPeek(false);
            if (nextMode === 'lip' && !IS_BAR) {
              resetTransientUi();
            } else if (wantExpanded) {
              setHistoryOpen(true);
              dsbRuntime.unreadCount = 0;
              dsbRuntime.unreadPreview = '';
              dsbRuntime.pendingBubbleTarget = null;
            }
          };
          return () => {
            dsbRuntime.setModeListener = null;
            dsbRuntime.onHiddenReset = null;
          };
        }, []);

        const activeHarness = IS_BAR ? dsbRuntime.activeHarness : 'dsh';
        const dshEntries = extractMessagesFromDshProps(props);
        const entries =
          activeHarness === 'dsh'
            ? dshEntries
            : Array.isArray(dsbRuntime.externalMessages[activeHarness])
              ? dsbRuntime.externalMessages[activeHarness]
              : [];
        const effectiveSessionId =
          activeHarness === 'dsh'
            ? sessionId
            : (dsbRuntime.externalSelection[activeHarness] &&
                dsbRuntime.externalSelection[activeHarness].sessionId) ||
              activeHarness + ':new';
        const assistantEntriesDesc = useMemo(() => {
          const list = [];
          for (let i = entries.length - 1; i >= 0; i -= 1) {
            if (entries[i].role === 'assistant') list.push(entries[i]);
          }
          return list;
        }, [entries]);

        const latestAssistant = useMemo(() => {
          if (assistantEntriesDesc.length > 0) return assistantEntriesDesc[0];
          return entries.length > 0 ? entries[entries.length - 1] : null;
        }, [assistantEntriesDesc, entries]);

        useEffect(() => {
          if (assistantEntriesDesc.length > 0) {
            setActiveAssistantId(String(assistantEntriesDesc[0].id));
          } else {
            setActiveAssistantId('');
          }
        }, [sessionId, assistantEntriesDesc.length > 0 ? String(assistantEntriesDesc[0].id) : '']);

        const jumpToAssistantReply = useCallback((id) => {
          const key = String(id || '');
          setActiveAssistantId(key);
          const scroller = scrollRef.current;
          const node = msgNodeMapRef.current.get(key);
          if (scroller && node) {
            const targetTop = Math.max(0, node.offsetTop - 10);
            scroller.scrollTo({ top: targetTop, behavior: 'smooth' });
          } else if (node && typeof node.scrollIntoView === 'function') {
            node.scrollIntoView({ behavior: 'smooth', block: 'start' });
          }
        }, []);

        const handleHistoryScroll = useCallback(() => {
          const scroller = scrollRef.current;
          if (!scroller || assistantEntriesDesc.length === 0) return;
          // If scrolled near the bottom, highlight the newest (leftmost) dot
          if (scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 28) {
            setActiveAssistantId(String(assistantEntriesDesc[0].id));
            return;
          }
          const viewTop = scroller.scrollTop + 24;
          let chosen = String(assistantEntriesDesc[0].id);
          let bestDist = Infinity;
          for (let i = 0; i < assistantEntriesDesc.length; i += 1) {
            const id = String(assistantEntriesDesc[i].id);
            const node = msgNodeMapRef.current.get(id);
            if (!node) continue;
            const dist = Math.abs(node.offsetTop - viewTop);
            if (dist < bestDist) {
              bestDist = dist;
              chosen = id;
            }
          }
          setActiveAssistantId(chosen);
        }, [assistantEntriesDesc]);

        const latestSingleLine = useMemo(
          () => (latestAssistant ? stripMarkdownForLip(latestAssistant.text) : ''),
          [latestAssistant],
        );

        const userMessageCount = useMemo(() => {
          let count = 0;
          for (let i = 0; i < entries.length; i += 1) {
            if (entries[i].role === 'user') count += 1;
          }
          return count;
        }, [entries]);

        const isExpanded = IS_BAR
          ? !dsbRuntime.popupOpen && (pinned || historyOpen || previewExpanded)
          : !dsbRuntime.popupOpen && (pinned || hovered || previewExpanded);

        // Load real composer options (models, efforts, modes, Codex plugins, slash actions) per Harness
        useEffect(() => {
          if (!IS_BAR || activeHarness === 'dsh') return;
          if (!dsbRuntime.harnessOptions[activeHarness]) {
            refreshHarnessOptions(activeHarness);
          }
          setComposerPopover(null);
        }, [activeHarness]);

        // Close composer popover on outside click or Esc
        useEffect(() => {
          if (!composerPopover || typeof document === 'undefined') return undefined;
          const onDown = (e) => {
            const target = e.target;
            if (
              target instanceof Element &&
              (target.closest('[data-dsb-composer-popover="1"]') ||
                target.closest('[data-dsb-plus-btn="1"]') ||
                target.closest('[data-dsb-model-btn="1"]') ||
                target.closest('[data-dsb-mode-btn="1"]'))
            ) {
              return;
            }
            setComposerPopover(null);
          };
          const onKey = (e) => {
            if (e.key === 'Escape') {
              e.preventDefault();
              e.stopPropagation();
              setComposerPopover(null);
            }
          };
          document.addEventListener('mousedown', onDown, true);
          document.addEventListener('keydown', onKey, true);
          return () => {
            document.removeEventListener('mousedown', onDown, true);
            document.removeEventListener('keydown', onKey, true);
          };
        }, [composerPopover]);

        // Track explicit user submit intent inside [data-composer-card] (prevents async session hydration from misfiring 'submitted')
        // Also intercepts Enter / Send clicks when targeting Antigravity or Codex so messages never leak to DSH!
        useEffect(() => {
          if (!IS_BAR || typeof document === 'undefined') return undefined;
          const extractCurrentComposerText = () => {
            if (typeof dsbRuntime.liveDraft === 'string' && dsbRuntime.liveDraft.trim()) {
              return dsbRuntime.liveDraft;
            }
            const ed =
              document.querySelector('[data-composer-card] [data-lexical-editor="true"]') ||
              document.querySelector('[data-composer-card] [contenteditable="true"]') ||
              document.querySelector('[data-composer-card] textarea');
            return ed ? String(ed.value || ed.textContent || '') : '';
          };
          const dispatchExternalSubmit = (e) => {
            const text = extractCurrentComposerText().trim();
            if (!text) return false;
            e.preventDefault();
            e.stopPropagation();
            if (typeof e.stopImmediatePropagation === 'function') {
              e.stopImmediatePropagation();
            }
            if (window.__DSB_BRIDGE__ && typeof window.__DSB_BRIDGE__.setDraft === 'function') {
              window.__DSB_BRIDGE__.setDraft('');
            }
            submitExternalHarnessPrompt(text);
            return true;
          };
          const isComposerSendButton = (btn, comp) => {
            if (!btn || btn.hasAttribute('data-dsb-session-chip')) return false;
            if (btn.type === 'submit') return true;
            const aria = String(btn.getAttribute('aria-label') || btn.getAttribute('title') || '');
            if (/send|submit|发送|提交/i.test(aria)) return true;
            const allBtns = Array.from(comp.querySelectorAll('button')).filter(
              (b) => !b.hasAttribute('data-dsb-session-chip') && b.offsetParent !== null,
            );
            return allBtns.length > 0 && allBtns[allBtns.length - 1] === btn;
          };
          const onKeyDown = (e) => {
            if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
              if (dsbRuntime.quickSearchOpen && dsbRuntime.quickSearchSelectedIdx >= 0) {
                return;
              }
              const comp = document.querySelector('[data-composer-card]');
              if (comp && e.target instanceof Node && comp.contains(e.target)) {
                if (dsbRuntime.activeHarness !== 'dsh') {
                  dispatchExternalSubmit(e);
                  return;
                }
                submitIntentUntilRef.current = Date.now() + 1800;
              }
            }
          };
          const onClick = (e) => {
            const comp = document.querySelector('[data-composer-card]');
            if (comp && e.target instanceof Element && comp.contains(e.target)) {
              const btn = e.target.closest('button');
              if (btn && !btn.hasAttribute('data-dsb-session-chip')) {
                if (dsbRuntime.activeHarness !== 'dsh' && isComposerSendButton(btn, comp)) {
                  dispatchExternalSubmit(e);
                  return;
                }
                submitIntentUntilRef.current = Date.now() + 1800;
              }
            }
          };
          document.addEventListener('keydown', onKeyDown, true);
          document.addEventListener('click', onClick, true);
          return () => {
            document.removeEventListener('keydown', onKeyDown, true);
            document.removeEventListener('click', onClick, true);
          };
        }, []);

        // Activity & explicit-intent message-submitted report.
        // Only report an assistant reply once it contains non-empty text so blank streaming
        // placeholders created right on Send never pop up an empty Lip bubble.
        const latestAssistantHasText = Boolean(
          latestAssistant && String(latestAssistant.text || '').trim().length > 0,
        );
        useEffect(() => {
          if (!IS_BAR) return;
          sendBarReport({
            kind: 'activity',
            sessionId: effectiveSessionId,
            latestAssistantId: latestAssistantHasText ? String(latestAssistant.id) : '',
            streaming: Boolean(latestAssistantHasText && latestAssistant && latestAssistant.streaming),
            userMessageCount,
          });
          if (activeHarness !== 'dsh') return;
          const prev = sessionSubBaselineRef.current;
          if (prev.sessionId !== sessionId) {
            sessionSubBaselineRef.current = { sessionId, userCount: userMessageCount };
          } else if (userMessageCount > prev.userCount) {
            sessionSubBaselineRef.current = { sessionId, userCount: userMessageCount };
            if (Date.now() < submitIntentUntilRef.current) {
              submitIntentUntilRef.current = 0;
              sendBarReport({ kind: 'submitted', sessionId });
            }
          } else if (userMessageCount < prev.userCount) {
            sessionSubBaselineRef.current = { sessionId, userCount: userMessageCount };
          }
        }, [
          activeHarness,
          effectiveSessionId,
          sessionId,
          latestAssistantHasText ? String(latestAssistant.id) : '',
          Boolean(latestAssistantHasText && latestAssistant && latestAssistant.streaming),
          userMessageCount,
        ]);

        // Scheme B: track unread replies arriving while Bar is Summoned with History Sheet closed
        useEffect(() => {
          if (!IS_BAR) return;
          const sKey = activeHarness + ':' + effectiveSessionId;
          const curId = latestAssistantHasText ? String(latestAssistant.id) : '';
          const prev = lastKnownAsstRef.current;
          if (prev.sessionKey !== sKey) {
            lastKnownAsstRef.current = { sessionKey: sKey, asstId: curId };
            return;
          }
          if (curId && prev.asstId && curId !== prev.asstId) {
            lastKnownAsstRef.current = { sessionKey: sKey, asstId: curId };
            if (barMode !== 'lip' && !isExpanded) {
              dsbRuntime.unreadCount = (dsbRuntime.unreadCount || 0) + 1;
              dsbRuntime.unreadPreview = stripMarkdownForLip(latestAssistant.text || '');
              notifyBarRuntime();
              if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
                dsbRuntime.scheduleLayoutReport(260);
              }
            }
          } else if (curId && !prev.asstId) {
            lastKnownAsstRef.current = { sessionKey: sKey, asstId: curId };
          }
        }, [activeHarness, effectiveSessionId, latestAssistantHasText ? String(latestAssistant.id) : '', barMode, isExpanded]);

        // Also detect queued message submission while streaming in Bar mode
        useEffect(() => {
          if (!IS_BAR || typeof document === 'undefined') return undefined;
          let lastQueueCount = 0;
          let prevSid = sessionId;
          const countQueueItems = () => {
            const qd = document.querySelector('[data-queue-dock]');
            if (!qd) return 0;
            const rect = qd.getBoundingClientRect();
            if (!rect || rect.height <= 1) return 0;
            const txt = qd.textContent || '';
            const m = txt.match(/(\d+)\s*条排队/);
            return m ? Number(m[1]) : 1;
          };
          lastQueueCount = countQueueItems();
          const mo = new MutationObserver(() => {
            const nextCount = countQueueItems();
            if (prevSid === sessionId && nextCount > lastQueueCount && Date.now() < submitIntentUntilRef.current) {
              submitIntentUntilRef.current = 0;
              sendBarReport({ kind: 'submitted', sessionId });
            }
            lastQueueCount = nextCount;
            prevSid = sessionId;
          });
          mo.observe(document.body, { childList: true, subtree: true, characterData: true });
          return () => mo.disconnect();
        }, [sessionId]);

        // UI state report
        useEffect(() => {
          if (!IS_BAR) return;
          const reportUi = () => {
            const activeEl = typeof document !== 'undefined' ? document.activeElement : null;
            const composerEl =
              typeof document !== 'undefined'
                ? dsbRuntime.activeHarness !== 'dsh'
                  ? document.querySelector('[data-dsb-external-composer]')
                  : document.querySelector('[data-composer-card]')
                : null;
            sendBarReport({
              kind: 'ui',
              hovered,
              pinned,
              popupOpen: dsbRuntime.popupOpen || dsbRuntime.quickSearchOpen,
              modal: dsbRuntime.modalOpen,
              composerFocused: Boolean(composerEl && activeEl && composerEl.contains(activeEl)),
            });
          };
          reportUi();
          dsbRuntime.listeners.add(reportUi);
          document.addEventListener('focusin', reportUi);
          document.addEventListener('focusout', reportUi);
          return () => {
            dsbRuntime.listeners.delete(reportUi);
            document.removeEventListener('focusin', reportUi);
            document.removeEventListener('focusout', reportUi);
          };
        }, [hovered, pinned]);

        // Esc keydown capture & composer pointerdown in Bar mode
        useEffect(() => {
          if (!IS_BAR || typeof document === 'undefined') return undefined;
          const onKeyDownCapture = (e) => {
            if (e.key !== 'Escape' || e.isComposing) return;
            const hasNativeMenu = Boolean(
              document.querySelector('[data-trigger-menu], [data-composer-overlay], [role="menu"], [role="listbox"], [role="dialog"], [data-radix-popper-content-wrapper], [data-floating-ui-portal]'),
            );
            if (dsbRuntime.popupOpen || dsbRuntime.quickSearchOpen || dsbRuntime.modalOpen || hasNativeMenu) return;
            e.preventDefault();
            e.stopPropagation();
            sendBarReport({ kind: 'step_down' });
          };
          const onPointerDownCapture = (e) => {
            const composerEl =
              dsbRuntime.activeHarness !== 'dsh'
                ? document.querySelector('[data-dsb-external-composer]')
                : document.querySelector('[data-composer-card]');
            if (composerEl && e.target instanceof Node && composerEl.contains(e.target)) {
              sendBarReport({ kind: 'composer_pointerdown' });
            }
          };
          document.addEventListener('keydown', onKeyDownCapture, true);
          document.addEventListener('pointerdown', onPointerDownCapture, true);
          return () => {
            document.removeEventListener('keydown', onKeyDownCapture, true);
            document.removeEventListener('pointerdown', onPointerDownCapture, true);
          };
        }, []);

        // Measure and report Bar layout + SEPARATE detached Win32 rounded-rect shapes to Tauri
        const measureAndReportLayout = useCallback(() => {
          if (!IS_BAR || typeof document === 'undefined') return;
          const cardEl = document.querySelector('[data-dsb-card]');
          const peekEl = document.querySelector('[data-dsb-floating-peek="1"]');
          const toastEl = document.querySelector('[data-dsb-floating-toast="1"]');
          const composerEl =
            barMode === 'lip'
              ? null
              : dsbRuntime.activeHarness !== 'dsh'
                ? document.querySelector('[data-dsb-external-composer]')
                : document.querySelector('[data-composer-card]');
          const queueEl =
            barMode === 'lip' || dsbRuntime.activeHarness !== 'dsh'
              ? null
              : document.querySelector('[data-queue-dock]');

          const cr = cardEl ? cardEl.getBoundingClientRect() : null;
          const prPeek = peekEl ? peekEl.getBoundingClientRect() : null;
          const prToast = toastEl ? toastEl.getBoundingClientRect() : null;
          const ir = composerEl ? composerEl.getBoundingClientRect() : null;
          const qr = queueEl ? queueEl.getBoundingClientRect() : null;
          const hasQueue = Boolean(qr && qr.width > 12 && qr.height > 4);

          const shapes = [];
          const cornerRadius = getHarnessMeta(dsbRuntime.activeHarness).cornerRadius || CARD_RADIUS_PX;

          // 1. Composer shape (100% pure native silhouette)
          if (ir && ir.width > 10 && ir.height > 10) {
            shapes.push({
              x: Math.round(ir.left),
              y: Math.round(ir.top),
              w: Math.round(ir.width),
              h: Math.round(ir.height),
              r: cornerRadius,
            });
          }

          // 2. Native DSH Queue Dock shape (untouched native rounded top cap!)
          if (hasQueue && qr) {
            shapes.push({
              x: Math.round(qr.left),
              y: Math.round(qr.top),
              w: Math.round(qr.width),
              h: Math.round(qr.height),
              r: 14,
            });
          }

          // 3. Detached Floating History Sheet / Lip Pill (with transparent 8px click-through gap!)
          if (cr && cr.width > 10 && cr.height > 10) {
            shapes.push({
              x: Math.round(cr.left),
              y: Math.round(cr.top),
              w: Math.round(cr.width),
              h: Math.round(cr.height),
              r: barMode === 'lip' ? 24 : cornerRadius,
            });
          }

          // 4. Detached Hover Peek or Unread Toast
          if (prPeek && prPeek.width > 12 && prPeek.height > 12) {
            shapes.push({
              x: Math.round(prPeek.left),
              y: Math.round(prPeek.top),
              w: Math.round(prPeek.width),
              h: Math.round(prPeek.height),
              r: 16,
            });
          }
          if (prToast && prToast.width > 12 && prToast.height > 12) {
            shapes.push({
              x: Math.round(prToast.left),
              y: Math.round(prToast.top),
              w: Math.round(prToast.width),
              h: Math.round(prToast.height),
              r: 20,
            });
          }

          // Compute exact bottom-up window height (zero dead Lip height when History is closed!)
          const liveMode = IS_BAR ? 'summon' : dsbRuntime.barMode;
          let targetHeight = COLLAPSED_LIP_PX;
          if (liveMode === 'lip') {
            targetHeight = COLLAPSED_LIP_PX;
          } else {
            if (!ir || ir.height <= 10) {
              // Composer is transiently mounting/switching; never collapse Win32 window to 56px!
              return;
            }
            const anchorTop = hasQueue && qr ? Math.min(ir.top, qr.top) : ir.top;
            const baseSpan = Math.ceil(Math.max(ir.height, ir.bottom - anchorTop));
            let detachedTop = 0;
            if (isExpanded) {
              detachedTop = expandedHeight + DETACHED_GAP_PX;
            } else if (prToast && prToast.height > 4) {
              detachedTop = Math.ceil(prToast.height) + DETACHED_GAP_PX;
            } else if (prPeek && prPeek.height > 4) {
              detachedTop = Math.ceil(prPeek.height) + DETACHED_GAP_PX;
            }
            targetHeight = Math.max(COLLAPSED_LIP_PX, baseSpan + detachedTop);
          }

          // 5. All open popups (Harness 3-level switcher, composer +/model/mode menus, AND DSH native +///@ popups!)
          const popupEls = document.querySelectorAll(
            '[data-dsb-popup], [data-trigger-menu], [data-composer-overlay]:not(:has([data-trigger-menu])), [role="menu"], [role="listbox"], [role="dialog"], [data-radix-popper-content-wrapper], [data-floating-ui-portal], [data-popover], [data-dsw-popover]',
          );
          for (const el of popupEls) {
            if (!(el instanceof HTMLElement)) continue;
            const cs = getComputedStyle(el);
            if (cs.display === 'none' || cs.visibility === 'hidden') continue;
            const pr = el.getBoundingClientRect();
            if (pr.width < 12 || pr.height < 12) continue;
            const subPanels = el.querySelectorAll('[data-dsb-popup-panel="1"]');
            if (subPanels && subPanels.length > 0) {
              for (const panel of subPanels) {
                if (!(panel instanceof HTMLElement)) continue;
                const pcs = getComputedStyle(panel);
                if (pcs.display === 'none' || pcs.visibility === 'hidden') continue;
                const sr = panel.getBoundingClientRect();
                if (sr.width < 12 || sr.height < 12) continue;
                shapes.push({
                  x: Math.round(sr.left),
                  y: Math.round(sr.top),
                  w: Math.round(sr.width),
                  h: Math.round(sr.height),
                  r: 14,
                });
              }
            } else {
              shapes.push({
                x: Math.round(pr.left),
                y: Math.round(pr.top),
                w: Math.round(pr.width),
                h: Math.round(pr.height),
                r: 12,
              });
            }
            const explicitBottom = Number(el.getAttribute('data-dsb-popup-bottom'));
            const minPopupH = Number(el.getAttribute('data-dsb-popup-min-height')) || 0;
            const effectivePopupH = Math.max(pr.height, minPopupH);
            const neededForPopup =
              Number.isFinite(explicitBottom) && explicitBottom > 0
                ? Math.ceil(explicitBottom + effectivePopupH + 8)
                : Math.ceil(window.innerHeight - pr.top + 8);
            if (neededForPopup > targetHeight) {
              targetHeight = neededForPopup;
            }
          }

          // Monotonic height lock while any ATP or DSH trigger popup stays open so hovering/filtering
          // between different Harnesses / Workspaces / Commands NEVER resizes or shakes the Win32 window!
          const hasStickyPopup = Boolean(
            document.querySelector('[data-dsb-popup], [data-trigger-menu]'),
          );
          if (hasStickyPopup) {
            const locked = Math.max(dsbRuntime.popupHeightLock || 0, targetHeight);
            dsbRuntime.popupHeightLock = locked;
            targetHeight = locked;
          } else {
            dsbRuntime.popupHeightLock = 0;
          }

          targetHeight = Math.max(COLLAPSED_LIP_PX, Math.min(880, Math.ceil(targetHeight)));
          const deltaY = Math.round(targetHeight - window.innerHeight);
          const adjustedShapes =
            deltaY !== 0
              ? shapes.map((s) => ({
                  x: s.x,
                  y: Math.max(0, s.y + deltaY),
                  w: s.w,
                  h: s.h,
                  r: s.r,
                }))
              : shapes;
          sendBarReport({
            kind: 'layout',
            targetHeight,
            width: Math.round(barWidth),
            shapes: adjustedShapes,
          });
        }, [barMode, barWidth, isExpanded, expandedHeight, activeHarness, hoverPeek, composerPopover]);

        const scheduleLayoutReport = useCallback(
          (durationMs = 280) => {
            if (!IS_BAR || typeof window === 'undefined') return;
            const until = performance.now() + durationMs;
            if (until > layoutAnimUntilRef.current) {
              layoutAnimUntilRef.current = until;
            }
            if (layoutRafRef.current) return;
            const step = () => {
              layoutRafRef.current = 0;
              measureAndReportLayout();
              if (performance.now() < layoutAnimUntilRef.current) {
                layoutRafRef.current = window.requestAnimationFrame(step);
              }
            };
            layoutRafRef.current = window.requestAnimationFrame(step);
          },
          [measureAndReportLayout],
        );

        useEffect(() => {
          dsbRuntime.scheduleLayoutReport = scheduleLayoutReport;
          return () => {
            dsbRuntime.scheduleLayoutReport = null;
          };
        }, [scheduleLayoutReport]);

        useEffect(() => {
          if (!IS_BAR || typeof document === 'undefined') return undefined;
          scheduleLayoutReport(340);
          const mo = new MutationObserver(() => scheduleLayoutReport(280));
          mo.observe(document.body, { childList: true, subtree: true, attributes: true });
          const ro = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(() => scheduleLayoutReport(120)) : null;
          const comp = document.querySelector('[data-composer-card]');
          const card = document.querySelector('[data-dsb-card]');
          if (ro && comp) ro.observe(comp);
          if (ro && card) ro.observe(card);
          window.addEventListener('resize', measureAndReportLayout);
          const onVisChange = () => {
            if (document.visibilityState === 'visible') {
              dsbRuntime.lastReportKeys.delete('layout');
              scheduleLayoutReport(320);
            }
          };
          document.addEventListener('visibilitychange', onVisChange);
          return () => {
            mo.disconnect();
            if (ro) ro.disconnect();
            window.removeEventListener('resize', measureAndReportLayout);
            document.removeEventListener('visibilitychange', onVisChange);
            if (layoutRafRef.current) {
              window.cancelAnimationFrame(layoutRafRef.current);
              layoutRafRef.current = 0;
            }
          };
        }, [scheduleLayoutReport, measureAndReportLayout, barMode, isExpanded, expandedHeight, barWidth]);

        // 2-line lip overflow detection + scroll to latest tail
        useLayoutEffect(() => {
          if (isExpanded) return;
          const vp = lipViewportRef.current;
          if (!vp) return;
          vp.scrollTop = vp.scrollHeight;
          const isOverflowing = vp.scrollHeight > vp.clientHeight + 1;
          setLipOverflow((prev) => (prev === isOverflowing ? prev : isOverflowing));
        }, [latestSingleLine, isExpanded, matchedCardWidth, barMode, barWidth]);

        // Match upper card width/position/surface 1:1 with active composer (DSH or 1:1 Antigravity/Codex replica)
        useLayoutEffect(() => {
          const dockEl = wrapRef.current;
          if (!dockEl || typeof ResizeObserver === 'undefined') return;
          const findComposer = () =>
            (dsbRuntime.activeHarness !== 'dsh'
              ? document.querySelector('[data-dsb-external-composer]')
              : null) || document.querySelector('[data-composer-card]');

          const applyDividerTheme = (cardEl, bgColor) => {
            if (!cardEl) return;
            let isDark = true;
            if (typeof bgColor === 'string') {
              const m = bgColor.match(/rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/i);
              if (m) {
                const lum = (0.299 * Number(m[1]) + 0.587 * Number(m[2]) + 0.114 * Number(m[3])) / 255;
                isDark = lum < 0.5;
              }
            }
            if (isDark) {
              cardEl.style.setProperty('--dsb-divider-line', 'rgba(0,0,0,0.32)');
              cardEl.style.setProperty('--dsb-divider-glow', 'rgba(0,0,0,0.10)');
            } else {
              cardEl.style.setProperty('--dsb-divider-line', 'rgba(0,0,0,0.075)');
              cardEl.style.setProperty('--dsb-divider-glow', 'rgba(0,0,0,0.025)');
            }
          };

          // Read-only Queue Dock rect inspection in Bar mode (NEVER mutates native DSH [data-queue-dock] styles!)
          const readQueueDockRect = () => {
            if (dsbRuntime.activeHarness !== 'dsh') return null;
            const queueDock = document.querySelector('[data-queue-dock]');
            const qr = queueDock ? queueDock.getBoundingClientRect() : null;
            return qr && qr.height > 4 ? qr : null;
          };

          const syncWidth = () => {
            const composerCard = findComposer();
            const wrap = wrapRef.current;
            if (!wrap) return;
            const cardEl = wrap.querySelector('[data-dsb-card]');
            const floatAnchorEl =
              wrap.querySelector('[data-dsb-floating-anchor="1"]') || cardEl;

            if (IS_BAR) {
              if (composerCard && barMode !== 'lip') {
                const rect = composerCard.getBoundingClientRect();
                if (rect.width > 120) {
                  const px = Math.round(rect.width * 100) / 100;
                  wrap.style.width = px + 'px';
                  wrap.style.maxWidth = px + 'px';
                  wrap.style.minWidth = px + 'px';
                  wrap.style.transform = 'none';
                  const own = wrap.getBoundingClientRect();
                  const dx = Math.round((rect.left - own.left) * 100) / 100;
                  if (Math.abs(dx) > 0.01) wrap.style.transform = 'translateX(' + dx + 'px)';
                }
              } else {
                wrap.style.width = '100vw';
                wrap.style.maxWidth = '100vw';
                wrap.style.minWidth = '100vw';
                wrap.style.transform = 'none';
                const own = wrap.getBoundingClientRect();
                if (Math.abs(own.left) > 0.01) {
                  wrap.style.transform = 'translateX(' + (-Math.round(own.left * 100) / 100) + 'px)';
                }
              }
              if (composerCard && cardEl) {
                const cs = getComputedStyle(composerCard);
                if (dsbRuntime.activeHarness === 'dsh') {
                  if (cs.backgroundColor && cs.backgroundColor !== 'rgba(0, 0, 0, 0)') {
                    cardEl.style.background = cs.backgroundColor;
                  }
                  cardEl.style.color = cs.color;
                  cardEl.style.fontSize = cs.fontSize;
                  cardEl.style.fontWeight = cs.fontWeight;
                  cardEl.style.fontFamily = cs.fontFamily;
                  applyDividerTheme(cardEl, cs.backgroundColor);
                }
              }
              if (composerCard && floatAnchorEl) {
                if (barMode !== 'lip') {
                  const rect = composerCard.getBoundingClientRect();
                  const qr = readQueueDockRect();
                  const anchorTop = qr ? Math.min(rect.top, qr.top) : rect.top;
                  const wr = wrap.getBoundingClientRect();
                  const gap = Math.round((anchorTop - wr.bottom) * 100) / 100;
                  floatAnchorEl.style.bottom = (-gap + DETACHED_GAP_PX) + 'px';
                } else {
                  floatAnchorEl.style.bottom = '0px';
                }
              }
              setMatchedCardWidth(barWidth);
              return;
            }

            if (!composerCard) return;
            const rect = composerCard.getBoundingClientRect();
            if (rect.width <= 120) return;

            const px = Math.round(rect.width * 100) / 100;
            wrap.style.width = px + 'px';
            wrap.style.maxWidth = px + 'px';
            wrap.style.minWidth = px + 'px';

            wrap.style.transform = 'none';
            const own = wrap.getBoundingClientRect();
            const dx = Math.round((rect.left - own.left) * 100) / 100;
            if (Math.abs(dx) > 0.01) wrap.style.transform = 'translateX(' + dx + 'px)';

            const cs = getComputedStyle(composerCard);
            if (cardEl) {
              cardEl.style.background = cs.backgroundColor;
              cardEl.style.color = cs.color;
              cardEl.style.fontSize = cs.fontSize;
              cardEl.style.fontWeight = cs.fontWeight;
              cardEl.style.fontFamily = cs.fontFamily;
              applyDividerTheme(cardEl, cs.backgroundColor);

              const qr = readQueueDockRect();
              const anchorTop = qr ? Math.min(rect.top, qr.top) : rect.top;
              const wr = wrap.getBoundingClientRect();
              const gap = Math.round((anchorTop - wr.bottom) * 100) / 100;
              cardEl.style.bottom = (-gap) + 'px';
            }

            setMatchedCardWidth(Math.round(px));
          };

          syncWidth();
          const composerCard = findComposer();
          const ro = new ResizeObserver(syncWidth);
          if (composerCard) ro.observe(composerCard);
          const rafTick = (frames) => {
            syncWidth();
            if (frames > 0) requestAnimationFrame(() => rafTick(frames - 1));
          };
          rafTick(24);
          window.addEventListener('resize', syncWidth);
          return () => {
            ro.disconnect();
            window.removeEventListener('resize', syncWidth);
          };
        }, [barMode, barWidth, activeHarness]);

        useEffect(() => {
          if (isExpanded && scrollRef.current) {
            scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
          }
          if (IS_BAR && isExpanded) {
            refreshHarnessAvailability();
          }
        }, [isExpanded, entries.length, latestSingleLine]);

        // =====================================================================
        // Bidirectional Live Twin Sync (Focus-aware Session + Same-Session Draft)
        // =====================================================================
        const applyIncomingTwinState = useCallback(
          (msg, isSnapshot = false) => {
            if (!msg || msg.sourceClientId === SELF_CLIENT_ID) return;

            if (IS_BAR) {
              // Never let an initial SSE snapshot or a bare root `/` path kick the Bar off an open
              // session back to the landing page ("选择一个工作区开始") where conversation.input.dock unmounts.
              if (!isSnapshot) {
                const incomingId = typeof msg.sessionId === 'string' ? msg.sessionId : '';
                const incomingPath = typeof msg.sessionPath === 'string' ? msg.sessionPath : '';
                if (incomingId && incomingId !== sessionId) {
                  switchToSession(incomingId, incomingPath);
                  return;
                }
                if (
                  !incomingId &&
                  incomingPath &&
                  incomingPath.startsWith('/') &&
                  incomingPath !== '/' &&
                  incomingPath !== window.location.pathname
                ) {
                  switchToSession('', incomingPath);
                  return;
                }
              }
            }

            // Draft mirroring ONLY on live events (never wipe local composer from a stale SSE snapshot
            // or while a local capsule insertion / typing edit is in progress!)
            if (
              !isSnapshot &&
              Date.now() >= dsbRuntime.localDraftProtectUntil &&
              typeof msg.draft === 'string' &&
              msg.draftSessionId &&
              msg.draftSessionId === sessionId &&
              inputActions &&
              typeof inputActions.setDraft === 'function' &&
              msg.draft !== liveDraft
            ) {
              lastRemoteDraftRef.current = msg.draft;
              inputActions.setDraft(msg.draft);
            }

            if (typeof msg.agentName === 'string' && msg.agentName !== agentName) {
              setAgentName(msg.agentName);
            }
            if (!IS_BAR && typeof msg.pinned === 'boolean' && msg.pinned !== pinned) {
              setPinned(msg.pinned);
            }
          },
          [inputActions, liveDraft, agentName, pinned, sessionId],
        );

        const applyIncomingTwinRef = useRef(applyIncomingTwinState);
        applyIncomingTwinRef.current = applyIncomingTwinState;

        // WebUI tab focus / visibility sync
        useEffect(() => {
          if (IS_BAR || typeof window === 'undefined') return undefined;
          const sendFocusSync = () => {
            const payload = {
              sourceClientId: SELF_CLIENT_ID,
              sessionPath: window.location.pathname,
              sessionId: sessionId || undefined,
              reason: 'focus',
            };
            if (broadcastChan) {
              try { broadcastChan.postMessage(payload); } catch {}
            }
            postTwinSync(payload);
          };
          const onVisibility = () => {
            if (document.visibilityState === 'visible') sendFocusSync();
          };
          window.addEventListener('focus', sendFocusSync);
          document.addEventListener('visibilitychange', onVisibility);
          return () => {
            window.removeEventListener('focus', sendFocusSync);
            document.removeEventListener('visibilitychange', onVisibility);
          };
        }, [sessionId]);

        // Broadcast local draft & route changes
        useEffect(() => {
          if (lastRemoteDraftRef.current === liveDraft) {
            lastRemoteDraftRef.current = null;
            return;
          }
          const payload = {
            sourceClientId: SELF_CLIENT_ID,
            reason: 'route',
            sessionPath: IS_BAR ? undefined : window.location.pathname,
            sessionId: IS_BAR ? undefined : (sessionId || undefined),
            draft: liveDraft,
            draftSessionId: sessionId || '',
          };
          if (broadcastChan) {
            try { broadcastChan.postMessage(payload); } catch {}
          }
          const timer = setTimeout(() => {
            postTwinSync(payload);
          }, 16);
          return () => clearTimeout(timer);
        }, [liveDraft, sessionId]);

        // Listen on BroadcastChannel + Host SSE (mount ONCE; never reconnect on keystrokes!)
        useEffect(() => {
          const onBc = (ev) => {
            if (typeof applyIncomingTwinRef.current === 'function') {
              applyIncomingTwinRef.current(ev.data, false);
            }
          };
          if (broadcastChan) {
            broadcastChan.addEventListener('message', onBc);
          }

          let es = null;
          if (typeof EventSource !== 'undefined') {
            es = new EventSource('/anything-to-prompt/events');
            es.addEventListener('twin-sync', (ev) => {
              try {
                if (typeof applyIncomingTwinRef.current === 'function') {
                  applyIncomingTwinRef.current(JSON.parse(ev.data), false);
                }
              } catch {}
            });
            es.addEventListener('snapshot', (ev) => {
              try {
                if (typeof applyIncomingTwinRef.current === 'function') {
                  applyIncomingTwinRef.current(JSON.parse(ev.data), true);
                }
              } catch {}
            });
          }

          return () => {
            if (broadcastChan) broadcastChan.removeEventListener('message', onBc);
            if (es) es.close();
          };
        }, []);

        const clearCollapseTimer = () => {
          if (collapseTimerRef.current) {
            clearTimeout(collapseTimerRef.current);
            collapseTimerRef.current = null;
          }
        };

        const handleMouseEnter = (e) => {
          if (e && e.buttons !== 0) return;
          clearCollapseTimer();
          if (hoverTimerRef.current) {
            clearTimeout(hoverTimerRef.current);
            hoverTimerRef.current = null;
          }
          hoverTimerRef.current = setTimeout(() => {
            hoverTimerRef.current = null;
            setHovered(true);
          }, HOVER_EXPAND_DELAY_MS);
        };

        const handleMouseLeave = () => {
          if (hoverTimerRef.current) {
            clearTimeout(hoverTimerRef.current);
            hoverTimerRef.current = null;
          }
          if (isResizingHeightRef.current || isResizingWidthRef.current || editingName) return;
          clearCollapseTimer();
          collapseTimerRef.current = setTimeout(() => {
            setHovered(false);
          }, 160);
        };

        const togglePin = (e) => {
          e.stopPropagation();
          setPinned((prev) => {
            const next = !prev;
            try {
              localStorage.setItem(STORAGE_KEY_PINNED, next ? '1' : '0');
            } catch {}
            postTwinSync({ pinned: next });
            return next;
          });
        };

        const handleStartResizeHeight = useCallback(
          (e) => {
            e.preventDefault();
            e.stopPropagation();
            isResizingHeightRef.current = true;
            const startY = e.clientY;
            const startH = expandedHeight;

            const onMove = (moveEv) => {
              const deltaY = startY - moveEv.clientY;
              const nextH = Math.max(
                MIN_EXPANDED_PX,
                Math.min(MAX_EXPANDED_PX, Math.round(startH + deltaY)),
              );
              setExpandedHeight(nextH);
            };

            const onUp = (upEv) => {
              isResizingHeightRef.current = false;
              const deltaY = startY - upEv.clientY;
              const finalH = Math.max(
                MIN_EXPANDED_PX,
                Math.min(MAX_EXPANDED_PX, Math.round(startH + deltaY)),
              );
              try {
                localStorage.setItem(STORAGE_KEY_HEIGHT, String(finalH));
              } catch {}
              postTwinSync({ expandedHeightPx: finalH });
              window.removeEventListener('mousemove', onMove);
              window.removeEventListener('mouseup', onUp);
            };

            window.addEventListener('mousemove', onMove);
            window.addEventListener('mouseup', onUp);
          },
          [expandedHeight],
        );

        const handleStartResizeWidth = useCallback(
          (side) => (e) => {
            if (!IS_BAR) return;
            e.preventDefault();
            e.stopPropagation();
            isResizingWidthRef.current = true;
            const startScreenX = e.screenX;
            const startW = dsbRuntime.barWidth;

            const onMove = (moveEv) => {
              const rawDelta = moveEv.screenX - startScreenX;
              const signedDelta = side === 'left' ? -rawDelta : rawDelta;
              const nextW = Math.max(
                MIN_BAR_WIDTH_PX,
                Math.min(MAX_BAR_WIDTH_PX, Math.round(startW + signedDelta * 2)),
              );
              dsbRuntime.barWidth = nextW;
              setBarWidth(nextW);
              scheduleLayoutReport(80);
            };

            const onUp = (upEv) => {
              isResizingWidthRef.current = false;
              const rawDelta = upEv.screenX - startScreenX;
              const signedDelta = side === 'left' ? -rawDelta : rawDelta;
              const finalW = Math.max(
                MIN_BAR_WIDTH_PX,
                Math.min(MAX_BAR_WIDTH_PX, Math.round(startW + signedDelta * 2)),
              );
              dsbRuntime.barWidth = finalW;
              setBarWidth(finalW);
              try {
                localStorage.setItem(STORAGE_KEY_BAR_WIDTH, String(finalW));
              } catch {}
              postTwinSync({ widthPx: finalW });
              scheduleLayoutReport(120);
              window.removeEventListener('mousemove', onMove);
              window.removeEventListener('mouseup', onUp);
            };

            window.addEventListener('mousemove', onMove);
            window.addEventListener('mouseup', onUp);
          },
          [scheduleLayoutReport],
        );

        const commitAgentName = () => {
          const trimmed = nameDraft.trim() || '小张';
          setAgentName(trimmed);
          setNameDraft(trimmed);
          setEditingName(false);
          try {
            localStorage.setItem(STORAGE_KEY_AGENT_NAME, trimmed);
          } catch {}
          postTwinSync({ agentName: trimmed });
        };

        const renderAgentPrefix = () => {
          if (editingName) {
            return h('input', {
              type: 'text',
              autoFocus: true,
              value: nameDraft,
              onChange: (ev) => setNameDraft(ev.target.value),
              onBlur: commitAgentName,
              onKeyDown: (ev) => {
                if (ev.key === 'Enter') commitAgentName();
                if (ev.key === 'Escape') {
                  ev.stopPropagation();
                  setNameDraft(agentName);
                  setEditingName(false);
                }
              },
              onClick: (ev) => ev.stopPropagation(),
              style: {
                width: Math.max(54, nameDraft.length * 10 + 18) + 'px',
                background: 'rgba(128,128,128,0.16)',
                color: 'inherit',
                border: '1px solid rgba(110,168,254,0.65)',
                borderRadius: '6px',
                padding: '1px 6px',
                fontSize: '12.5px',
                fontWeight: 600,
                outline: 'none',
              },
            });
          }
          return h(
            'span',
            {
              title: '双击修改显示名称',
              onDoubleClick: (ev) => {
                ev.stopPropagation();
                setNameDraft(agentName);
                setEditingName(true);
              },
              style: {
                fontWeight: 650,
                flexShrink: 0,
                cursor: 'text',
                userSelect: 'none',
              },
            },
            agentName + '：',
          );
        };

        // Scheme B 3-in-1 Top-Inside Handle: Drag (>4px) moves window | Click (<=4px) toggles Detached History | Hover (240ms) peeks latest reply | Double-click resets center
        const toggleHistorySheet = useCallback(() => {
          if (hoverPeekTimerRef.current) {
            clearTimeout(hoverPeekTimerRef.current);
            hoverPeekTimerRef.current = null;
          }
          setHoverPeek(false);
          setHistoryOpen((prev) => {
            const next = !prev;
            if (next) {
              dsbRuntime.unreadCount = 0;
              dsbRuntime.unreadPreview = '';
            } else {
              setPreviewExpanded(false);
              setPinned((wasPin) => {
                if (wasPin) {
                  try { localStorage.setItem(STORAGE_KEY_PINNED, '0'); } catch {}
                }
                return false;
              });
            }
            notifyBarRuntime();
            scheduleLayoutReport(300);
            return next;
          });
        }, [scheduleLayoutReport]);

        const handleStartDragOrToggle = useCallback(
          (e) => {
            if (!IS_BAR || e.button !== 0) return;
            e.preventDefault();
            e.stopPropagation();
            if (hoverPeekTimerRef.current) {
              clearTimeout(hoverPeekTimerRef.current);
              hoverPeekTimerRef.current = null;
            }
            const startX = e.screenX;
            const startY = e.screenY;
            let dragged = false;
            let dragStarted = false;
            let active = true;
            let rafId = 0;
            const step = () => {
              rafId = 0;
              if (!active || !dragStarted) return;
              tauriInvoke('bar_drag_step').catch(() => {});
            };
            const onMove = (mv) => {
              if (!active) return;
              if (!dragged && (Math.abs(mv.screenX - startX) > 4 || Math.abs(mv.screenY - startY) > 4)) {
                dragged = true;
                dragStarted = true;
                setHoverPeek(false);
                tauriInvoke('bar_drag_begin').catch(() => {});
              }
              if (dragStarted && !rafId) {
                rafId = requestAnimationFrame(step);
              }
            };
            const onUp = () => {
              active = false;
              if (rafId) cancelAnimationFrame(rafId);
              if (dragStarted) {
                tauriInvoke('bar_drag_end').catch(() => {});
              } else {
                toggleHistorySheet();
              }
              window.removeEventListener('mousemove', onMove);
              window.removeEventListener('mouseup', onUp);
            };
            window.addEventListener('mousemove', onMove);
            window.addEventListener('mouseup', onUp);
          },
          [toggleHistorySheet],
        );

        const handleResetWindowPos = useCallback((e) => {
          if (!IS_BAR) return;
          e.preventDefault();
          e.stopPropagation();
          tauriInvoke('bar_drag_reset').catch(() => {});
        }, []);

        const handleHandleMouseEnter = useCallback(() => {
          if (!IS_BAR || barMode === 'lip' || isExpanded || (dsbRuntime.unreadCount || 0) > 0) return;
          if (!latestSingleLine) return;
          if (hoverPeekTimerRef.current) clearTimeout(hoverPeekTimerRef.current);
          hoverPeekTimerRef.current = setTimeout(() => {
            setHoverPeek(true);
            scheduleLayoutReport(220);
          }, 240);
        }, [barMode, isExpanded, latestSingleLine, scheduleLayoutReport]);

        const handleHandleMouseLeave = useCallback(() => {
          if (hoverPeekTimerRef.current) {
            clearTimeout(hoverPeekTimerRef.current);
            hoverPeekTimerRef.current = null;
          }
          setHoverPeek((prev) => {
            if (prev) scheduleLayoutReport(180);
            return false;
          });
        }, [scheduleLayoutReport]);

        const renderDragHandle = (extraStyle) => {
          if (!IS_BAR || barMode === 'lip') return null;
          const hasUnread = !isExpanded && (dsbRuntime.unreadCount || 0) > 0;
          return h(
            'div',
            {
              'data-dsb-drag-handle': '1',
              title: isExpanded
                ? '单击收起聊天记录 · 按住拖动窗口 · 双击复位居中'
                : '单击展开聊天记录 · 悬停轻瞥最新回复 · 按住拖动窗口 · 双击复位',
              onMouseDown: handleStartDragOrToggle,
              onDoubleClick: handleResetWindowPos,
              onMouseEnter: handleHandleMouseEnter,
              onMouseLeave: handleHandleMouseLeave,
              style: Object.assign(
                {
                  position: 'absolute',
                  top: '1px',
                  left: '50%',
                  transform: 'translateX(-50%)',
                  width: '72px',
                  height: '13px',
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  gap: '5px',
                  cursor: 'pointer',
                  zIndex: 25,
                  userSelect: 'none',
                },
                extraStyle || null,
              ),
            },
            h('span', {
              style: {
                width: '36px',
                height: '3.5px',
                borderRadius: '999px',
                background: isExpanded
                  ? 'rgba(125, 211, 252, 0.72)'
                  : 'rgba(165, 172, 186, 0.36)',
                pointerEvents: 'none',
                transition: 'all 0.16s ease',
              },
            }),
            hasUnread
              ? h('span', {
                  style: {
                    width: '6px',
                    height: '6px',
                    borderRadius: '999px',
                    background: '#38bdf8',
                    animation: 'dsbUnreadPulse 2s infinite',
                    pointerEvents: 'none',
                    flexShrink: 0,
                  },
                })
              : null,
          );
        };

        const renderPinButton = () =>
          h(
            'button',
            {
              type: 'button',
              onClick: togglePin,
              'aria-label': pinned ? '取消固定' : '固定展开',
              'aria-pressed': pinned ? 'true' : 'false',
              title: pinned
                ? '取消常驻置顶（恢复点击外部自动关闭）'
                : '固定常驻前台（点击其他区域不关闭）',
              style: {
                width: '20px',
                height: '20px',
                padding: 0,
                borderRadius: '6px',
                border: 'none',
                outline: 'none',
                background: 'transparent',
                color: pinned ? '#ffffff' : 'inherit',
                opacity: pinned ? 1 : 0.62,
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
                cursor: 'pointer',
                flexShrink: 0,
                transition: 'opacity 0.15s ease, color 0.15s ease',
              },
            },
            h(
              'svg',
              {
                xmlns: 'http://www.w3.org/2000/svg',
                viewBox: '0 0 24 24',
                width: 14,
                height: 14,
                fill: pinned ? 'currentColor' : 'none',
                stroke: 'currentColor',
                strokeWidth: pinned ? 1.5 : 1.65,
                strokeLinecap: 'round',
                strokeLinejoin: 'round',
                style: {
                  display: 'block',
                  transform: 'rotate(38deg)',
                },
              },
              h('path', {
                d: 'M12 16v6',
              }),
              h('path', {
                d: 'M9.8 4.2h4.4a.8.8 0 0 1 .6 1.32l-.6.7v4.05l1.95 2.08a1 1 0 0 1 .27.69V14a.8.8 0 0 1-.8.8H8.38a.8.8 0 0 1-.8-.8v-.96a1 1 0 0 1 .27-.69l1.95-2.08V6.22l-.6-.7a.8.8 0 0 1 .6-1.32z',
              }),
            ),
          );

        // Listary-style instant pure-text file & app quick search (Option B: Tab selects, Enter inserts blue capsule, Ctrl+Enter opens)
        const insertQuickSearchHitAsCapsule = useCallback((hit) => {
          if (!hit || !hit.path) return;
          quickSyncedPreviewRef.current = null;
          quickTypedQueryRef.current = '';
          setQuickHits([]);
          setQuickSelectedIdx(-1);
          quickHitsRef.current = [];
          quickSelectedRef.current = -1;
          dsbRuntime.quickSearchOpen = false;
          dsbRuntime.quickSearchSelectedIdx = -1;
          notifyBarRuntime();
          tauriInvoke('quick_search_build_capsule', {
            path: hit.path,
            name: hit.name || '',
            kind: hit.kind || 'file',
          })
            .then((cap) => {
              if (!cap || !cap.text) return;
              const bridge = window.__DSB_BRIDGE__;
              if (bridge && typeof bridge.setDraft === 'function') {
                bridge.setDraft('');
              }
              setTimeout(() => {
                if (bridge && typeof bridge.insertCapsule === 'function') {
                  bridge.insertCapsule({
                    summary: {
                      label: cap.summaryLabel,
                      title: cap.summaryTitle,
                      detail: cap.summaryDetail,
                      body: cap.summaryBody,
                    },
                    text: cap.text,
                  });
                }
              }, 35);
            })
            .catch(() => {});
        }, []);

        const openQuickSearchHitDirectly = useCallback((hit) => {
          if (!hit || !hit.path) return;
          quickSyncedPreviewRef.current = null;
          quickTypedQueryRef.current = '';
          setQuickHits([]);
          setQuickSelectedIdx(-1);
          quickHitsRef.current = [];
          quickSelectedRef.current = -1;
          dsbRuntime.quickSearchOpen = false;
          dsbRuntime.quickSearchSelectedIdx = -1;
          if (window.__DSB_BRIDGE__ && typeof window.__DSB_BRIDGE__.setDraft === 'function') {
            window.__DSB_BRIDGE__.setDraft('');
          }
          notifyBarRuntime();
          tauriInvoke('quick_search_open_path', { path: hit.path }).catch(() => {});
        }, []);

        useEffect(() => {
          quickHitsRef.current = quickHits;
          quickSelectedRef.current = quickSelectedIdx;
          dsbRuntime.quickSearchOpen = quickHits.length > 0;
          dsbRuntime.quickSearchSelectedIdx = quickSelectedIdx;
          notifyBarRuntime();
          if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
            dsbRuntime.scheduleLayoutReport(240);
          }
        }, [quickHits, quickSelectedIdx]);

        const effectiveQuickDraft =
          dsbRuntime.activeHarness !== 'dsh' ? dsbRuntime.extDraftText || '' : liveDraft;
        const effectiveQuickHasCapsules =
          dsbRuntime.activeHarness !== 'dsh'
            ? (dsbRuntime.extCapsules || []).length > 0
            : countComposerCapsules(liveDraft) > 0;

        useEffect(() => {
          if (!IS_BAR || barMode === 'lip' || dsbRuntime.popupOpen || composerPopover) {
            if (quickHitsRef.current.length > 0) {
              setQuickHits([]);
              setQuickSelectedIdx(-1);
            }
            return undefined;
          }
          const raw = typeof effectiveQuickDraft === 'string' ? effectiveQuickDraft : '';
          const trimmed = raw.trim();
          const hasCapsules = effectiveQuickHasCapsules;
          const isPureQuery =
            !hasCapsules &&
            trimmed.length >= 1 &&
            trimmed.length <= 60 &&
            !raw.includes('\n') &&
            !raw.includes('\r') &&
            !trimmed.startsWith('/');

          if (!isPureQuery) {
            quickQueryRef.current = '';
            quickDismissedQueryRef.current = '';
            quickTypedQueryRef.current = '';
            quickSyncedPreviewRef.current = null;
            if (quickHitsRef.current.length > 0) {
              setQuickHits([]);
              setQuickSelectedIdx(-1);
            }
            return undefined;
          }

          // When Tab/Arrow selection synced a candidate filename into the composer,
          // keep the current candidate list intact instead of re-querying on the synced filename!
          if (
            quickSyncedPreviewRef.current !== null &&
            trimmed === String(quickSyncedPreviewRef.current).trim()
          ) {
            return undefined;
          }
          quickSyncedPreviewRef.current = null;

          if (quickDismissedQueryRef.current && quickDismissedQueryRef.current === trimmed) {
            return undefined;
          }
          if (quickDismissedQueryRef.current && quickDismissedQueryRef.current !== trimmed) {
            quickDismissedQueryRef.current = '';
          }

          quickQueryRef.current = trimmed;
          quickTypedQueryRef.current = raw;
          const seq = ++quickReqSeqRef.current;
          const timer = setTimeout(() => {
            tauriInvoke('quick_search_query', { query: trimmed, limit: 8 })
              .then((hits) => {
                if (quickReqSeqRef.current !== seq) return;
                const list = Array.isArray(hits) ? hits : [];
                setQuickHits(list);
                setQuickSelectedIdx(-1);
                if (list.length > 0) {
                  const nowTs = Date.now();
                  const curMode = quickStatus && quickStatus.engineMode;
                  const alreadyFull = curMode === 'ntfs_hybrid' || curMode === 'everything_ipc';
                  const lastTs = dsbRuntime._lastQuickStatusAt || 0;
                  if (!alreadyFull || nowTs - lastTs > 15000) {
                    dsbRuntime._lastQuickStatusAt = nowTs;
                    tauriInvoke('quick_search_status')
                      .then((st) => {
                        if (st && typeof st === 'object') setQuickStatus(st);
                      })
                      .catch(() => {});
                  }
                }
              })
              .catch(() => {
                if (quickReqSeqRef.current !== seq) return;
                setQuickHits([]);
                setQuickSelectedIdx(-1);
              });
          }, 22);
          return () => clearTimeout(timer);
        }, [effectiveQuickDraft, effectiveQuickHasCapsules, barMode, dsbRuntime.popupOpen, composerPopover]);

        useEffect(() => {
          if (!IS_BAR || typeof document === 'undefined') return undefined;
          const onQuickKeyDown = (e) => {
            const hits = quickHitsRef.current;
            if (!hits || hits.length === 0) return;
            if (e.isComposing) return;

            const syncCandidateFilenameToComposer = (nextIdx) => {
              const target = nextIdx >= 0 ? hits[nextIdx] : null;
              const nextText = target && target.name ? String(target.name) : '';
              if (!nextText) return;
              quickSyncedPreviewRef.current = nextText;
              dsbRuntime.localDraftProtectUntil = Date.now() + 900;
              const bridge = window.__DSB_BRIDGE__;
              if (bridge && typeof bridge.setDraft === 'function') {
                bridge.setDraft(nextText);
              }
            };

            if (e.key === 'Tab' && !e.ctrlKey && !e.altKey && !e.metaKey) {
              e.preventDefault();
              e.stopPropagation();
              if (typeof e.stopImmediatePropagation === 'function') e.stopImmediatePropagation();
              const prev = quickSelectedRef.current;
              const nextIdx =
                prev < 0
                  ? e.shiftKey
                    ? hits.length - 1
                    : 0
                  : (prev + (e.shiftKey ? -1 : 1) + hits.length) % hits.length;
              quickSelectedRef.current = nextIdx;
              setQuickSelectedIdx(nextIdx);
              syncCandidateFilenameToComposer(nextIdx);
              return;
            }

            const sel = quickSelectedRef.current;
            if (sel >= 0 && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
              e.preventDefault();
              e.stopPropagation();
              if (typeof e.stopImmediatePropagation === 'function') e.stopImmediatePropagation();
              const delta = e.key === 'ArrowDown' ? 1 : -1;
              const nextIdx = (sel + delta + hits.length) % hits.length;
              quickSelectedRef.current = nextIdx;
              setQuickSelectedIdx(nextIdx);
              syncCandidateFilenameToComposer(nextIdx);
              return;
            }

            if (sel >= 0 && e.key === 'Enter' && !e.shiftKey) {
              const target = hits[sel];
              if (target) {
                e.preventDefault();
                e.stopPropagation();
                if (typeof e.stopImmediatePropagation === 'function') e.stopImmediatePropagation();
                if (e.ctrlKey || e.metaKey) {
                  openQuickSearchHitDirectly(target);
                } else {
                  insertQuickSearchHitAsCapsule(target);
                }
              }
              return;
            }

            if (e.key === 'Escape') {
              e.preventDefault();
              e.stopPropagation();
              if (typeof e.stopImmediatePropagation === 'function') e.stopImmediatePropagation();
              if (sel >= 0) {
                quickSelectedRef.current = -1;
                setQuickSelectedIdx(-1);
                const origTyped = quickTypedQueryRef.current;
                if (typeof origTyped === 'string' && origTyped.trim()) {
                  quickSyncedPreviewRef.current = origTyped;
                  const bridge = window.__DSB_BRIDGE__;
                  if (bridge && typeof bridge.setDraft === 'function') {
                    bridge.setDraft(origTyped);
                  }
                }
              } else {
                quickSyncedPreviewRef.current = null;
                quickDismissedQueryRef.current = quickQueryRef.current;
                setQuickHits([]);
                setQuickSelectedIdx(-1);
              }
            }
          };
          window.addEventListener('keydown', onQuickKeyDown, true);
          return () => window.removeEventListener('keydown', onQuickKeyDown, true);
        }, [insertQuickSearchHitAsCapsule, openQuickSearchHitDirectly]);

        const renderQuickSearchPopup = () => {
          if (!IS_BAR || barMode === 'lip' || quickHits.length === 0) return null;
          const cardEl = typeof document !== 'undefined' ? document.querySelector('[data-dsb-card]') : null;
          const compEl =
            typeof document !== 'undefined'
              ? dsbRuntime.activeHarness !== 'dsh'
                ? document.querySelector('[data-dsb-external-composer]')
                : document.querySelector('[data-composer-card]')
              : null;
          const cr = cardEl ? cardEl.getBoundingClientRect() : null;
          const ir = compEl ? compEl.getBoundingClientRect() : null;
          const validCard = cr && cr.width > 10 && cr.height > 10;
          const validComp = ir && ir.width > 10 && ir.height > 10;
          const slabSpan =
            validCard && validComp
              ? Math.round(Math.max(cr.bottom, ir.bottom) - Math.min(cr.top, ir.top))
              : validCard
                ? Math.round(cr.height)
                : 152;
          const bottomPx = Math.max(96, Math.min(360, slabSpan + 6));
          const theme = HARNESS_META[activeHarness] || HARNESS_META.dsh;

          const badgeMetaForHit = (hit) => {
            if (hit.kind === 'app') return { label: 'APP', bg: 'rgba(59,130,246,0.20)', color: '#93c5fd' };
            if (hit.kind === 'dir') return { label: 'DIR', bg: 'rgba(245,158,11,0.20)', color: '#fcd34d' };
            const ext = String(hit.ext || '').toUpperCase().slice(0, 5);
            if (ext === 'SLDPRT' || ext === 'SLDASM' || ext === 'SLDDRW') {
              return { label: 'CAD', bg: 'rgba(16,185,129,0.22)', color: '#6ee7b7' };
            }
            return { label: ext || 'FILE', bg: 'rgba(148,163,184,0.18)', color: '#cbd5e1' };
          };

          const panel = h(
            'div',
            {
              'data-dsb-popup': '1',
              'data-dsb-popup-bottom': String(bottomPx),
              'data-dsb-quick-search': '1',
              onMouseDown: (ev) => ev.preventDefault(),
              style: {
                position: 'fixed',
                left: '2px',
                right: '2px',
                bottom: bottomPx + 'px',
                borderRadius: '16px',
                padding: '6px',
                background: theme.surface,
                border: '1px solid ' + theme.border,
                boxShadow: '0 10px 28px rgba(0,0,0,0.42)',
                display: 'flex',
                flexDirection: 'column',
                gap: '2px',
                zIndex: 9998,
                pointerEvents: 'auto',
                color: 'var(--dsw-alias-label-primary, #f3f4f6)',
                fontFamily: 'system-ui, -apple-system, "Segoe UI", Roboto, "PingFang SC", sans-serif',
                boxSizing: 'border-box',
              },
            },
            quickHits.map((hit, idx) => {
              const isSel = idx === quickSelectedIdx;
              const bm = badgeMetaForHit(hit);
              return h(
                'div',
                {
                  key: hit.path,
                  onMouseEnter: () => setQuickSelectedIdx(idx),
                  onClick: () => insertQuickSearchHitAsCapsule(hit),
                  onDoubleClick: (ev) => {
                    ev.stopPropagation();
                    openQuickSearchHitDirectly(hit);
                  },
                  style: {
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    gap: '8px',
                    padding: '6px 10px',
                    borderRadius: '10px',
                    cursor: 'pointer',
                    background: isSel ? theme.accentBg : 'transparent',
                    border: isSel ? '1px solid ' + theme.accentBorder : '1px solid transparent',
                    transition: 'background 0.1s ease',
                  },
                },
                h(
                  'div',
                  {
                    style: {
                      display: 'flex',
                      alignItems: 'center',
                      gap: '8px',
                      minWidth: 0,
                      flex: 1,
                    },
                  },
                  h(
                    'span',
                    {
                      style: {
                        fontSize: '10px',
                        fontWeight: 700,
                        padding: '2px 5px',
                        borderRadius: '6px',
                        background: bm.bg,
                        color: bm.color,
                        flexShrink: 0,
                        minWidth: '30px',
                        textAlign: 'center',
                      },
                    },
                    bm.label,
                  ),
                  h(
                    'span',
                    {
                      style: {
                        fontSize: '12.5px',
                        fontWeight: isSel ? 600 : 500,
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                        flexShrink: 0,
                        maxWidth: '50%',
                      },
                    },
                    hit.name,
                  ),
                  hit.parentDisplay
                    ? h(
                        'span',
                        {
                          style: {
                            fontSize: '11px',
                            opacity: 0.52,
                            overflow: 'hidden',
                            textOverflow: 'ellipsis',
                            whiteSpace: 'nowrap',
                            flex: '1 1 0',
                            minWidth: 0,
                          },
                        },
                        hit.parentDisplay,
                      )
                    : null,
                ),
                h(
                  'div',
                  {
                    style: {
                      display: 'flex',
                      alignItems: 'center',
                      gap: '5px',
                      flexShrink: 0,
                    },
                  },
                  isSel
                    ? h(
                        'span',
                        {
                          style: {
                            fontSize: '10.5px',
                            padding: '1px 6px',
                            borderRadius: '6px',
                            background: 'rgba(255,255,255,0.09)',
                            color: theme.accent,
                            fontWeight: 600,
                          },
                        },
                        'Enter 引用',
                      )
                    : null,
                  h(
                    'button',
                    {
                      type: 'button',
                      title: '直接打开 (Ctrl+Enter)',
                      onClick: (ev) => {
                        ev.stopPropagation();
                        openQuickSearchHitDirectly(hit);
                      },
                      style: {
                        fontSize: '11px',
                        padding: '2px 7px',
                        borderRadius: '6px',
                        border: '1px solid rgba(255,255,255,0.14)',
                        background: 'rgba(255,255,255,0.06)',
                        color: 'rgba(229,231,235,0.82)',
                        cursor: 'pointer',
                      },
                    },
                    '打开',
                  ),
                ),
              );
            }),
            h(
              'div',
              {
                style: {
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  padding: '3px 10px 1px',
                  fontSize: '10.5px',
                  opacity: 0.52,
                  borderTop: '1px solid rgba(128,128,128,0.14)',
                  marginTop: '2px',
                },
              },
              h('span', null, 'Tab 选中 · Enter 插入蓝色胶囊 · Ctrl+Enter 直接打开 · Esc 隐藏'),
              h(
                'span',
                null,
                (quickStatus && quickStatus.engineMode === 'everything_ipc'
                  ? 'Everything 实时 · '
                  : quickStatus && quickStatus.engineMode === 'ntfs_hybrid'
                    ? 'NTFS 全盘(' + (Array.isArray(quickStatus.drives) ? quickStatus.drives.join('/') : '') + ') · '
                    : '高价值索引 · ') +
                  quickHits.length +
                  ' 项匹配',
              ),
            ),
          );

          if (ReactDOM && typeof ReactDOM.createPortal === 'function' && typeof document !== 'undefined' && document.body) {
            return ReactDOM.createPortal(panel, document.body);
          }
          return panel;
        };

        const cardBorderRadius =
          IS_BAR
            ? (barMode === 'lip' ? CARD_RADIUS_PX : getHarnessMeta(activeHarness).cornerRadius || CARD_RADIUS_PX) + 'px'
            : CARD_RADIUS_PX + 'px ' + CARD_RADIUS_PX + 'px 0 0';

        const extCapsules = dsbRuntime.extCapsules || [];
        const extAttachments = dsbRuntime.extAttachments || [];
        const extDraft = dsbRuntime.extDraftText || '';
        const activeTurn = dsbRuntime.externalActiveTurn;
        const isExtStreaming = Boolean(activeTurn && activeTurn.harness === activeHarness);
        const activeAvail =
          (dsbRuntime.harnessAvailability || []).find((a) => a && a.id === activeHarness) || null;
        const extSel = dsbRuntime.externalSelection[activeHarness] || {};
        const activeHarnessOpts = dsbRuntime.harnessOptions[activeHarness] || null;
        const activeHarnessPrefs = dsbRuntime.harnessPrefs[activeHarness] || {};

        const insertTextIntoExtComposer = (snippet) => {
          const prev = dsbRuntime.extDraftText || '';
          const sep = prev && !/\s$/.test(prev) ? ' ' : '';
          dsbRuntime.extDraftText = prev + sep + snippet;
          notifyBarRuntime();
          focusComposerElement();
          scheduleLayoutReport(200);
        };

        const pickFilesForExtComposer = (imagesOnly) => {
          const initDir = (extSel && extSel.workspacePath) || '';
          dsbRuntime.modalOpen = true;
          notifyBarRuntime();
          tauriInvoke('app_pick_files', { initialDir: initDir || null, imagesOnly: Boolean(imagesOnly) })
            .then((list) => {
              if (Array.isArray(list) && list.length > 0) addExtAttachments(list);
            })
            .catch(() => {})
            .finally(() => {
              dsbRuntime.modalOpen = false;
              notifyBarRuntime();
              focusComposerElement();
            });
        };

        const pickFolderForExtComposer = () => {
          const initDir = (extSel && extSel.workspacePath) || '';
          dsbRuntime.modalOpen = true;
          notifyBarRuntime();
          tauriInvoke('app_pick_folder', { initialDir: initDir || null })
            .then((picked) => {
              if (typeof picked === 'string' && picked.trim()) {
                return tauriInvoke('app_describe_paths', { paths: [picked.trim()] }).then((list) => {
                  if (Array.isArray(list) && list.length > 0) addExtAttachments(list);
                });
              }
              return null;
            })
            .catch(() => {})
            .finally(() => {
              dsbRuntime.modalOpen = false;
              notifyBarRuntime();
              focusComposerElement();
            });
        };

        const handleExtPaste = (ev) => {
          const cd = ev.clipboardData;
          if (!cd) return;
          const items = cd.items ? Array.from(cd.items) : [];
          const imageItems = items.filter((it) => it && it.kind === 'file' && /^image\//i.test(it.type || ''));
          if (imageItems.length > 0) {
            ev.preventDefault();
            for (const imgItem of imageItems) {
              const file = imgItem.getAsFile();
              if (!file) continue;
              const reader = new FileReader();
              reader.onload = () => {
                const dataUrl = typeof reader.result === 'string' ? reader.result : '';
                if (!dataUrl) return;
                tauriInvoke('app_save_clipboard_image', {
                  base64Data: dataUrl,
                  mime: file.type || 'image/png',
                })
                  .then((att) => {
                    if (att) addExtAttachments([att]);
                  })
                  .catch(() => {});
              };
              reader.readAsDataURL(file);
            }
            return;
          }
          // Check Windows CF_HDROP when user copied files or folders in Explorer (Ctrl+C -> Ctrl+V)
          const plain = cd.getData ? cd.getData('text/plain') : '';
          if (!plain || (cd.files && cd.files.length > 0)) {
            tauriInvoke('app_read_clipboard_files')
              .then((list) => {
                if (Array.isArray(list) && list.length > 0) {
                  addExtAttachments(list);
                }
              })
              .catch(() => {});
          }
        };

        const handleExtDragOver = (ev) => {
          ev.preventDefault();
        };

        const handleExtDrop = (ev) => {
          ev.preventDefault();
          const dt = ev.dataTransfer;
          if (!dt || !dt.files || dt.files.length === 0) return;
          const files = Array.from(dt.files);
          const localPaths = [];
          for (const f of files) {
            if (typeof f.path === 'string' && f.path.trim()) {
              localPaths.push(f.path.trim());
            } else if (/^image\//i.test(f.type || '')) {
              const reader = new FileReader();
              reader.onload = () => {
                const dataUrl = typeof reader.result === 'string' ? reader.result : '';
                if (!dataUrl) return;
                tauriInvoke('app_save_clipboard_image', {
                  base64Data: dataUrl,
                  mime: f.type || 'image/png',
                })
                  .then((att) => {
                    if (att) addExtAttachments([att]);
                  })
                  .catch(() => {});
              };
              reader.readAsDataURL(f);
            }
          }
          if (localPaths.length > 0) {
            tauriInvoke('app_describe_paths', { paths: localPaths })
              .then((list) => {
                if (Array.isArray(list) && list.length > 0) addExtAttachments(list);
              })
              .catch(() => {});
          }
        };

        const openComposerPopover = (kind, anchorEl) => {
          setComposerSubView(null);
          setComposerPopover((prev) => {
            if (prev && prev.kind === kind) return null;
            let left = 12;
            let bottom = 92;
            let fullWidth = false;
            if (kind === 'plus' && dsbRuntime.activeHarness === 'codex') {
              const compEl = document.querySelector('[data-dsb-external-composer="codex"]');
              const cr = compEl ? compEl.getBoundingClientRect() : null;
              left = 0;
              bottom = cr && cr.height > 10 ? Math.max(88, Math.round(window.innerHeight - cr.top + 6)) : 126;
              fullWidth = true;
            } else if (anchorEl && typeof anchorEl.getBoundingClientRect === 'function') {
              const r = anchorEl.getBoundingClientRect();
              left = Math.max(8, Math.round(r.left));
              bottom = Math.max(48, Math.round(window.innerHeight - r.top + 10));
            }
            scheduleLayoutReport(260);
            return { kind, left, bottom, fullWidth };
          });
        };

        const renderExtAttachmentsRow = () => {
          if (extAttachments.length === 0) return null;
          return h(
            'div',
            {
              style: {
                display: 'flex',
                flexWrap: 'wrap',
                alignItems: 'center',
                gap: '6px',
                marginBottom: '8px',
              },
            },
            extAttachments.map((att) =>
              att.kind === 'image' && att.previewDataUrl
                ? h(
                    'div',
                    {
                      key: att.id,
                      title: att.path,
                      style: {
                        position: 'relative',
                        width: '44px',
                        height: '44px',
                        borderRadius: '10px',
                        overflow: 'hidden',
                        border: '1px solid rgba(255,255,255,0.16)',
                        background: 'rgba(0,0,0,0.35)',
                        flexShrink: 0,
                      },
                    },
                    h('img', {
                      src: att.previewDataUrl,
                      alt: att.name,
                      style: { width: '100%', height: '100%', objectFit: 'cover', display: 'block' },
                    }),
                    h(
                      'button',
                      {
                        type: 'button',
                        title: '移除图片',
                        onClick: (ev) => {
                          ev.stopPropagation();
                          removeExtAttachment(att.id);
                        },
                        style: {
                          position: 'absolute',
                          top: '2px',
                          right: '2px',
                          width: '16px',
                          height: '16px',
                          borderRadius: '999px',
                          border: 'none',
                          background: 'rgba(0,0,0,0.72)',
                          color: '#fff',
                          fontSize: '11px',
                          lineHeight: 1,
                          cursor: 'pointer',
                          padding: 0,
                        },
                      },
                      '×',
                    ),
                  )
                : h(
                    'span',
                    {
                      key: att.id,
                      title: att.path,
                      style: {
                        display: 'inline-flex',
                        alignItems: 'center',
                        gap: '5px',
                        padding: '3px 8px',
                        borderRadius: '8px',
                        fontSize: '12px',
                        background: 'rgba(255,255,255,0.08)',
                        border: '1px solid rgba(255,255,255,0.14)',
                        color: '#e5e7eb',
                        maxWidth: '220px',
                      },
                    },
                    h('span', { style: { flexShrink: 0 } }, att.kind === 'folder' ? '📁' : '📎'),
                    h(
                      'span',
                      { style: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' } },
                      att.name,
                    ),
                    h(
                      'button',
                      {
                        type: 'button',
                        onClick: (ev) => {
                          ev.stopPropagation();
                          removeExtAttachment(att.id);
                        },
                        style: {
                          border: 'none',
                          background: 'transparent',
                          color: 'rgba(229,231,235,0.68)',
                          cursor: 'pointer',
                          padding: '0 2px',
                          fontSize: '12px',
                          lineHeight: 1,
                        },
                      },
                      '×',
                    ),
                  ),
            ),
          );
        };

        // Query local files/folders when the @ mention picker is open
        useEffect(() => {
          if (!IS_BAR || !composerPopover || composerPopover.kind !== 'mention') return undefined;
          const q = String(mentionQuery || '').trim();
          const seq = ++mentionSeqRef.current;
          const effectiveQ = q || (extSel && extSel.workspacePath ? workspaceBasename(extSel.workspacePath) : 'a');
          const timer = setTimeout(() => {
            tauriInvoke('quick_search_query', { query: effectiveQ, limit: 8 })
              .then((hits) => {
                if (mentionSeqRef.current !== seq) return;
                setMentionHits(Array.isArray(hits) ? hits : []);
              })
              .catch(() => {
                if (mentionSeqRef.current !== seq) return;
                setMentionHits([]);
              });
          }, 30);
          return () => clearTimeout(timer);
        }, [composerPopover, mentionQuery]);

        const buildHarnessActionCtx = (hId) => ({
          selection: dsbRuntime.externalSelection[hId] || {},
          options: dsbRuntime.harnessOptions[hId] || null,
          prefs: dsbRuntime.harnessPrefs[hId] || {},
          updatePrefs: (patch) => updateHarnessPrefs(hId, patch),
          insertText: (snippet) => insertTextIntoExtComposer(snippet),
          pickFiles: (imagesOnly) => pickFilesForExtComposer(imagesOnly),
          pickFolder: () => pickFolderForExtComposer(),
          focusComposer: () => focusComposerElement(),
          openMentionPicker: () => {
            setMentionQuery('');
            const compEl = document.querySelector('[data-dsb-external-composer]');
            openComposerPopover('mention', compEl);
          },
          startNewSession: () => {
            const sel = dsbRuntime.externalSelection[hId] || {};
            startNewExternalConversation(hId, {
              workspaceId: sel.workspaceId || '',
              path: sel.workspacePath || '',
              label: sel.workspacePath ? workspaceBasename(sel.workspacePath) : getHarnessMeta(hId).label,
            });
            focusComposerElement();
          },
          showSessionStats: () => {
            setHistoryOpen(true);
            showExternalSessionStats(hId).then(() => scheduleLayoutReport(260));
          },
          runImmediatePrompt: (cmdText) => {
            setHistoryOpen(true);
            submitExternalHarnessPrompt(cmdText, []);
          },
        });

        const submitExternalComposer = () => {
          if (isExtStreaming) {
            cancelActiveExternalTurn();
            return;
          }
          const trimmedDraft = extDraft.trim();
          // Intercept standalone slash commands (e.g. /clear, /cost, /plan) and route through the active Harness module
          if (
            trimmedDraft.startsWith('/') &&
            !trimmedDraft.includes(' ') &&
            !trimmedDraft.includes('\n') &&
            extCapsules.length === 0 &&
            extAttachments.length === 0
          ) {
            const adapter = getHarnessAdapter(activeHarness);
            const uo = getUnifiedOptions(activeHarness, activeHarnessOpts);
            const known = (uo.actions || []).some((a) => a && a.id === trimmedDraft);
            if (known && adapter && typeof adapter.handleSlashAction === 'function') {
              dsbRuntime.extDraftText = '';
              setComposerPopover(null);
              notifyBarRuntime();
              adapter.handleSlashAction(trimmedDraft, buildHarnessActionCtx(activeHarness));
              return;
            }
          }
          const parts = [];
          for (const c of extCapsules) {
            if (c && c.text) parts.push(c.text);
          }
          if (trimmedDraft) parts.push(trimmedDraft);
          const fullText = parts.join('\n').trim();
          const attsToSend = extAttachments.slice();
          if (!fullText && attsToSend.length === 0) return;
          dsbRuntime.extDraftText = '';
          dsbRuntime.extCapsules = [];
          dsbRuntime.extAttachments = [];
          setComposerPopover(null);
          sendBarReport({ kind: 'capsules', capsuleCount: 0, afterInsert: false });
          notifyBarRuntime();
          submitExternalHarnessPrompt(fullText, attsToSend);
        };

        const removeExtCapsule = (capId) => {
          dsbRuntime.extCapsules = (dsbRuntime.extCapsules || []).filter((c) => c.id !== capId);
          sendBarReport({
            kind: 'capsules',
            capsuleCount: dsbRuntime.extCapsules.length,
            afterInsert: false,
          });
          notifyBarRuntime();
          if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
            dsbRuntime.scheduleLayoutReport(220);
          }
        };

        const renderExtCapsulesRow = () => {
          if (extCapsules.length === 0) return null;
          return h(
            'div',
            {
              style: {
                display: 'flex',
                flexWrap: 'wrap',
                gap: '6px',
                marginBottom: '8px',
              },
            },
            extCapsules.map((cap) =>
              h(
                'span',
                {
                  key: cap.id,
                  title: (cap.summary && cap.summary.body) || cap.text.slice(0, 120),
                  style: {
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: '5px',
                    padding: '2px 8px',
                    borderRadius: '999px',
                    fontSize: '12px',
                    fontWeight: 500,
                    background: 'rgba(56, 189, 248, 0.14)',
                    border: '1px solid rgba(56, 189, 248, 0.36)',
                    color: '#bae6fd',
                    maxWidth: '240px',
                  },
                },
                h(
                  'span',
                  {
                    style: {
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    },
                  },
                  (cap.summary && cap.summary.title) || '引用胶囊',
                ),
                h(
                  'button',
                  {
                    type: 'button',
                    onClick: (ev) => {
                      ev.stopPropagation();
                      removeExtCapsule(cap.id);
                    },
                    style: {
                      border: 'none',
                      background: 'transparent',
                      color: 'rgba(186, 230, 253, 0.72)',
                      cursor: 'pointer',
                      padding: '0 2px',
                      fontSize: '12px',
                      lineHeight: 1,
                    },
                  },
                  '×',
                ),
              ),
            ),
          );
        };

        const getUnifiedOptions = (hId, opts) => {
          const adapter = getHarnessAdapter(hId);
          const def =
            (adapter && typeof adapter.getDefaults === 'function' ? adapter.getDefaults() : null) || {
              models: [],
              efforts: [],
              modes: [],
              actions: [],
              defaultModel: '',
              defaultEffort: '',
              defaultMode: '',
            };
          const o = opts || {};
          const models = Array.isArray(o.models) && o.models.length > 0 ? o.models : def.models || [];
          const efforts = Array.isArray(o.efforts) && o.efforts.length > 0 ? o.efforts : def.efforts || [];
          const modes = Array.isArray(o.modes) && o.modes.length > 0 ? o.modes : def.modes || [];
          const actions = Array.isArray(o.actions) && o.actions.length > 0 ? o.actions : def.actions || [];
          return {
            models,
            efforts,
            modes,
            actions,
            defaultModel: o.defaultModel || def.defaultModel || '',
            defaultEffort: o.defaultEffort || def.defaultEffort || '',
            defaultMode: o.defaultMode || def.defaultMode || '',
          };
        };

        const handleExtInputKeyDown = (ev) => {
          if (ev.key === 'Tab' && ev.shiftKey && !ev.isComposing) {
            ev.preventDefault();
            ev.stopPropagation();
            const uo = getUnifiedOptions(activeHarness, activeHarnessOpts);
            const curModeId = (activeHarnessPrefs && activeHarnessPrefs.mode) || uo.defaultMode;
            const idx = Math.max(0, uo.modes.findIndex((m) => m.id === curModeId));
            const next = uo.modes[(idx + 1) % uo.modes.length];
            if (next) updateHarnessPrefs(activeHarness, { mode: next.id });
            return;
          }
          if (ev.key === 'Enter' && !ev.shiftKey && !ev.isComposing) {
            if (dsbRuntime.quickSearchOpen && dsbRuntime.quickSearchSelectedIdx >= 0) {
              return;
            }
            ev.preventDefault();
            ev.stopPropagation();
            submitExternalComposer();
          }
        };

        const handleExtInputChange = (ev) => {
          const val = ev.target.value;
          dsbRuntime.extDraftText = val;
          notifyBarRuntime();
          try {
            ev.target.style.height = 'auto';
            ev.target.style.height = Math.min(140, Math.max(24, ev.target.scrollHeight)) + 'px';
          } catch {}
          // Auto-trigger real inline `/` slash command menu or `@` file mention picker per Harness
          const compEl = document.querySelector('[data-dsb-external-composer]');
          const slashMatch = val.match(/^\/([a-zA-Z0-9_-]*)$/);
          const mentionMatch = val.match(/(?:^|\s)@([^\s@]{0,40})$/);
          if (slashMatch) {
            if (!composerPopover || composerPopover.kind !== 'slash') {
              const cr = compEl ? compEl.getBoundingClientRect() : null;
              setComposerPopover({
                kind: 'slash',
                left: 12,
                bottom: cr && cr.height > 10 ? Math.max(88, Math.round(window.innerHeight - cr.top + 8)) : 112,
                fullWidth: false,
              });
            }
          } else if (mentionMatch) {
            setMentionQuery(mentionMatch[1] || '');
            if (!composerPopover || composerPopover.kind !== 'mention') {
              const cr = compEl ? compEl.getBoundingClientRect() : null;
              setComposerPopover({
                kind: 'mention',
                left: 12,
                bottom: cr && cr.height > 10 ? Math.max(88, Math.round(window.innerHeight - cr.top + 8)) : 112,
                fullWidth: false,
              });
            }
          } else if (composerPopover && (composerPopover.kind === 'slash' || composerPopover.kind === 'mention')) {
            setComposerPopover(null);
          }
          if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
            dsbRuntime.scheduleLayoutReport(200);
          }
        };

        // v0.7.0 Unified DSH-Native Dual-Theme Composer (One Skeleton, Dedicated Per-Harness Modules)
        const renderUnifiedDshComposer = (ctx) => {
          const m = ctx.manifest;
          const t = getUnifiedTheme(m.id);
          const uo = getUnifiedOptions(m.id, ctx.options);
          const adapter = getHarnessAdapter(m.id);
          const prefs = ctx.prefs || {};
          const curModelId = prefs.model || uo.defaultModel;
          const curEffortId = prefs.effort || uo.defaultEffort;
          const curModeId = prefs.mode || uo.defaultMode;
          const curModel = uo.models.find((x) => x.id === curModelId) || uo.models[0];
          const curEffort = uo.efforts.find((x) => x.id === curEffortId) || uo.efforts[Math.min(2, uo.efforts.length - 1)] || uo.efforts[0];
          const curMode = uo.modes.find((x) => x.id === curModeId) || uo.modes[0];
          const isDangerMode = Boolean(
            curMode &&
              (curMode.danger ||
                curMode.id === 'turbo' ||
                curMode.id === 'danger-full-access' ||
                curMode.id === 'bypassPermissions' ||
                curMode.id === 'auto'),
          );
          const modeLabel =
            curMode && curMode.id === 'danger-full-access' ? '完全访问' : (curMode && curMode.label) || '默认';
          const activeFeatureChips =
            adapter && typeof adapter.getActiveChips === 'function'
              ? adapter.getActiveChips(buildHarnessActionCtx(m.id))
              : [];
          const wsLabel =
            (ctx.selection && ctx.selection.workspacePath
              ? ctx.workspaceBasename(ctx.selection.workspacePath)
              : '') ||
            (ctx.selection && ctx.selection.title) ||
            m.label;
          const cardShadow = t.isDark
            ? '0 14px 36px rgba(0, 0, 0, 0.46)'
            : '0 8px 24px rgba(15, 23, 42, 0.09)';

          return h(
            'div',
            {
              'data-dsb-external-composer': m.id,
              'data-dsb-unified-composer': '1',
              onDragOver: ctx.onDragOver,
              onDrop: ctx.onDrop,
              onContextMenu: (ev) => {
                if (ctx.onOpenSwitcher) {
                  ev.preventDefault();
                  ctx.onOpenSwitcher(ev);
                }
              },
              style: {
                width: '100vw',
                position: 'relative',
                overflow: 'hidden',
                background: t.surface,
                borderRadius: '18px',
                border: '1px solid ' + t.border,
                boxShadow: cardShadow,
                padding: '13px 15px 10px 15px',
                display: 'flex',
                flexDirection: 'column',
                boxSizing: 'border-box',
                pointerEvents: 'auto',
                color: t.fg,
                transition: 'background 180ms ease, border-color 180ms ease, color 180ms ease',
                fontFamily: '"Segoe UI", "Inter", "PingFang SC", "Microsoft YaHei", system-ui, sans-serif',
              },
            },
            // Official Brand Logo 2px Top Hairline
            h('div', {
              style: {
                position: 'absolute',
                left: 0,
                right: 0,
                top: 0,
                height: '2px',
                background: t.topBar,
                opacity: 0.95,
                pointerEvents: 'none',
              },
            }),
            ctx.renderDragHandle && ctx.renderDragHandle(),
            ctx.renderAttachmentsRow && ctx.renderAttachmentsRow(),
            ctx.renderCapsulesRow(),
            activeFeatureChips.length > 0
              ? h(
                  'div',
                  {
                    'data-dsb-feature-chips': '1',
                    style: {
                      display: 'flex',
                      flexWrap: 'wrap',
                      alignItems: 'center',
                      gap: '5px',
                      marginBottom: '6px',
                    },
                  },
                  activeFeatureChips.map((chip) =>
                    h(
                      'span',
                      {
                        key: chip.id,
                        title: chip.title || chip.label,
                        style: {
                          display: 'inline-flex',
                          alignItems: 'center',
                          gap: '4px',
                          padding: '1px 7px',
                          borderRadius: '999px',
                          fontSize: '11px',
                          fontWeight: 550,
                          background: t.accentBg,
                          border: '1px solid ' + t.accentBorder,
                          color: t.accent,
                        },
                      },
                      h('span', null, chip.label),
                      h(
                        'button',
                        {
                          type: 'button',
                          onClick: (ev) => {
                            ev.stopPropagation();
                            if (typeof chip.onRemove === 'function') chip.onRemove();
                          },
                          style: {
                            border: 'none',
                            background: 'transparent',
                            color: 'inherit',
                            opacity: 0.72,
                            cursor: 'pointer',
                            padding: '0 1px',
                            fontSize: '11.5px',
                            lineHeight: 1,
                          },
                        },
                        '×',
                      ),
                    ),
                  ),
                )
              : null,
            h('textarea', {
              'data-dsb-ext-input': '1',
              rows: 1,
              value: ctx.draft,
              onChange: ctx.onInputChange,
              onKeyDown: ctx.onInputKeyDown,
              onPaste: ctx.onPaste,
              placeholder: m.placeholder || '发消息或创建任务, / 调用指令, @ 文件或对话',
              style: {
                width: '100%',
                minHeight: '24px',
                maxHeight: '140px',
                border: 'none',
                outline: 'none',
                resize: 'none',
                background: 'transparent',
                color: t.fg,
                fontSize: '13.5px',
                lineHeight: '21px',
                fontFamily: 'inherit',
                padding: 0,
                margin: 0,
                boxSizing: 'border-box',
              },
            }),
            h(
              'div',
              {
                style: {
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  marginTop: '11px',
                  height: '30px',
                  gap: '8px',
                },
              },
              // Unified Left Toolbar: (+) + Permission + Brand Harness·Workspace Pill
              h(
                'div',
                { style: { display: 'flex', alignItems: 'center', gap: '7px', minWidth: 0 } },
                h(
                  'button',
                  {
                    type: 'button',
                    'data-dsb-plus-btn': '1',
                    title: '添加文件 / 文件夹 / 指令 / 插件',
                    onClick: (ev) => {
                      ev.stopPropagation();
                      ctx.openComposerPopover && ctx.openComposerPopover('plus', ev.currentTarget);
                    },
                    style: {
                      width: '28px',
                      height: '28px',
                      borderRadius: '999px',
                      border: 'none',
                      background: ctx.activePopover === 'plus' ? t.accentBg : t.plusBg,
                      padding: 0,
                      cursor: 'pointer',
                      display: 'inline-flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      flexShrink: 0,
                    },
                  },
                  HARNESS_SDK_ICONS.plus(15, ctx.activePopover === 'plus' ? t.accent : t.fg),
                ),
                h(
                  'button',
                  {
                    type: 'button',
                    'data-dsb-mode-btn': '1',
                    title: '切换权限模式（支持 Shift+Tab 循环）',
                    onClick: (ev) => {
                      ev.stopPropagation();
                      ctx.openComposerPopover && ctx.openComposerPopover('mode', ev.currentTarget);
                    },
                    style: {
                      height: '26px',
                      border: 'none',
                      borderRadius: '8px',
                      padding: '0 7px',
                      background: ctx.activePopover === 'mode' ? t.accentBg : 'transparent',
                      color: isDangerMode ? t.permDanger : t.fg,
                      fontSize: '12.5px',
                      fontWeight: 500,
                      cursor: 'pointer',
                      display: 'inline-flex',
                      alignItems: 'center',
                      gap: '4px',
                      whiteSpace: 'nowrap',
                      flexShrink: 0,
                    },
                  },
                  HARNESS_SDK_ICONS.shield(isDangerMode ? t.permDanger : t.muted),
                  h('span', null, modeLabel),
                  HARNESS_SDK_ICONS.chevronDown(t.muted),
                ),
                h(
                  'button',
                  {
                    type: 'button',
                    'data-dsb-session-chip': '1',
                    title: '点击展开 3 级 Harness / 工作区 / 对话切换器',
                    onClick: ctx.onOpenSwitcher,
                    style: {
                      height: '24px',
                      border: 'none',
                      borderRadius: '7px',
                      padding: '0 8px',
                      background: t.accentBg,
                      color: t.accent,
                      fontSize: '11.5px',
                      fontWeight: 500,
                      cursor: 'pointer',
                      display: 'inline-flex',
                      alignItems: 'center',
                      gap: '5px',
                      maxWidth: '210px',
                      minWidth: 0,
                    },
                  },
                  h('span', {
                    style: {
                      width: '7px',
                      height: '7px',
                      borderRadius: '999px',
                      background: t.dotStyle,
                      flexShrink: 0,
                    },
                  }),
                  h(
                    'span',
                    { style: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' } },
                    m.label + ' · ' + wsLabel,
                  ),
                ),
              ),
              // Unified Right Toolbar: Model·Effort + Brand Send/Stop Circle
              h(
                'div',
                { style: { display: 'flex', alignItems: 'center', gap: '8px', flexShrink: 0 } },
                h(
                  'button',
                  {
                    type: 'button',
                    'data-dsb-model-btn': '1',
                    title: '切换模型与推理强度',
                    onClick: (ev) => {
                      ev.stopPropagation();
                      ctx.openComposerPopover && ctx.openComposerPopover('model', ev.currentTarget);
                    },
                    style: {
                      height: '26px',
                      border: 'none',
                      borderRadius: '8px',
                      padding: '0 7px',
                      background: ctx.activePopover === 'model' ? t.accentBg : 'transparent',
                      color: t.fg,
                      fontSize: '12.5px',
                      fontWeight: 500,
                      cursor: 'pointer',
                      display: 'inline-flex',
                      alignItems: 'center',
                      gap: '5px',
                      whiteSpace: 'nowrap',
                    },
                  },
                  h('span', null, (curModel && curModel.label) || ''),
                  curEffort && curEffort.label
                    ? h('span', { style: { color: t.muted, fontWeight: 400 } }, curEffort.label)
                    : null,
                  HARNESS_SDK_ICONS.chevronDown(t.muted),
                ),
                h(
                  'button',
                  {
                    type: 'button',
                    title: ctx.isStreaming ? '停止生成' : '发送到 ' + m.fullLabel,
                    onClick: ctx.onSubmit,
                    style: {
                      width: '30px',
                      height: '30px',
                      borderRadius: '999px',
                      border: 'none',
                      background: ctx.isStreaming ? (t.isDark ? '#32343a' : '#e2e8f0') : t.sendBg,
                      boxShadow: ctx.isStreaming ? 'none' : t.sendRing,
                      opacity: ctx.isStreaming || ctx.hasPayload ? 1 : 0.88,
                      cursor: 'pointer',
                      display: 'inline-flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      padding: 0,
                      flexShrink: 0,
                    },
                  },
                  ctx.isStreaming
                    ? h('span', {
                        style: { width: '10px', height: '10px', borderRadius: '2px', background: '#ef4444' },
                      })
                    : HARNESS_SDK_ICONS.arrowUp(t.sendFg),
                ),
              ),
            ),
          );
        };

        // v0.6.0 Unified DSH-Native Dual-Theme Popover (Dark + Light, Official-Logo Palettes)
        const renderComposerPopover = () => {
          if (!IS_BAR || !composerPopover || activeHarness === 'dsh') return null;
          const opts = activeHarnessOpts || {};
          const prefs = activeHarnessPrefs || {};
          const meta = getHarnessMeta(activeHarness);
          const t = getUnifiedTheme(activeHarness);
          const uo = getUnifiedOptions(activeHarness, opts);
          const panelShadow = t.isDark
            ? '0 16px 42px rgba(0, 0, 0, 0.56)'
            : '0 12px 32px rgba(15, 23, 42, 0.14)';

          const svgIcon = (sz, strokeColor, children) =>
            h(
              'svg',
              {
                width: sz,
                height: sz,
                viewBox: '0 0 24 24',
                fill: 'none',
                stroke: strokeColor || t.muted,
                strokeWidth: 1.75,
                strokeLinecap: 'round',
                strokeLinejoin: 'round',
                style: { display: 'block', flexShrink: 0 },
              },
              children,
            );

          const ICONS = {
            clip: (c) => svgIcon(15, c, [h('path', { key: 'p', d: 'M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48' })]),
            folder: (c) => svgIcon(15, c, [h('path', { key: 'p', d: 'M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z' })]),
            image: (c) => svgIcon(15, c, [h('rect', { key: 'r', x: 3, y: 4, width: 18, height: 16, rx: 2 }), h('circle', { key: 'c', cx: 8.5, cy: 9.5, r: 1.5 }), h('path', { key: 'p', d: 'M21 15l-5-5L5 20' })]),
            at: (c) => svgIcon(15, c, [h('circle', { key: 'c', cx: 12, cy: 12, r: 4 }), h('path', { key: 'p', d: 'M16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-3.92 7.94' })]),
            actions: (c) => svgIcon(15, c, [h('rect', { key: 'r', x: 3.5, y: 3.5, width: 17, height: 17, rx: 3.5 }), h('path', { key: 'p', d: 'M9 15l6-6' })]),
            chrome: (c) => svgIcon(15, c, [h('circle', { key: 'o', cx: 12, cy: 12, r: 9 }), h('circle', { key: 'i', cx: 12, cy: 12, r: 3.5 }), h('path', { key: 'l', d: 'M12 8.5h8.4M8.97 13.75L4.77 6.48M15.03 13.75l-4.2 7.27' })]),
            slash: (c) => svgIcon(14, c, [h('path', { key: 's', d: 'M15 5L9 19', strokeWidth: 2 })]),
            check: (c) => svgIcon(14, c, [h('path', { key: 'p', d: 'M5 12.5l4.5 4.5L19 7.5', strokeWidth: 2.2 })]),
            pluginBadge: (id) => {
              const lower = String(id || '').toLowerCase();
              const bg = lower.includes('doc') ? '#2563eb' : lower.includes('pdf') ? '#dc2626' : lower.includes('sheet') ? '#16a34a' : lower.includes('present') ? '#ea580c' : t.accent;
              const txt = lower.includes('doc') ? 'D' : lower.includes('pdf') ? 'PDF' : lower.includes('sheet') ? 'X' : lower.includes('present') ? 'P' : '•';
              return h('span', { style: { width: '17px', height: '17px', borderRadius: '4px', background: bg, color: '#fff', fontSize: txt.length > 1 ? '7px' : '10px', fontWeight: 700, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 } }, txt);
            },
          };

          const sectionHdr = (key, title, withTopBorder) =>
            h(
              'div',
              {
                key,
                style: {
                  fontSize: '11px',
                  fontWeight: 500,
                  color: t.muted,
                  padding: withTopBorder ? '8px 10px 4px' : '3px 10px 5px',
                  borderTop: withTopBorder ? '1px solid ' + t.subtleBorder : 'none',
                  marginTop: withTopBorder ? '4px' : 0,
                },
              },
              title,
            );

          const popRow = (key, iconNode, title, sub, active, onClick, rightNode, titleColor) =>
            h(
              'button',
              {
                key,
                type: 'button',
                'data-dsb-pop-row': '1',
                onClick: (ev) => {
                  ev.stopPropagation();
                  onClick();
                },
                style: {
                  width: '100%',
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: '10px',
                  padding: sub ? '7px 10px' : '6px 10px',
                  borderRadius: '9px',
                  border: 'none',
                  background: active ? t.accentBg : 'transparent',
                  color: titleColor || (active ? t.accent : t.fg),
                  cursor: 'pointer',
                  textAlign: 'left',
                  boxSizing: 'border-box',
                },
              },
              h(
                'div',
                { style: { display: 'flex', alignItems: 'center', gap: '9px', minWidth: 0, flex: 1 } },
                iconNode || null,
                h(
                  'div',
                  { style: { minWidth: 0, flex: 1 } },
                  h(
                    'div',
                    {
                      style: {
                        fontSize: '12.5px',
                        fontWeight: active ? 600 : 500,
                        lineHeight: '18px',
                        whiteSpace: 'nowrap',
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                      },
                    },
                    title,
                  ),
                  sub
                    ? h(
                        'div',
                        {
                          style: {
                            fontSize: '11px',
                            color: t.muted,
                            lineHeight: '15px',
                            marginTop: '1px',
                            whiteSpace: 'nowrap',
                            overflow: 'hidden',
                            textOverflow: 'ellipsis',
                          },
                        },
                        sub,
                      )
                    : null,
                ),
              ),
              rightNode || (active ? ICONS.check(t.accent) : null),
            );

          let panelWidth = 312;
          let content = [];
          const adapter = getHarnessAdapter(activeHarness);
          const harnessPopCtx = Object.assign(buildHarnessActionCtx(activeHarness), {
            t,
            icons: ICONS,
            opts,
            prefs,
            subView: composerSubView,
            setSubView: (sv) => {
              setComposerSubView(sv);
              scheduleLayoutReport(220);
            },
            closePopover: () => {
              setComposerSubView(null);
              setComposerPopover(null);
            },
            sectionHdr,
            popRow,
          });

          if (composerPopover.kind === 'plus') {
            panelWidth = 316;
            if (adapter && typeof adapter.buildPlusMenu === 'function') {
              content = adapter.buildPlusMenu(harnessPopCtx) || [];
            }
          } else if (composerPopover.kind === 'mention') {
            panelWidth = 336;
            content = [
              sectionHdr('at-hdr', meta.label + ' · 引用文件或文件夹 (@)', false),
              h('input', {
                key: 'at-input',
                type: 'text',
                autoFocus: true,
                placeholder: '输入文件名或目录关键词筛选…',
                value: mentionQuery,
                onChange: (ev) => setMentionQuery(ev.target.value),
                onKeyDown: (ev) => {
                  if (ev.key === 'Escape') {
                    ev.stopPropagation();
                    setComposerPopover(null);
                    focusComposerElement();
                  } else if (ev.key === 'Enter' && mentionHits.length > 0) {
                    ev.preventDefault();
                    ev.stopPropagation();
                    const first = mentionHits[0];
                    if (first && first.path) {
                      tauriInvoke('app_describe_paths', { paths: [first.path] })
                        .then((list) => {
                          if (Array.isArray(list) && list.length > 0) addExtAttachments(list);
                        })
                        .catch(() => {});
                      dsbRuntime.extDraftText = (dsbRuntime.extDraftText || '').replace(/(?:^|\s)@[^\s@]{0,40}$/, '').trim();
                      setComposerPopover(null);
                      notifyBarRuntime();
                      focusComposerElement();
                    }
                  }
                },
                style: {
                  width: '100%',
                  boxSizing: 'border-box',
                  padding: '6px 9px',
                  marginBottom: '6px',
                  borderRadius: '8px',
                  fontSize: '12px',
                  background: t.isDark ? 'rgba(255,255,255,0.06)' : 'rgba(15,23,42,0.05)',
                  border: '1px solid ' + t.accentBorder,
                  color: t.fg,
                  outline: 'none',
                },
              }),
              ...(mentionHits.length === 0
                ? [
                    h(
                      'div',
                      { key: 'at-empty', style: { padding: '10px', fontSize: '12px', color: t.muted, textAlign: 'center' } },
                      '未找到匹配文件，可继续输入关键词或点「+」手动选择',
                    ),
                  ]
                : mentionHits.map((hit) =>
                    popRow(
                      'at-' + hit.path,
                      hit.kind === 'dir' ? ICONS.folder(t.accent) : ICONS.clip(t.muted),
                      hit.name || hit.path,
                      hit.parentDisplay || hit.path,
                      false,
                      () => {
                        tauriInvoke('app_describe_paths', { paths: [hit.path] })
                          .then((list) => {
                            if (Array.isArray(list) && list.length > 0) addExtAttachments(list);
                          })
                          .catch(() => {});
                        dsbRuntime.extDraftText = (dsbRuntime.extDraftText || '').replace(/(?:^|\s)@[^\s@]{0,40}$/, '').trim();
                        setComposerPopover(null);
                        notifyBarRuntime();
                        focusComposerElement();
                      },
                    ),
                  )),
            ];
          } else if (composerPopover.kind === 'slash') {
            panelWidth = 312;
            const q = String(extDraft || '').trim().toLowerCase();
            const actions = (uo.actions || []).filter(
              (a) => a && (!q || String(a.id || '').toLowerCase().includes(q) || String(a.desc || '').toLowerCase().includes(q)),
            );
            content = [
              sectionHdr('sl-hdr', meta.label + ' · 斜杠指令（回车或点击执行）', false),
              ...(actions.length === 0
                ? [h('div', { key: 'sl-none', style: { padding: '10px', fontSize: '12px', color: t.muted, textAlign: 'center' } }, '无匹配指令')]
                : actions.map((a) =>
                    popRow('sl-' + a.id, ICONS.slash(t.accent), a.label || a.id, a.desc || '', false, () => {
                      dsbRuntime.extDraftText = '';
                      setComposerPopover(null);
                      notifyBarRuntime();
                      if (adapter && typeof adapter.handleSlashAction === 'function') {
                        adapter.handleSlashAction(a.id, buildHarnessActionCtx(activeHarness));
                      } else {
                        insertTextIntoExtComposer(a.id + ' ');
                      }
                    }),
                  )),
            ];
          } else if (composerPopover.kind === 'mode') {
            panelWidth = 324;
            const curMode = prefs.mode || uo.defaultMode;
            content = [
              sectionHdr('m-hdr', meta.label + ' · 权限模式（Shift+Tab 循环）', false),
              ...uo.modes.map((md) => {
                const isSel = md.id === curMode;
                const isDanger = Boolean(
                  md.danger || md.id === 'turbo' || md.id === 'danger-full-access' || md.id === 'bypassPermissions' || md.id === 'auto',
                );
                return popRow(
                  'md-' + md.id,
                  HARNESS_SDK_ICONS.shield(isDanger ? t.permDanger : t.muted),
                  md.label,
                  md.desc || '',
                  isSel,
                  () => {
                    updateHarnessPrefs(activeHarness, { mode: md.id });
                    setComposerPopover(null);
                  },
                  null,
                  isDanger ? t.permDanger : undefined,
                );
              }),
            ];
          } else if (composerPopover.kind === 'model') {
            panelWidth = 312;
            const curModel = prefs.model || uo.defaultModel;
            const curEffort = prefs.effort || uo.defaultEffort;
            content = [
              h(
                'div',
                { key: 'mdl-hdr', style: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '3px 10px 5px' } },
                h('span', { style: { fontSize: '11px', fontWeight: 500, color: t.muted } }, meta.label + ' · 选择模型'),
                adapter && typeof adapter.renderModelHeaderExtra === 'function'
                  ? adapter.renderModelHeaderExtra(harnessPopCtx)
                  : null,
              ),
              ...uo.models.map((m) => {
                const isSel = m.id === curModel || (curModel === 'gemini-4-neon' && m.id === 'gemini-4-neon-high');
                const tagText = m.tag || (m.desc && m.desc.includes('|') ? m.desc.split('|')[1] : '');
                return popRow(
                  'mdl-' + m.id,
                  null,
                  m.label,
                  '',
                  isSel,
                  () => {
                    updateHarnessPrefs(activeHarness, { model: m.id });
                  },
                  h(
                    'span',
                    { style: { display: 'inline-flex', alignItems: 'center', gap: '6px', flexShrink: 0 } },
                    tagText
                      ? h(
                          'span',
                          {
                            style: {
                              fontSize: '10px',
                              padding: '1px 6px',
                              borderRadius: '999px',
                              background: t.isDark ? 'rgba(255,255,255,0.07)' : 'rgba(15,23,42,0.06)',
                              color: t.muted,
                            },
                          },
                          tagText,
                        )
                      : null,
                    isSel ? ICONS.check(t.accent) : null,
                  ),
                );
              }),
              uo.efforts.length > 0
                ? h(
                    'div',
                    {
                      key: 'eff-bar',
                      style: {
                        marginTop: '6px',
                        padding: '7px 6px 2px',
                        borderTop: '1px solid ' + t.subtleBorder,
                      },
                    },
                    h('div', { style: { fontSize: '10.5px', color: t.muted, marginBottom: '5px', paddingLeft: '2px' } }, '推理强度'),
                    h(
                      'div',
                      { style: { display: 'flex', gap: '4px' } },
                      uo.efforts.map((ef) => {
                        const isSel = ef.id === curEffort;
                        return h(
                          'button',
                          {
                            key: 'ef-' + ef.id,
                            type: 'button',
                            onClick: (ev) => {
                              ev.stopPropagation();
                              updateHarnessPrefs(activeHarness, { effort: ef.id });
                            },
                            style: {
                              flex: 1,
                              padding: '5px 3px',
                              borderRadius: '8px',
                              border: '1px solid ' + (isSel ? t.accentBorder : t.subtleBorder),
                              background: isSel ? t.accentBg : 'transparent',
                              color: isSel ? t.accent : t.muted,
                              fontSize: '11.5px',
                              fontWeight: isSel ? 600 : 400,
                              cursor: 'pointer',
                            },
                          },
                          ef.label,
                        );
                      }),
                    ),
                  )
                : null,
            ];
          }

          const clampedLeft = Math.max(8, Math.min(window.innerWidth - panelWidth - 8, composerPopover.left));
          const pop = h(
            'div',
            {
              'data-dsb-popup': '1',
              'data-dsb-composer-popover': '1',
              'data-dsb-popup-bottom': String(composerPopover.bottom),
              style: {
                position: 'fixed',
                left: clampedLeft + 'px',
                bottom: composerPopover.bottom + 'px',
                width: panelWidth + 'px',
                maxHeight: '360px',
                overflowY: 'auto',
                background: t.popover,
                color: t.fg,
                border: '1px solid ' + t.border,
                borderRadius: '14px',
                padding: '7px',
                boxShadow: panelShadow,
                zIndex: 9999,
                boxSizing: 'border-box',
                fontFamily: '"Segoe UI", "Inter", "PingFang SC", "Microsoft YaHei", system-ui, sans-serif',
              },
            },
            content,
          );
          if (ReactDOM && typeof ReactDOM.createPortal === 'function' && typeof document !== 'undefined' && document.body) {
            return ReactDOM.createPortal(pop, document.body);
          }
          return pop;
        };

        const renderExternalNativeComposer = () => {
          const manifest = getHarnessMeta(activeHarness);
          if (!IS_BAR || barMode === 'lip' || manifest.useNativeComposer) return null;

          const adapter = getHarnessAdapter(activeHarness);
          const composerCtx = {
            manifest,
            avail: activeAvail,
            options: activeHarnessOpts,
            prefs: activeHarnessPrefs,
            activePopover: composerPopover ? composerPopover.kind : null,
            openComposerPopover,
            selection: extSel,
            draft: extDraft,
            capsules: extCapsules,
            attachments: extAttachments,
            hasPayload: Boolean(extDraft.trim() || extCapsules.length > 0 || extAttachments.length > 0),
            isStreaming: isExtStreaming,
            onInputChange: handleExtInputChange,
            onInputKeyDown: handleExtInputKeyDown,
            onPaste: handleExtPaste,
            onDragOver: handleExtDragOver,
            onDrop: handleExtDrop,
            onSubmit: submitExternalComposer,
            onOpenSwitcher: (ev) => {
              ev.stopPropagation();
              setComposerPopover(null);
              if (typeof dsbRuntime.toggleHarnessMenu === 'function') {
                dsbRuntime.toggleHarnessMenu(ev.currentTarget);
              }
            },
            renderCapsulesRow: renderExtCapsulesRow,
            renderAttachmentsRow: renderExtAttachmentsRow,
            renderDragHandle,
            workspaceBasename,
            getPermissionMode: () => activeHarnessPrefs.mode || dsbRuntime.claudePermissionMode || 'auto',
            setPermissionMode: (modeId) => {
              updateHarnessPrefs('claude_code', { mode: modeId });
            },
          };

          return renderUnifiedDshComposer(composerCtx);
        };

        return h(
          'div',
          {
            'data-dsb-stack': '1',
            style: {
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'stretch',
              width: IS_BAR ? '100vw' : '100%',
              margin: 0,
              padding: 0,
              boxSizing: 'border-box',
            },
          },
          h(
            'div',
            {
              ref: wrapRef,
              'data-donegate-dock-wrap': '',
              onMouseEnter: handleMouseEnter,
              onMouseLeave: handleMouseLeave,
              style: {
                position: 'relative',
                width: IS_BAR
                  ? '100vw'
                  : matchedCardWidth
                    ? matchedCardWidth + 'px'
                    : '100%',
                maxWidth: IS_BAR
                  ? '100vw'
                  : matchedCardWidth
                    ? matchedCardWidth + 'px'
                    : '100%',
                marginInline: 'auto',
                flexShrink: 0,
                alignSelf: 'center',
                height: IS_BAR ? (barMode === 'lip' ? COLLAPSED_LIP_PX + 'px' : '0px') : COLLAPSED_LIP_PX + 'px',
                zIndex: 1,
                boxSizing: 'border-box',
                pointerEvents: IS_BAR && barMode !== 'lip' ? 'none' : 'auto',
                fontFamily: 'system-ui, -apple-system, "Segoe UI", Roboto, "PingFang SC", sans-serif',
              },
            },
          // Left & right 8px invisible width drag edges inside slab in Bar mode
          IS_BAR && barMode !== 'lip'
            ? [
                h('div', {
                  key: 'resize-w-left',
                  onMouseDown: handleStartResizeWidth('left'),
                  title: '左右拖动调节窗口宽度',
                  style: {
                    position: 'fixed',
                    left: 0,
                    top: 0,
                    bottom: 0,
                    width: '8px',
                    cursor: 'ew-resize',
                    pointerEvents: 'auto',
                    zIndex: 35,
                  },
                }),
                h('div', {
                  key: 'resize-w-right',
                  onMouseDown: handleStartResizeWidth('right'),
                  title: '左右拖动调节窗口宽度',
                  style: {
                    position: 'fixed',
                    right: 0,
                    top: 0,
                    bottom: 0,
                    width: '8px',
                    cursor: 'ew-resize',
                    pointerEvents: 'auto',
                    zIndex: 35,
                  },
                }),
              ]
            : null,

          // Scheme B: Detached Floating History Sheet / Hover Peek / Unread Toast (ONLY rendered when active!)
          !IS_BAR || barMode === 'lip' || isExpanded
            ? h(
                'div',
                {
                  'data-dsb-card': '',
                  'data-dsb-floating-anchor': '1',
                  style: {
                    position: 'absolute',
                    left: 0,
                    right: 0,
                    bottom: IS_BAR ? (barMode === 'lip' ? '0px' : DETACHED_GAP_PX + 'px') : -TUCK_OVERLAP_PX + 'px',
                    height: (isExpanded ? expandedHeight : COLLAPSED_LIP_PX) + (IS_BAR ? 0 : TUCK_OVERLAP_PX) + 'px',
                    paddingBottom: IS_BAR ? '0px' : TUCK_OVERLAP_PX + 'px',
                    borderRadius: cardBorderRadius,
                    borderBottom: 'none',
                    background: 'var(--dsw-alias-bg-layer-2, #2c2c2e)',
                    boxShadow: IS_BAR
                      ? '0 18px 46px rgba(0, 0, 0, 0.50), 0 0 0 0.5px var(--dsw-alias-border-l2, rgba(255, 255, 255, 0.10)) inset'
                      : isExpanded
                        ? '0 0 0 0.5px rgba(255, 255, 255, 0.14), 0 -14px 36px rgba(0, 0, 0, 0.42)'
                        : '0 0 0 0.5px rgba(255, 255, 255, 0.10)',
                    color: 'var(--dsw-alias-label-primary, #f3f4f6)',
                    display: 'flex',
                    flexDirection: 'column',
                    overflow: 'hidden',
                    boxSizing: 'border-box',
                    pointerEvents: 'auto',
                    transition: isResizingHeightRef.current
                      ? 'none'
                      : 'height 0.22s cubic-bezier(0.22, 1, 0.36, 1), box-shadow 0.2s ease',
                  },
                },
            isExpanded
              ? [
                  h(
                    'div',
                    {
                      key: 'grip-header',
                      onMouseDown: handleStartResizeHeight,
                      title: '上下拖拽调节消息抽屉高度',
                      style: {
                        position: 'relative',
                        height: '36px',
                        flexShrink: 0,
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'space-between',
                        padding: '0 8px 0 18px',
                        cursor: 'ns-resize',
                        borderBottom: '1px solid rgba(128,128,128,0.14)',
                        userSelect: 'none',
                      },
                    },
                    h(
                      'div',
                      {
                        'data-dsb-reply-dots': '',
                        onMouseDown: (ev) => ev.stopPropagation(),
                        style: {
                          display: 'flex',
                          alignItems: 'center',
                          gap: '6px',
                          maxWidth: '42%',
                          overflow: 'hidden',
                        },
                      },
                      assistantEntriesDesc.slice(0, 12).map((msg, idx) => {
                        const id = String(msg.id);
                        const isActive = activeAssistantId ? activeAssistantId === id : idx === 0;
                        const snippet = stripMarkdownForLip(msg.text || '').slice(0, 28);
                        const label =
                          idx === 0
                            ? '最新回复' + (snippet ? '：' + snippet : '')
                            : '前第 ' + idx + ' 条回复' + (snippet ? '：' + snippet : '');
                        return h(
                          'button',
                          {
                            key: id,
                            type: 'button',
                            title: label,
                            'aria-label': label,
                            onClick: (ev) => {
                              ev.stopPropagation();
                              jumpToAssistantReply(id);
                            },
                            style: {
                              width: '12px',
                              height: '12px',
                              padding: 0,
                              border: 'none',
                              background: 'transparent',
                              display: 'inline-flex',
                              alignItems: 'center',
                              justifyContent: 'center',
                              cursor: 'pointer',
                              flexShrink: 0,
                            },
                          },
                          h('span', {
                            style: {
                              width: isActive ? '7px' : '5.5px',
                              height: isActive ? '7px' : '5.5px',
                              borderRadius: '999px',
                              background: isActive
                                ? 'var(--dsw-alias-label-primary, #f3f4f6)'
                                : 'rgba(180,180,190,0.34)',
                              boxShadow: isActive ? '0 0 6px rgba(255,255,255,0.45)' : 'none',
                              transition: 'all 0.16s ease',
                              display: 'block',
                            },
                          }),
                        );
                      }),
                    ),
                    h('div', {
                      'data-dsb-grip-pill': '',
                      style: {
                        position: 'absolute',
                        left: '50%',
                        top: '50%',
                        transform: 'translate(-50%, -50%)',
                        width: '36px',
                        height: '4px',
                        borderRadius: '999px',
                        background: 'rgba(128,128,128,0.38)',
                        pointerEvents: 'none',
                      },
                    }),
                    h(
                      'div',
                      {
                        style: { display: 'flex', alignItems: 'center', gap: '6px' },
                        onMouseDown: (ev) => ev.stopPropagation(),
                      },
                      (() => {
                        const meta = getHarnessMeta(activeHarness);
                        const isDesktopRunning =
                          meta.useNativeComposer
                            ? true
                            : Boolean(
                                (activeAvail && activeAvail.running) ||
                                  (dsbRuntime.liveSyncMeta[activeHarness] &&
                                    dsbRuntime.liveSyncMeta[activeHarness].running),
                              );
                        const sel =
                          meta.useNativeComposer
                            ? { sessionId: sessionId || '', title: '' }
                            : dsbRuntime.externalSelection[activeHarness] || {};
                        return h(
                          'button',
                          {
                            type: 'button',
                            title: meta.detailButtonTitle,
                            onClick: (ev) => {
                              ev.stopPropagation();
                              tauriInvoke('harness_focus_desktop', {
                                harness: activeHarness,
                                sessionId: sel.sessionId || '',
                                title: sel.title || '',
                                workspacePath: sel.workspacePath || '',
                              }).catch(() => {});
                            },
                            style: {
                              fontSize: '11px',
                              fontWeight: 600,
                              padding: '2px 8px',
                              borderRadius: '999px',
                              background: meta.accentBg,
                              border: '1px solid ' + meta.accentBorder,
                              color: meta.accent,
                              letterSpacing: '0.01em',
                              display: 'inline-flex',
                              alignItems: 'center',
                              gap: '5px',
                              cursor: 'pointer',
                              transition: 'filter 0.14s ease',
                            },
                          },
                          h('span', {
                            style: {
                              width: '6px',
                              height: '6px',
                              borderRadius: '999px',
                              background: isDesktopRunning ? '#34d399' : '#71717a',
                              boxShadow: isDesktopRunning ? '0 0 6px #34d399' : 'none',
                              flexShrink: 0,
                            },
                          }),
                          '查看详情',
                        );
                      })(),
                      activeHarness !== 'dsh' &&
                        dsbRuntime.externalActiveTurn &&
                        dsbRuntime.externalActiveTurn.harness === activeHarness
                        ? h(
                            'button',
                            {
                              type: 'button',
                              onClick: (ev) => {
                                ev.stopPropagation();
                                cancelActiveExternalTurn();
                              },
                              title: '停止当前生成',
                              style: {
                                fontSize: '11px',
                                padding: '2px 7px',
                                borderRadius: '999px',
                                border: '1px solid rgba(248, 113, 113, 0.45)',
                                background: 'rgba(248, 113, 113, 0.14)',
                                color: '#fca5a5',
                                cursor: 'pointer',
                              },
                            },
                            '停止',
                          )
                        : null,
                      renderPinButton(),
                      IS_BAR
                        ? h(
                            'button',
                            {
                              type: 'button',
                              title: '收起聊天记录（回到纯净输入框）',
                              onClick: (ev) => {
                                ev.stopPropagation();
                                toggleHistorySheet();
                              },
                              style: {
                                width: '20px',
                                height: '20px',
                                padding: 0,
                                borderRadius: '6px',
                                border: 'none',
                                background: 'transparent',
                                color: 'inherit',
                                opacity: 0.6,
                                display: 'inline-flex',
                                alignItems: 'center',
                                justifyContent: 'center',
                                cursor: 'pointer',
                                flexShrink: 0,
                              },
                            },
                            HARNESS_SDK_ICONS.chevronDown('currentColor'),
                          )
                        : null,
                    ),
                  ),

                  h(
                    'div',
                    {
                      key: 'history-scroll',
                      ref: scrollRef,
                      onScroll: handleHistoryScroll,
                      'data-dg-scroll': '',
                      style: {
                        position: 'relative',
                        flex: 1,
                        minHeight: 0,
                        overflowY: 'auto',
                        padding: '12px 16px 14px',
                        display: 'flex',
                        flexDirection: 'column',
                        gap: '12px',
                      },
                    },
                    entries.length === 0
                      ? h(
                          'div',
                          {
                            style: {
                              opacity: 0.65,
                              fontSize: '12.5px',
                              textAlign: 'center',
                              padding: '28px 12px',
                            },
                          },
                          getHarnessMeta(activeHarness).emptyHistoryHint,
                        )
                      : entries.map((item, idx) => {
                          const isUser = item.role === 'user';
                          const itemId = String(item.id);
                          return h(
                            React.Fragment,
                            { key: itemId },
                            idx > 0
                              ? h('div', {
                                  'data-dsb-msg-sep': '',
                                  'aria-hidden': 'true',
                                  style: {
                                    height: '1px',
                                    margin: '2px -16px',
                                    flexShrink: 0,
                                    background: 'rgba(0, 0, 0, 0.45)',
                                    boxShadow:
                                      '0 2px 6px rgba(0, 0, 0, 0.55), 0 1px 0 rgba(255, 255, 255, 0.03)',
                                  },
                                })
                              : null,
                            h(
                              'div',
                              {
                                ref: !isUser
                                  ? (el) => {
                                      if (el) msgNodeMapRef.current.set(itemId, el);
                                      else msgNodeMapRef.current.delete(itemId);
                                    }
                                  : undefined,
                                style: {
                                  display: 'flex',
                                  flexDirection: 'column',
                                  alignItems: isUser ? 'flex-end' : 'flex-start',
                                  gap: '4px',
                                },
                              },
                              isUser
                                ? h(
                                    'div',
                                    {
                                      style: {
                                        maxWidth: '84%',
                                        padding: '7px 12px',
                                        borderRadius: '14px',
                                        background: 'var(--dsw-alias-bg-layer-3, rgba(128,128,128,0.18))',
                                        fontSize: '13px',
                                        lineHeight: 1.5,
                                        whiteSpace: 'pre-wrap',
                                        wordBreak: 'break-word',
                                      },
                                    },
                                    item.text,
                                  )
                                : h(MarkdownMessage, {
                                    text: item.text,
                                    accent: (HARNESS_META[activeHarness] || HARNESS_META.dsh).accent,
                                    streamingNode: item.streaming
                                      ? h(
                                          'span',
                                          {
                                            style: {
                                              color: (HARNESS_META[activeHarness] || HARNESS_META.dsh).accent,
                                              fontSize: '11.5px',
                                              fontWeight: 500,
                                            },
                                          },
                                          dsbRuntime.externalActiveTurn &&
                                            dsbRuntime.externalActiveTurn.harness === activeHarness &&
                                            dsbRuntime.externalActiveTurn.toolNote
                                            ? '● ' + dsbRuntime.externalActiveTurn.toolNote + '…'
                                            : '● 生成中…',
                                        )
                                      : null,
                                  }),
                            ),
                          );
                        }),
                  ),
                ]
              : // Collapsed 2-line 56px Cardholder Lip
                h(
                  'div',
                  {
                    key: 'collapsed-lip',
                    onClick: () => {
                      if (editingName) return;
                      if (IS_BAR && barMode === 'lip') {
                        const bubble = dsbRuntime.pendingBubbleTarget;
                        dsbRuntime.pendingBubbleTarget = null;
                        if (bubble && bubble.harness && bubble.sessionId) {
                          const targetH = normalizeHarnessId(bubble.harness);
                          if (targetH !== dsbRuntime.activeHarness) {
                            setActiveHarness(targetH, true);
                          }
                          dsbRuntime.externalSelection[targetH] = {
                            workspaceId: bubble.workspaceId || '',
                            workspacePath: bubble.workspacePath || '',
                            sessionId: bubble.sessionId,
                            title: bubble.title || HARNESS_META[targetH].label,
                          };
                          writeStoredExtSelection(targetH, dsbRuntime.externalSelection[targetH]);
                          loadExternalConversationMessages(targetH, bubble.sessionId);
                          notifyBarRuntime();
                        }
                        sendBarReport({ kind: 'summon_from_lip' });
                      } else {
                        setHovered(true);
                      }
                    },
                    style: {
                      height: COLLAPSED_LIP_PX + 'px',
                      padding: '9px 18px',
                      display: 'flex',
                      alignItems: 'center',
                      cursor: 'pointer',
                      overflow: 'hidden',
                      boxSizing: 'border-box',
                    },
                  },
                  h(
                    'div',
                    {
                      ref: lipViewportRef,
                      'data-dg-msg-pop': '',
                      style: {
                        flex: 1,
                        minWidth: 0,
                        height: '38px',
                        overflow: 'hidden',
                        fontSize: '13px',
                        lineHeight: '19px',
                        whiteSpace: 'normal',
                        wordBreak: 'break-word',
                        overflowWrap: 'anywhere',
                        opacity:
                          (IS_BAR && barMode === 'lip' && dsbRuntime.pendingBubbleTarget) ||
                          latestSingleLine
                            ? 0.92
                            : 0.62,
                      },
                    },
                    IS_BAR && barMode === 'lip' && dsbRuntime.pendingBubbleTarget
                      ? (dsbRuntime.pendingBubbleTarget.title
                          ? '[' + dsbRuntime.pendingBubbleTarget.title + '] '
                          : '') + dsbRuntime.pendingBubbleTarget.previewText
                      : latestSingleLine || getHarnessMeta(activeHarness).emptyLipHint,
                  ),
                ),
            !(IS_BAR)
              ? h('div', {
                  key: 'dsb-divider',
                  'data-dsb-divider': '',
                  'aria-hidden': 'true',
                  style: {
                    position: 'absolute',
                    left: 0,
                    right: 0,
                    bottom: 0,
                    height: '3px',
                    pointerEvents: 'none',
                    zIndex: 2,
                    background:
                      'linear-gradient(to top, var(--dsb-divider-line, rgba(0,0,0,0.32)) 0, var(--dsb-divider-line, rgba(0,0,0,0.32)) 1px, var(--dsb-divider-glow, rgba(0,0,0,0.10)) 1px, transparent 3px)',
                  },
                })
              : null,
            )
            : IS_BAR && barMode !== 'lip' && (dsbRuntime.unreadCount || 0) > 0
              ? h(
                  'div',
                  {
                    'data-dsb-floating-anchor': '1',
                    style: {
                      position: 'absolute',
                      left: 0,
                      right: 0,
                      bottom: DETACHED_GAP_PX + 'px',
                      display: 'flex',
                      justifyContent: 'center',
                      pointerEvents: 'none',
                    },
                  },
                  h(
                    'div',
                    {
                      'data-dsb-floating-toast': '1',
                      onClick: () => {
                        const bubble = dsbRuntime.pendingBubbleTarget;
                        dsbRuntime.pendingBubbleTarget = null;
                        if (bubble && bubble.harness && bubble.sessionId) {
                          const targetH = normalizeHarnessId(bubble.harness);
                          if (targetH !== dsbRuntime.activeHarness) {
                            setActiveHarness(targetH, true);
                          }
                          dsbRuntime.externalSelection[targetH] = {
                            workspaceId: bubble.workspaceId || '',
                            workspacePath: bubble.workspacePath || '',
                            sessionId: bubble.sessionId,
                            title: bubble.title || HARNESS_META[targetH].label,
                          };
                          writeStoredExtSelection(targetH, dsbRuntime.externalSelection[targetH]);
                          loadExternalConversationMessages(targetH, bubble.sessionId);
                        }
                        setHistoryOpen(true);
                        dsbRuntime.unreadCount = 0;
                        dsbRuntime.unreadPreview = '';
                        notifyBarRuntime();
                        scheduleLayoutReport(280);
                      },
                      style: {
                        pointerEvents: 'auto',
                        display: 'inline-flex',
                        alignItems: 'center',
                        gap: '8px',
                        maxWidth: '94%',
                        padding: '6px 10px 6px 12px',
                        borderRadius: '999px',
                        background: 'rgba(26, 28, 34, 0.94)',
                        border: '1px solid rgba(56, 189, 248, 0.34)',
                        boxShadow: '0 10px 28px rgba(0,0,0,0.46)',
                        color: '#eef2ff',
                        fontSize: '12px',
                        cursor: 'pointer',
                      },
                    },
                    h('span', {
                      style: {
                        width: '7px',
                        height: '7px',
                        borderRadius: '999px',
                        background: '#38bdf8',
                        animation: 'dsbUnreadPulse 2s infinite',
                        flexShrink: 0,
                      },
                    }),
                    h('span', { style: { color: '#7dd3fc', fontWeight: 600, flexShrink: 0 } }, (dsbRuntime.unreadCount || 1) + ' 条新回复'),
                    h(
                      'span',
                      { style: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', opacity: 0.9 } },
                      dsbRuntime.unreadPreview || latestSingleLine,
                    ),
                    h(
                      'button',
                      {
                        type: 'button',
                        title: '标为已读并收起提醒',
                        onClick: (ev) => {
                          ev.stopPropagation();
                          dsbRuntime.unreadCount = 0;
                          dsbRuntime.unreadPreview = '';
                          dsbRuntime.pendingBubbleTarget = null;
                          notifyBarRuntime();
                          scheduleLayoutReport(220);
                        },
                        style: {
                          border: 'none',
                          background: 'transparent',
                          color: 'rgba(229,231,235,0.55)',
                          cursor: 'pointer',
                          padding: '0 3px',
                          fontSize: '13px',
                          lineHeight: 1,
                          flexShrink: 0,
                        },
                      },
                      '×',
                    ),
                  ),
                )
              : IS_BAR && barMode !== 'lip' && hoverPeek && latestSingleLine
                ? h(
                    'div',
                    {
                      'data-dsb-floating-anchor': '1',
                      style: {
                        position: 'absolute',
                        left: 0,
                        right: 0,
                        bottom: DETACHED_GAP_PX + 'px',
                        pointerEvents: 'none',
                      },
                    },
                    h(
                      'div',
                      {
                        'data-dsb-floating-peek': '1',
                        style: {
                          padding: '9px 14px',
                          borderRadius: '16px',
                          background: 'rgba(24, 26, 31, 0.94)',
                          border: '1px solid rgba(255, 255, 255, 0.12)',
                          boxShadow: '0 12px 32px rgba(0, 0, 0, 0.48)',
                          color: '#f3f4f6',
                          fontSize: '12.5px',
                          lineHeight: '18px',
                        },
                      },
                      h(
                        'div',
                        { style: { fontSize: '11px', color: '#7dd3fc', marginBottom: '3px', fontWeight: 500 } },
                        '最新回复轻瞥 · 单击拉环展开完整历史',
                      ),
                      h(
                        'div',
                        {
                          style: {
                            display: '-webkit-box',
                            WebkitLineClamp: 2,
                            WebkitBoxOrient: 'vertical',
                            overflow: 'hidden',
                            opacity: 0.9,
                          },
                        },
                        latestSingleLine,
                      ),
                    ),
                  )
                : null,
            IS_BAR && barMode !== 'lip' && activeHarness === 'dsh'
              ? renderDragHandle({ top: 'auto', bottom: '-13px', zIndex: 40, pointerEvents: 'auto' })
              : null,
          ),
          renderExternalNativeComposer(),
          renderComposerPopover(),
          renderQuickSearchPopup(),
        );
      };
    }

    function makeConversationPickerChip(services) {
      const { sessions, workspaces, uiWorkspace, directoryPicker } = services;

      return function ConversationPickerChip(props) {
        if (!IS_BAR) return null;

        const [, setTick] = useState(0);
        const [menuOpen, setMenuOpen] = useState(false);
        const [settingsView, setSettingsView] = useState(false);
        const [actionToast, setActionToast] = useState('');
        const [hoveredHarnessId, setHoveredHarnessId] = useState('');
        const [harnessSearchOpen, setHarnessSearchOpen] = useState(false);
        const [harnessQuery, setHarnessQuery] = useState('');
        const [hoveredWsId, setHoveredWsId] = useState('');
        const [manualPathMode, setManualPathMode] = useState(false);
        const [manualPathValue, setManualPathValue] = useState('');
        const [pickerError, setPickerError] = useState('');
        const [pillMetrics, setPillMetrics] = useState(null);
        const [wsMarginBottom, setWsMarginBottom] = useState(0);
        const [submenuMarginBottom, setSubmenuMarginBottom] = useState(0);

        const btnRef = useRef(null);
        const extAnchorRef = useRef(null);
        const menuRef = useRef(null);
        const harnessPanelRef = useRef(null);
        const wsPanelRef = useRef(null);
        const convPanelRef = useRef(null);
        const harnessRowRefs = useRef(new Map());
        const wsRowRefs = useRef(new Map());

        useEffect(() => {
          dsbRuntime.toggleHarnessMenu = (anchorEl) => {
            extAnchorRef.current = anchorEl || null;
            setSettingsView(false);
            setMenuOpen((prev) => !prev);
          };
          dsbRuntime.openSettingsMenu = () => {
            refreshAppSettings();
            setSettingsView(true);
            setMenuOpen(true);
          };
          return () => {
            if (dsbRuntime.toggleHarnessMenu) dsbRuntime.toggleHarnessMenu = null;
            if (dsbRuntime.openSettingsMenu) dsbRuntime.openSettingsMenu = null;
          };
        }, []);

        // Subscribe to workspaces.list, sessions.list, and dsbRuntime updates
        useEffect(() => {
          const bump = () => setTick((n) => n + 1);
          dsbRuntime.listeners.add(bump);
          const offWs = workspaces && workspaces.list && workspaces.list.subscribe
            ? workspaces.list.subscribe(bump)
            : null;
          const offSess = sessions && sessions.list && sessions.list.subscribe
            ? sessions.list.subscribe(bump)
            : null;
          return () => {
            dsbRuntime.listeners.delete(bump);
            try { if (typeof offWs === 'function') offWs(); } catch {}
            try { if (typeof offSess === 'function') offSess(); } catch {}
          };
        }, []);

        // Sync popupOpen with dsbRuntime
        useEffect(() => {
          dsbRuntime.popupOpen = menuOpen;
          dsbRuntime.setPopupCloseListener = () => setMenuOpen(false);
          notifyBarRuntime();
          if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
            dsbRuntime.scheduleLayoutReport(280);
          }
          return () => {
            if (!menuOpen) dsbRuntime.popupOpen = false;
          };
        }, [menuOpen]);

        // Copy sibling toolbar pill height/radius/background/border when mounted
        useLayoutEffect(() => {
          if (typeof document === 'undefined') return;
          const siblingBtn =
            document.querySelector('[data-composer-card] [data-slot="conversation.input.permission"] button') ||
            document.querySelector('[data-composer-card] button');
          if (siblingBtn) {
            const cs = getComputedStyle(siblingBtn);
            setPillMetrics({
              height: cs.height || '28px',
              borderRadius: cs.borderRadius || '9999px',
              fontSize: cs.fontSize || '12px',
              background: cs.backgroundColor,
              color: cs.color,
              border: cs.border,
            });
          }
        }, []);

        const activeHarness = dsbRuntime.activeHarness || 'dsh';
        const activeMeta = getHarnessMeta(activeHarness);
        // Level 1 hover browses that Harness's Workspaces & Conversations immediately without
        // forcing an active-composer switch until the user clicks a Harness, Workspace, or Chat.
        const browsedHarness =
          menuOpen && hoveredHarnessId && HARNESS_META[hoveredHarnessId]
            ? hoveredHarnessId
            : activeHarness;
        const browsedMeta = getHarnessMeta(browsedHarness);
        const harnessTheme = activeMeta;

        const wsSnap = workspaces && workspaces.list ? workspaces.list.getSnapshot() : { items: [], archivedSessionIds: [], pinnedSessionIds: [] };
        const sessSnap = sessions && sessions.list ? sessions.list.getSnapshot() : { ids: [], byId: {} };
        const archivedSet = new Set(Array.isArray(wsSnap.archivedSessionIds) ? wsSnap.archivedSessionIds : []);
        const pinnedOrder = Array.isArray(wsSnap.pinnedSessionIds) ? wsSnap.pinnedSessionIds : [];
        const pinnedRank = new Map(pinnedOrder.map((id, idx) => [String(id), idx]));
        const wsItems = Array.isArray(wsSnap.items) ? wsSnap.items : [];
        const byId = sessSnap.byId || {};

        const dshActiveSessionId =
          (props && props.sessionId ? String(props.sessionId) : '') ||
          dsbRuntime.sessionId ||
          mainSessionIdOfList(sessSnap);

        const activeSession = dshActiveSessionId ? byId[dshActiveSessionId] : null;
        const activeExtSel = !activeMeta.useNativeComposer ? dsbRuntime.externalSelection[activeHarness] || {} : null;
        const browsedExtSel = !browsedMeta.useNativeComposer ? dsbRuntime.externalSelection[browsedHarness] || {} : null;
        const activeSessionId = activeMeta.useNativeComposer
          ? dshActiveSessionId
          : String((activeExtSel && activeExtSel.sessionId) || '');
        const browsedSessionId = browsedMeta.useNativeComposer
          ? dshActiveSessionId
          : String((browsedExtSel && browsedExtSel.sessionId) || '');
        const activeTitle = activeMeta.useNativeComposer
          ? sessionDisplayTitle(activeSession)
          : (activeExtSel && activeExtSel.title) || activeMeta.label + ' · 新对话';

        // Compute grouped workspaces and conversations for the browsed Harness (Level 2 & Level 3)
        const { groups, browsedActiveWsId } = useMemo(() => {
          if (!browsedMeta.useNativeComposer) {
            const catalog = dsbRuntime.externalCatalogs[browsedHarness];
            const extGroups = catalog && Array.isArray(catalog.groups) ? catalog.groups : [];
            const selWsId = (browsedExtSel && browsedExtSel.workspaceId) || '';
            const found = extGroups.some((g) => g.workspaceId === selWsId)
              ? selWsId
              : extGroups[0]
                ? extGroups[0].workspaceId
                : '';
            return {
              groups: extGroups,
              browsedActiveWsId: found,
            };
          }

          const toEpochMs = (v) => {
            if (typeof v === 'number' && Number.isFinite(v)) return v;
            if (typeof v === 'string' && v.trim()) {
              const n = Number(v);
              if (Number.isFinite(n)) return n;
              const p = Date.parse(v);
              if (Number.isFinite(p)) return p;
            }
            return 0;
          };
          // Exact 1:1 parity with @deepseek-ai/dsh-client-ui-workspace `sessionVisible`:
          // hide subagent sessions, hide non-active blank sessions, and hide archived sessions.
          const isDshSessionVisible = (id, s) => {
            if (!s) return false;
            if (s.origin === 'subagent') return false;
            if (s.blank && id !== dshActiveSessionId) return false;
            if (archivedSet.has(id)) return false;
            return true;
          };
          const sortDshConvs = (arr) => {
            arr.sort((a, b) => {
              const pa = pinnedRank.has(a.id) ? pinnedRank.get(a.id) : Infinity;
              const pb = pinnedRank.has(b.id) ? pinnedRank.get(b.id) : Infinity;
              if (pa !== pb) return pa - pb;
              const ta = toEpochMs(a.updatedAt);
              const tb = toEpochMs(b.updatedAt);
              if (ta !== tb) return tb - ta;
              return a.id < b.id ? -1 : 1;
            });
          };

          const accounted = new Set();
          let foundWsId = '';
          const result = [];

          for (const ws of wsItems) {
            if (!ws || !ws.workspaceId) continue;
            const convs = [];
            const ids = Array.isArray(ws.sessionIds) ? ws.sessionIds : [];
            for (const id of ids) {
              accounted.add(id);
              const s = byId[id];
              if (!isDshSessionVisible(id, s)) continue;
              if (id === dshActiveSessionId) foundWsId = ws.workspaceId;
              convs.push({
                id,
                title: sessionDisplayTitle(s),
                updatedAt: s.updatedAt || 0,
              });
            }
            sortDshConvs(convs);
            result.push({
              workspaceId: ws.workspaceId,
              label: ws.title || workspaceBasename(ws.path),
              path: ws.path || '',
              convs,
            });
          }

          const stray = [];
          for (const id of (Array.isArray(sessSnap.ids) ? sessSnap.ids : [])) {
            if (accounted.has(id)) continue;
            const s = byId[id];
            if (!isDshSessionVisible(id, s)) continue;
            if (id === dshActiveSessionId) foundWsId = '__ungrouped__';
            stray.push({
              id,
              title: sessionDisplayTitle(s),
              updatedAt: s.updatedAt || 0,
            });
          }
          if (stray.length > 0) {
            sortDshConvs(stray);
            result.push({
              workspaceId: '__ungrouped__',
              label: '其他对话',
              path: '',
              convs: stray,
            });
          }

          return {
            groups: result,
            browsedActiveWsId: foundWsId || (result[0] ? result[0].workspaceId : ''),
          };
        }, [
          browsedHarness,
          browsedMeta.useNativeComposer,
          wsItems,
          sessSnap,
          dshActiveSessionId,
          browsedExtSel && browsedExtSel.workspaceId,
          dsbRuntime.externalCatalogs[browsedHarness],
        ]);

        // Reset browsed Harness + search filter whenever menu opens
        useEffect(() => {
          if (menuOpen) {
            setHoveredHarnessId(activeHarness);
            setHarnessQuery('');
            setHarnessSearchOpen(HARNESS_ORDER.length > 6);
            refreshHarnessAvailability();
            if (!dsbRuntime.appSettings) {
              refreshAppSettings();
            }
            if (!activeMeta.useNativeComposer) {
              refreshExternalCatalog(activeHarness, false);
            }
            setManualPathMode(false);
            setPickerError('');
          }
        }, [menuOpen]);

        // Sync hovered workspace & lazily load catalog when hovering a different Harness in Level 1
        useEffect(() => {
          if (!menuOpen) return;
          if (!browsedMeta.useNativeComposer && !dsbRuntime.externalCatalogs[browsedHarness] && !dsbRuntime.externalLoading[browsedHarness]) {
            refreshExternalCatalog(browsedHarness, false);
          }
          setHoveredWsId(browsedActiveWsId || (groups[0] ? groups[0].workspaceId : ''));
          setManualPathMode(false);
          setPickerError('');
        }, [menuOpen, browsedHarness, browsedActiveWsId]);

        // Outside click / Esc handling for popup
        useEffect(() => {
          if (!menuOpen) return undefined;
          const onPointerDown = (e) => {
            const t = e.target;
            if (btnRef.current && t instanceof Node && btnRef.current.contains(t)) return;
            if (t instanceof Element && t.closest('[data-dsb-session-chip="1"]')) return;
            if (menuRef.current && t instanceof Node && menuRef.current.contains(t)) return;
            setMenuOpen(false);
          };
          const onKeyDown = (e) => {
            if (e.key === 'Escape') {
              e.preventDefault();
              e.stopPropagation();
              setMenuOpen(false);
            }
          };
          const onWinResize = () => setTick((n) => n + 1);
          document.addEventListener('pointerdown', onPointerDown, true);
          document.addEventListener('keydown', onKeyDown, true);
          window.addEventListener('resize', onWinResize);
          return () => {
            document.removeEventListener('pointerdown', onPointerDown, true);
            document.removeEventListener('keydown', onKeyDown, true);
            window.removeEventListener('resize', onWinResize);
          };
        }, [menuOpen]);

        // Re-snap popover coordinates and Win32 window shapes whenever Harness/columns change
        useEffect(() => {
          if (!menuOpen) return undefined;
          const raf = window.requestAnimationFrame(() => {
            setTick((n) => n + 1);
            if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
              dsbRuntime.scheduleLayoutReport(280);
            }
          });
          return () => window.cancelAnimationFrame(raf);
        }, [menuOpen, activeHarness, browsedHarness, hoveredWsId, settingsView]);

        const handleOpenSession = (conv, group, targetHarness = browsedHarness) => {
          setMenuOpen(false);
          const targetMeta = getHarnessMeta(targetHarness);
          if (!targetMeta.useNativeComposer) {
            selectExternalConversation(targetHarness, group, conv);
            if (targetHarness !== activeHarness) {
              setActiveHarness(targetHarness, true, true);
            }
            return;
          }
          if (targetHarness !== activeHarness) {
            setActiveHarness(targetHarness, true, true);
          }
          const id = typeof conv === 'string' ? conv : conv.id;
          writeStoredString(STORAGE_KEY_LAST_SESSION, id);
          if (uiWorkspace && typeof uiWorkspace.openSession === 'function') {
            try {
              uiWorkspace.openSession(id);
              return;
            } catch {}
          }
          window.location.replace('/sessions/' + encodeURIComponent(id) + '?dsh_bar_clone=1');
        };

        const handleNewConversation = async (group, targetHarness = browsedHarness) => {
          setMenuOpen(false);
          const targetMeta = getHarnessMeta(targetHarness);
          if (!targetMeta.useNativeComposer) {
            startNewExternalConversation(targetHarness, group);
            if (targetHarness !== activeHarness) {
              setActiveHarness(targetHarness, true, true);
            }
            return;
          }
          if (targetHarness !== activeHarness) {
            setActiveHarness(targetHarness, true, true);
          }
          const wsId = typeof group === 'string' ? group : group.workspaceId;
          try {
            if (uiWorkspace && typeof uiWorkspace.startSession === 'function') {
              await uiWorkspace.startSession(wsId);
              return;
            }
            if (sessions && typeof sessions.create === 'function') {
              const newId = await sessions.create({ workspaceId: wsId });
              if (newId) handleOpenSession(newId, group, targetHarness);
            }
          } catch (err) {
            setPickerError(err instanceof Error ? err.message : String(err));
          }
        };

        const inferDefaultWorkspaceParent = (wsList) => {
          if (!Array.isArray(wsList)) return null;
          for (const g of wsList) {
            const raw = g && g.path ? String(g.path).trim() : '';
            if (!raw) continue;
            const cleaned = raw.replace(/[\\/]+$/, '');
            const idx = Math.max(cleaned.lastIndexOf('\\'), cleaned.lastIndexOf('/'));
            if (idx > 2) {
              return cleaned.slice(0, idx);
            }
            if (cleaned) return cleaned;
          }
          return null;
        };

        const createWorkspaceAtPath = async (pickedPath, targetHarness = browsedHarness) => {
          if (!pickedPath) return;
          const targetMeta = getHarnessMeta(targetHarness);
          if (!targetMeta.useNativeComposer) {
            const label = workspaceBasename(pickedPath);
            const newGroup = {
              workspaceId: pickedPath,
              label,
              path: pickedPath,
              convs: [],
            };
            const curCat = dsbRuntime.externalCatalogs[targetHarness];
            if (curCat && Array.isArray(curCat.groups)) {
              const normKey = pickedPath.replace(/[\\/]+$/, '').toLowerCase();
              const exists = curCat.groups.some(
                (g) => g && g.path && String(g.path).replace(/[\\/]+$/, '').toLowerCase() === normKey,
              );
              if (!exists) {
                dsbRuntime.externalCatalogs[targetHarness] = {
                  ...curCat,
                  groups: [newGroup, ...curCat.groups],
                };
              }
            }
            startNewExternalConversation(targetHarness, newGroup);
            if (targetHarness !== activeHarness) {
              setActiveHarness(targetHarness, true, true);
            }
            setMenuOpen(false);
            return;
          }
          if (targetHarness !== activeHarness) {
            setActiveHarness(targetHarness, true, true);
          }
          if (!workspaces || typeof workspaces.create !== 'function') return;
          const ws = await workspaces.create({ path: pickedPath });
          const wsId = ws && ws.workspaceId ? ws.workspaceId : '';
          setMenuOpen(false);
          if (wsId && uiWorkspace && typeof uiWorkspace.startSession === 'function') {
            await uiWorkspace.startSession(wsId);
          }
        };

        const handleCreateWorkspace = async () => {
          setPickerError('');
          const initialDir = inferDefaultWorkspaceParent(groups);
          dsbRuntime.modalOpen = true;
          notifyBarRuntime();
          try {
            let pickedPath = null;
            try {
              pickedPath = await tauriInvoke('app_pick_folder', { initialDir });
            } catch {
              if (browsedMeta.useNativeComposer) {
                if (uiWorkspace && typeof uiWorkspace.pickDirectory === 'function') {
                  pickedPath = await uiWorkspace.pickDirectory();
                } else if (directoryPicker && typeof directoryPicker.pick === 'function') {
                  const res = await directoryPicker.pick();
                  if (res && res.ok) pickedPath = res.value;
                } else {
                  setManualPathMode(true);
                  return;
                }
              } else {
                setManualPathMode(true);
                return;
              }
            }
            if (pickedPath) {
              await createWorkspaceAtPath(pickedPath, browsedHarness);
            }
          } catch {
            setManualPathMode(true);
          } finally {
            dsbRuntime.modalOpen = false;
            notifyBarRuntime();
          }
        };

        const hoveredGroup =
          groups.find((g) => g.workspaceId === hoveredWsId) || groups[0] || null;

        const recomputeSubmenuAlignment = useCallback(() => {
          if (!menuOpen || settingsView) {
            setWsMarginBottom(0);
            setSubmenuMarginBottom(0);
            return;
          }
          const hPanelEl = harnessPanelRef.current;
          const wsPanelEl = wsPanelRef.current;
          const hRowEl = harnessRowRefs.current.get(browsedHarness);
          if (!hPanelEl || !wsPanelEl) {
            setWsMarginBottom(0);
            setSubmenuMarginBottom(0);
            return;
          }
          const hr = hPanelEl.getBoundingClientRect();
          const wr = wsPanelEl.getBoundingClientRect();
          if (hr.height < 12 || wr.height < 12) {
            setWsMarginBottom(0);
            setSubmenuMarginBottom(0);
            return;
          }

          // Level 1 (`harnessPanelRef`) always sits at marginBottom=0, so `hr.bottom` is
          // the exact bottom baseline of the 3-column flex container.
          const baseBottom = hr.bottom;

          // 1. Align Level 2 (Workspaces) with the hovered Harness row in Level 1
          let nextWsBottom = 0;
          if (hRowEl) {
            const hrr = hRowEl.getBoundingClientRect();
            if (hrr.height >= 8) {
              const desiredWsTop = Math.max(hr.top, hrr.top - 7);
              const maxWsBottom = Math.max(0, Math.round(hr.height - wr.height));
              const desiredWsBottom = Math.round(baseBottom - (desiredWsTop + wr.height));
              nextWsBottom = Math.max(0, Math.min(maxWsBottom, desiredWsBottom));
            }
          }

          // 2. Align Level 3 (Conversations) with the hovered Workspace row in Level 2
          //    (accounting for Level 2's lifted position `nextWsBottom`)
          let nextConvBottom = 0;
          const convEl = convPanelRef.current;
          const wRowEl = hoveredGroup ? wsRowRefs.current.get(hoveredGroup.workspaceId) : null;
          if (convEl && wRowEl) {
            const cr = convEl.getBoundingClientRect();
            const wrr = wRowEl.getBoundingClientRect();
            if (cr.height >= 12 && wrr.height >= 8) {
              const wsNewTop = baseBottom - nextWsBottom - wr.height;
              const rowOffsetInWs = wrr.top - wr.top;
              const wsRowNewTop = wsNewTop + rowOffsetInWs;
              const desiredConvTop = Math.max(wsNewTop, wsRowNewTop - 7);
              const maxConvBottom = Math.max(0, Math.round(baseBottom - (wsNewTop + cr.height)));
              const desiredConvBottom = Math.round(baseBottom - (desiredConvTop + cr.height));
              nextConvBottom = Math.max(0, Math.min(maxConvBottom, desiredConvBottom));
            }
          }

          let changed = false;
          setWsMarginBottom((prev) => {
            if (prev !== nextWsBottom) changed = true;
            return prev === nextWsBottom ? prev : nextWsBottom;
          });
          setSubmenuMarginBottom((prev) => {
            if (prev !== nextConvBottom) changed = true;
            return prev === nextConvBottom ? prev : nextConvBottom;
          });
          if (changed && typeof dsbRuntime.scheduleLayoutReport === 'function') {
            dsbRuntime.scheduleLayoutReport(180);
          }
        }, [
          menuOpen,
          settingsView,
          browsedHarness,
          groups.length,
          hoveredGroup ? hoveredGroup.workspaceId : '',
          hoveredGroup ? hoveredGroup.convs.length : 0,
        ]);

        useLayoutEffect(() => {
          recomputeSubmenuAlignment();
          if (!menuOpen || settingsView) return undefined;
          const raf = window.requestAnimationFrame(recomputeSubmenuAlignment);
          return () => window.cancelAnimationFrame(raf);
        }, [recomputeSubmenuAlignment, menuOpen, settingsView, browsedHarness, groups.length, harnessQuery]);

        const availById = useMemo(() => {
          const map = {};
          for (const item of dsbRuntime.harnessAvailability || []) {
            if (item && item.id) map[item.id] = item;
          }
          return map;
        }, [dsbRuntime.harnessAvailability]);

        const filteredHarnesses = useMemo(() => {
          const q = String(harnessQuery || '').trim().toLowerCase();
          if (!q) return HARNESS_ORDER;
          return HARNESS_ORDER.filter((hid) => {
            const m = getHarnessMeta(hid);
            if (hid.includes(q)) return true;
            if (String(m.label || '').toLowerCase().includes(q)) return true;
            if (String(m.fullLabel || '').toLowerCase().includes(q)) return true;
            if (Array.isArray(m.aliases) && m.aliases.some((a) => String(a).toLowerCase().includes(q))) return true;
            return false;
          });
        }, [harnessQuery]);

        const renderMenu = () => {
          if (!menuOpen) return null;
          const isValidRect = (r) => !!(r && r.width > 4 && r.height > 4 && r.bottom > 0);

          const cardEl = document.querySelector('[data-dsb-card]');
          const compEl = !activeMeta.useNativeComposer
            ? document.querySelector('[data-dsb-external-composer]')
            : document.querySelector('[data-composer-card]');
          const cr = cardEl ? cardEl.getBoundingClientRect() : null;
          const ir = compEl ? compEl.getBoundingClientRect() : null;
          let slabTop = null;
          let slabBottom = null;
          if (isValidRect(cr)) {
            slabTop = cr.top;
            slabBottom = cr.bottom;
          }
          if (isValidRect(ir)) {
            slabTop = slabTop === null ? ir.top : Math.min(slabTop, ir.top);
            slabBottom = slabBottom === null ? ir.bottom : Math.max(slabBottom, ir.bottom);
          }
          const fallbackSlabH = activeMeta.fallbackSlabHeight || 158;
          const slabHeight =
            slabTop !== null && slabBottom !== null
              ? Math.max(56, Math.min(380, Math.round(slabBottom - slabTop)))
              : fallbackSlabH;
          const bottomPx = slabHeight + 8;

          let chipRect = null;
          if (!activeMeta.useNativeComposer) {
            const liveExtChip = document.querySelector(
              '[data-dsb-external-composer] [data-dsb-session-chip="1"]',
            );
            const r1 = liveExtChip ? liveExtChip.getBoundingClientRect() : null;
            const r2 =
              extAnchorRef.current && extAnchorRef.current.isConnected
                ? extAnchorRef.current.getBoundingClientRect()
                : null;
            if (isValidRect(r1)) chipRect = r1;
            else if (isValidRect(r2)) chipRect = r2;
          } else if (btnRef.current && btnRef.current.isConnected) {
            const r0 = btnRef.current.getBoundingClientRect();
            if (isValidRect(r0)) chipRect = r0;
          }

          // Responsive 3-column widths so Harness -> Workspace -> Conversation fits comfortably
          // inside both standard 760px Bar and narrow dragged Bars (>= 480px).
          const winW = typeof window !== 'undefined' ? window.innerWidth || 760 : 760;
          const compact3Col = winW < 640;
          const col1W = compact3Col ? 138 : 166;
          const col2W = compact3Col ? 156 : 188;
          const col3W = compact3Col ? 176 : 236;
          const totalCascadeW = settingsView
            ? Math.min(468, Math.max(360, winW - 16))
            : col1W + 6 + col2W + 6 + col3W;
          const leftPx = chipRect
            ? Math.max(8, Math.min(Math.round(chipRect.left), Math.max(8, winW - totalCascadeW - 8)))
            : 12;

          const unifiedPanelTheme = getUnifiedTheme(activeHarness);
          const panelBg = unifiedPanelTheme.popover;
          const panelBorder = unifiedPanelTheme.border;
          const panelFg = unifiedPanelTheme.fg;
          const panelMuted = unifiedPanelTheme.muted;
          const panelRowBg = unifiedPanelTheme.isDark ? 'rgba(255,255,255,0.04)' : 'rgba(15,23,42,0.04)';
          const panelShadow = unifiedPanelTheme.isDark
            ? '0 10px 28px rgba(0,0,0,0.46)'
            : '0 10px 26px rgba(15,23,42,0.14)';
          const browsedTheme = Object.assign({}, browsedMeta, getUnifiedTheme(browsedHarness));

          const snap = dsbRuntime.appSettings || {
            version: '0.6.0',
            isElevated: false,
            dshStatus: '已连接',
            config: {
              outputTarget: 'dsh',
              startupBreathPulse: true,
              hideFromScreenCapture: false,
              showTrayIcon: false,
              syncExternalPlugins: true,
              preferAdmin: true,
              autostart: false,
            },
            syncStatus: {
              enabled: true,
              dshPlugin: true,
              antigravityPlugin: true,
              claudeCodePlugin: true,
              codexPlugin: true,
              codexLiveWatch: true,
            },
          };
          const cfg = snap.config || {};
          const syncSt = snap.syncStatus || {};

          const runSettingsAction = (action, toastMsg) => {
            if (toastMsg) {
              setActionToast(toastMsg);
              setTimeout(() => setActionToast(''), 1800);
            }
            tauriInvoke('app_settings_action', { action })
              .then(() => {
                if (action === 'resync_plugins' || action === 'reconnect_dsh') {
                  refreshAppSettings();
                }
              })
              .catch(() => {});
          };

          const renderToggleRow = (key, title, desc, checked, onToggle, extraBtn) =>
            h(
              'div',
              {
                key,
                style: {
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: '10px',
                  padding: '6px 8px',
                  borderRadius: '10px',
                  background: panelRowBg,
                  color: panelFg,
                },
              },
              h(
                'div',
                { style: { minWidth: 0, flex: 1 } },
                h('div', { style: { fontSize: '12px', fontWeight: 550, lineHeight: '16px' } }, title),
                desc
                  ? h(
                      'div',
                      { style: { fontSize: '10.5px', opacity: 0.56, marginTop: '2px', lineHeight: '14px' } },
                      desc,
                    )
                  : null,
              ),
              h(
                'div',
                { style: { display: 'flex', alignItems: 'center', gap: '6px', flexShrink: 0 } },
                extraBtn || null,
                h(
                  'button',
                  {
                    type: 'button',
                    role: 'switch',
                    'aria-checked': checked ? 'true' : 'false',
                    'data-dsb-setting-toggle': key,
                    onClick: () => onToggle(!checked),
                    style: {
                      width: '34px',
                      height: '19px',
                      borderRadius: '999px',
                      border: 'none',
                      padding: '2px',
                      cursor: 'pointer',
                      background: checked ? harnessTheme.accent : 'rgba(128,128,128,0.34)',
                      display: 'inline-flex',
                      alignItems: 'center',
                      justifyContent: checked ? 'flex-end' : 'flex-start',
                      transition: 'background 0.14s ease',
                    },
                  },
                  h('span', {
                    style: {
                      width: '15px',
                      height: '15px',
                      borderRadius: '999px',
                      background: '#ffffff',
                      boxShadow: '0 1px 3px rgba(0,0,0,0.32)',
                    },
                  }),
                ),
              ),
            );

          const renderSettingsBody = () =>
            h(
              'div',
              {
                'data-dsb-popup-panel': '1',
                'data-dsb-settings-panel': '1',
                'data-dg-scroll': '1',
                style: {
                  width: totalCascadeW + 'px',
                  maxHeight: '430px',
                  overflowY: 'auto',
                  borderRadius: '14px',
                  padding: '9px 10px',
                  background: panelBg,
                  color: panelFg,
                  border: '1px solid ' + panelBorder,
                  boxShadow: panelShadow,
                  display: 'flex',
                  flexDirection: 'column',
                  gap: '6px',
                  boxSizing: 'border-box',
                },
              },
              h(
                'div',
                {
                  style: {
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    padding: '0 2px 4px',
                    borderBottom: '1px solid ' + panelBorder,
                  },
                },
                h(
                  'div',
                  { style: { display: 'flex', alignItems: 'center', gap: '6px' } },
                  h('span', { style: { fontSize: '12.5px', fontWeight: 650, color: panelFg } }, '插件与悬浮条设置'),
                  h(
                    'span',
                    {
                      style: {
                        fontSize: '10.5px',
                        padding: '1px 6px',
                        borderRadius: '999px',
                        background: unifiedPanelTheme.accentBg,
                        color: unifiedPanelTheme.accent,
                        border: '1px solid ' + unifiedPanelTheme.accentBorder,
                      },
                    },
                    'v' + (snap.version || '0.6.0'),
                  ),
                  actionToast
                    ? h('span', { style: { fontSize: '11px', color: '#34d399', marginLeft: '4px' } }, actionToast)
                    : null,
                ),
                h(
                  'button',
                  {
                    type: 'button',
                    'data-dsb-settings-back': '1',
                    onClick: () => setSettingsView(false),
                    style: {
                      border: '1px solid ' + panelBorder,
                      background: panelRowBg,
                      color: panelFg,
                      borderRadius: '8px',
                      padding: '3px 8px',
                      fontSize: '11.5px',
                      cursor: 'pointer',
                    },
                  },
                  '← 返回菜单',
                ),
              ),
              // Dual-Theme Mode Selector (Auto / Light / Dark)
              h(
                'div',
                { style: { display: 'flex', flexDirection: 'column', gap: '4px' } },
                h('div', { style: { fontSize: '11px', color: panelMuted, paddingLeft: '4px' } }, '外观主题（支持明亮 / 黑暗双模）'),
                h(
                  'div',
                  { style: { display: 'flex', gap: '4px' } },
                  [
                    { id: 'auto', label: '跟随 DSH' },
                    { id: 'light', label: '☀️ 明亮' },
                    { id: 'dark', label: '🌙 黑暗' },
                  ].map((opt) => {
                    const active = (dsbRuntime.themePref || 'auto') === opt.id;
                    return h(
                      'button',
                      {
                        key: 'theme-' + opt.id,
                        type: 'button',
                        'data-dsb-theme-opt': opt.id,
                        onClick: () => setThemePreference(opt.id),
                        style: {
                          flex: 1,
                          padding: '5px 6px',
                          borderRadius: '8px',
                          fontSize: '11.5px',
                          fontWeight: active ? 600 : 450,
                          cursor: 'pointer',
                          border: active ? '1px solid ' + unifiedPanelTheme.accentBorder : '1px solid transparent',
                          background: active ? unifiedPanelTheme.accentBg : panelRowBg,
                          color: active ? unifiedPanelTheme.accent : panelFg,
                        },
                      },
                      opt.label,
                    );
                  }),
                ),
              ),
              h(
                'div',
                { style: { display: 'flex', flexDirection: 'column', gap: '4px' } },
                h('div', { style: { fontSize: '11px', color: panelMuted, paddingLeft: '4px' } }, '选区胶囊投递目标'),
                h(
                  'div',
                  { style: { display: 'flex', gap: '4px' } },
                  [
                    { id: 'dsh', label: '悬浮条（默认）' },
                    { id: 'dsh_and_clipboard', label: '悬浮条 + 剪贴板' },
                    { id: 'clipboard_only', label: '仅复制胶囊' },
                  ].map((opt) => {
                    const active = (cfg.outputTarget || 'dsh') === opt.id;
                    return h(
                      'button',
                      {
                        key: opt.id,
                        type: 'button',
                        'data-dsb-target-opt': opt.id,
                        onClick: () => updateAppSettings({ outputTarget: opt.id }),
                        style: {
                          flex: 1,
                          padding: '5px 6px',
                          borderRadius: '8px',
                          fontSize: '11.5px',
                          fontWeight: active ? 600 : 450,
                          cursor: 'pointer',
                          border: active ? '1px solid ' + unifiedPanelTheme.accentBorder : '1px solid transparent',
                          background: active ? unifiedPanelTheme.accentBg : panelRowBg,
                          color: active ? unifiedPanelTheme.accent : panelFg,
                        },
                      },
                      opt.label,
                    );
                  }),
                ),
              ),
              h(
                'div',
                { style: { display: 'flex', flexDirection: 'column', gap: '4px' } },
                renderToggleRow(
                  'syncExternalPlugins',
                  '多 Harness 伴随启动 & 实时对话同步',
                  'DSH ' +
                    (syncSt.dshPlugin ? '✓' : '·') +
                    '  Antigravity ' +
                    (syncSt.antigravityPlugin ? '✓' : '·') +
                    '  Claude ' +
                    (syncSt.claudeCodePlugin ? '✓' : '·') +
                    '  Codex ' +
                    (syncSt.codexPlugin || syncSt.codexLiveWatch ? '✓' : '·'),
                  cfg.syncExternalPlugins !== false,
                  (v) => updateAppSettings({ syncExternalPlugins: v }),
                ),
                renderToggleRow(
                  'startupBreathPulse',
                  '启动鼠标呼吸灯',
                  '就绪时指针柔和亮起蓝光 1 秒后熄灭，提示 Ctrl 已生效',
                  cfg.startupBreathPulse !== false,
                  (v) => updateAppSettings({ startupBreathPulse: v }),
                  h(
                    'button',
                    {
                      type: 'button',
                      onClick: () => runSettingsAction('preview_breath', '呼吸灯演示中…'),
                      style: {
                        fontSize: '11px',
                        padding: '2px 7px',
                        borderRadius: '6px',
                        border: '1px solid ' + harnessTheme.accentBorder,
                        background: harnessTheme.accentBg,
                        color: harnessTheme.accent,
                        cursor: 'pointer',
                      },
                    },
                    '试一下',
                  ),
                ),
                renderToggleRow(
                  'hideFromScreenCapture',
                  '向录屏 / 远程桌面隐藏窗口',
                  '关闭即可在 GameViewer / ToDesk / OBS 中正常显示',
                  Boolean(cfg.hideFromScreenCapture),
                  (v) => updateAppSettings({ hideFromScreenCapture: v }),
                ),
                renderToggleRow(
                  'showTrayIcon',
                  '显示右下角系统托盘图标',
                  '默认关闭（极简零图标，全部设置均在此面板）',
                  Boolean(cfg.showTrayIcon),
                  (v) => updateAppSettings({ showTrayIcon: v }),
                ),
                renderToggleRow(
                  'preferAdmin',
                  '优先管理员权限运行',
                  '确保可拾取管理员权限窗口（外部 CLI 仍走普通权限 Broker）',
                  cfg.preferAdmin !== false,
                  (v) => updateAppSettings({ preferAdmin: v }),
                ),
                renderToggleRow(
                  'autostart',
                  'Windows 开机自动启动',
                  '登录系统后自动在后台静默就绪，随时双击 Ctrl 唤出',
                  Boolean(cfg.autostart),
                  (v) => updateAppSettings({ autostart: v }),
                ),
              ),
              h(
                'div',
                {
                  style: {
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    gap: '4px',
                    paddingTop: '6px',
                    borderTop: '1px solid ' + panelBorder,
                  },
                },
                h(
                  'div',
                  { style: { display: 'flex', alignItems: 'center', gap: '4px', flexWrap: 'wrap' } },
                  [
                    { id: 'resync_plugins', label: '重同步插件', toast: '插件已同步' },
                    { id: 'reconnect_dsh', label: '重连 DSH', toast: '正在重连 DSH…' },
                    { id: 'open_captures', label: '截图目录', toast: '' },
                    { id: 'open_config', label: '配置文件', toast: '' },
                  ].map((act) =>
                    h(
                      'button',
                      {
                        key: act.id,
                        type: 'button',
                        onClick: () => runSettingsAction(act.id, act.toast),
                        style: {
                          fontSize: '11px',
                          padding: '4px 8px',
                          borderRadius: '7px',
                          border: '1px solid ' + panelBorder,
                          background: panelRowBg,
                          color: panelFg,
                          cursor: 'pointer',
                        },
                      },
                      act.label,
                    ),
                  ),
                ),
                h(
                  'button',
                  {
                    type: 'button',
                    'data-dsb-quit-btn': '1',
                    onClick: () => runSettingsAction('quit', ''),
                    style: {
                      fontSize: '11px',
                      padding: '4px 9px',
                      borderRadius: '7px',
                      border: '1px solid rgba(248,113,113,0.42)',
                      background: 'rgba(248,113,113,0.14)',
                      color: '#fca5a5',
                      cursor: 'pointer',
                      flexShrink: 0,
                    },
                  },
                  '退出',
                ),
              ),
            );

          // 3-Level Cascading Menu (Left -> Right):
          // Level 1: Harness (scrollable + searchable, scales to 100+ Harnesses)
          // Level 2: Workspaces of hovered Harness
          // Level 3: Conversations of hovered Workspace
          const menuNode = h(
            'div',
            {
              ref: menuRef,
              'data-dsb-popup': '1',
              'data-dsb-popup-bottom': String(bottomPx),
              'data-dsb-popup-min-height': String(settingsView ? 438 : 308),
              style: {
                position: 'fixed',
                bottom: bottomPx + 'px',
                left: leftPx + 'px',
                display: 'flex',
                flexDirection: 'row',
                alignItems: 'flex-end',
                gap: '6px',
                zIndex: 9999,
                pointerEvents: 'auto',
                color: 'var(--dsw-alias-label-primary, #f3f4f6)',
                fontFamily: 'system-ui, -apple-system, "Segoe UI", Roboto, "PingFang SC", sans-serif',
              },
            },
            settingsView
              ? renderSettingsBody()
              : h(
                  React.Fragment,
                  null,
                  // ===========================================================
                  // Level 1 (Left): Harness List (scales cleanly to 100+ items)
                  // ===========================================================
                  h(
                    'div',
                    {
                      ref: harnessPanelRef,
                      'data-dsb-popup-panel': '1',
                      'data-dsb-harness-col': '1',
                      style: {
                        width: col1W + 'px',
                        maxHeight: '300px',
                        borderRadius: '14px',
                        padding: '6px',
                        background: panelBg,
                        color: panelFg,
                        border: '1px solid ' + panelBorder,
                        boxShadow: panelShadow,
                        display: 'flex',
                        flexDirection: 'column',
                        gap: '2px',
                        boxSizing: 'border-box',
                      },
                    },
                    // Compact column header: title + count + search toggle
                    h(
                      'div',
                      {
                        style: {
                          display: 'flex',
                          alignItems: 'center',
                          justifyContent: 'space-between',
                          padding: '3px 6px 4px',
                          gap: '4px',
                        },
                      },
                      h(
                        'div',
                        { style: { display: 'inline-flex', alignItems: 'center', gap: '5px', minWidth: 0 } },
                        h(
                          'span',
                          { style: { fontSize: '11px', fontWeight: 600, opacity: 0.68, letterSpacing: '0.02em' } },
                          'Harness',
                        ),
                        h(
                          'span',
                          {
                            style: {
                              fontSize: '10px',
                              padding: '0 5px',
                              borderRadius: '999px',
                              background: 'rgba(255,255,255,0.07)',
                              color: 'rgba(229,231,235,0.7)',
                              lineHeight: '16px',
                            },
                          },
                          String(HARNESS_ORDER.length),
                        ),
                      ),
                      h(
                        'button',
                        {
                          type: 'button',
                          'data-dsb-harness-search-btn': '1',
                          title: harnessSearchOpen ? '收起搜索' : '筛选 Harness',
                          onClick: () => {
                            setHarnessSearchOpen((prev) => {
                              if (prev) setHarnessQuery('');
                              return !prev;
                            });
                          },
                          style: {
                            width: '22px',
                            height: '22px',
                            borderRadius: '6px',
                            border: 'none',
                            background: harnessSearchOpen ? 'rgba(255,255,255,0.10)' : 'transparent',
                            cursor: 'pointer',
                            display: 'inline-flex',
                            alignItems: 'center',
                            justifyContent: 'center',
                            padding: 0,
                          },
                        },
                        HARNESS_SDK_ICONS.search(12, harnessSearchOpen ? '#ffffff' : 'rgba(229,231,235,0.62)'),
                      ),
                    ),
                    harnessSearchOpen
                      ? h('input', {
                          type: 'text',
                          'data-dsb-harness-filter': '1',
                          autoFocus: HARNESS_ORDER.length <= 6,
                          placeholder: '筛选 Harness…',
                          value: harnessQuery,
                          onChange: (ev) => setHarnessQuery(ev.target.value),
                          onKeyDown: (ev) => {
                            if (ev.key === 'Escape' && harnessQuery) {
                              ev.stopPropagation();
                              setHarnessQuery('');
                            }
                          },
                          style: {
                            width: '100%',
                            boxSizing: 'border-box',
                            padding: '4px 8px',
                            marginBottom: '2px',
                            borderRadius: '8px',
                            fontSize: '11.5px',
                            background: 'rgba(255,255,255,0.06)',
                            border: '1px solid ' + panelBorder,
                            color: 'inherit',
                            outline: 'none',
                          },
                        })
                      : null,
                    h(
                      'div',
                      {
                        'data-dsb-menu-scroll': '1',
                        onScroll: recomputeSubmenuAlignment,
                        style: {
                          overflowY: 'auto',
                          display: 'flex',
                          flexDirection: 'column',
                          gap: '2px',
                          maxHeight: '204px',
                        },
                      },
                      filteredHarnesses.length === 0
                        ? h(
                            'div',
                            { style: { padding: '10px 6px', fontSize: '11.5px', opacity: 0.55, textAlign: 'center' } },
                            '无匹配项',
                          )
                        : filteredHarnesses.map((hid) => {
                            const meta = getHarnessMeta(hid);
                            const isActiveH = activeHarness === hid;
                            const isBrowsedH = browsedHarness === hid;
                            const avail = availById[hid];
                            const isReady = avail ? Boolean(avail.available) : true;
                            return h(
                              'button',
                              {
                                key: hid,
                                ref: (el) => {
                                  if (el) harnessRowRefs.current.set(hid, el);
                                  else harnessRowRefs.current.delete(hid);
                                },
                                type: 'button',
                                'data-dsb-harness-opt': hid,
                                title: meta.fullLabel + ' · ' + meta.syncHint,
                                onMouseEnter: () => setHoveredHarnessId(hid),
                                onClick: () => {
                                  setHoveredHarnessId(hid);
                                  setPickerError('');
                                  setManualPathMode(false);
                                  setActiveHarness(hid, true);
                                },
                                style: {
                                  width: '100%',
                                  textAlign: 'left',
                                  padding: '6px 9px',
                                  borderRadius: '10px',
                                  border: isBrowsedH
                                    ? '1px solid ' + meta.accentBorder
                                    : '1px solid transparent',
                                  background: isBrowsedH
                                    ? meta.accentBg
                                    : isActiveH
                                      ? panelRowBg
                                      : 'transparent',
                                  color: isBrowsedH || isActiveH ? panelFg : panelMuted,
                                  fontSize: '12px',
                                  fontWeight: isActiveH ? 650 : isBrowsedH ? 550 : 450,
                                  cursor: 'pointer',
                                  display: 'flex',
                                  alignItems: 'center',
                                  justifyContent: 'space-between',
                                  gap: '6px',
                                  opacity: isReady ? 1 : 0.52,
                                  transition: 'background 0.12s ease, border-color 0.12s ease',
                                },
                              },
                              h(
                                'span',
                                { style: { display: 'inline-flex', alignItems: 'center', gap: '7px', minWidth: 0, flex: 1 } },
                                h('span', {
                                  style: {
                                    width: '7px',
                                    height: '7px',
                                    borderRadius: '999px',
                                    background: isReady ? meta.accent : '#6b7280',
                                    boxShadow: isActiveH || isBrowsedH ? '0 0 7px ' + meta.accent : 'none',
                                    flexShrink: 0,
                                  },
                                }),
                                h(
                                  'span',
                                  { style: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' } },
                                  meta.label,
                                ),
                              ),
                              h(
                                'span',
                                { style: { opacity: isBrowsedH ? 0.9 : 0.5, fontSize: '11.5px', flexShrink: 0 } },
                                (isActiveH ? '● ' : '') + '›',
                              ),
                            );
                          }),
                    ),
                    h('div', { style: { height: '1px', margin: '4px 2px', background: panelBorder } }),
                    // Bottom pinned Settings button inside Level 1
                    h(
                      'button',
                      {
                        type: 'button',
                        'data-dsb-settings-btn': '1',
                        title: '插件与悬浮条设置',
                        onClick: () => {
                          refreshAppSettings();
                          setSettingsView(true);
                          if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
                            dsbRuntime.scheduleLayoutReport(240);
                          }
                        },
                        style: {
                          width: '100%',
                          textAlign: 'left',
                          padding: '5px 9px',
                          borderRadius: '9px',
                          border: 'none',
                          background: panelRowBg,
                          color: panelFg,
                          fontSize: '11.5px',
                          cursor: 'pointer',
                          display: 'flex',
                          alignItems: 'center',
                          justifyContent: 'space-between',
                          gap: '6px',
                        },
                      },
                      h(
                        'span',
                        { style: { display: 'inline-flex', alignItems: 'center', gap: '6px' } },
                        h('span', { style: { fontSize: '12.5px', opacity: 0.85 } }, '⚙'),
                        h('span', null, '设置'),
                      ),
                      h('span', { style: { fontSize: '10.5px', opacity: 0.45 } }, 'v' + (snap.version || '0.5.4')),
                    ),
                  ),

                  // ===========================================================
                  // Level 2 (Middle): Workspaces of hovered Harness (follows hovered Harness row)
                  // ===========================================================
                  h(
                    'div',
                    {
                      ref: wsPanelRef,
                      'data-dsb-popup-panel': '1',
                      'data-dsb-ws-col': '1',
                      style: {
                        width: col2W + 'px',
                        maxHeight: '300px',
                        marginBottom: wsMarginBottom + 'px',
                        borderRadius: '14px',
                        padding: '6px',
                        background: panelBg,
                        border: '1px solid ' + panelBorder,
                        boxShadow: '0 8px 24px rgba(0,0,0,0.35)',
                        display: 'flex',
                        flexDirection: 'column',
                        gap: '2px',
                        boxSizing: 'border-box',
                      },
                    },
                    manualPathMode
                      ? h(
                          'div',
                          { style: { padding: '4px', display: 'flex', flexDirection: 'column', gap: '4px' } },
                          h('input', {
                            type: 'text',
                            autoFocus: true,
                            placeholder: '输入工作区目录路径…',
                            value: manualPathValue,
                            onChange: (ev) => setManualPathValue(ev.target.value),
                            onKeyDown: (ev) => {
                              if (ev.key === 'Enter' && manualPathValue.trim()) {
                                createWorkspaceAtPath(manualPathValue.trim(), browsedHarness).catch((err) => {
                                  setPickerError(err instanceof Error ? err.message : String(err));
                                });
                              } else if (ev.key === 'Escape') {
                                ev.stopPropagation();
                                setManualPathMode(false);
                              }
                            },
                            style: {
                              width: '100%',
                              boxSizing: 'border-box',
                              padding: '4px 8px',
                              borderRadius: '8px',
                              fontSize: '12px',
                              background: 'rgba(128,128,128,0.14)',
                              border: '1px solid ' + browsedTheme.accentBorder,
                              color: 'inherit',
                              outline: 'none',
                            },
                          }),
                          h(
                            'div',
                            { style: { display: 'flex', gap: '4px', justifyContent: 'flex-end' } },
                            h(
                              'button',
                              {
                                type: 'button',
                                onClick: () => setManualPathMode(false),
                                style: { fontSize: '11.5px', padding: '2px 8px', borderRadius: '6px', background: 'transparent', color: 'inherit', border: '1px solid rgba(128,128,128,0.25)', cursor: 'pointer' },
                              },
                              '取消',
                            ),
                            h(
                              'button',
                              {
                                type: 'button',
                                onClick: () => {
                                  if (manualPathValue.trim()) {
                                    createWorkspaceAtPath(manualPathValue.trim(), browsedHarness).catch((err) => {
                                      setPickerError(err instanceof Error ? err.message : String(err));
                                    });
                                  }
                                },
                                style: { fontSize: '11.5px', padding: '2px 8px', borderRadius: '6px', background: browsedTheme.accentBg, color: browsedTheme.accent, border: '1px solid ' + browsedTheme.accentBorder, cursor: 'pointer' },
                              },
                              '确定',
                            ),
                          ),
                        )
                      : h(
                          'div',
                          { style: { display: 'flex', alignItems: 'center', gap: '4px' } },
                          h(
                            'button',
                            {
                              type: 'button',
                              onClick: handleCreateWorkspace,
                              style: {
                                flex: 1,
                                minWidth: 0,
                                textAlign: 'left',
                                padding: '6px 9px',
                                borderRadius: '10px',
                                fontSize: '12px',
                                fontWeight: 500,
                                color: browsedTheme.accent,
                                background: browsedTheme.accentBg,
                                border: 'none',
                                cursor: 'pointer',
                                display: 'flex',
                                alignItems: 'center',
                                gap: '5px',
                              },
                            },
                            h('span', null, '＋'),
                            h(
                              'span',
                              { style: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' } },
                              browsedMeta.newWorkspaceLabel,
                            ),
                          ),
                          !browsedMeta.useNativeComposer
                            ? h(
                                'button',
                                {
                                  type: 'button',
                                  title: '刷新 ' + browsedMeta.label + ' 工作区与历史会话',
                                  onClick: () => refreshExternalCatalog(browsedHarness, false),
                                  style: {
                                    width: '28px',
                                    height: '28px',
                                    borderRadius: '9px',
                                    border: 'none',
                                    background: panelRowBg,
                                    color: panelFg,
                                    fontSize: '13px',
                                    cursor: 'pointer',
                                    flexShrink: 0,
                                    display: 'inline-flex',
                                    alignItems: 'center',
                                    justifyContent: 'center',
                                    padding: 0,
                                  },
                                },
                                '↻',
                              )
                            : null,
                        ),
                    pickerError
                      ? h('div', { style: { fontSize: '11px', color: '#f87171', padding: '2px 8px' } }, pickerError)
                      : null,
                    h('div', { style: { height: '1px', margin: '4px 2px', background: panelBorder } }),
                    h(
                      'div',
                      {
                        'data-dsb-menu-scroll': '1',
                        onScroll: recomputeSubmenuAlignment,
                        style: { overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: '2px', maxHeight: '220px' },
                      },
                      !browsedMeta.useNativeComposer && dsbRuntime.externalLoading[browsedHarness] && groups.length === 0
                        ? h('div', { style: { padding: '12px 10px', fontSize: '12px', opacity: 0.65, textAlign: 'center' } }, '正在读取工作区…')
                        : groups.length === 0
                          ? h('div', { style: { padding: '12px 10px', fontSize: '12px', opacity: 0.65, textAlign: 'center' } }, '暂无工作区')
                          : groups.map((g) => {
                              const isHover = hoveredGroup && g.workspaceId === hoveredGroup.workspaceId;
                              const isActiveWs = browsedHarness === activeHarness && g.workspaceId === browsedActiveWsId;
                              return h(
                                'button',
                                {
                                  key: g.workspaceId,
                                  ref: (el) => {
                                    if (el) wsRowRefs.current.set(g.workspaceId, el);
                                    else wsRowRefs.current.delete(g.workspaceId);
                                  },
                                  type: 'button',
                                  onMouseEnter: () => setHoveredWsId(g.workspaceId),
                                  onClick: () => setHoveredWsId(g.workspaceId),
                                  style: {
                                    width: '100%',
                                    textAlign: 'left',
                                    padding: '6px 9px',
                                    borderRadius: '10px',
                                    fontSize: '12px',
                                    fontWeight: isActiveWs ? 600 : 400,
                                    background: isHover ? 'rgba(128,128,128,0.14)' : 'transparent',
                                    color: 'inherit',
                                    border: 'none',
                                    cursor: 'pointer',
                                    display: 'flex',
                                    alignItems: 'center',
                                    justifyContent: 'space-between',
                                    gap: '6px',
                                  },
                                },
                                h(
                                  'span',
                                  { style: { display: 'inline-flex', alignItems: 'center', gap: '6px', minWidth: 0, flex: 1 } },
                                  renderFolderIcon(14),
                                  h('span', { style: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' } }, g.label),
                                ),
                                h('span', { style: { opacity: 0.6, flexShrink: 0 } }, (isActiveWs ? '● ' : '') + '›'),
                              );
                            }),
                    ),
                  ),

                  // ===========================================================
                  // Level 3 (Right): Conversations in hovered Workspace
                  // ===========================================================
                  hoveredGroup
                    ? h(
                        'div',
                        {
                          ref: convPanelRef,
                          'data-dsb-popup-panel': '1',
                          'data-dsb-conv-col': '1',
                          style: {
                            width: col3W + 'px',
                            maxHeight: '300px',
                            marginBottom: submenuMarginBottom + 'px',
                            borderRadius: '14px',
                            padding: '6px',
                            background: panelBg,
                            border: '1px solid ' + panelBorder,
                            boxShadow: '0 8px 24px rgba(0,0,0,0.35)',
                            display: 'flex',
                            flexDirection: 'column',
                            gap: '2px',
                            boxSizing: 'border-box',
                          },
                        },
                        hoveredGroup.workspaceId !== '__ungrouped__'
                          ? h(
                              'button',
                              {
                                type: 'button',
                                onClick: () => handleNewConversation(hoveredGroup, browsedHarness),
                                style: {
                                  width: '100%',
                                  textAlign: 'left',
                                  padding: '6px 10px',
                                  borderRadius: '10px',
                                  fontSize: '12px',
                                  fontWeight: 500,
                                  color: browsedTheme.accent,
                                  background: browsedTheme.accentBg,
                                  border: 'none',
                                  cursor: 'pointer',
                                  display: 'flex',
                                  alignItems: 'center',
                                  gap: '6px',
                                },
                              },
                              h('span', null, '＋'),
                              h('span', null, '新对话'),
                            )
                          : null,
                        hoveredGroup.workspaceId !== '__ungrouped__'
                          ? h('div', { style: { height: '1px', margin: '4px 2px', background: panelBorder } })
                          : null,
                        h(
                          'div',
                          {
                            'data-dsb-menu-scroll': '1',
                            style: { overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: '2px', maxHeight: '220px' },
                          },
                          hoveredGroup.convs.length === 0
                            ? h('div', { style: { padding: '12px 10px', fontSize: '12px', opacity: 0.6, textAlign: 'center' } }, '暂无对话')
                            : hoveredGroup.convs.map((c) => {
                                const isSel = browsedHarness === activeHarness && c.id === browsedSessionId;
                                return h(
                                  'button',
                                  {
                                    key: c.id,
                                    type: 'button',
                                    onClick: () => handleOpenSession(c, hoveredGroup, browsedHarness),
                                    style: {
                                      width: '100%',
                                      textAlign: 'left',
                                      padding: '6px 10px',
                                      borderRadius: '10px',
                                      fontSize: '12px',
                                      background: isSel ? browsedTheme.accentBg : 'transparent',
                                      color: 'inherit',
                                      border: 'none',
                                      cursor: 'pointer',
                                      display: 'flex',
                                      alignItems: 'center',
                                      justifyContent: 'space-between',
                                      gap: '8px',
                                    },
                                  },
                                  h(
                                    'span',
                                    { style: { display: 'inline-flex', alignItems: 'center', gap: '6px', minWidth: 0, flex: 1 } },
                                    h('span', { style: { width: '12px', textAlign: 'center', color: browsedTheme.accent, flexShrink: 0 } }, isSel ? '✓' : ''),
                                    h('span', { style: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' } }, c.title),
                                  ),
                                  h('span', { style: { fontSize: '11.5px', opacity: 0.55, flexShrink: 0 } }, formatRelativeTime(c.updatedAt)),
                                );
                              }),
                        ),
                      )
                    : null,
                ),
          );

          if (ReactDOM && typeof ReactDOM.createPortal === 'function' && typeof document !== 'undefined' && document.body) {
            return ReactDOM.createPortal(menuNode, document.body);
          }
          return menuNode;
        };

        const activeAvail = availById[activeHarness] || null;
        const activeAdapter = getHarnessAdapter(activeHarness);

        return h(
          React.Fragment,
          null,
          activeAdapter && typeof activeAdapter.renderChipPrefix === 'function'
            ? activeAdapter.renderChipPrefix({ avail: activeAvail, manifest: activeMeta })
            : null,
          h(
            'button',
            {
              ref: btnRef,
              type: 'button',
              'data-dsb-session-chip': '1',
              title: '切换 Harness / 工作区 / 对话（当前：' + harnessTheme.fullLabel + '）',
              onClick: (e) => {
                e.stopPropagation();
                setMenuOpen((prev) => !prev);
              },
              style: {
                height: (pillMetrics && pillMetrics.height) || '28px',
                borderRadius: (pillMetrics && pillMetrics.borderRadius) || '9999px',
                fontSize: (pillMetrics && pillMetrics.fontSize) || '12px',
                background: activeMeta.useNativeComposer
                  ? (pillMetrics && pillMetrics.background) || 'var(--dsw-alias-bg-layer-3, rgba(128,128,128,0.12))'
                  : 'rgba(255, 255, 255, 0.05)',
                color: activeMeta.useNativeComposer
                  ? (pillMetrics && pillMetrics.color) || 'var(--dsw-alias-label-primary, inherit)'
                  : 'rgba(243, 244, 246, 0.90)',
                border: activeMeta.useNativeComposer
                  ? (pillMetrics && pillMetrics.border) || '1px solid var(--dsw-alias-border-l2, rgba(128,128,128,0.22))'
                  : '1px solid rgba(255, 255, 255, 0.11)',
                padding: '0 10px',
                display: 'inline-flex',
                alignItems: 'center',
                gap: '6px',
                maxWidth: '240px',
                cursor: 'pointer',
                flexShrink: 0,
                boxSizing: 'border-box',
              },
            },
            activeMeta.chipIconStyle === 'folder'
              ? renderFolderIcon(14)
              : h('span', {
                  style: {
                    width: '7px',
                    height: '7px',
                    borderRadius: '999px',
                    background: harnessTheme.accent,
                    boxShadow: '0 0 6px ' + harnessTheme.accent,
                    flexShrink: 0,
                  },
                }),
            h(
              'span',
              {
                style: {
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                },
              },
              activeTitle,
            ),
            h('span', { style: { opacity: 0.6, flexShrink: 0 } }, '▾'),
          ),
          renderMenu(),
        );
      };
    }

    function makeHarnessRightControls() {
      return function HarnessRightControls() {
        const [, setTick] = useState(0);
        useEffect(() => {
          const bump = () => setTick((n) => n + 1);
          dsbRuntime.listeners.add(bump);
          return () => dsbRuntime.listeners.delete(bump);
        }, []);

        const hId = dsbRuntime.activeHarness;
        const manifest = getHarnessMeta(hId);
        if (!IS_BAR || manifest.useNativeComposer) return null;
        const avail = (dsbRuntime.harnessAvailability || []).find((a) => a && a.id === hId) || null;
        const adapter = getHarnessAdapter(hId);
        if (adapter && typeof adapter.renderRightControls === 'function') {
          return adapter.renderRightControls({ avail, manifest });
        }
        return null;
      };
    }

    return {
      inject: ['slots', 'sessions', 'workspaces', 'uiWorkspace'],
      apply(ctx) {
        // In DSH's normal WebUI (not Bar), never inject UI, styles, or slots —
        // ONLY honor an explicit `#atp_session=<sid>` deep-link fragment when launched via "查看详情".
        if (!IS_BAR) {
          try {
            const rawHash =
              typeof window !== 'undefined' ? String(window.location.hash || '') : '';
            if (rawHash.startsWith('#atp_session=')) {
              const targetSid = decodeURIComponent(
                rawHash.slice('#atp_session='.length),
              ).trim();
              if (
                window.history &&
                typeof window.history.replaceState === 'function'
              ) {
                window.history.replaceState(
                  null,
                  '',
                  window.location.pathname + window.location.search,
                );
              }
              const sessions = ctx && (ctx.sessions || ctx.get?.('sessions'));
              const uiWorkspace = ctx && (ctx.uiWorkspace || ctx.get?.('uiWorkspace'));
              if (
                targetSid &&
                sessions &&
                sessions.list &&
                uiWorkspace &&
                typeof uiWorkspace.openSession === 'function'
              ) {
                let opened = false;
                let offSub = null;
                const tryOpenDeepLink = () => {
                  if (opened) return true;
                  const snap = sessions.list.getSnapshot();
                  if (!snap || snap.phase === 'pending') return false;
                  opened = true;
                  if (offSub) {
                    try {
                      offSub();
                    } catch {}
                    offSub = null;
                  }
                  setTimeout(() => {
                    try {
                      uiWorkspace.openSession(targetSid);
                    } catch {}
                    // Guard once in case DSH's own initial restore races right after boot
                    setTimeout(() => {
                      try {
                        const afterSnap = sessions.list.getSnapshot();
                        if (mainSessionIdOfList(afterSnap) !== targetSid) {
                          uiWorkspace.openSession(targetSid);
                        }
                      } catch {}
                    }, 480);
                  }, 80);
                  return true;
                };
                if (!tryOpenDeepLink()) {
                  if (typeof sessions.list.subscribe === 'function') {
                    offSub = sessions.list.subscribe(() => {
                      tryOpenDeepLink();
                    });
                  }
                  for (const ms of [350, 1000, 2200]) {
                    setTimeout(() => {
                      tryOpenDeepLink();
                    }, ms);
                  }
                }
              }
            }
          } catch {}
          return;
        }
        installGlobalBridge();
        ensureCardDockStyles();
        if (typeof document !== 'undefined') {
          document.documentElement.setAttribute('data-dsh-bar-clone', '1');
        }

        const slots = ctx && (ctx.slots || ctx.get?.('slots'));
        const sessions = ctx && (ctx.sessions || ctx.get?.('sessions'));
        const workspaces = ctx && (ctx.workspaces || ctx.get?.('workspaces'));
        const uiWorkspace = ctx && (ctx.uiWorkspace || ctx.get?.('uiWorkspace'));
        let directoryPicker = null;
        try {
          directoryPicker = ctx.get?.('remote.directoryPicker') || ctx.remote?.directoryPicker || null;
        } catch {}

        // In Bar mode, always ensure *this* WebView2 tab actually opens a session so
        // `conversation.input.dock` (CardholderSlipDock + ExternalComposer) mounts!
        // Note: `mainSessionIdOfList(snap)` reflects server-wide `retainedBy.mainView`, which can
        // be non-empty when another DSH browser tab is open even while this Bar tab is still on `/`.
        // Guard strictly against re-entrancy because `uiWorkspace.openSession` notifies `sessions.list`.
        if (IS_BAR && sessions && sessions.list && uiWorkspace) {
          let openInFlight = false;
          let openAttempts = 0;
          let offRestore = null;
          const attemptedIds = new Set();
          const getArchivedSet = () => {
            try {
              const wsSnap = workspaces && workspaces.list ? workspaces.list.getSnapshot() : null;
              return new Set(
                wsSnap && Array.isArray(wsSnap.archivedSessionIds) ? wsSnap.archivedSessionIds : [],
              );
            } catch {
              return new Set();
            }
          };
          const isValidCandidate = (snap, archivedSet, id) => {
            if (!id || attemptedIds.has(id) || archivedSet.has(id)) return false;
            const row = snap.byId && snap.byId[id];
            if (!row) return false;
            if (row.origin === 'subagent' || row.blank) return false;
            return true;
          };
          const pickBestSessionId = (snap) => {
            if (!snap || !snap.byId) return '';
            const archivedSet = getArchivedSet();
            const lastId = readStoredString(STORAGE_KEY_LAST_SESSION, '');
            if (isValidCandidate(snap, archivedSet, lastId)) {
              return lastId;
            }
            const currentId = mainSessionIdOfList(snap);
            if (isValidCandidate(snap, archivedSet, currentId)) {
              return currentId;
            }
            const ids = Array.isArray(snap.ids) ? snap.ids : Object.keys(snap.byId);
            for (const rawId of ids) {
              const id = String((snap.byId[rawId] && snap.byId[rawId].id) || rawId);
              if (isValidCandidate(snap, archivedSet, id)) {
                return id;
              }
            }
            for (const rawId of ids) {
              const id = String(rawId);
              if (!attemptedIds.has(id)) return id;
            }
            return '';
          };
          const tryRestore = (fromTimer) => {
            if (typeof document !== 'undefined' && document.querySelector('[data-dsb-stack="1"]')) {
              if (offRestore) {
                try { offRestore(); } catch {}
                offRestore = null;
              }
              return true;
            }
            if (openInFlight || openAttempts >= 4) return false;
            const snap = sessions.list.getSnapshot();
            if (!snap || snap.phase === 'pending') return false;
            const targetId = pickBestSessionId(snap);
            if (!targetId) return false;
            if (!fromTimer && openAttempts > 0) return false;
            openInFlight = true;
            openAttempts += 1;
            attemptedIds.add(targetId);
            dsbRuntime.sessionId = targetId;
            writeStoredString(STORAGE_KEY_LAST_SESSION, targetId);
            setTimeout(() => {
              try {
                if (!document.querySelector('[data-dsb-stack="1"]')) {
                  sendBarReport({
                    kind: 'diag',
                    detail: 'tryRestore opening session=' + targetId + ' attempt=' + openAttempts + ' path=' + window.location.pathname,
                  });
                  uiWorkspace.openSession(targetId);
                }
              } catch (e) {
                sendBarReport({ kind: 'diag', detail: 'tryRestore openSession err=' + String(e) });
              }
              setTimeout(() => {
                openInFlight = false;
              }, 350);
            }, 0);
            return false;
          };
          tryRestore(false);
          for (const ms of [450, 1300, 2500, 4000]) {
            setTimeout(() => {
              tryRestore(true);
            }, ms);
          }
          if (typeof sessions.list.subscribe === 'function') {
            offRestore = sessions.list.subscribe(() => {
              tryRestore(false);
            });
          }
        }

        if (!slots) return;
        const services = { sessions, workspaces, uiWorkspace, directoryPicker };
        const CardholderSlipDock = makeCardDockComponent(services);
        const ConversationPickerChip = makeConversationPickerChip(services);
        const HarnessRightControls = makeHarnessRightControls();

        // Capture unhandled errors & CardDock mount/unmount lifecycle in Bar mode for instant diagnosis
        const reportDiag = (msg) => {
          sendBarReport({ kind: 'diag', detail: String(msg || '') });
        };
        if (typeof window !== 'undefined' && !window.__DSB_ERR_HOOKED__) {
          window.__DSB_ERR_HOOKED__ = true;
          window.addEventListener('error', (ev) => {
            const msg = String((ev && (ev.message || (ev.error && ev.error.stack))) || 'unknown');
            if (msg.includes('ResizeObserver loop')) return;
            reportDiag('WIN_ERR: ' + msg);
          });
          window.addEventListener('unhandledrejection', (ev) => {
            const r = ev && ev.reason;
            reportDiag('REJ_ERR: ' + (r && r.stack ? r.stack.slice(0, 260) : String(r)));
          });
        }

        const SafeCardholderSlipDock = (() => {
          if (typeof React.Component !== 'function') return CardholderSlipDock;
          class CardDockBoundary extends React.Component {
            constructor(props) {
              super(props);
              this.state = { err: null };
            }
            static getDerivedStateFromError(error) {
              return { err: error };
            }
            componentDidCatch(error) {
              reportDiag(
                'CARD_BOUNDARY: ' +
                  (error && error.stack ? error.stack.slice(0, 320) : String(error)),
              );
            }
            render() {
              if (this.state.err) return null;
              return h(CardholderSlipDock, this.props);
            }
          }
          return CardDockBoundary;
        })();

        slots.inject('conversation.input.dock', () =>
          slots.register(
            {
              name: 'conversation.input.dock',
              id: 'cardholder-slip',
              order: 6,
            },
            SafeCardholderSlipDock,
          ),
        );

        slots.inject('conversation.input.left', () =>
          slots.register(
            {
              name: 'conversation.input.left',
              id: 'dsb-session-picker',
              order: 100,
            },
            ConversationPickerChip,
          ),
        );

        slots.inject('conversation.input.right', () =>
          slots.register(
            {
              name: 'conversation.input.right',
              id: 'dsb-harness-right',
              order: 10,
            },
            HarnessRightControls,
          ),
        );
      },
    };
  },
});
