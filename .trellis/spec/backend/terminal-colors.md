# Terminal Color and Appearance Contract

## 1. Scope / Trigger

Apply when changing terminal color controls, host observations, semantic color
metadata, controller transfer, input reply framing, history repaint, or cursor
overlays. These are terminal compatibility features. User theme files, presets,
theme UI, native chrome and translucent rendering are separate work.

`ZtermColorState` is owned by the existing host engine. The pinned Alacritty
engine remains the sole grid and SGR owner. Never add a second terminal emulator
to the frontend or forward child global setters to the physical terminal.

Protocol references: [xterm controls](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html)
and [kitty colors](https://sw.kovidgoyal.net/kitty/color-stack/).
`color_names.rs` contains the licensed, sorted X.Org rgb 1.1.0 table, never a
runtime host database or network lookup.

## 2. Signatures

```rust
TerminalModel::update_base_colors(&mut self, profile: TerminalColorProfile)
    -> Result<TerminalUpdate, TerminalError>
TerminalDriver::update_base_colors(&self, profile: TerminalColorProfile)
    -> Result<Option<Revision>, TerminalDriverError>
TerminalViewCommandWriter::update_colors(&self, profile: TerminalColorProfile)
    -> Result<(), DaemonError> // async
```

The private Session creation bundle `InitialTerminal` carries viewport and
colors together. `create_with_colors_until` and `prepare_attach_with_colors_until`
install observations before a newly created PTY can emit its first query.
Operation fingerprints include the creation profile. The frontend retains the
latest profile and supplies it with each fresh attachment identity on reconnect.

## 3. Contracts

### Semantic state and rendering

- A profile contains exactly 262 slots: palette 0–255, foreground 256,
  background 257, cursor 258, cursor text 259, selection foreground 260,
  selection background 261; appearance is Unknown, Dark or Light.
- Values are Unknown, opaque RGB, or Dynamic. Dynamic is legal only for slots
  258–261. Unknown means unobserved, never guessed black/default RGB.
  Owner-only `None` override means inherit the current controller base.
- Snapshot metadata contains the effective profile, `changed_at: Revision`,
  mode-5 flag, explicit-cursor provenance and 262 bounded inherited-source
  indices. Core fixed arrays are boxed to bound stack/enum size without losing
  cardinality. Wire representation is exactly one required snapshot/profile.
- `changed_at <= enclosing revision`. A candidate cannot decrease that stamp
  or change colors under an unchanged stamp. Unknown source mappings survive
  stack restore so the frontend can preserve physical fallback identity.
- Base changes advance the existing model revision only when the base differs.
  Effective changes use that same clock; app overrides may hide a base change.
  Color-only deltas can have zero row patches. They never rewrite semantic
  indexed/default cells or churn a full history buffer's epoch.
- Mode 5 maps default FG/BG, indices 0/7 and 8/15 once; other indices and literal
  RGB stay fixed. Cell SGR 7 remains independent. Known values are already
  mapped; only Unknown fallback needs its explicit source.
- Render order: semantic colors, inverse, selection, software cursor. Explicit
  underline color stays explicit; default underline follows final text.
  Underline shapes are none/single/double/curly/dotted/dashed.
- Any explicit cursor/cursor-text override, including Dynamic, uses a steady
  software block over the actual glyph span. Hide the native cursor, keep CUP
  at the semantic cursor for IME, repair wide/combining old and new spans, and
  hide the cursor in history. Reset returns to inherited native behavior.
- The sole presenter retains separate semantic and resolved physical baselines.
  Resolve only child content; child overrides never color ZTerm chrome.
  Successful output commits both; failed output keeps semantic fallback,
  invalidates physical output and forces full recovery.
- Pinned history uses the newest valid `changed_at` without refetching rows or
  losing a valid selection. A delayed history response cannot roll colors back.

### Child protocol

- OSC 4 queries/sets all 256 entries; 104 resets all or listed entries.
  OSC 10/11/12/17/19 support successive parameters and matching
  110/111/112/117/119 resets. Process each query at its exact stream position.
- OSC 21 accepts numeric entries and foreground/background/cursor/cursor_text/
  selection_foreground/selection_background. `key=?` queries; empty special
  setter means Dynamic; bare key resets. A supported Unknown query returns an
  empty value; an unsupported key returns `=?`. Invalid setters change nothing.
- Numeric color formats: X11 `#` with high-bit component precision, scaled
  `rgb:`, finite clamped `rgbi:`; licensed case-folded X11 names are supported.
  Only fully opaque alpha is accepted. Never apply CSS short-hex semantics.
- CSI 996 requests reported host appearance; CSI 997 reports it. DEC 2031
  subscribes, unsubscribes and supports DECRQM. No RGB luminance inference.
  External base/appearance changes notify subscribed children, including a
  palette change with the same appearance. App setters/resets/stack operations
  do not generate an external-change notification.
- Ten xterm stack slots: zero/default push/pop; 1–10 store/restore without
  changing stack depth; report depth and highest stored slot. OSC 30001/30101
  are aliases. Save effective values and unresolved sources; restore neither
  appearance nor mode 5 nor subscription. Overflow/underflow is a no-op.
- Main/alternate screen switches, detach, reconnect and takeover retain app
  overrides and stack. Reset uses the latest controller base. RIS clears app
  overrides, stack, mode 5 and 2031, but retains the controller base.
- Admit SGR 4:0–5/24/58/59 through Alacritty. Never discard a whole SGR because
  it contains 58/59; sibling resets, foreground and background still apply.
- DECRQSS reports the actual supported current SGR pen. XTGETTCAP allowlist:
  Co/colors=256, RGB=8, TN/name=xterm-256color, Tc/Su booleans, Smulx/Setulc
  templates. Unknown/malformed capability emits bare DCS 0+r ST and stops its
  query list. Never advertise an unsupported capability.

### Driver, wire and controller

- One driver commit mutex covers ingest/resize/base mutation through generated
  PTY replies and revision publication. Return Some(revision) only when this
  base update changes state, comparing under the gate/model lock; None means
  unchanged. Never infer that from a separately sampled revision watch, since
  unrelated PTY output can race and falsely require another takeover ACK.
  Release the model mutex before I/O;
  independent child interruption/reaping remains available while a reply blocks.
- Wire v2 remains canonical: create 209, attach 323, base update 324. Retire
  create 202 and attach 300 without aliases or downgrade. Required colors are
  validated before Session/PTY effects; an empty operation lease may already
  exist when an unknown mutation kind is rejected.
- Kind 324 carries exact attachment ID, complete profile, strictly increasing
  nonzero observation sequence. Only the current controller may update its base.
  Sequence scope is one attachment; reconnect starts at zero with latest profile.
  No input or prior update frames are replayed to the replacement attachment.
- A pending takeover stages its proposed base. Lease handoff applies it and,
  when colors change, replaces the prepared snapshot and requires its exact ACK.
  Fresh takeover must not inherit `ever_active` before that ACK. An already
  active controller retains the existing replacement-window input policy.

### Physical input and observation

- Probe once before interactive create/attach, bounded by 250 ms/64 KiB.
  Query all 256 palette entries with **one index per complete OSC 4 command**,
  then default/special roles, appearance and mode 2031, followed by one DSR 5n
  response boundary. Serialize all commands in one write/flush/observation round;
  never add a per-index wait. No IDs are available.
  Bound the response to each physical command, not only its request length:
  Ghostty 1.3.1 uses a fixed 1 KiB response allocator whose ArrayList growth
  fails on a 32-index reply, aborting slice processing and exposing query tails.
  Single-index OSCs avoid that demonstrated failure without brand detection or
  changing host-authoritative color semantics. A short incoming OSC is not proof
  its expanded RGB response fits a terminal's allocation budget.
- At most one accepting/draining round plus one pending refresh bit. Timeout
  closes acceptance but retains consume-only framing until DSR 0n. Missing
  boundary prevents another ambiguous round. Partial refresh preserves prior
  observations; only a completed round publishes its profile.
- One persistent `HostInputCodec` demultiplexes typed physical replies before
  prefix, selection and child input through prepare/ACK/reconnect. BEL/ST/C1,
  UTF-8 continuation, cancellation and oversized controls remain framed.
  Bracketed paste owns its entire apparent reply payload literally.
- Input epoch transitions join the reader and retain queued reply bytes; do
  not `tcflush` them away. Kernel drain is capped at 64 KiB, aggregate retained
  events at the existing resume-input bound. Decode old-epoch controls while
  dropping old keyboard units; never expose reply tails as keystrokes.
- Enable 2031 only after observing it reset. Restore only an enable owned by
  this guard, including normal/error/signal/panic cleanup. Unknown, already set
  and permanently set/reset states create no restoration ownership.
- The presenter writes physical queries and this owned subscription change.
  It never writes child palette/default/selection/cursor setters or stack ops.

## 4. Validation & Error Matrix

| Input or state | Result |
| --- | --- |
| Missing color message, wrong slot/source count, out-of-range source/RGB/enum, Dynamic FG/BG | reject before installing a semantic frame or creating a Session |
| Future/decreasing color stamp or different values with unchanged stamp | reject candidate transactionally |
| Old create/attach kind | unknown kind; no compatible fallback or Session/PTY effect |
| Wrong attachment ID, stale/zero observation sequence, former controller | typed stream/controller error; no base mutation |
| Invalid OSC setter, invalid stack slot, translucent color | no state mutation; unrelated valid siblings still run |
| Unobserved legacy query | no fabricated RGB reply; OSC 21 explicitly reports empty |
| Generated replies exceed 64 KiB | terminal-fatal reply overflow; no continuation with an ambiguous stream |
| Physical probe timeout or missing boundary | retain last observed profile; drain without accepting late values |
| Physical write/flush failure | preserve semantic baseline; invalidate physical baseline |

## 5. Good / Base / Bad Cases

- Good: a light controller provides observed FG/BG before the child startup
  OSC 11 query; the reply and painted default background agree.
- Base: an outer terminal reports nothing; Unknown inherits physical defaults,
  unsupported queries do not invent dark appearance or black RGB.
- Bad: restoring a saved palette after takeover uses the old controller base
  forever. Restore saved effective values; subsequent reset uses the new base.

## 6. Tests Required

- `terminal/tests/color_protocol.rs`: stream-position/chunk invariance, all
  entries/roles, stack/mode/reset lifetimes, notifications, actual pen, bounded
  malformed controls, full-capacity history epoch unchanged by color-only input.
- Core/proto tests: required exact metadata, legal values/sources/stamps,
  retired kinds, underlines and semantic conversion.
- `session_color_tests.rs` and `session_wire`: startup query before attach,
  controller/takeover ACK gate, authenticated remote base/delta, exact IDs.
- `terminal_driver`: block query I/O, race base update and prove reply order and
  publication watermark using gates, not sleeps.
- `host_colors` and reader-fence tests: fragmented replies across epochs, paste,
  missing boundary, bounded oversized controls, coalescing, owned restoration.
  `palette_probe_bounds_each_terminal_response_and_keeps_one_round` asserts
  independently framed single-index queries for every slot and exactly one
  flush/final fence. Slot-substring coverage alone missed the physical failure.
- Presenter/UI tests: palette-only live/history repaint, semantic retry,
  selection and wide cursor behavior; reconnect preserves latest profile.
- `just check` is the authoritative native gate. The bounded
  `tests/foundation/terminal-colors.py` fixture and adjacent smoke recipe cover
  physical comparisons; record host and application smoke evidence separately.

## 7. Wrong vs Correct

```rust
// Wrong: changes physical global state and lets queries disagree with history.
outer.write_all(child_osc_setter);

// Correct: state is owned by the host model; the sole presenter resolves cells.
let update = model.update_base_colors(observed_profile)?;
// Driver writes update.replies, then publishes update.revision under its gate.
```

Wrong: clear queued terminal input at activation, or treat all surviving bytes as
fresh keys. Correct: retain one framing codec across epochs, classify physical
replies first, discard stale keyboard units, and only then route active input.
