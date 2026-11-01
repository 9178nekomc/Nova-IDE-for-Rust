// NovaRS — Common Utilities (UTF-8 Char/Byte Slicing, Line Indexing, Process & OS Helpers)

use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use eframe::egui::{self, pos2, Vec2, ViewportCommand};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use crate::constants::{CREATE_NO_WINDOW, SPLASH_DURATION};

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 跨平台隐藏子进程控制台窗口
pub fn apply_no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// 检测 rustc 版本
pub fn run_rustc_version() -> Option<String> {
    let mut cmd = Command::new("rustc");
    apply_no_window(&mut cmd).arg("--version");
    let output = cmd.output().ok()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    } else {
        None
    }
}

/// 检测 cargo 版本
pub fn run_cargo_version() -> Option<String> {
    let mut cmd = Command::new("cargo");
    apply_no_window(&mut cmd).arg("--version");
    let output = cmd.output().ok()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    } else {
        None
    }
}

/// 检测 rustup 版本
pub fn run_rustup_version() -> Option<String> {
    let mut cmd = Command::new("rustup");
    apply_no_window(&mut cmd).arg("--version");
    let output = cmd.output().ok()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).lines().next().unwrap_or("").trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    } else {
        None
    }
}

/// 在 PATH 中查找可执行文件是否存在
pub fn which_exists(program: &str) -> bool {
    #[cfg(target_os = "windows")]
    let locator = "where";
    #[cfg(not(target_os = "windows"))]
    let locator = "which";

    let mut cmd = Command::new(locator);
    apply_no_window(&mut cmd).arg(program);
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

/// 读取系统剪贴板文本
pub fn read_clipboard_text() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("powershell");
        apply_no_window(&mut cmd).args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-Clipboard -TextFormatType UnicodeText",
        ]);
        let output = cmd.output().ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        let trimmed = text.strip_suffix("\r\n").unwrap_or(&text);
        let trimmed = trimmed.strip_suffix('\n').unwrap_or(trimmed);
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// 在系统文件管理器中显示文件/文件夹
pub fn reveal_in_explorer(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let target = if path.is_dir() {
            path.display().to_string()
        } else {
            format!("/select,{}", path.display())
        };
        let mut cmd = Command::new("explorer");
        apply_no_window(&mut cmd).arg(target);
        let _ = cmd.spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg("-R").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
        let _ = Command::new("xdg-open").arg(dir).spawn();
    }
}

pub fn read_git_branch(workspace: Option<&Path>) -> Option<String> {
    let ws = workspace?;
    let head = ws.join(".git").join("HEAD");
    let content = std::fs::read_to_string(&head).ok()?;
    let content = content.trim();
    content
        .strip_prefix("ref: refs/heads/")
        .map(|s| s.to_string())
        .or_else(|| Some(content.chars().take(7).collect()))
}

pub fn char_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

pub fn slice_by_char_range(text: &str, a: usize, b: usize) -> String {
    let ba = char_to_byte(text, a);
    let bb = char_to_byte(text, b);
    text[ba.min(bb)..ba.max(bb)].to_string()
}

pub fn replace_char_range(text: &str, a: usize, b: usize, insert: &str) -> String {
    let (lo, hi) = (a.min(b), a.max(b));
    let ba = char_to_byte(text, lo);
    let bb = char_to_byte(text, hi);
    let mut out = String::with_capacity(text.len() + insert.len());
    out.push_str(&text[..ba]);
    out.push_str(insert);
    out.push_str(&text[bb..]);
    out
}

pub fn line_start_char_index(text: &str, line_1based: usize) -> usize {
    let target_newlines = line_1based.saturating_sub(1);
    let mut newlines_seen = 0usize;
    let mut count = 0usize;
    for ch in text.chars() {
        if newlines_seen >= target_newlines {
            break;
        }
        if ch == '\n' {
            newlines_seen += 1;
        }
        count += 1;
    }
    count
}

pub fn line_start_byte_index(text: &str, line_1based: usize) -> usize {
    let target_newlines = line_1based.saturating_sub(1);
    if target_newlines == 0 {
        return 0;
    }
    let mut newlines_seen = 0usize;
    for (byte_idx, ch) in text.char_indices() {
        if ch == '\n' {
            newlines_seen += 1;
            if newlines_seen >= target_newlines {
                return byte_idx + 1;
            }
        }
    }
    text.len()
}

pub fn line_col_of_char_index(text: &str, char_idx: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 1usize;
    let mut seen = 0usize;
    for ch in text.chars() {
        if seen >= char_idx {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
        seen += 1;
    }
    (line, col)
}

/// 精确计算总行数（支持任意大小文件，末尾 `\n` 计入新行）
pub fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        1
    } else {
        let newlines = text.as_bytes().iter().filter(|&&b| b == b'\n').count();
        newlines + 1
    }
}

pub fn utf8_char_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >= 0xF0 {
        4
    } else if b >= 0xE0 {
        3
    } else {
        2
    }
}

