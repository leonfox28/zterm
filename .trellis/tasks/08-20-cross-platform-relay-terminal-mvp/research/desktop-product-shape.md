# 桌面端产品形态分析

## 问题重述

项目首先要解决的是：设备位于 NAT、CGNAT 或防火墙之后时，用户仍能安全地进入远程交互式终端。桌面端是否内置新的终端窗口，是解决方案选择，不是问题本身。

## 不可省略的桌面能力

无论是否有图形 App，作为宿主的 Linux/macOS/Windows 都需要：

- 一个可常驻的用户态 agent/daemon，维护设备身份、iroh endpoint、地址发布、中继连接和已授权设备。
- PTY 生命周期管理，以及输入、输出、resize、close、detach/reattach 等协议处理。
- 安装、启动/停止、升级、日志、诊断和卸载机制。1.0 采用本地命令按需 detached-spawn，不注册开机或登录启动项；自动启动属于后续阶段能力。
- 配对票据生成、授权设备查看与撤销、连接路径和健康状态查询。

作为控制端的桌面系统已经有成熟的本地终端模拟器；CLI 可以进入 raw mode，把按键、窗口尺寸和远端 PTY 输出桥接到当前 TTY。因此“桌面可作为控制端”并不要求自建终端渲染窗口。

## 三种形态

### A. Daemon + CLI

- `zterm daemon` 或系统用户服务负责宿主能力。
- `zterm connect <device>` 在 Terminal.app、Ghostty、WezTerm、Windows Terminal、Konsole 等现有终端中运行。
- `zterm pair/status/devices/revoke/logs` 管理设备。
- 优点：最小、跨平台核心一致、不重复开发终端渲染器，能把资源集中在 iroh、协议、重连和移动端。
- 缺点：首次安装和配对偏开发者化；状态、权限和更新反馈不够直观。

Mosh 是这种交互的直接先例：它是类似 SSH 的命令行程序，运行在现有终端模拟器中，同时专注于漫游、断网恢复和网络体验（https://mosh.org/）。Zedra 本地参考也采用桌面 daemon/CLI + 移动 App，而非桌面终端 GUI。

### B. Daemon + CLI + 轻量管理界面

- 保留 A 的终端交互，不内置终端渲染器。
- 后续增加菜单栏/托盘或仅本机可访问的管理页面，用于扫码配对、开机启动、路径状态、设备撤销、日志和升级。
- 优点：改善 onboarding，但不承担完整终端工作台的复杂度。
- 缺点：仍增加 UI、安全边界和各桌面平台集成工作；托盘实现也未必能完全跨平台共享。

Tailscale 展示了这种边界：SSH 会话仍可通过常规客户端进行，而本地 Web UI 负责设备设置和 SSH server 开关，并非重新实现一个终端（https://tailscale.com/docs/features/tailscale-ssh、https://tailscale.com/docs/features/client/device-web-interface）。

### C. 完整桌面终端 App

- 内置 terminal renderer、标签页、分屏、主机库、会话管理等。
- 只有当产品价值扩展到“统一远程运维工作台”时才明显优于现有终端。
- 优点：体验可控、发现性好，可承载文件传输、端口转发、多会话、跨端同步等功能。
- 缺点：终端仿真、输入法、快捷键、GPU 渲染、无障碍和三平台窗口集成会成为独立的大型产品线。

Termius 的桌面 App 正是依靠 Vault、SFTP、Groups、主机管理和跨端同步等工作台能力建立差异，而不只是提供远端字节流（https://termius.com/）。这些能力当前均未被 zterm 需求要求。

## 推荐

- 第一阶段选择 A：桌面交付 agent/daemon + CLI，不做完整终端 App。
- 用户已明确后续必须交付 C（完整桌面终端客户端），而不只是托盘或管理页；第四阶段同时覆盖 macOS、Linux 与 Windows，并保留 daemon + CLI 入口。
- Android/iOS 仍需要 App，因为移动系统缺少可直接复用且能承载本协议、配对、移动键盘和生命周期恢复的系统终端体验。

## 对阶段路线的影响

1. macOS/Linux：daemon + CLI，既可宿主也可控制。
2. Android：App，仅控制。
3. Windows：daemon + CLI，既可宿主也可控制。
4. macOS/Linux/Windows：完整桌面 GUI 控制客户端。
5. iOS：App，仅控制。

第一阶段无需再拆出 macOS GUI 子阶段，后续 GUI 作为独立可验收阶段规划。
