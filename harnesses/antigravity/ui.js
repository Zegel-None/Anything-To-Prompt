// harnesses/antigravity/ui.js — Dedicated Antigravity Controller (agy.exe CLI + Desktop agentapi Sidecar)
(function (sdk) {
  const { h } = sdk;

  const DEFAULTS = {
    models: [
      { id: 'gemini-4-neon-high', label: 'Gemini 4 Neon', tag: 'New' },
      { id: 'gemini-4-carbon-medium', label: 'Gemini 4 Carbon', tag: 'New' },
      { id: 'gemini-4-argon-medium', label: 'Gemini 4 Argon', tag: 'Leaving Soon' },
      { id: 'gemini-4-hydrogen-medium', label: 'Gemini 4 Hydrogen', tag: 'New' },
      { id: 'gemini-pro-orchestrator-neon', label: 'Orchestrator', tag: 'Opus 5.5' },
      { id: 'gemini-3.8-flash-medium', label: 'Gemini 3.8 Flash', tag: 'Leaving Soon' },
      { id: 'claude-opus-5-5-medium', label: 'Claude Opus 5.5', tag: 'New' },
      { id: 'claude-sonnet-5-5-medium', label: 'Claude Sonnet 5.5', tag: 'New' },
      { id: 'gpt-oss-120b-medium', label: 'GPT-OSS 120B (Medium)', tag: 'Notice' },
    ],
    efforts: [
      { id: 'low', label: 'Low' },
      { id: 'medium', label: 'Medium' },
      { id: 'high', label: 'High' },
      { id: 'xhigh', label: 'XHigh' },
      { id: 'max', label: 'Max' },
    ],
    modes: [
      { id: 'turbo', label: 'Turbo', desc: '--dangerously-skip-permissions · 无限制执行', danger: true },
      { id: 'auto', label: 'Auto', desc: '--mode auto · 自动评估风险执行', danger: false },
      { id: 'default', label: 'Default', desc: '--mode accept-edits · 敏感操作前询问', danger: false },
      { id: 'request-review', label: 'Request Review', desc: '--sandbox --mode plan · 沙箱与规划审批', danger: false },
    ],
    actions: [
      { id: '/plan', label: '/plan', desc: '先出步骤规划再执行' },
      { id: '/goal', label: '/goal', desc: '长期目标自主推进模式' },
      { id: '/browser', label: '/browser', desc: '调用内置浏览器查验网页' },
      { id: '/compact', label: '/compact', desc: '立即压缩上下文释放窗口' },
      { id: '/boost', label: '/boost', desc: '深度推理与多维验证' },
      { id: '/grill-me', label: '/grill-me', desc: '交互式需求对齐访谈' },
      { id: '/learn', label: '/learn', desc: '沉淀规则到长期记忆' },
    ],
    defaultModel: 'gemini-4-neon-high',
    defaultEffort: 'high',
    defaultMode: 'turbo',
  };

  function getDefaults() {
    return DEFAULTS;
  }

  function handleSlashAction(actionId, ctx) {
    const id = String(actionId || '').trim();
    if (!id) return false;
    if (id === '/compact' && ctx.selection && ctx.selection.sessionId) {
      ctx.runImmediatePrompt('/compact');
      return true;
    }
    if (id === '/plan') {
      ctx.updatePrefs({ mode: 'request-review' });
      ctx.insertText('/plan ');
      return true;
    }
    ctx.insertText(id + ' ');
    return true;
  }

  function buildPlusMenu(ctx) {
    const { t, icons, opts, prefs, subView, setSubView, closePopover, sectionHdr, popRow } = ctx;
    const actions =
      Array.isArray(opts && opts.actions) && opts.actions.length > 0
        ? opts.actions
        : DEFAULTS.actions;

    if (subView === 'ag-actions') {
      return [
        h(
          'div',
          {
            key: 'ag-act-hdr',
            style: {
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              padding: '3px 10px 6px',
            },
          },
          h(
            'button',
            {
              type: 'button',
              onClick: (ev) => {
                ev.stopPropagation();
                setSubView(null);
              },
              style: {
                border: 'none',
                background: 'transparent',
                color: t.muted,
                fontSize: '11.5px',
                cursor: 'pointer',
                padding: 0,
              },
            },
            '← Add Context',
          ),
          h('span', { style: { fontSize: '11px', color: t.muted } }, 'Antigravity Actions'),
        ),
        ...actions.map((a) =>
          popRow('act-' + a.id, icons.slash(t.accent), a.label, a.desc, false, () => {
            setSubView(null);
            closePopover();
            handleSlashAction(a.id, ctx);
          }),
        ),
      ];
    }

    const sandboxOn = Boolean(prefs && prefs.sandboxEnabled);
    return [
      sectionHdr('ag-hdr', 'Antigravity · Add Context', false),
      popRow('ag-files', icons.clip(t.muted), 'Files', '选择本地文件附加给 agy.exe', false, () => {
        closePopover();
        ctx.pickFiles(false);
      }),
      popRow('ag-folder', icons.folder(t.muted), 'Folder (--add-dir)', '以 --add-dir 授权工作目录', false, () => {
        closePopover();
        ctx.pickFolder();
      }),
      popRow('ag-at', icons.at(t.muted), 'Mentions (@)', '搜索本地文件或目录快速插入引用', false, () => {
        closePopover();
        ctx.openMentionPicker();
      }),
      popRow('ag-act', icons.actions(t.muted), 'Actions (/)', '展开 /plan /goal /grill-me /boost /compact', false, () => {
        setSubView('ag-actions');
      }),
      popRow('ag-browser', icons.chrome(t.muted), 'Browser (/browser)', '调用内置浏览器执行网页任务', false, () => {
        closePopover();
        handleSlashAction('/browser', ctx);
      }),
      sectionHdr('ag-flags', 'CLI 运行参数', true),
      popRow(
        'ag-sandbox',
        sdk.icons.shield(sandboxOn ? t.accent : t.muted),
        '终端沙箱 (--sandbox)',
        sandboxOn ? '已开启：限制终端高危命令' : '点击为本次 agy.exe 会话开启沙箱',
        sandboxOn,
        () => {
          ctx.updatePrefs({ sandboxEnabled: !sandboxOn });
        },
      ),
    ];
  }

  function getActiveChips(ctx) {
    const chips = [];
    if (ctx.prefs && ctx.prefs.sandboxEnabled) {
      chips.push({
        id: 'ag-sandbox',
        label: '🛡 Sandbox',
        title: '已开启 agy.exe --sandbox 终端沙箱（点击关闭）',
        onRemove: () => ctx.updatePrefs({ sandboxEnabled: false }),
      });
    }
    return chips;
  }

  function buildTurnPayload(ctx) {
    const prefs = ctx.prefs || {};
    const mode = ctx.resolvedMode || DEFAULTS.defaultMode;
    return {
      projectId: (ctx.selection && ctx.selection.workspaceId) || 'default-cli-project',
      executionMode: mode,
      model: ctx.resolvedModel || DEFAULTS.defaultModel,
      effort: ctx.resolvedEffort || DEFAULTS.defaultEffort,
      sandboxEnabled: Boolean(prefs.sandboxEnabled),
      planMode: mode === 'request-review' || mode === 'plan',
    };
  }

  function renderRightControls(ctx) {
    const modelText = (ctx.avail && ctx.avail.modelLabel) || 'Gemini 4 Neon';
    const effortText = (ctx.avail && ctx.avail.effortLabel) || 'High';
    const accessText = (ctx.avail && ctx.avail.accessLabel) || 'Turbo';
    return h(
      'div',
      {
        'data-dsb-harness-right': 'antigravity',
        style: {
          display: 'inline-flex',
          alignItems: 'center',
          gap: '10px',
          fontSize: '12px',
          color: 'var(--atp-muted, rgba(229, 231, 235, 0.78))',
          paddingRight: '4px',
          userSelect: 'none',
          flexShrink: 0,
        },
      },
      h(
        'span',
        { style: { display: 'inline-flex', alignItems: 'center', gap: '4px' } },
        h('span', { style: { color: 'var(--atp-fg, rgba(243, 244, 246, 0.88))' } }, modelText),
        h('span', { style: { opacity: 0.6 } }, effortText),
      ),
      h(
        'span',
        { style: { display: 'inline-flex', alignItems: 'center', gap: '4px' } },
        h('span', {
          style: {
            width: '7px',
            height: '7px',
            borderRadius: '999px',
            background:
              'conic-gradient(#4285F4 0 90deg, #34A853 90deg 180deg, #FBBC04 180deg 270deg, #EA4335 270deg 360deg)',
          },
        }),
        accessText,
      ),
    );
  }

  return {
    css: '',
    getDefaults,
    buildPlusMenu,
    getActiveChips,
    handleSlashAction,
    buildTurnPayload,
    renderRightControls,
  };
})
