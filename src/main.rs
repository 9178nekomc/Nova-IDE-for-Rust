// NovaRS — Nova IDE for Rust
// 纯 Rust 原生实现的轻量 IDE（egui + eframe，单二进制）
// 迭代 v0.2：rust-analyzer 内嵌智能补全 / 断点 / MiniMap / 终端 / 项目搜索 / 状态栏增强
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use eframe::egui;
use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::widgets::text_edit::TextEditState;
use egui::{
    pos2, Align, Align2, Color32, FontData, FontDefinitions, FontFamily,
    FontId, FontSelection, Galley, Id, Key, Rect, RichText, Sense, Stroke, TextFormat,
    Vec2, ViewportCommand, Visuals,
};
use walkdir::WalkDir;

// ============================ 常量 ============================

const APP_NAME: &str = "NovaRS";
const APP_FULL_NAME: &str = "Nova IDE for Rust";
const APP_VERSION: &str = "v0.1.0";

const SPLASH_SIZE: Vec2 = Vec2 { x: 480.0, y: 320.0 };
const MAIN_SIZE: Vec2 = Vec2 { x: 1280.0, y: 800.0 };
const SPLASH_DURATION: Duration = Duration::from_millis(2500);
const SPLASH_FORCE_DEADLINE: Duration = Duration::from_secs(5);
const SPLASH_MIN_VISIBLE: Duration = Duration::from_millis(1200);

const BRAND_RED: Color32 = Color32::from_rgb(0xe9, 0x45, 0x60);
const SPLASH_C1: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x2e);
const SPLASH_C2: Color32 = Color32::from_rgb(0x16, 0x21, 0x3e);
const SPLASH_C3: Color32 = Color32::from_rgb(0x0f, 0x34, 0x60);

const MONO_SIZE: f32 = 13.0;
const SPLASH_FLOW_LINES: usize = 4;
const SPLASH_LOAD_TEXTS: [&str; 3] = ["正在初始化引擎...", "加载文件系统...", "准备就绪..."];

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
    "extern", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
    "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "type",
    "unsafe", "use", "where", "while",
];

/// 子进程不创建控制台窗口（GUI 应用下 rustc/cargo/powershell 等子进程会闪黑框）
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 编译期嵌入的 rust-analyzer 可执行文件（build.rs 定位；未找到时为空，运行期降级）
const RA_EMBEDDED: &[u8] = include_bytes!(env!("NOVARS_RA_PATH"));

/// 编译期嵌入的中文界面字体（等线），保证单文件 exe 在任何机器上中文渲染一致
const FONT_CJK_EMBEDDED: &[u8] = include_bytes!("../assets/Deng.ttf");

const RA_RELEASE_NAME: &str = "novars-rust-analyzer.exe";
const MINIMAP_W: f32 = 96.0;
const GUTTER_W: f32 = 16.0;
const POPUP_ROW_H: f32 = 21.0;

// ============================ 通用辅助 ============================

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 检测 rustc 版本（命令与参数均为编译期字面量，参数列表调用，无 shell 解释）
fn run_rustc_version() -> Option<String> {
    let output = std::process::Command::new("rustc")
        .arg("--version")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    } else {
        None
    }
}

/// 检测 cargo 版本（命令与参数均为编译期字面量，参数列表调用，无 shell 解释）
fn run_cargo_version() -> Option<String> {
    let output = std::process::Command::new("cargo")
        .arg("--version")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    } else {
        None
    }
}

/// 在 PATH 中查找可执行文件是否存在（where 为 Windows 系统工具，参数列表调用）
fn which_exists(program: &str) -> bool {
    std::process::Command::new("where")
        .arg(program)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 读取系统剪贴板文本（PowerShell 参数列表调用，参数为编译期字面量）
fn read_clipboard_text() -> Option<String> {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-Clipboard -TextFormatType UnicodeText",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let trimmed = text.strip_suffix("\r\n").unwrap_or(&text);
    let trimmed = trimmed.strip_suffix('\n').unwrap_or(trimmed);
    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
}

/// 在系统文件管理器中显示文件/文件夹
fn reveal_in_explorer(path: &Path) {
    let target = if path.is_dir() {
        path.display().to_string()
    } else {
        format!("/select,{}", path.display())
    };
    let _ = std::process::Command::new("explorer")
        .arg(target)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

fn read_git_branch(workspace: Option<&Path>) -> Option<String> {
    let ws = workspace?;
    let head = ws.join(".git").join("HEAD");
    let content = std::fs::read_to_string(&head).ok()?;
    let content = content.trim();
    content
        .strip_prefix("ref: refs/heads/")
        .map(|s| s.to_string())
        .or_else(|| Some(content.chars().take(7).collect()))
}

fn lang_of_path(path: &Path) -> &'static str {
    match path.extension().map(|e| e.to_string_lossy().to_lowercase()) {
        Some(e) if e == "rs" => "Rust",
        Some(e) if e == "md" => "Markdown",
        Some(e) if e == "toml" => "TOML",
        Some(e) if e == "json" => "JSON",
        _ => "文本",
    }
}

fn char_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

fn slice_by_char_range(text: &str, a: usize, b: usize) -> String {
    let ba = char_to_byte(text, a);
    let bb = char_to_byte(text, b);
    text[ba.min(bb)..ba.max(bb)].to_string()
}

fn replace_char_range(text: &str, a: usize, b: usize, insert: &str) -> String {
    let (lo, hi) = (a.min(b), a.max(b));
    let ba = char_to_byte(text, lo);
    let bb = char_to_byte(text, hi);
    let mut out = String::with_capacity(text.len() + insert.len());
    out.push_str(&text[..ba]);
    out.push_str(insert);
    out.push_str(&text[bb..]);
    out
}

fn line_start_char_index(text: &str, line_1based: usize) -> usize {
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

fn line_col_of_char_index(text: &str, char_idx: usize) -> (usize, usize) {
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

fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        1
    } else if text.ends_with('\n') {
        text.lines().count() + 1
    } else {
        text.lines().count().max(1)
    }
}

fn utf8_char_len(b: u8) -> usize {
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

fn char_literal_end(bytes: &[u8], i: usize) -> Option<usize> {
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

fn center_window(ctx: &egui::Context, size: Vec2) {
    let monitor = ctx.input(|i| i.viewport().monitor_size);
    if let Some(m) = monitor {
        let x = ((m.x - size.x) / 2.0).max(0.0);
        let y = ((m.y - size.y) / 2.0).max(0.0);
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos2(x, y)));
    }
}

fn splash_duration_from_env() -> Duration {
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

// ============================ rust-analyzer LSP 补全 ============================

#[derive(Clone, PartialEq)]
struct LspCompletion {
    label: String,
    kind_tag: &'static str,
    detail: String,
}

fn lsp_kind_tag(kind: Option<u64>) -> &'static str {
    match kind {
        Some(2) | Some(3) | Some(4) => "fn",
        Some(5) | Some(6) | Some(21) => "let",
        Some(7) | Some(22) => "struct",
        Some(13) | Some(20) => "enum",
        Some(9) => "mod",
        Some(8) | Some(10) | Some(25) => "trait",
        Some(14) => "kw",
        Some(15) => "snippet",
        Some(17) | Some(18) | Some(19) => "file",
        Some(1) => "text",
        _ => "var",
    }
}

fn path_to_uri(path: &Path) -> String {
    let text = path.display().to_string().replace('\\', "/");
    let mut out = String::from("file:///");
    for ch in text.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~' | '/' | ':' => out.push(ch),
            _ => {
                let mut buf = [0u8; 4];
                for b in ch.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    out
}

struct LspSession {
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<i64>>,
    items: Arc<Mutex<Vec<LspCompletion>>>,
    items_gen: Arc<Mutex<i64>>,
    ready: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    versions: Arc<Mutex<HashMap<String, i64>>>,
    opened: Arc<Mutex<HashSet<String>>>,
}

impl LspSession {
    fn send(&self, value: &serde_json::Value) -> bool {
        let Ok(json) = serde_json::to_string(value) else { return false };
        let mut guard = lock(&self.stdin);
        let frame = format!("Content-Length: {}\r\n\r\n{}", json.len(), json);
        match guard.write_all(frame.as_bytes()).and_then(|_| guard.flush()) {
            Ok(()) => true,
            Err(_) => {
                drop(guard);
                self.alive.store(false, Ordering::Relaxed);
                false
            }
        }
    }

    fn next_request_id(&self) -> i64 {
        let mut guard = lock(&self.pending);
        let id = if *guard < 0 { 2 } else { *guard + 1 };
        *guard = id;
        id
    }

    fn request_completion(&self, uri: &str, line: usize, character: usize) -> i64 {
        let gen = self.next_request_id();
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": gen,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line.saturating_sub(1), "character": character }
            }
        }));
        gen
    }

    fn notify_did_open(&self, path: &Path, text: &str) {
        let uri = path_to_uri(path);
        {
            let mut opened = lock(&self.opened);
            if !opened.insert(uri.clone()) {
                return;
            }
        }
        *lock(&self.versions).entry(uri.clone()).or_insert(1) = 1;
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": { "uri": uri, "languageId": "rust", "version": 1, "text": text }
            }
        }));
    }

    fn notify_did_change(&self, path: &Path, text: &str) {
        let uri = path_to_uri(path);
        let was_open = lock(&self.opened).contains(&uri);
        if !was_open {
            self.notify_did_open(path, text);
            return;
        }
        let mut versions = lock(&self.versions);
        let version = versions.entry(uri.clone()).or_insert(1);
        *version += 1;
        let version = *version;
        drop(versions);
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [ { "text": text } ]
            }
        }));
    }
}

fn read_lsp_message(reader: &mut BufReader<ChildStdout>) -> Option<Vec<u8>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        if line.trim().is_empty() {
            break;
        }
        if let Some(rest) = line.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse::<usize>().ok();
        }
    }
    let n = content_length?;
    let mut buf = vec![0u8; n];
    reader.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn parse_completion_items(result: &serde_json::Value) -> Vec<LspCompletion> {
    let empty = Vec::new();
    let arr: &Vec<serde_json::Value> = match result {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(o) => match o.get("items").and_then(|x| x.as_array()) {
            Some(a) => a,
            None => return empty,
        },
        _ => return empty,
    };
    let mut out = Vec::new();
    for item in arr.iter().take(500) {
        let label = match item.get("label").and_then(|x| x.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let kind = item.get("kind").and_then(|x| x.as_u64());
        let detail = item
            .get("detail")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        out.push(LspCompletion { label, kind_tag: lsp_kind_tag(kind), detail });
    }
    out
}

fn lsp_reader(mut stdout: BufReader<ChildStdout>, session: Arc<LspSession>, gate: Arc<AtomicBool>) {
    loop {
        let Some(buf) = read_lsp_message(&mut stdout) else {
            session.alive.store(false, Ordering::Relaxed);
            gate.store(true, Ordering::Relaxed);
            return;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&buf) else { continue };
        let Some(id) = value.get("id").and_then(|x| x.as_i64()) else { continue };
        if id == 1 {
            // initialize 响应：LSP 就绪，发送 initialized 通知完成握手
            session.ready.store(true, Ordering::Relaxed);
            gate.store(true, Ordering::Relaxed);
            session.send(&serde_json::json!({
                "jsonrpc": "2.0", "method": "initialized", "params": {}
            }));
            continue;
        }
        let is_pending = { *lock(&session.pending) == id };
        if is_pending {
            let items = match value.get("result") {
                Some(r) if !r.is_null() => parse_completion_items(r),
                _ => Vec::new(),
            };
            *lock(&session.items) = items;
            *lock(&session.items_gen) = id;
            *lock(&session.pending) = -1;
        }
    }
}

/// 释放并启动 rust-analyzer，完成 LSP 握手。任何失败都会置位 gate 并返回 None（降级补全）。
fn start_rust_analyzer(
    embedded: &[u8],
    workspace: Option<PathBuf>,
    gate: Arc<AtomicBool>,
) -> Option<Arc<LspSession>> {
    if embedded.is_empty() {
        gate.store(true, Ordering::Relaxed);
        return None;
    }
    let exe = std::env::temp_dir().join(RA_RELEASE_NAME);
    if std::fs::write(&exe, embedded).is_err() {
        gate.store(true, Ordering::Relaxed);
        return None;
    }
    let mut cmd = std::process::Command::new(&exe);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    if let Some(ws) = &workspace {
        cmd.current_dir(ws);
    }
    let Ok(mut child) = cmd.spawn() else {
        gate.store(true, Ordering::Relaxed);
        return None;
    };
    let stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;

    let session = Arc::new(LspSession {
        stdin: Arc::new(Mutex::new(stdin)),
        pending: Arc::new(Mutex::new(-1)),
        items: Arc::new(Mutex::new(Vec::new())),
        items_gen: Arc::new(Mutex::new(-1)),
        ready: Arc::new(AtomicBool::new(false)),
        alive: Arc::new(AtomicBool::new(true)),
        versions: Arc::new(Mutex::new(HashMap::new())),
        opened: Arc::new(Mutex::new(HashSet::new())),
    });

    let root_uri = match &workspace {
        Some(ws) => serde_json::json!(path_to_uri(ws)),
        None => serde_json::Value::Null,
    };
    session.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        }
    }));

    {
        let session = session.clone();
        let gate = gate.clone();
        std::thread::spawn(move || lsp_reader(BufReader::new(stdout), session, gate));
    }
    Some(session)
}

// ============================ 内置降级补全 ============================

const STD_COMPLETIONS: &[(&str, &str)] = &[
    ("Vec", "struct"), ("String", "struct"), ("Option", "enum"), ("Result", "enum"),
    ("HashMap", "struct"), ("HashSet", "struct"), ("Box", "struct"), ("Rc", "struct"),
    ("Arc", "struct"), ("Some", "enum"), ("None", "kw"), ("Ok", "enum"), ("Err", "enum"),
    ("println", "fn"), ("print", "fn"), ("format", "fn"), ("vec!", "snippet"),
    ("write!", "snippet"), ("assert_eq", "fn"), ("match", "kw"), ("impl", "kw"),
    ("self", "kw"), ("Self", "kw"), ("super", "kw"), ("crate", "kw"),
];

fn builtin_completions(
    text: &str,
    prefix: &str,
    workspace: Option<&Path>,
) -> Vec<LspCompletion> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<LspCompletion> = Vec::new();
    {
        let mut push = |label: String, tag: &'static str, detail: &str| {
            if label.is_empty() || label == prefix || seen.contains(&label) || out.len() >= 200 {
                return;
            }
            seen.insert(label.clone());
            out.push(LspCompletion { label, kind_tag: tag, detail: detail.to_string() });
        };

        for line in text.lines() {
            let trimmed = line.trim_start();
            for (kw, tag) in [
                ("pub fn ", "fn"), ("fn ", "fn"), ("pub struct ", "struct"), ("struct ", "struct"),
                ("enum ", "enum"), ("trait ", "trait"), ("mod ", "mod"), ("use ", "use"),
            ] {
                if let Some(rest) = trimmed.strip_prefix(kw) {
                    let name: String =
                        rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    push(name, tag, "当前文件");
                }
            }
            if let Some(rest) = trimmed.strip_prefix("let ") {
                if let Some(name) = rest.split('=').next() {
                    let ident: String = name
                        .trim()
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    push(ident, "let", "当前文件");
                }
            }
        }
        for (name, tag) in STD_COMPLETIONS {
            push((*name).to_string(), tag, "标准库");
        }
        if let Some(ws) = workspace {
            let manifest = ws.join("Cargo.toml");
            if let Ok(content) = std::fs::read_to_string(&manifest) {
                let mut in_deps = false;
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with('[') {
                        in_deps = trimmed == "[dependencies]";
                        continue;
                    }
                    if in_deps {
                        if let Some(eq) = trimmed.find('=') {
                            let name = trimmed[..eq].trim().to_string();
                            push(name, "crate", "Cargo.toml 依赖");
                        }
                    }
                }
            }
        }
    }

    if !prefix.is_empty() {
        let lower_prefix = prefix.to_lowercase();
        out.retain(|c| c.label.to_lowercase().starts_with(&lower_prefix));
        out.sort_by(|a, b| {
            let a_exact = a.label.starts_with(prefix);
            let b_exact = b.label.starts_with(prefix);
            b_exact.cmp(&a_exact).then_with(|| a.label.cmp(&b.label))
        });
    }
    out.truncate(200);
    out
}

// ============================ MiniMap ============================

#[derive(Clone)]
struct MiniRow {
    indent: f32,
    width: f32,
    color: Color32,
}

fn build_minimap(text: &str) -> Vec<MiniRow> {
    let mut rows = Vec::new();
    for line in text.lines().take(4000) {
        let indent_spaces = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        let color = if trimmed.starts_with("//") {
            Color32::from_rgb(0x6a, 0x70, 0x7a)
        } else if trimmed.starts_with('"') || trimmed.starts_with('#') {
            Color32::from_rgb(0x4f, 0x7a, 0x4a)
        } else {
            let first_word: String =
                trimmed.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if RUST_KEYWORDS.contains(&first_word.as_str()) {
                Color32::from_rgb(0x7a, 0x5a, 0x8a)
            } else if first_word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                Color32::from_rgb(0x8a, 0x74, 0x4a)
            } else if trimmed.is_empty() {
                Color32::from_rgb(0x25, 0x28, 0x2e)
            } else {
                Color32::from_rgb(0x50, 0x58, 0x66)
            }
        };
        let width = (trimmed.chars().count() as f32 * 0.9).min(MINIMAP_W - 6.0);
        rows.push(MiniRow { indent: indent_spaces as f32 * 0.7, width, color });
    }
    rows
}

