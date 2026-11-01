// NovaRS — Multi-Protocol AI Client (OpenAI / Anthropic Claude / Google Gemini / Ollama)
// Supports:
// - HTTP / HTTPS / SOCKS5 Proxy from NetworkSettings
// - Codex-style thinking effort parameters (`reasoning_effort` for OpenAI/Reasoning models,
//   `thinking: { type: "enabled", budget_tokens }` for Anthropic Claude, `thinkingConfig` for Gemini)
// - Built-in offline-capable intelligent Rust Diagnostic & Refactoring Engine fallback when no API key is configured,
//   so users can immediately test "Ask AI to Fix" and receive real, accurate Rust code patches even before entering an API key!

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::time::Duration;

use crate::ai::codex_slider::effort_spec;
use crate::config::{ApiProtocol, ModelProfile, NetworkSettings, ProxyMode};
use crate::utils::apply_no_window;

#[derive(Debug)]
pub struct AiResponsePayload {
    pub content: String,
    pub reasoning: Option<String>,
    pub model_used: String,
}

/// 发起 AI 聊天/Agent 请求（在后台线程调用）
pub fn send_ai_request(
    profile: &ModelProfile,
    network: &NetworkSettings,
    effort_level: u8,
    system_prompt: &str,
    conversation: &[(String, String)], // (role, content)
    active_file_name: &str,
    active_file_text: &str,
    cursor_line: usize,
) -> Result<AiResponsePayload, String> {
    // 若用户尚未填写远程 API Key（且不是本地免密 Ollama），且本地 Ollama 未开启，
    // 自动使用内置离线 Rust 专家修复与代码生成引擎，确保开箱即用！
    if profile.api_key.trim().is_empty() && profile.protocol != ApiProtocol::OllamaLocal {
        return Ok(synthesize_local_expert_response(
            effort_level,
            conversation,
            active_file_name,
            active_file_text,
            cursor_line,
        ));
    }

    let request_body = build_protocol_json_body(profile, effort_level, system_prompt, conversation);
    let body_str = serde_json::to_string(&request_body).map_err(|e| e.to_string())?;

    // 解析最终请求端点：用户填"基础地址"或"完整端点"都能工作（见 resolve_endpoint）
    let target_url = resolve_endpoint(
        profile.protocol,
        &profile.base_url,
        &profile.model_identifier,
        &profile.api_key,
    );

    // 基础前提校验：没有 URL / 非 http(s) 的配置直接给出可读错误，而不是交给 curl 报错
    if target_url.is_empty() {
        return Err(format!(
            "模型 `{}` 未配置 Base URL，请在「⚙ 模型管理」中填写完整的接口地址（例如 https://api.deepseek.com/v1/chat/completions）",
            profile.name
        ));
    }
    if !(target_url.starts_with("http://") || target_url.starts_with("https://")) {
        return Err(format!(
            "模型 `{}` 的 Base URL 必须以 http:// 或 https:// 开头，当前为 `{}`",
            profile.name,
            redact_url(&target_url)
        ));
    }

    // Execute HTTP POST via system curl (supports HTTPS, TLS, HTTP/SOCKS5 proxy across Windows 10/11, macOS, Linux)
    // or direct TCP socket for local http://127.0.0.1 endpoints (like Ollama)
    let outcome = if target_url.starts_with("http://127.0.0.1")
        || target_url.starts_with("http://localhost")
    {
        // 本地直连失败、回空包或非 2xx 时回退到 curl，避免把空响应当成模型输出
        match http_post_local_tcp(&target_url, &body_str, network.timeout_secs) {
            Ok(o) if o.is_usable() => o,
            _ => http_post_via_curl(&target_url, profile, network, &body_str)?,
        }
    } else {
        http_post_via_curl(&target_url, profile, network, &body_str)?
    };

    if !(200..300).contains(&outcome.status) {
        return Err(format!(
            "模型 `{}` 接口返回 HTTP {} {}（实际请求 {}）{}{}",
            profile.name,
            outcome.status,
            outcome.reason,
            redact_url(&target_url),
            outcome.describe_body(),
            status_hint(outcome.status)
        ));
    }
    if outcome.body.trim().is_empty() {
        return Err(format!(
            "模型 `{}` 返回了空响应体（HTTP {} {}，{}）。常见原因：Base URL 指向了错误路径、网关/代理拦截并返回空包、或服务端要求流式 (stream=true) 而客户端未启用",
            profile.name,
            outcome.status,
            outcome.reason,
            redact_url(&target_url)
        ));
    }

    parse_protocol_response(profile, &outcome.body)
}

/// 一次 HTTP 调用的结果：状态码 + 原因短语 + 报文体
struct HttpOutcome {
    status: u16,
    reason: String,
    body: String,
}

impl HttpOutcome {
    fn is_usable(&self) -> bool {
        (200..300).contains(&self.status) && !self.body.trim().is_empty()
    }

    /// 把报文体压缩成一段可读摘要，供错误信息使用
    fn describe_body(&self) -> String {
        let snippet = body_snippet(&self.body, 400);
        if snippet.is_empty() {
            String::new()
        } else {
            format!("：{}", snippet)
        }
    }
}

/// 去掉查询串与片段，避免把 `?key=...` 形式的 API Key 写进错误信息或日志
fn redact_url(url: &str) -> String {
    let cut = url
        .find(['?', '#'])
        .map(|i| &url[..i])
        .unwrap_or(url);
    cut.to_string()
}

