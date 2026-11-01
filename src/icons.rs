// NovaRS — Dedicated File & Format Icon System
// Provides distinct, colored vector/badge + glyph icons for every common file format and folder type.

use std::path::Path;
use eframe::egui::{self, Color32, FontId, Rect, RichText, Sense, Vec2};

#[derive(Clone, Copy)]
pub struct FileIconSpec {
    pub glyph: &'static str,
    pub badge: &'static str,
    pub color: Color32,
    pub lang_name: &'static str,
}

pub fn icon_for_folder(name: &str, open: bool) -> FileIconSpec {
    let lower = name.to_lowercase();
    let color = match lower.as_str() {
        "src" | "source" => Color32::from_rgb(0x61, 0xaf, 0xef),
        "tests" | "test" | "benches" => Color32::from_rgb(0x98, 0xc3, 0x79),
        "assets" | "icons" | "images" | "res" => Color32::from_rgb(0xe5, 0xc0, 0x7b),
        "tools" | "scripts" | "bin" => Color32::from_rgb(0xc6, 0x78, 0xdd),
        "docs" | "doc" => Color32::from_rgb(0x56, 0xb6, 0xc2),
        "plugins" | "extensions" => Color32::from_rgb(0xe9, 0x45, 0x60),
        ".cargo" | ".vscode" | ".github" => Color32::from_rgb(0x7f, 0x84, 0x8e),
        _ => Color32::from_rgb(0xdc, 0xb6, 0x7a),
    };
    let glyph = if open { "📂" } else { "📁" };
    FileIconSpec {
        glyph,
        badge: "DIR",
        color,
        lang_name: "文件夹",
    }
}

