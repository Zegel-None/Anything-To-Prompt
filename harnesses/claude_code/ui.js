// harnesses/claude_code/ui.js — v0.6.0 Unified DSH Skeleton (Anthropic Terracotta #D97757 Palette in client.js)
(function (sdk) {
  const { h } = sdk;

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
    renderRightControls,
  };
})
