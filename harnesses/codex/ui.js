// harnesses/codex/ui.js — 1:1 Pixel-Exact Codex Desktop Composer (exact OpenAI.Codex app.asar tokens)
(function (sdk) {
  const { h, icons } = sdk;

  const css = [
    'html[data-dsh-bar-clone="1"][data-dsb-harness="codex"] [data-dsb-card] {',
    '  background: rgba(24, 24, 24, 0.96) !important;',
    '  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.08) inset !important;',
    '  border-radius: 18px !important;',
    '  color: #ededed !important;',
    '}',
    'html[data-dsh-bar-clone="1"][data-dsb-harness="codex"] [data-dsb-divider] {',
    '  display: none !important;',
    '}',
    '[data-dsb-external-composer="codex"] [data-dsb-ext-input="1"]::placeholder {',
    '  color: #8f8f8f;',
    '}',
  ].join('\n');

  function renderComposer(ctx) {
    const opts = ctx.options || {};
    const prefs = ctx.prefs || {};
    const models = Array.isArray(opts.models) && opts.models.length > 0 ? opts.models : [
      { id: 'gpt-6-luna', label: 'GPT-6.1 Terra', desc: 'Codex 默认旗舰编程模型' },
      { id: 'gpt-5.4', label: 'GPT-5.4', desc: '稳健高速代码模型' },
      { id: 'o3', label: 'o3', desc: '高强度数学与系统推理' },
    ];
    const efforts = Array.isArray(opts.efforts) && opts.efforts.length > 0 ? opts.efforts : [
      { id: 'low', label: '低', desc: '快速轻量推理' },
      { id: 'medium', label: '中', desc: '平衡速度与质量' },
      { id: 'high', label: '高', desc: '深度工程推理' },
      { id: 'xhigh', label: '极高', desc: '拉满思考预算' },
    ];
    const modes = Array.isArray(opts.modes) && opts.modes.length > 0 ? opts.modes : [
      { id: 'danger-full-access', label: '完全访问', desc: '无沙箱限制，直接读写与执行' },
      { id: 'workspace-write', label: '工作区写入', desc: '仅允许在当前工作区目录写入' },
      { id: 'read-only', label: '只读模式', desc: '只读分析，不修改任何文件' },
    ];

    const curModelId = prefs.model || opts.defaultModel || 'gpt-6-luna';
    const curEffortId = prefs.effort || opts.defaultEffort || 'high';
    const curModeId = prefs.mode || opts.defaultMode || 'danger-full-access';
    const curModel = models.find((m) => m.id === curModelId) || models[0];
    const curEffort = efforts.find((e) => e.id === curEffortId) || efforts[2] || efforts[0];
    const curMode = modes.find((m) => m.id === curModeId) || modes[0];

    const codexWsLabel =
      (ctx.selection && ctx.selection.workspacePath
        ? ctx.workspaceBasename(ctx.selection.workspacePath)
        : '') ||
      (ctx.selection && ctx.selection.title) ||
      'Codex';

    const modeColor =
      curMode.id === 'danger-full-access'
        ? '#ef6c35'
        : curMode.id === 'read-only'
          ? '#93c5fd'
          : '#afafaf';

    return h(
      'div',
      {
        'data-dsb-external-composer': 'codex',
        onDragOver: ctx.onDragOver,
        onDrop: ctx.onDrop,
        style: {
          width: '100vw',
          display: 'flex',
          flexDirection: 'column',
          background: 'transparent',
          boxSizing: 'border-box',
          pointerEvents: 'auto',
          fontFamily: '"Segoe UI", "PingFang SC", "Microsoft YaHei", system-ui, sans-serif',
        },
      },
      // Inset top shelf matching Codex Desktop (`#181818` --gray-100, `💻 此计算机  📁 <workspace>`)
      h(
        'div',
        {
          style: {
            margin: '0 14px',
            height: '32px',
            padding: '0 14px',
            background: '#181818',
            borderRadius: '14px 14px 0 0',
            borderLeft: '1px solid rgba(255, 255, 255, 0.08)',
            borderRight: '1px solid rgba(255, 255, 255, 0.08)',
            borderTop: '1px solid rgba(255, 255, 255, 0.08)',
            display: 'flex',
            alignItems: 'center',
            gap: '14px',
            boxSizing: 'border-box',
          },
        },
        h(
          'span',
          {
            style: {
              display: 'inline-flex',
              alignItems: 'center',
              gap: '6px',
              fontSize: '12px',
              color: '#cfcfcf',
              fontWeight: 500,
              flexShrink: 0,
              userSelect: 'none',
            },
          },
          icons.laptop('#afafaf'),
          '此计算机',
        ),
        h(
          'button',
          {
            type: 'button',
            'data-dsb-session-chip': '1',
            title: '点击切换 Harness / 工作区 / 对话',
            onClick: ctx.onOpenSwitcher,
            style: {
              border: 'none',
              background: 'transparent',
              padding: '2px 5px',
              borderRadius: '6px',
              cursor: 'pointer',
              display: 'inline-flex',
              alignItems: 'center',
              gap: '6px',
              fontSize: '12px',
              color: '#ededed',
              fontWeight: 500,
              maxWidth: '340px',
            },
          },
          icons.folderOutline('#cfcfcf'),
          h(
            'span',
            {
              style: {
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
              },
            },
            codexWsLabel,
          ),
        ),
      ),
      // Main 22px-radius Codex graphite composer card (`#303030` --gray-300, `--composer-radius: 22px`)
      h(
        'div',
        {
          style: {
            position: 'relative',
            width: '100%',
            background: '#343434',
            borderRadius: '22px',
            border: '1px solid rgba(255, 255, 255, 0.08)',
            padding: '15px 16px 11px 16px',
            display: 'flex',
            flexDirection: 'column',
            boxSizing: 'border-box',
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
          placeholder: ctx.manifest.placeholder || '和 Codex 一起工作',
          style: {
            width: '100%',
            minHeight: '26px',
            maxHeight: '140px',
            border: 'none',
            outline: 'none',
            resize: 'none',
            background: 'transparent',
            color: '#ededed',
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
              height: '30px',
              gap: '8px',
            },
          },
          // Left: Real `+` Add/Plugins menu button + Real Sandbox/Access selector
          h(
            'div',
            { style: { display: 'flex', alignItems: 'center', gap: '8px' } },
            h(
              'button',
              {
                type: 'button',
                'data-dsb-plus-btn': '1',
                title: '添加文件和文件夹 / 目标 / 计划模式 / 绘图 / 插件',
                onClick: (ev) => {
                  ev.stopPropagation();
                  ctx.openComposerPopover && ctx.openComposerPopover('plus', ev.currentTarget);
                },
                style: {
                  width: '28px',
                  height: '28px',
                  borderRadius: '999px',
                  border: 'none',
                  background: ctx.activePopover === 'plus' ? '#484848' : '#3d3d3d',
                  padding: 0,
                  cursor: 'pointer',
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                },
              },
              icons.plus(16, '#ededed'),
            ),
            h(
              'button',
              {
                type: 'button',
                'data-dsb-mode-btn': '1',
                title: '切换 Codex 沙箱权限（完全访问 / 工作区写入 / 只读模式）',
                onClick: (ev) => {
                  ev.stopPropagation();
                  ctx.openComposerPopover && ctx.openComposerPopover('mode', ev.currentTarget);
                },
                style: {
                  height: '28px',
                  border: 'none',
                  background: ctx.activePopover === 'mode' ? 'rgba(255, 255, 255, 0.08)' : 'transparent',
                  padding: '0 8px',
                  borderRadius: '8px',
                  fontSize: '13px',
                  fontWeight: 500,
                  color: modeColor,
                  cursor: 'pointer',
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: '4px',
                  userSelect: 'none',
                },
              },
              h('span', null, curMode.label),
              icons.chevronDown(modeColor),
            ),
          ),
          // Right: Real Model+Effort selector + Real `#0169cc` Send/Stop button
          h(
            'div',
            { style: { display: 'flex', alignItems: 'center', gap: '10px' } },
            h(
              'button',
              {
                type: 'button',
                'data-dsb-model-btn': '1',
                title: '切换 Codex 模型与推理强度',
                onClick: (ev) => {
                  ev.stopPropagation();
                  ctx.openComposerPopover && ctx.openComposerPopover('model', ev.currentTarget);
                },
                style: {
                  height: '28px',
                  border: 'none',
                  background: ctx.activePopover === 'model' ? 'rgba(255, 255, 255, 0.08)' : 'transparent',
                  padding: '0 8px',
                  borderRadius: '8px',
                  cursor: 'pointer',
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: '6px',
                  fontSize: '13px',
                  userSelect: 'none',
                },
              },
              h('span', { style: { color: '#ededed', fontWeight: 500 } }, curModel.label),
              h('span', { style: { color: '#8f8f8f' } }, curEffort.label),
              icons.chevronDown('#8f8f8f'),
            ),
            h(
              'button',
              {
                type: 'button',
                title: ctx.isStreaming ? '停止生成' : '发送到 Codex',
                onClick: ctx.onSubmit,
                style: {
                  width: '30px',
                  height: '30px',
                  borderRadius: '999px',
                  border: 'none',
                  background: ctx.isStreaming
                    ? '#414141'
                    : ctx.hasPayload
                      ? '#0169cc'
                      : '#2c67c5',
                  color: '#ffffff',
                  cursor: 'pointer',
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  padding: 0,
                  opacity: ctx.isStreaming || ctx.hasPayload ? 1 : 0.72,
                },
              },
              ctx.isStreaming
                ? h('span', {
                    style: {
                      width: '10px',
                      height: '10px',
                      borderRadius: '2px',
                      background: '#ffffff',
                    },
                  })
                : icons.arrowUp('#ffffff'),
            ),
          ),
        ),
      ),
    );
  }

  function renderChipPrefix(ctx) {
    return h(
      'span',
      {
        title: 'Codex 沙箱权限',
        style: {
          fontSize: '12px',
          fontWeight: 500,
          color: '#ef6c35',
          padding: '0 6px 0 2px',
          display: 'inline-flex',
          alignItems: 'center',
          gap: '4px',
          flexShrink: 0,
          userSelect: 'none',
        },
      },
      (ctx.avail && ctx.avail.accessLabel) || '完全访问',
    );
  }

  function renderRightControls(ctx) {
    const modelText = (ctx.avail && ctx.avail.modelLabel) || 'GPT-6.1 Terra';
    const effortText = (ctx.avail && ctx.avail.effortLabel) || '高';
    return h(
      'div',
      {
        'data-dsb-harness-right': 'codex',
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
      h('span', { style: { color: '#ededed', fontWeight: 500 } }, modelText),
      effortText ? h('span', { style: { color: '#8f8f8f' } }, effortText) : null,
    );
  }

  return {
    css,
    renderComposer,
    renderChipPrefix,
    renderRightControls,
  };
})
