use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntryBrief {
    pub path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub size_complete: bool,
    pub modified_epoch_secs: Option<u64>,
    pub file_count: usize,
    pub dir_count: usize,
    pub children_preview: Vec<String>,
    pub git_root: Option<String>,
    pub git_branch: Option<String>,
}

pub fn inspect_path_metadata(path: &Path, budget_ms: u64) -> Option<FileEntryBrief> {
    let meta = fs::metadata(path).ok()?;
    let modified_epoch_secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    let (git_root, git_branch) = detect_git_repo(path);

    if !meta.is_dir() {
        return Some(FileEntryBrief {
            path: path.to_string_lossy().to_string(),
            is_dir: false,
            size_bytes: meta.len(),
            size_complete: true,
            modified_epoch_secs,
            file_count: 1,
            dir_count: 0,
            children_preview: Vec::new(),
            git_root,
            git_branch,
        });
    }

    let deadline = Instant::now() + Duration::from_millis(budget_ms.max(100));
    let mut children_preview = Vec::new();
    let mut total_bytes = 0u64;
    let mut file_count = 0usize;
    let mut dir_count = 0usize;
    let mut size_complete = true;

    let mut stack: Vec<PathBuf> = vec![path.to_path_buf()];
    let mut is_root = true;

    while let Some(curr) = stack.pop() {
        if Instant::now() >= deadline {
            size_complete = false;
            break;
        }
        let Ok(rd) = fs::read_dir(&curr) else {
            continue;
        };
        for entry in rd.flatten() {
            if Instant::now() >= deadline {
                size_complete = false;
                break;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if is_root && children_preview.len() < 30 {
                if ft.is_dir() {
                    children_preview.push(format!("{name}/"));
                } else {
                    children_preview.push(name.clone());
                }
            }
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                dir_count += 1;
                stack.push(entry.path());
            } else if ft.is_file() {
                file_count += 1;
                if let Ok(m) = entry.metadata() {
                    total_bytes = total_bytes.saturating_add(m.len());
                }
            }
        }
        is_root = false;
    }

    children_preview.sort();

    Some(FileEntryBrief {
        path: path.to_string_lossy().to_string(),
        is_dir: true,
        size_bytes: total_bytes,
        size_complete,
        modified_epoch_secs,
        file_count,
        dir_count,
        children_preview,
        git_root,
        git_branch,
    })
}

pub fn detect_git_repo(start: &Path) -> (Option<String>, Option<String>) {
    let mut cur = if start.is_file() {
        start.parent()
    } else {
        Some(start)
    };

    while let Some(dir) = cur {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            let head_path = dot_git.join("HEAD");
            let branch = fs::read_to_string(head_path)
                .ok()
                .map(|s| parse_git_head_content(&s));
            return (Some(dir.to_string_lossy().to_string()), branch);
        } else if dot_git.is_file() {
            // Worktree / submodule gitdir pointer
            if let Ok(raw) = fs::read_to_string(&dot_git) {
                if let Some(gitdir_rel) = raw.trim().strip_prefix("gitdir:") {
                    let gitdir = dir.join(gitdir_rel.trim());
                    let branch = fs::read_to_string(gitdir.join("HEAD"))
                        .ok()
                        .map(|s| parse_git_head_content(&s));
                    return (Some(dir.to_string_lossy().to_string()), branch);
                }
            }
        }
        cur = dir.parent();
    }

    (None, None)
}

pub fn parse_git_head_content(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("ref: refs/heads/") {
        rest.to_string()
    } else if trimmed.len() >= 8 {
        format!("detached@{}", &trimmed[..8])
    } else {
        trimmed.to_string()
    }
}

pub fn format_human_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.2} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_git_head_branch_and_detached_sha() {
        assert_eq!(parse_git_head_content("ref: refs/heads/main\n"), "main");
        assert_eq!(
            parse_git_head_content("ref: refs/heads/feat/bar-shell\r\n"),
            "feat/bar-shell"
        );
        assert_eq!(
            parse_git_head_content("a1b2c3d4e5f60718293a\n"),
            "detached@a1b2c3d4"
        );
    }
}
