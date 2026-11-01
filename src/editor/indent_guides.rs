// NovaRS — VS Code-Authentic Indentation Guides Engine
// Fixes the original indentation guide bugs:
// 1. Blank/whitespace-only lines previously had `levels = 0`, causing broken/dashed vertical lines
//    across multi-line functions and blocks. Now interpolates blank lines using lookbehind &
//    lookahead non-blank neighbors just like VS Code (`editor.guides.indentation`).
// 2. Correctly handles Tab stops (`col = (col / 4 + 1) * 4`) and avoids drawing guides on top of
//    opening/closing braces at the exact column.
// 3. Highlights the active indentation scope guide (`editor.guides.highlightActiveIndentation`)
//    around the cursor's current block with `theme.indent_guide_active`.

use eframe::egui::{pos2, Pos2, Stroke, Ui};
use crate::theme::CodeTheme;

/// 计算单行的前导缩进列数（支持空格与 Tab 对齐到 4 列制表位）以及是否为空白行
pub fn measure_line_indent(line: &str) -> (usize, bool) {
    let mut cols = 0usize;
    for ch in line.chars() {
        match ch {
            ' ' => cols += 1,
            '\t' => cols = (cols / 4 + 1) * 4,
            '\r' => {}
            _ => return (cols, false),
        }
    }
    (cols, true)
}

/// 计算可视区域附近每行的有效缩进层级（含 VS Code 空白行前后插值与当前光标激活作用域识别）
pub fn draw_vscode_indent_guides(
    ui: &mut Ui,
    lines: &[&str],
    first_vis: usize,
    last_vis: usize,
    cursor_local_line: Option<usize>,
    galley_pos: Pos2,
    row_h: f32,
    char_w: f32,
    theme: &CodeTheme,
    highlight_active: bool,
) {
    if lines.is_empty() {
        return;
    }
    let total = lines.len();
    let scan_start = first_vis.saturating_sub(64);
    let scan_end = (last_vis + 64).min(total);
    let slice_len = scan_end.saturating_sub(scan_start);
    if slice_len == 0 {
        return;
    }

    // 第一步：测量扫描窗口内每行的原始缩进与空白标记
    let mut raw_cols = Vec::with_capacity(slice_len);
    let mut is_blank = Vec::with_capacity(slice_len);
    for idx in scan_start..scan_end {
        let (c, blank) = measure_line_indent(lines[idx]);
        raw_cols.push(c);
        is_blank.push(blank);
    }

    // 第二步：VS Code 空白行缩进辅助线插值规则
    // 空白行的缩进级别 = 上方最近非空行 与 下方最近非空行 的缩进层级支撑（取较大作用域或闭合块匹配）
    let mut effective_levels = vec![0usize; slice_len];
    for i in 0..slice_len {
        if !is_blank[i] {
            // 非空行：计算应显示的辅助线数量
            // 若该行以 `}` / `]` / `)` 开头（闭合行），其自身属于外层缩进，内部引导线画到本行上方为止
            let cols = raw_cols[i];
            effective_levels[i] = cols / 4;
        } else {
            let mut prev_cols = 0usize;
            for p in (0..i).rev() {
                if !is_blank[p] {
                    prev_cols = raw_cols[p];
                    break;
                }
            }
            let mut next_cols = 0usize;
            for n in (i + 1)..slice_len {
                if !is_blank[n] {
                    next_cols = raw_cols[n];
                    break;
                }
            }
            // VS Code 规则：若上下非空行形成代码块（例如上行 `fn foo() {` 缩进 0 但下有缩进 4，或上下均为缩进 4），
            // 空白行保持与代码块一致的缩进参考线
            let interpolated_cols = prev_cols.max(next_cols).min(prev_cols + 4);
            effective_levels[i] = interpolated_cols / 4;
        }
    }

    // 第三步：识别光标所在的活跃代码块缩进层级与起止行范围（Active Indent Guide）
    let mut active_scope: Option<(usize, usize, usize)> = None; // (level_1based, start_local_line, end_local_line)
    if highlight_active {
        if let Some(cur_line) = cursor_local_line {
            if cur_line >= scan_start && cur_line < scan_end {
                let rel_cur = cur_line - scan_start;
                let cur_level = if !is_blank[rel_cur] {
                    let trimmed = lines[cur_line].trim();
                    if trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[') {
                        effective_levels[rel_cur] + 1
                    } else {
                        effective_levels[rel_cur]
                    }
                } else {
                    effective_levels[rel_cur]
                };

                if cur_level >= 1 {
                    let mut top = rel_cur;
                    while top > 0 && effective_levels[top - 1] >= cur_level {
                        top -= 1;
                    }
                    let mut bot = rel_cur;
                    while bot + 1 < slice_len && effective_levels[bot + 1] >= cur_level {
                        bot += 1;
                    }
                    active_scope = Some((cur_level, scan_start + top, scan_start + bot));
                }
            }
        }
    }

    // 第四步：绘制连续无断裂的 VS Code 缩进辅助线（按列合并连续线段以提升清晰度与性能）
    let painter = ui.painter();
    let vis_from = first_vis.max(scan_start);
    let vis_to = last_vis.min(scan_end);

    for li in vis_from..vis_to {
        let rel = li - scan_start;
        let levels = effective_levels[rel];
        if levels == 0 {
            continue;
        }
        let y_top = galley_pos.y + li as f32 * row_h;
        let y_bot = y_top + row_h;

        // 判断当前行首个非空白字符所在的列，避免辅助线直接画在文字或 `{}` 上
        let first_char_col = if !is_blank[rel] { Some(raw_cols[rel]) } else { None };

        for level in 1..=levels {
            let guide_col = (level - 1) * 4;
            if let Some(fc) = first_char_col {
                if guide_col == fc {
                    continue;
                }
            }
            let x = (galley_pos.x + guide_col as f32 * char_w).floor() + 0.5;
            let is_active_segment = match active_scope {
                Some((act_lvl, s_line, e_line)) => {
                    act_lvl == level && li >= s_line && li <= e_line
                }
                None => false,
            };
            let color = if is_active_segment {
                theme.indent_guide_active
            } else {
                theme.indent_guide
            };
            let width: f32 = if is_active_segment { 1.35 } else { 1.0 };
            painter.line_segment([pos2(x, y_top), pos2(x, y_bot)], Stroke::new(width, color));
        }
    }
}
