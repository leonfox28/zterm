# 调整 CLI 命令与输出格式

## Goal

在不破坏关键生命周期与安全边界的前提下，重新梳理 zterm 面向用户的 CLI 命令结构、命名和输出格式，使常用流程更直接、帮助与结果更一致。

## Background and Confirmed Facts

- 用户已明确要求创建 Trellis 任务，并要求先列出当前 CLI 的全部命令，再共同决定调整方案。
- 用户选择先确定命令层级与命名，再讨论输出格式。2026-09-16 复审后，用户接受连接语义修订；已定命令契约集中在 Requirements 与 Target Command Tree，下文基线描述不代表目标行为。
- 复审依据与记录见 `research/reassessment-2026-09-16.md`。命令规则与输出风格均已确认；`research/output-format-proposal.md` 保留已接受的输出样例。
- 当前工作区版本为 `0.1.32`（`Cargo.toml:17`）。
- 当前 Clap 定义公开 12 个顶层命令，展开为 21 个可执行操作（`crates/cli/src/lib.rs:148`、`:233`、`:264`、`:303`、`:383`）。
- 完整实现前命令清单（`README.md:29`）已移至 `research/current-cli.md`，以免与下文唯一目标语法混淆。

- 裸 `zterm` 在尚未 setup 时只提示执行 `zterm setup`；完成 setup 后等价于 `zterm connect local --session main`（`crates/cli/src/lib.rs:648`）。
- 全局只提供 `-h/--help` 与 `-V/--version`，显式禁用了 `help` 子命令（`crates/cli/src/lib.rs:31`）。
- 另有 5 个从公开帮助隐藏、供 daemon 与发布流程使用的内部入口：`--internal-daemon`、`--internal-update`、`--internal-release-self-check`、`--internal-release-verify <MANIFEST> <SIGNATURE>`、`--internal-release-install <DESTINATION>`（`crates/cli/src/lib.rs:38`）。它们不是公开命令重构的默认对象。
- 2026-09-05 的已归档任务 `09-05-zterm-cli-commands-execution` 已将公开输出统一为人类可读文本，删除公开 `--json`/`--force`，并统一采用 `-y/--yes`。本任务以当前实现为基线，不恢复旧接口，除非用户明确选择新的兼容策略。
- 仓库调用点显示 `setup` 与 `update` 已被安装流程直接提示，`pair create` 已被 Android 文档直接引用，其他公开拼法主要由 README、详细文档和 CLI 测试消费；命令重命名至少需要同步帮助、文档、提示文案和解析测试。
- `zterm status` 与 `zterm daemon status` 当前调用完全相同的状态处理器（`crates/cli/src/lib.rs:625`、`:632`），属于真正的重复入口。
- `zterm connect` 与 `zterm session attach` 表面重叠但语义不同：`connect` 在选择 `main` 时允许不存在则创建，`session attach` 永远只连接既有会话（`crates/cli/src/lib.rs:769`、`:818`）。规划不能仅为减少命令数量而直接合并这两个契约。
- `pair`、`device`、`session` 与 `daemon` 已按领域分组；`status`、`doctor`、`connect`、`logs`、`reset`、`update`、`uninstall` 则是面向工作流的顶层入口，因此当前模型实质上是“常用动作 + 资源管理”的混合结构。

## Requirements

