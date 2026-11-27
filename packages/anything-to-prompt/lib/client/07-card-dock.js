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
    const [sideWidth, setSideWidth] = useState(() => dsbRuntime.sideWidth || DEFAULT_SIDE_WIDTH_PX);
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
    const [dockMenu, setDockMenu] = useState(null);
    const [dragSnapEdge, setDragSnapEdge] = useState('none');
    const [sideComposerHeight, setSideComposerHeight] = useState(112);
    const [mentionQuery, setMentionQuery] = useState('');
    const [mentionHits, setMentionHits] = useState([]);
    const mentionSeqRef = useRef(0);
    const hoverPeekTimerRef = useRef(null);
    const hoverPeekCloseTimerRef = useRef(null);
    const hoverPeekGuardUntilRef = useRef(0);
    const lastKnownAsstRef = useRef({ sessionKey: '', asstId: '' });
    const [expandedHeight, setExpandedHeight] = useState(() =>
      readStoredNumber(STORAGE_KEY_HEIGHT, DEFAULT_EXPANDED_PX, MIN_EXPANDED_PX, MAX_EXPANDED_PX),
    );
    const [matchedCardWidth, setMatchedCardWidth] = useState(null);
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
            swEntityKeys: collectActiveSwEntityKeys(),
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
        dismissNativeTriggerMenu();
        setComposerPopover(null);
        setDockMenu(null);
        setDragSnapEdge('none');
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

    const dockEdge = IS_BAR ? (dsbRuntime.dockEdge || 'none') : 'none';
    const dockCollapsed = IS_BAR ? Boolean(dsbRuntime.dockCollapsed) : false;
    const verticalMode = IS_BAR ? Boolean(dsbRuntime.verticalMode) : false;
    const isTopDock = dockEdge === 'top' && !verticalMode;
    const isSideDock = dockEdge === 'left' || dockEdge === 'right' || verticalMode;

    const isExpanded = IS_BAR
      ? isSideDock
        ? true
        : !dsbRuntime.popupOpen && (pinned || historyOpen || previewExpanded)
      : !dsbRuntime.popupOpen && (pinned || hovered || previewExpanded);

    // Close right-click quick dock menu on outside click or Esc
    useEffect(() => {
      if (!dockMenu || typeof document === 'undefined') return undefined;
      const onDown = (e) => {
        const target = e.target;
        if (target instanceof Element && target.closest('[data-dsb-dock-menu="1"]')) return;
        setDockMenu(null);
      };
      const onKey = (e) => {
        if (e.key === 'Escape') {
          e.preventDefault();
          e.stopPropagation();
          setDockMenu(null);
        }
      };
      document.addEventListener('mousedown', onDown, true);
      document.addEventListener('keydown', onKey, true);
      return () => {
        document.removeEventListener('mousedown', onDown, true);
        document.removeEventListener('keydown', onKey, true);
      };
    }, [dockMenu]);

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

    // Track unread replies arriving ONLY on the currently selected Harness & Conversation
    useEffect(() => {
      if (!IS_BAR) return;
      const sKey = activeHarness + ':' + effectiveSessionId;
      const curId = latestAssistantHasText ? String(latestAssistant.id) : '';
      const prev = lastKnownAsstRef.current;
      if (prev.sessionKey !== sKey) {
        lastKnownAsstRef.current = { sessionKey: sKey, asstId: curId };
        if (dsbRuntime.unreadCount || dsbRuntime.unreadPreview || dsbRuntime.pendingBubbleTarget) {
          dsbRuntime.unreadCount = 0;
          dsbRuntime.unreadPreview = '';
          dsbRuntime.pendingBubbleTarget = null;
          notifyBarRuntime();
          if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
            dsbRuntime.scheduleLayoutReport(220);
          }
        }
        return;
      }
      if (curId && prev.asstId && curId !== prev.asstId) {
        lastKnownAsstRef.current = { sessionKey: sKey, asstId: curId };
        if (barMode !== 'lip' && (!isExpanded || dsbRuntime.dockCollapsed)) {
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
    }, [activeHarness, effectiveSessionId, latestAssistantHasText ? String(latestAssistant.id) : '', barMode, isExpanded, dockCollapsed]);

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
          popupOpen: dsbRuntime.popupOpen || dsbRuntime.quickSearchOpen || Boolean(dockMenu) || Boolean(composerPopover),
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
    }, [hovered, pinned, dockMenu, composerPopover]);

    // Esc keydown capture, native + trigger-menu toggle/dismiss, & composer pointerdown in Bar mode
    useEffect(() => {
      if (!IS_BAR || typeof document === 'undefined') return undefined;
      const onKeyDownCapture = (e) => {
        if (e.key !== 'Escape' || e.isComposing) return;
        // Always allow Esc to close DSH's native '+' / trigger menu even when focus sits on the '+' button
        if (document.querySelector('[data-trigger-menu]')) {
          e.preventDefault();
          e.stopPropagation();
          dismissNativeTriggerMenu();
          return;
        }
        const hasNativeMenu = Boolean(
          document.querySelector(
            '[data-composer-overlay], [data-conversation-composer-overlay], [role="menu"], [role="listbox"], [role="dialog"], [role="alertdialog"], [aria-modal="true"], [id$="-menu"], body > [role="group"], body > [style*="position-anchor"], [data-radix-popper-content-wrapper], [data-floating-ui-portal]',
          ),
        );
        if (dsbRuntime.popupOpen || dsbRuntime.quickSearchOpen || dsbRuntime.modalOpen || hasNativeMenu) return;
        e.preventDefault();
        e.stopPropagation();
        sendBarReport({ kind: 'step_down' });
      };
      const onClickCapture = (e) => {
        if (dsbRuntime.activeHarness !== 'dsh') return;
        if (!document.querySelector('[data-trigger-menu]')) return;
        const target = e.target;
        if (!(target instanceof Element)) return;
        if (target.closest('[data-trigger-menu]')) return;
        // Re-clicking the native '+' button always toggles the open trigger menu closed
        // (works around DSH's InputTriggerController.track() clearing `launcher` to null after open)
        const plusBtn = target.closest('[data-composer-card] button[aria-haspopup="listbox"]');
        if (plusBtn && !plusBtn.hasAttribute('data-dsb-session-chip')) {
          e.preventDefault();
          e.stopPropagation();
          if (typeof e.stopImmediatePropagation === 'function') {
            e.stopImmediatePropagation();
          }
          dismissNativeTriggerMenu();
        }
      };
      const onPointerDownCapture = (e) => {
        const target = e.target;
        const composerEl =
          dsbRuntime.activeHarness !== 'dsh'
            ? document.querySelector('[data-dsb-external-composer]')
            : document.querySelector('[data-composer-card]');
        if (composerEl && target instanceof Node && composerEl.contains(target)) {
          sendBarReport({ kind: 'composer_pointerdown' });
          // Clicking the composer editor or other toolbar buttons while '+' menu is open
          // (when not actively typing a / or @ trigger token) cleanly dismisses the '+' menu.
          if (
            dsbRuntime.activeHarness === 'dsh' &&
            document.querySelector('[data-trigger-menu]') &&
            target instanceof Element &&
            !target.closest('[data-trigger-menu]') &&
            !target.closest('button[aria-haspopup="listbox"]') &&
            !/(?:^|\s)[/@][^\s]*$/.test(String(dsbRuntime.liveDraft || ''))
          ) {
            dismissNativeTriggerMenu();
          }
        }
      };
      document.addEventListener('keydown', onKeyDownCapture, true);
      document.addEventListener('click', onClickCapture, true);
      document.addEventListener('pointerdown', onPointerDownCapture, true);
      return () => {
        document.removeEventListener('keydown', onKeyDownCapture, true);
        document.removeEventListener('click', onClickCapture, true);
        document.removeEventListener('pointerdown', onPointerDownCapture, true);
      };
    }, []);

    // Measure and report Bar layout + SEPARATE detached Win32 rounded-rect shapes to Tauri
    const measureAndReportLayout = useCallback(() => {
      if (!IS_BAR || typeof document === 'undefined') return;
      const liveDock = dsbRuntime.dockEdge || 'none';
      const liveCollapsed = Boolean(dsbRuntime.dockCollapsed);
      const liveVertical = liveDock === 'left' || liveDock === 'right' || Boolean(dsbRuntime.verticalMode);

      // Never report mid-flight transformed rects during a 220ms GPU dock glide,
      // and let Rust own the 0x0 / 9dip collapsed RGN for Left / Right / Top!
      if (dsbRuntime.dockAnimUntil && performance.now() < dsbRuntime.dockAnimUntil) {
        return;
      }
      if (liveCollapsed && (liveDock === 'left' || liveDock === 'right' || liveDock === 'top')) {
        return;
      }

      // Bottom-Dock Collapsed: expose ONLY the clean minimal handle pill (zero card header / zero colored stripe!)
      if (liveDock === 'bottom' && liveCollapsed) {
        const handleEl = document.querySelector('[data-dsb-bottom-handle="1"]');
        const hr = handleEl ? handleEl.getBoundingClientRect() : null;
        const stageH = liveVertical
          ? Math.max(420, Math.round(window.innerHeight || BAR_STAGE_HEIGHT_DIP))
          : BAR_STAGE_HEIGHT_DIP;
        const handleShape =
          hr && hr.width > 8 && hr.height > 4
            ? {
                x: Math.round(hr.left),
                y: Math.round(hr.top),
                w: Math.round(hr.width),
                h: Math.round(hr.height),
                r: 8,
              }
            : {
                x: Math.round(((window.innerWidth || barWidth) - 68) / 2),
                y: Math.max(0, Math.round((window.innerHeight || stageH) - 16)),
                w: 68,
                h: 14,
                r: 8,
              };
        sendBarReport({
          kind: 'layout',
          targetHeight: stageH,
          width: liveVertical ? Math.round(dsbRuntime.sideWidth || DEFAULT_SIDE_WIDTH_PX) : Math.round(barWidth),
          shapes: [handleShape],
        });
        return;
      }

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

      // 3. Detached Floating History Sheet / Lip Pill (skip hidden pre-mounted card!)
      const cardHidden = cardEl && cardEl.getAttribute('data-dsb-card-hidden') === '1';
      if (!cardHidden && cr && cr.width > 10 && cr.height > 10) {
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

      // 5. All open popups, submenus, modals, tooltips, and goal/command overlays across ATP & DSH
      const popupEls = document.querySelectorAll(
        '[data-dsb-popup], [data-trigger-menu], [data-goal-bar], [data-composer-overlay]:not(:has([data-trigger-menu])), [data-conversation-composer-overlay]:not(:has([data-trigger-menu])), [role="menu"], [role="listbox"], [role="dialog"], [role="alertdialog"], [aria-modal="true"], [role="tooltip"], [id$="-menu"], body > [role="group"], body > [style*="position-anchor"], [data-radix-popper-content-wrapper], [data-floating-ui-portal], [data-popover], [data-dsw-popover]',
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

      // Keep the Bar's Win32 backing stage at a constant height (`BAR_STAGE_HEIGHT_DIP`, 680dip)
      // so clicking ANY button (+, permissions, models, harness switcher, hover-peek, history)
      // NEVER triggers Win32 SetWindowPos / WM_SIZE / WebView2 swapchain resize (zero flash!).
      // Only `SetWindowRgn` updates the exact rounded-rect clip/hit-test shapes in <0.1ms.
      const liveSideDock = liveDock === 'left' || liveDock === 'right' || Boolean(dsbRuntime.verticalMode);
      const liveTopDock = liveDock === 'top' && !liveSideDock;
      if (liveSideDock) {
        targetHeight = Math.max(420, Math.round(window.innerHeight || BAR_STAGE_HEIGHT_DIP));
        // Vertical-panel safety net: ensure SetWindowRgn never clips off the upper history panel
        // during initial mount/resize before `[data-dsb-card]` settles.
        const hasTallCard = shapes.some((s) => s.y <= 12 && s.h >= 160);
        if (!hasTallCard) {
          const stageW = Math.round(window.innerWidth || 960);
          const panelW = Math.min(stageW, Math.round(dsbRuntime.sideWidth || DEFAULT_SIDE_WIDTH_PX));
          const panelX =
            liveDock === 'left'
              ? 0
              : liveDock === 'right'
                ? Math.max(0, stageW - panelW)
                : Math.max(0, Math.round((stageW - panelW) / 2));
          // Prepend the vertical host panel rect WITHOUT wiping open popup shapes!
          shapes.unshift({
            x: panelX,
            y: 0,
            w: panelW,
            h: targetHeight,
            r: 16,
          });
        }
      } else {
        if (IS_BAR && liveMode !== 'lip') {
          targetHeight = Math.max(BAR_STAGE_HEIGHT_DIP, Math.round(window.innerHeight || BAR_STAGE_HEIGHT_DIP));
        } else {
          targetHeight = Math.max(COLLAPSED_LIP_PX, Math.min(880, Math.ceil(targetHeight)));
        }
      }
      // Constant-stage HWND height NEVER changes in Bar mode, so DOM rects are already in exact HWND coords!
      const deltaY = IS_BAR && liveMode !== 'lip' ? 0 : liveSideDock || liveTopDock ? 0 : Math.round(targetHeight - window.innerHeight);
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
        width: liveSideDock ? Math.round(dsbRuntime.sideWidth || DEFAULT_SIDE_WIDTH_PX) : Math.round(barWidth),
        shapes: adjustedShapes,
      });
    }, [barMode, barWidth, sideWidth, isExpanded, expandedHeight, activeHarness, hoverPeek, composerPopover, dockEdge, dockCollapsed, verticalMode, dockMenu, dragSnapEdge]);

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
          syncPanelWidthVar();
          wrap.style.width = '100%';
          wrap.style.maxWidth = '100%';
          wrap.style.minWidth = '0px';
          wrap.style.transform = 'none';
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
          if (composerCard) {
            const rect = composerCard.getBoundingClientRect();
            if (rect.height > 20) {
              const roundedH = Math.round(rect.height);
              setSideComposerHeight((prev) => (Math.abs(prev - roundedH) > 1 ? roundedH : prev));
            }
          }
          if (composerCard && floatAnchorEl) {
            const liveDock = dsbRuntime.dockEdge || 'none';
            const liveSide = liveDock === 'left' || liveDock === 'right' || Boolean(dsbRuntime.verticalMode);
            const liveTop = liveDock === 'top' && !liveSide;
            if (liveSide) {
              // Side-panel card spans top: 0 down to just above the bottom composer.
              // Cap composer reservation to <= 45% of viewport height so the history stream
              // and top drag bar are ALWAYS generously visible and draggable.
              const compRect = composerCard.getBoundingClientRect();
              const rawH = compRect.height > 20 ? Math.round(compRect.height) : sideComposerHeight;
              const maxCompH = Math.max(140, Math.round((window.innerHeight || 680) * 0.45));
              const compH = Math.min(Math.max(84, rawH), maxCompH);
              floatAnchorEl.style.top = '0px';
              floatAnchorEl.style.bottom = (compH + 8) + 'px';
              floatAnchorEl.style.height = 'auto';
            } else if (barMode !== 'lip') {
              const rect = composerCard.getBoundingClientRect();
              const qr = readQueueDockRect();
              const wr = wrap.getBoundingClientRect();
              if (liveTop) {
                const gapTop = Math.round((rect.bottom - wr.top + DETACHED_GAP_PX) * 100) / 100;
                floatAnchorEl.style.bottom = 'auto';
                floatAnchorEl.style.top = gapTop + 'px';
              } else {
                const anchorTop = qr ? Math.min(rect.top, qr.top) : rect.top;
                const gap = Math.round((anchorTop - wr.bottom) * 100) / 100;
                floatAnchorEl.style.top = 'auto';
                floatAnchorEl.style.bottom = (-gap + DETACHED_GAP_PX) + 'px';
              }
            } else {
              floatAnchorEl.style.top = 'auto';
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
      let syncRafId = 0;
      const rafTick = (frames) => {
        syncWidth();
        if (frames > 0) {
          syncRafId = requestAnimationFrame(() => rafTick(frames - 1));
        } else {
          syncRafId = 0;
        }
      };
      rafTick(12);
      window.addEventListener('resize', syncWidth);
      return () => {
        if (syncRafId) cancelAnimationFrame(syncRafId);
        ro.disconnect();
        window.removeEventListener('resize', syncWidth);
      };
    }, [barMode, barWidth, activeHarness, dockEdge, verticalMode]);

    useEffect(() => {
      if (!isExpanded) return undefined;
      const scrollToBottom = () => {
        if (scrollRef.current) {
          scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
        }
      };
      scrollToBottom();
      let raf2 = 0;
      const raf1 = requestAnimationFrame(() => {
        scrollToBottom();
        raf2 = requestAnimationFrame(scrollToBottom);
      });
      if (IS_BAR) {
        refreshHarnessAvailability();
      }
      return () => {
        cancelAnimationFrame(raf1);
        if (raf2) cancelAnimationFrame(raf2);
      };
    }, [isExpanded, entries.length, latestSingleLine, dockEdge, activeHarness]);

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

        if (!IS_BAR && typeof msg.pinned === 'boolean' && msg.pinned !== pinned) {
          setPinned(msg.pinned);
        }
      },
      [inputActions, liveDraft, pinned, sessionId],
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
      if (isResizingHeightRef.current || isResizingWidthRef.current) return;
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

        const finish = (clientY) => {
          isResizingHeightRef.current = false;
          const deltaY = startY - clientY;
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
          window.removeEventListener('blur', onBlur);
        };
        const onUp = (upEv) => finish(upEv.clientY);
        const onBlur = () => finish(startY);

        window.addEventListener('mousemove', onMove);
        window.addEventListener('mouseup', onUp);
        window.addEventListener('blur', onBlur);
      },
      [expandedHeight],
    );

    const handleStartResizeWidth = useCallback(
      (side) => (e) => {
        if (!IS_BAR) return;
        e.preventDefault();
        e.stopPropagation();
        isResizingWidthRef.current = true;
        if (typeof document !== 'undefined' && document.documentElement) {
          document.documentElement.setAttribute('data-dsb-resizing', '1');
        }
        const startScreenX = e.screenX;
        const liveDock = dsbRuntime.dockEdge || 'none';
        const resizingVert = liveDock === 'left' || liveDock === 'right' || Boolean(dsbRuntime.verticalMode);
        const isEdgePinned = liveDock === 'left' || liveDock === 'right';
        const scaleFactor = isEdgePinned ? 1 : 2;
        const startW = resizingVert
          ? dsbRuntime.sideWidth || DEFAULT_SIDE_WIDTH_PX
          : dsbRuntime.barWidth;
        const minW = resizingVert ? MIN_SIDE_WIDTH_PX : MIN_BAR_WIDTH_PX;
        const maxW = resizingVert ? MAX_SIDE_WIDTH_PX : MAX_BAR_WIDTH_PX;

        const applyW = (rawDelta, persist) => {
          const signedDelta = side === 'left' ? -rawDelta : rawDelta;
          const nextW = Math.max(minW, Math.min(maxW, Math.round(startW + signedDelta * scaleFactor)));
          if (resizingVert) {
            dsbRuntime.sideWidth = nextW;
            setSideWidth(nextW);
            if (persist) {
              try {
                localStorage.setItem(STORAGE_KEY_SIDE_WIDTH, String(nextW));
              } catch {}
            }
          } else {
            dsbRuntime.barWidth = nextW;
            setBarWidth(nextW);
            if (persist) {
              try {
                localStorage.setItem(STORAGE_KEY_BAR_WIDTH, String(nextW));
              } catch {}
              postTwinSync({ widthPx: nextW });
            }
          }
          syncPanelWidthVar();
          scheduleLayoutReport(persist ? 120 : 80);
        };

        const onMove = (moveEv) => applyW(moveEv.screenX - startScreenX, false);
        const finishW = (delta) => {
          isResizingWidthRef.current = false;
          if (typeof document !== 'undefined' && document.documentElement) {
            document.documentElement.removeAttribute('data-dsb-resizing');
          }
          if (typeof delta === 'number') applyW(delta, true);
          window.removeEventListener('mousemove', onMove);
          window.removeEventListener('mouseup', onUp);
          window.removeEventListener('blur', onBlur);
        };
        const onUp = (upEv) => finishW(upEv.screenX - startScreenX);
        const onBlur = () => finishW(null);

        window.addEventListener('mousemove', onMove);
        window.addEventListener('mouseup', onUp);
        window.addEventListener('blur', onBlur);
      },
      [scheduleLayoutReport],
    );

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
        setDockMenu(null);
        const startX = e.screenX;
        const startY = e.screenY;
        let dragged = false;
        let dragStarted = false;
        let active = true;
        let rafId = 0;
        const step = () => {
          rafId = 0;
          if (!active || !dragStarted) return;
          tauriInvoke('bar_drag_step')
            .then((res) => {
              if (!active) return;
              const nextSnap =
                res && typeof res.snapPreview === 'string'
                  ? res.snapPreview
                  : res && typeof res.snapEdge === 'string'
                    ? res.snapEdge
                    : 'none';
              setDragSnapEdge((prev) => (prev === nextSnap ? prev : nextSnap));
            })
            .catch(() => {});
        };
        const onUp = () => {
          if (!active) return;
          active = false;
          if (rafId) cancelAnimationFrame(rafId);
          setDragSnapEdge('none');
          if (dragStarted) {
            tauriInvoke('bar_drag_end')
              .then(() => {
                dsbRuntime.lastReportKeys.delete('layout');
                scheduleLayoutReport(320);
              })
              .catch(() => {});
          } else if (!(dsbRuntime.dockEdge === 'left' || dsbRuntime.dockEdge === 'right')) {
            toggleHistorySheet();
          }
          window.removeEventListener('mousemove', onMove);
          window.removeEventListener('mouseup', onUp);
          window.removeEventListener('blur', onUp);
        };
        const onMove = (mv) => {
          if (!active) return;
          if (typeof mv.buttons === 'number' && (mv.buttons & 1) === 0) {
            onUp();
            return;
          }
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
        window.addEventListener('mousemove', onMove);
        window.addEventListener('mouseup', onUp);
        window.addEventListener('blur', onUp);
      },
      [toggleHistorySheet, scheduleLayoutReport],
    );

    const handleOpenDockMenu = useCallback(
      (e) => {
        if (!IS_BAR) return;
        e.preventDefault();
        e.stopPropagation();
        setComposerPopover(null);
        setHoverPeek(false);
        const rect = e.currentTarget && typeof e.currentTarget.getBoundingClientRect === 'function'
          ? e.currentTarget.getBoundingClientRect()
          : null;
        const x = rect ? Math.round(rect.left + rect.width / 2) : Math.round(e.clientX || 120);
        const y = rect ? Math.round(rect.top) : Math.round(e.clientY || 120);
        setDockMenu({ x, y });
        scheduleLayoutReport(240);
      },
      [scheduleLayoutReport],
    );

    const renderDockQuickMenu = () => {
      if (!IS_BAR || !dockMenu) return null;
      const t = getUnifiedTheme(activeHarness);
      const winW = typeof window !== 'undefined' ? window.innerWidth || 760 : 760;
      const winH = typeof window !== 'undefined' ? window.innerHeight || BAR_STAGE_HEIGHT_DIP : BAR_STAGE_HEIGHT_DIP;
      const menuW = 228;
      const menuH = 196;
      const left = Math.max(8, Math.min(winW - menuW - 8, dockMenu.x - Math.round(menuW / 2)));
      const openDown = isTopDock || dockMenu.y < menuH + 16;
      const top = openDown
        ? Math.max(8, Math.min(winH - menuH - 8, dockMenu.y + 18))
        : Math.max(8, dockMenu.y - menuH - 8);
      const items = [
        { edge: 'top', icon: '⬆️', label: '停靠到屏幕顶部' },
        { edge: 'bottom', icon: '⬇️', label: '停靠到屏幕底部（任务栏上方）' },
        { edge: 'left', icon: '⬅️', label: '停靠到左侧（竖向侧栏）' },
        { edge: 'right', icon: '➡️', label: '停靠到右侧（竖向侧栏）' },
        { edge: 'none', icon: '🪟', label: '取消停靠并复位居中' },
      ];
      const node = h(
        'div',
        {
          'data-dsb-popup': '1',
          'data-dsb-dock-menu': '1',
          style: {
            position: 'fixed',
            left: left + 'px',
            top: top + 'px',
            width: menuW + 'px',
            borderRadius: '13px',
            padding: '6px',
            background: t.popover,
            color: t.fg,
            border: '1px solid ' + t.border,
            boxShadow: t.isDark ? '0 14px 36px rgba(0,0,0,0.56)' : '0 10px 28px rgba(15,23,42,0.16)',
            zIndex: 10000,
            display: 'flex',
            flexDirection: 'column',
            gap: '2px',
            boxSizing: 'border-box',
            fontFamily: '"Segoe UI", "Inter", "PingFang SC", "Microsoft YaHei", system-ui, sans-serif',
          },
        },
        h(
          'div',
          {
            style: {
              fontSize: '10.5px',
              fontWeight: 600,
              color: t.muted,
              padding: '2px 8px 4px',
              borderBottom: '1px solid ' + t.subtleBorder,
              marginBottom: '2px',
            },
          },
          '吸附停靠位置（贴边自动收起）',
        ),
        items.map((it) => {
          const active = dockEdge === it.edge;
          return h(
            'button',
            {
              key: it.edge,
              type: 'button',
              'data-dsb-pop-row': '1',
              onClick: (ev) => {
                ev.stopPropagation();
                setDockMenu(null);
                tauriInvoke('bar_set_dock_edge', { edge: it.edge, collapsed: false }).catch(() => {
                  applyDockState(it.edge, false, it.edge === 'left' || it.edge === 'right');
                });
              },
              style: {
                width: '100%',
                border: 'none',
                borderRadius: '8px',
                padding: '6px 8px',
                background: active ? t.accentBg : 'transparent',
                color: active ? t.accent : t.fg,
                fontSize: '12px',
                fontWeight: active ? 600 : 450,
                cursor: 'pointer',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                gap: '6px',
                textAlign: 'left',
              },
            },
            h(
              'span',
              { style: { display: 'inline-flex', alignItems: 'center', gap: '6px', minWidth: 0 } },
              h('span', { style: { fontSize: '12px', flexShrink: 0 } }, it.icon),
              h('span', { style: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' } }, it.label),
            ),
            active ? h('span', { style: { fontSize: '11px', flexShrink: 0 } }, '✓') : null,
          );
        }),
      );
      if (ReactDOM && typeof ReactDOM.createPortal === 'function' && typeof document !== 'undefined' && document.body) {
        return ReactDOM.createPortal(node, document.body);
      }
      return node;
    };

    const renderSnapPreviewBanner = () => {
      if (!IS_BAR || !dragSnapEdge || dragSnapEdge === 'none') return null;
      const t = getUnifiedTheme(activeHarness);
      const labels = {
        left: '🧲 松手停靠至屏幕左侧 · 切换竖向侧栏',
        right: '🧲 松手停靠至屏幕右侧 · 切换竖向侧栏',
        top: '🧲 松手停靠至屏幕顶部 · 悬停自动滑出',
        bottom: '🧲 松手停靠至屏幕底部 · 贴齐任务栏收起',
      };
      const text = labels[dragSnapEdge];
      if (!text) return null;
      const node = h(
        'div',
        {
          'data-dsb-popup': '1',
          'data-dsb-snap-preview': '1',
          style: {
            position: 'fixed',
            left: '50%',
            transform: 'translateX(-50%)',
            top: isTopDock ? '118px' : 'auto',
            bottom: isTopDock ? 'auto' : Math.max(118, sideComposerHeight + 12) + 'px',
            padding: '6px 14px',
            borderRadius: '999px',
            background: t.popover,
            color: t.accent,
            border: '1px solid ' + t.accentBorder,
            boxShadow: '0 8px 24px rgba(0,0,0,0.42)',
            fontSize: '12px',
            fontWeight: 600,
            whiteSpace: 'nowrap',
            zIndex: 10001,
            pointerEvents: 'none',
            fontFamily: '"Segoe UI", "Inter", "PingFang SC", "Microsoft YaHei", system-ui, sans-serif',
          },
        },
        text,
      );
      if (ReactDOM && typeof ReactDOM.createPortal === 'function' && typeof document !== 'undefined' && document.body) {
        return ReactDOM.createPortal(node, document.body);
      }
      return node;
    };

    const handleResetWindowPos = useCallback((e) => {
      if (!IS_BAR) return;
      e.preventDefault();
      e.stopPropagation();
      tauriInvoke('bar_drag_reset').catch(() => {});
    }, []);

    const handleHandleMouseEnter = useCallback(() => {
      if (hoverPeekCloseTimerRef.current) {
        clearTimeout(hoverPeekCloseTimerRef.current);
        hoverPeekCloseTimerRef.current = null;
      }
      if (!IS_BAR || barMode === 'lip' || isExpanded || (dsbRuntime.unreadCount || 0) > 0) return;
      if (!latestSingleLine) return;
      if (hoverPeek) return;
      if (hoverPeekTimerRef.current) clearTimeout(hoverPeekTimerRef.current);
      hoverPeekTimerRef.current = setTimeout(() => {
        hoverPeekTimerRef.current = null;
        // Guard against transient mouseleave emitted by Chromium during the 1-2 frames
        // when SetWindowPos grows the Win32 Bar upward for the floating peek card.
        hoverPeekGuardUntilRef.current = performance.now() + 450;
        setHoverPeek(true);
        scheduleLayoutReport(100);
      }, 220);
    }, [barMode, isExpanded, hoverPeek, latestSingleLine, scheduleLayoutReport]);

    const handleHandleMouseLeave = useCallback(() => {
      if (hoverPeekTimerRef.current) {
        clearTimeout(hoverPeekTimerRef.current);
        hoverPeekTimerRef.current = null;
      }
      if (hoverPeekCloseTimerRef.current) {
        clearTimeout(hoverPeekCloseTimerRef.current);
        hoverPeekCloseTimerRef.current = null;
      }
      const delayMs = performance.now() < hoverPeekGuardUntilRef.current ? 320 : 180;
      hoverPeekCloseTimerRef.current = setTimeout(() => {
        hoverPeekCloseTimerRef.current = null;
        if (
          typeof document !== 'undefined' &&
          document.querySelector('[data-dsb-drag-handle="1"]:hover, [data-dsb-floating-peek="1"]:hover')
        ) {
          return;
        }
        setHoverPeek((prev) => {
          if (prev) scheduleLayoutReport(100);
          return false;
        });
      }, delayMs);
    }, [scheduleLayoutReport]);

    const renderDragHandle = (extraStyle) => {
      if (!IS_BAR || barMode === 'lip' || isSideDock) return null;
      const t = getUnifiedTheme(activeHarness);
      const dockHint =
        dockEdge !== 'none'
          ? '按住向屏幕内侧拖出可取消停靠 · 右键停靠菜单 · 双击复位居中'
          : isExpanded
            ? '单击收起聊天记录 · 拖至屏幕四边自动吸附停靠 · 右键停靠菜单 · 双击复位'
            : '单击展开聊天记录 · 悬停轻瞥回复 · 拖至屏幕四边吸附停靠 · 右键菜单';
      return h(
        'div',
        {
          'data-dsb-drag-handle': '1',
          title: dockHint,
          onMouseDown: handleStartDragOrToggle,
          onDoubleClick: handleResetWindowPos,
          onContextMenu: handleOpenDockMenu,
          onMouseEnter: handleHandleMouseEnter,
          onMouseLeave: handleHandleMouseLeave,
          style: Object.assign(
            {
              position: 'absolute',
              top: isTopDock ? 'auto' : '1px',
              bottom: isTopDock ? '1px' : 'auto',
              left: '50%',
              transform: 'translateX(-50%)',
              width: '72px',
              height: '13px',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
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
            background:
              dockEdge !== 'none' || isExpanded
                ? t.accent
                : t.isDark
                  ? 'rgba(165, 172, 186, 0.36)'
                  : 'rgba(15, 23, 42, 0.24)',
            opacity: dockEdge !== 'none' || isExpanded ? 0.88 : 1,
            pointerEvents: 'none',
            transition: 'all 0.16s ease',
          },
        }),
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
      const isTopDockQs = dsbRuntime.dockEdge === 'top';
      const topPx = validComp ? Math.round(ir.bottom + 6) : 112;
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
          'data-dsb-popup-bottom': isTopDockQs ? undefined : String(bottomPx),
          'data-dsb-quick-search': '1',
          onMouseDown: (ev) => ev.preventDefault(),
          style: {
            position: 'fixed',
            left: '2px',
            right: '2px',
            top: isTopDockQs ? topPx + 'px' : 'auto',
            bottom: isTopDockQs ? 'auto' : bottomPx + 'px',
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
    const activeSessionSync = dsbRuntime.sessionSyncState[activeHarness] || null;
    const isDesktopReadOnly = Boolean(
      activeSessionSync &&
        activeSessionSync.readOnlyInDesktop &&
        activeSessionSync.sessionId &&
        extSel &&
        String(extSel.sessionId || '') === String(activeSessionSync.sessionId),
    );

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
        let top = 112;
        let fullWidth = false;
        if (kind === 'plus' && dsbRuntime.activeHarness === 'codex') {
          const compEl = document.querySelector('[data-dsb-external-composer="codex"]');
          const cr = compEl ? compEl.getBoundingClientRect() : null;
          left = cr && cr.width > 10 ? Math.max(6, Math.round(cr.left)) : 8;
          bottom = cr && cr.height > 10 ? Math.max(88, Math.round(window.innerHeight - cr.top + 6)) : 126;
          top = cr && cr.height > 10 ? Math.round(cr.bottom + 6) : 112;
          fullWidth = true;
        } else if (anchorEl && typeof anchorEl.getBoundingClientRect === 'function') {
          const r = anchorEl.getBoundingClientRect();
          left = Math.max(8, Math.round(r.left));
          bottom = Math.max(48, Math.round(window.innerHeight - r.top + 10));
          top = Math.max(48, Math.round(r.bottom + 8));
        }
        scheduleLayoutReport(260);
        return { kind, left, bottom, top, fullWidth };
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
      const rollbackSnapshot = {
        draft: extDraft,
        capsules: extCapsules.slice(),
        attachments: attsToSend.slice(),
      };
      dsbRuntime.extDraftText = '';
      dsbRuntime.extCapsules = [];
      dsbRuntime.extAttachments = [];
      setComposerPopover(null);
      sendBarReport({ kind: 'capsules', capsuleCount: 0, swEntityKeys: [], afterInsert: false });
      notifyBarRuntime();
      submitExternalHarnessPrompt(fullText, attsToSend, rollbackSnapshot);
    };

    const removeExtCapsule = (capId) => {
      const removed = (dsbRuntime.extCapsules || []).find((c) => c && c.id === capId);
      if (
        removed &&
        removed.meta &&
        typeof removed.meta.exe === 'string' &&
        removed.meta.exe &&
        typeof removed.insertedAt === 'number' &&
        Date.now() - removed.insertedAt <= 5000
      ) {
        sendBarReport({ kind: 'capsule_rejected', detail: removed.meta.exe });
      }
      dsbRuntime.extCapsules = renumberExtCapsules(
        (dsbRuntime.extCapsules || []).filter((c) => c.id !== capId),
      );
      sendBarReport({
        kind: 'capsules',
        capsuleCount: dsbRuntime.extCapsules.length,
        swEntityKeys: collectActiveSwEntityKeys(),
        afterInsert: false,
      });
      notifyBarRuntime();
      if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
        dsbRuntime.scheduleLayoutReport(220);
      }
    };

    const renderExtCapsulesRow = () => {
      const sendErr = dsbRuntime.externalSendError || '';
      if (extCapsules.length === 0 && !sendErr) return null;
      return h(
        React.Fragment,
        null,
        sendErr
          ? h(
              'div',
              {
                style: {
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: '8px',
                  marginBottom: '6px',
                  padding: '5px 9px',
                  borderRadius: '8px',
                  background: 'rgba(239, 68, 68, 0.14)',
                  border: '1px solid rgba(239, 68, 68, 0.34)',
                  color: '#fca5a5',
                  fontSize: '11.5px',
                  lineHeight: '16px',
                },
              },
              h(
                'span',
                { style: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' } },
                '⚠️ 上次发送失败（已恢复草稿与引用）：' + sendErr,
              ),
              h(
                'button',
                {
                  type: 'button',
                  onClick: () => {
                    dsbRuntime.externalSendError = '';
                    notifyBarRuntime();
                  },
                  style: {
                    border: 'none',
                    background: 'transparent',
                    color: 'inherit',
                    cursor: 'pointer',
                    padding: '0 2px',
                    fontSize: '12px',
                    flexShrink: 0,
                  },
                },
                '×',
              ),
            )
          : null,
        extCapsules.length > 0
          ? h(
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
            )
          : null,
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
        if (isDesktopReadOnly) return;
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
      const formatDynamicModelLabel = (rawId) => {
        const s = String(rawId || '').trim();
        if (!s) return '';
        return s
          .replace(/^gpt-/i, 'GPT-')
          .replace(/^claude-/i, 'Claude ')
          .replace(/^gemini-/i, 'Gemini ')
          .replace(/-/g, ' ')
          .replace(/\b([a-z])/g, (m) => m.toUpperCase());
      };
      const curModel =
        uo.models.find((x) => x.id === curModelId) ||
        (curModelId ? { id: curModelId, label: formatDynamicModelLabel(curModelId) } : uo.models[0]);
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
      const isUniversalMode = m.id === 'universal';
      const univTgt = dsbRuntime.universalTarget;
      const wsLabel = isUniversalMode
        ? (univTgt && univTgt.valid && univTgt.displayLabel ? '🎯 ' + univTgt.displayLabel : '🎯 上一个焦点窗口')
        : (ctx.selection && ctx.selection.workspacePath
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
            width: '100%',
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
        (() => {
          const plusBtn = h(
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
          );
          const modeBtn = h(
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
                padding: isSideDock ? '0 5px' : '0 7px',
                background: ctx.activePopover === 'mode' ? t.accentBg : 'transparent',
                color: isDangerMode ? t.permDanger : t.fg,
                fontSize: isSideDock ? '12px' : '12.5px',
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
          );
          const sessionChip = h(
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
                padding: isSideDock ? '0 6px' : '0 8px',
                background: t.accentBg,
                color: t.accent,
                fontSize: '11.5px',
                fontWeight: 500,
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                gap: '5px',
                maxWidth: isSideDock ? '155px' : '195px',
                minWidth: '28px',
                flex: '0 1 auto',
                overflow: 'hidden',
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
              { style: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 } },
              m.label + ' · ' + wsLabel,
            ),
          );
          const modelCtrl = isUniversalMode
            ? h(
                'span',
                {
                  title: '万能投递模式：发送后将自动切回目标窗口并粘贴',
                  style: {
                    fontSize: '11.5px',
                    color: t.muted,
                    padding: '0 4px',
                    whiteSpace: 'nowrap',
                  },
                },
                '万能投递 · 零侵入',
              )
            : h(
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
                    padding: isSideDock ? '0 4px' : '0 6px',
                    background: ctx.activePopover === 'model' ? t.accentBg : 'transparent',
                    color: t.fg,
                    fontSize: isSideDock ? '12px' : '12.5px',
                    fontWeight: 500,
                    cursor: 'pointer',
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: '4px',
                    whiteSpace: 'nowrap',
                    minWidth: 0,
                    maxWidth: isSideDock ? '168px' : 'none',
                    overflow: 'hidden',
                  },
                },
                h(
                  'span',
                  { style: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 } },
                  (curModel && curModel.label) || '',
                ),
                curEffort && curEffort.label
                  ? h('span', { style: { color: t.muted, fontWeight: 400, flexShrink: 0 } }, curEffort.label)
                  : null,
                HARNESS_SDK_ICONS.chevronDown(t.muted),
              );
          const orientBtn =
            IS_BAR && !isSideDock
              ? h(
                  'button',
                  {
                    type: 'button',
                    'data-dsb-orient-btn': '1',
                    title: '切换为竖向面板',
                    'aria-label': '切换为竖向面板',
                    onClick: (ev) => {
                      ev.stopPropagation();
                      triggerOrientationMorph(true);
                    },
                    style: {
                      width: '24px',
                      height: '24px',
                      borderRadius: '6px',
                      border: '1px solid ' + t.border,
                      background: 'transparent',
                      color: t.muted,
                      cursor: 'pointer',
                      display: 'inline-flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      padding: 0,
                      flexShrink: 0,
                    },
                  },
                  h(
                    'svg',
                    {
                      width: 13,
                      height: 13,
                      viewBox: '0 0 16 16',
                      fill: 'none',
                      stroke: 'currentColor',
                      strokeWidth: 1.5,
                      strokeLinecap: 'round',
                      strokeLinejoin: 'round',
                    },
                    h('rect', { x: 9.5, y: 1.5, width: 5, height: 13, rx: 1.5 }),
                    h('path', { d: 'M2 8h5M5 5.5L7.5 8 5 10.5' }),
                  ),
                )
              : null;
          const sendBtn = h(
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
          );

          // Always ONE crisp single row: Workspace/Conversation pill auto-shrinks cleanly on narrow widths!
          return h(
            'div',
            {
              style: {
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                flexWrap: 'nowrap',
                marginTop: '11px',
                height: '30px',
                gap: isSideDock ? '6px' : '8px',
                width: '100%',
                minWidth: 0,
              },
            },
            h(
              'div',
              {
                style: {
                  display: 'flex',
                  alignItems: 'center',
                  gap: isSideDock ? '5px' : '7px',
                  minWidth: 0,
                  flex: '1 1 auto',
                  overflow: 'hidden',
                },
              },
              plusBtn,
              modeBtn,
              sessionChip,
            ),
            h(
              'div',
              {
                style: {
                  display: 'flex',
                  alignItems: 'center',
                  gap: isSideDock ? '5px' : '8px',
                  flexShrink: 0,
                  minWidth: 0,
                },
              },
              modelCtrl,
              orientBtn,
              sendBtn,
            ),
          );
        })(),
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
        const syncNote =
          activeSessionSync && activeSessionSync.sessionId && extSel && String(extSel.sessionId || '') === String(activeSessionSync.sessionId)
            ? activeSessionSync.sourceNote || ''
            : '';
        content = [
          sectionHdr(
            'm-hdr',
            meta.label + (isDesktopReadOnly ? ' · 权限模式（桌面端同步只读）' : ' · 权限模式（Shift+Tab 循环）'),
            false,
          ),
          syncNote
            ? h(
                'div',
                {
                  key: 'm-sync-note',
                  style: {
                    fontSize: '10.5px',
                    lineHeight: '1.4',
                    color: isDesktopReadOnly ? t.accent : t.muted,
                    background: isDesktopReadOnly ? t.accentBg : 'transparent',
                    borderRadius: '7px',
                    padding: isDesktopReadOnly ? '5px 8px' : '0 10px 5px',
                    marginBottom: '4px',
                  },
                },
                syncNote,
              )
            : null,
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
                if (isDesktopReadOnly) return;
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
        const syncNote =
          activeSessionSync && activeSessionSync.sessionId && extSel && String(extSel.sessionId || '') === String(activeSessionSync.sessionId)
            ? activeSessionSync.sourceNote || ''
            : '';
        const modelList = uo.models.some((m) => m.id === curModel || (curModel === 'gemini-4-neon' && m.id === 'gemini-4-neon-high'))
          ? uo.models
          : (curModel ? [{ id: curModel, label: curModel, tag: '当前会话' }].concat(uo.models) : uo.models);
        content = [
          h(
            'div',
            { key: 'mdl-hdr', style: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '3px 10px 5px' } },
            h(
              'span',
              { style: { fontSize: '11px', fontWeight: 500, color: t.muted } },
              meta.label + (isDesktopReadOnly ? ' · 当前模型（桌面端同步只读）' : ' · 选择模型'),
            ),
            adapter && typeof adapter.renderModelHeaderExtra === 'function'
              ? adapter.renderModelHeaderExtra(harnessPopCtx)
              : null,
          ),
          syncNote
            ? h(
                'div',
                {
                  key: 'mdl-sync-note',
                  style: {
                    fontSize: '10.5px',
                    lineHeight: '1.4',
                    color: isDesktopReadOnly ? t.accent : t.muted,
                    background: isDesktopReadOnly ? t.accentBg : 'transparent',
                    borderRadius: '7px',
                    padding: isDesktopReadOnly ? '5px 8px' : '0 10px 5px',
                    marginBottom: '4px',
                  },
                },
                syncNote,
              )
            : null,
          ...modelList.map((m) => {
            const isSel = m.id === curModel || (curModel === 'gemini-4-neon' && m.id === 'gemini-4-neon-high');
            const tagText = m.tag || (m.desc && m.desc.includes('|') ? m.desc.split('|')[1] : '');
            return popRow(
              'mdl-' + m.id,
              null,
              m.label,
              '',
              isSel,
              () => {
                if (isDesktopReadOnly) return;
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
                    opacity: isDesktopReadOnly ? 0.65 : 1,
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
                          if (isDesktopReadOnly) return;
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
                          cursor: isDesktopReadOnly ? 'default' : 'pointer',
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

      const isTopDockPop = dsbRuntime.dockEdge === 'top';
      const activeCompEl =
        document.querySelector('[data-dsb-external-composer]') ||
        document.querySelector('[data-composer-card]');
      const activeCr = activeCompEl ? activeCompEl.getBoundingClientRect() : null;
      const stageW = window.innerWidth || 760;
      const stageH = window.innerHeight || 680;
      const hostLeft = activeCr && activeCr.width > 10 ? Math.round(activeCr.left) : 8;
      const hostW = activeCr && activeCr.width > 10 ? Math.round(activeCr.width) : stageW - 16;
      const hostRight = hostLeft + hostW;
      const effectivePanelW = composerPopover.fullWidth
        ? Math.max(220, hostW)
        : Math.min(panelWidth, Math.max(220, hostW - 12));
      const clampedLeft = Math.max(
        Math.max(8, hostLeft + 4),
        Math.min(Math.min(stageW - effectivePanelW - 8, hostRight - effectivePanelW - 4), composerPopover.left),
      );
      const popMaxH = Math.min(
        360,
        Math.max(
          180,
          isTopDockPop
            ? stageH - (composerPopover.top || 112) - 10
            : stageH - (composerPopover.bottom || 92) - 10,
        ),
      );
      const pop = h(
        'div',
        {
          'data-dsb-popup': '1',
          'data-dsb-composer-popover': '1',
          'data-dsb-popup-bottom': isTopDockPop ? undefined : String(composerPopover.bottom),
          style: {
            position: 'fixed',
            left: clampedLeft + 'px',
            top: isTopDockPop ? (composerPopover.top || 112) + 'px' : 'auto',
            bottom: isTopDockPop ? 'auto' : composerPopover.bottom + 'px',
            width: effectivePanelW + 'px',
            maxHeight: popMaxH + 'px',
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
          width: '100%',
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
              ? '100%'
              : matchedCardWidth
                ? matchedCardWidth + 'px'
                : '100%',
            maxWidth: IS_BAR
              ? '100%'
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
              'data-dsb-width-resize': 'left',
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
              'data-dsb-width-resize': 'right',
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

      // Clean minimal handle-only pill for Bottom-Edge Dock (always mounted when Bottom-docked; CSS fades opacity so expand/collapse causes ZERO React re-render!)
      IS_BAR && dockEdge === 'bottom'
        ? h(
            'div',
            {
              'data-dsb-bottom-handle': '1',
              title: '点击或悬停唤出输入框',
              onClick: (ev) => {
                ev.stopPropagation();
                tauriInvoke('bar_dock_expand').catch(() => {});
              },
              style: {
                position: 'fixed',
                left: '50%',
                bottom: (dsbRuntime.unreadCount || 0) > 0 ? '4px' : '2px',
                transform: 'translateX(-50%)',
                width: (dsbRuntime.unreadCount || 0) > 0 ? '74px' : '64px',
                height: '14px',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                cursor: 'pointer',
                pointerEvents: 'auto',
                zIndex: 99,
              },
            },
            h('span', {
              style: {
                width: (dsbRuntime.unreadCount || 0) > 0 ? '60px' : '50px',
                height: (dsbRuntime.unreadCount || 0) > 0 ? '6px' : '5px',
                borderRadius: '999px',
                background:
                  (dsbRuntime.unreadCount || 0) > 0
                    ? (HARNESS_META[activeHarness] || HARNESS_META.dsh).accent
                    : 'rgba(148, 163, 184, 0.62)',
                boxShadow:
                  (dsbRuntime.unreadCount || 0) > 0
                    ? '0 0 12px ' + (HARNESS_META[activeHarness] || HARNESS_META.dsh).accent
                    : '0 1px 6px rgba(0, 0, 0, 0.35)',
                animation:
                  (dsbRuntime.unreadCount || 0) > 0
                    ? 'dsbUnreadPulse 1.9s ease-in-out infinite'
                    : 'none',
                display: 'block',
              },
            }),
          )
        : null,

      // Scheme B: Pre-mounted History Sheet in Bar mode (0ms mount delay when morphing Horizontal <-> Vertical!)
      // When collapsed in Horizontal mode, `data-dsb-card-hidden="1"` hides it via GPU clip-path + opacity
      // and excludes it from `measureAndReportLayout()` so zero extra RGN pixels are claimed.
      h(
            'div',
            {
              'data-dsb-card': '',
              'data-dsb-floating-anchor': '1',
              'data-dsb-card-hidden': IS_BAR && barMode !== 'lip' && !isExpanded ? '1' : '0',
              style: {
                position: IS_BAR && isSideDock ? 'fixed' : 'absolute',
                left: 0,
                right: 0,
                top: IS_BAR && isSideDock ? '0px' : 'auto',
                bottom:
                  IS_BAR && isSideDock
                    ? Math.max(84, sideComposerHeight + 6) + 'px'
                    : IS_BAR
                      ? barMode === 'lip'
                        ? '0px'
                        : DETACHED_GAP_PX + 'px'
                      : -TUCK_OVERLAP_PX + 'px',
                height:
                  IS_BAR && isSideDock
                    ? 'auto'
                    : (isExpanded || IS_BAR ? expandedHeight : COLLAPSED_LIP_PX) + (IS_BAR ? 0 : TUCK_OVERLAP_PX) + 'px',
                paddingBottom: IS_BAR ? '0px' : TUCK_OVERLAP_PX + 'px',
                borderRadius: IS_BAR && isSideDock ? '18px 18px 14px 14px' : cardBorderRadius,
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
                pointerEvents: IS_BAR && barMode !== 'lip' && !isExpanded ? 'none' : 'auto',
                willChange: IS_BAR ? 'transform, opacity' : 'auto',
                transition: IS_BAR || isResizingHeightRef.current
                  ? undefined
                  : 'height 0.22s cubic-bezier(0.22, 1, 0.36, 1), box-shadow 0.2s ease',
              },
            },
        isExpanded || (IS_BAR && barMode !== 'lip')
          ? [
              h(
                'div',
                {
                  key: 'grip-header',
                  onMouseDown: isSideDock ? handleStartDragOrToggle : handleStartResizeHeight,
                  onDoubleClick: isSideDock ? handleResetWindowPos : undefined,
                  onContextMenu: isSideDock ? handleOpenDockMenu : undefined,
                  title: isSideDock
                    ? '按住向内侧拖出可取消停靠 · 右键停靠菜单 · 双击复位居中'
                    : '上下拖拽调节消息抽屉高度',
                  style: {
                    position: 'relative',
                    height: '36px',
                    flexShrink: 0,
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    padding: '0 8px 0 18px',
                    cursor: isSideDock ? 'grab' : 'ns-resize',
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
                    width: isSideDock ? '42px' : '36px',
                    height: '4px',
                    borderRadius: '999px',
                    background: isSideDock
                      ? (HARNESS_META[activeHarness] || HARNESS_META.dsh).accent
                      : 'rgba(128,128,128,0.38)',
                    opacity: isSideDock ? 0.85 : 1,
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
                          'data-dsb-orient-btn': '1',
                          title: isSideDock
                            ? '切换回横向悬浮条'
                            : '收起聊天记录（回到纯净输入框）',
                          'aria-label': isSideDock ? '切换回横向悬浮条' : '收起聊天记录',
                          onClick: (ev) => {
                            ev.stopPropagation();
                            if (isSideDock) {
                              triggerOrientationMorph(false);
                            } else {
                              toggleHistorySheet();
                            }
                          },
                          style: {
                            width: '22px',
                            height: '22px',
                            padding: 0,
                            borderRadius: '6px',
                            border: isSideDock ? '1px solid rgba(128,128,140,0.28)' : 'none',
                            background: 'transparent',
                            color: 'inherit',
                            opacity: 0.78,
                            display: 'inline-flex',
                            alignItems: 'center',
                            justifyContent: 'center',
                            cursor: 'pointer',
                            flexShrink: 0,
                          },
                        },
                        isSideDock
                          ? h(
                              'svg',
                              {
                                width: 12,
                                height: 12,
                                viewBox: '0 0 16 16',
                                fill: 'none',
                                stroke: 'currentColor',
                                strokeWidth: 1.5,
                                strokeLinecap: 'round',
                                strokeLinejoin: 'round',
                              },
                              h('rect', { x: 1.5, y: 9.5, width: 13, height: 4.5, rx: 1.5 }),
                              h('path', { d: 'M8 2.5v4.5M5.5 5L8 7.5 10.5 5' }),
                            )
                          : HARNESS_SDK_ICONS.chevronDown('currentColor'),
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
        ),
      // Floating Unread Toast / Hover-Peek Preview (rendered as sibling when History Sheet is hidden!)
      IS_BAR && barMode !== 'lip' && !isExpanded && (dsbRuntime.unreadCount || 0) > 0
          ? (() => {
              const t = getUnifiedTheme(activeHarness);
              const toastShadow = t.isDark
                ? '0 10px 28px rgba(0, 0, 0, 0.46)'
                : '0 6px 20px rgba(15, 23, 42, 0.11)';
              return h(
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
                      dsbRuntime.pendingBubbleTarget = null;
                      setHistoryOpen(true);
                      dsbRuntime.unreadCount = 0;
                      dsbRuntime.unreadPreview = '';
                      notifyBarRuntime();
                      scheduleLayoutReport(280);
                    },
                    style: {
                      pointerEvents: 'auto',
                      position: 'relative',
                      overflow: 'hidden',
                      display: 'inline-flex',
                      alignItems: 'center',
                      gap: '8px',
                      maxWidth: '94%',
                      padding: '6px 10px 6px 12px',
                      borderRadius: '999px',
                      background: t.surface,
                      border: '1px solid ' + t.accentBorder,
                      boxShadow: toastShadow,
                      color: t.fg,
                      fontSize: '12px',
                      cursor: 'pointer',
                      animation: 'dsbFloatReveal 170ms cubic-bezier(0.22, 1, 0.36, 1) both',
                      willChange: 'transform, opacity',
                      transition: 'background 180ms ease, border-color 180ms ease, color 180ms ease',
                      fontFamily: '"Segoe UI", "Inter", "PingFang SC", "Microsoft YaHei", system-ui, sans-serif',
                    },
                  },
                  h('span', {
                    style: {
                      width: '7px',
                      height: '7px',
                      borderRadius: '999px',
                      background: t.dotStyle,
                      animation: 'dsbUnreadPulse 2s infinite',
                      flexShrink: 0,
                    },
                  }),
                  h(
                    'span',
                    { style: { color: t.accent, fontWeight: 600, flexShrink: 0 } },
                    (dsbRuntime.unreadCount || 1) + ' 条新回复',
                  ),
                  h(
                    'span',
                    {
                      style: {
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                        color: t.fg,
                        opacity: t.isDark ? 0.9 : 0.86,
                      },
                    },
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
                        color: t.muted,
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
              );
            })()
          : IS_BAR && barMode !== 'lip' && !isExpanded && hoverPeek && latestSingleLine
            ? (() => {
                const t = getUnifiedTheme(activeHarness);
                const peekShadow = t.isDark
                  ? '0 12px 32px rgba(0, 0, 0, 0.48)'
                  : '0 8px 24px rgba(15, 23, 42, 0.11)';
                return h(
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
                      onMouseEnter: handleHandleMouseEnter,
                      onMouseLeave: handleHandleMouseLeave,
                      onClick: () => {
                        setHoverPeek(false);
                        toggleHistorySheet();
                      },
                      style: {
                        pointerEvents: 'auto',
                        cursor: 'pointer',
                        position: 'relative',
                        overflow: 'hidden',
                        padding: '9px 14px',
                        borderRadius: '16px',
                        background: t.surface,
                        border: '1px solid ' + t.border,
                        boxShadow: peekShadow,
                        color: t.fg,
                        fontSize: '12.5px',
                        lineHeight: '18px',
                        animation: 'dsbFloatReveal 170ms cubic-bezier(0.22, 1, 0.36, 1) both',
                        willChange: 'transform, opacity',
                        transition: 'background 180ms ease, border-color 180ms ease, color 180ms ease',
                        fontFamily: '"Segoe UI", "Inter", "PingFang SC", "Microsoft YaHei", system-ui, sans-serif',
                      },
                    },
                    h('div', {
                      style: {
                        position: 'absolute',
                        left: 0,
                        right: 0,
                        top: 0,
                        height: '2px',
                        background: t.topBar,
                        opacity: 0.9,
                        pointerEvents: 'none',
                      },
                    }),
                    h(
                      'div',
                      { style: { fontSize: '11px', color: t.accent, marginBottom: '3px', fontWeight: 600 } },
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
                          color: t.fg,
                          opacity: t.isDark ? 0.9 : 0.86,
                        },
                      },
                      latestSingleLine,
                    ),
                  ),
                );
              })()
            : null,
        IS_BAR && barMode !== 'lip' && activeHarness === 'dsh'
          ? renderDragHandle(
              isTopDock
                ? {
                    position: 'fixed',
                    top: Math.max(12, sideComposerHeight - 13) + 'px',
                    bottom: 'auto',
                    left: '50%',
                    transform: 'translateX(-50%)',
                    zIndex: 40,
                    pointerEvents: 'auto',
                  }
                : { top: 'auto', bottom: '-13px', zIndex: 40, pointerEvents: 'auto' },
            )
          : null,
      ),
      renderExternalNativeComposer(),
      renderComposerPopover(),
      renderQuickSearchPopup(),
      renderDockQuickMenu(),
      renderSnapPreviewBanner(),
    );
  };
}

