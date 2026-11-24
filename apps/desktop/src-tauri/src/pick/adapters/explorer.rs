use crate::broker::BrokerRequest;
use crate::pick::adapters::{AdapterContext, AdapterOutput};
use std::path::Path;

pub fn matches(ctx: &AdapterContext) -> bool {
    ctx.proc_meta.exe_name == "explorer.exe"
        || matches!(
            ctx.window_class.as_str(),
            "CabinetWClass" | "ExploreWClass" | "Progman" | "WorkerW"
        )
}

pub fn run(ctx: &AdapterContext) -> Option<AdapterOutput> {
    let mut out = AdapterOutput::new("explorer");

    // 1. Query current folder path of the Explorer window (or Desktop)
    let folder_resp = ctx.broker.call(BrokerRequest::ExplorerFolder {
        hwnd: ctx.hwnd,
        class_name: ctx.window_class.clone(),
    });
    let current_folder = if folder_resp.ok && !folder_resp.text.trim().is_empty() {
        let f = folder_resp.text.trim().to_string();
        out.lines.push(format!("explorer_folder: {f}"));
        Some(f)
    } else {
        None
    };

    // 2. If user tapped a specific ListItem / file item, resolve `<current_folder>\<item.name>` first!
    if let (Some(folder), Some(el)) = (&current_folder, &ctx.primary_element) {
        let item_name = el.name.trim();
        if !item_name.is_empty() && !item_name.contains('\\') && !item_name.contains('/') {
            let candidate = Path::new(folder).join(item_name);
            if candidate.exists() {
                let p_str = candidate.to_string_lossy().to_string();
                out.lines.push(format!("clicked_explorer_item: {p_str}"));
                out.badge = Some(item_name.to_string());
                out.paths.push(p_str);
            }
        }
    }

    // 3. Also include currently selected items in that Explorer / Desktop view
    let sel_resp = ctx.broker.call(BrokerRequest::ShellSelection {
        hwnd: ctx.hwnd,
        class_name: ctx.window_class.clone(),
    });
    if sel_resp.ok && !sel_resp.paths.is_empty() {
        out.lines
            .push(format!("explorer_selected_count: {}", sel_resp.paths.len()));
        for p in sel_resp.paths.into_iter().take(12) {
            if !out.paths.contains(&p) {
                out.paths.push(p);
            }
        }
    }

    if out.badge.is_none() {
        out.badge = current_folder
            .as_deref()
            .and_then(|f| Path::new(f).file_name())
            .map(|n| n.to_string_lossy().to_string());
    }

    if out.lines.is_empty() && out.paths.is_empty() {
        None
    } else {
        Some(out)
    }
}
