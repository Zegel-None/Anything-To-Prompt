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

    const CLAUDE_PERM_MODES = [
      { id: 'auto', label: '自动', title: 'Claude auto 模式（默认：智能自动执行并拦截高危操作）' },
      { id: 'acceptEdits', label: '自动编辑', title: 'acceptEdits（自动允许文件编辑）' },
      { id: 'plan', label: '仅规划', title: 'plan（只读分析与规划，不修改文件）' },
      { id: 'dontAsk', label: '免询问', title: 'dontAsk（免交互确认提示）' },
    ];

    const HARNESS_ORDER = ['dsh', 'antigravity', 'codex', 'claude_code'];
    const HARNESS_META = {
      dsh: {
        id: 'dsh',
        label: 'DeepSeek',
        fullLabel: 'DeepSeek Harness',
        badge: 'DS',
        syncHint: '双向实时同步',
        accent: '#60a5fa',
        accentBg: 'rgba(59, 130, 246, 0.16)',
        accentBorder: 'rgba(96, 165, 250, 0.42)',
        surface: 'var(--dsw-alias-bg-layer-2, #27272a)',
        border: 'var(--dsw-alias-border-l2, rgba(128,128,128,0.22))',
      },
      antigravity: {
        id: 'antigravity',
        label: 'Antigravity',
        fullLabel: 'Antigravity',
        badge: 'AG',
        syncHint: 'CLI Project 原生同步',
        placeholder: '提问任何问题，使用 @ 提及，/ 执行操作',
        accent: '#38bdf8',
        accentBg: 'rgba(56, 189, 248, 0.12)',
        accentBorder: 'rgba(255, 255, 255, 0.14)',
        surface: '#16171a',
        border: 'rgba(255, 255, 255, 0.11)',
      },
      codex: {
        id: 'codex',
        label: 'Codex',
        fullLabel: 'Codex',
        badge: 'CX',
        syncHint: 'app-server 共享历史',
        placeholder: '随心输入',
        accent: '#3b82f6',
        accentBg: 'rgba(59, 130, 246, 0.16)',
        accentBorder: 'rgba(255, 255, 255, 0.14)',
        surface: '#262628',
        border: 'rgba(255, 255, 255, 0.13)',
      },
      claude_code: {
        id: 'claude_code',
        label: 'Claude',
        fullLabel: 'Claude Code',
        badge: 'CC',
        syncHint: 'CLI 同库共享历史',
        placeholder: '今天想处理什么？支持按 Ctrl 圈选屏幕元素…',
        accent: '#D97757',
        accentBg: 'rgba(217, 119, 87, 0.17)',
        accentBorder: 'rgba(217, 119, 87, 0.46)',
        surface: '#2B2B29',
        border: 'rgba(255, 248, 240, 0.12)',
      },
    };

    function normalizeHarnessId(raw) {
      const s = String(raw || '').trim().toLowerCase();
      if (s === 'antigravity' || s === 'agy') return 'antigravity';
      if (s === 'codex') return 'codex';
      if (s === 'claude_code' || s === 'claude-code' || s === 'claudecode' || s === 'claude') return 'claude_code';
      return 'dsh';
    }

    function tauriInvoke(cmd, args) {
      if (typeof window === 'undefined') return Promise.reject(new Error('no window'));
      const core = window.__TAURI__ && window.__TAURI__.core;
      if (!core || typeof core.invoke !== 'function') {
        return Promise.reject(new Error('no tauri core'));
      }
      return core.invoke(cmd, args || {});
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

    // Shared runtime handles between Dock, Picker Chip, and Tauri Bridge
    const dsbRuntime = {
      barMode: INITIAL_BAR_MODE,
      previewExpanded: false,
      barWidth: readStoredNumber(STORAGE_KEY_BAR_WIDTH, DEFAULT_BAR_WIDTH_PX, MIN_BAR_WIDTH_PX, MAX_BAR_WIDTH_PX),
      modalOpen: false,
      popupOpen: false,
      quickSearchOpen: false,
      quickSearchSelectedIdx: -1,
      hovered: false,
      pinned: readStoredBool(STORAGE_KEY_PINNED, false),
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
      // Multi-harness state (DSH + Antigravity + Codex + Claude Code)
      activeHarness: normalizeHarnessId(readStoredString(STORAGE_KEY_ACTIVE_HARNESS, 'dsh')),
      harnessAvailability: [],
      externalCatalogs: { antigravity: null, codex: null, claude_code: null },
      externalSelection: {
        antigravity: { workspaceId: 'default-cli-project', workspacePath: '', sessionId: '', title: 'CLI Project · 新对话' },
        codex: { workspaceId: '', workspacePath: '', sessionId: '', title: 'Codex · 新对话' },
        claude_code: { workspaceId: '', workspacePath: '', sessionId: '', title: 'Claude Code · 新对话' },
      },
      externalMessages: { antigravity: [], codex: [], claude_code: [] },
      externalActiveTurn: null,
      externalLoading: { antigravity: false, codex: false, claude_code: false },
      liveSyncMeta: { antigravity: null, codex: null, claude_code: null },
      lastSyncCatalogRefresh: { antigravity: 0, codex: 0, claude_code: 0 },
      appSettings: null,
      claudePermissionMode: readStoredString(STORAGE_KEY_CLAUDE_PERM, 'auto'),
      extDraftText: '',
      extCapsules: [],
      toggleHarnessMenu: null,
      openSettingsMenu: null,
    };

    function applyHarnessThemeAttribute(harnessId) {
      const norm = normalizeHarnessId(harnessId);
      if (typeof document !== 'undefined' && document.documentElement) {
        document.documentElement.setAttribute('data-dsb-harness', norm);
      }
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
              }
            } else if (autoSelectIfEmpty && groups.length > 0) {
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

    function setActiveHarness(harnessId, persist = true) {
      const norm = normalizeHarnessId(harnessId);
      dsbRuntime.activeHarness = norm;
      applyHarnessThemeAttribute(norm);
      if (persist) {
        writeStoredString(STORAGE_KEY_ACTIVE_HARNESS, norm);
        tauriInvoke('harness_set_active', { harness: norm }).catch(() => {});
      }
      if (norm !== 'dsh') {
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

    function submitExternalHarnessPrompt(rawText) {
      const hId = dsbRuntime.activeHarness;
      if (hId === 'dsh') return false;
      const prompt = String(rawText || '').trim();
      if (!prompt) return false;
      if (dsbRuntime.externalActiveTurn && dsbRuntime.externalActiveTurn.harness === hId) {
        cancelActiveExternalTurn();
      }
      const turnId = 'turn-' + Date.now() + '-' + Math.random().toString(36).slice(2, 7);
      const userMsgId = 'ext-u-' + turnId;
      const asstMsgId = 'ext-a-' + turnId;
      const sel = dsbRuntime.externalSelection[hId] || {};
      const prevList = Array.isArray(dsbRuntime.externalMessages[hId]) ? dsbRuntime.externalMessages[hId] : [];
      dsbRuntime.externalMessages[hId] = prevList.concat([
        { id: userMsgId, role: 'user', text: prompt, streaming: false },
        { id: asstMsgId, role: 'assistant', text: '', streaming: true },
      ]);
      dsbRuntime.externalActiveTurn = {
        turnId,
        harness: hId,
        assistantMsgId: asstMsgId,
        toolNote: '',
      };
      if (!sel.sessionId) {
        const snippet = prompt.replace(/\s+/g, ' ').slice(0, 22);
        dsbRuntime.externalSelection[hId] = {
          ...sel,
          title: snippet + (prompt.length > 22 ? '…' : ''),
        };
      }
      notifyBarRuntime();
      sendBarReport({ kind: 'submitted', sessionId: sel.sessionId || turnId });

      tauriInvoke('harness_send_message', {
        payload: {
          harness: hId,
          turnId,
          cwd: sel.workspacePath || null,
          projectId: hId === 'antigravity' ? (sel.workspaceId || 'default-cli-project') : null,
          sessionId: sel.sessionId || null,
          permissionMode: hId === 'claude_code' ? (dsbRuntime.claudePermissionMode || 'auto') : null,
          prompt,
        },
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
      const shouldUpdateMessages =
        !curSel.sessionId ||
        !incomingSessionId ||
        curSel.sessionId === incomingSessionId ||
        evt.source === 'hook' ||
        evt.source === 'watch';

      if (shouldUpdateMessages) {
        if (incomingSessionId) {
          const catHit = findExternalCatalogEntry(hId, incomingSessionId);
          const sameSession = curSel.sessionId === incomingSessionId;
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
          dsbRuntime.externalSelection[hId] = {
            workspaceId: resolvedWsId,
            workspacePath: resolvedWsPath,
            sessionId: incomingSessionId,
            title: resolvedTitle,
          };
        }
        if (Array.isArray(evt.messages) && evt.messages.length > 0) {
          dsbRuntime.externalMessages[hId] = evt.messages;
        }
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
    ]);

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
      const normalized = mode === 'lip' ? 'lip' : 'summon';
      const wantExpanded = mode === 'preview' || mode === 'expanded';
      dsbRuntime.barMode = normalized;
      dsbRuntime.previewExpanded = wantExpanded;
      // Always invalidate cached layout dedupe key on mode switches (e.g. Hidden -> Summon)
      // so the physical Win32 window never stays stuck at 56px Lip height.
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
      if (!document.documentElement.getAttribute('data-dsb-mode')) {
        document.documentElement.setAttribute('data-dsb-mode', dsbRuntime.barMode);
      }
      applyHarnessThemeAttribute(dsbRuntime.activeHarness);
      window.__DSB_HARNESS_EVENT__ = handleIncomingHarnessEvent;
      window.__DSB_HARNESS_SYNC__ = handleIncomingHarnessSync;
      window.__DSB_BRIDGE__ = {
        setMode(mode) {
          setBarMode(mode);
          return dsbRuntime.barMode;
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
        '[data-composer-card] {',
        '  position: relative !important;',
        '  z-index: 2 !important;',
        '  border-radius: 0 0 24px 24px !important;',
        '  box-sizing: border-box !important;',
        '  background: var(--dsw-alias-bg-layer-2, #2c2c2e) !important;',
        '  box-shadow: none !important;',
        '  border: none !important;',
        '}',
        '[data-dsb-card] {',
        '  box-shadow: 0 -12px 30px rgba(0, 0, 0, 0.34) !important;',
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
        '[data-queue-dock] {',
        '  padding: 0 !important;',
        '}',
        '[data-queue-dock] > div {',
        '  border-radius: 0 !important;',
        '  padding: 0 !important;',
        '}',
        '[data-queue-dock] > div::before {',
        '  background: var(--dsb-surface, var(--dsw-alias-bg-layer-2)) !important;',
        '  backdrop-filter: none !important;',
        '}',
        '[data-queue-dock] > div::after {',
        '  border: none !important;',
        '  border-bottom: 1px solid var(--dsb-divider-line, rgba(0, 0, 0, 0.32)) !important;',
        '  box-shadow: inset 0 -2px 3px var(--dsb-divider-glow, rgba(0, 0, 0, 0.10)) !important;',
        '}',
        '[data-queue-dock] button {',
        '  border-radius: 0 !important;',
        '  color: var(--dsb-text, var(--dsw-alias-label-primary)) !important;',
        '  font-weight: var(--dsb-weight, 400) !important;',
        '  font-size: var(--dsb-fontsize, 13px) !important;',
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
        // ?dsh_bar_clone=1 — Bar Mode (transparent shell + zero-shadow clip)
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
        'html[data-dsh-bar-clone="1"] body *:not(:has([data-composer-seat])):not([data-composer-seat]):not([data-composer-seat] *):not([data-dsb-popup]):not([data-dsb-popup] *):not([role="menu"]):not([role="menu"] *):not([role="listbox"]):not([role="listbox"] *):not([role="dialog"]):not([role="dialog"] *):not([data-radix-popper-content-wrapper]):not([data-radix-popper-content-wrapper] *):not([data-floating-ui-portal]):not([data-floating-ui-portal] *) {',
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
        'html[data-dsh-bar-clone="1"] [data-composer-seat] *:has([data-composer-card]) > *:not([data-composer-card]):not(:has([data-composer-card])):not([data-donegate-dock-wrap]):not(:has([data-donegate-dock-wrap])):not([data-dsb-external-composer]):not(:has([data-dsb-external-composer])):not([data-dsb-stack]):not(:has([data-dsb-stack])):not([data-queue-dock]):not(:has([data-queue-dock])):not([data-slot="conversation.input.dock"]):not(:has([data-slot="conversation.input.dock"])) {',
        '  display: none !important;',
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
        'html[data-dsh-bar-clone="1"] [data-composer-card] ~ * {',
        '  display: none !important;',
        '}',
        'html[data-dsh-bar-clone="1"][data-dsb-mode="lip"] [data-composer-card],',
        'html[data-dsh-bar-clone="1"][data-dsb-mode="lip"] [data-queue-dock] {',
        '  display: none !important;',
        '}',
        'html[data-dsh-bar-clone="1"] [data-dsb-menu-scroll]::-webkit-scrollbar { width: 4px; }',
        'html[data-dsh-bar-clone="1"] [data-dsb-menu-scroll]::-webkit-scrollbar-thumb { background: rgba(128,128,128,0.28); border-radius: 999px; }',
        // =====================================================================
        // Multi-Harness 1:1 Native Composer Replicas (Antigravity, Codex & Claude Code)
        // When targeting an external CLI, hide DSH's native [data-composer-card]
        // and display the 1:1 replicated desktop composer ([data-dsb-external-composer]).
        // =====================================================================
        'html[data-dsh-bar-clone="1"][data-dsb-harness="antigravity"]:has([data-dsb-external-composer]) [data-composer-card],',
        'html[data-dsh-bar-clone="1"][data-dsb-harness="codex"]:has([data-dsb-external-composer]) [data-composer-card],',
        'html[data-dsh-bar-clone="1"][data-dsb-harness="claude_code"]:has([data-dsb-external-composer]) [data-composer-card] {',
        '  display: none !important;',
        '}',
        'html[data-dsh-bar-clone="1"][data-dsb-mode="lip"] [data-dsb-external-composer] {',
        '  display: none !important;',
        '}',
        'html[data-dsh-bar-clone="1"][data-dsb-harness="antigravity"] [data-dsb-card] {',
        '  background: #121213 !important;',
        '  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.06) inset !important;',
        '  border-radius: 16px 16px 0 0 !important;',
        '  color: #eceef2 !important;',
        '}',
        'html[data-dsh-bar-clone="1"][data-dsb-harness="antigravity"][data-dsb-mode="lip"] [data-dsb-card] {',
        '  border-radius: 16px !important;',
        '}',
        'html[data-dsh-bar-clone="1"][data-dsb-harness="antigravity"] [data-dsb-divider] {',
        '  display: none !important;',
        '}',
        'html[data-dsh-bar-clone="1"][data-dsb-harness="codex"] [data-dsb-card] {',
        '  background: #1f1f1f !important;',
        '  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.06) inset !important;',
        '  border-radius: 14px 14px 0 0 !important;',
        '  width: calc(100% - 28px) !important;',
        '  margin: 0 14px !important;',
        '  color: #f4f4f5 !important;',
        '}',
        'html[data-dsh-bar-clone="1"][data-dsb-harness="codex"][data-dsb-mode="lip"] [data-dsb-card] {',
        '  border-radius: 14px !important;',
        '}',
        'html[data-dsh-bar-clone="1"][data-dsb-harness="codex"] [data-dsb-divider] {',
        '  display: none !important;',
        '}',
        'html[data-dsh-bar-clone="1"][data-dsb-harness="claude_code"] [data-dsb-card] {',
        '  background: #242422 !important;',
        '  box-shadow: 0 0 0 1px rgba(255, 248, 240, 0.08) inset !important;',
        '  border-radius: 18px 18px 0 0 !important;',
        '  color: #EDEBE6 !important;',
        '}',
        'html[data-dsh-bar-clone="1"][data-dsb-harness="claude_code"][data-dsb-mode="lip"] [data-dsb-card] {',
        '  border-radius: 18px !important;',
        '}',
        'html[data-dsh-bar-clone="1"][data-dsb-harness="claude_code"] [data-dsb-divider] {',
        '  display: none !important;',
        '}',
        '[data-dsb-ext-input="1"]::placeholder {',
        '  color: rgba(255, 255, 255, 0.36);',
        '}',
        '[data-dsb-external-composer="claude_code"] [data-dsb-ext-input="1"]::placeholder {',
        '  color: rgba(237, 235, 230, 0.42);',
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
        const [pinned, setPinned] = useState(() => readStoredBool(STORAGE_KEY_PINNED, false));
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

        // Keep Rust pick counter synced 1:1 with the actual number of capsules currently in the composer
        useEffect(() => {
          if (!IS_BAR || typeof document === 'undefined') return undefined;
          let rafId = 0;
          const syncCapsuleCount = () => {
            if (rafId) cancelAnimationFrame(rafId);
            rafId = requestAnimationFrame(() => {
              rafId = 0;
              sendBarReport({
                kind: 'capsules',
                capsuleCount: countComposerCapsules(dsbRuntime.liveDraft),
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
        }, [liveDraft, sessionId]);

        useEffect(() => {
          dsbRuntime.setModeListener = (nextMode, wantExpanded = false) => {
            setBarModeState(nextMode);
            setPreviewExpanded(Boolean(wantExpanded));
            if (nextMode === 'lip') {
              setHovered(false);
              setPinned((prev) => {
                if (prev) {
                  try { localStorage.setItem(STORAGE_KEY_PINNED, '0'); } catch {}
                }
                return false;
              });
              if (typeof dsbRuntime.setPopupCloseListener === 'function') {
                dsbRuntime.setPopupCloseListener();
              }
            }
          };
          return () => {
            dsbRuntime.setModeListener = null;
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

        const isExpanded = !dsbRuntime.popupOpen && (pinned || hovered || previewExpanded);

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

        // Activity & explicit-intent message-submitted report
        useEffect(() => {
          if (!IS_BAR) return;
          sendBarReport({
            kind: 'activity',
            sessionId: effectiveSessionId,
            latestAssistantId: latestAssistant ? String(latestAssistant.id) : '',
            streaming: Boolean(latestAssistant && latestAssistant.streaming),
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
        }, [activeHarness, effectiveSessionId, sessionId, latestAssistant ? latestAssistant.id : '', Boolean(latestAssistant && latestAssistant.streaming), userMessageCount]);

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
              document.querySelector('[role="menu"], [role="listbox"], [role="dialog"], [data-radix-popper-content-wrapper], [data-floating-ui-portal]'),
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

        // Measure and report Bar layout + shapes to Tauri
        const measureAndReportLayout = useCallback(() => {
          if (!IS_BAR || typeof document === 'undefined') return;
          const cardEl = document.querySelector('[data-dsb-card]');
          const composerEl =
            barMode === 'lip'
              ? null
              : dsbRuntime.activeHarness !== 'dsh'
                ? document.querySelector('[data-dsb-external-composer]')
                : document.querySelector('[data-composer-card]');
          const cr = cardEl ? cardEl.getBoundingClientRect() : null;
          const ir = composerEl ? composerEl.getBoundingClientRect() : null;

          let slabLeft = 0;
          let slabTop = 0;
          let slabRight = barWidth;
          let slabBottom = window.innerHeight;
          let hasSlab = false;

          if (cr && cr.width > 10 && cr.height > 10) {
            slabLeft = cr.left;
            slabTop = cr.top;
            slabRight = cr.right;
            slabBottom = cr.bottom;
            hasSlab = true;
          }
          if (ir && ir.width > 10 && ir.height > 10) {
            if (hasSlab) {
              slabLeft = Math.min(slabLeft, ir.left);
              slabTop = Math.min(slabTop, ir.top);
              slabRight = Math.max(slabRight, ir.right);
              slabBottom = Math.max(slabBottom, ir.bottom);
            } else {
              slabLeft = ir.left;
              slabTop = ir.top;
              slabRight = ir.right;
              slabBottom = ir.bottom;
              hasSlab = true;
            }
          }

          const shapes = [];
          const cornerRadius =
            dsbRuntime.activeHarness === 'antigravity'
              ? 16
              : dsbRuntime.activeHarness === 'codex'
                ? 20
                : dsbRuntime.activeHarness === 'claude_code'
                  ? 18
                  : CARD_RADIUS_PX;
          if (hasSlab) {
            shapes.push({
              x: Math.round(slabLeft),
              y: Math.round(slabTop),
              w: Math.round(slabRight - slabLeft),
              h: Math.round(slabBottom - slabTop),
              r: cornerRadius,
            });
          }

          const cardTarget = isExpanded ? expandedHeight : COLLAPSED_LIP_PX;
          const queueEl = barMode === 'lip' ? null : document.querySelector('[data-queue-dock]');
          const qr = queueEl ? queueEl.getBoundingClientRect() : null;
          const hasQueue = !!(qr && qr.height > 1);
          let targetHeight = cardTarget;
          if (ir && ir.height > 0 && barMode !== 'lip') {
            const anchorTop = hasQueue ? Math.min(ir.top, qr.top) : ir.top;
            const composerSpan = Math.max(ir.height, ir.bottom - anchorTop);
            targetHeight = cardTarget + Math.ceil(composerSpan);
          }

          const popupEls = document.querySelectorAll(
            '[data-dsb-popup], [role="menu"], [role="listbox"], [role="dialog"], [data-radix-popper-content-wrapper], [data-floating-ui-portal]',
          );
          for (const el of popupEls) {
            if (!(el instanceof HTMLElement)) continue;
            const cs = getComputedStyle(el);
            if (cs.display === 'none' || cs.visibility === 'hidden') continue;
            const pr = el.getBoundingClientRect();
            if (pr.width < 12 || pr.height < 12) continue;
            shapes.push({
              x: Math.round(pr.left),
              y: Math.round(pr.top),
              w: Math.round(pr.width),
              h: Math.round(pr.height),
              r: 12,
            });
            const explicitBottom = Number(el.getAttribute('data-dsb-popup-bottom'));
            const neededForPopup =
              Number.isFinite(explicitBottom) && explicitBottom > 0
                ? Math.ceil(explicitBottom + pr.height + 8)
                : Math.ceil(window.innerHeight - pr.top + 6);
            if (neededForPopup > targetHeight) {
              targetHeight = neededForPopup;
            }
          }

          targetHeight = Math.max(COLLAPSED_LIP_PX, Math.min(880, Math.ceil(targetHeight)));
          // Because the Tauri window is bottom-anchored, any pending vertical resize shifts
          // every bottom-anchored element's viewport `top` by `deltaY = targetHeight - innerHeight`.
          // Pre-adjusting `shape.y` keeps Win32 SetWindowRgn pixel-exact even on Frame 1!
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
        }, [barMode, barWidth, isExpanded, expandedHeight, activeHarness]);

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

          const syncQueueDock = (rect, cs) => {
            const queueDock = document.querySelector('[data-queue-dock]');
            const qr = queueDock ? queueDock.getBoundingClientRect() : null;
            const hasQueue = !!(qr && qr.height > 1);
            if (hasQueue && queueDock) {
              const panel = queueDock.firstElementChild;
              if (panel) panel.style.borderRadius = '0px';
              queueDock.style.width = rect.width + 'px';
              queueDock.style.maxWidth = rect.width + 'px';
              queueDock.style.transform = 'none';
              const qrr = queueDock.getBoundingClientRect();
              const qdx = Math.round((rect.left - qrr.left) * 100) / 100;
              if (Math.abs(qdx) > 0.01) queueDock.style.transform = 'translateX(' + qdx + 'px)';
              queueDock.style.setProperty('--dsb-surface', cs.backgroundColor);
              queueDock.style.setProperty('--dsb-text', cs.color);
              queueDock.style.setProperty('--dsb-weight', cs.fontWeight);
              queueDock.style.setProperty('--dsb-fontsize', cs.fontSize);
              let isDark = true;
              const m = String(cs.backgroundColor || '').match(/rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/i);
              if (m) {
                isDark = (0.299 * Number(m[1]) + 0.587 * Number(m[2]) + 0.114 * Number(m[3])) / 255 < 0.5;
              }
              queueDock.style.setProperty('--dsb-divider-line', isDark ? 'rgba(0,0,0,0.32)' : 'rgba(0,0,0,0.075)');
              queueDock.style.setProperty('--dsb-divider-glow', isDark ? 'rgba(0,0,0,0.10)' : 'rgba(0,0,0,0.025)');
            }
            return hasQueue ? qr : null;
          };

          const syncWidth = () => {
            const composerCard = findComposer();
            const wrap = wrapRef.current;
            if (!wrap) return;
            const cardEl = wrap.querySelector('[data-dsb-card]');

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

                if (barMode !== 'lip') {
                  const rect = composerCard.getBoundingClientRect();
                  const qr = syncQueueDock(rect, cs);
                  const anchorTop = qr ? Math.min(rect.top, qr.top) : rect.top;
                  const wr = wrap.getBoundingClientRect();
                  const gap = Math.round((anchorTop - wr.bottom) * 100) / 100;
                  cardEl.style.bottom = (-gap) + 'px';
                } else {
                  cardEl.style.bottom = '0px';
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

              const qr = syncQueueDock(rect, cs);
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

        const renderPinButton = () =>
          h(
            'button',
            {
              type: 'button',
              onClick: togglePin,
              'aria-label': pinned ? '取消固定' : '固定展开',
              'aria-pressed': pinned ? 'true' : 'false',
              title: pinned ? '取消固定（鼠标移出自动收起）' : '固定展开聊天窗',
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
          if (!IS_BAR || barMode === 'lip' || dsbRuntime.popupOpen) {
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
        }, [effectiveQuickDraft, effectiveQuickHasCapsules, barMode, dsbRuntime.popupOpen]);

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
          IS_BAR && barMode === 'lip'
            ? CARD_RADIUS_PX + 'px'
            : CARD_RADIUS_PX + 'px ' + CARD_RADIUS_PX + 'px 0 0';

        const extCapsules = dsbRuntime.extCapsules || [];
        const extDraft = dsbRuntime.extDraftText || '';
        const activeTurn = dsbRuntime.externalActiveTurn;
        const isExtStreaming = Boolean(activeTurn && activeTurn.harness === activeHarness);
        const activeAvail =
          (dsbRuntime.harnessAvailability || []).find((a) => a && a.id === activeHarness) || null;
        const extSel = dsbRuntime.externalSelection[activeHarness] || {};

        const submitExternalComposer = () => {
          if (isExtStreaming) {
            cancelActiveExternalTurn();
            return;
          }
          const parts = [];
          for (const c of extCapsules) {
            if (c && c.text) parts.push(c.text);
          }
          if (extDraft.trim()) parts.push(extDraft.trim());
          const fullText = parts.join('\n').trim();
          if (!fullText) return;
          dsbRuntime.extDraftText = '';
          dsbRuntime.extCapsules = [];
          sendBarReport({ kind: 'capsules', capsuleCount: 0, afterInsert: false });
          notifyBarRuntime();
          submitExternalHarnessPrompt(fullText);
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

        const handleExtInputKeyDown = (ev) => {
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
          dsbRuntime.extDraftText = ev.target.value;
          notifyBarRuntime();
          try {
            ev.target.style.height = 'auto';
            ev.target.style.height = Math.min(140, Math.max(24, ev.target.scrollHeight)) + 'px';
          } catch {}
          if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
            dsbRuntime.scheduleLayoutReport(200);
          }
        };

        const renderPlusSvg = (sz = 16, col = '#a1a1aa') =>
          h(
            'svg',
            { width: sz, height: sz, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 2, strokeLinecap: 'round' },
            h('path', { d: 'M12 5v14M5 12h14' }),
          );

        const renderChevronDownSvg = (col = 'rgba(255,255,255,0.45)') =>
          h(
            'svg',
            { width: 11, height: 11, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 2.2, strokeLinecap: 'round', strokeLinejoin: 'round' },
            h('path', { d: 'M6 9l6 6 6-6' }),
          );

        const renderShieldSvg = (col = '#a1a1aa') =>
          h(
            'svg',
            { width: 13, height: 13, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 1.9, strokeLinecap: 'round', strokeLinejoin: 'round' },
            h('path', { d: 'M12 3l7 3v5c0 5-3.2 8.8-7 10-3.8-1.2-7-5-7-10V6l7-3z' }),
            h('path', { d: 'M12 8v4M12 16h.01' }),
          );

        const renderLaptopSvg = (col = '#d4d4d8') =>
          h(
            'svg',
            { width: 14, height: 14, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 1.8, strokeLinecap: 'round', strokeLinejoin: 'round' },
            h('rect', { x: 4, y: 5, width: 16, height: 11, rx: 1.8 }),
            h('path', { d: 'M2 19h20' }),
          );

        const renderFolderOutlineSvg = (col = '#d4d4d8') =>
          h(
            'svg',
            { width: 14, height: 14, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 1.8, strokeLinecap: 'round', strokeLinejoin: 'round' },
            h('path', { d: 'M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z' }),
          );

        const renderArrowUpSvg = (col = '#111214') =>
          h(
            'svg',
            { width: 14, height: 14, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 2.3, strokeLinecap: 'round', strokeLinejoin: 'round' },
            h('path', { d: 'M12 19V5M6 11l6-6 6 6' }),
          );

        const renderClaudeSparkSvg = (sz = 14, col = '#D97757') =>
          h(
            'svg',
            { width: sz, height: sz, viewBox: '0 0 24 24', fill: 'none', stroke: col, strokeWidth: 2, strokeLinecap: 'round' },
            h('path', { d: 'M12 2.5v19M2.5 12h19M5.2 5.2l13.6 13.6M18.8 5.2L5.2 18.8' }),
            h('circle', { cx: 12, cy: 12, r: 2.2, fill: col, stroke: 'none' }),
          );

        const renderExternalNativeComposer = () => {
          if (!IS_BAR || barMode === 'lip' || activeHarness === 'dsh') return null;

          const hasPayload = Boolean(extDraft.trim() || extCapsules.length > 0);
          const openSwitcher = (ev) => {
            ev.stopPropagation();
            if (typeof dsbRuntime.toggleHarnessMenu === 'function') {
              dsbRuntime.toggleHarnessMenu(ev.currentTarget);
            }
          };

          if (activeHarness === 'claude_code') {
            const claudeModel = (activeAvail && activeAvail.modelLabel) || 'Claude Opus';
            const claudeEffort = (activeAvail && activeAvail.effortLabel) || '高';
            const currentPermId = dsbRuntime.claudePermissionMode || 'auto';
            const currentPerm =
              CLAUDE_PERM_MODES.find((m) => m.id === currentPermId) || CLAUDE_PERM_MODES[0];
            const cycleClaudePerm = (ev) => {
              ev.stopPropagation();
              const idx = CLAUDE_PERM_MODES.findIndex((m) => m.id === currentPerm.id);
              const next = CLAUDE_PERM_MODES[(idx + 1) % CLAUDE_PERM_MODES.length];
              dsbRuntime.claudePermissionMode = next.id;
              writeStoredString(STORAGE_KEY_CLAUDE_PERM, next.id);
              notifyBarRuntime();
            };
            const claudeWsLabel =
              (extSel.workspacePath ? workspaceBasename(extSel.workspacePath) : '') ||
              extSel.title ||
              'Claude Code';

            // Claude Desktop (Code Mode) 1:1 Dark Replica:
            // Warm charcoal #242422 shell + #30302E rounded-16px composer surface + warm ivory #F2EFE9 text
            // + interactive permission pill + signature terracotta #D97757 rounded-square (9px) send button
            return h(
              'div',
              {
                'data-dsb-external-composer': 'claude_code',
                style: {
                  width: '100vw',
                  padding: '5px 6px 6px',
                  background: '#242422',
                  borderRadius: '0 0 18px 18px',
                  borderLeft: '1px solid rgba(255, 248, 240, 0.08)',
                  borderRight: '1px solid rgba(255, 248, 240, 0.08)',
                  borderBottom: '1px solid rgba(255, 248, 240, 0.08)',
                  boxSizing: 'border-box',
                  pointerEvents: 'auto',
                  fontFamily: '"Segoe UI", "PingFang SC", "Microsoft YaHei", system-ui, sans-serif',
                },
              },
              h(
                'div',
                {
                  style: {
                    background: '#30302E',
                    borderRadius: '15px',
                    border: '1px solid rgba(255, 248, 240, 0.11)',
                    padding: '13px 14px 10px 15px',
                    display: 'flex',
                    flexDirection: 'column',
                    boxSizing: 'border-box',
                    boxShadow: '0 4px 18px rgba(0, 0, 0, 0.28)',
                  },
                },
                renderExtCapsulesRow(),
                h('textarea', {
                  'data-dsb-ext-input': '1',
                  rows: 1,
                  value: extDraft,
                  onChange: handleExtInputChange,
                  onKeyDown: handleExtInputKeyDown,
                  placeholder: '今天想处理什么？支持按 Ctrl 圈选屏幕元素…',
                  style: {
                    width: '100%',
                    minHeight: '26px',
                    maxHeight: '140px',
                    border: 'none',
                    outline: 'none',
                    resize: 'none',
                    background: 'transparent',
                    color: '#F2EFE9',
                    fontSize: '14px',
                    lineHeight: '22px',
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
                      marginTop: '12px',
                      height: '32px',
                      gap: '8px',
                    },
                  },
                  h(
                    'div',
                    { style: { display: 'flex', alignItems: 'center', gap: '8px', minWidth: 0 } },
                    h(
                      'button',
                      {
                        type: 'button',
                        title: '切换 Harness / 工作区 / 对话',
                        onClick: openSwitcher,
                        style: {
                          width: '28px',
                          height: '28px',
                          borderRadius: '8px',
                          border: '1px solid rgba(255, 248, 240, 0.10)',
                          background: 'rgba(255, 248, 240, 0.04)',
                          padding: 0,
                          cursor: 'pointer',
                          display: 'inline-flex',
                          alignItems: 'center',
                          justifyContent: 'center',
                          flexShrink: 0,
                        },
                      },
                      renderPlusSvg(15, '#D8D4CA'),
                    ),
                    h(
                      'button',
                      {
                        type: 'button',
                        'data-dsb-claude-perm': currentPerm.id,
                        title: currentPerm.title + '（点击切换）',
                        onClick: cycleClaudePerm,
                        style: {
                          height: '28px',
                          padding: '0 9px',
                          borderRadius: '8px',
                          border:
                            currentPerm.id === 'plan'
                              ? '1px solid rgba(147, 197, 253, 0.34)'
                              : '1px solid rgba(217, 119, 87, 0.34)',
                          background:
                            currentPerm.id === 'plan'
                              ? 'rgba(59, 130, 246, 0.12)'
                              : 'rgba(217, 119, 87, 0.12)',
                          color: currentPerm.id === 'plan' ? '#93c5fd' : '#E89B7E',
                          fontSize: '12px',
                          fontWeight: 500,
                          cursor: 'pointer',
                          display: 'inline-flex',
                          alignItems: 'center',
                          gap: '5px',
                          flexShrink: 0,
                        },
                      },
                      renderShieldSvg(currentPerm.id === 'plan' ? '#93c5fd' : '#D97757'),
                      h('span', null, currentPerm.label),
                    ),
                    h(
                      'button',
                      {
                        type: 'button',
                        'data-dsb-session-chip': '1',
                        title: '点击切换 Harness / 工作区 / 对话',
                        onClick: openSwitcher,
                        style: {
                          height: '28px',
                          border: '1px solid rgba(255, 248, 240, 0.09)',
                          background: 'rgba(255, 248, 240, 0.04)',
                          padding: '0 9px',
                          borderRadius: '8px',
                          cursor: 'pointer',
                          display: 'inline-flex',
                          alignItems: 'center',
                          gap: '6px',
                          fontSize: '12.5px',
                          color: '#EAE6DF',
                          fontWeight: 500,
                          maxWidth: '250px',
                        },
                      },
                      renderClaudeSparkSvg(13, '#D97757'),
                      h(
                        'span',
                        {
                          style: {
                            overflow: 'hidden',
                            textOverflow: 'ellipsis',
                            whiteSpace: 'nowrap',
                          },
                        },
                        claudeWsLabel,
                      ),
                      renderChevronDownSvg('rgba(234, 230, 223, 0.55)'),
                    ),
                  ),
                  h(
                    'div',
                    { style: { display: 'flex', alignItems: 'center', gap: '10px', flexShrink: 0 } },
                    h(
                      'span',
                      {
                        title: '来自 ~/.claude/settings.json',
                        style: {
                          display: 'inline-flex',
                          alignItems: 'center',
                          gap: '5px',
                          fontSize: '12.5px',
                          whiteSpace: 'nowrap',
                          userSelect: 'none',
                        },
                      },
                      h('span', { style: { color: '#ECE9E2', fontWeight: 500 } }, claudeModel),
                      claudeEffort
                        ? h('span', { style: { color: '#9E9A90' } }, claudeEffort)
                        : null,
                      renderChevronDownSvg('#8A867C'),
                    ),
                    h(
                      'button',
                      {
                        type: 'button',
                        title: isExtStreaming ? '停止生成' : '发送到 Claude Code',
                        onClick: submitExternalComposer,
                        style: isExtStreaming
                          ? {
                              width: '30px',
                              height: '30px',
                              borderRadius: '9px',
                              border: '1px solid rgba(217, 119, 87, 0.45)',
                              background: '#262624',
                              cursor: 'pointer',
                              display: 'inline-flex',
                              alignItems: 'center',
                              justifyContent: 'center',
                              padding: 0,
                            }
                          : {
                              width: '30px',
                              height: '30px',
                              borderRadius: '9px',
                              border: 'none',
                              background: hasPayload ? '#D97757' : 'rgba(217, 119, 87, 0.46)',
                              color: '#FFFDF9',
                              cursor: 'pointer',
                              display: 'inline-flex',
                              alignItems: 'center',
                              justifyContent: 'center',
                              padding: 0,
                              transition: 'background 0.14s ease',
                            },
                      },
                      isExtStreaming
                        ? h('span', {
                            style: {
                              width: '10px',
                              height: '10px',
                              borderRadius: '2px',
                              background: '#D97757',
                            },
                          })
                        : renderArrowUpSvg('#FFFDF9'),
                    ),
                  ),
                ),
              ),
            );
          }

          if (activeHarness === 'antigravity') {
            const modelName = (activeAvail && activeAvail.modelLabel) || 'Gemini 4 Neon';
            const effortName = (activeAvail && activeAvail.effortLabel) || 'High';
            const accessName = (activeAvail && activeAvail.accessLabel) || '极速';
            const sessionLabel = extSel.title || 'CLI Project';

            return h(
              'div',
              {
                'data-dsb-external-composer': 'antigravity',
                style: {
                  width: '100vw',
                  padding: '4px 5px 5px',
                  background: '#111113',
                  borderRadius: '0 0 18px 18px',
                  borderLeft: '1px solid rgba(255, 255, 255, 0.06)',
                  borderRight: '1px solid rgba(255, 255, 255, 0.06)',
                  borderBottom: '1px solid rgba(255, 255, 255, 0.06)',
                  boxSizing: 'border-box',
                  pointerEvents: 'auto',
                  fontFamily: '"Segoe UI", "PingFang SC", "Microsoft YaHei", system-ui, sans-serif',
                },
              },
              h(
                'div',
                {
                  style: {
                    background: '#17171a',
                    borderRadius: '14px',
                    border: '1px solid rgba(255, 255, 255, 0.08)',
                    padding: '11px 13px 9px 14px',
                    display: 'flex',
                    flexDirection: 'column',
                    boxSizing: 'border-box',
                  },
                },
                renderExtCapsulesRow(),
                h('textarea', {
                  'data-dsb-ext-input': '1',
                  rows: 1,
                  value: extDraft,
                  onChange: handleExtInputChange,
                  onKeyDown: handleExtInputKeyDown,
                  placeholder: '提问任何问题，使用 @ 提及，/ 执行操作',
                  style: {
                    width: '100%',
                    minHeight: '24px',
                    maxHeight: '140px',
                    border: 'none',
                    outline: 'none',
                    resize: 'none',
                    background: 'transparent',
                    color: '#eceef2',
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
                      marginTop: '10px',
                      height: '30px',
                      gap: '8px',
                    },
                  },
                  h(
                    'div',
                    { style: { display: 'flex', alignItems: 'center', gap: '12px', minWidth: 0 } },
                    h(
                      'button',
                      {
                        type: 'button',
                        title: '切换 Harness / 工作区 / 对话',
                        onClick: openSwitcher,
                        style: {
                          border: 'none',
                          background: 'transparent',
                          padding: '2px',
                          cursor: 'pointer',
                          display: 'inline-flex',
                          alignItems: 'center',
                          justifyContent: 'center',
                          color: '#a1a1aa',
                        },
                      },
                      renderPlusSvg(15, '#a1a1aa'),
                    ),
                    h(
                      'span',
                      {
                        style: {
                          display: 'inline-flex',
                          alignItems: 'center',
                          gap: '5px',
                          fontSize: '12.5px',
                          whiteSpace: 'nowrap',
                          userSelect: 'none',
                        },
                      },
                      h('span', { style: { color: '#d4d4d8', fontWeight: 500 } }, modelName),
                      h('span', { style: { color: '#71717a' } }, effortName),
                      renderChevronDownSvg('#71717a'),
                    ),
                    h(
                      'button',
                      {
                        type: 'button',
                        'data-dsb-session-chip': '1',
                        title: '点击切换 Harness / 工作区 / 对话',
                        onClick: openSwitcher,
                        style: {
                          border: 'none',
                          background: 'transparent',
                          padding: '2px 4px',
                          borderRadius: '6px',
                          cursor: 'pointer',
                          display: 'inline-flex',
                          alignItems: 'center',
                          gap: '5px',
                          fontSize: '12px',
                          color: '#38bdf8',
                          maxWidth: '210px',
                        },
                      },
                      h('span', {
                        style: {
                          width: '5.5px',
                          height: '5.5px',
                          borderRadius: '999px',
                          background: '#38bdf8',
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
                        sessionLabel,
                      ),
                    ),
                  ),
                  h(
                    'div',
                    { style: { display: 'flex', alignItems: 'center', gap: '12px', flexShrink: 0 } },
                    h(
                      'span',
                      {
                        style: {
                          display: 'inline-flex',
                          alignItems: 'center',
                          gap: '5px',
                          fontSize: '12.5px',
                          color: '#d4d4d8',
                          userSelect: 'none',
                        },
                      },
                      renderShieldSvg('#9ca3af'),
                      accessName,
                      renderChevronDownSvg('#71717a'),
                    ),
                    h(
                      'button',
                      {
                        type: 'button',
                        title: isExtStreaming ? '停止生成' : '发送到 Antigravity',
                        onClick: submitExternalComposer,
                        style: isExtStreaming
                          ? {
                              width: '28px',
                              height: '28px',
                              borderRadius: '999px',
                              border: '1px solid rgba(255,255,255,0.08)',
                              background: '#232327',
                              cursor: 'pointer',
                              display: 'inline-flex',
                              alignItems: 'center',
                              justifyContent: 'center',
                              padding: 0,
                            }
                          : {
                              width: '28px',
                              height: '28px',
                              borderRadius: '999px',
                              border: 'none',
                              background: hasPayload ? '#f4f4f5' : '#d4d4d8',
                              cursor: 'pointer',
                              display: 'inline-flex',
                              alignItems: 'center',
                              justifyContent: 'center',
                              padding: 0,
                              opacity: hasPayload ? 1 : 0.88,
                            },
                      },
                      isExtStreaming
                        ? h('span', {
                            style: {
                              width: '10px',
                              height: '10px',
                              borderRadius: '2px',
                              background: '#c94a4a',
                            },
                          })
                        : renderArrowUpSvg('#111214'),
                    ),
                  ),
                ),
              ),
            );
          }

          // Codex Desktop 1:1 Replica (inset #1f1f1f top shelf + 20px #303032 graphite card + coral-orange 完全访问 + cobalt #2b6be6 button)
          const codexModel = (activeAvail && activeAvail.modelLabel) || 'GPT-6.1 Terra';
          const codexEffort = (activeAvail && activeAvail.effortLabel) || '高';
          const codexAccess = (activeAvail && activeAvail.accessLabel) || '完全访问';
          const codexWsLabel =
            (extSel.workspacePath ? workspaceBasename(extSel.workspacePath) : '') ||
            extSel.title ||
            'Codex';

          return h(
            'div',
            {
              'data-dsb-external-composer': 'codex',
              style: {
                width: '100vw',
                display: 'flex',
                flexDirection: 'column',
                background: 'transparent',
                boxSizing: 'border-box',
                pointerEvents: 'auto',
                fontFamily: '"Segoe UI", "PingFang SC", "Microsoft YaHei", system-ui, sans-serif',
              },
            },
            // Inset top shelf matching Codex Desktop (`💻 此计算机  📁 <workspace>`)
            h(
              'div',
              {
                style: {
                  margin: '0 14px',
                  height: '34px',
                  padding: '0 14px',
                  background: '#1f1f1f',
                  borderLeft: '1px solid rgba(255, 255, 255, 0.06)',
                  borderRight: '1px solid rgba(255, 255, 255, 0.06)',
                  borderTop: '1px solid rgba(255, 255, 255, 0.05)',
                  display: 'flex',
                  alignItems: 'center',
                  gap: '16px',
                  boxSizing: 'border-box',
                },
              },
              h(
                'span',
                {
                  style: {
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: '6px',
                    fontSize: '12.5px',
                    color: '#e4e4e7',
                    fontWeight: 500,
                    flexShrink: 0,
                    userSelect: 'none',
                  },
                },
                renderLaptopSvg('#d4d4d8'),
                '此计算机',
              ),
              h(
                'button',
                {
                  type: 'button',
                  'data-dsb-session-chip': '1',
                  title: '点击切换 Harness / 工作区 / 对话',
                  onClick: openSwitcher,
                  style: {
                    border: 'none',
                    background: 'transparent',
                    padding: '2px 4px',
                    borderRadius: '6px',
                    cursor: 'pointer',
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: '6px',
                    fontSize: '12.5px',
                    color: '#f4f4f5',
                    fontWeight: 500,
                    maxWidth: '340px',
                  },
                },
                renderFolderOutlineSvg('#d4d4d8'),
                h(
                  'span',
                  {
                    style: {
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    },
                  },
                  codexWsLabel,
                ),
              ),
            ),
            // Main 20px-radius Codex graphite composer card
            h(
              'div',
              {
                style: {
                  width: '100%',
                  background: '#303032',
                  borderRadius: '20px',
                  border: '1px solid rgba(255, 255, 255, 0.08)',
                  padding: '13px 16px 11px 16px',
                  display: 'flex',
                  flexDirection: 'column',
                  boxSizing: 'border-box',
                  boxShadow: '0 8px 26px rgba(0, 0, 0, 0.36)',
                },
              },
              renderExtCapsulesRow(),
              h('textarea', {
                'data-dsb-ext-input': '1',
                rows: 1,
                value: extDraft,
                onChange: handleExtInputChange,
                onKeyDown: handleExtInputKeyDown,
                placeholder: '随心输入',
                style: {
                  width: '100%',
                  minHeight: '26px',
                  maxHeight: '140px',
                  border: 'none',
                  outline: 'none',
                  resize: 'none',
                  background: 'transparent',
                  color: '#f4f4f5',
                  fontSize: '14px',
                  lineHeight: '22px',
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
                    marginTop: '14px',
                    height: '30px',
                    gap: '8px',
                  },
                },
                h(
                  'div',
                  { style: { display: 'flex', alignItems: 'center', gap: '12px' } },
                  h(
                    'button',
                    {
                      type: 'button',
                      title: '切换 Harness / 工作区 / 对话',
                      onClick: openSwitcher,
                      style: {
                        border: 'none',
                        background: 'transparent',
                        padding: '2px',
                        cursor: 'pointer',
                        display: 'inline-flex',
                        alignItems: 'center',
                        justifyContent: 'center',
                      },
                    },
                    renderPlusSvg(17, '#e4e4e7'),
                  ),
                  h(
                    'span',
                    {
                      title: '来自 ~/.codex/config.toml',
                      style: {
                        fontSize: '13px',
                        fontWeight: 500,
                        color: '#ef6c35',
                        userSelect: 'none',
                      },
                    },
                    codexAccess,
                  ),
                ),
                h(
                  'div',
                  { style: { display: 'flex', alignItems: 'center', gap: '12px' } },
                  h(
                    'span',
                    {
                      style: {
                        display: 'inline-flex',
                        alignItems: 'center',
                        gap: '6px',
                        fontSize: '13px',
                        userSelect: 'none',
                      },
                    },
                    h('span', { style: { color: '#f4f4f5', fontWeight: 500 } }, codexModel),
                    codexEffort
                      ? h('span', { style: { color: '#9ca3af' } }, codexEffort)
                      : null,
                  ),
                  h(
                    'button',
                    {
                      type: 'button',
                      title: isExtStreaming ? '停止生成' : '发送到 Codex',
                      onClick: submitExternalComposer,
                      style: {
                        width: '30px',
                        height: '30px',
                        borderRadius: '999px',
                        border: 'none',
                        background: '#2b6be6',
                        color: '#ffffff',
                        cursor: 'pointer',
                        display: 'inline-flex',
                        alignItems: 'center',
                        justifyContent: 'center',
                        padding: 0,
                      },
                    },
                    isExtStreaming
                      ? h('span', {
                          style: {
                            width: '10px',
                            height: '10px',
                            borderRadius: '2px',
                            background: '#ffffff',
                          },
                        })
                      : renderArrowUpSvg('#ffffff'),
                  ),
                ),
              ),
            ),
          );
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
                height: COLLAPSED_LIP_PX + 'px',
                zIndex: 1,
                boxSizing: 'border-box',
                pointerEvents: 'auto',
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
                    zIndex: 35,
                  },
                }),
              ]
            : null,

          h(
            'div',
            {
              'data-dsb-card': '',
              style: {
                position: 'absolute',
                left: 0,
                right: 0,
                bottom: -TUCK_OVERLAP_PX + 'px',
                height: (isExpanded ? expandedHeight : COLLAPSED_LIP_PX) + TUCK_OVERLAP_PX + 'px',
                paddingBottom: TUCK_OVERLAP_PX + 'px',
                borderRadius: cardBorderRadius,
                borderBottom: 'none',
                background: 'var(--dsw-alias-bg-layer-2, #2c2c2e)',
                boxShadow: IS_BAR
                  ? '0 0 0 0.5px var(--dsw-alias-border-l2, rgba(128, 128, 128, 0.22)) inset'
                  : isExpanded
                    ? '0 0 0 0.5px rgba(255, 255, 255, 0.14), 0 -14px 36px rgba(0, 0, 0, 0.42)'
                    : '0 0 0 0.5px rgba(255, 255, 255, 0.10)',
                color: 'var(--dsw-alias-label-primary, #f3f4f6)',
                display: 'flex',
                flexDirection: 'column',
                overflow: 'hidden',
                boxSizing: 'border-box',
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
                        const meta = HARNESS_META[activeHarness] || HARNESS_META.dsh;
                        const isDesktopRunning =
                          activeHarness === 'dsh'
                            ? true
                            : Boolean(
                                (activeAvail && activeAvail.running) ||
                                  (dsbRuntime.liveSyncMeta[activeHarness] &&
                                    dsbRuntime.liveSyncMeta[activeHarness].running),
                              );
                        const sel =
                          activeHarness === 'dsh'
                            ? { sessionId: sessionId || '', title: '' }
                            : dsbRuntime.externalSelection[activeHarness] || {};
                        return h(
                          'button',
                          {
                            type: 'button',
                            title:
                              activeHarness === 'dsh'
                                ? '点击在浏览器打开当前 DeepSeek Harness 对话'
                                : '点击将 ' + meta.label + ' 拉到最前并展示这个对话',
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
                          activeHarness === 'antigravity'
                            ? 'Antigravity 已就绪 — 发消息后自动同步到桌面端 CLI Project'
                            : activeHarness === 'codex'
                              ? 'Codex (app-server) 已就绪 — 发消息后写入 ~/.codex 共享历史'
                              : activeHarness === 'claude_code'
                                ? 'Claude Code CLI 已就绪 — 发消息后自动同步至 ~/.claude/projects 会话历史'
                                : '暂无历史消息 — 在下方输入框发消息后，对话流将在此实时展开',
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
                        opacity: latestSingleLine ? 0.92 : 0.62,
                      },
                    },
                    latestSingleLine ||
                      (activeHarness === 'antigravity'
                        ? 'Antigravity · CLI Project 已就绪 · 双击 Ctrl 唤出输入框'
                        : activeHarness === 'codex'
                          ? 'Codex · app-server 已就绪 · 双击 Ctrl 唤出输入框'
                          : activeHarness === 'claude_code'
                            ? 'Claude Code · CLI 已就绪 · 双击 Ctrl 唤出输入框'
                            : '暂无消息 · 双击 Ctrl 唤出输入框'),
                  ),
                ),
            !(IS_BAR && barMode === 'lip')
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
            ),
          ),
          renderExternalNativeComposer(),
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
        const [hoveredWsId, setHoveredWsId] = useState('');
        const [manualPathMode, setManualPathMode] = useState(false);
        const [manualPathValue, setManualPathValue] = useState('');
        const [pickerError, setPickerError] = useState('');
        const [pillMetrics, setPillMetrics] = useState(null);
        const [submenuMarginBottom, setSubmenuMarginBottom] = useState(0);

        const btnRef = useRef(null);
        const extAnchorRef = useRef(null);
        const menuRef = useRef(null);
        const wsPanelRef = useRef(null);
        const convPanelRef = useRef(null);
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
        const harnessTheme = HARNESS_META[activeHarness] || HARNESS_META.dsh;

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
        const extSel = activeHarness !== 'dsh' ? dsbRuntime.externalSelection[activeHarness] || {} : null;
        const activeSessionId = activeHarness === 'dsh' ? dshActiveSessionId : String((extSel && extSel.sessionId) || '');
        const activeTitle =
          activeHarness === 'dsh'
            ? sessionDisplayTitle(activeSession)
            : (extSel && extSel.title) || harnessTheme.label + ' · 新对话';

        // Compute grouped workspaces and conversations for the active Harness
        const { groups, activeWsId } = useMemo(() => {
          if (activeHarness !== 'dsh') {
            const catalog = dsbRuntime.externalCatalogs[activeHarness];
            const extGroups = catalog && Array.isArray(catalog.groups) ? catalog.groups : [];
            const selWsId = (extSel && extSel.workspaceId) || '';
            const found = extGroups.some((g) => g.workspaceId === selWsId)
              ? selWsId
              : extGroups[0]
                ? extGroups[0].workspaceId
                : '';
            return {
              groups: extGroups,
              activeWsId: found,
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
            activeWsId: foundWsId || (result[0] ? result[0].workspaceId : ''),
          };
        }, [activeHarness, wsItems, sessSnap, dshActiveSessionId, extSel && extSel.workspaceId, dsbRuntime.externalCatalogs[activeHarness]]);

        useEffect(() => {
          if (menuOpen) {
            refreshHarnessAvailability();
            if (activeHarness !== 'dsh') {
              refreshExternalCatalog(activeHarness, false);
            }
            setHoveredWsId(activeWsId || (groups[0] ? groups[0].workspaceId : ''));
            setManualPathMode(false);
            setPickerError('');
          }
        }, [menuOpen, activeHarness, activeWsId]);

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

        // Re-snap popover coordinates on the next frame whenever harness switches while menu is open
        useEffect(() => {
          if (!menuOpen) return undefined;
          const raf = window.requestAnimationFrame(() => {
            setTick((n) => n + 1);
            if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
              dsbRuntime.scheduleLayoutReport(280);
            }
          });
          return () => window.cancelAnimationFrame(raf);
        }, [menuOpen, activeHarness]);

        const handleOpenSession = (conv, group) => {
          setMenuOpen(false);
          if (activeHarness !== 'dsh') {
            selectExternalConversation(activeHarness, group, conv);
            return;
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

        const handleNewConversation = async (group) => {
          setMenuOpen(false);
          if (activeHarness !== 'dsh') {
            startNewExternalConversation(activeHarness, group);
            return;
          }
          const wsId = typeof group === 'string' ? group : group.workspaceId;
          try {
            if (uiWorkspace && typeof uiWorkspace.startSession === 'function') {
              await uiWorkspace.startSession(wsId);
              return;
            }
            if (sessions && typeof sessions.create === 'function') {
              const newId = await sessions.create({ workspaceId: wsId });
              if (newId) handleOpenSession(newId);
            }
          } catch (err) {
            setPickerError(err instanceof Error ? err.message : String(err));
          }
        };

        const createWorkspaceAtPath = async (pickedPath) => {
          if (!pickedPath) return;
          if (activeHarness !== 'dsh') {
            const label = workspaceBasename(pickedPath);
            startNewExternalConversation(activeHarness, {
              workspaceId:
                activeHarness === 'codex' || activeHarness === 'claude_code'
                  ? pickedPath
                  : 'default-cli-project',
              label,
              path: pickedPath,
            });
            setMenuOpen(false);
            return;
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
          if (activeHarness !== 'dsh') {
            setManualPathMode(true);
            return;
          }
          dsbRuntime.modalOpen = true;
          notifyBarRuntime();
          try {
            let pickedPath = null;
            if (uiWorkspace && typeof uiWorkspace.pickDirectory === 'function') {
              pickedPath = await uiWorkspace.pickDirectory();
            } else if (directoryPicker && typeof directoryPicker.pick === 'function') {
              const res = await directoryPicker.pick();
              if (res && res.ok) pickedPath = res.value;
            } else {
              setManualPathMode(true);
              return;
            }
            if (pickedPath) {
              await createWorkspaceAtPath(pickedPath);
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
          if (!menuOpen || settingsView || !hoveredGroup) {
            setSubmenuMarginBottom(0);
            return;
          }
          const panelEl = wsPanelRef.current;
          const convEl = convPanelRef.current;
          const rowEl = wsRowRefs.current.get(hoveredGroup.workspaceId);
          if (!panelEl || !convEl || !rowEl) {
            setSubmenuMarginBottom(0);
            return;
          }
          const pr = panelEl.getBoundingClientRect();
          const cr = convEl.getBoundingClientRect();
          const rr = rowEl.getBoundingClientRect();
          if (pr.height < 12 || cr.height < 12 || rr.height < 8) {
            setSubmenuMarginBottom(0);
            return;
          }
          // Align the right panel's first row (6px padding + 1px border = 7px from panel top)
          // directly with the hovered workspace row `rr.top`, while clamping so Level 2 never
          // overflows above Level 1's top or below Level 1's bottom (above the Bar card).
          const targetTop = Math.max(0, Math.round(rr.top - pr.top - 7));
          const maxBottomOffset = Math.max(0, Math.round(pr.height - cr.height));
          const desiredBottomOffset = Math.round(pr.height - (targetTop + cr.height));
          const clamped = Math.max(0, Math.min(maxBottomOffset, desiredBottomOffset));
          setSubmenuMarginBottom((prev) => (prev === clamped ? prev : clamped));
        }, [menuOpen, settingsView, hoveredGroup ? hoveredGroup.workspaceId : '', hoveredGroup ? hoveredGroup.convs.length : 0]);

        useLayoutEffect(() => {
          recomputeSubmenuAlignment();
          if (!menuOpen || settingsView) return undefined;
          const raf = window.requestAnimationFrame(recomputeSubmenuAlignment);
          return () => window.cancelAnimationFrame(raf);
        }, [recomputeSubmenuAlignment, menuOpen, settingsView, activeHarness, groups.length]);

        const availById = useMemo(() => {
          const map = {};
          for (const item of dsbRuntime.harnessAvailability || []) {
            if (item && item.id) map[item.id] = item;
          }
          return map;
        }, [dsbRuntime.harnessAvailability]);

        const renderMenu = () => {
          if (!menuOpen) return null;
          const isValidRect = (r) => !!(r && r.width > 4 && r.height > 4 && r.bottom > 0);

          // Always measure the live, visible Bar slab (upper card + active composer) so `bottomPx`
          // is mathematically invariant to window vertical resizes and NEVER drifts or flies off-screen
          // when switching harnesses (where old buttons unmount with 0x0 rect).
          const cardEl = document.querySelector('[data-dsb-card]');
          const compEl =
            dsbRuntime.activeHarness !== 'dsh'
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
          const fallbackSlabH =
            dsbRuntime.activeHarness === 'codex'
              ? 196
              : dsbRuntime.activeHarness === 'claude_code'
                ? 174
                : 158;
          // Pure element-span height (slabBottom - slabTop) is 100% invariant to
          // Win32 SetWindowPos resizes and window.innerHeight lag, eliminating 60Hz vertical jitter.
          const slabHeight =
            slabTop !== null && slabBottom !== null
              ? Math.max(56, Math.min(380, Math.round(slabBottom - slabTop)))
              : fallbackSlabH;
          const bottomPx = slabHeight + 8;

          // Live visible session chip for horizontal alignment (ignore detached / hidden 0x0 refs)
          let chipRect = null;
          if (dsbRuntime.activeHarness !== 'dsh') {
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

          const menuMaxW = 468;
          const leftPx = chipRect
            ? Math.max(8, Math.min(Math.round(chipRect.left), Math.max(8, window.innerWidth - menuMaxW - 8)))
            : 12;

          const panelBg = harnessTheme.surface;
          const panelBorder = harnessTheme.border;

          const snap = dsbRuntime.appSettings || {
            version: '0.3.14',
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
                  background: 'rgba(255,255,255,0.03)',
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
                'data-dsb-settings-panel': '1',
                'data-dg-scroll': '1',
                style: {
                  width: '466px',
                  maxHeight: '430px',
                  overflowY: 'auto',
                  borderRadius: '14px',
                  padding: '9px 10px',
                  background: panelBg,
                  border: '1px solid ' + panelBorder,
                  boxShadow: '0 8px 24px rgba(0,0,0,0.38)',
                  display: 'flex',
                  flexDirection: 'column',
                  gap: '6px',
                  boxSizing: 'border-box',
                },
              },
              // Header
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
                  h('span', { style: { fontSize: '12.5px', fontWeight: 650 } }, '插件与悬浮条设置'),
                  h(
                    'span',
                    {
                      style: {
                        fontSize: '10.5px',
                        padding: '1px 6px',
                        borderRadius: '999px',
                        background: harnessTheme.accentBg,
                        color: harnessTheme.accent,
                        border: '1px solid ' + harnessTheme.accentBorder,
                      },
                    },
                    'v' + (snap.version || '0.3.0'),
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
                      background: 'rgba(255,255,255,0.05)',
                      color: 'inherit',
                      borderRadius: '8px',
                      padding: '3px 8px',
                      fontSize: '11.5px',
                      cursor: 'pointer',
                    },
                  },
                  '← 返回工作区',
                ),
              ),

              // Output target segmented picker
              h(
                'div',
                { style: { display: 'flex', flexDirection: 'column', gap: '4px' } },
                h('div', { style: { fontSize: '11px', opacity: 0.65, paddingLeft: '4px' } }, '选区胶囊投递目标'),
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
                          border: active ? '1px solid ' + harnessTheme.accentBorder : '1px solid transparent',
                          background: active ? harnessTheme.accentBg : 'rgba(255,255,255,0.04)',
                          color: active ? '#ffffff' : 'rgba(229,231,235,0.78)',
                        },
                      },
                      opt.label,
                    );
                  }),
                ),
              ),

              // Core toggles
              h(
                'div',
                { style: { display: 'flex', flexDirection: 'column', gap: '4px' } },
                renderToggleRow(
                  'syncExternalPlugins',
                  '四家插件伴随启动 & 实时对话同步',
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
              ),

              // Quick action footer (replaces old system tray actions)
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
                    { id: 'resync_plugins', label: '重同步插件', toast: '四家插件已同步' },
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
                          background: 'rgba(255,255,255,0.04)',
                          color: 'rgba(229,231,235,0.88)',
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

          const menuNode = h(
            'div',
            {
              ref: menuRef,
              'data-dsb-popup': '1',
              'data-dsb-popup-bottom': String(bottomPx),
              style: {
                position: 'fixed',
                bottom: bottomPx + 'px',
                left: leftPx + 'px',
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'stretch',
                gap: '6px',
                zIndex: 9999,
                pointerEvents: 'auto',
                color: 'var(--dsw-alias-label-primary, #f3f4f6)',
                fontFamily: 'system-ui, -apple-system, "Segoe UI", Roboto, "PingFang SC", sans-serif',
              },
            },
            // Level 0 (Top): Harness Switcher (DeepSeek / Antigravity / Codex / Claude) + Refresh + Settings Gear
            h(
              'div',
              {
                'data-dsb-harness-bar': '1',
                style: {
                  width: '466px',
                  borderRadius: '14px',
                  padding: '5px 6px',
                  background: panelBg,
                  border: '1px solid ' + panelBorder,
                  boxShadow: '0 8px 24px rgba(0,0,0,0.38)',
                  display: 'flex',
                  alignItems: 'center',
                  gap: '4px',
                  boxSizing: 'border-box',
                },
              },
              HARNESS_ORDER.map((hid) => {
                const meta = HARNESS_META[hid];
                const isSelected = activeHarness === hid;
                const avail = availById[hid];
                const isReady = avail ? Boolean(avail.available) : true;
                return h(
                  'button',
                  {
                    key: hid,
                    type: 'button',
                    'data-dsb-harness-opt': hid,
                    title: meta.fullLabel + ' · ' + meta.syncHint,
                    onClick: () => {
                      setPickerError('');
                      setManualPathMode(false);
                      setActiveHarness(hid, true);
                    },
                    style: {
                      flex: 1,
                      padding: '6px 6px',
                      borderRadius: '10px',
                      border: isSelected ? '1px solid ' + meta.accentBorder : '1px solid transparent',
                      background: isSelected ? meta.accentBg : 'transparent',
                      color: isSelected ? '#ffffff' : 'rgba(229, 231, 235, 0.78)',
                      fontSize: '12px',
                      fontWeight: isSelected ? 650 : 500,
                      cursor: 'pointer',
                      display: 'inline-flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      gap: '5px',
                      opacity: isReady ? 1 : 0.55,
                      whiteSpace: 'nowrap',
                      transition: 'background 0.14s ease, border-color 0.14s ease',
                    },
                  },
                  h('span', {
                    style: {
                      width: '7px',
                      height: '7px',
                      borderRadius: '999px',
                      background: isReady ? meta.accent : '#6b7280',
                      boxShadow: isSelected ? '0 0 8px ' + meta.accent : 'none',
                      flexShrink: 0,
                    },
                  }),
                  h('span', null, meta.label),
                );
              }),
              activeHarness !== 'dsh' && !settingsView
                ? h(
                    'button',
                    {
                      type: 'button',
                      title: '刷新工作区与历史会话',
                      onClick: () => refreshExternalCatalog(activeHarness, false),
                      style: {
                        width: '26px',
                        height: '26px',
                        borderRadius: '8px',
                        border: 'none',
                        background: 'rgba(255,255,255,0.06)',
                        color: 'rgba(229,231,235,0.85)',
                        fontSize: '13px',
                        cursor: 'pointer',
                        flexShrink: 0,
                      },
                    },
                    '↻',
                  )
                : null,
              h(
                'button',
                {
                  type: 'button',
                  'data-dsb-settings-btn': '1',
                  title: settingsView ? '返回工作区列表' : '插件与悬浮条设置',
                  onClick: () => {
                    const next = !settingsView;
                    if (next) refreshAppSettings();
                    setSettingsView(next);
                    if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
                      dsbRuntime.scheduleLayoutReport(240);
                    }
                  },
                  style: {
                    width: '26px',
                    height: '26px',
                    borderRadius: '8px',
                    border: settingsView ? '1px solid ' + harnessTheme.accentBorder : 'none',
                    background: settingsView ? harnessTheme.accentBg : 'rgba(255,255,255,0.06)',
                    color: settingsView ? harnessTheme.accent : 'rgba(229,231,235,0.85)',
                    fontSize: '13px',
                    cursor: 'pointer',
                    flexShrink: 0,
                    display: 'inline-flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    padding: 0,
                  },
                },
                '⚙',
              ),
            ),

            // Either Settings Panel OR Workspaces | Conversations
            settingsView
              ? renderSettingsBody()
              : h(
              'div',
              {
                style: {
                  display: 'flex',
                  alignItems: 'flex-end',
                  gap: '6px',
                },
              },
            // Level 1: Workspaces
            h(
              'div',
              {
                ref: wsPanelRef,
                style: {
                  width: '196px',
                  maxHeight: '286px',
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
                          createWorkspaceAtPath(manualPathValue.trim()).catch((err) => {
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
                        border: '1px solid ' + harnessTheme.accentBorder,
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
                              createWorkspaceAtPath(manualPathValue.trim()).catch((err) => {
                                setPickerError(err instanceof Error ? err.message : String(err));
                              });
                            }
                          },
                          style: { fontSize: '11.5px', padding: '2px 8px', borderRadius: '6px', background: harnessTheme.accentBg, color: harnessTheme.accent, border: '1px solid ' + harnessTheme.accentBorder, cursor: 'pointer' },
                        },
                        '确定',
                      ),
                    ),
                  )
                : h(
                    'button',
                    {
                      type: 'button',
                      onClick: handleCreateWorkspace,
                      style: {
                        width: '100%',
                        textAlign: 'left',
                        padding: '6px 10px',
                        borderRadius: '10px',
                        fontSize: '12px',
                        fontWeight: 500,
                        color: harnessTheme.accent,
                        background: harnessTheme.accentBg,
                        border: 'none',
                        cursor: 'pointer',
                        display: 'flex',
                        alignItems: 'center',
                        gap: '6px',
                      },
                    },
                    h('span', null, '＋'),
                    h('span', null, activeHarness === 'dsh' ? '新建工作区…' : '指定目录新开…'),
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
                  style: { overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: '2px', maxHeight: '206px' },
                },
                activeHarness !== 'dsh' && dsbRuntime.externalLoading[activeHarness] && groups.length === 0
                  ? h('div', { style: { padding: '12px 10px', fontSize: '12px', opacity: 0.65, textAlign: 'center' } }, '正在读取工作区…')
                  : groups.length === 0
                    ? h('div', { style: { padding: '12px 10px', fontSize: '12px', opacity: 0.65, textAlign: 'center' } }, '暂无工作区')
                    : groups.map((g) => {
                  const isHover = hoveredGroup && g.workspaceId === hoveredGroup.workspaceId;
                  const isActiveWs = g.workspaceId === activeWsId;
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
                        padding: '6px 10px',
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

            // Level 2: Conversations in hovered workspace (first row aligned with hovered workspace row)
            hoveredGroup
              ? h(
                  'div',
                  {
                    ref: convPanelRef,
                    style: {
                      width: '248px',
                      maxHeight: '286px',
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
                      transition: 'margin-bottom 90ms ease-out',
                    },
                  },
                  hoveredGroup.workspaceId !== '__ungrouped__'
                    ? h(
                        'button',
                        {
                          type: 'button',
                          onClick: () => handleNewConversation(hoveredGroup),
                          style: {
                            width: '100%',
                            textAlign: 'left',
                            padding: '6px 10px',
                            borderRadius: '10px',
                            fontSize: '12px',
                            fontWeight: 500,
                            color: harnessTheme.accent,
                            background: harnessTheme.accentBg,
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
                          const isSel = c.id === activeSessionId;
                          return h(
                            'button',
                            {
                              key: c.id,
                              type: 'button',
                              onClick: () => handleOpenSession(c, hoveredGroup),
                              style: {
                                width: '100%',
                                textAlign: 'left',
                                padding: '6px 10px',
                                borderRadius: '10px',
                                fontSize: '12px',
                                background: isSel ? harnessTheme.accentBg : 'transparent',
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
                              h('span', { style: { width: '12px', textAlign: 'center', color: harnessTheme.accent, flexShrink: 0 } }, isSel ? '✓' : ''),
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

        return h(
          React.Fragment,
          null,
          activeHarness === 'codex'
            ? h(
                'span',
                {
                  title: 'Codex 沙箱权限（来自 ~/.codex/config.toml）',
                  style: {
                    fontSize: '12px',
                    fontWeight: 500,
                    color: '#fb6a33',
                    padding: '0 6px 0 2px',
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: '4px',
                    flexShrink: 0,
                    userSelect: 'none',
                  },
                },
                (activeAvail && activeAvail.accessLabel) || '完全访问',
              )
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
                background:
                  activeHarness === 'dsh'
                    ? (pillMetrics && pillMetrics.background) || 'var(--dsw-alias-bg-layer-3, rgba(128,128,128,0.12))'
                    : 'rgba(255, 255, 255, 0.05)',
                color:
                  activeHarness === 'dsh'
                    ? (pillMetrics && pillMetrics.color) || 'var(--dsw-alias-label-primary, inherit)'
                    : 'rgba(243, 244, 246, 0.90)',
                border:
                  activeHarness === 'dsh'
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
            activeHarness === 'dsh' || activeHarness === 'codex'
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
        if (!IS_BAR || hId === 'dsh') return null;
        const avail = (dsbRuntime.harnessAvailability || []).find((a) => a && a.id === hId) || null;

        if (hId === 'antigravity') {
          const modelText = (avail && avail.modelLabel) || 'Gemini';
          const effortText = (avail && avail.effortLabel) || 'High';
          const accessText = (avail && avail.accessLabel) || '极速';
          return h(
            'div',
            {
              'data-dsb-harness-right': 'antigravity',
              style: {
                display: 'inline-flex',
                alignItems: 'center',
                gap: '10px',
                fontSize: '12px',
                color: 'rgba(229, 231, 235, 0.78)',
                paddingRight: '4px',
                userSelect: 'none',
                flexShrink: 0,
              },
            },
            h(
              'span',
              { style: { display: 'inline-flex', alignItems: 'center', gap: '4px' } },
              h('span', { style: { color: 'rgba(243, 244, 246, 0.88)' } }, modelText),
              h('span', { style: { opacity: 0.55 } }, effortText),
            ),
            h(
              'span',
              {
                style: {
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: '4px',
                  color: 'rgba(209, 213, 219, 0.82)',
                },
              },
              h('span', {
                style: {
                  width: '6px',
                  height: '6px',
                  borderRadius: '999px',
                  background: '#38bdf8',
                },
              }),
              accessText,
            ),
          );
        }

        if (hId === 'codex') {
          const modelText = (avail && avail.modelLabel) || 'GPT-6.1 Terra';
          const effortText = (avail && avail.effortLabel) || '高';
          return h(
            'div',
            {
              'data-dsb-harness-right': 'codex',
              style: {
                display: 'inline-flex',
                alignItems: 'center',
                gap: '6px',
                fontSize: '12px',
                paddingRight: '4px',
                userSelect: 'none',
                flexShrink: 0,
              },
            },
            h('span', { style: { color: 'rgba(244, 244, 245, 0.90)', fontWeight: 500 } }, modelText),
            effortText
              ? h('span', { style: { color: 'rgba(212, 212, 216, 0.58)' } }, effortText)
              : null,
          );
        }

        if (hId === 'claude_code') {
          const modelText = (avail && avail.modelLabel) || 'Claude Opus';
          const effortText = (avail && avail.effortLabel) || '高';
          return h(
            'div',
            {
              'data-dsb-harness-right': 'claude_code',
              style: {
                display: 'inline-flex',
                alignItems: 'center',
                gap: '6px',
                fontSize: '12px',
                paddingRight: '4px',
                userSelect: 'none',
                flexShrink: 0,
              },
            },
            h('span', { style: { color: '#ECE9E2', fontWeight: 500 } }, modelText),
            effortText
              ? h('span', { style: { color: '#9E9A90' } }, effortText)
              : null,
          );
        }

        return null;
      };
    }

    return {
      inject: ['slots', 'sessions', 'workspaces', 'uiWorkspace'],
      apply(ctx) {
        // Never inject UI, styles, or slots into DSH's normal WebUI — only activate inside Anything-to-Prompt (?dsh_bar_clone=1)
        if (!IS_BAR) return;
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
            if (typeof document !== 'undefined' && document.querySelector('[data-dsb-card]')) {
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
                if (!document.querySelector('[data-dsb-card]')) {
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
