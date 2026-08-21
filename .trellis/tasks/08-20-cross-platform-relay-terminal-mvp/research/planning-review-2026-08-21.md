# zterm 规划与边界复审（2026-08-21）

## 1. 复审目的与方法

本轮不扩展功能，而是反向审查 PRD、design、implement 和研究记录是否：

1. 真实反映用户已经确认的决定；
2. 没有把技术假设冒充产品承诺；
3. 在断线、重试、撤销、升级和恶意输入下仍有明确语义；
4. 不为 Android/GUI/Agent 提前实现第一阶段用不到的系统。

证据包括：

- `trellis mem` 中 2026-08-20 的 Codex 原始对话（项目会话 `01a01efe-c72`）；
- 本任务的 PRD、技术设计、实施路线与全部研究记录；
- 本地 Zedra 源码、固定版本 Herdr 调查记录；
- 2026-08-21 重新核验的 Iroh 官方 release、DNS/Pkarr、self-host relay 文档与 1.0.3 源码。

## 2. 结论

总体方向成立：第零阶段先建立可重复的本机开发环境并把固定上游`iroh-relay`部署到用户公网服务器；第一阶段再作为 macOS/Linux 的持久远程终端 MVP，以每用户 daemon 持有 PTY 与权威 VT，用 Iroh 提供直连优先/中继回退。这让第一阶段从一开始就使用真实默认relay，而不是把基础设施推迟到发布末尾。

用户已在复审后确认配对为 SSH-like 单向授权，确认第一阶段 Linux 只正式支持主流 glibc x86_64/arm64，并在复核 Herdr 发行历史后确认第一阶段使用官方用户级 direct installer + 显式 `zterm update`。Alpine/musl、NixOS 原生支持和包管理器发行渠道延后。随后新增 GitHub托管、精确开发版、`~/.zterm/`、卸载身份销毁和桌面 self attach需求：撤销边界已确认跟随Zedra，本机销毁身份、泄露凭证逐宿主revoke；self attach确认复用同一SessionActor/PTY/lease；setup后裸`zterm`确认进入本机`main`。最新补充的第零阶段也已收敛：本地relay smoke后人工暂停并等待用户提供服务器连接方式，核心只使用官方`iroh-relay`，不写自有转发实现。当前没有阻塞最终规划审阅的用户决定。

## 3. 已确认的产品决定

