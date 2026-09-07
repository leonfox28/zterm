# One scrollable terminal (2026-09-06)

The user clarified that terminal output should feel inherently scrollable,
without separate current/history pages, and requested the eight-key bar during
scrolling as well as typing. The UI contract is one viewport with a reading
position: follow new output at the bottom; preserve the anchor while reading
above it; resume following when scrolled back to the bottom. Cached scrolling
and continuous selection stay local. Only missing ranges need network reads.

Internally, a mutable current screen and retained scrollback still have distinct
roles. A terminal program can position its cursor and rewrite cells; its screen
is not simply an append-only text document. Xterm.js's buffer API independently
exposes cursor position, the bottom page, and the visible viewport position:
[IBuffer documentation](https://xtermjs.org/docs/api/terminal/interfaces/ibuffer/).
That is supporting terminology, not a decision to use a WebView/xterm.js.

Fullscreen/alternate-screen programs may not retain earlier screen images.
XTerm documents normal-buffer saved lines and a display-sized alternate buffer:
[XTerm control sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-The-Alternate-Screen-Buffer).
Do not promise a replay of every overwritten screen. The Android UI can browse
only output retained by its existing host semantic contract. Keep this data
constraint out of ordinary UI labels; it does not require a History page.

The persistent row contains the same eight keys in every terminal position.
It moves above the visible IME and remains above the system navigation inset
when IME is hidden, without covering the grid. Existing input synchronization
and history/selection admission rules were unchanged by that v5 visibility
revision. The later accepted v8 review updates PRD SCROLL-3: a healthy return
on the same attachment retains the first complete input unit within the shared
bound and sends it once after sync acknowledgement and Active. Actual
disconnection/reconnect never queues input for later replay. See
[ui-completeness-review.md](ui-completeness-review.md) and `design.md` for the
shared-owner evidence and cancellation boundaries.

Album selection is also clarified: launch the Android single-image system
picker directly, using its Activity-contract system fallback if unavailable.
An App-owned gallery is out of scope. The official
[Photo picker documentation](https://developer.android.com/training/data-storage/shared/photo-picker)
documents `PickVisualMedia(ImageOnly)`, URI/cancel results, and the
`ACTION_OPEN_DOCUMENT` fallback. The prototype's old custom gallery is removed;
an editor-only note records this external OS interaction. Actual picker
appearance belongs to the device OS, including Xiaomi's installed system.
