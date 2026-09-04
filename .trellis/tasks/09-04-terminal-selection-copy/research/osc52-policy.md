# OSC 52 policy research

## Decision target

Define the smallest interoperable OSC 52 write subset that lets a nested TUI copy to the current
Zterm controller's real clipboard without turning PTY output into an unbounded or replayable host
side-effect channel.

## Protocol and implementation evidence

- XTerm defines `OSC 52;Pc;Pd`: `Pc` may address clipboard, primary, secondary, select, or cut
  buffers; `Pd` is normally RFC 4648 Base64; `?` requests a clipboard read. An empty selector has
  xterm-specific multi-target semantics (`s0`), so it must not be treated as an alias for the
  portable system clipboard.
  <https://www.invisible-island.net/xterm/ctlseqs/ctlseqs.html>
- Alacritty defaults OSC 52 to `OnlyCopy`: clipboard writes are useful for remote sessions while
  clipboard reads are an unnecessary exfiltration surface. Its 0.26.0 engine accepts `c` for the
  system clipboard, decodes standard padded Base64, and requires valid UTF-8 before producing a
  `String` clipboard event.
  <https://github.com/alacritty/alacritty/blob/master/extra/man/alacritty.5.scd>
- Ghostty defaults OSC 52 writes to allow and reads to ask. This supports Zterm's fixed
  allow-write/deny-read split, but Zterm deliberately does not inherit Ghostty's read prompt or add
  an attachment policy switch: its structured effect is already restricted to the event-time
  controller and can never return clipboard contents to the child.
  <https://ghostty.org/docs/config/reference>
- Kitty exposes separate read/write permissions and a very large generic 512 MB maximum. That cap
  is a terminal-emulator product choice, not appropriate evidence for a Zterm control message that
  crosses a network boundary.
  <https://sw.kovidgoyal.net/kitty/conf/#opt-kitty.clipboard_control>
- Herdr 0.8.2 accepts one standard text/plain clipboard content and caps it at 192 KiB. It rejects
  empty and oversized contents before its client clipboard path.
  <https://github.com/herdrdev/herdr/blob/v0.8.2/src/ghostty/mod.rs>

## Cap derivation

Current Zterm semantic bounds are 80 rows, 240 columns, and 22 UTF-8 bytes per cell. A pathological
full visible viewport plus one newline between each non-wrapped row is:

```text
80 * 240 * 22 + 79 = 422,479 bytes
```

A 512 KiB decoded cap (`524,288` bytes) therefore guarantees every selection in the current MVP's
complete visible viewport. Its maximum canonical padded Base64 length is:

```text
4 * ceil(524,288 / 3) = 699,052 bytes
```

The structured wire message carries decoded text, not Base64, so 512 KiB plus protobuf overhead is
comfortably below `MAX_CONTROL_PAYLOAD_BYTES = 1 MiB`. A 1 MiB decoded cap would leave no protobuf
overhead under that control-frame contract and is therefore not a valid choice. Herdr's 192 KiB is
safe but cannot guarantee Zterm's own current maximum visible selection.

## Recommended policy

1. Accept only non-empty system-clipboard writes with selector exactly `c`; reject reads, clears,
   ambiguous/multiple selectors, primary/secondary/select buffers, and cut buffers.
2. Require canonical standard padded RFC 4648 Base64, valid non-empty UTF-8, no NUL, and at most
   524,288 decoded bytes. Preserve valid contents byte-for-byte; never truncate or sanitize.
3. Keep the ordinary 1,024-byte control-string cap. Enter a dedicated bounded OSC 52 parser state
   only after recognizing its command, with a 699,052-byte encoded-data ceiling. Overflow consumes
   through the terminator and produces only a content-free rejection.
4. Route a decoded, structured, redacted effect only to the exact controller attachment that owned
   the Session when the event occurred. Never send it to observers, snapshots/history, persistence,
   reconnect replay, logs, or child replies.
5. Retain no more than one clipboard payload per ingest and one replaceable pending delivery per
   Session/controller. Latest wins; a slow client cannot backpressure the PTY or accumulate payloads.
   Do not add an arbitrary timer rate limit without an observed need.
6. Always allow bounded writes and unconditionally deny reads. Do not add a configuration surface
   without a current consumer. A process-name allowlist or per-request prompt has no reliable
   application identity and would make full-screen TUIs fragile. Revisit configurability only when
   an observed deployment or later client policy requires it.
7. Re-encode canonical OSC 52 only at the desktop clipboard sink and serialize it through the sole
   host-output owner. Android will use its native clipboard sink instead.
