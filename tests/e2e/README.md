# End-to-end test boundary

Use explicit disposable hosts and emulator app storage. Never inject test state
into the user's `~/.zterm`. Relay checks live in `tests/relay/`; runtime evidence
and remaining real-device/network gates live in
[session continuity verification](../../docs/verification/session-continuity.md).

## Android background connection and process recovery

Build `zterm-cli` example `presentation_fixture` and run `init <new-root>` then
`start <root>` in a separate process. All host state and Sessions belong to that
root. Wait for `cli <root> status` to report network Online before `pair <root>`.
The pair action writes a private ticket file; don't print or commit its contents.

Use a disposable emulator or a read-only emulator instance, install the debug
APK and instrumentation APK, and copy that ticket via `adb shell run-as` into
`files/background-ticket.txt` of `io.github.leonfox28.zterm.dev`. Regenerate a fresh
ticket before each test that pairs, removing only the prior fixture ticket file.
Select an explicit emulator serial for every adb command.

Run the instrumentation class
`io.github.leonfox28.zterm.BackgroundConnectionTest` with `-e backgroundFixture 1`:

- `#connectionSurvivesActivityAndServiceLifecycle` tests a real Session, foreground
  start eligibility, notification grant/denial, lock/unlock, Activity recreation,
  Chinese IME API commits, notification detach, lost control and Session end.
  Configure POST_NOTIFICATIONS with `pm grant` / `pm revoke` **before** starting
  instrumentation and pass `-e notificationVisible 1` / `0` respectively. Android
  kills a running process on revocation, so changing permission inside its test
  runner is not a service-start failure. After initial pairing, repeated runs can
  use `-e reuseBackgroundHost 1` with the saved `presentation-fixture` host.
- `#processRecoveryNeverCreatesOrSubstitutesSession` is coordinated across separate
  process invocations. First use `-e restoreMode prepare` with a fresh ticket.
  Force-stop the application; confirm that no service restarts by itself.
  Run `resume`, then force-stop and run `occupied`. Both retain the original ID.
  On the disposable host explicitly close that Session (`cli <root> session close
  main -y` on a fresh fixture), force-stop the app, and run `ended`. The latter
  must show Session-not-found on both restoration and Retry, with no new Session.

`NativeTerminalTest -e hostName presentation-fixture` can then verify Unicode,
resize, history and detach against the same disposable host. Run normal UI/input
and store tests separately; tests that require additional fixtures report skips.

For transport interruption, use an emulator that supports `adb root`, then run:

```sh
python3 tests/e2e/android-network-interruption.py \
  --serial emulator-5580 --output /tmp/zterm-network-interruption.log
```

Choose the actual disposable emulator serial; `--adb` can select the SDK's adb.
The coordinator waits for established output before dropping only the debug
app UID's IPv4/IPv6 traffic, and removes its uniquely tagged rules in `finally`.
The test waits for `reconnecting`, confirms input is rejected, then confirms
the original Session and shell variable survive restoration without replaying
the rejected input. Read-only reachability retries prepare this fixture; it is
not a first-connection latency test. No mutation is retried with a new ID.

Stop the disposable daemon with `cli <root> daemon stop -y` after acceptance.
This proves emulator/temporary-host behavior, not actual login, a phone's power
policy, cellular handover or signed release upgrades.
