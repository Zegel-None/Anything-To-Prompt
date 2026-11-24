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
- **红框编号调试截图自动轮转**：本地保存在 `%LOCALAPPDATA%\AnythingToPrompt\captures\`，不直接上传图片，仅在胶囊隐藏上下文附带路径供智能体按需查阅，发送满 20 条消息或超时 60 分钟自动清理。

### 3. 极简无遮挡悬浮卡片
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
│   └── install-plugin.ps1            # 可选：手动安装/修复 @local/anything-to-prompt 插件脚本
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
| `appSourceMap` | `{}` | 可执行文件名/路径到本地源码目录的自定义映射表 |

---

## License

[MIT](./LICENSE)
