# Android implementation progress

## Activation and first boundary — 2026-09-07

The user activated `android-app` with `task.py start --allow-empty-context`.
The task is now `in_progress` on `holy-zebra`, based on mainline `de25a38`.
The shell's absent session identity warning did not prevent task activation.
All earlier planning-only checkpoints remain historical, not current blockers.
The existing approved product scope and user-confirmed host baseline stand.
Implementation/checking is inline without any sub-agents.

Phase 1's behavior gap is the absence of a buildable Android App and native
Kotlin/Rust boundary. Ownership/files are `apps/android` for Gradle/App/JNI
packaging, `crates/android` for the application runtime and typed FFI,
`tools/uniffi-bindgen` for the exact matching generator, and `tools/android`
plus `Justfile` for repeatable builds. Root workspace/lock/ignore edits register
these outputs. No host Session/parser/PTY implementation moves in this phase.
Emulator bridge checks precede the shared-client extraction, whose existing
desktop tests must continue to prove behavior preservation.

Installed NDK `28.2.13676358` alongside the existing r30; cargo-ndk `4.1.2`
and Rust Android target were already present. Official Gradle 9.1.0 wrapper
and distribution checksums were verified. Native host build succeeded with
UniFFI 0.32.0; Android build/runtime evidence follows below.

UniFFI 0.32 requires global `[crates.<crate>.bindings.kotlin]` configuration
when supplying `--config`, unlike older flat samples. Its Kotlin objects own
`AutoCloseable.close`; the explicit runtime shutdown API uses `shutdown` to
avoid collision. Foreign future disposal aborts only its spawned waiter;
explicit runtime disposal cancels work and shuts down the executor without
blocking from inside an entered runtime.

Primary integration sources rechecked during implementation:
- https://developer.android.com/build/releases/agp-9-0-0-release-notes
- https://developer.android.com/build/extend-agp
- https://mozilla.github.io/uniffi-rs/latest/futures.html
- Exact UniFFI 0.32.0 source in Cargo's registry (`global_config.rs`, Kotlin templates).

## Phase 1 gate and Phase 2 boundary

The actual arm64 debug APK and instrumentation APK built successfully. All four
native bridge tests passed on Pixel_9/API 36 (`emulator-5554`): typed load/state,
explicit cancellation, coroutine disposal isolation, and idempotent shutdown.
Android lint passed; there are no JVM unit tests yet. All six bundled native
libraries passed ELF 16 KB LOAD alignment and APK zip alignment. The running AVD
has 4 KB pages, so this is not a claim of execution on a 16 KB-page device.
The scaffold was installed and launched. It is not yet a functional delivery.

Phase 2 moves the semantic surface and resume input bound, then bounded errors,
pair framing/controller authentication, and attachment/unary protocol ownership
into `zterm-client`. Daemon facade imports preserve existing desktop callers;
Unix transport and host persistence remain in their existing adapters. Existing
owner tests move with the implementation or exercise the facade. CLI extraction
regression passed: 83 tests, zero failures, three ignored subprocess helpers.
The new crate must remain free of host PTY/parser/presenter dependencies.

## Shared network and durable identity gate — 2026-09-07

- Moved the canonical attachment/reconnect, Session unary lease/replay, pairing
  controller exchange and normal confirmation, framing, handshake, route merging,
  and semantic surface into `zterm-client`; desktop facades call that owner.
- Shared unit tests compile the exact desktop Unix adapter source under cfg(test)
  rather than depending back on the daemon (which creates two client crate
  identities in Rust unit-test builds). Production client/Android dependencies
  contain no daemon/platform/PTY/presenter/portmapper.
- Android outbound Iroh adapter reuses one Endpoint and per-peer normal connection;
  pairing uses a provisional peer, committed only after the host record is saved.
  No Android inbound service ALPN or host Session owner was added.
- Android DNS uses active LinkProperties servers through Iroh's explicit resolver
  port, avoiding raw JNI globals/unsafe initialization. Iroh's default embedded
  WebPKI trust remains enabled. Reviewed pinned iroh-dns 1.0.3 android.rs and
  iroh-relay 1.0.3 tls.rs, and the Endpoint API reference.
- Keystore AES-GCM wraps the durable no-backup seed before network initialization.
  Identity corruption or lost wrapping key yields an explicit error without
  regeneration. Address book, preferences and exact recent target use AtomicFile.
