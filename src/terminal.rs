// NovaRS — Modern VS Code-Style Multi-Session Terminal Emulator
// Fixes the weird original terminal UI & behavior:
// 1. Dark integrated console viewport with authentic inline prompt (`PS cwd> `) at the bottom
// 2. Multi-tab terminal sessions (+ New Terminal, Switch, Close, Split/Clear)
// 3. Up/Down arrow command history navigation
// 4. Automatic ANSI escape stripping + semantic line coloring (errors in red, warnings in yellow,
//    prompts in cyan/green, output in crisp monospace)
// 5. Quick one-click Cargo & Git shortcut chips (`cargo build`, `cargo test`, `cargo clippy`, `git status`)
// 6. Cross-platform shell support (`powershell` on Windows with UTF-8 encoding forced, `bash`/`sh` on Unix)

use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use eframe::egui::{self, Align, Color32, FontId, FontSelection, Key, RichText, Stroke};

use crate::constants::{BRAND_ACCENT, BRAND_GREEN, BRAND_RED, BRAND_WARN, MONO_SIZE};
use crate::utils::{apply_no_window, lock, strip_ansi_escapes};

pub struct TerminalSession {
    pub id: usize,
    pub title: String,
    pub shell_name: String,
    pub child: Option<Arc<Mutex<Child>>>,
    pub stdin: Option<Arc<Mutex<ChildStdin>>>,
    pub log: Arc<Mutex<String>>,
    pub input: String,
    pub history: Vec<String>,
    pub history_cursor: Option<usize>,
    pub cwd_display: String,
}

impl TerminalSession {
    pub fn new(id: usize, title: impl Into<String>) -> Self {
        #[cfg(target_os = "windows")]
        let shell_name = "PowerShell".to_string();
        #[cfg(not(target_os = "windows"))]
        let shell_name = "bash".to_string();

        TerminalSession {
            id,
            title: title.into(),
            shell_name,
            child: None,
            stdin: None,
            log: Arc::new(Mutex::new(String::new())),
            input: String::new(),
            history: Vec::new(),
            history_cursor: None,
            cwd_display: String::from("~"),
        }
    }

    pub fn append(log: &Arc<Mutex<String>>, text: &str, ctx: &egui::Context) {
        let cleaned = strip_ansi_escapes(text);
        let mut guard = lock(log);
        guard.push_str(&cleaned);
        // 限制最大日志缓存防止内存无限膨胀
        if guard.len() > 256_000 {
            // 截断点必须落在 UTF-8 字符边界上，否则 `guard[drain_to..]` 会直接 panic
            let mut drain_to = guard.len() - 180_000;
            while drain_to < guard.len() && !guard.is_char_boundary(drain_to) {
                drain_to += 1;
            }
            if let Some(nl) = guard[drain_to..].find('\n') {
                let cut = drain_to + nl + 1;
                *guard = guard[cut..].to_string();
            } else {
                *guard = guard[drain_to..].to_string();
            }
        }
        drop(guard);
        ctx.request_repaint();
    }

    pub fn ensure_spawned(&mut self, workspace: Option<&Path>, ctx: &egui::Context) -> bool {
        if self.stdin.is_some() {
            return true;
        }
        let cwd = workspace
            .map(|p| p.to_path_buf())
            .unwrap_or_else(std::env::temp_dir);
        self.cwd_display = cwd.display().to_string();

        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = Command::new("powershell");
            apply_no_window(&mut c).args([
                "-NoLogo",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "-",
            ]);
            c
        };

        #[cfg(not(target_os = "windows"))]
        let mut cmd = {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
            Command::new(shell)
        };

        cmd.current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let Ok(mut child) = cmd.spawn() else {
            Self::append(&self.log, "[终端启动失败：未找到系统 Shell]\n", ctx);
            return false;
        };

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        self.child = Some(Arc::new(Mutex::new(child)));
        self.stdin = stdin.map(|s| Arc::new(Mutex::new(s)));

        #[cfg(target_os = "windows")]
        if let Some(stdin_arc) = &self.stdin {
            let mut guard = lock(stdin_arc);
            let init_script = format!(
                "$OutputEncoding = [Console]::InputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new(); Set-Location -LiteralPath '{}'\n",
                cwd.display().to_string().replace('\'', "''")
            );
            let _ = guard.write_all(init_script.as_bytes()).and_then(|_| guard.flush());
        }

        Self::append(
            &self.log,
            &format!(
                "╭─ NovaRS Integrated Terminal ({}) ─── 当前目录 / CWD: {}\n╰─ 输入命令按 Enter 执行，支持 ↑/↓ 历史命令切换\n\n",
                self.shell_name, self.cwd_display
            ),
            ctx,
        );

