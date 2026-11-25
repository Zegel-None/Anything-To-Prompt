// harnesses/antigravity/ui.js — 1:1 Pixel-Exact Antigravity 2.35.1 Desktop Composer & Controls
(function (sdk) {
  const { h, icons } = sdk;

  const css = [
    'html[data-dsh-bar-clone="1"][data-dsb-harness="antigravity"] [data-dsb-card] {',
    '  background: rgba(19, 19, 23, 0.96) !important;',
    '  box-shadow: 0 0 0 1px #27272c inset !important;',
    '  border-radius: 15px !important;',
    '  color: #e4e4e7 !important;',
    '}',
    'html[data-dsh-bar-clone="1"][data-dsb-harness="antigravity"] [data-dsb-divider] {',
    '  display: none !important;',
    '}',
    '[data-dsb-external-composer="antigravity"] [data-dsb-ext-input="1"]::placeholder {',
    '  color: #7c7c85;',
    '}',
  ].join('\n');

  const agSvg = (sz, stroke, children, fill = 'none', sw = 1.65) =>
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

  const AG_ICONS = {
    plus: (c = '#a1a1aa') =>
      agSvg(15, c, [h('path', { key: 'p', d: 'M12 5v14M5 12h14' })], 'none', 1.8),
    shieldAlert: (c = '#9ca3af') =>
      agSvg(14, c, [
        h('path', { key: 's', d: 'M12 3l7 3v5c0 4.5-2.9 8.4-7 10-4.1-1.6-7-5.5-7-10V6l7-3z' }),
        h('path', { key: 'l', d: 'M12 8.5v4' }),
        h('circle', { key: 'd', cx: 12, cy: 15.5, r: 0.7, fill: c }),
      ]),
    shield: (c = '#9ca3af') =>
      agSvg(14, c, [
        h('path', { key: 's', d: 'M12 3l7 3v5c0 4.5-2.9 8.4-7 10-4.1-1.6-7-5.5-7-10V6l7-3z' }),
      ]),
    lock: (c = '#9ca3af') =>
      agSvg(14, c, [
        h('rect', { key: 'r', x: 5, y: 11, width: 14, height: 10, rx: 2 }),
        h('path', { key: 'a', d: 'M8 11V7a4 4 0 0 1 8 0v4' }),
      ]),
    eye: (c = '#9ca3af') =>
      agSvg(14, c, [
        h('path', { key: 'e', d: 'M2 12s3.8-6.5 10-6.5S22 12 22 12s-3.8 6.5-10 6.5S2 12 2 12z' }),
        h('circle', { key: 'c', cx: 12, cy: 12, r: 2.8 }),
      ]),
    mic: (c = '#9ca3af') =>
      agSvg(15, c, [
        h('rect', { key: 'r', x: 9, y: 3, width: 6, height: 11, rx: 3 }),
        h('path', { key: 'u', d: 'M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21M9 21h6' }),
      ]),
    voiceSpark: () =>
      h(
        'svg',
        {
          width: 16,
          height: 16,
          viewBox: '0 0 24 24',
          fill: 'none',
          style: { display: 'block', flexShrink: 0 },
        },
        h('path', {
          d: 'M5 10.5v3M8.5 7.5v9M12 5.5v13M15.5 8.5v7',
          stroke: '#111115',
          strokeWidth: 2.1,
          strokeLinecap: 'round',
        }),
        h('path', {
          d: 'M19.5 4.5l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7.7-1.8z',
          fill: '#111115',
        }),
      ),
  };

  function renderComposer(ctx) {
    const opts = ctx.options || {};
    const prefs = ctx.prefs || {};
    const models =
      Array.isArray(opts.models) && opts.models.length > 0
        ? opts.models
        : [
            { id: 'gemini-4-neon-high', label: 'Gemini 4 Neon', effort: 'High', badge: 'New', info: true },
            { id: 'gemini-4-carbon-medium', label: 'Gemini 4 Carbon', effort: 'Medium', badge: 'New', info: true },
            { id: 'gemini-4-argon-medium', label: 'Gemini 4 Argon', effort: 'Medium', badge: 'Leaving Soon', info: true },
            { id: 'gemini-4-hydrogen-medium', label: 'Gemini 4 Hydrogen', effort: 'Medium', badge: 'New', info: true },
            { id: 'gemini-pro-orchestrator-neon', label: 'Orchestrator', effort: 'Claude Opus 5.5 High', badge: 'New', info: true },
            { id: 'gemini-3.8-flash-medium', label: 'Gemini 3.8 Flash', effort: 'Medium', badge: 'Leaving Soon', info: true },
            { id: 'claude-opus-5-5-medium', label: 'Claude Opus 5.5', effort: 'Medium', badge: 'New', info: false },
            { id: 'claude-sonnet-5-5-medium', label: 'Claude Sonnet 5.5', effort: 'Medium', badge: 'New', info: false },
            { id: 'gpt-oss-120b-medium', label: 'GPT-OSS 120B (Medium)', effort: '', badge: 'Notice', info: true },
          ];
    const efforts =
      Array.isArray(opts.efforts) && opts.efforts.length > 0
        ? opts.efforts
        : [
            { id: 'low', label: 'Low', desc: 'Fast low-latency' },
            { id: 'medium', label: 'Medium', desc: 'Balanced reasoning' },
            { id: 'high', label: 'High', desc: 'Deep complex reasoning' },
            { id: 'max', label: 'Max', desc: 'Maximum thinking budget' },
          ];
    const modes =
      Array.isArray(opts.modes) && opts.modes.length > 0
        ? opts.modes
        : [
            { id: 'request-review', label: 'Request Review', desc: 'Every command requires approval.' },
            { id: 'default', label: 'Default', desc: 'Ask before sensitive operations.' },
            { id: 'auto', label: 'Auto', desc: 'Automatically decide if commands run.' },
            { id: 'turbo', label: 'Turbo', desc: 'Run all commands without restrictions.' },
          ];

    const curModelId = prefs.model || opts.defaultModel || 'gemini-4-neon-high';
    const curEffortId = prefs.effort || opts.defaultEffort || 'high';
    const curModeId = prefs.mode || opts.defaultMode || 'turbo';
    const curModel = models.find((m) => m.id === curModelId) || models[0];
    const curEffort = efforts.find((e) => e.id === curEffortId) || efforts[2] || efforts[0];
    const curMode = modes.find((m) => m.id === curModeId) || modes[modes.length - 1] || modes[0];

    const modeIcon =
      curMode.id === 'request-review'
        ? AG_ICONS.eye('#9ca3af')
        : curMode.id === 'default'
          ? AG_ICONS.lock('#9ca3af')
          : curMode.id === 'auto'
            ? AG_ICONS.shield('#9ca3af')
            : AG_ICONS.shieldAlert('#9ca3af');

    return h(
      'div',
      {
        'data-dsb-external-composer': 'antigravity',
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
          background: '#15151a',
          borderRadius: '15px',
          border: '1px solid #27272c',
          padding: '12px 14px 10px 14px',
          display: 'flex',
          flexDirection: 'column',
          boxSizing: 'border-box',
          pointerEvents: 'auto',
          fontFamily: '"Segoe UI", "Inter", "PingFang SC", "Microsoft YaHei", system-ui, sans-serif',
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
        placeholder: ctx.manifest.placeholder || 'Ask anything, @ to mention, / for actions',
        style: {
          width: '100%',
          minHeight: '24px',
          maxHeight: '140px',
          border: 'none',
          outline: 'none',
          resize: 'none',
          background: 'transparent',
          color: '#e4e4e7',
          fontSize: '13.5px',
          lineHeight: '21px',
          fontFamily: 'inherit',
          padding: '1px 2px 0 2px',
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
            height: '28px',
            gap: '8px',
          },
        },
        // Left: Clean `+` Add Context button + Model & Effort selector (1:1 Antigravity 2.35.1)
        h(
          'div',
          { style: { display: 'flex', alignItems: 'center', gap: '4px', minWidth: 0 } },
          h(
            'button',
            {
              type: 'button',
              'data-dsb-plus-btn': '1',
              title: 'Add Context',
              onClick: (ev) => {
                ev.stopPropagation();
                ctx.openComposerPopover && ctx.openComposerPopover('plus', ev.currentTarget);
              },
              style: {
                width: '26px',
                height: '26px',
                borderRadius: '7px',
                border: 'none',
                background: ctx.activePopover === 'plus' ? 'rgba(255, 255, 255, 0.10)' : 'transparent',
                padding: 0,
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
                color: '#a1a1aa',
                flexShrink: 0,
              },
            },
            AG_ICONS.plus('#a1a1aa'),
          ),
          h(
            'button',
            {
              type: 'button',
              'data-dsb-model-btn': '1',
              title: 'Select model',
              onClick: (ev) => {
                ev.stopPropagation();
                ctx.openComposerPopover && ctx.openComposerPopover('model', ev.currentTarget);
              },
              style: {
                height: '26px',
                border: 'none',
                background: ctx.activePopover === 'model' ? 'rgba(255, 255, 255, 0.08)' : 'transparent',
                padding: '0 6px',
                borderRadius: '7px',
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
            h('span', { style: { color: '#d4d4d8', fontWeight: 450 } }, curModel.label),
            curEffort && curEffort.label
              ? h('span', { style: { color: '#7c7c85', fontWeight: 400 } }, curEffort.label)
              : null,
            icons.chevronDown('#71717a'),
          ),
        ),
        // Right: Permissions preset selector (Turbo/Auto/Default/Request Review) + Mic + White Voice/Send button
        h(
          'div',
          { style: { display: 'flex', alignItems: 'center', gap: '8px', flexShrink: 0 } },
          h(
            'button',
            {
              type: 'button',
              'data-dsb-mode-btn': '1',
              title: 'Agent permissions preset selector',
              onClick: (ev) => {
                ev.stopPropagation();
                ctx.openComposerPopover && ctx.openComposerPopover('mode', ev.currentTarget);
              },
              style: {
                height: '26px',
                border: 'none',
                background: ctx.activePopover === 'mode' ? 'rgba(255, 255, 255, 0.08)' : 'transparent',
                padding: '0 6px',
                borderRadius: '7px',
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                gap: '5px',
                fontSize: '12.5px',
                color: '#a1a1aa',
                userSelect: 'none',
              },
            },
            modeIcon,
            h('span', { style: { color: '#a1a1aa', fontWeight: 450 } }, curMode.label),
            icons.chevronDown('#71717a'),
          ),
          h(
            'button',
            {
              type: 'button',
              title: '语音输入',
              onClick: (ev) => {
                ev.stopPropagation();
                if (ctx.onOpenSwitcher) ctx.onOpenSwitcher(ev);
              },
              style: {
                width: '26px',
                height: '26px',
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
            AG_ICONS.mic('#9ca3af'),
          ),
          h(
            'button',
            {
              type: 'button',
              title: ctx.hasPayload ? '发送到 Antigravity' : '语音对话 / 发送',
              onClick: ctx.onSubmit,
              style: {
                width: '28px',
                height: '28px',
                borderRadius: '999px',
                border: 'none',
                background: '#ffffff',
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
                padding: 0,
                flexShrink: 0,
              },
            },
            ctx.hasPayload ? icons.arrowUp('#111115') : AG_ICONS.voiceSpark(),
          ),
          ctx.isStreaming
            ? h(
                'button',
                {
                  type: 'button',
                  title: '停止生成',
                  onClick: ctx.onSubmit,
                  style: {
                    width: '28px',
                    height: '28px',
                    borderRadius: '999px',
                    border: '1px solid rgba(255,255,255,0.10)',
                    background: '#232328',
                    cursor: 'pointer',
                    display: 'inline-flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    padding: 0,
                    flexShrink: 0,
                  },
                },
                h('span', {
                  style: {
                    width: '9px',
                    height: '9px',
                    borderRadius: '2px',
                    background: '#ef4444',
                  },
                }),
              )
            : null,
        ),
      ),
    );
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
