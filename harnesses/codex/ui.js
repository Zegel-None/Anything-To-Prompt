// harnesses/codex/ui.js — 1:1 Pixel-Exact Codex Desktop 26.1120 Composer mapped directly to Codex app-server API
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

  const cxSvg = (sz, stroke, children, fill = 'none', sw = 1.75) =>
    h(
      'svg',
      {
        width: sz,
        height: sz,
        viewBox: '0 0 24 24',
        fill,
        stroke,
        strokeWidth: sw,
        strokeLinecap: 'round',
        strokeLinejoin: 'round',
        style: { display: 'block', flexShrink: 0 },
      },
      children,
    );

  const CX_ICONS = {
    mic: (c = '#d4d4d4') =>
      cxSvg(16, c, [
        h('rect', { key: 'r', x: 9, y: 3, width: 6, height: 11, rx: 3 }),
        h('path', { key: 'u', d: 'M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21M9 21h6' }),
      ]),
    waveform: (c = '#ffffff') =>
      cxSvg(16, c, [
        h('path', { key: 'w', d: 'M5 10.5v3M8.5 7.5v9M12 5.5v13M15.5 8v8M19 10.5v3' }),
      ], 'none', 2.0),
  };

  function renderComposer(ctx) {
    const opts = ctx.options || {};
    const prefs = ctx.prefs || {};
    const models =
      Array.isArray(opts.models) && opts.models.length > 0
        ? opts.models
        : [
            { id: 'default', label: '默认' },
            { id: 'gpt-6.2-sol', label: 'GPT-6.2 Sol' },
            { id: 'gpt-6.2-astra', label: 'GPT-6.2 Astra' },
            { id: 'gpt-6.1-terra', label: 'GPT-6.1 Terra' },
            { id: 'gpt-6.1-sol', label: 'GPT-6.1 Sol' },
            { id: 'gpt-6-astra', label: 'GPT-6 Astra' },
            { id: 'gpt-6-sol', label: 'GPT-6 Sol' },
            { id: 'gpt-6-luna', label: 'GPT-6 Luna' },
            { id: 'gpt-5.6-sol', label: 'GPT-5.6 Sol' },
            { id: 'gpt-5.6-terra', label: 'GPT-5.6 Terra' },
          ];
    const efforts =
      Array.isArray(opts.efforts) && opts.efforts.length > 0
        ? opts.efforts
        : [
            { id: 'minimal', label: '极低' },
            { id: 'low', label: '低' },
            { id: 'high', label: '高' },
            { id: 'xhigh', label: '极高' },
            { id: 'max', label: '最高' },
          ];
    const modes =
      Array.isArray(opts.modes) && opts.modes.length > 0
        ? opts.modes
        : [
            { id: 'request-approval', label: '请求批准' },
            { id: 'help-approve', label: '帮我批准' },
            { id: 'danger-full-access', label: '完全访问权限' },
          ];

    const curModelId = prefs.model || opts.defaultModel || 'gpt-6-luna';
    const curEffortId = prefs.effort || opts.defaultEffort || 'high';
    const curModeId = prefs.mode || opts.defaultMode || 'danger-full-access';
    const curModel = models.find((m) => m.id === curModelId) || models.find((m) => m.id === 'gpt-6-luna') || models[0];
    const curEffort = efforts.find((e) => e.id === curEffortId) || efforts[2] || efforts[0];
    const curMode = modes.find((m) => m.id === curModeId) || modes[modes.length - 1] || modes[0];

    const modeShortLabel = curMode.id === 'danger-full-access' ? '完全访问' : curMode.label;
    const modeColor = curMode.id === 'danger-full-access' ? '#d96b27' : '#d4d4d4';

    return h(
      'div',
      {
        'data-dsb-external-composer': 'codex',
        onDragOver: ctx.onDragOver,
        onDrop: ctx.onDrop,
        onContextMenu: (ev) => {
          if (ctx.onOpenSwitcher) {
            ev.preventDefault();
            ctx.onOpenSwitcher(ev);
          }
        },
        style: {
          width: '100vw',
          position: 'relative',
          background: '#343434',
          borderRadius: '22px',
          border: '1px solid rgba(255, 255, 255, 0.08)',
          padding: '15px 16px 11px 16px',
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
        // Left: Clean `+` button + `完全访问` permissions selector (1:1 Codex Desktop)
        h(
          'div',
          { style: { display: 'flex', alignItems: 'center', gap: '6px' } },
          h(
            'button',
            {
              type: 'button',
              'data-dsb-plus-btn': '1',
              title: '添加文件和文件夹 / 目标 / 计划模式 / 插件',
              onClick: (ev) => {
                ev.stopPropagation();
                ctx.openComposerPopover && ctx.openComposerPopover('plus', ev.currentTarget);
              },
              style: {
                width: '28px',
                height: '28px',
                borderRadius: '999px',
                border: 'none',
                background: ctx.activePopover === 'plus' ? '#454545' : 'transparent',
                padding: 0,
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
              },
            },
            icons.plus(17, '#e5e5e5'),
          ),
          h(
            'button',
            {
              type: 'button',
              'data-dsb-mode-btn': '1',
              title: '更改权限（映射到 Codex app-server approvalPolicy / sandboxPolicy）',
              onClick: (ev) => {
                ev.stopPropagation();
                ctx.openComposerPopover && ctx.openComposerPopover('mode', ev.currentTarget);
              },
              style: {
                height: '28px',
                border: 'none',
                background: ctx.activePopover === 'mode' ? '#444444' : 'transparent',
                padding: '0 10px',
                borderRadius: '999px',
                fontSize: '13.5px',
                fontWeight: 500,
                color: modeColor,
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                userSelect: 'none',
              },
            },
            modeShortLabel,
          ),
        ),
        // Right: `GPT-6 Luna 高` / `选择推理强度` + Mic + Blue `#2c67c5` Voice/Send button
        h(
          'div',
          { style: { display: 'flex', alignItems: 'center', gap: '10px' } },
          h(
            'button',
            {
              type: 'button',
              'data-dsb-model-btn': '1',
              title: '选择推理强度与模型（映射到 Codex app-server effort / model）',
              onClick: (ev) => {
                ev.stopPropagation();
                ctx.openComposerPopover && ctx.openComposerPopover('model', ev.currentTarget);
              },
              style: {
                height: '28px',
                border: 'none',
                background: ctx.activePopover === 'model' ? '#454545' : 'transparent',
                padding: '0 10px',
                borderRadius: '999px',
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                gap: '6px',
                fontSize: '13.5px',
                userSelect: 'none',
              },
            },
            ctx.activePopover === 'model'
              ? h('span', { style: { color: '#cfcfcf', fontWeight: 450 } }, '选择推理强度')
              : [
                  h('span', { key: 'ml', style: { color: '#ededed', fontWeight: 500 } }, curModel.label),
                  h('span', { key: 'ef', style: { color: '#8f8f8f' } }, curEffort.label),
                ],
          ),
          h(
            'button',
            {
              type: 'button',
              title: '切换工作区 / 对话',
              onClick: (ev) => {
                ev.stopPropagation();
                if (ctx.onOpenSwitcher) ctx.onOpenSwitcher(ev);
              },
              style: {
                width: '28px',
                height: '28px',
                borderRadius: '999px',
                border: 'none',
                background: 'transparent',
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
                padding: 0,
              },
            },
            CX_ICONS.mic('#d4d4d4'),
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
                background: ctx.isStreaming ? '#414141' : '#2c67c5',
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
                  style: {
                    width: '10px',
                    height: '10px',
                    borderRadius: '2px',
                    background: '#ffffff',
                  },
                })
              : ctx.hasPayload
                ? icons.arrowUp('#ffffff')
                : CX_ICONS.waveform('#ffffff'),
          ),
        ),
      ),
    );
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
          color: '#afafaf',
          paddingRight: '4px',
          userSelect: 'none',
          flexShrink: 0,
        },
      },
      h('span', { style: { color: '#d96b27', fontWeight: 500 } }, accessText),
      h(
        'span',
        { style: { display: 'inline-flex', alignItems: 'center', gap: '4px' } },
        h('span', { style: { color: '#ededed', fontWeight: 500 } }, modelText),
        h('span', { style: { color: '#8f8f8f' } }, effortText),
      ),
    );
  }

  return {
    css,
    renderComposer,
    renderRightControls,
  };
})
