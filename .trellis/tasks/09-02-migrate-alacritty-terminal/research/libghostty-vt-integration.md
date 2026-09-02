# Research: libghostty-vt integration architecture

- Query: What is the safest, most reproducible architecture for replacing Zterm's Rust `vt100` engine with upstream `libghostty-vt`, especially across desktop and mobile targets?
- Scope: mixed (Zterm source/specs, upstream Ghostty, community Rust bindings, build and release artifacts)
- Date: 2026-09-02

## Findings

### Executive decision

The C boundary is the right boundary, but “Zterm directly calls C from `core`” is not the right architecture.

The recommended stack is:

```text
Zterm-owned terminal contract and wire types
    -> Zterm safe adapter + single-owner terminal actor
        -> audited existing safe `libghostty-vt` Rust wrapper (`!Send + !Sync`)
            -> checked-in generated `libghostty-vt-sys` bindings
                -> exact, statically linked libghostty-vt C ABI build
```

Concretely:

1. Reuse the two-crate shape and safe API already implemented by the community
   [`libghostty-rs`](https://github.com/Uzaaft/libghostty-rs) project; do not rewrite its FFI surface inside Zterm.
2. Put a second, fully safe Zterm adapter above it. The adapter retains Zterm's revision, history cursor/epoch, resource, snapshot/delta, callback-policy, and wire contracts. No Ghostty type or pointer crosses that boundary.
3. Construct, use, and destroy the Ghostty terminal on one dedicated owner thread. Commands and replies carry only Zterm-owned `Send` values. Do **not** add an unsafe `Send`/`Sync` implementation to preserve the current `Arc<Mutex<TerminalModel>>` shape. A separate bounded PTY writer actor must own potentially blocking writes.
4. Initially link an exact static `ReleaseSafe`, baseline-CPU, no-SIMD build. Pin the wrapper source, its embedded Ghostty source, Zig, Zig package inputs, build flags, generated header/bindings, and target tuple as one reviewed artifact identity.
5. Do not use the published crates.io `0.2.1` as-is. It predates reachable soundness fixes and lacks the current repository's mobile/build work, even though the repository still declares the same `0.2.1` version. The inspected Git HEAD is also probe-only until issue #70 is fixed in an exact successor revision.
6. Keep `portable-pty` for now. `libghostty-vt` replaces VT parsing/state/input encoding; it does not create processes or own a PTY.

This is a gated migration rather than a drop-in dependency swap. Ghostty can remove a substantial amount of parser, reflow, scrollback, selection-gesture, and mouse-encoding implementation work, but it does not replace Zterm's UI event routing or daemon/wire semantics.

### 2026-09-02 plan re-audit addendum

The later source-level plan audit in
[`plan-reaudit-2026-09-02.md`](./plan-reaudit-2026-09-02.md) supersedes three provisional assumptions
in the original research:

1. `5988a0b...` is a successful build/API probe, not the final cutover pin. Open
   [`libghostty-rs` issue #70](https://github.com/Uzaaft/libghostty-rs/issues/70) documents a safe
   `graphemes_buf` API which does not pass slice capacity to C and can write out of bounds. Gate A
   requires an exact upstream successor containing the fix.
2. Ghostty's exact generic formatter can emit OSC 4 palette, screen and other modes, OSC 7 PWD,
   and other state outside Zterm's current outer-terminal/security contract; cell OSC 8 hyperlinks
   are not faithfully reproduced by its VT page path. Zterm must project Ghostty state into a
   canonical owned surface and use a small allowlisted ANSI encoder for wire output.
3. Callback replies cannot be synchronously written by the terminal owner because current PTY
   `write_all + flush` may block. A separate bounded ordered PTY writer actor is required.

The original API/platform inventory remains evidence; the addendum and current PRD/design own the
final migration decision.

### Files found

#### Zterm

- `Cargo.toml:1-57` — workspace toolchain/dependencies; `vt100 = "=0.16.2"`, `portable-pty = "=0.9.0"`, and workspace `unsafe_code = "forbid"`.
- `deny.toml:12-54` — license allowlist, crates.io-only registry policy, and no allowed Git dependencies.
- `crates/core/src/terminal.rs:530-579` — `TerminalCheckpoint` currently owns a cloned `vt100::Screen`.
- `crates/core/src/terminal.rs:632-868` — `TerminalModel` owns `vt100::Parser` and implements ingest, resize, snapshot, delta, and semantic state.
- `crates/core/src/terminal.rs:876-1177` — history paging and resource projection; the latter explicitly uses `size_of::<vt100::Cell>()`.
- `crates/core/src/terminal.rs:1305-1359` — retained/formatted history currently clones or moves `vt100` scrollback.
- `crates/daemon/src/terminal_driver.rs:105-115` — daemon keeps a model thread plus shared terminal state.
- `crates/daemon/src/terminal_driver.rs:346-354` — resize locks and mutates the shared model from the calling thread.
- `crates/daemon/src/terminal_driver.rs:525-642` — attachment/query paths and ingest thread all access `Mutex<TerminalModel>` directly.
- `crates/cli/src/terminal_ui.rs:683-739` — UI decides whether pointer/wheel input addresses history or the live child terminal.
- `crates/cli/src/terminal_ui.rs:2484-2565` — UI currently performs X10/UTF-8/SGR mouse encoding.
- `crates/core/tests/terminal_corpus.rs` — parser chunk-invariance, query, unsafe-effect, and resize corpus.
- `crates/core/tests/terminal_snapshot_delta.rs` — snapshot/delta/history/resource/revision contract coverage.
- `.trellis/tasks/09-02-migrate-libghostty-vt/research/local-integration-spike.md` — macOS arm64 proof that the current Git wrapper can build and exercise basic state, callbacks, scrollback, formatter, and selection APIs.

#### Related specs

- `.trellis/spec/backend/terminal-model.md:11-13` — `vt100` is intentionally private and replaceable only after acceptance gates.
- `.trellis/spec/backend/terminal-model.md:17-46` — public terminal boundary must remain Zterm-owned.
- `.trellis/spec/backend/terminal-model.md:54-69` — revision, history, and mode contracts.
- `.trellis/spec/backend/terminal-model.md:88-118` — snapshot, delta, checkpoint, resource, query, and unsafe-effect contracts.
- `.trellis/spec/backend/terminal-driver.md:43-51` — one ordered terminal-model mutation path is required.
- `.trellis/spec/backend/terminal-driver.md:60-64` — attachment history access is read-only.
- `.trellis/spec/backend/terminal-driver.md:83-86` — PTY/model resize ordering is observable and must be preserved.

#### Upstream and binding sources

- Ghostty public umbrella header [`include/ghostty/vt.h`](https://github.com/ghostty-org/ghostty/blob/20abdb50a6216c450d6d4d010c41c7edf5ab15b2/include/ghostty/vt.h) — explicit API instability notice and public module list.
- Ghostty [`types.h`](https://github.com/ghostty-org/ghostty/blob/20abdb50a6216c450d6d4d010c41c7edf5ab15b2/include/ghostty/vt/types.h) — result codes, opaque handles, borrowed strings/buffers, sized structs, and ABI type manifest.
- Ghostty [`allocator.h`](https://github.com/ghostty-org/ghostty/blob/20abdb50a6216c450d6d4d010c41c7edf5ab15b2/include/ghostty/vt/allocator.h) — allocator lifetime and matching-free contract.
- Ghostty [`terminal.h`](https://github.com/ghostty-org/ghostty/blob/20abdb50a6216c450d6d4d010c41c7edf5ab15b2/include/ghostty/vt/terminal.h) — terminal ownership, effects, scrollback, mutation, and query APIs.
- Ghostty [`render.h`](https://github.com/ghostty-org/ghostty/blob/20abdb50a6216c450d6d4d010c41c7edf5ab15b2/include/ghostty/vt/render.h) — coherent copied render state and two-phase externally synchronized update.
- Ghostty [`formatter.h`](https://github.com/ghostty-org/ghostty/blob/20abdb50a6216c450d6d4d010c41c7edf5ab15b2/include/ghostty/vt/formatter.h) — borrowed terminal, synchronous writer, buffer ownership, and VT/plain/HTML formatting.
- Ghostty [`snapshot.h`](https://github.com/ghostty-org/ghostty/blob/20abdb50a6216c450d6d4d010c41c7edf5ab15b2/include/ghostty/vt/snapshot.h) — incremental internal-state snapshot/decoder and its unstable format warning.
- Ghostty [`selection.h`](https://github.com/ghostty-org/ghostty/blob/20abdb50a6216c450d6d4d010c41c7edf5ab15b2/include/ghostty/vt/selection.h) — borrowed/tracked selection endpoints and gesture state machine.
- Ghostty [`mouse.h`](https://github.com/ghostty-org/ghostty/blob/20abdb50a6216c450d6d4d010c41c7edf5ab15b2/include/ghostty/vt/mouse.h) — X10, UTF-8, SGR, URxvt, and SGR-pixels encoders.
- Ghostty [`build.zig`](https://github.com/ghostty-org/ghostty/blob/22d13172cde98a0a4dda05d3d6a3fcb0dd8ed018/build.zig) and [`build.zig.zon`](https://github.com/ghostty-org/ghostty/blob/22d13172cde98a0a4dda05d3d6a3fcb0dd8ed018/build.zig.zon) — library version, Zig requirement, source dependencies, static/shared/XCFramework outputs.
- Ghostty [`CMakeLists.txt`](https://github.com/ghostty-org/ghostty/blob/20abdb50a6216c450d6d4d010c41c7edf5ab15b2/CMakeLists.txt) and [CMake integration guide](https://github.com/ghostty-org/ghostty/blob/20abdb50a6216c450d6d4d010c41c7edf5ab15b2/dist/cmake/README.md) — official C/C++ consumption route.
- Community wrapper [`lib.rs`](https://github.com/Uzaaft/libghostty-rs/blob/5988a0b78b4aa804d1c12e66bbfe662bd97d81c0/crates/libghostty-vt/src/lib.rs#L49-L71) — explicit whole-library `!Send + !Sync` policy and channel/owner-thread recommendation.
- Community wrapper [`build.rs`](https://github.com/Uzaaft/libghostty-rs/blob/5988a0b78b4aa804d1c12e66bbfe662bd97d81c0/crates/libghostty-vt-sys/build.rs) — exact embedded Ghostty revision, build-time fetch behavior, target mapping, static/dynamic linking, and current iOS restrictions.

### 1. Public/released status and API maturity

There are three separate maturity statements that must not be conflated:

1. **The terminal implementation is mature.** Ghostty's README says the library is available and usable from C and Zig, is compatible with macOS, Linux, Windows, and Wasm, and that functionality is “extremely stable.” See the immutable [`README.md` section](https://github.com/ghostty-org/ghostty/blob/20abdb50a6216c450d6d4d010c41c7edf5ab15b2/README.md#L145-L170).
2. **The public ABI/API is explicitly not stable.** The same README says function signatures remain in flux and there is no independent `libghostty` version tag. The public [`vt.h` warning](https://github.com/ghostty-org/ghostty/blob/20abdb50a6216c450d6d4d010c41c7edf5ab15b2/include/ghostty/vt.h#L1-L27) calls the API incomplete/WIP and says breaking changes should be expected.
3. **There is a public rolling artifact, not a stable library release train.** The GitHub [`tip` release](https://github.com/ghostty-org/ghostty/releases/tag/tip) publishes a `libghostty-vt` source archive and Apple XCFramework, but `tip` is mutable. Ghostty application tags (for example the observed application `v1.3.x` series) are not a separate promise that the library ABI is stable. At the observed upstream revision, `build.zig` still reports the library as `0.1.0-dev`.

As observed on 2026-09-02:

- Ghostty `main`: `20abdb50a6216c450d6d4d010c41c7edf5ab15b2`.
- Community `libghostty-rs` repository HEAD: `5988a0b78b4aa804d1c12e66bbfe662bd97d81c0`.
- That wrapper embeds Ghostty commit `22d13172cde98a0a4dda05d3d6a3fcb0dd8ed018` and still declares Rust crate version `0.2.1`.
- The embedded Ghostty source declares application version `1.3.2-dev`, library version `0.1.0-dev`, and minimum Zig `0.16.0`.

Therefore `libghostty-vt` is a credible engine candidate, but it must be treated like a source-pinned pre-1.0 native component. A Cargo semver range, `main`, `tip`, or an application release tag is not an adequate compatibility pin.

Also note the naming distinction in Ghostty's own build: `libghostty-vt` is the public headless terminal-emulation C library. The target historically named `libghostty` is macOS GUI glue/internal integration and is explicitly described in `build.zig` as **not** the same library. Zterm should consume only `libghostty-vt`.

### 2. C ABI: ownership, errors, lifetimes, callbacks, and threading

#### Core terminal ABI

The migration-critical terminal entry points at the inspected upstream ABI are:

```c
GhosttyResult ghostty_terminal_new(
    const GhosttyAllocator* allocator,
    GhosttyTerminal* out,
    uint16_t cols,
    uint16_t rows);

void ghostty_terminal_free(GhosttyTerminal terminal);

GhosttyResult ghostty_terminal_resize(
    GhosttyTerminal terminal,
    uint16_t cols,
    uint16_t rows,
    uint32_t cell_width_px,
    uint32_t cell_height_px);

GhosttyResult ghostty_terminal_set(
    GhosttyTerminal terminal,
    GhosttyTerminalOption option,
    const void* value);

void ghostty_terminal_vt_write(
    GhosttyTerminal terminal,
    const uint8_t* bytes,
    size_t len);

GhosttyResult ghostty_terminal_get(
    GhosttyTerminal terminal,
    GhosttyTerminalData data,
    void* out);
```

- A successful `new` gives the caller an opaque terminal handle. The caller owns it and must call `ghostty_terminal_free`; freeing `NULL` is allowed.
- `set` and `get` are deliberately type-erased: the option/data discriminator determines the concrete pointee type. This is a strong reason to keep callers behind typed Rust methods rather than expose raw functions.
- `vt_write` returns `void` and is documented as never failing. Malformed or unsupported untrusted bytes are handled best-effort and may be logged. `GHOSTTY_TERMINAL_DATA_VT_PROCESSING_ERROR` is a sticky informational bit; it is not a per-write `Result`, cannot be cleared, and does not necessarily cover every unsupported sequence, ignored effect, or configured limit.
- A same-size Ghostty resize is a no-op. Zterm's own spec currently advances revision for every successful resize, including same-size requests, so the adapter—not Ghostty—must preserve that observable rule.

#### Results and ABI layout

`GhosttyResult` is a C-`int` enum with these current values:

| Code | Value | Meaning |
| --- | ---: | --- |
| `GHOSTTY_SUCCESS` | 0 | success |
| `GHOSTTY_OUT_OF_MEMORY` | -1 | allocation failed |
| `GHOSTTY_INVALID_VALUE` | -2 | invalid argument/value |
| `GHOSTTY_OUT_OF_SPACE` | -3 | caller buffer is too small |
| `GHOSTTY_NO_VALUE` | -4 | optional value is absent |
| `GHOSTTY_IO_ERROR` | -5 | external reader/writer failed |
| `GHOSTTY_LIMIT_EXCEEDED` | -6 | configured input/output bound exceeded |
| `GHOSTTY_REJECTED` | -7 | safety check rejected the operation |

All public C enums are intended to be backed by C `int`. The headers use fixed-enum syntax when supported and otherwise an `INT_MAX` sentinel. Rust code must not independently guess enum representation or accept arbitrary integers as a Rust enum; generated constants/newtypes plus checked conversion are safer.

ABI-extensible structs put `size_t size` first and must be zero-initialized with that field set (`GHOSTTY_INIT_SIZED` in C). The library exposes `ghostty_type_json()`, a process-lifetime JSON ABI manifest containing target, layout, enum, union, and packed-bit descriptors. Its own documentation says these descriptors describe the linked build and are **not** a cross-version stability promise. A pinned Rust binding should verify the expected schema and critical descriptors during startup/test instead of treating the manifest as permission to mix arbitrary headers and libraries.

#### Allocator and owned buffers

- Passing a `NULL` allocator selects Ghostty's default allocator (normally libc when linked; native freestanding is different).
- Custom allocator vtable callbacks and context must remain valid for all objects/buffers that use them.
- Memory returned by an allocating Ghostty API must be freed with `ghostty_free` and the exact same allocator (or `NULL` for the default). This matters particularly on Windows because the Zig and MSVC heaps can differ; calling ordinary `free` may be undefined behavior.
- `GhosttyString { ptr, len }` is borrowed, with the lifetime defined by its producing API. It is not necessarily UTF-8 merely because a safe wrapper would prefer `str`.
- `GhosttyBuffer` is caller-owned; on success `len` is bytes written, and on `OUT_OF_SPACE` it is the required capacity. `ptr == NULL, cap == 0` is valid only where the specific API documents size probing.

#### Borrowed and owned handles

- Ordinary grid/cell/row refs and render row/cell views are borrowed and are invalidated by the documented next mutation/update.
- Tracked grid references are caller-owned and must be explicitly freed. They follow cells through scrollback/reflow. If the terminal dies first they remain freeable, but report no value.
- A formatter borrows its terminal; the terminal must outlive the formatter. Its writer runs synchronously and may not re-enter the same formatter/terminal. Allocated formatter output must be released with `ghostty_free`.
- A render state owns a coherent copy. `begin_update` requires exclusive terminal access; after it returns, `end_update` operates on render-state-owned memory. Borrowed rows/cells remain valid only until the next render-state update. Raw packed cell bit positions are not a durable ABI; use typed accessors and validate the type manifest.
- A snapshot decoder borrows its source buffer or reader callback. A successfully decoded terminal is caller-owned, but remains borrowed by the decoder until `FINISH`/decoder free. Upstream marks full snapshot format v1 as WIP without binary compatibility guarantees; it must not become Zterm's wire or long-term persistence format.

#### Effects/callbacks

Effects are disabled by default. Queries, bell, title/PWD, device attributes, clipboard, notification, and unknown-sequence behavior only occurs when the embedder registers the relevant option.

Callbacks:

- run synchronously inside `vt_write`;
- must not recursively call `vt_write`/`write_until_ground` on the same terminal;
- must not block or perform expensive work;
- receive a single shared userdata pointer whose storage/address must remain valid;
- often receive borrowed bytes valid only during the callback.

The Rust bridge must copy callback bytes immediately into bounded actor-owned queues, return promptly, and never allow a Rust panic to unwind across C. Query replies should be queued for the PTY writer; policy-bearing effects such as OSC 52 must remain subject to Zterm's explicit deny/allow/bounds rather than inheriting a terminal library default.

#### Threading contract

The public C API does **not** promise that arbitrary handles can be moved or shared freely across threads. Individual APIs document specific synchronization patterns—for example the render-state two-phase update can shorten an embedder-owned terminal lock, and scrollback compression must be serialized with terminal writes/render/search—but that is not a general `Send + Sync` guarantee. Callbacks are non-reentrant.

The current safe wrapper intentionally makes the entire library `!Send + !Sync`; its [thread-safety documentation](https://github.com/Uzaaft/libghostty-rs/blob/5988a0b78b4aa804d1c12e66bbfe662bd97d81c0/crates/libghostty-vt/src/lib.rs#L49-L71) says the C implementation may use thread-local state and has no broad synchronization guarantee. It recommends constructing the terminal on a dedicated thread and communicating over channels.

This is a hard architecture constraint for Zterm:

- Creating a `Terminal` on one thread and moving it into a model thread is not valid.
- Wrapping it in `Mutex` does not make a `!Send` handle transferable.
- The terminal must be created inside its owner-thread closure, never leave it, and be dropped there.
- `resize`, snapshots, history pages, mode queries, mouse encoding, and shutdown all become actor commands.
- Actor responses contain only owned Zterm structs/bytes, never borrowed Ghostty views or handles.

The current daemon directly locks `TerminalModel` from ingest, resize, and attachment paths (`crates/daemon/src/terminal_driver.rs:346-354,525-642`), so the actor refactor is a prerequisite or an inseparable first migration phase. An unsafe hand-written `Send` implementation is not an acceptable shortcut.

### 3. Scrollback and mouse: what the library solves and what it does not

#### Scrollback

Ghostty supplies:

- pageable scrollback storage and resize reflow;
- main/alternate-screen behavior;
- active, viewport, screen, and history coordinate spaces;
- tracked/untracked grid refs;
- viewport top/bottom/delta/absolute-row movement and scrollbar information;
- history-aware selection/formatting;
- caller-driven incremental scrollback compression.

It does not create a background compression timer/thread. Zterm must schedule compression on the owner actor and serialize it with terminal access.

Ghostty's maximum scrollback byte and physical-line settings are approximate/page-granular. A page is comparatively large (currently on the order of hundreds of KiB), and a line limit can be exceeded by dozens of rows before a page is pruned. Zterm currently exposes exact capacity/resource semantics and estimates bytes from `size_of::<vt100::Cell>()`; that cannot be mechanically retained.

The adapter should set both Ghostty's byte and line bounds, expose at most the last configured logical Zterm rows, and separately measure actual resident/native memory. The migration gate must decide whether the existing exact resource contract is retained through a logical cap or deliberately changed in the spec.

History-coordinate lookups can traverse the page list and are not intended as a render-loop primitive. Full snapshot rendering should use render state; bounded history paging can use grid refs/selection formatting on the actor without mutating the user's viewport.

#### Mouse and selection

Ghostty supplies:

- mode state for X10/normal/button-event/any-event tracking and UTF-8/SGR/URxvt/SGR-pixel formats;
- a mouse encoder synchronized from terminal state;
- geometry-aware surface-position conversion;
- a selection gesture state machine for single/double/triple click, drag, release, deep press, and autoscroll.

This can replace Zterm's manual protocol byte encoding and much of its selection mechanics. It does **not** decide whether a wheel/click belongs to the remote TUI, local scrollback browsing, selection, or an application gesture. Zterm currently owns that decision in `crates/cli/src/terminal_ui.rs:683-739`; it must remain a Zterm UI policy. Alternate-scroll mode (`1007`) and live/history transitions still need explicit integration tests.

In short, adopting the library should make the observed scrollback and mouse bugs easier to solve, but it will not make them disappear automatically. Storage/reflow/encoding/selection-state move into the library; UI routing, remote protocol semantics, and attachment state remain ours.

### 4. Snapshot, delta, and checkpoint compatibility gaps

The largest non-build migration risk is not parsing; it is Zterm's downstream state contract.

- Current `TerminalCheckpoint` owns `vt100::Screen` and current delta generation uses `Screen::state_diff` (`crates/core/src/terminal.rs:530-579,739-833`). A `!Send` Ghostty terminal or borrowed screen cannot be stored in or moved with an attachment checkpoint.
- Ghostty's formatter can emit full plain/VT/HTML terminal state, but it does not expose an equivalent stable per-attachment ANSI `state_diff` API.
- Ghostty render dirty state is useful for a renderer; it is not automatically the same as Zterm's current wire delta/recovery baseline.
- Ghostty binary snapshot contains internal parser/screen/history state and is explicitly version-unstable. It cannot substitute for Zterm's wire snapshot or durable checkpoint.

The adapter therefore needs a Zterm-owned semantic checkpoint/diff representation. The smallest safe first phase can return a full Zterm resync snapshot whenever no proven delta exists, but the decision spike must quantify bandwidth and demonstrate a path to normal small deltas before deleting `vt100`.

Other semantic details that require an adapter rather than direct exposure:

- Zterm's “active grid only” snapshot rule and active/inactive screen behavior.
- One revision per non-empty ingest chunk and one per successful resize, including same-size resize.
- History epoch/gap/cursor stability under pruning and reflow.
- Exact DA/DSR/CPR reply bytes and device identity; Ghostty's defaults may differ.
- OSC 52, title reporting, notifications, and other effects safety policy.
- Zterm resource admission/projection independent of Ghostty's native allocation layout.

### 5. Official build/linking approach

#### Upstream build

The native upstream command is:

```text
zig build -Demit-lib-vt=true
```

Relevant controls include:

- `-Doptimize=Debug|ReleaseSafe|ReleaseFast|ReleaseSmall`;
- `-Dcpu=baseline` for distributable artifacts;
- `-Dsimd=false` to reduce native dependency/CPU surface during the first migration;
- `-Dvt-features=-all,+formatter,+selection,+render_state,+input_encode,+grid_introspection,...` to explicitly select library features;
- `-Demit-xcframework=true` on a macOS/Xcode host for Apple static slices.

Current feature groups include formatter, selection, render state, input encoding, grid introspection, snapshot, search, color, glyph protocols, and Kitty graphics. A conservative Zterm start is formatter + selection + render state + input encoding + grid introspection; snapshot/search/color/Kitty graphics should remain disabled until a concrete use is accepted.

Ghostty also provides an official CMake wrapper. The upstream example uses `FetchContent`, invokes Zig, and exports `ghostty-vt` (shared) and `ghostty-vt-static` targets. Its example `GIT_TAG main` is convenient documentation, not a reproducible production setting; Zterm must substitute an immutable commit/archive digest.

#### Static versus dynamic

Use static linking first.

Benefits for this migration:

- one version of native code and Rust bindings is selected at final link;
- no `rpath`, SONAME, DLL search, app-bundle framework placement, or runtime ABI mismatch;
- simplest path for iOS and Android packaging;
- easier rollback and per-target artifact attestation while the ABI remains pre-1.0.

Costs:

- larger shipped binaries/build caches;
- each final artifact must reproduce all upstream/native license notices;
- native runtime/system libraries may still be dynamic;
- the feature/target/optimization tuple becomes part of every final binary's identity.

Dynamic linking can be reconsidered for Linux distribution packaging after ABI stability improves. Today the shared library identifies as major `0`/`0.1.0-dev`, and a mismatched system-installed library is too easy to combine with stale generated bindings. On Windows, the shared import library and static archive have deliberately different names (`ghostty-vt.lib` versus `ghostty-vt-static.lib`); linking the wrong one can silently create an unwanted DLL runtime dependency.

For the first accepted artifact, use:

```text
static + ReleaseSafe + baseline CPU + default Rust features off
```

The inspected wrapper can set optimization and CPU, but does not currently forward Ghostty's
native SIMD/`vt-features` controls. Therefore the task must inventory the actual native archive and
must not claim a minimal native feature set. If that surface fails security/resource/size gates,
add an audited wrapper build option upstream (or reconsider a narrow fork) before cutover. Consider
`ReleaseFast` only after semantic, sanitizer, and target-platform gates pass.

### 6. Rust integration options

| Option | Assessment | Decision |
| --- | --- | --- |
| Manual `extern "C"` declarations in `zterm-core` | Brittle around C-int enums, sized structs, unions, type-erased set/get, callbacks, and moving pre-1.0 ABI; spreads unsafe into product code. | Reject |
| Run bindgen in every Cargo build | Tracks headers, but adds libclang/toolchain variability and still provides no safe ownership/thread/callback layer. | Reject |
| Dedicated checked-in `-sys` crate + newly written safe wrapper | Technically sound isolation, but makes Zterm permanently own a large, subtle unsafe surface already implemented elsewhere. | Fallback only |
| Existing `libghostty-vt-sys` + existing safe `libghostty-vt`, then Zterm safe adapter | Concentrates generated/raw FFI, RAII, lifetimes, callback plumbing, modes, formatter, selection, mouse, snapshot, and render APIs; lets product crates retain `unsafe_code = "forbid"`. | Recommend, after audit/fix pin |
| `cxx`/`autocxx`, a custom C shim, or direct Zig API | Adds a second bridge without removing the underlying C ownership/ABI problem; Zig API is not the intended cross-language stable boundary. | Reject unless a specific missing C API requires a tiny reviewed shim |
| Prebuilt target bundles behind a `-sys` crate | Strong release-time option once every target has verified artifacts/SBOM; current third-party offerings do not cover Zterm's full target matrix. | Future packaging improvement |

The existing community wrapper is valuable but not an official Ghostty deliverable. “Reuse” means
audit and pin its exact Git source under the narrow Zterm source-policy exception, not trust its
crate name as an upstream stability guarantee. A maintained fork/vendor is reserved for a required
fix or build control that cannot land upstream in time.

The Zterm adapter remains necessary even with a safe wrapper because the wrapper models Ghostty; it does not model Zterm's revision, history protocol, wire snapshots/deltas, resource admission, callbacks policy, or actor scheduling.

### 7. Critical wrapper version and soundness facts

The name/version `libghostty-vt 0.2.1` does not currently identify one source/build contract.

#### crates.io 0.2.1

- Published 2026-07-18.
- Publication source corresponds to repository commit `46a9d2ac941ed600cf43c5e6299c8dfd1d3a1ef0`.
- It pins Ghostty `a887df42c56f6de86c0fe6da9c4eeca37931e083`.
- That Ghostty revision requires Zig `0.15.2`.
- It has no current iOS target handling.
- The registry checksums observed for the two packages are:
  - `libghostty-vt 0.2.1`: `6219ed1c364b3ef5815fb0fd2acc0662f792adc706dabf59b7ca89280ee6d066`;
  - `libghostty-vt-sys 0.2.1`: `865fed12a8b2bba3507b3bccd0bef439e06ef1100e652fe6b71d132c41ee8db0`.

#### Repository HEAD with the same declared 0.2.1 version

- Inspected revision: `5988a0b78b4aa804d1c12e66bbfe662bd97d81c0`.
- `Cargo.toml` still declares workspace version `0.2.1` and Rust `1.90`.
- It pins Ghostty `22d13172cde98a0a4dda05d3d6a3fcb0dd8ed018`.
- That Ghostty revision requires Zig `0.16.0`.
- It contains newer Windows static-archive handling, Android mappings, and an iOS XCFramework path.
- It includes a 2026-09-01 [soundness fix](https://github.com/Uzaaft/libghostty-rs/commit/8de6c75c68823c4bf5f78a275f0912f406a66b70) for reachable callback/string issues: null pointer with zero-length clipboard-clear data, treating binary clipboard bytes as unchecked UTF-8, and nullable title/PWD strings.

This means the published crates.io `0.2.1` predates known reachable UB fixes. It is not suitable for a production migration. It also means a report that merely records “0.2.1” is non-reproducible and potentially unsafe.

Near-term choices are:

1. use the current two crates at an exact reviewed Git revision at or after the fix, with a single-repository source-policy exception and verified offline source preparation (the selected Zterm plan);
2. wait for and audit a new crates.io release containing the fix and needed target work; or
3. vendor/fork the current two crates at an exact reviewed commit, while also pinning its exact embedded Ghostty source and build inputs.

Zterm's `deny.toml` currently denies unknown Git dependencies and allows no Git source, so adding a Cargo Git dependency **without an explicit policy change** is not acceptable. This task resolves that boundary by allowing only the exact wrapper repository, pinning a full revision in `Cargo.toml`/`Cargo.lock`, and requiring verified source preparation plus an offline Cargo build. A reviewed local/vendor package or a newly published exact registry release remains a fallback if the narrow Git-source exception is later rejected.

### 8. Build reproducibility and supply-chain controls

The current community `build.rs` is not hermetic by default:

- absent `GHOSTTY_SOURCE_DIR`, it runs `git clone` and checks out the hard-coded Ghostty commit inside `OUT_DIR`;
- Zig may fetch hashed package inputs unless `GHOSTTY_ZIG_SYSTEM_DIR` supplies an immutable prefetched system directory;
- the default optimize mode changes with the Cargo profile (`Debug`, `ReleaseSmall`, otherwise `ReleaseFast`);
- CPU defaults to `baseline` in current Git, but remains an environment-controlled input;
- current Git does not pass Ghostty's `-Dvt-features`, so disabling the Rust crate's default features does **not** itself prove the native Ghostty build is minimal.

Required production controls:

1. No network in normal `cargo build`. Fetch source/toolchain/dependencies in a separate verified preparation step.
2. Pin the wrapper commit/package checksum, embedded Ghostty commit/source archive digest, public header digest, generated binding digest, Zig `0.16.0` distribution digest, Zig package hashes/system directory, target tuple, CPU, optimize mode, SIMD, and exact VT feature set.
3. Keep generated bindings checked in. Regenerate only through an explicit update command with a pinned bindgen/libclang tool; review the diff.
4. At build/test startup compare expected `ghostty_build_info` and critical `ghostty_type_json()` descriptors with the linked artifact. Fail closed on mismatch.
5. Produce per-target static artifacts with a manifest/SBOM and license inventory. A single Ghostty source commit does not uniquely identify target/compiler/feature output.
6. Add a dependency-update playbook: diff headers and bindings, re-audit every unsafe block/callback trampoline, run corpus/differential/fuzz/sanitizer/platform tests, then update the pins together.

The existing local spike confirms the exact current tuple can build on macOS arm64 with Rust 1.98 and Zig 0.16.0, but its 610 MiB temporary target directory also shows why build cache/artifact strategy should be deliberate. The final probe binary was 5.7 MiB and dynamically depended only on macOS `libSystem`; those numbers are observations for one debug probe, not production size guarantees.

### 9. Platform feasibility

| Target | Upstream lib feasibility | Current community wrapper state at `5988a0…` | Required Zterm gate |
| --- | --- | --- | --- |
| macOS arm64/x86_64 | Native static/shared supported; upstream CI covers Apple builds. | Direct target mappings exist; macOS arm64 local static spike passed. | Run both release arches and deployment-floor smoke tests. |
| Linux GNU/musl, x86_64/arm64 | Native/cross static/shared supported. | Explicit mappings for common GNU/musl targets. | Link/run on glibc floor and musl; inspect native dependencies. |
| Windows MSVC/GNU, x86_64/arm64 | Upstream builds static/shared; native Windows CI tests library; MSVC is default ABI when not explicit. | Current Git handles distinct static archive and maps MSVC/GNU targets. | Run real MSVC and GNU binaries; verify no accidental DLL dependency and allocator/free behavior. |
| iOS | Upstream supports only the VT library; static XCFramework, minimum iOS 13; device and Apple-silicon simulator slices. | Current Git supports only `aarch64-apple-ios` and `aarch64-apple-ios-sim`, requires macOS + Xcode/SDK, static only; x86_64 simulator is unsupported. Published crates.io 0.2.1 lacks this path. | Compile device/sim and run an app-level simulator smoke; verify Xcode packaging/codesign and deployment floor. |
| Android | Upstream NDK builds cover arm64, x86_64, and older arm; current upstream config accounts for Android 15+ 16 KiB pages. | Current Git maps arm64 and x86_64 Android; no equivalent strong wrapper runtime CI evidence was found. | NDK compile/link plus emulator/device runtime smoke; inspect 16 KiB-page compatibility. |
| Wasm | Upstream has a dedicated artifact/API path. | Not a current Zterm release requirement. | Treat separately; native Rust wrapper assumptions may not apply. |

Upstream compile coverage is encouraging but is not application runtime validation. The release gate needs actual Windows execution plus iOS simulator and Android emulator/device smoke tests.

For the all-platform product architecture, distinguish two cases:

- If iOS/Android remain remote clients receiving Zterm-owned snapshots/deltas from a host daemon, they do not need to link the terminal engine at all.
- If the apps locally replay ANSI or host a local terminal model, the static mobile library is feasible but must pass the gates above. This choice should be driven by protocol/offline requirements, not by the fact that an XCFramework happens to exist.

Likewise PTY portability is a separate concern. Keep `portable-pty` on current hosts; only reassess the PTY/process layer if a mobile app must launch local child processes. `libghostty-vt` neither solves nor requires that change.

### 10. Licenses and notices

- Ghostty is MIT-licensed; preserve the upstream [MIT license and copyright notice](https://github.com/ghostty-org/ghostty/blob/20abdb50a6216c450d6d4d010c41c7edf5ab15b2/LICENSE) in source and binary distributions.
- The community Rust workspace declares `MIT OR Apache-2.0`; its repository includes an MIT license. If vendored, select and preserve the actual license text/attribution rather than relying only on Cargo metadata.
- A static Ghostty build also incorporates or derives data/code from its Zig package/native dependency graph (for example Unicode/uucode and, depending on flags, highway/simdutf, Wuffs/Kitty graphics, zlib/png, and compiler runtimes). The exact list depends on selected features and build target.
- `cargo deny` audits Rust packages, not every Zig/C source incorporated into a static archive. Add a native-source SBOM/license inventory for the actual selected feature set. Do not assume Ghostty's top-level MIT notice alone covers every vendored component.

Starting with SIMD and Kitty graphics disabled reduces both attack surface and notice inventory, but the actual produced archive still needs inspection.

### 11. Testing and security risks

Positive upstream evidence:

- Ghostty has a dedicated `zig build test-lib-vt` target and C-ABI/type-schema tests.
- Upstream CI builds/tests across macOS, Linux, Windows and compiles iOS/Android/Wasm targets.
- Upstream has AFL++ targets/corpora for parser, OSC, and full VT streams.
- The community wrapper has Rust tests and Miri tests for Rust-owned seams.

Limits:

- Building a fuzz harness is not the same as continuously running a meaningful fuzz campaign.
- Miri cannot execute native FFI, so it cannot validate pointer lifetimes, allocator pairing, callbacks, or Zig/C behavior.
- The recent reachable wrapper soundness fix demonstrates that even a safe-looking wrapper needs an independent FFI audit.
- Upstream main contracts and the wrapper's older embedded Ghostty commit are different revisions. Main documentation is directional evidence; implementation must audit the exact pinned headers/bindings.
- Native build scripts that clone/fetch during Cargo build are a supply-chain and availability risk even when the final Git commit is hard-coded.

Required tests before migration completion:

- Existing Zterm corpus with whole-buffer, one-byte, fixed-size, and randomized chunking.
- Differential semantic comparison against `vt100` during the migration window, especially wide/combining characters, wrap/reflow, main/alternate screens, modes, malformed sequences, and resize.
- Exact DA/DSR/CPR reply and bounded-effect tests, including hostile OSC/DCS/Kitty inputs and OSC 52 policy.
- History retention/pruning/reflow/selection tests plus actual native-memory measurements.
- Snapshot/delta reconnect/recovery tests, including small-update bandwidth and history gap/epoch behavior.
- Mouse X10/UTF-8/SGR/URxvt/SGR-pixel encoding and Zterm live/history/alternate-scroll routing tests.
- ASan/UBSan native integration where supported, callback fuzzing, and repeated create/drop/resize/compress cycles.
- Locked offline build test that fails if either `git` or network access is attempted.
- Target runtime smoke on Windows; iOS simulator and Android emulator/device become required only
  if a local mobile terminal engine enters scope. For the current remote-client architecture, the
  mobile gate is that `zterm-core`/`zterm-proto` remain free of Ghostty/Zig/C dependencies.

### 12. Smallest decision spike before committing the migration

The existing macOS arm64 local probe is a useful Gate 0: it proved that Rust 1.98 can use current Git `libghostty-rs`, statically link its embedded Ghostty with Zig 0.16.0, and exercise basic terminal, callbacks, modes, scrollback, formatter, and selection APIs. It does **not** resolve Zterm's actor, history/resource, snapshot/delta, or platform contracts.

Before deleting `vt100` or committing to the full migration, run one isolated 2–3 day decision spike with this scope:

#### Locked inputs

- wrapper source exactly `5988a0b78b4aa804d1c12e66bbfe662bd97d81c0` or a later reviewed commit containing `8de6c75…`;
- embedded Ghostty exactly `22d13172cde98a0a4dda05d3d6a3fcb0dd8ed018` (or update both wrapper/bindings/source together after audit);
- Zig exactly `0.16.0` with recorded distribution checksum and prefetched Zig packages;
- static, `ReleaseSafe`, baseline CPU, wrapper default Rust features off, with the actual native
  feature/archive surface recorded rather than incorrectly claiming unsupported native trimming;
- no network during Cargo build.

#### Prototype boundary

Create a non-production terminal actor that constructs the safe wrapper inside its thread. Its command set should be only:

```text
Create / Ingest / Resize / SemanticState / FullSnapshot /
HistoryPage / EncodeMouse / Shutdown
```

Install bounded synchronous callbacks for PTY replies, title, and bell. Clipboard and other side effects remain disabled/rejected. No raw/safe Ghostty handle, borrowed view, callback reference, or formatter crosses the actor boundary.

#### Required evidence

1. Run the existing terminal corpus in whole/one-byte/fixed/random chunks and compare owned semantic output with `vt100`.
2. Prove exact DA/DSR/CPR and OSC 52 policy.
3. Feed more than configured history; fetch an absolute bounded history page without mutating viewport; measure retained logical rows and real native memory.
4. Exercise main/alternate screen, wide/combining cells, resize reflow, and same-size Zterm revision.
5. Exercise X10/UTF-8/SGR mouse plus alternate-scroll mode, while demonstrating that routing remains Zterm policy.
6. Produce the existing full snapshot semantics and either demonstrate a viable Zterm-owned delta or quantify the temporary full-resync penalty.
7. Record `ghostty_build_info`, ABI manifest/schema/hash, static dependency list, shipped size, RSS, and ingest throughput.
8. Compile/link the current macOS/Linux release matrix and Windows shared boundary; run Windows
   runtime smoke when that product target is claimed. If local mobile parsing later enters scope,
   separately compile/run the iOS device/simulator and Android arm64/x86_64 matrix.
9. Produce an offline-build proof and initial native SBOM/license report.

#### Go/no-go gate

Proceed with replacement only if the spike proves, or the task explicitly accepts spec changes for:

- single-owner actorization without breaking PTY drain/order;
- exact owned snapshot/history semantics;
- a credible delta/checkpoint plan;
- bounded callback/query/security behavior;
- logical and physical scrollback resource bounds;
- every currently required release target's static link/runtime evidence, without treating mobile
  remote clients as local-engine targets;
- a locked offline source/toolchain/artifact pipeline.

If snapshot/delta or resource semantics cannot be resolved, retain `vt100` while continuing an isolated adapter experiment. Do not ship a long-lived dual-engine production path; it doubles semantic and security maintenance.

## Caveats / Not Found

- Upstream has no independent stable `libghostty-vt` tag or ABI compatibility window as of the research date.
- No official upstream Rust binding was found; `Uzaaft/libghostty-rs` is community maintained.
- The current community repository contains substantial unpublished changes while still declaring `0.2.1`; crates.io `0.2.1` and Git `0.2.1` must never be treated as interchangeable.
- No general upstream C `Send`/`Sync` guarantee was found. The safe wrapper's conservative `!Send + !Sync` model is the correct contract to program against.
- No stable upstream equivalent of `vt100::Screen::state_diff` was found.
- No guarantee of exact scrollback row/byte enforcement was found; upstream documents page-granular approximate limits.
- No strong current community-wrapper Android runtime CI evidence was found. Current iOS support is limited to arm64 device and Apple-silicon simulator, on a macOS/Xcode host, static only.
- The platform matrix above describes library feasibility, not that Zterm's full application, PTY layer, packaging, or store policies are already portable.
- Main-branch API documentation was inspected for the newest contracts, while the recommended wrapper revision embeds an earlier Ghostty commit. Every implementation must re-check the exact pinned headers/generated bindings; no claim here authorizes mixing them.
- License compatibility of all transitive native sources must be verified from the actual feature-selected build/SBOM; this research only establishes the top-level licenses and identifies the audit boundary.
