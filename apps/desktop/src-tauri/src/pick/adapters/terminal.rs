use crate::pick::adapters::{AdapterContext, AdapterOutput};
use crate::pick::process::{list_child_processes, query_wmi_cmdline, read_process_cwd};
use uiautomation::patterns::UITextPattern;
use uiautomation::types::{ControlType, TreeScope, UIProperty};
use uiautomation::UIAutomation;

const TERMINAL_EXES: &[&str] = &[
    "windowsterminal.exe",
    "wt.exe",
    "powershell.exe",
    "pwsh.exe",
    "cmd.exe",
    "alacritty.exe",
    "wezterm-gui.exe",
    "mintty.exe",
    "conhost.exe",
];

pub fn matches(ctx: &AdapterContext) -> bool {
    if TERMINAL_EXES.contains(&ctx.proc_meta.exe_name.as_str()) {
        return true;
    }
    // Also match integrated terminal inside IDEs when picked element or ancestor mentions terminal/xterm
    if let Some(el) = &ctx.primary_element {
        let joined = format!(
            "{} {} {} {}",
            el.name,
            el.automation_id,
            el.class_name,
            el.ancestors.join(" ")
        )
        .to_ascii_lowercase();
        if joined.contains("terminal") || joined.contains("xterm") {
            return true;
        }
    }
    false
}

pub fn run(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let mut out = AdapterOutput::new("terminal");

    // 1. Inspect child shell processes (up to 2 levels deep for WindowsTerminal -> OpenConsole/pwsh -> tool)
    let mut shells = list_child_processes(ctx.proc_meta.pid);
    let first_level_pids: Vec<u32> = shells.iter().map(|(pid, _)| *pid).collect();
    for child_pid in first_level_pids {
        for grand in list_child_processes(child_pid) {
            if shells.len() < 12 {
                shells.push(grand);
            }
        }
    }

    for (spid, sname) in shells.into_iter().take(8) {
        let lower = sname.to_ascii_lowercase();
        if lower == "openconsole.exe" || lower == "conhost.exe" {
            continue;
        }
        let scwd = read_process_cwd(spid);
        let scmd = query_wmi_cmdline(spid).unwrap_or_default();
        if let Some(cwd_path) = &scwd {
            if out.badge.is_none() {
                out.badge = Some(format!("{sname} @ {cwd_path}"));
            }
            out.paths.push(cwd_path.clone());
        }
        out.lines.push(format!(
            "terminal_child: pid={spid} exe={sname} cwd={} cmd={}",
            scwd.as_deref().unwrap_or("-"),
            if scmd.is_empty() { "-" } else { &scmd }
        ));
    }

    if let Some(own_cwd) = &ctx.proc_meta.cwd {
        if out.paths.is_empty() {
            out.paths.push(own_cwd.clone());
        }
    }

    // 2. Extract last 80 lines of visible terminal buffer via UIA TextPattern / Document
    let tail_text = extract_terminal_tail_via_uia(ctx.hwnd, 80).or_else(|| {
        ctx.primary_element
            .as_ref()
            .filter(|e| !e.text_snippet.trim().is_empty())
            .map(|e| take_last_n_lines(&e.text_snippet, 80))
    });

    if let Some(tail) = tail_text {
        if !tail.trim().is_empty() {
            out.lines.push(format!("terminal_last_80_lines:\n{tail}"));
        }
    }

    if out.lines.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn extract_terminal_tail_via_uia(hwnd: isize, max_lines: usize) -> Option<String> {
    if hwnd == 0 {
        return None;
    }
    let automation = UIAutomation::new().ok()?;
    let root = automation
        .element_from_handle(uiautomation::types::Handle::from(hwnd))
        .ok()?;

    if let Ok(pat) = root.get_pattern::<UITextPattern>() {
        if let Ok(range) = pat.get_document_range() {
            if let Ok(txt) = range.get_text(32768) {
                let tail = take_last_n_lines(&txt, max_lines);
                if !tail.trim().is_empty() {
                    return Some(tail);
                }
            }
        }
    }

    let doc_cond = automation
        .create_property_condition(
            UIProperty::ControlType,
            (ControlType::Document as i32).into(),
            None,
        )
        .ok()?;
    let docs = root.find_all(TreeScope::Descendants, &doc_cond).ok()?;
    for doc in docs.into_iter().take(6) {
        if let Ok(pat) = doc.get_pattern::<UITextPattern>() {
            if let Ok(range) = pat.get_document_range() {
                if let Ok(txt) = range.get_text(32768) {
                    let tail = take_last_n_lines(&txt, max_lines);
                    if !tail.trim().is_empty() {
                        return Some(tail);
                    }
                }
            }
        }
    }
    None
}

pub fn take_last_n_lines(raw: &str, max_lines: usize) -> String {
    let non_empty_trimmed: Vec<&str> = raw.lines().map(|l| l.trim_end()).collect();
    // Drop trailing blank padding lines common in terminal screen buffers
    let last_content_idx = non_empty_trimmed
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map(|i| i + 1)
        .unwrap_or(0);
    let slice = &non_empty_trimmed[..last_content_idx];
    let start = slice.len().saturating_sub(max_lines);
    slice[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_trailing_blank_buffer_rows_and_keeps_last_n() {
        let buf = "line1\nline2\nline3\n   \n\n";
        assert_eq!(take_last_n_lines(buf, 2), "line2\nline3");
    }
}
