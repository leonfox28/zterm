# Daemon Logging Contract

## 1. Scope / Trigger

Apply this contract when adding daemon diagnostics, changing the tracing
subscriber, or modifying `zterm logs`. Recording belongs to the existing
Session/network/pairing owners and daemon process, plus explicitly scoped
interactive startup and update recorders; reading belongs to
`LocalRuntime`. There is no separate logging service or continuous reader.

## 2. Signatures and owners

```text
zterm logs [-n|--lines <n>]
LocalRuntime::log_tail(&self, requested_lines: usize) -> Result<Vec<String>, DaemonError>
lifecycle::init_lifecycle_logging()
NetworkReporter::update(update)
LocalRuntime::connection_progress() -> (ProgressObserver, watch::Receiver<ProgressHistory>)
LocalRuntime::with_connection_progress(progress: ProgressObserver) -> LocalRuntime
```

`init_lifecycle_logging` installs the daemon's text tracing subscriber. The
existing detached launcher redirects stdout/stderr to the managed daemon log.
CLI update renders typed `UpdateStage` progress from the independent updater.
The one-shot update owner also appends bounded configured-update stage/outcome
records to the existing daemon log; it does not depend on a stopped daemon's
subscriber or on the PTY that update may terminate.

## 3. Contracts

- Default records are readable English text with timestamp, severity, component
  and useful operation/outcome fields. Do not enable dependency DEBUG output to
  substitute for application events.
- INFO covers daemon ready/stopping (including version/PID at startup), Session
  creation/end, attachment/detach/takeover, primary connection changes, and
  committed pairing/authorization changes. WARN/ERROR identifies an actionable
  failure through a stable domain category; normal detach is not a warning.
- Emit at the owner of a committed event. Mutation replays and adapters must not
  emit duplicate Session create/end or pair-commit records. Use the existing
  NetworkReporter to compare meaningful states; unchanged refreshes, counters,
  RTT samples and terminal frames do not generate transition events.
- `controller_detached` requires that attachment to own the existing controller
  lease. Dropping a prepared takeover attachment that never acquired control
  must not claim that the active controller detached. Check existing ownership;
  do not add a logging-specific controller registry.
- Safe Session names/IDs and local connection correlation may identify events.
  Never log terminal/clipboard content, working directories, environment values,
  identity keys, bearer tickets, proof/nonces or full request/response dumps.
  Remote path sidebands must not expose peer IDs, addresses or Relay URLs.
  Use typed failure categories rather than arbitrary peer/source error trees.
- Logging adds no Session/connection state owner, cannot fail a successful
  operation, and must not hold registry locks over formatting/writing.
- Keep the current managed files `daemon.log` and `daemon.log.1`. The launcher
  rotates at startup if the current file is at least 4 MiB, retaining one
  predecessor. This is not a runtime capacity limit. Do not add extra managed
  files, a logging service, remote upload, transcript or retention engine.
- The explicit one-shot updater is an additional writer exception. It
  validates existing state/log directories and uses `open_append` for each
  stage, so final completion after startup rotation reaches current daemon.log.
  Records contain timestamp, updater PID, authenticated target, acceptance,
  stage and outcome/category; no payload/error-tree dumps. `stage=committed`
  distinguishes committed activation; `outcome=partial_completion` reports
  subsequent startup failure while retaining the new executable. Known log
  path errors fail before stop; later diagnostic failure must not roll back a
  successful operation. Before setup, update creates no log or identity state.
- Interactive terminal startup is the other explicit writer exception. One
  observer sends the same fixed stages to the bounded first-screen journal and
  existing `daemon.log`. Records contain Unix milliseconds, severity, component
  `connection_startup`, CLI PID, per-process invocation ordinal `connection`,
  build version, stable `stage` code and typed `category`. Screen text has no
  time or shortcut hint. No raw target/name, cwd, payload or error string belongs
  in these records. Startup records are observations, not duplicate Session
  lifecycle commits; ordering does not imply another stage succeeded.
