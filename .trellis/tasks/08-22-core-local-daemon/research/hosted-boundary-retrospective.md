# Hosted Platform Boundary Retrospective

## 1. Root Cause Category

- **Category**: C/D/E — change propagation, test coverage, and implicit
  assumptions.
- **Specific cause**: the cross-UID harness assumed the CI checkout path was
  searchable by `nobody`; the Windows boundary gated the listener but not all
  of its private service implementation.

## 2. Why Earlier Fixes Failed

1. Running the foreign-UID helper in `target/` fixed the code path but not the
   parent-directory traversal contract.
2. Gating only Unix callers fixed missing-symbol errors while leaving private
   fields/helpers visible to Windows `-D warnings`.
3. Local Windows cross-compilation stopped in `ring` without a Windows SDK, so
   it could not substitute for a native hosted runner.

## 3. Prevention Mechanisms

| Priority | Mechanism | Specific action | Status |
| --- | --- | --- | --- |
| P0 | Integration test | Copy the helper into one searchable private fixture directory, execute it as `nobody`, require zero reply bytes, then prove the owner still succeeds. | Done |
| P0 | Compile-time | Gate all private Unix imports/state/helpers while retaining the intentional public unsupported-platform API. | Done |
| P0 | Hosted CI | Require both Linux architectures and the native Windows shared-contract job. | Done |
| P1 | Documentation | Record both contracts in the project specs. | Done |

## 4. Systematic Expansion

- **Similar issues**: future helper binaries run under another account and any
  platform-specific daemon module can repeat these failures.
- **Design improvement**: native implementation details stay behind one cfg
  boundary; shared types remain small and explicit.
- **Process improvement**: local cross-builds are diagnostics; only a native
  hosted job closes the target-platform acceptance item.

## 5. Knowledge Capture

- [x] Updated `backend/local-daemon-ipc.md`.
- [x] Updated `guides/cross-platform-thinking-guide.md`.
- [x] No `src/templates/markdown/spec/` mirror exists in this repository, so
      there is no generated template to synchronize.