// ============================ 终端面板 ============================

struct TerminalState {
    visible: bool,
    child: Option<Arc<Mutex<Child>>>,
    stdin: Option<Arc<Mutex<ChildStdin>>>,
    log: Arc<Mutex<String>>,
    input: String,
}

impl TerminalState {
    fn new() -> Self {
        TerminalState {
            visible: false,
            child: None,
            stdin: None,
            log: Arc::new(Mutex::new(String::new())),
            input: String::new(),
        }
    }

    fn append(log: &Arc<Mutex<String>>, text: &str, ctx: &egui::Context) {
        lock(log).push_str(text);
        ctx.request_repaint();
    }

    fn ensure_spawned(&mut self, workspace: Option<&Path>, ctx: &egui::Context) -> bool {
        if self.stdin.is_some() {
            return true;
        }
        let Ok(mut child) = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "-"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        else {
            Self::append(&self.log, "[无法启动 powershell]\n", ctx);
            return false;
        };
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        self.child = Some(Arc::new(Mutex::new(child)));
        self.stdin = stdin.map(|s| Arc::new(Mutex::new(s)));

        // 让 PowerShell 后续所有输出（含错误流）转为 UTF-8，与界面解码一致，避免中文乱码
        if let Some(stdin_arc) = &self.stdin {
            let mut guard = lock(stdin_arc);
            let _ = guard
                .write_all(
                    b"[Console]::OutputEncoding=[System.Text.Encoding]::UTF8\n",
                )
                .and_then(|_| guard.flush());
        }

        let cwd = workspace
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| std::env::temp_dir().display().to_string());
        Self::append(
            &self.log,
            &format!("NovaRS 终端 — PowerShell 会话已启动（工作目录: {}）\n", cwd),
            ctx,
        );

        if let Some(out) = stdout {
            let log = self.log.clone();
            let ctx2 = ctx.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(out);
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let text = String::from_utf8_lossy(&buf[..n]).to_string();
                            Self::append(&log, &text, &ctx2);
                        }
                    }
                }
            });
        }
        if let Some(err) = stderr {
            let log = self.log.clone();
            let ctx2 = ctx.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(err);
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let text = String::from_utf8_lossy(&buf[..n]).to_string();
                            Self::append(&log, &text, &ctx2);
                        }
                    }
                }
            });
        }
        true
    }

    fn send_command(&mut self, workspace: Option<&Path>, ctx: &egui::Context) {
        let cmd = self.input.trim().to_string();
        if cmd.is_empty() {
            return;
        }
        if self.ensure_spawned(workspace, ctx) {
            Self::append(&self.log, &format!("PS> {}\n", cmd), ctx);
            if let Some(stdin_arc) = &self.stdin {
                let mut guard = lock(stdin_arc);
                let _ = guard
                    .write_all(cmd.as_bytes())
                    .and_then(|_| guard.write_all(b"\n"))
                    .and_then(|_| guard.flush());
            }
        }
        self.input.clear();
    }

    fn kill(&mut self) {
        if let Some(child_arc) = &self.child {
            let mut guard = lock(child_arc);
            let _ = guard.kill();
        }
        self.child = None;
        self.stdin = None;
    }
}

// ============================ 项目搜索 ============================

#[derive(Clone)]
struct SearchHit {
    path: PathBuf,
    line_no: usize,
    text: String,
}

struct SearchState {
    input: String,
    case_sensitive: bool,
    running: Arc<AtomicBool>,
    results: Arc<Mutex<Vec<SearchHit>>>,
}

struct FindState {
    input: String,
    total: usize,
    index: usize,
}

#[derive(Clone)]
struct CompletionPopup {
    items: Vec<LspCompletion>,
    selected: usize,
    gen: i64,
    base_char: usize,
}

// ============================ 新建项目模板 ============================

const PROJECT_TEMPLATES: &[&str] = &[
    "空项目",
    "输出",
    "变量",
    "判断",
    "输入判断输出",
    "异步任务",
];

/// 清洗为合法 Cargo 包名（字母数字连字符下划线）
fn sanitize_package_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('-').to_string();
    if cleaned.is_empty() {
        "new-project".to_string()
    } else {
        cleaned
    }
}

fn template_cargo_toml(package: &str) -> String {
    format!(
        "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        package
    )
}

fn template_main_rs(template: usize) -> String {
    match template {
        1 => r#"fn main() {
    println!("Hello, NovaRS!");
    println!("这是输出模板：数字 = {}", 42);
}
"#
        .to_string(),
        2 => r#"fn main() {
    let mut count: u32 = 0;
    let name = "NovaRS";

    count += 1;
    let message = format!("变量模板：{} 已运行 {} 次", name, count);
    println!("{}", message);
}
"#
        .to_string(),
        3 => r#"fn main() {
    let n: i32 = 7;

    if n % 2 == 0 {
        println!("{} 是偶数", n);
    } else {
        println!("{} 是奇数", n);
    }

    match n {
        0..=9 => println!("个位数"),
        10..=99 => println!("两位数"),
        _ => println!("更大的数"),
    }
}
"#
        .to_string(),
        4 => r#"use std::io;

fn main() {
    println!("请输入你的年龄：");

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .expect("读取输入失败");

    let age: u32 = input.trim().parse().expect("请输入一个数字");

    if age >= 18 {
        println!("{} 岁：你已成年", age);
    } else {
        println!("{} 岁：你还未成年", age);
    }
}
"#
        .to_string(),
        5 => r#"use std::thread;

fn main() {
    // 任务 1：在子线程中计数
    let handle = thread::spawn(|| {
        for i in 1..=5 {
            println!("[任务 1] 第 {} 次", i);
            thread::sleep(std::time::Duration::from_millis(100));
        }
    });

    // 任务 2：主线程同时计数
    for i in 1..=5 {
        println!("[任务 2] 第 {} 次", i);
        thread::sleep(std::time::Duration::from_millis(100));
    }

    let _ = handle.join();
    println!("所有异步任务完成");
}
"#
        .to_string(),
        _ => "fn main() {}\n".to_string(),
    }
}

struct ProjectDialog {
    name: String,
    location: PathBuf,
    template: usize,
    error: Option<String>,
}

/// 选区内每行行首缩进 4 空格，返回（新文本，新起点字符索引，新终点字符索引）
fn indent_lines(text: &str, start_char: usize, end_char: usize) -> (String, usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let mut start_line = start_char;
    while start_line > 0 && chars[start_line - 1] != '\n' {
        start_line -= 1;
    }
    let mut out = String::with_capacity(text.len() + 64);
    let mut new_start = start_char;
    let mut new_end = end_char;
    for (i, &ch) in chars.iter().enumerate() {
        let at_line_start = i == 0 || chars[i - 1] == '\n';
        // 选区结束恰在行首（\n 之后）时，该行不缩进
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

// ============================ 语法高亮 ============================

#[derive(Clone, Copy)]
enum TokKind {
    Keyword,
    Str,
    Comment,
    Number,
    Ty,
    Lifetime,
    Normal,
}

struct CodeTheme {
    keyword: Color32,
    string: Color32,
    comment: Color32,
    number: Color32,
    ty: Color32,
    lifetime: Color32,
    normal: Color32,
    line_number: Color32,
}

impl CodeTheme {
    fn new(dark: bool) -> Self {
        if dark {
            CodeTheme {
                keyword: Color32::from_rgb(0xc6, 0x78, 0xdd),
                string: Color32::from_rgb(0x98, 0xc3, 0x79),
                comment: Color32::from_rgb(0x7f, 0x84, 0x8e),
                number: Color32::from_rgb(0xd1, 0x9a, 0x66),
                ty: Color32::from_rgb(0xe5, 0xc0, 0x7b),
                lifetime: Color32::from_rgb(0xe0, 0x6c, 0x75),
                normal: Color32::from_rgb(0xab, 0xb2, 0xbf),
                line_number: Color32::from_rgb(0x5c, 0x63, 0x70),
            }
        } else {
            CodeTheme {
                keyword: Color32::from_rgb(0xa6, 0x26, 0xa4),
                string: Color32::from_rgb(0x50, 0xa1, 0x4f),
                comment: Color32::from_rgb(0xa0, 0xa1, 0xa7),
                number: Color32::from_rgb(0xb7, 0x6b, 0x01),
                ty: Color32::from_rgb(0xc1, 0x84, 0x01),
                lifetime: Color32::from_rgb(0xca, 0x4b, 0x37),
                normal: Color32::from_rgb(0x38, 0x3a, 0x42),
                line_number: Color32::from_rgb(0x9c, 0xa0, 0xa4),
            }
        }
    }

    fn color_of(&self, kind: TokKind) -> Color32 {
        match kind {
            TokKind::Keyword => self.keyword,
            TokKind::Str => self.string,
            TokKind::Comment => self.comment,
            TokKind::Number => self.number,
            TokKind::Ty => self.ty,
            TokKind::Lifetime => self.lifetime,
            TokKind::Normal => self.normal,
        }
    }
}

fn push_token(job: &mut LayoutJob, text: &str, start: usize, end: usize, kind: TokKind, theme: &CodeTheme, font: &FontId) {
    if end <= start || start >= text.len() {
        return;
    }
    let end = end.min(text.len());
    let mut fmt = TextFormat::simple(font.clone(), theme.color_of(kind));
    if matches!(kind, TokKind::Comment) {
        fmt.italics = true;
    }
    job.append(&text[start..end], 0.0, fmt);
}

fn highlight_rust(text: &str, theme: &CodeTheme, job: &mut LayoutJob) {
    let font = FontId::monospace(MONO_SIZE);
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    while i < n {
        let b = bytes[i];
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            let start = i;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            push_token(job, text, start, i, TokKind::Comment, theme, &font);
        } else if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            let start = i;
            let mut depth = 1usize;
            i += 2;
            while i < n && depth > 0 {
                if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && i + 1 < n && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            push_token(job, text, start, i, TokKind::Comment, theme, &font);
        } else if b == b'"' {
            let start = i;
            i += 1;
            while i < n {
                if bytes[i] == b'\\' {
                    i += 2;
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            push_token(job, text, start, i, TokKind::Str, theme, &font);
        } else if b == b'\'' {
            if let Some(end) = char_literal_end(bytes, i) {
                push_token(job, text, i, end, TokKind::Str, theme, &font);
                i = end;
            } else {
                let start = i;
                i += 1;
                while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                push_token(job, text, start, i, TokKind::Lifetime, theme, &font);
            }
        } else if b == b'r' && i + 1 < n && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') {
            let start = i;
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < n && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < n && bytes[j] == b'"' {
                j += 1;
                let mut end = n;
                while j < n {
                    if bytes[j] == b'"' {
                        let mut k = j + 1;
                        let mut cnt = 0usize;
                        while cnt < hashes && k < n && bytes[k] == b'#' {
                            cnt += 1;
                            k += 1;
                        }
                        if cnt == hashes {
                            end = k;
                            break;
                        }
                        j = k;
                    } else {
                        j += 1;
                    }
                }
                i = end;
                push_token(job, text, start, i, TokKind::Str, theme, &font);
            } else {
                while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let word = &text[start..i];
                let kind = if RUST_KEYWORDS.contains(&word) { TokKind::Keyword } else { TokKind::Normal };
                push_token(job, text, start, i, kind, theme, &font);
            }
        } else if b.is_ascii_digit() {
            let start = i;
            while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_') {
                i += 1;
            }
            push_token(job, text, start, i, TokKind::Number, theme, &font);
        } else if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            let kind = if RUST_KEYWORDS.contains(&word) {
                TokKind::Keyword
            } else if word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                TokKind::Ty
            } else {
                TokKind::Normal
            };
            push_token(job, text, start, i, kind, theme, &font);
        } else {
            let start = i;
            i += utf8_char_len(b);
            push_token(job, text, start, i, TokKind::Normal, theme, &font);
        }
    }
}

// ============================ AST 解析 ============================

