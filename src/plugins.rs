// NovaRS — Extensible Plugin & Extension Ecosystem Architecture
// Reserves standardized extension interfaces (`NovaPluginDescriptor`, Hook Points, Custom Commands,
// Linter/Inspection Hooks, and JSON Manifest Loader from `plugins/*.json`) to grow the NovaRS ecosystem.

use std::path::Path;
use eframe::egui::{self, RichText};
use serde::{Deserialize, Serialize};
use crate::constants::{BRAND_ACCENT, BRAND_GREEN, BRAND_PURPLE, BRAND_WARN};

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum PluginHookKind {
    EditorInspection,
    ToolbarAction,
    CargoSubcommand,
    AiPromptSkill,
}

impl PluginHookKind {
    pub fn label(&self) -> &'static str {
        match self {
            PluginHookKind::EditorInspection => "🔍 实时代码检查扩展 (Editor Linter)",
            PluginHookKind::ToolbarAction => "🛠 顶部工具栏动作 (Toolbar Command)",
            PluginHookKind::CargoSubcommand => "📦 Cargo 构建扩展 (Cargo Runner)",
            PluginHookKind::AiPromptSkill => "🤖 AI Agent 技能包 (AI Skill)",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NovaPluginDescriptor {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub hook_kind: PluginHookKind,
    pub command_or_rule: String,
    pub builtin: bool,
}

pub fn builtin_ecosystem_plugins() -> Vec<NovaPluginDescriptor> {
    vec![
        NovaPluginDescriptor {
            id: "rust-clippy-lens".to_string(),
            name: "Rust Clippy & Error Lens Pro".to_string(),
            version: "1.4.0".to_string(),
            author: "NovaRS Official".to_string(),
            description: "在代码行尾实时渲染编译错误、类型不匹配与 `.unwrap()` 风险内联提示".to_string(),
            hook_kind: PluginHookKind::EditorInspection,
            command_or_rule: "cargo clippy --message-format=json".to_string(),
            builtin: true,
        },
        NovaPluginDescriptor {
            id: "cargo-dependency-guard".to_string(),
            name: "Cargo Dependency & Security Guard".to_string(),
            version: "1.2.0".to_string(),
            author: "NovaRS Official".to_string(),
            description: "检测 Cargo.toml 依赖版本冲突并提供一键升级与特性管理".to_string(),
            hook_kind: PluginHookKind::CargoSubcommand,
            command_or_rule: "cargo tree -d".to_string(),
            builtin: true,
        },
        NovaPluginDescriptor {
            id: "todo-tree-highlighter".to_string(),
            name: "TODO & FIXME Inspector".to_string(),
            version: "1.1.0".to_string(),
            author: "NovaRS Community".to_string(),
            description: "扫描代码中的 `todo!()`、`unimplemented!()` 与注释标记，支持 AI 一键补全实现".to_string(),
            hook_kind: PluginHookKind::EditorInspection,
            command_or_rule: "inspect:todo".to_string(),
            builtin: true,
        },
        NovaPluginDescriptor {
            id: "unsafe-soundness-auditor".to_string(),
            name: "Unsafe Rust Soundness Auditor".to_string(),
            version: "1.0.2".to_string(),
            author: "NovaRS Security Lab".to_string(),
            description: "向 AI 副驾驶注入内存安全与并发无锁审计技能，严格审查 unsafe 块".to_string(),
            hook_kind: PluginHookKind::AiPromptSkill,
            command_or_rule: "Audit all unsafe blocks and raw pointers for UB and aliasing violations.".to_string(),
            builtin: true,
        },
        NovaPluginDescriptor {
            id: "cargo-expand-macro".to_string(),
            name: "Macro Expand & ASM Viewer".to_string(),
            version: "0.9.5".to_string(),
            author: "NovaRS Community".to_string(),
            description: "在终端一键展开过程宏与派生宏 (`cargo expand` / `cargo test -- --nocapture`)".to_string(),
            hook_kind: PluginHookKind::ToolbarAction,
            command_or_rule: "cargo test -- --nocapture".to_string(),
            builtin: true,
        },
    ]
}

pub struct PluginManagerState {
    pub open: bool,
    pub plugins: Vec<NovaPluginDescriptor>,
    pub new_plugin_name: String,
    pub new_plugin_cmd: String,
    pub new_plugin_desc: String,
}

pub enum PluginAction {
    None,
    RunCommand(String),
    SendAiSkillPrompt(String),
    ToggledPlugin,
}

impl Default for PluginManagerState {
    fn default() -> Self {
        PluginManagerState {
            open: false,
            plugins: builtin_ecosystem_plugins(),
            new_plugin_name: String::new(),
            new_plugin_cmd: String::new(),
            new_plugin_desc: String::new(),
        }
    }
}

impl PluginManagerState {
    /// 扫描工作区或应用目录下的 `plugins/*.json` 外部拓展包
    pub fn load_external_plugins(&mut self, workspace: Option<&Path>) {
        let mut list = builtin_ecosystem_plugins();
        let mut dirs = vec![Path::new("plugins").to_path_buf()];
        if let Some(ws) = workspace {
            dirs.push(ws.join("plugins"));
            dirs.push(ws.join(".novars").join("plugins"));
        }

        for dir in dirs {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map(|e| e == "json").unwrap_or(false) {
                        if let Ok(raw) = std::fs::read_to_string(&path) {
                            if let Ok(mut desc) = serde_json::from_str::<NovaPluginDescriptor>(&raw) {
                                desc.builtin = false;
                                if !list.iter().any(|p| p.id == desc.id) {
                                    list.push(desc);
                                }
                            }
                        }
                    }
                }
            }
        }
        self.plugins = list;
    }

    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        enabled_plugins: &mut Vec<String>,
        workspace: Option<&Path>,
    ) -> PluginAction {
        let mut action = PluginAction::None;
        if !self.open {
            return action;
        }
        let mut win_open = self.open;

        egui::Window::new("🧩 插件生态与拓展接口中心 (Extensions & Plugin API)")
            .open(&mut win_open)
            .resizable(true)
            .default_width(680.0)
            .default_height(500.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("支持标准 JSON 插件清单 (`plugins/*.json`)、Linter 钩子、Cargo 命令钩子与 AI Skill 拓展")
                            .size(11.5)
                            .weak(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("🔄 重新扫描外部插件目录").clicked() {
                            self.load_external_plugins(workspace);
                        }
                    });
                });
                ui.separator();

                egui::ScrollArea::vertical()
                    .id_salt("plugins_list_scroll")
                    .max_height(310.0)
                    .show(ui, |ui| {
                        for p in &self.plugins {
                            let mut is_enabled = enabled_plugins.contains(&p.id);
                            egui::Frame::none()
                                .fill(ui.visuals().faint_bg_color)
                                .rounding(egui::Rounding::same(5.0))
                                .inner_margin(egui::Margin::same(8.0))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        if ui.checkbox(&mut is_enabled, "").changed() {
                                            if is_enabled {
                                                if !enabled_plugins.contains(&p.id) {
                                                    enabled_plugins.push(p.id.clone());
                                                }
                                            } else {
                                                enabled_plugins.retain(|id| id != &p.id);
                                            }
                                            action = PluginAction::ToggledPlugin;
                                        }
                                        ui.label(
                                            RichText::new(&p.name)
                                                .strong()
                                                .size(13.0)
                                                .color(BRAND_ACCENT),
                                        );
                                        ui.label(
                                            RichText::new(format!("v{}", p.version))
                                                .monospace()
                                                .size(11.0)
                                                .color(BRAND_GREEN),
                                        );
                                        ui.label(
                                            RichText::new(p.hook_kind.label())
                                                .size(11.0)
                                                .color(BRAND_PURPLE),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if is_enabled {
                                                    match p.hook_kind {
                                                        PluginHookKind::ToolbarAction
                                                        | PluginHookKind::CargoSubcommand => {
                                                            if ui.small_button("▶ 立即执行").clicked() {
                                                                action = PluginAction::RunCommand(
                                                                    p.command_or_rule.clone(),
                                                                );
                                                            }
                                                        }
                                                        PluginHookKind::AiPromptSkill => {
                                                            if ui
                                                                .small_button("🤖 发送至 AI 副驾驶")
                                                                .clicked()
                                                            {
                                                                action =
                                                                    PluginAction::SendAiSkillPrompt(
                                                                        p.command_or_rule.clone(),
                                                                    );
                                                            }
                                                        }
                                                        PluginHookKind::EditorInspection => {
                                                            ui.label(
                                                                RichText::new("● 实时生效中")
                                                                    .size(11.0)
                                                                    .color(BRAND_GREEN),
                                                            );
                                                        }
                                                    }
                                                }
                                            },
                                        );
                                    });
                                    ui.label(RichText::new(&p.description).size(11.5));
                                    ui.label(
                                        RichText::new(format!(
                                            "ID: {} · 作者: {} · 触发指令: `{}`",
                                            p.id, p.author, p.command_or_rule
                                        ))
                                        .size(10.5)
                                        .weak(),
                                    );
                                });
                            ui.add_space(6.0);
                        }
                    });

                ui.separator();
                ui.label(RichText::new("🛠 快速创建自定义拓展插件 (写入 plugins/ 目录)").strong().color(BRAND_WARN));
                ui.horizontal(|ui| {
                    ui.label("名称:");
                    ui.add(egui::TextEdit::singleline(&mut self.new_plugin_name).desired_width(130.0));
                    ui.label("绑定命令:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_plugin_cmd)
                            .desired_width(170.0)
                            .hint_text("如 cargo bench"),
                    );
                    ui.label("说明:");
                    ui.add(egui::TextEdit::singleline(&mut self.new_plugin_desc).desired_width(140.0));
                    if ui.button("＋ 注册插件").clicked() && !self.new_plugin_name.trim().is_empty() {
                        let id = self
                            .new_plugin_name
                            .trim()
                            .to_lowercase()
                            .replace(' ', "-");
                        let desc = NovaPluginDescriptor {
                            id: id.clone(),
                            name: self.new_plugin_name.trim().to_string(),
                            version: "0.1.0".to_string(),
                            author: "User Custom".to_string(),
                            description: if self.new_plugin_desc.trim().is_empty() {
                                "用户自定义命令扩展插件".to_string()
                            } else {
                                self.new_plugin_desc.trim().to_string()
                            },
                            hook_kind: PluginHookKind::ToolbarAction,
                            command_or_rule: if self.new_plugin_cmd.trim().is_empty() {
                                "cargo check".to_string()
                            } else {
                                self.new_plugin_cmd.trim().to_string()
                            },
                            builtin: false,
                        };
                        if let Some(ws) = workspace {
                            let p_dir = ws.join("plugins");
                            let _ = std::fs::create_dir_all(&p_dir);
                            if let Ok(json) = serde_json::to_string_pretty(&desc) {
                                let _ = std::fs::write(p_dir.join(format!("{}.json", id)), json);
                            }
                        }
                        enabled_plugins.push(id);
                        self.plugins.push(desc);
                        self.new_plugin_name.clear();
                        self.new_plugin_cmd.clear();
                        self.new_plugin_desc.clear();
                        action = PluginAction::ToggledPlugin;
                    }
                });
            });

        self.open = win_open;
        action
    }
}
