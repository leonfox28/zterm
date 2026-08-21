# Gate 0 平台与执行环境

## 本地真实平台

- macOS 26.6.2 arm64 是 Gate 0 的主实现与真实 PTY/black-box 平台。
- Docker context `colima` 指向 Linux arm64 Docker Engine 29.5.2；Colima guest 是 Ubuntu 24.04.4、kernel 6.8，已有 `nft`、`tc` 和 user namespace 支持。Guest 的 AppArmor 当前限制普通用户创建 unprivileged user namespace，因此网络 Gate 使用一个 `--privileged --rm` 的临时测试容器，不永久改 guest sysctl。

## CI 平台

GitHub 当前标准 hosted runner 明确提供：

- macOS arm64：`macos-latest`
- macOS x86_64：`macos-15-intel`
- Linux x86_64：`ubuntu-24.04`
- Linux arm64：`ubuntu-24.04-arm`
- Windows x86_64：`windows-latest`

Gate 0 将 workspace 的常规 source policy、format、Clippy、unit/integration test 与 doc 扩到上述 runner。真实 Unix PTY tests 只在 macOS/Linux 执行；Windows 只要求 `zterm-platform` 的 portable-pty/ConPTY 边界可编译，Windows 行为验收仍属于第三阶段。

公网 Relay 和双 NAT Gate 不放进每次 push CI：它依赖无 SLA 的公网服务、需要特权网络容器，而且一次完整 Gate 证据已经有明确消费者。保留显式本地命令供 Iroh 或网络配置变化后重跑，避免把外部可用性变成普通代码提交的随机失败源。

## 证据

- [GitHub-hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
- [GitHub arm64 standard runners](https://github.blog/changelog/2026-01-29-arm64-standard-runners-are-now-available-in-private-repositories/)
