// NovaRS — Embedded rust-analyzer LSP Client & Intelligent Fallback Completion Engine

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::constants::RA_RELEASE_NAME;
use crate::utils::{apply_no_window, lock};

#[derive(Clone, PartialEq, Debug)]
pub struct LspCompletion {
    pub label: String,
    pub kind_tag: &'static str,
    pub detail: String,
}

pub fn lsp_kind_tag(kind: Option<u64>) -> &'static str {
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

pub fn path_to_uri(path: &Path) -> String {
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

pub struct LspSession {
    pub stdin: Arc<Mutex<ChildStdin>>,
    pub pending: Arc<Mutex<i64>>,
    pub items: Arc<Mutex<Vec<LspCompletion>>>,
    pub items_gen: Arc<Mutex<i64>>,
    pub ready: Arc<AtomicBool>,
    pub alive: Arc<AtomicBool>,
    pub versions: Arc<Mutex<HashMap<String, i64>>>,
    pub opened: Arc<Mutex<HashSet<String>>>,
}

impl LspSession {
    pub fn send(&self, value: &serde_json::Value) -> bool {
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

    pub fn next_request_id(&self) -> i64 {
        let mut guard = lock(&self.pending);
        let id = if *guard < 0 { 2 } else { *guard + 1 };
        *guard = id;
        id
    }

    pub fn request_completion(&self, uri: &str, line: usize, character: usize) -> i64 {
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

    pub fn notify_did_open(&self, path: &Path, text: &str) {
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

    pub fn notify_did_change(&self, path: &Path, text: &str) {
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
        out.push(LspCompletion {
            label,
            kind_tag: lsp_kind_tag(kind),
            detail,
        });
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

pub fn start_rust_analyzer(
    embedded: &[u8],
    workspace: Option<PathBuf>,
    gate: Arc<AtomicBool>,
) -> Option<Arc<LspSession>> {
    // 优先使用嵌入的二进制，如果为空则尝试系统 PATH 中的 rust-analyzer
    let exe_path = if !embedded.is_empty() {
        let exe = std::env::temp_dir().join(RA_RELEASE_NAME);
        if std::fs::write(&exe, embedded).is_ok() {
            Some(exe)
        } else {
            None
        }
    } else if crate::utils::which_exists("rust-analyzer") {
        Some(PathBuf::from("rust-analyzer"))
    } else {
        None
    };

    let Some(exe) = exe_path else {
        gate.store(true, Ordering::Relaxed);
        return None;
    };

    let mut cmd = Command::new(&exe);
    apply_no_window(&mut cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
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

pub const STD_COMPLETIONS: &[(&str, &str)] = &[
    ("Vec", "struct"),
    ("String", "struct"),
    ("Option", "enum"),
    ("Result", "enum"),
    ("HashMap", "struct"),
    ("HashSet", "struct"),
    ("BTreeMap", "struct"),
    ("BTreeSet", "struct"),
    ("VecDeque", "struct"),
    ("Box", "struct"),
    ("Rc", "struct"),
    ("Arc", "struct"),
    ("Mutex", "struct"),
    ("RwLock", "struct"),
    ("RefCell", "struct"),
    ("Path", "struct"),
    ("PathBuf", "struct"),
    ("Duration", "struct"),
    ("Instant", "struct"),
    ("Some", "enum"),
    ("None", "kw"),
    ("Ok", "enum"),
    ("Err", "enum"),
    ("println!", "snippet"),
    ("eprintln!", "snippet"),
    ("print!", "snippet"),
    ("format!", "snippet"),
    ("vec!", "snippet"),
    ("write!", "snippet"),
    ("writeln!", "snippet"),
    ("assert!", "fn"),
    ("assert_eq!", "fn"),
    ("debug_assert!", "fn"),
    ("todo!", "snippet"),
    ("match", "kw"),
    ("impl", "kw"),
    ("trait", "kw"),
    ("async", "kw"),
    ("await", "kw"),
    ("self", "kw"),
    ("Self", "kw"),
    ("super", "kw"),
    ("crate", "kw"),
];

pub fn builtin_completions(
    text: &str,
    prefix: &str,
    workspace: Option<&Path>,
) -> Vec<LspCompletion> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<LspCompletion> = Vec::new();
    {
        let mut push = |label: String, tag: &'static str, detail: &str| {
            if label.is_empty() || label == prefix || seen.contains(&label) || out.len() >= 250 {
                return;
            }
            seen.insert(label.clone());
            out.push(LspCompletion {
                label,
                kind_tag: tag,
                detail: detail.to_string(),
            });
        };

        for line in text.lines() {
            let trimmed = line.trim_start();
            for (kw, tag) in [
                ("pub fn ", "fn"),
                ("fn ", "fn"),
                ("pub struct ", "struct"),
                ("struct ", "struct"),
                ("pub enum ", "enum"),
                ("enum ", "enum"),
                ("pub trait ", "trait"),
                ("trait ", "trait"),
                ("pub mod ", "mod"),
                ("mod ", "mod"),
                ("use ", "use"),
                ("const ", "let"),
                ("pub const ", "let"),
            ] {
                if let Some(rest) = trimmed.strip_prefix(kw) {
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    push(name, tag, "当前文件");
                }
            }
            if let Some(rest) = trimmed.strip_prefix("let ") {
                let rest = rest.strip_prefix("mut ").unwrap_or(rest);
                if let Some(name) = rest.split(|c| c == '=' || c == ':').next() {
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
            push((*name).to_string(), tag, "Rust 标准库");
        }
        if let Some(ws) = workspace {
            let manifest = ws.join("Cargo.toml");
            if let Ok(content) = std::fs::read_to_string(&manifest) {
                let mut in_deps = false;
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with('[') {
                        in_deps = trimmed == "[dependencies]" || trimmed == "[dev-dependencies]";
                        continue;
                    }
                    if in_deps {
                        if let Some(eq) = trimmed.find('=') {
                            let name = trimmed[..eq].trim().replace('-', "_");
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
