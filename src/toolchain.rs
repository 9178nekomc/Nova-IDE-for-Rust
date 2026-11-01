// NovaRS — One-Click Environment Detector, Installer & Mirror/Toolchain Configurator
// Features:
// 1. Deep environment inspection: `rustc`, `cargo`, `rustup`, `rust-analyzer`, `clippy`, `rustfmt`,
//    active toolchain channel, host target triple, C/C++ linker (`link.exe` / `gcc` / `clang`), Git
// 2. One-click component installation (`rustup component add rust-analyzer clippy rustfmt rust-src`)
// 3. One-click Toolchain Channel Switcher (`stable`, `beta`, `nightly`)
// 4. One-click Crates.io & Rustup Domestic/Global Mirror Configurator (Official / USTC / TUNA / SJTUG / RsProxy)
//    generating `~/.cargo/config.toml` automatically

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use eframe::egui::{self, RichText, Ui};

use crate::constants::{BRAND_ACCENT, BRAND_GREEN, BRAND_WARN};
use crate::utils::{apply_no_window, lock, run_cargo_version, run_rustc_version, run_rustup_version, which_exists};

#[derive(Default, Clone)]
pub struct SystemInfo {
    pub rustc_full: Option<String>,
    pub rustc_short: Option<String>,
    pub cargo_full: Option<String>,
    pub rustup_full: Option<String>,
    pub active_channel: String,
    pub host_triple: String,
    pub rust_analyzer: bool,
    pub clippy_installed: bool,
    pub rustfmt_installed: bool,
    pub git_installed: bool,
    pub linker_ready: bool,
    pub status_log: String,
}

pub fn toolchain_summary(info: &SystemInfo) -> String {
    let rustc = info
        .rustc_full
        .clone()
        .unwrap_or_else(|| "rustc 未检测到".to_string());
    let cargo = info
        .cargo_full
        .clone()
        .unwrap_or_else(|| "cargo 未检测到".to_string());
    let ra = if info.rust_analyzer { "已就绪" } else { "内置引擎" };
    format!("{} · {} · RA: {}", rustc, cargo, ra)
}

pub fn detect_toolchain(toolchain: &Arc<Mutex<SystemInfo>>) {
    let rustc_full = run_rustc_version();
    let rustc_short = rustc_full
        .as_ref()
        .and_then(|s| s.split_whitespace().nth(1))
        .map(|s| s.to_string());
    let cargo_full = run_cargo_version();
    let rustup_full = run_rustup_version();
    let rust_analyzer = which_exists("rust-analyzer");
    let clippy_installed = which_exists("cargo-clippy") || check_cargo_subcommand("--version", "clippy");
    let rustfmt_installed = which_exists("rustfmt");
    let git_installed = which_exists("git");

    #[cfg(target_os = "windows")]
    let linker_ready = which_exists("link") || which_exists("gcc") || which_exists("clang") || rustc_full.is_some();
    #[cfg(not(target_os = "windows"))]
    let linker_ready = which_exists("cc") || which_exists("gcc") || which_exists("clang");

    let (active_channel, host_triple) = detect_active_toolchain_and_host();

    let mut guard = lock(toolchain);
    guard.rustc_full = rustc_full;
    guard.rustc_short = rustc_short;
    guard.cargo_full = cargo_full;
    guard.rustup_full = rustup_full;
    guard.active_channel = active_channel;
    guard.host_triple = host_triple;
    guard.rust_analyzer = rust_analyzer;
    guard.clippy_installed = clippy_installed;
    guard.rustfmt_installed = rustfmt_installed;
    guard.git_installed = git_installed;
    guard.linker_ready = linker_ready;
}

fn check_cargo_subcommand(flag: &str, sub: &str) -> bool {
    let mut cmd = Command::new("cargo");
    apply_no_window(&mut cmd).args([sub, flag]);
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

fn detect_active_toolchain_and_host() -> (String, String) {
    let mut cmd = Command::new("rustc");
    apply_no_window(&mut cmd).arg("-vV");
    if let Ok(out) = cmd.output() {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut host = String::from("unknown");
            let mut release = String::from("stable");
            for line in text.lines() {
                if let Some(h) = line.strip_prefix("host:") {
                    host = h.trim().to_string();
                } else if let Some(r) = line.strip_prefix("release:") {
                    release = if r.contains("nightly") {
                        "nightly".to_string()
                    } else if r.contains("beta") {
                        "beta".to_string()
                    } else {
                        "stable".to_string()
                    };
                }
            }
            return (release, host);
        }
    }
    ("stable".to_string(), std::env::consts::ARCH.to_string())
}

