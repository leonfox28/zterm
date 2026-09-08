# Android reconnect and connection status

## Goal

Make Android machine entry and Retry reach a usable terminal after a daemon restart, and show connection state, direct/relay route, and latency beside the machine name.

## Background and Evidence

- `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt:164` restores the remembered exact ID before listing Sessions; `:216` retries that same ID. A missing ID prevents fallback. `AppUi.kt:237` maps session_not_found/session_ended to the ended message.
- Daemon restart destroys daemon-lifetime Sessions. Desktop default attachment (`crates/daemon/src/client/session.rs:16`) uses the existing shared create_main contract (`crates/client/src/session.rs:197`). Ordinary named creation rejects reserved main (`crates/daemon/src/session.rs:1106`).
- `crates/android/src/terminal.rs:753` discards typed connection-status events. The header renders only the machine name (`apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalScreen.kt:55`).
- The original Android policy deliberately retained ended exact IDs (`.trellis/tasks/09-06-android-app/prd.md:50`). This request supersedes that machine-entry/retry policy; explicit row selection and takeover remain exact.

## Requirements

- **R1 — Recovery:** Prefer the remembered/current Session while it exists. After confirmed absence, refresh live Sessions: attach one unoccupied Session, offer selection for multiple/occupied Sessions, and open/create default main when empty. Apply to saved-host entry, recent card, successful pairing entry, and terminal Retry.
- **R2 — Identity and ownership:** Never infer absence from timeout, network, authorization, occupancy, or ambiguous mutation errors. Never take over implicitly. Store the authoritative returned Session ID after default attachment. Back/navigation fences late results.
- **R3 — Subtitle:** Preserve Session title and machine identity. The second line shows localized connecting/reconnecting/ended/disconnected state, or the fixed English route labels `Direct` / `Relay` with integer RTT in milliseconds, e.g. `my-mac · Direct · 23 ms`. These two route labels stay English regardless of system or app language, as requested in the planning follow-up. Unknown path/RTT is explicitly unavailable, never fabricated zero. Keep the 48 dp header and picker interaction.
- **R4 — Freshness:** Update on path/RTT changes, including idle terminals. True reconnect/end/closure retires old metrics; healthy synchronization/resize preserves established connection state. Metadata updates reuse immutable terminal rows.
- **R5 — Execution:** Main session implements and checks directly. No sub-agents.

## Acceptance Criteria

- [x] **AC1 / R1–R2:** After restarting a disposable paired daemon, saved-host/recent entry and Retry reach Active main, persist its new ID, and do not loop on the old ID.
- [x] **AC2 / R1–R2:** Valid remembered Session is reused; stale ID with one available Session reuses it; multiple/occupied candidates require selection/confirmed takeover, without unnecessary creation.
- [x] **AC3 / R2:** Timeout, unauthorized, occupied and operation_outcome_unknown do not cause blind creation. Back during list/attach/default creation does not reopen Terminal.
- [x] **AC4 / R3:** System/Chinese/English language settings retain title/host and show state, route and measured ms; `Direct` / `Relay` remain English in every setting. Long names and narrow screens keep status readable within current height.
- [x] **AC5 / R4:** Direct/relay changes, missing RTT, idle observation, reconnect, end and healthy synchronization produce correct metadata without stale metrics or redundant row conversion.
- [x] **AC6 / R5:** Native/client tests and Android lint/build plus focused instrumentation are recorded with actual runtime coverage and remaining device constraints.

## Out of Scope

Restoring killed processes; remotely starting the host daemon; automatically replacing a Session solely on shell exit or SessionEnded while the user remains idle; changes to explicit row selection, takeover confirmation, pairing, wire protocol or desktop selection; release publication. Replacement starts on machine entry or Retry.

## Planning Status

User approved PRD, design and execution plan on 2026-09-08, including fixed English Direct/Relay labels. Implementation and the full-scope check are complete; the user approved the work commit and then requested the release workflow on 2026-09-08. See verification.md for actual runtime evidence and device/route limits. Both deliverables share the native frame/Repository lifecycle boundary and terminal acceptance, so they remain one task.
