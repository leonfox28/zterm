# Android local scrolling implementation and acceptance — 2026-09-08

The user approved the row-window/render-reuse proposal after clarifying local
cache motion, bounded waiting at missing history, and reuse of received rows.
This follow-up includes the earlier uncommitted upward-scroll correctness fix;
it does not authorize another release or activation of the user's daemon.

## Implementation

- `NativeFrameSource` exposes a bounded immutable presentation window and an
  O(1), validated viewport-source binding. `content_generation` separates row
  identity from frame metadata. `first_row` plus `window_offset` address frozen
  content without assuming that its epoch shares the current live ordinal base.
- Repository resolves changed windows off Main and shares the same Kotlin rows
  across metadata frames. Native application/ACK does not wait for Android.
  Chrome observes only state/readiness/error/notice; View observers retain exact
  source ownership before Repository releases an older handle.
- View moves immediately within the exported rows. Integer intents are conflated,
  a missing edge requests one neighbor, and blocked distance is discarded.
  A delayed window after reversal cannot replace still-valid visible rows.
  Append preserves logical reading position and stops old-basis inertia.
  Selection/hits bind to the actually drawn viewport, including local-only steps.
- API 29+ hardware Canvas retains at most three viewports plus one row of text
  RenderNodes. Equal cells reuse display lists. Cursor/preedit/selection remain
  dynamic. Font/geometry/background/detach retire lists; software/API 26–28 keep
  the same cell painter. No whole-history bitmap or new terminal parser.
- Multi-page query margins include the nearby complete physical-edge viewport
  within the existing two-screen margin allowance. Single-window desktop query
  policy, wire schema, host runtime and input protocol stay unchanged.

## Bug analysis: missing overlap at the live edge

1. **Cause (cross-layer contract / coverage gap):** the old near-live query used
   zero newer margin. A probe centered one row above Live could therefore lack
   the last live row. The new View correctly refused an incomplete replacement,
   but could remain stuck at zero despite cached rows immediately above it.
2. **Failed local adjustment:** restoring the displayed zero target after an
   incompatible response created a 0→1→0 loop. It did not add the missing overlap.
   Output append was a related coordinate hazard, not the complete cause.
3. **Root fix:** reserve enough existing margin for the full adjacent physical
   edge viewport in the multi-page query owner. Do not merge unproven mutable
   live rows in Kotlin or replay a blocked row jump.
4. **Expansion:** both live and oldest-history edges need overlap; native warmup
   and View append-during-first-wait require their own assertions.
5. **Prevention:** core edge-overlap tests, native live/neighbor binding tests,
   held-delivery Canvas tests, and shared-client/Android code-spec updates.

## Functional validation

- Full `just check`: PASS, including workspace Clippy/tests/docs, source and
  dependency policy, secret scan and relay static checks. External log:
  `/tmp/zterm-scroll-window-just-check.log`.
- Core cache: 22 focused tests pass; native Android: 15 tests pass. Added coverage
  for both edge query shapes, warm live history, stable projection IDs,
  out-of-window rejection and retained input/origin/geometry fences.
- Six `TerminalRenderingTest` cases pass. Five use actual software Canvas pixels
  for multi-row local motion, late/reversed windows, both missing edges, append,
  and append while waiting at zero. Hardware PixelCopy verifies that replacing
  one row changes only that row and A-B-A content restores the exact prior image.
- `NativeTerminalTest` passes against the explicit `presentation-fixture` host:
  Unicode, three-screen copy, history return, resize/input epoch, stale A-B-A
  source rejection, cursor modes, rename/detach and owned Session cleanup.
- All four `TerminalUiTest` cases pass across the acceptance run and focused
  Herdr rerun: real keyboard/Chinese preedit, high/low/TUI resize, final clear,
  gesture/selection/Copy/Activity retention, raw child mouse/alternate-scroll,
  and actual Herdr tab clicks plus pane history.
- Herdr's first attempt was rejected by its nested-session guard because the
  fixture daemon inherited `HERDR_*` from the parent shell. The test now clears
  only those inherited Herdr variables in its isolated command; the focused
  rerun passed in 19.312 s. No user Herdr socket/session was operated.
- Android debug/non-debuggable build, lint and JVM tests pass. API 26–28 were
  compiled/linted; their direct painter is covered by software Canvas on API 36,
  not by an actual API 26–28 device run.

Logs: `/tmp/zterm-scroll-window-{acceptance,herdr,native,core,final-build}.log`.
The initial acceptance run had 9 passing cases plus the Herdr fixture failure;
the focused rerun fixes that evidence gap. The subsequently added sixth pixel
case passed in the 7-case non-debuggable pixel/profile run (39.95 s).

## Performance evidence

AVD `Zterm_Presentation_0908`, explicit serial `emulator-5556`, API 36 arm64,
1080×2424 at 420 dpi/60 Hz, font 12, 53×56 cells, 37 px row height. Independent
fixture daemon root `/tmp/zterm-presentation-fixture-0908`. Profiling creates and
closes its own 800-row dense ASCII Session. Actual touchscreen events enter via
UiAutomation; FrameMetrics are copied on a separate HandlerThread. Two rounds
of each phase, same gesture durations and warmup. No concurrent build/test work
was run during the final adjacent old/new comparison.

The archived old non-debuggable APK already includes the ghosting correction:
SHA-256 `2df27b83deb73d54df4c2b526affa5715ab42eab448a0cda294a9f8e078bf35c`.
Compared new non-debuggable APK:
`ad2d48cd9ab9b2f5d366fdb8c92c8859c381179db7015e43850c5ee6ef0e05fa`.
Both keep the isolated `.dev` identity and debug signature; only debuggability
is disabled through `/tmp/zterm-scroll-nondebug.gradle`.

