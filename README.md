# Anything-to-Prompt

> **Pick anything on your Windows desktop, straight into your AI prompt.**  
> 桌面万物直通 Prompt —— 面向 Windows 11 的极简无打扰悬浮条与全局元素拾取工具。

**Anything-to-Prompt** 基于 Tauri v2 + Rust (Win32 / UI Automation / 中完整性 Broker) 构建。它以极简圆角悬浮条常驻桌面，默认不侵入你正在使用的任何软件（也不注入 DSH 浏览器普通对话页），只需一个 `Ctrl` 键即可完成 **唤出 → 指哪选哪 → 编号胶囊入框 → 退出** 的完整闭环。

---

## 核心特性

### 1. 纯 `Ctrl` 单键闭环（零 Alt 冲突 · 8ms OS 物理键态驱动）
- **未唤起时双击 `Ctrl`（两短）**：立刻唤起悬浮输入框并自动聚焦光标，张手即打字。
- **未唤起时一短一长 `Ctrl`（双击第二下不撒手）**：唤起悬浮条并**直接进入选择模式**；点选或涂鸦圈选任意屏幕元素后，松开 `Ctrl` 自动退出选择模式，胶囊已入框且光标留在输入框中继续打字。
- **已激活态（聊天 / 输入 / 刚选完胶囊）**：
  - **单击 `Ctrl`（`< 220ms`）**：立刻关闭隐藏整个悬浮条；
  - **长按 `Ctrl`（`>= 220ms`）**：直接进入选择模式，松手退出选择模式并留在输入框；
  - **点击悬浮条外部任意位置**：立刻关闭隐藏整个悬浮条；
  - `Ctrl+C / V / A / Z / X / Backspace / Space / .`、`Ctrl+Click`、`Ctrl+Alt` 等所有常用组合键原样放行，零误触。

### 2. 桌面万物拾取 & 编号上下文胶囊（`① ② ③`）
- **蓝光玻璃指针**：选择模式下自动切换为空心暗玻璃描边 + 紧凑电光蓝外发光指针与克制短拖尾，实时高亮鼠标下的 UIA 控件。
- **大背景 0.5 秒防误触**：掠过窗口大背景或桌面画布时不闪烁误选；停稳 `0.5s` 后指针右下角转满小圆环进度条即可解锁选中。
- **点选 & 涂鸦圈选**：
  - **单击点选**：底层钩子无损吞掉目标窗口点击（点在关闭按钮 `✕` 或菜单上也绝不误触发），甚至支持选中 **Anything-to-Prompt 自身界面元素**；
  - **拖拽涂鸦圈选**：自由圈选任意区域，自动在包围盒内做 `6×6` 网格 UIA 采样去重与上下文提取。
- **按输入框当前胶囊数实时计数**：胶囊编号 `① ② ③` 严格与输入框当前实际存在的胶囊数量同步（删除/清空后自动从 `①` 重计，连续快选不撞号）。
- **深度应用上下文适配器**：
  - **浏览器**：抓取当前页面 URL；若为 `localhost:<port>` / `127.0.0.1:<port>`，自动反查监听进程 PID、可执行路径、命令行、工作目录与 Git 仓库。
  - **IDE / 编辑器**：VS Code / Antigravity / Cursor（解析 `storage.json` 映射工作区真实路径）、JetBrains、Visual Studio、Notepad++、Notepad。
  - **终端**：Windows Terminal / PowerShell / CMD / IDE 内置终端（抓取缓冲区文本 + 子进程工作目录与命令行）。
  - **文件 / Office / 专业软件**：资源管理器选区自动附带（含目录统计与 Git 分支）、Word / Excel / PPT / WPS / Outlook / SolidWorks / AutoCAD（通过中完整性 Broker 安全读取 COM）、微信 / 飞书 / 钉钉 / QQ / Obsidian / Unity / Unreal / Blender / Steam。
