# Clash Verge Rev (FlowCollect Fork) — Agent 宪法

> **Agent 读取指令**：
> 本仓库是 `clash-verge-rev/clash-verge-rev` 的魔改 Fork，核心目的是注入 FlowCollect Sidecar 流量上报能力。
> **接管任何任务前，必须先阅读本文件。**

---

## 1. 项目定位

本 Fork 在上游 Clash Verge Rev（Tauri + Vue 3 桌面客户端）的基础上，注入了 FlowCollect 客户端 Sidecar 的进程生命周期管理。上游保持活跃迭代，本 Fork 通过 `git rebase upstream/main` 跟进。

### 核心数据流

```
用户启动 Clash Verge Rev
  → CoreManager 启动 mihomo 内核
  → 自动启动 flow_collect_client Sidecar（读取 config.yaml 中的 x-flow-collect）
  → Sidecar 通过 WSS 上报流量至 FlowCollect 服务端
  → 关闭时 CoreManager 先停 Sidecar，再停 mihomo
```

---

## 2. 自定义资产清单（Rebase 时必须保护）

以下文件/代码块是本 Fork 的核心魔改资产，**在任何 `git rebase` 操作中必须完整保留**：

| 文件 | 改动类型 | 说明 |
|------|----------|------|
| `src-tauri/src/core/flow_collect.rs` | **新增文件** | `FlowCollectManager` 单例：Sidecar 进程的 start/stop/SIGTERM/is_running 生命周期管理 |
| `src-tauri/src/core/mod.rs` | 修改 | 注册 `pub mod flow_collect` |
| `src-tauri/src/core/manager/lifecycle.rs` | 修改 | CoreManager 中注入：core 启动后 `start_flow_collect()`，core 停止前 `stop_flow_collect()` |
| `src-tauri/src/feat/window.rs` | 修改 | `clean_async()` 中注入 `flow_collect::stop_flow_collect()` 确保退出时清理 |
| `src-tauri/tauri.conf.json` | 修改 | `externalBin` 数组追加 `"sidecar/flow_collect_client"` |
| `src-tauri/tauri.linux.conf.json` | 修改 | `externalBin` 数组追加 `"./sidecar/flow_collect_client"` |
| `scripts/prebuild.mjs` | 修改 | 新增 `resolveFlowCollect()` 任务：从 `FLOW_COLLECT_DIST` 拷贝 FC 二进制到 sidecar 目录 |

**Rebase 冲突处理原则**：遇到上述文件冲突时，优先保留本 Fork 的 FlowCollect 注入逻辑，同时融合上游的新结构。若无法自动推断，停止并等待人类介入。

---

## 3. 绝对禁忌

1. **禁止丢失 `flow_collect.rs`**：此文件是本 Fork 的灵魂，rebase 时绝不能被上游覆盖或删除。
2. **禁止断开 Sidecar 生命周期钩子**：`lifecycle.rs` 中的 start/stop 注入和 `window.rs` 中的 clean_async 清理必须始终存在。
3. **禁止移除 externalBin 注册**：`tauri.conf.json` 和 `tauri.linux.conf.json` 中的 `flow_collect_client` 条目不可删除。
4. **禁止硬编码服务端地址**：Sidecar 连接信息从 Clash 配置文件的 `x-flow-collect` 字段读取。
5. **禁止在本仓库中修改 FlowCollect 服务端代码**：服务端代码位于独立的 `FlowCollect` 仓库。

---

## 4. Git 工作流

### Rebase 策略

```bash
git fetch upstream
git rebase upstream/main
# 冲突解决后：
git add . && git rebase --continue
```

### 提交规范

- Commit Message 遵循 Angular 规范：`feat(scope): description`
- AI Agent 提交必须携带身份标识：
  ```bash
  git commit --author="Claude AI <claude@anthropic.com>" -m "feat(scope): description"
  ```
- Fork 特有提交使用 `feat(fc-client):` 或 `fix(fc-client):` 前缀

### Force Push

Rebase 完成后必须 force push：
```bash
git push origin HEAD --force
```

---

## 5. 构建依赖

| 工具 | 用途 |
|------|------|
| Rust / Cargo | Tauri 后端编译 |
| Bun / pnpm | Vue 前端依赖安装与构建 |
| `FLOW_COLLECT_DIST` 环境变量 | 指向 FlowCollect 客户端二进制目录，`prebuild.mjs` 用它拷贝 sidecar |

---

## 6. 与 FlowCollect 生态的关系

```
FlowCollect (核心仓库)
  ├── server/          → Go 服务端（NAS 部署）
  ├── client/          → Go Sidecar 源码（编译出 flow_collect_client 二进制）
  ├── smart_spend/     → Vue 3 前端仪表盘
  └── 本 Fork (下游消费)
      └── clash-verge-rev → 桌面客户端，集成 FC Sidecar
```

FlowCollect Release CI 自动编译所有平台的 `flow_collect_client` 二进制。本 Fork 的 `prebuild.mjs` 在构建时从 `FLOW_COLLECT_DIST` 目录拷贝对应架构的二进制到 Tauri sidecar 目录。
