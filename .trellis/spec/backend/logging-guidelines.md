# Daemon Logging Contract

## 1. Scope / Trigger

Apply this contract when adding daemon diagnostics, changing the tracing
subscriber, or modifying `zterm logs`. Recording belongs to the existing
Session/network/pairing owners and daemon process; reading belongs to
`LocalRuntime`. There is no separate logging service or continuous reader.

## 2. Signatures and owners

```text
zterm logs [-n|--lines <n>]
LocalRuntime::log_tail(&self, requested_lines: usize) -> Result<Vec<String>, DaemonError>
lifecycle::init_lifecycle_logging()
NetworkReporter::update(update)
```

`init_lifecycle_logging` installs the daemon's text tracing subscriber. The
existing detached launcher redirects stdout/stderr to the managed daemon log.
CLI update reports typed `UpdateStage` progress to its own terminal; those
messages do not claim to be captured by a stopped daemon's subscriber.

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
  files, a writer process, remote upload, transcript or retention engine.
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
| Update committed but daemon startup fails | CLI partial-completion error; no new file writer or false full success |

## 5. Good / Base / Bad Cases

- Good: Session owner records creation and eventual end with the same ID and
  a typed reason, without recording PTY bytes.
- Base: `zterm logs -n 50` reads the last 50 available lines once.
- Bad: adding a subscriber/recorder per command or logging raw network errors,
  ticket DTOs or frame payloads for convenience.

## 6. Tests Required

Capture actual Session lifecycle and network degrade/recovery events with an
isolated subscriber (and the existing exact child-test pattern where parallel
callsite caches interfere). Assert useful correlation/reason fields, no warning for
normal detach, no duplicate unchanged-state event, and absence of sentinel
terminal/ticket/cwd content. The pair-create/replay fixture checks the actual
returned ticket and one committed event. Do not modify the global subscriber for concurrent
unrelated tests. Existing log-tail/no-autospawn tests own read limits and the
empty output contract; no follow tests are needed.

## 7. Wrong vs Correct

Wrong: `tracing::warn!(?request, ?error, "request failed")` may expose bearer or
terminal material and treats expected operations as warnings.

Correct: record a committed operation or its stable `error.kind().code()` at
its existing owner, with safe Session/local correlation when useful. Keep
ordinary lifecycle transitions at INFO and the existing subscriber/writer.
