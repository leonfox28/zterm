# Local logs and diagnostics

Zterm saves key events by default on desktop and Android release/debug builds:
startup/shutdown, Session and controller changes, pairing/revocation, network and
route changes, connection/reconnect, upload/update outcomes, and frontend failures.
Records include safe IDs, timestamps, stable error categories and operation timing.
Terminal text, typed input, clipboard/notification/file contents, credentials,
paths and network addresses are excluded in both modes. Logs stay on this device.

## Inspect desktop logs

```sh
zterm logs -n 100
zterm logs --level warn --since 30m
zterm logs --component connection --session <session-id>
zterm logs --include-debug --json
```

The default is the latest 100 key records; maximum selection is 1,000 lines from
1 MiB of input across current/archive files. Including detail reserves half that
input budget for each lane. Filters apply to that bounded tail, so they do not
search the full retained history. `--level` is a minimum severity; `debug` includes
detail. `--since` accepts positive relative s/m/h/d durations. `-n 0` is empty.
These commands never start a daemon or create setup state. There is no follow mode.

Old text remains readable in unfiltered human output. JSON output, structured
filters and exports include only recognized safe records; skipped/truncated data
is reported. JSON records contain a schema/version/process envelope and optional
context in `fields`. Human output renders these as English event descriptions.

## Enable detail for a problem

```sh
zterm logs debug on
zterm logs debug status
zterm logs debug off
```

`on` enables or renews a **15-minute** local interval; `off` stops collection early.
A running recorder observes changes within about one second when storage is
available. No restart or Session interruption is required. Restarting a process
preserves only the valid remaining interval; malformed/expired controls are OFF.
Detailed startup stages and bounded synchronization/input/viewport observations
use a separate file budget. Turning detail off does not delete saved detail.

Android Settings → Diagnostics provides the same switch, remaining time and
renewal control, plus Export logs. It remains usable when identity/network setup
fails. Each application variant has its own private logs and control state.

## Export explicitly

```sh
zterm logs export --output ./zterm-logs.jsonl
zterm logs export --output ./zterm-detail.jsonl --include-debug
```

Desktop export creates a new 0600 file, refuses overwrite and destinations inside
managed state. Android uses the system document picker; select “Include saved
detailed diagnostics” when needed. Cancelling leaves terminal work unchanged.
The JSONL header includes build/platform and omitted/truncated/pending-loss counts.
Only validated event records follow; old text or arbitrary unknown fields are
excluded. No config, identity or full status dump is included. Sharing the result
is your explicit action; Zterm does not upload it.

## Retention and older versions

| Platform | Key events | Detailed diagnostics | Total event files |
| --- | --- | --- | --- |
| Desktop | 4 MiB current + 4 MiB archive | 4 MiB current + 4 MiB archive | 16 MiB |
| Android | 2 MiB current + 2 MiB archive | 2 MiB current + 2 MiB archive | 8 MiB |

Files rotate during runtime. Detailed traffic cannot evict key history. Retention
is based on size, not days. Desktop files live below the effective account's
`~/.zterm/logs`: `daemon.log[.1]`, `daemon.debug.log[.1]`, `diagnostics.json` and
`writer.lock`. Android files are private and excluded from backup. Control files
are at most 1 KiB; records are at most 4 KiB. Bounded queues/sink failures may drop
events; recovery emits a loss/suppression count. This is best-effort diagnostics,
not a lossless audit trail or a terminal transcript.

A daemon already running an older binary keeps its current file descriptors.
`logs debug status` identifies pending retention cutover; the cap applies after
normal daemon replacement. Logging does not force a restart. Oversized old files
are then reduced to complete recent lines. Current reset/uninstall recognizes all
managed files; an older cleanup binary may refuse the new filenames. No identity,
configuration, database or remote protocol migration is required.