- Emulator suite: eight tests passed; real-host fixture test skipped when absent.
  Then an explicitly installed one-time local macOS host ticket exercised the
  production pairing adapter, normal authorization, durable save/commit and Session
  listing. Device runner recorded 1 test, 0 failures (01:43:18–01:43:22). ADB
  disconnected before host-side result delivery; the device runner result and
  persisted identity/state were checked. A subsequent fixture-free run skipped
  as expected. No bearer value is retained in evidence.
- Earlier desktop regression after extraction: daemon lib 167 passed/1 ignored,
  plus controller lease 4, local pairing 2, local Session 7, pair manager 9, pair
  protocol 2, pair secrets 3, terminal recovery 1; connection-broker network gate
  remains the existing ignored integration target. Shared owner suite passed 62
  before moving two private route candidate tests. Clippy passed.

Remaining: full native UI/terminal, multi-page history and touch selection, QR
CLI output and camera/gallery integration, signed delivery and complete runtime
acceptance. These foundation results do not establish feature completion.

## Shared ownership gate and first live UI build

- Phase 2 passes: shared client 67 tests; CLI 83 passed/3 existing ignored
  plus its integration targets; strict client/CLI/Android Clippy and the extended
  host/Android dependency guard pass. The guard checks normal/build edges for
  aarch64 macOS and Android and preserves the exact official Alacritty/vte path.
- Mode-aware legacy encoding now has one owner. A modified cursor-key bug found
  during extraction emitted the decimal byte number rather than its final letter;
  corrected with a Ctrl+Up regression. Native key reports also honor synchronized
  Kitty progressive flags (official keyboard-protocol reference reviewed).
- Implemented the Application repository, live host cards, minimal credential
  dialog, OS photo picker, CameraX QR recognition, language/theme/font settings,
  title-anchored Session list, row Rename/Delete, explicit create/takeover forms,
  measured Canvas semantic rendering and native IME boundary. All compile and
  Android lint has no errors. Version/ABI/scaffold warnings remain, including
  application icon (to be supplied before delivery).
- Added the Rust complete-frame owner and bounded ordered input port over the
  shared Session driver. Initial/sync/resume ACKs follow installed native state;
  Activity observation is independent of endpoint/attachment lifetime.
- Runtime acceptance for these new UI/input features is in progress. Scrolling,
  multi-page selection, full input edge cases, QR host output, and signed delivery
  are still unfinished; do not treat this build as the final App.

## Live terminal diagnosis and QR presentation boundary

Linux pairing passed on the emulator (one test, 3.122 seconds). Full terminal
acceptance still fails: prior Linux run timed out, and a new cold-boot run listed
Sessions on a direct path (1 ms RTT) then returned create outcome unknown.
Root cause is undetermined; relay trouble alone does not explain these results.
Reuse only a task-created detached Session to isolate attachment/input from
creation. Add content-free stage/counter observations to that integration test;
never retry a create under a new logical identity automatically.

Host QR work is confined to CLI output: explicit mutually exclusive flags, local
canonical-ticket QR encoding, private atomic no-clobber PNG output, terminal
width/TTY admission. Pair/auth owners and ticket representation stay canonical.
The default ticket stdout remains unchanged, including PNG mode; QR presentation
failure keeps the same ticket available for manual entry. Meaningful checks own
no-overwrite, PNG raster correctness and Android decoder round trip.

### Android UDP batch failure isolated

Classification: platform adapter contract violation. The same direct Linux
connection listed Sessions at 1 ms RTT, but larger create/attach traffic stalled
and lost-packet count rose from 1 to 15. With only Iroh's
`enable_segmentation_offload(false)` changed in Android endpoint construction,
the exact previously failing detached Session attached, accepted Unicode input,
resized, renamed and detached in 2.523 seconds. Full fresh-create through cleanup
then passed on Linux in 3.318 seconds and macOS in 3.643 seconds. Both tests
verify exact Session identity, Unicode/combining output, measured dimensions,
rename, detach preserving host work, and explicit final cleanup.

Keep ordinary UDP sends in the Android adapter; do not change deadlines,
mutation replay, host transport, or device identity. The evidence isolates GSO
batch delivery in this Android emulator path, not a claim that every physical
Android driver fails. Runtime acceptance on Xiaomi remains separate. Official
Iroh 1.0.3 `QuicTransportConfigBuilder::enable_segmentation_offload` documents
that driver incompatibility may cause packet loss before automatic fallback.
No MTU workaround or new transport protocol was added.

