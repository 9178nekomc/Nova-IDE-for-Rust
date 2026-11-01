// NovaRS — Built-in Model Context Protocol (MCP) Server & Client Bridge
// Allows external AI Agents (Claude Desktop, Cursor, Claude Code CLI, Windsurf, custom agents)
// to connect via JSON-RPC 2.0 over HTTP (`http://127.0.0.1:<port>/mcp`) and inspect/operate the IDE,
// while also serving as the default built-in bridge for the NovaRS AI Copilot Sidebar.
//
// Security model (see README §MCP):
// - The listener binds to the loopback interface only.
// - Every request must carry `Authorization: Bearer <token>` (or `X-NovaRS-Token: <token>`).
//   The token is generated with the OS CSPRNG on every server start and is displayed in the MCP panel.
// - Requests carrying an `Origin` header are rejected: legitimate MCP clients are native processes,
//   so an Origin always indicates browser-originated traffic (i.e. a local-LAN/web page drive-by).
// - Every tool call is confined to the opened workspace by `verify_workspace_jail`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, Color32, RichText};
use walkdir::WalkDir;

use crate::ai::sandbox::verify_workspace_jail;
use crate::ast::collect_ast_items;
use crate::constants::{BRAND_ACCENT, BRAND_GREEN, BRAND_RED, BRAND_WARN, DEFAULT_MCP_PORT};
use crate::utils::{apply_no_window, lock};

/// 请求体硬上限（8 MiB）：write_file / apply_diff 的补丁通常远小于此值，
/// 超出即视为异常客户端，避免单连接耗尽内存。
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
/// 请求头硬上限（64 KiB）
const MAX_HEADER_BYTES: usize = 64 * 1024;
/// 单个连接的读写超时，防止半开连接长期占用线程
const IO_TIMEOUT: Duration = Duration::from_secs(20);
/// 单次 read_file 返回的最大行数
const MAX_READ_LINES: usize = 5_000;
/// search_code 单文件扫描上限
const MAX_SEARCH_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// search_code 最大命中条数
const MAX_SEARCH_HITS: usize = 50;

#[derive(Clone, Debug)]
pub struct McpCallLog {
    pub timestamp: String,
    pub source: String, // "Built-in Sidebar" or "External Agent (HTTP)"
    pub tool: String,
    pub status: String,
}

pub struct McpServerState {
    pub dialog_open: bool,
    pub running: Arc<AtomicBool>,
    pub port: u16,
    pub builtin_sidebar_connected: bool,
    /// 外部 HTTP 访问令牌；服务每次启动时用操作系统 CSPRNG 重新生成
    pub auth_token: Arc<Mutex<String>>,
    pub shared_workspace: Arc<Mutex<Option<PathBuf>>>,
    pub call_logs: Arc<Mutex<Vec<McpCallLog>>>,
    pub last_error: Arc<Mutex<Option<String>>>,
}

impl McpServerState {
    pub fn new(port: u16, enabled: bool, builtin_connected: bool, workspace: Option<PathBuf>) -> Self {
        let state = McpServerState {
            dialog_open: false,
            running: Arc::new(AtomicBool::new(false)),
            port: if port == 0 { DEFAULT_MCP_PORT } else { port },
            builtin_sidebar_connected: builtin_connected,
            auth_token: Arc::new(Mutex::new(String::new())),
            shared_workspace: Arc::new(Mutex::new(workspace)),
            call_logs: Arc::new(Mutex::new(Vec::new())),
            last_error: Arc::new(Mutex::new(None)),
        };
        if enabled {
            state.start_server();
        }
        state
    }

    pub fn update_workspace(&self, workspace: Option<PathBuf>) {
        *lock(&self.shared_workspace) = workspace;
    }

    pub fn start_server(&self) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        // 每次启动重新生成令牌：旧会话的凭据不会跨重启继续有效
        let token = generate_auth_token();
        *lock(&self.auth_token) = token.clone();

        let port = self.port;
        let running = self.running.clone();
        let ws_arc = self.shared_workspace.clone();
        let logs_arc = self.call_logs.clone();
        let err_arc = self.last_error.clone();
        let token_arc = self.auth_token.clone();