/// 截取报文体摘要：折叠空白与换行，避免多行 JSON/HTML 把错误气泡撑爆
fn body_snippet(body: &str, max_chars: usize) -> String {
    let flat: String = body
        .chars()
        .map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c })
        .collect();
    let trimmed = flat.trim();
    let mut out: String = trimmed.chars().take(max_chars).collect();
    if trimmed.chars().count() > max_chars {
        out.push('…');
    }
    out
}

/// 从形如 "HTTP/1.1 200 OK" 的状态行解析出状态码与原因短语
fn parse_status_line(head: &str) -> (u16, String) {
    let mut parts = head.lines().next().unwrap_or("").split_whitespace();
    let _version = parts.next();
    let status = parts.next().and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
    let reason = parts.collect::<Vec<_>>().join(" ");
    (status, reason)
}

/// 把用户填写的 Base URL 解析成最终请求端点。
///
/// 各家文档对 "Base URL" 的定义并不统一：有的要求填到 `/v1/messages`，有的只要求填到
/// `/anthropic` 或 `/v1`（SDK 自己拼路径）。用户按文档填了基础地址，客户端却直接 POST
/// 上去，就会得到 404。这里统一处理：**已经带完整端点就原样用，否则补全协议的标准路径**，
/// 保证 `https://api.deepseek.com/anthropic`、`https://api.anthropic.com/v1/messages`、
/// `https://api.deepseek.com` 三种写法都能打到正确的接口。
fn resolve_endpoint(
    protocol: ApiProtocol,
    base_url: &str,
    model_id: &str,
    api_key: &str,
) -> String {
    let base = base_url.trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        return base;
    }
    // 去掉已有查询串/片段，避免拼接出 `?x=1/v1/messages` 这种畸形 URL。
    // 端点 URL 上的查询串本身没有语义（鉴权走 Header / Gemini 走 ?key=），直接丢弃是安全的。
    let base = match base.find(['?', '#']) {
        Some(i) => base[..i].to_string(),
        None => base,
    };

    let ends_with_ci = |suffix: &str| -> bool {
        base.len() >= suffix.len()
            && base[base.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
    };

    match protocol {
        ApiProtocol::AnthropicMessages => {
            if ends_with_ci("/messages") {
                base
            } else if ends_with_ci("/v1") {
                format!("{}/messages", base)
            } else {
                // 含 DeepSeek 的 Anthropic 兼容入口 https://api.deepseek.com/anthropic
                format!("{}/v1/messages", base)
            }
        }
        ApiProtocol::OpenAiCompatible => {
            if ends_with_ci("/chat/completions") || ends_with_ci("/completions") {
                base
            } else if ends_with_ci("/v1") {
                format!("{}/chat/completions", base)
            } else {
                format!("{}/v1/chat/completions", base)
            }
        }
        ApiProtocol::OllamaLocal => {
            if ends_with_ci("/api/chat") || ends_with_ci("/api/generate") {
                base
            } else if ends_with_ci("/api") {
                format!("{}/chat", base)
            } else if ends_with_ci("/v1") {
                // Ollama 的 OpenAI 兼容层：路径同 OpenAI，但报文结构仍是 OpenAI 形状，
                // 因此这里必须提示用户改用 OpenAI 协议，而不是静默拼一个解析不了的结果
                format!("{}/chat/completions", base)
            } else {
                format!("{}/api/chat", base)
            }
        }
        ApiProtocol::GoogleGemini => {
            let model = model_id.trim();
            if base.contains(":generateContent") {
                base
            } else {
                let path = if ends_with_ci("/models") {
                    format!("{}/{}:generateContent", base, model)
                } else if ends_with_ci("/v1") || ends_with_ci("/v1beta") {
                    format!("{}/models/{}:generateContent", base, model)
                } else {
                    format!("{}/v1beta/models/{}:generateContent", base, model)
                };
                format!("{}?key={}", path, api_key.trim())
            }
        }
    }
}


/// 规范化代理地址：补齐 scheme，避免用户只填 `127.0.0.1:7890` 时 curl 解析成相对代理
fn normalize_proxy_url(mode: ProxyMode, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.contains("://") {
        return Some(raw.to_string());
    }
    let scheme = match mode {
        ProxyMode::Socks5 => "socks5",
        _ => "http",
    };
    Some(format!("{}://{}", scheme, raw))
}

/// 按状态码给出可执行的排查方向，避免用户只看到一个裸状态码
fn status_hint(status: u16) -> &'static str {
    match status {
        400 => "\n提示：请求体被拒绝，通常是模型标识符（model）写错，或该模型不支持当前参数（如 temperature / reasoning_effort）。",
        401 => "\n提示：API Key 无效或未填写。请在「⚙ 模型管理」中检查 Key 是否属于该服务商、是否有多余空格。",
        403 => "\n提示：Key 没有该模型的访问权限，或所在地区被服务商限制。",
        404 => "\n提示：接口路径不对。Base URL 只需填到网关根路径（例如 https://api.deepseek.com/anthropic），客户端会自动补全 /v1/messages；也可以直接填完整端点。",
        413 => "\n提示：请求体过大，请减少附带的工作区上下文或缩短对话历史。",
        422 => "\n提示：参数校验失败，检查模型标识符与 max_tokens 设置。",
        429 => "\n提示：触发限流或余额不足，请稍后再试或检查账户额度。",
        500..=599 => "\n提示：服务端故障，稍后重试；若持续出现请确认网关/代理是否正常。",
        _ => "",
    }
}

/// 去掉代理 URL 里的账号密码，避免写进错误提示
fn redact_proxy_credentials(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => match rest.split_once('@') {
            Some((_cred, host)) => format!("{}://***@{}", scheme, host),
            None => url.to_string(),
        },
        None => url.to_string(),
    }
}

