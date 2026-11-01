// NovaRS — AI Copilot & Autonomous Agent Sidebar UI
// Integrates:
// 1. Model Selector + Model Manager Dialog (Multi-protocol: OpenAI, Claude, Gemini, Ollama)
// 2. Codex-Style Thinking Effort Slider (`MIN` / `LOW` / `MED` / `HIGH` / `xHIGH`)
// 3. Multi-Conversation Session Manager (Create, Switch, Rename, Clear, Export)
// 4. Workspace & Context Pills (`@ActiveFile`, `@Diagnostics`, `@AST`, MCP Bridge Status Badge)
// 5. Layered Sandbox & Permission Tier Selector (`Strict Step-by-Step`, `Balanced`, `YOLO`)
// 6. Interactive Pending Action Approval Cards + One-Click Code Patch `Accept` / `Revert`

pub mod client;
pub mod codex_slider;
pub mod models;
pub mod prompts;
pub mod sandbox;
pub mod sessions;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use eframe::egui::{self, Align, Color32, FontId, Key, RichText, Stroke, Ui};

use crate::ai::client::send_ai_request;
use crate::ai::codex_slider::draw_codex_thinking_slider;
use crate::ai::models::ModelManagerDialog;
use crate::ai::prompts::build_claude_code_system_prompt;
use crate::ai::sandbox::{classify_tool_risk, requires_user_confirmation, PendingAgentAction};
use crate::ai::sessions::{
    extract_mcp_tool_call, extract_patches_from_response, ChatMessage, ChatRole, ChatSession,
    ExtractedCodePatch,
};
use crate::config::{ModelProfile, NetworkSettings, SandboxPermissionTier};
use crate::constants::{BRAND_ACCENT, BRAND_GREEN, BRAND_PURPLE, BRAND_RED, BRAND_WARN};
use crate::editor::diagnostics::CodeDiagnostic;
use crate::utils::lock;

pub struct AiSidebarAction {
    pub apply_code_patch: Option<ExtractedCodePatch>,
    pub accept_active_patch: bool,
    pub revert_active_patch: bool,
    pub execute_mcp_tool: Option<(String, serde_json::Value)>,
    pub settings_changed: bool,
}

pub struct AiSidebarState {
    pub visible: bool,
    pub sessions: Vec<ChatSession>,
    pub active_session_idx: usize,
    pub next_session_id: u64,
    pub input_text: String,
    pub is_generating: Arc<AtomicBool>,
    pub incoming_reply: Arc<Mutex<Option<Result<ChatMessage, String>>>>,
    pub model_dialog: ModelManagerDialog,
    pub pending_approval: Option<PendingAgentAction>,
    pub next_action_id: u64,
    pub show_system_prompt_preview: bool,
}

impl AiSidebarState {
    pub fn new(workspace: Option<PathBuf>) -> Self {
        let initial_session = ChatSession::new(1, "对话 #1 (Rust Copilot)", workspace);
        AiSidebarState {
            visible: true, // 默认开启 AI 副驾驶侧边栏
            sessions: vec![initial_session],
            active_session_idx: 0,
            next_session_id: 2,
            input_text: String::new(),
            is_generating: Arc::new(AtomicBool::new(false)),
            incoming_reply: Arc::new(Mutex::new(None)),
            model_dialog: ModelManagerDialog::default(),
            pending_approval: None,
            next_action_id: 1,
            show_system_prompt_preview: false,
        }
    }

    pub fn active_session_mut(&mut self) -> &mut ChatSession {
        if self.sessions.is_empty() {
            let id = self.next_session_id;
            self.next_session_id += 1;
            self.sessions.push(ChatSession::new(id, format!("对话 #{}", id), None));
            self.active_session_idx = 0;
        }
        let idx = self.active_session_idx.min(self.sessions.len() - 1);
        self.active_session_idx = idx;
        &mut self.sessions[idx]
    }

