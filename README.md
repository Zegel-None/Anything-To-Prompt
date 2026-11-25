# Anything-to-Prompt

<p align="center">
  <b>简体中文</b> | <a href="./README_EN.md">English</a>
</p>

> **桌面万物直通 AI Prompt —— 面向 Windows 11 的极简无打扰悬浮条、多 Harness 1:1 原生复刻输入框、全局蓝/红双笔拾取与 Listary 级全盘瞬时搜索。**  
> **Pick anything on your Windows desktop, straight into your AI prompt.**

**Anything-to-Prompt** 基于 Tauri v2 + Rust（Win32 / UI Automation / NTFS USN MFT / 中完整性 Broker）构建。它以极简圆角悬浮条常驻桌面，默认不侵入你正在使用的任何软件，只需一个 `Ctrl` 键即可完成 **唤出 → 指哪选哪 / 框圈选 / 红线涂鸦 / 瞬时搜文件 → 编号胶囊入框 → 发送到 DeepSeek Harness / Antigravity / Codex** 的完整闭环。

---

## 核心特性

### 1. 多 Harness 一键切换 + 1:1 原生桌面输入框复刻
点击左下角工作区/对话胶囊，在顶部一级切换栏即可无缝切换 **DeepSeek Harness / Antigravity / Codex**，不修改任何原软件安装目录，所有外部 CLI 均通过 **中完整性（Medium Integrity）Broker** 非提权运行，绝不污染用户目录 ACL：

| Harness | 视觉与输入框形态 | 对话历史与桌面端同步 |
| :--- | :--- | :--- |
| **DeepSeek Harness (`dsh`)** | 原生 DSH 浮条输入卡 | 本地 WebUI 毫秒级双向实时孪生（零侵入浏览器正常页面） |
| **Antigravity (`agy.exe`)** | **1:1 复刻 Antigravity 桌面版输入框**（双层暗黑圆角、`Gemini 4 Neon High`、盾牌 `极速`、银白圆形发送键 / 红方块停止键） | 只读同步本地 `conversation_summaries.db` 与 `transcript.jsonl`，新消息通过 `agy --print --output-format stream-json` **原生同步到 Antigravity 桌面端 `CLI Project`** |
| **Codex (`codex.exe`)** | **1:1 复刻 Codex 桌面版堆叠输入框**（内缩 `#1f1f1f`「此计算机 / 工作区」卡槽、`20px` 大圆角石墨灰输入卡、橙红 `完全访问`、自动读取 `~/.codex/config.toml` 模型/推理档位、钴蓝圆形发送键） | 只读同步 `~/.codex/state_5.sqlite` 与 `rollout-*.jsonl`，经由 `codex app-server` JSON-RPC 2.0 写入同库共享历史（桌面端刷新可见） |

### 2. 纯 `Ctrl` 单键闭环 + 蓝/红双键画笔（CAD 级框选↔圈选无缝切换）
- **纯 `Ctrl` 键态控制（8ms OS 物理键态驱动，零 Alt / 组合键冲突）**：
  - 未唤起时 **双击 `Ctrl`** 立即唤出并聚焦输入框；**一短一长 `Ctrl`** 直接进入选择模式；
  - 已激活时 **单击 `Ctrl`（`< 220ms`）** 或点击外部立即隐藏，**长按 `Ctrl`** 进入选择模式；
  - `Ctrl+C / V / A / Z / X / Backspace / Space / .`、`Ctrl+Click`、`Ctrl+Alt` 原样放行。
- **左键短按（单元素拾取）**：底层钩子无损吞掉目标窗口点击，提取精确 UIA 控件、路径、URL、代码工作区、Office/WPS 内容或 SolidWorks 特征。
- **左键拖拽（蓝线 · CAD 级框选 ↔ 自由圈选自动切换 + 文本框选专项优化）**：
  - **对角直线拖拽（如左上 → 右下）** → 实时预览**蓝色半透明框选方框**；
  - **途中弯曲、折返、画圆或不规则轨迹** → 以 **160ms 平滑淡出/绽放动效** 无缝切换为**蓝色自由圈选套索**，蓝线**接触到或围住的全部前景元素**一网打尽；
  - **斜划文本精准选字**：从文本开头第一个字斜划到结尾最后一个字，优先通过 UIA `TextPattern::RangeFromPoint`（及 Win32 `Edit` / 同容器多行 `Text` 兜底）直接提取首尾字符间的整段文字，生成 **`① 选字 · 开头预览…`** 胶囊；
  - **前景元素组（严格排除背景）**：框/圈中多个非纯文本前景元素自动打包为 **`① 元素组 (N项)`** 胶囊，自动屏蔽桌面壁纸与大块背景容器，绝不误选背景。