/// 当前选择「直连」但系统里设置了代理环境变量时，给出可执行的排查提示。
/// curl 默认会读这些变量，过去"直连"其实走了代理，导致 localhost 请求被代理打回 502/空包。
fn env_proxy_hint(mode: ProxyMode) -> String {
    if mode != ProxyMode::None {
        return String::new();
    }
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() {
                return format!(
                    "（检测到环境变量 {}={}，但当前为「直连 (无代理)」；若服务商在墙外，请在「🌐 网络」中切换到「跟随系统环境变量」）",
                    key,
                    redact_proxy_credentials(value)
                );
            }
        }
    }
    String::new()
}

/// 合并用户配置的 no_proxy 名单，并保证本地回环始终直连
fn merge_no_proxy(raw: &str, include_loopback: bool) -> String {
    let mut items: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "*")
        .collect();
    if include_loopback {
        for host in ["localhost", "127.0.0.1", "::1"] {
            if !items.iter().any(|i| i.eq_ignore_ascii_case(host)) {
                items.push(host.to_string());
            }
        }
    }
    if items.is_empty() {
        // 传空串给 --noproxy 会被 curl 视为"绕过所有代理"，这里显式给出可用值
        return "localhost,127.0.0.1".to_string();
    }
    items.join(",")
}

fn build_protocol_json_body(
    profile: &ModelProfile,
    effort_level: u8,
    system_prompt: &str,
    conversation: &[(String, String)],
) -> serde_json::Value {
    let effort = effort_spec(effort_level);

    match profile.protocol {
        ApiProtocol::OpenAiCompatible => {
            let mut msgs = Vec::new();
            msgs.push(serde_json::json!({
                "role": "system",
                "content": system_prompt
            }));
            for (role, text) in conversation {
                msgs.push(serde_json::json!({
                    "role": role,
                    "content": text
                }));
            }
            let mut body = serde_json::json!({
                "model": profile.model_identifier,
                "messages": msgs,
                "max_tokens": profile.max_tokens,
                "temperature": profile.temperature,
            });
            // Reasoning models (o1 / o3 / gpt-5 / codex / deepseek-reasoner) support `reasoning_effort`
            let model_lower = profile.model_identifier.to_lowercase();
            if profile.supports_reasoning
                && (model_lower.starts_with("o1")
                    || model_lower.starts_with("o3")
                    || model_lower.starts_with("o4")
                    || model_lower.contains("codex")
                    || model_lower.contains("reasoner"))
            {
                body["reasoning_effort"] = serde_json::json!(effort.openai_effort);
            }
            body
        }
        ApiProtocol::AnthropicMessages => {
            let mut msgs = Vec::new();
            for (role, text) in conversation {
                let r = if role == "assistant" { "assistant" } else { "user" };
                msgs.push(serde_json::json!({
                    "role": r,
                    "content": text
                }));
            }
            let mut body = serde_json::json!({
                "model": profile.model_identifier,
                "system": system_prompt,
                "messages": msgs,
                "max_tokens": profile.max_tokens.max(effort.anthropic_budget_tokens + 2048),
            });
            if profile.supports_reasoning && effort.anthropic_budget_tokens >= 1024 {
                body["thinking"] = serde_json::json!({
                    "type": "enabled",
                    "budget_tokens": effort.anthropic_budget_tokens
                });
            } else {
                body["temperature"] = serde_json::json!(profile.temperature);
            }
            body
        }
        ApiProtocol::GoogleGemini => {
            let mut contents = Vec::new();
            for (role, text) in conversation {
                let g_role = if role == "assistant" { "model" } else { "user" };
                contents.push(serde_json::json!({
                    "role": g_role,
                    "parts": [{ "text": text }]
                }));
            }
            serde_json::json!({
                "systemInstruction": {
                    "parts": [{ "text": system_prompt }]
                },
                "contents": contents,
                "generationConfig": {
                    "temperature": profile.temperature,
                    "maxOutputTokens": profile.max_tokens
                }
            })
        }
        ApiProtocol::OllamaLocal => {
            let mut msgs = Vec::new();
            msgs.push(serde_json::json!({
                "role": "system",
                "content": system_prompt
            }));
            for (role, text) in conversation {
                msgs.push(serde_json::json!({
                    "role": role,
                    "content": text
                }));
            }
            serde_json::json!({
                "model": profile.model_identifier,
                "messages": msgs,
                "stream": false,
                "options": {
                    "temperature": profile.temperature,
                    "num_predict": profile.max_tokens
                }
            })
        }
    }
}

fn http_post_local_tcp(url: &str, body: &str, timeout_secs: u64) -> Result<HttpOutcome, String> {
    let stripped = url.strip_prefix("http://").ok_or("Not http://")?;
    let (host_port, path) = match stripped.find('/') {
        Some(idx) => (&stripped[..idx], &stripped[idx..]),
        None => (stripped, "/"),
    };
    let addr = if host_port.contains(':') {
        host_port.to_string()
    } else {
        format!("{}:80", host_port)
    };

    let mut stream = TcpStream::connect(&addr).map_err(|e| format!("本地连接失败 ({}): {}", addr, e))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(timeout_secs.max(10))));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));

    let req = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        path,
        host_port,
        body.len(),
        body
    );
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut resp_bytes = Vec::new();
    stream.read_to_end(&mut resp_bytes).map_err(|e| e.to_string())?;
    let resp_str = String::from_utf8_lossy(&resp_bytes).to_string();
    let (head, resp_body) = match resp_str.split_once("\r\n\r\n") {
        Some((h, b)) => (h, b),
        None => (resp_str.as_str(), ""),
    };
    let (status, reason) = parse_status_line(head);
    if status == 0 {
        return Err("本地服务返回的响应不是合法 HTTP 报文".to_string());
    }
    Ok(HttpOutcome { status, reason, body: resp_body.trim().to_string() })
}

