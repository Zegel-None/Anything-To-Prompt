use crate::broker::BrokerRequest;
use crate::pick::adapters::ide::extract_path_from_cmdline;
use crate::pick::adapters::{AdapterContext, AdapterOutput};
use std::fs;
use std::path::{Path, PathBuf};
use uiautomation::types::{ControlType, TreeScope, UIProperty};
use uiautomation::UIAutomation;

// ============================================================================
// Wave B Deep App Adapters:
// 1. PDF Readers (Acrobat, SumatraPDF, Foxit, Browser/WPS .pdf)
// 2. Chat Apps (WeChat, Feishu/Lark, DingTalk, QQ, Teams, Slack, Telegram, Discord -> ±5 messages)
// 3. Outlook (Classic COM + New Outlook UIA)
// 4. Notes (OneNote, Obsidian via obsidian.json, Notion, Typora)
// 5. CAD (SolidWorks & AutoCAD via medium-integrity COM)
// 6. Game/3D Engines (Unity, Unreal Editor, Blender)
// 7. Design & Dev Tools (Figma, Photoshop, GitHub Desktop, Sourcetree, Postman, DBeaver, Navicat)
// 8. Media Players (PotPlayer, VLC, mpv)
// 9. System / Power Tools (TaskMgr, ProcExp, RegEdit, Everything, TotalCmd, DOpus, BeyondCompare, WinMerge, Docker Desktop, Steam VDF)
// ============================================================================

const PDF_EXES: &[&str] = &[
    "acrobat.exe",
    "acrord32.exe",
    "sumatrapdf.exe",
    "foxitpdfreader.exe",
    "foxitreader.exe",
    "pdfxedit.exe",
];

const CHAT_EXES: &[&str] = &[
    "weixin.exe",
    "wechat.exe",
    "wechatappex.exe",
    "feishu.exe",
    "lark.exe",
    "dingtalk.exe",
    "qq.exe",
    "ms-teams.exe",
    "teams.exe",
    "slack.exe",
    "telegram.exe",
    "discord.exe",
];

pub fn run_all_wave_b(ctx: &AdapterContext) -> Vec<AdapterOutput> {
    let mut results = Vec::new();
    for func in [
        try_pdf_adapter,
        try_chat_adapter,
        try_outlook_adapter,
        try_notes_adapter,
        try_cad_adapter,
        try_engine_adapter,
        try_dev_design_adapter,
        try_media_adapter,
        try_sys_tools_adapter,
    ] {
        if let Some(out) = func(ctx) {
            results.push(out);
        }
    }
    results
}

fn try_pdf_adapter(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let exe = ctx.proc_meta.exe_name.as_str();
    let title_lower = ctx.window_title.to_ascii_lowercase();
    if !PDF_EXES.contains(&exe) && !title_lower.contains(".pdf") {
        return None;
    }

    let mut out = AdapterOutput::new("pdf");
    if let Some(cmd_pdf) = find_extension_in_cmdline(&ctx.proc_meta.cmdline, ".pdf") {
        out.lines.push(format!("pdf_path: {cmd_pdf}"));
        if Path::new(&cmd_pdf).exists() {
            out.paths.push(cmd_pdf.clone());
        }
        out.badge = Path::new(&cmd_pdf)
            .file_name()
            .map(|s| s.to_string_lossy().to_string());
    } else {
        out.lines
            .push(format!("pdf_title: {}", ctx.window_title.trim()));
    }

    if let Some(page_hint) = find_pdf_page_via_uia(ctx.hwnd) {
        out.lines.push(format!("pdf_page: {page_hint}"));
    }
    Some(out)
}

fn find_pdf_page_via_uia(hwnd: isize) -> Option<String> {
    if hwnd == 0 {
        return None;
    }
    let automation = UIAutomation::new().ok()?;
    let root = automation
        .element_from_handle(uiautomation::types::Handle::from(hwnd))
        .ok()?;
    let edit_cond = automation
        .create_property_condition(
            UIProperty::ControlType,
            (ControlType::Edit as i32).into(),
            None,
        )
        .ok()?;
    let edits = root.find_all(TreeScope::Descendants, &edit_cond).ok()?;
    for ed in edits.into_iter().take(16) {
        let name = ed.get_name().unwrap_or_default().to_ascii_lowercase();
        if name.contains("page") || name.contains("页") {
            if let Ok(v) = ed.get_property_value(UIProperty::ValueValue) {
                let s = crate::pick::clean_uia_variant_string(&v.to_string());
                if !s.is_empty() && s.len() <= 24 {
                    return Some(s);
                }
            }
        }
    }
    None
}