- **右键拖拽（红线 · 双图涂鸦截图 + 彻底屏蔽右键菜单）**：
  - `Ctrl` 选择模式下底层钩子同时吞掉 `WM_RBUTTONDOWN` 与 `WM_RBUTTONUP`，彻底杜绝右键菜单弹出；
  - 始终绘制**红色自由涂鸦**，松手生成 **`① 红线截图`** 胶囊，并自动保存**两张同一时刻的截图**：
    1. `screenshot_annotated_path`：带红线涂鸦与编号，给视觉模型精准定位关注区域；
    2. `screenshot_clean_path`：无红线遮挡的原始高清参考图，避免粗线条挡住小字或图标细节。

### 3. Listary 级全盘瞬时搜索（纯文字即搜即得 · 0 匹配自动隐藏）
在空输入框（无胶囊、非 `/` 命令、单行纯文本）中**从第 1 个字符起**实时触发，若继续输入正常 AI 句子导致 0 匹配则瞬间自动隐藏，绝不打扰正常聊天：
- **三层混合高速引擎**：
  - **Layer 0**：自动探测本机运行中的 `voidtools Everything`（零 DLL 原生 `WM_COPYDATA` IPC）；
  - **Tier 1（~300ms 冷启动就绪）**：基于 `ignore` 遍历开始菜单应用 + 桌面/文档/下载/工作区高频目录；
  - **Tier 2（后台并发全盘索引）**：原生 Win32 `FSCTL_ENUM_USN_DATA`（1 MiB 缓冲并发扫描所有固定 NTFS 盘符，过滤系统/依赖噪音）+ `FSCTL_READ_USN_JOURNAL` 2 秒增量监听；
  - 统一融合 Helix **`nucleo-matcher`** 模糊打分、**中文全拼/首字母匹配**（如输入 `wx` 首位命中「微信」）与本地 Frecency 频次加权，18 万+ 全盘索引下平均检索耗时 **< 10 ms**。
- **键盘无感工作流**：`Tab` / `Shift+Tab`（选中后可用 `↑/↓`）选择结果 → `Enter` 将文件/文件夹/应用插入为**蓝色胶囊**并清空搜索词；`Ctrl+Enter` 或双击通过中完整性 Broker 直接打开；未按 `Tab` 直接 `Enter` 原样发送给 AI。