The final adjacent comparison uses `gesturesOnly=1` so the exact same current
instrumentation APK can exercise the archived Repository scroll signature.
Optional `rowWindowMetrics=1` is for the new APK only; it records content IDs
without inspecting pixels or sampling private View positions.

Drawing duration, aggregated over both rounds; milliseconds, p50/p95:

| Scenario | Old | New |
| --- | ---: | ---: |
| Redraw only | 14.61 / 17.60 | 1.29 / 2.93 |
| Slow drag | 15.30 / 19.99 | 1.87 / 4.68 |
| Fast drag | 14.59 / 18.07 | 2.45 / 10.79 |
| Reversal | 16.31 / 21.75 | 1.91 / 4.36 |
| Fling | 14.29 / 19.85 | 1.65 / 5.61 |
| All gestures | 15.01 / 20.11 | 1.79 / 5.30 |

Gesture drawing p50 decreases about 88%; p95 about 74%. These are drawing costs,
not end-to-end input latency or physical-phone FPS. All measured phases in both
APKs had zero history cache misses; background prefetch still sends bounded
requests. Fling motion continues after release through 23–29 distinct native
positions in the final new run. Fling deadline overruns decrease from 143/209
to 100/255 recorded frames; redraw overruns decrease from 62/90 to 47/109.

**Limits:** manual-drag/reversal deadline behavior is not uniformly improved.
Slow-drag frame counts are 158 old / 155 new; reversal 99 / 100. Their total frame
p50 remains roughly 84–101 ms in this emulator; the system-supplied deadlines
also vary. The draw-time improvement alone does not prove universally smoother
physical presentation. No claim of stable 60 FPS, complete stall elimination,
or actual phone acceptance is made.

A separate full new non-debuggable run includes same-row metadata requests:
55 and 52 deliveries over two 1.2 s phases share one content ID per phase and
perform zero history requests. Slow drag and reversal likewise reuse one
projected window per phase (23–45 native deliveries). Observed windows contain
155–159 rows, below the 160-row bound for this grid.

The first new Dev run contains a long UI/GPU stall and no motion in two phases;
do not use those phases as motion acceptance. The old APK also draws materially
slower now than in the earlier morning capture, so the adjacent comparison above
is the primary quantitative evidence. Keep the anomalous artifact rather than
silently treating it as a smooth run.

Accepted raw artifacts outside Git:

- `/tmp/zterm-scroll-profile-same-run-{old,new}.json`
- `/tmp/zterm-scroll-same-run-{old,new}-test.log` (33.984 / 35.428 s, PASS)
- `/tmp/zterm-scroll-profile-window-nondebug.json` (full phase/content-ID run)
- `/tmp/zterm-scroll-window-nondebug-profile-test.log` (7 cases, PASS)
- `/tmp/zterm-scroll-profile-window-debug.json` (contains the noted stall)
- `/tmp/zterm-scroll-profile-timings.py` (nearest-rank percentile report)

## Delivery boundary

Restore the ordinary debuggable development build after diagnostic comparisons.
Only the owned emulator was updated. The physical phone is not attached and was
not updated by this follow-up. No release/push/real daemon activation occurred.
Keep the overall presentation task active for the prior keyboard-close/history
refill and broader desktop/phone visual acceptance; this scroll implementation
is ready for review and the existing commit workflow.

Final restored Dev APK: version 0.1.27, versionCode 102799, DEBUGGABLE verified,
SHA-256 `8c61cdf09c9e328495929d7737ad8480710c2f945296ee4a9e0f8691941fbd1b`.
Final Dev six-pixel-case run passes in 3.946 s:
`/tmp/zterm-scroll-window-final-pixels.log`. Final build/lint/JVM passes in 6 s.

Cleanup verified: the explicit fixture root had no running Sessions; its daemon
was stopped through `presentation_fixture cli <root> daemon stop --yes`, and only
AVD serial `emulator-5556` was closed. The user's running daemon/main Session was
not addressed.


## Physical-phone development update — 2026-09-08

Authorized explicitly by the user after the scrolling implementation report.
Updated the existing `zterm Dev` package on USB device `1680fce0` (2509FPN0BC)
using `adb -s 1680fce0 install -r` and the already tested final debug APK.
Installation returned Success. Reading the installed base APK's SHA-256 matched
`8c61cdf09c9e328495929d7737ad8480710c2f945296ee4a9e0f8691941fbd1b` exactly.
Package `io.github.leonfox28.zterm.dev`, version 0.1.27 / code 102799, DEBUGGABLE;
phone lastUpdateTime is 2026-09-08 13:53:11. First install time stayed
2026-09-07 11:51:23; this was an in-place update, without uninstall/data clearing.
No Session was connected or operated. Actual phone scrolling acceptance remains
pending the user's trial; installation verification does not establish smoothness.


## 2026-09-08 — phone acceptance and v0.1.28 authorization

The user tested the installed 8c61cdf0 development APK and reported
“测试了一下非常棒”, then explicitly requested committing all changes and the
release workflow. This accepts phone scrolling experience; it does not establish
a phone FPS measurement or close the separate keyboard-close/history-refill and
desktop visual gaps. Publish the scoped scrolling improvements as v0.1.28 using
normal PR/CI/main-candidate/tag/protected-signing/publication checks. No running
Mac daemon update or interaction with its main Session is authorized.
