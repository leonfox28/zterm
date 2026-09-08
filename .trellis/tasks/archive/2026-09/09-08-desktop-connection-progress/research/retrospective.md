# Bug Analysis: physical palette probe and startup feedback

## 1. Root Cause Category

- **E — Implicit assumption:** bounded incoming query size and documented OSC
  multi-query support were assumed to imply safe expanded replies.
- **D — Test coverage gap:** query-slot substring tests and correct client reply
  decoding did not execute the physical terminal's response allocation path.
- Ghostty 1.3.1's per-OSC 1 KiB allocator fails during ArrayList growth for a
  32-index RGB response. Processing abandons a slice and query tails can become
  visible. The existing presenter/round/decoder architecture is sufficient;
  replace compound palette commands with complete single-index commands.

## 2. Rejected explanations and discriminating evidence

- Both local and remote routes query before Session preparation; relay latency
  does not explain query payload characters in the outer display.
- Existing client reply tests passed; the screenshot contains outgoing `?`
  queries rather than returned RGB payloads.
- A 197-byte query fits the upstream OSC parser's 2048-byte input buffer. The
  decisive limit was the separate response allocator.
- Splitting writes at 1024 did not fix the physical reproduction. Single-index
  commands returned a stationary cursor; the batch cases produced matching
  OutOfMemory errors in the macOS diagnostic log.
- No speculative product patch was used to reach this conclusion. A progress
  repaint alone would conceal the bytes while leaving observation broken.

## 3. Prevention Mechanisms

| Priority | Mechanism | Action | Status |
| --- | --- | --- | --- |
| P0 | Output contract | One palette index per OSC, one overall round/flush/fence | Implemented |
| P0 | Regression | Actual emitter framing test fails against the original emitter | Verified |
| P1 | Display ownership | Startup commits physical coverage but no semantic/history baseline | Implemented and replay-tested |
| P1 | Lifecycle regression | Pending stage display, cancel/output failure, retained Session ID | Verified |
| P1 | Durable spec | Physical response budget and initial presentation contracts | Updated |

## 4. Systematic Expansion

- Lifecycle refresh and reconnect use the same emitter, so the correction covers
  them without a second query implementation.
- Default/special-role commands are already small; no arbitrary global batch
  framework, terminal brand switch or protocol extension is justified.
- Allocation growth/headroom matters in addition to final payload length. Test
  the consumer's behavior where possible and preserve physical smoke evidence
  separately from headless replay.

## 5. Knowledge Capture

- Updated `.trellis/spec/backend/terminal-colors.md`.
- Updated `.trellis/spec/backend/local-daemon-ipc.md`.
- Retained user-run probe bytes/results and the root-cause trace in this task.
- This application repository has no `src/templates/markdown/spec/` mirror;
  there is no template copy to synchronize.