fn collect_ast_items(items: &[syn::Item], depth: usize, out: &mut Vec<AstItem>) {
    for item in items {
        match item {
            syn::Item::Mod(m) => {
                out.push(AstItem {
                    kind: "mod",
                    name: m.ident.to_string(),
                    line: m.ident.span().start().line,
                    depth,
                });
                if let Some((_, inner)) = &m.content {
                    collect_ast_items(inner, depth + 1, out);
                }
            }
            syn::Item::Struct(s) => out.push(AstItem {
                kind: "struct",
                name: s.ident.to_string(),
                line: s.ident.span().start().line,
                depth,
            }),
            syn::Item::Enum(e) => out.push(AstItem {
                kind: "enum",
                name: e.ident.to_string(),
                line: e.ident.span().start().line,
                depth,
            }),
            syn::Item::Trait(t) => out.push(AstItem {
                kind: "trait",
                name: t.ident.to_string(),
                line: t.ident.span().start().line,
                depth,
            }),
            syn::Item::Fn(f) => out.push(AstItem {
                kind: "fn",
                name: f.sig.ident.to_string(),
                line: f.sig.ident.span().start().line,
                depth,
            }),
            syn::Item::Impl(imp) => {
                for impl_item in &imp.items {
                    if let syn::ImplItem::Fn(f) = impl_item {
                        out.push(AstItem {
                            kind: "fn",
                            name: f.sig.ident.to_string(),
                            line: f.sig.ident.span().start().line,
                            depth: depth + 1,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

fn ast_kind_color(kind: &str, dark: bool) -> Color32 {
    if dark {
        match kind {
            "mod" => Color32::from_rgb(0xe5, 0xc0, 0x7b),
            "struct" => Color32::from_rgb(0x61, 0xaf, 0xef),
            "enum" => Color32::from_rgb(0xc6, 0x78, 0xdd),
            "trait" => Color32::from_rgb(0x56, 0xb6, 0xc2),
            _ => Color32::from_rgb(0x98, 0xc3, 0x79),
        }
    } else {
        match kind {
            "mod" => Color32::from_rgb(0xb7, 0x6b, 0x01),
            "struct" => Color32::from_rgb(0x40, 0x78, 0xbf),
            "enum" => Color32::from_rgb(0x98, 0x68, 0x01),
            "trait" => Color32::from_rgb(0x0e, 0x8a, 0x84),
            _ => Color32::from_rgb(0x50, 0xa1, 0x4f),
        }
    }
}

// ============================ 数据结构 ============================

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Settings {
    dark_mode: bool,
    last_workspace: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { dark_mode: true, last_workspace: None }
    }
}

fn settings_path() -> Option<PathBuf> {
    std::env::var("APPDATA")
        .ok()
        .map(|base| PathBuf::from(base).join("NovaRS").join("settings.json"))
}

fn load_settings() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(json) => serde_json::from_str::<Settings>(&json).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

#[derive(Default, Clone)]
struct SystemInfo {
    rustc_full: Option<String>,
    rustc_short: Option<String>,
    cargo_full: Option<String>,
    rust_analyzer: bool,
}

fn toolchain_summary(info: &SystemInfo) -> String {
    let rustc = info.rustc_full.clone().unwrap_or_else(|| "rustc 未检测到".to_string());
    let cargo = info.cargo_full.clone().unwrap_or_else(|| "cargo 未检测到".to_string());
    let ra = if info.rust_analyzer { "已安装" } else { "未安装" };
    format!("{} · {} · rust-analyzer: {}", rustc, cargo, ra)
}

fn detect_toolchain(toolchain: &Arc<Mutex<SystemInfo>>) {
    let rustc_full = run_rustc_version();
    let rustc_short = rustc_full
        .as_ref()
        .and_then(|s| s.split_whitespace().nth(1))
        .map(|s| s.to_string());
    let cargo_full = run_cargo_version();
    let rust_analyzer = which_exists("rust-analyzer");
    let mut guard = lock(toolchain);
    guard.rustc_full = rustc_full;
    guard.rustc_short = rustc_short;
    guard.cargo_full = cargo_full;
    guard.rust_analyzer = rust_analyzer;
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Splash,
    Main,
}

#[derive(Clone, Copy, PartialEq)]
enum RunMode {
    Run,
    Check,
}

#[derive(Clone)]
enum NameKind {
    NewFolder(PathBuf),
    Rename(PathBuf),
}

struct NameDialog {
    title: String,
    hint: String,
    value: String,
    error: Option<String>,
    kind: NameKind,
}

#[derive(Clone)]
struct AstItem {
    kind: &'static str,
    name: String,
    line: usize,
    depth: usize,
}

struct FileNode {
    name: String,
    path: PathBuf,
    is_dir: bool,
    children: Vec<usize>,
}

struct EditorTab {
    id: u64,
    path: PathBuf,
    title: String,
    text: String,
    dirty: bool,
    undo: Vec<String>,
    redo: Vec<String>,
    last_text: String,
    last_snapshot: Instant,
    breakpoints: HashSet<usize>,
    line_ending: &'static str,
}

fn is_ignored_entry(entry: &walkdir::DirEntry) -> bool {
    const IGNORED: [&str; 3] = [".git", "target", "node_modules"];
    entry.file_type().is_dir()
        && IGNORED.contains(&entry.file_name().to_string_lossy().as_ref())
}

fn build_tree(root: &Path) -> (Vec<FileNode>, Vec<usize>) {
    let mut nodes: Vec<FileNode> = Vec::new();
    let mut index: HashMap<PathBuf, usize> = HashMap::new();
    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root.display().to_string());
    nodes.push(FileNode { name: root_name, path: root.to_path_buf(), is_dir: true, children: Vec::new() });
    index.insert(root.to_path_buf(), 0);

    let walker = WalkDir::new(root).min_depth(1).sort_by(|a, b| {
        let a_dir = a.file_type().is_dir();
        let b_dir = b.file_type().is_dir();
        b_dir.cmp(&a_dir).then_with(|| a.file_name().cmp(b.file_name()))
    });

    for entry in walker
        .into_iter()
        .filter_entry(|e| !is_ignored_entry(e))
        .filter_map(|e| e.ok())
    {
        let path = entry.path().to_path_buf();
        let parent_idx = entry.path().parent().and_then(|p| index.get(p)).copied();
        if let Some(pidx) = parent_idx {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().is_dir();
            let idx = nodes.len();
            nodes.push(FileNode { name, path: path.clone(), is_dir, children: Vec::new() });
            nodes[pidx].children.push(idx);
            index.insert(path, idx);
        }
    }
    (nodes, vec![0])
}

// ============================ 应用主体 ============================

struct App {
    phase: Phase,
    splash_start: Instant,
    splash_duration: Duration,
    splash_centered: bool,
    toolchain: Arc<Mutex<SystemInfo>>,

    workspace: Option<PathBuf>,
    nodes: Vec<FileNode>,
    tree_root: Vec<usize>,

    tabs: Vec<EditorTab>,
    active_tab: Option<usize>,
    next_tab_id: u64,
    last_cursor_range: Option<(u64, (usize, usize))>,
    pending_selection: Option<(u64, usize, usize)>,
    cursor_line: usize,
    cursor_col: usize,
    scroll_to_line: Option<(u64, usize)>,

    dark_mode: bool,
    show_ast: bool,
    show_output: bool,
    about_open: bool,

    output_log: Arc<Mutex<String>>,
    cargo_child: Arc<Mutex<Option<Child>>>,

    ast_items: Vec<AstItem>,
    ast_error: Option<String>,
    ast_sig: Option<(u64, u64)>,

    name_dialog: Option<NameDialog>,
    project_dialog: Option<ProjectDialog>,
    toast: Option<(String, Instant)>,
    main_size_ok: bool,
    capture_dir: Option<PathBuf>,
    capture_seen: Option<SystemTime>,
    capture_name: Option<String>,

    lsp_slot: Arc<Mutex<Option<Arc<LspSession>>>>,
    lsp: Option<Arc<LspSession>>,
    ra_gate: Arc<AtomicBool>,
    completion: Option<CompletionPopup>,
    completion_anchor: Option<egui::Pos2>,
    force_completion: bool,
    popup_cmd: Option<u8>,
    auto_complete_pending: bool,
    ime_composing: bool,
    ime_commit_at: Option<Instant>,

    error_files: Arc<Mutex<HashSet<PathBuf>>>,
    terminal: TerminalState,
    search: Option<SearchState>,
    find: Option<FindState>,
    minimap_cache: Option<(u64, Vec<MiniRow>)>,
    scroll_to_offset: Option<f32>,
    editor_scroll_meta: Option<(f32, f32, f32, f32)>,
    match_highlight: Option<(u64, usize, usize)>,
    git_branch: Option<String>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = load_settings();
        setup_fonts(&cc.egui_ctx);
        cc.egui_ctx
            .set_visuals(if settings.dark_mode { Visuals::dark() } else { Visuals::light() });

        let toolchain = Arc::new(Mutex::new(SystemInfo::default()));
        let ra_gate = Arc::new(AtomicBool::new(false));
        let lsp_slot: Arc<Mutex<Option<Arc<LspSession>>>> = Arc::new(Mutex::new(None));
        // 工作区需在启动线程前确定（rust-analyzer 以其作为 LSP rootUri）
        let startup_workspace: Option<PathBuf> = settings
            .last_workspace
            .as_ref()
            .map(PathBuf::from)
            .filter(|p| p.is_dir());
        {
            let tc = toolchain.clone();
            let gate = ra_gate.clone();
            let slot = lsp_slot.clone();
            let ws = startup_workspace.clone();
            std::thread::spawn(move || {
                detect_toolchain(&tc);
                // rust-analyzer：释放嵌入的可执行文件到临时目录并启动 LSP 会话，
                // 任何失败都会置位 gate，使补全降级为内置引擎
                if let Some(session) = start_rust_analyzer(RA_EMBEDDED, ws, gate) {
                    *lock(&slot) = Some(session);
                }
            });
        }

        let mut app = App {
            phase: Phase::Splash,
            splash_start: Instant::now(),
            splash_duration: splash_duration_from_env(),
            splash_centered: false,
            toolchain,
            workspace: None,
            nodes: Vec::new(),
            tree_root: Vec::new(),
            tabs: Vec::new(),
            active_tab: None,
            next_tab_id: 1,
            last_cursor_range: None,
            pending_selection: None,
            cursor_line: 1,
            cursor_col: 1,
            scroll_to_line: None,
            dark_mode: settings.dark_mode,
            show_ast: false,
            show_output: false,
            about_open: false,
            output_log: Arc::new(Mutex::new(String::new())),
            cargo_child: Arc::new(Mutex::new(None)),
            ast_items: Vec::new(),
            ast_error: None,
            ast_sig: None,
            name_dialog: None,
            project_dialog: None,
            toast: None,
            main_size_ok: false,
            capture_dir: std::env::var("NOVARS_CAPTURE").ok().map(PathBuf::from),
            capture_seen: None,
            capture_name: None,
            lsp_slot,
            lsp: None,
            ra_gate,
            completion: None,
            completion_anchor: None,
            force_completion: false,
            popup_cmd: None,
            auto_complete_pending: false,
            ime_composing: false,
            ime_commit_at: None,
            error_files: Arc::new(Mutex::new(HashSet::new())),
            terminal: TerminalState::new(),
            search: None,
            find: None,
            minimap_cache: None,
            scroll_to_offset: None,
            editor_scroll_meta: None,
            match_highlight: None,
            git_branch: None,
        };
        if let Some(path) = startup_workspace {
            app.workspace = Some(path);
            app.refresh_tree();
        }
        if app.workspace.is_some() {
            app.git_branch = read_git_branch(app.workspace.as_deref());
        }
        app
    }

    fn save_settings(&self) {
        let Some(path) = settings_path() else { return };
        let settings = Settings {
            dark_mode: self.dark_mode,
            last_workspace: self.workspace.as_ref().map(|p| p.display().to_string()),
        };
        if let Ok(json) = serde_json::to_string_pretty(&settings) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, json);
        }
    }

    fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    // ---------- 迭代新增：快捷键 / 补全 / 搜索 / 终端 / 错误检查 ----------

    fn handle_global_shortcuts(&mut self, ctx: &egui::Context) {
        if self.phase != Phase::Main {
            return;
        }
        let key = |key: egui::Key, command: bool, shift: bool| {
            ctx.input(|i| {
                i.events.iter().any(|e| {
                    matches!(e, egui::Event::Key { key: k, pressed: true, modifiers: m, .. }
                        if *k == key && m.command == command && m.shift == shift)
                })
            })
        };

        // Ctrl+Space：手动触发补全（吞掉空格键，避免向编辑器插入空格）
        if key(Key::Space, true, false) {
            ctx.input_mut(|i| {
                i.events.retain(|e| {
                    !matches!(e, egui::Event::Key { key: Key::Space, pressed: true, modifiers: m, .. } if m.command)
                })
            });
            self.force_completion = true;
        }
        if key(Key::F5, false, false) {
            self.run_cargo(ctx, RunMode::Run);
        }
        if key(Key::F7, false, false) {
            self.run_cargo(ctx, RunMode::Check);
        }
        if key(Key::N, true, false) {
            self.menu_new_file();
        }
        if key(Key::O, true, false) {
            self.menu_open_folder();
        }
        if key(Key::S, true, true) {
            self.save_all(ctx);
        }
        if key(Key::W, true, false) {
            if let Some(i) = self.active_tab {
                self.request_close_tab(i);
            }
        }
        if key(Key::Backtick, true, false) {
            self.toggle_terminal(ctx);
        }
        if key(Key::F, true, true) {
            self.search = Some(SearchState {
                input: String::new(),
                case_sensitive: false,
                running: Arc::new(AtomicBool::new(false)),
                results: Arc::new(Mutex::new(Vec::new())),
            });
            self.completion = None;
        }
        if key(Key::F, true, false) {
            if self.find.is_none() {
                self.find = Some(FindState { input: String::new(), total: 0, index: 0 });
            }
        }
    }

    fn handle_popup_keys(&mut self, ctx: &egui::Context) {
        if self.completion.is_none() {
            return;
        }
        let mut cmd: Option<u8> = None;
        ctx.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Key { key, pressed: true, .. } => match key {
                    Key::ArrowDown => {
                        cmd = Some(0);
                        false
                    }
                    Key::ArrowUp => {
                        cmd = Some(1);
                        false
                    }
                    Key::Tab => {
                        cmd = Some(2);
                        false
                    }
                    Key::Escape => {
                        cmd = Some(3);
                        false
                    }
                    _ => true,
                },
                _ => true,
            })
        });
        self.popup_cmd = cmd;
    }

    /// 无补全弹窗时，Tab 插入 4 空格缩进；有选区时整块缩进（VS Code 风格）
    fn handle_tab_indent(&mut self, ctx: &egui::Context) {
        if self.completion.is_some() {
            return;
        }
        let tab_pressed = ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { key: Key::Tab, pressed: true, modifiers: m, .. } if !m.shift))
        });
        if !tab_pressed {
            return;
        }
        // 吞掉 Tab，避免 TextEdit 插入 \t
        ctx.input_mut(|i| {
            i.events
                .retain(|e| !matches!(e, egui::Event::Key { key: Key::Tab, pressed: true, .. }))
        });
        let Some(ti) = self.active_tab else { return };
        let tab_id = self.tabs[ti].id;
        let editor_id = Self::editor_id_of(tab_id);
        // 按下 Tab 的瞬间直接从编辑器状态读取当前选区（上一帧渲染后的真实值）
        let live_range = TextEditState::load(ctx, editor_id)
            .and_then(|st| st.cursor.char_range())
            .map(|r| (r.primary.index, r.secondary.index))
            .map(|(a, b)| (tab_id, (a, b)));
        #[cfg(debug_assertions)]
        eprintln!(
            "[tab] live_range={:?} last={:?} ime_composing={}",
            live_range, self.last_cursor_range, self.ime_composing
        );
        let selection = live_range.or(self.last_cursor_range);
        let Some((tab_id, (c1, c2))) = selection.filter(|(id, _)| *id == tab_id) else { return; };
        let tab_path = self.tabs[ti].path.clone();
        if c1 == c2 {
            // 无选区：光标处插入 4 空格
            let tab = &mut self.tabs[ti];
            tab.text = replace_char_range(&tab.text, c1, c1, "    ");
            tab.dirty = true;
            let tab_text = tab.text.clone();
            self.set_editor_cursor(ctx, tab_id, c1 + 4);
            if let Some(lsp) = &self.lsp {
                lsp.notify_did_change(&tab_path, &tab_text);
            }
            #[cfg(debug_assertions)]
            eprintln!("[tab] no-selection indent: cursor={}", c1 + 4);
        } else {
            // 有选区：选区内每行行首缩进 4 空格，并保持选区
            let text = self.tabs[ti].text.clone();
            let (new_text, new_start, new_end) = indent_lines(&text, c1.min(c2), c1.max(c2));
            {
                let tab = &mut self.tabs[ti];
                tab.text = new_text;
                tab.dirty = true;
            }
            self.set_editor_selection(tab_id, new_start, new_end);
            let tab_path = self.tabs[ti].path.clone();
            let tab_text = self.tabs[ti].text.clone();
            if let Some(lsp) = &self.lsp {
                lsp.notify_did_change(&tab_path, &tab_text);
            }
            #[cfg(debug_assertions)]
            eprintln!("[tab] selection indent: new_start={} new_end={}", new_start, new_end);
        }
    }

    fn completion_word_prefix(text: &str, char_idx: usize) -> (usize, String) {
        let byte_idx = char_to_byte(text, char_idx);
        let start_byte = text[..byte_idx]
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
            .map(|(b, _)| b)
            .last();
        match start_byte {
            Some(b) => {
                let start_char = text[..b].chars().count();
                let prefix = slice_by_char_range(text, start_char, char_idx);
                (start_char, prefix)
            }
            None => (char_idx, String::new()),
        }
    }

    /// 打开补全弹窗。force=false（打字自动触发）时前缀为空直接关闭；
    /// force=true（Ctrl+Space 手动）时显示全量候选。
    fn open_completion(&mut self, force: bool) {
        let Some(ti) = self.active_tab else { return };
        let Some((tab_id, (char_idx, _))) = self.last_cursor_range else { return };
        if self.tabs.get(ti).map(|t| t.id) != Some(tab_id) {
            return;
        }
        let tab_text = self.tabs[ti].text.clone();
        let tab_path = self.tabs[ti].path.clone();
        let (base_char, prefix) = Self::completion_word_prefix(&tab_text, char_idx);
        if !force && prefix.is_empty() {
            self.completion = None;
            return;
        }
        let items = builtin_completions(&tab_text, &prefix, self.workspace.as_deref());
        let mut gen = -1i64;
        if let Some(lsp) = &self.lsp {
            if lsp.ready.load(Ordering::Relaxed) && lsp.alive.load(Ordering::Relaxed) {
                let (line, col) = line_col_of_char_index(&tab_text, char_idx);
                let uri = path_to_uri(&tab_path);
                gen = lsp.request_completion(&uri, line, col.saturating_sub(1));
            }
        }
        if gen < 0 && items.is_empty() {
            self.completion = None;
            return;
        }
        self.completion = Some(CompletionPopup { items, selected: 0, gen, base_char });
    }

    fn apply_completion(&mut self, ctx: &egui::Context) {
        let Some(popup) = self.completion.clone() else { return };
        let Some(item) = popup.items.get(popup.selected) else { return };
        let insert = item.label.trim_end_matches('(').to_string();
        let Some(ti) = self.active_tab else { return };
        let Some((tab_id, (char_idx, _))) = self.last_cursor_range else { return };
        if self.tabs.get(ti).map(|t| t.id) != Some(tab_id) {
            return;
        }
        {
            let tab = &mut self.tabs[ti];
            let new_text = replace_char_range(&tab.text, popup.base_char, char_idx, &insert);
            let new_cursor = popup.base_char + insert.chars().count();
            tab.text = new_text;
            tab.dirty = true;
            self.set_editor_cursor(ctx, tab_id, new_cursor);
        }
        let tab_path = self.tabs[ti].path.clone();
        let tab_text = self.tabs[ti].text.clone();
        if let Some(lsp) = &self.lsp {
            lsp.notify_did_change(&tab_path, &tab_text);
        }
        self.completion = None;
        self.set_toast(format!("已补全: {}", insert));
    }

    fn save_all(&mut self, ctx: &egui::Context) {
        let mut saved = 0usize;
        for i in 0..self.tabs.len() {
            if self.tabs[i].dirty {
                let path = self.tabs[i].path.clone();
                let text = self.tabs[i].text.clone();
                match std::fs::write(&path, &text) {
                    Ok(()) => {
                        self.tabs[i].dirty = false;
                        saved += 1;
                    }
                    Err(e) => self.set_toast(format!("保存失败 {}: {}", path.display(), e)),
                }
            }
        }
        if saved > 0 {
            self.set_toast(format!("已保存全部 {} 个修改的文件", saved));
            self.spawn_error_check(ctx);
        } else {
            self.set_toast("没有待保存的修改");
        }
    }

    fn spawn_error_check(&mut self, ctx: &egui::Context) {
        let Some(ws) = self.workspace.clone() else { return };
        let set = self.error_files.clone();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let mut errors: HashSet<PathBuf> = HashSet::new();
            if let Ok(output) = std::process::Command::new("cargo")
                .args(["check", "--message-format=json"])
                .current_dir(&ws)
                .creation_flags(CREATE_NO_WINDOW)
                .output()
            {
                for line in String::from_utf8_lossy(&output.stdout).lines() {
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
                    if v.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
                        continue;
                    }
                    if v["message"]["level"].as_str() != Some("error") {
                        continue;
                    }
                    if let Some(spans) = v["message"]["spans"].as_array() {
                        for span in spans {
                            if let Some(file) = span["file_name"].as_str() {
                                errors.insert(PathBuf::from(file));
                            }
                        }
                    }
                }
            }
            *lock(&set) = errors;
            ctx2.request_repaint();
        });
    }

    fn toggle_terminal(&mut self, ctx: &egui::Context) {
        self.terminal.visible = !self.terminal.visible;
        if self.terminal.visible {
            let ws = self.workspace.clone();
            self.terminal.ensure_spawned(ws.as_deref(), ctx);
        }
    }

    fn start_project_search(&mut self, ctx: &egui::Context) {
        let Some(ws) = self.workspace.clone() else {
            self.set_toast("请先打开一个文件夹");
            return;
        };
        let Some(search) = &mut self.search else { return };
        let query = search.input.trim().to_string();
        if query.is_empty() {
            self.set_toast("请输入搜索关键词");
            return;
        }
        if search.running.swap(true, Ordering::Relaxed) {
            return;
        }
        let case_sensitive = search.case_sensitive;
        let results = search.results.clone();
        let running = search.running.clone();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let needle = if case_sensitive { query.clone() } else { query.to_lowercase() };
            let mut hits: Vec<SearchHit> = Vec::new();
            let walker = WalkDir::new(&ws)
                .min_depth(1)
                .into_iter()
                .filter_entry(|e| !is_ignored_entry(e))
                .filter_map(|e| e.ok());
            for entry in walker {
                if hits.len() >= 1000 {
                    break;
                }
                if entry.file_type().is_dir() {
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(entry.path()) else { continue };
                if content.contains('\0') {
                    continue;
                }
                'lines: for (idx, line) in content.lines().enumerate() {
                    let matched = if case_sensitive {
                        line.contains(&query)
                    } else {
                        line.to_lowercase().contains(&needle)
                    };
                    if matched {
                        hits.push(SearchHit {
                            path: entry.path().to_path_buf(),
                            line_no: idx + 1,
                            text: line.trim().chars().take(160).collect(),
                        });
                        if hits.len() >= 1000 {
                            break 'lines;
                        }
                    }
                }
            }
            *lock(&results) = hits;
            running.store(false, Ordering::Relaxed);
            ctx2.request_repaint();
        });
    }

    fn find_next_in_file(&mut self, ctx: &egui::Context) {
        let Some(query) = self.find.as_ref().map(|f| f.input.clone()) else { return };
        if query.is_empty() {
            return;
        }
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let text = self.tabs[ti].text.clone();
        let lower_text = text.to_lowercase();
        let needle = query.to_lowercase();
        let mut char_positions: Vec<usize> = Vec::new();
        let mut byte_from = 0usize;
        while let Some(rel) = lower_text[byte_from..].find(&needle) {
            let byte_at = byte_from + rel;
            char_positions.push(text[..byte_at].chars().count());
            byte_from = byte_at + needle.len().max(1);
            if char_positions.len() >= 5000 {
                break;
            }
        }
        let total = char_positions.len();
        let Some(find) = self.find.as_mut() else { return };
        find.total = total;
        if total == 0 {
            self.set_toast(format!("未找到 “{}”", query));
            return;
        }
        let idx = find.index % total;
        find.index = find.index.wrapping_add(1);
        let char_at = char_positions[idx];
        let line = line_col_of_char_index(&text, char_at).0;
        let tab_id = self.tabs[ti].id;
        self.set_editor_cursor(ctx, tab_id, char_at);
        self.scroll_to_line = Some((tab_id, line));
        // 匹配文本高亮（编辑器内黄色标注）
        self.match_highlight = Some((tab_id, char_at, char_at + query.chars().count()));
    }

    // ---------- 文件与工作区 ----------

    fn refresh_tree(&mut self) {
        if let Some(ws) = self.workspace.clone() {
            let (nodes, root) = build_tree(&ws);
            self.nodes = nodes;
            self.tree_root = root;
        } else {
            self.nodes.clear();
            self.tree_root.clear();
        }
    }

    fn set_workspace(&mut self, path: PathBuf) {
        self.workspace = Some(path);
        self.refresh_tree();
        self.git_branch = read_git_branch(self.workspace.as_deref());
        self.save_settings();
    }

    fn open_file_in_tab(&mut self, path: PathBuf) {
        if let Some(i) = self.tabs.iter().position(|t| t.path == path) {
            self.active_tab = Some(i);
            self.last_cursor_range = None;
            self.ast_sig = None;
            self.completion = None;
            return;
        }
        match std::fs::read_to_string(&path) {
            Ok(raw) => {
                let line_ending: &'static str = if raw.contains("\r\n") { "CRLF" } else { "LF" };
                let content = raw.replace("\r\n", "\n");
                let title = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                let id = self.next_tab_id;
                self.next_tab_id += 1;
                self.tabs.push(EditorTab {
                    id,
                    path: path.clone(),
                    title,
                    text: content.clone(),
                    dirty: false,
                    undo: Vec::new(),
                    redo: Vec::new(),
                    last_text: content,
                    last_snapshot: Instant::now(),
                    breakpoints: HashSet::new(),
                    line_ending,
                });
                self.active_tab = Some(self.tabs.len() - 1);
                self.last_cursor_range = None;
                self.ast_sig = None;
                self.completion = None;
                if let Some(lsp) = &self.lsp {
                    if lsp.ready.load(Ordering::Relaxed) {
                        let text = self.tabs[self.tabs.len() - 1].text.clone();
                        lsp.notify_did_open(&path, &text);
                    }
                }
            }
            Err(e) => self.set_toast(format!("无法打开文件: {}", e)),
        }
    }

    fn save_active(&mut self, ctx: &egui::Context) {
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let result = {
            let tab = &self.tabs[ti];
            std::fs::write(&tab.path, &tab.text)
        };
        match result {
            Ok(()) => {
                self.tabs[ti].dirty = false;
                self.set_toast(format!("已保存: {}", self.tabs[ti].path.display()));
                self.spawn_error_check(ctx);
            }
            Err(e) => self.set_toast(format!("保存失败: {}", e)),
        }
    }

    fn save_tab_at(&mut self, idx: usize) {
        let result = {
            let tab = &self.tabs[idx];
            std::fs::write(&tab.path, &tab.text)
        };
        match result {
            Ok(()) => self.tabs[idx].dirty = false,
            Err(e) => self.set_toast(format!("保存失败: {}", e)),
        }
    }

    fn remove_tab_at(&mut self, idx: usize) {
        self.tabs.remove(idx);
        self.active_tab = match (self.active_tab, self.tabs.len()) {
            (_, 0) => None,
            (Some(a), n) if a >= n => Some(n - 1),
            (a, _) => a,
        };
        self.last_cursor_range = None;
        self.ast_sig = None;
        self.completion = None;
    }

    fn request_close_tab(&mut self, idx: usize) {
        let (title, dirty) = {
            let tab = &self.tabs[idx];
            (tab.title.clone(), tab.dirty)
        };
        if dirty {
            let desc = format!("“{}” 有未保存的修改，是否保存？", title);
            let confirmed = rfd::MessageDialog::new()
                .set_title("关闭标签页")
                .set_description(&desc)
                .set_buttons(rfd::MessageButtons::YesNo)
                .set_level(rfd::MessageLevel::Info)
                .show();
            if matches!(confirmed, rfd::MessageDialogResult::Yes) {
                self.save_tab_at(idx);
            }
        }
        self.remove_tab_at(idx);
        self.set_toast(format!("已关闭 {}", title));
    }

    fn close_tabs_under(&mut self, target: &Path) {
        self.tabs.retain(|t| !(t.path == target || t.path.starts_with(target)));
        self.active_tab = match (self.active_tab, self.tabs.len()) {
            (_, 0) => None,
            (Some(a), n) if a >= n => Some(n - 1),
            (a, _) => a,
        };
        self.last_cursor_range = None;
        self.ast_sig = None;
        self.completion = None;
    }

    fn delete_path(&mut self, path: PathBuf) {
        let is_dir = path.is_dir();
        let kind = if is_dir { "文件夹" } else { "文件" };
        let desc = format!(
            "确定要删除这个{}吗？\n{}\n\n此操作不可撤销。",
            kind,
            path.display()
        );
        let confirmed = rfd::MessageDialog::new()
            .set_title("删除确认")
            .set_description(&desc)
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_level(rfd::MessageLevel::Warning)
            .show();
        if matches!(confirmed, rfd::MessageDialogResult::Yes) {
            let result = if is_dir {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            match result {
                Ok(()) => {
                    self.set_toast(format!("已删除 {}", path.display()));
                    self.refresh_tree();
                    self.close_tabs_under(&path);
                }
                Err(e) => self.set_toast(format!("删除失败: {}", e)),
            }
        }
    }

    // ---------- 菜单动作 ----------

    fn menu_new_project(&mut self) {
        // 第一步：选择创建位置（父目录）
        let Some(location) = rfd::FileDialog::new()
            .set_title("选择新项目的创建位置")
            .pick_folder()
        else {
            return;
        };
        self.project_dialog = Some(ProjectDialog {
            name: String::new(),
            location,
            template: 1,
            error: None,
        });
    }

    fn create_project(&mut self, dialog: ProjectDialog) -> Option<ProjectDialog> {
        let name = dialog.name.trim().to_string();
        if name.is_empty() {
            let mut d = dialog;
            d.error = Some("项目名称不能为空".to_string());
            return Some(d);
        }
        if name.contains('\\') || name.contains('/') {
            let mut d = dialog;
            d.error = Some("项目名称不能包含路径分隔符".to_string());
            return Some(d);
        }
        let package = sanitize_package_name(&name);
        let project_dir = dialog.location.join(&name);
        if project_dir.exists() {
            let mut d = dialog;
            d.error = Some(format!("目录已存在: {}", project_dir.display()));
            return Some(d);
        }
        let src_dir = project_dir.join("src");
        if std::fs::create_dir_all(&src_dir).is_err() {
            let mut d = dialog;
            d.error = Some("创建目录失败".to_string());
            return Some(d);
        }
        let cargo = template_cargo_toml(&package);
        let main_rs = template_main_rs(dialog.template);
        if std::fs::write(project_dir.join("Cargo.toml"), cargo).is_err()
            || std::fs::write(src_dir.join("main.rs"), main_rs).is_err()
        {
            let _ = std::fs::remove_dir_all(&project_dir);
            let mut d = dialog;
            d.error = Some("写入项目文件失败".to_string());
            return Some(d);
        }
        self.set_workspace(project_dir);
        self.set_toast(format!(
            "已创建项目 “{}”（模板: {}）",
            name, PROJECT_TEMPLATES[dialog.template]
        ));
        None
    }

    fn draw_project_dialog(&mut self, ctx: &egui::Context) {
        let mut dialog = match self.project_dialog.take() {
            Some(d) => d,
            None => return,
        };
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            return; // Esc 关闭
        }
        let mut cancelled = false;
        let mut confirmed = false;
        let mut change_location = false;
        let mut win_open = true;
        egui::Window::new("新建项目")
            .open(&mut win_open)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("项目名称:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut dialog.name)
                            .desired_width(240.0)
                            .hint_text("例如: my_first_rust"),
                    );
                    // 中文输入法组合中按回车是“上屏候选”，不视为确认
                    let ime_busy = self.ime_composing
                        || self
                            .ime_commit_at
                            .map(|t| t.elapsed() < Duration::from_millis(150))
                            .unwrap_or(false);
                    if resp.lost_focus() && !ime_busy && ctx.input(|i| i.key_pressed(Key::Enter)) {
                        confirmed = true;
                    }
                });
                ui.add_space(6.0);
                ui.label("选择模板:");
                for (idx, tname) in PROJECT_TEMPLATES.iter().enumerate() {
                    ui.radio_value(&mut dialog.template, idx, *tname);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("创建位置:");
                    ui.label(
                        RichText::new(dialog.location.display().to_string())
                            .size(11.0)
                            .weak(),
                    );
                    if ui.small_button("更改...").clicked() {
                        change_location = true;
                    }
                });
                if let Some(err) = &dialog.error {
                    ui.label(RichText::new(err.as_str()).small().color(BRAND_RED));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("创建项目").clicked() {
                        confirmed = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });
        if change_location {
            if let Some(dir) =
                rfd::FileDialog::new().set_title("选择新项目的创建位置").pick_folder()
            {
                dialog.location = dir;
                dialog.error = None;
            }
        }
        if cancelled {
            win_open = false;
        }
        if confirmed {
            let kept = self.create_project(dialog);
            if let Some(d) = kept {
                self.project_dialog = Some(d);
            }
        } else if win_open {
            self.project_dialog = Some(dialog);
        }
    }

    fn menu_new_file(&mut self) {
        if self.workspace.is_none() {
            self.set_toast("请先打开一个文件夹（文件 → 打开文件夹）");
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .set_title("新建文件")
            .set_file_name("未命名.rs")
            .save_file()
        {
            if path.exists() {
                self.open_file_in_tab(path);
            } else {
                match std::fs::write(&path, "") {
                    Ok(()) => {
                        self.set_toast(format!("已创建: {}", path.display()));
                        self.refresh_tree();
                        self.open_file_in_tab(path);
                    }
                    Err(e) => self.set_toast(format!("创建失败: {}", e)),
                }
            }
        }
    }

    fn menu_new_folder(&mut self) {
        if self.workspace.is_none() {
            self.set_toast("请先打开一个文件夹（文件 → 打开文件夹）");
            return;
        }
        if let Some(parent) = rfd::FileDialog::new()
            .set_title("选择新建文件夹的位置")
            .pick_folder()
        {
            self.name_dialog = Some(NameDialog {
                title: "新建文件夹".to_string(),
                hint: format!("在 {} 下创建新文件夹，请输入名称：", parent.display()),
                value: "新建文件夹".to_string(),
                error: None,
                kind: NameKind::NewFolder(parent),
            });
        }
    }

    fn menu_open_folder(&mut self) {
        if let Some(folder) = rfd::FileDialog::new().set_title("打开文件夹").pick_folder() {
            self.set_workspace(folder);
            self.set_toast("文件夹已打开");
        }
    }

    fn exit_app(&mut self, ctx: &egui::Context) {
        {
            let mut guard = lock(&self.cargo_child);
            if let Some(child) = guard.as_mut() {
                let _ = child.kill();
            }
        }
        if self.terminal.child.is_some() {
            self.terminal.kill();
        }
        self.save_settings();
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    // ---------- 编辑动作 ----------

    fn editor_id_of(tab_id: u64) -> Id {
        Id::new(("novars_editor", tab_id))
    }

    fn set_editor_cursor(&mut self, _ctx: &egui::Context, tab_id: u64, char_idx: usize) {
        // 渲染后二次修正（draw_editor 尾部应用）：直接 store 会被同帧渲染消费
        self.pending_selection = Some((tab_id, char_idx, char_idx));
        self.last_cursor_range = Some((tab_id, (char_idx, char_idx)));
    }

    fn set_editor_selection(&mut self, tab_id: u64, start: usize, end: usize) {
        self.pending_selection = Some((tab_id, start.min(end), start.max(end)));
        self.last_cursor_range = Some((tab_id, (start.min(end), start.max(end))));
    }

    fn track_undo(&mut self, ti: usize) {
        let changed = self.tabs[ti].text != self.tabs[ti].last_text;
        if changed {
            {
                let tab = &mut self.tabs[ti];
                let should_push =
                    tab.last_snapshot.elapsed() >= Duration::from_millis(600) || tab.undo.is_empty();
                if should_push {
                    tab.undo.push(tab.last_text.clone());
                    if tab.undo.len() > 300 {
                        tab.undo.remove(0);
                    }
                    tab.last_snapshot = Instant::now();
                }
                tab.redo.clear();
                tab.last_text = tab.text.clone();
                tab.dirty = true;
            }
            self.auto_complete_pending = true;
            self.match_highlight = None;
            if let Some(lsp) = &self.lsp {
                if lsp.ready.load(Ordering::Relaxed) {
                    let tab = &self.tabs[ti];
                    lsp.notify_did_change(&tab.path, &tab.text);
                }
            }
        }
    }

    fn edit_undo(&mut self) {
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let popped = self.tabs[ti].undo.pop();
        match popped {
            Some(prev) => {
                let tab = &mut self.tabs[ti];
                tab.redo.push(tab.text.clone());
                tab.text = prev;
                tab.last_text = tab.text.clone();
                tab.dirty = true;
                self.set_toast("已撤销");
            }
            None => self.set_toast("没有可撤销的操作"),
        }
    }

    fn edit_redo(&mut self) {
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let popped = self.tabs[ti].redo.pop();
        match popped {
            Some(next) => {
                let tab = &mut self.tabs[ti];
                tab.undo.push(tab.text.clone());
                tab.text = next;
                tab.last_text = tab.text.clone();
                tab.dirty = true;
                self.set_toast("已重做");
            }
            None => self.set_toast("没有可重做的操作"),
        }
    }

    fn edit_copy(&mut self, ctx: &egui::Context) {
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let Some((tid, (a, b))) = self.last_cursor_range else {
            self.set_toast("没有选中的文本");
            return;
        };
        if tid != self.tabs[ti].id || b <= a {
            self.set_toast("没有选中的文本");
            return;
        }
        let content = slice_by_char_range(&self.tabs[ti].text, a, b);
        ctx.copy_text(content);
        self.set_toast("已复制到剪贴板");
    }

    fn edit_cut(&mut self, ctx: &egui::Context) {
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let Some((tid, (a, b))) = self.last_cursor_range else {
            self.set_toast("没有选中的文本");
            return;
        };
        if tid != self.tabs[ti].id || b <= a {
            self.set_toast("没有选中的文本");
            return;
        }
        let removed = slice_by_char_range(&self.tabs[ti].text, a, b);
        ctx.copy_text(removed);
        let new_text = replace_char_range(&self.tabs[ti].text, a, b, "");
        self.tabs[ti].text = new_text;
        self.tabs[ti].dirty = true;
        let tab_id = tid;
        self.set_editor_cursor(ctx, tab_id, a.min(b));
        self.set_toast("已剪切");
    }

    fn edit_paste(&mut self, ctx: &egui::Context) {
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let Some(clip) = read_clipboard_text() else {
            self.set_toast("无法读取剪贴板内容");
            return;
        };
        let tab_id = self.tabs[ti].id;
        let (a, b) = match self.last_cursor_range {
            Some((tid, (a, b))) if tid == tab_id => (a.min(b), a.max(b)),
            _ => {
                let end = self.tabs[ti].text.chars().count();
                (end, end)
            }
        };
        let new_text = replace_char_range(&self.tabs[ti].text, a, b, &clip);
        let new_cursor = a + clip.chars().count();
        self.tabs[ti].text = new_text;
        self.tabs[ti].dirty = true;
        self.set_editor_cursor(ctx, tab_id, new_cursor);
        self.set_toast("已粘贴");
    }

    fn edit_select_all(&mut self, _ctx: &egui::Context) {
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let (tab_id, count) = {
            let tab = &self.tabs[ti];
            (tab.id, tab.text.chars().count())
        };
        self.set_editor_selection(tab_id, 0, count);
    }

    fn jump_to_line(&mut self, ctx: &egui::Context, line: usize) {
        let Some(ti) = self.active_tab else { return };
        let tab_id = self.tabs[ti].id;
        let char_idx = line_start_char_index(&self.tabs[ti].text, line);
        self.set_editor_cursor(ctx, tab_id, char_idx);
        self.scroll_to_line = Some((tab_id, line));
    }

    // ---------- 运行 ----------

    fn run_cargo(&mut self, ctx: &egui::Context, mode: RunMode) {
        let Some(ws) = self.workspace.clone() else {
            self.set_toast("请先打开一个 Rust 项目文件夹");
            return;
        };
        if !ws.join("Cargo.toml").is_file() {
            self.set_toast("当前文件夹不是 Cargo 项目（缺少 Cargo.toml）");
            return;
        }
        let already_running = lock(&self.cargo_child).is_some();
        if already_running {
            self.set_toast("已有任务正在运行，请先终止");
            return;
        }
        {
            let mut log = lock(&self.output_log);
            log.clear();
            log.push_str(match mode {
                RunMode::Run => "$ cargo run\n",
                RunMode::Check => "$ cargo check\n",
            });
        }
        self.show_output = true;
        let mut command = std::process::Command::new("cargo");
        match mode {
            RunMode::Run => {
                command.arg("run");
            }
            RunMode::Check => {
                command.arg("check");
            }
        }
        command
            .current_dir(&ws)
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        match command.spawn() {
            Ok(child) => {
                *lock(&self.cargo_child) = Some(child);
                let child_arc = self.cargo_child.clone();
                let log_arc = self.output_log.clone();
                {
                    let mut guard = lock(&child_arc);
                    if let Some(c) = guard.as_mut() {
                        let stdout = c.stdout.take();
                        spawn_reader(stdout, log_arc.clone(), ctx.clone());
                        let stderr = c.stderr.take();
                        spawn_reader(stderr, log_arc.clone(), ctx.clone());
                    }
                }
                let ws2 = ws.clone();
                let error_files = self.error_files.clone();
                let ctx2 = ctx.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(200));
                    let mut guard = lock(&child_arc);
                    match guard.as_mut() {
                        Some(c) => match c.try_wait() {
                            Ok(Some(status)) => {
                                lock(&log_arc).push_str(&format!("\n[进程已退出: {}]\n", status));
                                *guard = None;
                                ctx2.request_repaint();
                                // 编译错误 → 文件树红色标记（cargo check --message-format=json）
                                if let Ok(output) = std::process::Command::new("cargo")
                                    .args(["check", "--message-format=json"])
                                    .current_dir(&ws2)
                                    .creation_flags(CREATE_NO_WINDOW)
                                    .output()
                                {
                                    let mut errors: HashSet<PathBuf> = HashSet::new();
                                    for line in String::from_utf8_lossy(&output.stdout).lines() {
                                        let Ok(v) =
                                            serde_json::from_str::<serde_json::Value>(line)
                                        else {
                                            continue;
                                        };
                                        if v.get("reason").and_then(|r| r.as_str())
                                            != Some("compiler-message")
                                        {
                                            continue;
                                        }
                                        if v["message"]["level"].as_str() != Some("error") {
                                            continue;
                                        }
                                        if let Some(spans) = v["message"]["spans"].as_array() {
                                            for span in spans {
                                                if let Some(file) = span["file_name"].as_str() {
                                                    errors.insert(PathBuf::from(file));
                                                }
                                            }
                                        }
                                    }
                                    *lock(&error_files) = errors;
                                }
                                ctx2.request_repaint();
                                break;
                            }
                            Ok(None) => {}
                            Err(_) => {
                                *guard = None;
                                break;
                            }
                        },
                        None => break,
                    }
                });
                self.set_toast(match mode {
                    RunMode::Run => "已启动 cargo run",
                    RunMode::Check => "已启动 cargo check",
                });
            }
            Err(e) => self.set_toast(format!("启动 cargo 失败: {}", e)),
        }
    }

    fn terminate_cargo(&mut self) {
        let killed = {
            let mut guard = lock(&self.cargo_child);
            match guard.as_mut() {
                Some(child) => {
                    let _ = child.kill();
                    true
                }
                None => false,
            }
        };
        if killed {
            lock(&self.output_log).push_str("\n[已请求终止进程]\n");
            self.set_toast("已发送终止信号");
        } else {
            self.set_toast("没有正在运行的任务");
        }
    }

    // ---------- 名称对话框 ----------

    fn confirm_name_dialog(&mut self, dialog: NameDialog) -> Option<NameDialog> {
        let name = dialog.value.trim().to_string();
        if name.is_empty() {
            let mut d = dialog;
            d.error = Some("名称不能为空".to_string());
            return Some(d);
        }
        if name.contains('\\') || name.contains('/') {
            let mut d = dialog;
            d.error = Some("名称不能包含路径分隔符".to_string());
            return Some(d);
        }
        match dialog.kind {
            NameKind::NewFolder(ref parent) => {
                let target = parent.join(&name);
                match std::fs::create_dir_all(&target) {
                    Ok(()) => {
                        self.set_toast(format!("已创建文件夹: {}", target.display()));
                        self.refresh_tree();
                        None
                    }
                    Err(e) => {
                        let mut d = dialog;
                        d.error = Some(format!("创建失败: {}", e));
                        Some(d)
                    }
                }
            }
            NameKind::Rename(ref old) => {
                let Some(parent) = old.parent().map(|p| p.to_path_buf()) else {
                    let mut d = dialog;
                    d.error = Some("无效路径".to_string());
                    return Some(d);
                };
                let target = parent.join(&name);
                match std::fs::rename(&old, &target) {
                    Ok(()) => {
                        for tab in self.tabs.iter_mut() {
                            if tab.path == *old {
                                tab.path = target.clone();
                                tab.title = target
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| name.clone());
                            }
                        }
                        self.set_toast(format!("已重命名为: {}", name));
                        self.refresh_tree();
                        None
                    }
                    Err(e) => {
                        let mut d = dialog;
                        d.error = Some(format!("重命名失败: {}", e));
                        Some(d)
                    }
                }
            }
        }
    }

    // ---------- 绘制：启动画面 ----------

    fn draw_splash(&mut self, ctx: &egui::Context) {
        if !self.splash_centered {
            self.splash_centered = true;
            center_window(ctx, SPLASH_SIZE);
        }
        ctx.request_repaint_after(Duration::from_millis(16));
        let elapsed = self.splash_start.elapsed();
        // 启动门控：主窗口渲染（本帧即证明）+ 文件树同步构建 + rust-analyzer 启动/降级完成
        let ra_done = self.ra_gate.load(Ordering::Relaxed);
        let all_ready = ra_done && elapsed >= SPLASH_MIN_VISIBLE;
        let force = elapsed >= SPLASH_FORCE_DEADLINE;
        let total_secs = self.splash_duration.as_secs_f64();
        let frac = if ra_done {
            (elapsed.as_secs_f64() / total_secs).clamp(0.0, 1.0) as f32
        } else {
            ((elapsed.as_secs_f64() / SPLASH_FORCE_DEADLINE.as_secs_f64()) * 0.9)
                .clamp(0.0, 0.9) as f32
        };
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                // 深色渐变背景（#1a1a2e → #16213e → #0f3460，顶点色网格平滑渐变）
                {
                    let mut mesh = egui::Mesh::default();
                    let mid_y = (rect.top() + rect.bottom()) * 0.5;
                    let white_uv = egui::Pos2::ZERO;
                    let corners = [
                        (rect.left_top(), SPLASH_C1),
                        (rect.right_top(), SPLASH_C1),
                        (pos2(rect.left(), mid_y), SPLASH_C2),
                        (pos2(rect.right(), mid_y), SPLASH_C2),
                        (rect.left_bottom(), SPLASH_C3),
                        (rect.right_bottom(), SPLASH_C3),
                    ];
                    for (pos, color) in corners {
                        mesh.colored_vertex(pos, color);
                        if let Some(vertex) = mesh.vertices.last_mut() {
                            vertex.uv = white_uv;
                        }
                    }
                    mesh.indices
                        .extend_from_slice(&[0, 2, 1, 1, 2, 3, 2, 4, 3, 3, 4, 5]);
                    ui.painter().add(egui::Shape::mesh(mesh));
                }
                // 斜向流动线条（品牌红，30% 透明度，随时间偏移；始终穿过窗口中部）
                let time = ctx.input(|i| i.time) as f32;
                {
                    let painter = ui.painter();
                    for k in 0..SPLASH_FLOW_LINES {
                        let phase = (time * 0.12 + k as f32 / SPLASH_FLOW_LINES as f32) % 1.0;
                        let slope = -0.38 - k as f32 * 0.07;
                        let center_y = rect.top() + (phase * 1.4 - 0.2) * rect.height();
                        let intercept = center_y - slope * rect.width() * 0.5;
                        let p1 = pos2(rect.left(), intercept);
                        let p2 = pos2(rect.right(), intercept + slope * rect.width());
                        painter.line_segment(
                            [p1, p2],
                            Stroke::new(
                                2.0_f32,
                                Color32::from_rgba_unmultiplied(0xe9, 0x45, 0x60, 77),
                            ),
                        );
                    }
                }
                // Logo 与标题
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("🦀✨").size(48.0));
                    ui.add_space(4.0);
                    ui.label(RichText::new(APP_FULL_NAME).size(28.0).strong().color(Color32::WHITE));
                    ui.add_space(2.0);
                    ui.label(RichText::new(APP_VERSION).size(14.0).color(BRAND_RED));
                    ui.add_space(16.0);
                    let idx = ((elapsed.as_millis() / 500) % 3) as usize;
                    ui.label(
                        RichText::new(SPLASH_LOAD_TEXTS[idx])
                            .size(13.0)
                            .color(Color32::from_rgb(0xc8, 0xd3, 0xe8)),
                    );
                    let info = { toolchain_summary(&lock(&self.toolchain)) };
                    ui.label(RichText::new(info).size(11.0).weak());
                    let lsp_ready = self.lsp.as_ref().map(|s| s.ready.load(Ordering::Relaxed)).unwrap_or(false);
                    let ra_text = if lsp_ready {
                        "rust-analyzer 智能补全已就绪"
                    } else if ra_done {
                        "内置补全引擎已就绪"
                    } else {
                        "正在启动 rust-analyzer 智能补全引擎..."
                    };
                    ui.label(RichText::new(ra_text).size(11.0).weak());
                    ui.add_space(10.0);
                    // 进度条 300x3
                    let (bar_rect, _) = ui.allocate_exact_size(Vec2::new(300.0, 3.0), Sense::hover());
                    let painter = ui.painter();
                    painter.rect_filled(
                        bar_rect,
                        1.5,
                        Color32::from_rgba_unmultiplied(255, 255, 255, 30),
                    );
                    let fill = Rect::from_min_size(bar_rect.min, Vec2::new(300.0 * frac, 3.0));
                    painter.rect_filled(fill, 1.5, BRAND_RED);
                });
                if all_ready || force {
                    self.phase = Phase::Main;
                    ctx.send_viewport_cmd(ViewportCommand::Decorations(true));
                    ctx.send_viewport_cmd(ViewportCommand::Resizable(true));
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(MAIN_SIZE));
                    ctx.send_viewport_cmd(ViewportCommand::Title(APP_FULL_NAME.to_string()));
                    center_window(ctx, MAIN_SIZE);
                }
            });
    }

    // ---------- 绘制：菜单栏 ----------

    fn draw_menu_bar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("文件", |ui| {
                if ui.button("新建项目...").clicked() {
                    ui.close_menu();
                    self.menu_new_project();
                }
                ui.separator();
                if ui.button("新建文件 (Ctrl+N)").clicked() {
                    ui.close_menu();
                    self.menu_new_file();
                }
                if ui.button("新建文件夹").clicked() {
                    ui.close_menu();
                    self.menu_new_folder();
                }
                if ui.button("打开文件夹 (Ctrl+O)").clicked() {
                    ui.close_menu();
                    self.menu_open_folder();
                }
                ui.separator();
                if ui.button("保存 (Ctrl+S)").clicked() {
                    ui.close_menu();
                    self.save_active(ctx);
                }
                if ui.button("全部保存 (Ctrl+Shift+S)").clicked() {
                    ui.close_menu();
                    self.save_all(ctx);
                }
                ui.separator();
                if ui.button("退出").clicked() {
                    ui.close_menu();
                    self.exit_app(ctx);
                }
            });
            ui.menu_button("编辑", |ui| {
                if ui.button("撤销").clicked() {
                    ui.close_menu();
                    self.edit_undo();
                }
                if ui.button("重做").clicked() {
                    ui.close_menu();
                    self.edit_redo();
                }
                ui.separator();
                if ui.button("剪切").clicked() {
                    ui.close_menu();
                    self.edit_cut(ctx);
                }
                if ui.button("复制").clicked() {
                    ui.close_menu();
                    self.edit_copy(ctx);
                }
                if ui.button("粘贴").clicked() {
                    ui.close_menu();
                    self.edit_paste(ctx);
                }
                if ui.button("全选").clicked() {
                    ui.close_menu();
                    self.edit_select_all(ctx);
                }
            });
            ui.menu_button("视图", |ui| {
                if ui.button("刷新文件树").clicked() {
                    ui.close_menu();
                    self.refresh_tree();
                    self.set_toast("文件树已刷新");
                }
                if ui.button("切换暗色亮色").clicked() {
                    ui.close_menu();
                    self.toggle_theme(ctx);
                }
                ui.checkbox(&mut self.show_ast, "显示 AST 侧边栏");
                ui.checkbox(&mut self.show_output, "显示输出面板");
            });
            ui.menu_button("运行", |ui| {
                if ui.button("运行 (F5)").clicked() {
                    ui.close_menu();
                    self.run_cargo(ctx, RunMode::Run);
                }
                if ui.button("检查 (F7)").clicked() {
                    ui.close_menu();
                    self.run_cargo(ctx, RunMode::Check);
                }
                if ui.button("终止运行").clicked() {
                    ui.close_menu();
                    self.terminate_cargo();
                }
            });
            ui.menu_button("终端", |ui| {
                if ui.button("新建终端 (Ctrl+`)").clicked() {
                    ui.close_menu();
                    self.terminal.visible = true;
                    let ws = self.workspace.clone();
                    self.terminal.ensure_spawned(ws.as_deref(), ctx);
                }
                if ui.button("关闭终端").clicked() {
                    ui.close_menu();
                    self.terminal.kill();
                    self.terminal.visible = false;
                }
            });
            ui.menu_button("搜索", |ui| {
                if ui.button("在文件中搜索 (Ctrl+Shift+F)").clicked() {
                    ui.close_menu();
                    if self.search.is_none() {
                        self.search = Some(SearchState {
                            input: String::new(),
                            case_sensitive: false,
                            running: Arc::new(AtomicBool::new(false)),
                            results: Arc::new(Mutex::new(Vec::new())),
                        });
                    }
                    self.completion = None;
                }
                if ui.button("在当前文件中查找 (Ctrl+F)").clicked() {
                    ui.close_menu();
                    if self.find.is_none() {
                        self.find = Some(FindState { input: String::new(), total: 0, index: 0 });
                    }
                }
            });
            ui.menu_button("帮助", |ui| {
                if ui.button("关于 NovaRS").clicked() {
                    ui.close_menu();
                    self.about_open = true;
                }
            });
        });
    }

    fn toggle_theme(&mut self, ctx: &egui::Context) {
        self.dark_mode = !self.dark_mode;
        ctx.set_visuals(if self.dark_mode { Visuals::dark() } else { Visuals::light() });
        self.save_settings();
        self.set_toast(if self.dark_mode { "已切换到暗色模式" } else { "已切换到亮色模式" });
    }

    // ---------- 绘制：文件树 ----------

    fn draw_tree(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("资源管理器").strong());
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("刷新").clicked() {
                    self.refresh_tree();
                    self.set_toast("文件树已刷新");
                }
            });
        });
        ui.separator();
        if self.workspace.is_none() {
            ui.label("未打开文件夹");
            ui.label("使用菜单：文件 → 打开文件夹");
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let root_children = self.tree_root.clone();
            for idx in root_children {
                self.draw_node(ui, idx, 0);
            }
        });
    }

    fn tree_file_color(&self, path: &Path) -> Option<Color32> {
        if self
            .error_files
            .lock()
            .map(|s| s.contains(path))
            .unwrap_or(false)
        {
            return Some(Color32::from_rgb(0xe9, 0x45, 0x60));
        }
        match self.tabs.iter().find(|t| t.path == path) {
            Some(t) if t.dirty => Some(Color32::from_rgb(0xe5, 0xc0, 0x7b)),
            Some(_) => Some(Color32::from_rgb(0x50, 0xa1, 0x4f)),
            None => None,
        }
    }

    fn draw_node(&mut self, ui: &mut egui::Ui, idx: usize, depth: usize) {
        let (name, path, is_dir) = {
            let node = &self.nodes[idx];
            (node.name.clone(), node.path.clone(), node.is_dir)
        };
        if is_dir {
            let header = egui::CollapsingHeader::new(RichText::new(format!("📁 {}", name)))
                .default_open(depth == 0)
                .id_salt(format!("tree-node:{}", path.display()));
            let resp = header.show(ui, |ui| {
                let children = self.nodes[idx].children.clone();
                if children.is_empty() {
                    ui.weak("（空文件夹）");
                }
                for child in children {
                    self.draw_node(ui, child, depth + 1);
                }
            });
            resp.header_response.context_menu(|ui| {
                if ui.button("新建文件").clicked() {
                    ui.close_menu();
                    self.menu_new_file();
                }
                if ui.button("新建文件夹").clicked() {
                    ui.close_menu();
                    self.name_dialog = Some(NameDialog {
                        title: "新建文件夹".to_string(),
                        hint: format!("在 {} 下创建新文件夹，请输入名称：", path.display()),
                        value: "新建文件夹".to_string(),
                        error: None,
                        kind: NameKind::NewFolder(path.clone()),
                    });
                }
                if depth > 0 {
                    if ui.button("重命名").clicked() {
                        ui.close_menu();
                        self.name_dialog = Some(NameDialog {
                            title: "重命名".to_string(),
                            hint: format!("将 “{}” 重命名为：", name),
                            value: name.clone(),
                            error: None,
                            kind: NameKind::Rename(path.clone()),
                        });
                    }
                    if ui.button("删除").clicked() {
                        ui.close_menu();
                        self.delete_path(path.clone());
                    }
                }
                ui.separator();
                if ui.button("在文件管理器中显示").clicked() {
                    ui.close_menu();
                    reveal_in_explorer(&path);
                }
                if ui.button("复制路径").clicked() {
                    ui.close_menu();
                    ui.output_mut(|o| o.copied_text = path.display().to_string());
                    self.set_toast(format!("已复制路径: {}", path.display()));
                }
            });
        } else {
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let icon = if ext == "rs" {
                "🦀"
            } else if name == "Cargo.toml" {
                "📦"
            } else {
                "📄"
            };
            let selected = self
                .active_tab
                .map(|i| self.tabs[i].path == path)
                .unwrap_or(false);
            let status_color = self.tree_file_color(&path);
            let label_text = match status_color {
                Some(c) => RichText::new(format!("{} {}", icon, name)).color(c),
                None => RichText::new(format!("{} {}", icon, name)),
            };
            let resp = ui.selectable_label(selected, label_text);
            if resp.clicked() {
                self.open_file_in_tab(path.clone());
            }
            resp.context_menu(|ui| {
                if ui.button("打开").clicked() {
                    ui.close_menu();
                    self.open_file_in_tab(path.clone());
                }
                if ui.button("重命名").clicked() {
                    ui.close_menu();
                    self.name_dialog = Some(NameDialog {
                        title: "重命名".to_string(),
                        hint: format!("将 “{}” 重命名为：", name),
                        value: name.clone(),
                        error: None,
                        kind: NameKind::Rename(path.clone()),
                    });
                }
                if ui.button("删除").clicked() {
                    ui.close_menu();
                    self.delete_path(path.clone());
                }
                ui.separator();
                if ui.button("在文件管理器中显示").clicked() {
                    ui.close_menu();
                    reveal_in_explorer(&path);
                }
                if ui.button("复制路径").clicked() {
                    ui.close_menu();
                    ui.output_mut(|o| o.copied_text = path.display().to_string());
                    self.set_toast(format!("已复制路径: {}", path.display()));
                }
            });
        }
    }

    // ---------- 绘制：标签栏与编辑器 ----------

    fn draw_tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let infos: Vec<(usize, String, String, bool, bool)> = self
                .tabs
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    (
                        i,
                        t.title.clone(),
                        t.path.display().to_string(),
                        t.dirty,
                        self.active_tab == Some(i),
                    )
                })
                .collect();
            for (i, title, path, dirty, active) in infos {
                let label = if dirty { format!("{}*", title) } else { title };
                let resp = ui.selectable_label(active, RichText::new(label));
                if resp.clicked() {
                    self.active_tab = Some(i);
                    self.last_cursor_range = None;
                    self.ast_sig = None;
                    self.completion = None;
                    self.match_highlight = None;
                }
                if resp.hovered() {
                    resp.on_hover_text(path);
                }
                if ui.small_button("✕").clicked() {
                    self.request_close_tab(i);
                }
            }
        });
        ui.separator();
    }

    fn draw_editor(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let Some(ti) = self.active_tab else { return };
        // IME 组合/提交窗口期：吞掉 Enter，避免中文输入法上屏时误换行
        let mut swallow_enter = false;
        if self.ime_composing {
            swallow_enter = true;
        } else if let Some(t) = self.ime_commit_at {
            if t.elapsed() < Duration::from_millis(150) {
                swallow_enter = true;
            } else {
                self.ime_commit_at = None;
            }
        }
        if swallow_enter {
            ctx.input_mut(|i| {
                i.events
                    .retain(|e| !matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, .. }))
            });
        }
        self.track_undo(ti);
        if self.auto_complete_pending {
            self.auto_complete_pending = false;
            self.open_completion(false);
        }
        if self.force_completion {
            self.force_completion = false;
            self.open_completion(true);
        }

        let tab_id = self.tabs[ti].id;
        let theme = CodeTheme::new(self.dark_mode);
        let row_h = ctx.fonts(|f| f.row_height(&FontId::monospace(MONO_SIZE)));
        let char_w = ctx.fonts(|f| {
            f.layout_no_wrap("M".to_string(), FontId::monospace(MONO_SIZE), Color32::WHITE)
                .size()
                .x
        });
        let mut text = std::mem::take(&mut self.tabs[ti].text);
        let scroll_target = self.scroll_to_line;
        let _breakpoints = self.tabs[ti].breakpoints.clone();
        let pending_offset = self.scroll_to_offset.take();

        let line_count = count_lines(&text);
        let digits = line_count.to_string().len();
        let num_w = digits as f32 * char_w + 18.0;
        let max_line_chars = text.lines().map(|l| l.chars().count()).max().unwrap_or(0);
        let edit_avail_w = (ui.available_width() - 12.0).max(300.0);
        let needed_w = (max_line_chars as f32 * char_w + 2.0 * char_w)
            .max(edit_avail_w - num_w - GUTTER_W)
            .min(20_000.0);
        // 视口高度防护：避免布局异常时拿到 0/负/无限值导致内容高度塌缩
        let avail_h = ui.available_height();
        let avail_h = if avail_h.is_finite() && avail_h > 120.0 { avail_h } else { 600.0 };
        let content_h = (line_count as f32 * row_h + row_h).max(avail_h - 4.0);

        // 行号/断点侧栏在 draw_main 中绘制（与滚动裁剪解耦），此处不再计算
        let digits = line_count.to_string().len();
        let _ = digits;

        let editor_id = Self::editor_id_of(tab_id);
        let mut consumed_scroll = false;
        let mut clicked_gutter_y: Option<f32> = None;
        let mut popup_anchor_local: Option<egui::Pos2> = None;
        let pending_selection = self.pending_selection.take();

        // 注意：MiniMap 使用独立右侧面板（不在本行内并排），避免 horizontal 布局
        // 与 ScrollArea::both() 组合时视口高度计算异常导致编辑器内容塌缩
        let mut area = egui::ScrollArea::both().auto_shrink([false, false]);
        if let Some(offset) = pending_offset {
            area = area.vertical_scroll_offset(offset);
        }
        let scroll_out = area.show(ui, |ui| {
            ui.horizontal(|ui| {
                // 行号/断点槽已移到 ScrollArea 外部侧栏，这里只保留等宽占位
                let (_, gutter_resp) =
                    ui.allocate_exact_size(Vec2::new(GUTTER_W + num_w, content_h), Sense::click());
                if gutter_resp.clicked() {
                    if let Some(pos) = gutter_resp.interact_pointer_pos() {
                        clicked_gutter_y = Some(pos.y);
                    }
                }
                let mut layouter =
                    |_ui: &egui::Ui, content: &str, _wrap_width: f32| -> Arc<Galley> {
                        let mut job = LayoutJob::default();
                        highlight_rust(content, &theme, &mut job);
                        ctx.fonts(|f| f.layout_job(job))
                    };
                let out = egui::TextEdit::multiline(&mut text)
                    .id(editor_id)
                    .font(FontSelection::FontId(FontId::monospace(MONO_SIZE)))
                    .frame(false)
                    .desired_width(needed_w)
                    .min_size(Vec2::new(needed_w, content_h))
                    .lock_focus(true)
                    .layouter(&mut layouter)
                    .show(ui);
                    if let Some(range) = out.cursor_range {
                        let p = range.primary.ccursor.index;
                        let s = range.secondary.ccursor.index;
                        let (l, c) = line_col_of_char_index(&text, p);
                        self.cursor_line = l;
                        self.cursor_col = c;
                        // 记录完整选区（secondary=锚点，primary=活动端）
                        self.last_cursor_range = Some((tab_id, (p, s)));
                    }
                    // 渲染后二次修正：应用待定光标/选区（AST 跳转、Tab 缩进、查找定位等），
                    // 下一帧 TextEdit 加载后光标/选区生效并保持
                    if let Some((tid, s, e)) = pending_selection {
                        if tid == tab_id {
                            let mut st = out.state.clone();
                            st.cursor.set_char_range(Some(CCursorRange::two(
                                CCursor::new(s.min(e)),
                                CCursor::new(s.max(e)),
                            )));
                            st.store(ctx, editor_id);
                        }
                    }
                // 行号与断点圆点由外部侧栏绘制（本处仅保留占位交互）
                // 断点槽点击 → 以文本 galley 原点解析行号
                if let Some(click_y) = clicked_gutter_y {
                    let rel = click_y - out.galley_pos.y;
                    let line = (rel / row_h).floor() as usize + 1;
                    if line >= 1 && line <= line_count {
                        let tab = &mut self.tabs[ti];
                        if !tab.breakpoints.remove(&line) {
                            tab.breakpoints.insert(line);
                        }
                        self.set_toast(format!("断点: 行 {}", line));
                    }
                }
                // 补全弹窗锚点：光标右上角
                if let Some(range) = out.cursor_range {
                    let local = out.galley.pos_from_ccursor(range.primary.ccursor);
                    popup_anchor_local = Some(egui::pos2(
                        local.max.x + out.galley_pos.x + 6.0,
                        local.min.y + out.galley_pos.y - 2.0,
                    ));
                }
                    // VS Code 风格缩进辅助线：按每行实际缩进级别画线段（视口裁剪）
                    {
                        let meta = self.editor_scroll_meta;
                        let first_vis = meta
                            .map(|m| ((m.1 / row_h) as usize).saturating_sub(3))
                            .unwrap_or(0);
                        let last_vis = meta
                            .map(|m| ((m.1 + m.3) / row_h) as usize + 3)
                            .unwrap_or(line_count);
                        let line_color = if self.dark_mode {
                            Color32::from_rgba_unmultiplied(255, 255, 255, 16)
                        } else {
                            Color32::from_rgba_unmultiplied(0, 0, 0, 22)
                        };
                        for (li, line) in text.lines().enumerate() {
                            if li < first_vis || li > last_vis {
                                continue;
                            }
                            // 行前导缩进列数（空格 = 1 列，Tab = 4 列）
                            let mut indent_cols = 0usize;
                            for c in line.chars() {
                                match c {
                                    ' ' => indent_cols += 1,
                                    '\t' => indent_cols += 4,
                                    _ => break,
                                }
                            }
                            let levels = indent_cols / 4;
                            if levels == 0 {
                                continue;
                            }
                            let y_top = out.galley_pos.y + li as f32 * row_h + 1.0;
                            let y_bot = y_top + row_h - 2.0;
                            for k in 1..=levels {
                                let x = out.galley_pos.x + (k * 4) as f32 * char_w + 0.5;
                                ui.painter().line_segment(
                                    [pos2(x, y_top), pos2(x, y_bot)],
                                    Stroke::new(1.0_f32, line_color),
                                );
                            }
                        }
                    }
                    // 当前匹配高亮（查找/搜索定位的文本范围，覆盖在文字上的半透明黄色）
                if let Some((hid, hs, he)) = self.match_highlight {
                    if hid == tab_id && he > hs && he <= text.chars().count() {
                        let r0 = out.galley.pos_from_ccursor(CCursor::new(hs));
                        let r1 = out.galley.pos_from_ccursor(CCursor::new(he));
                        let local = r0.union(r1);
                        let highlight_rect = Rect::from_min_size(
                            pos2(
                                local.min.x + out.galley_pos.x,
                                local.min.y + out.galley_pos.y,
                            ),
                            Vec2::new(local.width().max(char_w), local.height()),
                        );
                        ui.painter().rect_filled(
                            highlight_rect,
                            2.0,
                            Color32::from_rgba_unmultiplied(0xff, 0xd5, 0x4f, 72),
                        );
                    }
                }
                // AST 跳转定位：滚动到目标行
                if let Some((tid, line)) = scroll_target {
                    if tid == tab_id {
                        let y = out.galley_pos.y + (line as f32 - 1.0) * row_h;
                        let target_rect =
                            Rect::from_min_size(pos2(out.galley_pos.x, y), Vec2::new(4.0, row_h));
                        ui.allocate_rect(target_rect, Sense::hover())
                            .scroll_to_me(Some(Align::Center));
                        consumed_scroll = true;
                    }
                }
            });
        });
        // 滚动元数据写入 App（携带标签 id），供右侧 MiniMap 面板使用（一帧延迟，60fps 下不可感知）
        self.editor_scroll_meta = Some((
            tab_id as f32,
            scroll_out.state.offset.y,
            scroll_out.content_size.y.max(1.0),
            scroll_out.inner_rect.height(),
        ));        // MiniMap 行缓存（哈希变更时重建）
        let text_hash = {
            let mut hasher = DefaultHasher::new();
            text.hash(&mut hasher);
            hasher.finish()
        };
        if self.minimap_cache.as_ref().map(|(h, _)| *h) != Some(text_hash) {
            self.minimap_cache = Some((text_hash, build_minimap(&text)));
        }

        self.tabs[ti].text = text;
        if consumed_scroll {
            self.scroll_to_line = None;
        }
        self.completion_anchor = popup_anchor_local;
        if let Some(cmd) = self.popup_cmd.take() {
            match cmd {
                0 => {
                    if let Some(p) = &mut self.completion {
                        if !p.items.is_empty() {
                            p.selected = (p.selected + 1) % p.items.len();
                        }
                    }
                }
                1 => {
                    if let Some(p) = &mut self.completion {
                        if !p.items.is_empty() {
                            p.selected = (p.selected + p.items.len() - 1) % p.items.len();
                        }
                    }
                }
                2 => self.apply_completion(ctx),
                _ => self.completion = None,
            }
        }
    }

    fn draw_welcome(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(100.0);
            ui.label(RichText::new("🦀✨").size(56.0));
            ui.add_space(8.0);
            ui.heading(RichText::new(format!("{} {}", APP_FULL_NAME, APP_VERSION)).strong());
            ui.label("纯 Rust 原生 IDE — egui + eframe + syn + walkdir");
            ui.add_space(16.0);
            if ui.button(RichText::new("打开文件夹...").size(15.0)).clicked() {
                self.menu_open_folder();
            }
            ui.add_space(6.0);
            ui.label(RichText::new("提示：文件 → 新建文件 可创建第一个 Rust 源文件").weak());
        });
    }

    // ---------- 绘制：AST 侧边栏 ----------

    fn draw_ast_panel(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("AST 结构").strong());
        });
        ui.separator();
        let Some(ti) = self.active_tab else {
            ui.weak("未打开文件");
            return;
        };
        let (tab_id, is_rs, text) = {
            let tab = &self.tabs[ti];
            (
                tab.id,
                tab.path.extension().map(|e| e == "rs").unwrap_or(false),
                tab.text.clone(),
            )
        };
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let sig = Some((tab_id, hasher.finish()));
        if self.ast_sig != sig {
            self.ast_sig = sig;
            if is_rs {
                match syn::parse_file(&text) {
                    Ok(file) => {
                        let mut items = Vec::new();
                        collect_ast_items(&file.items, 0, &mut items);
                        self.ast_items = items;
                        self.ast_error = None;
                    }
                    Err(e) => {
                        self.ast_items.clear();
                        self.ast_error = Some(format!("解析错误: {}", e));
                    }
                }
            } else {
                self.ast_items.clear();
                self.ast_error = Some("仅解析 Rust 源文件（.rs）".to_string());
            }
        }
        if let Some(err) = &self.ast_error {
            ui.label(RichText::new(err).small().color(BRAND_RED));
            ui.separator();
        }
        if self.ast_items.is_empty() {
            ui.weak("没有可显示的结构");
            return;
        }
        let items = self.ast_items.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for item in items {
                let indent = "  ".repeat(item.depth.min(6));
                let label = format!("{}{:>6}  {}  · L{}", indent, item.kind, item.name, item.line);
                let resp = ui.selectable_label(
                    false,
                    RichText::new(label)
                        .monospace()
                        .size(12.0)
                        .color(ast_kind_color(item.kind, self.dark_mode)),
                );
                if resp.clicked() {
                    self.jump_to_line(ctx, item.line);
                }
            }
        });
    }

    // ---------- 绘制：MiniMap 面板 ----------

    fn draw_minimap_panel(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let dark = self.dark_mode;
        let mm_bg = if dark {
            Color32::from_rgb(0x1a, 0x1d, 0x24)
        } else {
            Color32::from_rgb(0xe8, 0xea, 0xed)
        };
        let mp = ui.painter_at(rect);
        mp.rect_filled(rect, 0.0, mm_bg);
        // 滚动元数据必须属于当前激活标签，否则（切标签/未打开文件）只画行块不画视口框
        let active_id = self.active_tab.map(|i| self.tabs[i].id);
        let Some(meta) = self
            .editor_scroll_meta
            .filter(|m| active_id == Some(m.0 as u64))
        else {
            return;
        };
        let (_, offset_y, content_h_real, viewport_h) = meta;
        let rows = self
            .minimap_cache
            .as_ref()
            .map(|(_, r)| r.clone())
            .unwrap_or_default();
        let total_mm = (rows.len() as f32 * 2.0).max(1.0);
        let scale = (rect.height() / total_mm).min(1.0);
        for (i, row) in rows.iter().enumerate() {
            let y = rect.top() + i as f32 * 2.0 * scale;
            if y > rect.bottom() {
                break;
            }
            let x = rect.left() + 3.0 + row.indent * 0.5;
            let w = row.width.min(rect.right() - x - 1.0);
            if w > 0.5 {
                mp.rect_filled(Rect::from_min_size(pos2(x, y), Vec2::new(w, 1.6)), 0.0, row.color);
            }
        }
        if content_h_real > 1.0 && rect.height() > 4.0 {
            let view_h = (viewport_h / content_h_real * rect.height())
                .clamp(10.0, rect.height());
            let view_y = (rect.top() + offset_y / content_h_real * rect.height())
                .clamp(rect.top(), rect.bottom() - view_h);
            let view_rect =
                Rect::from_min_max(pos2(rect.left(), view_y), pos2(rect.right(), view_y + view_h));
            // 视口覆盖比例越高填充越淡，避免小文件时整面板泛红
            let cover = (view_h / rect.height()).clamp(0.0, 1.0);
            if cover < 0.97 {
                let alpha = (70.0 * (1.0 - cover) + 6.0) as u8;
                mp.rect_filled(view_rect, 2.0, Color32::from_rgba_unmultiplied(0xe9, 0x45, 0x60, alpha));
            }
            mp.rect_stroke(
                view_rect,
                2.0,
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0xe9, 0x45, 0x60, 120)),
            );
        }
        // 点击/拖拽 MiniMap → 滚动编辑器（滚动元数据下一帧生效）
        let resp = ui.interact(rect, Id::new("minimap_interact"), Sense::click_and_drag());
        if resp.dragged() || resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let frac = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
                self.scroll_to_offset =
                    Some((frac * content_h_real - viewport_h * 0.5).max(0.0));
            }
        }
    }

    // ---------- 绘制：终端面板 ----------

    fn draw_terminal_panel(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("终端").strong());
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("关闭").clicked() {
                    self.terminal.kill();
                    self.terminal.visible = false;
                }
                if ui.small_button("清空").clicked() {
                    lock(&self.terminal.log).clear();
                }
                if ui.small_button("重启会话").clicked() {
                    self.terminal.kill();
                    let ws = self.workspace.clone();
                    self.terminal.ensure_spawned(ws.as_deref(), ctx);
                    self.set_toast("终端会话已重启");
                }
            });
        });
        ui.separator();
        // 命令输入行固定在面板顶部（不随日志滚动，始终可见可输入）
        ui.horizontal(|ui| {
            ui.label(RichText::new("PS>").monospace().color(BRAND_RED));
            let width = (ui.available_width() - 80.0).max(120.0);
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.terminal.input)
                    .desired_width(width)
                    .hint_text("输入命令，Enter 执行（真实 PowerShell 会话，cd 等状态保留）"),
            );
            let enter = resp.lost_focus() && ctx.input(|i| i.key_pressed(Key::Enter));
            if enter {
                let ws = self.workspace.clone();
                self.terminal.send_command(ws.as_deref(), ctx);
            }
            if ui.button("执行").clicked() {
                let ws = self.workspace.clone();
                self.terminal.send_command(ws.as_deref(), ctx);
            }
        });
        ui.separator();
        // 日志占满剩余空间：面板拉多高，日志区就有多高
        let log = lock(&self.terminal.log).clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(log).monospace().size(12.0)).selectable(true),
                );
            });
    }

    // ---------- 绘制：状态栏 / 输出 / 弹窗 ----------

    fn draw_status_bar(&mut self, ui: &mut egui::Ui) {
        let left_text = match self.active_tab {
            Some(i) => self.tabs[i].path.display().to_string(),
            None => match &self.workspace {
                Some(w) => format!("工作区: {}", w.display()),
                None => "未打开文件".to_string(),
            },
        };
        let mid_text = format!("行 {}, 列 {}", self.cursor_line, self.cursor_col);
        let (lang, eol) = match self.active_tab {
            Some(i) => {
                let tab = &self.tabs[i];
                (lang_of_path(&tab.path), tab.line_ending)
            }
            None => ("—", "—"),
        };
        let git_text = self
            .git_branch
            .clone()
            .map(|b| format!("git:{}", b))
            .unwrap_or_default();
        let info_text = format!("{} · UTF-8 · {}  {}", lang, eol, git_text);
        let right_text = {
            let info = lock(&self.toolchain).clone();
            let rust_label = match info.rustc_short {
                Some(v) => format!("Rust {}", v),
                None => "Rust 未检测到".to_string(),
            };
            format!("{} | {}", APP_NAME_WITH_VERSION, rust_label)
        };
        ui.columns(4, |cols| {
            cols[0].with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(left_text).size(12.0));
            });
            cols[1].with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(mid_text).size(12.0).weak());
            });
            cols[2].with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(info_text).size(12.0).weak());
            });
            cols[3].with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(right_text).size(12.0).weak());
            });
        });
    }

    fn draw_output_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("输出").strong());
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("关闭").clicked() {
                    self.show_output = false;
                }
                if ui.small_button("清空").clicked() {
                    lock(&self.output_log).clear();
                }
            });
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let log = lock(&self.output_log).clone();
                ui.label(RichText::new(log).monospace().size(12.0));
            });
    }

    fn draw_name_dialog(&mut self, ctx: &egui::Context) {
        let mut dialog = match self.name_dialog.take() {
            Some(d) => d,
            None => return,
        };
        let mut open = true;
        let mut cancelled = false;
        let mut confirmed = false;
        egui::Window::new(RichText::new(dialog.title.as_str()).strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.label(dialog.hint.as_str());
                ui.add_space(6.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut dialog.value)
                        .desired_width(300.0)
                        .font(FontSelection::FontId(FontId::monospace(MONO_SIZE))),
                );
                // 中文输入法组合中按回车是“上屏候选”，不视为确认
                let ime_busy = self.ime_composing
                    || self
                        .ime_commit_at
                        .map(|t| t.elapsed() < Duration::from_millis(150))
                        .unwrap_or(false);
                if resp.lost_focus() && !ime_busy && ctx.input(|i| i.key_pressed(Key::Enter)) {
                    confirmed = true;
                }
                if let Some(err) = &dialog.error {
                    ui.label(RichText::new(err.as_str()).small().color(BRAND_RED));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("确定").clicked() {
                        confirmed = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });
        if cancelled {
            open = false;
        }
        if confirmed {
            let kept = self.confirm_name_dialog(dialog);
            if let Some(d) = kept {
                self.name_dialog = Some(d);
            }
        } else if open {
            self.name_dialog = Some(dialog);
        }
    }

    fn draw_about(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let mut open = true;
        egui::Window::new("关于 NovaRS")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new("🦀✨").size(40.0));
                    ui.heading(RichText::new(APP_FULL_NAME).strong());
                    ui.label(RichText::new(APP_VERSION).size(16.0).color(BRAND_RED));
                    ui.add_space(6.0);
                    ui.label("纯 Rust 原生实现 · egui + eframe · 单二进制");
                    let info = { toolchain_summary(&lock(&self.toolchain)) };
                    ui.label(RichText::new(info).small().weak());
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("© 2026 {}", APP_NAME)).small().weak());
                });
            });
        self.about_open = open;
    }

    fn draw_toast(&mut self, ctx: &egui::Context) {
        let active = match &self.toast {
            Some((_, at)) => at.elapsed() < Duration::from_secs(4),
            None => false,
        };
        if !active {
            self.toast = None;
            return;
        }
        if let Some((_, at)) = &self.toast {
            let remaining = Duration::from_secs(4).saturating_sub(at.elapsed());
            ctx.request_repaint_after(remaining);
        }
        let msg = self
            .toast
            .as_ref()
            .map(|(m, _)| m.clone())
            .unwrap_or_default();
        egui::Area::new(Id::new("toast_area"))
            .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -32.0))
            .show(ctx, |ui| {
                let frame = egui::Frame::popup(ui.style());
                frame.show(ui, |ui| {
                    ui.label(RichText::new(msg).size(13.0));
                });
            });
    }

    // ---------- 绘制：智能补全弹窗 ----------

    fn completion_kind_color(tag: &str, dark: bool) -> Color32 {
        if dark {
            match tag {
                "fn" => Color32::from_rgb(0x61, 0xaf, 0xef),
                "struct" => Color32::from_rgb(0xe5, 0xc0, 0x7b),
                "enum" => Color32::from_rgb(0xc6, 0x78, 0xdd),
                "mod" | "crate" => Color32::from_rgb(0x56, 0xb6, 0xc2),
                "let" | "var" => Color32::from_rgb(0xe0, 0x6c, 0x75),
                "use" => Color32::from_rgb(0x98, 0xc3, 0x79),
                "kw" | "snippet" => Color32::from_rgb(0xc6, 0x78, 0xdd),
                _ => Color32::from_rgb(0xab, 0xb2, 0xbf),
            }
        } else {
            match tag {
                "fn" => Color32::from_rgb(0x40, 0x78, 0xbf),
                "struct" => Color32::from_rgb(0xb7, 0x6b, 0x01),
                "enum" => Color32::from_rgb(0x98, 0x68, 0x01),
                "mod" | "crate" => Color32::from_rgb(0x0e, 0x8a, 0x84),
                "let" | "var" => Color32::from_rgb(0xca, 0x4b, 0x37),
                "use" => Color32::from_rgb(0x50, 0xa1, 0x4f),
                "kw" | "snippet" => Color32::from_rgb(0xa6, 0x26, 0xa4),
                _ => Color32::from_rgb(0x38, 0x3a, 0x42),
            }
        }
    }

    fn draw_completion_popup(&mut self, ctx: &egui::Context) {
        if self.completion.is_none() {
            return;
        }
        // LSP 结果到达后替换内置列表（同代请求才生效）
        {
            let pending_gen = self.completion.as_ref().map(|p| p.gen).unwrap_or(-1);
            if pending_gen >= 0 {
                if let Some(lsp) = &self.lsp {
                    if *lock(&lsp.items_gen) == pending_gen {
                        let items = lock(&lsp.items).clone();
                        if !items.is_empty() {
                            if let Some(p) = &mut self.completion {
                                if p.items != items {
                                    p.items = items;
                                    p.selected = p.selected.min(p.items.len() - 1);
                                }
                            }
                        }
                    }
                }
            }
        }
        let Some(anchor) = self.completion_anchor else { return };
        let Some(popup) = self.completion.as_ref() else { return };
        if popup.items.is_empty() {
            self.completion = None;
            return;
        }
        let visible_rows = popup.items.len().min(9);
        let height = (visible_rows as f32 * POPUP_ROW_H + 10.0).min(200.0);
        let width = (popup
            .items
            .iter()
            .map(|c| c.label.chars().count() + c.detail.chars().count() / 2)
            .max()
            .unwrap_or(12) as f32
            * 7.5
            + 60.0)
            .clamp(140.0, 400.0);
        let screen = ctx.screen_rect();
        let pos = egui::pos2(
            (anchor.x + 2.0).min(screen.right() - width - 8.0).max(screen.left() + 8.0),
            (anchor.y - height - 6.0).max(screen.top() + 36.0),
        );
        let dark = self.dark_mode;
        let selected = popup.selected;
        let count = popup.items.len();
        let items = popup.items.clone();
        egui::Area::new(Id::new("completion_popup"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let mut frame = egui::Frame::none();
                frame.fill = if dark {
                    Color32::from_rgba_unmultiplied(0x1e, 0x22, 0x2e, 0xf5)
                } else {
                    Color32::from_rgba_unmultiplied(0xfa, 0xfa, 0xfc, 0xf5)
                };
                frame.rounding = egui::Rounding::same(6.0);
                frame.stroke =
                    Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0xe9, 0x45, 0x60, 140));
                frame.shadow = egui::Shadow {
                    offset: egui::vec2(0.0, 6.0),
                    blur: 14.0,
                    spread: 1.0,
                    color: Color32::from_black_alpha(110),
                };
                frame.show(ui, |ui| {
                    ui.set_min_width(width);
                    egui::ScrollArea::vertical()
                        .max_height(height - 8.0)
                        .show(ui, |ui| {
                            for (i, item) in items.iter().enumerate() {
                                let is_selected = i == selected;
                                let kind_color =
                                    Self::completion_kind_color(item.kind_tag, dark);
                                let row = ui.allocate_ui(
                                    Vec2::new(width - 12.0, POPUP_ROW_H),
                                    |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(format!("{:>7}", item.kind_tag))
                                                    .monospace()
                                                    .size(11.0)
                                                    .color(kind_color),
                                            );
                                            let mut label =
                                                RichText::new(item.label.as_str()).size(13.0);
                                            if is_selected {
                                                label = label.strong();
                                            }
                                            ui.label(label);
                                            if !item.detail.is_empty() {
                                                ui.label(
                                                    RichText::new(item.detail.as_str())
                                                        .size(11.0)
                                                        .weak(),
                                                );
                                            }
                                        });
                                    },
                                );
                                if is_selected {
                                    ui.painter().rect_filled(
                                        row.response.rect,
                                        3.0,
                                        Color32::from_rgba_unmultiplied(0xe9, 0x45, 0x60, 46),
                                    );
                                }
                                let row_resp = row.response.interact(Sense::click());
                                if row_resp.clicked() {
                                    if let Some(p) = &mut self.completion {
                                        p.selected = i;
                                    }
                                }
                            }
                        });
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "{} 项 · ↑↓ 选择 · Tab 确认 · Esc 关闭  ({}/{})",
                            count,
                            selected + 1,
                            count
                        ))
                        .size(10.0)
                        .weak(),
                    );
                });
            });
    }

    // ---------- 绘制：搜索面板 / 查找条 ----------

    fn draw_search_overlay(&mut self, ctx: &egui::Context) {
        let Some(mut search) = self.search.take() else { return };
        let close = ctx.input(|i| i.key_pressed(Key::Escape));
        if close {
            self.search = None;
            return;
        }
        let running = search.running.load(Ordering::Relaxed);
        let results = lock(&search.results).clone();
        let dark = self.dark_mode;
        let mut start_clicked = false;
        let mut open_target: Option<(PathBuf, usize)> = None;
        egui::Area::new(Id::new("search_overlay"))
            .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 44.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let mut frame = egui::Frame::none();
                frame.fill = if dark {
                    Color32::from_rgba_unmultiplied(0x1a, 0x1e, 0x28, 0xf2)
                } else {
                    Color32::from_rgba_unmultiplied(0xfc, 0xfc, 0xfe, 0xf2)
                };
                frame.rounding = egui::Rounding::same(8.0);
                frame.stroke =
                    Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0xe9, 0x45, 0x60, 130));
                frame.shadow = egui::Shadow {
                    offset: egui::vec2(0.0, 8.0),
                    blur: 18.0,
                    spread: 2.0,
                    color: Color32::from_black_alpha(120),
                };
                frame.show(ui, |ui| {
                    ui.set_min_width(640.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("在文件中搜索").strong());
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("✕").clicked() {
                                self.search = None;
                            }
                        });
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("关键词:");
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut search.input)
                                .desired_width(320.0)
                                .hint_text("回车开始搜索（walkdir 遍历工作区，跳过 .git/target）"),
                        );
                        ui.checkbox(&mut search.case_sensitive, "大小写敏感");
                        if ui.button("搜索").clicked()
                            || (resp.lost_focus()
                                && ctx.input(|i| i.key_pressed(Key::Enter)))
                        {
                            start_clicked = true;
                        }
                    });
                    ui.separator();
                    if running {
                        ui.label(RichText::new("正在搜索……").weak());
                    } else {
                        ui.label(
                            RichText::new(format!(
                                "共 {} 个匹配（最多显示 1000 条，点击结果跳转）",
                                results.len()
                            ))
                            .weak(),
                        );
                    }
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for hit in &results {
                                let label = format!(
                                    "{}:{}  {}",
                                    hit.path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default(),
                                    hit.line_no,
                                    hit.text
                                );
                                let resp = ui.selectable_label(
                                    false,
                                    RichText::new(label).monospace().size(12.0),
                                );
                                if resp.clicked() {
                                    open_target =
                                        Some((hit.path.clone(), hit.line_no));
                                }
                            }
                        });
                });
            });
        if start_clicked {
            self.search = Some(search);
            self.start_project_search(ctx);
            return;
        }
        self.search = Some(search);
        if let Some((path, line)) = open_target {
            self.open_file_in_tab(path);
            self.jump_to_line(ctx, line);
            // 匹配行整行高亮（编辑器内黄色标注）
            if let Some(ti2) = self.active_tab {
                let tab_id = self.tabs[ti2].id;
                let text = self.tabs[ti2].text.clone();
                let start = line_start_char_index(&text, line);
                let line_len = text
                    .lines()
                    .nth(line.saturating_sub(1))
                    .map(|l| l.chars().count())
                    .unwrap_or(0);
                self.match_highlight = Some((tab_id, start, start + line_len));
            }
        }
    }

    fn draw_find_overlay(&mut self, ctx: &egui::Context) {
        if self.find.is_none() {
            return;
        }
        let close = ctx.input(|i| i.key_pressed(Key::Escape));
        if close {
            self.find = None;
            return;
        }
        let (total, index) = match &self.find {
            Some(f) => (f.total, f.index),
            None => (0, 0),
        };
        let mut next_clicked = false;
        let mut close_clicked = false;
        egui::Area::new(Id::new("find_overlay"))
            .anchor(Align2::RIGHT_TOP, egui::vec2(-24.0, 40.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let frame = egui::Frame::popup(ui.style());
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("查找:");
                        if let Some(find) = self.find.as_mut() {
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut find.input).desired_width(180.0),
                            );
                            let enter = resp.lost_focus()
                                && ctx.input(|i| i.key_pressed(Key::Enter));
                            if enter || ui.button("下一个").clicked() {
                                next_clicked = true;
                            }
                        }
                        ui.label(
                            RichText::new(if total > 0 {
                                format!("{}/{}", (index - 1 + total) % total + 1, total)
                            } else {
                                "0/0".to_string()
                            })
                            .weak(),
                        );
                        if ui.small_button("✕").clicked() {
                            close_clicked = true;
                        }
                    });
                });
            });
        if close_clicked {
            self.find = None;
        }
        if next_clicked {
            self.find_next_in_file(ctx);
        }
    }

    // ---------- 主界面帧 ----------

    fn draw_main(&mut self, ctx: &egui::Context) {
        // 纭繚绐楀彛灏哄涓?1280x800 閫昏緫鐐逛笖涓庢樉绀哄櫒 DPI 鍚屾锛堥槻寰?winit 鍒濆 scale 绔炴€侊級
        if !self.main_size_ok {
            let native_ppp = ctx.input(|i| i.viewport().native_pixels_per_point);
            if let Some(n) = native_ppp {
                if (ctx.pixels_per_point() - n).abs() > 0.01 {
                    ctx.set_pixels_per_point(n);
                }
            }
            if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
                let dw = (rect.width() - MAIN_SIZE.x).abs();
                let dh = (rect.height() - MAIN_SIZE.y).abs();
                if dw < 3.0 && dh < 3.0 {
                    self.main_size_ok = true;
                } else {
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(MAIN_SIZE));
                    center_window(ctx, MAIN_SIZE);
                }
            }
        }
        egui::TopBottomPanel::top("menu_bar_panel").show(ctx, |ui| {
            self.draw_menu_bar(ctx, ui);
        });
        egui::TopBottomPanel::bottom("status_panel")
            .exact_height(24.0)
            .show(ctx, |ui| {
                self.draw_status_bar(ui);
            });
        if self.terminal.visible {
            egui::TopBottomPanel::bottom("terminal_panel")
                .resizable(true)
                .default_height(180.0)
                .min_height(80.0)
                .show(ctx, |ui| {
                    self.draw_terminal_panel(ctx, ui);
                });
        }
        if self.show_output {
            egui::TopBottomPanel::bottom("output_panel")
                .resizable(true)
                .default_height(180.0)
                .min_height(80.0)
                .max_height(400.0)
                .show(ctx, |ui| {
                    self.draw_output_panel(ui);
                });
        }
        if self.show_ast {
            egui::SidePanel::right("ast_panel")
                .resizable(true)
                .default_width(240.0)
                .min_width(160.0)
                .show(ctx, |ui| {
                    self.draw_ast_panel(ctx, ui);
                });
        }
        egui::SidePanel::right("minimap_panel")
            .resizable(false)
            .exact_width(MINIMAP_W)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                self.draw_minimap_panel(ui);
            });
        egui::SidePanel::left("files_panel")
            .resizable(true)
            .default_width(220.0)
            .min_width(140.0)
            .show(ctx, |ui| {
                self.draw_tree(ui);
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.tabs.is_empty() {
                self.draw_welcome(ui);
            } else {
                self.draw_tab_bar(ui);
                // 行号/断点侧栏 + 编辑器（横向并排）
                let ti = self.active_tab.unwrap_or(0);
                let tab_id = self.tabs[ti].id;
                let theme = CodeTheme::new(self.dark_mode);
                let row_h = ctx.fonts(|f| f.row_height(&FontId::monospace(MONO_SIZE)));
                let char_w = ctx.fonts(|f| {
                    f.layout_no_wrap(
                        "M".to_string(),
                        FontId::monospace(MONO_SIZE),
                        Color32::WHITE,
                    )
                    .size()
                    .x
                });
                let line_count = count_lines(&self.tabs[ti].text);
                let digits = line_count.to_string().len();
                let num_w = digits as f32 * char_w + 18.0;
                let _breakpoints = self.tabs[ti].breakpoints.clone();
                ui.horizontal(|ui| {
                    let sidebar_w = GUTTER_W + num_w + 10.0;
                    let (side_rect, side_resp) = ui
                        .allocate_exact_size(
                            Vec2::new(sidebar_w, ui.available_height()),
                            Sense::click(),
                        );
                    if side_resp.clicked() {
                        if let Some(pos) = side_resp.interact_pointer_pos() {
                            // 侧栏点击 → 行号换算（基于滚动元数据）
                            if let Some((tid, off, _ch, _vh)) = self.editor_scroll_meta {
                                if tid as u64 == tab_id {
                                    let row =
                                        (((pos.y - side_rect.top() + off) / row_h).floor()
                                            as usize)
                                            + 1;
                                    if row >= 1 && row <= line_count {
                                        let tab = &mut self.tabs[ti];
                                        if !tab.breakpoints.remove(&row) {
                                            tab.breakpoints.insert(row);
                                        }
                                        self.set_toast(format!("断点: 行 {}", row));
                                    }
                                }
                            }
                        }
                    }
                    // 独立 Painter（Rect::EVERYTHING，不受父级裁剪链交集限制）——
                    // ui.painter_at 会与父裁剪取交集，在高滚动偏移处会截断行号
                    let sp = egui::Painter::new(
                        ctx.clone(),
                        ui.layer_id(),
                        egui::Rect::EVERYTHING,
                    );
                    let side_bg = if self.dark_mode {
                        Color32::from_rgba_unmultiplied(255, 255, 255, 5)
                    } else {
                        Color32::from_rgba_unmultiplied(0, 0, 0, 5)
                    };
                    sp.rect_filled(side_rect, 0.0, side_bg);
                    if let Some((tid, off, _ch, vh)) = self.editor_scroll_meta {
                        if tid as u64 == tab_id {
                            // 首个可视行（部分露出的行也算），行号 galley 从这里起画
                            let first_f = (off / row_h).floor();
                            let first = (first_f as usize).min(line_count);
                            let last = ((((off + vh) / row_h).ceil() as usize) + 1)
                                .min(line_count);
                            // 断点圆点
                            for line in &_breakpoints {
                                if *line >= first && *line <= last {
                                    let y = side_rect.top()
                                        + (*line as f32 * row_h - off)
                                        + row_h * 0.5;
                                    if side_rect.contains(pos2(side_rect.center().x, y)) {
                                        sp.circle_filled(
                                            pos2(GUTTER_W * 0.5 + side_rect.left(), y),
                                            3.5,
                                            BRAND_RED,
                                        );
                                    }
                                }
                            }
                            // 行号：逐行独立绘制，每行先做可视区判定（超出即销毁不画），
                            // 屏幕坐标恒在可视区内，从原理上免疫任何裁剪/截断
                            let num_x = side_rect.left() + GUTTER_W;
                            let top_limit = side_rect.top() - row_h;
                            let bottom_limit = side_rect.bottom();
                            let mut row = first;
                            while row <= last {
                                let y = row as f32 * row_h - off + side_rect.top();
                                if y < top_limit {
                                    row += 1;
                                    continue;
                                }
                                if y > bottom_limit {
                                    break;
                                }
                                let text_str = format!("{:>width$}", row, width = digits);
                                sp.text(
                                    pos2(num_x, y),
                                    Align2::LEFT_TOP,
                                    text_str,
                                    FontId::monospace(MONO_SIZE),
                                    theme.line_number,
                                );
                                row += 1;
                            }
                        }
                    }
                    self.draw_editor(ctx, ui);
                });
            }
        });

        self.draw_completion_popup(ctx);
        self.draw_search_overlay(ctx);
        self.draw_find_overlay(ctx);
        self.draw_project_dialog(ctx);
        self.draw_name_dialog(ctx);
        self.draw_about(ctx);
        self.draw_toast(ctx);
    }
}