        std::thread::spawn(move || {
            let addr = format!("127.0.0.1:{}", port);
            let listener = match TcpListener::bind(&addr) {
                Ok(l) => {
                    *lock(&err_arc) = None;
                    l
                }
                Err(e) => {
                    *lock(&err_arc) = Some(format!("端口 {} 绑定失败: {}", port, e));
                    running.store(false, Ordering::SeqCst);
                    return;
                }
            };

            for stream in listener.incoming() {
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let ws = lock(&ws_arc).clone();
                let logs = logs_arc.clone();
                let expected_token = lock(&token_arc).clone();

                std::thread::spawn(move || {
                    handle_http_connection(stream, ws.as_deref(), &logs, &expected_token);
                });
            }
        });
    }

    pub fn stop_server(&self) {
        self.running.store(false, Ordering::SeqCst);
        // 发送空连接唤醒阻塞在 accept 上的线程
        let _ = std::net::TcpStream::connect(format!("127.0.0.1:{}", self.port));
    }

    /// 供内置 AI 副驾驶侧边栏直接同步调用 MCP 工具（进程内调用，无需令牌）
    pub fn execute_tool_direct(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let ws = lock(&self.shared_workspace).clone();
        let result = dispatch_mcp_tool(tool_name, args, ws.as_deref());
        record_call(&self.call_logs, "内置侧栏", "Built-in AI Sidebar", tool_name, result.is_ok());
        result
    }

    pub fn draw_dialog(&mut self, ctx: &egui::Context) -> bool {
        let mut changed = false;
        if !self.dialog_open {
            return false;
        }
        let mut open = self.dialog_open;
        let is_running = self.running.load(Ordering::SeqCst);
        let token = lock(&self.auth_token).clone();

        egui::Window::new("🔌 MCP 协议服务与外部 AI Agent 桥接中心 (Model Context Protocol)")
            .open(&mut open)
            .resizable(true)
            .default_width(680.0)
            .default_height(520.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let badge = if is_running {
                        RichText::new(format!("🟢 MCP 服务运行中 (http://127.0.0.1:{}/mcp)", self.port))
                            .strong()
                            .color(BRAND_GREEN)
                    } else {
                        RichText::new("🔴 MCP 服务已停止").strong().color(BRAND_RED)
                    };
                    ui.label(badge);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if is_running {
                            if ui.button("⏹ 停止 MCP 服务").clicked() {
                                self.stop_server();
                                changed = true;
                            }
                        } else if ui.button("▶ 启动 MCP 服务").clicked() {
                            self.start_server();
                            changed = true;
                        }
                    });
                });

                if let Some(ref err) = *lock(&self.last_error) {
                    ui.label(RichText::new(err).color(BRAND_RED).size(11.5));
                }

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("监听端口 (Port):");
                    let mut p = self.port as i32;
                    if ui
                        .add_enabled(!is_running, egui::DragValue::new(&mut p).range(1024..=65535))
                        .changed()
                    {
                        self.port = p as u16;
                        changed = true;
                    }
                    if ui
                        .checkbox(
                            &mut self.builtin_sidebar_connected,
                            "默认直连内置 AI 副驾驶侧栏 (Built-in Copilot Auto-Bridge)",
                        )
                        .changed()
                    {
                        changed = true;
                    }
                });

                // 访问令牌：外部 Agent 必须携带，防止任意本机网页借 CORS 直接驱动 IDE
                ui.separator();
                ui.label(RichText::new("🔑 访问令牌 (Bearer Token，每次启动重新生成)").strong().color(BRAND_WARN));
                ui.horizontal(|ui| {
                    if is_running && !token.is_empty() {
                        ui.label(RichText::new(&token).monospace().size(11.5).color(BRAND_GREEN));
                        if ui.small_button("📋 复制令牌").clicked() {
                            ui.output_mut(|o| o.copied_text = token.clone());
                        }
                    } else {
                        ui.weak("服务未运行 — 启动后将生成一次性令牌");
                    }
                });
                ui.weak("外部客户端必须发送 `Authorization: Bearer <token>`；携带 `Origin` 头的请求一律拒绝。");

                ui.separator();
                ui.label(
                    RichText::new("📋 暴露给外部 AI Agent (Claude Code / Cursor / 外部客户端) 的标准 MCP 工具集:")
                        .strong()
                        .color(BRAND_ACCENT),
                );
                egui::Grid::new("mcp_tools_table")
                    .num_columns(3)
                    .spacing([14.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for (tname, params, desc) in [
                            ("read_file", "{ path, start_line, end_line }", "读取工作区内源码文件指定行范围"),
                            ("write_file", "{ path, content }", "在工作区沙箱内创建或覆写源码文件"),
                            ("apply_diff", "{ path, start_line, end_line, replacement }", "向指定文件应用精确行级补丁"),
                            ("list_files", "{ dir }", "遍历工作区目录文件树"),
                            ("search_code", "{ query, case_sensitive }", "全项目高性能代码检索"),
                            ("get_diagnostics", "{}", "执行 cargo check 并返回结构化编译诊断"),
                            ("get_ast_outline", "{ path }", "使用 syn 解析 Rust 文件的 AST 符号大纲"),
                            ("run_cargo", "{ subcommand }", "安全执行 cargo check / test / clippy / fmt"),
                        ] {
                            ui.label(RichText::new(tname).monospace().strong().color(BRAND_GREEN));
                            ui.label(RichText::new(params).monospace().size(11.0).weak());
                            ui.label(RichText::new(desc).size(11.5));
                            ui.end_row();
                        }
                    });

                ui.separator();
                ui.strong("📡 外部 Agent 接入配置示例 (JSON-RPC 2.0 / HTTP):");
                let cfg_snippet = format!(
                    "{{\"mcpServers\": {{\"novars-ide\": {{\n  \"url\": \"http://127.0.0.1:{}/mcp\",\n  \"headers\": {{ \"Authorization\": \"Bearer {}\" }}\n}}}}}}",
                    self.port,
                    if token.is_empty() { "<启动服务后生成>" } else { token.as_str() }
                );
                ui.label(RichText::new(cfg_snippet).monospace().size(11.0).color(BRAND_WARN));

                ui.separator();
                ui.horizontal(|ui| {
                    ui.strong("📜 最近 MCP 工具调用审计日志:");
                    if ui.small_button("清空日志").clicked() {
                        lock(&self.call_logs).clear();
                    }
                });
                let logs = lock(&self.call_logs).clone();
                egui::ScrollArea::vertical()
                    .id_salt("mcp_audit_logs")
                    .max_height(110.0)
                    .show(ui, |ui| {
                        if logs.is_empty() {
                            ui.weak("暂无调用记录");
                        }
                        for l in logs.iter().rev() {
                            ui.label(
                                RichText::new(format!(
                                    "[{}] {} → {} ({})",
                                    l.timestamp, l.source, l.tool, l.status
                                ))
                                .monospace()
                                .size(11.0)
                                .color(Color32::LIGHT_GRAY),
                            );
                        }
                    });
            });

        self.dialog_open = open;
        changed
    }
}

