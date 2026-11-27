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
    const [customAcpLabel, setCustomAcpLabel] = useState('');
    const [customAcpCmd, setCustomAcpCmd] = useState('');
    const [customAcpArgs, setCustomAcpArgs] = useState('');
    const [customAcpErr, setCustomAcpErr] = useState('');
    const [uiProfilesOpen, setUiProfilesOpen] = useState(false);
    const [uiProfilesData, setUiProfilesData] = useState({ profiles: [], visualBoxProvider: null });
    const [importProfileText, setImportProfileText] = useState('');
    const [importProfileErr, setImportProfileErr] = useState('');
    const [adapterPluginsOpen, setAdapterPluginsOpen] = useState(false);
    const [adapterPluginsData, setAdapterPluginsData] = useState({
      plugins: [],
      pluginsDir: '',
      registryUrl: '',
      builtinCount: 0,
      externalCount: 0,
      enabledCount: 0,
    });
    const [adapterPluginTab, setAdapterPluginTab] = useState('all');
    const [adapterInstallInput, setAdapterInstallInput] = useState('');
    const [adapterPluginBusy, setAdapterPluginBusy] = useState(false);
    const [adapterPluginErr, setAdapterPluginErr] = useState('');
    const [adapterTestNote, setAdapterTestNote] = useState('');
    const [adapterRegistry, setAdapterRegistry] = useState(null);
    const [adapterRegistryUrlInput, setAdapterRegistryUrlInput] = useState('');
    const [adapterRegistryLoading, setAdapterRegistryLoading] = useState(false);
    const [visionOpen, setVisionOpen] = useState(false);
    const [visionCfg, setVisionCfg] = useState(null);
    const [visionHw, setVisionHw] = useState(null);
    const [visionTesting, setVisionTesting] = useState(false);
    const [visionTestResult, setVisionTestResult] = useState(null);
    const [visionShowKey, setVisionShowKey] = useState(false);
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
        try { if (typeof offWs === 'function') offWs(); } catch (err) { void err; }
        try { if (typeof offSess === 'function') offSess(); } catch (err) { void err; }
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

    // Re-snap popover coordinates and Win32 window shapes whenever Harness/columns/settings accordions change
    useEffect(() => {
      if (!menuOpen) return undefined;
      const raf = window.requestAnimationFrame(() => {
        setTick((n) => n + 1);
        if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
          dsbRuntime.scheduleLayoutReport(280);
        }
      });
      return () => window.cancelAnimationFrame(raf);
    }, [
      menuOpen,
      activeHarness,
      browsedHarness,
      hoveredWsId,
      settingsView,
      adapterPluginsOpen,
      adapterPluginTab,
      visionOpen,
      uiProfilesOpen,
      dsbRuntime.dockEdge,
      dsbRuntime.verticalMode,
    ]);

    // Observe real DOM size changes of the open popup (e.g., settings sections expanding/collapsing)
    // so Win32 SetWindowRgn ALWAYS matches the exact rounded panel silhouette with zero clipping!
    useEffect(() => {
      if (!menuOpen || typeof ResizeObserver === 'undefined') return undefined;
      const node = menuRef.current;
      if (!node) return undefined;
      const ro = new ResizeObserver(() => {
        if (typeof dsbRuntime.scheduleLayoutReport === 'function') {
          dsbRuntime.scheduleLayoutReport(120);
        }
      });
      ro.observe(node);
      return () => ro.disconnect();
    }, [menuOpen, settingsView]);

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
        } catch (err) {
          void err;
        }
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
      if (!menuOpen || settingsView || dsbRuntime.dockEdge === 'top') {
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

      const compEl = !activeMeta.useNativeComposer
        ? document.querySelector('[data-dsb-external-composer]')
        : document.querySelector('[data-composer-card]');
      const queueEl = activeMeta.useNativeComposer
        ? document.querySelector('[data-queue-dock]')
        : null;
      const ir = compEl ? compEl.getBoundingClientRect() : null;
      const qr = queueEl ? queueEl.getBoundingClientRect() : null;

      const winW = typeof window !== 'undefined' ? window.innerWidth || 760 : 760;
      const winH = typeof window !== 'undefined' ? window.innerHeight || 680 : 680;
      const isTopDockMenu = dsbRuntime.dockEdge === 'top';
      const isSideDockMenu = dsbRuntime.dockEdge === 'left' || dsbRuntime.dockEdge === 'right';
      const isVerticalLayout = isSideDockMenu || Boolean(dsbRuntime.verticalMode);

      // Anchor directly above the active composer (and native queue dock if present) — NEVER
      // measure hidden pre-mounted `[data-dsb-card][data-dsb-card-hidden="1"]` or full-height
      // vertical history sheets (which previously floated the menu 380px in mid-air!).
      const fallbackSlabH = activeMeta.fallbackSlabHeight || 112;
      let anchorTop = isValidRect(ir) ? ir.top : winH - fallbackSlabH;
      if (isValidRect(qr) && qr.top < anchorTop && anchorTop - qr.bottom < 24) {
        anchorTop = qr.top;
      }
      const bottomPx = Math.max(8, Math.min(winH - 180, Math.round(winH - anchorTop + 8)));
      const menuTopPx = isValidRect(ir) ? Math.round(ir.bottom + 8) : 112;
      const availPopupH = isTopDockMenu
        ? Math.max(200, winH - menuTopPx - 10)
        : Math.max(200, winH - bottomPx - 10);
      const settingsMaxH = Math.min(430, availPopupH);
      const colMaxH = Math.min(300, availPopupH);

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

      // Compute visible host panel bounds inside the constant 960dip stage so BOTH horizontal
      // and vertical (free-floating or left/right docked) menus align flush with the actual bar!
      const fallbackPanelW = isVerticalLayout
        ? Math.round(dsbRuntime.sideWidth || 420)
        : Math.round(dsbRuntime.barWidth || 760);
      const hostW = isValidRect(ir) ? Math.round(ir.width) : Math.min(winW, fallbackPanelW);
      const hostLeft = isValidRect(ir)
        ? Math.round(ir.left)
        : dsbRuntime.dockEdge === 'left'
          ? 0
          : dsbRuntime.dockEdge === 'right'
            ? Math.max(0, winW - hostW)
            : Math.max(0, Math.round((winW - hostW) / 2));
      const hostRight = hostLeft + hostW;

      // Responsive 3-column widths based on actual host panel width (not 960dip stage width!)
      const ultraCompact = isVerticalLayout || hostW < 470;
      const compact3Col = !ultraCompact && hostW < 640;
      const colGap = ultraCompact ? 4 : 6;
      const col1W = ultraCompact ? 112 : compact3Col ? 138 : 166;
      const col2W = ultraCompact ? 128 : compact3Col ? 156 : 188;
      const col3W = ultraCompact ? 148 : compact3Col ? 176 : 236;
      const totalCascadeW = settingsView
        ? isVerticalLayout
          ? Math.max(320, Math.min(468, hostW - 12))
          : Math.min(468, Math.max(340, hostW - 16))
        : col1W + colGap + col2W + colGap + col3W;

      let leftPx;
      if (totalCascadeW <= hostW - 12) {
        const rawLeft = chipRect ? Math.round(chipRect.left) : hostLeft + 6;
        leftPx = Math.max(hostLeft + 6, Math.min(rawLeft, hostRight - totalCascadeW - 6));
      } else if (dsbRuntime.dockEdge === 'left') {
        leftPx = Math.max(6, hostLeft + 4);
      } else if (dsbRuntime.dockEdge === 'right') {
        leftPx = Math.max(6, hostRight - totalCascadeW - 4);
      } else {
        const centered = Math.round(hostLeft + (hostW - totalCascadeW) / 2);
        leftPx = Math.max(6, Math.min(centered, Math.max(6, winW - totalCascadeW - 6)));
      }

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

      const refreshUiProfiles = () => {
        tauriInvoke('pick_list_profiles', {})
          .then((res) => {
            if (res && Array.isArray(res.profiles)) {
              setUiProfilesData({
                profiles: res.profiles,
                visualBoxProvider: res.visualBoxProvider || null,
              });
            }
          })
          .catch((err) => {
            setImportProfileErr(String(err || ''));
          });
      };

      const refreshAdapterPlugins = (reloadDisk = false) => {
        const cmd = reloadDisk ? 'adapter_plugins_reload' : 'adapter_plugins_list';
        tauriInvoke(cmd, {})
          .then((res) => {
            if (res && Array.isArray(res.plugins)) {
              setAdapterPluginsData(res);
              if (res.registryUrl && !adapterRegistryUrlInput) {
                setAdapterRegistryUrlInput(res.registryUrl);
              }
            }
          })
          .catch((err) => {
            setAdapterPluginErr(String(err || '加载插件列表失败'));
          });
      };

      const refreshAdapterRegistry = (overrideUrl) => {
        setAdapterRegistryLoading(true);
        setAdapterPluginErr('');
        const targetUrl =
          typeof overrideUrl === 'string' && overrideUrl.trim()
            ? overrideUrl.trim()
            : adapterRegistryUrlInput.trim() || null;
        tauriInvoke('adapter_plugins_fetch_registry', { registryUrl: targetUrl })
          .then((res) => {
            if (res) setAdapterRegistry(res);
          })
          .catch((err) => {
            setAdapterPluginErr('获取社区市场失败: ' + String(err || ''));
          })
          .finally(() => setAdapterRegistryLoading(false));
      };

      const toggleAdapterPlugin = (item) => {
        setAdapterPluginErr('');
        const nextEnabled = !item.enabled;
        tauriInvoke('adapter_plugins_set_enabled', { id: item.id, enabled: nextEnabled })
          .then(() => {
            refreshAdapterPlugins(false);
            setActionToast(
              (nextEnabled ? '已启用 ' : '已热禁用 ') + (item.name || item.id),
            );
            setTimeout(() => setActionToast(''), 1800);
          })
          .catch((err) => {
            setAdapterPluginErr(String(err || '切换插件状态失败'));
          });
      };

      const installAdapterFromInput = () => {
        const raw = adapterInstallInput.trim();
        if (!raw) {
          setAdapterPluginErr('请先输入 GitHub / HTTPS 插件链接，或粘贴 adapter.json');
          return;
        }
        setAdapterPluginErr('');
        setAdapterPluginBusy(true);
        const p = raw.startsWith('{')
          ? tauriInvoke('adapter_plugins_install_json', {
              rawJson: raw,
              sourceHint: 'pasted-json',
              trustCommand: false,
            })
          : tauriInvoke('adapter_plugins_install_url', {
              url: raw,
              trustCommand: false,
            });
        p.then((installed) => {
          setAdapterInstallInput('');
          refreshAdapterPlugins(false);
          if (adapterRegistry) refreshAdapterRegistry();
          const msg =
            installed && installed.requiresTrustConfirm
              ? '已下载 ' + installed.name + '（脚本插件需点击“信任并启用”）'
              : '已安装并启用 ' + ((installed && installed.name) || '插件');
          setActionToast(msg);
          setTimeout(() => setActionToast(''), 2600);
        })
          .catch((err) => {
            setAdapterPluginErr(String(err || '安装失败'));
          })
          .finally(() => setAdapterPluginBusy(false));
      };

      const installAdapterFromLocalFile = () => {
        setAdapterPluginErr('');
        tauriInvoke('app_pick_files', {})
          .then((picked) => {
            const list = Array.isArray(picked) ? picked : picked && Array.isArray(picked.paths) ? picked.paths : [];
            if (!list.length) return;
            setAdapterPluginBusy(true);
            return tauriInvoke('adapter_plugins_install_file', {
              path: String(list[0]),
              trustCommand: false,
            }).then((installed) => {
              refreshAdapterPlugins(false);
              if (adapterRegistry) refreshAdapterRegistry();
              const msg =
                installed && installed.requiresTrustConfirm
                  ? '已导入 ' + installed.name + '（脚本插件需点击“信任并启用”）'
                  : '已导入并启用 ' + ((installed && installed.name) || '插件');
              setActionToast(msg);
              setTimeout(() => setActionToast(''), 2400);
            });
          })
          .catch((err) => {
            setAdapterPluginErr(String(err || '本地导入失败'));
          })
          .finally(() => setAdapterPluginBusy(false));
      };

      const installMarketEntry = (entry) => {
        setAdapterPluginErr('');
        setAdapterPluginBusy(true);
        const tryInlineFallback = () => {
          if (!entry.inlineManifest) {
            return Promise.reject(new Error('远程地址暂不可达且无内嵌清单'));
          }
          return tauriInvoke('adapter_plugins_install_json', {
            rawJson: JSON.stringify(entry.inlineManifest),
            sourceHint: entry.downloadUrl || 'curated-registry',
            trustCommand: false,
          });
        };
        const req =
          entry.inlineManifest && adapterRegistry && adapterRegistry.source === 'curated_fallback'
            ? tryInlineFallback()
            : tauriInvoke('adapter_plugins_install_url', {
                url: entry.downloadUrl,
                trustCommand: false,
              }).catch(() => tryInlineFallback());

        req
          .then((installed) => {
            refreshAdapterPlugins(false);
            refreshAdapterRegistry();
            const msg =
              installed && installed.requiresTrustConfirm
                ? '已下载 ' + installed.name + '（脚本插件需点击“信任并启用”）'
                : '已安装 ' + ((installed && installed.name) || entry.name);
            setActionToast(msg);
            setTimeout(() => setActionToast(''), 2600);
          })
          .catch((err) => {
            setAdapterPluginErr('下载安装失败: ' + String(err || ''));
          })
          .finally(() => setAdapterPluginBusy(false));
      };

      const refreshVisionConfig = () => {
        tauriInvoke('vision_get_config', {})
          .then((res) => {
            if (res && res.config) setVisionCfg(res.config);
            if (res && res.hardware) setVisionHw(res.hardware);
          })
          .catch(() => {});
      };

      const saveVisionPatch = (patch, applyPreset = false, toastMsg = '') => {
        tauriInvoke('vision_update_config', { patch, applyPreset })
          .then((nextCfg) => {
            if (nextCfg) setVisionCfg(nextCfg);
            if (toastMsg) {
              setActionToast(toastMsg);
              setTimeout(() => setActionToast(''), 1800);
            }
          })
          .catch((err) => {
            setActionToast('保存失败: ' + String(err || ''));
            setTimeout(() => setActionToast(''), 2200);
          });
      };

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
              maxHeight: settingsMaxH + 'px',
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
          // Open Harness Platform: Auto-Discovered ACP & Custom ACP Registration
          h(
            'div',
            {
              'data-dsb-acp-settings': '1',
              style: {
                display: 'flex',
                flexDirection: 'column',
                gap: '5px',
                padding: '7px 8px',
                borderRadius: '10px',
                background: panelRowBg,
                border: '1px solid ' + panelBorder,
              },
            },
            h(
              'div',
              { style: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: '6px' } },
              h('span', { style: { fontSize: '11.5px', fontWeight: 600, color: panelFg } }, '开放 Harness（ACP 标准生态 & 自定义）'),
              h('span', { style: { fontSize: '10px', color: panelMuted } }, '自动探测本机 · 零代码接入新工具'),
            ),
            h(
              'div',
              { style: { display: 'flex', flexWrap: 'wrap', gap: '4px' } },
              (dsbRuntime.harnessAvailability || [])
                .filter((x) => x && x.providerKind === 'acp')
                .map((item) =>
                  h(
                    'span',
                    {
                      key: 'acp-chip-' + item.id,
                      title: (item.command || '') + (Array.isArray(item.args) && item.args.length ? ' ' + item.args.join(' ') : ''),
                      style: {
                        display: 'inline-flex',
                        alignItems: 'center',
                        gap: '5px',
                        padding: '2px 7px',
                        borderRadius: '999px',
                        fontSize: '11px',
                        background: item.available ? 'rgba(16, 185, 129, 0.14)' : 'rgba(148, 163, 184, 0.14)',
                        border: '1px solid ' + (item.available ? 'rgba(16, 185, 129, 0.35)' : panelBorder),
                        color: panelFg,
                      },
                    },
                    h('span', null, (item.available ? '✓ ' : '· ') + item.label),
                    item.custom
                      ? h(
                          'button',
                          {
                            type: 'button',
                            title: '移除该自定义 ACP Harness',
                            onClick: () => {
                              tauriInvoke('harness_remove_custom_acp', { id: item.id })
                                .then(() => {
                                  refreshHarnessAvailability();
                                  setActionToast('已移除 ' + item.label);
                                  setTimeout(() => setActionToast(''), 1800);
                                })
                                .catch(() => {});
                            },
                            style: {
                              border: 'none',
                              background: 'transparent',
                              color: '#fca5a5',
                              cursor: 'pointer',
                              padding: '0 2px',
                              fontSize: '11px',
                              lineHeight: 1,
                            },
                          },
                          '×',
                        )
                      : null,
                  ),
                ),
            ),
            h(
              'div',
              { style: { display: 'flex', gap: '4px', alignItems: 'center' } },
              h('input', {
                type: 'text',
                placeholder: '名称（如 Goose）',
                value: customAcpLabel,
                onChange: (ev) => setCustomAcpLabel(ev.target.value),
                style: {
                  width: '28%',
                  padding: '4px 6px',
                  borderRadius: '7px',
                  fontSize: '11px',
                  background: 'rgba(128,128,128,0.12)',
                  border: '1px solid ' + panelBorder,
                  color: panelFg,
                  outline: 'none',
                  boxSizing: 'border-box',
                },
              }),
              h('input', {
                type: 'text',
                placeholder: '命令（如 goose / npx）',
                value: customAcpCmd,
                onChange: (ev) => setCustomAcpCmd(ev.target.value),
                style: {
                  width: '32%',
                  padding: '4px 6px',
                  borderRadius: '7px',
                  fontSize: '11px',
                  background: 'rgba(128,128,128,0.12)',
                  border: '1px solid ' + panelBorder,
                  color: panelFg,
                  outline: 'none',
                  boxSizing: 'border-box',
                },
              }),
              h('input', {
                type: 'text',
                placeholder: '参数（如 acp）',
                value: customAcpArgs,
                onChange: (ev) => setCustomAcpArgs(ev.target.value),
                style: {
                  flex: 1,
                  minWidth: 0,
                  padding: '4px 6px',
                  borderRadius: '7px',
                  fontSize: '11px',
                  background: 'rgba(128,128,128,0.12)',
                  border: '1px solid ' + panelBorder,
                  color: panelFg,
                  outline: 'none',
                  boxSizing: 'border-box',
                },
              }),
              h(
                'button',
                {
                  type: 'button',
                  'data-dsb-add-acp-btn': '1',
                  onClick: () => {
                    const cmd = customAcpCmd.trim();
                    if (!cmd) {
                      setCustomAcpErr('请先填写启动命令');
                      return;
                    }
                    setCustomAcpErr('');
                    const parsedArgs = customAcpArgs
                      .trim()
                      .split(/\s+/)
                      .filter(Boolean);
                    tauriInvoke('harness_upsert_custom_acp', {
                      item: {
                        id: customAcpLabel.trim() || cmd,
                        label: customAcpLabel.trim() || cmd,
                        command: cmd,
                        args: parsedArgs,
                        accent: '#38bdf8',
                        defaultCwd: null,
                      },
                    })
                      .then(() => {
                        setCustomAcpLabel('');
                        setCustomAcpCmd('');
                        setCustomAcpArgs('');
                        refreshHarnessAvailability();
                        setActionToast('已接入自定义 ACP Harness');
                        setTimeout(() => setActionToast(''), 2000);
                      })
                      .catch((err) => {
                        setCustomAcpErr(String(err || '添加失败'));
                      });
                  },
                  style: {
                    padding: '4px 9px',
                    borderRadius: '7px',
                    border: '1px solid ' + unifiedPanelTheme.accentBorder,
                    background: unifiedPanelTheme.accentBg,
                    color: unifiedPanelTheme.accent,
                    fontSize: '11px',
                    fontWeight: 600,
                    cursor: 'pointer',
                    flexShrink: 0,
                  },
                },
                '＋ 接入',
              ),
            ),
            customAcpErr
              ? h('div', { style: { fontSize: '10.5px', color: '#fca5a5' } }, customAcpErr)
              : null,
          ),
          // Multi-Tier Vision Engine, Re-brush VLM Verification & Natural Photo Semantic Matting
          h(
            'div',
            {
              'data-dsb-vision-settings': '1',
              style: {
                display: 'flex',
                flexDirection: 'column',
                gap: '6px',
                padding: '7px 8px',
                borderRadius: '10px',
                background: panelRowBg,
                border: '1px solid ' + panelBorder,
              },
            },
            h(
              'button',
              {
                type: 'button',
                onClick: () => {
                  const next = !visionOpen;
                  setVisionOpen(next);
                  if (next) refreshVisionConfig();
                },
                style: {
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: '6px',
                  width: '100%',
                  background: 'transparent',
                  border: 'none',
                  padding: 0,
                  color: panelFg,
                  cursor: 'pointer',
                  textAlign: 'left',
                },
              },
              h(
                'span',
                { style: { fontSize: '11.5px', fontWeight: 600, color: panelFg } },
                '视觉引擎与智能抠图（4 档硬件预设 & 摇一摇续加剂量）',
              ),
              h(
                'span',
                { style: { fontSize: '10px', color: panelMuted } },
                visionOpen ? '收起 ▲' : '展开配置 ▼',
              ),
            ),
            visionOpen
              ? (() => {
                  const vc = visionCfg || {
                    preset: 'lite_office',
                    segmentationBackend: 'disabled',
                    vlmBackend: 'disabled',
                    autoPhotoMatting: true,
                    rebrushVlmAudit: true,
                    idleUnloadSeconds: 180,
                    cloudProviderHint: 'openai',
                    cloudBaseUrl: 'https://api.openai.com/v1',
                    cloudModel: 'gpt-4o-mini',
                    cloudApiKey: '',
                    ollamaBaseUrl: 'http://127.0.0.1:11434',
                    ollamaModel: 'qwen2.5vl:7b',
                  };
                  const hw = visionHw;
                  const presetCards = [
                    { id: 'lite_office', title: '极速办公本', sub: '0 显存 · 云端 VLM + CPU 抠图' },
                    { id: 'balanced_universal', title: '中度万金油', sub: '6-8G · SAM2 Tiny + Florence2' },
                    { id: 'pro_ai_workstation', title: '旗舰 AI 主机', sub: '≥12G · SAM2 Small + QwenVL' },
                    { id: 'custom_hybrid', title: '自定义混合', sub: '自由组合抠图 × VLM 后端' },
                  ];
                  const cloudTemplates = [
                    { id: 'openai', label: 'OpenAI', url: 'https://api.openai.com/v1', model: 'gpt-4o-mini', backend: 'cloud_open_ai_compat' },
                    { id: 'gemini', label: 'Gemini', url: 'https://generativelanguage.googleapis.com/v1beta', model: 'gemini-2.5-flash', backend: 'cloud_gemini' },
                    { id: 'qwen', label: '通义千问', url: 'https://dashscope.aliyuncs.com/compatible-mode/v1', model: 'qwen2.5-vl-7b-instruct', backend: 'cloud_open_ai_compat' },
                    { id: 'deepseek', label: '硅基/自定义', url: 'https://api.siliconflow.cn/v1', model: 'Qwen/Qwen2.5-VL-72B-Instruct', backend: 'cloud_open_ai_compat' },
                  ];
                  const fieldStyle = {
                    width: '100%',
                    padding: '4px 6px',
                    borderRadius: '7px',
                    fontSize: '10.5px',
                    background: 'rgba(128,128,128,0.12)',
                    border: '1px solid ' + panelBorder,
                    color: panelFg,
                    outline: 'none',
                    boxSizing: 'border-box',
                  };
                  const miniBtn = {
                    padding: '3px 7px',
                    borderRadius: '6px',
                    border: '1px solid ' + panelBorder,
                    background: 'transparent',
                    color: panelFg,
                    fontSize: '10px',
                    cursor: 'pointer',
                  };
                  return h(
                    'div',
                    { style: { display: 'flex', flexDirection: 'column', gap: '6px', marginTop: '2px' } },
                    hw
                      ? h(
                          'div',
                          {
                            style: {
                              padding: '5px 7px',
                              borderRadius: '8px',
                              background: 'rgba(56,189,248,0.08)',
                              border: '1px solid rgba(56,189,248,0.22)',
                              fontSize: '10px',
                              color: panelFg,
                              display: 'flex',
                              flexDirection: 'column',
                              gap: '3px',
                            },
                          },
                          h(
                            'div',
                            { style: { display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: '4px' } },
                            h(
                              'span',
                              { style: { fontWeight: 600 } },
                              hw.gpuName + (hw.vramTotalMb > 0 ? ' (' + (hw.vramTotalMb / 1024).toFixed(1) + 'G / 可用 ' + (hw.vramFreeMb / 1024).toFixed(1) + 'G)' : ''),
                            ),
                            h(
                              'div',
                              { style: { display: 'flex', gap: '4px' } },
                              h('button', { type: 'button', style: miniBtn, onClick: refreshVisionConfig }, '刷新'),
                              h(
                                'button',
                                {
                                  type: 'button',
                                  style: miniBtn,
                                  onClick: () => {
                                    tauriInvoke('vision_unload_models', {})
                                      .then(() => {
                                        setActionToast('已触发释放本地模型显存');
                                        setTimeout(() => setActionToast(''), 1800);
                                        refreshVisionConfig();
                                      })
                                      .catch(() => {});
                                  },
                                },
                                '释放显存',
                              ),
                            ),
                          ),
                          h(
                            'div',
                            { style: { color: panelMuted } },
                            'Sidecar: ' +
                              (hw.sidecarEnvReady ? '已安装' : '未安装') +
                              ' · Ollama: ' +
                              (hw.ollamaRunning ? '在线(' + (hw.ollamaModels || []).length + '模型)' : '未启动') +
                              ' · 推荐: ' +
                              (presetCards.find((p) => p.id === hw.recommendedPreset)?.title || hw.recommendedPreset),
                          ),
                        )
                      : null,
                    // 4 Hardware Preset Buttons
                    h(
                      'div',
                      { style: { display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '4px' } },
                      presetCards.map((p) => {
                        const active = vc.preset === p.id;
                        const isRec = hw && hw.recommendedPreset === p.id;
                        return h(
                          'button',
                          {
                            key: 'vp-' + p.id,
                            type: 'button',
                            onClick: () =>
                              saveVisionPatch(
                                { preset: p.id },
                                p.id !== 'custom_hybrid',
                                '已切换至「' + p.title + '」档位',
                              ),
                            style: {
                              textAlign: 'left',
                              padding: '5px 7px',
                              borderRadius: '8px',
                              cursor: 'pointer',
                              border: '1px solid ' + (active ? unifiedPanelTheme.accentBorder : panelBorder),
                              background: active ? unifiedPanelTheme.accentBg : 'rgba(128,128,128,0.06)',
                              color: panelFg,
                            },
                          },
                          h(
                            'div',
                            { style: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontSize: '11px', fontWeight: 600 } },
                            h('span', null, p.title),
                            isRec
                              ? h(
                                  'span',
                                  { style: { fontSize: '9px', padding: '0 4px', borderRadius: '999px', background: 'rgba(16,185,129,0.2)', color: '#34d399' } },
                                  '推荐',
                                )
                              : null,
                          ),
                          h('div', { style: { fontSize: '9.5px', color: panelMuted, marginTop: '1px' } }, p.sub),
                        );
                      }),
                    ),
                    // Granular backend selectors
                    h(
                      'div',
                      { style: { display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '5px' } },
                      h(
                        'label',
                        { style: { display: 'flex', flexDirection: 'column', gap: '2px', fontSize: '10px', color: panelMuted } },
                        '轮廓抠图后端',
                        h(
                          'select',
                          {
                            value: vc.segmentationBackend || 'disabled',
                            onChange: (ev) =>
                              saveVisionPatch({ preset: 'custom_hybrid', segmentationBackend: ev.target.value }, false, '已更新抠图后端'),
                            style: fieldStyle,
                          },
                          h('option', { value: 'disabled' }, '关闭（仅矩形框）'),
                          h('option', { value: 'onnx_mobile_sam_cpu' }, '轻量 CPU MobileSAM ONNX'),
                          h('option', { value: 'gpu_sam2_tiny' }, '中度 GPU SAM 2.1 Tiny'),
                          h('option', { value: 'gpu_sam2_small' }, '旗舰 GPU SAM 2.1 Small'),
                        ),
                      ),
                      h(
                        'label',
                        { style: { display: 'flex', flexDirection: 'column', gap: '2px', fontSize: '10px', color: panelMuted } },
                        '视觉大模型 / 语义引擎',
                        h(
                          'select',
                          {
                            value: vc.vlmBackend || 'disabled',
                            onChange: (ev) =>
                              saveVisionPatch({ preset: 'custom_hybrid', vlmBackend: ev.target.value }, false, '已更新 VLM 后端'),
                            style: fieldStyle,
                          },
                          h('option', { value: 'disabled' }, '关闭（仅本地 L1-L3）'),
                          h('option', { value: 'sidecar_florence2' }, '本地离线 Florence-2'),
                          h('option', { value: 'local_ollama' }, '本机 Ollama VLM'),
                          h('option', { value: 'cloud_open_ai_compat' }, '云端 OpenAI 兼容 VLM'),
                          h('option', { value: 'cloud_gemini' }, '云端 Google Gemini'),
                        ),
                      ),
                    ),
                    // Cloud VLM config block
                    vc.vlmBackend === 'cloud_open_ai_compat' || vc.vlmBackend === 'cloud_gemini'
                      ? h(
                          'div',
                          { style: { display: 'flex', flexDirection: 'column', gap: '4px', padding: '5px 6px', borderRadius: '8px', background: 'rgba(128,128,128,0.06)' } },
                          h(
                            'div',
                            { style: { display: 'flex', gap: '4px', flexWrap: 'wrap' } },
                            cloudTemplates.map((tpl) =>
                              h(
                                'button',
                                {
                                  key: 'ctpl-' + tpl.id,
                                  type: 'button',
                                  style: Object.assign({}, miniBtn, {
                                    borderColor: vc.cloudProviderHint === tpl.id ? unifiedPanelTheme.accentBorder : panelBorder,
                                  }),
                                  onClick: () =>
                                    setVisionCfg(
                                      Object.assign({}, vc, {
                                        cloudProviderHint: tpl.id,
                                        cloudBaseUrl: tpl.url,
                                        cloudModel: tpl.model,
                                        vlmBackend: tpl.backend,
                                      }),
                                    ),
                                },
                                tpl.label,
                              ),
                            ),
                          ),
                          h('input', {
                            type: 'text',
                            placeholder: 'Base URL (例如 https://api.openai.com/v1)',
                            value: vc.cloudBaseUrl || '',
                            onChange: (ev) => setVisionCfg(Object.assign({}, vc, { cloudBaseUrl: ev.target.value })),
                            style: fieldStyle,
                          }),
                          h('input', {
                            type: 'text',
                            placeholder: '模型名 (如 gpt-4o-mini / gemini-2.5-flash / qwen2.5-vl-7b-instruct)',
                            value: vc.cloudModel || '',
                            onChange: (ev) => setVisionCfg(Object.assign({}, vc, { cloudModel: ev.target.value })),
                            style: fieldStyle,
                          }),
                          h(
                            'div',
                            { style: { display: 'flex', gap: '4px' } },
                            h('input', {
                              type: visionShowKey ? 'text' : 'password',
                              placeholder: '云端 API Key（仅存于本机 config.json）',
                              value: vc.cloudApiKey || '',
                              onChange: (ev) => setVisionCfg(Object.assign({}, vc, { cloudApiKey: ev.target.value })),
                              style: Object.assign({}, fieldStyle, { flex: 1 }),
                            }),
                            h(
                              'button',
                              { type: 'button', style: miniBtn, onClick: () => setVisionShowKey((s) => !s) },
                              visionShowKey ? '隐藏' : '显示',
                            ),
                          ),
                        )
                      : null,
                    // Local Ollama config block
                    vc.vlmBackend === 'local_ollama'
                      ? h(
                          'div',
                          { style: { display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '4px' } },
                          h('input', {
                            type: 'text',
                            placeholder: 'Ollama URL',
                            value: vc.ollamaBaseUrl || 'http://127.0.0.1:11434',
                            onChange: (ev) => setVisionCfg(Object.assign({}, vc, { ollamaBaseUrl: ev.target.value })),
                            style: fieldStyle,
                          }),
                          h('input', {
                            type: 'text',
                            placeholder: '模型 (如 qwen2.5vl:7b)',
                            value: vc.ollamaModel || 'qwen2.5vl:7b',
                            onChange: (ev) => setVisionCfg(Object.assign({}, vc, { ollamaModel: ev.target.value })),
                            style: fieldStyle,
                          }),
                        )
                      : null,
                    // Feature toggles
                    h(
                      'div',
                      { style: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: '8px', fontSize: '10.5px', color: panelFg } },
                      h(
                        'label',
                        { style: { display: 'flex', alignItems: 'center', gap: '4px', cursor: 'pointer' } },
                        h('input', {
                          type: 'checkbox',
                          checked: vc.autoPhotoMatting !== false,
                          onChange: (ev) => saveVisionPatch({ autoPhotoMatting: ev.target.checked }, false),
                        }),
                        '照片区自动语义抠图',
                      ),
                      h(
                        'label',
                        { style: { display: 'flex', alignItems: 'center', gap: '4px', cursor: 'pointer' } },
                        h('input', {
                          type: 'checkbox',
                          checked: vc.rebrushVlmAudit !== false,
                          onChange: (ev) => saveVisionPatch({ rebrushVlmAudit: ev.target.checked }, false),
                        }),
                        '持续摇晃自动 VLM 仲裁',
                      ),
                    ),
                    // Save & Smoke Test actions
                    h(
                      'div',
                      { style: { display: 'flex', gap: '5px', alignItems: 'center' } },
                      h(
                        'button',
                        {
                          type: 'button',
                          onClick: () => saveVisionPatch(vc, false, '视觉引擎配置已保存'),
                          style: {
                            padding: '4px 9px',
                            borderRadius: '7px',
                            border: '1px solid ' + unifiedPanelTheme.accentBorder,
                            background: unifiedPanelTheme.accentBg,
                            color: unifiedPanelTheme.accent,
                            fontSize: '10.5px',
                            fontWeight: 600,
                            cursor: 'pointer',
                          },
                        },
                        '保存配置',
                      ),
                      h(
                        'button',
                        {
                          type: 'button',
                          disabled: visionTesting,
                          onClick: () => {
                            setVisionTesting(true);
                            setVisionTestResult(null);
                            tauriInvoke('vision_update_config', { patch: vc, applyPreset: false })
                              .then((updated) => {
                                if (updated) setVisionCfg(updated);
                                return tauriInvoke('vision_test_pipeline', {});
                              })
                              .then((res) => setVisionTestResult(res))
                              .catch((err) => setVisionTestResult({ ok: false, error: String(err) }))
                              .finally(() => setVisionTesting(false));
                          },
                          style: miniBtn,
                        },
                        visionTesting ? '正在自检…' : '一键自检当前方案',
                      ),
                    ),
                    visionTestResult
                      ? h(
                          'div',
                          {
                            style: {
                              padding: '5px 7px',
                              borderRadius: '7px',
                              fontSize: '10px',
                              background: visionTestResult.ok ? 'rgba(16,185,129,0.12)' : 'rgba(248,113,113,0.12)',
                              color: visionTestResult.ok ? '#6ee7b7' : '#fca5a5',
                            },
                          },
                          visionTestResult.error
                            ? '自检失败: ' + visionTestResult.error
                            : (visionTestResult.ok ? '✓ 全部就绪' : '⚠ 部分组件未就绪') +
                                ' (' +
                                (visionTestResult.latencyMs || 0) +
                                'ms) · 抠图: ' +
                                (visionTestResult.segmentation?.detail || '-') +
                                ' · VLM: ' +
                                (visionTestResult.vlm?.detail || '-'),
                        )
                      : null,
                  );
                })()
              : null,
          ),
          // Pluggable Deep App Adapter Plugin Center (Hot Plug/Unplug + Community Download)
          h(
            'div',
            {
              'data-dsb-adapter-plugins-settings': '1',
              style: {
                display: 'flex',
                flexDirection: 'column',
                gap: '5px',
                padding: '7px 8px',
                borderRadius: '10px',
                background: panelRowBg,
                border: '1px solid ' + panelBorder,
              },
            },
            h(
              'button',
              {
                type: 'button',
                onClick: () => {
                  const next = !adapterPluginsOpen;
                  setAdapterPluginsOpen(next);
                  if (next) refreshAdapterPlugins(false);
                },
                style: {
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: '6px',
                  width: '100%',
                  background: 'transparent',
                  border: 'none',
                  padding: 0,
                  color: panelFg,
                  cursor: 'pointer',
                  textAlign: 'left',
                },
              },
              h(
                'span',
                { style: { fontSize: '11.5px', fontWeight: 600, color: panelFg } },
                '深度追踪插件中心（热插拔 & 社区下载）',
              ),
              h(
                'span',
                { style: { fontSize: '10px', color: panelMuted } },
                adapterPluginsOpen ? '收起 ▲' : '展开管理 ▼',
              ),
            ),
            adapterPluginsOpen
              ? (() => {
                  const miniPluginBtn = {
                    border: '1px solid ' + panelBorder,
                    background: 'transparent',
                    color: panelMuted,
                    borderRadius: '5px',
                    padding: '1px 6px',
                    fontSize: '10px',
                    cursor: 'pointer',
                    flexShrink: 0,
                  };
                  const allList = Array.isArray(adapterPluginsData.plugins)
                    ? adapterPluginsData.plugins
                    : [];
                  const filteredList = allList.filter((it) => {
                    if (adapterPluginTab === 'external') return !it.isBuiltin;
                    if (adapterPluginTab === 'builtin') return !!it.isBuiltin;
                    return true;
                  });
                  const tabDefs = [
                    { id: 'all', label: '全部 (' + allList.length + ')' },
                    { id: 'external', label: '已装扩展 (' + (adapterPluginsData.externalCount || 0) + ')' },
                    { id: 'builtin', label: '内置 (' + (adapterPluginsData.builtinCount || 0) + ')' },
                    { id: 'market', label: '社区下载' },
                  ];
                  return h(
                    'div',
                    { style: { display: 'flex', flexDirection: 'column', gap: '5px', marginTop: '2px' } },
                    // Header stats + folder / reload actions
                    h(
                      'div',
                      {
                        style: {
                          display: 'flex',
                          alignItems: 'center',
                          justifyContent: 'space-between',
                          gap: '6px',
                          fontSize: '10px',
                          color: panelMuted,
                        },
                      },
                      h(
                        'span',
                        null,
                        '已启用 ' +
                          (adapterPluginsData.enabledCount || 0) +
                          ' / ' +
                          allList.length +
                          ' 个适配器 · 开关实时生效',
                      ),
                      h(
                        'div',
                        { style: { display: 'flex', gap: '4px' } },
                        h(
                          'button',
                          {
                            type: 'button',
                            style: miniPluginBtn,
                            onClick: () => {
                              refreshAdapterPlugins(true);
                              setActionToast('已重新扫描插件目录');
                              setTimeout(() => setActionToast(''), 1600);
                            },
                          },
                          '扫描',
                        ),
                        h(
                          'button',
                          {
                            type: 'button',
                            style: miniPluginBtn,
                            onClick: () => {
                              tauriInvoke('adapter_plugins_open_dir', {})
                                .then(() => {
                                  setActionToast('已打开插件目录');
                                  setTimeout(() => setActionToast(''), 1600);
                                })
                                .catch((err) => setAdapterPluginErr(String(err || '打开目录失败')));
                            },
                          },
                          '打开目录',
                        ),
                      ),
                    ),
                    // Tabs
                    h(
                      'div',
                      { style: { display: 'flex', gap: '4px', flexWrap: 'wrap' } },
                      tabDefs.map((t) => {
                        const active = adapterPluginTab === t.id;
                        return h(
                          'button',
                          {
                            key: 'ap-tab-' + t.id,
                            type: 'button',
                            onClick: () => {
                              setAdapterPluginTab(t.id);
                              if (t.id === 'market' && !adapterRegistry) {
                                refreshAdapterRegistry();
                              }
                            },
                            style: {
                              padding: '2px 7px',
                              borderRadius: '999px',
                              fontSize: '10px',
                              fontWeight: active ? 600 : 400,
                              cursor: 'pointer',
                              border:
                                '1px solid ' +
                                (active ? unifiedPanelTheme.accentBorder : panelBorder),
                              background: active ? unifiedPanelTheme.accentBg : 'transparent',
                              color: active ? unifiedPanelTheme.accent : panelMuted,
                            },
                          },
                          t.label,
                        );
                      }),
                    ),
                    // Install by URL / JSON / Local File bar
                    h(
                      'div',
                      { style: { display: 'flex', gap: '4px', alignItems: 'center' } },
                      h('input', {
                        type: 'text',
                        placeholder: '粘贴 GitHub / HTTPS 插件链接(.json/.zip) 或 JSON',
                        value: adapterInstallInput,
                        onChange: (ev) => setAdapterInstallInput(ev.target.value),
                        style: {
                          flex: 1,
                          minWidth: 0,
                          padding: '4px 6px',
                          borderRadius: '7px',
                          fontSize: '10.5px',
                          background: 'rgba(128,128,128,0.12)',
                          border: '1px solid ' + panelBorder,
                          color: panelFg,
                          outline: 'none',
                          boxSizing: 'border-box',
                        },
                      }),
                      h(
                        'button',
                        {
                          type: 'button',
                          disabled: adapterPluginBusy,
                          onClick: installAdapterFromInput,
                          style: {
                            padding: '4px 8px',
                            borderRadius: '7px',
                            border: '1px solid ' + unifiedPanelTheme.accentBorder,
                            background: unifiedPanelTheme.accentBg,
                            color: unifiedPanelTheme.accent,
                            fontSize: '10.5px',
                            fontWeight: 600,
                            cursor: adapterPluginBusy ? 'wait' : 'pointer',
                            flexShrink: 0,
                          },
                        },
                        adapterPluginBusy ? '安装中…' : '下载安装',
                      ),
                      h(
                        'button',
                        {
                          type: 'button',
                          disabled: adapterPluginBusy,
                          title: '选择本地 .json 或 .zip 插件包',
                          onClick: installAdapterFromLocalFile,
                          style: {
                            padding: '4px 7px',
                            borderRadius: '7px',
                            border: '1px solid ' + panelBorder,
                            background: 'transparent',
                            color: panelFg,
                            fontSize: '10.5px',
                            cursor: 'pointer',
                            flexShrink: 0,
                          },
                        },
                        '本地包',
                      ),
                    ),
                    // Market Tab OR Installed/Builtin List
                    adapterPluginTab === 'market'
                      ? h(
                          'div',
                          { style: { display: 'flex', flexDirection: 'column', gap: '4px' } },
                          h(
                            'div',
                            { style: { display: 'flex', gap: '4px', alignItems: 'center' } },
                            h('input', {
                              type: 'text',
                              placeholder: '社区市场索引 registry.json 地址',
                              value: adapterRegistryUrlInput,
                              onChange: (ev) => setAdapterRegistryUrlInput(ev.target.value),
                              style: {
                                flex: 1,
                                minWidth: 0,
                                padding: '3px 6px',
                                borderRadius: '6px',
                                fontSize: '10px',
                                background: 'rgba(128,128,128,0.1)',
                                border: '1px solid ' + panelBorder,
                                color: panelFg,
                                outline: 'none',
                              },
                            }),
                            h(
                              'button',
                              {
                                type: 'button',
                                style: miniPluginBtn,
                                onClick: () => refreshAdapterRegistry(adapterRegistryUrlInput),
                              },
                              adapterRegistryLoading ? '刷新中…' : '刷新市场',
                            ),
                          ),
                          adapterRegistry && adapterRegistry.warning
                            ? h(
                                'div',
                                { style: { fontSize: '9.5px', color: panelMuted } },
                                adapterRegistry.warning,
                              )
                            : null,
                          h(
                            'div',
                            {
                              style: {
                                display: 'flex',
                                flexDirection: 'column',
                                gap: '4px',
                                maxHeight: '148px',
                                overflowY: 'auto',
                                paddingRight: '2px',
                              },
                            },
                            ((adapterRegistry && adapterRegistry.entries) || []).map((ent) =>
                              h(
                                'div',
                                {
                                  key: 'ap-mkt-' + ent.id,
                                  style: {
                                    display: 'flex',
                                    alignItems: 'center',
                                    justifyContent: 'space-between',
                                    gap: '6px',
                                    padding: '5px 6px',
                                    borderRadius: '7px',
                                    background: 'rgba(128,128,128,0.08)',
                                    fontSize: '10.5px',
                                    color: panelFg,
                                  },
                                },
                                h(
                                  'div',
                                  { style: { display: 'flex', flexDirection: 'column', gap: '1px', minWidth: 0 } },
                                  h(
                                    'div',
                                    { style: { display: 'flex', alignItems: 'center', gap: '4px' } },
                                    h(
                                      'span',
                                      { style: { fontWeight: 600, fontSize: '10.5px' } },
                                      ent.name,
                                    ),
                                    h(
                                      'span',
                                      {
                                        style: {
                                          padding: '0 4px',
                                          borderRadius: '999px',
                                          fontSize: '9px',
                                          background:
                                            ent.runtimeKind === 'command'
                                              ? 'rgba(245,158,11,0.18)'
                                              : 'rgba(16,185,129,0.16)',
                                          color:
                                            ent.runtimeKind === 'command' ? '#fbbf24' : '#6ee7b7',
                                        },
                                      },
                                      ent.runtimeKind === 'command' ? '脚本' : '零代码',
                                    ),
                                  ),
                                  h(
                                    'span',
                                    {
                                      style: {
                                        fontSize: '9.5px',
                                        color: panelMuted,
                                        overflow: 'hidden',
                                        textOverflow: 'ellipsis',
                                        whiteSpace: 'nowrap',
                                      },
                                      title: ent.description,
                                    },
                                    (ent.exes || []).join(', ') + ' · ' + ent.description,
                                  ),
                                ),
                                h(
                                  'button',
                                  {
                                    type: 'button',
                                    disabled: adapterPluginBusy,
                                    onClick: () => installMarketEntry(ent),
                                    style: {
                                      padding: '2px 7px',
                                      borderRadius: '6px',
                                      fontSize: '10px',
                                      fontWeight: 600,
                                      cursor: 'pointer',
                                      flexShrink: 0,
                                      border:
                                        '1px solid ' +
                                        (ent.installed
                                          ? 'rgba(16,185,129,0.4)'
                                          : unifiedPanelTheme.accentBorder),
                                      background: ent.installed
                                        ? 'rgba(16,185,129,0.14)'
                                        : unifiedPanelTheme.accentBg,
                                      color: ent.installed ? '#6ee7b7' : unifiedPanelTheme.accent,
                                    },
                                  },
                                  ent.installed ? '重装' : '下载',
                                ),
                              ),
                            ),
                          ),
                        )
                      : h(
                          'div',
                          {
                            style: {
                              display: 'flex',
                              flexDirection: 'column',
                              gap: '3px',
                              maxHeight: '154px',
                              overflowY: 'auto',
                              paddingRight: '2px',
                            },
                          },
                          filteredList.length === 0
                            ? h(
                                'div',
                                { style: { fontSize: '10px', color: panelMuted, padding: '6px 2px' } },
                                '暂无已安装的外部扩展插件，可切换到“社区下载”或粘贴 GitHub 链接安装。',
                              )
                            : filteredList.map((item) => {
                                const kindLabel = item.isBuiltin
                                  ? '内置'
                                  : item.runtimeKind === 'command'
                                    ? '脚本'
                                    : '零代码';
                                const kindBg = item.isBuiltin
                                  ? 'rgba(148,163,184,0.16)'
                                  : item.runtimeKind === 'command'
                                    ? 'rgba(245,158,11,0.18)'
                                    : 'rgba(56,189,248,0.16)';
                                return h(
                                  'div',
                                  {
                                    key: 'ap-item-' + item.id,
                                    style: {
                                      display: 'flex',
                                      flexDirection: 'column',
                                      gap: '3px',
                                      padding: '4px 6px',
                                      borderRadius: '7px',
                                      background: 'rgba(128,128,128,0.08)',
                                      opacity: item.enabled ? 1 : 0.72,
                                      fontSize: '10.5px',
                                      color: panelFg,
                                    },
                                  },
                                  h(
                                    'div',
                                    {
                                      style: {
                                        display: 'flex',
                                        alignItems: 'center',
                                        justifyContent: 'space-between',
                                        gap: '5px',
                                      },
                                    },
                                    h(
                                      'div',
                                      { style: { display: 'flex', alignItems: 'center', gap: '4px', minWidth: 0 } },
                                      h(
                                        'span',
                                        {
                                          style: {
                                            padding: '1px 5px',
                                            borderRadius: '999px',
                                            fontSize: '9px',
                                            background: kindBg,
                                            flexShrink: 0,
                                          },
                                        },
                                        kindLabel,
                                      ),
                                      h(
                                        'span',
                                        {
                                          title:
                                            (item.description || '') +
                                            (item.exes && item.exes.length
                                              ? '\n目标进程: ' + item.exes.join(', ')
                                              : ''),
                                          style: {
                                            overflow: 'hidden',
                                            textOverflow: 'ellipsis',
                                            whiteSpace: 'nowrap',
                                            fontWeight: 500,
                                          },
                                        },
                                        item.name,
                                      ),
                                    ),
                                    h(
                                      'div',
                                      { style: { display: 'flex', alignItems: 'center', gap: '3px', flexShrink: 0 } },
                                      h(
                                        'button',
                                        {
                                          type: 'button',
                                          title: item.requiresTrustConfirm
                                            ? '该插件会执行外部命令，请确认信任后启用'
                                            : item.enabled
                                              ? '点击热禁用（即时生效，无需重启）'
                                              : '点击热启用（即时生效，无需重启）',
                                          onClick: () => toggleAdapterPlugin(item),
                                          style: {
                                            padding: '1px 6px',
                                            borderRadius: '999px',
                                            fontSize: '9.5px',
                                            fontWeight: 600,
                                            cursor: 'pointer',
                                            border:
                                              '1px solid ' +
                                              (item.requiresTrustConfirm
                                                ? 'rgba(245,158,11,0.55)'
                                                : item.enabled
                                                  ? 'rgba(16,185,129,0.5)'
                                                  : panelBorder),
                                            background: item.requiresTrustConfirm
                                              ? 'rgba(245,158,11,0.16)'
                                              : item.enabled
                                                ? 'rgba(16,185,129,0.16)'
                                                : 'transparent',
                                            color: item.requiresTrustConfirm
                                              ? '#fbbf24'
                                              : item.enabled
                                                ? '#6ee7b7'
                                                : panelMuted,
                                          },
                                        },
                                        item.requiresTrustConfirm
                                          ? '信任并启用'
                                          : item.enabled
                                            ? '已启用'
                                            : '已停用',
                                      ),
                                      h(
                                        'button',
                                        {
                                          type: 'button',
                                          title: '干跑自检',
                                          style: miniPluginBtn,
                                          onClick: () => {
                                            setAdapterTestNote('正在自检 ' + item.name + '…');
                                            tauriInvoke('adapter_plugins_test', { id: item.id })
                                              .then((rep) => {
                                                const txt = rep.ok
                                                  ? '✓ ' +
                                                    item.id +
                                                    ' (' +
                                                    (rep.latencyMs || 0) +
                                                    'ms): ' +
                                                    ((rep.lines && rep.lines[0]) || '通过')
                                                  : '✗ ' + item.id + ': ' + (rep.error || '失败');
                                                setAdapterTestNote(txt);
                                              })
                                              .catch((err) => setAdapterTestNote('✗ 自检出错: ' + String(err || '')));
                                          },
                                        },
                                        '测试',
                                      ),
                                      h(
                                        'button',
                                        {
                                          type: 'button',
                                          title: '导出可分享插件 JSON 到剪贴板',
                                          style: miniPluginBtn,
                                          onClick: () => {
                                            tauriInvoke('adapter_plugins_export', { id: item.id })
                                              .then((jsonStr) => {
                                                if (navigator.clipboard && navigator.clipboard.writeText) {
                                                  navigator.clipboard
                                                    .writeText(String(jsonStr || ''))
                                                    .catch((err) => setAdapterPluginErr(String(err || '')));
                                                }
                                                setActionToast('已复制 ' + item.id + ' 分享包 JSON');
                                                setTimeout(() => setActionToast(''), 1800);
                                              })
                                              .catch((err) => setAdapterPluginErr(String(err || '导出失败')));
                                          },
                                        },
                                        '导出',
                                      ),
                                      !item.isBuiltin
                                        ? h(
                                            'button',
                                            {
                                              type: 'button',
                                              title: '彻底卸载该外部插件',
                                              onClick: () => {
                                                tauriInvoke('adapter_plugins_uninstall', { id: item.id })
                                                  .then(() => {
                                                    refreshAdapterPlugins(false);
                                                    if (adapterRegistry) refreshAdapterRegistry();
                                                    setActionToast('已卸载 ' + item.name);
                                                    setTimeout(() => setActionToast(''), 1800);
                                                  })
                                                  .catch((err) => setAdapterPluginErr(String(err || '卸载失败')));
                                              },
                                              style: {
                                                border: '1px solid rgba(248,113,113,0.4)',
                                                background: 'rgba(248,113,113,0.1)',
                                                color: '#fca5a5',
                                                borderRadius: '5px',
                                                padding: '1px 5px',
                                                fontSize: '10px',
                                                cursor: 'pointer',
                                              },
                                            },
                                            '卸载',
                                          )
                                        : null,
                                    ),
                                  ),
                                  item.requiresTrustConfirm && item.commandPreview
                                    ? h(
                                        'div',
                                        { style: { fontSize: '9.5px', color: '#fbbf24' } },
                                        '待信任命令: ' + item.commandPreview,
                                      )
                                    : null,
                                );
                              }),
                        ),
                    adapterTestNote
                      ? h(
                          'div',
                          {
                            style: {
                              fontSize: '10px',
                              padding: '3px 6px',
                              borderRadius: '6px',
                              background: 'rgba(56,189,248,0.12)',
                              color: '#7dd3fc',
                            },
                          },
                          adapterTestNote,
                        )
                      : null,
                    adapterPluginErr
                      ? h('div', { style: { fontSize: '10px', color: '#fca5a5' } }, adapterPluginErr)
                      : null,
                  );
                })()
              : null,
          ),
          // Universal Anything UI Grounding: Builtin + Learned App Profiles & Shareable Cards
          h(
            'div',
            {
              'data-dsb-ui-profiles-settings': '1',
              style: {
                display: 'flex',
                flexDirection: 'column',
                gap: '5px',
                padding: '7px 8px',
                borderRadius: '10px',
                background: panelRowBg,
                border: '1px solid ' + panelBorder,
              },
            },
            h(
              'button',
              {
                type: 'button',
                onClick: () => {
                  const next = !uiProfilesOpen;
                  setUiProfilesOpen(next);
                  if (next) refreshUiProfiles();
                },
                style: {
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: '6px',
                  width: '100%',
                  background: 'transparent',
                  border: 'none',
                  padding: 0,
                  color: panelFg,
                  cursor: 'pointer',
                  textAlign: 'left',
                },
              },
              h(
                'span',
                { style: { fontSize: '11.5px', fontWeight: 600, color: panelFg } },
                '软件识别档案（出厂配方 & 自学习适配卡）',
              ),
              h(
                'span',
                { style: { fontSize: '10px', color: panelMuted } },
                uiProfilesOpen ? '收起 ▲' : '展开管理 ▼',
              ),
            ),
            uiProfilesOpen
              ? h(
                  'div',
                  { style: { display: 'flex', flexDirection: 'column', gap: '5px', marginTop: '2px' } },
                  h(
                    'div',
                    {
                      style: {
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'space-between',
                        fontSize: '10px',
                        color: panelMuted,
                      },
                    },
                    h(
                      'span',
                      null,
                      '共 ' +
                        (uiProfilesData.profiles || []).length +
                        ' 款档案 · 本地找框增强包：' +
                        (uiProfilesData.visualBoxProvider || '未安装（本体零显存）'),
                    ),
                  ),
                  h(
                    'div',
                    {
                      style: {
                        display: 'flex',
                        flexDirection: 'column',
                        gap: '3px',
                        maxHeight: '116px',
                        overflowY: 'auto',
                        paddingRight: '2px',
                      },
                    },
                    (uiProfilesData.profiles || []).map((prof) => {
                      const srcLabel =
                        prof.source === 'imported'
                          ? '已导入'
                          : prof.source === 'learned'
                            ? '已学习'
                            : '出厂';
                      const srcBg =
                        prof.source === 'imported'
                          ? 'rgba(56, 189, 248, 0.16)'
                          : prof.source === 'learned'
                            ? 'rgba(16, 185, 129, 0.16)'
                            : 'rgba(148, 163, 184, 0.14)';
                      return h(
                        'div',
                        {
                          key: 'ui-prof-' + prof.exeName,
                          style: {
                            display: 'flex',
                            alignItems: 'center',
                            justifyContent: 'space-between',
                            gap: '6px',
                            padding: '3px 6px',
                            borderRadius: '7px',
                            background: 'rgba(128,128,128,0.08)',
                            fontSize: '10.5px',
                            color: panelFg,
                          },
                        },
                        h(
                          'div',
                          { style: { display: 'flex', alignItems: 'center', gap: '5px', minWidth: 0 } },
                          h(
                            'span',
                            {
                              style: {
                                padding: '1px 5px',
                                borderRadius: '999px',
                                fontSize: '9.5px',
                                background: srcBg,
                                flexShrink: 0,
                              },
                            },
                            srcLabel,
                          ),
                          h(
                            'span',
                            {
                              title: prof.notes || prof.exeName,
                              style: {
                                overflow: 'hidden',
                                textOverflow: 'ellipsis',
                                whiteSpace: 'nowrap',
                              },
                            },
                            (prof.displayName || prof.exeName) + ' (' + prof.exeName + ')',
                          ),
                        ),
                        h(
                          'div',
                          { style: { display: 'flex', alignItems: 'center', gap: '4px', flexShrink: 0 } },
                          h(
                            'button',
                            {
                              type: 'button',
                              title: '导出该软件适配卡 JSON 到剪贴板',
                              onClick: () => {
                                tauriInvoke('pick_export_profile', { exeName: prof.exeName })
                                  .then((jsonStr) => {
                                    if (navigator.clipboard && navigator.clipboard.writeText) {
                                      navigator.clipboard.writeText(String(jsonStr || '')).catch(() => {});
                                    }
                                    setActionToast('已复制 ' + prof.exeName + ' 适配卡');
                                    setTimeout(() => setActionToast(''), 1800);
                                  })
                                  .catch(() => {});
                              },
                              style: {
                                border: '1px solid ' + panelBorder,
                                background: 'transparent',
                                color: panelMuted,
                                borderRadius: '5px',
                                padding: '1px 5px',
                                fontSize: '10px',
                                cursor: 'pointer',
                              },
                            },
                            '导出',
                          ),
                          prof.source !== 'builtin'
                            ? h(
                                'button',
                                {
                                  type: 'button',
                                  title: '重置本软件的学习记录',
                                  onClick: () => {
                                    tauriInvoke('pick_reset_profile', { exeName: prof.exeName })
                                      .then(() => {
                                        refreshUiProfiles();
                                        setActionToast('已重置 ' + prof.exeName + ' 学习记录');
                                        setTimeout(() => setActionToast(''), 1800);
                                      })
                                      .catch(() => {});
                                  },
                                  style: {
                                    border: '1px solid rgba(248,113,113,0.4)',
                                    background: 'rgba(248,113,113,0.1)',
                                    color: '#fca5a5',
                                    borderRadius: '5px',
                                    padding: '1px 5px',
                                    fontSize: '10px',
                                    cursor: 'pointer',
                                  },
                                },
                                '重置',
                              )
                            : null,
                        ),
                      );
                    }),
                  ),
                  h(
                    'div',
                    { style: { display: 'flex', gap: '4px', alignItems: 'center' } },
                    h('input', {
                      type: 'text',
                      placeholder: '粘贴 .atp-profile.json 适配卡内容以导入',
                      value: importProfileText,
                      onChange: (ev) => setImportProfileText(ev.target.value),
                      style: {
                        flex: 1,
                        padding: '4px 6px',
                        borderRadius: '7px',
                        fontSize: '10.5px',
                        background: 'rgba(128,128,128,0.12)',
                        border: '1px solid ' + panelBorder,
                        color: panelFg,
                        outline: 'none',
                        boxSizing: 'border-box',
                      },
                    }),
                    h(
                      'button',
                      {
                        type: 'button',
                        onClick: () => {
                          const raw = importProfileText.trim();
                          if (!raw) {
                            setImportProfileErr('请先粘贴适配卡 JSON');
                            return;
                          }
                          setImportProfileErr('');
                          tauriInvoke('pick_import_profile', { rawJson: raw })
                            .then((saved) => {
                              setImportProfileText('');
                              refreshUiProfiles();
                              setActionToast('已导入 ' + (saved && saved.exeName ? saved.exeName : '适配卡'));
                              setTimeout(() => setActionToast(''), 1800);
                            })
                            .catch((err) => {
                              setImportProfileErr(String(err || '导入失败'));
                            });
                        },
                        style: {
                          padding: '4px 8px',
                          borderRadius: '7px',
                          border: '1px solid ' + unifiedPanelTheme.accentBorder,
                          background: unifiedPanelTheme.accentBg,
                          color: unifiedPanelTheme.accent,
                          fontSize: '10.5px',
                          fontWeight: 600,
                          cursor: 'pointer',
                          flexShrink: 0,
                        },
                      },
                      '导入适配卡',
                    ),
                  ),
                  importProfileErr
                    ? h('div', { style: { fontSize: '10px', color: '#fca5a5' } }, importProfileErr)
                    : null,
                )
              : null,
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
          'data-dsb-popup-bottom': isTopDockMenu ? undefined : String(bottomPx),
          'data-dsb-popup-min-height': String(settingsView ? 438 : 308),
          style: {
            position: 'fixed',
            top: isTopDockMenu ? menuTopPx + 'px' : 'auto',
            bottom: isTopDockMenu ? 'auto' : bottomPx + 'px',
            left: leftPx + 'px',
            display: 'flex',
            flexDirection: 'row',
            alignItems: isTopDockMenu ? 'flex-start' : 'flex-end',
            gap: colGap + 'px',
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
                    maxHeight: colMaxH + 'px',
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
                            { style: { display: 'inline-flex', alignItems: 'center', gap: '4px', flexShrink: 0 } },
                            meta.providerKind === 'acp'
                              ? h(
                                  'span',
                                  {
                                    style: {
                                      fontSize: '9px',
                                      padding: '0 4px',
                                      borderRadius: '4px',
                                      background: meta.accentBg,
                                      color: meta.accent,
                                      lineHeight: '14px',
                                      fontWeight: 600,
                                    },
                                  },
                                  'ACP',
                                )
                              : meta.providerKind === 'universal'
                                ? h(
                                    'span',
                                    {
                                      style: {
                                        fontSize: '9px',
                                        padding: '0 4px',
                                        borderRadius: '4px',
                                        background: meta.accentBg,
                                        color: meta.accent,
                                        lineHeight: '14px',
                                        fontWeight: 600,
                                      },
                                    },
                                    '万能',
                                  )
                                : null,
                            h(
                              'span',
                              { style: { opacity: isBrowsedH ? 0.9 : 0.5, fontSize: '11.5px' } },
                              (isActiveH ? '● ' : '') + '›',
                            ),
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
                    maxHeight: colMaxH + 'px',
                    marginBottom: wsMarginBottom + 'px',
                    borderRadius: '14px',
                    padding: '6px',
                    background: panelBg,
                    color: panelFg,
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
                      ? h(
                          'div',
                          {
                            style: {
                              padding: '10px 8px',
                              display: 'flex',
                              flexDirection: 'column',
                              alignItems: 'center',
                              gap: '8px',
                              textAlign: 'center',
                            },
                          },
                          h('div', { style: { fontSize: '12px', opacity: 0.65 } }, '暂无历史工作区'),
                          h(
                            'button',
                            {
                              type: 'button',
                              onClick: async () => {
                                const defPath = await ensureDefaultWorkspacePath();
                                if (defPath) {
                                  await createWorkspaceAtPath(defPath, browsedHarness);
                                }
                              },
                              style: {
                                width: '100%',
                                padding: '6px 9px',
                                borderRadius: '9px',
                                border: '1px solid ' + browsedTheme.accentBorder,
                                background: browsedTheme.accentBg,
                                color: browsedTheme.accent,
                                fontSize: '11.5px',
                                fontWeight: 550,
                                cursor: 'pointer',
                              },
                            },
                            '✨ 在默认工作区直接开始对话',
                          ),
                        )
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
                        maxHeight: colMaxH + 'px',
                        marginBottom: submenuMarginBottom + 'px',
                        borderRadius: '14px',
                        padding: '6px',
                        background: panelBg,
                        color: panelFg,
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
