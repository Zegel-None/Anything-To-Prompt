# Anything-to-Prompt 深度追踪适配器插件规范 (`atp-adapter/v1`)

Anything-to-Prompt 将对各桌面软件（CAD / Office / IDE / 浏览器 / 聊天工具 / 3D 引擎 / 音视频 / 行业软件）的深度上下文追踪统一为**可热插拔、可分享、可远程下载的插件体系**。

## 1. 安装位置与热插拔

- 本地插件目录：`%LOCALAPPDATA%\AnythingToPrompt\adapter-plugins\`
- 每个扩展插件占用独立子目录：`<plugins_dir>/<plugin-id>/adapter.json`（及可选脚本/二进制文件）
- 启用/禁用状态持久化在 `<plugins_dir>/state.json`
- **零重启热插拔**：在设置面板 →「深度追踪插件（热插拔 & 社区下载）」中随时开关任意内置或外部插件、粘贴 GitHub / HTTPS 链接下载、导入本地 `.json` / `.zip` 或从社区市场一键安装，下次 Ctrl 点选立即生效。

## 2. 插件清单 (`adapter.json`)

```json
{
  "schemaVersion": 1,
  "id": "my-cad-inspector",
  "name": "某软件深度追踪",
  "version": "1.0.0",
  "author": "your-name",
  "description": "提取当前工程路径、选中对象属性并优化 UI 找框策略",
  "homepage": "https://github.com/you/my-cad-inspector",
  "category": "cad_3d",
  "match": {
    "exes": ["mycad.exe"],
    "windowClasses": [],
    "titleContains": []
  },
  "capabilities": ["enrich_pick", "tap_override", "ui_profile"],
  "priority": 100,
  "timeoutMs": 1200,
  "overrideBuiltin": false,
  "replacesBuiltinId": null,
  "integrity": "inherit",
  "uiProfile": {
    "uiaPolicy": "watchdog_only",
    "preferredLayers": ["static_visual_hit_map", "msaa_hit_map"],
    "ignoreBackground": true,
    "toolbarClasses": [],
    "ignoreCanvasClasses": []
  },
  "runtime": {
    "kind": "declarative",
    "rules": []
  }
}
```

## 3. 两种外部运行时

### 3.1 `declarative`（零代码纯 JSON 插件，最推荐，免信任确认即装即用）
支持在 `window_title`、`cmdline`、`exe_path`、`window_class`、`element_name`、`element_value` 上执行零开销确定性提取：
- `op`: `"whole"` | `"contains"` | `"prefix"` | `"suffix"` | `"between"` (`arg1`, `arg2`) | `"split"` (`arg1`, `index`, 支持负索引) | `"file_ext"` (`arg1` 如 `".blend"`)
- 可同时附带 `uiProfile` 覆盖目标软件的 UIA 策略、找框层序和忽略画布类名。

### 3.2 `command`（外部脚本 / 可执行程序，`json_stdio_v1` 协议）
支持 Python、Node.js、PowerShell 或任意原生 `.exe`：
- **安全机制**：首次安装默认处于「待信任」禁用状态，在设置界面明确展示真实执行命令，须用户手动点击「信任并启用」后才会执行；运行时强制限制 `200~3500ms` 超时自动终止与 `64KB` 输出上限。
- **中权限 COM 访问**：若主程序以管理员运行而插件需要访问中权限 COM（如 Rhino / AutoCAD / Revit / Office），设置 `"integrity": "medium"` 并将 `command` 设为 `powershell.exe` + `.ps1` 文件，宿主会自动通过中权限 Broker 转发执行。
- **单文件分享**：支持通过 `inlineScript` + `inlineFilename` 将脚本直接内嵌到单个 `adapter.json` 中，他人粘贴一个 GitHub 链接即可一键安装。

#### `stdin` 请求格式 (`atp-adapter/v1`)
```json
{
  "protocol": "atp-adapter/v1",
  "hook": "enrich_pick",
  "hwnd": 123456,
  "pid": 44196,
  "exeName": "blender.exe",
  "exePath": "C:\\Program Files\\Blender\\blender.exe",
  "cmdline": "...",
  "windowTitle": "scene.blend - Blender",
  "windowClass": "GHOST_WindowClass",
  "tapPoint": [640, 480],
  "primaryElement": null
}
```

#### `stdout` 响应格式
```json
{
  "ok": true,
  "badge": "scene.blend",
  "title": "Cube · Mesh",
  "detail": "Blender · scene.blend",
  "lines": ["active_object: Cube", "modifiers: [Subsurf, Bevel]"],
  "paths": ["D:\\Models\\scene.blend"],
  "overrideCapsule": false
}
```
> 若插件声明 `"capabilities": ["tap_override"]` 并在 `hook == "tap_override"` 时返回 `"overrideCapsule": true`，可直接接管该软件的单击拾取并生成定制胶囊（与 SolidWorks / Office 同等级别能力）。

## 4. 分享与发布到社区市场

1. 在设置面板点击任意插件的 **「导出」** 即可复制完整自包含 `adapter.json` 发给他人。
2. 将 `adapter.json`（或 `.zip` 包）上传到 GitHub 仓库，他人直接粘贴 GitHub 文件链接即可安装（宿主自动将 `github.com/.../blob/...` 转为 raw 地址）。
3. 向 `adapter-registry/index.json` 提交 PR，或在设置中配置自定义团队市场索引 URL。
