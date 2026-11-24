use crate::pick::adapters::{AdapterContext, AdapterOutput};
use std::fs;
use std::path::{Path, PathBuf};
use url::Url;

const VSCODE_FAMILY: &[(&str, &str)] = &[
    ("code.exe", "Code"),
    ("code - insiders.exe", "Code - Insiders"),
    ("cursor.exe", "Cursor"),
    ("antigravity.exe", "Antigravity"),
    ("windsurf.exe", "Windsurf"),
    ("vscodium.exe", "VSCodium"),
];

const JETBRAINS_EXES: &[&str] = &[
    "idea64.exe",
    "pycharm64.exe",
    "webstorm64.exe",
    "goland64.exe",
    "clion64.exe",
    "rustrover64.exe",
    "rider64.exe",
    "phpstorm64.exe",
    "studio64.exe",
];

pub fn matches(ctx: &AdapterContext) -> bool {
    let exe = ctx.proc_meta.exe_name.as_str();
    VSCODE_FAMILY.iter().any(|(e, _)| *e == exe)
        || JETBRAINS_EXES.contains(&exe)
        || matches!(exe, "devenv.exe" | "notepad++.exe" | "notepad.exe")
}

pub fn run(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let mut out = AdapterOutput::new("ide");
    let exe = ctx.proc_meta.exe_name.as_str();

    if let Some((_, app_dir_name)) = VSCODE_FAMILY.iter().find(|(e, _)| *e == exe) {
        let (file_hint, project_hint) = parse_vscode_window_title(&ctx.window_title);
        if let Some(f) = &file_hint {
            out.lines.push(format!("active_editor_tab: {f}"));
        }
        if let Some(proj) = &project_hint {
            out.lines.push(format!("workspace_name: {proj}"));
            if let Some(ws_path) = resolve_vscode_workspace_folder(app_dir_name, proj) {
                out.lines
                    .push(format!("workspace_root: {}", ws_path.display()));
                out.paths.push(ws_path.to_string_lossy().to_string());
                if let Some(f) = &file_hint {
                    let candidate = ws_path.join(f);
                    if candidate.exists() {
                        out.lines
                            .push(format!("active_file_path: {}", candidate.display()));
                        out.paths.push(candidate.to_string_lossy().to_string());
                    }
                }
            }
        }
        out.badge = file_hint.or(project_hint);
    } else if JETBRAINS_EXES.contains(&exe) {
        // JetBrains title format: "project – file.rs" or "project [D:\path\to\project] - ..."
        let (proj, file_or_tail, bracket_path) = parse_jetbrains_title(&ctx.window_title);
        if let Some(p) = &proj {
            out.lines.push(format!("jetbrains_project: {p}"));
        }
        if let Some(f) = &file_or_tail {
            out.lines.push(format!("active_file_hint: {f}"));
        }
        if let Some(bp) = bracket_path {
            out.lines.push(format!("project_path: {bp}"));
            out.paths.push(bp);
        }
        out.badge = file_or_tail.or(proj);
    } else if exe == "notepad++.exe" {
        // Notepad++ title: "*C:\path\to\file.txt - Notepad++" or "C:\path\to\file.txt - Notepad++"
        if let Some(p) = parse_notepad_plusplus_title(&ctx.window_title) {
            out.lines.push(format!("document_path: {p}"));
            if Path::new(&p).exists() {
                out.paths.push(p.clone());
            }
            out.badge = Path::new(&p)
                .file_name()
                .map(|s| s.to_string_lossy().to_string());
        }
    } else if exe == "notepad.exe" {
        if let Some(p) = extract_path_from_cmdline(&ctx.proc_meta.cmdline, "notepad.exe") {
            if Path::new(&p).exists() {
                out.lines.push(format!("document_path: {p}"));
                out.paths.push(p);
            }
        }
        let clean = ctx
            .window_title
            .trim_start_matches('*')
            .split(" - ")
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if !clean.is_empty() {
            out.lines.push(format!("notepad_tab: {clean}"));
            out.badge = Some(clean);
        }
    } else if exe == "devenv.exe" {
        let clean = ctx
            .window_title
            .split(" - Microsoft Visual Studio")
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if !clean.is_empty() {
            out.lines.push(format!("visual_studio_solution: {clean}"));
            out.badge = Some(clean);
        }
    }

    if let Some(el) = &ctx.primary_element {
        if !el.text_snippet.is_empty() {
            out.lines.push(format!(
                "editor_text_excerpt:\n{}",
                truncate_lines(&el.text_snippet, 40)
            ));
        }
    }

    if out.lines.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub fn parse_vscode_window_title(title: &str) -> (Option<String>, Option<String>) {
    // Typical title: "● client.js - Deepseek Bar - Antigravity" or "lib.rs - DeepSeek-Bar - Visual Studio Code"
    let parts: Vec<&str> = title
        .split(" - ")
        .map(|s| s.trim().trim_start_matches('●').trim())
        .filter(|s| !s.is_empty())
        .collect();

    if parts.len() >= 3 {
        let file = parts[0].to_string();
        let proj = parts[parts.len() - 2]
            .trim_end_matches("(Workspace)")
            .trim_end_matches("(工作区)")
            .trim()
            .to_string();
        (Some(file), Some(proj))
    } else if parts.len() == 2 {
        (None, Some(parts[0].to_string()))
    } else {
        (None, None)
    }
}

fn resolve_vscode_workspace_folder(app_dir_name: &str, project_name: &str) -> Option<PathBuf> {
    let appdata = dirs::config_dir()?;
    let storage_json = appdata
        .join(app_dir_name)
        .join("User")
        .join("globalStorage")
        .join("storage.json");
    let raw = fs::read_to_string(storage_json).ok()?;
    resolve_workspace_from_storage_json(&raw, project_name)
}

pub fn resolve_workspace_from_storage_json(raw_json: &str, project_name: &str) -> Option<PathBuf> {
    let val: serde_json::Value = serde_json::from_str(raw_json).ok()?;
    let target_lower = project_name.trim().to_ascii_lowercase();
    let mut uri_strings = Vec::new();

    if let Some(assoc) = val
        .get("profileAssociations")
        .and_then(|p| p.get("workspaces"))
        .and_then(|w| w.as_object())
    {
        for k in assoc.keys() {
            uri_strings.push(k.clone());
        }
    }
    if let Some(bw) = val
        .get("backupWorkspaces")
        .and_then(|b| b.get("folders"))
        .and_then(|f| f.as_array())
    {
        for item in bw {
            if let Some(u) = item.get("folderUri").and_then(|u| u.as_str()) {
                uri_strings.push(u.to_string());
            }
        }
    }

    for uri in uri_strings {
        if let Ok(parsed) = Url::parse(&uri) {
            if parsed.scheme() == "file" {
                if let Ok(path) = parsed.to_file_path() {
                    let name_match = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_ascii_lowercase() == target_lower)
                        .unwrap_or(false);
                    let stem_match = path
                        .file_stem()
                        .map(|n| n.to_string_lossy().to_ascii_lowercase() == target_lower)
                        .unwrap_or(false);
                    if name_match || stem_match {
                        if path.is_file() {
                            return path.parent().map(|p| p.to_path_buf());
                        }
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}

pub fn parse_jetbrains_title(title: &str) -> (Option<String>, Option<String>, Option<String>) {
    let mut bracket_path = None;
    if let (Some(l), Some(r)) = (title.find('['), title.find(']')) {
        if r > l + 2 {
            let inside = title[l + 1..r].trim();
            if inside.contains('\\') || inside.contains('/') {
                bracket_path = Some(inside.to_string());
            }
        }
    }
    let sep = if title.contains(" – ") {
        " – "
    } else {
        " - "
    };
    let parts: Vec<&str> = title.split(sep).map(|s| s.trim()).collect();
    if parts.len() >= 2 {
        let proj = parts[0]
            .split('[')
            .next()
            .unwrap_or(parts[0])
            .trim()
            .to_string();
        let file = parts[1].to_string();
        (Some(proj), Some(file), bracket_path)
    } else {
        (None, None, bracket_path)
    }
}

pub fn parse_notepad_plusplus_title(title: &str) -> Option<String> {
    let stripped = title.trim().trim_start_matches('*').trim();
    let before = stripped.split(" - Notepad++").next()?.trim();
    if before.len() >= 3 && (before.contains(":\\") || before.starts_with("\\\\")) {
        Some(before.to_string())
    } else {
        None
    }
}

pub fn extract_path_from_cmdline(cmdline: &str, exe_hint: &str) -> Option<String> {
    let trimmed = cmdline.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let idx = lower.find(&exe_hint.to_ascii_lowercase())?;
    let after = trimmed[idx + exe_hint.len()..]
        .trim_start_matches('"')
        .trim();
    let cleaned = after.trim_matches('"').trim();
    if cleaned.len() >= 3 && (cleaned.contains(":\\") || cleaned.starts_with("\\\\")) {
        Some(cleaned.to_string())
    } else {
        None
    }
}

fn truncate_lines(s: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    if lines.len() <= max_lines {
        s.to_string()
    } else {
        lines[lines.len() - max_lines..].join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vscode_and_antigravity_titles_and_storage() {
        let (f, p) = parse_vscode_window_title("● client.js - Deepseek Bar - Antigravity");
        assert_eq!(f.as_deref(), Some("client.js"));
        assert_eq!(p.as_deref(), Some("Deepseek Bar"));

        let sample_storage = r#"{
            "profileAssociations": {
                "workspaces": {
                    "file:///d%3A/Agent-Studio/Antigravity-Workspace/Deepseek%20Bar": "__default__profile__"
                }
            }
        }"#;
        let resolved = resolve_workspace_from_storage_json(sample_storage, "Deepseek Bar").unwrap();
        assert_eq!(
            resolved,
            PathBuf::from(r"d:\Agent-Studio\Antigravity-Workspace\Deepseek Bar")
        );
    }

    #[test]
    fn parses_notepad_plusplus_and_jetbrains_titles() {
        assert_eq!(
            parse_notepad_plusplus_title(r"*D:\notes\todo.md - Notepad++").as_deref(),
            Some(r"D:\notes\todo.md")
        );
        let (proj, file, path) =
            parse_jetbrains_title(r"deepseek-bar [D:\work\deepseek-bar] – src\lib.rs");
        assert_eq!(proj.as_deref(), Some("deepseek-bar"));
        assert_eq!(file.as_deref(), Some(r"src\lib.rs"));
        assert_eq!(path.as_deref(), Some(r"D:\work\deepseek-bar"));
    }
}
