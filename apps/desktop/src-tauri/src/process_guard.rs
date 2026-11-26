use std::collections::HashSet;
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupAction {
    Ignore,
    KeepSelfOrAncestor,
    KeepLiveSidecar,
    KeepTransientCli,
    KillResidualBar,
    KillResidualBroker,
    KillOrphanSidecar,
}

impl CleanupAction {
    pub fn should_kill(self) -> bool {
        matches!(
            self,
            Self::KillResidualBar | Self::KillResidualBroker | Self::KillOrphanSidecar
        )
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CleanupReport {
    pub killed_bar: usize,
    pub killed_broker: usize,
    pub killed_orphan_sidecar: usize,
    pub killed_webview: usize,
    pub kept_live_sidecar: usize,
}

impl CleanupReport {
    pub fn total_killed(&self) -> usize {
        self.killed_bar + self.killed_broker + self.killed_orphan_sidecar + self.killed_webview
    }
}

/// Pure classifier deciding whether another process should be terminated when the user
/// directly double-clicks `anything-to-prompt.exe` to start a fresh instance.
///
/// Rules:
/// - Never kill self or our immediate launcher ancestor chain (e.g., UAC relaunch parent).
/// - Preserve live `--mcp-sidecar` / `--antigravity-sidecar` whose parent IDE is still alive,
///   so active Antigravity / Claude Code / Codex MCP sessions are not disrupted; kill orphaned
///   sidecars whose parent IDE has already exited.
/// - Preserve transient one-shot CLI relays (`--hook-event`, `--install-plugins`, `--uninstall-plugins`).
/// - Kill all residual Bar instances and residual `--broker` helpers so stuck/old/trapped instances
///   release the single-instance mutex, named pipe, and WebView2 profile cleanly.
pub fn classify_candidate_process(
    pid: u32,
    self_pid: u32,
    ancestor_pids: &[u32],
    exe_name: &str,
    cmd_args: &[String],
    parent_alive: bool,
) -> CleanupAction {
    if pid == self_pid || ancestor_pids.contains(&pid) {
        return CleanupAction::KeepSelfOrAncestor;
    }
    if !exe_name.eq_ignore_ascii_case("anything-to-prompt.exe") {
        return CleanupAction::Ignore;
    }

    let flags: Vec<String> = cmd_args
        .iter()
        .skip(1)
        .map(|s| s.trim().to_ascii_lowercase())
        .collect();

    if flags
        .iter()
        .any(|f| f == "--mcp-sidecar" || f == "--antigravity-sidecar")
    {
        return if parent_alive {
            CleanupAction::KeepLiveSidecar
        } else {
            CleanupAction::KillOrphanSidecar
        };
    }

    if flags
        .iter()
        .any(|f| f == "--hook-event" || f == "--install-plugins" || f == "--uninstall-plugins")
    {
        return CleanupAction::KeepTransientCli;
    }

    if flags.iter().any(|f| f == "--broker") {
        return CleanupAction::KillResidualBroker;
    }

    CleanupAction::KillResidualBar
}

/// Enumerates and cleanly terminates residual `anything-to-prompt.exe` Bar/broker/orphan-sidecar
/// processes using `sysinfo`, waiting up to 1.5s for their OS handles, mutexes, and WebView2
/// children to release before the fresh instance initializes.
pub fn cleanup_stale_processes_on_direct_launch() -> CleanupReport {
    let mut report = CleanupReport::default();
    let self_pid_u32 = std::process::id();
    let self_pid = Pid::from_u32(self_pid_u32);

    let Ok(mut sys) = System::new() else {
        return report;
    };
    let refresh_kind = ProcessRefreshKind::nothing()
        .with_cmd(UpdateKind::Always)
        .with_exe(UpdateKind::Always);
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind);

    // Collect up to 4 ancestor PIDs so a UAC relaunch child never touches its own parent
    let mut ancestor_pids = Vec::new();
    let mut cursor = sys.process(self_pid).and_then(|p| p.parent());
    for _ in 0..4 {
        let Some(anc) = cursor else {
            break;
        };
        ancestor_pids.push(anc.as_u32());
        cursor = sys.process(anc).and_then(|p| p.parent());
    }

    let mut targets: Vec<(Pid, CleanupAction)> = Vec::new();
    for (&pid, proc_) in sys.processes() {
        let exe_name = proc_
            .exe()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| proc_.name().to_string_lossy().to_string());

        let cmd_args: Vec<String> = proc_
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect();

        let parent_alive = proc_
            .parent()
            .map(|ppid| sys.process(ppid).is_some())
            .unwrap_or(false);

        let action = classify_candidate_process(
            pid.as_u32(),
            self_pid_u32,
            &ancestor_pids,
            &exe_name,
            &cmd_args,
            parent_alive,
        );

        match action {
            CleanupAction::KeepLiveSidecar => {
                report.kept_live_sidecar += 1;
            }
            a if a.should_kill() => {
                targets.push((pid, a));
            }
            _ => {}
        }
    }

