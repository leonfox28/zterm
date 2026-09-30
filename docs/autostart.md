# Daemon login autostart

```sh
zterm daemon autostart enable
zterm daemon autostart disable
zterm daemon autostart status
```

Autostart is off by default. Installation and `setup` do not enable it. `enable`
requires completed setup and installs future-login configuration without starting
a daemon now. `disable` removes that configuration without stopping a running
daemon or its Sessions. Both are idempotent. `status` is read-only and reports
missing, altered, unsafe or stale executable configuration as an error.

On macOS the owned file is
`~/Library/LaunchAgents/io.github.leonfox28.zterm.daemon.plist`. It has `RunAtLoad`
and no `KeepAlive`. It is not bootstrapped during enable, and disable does not
boot out the loaded job: bootout would kill the running daemon. At logout launchd
releases its job; the removed file is not loaded on the next login.

On Linux the owned unit is `~/.config/systemd/user/zterm-daemon.service`, with an
owned relative link in `default.target.wants`. These are the same persistent
unit/link registration used by systemd enable, installed for the next user-manager
startup without activating the current default target. `Restart=no` keeps an
explicit daemon stop stopped. `KillMode=process` lets the existing detached
updater finish after daemon shutdown; the daemon itself owns Session cleanup.
No linger setting is changed. The user manager normally starts at login and ends
after the last logout; existing administrator linger policy is not overridden.
Linux without systemd and a custom `XDG_CONFIG_HOME` are explicitly unsupported.

Both managers execute the absolute zterm path with the hidden foreground entry.
It never forks or calls `setsid`; it acquires the existing lifetime lock. Racing
manual startup cannot create a second daemon. Login startup does not create a
Session, shell or terminal window. Stopping or crashing does not schedule a retry.

Updates replace the executable at its stable installed path and leave registration
unchanged. Moving the executable requires another `enable`. Reset and uninstall
remove the owned files, including stale registration after state removal. After
stopping the daemon, they also boot out the launchd job with that exact file origin
or reload the systemd user manager. Unrecognized files and unsafe paths are refused. Service directories themselves
are preserved. The hidden entry accepts no state-path override.

The registration commands don't alter user-supplied launchd overrides or custom
systemd units. After manually changing service-manager policy, inspect it with
`launchctl print-disabled gui/$(id -u)` or `systemctl --user is-enabled zterm-daemon.service`
as appropriate. Actual login/logout behavior still needs an isolated account
acceptance run; see [verification](verification/session-continuity.md).

References: [Apple launchd process requirements](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html),
[systemd unit enable semantics](https://github.com/systemd/systemd/blob/main/man/systemctl.xml).
