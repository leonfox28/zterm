# Phase Zero Implementation Plan

## 1. Z0-A — Local Environment

- 记录 OS/CPU/内存/磁盘和已安装工具版本。
- 确认 Rust 1.98.0、组件与 targets；保留现有 1.95.0。
- 安装缺失的 Docker CLI、Compose、Colima、pkg-config 与固定版本 cargo-deny。
- 启动 Colima 并验证 `docker version`、`docker compose version` 与最小容器。

## 2. Z0-B — Rust Workspace

- 添加精确 toolchain、workspace、统一 lint/profile/dependency 配置；根
  workspace 以 `0.1.0` 作为首个 lockstep 产品版本，五个产品 crate 全部继承。
- 创建五个最小 crate、固定 protobuf schema/build pipeline 和无副作用 CLI 占位入口。
- 添加 README、开发文档、deny 配置与跨平台 CI。
- 运行 fmt、Clippy、test、doc 和 dependency checks。

## 3. Z0-C — Relay Bundle

- 依据 Iroh 1.0.3 官方 release/source 确认 artifact 名称、checksums、CLI 与配置 schema。
- 添加多架构、checksum 强制校验、最小非 root Dockerfile。
- 添加 relay 配置、Compose、`.env.example`、健康检查、日志轮转与回滚文档。
- 添加专用 GitHub Actions 发布工作流，以最小权限和完整 Action commit SHA
  产出 linux/amd64 + linux/arm64 manifest：稳定 release 使用
  `ghcr.io/leonfox28/zterm-relay`，prerelease 与 manual 使用独立的
  `ghcr.io/leonfox28/zterm-relay-dev`；输出完整 image+digest，并验证两个
  package 的 tag/channel 隔离、stable/prerelease canonical SemVer、Git tag
  去 `v` 映射、workspace 完整版本相等、build metadata 拒绝、manual tag
  原样映射/`latest` 拒绝以及 provenance/SBOM。
- 生产 Compose 移除本地 `build` 回退并强制 GHCR digest；保留本地
  Compose/Buildx 作为开发与 CI 质量门禁。
- 保留 direct TLS/ACME 模式，并添加同机 OpenResty 反代模式：只绑定宿主
  `127.0.0.1:38451` 与回环 metrics，不使用 `--dev`、TLS/ACME 或 UDP QAD。
- 添加静态和运行时测试，完成本地多架构 build/Compose smoke test，记录 digest。
- 验证反代模式的普通 HTTP Relay schema、`/relay` WebSocket upgrade 和端口
  暴露面；文档明确 QAD 只是可选地址发现辅助，relay 回退不依赖 QAD，并把
  是否增加 QAD-only 服务留给第一阶段真实 NAT/path-events 测试决定。
- 执行 secret scan，确认没有自定义 relay、monitor 或公网 metrics。
- 将默认服务器部署根路径改为
  `/opt/1panel/docker/compose/zterm-relay`，并明确 9090 是 loopback-only
  Prometheus metrics 而非 relay 流量。

## 4. Mandatory Public-Server Checkpoint

- 汇总本地验收结果并暂停，不连接任何公网服务器。
- 向用户请求安全连接入口/登录与认证机制、relay 域名/DNS 现状、Docker 现状；不请求私钥内容。
- 得到授权后先做只读 preflight，再说明并实施必要变更。
- 对当前反代部署验证公网 DNS/TLS、真实 Relay WebSocket 握手、宿主回环
  38451/9090、私有 metrics 和回滚；明确记录 QAD/UDP 未启用，不开放 UDP
  7842、不修改防火墙，也不把 Relay 部署误报为 NAT 打洞验收。Direct ACME
  模式仍保留 TCP/UDP 完整验收要求。若Cloudflare代理已启用，同时验证
  WebSockets/WAF/Argo配置边界与一次实际断开重连。

## Local Validation Commands

```bash
cargo +1.98.0 fmt --all -- --check
cargo +1.98.0 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.98.0 test --workspace --all-features
cargo +1.98.0 doc --workspace --no-deps
cargo deny check
docker compose -f deploy/relay/compose.yaml config
RELAY_IMAGE=ghcr.io/leonfox28/zterm-relay@sha256:0000000000000000000000000000000000000000000000000000000000000000 \
  docker compose -f deploy/relay/compose.reverse-proxy.yaml config
sh tests/relay/reverse-proxy-smoke.sh
```

Relay 的多架构、checksum、健康与暴露面测试以 `tests/relay/` 中的脚本/测试入口为准。

## Rollback

- 本机工具不自动卸载；停止 Colima 即可释放运行环境，且不删除用户数据。
- Relay 通过 digest/版本化镜像回滚；Compose 配置保留上一份已验证版本。
- 首次稳定 release 发布生产 `zterm-relay` digest 前保留已验证本地镜像作为
  临时 bootstrap 例外；没有真实 workflow 输出时不得编造 registry digest，
  开发 package digest 也不得用于生产，首次稳定发布后再切换并验证回滚。
- 不删除用户已有 Rust toolchain，不改写全局 shell 配置。