    if targets.is_empty() {
        return report;
    }

    let mut killed_bar_pids: HashSet<Pid> = HashSet::new();
    let mut killed_pids: Vec<Pid> = Vec::new();

    for (pid, action) in targets {
        if let Some(proc_) = sys.process(pid) {
            if proc_.kill() {
                killed_pids.push(pid);
                match action {
                    CleanupAction::KillResidualBar => {
                        killed_bar_pids.insert(pid);
                        report.killed_bar += 1;
                    }
                    CleanupAction::KillResidualBroker => {
                        report.killed_broker += 1;
                    }
                    CleanupAction::KillOrphanSidecar => {
                        report.killed_orphan_sidecar += 1;
                    }
                    _ => {}
                }
            }
        }
    }

    if killed_pids.is_empty() {
        return report;
    }

    // Wait up to 1.5s for terminated processes to exit and release mutexes/locks
    let deadline = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < deadline {
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&killed_pids),
            true,
            ProcessRefreshKind::nothing(),
        );
        if killed_pids.iter().all(|pid| sys.process(*pid).is_none()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
    }

    // If any Bar instance was terminated, ensure no lingering WebView2 browser children remain
    // holding locks on `%LOCALAPPDATA%\AnythingToPrompt\webview\...`.
    if !killed_bar_pids.is_empty() {
        std::thread::sleep(Duration::from_millis(180));
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        let mut wv_targets = Vec::new();
        for (&pid, proc_) in sys.processes() {
            let is_wv = proc_
                .name()
                .to_string_lossy()
                .eq_ignore_ascii_case("msedgewebview2.exe");
            if is_wv {
                if let Some(ppid) = proc_.parent() {
                    if killed_bar_pids.contains(&ppid) {
                        wv_targets.push(pid);
                    }
                }
            }
        }
        for pid in &wv_targets {
            if let Some(proc_) = sys.process(*pid) {
                if proc_.kill() {
                    report.killed_webview += 1;
                }
            }
        }
        if !wv_targets.is_empty() {
            let wv_deadline = Instant::now() + Duration::from_millis(600);
            while Instant::now() < wv_deadline {
                sys.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&wv_targets),
                    true,
                    ProcessRefreshKind::nothing(),
                );
                if wv_targets.iter().all(|pid| sys.process(*pid).is_none()) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn classifies_bar_broker_live_sidecar_and_orphan_sidecar_accurately() {
        // Self & ancestor are always protected
        assert_eq!(
            classify_candidate_process(100, 100, &[90], "anything-to-prompt.exe", &s(&["a.exe"]), true),
            CleanupAction::KeepSelfOrAncestor
        );
        assert_eq!(
            classify_candidate_process(90, 100, &[90], "anything-to-prompt.exe", &s(&["a.exe"]), true),
            CleanupAction::KeepSelfOrAncestor
        );

        // Unrelated processes are ignored
        assert_eq!(
            classify_candidate_process(200, 100, &[90], "explorer.exe", &s(&["explorer.exe"]), true),
            CleanupAction::Ignore
        );

        // Previous Bar instances (plain, elevated-relaunch, or ensure-running) are killed
        assert_eq!(
            classify_candidate_process(301, 100, &[90], "anything-to-prompt.exe", &s(&["anything-to-prompt.exe"]), true),
            CleanupAction::KillResidualBar
        );
        assert_eq!(
            classify_candidate_process(
                302,
                100,
                &[90],
                "Anything-To-Prompt.EXE",
                &s(&["anything-to-prompt.exe", "--ensure-running", "--launched-by", "antigravity"]),
                true
            ),
            CleanupAction::KillResidualBar
        );

        // Residual broker children are killed
        assert_eq!(
            classify_candidate_process(
                303,
                100,
                &[90],
                "anything-to-prompt.exe",
                &s(&["anything-to-prompt.exe", "--broker", r"\\.\pipe\atp-broker-1"]),
                false
            ),
            CleanupAction::KillResidualBroker
        );

        // Live sidecars owned by a running IDE are preserved; orphaned sidecars are killed
        assert_eq!(
            classify_candidate_process(
                401,
                100,
                &[90],
                "anything-to-prompt.exe",
                &s(&["anything-to-prompt.exe", "--mcp-sidecar", "--harness", "antigravity"]),
                true
            ),
            CleanupAction::KeepLiveSidecar
        );
        assert_eq!(
            classify_candidate_process(
                402,
                100,
                &[90],
                "anything-to-prompt.exe",
                &s(&["anything-to-prompt.exe", "--antigravity-sidecar"]),
                false
            ),
            CleanupAction::KillOrphanSidecar
        );

        // Transient hook relays are untouched
        assert_eq!(
            classify_candidate_process(
                501,
                100,
                &[90],
                "anything-to-prompt.exe",
                &s(&["anything-to-prompt.exe", "--hook-event", "antigravity"]),
                true
            ),
            CleanupAction::KeepTransientCli
        );
    }
}
