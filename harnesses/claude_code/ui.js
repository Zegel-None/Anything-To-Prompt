// harnesses/claude_code/ui.js — 1:1 Claude Desktop (Code Mode) Composer & Theme Adapter
(function (sdk) {
  const { h, icons } = sdk;

  const PERM_MODES = [
    { id: 'auto', label: '自动', title: 'Claude auto 模式（默认：智能自动执行并拦截高危操作）' },
    { id: 'acceptEdits', label: '自动编辑', title: 'acceptEdits（自动允许文件编辑）' },
    { id: 'plan', label: '仅规划', title: 'plan（只读分析与规划，不修改文件）' },
    { id: 'dontAsk', label: '免询问', title: 'dontAsk（免交互确认提示）' },
  ];

  const css = [
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
    '[data-dsb-external-composer="claude_code"] [data-dsb-ext-input="1"]::placeholder {',
    '  color: rgba(237, 235, 230, 0.42);',
    '}',
  ].join('\n');

  function renderComposer(ctx) {
    const claudeModel = (ctx.avail && ctx.avail.modelLabel) || 'Claude Opus';
    const claudeEffort = (ctx.avail && ctx.avail.effortLabel) || '高';
    const currentPermId = ctx.getPermissionMode ? ctx.getPermissionMode() : 'auto';
    const currentPerm = PERM_MODES.find((m) => m.id === currentPermId) || PERM_MODES[0];
    const cycleClaudePerm = (ev) => {
      ev.stopPropagation();
      const idx = PERM_MODES.findIndex((m) => m.id === currentPerm.id);
      const next = PERM_MODES[(idx + 1) % PERM_MODES.length];
      if (ctx.setPermissionMode) ctx.setPermissionMode(next.id);
    };
    const claudeWsLabel =
      (ctx.selection && ctx.selection.workspacePath
        ? ctx.workspaceBasename(ctx.selection.workspacePath)
        : '') ||
      (ctx.selection && ctx.selection.title) ||
      'Claude Code';

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
        ctx.renderCapsulesRow(),
        h('textarea', {
          'data-dsb-ext-input': '1',
          rows: 1,
          value: ctx.draft,
          onChange: ctx.onInputChange,
          onKeyDown: ctx.onInputKeyDown,
          placeholder: ctx.manifest.placeholder || '今天想处理什么？支持按 Ctrl 圈选屏幕元素…',
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
                onClick: ctx.onOpenSwitcher,
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
              icons.plus(15, '#D8D4CA'),
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
              icons.shield(currentPerm.id === 'plan' ? '#93c5fd' : '#D97757'),
              h('span', null, currentPerm.label),
            ),
            h(
              'button',
              {
                type: 'button',
                'data-dsb-session-chip': '1',
                title: '点击切换 Harness / 工作区 / 对话',
                onClick: ctx.onOpenSwitcher,
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
              icons.claudeSpark(13, '#D97757'),
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
              icons.chevronDown('rgba(234, 230, 223, 0.55)'),
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
              claudeEffort ? h('span', { style: { color: '#9E9A90' } }, claudeEffort) : null,
              icons.chevronDown('#8A867C'),
            ),
            h(
              'button',
              {
                type: 'button',
                title: ctx.isStreaming ? '停止生成' : '发送到 Claude Code',
                onClick: ctx.onSubmit,
                style: ctx.isStreaming
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
                      background: ctx.hasPayload ? '#D97757' : 'rgba(217, 119, 87, 0.46)',
                      color: '#FFFDF9',
                      cursor: 'pointer',
                      display: 'inline-flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      padding: 0,
                      transition: 'background 0.14s ease',
                    },
              },
              ctx.isStreaming
                ? h('span', {
                    style: {
                      width: '10px',
                      height: '10px',
                      borderRadius: '2px',
                      background: '#D97757',
                    },
                  })
                : icons.arrowUp('#FFFDF9'),
            ),
          ),
        ),
      ),
    );
  }

  function renderRightControls(ctx) {
    const modelText = (ctx.avail && ctx.avail.modelLabel) || 'Claude Opus';
    const effortText = (ctx.avail && ctx.avail.effortLabel) || '高';
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
      effortText ? h('span', { style: { color: '#9E9A90' } }, effortText) : null,
    );
  }

  return {
    css,
    renderComposer,
    renderRightControls,
  };
})