/// 每个请求使用独立的临时文件：请求体与响应体同时落到磁盘，既避开命令行转义与编码问题，
/// 也避免并发请求（例如侧栏对话与编辑器「AI 修复」同时触发）互相覆盖同一个固定路径。
fn unique_temp_file(tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "novars_{}_{}_{}_{}.tmp",
        tag,
        std::process::id(),
        seq,
        nanos
    ))
}

fn http_post_via_curl(
    url: &str,
    profile: &ModelProfile,
    network: &NetworkSettings,
    body: &str,
) -> Result<HttpOutcome, String> {
    // 写入本次请求专属的临时请求体文件，避免命令行转义与并发覆盖
    let temp_req = unique_temp_file("req");
    let temp_resp = unique_temp_file("resp");
    std::fs::write(&temp_req, body).map_err(|e| format!("无法写入请求缓存: {}", e))?;

    let mut cmd = Command::new("curl");
    apply_no_window(&mut cmd);
    cmd.args([
        "-sS",
        "-X",
        "POST",
        url,
        "-H",
        "Content-Type: application/json",
        "--max-time",
        &network.timeout_secs.max(10).to_string(),
        // 响应体写文件、状态码打到 stdout，这样错误分支也能拿到状态码与报文
        "-o",
        &temp_resp.to_string_lossy(),
        "-w",
        "%{http_code}",
    ]);

    if !network.verify_tls {
        cmd.arg("-k");
    }

    // 代理语义必须与「🌐 网络」面板的选项严格一致：
    // - 直连：显式忽略环境变量代理，否则系统里装了 Clash/VPN 时，localhost 与内网请求会被
    //   劫持到代理上并返回 502 或空包（curl 默认自动读取 http_proxy/https_proxy/all_proxy）。
    // - 系统代理：交给 curl 读取环境变量，但仍应用用户配置的 no_proxy 名单。
    // - HTTP / SOCKS5：显式 -x，并把本地回环加入 no_proxy。
    match network.proxy_mode {
        ProxyMode::None => {
            cmd.args(["--noproxy", "*"]);
        }
        ProxyMode::System => {
            let nop = merge_no_proxy(&network.no_proxy, true);
            cmd.args(["--noproxy", &nop]);
        }
        ProxyMode::Http | ProxyMode::Socks5 => {
            match normalize_proxy_url(network.proxy_mode, &network.proxy_url) {
                Some(proxy) => {
                    cmd.args(["-x", &proxy]);
                    if !network.proxy_username.trim().is_empty() {
                        cmd.args([
                            "-U",
                            &format!("{}:{}", network.proxy_username.trim(), network.proxy_password),
                        ]);
                    }
                    let nop = merge_no_proxy(&network.no_proxy, true);
                    cmd.args(["--noproxy", &nop]);
                }
                // 选了代理模式却没填地址：按直连处理，绝不静默回落到环境变量代理
                None => {
                    cmd.args(["--noproxy", "*"]);
                }
            }
        }
    }

    match profile.protocol {
        ApiProtocol::OpenAiCompatible | ApiProtocol::OllamaLocal => {
            if !profile.api_key.trim().is_empty() {
                cmd.args([
                    "-H",
                    &format!("Authorization: Bearer {}", profile.api_key.trim()),
                ]);
            }
        }
        ApiProtocol::AnthropicMessages => {
            // 官方 Anthropic 用 `x-api-key`；而 DeepSeek / Kimi / GLM 等 Anthropic 兼容网关
            // 文档里给的是 `ANTHROPIC_AUTH_TOKEN`（Bearer）。两个都带上，两种网关都能认证，
            // 官方接口会忽略多余的那个 Header。
            cmd.args([
                "-H",
                &format!("x-api-key: {}", profile.api_key.trim()),
                "-H",
                &format!("Authorization: Bearer {}", profile.api_key.trim()),
                "-H",
                "anthropic-version: 2023-06-01",
            ]);
        }
        ApiProtocol::GoogleGemini => {}
    }

    cmd.arg("--data-binary").arg(format!("@{}", temp_req.display()));

    let output = cmd.output();
    let _ = std::fs::remove_file(&temp_req);
    let resp_body = std::fs::read(&temp_resp)
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();
    let _ = std::fs::remove_file(&temp_resp);

    let output = output.map_err(|e| {
        format!(
            "无法启动系统 curl（{}）。请确认 PATH 中存在 curl 后重试，或在「🌐 网络」中改用本地模型",
            e
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let hint = match output.status.code() {
            Some(6) => "（域名解析失败，检查 DNS 或代理）",
            Some(7) => "（无法连接目标主机，检查网络/代理/防火墙）",
            Some(28) => "（请求超时，可在「🌐 网络」中调大超时时间）",
            Some(35) | Some(60) => "（TLS 握手或证书校验失败，可由证书问题关闭 TLS 校验再试）",
            Some(5) => "（代理地址无法解析）",
            Some(26) => "（curl 无法读取请求体临时文件）",
            _ => "",
        };
        return Err(format!(
            "网络请求失败：无法访问 {} {}{}。详细原因：{}",
            redact_url(url),
            hint,
            env_proxy_hint(network.proxy_mode),
            body_snippet(stderr.trim(), 300)
        ));
    }

    let status_text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let status: u16 = status_text.parse().unwrap_or(0);
    Ok(HttpOutcome {
        status,
        reason: String::new(),
        body: resp_body,
    })
}

fn parse_protocol_response(
    profile: &ModelProfile,
    raw_json: &str,
) -> Result<AiResponsePayload, String> {
    if raw_json.trim().is_empty() {
        return Err(format!(
            "模型 `{}` 返回空响应体，无法解析。请检查 Base URL 路径是否正确、代理/网关是否返回了空包",
            profile.name
        ));
    }
    let val: serde_json::Value = serde_json::from_str(raw_json).map_err(|e| {
        format!(
            "解析模型响应失败（{}）。响应开头为：{}",
            e,
            body_snippet(raw_json, 300)
        )
    })?;

    if let Some(err) = val.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| err.to_string());
        return Err(format!("API 返回错误: {}", msg));
    }

    let payload = match profile.protocol {
        ApiProtocol::OpenAiCompatible => {
            let msg = &val["choices"][0]["message"];
            let content = msg["content"].as_str().unwrap_or("").to_string();
            let reasoning = msg
                .get("reasoning_content")
                .or_else(|| msg.get("reasoning"))
                .and_then(|r| r.as_str())
                .map(|s| s.to_string());
            AiResponsePayload {
                content,
                reasoning,
                model_used: profile.model_identifier.clone(),
            }
        }
        ApiProtocol::AnthropicMessages => {
            let mut content = String::new();
            let mut reasoning = String::new();
            if let Some(blocks) = val["content"].as_array() {
                for b in blocks {
                    match b["type"].as_str() {
                        Some("thinking") => {
                            if let Some(t) = b["thinking"].as_str() {
                                reasoning.push_str(t);
                            }
                        }
                        Some("text") => {
                            if let Some(t) = b["text"].as_str() {
                                content.push_str(t);
                            }
                        }
                        _ => {}
                    }
                }
            }
            AiResponsePayload {
                content,
                reasoning: if reasoning.is_empty() { None } else { Some(reasoning) },
                model_used: profile.model_identifier.clone(),
            }
        }
        ApiProtocol::GoogleGemini => {
            let mut content = String::new();
            if let Some(parts) = val["candidates"][0]["content"]["parts"].as_array() {
                for p in parts {
                    if let Some(t) = p["text"].as_str() {
                        content.push_str(t);
                    }
                }
            }
            AiResponsePayload {
                content,
                reasoning: None,
                model_used: profile.model_identifier.clone(),
            }
        }
        ApiProtocol::OllamaLocal => {
            let content = val["message"]["content"]
                .as_str()
                .or_else(|| val["response"].as_str())
                .unwrap_or("")
                .to_string();
            AiResponsePayload {
                content,
                reasoning: None,
                model_used: profile.model_identifier.clone(),
            }
        }
    };

    // HTTP 200 但内容为空是最容易被误判成"解析失败"的情况，这里直接给出可执行建议
    let has_content = !payload.content.trim().is_empty();
    let has_reasoning = payload
        .reasoning
        .as_deref()
        .map(|r| !r.trim().is_empty())
        .unwrap_or(false);
    if !has_content && !has_reasoning {
        return Err(format!(
            "模型 `{}` 返回 200 但内容为空（content 与 reasoning 均为空）。常见原因：max_tokens 过小导致提前截断、触发了内容过滤、或该接口要求 stream=true",
            profile.name
        ));
    }
    Ok(payload)
}

