// NovaRS — Visual Dependency & Crate Version Manager
// Parses and updates `Cargo.toml`, allows one-click adding/removing/upgrading dependencies,
// toggling crate features, managing `[package] edition` and Rust versions.

use std::path::Path;
use eframe::egui::{self, RichText};
use crate::constants::{BRAND_ACCENT, BRAND_GREEN, BRAND_RED, BRAND_WARN};

#[derive(Clone, Debug)]
pub struct CrateDepEntry {
    pub name: String,
    pub version: String,
    pub features: String,
    pub dev: bool,
}

#[derive(Default)]
pub struct DependencyManagerState {
    pub open: bool,
    pub package_name: String,
    pub package_version: String,
    pub edition: String,
    pub rust_version: String,
    pub deps: Vec<CrateDepEntry>,
    pub new_crate_name: String,
    pub new_crate_version: String,
    pub new_crate_features: String,
    pub new_is_dev: bool,
    pub loaded_workspace: Option<String>,
}

pub enum DepManagerAction {
    None,
    RunCargoCommand(String),
    SavedCargoToml(String),
}

pub const POPULAR_CRATES: &[(&str, &str, &str, &str)] = &[
    ("serde", "1.0", "derive", "序列化与反序列化标准库"),
    ("serde_json", "1.0", "", "JSON 解析与生成"),
    ("tokio", "1", "full", "异步运行时"),
    ("anyhow", "1.0", "", "灵活易用的错误处理"),
    ("thiserror", "2.0", "", "派生自定义 Error 类型"),
    ("clap", "4.5", "derive", "命令行参数解析"),
    ("reqwest", "0.12", "json,blocking", "HTTP 客户端"),
    ("rayon", "1.10", "", "数据并行计算库"),
    ("tracing", "0.1", "", "结构化日志与追踪"),
    ("rand", "0.8", "", "随机数生成"),
    ("regex", "1.11", "", "高性能正则表达式"),
    ("chrono", "0.4", "serde", "日期与时间处理"),
];

