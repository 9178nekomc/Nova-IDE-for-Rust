// NovaRS v2.0 (Super Upgrade Edition) — Modular Native Rust IDE
// Built with egui + eframe + syn + walkdir + rfd + serde_json
// Architecture:
// - constants: Global layout, brand colors, and virtual windowing thresholds
// - config:    Persistent settings, proxy/network config, AI models, and sandbox permission tiers
// - theme:     8 IDE & code syntax themes (VS Code Dark+, One Dark Pro, Dracula, Tokyo Night, Catppuccin, etc.)
// - icons:     Dedicated file & folder format icons + colored badges for 30+ extensions
// - utils:     UTF-8 safe char/byte slicing, ANSI stripper, indent/unindent, and OS helpers
// - editor:    Large-file virtualized editor, dynamic-width line gutter (no truncation at 4000-100000+ lines),
//              VS Code authentic indentation guides, and IDEA-style real-time code diagnostics + AI Quick-Fix
// - lsp:       Embedded rust-analyzer JSON-RPC client + intelligent fallback completion engine
// - ast:       syn-powered real-time AST outline tree
// - terminal:  Modern VS Code-style multi-tab integrated terminal with command history & UTF-8 support
// - toolchain: One-click Rust environment detector, installer, channel switcher, and crates.io mirror configurator
// - deps:      Visual Cargo.toml dependency & version manager
// - project:   Multi-template project generator & Debug/Release/Backtrace runner
// - ai:        AI Copilot & Agent Sidebar (multi-protocol APIs, Codex thinking effort slider, multi-session,
//              Strict/Balanced/YOLO sandbox tiers, and Claude Code-grade English system prompt)
// - mcp:       Built-in Model Context Protocol (MCP) HTTP JSON-RPC 2.0 server & built-in AI sidebar bridge
// - plugins:   Extensible plugin & hook ecosystem (`plugins/*.json`)
// - app:       Main application state and UI layout orchestrator

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod ai;
pub mod app;
pub mod ast;
pub mod config;
pub mod constants;
pub mod deps;
pub mod editor;
pub mod icons;
pub mod lsp;
pub mod mcp;
pub mod plugins;
pub mod project;
pub mod terminal;
pub mod theme;
pub mod toolchain;
pub mod utils;

use crate::app::App;
use crate::constants::{APP_FULL_NAME, SPLASH_SIZE};
use eframe::egui;

fn main() -> Result<(), eframe::Error> {
    let icon_bytes = include_bytes!("../icons/novars-512.png");
    let icon = match image::load_from_memory(icon_bytes) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = (rgba.width(), rgba.height());
            Some(std::sync::Arc::new(egui::IconData {
                width: w,
                height: h,
                rgba: rgba.into_raw(),
            }))
        }
        Err(_) => None,
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(APP_FULL_NAME)
        .with_inner_size([SPLASH_SIZE.x, SPLASH_SIZE.y])
        .with_decorations(false)
        .with_resizable(false);

    if let Some(ic) = icon {
        viewport = viewport.with_icon(ic);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        APP_FULL_NAME,
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