| 领域 | 已确认决定 | 边界 |
| --- | --- | --- |
| 项目身份 | 全新 zterm，旧项目是 `zterm_old` | 不迁移旧数据/协议/UI |
| 路线图 | 0 开发环境+公网relay；1 macOS/Linux CLI；2 Android；3 Windows；4 桌面 GUI；5 iOS | 第零阶段无终端产品功能；桌面 GUI 首阶段不做 |
| 第零阶段 | 探测后补齐本机Rust/Docker环境，封装官方relay，本地smoke后等待用户给服务器连接方式，再公网部署/回滚 | 不提前连接服务器；系统级缺口先报告影响；秘密不入仓库 |
| 产品形态 | 桌面可托管也可控制；手机只控制 | CLI 可完成第一阶段，以后 GUI 保留 CLI/daemon |
| 终端生命周期 | 网络/CLI 断开只 detach，PTY 继续 | daemon 停止/崩溃/宿主重启可中断，1.0 不恢复进程 |
| session | 默认 `main`，可新建/切换/关闭多个 session | 不按 idle 自动关闭；CLI 每进程一个 view |
| 本机接续 | macOS/Linux首阶段、Windows第三阶段均可经same-UID IPC attach本机daemon同一session | 不self-dial、不自配对；裸`zterm`进入本机`main`，仍遵守显式takeover |
| connection | 每设备对一条 Iroh connection，session 用独立 stream | 冷 tab 不传 terminal bytes，切回时 snapshot |
| 控制权 | 1.0 一个 controller/session + 显式 takeover | 未来一写多观察；1.0 不做 observer UI/多写者 |
| 终端状态 | 宿主权威 VT + snapshot/revision/delta + 有界内存 scrollback | 不落盘终端内容，不承诺任意 TUI 完整 transcript |
| Shell | 当前 OS 账户 login shell，home/显式 `--cwd` | 首阶段不把任意命令作为 session 根进程 |
| 权限 | 已授权设备等价于当前 OS 用户的远程 Shell 公钥 | 现在和未来都不做分享、访客、per-session ACL |
| daemon | 官方 direct installer 用户级安装，单一原生程序按需 detached-spawn | 1.0 无开机/登录自启、无 supervisor、无管理员授权 |
| 安装版本 | GitHub Release托管；默认latest stable，`--version`可选签名稳定/开发prerelease | 不安装任意commit、branch或会过期Actions artifact |
| 卸载 | 删除`~/.zterm`与本机身份，正常重装必须重新配对 | 跟随Zedra，无RevokeSelf/中央吊销；泄露凭证逐宿主revoke |
| 升级 | 用户显式 `zterm update`，允许确认后中断 session | 无后台检查/自动安装；先验证 artifact 再处理 daemon |
| 云端 | 寻址/NAT 协调/密文 relay，终端 E2EE | 无账号、业务 API、云端授权、终端存储/解密 |
| 地址服务 | 官方 `dns.iroh.link`，只公开 home relay | 首阶段不自建 iroh-dns-server，不公开 direct IP |
| relay | 项目默认 + 允许自建，固定官方`iroh-relay` binary，Everyone，无 token/名单/限速 | 不写转发核心；只维护镜像/配置/Compose/运维，无自定义monitor sidecar |
| Agent | 2.0 前只是通用终端 | 仅保留旁路 event/capability 兼容形状，不做空插件框架 |

## 4. 本轮发现并修正的问题

| 问题 | 风险 | 复审后的契约 |
| --- | --- | --- |
| ticket 作为 CLI 位置参数 | 进入 shell history/进程 argv | `pair accept` 默认不回显 prompt，自动化只从显式 stdin 导入 |
| 变更 RPC 只有普通 request ID | 已提交但响应丢失时重复 create/rename/close | 128-bit operation ID + 每设备有界去重结果 |
| snapshot 同步期“拒绝/排队”输入 | 旧按键在新画面上误执行 | CLI 持续 drain 并丢弃普通输入，不排队/重放，仍允许本地 detach |
| 配对后再要宿主本地确认 | 远程宿主场景无法完成，且重复意图 | 宿主 `pair create` + 接收方导入就是显式意图，不叠加二次本地确认 |
| 包管理器更新无法安全协调 session | 外部更新器无法在替换前可靠读取 daemon 状态 | 改用显式 `zterm update`：先下载/验签/校验，再列出 session、确认中断、停止 daemon并原子激活；失败恢复旧 binary |
| 首阶段实现 history paging | CLI 用不到，引入 cursor/epoch/过期协议 | 首阶段 snapshot 只带有界近期历史，paging 延后为 `HISTORY_PAGING` capability |
| 首阶段实现长生命周期 device-event stream | CLI 没有冷 tab，提前冻结未经 Android/GUI 验证的消息 | 只保留 `DEVICE_EVENTS` capability/stream 升级边界，实现延后 |
| 持久 audit table | 增加数据保留面和 schema，对 MVP 无必要 | 删除；只用有界、脱敏、无终端内容的结构化日志 |
| 16 session/10k 行/256 MiB 被写成产品承诺 | 未实测即冻结假精确值 | 作为 Gate 0 候选压力目标；最终值实测，但至少支持 `main + 2` 且全部资源有界 |
| 未识别 OSC/DCS/APC 可能原样到本地终端 | 远程程序可触发本地 clipboard/图形等副作用 | 只发 TerminalModel 生成的受控 ANSI/state，1.0 禁用 OSC 52 等高风险能力 |
| 未认证建连只有帧大小限制 | 公开 relay 可被利用耗尽 daemon 任务/内存 | 增加全局/单 EndpointId 并发、首帧 deadline 和总字节上限 |
| relay 日志被简化为“短 hash” | 低估元数据暴露 | 披露缩短 EndpointId 和可能的 remote IP，以日志级别/轮转/保留期控制 |
| CLI/实施命令不一致 | rename、restart、logs、identity reset 在实现中漏项 | 命令面和 M2-M10 验收已重新对齐 |