/// 小写化并可选地返回"小写串字节偏移 → 原串字节偏移"的映射。
///
/// `str::to_lowercase` 可能改变字节长度（例如 `İ` U+0130 会展开为 `i` + U+0307，
/// Kelvin 符号 `K` U+212A 会收缩为 `k`）。若直接把小写串的命中偏移拿去切原串，
/// `&text[..byte_at]` 会落在非字符边界上直接 panic。长度不变时（ASCII/CJK 等绝大多数
/// 场景）返回 `None`，不做任何额外分配。
pub fn lowercase_with_offset_map(text: &str) -> (String, Option<Vec<usize>>) {
    let lowered = text.to_lowercase();
    if lowered.len() == text.len() {
        return (lowered, None);
    }
    let mut rebuilt = String::with_capacity(lowered.len());
    let mut map: Vec<usize> = Vec::with_capacity(lowered.len() + 1);
    for (byte_idx, ch) in text.char_indices() {
        for lc in ch.to_lowercase() {
            rebuilt.push(lc);
            for _ in 0..lc.len_utf8() {
                map.push(byte_idx);
            }
        }
    }
    map.push(text.len());
    (rebuilt, Some(map))
}

pub fn char_literal_end(bytes: &[u8], i: usize) -> Option<usize> {
    let n = bytes.len();
    let next = *bytes.get(i + 1)?;
    if next == b'\\' {
        let after = i + 2;
        if after < n && bytes.get(after) == Some(&b'\'') {
            return Some(after + 1);
        }
        let mut k = after;
        while k < n {
            if bytes[k] == b'\'' {
                return Some(k + 1);
            }
            k += 1;
        }
        None
    } else if i + 2 < n && bytes[i + 2] == b'\'' {
        Some(i + 3)
    } else {
        None
    }
}

pub fn center_window(ctx: &egui::Context, size: Vec2) {
    let monitor = ctx.input(|i| i.viewport().monitor_size);
    if let Some(m) = monitor {
        let x = ((m.x - size.x) / 2.0).max(0.0);
        let y = ((m.y - size.y) / 2.0).max(0.0);
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos2(x, y)));
    }
}

pub fn splash_duration_from_env() -> Duration {
    match std::env::var("NOVARS_SPLASH_SECS") {
        Ok(raw) => raw
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| *v > 0.0 && *v < 3600.0)
            .map(Duration::from_secs_f64)
            .unwrap_or(SPLASH_DURATION),
        Err(_) => SPLASH_DURATION,
    }
}

/// 剥离终端输出中的 ANSI CSI/OSC 控制序列，避免终端面板出现乱码控制符
pub fn strip_ansi_escapes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if ch != '\r' {
            out.push(ch);
        }
    }
    out
}

/// 选区内每行行首缩进 4 空格，返回（新文本，新起点字符索引，新终点字符索引）
pub fn indent_lines(text: &str, start_char: usize, end_char: usize) -> (String, usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let mut start_line = start_char.min(chars.len());
    while start_line > 0 && chars[start_line - 1] != '\n' {
        start_line -= 1;
    }
    let mut out = String::with_capacity(text.len() + 64);
    let mut new_start = start_char;
    let mut new_end = end_char;
    for (i, &ch) in chars.iter().enumerate() {
        let at_line_start = i == 0 || chars[i - 1] == '\n';
        let end_in_line_mid = i == end_char
            && chars.get(end_char).map(|c| *c != '\n').unwrap_or(false);
        let do_indent = at_line_start && i >= start_line && (i < end_char || end_in_line_mid);
        if do_indent {
            out.push_str("    ");
            if i < start_char {
                new_start += 4;
            }
            new_end += 4;
        }
        out.push(ch);
    }
    (out, new_start, new_end)
}

/// 选区内每行取消最多 4 空格缩进（Shift+Tab）
pub fn unindent_lines(text: &str, start_char: usize, end_char: usize) -> (String, usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let mut start_line = start_char.min(chars.len());
    while start_line > 0 && chars[start_line - 1] != '\n' {
        start_line -= 1;
    }
    let mut out = String::with_capacity(text.len());
    let mut new_start = start_char;
    let mut new_end = end_char;
    let mut i = 0usize;
    while i < chars.len() {
        let at_line_start = i == 0 || chars[i - 1] == '\n';
        let end_in_line_mid = i == end_char
            && chars.get(end_char).map(|c| *c != '\n').unwrap_or(false);
        let do_unindent = at_line_start && i >= start_line && (i <= end_char || end_in_line_mid);
        if do_unindent {
            let mut removed = 0usize;
            while i < chars.len() && removed < 4 && chars[i] == ' ' {
                if i < start_char {
                    new_start = new_start.saturating_sub(1);
                }
                if i < end_char {
                    new_end = new_end.saturating_sub(1);
                }
                i += 1;
                removed += 1;
            }
            if removed == 0 && i < chars.len() && chars[i] == '\t' {
                if i < start_char {
                    new_start = new_start.saturating_sub(1);
                }
                if i < end_char {
                    new_end = new_end.saturating_sub(1);
                }
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    (out, new_start, new_end)
}
