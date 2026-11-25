// harnesses/_template/ui.js — 复制本文件夹到 harnesses/<your_id>/ 即可自动注册！
// 如果不需要定制 1:1 桌面端输入框，甚至可以省略 renderComposer，宿主会自动用 harness.json 的配色渲染默认自适应输入框。
(function (sdk) {
  const { h } = sdk;
  return {
    // 可选：作用域 CSS，仅在当前 Harness 激活时生效
    css: '',
    // 可选：自定义 1:1 输入框（留空则自动使用内置 Adaptive Default Composer）
    // renderComposer(ctx) { return h('div', { 'data-dsb-external-composer': ctx.manifest.id }, ...); },
    // 可选：右侧状态指示控件
    // renderRightControls(ctx) { return h('span', null, ctx.avail && ctx.avail.modelLabel); },
  };
})
