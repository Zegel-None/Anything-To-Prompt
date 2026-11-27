function createAnythingToPromptPlugin() {
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
        try {
          const sp = new URLSearchParams(window.location.search || '');
          const previewDock = sp.get('dsb_dock');
          const previewVert = sp.get('dsb_vertical');
          if (previewDock || previewVert) {
            const wantVert =
              previewVert === '1' || previewVert === 'true'
                ? true
                : previewVert === '0' || previewVert === 'false'
                  ? false
                  : previewDock === 'left' || previewDock === 'right';
            applyDockState(previewDock || 'none', false, wantVert);
          }
        } catch (err) {
          void err;
        }
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
        let defaultBootstrapAttempted = false;
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
        const pickFirstExistingWorkspaceId = () => {
          try {
            const wsSnap = workspaces && workspaces.list ? workspaces.list.getSnapshot() : null;
            if (!wsSnap || wsSnap.phase === 'pending') return { ready: false, wsId: '' };
            const items = Array.isArray(wsSnap.items)
              ? wsSnap.items
              : wsSnap.byId && typeof wsSnap.byId === 'object'
                ? Object.values(wsSnap.byId)
                : [];
            const first = items.find((w) => w && w.workspaceId);
            return { ready: true, wsId: first ? String(first.workspaceId) : '' };
          } catch {
            return { ready: true, wsId: '' };
          }
        };
        const bootstrapDefaultDshSession = async () => {
          if (defaultBootstrapAttempted || openInFlight) return;
          const wsInfo = pickFirstExistingWorkspaceId();
          if (!wsInfo.ready) return;
          defaultBootstrapAttempted = true;
          openInFlight = true;
          try {
            let wsId = wsInfo.wsId;
            if (!wsId && workspaces && typeof workspaces.create === 'function') {
              const defaultPath = await ensureDefaultWorkspacePath();
              if (defaultPath) {
                sendBarReport({
                  kind: 'diag',
                  detail: 'Cold-start DSH: creating default workspace at ' + defaultPath,
                });
                const created = await workspaces.create({ path: defaultPath });
                wsId = created && created.workspaceId ? String(created.workspaceId) : '';
              }
            }
            if (wsId) {
              sendBarReport({
                kind: 'diag',
                detail: 'Cold-start DSH: auto-starting first session in workspaceId=' + wsId,
              });
              if (typeof uiWorkspace.startSession === 'function') {
                await uiWorkspace.startSession(wsId);
              } else if (typeof sessions.create === 'function') {
                const newId = await sessions.create({ workspaceId: wsId });
                if (newId && typeof uiWorkspace.openSession === 'function') {
                  uiWorkspace.openSession(newId);
                }
              }
            }
          } catch (err) {
            sendBarReport({
              kind: 'diag',
              detail: 'Cold-start DSH bootstrap failed: ' + String(err),
            });
          } finally {
            setTimeout(() => {
              openInFlight = false;
            }, 400);
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
          if (!targetId) {
            // Zero existing sessions in DSH -> automatically create default workspace + first conversation!
            if (fromTimer || (workspaces && workspaces.list && workspaces.list.getSnapshot()?.phase !== 'pending')) {
              bootstrapDefaultDshSession();
            }
            return false;
          }
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
}
