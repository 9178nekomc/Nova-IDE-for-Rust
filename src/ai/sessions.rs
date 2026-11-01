// NovaRS — Multi-Conversation Session Manager & Context Attachment Engine

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtractedCodePatch {
    pub file_hint: String,
    pub start_line: usize,
    pub end_line: usize,
    pub original_code: String,
    pub replacement_code: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    pub reasoning_trace: Option<String>,
    pub timestamp: String,
    pub patches: Vec<ExtractedCodePatch>,
    pub mcp_tool_call: Option<(String, serde_json::Value)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: u64,
    pub title: String,
    pub workspace_scope: Option<PathBuf>,
    pub messages: Vec<ChatMessage>,
    pub attach_active_file: bool,
    pub attach_diagnostics: bool,
    pub attach_ast_outline: bool,
}

impl ChatSession {
    pub fn new(id: u64, title: impl Into<String>, workspace: Option<PathBuf>) -> Self {
        ChatSession {
            id,
            title: title.into(),
            workspace_scope: workspace,
            messages: Vec::new(),
            attach_active_file: true,
            attach_diagnostics: true,
            attach_ast_outline: false,
        }
    }
}

/// 从 AI 回复中解析 `novars-patch` 结构化补丁或标准 ```rust 代码块
pub fn extract_patches_from_response(
    response: &str,
    default_file: &str,
    default_line: usize,
) -> Vec<ExtractedCodePatch> {
    let mut patches = Vec::new();

    // 1. 解析标准 ```novars-patch 块
    let mut rest = response;
    while let Some(start) = rest.find("```novars-patch") {
        let after_start = &rest[start + "```novars-patch".len()..];
        let Some(end) = after_start.find("```") else { break };
        let block = after_start[..end].trim();
        if let Some(patch) = parse_novars_patch_block(block, default_file, default_line) {
            patches.push(patch);
        }
        rest = &after_start[end + 3..];
    }

    // 2. 如果没有显式 novars-patch，但有 ```rust 代码块，提取为可选替换块
    if patches.is_empty() {
        let mut r2 = response;
        while let Some(start) = r2.find("```rust") {
            let after_start = &r2[start + "```rust".len()..];
            let Some(end) = after_start.find("```") else { break };
            let code = after_start[..end].trim().to_string();
            if !code.is_empty() {
                patches.push(ExtractedCodePatch {
                    file_hint: default_file.to_string(),
                    start_line: default_line.max(1),
                    end_line: default_line.max(1),
                    original_code: String::new(),
                    replacement_code: code,
                });
            }
            r2 = &after_start[end + 3..];
        }
    }

    patches
}

fn parse_novars_patch_block(
    block: &str,
    default_file: &str,
    default_line: usize,
) -> Option<ExtractedCodePatch> {
    let mut file_hint = default_file.to_string();
    let mut start_line = default_line.max(1);
    let mut end_line = default_line.max(1);

    for line in block.lines().take(4) {
        if let Some(f) = line.strip_prefix("FILE:") {
            file_hint = f.trim().to_string();
        } else if let Some(l) = line.strip_prefix("LINES:") {
            let l = l.trim();
            if let Some((a, b)) = l.split_once('-') {
                start_line = a.trim().parse().unwrap_or(default_line);
                end_line = b.trim().parse().unwrap_or(start_line);
            } else if let Ok(single) = l.parse::<usize>() {
                start_line = single;
                end_line = single;
            }
        }
    }

    if let (Some(orig_pos), Some(rep_pos)) = (block.find("<<<ORIGINAL"), block.find("===REPLACE")) {
        let orig_start = orig_pos + "<<<ORIGINAL".len();
        let original_code = block[orig_start..rep_pos].trim_matches('\n').to_string();
        let rep_start = rep_pos + "===REPLACE".len();
        let rep_end = block[rep_start..]
            .find(">>>")
            .map(|i| rep_start + i)
            .unwrap_or(block.len());
        let replacement_code = block[rep_start..rep_end].trim_matches('\n').to_string();
        return Some(ExtractedCodePatch {
            file_hint,
            start_line,
            end_line,
            original_code,
            replacement_code,
        });
    }

    None
}

/// 从 AI 回复中提取 `novars-mcp` 工具调用请求
pub fn extract_mcp_tool_call(response: &str) -> Option<(String, serde_json::Value)> {
    let start = response.find("```novars-mcp")?;
    let after = &response[start + "```novars-mcp".len()..];
    let end = after.find("```")?;
    let json_str = after[..end].trim();
    let val: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let tool = val.get("tool")?.as_str()?.to_string();
    let args = val.get("args").cloned().unwrap_or(serde_json::json!({}));
    Some((tool, args))
}
