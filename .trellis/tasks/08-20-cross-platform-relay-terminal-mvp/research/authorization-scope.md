# 配对授权范围

## 基本事实

zterm 1.0 的宿主 daemon、PTY、Shell 和其子进程全部使用同一个非特权 OS 用户身份。一个获得可写终端权限的远端设备可以：

- 运行任意该用户有权执行的命令；
- 读取和修改该用户可访问的文件；
- 查看或向该用户的其他进程发送其权限允许的信号；
- 访问该用户拥有的本地 socket 和配置，除非另行引入不同 OS 身份、沙箱或 capability broker。

因此，在同一 daemon 内仅用 session ID 建 ACL，并不能在两个通用 Shell 之间形成真正的安全边界。即使 RPC 拒绝设备 attach `session B`，设备也可能从获准控制的 `session A` 读取同用户资源、调用本地工具，或操作 session B 中属于同用户的进程。

## Zedra 对比

Zedra 当前同时保存一个类似 `authorized_keys` 的全局公钥集合和每个 registry session 的 ACL，attach 时检查 per-session ACL。全局授权设备在原 session 丢失时可以回退到其已有 ACL 的 session，或创建一个新 session 并加入 ACL。

这一实现适合 Zedra 自身的 workspace/session 产品模型，但不能证明 zterm 的同用户通用 Shell 获得了安全隔离。若照搬字段却不提供 OS 身份隔离，用户可能误以为某设备只能看到一个 terminal，而实际上它已经拥有该 OS 用户的代码执行能力。

## 1.0 建议

一次配对应明确表示“信任此控制设备以当前 OS 用户身份使用这台 zterm 宿主”，默认授权：

- 列出所有 session；
- 创建新 session；
- attach/takeover 当前及未来 session；
- resize、输入和读取终端输出；
- 显式关闭 session。

撤销也相应是主机级设备授权撤销，而不是逐 session 清理 ACL。1.0 及后续产品不显示或执行没有真实隔离保证的 per-session 权限 UI，也不把已配对设备划分为不同授权角色。

## 管理授权的现实边界

即使协议层禁止远端 RPC 直接执行 `pair` 或 `revoke`，一个已获完整 Shell 的设备仍可能在远端 Shell 中运行本机 `zterm` CLI，并以同一 OS 用户访问 daemon 的本地 IPC。因此 1.0 不应宣称“控制设备不能管理其他设备”是强安全边界。要实现此类区分，需要额外的本地交互证明、操作系统凭据分离或独立管理身份，均超出当前无账号、单用户 daemon 范围。

本地管理命令仍应保留明确确认和审计日志，以避免误操作；但产品文档必须把配对描述为高信任操作，类似把设备公钥加入该用户的远程 Shell `authorized_keys`。

## 多端状态不是授权角色

用户已明确未来也不提供向其他人分享连接或单个 session 的能力。规划中的多端 controller/observer 只描述用户自己的完全可信设备在某个 session 上当下是否持有输入和 resize 权限；observer 仍是主机级授权设备，可以创建其他 session 或显式发起 takeover，因此不能把它描述为低信任或只读授权角色。

如果产品方向以后发生变化并要求低信任分享，必须重新设计独立的安全边界，例如受限 endpoint、沙箱/容器或不同 OS 用户；仅增加 `session_acl` 字段不够，且不属于当前路线图。

## 取舍

主机级信任模型简单、诚实，并与通用远程 Shell 的实际能力以及“不向他人分享”的产品边界一致。per-session ACL 看起来更细粒度，但在同用户任意代码执行前提下不构成真实隔离，并显著增加配对、session 创建、未来 tab 和撤销同步复杂度。