fn try_chat_adapter(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let exe = ctx.proc_meta.exe_name.as_str();
    if !CHAT_EXES.contains(&exe) {
        return None;
    }
    let mut out = AdapterOutput::new("chat");
    out.lines
        .push(format!("chat_window: {}", ctx.window_title.trim()));

    if let Some((cx, cy)) = ctx.tap_point {
        if let Some(siblings) = sample_surrounding_chat_messages(cx, cy, 5) {
            if !siblings.is_empty() {
                out.lines.push(format!(
                    "chat_context_messages (±5):\n{}",
                    siblings.join("\n")
                ));
            }
        }
    }
    out.badge = Some(ctx.window_title.trim().to_string());
    Some(out)
}

fn sample_surrounding_chat_messages(x: i32, y: i32, radius: usize) -> Option<Vec<String>> {
    let automation = UIAutomation::new().ok()?;
    let walker = automation.get_control_view_walker().ok()?;
    let hit = automation
        .element_from_point(uiautomation::types::Point::new(x, y))
        .ok()?;

    // Walk up up to 6 levels to find ListItem whose parent is List
    let mut curr = hit;
    let mut list_item = None;
    for _ in 0..6 {
        if let Ok(ct) = curr.get_control_type() {
            if ct == ControlType::ListItem {
                list_item = Some(curr);
                break;
            }
        }
        let Ok(parent) = walker.get_parent(&curr) else {
            break;
        };
        curr = parent;
    }

    let target_item = list_item?;
    let mut before = Vec::new();
    let mut p = target_item.clone();
    for _ in 0..radius {
        let Ok(prev) = walker.get_previous_sibling(&p) else {
            break;
        };
        let txt = prev.get_name().unwrap_or_default().trim().to_string();
        if !txt.is_empty() {
            before.push(format!("  - {txt}"));
        }
        p = prev;
    }
    before.reverse();

    let center_txt = target_item
        .get_name()
        .unwrap_or_default()
        .trim()
        .to_string();
    if !center_txt.is_empty() {
        before.push(format!("  * [当前选中] {center_txt}"));
    }

    let mut n = target_item;
    for _ in 0..radius {
        let Ok(next) = walker.get_next_sibling(&n) else {
            break;
        };
        let txt = next.get_name().unwrap_or_default().trim().to_string();
        if !txt.is_empty() {
            before.push(format!("  - {txt}"));
        }
        n = next;
    }

    Some(before)
}

fn try_outlook_adapter(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let exe = ctx.proc_meta.exe_name.as_str();
    if exe != "outlook.exe" && exe != "olk.exe" {
        return None;
    }
    let mut out = AdapterOutput::new("outlook");
    if exe == "outlook.exe" {
        let script = r#"
try {
  $ol = [Runtime.InteropServices.Marshal]::GetActiveObject('Outlook.Application')
  $item = $null
  if ($null -ne $ol.ActiveInspector()) { $item = $ol.ActiveInspector().CurrentItem }
  elseif ($null -ne $ol.ActiveExplorer() -and $ol.ActiveExplorer().Selection.Count -gt 0) { $item = $ol.ActiveExplorer().Selection.Item(1) }
  if ($null -ne $item) {
    $body = [string]$item.Body
    if ($body.Length -gt 800) { $body = $body.Substring(0, 800) }
    @{ subject = [string]$item.Subject; sender = [string]$item.SenderName; received = [string]$item.ReceivedTime; body = $body } | ConvertTo-Json -Compress
  }
} catch {}
"#;
        let resp = ctx.broker.call(BrokerRequest::RunPs51 {
            script: script.to_string(),
            timeout_ms: 1200,
        });
        if resp.ok && !resp.text.trim().is_empty() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp.text) {
                let subj = v.get("subject").and_then(|s| s.as_str()).unwrap_or("");
                let sender = v.get("sender").and_then(|s| s.as_str()).unwrap_or("");
                let recv = v.get("received").and_then(|s| s.as_str()).unwrap_or("");
                let body = v.get("body").and_then(|s| s.as_str()).unwrap_or("");
                if !subj.is_empty() {
                    out.badge = Some(subj.to_string());
                    out.lines.push(format!(
                        "outlook_mail: subject=\"{subj}\" from=\"{sender}\" at=\"{recv}\""
                    ));
                }
                if !body.trim().is_empty() {
                    out.lines
                        .push(format!("outlook_body_excerpt:\n{}", body.trim()));
                }
            }
        }
    }
    if out.lines.is_empty() {
        out.lines
            .push(format!("outlook_window: {}", ctx.window_title.trim()));
    }
    Some(out)
}

