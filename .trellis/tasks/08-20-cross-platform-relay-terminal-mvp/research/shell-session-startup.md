# Session Shell 启动约定

## 参考实现观察

- Zedra 的 Unix PTY 路径优先读取显式配置，其次使用 `$SHELL`，最后回退到 `/bin/bash`，并用 `-l` 启动 Shell；它还允许用 shell 字符串包装启动命令（`/Users/huyuanzhe/projects/zedra/crates/zedra-host/src/pty.rs:27-106`、`:152-171`）。这证明 login shell 可以恢复用户环境，但继承 daemon 的 `$SHELL` 和字符串命令都不适合作为 zterm 的长期协议约定。
- Herdr 固定调查提交 `9d7b6c24c4d251a62a861f37c2c394078e083ca8` 的 [`src/pane.rs`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/src/pane.rs)支持配置 Shell 和 login/non-login/auto 模式；auto 在 macOS 使用 login shell，而其他 Unix 平台使用普通交互 Shell。它说明平台可以采用不同默认值，但这会让同一个 zterm 产品在 macOS 与 Linux 上加载不同的用户启动文件。
- 后台 daemon 的环境不等同于用户当前打开的交互终端；第一阶段虽由本地命令 detached-spawn，仍会关闭 stdio、脱离控制终端并使用稳定运行目录。session 若直接继承 daemon cwd 或 `$SHELL`，重新拉起后可能得到意外目录或过期环境；未来改用 launchd/systemd 时同样存在这一问题。

## 已确认的 zterm 行为

- macOS/Linux 第一阶段统一启动当前 OS 账户配置的交互式 login shell，使两平台的用户环境语义一致。
- 默认 cwd 是该用户 home；`session new` 可以显式指定宿主路径 `--cwd`，无效路径在 PTY 创建前失败。
- daemon 的服务启动 cwd 和 `$SHELL` 不作为默认 session 语义。
- 第一阶段不支持 session 创建时直接执行任意命令。用户 attach 后在持久 Shell 中启动程序，确保 Codex、Herdr、tmux 等前台程序退出后仍返回同一 Shell，而不是结束 session。

## 取舍

login shell 会执行用户自己的登录启动文件，因此启动速度和错误行为受用户配置影响；但它比继承后台服务环境更符合“连接后得到自己的正常终端”的预期。暂缓任意启动命令减少 argv/引用规则、根进程退出语义和配置兼容面，代价是自动化场景需要先 attach 再输入命令。
