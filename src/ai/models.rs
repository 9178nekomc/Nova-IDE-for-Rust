// NovaRS — Multi-Protocol AI Model Manager Dialog (Add / Edit / Delete / Test Models)

use eframe::egui::{self, RichText};
use crate::config::{ApiProtocol, ModelProfile};
use crate::constants::{BRAND_ACCENT, BRAND_GREEN, BRAND_RED};

#[derive(Default)]
pub struct ModelManagerDialog {
    pub open: bool,
    pub selected_idx: usize,
    pub draft: Option<ModelProfile>,
}

impl ModelManagerDialog {
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        models: &mut Vec<ModelProfile>,
        active_model_id: &mut String,
    ) -> bool {
        let mut modified = false;
        if !self.open {
            return false;
        }
        if models.is_empty() {
            *models = crate::config::default_model_profiles();
            modified = true;
        }
        if self.selected_idx >= models.len() {
            self.selected_idx = 0;
        }

        let mut win_open = self.open;
        egui::Window::new("🤖 AI 模型与多协议接入管理 (Model & API Protocol Manager)")
            .open(&mut win_open)
            .resizable(true)
            .default_width(680.0)
            .default_height(460.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("支持协议: OpenAI / DeepSeek / OpenRouter · Anthropic Claude · Google Gemini · Ollama 本地模型")
                            .size(11.5)
                            .weak(),
                    );
                });
                ui.separator();

                ui.columns(2, |cols| {
                    // 左侧：模型配置列表
                    cols[0].vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.strong("已配置模型列表");
                            if ui.small_button("＋ 新增模型").clicked() {
                                let new_id = format!("custom-model-{}", models.len() + 1);
                                models.push(ModelProfile {
                                    id: new_id.clone(),
                                    name: "自定义 OpenAI/兼容模型".to_string(),
                                    protocol: ApiProtocol::OpenAiCompatible,
                                    base_url: "https://api.openai.com/v1/chat/completions".to_string(),
                                    api_key: String::new(),
                                    model_identifier: "gpt-4.1".to_string(),
                                    max_tokens: 8192,
                                    temperature: 0.2,
                                    supports_reasoning: true,
                                });
                                self.selected_idx = models.len() - 1;
                                modified = true;
                            }
                        });
                        ui.separator();

                        egui::ScrollArea::vertical()
                            .id_salt("model_list_scroll")
                            .max_height(320.0)
                            .show(ui, |ui| {
                                for (i, m) in models.iter().enumerate() {
                                    let is_sel = i == self.selected_idx;
                                    let is_active = m.id == *active_model_id;
                                    let prefix = if is_active { "🟢 " } else { "   " };
                                    let label = format!(
                                        "{}[{}] {}",
                                        prefix,
                                        m.protocol.short_tag(),
                                        m.name
                                    );
                                    if ui
                                        .selectable_label(
                                            is_sel,
                                            RichText::new(label).color(if is_active {
                                                BRAND_GREEN
                                            } else {
                                                ui.visuals().text_color()
                                            }),
                                        )
                                        .clicked()
                                    {
                                        self.selected_idx = i;
                                    }
                                }
                            });
                    });

                    // 右侧：当前选中模型的详细配置编辑
                    cols[1].vertical(|ui| {
                        let idx = self.selected_idx.min(models.len() - 1);
                        let m = &mut models[idx];

                        ui.horizontal(|ui| {
                            ui.strong("编辑模型参数");
                            if m.id != *active_model_id {
                                if ui
                                    .button(RichText::new("设为当前默认模型").color(BRAND_GREEN))
                                    .clicked()
                                {
                                    *active_model_id = m.id.clone();
                                    modified = true;
                                }
                            } else {
                                ui.label(RichText::new("★ 当前激活模型").color(BRAND_GREEN));
                            }
                        });
                        ui.separator();

                        egui::Grid::new("model_edit_grid")
                            .num_columns(2)
                            .spacing([10.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("显示名称:");
                                if ui
                                    .add(egui::TextEdit::singleline(&mut m.name).desired_width(240.0))
                                    .changed()
                                {
                                    modified = true;
                                }
                                ui.end_row();

                                ui.label("通信协议:");
                                egui::ComboBox::from_id_salt("protocol_selector")
                                    .selected_text(m.protocol.label())
                                    .width(240.0)
                                    .show_ui(ui, |ui| {
                                        for &p in ApiProtocol::ALL {
                                            if ui
                                                .selectable_value(&mut m.protocol, p, p.label())
                                                .clicked()
                                            {
                                                // 自动填充对应协议的标准端点提示
                                                match p {
                                                    ApiProtocol::OpenAiCompatible => {
                                                        if m.base_url.is_empty() {
                                                            m.base_url = "https://api.openai.com/v1/chat/completions".to_string();
                                                        }
                                                    }
                                                    ApiProtocol::AnthropicMessages => {
                                                        m.base_url = "https://api.anthropic.com/v1/messages".to_string();
                                                    }
                                                    ApiProtocol::GoogleGemini => {
                                                        m.base_url = "https://generativelanguage.googleapis.com/v1beta/models".to_string();
                                                    }
                                                    ApiProtocol::OllamaLocal => {
                                                        m.base_url = "http://127.0.0.1:11434/api/chat".to_string();
                                                    }
                                                }
                                                modified = true;
                                            }
                                        }
                                    });
                                ui.end_row();

                                ui.label("API 端点 (URL):");
                                if ui
                                    .add(
                                        egui::TextEdit::singleline(&mut m.base_url)
                                            .desired_width(240.0)
                                            .hint_text("https://..."),
                                    )
                                    .changed()
                                {
                                    modified = true;
                                }
                                ui.end_row();

                                ui.label("API Key:");
                                if ui
                                    .add(
                                        egui::TextEdit::singleline(&mut m.api_key)
                                            .password(true)
                                            .desired_width(240.0)
                                            .hint_text("sk-... (Ollama 本地可留空)"),
                                    )
                                    .changed()
                                {
                                    modified = true;
                                }
                                ui.end_row();

                                ui.label("模型 ID (Model):");
                                if ui
                                    .add(
                                        egui::TextEdit::singleline(&mut m.model_identifier)
                                            .desired_width(240.0)
                                            .hint_text("claude-sonnet-4-20250514 / o3-mini / deepseek-reasoner"),
                                    )
                                    .changed()
                                {
                                    modified = true;
                                }
                                ui.end_row();

                                ui.label("最大输出 Tokens:");
                                let mut max_tok = m.max_tokens as i32;
                                if ui
                                    .add(egui::DragValue::new(&mut max_tok).range(256..=65536).speed(128))
                                    .changed()
                                {
                                    m.max_tokens = max_tok.max(256) as u32;
                                    modified = true;
                                }
                                ui.end_row();

                                ui.label("采样温度 (Temp):");
                                if ui
                                    .add(egui::Slider::new(&mut m.temperature, 0.0..=1.5).step_by(0.05))
                                    .changed()
                                {
                                    modified = true;
                                }
                                ui.end_row();

                                ui.label("推理/思考预算:");
                                if ui
                                    .checkbox(
                                        &mut m.supports_reasoning,
                                        "支持 Codex 思考滑块 (reasoning_effort / thinking budget)",
                                    )
                                    .changed()
                                {
                                    modified = true;
                                }
                                ui.end_row();
                            });

                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            if models.len() > 1
                                && ui
                                    .button(RichText::new("🗑 删除此模型").color(BRAND_RED))
                                    .clicked()
                            {
                                let removed_id = models[idx].id.clone();
                                models.remove(idx);
                                if *active_model_id == removed_id {
                                    *active_model_id = models[0].id.clone();
                                }
                                self.selected_idx = 0;
                                modified = true;
                            }
                            if ui
                                .button(RichText::new("恢复预设模型列表").color(BRAND_ACCENT))
                                .clicked()
                            {
                                *models = crate::config::default_model_profiles();
                                *active_model_id = models[0].id.clone();
                                self.selected_idx = 0;
                                modified = true;
                            }
                        });
                    });
                });
            });

        self.open = win_open;
        modified
    }
}
