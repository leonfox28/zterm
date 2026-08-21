# PTY wait-lock retrospective

## Bug Analysis: `wait()` starved terminal query replies

### 1. Root Cause Category

- **Category:** E — implicit assumption, with a D integration-test signal.
- **Specific cause:** `TerminalDriver::wait()` constructed the PTY mutex guard
  inside a `match` scrutinee. Rust kept that temporary alive through the
  selected arm, including the polling sleep. The polling thread could then
  reacquire the mutex repeatedly while the model owner was waiting to write a
  DSR reply. The child waited for that reply, so the test timed out.

### 2. Why earlier fixes did not hold

1. Replacing the original blocking child wait with `try_wait()` polling removed
   the indefinite hold but did not end the guard before the sleep.
2. An isolated rerun passed, which hid mutex starvation because scheduling made
   the model owner win that time.
3. Replacing the fixture's `stty` subprocess with direct termios removed one
   possible source of nondeterminism but did not address the held guard.

The discriminating evidence was the queue state on failure: one chunk had been
read and removed from the queue, but zero chunks had completed processing. That
placed the stall after ingest and before reply completion, at the PTY mutex.

### 3. Prevention mechanisms

| Priority | Mechanism | Specific action | Status |
| --- | --- | --- | --- |
| P0 | Architecture | End the PTY guard in an explicit lexical scope before polling sleep | Done |
| P0 | Integration test | Keep a real raw-PTY DSR child whose exit depends on the model reply | Done |
| P1 | Diagnostics | Include queue progress and revision in deadline failures | Done |
| P1 | Documentation | Record the temporary-lifetime rule in `terminal-driver.md` | Done |

### 4. Systematic expansion

- Audit future daemon polling code for mutex guards created in control-flow
  scrutinees whose arms wait, sleep, retry, or perform I/O.
- Keep child observation and PTY reply/input ownership separable; attachments
  and transports still receive no PTY ownership.
- A flaky concurrency test is a failed gate, not evidence for adding retries.

### 5. Knowledge capture

- Updated `.trellis/spec/backend/terminal-driver.md` with the lexical-scope
  contract, deterministic fixture boundary, and correct polling example.
- Retained the integration regression and verified it in 30 consecutive runs.
- This repository has no `src/templates/markdown/spec/` mirror, so there is no
  project-local template copy to synchronize.
