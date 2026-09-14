# Android terminal peer comparison

Research date: 2026-09-10. The user asks whether established Android terminal/SSH
apps use the same keyboard resize and diff mechanisms, and what Zterm can borrow.
This is source/documentation research only: no competitor APK, emulator, phone,
remote Session or daemon was operated. No product code or development APK changed.

## Source identity and limits

Inspect official repositories, then check released tags instead of treating
development branches as installed versions. Public source was downloaded to
`/tmp/zterm-peer-review-0910`; no dependency was added to Zterm.

| Project | Released source inspected | Development head cross-check |
| --- | --- | --- |
| Termux | v0.118.3, `5b657c6adf4304e5198951ce815fe0205dcac29c` | `3b66f8799635a4dba4a206563048ff0e6792c487`, 2026-08-24 |
| ConnectBot | v1.10.9, `21ce6f47cf16785bf2314b4ea931900164c179c1`, released 2026-06-09 | `31f6cf825da5fc9987efbd6ff01a98236691cba9`, 2026-08-09 |
| ConnectBot termlib | 0.1.0, `e3f4bdc3b3b5563fee54b0eca4b50d0e611bfd07`; dependency declared by ConnectBot v1.10.9 | `735c6dc646a845099f34083a15bd295be145a6ce`, 2026-08-09 |

