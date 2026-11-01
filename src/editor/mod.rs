// NovaRS — Editor Module (Tab State, Find/Search, Completion Popup, Large-File Windowing)

pub mod diagnostics;
pub mod gutter;
pub mod indent_guides;
pub mod syntax;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use crate::editor::diagnostics::CodeDiagnostic;
use crate::lsp::LspCompletion;

#[derive(Clone)]
pub struct SearchHit {
    pub path: PathBuf,
    pub line_no: usize,
    pub text: String,
}

pub struct SearchState {
    pub input: String,
    pub case_sensitive: bool,
    pub running: Arc<AtomicBool>,
    pub results: Arc<Mutex<Vec<SearchHit>>>,
}

pub struct FindState {
    pub input: String,
    pub replace_input: String,
    pub show_replace: bool,
    pub total: usize,
    pub index: usize,
}

#[derive(Clone)]
pub struct CompletionPopup {
    pub items: Vec<LspCompletion>,
    pub selected: usize,
    pub gen: i64,
    pub base_char: usize,
}

pub struct EditorTab {
    pub id: u64,
    pub path: PathBuf,
    pub title: String,
    pub text: String,
    pub dirty: bool,
    pub undo: Vec<String>,
    pub redo: Vec<String>,
    pub last_text: String,
    pub last_snapshot: Instant,
    pub breakpoints: HashSet<usize>,
    pub line_ending: &'static str,
    /// 大文件分页/滑动视口起始行（0-based），用于解决 4000~5000+ 行超大文件在 egui 单 Galley 下的截断问题
    pub virtual_start_line: usize,
    /// 当前文件的实时诊断列表
    pub diagnostics: Vec<CodeDiagnostic>,
    pub selected_diagnostic: Option<usize>,
    pub last_diag_hash: u64,
}
