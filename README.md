# ControlVPS

ControlVPS 是一款**本地优先（Local-First）**、**无 Agent（Agentless）**的跨平台 VPS 运维桌面应用，基于 Tauri 2、React 19、TypeScript 与 Rust 构建。通过标准 SSH/SFTP 连接直接管理服务器资产、交互式终端、容器、监控告警、自动化 Runbook、加密备份与本地 MCP AI 接入。

## 核心特性

- **资产与连接路由**：管理多台 VPS、分组默认配置、共享认证身份（`AuthProfile`）、最多 4 跳多级跳板链，以及 DigitalOcean / Tailscale 只读云资产同步与 `~/.ssh/config` 导入。
- **交互式终端与录制**：内置 xterm.js SSH 终端工作区与 asciicast v2 会话录制/回放（仅记录输出，不落盘原始键盘输入，文件权限 `0600`）。
- **SFTP 文件与传输队列**：远程目录浏览、原子替换上传/下载、断点续传队列与多文件并发传输。
- **监控与安全审计**：基于 SSH 的主机 CPU / 内存 / 磁盘 / 网络采样与本地告警、HTTP/TCP/DNS/TLS 服务可用性巡检，以及只读系统补丁、监听端口、防火墙与 SSH 配置安全审计。
- **容器与服务管理**：Docker 容器与 Docker Compose 应用部署/回滚（含远端锁与环境密钥脱敏）、systemd 服务状态与日志查看。
- **批量运维与 Runbook**：多机并发命令、灰度发布（Canary）、单步超时、本地计划调度与持久化执行历史。
- **SSH 隧道与数据保护**：本地转发、远程转发与动态 SOCKS5 代理；基于 Restic 的目录与数据库流式快照备份（零明文临时文件、凭据经标准输入传递）；本地工作区 Argon2id + AES-256-GCM 加密导出与恢复。
- **本地 MCP 服务（AI 接入）**：内置仅监听 `127.0.0.1` 的 Streamable HTTP MCP 端点，默认只读模式，支持 Bearer 令牌轮换与 DNS Rebinding / Origin 防护，可直接接入 Claude Code、Cursor 等客户端。

## 安全模型

- **凭据隔离**：SSH 密码、私钥口令、云平台只读令牌、Restic/S3 凭据与 MCP Bearer 令牌均保存于操作系统钥匙串（macOS Keychain / Windows Credential Manager / Linux Secret Service），不以明文落盘 SQLite。
- **主机密钥校验**：独立维护应用级 `known_hosts`（原子写入 + `0600` 权限），首次连接需人工确认 SHA-256 指纹，指纹变化立即拒绝连接。
- **敏感信息保护**：远端敏感环境变量通过 SSH `stdin` 注入，不出现在远端进程命令行参数（`ps`）中；日志与输出在入库前经过脱敏处理，导出备份的密钥缓冲区在释放前清零。

## 目录结构

```text
.
├── src/                    # React 19 + TypeScript + Tailwind CSS 前端
│   ├── components/         # 页面与业务组件（非首页按路由懒加载）
│   ├── hooks/              # 监控、主机信任队列、应用更新等 Hooks
│   └── lib/                # Tauri IPC 封装、格式化、通知工具
├── src-tauri/              # Rust + Tauri 2 后端
│   └── src/                # SSH、终端、隧道、SFTP、DB、MCP、安全审计、备份等模块
└── .github/workflows/      # GitHub Actions CI（前端构建、Clippy、单元测试）
```

## 本地开发

### 环境要求

- Node.js 22+
- Rust 1.85+（含 `cargo` 与 `clippy`）
- 对应平台的 Tauri 2 系统依赖（macOS Xcode Command Line Tools / Linux `webkit2gtk-4.1` 等）

### 常用命令

```bash
# 安装前端依赖
npm ci

# 启动桌面应用开发模式
npm run tauri:dev

# 仅启动前端预览（无桌面后端，运行在只读模拟模式）
npm run dev

# 前端类型检查与生产构建
npm run build

# 后端静态检查与单元测试
cd src-tauri
cargo clippy --all-targets -- -D warnings
cargo test

# 打包发布版本
cd ..
npm run tauri:build
```

## 许可证

本项目基于 [GNU General Public License v3.0](LICENSE) 开源。
