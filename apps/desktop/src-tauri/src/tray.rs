use crate::config::AppConfig;
use crate::elevation::is_elevated;
use crate::state::StateEvent;
use crate::AppShared;
use anyhow::Result;
use std::os::windows::process::CommandExt;
use std::process::Command;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

const TASK_NAME: &str = "AnythingToPrompt";
const RUN_REG_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

pub struct TrayHandles {
    pub status_item: MenuItem<tauri::Wry>,
}

pub fn apply_autostart_setting(enable: bool) -> Result<()> {
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_string_lossy().to_string();

    if enable {
        if is_elevated() {
            let tr = format!("\"{exe_str}\"");
            let status = Command::new("schtasks.exe")
                .args([
                    "/Create", "/TN", TASK_NAME, "/TR", &tr, "/SC", "ONLOGON", "/RL", "HIGHEST",
                    "/F",
                ])
                .creation_flags(CREATE_NO_WINDOW.0)
                .status();
            if let Ok(st) = status {
                if st.success() {
                    // Clean up duplicate HKCU Run entry if any
                    let _ = Command::new("reg.exe")
                        .args(["delete", RUN_REG_KEY, "/v", TASK_NAME, "/f"])
                        .creation_flags(CREATE_NO_WINDOW.0)
                        .status();
                    return Ok(());
                }
            }
        }

        // Fallback: HKCU Run key
        let quoted = format!("\"{exe_str}\"");
        let _ = Command::new("reg.exe")
            .args([
                "add",
                RUN_REG_KEY,
                "/v",
                TASK_NAME,
                "/t",
                "REG_SZ",
                "/d",
                &quoted,
                "/f",
            ])
            .creation_flags(CREATE_NO_WINDOW.0)
            .status()?;
    } else {
        let _ = Command::new("schtasks.exe")
            .args(["/Delete", "/TN", TASK_NAME, "/F"])
            .creation_flags(CREATE_NO_WINDOW.0)
            .status();
        let _ = Command::new("reg.exe")
            .args(["delete", RUN_REG_KEY, "/v", TASK_NAME, "/f"])
            .creation_flags(CREATE_NO_WINDOW.0)
            .status();
    }
    Ok(())
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<TrayHandles> {
    let cfg = app
        .state::<AppShared>()
        .config
        .lock()
        .map(|c| c.clone())
        .unwrap_or_default();

    let toggle_item = MenuItem::with_id(
        app,
        "toggle",
        "显示 / 隐藏 Anything-to-Prompt (双击 Ctrl 唤出 · 单击退出 · 长按选区)",
        true,
        None::<&str>,
    )?;
    let status_item = MenuItem::with_id(app, "status", "DSH 状态：正在检测…", false, None::<&str>)?;
    let reconnect_item = MenuItem::with_id(app, "reconnect", "重新连接 DSH", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;

    let autostart_item = CheckMenuItem::with_id(
        app,
        "autostart",
        "开机自动启动",
        true,
        cfg.autostart,
        None::<&str>,
    )?;
    let admin_item = CheckMenuItem::with_id(
        app,
        "prefer_admin",
        "优先以管理员权限启动",
        true,
        cfg.prefer_admin,
        None::<&str>,
    )?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    let open_cfg_item = MenuItem::with_id(app, "open_config", "打开配置文件", true, None::<&str>)?;
    let open_cap_item =
        MenuItem::with_id(app, "open_captures", "打开调试截图目录", true, None::<&str>)?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出 Anything-to-Prompt", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &toggle_item,
            &status_item,
            &reconnect_item,
            &sep1,
            &autostart_item,
            &admin_item,
            &sep2,
            &open_cfg_item,
            &open_cap_item,
            &sep3,
            &quit_item,
        ],
    )?;

    let autostart_clone = autostart_item.clone();
    let admin_clone = admin_item.clone();

    let mut builder = TrayIconBuilder::with_id("dsb-tray")
        .menu(&menu)
        .tooltip("Anything-to-Prompt")
        .show_menu_on_left_click(false)
        .on_menu_event(move |app_handle, event| match event.id().as_ref() {
            "toggle" => {
                crate::dispatch_state_event(app_handle, StateEvent::TrayToggle);
            }
            "reconnect" => {
                crate::trigger_dsh_reconnect(app_handle.clone());
            }
            "autostart" => {
                let checked = autostart_clone.is_checked().unwrap_or(false);
                if let Ok(mut c) = app_handle.state::<AppShared>().config.lock() {
                    c.autostart = checked;
                    let _ = c.save();
                }
                if let Err(e) = apply_autostart_setting(checked) {
                    log::warn!("apply_autostart_setting failed: {e}");
                }
            }
            "prefer_admin" => {
                let checked = admin_clone.is_checked().unwrap_or(true);
                if let Ok(mut c) = app_handle.state::<AppShared>().config.lock() {
                    c.prefer_admin = checked;
                    let _ = c.save();
                }
            }
            "open_config" => {
                let p = AppConfig::config_path();
                if let Some(parent) = p.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = Command::new("explorer.exe").arg(&p).spawn();
            }
            "open_captures" => {
                let dir = AppConfig::captures_dir();
                let _ = std::fs::create_dir_all(&dir);
                let _ = Command::new("explorer.exe").arg(&dir).spawn();
            }
            "quit" => {
                crate::cursor::restore_system_cursors();
                app_handle.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if crate::was_recently_hidden_by_outside_click(app, 350) {
                    return;
                }
                crate::dispatch_state_event(app, StateEvent::TrayToggle);
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    let _tray = builder.build(app)?;
    Ok(TrayHandles { status_item })
}

pub fn refresh_tray_status(app: &AppHandle) {
    let Some(shared) = app.try_state::<AppShared>() else {
        return;
    };
    let text = shared
        .dsh_conn
        .lock()
        .map(|s| {
            if s.status_label.is_empty() {
                "DSH 状态：正在检测…".to_string()
            } else {
                format!("DSH 状态：{}", s.status_label)
            }
        })
        .unwrap_or_else(|_| "DSH 状态：未知".to_string());

    if let Ok(guard) = shared.tray_handles.lock() {
        if let Some(handles) = guard.as_ref() {
            let _ = handles.status_item.set_text(text);
        }
    };
}