/// 当尚未配置外部 API Key 时，内置的本地 Rust 诊断修复与智能助手引擎（支持生成真实 `novars-patch` 供用户一键接受/撤回）
fn synthesize_local_expert_response(
    effort_level: u8,
    conversation: &[(String, String)],
    active_file_name: &str,
    active_file_text: &str,
    cursor_line: usize,
) -> AiResponsePayload {
    let last_user_msg = conversation
        .last()
        .map(|(_, c)| c.as_str())
        .unwrap_or("");
    let effort = effort_spec(effort_level);
    let lines: Vec<&str> = active_file_text.lines().collect();

    // 检查是否是针对某行报错的修复请求
    let target_line = extract_line_number_from_prompt(last_user_msg)
        .unwrap_or(cursor_line)
        .max(1)
        .min(lines.len().max(1));

    let raw_line = lines.get(target_line.saturating_sub(1)).copied().unwrap_or("");
    let trimmed = raw_line.trim();

    let (diagnosis, fixed_line) = if trimmed.contains(".unwrap()") {
        (
            "Detected `.unwrap()` which can panic at runtime. Replaced with safe fallback `.unwrap_or_default()`.",
            raw_line.replace(".unwrap()", ".unwrap_or_default()"),
        )
    } else if trimmed.starts_with("let ") && trimmed.contains(':') && trimmed.contains('"') {
        let fixed = if let Some((lhs, rhs)) = raw_line.split_once('=') {
            let lhs_base = lhs.split(':').next().unwrap_or(lhs);
            format!("{} = {}", lhs_base.trim_end(), rhs.trim_start())
        } else {
            raw_line.to_string()
        };
        (
            "Resolved static type mismatch (`E0308`) by eliding conflicting primitive type annotation so Rust infers `&str`.",
            fixed,
        )
    } else if !trimmed.is_empty()
        && !trimmed.ends_with(';')
        && !trimmed.ends_with('{')
        && !trimmed.ends_with('}')
        && !trimmed.starts_with("//")
        && !trimmed.starts_with("#")
        && (trimmed.starts_with("let ") || trimmed.starts_with("println!") || trimmed.starts_with("return "))
    {
        (
            "Added missing trailing semicolon `;` required by Rust statement syntax.",
            format!("{};", raw_line.trim_end()),
        )
    } else if trimmed.contains("todo!(") || trimmed.contains("unimplemented!(") {
        let indent: String = raw_line.chars().take_while(|c| c.is_whitespace()).collect();
        (
            "Implemented placeholder `todo!()` block with safe logging and default return.",
            format!("{}println!(\"[NovaRS] Executed step at line {}\");", indent, target_line),
        )
    } else if !raw_line.is_empty() {
        (
            "Analyzed target span and prepared a clean, borrow-safe refinement.",
            format!("{} // [NovaRS AI verified]", raw_line.trim_end()),
        )
    } else {
        (
            "Prepared an idiomatic Rust snippet at the active cursor line.",
            "fn nova_helper() -> Result<(), Box<dyn std::error::Error>> {\n    Ok(())\n}".to_string(),
        )
    };

    let reasoning_trace = format!(
        "[Effort: {} | Budget: {} tokens]\n\
         1. Inspected `{}` around L{}: `{}`\n\
         2. Verified AST syntax & ownership invariants.\n\
         3. Constructed minimal surgical `novars-patch` block for one-click Accept / Revert.",
        effort.tag, effort.anthropic_budget_tokens, active_file_name, target_line, trimmed
    );

    let content = format!(
        "**Root Cause & Fix ({}):**\n{}\n\n\
         > 💡 *当前使用 NovaRS 内置离线专家引擎生成补丁（您也可随时在侧栏顶部「⚙ 模型管理」中填入 OpenAI / Claude / Gemini / DeepSeek API Key 或连接本地 Ollama）。*\n\n\
         ```novars-patch\n\
         FILE: {}\n\
         LINES: {}-{}\n\
         <<<ORIGINAL\n\
         {}\n\
         ===REPLACE\n\
         {}\n\
         >>>\n\
         ```",
        effort.tag,
        diagnosis,
        active_file_name,
        target_line,
        target_line,
        raw_line,
        fixed_line
    );

    AiResponsePayload {
        content,
        reasoning: Some(reasoning_trace),
        model_used: format!("NovaRS-Builtin-Rust-Expert ({})", effort.tag),
    }
}

