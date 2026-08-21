# 云端职责与信任边界

> **2026-08-21 后续决策：** 第一阶段默认基础设施现为 Iroh 1.0.3 官方 n0 生产 Relay/QAD 与生产 DNS/Pkarr；`relay.zenithconsulting.cn` 仍可独立部署，但不再是产品默认。本文后续关于“先以自建 Relay/QAD 关闭测量”的文字属于被覆盖的历史方案。

## 结论

用户提出的目标成立：zterm 不需要账号、业务控制平面或云端数据存储；云端只承担跨公网建连所需的网络基础设施，且不能解密设备之间的终端流量。

但“只有 NAT 打洞失败后才使用云端”不符合 Iroh 的常见建连顺序。Iroh 通常先利用端点寻址和 home relay 让双方找到彼此并传递初始 QUIC 握手，再交换可用地址、进行打洞并优先升级为直连；若直连失败或随后失效，业务流量才持续经过 relay。产品语义应写成“数据路径优先直连，中继负责建连引导与失败回退”，而不是“成功打洞时完全不接触云端”。

这里必须区分“云端可能参与建连”与“最终数据路径经过中继”。NAT打洞成功时，选中的业务数据路径是端到端直连；失败或direct失效时，端到端密文才持续由relay转发。QAD只是relay进程可选提供的公网观察地址服务，可能改善部分网络的打洞成功率，但它不参与relay转发，relay fallback也不依赖QAD。当前`relay.zenithconsulting.cn`部署只提供反代后的Relay而关闭QAD/UDP；第一阶段将先以该真实配置测量direct/relay path events，再决定是否有证据支持独立QAD-only服务。

## 官方依据

