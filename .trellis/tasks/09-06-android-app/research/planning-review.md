# Planning Review

Reviewed on 2026-09-06 against the current user decisions and baseline
`68eab39087ec1e564dec536cf01c7664da5e211c`.

- Goal and value: a phone controller that manages host Sessions, works in the
  live terminal, and reads/copies historical output.
- User-requested scope preserved: camera QR, lower-left album, lower-right
  ticket, Session creation/rename/close/switch, touch selection/copy, history
  scrolling, system IME plus eight shortcuts, background connection retention,
  local emulator development, Xiaomi 17 Pro Max acceptance, APK without store
  release. The new scope answer does not approve the prior narrower final plan.
- Requirements have observable acceptance checks. Existing PAIR, INPUT, and
  LIFE IDs and repository source anchors were preserved through convergence;
  CORE and DELIVERY IDs make the overall journey/delivery checks explicit.
  SESSION, SCROLL, and SELECT IDs now cover the three additions and map to
  acceptance cases and an implementation/check owner.
- The latest clarification supersedes the single-screen selection proposal.
  SELECT-1/3 now cover offscreen content anchors, edge auto-scroll, cross-page
  pinning, and captured text under new output; SCROLL-4 explicitly requires
  multi-page local cache, proactive prefetch, and request-independent cached
  gesture rendering. New acceptance covers page seams, delayed replies, cache
  request counts, native frame behavior, and memory/selection-pin bounds.
- PRD convergence pass completed: read top to bottom, removed resolved open
  questions and superseded proposals, moved technology/execution ownership to
  the design/plan, and retained the user-selected background behavior. Removed
  obsolete exclusions and host-only creation instructions from all current
  planning artifacts; historical research now points at the expanded PRD.
- Design specifies native UI, shared Rust client extraction, generated bridge,
  authentication, mutation/replay ownership, frame/ACK/presentation identity,
  bounded multi-page history and prefetch, native gesture ownership/cross-screen
  selection/copy, storage, keyboard
  handling, QR output, build configuration, and rollback boundaries.
- Cross-screen continuous selection and native local-cache scrolling are part
  of the revised proposal. Concurrent terminal tabs, host-driven clipboard,
  unlimited local history, and full-history export remain explicit exclusions.
- Current core cache stores one window and current range coordinates describe
  visible rows. The design now explicitly extends these shared owners; it does
  not claim that their existing APIs already implement the mobile behavior.
  Host epoch churn and missing historical ranges remain honest limits: captured
  text is preserved, but incompatible/missing pages cannot be guessed or joined.
- Keep one integrated App task with ordered cache/range work and independently
  checkable gates; the additions serve the same accepted mobile release flow.
- Plan orders baseline evidence, native bridge proof, shared extraction,
  pairing, Session management/terminal/lifecycle, native history/selection,
  and APK/phone acceptance. Gates are explicit.
- New library versions were checked for publication. Combined build, FFI,
  camera, network, rendering, APK update, and real-phone results remain
  execution work; none is reported as passed.
- Existing macOS/Linux six-path runtime prerequisite remains unproven by this
  planning pass. Its evidence must be reconciled before Android implementation.
- Local validation confirms required artifacts/IDs/research references and a
  `planning` task status. Only task artifacts have been changed.

Next state: present the complete planning summary for user review. Do not run
`task.py start` until a subsequent message approves that summary. Inline mode
does not require JSONL curation or implementation/check sub-agent dispatch.
