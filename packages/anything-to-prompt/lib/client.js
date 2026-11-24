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
    };

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
      window.__DSB_BRIDGE__ = {
        setMode(mode) {
          setBarMode(mode);
          return dsbRuntime.barMode;
        },
        focusComposer() {
          return focusComposerElement();
        },
        setDraft(text) {
          const val = String(text || '');
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
          const composerEl = typeof document !== 'undefined' ? document.querySelector('[data-composer-card]') : null;
          return {
            mode: dsbRuntime.barMode,
            sessionId: dsbRuntime.sessionId,
            hovered: dsbRuntime.hovered,
            pinned: dsbRuntime.pinned,
            popupOpen: dsbRuntime.popupOpen,
            modal: dsbRuntime.modalOpen,
            composerFocused: Boolean(composerEl && activeEl && composerEl.contains(activeEl)),
            capsuleCount: countComposerCapsules(dsbRuntime.liveDraft),
            expandedHeight: dsbRuntime.expandedHeight,
            barWidth: dsbRuntime.barWidth,
          };
        },
      };
    }

    const STYLE_ID = 'anything-to-prompt-standalone-styles-v4';
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
        '[data-dsb-card], [data-dsb-card] * {',
        '  font-family: inherit !important;',
        '  font-weight: inherit !important;',
        '}',
        '[data-dsb-card] * {',
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
        'html[data-dsh-bar-clone="1"] [data-composer-seat] *:has([data-composer-card]) > *:not([data-composer-card]):not(:has([data-composer-card])):not([data-donegate-dock-wrap]):not(:has([data-donegate-dock-wrap])):not([data-queue-dock]):not(:has([data-queue-dock])):not([data-slot="conversation.input.dock"]):not(:has([data-slot="conversation.input.dock"])) {',
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
      ].join('\n');
      document.head.appendChild(styleEl);
    }

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

        const entries = extractMessagesFromDshProps(props);
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

        const latestSingleLine = latestAssistant
          ? latestAssistant.text.replace(/\s+/g, ' ').trim()
          : '';

        const userMessageCount = useMemo(() => {
          let count = 0;
          for (let i = 0; i < entries.length; i += 1) {
            if (entries[i].role === 'user') count += 1;
          }
          return count;
        }, [entries]);

        const isExpanded = pinned || hovered || previewExpanded;

        // Track explicit user submit intent inside [data-composer-card] (prevents async session hydration from misfiring 'submitted')
        useEffect(() => {
          if (!IS_BAR || typeof document === 'undefined') return undefined;
          const onKeyDown = (e) => {
            if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
              const comp = document.querySelector('[data-composer-card]');
              if (comp && e.target instanceof Node && comp.contains(e.target)) {
                submitIntentUntilRef.current = Date.now() + 1800;
              }
            }
          };
          const onClick = (e) => {
            const comp = document.querySelector('[data-composer-card]');
            if (comp && e.target instanceof Element && comp.contains(e.target)) {
              const btn = e.target.closest('button');
              if (btn && !btn.hasAttribute('data-dsb-session-chip')) {
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
            sessionId,
            latestAssistantId: latestAssistant ? String(latestAssistant.id) : '',
            streaming: Boolean(latestAssistant && latestAssistant.streaming),
            userMessageCount,
          });
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
        }, [sessionId, latestAssistant ? latestAssistant.id : '', Boolean(latestAssistant && latestAssistant.streaming), userMessageCount]);

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
            const composerEl = typeof document !== 'undefined' ? document.querySelector('[data-composer-card]') : null;
            sendBarReport({
              kind: 'ui',
              hovered,
              pinned,
              popupOpen: dsbRuntime.popupOpen,
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
            if (dsbRuntime.popupOpen || dsbRuntime.modalOpen || hasNativeMenu) return;
            e.preventDefault();
            e.stopPropagation();
            sendBarReport({ kind: 'step_down' });
          };
          const onPointerDownCapture = (e) => {
            const composerEl = document.querySelector('[data-composer-card]');
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
          const composerEl = barMode === 'lip' ? null : document.querySelector('[data-composer-card]');
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
          if (hasSlab) {
            shapes.push({
              x: Math.round(slabLeft),
              y: Math.round(slabTop),
              w: Math.round(slabRight - slabLeft),
              h: Math.round(slabBottom - slabTop),
              r: CARD_RADIUS_PX,
            });
          }

          const cardTarget = isExpanded ? expandedHeight : COLLAPSED_LIP_PX;
          const queueEl = barMode === 'lip' ? null : document.querySelector('[data-queue-dock]');
          const qr = queueEl ? queueEl.getBoundingClientRect() : null;
          const hasQueue = !!(qr && qr.height > 1);
          let targetHeight = cardTarget;
          if (ir && ir.height > 0 && barMode !== 'lip') {
            const anchorTop = hasQueue ? Math.min(ir.top, qr.top) : ir.top;
            targetHeight = cardTarget + Math.ceil(window.innerHeight - anchorTop);
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
            const neededForPopup = Math.ceil(window.innerHeight - pr.top + 6);
            if (neededForPopup > targetHeight) {
              targetHeight = neededForPopup;
            }
          }

          targetHeight = Math.max(COLLAPSED_LIP_PX, Math.min(880, Math.ceil(targetHeight)));
          sendBarReport({
            kind: 'layout',
            targetHeight,
            width: Math.round(barWidth),
            shapes,
          });
        }, [barMode, barWidth, isExpanded, expandedHeight]);

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

        // Match upper card width/position/surface 1:1 with [data-composer-card] (or 100vw in Bar mode)
        useLayoutEffect(() => {
          const dockEl = wrapRef.current;
          if (!dockEl || typeof ResizeObserver === 'undefined') return;
          const findComposer = () => document.querySelector('[data-composer-card]');

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
                if (cs.backgroundColor && cs.backgroundColor !== 'rgba(0, 0, 0, 0)') {
                  cardEl.style.background = cs.backgroundColor;
                }
                cardEl.style.color = cs.color;
                cardEl.style.fontSize = cs.fontSize;
                cardEl.style.fontWeight = cs.fontWeight;
                cardEl.style.fontFamily = cs.fontFamily;
                applyDividerTheme(cardEl, cs.backgroundColor);

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
        }, [barMode, barWidth]);

        useEffect(() => {
          if (isExpanded && scrollRef.current) {
            scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
          }
        }, [isExpanded, entries.length, latestSingleLine]);

        // =====================================================================
        // Bidirectional Live Twin Sync (Focus-aware Session + Same-Session Draft)
        // =====================================================================
        const applyIncomingTwinState = useCallback(
          (msg, isSnapshot = false) => {
            if (!msg || msg.sourceClientId === SELF_CLIENT_ID) return;

            if (IS_BAR) {
              const incomingId = typeof msg.sessionId === 'string' ? msg.sessionId : '';
              const incomingPath = typeof msg.sessionPath === 'string' ? msg.sessionPath : '';
              if (incomingId && incomingId !== sessionId) {
                switchToSession(incomingId, incomingPath);
                return;
              }
              if (!incomingId && incomingPath && incomingPath.startsWith('/') && incomingPath !== window.location.pathname) {
                switchToSession('', incomingPath);
                return;
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

        const cardBorderRadius =
          IS_BAR && barMode === 'lip'
            ? CARD_RADIUS_PX + 'px'
            : CARD_RADIUS_PX + 'px ' + CARD_RADIUS_PX + 'px 0 0';

        return h(
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
                        const snippet = (msg.text || '').replace(/\s+/g, ' ').trim().slice(0, 28);
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
                              opacity: 0.6,
                              fontSize: '12.5px',
                              textAlign: 'center',
                              padding: '28px 12px',
                            },
                          },
                          '暂无历史消息 — 在下方输入框发消息后，对话流将在此实时展开',
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
                                : h(
                                    'div',
                                    {
                                      style: {
                                        width: '100%',
                                        fontSize: '13px',
                                        lineHeight: 1.58,
                                        whiteSpace: 'pre-wrap',
                                        wordBreak: 'break-word',
                                      },
                                    },
                                    item.text,
                                    item.streaming
                                      ? h(
                                          'span',
                                          {
                                            style: {
                                              marginLeft: '6px',
                                              color: '#6ea8fe',
                                              fontSize: '11.5px',
                                            },
                                          },
                                          '● 生成中…',
                                        )
                                      : null,
                                  ),
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
                    latestSingleLine || '暂无消息 · 双击 Ctrl 唤出输入框',
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
        );
      };
    }

    function makeConversationPickerChip(services) {
      const { sessions, workspaces, uiWorkspace, directoryPicker } = services;

      return function ConversationPickerChip(props) {
        if (!IS_BAR) return null;

        const [, setTick] = useState(0);
        const [menuOpen, setMenuOpen] = useState(false);
        const [hoveredWsId, setHoveredWsId] = useState('');
        const [manualPathMode, setManualPathMode] = useState(false);
        const [manualPathValue, setManualPathValue] = useState('');
        const [pickerError, setPickerError] = useState('');
        const [pillMetrics, setPillMetrics] = useState(null);

        const btnRef = useRef(null);
        const menuRef = useRef(null);

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

        const wsSnap = workspaces && workspaces.list ? workspaces.list.getSnapshot() : { items: [], archivedSessionIds: [] };
        const sessSnap = sessions && sessions.list ? sessions.list.getSnapshot() : { ids: [], byId: {} };
        const archivedSet = new Set(Array.isArray(wsSnap.archivedSessionIds) ? wsSnap.archivedSessionIds : []);
        const wsItems = Array.isArray(wsSnap.items) ? wsSnap.items : [];
        const byId = sessSnap.byId || {};

        const activeSessionId =
          (props && props.sessionId ? String(props.sessionId) : '') ||
          dsbRuntime.sessionId ||
          mainSessionIdOfList(sessSnap);

        const activeSession = activeSessionId ? byId[activeSessionId] : null;
        const activeTitle = sessionDisplayTitle(activeSession);

        // Compute grouped workspaces and conversations
        const { groups, activeWsId } = useMemo(() => {
          const accounted = new Set();
          let foundWsId = '';
          const result = [];

          for (const ws of wsItems) {
            if (!ws || !ws.workspaceId) continue;
            const convs = [];
            const ids = Array.isArray(ws.sessionIds) ? ws.sessionIds : [];
            for (const id of ids) {
              accounted.add(id);
              if (archivedSet.has(id) && id !== activeSessionId) continue;
              const s = byId[id];
              if (!s) continue;
              if (id === activeSessionId) foundWsId = ws.workspaceId;
              convs.push({
                id,
                title: sessionDisplayTitle(s),
                updatedAt: s.updatedAt || 0,
              });
            }
            convs.sort((a, b) => Number(b.updatedAt || 0) - Number(a.updatedAt || 0));
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
            if (archivedSet.has(id) && id !== activeSessionId) continue;
            const s = byId[id];
            if (!s) continue;
            if (id === activeSessionId) foundWsId = '__ungrouped__';
            stray.push({
              id,
              title: sessionDisplayTitle(s),
              updatedAt: s.updatedAt || 0,
            });
          }
          if (stray.length > 0) {
            stray.sort((a, b) => Number(b.updatedAt || 0) - Number(a.updatedAt || 0));
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
        }, [wsItems, sessSnap, activeSessionId]);

        useEffect(() => {
          if (menuOpen) {
            setHoveredWsId(activeWsId || (groups[0] ? groups[0].workspaceId : ''));
            setManualPathMode(false);
            setPickerError('');
          }
        }, [menuOpen, activeWsId]);

        // Outside click / Esc handling for popup
        useEffect(() => {
          if (!menuOpen) return undefined;
          const onPointerDown = (e) => {
            const t = e.target;
            if (btnRef.current && t instanceof Node && btnRef.current.contains(t)) return;
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
          document.addEventListener('pointerdown', onPointerDown, true);
          document.addEventListener('keydown', onKeyDown, true);
          return () => {
            document.removeEventListener('pointerdown', onPointerDown, true);
            document.removeEventListener('keydown', onKeyDown, true);
          };
        }, [menuOpen]);

        const handleOpenSession = (id) => {
          setMenuOpen(false);
          writeStoredString(STORAGE_KEY_LAST_SESSION, id);
          if (uiWorkspace && typeof uiWorkspace.openSession === 'function') {
            try {
              uiWorkspace.openSession(id);
              return;
            } catch {}
          }
          window.location.replace('/sessions/' + encodeURIComponent(id) + '?dsh_bar_clone=1');
        };

        const handleNewConversation = async (wsId) => {
          setMenuOpen(false);
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
          if (!pickedPath || !workspaces || typeof workspaces.create !== 'function') return;
          const ws = await workspaces.create({ path: pickedPath });
          const wsId = ws && ws.workspaceId ? ws.workspaceId : '';
          setMenuOpen(false);
          if (wsId && uiWorkspace && typeof uiWorkspace.startSession === 'function') {
            await uiWorkspace.startSession(wsId);
          }
        };

        const handleCreateWorkspace = async () => {
          setPickerError('');
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

        const renderMenu = () => {
          if (!menuOpen) return null;
          const btnRect = btnRef.current ? btnRef.current.getBoundingClientRect() : null;
          const bottomPx = btnRect ? Math.max(44, Math.round(window.innerHeight - btnRect.top + 8)) : 52;
          const menuMaxW = 452;
          const leftPx = btnRect
            ? Math.max(8, Math.min(Math.round(btnRect.left), Math.max(8, window.innerWidth - menuMaxW - 8)))
            : 12;

          const menuNode = h(
            'div',
            {
              ref: menuRef,
              'data-dsb-popup': '1',
              style: {
                position: 'fixed',
                bottom: bottomPx + 'px',
                left: leftPx + 'px',
                display: 'flex',
                alignItems: 'flex-end',
                gap: '6px',
                zIndex: 9999,
                pointerEvents: 'auto',
                color: 'var(--dsw-alias-label-primary, #f3f4f6)',
                fontFamily: 'system-ui, -apple-system, "Segoe UI", Roboto, "PingFang SC", sans-serif',
              },
            },
            // Level 1: Workspaces
            h(
              'div',
              {
                style: {
                  width: '196px',
                  maxHeight: '300px',
                  borderRadius: '14px',
                  padding: '6px',
                  background: 'var(--dsw-alias-bg-layer-2, #27272a)',
                  border: '1px solid var(--dsw-alias-border-l2, rgba(128,128,128,0.22))',
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
                        border: '1px solid rgba(96,165,250,0.5)',
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
                          style: { fontSize: '11.5px', padding: '2px 8px', borderRadius: '6px', background: 'rgba(59,130,246,0.25)', color: '#60a5fa', border: '1px solid rgba(59,130,246,0.45)', cursor: 'pointer' },
                        },
                        '创建',
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
                        color: '#60a5fa',
                        background: 'rgba(59,130,246,0.12)',
                        border: 'none',
                        cursor: 'pointer',
                        display: 'flex',
                        alignItems: 'center',
                        gap: '6px',
                      },
                    },
                    h('span', null, '＋'),
                    h('span', null, '新建工作区…'),
                  ),
              pickerError
                ? h('div', { style: { fontSize: '11px', color: '#f87171', padding: '2px 8px' } }, pickerError)
                : null,
              h('div', { style: { height: '1px', margin: '4px 2px', background: 'var(--dsw-alias-border-l2, rgba(128,128,128,0.16))' } }),
              h(
                'div',
                {
                  'data-dsb-menu-scroll': '1',
                  style: { overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: '2px', maxHeight: '220px' },
                },
                groups.map((g) => {
                  const isHover = hoveredGroup && g.workspaceId === hoveredGroup.workspaceId;
                  const isActiveWs = g.workspaceId === activeWsId;
                  return h(
                    'button',
                    {
                      key: g.workspaceId,
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

            // Level 2: Conversations in hovered workspace
            hoveredGroup
              ? h(
                  'div',
                  {
                    style: {
                      width: '248px',
                      maxHeight: '300px',
                      borderRadius: '14px',
                      padding: '6px',
                      background: 'var(--dsw-alias-bg-layer-2, #27272a)',
                      border: '1px solid var(--dsw-alias-border-l2, rgba(128,128,128,0.22))',
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
                          onClick: () => handleNewConversation(hoveredGroup.workspaceId),
                          style: {
                            width: '100%',
                            textAlign: 'left',
                            padding: '6px 10px',
                            borderRadius: '10px',
                            fontSize: '12px',
                            fontWeight: 500,
                            color: '#60a5fa',
                            background: 'rgba(59,130,246,0.12)',
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
                    ? h('div', { style: { height: '1px', margin: '4px 2px', background: 'var(--dsw-alias-border-l2, rgba(128,128,128,0.16))' } })
                    : null,
                  h(
                    'div',
                    {
                      'data-dsb-menu-scroll': '1',
                      style: { overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: '2px', maxHeight: '236px' },
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
                              onClick: () => handleOpenSession(c.id),
                              style: {
                                width: '100%',
                                textAlign: 'left',
                                padding: '6px 10px',
                                borderRadius: '10px',
                                fontSize: '12px',
                                background: isSel ? 'rgba(59,130,246,0.18)' : 'transparent',
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
                              h('span', { style: { width: '12px', textAlign: 'center', color: '#60a5fa', flexShrink: 0 } }, isSel ? '✓' : ''),
                              h('span', { style: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' } }, c.title),
                            ),
                            h('span', { style: { fontSize: '11.5px', opacity: 0.55, flexShrink: 0 } }, formatRelativeTime(c.updatedAt)),
                          );
                        }),
                  ),
                )
              : null,
          );

          if (ReactDOM && typeof ReactDOM.createPortal === 'function' && typeof document !== 'undefined' && document.body) {
            return ReactDOM.createPortal(menuNode, document.body);
          }
          return menuNode;
        };

        return h(
          React.Fragment,
          null,
          h(
            'button',
            {
              ref: btnRef,
              type: 'button',
              'data-dsb-session-chip': '1',
              title: '切换绑定对话或工作区',
              onClick: (e) => {
                e.stopPropagation();
                setMenuOpen((prev) => !prev);
              },
              style: {
                height: (pillMetrics && pillMetrics.height) || '28px',
                borderRadius: (pillMetrics && pillMetrics.borderRadius) || '9999px',
                fontSize: (pillMetrics && pillMetrics.fontSize) || '12px',
                background: (pillMetrics && pillMetrics.background) || 'var(--dsw-alias-bg-layer-3, rgba(128,128,128,0.12))',
                color: (pillMetrics && pillMetrics.color) || 'var(--dsw-alias-label-primary, inherit)',
                border: (pillMetrics && pillMetrics.border) || '1px solid var(--dsw-alias-border-l2, rgba(128,128,128,0.22))',
                padding: '0 10px',
                display: 'inline-flex',
                alignItems: 'center',
                gap: '5px',
                maxWidth: '220px',
                cursor: 'pointer',
                flexShrink: 0,
                boxSizing: 'border-box',
              },
            },
            renderFolderIcon(14),
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
            h('span', { style: { opacity: 0.65, flexShrink: 0 } }, '▾'),
          ),
          renderMenu(),
        );
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

        // In Bar mode on root without an open session, restore last bound session once sessions load
        if (IS_BAR && sessions && sessions.list && uiWorkspace) {
          let restored = false;
          const tryRestore = () => {
            if (restored) return;
            const snap = sessions.list.getSnapshot();
            if (!snap || snap.phase === 'pending') return;
            const currentId = mainSessionIdOfList(snap);
            if (currentId) {
              restored = true;
              dsbRuntime.sessionId = currentId;
              writeStoredString(STORAGE_KEY_LAST_SESSION, currentId);
              return;
            }
            const lastId = readStoredString(STORAGE_KEY_LAST_SESSION, '');
            const targetId =
              (lastId && snap.byId && snap.byId[lastId] ? lastId : '') ||
              (Array.isArray(snap.ids) && snap.ids.length > 0 ? String(snap.ids[0]) : '');
            if (targetId) {
              restored = true;
              try {
                uiWorkspace.openSession(targetId);
              } catch {}
            }
          };
          tryRestore();
          if (!restored && typeof sessions.list.subscribe === 'function') {
            const offRestore = sessions.list.subscribe(() => {
              tryRestore();
              if (restored) {
                try { offRestore(); } catch {}
              }
            });
          }
        }

        if (!slots) return;
        const services = { sessions, workspaces, uiWorkspace, directoryPicker };
        const CardholderSlipDock = makeCardDockComponent(services);
        const ConversationPickerChip = makeConversationPickerChip(services);

        slots.inject('conversation.input.dock', () =>
          slots.register(
            {
              name: 'conversation.input.dock',
              id: 'cardholder-slip',
              order: 6,
            },
            CardholderSlipDock,
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
      },
    };
  },
});
