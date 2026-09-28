# Comprehensive Zterm Logging

## Goal

Make Zterm incidents diagnosable from safe, correlated, bounded local logs: a
developer should be able to tell which operation failed, where it failed,
whether it committed, and how the client recovered. Cover the desktop
daemon/CLI and Android client rather than assuming daemon logs describe every
frontend fault.

The user explicitly requested a new branch and Trellis task, an audit of current
logging, and comprehensive improvements. Status: approved and in progress; user authorized implementation with "开始吧".

## Background and Evidence

- Baseline: `main` at `acecb45` (v0.1.36), initially clean.
- Working branch: `feat/logging-improvements`.
- [Logging audit](research/logging-audit.md) is the authoritative inventory of
  producers, events, retention, readers, evidence anchors and gaps G1–G9. This
  pass inspected source/configuration/test definitions; it did not inspect
  private deployed logs or run runtime acceptance tests. Design/validation
  sources and architectural boundaries are recorded in `design.md`.
- The existing contract lives in `.trellis/spec/backend/logging-guidelines.md`.
  Runtime rotation, Android persistence and export would extend its scope.
- Previous scope explicitly excluded `logs -f`, remote upload and transcripts;
  this proposal preserves those boundaries.

## Collection Decision and Review Status

The user subsequently approved including the existing Store timeout race found
by the logging quality gate ("好的，那你就一起改了"). This narrow addition
corrects EXPIRED versus STARTED error classification, with a deterministic owner
regression and the existing persistence/full gates; see `review.md`.

Confirmed user decision, 2026-09-28: "默认保存关键事件，详细诊断按需开启".
Both modes retain the same content exclusions. The user approved the detailed size/time/UI defaults with "开始吧". There are no remaining research blockers or separate unresolved
scope alternatives in the proposed plan.

## Requirements for Final Review

| ID | Requirement | Audit mapping |
| --- | --- | --- |
| LOG-1 | Versioned JSONL events with consistent UTC timestamps, levels, stage/outcome/category and elapsed duration. Readable CLI rendering; safe process/local operation/connection correlation plus known Session/attachment IDs. Do not promise distributed trace IDs before a Session is known. | G4, G8 |
| LOG-2 | Persist actionable failures and committed outcomes at existing owners, including attachment/service failures, early update failure, and committed revocation followed by cleanup failure. Separate ordinary cancellation/detach and deduplicate replay/adapter reporting. | G2, G3, G6 |
| LOG-3 | Release Android diagnostics across Kotlin and Rust ownership: lifecycle, network, connect/reconnect and typed terminal-state failures. | G1, G7 |
| LOG-4 | Separate key/detail capacity: desktop 8 MiB per lane, Android 4 MiB per lane, each current file plus one archive. Rotate during runtime for participating new writers; strict reader/queue/record bounds. Preserve safe paths, reset/uninstall ownership and business outcomes when logging fails. | G5 |
| LOG-5 | One-shot CLI tail/filter/archive/JSON support plus explicit local JSONL export on both platforms, with safe build context. Preserve `-n/--lines`, no-autospawn inspection and account/app isolation. Android export remains available after identity/network initialization failure. | G9 |
| LOG-6 | Key transitions/outcomes persist by default. Explicit 15-minute local detail interval, early stop/renewal/status and automatic expiry; apply without daemon restart. Resume only the unexpired interval after process restart. CLI controls and Android Settings expose the same behavior. | G7, G8 |
| LOG-7 | Maintain content safety across normal/error/dependency/crash reporting and extend operation-driven tests at changed boundaries. | G1–G9; audit section 4 |

## Acceptance Criteria

- [x] **Audit:** inventory and prioritized gaps have repository evidence and
  clearly distinguish current behavior from recommendations.
- [x] **LOG-1/2:** records from a representative operation/retry share scoped
  correlation and durations, acquire the correct Session identity when known,
  and distinguish simultaneous connections. Stages/outcomes/categories are
  useful without payloads or duplicate committed lifecycle events.
