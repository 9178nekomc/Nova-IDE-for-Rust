// NovaRS — Layered Sandbox & Permission Guard for AI Agent Actions
// Implements 3 permission tiers:
// 1. StrictStepByStep: Every tool execution and file patch requires explicit user confirmation.
// 2. WorkspaceBalanced: Read-only tools inside the workspace jail run automatically; writes & shell commands require approval.
// 3. YoloAutonomous: Autonomous read/write/check inside the workspace jail (still blocks path traversal `..` or system destruction).
//
// The jail predicate here is the single choke point for every MCP/AI file operation, so it is
// implemented lexically (no filesystem dependency) *and* re-validated against the canonical path
// when the target already exists — a symlink/junction inside the workspace can otherwise redirect
// a write outside the jail even though the lexical path looks clean.

use std::path::{Component, Path, PathBuf};

use crate::config::SandboxPermissionTier;

#[derive(Clone, Debug)]
pub struct PendingAgentAction {
    pub id: u64,
    pub tool_name: String,
    pub args: serde_json::Value,
    pub summary: String,
    pub risk_level: &'static str, // "LOW (Read)", "MEDIUM (Write/Edit)", "HIGH (Exec)"
}

/// 词法规范化：折叠 `.`、消解 `..`、统一分隔符，不触碰文件系统。
/// 若 `..` 试图逃出根目录则返回 `None`（视为非法路径）。
fn normalize_lexical(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    let mut depth = 0usize;
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => {
                out.push(p.as_os_str());
            }
            Component::RootDir => {
                out.push(Component::RootDir.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    // 已经在根/前缀之上，再往上就是越界
                    return None;
                }
                out.pop();
                depth -= 1;
            }
            Component::Normal(seg) => {
                out.push(seg);
                depth += 1;
            }
        }
    }
    Some(out)
}

/// 按路径分量比较（而非字符串前缀），因此在 Windows 上可正确区分大小写不敏感，
/// 并杜绝 `C:\ws` 误判 `C:\ws-evil` 这类"同前缀兄弟目录"逃逸。
fn path_within(child: &Path, parent: &Path) -> bool {
    let case_insensitive = cfg!(windows);
    let mut child_it = child.components();
    for pc in parent.components() {
        let Some(cc) = child_it.next() else { return false };
        let (mut a, mut b) = (cc.as_os_str().to_owned(), pc.as_os_str().to_owned());
        if case_insensitive {
            a = a.to_string_lossy().to_lowercase().into();
            b = b.to_string_lossy().to_lowercase().into();
        }
        if a != b {
            return false;
        }
    }
    true
}

/// 校验目标路径是否严格限制在当前工作区沙箱内（防止路径穿越与同前缀目录逃逸）
pub fn verify_workspace_jail(workspace: Option<&Path>, candidate: &Path) -> Result<PathBuf, String> {
    let Some(ws) = workspace else {
        return Err("未打开工作区文件夹，沙箱拒绝访问外部路径".to_string());
    };
    let Some(ws_norm) = normalize_lexical(ws) else {
        return Err(format!("工作区路径非法: {}", ws.display()));
    };
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        ws_norm.join(candidate)
    };
    let Some(target) = normalize_lexical(&joined) else {
        return Err(format!(
            "沙箱安全拦截：目标路径经 `..` 消解后越过文件系统根 ({})",
            joined.display()
        ));
    };
    if !path_within(&target, &ws_norm) {
        return Err(format!(
            "沙箱越界拦截：目标路径 `{}` 不在工作区 `{}` 内",
            target.display(),
            ws_norm.display()
        ));
    }
    // 目标已存在时，再走一次真实路径校验，阻断工作区内软链接/目录联接指向外部的逃逸
    if let Some(existing) = nearest_existing_ancestor(&target) {
        if let (Ok(canonical), Ok(ws_canonical)) = (existing.canonicalize(), ws_norm.canonicalize()) {
            if !path_within(&canonical, &ws_canonical) {
                return Err(format!(
                    "沙箱越界拦截：`{}` 通过链接指向工作区外的 `{}`",
                    target.display(),
                    canonical.display()
                ));
            }
        }
    }
    Ok(target)
}

/// 返回自身或最近的已存在祖先目录（用于对尚不存在的待创建文件做真实路径校验）
fn nearest_existing_ancestor(path: &Path) -> Option<&Path> {
    let mut cur = Some(path);
    while let Some(p) = cur {
        if p.exists() {
            return Some(p);
        }
        cur = p.parent();
    }
    None
}

/// 判断某个 MCP 工具调用是否为只读安全操作
pub fn is_readonly_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "read_file" | "list_files" | "search_code" | "get_diagnostics" | "get_ast_outline"
    )
}

/// 根据当前权限层级决定某个工具调用是否需要挂起等待用户点击「批准执行」
pub fn requires_user_confirmation(tier: SandboxPermissionTier, tool_name: &str) -> bool {
    match tier {
        SandboxPermissionTier::StrictStepByStep => true,
        SandboxPermissionTier::WorkspaceBalanced => !is_readonly_tool(tool_name),
        SandboxPermissionTier::YoloAutonomous => false,
    }
}

pub fn classify_tool_risk(tool_name: &str) -> &'static str {
    if is_readonly_tool(tool_name) {
        "🟢 只读安全 (Read-Only)"
    } else if matches!(tool_name, "write_file" | "apply_diff") {
        "🟡 代码修改 (Workspace Write)"
    } else {
        "🔴 命令执行 (Process Exec)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\work\demo")
        } else {
            PathBuf::from("/work/demo")
        }
    }

    #[test]
    fn allows_plain_relative_path() {
        let p = verify_workspace_jail(Some(&ws()), Path::new("src/main.rs")).unwrap();
        assert!(p.to_string_lossy().ends_with("main.rs"));
    }

    #[test]
    fn rejects_parent_traversal() {
        assert!(verify_workspace_jail(Some(&ws()), Path::new("../secret.txt")).is_err());
        assert!(verify_workspace_jail(Some(&ws()), Path::new("src/../../secret.txt")).is_err());
    }

    #[test]
    fn rejects_sibling_directory_with_shared_prefix() {
        // 这是旧实现的真实逃逸路径：字符串前缀比较会放行 `demo-evil`
        let sibling = if cfg!(windows) {
            Path::new(r"C:\work\demo-evil\pwn.rs")
        } else {
            Path::new("/work/demo-evil/pwn.rs")
        };
        assert!(verify_workspace_jail(Some(&ws()), sibling).is_err());
    }

    #[test]
    fn rejects_absolute_path_outside_workspace() {
        let outside = if cfg!(windows) {
            Path::new(r"C:\Windows\System32\drivers\etc\hosts")
        } else {
            Path::new("/etc/hosts")
        };
        assert!(verify_workspace_jail(Some(&ws()), outside).is_err());
    }

    #[test]
    fn rejects_without_workspace() {
        assert!(verify_workspace_jail(None, Path::new("a.rs")).is_err());
    }

    #[test]
    fn resolves_inner_dot_segments() {
        let p = verify_workspace_jail(Some(&ws()), Path::new("./src/./lib.rs")).unwrap();
        assert!(!p.to_string_lossy().contains("./"));
    }

    #[test]
    fn normalizes_to_parent_inside_workspace() {
        // 工作区内合法的 `..` 应被消解而不是一刀切拒绝
        let p = verify_workspace_jail(Some(&ws()), Path::new("src/../Cargo.toml")).unwrap();
        assert!(p.to_string_lossy().ends_with("Cargo.toml"));
    }
}
