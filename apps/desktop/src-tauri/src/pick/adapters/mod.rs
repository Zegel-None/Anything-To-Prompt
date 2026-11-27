pub mod browser;
pub mod explorer;
pub mod ide;
pub mod office;
pub mod solidworks;
pub mod solidworks_com;
pub mod terminal;
pub mod wave_b;

use crate::broker::BrokerClient;
use crate::pick::process::ProcessMetadata;
use crate::pick::UiaElementInfo;
use std::sync::{mpsc, Arc};
use std::time::Duration;

#[derive(Clone)]
pub struct AdapterContext {
    pub hwnd: isize,
    pub window_title: String,
    pub window_class: String,
    pub proc_meta: ProcessMetadata,
    pub primary_element: Option<UiaElementInfo>,
    pub tap_point: Option<(i32, i32)>,
    pub broker: Arc<BrokerClient>,
}

#[derive(Debug, Clone)]
pub struct AdapterOutput {
    pub adapter: String,
    pub badge: Option<String>,
    pub lines: Vec<String>,
    pub paths: Vec<String>,
}

impl AdapterOutput {
    pub fn new(adapter: impl Into<String>) -> Self {
        Self {
            adapter: adapter.into(),
            badge: None,
            lines: Vec::new(),
            paths: Vec::new(),
        }
    }
}

/// Runs all enabled built-in Wave A / Wave B adapters AND matching external adapter plugins
/// within `timeout_ms` budget (default 1500ms).
pub fn collect_deep_adapter_outputs(ctx: AdapterContext, timeout_ms: u64) -> Vec<AdapterOutput> {
    let (tx, rx) = mpsc::channel::<Vec<AdapterOutput>>();

    if std::thread::Builder::new()
        .name("dsb-adapters".to_string())
        .spawn(move || {
            use crate::pick::adapter_plugins;
            let mut list = Vec::new();
            let overridden_by_external =
                adapter_plugins::has_enabled_external_override_for_exe(&ctx.proc_meta.exe_name);

            if !overridden_by_external {
                if adapter_plugins::is_builtin_adapter_enabled("explorer") && explorer::matches(&ctx)
                {
                    if let Some(o) = explorer::run(&ctx) {
                        list.push(o);
                    }
                }
                if adapter_plugins::is_builtin_adapter_enabled("browser") && browser::matches(&ctx) {
                    if let Some(o) = browser::run(&ctx) {
                        list.push(o);
                    }
                }
                if adapter_plugins::is_builtin_adapter_enabled("ide") && ide::matches(&ctx) {
                    if let Some(o) = ide::run(&ctx) {
                        list.push(o);
                    }
                }
                if adapter_plugins::is_builtin_adapter_enabled("terminal") && terminal::matches(&ctx)
                {
                    if let Some(o) = terminal::run(&ctx) {
                        list.push(o);
                    }
                }
                if adapter_plugins::is_builtin_adapter_enabled("office") && office::matches(&ctx) {
                    if let Some(o) = office::run(&ctx) {
                        list.push(o);
                    }
                }
                if adapter_plugins::is_builtin_adapter_enabled("solidworks")
                    && solidworks::matches(&ctx)
                {
                    if let Some(o) = solidworks::run(&ctx) {
                        list.push(o);
                    }
                }

                let wb = wave_b::run_all_wave_b(&ctx);
                list.extend(wb);
            }

            let ext_outputs = adapter_plugins::run_external_plugins_for_context(&ctx, timeout_ms);
            list.extend(ext_outputs);
            let _ = tx.send(list);
        })
        .is_err()
    {
        return Vec::new();
    }

    match rx.recv_timeout(Duration::from_millis(timeout_ms.max(300))) {
        Ok(outputs) => outputs,
        Err(_) => {
            log::warn!("Deep app adapter enrichment timed out after {timeout_ms}ms");
            Vec::new()
        }
    }
}