### Multi-page cache implementation boundary

Extend the existing core cache with an opt-in bounded multi-page policy; preserve
single-window defaults and desktop tests. Keep its sole pending query and anchor
validation. Retain immutable windows with accounted allocations and explicit Arc
pins, evict unpinned least-recently-used windows, and exclude stale mutable-live
rows from reuse while preserving proven historical slices across append. Android
supplies row allocation cost, prefetch horizon and selection pins. No second
per-page request owner or host/wire representation is introduced.

### QR and multi-page implementation evidence

- Explicit CLI QR flags, private atomic PNG generation, canonical text fallback,
  no-clobber and module/quiet-zone tests pass. Android's bundled ML Kit decoded
  a PNG generated by the actual new CLI to exactly the original ticket (one test,
  0.09 seconds); both fixture files were removed after use. Camera/OS picker UI
  routes still need actual end-to-end acceptance.
- Core ViewportCache now opts into an accounted multi-page store while preserving
  the desktop default. Twenty-one cache tests pass (13 preserved and eight new),
  covering multi-page local backtracking, append/mutable-live distinction,
  pinned rows, byte/row budgets, eviction, one pending query, warmup and epoch
  invalidation. Shared text extraction now also accepts borrowed semantic rows
  so selection can reuse pins without copying complete cells.
- The native navigation reducer and Android GestureDetector/OverScroller/floating
  Copy UI compile. Native frames carry input epoch/readiness, reading offset and
  logical selection coordinates. Application input queues bind attachment and
  native epoch; actual foreground visibility pauses only speculative reads.
- Strict core/client/CLI/Android Clippy passes for this intermediate build.
- Live extended acceptance is ongoing. The first extended run stalled at a test
  threshold: 160 printed rows minus the 24-row viewport need not retain 150 rows.
  Corrected the fixture's minimum to 120; its exact final-row assertion remains.
  Host Session cleanup succeeded. No history/selection acceptance pass is claimed
  before the rerun.

Remaining navigation review includes continuous-output hit-test identity, frozen
live-row joins across append, resource/query error recovery, selection rendering
and edge scrolling, native handle appearance, lifecycle retention and input epoch
races. Complete these before marking Phase 5 done or delivering an APK.

Extended native emulator acceptance now passes on the real Linux host in
8.372 seconds: 160 lines of output, historical-page reads, selection spanning
three 24-row screens, exact Unicode copy through the shared extractor, healthy
return preserving the first command, resize, rename, detach and final cleanup.
The test's original retained-row threshold was the failed assertion condition,
not a lost-output transport regression (actual retained count was 141).
Actual Android touch/handle/clipboard UI and disconnect/lifecycle cases remain
separate required acceptance work.

Continuous-output hit testing needs an explicit captured-frame source: an older
rendered frame cannot be selected by applying its screen coordinates to whatever
live revision the native reducer happens to hold later. Add a bounded immutable
frame-source handle backed by the existing accounted cache; retain it through
Repository/View ownership and pass it back for selection. This keeps one semantic
selection source and avoids timing retries or guessing text from a later frame.

### Actual Android UI acceptance (2026-09-07 continuation)

- Latest opaque-source integration passes Linux native acceptance in 4.288s.
  Removed hit-test timing retries and release intermediate test frame handles.
- Added `TerminalUiTest` driving the production custom View's InputConnection,
  GestureDetector, floating ActionMode, system clipboard and Activity lifecycle.
  The extended 22.91s passing run creates a private Mac Session, prints 400 exact
  Unicode rows, swipes through local pages, drags the native-style handle at the
  edge for continuous selection spanning at least three measured screens, copies
  through Android's floating Copy button, and verifies exact CJK/combining text
  and newline extent. Overlay preserves viewport dimensions, shortcuts stay
  visible, background/recreation keeps exact Session, first return input arrives.
  It closes only its own Session. Evidence: target/android-toolchain/
  edge-selection-pass.log and selection-ui.png (task fixture text only).
- Fixed actual Canvas overdraw discovered in the screenshot: explicitly clip the
  custom View canvas so history overscan cannot cover Compose chrome. Native
  theme selection drawables replace circles; nearest-handle hit testing avoids
  choosing the wrong endpoint for short selections.
