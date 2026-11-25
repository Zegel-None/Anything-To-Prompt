// harnesses/antigravity/ui.js — 1:1 Pixel-Exact Antigravity Desktop Composer & Real Controls
(function (sdk) {
  const { h, icons } = sdk;

  const css = [
    'html[data-dsh-bar-clone="1"][data-dsb-harness="antigravity"] [data-dsb-card] {',
    '  background: rgba(18, 18, 20, 0.96) !important;',
    '  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.08) inset !important;',
    '  border-radius: 16px !important;',
    '  color: #eceef2 !important;',
    '}',
    'html[data-dsh-bar-clone="1"][data-dsb-harness="antigravity"] [data-dsb-divider] {',
    '  display: none !important;',
    '}',
    '[data-dsb-external-composer="antigravity"] [data-dsb-ext-input="1"]::placeholder {',
    '  color: rgba(236, 238, 242, 0.42);',
    '}',
  ].join('\n');

  function renderComposer(ctx) {
    const opts = ctx.options || {};
    const prefs = ctx.prefs || {};
    const models = Array.isArray(opts.models) && opts.models.length > 0 ? opts.models : [
      { id: 'gemini-4-neon', label: 'Gemini 4 Neon', desc: '最新旗舰多模态模型' },
      { id: 'gemini-3.5-pro', label: 'Gemini 3.5 Pro', desc: '深度推理与工程代码' },
      { id: 'claude-opus-4-6', label: 'Claude Opus', desc: '长上下文深度思考' },
      { id: 'claude-sonnet-4-6', label: 'Claude Sonnet', desc: '高速均衡编程模型' },
    ];
    const efforts = Array.isArray(opts.efforts) && opts.efforts.length > 0 ? opts.efforts : [
      { id: 'low', label: 'Low', desc: '极速低延迟' },
      { id: 'medium', label: 'Medium', desc: '日常均衡推理' },
      { id: 'high', label: 'High', desc: '深度复杂推理' },
      { id: 'max', label: 'Max', desc: '最高算力思考' },
    ];
    const modes = Array.isArray(opts.modes) && opts.modes.length > 0 ? opts.modes : [
      { id: 'auto', label: '极速', desc: 'Fast · 自动执行常规操作' },
      { id: 'plan', label: '规划', desc: 'Planning · 先拆解方案再执行' },
      { id: 'accept-edits', label: '自动编辑', desc: 'Accept Edits · 自动写入代码修改' },
    ];

    const curModelId = prefs.model || opts.defaultModel || 'gemini-4-neon';
    const curEffortId = prefs.effort || opts.defaultEffort || 'high';
    const curModeId = prefs.mode || opts.defaultMode || 'auto';
    const curModel = models.find((m) => m.id === curModelId) || models[0];
    const curEffort = efforts.find((e) => e.id === curEffortId) || efforts[2] || efforts[0];
    const curMode = modes.find((m) => m.id === curModeId) || modes[0];
    const sessionLabel = (ctx.selection && ctx.selection.title) || 'CLI Project';

    return h(
      'div',
      {
        'data-dsb-external-composer': 'antigravity',
        onDragOver: ctx.onDragOver,
        onDrop: ctx.onDrop,
        style: {
          width: '100vw',
          position: 'relative',
          background: '#17171a',
          borderRadius: '16px',
          border: '1px solid rgba(255, 255, 255, 0.085)',
          padding: '15px 14px 10px 14px',
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
        placeholder: ctx.manifest.placeholder || '提问任何问题，使用 @ 提及，/ 执行操作',
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
        // Left: Real `+` Add Context button, Real Model+Effort button, Real Conversation chip
        h(
          'div',
          { style: { display: 'flex', alignItems: 'center', gap: '8px', minWidth: 0 } },
          h(
            'button',
            {
              type: 'button',
              'data-dsb-plus-btn': '1',
              title: 'Add Context（添加文件 / 文件夹 / @ 提及 / / 指令）',
              onClick: (ev) => {
                ev.stopPropagation();
                ctx.openComposerPopover && ctx.openComposerPopover('plus', ev.currentTarget);
              },
              style: {
                width: '28px',
                height: '28px',
                borderRadius: '999px',
                border: '1px solid rgba(255, 255, 255, 0.08)',
                background: ctx.activePopover === 'plus' ? 'rgba(255, 255, 255, 0.14)' : '#232428',
                padding: 0,
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
                color: '#d4d4d8',
                flexShrink: 0,
              },
            },
            icons.plus(15, '#d4d4d8'),
          ),
          h(
            'button',
            {
              type: 'button',
              'data-dsb-model-btn': '1',
              title: '切换模型与推理深度',
              onClick: (ev) => {
                ev.stopPropagation();
                ctx.openComposerPopover && ctx.openComposerPopover('model', ev.currentTarget);
              },
              style: {
                height: '28px',
                border: 'none',
                background: ctx.activePopover === 'model' ? 'rgba(255, 255, 255, 0.09)' : 'transparent',
                padding: '0 7px',
                borderRadius: '8px',
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                gap: '5px',
                fontSize: '12.5px',
                whiteSpace: 'nowrap',
                userSelect: 'none',
                flexShrink: 0,
              },
            },
            h('span', { style: { color: '#e4e4e7', fontWeight: 500 } }, curModel.label),
            h('span', { style: { color: '#8b8d98' } }, curEffort.label),
            icons.chevronDown('#71717a'),
          ),
          h(
            'button',
            {
              type: 'button',
              'data-dsb-session-chip': '1',
              title: '点击切换 Harness / 工作区 / 对话',
              onClick: ctx.onOpenSwitcher,
              style: {
                height: '26px',
                border: 'none',
                background: 'rgba(56, 189, 248, 0.08)',
                padding: '0 8px',
                borderRadius: '7px',
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                gap: '5px',
                fontSize: '12px',
                color: '#38bdf8',
                maxWidth: '200px',
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
        // Right: Real Mode button (极速/规划/自动编辑) + Send/Stop button
        h(
          'div',
          { style: { display: 'flex', alignItems: 'center', gap: '8px', flexShrink: 0 } },
          h(
            'button',
            {
              type: 'button',
              'data-dsb-mode-btn': '1',
              title: '切换执行模式（极速 / 规划 / 自动编辑）',
              onClick: (ev) => {
                ev.stopPropagation();
                ctx.openComposerPopover && ctx.openComposerPopover('mode', ev.currentTarget);
              },
              style: {
                height: '28px',
                border: 'none',
                background: ctx.activePopover === 'mode' ? 'rgba(255, 255, 255, 0.09)' : 'transparent',
                padding: '0 7px',
                borderRadius: '8px',
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                gap: '5px',
                fontSize: '12.5px',
                color: curMode.id === 'plan' ? '#93c5fd' : '#d4d4d8',
                userSelect: 'none',
              },
            },
            icons.shield(curMode.id === 'plan' ? '#93c5fd' : '#9ca3af'),
            h('span', null, curMode.label),
            icons.chevronDown('#71717a'),
          ),
          h(
            'button',
            {
              type: 'button',
              title: ctx.isStreaming ? '停止生成' : '发送到 Antigravity',
              onClick: ctx.onSubmit,
              style: ctx.isStreaming
                ? {
                    width: '28px',
                    height: '28px',
                    borderRadius: '999px',
                    border: '1px solid rgba(255,255,255,0.10)',
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
                    background: ctx.hasPayload ? '#f4f4f5' : '#3a3b40',
                    cursor: 'pointer',
                    display: 'inline-flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    padding: 0,
                    opacity: ctx.hasPayload ? 1 : 0.85,
                  },
            },
            ctx.isStreaming
              ? h('span', {
                  style: {
                    width: '10px',
                    height: '10px',
                    borderRadius: '2px',
                    background: '#ef4444',
                  },
                })
              : icons.arrowUp(ctx.hasPayload ? '#111214' : '#9ca3af'),
          ),
        ),
      ),
    );
  }

  function renderRightControls(ctx) {
    const modelText = (ctx.avail && ctx.avail.modelLabel) || 'Gemini 4 Neon';
    const effortText = (ctx.avail && ctx.avail.effortLabel) || 'High';
    const accessText = (ctx.avail && ctx.avail.accessLabel) || '极速';
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

  return {
    css,
    renderComposer,
    renderRightControls,
  };
})
