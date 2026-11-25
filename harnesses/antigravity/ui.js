// harnesses/antigravity/ui.js — v0.6.0 Unified DSH Skeleton (Google 4-Color Prism Palette in client.js)
(function (sdk) {
  const { h } = sdk;

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
          color: 'var(--atp-muted, rgba(229, 231, 235, 0.78))',
          paddingRight: '4px',
          userSelect: 'none',
          flexShrink: 0,
        },
      },
      h(
        'span',
        { style: { display: 'inline-flex', alignItems: 'center', gap: '4px' } },
        h('span', { style: { color: 'var(--atp-fg, rgba(243, 244, 246, 0.88))' } }, modelText),
        h('span', { style: { opacity: 0.6 } }, effortText),
      ),
      h(
        'span',
        { style: { display: 'inline-flex', alignItems: 'center', gap: '4px' } },
        h('span', {
          style: {
            width: '7px',
            height: '7px',
            borderRadius: '999px',
            background: 'conic-gradient(#4285F4 0 90deg, #34A853 90deg 180deg, #FBBC04 180deg 270deg, #EA4335 270deg 360deg)',
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