- [Iroh 官方仓库](https://github.com/n0-computer/iroh)说明端点以公钥作为身份，连接会选择最快路径、尝试打洞并在必要时回退到 relay；QUIC 提供认证加密。仓库同时包含 `iroh-relay` 和为 `dns.iroh.link` 提供 Pkarr 查询的 `iroh-dns-server`。
- [Iroh 1.0 建连流程说明](https://github.com/n0-computer/iroh/discussions/4344)描述了常见流程：端点连接 home relay 并发布 `(endpoint_id, relay_url)`，发起方查询端点、经 relay 完成 QUIC 握手，认证后双方交换 IP 地址并尝试打洞。mDNS 和直接发布 IP 是特定场景的替代方案。
- [`iroh-relay` 服务端源码](https://github.com/n0-computer/iroh/blob/main/iroh-relay/src/main.rs)表明 relay 除数据转发外也可提供 QUIC 地址发现能力；这两项职责在部署上可以配置，但都属于公网建连基础设施。
- [relay 协议源码](https://github.com/n0-computer/iroh/blob/main/iroh-relay/src/protos/relay.rs)显示中继帧带有来源/目标端点标识并承载不透明数据报。因此中继不能看到 QUIC 载荷明文，但能看到连接元数据和密文流量特征。

## 建议的系统边界

```text
设备 A ── 端到端认证加密的 QUIC ── 设备 B
   │             │                  │
   └── 寻址 / 打洞协调 / 密文中继 ──┘
                  云端
```

云端允许承担：

- 端点寻址或路由提示。
- NAT 地址发现和打洞协调。
- 直连建立前的初始握手转发。
- 直连失败或失效后的加密数据包转发。
- 最小化的健康指标、安全审计和故障诊断元数据。

云端禁止承担：

- 用户账号、业务设备目录或授权真相源。
- 私钥、内容密钥或可用于解密会话的材料托管。
- 终端输入输出、会话回放、设备配置或授权关系的持久化。
- 终端明文的检查、索引、分析或服务端处理。

## “不保存数据”的精确定义

relay 在转发时必然会短暂持有内存中的密文数据包，也天然可观察来源 IP、端点公钥/标识、连接时间、流量大小和路径状态。即使关闭所有日志，也无法把这些运行时信息变成“云端不可见”。因此需求应采用可验证的表述：

- **应用载荷不落盘**：不持久化终端或协议内容。
- **明文不可得**：端到端密钥只在设备上，中继不是加密终点。
- **元数据最小化**：定义允许记录的字段、用途、保留期和访问权限。
- **密文可被观察或记录**：恶意或被入侵的中继可以复制密文，但在密码学未被攻破、端点未失陷的前提下不能解密。

## 版本注意事项

Zedra 当前基于 Iroh 0.96，其实现可以证明总体拓扑和已遇到的平台问题，但 Iroh 官方已进入 1.0 代际。zterm 的设计阶段应针对当前 Iroh 1.x API 重新核实 endpoint discovery、relay map、路径观测和自建服务配置，不能直接复制 Zedra 的依赖版本与 API。

截至 2026-08-20，官方最新发布为 Iroh 1.0.3。官方 1.x 文档进一步确认：

- [Use your own relay](https://docs.iroh.computer/add-a-relay)建议生产应用使用专用 relay，并允许通过自定义 relay URL 配置自建 `iroh-relay`。
- [DNS Address Lookup](https://docs.iroh.computer/connecting/dns-address-lookup)说明仅知道 EndpointId 时，需要地址查询把它解析为 home relay URL 或直连地址；自建时客户端必须同时配置指向自有 `iroh-dns-server` 的 `PkarrPublisher` 与 `DnsAddressLookup`，发布端与解析端必须使用同一服务。
- [Address Lookup](https://docs.iroh.computer/concepts/address-lookup)说明 Pkarr 记录由端点私钥签名，默认只需发布 EndpointId 对应的 home relay URL；这类记录属于可验证的建连元数据，不是 zterm 授权数据。
- 上游 [`iroh-relay` 配置源码](https://github.com/n0-computer/iroh/blob/main/iroh-relay/src/main.rs)已经提供 TLS/ACME、QUIC 地址发现、连接/流量限速、metrics，以及 everyone、allowlist、denylist、shared token 和外部 HTTP 判定等访问模式。第零阶段直接封装这些能力，不开发自有 relay 协议。

官方文档明确把 `dns.iroh.link` 作为 Iroh 默认的公共 DNS/Pkarr 服务：端点通过 HTTPS 发布签名记录，其他端点通过 DNS 或 HTTP 查询。官方当前声明该公共服务免费，但会限速且没有可用性保证；官方允许在性能可接受时用于生产，也建议需要容量、SLA 或完全控制时自建。

`dns.iroh.link` 是服务域名/DNS origin，不是固定 IP。它只参与地址查询，不转发终端流量；服务可见 EndpointId、home relay URL、请求来源与查询时序等元数据，但 Pkarr 记录由端点签名，且 zterm 的 QUIC 连接仍以 EndpointId 做端到端身份验证，所以地址查询服务不能解密或冒充终端连接。它仍可通过拒绝、延迟或返回陈旧结果影响可用性。

## Zedra 的实际选择

Zedra 正是“自建 relay、官方地址查询”的组合：

- 宿主构建自定义 relay map，但在非 relay-only 模式调用 `PkarrPublisher::n0_dns()`（`/Users/huyuanzhe/projects/zedra/crates/zedra-host/src/iroh_listener.rs:47-76`）。
- 控制端同样使用自定义 relay map，并调用 `PkarrResolver::n0_dns()`（`/Users/huyuanzhe/projects/zedra/crates/zedra-session/src/connect.rs:472-485`）。
- Zedra 文档明确写明宿主发布到 `dns.iroh.link`，配对二维码因此只需携带 EndpointId，控制端连接时取得最新 home relay（`/Users/huyuanzhe/projects/zedra/docs/NETWORK_TRANSPORT.md:216-242`）。

这说明 zterm 可以只自建 relay 而不自建 `iroh-dns-server`，代价是地址查询仍依赖 Iroh 官方的免费、无 SLA 服务。

## 是否必须使用地址查询

也不是。Iroh 1.0.3 的 [`Endpoint::connect`](https://docs.rs/iroh/latest/iroh/endpoint/struct.Endpoint.html#method.connect)接受带 EndpointId 与 relay URL/直连地址的完整 `EndpointAddr`；只有在仅提供 EndpointId 时，才需要 AddressLookup 补全路由信息。zterm 可以把当前 home relay URL 放入一次性配对票据并在配对后本地保存，从而在单 relay 场景下不依赖 DNS/Pkarr。

不使用地址查询的代价是路由提示可能过期：宿主更换 home relay 后，旧控制端无法只凭稳定 EndpointId 找到新位置。第一阶段只有固定项目 relay 时风险较低；后续多地域自动选择或 relay 迁移时，动态地址查询的价值会显著上升。

## 已确认的部署边界

用户已确认项目方有一台公网服务器，可以通过 Docker 运行并作为 zterm 客户端默认配置的 relay，同时允许用户选择自建 relay。第零阶段先交付单区域、单节点、可重复部署的官方 `iroh-relay` 封装，第一阶段直接用它做网络门禁；地址查询沿用官方免费 `dns.iroh.link`，不部署 `iroh-dns-server`。

这个范围不要求第零或第一阶段自研业务服务器、账号、数据库、控制平面或 relay 协议。第零阶段只交付上游组件的版本固定、Docker 编排、TLS/域名/端口配置、健康与私有 metrics、日志最小化，以及升级回滚说明；第一阶段再接入客户端 profile。多地域、高可用、弹性容量和正式 SLA 延后，但客户端配置应从第一天允许多个 relay URL，并把地址查询配置与 relay 配置解耦。

第一阶段沿用 Zedra 已验证的 `dns.iroh.link`，同时让配对票据和本地设备记录保留 home relay URL 作为官方查询故障时的回退；暂不部署 `iroh-dns-server`。这样最省运维并能取得新鲜路由，代价是会向 Iroh 官方暴露建连元数据且没有地址查询 SLA。若后续要求完全不依赖第三方，则可改为完整 `EndpointAddr` + 固定 relay，或交付自建 `iroh-dns-server`。

用户已明确默认公共 relay 的准入策略：与 Zedra 当前配置一致，采用 `access = Everyone`，省略 `limits`，不使用 shared token、allowlist/denylist 或外部准入回调。健康、容量、流量与成本监控保留，但第一阶段不据此自动限速或封禁。该选择换取无账号、无凭据的开箱可用，同时明确接受非 zterm 流量、滥用与不可预测成本风险；若监控证明风险实际发生，再作为独立需求设计限制策略。
