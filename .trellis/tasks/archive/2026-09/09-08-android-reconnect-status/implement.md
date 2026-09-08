# Execution plan

## Review Gate

- [x] Inspect Android entry/Retry, native bridge, desktop default creation, transport observations and existing tests/specs.
- [x] Record requirements, causal evidence, boundaries, compatibility and acceptance criteria.
- [x] User approved the updated plan on 2026-09-08 (好，开始吧).
- [x] Run task.py start --allow-empty-context for this inline task; load trellis-before-dev and the applicable specs. Inline mode: no sub-agents or JSONL dispatch manifests.

## Ordered Work

1. [x] Add native default attachment via existing create_main and expose actual Session ID. Share setup while preserving exact attachment behavior.
2. [x] Centralize machine-entry/Retry fallback, authoritative identity persistence, typed error handling and navigation fences.
3. [x] Project typed path/optional RTT through native actor and Kotlin metadata; retire metrics at true attachment boundaries.
4. [x] Make idle transport observations bounded and current; review cancel-safe silent-stream observation and verify preserved event/input handling. Forced idle route migration remains outside the available runtime evidence (see verification.md).
5. [x] Render fixed-height subtitle with localized connection states and fixed English `Direct` / `Relay` route labels; update generated outputs/fixtures through normal build.
6. [x] Add native and Android regression coverage for stale IDs/default recovery, header/status transitions, preserved occupancy and geometry.
7. [x] Run trellis-check directly, fix findings, update owning specs/docs and record exact evidence. Publication is outside scope.

## Validation

- cargo +1.98.0 fmt --all -- --check
- cargo +1.98.0 test -p zterm-android -p zterm-client --all-features
- cargo +1.98.0 clippy -p zterm-android -p zterm-client --all-targets --all-features -- -D warnings
- sh tests/terminal-dependency-policy.sh
- sh tools/android/build.sh :app:lintDebug :app:testDebugUnitTest :app:assembleDebug :app:assembleDebugAndroidTest
- Focused instrumentation on an explicitly selected available arm64 emulator/device and disposable paired daemon. Cover empty host, old ID after restart, valid/one/multiple/occupied Session selection, Retry, timeout/unauthorized/ambiguous outcome, Back during suspended work, direct/relay/unknown/idle/reconnect/end and healthy resize, system/Chinese/English language settings and narrow width. Assert `Direct` / `Relay` remain English in all language settings.
- Run existing OccupiedRecoveryTest and AttachGeometryTest separately with their opt-in arguments when fixtures are available. Do not close unrelated Sessions or restart non-test daemons. Record unavailable runtime checks as pending.

## Review Points

Inspect shared attachment preparation before adding selection policy; inspect final native frames and source retention before UI verification. If implementation must cross additional boundaries, update design and reconverge first. No product code changed during planning.

## Completion

Implementation and direct trellis-check are complete. All executed final checks pass. See verification.md for evidence, the isolated cold-network fixture correction, and explicit physical-device/forced-route limits. The user approved the work commit and release workflow on 2026-09-08; execute the commit-plan.md batch and finish-work before release preparation. No sub-agents were used.