fn record_call(
    logs: &Arc<Mutex<Vec<McpCallLog>>>,
    timestamp: &str,
    source: &str,
    tool: &str,
    ok: bool,
) {
    let mut guard = lock(logs);
    guard.push(McpCallLog {
        timestamp: timestamp.to_string(),
        source: source.to_string(),
        tool: tool.to_string(),
        status: if ok { "OK".to_string() } else { "ERR".to_string() },
    });
    // 审计日志只保留最近 500 条，避免长时间运行后无限增长
    if guard.len() > 500 {
        let overflow = guard.len() - 500;
        guard.drain(0..overflow);
    }
}

/// 从操作系统 CSPRNG 填充缓冲区。
/// Windows 走 `BCryptGenRandom(BCRYPT_USE_SYSTEM_PREFERRED_RNG)`，类 Unix 走 `/dev/urandom`，
/// 均不引入额外 crate —— 避免为了 32 字节随机数把整条依赖链拖进构建。
fn os_random_fill(buf: &mut [u8]) -> bool {
    #[cfg(target_os = "windows")]
    {
        // NTSTATUS BCryptGenRandom(BCRYPT_ALG_HANDLE, PUCHAR, ULONG, ULONG)
        // BCRYPT_USE_SYSTEM_PREFERRED_RNG = 0x00000002，传 NULL 句柄即使用系统首选算法
        #[link(name = "bcrypt")]
        extern "system" {
            fn BCryptGenRandom(
                algorithm: *mut core::ffi::c_void,
                buffer: *mut u8,
                len: u32,
                flags: u32,
            ) -> i32;
        }
        if buf.len() > u32::MAX as usize {
            return false;
        }
        let status = unsafe {
            BCryptGenRandom(
                core::ptr::null_mut(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                0x0000_0002,
            )
        };
        status == 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        use std::io::Read as _;
        match std::fs::File::open("/dev/urandom") {
            Ok(mut f) => f.read_exact(buf).is_ok(),
            Err(_) => false,
        }
    }
}

/// 用操作系统 CSPRNG 生成 32 字节随机令牌（64 位十六进制字符串）。
/// CSPRNG 不可用时退化为时间/进程/计数器混合（仅在极端受限环境下发生）。
fn generate_auth_token() -> String {
    let mut buf = [0u8; 32];
    if !os_random_fill(&mut buf) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        static CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = CTR.fetch_add(1, Ordering::SeqCst);
        // 地址熵在借用前先取出，避免与 chunks_mut 的可变借用冲突
        let addr_key = &buf as *const _ as usize;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for chunk in buf.chunks_mut(8) {
            let mut h = DefaultHasher::new();
            now.hash(&mut h);
            std::process::id().hash(&mut h);
            n.hash(&mut h);
            addr_key.hash(&mut h);
            chunk.copy_from_slice(&h.finish().to_le_bytes());
        }
    }
    to_hex(&buf)
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// 定长时间比较，避免令牌校验被计时侧信道逐字节猜测
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

enum HttpFailure {
    /// 状态码 + 面向客户端的简短说明
    Status(u16, &'static str),
}

fn write_response(stream: &mut TcpStream, status: u16, reason: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        status,
        reason,
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn write_json_error(stream: &mut TcpStream, status: u16, reason: &str, message: &str) {
    let body = serde_json::json!({ "error": message }).to_string();
    write_response(stream, status, reason, &body);
}

/// 严格按 HTTP 消息边界读取请求：先读到 `\r\n\r\n` 解析报头，再按 Content-Length
/// 精确补齐报文体。原先"单次 read 65536 字节"的实现会在报文分片或 body > 64 KiB 时
/// 静默截断 JSON，导致工具调用随机失败。
fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, HttpFailure> {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));

    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut header_end = None;
    let mut chunk = [0u8; 8192];
    while header_end.is_none() {
        let n = match stream.read(&mut chunk) {
            Ok(0) => return Err(HttpFailure::Status(400, "Bad Request")),
            Ok(n) => n,
            Err(_) => return Err(HttpFailure::Status(408, "Request Timeout")),
        };
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
            header_end = Some(pos);
        } else if buf.len() > MAX_HEADER_BYTES {
            return Err(HttpFailure::Status(431, "Request Header Fields Too Large"));
        }
    }
    let header_end = header_end.unwrap_or(0);
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_ascii_uppercase();
    let path = parts.next().unwrap_or("/").to_string();
    if method.is_empty() || path.is_empty() {
        return Err(HttpFailure::Status(400, "Bad Request"));
    }

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    if headers
        .get("transfer-encoding")
        .map(|v| v.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false)
    {
        // 简单起见只支持 Content-Length 定长报文，显式拒绝而非静默解析错误
        return Err(HttpFailure::Status(411, "Length Required"));
    }

    let content_len: usize = match headers.get("content-length") {
        Some(v) => v.parse().map_err(|_| HttpFailure::Status(400, "Bad Request"))?,
        None => 0,
    };
    if content_len > MAX_BODY_BYTES {
        return Err(HttpFailure::Status(413, "Payload Too Large"));
    }

    let body_start = header_end + 4;
    let mut body = buf[body_start.min(buf.len())..].to_vec();
    while body.len() < content_len {
        let n = match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return Err(HttpFailure::Status(408, "Request Timeout")),
        };
        body.extend_from_slice(&chunk[..n]);
        if body.len() > MAX_BODY_BYTES {
            return Err(HttpFailure::Status(413, "Payload Too Large"));
        }
    }
    if body.len() < content_len {
        return Err(HttpFailure::Status(400, "Bad Request"));
    }
    body.truncate(content_len);

    Ok(HttpRequest { method, path, headers, body })
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn handle_http_connection(
    mut stream: TcpStream,
    workspace: Option<&Path>,
    logs: &Arc<Mutex<Vec<McpCallLog>>>,
    expected_token: &str,
) {
    let req = match read_http_request(&mut stream) {
        Ok(r) => r,
        Err(HttpFailure::Status(code, reason)) => {
            write_json_error(&mut stream, code, reason, reason);
            return;
        }
    };

    if req.method != "POST" {
        write_json_error(&mut stream, 405, "Method Not Allowed", "只接受 POST /mcp");
        return;
    }
    if req.path.split('?').next() != Some("/mcp") && req.path != "/" {
        write_json_error(&mut stream, 404, "Not Found", "未知端点，MCP 端点为 /mcp");
        return;
    }
    // 浏览器发起的跨站请求必然带 Origin；原生 MCP 客户端不会。直接拒绝可封堵
    // "任意网页通过 localhost 驱动本机 IDE 读写文件" 的攻击面。
    if req.headers.contains_key("origin") {
        record_call(logs, "HTTP-RPC", "External Agent", "blocked(browser-origin)", false);
        write_json_error(&mut stream, 403, "Forbidden", "拒绝携带 Origin 头的浏览器请求");
        return;
    }

    let provided = req
        .headers
        .get("authorization")
        .map(|v| v.trim().to_string())
        .or_else(|| req.headers.get("x-novars-token").map(|v| v.trim().to_string()))
        .unwrap_or_default();
    let provided = provided
        .strip_prefix("Bearer ")
        .or_else(|| provided.strip_prefix("bearer "))
        .unwrap_or(&provided)
        .to_string();
    if expected_token.is_empty() || !constant_time_eq(&provided, expected_token) {
        record_call(logs, "HTTP-RPC", "External Agent", "blocked(bad-token)", false);
        write_json_error(&mut stream, 401, "Unauthorized", "缺少或错误的访问令牌 (Authorization: Bearer <token>)");
        return;
    }

    let body = String::from_utf8_lossy(&req.body).to_string();
    match handle_jsonrpc_mcp_request(&body, workspace, logs) {
        Some(response) => {
            let resp_str = serde_json::to_string(&response).unwrap_or_else(|_| "{}".to_string());
            write_response(&mut stream, 200, "OK", &resp_str);
        }
        // JSON-RPC 通知（notification）没有 id，协议规定不得返回响应体
        None => write_response(&mut stream, 202, "Accepted", ""),
    }
}

