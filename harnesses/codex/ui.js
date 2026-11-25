// harnesses/codex/ui.js — v0.6.0 Unified DSH Skeleton (OpenAI Monochrome + Periwinkle Violet Palette in client.js)
(function (sdk) {
  const { h } = sdk;

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
    renderRightControls,
  };
})
