// NovaRS — Syntax Highlighting & Large-File Minimap Engine

use eframe::egui::text::LayoutJob;
use eframe::egui::{Color32, FontId, TextFormat};
use crate::constants::{MINIMAP_W, MONO_SIZE, RUST_KEYWORDS};
use crate::theme::{CodeTheme, TokKind};
use crate::utils::{char_literal_end, utf8_char_len};

const CONTROL_FLOW_KEYWORDS: &[&str] = &[
    "async", "await", "break", "continue", "else", "for", "if", "loop", "match", "return", "while",
];

#[derive(Clone)]
pub struct MiniRow {
    pub indent: f32,
    pub width: f32,
    pub color: Color32,
    pub has_error: bool,
}

/// 构建代码迷你地图（支持超过 4000 行的超大文件自动步进采样，保证全文件比例完整映射）
pub fn build_minimap(text: &str) -> Vec<MiniRow> {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len().max(1);
    let max_samples = 2400usize;
    let step = (total / max_samples).max(1);
    let mut rows = Vec::with_capacity(total.min(max_samples) + 1);

    for idx in (0..total).step_by(step) {
        let line = lines[idx];
        let indent_spaces = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        let color = if trimmed.starts_with("//") {
            Color32::from_rgb(0x6a, 0x99, 0x55)
        } else if trimmed.starts_with('"') || trimmed.starts_with("#[") || trimmed.starts_with("#![") {
            Color32::from_rgb(0x98, 0xc3, 0x79)
        } else {
            let first_word: String = trimmed
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if RUST_KEYWORDS.contains(&first_word.as_str()) {
                Color32::from_rgb(0xc6, 0x78, 0xdd)
            } else if first_word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                Color32::from_rgb(0xe5, 0xc0, 0x7b)
            } else if trimmed.is_empty() {
                Color32::from_rgb(0x25, 0x28, 0x2e)
            } else {
                Color32::from_rgb(0x61, 0xaf, 0xef)
            }
        };
        let width = (trimmed.chars().count() as f32 * 0.9).min(MINIMAP_W - 8.0);
        rows.push(MiniRow {
            indent: (indent_spaces as f32 * 0.65).min(36.0),
            width,
            color,
            has_error: false,
        });
    }
    rows
}

fn push_token(
    job: &mut LayoutJob,
    text: &str,
    start: usize,
    end: usize,
    kind: TokKind,
    theme: &CodeTheme,
    font: &FontId,
) {
    if end <= start || start >= text.len() {
        return;
    }
    let end = end.min(text.len());
    let mut fmt = TextFormat::simple(font.clone(), theme.color_of(kind));
    if matches!(kind, TokKind::Comment) {
        fmt.italics = true;
    }
    job.append(&text[start..end], 0.0, fmt);
}

pub fn highlight_rust(text: &str, theme: &CodeTheme, job: &mut LayoutJob) {
    let font = FontId::monospace(MONO_SIZE);
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    while i < n {
        let b = bytes[i];
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            let start = i;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            push_token(job, text, start, i, TokKind::Comment, theme, &font);
        } else if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            let start = i;
            let mut depth = 1usize;
            i += 2;
            while i < n && depth > 0 {
                if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && i + 1 < n && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            push_token(job, text, start, i, TokKind::Comment, theme, &font);
        } else if b == b'#' && i + 1 < n && (bytes[i + 1] == b'[' || bytes[i + 1] == b'!') {
            let start = i;
            let mut bracket_depth = 0usize;
            while i < n && bytes[i] != b'\n' {
                if bytes[i] == b'[' {
                    bracket_depth += 1;
                } else if bytes[i] == b']' {
                    bracket_depth = bracket_depth.saturating_sub(1);
                    if bracket_depth == 0 {
                        i += 1;
                        break;
                    }
                }
                i += utf8_char_len(bytes[i]);
            }
            push_token(job, text, start, i, TokKind::Attribute, theme, &font);
        } else if b == b'"' {
            let start = i;
            i += 1;
            while i < n {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(n);
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            push_token(job, text, start, i, TokKind::Str, theme, &font);
        } else if b == b'\'' {
            if let Some(end) = char_literal_end(bytes, i) {
                push_token(job, text, i, end, TokKind::Str, theme, &font);
                i = end;
            } else {
                let start = i;
                i += 1;
                while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                push_token(job, text, start, i, TokKind::Lifetime, theme, &font);
            }
        } else if b == b'r' && i + 1 < n && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') {
            let start = i;
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < n && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < n && bytes[j] == b'"' {
                j += 1;
                let mut end = n;
                while j < n {
                    if bytes[j] == b'"' {
                        let mut k = j + 1;
                        let mut cnt = 0usize;
                        while cnt < hashes && k < n && bytes[k] == b'#' {
                            cnt += 1;
                            k += 1;
                        }
                        if cnt == hashes {
                            end = k;
                            break;
                        }
                        j = k;
                    } else {
                        j += 1;
                    }
                }
                i = end;
                push_token(job, text, start, i, TokKind::Str, theme, &font);
            } else {
                while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let word = &text[start..i];
                let kind = if CONTROL_FLOW_KEYWORDS.contains(&word) {
                    TokKind::ControlFlow
                } else if RUST_KEYWORDS.contains(&word) {
                    TokKind::Keyword
                } else {
                    TokKind::Normal
                };
                push_token(job, text, start, i, kind, theme, &font);
            }
        } else if b.is_ascii_digit() {
            let start = i;
            while i < n
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_')
            {
                i += 1;
            }
            push_token(job, text, start, i, TokKind::Number, theme, &font);
        } else if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            let kind = if i < n && bytes[i] == b'!' {
                i += 1;
                TokKind::Macro
            } else if CONTROL_FLOW_KEYWORDS.contains(&word) {
                TokKind::ControlFlow
            } else if RUST_KEYWORDS.contains(&word) {
                TokKind::Keyword
            } else if word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                TokKind::Ty
            } else if i < n && bytes[i] == b'(' {
                TokKind::Function
            } else {
                TokKind::Normal
            };
            push_token(job, text, start, i, kind, theme, &font);
        } else {
            let start = i;
            i += utf8_char_len(b);
            push_token(job, text, start, i, TokKind::Normal, theme, &font);
        }
    }
}