## 5. 信任、安全与不保证项

- 可信：宿主 OS、宿主当前用户/同 UID 进程、用户主动授权的完整设备。
- 不可信：relay、DNS/Pkarr、公网、未授权 EndpointId、畸形 frame、终端程序产生的任意控制序列。
- 保证：设备身份认证、终端内容机密性/完整性、未授权设备无 RPC 访问、交通字节不落盘为业务数据。
- 不保证：可用性、匿名性、流量分析防护、已配对设备/宿主失陷后的隔离、开放 relay 的成本上限。

这一边界与 Iroh 官方现状一致：公共基础设施免费但有限速且无 uptime 保证，自建 relay 是支持路径。来源：[Iroh DNS 文档](https://docs.iroh.computer/connecting/dns-address-lookup)、[Iroh hosting](https://www.iroh.computer/services/hosting)、[Iroh self-host relay](https://docs.iroh.computer/add-a-relay)。

## 6. 技术风险排序与停止线

1. **最高：权威 VT 兼容性**。不是“能显示 Shell”就算通过；必须先证明 snapshot/delta、alternate screen、Unicode、查询响应、tmux/Herdr 和无 attachment 持续排空。失败时替换 `TerminalModel` 实现，不降低恢复语义。
2. **高：公开 relay 容量/成本**。Everyone + 无 limits 是已接受 beta 策略，但必须有私有 metrics、日志保留和手动止损文档；不虚假承诺成本上限。
3. **高：daemon 生命周期**。无开机自启使重启后暂时不可达，daemon 崩溃会结束 PTY；这是 1.0 明确边界，必须在 UX 中直说。
4. **中：installer 供应链与 Linux 兼容性**。第一阶段已明确只承诺主流 glibc x86_64/arm64；installer 必须在下载前拒绝 Alpine/musl、NixOS 原生和不满足 glibc 基线的环境，并对 bootstrap 信任、manifest 签名、校验与回滚负责。
5. **中：local/remote adapter 漂移**。本机 self attach 与远端 QUIC attachment 必须调用同一 SessionService、TerminalModel和lease状态机；用交叉端到端测试证明不存在复制session、隐式本机优先级或self-dial。
6. **中：长时间高输出资源**。必须用 Gate 0 校准 scrollback/session/snapshot 默认值，不把候选数字当作性能承诺。
7. **中：第零阶段外部服务器变更**。连接方式由用户在人工检查点提供；先只读preflight。Docker安装、防火墙或端口冲突处理若超出现状，必须先报告精确影响，避免误改同机其他服务。

## 7. 用户决定状态

### 已解决：配对授权方向

用户确认采用 SSH-like 单向授权。宿主生成 ticket、控制端导入后，控制端获得控制宿主的权限，宿主不因此自动获得反向权限。两台桌面设备需要互相控制时，反向再生成一张 ticket；两个授权方向分别撤销。

### 已解决：Linux 发行边界

当前设计只发布 `x86_64-unknown-linux-gnu` 和 `aarch64-unknown-linux-gnu` artifact，即默认只承诺主流 glibc 发行版。Rust 官方工具链也包含 musl 目标，所以原生 Alpine 支持以后可以在不改变 core/protocol/state 的前提下增加 artifact、installer target mapping 与实机验收。来源：[Rust 目标支持](https://doc.rust-lang.org/rustc/platform-support.html)。

Zedra 当前自身的 installer 也只发布 x86_64/aarch64 GNU 产物（`/Users/huyuanzhe/projects/zedra/scripts/install.sh:52-73`）。用户已接受复审建议：第一阶段只正式支持主流 glibc Linux，Alpine/musl 和 NixOS 原生包装延后。

### 已解决：发行与更新所有权

2026-08-21 通过 `npm view zterm` 复核发现，无作用域的 `zterm` 已由 `catpea` 发布 1.0.3，repository 指向 `catpea/zterm`。随后对 Herdr 的完整公开历史复核发现，其首个公开提交已经采用 curl installer 与内置 updater；npm 上的 `herdr@0.0.0` 只是后来创建的包名占位，不是历史主发行渠道。详见 `research/distribution-update-channel.md`。

用户确认第一阶段改用用户级 direct installer + 完全手动触发的 `zterm update`，不把 npm 或其他包管理器作为官方首阶段渠道。npm 包名不再构成阻塞；项目明确承担 installer、manifest、签名/校验、原子替换、回滚与 bootstrap 信任披露。

### 已解决：安装、身份与卸载

installer脚本、manifest和产物托管在项目GitHub；无参数安装latest non-prerelease stable，`--version`精确选择已签名稳定版或开发prerelease。桌面持久数据统一放在`~/.zterm/`（Windows为用户home下等价目录）。身份由`zterm setup`生成，不由installer生成。

用户在核对本地Zedra提交`a30bc6c`后确认跟随其卸载边界：官方卸载删除本机身份、授权与已知设备状态，正常重装生成新EndpointId并要求重新配对；不实现卸载前`RevokeSelf`或中央撤销服务。设备/私钥泄露由每台宿主的device revoke立即切断。

### 已解决：桌面本机接续

macOS/Linux第一阶段和Windows第三阶段必须允许当前用户经same-UID local IPC attach本机daemon持有的同一SessionActor、PTY、权威VT和controller lease；不self-dial Iroh、不自配对、不依赖tmux/Herdr。手机后台detach后本机直接接续；手机仍控制时本机必须显式takeover。canonical入口为`zterm connect local`，setup后裸`zterm`等价于进入本机`main`，帮助使用`zterm --help`。

### 已解决：第零阶段与 Relay 实现所有权

用户要求在第一阶段前增加第零阶段，先完成本机开发环境和自建默认relay。当前开发机已有Xcode、Homebrew、Git与CMake，缺Docker、`protoc`和`pkg-config`。用户随后重新运行官方rustup installer，把现有stable从1.97.1更新到Rust/Cargo 1.98.0；rustfmt、Clippy、rust-analyzer及Apple/iOS/Android targets已验证存在。用户决定项目使用最新版，因此规划不再安装/验证Rust 1.91，而在`rust-toolchain.toml`精确固定1.98.0；后续升级显式进行，不让浮动stable改变构建。默认本地容器runtime仍为Docker CLI/Compose + Colima，Protobuf生成不让最终用户依赖系统`protoc`。

Zedra提交`a30bc6c`的`deploy/relay/Dockerfile:5-13`证明其转发核心来自固定上游Iroh，Compose、部署脚本和monitor才是Zedra自有外壳。Iroh v1.0.3官方Release已经提供Linux relay binary与SHA-256。zterm因此只封装官方binary、配置、Compose、health/metrics、日志和回滚，不写relay数据平面，也不复制Zedra monitor。Z0-B本地smoke后必须暂停通知用户；收到连接方式后先只读preflight，再部署公网服务器。

## 8. 开发状态

本轮只修正规划与研究文档。原复审一度收敛，随后用户补充安装、卸载、桌面本机接续和第零阶段需求，因此任务继续保持`planning`。第零阶段开发环境、官方relay复用、人工服务器检查点与公网部署/回滚门已经写入PRD/design/implement；等待用户批准新的最终规划摘要。未安装本机工具、未连接公网服务器、未创建产品源码、未运行`task.py start`，未创建或启动实施子任务。
