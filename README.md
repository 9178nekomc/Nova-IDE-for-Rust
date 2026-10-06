<div align="center">

# 🦀 NovaRS

**Nova IDE for Rust v0.1.0**

纯 Rust 原生实现的轻量级 IDE —— 单个 exe，双击即跑，无需安装。

`egui + eframe` · 无 WebView · 无 Electron · 无外部依赖

</div>

---

## ✨ 特性一览

### 智能编辑器
- **Rust 语法高亮**：关键字 / 字符串 / 注释 / 数字 / 类型 / 生命周期全着色，中文注释完美渲染
- **智能补全**：内嵌 rust-analyzer（LSP over stdio，编译期打包进 exe，用户零安装），打字自动触发 + `Ctrl+Space` 手动触发；LSP 未就绪时自动降级为内置引擎（当前文件标识符 + 标准库 + Cargo.toml 依赖）
- **行号与缩进辅助线**：VS Code 风格逐行缩进参考线，大文件只渲染可视行
- **多标签编辑**：✕ 关闭、脏标记 `*`、关闭未保存文件确认
- **撤销 / 重做 / 剪切 / 复制 / 粘贴 / 全选**（含中文输入法回车保护）
- **Tab 缩进**：4 空格，选区整块缩进
- **断点标记**：行号侧栏点击切换，红色圆点

### 工程能力
- **文件树**：walkdir 真实遍历，📁📄🦀📦 图标，状态色标记（绿=已保存 / 黄=未保存 / 红=编译错误），右键菜单（新建 / 重命名 / 删除 / 在文件管理器中显示 / 复制路径）
- **新建项目**：6 种模板（空项目 / 输出 / 变量 / 判断 / 输入判断输出 / 异步任务），选位置、输入名称即创建
- **项目搜索**：`Ctrl+Shift+F` 全项目文本搜索（跳过 .git/target），点击结果跳转 + 编辑器内黄色高亮
- **当前文件查找**：`Ctrl+F`，匹配计数，编辑器内高亮
- **AST 侧边栏**：syn 实时解析 mod / struct / fn / enum / trait，点击跳转对应行
- **真实终端**：内嵌 PowerShell 持久会话（`cd` 等状态保留），UTF-8 输出，面板高度自由拖拽
- **运行与检查**：`F5` cargo run / `F7` cargo check，输出面板实时显示，编译错误映射到文件树红色标记

### 界面
- **日落风启动画面**：三段渐变 + 流动斜线 + 工具链实时检测，就绪后自动进入主界面
- **代码迷你地图**：缩略色块 + 视口高亮框，点击/拖拽跳转
- **暗色 / 亮色主题**：一键切换，持久化偏好
- **增强状态栏**：文件路径 · 行/列 · 语言 · UTF-8 · LF/CRLF · Git 分支 · 版本

### 交付形态
- **真正的单文件**：rust-analyzer、中文字体、应用图标全部编译期嵌入；CRT 静态链接。拷贝到任何 Windows 10/11 机器双击即用，无需 VC++ 运行库、无需任何安装

---

## ⌨️ 快捷键

| 快捷键 | 功能 |
|--------|------|
| `Ctrl+N` | 新建文件 |
| `Ctrl+O` | 打开文件夹 |
| `Ctrl+S` | 保存 |
| `Ctrl+Shift+S` | 全部保存 |
| `Ctrl+W` | 关闭当前标签 |
| `Ctrl+F` | 当前文件查找 |
| `Ctrl+Shift+F` | 在文件中搜索 |
| `Ctrl+Space` | 触发补全 |
| `Tab` | 确认补全 / 缩进 |
| `Esc` | 关闭补全 / 搜索面板 |
| ``Ctrl+` `` | 切换终端面板 |
| `F5` | 运行（cargo run） |
| `F7` | 检查（cargo check） |

---

## 🚀 快速开始

### 使用预编译版本
1. 从 `dist/` 目录获取 `novars.exe`（约 20 MB）
2. 双击运行 —— 就这么简单

### 从源码构建
```bash
git clone <repo-url>
cd NovaRustIDE
cargo build --release
# 产物：target/release/novars.exe
```

> 构建时 build.rs 会自动在 PATH 和 `~/.cargo/bin` 中定位 rust-analyzer 并嵌入；
> 若本机未安装 rust-analyzer，exe 照常构建，补全自动降级为内置引擎。

---

## 🏗️ 技术栈

| 组件 | 说明 |
|------|------|
| [egui](https://github.com/emilk/egui) / [eframe](https://github.com/emilk/egui/tree/master/crates/eframe) | 即时模式 GUI 框架，纯 GPU 渲染 |
| [syn](https://github.com/dtolnay/syn) | Rust 源码 AST 解析（侧边栏） |
| [walkdir](https://github.com/BurntSushi/walkdir) | 文件树 / 项目搜索遍历 |
| [rfd](https://github.com/PolyMeilex/rfd) | 原生文件对话框 |
| [serde_json](https://github.com/serde-rs/json) | LSP JSON-RPC 通信 |
| [image](https://github.com/image-rs/image) | PNG 编解码（帧导出 / 图标） |
| rust-analyzer | 编译期嵌入的 LSP 补全引擎 |

**硬性约束**（项目自始贯彻）：无 WebView/DOM · 无 async/tokio · 无 unsafe · 无 unwrap（全部显式错误处理）· 所有文件操作走 std::fs

---

## 📁 项目结构

```
NovaRustIDE/
├── src/main.rs        # 全部实现（~4600 行）
├── build.rs           # rust-analyzer 定位嵌入 + exe 图标资源
├── assets/Deng.ttf    # 嵌入的中文界面字体（等线）
├── icons/             # SVG 图标源 + ICO + PNG
├── demo_project/      # 验收用示例项目
├── tools/             # 构建/验收辅助脚本
└── dist/novars.exe    # 单文件成品
```

---

## 📄 许可证

本项目基于 [MIT License](LICENSE) 发布。

```
MIT License

Copyright (c) 2026 NovaRS Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