pub fn icon_for_path(path: &Path) -> FileIconSpec {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let lower_name = file_name.to_lowercase();

    // Special named files first
    match lower_name.as_str() {
        "cargo.toml" => {
            return FileIconSpec {
                glyph: "📦",
                badge: "TOML",
                color: Color32::from_rgb(0xe5, 0x78, 0x42),
                lang_name: "Cargo Manifest (TOML)",
            };
        }
        "cargo.lock" => {
            return FileIconSpec {
                glyph: "🔒",
                badge: "LOCK",
                color: Color32::from_rgb(0xd1, 0x9a, 0x66),
                lang_name: "Cargo Lockfile",
            };
        }
        "build.rs" => {
            return FileIconSpec {
                glyph: "🔨",
                badge: "BLD",
                color: Color32::from_rgb(0xe9, 0x45, 0x60),
                lang_name: "Rust Build Script",
            };
        }
        "main.rs" | "lib.rs" | "mod.rs" => {
            return FileIconSpec {
                glyph: "🦀",
                badge: "RS",
                color: Color32::from_rgb(0xde, 0xa5, 0x84),
                lang_name: "Rust Entry Module",
            };
        }
        "readme.md" | "readme" | "changelog.md" => {
            return FileIconSpec {
                glyph: "📘",
                badge: "MD",
                color: Color32::from_rgb(0x51, 0x9a, 0xba),
                lang_name: "Markdown Documentation",
            };
        }
        "license" | "license.md" | "license.txt" | " license-mit" | "license-apache" => {
            return FileIconSpec {
                glyph: "⚖",
                badge: "LIC",
                color: Color32::from_rgb(0xe5, 0xc0, 0x7b),
                lang_name: "License",
            };
        }
        "dockerfile" | "docker-compose.yml" | "docker-compose.yaml" => {
            return FileIconSpec {
                glyph: "🐳",
                badge: "DCK",
                color: Color32::from_rgb(0x24, 0x96, 0xed),
                lang_name: "Docker",
            };
        }
        "makefile" | "justfile" | "cmakelists.txt" => {
            return FileIconSpec {
                glyph: "⚙",
                badge: "MK",
                color: Color32::from_rgb(0x6d, 0x80, 0x86),
                lang_name: "Build Makefile",
            };
        }
        ".gitignore" | ".gitattributes" | ".gitmodules" => {
            return FileIconSpec {
                glyph: "🌿",
                badge: "GIT",
                color: Color32::from_rgb(0xf1, 0x4e, 0x32),
                lang_name: "Git Config",
            };
        }
        ".env" | ".env.local" | ".env.example" => {
            return FileIconSpec {
                glyph: "🔑",
                badge: "ENV",
                color: Color32::from_rgb(0xfa, 0xb3, 0x87),
                lang_name: "Environment Config",
            };
        }
        "rust-toolchain.toml" | "rust-toolchain" | "rustfmt.toml" | ".rustfmt.toml" | "clippy.toml" => {
            return FileIconSpec {
                glyph: "🛠",
                badge: "CFG",
                color: Color32::from_rgb(0xe0, 0x6c, 0x75),
                lang_name: "Rust Toolchain Config",
            };
        }
        _ => {}
    }

    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "rs" => FileIconSpec {
            glyph: "🦀",
            badge: "RS",
            color: Color32::from_rgb(0xde, 0xa5, 0x84),
            lang_name: "Rust",
        },
        "toml" => FileIconSpec {
            glyph: "⚙",
            badge: "TOML",
            color: Color32::from_rgb(0x9c, 0x42, 0x21),
            lang_name: "TOML",
        },
        "json" | "jsonc" | "jsonl" => FileIconSpec {
            glyph: "{}",
            badge: "JSON",
            color: Color32::from_rgb(0xcb, 0xcb, 0x41),
            lang_name: "JSON",
        },
        "md" | "markdown" | "mdx" => FileIconSpec {
            glyph: "📝",
            badge: "MD",
            color: Color32::from_rgb(0x51, 0x9a, 0xba),
            lang_name: "Markdown",
        },
        "yml" | "yaml" => FileIconSpec {
            glyph: "📐",
            badge: "YML",
            color: Color32::from_rgb(0xcb, 0x17, 0x1e),
            lang_name: "YAML",
        },
        "xml" | "plist" | "manifest" => FileIconSpec {
            glyph: "📰",
            badge: "XML",
            color: Color32::from_rgb(0xe3, 0x79, 0x33),
            lang_name: "XML",
        },
        "html" | "htm" => FileIconSpec {
            glyph: "🌐",
            badge: "HTML",
            color: Color32::from_rgb(0xe4, 0x4d, 0x26),
            lang_name: "HTML",
        },
        "css" | "scss" | "less" => FileIconSpec {
            glyph: "🎨",
            badge: "CSS",
            color: Color32::from_rgb(0x42, 0xa5, 0xf5),
            lang_name: "CSS",
        },
        "js" | "mjs" | "cjs" => FileIconSpec {
            glyph: "📜",
            badge: "JS",
            color: Color32::from_rgb(0xf7, 0xdf, 0x1e),
            lang_name: "JavaScript",
        },
        "ts" | "tsx" | "jsx" => FileIconSpec {
            glyph: "💠",
            badge: "TS",
            color: Color32::from_rgb(0x31, 0x78, 0xc6),
            lang_name: "TypeScript",
        },
        "py" | "pyw" | "pyi" => FileIconSpec {
            glyph: "🐍",
            badge: "PY",
            color: Color32::from_rgb(0x35, 0x72, 0xa5),
            lang_name: "Python",
        },
        "c" | "h" => FileIconSpec {
            glyph: "©",
            badge: "C",
            color: Color32::from_rgb(0x55, 0x55, 0xff),
            lang_name: "C/Header",
        },
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => FileIconSpec {
            glyph: "➕",
            badge: "C++",
            color: Color32::from_rgb(0xf3, 0x4b, 0x7d),
            lang_name: "C++",
        },
        "go" => FileIconSpec {
            glyph: "🐹",
            badge: "GO",
            color: Color32::from_rgb(0x00, 0xad, 0xd8),
            lang_name: "Go",
        },
        "java" | "jar" | "kt" => FileIconSpec {
            glyph: "☕",
            badge: "JVM",
            color: Color32::from_rgb(0xb0, 0x72, 0x19),
            lang_name: "Java/Kotlin",
        },
        "sh" | "bash" | "zsh" => FileIconSpec {
            glyph: "🐚",
            badge: "SH",
            color: Color32::from_rgb(0x89, 0xe0, 0x51),
            lang_name: "Shell Script",
        },
        "ps1" | "psm1" | "bat" | "cmd" => FileIconSpec {
            glyph: "⚡",
            badge: "PS1",
            color: Color32::from_rgb(0x01, 0x24, 0x56),
            lang_name: "PowerShell/Batch",
        },
        "sql" | "db" | "sqlite" | "sqlite3" => FileIconSpec {
            glyph: "🗄",
            badge: "SQL",
            color: Color32::from_rgb(0xe3, 0x8c, 0x00),
            lang_name: "SQL/Database",
        },
        "wgsl" | "glsl" | "hlsl" | "vert" | "frag" => FileIconSpec {
            glyph: "🔮",
            badge: "SHD",
            color: Color32::from_rgb(0xc6, 0x78, 0xdd),
            lang_name: "Shader",
        },
        "proto" => FileIconSpec {
            glyph: "📡",
            badge: "PB",
            color: Color32::from_rgb(0x56, 0xb6, 0xc2),
            lang_name: "Protobuf",
        },
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" => FileIconSpec {
            glyph: "🖼",
            badge: "IMG",
            color: Color32::from_rgb(0xa0, 0x74, 0xc4),
            lang_name: "Image Asset",
        },
        "ttf" | "otf" | "woff" | "woff2" | "ttc" => FileIconSpec {
            glyph: "🔤",
            badge: "FNT",
            color: Color32::from_rgb(0xe0, 0x6c, 0x75),
            lang_name: "Font Asset",
        },
        "zip" | "tar" | "gz" | "7z" | "xz" | "zst" | "rlib" => FileIconSpec {
            glyph: "🗜",
            badge: "ARC",
            color: Color32::from_rgb(0xd1, 0x9a, 0x66),
            lang_name: "Archive",
        },
        "exe" | "dll" | "so" | "dylib" | "wasm" | "bin" | "pdb" => FileIconSpec {
            glyph: "⚙",
            badge: "BIN",
            color: Color32::from_rgb(0x98, 0xc3, 0x79),
            lang_name: "Binary/WASM",
        },
        "lock" => FileIconSpec {
            glyph: "🔒",
            badge: "LCK",
            color: Color32::from_rgb(0x7f, 0x84, 0x8e),
            lang_name: "Lockfile",
        },
        "log" | "txt" | "csv" | "tsv" => FileIconSpec {
            glyph: "📄",
            badge: "TXT",
            color: Color32::from_rgb(0x8a, 0x93, 0xa5),
            lang_name: "Plain Text",
        },
        _ => FileIconSpec {
            glyph: "📄",
            badge: "FILE",
            color: Color32::from_rgb(0x9d, 0xa5, 0xb4),
            lang_name: "文本",
        },
    }
}