- Cache prefetch replies can no longer move a frozen reading viewport; selected
  mutable rows cannot be visually replaced with later content while Copy still
  uses the captured pin. Six navigation regression tests pass.
- Input admission now passes the originating native epoch through Kotlin/UniFFI
  all the way to the actor, closing a compare/submit race across disconnection.
  Resize immediately suspends input and retains previous pixels until sync.
- Added multi-code local ticket inspection and host choice, protocol-owned input
  text limit, permanent camera-denial Settings entry, independent preference
  updates, and fences for late navigation/session results.
- First real scanner UI run found an ActivityResultRegistryOwner failure caused
  by createConfigurationContext losing its Activity owner. Replace it with a
  configuration-overridden ContextThemeWrapper retaining the Activity chain.
  Scanner UI rerun is pending; do not claim camera/gallery/manual acceptance yet.
- Added external stable local APK-signing helper and an Android hosted build/lint
  workflow. Signing execution, release locks and update acceptance remain pending.

Scanner UI now passes in 14.746s using actual new CLI host tickets: invalid manual
input remains editable, valid input pairs and durably saves the host, the Android
system photo picker selects a MediaStore fixture, bundled ML Kit recognizes it,
and normal authorization permits Session listing. Fixtures are deleted after use.
CameraX virtual-scene poster recognition did not complete within 35s; camera
acceptance is not yet claimed. Android build/lint/JVM checks now pass; remaining
lint warnings include intentionally pinned SDK/dependencies and platform coverage.
The first stable external-key release APK was generated and signature/16KB
packaging verified (code 1001), but it is intermediate. Packaging review found
cargo-ndk copying stale Iroh cdylibs from an earlier target directory. ELF NEEDED
shows the bridge only needs Android system libraries. The native build now copies
only libzterm_android.so; newer APK must replace the intermediate artifact.

### Broader emulator and quality evidence

- CameraX/bundled ML Kit now pairs the real Mac host from the virtual-scene wall
  poster in 9.544s. The SDK's documented poster coordinates and authenticated
  EmulatorController physical-model API positioned the camera; reset images and
  pose afterwards. This remains distinct from physical camera acceptance.
- All nine language/theme pairs persist and preserve connection/selection during
  the real cross-screen UI test (19.116s pass). With native evidence counters,
  the 17.287s test records 432 local cache hits, one miss, 15 bounded queries,
  2,945 retained allocated rows and 15,031,357 peak accounted bytes (<16 MiB).
- Full Session UI extension passes in 28.887s: rename exact row, create a second
  Session, cancel deletion, confirm deletion, and switch back to the original
  retained Session, in addition to the earlier IME/scroll/copy/lifecycle cases.
- A separate API36 Google APIs AVD (`Zterm_Acceptance`, emulator-5556) enables
  isolated uid-scoped network tests; original Play Store AVD remains untouched.
  UDP DROP except DNS forces the actual unmodified Android binary through Relay.
  Pairing passes and terminal acceptance passes in 38.767s with asserted `relay`
  path, observed RTT 682–838ms and zero lost packets at create. It covers Unicode,
  three-screen copy, first return input, resize, stale-epoch rejection, rename,
  detach and final cleanup. IPv4/IPv6 rules are removed in a finally block.
  REJECT was unsuitable for the reachability fixture: it returns a local send
  error rather than silently blackholing direct-path probes; no product retry or
  timeout was changed to accommodate that fixture.
- Workspace all-feature tests and rustdoc pass. The first full `just check`
  stopped at dependency-license policy for the already selected UniFFI 0.32.0
  family. Added exact-version MPL-2.0 exceptions following existing policy plus
  upstream license/source notices in APK assets; dependency checks now pass.
  Remaining relay static/check steps are running separately.
- Signed code 1002 now packages exactly six intended native libraries, all 16KB
  aligned, and has SHA-256 ebb7cfc7723c4f7d7293743960d369bffb2ba71851e60cb5b6470b1cdd6af0ec.
  Signed install/update identity acceptance is in progress; code 1001 is obsolete.

### Final IME, copy and subscription checks

- Android lint passes on the latest sources. The JVM unit-test Gradle task has
  NO-SOURCE; it is not evidence of executed JVM tests. Actual identity/bridge/UI
  verification uses device instrumentation plus Rust tests.
- Signed 1002 fresh installation generated a new controller identity after the
  task-owned debug install was uninstalled. Signed 1002 -> 1003 update preserved
  the exact public controller identity and known host. See signed-update-pass.log.