- [x] **LOG-2:** configured-user update preparation failures persist; committed
  activation/revocation followed by startup/cleanup failure is not described as
  uncommitted failure. Pre-setup commands create no log state.
- [x] **LOG-3:** a release APK retains bounded diagnostic history across app
  restart, including a connection failure and reconnect transition.
- [x] **LOG-4:** sustained writes stay within the lane budgets; detail traffic
  cannot evict key records; CLI/daemon/updater records remain readable across
  rotation. Individual records stay at most 4 KiB and queues obey both entry
  and byte limits; producer actors perform no filesystem I/O.
- [x] **LOG-4/5:** default tail returns 100/max 1,000 records with a hard 1 MiB
  input bound under concurrent appends; zero lines reads no record data. Legacy
  human output works; structured filtering/export explains skipped old lines.
- [x] **LOG-5:** CLI export refuses overwrite and Android saves to the chosen
  system document destination; cancellation does not affect terminal work.
  Export streams within retained event budgets plus a 4 KiB metadata header.
  Missing logs are explained and inspection starts no daemon or setup state.
- [x] **LOG-6:** default detail is OFF. Explicit on/off/renewal applies within
  one second; expiry stops detail automatically, restart preserves only valid
  remaining time, and no control action interrupts existing Sessions.
- [x] **LOG-6/7:** healthy use produces no per-frame/per-key/per-RTT stream;
  detailed summaries and repeated rejection are bounded. Operation-driven
  tests exclude terminal, clipboard, notification/file bodies, ticket, key, cwd
  and environment sentinels while preserving ordinary cancellation semantics.
- [x] **Compatibility:** existing managed logs survive bounded migration; unsafe
  paths and unknown files remain protected. A live old daemon is not restarted
  for logging; status identifies the pending retention cutover. New cleanup
  recognizes all new files; documented older-cleanup limitations are tested.
- [x] **Planning artifacts:** PRD convergence pass completed; `design.md` and
  `implement.md` specify scope, defaults, ordering, checks and risks.
- [x] **Planning review:** user explicitly approves the latest summary in a
  subsequent message before `task.py start` or product edits.

## Out of Scope Unless Explicitly Changed

- Terminal transcripts, typed text, file/clipboard/notification bodies,
  credentials, proofs/nonces, environment dumps and raw remote errors.
- Cloud telemetry, automatic uploads, centralized logging, upstream relay or
  encrypted data-plane changes.
- `logs -f`/watchers and general-purpose profiling/metrics systems.
- A logging-specific Session/connection state owner or changed terminal/auth
  behavior merely to support diagnostics.
- Distributed trace propagation or remote log retrieval, arbitrary dependency
  debugging, raw crash dumps, and Windows/iOS platform expansion.

## Risks and Deferred Behavior

- Multi-process rotation must replace all current writers together; an old
  running daemon remains outside the new capacity guarantee until normal
  replacement. Migration may briefly use one extra bounded file of scratch.
- Detailed collection stops on expiry/off; existing detail remains bounded until
  rotation. Retention is size-based rather than a guaranteed number of days.
- Diagnostic writes are best effort under disk failure/overload. Report dropped
  counts when possible; do not promise lossless crash/power-loss capture.
- Older cleanup tools may reject the new managed filenames. No config/database
  or remote protocol migration is needed.
- Android release-runtime and real remote-network acceptance require disposable
  selected emulator/Linux fixtures; build-only results do not satisfy them.

## Artifact Status

- `research/logging-audit.md`: source audit complete.
- `prd.md`: approved requirements and acceptance criteria verified; see `verification.md`.
- `design.md`: approved ownership, migration, limits and API contracts, now implemented.
- `implement.md`: implementation, spec sync and final quality gates complete; commit/archive and PR creation authorized by the user.
- Implementation activated after explicit user approval on 2026-09-28.