- **Computer Use 全坐标系机械锚定（适配 Codex / Claude Code / Antigravity / DSH / Windows-MCP）**：
  - 每次点选或拖拽，胶囊内自动机械计算并附带 `computer_use_grounding` 区块（`<0.1ms` 零误差），同时给出 **4 套坐标体系**：
    - `px`（物理屏幕像素，直喂 Win32 / `Windows-MCP`）；
    - `dip`（逻辑像素，自动除以当前显示器 `dpi_scale`）；
    - `norm1000`（`0~1000` 显示器归一化坐标，直喂 Gemini / Antigravity 及缩图后的 Claude / Codex CUA）；
    - `win_rel` & `win_ratio`（相对目标窗口左上角偏移，窗口被拖动后依然命中）；
  - 同步区分 **`element_center`（元素中心点）** 与 **`click_point`（用户落点）**、拖拽起终点及圈内每个子控件中心坐标，并附带 **UIA 交互能力画像**（`enabled / focusable / patterns=[Invoke,Value,Toggle,...] / state / runtime_id`）。
  - 支持在系统托盘一键切换 **多 Harness 投递目标**：`DSH 悬浮条` / `DSH + 同步复制到剪贴板` / `仅复制胶囊到剪贴板`（直接粘贴进 Codex、Claude Code、Antigravity）。
- **红框编号调试截图自动轮转**：本地保存在 `%LOCALAPPDATA%\AnythingToPrompt\captures\`，不直接上传图片，仅在胶囊隐藏上下文附带路径供智能体按需查阅，发送满 20 条消息或超时 60 分钟自动清理。

### 3. Office / WPS 原生选择模式 & 实时原地修改生态依赖
- **专为办公三件套定制的拾取手感**（兼容 **Microsoft Office 2016 ~ 2026** 与 **WPS 最新版**）：
  - **Word / WPS 文字**：选择模式下拖拽**不再画红线**，自动切换发光 I-beam 光标实时划选文字，胶囊附带选中文本、页码、段落上下文及精确字符偏移 `rangeStart..rangeEnd`。
  - **Excel / WPS 表格**：单击精准选中单个单元格（`Sheet!C5`，含显示值/原值/公式/行列标题），滑动拖拽显示绿色框选（**不画红线**）抓取区域表格与公式。
  - **PowerPoint / WPS 演示**：单击精准选中单个设计元素（文本框、形状、图片、表格及组合子元素），抓取 `SlideIndex`、`ShapeId`、几何坐标与样式；**拖拽仍保持画红线批注不变**。
  - **办公/画布类软件自动忽略背景**：在 Office / WPS 三件套、IDE、PDF、笔记及设计画布软件中自动关闭背景 0.5s 停顿框选，专注内容本身不误触。
- **不关闭原文档的实时修改生态依赖（全 MIT 协议）**：
  - 传统 `python-docx` / `openpyxl` / `python-pptx` 在文档打开时会撞文件锁。Anything-to-Prompt 的胶囊输出精准定位坐标，并自动路由到以下成熟开源实时 MCP 依赖：

  | 兼容套件 | MCP 名称 | 依赖项目 | License |
  | :--- | :--- | :--- | :---: |
  | **MS Office 2016 ~ 2026**（Excel） | `excel-live` | [`negokaz/excel-mcp-server`](https://github.com/negokaz/excel-mcp-server) | MIT |
  | **MS Office 2016 ~ 2026**（Word） | `word-live` | [`ykarapazar/word-mcp-live`](https://github.com/ykarapazar/word-mcp-live) | MIT |
  | **MS Office 2016 ~ 2026**（PPT） | `ppt-live` | [`ykuwai/ppt-mcp`](https://github.com/ykuwai/ppt-mcp) | MIT |
  | **WPS 最新版**（文字 / 表格 / 演示） | `wps-office` | [`lc2panda/wps-skills`](https://github.com/lc2panda/wps-skills) | MIT |

  一键自动探测并安装对应生态依赖（可加 `-ConfigureDsh` 自动写入 DSH MCP 配置）：

  ```powershell
  powershell -ExecutionPolicy Bypass -File .\scripts\install-office-mcp.ps1 -ConfigureDsh
  # 或显式指定：-Suite wps | -Suite msoffice | -Suite all
  ```

### 4. 极简无遮挡悬浮卡片
- **零隐形点击遮挡**：Win32 `SetWindowRgn` 动态圆角并集裁剪，折叠态仅为 `56px` 干净两行窄条，展开态向上生长，收起后原展开区 **100% 鼠标穿透**。
- **纯净对话流**：展开态自动过滤工具调用与思考块，相邻消息以精致深色阴影线分隔；左上角一排同心圆点代表 AI 历次回复（最左为最新回复，点击跳转），正中绝对居中拖拽调高把手，右上角无框斜图钉（空心/纯白实心固定）。
- **零侵入 DSH WebUI**：内置唯一配套插件 `@local/anything-to-prompt` 仅在 Anything-to-Prompt 自身悬浮窗（`?dsh_bar_clone=1`）生效，绝不向浏览器打开的 DSH 正常对话页注入任何样式或组件。

---

## 目录结构

```text
anything-to-prompt/
├── apps/
│   └── desktop/                      # Tauri v2 + Rust (Win32 / UIA / Broker) 桌面端源码
├── packages/
│   └── anything-to-prompt/           # 内置 DSH 配套插件 (@local/anything-to-prompt) 源码
├── scripts/
│   ├── install-plugin.ps1            # 可选：手动安装/修复 @local/anything-to-prompt 插件脚本
│   └── install-office-mcp.ps1        # 可选：一键安装 Office 2016-2026 / WPS 实时编辑 MCP 生态依赖
├── LICENSE                           # MIT License
└── README.md
```

---

## 快速开始

### 直接下载使用（推荐）

前往 GitHub **Releases** 下载最新版本：
- `anything-to-prompt.exe`（免安装绿色便携版）
- `Anything-to-Prompt_0.1.0_x64-setup.exe`（Windows 安装包）
- `Anything-to-Prompt_0.1.0_x64_en-US.msi`（MSI 安装包）

首次启动会自动将内置配套插件 `@local/anything-to-prompt` 同步到 `~/.dsh/profiles/web` 并连接本机 DeepSeek Harness（默认端口 `3080`）。驻留系统托盘后，**双击 `Ctrl`** 即可唤出输入框，**一短一长 `Ctrl`** 或**前台长按 `Ctrl`** 即可进入选择模式。

### 从源码自行构建

1. 环境要求：Windows 11（内置 WebView2）、Node.js >= 20、Rust / Cargo（`x86_64-pc-windows-msvc`）、已安装 DeepSeek Harness (`dsh`)。
2. 构建命令：

```powershell
# 安装前端依赖（首次）
npm --prefix .\apps\desktop ci

