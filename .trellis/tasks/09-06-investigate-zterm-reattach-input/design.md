# Implemented design: initialize viewport reconciliation from the actual snapshot

## Status and classification

Investigation is complete; the user approved this bounded implementation on
2026-09-06 with “好 那你修复吧”. Implementation and direct verification are complete.
The confirmed defect is a local initialization contract violation, documented
with generic and Herdr evidence in [research/findings.md](./research/findings.md).

## Owning invariant

Before ordinary input is enabled, resize reconciliation must compare:

- actual host size: the authoritative initial snapshot;
- desired size: the latest physical terminal dimensions projected through the
  existing ChromeLayout using the snapshot's Main/Alternate screen.

A provisional creation hint is neither an accepted host resize nor an actual
size of a retained Session. Equal actual/desired sizes must not generate a
resize or turn Active back into Synchronizing. A real difference must generate
one resize and complete its existing snapshot barrier before forwarding input.

## Bounded change

Keep ResizeCoalescer as the one CLI resize deduplication owner. Reconcile or
initialize its baseline from the returned initial snapshot, then observe the
latest desired layout. Do not retain a separate hard-coded Alternate fix.

Preserve physical resizes observed while preparing/acknowledging the attachment.
Both waits retain physical sizes. After the initial acknowledgement, resample
the outer terminal, update viewport/status with the known screen, and observe
one final desired child size before the first Active event. This also handles a
ready future winning the biased select ahead of a queued SIGWINCH. Static review also
found `await_while_inactive` uses the Main-only `child_terminal_size` helper
(terminal_ui.rs:457-463, 658-659); account for that within this initialization
boundary rather than discarding pending size observations. This timing path is
a follow-up check, not the measured cause of the same-width baseline failure.

Reuse the existing shared local/remote run_view path, presentation owner,
transport state, input epoch and Session lifetime. No daemon production, wire,
keyboard protocol, copy/paste or prefix change is currently indicated.

## Required behavior

| Initial snapshot / outer layout | Result |
| --- | --- |
| Main 139, desired Main 139 | No resize; input becomes Active |
| Alternate 140, desired Alternate 140 | No resize; input becomes Active |
| Main 139, desired Main 149 after outer width change | One real resize to 149; proper barrier then input |
| Alternate 140, desired Alternate 150 | One real resize to 150; proper barrier then input |
| Physical size changes before initial acknowledgement completes | Latest correctly projected desired size wins |

Legacy and enhanced input remain byte-preserving on their existing routes.
A new attachment must accept useful keyboard input, not merely repaint the
retained screen. Detach must continue to preserve the same child and Session.

## Forbidden repairs

- Do not special-case Herdr, terminal names, key codes or screen text.
- Do not force all deltas to be snapshot acknowledgements or treat ordinary
  output as permission to reopen an unrelated pending synchronization fence.
- Do not force Active on a timer, remove input admission fences or add retry
  repaint/resize loops. Do not globally force a terminal keyboard mode.
- Do not change Session attach to resize retained children implicitly merely to
  make the frontend's provisional assumption appear true.

## Acceptance and rollback

Use the real run_terminal initializer in an outer-PTY regression. The existing
session test constructor bypasses initialization and cannot alone own this bug.
Combine retained Alternate mode, detach, initial snapshot and actual subsequent
child input. Cover the Main/new-width sibling and input received during startup.
Keep the queued-delta synchronization, copy/paste, history and cleanup tests.

A paired-route smoke should use a new disposable Session and private Herdr
state; never attach, take over or terminate the user's existing dev Session.
Record actual platform/route limits. Roll back the bounded initializer change
and tests as one unit; there is no persisted state migration.
