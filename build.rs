// NovaRS 构建脚本：编译期定位 rust-analyzer 可执行文件并嵌入最终二进制。
// 找不到时写入空标记文件，程序运行时自动降级为内置补全。
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // exe 资源图标（资源管理器/任务栏/快捷方式显示的文件图标）
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/novars.ico");
        println!("cargo:rerun-if-changed=icons/novars.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=NovaRS: exe icon resource skipped ({e})");
        }
    }
    match locate_rust_analyzer() {
        Some(path) => {
            println!("cargo:rerun-if-changed={}", path.display());
            println!("cargo:rustc-env=NOVARS_RA_PATH={}", path.display());
            println!("cargo:rustc-env=NOVARS_RA_EMBED=1");
            println!("cargo:warning=NovaRS: 已嵌入 rust-analyzer ({})", path.display());
        }
        None => {
            let marker = marker_path();
            let _ = std::fs::write(&marker, b"");
            println!("cargo:rustc-env=NOVARS_RA_PATH={}", marker.display());
            println!("cargo:rustc-env=NOVARS_RA_EMBED=0");
            println!("cargo:warning=NovaRS: 未找到 rust-analyzer，补全将使用内置降级引擎");
        }
    }
}

fn marker_path() -> PathBuf {
    let out = std::env::var("OUT_DIR").unwrap_or_else(|_| ".".to_string());
    Path::new(&out).join("novars-ra-absent.marker")
}

fn locate_rust_analyzer() -> Option<PathBuf> {
    // 1) PATH 查找（where 为 Windows 系统工具，参数列表调用）
    if let Ok(output) = Command::new("where").arg("rust-analyzer").output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let candidate = PathBuf::from(line.trim());
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    // 2) 常见安装位置回退
    if let Ok(home) = std::env::var("USERPROFILE") {
        let candidate = PathBuf::from(&home).join(".cargo").join("bin").join("rust-analyzer.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let candidate = PathBuf::from(&home).join(".cargo").join("bin").join("rust-analyzer");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
