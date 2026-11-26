// harnesses/claude_code/ui.js — Dedicated Anthropic Claude Code Controller (claude.exe CLI)
(function (sdk) {
  const { h } = sdk;

  const DEFAULTS = {
    models: [
      { id: 'claude-opus-5-5', label: 'Opus 5.5', tag: 'Default' },
      { id: 'claude-fable-5-2', label: 'Fable 5.2', tag: '最强' },
      { id: 'claude-sonnet-5-5', label: 'Sonnet 5.5', tag: '均衡' },
      { id: 'claude-haiku-5-5', label: 'Haiku 5.5', tag: '极速' },
    ],
    efforts: [
      { id: 'low', label: 'Low' },
      { id: 'medium', label: 'Medium' },
      { id: 'high', label: 'High' },
      { id: 'xhigh', label: 'XHigh' },
      { id: 'max', label: 'Max' },
    ],
    modes: [
      { id: 'auto', label: 'Auto', desc: '--permission-mode auto · 智能自动执行并拦截高危', danger: true },
      { id: 'acceptEdits', label: 'Accept Edits', desc: '--permission-mode acceptEdits · 自动写入代码', danger: false },
      { id: 'plan', label: 'Plan', desc: '--permission-mode plan · 只读分析与规划', danger: false },
      { id: 'manual', label: 'Manual', desc: '--permission-mode manual · 每步手动审批', danger: false },
      { id: 'dontAsk', label: "Don't Ask", desc: '--permission-mode dontAsk · 仅放行预授权工具', danger: false },
      { id: 'bypassPermissions', label: 'Bypass All', desc: '--permission-mode bypassPermissions · 跳过全部权限', danger: true },
    ],
    actions: [
      { id: '/plan', label: '/plan', desc: '立即切换到 --permission-mode plan 只读规划' },
      { id: '/compact', label: '/compact', desc: '立即向当前会话执行上下文压缩' },
      { id: '/review', label: '/review', desc: '审查当前工作区代码变更' },
      { id: '/cost', label: '/cost', desc: '查看当前会话真实 Token 消耗统计' },
      { id: '/clear', label: '/clear', desc: '立即清空当前会话并开启新对话' },
    ],
    defaultModel: 'claude-opus-5-5',
    defaultEffort: 'medium',
    defaultMode: 'auto',
  };

  function getDefaults() {
    return DEFAULTS;
  }

  function handleSlashAction(actionId, ctx) {
    const id = String(actionId || '').trim();
    if (!id) return false;
    if (id === '/clear') {
      ctx.startNewSession();
      return true;
    }
    if (id === '/cost') {
      ctx.showSessionStats();
      return true;
    }
    if (id === '/plan') {
      ctx.updatePrefs({ mode: 'plan' });
      ctx.focusComposer();
      return true;
    }
    if (id === '/compact') {
      if (ctx.selection && ctx.selection.sessionId) {
        ctx.runImmediatePrompt('/compact');
      } else {
        ctx.insertText('/compact ');
      }
      return true;
    }
    if (id === '/review') {
      ctx.insertText('/review ');
      return true;
    }
    ctx.insertText(id + ' ');
    return true;
  }

  function buildPlusMenu(ctx) {
    const { t, icons, opts, prefs, closePopover, sectionHdr, popRow } = ctx;
    const actions =
      Array.isArray(opts && opts.actions) && opts.actions.length > 0
        ? opts.actions
        : DEFAULTS.actions;
    const chromeOn = Boolean(prefs && prefs.chromeEnabled);

    return [
      sectionHdr('cc-add', 'Claude Code · 添加上下文', false),
      popRow('cc-files', icons.clip(t.muted), '添加文件', '选择本地文件随 prompt 传入 claude.exe', false, () => {
        closePopover();
        ctx.pickFiles(false);
      }),
      popRow('cc-img', icons.image(t.muted), '添加图片', '选择本地截图或设计稿', false, () => {
        closePopover();
        ctx.pickFiles(true);
      }),
      popRow('cc-dir', icons.folder(t.muted), '添加工作目录 (--add-dir)', '以 --add-dir 授权给 Claude Code', false, () => {
        closePopover();
        ctx.pickFolder();
      }),
      popRow('cc-at', icons.at(t.muted), '引用本地文件 (@)', '搜索文件快速插入引用', false, () => {
        closePopover();
        ctx.openMentionPicker();
      }),
      popRow(
        'cc-chrome',
        icons.chrome(chromeOn ? t.accent : t.muted),
        'Claude in Chrome (--chrome)',
        chromeOn ? '已开启：允许调用本机 Chrome 浏览器' : '点击为本次会话启用 --chrome 浏览器工具',
        chromeOn,
        () => {
          ctx.updatePrefs({ chromeEnabled: !chromeOn });
        },
      ),
      sectionHdr('cc-act', '斜杠指令（真实执行）', true),
      ...actions.map((a) =>
        popRow('cc-act-' + a.id, icons.slash(t.accent), a.label, a.desc, false, () => {
          closePopover();
          handleSlashAction(a.id, ctx);
        }),
      ),
    ];
  }

  function getActiveChips(ctx) {
    const chips = [];
    if (ctx.prefs && ctx.prefs.chromeEnabled) {
      chips.push({
        id: 'cc-chrome',
        label: '🌐 --chrome',
        title: '已开启 Claude in Chrome（点击关闭）',
        onRemove: () => ctx.updatePrefs({ chromeEnabled: false }),
      });
    }
    return chips;
  }

  function buildTurnPayload(ctx) {
    const prefs = ctx.prefs || {};
    const mode = ctx.resolvedMode || DEFAULTS.defaultMode;
    return {
      permissionMode: mode,
      model: ctx.resolvedModel || DEFAULTS.defaultModel,
      effort: ctx.resolvedEffort || DEFAULTS.defaultEffort,
      chromeEnabled: Boolean(prefs.chromeEnabled),
      planMode: mode === 'plan',
    };
  }

  function renderRightControls(ctx) {
    const modelText = (ctx.avail && ctx.avail.modelLabel) || 'Opus 5.5';
    const effortText = (ctx.avail && ctx.avail.effortLabel) || 'Medium';
    const accessText = (ctx.avail && ctx.avail.accessLabel) || 'Auto';
    return h(
      'div',
      {
        'data-dsb-harness-right': 'claude_code',
        style: {
          display: 'inline-flex',
          alignItems: 'center',
          gap: '10px',
          fontSize: '12px',
          color: 'var(--atp-muted, #9C948A)',
          paddingRight: '4px',
          userSelect: 'none',
          flexShrink: 0,
        },
      },
      h(
        'span',
        { style: { display: 'inline-flex', alignItems: 'center', gap: '4px' } },
        h('span', { style: { color: 'var(--atp-fg, #F2EEE8)' } }, modelText),
        h('span', { style: { opacity: 0.6 } }, effortText),
      ),
      h(
        'span',
        { style: { display: 'inline-flex', alignItems: 'center', gap: '4px', color: '#D97757' } },
        h('span', {
          style: {
            width: '6px',
            height: '6px',
            borderRadius: '999px',
            background: '#D97757',
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
