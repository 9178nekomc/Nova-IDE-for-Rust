// NovaRS — Application Constants & Global Configuration Defaults

use std::time::Duration;
use eframe::egui::{Color32, Vec2};

pub const APP_NAME: &str = "NovaRS";
pub const APP_FULL_NAME: &str = "Nova IDE for Rust";
pub const APP_VERSION: &str = "v2.0.0-Super";
pub const APP_NAME_WITH_VERSION: &str = "NovaRS v2.0.0-Super";

pub const SPLASH_SIZE: Vec2 = Vec2 { x: 520.0, y: 340.0 };
pub const MAIN_SIZE: Vec2 = Vec2 { x: 1400.0, y: 860.0 };
pub const SPLASH_DURATION: Duration = Duration::from_millis(2200);
pub const SPLASH_FORCE_DEADLINE: Duration = Duration::from_secs(4);
pub const SPLASH_MIN_VISIBLE: Duration = Duration::from_millis(1000);

pub const BRAND_RED: Color32 = Color32::from_rgb(0xe9, 0x45, 0x60);
pub const BRAND_ACCENT: Color32 = Color32::from_rgb(0x61, 0xaf, 0xef);
pub const BRAND_GREEN: Color32 = Color32::from_rgb(0x98, 0xc3, 0x79);
pub const BRAND_WARN: Color32 = Color32::from_rgb(0xe5, 0xc0, 0x7b);
pub const BRAND_PURPLE: Color32 = Color32::from_rgb(0xc6, 0x78, 0xdd);

pub const SPLASH_C1: Color32 = Color32::from_rgb(0x14, 0x16, 0x26);
pub const SPLASH_C2: Color32 = Color32::from_rgb(0x16, 0x21, 0x3e);
pub const SPLASH_C3: Color32 = Color32::from_rgb(0x0f, 0x34, 0x60);

pub const MONO_SIZE: f32 = 13.5;
pub const SPLASH_FLOW_LINES: usize = 5;
pub const SPLASH_LOAD_TEXTS: [&str; 4] = [
    "正在初始化核心引擎 & 虚拟化渲染管线...",
    "正在启动 AI Copilot & MCP 协议服务...",
    "正在检测 Rust 工具链与工作区索引...",
    "准备就绪，进入 NovaRS v2.0...",
];

pub const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
    "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
    "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true",
    "type", "unsafe", "use", "where", "while", "union", "macro_rules",
];

/// 子进程不创建控制台窗口（Windows GUI 应用下避免弹黑框）
#[cfg(target_os = "windows")]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 编译期嵌入的 rust-analyzer 可执行文件（build.rs 定位；未找到时为空，运行期降级）
pub const RA_EMBEDDED: &[u8] = include_bytes!(env!("NOVARS_RA_PATH"));

/// 编译期嵌入的中文界面字体（等线），保证单文件 exe 在任何机器上中文渲染一致
pub const FONT_CJK_EMBEDDED: &[u8] = include_bytes!("../assets/Deng.ttf");

pub const RA_RELEASE_NAME: &str = "novars-rust-analyzer.exe";
pub const MINIMAP_W: f32 = 96.0;
pub const GUTTER_BP_W: f32 = 20.0;
pub const POPUP_ROW_H: f32 = 22.0;

/// 编辑器大文件虚拟化窗口参数：
/// 当文件超过 VIRTUAL_WINDOW_THRESHOLD 行时，自动启用滑动视口窗口或无截断分块渲染，
/// 彻底解决 4000~5000 行以上时 GPU 纹理/网格顶点上限导致的行号与文本截断问题。
pub const LARGE_FILE_LINE_THRESHOLD: usize = 2500;
pub const VIRTUAL_CHUNK_LINES: usize = 1200;

/// 默认内置 MCP 服务端口
pub const DEFAULT_MCP_PORT: u16 = 18765;
