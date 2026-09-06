# Implementation and verification record

Implemented inline on 2026-09-06 in mundane-moth, from 4b5ec91 (v0.1.21).
User explicitly requested no subagents; none were started for implementation or
review. All runtime fixtures use temporary state. No production daemon, remote
host, deployment, release or real ~/.zterm state was changed.

## Delivered

- One engine-owned color state, bounded core/wire observations and effective
  snapshots, all 256 palette entries plus six roles, Unknown/RGB/Dynamic values,
  per-revision color metadata and unknown source provenance.
- Exact OSC set/query/reset families and OSC 21, licensed X11 names and numeric
  forms, ten-slot stack and aliases, reported appearance and 2031 subscription,
  supported mode queries, SGR DECRQSS and XTGETTCAP.
- Underline shape/color projection and rendering; mixed SGR 58/59 no longer
  discards foreground, background or reset siblings.
- Bounded outer probe before creation/attach, reply framing through epochs,
  owned subscription restoration, typed current-controller updates and latest
  observations on reconnect.
- Semantic palette repaint of live/history content, preserved selection and
  literal RGB, explicit software cursor over real wide/combining glyphs.
- Ordered driver commits, creation fingerprint/profile before startup queries,
  staged takeover profiles and replacement snapshot ACK input gate.
- Wire v2 canonical create209/attach323/update324; old create202/attach300
  retired. Matching CLI, viewer daemon and target daemon binaries are required.
- Executable color spec plus synchronized existing owner specs; bounded neutral
  color fixture and physical smoke recipe under tests/foundation.

## Acceptance evidence

| AC | Authoritative evidence and assertions |
| --- | --- |
| A01 | terminal/tests/color_protocol.rs: all 256 and special roles, numeric/name parsing, modern negative replies, every two-way split for set/query/set, invalid and C1 controls |
| A02 | host_colors tests: 250 ms/64 KiB bounded probe, partial/missing boundary and coalescing; named_create_supplies_colors_before_startup_query_and_fingerprints_them waits for exact child reply before any attach |
| A03 | appearance_notifications_only_follow_external_base_changes; driver query/base ordering test verifies canonical old query reply precedes new 997 |
| A04 | session_color_tests: active/pending/old-controller authorization, takeover replacement ACK, override/base/reset across detach; authenticated remote color initial snapshot/zero-row update; existing controller_lease/local IPC/recovery and reconnect suites |
| A05 | palette_only_updates_retain_grid_history_epoch_and_carry_color_revision at full history capacity; palette_only_delta_recolors_live_and_pinned_history_without_losing_selection; presenter delayed-history and failed-output semantic baseline test |
| A06 | underline_shapes_and_colors_preserve_mixed_sgr_siblings_and_report_current_pen covers 31;58, 0;59;48, indexed/RGB and every shape; semantic DTO/presenter paths and existing corpus |
| A07 | software_cursor_preserves_wide_combining_glyph_and_repairs_old_span, explicit selection tests, unknown_default_sources_reverse_once_and_explicit_underline_stays_literal and mode-5 model fixture |
| A08 | stack_aliases_share_ten_slots_and_retain_unobserved_sources: depth/slots/reports, Unknown provenance, old saved base restored after new base, reset to new base; dynamic/reset cases |
| A09 | host_colors codec fixtures: BEL/ST/C1, UTF-8, prefix, literal paste, old epochs, oversized control drain; existing reader fencing, history resume and exact-once paste fixtures |
| A10 | core/proto strict required metadata and retired kinds; color_base_commits_cannot_overtake_a_query_blocked_on_pty_io; presenter transactional failure; remote exact attachment rejection |
| A11 | terminal_guard_restores_only_owned_appearance_subscription; per-Session model ownership and existing normal/error/signal real-PTY guard/autospawn fixtures; physical setters absent from the presenter |
| A12 | Final native just check result recorded below; physical GUI/theme smoke and Linux evidence remain explicitly separate |

## Review findings fixed

1. Full-capacity history conservatively advanced its epoch on every ingest.
   Color-only ingress now records that no grid input occurred; it carries colors
   without evicting or refetching history.
2. A fresh takeover briefly inherited ever-active permission before its new
   color snapshot ACK. Preserve prior readiness and wait for the exact new ACK.
3. Comparing an earlier revision watch against a base update confused concurrent
   PTY output with a color change. Driver now returns Option<Revision> based on
   before/after under its commit/model locks; unchanged base produces no wake.
4. Stale Active/Awaiting/revision work could race takeover and close the stream
   before LeaseLost delivery. Internal update polling defers to the lifecycle
   event; a deterministic stale-notification regression asserts the wire event.
5. Existing outer-frame replay fixture echoed new standard query replies into
   its own completion marker. Disable only that fixture's PTY echo and retain
   its semantic screen assertions; daemon_autospawn passes.
6. Host control drains retain CSI/OSC framing across oversized input, UTF-8 ST
   continuation and paste markers. Aggregate epoch-retained input is bounded.
   An accepting probe round cannot publish partial observations.
7. Boxed fixed metadata arrays avoid inflating enum/command stack footprints
   while maintaining exact 262-entry validation. No dependencies were changed.

## Verification status

Final native `just check` completed successfully (exit 0) on macOS arm64 with
Rust 1.98.0. Rust test summaries: 560 passed, 0 failed, 7 ignored across 48
groups. Ignored entries are existing isolated child helpers and Linux-only Iroh
runtime tests; harness-free integration gates also completed successfully.

The gate passed source/version/dependency policy, formatting, actionlint,
release/static/operator/candidate fixtures, shell/Python checks, Clippy with
warnings denied, secret scans, workspace tests, docs, cargo-deny for workspace
and relay probe, relay probe lint, upstream checksum verification and available
relay Compose static checks. Full log for this session: /tmp/zterm-just-check.log.

Focused model/proto/core, CLI observation/presenter/selection/reader, driver,
Session startup/takeover, authenticated remote, reconnect and deterministic
stale-lifecycle regressions passed. New Python fixture syntax and isolated PTY
execution were checked separately. No broad checks were repeated after this
green gate; remaining edits only record verification and commit planning.

The Python swatch fixture passed an isolated PTY run: all 256 query entries,
bounded response collection, escaped reports, SGR swatches, and termios restore
(macOS PENDIN normalized consistently with the existing native test contract).
This is harness evidence, not a physical GUI or Herdr/Pi smoke result.

Physical outer-terminal light/dark and same-appearance theme switching, actual
Herdr/Pi appearance, and real remote-host deployment were not run. Other supported
Linux/macOS hosts, glibc, Docker/QEMU and release evidence retain existing CI owners.
Unknown outer colors/appearance are never fabricated; a missing DSR boundary
prevents overlapping refresh rounds. Custom cursor is a steady software block;
native shape/blink customization and theme UI/presets remain out of scope.

## Commit boundary

All changes belong to this task. On 2026-09-06 the user authorized the commit
and release workflow. Implementation evidence is complete; proceed through the
work commit, task archive/journal, v0.1.22 preparation, exact PR/main CI and
protected signed publication. Physical GUI/Herdr smoke remains unperformed.