fn spawn_reader<R: std::io::Read + Send + 'static>(
    stream: Option<R>,
    log: Arc<Mutex<String>>,
    ctx: egui::Context,
) {
    if let Some(stream) = stream {
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        lock(&log).push_str(&line);
                        ctx.request_repaint();
                    }
                }
            }
        });
    }
}

const APP_NAME_WITH_VERSION: &str = "NovaRS v0.1.0";

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    // Emoji 回退（🦀 不在 egui 内置 NotoEmoji 中，使用 Windows 系统 Segoe UI Emoji）
    let emoji_path = "C:\\Windows\\Fonts\\seguiemj.ttf";
    if Path::new(emoji_path).is_file() {
        if let Ok(bytes) = std::fs::read(emoji_path) {
            let loaded = catch_unwind(AssertUnwindSafe(|| FontData::from_owned(bytes)));
            if let Ok(data) = loaded {
                fonts.font_data.insert("nova_emoji".to_string(), data);
                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .push("nova_emoji".to_string());
                fonts
                    .families
                    .entry(FontFamily::Monospace)
                    .or_default()
                    .push("nova_emoji".to_string());
            }
        }
    }
    // 中文界面字体：优先使用嵌入的等线（单文件自包含、跨机器渲染一致），
    // 失败时回退系统字体链（msyh/simhei）
    let embedded = catch_unwind(AssertUnwindSafe(|| {
        FontData::from_owned(FONT_CJK_EMBEDDED.to_vec())
    }));
    if let Ok(data) = embedded {
        fonts.font_data.insert("nova_cjk_embedded".to_string(), data);
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .push("nova_cjk_embedded".to_string());
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push("nova_cjk_embedded".to_string());
    }
    let candidates: [(&str, &str); 3] = [
        ("nova_cjk_deng", "C:\\Windows\\Fonts\\Deng.ttf"),
        ("nova_cjk_msyh", "C:\\Windows\\Fonts\\msyh.ttc"),
        ("nova_cjk_simhei", "C:\\Windows\\Fonts\\simhei.ttf"),
    ];
    for (name, path) in candidates {
        if !Path::new(path).is_file() {
            continue;
        }
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let loaded = catch_unwind(AssertUnwindSafe(|| FontData::from_owned(bytes)));
        match loaded {
            Ok(data) => {
                fonts.font_data.insert(name.to_string(), data);
                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .push(name.to_string());
                fonts
                    .families
                    .entry(FontFamily::Monospace)
                    .or_default()
                    .push(name.to_string());
                break;
            }
            Err(_) => continue,
        }
    }
    ctx.set_fonts(fonts);
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.capture_dir.is_some() {
            // 帧导出诊断模式：保持低频重绘以轮询截图请求
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        // 接收启动线程交付的 LSP 会话（rust-analyzer 握手完成后就绪）
        if self.lsp.is_none() {
            let delivered = lock(&self.lsp_slot).take();
            if let Some(session) = delivered {
                self.lsp = Some(session);
            }
        }
        self.poll_capture_request(ctx);
        self.deliver_screenshots(ctx);
        // IME 组合状态跟踪：中文输入法组合中，抑制回车确认/换行误触
        {
            let mut composing = self.ime_composing;
            ctx.input(|i| {
                for e in &i.raw.events {
                    match e {
                        egui::Event::Ime(egui::ImeEvent::Enabled)
                        | egui::Event::Ime(egui::ImeEvent::Preedit(_)) => composing = true,
                        egui::Event::Ime(egui::ImeEvent::Commit(_))
                        | egui::Event::Ime(egui::ImeEvent::Disabled) => composing = false,
                        _ => {}
                    }
                }
            });
            self.ime_composing = composing;
        }
        self.handle_global_shortcuts(ctx);
        self.handle_popup_keys(ctx);
        self.handle_tab_indent(ctx);
        match self.phase {
            Phase::Splash => self.draw_splash(ctx),
            Phase::Main => self.draw_main(ctx),
        }
    }
}

