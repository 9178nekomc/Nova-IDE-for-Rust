<div align="center">

# 🦀 NovaRS v2.0 (Super Upgrade Edition)

**Nova IDE for Rust v2.0.2 — 模块化 AI 原生 Rust 集成开发环境**

纯 Rust 原生实现 · 模块化标准架构 · 内置 AI Agent 副驾驶 · 内置 MCP 协议服务 · IDEA 级实时报错预检 · 大文件无截断引擎

`egui + eframe + syn + walkdir + serde_json` · 无 WebView · 无 Electron · 零运行时冗余依赖

</div>

---

## 目录

- [快速开始](#快速开始)
- [v2.0 核心进化](#v20-核心进化)
- [架构与目录](#架构与目录)
- [MCP 服务与鉴权](#mcp-服务与鉴权)
- [快捷键](#快捷键)
- [测试与质量门禁](#测试与质量门禁)
- [安全模型与已知限制](#安全模型与已知限制)
- [许可证](#许可证)

---

## 快速开始

### 环境要求

| 项 | 要求 |
|----|------|
| Rust 工具链 | `stable` ≥ 1.77（开发验证于 1.98.1） |
| 目标平台 | Windows 10/11 x64（主目标）；macOS / Linux 走同一 eframe 后端 |
| 链接器 | MSVC 工具链需 VS Build Tools；GNU 工具链需 MinGW-w64 |
| 可选 | `rust-analyzer`（位于 PATH 或 `%USERPROFILE%\.cargo\bin`，构建期会被嵌入） |
| 可选 | `curl`（远程模型 API 走系统 curl 发起 HTTPS 请求） |

### 构建

```bash
git clone https://github.com/9178nekomc/Nova-IDE-for-Rust.git
cd Nova-IDE-for-Rust
cargo build --release
# 产物：target/release/novars.exe（Windows）/ target/release/novars
```

首次构建约 3–5 分钟（egui/eframe 依赖链），增量构建约 20 秒。

### 运行

```bash
cargo run --release
# 或直接运行产物
./target/release/novars.exe
```

启动后先显示 520×340 无边框启动动画，随后切换为主窗口（1400×860）。若需调整启动动画时长（例如自动化截图），设置环境变量：

```bash
NOVARS_SPLASH_SECS=0.5 ./target/release/novars.exe   # 取值 (0, 3600)
```

### 首次使用建议

1. `Ctrl+O` 打开一个 Cargo 工程目录（后续启动自动恢复）。
2. `Ctrl+Shift+I` 展开 AI 侧栏，在「⚙ 模型管理」中填入 API Key；不填也可用内置离线 Rust 专家引擎验证修复流程。
3. 在「🩺 环境检测」中一键补齐 `rust-analyzer` / `clippy` / `rustfmt` 组件，必要时写入国内 crates 镜像。

---

## v2.0 核心进化

### 1. 🏗 标准化、模块化工程架构

原本 4,600+ 行的单文件 `src/main.rs` 按现代 Rust 工业标准拆分：**15 个顶层模块 + `editor/` 与 `ai/` 两个子模块组，共 27 个源码文件**（`src/app.rs` 仍承担界面编排，其余按职责切分）。

### 2. 🔧 编辑器顽疾根治

- **4000~5000+ 行行号截断**：根因是整份文本一次性塞进单个 `egui::TextEdit` Galley，行数超阈值后高度突破 GPU 纹理与单 Mesh 顶点上限。现采用**动态自适应数字位宽** + **大文件滑动视口窗口化渲染**（`LARGE_FILE_LINE_THRESHOLD = 2500`，窗口 1200 行），编辑后按字节区间无缝回写全文。
- **VS Code 级缩进辅助线**：空白行按前后非空邻居插值（Lookbehind/Lookahead），跨越空行不断线；支持当前代码块激活高亮，并自动避开 `{}` 首字符。
- **编辑区高度与行号基线（v2.0.1 修复）**：
  - `Ui::horizontal` 只为子 Ui 申请 `spacing().interact_size.y`（默认 18px）行高，子 Ui 内的 `available_height()` 并不等于面板剩余高度。行号栏与编辑器若照此取值，整块编辑区会塌缩成一条（实测 120px）。现先取父 Ui 真实可用尺寸，再用 `set_height` 显式撑满整行。
  - `Fonts::row_height()` 与 Galley 实际排版行高可能相差零点几像素（当前字体为 15.71 vs 实测 16.00）。上千行后累计成数十行错位，行号与代码对不上。现改为**以 Galley 实测行高为唯一基准**，行号栏、缩进线、诊断线、行跳转共用同一 `row_h`。
  - 行号栏与迷你地图消费的是上一帧的滚动偏移，egui 按需重绘时可能不再产生下一帧、把错位定格。现在偏移或行高变化时主动 `request_repaint` 补一帧。

### 3. 🔍 IDEA 风格实时预检与一键修复

- 打字瞬间用 `syn::parse_file` 做 AST 语法级预检，叠加启发式检查（未闭合定界符、缺分号、`.unwrap()` 崩溃风险、`todo!()`），并融合后台 `cargo check --message-format=json` 的编译器诊断。
- 行内波浪线 + 行尾 Error Lens + `◀/▶` 错误导航；选中报错后可「🤖 让 AI 修复」或「⚡ 快速修复」，修改先进入预览态，由绿色对比横幅决定 **接受** 或 **撤回**。

### 4. 🤖 AI Agent 副驾驶

- 多协议：OpenAI 兼容（DeepSeek / OpenRouter / vLLM）、Anthropic Messages、Google Gemini、Ollama 本地。
- Codex 风格 5 档思考强度滑块（MIN / LOW / MED / HIGH / xHIGH），映射为 `reasoning_effort` 或 `thinking.budget_tokens`。
- 多会话标签、工作区上下文附着、`novars-patch` 行级补丁协议、`novars-mcp` 工具调用协议。
- 三级沙箱：**严格逐步确认** / **工作区均衡**（只读放行、写与命令需确认）/ **YOLO 全自动**，三者统一由 `verify_workspace_jail` 强制约束在工作区内。

### 5. 🔌 内置 MCP 服务

标准 JSON-RPC 2.0 over HTTP，暴露 `read_file` / `write_file` / `apply_diff` / `list_files` / `search_code` / `get_diagnostics` / `get_ast_outline` / `run_cargo`。详见 [MCP 服务与鉴权](#mcp-服务与鉴权)。

### 6. 🖥 集成终端

多标签、`↑/↓` 历史回溯、ANSI 控制序列剥离、强制 UTF-8 编码、快捷指令芯片（`cargo check/run/test/clippy`、`git status`）。日志缓存上限 256 KB 并按 UTF-8 字符边界安全裁剪。

### 7. 🛠 环境、依赖、主题、图标与插件

- 环境检测中心：`rustc`/`cargo`/`rustup`/`rust-analyzer`/`clippy`/`rustfmt` 与链接器检测、组件一键安装、`stable/beta/nightly` 频道切换、RsProxy/TUNA/USTC 稀疏索引镜像写入。
- 依赖管理：解析并编辑 `Cargo.toml`，增删依赖、配置 `features`、切换 Edition、改版本号、12 款精选库一键引入。
- 8 套主题（VS Code Dark+ / One Dark Pro / Dracula / Tokyo Night / Catppuccin Mocha / Monokai Pro / Solarized Light / GitHub Light）。
- 30+ 文件格式专属彩色徽章；插件清单加载（`plugins/*.json`）。

### 8. 🌐 网络与代理语义（v2.0.2 修正）

模型请求统一走系统 `curl`，代理设置与面板选项严格对应：

| 面板选项 | curl 行为 |
|---------|-----------|
| 直连 (无代理) | `--noproxy *`，**完全忽略** `HTTP(S)_PROXY` / `ALL_PROXY` 环境变量 |
| 跟随系统环境变量 | 读取环境变量代理，并按 `no_proxy` 名单放行直连 |
| HTTP / SOCKS5 代理 | 显式 `-x`（缺 scheme 时自动补 `http://` 或 `socks5://`），本地回环始终直连 |

修正前的实现只在「显式代理」时传 `-x`，其余情况 curl 仍会读取环境变量：系统里跑着 Clash/VPN 时，
即便选了「直连」，请求也会被送进代理；对 `127.0.0.1` 的本地模型（Ollama）尤其致命——代理会返回
`502 upstream connect failed` 或空包，而空包在旧实现里只会表现为 `EOF while parsing a value at
line 1 column 0`。

配套改进：

- 每次请求使用独立临时文件（`novars_req_<pid>_<seq>_<nanos>.tmp`），侧栏对话与「AI 修复」并发时不再互相覆盖。
- 响应体改由 curl `-o` 写文件、`-w "%{http_code}"` 回传状态码，因此 **4xx/5xx 与空包都能拿到状态码与报文摘要**。
- 错误信息统一附带模型名、脱敏后的 URL（去掉 `?key=...` 与代理账号密码）与可执行建议；
  常见 curl 退出码（6/7/28/35/60）会翻译成具体排查方向。

---

## 架构与目录

```text
Nova-IDE-for-Rust/
├── Cargo.toml                  # 依赖与包元数据
├── build.rs                    # 编译期定位并嵌入 rust-analyzer + Windows 图标资源
├── assets/Deng.ttf             # 内嵌中文字体（保证跨机渲染一致）
├── icons/                      # SVG + ICO + 512px PNG
├── plugins/sample-profiler.json# 插件清单示例
└── src/
    ├── main.rs                 # 入口：视口初始化、图标解码
    ├── app.rs                  # 应用状态机、菜单、多面板布局、快捷键调度
    ├── constants.rs            # 全局常量、品牌色、虚拟化阈值
    ├── config.rs               # 持久化配置、代理、模型档案、沙箱层级
    ├── theme.rs                # 8 套主题引擎
    ├── icons.rs                # 文件类型图标与徽章
    ├── utils.rs                # UTF-8 安全切片、ANSI 剥离、缩进、小写偏移映射
    ├── lsp.rs                  # rust-analyzer JSON-RPC 客户端 + 降级补全
    ├── ast.rs                  # syn AST 大纲
    ├── terminal.rs             # 多标签集成终端
    ├── toolchain.rs            # 工具链检测与镜像配置
    ├── deps.rs                 # Cargo.toml 可视化依赖管理
    ├── project.rs              # 脚手架模板与 Debug/Release 运行器
    ├── mcp.rs                  # MCP HTTP 服务端（鉴权 / 审计 / 工具分发）
    ├── plugins.rs              # 插件与钩子
    ├── editor/
    │   ├── mod.rs              # 编辑器标签状态
    │   ├── gutter.rs           # 动态宽度行号栏
    │   ├── indent_guides.rs    # 缩进参考线
    │   ├── syntax.rs           # 语法高亮 + 迷你地图
    │   └── diagnostics.rs      # 实时诊断与 AI 修复预览
    └── ai/
        ├── mod.rs              # AI 侧栏 UI 与状态
        ├── client.rs           # 多协议 HTTP 客户端
        ├── models.rs           # 模型档案管理
        ├── prompts.rs          # 系统提示词与补丁协议
        ├── sessions.rs         # 多会话
        ├── codex_slider.rs     # 思考强度滑块
        └── sandbox.rs          # 工作区越界校验（单一收口点）
```

---

## MCP 服务与鉴权

默认监听 `http://127.0.0.1:18765/mcp`，可在「🔌 MCP」面板启停或改端口。

### 鉴权（v2.0 起强制）

服务每次启动会用操作系统 CSPRNG（Windows `BCryptGenRandom`，类 Unix `/dev/urandom`）生成 32 字节随机令牌，以 64 位十六进制显示在 MCP 面板中。外部客户端必须携带：

```http
Authorization: Bearer <token>
```

也接受 `X-NovaRS-Token: <token>`。此外：

- 携带 `Origin` 头的请求一律返回 `403` —— 合法 MCP 客户端是原生进程，出现 `Origin` 即意味着浏览器发起的跨站请求（防"任意网页驱动本机 IDE 读写文件"）。
- 响应不返回任何 CORS 头。
- 令牌校验使用定长时间比较，避免逐字节计时侧信道。
- 只接受 `POST /mcp`，只支持 `Content-Length` 定长报文（`Transfer-Encoding: chunked` 显式返回 `411`），报文头上限 64 KiB、正文上限 8 MiB。

### 客户端配置示例

```json
{
  "mcpServers": {
    "novars-ide": {
      "url": "http://127.0.0.1:18765/mcp",
      "headers": { "Authorization": "Bearer <启动后从 MCP 面板复制的令牌>" }
    }
  }
}
```

### 协议行为

| 方法 | 行为 |
|------|------|
| `initialize` | 返回 `protocolVersion: 2024-11-05`、`serverInfo`、`capabilities.tools` |
| `notifications/*`（无 `id`） | 返回 `202 Accepted`，**不返回响应体**（符合 JSON-RPC 通知语义） |
| `tools/list` | 每个工具均带 `inputSchema`（JSON Schema） |
| `tools/call` | 结果统一为 `content[].text` + `isError` 布尔，错误不抛 HTTP 异常 |
| 未知方法 | `-32601 Method not found` |
| 非法 JSON | `-32700 Parse error` |

### 工具约束

- 所有文件操作经 `verify_workspace_jail`：词法规范化折叠 `.`/`..`、按**路径分量**比较（不是字符串前缀，故 `C:\ws-evil` 不会冒充 `C:\ws`）、并对已存在的最近祖先做真实路径校验（阻断软链接/目录联接外逃）。
- `run_cargo` 只允许 `check / test / clippy / fmt / build` 白名单。
- `read_file` 单次最多 5,000 行；`search_code` 单文件上限 4 MiB、最多 50 条命中；`list_files` 最多 5,000 条并按路径分量剪枝 `target`/`.git`/`node_modules`。
- 审计日志保留最近 500 条（面板内可清空）。

---

## 快捷键

| 快捷键 | 功能 |
|--------|------|
| `Ctrl+N` / `Ctrl+O` | 新建文件 / 打开项目文件夹 |
| `Ctrl+S` / `Ctrl+Shift+S` | 保存当前 / 全部保存（触发后台 Cargo 诊断） |
| `Ctrl+W` / `Ctrl+F` / `Ctrl+Shift+F` | 关闭标签 / 文件内查找 / 全项目搜索 |
| `Ctrl+Space` | 触发智能补全 |
| `Tab` / `Shift+Tab` | 确认补全 / 缩进与反缩进 |
| `Ctrl+Shift+I` | 展开/收起 AI 副驾驶 |
| ``Ctrl+` `` | 展开/收起集成终端 |
| `F5` / `Shift+F5` | 运行 / 带 `RUST_BACKTRACE=full` 调试运行 |
| `F7` | `cargo check` |

---

## 测试与质量门禁

```bash
cargo build --release     # 必须零 error、零源码 warning
cargo test                # 21 个单元/集成用例
cargo clippy --release    # 仅剩风格级建议（无 correctness 级告警）
```

测试覆盖范围：

- `ai::sandbox` —— 工作区越界：`..` 逃逸、绝对路径、同前缀兄弟目录、`./` 归一、工作区内部 `..` 合法消解、无工作区拒绝。
- `mcp::tests` —— 令牌熵与格式、定长时间比较、报文边界查找、构建目录剪枝。
- `mcp::http_tests` —— 真实 TCP 端到端：无令牌 `401`、错令牌 `401`、带 `Origin` `403`、`initialize` 成功、通知返回 `202` 空体、`tools/list` 全部带 `inputSchema`、`write_file`→`read_file` 往返、越界写被拦截、**200 KiB 正文完整收发**（回归旧实现 64 KiB 截断缺陷）、未知方法 `-32601`。

GUI 冒烟：以 `NOVARS_SPLASH_SECS=0.6` 启动产物，确认进程存活、主窗口渲染、MCP 端口 `LISTENING`，并实测未鉴权请求返回 `401`、带 `Origin` 返回 `403`。

---

## 安全模型与已知限制

已实施：

- MCP 服务仅绑定 `127.0.0.1`，强制 Bearer 令牌，拒绝浏览器源，无 CORS 头。
- AI 与 MCP 的所有文件写入统一经工作区沙箱校验。
- 子进程一律以 `CREATE_NO_WINDOW` 启动，不弹控制台。
- 终端日志、审计日志、搜索命中数均有硬上限，避免长跑内存无界增长。
- 全文查找在大小写折叠改变字节长度时使用偏移映射（避免非字符边界切片 panic）。

**已知限制（明确记录，不隐瞒）**：

1. **模型 API Key 以明文存储在 `%APPDATA%\NovaRS\settings.json`**。当前依赖栈未引入 DPAPI/Keychain/Secret Service 封装，属于已知短板；在共享机器上请勿保存密钥。后续若接入 `keyring` 类凭证后端会一并迁移为密文。
2. **exe 资源图标可能缺失**：`build.rs` 通过 `winresource` 调用 `rc.exe`；若构建机缺少 Windows SDK 或安全策略禁止其查询注册表，则跳过资源图标（仅 warning，不影响功能）。运行时窗口图标由 `ViewportBuilder::with_icon` 独立设置，始终正常。
3. **MCP 未实现 `Mcp-Session-Id` / SSE 流式传输**，仅支持请求-响应式工具调用；依赖流式通知的客户端需降级使用。
4. **`get_diagnostics` 会真实执行 `cargo check`**，首次调用在该工程冷编译时可能耗时数十秒。
5. 嵌入 `rust-analyzer` 会显著增大产物（当前约 37 MB）；若不需要，可从 PATH 中移除 `rust-analyzer` 后重新构建，`build.rs` 会自动降级为内置补全引擎。

---

## 许可证

本项目基于 [MIT License](LICENSE) 开源发布。
