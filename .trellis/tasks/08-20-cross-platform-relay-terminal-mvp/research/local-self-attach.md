# 桌面端连接本机 daemon 并接续同一 session

## 用户场景

用户在 macOS/Linux/Windows 项目主机上运行 zterm daemon，从 Android 等远端控制设备进入某个持久 session 开发。回到项目主机后，即使没有 Herdr、tmux、GNU Screen 或其他终端复用器，也必须能从本机 CLI/未来桌面 GUI attach 同一个 zterm session，看到同一个 SessionId、当前权威 screen 与有界 scrollback，并继续操作原 PTY。

该需求不是创建“本地副本”或把终端内容同步回来；PTY 一直就在本机 daemon 中，本地视图只是为它增加一个 attachment。

## 参考模型

- Herdr 的普通本地命令是 thin client：先探测本地 socket，必要时拉起 server，再 attach server 持有的 session。现有调查固定在 `herdrdev/herdr` 提交 `9d7b6c24c4d251a62a861f37c2c394078e083ca8`，详见 `research/per-user-daemon.md`。
- tmux 同样由 server 持有 session/PTY，本地 terminal 只是 client。zterm 不需要识别或依赖 tmux，而是把这层最小持久会话能力作为自身产品契约。
- zterm 比普通本地复用器多一个约束：同一个 SessionActor 还可能被 Android/其他桌面设备远程 attach，因此本地视图不能绕过 controller lease 或另建一套本地 PTY 状态。

## 推荐数据路径

```text
本机 CLI / future GUI
        │ same-UID local IPC
        ▼
本机 zterm daemon
        │ LocalSessionService
        ▼
同一个 SessionRegistry / SessionActor / PTY / TerminalModel
        ▲
        │ remote attachment stream over authenticated Iroh connection
手机或其他远端控制设备
```

本地 self attach：

- 不建立到自身 EndpointId 的 Iroh connection；
- 不查询 DNS/Pkarr，不经过 relay，不做 NAT 穿透；
- 不要求把本设备公钥配对到自己；
- 依赖 socket ownership + peer UID 验证，因为同一 OS 用户的任意进程本来就拥有等价 Shell 权限；
- 复用与远端完全相同的 SessionActor 操作、snapshot/revision/delta、输入/resize、detach、takeover 和资源限制；
- daemon 内可把 attachment principal 表示为本机 EndpointId + `LOCAL_SAME_UID` 信任来源，用 AttachmentId 区分多个本地 view，但不能把自己的 EndpointId写入远程 `device_auth`。

local IPC 与 QUIC 可以复用 protobuf/framing 和终端消息形状，但 adapter 必须落到同一个内部 `SessionService`，不能复制两套 session 逻辑。这样本机与手机看到的 revision、控制权和结束原因完全一致。

## 生命周期与控制权

- 本地 attach 不创建新 session 副本；指定名称存在就进入，不存在时仅按与远端相同的 create-and-attach 契约创建。
- 本地 CLI 退出只删除本地 attachment，不能关闭 PTY。
- 手机断网、App 冷 tab/后台 detach 后，其 controller lease 随 attachment 释放，本地 attach 可以直接获得控制权。
- 若手机仍持有 controller lease，本地普通 attach 与任何第二控制端一样返回 `occupied`；只有显式 `--takeover` 才原子转移 lease。物理上位于宿主机器旁边不构成绕过产品控制权语义的理由。
- 本地 takeover 后手机得到明确 `LeaseLost(TAKEN_OVER)`；session、PTY、SessionId 和 revision 不变。
- daemon 停止、崩溃、更新或主机重启仍会结束 PTY；local attach 不能恢复已经随 daemon 消失的 session。

## CLI 形状

统一 device selector 的 canonical 入口建议是：

```text
zterm connect local [--session main] [--takeover]
zterm session list local
zterm session new local build [--cwd <local-host-path>]
zterm session attach|rename|close local ...
```

`local` 是保留的内建 selector，未来 GUI 显示为 “This Device”，不能与用户设备 alias 冲突。用户已确认 setup 后裸 `zterm` 成为 `zterm connect local --session main` 的快捷方式；未 setup 时不静默创建身份，只提示运行 `zterm setup`，命令帮助使用 `zterm --help`。canonical 命令继续用于脚本、显式目标与命名 session。

## 跨阶段边界

- 第一阶段：macOS/Linux CLI 支持 self attach。
- 第二阶段 Android：仍是 controller-only；后台冷 tab detach/release terminal lease，session 留在桌面 daemon。
- 第三阶段 Windows：Named Pipe + 当前用户 ACL 提供等价 self attach，最终 PTY 使用同一 ConPTY SessionActor。
- 第四阶段桌面 GUI：本机 tab 与远端设备 tab 都调用 daemon local IPC；本机 tab 使用 self target。
- 第五阶段 iOS：与 Android 相同，不托管本机通用 Shell。

## 验收场景

1. 手机 attach macOS/Linux `main` 并启动长任务；手机 detach/进入后台后，本机 self attach 得到相同 SessionId、进程、cwd、当前 screen 和近期历史。
2. 本机输入继续推进该 PTY；本机 detach 后手机 reattach 看到相同后续状态。
3. 手机仍在线控制时，本机普通 attach 不打断它；本机显式 takeover 后原手机收到 lease lost，PTY 不变。
4. relay、DNS/Pkarr 和外网全部不可用时，本机 self attach 仍工作，且网络观测中没有 self-dial。
5. 两个同 UID 本地 CLI 遵守同一个 controller lease；其他 OS 用户不能访问 socket 或 session。
6. 本机创建/rename/close 的 session 立即成为远端 session list 中的同一对象，反向操作亦然。
7. 本机不安装或运行 tmux/Herdr 时仍通过上述全部验收；安装后把它们作为普通前台 TUI 使用也不改变路径。
