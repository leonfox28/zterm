# Ghostty + Doubao input-method investigation

## Evidence status

The user reports that Ctrl+] followed by period appears to work only with an
English input method. They identified the outer terminal/input method as
Ghostty + Doubao. Installed application bundle versions read on 2026-09-05:

- Ghostty 1.3.1: /Applications/Ghostty.app/Contents/Info.plist.
- Doubao 0.9.7: /Library/Input Methods/DoubaoIme.app/Contents/Info.plist.

The user completed the physical-key probe and reported completion. The complete
manual record is retained as [doubao-physical-keys.json](./doubao-physical-keys.json),
copied unchanged from target/herdr-escape-probe/physical-keys/
doubao-1788617018601879000.json. It contains two chords per phase and completed=true.
Computer Use had rejected Ghostty access before any UI interaction; these are
user-operated keys, not agent-injected events. No alternative UI automation was
attempted.

The enhanced-prefix defect is already independently proven in
[findings.md](./findings.md). This capture establishes a second cause in the
reported environment: period is committed as U+3002 in modes 0 and 7. Ctrl+]
does arrive. Every prefix-to-command interval is below half a second, ruling
out the existing one-second deadline for these six attempts.

## Actual physical-input observations

In this table ESC denotes byte 0x1b. Rows describe both recorded attempts.

| Outer flags | Ctrl+] press | Period press | Prefix-to-period intervals |
| --- | --- | --- | --- |
| 0 | 0x1d | UTF-8 e3 80 82, U+3002 `。` | 387 ms, 374 ms |
| 7 | ESC [93;5u | UTF-8 e3 80 82, U+3002 `。` | 350 ms, 448 ms |
| 15 | ESC [93;5u | ESC [46u, period key | 448 ms, 432 ms |

Flags 7 also report Ctrl+] release (ESC [93;5:3u) and period release
(ESC [46;1:3u). A release revealing key 46 does not make the preceding plain
text a key-press event; do not implement commands on release or retrospectively
reinterpret arbitrary text to work around this.

Flags 15 additionally report left Control press/release (key 57442). The
observed order is Control press, Ctrl+] press, ] release with Ctrl still held,
Control release, period press, period release. The command state must not treat
Control release as an unknown command that cancels the prefix. This is a general
modifier-lifecycle requirement, separate from consuming the ] release.

This proves report-all exposes the intended period key in this environment
when enabled before the chord. It does not test turning reporting on immediately
after Ctrl+], fast queued input, an already-open composition, future letter
bindings, all other IMEs, or a remote session. No separate English hardware
capture is needed to establish the observed U+3002-versus-key-46 distinction.

## Pinned primary-source findings

[Ghostty v1.3.1 key encoder](https://github.com/ghostty-org/ghostty/blob/v1.3.1/src/input/key_encode.zig):
encoding selects Kitty when enabled (:74). Under flags 7, unmodified printable
input can still be sent as UTF-8 (:175-196), so enhanced mode does not guarantee
an encoded period key. Report-all skips this shortcut (:156), allowing a known
key to be encoded. Composition can suppress non-modifier events (:133-139),
and a pure-text event without a key entry still emits UTF-8 (:201-205).
Alternate/base-layout reporting is conditional (:229-260). Consequently flags
15 are useful for a diagnostic comparison, not proof of universal IME support.

[Ghostty v1.3.1 macOS input view](https://github.com/ghostty-org/ghostty/blob/v1.3.1/macos/Sources/Ghostty/Surface%20View/SurfaceView_AppKit.swift):
keyDown invokes AppKit interpretation (:1155) and uses accumulated committed
text in a key event when available (:1168-1180). Composition is also represented
explicitly (:1188-1194). insertText outside that accumulator sends text directly
(:1960-1990). Therefore not every IME commit reaches the terminal as a complete
physical-key identity. This describes possible paths, not which path Doubao
took in the user's failing attempt.

[Kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/):
alternate-key reporting can include a base-layout key useful for shortcut
matching. Associated text and key identity are distinct. Report-all requests
encoded printable keys; it does not let a downstream application reconstruct
an event that the terminal/input method did not deliver.

The local prefix parser only accepts bytes 0x1d then 0x2e
(crates/cli/src/terminal_ui.rs:1320), within one second (:109). The captured
U+3002 cannot match 0x2e. Likewise the captured encoded Ctrl+] does not match
0x1d. No U+FF0E or upstream-suppressed prefix was observed in this sample.

## Minimal physical-input capture

Run [capture_host_keys.py](./capture_host_keys.py) in a fresh Ghostty shell,
outside zterm, after selecting Doubao Chinese:

```sh
python3 .trellis/tasks/09-05-investigate-zterm-herdr-escape/research/capture_host_keys.py --label doubao
```

Each of three phases lasts eight seconds. On each prompt, physically press
Ctrl+] then release Ctrl and press period, twice. Do not paste a text chord.
The phases request flags 0 (ordinary shell), 7 (observed Herdr flags), and 15
(same plus report-all). The script records raw chunks/timing, not assumptions
about physical identity. It creates no child session or network connection.
Its keyboard-stack entry is popped and termios restored on completion or
SIGTERM. Results go to ignored target/herdr-escape-probe/physical-keys/.

An optional English comparison can use the same command with --label english.
Keep input-method switching outside a capture, so switching keystrokes do not
obscure the command sequence. A mode already active before the prompt does not
test the race involved in changing modes immediately after a prefix.

Recorder checks passed with an isolated injected outer PTY: exact flags-0/7/15
sample bytes, normal completion and SIGTERM restoration. macOS's kernel-managed
PENDIN bit was excluded from the termios equality comparison. All synthetic
records use --label injected-smoke and kind injected-pty-smoke; they must never
be reported as physical Ghostty or Doubao evidence.

## Final scope after user review

The user selected local cancellation for unknown commands and timeout, then
reviewed the drawbacks of temporary keyboard reporting and declined that
input-method adaptation. Retain the shared semantic prefix/command dispatcher
and remove --escape. Preserve existing child/selection-driven keyboard modes.

No prefix-triggered report-all request, mode-query/acknowledgement state,
punctuation remapping, IME switching, terminal-brand patch or additional physical
IME probe is planned. Decode key identity when already supplied, but do not turn
U+3002 text or a period release into a period press. The captured Chinese
punctuation limitation remains in modes 0/7 even after semantic prefix repair.

The rejected approach could expose the period identity once reporting was active,
but did not establish reliable prefix-triggered switching or future letter input.
Its costs included queued suffix input preceding mode activation, composition
suppressing key events, varying terminal support, and additional event/mode
restoration ownership. These findings remain research evidence rather than
implementation requirements.

Physical diagnosis is complete for the captured chord. The task returns to the
reduced command-router change, with no product code changed yet.
