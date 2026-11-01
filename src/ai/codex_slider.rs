// NovaRS — Codex-Style Thinking Effort Slider Component
// Supports 5 discrete & smooth levels:
// 0: Minimal (Instant / 0 thinking budget)
// 1: Low     (Fast / 1,024 budget tokens / reasoning_effort="low")
// 2: Medium  (Balanced / 4,096 budget tokens / reasoning_effort="medium")
// 3: High    (Deep / 16,384 budget tokens / reasoning_effort="high")
// 4: Max     (xHigh / 32,768 budget tokens / Ultra reasoning)

use eframe::egui::{pos2, Color32, Rect, RichText, Sense, Stroke, Ui, Vec2};
use crate::constants::{BRAND_ACCENT, BRAND_GREEN, BRAND_PURPLE, BRAND_RED, BRAND_WARN};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EffortSpec {
    pub level: u8,
    pub tag: &'static str,
    pub cn_label: &'static str,
    pub openai_effort: &'static str,
    pub anthropic_budget_tokens: u32,
    pub color: Color32,
}

pub fn effort_spec(level: u8) -> EffortSpec {
    match level {
        0 => EffortSpec {
            level: 0,
            tag: "MIN",
            cn_label: "极速 (Minimal · 0 思考预算)",
            openai_effort: "low",
            anthropic_budget_tokens: 0,
            color: Color32::from_rgb(0x7f, 0x84, 0x8e),
        },
        1 => EffortSpec {
            level: 1,
            tag: "LOW",
            cn_label: "轻量思考 (Low · 1K Tokens)",
            openai_effort: "low",
            anthropic_budget_tokens: 1024,
            color: BRAND_GREEN,
        },
        2 => EffortSpec {
            level: 2,
            tag: "MED",
            cn_label: "标准推理 (Medium · 4K Tokens)",
            openai_effort: "medium",
            anthropic_budget_tokens: 4096,
            color: BRAND_ACCENT,
        },
        3 => EffortSpec {
            level: 3,
            tag: "HIGH",
            cn_label: "深度推理 (High · 16K Tokens)",
            openai_effort: "high",
            anthropic_budget_tokens: 16384,
            color: BRAND_WARN,
        },
        _ => EffortSpec {
            level: 4,
            tag: "xHIGH",
            cn_label: "极限超算 (Max xHigh · 32K Tokens)",
            openai_effort: "high",
            anthropic_budget_tokens: 32768,
            color: BRAND_PURPLE,
        },
    }
}

/// 绘制 Codex 风格分段式交互滑块（带辉光刻度点与实时 Token 预算指示）
pub fn draw_codex_thinking_slider(ui: &mut Ui, level: &mut u8) -> bool {
    let mut changed = false;
    let current = effort_spec(*level);

    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("🧠 思考强度 (Codex Effort):").size(11.5).strong());
            ui.label(
                RichText::new(format!("[{}] {}", current.tag, current.cn_label))
                    .size(11.5)
                    .color(current.color)
                    .strong(),
            );
        });

        let desired_w = ui.available_width().max(180.0);
        let bar_h = 24.0;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(desired_w, bar_h), Sense::click_and_drag());

        if resp.clicked() || resp.dragged() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let rel_x = ((pos.x - rect.left() - 10.0) / (rect.width() - 20.0)).clamp(0.0, 1.0);
                let new_lvl = (rel_x * 4.0).round() as u8;
                if new_lvl != *level {
                    *level = new_lvl;
                    changed = true;
                }
            }
        }

        let painter = ui.painter();
        let track_y = rect.center().y;
        let left_x = rect.left() + 12.0;
        let right_x = rect.right() - 12.0;
        let span_w = (right_x - left_x).max(40.0);

        // 背景轨道
        painter.line_segment(
            [pos2(left_x, track_y), pos2(right_x, track_y)],
            Stroke::new(4.0_f32, Color32::from_rgba_unmultiplied(120, 130, 150, 70)),
        );

        // 已激活轨道
        let active_x = left_x + (*level as f32 / 4.0) * span_w;
        painter.line_segment(
            [pos2(left_x, track_y), pos2(active_x, track_y)],
            Stroke::new(4.0_f32, current.color),
        );

        // 5 个档位刻度圆点与标签
        for idx in 0..=4u8 {
            let spec = effort_spec(idx);
            let x = left_x + (idx as f32 / 4.0) * span_w;
            let is_reached = idx <= *level;
            let is_current = idx == *level;

            let radius = if is_current { 6.5 } else { 4.2 };
            let fill = if is_reached {
                spec.color
            } else {
                Color32::from_rgb(0x45, 0x4a, 0x56)
            };
            painter.circle_filled(pos2(x, track_y), radius, fill);
            if is_current {
                painter.circle_stroke(pos2(x, track_y), radius + 2.0, Stroke::new(1.5_f32, BRAND_RED));
            }
        }

        // 快捷档位胶囊按钮
        ui.horizontal(|ui| {
            for idx in 0..=4u8 {
                let spec = effort_spec(idx);
                let selected = *level == idx;
                let btn_rect_resp = ui.selectable_label(
                    selected,
                    RichText::new(spec.tag)
                        .monospace()
                        .size(10.5)
                        .color(if selected { spec.color } else { Color32::GRAY }),
                );
                if btn_rect_resp.clicked() && *level != idx {
                    *level = idx;
                    changed = true;
                }
            }
        });
    });

    let _ = Rect::NOTHING;
    changed
}
