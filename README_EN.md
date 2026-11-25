# Anything-to-Prompt

<p align="center">
  <a href="./README.md">简体中文</a> | <b>English</b>
</p>

> **Pick anything on your Windows desktop, straight into your AI prompt.**  
> A minimal, non-intrusive Windows 11 floating prompt bar with 1:1 native replicated composers for DeepSeek Harness / Antigravity / Codex, CAD-style dual-pen desktop picking (Blue Auto-Box/Lasso + Red Dual-Screenshot), SolidWorks 3D feature picking, and Listary-grade instant full-drive search.

Built with **Tauri v2 + Rust** (Win32 / UI Automation / NTFS USN MFT / Medium-Integrity Broker). Press `Ctrl` anywhere on Windows to summon the bar, pick UI controls / text / 3D CAD features / annotated screenshots / local files as numbered capsules (`① ② ③`), and send straight to your AI agent.

---

## Key Features

### 1. Multi-Harness Switcher with 1:1 Native Desktop Composer Replicas
Switch seamlessly between **DeepSeek Harness**, **Antigravity**, and **Codex** from the workspace/session popover. No third-party app binaries are modified, and all external CLIs run strictly at **Medium Integrity** via the built-in Broker (never elevating or corrupting `~/.codex` / `~/.gemini` ACLs):

| Harness | Composer UI | History & Desktop Sync |
| :--- | :--- | :--- |
| **DeepSeek Harness (`dsh`)** | Native DSH floating composer | Real-time bidirectional twin with local DSH WebUI (zero injection into normal browser tabs) |
| **Antigravity (`agy.exe`)** | **1:1 Antigravity Desktop composer replica** (double-bezel dark obsidian frame, `Gemini 4 Neon High`, shield speed badge, light circular send / crimson square stop button) | Read-only sync from `conversation_summaries.db` & `transcript.jsonl`; new turns stream via `agy --print --output-format stream-json` directly into Antigravity Desktop's **`CLI Project`** |
| **Codex (`codex.exe`)** | **1:1 Codex Desktop stacked composer replica** (inset `#1f1f1f` computer/workspace shelf, `20px` graphite card, coral-orange access mode, live model/effort parsed from `~/.codex/config.toml`, cobalt-blue circular send button) | Read-only sync from `~/.codex/state_5.sqlite` & `rollout-*.jsonl`; turns run via `codex app-server` JSON-RPC 2.0 sharing the exact same `~/.codex` history |

### 2. Pure `Ctrl` Loop + Dual-Button Pen (CAD Auto-Box↔Lasso + Red Dual-Screenshot)
- **Zero-Conflict `Ctrl` State Machine (8ms OS physical key-state polling)**:
  - Double-tap `Ctrl` to summon & focus; tap-then-hold `Ctrl` (or hold `Ctrl` while summoned) to enter Selection Mode; single-tap `Ctrl` or click outside to hide.
  - All chords (`Ctrl+C/V/A/Z/X`, `Ctrl+Click`, `Ctrl+Alt`) pass through untouched.
- **Left Click**: Lossless single-element pick (buttons, URLs, IDE workspaces, terminal buffers, Office/WPS elements, SolidWorks 3D features).
- **Left Drag (Blue Ink — CAD Auto-Switch Box ↔ Freehand Lasso + Diagonal Text Sweep)**:
  - **Monotonic diagonal drag** previews a translucent **blue selection box**;
  - **Curving, reversing, or looping mid-drag** smoothly morphs (`160ms`) into a **blue freehand lasso** that selects every foreground element touched or enclosed;
  - **Diagonal Text Selection**: Dragging from the first character to the last character of a paragraph extracts the exact character span via UIA `TextPattern::RangeFromPoint` (with Win32 `Edit` `EM_CHARFROMPOS` & sibling `Text` reading-order fallbacks);
  - **Foreground Element Groups (`元素组`)**: Multiple selected controls are bundled into a single numbered `元素组 (N项)` capsule, strictly excluding desktop wallpaper and large background containers.
- **Right Drag (Red Ink — Dual Annotated + Clean Reference Screenshots & Context Menu Suppression)**:
  - Both `WM_RBUTTONDOWN` and `WM_RBUTTONUP` are swallowed in `Ctrl` Selection Mode so right-click context menus never pop up;
  - Always draws a **red freehand doodle** and produces a **`红线截图`** capsule containing **two synchronized PNGs** (auto-rotated):
    1. `screenshot_annotated_path`: with the red doodle & numbered badge so vision models immediately see where you pointed;
    2. `screenshot_clean_path`: clean unoccluded original capture so thick strokes never obscure small text or icons.

### 3. Listary-Grade Instant Full-Drive Search (Auto-Hide on 0 Matches)
Starts matching from the 1st character on single-line pure text (no capsules, not `/` commands), and immediately hides whenever a normal AI sentence yields 0 file matches:
- **3-Tier Hybrid Engine**:
  - **Layer 0**: Auto-detects running `voidtools Everything` via zero-DLL Win32 `WM_COPYDATA` IPC;
  - **Tier 1 (~300ms ready)**: Start Menu apps + Desktop / Documents / Downloads / Workspaces via `ignore`;
  - **Tier 2 (Parallel NTFS USN/MFT)**: Concurrent Win32 `FSCTL_ENUM_USN_DATA` across all fixed NTFS drives + 2s incremental `FSCTL_READ_USN_JOURNAL` watcher;
  - Unified Helix **`nucleo-matcher`** fuzzy scoring, **Chinese full/initial Pinyin matching**, and persistent frecency (< 10ms over 180k+ indexed entries).
- **Keyboard Flow**: `Tab` / `Shift+Tab` (or `↑/↓` once selected) → `Enter` inserts as a **Blue Capsule** and clears search text; `Ctrl+Enter` / double-click opens at Medium Integrity; pressing `Enter` without `Tab` submits directly to AI.

### 4. SolidWorks 3D Feature Picking & Office/WPS Live Ecosystems
- **SolidWorks 2026+**: 3-tier COM/UIA feature & entity picker (FeatureManager tree match → pure-math `IModelView.Transform` ray-pick via `SelectByRay` → `ISelectionMgr` readback) emitting `[SW-TARGET]` structured anchors aligned with [`zegel-solidworks-engineering-delivery`](https://github.com/q862877400-ux/zegel-solidworks-engineering-delivery) (`scripts/install-solidworks-mcp.ps1`).
- **MS Office 2016–2026 & WPS Office**: Native glowing I-beam text selection in Word/WPS, green cell range box in Excel/WPS, shape picking in PPT, plus one-click MIT live-editing MCP installer (`scripts/install-office-mcp.ps1`).
- **4-Coordinate Computer Use Grounding**: Every pick includes `px`, `dip`, `norm1000`, and `win_rel / win_ratio` coordinates plus UIA interaction affordances.

---

## Quick Start

### Download Prebuilt Binaries (Recommended)
Download `v0.2.0` from GitHub **Releases**:
- `anything-to-prompt.exe` (portable)
- `Anything-to-Prompt_0.2.0_x64-setup.exe` (NSIS installer)
- `Anything-to-Prompt_0.2.0_x64_en-US.msi` (MSI installer)

### Build from Source
Requirements: Windows 11 (WebView2), Node.js >= 20, Rust / Cargo (`x86_64-pc-windows-msvc`).

```powershell
npm --prefix .\apps\desktop ci
npm --prefix .\apps\desktop run build
```

---

## License

[MIT](./LICENSE)
