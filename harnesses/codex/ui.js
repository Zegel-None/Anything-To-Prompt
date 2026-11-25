// harnesses/codex/ui.js — 1:1 Codex Desktop Composer & Theme Adapter
(function (sdk) {
  const { h, icons } = sdk;

  const css = [
    'html[data-dsh-bar-clone="1"][data-dsb-harness="codex"] [data-dsb-card] {',
    '  background: #1f1f1f !important;',
    '  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.06) inset !important;',
    '  border-radius: 14px 14px 0 0 !important;',
    '  width: calc(100% - 28px) !important;',
    '  margin: 0 14px !important;',
    '  color: #f4f4f5 !important;',
    '}',
    'html[data-dsh-bar-clone="1"][data-dsb-harness="codex"][data-dsb-mode="lip"] [data-dsb-card] {',
    '  border-radius: 14px !important;',
    '}',
    'html[data-dsh-bar-clone="1"][data-dsb-harness="codex"] [data-dsb-divider] {',
    '  display: none !important;',
    '}',
  ].join('\n');

  function renderComposer(ctx) {
    const codexModel = (ctx.avail && ctx.avail.modelLabel) || 'GPT-6.1 Terra';
    const codexEffort = (ctx.avail && ctx.avail.effortLabel) || '高';
    const codexAccess = (ctx.avail && ctx.avail.accessLabel) || '完全访问';
    const codexWsLabel =
      (ctx.selection && ctx.selection.workspacePath
        ? ctx.workspaceBasename(ctx.selection.workspacePath)
        : '') ||
      (ctx.selection && ctx.selection.title) ||
      'Codex';

    return h(
      'div',
      {
        'data-dsb-external-composer': 'codex',
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
      // Inset top shelf matching Codex Desktop (`💻 此计算机  📁 <workspace>`)
      h(
        'div',
        {
          style: {
            margin: '0 14px',
            height: '34px',
            padding: '0 14px',
            background: '#1f1f1f',
            borderLeft: '1px solid rgba(255, 255, 255, 0.06)',
            borderRight: '1px solid rgba(255, 255, 255, 0.06)',
            borderTop: '1px solid rgba(255, 255, 255, 0.05)',
            display: 'flex',
            alignItems: 'center',
            gap: '16px',
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
              fontSize: '12.5px',
              color: '#e4e4e7',
              fontWeight: 500,
              flexShrink: 0,
              userSelect: 'none',
            },
          },
          icons.laptop('#d4d4d8'),
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
              padding: '2px 4px',
              borderRadius: '6px',
              cursor: 'pointer',
              display: 'inline-flex',
              alignItems: 'center',
              gap: '6px',
              fontSize: '12.5px',
              color: '#f4f4f5',
              fontWeight: 500,
              maxWidth: '340px',
            },
          },
          icons.folderOutline('#d4d4d8'),
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
      // Main 20px-radius Codex graphite composer card
      h(
        'div',
        {
          style: {
            width: '100%',
            background: '#303032',
            borderRadius: '20px',
            border: '1px solid rgba(255, 255, 255, 0.08)',
            padding: '13px 16px 11px 16px',
            display: 'flex',
            flexDirection: 'column',
            boxSizing: 'border-box',
            boxShadow: '0 8px 26px rgba(0, 0, 0, 0.36)',
          },
        },
        ctx.renderCapsulesRow(),
        h('textarea', {
          'data-dsb-ext-input': '1',
          rows: 1,
          value: ctx.draft,
          onChange: ctx.onInputChange,
          onKeyDown: ctx.onInputKeyDown,
          placeholder: ctx.manifest.placeholder || '随心输入',
          style: {
            width: '100%',
            minHeight: '26px',
            maxHeight: '140px',
            border: 'none',
            outline: 'none',
            resize: 'none',
            background: 'transparent',
            color: '#f4f4f5',
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
              marginTop: '14px',
              height: '30px',
              gap: '8px',
            },
          },
          h(
            'div',
            { style: { display: 'flex', alignItems: 'center', gap: '12px' } },
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
                },
              },
              icons.plus(17, '#e4e4e7'),
            ),
            h(
              'span',
              {
                title: '来自 ~/.codex/config.toml',
                style: {
                  fontSize: '13px',
                  fontWeight: 500,
                  color: '#ef6c35',
                  userSelect: 'none',
                },
              },
              codexAccess,
            ),
          ),
          h(
            'div',
            { style: { display: 'flex', alignItems: 'center', gap: '12px' } },
            h(
              'span',
              {
                style: {
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: '6px',
                  fontSize: '13px',
                  userSelect: 'none',
                },
              },
              h('span', { style: { color: '#f4f4f5', fontWeight: 500 } }, codexModel),
              codexEffort ? h('span', { style: { color: '#9ca3af' } }, codexEffort) : null,
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
                  background: '#2b6be6',
                  color: '#ffffff',
                  cursor: 'pointer',
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  padding: 0,
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
        title: 'Codex 沙箱权限（来自 ~/.codex/config.toml）',
        style: {
          fontSize: '12px',
          fontWeight: 500,
          color: '#fb6a33',
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
      h('span', { style: { color: 'rgba(244, 244, 245, 0.90)', fontWeight: 500 } }, modelText),
      effortText
        ? h('span', { style: { color: 'rgba(212, 212, 216, 0.58)' } }, effortText)
        : null,
    );
  }

  return {
    css,
    renderComposer,
    renderChipPrefix,
    renderRightControls,
  };
})