        if let Some(out) = stdout {
            let log = self.log.clone();
            let ctx2 = ctx.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(out);
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let text = String::from_utf8_lossy(&buf[..n]).to_string();
                            Self::append(&log, &text, &ctx2);
                        }
                    }
                }
            });
        }

        if let Some(err) = stderr {
            let log = self.log.clone();
            let ctx2 = ctx.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(err);
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let text = String::from_utf8_lossy(&buf[..n]).to_string();
                            Self::append(&log, &text, &ctx2);
                        }
                    }
                }
            });
        }
        true
    }

    pub fn send_command_str(&mut self, cmd_raw: &str, workspace: Option<&Path>, ctx: &egui::Context) {
        let cmd = cmd_raw.trim().to_string();
        if cmd.is_empty() {
            return;
        }
        if cmd == "clear" || cmd == "cls" {
            lock(&self.log).clear();
            return;
        }
        if let Some(rest) = cmd.strip_prefix("cd ") {
            let target = rest.trim().trim_matches('"').trim_matches('\'');
            let base = Path::new(&self.cwd_display);
            let joined = base.join(target);
            if joined.is_dir() {
                if let Ok(canon) = std::fs::canonicalize(&joined) {
                    self.cwd_display = canon.display().to_string();
                } else {
                    self.cwd_display = joined.display().to_string();
                }
            }
        }
        if self.history.last().map(|s| s.as_str()) != Some(&cmd) {
            self.history.push(cmd.clone());
        }
        self.history_cursor = None;

        if self.ensure_spawned(workspace, ctx) {
            Self::append(
                &self.log,
                &format!("\n❯ {}\n", cmd),
                ctx,
            );
            if let Some(stdin_arc) = &self.stdin {
                let mut guard = lock(stdin_arc);
                let _ = guard
                    .write_all(cmd.as_bytes())
                    .and_then(|_| guard.write_all(b"\n"))
                    .and_then(|_| guard.flush());
            }
        }
    }

    pub fn send_input(&mut self, workspace: Option<&Path>, ctx: &egui::Context) {
        let cmd = std::mem::take(&mut self.input);
        self.send_command_str(&cmd, workspace, ctx);
    }

    pub fn kill(&mut self) {
        if let Some(child_arc) = &self.child {
            let mut guard = lock(child_arc);
            let _ = guard.kill();
        }
        self.child = None;
        self.stdin = None;
    }
}

pub struct TerminalState {
    pub visible: bool,
    pub sessions: Vec<TerminalSession>,
    pub active_session: usize,
    pub next_id: usize,
    pub focus_input: bool,
}

impl TerminalState {
    pub fn new() -> Self {
        TerminalState {
            visible: false,
            sessions: vec![TerminalSession::new(1, "pwsh #1")],
            active_session: 0,
            next_id: 2,
            focus_input: false,
        }
    }

    pub fn ensure_spawned(&mut self, workspace: Option<&Path>, ctx: &egui::Context) -> bool {
        if self.sessions.is_empty() {
            let id = self.next_id;
            self.next_id += 1;
            self.sessions.push(TerminalSession::new(id, format!("终端 #{}", id)));
            self.active_session = 0;
        }
        let idx = self.active_session.min(self.sessions.len() - 1);
        self.sessions[idx].ensure_spawned(workspace, ctx)
    }

    pub fn new_tab(&mut self, workspace: Option<&Path>, ctx: &egui::Context) {
        let id = self.next_id;
        self.next_id += 1;
        let mut session = TerminalSession::new(id, format!("终端 #{}", id));
        session.ensure_spawned(workspace, ctx);
        self.sessions.push(session);
        self.active_session = self.sessions.len() - 1;
        self.visible = true;
        self.focus_input = true;
    }

    pub fn run_command_in_active(&mut self, cmd: &str, workspace: Option<&Path>, ctx: &egui::Context) {
        self.visible = true;
        self.ensure_spawned(workspace, ctx);
        let idx = self.active_session.min(self.sessions.len() - 1);
        self.sessions[idx].send_command_str(cmd, workspace, ctx);
    }

    pub fn kill_all(&mut self) {
        for s in &mut self.sessions {
            s.kill();
        }
    }