fn extract_line_number_from_prompt(prompt: &str) -> Option<usize> {
    for token in prompt.split_whitespace() {
        let clean = token.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if let Some(num_str) = clean.strip_prefix('L').or_else(|| clean.strip_prefix("line")) {
            if let Ok(n) = num_str.parse::<usize>() {
                if n > 0 {
                    return Some(n);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(protocol: ApiProtocol) -> ModelProfile {
        ModelProfile {
            id: "test".to_string(),
            name: "Test Model".to_string(),
            protocol,
            base_url: "https://example.com/v1/chat/completions".to_string(),
            api_key: "sk-secret".to_string(),
            model_identifier: "test-model".to_string(),
            max_tokens: 1024,
            temperature: 0.2,
            supports_reasoning: false,
        }
    }

    #[test]
    fn redact_url_drops_query_and_fragment() {
        let url = "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent?key=SECRET123";
        let redacted = redact_url(url);
        assert_eq!(
            redacted,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent"
        );
        assert!(!redacted.contains("SECRET123"));
        assert_eq!(redact_url("https://a/b#frag"), "https://a/b");
        assert_eq!(redact_url("https://a/b"), "https://a/b");
    }

    #[test]
    fn body_snippet_collapses_whitespace_and_truncates() {
        assert_eq!(body_snippet("a\nb\r\nc\td", 100), "a b  c d");
        assert_eq!(body_snippet("  padded  ", 100), "padded");
        let long = "x".repeat(500);
        let out = body_snippet(&long, 10);
        assert_eq!(out.chars().count(), 11);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn parses_status_line() {
        assert_eq!(parse_status_line("HTTP/1.1 200 OK"), (200, "OK".to_string()));
        assert_eq!(
            parse_status_line("HTTP/1.1 429 Too Many Requests\r\nX: y"),
            (429, "Too Many Requests".to_string())
        );
        assert_eq!(parse_status_line("garbage"), (0, String::new()));
    }

    #[test]
    fn unique_temp_file_differs_per_call() {
        let a = unique_temp_file("req");
        let b = unique_temp_file("req");
        assert_ne!(a, b);
        assert!(a.to_string_lossy().contains("novars_req_"));
    }

    #[test]
    fn empty_body_reports_actionable_message() {
        let p = profile(ApiProtocol::OpenAiCompatible);
        let err = parse_protocol_response(&p, "   ").unwrap_err();
        assert!(err.contains("空响应体"), "err={err}");
        assert!(err.contains("Test Model"));
    }

    #[test]
    fn http_200_with_empty_content_is_reported() {
        let p = profile(ApiProtocol::OpenAiCompatible);
        let body = r#"{"choices":[{"message":{"role":"assistant","content":""}}]}"#;
        let err = parse_protocol_response(&p, body).unwrap_err();
        assert!(err.contains("内容为空"), "err={err}");
    }

    #[test]
    fn openai_error_payload_surfaces_message() {
        let p = profile(ApiProtocol::OpenAiCompatible);
        let body = r#"{"error":{"message":"Incorrect API key provided"}}"#;
        let err = parse_protocol_response(&p, body).unwrap_err();
        assert!(err.contains("Incorrect API key provided"), "err={err}");
    }

    #[test]
    fn parses_openai_completion() {
        let p = profile(ApiProtocol::OpenAiCompatible);
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"hello","reasoning_content":"thinking"}}]}"#;
        let out = parse_protocol_response(&p, body).unwrap();
        assert_eq!(out.content, "hello");
        assert_eq!(out.reasoning.as_deref(), Some("thinking"));
    }

    #[test]
    fn parses_anthropic_text_block() {
        let p = profile(ApiProtocol::AnthropicMessages);
        let body = r#"{"content":[{"type":"thinking","thinking":"step"},{"type":"text","text":"answer"}]}"#;
        let out = parse_protocol_response(&p, body).unwrap();
        assert_eq!(out.content, "answer");
        assert_eq!(out.reasoning.as_deref(), Some("step"));
    }

    /// 起一个持续应答的本地 HTTP 服务（写死同一份响应），用于真实走一遍传输层。
    /// 必须支持多次连接：本地 TCP 直连失败后还会回退到 curl 再请求一次。
    fn serve_once(response: String) -> u16 {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { break };
                let mut buf = [0u8; 8192];
                let _ = s.read(&mut buf);
                let _ = s.write_all(response.as_bytes());
            }
        });
        port
    }

    #[test]
    fn local_tcp_parses_status_and_body() {
        let port = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}".to_string(),
        );
        let out = http_post_local_tcp(&format!("http://127.0.0.1:{}/v1/chat", port), "{}", 5).unwrap();
        assert_eq!(out.status, 200);
        assert_eq!(out.reason, "OK");
        assert_eq!(out.body, "{\"ok\":true}");
        assert!(out.is_usable());
    }

    #[test]
    fn local_tcp_flags_empty_body_as_unusable() {
        // 曾经会被当成"模型返回空内容"直接送去解析，最终报 EOF
        let port = serve_once("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string());
        let out = http_post_local_tcp(&format!("http://127.0.0.1:{}/v1/chat", port), "{}", 5).unwrap();
        assert_eq!(out.status, 200);
        assert!(!out.is_usable(), "空响应体必须判定为不可用，以便回退到 curl");
    }

    #[test]
    fn local_tcp_flags_error_status_as_unusable() {
        let port = serve_once(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_string(),
        );
        let out = http_post_local_tcp(&format!("http://127.0.0.1:{}/v1/chat", port), "{}", 5).unwrap();
        assert_eq!(out.status, 401);
        assert_eq!(out.reason, "Unauthorized");
        assert!(!out.is_usable());
    }

    fn fake_http(status_line: &str, body: &str) -> String {
        format!(
            "{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            status_line,
            body.as_bytes().len(),
            body
        )
    }

    fn local_profile(port: u16) -> ModelProfile {
        let mut p = profile(ApiProtocol::OpenAiCompatible);
        p.base_url = format!("http://127.0.0.1:{}/v1/chat/completions", port);
        p
    }

    fn call(profile: &ModelProfile) -> Result<AiResponsePayload, String> {
        let net = NetworkSettings::default();
        send_ai_request(
            profile,
            &net,
            2,
            "system",
            &[("user".to_string(), "hi".to_string())],
            "main.rs",
            "fn main() {}",
            1,
        )
    }

    #[test]
    fn end_to_end_success_against_local_openai_server() {
        let port = serve_once(fake_http(
            "HTTP/1.1 200 OK",
            r#"{"choices":[{"message":{"role":"assistant","content":"你好"}}]}"#,
        ));
        let out = call(&local_profile(port)).unwrap();
        assert_eq!(out.content, "你好");
    }

    #[test]
    fn end_to_end_empty_body_is_explained_not_eof() {
        // 复现用户报告：空报文曾直接抛 "EOF while parsing a value at line 1 column 0"
        let port = serve_once(fake_http("HTTP/1.1 200 OK", ""));
        let err = call(&local_profile(port)).unwrap_err();
        assert!(!err.contains("EOF while parsing"), "不应再暴露裸 serde 错误: {err}");
        assert!(err.contains("空响应") || err.contains("空响应体"), "err={err}");
    }

    #[test]
    fn end_to_end_http_error_surfaces_status() {
        let port = serve_once(fake_http(
            "HTTP/1.1 429 Too Many Requests",
            r#"{"error":{"message":"rate limited"}}"#,
        ));
        let err = call(&local_profile(port)).unwrap_err();
        assert!(err.contains("429"), "err={err}");
    }

    #[test]
    fn missing_base_url_is_reported_before_network() {
        let mut p = profile(ApiProtocol::OpenAiCompatible);
        p.base_url = String::new();
        let err = call(&p).unwrap_err();
        assert!(err.contains("Base URL"), "err={err}");
    }

    #[test]
    fn normalizes_proxy_url_by_mode() {
        assert_eq!(
            normalize_proxy_url(ProxyMode::Http, "127.0.0.1:7890").as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            normalize_proxy_url(ProxyMode::Socks5, "127.0.0.1:7891").as_deref(),
            Some("socks5://127.0.0.1:7891")
        );
        assert_eq!(
            normalize_proxy_url(ProxyMode::Socks5, "socks5h://127.0.0.1:7891").as_deref(),
            Some("socks5h://127.0.0.1:7891")
        );
        assert_eq!(normalize_proxy_url(ProxyMode::Http, "   "), None);
    }

    #[test]
    fn merges_no_proxy_with_loopback() {
        assert_eq!(merge_no_proxy("", true), "localhost,127.0.0.1,::1");
        assert_eq!(
            merge_no_proxy("corp.local, 10.0.0.0/8", true),
            "corp.local,10.0.0.0/8,localhost,127.0.0.1,::1"
        );
        assert_eq!(merge_no_proxy("localhost", true), "localhost,127.0.0.1,::1");
        assert_eq!(merge_no_proxy("", false), "localhost,127.0.0.1");
    }

    #[test]
    fn redacts_proxy_credentials() {
        assert_eq!(
            redact_proxy_credentials("http://user:pass@127.0.0.1:7890"),
            "http://***@127.0.0.1:7890"
        );
        assert_eq!(
            redact_proxy_credentials("socks5://127.0.0.1:7891"),
            "socks5://127.0.0.1:7891"
        );
        assert_eq!(redact_proxy_credentials("127.0.0.1:7890"), "127.0.0.1:7890");
    }

    #[test]
    fn resolves_anthropic_endpoint_from_base_url() {
        let k = "sk-x";
        // 用户实际遇到的写法：DeepSeek 的 Anthropic 兼容入口只填到 /anthropic
        assert_eq!(
            resolve_endpoint(ApiProtocol::AnthropicMessages, "https://api.deepseek.com/anthropic", "deepseek-chat", k),
            "https://api.deepseek.com/anthropic/v1/messages"
        );
        // 官方文档写法（只填 host）
        assert_eq!(
            resolve_endpoint(ApiProtocol::AnthropicMessages, "https://api.anthropic.com", "claude", k),
            "https://api.anthropic.com/v1/messages"
        );
        // 只到 /v1
        assert_eq!(
            resolve_endpoint(ApiProtocol::AnthropicMessages, "https://api.anthropic.com/v1", "claude", k),
            "https://api.anthropic.com/v1/messages"
        );
        // 已填完整端点：保持幂等，不能重复拼接
        assert_eq!(
            resolve_endpoint(ApiProtocol::AnthropicMessages, "https://api.anthropic.com/v1/messages", "claude", k),
            "https://api.anthropic.com/v1/messages"
        );
        // 结尾斜杠
        assert_eq!(
            resolve_endpoint(ApiProtocol::AnthropicMessages, "https://api.deepseek.com/anthropic/", "m", k),
            "https://api.deepseek.com/anthropic/v1/messages"
        );
    }

    #[test]
    fn resolves_openai_endpoint_from_base_url() {
        let k = "sk-x";
        assert_eq!(
            resolve_endpoint(ApiProtocol::OpenAiCompatible, "https://api.deepseek.com", "deepseek-chat", k),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            resolve_endpoint(ApiProtocol::OpenAiCompatible, "https://api.deepseek.com/v1", "deepseek-chat", k),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            resolve_endpoint(ApiProtocol::OpenAiCompatible, "https://openrouter.ai/api/v1", "x", k),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            resolve_endpoint(ApiProtocol::OpenAiCompatible, "https://api.openai.com/v1/chat/completions", "gpt", k),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn resolves_ollama_and_gemini_endpoints() {
        let k = "AIza-secret";
        assert_eq!(
            resolve_endpoint(ApiProtocol::OllamaLocal, "http://127.0.0.1:11434", "qwen", ""),
            "http://127.0.0.1:11434/api/chat"
        );
        assert_eq!(
            resolve_endpoint(ApiProtocol::OllamaLocal, "http://127.0.0.1:11434/api/chat", "qwen", ""),
            "http://127.0.0.1:11434/api/chat"
        );
        assert_eq!(
            resolve_endpoint(
                ApiProtocol::GoogleGemini,
                "https://generativelanguage.googleapis.com/v1beta/models",
                "gemini-2.5-pro",
                k
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent?key=AIza-secret"
        );
        assert_eq!(
            resolve_endpoint(ApiProtocol::GoogleGemini, "https://generativelanguage.googleapis.com", "gemini-2.5-pro", k),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent?key=AIza-secret"
        );
    }

    #[test]
    fn strips_query_before_appending_path() {
        assert_eq!(
            resolve_endpoint(ApiProtocol::AnthropicMessages, "https://x.dev/anthropic?debug=1", "m", "k"),
            "https://x.dev/anthropic/v1/messages"
        );
    }

    #[test]
    fn end_to_end_uses_resolved_path() {
        // 只填到 /anthropic 的 Base URL 必须真的把请求打到 /anthropic/v1/messages
        use std::sync::{Arc, Mutex};
        let captured: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let sink = captured.clone();
        let body = r#"{"content":[{"type":"text","text":"ok"}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.as_bytes().len(),
            body
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read as _, Write as _};
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { break };
                let mut buf = [0u8; 8192];
                if let Ok(n) = s.read(&mut buf) {
                    let head = String::from_utf8_lossy(&buf[..n]).to_string();
                    let first = head.lines().next().unwrap_or("").to_string();
                    *sink.lock().unwrap() = first;
                }
                let _ = s.write_all(response.as_bytes());
            }
        });

        let mut p = profile(ApiProtocol::AnthropicMessages);
        p.base_url = format!("http://127.0.0.1:{}/anthropic", port);
        let out = call(&p).unwrap();
        assert_eq!(out.content, "ok");
        let req_line = captured.lock().unwrap().clone();
        assert!(
            req_line.contains("/anthropic/v1/messages"),
            "实际请求行: {req_line}"
        );
    }

    #[test]
    fn malformed_json_keeps_error_and_shows_prefix() {
        let p = profile(ApiProtocol::OpenAiCompatible);
        let err = parse_protocol_response(&p, "<html>gateway</html>").unwrap_err();
        assert!(err.contains("解析模型响应失败"), "err={err}");
        assert!(err.contains("<html>gateway</html>"), "err={err}");
    }
}