impl DependencyManagerState {
    pub fn reload_from_workspace(&mut self, workspace: Option<&Path>) {
        self.deps.clear();
        let Some(ws) = workspace else { return };
        self.loaded_workspace = Some(ws.display().to_string());
        let toml_path = ws.join("Cargo.toml");
        let Ok(content) = std::fs::read_to_string(&toml_path) else { return };

        self.package_name = "my-app".to_string();
        self.package_version = "0.1.0".to_string();
        self.edition = "2021".to_string();
        self.rust_version = String::new();

        let mut section = String::new();
        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line.to_string();
                continue;
            }
            if section == "[package]" {
                if let Some((k, v)) = line.split_once('=') {
                    let k = k.trim();
                    let v = v.trim().trim_matches('"');
                    match k {
                        "name" => self.package_name = v.to_string(),
                        "version" => self.package_version = v.to_string(),
                        "edition" => self.edition = v.to_string(),
                        "rust-version" => self.rust_version = v.to_string(),
                        _ => {}
                    }
                }
            } else if section == "[dependencies]" || section == "[dev-dependencies]" {
                let is_dev = section == "[dev-dependencies]";
                if let Some((k, v)) = line.split_once('=') {
                    let name = k.trim().to_string();
                    let rhs = v.trim();
                    let (version, features) = parse_dep_rhs(rhs);
                    self.deps.push(CrateDepEntry {
                        name,
                        version,
                        features,
                        dev: is_dev,
                    });
                }
            }
        }
    }

    pub fn add_or_update_dep_in_cargo_toml(
        workspace: &Path,
        crate_name: &str,
        version: &str,
        features: &str,
        is_dev: bool,
    ) -> Result<(), String> {
        let toml_path = workspace.join("Cargo.toml");
        let content = std::fs::read_to_string(&toml_path).map_err(|e| e.to_string())?;
        let target_header = if is_dev { "[dev-dependencies]" } else { "[dependencies]" };

        let formatted_line = if features.trim().is_empty() {
            format!("{} = \"{}\"", crate_name.trim(), version.trim())
        } else {
            let feats: Vec<String> = features
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| format!("\"{}\"", s))
                .collect();
            format!(
                "{} = {{ version = \"{}\", features = [{}] }}",
                crate_name.trim(),
                version.trim(),
                feats.join(", ")
            )
        };

        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        let mut in_target = false;
        let mut replaced = false;
        let mut insert_idx = None;

        for (i, line) in lines.iter_mut().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                if in_target && insert_idx.is_none() {
                    insert_idx = Some(i);
                }
                in_target = trimmed == target_header;
                continue;
            }
            if in_target {
                if let Some((k, _)) = trimmed.split_once('=') {
                    if k.trim() == crate_name.trim() {
                        *line = formatted_line.clone();
                        replaced = true;
                        break;
                    }
                }
            }
        }

        if !replaced {
            if let Some(idx) = insert_idx {
                lines.insert(idx, formatted_line);
            } else if in_target {
                lines.push(formatted_line);
            } else {
                lines.push(String::new());
                lines.push(target_header.to_string());
                lines.push(formatted_line);
            }
        }

        let mut out = lines.join("\n");
        out.push('\n');
        std::fs::write(&toml_path, out).map_err(|e| e.to_string())
    }

    pub fn remove_dep_from_cargo_toml(workspace: &Path, crate_name: &str) -> Result<(), String> {
        let toml_path = workspace.join("Cargo.toml");
        let content = std::fs::read_to_string(&toml_path).map_err(|e| e.to_string())?;
        let mut out_lines = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some((k, _)) = trimmed.split_once('=') {
                if k.trim() == crate_name {
                    continue;
                }
            }
            out_lines.push(line);
        }
        let mut out = out_lines.join("\n");
        out.push('\n');
        std::fs::write(&toml_path, out).map_err(|e| e.to_string())
    }

    pub fn update_package_meta(
        workspace: &Path,
        new_version: &str,
        new_edition: &str,
    ) -> Result<(), String> {
        let toml_path = workspace.join("Cargo.toml");
        let content = std::fs::read_to_string(&toml_path).map_err(|e| e.to_string())?;
        let mut in_pkg = false;
        let mut out_lines = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                in_pkg = trimmed == "[package]";
                out_lines.push(line.to_string());
                continue;
            }
            if in_pkg {
                if let Some((k, _)) = trimmed.split_once('=') {
                    if k.trim() == "version" {
                        out_lines.push(format!("version = \"{}\"", new_version.trim()));
                        continue;
                    } else if k.trim() == "edition" {
                        out_lines.push(format!("edition = \"{}\"", new_edition.trim()));
                        continue;
                    }
                }
            }
            out_lines.push(line.to_string());
        }
        let mut out = out_lines.join("\n");
        out.push('\n');
        std::fs::write(&toml_path, out).map_err(|e| e.to_string())
    }

    pub fn draw(&mut self, ctx: &egui::Context, workspace: Option<&Path>) -> DepManagerAction {
        let mut action = DepManagerAction::None;
        if !self.open {
            return action;
        }

        let ws_str = workspace.map(|p| p.display().to_string());
        if self.loaded_workspace != ws_str {
            self.reload_from_workspace(workspace);
        }

        let mut win_open = self.open;
        egui::Window::new("📦 依赖与版本管理中心 (Cargo Dependency & Version Manager)")
            .open(&mut win_open)
            .resizable(true)
            .default_width(680.0)
            .default_height(520.0)
            .show(ctx, |ui| {
                let Some(ws) = workspace else {
                    ui.label(RichText::new("请先打开一个包含 Cargo.toml 的 Rust 项目文件夹").color(BRAND_WARN));
                    return;
                };

                // 1. 项目版本与 Edition 设置
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("项目: {}", self.package_name)).strong());
                    ui.separator();
                    ui.label("版本 (Version):");
                    ui.add(egui::TextEdit::singleline(&mut self.package_version).desired_width(72.0));
                    ui.label("Edition:");
                    egui::ComboBox::from_id_salt("edition_combo")
                        .selected_text(&self.edition)
                        .width(70.0)
                        .show_ui(ui, |ui| {
                            for ed in ["2018", "2021", "2024"] {
                                ui.selectable_value(&mut self.edition, ed.to_string(), ed);
                            }
                        });
                    if ui.button("💾 保存包信息").clicked() {
                        if Self::update_package_meta(ws, &self.package_version, &self.edition).is_ok() {
                            self.reload_from_workspace( Some(ws));
                            action = DepManagerAction::SavedCargoToml("已更新 Cargo.toml 版本与 Edition".to_string());
                        }
                    }
                    if ui.button("🔄 cargo update").clicked() {
                        action = DepManagerAction::RunCargoCommand("cargo update".to_string());
                    }
                    if ui.button("🌳 cargo tree").clicked() {
                        action = DepManagerAction::RunCargoCommand("cargo tree".to_string());
                    }
                });
                ui.separator();

                // 2. 添加新依赖
                ui.label(RichText::new("➕ 添加或更新 Crate 依赖").strong().color(BRAND_ACCENT));
                ui.horizontal(|ui| {
                    ui.label("包名:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_crate_name)
                            .desired_width(130.0)
                            .hint_text("如 serde"),
                    );
                    ui.label("版本:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_crate_version)
                            .desired_width(70.0)
                            .hint_text("1.0"),
                    );
                    ui.label("Features:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_crate_features)
                            .desired_width(140.0)
                            .hint_text("derive, full (逗号分隔)"),
                    );
                    ui.checkbox(&mut self.new_is_dev, "dev");
                    if ui.button(RichText::new("添加依赖").color(BRAND_GREEN).strong()).clicked() {
                        let name = self.new_crate_name.trim().to_string();
                        let ver = if self.new_crate_version.trim().is_empty() {
                            "*".to_string()
                        } else {
                            self.new_crate_version.trim().to_string()
                        };
                        if !name.is_empty() {
                            if Self::add_or_update_dep_in_cargo_toml(
                                ws,
                                &name,
                                &ver,
                                &self.new_crate_features,
                                self.new_is_dev,
                            )
                            .is_ok()
                            {
                                self.new_crate_name.clear();
                                self.new_crate_features.clear();
                                self.reload_from_workspace(Some(ws));
                                action = DepManagerAction::SavedCargoToml(format!("已添加依赖 {}", name));
                            }
                        }
                    }
                });

                ui.add_space(4.0);
                // 3. 当前已安装依赖列表
                ui.label(RichText::new(format!("📋 当前项目依赖 ({} 项)", self.deps.len())).strong());
                let mut to_remove: Option<String> = None;
                egui::ScrollArea::vertical()
                    .id_salt("current_deps_scroll")
                    .max_height(190.0)
                    .show(ui, |ui| {
                        egui::Grid::new("deps_list_grid")
                            .num_columns(5)
                            .spacing([14.0, 6.0])
                            .striped(true)
                            .show(ui, |ui| {
                                ui.strong("Crate 名称");
                                ui.strong("版本约束");
                                ui.strong("开启特性 (Features)");
                                ui.strong("类型");
                                ui.strong("操作");
                                ui.end_row();

                                for dep in &self.deps {
                                    ui.label(RichText::new(&dep.name).monospace().strong());
                                    ui.label(RichText::new(&dep.version).monospace().color(BRAND_GREEN));
                                    ui.label(
                                        RichText::new(if dep.features.is_empty() {
                                            "default"
                                        } else {
                                            &dep.features
                                        })
                                        .size(11.5)
                                        .weak(),
                                    );
                                    ui.label(if dep.dev { "dev" } else { "normal" });
                                    if ui
                                        .small_button(RichText::new("移除").color(BRAND_RED))
                                        .clicked()
                                    {
                                        to_remove = Some(dep.name.clone());
                                    }
                                    ui.end_row();
                                }
                            });
                    });

                if let Some(rem) = to_remove {
                    if Self::remove_dep_from_cargo_toml(ws, &rem).is_ok() {
                        self.reload_from_workspace(Some(ws));
                        action = DepManagerAction::SavedCargoToml(format!("已移除依赖 {}", rem));
                    }
                }

                ui.separator();
                // 4. 常用生态库一键集成
                ui.label(RichText::new("🌟 Rust 精选常用库一键引入").strong());
                egui::ScrollArea::vertical()
                    .id_salt("popular_crates_scroll")
                    .max_height(150.0)
                    .show(ui, |ui| {
                        for &(cname, cver, cfeat, cdesc) in POPULAR_CRATES {
                            let already = self.deps.iter().any(|d| d.name == cname);
                            ui.horizontal(|ui| {
                                if already {
                                    ui.add_enabled(false, egui::Button::new("✓ 已安装").small());
                                } else if ui.small_button("＋ 一键引入").clicked() {
                                    if Self::add_or_update_dep_in_cargo_toml(ws, cname, cver, cfeat, false).is_ok() {
                                        self.reload_from_workspace(Some(ws));
                                        action = DepManagerAction::SavedCargoToml(format!("已一键引入 {} v{}", cname, cver));
                                    }
                                }
                                ui.label(RichText::new(cname).monospace().strong().color(BRAND_ACCENT));
                                ui.label(RichText::new(format!("v{}", cver)).monospace().size(11.5));
                                if !cfeat.is_empty() {
                                    ui.label(RichText::new(format!("[{}]", cfeat)).size(11.0).color(BRAND_WARN));
                                }
                                ui.label(RichText::new(cdesc).size(11.5).weak());
                            });
                        }
                    });
            });
        self.open = win_open;
        action
    }
}

fn parse_dep_rhs(rhs: &str) -> (String, String) {
    if rhs.starts_with('"') {
        return (rhs.trim_matches('"').to_string(), String::new());
    }
    let mut version = String::from("*");
    let mut features = String::new();
    if let Some(v_pos) = rhs.find("version") {
        let rest = &rhs[v_pos..];
        if let Some(q1) = rest.find('"') {
            let after_q1 = &rest[q1 + 1..];
            if let Some(q2) = after_q1.find('"') {
                version = after_q1[..q2].to_string();
            }
        }
    }
    if let Some(f_pos) = rhs.find("features") {
        let rest = &rhs[f_pos..];
        if let Some(b1) = rest.find('[') {
            let after_b1 = &rest[b1 + 1..];
            if let Some(b2) = after_b1.find(']') {
                features = after_b1[..b2].replace('"', "").trim().to_string();
            }
        }
    }
    (version, features)
}
