// NovaRS — Dynamic Line Number Gutter & Diagnostic Marker Bar
// Fixes the 4000~5000 line truncation bug by:
// 1. Dynamically sizing gutter width for 1 to 1,000,000+ lines with generous margin
// 2. Supporting virtualized line window offsets (`base_line_0based`) so large files (>2500 lines)
//    never exceed egui/GPU texture or vertex limits
// 3. Synchronizing 1-based line numbers directly with the active viewport & galley row height
// 4. Displaying breakpoints, IDEA-style error/warning badges, and active-line highlight

use std::collections::{HashMap, HashSet};
use eframe::egui::{self, pos2, Align2, Color32, FontId, Painter, Rect, Sense, Stroke, Vec2};
use crate::constants::{BRAND_RED, GUTTER_BP_W, MONO_SIZE};
use crate::editor::diagnostics::DiagSeverity;
use crate::theme::CodeTheme;

pub struct GutterRenderParams<'a> {
    pub total_lines: usize,
    pub base_line_0based: usize,
    pub visible_line_count: usize,
    pub scroll_offset_y: f32,
    pub row_h: f32,
    pub char_w: f32,
    pub cursor_line_1based: usize,
    pub breakpoints: &'a HashSet<usize>,
    pub line_diagnostics: &'a HashMap<usize, DiagSeverity>,
    pub theme: &'a CodeTheme,
}

pub struct GutterInteraction {
    pub toggled_breakpoint_line: Option<usize>,
    pub clicked_diagnostic_line: Option<usize>,
}

/// 计算行号栏总宽度（保证 5~7 位行号永不因宽度不足被截断）
pub fn compute_gutter_width(total_lines: usize, char_w: f32) -> (usize, f32, f32) {
    let digits = total_lines.max(1).to_string().len().max(4);
    let num_w = (digits as f32 + 1.5) * char_w + 18.0;
    let total_w = GUTTER_BP_W + num_w + 12.0;
    (digits, num_w, total_w)
}

pub fn draw_gutter(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    params: &GutterRenderParams<'_>,
) -> GutterInteraction {
    let (digits, _num_w, sidebar_w) = compute_gutter_width(params.total_lines, params.char_w);
    let avail_h = ui.available_height().max(120.0);
    let (side_rect, side_resp) = ui.allocate_exact_size(Vec2::new(sidebar_w, avail_h), Sense::click());

    let mut interaction = GutterInteraction {
        toggled_breakpoint_line: None,
        clicked_diagnostic_line: None,
    };

    // 使用独立 Painter 但限定在 side_rect 的垂直可视范围内，水平方向留足裕量，彻底杜绝截断
    let clip_rect = Rect::from_min_max(
        pos2(side_rect.left(), side_rect.top()),
        pos2(side_rect.right() + 4.0, side_rect.bottom()),
    );
    let sp = Painter::new(ctx.clone(), ui.layer_id(), clip_rect);

    sp.rect_filled(side_rect, 0.0, params.theme.gutter_bg);
    // 右侧微细分隔线
    sp.line_segment(
        [
            pos2(side_rect.right() - 1.0, side_rect.top()),
            pos2(side_rect.right() - 1.0, side_rect.bottom()),
        ],
        Stroke::new(1.0_f32, params.theme.indent_guide),
    );

    let first_local = (params.scroll_offset_y / params.row_h).floor().max(0.0) as usize;
    let visible_rows = (avail_h / params.row_h).ceil() as usize + 3;
    let last_local = (first_local + visible_rows).min(params.visible_line_count);

    if side_resp.clicked() {
        if let Some(pos) = side_resp.interact_pointer_pos() {
            let local_row = ((pos.y - side_rect.top() + params.scroll_offset_y) / params.row_h)
                .floor()
                .max(0.0) as usize;
            let global_line_1based = params.base_line_0based + local_row + 1;
            if global_line_1based >= 1 && global_line_1based <= params.total_lines {
                if params.line_diagnostics.contains_key(&global_line_1based)
                    && pos.x > side_rect.left() + GUTTER_BP_W
                {
                    interaction.clicked_diagnostic_line = Some(global_line_1based);
                } else {
                    interaction.toggled_breakpoint_line = Some(global_line_1based);
                }
            }
        }
    }

    let right_num_anchor_x = side_rect.right() - 8.0;
    let bp_center_x = side_rect.left() + GUTTER_BP_W * 0.5 + 2.0;

    for local_idx in first_local..last_local {
        let global_line_1based = params.base_line_0based + local_idx + 1;
        if global_line_1based > params.total_lines {
            break;
        }
        let y = side_rect.top() + (local_idx as f32 * params.row_h) - params.scroll_offset_y;
        if y + params.row_h < side_rect.top() || y > side_rect.bottom() {
            continue;
        }

        let is_active = global_line_1based == params.cursor_line_1based;
        if is_active {
            let row_rect = Rect::from_min_size(
                pos2(side_rect.left(), y),
                Vec2::new(side_rect.width() - 1.0, params.row_h),
            );
            sp.rect_filled(row_rect, 0.0, params.theme.current_line_bg);
        }

        // 1) 断点或 IDEA 风格诊断图标
        let center_y = y + params.row_h * 0.5;
        if params.breakpoints.contains(&global_line_1based) {
            sp.circle_filled(pos2(bp_center_x, center_y), 4.2, BRAND_RED);
            sp.circle_stroke(
                pos2(bp_center_x, center_y),
                4.2,
                Stroke::new(1.0_f32, Color32::WHITE),
            );
        } else if let Some(sev) = params.line_diagnostics.get(&global_line_1based) {
            let badge_color = match sev {
                DiagSeverity::Error => params.theme.error_squiggly,
                DiagSeverity::Warning => params.theme.warning_squiggly,
                DiagSeverity::Info => params.theme.info_squiggly,
            };
            sp.circle_filled(pos2(bp_center_x, center_y), 3.8, badge_color);
        }

        // 2) 行号文本（右对齐渲染，无论 4000 行、50000 行还是 100000 行均完整显示）
        let num_color = if let Some(DiagSeverity::Error) = params.line_diagnostics.get(&global_line_1based) {
            params.theme.error_squiggly
        } else if is_active {
            params.theme.line_number_active
        } else {
            params.theme.line_number
        };

        let text_str = format!("{:>width$}", global_line_1based, width = digits);
        sp.text(
            pos2(right_num_anchor_x, y),
            Align2::RIGHT_TOP,
            text_str,
            FontId::monospace(MONO_SIZE),
            num_color,
        );
    }

    interaction
}