- R1：完整记录当前公开命令、子命令、主要参数、默认入口和隐藏内部入口。
- R2：采用以下已确认的命令树、参数、行为和兼容策略；原有未列为变更的命令继续保持语义。
- R2.1：目标采用精简的混合模型。保留日常高频入口的短路径，保留低频管理命令的领域分组，只消除真实重复、含混命名和不必要层级。
- R2.2：删除 `daemon status` 子命令，以顶层 `status` 作为唯一状态入口。同步清理帮助、文档、提示文本和解析测试；不得保留兼容别名。`status` 继续保持不创建状态、不启动 daemon 的观察语义。
- R2.3：删除 `session attach` 子命令，以顶层 `connect` 作为唯一显式会话连接命令，不保留兼容别名。只有省略 `--session` 时，才连接默认 `main` 并在不存在时原子创建；显式 `--session <name-or-id>` 对所有名称（包括 `main`）和 ID 都只连接已有会话，不存在则报错且不创建。这是 2026-09-16 用户接受的修订，取代先前按名称 `main` 自动创建的决定。
- R2.4：`connect` 的 target 位置参数可省略并默认 `local`；显式本地或远端 target 与 `--takeover` 继续可用。setup 完成后的裸 `zterm` 与 `connect`、`connect local` 共用省略 `--session` 的默认路径；setup 前裸调用维持原有提示和退出语义，不隐式初始化。保留精确 selector 校验及显式接管规则。
- R2.5：将 `session new` 重命名为 `session create`，不提供旧名称别名。语法采用 R2.9 的本机默认规则；成功创建后立即连接精确返回的 Session ID，连接失败时仍报告已经创建且继续存活的 Session ID。帮助必须明确这是创建并进入会话，仍要求交互终端，`--cwd` 继续由目标宿主解释。
- R2.6：`pair create` 不再有业务参数，拒绝原有 `--ttl`、`--qr`、`--qr-image`，保留标准 `-h/--help`。每次使用现有默认值 600 秒（`crates/core/src/pairing.rs:31`）创建一个票据；交互式成功输出同时包含同一票据的终端二维码、可用于手动输入/粘贴的票据文本、有效期和接收端操作指引。票据只创建一次，不得因二维码呈现失败而创建第二张票据。
- R2.7：移除配对 QR PNG 导出能力，其他图片上传功能继续可用；保留票据零化和敏感信息不进入日志、错误或 Debug 的边界。
- R2.8：`pair create` 必须按呈现能力自动选择输出。非交互 stdout 只输出原始票据和一个换行，不输出 ANSI、二维码或说明；有效期、操作指引与退化诊断使用 stderr。交互终端过窄或二维码渲染失败时给出不泄露票据的说明并继续显示手动票据；上述分支共享同一张票据和同一有效期。真正的输出写入失败仍须报错，不属于二维码退化成功。
- R2.9：`session` 管理命令采用 Target Command Tree 的 local-first 参数形式，位置参数仅用于会话名称/ID 与新名称。省略 `--target` 时使用 `local`；显式 target 继续使用现有精确 alias/full ID 解析与权限边界。
- R2.10：删除 `session` 命令原有的 target-first 位置参数形式，不做兼容性猜测或双重解析；旧调用应稳定成为解析错误，避免把原 target 错当成 session/name 后执行到错误对象。
- R2.11：将身份重置命令简化为 `reset [-y|--yes]`，删除并拒绝旧的 `--identity`。命令帮助与确认必须明确其清除本机受管理状态（含 identity、配置、配对数据）、结束所有会话且保留程序的影响；存在实际删除影响且没有 `-y/--yes` 时继续要求确认，已无状态时维持成功空操作。不得因语法缩短而削弱删除前检查、精确目标冻结或可重试行为。
- R3：保留现有 setup/daemon 启动、会话生命周期、配对票据保密、破坏性操作确认以及内部发布入口等边界，除非规划阶段明确记录变更理由和替代契约。
- R4：最终帮助、README、详细 CLI 文档和解析/输出测试必须与新的公开接口一致。
- R5：普通 CLI 输出统一使用简洁英文纯文本：单对象采用对齐字段，列表名称优先，完整 ID 放次行且可复制；不增加边框、emoji、动画或输出模式参数。配对二维码保留固定黑白呈现，原有交互终端界面不属于此规则的重构对象。
- R5.1：`status` 依次展示可取得的设备名、版本、Setup、Daemon、Infrastructure、Network、Session 数量与名称，完整设备 ID 次行展示；保留未初始化与已配置停止的区别。`session list` 标明目标，以 Name / State / Size 展示名称、Attached/Detached 和列数 x 行数，每个会话下一行提供完整 ID。
- R5.2：`device list` 保留名称、连接观察、出站已知记录和入站授权两个方向及完整设备 ID；不得把未连接称为不可达，也不得把本地已知记录等同于远端当前授权。保留空列表的下一步指引，不丢失已撤销状态。
- R5.3：成功提示简短并保留必要目标和身份；错误先说明失败操作/目标，再在既有错误类别足以支持时给出可执行下一步。保留取消、超时、结果未知及部分成功的区别，尤其创建成功而连接失败时的 Session ID；不得通过猜测错误字符串给出错误的重试指令。示例中的命令必须按真实语法和名称正确引用。
- R5.4：确认提示先展示准确目标与影响，再显示 `Continue? [y/N]:`；输入规则和各操作触发确认的条件仍服从 R3。`reset` 说明本机清理范围及保留程序，`uninstall` 则明确程序也会删除。
- R5.5：命令结果写 stdout；进度、错误、提示及确认写 stderr。`doctor` 检查报告、`logs` 日志尾部属于查询结果，仍写 stdout；不修改既有 daemon 日志记录格式或跟随行为。配对票据严格遵循 R2.8；隐藏内部入口继续维持消费者所需的原始输出。

## Target Command Tree

以下是已确认的目标语法：12 个公开顶层入口、19 个具体操作。方括号代表可选参数。

