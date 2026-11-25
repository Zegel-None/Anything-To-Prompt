# Harness 模块化适配指南 / Modular Harness Adapter Guide

Anything-to-Prompt 采用 **「一个公共桌面宿主 (`anything-to-prompt.exe`) + 多个即插即用 Harness 模块 (`harnesses/<id>/`)」** 架构：

- 无论用户电脑上只装了 1 家 AI 软件，还是同时装了 4 家甚至 20 家 AI 软件，全局只运行**唯一一个** `anything-to-prompt.exe` 公共宿主进程（负责全局 `Ctrl` 键鼠钩子、选区蓝框、屏幕接地截图、UIA 拾取与单实例悬浮输入框）。
- 每家 AI Harness 在菜单与输入框层的 UI、配色、1:1 输入框复刻、元信息全部独立封装在 `harnesses/<id>/` 文件夹中，编译期由 `apps/desktop/src-tauri/build.rs` 自动扫描注册。

---

## 如何新增一个 Harness 适配（只需新建一个文件夹）

### 第 1 步：复制模板目录
复制 `harnesses/_template/` 为 `harnesses/<your_harness_id>/`（目录名与 `harness.json` 中的 `id` 保持一致；`_` 开头的目录会被自动忽略）。

### 第 2 步：填写 `harness.json`
在 `harnesses/<your_harness_id>/harness.json` 中声明名称、排序、配色与默认提示文案：
- `id`: 唯一标识（如 `cursor`、`windsurf`、`opencode`）
- `order`: 在三级菜单最左侧「Harness 列表」中的排序权重（越小越靠前）
- `accent` / `accentBg` / `accentBorder` / `surface` / `border`: 主题色板
- `useNativeComposer`: 仅 `dsh` 为 `true`；其他外部 Harness 填 `false`

### 第 3 步（可选）：自定义 1:1 输入框 `ui.js`
- 如果只提供 `harness.json`（或 `ui.js` 不导出 `renderComposer`），悬浮条会自动使用内置的 **Adaptive Default Composer**，套用你的主题色与占位文案，立即可用。
- 如果想做到像 `antigravity`、`codex`、`claude_code` 那样的 **1:1 像素级桌面端输入框复刻**，在 `ui.js` 中实现 `renderComposer(ctx)` 即可，完全不需要修改 `packages/anything-to-prompt/lib/client.js` 或菜单代码！

### 第 4 步：重新编译
运行 `cargo build --release`（位于 `apps/desktop/src-tauri`）。`build.rs` 会自动发现新目录并将其注入三级级联菜单（**Harness → 工作区 → 对话**）与悬浮条运行时。

---

## English Quick Guide

Anything-to-Prompt runs **one shared host binary (`anything-to-prompt.exe`)** and auto-discovers every drop-in Harness module under `harnesses/<id>/` at build time via `apps/desktop/src-tauri/build.rs`.

1. Copy `harnesses/_template/` to `harnesses/<your_id>/`.
2. Edit `harness.json` (id, order, colors, labels, hints).
3. *(Optional)* Customize `ui.js` to return `{ css, renderComposer, renderRightControls, renderChipPrefix }`. If omitted, the host automatically renders an adaptive dark composer styled with your `harness.json` palette.
4. Run `cargo build --release` — your Harness automatically appears in the 3-level cascading menu (**Harness → Workspace → Conversation**) with zero edits to `client.js`.