pub fn cargo_config_path() -> Option<PathBuf> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .map(|h| PathBuf::from(h).join(".cargo").join("config.toml"))
}

/// 一键写入 Crates.io 镜像加速源配置 (`~/.cargo/config.toml`)
pub fn apply_crates_mirror(mirror_id: &str, proxy_url: Option<&str>) -> Result<String, String> {
    let Some(path) = cargo_config_path() else {
        return Err("无法定位用户主目录 (~/.cargo)".to_string());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let mut content = String::from("# Generated by NovaRS IDE Environment Configurator\n\n");
    match mirror_id {
        "rsproxy" => {
            content.push_str(
                "[source.crates-io]\nreplace-with = 'rsproxy-sparse'\n\n\
                 [source.rsproxy]\nregistry = \"https://rsproxy.cn/crates.io-index\"\n\n\
                 [source.rsproxy-sparse]\nregistry = \"sparse+https://rsproxy.cn/index/\"\n\n\
                 [registries.rsproxy]\nindex = \"https://rsproxy.cn/crates.io-index\"\n\n",
            );
        }
        "tuna" => {
            content.push_str(
                "[source.crates-io]\nreplace-with = 'tuna'\n\n\
                 [source.tuna]\nregistry = \"sparse+https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/\"\n\n",
            );
        }
        "ustc" => {
            content.push_str(
                "[source.crates-io]\nreplace-with = 'ustc'\n\n\
                 [source.ustc]\nregistry = \"sparse+https://mirrors.ustc.edu.cn/crates.io-index/\"\n\n",
            );
        }
        _ => {
            content.push_str("# Using Official crates.io Sparse Registry\n");
        }
    }

    if let Some(proxy) = proxy_url {
        if !proxy.trim().is_empty() {
            content.push_str(&format!(
                "[http]\nproxy = \"{}\"\ncheck-revoke = false\n\n[https]\nproxy = \"{}\"\n",
                proxy.trim(),
                proxy.trim()
            ));
        }
    }

    std::fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(format!("已成功写入 Cargo 配置文件: {}", path.display()))
}

pub struct ToolchainDoctorAction {
    pub run_terminal_cmd: Option<String>,
    pub re_detect: bool,
    pub apply_mirror: Option<String>,
}

pub fn draw_environment_doctor_window(
    ctx: &egui::Context,
    open: &mut bool,
    toolchain: &Arc<Mutex<SystemInfo>>,
    current_mirror: &mut String,
) -> ToolchainDoctorAction {
    let mut action = ToolchainDoctorAction {
        run_terminal_cmd: None,
        re_detect: false,
        apply_mirror: None,
    };
    if !*open {
        return action;
    }

    let info = lock(toolchain).clone();
    egui::Window::new("🛠 环境检测与一键配置中心 (Environment & Toolchain Doctor)")
        .open(open)
        .resizable(true)
        .default_width(620.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Rust 工具链与开发环境全检");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("🔄 立即重新检测").clicked() {
                        action.re_detect = true;
                    }
                });
            });
            ui.separator();

            egui::Grid::new("env_doctor_grid")
                .num_columns(3)
                .spacing([16.0, 8.0])
                .striped(true)
                .show(ui, |ui| {
                    draw_status_row(
                        ui,
                        "Rust 编译器 (rustc)",
                        info.rustc_full.is_some(),
                        info.rustc_full.as_deref().unwrap_or("未检测到 rustc"),
                    );
                    if info.rustc_full.is_none() && ui.small_button("一键安装 Rustup").clicked() {
                        #[cfg(target_os = "windows")]
                        {
                            action.run_terminal_cmd = Some(
                                "winget install --id Rustlang.Rustup -e".to_string(),
                            );
                        }
                        #[cfg(not(target_os = "windows"))]
                        {
                            action.run_terminal_cmd = Some(
                                "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y".to_string(),
                            );
                        }
                    } else {
                        ui.label(RichText::new("✓ 正常").color(BRAND_GREEN));
                    }
                    ui.end_row();

                    draw_status_row(
                        ui,
                        "包管理器 (cargo)",
                        info.cargo_full.is_some(),
                        info.cargo_full.as_deref().unwrap_or("未检测到 cargo"),
                    );
                    ui.label(if info.cargo_full.is_some() { "✓ 正常" } else { "随 rustup 安装" });
                    ui.end_row();

                    draw_status_row(
                        ui,
                        "工具链管理器 (rustup)",
                        info.rustup_full.is_some(),
                        info.rustup_full.as_deref().unwrap_or("未检测到 rustup"),
                    );
                    if ui.small_button("更新工具链 (rustup update)").clicked() {
                        action.run_terminal_cmd = Some("rustup update".to_string());
                    }
                    ui.end_row();

                    draw_status_row(
                        ui,
                        "LSP 语言服务 (rust-analyzer)",
                        info.rust_analyzer,
                        if info.rust_analyzer {
                            "系统已安装 + 内置双保险"
                        } else {
                            "系统未安装（可使用内置引擎或一键安装）"
                        },
                    );
                    if ui.small_button("一键安装组件").clicked() {
                        action.run_terminal_cmd =
                            Some("rustup component add rust-analyzer rust-src".to_string());
                    }
                    ui.end_row();

                    draw_status_row(
                        ui,
                        "代码质量检查 (Clippy)",
                        info.clippy_installed,
                        if info.clippy_installed { "已安装 cargo-clippy" } else { "未安装 clippy" },
                    );
                    if ui.small_button("安装 clippy").clicked() {
                        action.run_terminal_cmd = Some("rustup component add clippy".to_string());
                    }
                    ui.end_row();

                    draw_status_row(
                        ui,
                        "代码格式化 (rustfmt)",
                        info.rustfmt_installed,
                        if info.rustfmt_installed { "已安装 rustfmt" } else { "未安装 rustfmt" },
                    );
                    if ui.small_button("安装 rustfmt").clicked() {
                        action.run_terminal_cmd = Some("rustup component add rustfmt".to_string());
                    }
                    ui.end_row();

                    draw_status_row(
                        ui,
                        "系统链接器 & 构建环境",
                        info.linker_ready,
                        &format!("目标平台: {} (频道: {})", info.host_triple, info.active_channel),
                    );
                    if ui.small_button("一键补全全部标准组件").clicked() {
                        action.run_terminal_cmd = Some(
                            "rustup component add rust-analyzer clippy rustfmt rust-src llvm-tools-preview".to_string(),
                        );
                    }
                    ui.end_row();
                });

            ui.add_space(10.0);
            ui.separator();
            ui.label(RichText::new("🔀 Rust 版本频道管理 (Toolchain Channel Switcher)").strong());
            ui.horizontal(|ui| {
                ui.label(format!("当前活动频道: {}", info.active_channel));
                for ch in ["stable", "beta", "nightly"] {
                    if ui
                        .button(format!("切换到 {}", ch))
                        .on_hover_text(format!("执行 rustup default {}", ch))
                        .clicked()
                    {
                        action.run_terminal_cmd =
                            Some(format!("rustup toolchain install {0} && rustup default {0}", ch));
                    }
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.label(RichText::new("🚀 Crates.io 镜像源一键加速配置 (~/.cargo/config.toml)").strong());
            ui.horizontal(|ui| {
                for (id, label) in [
                    ("official", "🌐 官方源 (crates.io)"),
                    ("rsproxy", "⚡ 字节跳动 RsProxy 稀疏索引"),
                    ("tuna", "🎓 清华大学 TUNA 稀疏镜像"),
                    ("ustc", "🔬 中科大 USTC 稀疏镜像"),
                ] {
                    ui.radio_value(current_mirror, id.to_string(), label);
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("✅ 一键写入 Cargo 镜像与代理配置").color(BRAND_ACCENT).strong())
                    .clicked()
                {
                    action.apply_mirror = Some(current_mirror.clone());
                }
            });
        });

    action
}

fn draw_status_row(ui: &mut Ui, title: &str, ok: bool, detail: &str) {
    let icon = if ok { "🟢" } else { "🟡" };
    let color = if ok { BRAND_GREEN } else { BRAND_WARN };
    ui.label(RichText::new(format!("{} {}", icon, title)).strong());
    ui.label(RichText::new(detail).color(color).size(12.0));
}
