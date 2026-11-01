// NovaRS — Main Application State & UI Orchestrator (`src/app.rs`)

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;
use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::widgets::text_edit::TextEditState;
use egui::{
    pos2, Align, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, FontSelection,
    Galley, Id, Key, Rect, RichText, Sense, Stroke, Vec2, ViewportCommand,
};
use walkdir::WalkDir;

use crate::ai::sessions::{ChatMessage, ChatRole, ExtractedCodePatch};
use crate::ai::AiSidebarState;
use crate::ast::{ast_kind_color, collect_ast_items, AstItem};
use crate::config::{
    load_settings, save_settings_to_disk, ProxyMode, Settings,
};
use crate::constants::*;
use crate::deps::{DepManagerAction, DependencyManagerState};
use crate::editor::diagnostics::{
    build_line_severity_map, draw_diagnostic_inspector_bar, draw_inline_diagnostics,
    parse_cargo_json_diagnostics, run_live_inspections, CodeDiagnostic, DiagSeverity,
    PendingFixPatch,
};
use crate::editor::gutter::{compute_gutter_width, draw_gutter, GutterRenderParams};
use crate::editor::indent_guides::draw_vscode_indent_guides;
use crate::editor::syntax::{build_minimap, highlight_rust, MiniRow};
use crate::editor::{CompletionPopup, EditorTab, FindState, SearchHit, SearchState};
use crate::icons::{draw_file_badge, icon_for_folder, icon_for_path, lang_of_path, rich_file_label};
use crate::lsp::{builtin_completions, path_to_uri, start_rust_analyzer, LspSession};
use crate::mcp::McpServerState;
use crate::plugins::{PluginAction, PluginManagerState};
use crate::project::{
    sanitize_package_name, template_cargo_toml, template_main_rs, BuildProfile, ProjectDialog,
    RunMode, PROJECT_TEMPLATES,
};
use crate::terminal::TerminalState;
use crate::theme::{CodeTheme, ThemePreset};
use crate::toolchain::{
    apply_crates_mirror, detect_toolchain, draw_environment_doctor_window, toolchain_summary,
    SystemInfo,
};
use crate::utils::{
    apply_no_window, center_window, char_to_byte, count_lines, indent_lines,
    line_col_of_char_index, line_start_byte_index, line_start_char_index, lock,
    read_clipboard_text, read_git_branch, replace_char_range, reveal_in_explorer,
    slice_by_char_range, splash_duration_from_env, unindent_lines,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Splash,
    Main,
}

#[derive(Clone)]
pub enum NameKind {
    NewFolder(PathBuf),
    Rename(PathBuf),
}

pub struct NameDialog {
    pub title: String,
    pub hint: String,
    pub value: String,
    pub error: Option<String>,
    pub kind: NameKind,
}

pub struct FileNode {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub children: Vec<usize>,
}

pub fn is_ignored_entry(entry: &walkdir::DirEntry) -> bool {
    const IGNORED: [&str; 3] = [".git", "target", "node_modules"];
    entry.file_type().is_dir()
        && IGNORED.contains(&entry.file_name().to_string_lossy().as_ref())
}

pub fn build_tree(root: &Path) -> (Vec<FileNode>, Vec<usize>) {
    let mut nodes: Vec<FileNode> = Vec::new();
    let mut index: HashMap<PathBuf, usize> = HashMap::new();
    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root.display().to_string());
    nodes.push(FileNode {
        name: root_name,
        path: root.to_path_buf(),
        is_dir: true,
        children: Vec::new(),
    });
    index.insert(root.to_path_buf(), 0);

    let walker = WalkDir::new(root).min_depth(1).sort_by(|a, b| {
        let a_dir = a.file_type().is_dir();
        let b_dir = b.file_type().is_dir();
        b_dir.cmp(&a_dir).then_with(|| a.file_name().cmp(b.file_name()))
    });

    for entry in walker
        .into_iter()
        .filter_entry(|e| !is_ignored_entry(e))
        .filter_map(|e| e.ok())
    {
        let path = entry.path().to_path_buf();
        let parent_idx = entry.path().parent().and_then(|p| index.get(p)).copied();
        if let Some(pidx) = parent_idx {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().is_dir();
            let idx = nodes.len();
            nodes.push(FileNode {
                name,
                path: path.clone(),
                is_dir,
                children: Vec::new(),
            });
            nodes[pidx].children.push(idx);
            index.insert(path, idx);
        }
    }
    (nodes, vec![0])
}

pub struct App {
    pub phase: Phase,
    pub splash_start: Instant,
    pub splash_duration: Duration,
    pub splash_centered: bool,
    pub toolchain: Arc<Mutex<SystemInfo>>,
    pub settings: Settings,

    pub workspace: Option<PathBuf>,
    pub nodes: Vec<FileNode>,
    pub tree_root: Vec<usize>,

    pub tabs: Vec<EditorTab>,
    pub active_tab: Option<usize>,
    pub next_tab_id: u64,
    pub last_cursor_range: Option<(u64, (usize, usize))>,
    pub pending_selection: Option<(u64, usize, usize)>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub scroll_to_line: Option<(u64, usize)>,

    pub show_ast: bool,
    pub show_output: bool,
    pub about_open: bool,
    pub network_dialog_open: bool,
    pub env_doctor_open: bool,
    pub build_profile: BuildProfile,

    pub output_log: Arc<Mutex<String>>,
    pub cargo_child: Arc<Mutex<Option<Child>>>,

    pub ast_items: Vec<AstItem>,
    pub ast_error: Option<String>,
    pub ast_sig: Option<(u64, u64)>,

    pub name_dialog: Option<NameDialog>,
    pub project_dialog: Option<ProjectDialog>,
    pub toast: Option<(String, Instant)>,
    pub main_size_ok: bool,
    pub capture_dir: Option<PathBuf>,
    pub capture_seen: Option<SystemTime>,
    pub capture_name: Option<String>,

    pub lsp_slot: Arc<Mutex<Option<Arc<LspSession>>>>,
    pub lsp: Option<Arc<LspSession>>,
    pub ra_gate: Arc<AtomicBool>,
    pub completion: Option<CompletionPopup>,
    pub completion_anchor: Option<egui::Pos2>,
    pub force_completion: bool,
    pub popup_cmd: Option<u8>,
    pub auto_complete_pending: bool,
    pub ime_composing: bool,
    pub ime_commit_at: Option<Instant>,

    pub error_files: Arc<Mutex<HashSet<PathBuf>>>,
    pub cargo_diagnostics: Arc<Mutex<Vec<CodeDiagnostic>>>,
    pub pending_fix_patch: Option<PendingFixPatch>,

    pub terminal: TerminalState,
    pub ai_sidebar: AiSidebarState,
    pub mcp_server: McpServerState,
    pub dep_manager: DependencyManagerState,
    pub plugin_manager: PluginManagerState,

    pub search: Option<SearchState>,
    pub find: Option<FindState>,
    pub minimap_cache: Option<(u64, Vec<MiniRow>)>,
    pub scroll_to_offset: Option<f32>,
    pub editor_scroll_meta: Option<(f32, f32, f32, f32)>,
    pub match_highlight: Option<(u64, usize, usize)>,
    pub git_branch: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut settings = load_settings();
        setup_fonts(&cc.egui_ctx);
        settings.dark_mode = settings.theme_preset.is_dark();
        settings.theme_preset.apply_to_context(&cc.egui_ctx);

        let toolchain = Arc::new(Mutex::new(SystemInfo::default()));
        let ra_gate = Arc::new(AtomicBool::new(false));
        let lsp_slot: Arc<Mutex<Option<Arc<LspSession>>>> = Arc::new(Mutex::new(None));

        let startup_workspace: Option<PathBuf> = settings
            .last_workspace
            .as_ref()
            .map(PathBuf::from)
            .filter(|p| p.is_dir());

        {
            let tc = toolchain.clone();
            let gate = ra_gate.clone();
            let slot = lsp_slot.clone();
            let ws = startup_workspace.clone();
            std::thread::spawn(move || {
                detect_toolchain(&tc);
                if let Some(session) = start_rust_analyzer(RA_EMBEDDED, ws, gate) {
                    *lock(&slot) = Some(session);
                }
            });
        }

        let mcp_server = McpServerState::new(
            settings.mcp_server_port,
            settings.mcp_server_enabled,
            settings.mcp_builtin_sidebar_connected,
            startup_workspace.clone(),
        );
        let ai_sidebar = AiSidebarState::new(startup_workspace.clone());
        let mut plugin_manager = PluginManagerState::default();
        plugin_manager.load_external_plugins(startup_workspace.as_deref());

        let mut app = App {
            phase: Phase::Splash,
            splash_start: Instant::now(),
            splash_duration: splash_duration_from_env(),
            splash_centered: false,
            toolchain,
            settings,
            workspace: None,
            nodes: Vec::new(),
            tree_root: Vec::new(),
            tabs: Vec::new(),
            active_tab: None,
            next_tab_id: 1,
            last_cursor_range: None,
            pending_selection: None,
            cursor_line: 1,
            cursor_col: 1,
            scroll_to_line: None,
            show_ast: false,
            show_output: false,
            about_open: false,
            network_dialog_open: false,
            env_doctor_open: false,
            build_profile: BuildProfile::Debug,
            output_log: Arc::new(Mutex::new(String::new())),
            cargo_child: Arc::new(Mutex::new(None)),
            ast_items: Vec::new(),
            ast_error: None,
            ast_sig: None,
            name_dialog: None,
            project_dialog: None,
            toast: None,
            main_size_ok: false,
            capture_dir: std::env::var("NOVARS_CAPTURE").ok().map(PathBuf::from),
            capture_seen: None,
            capture_name: None,
            lsp_slot,
            lsp: None,
            ra_gate,
            completion: None,
            completion_anchor: None,
            force_completion: false,
            popup_cmd: None,
            auto_complete_pending: false,
            ime_composing: false,
            ime_commit_at: None,
            error_files: Arc::new(Mutex::new(HashSet::new())),
            cargo_diagnostics: Arc::new(Mutex::new(Vec::new())),
            pending_fix_patch: None,
            terminal: TerminalState::new(),
            ai_sidebar,
            mcp_server,
            dep_manager: DependencyManagerState::default(),
            plugin_manager,
            search: None,
            find: None,
            minimap_cache: None,
            scroll_to_offset: None,
            editor_scroll_meta: None,
            match_highlight: None,
            git_branch: None,
        };