impl App {
    fn poll_capture_request(&mut self, ctx: &egui::Context) {
        let Some(dir) = self.capture_dir.clone() else { return };
        let request_path = dir.join("request.txt");
        let modified = match std::fs::metadata(&request_path).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => return,
        };
        if self.capture_seen == Some(modified) {
            return;
        }
        // 规避 egui_glow read_screen_rgba 在窗口尺寸缓存为 0 时的除零 panic：
        // 仅在窗口可见且尺寸正常时发送截图请求，否则等待下一轮轮询
        let size_ok = ctx.input(|i| {
            let rect = i.screen_rect;
            rect.width() > 10.0 && rect.height() > 10.0 && i.viewport().minimized != Some(true)
        });
        if !size_ok {
            return;
        }
        self.capture_seen = Some(modified);
        let raw = match std::fs::read_to_string(&request_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let name = raw.trim().replace('\\', "_").replace('/', "_");
        if name.is_empty() || !name.ends_with(".png") {
            return;
        }
        self.capture_name = Some(name);
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
    }

    fn deliver_screenshots(&mut self, ctx: &egui::Context) {
        if self.capture_dir.is_none() {
            return;
        }
        let events = ctx.input(|i| i.raw.events.clone());
        for event in events {
            if let egui::Event::Screenshot { image, .. } = event {
                let Some(dir) = self.capture_dir.clone() else { continue };
                let Some(name) = self.capture_name.clone() else { continue };
                let path = dir.join(name);
                let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
                for color in &image.pixels {
                    rgba.push(color.r());
                    rgba.push(color.g());
                    rgba.push(color.b());
                    rgba.push(color.a());
                }
                let _ = image::save_buffer(
                    &path,
                    &rgba,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                );
                self.capture_name = None;
            }
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    // 窗口/任务栏图标（编译期嵌入 512px PNG，启动时解码一次）
    let icon_bytes = include_bytes!("../icons/novars-512.png");
    let icon_img = image::load_from_memory(icon_bytes).expect("内置图标解码失败").to_rgba8();
    let (icon_w, icon_h) = (icon_img.width(), icon_img.height());
    let icon = egui::IconData {
        width: icon_w,
        height: icon_h,
        rgba: icon_img.into_raw(),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_FULL_NAME)
            .with_icon(std::sync::Arc::new(icon))
            .with_inner_size([SPLASH_SIZE.x, SPLASH_SIZE.y])
            .with_decorations(false)
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(APP_FULL_NAME, options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
