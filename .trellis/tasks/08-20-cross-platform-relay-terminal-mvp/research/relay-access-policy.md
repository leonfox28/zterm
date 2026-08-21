# Relay 公开准入与滥用控制

## 两层认证不能混淆

Iroh relay 握手中的 EndpointId 签名只证明连接方持有该 Iroh 私钥；relay 准入策略决定是否愿意为这个 EndpointId 转发密文。zterm 自己的配对与 ALPN/RPC 认证才决定某设备能否访问宿主终端。允许陌生 EndpointId 使用 relay 会消耗带宽，但不会自动授予任何 zterm session 权限。

## Zedra 当前实现

Zedra 的生产 relay 是公开准入：

- [`deploy/relay/relay.toml`](/Users/huyuanzhe/projects/zedra/deploy/relay/relay.toml:1)只启用 relay、QUIC 地址发现、metrics 和 TLS，没有 `access`、allowlist、denylist 或外部授权配置。
- Zedra 所用 Iroh relay 配置在省略 `access` 时默认为 `Everyone`，省略 `limits` 时为 `None`，即不启用显式连接/单客户端流量限制（本机 crates.io 源码 `/Users/huyuanzhe/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/iroh-relay-0.96.1/src/main.rs:127-147`、`:294-306`）。
- [`deploy/relay/docker-compose.yml`](/Users/huyuanzhe/projects/zedra/deploy/relay/docker-compose.yml:1)把 80/443/7842 暴露到公网，并部署独立 monitor；metrics/检查 API 仅绑定本地，容器日志有轮转上限。
- Zedra monitor 会采集连接数、收发字节、丢包和 rate-limit 计数并发送告警，但当前 relay 配置没有启用产生这些 rate-limit 事件的显式限额（`/Users/huyuanzhe/projects/zedra/packages/relay-monitor/monitor.ts:97-122`）。

因此 Zedra 与 zterm 选定的“无需 relay 凭据”方向相同；Zedra 当前也没有启用显式限速或 EndpointId 访问限制。

## zterm 第一阶段决定

- 项目默认 relay 接受任意合法 Iroh EndpointId，不要求账号、项目 token 或设备预登记。
- 与 Zedra 一样省略 `limits` 与访问限制配置，不实施连接/带宽限速、allowlist/denylist、shared token 或外部准入回调。
- metrics 只在私网或 localhost 暴露，保留健康、容量、流量和成本观察以及日志轮转，但不自动触发限制。
- 仓库提供的用户自建模板保持同样简单的开放默认；高级用户仍可直接使用上游 `iroh-relay` 自有配置能力，但这不是 zterm 第一阶段承诺的产品功能。
- relay 被运维人员停止时只会中断 transport，不得终止宿主 PTY。

## 取舍

公开且无限制的 relay 让安装后的 zterm 无需账号或凭据即可联网，并完整复用 Zedra 当前可运行模型；代价是项目方承担非 zterm Iroh 流量、滥用和不可预测成本，单个客户端也可能挤占容量。用户明确选择第一阶段接受该风险，以换取最少的后端配置；监控数据若证明风险实际发生，再把限制作为独立需求重新设计。