        if let Some(path) = startup_workspace {
            app.workspace = Some(path);
            app.refresh_tree();
            app.git_branch = read_git_branch(app.workspace.as_deref());
        }
        app
    }

    pub fn save_settings(&mut self) {
        self.settings.dark_mode = self.settings.theme_preset.is_dark();
        self.settings.last_workspace = self.workspace.as_ref().map(|p| p.display().to_string());
        self.settings.mcp_server_port = self.mcp_server.port;
        self.settings.mcp_server_enabled = self.mcp_server.running.load(Ordering::Relaxed);
        self.settings.mcp_builtin_sidebar_connected = self.mcp_server.builtin_sidebar_connected;
        save_settings_to_disk(&self.settings);
    }

    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    // ---------- 全局快捷键与编辑器按键处理 ----------

    pub fn handle_global_shortcuts(&mut self, ctx: &egui::Context) {
        if self.phase != Phase::Main {
            return;
        }
        let key = |key: egui::Key, command: bool, shift: bool| {
            ctx.input(|i| {
                i.events.iter().any(|e| {
                    matches!(e, egui::Event::Key { key: k, pressed: true, modifiers: m, .. }
                        if *k == key && m.command == command && m.shift == shift)
                })
            })
        };

        if key(Key::Space, true, false) {
            ctx.input_mut(|i| {
                i.events.retain(|e| {
                    !matches!(e, egui::Event::Key { key: Key::Space, pressed: true, modifiers: m, .. } if m.command)
                })
            });
            self.force_completion = true;
        }
        if key(Key::F5, false, false) {
            self.run_cargo(ctx, RunMode::Run);
        }
        if key(Key::F5, false, true) {
            self.run_cargo(ctx, RunMode::DebugRun);
        }
        if key(Key::F7, false, false) {
            self.run_cargo(ctx, RunMode::Check);
        }
        if key(Key::N, true, false) {
            self.menu_new_file();
        }
        if key(Key::O, true, false) {
            self.menu_open_folder();
        }
        if key(Key::S, true, false) {
            self.save_active(ctx);
        }
        if key(Key::S, true, true) {
            self.save_all(ctx);
        }
        if key(Key::W, true, false) {
            if let Some(i) = self.active_tab {
                self.request_close_tab(i);
            }
        }
        if key(Key::Backtick, true, false) {
            self.toggle_terminal(ctx);
        }
        if key(Key::I, true, true) {
            self.ai_sidebar.visible = !self.ai_sidebar.visible;
        }
        if key(Key::F, true, true) {
            self.search = Some(SearchState {
                input: String::new(),
                case_sensitive: false,
                running: Arc::new(AtomicBool::new(false)),
                results: Arc::new(Mutex::new(Vec::new())),
            });
            self.completion = None;
        }
        if key(Key::F, true, false) && self.find.is_none() {
            self.find = Some(FindState {
                input: String::new(),
                replace_input: String::new(),
                show_replace: false,
                total: 0,
                index: 0,
            });
        }
    }

    pub fn handle_popup_keys(&mut self, ctx: &egui::Context) {
        if self.completion.is_none() {
            return;
        }
        let mut cmd: Option<u8> = None;
        ctx.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Key { key, pressed: true, .. } => match key {
                    Key::ArrowDown => {
                        cmd = Some(0);
                        false
                    }
                    Key::ArrowUp => {
                        cmd = Some(1);
                        false
                    }
                    Key::Tab => {
                        cmd = Some(2);
                        false
                    }
                    Key::Escape => {
                        cmd = Some(3);
                        false
                    }
                    _ => true,
                },
                _ => true,
            })
        });
        self.popup_cmd = cmd;
    }

    pub fn handle_tab_indent(&mut self, ctx: &egui::Context) {
        if self.completion.is_some() {
            return;
        }
        let (tab_pressed, shift_held) = ctx.input(|i| {
            let mut tp = false;
            let mut sh = false;
            for e in &i.events {
                if let egui::Event::Key {
                    key: Key::Tab,
                    pressed: true,
                    modifiers,
                    ..
                } = e
                {
                    tp = true;
                    sh = modifiers.shift;
                }
            }
            (tp, sh)
        });
        if !tab_pressed {
            return;
        }
        ctx.input_mut(|i| {
            i.events
                .retain(|e| !matches!(e, egui::Event::Key { key: Key::Tab, pressed: true, .. }))
        });
        let Some(ti) = self.active_tab else { return };
        let tab_id = self.tabs[ti].id;
        let editor_id = Self::editor_id_of(tab_id);
        let base_char_offset = line_start_char_index(&self.tabs[ti].text, self.tabs[ti].virtual_start_line + 1);

        let live_range = TextEditState::load(ctx, editor_id)
            .and_then(|st| st.cursor.char_range())
            .map(|r| (r.primary.index + base_char_offset, r.secondary.index + base_char_offset))
            .map(|(a, b)| (tab_id, (a, b)));

        let selection = live_range.or(self.last_cursor_range);
        let Some((tab_id, (c1, c2))) = selection.filter(|(id, _)| *id == tab_id) else { return };
        let tab_path = self.tabs[ti].path.clone();

        if shift_held {
            let text = self.tabs[ti].text.clone();
            let (new_text, new_start, new_end) = unindent_lines(&text, c1.min(c2), c1.max(c2));
            self.tabs[ti].text = new_text;
            self.tabs[ti].dirty = true;
            self.set_editor_selection(tab_id, new_start, new_end);
        } else if c1 == c2 {
            let tab = &mut self.tabs[ti];
            tab.text = replace_char_range(&tab.text, c1, c1, "    ");
            tab.dirty = true;
            self.set_editor_cursor(ctx, tab_id, c1 + 4);
        } else {
            let text = self.tabs[ti].text.clone();
            let (new_text, new_start, new_end) = indent_lines(&text, c1.min(c2), c1.max(c2));
            self.tabs[ti].text = new_text;
            self.tabs[ti].dirty = true;
            self.set_editor_selection(tab_id, new_start, new_end);
        }
        let tab_text = self.tabs[ti].text.clone();
        if let Some(lsp) = &self.lsp {
            lsp.notify_did_change(&tab_path, &tab_text);
        }
    }

    pub fn completion_word_prefix(text: &str, char_idx: usize) -> (usize, String) {
        let byte_idx = char_to_byte(text, char_idx);
        let start_byte = text[..byte_idx]
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
            .map(|(b, _)| b)
            .last();
        match start_byte {
            Some(b) => {
                let start_char = text[..b].chars().count();
                let prefix = slice_by_char_range(text, start_char, char_idx);
                (start_char, prefix)
            }
            None => (char_idx, String::new()),
        }
    }

    pub fn open_completion(&mut self, force: bool) {
        let Some(ti) = self.active_tab else { return };
        let Some((tab_id, (char_idx, _))) = self.last_cursor_range else { return };
        if self.tabs.get(ti).map(|t| t.id) != Some(tab_id) {
            return;
        }
        let tab_text = self.tabs[ti].text.clone();
        let tab_path = self.tabs[ti].path.clone();
        let (base_char, prefix) = Self::completion_word_prefix(&tab_text, char_idx);
        if !force && prefix.is_empty() {
            self.completion = None;
            return;
        }
        let items = builtin_completions(&tab_text, &prefix, self.workspace.as_deref());
        let mut gen = -1i64;
        if let Some(lsp) = &self.lsp {
            if lsp.ready.load(Ordering::Relaxed) && lsp.alive.load(Ordering::Relaxed) {
                let (line, col) = line_col_of_char_index(&tab_text, char_idx);
                let uri = path_to_uri(&tab_path);
                gen = lsp.request_completion(&uri, line, col.saturating_sub(1));
            }
        }
        if gen < 0 && items.is_empty() {
            self.completion = None;
            return;
        }
        self.completion = Some(CompletionPopup {
            items,
            selected: 0,
            gen,
            base_char,
        });
    }

    pub fn apply_completion(&mut self, ctx: &egui::Context) {
        let Some(popup) = self.completion.clone() else { return };
        let Some(item) = popup.items.get(popup.selected) else { return };
        let insert = item.label.trim_end_matches('(').to_string();
        let Some(ti) = self.active_tab else { return };
        let Some((tab_id, (char_idx, _))) = self.last_cursor_range else { return };
        if self.tabs.get(ti).map(|t| t.id) != Some(tab_id) {
            return;
        }
        {
            let tab = &mut self.tabs[ti];
            let new_text = replace_char_range(&tab.text, popup.base_char, char_idx, &insert);
            let new_cursor = popup.base_char + insert.chars().count();
            tab.text = new_text;
            tab.dirty = true;
            self.set_editor_cursor(ctx, tab_id, new_cursor);
        }
        let tab_path = self.tabs[ti].path.clone();
        let tab_text = self.tabs[ti].text.clone();
        if let Some(lsp) = &self.lsp {
            lsp.notify_did_change(&tab_path, &tab_text);
        }
        self.completion = None;
        self.set_toast(format!("已补全: {}", insert));
    }

    pub fn save_all(&mut self, ctx: &egui::Context) {
        let mut saved = 0usize;
        for i in 0..self.tabs.len() {
            if self.tabs[i].dirty {
                let path = self.tabs[i].path.clone();
                let text = self.tabs[i].text.clone();
                match std::fs::write(&path, &text) {
                    Ok(()) => {
                        self.tabs[i].dirty = false;
                        saved += 1;
                    }
                    Err(e) => self.set_toast(format!("保存失败 {}: {}", path.display(), e)),
                }
            }
        }
        if saved > 0 {
            self.set_toast(format!("已保存全部 {} 个修改的文件", saved));
            self.spawn_error_check(ctx);
        } else {
            self.set_toast("没有待保存的修改");
        }
    }

    pub fn spawn_error_check(&mut self, ctx: &egui::Context) {
        let Some(ws) = self.workspace.clone() else { return };
        let set = self.error_files.clone();
        let cargo_diags_arc = self.cargo_diagnostics.clone();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let mut errors: HashSet<PathBuf> = HashSet::new();
            let mut parsed_diags: Vec<CodeDiagnostic> = Vec::new();
            let mut cmd = Command::new("cargo");
            apply_no_window(&mut cmd)
                .args(["check", "--message-format=json"])
                .current_dir(&ws);
            if let Ok(output) = cmd.output() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                parsed_diags = parse_cargo_json_diagnostics(&stdout, Some(&ws));
                for d in &parsed_diags {
                    if d.severity == DiagSeverity::Error {
                        errors.insert(d.file_path.clone());
                    }
                }
            }
            *lock(&set) = errors;
            *lock(&cargo_diags_arc) = parsed_diags;
            ctx2.request_repaint();
        });
    }

    pub fn toggle_terminal(&mut self, ctx: &egui::Context) {
        self.terminal.visible = !self.terminal.visible;
        if self.terminal.visible {
            let ws = self.workspace.clone();
            self.terminal.ensure_spawned(ws.as_deref(), ctx);
            self.terminal.focus_input = true;
        }
    }

    pub fn start_project_search(&mut self, ctx: &egui::Context) {
        let Some(ws) = self.workspace.clone() else {
            self.set_toast("请先打开一个文件夹");
            return;
        };
        let Some(search) = &mut self.search else { return };
        let query = search.input.trim().to_string();
        if query.is_empty() {
            self.set_toast("请输入搜索关键词");
            return;
        }
        if search.running.swap(true, Ordering::Relaxed) {
            return;
        }
        let case_sensitive = search.case_sensitive;
        let results = search.results.clone();
        let running = search.running.clone();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let needle = if case_sensitive {
                query.clone()
            } else {
                query.to_lowercase()
            };
            let mut hits: Vec<SearchHit> = Vec::new();
            let walker = WalkDir::new(&ws)
                .min_depth(1)
                .into_iter()
                .filter_entry(|e| !is_ignored_entry(e))
                .filter_map(|e| e.ok());
            for entry in walker {
                if hits.len() >= 1000 {
                    break;
                }
                if entry.file_type().is_dir() {
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(entry.path()) else { continue };
                if content.contains('\0') {
                    continue;
                }
                'lines: for (idx, line) in content.lines().enumerate() {
                    let matched = if case_sensitive {
                        line.contains(&query)
                    } else {
                        line.to_lowercase().contains(&needle)
                    };
                    if matched {
                        hits.push(SearchHit {
                            path: entry.path().to_path_buf(),
                            line_no: idx + 1,
                            text: line.trim().chars().take(160).collect(),
                        });
                        if hits.len() >= 1000 {
                            break 'lines;
                        }
                    }
                }
            }
            *lock(&results) = hits;
            running.store(false, Ordering::Relaxed);
            ctx2.request_repaint();
        });
    }

    pub fn find_next_in_file(&mut self, ctx: &egui::Context) {
        let Some(query) = self.find.as_ref().map(|f| f.input.clone()) else { return };
        if query.is_empty() {
            return;
        }
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let text = self.tabs[ti].text.clone();
        let needle = query.to_lowercase();
        // 小写化可能改变字节长度，必须经映射回到原串偏移，否则切片会落在非字符边界上 panic
        let (lower_text, offset_map) = crate::utils::lowercase_with_offset_map(&text);
        let mut char_positions: Vec<usize> = Vec::new();
        let mut byte_from = 0usize;
        while let Some(rel) = lower_text[byte_from..].find(&needle) {
            let byte_lower = byte_from + rel;
            let byte_at = match offset_map.as_ref().and_then(|m| m.get(byte_lower)).copied() {
                Some(b) => b,
                None => byte_lower,
            };
            char_positions.push(text[..byte_at].chars().count());
            byte_from = byte_lower + needle.len().max(1);
            if char_positions.len() >= 5000 {
                break;
            }
        }
        let total = char_positions.len();
        let Some(find) = self.find.as_mut() else { return };
        find.total = total;
        if total == 0 {
            self.set_toast(format!("未找到 “{}”", query));
            return;
        }
        let idx = find.index % total;
        find.index = find.index.wrapping_add(1);
        let char_at = char_positions[idx];
        let line = line_col_of_char_index(&text, char_at).0;
        let tab_id = self.tabs[ti].id;
        self.jump_to_line(ctx, line);
        self.set_editor_cursor(ctx, tab_id, char_at);
        self.match_highlight = Some((tab_id, char_at, char_at + query.chars().count()));
    }

    // ---------- 代码修复补丁应用 / 接受 / 撤回 ----------

    pub fn apply_ai_or_quick_patch(
        &mut self,
        ctx: &egui::Context,
        start_line_1based: usize,
        end_line_1based: usize,
        replacement: &str,
        reason: &str,
    ) {
        let Some(ti) = self.active_tab else { return };
        let tab = &mut self.tabs[ti];
        let original_full = tab.text.clone();
        let mut lines: Vec<String> = tab.text.lines().map(|s| s.to_string()).collect();
        let total = lines.len().max(1);
        let s_idx = start_line_1based.saturating_sub(1).min(total);
        let e_idx = end_line_1based.min(total).max(s_idx);

        let rep_lines: Vec<String> = replacement.lines().map(|s| s.to_string()).collect();
        lines.splice(s_idx..e_idx, rep_lines);
        let mut patched = lines.join("\n");
        if original_full.ends_with('\n') && !patched.ends_with('\n') {
            patched.push('\n');
        }

        tab.undo.push(original_full.clone());
        tab.text = patched.clone();
        tab.last_text = patched.clone();
        tab.dirty = true;
        tab.last_diag_hash = 0; // 强制刷新实时诊断

        let tab_id = tab.id;
        let file_path = tab.path.clone();

        self.pending_fix_patch = Some(PendingFixPatch {
            tab_id,
            file_path,
            diagnostic_message: reason.to_string(),
            target_line: start_line_1based,
            original_full_text: original_full,
            patched_full_text: patched,
            diff_summary: format!("L{}-L{} → `{}`", start_line_1based, end_line_1based, replacement.trim()),
            applied: true,
        });

        self.jump_to_line(ctx, start_line_1based);
        self.set_toast(format!(
            "✨ 已应用修复预览 (行 {}) — 您可在上方点击「接受」或「撤回」",
            start_line_1based
        ));
    }

    pub fn accept_pending_patch(&mut self, ctx: &egui::Context) {
        if let Some(patch) = self.pending_fix_patch.take() {
            self.save_active(ctx);
            self.set_toast(format!("✅ 已接受并保存行 {} 的修复修改", patch.target_line));
        }
    }

    pub fn revert_pending_patch(&mut self) {
        if let Some(patch) = self.pending_fix_patch.take() {
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == patch.tab_id) {
                tab.text = patch.original_full_text;
                tab.last_text = tab.text.clone();
                tab.dirty = true;
                tab.last_diag_hash = 0;
                self.set_toast(format!("↩ 已撤回行 {} 的修复修改，恢复原状", patch.target_line));
            }
        }
    }

    // ---------- 工作区与文件管理 ----------

    pub fn refresh_tree(&mut self) {
        if let Some(ws) = self.workspace.clone() {
            let (nodes, root) = build_tree(&ws);
            self.nodes = nodes;
            self.tree_root = root;
        } else {
            self.nodes.clear();
            self.tree_root.clear();
        }
    }

    pub fn set_workspace(&mut self, path: PathBuf) {
        let p_str = path.display().to_string();
        self.settings.recent_workspaces.retain(|x| x != &p_str);
        self.settings.recent_workspaces.insert(0, p_str);
        self.settings.recent_workspaces.truncate(12);

        self.workspace = Some(path.clone());
        self.mcp_server.update_workspace(Some(path.clone()));
        self.dep_manager.reload_from_workspace(Some(&path));
        self.plugin_manager.load_external_plugins(Some(&path));
        self.refresh_tree();
        self.git_branch = read_git_branch(self.workspace.as_deref());
        self.save_settings();
    }

    pub fn open_file_in_tab(&mut self, path: PathBuf) {
        if let Some(i) = self.tabs.iter().position(|t| t.path == path) {
            self.active_tab = Some(i);
            self.last_cursor_range = None;
            self.ast_sig = None;
            self.completion = None;
            return;
        }
        match std::fs::read_to_string(&path) {
            Ok(raw) => {
                let line_ending: &'static str = if raw.contains("\r\n") { "CRLF" } else { "LF" };
                let content = raw.replace("\r\n", "\n");
                let title = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                let id = self.next_tab_id;
                self.next_tab_id += 1;

                let cargo_diags = lock(&self.cargo_diagnostics).clone();
                let initial_diags = run_live_inspections(&path, &content, &cargo_diags);

                self.tabs.push(EditorTab {
                    id,
                    path: path.clone(),
                    title,
                    text: content.clone(),
                    dirty: false,
                    undo: Vec::new(),
                    redo: Vec::new(),
                    last_text: content,
                    last_snapshot: Instant::now(),
                    breakpoints: HashSet::new(),
                    line_ending,
                    virtual_start_line: 0,
                    diagnostics: initial_diags,
                    selected_diagnostic: None,
                    last_diag_hash: 0,
                });
                self.active_tab = Some(self.tabs.len() - 1);
                self.last_cursor_range = None;
                self.ast_sig = None;
                self.completion = None;
                if let Some(lsp) = &self.lsp {
                    if lsp.ready.load(Ordering::Relaxed) {
                        let text = self.tabs[self.tabs.len() - 1].text.clone();
                        lsp.notify_did_open(&path, &text);
                    }
                }
            }
            Err(e) => self.set_toast(format!("无法打开文件: {}", e)),
        }
    }

    pub fn save_active(&mut self, ctx: &egui::Context) {
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let result = {
            let tab = &self.tabs[ti];
            std::fs::write(&tab.path, &tab.text)
        };
        match result {
            Ok(()) => {
                self.tabs[ti].dirty = false;
                self.set_toast(format!("已保存: {}", self.tabs[ti].path.display()));
                self.spawn_error_check(ctx);
            }
            Err(e) => self.set_toast(format!("保存失败: {}", e)),
        }
    }

    pub fn save_tab_at(&mut self, idx: usize) {
        let result = {
            let tab = &self.tabs[idx];
            std::fs::write(&tab.path, &tab.text)
        };
        match result {
            Ok(()) => self.tabs[idx].dirty = false,
            Err(e) => self.set_toast(format!("保存失败: {}", e)),
        }
    }

    pub fn remove_tab_at(&mut self, idx: usize) {
        self.tabs.remove(idx);
        self.active_tab = match (self.active_tab, self.tabs.len()) {
            (_, 0) => None,
            (Some(a), n) if a >= n => Some(n - 1),
            (a, _) => a,
        };
        self.last_cursor_range = None;
        self.ast_sig = None;
        self.completion = None;
    }

    pub fn request_close_tab(&mut self, idx: usize) {
        let (title, dirty) = {
            let tab = &self.tabs[idx];
            (tab.title.clone(), tab.dirty)
        };
        if dirty {
            let desc = format!("“{}” 有未保存的修改，是否保存？", title);
            let confirmed = rfd::MessageDialog::new()
                .set_title("关闭标签页")
                .set_description(&desc)
                .set_buttons(rfd::MessageButtons::YesNo)
                .set_level(rfd::MessageLevel::Info)
                .show();
            if matches!(confirmed, rfd::MessageDialogResult::Yes) {
                self.save_tab_at(idx);
            }
        }
        self.remove_tab_at(idx);
        self.set_toast(format!("已关闭 {}", title));
    }

    pub fn close_tabs_under(&mut self, target: &Path) {
        self.tabs
            .retain(|t| !(t.path == target || t.path.starts_with(target)));
        self.active_tab = match (self.active_tab, self.tabs.len()) {
            (_, 0) => None,
            (Some(a), n) if a >= n => Some(n - 1),
            (a, _) => a,
        };
        self.last_cursor_range = None;
        self.ast_sig = None;
        self.completion = None;
    }

    pub fn delete_path(&mut self, path: PathBuf) {
        let is_dir = path.is_dir();
        let kind = if is_dir { "文件夹" } else { "文件" };
        let desc = format!(
            "确定要删除这个{}吗？\n{}\n\n此操作不可撤销。",
            kind,
            path.display()
        );
        let confirmed = rfd::MessageDialog::new()
            .set_title("删除确认")
            .set_description(&desc)
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_level(rfd::MessageLevel::Warning)
            .show();
        if matches!(confirmed, rfd::MessageDialogResult::Yes) {
            let result = if is_dir {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            match result {
                Ok(()) => {
                    self.set_toast(format!("已删除 {}", path.display()));
                    self.refresh_tree();
                    self.close_tabs_under(&path);
                }
                Err(e) => self.set_toast(format!("删除失败: {}", e)),
            }
        }
    }

    // ---------- 菜单动作 ----------

    pub fn menu_new_project(&mut self) {
        let Some(location) = rfd::FileDialog::new()
            .set_title("选择新项目的创建位置")
            .pick_folder()
        else {
            return;
        };
        self.project_dialog = Some(ProjectDialog {
            name: String::new(),
            location,
            template: 1,
            error: None,
        });
    }

    pub fn create_project(&mut self, dialog: ProjectDialog) -> Option<ProjectDialog> {
        let name = dialog.name.trim().to_string();
        if name.is_empty() {
            let mut d = dialog;
            d.error = Some("项目名称不能为空".to_string());
            return Some(d);
        }
        if name.contains('\\') || name.contains('/') {
            let mut d = dialog;
            d.error = Some("项目名称不能包含路径分隔符".to_string());
            return Some(d);
        }
        let package = sanitize_package_name(&name);
        let project_dir = dialog.location.join(&name);
        if project_dir.exists() {
            let mut d = dialog;
            d.error = Some(format!("目录已存在: {}", project_dir.display()));
            return Some(d);
        }
        let src_dir = project_dir.join("src");
        if std::fs::create_dir_all(&src_dir).is_err() {
            let mut d = dialog;
            d.error = Some("创建目录失败".to_string());
            return Some(d);
        }
        let cargo = template_cargo_toml(&package);
        let main_rs = template_main_rs(dialog.template);
        let entry_file = if dialog.template == 6 {
            src_dir.join("lib.rs")
        } else {
            src_dir.join("main.rs")
        };
        if std::fs::write(project_dir.join("Cargo.toml"), cargo).is_err()
            || std::fs::write(&entry_file, main_rs).is_err()
        {
            let _ = std::fs::remove_dir_all(&project_dir);
            let mut d = dialog;
            d.error = Some("写入项目文件失败".to_string());
            return Some(d);
        }
        self.set_workspace(project_dir);
        self.open_file_in_tab(entry_file);
        self.set_toast(format!(
            "已创建项目 “{}”（模板: {}）",
            name, PROJECT_TEMPLATES[dialog.template]
        ));
        None
    }

    pub fn menu_new_file(&mut self) {
        if self.workspace.is_none() {
            self.set_toast("请先打开一个文件夹（文件 → 打开文件夹）");
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .set_title("新建文件")
            .set_file_name("未命名.rs")
            .save_file()
        {
            if path.exists() {
                self.open_file_in_tab(path);
            } else {
                match std::fs::write(&path, "") {
                    Ok(()) => {
                        self.set_toast(format!("已创建: {}", path.display()));
                        self.refresh_tree();
                        self.open_file_in_tab(path);
                    }
                    Err(e) => self.set_toast(format!("创建失败: {}", e)),
                }
            }
        }
    }

    pub fn menu_new_folder(&mut self) {
        if self.workspace.is_none() {
            self.set_toast("请先打开一个文件夹（文件 → 打开文件夹）");
            return;
        }
        if let Some(parent) = rfd::FileDialog::new()
            .set_title("选择新建文件夹的位置")
            .pick_folder()
        {
            self.name_dialog = Some(NameDialog {
                title: "新建文件夹".to_string(),
                hint: format!("在 {} 下创建新文件夹，请输入名称：", parent.display()),
                value: "新建文件夹".to_string(),
                error: None,
                kind: NameKind::NewFolder(parent),
            });
        }
    }

    pub fn menu_open_folder(&mut self) {
        if let Some(folder) = rfd::FileDialog::new().set_title("打开文件夹").pick_folder() {
            self.set_workspace(folder);
            self.set_toast("文件夹已打开");
        }
    }

    pub fn exit_app(&mut self, ctx: &egui::Context) {
        {
            let mut guard = lock(&self.cargo_child);
            if let Some(child) = guard.as_mut() {
                let _ = child.kill();
            }
        }
        self.terminal.kill_all();
        self.mcp_server.stop_server();
        self.save_settings();
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    // ---------- 编辑器状态与虚拟窗口跳转 ----------

    pub fn editor_id_of(tab_id: u64) -> Id {
        Id::new(("novars_editor", tab_id))
    }

    pub fn set_editor_cursor(&mut self, _ctx: &egui::Context, tab_id: u64, char_idx: usize) {
        self.pending_selection = Some((tab_id, char_idx, char_idx));
        self.last_cursor_range = Some((tab_id, (char_idx, char_idx)));
    }

    pub fn set_editor_selection(&mut self, tab_id: u64, start: usize, end: usize) {
        self.pending_selection = Some((tab_id, start.min(end), start.max(end)));
        self.last_cursor_range = Some((tab_id, (start.min(end), start.max(end))));
    }

    pub fn track_undo(&mut self, ti: usize) {
        let changed = self.tabs[ti].text != self.tabs[ti].last_text;
        if changed {
            {
                let tab = &mut self.tabs[ti];
                let should_push = tab.last_snapshot.elapsed() >= Duration::from_millis(600)
                    || tab.undo.is_empty();
                if should_push {
                    tab.undo.push(tab.last_text.clone());
                    if tab.undo.len() > 300 {
                        tab.undo.remove(0);
                    }
                    tab.last_snapshot = Instant::now();
                }
                tab.redo.clear();
                tab.last_text = tab.text.clone();
                tab.dirty = true;
            }
            self.auto_complete_pending = true;
            self.match_highlight = None;
            if let Some(lsp) = &self.lsp {
                if lsp.ready.load(Ordering::Relaxed) {
                    let tab = &self.tabs[ti];
                    lsp.notify_did_change(&tab.path, &tab.text);
                }
            }
        }

        // 实时代码预检更新（当文件哈希变化或开启了实时检查时自动触发）
        if self.settings.auto_precheck_on_type {
            let mut hasher = DefaultHasher::new();
            self.tabs[ti].text.hash(&mut hasher);
            let h = hasher.finish();
            if self.tabs[ti].last_diag_hash != h {
                self.tabs[ti].last_diag_hash = h;
                let cargo_diags = lock(&self.cargo_diagnostics).clone();
                let diags = run_live_inspections(
                    &self.tabs[ti].path,
                    &self.tabs[ti].text,
                    &cargo_diags,
                );
                self.tabs[ti].diagnostics = diags;
            }
        }
    }

    pub fn edit_undo(&mut self) {
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let popped = self.tabs[ti].undo.pop();
        match popped {
            Some(prev) => {
                let tab = &mut self.tabs[ti];
                tab.redo.push(tab.text.clone());
                tab.text = prev;
                tab.last_text = tab.text.clone();
                tab.dirty = true;
                tab.last_diag_hash = 0;
                self.set_toast("已撤销");
            }
            None => self.set_toast("没有可撤销的操作"),
        }
    }

    pub fn edit_redo(&mut self) {
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let popped = self.tabs[ti].redo.pop();
        match popped {
            Some(next) => {
                let tab = &mut self.tabs[ti];
                tab.undo.push(tab.text.clone());
                tab.text = next;
                tab.last_text = tab.text.clone();
                tab.dirty = true;
                tab.last_diag_hash = 0;
                self.set_toast("已重做");
            }
            None => self.set_toast("没有可重做的操作"),
        }
    }

    pub fn edit_copy(&mut self, ctx: &egui::Context) {
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let Some((tid, (a, b))) = self.last_cursor_range else {
            self.set_toast("没有选中的文本");
            return;
        };
        if tid != self.tabs[ti].id || b == a {
            self.set_toast("没有选中的文本");
            return;
        }
        let content = slice_by_char_range(&self.tabs[ti].text, a.min(b), a.max(b));
        ctx.copy_text(content);
        self.set_toast("已复制到剪贴板");
    }

    pub fn edit_cut(&mut self, ctx: &egui::Context) {
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let Some((tid, (a, b))) = self.last_cursor_range else {
            self.set_toast("没有选中的文本");
            return;
        };
        if tid != self.tabs[ti].id || b == a {
            self.set_toast("没有选中的文本");
            return;
        }
        let (lo, hi) = (a.min(b), a.max(b));
        let removed = slice_by_char_range(&self.tabs[ti].text, lo, hi);
        ctx.copy_text(removed);
        let new_text = replace_char_range(&self.tabs[ti].text, lo, hi, "");
        self.tabs[ti].text = new_text;
        self.tabs[ti].dirty = true;
        self.set_editor_cursor(ctx, tid, lo);
        self.set_toast("已剪切");
    }

    pub fn edit_paste(&mut self, ctx: &egui::Context) {
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let Some(clip) = read_clipboard_text() else {
            self.set_toast("无法读取剪贴板内容");
            return;
        };
        let tab_id = self.tabs[ti].id;
        let (a, b) = match self.last_cursor_range {
            Some((tid, (a, b))) if tid == tab_id => (a.min(b), a.max(b)),
            _ => {
                let end = self.tabs[ti].text.chars().count();
                (end, end)
            }
        };
        let new_text = replace_char_range(&self.tabs[ti].text, a, b, &clip);
        let new_cursor = a + clip.chars().count();
        self.tabs[ti].text = new_text;
        self.tabs[ti].dirty = true;
        self.set_editor_cursor(ctx, tab_id, new_cursor);
        self.set_toast("已粘贴");
    }

    pub fn edit_select_all(&mut self, _ctx: &egui::Context) {
        let Some(ti) = self.active_tab else {
            self.set_toast("没有打开的文件");
            return;
        };
        let (tab_id, count) = {
            let tab = &self.tabs[ti];
            (tab.id, tab.text.chars().count())
        };
        self.set_editor_selection(tab_id, 0, count);
    }

    pub fn jump_to_line(&mut self, ctx: &egui::Context, line: usize) {
        let Some(ti) = self.active_tab else { return };
        let tab_id = self.tabs[ti].id;
        let total_lines = count_lines(&self.tabs[ti].text);
        let target_line = line.clamp(1, total_lines);

        // 若当前文件是超大文件（启用了滑动窗口分页），自动将窗口定位到包含目标行的区间！
        if total_lines > LARGE_FILE_LINE_THRESHOLD {
            let cur_start = self.tabs[ti].virtual_start_line;
            let cur_end = cur_start + VIRTUAL_CHUNK_LINES;
            if target_line - 1 < cur_start || target_line - 1 >= cur_end {
                self.tabs[ti].virtual_start_line =
                    target_line.saturating_sub(VIRTUAL_CHUNK_LINES / 3);
            }
        } else {
            self.tabs[ti].virtual_start_line = 0;
        }

        let char_idx = line_start_char_index(&self.tabs[ti].text, target_line);
        self.set_editor_cursor(ctx, tab_id, char_idx);
        self.scroll_to_line = Some((tab_id, target_line));
    }

    // ---------- Cargo 构建 / 运行 / 调试 / 检查 ----------

    pub fn run_cargo(&mut self, ctx: &egui::Context, mode: RunMode) {
        let Some(ws) = self.workspace.clone() else {
            self.set_toast("请先打开一个 Rust 项目文件夹");
            return;
        };
        if !ws.join("Cargo.toml").is_file() {
            self.set_toast("当前文件夹不是 Cargo 项目（缺少 Cargo.toml）");
            return;
        }
        let already_running = lock(&self.cargo_child).is_some();
        if already_running {
            self.set_toast("已有任务正在运行，请先终止");
            return;
        }

        let is_release = self.build_profile == BuildProfile::Release;
        let mut cmd_desc = format!("$ cargo {:?}", mode).to_lowercase();
        if is_release {
            cmd_desc.push_str(" --release");
        }

        {
            let mut log = lock(&self.output_log);
            log.clear();
            log.push_str(&format!("{} (Profile: {})\n", mode.label(), self.build_profile.label()));
            if mode == RunMode::DebugRun {
                log.push_str("🐞 [NovaRS Debugger] 已开启 RUST_BACKTRACE=full 栈回溯追踪与断点日志监控\n");
            }
            log.push_str("────────────────────────────────────────────────────────\n");
        }
        self.show_output = true;

        let mut command = Command::new("cargo");
        apply_no_window(&mut command);
        match mode {
            RunMode::Run => {
                command.arg("run");
                if is_release {
                    command.arg("--release");
                }
            }
            RunMode::DebugRun => {
                command.arg("run");
                command.env("RUST_BACKTRACE", "full");
                command.env("RUST_LOG", "debug");
            }
            RunMode::Build => {
                command.arg("build");
                if is_release {
                    command.arg("--release");
                }
            }
            RunMode::Check => {
                command.arg("check");
            }
            RunMode::Test => {
                command.arg("test");
                command.args(["--", "--nocapture"]);
            }
            RunMode::Clippy => {
                command.arg("clippy");
            }
            RunMode::Fmt => {
                command.arg("fmt");
            }
            RunMode::Doc => {
                command.args(["doc", "--no-deps"]);
            }
        }

        // 注入网络代理环境变量（若用户在设置中开启了代理）
        if let Some(proxy) = self.settings.network.effective_proxy_url() {
            command.env("HTTP_PROXY", &proxy);
            command.env("HTTPS_PROXY", &proxy);
            command.env("CARGO_HTTP_PROXY", &proxy);
        }

        command
            .current_dir(&ws)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        match command.spawn() {
            Ok(child) => {
                *lock(&self.cargo_child) = Some(child);
                let child_arc = self.cargo_child.clone();
                let log_arc = self.output_log.clone();
                {
                    let mut guard = lock(&child_arc);
                    if let Some(c) = guard.as_mut() {
                        let stdout = c.stdout.take();
                        spawn_reader(stdout, log_arc.clone(), ctx.clone());
                        let stderr = c.stderr.take();
                        spawn_reader(stderr, log_arc.clone(), ctx.clone());
                    }
                }
                let ws2 = ws.clone();
                let error_files = self.error_files.clone();
                let cargo_diags_arc = self.cargo_diagnostics.clone();
                let ctx2 = ctx.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(200));
                    let mut guard = lock(&child_arc);
                    match guard.as_mut() {
                        Some(c) => match c.try_wait() {
                            Ok(Some(status)) => {
                                lock(&log_arc).push_str(&format!("\n[进程已退出: {}]\n", status));
                                *guard = None;
                                ctx2.request_repaint();

                                let mut check_cmd = Command::new("cargo");
                                apply_no_window(&mut check_cmd)
                                    .args(["check", "--message-format=json"])
                                    .current_dir(&ws2);
                                if let Ok(output) = check_cmd.output() {
                                    let stdout = String::from_utf8_lossy(&output.stdout);
                                    let diags = parse_cargo_json_diagnostics(&stdout, Some(&ws2));
                                    let mut errors: HashSet<PathBuf> = HashSet::new();
                                    for d in &diags {
                                        if d.severity == DiagSeverity::Error {
                                            errors.insert(d.file_path.clone());
                                        }
                                    }
                                    *lock(&error_files) = errors;
                                    *lock(&cargo_diags_arc) = diags;
                                }
                                ctx2.request_repaint();
                                break;
                            }
                            Ok(None) => {}
                            Err(_) => {
                                *guard = None;
                                break;
                            }
                        },
                        None => break,
                    }
                });
                self.set_toast(format!("已启动 {}", mode.label()));
            }
            Err(e) => self.set_toast(format!("启动 cargo 失败: {}", e)),
        }
    }

    pub fn terminate_cargo(&mut self) {
        let killed = {
            let mut guard = lock(&self.cargo_child);
            match guard.as_mut() {
                Some(child) => {
                    let _ = child.kill();
                    true
                }
                None => false,
            }
        };
        if killed {
            lock(&self.output_log).push_str("\n[已请求终止进程]\n");
            self.set_toast("已发送终止信号");
        } else {
            self.set_toast("没有正在运行的任务");
        }
    }

    // ---------- 弹窗与对话框 ----------

    pub fn confirm_name_dialog(&mut self, dialog: NameDialog) -> Option<NameDialog> {
        let name = dialog.value.trim().to_string();
        if name.is_empty() {
            let mut d = dialog;
            d.error = Some("名称不能为空".to_string());
            return Some(d);
        }
        if name.contains('\\') || name.contains('/') {
            let mut d = dialog;
            d.error = Some("名称不能包含路径分隔符".to_string());
            return Some(d);
        }
        match dialog.kind {
            NameKind::NewFolder(ref parent) => {
                let target = parent.join(&name);
                match std::fs::create_dir_all(&target) {
                    Ok(()) => {
                        self.set_toast(format!("已创建文件夹: {}", target.display()));
                        self.refresh_tree();
                        None
                    }
                    Err(e) => {
                        let mut d = dialog;
                        d.error = Some(format!("创建失败: {}", e));
                        Some(d)
                    }
                }
            }
            NameKind::Rename(ref old) => {
                let Some(parent) = old.parent().map(|p| p.to_path_buf()) else {
                    let mut d = dialog;
                    d.error = Some("无效路径".to_string());
                    return Some(d);
                };
                let target = parent.join(&name);
                match std::fs::rename(old, &target) {
                    Ok(()) => {
                        for tab in self.tabs.iter_mut() {
                            if tab.path == *old {
                                tab.path = target.clone();
                                tab.title = target
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| name.clone());
                            }
                        }
                        self.set_toast(format!("已重命名为: {}", name));
                        self.refresh_tree();
                        None
                    }
                    Err(e) => {
                        let mut d = dialog;
                        d.error = Some(format!("重命名失败: {}", e));
                        Some(d)
                    }
                }
            }
        }
    }

    pub fn draw_network_settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.network_dialog_open {
            return;
        }
        let mut open = self.network_dialog_open;
        let mut changed = false;

        egui::Window::new("🌐 网络与代理配置中心 (Network & Proxy Settings)")
            .open(&mut open)
            .resizable(false)
            .default_width(500.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("配置全局 HTTP / HTTPS / SOCKS5 代理，自动应用于 AI 副驾驶模型请求与 Cargo 构建下载")
                        .size(11.5)
                        .weak(),
                );
                ui.separator();

                let net = &mut self.settings.network;
                ui.horizontal(|ui| {
                    ui.label("代理模式:");
                    for (mode, label) in [
                        (ProxyMode::None, "直连 (无代理)"),
                        (ProxyMode::System, "跟随系统环境变量"),
                        (ProxyMode::Http, "HTTP/HTTPS 代理"),
                        (ProxyMode::Socks5, "SOCKS5 代理"),
                    ] {
                        if ui.radio_value(&mut net.proxy_mode, mode, label).changed() {
                            changed = true;
                        }
                    }
                });

                ui.add_space(6.0);
                egui::Grid::new("net_proxy_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("代理服务器地址:");
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut net.proxy_url)
                                    .desired_width(300.0)
                                    .hint_text("http://127.0.0.1:7890 或 socks5://127.0.0.1:1080"),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("代理用户名 (可选):");
                        if ui
                            .add(egui::TextEdit::singleline(&mut net.proxy_username).desired_width(200.0))
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("代理密码 (可选):");
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut net.proxy_password)
                                    .password(true)
                                    .desired_width(200.0),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("跳过代理 (NO_PROXY):");
                        if ui
                            .add(egui::TextEdit::singleline(&mut net.no_proxy).desired_width(300.0))
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("请求超时 (秒):");
                        let mut t = net.timeout_secs as i32;
                        if ui.add(egui::DragValue::new(&mut t).range(5..=300)).changed() {
                            net.timeout_secs = t as u64;
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("TLS 证书校验:");
                        if ui
                            .checkbox(&mut net.verify_tls, "验证 HTTPS SSL/TLS 证书 (推荐开启)")
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();
                    });
            });

        self.network_dialog_open = open;
        if changed {
            self.save_settings();
        }
    }
}

