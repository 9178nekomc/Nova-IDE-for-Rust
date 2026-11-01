// NovaRS — Project & Workspace Manager + Debug/Release Build & Run/Test/Debug Profiles

use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RunMode {
    Run,
    Build,
    Check,
    Test,
    Clippy,
    Fmt,
    Doc,
    DebugRun, // 带 RUST_BACKTRACE=full 与断点追踪日志的调试编译运行
}

impl RunMode {
    pub fn label(&self) -> &'static str {
        match self {
            RunMode::Run => "运行 (cargo run)",
            RunMode::Build => "构建 (cargo build)",
            RunMode::Check => "检查 (cargo check)",
            RunMode::Test => "单元测试 (cargo test)",
            RunMode::Clippy => "静态审查 (cargo clippy)",
            RunMode::Fmt => "代码格式化 (cargo fmt)",
            RunMode::Doc => "生成文档 (cargo doc)",
            RunMode::DebugRun => "🐞 调试编译运行 (Backtrace + 断点追踪)",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuildProfile {
    Debug,
    Release,
}

impl BuildProfile {
    pub fn label(&self) -> &'static str {
        match self {
            BuildProfile::Debug => "Debug (快速编译 + 调试符号)",
            BuildProfile::Release => "Release (-O3 优化发布)",
        }
    }
}

pub const PROJECT_TEMPLATES: &[&str] = &[
    "空项目 (Empty Binary)",
    "控制台输出示例 (Hello & Formatting)",
    "变量与所有权演练 (Variables & Ownership)",
    "模式匹配与分支判断 (Pattern Matching)",
    "交互式终端输入输出 (CLI Input/Output)",
    "多线程并发任务 (Concurrency & Threads)",
    "ライブラリ库项目 (Cargo Library lib.rs)",
    "HTTP 微服务脚手架 (TCP Web Server)",
];

pub fn sanitize_package_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('-').to_string();
    if cleaned.is_empty() {
        "new-project".to_string()
    } else {
        cleaned
    }
}

pub fn template_cargo_toml(package: &str) -> String {
    format!(
        "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
        package
    )
}

pub fn template_main_rs(template: usize) -> String {
    match template {
        1 => r#"fn main() {
    println!("Hello, NovaRS!");
    println!("这是输出模板：数字 = {}", 42);
}
"#
        .to_string(),
        2 => r#"fn main() {
    let mut count: u32 = 0;
    let name = "NovaRS";

    count += 1;
    let message = format!("变量模板：{} 已运行 {} 次", name, count);
    println!("{}", message);
}
"#
        .to_string(),
        3 => r#"fn main() {
    let n: i32 = 7;

    if n % 2 == 0 {
        println!("{} 是偶数", n);
    } else {
        println!("{} 是奇数", n);
    }

    match n {
        0..=9 => println!("个位数"),
        10..=99 => println!("两位数"),
        _ => println!("更大的数"),
    }
}
"#
        .to_string(),
        4 => r#"use std::io;

fn main() {
    println!("请输入你的年龄：");

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_ok() {
        let age: u32 = input.trim().parse().unwrap_or(18);
        if age >= 18 {
            println!("{} 岁：你已成年", age);
        } else {
            println!("{} 岁：你还未成年", age);
        }
    }
}
"#
        .to_string(),
        5 => r#"use std::thread;

fn main() {
    let handle = thread::spawn(|| {
        for i in 1..=5 {
            println!("[任务 1] 第 {} 次", i);
            thread::sleep(std::time::Duration::from_millis(100));
        }
    });

    for i in 1..=5 {
        println!("[任务 2] 第 {} 次", i);
        thread::sleep(std::time::Duration::from_millis(100));
    }

    let _ = handle.join();
    println!("所有异步任务完成");
}
"#
        .to_string(),
        6 => r#"pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
"#
        .to_string(),
        7 => r#"use std::io::{Read, Write};
use std::net::TcpListener;

fn main() {
    let addr = "127.0.0.1:8080";
    println!("NovaRS Mini HTTP Server listening on http://{}", addr);
    let Ok(listener) = TcpListener::bind(addr) else {
        eprintln!("Failed to bind {}", addr);
        return;
    };
    for stream in listener.incoming().flatten().take(5) {
        let mut stream = stream;
        let mut buf = [0u8; 512];
        let _ = stream.read(&mut buf);
        let body = "{\"status\":\"ok\",\"server\":\"NovaRS\"}";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(resp.as_bytes());
    }
}
"#
        .to_string(),
        _ => "fn main() {\n    println!(\"Hello, world!\");\n}\n".to_string(),
    }
}

pub struct ProjectDialog {
    pub name: String,
    pub location: PathBuf,
    pub template: usize,
    pub error: Option<String>,
}