fn try_notes_adapter(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let exe = ctx.proc_meta.exe_name.as_str();
    if !matches!(
        exe,
        "obsidian.exe" | "onenote.exe" | "notion.exe" | "typora.exe"
    ) {
        return None;
    }

    let mut out = AdapterOutput::new("notes");
    if exe == "obsidian.exe" {
        // Obsidian title: "NoteName - VaultName - Obsidian v1.x.x"
        let parts: Vec<&str> = ctx.window_title.split(" - ").map(|s| s.trim()).collect();
        if parts.len() >= 3 {
            let note_name = parts[0];
            let vault_name = parts[parts.len() - 2];
            out.lines
                .push(format!("obsidian_note: {note_name} (vault: {vault_name})"));
            out.badge = Some(format!("{note_name}.md"));
            if let Some(vault_path) = resolve_obsidian_vault_path(vault_name) {
                out.lines
                    .push(format!("obsidian_vault_root: {}", vault_path.display()));
                out.paths.push(vault_path.to_string_lossy().to_string());
                let md_cand = vault_path.join(format!("{note_name}.md"));
                if md_cand.exists() {
                    out.paths.push(md_cand.to_string_lossy().to_string());
                }
            }
        }
    } else if exe == "typora.exe" {
        if let Some(md_path) = find_extension_in_cmdline(&ctx.proc_meta.cmdline, ".md") {
            out.lines.push(format!("typora_markdown_path: {md_path}"));
            if Path::new(&md_path).exists() {
                out.paths.push(md_path);
            }
        } else {
            out.lines
                .push(format!("typora_title: {}", ctx.window_title.trim()));
        }
    } else {
        out.lines
            .push(format!("notes_page_title: {}", ctx.window_title.trim()));
    }

    if out.lines.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn resolve_obsidian_vault_path(vault_name: &str) -> Option<PathBuf> {
    let cfg_dir = dirs::config_dir()?;
    let raw = fs::read_to_string(cfg_dir.join("obsidian").join("obsidian.json")).ok()?;
    parse_obsidian_vault_json(&raw, vault_name)
}

pub fn parse_obsidian_vault_json(raw_json: &str, vault_name: &str) -> Option<PathBuf> {
    let val: serde_json::Value = serde_json::from_str(raw_json).ok()?;
    let vaults = val.get("vaults")?.as_object()?;
    let target = vault_name.trim().to_ascii_lowercase();
    for (_id, entry) in vaults {
        if let Some(p_str) = entry.get("path").and_then(|p| p.as_str()) {
            let p = PathBuf::from(p_str);
            let matched = p
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase() == target)
                .unwrap_or(false);
            if matched {
                return Some(p);
            }
        }
    }
    None
}

fn try_cad_adapter(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let exe = ctx.proc_meta.exe_name.as_str();
    if exe != "sldworks.exe" && exe != "acad.exe" {
        return None;
    }

    let script = if exe == "sldworks.exe" {
        r#"
try {
  $sw = [Runtime.InteropServices.Marshal]::GetActiveObject('SldWorks.Application')
  $doc = $sw.ActiveDoc
  if ($null -ne $doc) {
    @{ path = [string]$doc.GetPathName(); title = [string]$doc.GetTitle() } | ConvertTo-Json -Compress
  }
} catch {}
"#
    } else {
        r#"
try {
  $ac = [Runtime.InteropServices.Marshal]::GetActiveObject('AutoCAD.Application')
  $doc = $ac.ActiveDocument
  if ($null -ne $doc) {
    @{ path = [string]$doc.FullName; title = [string]$doc.Name } | ConvertTo-Json -Compress
  }
} catch {}
"#
    };

    let mut out = AdapterOutput::new("cad");
    let resp = ctx.broker.call(BrokerRequest::RunPs51 {
        script: script.to_string(),
        timeout_ms: 1200,
    });
    if resp.ok && !resp.text.trim().is_empty() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp.text) {
            let p = v.get("path").and_then(|s| s.as_str()).unwrap_or("");
            let t = v.get("title").and_then(|s| s.as_str()).unwrap_or("");
            if !p.is_empty() {
                out.lines.push(format!("cad_model_path: {p}"));
                if Path::new(p).exists() {
                    out.paths.push(p.to_string());
                }
            }
            if !t.is_empty() {
                out.badge = Some(t.to_string());
                out.lines.push(format!("cad_document_title: {t}"));
            }
        }
    }
    if out.lines.is_empty() {
        out.lines
            .push(format!("cad_window: {}", ctx.window_title.trim()));
    }
    Some(out)
}