# 构建 Release 可执行文件与安装包
npm --prefix .\apps\desktop run build
```

构建产物位于 `apps/desktop/src-tauri/target/release/anything-to-prompt.exe` 以及 `target/release/bundle/{nsis,msi}/`。

---

## 配置文件说明

配置文件位于 `%APPDATA%\AnythingToPrompt\config.json`（可通过系统托盘菜单「打开配置文件」直接编辑）：

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `preferredPort` | `3080` | 优先连接的 DSH 端口 |
| `dshUrl` | `null` | 自定义 DSH 地址（留空自动探测 `instances/*.json`） |
| `preferAdmin` | `true` | 启动时优先请求管理员权限（拒绝 UAC 自动回退普通权限；DSH 与 COM 始终通过中完整性 Broker 非提权运行） |
| `autostart` | `false` | 是否开机自启（可在托盘菜单切换） |
| `barWidth` | `760` | 悬浮条默认宽度（支持左右内边缘拖拽调节，`480–1200`） |
| `doubleTapMs` | `400` | 双击 `Ctrl` 判定时间窗（毫秒） |
| `ctrlHoldMs` | `220` | 长按 `Ctrl` 进入选择模式阈值（毫秒） |
| `captureKeepMessages` | `20` | 调试截图保留的后续已发送消息条数 |
| `captureOrphanTtlMinutes` | `60` | 未发送孤儿截图自动清理分钟数 |
| `excludeFromCapture` | `true` | 常规截屏时隐藏悬浮条自身窗口（拾取自身元素时会自动临时放开） |
| `outputTarget` | `"dsh"` | 胶囊投递目标：`dsh`（悬浮条）/ `dsh_and_clipboard`（悬浮条+剪贴板）/ `clipboard_only`（仅复制，配合 Codex / Claude Code / Antigravity） |
| `appSourceMap` | `{}` | 可执行文件名/路径到本地源码目录的自定义映射表 |

---

## License

[MIT](./LICENSE)