fn rpc_tools_list() -> serde_json::Value {
    serde_json::json!({ "tools": [
        {
            "name": "read_file",
            "description": "Read a line range from a text file inside the active NovaRS workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace-relative path" },
                    "start_line": { "type": "integer", "minimum": 1, "default": 1 },
                    "end_line": { "type": "integer", "minimum": 1 }
                },
                "required": ["path"]
            }
        },
        {
            "name": "write_file",
            "description": "Create or overwrite a file inside the active NovaRS workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace-relative path" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }
        },
        {
            "name": "apply_diff",
            "description": "Replace an inclusive 1-based line range of a workspace file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "start_line": { "type": "integer", "minimum": 1 },
                    "end_line": { "type": "integer", "minimum": 1 },
                    "replacement": { "type": "string" }
                },
                "required": ["path", "start_line", "end_line", "replacement"]
            }
        },
        {
            "name": "list_files",
            "description": "List files of the active workspace (target/, .git/, node_modules/ pruned).",
            "inputSchema": {
                "type": "object",
                "properties": { "dir": { "type": "string", "description": "Optional sub-directory" } }
            }
        },
        {
            "name": "search_code",
            "description": "Literal text search across workspace source files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "case_sensitive": { "type": "boolean", "default": true }
                },
                "required": ["query"]
            }
        },
        {
            "name": "get_diagnostics",
            "description": "Run `cargo check --message-format=json` and return structured compiler diagnostics.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_ast_outline",
            "description": "Parse a Rust file with syn and return its item outline (mod/struct/enum/trait/fn).",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }
        },
        {
            "name": "run_cargo",
            "description": "Run a whitelisted cargo subcommand (check/test/clippy/fmt/build) in the workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "subcommand": { "type": "string", "enum": ["check", "test", "clippy", "fmt", "build"] }
                },
                "required": ["subcommand"]
            }
        }
    ] })
}