- Signed 1004 passes the expanded production UI test in 20.5s on the separate
  API36 Google APIs AVD. This includes full Gboard shown/hidden measured geometry,
  persistent shortcuts, rejection of an old IME callback after resize, canceled
  asynchronous Copy with no late effect, 3-screen handle selection and system
  clipboard, all nine appearance pairs, Activity retention and Session management.
  Peak cache allocation is 15,379,683 bytes with 431 local hits/one miss/15 queries.
  Original AVD Gboard instead showed a zero-inset physical-keyboard side strip;
  that failed assertion did not establish a terminal resize defect.
- Real desktop takeover exposed a sleeping subscriber race: the native actor
  published lease_lost then canceled; a biased cancellation select skipped that
  already published final frame. Re-read the latest generation on cancellation.
  Deterministic pending-future tests cover lease_lost, ended and closed; all seven
  Android Rust tests and strict Clippy pass. This change is in signed build 1005.
  Actual round-trip handoff, network interruption and rotation are still running.
- Build 1005 is signed by the same external durable key, with all six packaged
  native libraries and APK zip entries verified for 16 KiB alignment. SHA-256:
  9e59d011299eaa8e26950d2909c446703f66c7f6eb44a4b9d7da9b9fbce20b42.
  Earlier codes 1001–1004 are intermediate artifacts.

### Final signed-artifact acceptance (code 1005)

- `handoff-pass.log`: PASS, 4.689s. Actual desktop CLI with a controlling PTY and
  measured grid explicitly takes control from Android; Android receives lease
  loss, desktop sends shell input, Android explicitly takes back the same ID and
  reads that exact output. Test coordinator must drain the PTY while waiting for
  takeover ACK; output backpressure otherwise stalls CLI initialization. It must
  also wait for takeover before sending input and clear stale handshake files.
- `network-pass.log`: PASS, 55.902s. IPv4/IPv6 OUTPUT DROP scoped only to the signed
  app UID interrupts a real connection. Android retains pixels, enters reconnect,
  rejects attempted disconnected input, resumes the exact Session after rules
  are removed, and sends a new command without replaying the rejected text.
  Both firewall chains were verified restored. No host restart/user Session was
  needed; only the test's exact Session was closed.
- `final-signed-ui-pass.log`: PASS, 21.033s, signed 1005. All previous production
  UI cases pass plus actual landscape/portrait and 12/14/16 font changes with
  measured native grids and unchanged Session identity. Peak accounted memory
  15,380,161 bytes; 432 local hits, one miss, 15 queries, RTT 9ms. These are one
  emulator fixture's observed measurements, not broad performance guarantees.
- `alternate-native-pass.log`: PASS, 4.348s on the original paired Pixel AVD,
  using the same latest native source. Real main/alternate screen programs hide
  and move the cursor; semantic IME anchor stays at row 3 / column 6 while the
  glyph is hidden, and normal cursor visibility returns after leaving each.
- Earlier cold endpoint attempts occasionally exceeded the existing handshake
  deadline (the shared welcome reader reports unauthorized on incomplete normal
  handshakes). Mac relay logs also showed path changes. No authorization rule,
  timeout, or retry contract was weakened to turn those attempts into passes.
- Temporary bearer/QR fixtures were removed locally and from /data/local/tmp.
  Test-only leftover desktop-handoff Session was closed by its exact ID; the
  user's Linux main Session was never touched.

Physical Xiaomi 17 Pro Max acceptance remains pending: actual OS/page size,
physical camera, Xiaomi IME/touch/background/network behavior. Additional broad
fault-injection/stress permutations remain explicitly unchecked in implement.md;
adjacent passing cases do not claim those results. The task remains in_progress.

Final signed 1005 bridge/identity suite passes 8 tests (0.069s), including the
expected public controller identity and one known host after updates. Final lint
and dependency-policy checks pass. Trellis references were populated without
sub-agent dispatch; task validation passes. Summary: apk-1005-acceptance.md.

### Application ID separation and controller geometry (1006)

User follow-up requests separate development/release packages and asks about
architecture and handoff sizing. Release retains `io.github.leonfox28.zterm`;
debug now adds `.dev` and uses `zterm Dev` in both resource languages. Kotlin
namespace/UniFFI names and existing release signing identity remain unchanged.

