# Color reporting as the first theme milestone

> Historical research: the user subsequently requested all C05–C18 gaps and C13.
> The converged PRD/design and color-protocol-contracts.md now own scope and decisions.

## User decision

On 2026-09-06, the user asked to put other theme topics aside and first complete
zterm's downward color reporting, questioning whether this belongs to the basic
responsibilities of a terminal multiplexer. This establishes the priority of P8.
It does not establish that the earlier Herdr screenshot has a proven root cause,
nor approve implementation of an unfinished design.

## Protocol evidence

- The [xterm control sequence reference](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html)
  documents `OSC 4 ; index ; ?` for indexed palette queries, including multiple
  index/spec pairs, and `OSC 10/11/12 ; ?` for default text foreground,
  background, and text cursor colors. Dynamic-color queries can also request
  successive colors. A compatible implementation needs valid reply framing as
  well as the correct color values. These are xterm ecosystem protocols, not a
  universal mandate that every terminal implement every xterm extension.
- [tmux's input implementation](https://github.com/tmux/tmux/blob/master/input.c)
  has handlers for OSC 4, 10, 11, and 12, plus current-theme reporting. This is
  evidence that color inquiry is an established multiplexer responsibility.
- The [Contour appearance extension](https://contour-terminal.org/vt-extensions/color-palette-update-notifications/)
  defines `CSI ? 996 n`, dark/light replies `CSI ? 997 ; 1/2 n`, and opt-in
  `CSI ? 2031 h/l` notifications. The notification also covers terminal profile
  palette changes, so consumers may need to query actual colors again even if
  the dark/light classification remains the same. This is a modern extension
  with a documented, non-universal adoption matrix.

Sources checked on 2026-09-06. Moving tmux master is comparative evidence only;
implementation research should pin any source behavior it relies on.

## Proposed first deliverable

1. Make default foreground/background and palette indices 0–255 queryable with
   values consistent with the effective terminal view.
2. Include cursor-color query compatibility, with explicit handling for outer
   terminals that cannot report a concrete cursor color. Do not fabricate a
   successful reply with an unrelated color.
3. Define direct dark/light inquiry separately from the concrete RGB values.
4. Recommend opt-in runtime change notifications and color refresh in this same
   deliverable. The user has not yet chosen between full runtime behavior and
   an initial attach-time-only milestone.
5. Cover both local and remote Sessions and reattachment. A missing outer reply
   must not stall PTY output or consume ordinary user input.

The invariants are accurate reporting, coherent display/query state, and
application-neutral behavior. Herdr/Pi are smoke fixtures, not routing inputs.

## Ownership evidence and open design work

- The current Session contract permits one controller per Session; an ordinary
  second attach is occupied, and takeover changes the controller lease
  (`.trellis/spec/backend/session-service.md:108`). Concurrent viewers with
  independent controlling themes are therefore not a current product ambiguity.
- Recommended base-color authority is the current controlling frontend's
  effective terminal view. The Session-side terminal owner must respond from a
  coherent color state across the local/remote boundary. Attach/takeover,
  disconnected retained Sessions, unavailable outer color responses, and
  initialization ordering still require a concrete design.
- Programmatic color setting/reset is a separate capability from read queries.
  The user's present priority is reporting; do not silently turn it into a full
  dynamic-palette mutation task. A future mutation feature must update the same
  effective state used for rendering and query replies.
- There is no need to wait for an SSH comparison to justify supporting these
  protocols. That comparison remains useful only for attributing the earlier
  screenshot and must not block this milestone.

## Planning status

Priority is confirmed. Detailed protocol coverage, fallback/lifecycle contracts,
and runtime notification scope have not converged; no product code is changed.
