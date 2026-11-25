// harnesses/antigravity/ui.js — 1:1 Antigravity Desktop Composer & Theme Adapter
(function (sdk) {
  const { h, icons } = sdk;

  const css = [
    'html[data-dsh-bar-clone="1"][data-dsb-harness="antigravity"] [data-dsb-card] {',
    '  background: #121213 !important;',
    '  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.06) inset !important;',
    '  border-radius: 16px 16px 0 0 !important;',
    '  color: #eceef2 !important;',
    '}',
    'html[data-dsh-bar-clone="1"][data-dsb-harness="antigravity"][data-dsb-mode="lip"] [data-dsb-card] {',
    '  border-radius: 16px !important;',
    '}',
    'html[data-dsh-bar-clone="1"][data-dsb-harness="antigravity"] [data-dsb-divider] {',
    '  display: none !important;',
    '}',
  ].join('\n');

  function renderComposer(ctx) {
    const modelName = (ctx.avail && ctx.avail.modelLabel) || 'Gemini 4 Neon';
    const effortName = (ctx.avail && ctx.avail.effortLabel) || 'High';
    const accessName = (ctx.avail && ctx.avail.accessLabel) || '极速';
    const sessionLabel = (ctx.selection && ctx.selection.title) || 'CLI Project';

    return h(
      'div',
      {
        'data-dsb-external-composer': 'antigravity',
        style: {
          width: '100vw',
          padding: '4px 5px 5px',
          background: '#111113',
          borderRadius: '0 0 18px 18px',
          borderLeft: '1px solid rgba(255, 255, 255, 0.06)',
          borderRight: '1px solid rgba(255, 255, 255, 0.06)',
          borderBottom: '1px solid rgba(255, 255, 255, 0.06)',
          boxSizing: 'border-box',
          pointerEvents: 'auto',
          fontFamily: '"Segoe UI", "PingFang SC", "Microsoft YaHei", system-ui, sans-serif',
        },
      },
      h(
        'div',
        {
          style: {
            position: 'relative',
            background: '#17171a',
            borderRadius: '14px',
            border: '1px solid rgba(255, 255, 255, 0.08)',
            padding: '15px 13px 9px 14px',
            display: 'flex',
            flexDirection: 'column',
            boxSizing: 'border-box',
          },
        },
        ctx.renderDragHandle && ctx.renderDragHandle(),
        ctx.renderCapsulesRow(),
        h('textarea', {
          'data-dsb-ext-input': '1',
          rows: 1,
          value: ctx.draft,
          onChange: ctx.onInputChange,
          onKeyDown: ctx.onInputKeyDown,
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
          h(
            'div',
            { style: { display: 'flex', alignItems: 'center', gap: '12px', minWidth: 0 } },
            h(
              'button',
              {
                type: 'button',
                title: '切换 Harness / 工作区 / 对话',
                onClick: ctx.onOpenSwitcher,
                style: {
                  border: 'none',
                  background: 'transparent',
                  padding: '2px',
                  cursor: 'pointer',
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  color: '#a1a1aa',
                },
              },
              icons.plus(15, '#a1a1aa'),
            ),
            h(
              'span',
              {
                style: {
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: '5px',
                  fontSize: '12.5px',
                  whiteSpace: 'nowrap',
                  userSelect: 'none',
                },
              },
              h('span', { style: { color: '#d4d4d8', fontWeight: 500 } }, modelName),
              h('span', { style: { color: '#71717a' } }, effortName),
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
                  border: 'none',
                  background: 'transparent',
                  padding: '2px 4px',
                  borderRadius: '6px',
                  cursor: 'pointer',
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: '5px',
                  fontSize: '12px',
                  color: '#38bdf8',
                  maxWidth: '210px',
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
          h(
            'div',
            { style: { display: 'flex', alignItems: 'center', gap: '12px', flexShrink: 0 } },
            h(
              'span',
              {
                style: {
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: '5px',
                  fontSize: '12.5px',
                  color: '#d4d4d8',
                  userSelect: 'none',
                },
              },
              icons.shield('#9ca3af'),
              accessName,
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
                      border: '1px solid rgba(255,255,255,0.08)',
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
                      background: ctx.hasPayload ? '#f4f4f5' : '#d4d4d8',
                      cursor: 'pointer',
                      display: 'inline-flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      padding: 0,
                      opacity: ctx.hasPayload ? 1 : 0.88,
                    },
              },
              ctx.isStreaming
                ? h('span', {
                    style: {
                      width: '10px',
                      height: '10px',
                      borderRadius: '2px',
                      background: '#c94a4a',
                    },
                  })
                : icons.arrowUp('#111214'),
            ),
          ),
        ),
      ),
    );
  }

  function renderRightControls(ctx) {
    const modelText = (ctx.avail && ctx.avail.modelLabel) || 'Gemini';
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
