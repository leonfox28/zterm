# Occupied Session recovery

Observed on the Xiaomi development 1010: restoring the exact remembered Mac
main Session returns session_occupied and shows only In use / Retry / Sessions.
Read-only host listing confirms that exact ID already has a controller (140x39),
and a desktop zterm client is running. No user Session was taken over or closed.

Classification: local implementation defect. The existing host lease and explicit
takeover contract is adequate and correctly rejects a second controller. The UI
confuses AppState.sessionId (the selected/retry target, set before attach) with
an owned attachment. It marks the failed target Current and suppresses the row
confirmation, while the failed-attach page only repeats an ordinary attach.
The same predicate also mislabels retained ended/lease-lost frames as current.

Boundary: TerminalScreen derives current ownership from both target identity and
native attachment lifecycle, and offers the existing explicit takeover confirmation
for occupied/lease-lost recovery. Keep the frozen selected ID authoritative; no
implicit takeover, creation, target substitution, host/protocol or Repository
lifecycle changes. AppUi and two locale resources distinguish an occupied error
from the compact list badge. An emulator integration test owns one disposable
Session and a competing native attachment; verify cancel, row recovery, explicit
confirmation and subsequent lease-lost recovery. This is the validation owner;
the user's live main Session is only observed.

Baseline 1010 fails the new real-host regression exactly at the first Take over
button lookup: no node exists (`occupied-1010-baseline.log`). Setup uses only
one newly created disposable Session, with its exact remembered ID saved before
the lazy Repository launches. Cancel and lease-lost branches share this fixture.
Current-row dismissal also stays local while synchronizing/reconnecting; it must
not call selectSession and accidentally attempt a second ordinary attachment.

Validation on signed release 1011, emulator-5556 with the real Mac host:
- occupied-1011-ui.log: PASS, 8.765 s. Failed remembered-target attach exposes
  confirmation without a list; cancel preserves the competing controller; the
  occupied target is not labelled Current; confirmed row selection takes over
  the exact owned ID. A later competing takeover exposes the same recovery,
  with cancel/confirm verified again. Only the fixture Session is closed.
- occupied-1011-terminal-regression.log: PASS, 30.395 s. Existing full terminal
  UI regression covers IME, cached bottom, selection/copy, Activity retention,
  rotation/fonts, current row and Session management.
- Debug/release build and lint pass; Trellis validation and diff whitespace
  checks pass. Both final APK signatures and native ELF/zip 16 KiB alignment
  verify. No Rust/protocol/host source changes were required for this fix.

Artifacts and SHA-256: target/android-apk/1011/{build.json,SHA256SUMS}.
