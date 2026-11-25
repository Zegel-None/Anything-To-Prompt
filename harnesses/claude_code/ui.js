// harnesses/claude_code/ui.js — 1:1 Pixel-Exact Claude Code Composer & Real Controls
(function (sdk) {
  const { h, icons } = sdk;

  const css = [
    'html[data-dsh-bar-clone="1"][data-dsb-harness="claude_code"] [data-dsb-card] {',
    '  background: rgba(36, 36, 34, 0.96) !important;',
    '  box-shadow: 0 0 0 1px rgba(255, 248, 240, 0.09) inset !important;',
    '  border-radius: 16px !important;',
    '  color: #EDEBE6 !important;',
    '}',
    'html[data-dsh-bar-clone="1"][data-dsb-harness="claude_code"] [data-dsb-divider] {',
    '  display: none !important;',
    '}',
    '[data-dsb-external-composer="claude_code"] [data-dsb-ext-input="1"]::placeholder {',
    '  color: rgba(237, 235, 230, 0.42);',
    '}',
  ].join('\n');

  function renderComposer(ctx) {
    const opts = ctx.options || {};
    const prefs = ctx.prefs || {};
    const models = Array.isArray(opts.models) && opts.models.length > 0 ? opts.models : [
      { id: 'claude-opus-4-6', label: 'Claude Opus', desc: '最强深度架构与复杂重构' },
      { id: 'claude-sonnet-4-6', label: 'Claude Sonnet', desc: '日常高速编码主力' },
      { id: 'claude-haiku-4-5', label: 'Claude Haiku', desc: '极速轻量问答与脚本' },
    ];
    const efforts = Array.isArray(opts.efforts) && opts.efforts.length > 0 ? opts.efforts : [
      { id: 'low', label: '低', desc: '快速响应' },
      { id: 'medium', label: '中', desc: '常规推理' },
      { id: 'high', label: '高', desc: '深度推理' },
      { id: 'max', label: '极高', desc: '最大思考深度' },
    ];
    const modes = Array.isArray(opts.modes) && opts.modes.length > 0 ? opts.modes : [
      { id: 'auto', label: '自动', desc: '智能自动执行并拦截高危操作' },
      { id: 'acceptEdits', label: '自动编辑', desc: '自动允许文件编辑' },
      { id: 'plan', label: '仅规划', desc: '只读分析与规划，不修改文件' },
      { id: 'dontAsk', label: '免询问', desc: '免交互确认提示' },
    ];

    const curModelId = prefs.model || opts.defaultModel || 'claude-opus-4-6';
    const curEffortId = prefs.effort || opts.defaultEffort || 'high';
    const curPermId = prefs.mode || (ctx.getPermissionMode ? ctx.getPermissionMode() : 'auto');
    const curModel = models.find((m) => m.id === curModelId) || models[0];
    const curEffort = efforts.find((e) => e.id === curEffortId) || efforts[2] || efforts[0];
    const currentPerm = modes.find((m) => m.id === curPermId) || modes[0];

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
        onDragOver: ctx.onDragOver,
        onDrop: ctx.onDrop,
        style: {
          width: '100vw',
          position: 'relative',
          background: '#30302E',
          borderRadius: '16px',
          border: '1px solid rgba(255, 248, 240, 0.11)',
          padding: '15px 14px 10px 15px',
          display: 'flex',
          flexDirection: 'column',
          boxSizing: 'border-box',
          pointerEvents: 'auto',
          fontFamily: '"Segoe UI", "PingFang SC", "Microsoft YaHei", system-ui, sans-serif',
        },
      },
      ctx.renderDragHandle && ctx.renderDragHandle(),
      ctx.renderAttachmentsRow && ctx.renderAttachmentsRow(),
      ctx.renderCapsulesRow(),
      h('textarea', {
        'data-dsb-ext-input': '1',
        rows: 1,
        value: ctx.draft,
        onChange: ctx.onInputChange,
        onKeyDown: ctx.onInputKeyDown,
        onPaste: ctx.onPaste,
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
            marginTop: '11px',
            height: '32px',
            gap: '8px',
          },
        },
        // Left: Real `+` Add Context button, Real Permission Dropdown, Real Workspace chip
        h(
          'div',
          { style: { display: 'flex', alignItems: 'center', gap: '8px', minWidth: 0 } },
          h(
            'button',
            {
              type: 'button',
              'data-dsb-plus-btn': '1',
              title: '添加文件 / 图片 / 工作目录 (--add-dir) / 斜杠指令',
              onClick: (ev) => {
                ev.stopPropagation();
                ctx.openComposerPopover && ctx.openComposerPopover('plus', ev.currentTarget);
              },
              style: {
                width: '28px',
                height: '28px',
                borderRadius: '8px',
                border: '1px solid rgba(255, 248, 240, 0.11)',
                background:
                  ctx.activePopover === 'plus'
                    ? 'rgba(255, 248, 240, 0.14)'
                    : 'rgba(255, 248, 240, 0.05)',
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
              'data-dsb-mode-btn': '1',
              'data-dsb-claude-perm': currentPerm.id,
              title: currentPerm.desc + '（点击切换）',
              onClick: (ev) => {
                ev.stopPropagation();
                ctx.openComposerPopover && ctx.openComposerPopover('mode', ev.currentTarget);
              },
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
            icons.chevronDown(currentPerm.id === 'plan' ? '#93c5fd' : '#E89B7E'),
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
                maxWidth: '230px',
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
        // Right: Real Model+Effort Dropdown + Send/Stop Button
        h(
          'div',
          { style: { display: 'flex', alignItems: 'center', gap: '8px', flexShrink: 0 } },
          h(
            'button',
            {
              type: 'button',
              'data-dsb-model-btn': '1',
              title: '切换 Claude 模型与推理深度 (--model / --effort)',
              onClick: (ev) => {
                ev.stopPropagation();
                ctx.openComposerPopover && ctx.openComposerPopover('model', ev.currentTarget);
              },
              style: {
                height: '28px',
                border: 'none',
                background:
                  ctx.activePopover === 'model'
                    ? 'rgba(255, 248, 240, 0.10)'
                    : 'transparent',
                padding: '0 8px',
                borderRadius: '8px',
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                gap: '5px',
                fontSize: '12.5px',
                whiteSpace: 'nowrap',
                userSelect: 'none',
              },
            },
            h('span', { style: { color: '#ECE9E2', fontWeight: 500 } }, curModel.label),
            h('span', { style: { color: '#9E9A90' } }, curEffort.label),
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