    /// 当用户在编辑器里选中一个报错并点击「🤖 让 AI 帮我修复」时触发
    #[allow(clippy::too_many_arguments)]
    pub fn trigger_fix_diagnostic(
        &mut self,
        ctx: &egui::Context,
        diag: &CodeDiagnostic,
        file_name: &str,
        file_text: &str,
        models: &[ModelProfile],
        active_model_id: &str,
        network: &NetworkSettings,
        effort_level: u8,
        sandbox_tier: SandboxPermissionTier,
        mcp_connected: bool,
        workspace: Option<&Path>,
    ) {
        self.visible = true;
        let lines: Vec<&str> = file_text.lines().collect();
        let start_ctx = diag.line.saturating_sub(4).max(1);
        let end_ctx = (diag.line + 4).min(lines.len().max(1));
        let mut snippet_block = String::new();
        for ln in start_ctx..=end_ctx {
            if let Some(l_str) = lines.get(ln - 1) {
                let marker = if ln == diag.line { ">> " } else { "   " };
                snippet_block.push_str(&format!("{}L{}: {}\n", marker, ln, l_str));
            }
        }

        let prompt = format!(
            "Fix this diagnostic in `{}` at L{}:\n\
             - Code: `{}` ({})\n\
             - Message: {}\n\
             - Context:\n```rust\n{}```\n\
             Provide a brief explanation and a `novars-patch` block for L{}.",
            file_name,
            diag.line,
            diag.code,
            diag.severity.label(),
            diag.message,
            snippet_block,
            diag.line
        );

        self.input_text = prompt;
        self.submit_user_message(
            ctx,
            file_name,
            file_text,
            diag.line,
            &[diag.clone()],
            models,
            active_model_id,
            network,
            effort_level,
            sandbox_tier,
            mcp_connected,
            workspace,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit_user_message(
        &mut self,
        ctx: &egui::Context,
        active_file_name: &str,
        active_file_text: &str,
        cursor_line: usize,
        diagnostics: &[CodeDiagnostic],
        models: &[ModelProfile],
        active_model_id: &str,
        network: &NetworkSettings,
        effort_level: u8,
        sandbox_tier: SandboxPermissionTier,
        mcp_connected: bool,
        workspace: Option<&Path>,
    ) {
        let user_prompt = self.input_text.trim().to_string();
        if user_prompt.is_empty() || self.is_generating.load(Ordering::Relaxed) {
            return;
        }
        self.input_text.clear();

        let (attach_file, attach_diags) = {
            let sess = self.active_session_mut();
            sess.messages.push(ChatMessage {
                role: ChatRole::User,
                content: user_prompt.clone(),
                reasoning_trace: None,
                timestamp: "刚刚".to_string(),
                patches: Vec::new(),
                mcp_tool_call: None,
            });
            (sess.attach_active_file, sess.attach_diagnostics)
        };

        let profile = models
            .iter()
            .find(|m| m.id == active_model_id)
            .or_else(|| models.first())
            .cloned()
            .unwrap_or_else(|| crate::config::default_model_profiles()[0].clone());

        let ws_str = workspace.map(|p| p.display().to_string());
        let sys_prompt = build_claude_code_system_prompt(
            effort_level,
            sandbox_tier,
            mcp_connected,
            ws_str.as_deref(),
            Some(active_file_name),
            cursor_line,
        );

        // 组装精简上下文（省 Token 设计：仅附带光标/报错附近的关键窗口而非整份几千行源码）
        let mut enriched_history: Vec<(String, String)> = Vec::new();
        let sess = self.active_session_mut();
        let msg_len = sess.messages.len();
        let history_start = msg_len.saturating_sub(8);
        for (idx, m) in sess.messages[history_start..].iter().enumerate() {
            let role_str = match m.role {
                ChatRole::Assistant => "assistant",
                _ => "user",
            };
            let mut content = m.content.clone();
            // 在最后一条用户消息上附加轻量级局部代码上下文
            if idx + history_start + 1 == msg_len {
                if attach_file && !active_file_text.is_empty() {
                    let lines: Vec<&str> = active_file_text.lines().collect();
                    let from = cursor_line.saturating_sub(20).max(1);
                    let to = (cursor_line + 20).min(lines.len().max(1));
                    let mut excerpt = String::new();
                    for ln in from..=to {
                        if let Some(s) = lines.get(ln - 1) {
                            excerpt.push_str(&format!("L{}: {}\n", ln, s));
                        }
                    }
                    content.push_str(&format!(
                        "\n\n[Active File `{}` Excerpt (L{}-L{})]:\n```rust\n{}```",
                        active_file_name, from, to, excerpt
                    ));
                }
                if attach_diags && !diagnostics.is_empty() {
                    let mut diag_summary = String::new();
                    for d in diagnostics.iter().take(5) {
                        diag_summary.push_str(&format!(
                            "- L{} [{}]: {}\n",
                            d.line, d.code, d.message
                        ));
                    }
                    content.push_str(&format!("\n[Active Diagnostics]:\n{}", diag_summary));
                }
            }
            enriched_history.push((role_str.to_string(), content));
        }

        let is_gen = self.is_generating.clone();
        let slot = self.incoming_reply.clone();
        let net_clone = network.clone();
        let file_name_owned = active_file_name.to_string();
        let file_text_owned = active_file_text.to_string();
        let ctx_clone = ctx.clone();

        is_gen.store(true, Ordering::Relaxed);
        std::thread::spawn(move || {
            let res = send_ai_request(
                &profile,
                &net_clone,
                effort_level,
                &sys_prompt,
                &enriched_history,
                &file_name_owned,
                &file_text_owned,
                cursor_line,
            );
            let mapped = res.map(|payload| {
                let patches = extract_patches_from_response(
                    &payload.content,
                    &file_name_owned,
                    cursor_line,
                );
                let mcp_call = extract_mcp_tool_call(&payload.content);
                ChatMessage {
                    role: ChatRole::Assistant,
                    content: payload.content,
                    reasoning_trace: payload.reasoning,
                    timestamp: payload.model_used,
                    patches,
                    mcp_tool_call: mcp_call,
                }
            });
            *lock(&slot) = Some(mapped);
            is_gen.store(false, Ordering::Relaxed);
            ctx_clone.request_repaint();
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        ui: &mut Ui,
        models: &mut Vec<ModelProfile>,
        active_model_id: &mut String,
        effort_level: &mut u8,
        sandbox_tier: &mut SandboxPermissionTier,
        mcp_connected: &mut bool,
        mcp_port: u16,
        network: &NetworkSettings,
        workspace: Option<&Path>,
        active_file_name: &str,
        active_file_text: &str,
        cursor_line: usize,
        diagnostics: &[CodeDiagnostic],
        has_pending_editor_patch: bool,
    ) -> AiSidebarAction {
        let mut action = AiSidebarAction {
            apply_code_patch: None,
            accept_active_patch: false,
            revert_active_patch: false,
            execute_mcp_tool: None,
            settings_changed: false,
        };

        // 轮询后台线程完成的 AI 回复。
        // 注意：必须先把结果从 Mutex 中取出到独立绑定，让 MutexGuard 的临时生命周期在此行结束，
        // 否则下面 `self.active_session_mut()` 的可变借用会与守卫持有的不可变借用冲突 (E0502)。
        let incoming_outcome = lock(&self.incoming_reply).take();
        if let Some(outcome) = incoming_outcome {
            match outcome {
                Ok(msg) => {
                    // 检查是否包含 MCP 工具调用以及沙箱权限分层处理
                    if let Some((ref tool_name, ref args)) = msg.mcp_tool_call {
                        if *mcp_connected {
                            if requires_user_confirmation(*sandbox_tier, tool_name) {
                                let aid = self.next_action_id;
                                self.next_action_id += 1;
                                self.pending_approval = Some(PendingAgentAction {
                                    id: aid,
                                    tool_name: tool_name.clone(),
                                    args: args.clone(),
                                    summary: format!("调用 MCP 工具 `{}` 参数: {}", tool_name, args),
                                    risk_level: classify_tool_risk(tool_name),
                                });
                            } else {
                                action.execute_mcp_tool = Some((tool_name.clone(), args.clone()));
                            }
                        }
                    }
                    // 在 YOLO 模式下，如果生成了唯一的代码修复补丁，自动进入编辑器补丁预览
                    if *sandbox_tier == SandboxPermissionTier::YoloAutonomous {
                        if let Some(first_patch) = msg.patches.first() {
                            action.apply_code_patch = Some(first_patch.clone());
                        }
                    }
                    self.active_session_mut().messages.push(msg);
                }
                Err(err_msg) => {
                    self.active_session_mut().messages.push(ChatMessage {
                        role: ChatRole::Assistant,
                        content: format!("❌ 请求错误: {}", err_msg),
                        reasoning_trace: None,
                        timestamp: "Error".to_string(),
                        patches: Vec::new(),
                        mcp_tool_call: None,
                    });
                }
            }
        }

        // 1. 顶部标题栏：AI Agent 副驾驶 + MCP 连接状态 + 模型管理入口
        ui.horizontal(|ui| {
            ui.label(RichText::new("🤖 NovaRS AI 副驾驶 (Agent)").strong().size(13.5));
            let mcp_badge = if *mcp_connected {
                RichText::new(format!("🔌 MCP :{}", mcp_port))
                    .size(10.5)
                    .color(BRAND_GREEN)
            } else {
                RichText::new("🔌 MCP 断开").size(10.5).weak()
            };
            if ui
                .button(mcp_badge)
                .on_hover_text("内置 AI 侧栏默认直连内置 MCP 服务，点击切换连接状态")
                .clicked()
            {
                *mcp_connected = !*mcp_connected;
                action.settings_changed = true;
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("⚙ 模型管理").clicked() {
                    self.model_dialog.open = true;
                }
                if ui
                    .small_button("📜 Prompt")
                    .on_hover_text("查看 Claude Code 级英文省 Token 系统提示词")
                    .clicked()
                {
                    self.show_system_prompt_preview = !self.show_system_prompt_preview;
                }
            });
        });
        ui.separator();

        // 2. 模型选择器 & 沙箱权限分层选择器
        ui.horizontal(|ui| {
            ui.label(RichText::new("模型:").size(11.5));
            let current_model_name = models
                .iter()
                .find(|m| m.id == *active_model_id)
                .map(|m| format!("[{}] {}", m.protocol.short_tag(), m.name))
                .unwrap_or_else(|| "选择模型".to_string());
            egui::ComboBox::from_id_salt("sidebar_model_selector")
                .selected_text(RichText::new(current_model_name).size(11.5))
                .width(210.0)
                .show_ui(ui, |ui| {
                    for m in models.iter() {
                        let label = format!("[{}] {}", m.protocol.short_tag(), m.name);
                        if ui
                            .selectable_value(active_model_id, m.id.clone(), label)
                            .clicked()
                        {
                            action.settings_changed = true;
                        }
                    }
                });
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("权限沙箱:").size(11.5));
            egui::ComboBox::from_id_salt("sandbox_tier_selector")
                .selected_text(RichText::new(sandbox_tier.label()).size(11.5).color(
                    match sandbox_tier {
                        SandboxPermissionTier::StrictStepByStep => BRAND_ACCENT,
                        SandboxPermissionTier::WorkspaceBalanced => BRAND_GREEN,
                        SandboxPermissionTier::YoloAutonomous => BRAND_WARN,
                    },
                ))
                .width(195.0)
                .show_ui(ui, |ui| {
                    for tier in [
                        SandboxPermissionTier::StrictStepByStep,
                        SandboxPermissionTier::WorkspaceBalanced,
                        SandboxPermissionTier::YoloAutonomous,
                    ] {
                        if ui
                            .selectable_value(sandbox_tier, tier, tier.label())
                            .on_hover_text(tier.description())
                            .clicked()
                        {
                            action.settings_changed = true;
                        }
                    }
                });
        });

        // 3. Codex 风格思考程度调节滑块
        if draw_codex_thinking_slider(ui, effort_level) {
            action.settings_changed = true;
        }

        // 可选展示 Claude Code 英文 System Prompt 预览
        if self.show_system_prompt_preview {
            let ws_str = workspace.map(|p| p.display().to_string());
            let sys_preview = build_claude_code_system_prompt(
                *effort_level,
                *sandbox_tier,
                *mcp_connected,
                ws_str.as_deref(),
                Some(active_file_name),
                cursor_line,
            );
            egui::Frame::none()
                .fill(Color32::from_rgba_unmultiplied(20, 24, 34, 220))
                .stroke(Stroke::new(1.0_f32, BRAND_PURPLE))
                .rounding(egui::Rounding::same(4.0))
                .inner_margin(egui::Margin::same(6.0))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Claude Code-Grade System Prompt (English, Token-Optimized):")
                            .size(10.5)
                            .color(BRAND_PURPLE)
                            .strong(),
                    );
                    egui::ScrollArea::vertical()
                        .id_salt("sys_prompt_scroll")
                        .max_height(110.0)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(sys_preview)
                                    .font(FontId::monospace(10.0))
                                    .color(Color32::LIGHT_GRAY),
                            );
                        });
                });
        }

        ui.separator();

        // 4. 多对话会话管理栏 (Multi-Session Tabs)
        ui.horizontal(|ui| {
            egui::ScrollArea::horizontal()
                .id_salt("ai_sessions_bar")
                .max_width(ui.available_width() - 68.0)
                .show(ui, |ui| {
                    let mut remove_idx: Option<usize> = None;
                    for (idx, s) in self.sessions.iter().enumerate() {
                        let is_active = idx == self.active_session_idx;
                        if ui
                            .selectable_label(is_active, RichText::new(&s.title).size(11.5))
                            .clicked()
                        {
                            self.active_session_idx = idx;
                        }
                        if self.sessions.len() > 1 && ui.small_button("×").clicked() {
                            remove_idx = Some(idx);
                        }
                    }
                    if let Some(ri) = remove_idx {
                        self.sessions.remove(ri);
                        if self.active_session_idx >= self.sessions.len() {
                            self.active_session_idx = self.sessions.len().saturating_sub(1);
                        }
                    }
                });

            if ui.small_button("＋ 新对话").clicked() {
                let id = self.next_session_id;
                self.next_session_id += 1;
                self.sessions.push(ChatSession::new(
                    id,
                    format!("对话 #{}", id),
                    workspace.map(|p| p.to_path_buf()),
                ));
                self.active_session_idx = self.sessions.len() - 1;
            }
            if ui
                .small_button("🗑")
                .on_hover_text("清空当前对话消息")
                .clicked()
            {
                self.active_session_mut().messages.clear();
            }
        });

        // 工作区与上下文附着开关
        {
            let ws_label = workspace
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "无工作区".to_string());
            let sess = self.active_session_mut();
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("📁 {}", ws_label)).size(11.0).weak());
                ui.checkbox(&mut sess.attach_active_file, "附带当前文件");
                ui.checkbox(
                    &mut sess.attach_diagnostics,
                    format!("附带报错 ({})", diagnostics.len()),
                );
            });
        }
        ui.separator();

        // 5. 待确认的沙箱动作审批卡片（严格模式 / 读写分离模式）
        if let Some(pending) = self.pending_approval.clone() {
            egui::Frame::none()
                .fill(Color32::from_rgba_unmultiplied(0x33, 0x26, 0x1a, 235))
                .stroke(Stroke::new(1.0_f32, BRAND_WARN))
                .rounding(egui::Rounding::same(5.0))
                .inner_margin(egui::Margin::same(8.0))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!("🛡 沙箱权限拦截 — 待您确认 ({})", pending.risk_level))
                            .strong()
                            .color(BRAND_WARN),
                    );
                    ui.label(RichText::new(&pending.summary).monospace().size(11.5));
                    ui.horizontal(|ui| {
                        if ui
                            .button(RichText::new("✅ 批准执行 (Approve)").color(BRAND_GREEN).strong())
                            .clicked()
                        {
                            action.execute_mcp_tool =
                                Some((pending.tool_name.clone(), pending.args.clone()));
                            self.pending_approval = None;
                        }
                        if ui
                            .button(RichText::new("✕ 拒绝 (Deny)").color(BRAND_RED))
                            .clicked()
                        {
                            self.pending_approval = None;
                        }
                    });
                });
            ui.add_space(4.0);
        }

        // 6. 聊天消息流区域（含思考链折叠面板 + 代码补丁一键预览/接受/撤回）
        let avail_h = ui.available_height().max(120.0);
        let chat_h = (avail_h - 115.0).max(80.0);

        let messages_snapshot = self.active_session_mut().messages.clone();
        egui::ScrollArea::vertical()
            .id_salt("ai_chat_messages_scroll")
            .max_height(chat_h)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if messages_snapshot.is_empty() {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("👋 欢迎使用 NovaRS AI 副驾驶 (Claude Code 架构)")
                            .strong()
                            .color(BRAND_ACCENT),
                    );
                    ui.label(
                        RichText::new(
                            "• 支持提前实时检测代码语法与类型报错，点击报错即可让 AI 生成修复补丁并支持「接受」或「撤回」\n\
                             • 支持 OpenAI / Claude / Gemini / DeepSeek / Ollama 多协议接入\n\
                             • 支持 Codex 风格 5 档思考程度滑块调节\n\
                             • 内置 MCP 协议服务，默认与本侧栏实时联通",
                        )
                        .size(11.5)
                        .weak(),
                    );
                    ui.add_space(8.0);
                    ui.label(RichText::new("快捷指令:").size(11.5).strong());
                    let mut quick_prompt: Option<String> = None;
                    ui.horizontal_wrapped(|ui| {
                        for (btn_label, q_text) in [
                            ("🔍 检查并修复当前文件报错", "请检查当前文件中的所有编译/语法报错与警告，并给出可直接应用的 novars-patch 修复补丁。"),
                            ("✨ 重构优化与消除 unwrap", "请将当前光标附近的代码重构为地道的 Rust 错误处理风格，消除潜在 panic。"),
                            ("🧪 生成单元测试", "请为当前文件中的核心函数编写完整的 #[cfg(test)] 单元测试模块。"),
                        ] {
                            if ui.small_button(btn_label).clicked() {
                                quick_prompt = Some(q_text.to_string());
                            }
                        }
                    });
                    if let Some(qp) = quick_prompt {
                        self.input_text = qp;
                    }
                }

                for (m_idx, msg) in messages_snapshot.iter().enumerate() {
                    let is_user = msg.role == ChatRole::User;
                    let bg = if is_user {
                        Color32::from_rgba_unmultiplied(0x26, 0x4f, 0x78, 95)
                    } else {
                        Color32::from_rgba_unmultiplied(0x20, 0x24, 0x2e, 195)
                    };
                    let border = if is_user {
                        BRAND_ACCENT
                    } else {
                        Color32::from_rgba_unmultiplied(120, 130, 150, 70)
                    };

                    egui::Frame::none()
                        .fill(bg)
                        .stroke(Stroke::new(0.8_f32, border))
                        .rounding(egui::Rounding::same(6.0))
                        .inner_margin(egui::Margin::same(8.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let role_label = match msg.role {
                                    ChatRole::User => "🧑 You",
                                    ChatRole::Assistant => "🤖 NovaRS Agent",
                                    ChatRole::Tool => "🔌 MCP Tool Result",
                                    ChatRole::System => "⚙ System",
                                };
                                ui.label(
                                    RichText::new(role_label)
                                        .size(11.5)
                                        .strong()
                                        .color(if is_user { BRAND_ACCENT } else { BRAND_GREEN }),
                                );
                                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(RichText::new(&msg.timestamp).size(10.0).weak());
                                });
                            });

                            if let Some(ref reasoning) = msg.reasoning_trace {
                                egui::CollapsingHeader::new(
                                    RichText::new("💭 查看模型思考过程 (Reasoning Trace)")
                                        .size(11.0)
                                        .color(BRAND_PURPLE),
                                )
                                .id_salt(("reasoning_trace", m_idx))
                                .default_open(false)
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(reasoning)
                                            .font(FontId::monospace(11.0))
                                            .weak(),
                                    );
                                });
                            }

                            ui.add(
                                egui::Label::new(
                                    RichText::new(&msg.content).size(12.2),
                                )
                                .selectable(true),
                            );

                            // 如果消息中包含代码修复补丁，提供一键「应用预览」「接受修改」「撤回修改」按钮
                            for (p_idx, patch) in msg.patches.iter().enumerate() {
                                ui.add_space(4.0);
                                egui::Frame::none()
                                    .fill(Color32::from_rgba_unmultiplied(0x16, 0x2c, 0x22, 220))
                                    .stroke(Stroke::new(1.0_f32, BRAND_GREEN))
                                    .rounding(egui::Rounding::same(4.0))
                                    .inner_margin(egui::Margin::same(6.0))
                                    .show(ui, |ui| {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label(
                                                RichText::new(format!(
                                                    "🔧 补丁 #{} ({} · L{}-L{})",
                                                    p_idx + 1,
                                                    patch.file_hint,
                                                    patch.start_line,
                                                    patch.end_line
                                                ))
                                                .size(11.5)
                                                .strong()
                                                .color(BRAND_GREEN),
                                            );
                                            if ui
                                                .small_button(
                                                    RichText::new("✨ 应用到编辑器预览")
                                                        .color(BRAND_ACCENT)
                                                        .strong(),
                                                )
                                                .clicked()
                                            {
                                                action.apply_code_patch = Some(patch.clone());
                                            }
                                            if has_pending_editor_patch {
                                                if ui
                                                    .small_button(
                                                        RichText::new("✅ 接受").color(BRAND_GREEN),
                                                    )
                                                    .clicked()
                                                {
                                                    action.accept_active_patch = true;
                                                }
                                                if ui
                                                    .small_button(
                                                        RichText::new("↩ 撤回").color(BRAND_RED),
                                                    )
                                                    .clicked()
                                                {
                                                    action.revert_active_patch = true;
                                                }
                                            }
                                        });
                                    });
                            }
                        });
                    ui.add_space(6.0);
                }

                if self.is_generating.load(Ordering::Relaxed) {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new("AI Agent 正在深入思考并生成代码方案...")
                                .size(12.0)
                                .color(BRAND_ACCENT),
                        );
                    });
                }
            });

        ui.separator();

        // 7. 底部多行输入框与发送栏
        ui.vertical(|ui| {
            let resp = ui.add(
                egui::TextEdit::multiline(&mut self.input_text)
                    .desired_rows(3)
                    .desired_width(ui.available_width())
                    .hint_text("输入问题、选中报错修复或下达重构指令 (Ctrl+Enter 发送)..."),
            );
            let ctrl_enter = resp.has_focus()
                && ctx.input(|i| i.modifiers.command && i.key_pressed(Key::Enter));

            ui.horizontal(|ui| {
                if !diagnostics.is_empty()
                    && ui
                        .small_button(
                            RichText::new(format!("⚡ 一键修复首个报错 (L{})", diagnostics[0].line))
                                .color(BRAND_WARN),
                        )
                        .clicked()
                {
                    let d = diagnostics[0].clone();
                    self.trigger_fix_diagnostic(
                        ctx,
                        &d,
                        active_file_name,
                        active_file_text,
                        models,
                        active_model_id,
                        network,
                        *effort_level,
                        *sandbox_tier,
                        *mcp_connected,
                        workspace,
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    let busy = self.is_generating.load(Ordering::Relaxed);
                    if (ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(
                                RichText::new("🚀 发送 (Ctrl+Enter)").color(BRAND_ACCENT).strong(),
                            ),
                        )
                        .clicked()
                        || ctrl_enter)
                        && !busy
                    {
                        self.submit_user_message(
                            ctx,
                            active_file_name,
                            active_file_text,
                            cursor_line,
                            diagnostics,
                            models,
                            active_model_id,
                            network,
                            *effort_level,
                            *sandbox_tier,
                            *mcp_connected,
                            workspace,
                        );
                    }
                });
            });
        });

        // 模型管理弹窗
        if self.model_dialog.draw(ctx, models, active_model_id) {
            action.settings_changed = true;
        }

        action
    }
}