fn try_engine_adapter(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let exe = ctx.proc_meta.exe_name.as_str();
    if !matches!(
        exe,
        "unity.exe" | "unrealeditor.exe" | "ue4editor.exe" | "blender.exe"
    ) {
        return None;
    }

    let mut out = AdapterOutput::new("engine");
    if exe == "unity.exe" {
        if let Some(proj) = extract_flag_value(&ctx.proc_meta.cmdline, "-projectpath") {
            out.lines.push(format!("unity_project_path: {proj}"));
            if Path::new(&proj).exists() {
                out.paths.push(proj);
            }
        }
    } else if exe.contains("unreal") || exe.contains("ue4") {
        if let Some(uproj) = find_extension_in_cmdline(&ctx.proc_meta.cmdline, ".uproject") {
            out.lines.push(format!("unreal_uproject: {uproj}"));
            if Path::new(&uproj).exists() {
                out.paths.push(uproj);
            }
        }
    } else if exe == "blender.exe" {
        if let Some(blend) = find_extension_in_cmdline(&ctx.proc_meta.cmdline, ".blend") {
            out.lines.push(format!("blender_file: {blend}"));
            if Path::new(&blend).exists() {
                out.paths.push(blend);
            }
        }
    }
    out.lines
        .push(format!("engine_scene_title: {}", ctx.window_title.trim()));
    Some(out)
}

fn try_dev_design_adapter(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let exe = ctx.proc_meta.exe_name.as_str();
    if !matches!(
        exe,
        "figma.exe"
            | "photoshop.exe"
            | "githubdesktop.exe"
            | "sourcetree.exe"
            | "postman.exe"
            | "dbeaver.exe"
            | "navicat.exe"
    ) {
        return None;
    }
    let mut out = AdapterOutput::new("devtools");
    out.lines.push(format!(
        "dev_design_context: app={} title=\"{}\"",
        exe,
        ctx.window_title.trim()
    ));
    out.badge = Some(ctx.window_title.trim().to_string());
    Some(out)
}

fn try_media_adapter(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let exe = ctx.proc_meta.exe_name.as_str();
    if !matches!(
        exe,
        "potplayer64.exe" | "potplayermini64.exe" | "potplayer.exe" | "vlc.exe" | "mpv.exe"
    ) {
        return None;
    }

    let mut out = AdapterOutput::new("media");
    if let Some(p) = extract_path_from_cmdline(&ctx.proc_meta.cmdline, exe) {
        out.lines.push(format!("media_file_path: {p}"));
        if Path::new(&p).exists() {
            out.paths.push(p.clone());
        }
        out.badge = Path::new(&p)
            .file_name()
            .map(|s| s.to_string_lossy().to_string());
    }
    out.lines
        .push(format!("media_window_title: {}", ctx.window_title.trim()));
    Some(out)
}

fn try_sys_tools_adapter(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let exe = ctx.proc_meta.exe_name.as_str();
    match exe {
        "regedit.exe" => {
            let mut out = AdapterOutput::new("regedit");
            if let Some(key_path) = find_regedit_address_via_uia(ctx.hwnd) {
                out.lines.push(format!("registry_key_path: {key_path}"));
                out.badge = Some(key_path);
            }
            Some(out)
        }
        "steam.exe" | "steamwebhelper.exe" => {
            let mut out = AdapterOutput::new("steam");
            out.lines
                .push(format!("steam_window: {}", ctx.window_title.trim()));
            let steam_dir = Path::new(&ctx.proc_meta.install_dir);
            let vdf_candidates = [
                steam_dir.join("steamapps").join("libraryfolders.vdf"),
                steam_dir
                    .parent()
                    .and_then(|p| p.parent())
                    .map(|p| p.join("steamapps").join("libraryfolders.vdf"))
                    .unwrap_or_default(),
            ];
            for vdf in vdf_candidates {
                if vdf.exists() {
                    if let Ok(raw) = fs::read_to_string(&vdf) {
                        let libs = parse_steam_library_folders_vdf(&raw);
                        if !libs.is_empty() {
                            out.lines
                                .push(format!("steam_library_folders: {}", libs.join(" | ")));
                        }
                    }
                    break;
                }
            }
            Some(out)
        }
        "taskmgr.exe" | "procexp64.exe" | "procexp.exe" | "everything.exe" | "everything64.exe"
        | "totalcmd64.exe" | "totalcmd.exe" | "dopus.exe" | "bcompare.exe" | "winmergeu.exe"
        | "docker desktop.exe" => {
            let mut out = AdapterOutput::new("systools");
            out.lines.push(format!(
                "tool_context: app={} window=\"{}\"",
                exe,
                ctx.window_title.trim()
            ));
            Some(out)
        }
        _ => None,
    }
}