```text
zterm
zterm setup [--name <name>] [--profile <official-n0|self-hosted>] [--relay-url <https-url>]
zterm status
zterm doctor
zterm pair create
zterm pair accept [--stdin] [--alias <alias>]
zterm device list
zterm device rename <device> <alias>
zterm device revoke <device> [-y|--yes]
zterm connect [<target>] [--session <name-or-id>] [--takeover]
zterm session list [--target <target>]
zterm session create <name> [--target <target>] [--cwd <host-path>]
zterm session rename <session> <new-name> [--target <target>]
zterm session close <session> [--target <target>] [-y|--yes]
zterm daemon stop [-y|--yes]
zterm daemon restart [-y|--yes]
zterm logs [-n|--lines <n>]
zterm reset [-y|--yes]
zterm update [--version <vSEMVER>] [-y|--yes]
zterm uninstall [-y|--yes]
```

`-h/--help`、顶层 `-V/--version`、既有隐藏内部入口保持原有边界。普通输出继续使用英文人类可读文本，不重新引入 `--json` 或 `--force`。

## Acceptance Criteria

- [x] 当前命令清单与 Clap 定义逐项对应，覆盖 12 个顶层命令和 21 个具体操作。
- [x] 裸调用、帮助/版本选项和 5 个隐藏内部入口已单独记录。
- [x] 命令规则、参数、破坏性兼容策略与 2026-09-16 连接语义修订均得到用户确认。
- [x] 用户已确认输出样例的简洁英文纯文本、对齐字段、名称优先和完整 ID 次行风格。
- [x] R2.1/R2.2/R2.3/R4：公开帮助与目标树一致；移除的子命令不是隐藏别名，状态查询不启动 daemon。
- [x] R2.3/R2.4：默认连接在 `main` 缺失时创建、存在时复用；显式 `--session main` 和其他名称/ID 在缺失时都不创建。上述契约对本地和远端目标使用同一请求路径。
- [x] R2.4：省略 target 和显式 `local` 选择相同目标；setup 前裸调用仍只给出指引且不创建状态。
- [x] R2.5/R2.9：创建会话立即连接准确返回的 ID；连接失败时该会话仍存活并报告 ID；目标宿主仍解释 `--cwd`。
- [x] R2.6/R2.8：交互输出的二维码与文本对应同一张 600 秒票据；非 TTY stdout 严格为票据加换行；窄终端/渲染失败保留同一手动票据；真实写入失败不报成功。
- [x] R2.6/R2.7：三个旧配对选项被拒绝，无 PNG 导出功能；配对帮助仍可用且无 daemon/票据副作用。
- [x] R2.9/R2.10：各 Session 管理命令默认 `local`，显式 `--target` 正确传递；旧的完整 target-first 调用因多余位置参数或已删除子命令而被拒绝，不能执行到其他目标。
- [x] R2.11：`reset --identity` 被拒绝；新的 `reset` 保持现有实际删除确认、非交互 `-y`、已无状态时的空操作和程序保留行为。
- [x] R3：精确 selector、显式 takeover、方向性授权、票据保密以及内部发布入口仍符合既有契约。
- [x] R4：当前文档与提示不再推荐旧命令；历史任务记录不改写。
- [x] R5/R5.1：状态的未初始化/运行/停止三类结果仍可区分；会话列表有目标、控制端状态、尺寸和完整 ID；中文/Unicode 名称按显示宽度对齐。
- [x] R5.2：设备列表保持两个授权方向、撤销状态和完整 ID，空列表有合法指引，未连接不误报不可达。
- [x] R5.3/R5.4：成功、已取消、明确失败、结果未知和部分完成仍可辨认；提示引用真实目标，原确认保护和精确身份绑定未减弱。
- [x] R5.5：重定向 stdout 不混入提示/确认/错误；查询结果仍完整保留，配对原始票据输出及隐藏发布自检保持其既有机器消费者契约。

## Out of Scope

- 不调整隐藏 daemon/发布入口、远端协议、会话持久化、终端渲染或网络行为。
- 不增加交互式设备/会话选择器、后台创建选项、命令别名、JSON、日志跟随、动态补全或通用输出框架。
- 不改变配对授权方向、默认基础设施、精确 ID 选择、更新流程或本机状态删除范围。

## Risks and Verification Limits

- 命令拼法立即破坏兼容性，旧脚本需迁移；显式 `--session main` 的语义变更须在文档写明。
- 管道票据和 QR 包含同一份短期配对权限，不能在改版输出时进入 Debug、日志或错误。
- 2026-09-16 规划期间的 Xcode 许可阻塞已由用户解决，实施前 Git 检查正常。macOS 上真实 Iroh/Endpoint 测试继续仅编译，运行证据由现有 Linux CI 提供。

## Artifact Status

- `prd.md`：已收敛命令与输出要求、范围和可观察验收；无待定产品问题。
- `research/output-format-proposal.md`：已接受的输出样例，技术设计落实字段可取得性和呈现边界。
- `design.md` / `implement.md`：用户已批准实施，任务处于 in_progress；代码与文档已实现，最终验证进度见 implement.md。