Code review found a missed initial geometry contract: the attach viewport is a
host creation hint, so a retained Session still reports the previous controller's
size. NativeRuntime now submits the requested size through the existing actor
before returning its handle; the actor defers resize until takeover/Active and
resynchronizes before admitting input. No host or wire behavior changed.

- Signed release 1006 + development 1006 install concurrently on emulator-5556;
  PackageManager reports distinct UIDs. Release retains its expected controller
  identity and one host; development has a distinct identity and zero hosts.
- Development bridge/identity instrumentation: 8 tests pass (0.114s). Release
  identity-retention assertion also passes. Manifest badging verifies both IDs,
  labels in default/Chinese resources, and the shared resolved MainActivity.
- Expanded `DesktopHandoffTest`: PASS 4.079s. Host Session starts at 80 columns;
  native attach applies the phone's 32x52 grid. Real desktop CLI takes over and
  applies 79 content columns (one of its physical 80 columns is its scrollbar),
  writes a marker, and Android takes back the same ID at 32x52 with marker intact.
  The earlier 1005 handoff fixture used equal requested sizes and missed this gap.
- Full production UI suite on signed 1006: PASS 26.82s, including IME, selection,
  copy cancellation, preferences, rotation/font geometry and Session management.
- All 7 Android Rust tests and strict Clippy pass; debug/release builds and Android
  lint pass. Both APK signatures and all native ELF/zip 16 KiB alignments verify.

Artifacts under target/android-apk/1006, with both hashes in SHA256SUMS:
- zterm-android-arm64-1006.apk (release):
  24e30d0193305f2a685b6c332a4144c09c1aa66496e5b0747a78090f967dfa92
- zterm-dev-arm64-1006.apk (development):
  27cae371bd0d195c51953997bdc5dd92eb809a85346ebd0a9ed6aa80b845b57c

1006 supersedes 1005 for installation. Old unsuffixed debug installs are retained;
no uninstall/data migration was performed. Task stays inline and in_progress,
with existing physical-phone and broader acceptance boundaries unchanged.

### Xiaomi physical installation

After the user enabled USB installation, both 1006 APKs installed successfully
on the connected Xiaomi phone via adb --no-incremental. PackageManager confirms
release io.github.leonfox28.zterm and development io.github.leonfox28.zterm.dev
coexist with distinct UIDs, versionCode 1006 and versionName 0.1.24. Observed
device properties: model 2509FPN0BC, Android release 17, page size 4096 bytes.
This proves physical installation only; camera, input, connection and background
phone acceptance remain pending. No existing app was uninstalled.


### Keyboard, bottom-scroll and Herdr touch follow-up (1010)

Implemented the accepted once-after-IME geometry policy, immediate local return
to the live bottom, and mode-aware child taps/wheels with local long-press copy.
The keyboard regression used 400 lines and recording load; it exposed and
rejected intermediate candidates before final pre-draw commit. A rapid-tap
regression also caught the default double-tap listener swallowing a click.

Signed 1010 passes all 3 production UI tests (63.074 s), including real isolated
Herdr pane scrolling/tab selection and raw pointer encoding/ownership. Release
native/identity suite passes 9 tests; development passes 8. Both packages install
concurrently and preserve their independent identities. Rust regression,
desktop routing, strict Clippy, lint, signatures and 16 KiB checks pass.

Evidence, hashes and explicit limits: research/apk-1010-acceptance.md. Root causes
and failed candidates: research/keyboard-scroll-smoothness.md. APKs are under
target/android-apk/1010. The unplugged Xiaomi remains on 1006; no claim of phone
update or physical acceptance is made. Task remains inline and in_progress.


### Xiaomi development update to 1010

At the user's request, adb installed the development APK 1010 over 1006 on
the connected Xiaomi 17 Pro Max using install --no-incremental -r. Installation
returned Success; PackageManager confirms io.github.leonfox28.zterm.dev at
versionCode 1010, versionName 0.1.24, with its firstInstallTime unchanged.
No uninstall or data clear was performed. Physical touch/IME behavior still
requires user acceptance; this update establishes installation only.


### Occupied recovery and Xiaomi development 1011

The reported In use error is an authoritative occupied response for the user's
remembered Mac main Session. Read-only listing shows it occupied at 140x39 and
unchanged after this work. Fixed the UI's target-versus-current predicate and
added explicit confirmation to occupied/lease-lost recovery. No implicit takeover.
The exact cause, scope and emulator evidence are in research/occupied-recovery.md.