fn find_regedit_address_via_uia(hwnd: isize) -> Option<String> {
    if hwnd == 0 {
        return None;
    }
    let automation = UIAutomation::new().ok()?;
    let root = automation
        .element_from_handle(uiautomation::types::Handle::from(hwnd))
        .ok()?;
    let edit_cond = automation
        .create_property_condition(
            UIProperty::ControlType,
            (ControlType::Edit as i32).into(),
            None,
        )
        .ok()?;
    let edits = root.find_all(TreeScope::Descendants, &edit_cond).ok()?;
    for ed in edits.into_iter().take(8) {
        if let Ok(v) = ed.get_property_value(UIProperty::ValueValue) {
            let s = crate::pick::clean_uia_variant_string(&v.to_string());
            if s.contains("HKEY_") || s.starts_with("计算机\\") || s.starts_with("Computer\\") {
                return Some(s);
            }
        }
    }
    None
}

pub fn extract_flag_value(cmdline: &str, flag_lower: &str) -> Option<String> {
    let lower = cmdline.to_ascii_lowercase();
    let idx = lower.find(flag_lower)?;
    let rest = cmdline[idx + flag_lower.len()..].trim_start();
    if let Some(quoted) = rest.strip_prefix('"') {
        let end = quoted.find('"')?;
        Some(quoted[..end].to_string())
    } else {
        let tok = rest.split_whitespace().next()?;
        Some(tok.to_string())
    }
}

pub fn find_extension_in_cmdline(cmdline: &str, ext_lower: &str) -> Option<String> {
    // Scan quoted tokens first, then whitespace tokens
    let mut in_quotes = false;
    let mut current = String::new();
    let mut tokens = Vec::new();
    for ch in cmdline.chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
            if !in_quotes && !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else if ch.is_whitespace() && !in_quotes {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
        .into_iter()
        .find(|t| t.to_ascii_lowercase().ends_with(ext_lower))
}

pub fn parse_steam_library_folders_vdf(raw_vdf: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in raw_vdf.lines() {
        let trimmed = line.trim();
        // Match: "path"		"D:\\SteamLibrary"
        let parts: Vec<&str> = trimmed.split('"').collect();
        if parts.len() >= 4 && parts[1].eq_ignore_ascii_case("path") {
            let unescaped = parts[3].replace("\\\\", "\\");
            if !unescaped.is_empty() && !out.contains(&unescaped) {
                out.push(unescaped);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_obsidian_vault_and_steam_vdf_and_unity_flag() {
        let obs =
            r#"{"vaults":{"123":{"path":"D:\\Knowledge\\MyVault","ts":1700000000,"open":true}}}"#;
        assert_eq!(
            parse_obsidian_vault_json(obs, "MyVault"),
            Some(PathBuf::from(r"D:\Knowledge\MyVault"))
        );

        let vdf = r#"
"libraryfolders"
{
    "0"
    {
        "path"		"C:\\Program Files (x86)\\Steam"
    }
    "1"
    {
        "path"		"D:\\Games\\SteamLibrary"
    }
}
"#;
        assert_eq!(
            parse_steam_library_folders_vdf(vdf),
            vec![
                r"C:\Program Files (x86)\Steam".to_string(),
                r"D:\Games\SteamLibrary".to_string()
            ]
        );

        let unity_cmd = r#""C:\Program Files\Unity\Editor\Unity.exe" -projectPath "D:\Projects\SpaceGame" -useHub"#;
        assert_eq!(
            extract_flag_value(unity_cmd, "-projectpath").as_deref(),
            Some(r"D:\Projects\SpaceGame")
        );

        let pdf_cmd = r#""C:\Program Files\SumatraPDF\SumatraPDF.exe" "D:\Papers\Attention Is All You Need.pdf""#;
        assert_eq!(
            find_extension_in_cmdline(pdf_cmd, ".pdf").as_deref(),
            Some(r"D:\Papers\Attention Is All You Need.pdf")
        );
    }
}
