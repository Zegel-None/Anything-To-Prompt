use crate::pick::adapters::{AdapterContext, AdapterOutput};
use crate::pick::process::{find_listening_pid_for_tcp_port, inspect_process};
use std::collections::HashMap;
use uiautomation::types::{ControlType, TreeScope, UIProperty};
use uiautomation::UIAutomation;
use url::Url;

const BROWSER_EXES: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "vivaldi.exe",
    "opera.exe",
    "arc.exe",
    "waterfox.exe",
    "zen.exe",
];

pub fn matches(ctx: &AdapterContext) -> bool {
    BROWSER_EXES.contains(&ctx.proc_meta.exe_name.as_str())
}

pub fn run(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let mut out = AdapterOutput::new("browser");
    let raw_url = find_browser_url_via_uia(ctx.hwnd).or_else(|| extract_url_like_from_element(ctx));

    if let Some(u) = raw_url {
        let normalized = normalize_browser_url(&u);
        out.lines.push(format!("page_url: {normalized}"));

        if let Some((host, port)) = extract_loopback_port(&normalized) {
            if let Some(srv_pid) = find_listening_pid_for_tcp_port(port) {
                let srv_meta = inspect_process(srv_pid, &HashMap::new());
                out.badge = Some(format!("{host}:{port} -> {}", srv_meta.exe_name));
                out.lines.push(format!(
                    "localhost_server: pid={} exe={}",
                    srv_pid, srv_meta.exe_path
                ));
                if !srv_meta.cmdline.is_empty() {
                    out.lines
                        .push(format!("localhost_cmdline: {}", srv_meta.cmdline));
                }
                if let Some(cwd) = srv_meta.cwd {
                    out.lines.push(format!("localhost_cwd: {cwd}"));
                    out.paths.push(cwd);
                }
            } else {
                out.badge = Some(format!("{host}:{port}"));
            }
        } else if let Ok(parsed) = Url::parse(&normalized) {
            if parsed.scheme() == "file" {
                if let Ok(local_path) = parsed.to_file_path() {
                    let p_str = local_path.to_string_lossy().to_string();
                    out.lines.push(format!("local_file_url_path: {p_str}"));
                    out.paths.push(p_str);
                }
            }
            if out.badge.is_none() {
                out.badge = parsed.host_str().map(|h| h.to_string());
            }
        }
    }

    if out.lines.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn extract_url_like_from_element(ctx: &AdapterContext) -> Option<String> {
    if let Some(el) = &ctx.primary_element {
        for cand in [&el.value, &el.name] {
            let t = cand.trim();
            if t.starts_with("http://")
                || t.starts_with("https://")
                || t.starts_with("file:///")
                || t.starts_with("localhost:")
                || t.starts_with("127.0.0.1:")
            {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn find_browser_url_via_uia(hwnd: isize) -> Option<String> {
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

    for edit in edits.into_iter().take(24) {
        let name = edit.get_name().unwrap_or_default().to_ascii_lowercase();
        let aid = edit
            .get_automation_id()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let looks_like_addr = name.contains("address")
            || name.contains("地址")
            || name.contains("search or enter")
            || aid.contains("address")
            || aid.contains("urlbar")
            || aid.contains("omnibox");

        if let Ok(val_var) = edit.get_property_value(UIProperty::ValueValue) {
            let val = crate::pick::clean_uia_variant_string(&val_var.to_string());
            if !val.is_empty()
                && (looks_like_addr
                    || val.starts_with("http://")
                    || val.starts_with("https://")
                    || val.starts_with("file:///")
                    || val.starts_with("localhost:"))
            {
                return Some(val);
            }
        }
    }
    None
}

pub fn normalize_browser_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.contains("://") {
        return trimmed.to_string();
    }
    if trimmed.starts_with("localhost:")
        || trimmed.starts_with("127.0.0.1:")
        || trimmed.starts_with("[::1]:")
    {
        return format!("http://{trimmed}");
    }
    format!("https://{trimmed}")
}

pub fn extract_loopback_port(url_str: &str) -> Option<(String, u16)> {
    let parsed = Url::parse(url_str).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let is_local = matches!(
        host.as_str(),
        "localhost" | "127.0.0.1" | "::1" | "[::1]" | "0.0.0.0"
    ) || host.ends_with(".localhost");
    if !is_local {
        return None;
    }
    let port = parsed.port_or_known_default()?;
    Some((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_localhost_ports_and_normalizes_omnibox_urls() {
        let u1 = normalize_browser_url("localhost:5173/dashboard");
        assert_eq!(u1, "http://localhost:5173/dashboard");
        assert_eq!(
            extract_loopback_port(&u1),
            Some(("localhost".to_string(), 5173))
        );

        let u2 = normalize_browser_url("http://127.0.0.1:3080/?dsh_bar_clone=1");
        assert_eq!(
            extract_loopback_port(&u2),
            Some(("127.0.0.1".to_string(), 3080))
        );

        let ext = normalize_browser_url("github.com/deepseek-ai");
        assert_eq!(ext, "https://github.com/deepseek-ai");
        assert_eq!(extract_loopback_port(&ext), None);
    }
}