// ---------- App UI 渲染实现 (Splash, Menu Bar, Tree, Virtualized Editor, Panels) ----------

impl App {
    pub fn draw_splash(&mut self, ctx: &egui::Context) {
        if !self.splash_centered {
            self.splash_centered = true;
            center_window(ctx, SPLASH_SIZE);
        }
        ctx.request_repaint_after(Duration::from_millis(16));
        let elapsed = self.splash_start.elapsed();
        let ra_done = self.ra_gate.load(Ordering::Relaxed);
        let all_ready = ra_done && elapsed >= SPLASH_MIN_VISIBLE;
        let force = elapsed >= SPLASH_FORCE_DEADLINE;
        let total_secs = self.splash_duration.as_secs_f64();
        let frac = if ra_done {
            (elapsed.as_secs_f64() / total_secs).clamp(0.0, 1.0) as f32
        } else {
            ((elapsed.as_secs_f64() / SPLASH_FORCE_DEADLINE.as_secs_f64()) * 0.9)
                .clamp(0.0, 0.9) as f32
        };
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                {
                    let mut mesh = egui::Mesh::default();
                    let mid_y = (rect.top() + rect.bottom()) * 0.5;
                    let white_uv = egui::Pos2::ZERO;
                    let corners = [
                        (rect.left_top(), SPLASH_C1),
                        (rect.right_top(), SPLASH_C1),
                        (pos2(rect.left(), mid_y), SPLASH_C2),
                        (pos2(rect.right(), mid_y), SPLASH_C2),
                        (rect.left_bottom(), SPLASH_C3),
                        (rect.right_bottom(), SPLASH_C3),
                    ];
                    for (pos, color) in corners {
                        mesh.colored_vertex(pos, color);
                        if let Some(vertex) = mesh.vertices.last_mut() {
                            vertex.uv = white_uv;
                        }
                    }
                    mesh.indices
                        .extend_from_slice(&[0, 2, 1, 1, 2, 3, 2, 4, 3, 3, 4, 5]);
                    ui.painter().add(egui::Shape::mesh(mesh));
                }
                let time = ctx.input(|i| i.time) as f32;
                {
                    let painter = ui.painter();
                    for k in 0..SPLASH_FLOW_LINES {
                        let phase = (time * 0.12 + k as f32 / SPLASH_FLOW_LINES as f32) % 1.0;
                        let slope = -0.38 - k as f32 * 0.07;
                        let center_y = rect.top() + (phase * 1.4 - 0.2) * rect.height();
                        let intercept = center_y - slope * rect.width() * 0.5;
                        let p1 = pos2(rect.left(), intercept);
                        let p2 = pos2(rect.right(), intercept + slope * rect.width());
                        painter.line_segment(
                            [p1, p2],
                            Stroke::new(
                                2.0_f32,
                                Color32::from_rgba_unmultiplied(0xe9, 0x45, 0x60, 77),
                            ),
                        );
                    }
                }
                ui.add_space(28.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("🦀✨").size(48.0));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(APP_FULL_NAME)
                            .size(28.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(format!(
                            "{} · AI Copilot · MCP Server · VS Code Engine",
                            APP_VERSION
                        ))
                        .size(13.0)
                        .color(BRAND_RED),
                    );
                    ui.add_space(16.0);
                    let idx = ((elapsed.as_millis() / 450) % SPLASH_LOAD_TEXTS.len() as u128) as usize;
                    ui.label(
                        RichText::new(SPLASH_LOAD_TEXTS[idx])
                            .size(13.0)
                            .color(Color32::from_rgb(0xc8, 0xd3, 0xe8)),
                    );
                    let info = { toolchain_summary(&lock(&self.toolchain)) };
                    ui.label(RichText::new(info).size(11.0).weak());
                    let lsp_ready = self
                        .lsp
                        .as_ref()
                        .map(|s| s.ready.load(Ordering::Relaxed))
                        .unwrap_or(false);
                    let ra_text = if lsp_ready {
                        "rust-analyzer 智能补全与实时语法预检已就绪"
                    } else if ra_done {
                        "内置补全与 AST 实时检查引擎已就绪"
                    } else {
                        "正在启动 rust-analyzer 与 MCP 服务..."
                    };
                    ui.label(RichText::new(ra_text).size(11.0).weak());
                    ui.add_space(12.0);
                    let (bar_rect, _) =
                        ui.allocate_exact_size(Vec2::new(320.0, 4.0), Sense::hover());
                    let painter = ui.painter();
                    painter.rect_filled(
                        bar_rect,
                        2.0,
                        Color32::from_rgba_unmultiplied(255, 255, 255, 30),
                    );
                    let fill = Rect::from_min_size(bar_rect.min, Vec2::new(320.0 * frac, 4.0));
                    painter.rect_filled(fill, 2.0, BRAND_RED);
                });
                if all_ready || force {
                    self.phase = Phase::Main;
                    ctx.send_viewport_cmd(ViewportCommand::Decorations(true));
                    ctx.send_viewport_cmd(ViewportCommand::Resizable(true));
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(MAIN_SIZE));
                    ctx.send_viewport_cmd(ViewportCommand::Title(format!(
                        "{} {}",
                        APP_FULL_NAME, APP_VERSION
                    )));
                    center_window(ctx, MAIN_SIZE);
                }
            });
    }

    pub fn draw_menu_bar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        egui::menu::bar(ui, |ui| {
            // 1. 文件与项目管理菜单
            ui.menu_button("📁 文件", |ui| {
                if ui.button("🚀 新建 Rust 项目...").clicked() {
                    ui.close_menu();
                    self.menu_new_project();
                }
                ui.separator();
                if ui.button("📄 新建文件 (Ctrl+N)").clicked() {
                    ui.close_menu();
                    self.menu_new_file();
                }
                if ui.button("📂 新建文件夹").clicked() {
                    ui.close_menu();
                    self.menu_new_folder();
                }
                if ui.button("📦 打开项目文件夹 (Ctrl+O)").clicked() {
                    ui.close_menu();
                    self.menu_open_folder();
                }
                if !self.settings.recent_workspaces.is_empty() {
                    ui.menu_button("🕒 最近打开的项目", |ui| {
                        let recents = self.settings.recent_workspaces.clone();
                        for r in recents {
                            if ui.button(&r).clicked() {
                                ui.close_menu();
                                let p = PathBuf::from(&r);
                                if p.is_dir() {
                                    self.set_workspace(p);
                                } else {
                                    self.set_toast(format!("目录不存在: {}", r));
                                }
                            }
                        }
                    });
                }
                ui.separator();
                if ui.button("💾 保存 (Ctrl+S)").clicked() {
                    ui.close_menu();
                    self.save_active(ctx);
                }
                if ui.button("💾 全部保存 (Ctrl+Shift+S)").clicked() {
                    ui.close_menu();
                    self.save_all(ctx);
                }
                ui.separator();
                if ui.button("🚪 退出").clicked() {
                    ui.close_menu();
                    self.exit_app(ctx);
                }
            });

            // 2. 编辑菜单
            ui.menu_button("✏ 编辑", |ui| {
                if ui.button("↩ 撤销 (Ctrl+Z)").clicked() {
                    ui.close_menu();
                    self.edit_undo();
                }
                if ui.button("↪ 重做 (Ctrl+Y)").clicked() {
                    ui.close_menu();
                    self.edit_redo();
                }
                ui.separator();
                if ui.button("✂ 剪切 (Ctrl+X)").clicked() {
                    ui.close_menu();
                    self.edit_cut(ctx);
                }
                if ui.button("📋 复制 (Ctrl+C)").clicked() {
                    ui.close_menu();
                    self.edit_copy(ctx);
                }
                if ui.button("📌 粘贴 (Ctrl+V)").clicked() {
                    ui.close_menu();
                    self.edit_paste(ctx);
                }
                if ui.button("🔲 全选 (Ctrl+A)").clicked() {
                    ui.close_menu();
                    self.edit_select_all(ctx);
                }
            });

            // 3. 视图与多主题菜单
            ui.menu_button("🎨 视图与主题", |ui| {
                ui.menu_button("🌈 切换代码与界面主题 (8 款预设)", |ui| {
                    for &preset in ThemePreset::ALL {
                        let is_cur = self.settings.theme_preset == preset;
                        if ui
                            .selectable_label(is_cur, preset.display_name())
                            .clicked()
                        {
                            self.settings.theme_preset = preset;
                            self.settings.dark_mode = preset.is_dark();
                            preset.apply_to_context(ctx);
                            self.save_settings();
                            self.set_toast(format!("已切换主题: {}", preset.display_name()));
                            ui.close_menu();
                        }
                    }
                });
                ui.separator();
                if ui
                    .checkbox(&mut self.settings.show_indent_guides, "显示 VS Code 缩进参考线")
                    .changed()
                {
                    self.save_settings();
                }
                if ui
                    .checkbox(
                        &mut self.settings.show_active_indent_guide,
                        "高亮当前代码块激活缩进线",
                    )
                    .changed()
                {
                    self.save_settings();
                }
                if ui
                    .checkbox(
                        &mut self.settings.show_inline_diagnostics,
                        "显示 IDEA 风格行内报错波浪线与 Error Lens",
                    )
                    .changed()
                {
                    self.save_settings();
                }
                if ui
                    .checkbox(&mut self.settings.show_file_badges, "显示文件树彩色格式徽章")
                    .changed()
                {
                    self.save_settings();
                }
                ui.separator();
                ui.checkbox(&mut self.ai_sidebar.visible, "🤖 显示 AI 副驾驶侧边栏 (Ctrl+Shift+I)");
                ui.checkbox(&mut self.show_ast, "🌳 显示 AST 语法树侧边栏");
                ui.checkbox(&mut self.show_output, "📟 显示构建输出面板");
                if ui.button("🔄 刷新文件树").clicked() {
                    ui.close_menu();
                    self.refresh_tree();
                }
            });

            // 4. 运行与调试菜单
            ui.menu_button("▶ 编译与调试", |ui| {
                ui.label(RichText::new("构建配置 (Build Profile):").size(11.0).weak());
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.build_profile, BuildProfile::Debug, "Debug");
                    ui.radio_value(&mut self.build_profile, BuildProfile::Release, "Release");
                });
                ui.separator();
                if ui.button("▶ 运行项目 (F5 · cargo run)").clicked() {
                    ui.close_menu();
                    self.run_cargo(ctx, RunMode::Run);
                }
                if ui
                    .button("🐞 调试编译运行 (Shift+F5 · Backtrace + 断点追踪)")
                    .clicked()
                {
                    ui.close_menu();
                    self.run_cargo(ctx, RunMode::DebugRun);
                }
                if ui.button("🔨 仅编译构建 (cargo build)").clicked() {
                    ui.close_menu();
                    self.run_cargo(ctx, RunMode::Build);
                }
                if ui.button("🔍 快速类型与语法检查 (F7 · cargo check)").clicked() {
                    ui.close_menu();
                    self.run_cargo(ctx, RunMode::Check);
                }
                if ui.button("🧪 运行单元测试 (cargo test)").clicked() {
                    ui.close_menu();
                    self.run_cargo(ctx, RunMode::Test);
                }
                if ui.button("🧹 Clippy 代码静态审查 (cargo clippy)").clicked() {
                    ui.close_menu();
                    self.run_cargo(ctx, RunMode::Clippy);
                }
                if ui.button("✨ 格式化全部源码 (cargo fmt)").clicked() {
                    ui.close_menu();
                    self.run_cargo(ctx, RunMode::Fmt);
                }
                if ui.button("📚 生成项目文档 (cargo doc)").clicked() {
                    ui.close_menu();
                    self.run_cargo(ctx, RunMode::Doc);
                }
                ui.separator();
                if ui
                    .button(RichText::new("⏹ 终止当前运行进程").color(BRAND_RED))
                    .clicked()
                {
                    ui.close_menu();
                    self.terminate_cargo();
                }
            });

            // 5. 工程、依赖、环境与专业工具菜单
            ui.menu_button("🛠 环境与工程工具", |ui| {
                if ui
                    .button("🩺 一键环境检测与配置中心 (Rustup / 工具链 / 镜像源)...")
                    .clicked()
                {
                    ui.close_menu();
                    self.env_doctor_open = true;
                }
                if ui
                    .button("📦 依赖与版本管理中心 (Cargo.toml 可视化管理)...")
                    .clicked()
                {
                    ui.close_menu();
                    self.dep_manager.reload_from_workspace(self.workspace.as_deref());
                    self.dep_manager.open = true;
                }
                if ui
                    .button("🧩 插件生态与拓展接口管理 (Extensions & Hooks)...")
                    .clicked()
                {
                    ui.close_menu();
                    self.plugin_manager.open = true;
                }
                ui.separator();
                if ui
                    .button("🔌 MCP 协议服务设置 (Model Context Protocol Server)...")
                    .clicked()
                {
                    ui.close_menu();
                    self.mcp_server.dialog_open = true;
                }
                if ui.button("🌐 网络与代理服务器设置 (Proxy & TLS)...").clicked() {
                    ui.close_menu();
                    self.network_dialog_open = true;
                }
            });

            // 6. AI 副驾驶菜单
            ui.menu_button("🤖 AI 副驾驶", |ui| {
                if ui.button("💬 打开/切换 AI Agent 侧边栏").clicked() {
                    ui.close_menu();
                    self.ai_sidebar.visible = !self.ai_sidebar.visible;
                }
                if ui.button("⚙ 模型与多协议接入配置 (OpenAI/Claude/Gemini/Ollama)...").clicked() {
                    ui.close_menu();
                    self.ai_sidebar.model_dialog.open = true;
                }
                if ui.button("➕ 新建 AI 会话").clicked() {
                    ui.close_menu();
                    self.ai_sidebar.visible = true;
                    let id = self.ai_sidebar.next_session_id;
                    self.ai_sidebar.next_session_id += 1;
                    self.ai_sidebar.sessions.push(crate::ai::sessions::ChatSession::new(
                        id,
                        format!("对话 #{}", id),
                        self.workspace.clone(),
                    ));
                    self.ai_sidebar.active_session_idx = self.ai_sidebar.sessions.len() - 1;
                }
            });

            // 7. 终端与搜索菜单
            ui.menu_button("🖥 终端", |ui| {
                if ui.button("⚡ 打开/切换集成终端 (Ctrl+`)").clicked() {
                    ui.close_menu();
                    self.toggle_terminal(ctx);
                }
                if ui.button("＋ 新建终端会话标签页").clicked() {
                    ui.close_menu();
                    let ws = self.workspace.clone();
                    self.terminal.new_tab(ws.as_deref(), ctx);
                }
            });

            ui.menu_button("🔍 搜索", |ui| {
                if ui.button("在当前文件中查找 (Ctrl+F)").clicked() {
                    ui.close_menu();
                    if self.find.is_none() {
                        self.find = Some(FindState {
                            input: String::new(),
                            replace_input: String::new(),
                            show_replace: false,
                            total: 0,
                            index: 0,
                        });
                    }
                }
                if ui.button("在全项目中搜索 (Ctrl+Shift+F)").clicked() {
                    ui.close_menu();
                    if self.search.is_none() {
                        self.search = Some(SearchState {
                            input: String::new(),
                            case_sensitive: false,
                            running: Arc::new(AtomicBool::new(false)),
                            results: Arc::new(Mutex::new(Vec::new())),
                        });
                    }
                }
            });

            ui.menu_button("❓ 帮助", |ui| {
                if ui.button("关于 NovaRS v2.0").clicked() {
                    ui.close_menu();
                    self.about_open = true;
                }
            });

            // 右侧快捷工具条按钮（一键运行、调试、依赖、环境、AI）
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .button(
                        RichText::new(if self.ai_sidebar.visible {
                            "🤖 AI 副驾驶 ▾"
                        } else {
                            "🤖 开启 AI"
                        })
                        .color(BRAND_ACCENT)
                        .strong(),
                    )
                    .clicked()
                {
                    self.ai_sidebar.visible = !self.ai_sidebar.visible;
                }
                if ui
                    .small_button("🔌 MCP")
                    .on_hover_text("管理内置 MCP 服务与外部 Agent 连接")
                    .clicked()
                {
                    self.mcp_server.dialog_open = true;
                }
                if ui
                    .small_button("📦 依赖")
                    .on_hover_text("打开 Cargo 依赖与版本管理中心")
                    .clicked()
                {
                    self.dep_manager.reload_from_workspace(self.workspace.as_deref());
                    self.dep_manager.open = true;
                }
                if ui
                    .small_button("🩺 环境")
                    .on_hover_text("一键检测与配置 Rust 环境/镜像源")
                    .clicked()
                {
                    self.env_doctor_open = true;
                }
                if ui
                    .small_button(RichText::new("🐞 调试").color(BRAND_WARN))
                    .on_hover_text("以调试追踪模式编译并运行 (RUST_BACKTRACE=full)")
                    .clicked()
                {
                    self.run_cargo(ctx, RunMode::DebugRun);
                }
                if ui
                    .small_button(RichText::new("▶ 运行").color(BRAND_GREEN).strong())
                    .on_hover_text("运行当前项目 (F5)")
                    .clicked()
                {
                    self.run_cargo(ctx, RunMode::Run);
                }
            });
        });
    }

    // ---------- 绘制：文件树（带专属彩色格式徽章与图标） ----------

    pub fn draw_tree(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("🗂 资源管理器").strong());
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("🔄").on_hover_text("刷新文件树").clicked() {
                    self.refresh_tree();
                    self.set_toast("文件树已刷新");
                }
                if ui.small_button("＋文件").on_hover_text("新建文件").clicked() {
                    self.menu_new_file();
                }
            });
        });
        ui.separator();
        if self.workspace.is_none() {
            ui.label("未打开项目文件夹");
            if ui.button("📂 打开文件夹...").clicked() {
                self.menu_open_folder();
            }
            if ui.button("🚀 新建 Rust 项目...").clicked() {
                self.menu_new_project();
            }
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let root_children = self.tree_root.clone();
                for idx in root_children {
                    self.draw_node(ui, idx, 0);
                }
            });
    }

    pub fn tree_file_color(&self, path: &Path) -> Option<Color32> {
        if self
            .error_files
            .lock()
            .map(|s| s.contains(path))
            .unwrap_or(false)
        {
            return Some(BRAND_RED);
        }
        if let Some(tab) = self.tabs.iter().find(|t| t.path == path) {
            if tab.diagnostics.iter().any(|d| d.severity == DiagSeverity::Error) {
                return Some(BRAND_RED);
            }
            if tab.dirty {
                return Some(BRAND_WARN);
            }
            return Some(BRAND_GREEN);
        }
        None
    }

    pub fn draw_node(&mut self, ui: &mut egui::Ui, idx: usize, depth: usize) {
        let (name, path, is_dir) = {
            let node = &self.nodes[idx];
            (node.name.clone(), node.path.clone(), node.is_dir)
        };
        if is_dir {
            let folder_spec = icon_for_folder(&name, depth == 0);
            let header = egui::CollapsingHeader::new(
                RichText::new(format!("{} {}", folder_spec.glyph, name)).color(folder_spec.color),
            )
            .default_open(depth == 0)
            .id_salt(format!("tree-node:{}", path.display()));
            let resp = header.show(ui, |ui| {
                let children = self.nodes[idx].children.clone();
                if children.is_empty() {
                    ui.weak("（空文件夹）");
                }
                for child in children {
                    self.draw_node(ui, child, depth + 1);
                }
            });
            resp.header_response.context_menu(|ui| {
                if ui.button("新建文件").clicked() {
                    ui.close_menu();
                    self.menu_new_file();
                }
                if ui.button("新建文件夹").clicked() {
                    ui.close_menu();
                    self.name_dialog = Some(NameDialog {
                        title: "新建文件夹".to_string(),
                        hint: format!("在 {} 下创建新文件夹，请输入名称：", path.display()),
                        value: "新建文件夹".to_string(),
                        error: None,
                        kind: NameKind::NewFolder(path.clone()),
                    });
                }
                if depth > 0 {
                    if ui.button("重命名").clicked() {
                        ui.close_menu();
                        self.name_dialog = Some(NameDialog {
                            title: "重命名".to_string(),
                            hint: format!("将 “{}” 重命名为：", name),
                            value: name.clone(),
                            error: None,
                            kind: NameKind::Rename(path.clone()),
                        });
                    }
                    if ui.button("删除").clicked() {
                        ui.close_menu();
                        self.delete_path(path.clone());
                    }
                }
                ui.separator();
                if ui.button("在文件管理器中显示").clicked() {
                    ui.close_menu();
                    reveal_in_explorer(&path);
                }
                if ui.button("复制路径").clicked() {
                    ui.close_menu();
                    ui.output_mut(|o| o.copied_text = path.display().to_string());
                    self.set_toast(format!("已复制路径: {}", path.display()));
                }
            });
        } else {
            let selected = self
                .active_tab
                .map(|i| self.tabs[i].path == path)
                .unwrap_or(false);
            let status_color = self.tree_file_color(&path);
            let (spec, label_text) = rich_file_label(&path, &name, status_color);

            let resp = ui
                .horizontal(|ui| {
                    if self.settings.show_file_badges {
                        draw_file_badge(ui, &spec);
                    }
                    ui.selectable_label(selected, label_text)
                })
                .inner;

            if resp.clicked() {
                self.open_file_in_tab(path.clone());
            }
            resp.context_menu(|ui| {
                if ui.button("打开").clicked() {
                    ui.close_menu();
                    self.open_file_in_tab(path.clone());
                }
                if ui.button("重命名").clicked() {
                    ui.close_menu();
                    self.name_dialog = Some(NameDialog {
                        title: "重命名".to_string(),
                        hint: format!("将 “{}” 重命名为：", name),
                        value: name.clone(),
                        error: None,
                        kind: NameKind::Rename(path.clone()),
                    });
                }
                if ui.button("删除").clicked() {
                    ui.close_menu();
                    self.delete_path(path.clone());
                }
                ui.separator();
                if ui.button("在文件管理器中显示").clicked() {
                    ui.close_menu();
                    reveal_in_explorer(&path);
                }
                if ui.button("复制路径").clicked() {
                    ui.close_menu();
                    ui.output_mut(|o| o.copied_text = path.display().to_string());
                    self.set_toast(format!("已复制路径: {}", path.display()));
                }
            });
        }
    }

    // ---------- 绘制：标签栏与带大文件不截断虚拟化的智能编辑器 ----------

    pub fn draw_tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let infos: Vec<(usize, String, PathBuf, bool, bool, usize)> = self
                .tabs
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let err_cnt = t
                        .diagnostics
                        .iter()
                        .filter(|d| d.severity == DiagSeverity::Error)
                        .count();
                    (
                        i,
                        t.title.clone(),
                        t.path.clone(),
                        t.dirty,
                        self.active_tab == Some(i),
                        err_cnt,
                    )
                })
                .collect();
            for (i, title, path, dirty, active, err_cnt) in infos {
                let spec = icon_for_path(&path);
                let mut label = format!("{} {}", spec.glyph, title);
                if dirty {
                    label.push('*');
                }
                if err_cnt > 0 {
                    label.push_str(&format!(" ({})", err_cnt));
                }
                let text = if err_cnt > 0 {
                    RichText::new(label).color(BRAND_RED)
                } else {
                    RichText::new(label)
                };
                let resp = ui.selectable_label(active, text);
                if resp.clicked() {
                    self.active_tab = Some(i);
                    self.last_cursor_range = None;
                    self.ast_sig = None;
                    self.completion = None;
                    self.match_highlight = None;
                }
                if resp.hovered() {
                    resp.on_hover_text(format!("{} ({})", path.display(), spec.lang_name));
                }
                if ui.small_button("✕").clicked() {
                    self.request_close_tab(i);
                }
            }
        });
        ui.separator();
    }

    pub fn draw_editor_area(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let Some(ti) = self.active_tab else { return };

        // 1. 顶部绘制 IDEA 风格实时代码检查与 AI 快速修复状态栏 + 补丁接受/撤回横幅
        let (diags_snapshot, mut sel_diag) = {
            let tab = &self.tabs[ti];
            (tab.diagnostics.clone(), tab.selected_diagnostic)
        };
        let diag_act = draw_diagnostic_inspector_bar(
            ui,
            &diags_snapshot,
            &mut sel_diag,
            self.pending_fix_patch.as_ref(),
        );
        self.tabs[ti].selected_diagnostic = sel_diag;

        if diag_act.accept_pending_patch {
            self.accept_pending_patch(ctx);
        }
        if diag_act.revert_pending_patch {
            self.revert_pending_patch();
        }
        if let Some(line) = diag_act.jump_to_line {
            self.jump_to_line(ctx, line);
        }
        if let Some(instant_diag) = diag_act.apply_instant_fix {
            if let Some((s_line, e_line, ref rep)) = instant_diag.suggested_replacement {
                self.apply_ai_or_quick_patch(ctx, s_line, e_line, rep, &instant_diag.message);
            }
        }
        if let Some(ai_diag) = diag_act.ask_ai_fix {
            let file_name = self.tabs[ti].title.clone();
            let file_text = self.tabs[ti].text.clone();
            let ws = self.workspace.clone();
            self.ai_sidebar.trigger_fix_diagnostic(
                ctx,
                &ai_diag,
                &file_name,
                &file_text,
                &self.settings.models,
                &self.settings.active_model_id,
                &self.settings.network,
                self.settings.thinking_effort_level,
                self.settings.sandbox_tier,
                self.mcp_server.builtin_sidebar_connected,
                ws.as_deref(),
            );
        }

        // 2. 超大文件（>2500 行，含 4000~5000+ 行至 100,000+ 行）滑动视口控制器
        // 彻底根治 egui 单 Galley 超过 4000 行后 GPU 纹理与网格顶点被截断导致行号和文字消失的顽疾！
        let total_lines = count_lines(&self.tabs[ti].text);
        let is_large_file = total_lines > LARGE_FILE_LINE_THRESHOLD;
        if is_large_file {
            let max_start = total_lines.saturating_sub(VIRTUAL_CHUNK_LINES);
            let mut v_start = self.tabs[ti].virtual_start_line.min(max_start);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "⚡ 大文件无损渲染模式 (总计 {} 行 · 当前视窗 L{} - L{})",
                        total_lines,
                        v_start + 1,
                        (v_start + VIRTUAL_CHUNK_LINES).min(total_lines)
                    ))
                    .size(11.5)
                    .color(BRAND_ACCENT)
                    .strong(),
                );
                if ui.small_button("⏮ 首段 (L1)").clicked() {
                    v_start = 0;
                    self.scroll_to_offset = Some(0.0);
                }
                if ui.small_button("◀ 上一页 (-1000行)").clicked() {
                    v_start = v_start.saturating_sub(1000);
                    self.scroll_to_offset = Some(0.0);
                }
                if ui
                    .add(
                        egui::Slider::new(&mut v_start, 0..=max_start)
                            .prefix("起始行 L")
                            .step_by(100.0),
                    )
                    .changed()
                {
                    self.scroll_to_offset = Some(0.0);
                }
                if ui.small_button("下一页 (+1000行) ▶").clicked() {
                    v_start = (v_start + 1000).min(max_start);
                    self.scroll_to_offset = Some(0.0);
                }
                if ui
                    .small_button(format!("⏭ 末段 (L{})", total_lines))
                    .clicked()
                {
                    v_start = max_start;
                    self.scroll_to_offset = Some(0.0);
                }
            });
            self.tabs[ti].virtual_start_line = v_start;
        } else {
            self.tabs[ti].virtual_start_line = 0;
        }

        // 3. 横向并排：左侧动态宽度不截断行号/断点/诊断栏 + 右侧代码编辑区
        let theme = CodeTheme::from_preset(self.settings.theme_preset);
        let row_h = ctx.fonts(|f| f.row_height(&FontId::monospace(MONO_SIZE)));
        let char_w = ctx.fonts(|f| {
            f.layout_no_wrap("M".to_string(), FontId::monospace(MONO_SIZE), Color32::WHITE)
                .size()
                .x
        });

        let tab_id = self.tabs[ti].id;
        let base_line_0based = self.tabs[ti].virtual_start_line;
        let visible_window_lines = if is_large_file {
            (total_lines - base_line_0based).min(VIRTUAL_CHUNK_LINES)
        } else {
            total_lines
        };

        let scroll_off_y = self
            .editor_scroll_meta
            .filter(|m| m.0 as u64 == tab_id)
            .map(|m| m.1)
            .unwrap_or(0.0);

        let breakpoints = self.tabs[ti].breakpoints.clone();
        let line_diag_map = build_line_severity_map(&self.tabs[ti].diagnostics);

        ui.horizontal(|ui| {
            let gutter_out = draw_gutter(
                ctx,
                ui,
                &GutterRenderParams {
                    total_lines,
                    base_line_0based,
                    visible_line_count: visible_window_lines,
                    scroll_offset_y: scroll_off_y,
                    row_h,
                    char_w,
                    cursor_line_1based: self.cursor_line,
                    breakpoints: &breakpoints,
                    line_diagnostics: &line_diag_map,
                    theme: &theme,
                },
            );

            if let Some(bp_line) = gutter_out.toggled_breakpoint_line {
                let tab = &mut self.tabs[ti];
                if !tab.breakpoints.remove(&bp_line) {
                    tab.breakpoints.insert(bp_line);
                }
                self.set_toast(format!("🐞 断点切换: 行 {}", bp_line));
            }
            if let Some(diag_line) = gutter_out.clicked_diagnostic_line {
                if let Some(pos) = self.tabs[ti]
                    .diagnostics
                    .iter()
                    .position(|d| d.line == diag_line)
                {
                    self.tabs[ti].selected_diagnostic = Some(pos);
                }
            }

            self.draw_editor_core(ctx, ui, ti, &theme, row_h, char_w, total_lines, base_line_0based);
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_editor_core(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        ti: usize,
        theme: &CodeTheme,
        row_h: f32,
        char_w: f32,
        total_lines: usize,
        base_line_0based: usize,
    ) {
        let mut swallow_enter = false;
        if self.ime_composing {
            swallow_enter = true;
        } else if let Some(t) = self.ime_commit_at {
            if t.elapsed() < Duration::from_millis(150) {
                swallow_enter = true;
            } else {
                self.ime_commit_at = None;
            }
        }
        if swallow_enter {
            ctx.input_mut(|i| {
                i.events.retain(|e| {
                    !matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, .. })
                })
            });
        }

        self.track_undo(ti);
        if self.auto_complete_pending {
            self.auto_complete_pending = false;
            self.open_completion(false);
        }
        if self.force_completion {
            self.force_completion = false;
            self.open_completion(true);
        }

        let tab_id = self.tabs[ti].id;
        let full_text = std::mem::take(&mut self.tabs[ti].text);
        let is_windowed = total_lines > LARGE_FILE_LINE_THRESHOLD;

        // 提取当前渲染窗口切片（若为普通文件则直接零拷贝使用全文；若为超大文件则提取当前 1200 行窗口并在编辑后无缝拼接）
        let (win_start_byte, win_end_byte, mut window_text) = if is_windowed {
            let sb = line_start_byte_index(&full_text, base_line_0based + 1);
            let eb = line_start_byte_index(&full_text, base_line_0based + VIRTUAL_CHUNK_LINES + 1);
            let slice = full_text[sb..eb].trim_end_matches('\n').to_string();
            (sb, eb, slice)
        } else {
            (0, full_text.len(), full_text.clone())
        };

        let base_char_offset = if is_windowed {
            full_text[..win_start_byte].chars().count()
        } else {
            0
        };

        let window_line_count = count_lines(&window_text);
        let (_, num_w, _) = compute_gutter_width(total_lines, char_w);
        let max_line_chars = window_text
            .lines()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0);
        let edit_avail_w = (ui.available_width() - 12.0).max(300.0);
        let needed_w = (max_line_chars as f32 * char_w + 24.0 * char_w)
            .max(edit_avail_w - num_w)
            .min(20_000.0);

        let avail_h = ui.available_height();
        let avail_h = if avail_h.is_finite() && avail_h > 120.0 {
            avail_h
        } else {
            600.0
        };
        let content_h = (window_line_count as f32 * row_h + row_h * 2.0).max(avail_h - 4.0);

        let editor_id = Self::editor_id_of(tab_id);
        let scroll_target = self.scroll_to_line;
        let pending_offset = self.scroll_to_offset.take();
        let pending_selection = self.pending_selection.take();
        let mut consumed_scroll = false;
        let mut popup_anchor_local: Option<egui::Pos2> = None;

        let show_indent_guides = self.settings.show_indent_guides;
        let show_active_guide = self.settings.show_active_indent_guide;
        let show_inline_diags = self.settings.show_inline_diagnostics;
        let tab_diags = self.tabs[ti].diagnostics.clone();

        let mut area = egui::ScrollArea::both().auto_shrink([false, false]);
        if let Some(offset) = pending_offset {
            area = area.vertical_scroll_offset(offset);
        }

        let scroll_out = area.show(ui, |ui| {
            let mut layouter =
                |_ui: &egui::Ui, content: &str, _wrap_width: f32| -> Arc<Galley> {
                    let mut job = LayoutJob::default();
                    highlight_rust(content, theme, &mut job);
                    ctx.fonts(|f| f.layout_job(job))
                };

            let out = egui::TextEdit::multiline(&mut window_text)
                .id(editor_id)
                .font(FontSelection::FontId(FontId::monospace(MONO_SIZE)))
                .frame(false)
                .desired_width(needed_w)
                .min_size(Vec2::new(needed_w, content_h))
                .lock_focus(true)
                .layouter(&mut layouter)
                .show(ui);

            if let Some(range) = out.cursor_range {
                let p_local = range.primary.ccursor.index;
                let s_local = range.secondary.ccursor.index;
                let (l_local, c_col) = line_col_of_char_index(&window_text, p_local);
                self.cursor_line = base_line_0based + l_local;
                self.cursor_col = c_col;
                self.last_cursor_range =
                    Some((tab_id, (base_char_offset + p_local, base_char_offset + s_local)));
            }

            if let Some((tid, s_global, e_global)) = pending_selection {
                if tid == tab_id {
                    let s_loc = s_global.saturating_sub(base_char_offset);
                    let e_loc = e_global.saturating_sub(base_char_offset);
                    let mut st = out.state.clone();
                    st.cursor.set_char_range(Some(CCursorRange::two(
                        CCursor::new(s_loc.min(e_loc)),
                        CCursor::new(s_loc.max(e_loc)),
                    )));
                    st.store(ctx, editor_id);
                }
            }

            if let Some(range) = out.cursor_range {
                let local = out.galley.pos_from_ccursor(range.primary.ccursor);
                popup_anchor_local = Some(egui::pos2(
                    local.max.x + out.galley_pos.x + 6.0,
                    local.min.y + out.galley_pos.y - 2.0,
                ));
            }

            // 计算可视行区间
            let meta = self.editor_scroll_meta;
            let first_vis = meta
                .map(|m| ((m.1 / row_h) as usize).saturating_sub(3))
                .unwrap_or(0);
            let last_vis = meta
                .map(|m| ((m.1 + m.3) / row_h) as usize + 4)
                .unwrap_or(window_line_count)
                .min(window_line_count);

            let win_lines: Vec<&str> = window_text.lines().collect();

            // 1) VS Code 风格缩进辅助线（含空白行前后插值 + 激活代码块作用域高亮）
            if show_indent_guides {
                let cursor_local = if self.cursor_line > base_line_0based {
                    Some(self.cursor_line - base_line_0based - 1)
                } else {
                    None
                };
                draw_vscode_indent_guides(
                    ui,
                    &win_lines,
                    first_vis,
                    last_vis,
                    cursor_local,
                    out.galley_pos,
                    row_h,
                    char_w,
                    theme,
                    show_active_guide,
                );
            }

            // 2) IDEA 风格行内波浪线与行尾 Error Lens 诊断渲染
            if show_inline_diags && !tab_diags.is_empty() {
                draw_inline_diagnostics(
                    ui,
                    &tab_diags,
                    base_line_0based,
                    first_vis,
                    last_vis,
                    &win_lines,
                    out.galley_pos,
                    row_h,
                    char_w,
                    theme,
                );
            }

            // 3) 查找/搜索匹配高亮
            if let Some((hid, hs_global, he_global)) = self.match_highlight {
                if hid == tab_id && he_global > hs_global && hs_global >= base_char_offset {
                    let hs = hs_global - base_char_offset;
                    let he = he_global - base_char_offset;
                    if he <= window_text.chars().count() {
                        let r0 = out.galley.pos_from_ccursor(CCursor::new(hs));
                        let r1 = out.galley.pos_from_ccursor(CCursor::new(he));
                        let local = r0.union(r1);
                        let highlight_rect = Rect::from_min_size(
                            pos2(
                                local.min.x + out.galley_pos.x,
                                local.min.y + out.galley_pos.y,
                            ),
                            Vec2::new(local.width().max(char_w), local.height()),
                        );
                        ui.painter().rect_filled(
                            highlight_rect,
                            2.0,
                            Color32::from_rgba_unmultiplied(0xff, 0xd5, 0x4f, 76),
                        );
                    }
                }
            }

            // 4) 行号跳转滚动
            if let Some((tid, target_global_line)) = scroll_target {
                if tid == tab_id && target_global_line > base_line_0based {
                    let local_line = target_global_line - base_line_0based;
                    let y = out.galley_pos.y + (local_line as f32 - 1.0) * row_h;
                    let target_rect =
                        Rect::from_min_size(pos2(out.galley_pos.x, y), Vec2::new(4.0, row_h));
                    ui.allocate_rect(target_rect, Sense::hover())
                        .scroll_to_me(Some(Align::Center));
                    consumed_scroll = true;
                }
            }
        });

        self.editor_scroll_meta = Some((
            tab_id as f32,
            scroll_out.state.offset.y,
            scroll_out.content_size.y.max(1.0),
            scroll_out.inner_rect.height(),
        ));

        // 写回完整文本
        let updated_full_text = if is_windowed {
            let mut rebuilt = String::with_capacity(full_text.len() + 64);
            rebuilt.push_str(&full_text[..win_start_byte]);
            rebuilt.push_str(&window_text);
            if win_end_byte < full_text.len() {
                rebuilt.push('\n');
                rebuilt.push_str(&full_text[win_end_byte..]);
            } else if full_text.ends_with('\n') && !rebuilt.ends_with('\n') {
                rebuilt.push('\n');
            }
            rebuilt
        } else {
            window_text
        };

        let text_hash = {
            let mut hasher = DefaultHasher::new();
            updated_full_text.hash(&mut hasher);
            hasher.finish()
        };
        if self.minimap_cache.as_ref().map(|(h, _)| *h) != Some(text_hash) {
            self.minimap_cache = Some((text_hash, build_minimap(&updated_full_text)));
        }

        self.tabs[ti].text = updated_full_text;
        if consumed_scroll {
            self.scroll_to_line = None;
        }
        self.completion_anchor = popup_anchor_local;
        if let Some(cmd) = self.popup_cmd.take() {
            match cmd {
                0 => {
                    if let Some(p) = &mut self.completion {
                        if !p.items.is_empty() {
                            p.selected = (p.selected + 1) % p.items.len();
                        }
                    }
                }
                1 => {
                    if let Some(p) = &mut self.completion {
                        if !p.items.is_empty() {
                            p.selected = (p.selected + p.items.len() - 1) % p.items.len();
                        }
                    }
                }
                2 => self.apply_completion(ctx),
                _ => self.completion = None,
            }
        }
    }

    pub fn draw_welcome(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(70.0);
            ui.label(RichText::new("🦀✨").size(56.0));
            ui.add_space(6.0);
            ui.heading(RichText::new(format!("{} {}", APP_FULL_NAME, APP_VERSION)).strong());
            ui.label(
                RichText::new(
                    "纯 Rust 原生模块化 IDE · AI Agent 副驾驶 · 内置 MCP 服务 · IDEA 级实时报错预检 · 大文件无截断引擎",
                )
                .color(BRAND_ACCENT),
            );
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                let pad = (ui.available_width() - 420.0).max(0.0) * 0.5;
                ui.add_space(pad);
                if ui
                    .button(RichText::new("📂 打开项目文件夹...").size(14.5))
                    .clicked()
                {
                    self.menu_open_folder();
                }
                if ui
                    .button(RichText::new("🚀 新建 Rust 项目...").size(14.5))
                    .clicked()
                {
                    self.menu_new_project();
                }
                if ui
                    .button(RichText::new("🩺 环境一键检测与配置").size(14.5))
                    .clicked()
                {
                    self.env_doctor_open = true;
                }
            });
            ui.add_space(12.0);
            ui.label(
                RichText::new(
                    "快捷键：F5 运行 · Shift+F5 调试运行 · F7 检查 · Ctrl+` 终端 · Ctrl+Shift+I 切换 AI 副驾驶",
                )
                .weak(),
            );
        });
    }

    // ---------- AST 侧栏 / MiniMap / 状态栏 / 搜索弹窗 ----------

    pub fn draw_ast_panel(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("🌳 AST 语法结构").strong());
        });
        ui.separator();
        let Some(ti) = self.active_tab else {
            ui.weak("未打开文件");
            return;
        };
        let (tab_id, is_rs, text) = {
            let tab = &self.tabs[ti];
            (
                tab.id,
                tab.path.extension().map(|e| e == "rs").unwrap_or(false),
                tab.text.clone(),
            )
        };
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let sig = Some((tab_id, hasher.finish()));
        if self.ast_sig != sig {
            self.ast_sig = sig;
            if is_rs {
                match syn::parse_file(&text) {
                    Ok(file) => {
                        let mut items = Vec::new();
                        collect_ast_items(&file.items, 0, &mut items);
                        self.ast_items = items;
                        self.ast_error = None;
                    }
                    Err(e) => {
                        self.ast_items.clear();
                        self.ast_error = Some(format!("AST 语法错误 (L{}): {}", e.span().start().line, e));
                    }
                }
            } else {
                self.ast_items.clear();
                self.ast_error = Some("仅解析 Rust 源文件（.rs）".to_string());
            }
        }
        if let Some(err) = &self.ast_error {
            ui.label(RichText::new(err).small().color(BRAND_RED));
            ui.separator();
        }
        if self.ast_items.is_empty() {
            ui.weak("没有可显示的符号结构");
            return;
        }
        let items = self.ast_items.clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for item in items {
                    let indent = "  ".repeat(item.depth.min(6));
                    let label =
                        format!("{}{:>6}  {}  · L{}", indent, item.kind, item.name, item.line);
                    let resp = ui.selectable_label(
                        false,
                        RichText::new(label)
                            .monospace()
                            .size(12.0)
                            .color(ast_kind_color(item.kind, self.settings.dark_mode)),
                    );
                    if resp.clicked() {
                        self.jump_to_line(ctx, item.line);
                    }
                }
            });
    }

    pub fn draw_minimap_panel(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let dark = self.settings.dark_mode;
        let mm_bg = if dark {
            Color32::from_rgb(0x18, 0x1a, 0x20)
        } else {
            Color32::from_rgb(0xe8, 0xea, 0xed)
        };
        let mp = ui.painter_at(rect);
        mp.rect_filled(rect, 0.0, mm_bg);
        let active_id = self.active_tab.map(|i| self.tabs[i].id);
        let Some(meta) = self
            .editor_scroll_meta
            .filter(|m| active_id == Some(m.0 as u64))
        else {
            return;
        };
        let (_, offset_y, content_h_real, viewport_h) = meta;
        let rows = self
            .minimap_cache
            .as_ref()
            .map(|(_, r)| r.clone())
            .unwrap_or_default();
        let total_mm = (rows.len() as f32 * 2.0).max(1.0);
        let scale = (rect.height() / total_mm).min(1.0);
        for (i, row) in rows.iter().enumerate() {
            let y = rect.top() + i as f32 * 2.0 * scale;
            if y > rect.bottom() {
                break;
            }
            let x = rect.left() + 3.0 + row.indent * 0.5;
            let w = row.width.min(rect.right() - x - 1.0);
            if w > 0.5 {
                mp.rect_filled(
                    Rect::from_min_size(pos2(x, y), Vec2::new(w, 1.6)),
                    0.0,
                    row.color,
                );
            }
        }
        if content_h_real > 1.0 && rect.height() > 4.0 {
            let view_h = (viewport_h / content_h_real * rect.height()).clamp(10.0, rect.height());
            let view_y = (rect.top() + offset_y / content_h_real * rect.height())
                .clamp(rect.top(), rect.bottom() - view_h);
            let view_rect =
                Rect::from_min_max(pos2(rect.left(), view_y), pos2(rect.right(), view_y + view_h));
            let cover = (view_h / rect.height()).clamp(0.0, 1.0);
            if cover < 0.97 {
                let alpha = (70.0 * (1.0 - cover) + 6.0) as u8;
                mp.rect_filled(
                    view_rect,
                    2.0,
                    Color32::from_rgba_unmultiplied(0xe9, 0x45, 0x60, alpha),
                );
            }
            mp.rect_stroke(
                view_rect,
                2.0,
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0xe9, 0x45, 0x60, 120)),
            );
        }
        let resp = ui.interact(rect, Id::new("minimap_interact"), Sense::click_and_drag());
        if resp.dragged() || resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let frac = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
                if let Some(ti) = self.active_tab {
                    let total = count_lines(&self.tabs[ti].text);
                    if total > LARGE_FILE_LINE_THRESHOLD {
                        let target_line = ((frac * total as f32) as usize).clamp(1, total);
                        self.tabs[ti].virtual_start_line =
                            target_line.saturating_sub(VIRTUAL_CHUNK_LINES / 2);
                    }
                }
                self.scroll_to_offset = Some((frac * content_h_real - viewport_h * 0.5).max(0.0));
            }
        }
    }

    pub fn draw_status_bar(&mut self, ui: &mut egui::Ui) {
        let left_text = match self.active_tab {
            Some(i) => self.tabs[i].path.display().to_string(),
            None => match &self.workspace {
                Some(w) => format!("工作区: {}", w.display()),
                None => "未打开文件".to_string(),
            },
        };
        let total_lines = self
            .active_tab
            .map(|i| count_lines(&self.tabs[i].text))
            .unwrap_or(0);
        let mid_text = format!(
            "行 {}/{}, 列 {} · 主题: {}",
            self.cursor_line,
            total_lines,
            self.cursor_col,
            self.settings.theme_preset.display_name()
        );
        let (lang, eol) = match self.active_tab {
            Some(i) => {
                let tab = &self.tabs[i];
                (lang_of_path(&tab.path), tab.line_ending)
            }
            None => ("—", "—"),
        };
        let git_text = self
            .git_branch
            .clone()
            .map(|b| format!("🌿 {}", b))
            .unwrap_or_default();
        let mcp_text = if self.mcp_server.running.load(Ordering::Relaxed) {
            format!("🔌 MCP:{}", self.mcp_server.port)
        } else {
            "MCP:Off".to_string()
        };
        let info_text = format!("{} · UTF-8 · {} · {} · {}", lang, eol, git_text, mcp_text);
        let right_text = {
            let info = lock(&self.toolchain).clone();
            let rust_label = match info.rustc_short {
                Some(v) => format!("Rust {}", v),
                None => "Rust 未检测".to_string(),
            };
            format!("{} | {}", APP_NAME_WITH_VERSION, rust_label)
        };
        ui.columns(4, |cols| {
            cols[0].with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(left_text).size(11.5));
            });
            cols[1].with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(mid_text).size(11.5).weak());
            });
            cols[2].with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(info_text).size(11.5).weak());
            });
            cols[3].with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(right_text).size(11.5).weak());
            });
        });
    }

    pub fn draw_output_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("📟 构建与调试输出").strong());
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("关闭").clicked() {
                    self.show_output = false;
                }
                if ui.small_button("清空").clicked() {
                    lock(&self.output_log).clear();
                }
            });
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let log = lock(&self.output_log).clone();
                ui.add(egui::Label::new(RichText::new(log).monospace().size(12.0)).selectable(true));
            });
    }

    pub fn draw_project_dialog(&mut self, ctx: &egui::Context) {
        let mut dialog = match self.project_dialog.take() {
            Some(d) => d,
            None => return,
        };
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            return;
        }
        let mut cancelled = false;
        let mut confirmed = false;
        let mut change_location = false;
        let mut win_open = true;
        egui::Window::new("🚀 新建 Rust 项目")
            .open(&mut win_open)
            .collapsible(false)
            .resizable(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("项目名称:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut dialog.name)
                            .desired_width(260.0)
                            .hint_text("例如: my_rust_service"),
                    );
                    let ime_busy = self.ime_composing
                        || self
                            .ime_commit_at
                            .map(|t| t.elapsed() < Duration::from_millis(150))
                            .unwrap_or(false);
                    if resp.lost_focus() && !ime_busy && ctx.input(|i| i.key_pressed(Key::Enter)) {
                        confirmed = true;
                    }
                });
                ui.add_space(6.0);
                ui.label("选择项目脚手架模板:");
                for (idx, tname) in PROJECT_TEMPLATES.iter().enumerate() {
                    ui.radio_value(&mut dialog.template, idx, *tname);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("创建位置:");
                    ui.label(
                        RichText::new(dialog.location.display().to_string())
                            .size(11.0)
                            .weak(),
                    );
                    if ui.small_button("更改...").clicked() {
                        change_location = true;
                    }
                });
                if let Some(err) = &dialog.error {
                    ui.label(RichText::new(err.as_str()).small().color(BRAND_RED));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("创建项目").clicked() {
                        confirmed = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });
        if change_location {
            if let Some(dir) = rfd::FileDialog::new()
                .set_title("选择新项目的创建位置")
                .pick_folder()
            {
                dialog.location = dir;
                dialog.error = None;
            }
        }
        if cancelled {
            win_open = false;
        }
        if confirmed {
            let kept = self.create_project(dialog);
            if let Some(d) = kept {
                self.project_dialog = Some(d);
            }
        } else if win_open {
            self.project_dialog = Some(dialog);
        }
    }

    pub fn draw_name_dialog(&mut self, ctx: &egui::Context) {
        let mut dialog = match self.name_dialog.take() {
            Some(d) => d,
            None => return,
        };
        let mut open = true;
        let mut cancelled = false;
        let mut confirmed = false;
        egui::Window::new(RichText::new(dialog.title.as_str()).strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.label(dialog.hint.as_str());
                ui.add_space(6.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut dialog.value)
                        .desired_width(300.0)
                        .font(FontSelection::FontId(FontId::monospace(MONO_SIZE))),
                );
                let ime_busy = self.ime_composing
                    || self
                        .ime_commit_at
                        .map(|t| t.elapsed() < Duration::from_millis(150))
                        .unwrap_or(false);
                if resp.lost_focus() && !ime_busy && ctx.input(|i| i.key_pressed(Key::Enter)) {
                    confirmed = true;
                }
                if let Some(err) = &dialog.error {
                    ui.label(RichText::new(err.as_str()).small().color(BRAND_RED));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("确定").clicked() {
                        confirmed = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });
        if cancelled {
            open = false;
        }
        if confirmed {
            let kept = self.confirm_name_dialog(dialog);
            if let Some(d) = kept {
                self.name_dialog = Some(d);
            }
        } else if open {
            self.name_dialog = Some(dialog);
        }
    }

    pub fn draw_about(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let mut open = true;
        egui::Window::new("关于 NovaRS")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new("🦀✨").size(44.0));
                    ui.heading(RichText::new(APP_FULL_NAME).strong());
                    ui.label(RichText::new(APP_VERSION).size(16.0).color(BRAND_RED));
                    ui.add_space(6.0);
                    ui.label("纯 Rust 模块化原生 IDE · AI Agent 副驾驶 · 内置 MCP 协议服务");
                    ui.label("支持大文件无截断行号 · VS Code 缩进线 · IDEA 级报错预检与一键修复");
                    let info = { toolchain_summary(&lock(&self.toolchain)) };
                    ui.label(RichText::new(info).small().weak());
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("© 2026 {}", APP_NAME)).small().weak());
                });
            });
        self.about_open = open;
    }

    pub fn draw_toast(&mut self, ctx: &egui::Context) {
        let active = match &self.toast {
            Some((_, at)) => at.elapsed() < Duration::from_secs(4),
            None => false,
        };
        if !active {
            self.toast = None;
            return;
        }
        if let Some((_, at)) = &self.toast {
            let remaining = Duration::from_secs(4).saturating_sub(at.elapsed());
            ctx.request_repaint_after(remaining);
        }
        let msg = self
            .toast
            .as_ref()
            .map(|(m, _)| m.clone())
            .unwrap_or_default();
        egui::Area::new(Id::new("toast_area"))
            .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -32.0))
            .show(ctx, |ui| {
                let frame = egui::Frame::popup(ui.style());
                frame.show(ui, |ui| {
                    ui.label(RichText::new(msg).size(13.0));
                });
            });
    }

    pub fn completion_kind_color(tag: &str, dark: bool) -> Color32 {
        if dark {
            match tag {
                "fn" => Color32::from_rgb(0x61, 0xaf, 0xef),
                "struct" => Color32::from_rgb(0xe5, 0xc0, 0x7b),
                "enum" => Color32::from_rgb(0xc6, 0x78, 0xdd),
                "mod" | "crate" => Color32::from_rgb(0x56, 0xb6, 0xc2),
                "let" | "var" => Color32::from_rgb(0xe0, 0x6c, 0x75),
                "use" => Color32::from_rgb(0x98, 0xc3, 0x79),
                "kw" | "snippet" => Color32::from_rgb(0xc6, 0x78, 0xdd),
                _ => Color32::from_rgb(0xab, 0xb2, 0xbf),
            }
        } else {
            match tag {
                "fn" => Color32::from_rgb(0x40, 0x78, 0xbf),
                "struct" => Color32::from_rgb(0xb7, 0x6b, 0x01),
                "enum" => Color32::from_rgb(0x98, 0x68, 0x01),
                "mod" | "crate" => Color32::from_rgb(0x0e, 0x8a, 0x84),
                "let" | "var" => Color32::from_rgb(0xca, 0x4b, 0x37),
                "use" => Color32::from_rgb(0x50, 0xa1, 0x4f),
                "kw" | "snippet" => Color32::from_rgb(0xa6, 0x26, 0xa4),
                _ => Color32::from_rgb(0x38, 0x3a, 0x42),
            }
        }
    }

    pub fn draw_completion_popup(&mut self, ctx: &egui::Context) {
        if self.completion.is_none() {
            return;
        }
        {
            let pending_gen = self.completion.as_ref().map(|p| p.gen).unwrap_or(-1);
            if pending_gen >= 0 {
                if let Some(lsp) = &self.lsp {
                    if *lock(&lsp.items_gen) == pending_gen {
                        let items = lock(&lsp.items).clone();
                        if !items.is_empty() {
                            if let Some(p) = &mut self.completion {
                                if p.items != items {
                                    p.items = items;
                                    p.selected = p.selected.min(p.items.len() - 1);
                                }
                            }
                        }
                    }
                }
            }
        }
        let Some(anchor) = self.completion_anchor else { return };
        let Some(popup) = self.completion.as_ref() else { return };
        if popup.items.is_empty() {
            self.completion = None;
            return;
        }
        let visible_rows = popup.items.len().min(9);
        let height = (visible_rows as f32 * POPUP_ROW_H + 10.0).min(210.0);
        let width = (popup
            .items
            .iter()
            .map(|c| c.label.chars().count() + c.detail.chars().count() / 2)
            .max()
            .unwrap_or(12) as f32
            * 7.5
            + 60.0)
            .clamp(160.0, 420.0);
        let screen = ctx.screen_rect();
        let pos = egui::pos2(
            (anchor.x + 2.0)
                .min(screen.right() - width - 8.0)
                .max(screen.left() + 8.0),
            (anchor.y - height - 6.0).max(screen.top() + 36.0),
        );
        let dark = self.settings.dark_mode;
        let selected = popup.selected;
        let count = popup.items.len();
        let items = popup.items.clone();
        egui::Area::new(Id::new("completion_popup"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let mut frame = egui::Frame::none();
                frame.fill = if dark {
                    Color32::from_rgba_unmultiplied(0x1e, 0x22, 0x2e, 0xf5)
                } else {
                    Color32::from_rgba_unmultiplied(0xfa, 0xfa, 0xfc, 0xf5)
                };
                frame.rounding = egui::Rounding::same(6.0);
                frame.stroke =
                    Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0xe9, 0x45, 0x60, 140));
                frame.shadow = egui::Shadow {
                    offset: egui::vec2(0.0, 6.0),
                    blur: 14.0,
                    spread: 1.0,
                    color: Color32::from_black_alpha(110),
                };
                frame.show(ui, |ui| {
                    ui.set_min_width(width);
                    egui::ScrollArea::vertical()
                        .max_height(height - 8.0)
                        .show(ui, |ui| {
                            for (i, item) in items.iter().enumerate() {
                                let is_selected = i == selected;
                                let kind_color = Self::completion_kind_color(item.kind_tag, dark);
                                let row =
                                    ui.allocate_ui(Vec2::new(width - 12.0, POPUP_ROW_H), |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(format!("{:>7}", item.kind_tag))
                                                    .monospace()
                                                    .size(11.0)
                                                    .color(kind_color),
                                            );
                                            let mut label =
                                                RichText::new(item.label.as_str()).size(13.0);
                                            if is_selected {
                                                label = label.strong();
                                            }
                                            ui.label(label);
                                            if !item.detail.is_empty() {
                                                ui.label(
                                                    RichText::new(item.detail.as_str())
                                                        .size(11.0)
                                                        .weak(),
                                                );
                                            }
                                        });
                                    });
                                if is_selected {
                                    ui.painter().rect_filled(
                                        row.response.rect,
                                        3.0,
                                        Color32::from_rgba_unmultiplied(0xe9, 0x45, 0x60, 46),
                                    );
                                }
                                let row_resp = row.response.interact(Sense::click());
                                if row_resp.clicked() {
                                    if let Some(p) = &mut self.completion {
                                        p.selected = i;
                                    }
                                }
                            }
                        });
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "{} 项 · ↑↓ 选择 · Tab 确认 · Esc 关闭  ({}/{})",
                            count,
                            selected + 1,
                            count
                        ))
                        .size(10.0)
                        .weak(),
                    );
                });
            });
    }

    pub fn draw_search_overlay(&mut self, ctx: &egui::Context) {
        let Some(mut search) = self.search.take() else { return };
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.search = None;
            return;
        }
        let running = search.running.load(Ordering::Relaxed);
        let results = lock(&search.results).clone();
        let dark = self.settings.dark_mode;
        let mut start_clicked = false;
        let mut open_target: Option<(PathBuf, usize)> = None;
        egui::Area::new(Id::new("search_overlay"))
            .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 44.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let mut frame = egui::Frame::none();
                frame.fill = if dark {
                    Color32::from_rgba_unmultiplied(0x1a, 0x1e, 0x28, 0xf2)
                } else {
                    Color32::from_rgba_unmultiplied(0xfc, 0xfc, 0xfe, 0xf2)
                };
                frame.rounding = egui::Rounding::same(8.0);
                frame.stroke =
                    Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0xe9, 0x45, 0x60, 130));
                frame.shadow = egui::Shadow {
                    offset: egui::vec2(0.0, 8.0),
                    blur: 18.0,
                    spread: 2.0,
                    color: Color32::from_black_alpha(120),
                };
                frame.show(ui, |ui| {
                    ui.set_min_width(640.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("🔍 在全项目中搜索").strong());
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("✕").clicked() {
                                self.search = None;
                            }
                        });
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("关键词:");
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut search.input)
                                .desired_width(320.0)
                                .hint_text("回车开始搜索（跳过 .git/target）"),
                        );
                        ui.checkbox(&mut search.case_sensitive, "大小写敏感");
                        if ui.button("搜索").clicked()
                            || (resp.lost_focus() && ctx.input(|i| i.key_pressed(Key::Enter)))
                        {
                            start_clicked = true;
                        }
                    });
                    ui.separator();
                    if running {
                        ui.label(RichText::new("正在搜索……").weak());
                    } else {
                        ui.label(
                            RichText::new(format!(
                                "共 {} 个匹配（最多显示 1000 条，点击结果跳转）",
                                results.len()
                            ))
                            .weak(),
                        );
                    }
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for hit in &results {
                                let label = format!(
                                    "{}:{}  {}",
                                    hit.path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default(),
                                    hit.line_no,
                                    hit.text
                                );
                                let resp = ui.selectable_label(
                                    false,
                                    RichText::new(label).monospace().size(12.0),
                                );
                                if resp.clicked() {
                                    open_target = Some((hit.path.clone(), hit.line_no));
                                }
                            }
                        });
                });
            });
        if start_clicked {
            self.search = Some(search);
            self.start_project_search(ctx);
            return;
        }
        self.search = Some(search);
        if let Some((path, line)) = open_target {
            self.open_file_in_tab(path);
            self.jump_to_line(ctx, line);
            if let Some(ti2) = self.active_tab {
                let tab_id = self.tabs[ti2].id;
                let text = self.tabs[ti2].text.clone();
                let start = line_start_char_index(&text, line);
                let line_len = text
                    .lines()
                    .nth(line.saturating_sub(1))
                    .map(|l| l.chars().count())
                    .unwrap_or(0);
                self.match_highlight = Some((tab_id, start, start + line_len));
            }
        }
    }

    pub fn draw_find_overlay(&mut self, ctx: &egui::Context) {
        if self.find.is_none() {
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.find = None;
            return;
        }
        let (total, index) = match &self.find {
            Some(f) => (f.total, f.index),
            None => (0, 0),
        };
        let mut next_clicked = false;
        let mut close_clicked = false;
        egui::Area::new(Id::new("find_overlay"))
            .anchor(Align2::RIGHT_TOP, egui::vec2(-24.0, 40.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let frame = egui::Frame::popup(ui.style());
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("查找:");
                        if let Some(find) = self.find.as_mut() {
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut find.input).desired_width(180.0),
                            );
                            let enter =
                                resp.lost_focus() && ctx.input(|i| i.key_pressed(Key::Enter));
                            if enter || ui.button("下一个").clicked() {
                                next_clicked = true;
                            }
                        }
                        ui.label(
                            RichText::new(if total > 0 {
                                format!("{}/{}", (index - 1 + total) % total + 1, total)
                            } else {
                                "0/0".to_string()
                            })
                            .weak(),
                        );
                        if ui.small_button("✕").clicked() {
                            close_clicked = true;
                        }
                    });
                });
            });
        if close_clicked {
            self.find = None;
        }
        if next_clicked {
            self.find_next_in_file(ctx);
        }
    }

    // ---------- 主界面帧 ----------

    pub fn draw_main(&mut self, ctx: &egui::Context) {
        // 确保窗口尺寸与显示器 DPI 同步（防御 winit 初始 scale 竞态）
        if !self.main_size_ok {
            let native_ppp = ctx.input(|i| i.viewport().native_pixels_per_point);
            if let Some(n) = native_ppp {
                if (ctx.pixels_per_point() - n).abs() > 0.01 {
                    ctx.set_pixels_per_point(n);
                }
            }
            if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
                let dw = (rect.width() - MAIN_SIZE.x).abs();
                let dh = (rect.height() - MAIN_SIZE.y).abs();
                if dw < 3.0 && dh < 3.0 {
                    self.main_size_ok = true;
                } else {
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(MAIN_SIZE));
                    center_window(ctx, MAIN_SIZE);
                }
            }
        }

        egui::TopBottomPanel::top("menu_bar_panel").show(ctx, |ui| {
            self.draw_menu_bar(ctx, ui);
        });
        egui::TopBottomPanel::bottom("status_panel")
            .exact_height(24.0)
            .show(ctx, |ui| {
                self.draw_status_bar(ui);
            });

        if self.terminal.visible {
            let ws = self.workspace.clone();
            let dark = self.settings.dark_mode;
            egui::TopBottomPanel::bottom("terminal_panel")
                .resizable(true)
                .default_height(220.0)
                .min_height(100.0)
                .show(ctx, |ui| {
                    self.terminal.draw(ctx, ui, ws.as_deref(), dark);
                });
        }
        if self.show_output {
            egui::TopBottomPanel::bottom("output_panel")
                .resizable(true)
                .default_height(180.0)
                .min_height(80.0)
                .max_height(400.0)
                .show(ctx, |ui| {
                    self.draw_output_panel(ui);
                });
        }

        // 右侧 AI 副驾驶侧边栏（默认开启，类似 VS Code Copilot / Claude Code）
        if self.ai_sidebar.visible {
            let active_file_name = self
                .active_tab
                .map(|i| self.tabs[i].title.clone())
                .unwrap_or_else(|| "none.rs".to_string());
            let active_file_text = self
                .active_tab
                .map(|i| self.tabs[i].text.clone())
                .unwrap_or_default();
            let active_diags = self
                .active_tab
                .map(|i| self.tabs[i].diagnostics.clone())
                .unwrap_or_default();
            let cursor_line = self.cursor_line;
            let ws = self.workspace.clone();
            let has_pending_patch = self.pending_fix_patch.is_some();

            let mut ai_act = None;
            egui::SidePanel::right("ai_copilot_panel")
                .resizable(true)
                .default_width(370.0)
                .min_width(280.0)
                .show(ctx, |ui| {
                    ai_act = Some(self.ai_sidebar.draw(
                        ctx,
                        ui,
                        &mut self.settings.models,
                        &mut self.settings.active_model_id,
                        &mut self.settings.thinking_effort_level,
                        &mut self.settings.sandbox_tier,
                        &mut self.mcp_server.builtin_sidebar_connected,
                        self.mcp_server.port,
                        &self.settings.network,
                        ws.as_deref(),
                        &active_file_name,
                        &active_file_text,
                        cursor_line,
                        &active_diags,
                        has_pending_patch,
                    ));
                });

            if let Some(act) = ai_act {
                if act.settings_changed {
                    self.save_settings();
                }
                if let Some(patch) = act.apply_code_patch {
                    self.apply_extracted_patch(ctx, &patch);
                }
                if act.accept_active_patch {
                    self.accept_pending_patch(ctx);
                }
                if act.revert_active_patch {
                    self.revert_pending_patch();
                }
                if let Some((tool_name, args)) = act.execute_mcp_tool {
                    let out = self.mcp_server.execute_tool_direct(&tool_name, &args);
                    let tool_reply = match out {
                        Ok(res) => format!("✅ MCP `{}` 执行成功:\n```\n{}\n```", tool_name, res),
                        Err(err) => format!("❌ MCP `{}` 执行失败: {}", tool_name, err),
                    };
                    self.ai_sidebar.active_session_mut().messages.push(ChatMessage {
                        role: ChatRole::Tool,
                        content: tool_reply,
                        reasoning_trace: None,
                        timestamp: "MCP Bridge".to_string(),
                        patches: Vec::new(),
                        mcp_tool_call: None,
                    });
                    self.refresh_tree();
                }
            }
        }

        if self.show_ast {
            egui::SidePanel::right("ast_panel")
                .resizable(true)
                .default_width(230.0)
                .min_width(160.0)
                .show(ctx, |ui| {
                    self.draw_ast_panel(ctx, ui);
                });
        }

        egui::SidePanel::right("minimap_panel")
            .resizable(false)
            .exact_width(MINIMAP_W)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                self.draw_minimap_panel(ui);
            });

        egui::SidePanel::left("files_panel")
            .resizable(true)
            .default_width(230.0)
            .min_width(150.0)
            .show(ctx, |ui| {
                self.draw_tree(ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.tabs.is_empty() {
                self.draw_welcome(ui);
            } else {
                self.draw_tab_bar(ui);
                self.draw_editor_area(ctx, ui);
            }
        });

        // 各类专业工具弹窗
        self.draw_completion_popup(ctx);
        self.draw_search_overlay(ctx);
        self.draw_find_overlay(ctx);
        self.draw_project_dialog(ctx);
        self.draw_name_dialog(ctx);
        self.draw_network_settings_dialog(ctx);

        let doc_act = draw_environment_doctor_window(
            ctx,
            &mut self.env_doctor_open,
            &self.toolchain,
            &mut self.settings.network.crates_io_mirror,
        );
        if doc_act.re_detect {
            let tc = self.toolchain.clone();
            std::thread::spawn(move || detect_toolchain(&tc));
            self.set_toast("正在后台重新检测工具链...");
        }
        if let Some(cmd) = doc_act.run_terminal_cmd {
            let ws = self.workspace.clone();
            self.terminal.run_command_in_active(&cmd, ws.as_deref(), ctx);
        }
        if let Some(mirror) = doc_act.apply_mirror {
            let proxy = self.settings.network.effective_proxy_url();
            match apply_crates_mirror(&mirror, proxy.as_deref()) {
                Ok(msg) => {
                    self.save_settings();
                    self.set_toast(msg);
                }
                Err(err) => self.set_toast(format!("写入镜像配置失败: {}", err)),
            }
        }

        let ws_clone = self.workspace.clone();
        match self.dep_manager.draw(ctx, ws_clone.as_deref()) {
            DepManagerAction::None => {}
            DepManagerAction::RunCargoCommand(cmd) => {
                self.terminal
                    .run_command_in_active(&cmd, ws_clone.as_deref(), ctx);
            }
            DepManagerAction::SavedCargoToml(msg) => {
                self.set_toast(msg);
                self.spawn_error_check(ctx);
            }
        }

        match self.plugin_manager.draw(
            ctx,
            &mut self.settings.enabled_plugins,
            ws_clone.as_deref(),
        ) {
            PluginAction::None => {}
            PluginAction::RunCommand(cmd) => {
                self.terminal
                    .run_command_in_active(&cmd, ws_clone.as_deref(), ctx);
            }
            PluginAction::SendAiSkillPrompt(prompt) => {
                self.ai_sidebar.visible = true;
                self.ai_sidebar.input_text = prompt;
                self.set_toast("已将插件技能指令填入 AI 副驾驶输入框");
            }
            PluginAction::ToggledPlugin => {
                self.save_settings();
            }
        }

        if self.mcp_server.draw_dialog(ctx) {
            self.save_settings();
        }

        self.draw_about(ctx);
        self.draw_toast(ctx);
    }

    fn apply_extracted_patch(&mut self, ctx: &egui::Context, patch: &ExtractedCodePatch) {
        self.apply_ai_or_quick_patch(
            ctx,
            patch.start_line,
            patch.end_line,
            &patch.replacement_code,
            "AI 副驾驶生成的修复补丁",
        );
    }

    pub fn poll_capture_request(&mut self, ctx: &egui::Context) {
        let Some(dir) = self.capture_dir.clone() else { return };
        let request_path = dir.join("request.txt");
        let modified = match std::fs::metadata(&request_path).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => return,
        };
        if self.capture_seen == Some(modified) {
            return;
        }
        let size_ok = ctx.input(|i| {
            let rect = i.screen_rect;
            rect.width() > 10.0 && rect.height() > 10.0 && i.viewport().minimized != Some(true)
        });
        if !size_ok {
            return;
        }
        self.capture_seen = Some(modified);
        let raw = match std::fs::read_to_string(&request_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let name = raw.trim().replace('\\', "_").replace('/', "_");
        if name.is_empty() || !name.ends_with(".png") {
            return;
        }
        self.capture_name = Some(name);
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
    }

    pub fn deliver_screenshots(&mut self, ctx: &egui::Context) {
        if self.capture_dir.is_none() {
            return;
        }
        let events = ctx.input(|i| i.raw.events.clone());
        for event in events {
            if let egui::Event::Screenshot { image, .. } = event {
                let Some(dir) = self.capture_dir.clone() else { continue };
                let Some(name) = self.capture_name.clone() else { continue };
                let path = dir.join(name);
                let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
                for color in &image.pixels {
                    rgba.push(color.r());
                    rgba.push(color.g());
                    rgba.push(color.b());
                    rgba.push(color.a());
                }
                let _ = image::save_buffer(
                    &path,
                    &rgba,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                );
                self.capture_name = None;
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.capture_dir.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        if self.lsp.is_none() {
            let delivered = lock(&self.lsp_slot).take();
            if let Some(session) = delivered {
                self.lsp = Some(session);
            }
        }
        self.poll_capture_request(ctx);
        self.deliver_screenshots(ctx);
        {
            let mut composing = self.ime_composing;
            ctx.input(|i| {
                for e in &i.raw.events {
                    match e {
                        egui::Event::Ime(egui::ImeEvent::Enabled)
                        | egui::Event::Ime(egui::ImeEvent::Preedit(_)) => composing = true,
                        egui::Event::Ime(egui::ImeEvent::Commit(_)) => {
                            composing = false;
                            self.ime_commit_at = Some(Instant::now());
                        }
                        egui::Event::Ime(egui::ImeEvent::Disabled) => composing = false,
                        _ => {}
                    }
                }
            });
            self.ime_composing = composing;
        }
        self.handle_global_shortcuts(ctx);
        self.handle_popup_keys(ctx);
        self.handle_tab_indent(ctx);
        match self.phase {
            Phase::Splash => self.draw_splash(ctx),
            Phase::Main => self.draw_main(ctx),
        }
    }
}

pub fn spawn_reader<R: std::io::Read + Send + 'static>(
    stream: Option<R>,
    log: Arc<Mutex<String>>,
    ctx: egui::Context,
) {
    if let Some(stream) = stream {
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        lock(&log).push_str(&line);
                        ctx.request_repaint();
                    }
                }
            }
        });
    }
}

pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let emoji_path = "C:\\Windows\\Fonts\\seguiemj.ttf";
    if Path::new(emoji_path).is_file() {
        if let Ok(bytes) = std::fs::read(emoji_path) {
            let loaded = catch_unwind(AssertUnwindSafe(|| FontData::from_owned(bytes)));
            if let Ok(data) = loaded {
                fonts.font_data.insert("nova_emoji".to_string(), data);
                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .push("nova_emoji".to_string());
                fonts
                    .families
                    .entry(FontFamily::Monospace)
                    .or_default()
                    .push("nova_emoji".to_string());
            }
        }
    }
    let embedded = catch_unwind(AssertUnwindSafe(|| {
        FontData::from_owned(FONT_CJK_EMBEDDED.to_vec())
    }));
    if let Ok(data) = embedded {
        fonts.font_data.insert("nova_cjk_embedded".to_string(), data);
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .push("nova_cjk_embedded".to_string());
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push("nova_cjk_embedded".to_string());
    }
    let candidates: [(&str, &str); 3] = [
        ("nova_cjk_deng", "C:\\Windows\\Fonts\\Deng.ttf"),
        ("nova_cjk_msyh", "C:\\Windows\\Fonts\\msyh.ttc"),
        ("nova_cjk_simhei", "C:\\Windows\\Fonts\\simhei.ttf"),
    ];
    for (name, path) in candidates {
        if !Path::new(path).is_file() {
            continue;
        }
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let loaded = catch_unwind(AssertUnwindSafe(|| FontData::from_owned(bytes)));
        match loaded {
            Ok(data) => {
                fonts.font_data.insert(name.to_string(), data);
                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .push(name.to_string());
                fonts
                    .families
                    .entry(FontFamily::Monospace)
                    .or_default()
                    .push(name.to_string());
                break;
            }
            Err(_) => continue,
        }
    }
    ctx.set_fonts(fonts);
}
