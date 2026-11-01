// NovaRS — Persistent Configuration, Network/Proxy Settings, AI & MCP Settings

use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use crate::constants::DEFAULT_MCP_PORT;
use crate::theme::ThemePreset;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ProxyMode {
    None,
    System,
    Http,
    Socks5,
}

impl Default for ProxyMode {
    fn default() -> Self {
        ProxyMode::None
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkSettings {
    pub proxy_mode: ProxyMode,
    pub proxy_url: String,
    pub proxy_username: String,
    pub proxy_password: String,
    pub no_proxy: String,
    pub timeout_secs: u64,
    pub verify_tls: bool,
    pub crates_io_mirror: String,
    pub rustup_dist_server: String,
    pub rustup_update_root: String,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        NetworkSettings {
            proxy_mode: ProxyMode::None,
            proxy_url: "http://127.0.0.1:7890".to_string(),
            proxy_username: String::new(),
            proxy_password: String::new(),
            no_proxy: "localhost,127.0.0.1".to_string(),
            timeout_secs: 60,
            verify_tls: true,
            crates_io_mirror: "official".to_string(),
            rustup_dist_server: "https://static.rust-lang.org".to_string(),
            rustup_update_root: "https://static.rust-lang.org/rustup".to_string(),
        }
    }
}

impl NetworkSettings {
    pub fn effective_proxy_url(&self) -> Option<String> {
        match self.proxy_mode {
            ProxyMode::None => None,
            ProxyMode::System => std::env::var("HTTPS_PROXY")
                .or_else(|_| std::env::var("HTTP_PROXY"))
                .or_else(|_| std::env::var("ALL_PROXY"))
                .ok()
                .filter(|s| !s.trim().is_empty()),
            ProxyMode::Http | ProxyMode::Socks5 => {
                let u = self.proxy_url.trim();
                if u.is_empty() {
                    None
                } else {
                    Some(u.to_string())
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ApiProtocol {
    OpenAiCompatible,
    AnthropicMessages,
    GoogleGemini,
    OllamaLocal,
}

impl ApiProtocol {
    pub const ALL: &'static [ApiProtocol] = &[
        ApiProtocol::OpenAiCompatible,
        ApiProtocol::AnthropicMessages,
        ApiProtocol::GoogleGemini,
        ApiProtocol::OllamaLocal,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ApiProtocol::OpenAiCompatible => "OpenAI / Compatible (DeepSeek / OpenRouter / vLLM)",
            ApiProtocol::AnthropicMessages => "Anthropic Claude Messages API",
            ApiProtocol::GoogleGemini => "Google Gemini generateContent API",
            ApiProtocol::OllamaLocal => "Ollama Native Local API (/api/chat)",
        }
    }

    pub fn short_tag(&self) -> &'static str {
        match self {
            ApiProtocol::OpenAiCompatible => "OpenAI",
            ApiProtocol::AnthropicMessages => "Claude",
            ApiProtocol::GoogleGemini => "Gemini",
            ApiProtocol::OllamaLocal => "Ollama",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelProfile {
    pub id: String,
    pub name: String,
    pub protocol: ApiProtocol,
    pub base_url: String,
    pub api_key: String,
    pub model_identifier: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub supports_reasoning: bool,
}

pub fn default_model_profiles() -> Vec<ModelProfile> {
    vec![
        ModelProfile {
            id: "claude-sonnet-4".to_string(),
            name: "Claude Sonnet 4 (Anthropic)".to_string(),
            protocol: ApiProtocol::AnthropicMessages,
            base_url: "https://api.anthropic.com/v1/messages".to_string(),
            api_key: String::new(),
            model_identifier: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 8192,
            temperature: 0.2,
            supports_reasoning: true,
        },
        ModelProfile {
            id: "openai-codex-o3".to_string(),
            name: "OpenAI o3 / GPT-4.1 (OpenAI)".to_string(),
            protocol: ApiProtocol::OpenAiCompatible,
            base_url: "https://api.openai.com/v1/chat/completions".to_string(),
            api_key: String::new(),
            model_identifier: "o3-mini".to_string(),
            max_tokens: 8192,
            temperature: 0.2,
            supports_reasoning: true,
        },
        ModelProfile {
            id: "deepseek-r1".to_string(),
            name: "DeepSeek-R1 / V3 (OpenAI-Compatible)".to_string(),
            protocol: ApiProtocol::OpenAiCompatible,
            base_url: "https://api.deepseek.com/v1/chat/completions".to_string(),
            api_key: String::new(),
            model_identifier: "deepseek-reasoner".to_string(),
            max_tokens: 8192,
            temperature: 0.2,
            supports_reasoning: true,
        },
        ModelProfile {
            id: "gemini-2.5-pro".to_string(),
            name: "Gemini 2.5 Pro (Google)".to_string(),
            protocol: ApiProtocol::GoogleGemini,
            base_url: "https://generativelanguage.googleapis.com/v1beta/models".to_string(),
            api_key: String::new(),
            model_identifier: "gemini-2.5-pro".to_string(),
            max_tokens: 8192,
            temperature: 0.2,
            supports_reasoning: true,
        },
        ModelProfile {
            id: "ollama-qwen-coder".to_string(),
            name: "Ollama Local (qwen2.5-coder:14b)".to_string(),
            protocol: ApiProtocol::OllamaLocal,
            base_url: "http://127.0.0.1:11434/api/chat".to_string(),
            api_key: String::new(),
            model_identifier: "qwen2.5-coder:14b".to_string(),
            max_tokens: 4096,
            temperature: 0.1,
            supports_reasoning: true,
        },
    ]
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum SandboxPermissionTier {
    /// 严格确认模式：每一步文件读取外的修改、写入、终端命令、依赖变更均需用户明确点击批准
    StrictStepByStep,
    /// 标准工作区沙箱：工作区内只读分析与诊断自动放行，修改文件/执行构建需用户确认并可预览接受/撤回
    WorkspaceBalanced,
    /// 宽松 YOLO 全自动模式：允许 AI Agent 在工作区沙箱内自主读写代码、修复报错、运行 cargo check/test（但仍受系统级高危路径保护）
    YoloAutonomous,
}

impl Default for SandboxPermissionTier {
    fn default() -> Self {
        SandboxPermissionTier::StrictStepByStep
    }
}

impl SandboxPermissionTier {
    pub fn label(&self) -> &'static str {
        match self {
            SandboxPermissionTier::StrictStepByStep => "🛡 严格模式 (每步确认)",
            SandboxPermissionTier::WorkspaceBalanced => "⚖ 工作区均衡 (读写分离确认)",
            SandboxPermissionTier::YoloAutonomous => "⚡ YOLO 模式 (全自动执行)",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            SandboxPermissionTier::StrictStepByStep => {
                "所有工具调用、代码补丁应用、命令执行均需您逐一点击「批准」后方可执行。"
            }
            SandboxPermissionTier::WorkspaceBalanced => {
                "工作区内代码读取、AST 检索、报错分析自动执行；写入文件与终端命令需确认并支持一键撤回。"
            }
            SandboxPermissionTier::YoloAutonomous => {
                "AI Agent 自动连续执行读取、代码修复、保存与 Cargo 检查（仍拦截工作区越界与高危删除）。"
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub dark_mode: bool,
    pub theme_preset: ThemePreset,
    pub last_workspace: Option<String>,
    pub recent_workspaces: Vec<String>,
    pub show_indent_guides: bool,
    pub show_active_indent_guide: bool,
    pub show_inline_diagnostics: bool,
    pub auto_precheck_on_type: bool,
    pub show_file_badges: bool,
    pub network: NetworkSettings,
    pub models: Vec<ModelProfile>,
    pub active_model_id: String,
    /// Codex 风格思考程度：0=Minimal, 1=Low, 2=Medium, 3=High, 4=Max (xHigh)
    pub thinking_effort_level: u8,
    pub sandbox_tier: SandboxPermissionTier,
    pub mcp_server_enabled: bool,
    pub mcp_server_port: u16,
    pub mcp_builtin_sidebar_connected: bool,
    pub enabled_plugins: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        let models = default_model_profiles();
        let active_id = models.first().map(|m| m.id.clone()).unwrap_or_default();
        Settings {
            dark_mode: true,
            theme_preset: ThemePreset::VsCodeDarkPlus,
            last_workspace: None,
            recent_workspaces: Vec::new(),
            show_indent_guides: true,
            show_active_indent_guide: true,
            show_inline_diagnostics: true,
            auto_precheck_on_type: true,
            show_file_badges: true,
            network: NetworkSettings::default(),
            models,
            active_model_id: active_id,
            thinking_effort_level: 2, // Default: Medium
            sandbox_tier: SandboxPermissionTier::StrictStepByStep,
            mcp_server_enabled: true,
            mcp_server_port: DEFAULT_MCP_PORT,
            mcp_builtin_sidebar_connected: true, // 默认开启连接内置 AI 侧栏
            enabled_plugins: vec![
                "rust-clippy-lens".to_string(),
                "cargo-dependency-guard".to_string(),
                "todo-tree-highlighter".to_string(),
            ],
        }
    }
}

pub fn settings_path() -> Option<PathBuf> {
    std::env::var("APPDATA")
        .or_else(|_| std::env::var("HOME").map(|h| format!("{}/.config", h)))
        .ok()
        .map(|base| PathBuf::from(base).join("NovaRS").join("settings.json"))
}

pub fn load_settings() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(json) => {
            let mut s = serde_json::from_str::<Settings>(&json).unwrap_or_default();
            if s.models.is_empty() {
                s.models = default_model_profiles();
            }
            s
        }
        Err(_) => Settings::default(),
    }
}

pub fn save_settings_to_disk(settings: &Settings) {
    let Some(path) = settings_path() else { return };
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, json);
    }
}