/// 返回 `None` 表示这是一个通知（notification），无需回包。
fn handle_jsonrpc_mcp_request(
    body: &str,
    workspace: Option<&Path>,
    logs: &Arc<Mutex<Vec<McpCallLog>>>,
) -> Option<serde_json::Value> {
    let req: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return Some(serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": format!("Parse error: {}", e) }
            }))
        }
    };

    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");

    // 无 id 即为通知：initialize 之后的 `notifications/initialized` 属于此类
    let id = match id {
        Some(v) if !v.is_null() => v,
        _ => return None,
    };

    let result = match method {
        "initialize" => serde_json::json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": { "name": "novars-ide-mcp", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "tools": { "listChanged": false } }
        }),
        "ping" => serde_json::json!({}),
        "tools/list" => rpc_tools_list(),
        "tools/call" => {
            let params = &req["params"];
            let tool_name = params["name"].as_str().unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(serde_json::json!({}));
            let res = dispatch_mcp_tool(tool_name, &args, workspace);
            record_call(logs, "HTTP-RPC", "External AI Agent", tool_name, res.is_ok());
            match res {
                Ok(text) => serde_json::json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": false
                }),
                Err(err) => serde_json::json!({
                    "content": [{ "type": "text", "text": err }],
                    "isError": true
                }),
            }
        }
        other => {
            return Some(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Method not found: {}", other) }
            }))
        }
    };

    Some(serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// 跳过 `target` / `.git` / `node_modules` 等构建与依赖目录（按路径分量精确匹配，
/// 不再用子串匹配，避免 `my_target.rs` 之类的正常文件被误过滤）。
fn is_pruned_dir(name: &str) -> bool {
    matches!(name, "target" | ".git" | "node_modules" | ".idea" | ".vscode" | "dist")
}

fn path_is_pruned(path: &Path, workspace: &Path) -> bool {
    path.strip_prefix(workspace)
        .map(|rel| rel.components().any(|c| is_pruned_dir(&c.as_os_str().to_string_lossy())))
        .unwrap_or(false)
}

pub fn dispatch_mcp_tool(
    tool_name: &str,
    args: &serde_json::Value,
    workspace: Option<&Path>,
) -> Result<String, String> {
    match tool_name {
        "read_file" => {
            let rel = args["path"].as_str().ok_or("Missing `path`")?;
            let safe_path = verify_workspace_jail(workspace, Path::new(rel))?;
            let content = std::fs::read_to_string(&safe_path).map_err(|e| e.to_string())?;
            let start = args["start_line"].as_u64().unwrap_or(1).max(1) as usize;
            let end = args["end_line"]
                .as_u64()
                .map(|v| v as usize)
                .unwrap_or(start.saturating_add(MAX_READ_LINES - 1))
                .max(start);
            // 单次返回行数封顶，避免把整个巨型文件塞进模型上下文
            let end = end.min(start.saturating_add(MAX_READ_LINES - 1));
            let mut out = String::new();
            for (idx, line) in content.lines().enumerate() {
                let ln = idx + 1;
                if ln >= start && ln <= end {
                    out.push_str(&format!("L{}: {}\n", ln, line));
                }
                if ln > end {
                    break;
                }
            }
            if out.is_empty() {
                Ok(format!("(no content in L{}-L{}; file has {} lines)", start, end, content.lines().count()))
            } else {
                Ok(out)
            }
        }
        "write_file" => {
            let rel = args["path"].as_str().ok_or("Missing `path`")?;
            let content = args["content"].as_str().ok_or("Missing `content`")?;
            let safe_path = verify_workspace_jail(workspace, Path::new(rel))?;
            if let Some(parent) = safe_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&safe_path, content).map_err(|e| e.to_string())?;
            Ok(format!(
                "Successfully wrote {} bytes to {}",
                content.len(),
                safe_path.display()
            ))
        }
        "apply_diff" => {
            let rel = args["path"].as_str().ok_or("Missing `path`")?;
            let replacement = args["replacement"].as_str().ok_or("Missing `replacement`")?;
            let start = args["start_line"].as_u64().unwrap_or(1).max(1) as usize;
            let end = args["end_line"].as_u64().unwrap_or(start as u64) as usize;
            let safe_path = verify_workspace_jail(workspace, Path::new(rel))?;
            let content = std::fs::read_to_string(&safe_path).map_err(|e| e.to_string())?;
            let had_trailing_newline = content.ends_with('\n');
            let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
            let s_idx = start.saturating_sub(1).min(lines.len());
            let e_idx = end.min(lines.len()).max(s_idx);
            let rep_lines: Vec<String> = replacement.lines().map(|s| s.to_string()).collect();
            lines.splice(s_idx..e_idx, rep_lines);
            let mut new_content = lines.join("\n");
            // 保留原文件的行尾习惯：原本无结尾换行就不要强行补一个
            if had_trailing_newline {
                new_content.push('\n');
            }
            std::fs::write(&safe_path, new_content).map_err(|e| e.to_string())?;
            Ok(format!("Applied patch on {} L{}-L{}", safe_path.display(), start, end))
        }
        "list_files" => {
            let Some(ws) = workspace else {
                return Err("No workspace open".to_string());
            };
            let root = match args["dir"].as_str() {
                Some(d) if !d.trim().is_empty() => verify_workspace_jail(Some(ws), Path::new(d))?,
                _ => ws.to_path_buf(),
            };
            let mut list = Vec::new();
            for entry in WalkDir::new(&root).min_depth(1).max_depth(6).into_iter().flatten() {
                if path_is_pruned(entry.path(), ws) {
                    continue;
                }
                if entry.file_type().is_file() {
                    if let Ok(rel) = entry.path().strip_prefix(ws) {
                        list.push(rel.display().to_string().replace('\\', "/"));
                    }
                }
                if list.len() >= 5_000 {
                    break;
                }
            }
            list.sort();
            Ok(list.join("\n"))
        }
        "search_code" => {
            let Some(ws) = workspace else {
                return Err("No workspace open".to_string());
            };
            let query = args["query"].as_str().unwrap_or("");
            if query.is_empty() {
                return Ok("Empty query".to_string());
            }
            let case_sensitive = args["case_sensitive"].as_bool().unwrap_or(true);
            let needle = query.to_lowercase();
            let mut matches = Vec::new();
            'outer: for entry in WalkDir::new(ws).min_depth(1).into_iter().flatten() {
                if path_is_pruned(entry.path(), ws) || !entry.file_type().is_file() {
                    continue;
                }
                // 跳过二进制/超大文件，避免把整棵树读进内存
                match entry.metadata() {
                    Ok(m) if m.len() <= MAX_SEARCH_FILE_BYTES => {}
                    _ => continue,
                }
                let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
                for (i, line) in text.lines().enumerate() {
                    let hit = if case_sensitive {
                        line.contains(query)
                    } else {
                        line.to_lowercase().contains(&needle)
                    };
                    if hit {
                        if let Ok(rel) = entry.path().strip_prefix(ws) {
                            matches.push(format!(
                                "{}:{}: {}",
                                rel.display().to_string().replace('\\', "/"),
                                i + 1,
                                line.trim()
                            ));
                        }
                        if matches.len() >= MAX_SEARCH_HITS {
                            break 'outer;
                        }
                    }
                }
            }
            Ok(matches.join("\n"))
        }
        "get_diagnostics" => {
            let Some(ws) = workspace else {
                return Err("No workspace open".to_string());
            };
            if !ws.join("Cargo.toml").is_file() {
                return Err("当前工作区不是 Cargo 工程（缺少 Cargo.toml）".to_string());
            }
            let mut cmd = Command::new("cargo");
            apply_no_window(&mut cmd)
                .args(["check", "--message-format=json", "--quiet"])
                .current_dir(ws);
            let out = cmd.output().map_err(|e| format!("cargo check 启动失败: {}", e))?;
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut summary = Vec::new();
            for line in stdout.lines() {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
                if v["reason"] != "compiler-message" {
                    continue;
                }
                let msg = &v["message"];
                let level = msg["level"].as_str().unwrap_or("info");
                if level != "error" && level != "warning" {
                    continue;
                }
                let text = msg["message"].as_str().unwrap_or("");
                let code = msg["code"]["code"].as_str().unwrap_or("");
                let primary = msg["spans"]
                    .as_array()
                    .and_then(|spans| spans.iter().find(|s| s["is_primary"].as_bool() == Some(true)))
                    .or_else(|| msg["spans"].as_array().and_then(|s| s.first()));
                let (file, line_no, col) = match primary {
                    Some(s) => (
                        s["file_name"].as_str().unwrap_or("<unknown>").to_string(),
                        s["line_start"].as_u64().unwrap_or(0),
                        s["column_start"].as_u64().unwrap_or(0),
                    ),
                    None => ("<unknown>".to_string(), 0, 0),
                };
                summary.push(if code.is_empty() {
                    format!("{} {}:{}:{} {}", level, file, line_no, col, text)
                } else {
                    format!("{}[{}] {}:{}:{} {}", level, code, file, line_no, col, text)
                });
                if summary.len() >= 200 {
                    summary.push("… (truncated at 200 diagnostics)".to_string());
                    break;
                }
            }
            if summary.is_empty() {
                Ok(format!("✅ cargo check 通过（exit={}），无 error/warning", out.status))
            } else {
                Ok(summary.join("\n"))
            }
        }
        "get_ast_outline" => {
            let rel = args["path"].as_str().ok_or("Missing `path`")?;
            let safe_path = verify_workspace_jail(workspace, Path::new(rel))?;
            let content = std::fs::read_to_string(&safe_path).map_err(|e| e.to_string())?;
            let parsed = syn::parse_file(&content).map_err(|e| format!("AST Error: {}", e))?;
            let mut items = Vec::new();
            collect_ast_items(&parsed.items, 0, &mut items);
            let formatted: Vec<String> = items
                .iter()
                .map(|it| format!("{} {} (L{})", it.kind, it.name, it.line))
                .collect();
            Ok(formatted.join("\n"))
        }
        "run_cargo" => {
            let Some(ws) = workspace else {
                return Err("No workspace open".to_string());
            };
            let sub = args["subcommand"].as_str().unwrap_or("check");
            if !matches!(sub, "check" | "test" | "clippy" | "fmt" | "build") {
                return Err(format!("Subcommand `{}` not allowed in MCP sandbox", sub));
            }
            let mut cmd = Command::new("cargo");
            apply_no_window(&mut cmd).arg(sub).current_dir(ws);
            let out = cmd.output().map_err(|e| e.to_string())?;
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            Ok(format!("Exit: {}\n{}\n{}", out.status, stdout, stderr))
        }
        _ => Err(format!("Unknown MCP tool: {}", tool_name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_64_hex_chars() {
        let t = generate_auth_token();
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(t, generate_auth_token(), "连续两次生成的令牌必须不同");
    }
    #[test]
    fn constant_time_eq_behaves_like_eq() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn finds_header_terminator() {
        assert_eq!(find_subsequence(b"a\r\n\r\nb", b"\r\n\r\n"), Some(1));
        assert_eq!(find_subsequence(b"abc", b"\r\n\r\n"), None);
    }

    #[test]
    fn prunes_build_dirs_but_not_lookalikes() {
        let ws = Path::new("C:/ws");
        assert!(path_is_pruned(Path::new("C:/ws/target/debug/a.rs"), ws));
        assert!(path_is_pruned(Path::new("C:/ws/.git/HEAD"), ws));
        assert!(!path_is_pruned(Path::new("C:/ws/src/my_target.rs"), ws));
    }
}

/// 端到端 HTTP 层冒烟测试：真实监听 127.0.0.1 随机端口、真实走 TCP、
/// 覆盖鉴权 / Origin 拦截 / JSON-RPC 分发 / 工作区越界拦截。
#[cfg(test)]
mod http_tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn temp_workspace() -> PathBuf {
        static CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = CTR.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("novars-mcp-test-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 起一个只处理单个连接的"一次性服务器"，返回端口
    fn serve_once(ws: Option<PathBuf>, logs: Option<Arc<Mutex<Vec<McpCallLog>>>>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let logs = logs.unwrap_or_else(|| Arc::new(Mutex::new(Vec::new())));
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                handle_http_connection(stream, ws.as_deref(), &logs, TOKEN);
            }
        });
        port
    }

    fn raw_request(port: u16, body: &str, extra_headers: &str) -> (String, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",
            body.len(),
            extra_headers,
            body
        );
        stream.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        let (head, payload) = resp.split_once("\r\n\r\n").unwrap_or((&resp, ""));
        (head.to_string(), payload.to_string())
    }

    fn authorized(port: u16, body: &str) -> (String, String) {
        raw_request(port, body, &format!("Authorization: Bearer {}\r\n", TOKEN))
    }

    #[test]
    fn rejects_request_without_token() {
        let port = serve_once(None, None);
        let (head, _) = raw_request(port, r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#, "");
        assert!(head.starts_with("HTTP/1.1 401"), "head={head}");
    }

    #[test]
    fn rejects_wrong_token() {
        let port = serve_once(None, None);
        let (head, _) = raw_request(
            port,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            "Authorization: Bearer deadbeef\r\n",
        );
        assert!(head.starts_with("HTTP/1.1 401"), "head={head}");
    }

    #[test]
    fn rejects_browser_origin_even_with_token() {
        let port = serve_once(None, None);
        let (head, _) = raw_request(
            port,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            &format!("Origin: https://evil.example\r\nAuthorization: Bearer {}\r\n", TOKEN),
        );
        assert!(head.starts_with("HTTP/1.1 403"), "head={head}");
    }

    #[test]
    fn initialize_succeeds_with_token() {
        let port = serve_once(None, None);
        let (head, body) = authorized(port, r#"{"jsonrpc":"2.0","id":7,"method":"initialize"}"#);
        assert!(head.starts_with("HTTP/1.1 200"), "head={head}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["result"]["serverInfo"]["name"], "novars-ide-mcp");
    }

    #[test]
    fn notification_gets_202_without_body() {
        let port = serve_once(None, None);
        let (head, body) = authorized(port, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        assert!(head.starts_with("HTTP/1.1 202"), "head={head}");
        assert!(body.is_empty());
    }

    #[test]
    fn tools_list_advertises_input_schemas() {
        let port = serve_once(None, None);
        let (head, body) = authorized(port, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        assert!(head.starts_with("HTTP/1.1 200"));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let tools = v["result"]["tools"].as_array().unwrap();
        assert!(tools.len() >= 8);
        for t in tools {
            assert!(t["inputSchema"].is_object(), "tool {:?} 缺少 inputSchema", t["name"]);
        }
    }

    #[test]
    fn round_trips_write_then_read() {
        let ws = temp_workspace();
        let port = serve_once(Some(ws.clone()), None);
        let call = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"write_file","arguments":{"path":"src/lib.rs","content":"fn main() {}\n"}}}"#;
        let (head, body) = authorized(port, call);
        assert!(head.starts_with("HTTP/1.1 200"));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["isError"], false, "body={body}");
        assert!(ws.join("src").join("lib.rs").is_file());

        let port = serve_once(Some(ws.clone()), None);
        let call = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"read_file","arguments":{"path":"src/lib.rs"}}}"#;
        let (_, body) = authorized(port, call);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("L1: fn main() {}"), "text={text}");

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn blocks_traversal_via_tool_call() {
        let ws = temp_workspace();
        let port = serve_once(Some(ws.clone()), None);
        let call = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"write_file","arguments":{"path":"../../../pwned.txt","content":"x"}}}"#;
        let (_, body) = authorized(port, call);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["isError"], true, "body={body}");
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("沙箱"), "text={text}");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn accepts_body_larger_than_single_socket_read() {
        // 旧实现单次 read 最多 64 KiB，超过即截断 JSON；这里用 200 KiB 正文验证完整性
        let ws = temp_workspace();
        let payload = "a".repeat(200 * 1024);
        let call = serde_json::json!({
            "jsonrpc": "2.0", "id": 6, "method": "tools/call",
            "params": { "name": "write_file", "arguments": { "path": "big.txt", "content": payload } }
        })
        .to_string();
        let port = serve_once(Some(ws.clone()), None);
        let (_, body) = authorized(port, &call);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["isError"], false, "body={body}");
        assert_eq!(std::fs::read_to_string(ws.join("big.txt")).unwrap().len(), 200 * 1024);
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn unknown_method_returns_jsonrpc_error() {
        let port = serve_once(None, None);
        let (_, body) = authorized(port, r#"{"jsonrpc":"2.0","id":9,"method":"tools/bogus"}"#);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["error"]["code"], -32601);
    }
}