### 4. SolidWorks 3D 特征拾取 + Zegel-SWE 生态协同
- 在 SolidWorks 中按住 `Ctrl` 点击即可直接拾取 FeatureManager 特征树节点或 3D 视口实体（三层 COM/UIA 管线：特征树标签匹配 → 数学求逆 `IModelView.Transform` + `Extension.SelectByRay` 射线拾取 → `ISelectionMgr` 预选回读），输出结构化 `[SW-TARGET]` 尺寸、父子特征与 Zegel-SWE 语义选择器（`__current_selection__` / `+z/-z/+x/-x/+y/-y` / `point_mm`）。
- 提供一键脚本挂载 [`zegel-solidworks-engineering-delivery`](https://github.com/q862877400-ux/zegel-solidworks-engineering-delivery) 工程交付 MCP 生态（544 工具 + 66 机器门禁）：
  ```powershell
  powershell -ExecutionPolicy Bypass -File .\scripts\install-solidworks-mcp.ps1 -ConfigureDsh
  ```

### 5. Office / WPS 原生选择 & Computer Use 四坐标系锚定
- **Office 2016–2026 / WPS 最新版**：左键在 Word/WPS 中自动切换发光 I-beam 划选文字、在 Excel/WPS 中显示绿色单元格框选、在 PPT 中精确定位形状与幻灯片；右键红线在三件套内仍可随时涂鸦双图截图。支持一键挂载开源 MIT 实时编辑 MCP（`scripts/install-office-mcp.ps1`）。
- **Computer Use 四坐标系机械锚定**：每个拾取胶囊自动附带 `px`（物理像素）、`dip`（逻辑像素）、`norm1000`（`0~1000` 归一化）与 `win_rel / win_ratio`（窗口相对坐标）以及 UIA 交互能力画像，直喂 DSH / Antigravity / Codex / Claude Code / Windows-MCP。
- **零隐形点击遮挡**：Win32 `SetWindowRgn` 动态圆角裁剪，收起后原展开区 100% 鼠标穿透。

---

## 目录结构

```text
anything-to-prompt/
├── apps/
│   └── desktop/                      # Tauri v2 + Rust (Win32 / UIA / NTFS USN / Broker) 桌面端源码
├── packages/
│   └── anything-to-prompt/           # 内置 DSH 配套插件 (@local/anything-to-prompt) 源码
├── scripts/
│   ├── install-plugin.ps1            # 手动安装/修复 @local/anything-to-prompt 插件
│   ├── install-office-mcp.ps1        # 一键安装 Office 2016–2026 / WPS 实时编辑 MCP 生态依赖
│   └── install-solidworks-mcp.ps1    # 一键挂载 SolidWorks (Zegel-SWE) 工程交付 MCP 生态
├── LICENSE                           # MIT License
├── README.md                         # 简体中文文档（默认）
└── README_EN.md                      # English Documentation
```

---

## 快速开始

### 直接下载使用（推荐）

前往 GitHub **Releases** 下载 `v0.2.0` 最新版本：
- `anything-to-prompt.exe`（免安装绿色便携版）
- `Anything-to-Prompt_0.2.0_x64-setup.exe`（Windows NSIS 安装包）
- `Anything-to-Prompt_0.2.0_x64_en-US.msi`（Windows MSI 安装包）

首次启动会自动同步内置插件 `@local/anything-to-prompt` 并连接本机 DSH（默认端口 `3080`），同时自动探测本机 `agy.exe`（Antigravity）与 `codex.exe`（Codex）。

### 从源码自行构建

1. 环境要求：Windows 11（内置 WebView2）、Node.js >= 20、Rust / Cargo（`x86_64-pc-windows-msvc`）。
2. 构建命令：

```powershell
# 安装前端依赖（首次）
npm --prefix .\apps\desktop ci

# 构建 Release 便携版与 NSIS/MSI 安装包
npm --prefix .\apps\desktop run build
```

构建产物位于 `apps/desktop/src-tauri/target/release/anything-to-prompt.exe` 以及 `target/release/bundle/{nsis,msi}/`。

---

## 配置文件说明

配置文件位于 `%APPDATA%\AnythingToPrompt\config.json`（可通过系统托盘菜单「打开配置文件」编辑）：

| 字段 | 默认值 | 说明 |
| :--- | :--- | :--- |
| `activeHarness` | `"dsh"` | 当前激活的 Harness：`dsh` / `antigravity` / `codex` |
| `preferredPort` | `3080` | 优先连接的 DSH 端口 |
| `dshUrl` | `null` | 自定义 DSH 地址（留空自动探测） |
| `preferAdmin` | `true` | 启动时优先请求管理员权限（支持全盘 NTFS MFT 枚举与高权限窗口拾取；所有外部 CLI 与 COM 严格通过中完整性 Broker 非提权运行） |
| `autostart` | `false` | 是否开机自启 |
| `barWidth` | `760` | 悬浮条默认宽度（`480–1200`，支持左右边缘拖拽调节） |
| `doubleTapMs` | `400` | 双击 `Ctrl` 判定时间窗（毫秒） |
| `ctrlHoldMs` | `220` | 长按 `Ctrl` 进入选择模式阈值（毫秒） |
| `searchRoots` | `[]` | Listary 瞬时搜索额外自定义高优根目录列表 |
| `captureKeepMessages` | `20` | 截图保留的后续已发送消息条数 |
| `captureOrphanTtlMinutes` | `60` | 未发送孤儿截图自动清理分钟数 |
| `excludeFromCapture` | `true` | 常规截屏时隐藏悬浮条自身窗口 |
| `outputTarget` | `"dsh"` | 胶囊投递目标：`dsh` / `dsh_and_clipboard` / `clipboard_only` |
| `appSourceMap` | `{}` | 可执行文件到本地源码目录的自定义映射表 |

---

## License

[MIT](./LICENSE)