Termux also has a v0.119.0-beta.3 release; v0.118.3 is the stable upstream source
used here. This does not identify every Play/F-Droid installation. The checked
mechanisms below are present in the released sources, not only current heads.
[Termux releases](https://github.com/termux/termux-app/releases),
[ConnectBot release](https://github.com/connectbot/connectbot/releases/tag/v1.10.9),
[ConnectBot dependency versions](https://github.com/connectbot/connectbot/blob/21ce6f47cf16785bf2314b4ea931900164c179c1/gradle/libs.versions.toml#L26).

No public core renderer/IME implementation was found for Termius or JuiceSSH.
Termius's official Android page documents its terminal/keyboard features, while
Sonelli's public repositories include plugins and an Android Mosh port. Neither
establishes their Android row-cache granularity, animation timing or GPU backend.
JuiceSSH's site/store pages were unavailable to the web fetcher; do not infer
product status from that. Exclude similarly named unofficial Termius repositories.
[Termius Android](https://www.termius.com/free-ssh-client-for-android),
[Sonelli repositories](https://github.com/sonelli).

## What the source establishes

**Termux owns the terminal emulator locally.** View size changes calculate rows
and columns, then update the local PTY and emulator. Local grid resizing can
complete before a remote SSH application repaints; this alone proves no animation
or network latency advantage in a particular scene.
[View sizing](https://github.com/termux/termux-app/blob/5b657c6adf4304e5198951ce815fe0205dcac29c/terminal-view/src/main/java/com/termux/view/TerminalView.java#L958),
[session resize](https://github.com/termux/termux-app/blob/5b657c6adf4304e5198951ce815fe0205dcac29c/terminal-emulator/src/main/java/com/termux/terminal/TerminalSession.java#L103).

Its same-width resize path moves the ring-buffer origin, considers blank rows
below the caret on shrink, and restores available transcript on growth. Alternate
history remains zero. This is an emulator-owned content policy, not Zterm's
unconditional local height difference with a visible-caret top constraint.
Copying its blank-row decisions into Android alone would add disagreement with
Zterm's authoritative host model.
[buffer resize](https://github.com/termux/termux-app/blob/5b657c6adf4304e5198951ce815fe0205dcac29c/terminal-emulator/src/main/java/com/termux/terminal/TerminalBuffer.java#L203).

The renderer traverses visible rows on each invoked render, groups adjacent text
by style/cursor/selection and compatible measured widths, then submits text runs.
Combining scalars stay attached to their base; width mismatches break runs. This
is batching within a draw, not evidence of row-damage caching or a character-diff
renderer. No equivalent of Zterm's content-keyed row RenderNodes appears in this
inspected renderer.
[Termux renderer](https://github.com/termux/termux-app/blob/5b657c6adf4304e5198951ce815fe0205dcac29c/terminal-view/src/main/java/com/termux/view/TerminalRenderer.java#L57).

**ConnectBot provides a direct precedent for incremental row preparation.**
Its local libvterm wrapper accumulates damage, rebuilds affected rows, keeps
unchanged immutable line objects and reuses the scrollback list until it changes.
With the default Main looper it schedules preparation via Choreographer, merging
pending work before building the next snapshot; other loopers use Handler posts.
On resize it rebuilds all lines, so ordinary row reuse is not cross-size content
reuse. These are code mechanisms, not independently measured FPS results.
[termlib preparation](https://github.com/connectbot/termlib/blob/e3f4bdc3b3b5563fee54b0eca4b50d0e611bfd07/lib/src/main/java/org/connectbot/terminal/TerminalEmulator.kt#L883).

The UI uses separate per-row Canvas compositions keyed by row/content metadata,
and a separate cursor/selection overlay. Its resize path derives dimensions from
available pixels and calls the local emulator. This shares the row/overlay
separation with Zterm but uses different cache identities and rendering owners.
[termlib rows](https://github.com/connectbot/termlib/blob/e3f4bdc3b3b5563fee54b0eca4b50d0e611bfd07/lib/src/main/java/org/connectbot/terminal/Terminal.kt#L1619).

ConnectBot's console uses `imeAnimationTarget` in its layout insets; the emulator's
resize callback enqueues the SSH dimensions operation. This supports using a
known final IME target. The inspected path directly bases layout on that target;
it does not demonstrate Zterm's separate moving old grid and retained-source
handoff. Do not infer its physical animation trajectory from source alone.
[console insets](https://github.com/connectbot/connectbot/blob/21ce6f47cf16785bf2314b4ea931900164c179c1/app/src/main/java/org/connectbot/ui/screens/console/ConsoleScreen.kt#L536),
[transport callback](https://github.com/connectbot/connectbot/blob/21ce6f47cf16785bf2314b4ea931900164c179c1/app/src/main/java/org/connectbot/service/TerminalBridge.kt#L308).

**Mosh is the closer protocol-level comparison.** Traditional SSH transports
terminal bytes for a client-side emulator; Mosh synchronizes screen state and can
skip intermediate states while converging on the server's latest screen. This
supports the general server-state approach, not adopting Mosh's UDP transport,
prediction, scrollback limitations or assuming Zterm deltas can be dropped.
[Mosh design](https://mosh.org/#techinfo).

## Zterm implications and recommended order

The existing wire delta, immutable presentation window and row drawing reuse
are separate stages. `AppRepository.kt:249` already reuses rows for equal content
generations and resolves changed windows off Main. However, a changed window
still calls `presentationRows()`; `terminal.rs:1232` projects each row/cell in
that window. The observer is not explicitly paced to display frames.
`TerminalRowRenderer.kt` later avoids recording equal row content; that cannot
recover preparation work already spent. `TerminalView.drawRow` still submits
glyphs per nonempty/non-space cell inside newly recorded rows.

1. **Prioritize row sharing and preparation coalescing as one measured follow-up.**
   Preserve unchanged row identities through native projection/FFI, and prepare
   only the latest eligible visible candidate at display opportunities. Preserve
   every required wire install/ACK, immutable source/Copy semantics, epochs and
   bounds. Do not move expensive conversion onto Main merely because ConnectBot
   uses a Main Choreographer callback. This makes the existing PERF-1 proposal
   concrete; measure cursor-only, sparse and dense updates before acceptance.
2. **Evaluate style-run painting as a smaller independent optimization.** Merge
   compatible adjacent text/background work inside a changed row. Start with a
   narrow equal-metric case, retain complete-cell clipping and wide/combining
   fallback, and verify hardware pixels and cold recording cost. This targets
   the measured endpoint row-recording work; it is not a finer wire diff.
3. **Keep explicit IME endpoints and the moved/drawn-source distinction.** The
   competitor evidence supports final-target sizing and local responsiveness,
   but does not establish Zterm's exact bottom-anchor rule as an industry norm.
   Reconcile local/host resize row correspondence before claiming no jump in
   every case. `unified-ime.md` already reproduces differing high-caret layouts;
   an exact diff correctly displays those differences. Do not hide them with a
   timeout, guessed blank-screen completion or a second terminal parser.

The user's requested common movement policy remains implemented for evaluation.
This comparison authorizes no additional product change, renderer replacement,
cell-patch protocol or physical-device performance claim. Preserve it as research
for the next explicitly scoped implementation.
