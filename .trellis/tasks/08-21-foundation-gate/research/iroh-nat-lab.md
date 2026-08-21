# Iroh 1.0.3 NAT Gate 与单 VM 网络实验

## 结论

- Relay path 与 direct NAT traversal 是独立路径。自建 `https://relay.zenithconsulting.cn` 的无 QAD fallback 已证明可用，但 2026-08-21 用户决定当前产品 profile 改用 Iroh 1.0.3 官方公共生产基础设施。
- Iroh 1.0.3 `presets::N0` 会让 Relay、DNS/Pkarr 便捷构造一起读取 `IROH_FORCE_STAGING_RELAYS`。zterm 因此从 `presets::Minimal` 开始，使用 Iroh 的 `N0_DNS_PKARR_RELAY_PROD`、`N0_DNS_ENDPOINT_ORIGIN_PROD` 与公开 builder 显式安装生产 lookup，再选择 production `RelayMode::Default`。该版本默认 map 由 US east、US west、EU、AP 四个 n0 Relay 组成；由 `RelayUrl` 转成 `RelayConfig` 时带默认 QUIC address-discovery 配置，因此 QAD 是新产品 baseline 的组成部分。
- n0 官方说明公共 Relay 免费、限速且没有 uptime guarantee，适合开发和测试；正式生产发布前仍需决定托管或自建。当前选择不会删除 Phase Zero 已保留的自建镜像与部署能力。
- Gate 0 不准备两台 Linux VM。使用现有 Colima 0.10.3（Ubuntu 24.04.4、Linux arm64、Docker Engine 29.5.2）承载一个临时特权测试容器；容器内的 endpoint、router、IX 和 NAT 均为短生命周期 namespace。容器删除即回收测试网络，Colima 本身不删除。
- Iroh v1.0.3 自己用 `patchbay 0.6.0` 做 Linux namespace NAT matrix，且官方测试明确支持在 macOS 外包一层 Linux container/VM。zterm 复用同一标准库与 Home NAT 模型，不另写 NAT 类型模拟器。
- 上游 `holepunch_simple` 与当前 N0 baseline 都使用 QAD，但 zterm 仍通过自己的 A/B/C fixture 分别证明自动发现、已知候选打洞和无 UDP 时的 Relay fallback。
- Bettbox 后续已配置 `+.iroh.link` fake-IP filter 与 `DOMAIN-SUFFIX,iroh.link` 直连，本机复核五个官方 DNS/Relay hostname 均解析为真实公网地址。Gate runner 仍保留测试专用 DoH 注入，因为它应独立于个人代理配置，且直连后的 peer UDP 也不一定带 `iroh.link` 域名上下文。

## QAD、地址发现与打洞的边界

Iroh 官方 NAT 文档把流程拆为：先经 relay 建立路径、交换候选地址、双方同时发送 UDP、失败则继续 relay。QAD 文档进一步说明，打洞需要知道 NAT 映射后的地址，而 QAD 的职责是让 endpoint 得到该 reflexive transport address。

Iroh v1.0.3 源码中的 `DirectAddrType` 只有以下来源：

- `Local`
- `Qad`
- `Portmapped`
- `Qad4LocalPort`
- `Config`

Iroh 默认 `portmapper` feature 可通过 UPnP/PCP/NAT-PMP 提供另一类公网候选，但 Patchbay 的 Home NAT 不模拟家庭路由器的这些控制协议。当前 N0 map 自带 QAD，因此 Case A 应能从官方 Relay 获得 reflexive candidate；Case B 仍用于隔离已知 candidate 下的打洞能力。

若官方 QAD 下 Case A 仍只有 local candidate 或始终留在 Relay，Gate 必须记录实际 address/path evidence；不能用 Case B 的手工地址对照冒充产品 profile 已通过。当前结果发生在共享外层 Colima/TUN NAT，用户已批准把它归类为实验环境无法证明 automatic discovery，而不是继续扩建单机实验室；B direct 与 C Relay fallback 允许 Foundation 继续，自动发现成功率在父任务 M10 的两条真实网络补验。

## Gate 实验矩阵

| Case | Endpoint 配置 | NAT/网络条件 | 证明内容 | Gate 语义 |
| --- | --- | --- | --- | --- |
| A 产品基线 | 官方生产 profile + 四个 production Relay + n0 DNS/Pkarr；QAD on；无注入地址 | Home × Home；可访问公网官方 Relay/QAD | 当前嵌套实验室的自动地址发现与 direct upgrade | 成功可证明该实验直连；失败保留原始证据并延期到真实双网络，不能宣称产品自动发现已通过 |
| B 已知候选对照 | 同 A，但测试夹具向双方注入各自 NAT WAN 映射候选 | Home × Home；UDP 可通 | Iroh QNT/holepunch 与 direct path 本身 | 只定位问题在打洞还是候选发现；不进入产品配置 |
| C Relay fallback | 同 A | 只允许 DNS UDP，阻断其他 UDP；保留 HTTPS/WSS 到官方 Relay | Relay path、多条 QUIC stream、E2EE connection 在 direct 不可用时继续 | 必须通过；与 Case A 是否 direct 无关 |

三类实验都收集 `Endpoint::addr()`、候选 `DirectAddrType`、home relay、初始与选中 path、完整 `path_events()` 时间线及多 stream 回显。断言 RelayMap 恰为 Iroh 1.0.3 四个 production host，不能出现 staging 或 `relay.zenithconsulting.cn`。

## 外网接入与清理

Patchbay 的 `Lab` 自身是隔离 IX。测试 runner 在临时特权容器内为该 IX 增加一条外网出口，使 namespace 能访问公网 Relay；所有 route/NAT 规则都存在于临时容器，不写入 macOS、Colima VM 或公网服务器。测试入口负责：

1. 创建唯一命名的临时容器并运行 Gate；
2. 无论成功或失败都删除容器；
3. 结束后断言没有同名 container/network 残留；
4. 不修改官方或自建 Relay、OpenResty、Cloudflare、OCI、UDP 端口或防火墙。

实现时先验证 Patchbay 0.6.0 在该容器内的 egress 接线；如果它不能在不 fork Patchbay 的前提下访问公网 Relay，则回退为同一容器内的最小 `ip netns` 拓扑，而不是创建第二台 VM。此回退只替换实验夹具，不改变 Case A/B/C 的产品判定。

## 证据

- [Iroh NAT traversal 概念](https://docs.iroh.computer/concepts/nat-traversal)
- [Iroh QAD 说明](https://www.iroh.computer/blog/qad)
- [Iroh v1.0.3 `RelayConfig` 与 `RelayQuicConfig`](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh-relay/src/relay_map.rs)
- [Iroh v1.0.3 production default Relay map](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh/src/defaults.rs)
- [Iroh v1.0.3 N0 preset](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh/src/endpoint/presets.rs)
- [Iroh public hosting limits](https://www.iroh.computer/services/hosting)
- [Iroh v1.0.3 direct address 类型](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh/src/socket.rs)
- [Iroh v1.0.3 Patchbay NAT tests](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh/tests/patchbay/nat.rs)
- [Iroh v1.0.3 Patchbay relay helper](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh/tests/patchbay/util.rs)
- [Patchbay 0.6.0](https://github.com/n0-computer/patchbay/tree/patchbay-v0.6.0)
