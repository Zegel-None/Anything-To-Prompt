// harnesses/codex/ui.js — Dedicated OpenAI Codex Controller (codex app-server JSON-RPC 2.0)
(function (sdk) {
  const { h } = sdk;

  const DEFAULTS = {
    models: [
      { id: 'default', label: '默认', tag: '推荐' },
      { id: 'gpt-6.2-sol', label: 'GPT-6.2 Sol', tag: '旗舰' },
      { id: 'gpt-6.2-astra', label: 'GPT-6.2 Astra', tag: '' },
      { id: 'gpt-6.1-terra', label: 'GPT-6.1 Terra', tag: '' },
      { id: 'gpt-6.1-sol', label: 'GPT-6.1 Sol', tag: '' },
      { id: 'gpt-6-astra', label: 'GPT-6 Astra', tag: '' },
      { id: 'gpt-6-sol', label: 'GPT-6 Sol', tag: '' },
      { id: 'gpt-6-luna', label: 'GPT-6 Luna', tag: '' },
      { id: 'gpt-5.6-sol', label: 'GPT-5.6 Sol', tag: '' },
      { id: 'gpt-5.6-terra', label: 'GPT-5.6 Terra', tag: '' },
    ],
    efforts: [
      { id: 'minimal', label: '极低' },
      { id: 'low', label: '低' },
      { id: 'medium', label: '中' },
      { id: 'high', label: '高' },
      { id: 'xhigh', label: '极高' },
    ],
    modes: [
      { id: 'danger-full-access', label: '完全访问', desc: 'approvalPolicy=never · dangerFullAccess', danger: true },
      { id: 'help-approve', label: '帮我批准', desc: 'approvalsReviewer=auto_review · workspaceWrite', danger: false },
      { id: 'request-approval', label: '请求批准', desc: 'approvalPolicy=on-request · workspaceWrite', danger: false },
      { id: 'read-only', label: '只读模式', desc: 'sandboxPolicy=readOnly · 仅分析不修改', danger: false },
    ],
    actions: [
      { id: '/plan', label: '/plan', desc: '切换 Codex 只读计划模式' },
      { id: '/goal', label: '/goal', desc: '设置要持续追求的目标' },
      { id: '/cost', label: '/cost', desc: '查看当前线程已消耗 Tokens' },
    ],
    defaultModel: 'default',
    defaultEffort: 'high',
    defaultMode: 'danger-full-access',
  };

  const BUILTIN_PLUGINS = [
    { id: 'documents@openai-curated', label: 'Documents', desc: 'Create and edit documents' },
    { id: 'pdf@openai-curated', label: 'PDF', desc: 'Read, create, and verify PDFs' },
    { id: 'spreadsheets@openai-curated', label: 'Spreadsheets', desc: 'Create and edit spreadsheets' },
    { id: 'presentations@openai-curated', label: 'Presentations', desc: 'Create and edit presentations' },
  ];

  function getDefaults() {
    return DEFAULTS;
  }

  function getEnabledPlugins(prefs) {
    return Array.isArray(prefs && prefs.enabledPlugins) ? prefs.enabledPlugins : [];
  }

  function togglePlugin(ctx, pluginId) {
    const cur = getEnabledPlugins(ctx.prefs);
    const exists = cur.includes(pluginId);
    const next = exists ? cur.filter((x) => x !== pluginId) : cur.concat([pluginId]);
    ctx.updatePrefs({ enabledPlugins: next });
  }

  function handleSlashAction(actionId, ctx) {
    const id = String(actionId || '').trim();
    if (!id) return false;
    if (id === '/plan') {
      const nextPlan = !(ctx.prefs && ctx.prefs.planMode);
      ctx.updatePrefs({ planMode: nextPlan });
      return true;
    }
    if (id === '/cost') {
      ctx.showSessionStats();
      return true;
    }
    ctx.insertText(id + ' ');
    return true;
  }

  function renderModelHeaderExtra(ctx) {
    const { t, prefs } = ctx;
    const fastOn = Boolean(prefs && prefs.fastSpeed);
    return h(
      'button',
      {
        type: 'button',
        'data-dsb-codex-fast-btn': '1',
        title: '切换 Codex app-server serviceTier=fast (1.5x 极速推理)',
        onClick: (ev) => {
          ev.stopPropagation();
          ctx.updatePrefs({ fastSpeed: !fastOn });
        },
        style: {
          border: '1px solid ' + (fastOn ? t.accentBorder : t.subtleBorder),
          background: fastOn ? t.accentBg : 'transparent',
          color: fastOn ? t.accent : t.muted,
          borderRadius: '999px',
          padding: '1px 8px',
          fontSize: '10.5px',
          fontWeight: fastOn ? 600 : 450,
          cursor: 'pointer',
        },
      },
      '⚡ 1.5x ' + (fastOn ? '开' : '关'),
    );
  }

  function buildPlusMenu(ctx) {
    const { t, icons, opts, prefs, closePopover, sectionHdr, popRow } = ctx;
    const enabledList = getEnabledPlugins(prefs);
    const planOn = Boolean(prefs && prefs.planMode);

    const livePlugins = Array.isArray(opts && opts.plugins) ? opts.plugins : [];
    const seen = new Set();
    const mergedPlugins = [];
    for (const p of livePlugins.concat(BUILTIN_PLUGINS)) {
      if (!p || !p.label) continue;
      const key = String(p.label).toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      mergedPlugins.push(p);
    }

    return [
      sectionHdr('cx-add', 'Codex · 添加上下文', false),
      popRow('cx-files', icons.clip(t.muted), '文件和文件夹', '图片走 localImage，文件随路径传入 turn/start', false, () => {
        closePopover();
        ctx.pickFiles(false);
      }),
      popRow('cx-folder', icons.folder(t.muted), '选择整个文件夹', '附加目录路径作为工作上下文', false, () => {
        closePopover();
        ctx.pickFolder();
      }),
      popRow('cx-at', icons.at(t.muted), '引用本地文件 (@)', '搜索并附加本地文件到当前对话', false, () => {
        closePopover();
        ctx.openMentionPicker();
      }),
      popRow(
        'cx-plan',
        icons.slash(planOn ? t.accent : t.muted),
        '计划模式 (Plan)',
        planOn ? '已开启：sandboxPolicy=readOnly 仅输出方案' : '切换只读计划模式（不直接改文件）',
        planOn,
        () => {
          ctx.updatePrefs({ planMode: !planOn });
        },
      ),
      popRow('cx-goal', icons.slash(t.accent), '目标 (/goal)', '插入 /goal 持续推进目标', false, () => {
        closePopover();
        ctx.insertText('/goal ');
      }),
      popRow('cx-cost', icons.slash(t.muted), '查看用量 (/cost)', '读取当前 Codex 线程真实 Token 消耗', false, () => {
        closePopover();
        ctx.showSessionStats();
      }),
      sectionHdr('cx-plg', '插件（以 mention 协议挂载到 turn/start）', true),
      ...mergedPlugins.slice(0, 8).map((plg) => {
        const pid = String(plg.id || plg.label);
        const active = enabledList.includes(pid);
        return popRow(
          'plg-' + pid,
          icons.pluginBadge(pid || plg.label),
          plg.label,
          active ? '已挂载到本轮对话（点击取消）' : plg.desc || '点击挂载该插件',
          active,
          () => {
            togglePlugin(ctx, pid);
          },
        );
      }),
    ];
  }

  function getActiveChips(ctx) {
    const chips = [];
    const prefs = ctx.prefs || {};
    if (prefs.fastSpeed) {
      chips.push({
        id: 'cx-fast',
        label: '⚡ 1.5x 极速',
        title: 'serviceTier=fast（点击恢复标准速度）',
        onRemove: () => ctx.updatePrefs({ fastSpeed: false }),
      });
    }
    if (prefs.planMode) {
      chips.push({
        id: 'cx-plan',
        label: '📋 计划模式',
        title: 'sandboxPolicy=readOnly（点击退出计划模式）',
        onRemove: () => ctx.updatePrefs({ planMode: false }),
      });
    }
    for (const pid of getEnabledPlugins(prefs)) {
      const short = String(pid).split('@')[0] || pid;
      chips.push({
        id: 'cx-plg-' + pid,
        label: '🧩 ' + short,
        title: 'Codex 插件 mention: plugin://' + pid + '（点击移除）',
        onRemove: () => togglePlugin(ctx, pid),
      });
    }
    return chips;
  }

  function buildTurnPayload(ctx) {
    const prefs = ctx.prefs || {};
    return {
      sandboxMode: ctx.resolvedMode || DEFAULTS.defaultMode,
      model: ctx.resolvedModel || DEFAULTS.defaultModel,
      effort: ctx.resolvedEffort || DEFAULTS.defaultEffort,
      fastSpeed: Boolean(prefs.fastSpeed),
      planMode: Boolean(prefs.planMode),
      enabledPlugins: getEnabledPlugins(prefs),
    };
  }

  function renderRightControls(ctx) {
    const modelText = (ctx.avail && ctx.avail.modelLabel) || 'GPT-6 Luna';
    const effortText = (ctx.avail && ctx.avail.effortLabel) || '高';
    const accessText = (ctx.avail && ctx.avail.accessLabel) || '完全访问';
    return h(
      'div',
      {
        'data-dsb-harness-right': 'codex',
        style: {
          display: 'inline-flex',
          alignItems: 'center',
          gap: '10px',
          fontSize: '12px',
          color: 'var(--atp-muted, #afafaf)',
          paddingRight: '4px',
          userSelect: 'none',
          flexShrink: 0,
        },
      },
      h('span', { style: { color: '#A39BFF', fontWeight: 500 } }, accessText),
      h(
        'span',
        { style: { display: 'inline-flex', alignItems: 'center', gap: '4px' } },
        h('span', { style: { color: 'var(--atp-fg, #ededed)', fontWeight: 500 } }, modelText),
        h('span', { style: { opacity: 0.6 } }, effortText),
      ),
    );
  }

  return {
    css: '',
    getDefaults,
    renderModelHeaderExtra,
    buildPlusMenu,
    getActiveChips,
    handleSlashAction,
    buildTurnPayload,
    renderRightControls,
  };
})