pub fn lang_of_path(path: &Path) -> &'static str {
    icon_for_path(path).lang_name
}

/// 在 UI 中绘制一个迷你彩色文件格式徽章 + 图标
pub fn draw_file_badge(ui: &mut egui::Ui, spec: &FileIconSpec) {
    let badge_w = 24.0;
    let badge_h = 15.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(badge_w, badge_h), Sense::hover());
    let painter = ui.painter();
    let bg = Color32::from_rgba_unmultiplied(spec.color.r(), spec.color.g(), spec.color.b(), 42);
    let border = Color32::from_rgba_unmultiplied(spec.color.r(), spec.color.g(), spec.color.b(), 150);
    painter.rect_filled(Rect::from_min_max(rect.min, rect.max), 3.0, bg);
    painter.rect_stroke(
        Rect::from_min_max(rect.min, rect.max),
        3.0,
        egui::Stroke::new(0.8_f32, border),
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        spec.badge,
        FontId::monospace(8.5),
        spec.color,
    );
}

pub fn rich_file_label(path: &Path, name: &str, override_color: Option<Color32>) -> (FileIconSpec, RichText) {
    let spec = icon_for_path(path);
    let text = format!("{} {}", spec.glyph, name);
    let rich = match override_color {
        Some(c) => RichText::new(text).color(c),
        None => RichText::new(text),
    };
    (spec, rich)
}
