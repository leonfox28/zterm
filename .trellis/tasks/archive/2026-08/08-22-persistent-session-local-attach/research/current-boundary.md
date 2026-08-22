# M4 当前代码边界与继承证据

检查日期：2026-08-22。

## 已存在且必须复用

- `zterm-core::domain` 已有固定宽度 DeviceId/SessionId/AttachmentId、Revision、
  AttachmentPrincipal、ControllerLease、ResourceLimits、OperationId/OperationWindow。
- `zterm-core::terminal` 已有唯一 TerminalModel、snapshot/checkpoint/latest merged delta、
  安全查询回复/side event 和固定 cell projection。Foundation 默认已经固定为 8 Session、
  2,000 history、120x40 fallback、240x80 max、128 MiB aggregate projection、256 MiB RSS
  measurement target。
- `zterm-platform::pty` 已从 effective account database 选择 login shell/home/cwd；PtySession
  drop 不杀 child，只有 owner 调用 explicit close。
- `zterm-daemon::terminal_driver` 已把 PTY reader 与单 TerminalModel owner 分离，持续排空、
  自动写回 DA/DSR/CPR replies，并提供 latest-only TerminalAttachment checkpoint。
- M3 local IPC 已在读取 frame 前验证 same UID，使用单 FrameDecoder、8 MiB/1 MiB bound、
  deadline、connection semaphore 和 stop-after-flush；当前刻意只允许一连接一个 unary request。
- protobuf 已预留 session list/create/rename/close/takeover 与 terminal attach/snapshot/delta/
  input/resize/detach/snapshot-applied/sync kind，但 DaemonService 仍统一返回
  ServiceNotImplemented，没有 SessionRegistry 或 handler。
- identity/config/authorization 持久化属于 M3 SQLite；Session/PTY/attachment/replay 尚未持久化，
  M4 也不得把它们写入数据库。

## 必须新增而不能复制

1. 一个 transport-independent SessionService + 一个 SessionRegistry。
2. 每 Session 一个 owner/actor，组合已有 PtyHost、TerminalModel、TerminalDriver。
3. 一个全局 ResourceGovernor，复用 TerminalModel 的唯一 projection 算法。
4. 在现有 same-UID listener 上增加 attachment duplex 分支；不是第二个 socket 协议或 self-Iroh。
5. 现有 proto registry 的最小缺口：selector/viewport、lease lost、session ended。
6. operation replay 在 M4 的首次真实消费者：create/rename/close/takeover。

## 已确认的历史决定

- default `main` + 多 Session，idle 永不自动关闭。
- local 与 remote 必须看到同一 SessionActor/PTY/VT/controller lease。
- 1.0 单 controller；普通第二 attach occupied，显式 takeover。
- tmux/Herdr/其他复用器全部走普通 bytes/resize/snapshot，不做名称分支。
- login shell + home/validated cwd，不接受任意 create command。
- daemon stop/升级允许结束 Session；不自动更新，不跨 daemon/宿主重启恢复进程。

## M4 明确不拥有

- Iroh bind、N0/relay/path、pairing/auth gate、ConnectionRegistry。
- raw CLI terminal renderer 与 detach prefix。
- Windows runtime、Android/iOS/GUI、多 controller/observer。
- Agent state/notification、transcript persistence 或 history paging。

## Repeat architecture investigation

The repeat M4 pass inspected `portable-pty` 0.9.0 rather than assuming that a cloned
master or child-killer handle could provide independent, truthful shutdown:

- `portable-pty/src/lib.rs` defines `ChildKiller`/`ProcessSignaller`, but its portable
  contract does not promise a second owned child handle that can both escalate and reap.
- `portable-pty/src/unix.rs` implements the cloned Unix process signaller as signal
  delivery; it is not an independently owned wait/reap handle. Dropping or closing a
  cloned PTY endpoint is therefore not a sound substitute for owning child shutdown.
- `portable-pty/src/unix.rs` also keeps master-side resize/read/write behavior behind
  the master implementation. Zterm consequently splits its own PTY I/O ownership from
  the single child-control owner: a blocked writer cannot hold the child mutex, while
  explicit close retains one place that kills, escalates, and reaps.

No upstream Herdr source was needed for this design. The approved bounded actor,
disconnect-persistent operation cell, and child-ownership contracts were implementable
from Zterm's existing boundaries without importing Herdr product or agent assumptions.