Signed 1011 occupied recovery regression passes (8.765 s) and existing full
terminal UI regression passes (30.395 s); debug/release lint/build and APK checks
pass. Xiaomi development updated in place to 1011 via adb, Success; PackageManager
confirms versionCode 1011 and unchanged firstInstallTime. No uninstall/data clear
was performed. The user's live main Session remains occupied at 140x39; takeovers
were tested only on the disposable fixture. Physical interaction acceptance and
the broader original task remain pending. No sub-agents, commit or archive.

### Initial attach geometry and Xiaomi development 1012

The user reports a visible Herdr frame followed by Connection failed after
takeover, then a successful retry at 80x24 that becomes correct after opening the
keyboard. These are tracked separately: the original transport closure has not
reproduced or been attributed to a root cause; the initial size race is proven.
See research/takeover-connection-exit.md for evidence and explicit boundaries.

AppRepository now records the latest desired grid synchronously and wakes its
single resize consumer after asynchronous attach if measurements changed while
the handle was unavailable. Retired attachment resize errors cannot affect the
new attachment. Content-free DEBUG-only state/error diagnostics support the
unresolved transport report without logging tickets, addresses or terminal text.

AttachGeometryTest reproduces the final-grid failure on 1011 and passes on signed
1012 (5.221 s), checking the actual host size. The terminal UI regression passes
(27.419 s); debug/release builds, lint, APK signatures and native ELF/zip 16 KiB
checks pass. Expanded Herdr takeover tests fail twice before creating their test
Session with deadline_exceeded, so this extension has no new acceptance result.

Both 1012 artifacts are under target/android-apk/1012 with hashes in SHA256SUMS.
Xiaomi development updated in place to the final 1012 at 14:45:01; adb returns
Success and PackageManager confirms the version with firstInstallTime unchanged.
This supersedes the earlier diagnostic 1012 on the phone. Release on the phone
remains 1006. Physical cold-attach acceptance and the broader task remain pending;
no sub-agents, commit, archive, uninstall or data clear.


### Herdr desktop-first takeover: host wide-cell clipping

The original immediate post-takeover error was reproduced on emulator-5554
against the existing Mac main. Protocol validation reported InvalidWidePair
at the first narrowed snapshot. Host Alacritty 0.26 alternate-screen clipping
left a wide head without its spacer; the semantic projection now emits a
style-preserving blank for that fragment. Whole terminal tests, strict Clippy,
attachment resync and real Herdr 0.8.2 black-box checks pass. The corrected host
binary is built but the running user daemon has not been restarted.

The attempted separate network acceptance host caused a macOS firewall prompt
which the user rejected. That helper was stopped and removed; emulator state
and final development APK 1012 were restored. Corrected-host end-to-end takeover
remains pending host restart confirmation, not accepted from adjacent tests.
No physical phone operations. Details and evidence in
`research/takeover-connection-exit.md`.


### Native patch release 0.1.25

User authorized the native new-version release flow. The projection fix,
regression and spec were released separately from Android WIP through PR #32.
Full local `just check`, PR CI, exact-main CI/candidates, protected signing and
all three final installer checks passed. Immutable latest stable Release:
https://github.com/leonfox28/zterm/releases/tag/v0.1.25 (2026-09-07T08:02:24Z).
Mac local activation/restart and corrected-host Android takeover are still
pending. Evidence: `research/release-0.1.25.md`.

### Post-update emulator acceptance

User activated installed native 0.1.25. Emulator-5554 retains the final signed
development 1012. Exact wide-cell clipping takeover passes through both native
and actual App UI paths; installed Herdr 0.8.2 native takeover also passes.
The user's real Herdr/Pi main was successfully taken over through the App at
16:18:19.958: Mac 140x39 becomes 56x53 immediately, remains attached for over
ten minutes without a new error, and touch scrolling responds. No commands were
sent to the user's child agent. Emulator left connected; no physical phone
operations, APK changes, or rejected network helper restarts.

Two automated Herdr UI fixture starts separately fail with `unauthorized`
before creating a test Session. Their cause is unclassified; this error can also
represent an incomplete handshake. Standard docked IME resize was not retested
successfully because Gboard used floating/handwriting mode; it is left hidden.
These remaining checks keep the broader Android task in progress. Detailed
evidence and acceptance boundaries: `research/takeover-connection-exit.md`.