- The terminal recorder validates committed setup without creating state. It
  validates existing state/log directories and reopens with `open_append` for
  every record, so daemon startup rotation does not strand later stages in the
  archive. Missing setup creates no log, directories, identity or daemon.
  Diagnostic errors are best effort and never change connection outcomes or
  remove screen observations. INFO covers stages, ready and ordinary cancellation;
  WARN `stage=failed` includes a domain or explicit frontend failure category.
  A Session that ends before Active records `session_ended`, not user cancellation.
  Initial ready is emitted only after the effective Active input/presentation
  fence, then all observer clones stop. Later reconnect/detach cannot append
  another startup outcome. Early definitive failure/cancellation emits its final
  typed event after terminal restoration through the existing CLI owner.
- `logs` reads once without creating paths or starting a daemon: default 100
  lines, maximum 1,000 lines and 1 MiB. `-n` aliases `--lines`. Missing/empty logs
  get an English explanation; explicitly selecting zero lines remains empty.
  There is no `-f`/`--follow`, watcher, polling loop or rotation-follow state.

## 4. Validation & Error Matrix

| Condition | Required behavior |
| --- | --- |
| Missing daemon log | English explanation; no state creation or daemon start |
| Explicit zero lines | Empty selection, no claim that the file is absent |
| Large line request/file | Preserve existing line/byte bounds |
| Same network state with changed counters/RTT | No duplicate transition event |
| Ordinary controller detach | INFO event; live Session remains available |
| Prepared takeover attachment removed before acquiring control | No controller-detached event; actual controller unaffected |
| Typed operation failure | Useful component/stage/category without payload text |
| Update committed but daemon startup fails | CLI partial-completion error and updater partial-completion record; no false full success |
| Configured updater's foreground PTY ends | existing log retains stages and final outcome independently |
| Daemon startup rotates the log during update/startup | later stages reopen/append current daemon.log |
| CLI startup ends before Active | final failure/cancellation/session-ended as observed; never false ready |
| Log append fails or path is unsafe | screen continues; no external-path write or changed operation result |
| Retained connector reconnects after initial Active | no further startup records |

## 5. Good / Base / Bad Cases

- Good: Session owner records creation and eventual end with the same ID and
  a typed reason, without recording PTY bytes.
- Base: `zterm logs -n 50` reads the last 50 available lines once.
- Bad: adding a subscriber/recorder per command or logging raw network errors,
  ticket DTOs or frame payloads for convenience. An interactive startup observer
  is explicitly scoped and retired; it does not install a tracing subscriber.

## 6. Tests Required

Capture actual Session lifecycle and network degrade/recovery events with an
isolated subscriber (and the existing exact child-test pattern where parallel
callsite caches interfere). Assert useful correlation/reason fields, no warning for
normal detach, no duplicate unchanged-state event, and absence of sentinel
terminal/ticket/cwd content. The pair-create/replay fixture checks the actual
returned ticket and one committed event. Do not modify the global subscriber for concurrent
unrelated tests. Existing log-tail/no-autospawn tests own read limits and the
empty output contract; no follow tests are needed.
`configured_progress_logs_reopen_after_rotation_and_stop_at_ready` checks real
configured file records, correlation, typed failure, no target sentinel, rotation,
observer retirement, missing-setup no-creation and symlink refusal. The real
`daemon_autospawn` outer-PTY fixture asserts the complete local startup stage
sequence for its child PID; isolated progress wire fixtures cover remote stage
forwarding without assuming a live network. Broker fresh/reuse integration
assertions require the existing Linux real-Iroh fixture (ignored on macOS).
The updater's process tests own outcome persistence across frontend loss,
post-commit failure and startup rotation; no separate retention subsystem tests
are needed.

## 7. Wrong vs Correct

Wrong: `tracing::warn!(?request, ?error, "request failed")` may expose bearer or
terminal material and treats expected operations as warnings.

Correct: record a committed operation or its stable `error.kind().code()` at
its existing owner, with safe Session/local correlation when useful. Keep
ordinary lifecycle transitions at INFO and the existing subscriber/writer.
