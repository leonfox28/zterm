# GitHub 安装、版本选择与身份卸载生命周期

## 用户补充需求

2026-08-21 用户补充确认：

1. installer 可以托管在 GitHub；
2. 无参数默认安装最新稳定版，也能精确指定稳定版或开发版；
3. 桌面平台持久配置统一放在当前用户 home 的 `.zterm` 目录；
4. 卸载后正常重装必须成为新设备，旧配对不能直接复用。

## GitHub 发行能力

GitHub Release 足以同时承担 bootstrap 脚本、版本 manifest、签名、checksum 与四个平台 artifact：

- GitHub 提供 `/releases/latest` 和 `/releases/latest/download/<asset>` 形式的最新 release 链接：[Linking to releases](https://docs.github.com/en/repositories/releasing-projects-on-github/linking-to-releases)。
- REST latest release 明确定义为最近的 non-prerelease、non-draft release，因此它适合作为无参数的稳定默认值；开发版标记为 prerelease 后不会进入这条路径：[REST API endpoints for releases](https://docs.github.com/en/rest/releases/releases#get-the-latest-release)。
- Release 是 tag 对应的可部署版本并可以附加二进制资产：[About releases](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases)。
- GitHub Actions artifact 有 retention 上限，公共仓库最长 90 天，不适合作为“以后仍可精确指定”的开发版本来源：[artifact retention](https://docs.github.com/en/organizations/managing-organization-settings/configuring-the-retention-period-for-github-actions-artifacts-and-logs-in-your-organization)。

建议的 CLI 契约：

```sh
# latest non-prerelease stable
curl -fsSL https://raw.githubusercontent.com/<owner>/zterm/main/install/install.sh | sh

# exact stable
curl -fsSL https://raw.githubusercontent.com/<owner>/zterm/main/install/install.sh | sh -s -- --version v1.0.2

# exact signed development prerelease
curl -fsSL https://raw.githubusercontent.com/<owner>/zterm/main/install/install.sh | sh -s -- --version v1.1.0-dev.20260821.abc1234
```

开发版不是任意 commit 或 branch head。CI 只有在创建 GitHub prerelease、发布四个平台完整 assets、签名 manifest、checksum、SBOM 与 provenance 后，才产生可由用户指定的开发版本。`zterm update` 采用相同选择规则：无参数 stable，`--version` 精确选择；不存在后台检查、自动 nightly 或自动安装。

installer 的 raw `main` URL 可作为短小、向后兼容的 bootstrap 入口，但它是可变内容。文档还需提供“下载脚本后审阅”和“从 immutable versioned Release 手工验签安装”两条路径，不能把 HTTPS + checksum 描述为脚本源失陷后的独立防护。

## Zedra 证据

Zedra 证明了 GitHub 托管和版本选择的基本形态：

- `scripts/install.sh:1-4` 给出 raw GitHub one-liner 与 `--version` 示例；
- `scripts/install.sh:84-106` 无参数跟随 `/releases/latest`，有参数直接采用指定 tag；
- `scripts/install.sh:169-190` 从对应 GitHub Release 下载目标 artifact 与 checksum；
- `crates/zedra-host/src/identity.rs:15-87` 持久化 Iroh `SecretKey`，由其公开部分得到 EndpointId，并用 Ed25519 签名 challenge；
- `crates/zedra-host/src/client.rs:60-99` 为客户端持久化独立 Ed25519 signing key；
- `crates/zedra-host/src/uninstall.rs:13-77,94-139` 在删除前确认、停止 daemon、删除本地 state 和 binary，并明确警告 identity keys 会丢失、paired devices 必须重新扫码。

zterm 可以沿用“持久私钥 + 删除 state 后重新配对”的安全模型，但要加强两点：所有 release（包括开发 prerelease）必须强制验签/checksum；卸载的产品语义固定为完整删除 `~/.zterm/`，而不是让用户无意保留身份。

## zterm 身份模型

E2EE 不需要一个由云端签发的 bearer token。每个 zterm 安装在 `zterm setup` 时生成一份长期 Iroh Ed25519 `SecretKey`：

- 私钥只存在本机 `~/.zterm/identity.key`，Unix 权限 `0600`；
- 公开 EndpointId 是设备的稳定网络身份；
- pairing ticket 只是一份短时、一次性的授权引导秘密，不是长期 token；
- 宿主在 `device_auth` 中保存 controller EndpointId；controller 用私钥签名 challenge 证明持有该身份；
- Iroh/QUIC 在每次连接上再建立临时流量密钥，relay 只转发端到端密文。

installer 本身不生成密钥，因为用户可能只想下载/检查 binary。`zterm setup` 才创建 `~/.zterm/`、身份和数据库。

## 卸载与“失效”的精确保证

官方 `zterm uninstall` 在任何删除前完整显示影响并确认；非交互必须显式 `--yes`，存在活动 session 时还需 `--force`。执行顺序是停止 daemon/PTY、删除整个 `~/.zterm/`、最后删除 binary，且中途失败可重试。

这能保证：

- 本机不再持有旧私钥，正常重新安装/setup 会产生新的 EndpointId；
- 若卸载的是 host，它的本地授权数据库也被删除，旧 controllers 不能连接新 host identity；
- 若卸载的是 controller，新安装没有旧 controller 私钥，无法使用远端宿主中残留的旧授权；
- 因此在没有复制/备份旧私钥的正常路径里，所有方向都必须重新配对。

但“删除私钥”不是可向全网广播的吊销事件。没有账号或控制平面时：

- 远端宿主不会自动删除对旧 EndpointId 的 stale authorization；
- 如果攻击者在卸载前复制了旧私钥，该副本仍能证明旧 EndpointId；
- SSD、快照与备份也使“法证级安全擦除”无法由普通文件删除保证；
- 真正处理泄露密钥仍需在每台曾授权它的宿主上分别 revoke。

若产品要求“即使旧私钥已有副本，卸载一次也要立刻全局失效”，就必须引入在线撤销控制平面、全局账户/设备目录，或保证逐宿主收到并持久化撤销。这与当前“云端只做地址查询/NAT 协调/密文 relay、无中央授权”的边界冲突，是需要用户明确选择的产品取舍。

## 为什么“删掉密钥”不自动等于“远端已撤销”

当前 SSH-like 模型中没有 CA 签发的证书。B 配对 A 后，A 本地保存的是 `allow(B_public_key)`；以后 B 只需用 `B_private_key` 对 challenge 签名，A 验证签名并查询自己的授权数据库。

如果 B 卸载时只删除 `B_private_key`：

- 正常重装后的新密钥无法生成 B 的旧签名，所以连接失败；
- 但 A 的本地授权数据库并不知道 B 是否真的删掉了私钥；
- 如果旧私钥已有副本，该副本生成的签名与原设备完全相同，A 从网络上无法区分“原件”和“副本”。

这与 SSH 相同：删除笔记本上的 `~/.ssh/id_ed25519` 不会自动删除服务器 `authorized_keys` 中的公钥。所谓证书撤销也不是让证书文件发生物理变化，而是让验证方查询 CA 的 CRL/OCSP 或其他撤销记录后拒绝它。

在不增加中央控制平面的前提下，zterm 可以提供一个更符合用户直觉的 best-effort 流程：

1. 卸载前遍历 `known_devices`；
2. 对所有当前可达、曾授权本设备的宿主发送认证的 `RevokeSelf`；
3. 宿主持久化 revoked tombstone、关闭该设备现有 connection，并返回 receipt；
4. 卸载器列出已确认撤销与当前不可达的宿主；
5. 用户确认后删除本机身份和程序。

收到并持久化 `RevokeSelf` 的宿主以后会拒绝旧私钥副本。这不依赖账号或中央云端，但无法保证离线、已重装或地址不可达的宿主收到消息。若必须对这些宿主也自动生效，需要一个它们在下次认证前强制查询的持久撤销服务，或以后由另一台可信设备逐宿主完成撤销。

## Zedra 当前实际行为（`a30bc6c`）

本轮重新核对本地 `/Users/huyuanzhe/projects/zedra` 的干净工作树与提交 `a30bc6c69d812afacbe0e1fb6ad4d25665d4030e`，结果是 Zedra 没有实现终端配对的全局撤销或卸载 `RevokeSelf`：

1. 宿主身份是 `~/.config/zedra/identity.key` 中的长期 Iroh Ed25519 SecretKey；公开部分是 EndpointId（`docs/ARCHITECTURE.md:42-47`、`crates/zedra-host/src/identity.rs:15-87`）。
2. 手机 App 在应用数据目录的 `zedra/client.key` 保存长期 Ed25519 client key（`crates/zedra/src/workspaces.rs:708-720`）。
3. 首次扫码用 pairing handshake HMAC 注册 client public key；宿主把它加入全局 `authorized_clients` 和 session ACL，并持久化到 workspace `sessions.json`。源码把该集合直接称为 SSH `authorized_keys` 等价物（`crates/zedra-host/src/session_registry.rs:682-723,828-865,1088-1133`）。
4. 重连时宿主先检查 client public key 仍在本地全局授权集合，再用该公钥验证 challenge 的 Ed25519 签名（`crates/zedra-host/src/rpc_daemon.rs:1965-1969,2105-2155`）。
5. `ZedraProto` 的完整终端 RPC 枚举以 `HostWorkspaceOpen` 结束，没有 device revoke、unpair 或 `RevokeSelf` variant（`crates/zedra-rpc/src/proto.rs:21-269`）。App 的“移除 workspace”只断开 transport 并删除本地保存的 workspace 条目，不修改宿主 ACL（`crates/zedra/src/workspaces.rs:622-673`、`crates/zedra/src/workspace.rs:1728-1732`）。
6. `zedra uninstall` 只在本机确认、停止 daemon、删除 state 目录和 binary；提示文字明确说明 identity keys 丢失后 paired devices 必须重新扫码，但没有网络调用或远端撤销步骤（`crates/zedra-host/src/uninstall.rs:13-149`、`README.md:55-61`）。

Zedra 中名为 `stack revoke` 的功能属于 Delta 云服务 node ability/grant，不是 Iroh 终端配对公钥撤销，不能拿来证明卸载时会通知远端。

因此 Zedra 的可观察语义是：

- 卸载宿主并删除 state 后，重装得到新 host EndpointId 和空 ACL，所有手机需要重新扫码；
- 卸载手机 App 后，OS 删除 app data/client key，重装产生新 client key，旧宿主虽可能仍保存旧公钥，但新 App 无法证明它，所以也要重新扫码；
- 如果旧私钥在删除前被复制，Zedra 没有中央 revocation list 或 peer revoke RPC 阻止该副本继续使用宿主中残留的授权。

这正好满足用户最初要求的“正常卸载重装不能直接复用旧配对”，但不提供更强的“已复制私钥也全局失效”保证。对 Android/iOS 来说，操作系统卸载 App 时也不能依赖 App 获得可靠的卸载前网络回调，因此 Zedra 的本地身份销毁模型具有跨平台一致性；显式设备泄露仍应由宿主侧 revoke 功能处理。