### Explicit keyboard button and 8–16 font slider: development 1013

User confirmed a keyboard show/hide button at the far right of the bottom row;
content taps now keep local/child meaning without opening IME. Kotlin retains
all eight terminal shortcuts and observes actual IME visibility for both the
button and Back, including floating mode. User additionally requested a
horizontal integer font slider from 8 through 16, step 1; Settings previews
while dragging, persists on finish, and AppStore accepts all nine sizes on reload.

Old 1012 fails the exact no-IME-on-child-tap regression. New 1013 passes all three
real-host terminal UI cases (including Herdr, full IME resize and smaller/larger
fonts) and the separate corrected Settings slider/persistence/recreation test.
Debug/Release lint/build and APK signature/16 KiB checks pass. Development 1013
is installed on emulator-5554; its identity and saved state are retained exactly.
No physical phone access. Full evidence, earlier test-coordinate failures and
artifact hashes: `research/explicit-keyboard-entry.md`. Broader task in progress;
no commit/archive or additional native publication.

### Keyboard shortcut visual consistency: development 1014

The rightmost keyboard control now shares TextButton styling, primary/disabled
colors, zero padding and equal row sizing with adjacent shortcuts; its glyph
is 18 dp instead of 24 dp. Debug build, Debug/Release lint, signed Release build
and both APK signature/16 KiB checks pass. Emulator-5554 updated to 1014 without
identity/state changes. Cropped before/after visual checks and manual keyboard
show/hide pass; main remains connected and IME hidden for manual testing. No
physical phone access or child commands. Details: explicit-keyboard-entry.md.

### User-requested phone development update to 1014

The user explicitly requested updating the phone development package. ADB
identified physical device 1680fce0 (2509FPN0BC). The finalized development APK
SHA-256 matches `c5ff6276392b55d5ea4a552b75cba970e3e070da7270a8c4c3801a34500709f4`.
`adb -s 1680fce0 install --no-incremental -r` returned Success. PackageManager
confirms versionCode 1012 -> 1014, lastUpdateTime 2026-09-07 17:08:02, and unchanged
firstInstallTime 2026-09-07 11:51:23. No uninstall, data clear, release-package
installation, app launch or automated phone interaction was performed. This
explicit authorization covers the requested installation; automated acceptance
remains emulator-only. The native release and emulator were not changed.

### Selection touch recovery and local projection optimization: development 1015

Kept the existing desktop IPC/daemon and Android direct-Iroh architecture.
Android selection exit omitted the existing local Live transition, leaving
Herdr frozen with pointer mode None. Copy/cancel at the bottom now shares the
scroll-to-zero transition, preserving older reading positions, pending queries
and recovery barriers. Pinned presentation now converts only displayed rows,
removing the unused live-grid conversion. Old 1014 fails the exact cancel→touch
regression; final 1015 passes all 3 terminal UI cases (63.799s), including real
Herdr, plus all 12 native tests, Clippy, dependency policy, lint/JVM checks and
both APK signature/16 KiB checks. Emulator-5554 updated with saved identity/state
unchanged; dedicated emulator-5556 stopped. No phone operation, desktop changes,
native publication, commit or archive in this increment. Evidence and hashes:
`research/selection-touch-recovery.md`.

### Architecture cleanup and precise handshake errors: development 1016

Completed the earlier requested shared-client cleanup while keeping the topology.
Desktop daemon restart ownership moved from shared Session state to the Unix
connector; desktop/mobile now share the entire outbound Hello/Welcome sequence.
Host/controller dependency separation and common route ordering were already
present and were retained/documented. Welcome timeout/reset no longer becomes
Unauthorized; only explicit peer 0x100 does. Emulator capture additionally proved
0x102 duplicate rejection after explicit runtime shutdown failed to close the
old connection before stopping Tokio. Async explicit shutdown now awaits Iroh
closure; background/Activity behavior is unchanged. Four immediate same-identity
restarts and all 8 final emulator tests pass (36.474s), as do workspace/native,
Clippy, dependency, lint/JVM and APK checks. Emulator-5554 updated to verified
1016 with identity/state retained; test emulator stopped. Desktop changes remain
unpublished working-tree code. Historical overwritten errors and abrupt OS-kill
restarts are not claimed resolved. Full evidence: `research/client-architecture-cleanup.md`.