    pub fn draw(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, workspace: Option<&Path>, dark_mode: bool) {
        if self.sessions.is_empty() {
            self.ensure_spawned(workspace, ctx);
        }
        let active_idx = self.active_session.min(self.sessions.len().saturating_sub(1));
        self.active_session = active_idx;

        // 顶部栏：VS Code 风格多终端标签页 + 快捷 Cargo 指令栏 + 工具按钮
        ui.horizontal(|ui| {
            ui.label(RichText::new("🖥 集成终端").strong());
            ui.separator();

            let mut close_idx: Option<usize> = None;
            for (idx, sess) in self.sessions.iter().enumerate() {
                let is_active = idx == self.active_session;
                let resp = ui.selectable_label(
                    is_active,
                    RichText::new(format!("⚡ {}", sess.title)).size(12.0),
                );
                if resp.clicked() {
                    self.active_session = idx;
                    self.focus_input = true;
                }
                if self.sessions.len() > 1 && ui.small_button("×").clicked() {
                    close_idx = Some(idx);
                }
            }
            if let Some(ci) = close_idx {
                self.sessions[ci].kill();
                self.sessions.remove(ci);
                if self.active_session >= self.sessions.len() {
                    self.active_session = self.sessions.len().saturating_sub(1);
                }
                return;
            }

            if ui.small_button("＋ 新建终端").clicked() {
                self.new_tab(workspace, ctx);
                return;
            }

            ui.separator();
            ui.label(RichText::new("快捷:").size(11.0).weak());
            for (label, cmd) in [
                ("cargo check", "cargo check"),
                ("cargo run", "cargo run"),
                ("cargo test", "cargo test"),
                ("cargo clippy", "cargo clippy"),
                ("git status", "git status"),
            ] {
                if ui.small_button(label).clicked() {
                    self.sessions[self.active_session].send_command_str(cmd, workspace, ctx);
                }
            }

            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("✕ 隐藏").clicked() {
                    self.visible = false;
                }
                if ui.small_button("🔄 重启").clicked() {
                    let s = &mut self.sessions[self.active_session];
                    s.kill();
                    s.ensure_spawned(workspace, ctx);
                }
                if ui.small_button("🗑 清屏").clicked() {
                    lock(&self.sessions[self.active_session].log).clear();
                }
            });
        });
        ui.separator();

        // 终端控制台主体（深色沉浸式终端背景 + 彩色语义渲染 + 底部内联输入行）
        let term_bg = if dark_mode {
            Color32::from_rgb(0x11, 0x13, 0x19)
        } else {
            Color32::from_rgb(0x18, 0x1b, 0x22)
        };
        let frame = egui::Frame::none()
            .fill(term_bg)
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x2c, 0x32, 0x40)))
            .rounding(egui::Rounding::same(4.0))
            .inner_margin(egui::Margin::symmetric(10.0, 6.0));

        frame.show(ui, |ui| {
            let avail_h = ui.available_height().max(60.0);
            let output_h = (avail_h - 32.0).max(36.0);

            let sess = &mut self.sessions[self.active_session];
            let log_snapshot = lock(&sess.log).clone();

            egui::ScrollArea::vertical()
                .id_salt(("term_scroll", sess.id))
                .max_height(output_h)
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in log_snapshot.lines() {
                        let color = if line.starts_with("❯ ") || line.starts_with("PS>") {
                            BRAND_ACCENT
                        } else if line.starts_with("╭─") || line.starts_with("╰─") {
                            BRAND_GREEN
                        } else if line.contains("error[")
                            || line.starts_with("error:")
                            || line.contains("panicked at")
                            || line.contains("CategoryInfo")
                        {
                            BRAND_RED
                        } else if line.contains("warning:") || line.contains("warning[") {
                            BRAND_WARN
                        } else {
                            Color32::from_rgb(0xd8, 0xde, 0xe9)
                        };
                        ui.add(
                            egui::Label::new(
                                RichText::new(line)
                                    .font(FontId::monospace(MONO_SIZE - 0.5))
                                    .color(color),
                            )
                            .selectable(true),
                        );
                    }
                });

            ui.add_space(4.0);
            // 底部真实终端风格内联命令输入行
            ui.horizontal(|ui| {
                let short_cwd = Path::new(&sess.cwd_display)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| sess.cwd_display.clone());
                ui.label(
                    RichText::new(format!("{} ❯", short_cwd))
                        .monospace()
                        .strong()
                        .color(BRAND_GREEN),
                );
                let input_w = (ui.available_width() - 64.0).max(120.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut sess.input)
                        .id(egui::Id::new(("term_input_line", sess.id)))
                        .font(FontSelection::FontId(FontId::monospace(MONO_SIZE)))
                        .text_color(Color32::WHITE)
                        .frame(false)
                        .desired_width(input_w)
                        .hint_text("输入 PowerShell / Cargo / Git 命令，Enter 执行，↑/↓ 浏览历史..."),
                );

                if self.focus_input {
                    resp.request_focus();
                    self.focus_input = false;
                }

                // 历史记录上下键导航
                if resp.has_focus() {
                    if ctx.input(|i| i.key_pressed(Key::ArrowUp)) && !sess.history.is_empty() {
                        let next_cur = match sess.history_cursor {
                            Some(c) => c.saturating_sub(1),
                            None => sess.history.len() - 1,
                        };
                        sess.history_cursor = Some(next_cur);
                        sess.input = sess.history[next_cur].clone();
                    } else if ctx.input(|i| i.key_pressed(Key::ArrowDown)) && !sess.history.is_empty() {
                        if let Some(c) = sess.history_cursor {
                            if c + 1 < sess.history.len() {
                                sess.history_cursor = Some(c + 1);
                                sess.input = sess.history[c + 1].clone();
                            } else {
                                sess.history_cursor = None;
                                sess.input.clear();
                            }
                        }
                    }
                }

                let enter_pressed = resp.lost_focus() && ctx.input(|i| i.key_pressed(Key::Enter));
                if enter_pressed || ui.small_button("运行 ⏎").clicked() {
                    sess.send_input(workspace, ctx);
                    resp.request_focus();
                }
            });
        });
    }
}
